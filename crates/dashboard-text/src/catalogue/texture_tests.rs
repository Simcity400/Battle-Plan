
use super::tests::{Folder, folder_catalogue};
use super::*;

pub fn write_sprite_program(root: &std::path::Path) {
    for (path, bytes) in [
        ("gfx/FX/buttonstate.shader", ARTWORK_SHADER.as_bytes()),
        ("gfx/FX/defines_hlsl_dx11.fxh", b"".as_slice()),
        ("map/default.map", ARTWORK_MAP.as_bytes()),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    for path in ["map/grid.bmp", "map/provinces.bmp"] {
        std::fs::write(root.join(path), map_bitmap()).unwrap();
    }
}

pub fn texture_catalogue(
    files: &[(&str, &str)],
    textures: &[(&str, Vec<u8>)],
) -> (Catalogue, [Folder; 2]) {
    let (_, base) = folder_catalogue(&[(
        "interface/art.gfx",
        "spriteTypes = { spriteType = { name = GFX_mod_sheet textureFile = \"gfx/base.tga\" } }",
    )]);
    let (mut catalogue, modification) = folder_catalogue(files);
    for (path, bytes) in textures {
        let file = modification.0.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let layers =
        ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&modification.0, &[][..])]).unwrap();
    catalogue.sprites = content_reader_gfx::gfx::read(&layers).unwrap();
    catalogue.guis = Default::default();
    catalogue.content = Arc::new(ResolvedContent {
        layers,
        stats: Default::default(),
        changed: Vec::new(),
        notes: Vec::new(),
    });
    (catalogue, [base, modification])
}

pub fn png_bytes(image: &Rgba8Image) -> Vec<u8> {
    crate::artwork::url_bytes(&png_url(image).unwrap())
        .unwrap()
        .to_vec()
}

pub fn from_url(url: &str) -> Rgba8Image {
    let bytes = crate::artwork::url_bytes(url).unwrap();
    let mut reader = png::Decoder::new(io::Cursor::new(&*bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(frame.buffer_size());
    Rgba8Image {
        width: frame.width,
        height: frame.height,
        pixels,
    }
}

fn strip() -> Rgba8Image {
    Rgba8Image {
        width: 6,
        height: 3,
        pixels: (0..3)
            .flat_map(|y| (0..6).flat_map(move |x| [10 + x, 20 + y, 30, 200]))
            .collect(),
    }
}

#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn rendered_sprite_frames_are_shared_across_callers_and_new_stacks_read_fresh() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, folders) = texture_catalogue(
        &[(
            "interface/art.gfx",
            "spriteTypes={spriteType={name=GFX_cached textureFile=gfx/sheet.png noOfFrames=3}}",
        )],
        &[("gfx/sheet.png", png_bytes(&strip()))],
    );
    write_sprite_program(&folders[1].0);
    let layers = &catalogue.content.layers;
    let sprite = catalogue.sprites.lookup("GFX_cached").unwrap();
    let decode = Decode {
        force_pow2_textures: false,
    };
    let first = sprite_frame_png(layers, sprite, decode, 0, 96).unwrap();
    let second = sprite_frame_png(layers, sprite, decode, 1, 96).unwrap();
    assert_ne!(from_url(&first).pixels, from_url(&second).pixels);
    std::fs::remove_file(folders[1].0.join("gfx/sheet.png")).unwrap();
    std::thread::scope(|threads| {
        let callers: Vec<_> = (0..8)
            .map(|_| {
                threads.spawn(|| sprite_frame_png(&layers.clone(), sprite, decode, 0, 96).unwrap())
            })
            .collect();
        for caller in callers {
            assert_eq!(caller.join().unwrap(), first);
        }
    });
    assert_eq!(
        sprite_frame_png(layers, sprite, decode, 1, 96).unwrap(),
        second
    );
    let fresh = ResolvedLayers::from_folders(&[(&folders[0].0, &[][..]), (&folders[1].0, &[][..])])
        .unwrap();
    assert!(sprite_frame_png(&fresh, sprite, decode, 0, 96).is_err());
}

