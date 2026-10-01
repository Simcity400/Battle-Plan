//! The screen-space sprite batch: textured quads in **device pixels**.
//!
//! R2 section 5.4 measures the 29 x 27 victory-point frame as 28 x 27 device
//! pixels in the 2560 x 1440 screenshot, which is a 1.25x upscale of a
//! 2048 x 1152 frame (R1 section 0): the map-icon layer draws 1:1 with no GUI
//! scaling. Section 6.7 confirms it independently on the 27 px counter plate.
//! So this pass takes rectangles in device pixels, snaps them to the pixel grid
//! and converts to clip space on the CPU — there is no projection matrix.
//!
//! The one scale factor is the viewer's own interface scale
//! ([`super::ui_scale`]): callers lay the batch out in interface pixels and
//! [`device_corners`] multiplies each snapped rectangle into device pixels at
//! upload. At 1.0, which a dump uses, interface and device pixels are the same.
//!
//! Every glyph of [`super::text`] goes through the same batch, so a label's
//! sprite and its text are one draw when they share a page. Quads are grouped
//! by texture before upload, which changes the order of *unrelated* quads only;
//! the map-icon priorities of R2 5.5 are honoured by the order the caller
//! pushes them in, and everything this slice draws is priority 5.
//!
//! Depth is off and the batch runs after every world pass, so it always wins
//! against the terrain, the water and the country names.

use std::{collections::HashMap, io, sync::Arc};

#[cfg(test)]
pub(super) mod oracle;

use eframe::wgpu;
use installed_content::{
    ResolvedLayers, gfx,
    image::{dds, native::Decode},
};

use super::{
    // counters
    counters::CounterAssets,
    lazy_icons::{DYNAMIC_BASE, LazyTextures},
    scene::{Targets, sampler_entry, texture_entry},
    text,
    textures::{Bound, ColorSpace, upload},
};

/// One frame-animated sprite: a horizontal strip of equal-width frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteSheet {
    /// The `GFX_...` name.
    pub name: String,
    /// Texture id in [`MapIcons::images`].
    pub texture: usize,
    /// `noOfFrames`.
    pub frames: u32,
    /// Whole-strip size in texels.
    pub size: [u32; 2],
}

impl SpriteSheet {
    /// One frame's size in device pixels: `floor(w / n)` texels wide.
    /// **UNVERIFIED** for strips that do not divide evenly: the game may draw
    /// the frame `w / n` wide, a third of a pixel more on the naval-base strip.
    pub fn frame_size(&self) -> [f32; 2] {
        [
            (self.size[0] / self.frames.max(1)) as f32,
            self.size[1] as f32,
        ]
    }

    /// The atlas rectangle of a zero-based frame, clamped to the last frame.
    ///
    /// Frames sit at `i / n` of the strip. The naval-base strip is 301 texels
    /// for 11 frames and its glyphs are authored on that 27.36-texel pitch;
    /// slicing at `i * floor(w / n)` drew frame `i` `0.36 * i` px right of the
    /// game (measured on frames 0-7 in the 2026-09-24 L1a pairs) and cut the
    /// gold badge's right edge.
    pub fn frame_uv(&self, frame: u32) -> [f32; 4] {
        let frames = self.frames.max(1);
        let index = frame.min(frames - 1);
        [
            index as f32 / frames as f32,
            0.0,
            (index + 1) as f32 / frames as f32,
            1.0,
        ]
    }
}

/// Every texture and font the icon layer draws from.
pub struct MapIcons {
    pub combat: crate::combat::Assets,
    /// Decoded pages and strips; the batch refers to these by index.
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    pub victory_points: SpriteSheet,
    // counters: the unit-counter, base and combat sheets, and the flag atlas.
    pub counters: CounterAssets,
    /// The raid map icon's layout, sprites and raid content.
    pub raids: super::raids::RaidAssets,
    /// Flags and portraits decoded on first draw, above [`DYNAMIC_BASE`].
    pub lazy: Arc<LazyTextures>,
}

impl MapIcons {
    /// Reads the fonts and the map-icon sprites from the resolved layers.
    ///
    /// `interface` and `gfx/interface` are covered content and go through
    /// [`ResolvedLayers::read`]; `gfx/fonts` is not, and [`text::Fonts::load`]
    /// states that limitation ([archived rendering notes](#) section 4).
    ///
    /// `decode` is the client's texture decode state
    /// ([`installed_content::settings::SettingsKeys::texture_decode`]), which the font pages load under.
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content::defines::Defines,
        map: &installed_content::map::MapData,
        language: &str,
        map_size: [u32; 2],
        gui_viewport: [u32; 2],
        decode: Decode,
    ) -> io::Result<Self> {
        let mut images = Vec::new();
        // The label sprite and font are the ones `mapicons.gui` names.
        let labels = super::labels::LabelLayout::read(layers)?;
        // The natively drawn map-icon windows read nothing the legacy readers give. This read
        // decides only which sprites, fonts and textures the windows' resources load, a set no
        // GUI viewport changes (`max_resolution` changes a `windowType`'s size, never which
        // elements exist); every read a view lays out and draws is at its own viewport
        // (`CounterAssets::windows_at`).
        let windows = super::counters::NativeWindows::load_at(layers, gui_viewport)?;
        let layouts = super::counters::Layouts::read(layers)?;
        let mut font_warnings = Vec::new();
        // The battle-plan label font is named by `maparrows.txt`'s `arrowText`.
        let arrow_font = super::orders::arrow_text_font(layers)?;
        // The fonts and sprites of one read of the interface files, as the game reads them.
        let registry = installed_content::gui::native::resources::Registry::load(layers)?;
        let interface = Interface {
            sprites: gfx::SpriteSet::new(&registry.files),
            registry,
            decode,
        };
        let fonts = text::Fonts::load(
            layers,
            &interface,
            language,
            &text::DeclaredFonts {
                label: &labels.font,
                counter: layouts.land.count.font.as_deref(),
                ping: layouts
                    .faction_ping
                    .as_ref()
                    .ok()
                    .and_then(|ping| ping.count.font.as_deref()),
                arrow: arrow_font.as_deref(),
            },
            &mut images,
            &mut font_warnings,
        )?;
        let sprites = &interface.sprites;
        let victory_points = load_sheet(layers, sprites, &labels.sprite, &mut images)?;
        // Flag and portrait reads happen after this load returns, so the lazy
        // stores keep their own handle on the layers.
        let shared = Arc::new(layers.clone());
        let lazy = LazyTextures::new();
        // counters
        let mut counters = CounterAssets::load(
            &shared,
            (layouts, windows),
            defines,
            sprites,
            (language, map_size, decode),
            &mut images,
            &lazy,
        )?;
        counters.warnings.extend(font_warnings);
        // What the game does not define in the interface files (each file's own level, each
        // sprite and each font declaration), named and counted; a consumer asking for a refused
        // sprite is told why ([`gfx::SpriteSet::lookup`]), and a refused font does not load
        // ([`text::load`]).
        let refused: Vec<String> = sprites
            .refused
            .iter()
            .cloned()
            .chain(
                interface
                    .registry
                    .fonts
                    .fonts()
                    .iter()
                    .flat_map(|font| &font.facts.refusals)
                    .map(|refusal| format!("font {refusal}")),
            )
            .collect();
        if !refused.is_empty() {
            counters.warnings.push(format!(
                "Interface files: {} refusals of what the game does not define: {}.",
                refused.len(),
                refused.join("; ")
            ));
        }
        let combat = crate::combat::Assets::load(
            &shared,
            defines,
            &interface,
            &counters.flags,
            language,
            &mut images,
            &lazy,
        )?;
        let mut raids = super::raids::RaidAssets::load(layers, defines, map, sprites, &mut images)?;
        raids.triggers = combat.tactics.trigger_names();
        if images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(Self {
            combat,
            images,
            fonts,
            victory_points,
            counters,
            raids,
            lazy,
        })
    }
}

impl MapIcons {
    /// Loads the map-icon windows' resources ([`CounterAssets::load_gui`]) once the names pass
    /// has read `localisation`, appending their textures after every other static page. Returns
    /// what does not load, for the warning area.
    pub fn load_gui(
        &mut self,
        layers: &ResolvedLayers,
        localisation: &installed_content::localisation::Localisation,
    ) -> io::Result<Vec<String>> {
        let warnings = self
            .counters
            .load_gui(layers, localisation, &mut self.images)?;
        if self.images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(warnings)
    }
}

/// The interface files as the game read them (the resources and each sprite by name), and the
/// client's texture decode state.
pub struct Interface {
    pub registry: installed_content::gui::native::resources::Registry,
    pub sprites: gfx::SpriteSet,
    pub decode: Decode,
}

/// Loads one declared sprite and appends its texture to `images`.
pub fn load_sheet(
    layers: &ResolvedLayers,
    sprites: &gfx::SpriteSet,
    name: &str,
    images: &mut Vec<dds::Texture>,
) -> io::Result<SpriteSheet> {
    let (texture, frames, size) = decode_sheet(layers, sprites, name)?;
    let id = images.len();
    images.push(texture);
    Ok(SpriteSheet {
        name: name.to_owned(),
        texture: id,
        frames,
        size,
    })
}

/// Reads and decodes one declared sprite without appending it anywhere, so
/// callers can decode sheets on scoped threads and append in a fixed order.
pub fn decode_sheet(
    layers: &ResolvedLayers,
    sprites: &gfx::SpriteSet,
    name: &str,
) -> io::Result<(dds::Texture, u32, [u32; 2])> {
    let sprite = sprites.lookup(name).map_err(invalid)?;
    let path = sprite
        .texture
        .as_deref()
        .ok_or_else(|| invalid(format!("sprite {name} names no texturefile")))?;
    let bytes = layers.read(path)?;
    let texture = dds::decode(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
    if sprite.frames == 0 || texture.width < sprite.frames {
        return Err(invalid(format!(
            "sprite {name} declares {} frames for a {} texel wide strip",
            sprite.frames, texture.width
        )));
    }
    let size = [texture.width, texture.height];
    Ok((texture, sprite.frames, size))
}

/// One textured quad in device pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    /// Texture id in [`MapIcons::images`].
    pub texture: usize,
    /// `x, y, width, height` in device pixels, `y` down from the top.
    pub rect: [f32; 4],
    /// Normalised atlas rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Straight RGBA multiplier; `[1, 1, 1, 1]` draws the texture as authored.
    pub color: [f32; 4],
    // counters
    /// Clockwise rotation about the rectangle's centre, in radians.
    ///
    /// `mapicons.gui` rotates two children of the map icons this pass draws:
    /// the counter bars by `1.5708` and `battle_indicator_direction` by the
    /// combat's own heading. Everything else pushes zero and keeps the
    /// pixel-snapped axis-aligned path.
    pub rotation: f32,
    // counters
    /// The sort key [`layer`] builds; see it for the field order.
    pub layer: u64,
    /// `buttonstate.shader`'s Over (positive) and Down (negative) techniques: after the colour,
    /// `rgb + (0.5 + rgb) * shade`; 0 for none.
    pub shade: f32,
    /// A quad [`super::native_gui`] places and shades as the game does; `rect`, `uv`, `rotation`
    /// and `shade` are then unused.
    pub exact: Option<Exact>,
}

