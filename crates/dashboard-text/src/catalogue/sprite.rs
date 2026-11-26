use super::*;
use content_reader_gui::gui::native::sprites::FrameAnimation;
use content_reader_image::dds;

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
    if plain.frames <= 0 || max_edge == 0 {
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
    let (stretch, v_scale, flip) = if animated.is_some() {
        (plain.stretch, plain.v_scale, plain.flip_u)
    } else {
        (1.0, 1.0, false)
    };
    let grid = plain.grid();
    let du = (size[0] as f32 * (1.0 / size[0] as f32)) * (1.0 / grid[0] as f32);
    let dv = (size[1] as f32 * (1.0 / size[1] as f32)) * v_scale * (1.0 / grid[1] as f32);
    let step = plain.draw_step([du, dv]);
    let offset = |frame: u64| frame_offset(plain, step, frame);
    let [ox, oy] = offset(u64::from(frame));
    let [nx, ny] = offset(u64::from(frame) + 1);
    let vertices = vertices(geometry, stretch, [du, dv], flip);
    let mut c0 = vec![0.0; 64];
    c0[0] = 2.0 / extent[0] as f32;
    c0[5] = 2.0 / extent[1] as f32;
    c0[10] = 1.0;
    c0[12] = -1.0;
    c0[13] = -1.0;
    c0[14] = -1.0;
    c0[15] = 1.0;
    c0[16..20].fill(1.0);
    c0[20..24].copy_from_slice(&[ox, oy, nx, ny]);
    if let Some(animation) = animated {
        let native_frame = if animation.play_on_show {
            i64::from(frame) + 1
        } else {
            i64::from(frame)
        };
        c0[25] = ((0.0 - (native_frame - 1) as f32 * animation.seconds_per_frame)
            / animation.seconds_per_frame)
            .clamp(0.0, 1.0);
    }
    let mut defines = Vec::new();
    let mut textures = vec![(0, Some(texture))];
    if !plain.masking_texture.is_empty()
        && let Some(mask) = optional(layers, &plain.masking_texture, decode)?
    {
        defines.push("MASKING".into());
        textures.push((5, Some(mask)));
    }
    let mut constants = vec![(0, c0, std::iter::once(0..104).collect())];
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
            let mut c1 = vec![0.0; 64];
            let mut known = vec![0..48, 80..128];
            let mut count = 0;
            for data in &plain.animations {
                if !active_at_rest(data) {
                    continue;
                }
                let mask = optional(layers, &data.mask_file, decode)?;
                let overlay = optional(layers, &data.texture_file, decode)?;
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
        pixel_constants: true,
        textures,
    })
}

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

fn frame_offset(plain: &Plain, step: [f32; 2], frame: u64) -> [f32; 2] {
    let [x, y] = match plain.frame_direction {
        0 => [frame, 0],
        1 => [0, frame],
        2 => [frame % plain.frames as u64, frame / plain.frames as u64],
        _ => [0, 0],
    };
    [x as f32 * step[0], y as f32 * step[1]]
}

fn active_at_rest(data: &content_reader_gui::gui::native::sprites::AnimationData) -> bool {
    (data.trigger_frames == 0 || data.trigger_frames & 2 != 0)
        && (data.frames == u32::MAX || data.frames & 2 != 0)
        && (data.looping || data.time >= 0.0 || data.time.is_nan())
}