#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn requested_sprite_batches_match_single_images_and_share_the_session() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, folders) = texture_catalogue(
        &[(
            "interface/art.gfx",
            "spriteTypes={spriteType={name=GFX_first textureFile=gfx/sheet.png noOfFrames=3} spriteType={name=GFX_second textureFile=gfx/sheet.png noOfFrames=2}}",
        )],
        &[("gfx/sheet.png", png_bytes(&strip()))],
    );
    write_sprite_program(&folders[1].0);
    let fresh = || {
        ResolvedLayers::from_folders(&[(&folders[0].0, &[][..]), (&folders[1].0, &[][..])]).unwrap()
    };
    let sprites = [
        catalogue.sprites.lookup("GFX_first").unwrap(),
        catalogue.sprites.lookup("GFX_second").unwrap(),
    ];
    let layers = fresh();
    let decode = Decode::default();
    let expected = sprites
        .iter()
        .map(|sprite| sprite_frame_png(&layers, sprite, decode, 0, 96).unwrap())
        .collect::<Vec<_>>();
    let batched = fresh();
    let requests = sprites.map(|sprite| (sprite, 0));
    sprite_frames_png(&batched, &requests, decode, 96).unwrap();
    std::fs::remove_file(folders[1].0.join("gfx/sheet.png")).unwrap();
    for (sprite, expected) in sprites.iter().zip(expected) {
        assert_eq!(
            sprite_frame_png(&batched, sprite, decode, 0, 96).unwrap(),
            expected
        );
    }
}

#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn batched_icons_replay_only_their_own_reads_and_shader_notes_in_either_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let first = format!("Includes = {{ absent_first.fxh }}\n{ARTWORK_SHADER}");
    let second = format!("Includes = {{ absent_second.fxh }}\n{ARTWORK_SHADER}");
    let (catalogue, folders) = texture_catalogue(
        &[
            (
                "interface/art.gfx",
                "spriteTypes = {
                    spriteType = { name = GFX_first textureFile = outside/first.png effectFile = outside/first.lua }
                    spriteType = { name = GFX_second textureFile = outside/second.png effectFile = outside/second.lua }
                }",
            ),
            ("outside/first.shader", &first),
            ("outside/second.shader", &second),
        ],
        &[
            ("outside/first.png", png_bytes(&strip())),
            ("outside/second.png", png_bytes(&strip())),
        ],
    );
    let fresh = || {
        let layers =
            ResolvedLayers::from_folders(&[(&folders[0].0, &[][..]), (&folders[1].0, &[][..])])
                .unwrap();
        let _ = layers.report_uncovered_reads();
        layers
    };
    let sprites = [
        catalogue.sprites.lookup("GFX_first").unwrap(),
        catalogue.sprites.lookup("GFX_second").unwrap(),
    ];
    let draw = |layers: &ResolvedLayers, index: usize| {
        diagnostics::observe(|| {
            sprite_frame_png(layers, sprites[index], Decode::default(), 0, 96).unwrap()
        })
    };
    let expected = [draw(&fresh(), 0), draw(&fresh(), 1)];
    for (index, name) in ["first", "second"].into_iter().enumerate() {
        assert!(expected[index].1.has_uncovered());
        assert!(
            expected[index]
                .1
                .shader_warnings()
                .iter()
                .any(|warning| warning.contains(&format!("absent_{name}.fxh")))
        );
        assert_ne!(expected[index].1, expected[1 - index].1);
    }
    for order in [[0, 1], [1, 0]] {
        let layers = fresh();
        let requests = order.map(|index| (sprites[index], 0));
        diagnostics::observe(|| {
            sprite_frames_png(&layers, &requests, Decode::default(), 96).unwrap();
        });
        for index in order.into_iter().chain(order.into_iter().rev()) {
            assert_eq!(draw(&layers, index), expected[index]);
        }
    }
}

#[test]
fn a_sprite_batch_draws_only_what_its_images_do_not_hold() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folders) = texture_catalogue(
        &[(
            "interface/art.gfx",
            "spriteTypes = { corneredTileSpriteType = { name = GFX_first texturefile = gfx/a.dds }              corneredTileSpriteType = { name = GFX_second texturefile = gfx/a.dds } }",
        )],
        &[],
    );
    let layers = &catalogue.content.layers;
    let sprites = [
        catalogue.sprites.lookup("GFX_first").unwrap(),
        catalogue.sprites.lookup("GFX_second").unwrap(),
    ];
    let decode = Decode::default();
    let requests = sprites.map(|sprite| (sprite, 0));
    assert!(sprite_frames_png(layers, &requests, decode, 96).is_err());
    let images = layers.presentation().derived::<SpriteImages>();
    let hold = |sprite| {
        images
            .get(
                sprite_image_key(sprite, decode, 0, 96),
                || Ok("held".into()),
            )
            .unwrap()
    };
    hold(sprites[0]);
    let repeated = [requests[0], requests[1], requests[1]];
    sprite_frames_png(layers, &repeated, decode, 96).unwrap();
    hold(sprites[1]);
    sprite_frames_png(layers, &requests, decode, 96).unwrap();
    assert!(
        content_reader_layers::content::strict(|| sprite_frames_png(layers, &requests, decode, 96))
            .is_err()
    );
}

