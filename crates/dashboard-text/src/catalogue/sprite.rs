//! Native Up-state sprite draw at a newly shown drawable's time zero.
use super::*;
use installed_content_core::gui::native::sprites::FrameAnimation;
use installed_content_image::dds;

pub(super) fn frame(
    layers: &ResolvedLayers,
    plain: &Plain,
    animated: Option<&FrameAnimation>,
    decode: Decode,
    frame: u32,
    max_edge: u32,
) -> io::Result<Rgba8Image> {
    let draw = draw(layers, plain, animated, decode, frame, max_edge)?;
    let image = shader_pipeline::artwork::render(layers, draw, decode).map_err(invalid)?;
    super::native_frame(&image, &Plain::default(), 0, max_edge)
}

pub(super) fn draw(
    layers: &ResolvedLayers,
    plain: &Plain,
    animated: Option<&FrameAnimation>,
    decode: Decode,
    frame: u32,
    max_edge: u32,
) -> io::Result<shader_pipeline::artwork::Draw> {
    if plain.frames <= 0 || frame >= plain.frames as u32 || max_edge == 0 {
        return Err(invalid("invalid icon frame"));
    }
    let texture = load(layers, &plain.texture, decode)?;
    let size = [texture.width, texture.height];
    let extent = resources::frame_extent(plain, size);
    if extent.iter().any(|edge| *edge <= 0) {
        return Err(invalid("sprite frame has no pixels"));
    }
    let extent = extent.map(|edge| edge as u32);
    let geometry = resources::frame_geometry(plain, size);
    // Plain sprites overwrite the type's stretch, v_scale and flip with a
    // new drawable's (1, 1, false); frame-animated sprites keep the authored
    // geometry unless their drawable supplies a stretch.
    let (stretch, v_scale, flip) = if animated.is_some() {
        (plain.stretch, plain.v_scale, plain.flip_u)
    } else {
        (1.0, 1.0, false)
    };
    // Geometry normalizes the texture before dividing its strip.
    let grid = plain.grid();
    let du = (size[0] as f32 * (1.0 / size[0] as f32)) * (1.0 / grid[0] as f32);
    let dv = (size[1] as f32 * (1.0 / size[1] as f32)) * v_scale * (1.0 / grid[1] as f32);
    let step = plain.draw_step([du, dv]);
    let offset = |frame: u32| frame_offset(plain, step, frame);
    let [ox, oy] = offset(frame);
    let [nx, ny] = offset(frame + 1);
    let vertices = vertices(geometry, stretch, [du, dv], flip);
    // The 0x70-byte upload is pooled in a 256-byte buffer.
    let mut c0 = vec![0.0; 64];
    c0[0] = 2.0 / extent[0] as f32;
    c0[5] = 2.0 / extent[1] as f32;
    c0[10] = 1.0;
    c0[12] = -1.0;
    // World placement puts the bottom-left at the truncated queried
    // height, with z=-1; the geometry keeps local z=1.
    c0[13] = -1.0;
    c0[14] = -1.0;
    c0[15] = 1.0;
    c0[16..20].fill(1.0);
    c0[20..24].copy_from_slice(&[ox, oy, nx, ny]);
    // A shown play_on_show drawable's zero-dt tick writes frame 1.
    // The draw uploads clamp((elapsed - (F-1)*SPF)/SPF, 0, 1), preserving
    // a NaN source operand.
    if let Some(animation) = animated {
        let native_frame = if animation.play_on_show {
            frame as i32 + 1
        } else {
            frame as i32
        };
        c0[25] = ((0.0 - (native_frame - 1) as f32 * animation.seconds_per_frame)
            / animation.seconds_per_frame)
            .clamp(0.0, 1.0);
    }
    let mut defines = Vec::new();
    let mut textures = vec![(0, Some(texture))];
    // MASKING is enabled only when its texture loaded; a texture failure
    // leaves the unmasked program rather than a failed drawable.
    if !plain.masking_texture.is_empty()
        && let Some(mask) = optional(layers, &plain.masking_texture, decode)?
    {
        defines.push("MASKING".into());
        textures.push((5, Some(mask)));
    }
    let mut constants = vec![(0, c0, std::iter::once(0..104).collect())];
    // Unsupported animation sets are cleared; frame-animated sprites
    // never select the overlay animation program.
    if animated.is_none() && !plain.legacy_lazy_load && !plain.animations.is_empty() {
        let program = shader_pipeline::artwork::program(layers, plain.effect()).map_err(invalid)?;
        let slots = program
            .common
            .samplers
            .iter()
            .chain(&program.pixel.samplers)
            .map(|s| s.index + 1)
            .max()
            .unwrap_or(0);
        if program.technique("Up").is_some() && slots >= 5 {
            // The pooled 0xA0-byte animation upload also exposes 256 bytes.
            let mut c1 = vec![0.0; 64];
            let mut known = vec![0..48, 80..128];
            let mut count = 0;
            for data in &plain.animations {
                if !active_at_rest(data) {
                    continue;
                }
                let mask = optional(layers, &data.mask_file, decode)?;
                let overlay = optional(layers, &data.texture_file, decode)?;
                // A nonempty texture name overwrites dimensions;
                // failed loads cache zero dimensions.
                let overlay_size = overlay.as_ref().map_or_else(
                    || {
                        if data.texture_file.is_empty() {
                            data.texture_size
                        } else {
                            [0; 2]
                        }
                    },
                    |image| [image.width as i32, image.height as i32],
                );
                let values = at_rest(data, size, overlay_size);
                c1[count * 20..count * 20 + 17].copy_from_slice(&values);
                // The upload writes 0..48, blend/mode/clamp, and offset/type
                // only for animation types 1..4. It never writes bytes 68..80.
                let base = count as u32 * 80;
                if (1..=4).contains(&data.kind) {
                    known.push(base + 48..base + 68);
                } else {
                    known.extend([base + 52..base + 56, base + 60..base + 68]);
                }
                textures.extend([
                    (1 + count as u32 * 2, mask),
                    (2 + count as u32 * 2, overlay),
                ]);
                count += 1;
                if count == 2 {
                    break;
                }
            }
            if count != 0 {
                defines = vec!["ANIMATED".into(), format!("NUM_ANIMATIONS_{count}")];
                constants.push((1, c1, known));
            }
        }
    }
    Ok(shader_pipeline::artwork::Draw {
        effect: plain.effect().to_vec(),
        defines,
        size: extent,
        vertices,
        constants,
        textures,
    })
}

