use super::*;
use content_reader_gui::gui::native;
use content_reader_image::Rgba8Image;
use std::fs;
use viewer_gpu::test_support::gpu;
use viewer_native_gui::{
    sprites::{IconTextures, Quad, SpritePass},
    test_support::gui as gui_fixture,
};

const SYNTHETIC_BUTTON_SHADER: &str = r#"
VertexStruct In {
    float3 at : POSITION;
    float2 uv : TEXCOORD0;
};
VertexStruct Out {
    float4 at : SV_POSITION;
    float2 uv : TEXCOORD0;
};
ConstantBuffer(0, 0) {
    float4x4 world;
    float4 tint;
    float4 offset;
};
VertexShader = { MainCode V [[ Out main(In v) {
    Out o; o.at = mul(world, float4(v.at, 1)); o.uv = v.uv + offset.xy; return o;
} ]] }
PixelShader = {
    Samplers = {
        Base = { Index = 0 }
    }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        return Base_Texture.Sample(Base_Sampler, v.uv) * tint;
    } ]]
}
BlendState AlphaBlend {
    BlendEnable = yes
    SourceBlend = "SRC_ALPHA"
    DestBlend = "INV_SRC_ALPHA"
}
Effect Up { VertexShader = V PixelShader = P BlendState = AlphaBlend }
Effect Down { VertexShader = V PixelShader = P BlendState = AlphaBlend }
Effect Over { VertexShader = V PixelShader = P BlendState = AlphaBlend }
Effect Disable { VertexShader = V PixelShader = P BlendState = AlphaBlend }
"#;