#[test]
fn sprite_images_keep_only_the_strips_frames() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folders) = texture_catalogue(
        &[(
            "interface/art.gfx",
            "spriteTypes = { spriteType = { name = GFX_strip textureFile = gfx/absent.png noOfFrames = 2 } }",
        )],
        &[],
    );
    let layers = &catalogue.content.layers;
    let sprite = catalogue.sprites.lookup("GFX_strip").unwrap();
    let decode = Decode::default();
    let images = layers.presentation().derived::<SpriteImages>();
    for frame in [1, 2, 7, u32::MAX] {
        let why = sprite_frame_png(layers, sprite, decode, frame, 96).unwrap_err();
        assert!(why.to_string().contains("absent.png"), "{why}");
        assert_eq!(
            images.contains(&sprite_image_key(sprite, decode, frame, 96)),
            frame < 2,
            "{frame}"
        );
    }
    sprite_frames_png(layers, &[(sprite, 2), (sprite, 3)], decode, 96).unwrap();
    for frame in [2, 3] {
        assert!(!images.contains(&sprite_image_key(sprite, decode, frame, 96)));
    }
}

#[test]
fn a_mod_png_uses_native_frames_with_and_without_padding() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let image = strip();
    let (catalogue, _folders) = texture_catalogue(
        &[(
            "interface/art.gfx",
            "spriteTypes = { spriteType = { name = GFX_mod_sheet textureFile = \"gfx/mod/sheet.png\" noOfFrames = 3 } }",
        )],
        &[("gfx/mod/sheet.png", png_bytes(&image))],
    );
    let sprite = catalogue.sprites.lookup("GFX_mod_sheet").unwrap();
    let path = sprite.texture.as_deref().unwrap();
    assert_eq!(path, "gfx/mod/sheet.png");
    for (force_pow2_textures, expected_width, expected_height, columns) in
        [(false, 6, 3, [2, 3]), (true, 8, 4, [3, 4])]
    {
        let decode = Decode {
            force_pow2_textures,
        };
        let loaded = read_texture(&catalogue.content.layers, path, decode).unwrap();
        assert_eq!(
            (loaded.width, loaded.height),
            (expected_width, expected_height)
        );
        let frame = from_url(
            &texture_frame_png(
                &catalogue.content.layers,
                path,
                decode,
                sprite.frames,
                1,
                96,
            )
            .unwrap(),
        );
        assert_eq!((frame.width, frame.height), (2, expected_height));
        let expected: Vec<u8> = (0..expected_height)
            .flat_map(|y| {
                columns.into_iter().flat_map(move |x| {
                    if y < 3 {
                        [10 + x, 20 + y as u8, 30, 200]
                    } else {
                        [0; 4]
                    }
                })
            })
            .collect();
        assert_eq!(frame.pixels, expected);
        let first =
            texture_png(&catalogue.content.layers, path, decode, sprite.frames, 96).unwrap();
        assert_eq!(
            first,
            texture_frame_png(
                &catalogue.content.layers,
                path,
                decode,
                sprite.frames,
                0,
                96
            )
            .unwrap()
        );
    }
}

fn bgra(image: &Rgba8Image) -> Vec<u8> {
    image
        .pixels
        .chunks_exact(4)
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect()
}

fn dds(image: &Rgba8Image) -> Vec<u8> {
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [
        (4, 124),
        (8, 0x100F),
        (12, image.height),
        (16, image.width),
        (20, image.width * 4),
        (28, 1),
        (76, 32),
        (80, 0x41),
        (88, 32),
        (92, 0xFF0000),
        (96, 0xFF00),
        (100, 0xFF),
        (104, 0xFF000000),
        (108, 0x1000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes.extend(bgra(image));
    bytes
}

#[cfg(test)]
fn tga(image: &Rgba8Image) -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&(image.width as u16).to_le_bytes());
    bytes[14..16].copy_from_slice(&(image.height as u16).to_le_bytes());
    bytes[16] = 32;
    bytes[17] = 0x28;
    bytes.extend(bgra(image));
    bytes
}