/// A quad drawn where the game's GUI draws it (`CSpriteType::build_geometry@14222CB90` through
/// the composed world matrix to the rasteriser): its corners are not snapped, a scissor cuts it,
/// and its effect's program runs with the effect's sampler state (see
/// [`super::native_effects`]). `Quad::texture` is slot 0 and `Quad::color` the program's `Color`
/// (a progress bar's first colour, a glyph's vertex colour).
#[derive(Clone, Debug, PartialEq)]
pub struct Exact {
    pub native: Option<Arc<super::native_shader::draw::Quad>>,
    /// Where the vertices land, in interface pixels.
    pub corners: [[f32; 2]; 4],
    /// Each vertex's texture coordinate as the vertex buffer holds it (before `Offset`).
    pub uv: [[f32; 2]; 4],
    /// The scissor `x0, y0, x1, y1` in whole interface pixels: the device's is each coordinate
    /// times the interface scale, truncated (`CGraphics::convert_coordinates@1422225B0`).
    pub scissor: Option<[i32; 4]>,
    /// [`super::native_effects::Pixel::packed`].
    pub pixel: u32,
    pub blend: super::native_effects::Blend,
    /// The samplers of slot 0, slot 1 and the masking slot 5, ten bits each from bit 0, 10 and
    /// 20 ([`super::native_effects::Sampler::packed`]).
    pub samplers: u32,
    /// Each slot's `MipMapLodBias` (slot 0, slot 1, slot 5).
    pub lod_bias: [f32; 3],
    /// Slot 1 (a progress bar's `TextureTwo`) and slot 5 (the masking texture).
    pub second: Option<usize>,
    pub masking: Option<usize>,
    /// `Offset` and `NextOffset`.
    pub offsets: [f32; 4],
    /// `Time - AnimationTime`, `AnimationTime` and a progress bar's `CurrentState`.
    pub inputs: [f32; 4],
    /// A progress bar's `vSecondColor`, or a text's `ModColor`.
    pub second_color: [f32; 4],
}

// counters
/// A quad's sort key: the engine's icon priority, then the icon's own place in
/// the pass, then the child's place inside its window.
///
/// Quads are uploaded sorted by `(layer, texture)`. The engine sorts map icons
/// by `DEFAULT_PRIORITY_*` (R2 section 5.5) and draws each icon's children in
/// the order `mapicons.gui` declares them, so **only quads that are the same
/// child of the same icon may be reordered** — which is what batches a run of
/// glyphs onto one page without ever letting one counter's flag overtake
/// another counter's plate. Each icon has [`CHILD_SLOTS`] child slots, more
/// than the land counter's 19 child kinds; a caller that would need more
/// refuses rather than share a slot.
///
/// The key is 64-bit so every `u32` icon ordinal keeps its own range under its
/// priority: the icon count grows with the countries and units a recording
/// holds, and a narrower key let icon 62,500 of one priority collide with the
/// next priority's first icon. The signed priority maps onto bands 0 to 255 in order.
pub const fn layer(priority: i8, icon: u32, child: u32) -> u64 {
    band_layer((priority as u8 ^ 0x80) as u64, icon, child)
}

/// A key in the band above every map-icon priority, where the combat window
/// draws over the map icons as the GUI does.
pub const fn gui_layer(icon: u32, child: u32) -> u64 {
    band_layer(GUI_BAND, icon, child)
}

const fn band_layer(band: u64, icon: u32, child: u32) -> u64 {
    band * PRIORITY_STRIDE + icon as u64 * CHILD_SLOTS as u64 + child as u64
}

/// `NMapIcons.<name>` as an icon's sort priority: `SetMapIconPrioritiesForMapMode`
/// (`140e02410`) stores the integer define's low byte, and every icon reads it back
/// sign-extended into `CMapIcon + 0x70` (`MOVSX`, e.g. `1418bbed9`), so 300 sorts as 44.
pub fn priority(defines: &installed_content::defines::Defines, name: &str) -> io::Result<i8> {
    Ok(defines.int32("NMapIcons", name)? as i8)
}

/// Child slots per icon in a [`layer`] key.
pub const CHILD_SLOTS: u32 = 32;

/// Keys one priority spans: room for every `u32` icon of [`CHILD_SLOTS`] children.
const PRIORITY_STRIDE: u64 = 1 << 40;
const _: () = assert!((u32::MAX as u64 + 1) * CHILD_SLOTS as u64 <= PRIORITY_STRIDE);
/// The band [`gui_layer`] keys, above the 256 map-icon priorities.
const GUI_BAND: u64 = 256;

/// The priority a [`layer`] key was built with.
#[cfg(test)]
pub const fn layer_priority(layer: u64) -> i64 {
    (layer / PRIORITY_STRIDE) as i64 - 128
}

/// The icon ordinal of a [`layer`] key, within its priority.
#[cfg(test)]
pub const fn layer_icon(layer: u64) -> u64 {
    layer % PRIORITY_STRIDE / CHILD_SLOTS as u64
}

/// What moves a [`layer`] key by `icons` whole icons.
pub const fn icon_offset(icons: u32) -> u64 {
    icons as u64 * CHILD_SLOTS as u64
}

/// A frame's worth of screen-space quads, in the order the caller pushed them.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub quads: Vec<Quad>,
    // counters
    /// The layer subsequent pushes are tagged with.
    layer: u64,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Empties the batch as [`Batch::default`] does, keeping its storage.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.layer = 0;
    }

    // counters
    /// Tags every later push with this sort layer.
    pub fn at_layer(&mut self, layer: u64) {
        self.layer = layer;
    }

    /// Adds `quad` at the current layer.
    pub fn push(&mut self, quad: Quad) {
        self.quads.push(Quad {
            layer: self.layer,
            ..quad
        });
    }

    /// Adds one sprite frame with its top-left corner at `at`.
    pub fn sprite(&mut self, sheet: &SpriteSheet, frame: u32, at: [f32; 2], color: [f32; 4]) {
        let size = sheet.frame_size();
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv: sheet.frame_uv(frame),
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
        });
    }

    // counters
    /// Adds one sprite frame drawn into an explicit device-pixel size.
    ///
    /// `mapicons.gui` gives several children a `scale`, and the flag cells are
    /// drawn from an atlas whose cell is not the sprite's frame, so the size
    /// cannot always come from the sheet.
    pub fn scaled(
        &mut self,
        sheet: &SpriteSheet,
        frame: u32,
        at: [f32; 2],
        size: [f32; 2],
        color: [f32; 4],
    ) {
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv: sheet.frame_uv(frame),
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
        });
    }

    // counters
    /// Adds one sprite frame rotated clockwise about its own centre.
    pub fn rotated(
        &mut self,
        sheet: &SpriteSheet,
        frame: u32,
        at: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        color: [f32; 4],
    ) {
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv: sheet.frame_uv(frame),
            color,
            rotation,
            layer: self.layer,
            shade: 0.0,
            exact: None,
        });
    }

    // counters
    /// Adds an explicit atlas rectangle of an already-known page.
    pub fn cell(
        &mut self,
        texture: usize,
        at: [f32; 2],
        size: [f32; 2],
        uv: [f32; 4],
        color: [f32; 4],
    ) {
        self.quads.push(Quad {
            texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv,
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
        });
    }

    // counters
    /// Adds the leftmost `pixels` of one frame, clipping the rectangle and the uv.
    ///
    /// This is how the counter bars are filled: `mapicons.gui` declares
    /// `bar_org` and `bar_str` with `rotation = 1.5708`, which turns the
    /// 3 x 17 texture into a 17 x 3 horizontal bar. Both textures are a
    /// **single flat colour** — `mapicon_bar_org_sprite.dds` is
    /// `(105, 195, 115)` in every one of its 51 texels and
    /// `mapicon_bar_str_sprite.dds` is `(195, 145, 75)` — so the quarter turn
    /// produces exactly the same pixels as drawing the frame into the rotated
    /// rectangle, and the caller passes the already-rotated size.
    pub fn bar(
        &mut self,
        sheet: &SpriteSheet,
        at: [f32; 2],
        size: [f32; 2],
        pixels: f32,
        color: [f32; 4],
    ) {
        // A full native counter bar covers one pixel past its sprite
        // (`super::counter_inputs::bar_pixels`); the texture is one flat colour, so the quad
        // simply stretches over it.
        let width = pixels.clamp(0.0, size[0] + 1.0).floor();
        if width < 1.0 {
            return;
        }
        let fill = (width / size[0]).min(1.0);
        let [u0, v0, u1, v1] = sheet.frame_uv(0);
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], width, size[1]],
            uv: [u0, v0, u0 + (u1 - u0) * fill, v1],
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
        });
    }

    /// Adds a laid-out run of glyphs.
    pub fn text(&mut self, glyphs: &[text::TextGlyph], color: [f32; 4]) {
        self.quads.extend(glyphs.iter().map(|glyph| Quad {
            texture: glyph.page,
            rect: glyph.rect,
            uv: glyph.uv,
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
        }));
    }
}

