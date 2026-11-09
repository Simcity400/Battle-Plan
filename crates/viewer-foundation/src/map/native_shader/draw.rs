//! Effect selection and native draw inputs carried from GUI layout to sorted GPU submission.
use super::{library, vertices::Declaration};
use crate::map::native_gui;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub original: native_gui::PartKind,
    pub scale: f32,
    pub offset: [f32; 2],
}

/// A technique's sampler-slot count: one past the highest common or pixel sampler index.
/// An undeclared technique has no sampler slots.
fn sampler_slots(effect: &Effect) -> usize {
    let source = &effect.asset.source;
    if source.technique(&effect.technique).is_none() {
        return 0;
    }
    source
        .common
        .samplers
        .iter()
        .chain(&source.pixel.samplers)
        .map(|sampler| sampler.index as usize + 1)
        .max()
        .unwrap_or(0)
}

/// How a draw's records are submitted: its topology and the order of the quad's corners
/// (`points`: 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left, y up) in its records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Strip([usize; 4]),
    List([usize; 6]),
}

/// Sprites, progress bars, masked flags and overlays, and text-sprite quads share a strip
/// ordered bottom-left, top-left, bottom-right, top-right.
const SPRITE_STRIP: Order = Order::Strip([3, 0, 2, 1]);
/// Plain tiles use bottom-left, top-left, top-right, bottom-right as a strip. Its triangles
/// follow the game's left-top-right, then top-right-bottom order, rather than a filled quad.
const TILE_STRIP: Order = Order::Strip([3, 0, 1, 2]);
/// Inline icons use a four-record strip: top-left, top-right, bottom-left, bottom-right.
const INLINE_STRIP: Order = Order::Strip([0, 1, 3, 2]);
/// Glyphs, nine-slice bake backgrounds and cornered tiles use triangle lists, six records
/// per quad.
const QUAD_LIST: Order = Order::List([3, 0, 2, 1, 2, 0]);
const CORNERED_LIST: Order = Order::List([3, 0, 1, 1, 2, 3]);

/// Byte offsets of `Time` and `AnimationTime` in sprite constants (`b0`, bound to both
/// stages), after the world matrix (64 bytes), `Color` (16) and texture offsets (16).
pub const SPRITE_TIME: u32 = 96;
pub const SPRITE_ANIMATION_TIME: u32 = 100;

/// The GUI viewport for a target of `size` device pixels at `ui_scale` device pixels per
/// GUI pixel, truncating each scaled extent. The settings' `min_gui` clamp is already in
/// `ui_scale`, resolved through `installed_content_settings::settings::Display::interface_to_frame`, so it
/// applies once. Fonts and callers' GUI layouts use this same viewport.
pub fn gui_viewport(size: [u32; 2], ui_scale: f32) -> [u32; 2] {
    let inverse = 1.0 / ui_scale;
    size.map(|extent| (extent as f32 * inverse) as u32)
}