pub fn refusal_textures() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let mut device = dds(&strip());
    device[16..20].copy_from_slice(&16385_u32.to_le_bytes());
    let face = Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![255; 4],
    };
    let mut claim = dds(&strip());
    claim[12..20].copy_from_slice(&[9000_u32.to_le_bytes(), 9000_u32.to_le_bytes()].concat());
    let mut cube = dds(&face);
    cube[112..116].copy_from_slice(&0xFE00_u32.to_le_bytes());
    for _ in 1..6 {
        cube.extend(bgra(&face));
    }
    vec![
        ("gfx/mod/absent.png", Vec::new(), "tried"),
        ("gfx/mod/no_extension", Vec::new(), "has no extension"),
        (
            "gfx/mod/noise.png",
            vec![0x13; 20],
            "the game draws no texture either",
        ),
        (
            "gfx/mod/tiny.png",
            vec![0, 0, 1],
            "does not determine what the game draws",
        ),
        ("gfx/mod/device.dds", device, "graphics device"),
        ("gfx/mod/cube.dds", cube, "cube-map DDS"),
        ("gfx/mod/claim.dds", claim, "first level runs past the file"),
        (
            "gfx/mod/large.png",
            vec![0x13; (16 << 20) + 1],
            "16 MiB, so it does not load",
        ),
    ]
}

#[test]
fn dds_and_engine_targa_still_decode_and_resolver_candidates_win() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let image = strip();
    let (catalogue, _folders) = texture_catalogue(
        &[],
        &[
            ("gfx/mod/strip.dds", dds(&image)),
            ("gfx/mod/strip.tga", tga(&image)),
            ("gfx/mod/strip.png", vec![0; 20]),
            ("gfx/mod/targa.tga", tga(&image)),
        ],
    );
    for name in [
        "gfx/mod/strip.dds",
        "gfx/mod/strip.png",
        "gfx/mod/targa.tga",
    ] {
        let loaded = read_texture(&catalogue.content.layers, name, Decode::default()).unwrap();
        assert_eq!(
            (loaded.width, loaded.height),
            (image.width, image.height),
            "{name}"
        );
        assert_eq!(loaded.pixels, image.pixels, "{name}");
    }
}

#[test]
fn padding_and_edges_above_2048_decode() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let wide = Rgba8Image {
        width: 4097,
        height: 1,
        pixels: vec![255; 4097 * 4],
    };
    let (catalogue, _folders) = texture_catalogue(
        &[],
        &[
            ("gfx/mod/icon.png", png_bytes(&strip())),
            ("gfx/mod/wide.png", png_bytes(&wide)),
        ],
    );
    let layers = &catalogue.content.layers;
    let padded = Decode {
        force_pow2_textures: true,
    };
    let image = read_texture(layers, "gfx/mod/icon.png", padded).unwrap();
    assert_eq!([image.width, image.height], [8, 4]);
    let image = read_texture(layers, "gfx/mod/wide.png", Decode::default()).unwrap();
    assert_eq!([image.width, image.height], [4097, 1]);
}

#[test]
fn native_failures_keep_their_distinct_reasons() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let refusals = refusal_textures();
    let files: Vec<_> = refusals
        .iter()
        .filter(|(_, bytes, _)| !bytes.is_empty())
        .map(|(path, bytes, _)| (*path, bytes.clone()))
        .collect();
    let (catalogue, _folders) = texture_catalogue(&[], &files);
    let reason = |path| {
        read_texture(&catalogue.content.layers, path, Decode::default())
            .unwrap_err()
            .to_string()
    };
    let missing = reason("gfx/mod/absent.png");
    for candidate in [
        "gfx/mod/absent.dds",
        "gfx/mod/absent.tga",
        "gfx/mod/absent.png",
    ] {
        assert!(missing.contains(candidate), "{missing}");
    }
    let noise = reason("gfx/mod/noise.png");
    assert!(
        noise.contains("the game draws no texture either"),
        "{noise}"
    );
    let tiny = reason("gfx/mod/tiny.png");
    assert!(
        tiny.contains("does not determine what the game draws"),
        "{tiny}"
    );
    assert!(!tiny.contains("no layer holds"), "{tiny}");
    let device = reason("gfx/mod/device.dds");
    assert!(
        device.contains("graphics device") && device.contains("not recorded"),
        "{device}"
    );
    for (path, _, expected) in refusals {
        let why = reason(path);
        assert!(why.contains(expected), "{path}: {why}");
    }
    assert!(reason("").contains("names no texture"));
}

