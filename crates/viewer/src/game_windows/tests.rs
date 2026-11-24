use super::*;
use content_reader_gui::{gui::native, pdxshader};
use content_reader_image::Rgba8Image;
use shader_pipeline::{
    Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput, library::Effect,
};
use std::fs;
use viewer_gpu::test_support::gpu;
use viewer_native_gui::{
    native_effects::Blend,
    native_shader::draw,
    sprites::{Exact, IconTextures, Quad, SpritePass},
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
        Base = {
            Index = 0
            MagFilter = Point
            MinFilter = Point
            MipFilter = Point
            AddressU = Clamp
            AddressV = Clamp
        }
    }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        return Base_Texture.Sample(Base_Sampler, v.uv) * tint;
    } ]]
}
BlendState Alpha {
    BlendEnable = yes
    SourceBlend = src_alpha
    DestBlend = inv_src_alpha
}
Effect Up { VertexShader = V PixelShader = P BlendState = Alpha }
Effect Down { VertexShader = V PixelShader = P BlendState = Alpha }
Effect Over { VertexShader = V PixelShader = P BlendState = Alpha }
Effect Disable { VertexShader = V PixelShader = P BlendState = Alpha }
"#;

const SYNTHETIC_OPAQUE_SHADER: &str = r#"
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
        Base = {
            Index = 0
            MagFilter = Point
            MinFilter = Point
            MipFilter = Point
            AddressU = Clamp
            AddressV = Clamp
        }
    }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        return Base_Texture.Sample(Base_Sampler, v.uv) * tint;
    } ]]
}
Effect Up { VertexShader = V PixelShader = P }
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

const SYNTHETIC_MAP_SIZE: [u32; 2] = [64, 32];

