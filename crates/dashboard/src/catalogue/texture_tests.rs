//! Dashboard images through a synthetic base/mod stack and the native loaders.

use super::tests::{Folder, folder_catalogue};
use super::*;

pub(crate) fn texture_catalogue(
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
    catalogue.sprites = installed_content::gfx::read(&layers).unwrap();
    catalogue.guis = installed_content::gui::native::read_layers(&layers).unwrap();
    catalogue.content = Arc::new(ResolvedContent {
        layers,
        stats: Default::default(),
    });
    (catalogue, [base, modification])
}

pub(crate) fn png_bytes(image: &Rgba8Image) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(
            png_data_url(image)
                .unwrap()
                .strip_prefix("data:image/png;base64,")
                .unwrap(),
        )
        .unwrap()
}

pub(crate) fn from_url(url: &str) -> Rgba8Image {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(url.strip_prefix("data:image/png;base64,").unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(io::Cursor::new(bytes))
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

/// A mod replaces a base sprite with a PNG strip. Padding is part of the native
/// frame pitch, including when the resulting width does not divide by the frame count.
#[test]
fn a_mod_png_uses_native_frames_with_and_without_padding() {
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

/// Small, valid uncompressed DDS and Targa inputs, both holding top-first BGRA.
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

/// Inputs whose distinct refusals every artwork consumer must preserve.
pub(crate) fn refusal_textures() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let mut device = dds(&strip());
    device[16..20].copy_from_slice(&16385_u32.to_le_bytes());
    let face = Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![255; 4],
    };
    // 9000x9000: within the device limit, past the dashboard's decode budget.
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
        ("gfx/mod/claim.dds", claim, "texel decode budget"),
        (
            "gfx/mod/large.png",
            vec![0x13; (16 << 20) + 1],
            "16 MiB, so it does not load",
        ),
    ]
}

#[test]
fn dds_and_engine_targa_still_decode_and_resolver_candidates_win() {
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
    // Only the native decoder's device limit bounds an edge.
    let image = read_texture(layers, "gfx/mod/wide.png", Decode::default()).unwrap();
    assert_eq!([image.width, image.height], [4097, 1]);
}

#[test]
fn native_failures_keep_their_distinct_reasons() {
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
fn sprite_and_texture_frames_share_fractional_padded_pixels() {
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
        let across = sprite_frame_png(layers, sprite("GFX_mod_sheet"), decode, frame, 96).unwrap();
        assert_eq!(
            across,
            texture_frame_png(layers, "gfx/mod/sheet.png", decode, 3, frame, 96).unwrap()
        );
        // A vertical strip steps down by the same native pitch.
        let down =
            from_url(&sprite_frame_png(layers, sprite("GFX_mod_down"), decode, frame, 96).unwrap());
        assert_eq!(transposed(&down), from_url(&across), "{frame}");
    }
    let why = sprite_frame_png(layers, sprite("GFX_mod_shine"), decode, 0, 96)
        .unwrap_err()
        .to_string();
    assert!(why.contains("needs native sprite rendering"), "{why}");
    for (frames, frame) in [(0, 0), (3, 3), (9, 0)] {
        assert!(
            gui_texture_frame_png(layers, b"gfx/mod/sheet.png", decode, frames, frame, 96).is_err()
        );
    }
    assert!(gui_texture_frame_png(layers, b"gfx/mod/sheet.png", decode, 3, 0, 0).is_err());
}

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