#[test]
fn texture_frames_keep_fractional_padded_pixels_in_both_directions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folders) = texture_catalogue(
        &[(
            "interface/art.gfx",
            "spriteTypes = { spriteType = { name = GFX_mod_sheet textureFile = \"gfx/mod/sheet.png\" noOfFrames = 3 }              spriteType = { name = GFX_mod_down textureFile = \"gfx/mod/down.png\" noOfFrames = 3 framedirectiony = yes }              spriteType = { name = GFX_mod_shine textureFile = \"gfx/mod/sheet.png\" effectFile = \"gfx/FX/mod.lua\" } }",
        )],
        &[
            ("gfx/mod/sheet.png", png_bytes(&strip())),
            ("gfx/mod/down.png", png_bytes(&transposed(&strip()))),
        ],
    );
    let layers = &catalogue.content.layers;
    let sprite = |name| catalogue.sprites.lookup(name).unwrap();
    let decode = Decode {
        force_pow2_textures: true,
    };
    for frame in 0..3 {
        let across = png_url(
            &native_frame(
                &read_texture(layers, "gfx/mod/sheet.png", decode).unwrap(),
                sprite("GFX_mod_sheet").facts.kind.plain().unwrap(),
                frame,
                96,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            across,
            texture_frame_png(layers, "gfx/mod/sheet.png", decode, 3, frame, 96).unwrap()
        );
        let down = native_frame(
            &read_texture(layers, "gfx/mod/down.png", decode).unwrap(),
            sprite("GFX_mod_down").facts.kind.plain().unwrap(),
            frame,
            96,
        )
        .unwrap();
        assert_eq!(transposed(&down), from_url(&across), "{frame}");
    }
    let why = sprite_frame_png(layers, sprite("GFX_mod_shine"), decode, 0, 96)
        .unwrap_err()
        .to_string();
    assert!(why.contains("gfx/FX/mod"), "{why}");
    for (frames, frame) in [(0, 0), (3, 3), (9, 0)] {
        assert!(
            gui_texture_frame_png(layers, b"gfx/mod/sheet.png", decode, frames, frame, 96).is_err()
        );
    }
    assert!(gui_texture_frame_png(layers, b"gfx/mod/sheet.png", decode, 3, 0, 0).is_err());
}

#[test]
fn plain_sprite_refuses_a_missing_resolved_default_program_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, folder) = folder_catalogue(&[(
        "interface/art.gfx",
        "spriteTypes={spriteType={name=GFX_plain textureFile=plain.png}}",
    )]);
    std::fs::write(folder.0.join("plain.png"), png_bytes(&strip())).unwrap();
    std::fs::remove_file(folder.0.join("gfx/FX/buttonstate.shader")).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let why = sprite_frame_png(
        &layers,
        catalogue.sprites.lookup("GFX_plain").unwrap(),
        Decode::default(),
        0,
        96,
    )
    .unwrap_err()
    .to_string();
    assert!(
        why.contains("GFX_plain")
            && why.contains("buttonstate.lua")
            && why.contains("no Up technique"),
        "{why}"
    );
}

#[cfg(test)]
fn transposed(image: &Rgba8Image) -> Rgba8Image {
    let [width, height] = [image.width as usize, image.height as usize];
    Rgba8Image {
        width: image.height,
        height: image.width,
        pixels: (0..width)
            .flat_map(|x| {
                (0..height).flat_map(move |y| (0..4).map(move |c| (y * width + x) * 4 + c))
            })
            .map(|at| image.pixels[at])
            .collect(),
    }
}