fn rgb_bitmap(width: u32, height: u32) -> Vec<u8> {
    let stride = (width * 3).div_ceil(4) * 4;
    let mut bitmap = vec![0; 54 + (stride * height) as usize];
    bitmap[..2].copy_from_slice(b"BM");
    for (at, value) in [(10, 54_u32), (14, 40), (18, width), (22, height)] {
        bitmap[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bitmap
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
        force_pow2_textures: false,
        art_stack: [0; 32],
        window: window.into(),
    }
}

#[test]
fn cache_keys_vary_each_field_independently_and_evict_cleanly() {
    let mut cache: WindowCache<u32> = WindowCache::new(10);
    let base = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-1", "english", "ship");
    let diff_source = cache_key("src-b", "rev-1", "game-1", "user-1", "vmod-1", "english", "ship");
    let diff_rev = cache_key("src-a", "rev-2", "game-1", "user-1", "vmod-1", "english", "ship");
    let diff_game = cache_key("src-a", "rev-1", "game-2", "user-1", "vmod-1", "english", "ship");
    let diff_user = cache_key("src-a", "rev-1", "game-1", "user-2", "vmod-1", "english", "ship");
    let diff_visual = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-2", "english", "ship");
    let diff_lang = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-1", "german", "ship");
    let diff_window = cache_key("src-a", "rev-1", "game-1", "user-1", "vmod-1", "english", "tank");
    let mut diff_pow2 = base.clone();
    diff_pow2.force_pow2_textures = true;
    let mut diff_art = base.clone();
    diff_art.art_stack = [1; 32];

    for (idx, key) in [
        &base,
        &diff_source,
        &diff_rev,
        &diff_game,
        &diff_user,
        &diff_visual,
        &diff_lang,
        &diff_window,
        &diff_pow2,
        &diff_art,
    ]
    .into_iter()
    .enumerate()
    {
        cache.insert(key.clone(), idx as u32 + 1);
    }
    assert_eq!(cache.len(), 10);
    assert_ne!(base.interface_key(), diff_art.interface_key());
    assert_eq!(cache.get_mut(&base).copied(), Some(1));
    assert_eq!(cache.get_mut(&diff_source).copied(), Some(2));
    assert_eq!(cache.get_mut(&diff_rev).copied(), Some(3));
    assert_eq!(cache.get_mut(&diff_game).copied(), Some(4));
    assert_eq!(cache.get_mut(&diff_user).copied(), Some(5));
    assert_eq!(cache.get_mut(&diff_visual).copied(), Some(6));
    assert_eq!(cache.get_mut(&diff_lang).copied(), Some(7));
    assert_eq!(cache.get_mut(&diff_window).copied(), Some(8));
    assert_eq!(cache.get_mut(&diff_pow2).copied(), Some(9));
    assert_eq!(cache.get_mut(&diff_art).copied(), Some(10));

    let evicted = cache.evict_source("src-a");
    assert_eq!(evicted.len(), 9);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get_mut(&diff_source).copied(), Some(2));
    assert!(cache.get_mut(&base).is_none());

    let cleared = cache.clear();
    assert_eq!(cleared.len(), 1);
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
                buttonType = {
                    name = "ghost_close"
                    position = { x = 60 y = 30 }
                    spriteType = "GFX_even"
                    alwaystransparent = yes
                    pdx_tooltip = "CLOSE_TIP"
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
        [0.0, 0.0],
        &resources,
        &localisation,
        &["close_button", "ghost_close"],
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
    assert!(
        !regions.iter().any(|region| region.path.ends_with("ghost_close")),
        "alwaystransparent close buttons must not become clickable"
    );
    let ghost_tips: BTreeMap<String, GameWindowTooltip> = layout
        .regions
        .iter()
        .filter(|region| region.path.ends_with("ghost_icon") || region.path.ends_with("ghost_close"))
        .map(|region| {
            (
                region.path.clone(),
                GameWindowTooltip {
                    title: "n/a".to_owned(),
                    body: None,
                    detail: None,
                },
            )
        })
        .collect();
    assert_eq!(ghost_tips.len(), 2);
    let explained = regions_from_layout(
        &[root],
        &layout.regions,
        [0.0, 0.0],
        &resources,
        &localisation,
        &["close_button", "ghost_close"],
        &ghost_tips,
    );
    for name in ["ghost_icon", "ghost_close"] {
        let ghost = explained
            .iter()
            .find(|region| region.path.ends_with(name))
            .unwrap();
        assert!(ghost.passthrough, "{name} keeps its tooltip but lets clicks through");
        assert!(!ghost.close);
    }
    assert!(explained.iter().filter(|region| !region.passthrough).all(|region| !region.path.contains("ghost")));

    let alpha_gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "alpha_panel"
                position = { x = 0 y = 0 }
                size = { width = 20 height = 10 }
                buttonType = {
                    name = "close_button"
                    position = { x = 0 y = 0 }
                    spriteType = "GFX_alpha_hole"
                    pdx_tooltip = "CLOSE_TIP"
                }
            }
        }"#,
        [400, 300],
    );
    let alpha_root = alpha_gui.template("alpha_panel").unwrap();
    let mut alpha_resources = gui_fixture::fixture();
    alpha_resources.physical = Some([400, 300]);
    alpha_resources.add_sprite("GFX_alpha_hole", [4, 2], 1);
    alpha_resources.sprites.get_mut("GFX_alpha_hole").unwrap().hit = Hit::Alpha;
    alpha_resources.alpha.insert(
        "GFX_alpha_hole",
        native_gui::AlphaMask {
            width: 4,
            transparent: Arc::from([1, 0, 0, 1, 0, 1, 1, 0]),
        },
    );
    let mut alpha_instance = Instance::construct(alpha_root, [400, 300], &alpha_resources).unwrap();
    alpha_instance.gui_show(alpha_root, alpha_root).unwrap();
    alpha_instance
        .tick(alpha_root, &alpha_resources, SETTLE_TICK)
        .unwrap();
    alpha_instance
        .tick(alpha_root, &alpha_resources, STEADY_TICK)
        .unwrap();
    let alpha_layout = alpha_instance
        .layout_with_regions(alpha_root, &alpha_resources, None, true)
        .unwrap()
        .unwrap();
    let alpha_regions = regions_from_layout(
        &[alpha_root],
        &alpha_layout.regions,
        [0.0, 0.0],
        &alpha_resources,
        &localisation,
        &["close_button"],
        &BTreeMap::new(),
    );
    let alpha_close = alpha_regions.iter().find(|region| region.close).unwrap();
    let mask = alpha_close.alpha_mask.as_ref().unwrap();
    assert_eq!([mask.width, mask.height], [4, 2]);
    assert_eq!(mask.bits_base64, encode_base64(&[0b1001_0110]));

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

