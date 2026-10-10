//! Screen-space sprites and compiled GUI draws.
//! Callers lay out rectangles in interface pixels. The simple atlas path snaps their corners
//! to device pixels and supplies clip-space vertices; compiled GUI draws retain their own
//! geometry, constants, sampler states and scissors. Layer keys preserve icon-child ordering
//! while grouping compatible glyphs by texture. The pass draws after the world without depth.

use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

#[cfg(test)]
pub(super) mod oracle;

use eframe::wgpu;
use installed_content::{
    ResolvedLayers, gfx,
    gui::native::resources,
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
    /// One frame's size, with its horizontal extent truncated to whole texels.
    pub fn frame_size(&self) -> [f32; 2] {
        [
            (self.size[0] / self.frames.max(1)) as f32,
            self.size[1] as f32,
        ]
    }

    /// The atlas rectangle of a zero-based frame, clamped to the last frame.
    /// Frames retain their evenly divided strip pitch even when their width is fractional.
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
    // counters: the unit-counter, base and combat sheets, and the flag atlas.
    pub counters: CounterAssets,
    /// The raid map icon's layout, sprites and raid content.
    pub raids: super::raids::RaidAssets,
    /// Native world-text definitions and font demand, from the recorded content stack.
    /// Flags and portraits decoded on first draw, above [`DYNAMIC_BASE`].
    pub lazy: Arc<LazyTextures>,
}