#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn frame_animated_sprites_render_native_strip_frames() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folders) = texture_catalogue(
        &[("interface/art.gfx", "spriteTypes = {
            frameAnimatedSpriteType = { name = GFX_mod_sheet textureFile = gfx/sheet.png noOfFrames = 3 animation_rate_fps = 15 looping = yes play_on_show = yes }
            frameAnimatedSpriteType = { name = GFX_mod_vertical textureFile = gfx/down.png noOfFrames = 3 framedirectiony = yes animation_rate_spf = 0.5 }
        }"), ("gfx/FX/buttonstate.shader", ARTWORK_SHADER), ("gfx/FX/defines_hlsl_dx11.fxh", ""), ("map/default.map", ARTWORK_MAP)],
        &[("gfx/sheet.png", png_bytes(&strip())), ("gfx/down.png", png_bytes(&transposed(&strip()))), ("map/grid.bmp", map_bitmap())],
    );
    for decode in [
        Decode::default(),
        Decode {
            force_pow2_textures: true,
        },
    ] {
        let sheet = read_texture(&catalogue.content.layers, "gfx/sheet.png", decode).unwrap();
        let down = read_texture(&catalogue.content.layers, "gfx/down.png", decode).unwrap();
        let expected = |source: &Rgba8Image, columns: &[u32], rows: &[u32]| Rgba8Image {
            width: columns.len() as u32,
            height: rows.len() as u32,
            pixels: rows
                .iter()
                .flat_map(|y| {
                    columns.iter().flat_map(move |x| {
                        let at = ((y * source.width + x) * 4) as usize;
                        source.pixels[at..at + 4].iter().copied()
                    })
                })
                .collect::<Vec<u8>>(),
        };
        for frame in 0..3 {
            let image = sprite_frame_png(
                &catalogue.content.layers,
                catalogue.sprites.lookup("GFX_mod_sheet").unwrap(),
                decode,
                frame,
                96,
            )
            .unwrap();
            let columns = if decode.force_pow2_textures {
                [[0, 1], [3, 4], [5, 6]][frame as usize]
            } else {
                [frame * 2, frame * 2 + 1]
            };
            let rows: Vec<_> = (0..sheet.height).collect();
            assert_eq!(from_url(&image), expected(&sheet, &columns, &rows));
            let vertical = from_url(
                &sprite_frame_png(
                    &catalogue.content.layers,
                    catalogue.sprites.lookup("GFX_mod_vertical").unwrap(),
                    decode,
                    frame,
                    96,
                )
                .unwrap(),
            );
            let rows = if decode.force_pow2_textures {
                [[1, 2], [3, 4], [6, 7]][frame as usize]
            } else {
                [frame * 2, frame * 2 + 1]
            };
            let columns: Vec<_> = (0..down.width).collect();
            assert_eq!(vertical, expected(&down, &columns, &rows));
        }
    }
}

pub(crate) const ARTWORK_SHADER: &str = r#"
VertexStruct Input { float3 position : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct Output { float4 position : SV_POSITION;
float2 uv : TEXCOORD0;
};
ConstantBuffer(0,0) { float4x4 M;
float4 Color;
float4 Offsets;
float Time;
float AnimationTime;
};
VertexShader = { MainCode V [[
Output main(Input i) { Output o; o.position = mul(M,float4(i.position,1)); o.uv=i.uv+Offsets.xy; return o; }
]] }
PixelShader = {
Samplers = {
 Base { Index = 0 MagFilter = point MinFilter = point AddressU = clamp AddressV = clamp }
 AnimationMask { Index = 1 MagFilter = point MinFilter = point }
 Overlay { Index = 2 MagFilter = point MinFilter = point }
 SpareMask { Index = 3 }
 SpareOverlay { Index = 4 }
 Mask { Index = 5 MagFilter = point MinFilter = point }
}
ConstantBuffer(1,0) { float4 Rotation;
float4 UVScale;
float4 Parameters;
float4 Animation;
float4 Clamp;
};
MainCode P [[
float4 main(Output i) : SV_TARGET {
 float4 color=Base_Texture.Sample(Base_Sampler,i.uv)*Color;
 #ifdef MASKING
 color.a *= Mask_Texture.Sample(Mask_Sampler,i.uv).a;
 #endif
 #ifdef ANIMATED
 color.r += Overlay_Texture.Sample(Overlay_Sampler,i.uv).r * AnimationMask_Texture.Sample(AnimationMask_Sampler,i.uv).r * Animation.y;
 #endif
 return color;
}
]] }
Effect Up { VertexShader = V PixelShader = P }
"#;

pub fn replacement_programs() -> Vec<(&'static str, String)> {
    vec![
        (
            "gfx/FX/buttonstate.shader",
            ARTWORK_SHADER.replace("return color;", "color.b = 0.0; return color;"),
        ),
        ("gfx/FX/custom.shader", ARTWORK_SHADER.to_owned()),
    ]
}

pub fn strip_png(colours: &[[u8; 4]], down: bool) -> Vec<u8> {
    let long = colours.len() as u32 * 2;
    let (width, height) = if down { (2, long) } else { (long, 2) };
    png_bytes(&Rgba8Image {
        width,
        height,
        pixels: (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| colours[(if down { y } else { x } / 2) as usize])
            })
            .collect(),
    })
}