struct SyntheticModFixture {
    root_dir: PathBuf,
    layers: ResolvedLayers,
}

impl SyntheticModFixture {
    fn create(button_shader: &str) -> Self {
        content_reader_script::test_game::enter();
        let root_dir = std::env::temp_dir().join(format!(
            "battleplan-game-window-mod-test-{}",
            uuid::Uuid::new_v4()
        ));
        let base_dir = root_dir.join("base");
        let mod_dir = root_dir.join("mod");
        fs::create_dir_all(base_dir.join("gfx/FX")).unwrap();
        fs::create_dir_all(base_dir.join("gfx/ui")).unwrap();
        fs::create_dir_all(base_dir.join("map")).unwrap();
        fs::create_dir_all(mod_dir.join("gfx/ui")).unwrap();
        fs::create_dir_all(mod_dir.join("interface")).unwrap();
        fs::create_dir_all(mod_dir.join("localisation/english")).unwrap();
        fs::write(
            base_dir.join("map/default.map"),
            b"definitions=d.csv provinces=provinces.bmp terrain=t.bmp rivers=r.bmp heightmap=h.bmp tree_definition=tr.bmp",
        )
        .unwrap();
        fs::write(
            base_dir.join("map/provinces.bmp"),
            rgb_bitmap(SYNTHETIC_MAP_SIZE[0], SYNTHETIC_MAP_SIZE[1]),
        )
        .unwrap();
        fs::write(
            base_dir.join("gfx/ui/plate.tga"),
            make_tga_rgba8(24, 16, &[10, 200, 10, 255].repeat(24 * 16)),
        )
        .unwrap();

        fs::write(base_dir.join("gfx/FX/defines_hlsl_dx11.fxh"), b"").unwrap();
        for path in [
            "gfx/FX/buttonstate.shader",
            "gfx/FX/buttonstate.lua",
            "gfx/FX/text.shader",
            "gfx/FX/text.lua",
        ] {
            fs::write(base_dir.join(path), button_shader.as_bytes()).unwrap();
        }

        let tex_rgba = synthetic_plate_rgba();
        fs::write(
            mod_dir.join("gfx/ui/plate.tga"),
            make_tga_rgba8(24, 16, &tex_rgba),
        )
        .unwrap();

        fs::write(
            mod_dir.join("interface/mod_sprites.gfx"),
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
            mod_dir.join("interface/mod_window.gui"),
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
            mod_dir.join("localisation/english/mod_l_english.yml"),
            "\u{feff}l_english:\n MOD_CLOSE:0 \"Close Mod Panel\"\n".as_bytes(),
        )
        .unwrap();

        let layers =
            ResolvedLayers::from_folders(&[(&base_dir, &[][..]), (&mod_dir, &[][..])]).unwrap();
        Self { root_dir, layers }
    }

    fn key(&self, window: &str) -> WindowCacheKey {
        let mut key = cache_key(
            "mod-src",
            "rev-1",
            &self.root_dir.to_string_lossy(),
            "",
            "",
            "english",
            window,
        );
        key.art_stack = self.layers.art_stack_key();
        key
    }
}

impl Drop for SyntheticModFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root_dir);
    }
}