/// A quad's `x, y, width, height` in interface pixels as device-pixel corners
/// `x0, y0, x1, y1`.
///
/// The rectangle is rounded to whole interface pixels first: the layout adds up
/// `.gui` integers on top of a projected anchor, so the fractional part is the
/// anchor's alone, and keeping it would resample a 1:1 sprite. The corners are
/// then scaled and rounded to device pixels, corners rather than sizes, so
/// quads that touch in interface pixels still touch on screen and every edge
/// stays on the pixel grid. At a scale of 1.0 this is the plain rounding.
pub fn device_corners(rect: [f32; 4], ui_scale: f32) -> [f32; 4] {
    let x0 = rect[0].round();
    let y0 = rect[1].round();
    let corners = [x0, y0, x0 + rect[2].round(), y0 + rect[3].round()];
    corners.map(|value| (value * ui_scale).round())
}

/// An exact quad's scissor in device pixels, `x, y, width, height`, within a target of `size`:
/// each coordinate times the interface scale, truncated (`CGraphics::convert_coordinates
/// @1422225B0`), then held inside the target, which the device's rasteriser never passes.
/// `None` when nothing is left.
pub fn device_scissor(scissor: [i32; 4], ui_scale: f32, size: [u32; 2]) -> Option<[u32; 4]> {
    let device = scissor.map(|value| (value as f32 * ui_scale) as i64);
    let x0 = device[0].clamp(0, i64::from(size[0]));
    let y0 = device[1].clamp(0, i64::from(size[1]));
    let x1 = device[2].clamp(0, i64::from(size[0]));
    let y1 = device[3].clamp(0, i64::from(size[1]));
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0].map(|value| value as u32))
}

/// The texel-centre window a quad's atlas rectangle is sampled inside, as the
/// shader's `window` computes it: the rectangle inset by half a texel on every
/// side, so no bilinear tap reaches a neighbouring frame, glyph or flag cell.
/// A rectangle narrower than a texel collapses to its centre.
///
/// A 1:1 or minified quad on whole device pixels already samples inside it,
/// so this changes nothing there. A magnified one, which a UI scale above 1.0
/// draws, samples its edge pixels past the last texel centre; clamping them to
/// the window repeats the cell's edge texels, which is what an edge-extended
/// gutter around the cell would hold. The shader's `window` is the one that
/// runs; this is its specification for the tests.
#[cfg(test)]
pub fn sample_window(uv: [f32; 4], texels: [u32; 2]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for axis in 0..2 {
        let half = 0.5 / texels[axis].max(1) as f32;
        let low = uv[axis].min(uv[axis + 2]);
        let high = uv[axis].max(uv[axis + 2]);
        let middle = (low + high) * 0.5;
        out[axis] = (low + half).min(middle);
        out[axis + 2] = (high - half).max(middle);
    }
    out
}

/// `position`, `uv` and `color`, already in clip space; `cell` is the quad's
/// whole atlas rectangle and `filter` one of [`filter_mode`]'s values.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    cell: [f32; 4],
    filter: f32,
    shade: f32,
}

/// One vertex of an [`Exact`] quad: its position in clip space, its texture coordinate as the
/// vertex buffer holds it and its masking coordinate, then the draw's constants.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[cfg(test)]
struct ExactVertex {
    position: [f32; 2],
    uv: [f32; 2],
    mask_uv: [f32; 2],
    color: [f32; 4],
    second: [f32; 4],
    offsets: [f32; 4],
    inputs: [f32; 4],
    lod_bias: [f32; 4],
    pixel: u32,
    samplers: u32,
}

/// How the shader samples a batch drawn at `ui_scale`: 0 the plain bilinear
/// tap of the unscaled layer, whose output a scale of 1.0 keeps exactly; 1 that
/// tap clamped to the cell's texel-centre window when magnified; 2 clamped and averaged over
/// every texel a pixel covers when minified, so the icon does not alias.
pub fn filter_mode(ui_scale: f32) -> f32 {
    if ui_scale == 1.0 {
        0.0
    } else if ui_scale > 1.0 {
        1.0
    } else {
        2.0
    }
}

/// Every icon page on the GPU, shared by the sprite batch and the name pass.
///
/// Both passes sample the same font atlases, so the pages are uploaded once and
/// each pass builds its own bind groups over them with the sampler its shader
/// declares.
pub struct IconTextures {
    bound: Vec<Bound>,
    /// Clamped and pinned to the top mip: what a 1:1 screen quad needs.
    screen: wgpu::Sampler,
}

impl IconTextures {
    pub fn views(&self) -> Vec<wgpu::TextureView> {
        self.bound.iter().map(|bound| bound.view.clone()).collect()
    }

    /// Uploads every page of `icons`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, icons: &MapIcons) -> io::Result<Self> {
        Self::from_images(device, queue, &icons.images)
    }

    fn from_images(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[dds::Texture],
    ) -> io::Result<Self> {
        let mut bound = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            bound.push(upload(
                device,
                queue,
                &format!("map icon {index}"),
                image,
                ColorSpace::Linear,
            )?);
        }
        let clamp = |label, mip, max_lod| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: mip,
                lod_min_clamp: 0.0,
                lod_max_clamp: max_lod,
                ..Default::default()
            })
        };
        Ok(Self {
            bound,
            // The batch samples the top mip only: at 1:1 on integer pixel
            // corners that is the only one a sample can land in, and a mip of
            // an atlas would blend neighbouring cells. Scaled quads keep to
            // their cells through the shader's window and area filter instead
            // (`SpritePass::upload`); saying so outright keeps a driver's
            // derivative estimate out of the result.
            screen: clamp("icon screen sampler", wgpu::MipmapFilterMode::Nearest, 0.0),
        })
    }

    /// One bind group per page, in page order, for a texture + sampler layout.
    pub fn bind_all(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
    ) -> Vec<wgpu::BindGroup> {
        self.bound
            .iter()
            .map(|texture| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("icon page"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                    ],
                })
            })
            .collect()
    }

    pub fn screen_clamp(&self) -> &wgpu::Sampler {
        &self.screen
    }

    /// Pages uploaded, for reporting.
    pub fn page_count(&self) -> usize {
        self.bound.len()
    }
}

/// Which pipeline a run draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pipe {
    /// The snapped screen-space sprites.
    Legacy,
    /// [`Exact`] quads, blended or not.
    #[cfg(test)]
    Exact(super::native_effects::Blend),
    Native(usize),
}

/// The textures an exact run binds: slot 0, slot 1 and the masking slot.
#[cfg(test)]
type Slots = [usize; 3];

/// One run of consecutive vertices sharing a pipeline, its textures and a scissor.
struct Run {
    pipe: Pipe,
    texture: usize,
    #[cfg(test)]
    slots: Slots,
    scissor: Option<[u32; 4]>,
    range: std::ops::Range<u32>,
}

/// A lazy texture on the GPU, at the revision it was uploaded from.
struct DynamicPage {
    revision: u64,
    texture: Bound,
    group: wgpu::BindGroup,
}

/// A slot no texture fills: [`SpritePass`] binds its one transparent texel there.
pub const NO_TEXTURE: usize = usize::MAX;

/// The screen-space pipeline and the frame's vertices.
pub struct SpritePass {
    native: super::native_shader::stream::Stream,
    native_draws: Vec<super::native_shader::stream::Submission>,
    native_format: wgpu::TextureFormat,
    native_samples: u32,
    bakes: Vec<CachedBake>,
    bake_stream: super::native_shader::stream::Stream,
    /// Refusals of the most recent frame, by effect/path/compiler reason.
    pub native_refusals: std::collections::BTreeMap<String, u64>,
    pipeline: wgpu::RenderPipeline,
    bind_groups: Vec<wgpu::BindGroup>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// [`Exact`] quads: blended (`src_alpha`, `inv_src_alpha`) and not.
    #[cfg(test)]
    exact_alpha: wgpu::RenderPipeline,
    #[cfg(test)]
    exact_off: wgpu::RenderPipeline,
    #[cfg(test)]
    exact_layout: wgpu::BindGroupLayout,
    /// The static pages' views, which exact bind groups combine.
    views: Vec<wgpu::TextureView>,
    /// One transparent texel for an empty slot.
    empty: Bound,
    /// Exact bind groups by their slots, rebuilt when the lazy textures change.
    #[cfg(test)]
    exact_groups: HashMap<Slots, wgpu::BindGroup>,
    /// The content's lazy textures, mirrored into `dynamic` by [`SpritePass::sync`].
    lazy: Arc<LazyTextures>,
    synced: Option<u64>,
    dynamic: HashMap<usize, DynamicPage>,
    /// Lazy textures the device refused, at the revision it refused.
    failed: HashMap<usize, u64>,
    vertices: wgpu::Buffer,
    capacity: u64,
    #[cfg(test)]
    exact_vertices: wgpu::Buffer,
    #[cfg(test)]
    exact_capacity: u64,
    runs: Vec<Run>,
    count: u32,
    exact_count: u32,
}

/// Vertices a freshly built pass can hold before it grows; six per quad.
const INITIAL_VERTICES: u64 = 6 * 4096;