impl MapIcons {
    /// Reads fonts and map-icon sprites through the resolved content stack, using the client's
    /// texture decode state for their pages.
    #[allow(clippy::too_many_arguments)] // The content stack plus the client's settings.
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content::defines::Defines,
        map: &installed_content::map::MapData,
        regions: &BTreeMap<u32, installed_content::regions::StrategicRegion>,
        language: &str,
        map_size: [u32; 2],
        gui_viewport: [u32; 2],
        decode: Decode,
    ) -> io::Result<Self> {
        let mut images = Vec::new();
        // The natively drawn map-icon windows read nothing the legacy readers give. This read
        // decides only which sprites, fonts and textures the windows' resources load, a set no
        // GUI viewport changes (`max_resolution` changes a `windowType`'s size, never which
        // elements exist); every read a view lays out and draws is at its own viewport
        // (`CounterAssets::windows_at`).
        let windows = super::counters::NativeWindows::load_at(layers, gui_viewport)?;
        let layouts = super::counters::Layouts::read(&windows);
        let mut font_warnings = Vec::new();
        // The battle-plan label font is named by `maparrows.txt`'s `arrowText`.
        let arrow_font = super::orders::arrow_text_font(layers)?;
        // Interface definitions and graphics files share registries, so later font and colour
        // overrides affect both world and GUI text.
        let registry = installed_content::gui::native::resources::Registry::from_files(
            installed_content::gui::native::sprites::read_object_layers(layers)?,
        );
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
                counter: layouts
                    .land
                    .as_ref()
                    .ok()
                    .and_then(|land| land.count.font.as_deref()),
                arrow: arrow_font.as_deref(),
            },
            &mut images,
            &mut font_warnings,
        )?;
        let sprites = &interface.sprites;
        // Flag and portrait reads happen after this load returns, so the lazy
        // stores keep their own handle on the layers.
        let shared = Arc::new(layers.clone());
        let lazy = LazyTextures::new();
        // counters
        let mut counters = CounterAssets::load(
            &shared,
            (layouts, windows),
            defines,
            (sprites, &interface.registry),
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
            regions,
            defines,
            &interface,
            &counters.large_flags,
            (language, map_size, &counters.windows),
            &mut images,
            &lazy,
        )?;
        let mut raids = super::raids::RaidAssets::load(layers, defines, map, &interface.registry)?;
        raids.triggers = combat.tactics.trigger_names();
        raids.character_traits = combat.tactics.trait_databases();
        raids.transport_sub_units = combat.sub_units().cloned().map_err(str::to_owned);
        counters
            .gui
            .load(layers, &raids.gui_assets(), &mut images)?;
        if images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(Self {
            combat,
            images,
            fonts,
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
        // Both registries consume this one harvest of localisation and combat shortcut keycaps.
        let (extra, mut warnings) = self.combat.localisation_assets(localisation);
        warnings.extend(self.counters.load_gui(
            layers,
            localisation,
            extra.clone(),
            &mut self.images,
        )?);
        warnings.extend(
            self.combat
                .load_gui(layers, localisation, &extra, &mut self.images)?,
        );
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
    decode: Decode,
    images: &mut Vec<dds::Texture>,
) -> io::Result<SpriteSheet> {
    let (texture, frames, size) = decode_sheet(layers, sprites, name, decode)?;
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
    decode: Decode,
) -> io::Result<(dds::Texture, u32, [u32; 2])> {
    let sprite = sprites.lookup(name).map_err(invalid)?;
    let path = sprite
        .texture
        .as_deref()
        .ok_or_else(|| invalid(format!("sprite {name} names no texturefile")))?;
    let texture = resources::stack_texture(layers, path.as_bytes(), decode)?
        .map_err(|why| invalid(format!("sprite {name}: {why}")))?;
    let texture = texture.image;
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
    /// Clockwise rotation about the rectangle's centre, in radians. Zero retains the snapped
    /// axis-aligned path.
    pub rotation: f32,
    // counters
    /// The sort key [`layer`] builds; see it for the field order.
    pub layer: u64,
    /// Signed brightness adjustment after tinting: positive brightens, negative darkens.
    pub shade: f32,
    /// A quad [`super::native_gui`] places and shades as the game does; `rect`, `uv`, `rotation`
    /// and `shade` are then unused.
    pub exact: Option<Exact>,
}

/// An unsnapped GUI draw with compiled resources, or layout-fixture geometry and metadata.
/// Its scissor clips interface-pixel corners after scaling. Compiled draws carry their own
/// complete vertex and constant payload in `native`.
#[derive(Clone, Debug, PartialEq)]
pub struct Exact {
    pub native: Option<Arc<super::native_shader::draw::Quad>>,
    /// Where the vertices land, in interface pixels.
    pub corners: [[f32; 2]; 4],
    /// Each vertex's texture coordinate as the vertex buffer holds it (before `Offset`).
    pub uv: [[f32; 2]; 4],
    /// The scissor `x0, y0, x1, y1` in whole interface pixels: the device's is each coordinate
    /// times the interface scale, truncated.
    pub scissor: Option<[i32; 4]>,
    /// Selected technique ID for compiler-independent layout fixtures.
    pub technique: u32,
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
/// by the content priorities and draws each icon's children in
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

/// A window containing the active GUI object adds five to its signed content priority.
/// The sum can exceed i8::MAX, so it must not wrap back below the map.
pub const fn hovered_layer(priority: i8, hovered: bool, icon: u32, child: u32) -> u64 {
    band_layer(
        (priority as i32 + 128 + if hovered { 5 } else { 0 }) as u64,
        icon,
        child,
    )
}

/// A key in the band above every map-icon priority, where the combat window
/// draws over the map icons as the GUI does.
pub const fn gui_layer(icon: u32, child: u32) -> u64 {
    band_layer(GUI_BAND, icon, child)
}

const fn band_layer(band: u64, icon: u32, child: u32) -> u64 {
    band * PRIORITY_STRIDE + icon as u64 * CHILD_SLOTS as u64 + child as u64
}

/// Reads an icon's content priority as a sign-extended low byte.
pub fn priority(defines: &installed_content::defines::Defines, name: &str) -> io::Result<i8> {
    Ok(defines.int32("NMapIcons", name)? as i8)
}

/// Child slots per icon in a [`layer`] key.
pub const CHILD_SLOTS: u32 = 32;

/// Keys one priority spans: room for every `u32` icon of [`CHILD_SLOTS`] children.
const PRIORITY_STRIDE: u64 = 1 << 40;
const _: () = assert!((u32::MAX as u64 + 1) * CHILD_SLOTS as u64 <= PRIORITY_STRIDE);
/// The band [`gui_layer`] keys, above every signed-byte map-icon priority
/// including the native five-point GUI-hover lift.
const GUI_BAND: u64 = 261;

/// Tooltips use the final GUI pass above ordinary windows.
pub const TOOLTIP_LAYER: u64 = band_layer(GUI_BAND + 1, 0, 0);

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
/// Each coordinate is scaled and truncated, then clipped to the target bounds.
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
/// shader uses: the rectangle inset by half a texel on every
/// side, so no bilinear tap reaches a neighbouring frame, glyph or flag cell.
/// A rectangle narrower than a texel collapses to its centre.
///
/// A 1:1 or minified quad on whole device pixels already samples inside it,
/// so this changes nothing there. A magnified one, which a UI scale above 1.0
/// draws, samples its edge pixels past the last texel centre; clamping them to
/// the window repeats the cell's edge texels, which is what an edge-extended
/// gutter around the cell would hold. Tests use this window to check atlas bounds.
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
    Native(usize),
}

/// One run of consecutive vertices sharing a pipeline, its textures and a scissor.
struct Run {
    pipe: Pipe,
    texture: usize,
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
    /// Static page views used by compiled effect bindings.
    views: Vec<wgpu::TextureView>,
    /// One transparent texel for an empty slot.
    empty: Bound,
    /// The content's lazy textures, mirrored into `dynamic` by [`SpritePass::sync`].
    lazy: Arc<LazyTextures>,
    synced: Option<u64>,
    dynamic: HashMap<usize, DynamicPage>,
    /// Lazy textures the device refused, at the revision it refused.
    failed: HashMap<usize, u64>,
    vertices: wgpu::Buffer,
    capacity: u64,
    runs: Vec<Run>,
    count: u32,
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
            // Priorities order the final icon layer; they do not participate in depth tests.
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
            views: textures
                .bound
                .iter()
                .map(|texture| texture.view.clone())
                .collect(),
            empty,
            lazy: Arc::clone(lazy),
            synced: None,
            dynamic: HashMap::new(),
            failed: HashMap::new(),
            vertices: buffer(device, INITIAL_VERTICES, std::mem::size_of::<Vertex>()),
            capacity: INITIAL_VERTICES,
            runs: Vec::new(),
            count: 0,
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
                self.native.invalidate_bindings();
                *self
                    .native_refusals
                    .entry("GUI quad has no compiled effect payload".into())
                    .or_default() += 1;
                continue;
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
                    scissor: None,
                    range: start..end,
                }),
            }
        }
        self.bakes.retain(|bake| bake.used);
        self.count = vertices.len() as u32;
        if self.count > 0 {
            if u64::from(self.count) > self.capacity {
                self.capacity = u64::from(self.count).next_power_of_two();
                self.vertices = buffer(device, self.capacity, std::mem::size_of::<Vertex>());
            }
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
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
                    Pipe::Native(_) => unreachable!(),
                }
                bound = Some(run.pipe);
            }
            let [x, y, w, h] = run.scissor.unwrap_or([0, 0, width, height]);
            pass.set_scissor_rect(x, y, w, h);
            let group = match run.pipe {
                Pipe::Legacy => self.group(run.texture),
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
        self.count / 6 + self.native_draws.len() as u32
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
            label: Some("text sprite bake"),
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

/// Applies the draw's program, constants, textures and samplers before preparing submission.
/// A refusal retains available bindings and forgets missing resources, so later inherited
/// draws cannot accidentally use resources from an earlier successful draw. Missing techniques
/// retain the program; a declared technique that fails compilation makes it unavailable.
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

/// The screen-space atlas program.
pub(super) const SHADER: &str = r#"
struct SpriteVertex {
    @location(0) clip_xy: vec2<f32>,
    @location(1) texcoord: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) cell: vec4<f32>,
    @location(4) sampling: f32,
    @location(5) shade: f32,
}