/// The bottom-left, top-left, bottom-right, top-right vertex strip.
fn vertices(geometry: [f32; 2], stretch: f32, uv: [f32; 2], flip: bool) -> [[f32; 5]; 4] {
    let height = stretch * geometry[1];
    let inset = -(height - geometry[1]) * 0.5;
    let top = height + inset;
    let [du, dv] = uv;
    let [u0, u1] = if flip { [1.0, 1.0 - du] } else { [0.0, du] };
    [
        [0.0, inset, 1.0, u0, dv],
        [0.0, top, 1.0, u0, 0.0],
        [geometry[0], inset, 1.0, u1, dv],
        [geometry[0], top, 1.0, u1, 0.0],
    ]
}

/// Select integer frame coordinates before scaling the step.
fn frame_offset(plain: &Plain, step: [f32; 2], frame: u32) -> [f32; 2] {
    let [x, y] = match plain.frame_direction {
        0 => [frame, 0],
        1 => [0, frame],
        2 => [frame % plain.frames as u32, frame / plain.frames as u32],
        _ => [0, 0],
    };
    [x as f32 * step[0], y as f32 * step[1]]
}

fn active_at_rest(data: &installed_content_core::gui::native::sprites::AnimationData) -> bool {
    // Each record starts at (start=-1,last=0). The first draw enters Up (1),
    // starting records triggered by its bit.
    (data.trigger_frames == 0 || data.trigger_frames & 2 != 0)
        && (data.frames == u32::MAX || data.frames & 2 != 0)
        && (data.looping || data.time >= 0.0 || data.time.is_nan())
}