impl SpritePass {
    /// Builds the pipeline over already-uploaded icon pages.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
        textures: &IconTextures,
        lazy: &Arc<LazyTextures>,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite texture"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(1),
            ],
        });
        let bind_groups = textures.bind_all(device, &layout, textures.screen_clamp());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprites"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let target = |blend| {
            [Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::COLOR,
            })]
        };
        let legacy_targets = target(Some(wgpu::BlendState::ALPHA_BLENDING));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4,
                        4 => Float32, 5 => Float32
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            // The icon layer is drawn last and never tests depth (R2 5.5: the
            // priorities are a sort order inside the layer, not a depth test).
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &legacy_targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        #[cfg(test)]
        let exact_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exact sprite textures"),
            entries: &[0, 1, 2].map(|binding| {
                texture_entry(
                    binding,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                )
            }),
        });
        #[cfg(test)]
        let exact_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("exact sprites"),
            source: wgpu::ShaderSource::Wgsl(EXACT_SHADER.into()),
        });
        #[cfg(test)]
        let exact_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exact sprites"),
                bind_group_layouts: &[Some(&exact_layout)],
                immediate_size: 0,
            });
        #[cfg(test)]
        let exact = |label, blend: Option<wgpu::BlendState>| {
            let targets = target(blend);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&exact_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &exact_module,
                    entry_point: Some("exact_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ExactVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4,
                            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
                            8 => Uint32, 9 => Uint32
                        ],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &exact_module,
                    entry_point: Some("exact_fragment"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // `src_alpha` over `inv_src_alpha` on colour; the write mask keeps the target's alpha,
        // so an effect's alpha operation (`text.shader`'s `BlendOpAlpha = max`) writes nothing.
        #[cfg(test)]
        let exact_alpha = exact("exact sprites", Some(wgpu::BlendState::ALPHA_BLENDING));
        #[cfg(test)]
        let exact_off = exact("exact sprites, no blending", None);
        let empty = upload(
            device,
            queue,
            "empty sprite slot",
            &dds::Texture::from_rgba8(installed_content::image::Rgba8Image {
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }),
            ColorSpace::Linear,
        )?;
        Ok(Self {
            native: Default::default(),
            native_draws: Vec::new(),
            bakes: Vec::new(),
            bake_stream: Default::default(),
            native_format: format,
            native_samples: samples,
            native_refusals: Default::default(),
            pipeline,
            bind_groups,
            layout,
            sampler: textures.screen_clamp().clone(),
            #[cfg(test)]
            exact_alpha,
            #[cfg(test)]
            exact_off,
            #[cfg(test)]
            exact_layout,
            views: textures
                .bound
                .iter()
                .map(|texture| texture.view.clone())
                .collect(),
            empty,
            #[cfg(test)]
            exact_groups: HashMap::new(),
            lazy: Arc::clone(lazy),
            synced: None,
            dynamic: HashMap::new(),
            failed: HashMap::new(),
            vertices: buffer(device, INITIAL_VERTICES, std::mem::size_of::<Vertex>()),
            capacity: INITIAL_VERTICES,
            #[cfg(test)]
            exact_vertices: buffer(device, INITIAL_VERTICES, std::mem::size_of::<ExactVertex>()),
            #[cfg(test)]
            exact_capacity: INITIAL_VERTICES,
            runs: Vec::new(),
            count: 0,
            exact_count: 0,
        })
    }

    /// Mirrors the content's lazy textures: uploads new and changed ones and
    /// drops evicted ones. Costs one atomic read when nothing changed.
    ///
    /// A texture the device refuses is warned about once and left undrawn,
    /// rather than failing every frame's render; a later revision of it is
    /// tried again.
    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let version = self.lazy.version();
        if self.synced == Some(version) {
            return;
        }
        let resident = self.lazy.snapshot();
        let uploaded: HashMap<usize, u64> = self
            .dynamic
            .iter()
            .map(|(id, page)| (*id, page.revision))
            .collect();
        let revisions: Vec<(usize, u64)> = resident
            .iter()
            .map(|(id, entry)| (*id, entry.revision))
            .collect();
        let plan = plan_sync(&uploaded, &self.failed, &revisions);
        for id in &plan.drop {
            self.dynamic.remove(id);
            self.failed.remove(id);
        }
        for (id, entry) in resident {
            if !plan.upload.contains(&id) {
                continue;
            }
            let texture = match upload(
                device,
                queue,
                &format!("lazy icon {}", id - DYNAMIC_BASE),
                &entry.texture,
                ColorSpace::Linear,
            ) {
                Ok(texture) => texture,
                Err(error) => {
                    if self.failed.insert(id, entry.revision).is_none() {
                        eprintln!("Warning: an icon texture is not drawn: {error}");
                    }
                    self.dynamic.remove(&id);
                    continue;
                }
            };
            self.failed.remove(&id);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lazy icon"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.dynamic.insert(
                id,
                DynamicPage {
                    revision: entry.revision,
                    texture,
                    group,
                },
            );
        }
        // A combination may name a page that changed or left.
        #[cfg(test)]
        self.exact_groups.clear();
        self.synced = Some(version);
    }

    /// The bind group of a static page or a resident lazy texture.
    fn group(&self, texture: usize) -> Option<&wgpu::BindGroup> {
        self.bind_groups
            .get(texture)
            .or_else(|| self.dynamic.get(&texture).map(|page| &page.group))
    }

    /// The view of a static page or a resident lazy texture, the empty texel for no texture.
    fn view(&self, texture: usize) -> Option<&wgpu::TextureView> {
        if texture == NO_TEXTURE {
            return Some(&self.empty.view);
        }
        self.views
            .get(texture)
            .or_else(|| self.dynamic.get(&texture).map(|page| &page.texture.view))
    }

    /// Makes the exact bind group of `slots`; `false` when a slot's texture is not resident.
    #[cfg(test)]
    fn exact_group(&mut self, device: &wgpu::Device, slots: Slots) -> bool {
        if self.exact_groups.contains_key(&slots) {
            return true;
        }
        let Some(views) = slots
            .iter()
            .map(|slot| self.view(*slot).cloned())
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        let entries: Vec<wgpu::BindGroupEntry> = views
            .iter()
            .enumerate()
            .map(|(binding, view)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exact sprite textures"),
            layout: &self.exact_layout,
            entries: &entries,
        });
        self.exact_groups.insert(slots, group);
        true
    }

    /// Turns one frame's quads into vertices, grouped by texture. Compiled GUI draws upload
    /// their constants and vertices through `arena`, the frame's one
    /// [`shader_pipeline::gpu::Arena`] the map passes share; the caller flushes it before
    /// submitting the commands that draw them and resets it before the next frame.
    ///
    /// `batch` is in interface pixels and `ui_scale` device pixels per
    /// interface pixel; see [`device_corners`] for the snapping and
    /// [`filter_mode`] for the sampling. [`Exact`] quads are not snapped, keep their push
    /// order within their layer and are cut by their scissor.
    #[allow(clippy::too_many_arguments)]
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
    ) {
        self.runs.clear();
        self.native_draws.clear();
        for bake in &mut self.bakes {
            bake.used = false;
        }
        self.native_refusals.clear();
        self.native.refused.clear();
        self.native.invalidate_bindings();
        self.count = 0;
        self.exact_count = 0;
        // The lazy textures this frame draws stay pinned in their LRU.
        self.lazy.mark_drawn(
            batch
                .quads
                .iter()
                .flat_map(|quad| {
                    let exact = quad
                        .exact
                        .as_ref()
                        .map_or([None, None], |exact| [exact.second, exact.masking]);
                    let native = quad
                        .exact
                        .as_ref()
                        .and_then(|e| e.native.as_ref())
                        .into_iter()
                        .flat_map(|draw| {
                            draw.textures
                                .iter()
                                .chain(
                                    draw.bake
                                        .iter()
                                        .flat_map(|b| b.draws.iter().flat_map(|d| &d.textures)),
                                )
                                .filter_map(|(_, id)| *id)
                        });
                    std::iter::once(Some(quad.texture))
                        .chain(exact)
                        .flatten()
                        .chain(native)
                })
                .filter(|texture| *texture >= DYNAMIC_BASE)
                .collect::<std::collections::BTreeSet<_>>(),
        );
        if batch.is_empty() || size[0] == 0 || size[1] == 0 {
            self.bakes.clear();
            return;
        }
        let mut order: Vec<usize> = (0..batch.quads.len()).collect();
        // counters: `(layer, texture)`, so a page batch never overtakes an
        // icon child that has to draw under or over another. An exact quad keeps its push
        // order in its layer: the game's GUI draws in attach order, whatever the texture.
        order.sort_by_key(|&index| {
            let quad = &batch.quads[index];
            (
                quad.layer,
                if quad.exact.is_some() {
                    0
                } else {
                    quad.texture
                },
            )
        });
        let width = size[0] as f32;
        let height = size[1] as f32;
        let clip = |x: f32, y: f32| [x / width * 2.0 - 1.0, 1.0 - y / height * 2.0];
        let mut vertices: Vec<Vertex> = Vec::with_capacity(order.len() * 6);
        #[cfg(test)]
        let mut exact_vertices: Vec<ExactVertex> = Vec::new();
        let filter = filter_mode(ui_scale);
        for index in order {
            let quad = batch.quads[index].clone();
            if let Some(exact) = quad.exact {
                if let Some(native) = &exact.native {
                    match self.prepare_native(device, queue, arena, native, size, ui_scale) {
                        Ok(submission) => {
                            let scissor = match exact.scissor {
                                Some(clip) => match device_scissor(clip, ui_scale, size) {
                                    Some(clip) => Some(clip),
                                    None => continue,
                                },
                                None => None,
                            };
                            let index = self.native_draws.len();
                            self.native_draws.push(submission);
                            self.runs.push(Run {
                                pipe: Pipe::Native(index),
                                texture: NO_TEXTURE,
                                #[cfg(test)]
                                slots: [NO_TEXTURE; 3],
                                scissor,
                                range: 0..0,
                            });
                        }
                        // The refused draw left the binds the game makes for it, less what the
                        // replay lacks ([`prepare_draw`]).
                        Err(reason) => {
                            if !self.native_refusals.contains_key(&reason) {
                                eprintln!("{reason}");
                            }
                            *self.native_refusals.entry(reason).or_default() += 1;
                        }
                    }
                    continue;
                }
                #[cfg(not(test))]
                {
                    // What the game binds for it is unknown here.
                    self.native.invalidate_bindings();
                    *self
                        .native_refusals
                        .entry("GUI quad has no compiled effect payload".into())
                        .or_default() += 1;
                    continue;
                }
                #[cfg(test)]
                {
                    self.native.invalidate_bindings();
                    let slots = [
                        quad.texture,
                        exact.second.unwrap_or(NO_TEXTURE),
                        exact.masking.unwrap_or(NO_TEXTURE),
                    ];
                    if !self.exact_group(device, slots) {
                        continue;
                    }
                    let scissor = match exact.scissor {
                        Some(scissor) => match device_scissor(scissor, ui_scale, size) {
                            Some(device) => Some(device),
                            None => continue,
                        },
                        None => None,
                    };
                    let vertex = |corner: usize| {
                        let [x, y] = exact.corners[corner];
                        let uv = exact.uv[corner];
                        ExactVertex {
                            position: clip(x * ui_scale, y * ui_scale),
                            uv,
                            // `saturate(vTexCoord * 1000)` per vertex (buttonstate's `MASKING`).
                            mask_uv: uv.map(|value| (value * 1000.0).clamp(0.0, 1.0)),
                            color: quad.color,
                            second: exact.second_color,
                            offsets: exact.offsets,
                            inputs: exact.inputs,
                            lod_bias: [
                                exact.lod_bias[0],
                                exact.lod_bias[1],
                                exact.lod_bias[2],
                                0.0,
                            ],
                            pixel: exact.pixel,
                            samplers: exact.samplers,
                        }
                    };
                    let start = exact_vertices.len() as u32;
                    exact_vertices.extend([0, 1, 2, 0, 2, 3].map(vertex));
                    let end = exact_vertices.len() as u32;
                    let pipe = Pipe::Exact(exact.blend);
                    match self.runs.last_mut() {
                        Some(run)
                            if run.pipe == pipe
                                && run.slots == slots
                                && run.scissor == scissor
                                && run.range.end == start =>
                        {
                            run.range.end = end;
                        }
                        _ => self.runs.push(Run {
                            pipe,
                            texture: quad.texture,
                            slots,
                            scissor,
                            range: start..end,
                        }),
                    }
                    continue;
                }
            }
            self.native.invalidate_bindings();
            if self.group(quad.texture).is_none() {
                continue;
            }
            let [x0, y0, x1, y1] = device_corners(quad.rect, ui_scale);
            // counters: a rotated quad turns about its own centre, with `y`
            // down, so a positive angle is clockwise on screen.
            let spin = (quad.rotation != 0.0).then(|| {
                let (sin, cos) = quad.rotation.sin_cos();
                ([(x0 + x1) * 0.5, (y0 + y1) * 0.5], sin, cos)
            });
            let corner = |x: f32, y: f32, u: f32, v: f32| {
                let (x, y) = match spin {
                    Some((centre, sin, cos)) => {
                        let (dx, dy) = (x - centre[0], y - centre[1]);
                        (
                            centre[0] + dx * cos - dy * sin,
                            centre[1] + dx * sin + dy * cos,
                        )
                    }
                    None => (x, y),
                };
                Vertex {
                    position: clip(x, y),
                    uv: [u, v],
                    color: quad.color,
                    cell: quad.uv,
                    filter,
                    shade: quad.shade,
                }
            };
            let [u0, v0, u1, v1] = quad.uv;
            let start = vertices.len() as u32;
            vertices.extend([
                corner(x0, y0, u0, v0),
                corner(x1, y0, u1, v0),
                corner(x1, y1, u1, v1),
                corner(x0, y0, u0, v0),
                corner(x1, y1, u1, v1),
                corner(x0, y1, u0, v1),
            ]);
            let end = vertices.len() as u32;
            match self.runs.last_mut() {
                Some(run)
                    if run.pipe == Pipe::Legacy
                        && run.texture == quad.texture
                        && run.range.end == start =>
                {
                    run.range.end = end
                }
                _ => self.runs.push(Run {
                    pipe: Pipe::Legacy,
                    texture: quad.texture,
                    #[cfg(test)]
                    slots: [NO_TEXTURE; 3],
                    scissor: None,
                    range: start..end,
                }),
            }
        }
        self.bakes.retain(|bake| bake.used);
        self.count = vertices.len() as u32;
        #[cfg(test)]
        {
            self.exact_count = exact_vertices.len() as u32;
        }
        if self.count > 0 {
            if u64::from(self.count) > self.capacity {
                self.capacity = u64::from(self.count).next_power_of_two();
                self.vertices = buffer(device, self.capacity, std::mem::size_of::<Vertex>());
            }
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
        }
        #[cfg(test)]
        if self.exact_count > 0 {
            if u64::from(self.exact_count) > self.exact_capacity {
                self.exact_capacity = u64::from(self.exact_count).next_power_of_two();
                self.exact_vertices = buffer(
                    device,
                    self.exact_capacity,
                    std::mem::size_of::<ExactVertex>(),
                );
            }
            queue.write_buffer(
                &self.exact_vertices,
                0,
                bytemuck::cast_slice(&exact_vertices),
            );
        }
    }

    /// Draws the uploaded quads over whatever the target already holds.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets) {
        if self.runs.is_empty() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
            color_attachments: &[Some(targets.output_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let [width, height] = targets.size();
        self.draw_runs(&mut pass, [width, height]);
    }

    fn draw_runs(&self, pass: &mut wgpu::RenderPass<'_>, [width, height]: [u32; 2]) {
        let mut bound = None;
        for run in &self.runs {
            let [x, y, w, h] = run.scissor.unwrap_or([0, 0, width, height]);
            pass.set_scissor_rect(x, y, w, h);
            if let Pipe::Native(index) = run.pipe {
                let submission = &self.native_draws[index];
                submission.pipeline.draw(pass, &submission.draw);
                bound = None;
                continue;
            }
            if bound != Some(run.pipe) {
                match run.pipe {
                    Pipe::Legacy => {
                        pass.set_pipeline(&self.pipeline);
                        pass.set_vertex_buffer(0, self.vertices.slice(..));
                    }
                    #[cfg(test)]
                    Pipe::Exact(blend) => {
                        pass.set_pipeline(match blend {
                            super::native_effects::Blend::Alpha => &self.exact_alpha,
                            super::native_effects::Blend::Off => &self.exact_off,
                        });
                        pass.set_vertex_buffer(0, self.exact_vertices.slice(..));
                    }
                    Pipe::Native(_) => unreachable!(),
                }
                bound = Some(run.pipe);
            }
            let [x, y, w, h] = run.scissor.unwrap_or([0, 0, width, height]);
            pass.set_scissor_rect(x, y, w, h);
            let group = match run.pipe {
                Pipe::Legacy => self.group(run.texture),
                #[cfg(test)]
                Pipe::Exact(_) => self.exact_groups.get(&run.slots),
                Pipe::Native(_) => unreachable!(),
            };
            let Some(group) = group else {
                continue;
            };
            pass.set_bind_group(0, group, &[]);
            pass.draw(run.range.clone(), 0..1);
        }
    }

    /// Quads the last upload produced, for reporting.
    pub fn quad_count(&self) -> u32 {
        (self.count + self.exact_count) / 6 + self.native_draws.len() as u32
    }

    fn prepare_native(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        draw: &super::native_shader::draw::Quad,
        size: [u32; 2],
        scale: f32,
    ) -> Result<super::native_shader::stream::Submission, String> {
        let baked = draw.bake.as_ref().map(|bake| {
            self.prepare_bake(device, queue, arena, bake)
                .map_err(|why| {
                    format!(
                        "effect {:?}, text sprite {:?}: {why}",
                        draw.effect.asset.source.effect, draw.path
                    )
                })
        });
        let views = self.native_views(draw);
        prepare_draw(
            &mut self.native,
            arena,
            draw,
            &views,
            baked,
            super::native_shader::draw::gui_viewport(size, scale),
            self.native_format,
            self.native_samples,
        )
    }

    /// Each slot `draw` binds: its texture, `None` for a null bind, `Err` naming a texture that
    /// is not resident.
    fn native_views(&self, draw: &super::native_shader::draw::Quad) -> Vec<(u32, View)> {
        draw.textures
            .iter()
            .map(|&(slot, id)| {
                (
                    slot,
                    id.map(|id| {
                        self.view(id).cloned().ok_or_else(|| {
                            format!(
                                "effect {:?}, draw {:?}: texture {id} is not resident",
                                draw.effect.asset.source.effect, draw.path
                            )
                        })
                    })
                    .transpose(),
                )
            })
            .collect()
    }

    fn prepare_bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        bake: &Arc<super::native_shader::draw::Bake>,
    ) -> Result<wgpu::TextureView, String> {
        // Changed lazy pages invalidate a bake even if its layout is unchanged. The bake
        // inherits the caller's retained binds, so only a bake that reads nothing it does not
        // bind itself gives the same pixels wherever it is drawn.
        let revision = self.lazy.version();
        let stable = bake.draws.iter().all(self_contained);
        if stable
            && let Some(cached) = self
                .bakes
                .iter_mut()
                .find(|c| c.revision == revision && c.source == *bake)
        {
            cached.used = true;
            return Ok(cached.view.clone());
        }
        let [width, height] = bake.size;
        if width == 0 || height == 0 || width.max(height) > device.limits().max_texture_dimension_2d
        {
            return Err(format!(
                "text bake target {width}x{height} exceeds device limits"
            ));
        }
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CTextSprite bake14228C8D0"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.bake_stream.inherit_bindings(&self.native);
        let mut submissions = Vec::new();
        for draw in &bake.draws {
            let views = self.native_views(draw);
            submissions.push(prepare_draw(
                &mut self.bake_stream,
                arena,
                draw,
                &views,
                None,
                bake.size,
                format,
                1,
            )?);
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text background then alpha MAX glyphs"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for submission in &submissions {
                submission.pipeline.draw(&mut pass, &submission.draw);
            }
        }
        // The bake is submitted before the frame: its uploads are written first.
        arena.flush(queue);
        queue.submit([encoder.finish()]);
        self.bakes.push(CachedBake {
            source: bake.clone(),
            revision,
            view: view.clone(),
            used: true,
        });
        Ok(view)
    }
}