pub fn transform(part: &mut native_gui::Part, scale: f32, offset: [f32; 2]) {
    if part.shading.compiled.is_none() {
        return;
    }
    let native = part.native.get_or_insert_with(|| {
        Box::new(Part {
            original: part.kind.clone(),
            scale: 1.,
            offset: [0.; 2],
        })
    });
    native.scale *= scale;
    native.offset = [0, 1].map(|i| offset[i] + scale * native.offset[i]);
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub asset: Arc<library::Effect>,
    pub technique: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    pub path: String,
    pub effect: Effect,
    pub declaration: Declaration,
    /// Original native positions and UVs, before the world/projection matrix.
    pub vertices: Vec<u8>,
    pub stride: usize,
    pub source_draws: u64,
    /// Native buffer zero, including the world matrix; projection is applied at upload. Its
    /// length is what the native binding shows a shader ([`Allocation::visible`]), of which only
    /// the first [`Quad::known`] bytes are written by the draw; the rest is uninitialised in the
    /// game.
    pub constants: Vec<f32>,
    /// Bytes of [`Quad::constants`] the draw writes before upload, from byte 0. The whole
    /// allocation is uploaded and a pooled binding exposes its remaining slot bytes too.
    /// Reading unwritten bytes is refused by name where the draw is submitted.
    pub known: u32,
    /// Text and masked flags bind vertex constants only; sprites, progress bars and tiles
    /// bind both stages.
    pub pixel_constants: bool,
    /// Only actual native binds; an absent entry retains the previous binding.
    pub textures: Vec<(u32, Option<usize>)>,
    /// Other native constant buffers: slot, the whole allocation, and the byte ranges the draw
    /// writes (the rest is uninitialised in the game).
    pub extra_constants: Vec<(u32, Vec<f32>, Vec<std::ops::Range<u32>>)>,
    /// A viewport explicitly written to a text part's font, also used by its inline quads.
    /// Other draws use the GUI viewport; bakes project through their target.
    pub font_viewport: Option<[i32; 2]>,
    /// The native topology the records are drawn as ([`Order`]).
    pub topology: eframe::wgpu::PrimitiveTopology,
    /// Use the renderer's default blend state for a bake background: blending off and all
    /// channels written. Other draws use their technique's blend state.
    pub default_blend: bool,
    pub bake: Option<Arc<Bake>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bake {
    pub size: [u32; 2],
    pub draws: Vec<Quad>,
}

impl Quad {
    /// Buffer zero with a 2D projection over `viewport`, in column-major HLSL storage.
    /// On-screen draws use the GUI viewport; bakes use their target. Each scale divides 2
    /// by the integer extent, followed by the -1, +1 translation. Text can instead use an
    /// explicitly written font viewport ([`Quad::font_viewport`]).
    pub fn projected_constants(&self, viewport: [u32; 2]) -> Vec<f32> {
        let mut constants = self.constants.clone();
        let matrix = Matrix::from_cols_array(constants[..16].try_into().unwrap());
        let viewport = self
            .font_viewport
            .map_or(viewport.map(|extent| extent as f32), |font| {
                font.map(|extent| extent as f32)
            });
        let projection = Matrix::from_cols_array(&[
            2. / viewport[0],
            0.,
            0.,
            0.,
            0.,
            2. / viewport[1],
            0.,
            0.,
            0.,
            0.,
            1.,
            0.,
            -1.,
            1.,
            0.,
            1.,
        ]);
        constants[..16].copy_from_slice(&(projection * matrix).to_cols_array());
        constants
    }
}

pub fn line_chart(
    effect: Effect,
    rect: [f32; 4],
    half_width: f32,
    points: &[[f32; 4]],
    batch: &mut crate::map::sprites::Batch,
) -> Result<(), String> {
    use crate::map::{native_effects::Blend, sprites};
    if points.len() > 40 {
        return Err("series exceeds the native linechart constant buffer".into());
    }
    if points.len() < 2 {
        return Ok(());
    }
    let mut vertices = Vec::with_capacity((points.len() - 1) * 48);
    for current in 0..points.len() as i16 - 1 {
        let next = current + 1;
        for record in [
            [next, -1, 1, 0],
            [current, 1, 1, 0],
            [current, 1, -1, 0],
            [next, -1, 1, 0],
            [next, -1, -1, 0],
            [current, 1, 1, 0],
        ] {
            for value in record {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let mut constants = vec![0.; 184];
    constants[..16].copy_from_slice(&Matrix::from_scale([1., -1., 1.]).to_cols_array());
    for (index, point) in points.iter().enumerate() {
        constants[16 + index * 4..20 + index * 4].copy_from_slice(point);
    }
    constants[176..180].copy_from_slice(&rect);
    constants[180..182].copy_from_slice(&[half_width, rect[2] / (points.len() - 1) as f32]);
    let [x, y, w, h] = rect;
    batch.push(sprites::Quad {
        texture: sprites::NO_TEXTURE,
        rect,
        uv: [0.; 4],
        color: [1.; 4],
        rotation: 0.,
        layer: 0,
        shade: 0.,
        exact: Some(sprites::Exact {
            native: Some(Arc::new(Quad {
                source_draws: 1,
                path: "air history linechart".into(),
                effect,
                declaration: Declaration::LineChart,
                vertices,
                stride: 8,
                constants,
                known: 736,
                pixel_constants: false,
                textures: Vec::new(),
                extra_constants: Vec::new(),
                font_viewport: None,
                topology: eframe::wgpu::PrimitiveTopology::TriangleList,
                default_blend: false,
                bake: None,
            })),
            corners: [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            uv: [[0.; 2]; 4],
            scissor: None,
            technique: 0,
            blend: Blend::Off,
            samplers: 0,
            lod_bias: [0.; 3],
            second: None,
            masking: None,
            offsets: [0.; 4],
            inputs: [0.; 4],
            second_color: [0.; 4],
        }),
        anchor: None,
        present: None,
    });
    Ok(())
}

/// Preserve the native vertex ABI. In particular, shaders may inspect local POSITION, COLOR1
/// or any byte of c0 regardless of the names they give their declarations.
pub fn emit(
    part: &native_gui::Part,
    origin: [i32; 2],
    sheets: &dyn native_gui::Sheets,
    batch: &mut crate::map::sprites::Batch,
) -> Result<(), native_gui::Refusal> {
    emit_inner(part, origin, sheets, batch, true)
}

fn emit_inner(
    part: &native_gui::Part,
    origin: [i32; 2],
    sheets: &dyn native_gui::Sheets,
    batch: &mut crate::map::sprites::Batch,
    join_text: bool,
) -> Result<(), native_gui::Refusal> {
    use crate::map::{native_effects::Blend, sprites};
    use native_gui::{InlineSource, PartKind, Refusal, RefusalKind};
    let effect = part.shading.compiled.as_ref().unwrap();
    let pixel_constants = !matches!(
        part.kind,
        PartKind::Text { .. }
            | PartKind::Inline { .. }
            | PartKind::MaskedFlag { .. }
            | PartKind::Fan { .. }
    );
    let refusal = |kind, detail| Refusal {
        path: part.path.clone(),
        kind,
        detail,
    };
    let (kind, scale, offset) = match &part.native {
        Some(native) => (&native.original, native.scale, native.offset),
        None => (&part.kind, 1., [0.; 2]),
    };
    // These wrappers preserve the original local geometry and parent transform while
    // adding the native texture/buffer binds around an ordinary sprite draw.
    if let PartKind::Animated(animated) = kind {
        let mut plain = part.clone();
        let replacement = PartKind::Sprite {
            sprite: animated.sprite.clone(),
            geometry: animated.geometry,
            uv: animated.uv,
            offsets: animated.offsets,
            masked: false,
        };
        plain.kind = replacement.clone();
        if let Some(native) = &mut plain.native {
            native.original = replacement;
        }
        // The caller's set only: the variant keeps the library's globals itself.
        let mut defines = effect
            .asset
            .caller()
            .iter()
            .filter(|d| *d != "MASKING" && *d != "ANIMATED" && !d.starts_with("NUM_ANIMATIONS_"))
            .cloned()
            .collect::<Vec<_>>();
        defines.extend([
            "ANIMATED".into(),
            format!("NUM_ANIMATIONS_{}", animated.records.len()),
        ]);
        let variant = Effect {
            asset: effect.asset.variant(defines),
            technique: animated.technique.into(),
        };
        // Bind animation records only when the animated technique has sampler slots.
        // Otherwise draw the ordinary sprite, including its masking.
        if sampler_slots(&variant) == 0 {
            let ordinary = PartKind::Sprite {
                sprite: animated.sprite.clone(),
                geometry: animated.geometry,
                uv: animated.uv,
                offsets: animated.offsets,
                masked: animated.masked,
            };
            plain.kind = ordinary.clone();
            if let Some(native) = &mut plain.native {
                native.original = ordinary;
            }
            return emit(&plain, origin, sheets, batch);
        }
        plain.shading.compiled = Some(variant);
        let mask = if animated.masked {
            Some(
                sheets
                    .masking(&animated.sprite)
                    .ok_or_else(|| {
                        refusal(
                            RefusalKind::MissingResource,
                            format!(
                                "sprite {:?}'s masking texture is not loaded",
                                native_gui::lossy(&animated.sprite)
                            ),
                        )
                    })?
                    .texture,
            )
        } else {
            None
        };
        let start = batch.quads.len();
        emit_inner(&plain, origin, sheets, batch, join_text)?;
        for quad in &mut batch.quads[start..] {
            let draw = Arc::make_mut(quad.exact.as_mut().unwrap().native.as_mut().unwrap());
            // Two 80-byte records are uploaded to b1 of both stages. Each record's first
            // 48 bytes are zeroed; an active record contributes its written ranges.
            // The shader sees the pooled slot beyond the 160 uploaded bytes.
            let mut constants = vec![0.; ANIMATION_RECORDS.visible as usize / 4];
            let mut known = vec![0..48, 80..128];
            for slot in 1..=4 {
                draw.textures.push((slot, None));
            }
            if let Some(mask) = mask {
                draw.textures.push((5, Some(mask)));
            }
            for (i, record) in animated.records.iter().enumerate() {
                constants[i * 20..(i + 1) * 20]
                    .copy_from_slice(bytemuck::cast_slice(&record.constants));
                let base = i as u32 * 80;
                known.extend(
                    record
                        .written()
                        .into_iter()
                        .map(|range| base + range.start..base + range.end),
                );
                draw.textures[i * 2 + 1].1 =
                    (record.mask != sprites::NO_TEXTURE).then_some(record.mask);
                draw.textures[i * 2 + 2].1 =
                    (record.texture != sprites::NO_TEXTURE).then_some(record.texture);
            }
            draw.extra_constants.push((1, constants, known));
        }
        return Ok(());
    }
    if let PartKind::TextSprite {
        sprite,
        bake,
        geometry,
        uv,
        offsets,
    } = kind
    {
        let baked = bake_draws(bake, &part.path, sheets)?;
        let mut plain = part.clone();
        // The target is supplied at upload, so no source sprite texture is required.
        let replacement = PartKind::Cell {
            texture: sprites::NO_TEXTURE,
            masking: None,
            geometry: *geometry,
            uv: *uv,
        };
        plain.kind = replacement.clone();
        if let Some(native) = &mut plain.native {
            native.original = replacement;
        }
        let start = batch.quads.len();
        emit_inner(&plain, origin, sheets, batch, join_text)?;
        for quad in &mut batch.quads[start..] {
            let draw = Arc::make_mut(quad.exact.as_mut().unwrap().native.as_mut().unwrap());
            draw.constants[20..24].copy_from_slice(offsets);
            draw.textures = vec![(0, None)];
            draw.bake = Some(baked.clone());
            draw.path = format!("{} / text sprite {}", part.path, native_gui::lossy(sprite));
        }
        return Ok(());
    }
    // Compilation happens at sorted submission, which refuses and counts only that draw.
    let declared = effect.asset.source.technique(&effect.technique).is_some();
    if !declared && matches!(part.kind, PartKind::Sprite { masked: true, .. }) {
        return Err(refusal(
            RefusalKind::NullRead,
            format!(
                "effect {:?} has no {:?} technique: masking reads the null effect record's texture bindings",
                effect.asset.source.effect, effect.technique
            ),
        ));
    }
    let offset = [0, 1].map(|i| offset[i] + origin[i] as f32);
    let parent = Matrix::from_translation([offset[0], -offset[1], 0.])
        * Matrix::from_scale([scale, scale, 1.]);
    let scissor = part.clip.map(|clip| {
        [
            clip[0] + origin[0],
            clip[1] + origin[1],
            clip[2] + origin[0],
            clip[3] + origin[1],
        ]
    });
    let missing = |what| refusal(RefusalKind::MissingResource, what);
    let texture = |name: &[u8]| {
        sheets.sheet(name).map(|s| s.texture).ok_or_else(|| {
            missing(format!(
                "sprite {:?} is not loaded",
                native_gui::lossy(name)
            ))
        })
    };
    let uv = |[u0, v0, u1, v1]: [f32; 4]| [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
    // After the 16-float world matrix: `Color`, the offsets, then `Time` and `AnimationTime`
    // ([`SPRITE_TIME`], [`SPRITE_ANIMATION_TIME`]).
    let sprite_tail = |offsets: [f32; 4]| {
        let mut tail = [0.; 10];
        tail[..4].copy_from_slice(&part.color);
        tail[4..8].copy_from_slice(&offsets);
        tail[8..].copy_from_slice(&[part.shading.time, part.shading.animation]);
        debug_assert_eq!(16 + tail.len(), SPRITE_ANIMATION_TIME as usize / 4 + 1);
        tail
    };
    let push = |batch: &mut sprites::Batch,
                positions: [[f32; 3]; 4],
                coordinates: [[f32; 2]; 4],
                matrix: Matrix,
                tail: &[f32],
                colors: Option<([f32; 4], u32)>,
                textures: Vec<(u32, Option<usize>)>,
                order: Order| {
        let declaration = if colors.is_some() {
            Declaration::Text
        } else {
            Declaration::Sprite
        };
        let stride = if colors.is_some() { 28 } else { 20 };
        let mut vertices = Vec::with_capacity(stride * 6);
        let topology = append_vertices(&mut vertices, positions, coordinates, colors, order);
        let world = parent * matrix;
        let landing = land(world, positions);
        let allocation = constant_allocation(kind, part.shading.clocked);
        let mut constants = Vec::with_capacity(allocation.visible as usize / 4);
        constants.extend_from_slice(&world.to_cols_array());
        constants.extend_from_slice(tail);
        // What the draw writes, then the rest of the native allocation, unwritten.
        let known = (constants.len() * 4) as u32;
        debug_assert!(known <= allocation.upload, "{known} > {allocation:?}");
        constants.resize(allocation.visible as usize / 4, 0.);
        let primary = textures
            .first()
            .and_then(|(_, t)| *t)
            .unwrap_or(sprites::NO_TEXTURE);
        batch.push(sprites::Quad {
            texture: primary,
            rect: [0.; 4],
            uv: [0.; 4],
            color: part.color,
            rotation: 0.,
            layer: 0,
            shade: 0.,
            exact: Some(sprites::Exact {
                native: Some(Arc::new(Quad {
                    source_draws: 1,
                    path: part.path.clone(),
                    effect: effect.clone(),
                    declaration,
                    vertices,
                    stride,
                    constants,
                    known,
                    pixel_constants,
                    textures,
                    extra_constants: Vec::new(),
                    font_viewport: None,
                    topology,
                    default_blend: false,
                    bake: None,
                })),
                corners: landing,
                uv: coordinates,
                scissor,
                technique: 0,
                blend: Blend::Off,
                samplers: 0,
                lod_bias: [0.; 3],
                second: None,
                masking: None,
                offsets: [0.; 4],
                inputs: [0.; 4],
                second_color: [0.; 4],
            }),
            anchor: None,
            present: None,
        });
    };
    let points =
        |[x, y, w, h]: [f32; 4], z| [[x, y + h, z], [x + w, y + h, z], [x + w, y, z], [x, y, z]];
    let translation = |[x, y]: [f32; 2], z| Matrix::from_translation([x, -y, z]);
    let geometry = |g: &native_gui::Geometry| {
        let bottom = g.rect[1] + g.local[1] + g.local[3];
        let x = g.rect[0] - g.local[0];
        let pivot = [g.pivot[0], -g.pivot[1], 0.];
        let matrix = Matrix::from_translation(pivot)
            * Matrix::from_rotation_z(g.rotation)
            * Matrix::from_translation(pivot.map(|v| -v))
            * translation([x, bottom], -1.);
        (points(g.local, 1.), matrix)
    };
    match kind {
        PartKind::Animated(_) | PartKind::TextSprite { .. } => unreachable!(),
        PartKind::Fan {
            sprite,
            centre,
            radius,
            rotation,
            weights,
        } => {
            let [a, b] = sheets.progress(sprite).ok_or_else(|| {
                missing(format!(
                    "circular progress {:?}: textures not loaded",
                    native_gui::lossy(sprite)
                ))
            })?;
            // Circular geometry binds b0 to the vertex stage only.
            let mut vertices = Vec::new();
            let rim = |angle: f32| [angle.cos(), angle.sin()];
            let mut previous = rim(*rotation);
            let mut k = 0;
            for (sector, weight) in weights.iter().enumerate() {
                for _ in 0..(*weight).max(0) {
                    k += 1;
                    let next = rim((k as f32 * 6.283_185_5) / 100. + rotation);
                    for p in [previous, [0., 0.], next] {
                        vertices.extend_from_slice(bytemuck::cast_slice(&[
                            p[0] * radius[0],
                            p[1] * radius[1],
                            p[0],
                            p[1],
                            sector as f32,
                            0.,
                        ]));
                    }
                    previous = next;
                }
            }
            if !vertices.is_empty() {
                push(
                    batch,
                    [[0.; 3]; 4],
                    [[0.; 2]; 4],
                    translation(*centre, 0.),
                    &[],
                    None,
                    // Bind at most two textures, limited by the technique's sampler-slot count.
                    // An undeclared technique has none.
                    [(0, Some(a.texture)), (1, Some(b.texture))]
                        .into_iter()
                        .take(sampler_slots(effect))
                        .collect(),
                    QUAD_LIST,
                );
                let draw = Arc::make_mut(
                    batch
                        .quads
                        .last_mut()
                        .unwrap()
                        .exact
                        .as_mut()
                        .unwrap()
                        .native
                        .as_mut()
                        .unwrap(),
                );
                draw.vertices = vertices;
                draw.stride = 24;
                draw.declaration = Declaration::Circular;
            }
        }
        PartKind::Sprite {
            sprite,
            geometry: g,
            uv: coordinates,
            offsets,
            masked,
        } => {
            let mut textures = vec![(0, Some(texture(sprite)?))];
            if *masked {
                textures.push((
                    5,
                    Some(
                        sheets
                            .masking(sprite)
                            .ok_or_else(|| {
                                missing(format!(
                                    "sprite {:?}'s masking texture is not loaded",
                                    native_gui::lossy(sprite)
                                ))
                            })?
                            .texture,
                    ),
                ));
            }
            let (positions, matrix) = geometry(g);
            push(
                batch,
                positions,
                uv(*coordinates),
                matrix,
                &sprite_tail(*offsets),
                None,
                textures,
                SPRITE_STRIP,
            );
        }
        PartKind::Cell {
            texture,
            masking,
            uv: coordinates,
            geometry: g,
        } => {
            let (positions, matrix) = geometry(g);
            let mut textures = vec![(0, Some(*texture))];
            if let Some(sprite) = masking {
                textures.push((
                    5,
                    Some(
                        sheets
                            .masking(sprite)
                            .ok_or_else(|| {
                                missing(format!(
                                    "sprite {:?}'s masking texture is not loaded",
                                    native_gui::lossy(sprite)
                                ))
                            })?
                            .texture,
                    ),
                ));
            }
            push(
                batch,
                positions,
                uv(*coordinates),
                matrix,
                &sprite_tail([0.; 4]),
                None,
                textures,
                SPRITE_STRIP,
            );
        }
        PartKind::Progress {
            sprite,
            geometry: g,
            uv,
            state,
            colors,
            textured,
        } => {
            let textures = if *textured {
                let [a, b] = sheets.progress(sprite).ok_or_else(|| {
                    missing(format!(
                        "progress bar {:?}'s textures are not loaded",
                        native_gui::lossy(sprite)
                    ))
                })?;
                vec![(0, Some(a.texture)), (1, Some(b.texture))]
            } else {
                vec![(0, None), (1, None)]
            };
            let mut tail = [0.; 9];
            tail[..4].copy_from_slice(&colors[0]);
            tail[4..8].copy_from_slice(&colors[1]);
            tail[8] = *state;
            let (positions, matrix) = geometry(g);
            push(
                batch,
                positions,
                *uv,
                matrix,
                &tail,
                None,
                textures,
                SPRITE_STRIP,
            );
        }
        PartKind::Tiles {
            sprite,
            quads,
            native_origin,
            cornered,
        } => {
            let texture = texture(sprite)?;
            for ([x, y, w, h], coordinates) in quads {
                let local = [x - native_origin[0], native_origin[1] - y - h, *w, *h];
                push(
                    batch,
                    points(local, 1.),
                    uv(*coordinates),
                    translation(*native_origin, -1.),
                    &sprite_tail([0.; 4]),
                    None,
                    vec![(0, Some(texture))],
                    if *cornered {
                        CORNERED_LIST
                    } else if part.shading.clocked {
                        TILE_STRIP
                    } else {
                        // A masked shield's overlay uses the sprite strip.
                        SPRITE_STRIP
                    },
                );
            }
        }
        PartKind::MaskedFlag {
            sprite,
            flag,
            flag_coords,
            rect: [x, y, w, h],
            native_origin,
        } => {
            let mask = sheets.mask(sprite).ok_or_else(|| {
                missing(format!(
                    "masked shield {:?}'s mask is not loaded",
                    native_gui::lossy(sprite)
                ))
            })?;
            let local = [x - native_origin[0], native_origin[1] - y - h, *w, *h];
            push(
                batch,
                points(local, 1.),
                uv([0., 0., 1., 1.]),
                translation(*native_origin, -1.),
                // `FlagCoords` followed by 16 zero bytes: all 96 bytes are written.
                &[
                    flag_coords[0],
                    flag_coords[1],
                    flag_coords[2],
                    flag_coords[3],
                    0.,
                    0.,
                    0.,
                    0.,
                ],
                None,
                vec![(0, Some(*flag)), (1, Some(mask.texture))],
                SPRITE_STRIP,
            );
        }
        PartKind::Inline {
            source,
            rect: [x, y, w, h],
            uv: coordinates,
            modulate,
            font_viewport,
            ..
        } => {
            let texture = match source {
                InlineSource::Sprite(name) => texture(name)?,
                InlineSource::Texture(id) => *id,
            };
            push(
                batch,
                points([*x, -y - h, *w, *h], 0.),
                uv(*coordinates),
                Matrix::IDENTITY,
                modulate,
                Some(([1.; 4], 0)),
                vec![(0, Some(texture))],
                INLINE_STRIP,
            );
            native_projection(batch.quads.last_mut(), *font_viewport);
        }
        PartKind::Text {
            glyphs,
            scale,
            modulate,
            border_color,
            font_viewport,
            ..
        } => {
            let join = join_text && glyphs.len() > 1 && independent_draws(effect);
            let geometry = |glyph: &native_gui::DrawnGlyph| {
                let [x, y] = glyph.at;
                let [width, height] = glyph.size.map(|value| value as f32 * scale);
                points([x, -y - height, width, height], 0.)
            };
            let mut remaining = glyphs.as_slice();
            while let Some((first, following)) = remaining.split_first() {
                let count = if join {
                    following
                        .iter()
                        .take_while(|glyph| glyph.texture == first.texture)
                        .count()
                } else {
                    0
                };
                push(
                    batch,
                    geometry(first),
                    uv(first.uv),
                    Matrix::IDENTITY,
                    modulate,
                    Some((first.color, *border_color)),
                    vec![(0, Some(first.texture))],
                    QUAD_LIST,
                );
                let exact = batch.quads.last_mut().unwrap().exact.as_mut().unwrap();
                let native = Arc::make_mut(exact.native.as_mut().unwrap());
                native.font_viewport = *font_viewport;
                native.source_draws = 1 + count as u64;
                if count > 0 {
                    native.vertices.reserve(count * 6 * native.stride);
                    let world = Matrix::from_cols_array(native.constants[..16].try_into().unwrap());
                    for glyph in &following[..count] {
                        let positions = geometry(glyph);
                        append_vertices(
                            &mut native.vertices,
                            positions,
                            uv(glyph.uv),
                            Some((glyph.color, *border_color)),
                            QUAD_LIST,
                        );
                        exact.corners = union(exact.corners, land(world, positions));
                    }
                }
                remaining = &following[count..];
            }
        }
    }
    Ok(())
}

fn native_projection(
    quad: Option<&mut crate::map::sprites::Quad>,
    font_viewport: Option<[i32; 2]>,
) {
    if let Some(native) = quad.and_then(|q| q.exact.as_mut()?.native.as_mut()) {
        Arc::make_mut(native).font_viewport = font_viewport;
    }
}

/// Constant-buffer sizes: `upload` is the allocation copied by the draw, and `visible`
/// is the extent exposed to the shader. Pooled buffers occupy ring slots rounded to
/// 256 bytes; bytes after the upload retain earlier slot contents. Unpooled bindings
/// expose only their allocation, and reads past it return zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub upload: u32,
    pub visible: u32,
}

impl Allocation {
    const fn unpooled(size: u32) -> Self {
        Self {
            upload: size,
            visible: size,
        }
    }

    const fn pooled(size: u32) -> Self {
        Self {
            upload: size,
            visible: (size + 0xFF) & !0xFF,
        }
    }
}

/// Animation records use a pooled b1 allocation of two 80-byte records.
pub const ANIMATION_RECORDS: Allocation = Allocation::pooled(0xA0);

/// The b0 allocation for a draw of `kind`. Each draw writes a prefix ([`Quad::known`]);
/// remaining visible bytes are unwritten:
/// - sprites, bound textures, plain tiles and text-sprite quads: 112 uploaded bytes,
///   pooled to 256 visible, with 104 written including both clocks;
/// - cornered tiles and shield overlays: 112 unpooled bytes, 104 written; overlay clocks
///   are zero ([`native_gui::Shading::clocked`]);
/// - progress bars: 112 unpooled bytes, 100 written for matrix, colours and fill;
/// - masked flags: all 96 unpooled bytes written for matrix, flag coordinates and a
///   16-byte zero tail;
/// - text and inline quads: all 80 unpooled bytes; inline quads retain the text's binding,
///   which their payload repeats, and bake backgrounds zero the full block first;
/// - circular progress bars: a 64-byte unpooled matrix.
///
/// Animation records use b1 with separate known ranges in [`Quad::extra_constants`].
fn constant_allocation(kind: &native_gui::PartKind, clocked: bool) -> Allocation {
    use native_gui::PartKind;
    match kind {
        PartKind::Text { .. } | PartKind::Inline { .. } => Allocation::unpooled(0x50),
        PartKind::MaskedFlag { .. } => Allocation::unpooled(0x60),
        PartKind::Fan { .. } => Allocation::unpooled(0x40),
        PartKind::Progress { .. } => Allocation::unpooled(0x70),
        PartKind::Tiles { cornered: true, .. } => Allocation::unpooled(0x70),
        PartKind::Tiles { .. } if !clocked => Allocation::unpooled(0x70),
        _ => Allocation::pooled(0x70),
    }
}

/// Bake backgrounds and glyphs use Text with white modulation and a y-up projection.
/// Backgrounds use the renderer's default blend; glyphs use the Text technique's blend.
fn bake_draws(
    bake: &native_gui::text_sprites::TextBake,
    path: &str,
    sheets: &dyn native_gui::Sheets,
) -> Result<Arc<Bake>, native_gui::Refusal> {
    use native_gui::{Part, PartKind};
    let mut batch = crate::map::sprites::Batch::default();
    let mut append = |kind, default_blend: bool, z: f32| -> Result<(), native_gui::Refusal> {
        let part = Part {
            path: path.into(),
            child: None,
            kind,
            clip: None,
            color: [1.; 4],
            shading: bake.shading.clone(),
            native: None,
        };
        if part.shading.compiled.is_none() {
            return Err(native_gui::Refusal {
                path: path.into(),
                kind: native_gui::RefusalKind::CustomEffect,
                detail: "text bake has no compiled Text effect".into(),
            });
        }
        let start = batch.quads.len();
        emit_inner(&part, [0, 0], sheets, &mut batch, false)?;
        for quad in &mut batch.quads[start..] {
            let draw = Arc::make_mut(quad.exact.as_mut().unwrap().native.as_mut().unwrap());
            draw.default_blend = default_blend;
            if default_blend {
                // The nine-slice background is one triangle list, with [`QUAD_LIST`] for
                // each piece emitted as an inline strip.
                let Order::List(list) = QUAD_LIST else {
                    unreachable!()
                };
                let Order::Strip(strip) = INLINE_STRIP else {
                    unreachable!()
                };
                let record = |corner: usize| strip.iter().position(|c| *c == corner).unwrap();
                let records: Vec<&[u8]> = draw.vertices.chunks_exact(draw.stride).collect();
                draw.vertices = list
                    .iter()
                    .flat_map(|&c| records[record(c)])
                    .copied()
                    .collect();
                draw.topology = eframe::wgpu::PrimitiveTopology::TriangleList;
            }
            // The bake projection runs from zero to its height with depth zero to one.
            // Preserve positive local y and z.
            draw.constants[13] = -(bake.size[1] as f32);
            for vertex in draw.vertices.chunks_exact_mut(draw.stride) {
                let y = f32::from_le_bytes(vertex[4..8].try_into().unwrap()) + bake.size[1] as f32;
                vertex[4..8].copy_from_slice(&y.to_le_bytes());
                vertex[8..12].copy_from_slice(&z.to_le_bytes());
            }
        }
        Ok(())
    };
    if let Some(background) = &bake.background {
        for &(rect, uv) in &background.quads {
            append(
                PartKind::Inline {
                    source: background.source.clone(),
                    rect,
                    uv,
                    modulate: [1.; 4],
                    projection: [1.; 2],
                    font_viewport: None,
                },
                true,
                0.5,
            )?;
        }
    }
    let mut start = [0, 0];
    for &end in &bake.frame_ends {
        // Inline icons draw while the frame's glyph buffer is built; grouped glyph
        // submission follows, so overlapping glyphs draw last.
        for (source, rect, uv) in &bake.inline[start[1]..end[1]] {
            append(
                PartKind::Inline {
                    source: source.clone(),
                    rect: *rect,
                    uv: *uv,
                    modulate: [1.; 4],
                    projection: [1.; 2],
                    font_viewport: None,
                },
                false,
                0.,
            )?;
        }
        if end[0] > start[0] {
            append(
                PartKind::Text {
                    font: Vec::new(),
                    glyphs: bake.glyphs[start[0]..end[0]].to_vec(),
                    scale: 1.,
                    modulate: [1.; 4],
                    border_color: bake.border_color,
                    projection: [1.; 2],
                    font_viewport: None,
                },
                false,
                1.,
            )?;
        }
        start = end;
    }
    let draws = batch
        .quads
        .into_iter()
        .map(|quad| (*quad.exact.unwrap().native.unwrap()).clone())
        .collect();
    Ok(Arc::new(Bake {
        size: bake.size.map(|v| v as u32),
        draws,
    }))
}

fn independent_draws(effect: &Effect) -> bool {
    let Some(index) = effect
        .asset
        .source
        .techniques
        .iter()
        .position(|technique| technique.name.eq_ignore_ascii_case(&effect.technique))
    else {
        return false;
    };
    effect.asset.independent_draws(index)
}

fn union(a: [[f32; 2]; 4], b: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
    let bounds = a.iter().chain(&b).fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    );
    [
        [bounds[0], bounds[1]],
        [bounds[2], bounds[1]],
        [bounds[2], bounds[3]],
        [bounds[0], bounds[3]],
    ]
}

fn land(world: Matrix, positions: [[f32; 3]; 4]) -> [[f32; 2]; 4] {
    let m = world.0;
    positions.map(|[x, y, z]| {
        [
            m[0] * x + m[4] * y + m[8] * z + m[12],
            -(m[1] * x + m[5] * y + m[9] * z + m[13]),
        ]
    })
}

fn append_vertices(
    vertices: &mut Vec<u8>,
    positions: [[f32; 3]; 4],
    coordinates: [[f32; 2]; 4],
    colors: Option<([f32; 4], u32)>,
    order: Order,
) -> eframe::wgpu::PrimitiveTopology {
    let (corners, topology): (&[usize], _) = match &order {
        Order::Strip(corners) => (corners, eframe::wgpu::PrimitiveTopology::TriangleStrip),
        Order::List(corners) => (corners, eframe::wgpu::PrimitiveTopology::TriangleList),
    };
    for &i in corners {
        vertices.extend_from_slice(bytemuck::cast_slice(&positions[i]));
        vertices.extend_from_slice(bytemuck::cast_slice(&coordinates[i]));
        if let Some((color, border)) = colors {
            vertices.extend([2, 1, 0, 3].map(|i| (color[i] * 255.) as u8));
            vertices.extend(border.to_le_bytes());
        }
    }
    topology
}

// Column-major native matrix storage; keep scalar f32 operations, without SIMD reassociation.
#[derive(Clone, Copy)]
struct Matrix([f32; 16]);
impl Matrix {
    const IDENTITY: Self = Self([
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]);
    fn from_cols_array(values: &[f32; 16]) -> Self {
        Self(*values)
    }
    fn to_cols_array(self) -> [f32; 16] {
        self.0
    }
    fn from_translation(values: [f32; 3]) -> Self {
        let mut result = Self::IDENTITY;
        result.0[12..15].copy_from_slice(&values);
        result
    }
    fn from_scale(values: [f32; 3]) -> Self {
        let mut result = Self::IDENTITY;
        for (i, value) in values.into_iter().enumerate() {
            result.0[i * 5] = value;
        }
        result
    }
    fn from_rotation_z(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self([c, s, 0., 0., -s, c, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.])
    }
}
impl std::ops::Mul for Matrix {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|i| {
            let row = i % 4;
            let col = i / 4;
            self.0[row] * rhs.0[col * 4]
                + self.0[4 + row] * rhs.0[col * 4 + 1]
                + self.0[8 + row] * rhs.0[col * 4 + 2]
                + self.0[12 + row] * rhs.0[col * 4 + 3]
        }))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod projection_tests;