struct SpriteFragment {
    @builtin(position) position: vec4<f32>,
    @location(0) texcoord: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) @interpolate(flat) cell: vec4<f32>,
    @location(3) @interpolate(flat) sampling: f32,
    @location(4) @interpolate(flat) shade: f32,
}

@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;

@vertex
fn vertex_main(vertex: SpriteVertex) -> SpriteFragment {
    var fragment: SpriteFragment;
    fragment.position = vec4<f32>(vertex.clip_xy, 0.0, 1.0);
    fragment.texcoord = vertex.texcoord;
    fragment.tint = vertex.tint;
    fragment.cell = vertex.cell;
    fragment.sampling = vertex.sampling;
    fragment.shade = vertex.shade;
    return fragment;
}

fn read_sprite(
    texcoord: vec2<f32>,
    cell: vec4<f32>,
    sampling: f32,
    horizontal: vec2<f32>,
    vertical: vec2<f32>,
) -> vec4<f32> {
    if sampling < 0.5 {
        return textureSampleLevel(sprite_texture, sprite_sampler, texcoord, 0.0);
    }

    let dimensions = vec2<f32>(textureDimensions(sprite_texture, 0));
    let first_corner = min(cell.xy, cell.zw);
    let last_corner = max(cell.xy, cell.zw);
    let middle = (first_corner + last_corner) * 0.5;
    let inset = vec2<f32>(0.5) / dimensions;
    let lower = min(first_corner + inset, middle);
    let upper = max(last_corner - inset, middle);

    if sampling <= 1.5 {
        return textureSampleLevel(
            sprite_texture,
            sprite_sampler,
            clamp(texcoord, lower, upper),
            0.0,
        );
    }

    let footprint = max(abs(horizontal), abs(vertical)) * dimensions;
    let longest_axis = max(footprint.x, footprint.y);
    let count = u32(clamp(ceil(longest_axis - 0.001), 1.0, 4.0));
    let taps_per_axis = f32(count);
    var total = vec4<f32>(0.0);

    for (var row = 0u; row < count; row += 1u) {
        let row_center = (f32(row) + 0.5) / taps_per_axis - 0.5;
        for (var column = 0u; column < count; column += 1u) {
            let column_center = (f32(column) + 0.5) / taps_per_axis - 0.5;
            let horizontal_uv = texcoord + horizontal * column_center;
            let tap = clamp(horizontal_uv + vertical * row_center, lower, upper);
            total += textureSampleLevel(sprite_texture, sprite_sampler, tap, 0.0);
        }
    }
    return total / f32(count * count);
}