pub fn write_replacement_textures(root: &std::path::Path) {
    let solid = |rgba: [u8; 4]| {
        png_bytes(&Rgba8Image {
            width: 4,
            height: 4,
            pixels: rgba.repeat(16),
        })
    };
    for (path, rgba) in [
        ("gfx/replacement/base.png", [40, 80, 120, 200]),
        ("gfx/replacement/mask.png", [255, 0, 0, 128]),
        ("gfx/replacement/overlay.png", [50, 0, 0, 255]),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, solid(rgba)).unwrap();
    }
}

pub fn replacement_sprites(name: &str) -> Vec<(String, [u8; 4])> {
    let texture = "textureFile = gfx/replacement/base.png";
    [
        (format!("spriteType = {{ name = {name} {texture} }}"), [40, 80, 0, 200]),
        (
            format!("spriteType = {{ name = {name} {texture} effectFile = gfx/FX/custom.lua }}"),
            [40, 80, 120, 200],
        ),
        (
            format!("spriteType = {{ name = {name} {texture} effectFile = gfx/FX/custom.lua masking_texture = gfx/replacement/mask.png }}"),
            [40, 80, 120, 100],
        ),
        (
            format!("spriteType = {{ name = {name} {texture} effectFile = gfx/FX/custom.lua legacy_lazy_load = no
                animation = {{ animationtype = pulsing animationmaskfile = gfx/replacement/mask.png animationtexturefile = gfx/replacement/overlay.png animationblendmode = add }} }}"),
            [90, 80, 120, 200],
        ),
        (
            format!("frameAnimatedSpriteType = {{ name = {name} {texture} effectFile = gfx/FX/custom.lua play_on_show = yes }}"),
            [40, 80, 120, 200],
        ),
    ]
    .into()
}

pub fn shield_flag_program(blend: &str) -> String {
    format!(
        r#"VertexStruct Input {{ float3 position : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct Output {{ float4 position : SV_POSITION;
float2 uv : TEXCOORD0;
float2 flag : TEXCOORD1;
}};
ConstantBuffer(0,0) {{ float4x4 M;
float4 FlagCoords;
float4 Tail;
}};
VertexShader = {{ MainCode V [[
Output main(Input i) {{ Output o; o.position = mul(M, float4(i.position, 1)); o.uv = i.uv; o.flag = FlagCoords.xy + i.uv * FlagCoords.zw; return o; }}
]] }}
PixelShader = {{
Samplers = {{
 Flag {{ Index = 0 MagFilter = point MinFilter = point AddressU = clamp AddressV = clamp }}
 Mask {{ Index = 1 MagFilter = point MinFilter = point AddressU = clamp AddressV = clamp }}
}}
MainCode P [[
float4 main(Output i) : SV_TARGET {{ float4 color = Flag_Texture.Sample(Flag_Sampler, i.flag); color.a = Mask_Texture.Sample(Mask_Sampler, i.uv).a; return color; }}
]] }}
BlendState BlendState {{ BlendEnable = yes {blend} }}
Effect Up {{ VertexShader = V PixelShader = P }}
"#
    )
}

