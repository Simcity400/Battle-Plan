use super::*;
use content_reader_gui::gui::native;
use content_reader_image::Rgba8Image;
use viewer_gpu::test_support::gpu;
use viewer_native_gui::{
    native_gui::{Hit, SpriteInfo, SpriteKind},
    sprites::{IconTextures, Quad, SpritePass, SpriteSheet},
    test_support::gui as gui_fixture,
};

fn cache_key(
    source_id: &str,
    revision: &str,
    game_dir: &str,
    visual_revision: &str,
    window: &str,
) -> WindowCacheKey {
    WindowCacheKey {
        source_id: source_id.into(),
        revision: revision.into(),
        game_dir: PathBuf::from(game_dir),
        user_dir: PathBuf::from("user"),
        visual_revision: visual_revision.into(),
        window: window.into(),
    }
}

#[test]
fn cache_keys_separate_revision_folders_visual_mods_and_evict_on_close() {
    let mut cache: WindowCache<u32> = WindowCache::new(3);
    let k1 = cache_key("src-a", "rev-1", "game-1", "", "ship");
    let k2 = cache_key("src-a", "rev-2", "game-1", "", "ship");
    let k3 = cache_key("src-b", "rev-1", "game-1", "vmod-1", "tank");
    cache.insert(k1.clone(), 1);
    cache.insert(k2.clone(), 2);
    cache.insert(k3.clone(), 3);
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.get_mut(&k1).copied(), Some(1));
    assert_eq!(cache.get_mut(&k2).copied(), Some(2));
    assert_eq!(cache.get_mut(&k3).copied(), Some(3));

    let k4 = cache_key("src-b", "rev-1", "game-2", "vmod-1", "tank");
    cache.insert(k4.clone(), 4);
    assert_eq!(cache.len(), 3);
    assert!(cache.get_mut(&k1).is_none());
    assert_eq!(cache.get_mut(&k4).copied(), Some(4));

    cache.evict_source("src-a");
    assert_eq!(cache.len(), 2);
    assert!(cache.get_mut(&k2).is_none());
    assert_eq!(cache.get_mut(&k3).copied(), Some(3));
    assert_eq!(cache.get_mut(&k4).copied(), Some(4));

    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn regions_map_authored_tooltips_disabled_fallback_close_and_scroll() {
    content_reader_script::test_game::enter();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "synthetic_panel"
                position = { x = 10 y = 20 }
                size = { width = 120 height = 80 }
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
            }
        }"#,
        [400, 300],
    );
    let root = gui.template("synthetic_panel").unwrap();
    let resources = gui_fixture::fixture();
    let localisation: Localisation = [
        ("CLOSE_TIP", "§YClose§! window"),
        ("ACTION_TITLE", "Normal title"),
        ("ACTION_BODY", "Delayed §Gdetail§!"),
        ("ACTION_DISABLED", "§RDisabled§! in replay"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let mut instance = Instance::construct(root, [400, 300], &resources).unwrap();
    instance.gui_show(root, root).unwrap();
    let action = root.find(native::Kind::Button, "action_button").unwrap();
    instance.control(root, action).unwrap().enabled = Some(false);
    instance.tick(root, &resources, STEADY_TICK).unwrap();
    let layout = instance
        .layout_with_regions(root, &resources, None, true)
        .unwrap()
        .unwrap();

    let mut overrides = BTreeMap::new();
    overrides.insert(
        "synthetic_panel.custom".to_owned(),
        GameWindowTooltip {
            title: "Custom".into(),
            body: None,
            detail: None,
        },
    );
    let regions = regions_from_layout(
        &[root],
        &layout.regions,
        [0, 0],
        &resources,
        &localisation,
        &["close_button"],
        &overrides,
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
}

#[test]
fn missing_and_unsupported_windows_are_refused_by_name() {
    let missing = missing_window_refusal("custom_mod_designer_view");
    assert!(missing.contains("custom_mod_designer_view"));
    let unsupported = unsupported_window_kind_refusal("unknownDesigner");
    assert!(unsupported.contains("unknownDesigner"));
}

#[test]
fn offscreen_render_draws_synthetic_window_at_exact_dpr_without_game_files() {
    content_reader_script::test_game::enter();
    let (_guard, device, queue) = gpu::device();
    let gpu = GpuContext::from_device(device, queue);

    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "synthetic_window"
                position = { x = 12 y = 16 }
                size = { width = 40 height = 24 }
                iconType = {
                    name = "plate"
                    position = { x = 4 y = 4 }
                    spriteType = "GFX_panel"
                }
            }
        }"#,
        [200, 120],
    );
    let root = gui.template("synthetic_window").unwrap();

    let mut resources = gui_fixture::fixture();
    let sprite_info = SpriteInfo {
        texture: [20, 12],
        frames: 1,
        vertical: false,
        masked: false,
        animated: false,
        kind: SpriteKind::Plain,
        anchor: [0.0, 0.0],
        effect: Ok(viewer_native_gui::native_effects::synthetic::buttonstate()),
        hit: Hit::Frame,
        missing: None,
        refusals: Vec::new(),
    };
    resources.sprites.insert("GFX_panel", sprite_info);

    struct SyntheticSheets {
        panel: SpriteSheet,
    }
    impl Sheets for SyntheticSheets {
        fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
            (sprite == b"GFX_panel").then_some(&self.panel)
        }
        fn progress(&self, _: &[u8]) -> Option<[&SpriteSheet; 2]> {
            None
        }
        fn masking(&self, _: &[u8]) -> Option<&SpriteSheet> {
            None
        }
        fn mask(&self, _: &[u8]) -> Option<&SpriteSheet> {
            None
        }
    }

    let mut pixels = vec![0u8; 20 * 12 * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[220, 110, 40, 255]);
    }
    let images = vec![dds::Texture::from_rgba8(Rgba8Image {
        width: 20,
        height: 12,
        pixels,
    })];
    let textures = IconTextures::new(gpu.device(), gpu.queue(), &images).unwrap();
    let lazy = LazyTextures::new();
    let mut pass = SpritePass::new(gpu.device(), gpu.queue(), &textures, &lazy, OUTPUT_FORMAT).unwrap();
    let sheets = SyntheticSheets {
        panel: SpriteSheet {
            name: "GFX_panel".into(),
            texture: 0,
            frames: 1,
            size: [20, 12],
        },
    };

    let mut instance = Instance::construct(root, [200, 120], &resources).unwrap();
    instance.gui_show(root, root).unwrap();
    instance.tick(root, &resources, STEADY_TICK).unwrap();
    let layout = instance
        .layout_with_regions(root, &resources, None, true)
        .unwrap()
        .unwrap();

    let bounds = drawn_bounds(&layout);
    assert_eq!(bounds, [4, 4, 20, 12]);
    let mut emitted = Batch::default();
    native_gui::draw(&layout, [-bounds[0], -bounds[1]], &sheets, &mut emitted).unwrap();
    assert!(!emitted.quads.is_empty());

    let plain_batch = Batch::loose(vec![Quad {
        texture: 0,
        rect: [0.0, 0.0, 20.0, 12.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: [1.0, 1.0, 1.0, 1.0],
        rotation: None,
        layer: 0,
        shade: 0.0,
        exact: None,
        anchor: None,
        present: None,
    }]);

    let rendered_1x =
        render_batch_offscreen(&gpu, &mut pass, &plain_batch, [20, 12], [4, 4], 1.0).unwrap();
    assert_eq!(rendered_1x.logical_size, [20, 12]);
    assert_eq!(rendered_1x.pixel_size, [20, 12]);
    assert_eq!(rendered_1x.origin, [4, 4]);
    assert_eq!(&rendered_1x.png[..8], b"\x89PNG\r\n\x1a\n");
    let center_1x = (6 * 20 + 10) * 4;
    assert!(rendered_1x.rgba[center_1x] > 200);
    assert!(rendered_1x.rgba[center_1x + 1] > 80);
    assert!(rendered_1x.rgba[center_1x] > rendered_1x.rgba[center_1x + 2]);

    let rendered_15x =
        render_batch_offscreen(&gpu, &mut pass, &plain_batch, [20, 12], [4, 4], 1.5).unwrap();
    assert_eq!(rendered_15x.logical_size, [20, 12]);
    assert_eq!(rendered_15x.pixel_size, [30, 18]);
    let center_15x = (9 * 30 + 15) * 4;
    assert!(rendered_15x.rgba[center_15x] > 200);
    assert!(rendered_15x.rgba[center_15x] > rendered_15x.rgba[center_15x + 2]);
}