struct CachedBake {
    source: Arc<super::native_shader::draw::Bake>,
    revision: u64,
    view: wgpu::TextureView,
    used: bool,
}

/// Whether `draw`'s compiled program reads only what the draw binds itself: its technique is
/// declared, and every constant buffer, texture and sampler it reflects is one of its own binds
/// (with the effect's own sampler state), none retained from an earlier draw.
fn self_contained(draw: &super::native_shader::draw::Quad) -> bool {
    use super::native_shader::binding;
    use installed_content::pdxshader::program::Stage;
    let Ok(Some(selection)) = draw.effect.asset.select(&draw.effect.technique) else {
        return false;
    };
    let Ok(compiled) = selection.effect.compiled(selection.index) else {
        return false;
    };
    let textures: Vec<u32> = draw
        .textures
        .iter()
        .map(|(slot, _)| binding(*slot, Stage::Pixel))
        .collect();
    let mut buffers = vec![binding(0, Stage::Vertex)];
    if draw.pixel_constants {
        buffers.push(binding(0, Stage::Pixel));
    }
    for (slot, _, _) in &draw.extra_constants {
        buffers.extend([binding(*slot, Stage::Vertex), binding(*slot, Stage::Pixel)]);
    }
    let source = &selection.effect.source;
    [&compiled.vertex, &compiled.pixel]
        .into_iter()
        .all(|shader| {
            shader.buffers.iter().all(|b| buffers.contains(&b.binding))
                && shader.textures.iter().all(|(b, _)| textures.contains(b))
                && shader
                    .samplers
                    .iter()
                    .all(|b| textures.contains(b) && source.sampler(b / 2).is_some())
        })
}