pub fn shield_programs() -> Vec<(&'static str, String)> {
    let over = "SourceBlend = \"SRC_ALPHA\" DestBlend = \"INV_SRC_ALPHA\"";
    vec![
        ("gfx/FX/shield_flag.shader", shield_flag_program(over)),
        (
            "gfx/FX/buttonstate.shader",
            format!("{ARTWORK_SHADER}
BlendState BlendState {{ BlendEnable = yes {over} }}
"),
        ),
    ]
}

const ARTWORK_MAP: &str = "definitions=definitions.csv provinces=grid.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp";

pub fn map_bitmap() -> Vec<u8> {
    let mut bytes = vec![0; 58];
    bytes[..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&58_u32.to_le_bytes());
    bytes[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bytes[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&1_i32.to_le_bytes());
    bytes[22..26].copy_from_slice(&1_i32.to_le_bytes());
    bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bytes
}

#[test]
#[ignore = "requires DXC and a GPU; synthetic content only"]
fn sprite_shaders_execute_stack_effects_masks_and_animation_at_rest() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let lazy_shader = ARTWORK_SHADER.replace("return color;", "color.b = 0.0; return color;");
    let unavailable_shader = ARTWORK_SHADER
        .replace(
            "float4 Clamp;",
            "float4 Clamp;\nfloat4 UnusedSecond[5];\nfloat4 Tail;",
        )
        .replace("return color;", "color.r += Tail.x; return color;");
    let pixel = |rgba: [u8; 4]| {
        png_bytes(&Rgba8Image {
            width: 4,
            height: 4,
            pixels: rgba.repeat(16),
        })
    };
    let (catalogue, _folders) = texture_catalogue(
        &[("interface/art.gfx", "spriteTypes = {
            spriteType = { name = GFX_custom textureFile = gfx/base.png effectFile = outside/test.lua }
            spriteType = { name = GFX_masked textureFile = gfx/base.png effectFile = gfx/FX/test.lua masking_texture = gfx/mask.png }
            spriteType = { name = GFX_animated textureFile = gfx/base.png effectFile = gfx/FX/test.lua legacy_lazy_load = no
                animation = { animationtype = pulsing animationmaskfile = gfx/mask.png animationtexturefile = gfx/overlay.png animationblendmode = add }
            }
            spriteType = { name = GFX_lazy textureFile = gfx/base.png legacy_lazy_load = yes
                animation = { animationtype = pulsing animationmaskfile = gfx/mask.png animationtexturefile = gfx/overlay.png animationblendmode = add }
            }
            spriteType = { name = GFX_plain textureFile = gfx/base.png }
            spriteType = { name = GFX_unknown textureFile = gfx/base.png effectFile = gfx/FX/partial.lua legacy_lazy_load = no
                animation = { animationtype = pulsing animationmaskfile = gfx/mask.png animationtexturefile = gfx/overlay.png }
            }
            frameAnimatedSpriteType = { name = GFX_custom_frames textureFile = gfx/base.png effectFile = gfx/FX/test.lua play_on_show = yes }
        }"), ("gfx/FX/test.shader", ARTWORK_SHADER), ("gfx/FX/buttonstate.shader", &lazy_shader), ("gfx/FX/partial.shader", &unavailable_shader), ("outside/test.shader", ARTWORK_SHADER), ("gfx/FX/defines_hlsl_dx11.fxh", ""), ("map/default.map", ARTWORK_MAP)],
        &[("gfx/base.png", pixel([40,80,120,200])), ("gfx/mask.png", pixel([255,0,0,128])), ("gfx/overlay.png", pixel([50,0,0,255])), ("map/grid.bmp", map_bitmap())],
    );
    let reads = catalogue.content.layers.report_uncovered_reads();
    for (name, expected) in [
        ("GFX_custom", [40, 80, 120, 200]),
        ("GFX_masked", [40, 80, 120, 100]),
        ("GFX_animated", [90, 80, 120, 200]),
        ("GFX_lazy", [40, 80, 0, 200]),
        ("GFX_plain", [40, 80, 0, 200]),
        ("GFX_custom_frames", [40, 80, 120, 200]),
    ] {
        let image = from_url(
            &sprite_frame_png(
                &catalogue.content.layers,
                catalogue.sprites.lookup(name).unwrap(),
                Decode::default(),
                0,
                96,
            )
            .unwrap(),
        );
        assert_eq!([image.width, image.height], [4, 4]);
        assert!(
            image.pixels.chunks_exact(4).all(|pixel| pixel == expected),
            "{name}: {:?}",
            image.pixels
        );
    }
    assert!(reads.warning().unwrap().contains("outside/test.shader"));
    let why = sprite_frame_png(
        &catalogue.content.layers,
        catalogue.sprites.lookup("GFX_unknown").unwrap(),
        Decode::default(),
        0,
        96,
    )
    .unwrap_err()
    .to_string();
    assert!(
        why.contains("GFX_unknown")
            && why.contains("b1 reads unavailable bytes 160..")
            && why.contains("physical size 256"),
        "{why}"
    );
}

#[test]
fn artwork_shader_source_warnings_survive_lazy_reads() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folders) = texture_catalogue(
        &[
            (
                "outside/test.shader",
                "Includes = { absent.fxh } Effect Up { }",
            ),
            ("gfx/FX/defines_hlsl_dx11.fxh", ""),
        ],
        &[],
    );
    let layers = &catalogue.content.layers;
    let reads = layers.report_uncovered_reads();
    let ((program, warnings), _) = content_reader_layers::content::diagnostics::capture(|| {
        let program =
            content_reader_gui::pdxshader::program::read_reported(layers, b"outside/test.lua")
                .unwrap();
        (program, catalogue.reader_warning_lines())
    });
    assert!(warnings.contains(&reads.warning().unwrap()));
    for warning in program
        .warnings
        .iter()
        .filter(|warning| warning.contains("not loaded"))
    {
        assert!(warnings.contains(warning), "{warnings:?}");
    }
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("absent.fxh"))
    );
}