#[test]
fn service_loads_mod_only_window_and_refuses_missing_blank_or_cancelled() {
    let fixture = SyntheticModFixture::create(SYNTHETIC_BUTTON_SHADER);
    let service = Service::default();
    let cancel = AtomicBool::new(false);

    let shared = SharedInterface::load(&fixture.layers, DEFAULT_VIEWPORT, Some("english")).unwrap();
    assert!(shared.guis.template(b"mod_only_panel").is_some());
    assert!(shared.guis.template(b"unit_counter_navy").is_none());

    let empty_tips = BTreeMap::new();
    let missing_err = service
        .render_composed_from_layers(
            fixture.key("custom_mod_missing_window"),
            &fixture.layers,
            RenderParams {
                window_name: "custom_mod_missing_window",
                scale: 1.0,
                read_hour: Some(0),
                art_from_current_install: true,
                close_paths: &[],
                tooltip_overrides: &empty_tips,
                extra_warnings: Vec::new(),
                admitted_gen: None,
                scroll: &BTreeMap::new(),
            },
            &cancel,
            |_| Ok(Prepared::uncached(Assets::default(), ())),
            |_, _, _, _, _, _, _| {},
            |_, _, _, _, _, _, _| Ok(()),
        )
        .err()
        .unwrap();
    assert!(matches!(missing_err, GameWindowError::Refused(_)));
    assert!(missing_err.to_string().contains("custom_mod_missing_window"));

    let blank_err = service
        .render_composed_from_layers(
            fixture.key("mod_blank_panel"),
            &fixture.layers,
            RenderParams {
                window_name: "mod_blank_panel",
                scale: 1.0,
                read_hour: Some(0),
                art_from_current_install: true,
                close_paths: &[],
                tooltip_overrides: &empty_tips,
                extra_warnings: Vec::new(),
                admitted_gen: None,
                scroll: &BTreeMap::new(),
            },
            &cancel,
            |_| Ok(Prepared::uncached(Assets::default(), ())),
            |_, _, _, _, _, _, _| {},
            |_, _, _, _, _, _, _| Ok(()),
        )
        .err()
        .unwrap();
    assert!(matches!(blank_err, GameWindowError::Refused(_)));

    let cancelled = AtomicBool::new(true);
    let cancel_err = service
        .render_composed_from_layers(
            fixture.key("mod_only_panel"),
            &fixture.layers,
            RenderParams {
                window_name: "mod_only_panel",
                scale: 1.0,
                read_hour: Some(0),
                art_from_current_install: true,
                close_paths: &[],
                tooltip_overrides: &empty_tips,
                extra_warnings: Vec::new(),
                admitted_gen: None,
                scroll: &BTreeMap::new(),
            },
            &cancelled,
            |_| Ok(Prepared::uncached(Assets::default(), ())),
            |_, _, _, _, _, _, _| {},
            |_, _, _, _, _, _, _| Ok(()),
        )
        .err()
        .unwrap();
    assert_eq!(cancel_err, GameWindowError::Cancelled);
}

fn compiled_wgsl_effect(layers: &ResolvedLayers, path: &[u8]) -> Arc<Effect> {
    let program = pdxshader::program::read(layers, path).unwrap();
    let header =
        "struct Constants { m: mat4x4<f32>, color: vec4<f32>, offsets: vec4<f32>, clocks: vec4<f32> }";
    let shader = |wgsl: String| Shader {
        wgsl,
        independent_draws: false,
        buffers: vec![],
        textures: vec![],
        samplers: vec![],
        sampling_pairs: vec![],
        inputs: vec![],
        clock: Default::default(),
    };
    let mut vertex = shader(format!(
        "{header}\n{}",
        r#"
@group(0) @binding(0) var<uniform> c: Constants;
struct Input { @location(0) position: vec3<f32>, @location(1) uv: vec2<f32> }
struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn main(v: Input) -> Output { var o: Output; o.position = c.m * vec4<f32>(v.position, 1.0); o.uv = v.uv; return o; }
"#
    ));
    vertex.buffers = vec![Buffer {
        binding: 0,
        size: 112,
        reads: std::iter::once(0..64).collect(),
    }];
    vertex.inputs = [("POSITION", 3), ("TEXCOORD0", 2)]
        .into_iter()
        .enumerate()
        .map(|(i, (semantic, components))| VertexInput {
            kind: ScalarKind::Float,
            location: i as u32,
            semantic: semantic.into(),
            components,
        })
        .collect();
    let mut pixel = shader(format!(
        "{header}\n{}",
        r#"
@group(0) @binding(1) var<uniform> c: Constants;
@group(1) @binding(1) var image: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;
@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return textureSample(image, image_sampler, uv) * c.color; }
"#
    ));
    pixel.buffers = vec![Buffer {
        binding: 1,
        size: 112,
        reads: std::iter::once(64..80).collect(),
    }];
    pixel.textures = vec![(1, TextureKind::D2)];
    pixel.samplers = vec![1];
    pixel.sampling_pairs = vec![(1, 1)];
    let compiled = Arc::new(Compiled { vertex, pixel });
    let count = program.techniques.len();
    Effect::from_compiled(Arc::new(program), vec![], vec![Ok(compiled); count])
}

fn synthetic_plate_rgba() -> Vec<u8> {
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
    tex_rgba
}