fn shade_sprite(source: vec4<f32>, tint: vec4<f32>, shade: f32) -> vec4<f32> {
    if tint.a < 0.0 {
        let luminance = dot(source.rgb, vec3<f32>(0.212671, 0.715160, 0.072169));
        return vec4<f32>(luminance * tint.rgb, source.a * abs(tint.a));
    }

    let tinted = source * tint;
    let brightened = tinted.rgb + shade * (tinted.rgb + vec3<f32>(0.5));
    return vec4<f32>(brightened, tinted.a);
}

@fragment
fn fragment_main(fragment: SpriteFragment) -> @location(0) vec4<f32> {
    let horizontal = dpdx(fragment.texcoord);
    let vertical = dpdy(fragment.texcoord);
    let source = read_sprite(
        fragment.texcoord,
        fragment.cell,
        fragment.sampling,
        horizontal,
        vertical,
    );
    return shade_sprite(source, fragment.tint, fragment.shade);
}
"#;

/// Disabled-state tint: negative alpha selects luminance greyscale, its magnitude the opacity.
pub const DISABLED: [f32; 4] = [1.0, 1.0, 1.0, -1.0];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_sheet_uses_the_native_texture_decoder_and_client_setting() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_counter_native_texture_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        std::fs::write(
            root.join("gfx/test.tga"),
            super::super::test_tga_rgba8(3, 2),
        )
        .unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = gfx::SpriteSet::new(&[(
            "interface/test.gfx".into(),
            installed_content::gui::native::sprites::read_sprites(
                b"spriteTypes = { spriteType = { name = GFX_test texturefile = gfx/test.tga } }",
            ),
        )]);
        let plain = decode_sheet(&layers, &sprites, "GFX_test", Decode::default()).unwrap();
        let padded = decode_sheet(
            &layers,
            &sprites,
            "GFX_test",
            Decode {
                force_pow2_textures: true,
            },
        )
        .unwrap();
        assert_eq!(plain.2, [3, 2]);
        assert_eq!(padded.2, [4, 2]);
        std::fs::remove_dir_all(root).unwrap();
    }

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
        // Twenty frames of 29 by 27 pixels share one strip.
        let vp = strip(20, 580);
        assert_eq!(vp.frame_size(), [29.0, 27.0]);
        assert_eq!(vp.frame_uv(0), [0.0, 0.0, 29.0 / 580.0, 1.0]);
        assert_eq!(vp.frame_uv(19), [551.0 / 580.0, 0.0, 1.0, 1.0]);
        // A frame past the end clamps rather than sampling outside the strip.
        assert_eq!(vp.frame_uv(20), vp.frame_uv(19));
    }

    #[test]
    fn a_strip_that_does_not_divide_evenly_keeps_the_authored_pitch() {
        // A fractional frame pitch retains the full strip extent.
        let naval = strip(11, 301);
        assert_eq!(naval.frame_size()[0], 27.0);
        assert_eq!(naval.frame_uv(10), [10.0 / 11.0, 0.0, 1.0, 1.0]);
        // `mapicon_building_airbase_large.dds` is 309 wide with 7 frames.
        assert_eq!(strip(7, 309).frame_size()[0], 44.0);
    }

    #[test]
    fn layer_keys_stay_distinct_and_ordered_past_62500_icons() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        assert!(gui_layer(u32::MAX, CHILD_SLOTS - 1) < TOOLTIP_LAYER);
        assert_eq!(layer_priority(layer(-56, 3, 0)), -56);
        assert_eq!(layer_priority(hovered_layer(i8::MAX, true, 0, 0)), 132);
        assert!(hovered_layer(i8::MAX, true, u32::MAX, CHILD_SLOTS - 1) < gui_layer(0, 0));
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