/// A slot's texture as the replay holds it: `Ok(None)` a null bind, `Err` one it does not hold.
type View = Result<Option<wgpu::TextureView>, String>;

/// Makes `draw`'s binds on `native` in the game's order, then prepares it. A draw the replay
/// cannot make is refused with its first reason (its bake, a texture that is not resident, a
/// technique that does not compile, its blend state), but still leaves the binds the game makes
/// for it: its program record (`shader_program_binder@142400520`: a declared technique binds
/// its record even when its shaders failed, `142344DF0`; an undeclared one binds nothing), its
/// constants, and its textures and samplers (`1424005A0`). What the replay lacks of them (the
/// program of a technique it could not compile, a texture it does not hold) is forgotten, so a
/// later draw that reads it is refused rather than given an earlier bind.
#[allow(clippy::too_many_arguments)]
fn prepare_draw(
    native: &mut super::native_shader::stream::Stream,
    arena: &shader_pipeline::gpu::Arena,
    draw: &super::native_shader::draw::Quad,
    views: &[(u32, View)],
    baked: Option<Result<wgpu::TextureView, String>>,
    viewport: [u32; 2],
    format: wgpu::TextureFormat,
    samples: u32,
) -> Result<super::native_shader::stream::Submission, String> {
    use installed_content::pdxshader::program::Stage;
    let label = format!(
        "{} / {}:{}",
        draw.path, draw.effect.asset.source.effect, draw.effect.technique
    );
    let mut refused = baked
        .as_ref()
        .and_then(|baked| baked.as_ref().err().cloned());
    if refused.is_none() {
        refused = views
            .iter()
            .find_map(|(_, view)| view.as_ref().err().cloned());
    }
    let selection = draw.effect.asset.select(&draw.effect.technique);
    // A declared technique's record carries the effect's samplers, compiled or not.
    let declared = !matches!(selection, Ok(None));
    match selection {
        Ok(selection) => {
            if let Err(why) = native.bind_effect(selection, draw.declaration) {
                refused.get_or_insert(format!("native GUI draw {label:?}: {why}"));
            }
        }
        Err(why) => {
            native.forget_program();
            refused.get_or_insert(why);
        }
    }
    if draw.default_blend {
        native.bind_blend(None, wgpu::ColorWrites::ALL);
    }
    let constants = draw.projected_constants(viewport);
    let known = [std::ops::Range {
        start: 0,
        end: draw.known,
    }];
    for stage in [Stage::Vertex, Stage::Pixel] {
        if stage == Stage::Pixel && !draw.pixel_constants {
            continue;
        }
        // The whole native allocation, of which the draw writes only `known` bytes: a read of
        // the rest is refused by name where the draw is prepared (`Stream::check_constants`),
        // through retained and null-technique binds alike.
        native.bind_partial_constants(0, stage, Some(bytemuck::cast_slice(&constants)), &known);
    }
    // Animation records' b1: the whole allocation, known only where the game writes it.
    for (slot, constants, known) in &draw.extra_constants {
        for stage in [Stage::Vertex, Stage::Pixel] {
            native.bind_partial_constants(
                *slot,
                stage,
                Some(bytemuck::cast_slice(constants)),
                known,
            );
        }
    }
    for (slot, view) in views {
        let slot = *slot;
        let view = match (&baked, slot) {
            (Some(baked), 0) => baked.clone().map(Some),
            _ => view.clone(),
        };
        let sampler = declared
            .then(|| draw.effect.asset.source.sampler(slot))
            .flatten();
        match view {
            Ok(view) => native.bind_texture(slot, Stage::Pixel, view, sampler),
            Err(_) => native.forget_texture(slot, Stage::Pixel, sampler),
        }
    }
    if let Some(reason) = refused {
        return Err(reason);
    }
    native.prepare(
        arena,
        &label,
        &draw.vertices,
        draw.stride,
        draw.vertices.len() / draw.stride,
        draw.topology,
        format,
        samples,
    )
}

/// What [`SpritePass::sync`] does with the lazy textures.
#[derive(Debug, Default, PartialEq, Eq)]
struct SyncPlan {
    /// Uploaded or refused textures that are no longer resident.
    drop: Vec<usize>,
    /// Resident textures to upload: new, changed, or a new revision of one
    /// the device refused. A refused revision is not tried again.
    upload: Vec<usize>,
}

fn plan_sync(
    uploaded: &HashMap<usize, u64>,
    failed: &HashMap<usize, u64>,
    resident: &[(usize, u64)],
) -> SyncPlan {
    let live: std::collections::HashSet<usize> = resident.iter().map(|(id, _)| *id).collect();
    let mut drop: Vec<usize> = uploaded
        .keys()
        .chain(failed.keys())
        .filter(|id| !live.contains(id))
        .copied()
        .collect();
    drop.sort_unstable();
    drop.dedup();
    let upload = resident
        .iter()
        .filter(|(id, revision)| {
            uploaded.get(id) != Some(revision) && failed.get(id) != Some(revision)
        })
        .map(|(id, _)| *id)
        .collect();
    SyncPlan { drop, upload }
}

fn buffer(device: &wgpu::Device, vertices: u64, stride: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite vertices"),
        size: vertices * stride as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The whole pass: positions arrive in clip space, so there is no transform.
pub(super) const SHADER: &str = r#"
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) cell: vec4<f32>,
    @location(3) @interpolate(flat) sampling: f32,
    @location(4) @interpolate(flat) shade: f32,
}

@group(0) @binding(0) var page: texture_2d<f32>;
@group(0) @binding(1) var page_sampler: sampler;

@vertex
fn vertex_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) cell: vec4<f32>,
    @location(4) sampling: f32,
    @location(5) shade: f32,
) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    out.cell = cell;
    out.sampling = sampling;
    out.shade = shade;
    return out;
}

// `sample_window` in sprites.rs: the cell inset by half a texel.
fn window(cell: vec4<f32>, texels: vec2<f32>) -> vec4<f32> {
    let half = 0.5 / texels;
    let low = min(cell.xy, cell.zw);
    let high = max(cell.xy, cell.zw);
    let middle = (low + high) * 0.5;
    return vec4<f32>(min(low + half, middle), max(high - half, middle));
}

// `filter_mode` in sprites.rs: 0 one plain tap; 1 one tap clamped to the
// cell's window; 2 the texels a minified pixel covers, up to 4 x 4, each
// clamped. The derivatives are taken before any branch.
fn texel(uv: vec2<f32>, cell: vec4<f32>, sampling: f32) -> vec4<f32> {
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    if (sampling < 0.5) {
        return textureSampleLevel(page, page_sampler, uv, 0.0);
    }
    let texels = vec2<f32>(textureDimensions(page, 0));
    let bounds = window(cell, texels);
    var taps = 1u;
    if (sampling > 1.5) {
        let footprint = max(abs(dx), abs(dy)) * texels;
        taps = u32(clamp(ceil(max(footprint.x, footprint.y) - 0.001), 1.0, 4.0));
    }
    var sum = vec4<f32>(0.0);
    for (var j = 0u; j < taps; j = j + 1u) {
        for (var i = 0u; i < taps; i = i + 1u) {
            let offset = (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(taps) - 0.5;
            let at = clamp(uv + dx * offset.x + dy * offset.y, bounds.xy, bounds.zw);
            sum = sum + textureSampleLevel(page, page_sampler, at, 0.0);
        }
    }
    return sum / f32(taps * taps);
}

// The atlases are authored straight, not premultiplied (R2 5.3), so the colour
// multiplies both parts and the blend state does the compositing. A negative
// alpha selects `buttonstate.shader`'s `PixelShaderDisable`: the texel's
// luminance grey, then the colour with its alpha negated back. A shade is its
// Over (positive) or Down (negative) technique, applied after the colour.
@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sample = texel(in.uv, in.cell, in.sampling);
    if (in.color.a < 0.0) {
        let grey = dot(sample.rgb, vec3<f32>(0.212671, 0.715160, 0.072169));
        return vec4<f32>(grey, grey, grey, sample.a) * vec4<f32>(in.color.rgb, -in.color.a);
    }
    let out = sample * in.color;
    return vec4<f32>(out.rgb + (0.5 + out.rgb) * in.shade, out.a);
}
"#;

/// The [`Exact`] pass: the modelled programs of [`super::native_effects`], each texel read and
/// filtered as the effect's sampler state reads it (every addressing mode, the magnifying or
/// minifying filter by the level of detail, the mip filter over the texture's levels).
#[cfg(test)]
pub(super) const EXACT_SHADER: &str = r#"
struct ExactOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) mask_uv: vec2<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) second: vec4<f32>,
    @location(4) @interpolate(flat) offsets: vec4<f32>,
    @location(5) @interpolate(flat) inputs: vec4<f32>,
    @location(6) @interpolate(flat) lod_bias: vec4<f32>,
    @location(7) @interpolate(flat) pixel: u32,
    @location(8) @interpolate(flat) samplers: u32,
}

@group(0) @binding(0) var slot0: texture_2d<f32>;
@group(0) @binding(1) var slot1: texture_2d<f32>;
@group(0) @binding(2) var slot5: texture_2d<f32>;

@vertex
fn exact_vertex(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) mask_uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) second: vec4<f32>,
    @location(5) offsets: vec4<f32>,
    @location(6) inputs: vec4<f32>,
    @location(7) lod_bias: vec4<f32>,
    @location(8) pixel: u32,
    @location(9) samplers: u32,
) -> ExactOut {
    var out: ExactOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.mask_uv = mask_uv;
    out.color = color;
    out.second = second;
    out.offsets = offsets;
    out.inputs = inputs;
    out.lod_bias = lod_bias;
    out.pixel = pixel;
    out.samplers = samplers;
    return out;
}

// A texel index on one axis: wrap, mirror, clamp, border (-1: transparent black, the only border
// colour `lower_d3d11@1423FD550` sets), mirror once.
fn address(c: i32, size: i32, mode: u32) -> i32 {
    switch mode {
        case 0u: {
            return ((c % size) + size) % size;
        }
        case 1u: {
            let period = 2 * size;
            let m = ((c % period) + period) % period;
            return select(m, period - 1 - m, m >= size);
        }
        case 2u: {
            return clamp(c, 0, size - 1);
        }
        case 3u: {
            return select(c, -1, c < 0 || c >= size);
        }
        default: {
            return min(select(c, -c - 1, c < 0), size - 1);
        }
    }
}