fn plate_batch(effect: Arc<Effect>, width: f32, height: f32) -> Batch {
    let mut constants = vec![0.0_f32; 28];
    for i in [0, 5, 10, 15] {
        constants[i] = 1.0;
    }
    constants[16..20].fill(1.0);
    let vertices: [f32; 20] = [
        0.0, -height, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, width, -height, 0.0, 1.0, 1.0,
        width, 0.0, 0.0, 1.0, 0.0,
    ];
    let native = draw::Quad {
        path: "mod_only_panel/close_button".into(),
        effect: draw::Effect {
            asset: effect,
            technique: "Up".into(),
        },
        declaration: shader_pipeline::vertices::Declaration::Sprite,
        vertices: bytemuck::cast_slice(&vertices).to_vec(),
        stride: 20,
        source_draws: 1,
        constants,
        known: 112,
        pixel_constants: true,
        textures: vec![(0, Some(0))],
        extra_constants: vec![],
        font_viewport: None,
        topology: wgpu::PrimitiveTopology::TriangleStrip,
        default_blend: false,
        bake: None,
    };
    let mut batch = Batch::default();
    batch.quads.push(Quad {
        texture: 0,
        rect: [0.0, 0.0, width, height],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [1.0; 4],
        rotation: None,
        layer: 0,
        shade: 0.0,
        anchor: None,
        present: None,
        exact: Some(Exact {
            native: Some(Arc::new(native)),
            corners: [[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            scissor: None,
            technique: 0,
            blend: Blend::Off,
            samplers: 0,
            lod_bias: [0.0; 3],
            second: None,
            masking: None,
            offsets: [0.0; 4],
            inputs: [0.0; 4],
            second_color: [0.0; 4],
        }),
    });
    batch
}

#[test]
fn service_renders_straight_alpha_translucent_edges_and_trims_transparent_border() {
    let fixture = SyntheticModFixture::create(SYNTHETIC_BUTTON_SHADER);
    let (_guard, device, queue) = gpu::device();
    let gpu = GpuContext::from_device(device, queue);
    let plate = dds::Texture::from_rgba8(Rgba8Image {
        width: 24,
        height: 16,
        pixels: synthetic_plate_rgba(),
    });
    let textures = IconTextures::new(gpu.device(), gpu.queue(), &[plate]).unwrap();
    let lazy = LazyTextures::new();
    let mut pass =
        SpritePass::new(gpu.device(), gpu.queue(), &textures, &lazy, OUTPUT_FORMAT).unwrap();
    let effect = compiled_wgsl_effect(&fixture.layers, b"gfx/FX/buttonstate.shader");
    let batch = plate_batch(effect, 24.0, 16.0);

    let output = render_batch_offscreen(&gpu, &mut pass, &batch, [24, 16], [10, 10], 1.0).unwrap();
    assert_eq!(output.logical_size, [20.0, 12.0]);
    assert_eq!(output.pixel_size, [20, 12]);
    assert_eq!(output.origin, [12.0, 12.0]);

    assert_eq!(&output.png[..8], b"\x89PNG\r\n\x1a\n");
    let decoder = png::Decoder::new(std::io::Cursor::new(&output.png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!([info.width, info.height], [20, 12]);
    let px = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * 20 + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    };

    assert_eq!(
        px(10, 6),
        [220, 110, 40, 255],
        "opaque interior must stay exact opaque RGBA"
    );
    assert_eq!(
        px(0, 0),
        [0, 0, 0, 0],
        "transparent cutout must stay exact alpha 0"
    );

    let edge = px(10, 0);
    assert!(
        edge[3].abs_diff(128) <= 1,
        "translucent alpha lost: {edge:?}"
    );
    assert!(
        edge[0].abs_diff(200) <= 2 && edge[1].abs_diff(100) <= 2 && edge[2].abs_diff(50) <= 2,
        "translucent edge must be unpremultiplied straight RGBA: {edge:?}"
    );

    let output_15 =
        render_batch_offscreen(&gpu, &mut pass, &batch, [24, 16], [10, 10], 1.5).unwrap();
    assert_eq!(output_15.logical_size, [20.0, 12.0]);
    assert_eq!(output_15.pixel_size, [30, 18]);
    assert_eq!(output_15.origin, [12.0, 12.0]);
}

#[test]
fn trim_transparent_edges_crops_exact_device_pixels_at_fractional_dpr() {
    let width = 12usize;
    let height = 9usize;
    let mut rgba = vec![0u8; width * height * 4];
    for y in 1..8 {
        for x in 2..11 {
            let idx = (y * width + x) * 4;
            rgba[idx..idx + 4].copy_from_slice(&[200, 100, 50, 255]);
        }
    }
    let (cropped, logical, pixels, origin) =
        trim_transparent_edges(rgba, [width as u32, height as u32], [10, 20], 1.5).unwrap();
    assert_eq!(pixels, [9, 7]);
    assert!((logical[0] - 6.0).abs() < 1e-4);
    assert!((logical[1] - (7.0 / 1.5)).abs() < 1e-4);
    assert!((origin[0] - (10.0 + 2.0 / 1.5)).abs() < 1e-4);
    assert!((origin[1] - (20.0 + 1.0 / 1.5)).abs() < 1e-4);
    for px in cropped.chunks_exact(4) {
        assert_eq!(px[3], 255);
    }

    let mut rgba2 = vec![0u8; 10 * 6 * 4];
    for y in 1..5 {
        for x in 1..9 {
            let idx = (y * 10 + x) * 4;
            rgba2[idx..idx + 4].copy_from_slice(&[10, 20, 30, 255]);
        }
    }
    let (_, logical2, pixels2, origin2) =
        trim_transparent_edges(rgba2, [10, 6], [4, 8], 2.0).unwrap();
    assert_eq!(pixels2, [8, 4]);
    assert_eq!(logical2, [4.0, 2.0]);
    assert_eq!(origin2, [4.5, 8.5]);
}

#[test]
fn stale_generation_cancels_without_evicting_newer_cache_entry() {
    let fixture = SyntheticModFixture::create(SYNTHETIC_BUTTON_SHADER);
    let service = Service::default();
    let stale_gen = service.capture_generation("mod-src");
    service.evict_source("mod-src");
    let fresh_gen = service.capture_generation("mod-src");
    assert_ne!(stale_gen, fresh_gen);

    let key = fixture.key("mod_only_panel");
    let shared = Arc::new(SharedInterface::load(&fixture.layers, DEFAULT_VIEWPORT, Some("english")).unwrap());
    {
        let mut cache = service.cache.lock().unwrap();
        cache
            .interfaces
            .insert(key.interface_key(), Arc::clone(&shared));
    }

    let cancel = AtomicBool::new(false);
    let empty_tips = BTreeMap::new();
    let err = service
        .render_composed_from_layers(
            key.clone(),
            &fixture.layers,
            RenderParams {
                window_name: "mod_only_panel",
                scale: 1.0,
                read_hour: Some(0),
                art_from_current_install: true,
                close_paths: &["close_button"],
                tooltip_overrides: &empty_tips,
                extra_warnings: Vec::new(),
                admitted_gen: Some(stale_gen),
                scroll: &BTreeMap::new(),
            },
            &cancel,
            |_| Ok(Prepared::uncached(Assets::default(), ())),
            |_, _, _, _, _, _, _| {},
            |_, _, _, _, _, _, _| Ok(()),
        )
        .err()
        .unwrap();
    assert_eq!(err, GameWindowError::Cancelled);
    assert_eq!(service.capture_generation("mod-src"), fresh_gen);
    let cache = service.cache.lock().unwrap();
    assert!(cache.interfaces.contains_key(&key.interface_key()));
}

#[test]
fn a_render_admitted_before_its_source_is_released_is_cancelled_and_caches_nothing() {
    let fixture = SyntheticModFixture::create(SYNTHETIC_BUTTON_SHADER);
    let service = Service::default();
    let released_while_acquiring = service
        .admit("mod-src", || {
            service.evict_source("mod-src");
            Ok::<_, ()>(())
        })
        .unwrap();
    assert!(released_while_acquiring.is_none());

    let ((), admitted) = service.admit("mod-src", || Ok::<_, ()>(())).unwrap().unwrap();
    service.evict_source("mod-src");
    let cancel = AtomicBool::new(false);
    let empty_tips = BTreeMap::new();
    let err = service
        .render_composed_from_layers(
            fixture.key("mod_only_panel"),
            &fixture.layers,
            RenderParams {
                window_name: "mod_only_panel",
                scale: 1.0,
                read_hour: Some(0),
                art_from_current_install: true,
                close_paths: &["close_button"],
                tooltip_overrides: &empty_tips,
                extra_warnings: Vec::new(),
                admitted_gen: Some(admitted),
                scroll: &BTreeMap::new(),
            },
            &cancel,
            |_| {
                Ok(Prepared {
                    assets: Assets::default(),
                    state: (),
                    fingerprint: Some([7; 32]),
                })
            },
            |_, _, _, _, _, _, _| {},
            |_, _, _, _, _, _, _| Ok(()),
        )
        .err()
        .unwrap();
    assert_eq!(err, GameWindowError::Cancelled);
    let cache = service.cache.lock().unwrap();
    assert!(cache.interfaces.is_empty());
    assert!(cache.windows.is_empty());
    assert_eq!(cache.renders.len(), 0);
}

#[test]
fn rendered_frames_are_reused_by_fingerprint_bounded_and_dropped_with_their_source() {
    let render = |title: &str| GameWindowRender {
        title: Some(title.to_owned()),
        png_base64: String::new(),
        logical_size: [1.0, 1.0],
        pixel_size: [1, 1],
        scale: 1.0,
        read_hour: Some(0),
        art_from_current_install: true,
        art_label: None,
        regions: Vec::new(),
        warnings: Vec::new(),
    };
    let frame = |source: &str, fingerprint: u8| RenderFrameKey {
        window: cache_key(source, "rev-1", "game", "user", "", "english", "ship"),
        scale_bits: 1.0_f32.to_bits(),
        fingerprint: [fingerprint; 32],
    };
    let mut cache = RenderFrameCache::new(2);
    cache.insert(frame("a", 1), render("one"));
    cache.insert(frame("a", 2), render("two"));
    assert_eq!(cache.get(&frame("a", 1)).and_then(|hit| hit.title).as_deref(), Some("one"));
    cache.insert(frame("b", 3), render("three"));
    assert_eq!(cache.len(), 2);
    assert!(cache.get(&frame("a", 2)).is_none(), "the least recently used frame is evicted");
    assert!(cache.get(&frame("a", 1)).is_some());
    cache.evict_source("a");
    assert_eq!(cache.len(), 1);
    assert!(cache.get(&frame("b", 3)).is_some());

    let window = [9; 32];
    let unscrolled = BTreeMap::new();
    let base = frame_fingerprint(window, Some(10), true, &[], &unscrolled);
    assert_ne!(base, frame_fingerprint(window, Some(11), true, &[], &unscrolled));
    assert_ne!(base, frame_fingerprint(window, Some(10), true, &["warned".to_owned()], &unscrolled));
    assert_ne!(base, frame_fingerprint([8; 32], Some(10), true, &[], &unscrolled));
    let scrolled = BTreeMap::from([("root/list".to_owned(), 30.0)]);
    assert_ne!(base, frame_fingerprint(window, Some(10), true, &[], &scrolled));
    assert_ne!(
        frame_fingerprint(window, Some(10), true, &[], &scrolled),
        frame_fingerprint(window, Some(10), true, &[], &BTreeMap::from([("root/list".to_owned(), 45.0)]))
    );
}

#[test]
fn sub_window_regions_take_their_place_in_paint_order() {
    let region = |path: &str| GameWindowRegion {
        path: path.to_owned(),
        rect: [0.0, 0.0, 1.0, 1.0],
        clip: None,
        disabled: false,
        close: false,
        passthrough: false,
        view: None,
        tooltip: None,
        alpha_mask: None,
        scroll: None,
    };
    let parts = ["root", "root.backdrop", "root.slots", "root.close_button"];
    let ordered = paint_ordered_regions(
        &parts,
        vec![region("root.backdrop"), region("root.close_button")],
        vec![(2, vec![region("sub.slot")]), (9, vec![region("sub.top")])],
    );
    let paths: Vec<&str> = ordered.iter().map(|region| region.path.as_str()).collect();
    assert_eq!(paths, ["root.backdrop", "sub.slot", "root.close_button", "sub.top"]);
}

#[test]
fn production_effect_compilation_and_gui_binding_renders_mod_window() {
    let fixture = SyntheticModFixture::create(SYNTHETIC_BUTTON_SHADER);
    let (_guard, device, queue) = gpu::device();
    let gpu = GpuContext::from_device(device, queue);
    let shared = SharedInterface::load(&fixture.layers, DEFAULT_VIEWPORT, Some("english")).unwrap();
    let mut cached = CachedWindow::load(
        &gpu,
        &fixture.layers,
        &shared,
        "mod_only_panel",
        &Assets::default(),
        Decode::default(),
    )
    .unwrap();
    let root = cached.guis.template(b"mod_only_panel").unwrap();
    let assets = cached.gui_assets(DEFAULT_VIEWPORT.map(|v| v as i32));
    let layout = layout_window(
        root,
        &assets,
        DEFAULT_VIEWPORT.map(|v| v as i32),
        |_| {},
    )
    .unwrap();
    let CachedWindow {
        guis,
        gui,
        localisation,
        pass,
        ..
    } = &mut cached;
    let draw_assets = WindowGuiAssets {
        gui,
        guis,
        localisation,
        physical_viewport: Some(DEFAULT_VIEWPORT.map(|v| v as i32)),
    };
    let output = match render_layout_offscreen(&gpu, pass, &layout, &draw_assets, 1.0) {
        Ok(rendered) => rendered,
        Err(GameWindowError::Failed(msg)) if msg.contains("DXC not found") => {
            let mut probe = Batch::default();
            native_gui::draw(&layout, [0, 0], &draw_assets, &mut probe).unwrap();
            let [min_x, min_y, width, height] = batch_bounds(&probe).unwrap();
            let mut batch = Batch::default();
            native_gui::draw(&layout, [-min_x, -min_y], &draw_assets, &mut batch).unwrap();
            assert!(
                !batch.quads.is_empty(),
                "CachedWindow + layout_window must emit bound quads"
            );
            let native_quad = batch.quads[0]
                .exact
                .as_ref()
                .and_then(|exact| exact.native.as_ref())
                .expect("production GuiResources must attach parsed native shader");
            assert_eq!(native_quad.effect.technique.as_str(), "Up");
            assert_eq!(
                native_quad.effect.asset.defines,
                shader_pipeline::library::globals(SYNTHETIC_MAP_SIZE, Decode::default())
            );
            let wgsl_effect = compiled_wgsl_effect(&fixture.layers, b"gfx/FX/buttonstate.shader");
            for quad in &mut batch.quads {
                if let Some(exact) = quad.exact.as_mut()
                    && let Some(native) = exact.native.as_mut()
                {
                    Arc::make_mut(native).effect.asset = Arc::clone(&wgsl_effect);
                }
            }
            render_batch_offscreen(
                &gpu,
                pass,
                &batch,
                [width as u32, height as u32],
                [min_x, min_y],
                1.0,
            )
            .unwrap()
        }
        Err(other) => panic!("unexpected render failure: {other}"),
    };
    assert_eq!(output.pixel_size, [20, 12]);
    assert_eq!(output.logical_size, [20.0, 12.0]);
    assert_eq!(output.origin, [2.0, 2.0]);
    let center = ((6 * 20 + 10) * 4) as usize;
    assert_eq!(&output.rgba[center..center + 4], &[220, 110, 40, 255]);
}

#[test]
fn service_refuses_non_blended_shader_in_layer_mode() {
    let fixture = SyntheticModFixture::create(SYNTHETIC_OPAQUE_SHADER);
    let (_guard, device, queue) = gpu::device();
    let gpu = GpuContext::from_device(device, queue);
    let plate = dds::Texture::from_rgba8(Rgba8Image {
        width: 24,
        height: 16,
        pixels: synthetic_plate_rgba(),
    });
    let textures = IconTextures::new(gpu.device(), gpu.queue(), &[plate]).unwrap();
    let lazy = LazyTextures::new();
    let mut pass =
        SpritePass::new(gpu.device(), gpu.queue(), &textures, &lazy, OUTPUT_FORMAT).unwrap();
    let effect = compiled_wgsl_effect(&fixture.layers, b"gfx/FX/buttonstate.shader");
    let batch = plate_batch(effect, 24.0, 16.0);

    let err = render_batch_offscreen(&gpu, &mut pass, &batch, [24, 16], [0, 0], 1.0)
        .err()
        .unwrap();
    assert!(matches!(err, GameWindowError::Failed(_)));
    assert!(
        err.to_string().contains("without blending"),
        "expected non-blended layer refusal, got: {err}"
    );
}