fn load(layers: &ResolvedLayers, path: &[u8], decode: Decode) -> io::Result<dds::Texture> {
    // Texture loading uses the loaded dimensions and device limits,
    // without a decoded-texel ceiling.
    resources::stack_texture(layers, path, decode)?
        .map(|texture| texture.image)
        .map_err(|why| {
            invalid(format!(
                "GUI texture {} is unavailable: {why}{}",
                String::from_utf8_lossy(path),
                if why.native_failure() {
                    "; the game draws no texture either"
                } else {
                    ""
                }
            ))
        })
}

fn optional(
    layers: &ResolvedLayers,
    path: &[u8],
    decode: Decode,
) -> io::Result<Option<dds::Texture>> {
    match resources::stack_texture(layers, path, decode)? {
        Ok(texture) => Ok(Some(texture.image)),
        Err(why) if why.native_failure() => Ok(None),
        Err(why) => Err(invalid(format!(
            "texture {}: {why}",
            String::from_utf8_lossy(path)
        ))),
    }
}

/// Animation values at t=0 and BlendFactor=1; scrolling, rotating, pulsing.
fn at_rest(
    data: &installed_content_core::gui::native::sprites::AnimationData,
    size: [u32; 2],
    overlay: [i32; 2],
) -> [f32; 17] {
    let [w, h] = size.map(|v| v as f32);
    let [aw, ah] = overlay.map(|v| v as f32);
    let ratio = w.min(h) / w.max(h);
    let wrap = || {
        if data.looping {
            0.0 % (data.delay + data.time)
        } else {
            0.0
        }
    };
    // Division/remainder remains even at t=0; zero duration can produce
    // NaNs, which saturation passes to the shader.
    let angle_at_rest = || {
        let phase = (wrap() / data.time).clamp(0.0, 1.0);
        (phase + phase) * std::f32::consts::PI
    };
    let (kind, offset, angle) = match data.kind {
        1 => {
            let k = (h / ah) * f32::from_bits(0x3FB5_0481);
            let s = 1.0 / ratio;
            let t = if data.looping && data.delay > 0.0 {
                wrap()
            } else {
                0.0
            };
            (
                1.0,
                t * ((k * s + 1.0) / data.time) - (s + 1.0) * 0.5 * k,
                data.rotation,
            )
        }
        2 | 3 => {
            let angle = angle_at_rest();
            (2.0, 0.0, if data.kind == 2 { -angle } else { angle })
        }
        4 => (3.0, 0.5 - 0.5 * angle_at_rest().cos(), data.rotation),
        _ => (0.0, 0.0, data.rotation),
    };
    let (sin, cos) = angle.sin_cos();
    [
        cos,
        -sin,
        sin,
        cos,
        if w >= h { 1.0 } else { w / h },
        if w >= h { h / w } else { 1.0 },
        w / aw,
        h / ah,
        data.rotation_offset[0],
        data.rotation_offset[1],
        data.texture_scale[0],
        data.texture_scale[1],
        offset,
        1.0,
        kind,
        data.blend_mode as f32,
        if !data.looping || data.delay > 0.0 {
            1.0
        } else {
            0.0
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content_core::gui::native::sprites::AnimationData;

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn artwork_loaders_admit_native_dimensions_above_the_former_texel_ceiling() {
        use std::io::Write;
        let (_, folder) = super::super::tests::folder_catalogue(&[]);
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 8193, 8193);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut png = encoder.write_header().unwrap();
            let mut writer = png.stream_writer().unwrap();
            let row = vec![37; 8193];
            for _ in 0..8193 {
                writer.write_all(&row).unwrap();
            }
            writer.finish().unwrap();
        }
        assert!(bytes.len() < 16 << 20);
        std::fs::write(folder.0.join("large.png"), bytes).unwrap();
        let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        for optional_texture in [false, true] {
            let image = if optional_texture {
                optional(&layers, b"large.png", Decode::default())
                    .unwrap()
                    .unwrap()
            } else {
                load(&layers, b"large.png", Decode::default()).unwrap()
            };
            assert_eq!([image.width, image.height], [8193, 8193]);
            assert!(u64::from(image.width) * u64::from(image.height) > 64 << 20);
            assert_eq!(&image.base().data[..4], &[37, 37, 37, 255]);
        }
        let image = super::super::gui_texture(&layers, b"large.png", Decode::default()).unwrap();
        assert_eq!([image.width, image.height], [8193, 8193]);
        assert_eq!(&image.pixels[image.pixels.len() - 4..], &[37, 37, 37, 255]);
    }

    #[test]
    fn geometry_keeps_native_local_coordinates_and_centres_stretch() {
        assert_eq!(
            vertices([7.5, 4.5], 2.0, [0.25, 0.75], true),
            [
                [0.0, -2.25, 1.0, 1.0, 0.75],
                [0.0, 6.75, 1.0, 1.0, 0.0],
                [7.5, -2.25, 1.0, 0.75, 0.75],
                [7.5, 6.75, 1.0, 0.75, 0.0],
            ]
        );
        let quad = vertices([7.5, 4.5], 1.0, [0.25, 0.75], false);
        assert_eq!(quad[0][1], 0.0);
        assert_eq!(quad[1][1], 4.5);
        // Native placement is at int_h=4: the fractional half-pixel is above
        // the top, rather than below the bottom.
        assert_eq!(4.0 - quad[1][1], -0.5);
    }

    #[test]
    fn native_frame_coordinates_keep_direction_two_grid_and_invalid_direction_zero() {
        let mut plain = Plain {
            frames: 3,
            frame_direction: 2,
            ..Default::default()
        };
        assert_eq!(frame_offset(&plain, [0.25, 0.75], 2), [0.5, 0.0]);
        assert_eq!(frame_offset(&plain, [0.25, 0.75], 3), [0.0, 0.75]);
        plain.frame_direction = 7;
        assert_eq!(frame_offset(&plain, [0.25, 0.75], 2), [0.0, 0.0]);
    }

    #[test]
    fn first_draw_starts_up_trigger_records_and_preserves_native_admission() {
        let mut data = AnimationData {
            trigger_frames: 2,
            ..Default::default()
        };
        assert!(active_at_rest(&data));
        data.trigger_frames = 4;
        assert!(!active_at_rest(&data));
        data.trigger_frames = 0;
        data.frames = 4;
        assert!(!active_at_rest(&data));
        data.frames = 2;
        data.looping = false;
        data.time = -1.0;
        assert!(!active_at_rest(&data));
        data.time = f32::NAN;
        assert!(active_at_rest(&data));
    }

    #[test]
    fn time_zero_keeps_native_undefined_duration_and_missing_overlay_math() {
        let mut data = AnimationData {
            time: 0.0,
            looping: false,
            ..Default::default()
        };
        for kind in [1, 4] {
            data.kind = kind;
            assert!(at_rest(&data, [4, 4], [4, 4])[12].is_nan());
        }
        for kind in [2, 3] {
            data.kind = kind;
            assert!(
                at_rest(&data, [4, 4], [4, 4])[..4]
                    .iter()
                    .all(|v| v.is_nan())
            );
        }
        data.kind = 1;
        data.time = 1.0;
        let draw = at_rest(&data, [4, 4], [0, 0]);
        assert!(draw[6].is_infinite());
        assert!(draw[7].is_infinite());
        assert!(draw[12].is_nan());
        data.kind = 2;
        data.looping = true;
        data.delay = -1.0;
        assert!(
            at_rest(&data, [4, 4], [4, 4])[..4]
                .iter()
                .all(|v| v.is_nan())
        );
    }
}