fn make_tga_rgba8(width: u16, height: u16, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(18 + usize::from(width) * usize::from(height) * 4);
    out.extend_from_slice(&[
        0,
        0,
        2,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        (width & 0xff) as u8,
        (width >> 8) as u8,
        (height & 0xff) as u8,
        (height >> 8) as u8,
        32,
        0x28,
    ]);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

fn cache_key(
    source_id: &str,
    revision: &str,
    game_dir: &str,
    user_dir: &str,
    visual_revision: &str,
    language: &str,
    window: &str,
) -> WindowCacheKey {
    WindowCacheKey {
        source_id: source_id.into(),
        revision: revision.into(),
        game_dir: PathBuf::from(game_dir),
        user_dir: PathBuf::from(user_dir),
        visual_revision: visual_revision.into(),
        language: language.into(),
        window: window.into(),
    }
}

#[test]
fn cache_keys_vary_each_field_independently_and_evict_cleanly() {
    let mut cache: WindowCache<u32> = WindowCache::new(8);
    let base = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-1", "english", "ship");
    let diff_source = cache_key("src-b", "rev-1", "game-1", "user-1", "vmod-1", "english", "ship");
    let diff_rev = cache_key("src-a", "rev-2", "game-1", "user-1", "vmod-1", "english", "ship");
    let diff_game = cache_key("src-a", "rev-1", "game-2", "user-1", "vmod-1", "english", "ship");
    let diff_user = cache_key("src-a", "rev-1", "game-1", "user-2", "vmod-1", "english", "ship");
    let diff_visual = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-2", "english", "ship");
    let diff_lang = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-1", "german", "ship");
    let diff_window = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-1", "english", "tank");

    for (idx, key) in [
        &base,
        &diff_source,
        &diff_rev,
        &diff_game,
        &diff_user,
        &diff_visual,
        &diff_lang,
        &diff_window,
    ]
    .into_iter()
    .enumerate()
    {
        cache.insert(key.clone(), idx as u32 + 1);
    }
    assert_eq!(cache.len(), 8);
    assert_eq!(cache.get_mut(&base).copied(), Some(1));
    assert_eq!(cache.get_mut(&diff_source).copied(), Some(2));
    assert_eq!(cache.get_mut(&diff_rev).copied(), Some(3));
    assert_eq!(cache.get_mut(&diff_game).copied(), Some(4));
    assert_eq!(cache.get_mut(&diff_user).copied(), Some(5));
    assert_eq!(cache.get_mut(&diff_visual).copied(), Some(6));
    assert_eq!(cache.get_mut(&diff_lang).copied(), Some(7));
    assert_eq!(cache.get_mut(&diff_window).copied(), Some(8));

    cache.evict_source("src-a");
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get_mut(&diff_source).copied(), Some(2));
    assert!(cache.get_mut(&base).is_none());

    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn regions_map_nested_backgrounds_disabled_fallback_close_and_skip_passthrough() {
    content_reader_script::test_game::enter();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "synthetic_panel"
                position = { x = 10 y = 20 }
                size = { width = 120 height = 80 }
                background = {
                    name = "bg"
                    spriteType = "GFX_even"
                    pdx_tooltip = "BG_TIP"
                }
                buttonType = {
                    name = "close_button"
                    position = { x = 90 y = 4 }
                    spriteType = "GFX_even"
                    pdx_tooltip = "CLOSE_TIP"
                }
                buttonType = {
                    name = "action_button"
                    position = { x = 10 y = 30 }
                    spriteType = "GFX_even"
                    pdx_tooltip = "ACTION_TITLE"
                    pdx_tooltip_delayed = "ACTION_BODY"
                    pdx_disabled_tooltip = "ACTION_DISABLED"
                }
                iconType = {
                    name = "ghost_icon"
                    position = { x = 40 y = 30 }
                    spriteType = "GFX_even"
                    alwaystransparent = yes
                    pdx_tooltip = "GHOST_TIP"
                }
            }
        }"#,
        [400, 300],
    );
    let root = gui.template("synthetic_panel").unwrap();
    let resources = gui_fixture::fixture();
    let localisation: Localisation = [
        ("BG_TIP", "Panel background"),
        ("CLOSE_TIP", "§YClose§! window"),
        ("ACTION_TITLE", "Normal title"),
        ("ACTION_BODY", "Delayed §Gdetail§!"),
        ("ACTION_DISABLED", "§RDisabled§! in replay"),
        ("GHOST_TIP", "Should be skipped"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let mut instance = Instance::construct(root, [400, 300], &resources).unwrap();
    instance.gui_show(root, root).unwrap();
    let action = root.find(native::Kind::Button, "action_button").unwrap();
    instance.control(root, action).unwrap().enabled = Some(false);
    instance.tick(root, &resources, SETTLE_TICK).unwrap();
    instance.tick(root, &resources, STEADY_TICK).unwrap();
    let layout = instance
        .layout_with_regions(root, &resources, None, true)
        .unwrap()
        .unwrap();

    let regions = regions_from_layout(
        &[root],
        &layout.regions,
        [0, 0],
        &resources,
        &localisation,
        &["close_button"],
        &BTreeMap::new(),
    );
    let close = regions.iter().find(|region| region.close).unwrap();
    assert_eq!(close.rect, [90.0, 4.0, 20.0, 10.0]);
    assert_eq!(
        close.tooltip.as_ref().map(|tip| tip.title.as_str()),
        Some("Close window")
    );

    let disabled = regions
        .iter()
        .find(|region| region.path.ends_with("action_button"))
        .unwrap();
    assert!(disabled.disabled);
    assert!(!disabled.close);
    assert_eq!(disabled.rect, [10.0, 30.0, 20.0, 10.0]);
    let tip = disabled.tooltip.as_ref().unwrap();
    assert_eq!(tip.title, "Disabled in replay");
    assert_eq!(tip.body.as_deref(), Some("Delayed detail"));

    assert!(
        !regions.iter().any(|region| region.path.ends_with("ghost_icon")),
        "alwaystransparent regions must not steal hits"
    );

    let icons = RegistryIcons {
        resources: &resources,
    };
    assert_eq!(icons.icon("even"), Some((1, 0)));
    assert_eq!(icons.icon("GFX_even"), Some((1, 0)));
    assert_eq!(icons.icon("even|2"), Some((1, 1)));
    assert!(element_by_path(root, &root.nested()[0].path).is_some());
}

#[test]
fn batch_bounds_covers_rotated_and_overhanging_quads() {
    let mut batch = Batch::default();
    batch.quads.push(Quad {
        texture: 0,
        rect: [10.0, 10.0, 20.0, 10.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [1.0; 4],
        rotation: Some(std::f32::consts::FRAC_PI_2),
        layer: 0,
        shade: 0.0,
        exact: None,
        anchor: None,
        present: None,
    });
    batch.quads.push(Quad {
        texture: 0,
        rect: [0.0, 0.0, 0.0, 0.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [1.0; 4],
        rotation: None,
        layer: 0,
        shade: 0.0,
        exact: Some(viewer_native_gui::sprites::Exact {
            native: None,
            corners: [[0.0, 0.0], [50.0, 0.0], [50.0, 30.0], [0.0, 30.0]],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            scissor: Some([5, 5, 45, 25]),
            technique: 0,
            blend: viewer_native_gui::native_effects::Blend::Alpha,
            samplers: 0,
            lod_bias: [0.0; 3],
            second: None,
            masking: None,
            offsets: [0.0; 4],
            inputs: [0.0; 4],
            second_color: [0.0; 4],
        }),
        anchor: None,
        present: None,
    });
    let bounds = batch_bounds(&batch).unwrap();
    assert_eq!(bounds, [5, 5, 40, 20]);
}

#[test]
fn oversized_surface_is_refused_without_clamping() {
    let (_guard, device, queue) = gpu::device();
    let gpu = GpuContext::from_device(device, queue);
    let page = dds::Texture::from_rgba8(Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![255; 4],
    });
    let textures = IconTextures::new(gpu.device(), gpu.queue(), &[page]).unwrap();
    let lazy = LazyTextures::new();
    let mut pass =
        SpritePass::new(gpu.device(), gpu.queue(), &textures, &lazy, OUTPUT_FORMAT).unwrap();
    let err = render_batch_offscreen(&gpu, &mut pass, &Batch::default(), [3000, 1000], [0, 0], 2.0)
        .err()
        .unwrap();
    assert!(matches!(err, GameWindowError::Refused(_)));
    assert!(err.to_string().contains("4096"));
}

#[test]
fn service_renders_mod_only_window_with_straight_alpha_trim_and_refusals() {
    content_reader_script::test_game::enter();
    let root_dir = std::env::temp_dir().join(format!(
        "battleplan-game-window-mod-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(root_dir.join("gfx/FX")).unwrap();
    fs::create_dir_all(root_dir.join("gfx/ui")).unwrap();
    fs::create_dir_all(root_dir.join("interface")).unwrap();
    fs::create_dir_all(root_dir.join("localisation/english")).unwrap();

    fs::write(root_dir.join("gfx/FX/defines_hlsl_dx11.fxh"), b"").unwrap();
    fs::write(
        root_dir.join("gfx/FX/buttonstate.lua"),
        SYNTHETIC_BUTTON_SHADER.as_bytes(),
    )
    .unwrap();
    fs::write(
        root_dir.join("gfx/FX/text.lua"),
        SYNTHETIC_BUTTON_SHADER.as_bytes(),
    )
    .unwrap();

    let tex_w: usize = 24;
    let tex_h: usize = 16;
    let mut tex_rgba = vec![0u8; tex_w * tex_h * 4];
    for y in 2..14 {
        for x in 2..22 {
            let idx = (y * tex_w + x) * 4;
            if x == 2 && y == 2 {
                tex_rgba[idx..idx + 4].copy_from_slice(&[0, 0, 0, 0]);
            } else if x == 2 || x == 21 || y == 2 || y == 13 {
                tex_rgba[idx..idx + 4].copy_from_slice(&[200, 100, 50, 128]);
            } else {
                tex_rgba[idx..idx + 4].copy_from_slice(&[220, 110, 40, 255]);
            }
        }
    }
    fs::write(
        root_dir.join("gfx/ui/plate.tga"),
        make_tga_rgba8(tex_w as u16, tex_h as u16, &tex_rgba),
    )
    .unwrap();

    fs::write(
        root_dir.join("interface/mod_sprites.gfx"),
        br#"spriteTypes = {
            spriteType = {
                name = "GFX_mod_plate"
                texturefile = "gfx/ui/plate.tga"
            }
            spriteType = {
                name = "GFX_mod_missing"
                texturefile = "gfx/ui/does_not_exist.tga"
            }
        }"#,
    )
    .unwrap();
    fs::write(
        root_dir.join("interface/mod_window.gui"),
        br#"guiTypes = {
            containerWindowType = {
                name = "mod_only_panel"
                position = { x = 10 y = 10 }
                size = { width = 24 height = 16 }
                buttonType = {
                    name = "close_button"
                    position = { x = 0 y = 0 }
                    spriteType = "GFX_mod_plate"
                    pdx_tooltip = "MOD_CLOSE"
                }
            }
            containerWindowType = {
                name = "mod_blank_panel"
                position = { x = 0 y = 0 }
                size = { width = 20 height = 20 }
                iconType = {
                    name = "missing_art"
                    position = { x = 0 y = 0 }
                    spriteType = "GFX_mod_missing"
                }
            }
        }"#,
    )
    .unwrap();
    fs::write(
        root_dir.join("localisation/english/mod_l_english.yml"),
        "\u{feff}l_english:\n MOD_CLOSE:0 \"Close Mod Panel\"\n".as_bytes(),
    )
    .unwrap();

    let layers = ResolvedLayers::from_folders(&[(&root_dir, &[][..])]).unwrap();
    let service = Service::default();
    let cancel = AtomicBool::new(false);
    let key = cache_key(
        "mod-src",
        "rev-1",
        &root_dir.to_string_lossy(),
        "",
        "",
        "english",
        "mod_only_panel",
    );

    let rendered = service
        .render_window_from_layers(
            key.clone(),
            &layers,
            "mod_only_panel",
            &Assets::default(),
            1.0,
            Some(0),
            true,
            &["close_button"],
            &BTreeMap::new(),
            Vec::new(),
            &cancel,
            |_, _, _| {},
        )
        .unwrap();
    assert_eq!(rendered.logical_size, [20, 12]);
    assert_eq!(rendered.pixel_size, [20, 12]);
    let close = rendered.regions.iter().find(|r| r.close).unwrap();
    assert_eq!(close.rect, [-2.0, -2.0, 24.0, 16.0]);

    let png_bytes = decode_base64_for_test(&rendered.png_base64);
    assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n");
    let decoder = png::Decoder::new(std::io::Cursor::new(&png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!([info.width, info.height], [20, 12]);
    let px = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * 20 + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    };
    let center = px(10, 6);
    assert_eq!(center[3], 255);
    assert!((i16::from(center[0]) - 220).abs() <= 2, "{center:?}");
    assert!((i16::from(center[1]) - 110).abs() <= 2, "{center:?}");
    assert!((i16::from(center[2]) - 40).abs() <= 2, "{center:?}");

    let edge = px(10, 0);
    assert!(edge[3] > 110 && edge[3] < 145, "translucent alpha lost: {edge:?}");
    assert!(
        (i16::from(edge[0]) - 200).abs() <= 8
            && (i16::from(edge[1]) - 100).abs() <= 8
            && (i16::from(edge[2]) - 50).abs() <= 8,
        "translucent edge should be unpremultiplied straight RGBA: {edge:?}"
    );
    assert_eq!(px(0, 0)[3], 0, "transparent cutout must keep alpha 0");

    let rendered_15 = service
        .render_window_from_layers(
            key.clone(),
            &layers,
            "mod_only_panel",
            &Assets::default(),
            1.5,
            Some(0),
            true,
            &["close_button"],
            &BTreeMap::new(),
            Vec::new(),
            &cancel,
            |_, _, _| {},
        )
        .unwrap();
    assert_eq!(rendered_15.logical_size, [20, 12]);
    assert_eq!(rendered_15.pixel_size, [30, 18]);

    let missing_err = service
        .render_window_from_layers(
            WindowCacheKey {
                window: "custom_mod_missing_window".into(),
                ..key.clone()
            },
            &layers,
            "custom_mod_missing_window",
            &Assets::default(),
            1.0,
            Some(0),
            true,
            &[],
            &BTreeMap::new(),
            Vec::new(),
            &cancel,
            |_, _, _| {},
        )
        .err()
        .unwrap();
    assert!(matches!(missing_err, GameWindowError::Refused(_)));
    assert!(missing_err.to_string().contains("custom_mod_missing_window"));

    let blank_err = service
        .render_window_from_layers(
            WindowCacheKey {
                window: "mod_blank_panel".into(),
                ..key.clone()
            },
            &layers,
            "mod_blank_panel",
            &Assets::default(),
            1.0,
            Some(0),
            true,
            &[],
            &BTreeMap::new(),
            Vec::new(),
            &cancel,
            |_, _, _| {},
        )
        .err()
        .unwrap();
    assert!(matches!(blank_err, GameWindowError::Refused(_)));

    let cancelled = AtomicBool::new(true);
    let cancel_err = service
        .render_window_from_layers(
            key,
            &layers,
            "mod_only_panel",
            &Assets::default(),
            1.0,
            Some(0),
            true,
            &[],
            &BTreeMap::new(),
            Vec::new(),
            &cancelled,
            |_, _, _| {},
        )
        .err()
        .unwrap();
    assert_eq!(cancel_err, GameWindowError::Cancelled);

    assert_eq!(service.cache_len(), 1);
    service.evict_source("mod-src");
    assert_eq!(service.cache_len(), 0);

    let _ = fs::remove_dir_all(&root_dir);
}

fn decode_base64_for_test(input: &str) -> Vec<u8> {
    let val = |b: u8| -> u8 {
        match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            break;
        }
        let n = ((val(chunk[0]) as u32) << 18)
            | ((val(chunk[1]) as u32) << 12)
            | ((val(chunk[2]) as u32) << 6)
            | (val(chunk[3]) as u32);
        out.push(((n >> 16) & 0xff) as u8);
        if chunk[2] != b'=' {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if chunk[3] != b'=' {
            out.push((n & 0xff) as u8);
        }
    }
    out
}