fn fetch(t: texture_2d<f32>, at: vec2<i32>, level: i32, state: u32) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t, level));
    let x = address(at.x, size.x, state & 7u);
    let y = address(at.y, size.y, (state >> 3u) & 7u);
    if (x < 0 || y < 0) {
        return vec4<f32>(0.0);
    }
    return textureLoad(t, vec2<i32>(x, y), level);
}

fn filtered(t: texture_2d<f32>, uv: vec2<f32>, level: i32, linear: bool, state: u32) -> vec4<f32> {
    let at = uv * vec2<f32>(textureDimensions(t, level));
    if (!linear) {
        return fetch(t, vec2<i32>(floor(at)), level, state);
    }
    let p = at - 0.5;
    let base = floor(p);
    let f = p - base;
    let i = vec2<i32>(base);
    let a = fetch(t, i, level, state);
    let b = fetch(t, i + vec2<i32>(1, 0), level, state);
    let c = fetch(t, i + vec2<i32>(0, 1), level, state);
    let d = fetch(t, i + vec2<i32>(1, 1), level, state);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// `state` (ten bits): the addressing, the magnifying and minifying filters (bits 6 and 7), and
// the mip filter (bits 8-9: 1 point, 2 linear). `lod` already carries the slot's bias.
fn sample_slot(t: texture_2d<f32>, uv: vec2<f32>, state: u32, lod: f32) -> vec4<f32> {
    let magnified = lod <= 0.0;
    let linear = select(((state >> 7u) & 1u) == 1u, ((state >> 6u) & 1u) == 1u, magnified);
    let mip = (state >> 8u) & 3u;
    let top = f32(textureNumLevels(t) - 1u);
    if (magnified) {
        return filtered(t, uv, 0, linear, state);
    }
    let level = clamp(lod, 0.0, top);
    if (mip == 1u) {
        return filtered(t, uv, i32(floor(level + 0.5)), linear, state);
    }
    let low = floor(level);
    let high = min(low + 1.0, top);
    return mix(
        filtered(t, uv, i32(low), linear, state),
        filtered(t, uv, i32(high), linear, state),
        level - low,
    );
}

fn lod_of(t: texture_2d<f32>, dx: vec2<f32>, dy: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(t, 0));
    let a = dx * size;
    let b = dy * size;
    return 0.5 * log2(max(dot(a, a), dot(b, b)));
}

// buttonstate's timed Over (k 4, +) and Down (k 16, -) shade.
fn timed(c: vec4<f32>, since: f32, k: f32, sign: f32) -> vec4<f32> {
    var t = 0.9 - saturate(since * k);
    t = t * t;
    t = 0.9 * 0.9 - t;
    return vec4<f32>(c.rgb + sign * (0.5 + c.rgb) * (vec3<f32>(0.15) * t), c.a);
}

fn sprite(in: ExactOut, p: u32, lods: vec3<f32>) -> vec4<f32> {
    let s0 = in.samplers & 1023u;
    let s5 = (in.samplers >> 20u) & 1023u;
    var offset = in.offsets.xy;
    if (((p >> 5u) & 1u) == 1u) {
        offset = vec2<f32>(in.offsets.x, 0.0);
    }
    let uv = in.uv + offset;
    var c = sample_slot(slot0, uv, s0, lods.x);
    if (((p >> 4u) & 1u) == 1u) {
        let next = sample_slot(slot0, in.uv + in.offsets.zw, s0, lods.x);
        c = mix(c, next, in.inputs.y);
    }
    if (((p >> 6u) & 1u) == 1u) {
        let grey = dot(c.rgb, vec3<f32>(0.212671, 0.715160, 0.072169));
        c = vec4<f32>(grey, grey, grey, c.a);
    }
    if (((p >> 14u) & 1u) == 1u) {
        var at = uv;
        if (((p >> 15u) & 1u) == 1u) {
            at = in.mask_uv;
        }
        c.a = c.a * sample_slot(slot5, at, s5, lods.z).a;
    }
    if (((p >> 12u) & 1u) == 1u) {
        let t = in.inputs.y;
        c = c * (min(t / 0.05, 1.0) * min((1.0 - t) / 0.1, 1.0));
    }
    if (((p >> 7u) & 1u) == 1u) {
        c = c * in.color;
    }
    switch (p >> 8u) & 7u {
        case 1u: {
            c = timed(c, in.inputs.x, 4.0, 1.0);
        }
        case 2u: {
            c = timed(c, in.inputs.x, 16.0, -1.0);
        }
        case 3u: {
            c = c + vec4<f32>(0.1, 0.1, 0.1, 0.0);
        }
        case 4u: {
            c = c - vec4<f32>(0.1, 0.1, 0.1, 0.0);
        }
        case 5u: {
            c = vec4<f32>(c.rgb + (0.5 + c.rgb) * 0.15, c.a);
        }
        case 6u: {
            c = vec4<f32>(c.rgb - (0.5 + c.rgb) * 0.15, c.a);
        }
        default: {}
    }
    if (((p >> 11u) & 1u) == 1u) {
        c = c * in.color;
    }
    if (((p >> 13u) & 1u) == 1u) {
        c.a = 1.0;
    }
    return c;
}

// The start, middle and stop thirds of the first texture (progress_startend, _reverse).
fn thirds(uv: vec2<f32>, state: f32, s0: u32, lod: f32) -> vec4<f32> {
    var start = uv;
    start.y = start.y / 3.0;
    start.y = start.y + 1.0 / 3.0;
    var middle = start;
    middle.y = middle.y + 1.0 / 3.0;
    var stop = middle;
    stop.y = stop.y + 1.0 / 3.0;
    stop.x = stop.x + 1.0 - state;
    let a = sample_slot(slot0, start, s0, lod);
    let m = sample_slot(slot0, middle, s0, lod);
    let z = sample_slot(slot0, stop, s0, lod);
    var c = mix(m, a, a.a);
    c = mix(c, z, z.a);
    c.a = m.a;
    return c;
}

fn progress(in: ExactOut, p: u32, lods: vec3<f32>) -> vec4<f32> {
    let texture = ((p >> 4u) & 1u) == 1u;
    let variant = (p >> 6u) & 7u;
    var uv = in.uv;
    if (((p >> 5u) & 1u) == 1u) {
        uv.y = -uv.y;
    }
    let state = in.inputs.z;
    let s0 = in.samplers & 1023u;
    let s1 = (in.samplers >> 10u) & 1023u;
    let x = uv.x;
    switch variant {
        case 1u: {
            let first = x >= 1.0 - state;
            if (!texture) {
                return select(in.second, in.color, first);
            }
            if (first) {
                return thirds(uv, state, s0, lods.x);
            }
            return sample_slot(slot1, uv, s1, lods.y);
        }
        case 2u: {
            let first = x <= state;
            if (!texture) {
                return select(in.second, in.color, first);
            }
            if (first) {
                return thirds(uv, state, s0, lods.x);
            }
            return sample_slot(slot1, uv, s1, lods.y);
        }
        case 3u: {
            if (!texture) {
                let front = (x <= 0.5 && x <= state) || (x >= 0.5 && x >= state);
                return select(in.second, in.color, front);
            }
            let front = (x < 0.5 && x >= state) || (x >= 0.5 && x <= state);
            if (front) {
                return sample_slot(slot0, uv, s0, lods.x);
            }
            return sample_slot(slot1, uv, s1, lods.y);
        }
        case 4u: {
            if (!texture) {
                return select(in.second, in.color, x <= state);
            }
            let diff = vec2<f32>(0.5) - uv;
            let angle = atan2(diff.x, diff.y) + 3.14159265;
            let t = saturate((angle - state * 3.14159265 * 2.0) * 50.0);
            return mix(
                sample_slot(slot0, uv, s0, lods.x),
                sample_slot(slot1, uv, s1, lods.y),
                t,
            );
        }
        default: {
            let first = x <= state;
            if (!texture) {
                return select(in.second, in.color, first);
            }
            if (first) {
                return sample_slot(slot0, uv, s0, lods.x);
            }
            return sample_slot(slot1, uv, s1, lods.y);
        }
    }
}

@fragment
fn exact_fragment(in: ExactOut) -> @location(0) vec4<f32> {
    // Derivatives before any branch on the quad's own program.
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let lods = vec3<f32>(lod_of(slot0, dx, dy), lod_of(slot1, dx, dy), lod_of(slot5, dx, dy))
        + in.lod_bias.xyz;
    // A masked flag's slot 0 is read at `FlagCoords.xy + uv * FlagCoords.zw`.
    let flag_uv = in.offsets.xy + in.uv * in.offsets.zw;
    let flag_lod = lod_of(slot0, dpdx(flag_uv), dpdy(flag_uv)) + in.lod_bias.x;
    // circularprogressbar.shader reads both slots at half the vertex coordinate, flipped.
    var circle = (in.uv - vec2<f32>(0.5)) * 0.5 + vec2<f32>(0.25);
    circle.y = 1.0 - circle.y;
    let circle_lods = vec3<f32>(
        lod_of(slot0, dpdx(circle), dpdy(circle)),
        lod_of(slot1, dpdx(circle), dpdy(circle)),
        0.0,
    ) + in.lod_bias.xyz;
    switch in.pixel & 15u {
        case 1u: {
            return progress(in, in.pixel, lods);
        }
        case 3u: {
            // maskedflag.shader: the flag at `FlagCoords.xy + uv * FlagCoords.zw` (carried in
            // `offsets`), the mask's alpha at `uv`.
            var c = sample_slot(slot0, flag_uv, in.samplers & 1023u, flag_lod);
            c.a = sample_slot(slot1, in.uv, (in.samplers >> 10u) & 1023u, lods.y).a;
            switch (in.pixel >> 4u) & 3u {
                case 1u: {
                    c = vec4<f32>(c.rgb + vec3<f32>(0.1), c.a);
                }
                case 2u: {
                    c = vec4<f32>(c.rgb - vec3<f32>(0.1), c.a);
                }
                case 3u: {
                    let grey = dot(c.rgb, vec3<f32>(0.212671, 0.715160, 0.072169));
                    c = vec4<f32>(grey, grey, grey, c.a);
                }
                default: {}
            }
            return c;
        }
        case 4u: {
            // circularprogressbar.shader: `lerp(TextureOne, TextureTwo, vTextureToRender.x)` at
            // `circle - 0.5`, the sector carried in `inputs.z`.
            let at = circle - vec2<f32>(0.5);
            let one = sample_slot(slot0, at, in.samplers & 1023u, circle_lods.x);
            let two = sample_slot(slot1, at, (in.samplers >> 10u) & 1023u, circle_lods.y);
            return mix(one, two, in.inputs.z);
        }
        case 2u: {
            // text.shader: the vertex program forms `vColor * ModColor`; the pixel program
            // multiplies the glyph texel by it.
            return sample_slot(slot0, in.uv, in.samplers & 1023u, lods.x) * (in.color * in.second);
        }
        default: {
            return sprite(in, in.pixel, lods);
        }
    }
}
"#;