pub(super) fn load(
    layers: &ResolvedLayers,
    path: &[u8],
    decode: Decode,
) -> io::Result<dds::Texture> {
    super::check_unmounted_texture(layers, path)?;
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

pub(super) fn optional(
    layers: &ResolvedLayers,
    path: &[u8],
    decode: Decode,
) -> io::Result<Option<dds::Texture>> {
    super::check_unmounted_texture(layers, path)?;
    match resources::stack_texture(layers, path, decode)? {
        Ok(texture) => Ok(Some(texture.image)),
        Err(why) if why.native_failure() => Ok(None),
        Err(why) => Err(invalid(format!(
            "texture {}: {why}",
            String::from_utf8_lossy(path)
        ))),
    }
}

fn at_rest(
    data: &content_reader_gui::gui::native::sprites::AnimationData,
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
    use dashboard_core::test_fixtures::game;
    use content_reader_gui::gui::native::sprites::AnimationData;

    #[test]
    fn s4_native_frames_keep_offsets_past_the_strip_in_every_direction() {
        use super::super::texture_tests::png_bytes;
        game::enter();
        let (_, folder) = super::super::tests::folder_catalogue(&[]);
        let image = Rgba8Image {
            width: 8,
            height: 8,
            pixels: [30, 80, 140, 255].repeat(64),
        };
        std::fs::write(folder.0.join("strip.png"), png_bytes(&image)).unwrap();
        let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
        for (direction, frame, expected) in [
            (0, 0, [0.0, 0.0, 0.5, 0.0]),
            (0, 2, [1.0, 0.0, 1.5, 0.0]),
            (0, 7, [3.5, 0.0, 4.0, 0.0]),
            (1, 2, [0.0, 1.0, 0.0, 1.5]),
            (1, 7, [0.0, 3.5, 0.0, 4.0]),
            (2, 2, [0.0, 1.0, 0.5, 1.0]),
            (2, 3, [0.5, 1.0, 0.0, 2.0]),
            (7, 7, [0.0; 4]),
            (0, u32::MAX, [2_147_483_648.0, 0.0, 2_147_483_648.0, 0.0]),
        ] {
            let plain = Plain {
                texture: b"strip.png".to_vec(),
                frames: 2,
                frame_direction: direction,
                ..Default::default()
            };
            for animated in [None, Some(false), Some(true)] {
                let animation = animated.map(|play_on_show| FrameAnimation {
                    play_on_show,
                    ..Default::default()
                });
                let draw = draw(
                    &layers,
                    &plain,
                    animation.as_ref(),
                    Decode::default(),
                    frame,
                    96,
                )
                .unwrap();
                assert_eq!(
                    &draw.constants[0].1[20..24],
                    &expected,
                    "{direction}/{frame}"
                );
                assert_eq!(draw.size, if direction == 1 { [8, 4] } else { [4, 8] });
                if frame > 0 && animation.is_some() {
                    assert_eq!(draw.constants[0].1[25], 0.0);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires DXC and a GPU; synthetic content only"]
    fn s4_native_frames_sample_the_resolved_effect_and_sampler_past_the_strip() {
        use super::super::texture_tests::{ARTWORK_SHADER, png_bytes};
        game::enter();
        let (_, folder) =
            super::super::tests::folder_catalogue(&[("interface/art.gfx", "spriteTypes={}")]);
        let image = Rgba8Image {
            width: 8,
            height: 8,
            pixels: (0..8)
                .flat_map(|y| (0..8).flat_map(move |x| [20 + 10 * x, 30 + 10 * y, 140, 255]))
                .collect(),
        };
        std::fs::write(folder.0.join("strip.png"), png_bytes(&image)).unwrap();
        for mode in ["clamp", "wrap", "mirror", "border"] {
            let shader = ARTWORK_SHADER
                .replace(
                    "AddressU = clamp AddressV = clamp",
                    &format!("AddressU = {mode} AddressV = {mode}"),
                )
                .replace("return color;", "color.b = 0.0; return color;");
            std::fs::write(folder.0.join("gfx/FX/sample.shader"), shader).unwrap();
            let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
            for direction in [0, 1, 2] {
                let plain = Plain {
                    texture: b"strip.png".to_vec(),
                    effect_file: b"gfx/FX/sample.lua".to_vec(),
                    frames: 2,
                    frame_direction: direction,
                    ..Default::default()
                };
                let rendered = frame(&layers, &plain, None, Decode::default(), 2, 96).unwrap();
                let [width, height] = if direction == 1 { [8, 4] } else { [4, 8] };
                assert_eq!([rendered.width, rendered.height], [width, height]);
                let expected: Vec<u8> = (0..height)
                    .flat_map(|y| {
                        (0..width).flat_map(move |x| {
                            let (x, y) = match (mode, direction) {
                                ("clamp", 0) => (7, y),
                                ("clamp", _) => (x, 7),
                                ("mirror", 0) => (7 - x, y),
                                ("mirror", _) => (x, 7 - y),
                                _ => (x, y),
                            };
                            if mode == "border" {
                                [0; 4]
                            } else {
                                [20 + 10 * x as u8, 30 + 10 * y as u8, 0, 255]
                            }
                        })
                    })
                    .collect();
                assert_eq!(rendered.pixels, expected, "{mode}/{direction}");
            }
        }
    }

    #[test]
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