/// The quad colour that draws a sprite in the GUI's disabled state (`buttonstate.shader`'s
/// `PixelShaderDisable`, luminance weights `0.212671, 0.715160, 0.072169`); the negative alpha is
/// the flag and its magnitude the multiplier.
pub const DISABLED: [f32; 4] = [1.0, 1.0, 1.0, -1.0];

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(frames: u32, width: u32) -> SpriteSheet {
        SpriteSheet {
            name: "GFX_test".into(),
            texture: 0,
            frames,
            size: [width, 27],
        }
    }

    #[test]
    fn frames_slice_the_strip_evenly() {
        // `GFX_onmap_victorypoints_strip`: 580 x 27, 20 frames of 29 x 27.
        let vp = strip(20, 580);
        assert_eq!(vp.frame_size(), [29.0, 27.0]);
        assert_eq!(vp.frame_uv(0), [0.0, 0.0, 29.0 / 580.0, 1.0]);
        assert_eq!(vp.frame_uv(19), [551.0 / 580.0, 0.0, 1.0, 1.0]);
        // A frame past the end clamps rather than sampling outside the strip.
        assert_eq!(vp.frame_uv(20), vp.frame_uv(19));
    }

    #[test]
    fn a_strip_that_does_not_divide_evenly_keeps_the_authored_pitch() {
        // `mapicon_building_navalbase.dds` is 301 wide with 11 frames.
        let naval = strip(11, 301);
        assert_eq!(naval.frame_size()[0], 27.0);
        assert_eq!(naval.frame_uv(10), [10.0 / 11.0, 0.0, 1.0, 1.0]);
        // `mapicon_building_airbase_large.dds` is 309 wide with 7 frames.
        assert_eq!(strip(7, 309).frame_size()[0], 44.0);
    }

    #[test]
    fn layer_keys_stay_distinct_and_ordered_past_62500_icons() {
        // The old `priority * 1_000_000 + icon * 16 + child` key made icon
        // 62,500 of priority 5 equal to priority 6's first key.
        assert_ne!(layer(5, 62_500, 0), layer(6, 0, 0));
        assert!(layer(5, 62_500, 0) < layer(6, 0, 0));
        assert!(layer(5, u32::MAX, CHILD_SLOTS - 1) < layer(6, 0, 0));
        let mut keys: Vec<u64> = (0..200_000u32)
            .flat_map(|icon| [layer(10, icon, 0), layer(10, icon, CHILD_SLOTS - 1)])
            .collect();
        let count = keys.len();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        keys.dedup();
        assert_eq!(keys.len(), count);
        let key = layer(20, 150_000, 3);
        assert_eq!(layer_priority(key), 20);
        assert_eq!(layer_icon(key), 150_000);
        assert_eq!(layer(10, 7, 2) + icon_offset(70_000), layer(10, 70_007, 2));
    }

    #[test]
    fn priorities_are_the_defines_signed_low_byte_in_order_under_the_gui() {
        let defines = installed_content::defines::Defines::new().unwrap();
        defines
            .load(
                "mod.lua",
                b"NDefines = { NMapIcons = { A = 20.9, B = 300, C = 200, D = -1.5 } }",
            )
            .unwrap();
        let read = |name| priority(&defines, name).unwrap();
        assert_eq!(
            [read("A"), read("B"), read("C"), read("D")],
            [20, 44, -56, -1]
        );
        assert!(layer(-128, u32::MAX, CHILD_SLOTS - 1) < layer(-127, 0, 0));
        assert!(layer(-1, u32::MAX, CHILD_SLOTS - 1) < layer(0, 0, 0));
        assert!(layer(i8::MAX, u32::MAX, CHILD_SLOTS - 1) < gui_layer(0, 0));
        assert_eq!(layer_priority(layer(-56, 3, 0)), -56);
    }

    #[test]
    fn a_refused_upload_is_skipped_until_its_texture_changes() {
        let base = DYNAMIC_BASE;
        let uploaded = HashMap::from([(base, 1), (base + 1, 3)]);
        let failed = HashMap::from([(base + 2, 5)]);
        // base + 1 changed, base + 2 still at its refused revision, base + 3 new,
        // and base was evicted.
        let resident = [(base + 1, 4), (base + 2, 5), (base + 3, 0)];
        assert_eq!(
            plan_sync(&uploaded, &failed, &resident),
            SyncPlan {
                drop: vec![base],
                upload: vec![base + 1, base + 3],
            }
        );
        // A later revision of the refused texture is tried again, and an
        // evicted refused texture is forgotten.
        assert_eq!(
            plan_sync(&uploaded, &failed, &[(base + 2, 6)]).upload,
            [base + 2]
        );
        assert_eq!(plan_sync(&HashMap::new(), &failed, &[]).drop, [base + 2]);
    }

    #[test]
    fn a_batch_keeps_the_push_order_and_names_its_texture() {
        let mut batch = Batch::default();
        assert!(batch.is_empty());
        batch.sprite(&strip(20, 580), 2, [10.0, 20.0], [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(batch.quads[0].rect, [10.0, 20.0, 29.0, 27.0]);
        assert_eq!(batch.quads[0].texture, 0);
        batch.text(
            &[text::TextGlyph {
                page: 3,
                rect: [1.0, 2.0, 3.0, 4.0],
                uv: [0.0, 0.0, 1.0, 1.0],
            }],
            [0.5, 0.5, 0.5, 1.0],
        );
        assert_eq!(batch.quads[1].texture, 3);
        assert_eq!(batch.quads[1].color, [0.5, 0.5, 0.5, 1.0]);
    }

    /// Every device pixel of a quad: its bilinear footprint in texels along x,
    /// with the sample clamped to `window` when one is given.
    fn footprints(
        rect: [f32; 4],
        ui_scale: f32,
        uv: [f32; 4],
        texels: u32,
        window: Option<[f32; 4]>,
    ) -> Vec<(f32, f32)> {
        let [x0, _, x1, _] = device_corners(rect, ui_scale);
        (x0 as u32..x1 as u32)
            .map(|pixel| {
                let t = (pixel as f32 + 0.5 - x0) / (x1 - x0);
                let mut u = uv[0] + (uv[2] - uv[0]) * t;
                if let Some(window) = window {
                    u = u.clamp(window[0], window[2]);
                }
                let texel = u * texels as f32;
                (texel - 0.5, texel + 0.5)
            })
            .collect()
    }

    #[test]
    fn a_scaled_flag_does_not_bleed_into_the_next_cell() {
        // A 41-texel flag cell at texels 82..123 of a 512-texel page, with other
        // flags on both sides, drawn 1:1 at 41 interface pixels.
        let texels = 512;
        let uv = [82.0 / 512.0, 0.0, 123.0 / 512.0, 1.0];
        let rect = [100.0, 0.0, 41.0, 26.0];
        let inside = |(low, high): (f32, f32)| low >= 82.0 - 1e-3 && high <= 123.0 + 1e-3;
        // At 1.25 the edge pixels' taps reach a quarter texel into each neighbour.
        let unclamped = footprints(rect, 1.25, uv, texels, None);
        assert!(!inside(unclamped[0]) && !inside(*unclamped.last().unwrap()));
        let window = sample_window(uv, [texels, 1]);
        for scale in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let pixels = footprints(rect, scale, uv, texels, Some(window));
            assert!(pixels.iter().copied().all(inside), "{scale}");
        }
        // At 1:1 the window moves no sample: every pixel is a texel centre.
        let clamped = footprints(rect, 1.0, uv, texels, Some(window));
        let free = footprints(rect, 1.0, uv, texels, None);
        assert_eq!(clamped.len(), 41);
        for (a, b) in clamped.iter().zip(&free) {
            assert!((a.0 - b.0).abs() < 1e-3, "{a:?} {b:?}");
        }
    }

    #[test]
    fn only_a_scaled_batch_changes_how_it_samples() {
        assert_eq!(filter_mode(1.0), 0.0);
        assert_eq!(filter_mode(1.25), 1.0);
        assert_eq!(filter_mode(0.75), 2.0);
    }

    #[test]
    fn a_window_narrower_than_a_texel_collapses_to_its_centre() {
        let window = sample_window([0.5, 0.25, 0.5, 0.75], [4, 4]);
        assert_eq!(window, [0.5, 0.375, 0.5, 0.625]);
        // Flipped rectangles keep their bounds ordered.
        assert_eq!(sample_window([0.75, 0.0, 0.25, 1.0], [4, 1])[0], 0.375);
    }

    #[test]
    fn at_scale_one_the_corners_are_the_plain_rounding() {
        assert_eq!(
            device_corners([10.4, 20.6, 29.0, 27.0], 1.0),
            [10.0, 21.0, 39.0, 48.0]
        );
    }

    #[test]
    fn scaled_quads_that_touch_in_interface_pixels_still_touch_on_screen() {
        // Two 10-pixel cells side by side at 1.25: rounding sizes would give
        // 13 + 13 from 13 and 25, overlapping by a pixel.
        let left = device_corners([10.0, 0.0, 10.0, 10.0], 1.25);
        let right = device_corners([20.0, 0.0, 10.0, 10.0], 1.25);
        assert_eq!(left, [13.0, 0.0, 25.0, 13.0]);
        assert_eq!(right[0], left[2]);
        assert_eq!(right, [25.0, 0.0, 38.0, 13.0]);
        // The owner's display: 2048 interface pixels fill 2560 device pixels.
        assert_eq!(
            device_corners([0.0, 0.0, 2048.0, 1152.0], 1.25),
            [0.0, 0.0, 2560.0, 1440.0]
        );
    }
}
