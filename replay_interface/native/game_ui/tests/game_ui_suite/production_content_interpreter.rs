use game_ui::content_interpreter::ProductionContentInterpreter;
use map_viewer::{GameFileResolver, PlaybackEnvironment, RecordedContentSource};
use replay_core::{
    ContentLayerKind, ContentManifest, DlcSelection, LayerStorageKind, RecordedContentLayer,
    RequiredContent, RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
    UsedContentKey,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "hoi4-production-content-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn write(root: &Path, rel: &str, value: &str) {
    let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value).unwrap();
}

fn layer_kind(kind: &str) -> ContentLayerKind {
    match kind {
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => ContentLayerKind::Base,
    }
}

fn manifest_for(
    resolver: &GameFileResolver,
    paths: impl IntoIterator<Item = String>,
    language: &str,
    dlc_selection: Vec<DlcSelection>,
) -> (ContentManifest, PlaybackEnvironment) {
    let resolved = resolver.resolve_used_content(paths).unwrap();
    let used_ordinals = resolved
        .iter()
        .map(|row| row.source.ordinal)
        .collect::<BTreeSet<_>>();
    let effective_vfs_manifest_sha256 = "ab".repeat(32);
    let ordered_layers = resolver
        .active_sources()
        .into_iter()
        .map(|source| RecordedContentLayer {
            ordinal: source.ordinal as u32,
            layer_kind: layer_kind(&source.kind),
            display_name: source.source_id.clone(),
            layer_id: source.source_id,
            descriptor_sha256: source.descriptor_sha256,
            source_container_sha256: source.archive_sha256.clone(),
            storage_kind: if source.archive_sha256.is_some() {
                LayerStorageKind::ZipArchive
            } else {
                LayerStorageKind::Directory
            },
            replace_paths: source.replace_path,
            required_for_playback: used_ordinals.contains(&source.ordinal),
        })
        .collect::<Vec<_>>();
    let required_content = resolved
        .into_iter()
        .map(|row| RequiredContent {
            content_key: row.logical_path.to_ascii_lowercase(),
            kind: RequiredContentKind::Other,
            observation_count: 1,
            consumers: vec!["game_ui.production_content_interpreter.test".into()],
            content: UsedContentKey {
                effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.clone(),
                stored_game_relative_path: row.source_relative_path.clone(),
                normalized_game_relative_path: row.logical_path.to_ascii_lowercase(),
                winning_layer_ordinal: row.source.ordinal as u32,
                layer_kind: layer_kind(&row.source.kind),
                layer_id: row.source.source_id,
                source_sha256: row
                    .source
                    .archive_sha256
                    .unwrap_or_else(|| row.sha256.clone()),
                payload_sha256: row.sha256,
                payload_bytes: row.byte_len,
            },
        })
        .collect();
    let source_game = SourceGameIdentity {
        version: "1.19.2.0".into(),
        build: "a729".into(),
        executable_sha256: "87".repeat(32),
        build_profile: "test".into(),
    };
    let mut source_profile = SourceProfileIdentity {
        profile_id: "portable-profile".into(),
        playset_id: "portable-playset".into(),
        selection_sha256: "00".repeat(32),
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "99".repeat(32),
        source_game: source_game.clone(),
        source_profile: source_profile.clone(),
        effective_vfs_manifest_sha256,
        ordered_layers,
        dlc_selection: dlc_selection.clone(),
        localisation_language: language.into(),
        required_settings: BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content,
        manifest_digest_sha256: None,
    };
    source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.source_profile = source_profile.clone();
    let manifest = manifest.with_computed_digest().unwrap();
    let environment = PlaybackEnvironment {
        source_game,
        source_profile,
        dlc_selection,
        localisation_language: language.into(),
        settings: BTreeMap::new(),
    };
    (manifest, environment)
}

fn write_content_fixture(game: &Path, userdata: &Path, mod_content: &Path) {
    write(
        game,
        "interface/panel.gui",
        "guiTypes = { windowType = { name = base_panel } }",
    );
    write(
        game,
        "interface/icons.gfx",
        "spriteTypes = { spriteType = { name = GFX_base texturefile = gfx/interface/base.dds } }",
    );
    write(
        game,
        "localisation/french/content_l_french.yml",
        "l_french:\n PANEL_TITLE:0 \"Base\"\n",
    );
    write(
        game,
        "common/scripted_guis/panel.txt",
        "scripted_gui = { base_panel = { context_type = player_context } }",
    );
    write(
        game,
        "common/defines/runtime.lua",
        "NRuntime = { VALUE = 1 }",
    );
    write(game, "map/runtime.txt", "1 3 10 11 12\n");
    write(
        mod_content,
        "interface/panel.gui",
        "guiTypes = { windowType = { name = mod_panel } }",
    );
    write(
        mod_content,
        "interface/icons.gfx",
        "spriteTypes = { spriteType = { name = GFX_mod texturefile = gfx/interface/mod.dds } }",
    );
    write(
        mod_content,
        "localisation/french/content_l_french.yml",
        "l_french:\n PANEL_TITLE:0 \"Modifie\"\n",
    );
    write(
        mod_content,
        "common/scripted_guis/panel.txt",
        "scripted_gui = { mod_panel = { context_type = player_context window_name = mod_panel } }",
    );
    write(
        mod_content,
        "gfx/entities/runtime.asset",
        "entity = { name = mod_entity material = mod_material particle = mod_particle }",
    );
    write(
        mod_content,
        "gfx/FX/runtime.shader",
        "Effect mod_effect { VertexShader = \"VS\" PixelShader = \"PS\" }",
    );
    write(
        mod_content,
        "gfx/models/runtime.mesh",
        "@@t@pdxasset i 2 1 0\n[object]\n",
    );
    write(
        mod_content,
        "common/defines/runtime.lua",
        "NRuntime = { VALUE = 99 }",
    );
    write(mod_content, "map/runtime.txt", "9 2 20 21\n");
    let content_path = mod_content.to_string_lossy().replace('\\', "/");
    write(
        userdata,
        "mod/portable.mod",
        &format!("name=\"Portable\"\npath=\"{content_path}\"\nreplace_path=\"interface\"\n"),
    );
    write(
        userdata,
        "dlc_load.json",
        r#"{"enabled_mods":["mod/portable.mod"],"disabled_dlcs":[]}"#,
    );
}

#[test]
fn interpreters_consume_mod_winners_and_record_complete_provenance() {
    let root = temp_root("overrides");
    let game = root.join("game");
    let userdata = root.join("profile");
    let mod_content = root.join("mod-content");
    write_content_fixture(&game, &userdata, &mod_content);
    let resolver = GameFileResolver::from_recorded_content_layers(
        &game,
        userdata,
        &[RecordedContentSource {
            ordinal: 4,
            source_id: "mod/portable.mod".into(),
        }],
    )
    .unwrap();
    let paths = [
        "interface/panel.gui",
        "interface/icons.gfx",
        "localisation/french/content_l_french.yml",
        "common/scripted_guis/panel.txt",
        "gfx/entities/runtime.asset",
        "gfx/FX/runtime.shader",
        "gfx/models/runtime.mesh",
        "common/defines/runtime.lua",
        "map/runtime.txt",
    ]
    .into_iter()
    .map(str::to_string);
    let (manifest, environment) = manifest_for(&resolver, paths, "l_french", Vec::new());
    let inventory = resolver.playback_inventory(&manifest, environment).unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let resolver = resolver.with_content_admission(&admission).unwrap();
    let interpreter = ProductionContentInterpreter::new(&resolver, &admission, "l_french").unwrap();

    let gui_key = interpreter.content_key("interface/panel.gui").unwrap();
    let gui = interpreter.parse_gui(&gui_key).unwrap();
    assert_eq!(gui.value[0].name, "mod_panel");
    assert_eq!(gui.provenance[0].winning_layer_ordinal, 4);

    let gfx_key = interpreter.content_key("interface/icons.gfx").unwrap();
    let gfx = interpreter.parse_gfx(&gfx_key).unwrap();
    assert!(gfx.value.sprites.get("GFX_mod").is_some());
    assert!(gfx.value.sprites.get("GFX_base").is_none());

    let loc_key = interpreter
        .content_key("localisation/french/content_l_french.yml")
        .unwrap();
    let loc = interpreter.parse_localisation(&loc_key).unwrap();
    assert_eq!(loc.value.get("PANEL_TITLE"), Some("Modifie"));

    let scripted = interpreter.load_scripted_gui_registry().unwrap();
    assert!(scripted.value.definitions.contains_key("mod_panel"));
    assert!(!scripted.value.definitions.contains_key("base_panel"));
    assert_eq!(scripted.provenance.len(), 1);

    let entity_key = interpreter
        .content_key("gfx/entities/runtime.asset")
        .unwrap();
    let entity = interpreter.parse_clausewitz(&entity_key).unwrap();
    assert_eq!(
        entity
            .value
            .root
            .first_value_ci("entity")
            .unwrap()
            .first_scalar_ci("name"),
        Some("mod_entity")
    );
    let shader_key = interpreter.content_key("gfx/FX/runtime.shader").unwrap();
    let shader = interpreter.parse_shader_effect(&shader_key).unwrap();
    assert_eq!(shader.value.effects[0].name, "mod_effect");
    let mesh_key = interpreter.content_key("gfx/models/runtime.mesh").unwrap();
    let mesh = interpreter.parse_pdx_asset(&mesh_key).unwrap();
    assert_eq!(mesh.value.version, vec![1, 0]);
    let define_key = interpreter
        .content_key("common/defines/runtime.lua")
        .unwrap();
    assert!(
        String::from_utf8(interpreter.read(&define_key).unwrap().value)
            .unwrap()
            .contains("VALUE = 99")
    );
    let map_key = interpreter.content_key("map/runtime.txt").unwrap();
    assert_eq!(
        String::from_utf8(interpreter.read(&map_key).unwrap().value).unwrap(),
        "9 2 20 21\n"
    );
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}

#[test]
fn replace_path_missing_content_and_changed_dlc_fail_closed() {
    let root = temp_root("negative");
    let game = root.join("game");
    let userdata = root.join("profile");
    let mod_content = root.join("mod-content");
    write_content_fixture(&game, &userdata, &mod_content);
    write(
        &game,
        "interface/base_only.gui",
        "guiTypes = { windowType = { name = forbidden_base } }",
    );
    let resolver = GameFileResolver::from_recorded_content_layers(
        &game,
        userdata,
        &[RecordedContentSource {
            ordinal: 2,
            source_id: "mod/portable.mod".into(),
        }],
    )
    .unwrap();
    assert!(resolver
        .resolve("interface/base_only.gui")
        .unwrap()
        .is_none());
    let required_dlc = vec![DlcSelection {
        dlc_id: "dlc099_fixture".into(),
        enabled: true,
        required_for_playback: true,
    }];
    let (manifest, environment) = manifest_for(
        &resolver,
        vec!["interface/panel.gui".into()],
        "l_french",
        required_dlc,
    );
    let mut wrong_environment = environment;
    wrong_environment.dlc_selection.clear();
    let wrong_inventory = resolver
        .playback_inventory(&manifest, wrong_environment)
        .unwrap();
    assert!(manifest.admit_playback(&wrong_inventory).is_err());
    assert!(resolver
        .resolve_required_with_provenance("interface/missing.gui", "required UI")
        .is_err());
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}

#[test]
fn decode_texture_uses_immutable_admitted_bytes_after_source_mutation() {
    let root = temp_root("texture");
    let game = root.join("game");
    let userdata = root.join("profile");
    let mod_content = root.join("mod-content");
    write_content_fixture(&game, &userdata, &mod_content);
    let mut tga = vec![0u8; 18];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&2u16.to_le_bytes());
    tga[14..16].copy_from_slice(&1u16.to_le_bytes());
    tga[16] = 32;
    tga[17] = 0x20 | 8;
    tga.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 128]);
    let tga_path = game.join("gfx").join("interface").join("icon.tga");
    fs::create_dir_all(tga_path.parent().unwrap()).unwrap();
    fs::write(&tga_path, &tga).unwrap();
    write(
        &game,
        "gfx/interface/photo.png",
        "png payloads stay refused",
    );
    let resolver = GameFileResolver::from_recorded_content_layers(
        &game,
        userdata,
        &[RecordedContentSource {
            ordinal: 2,
            source_id: "mod/portable.mod".into(),
        }],
    )
    .unwrap();
    let paths = ["gfx/interface/icon.tga", "gfx/interface/photo.png"]
        .into_iter()
        .map(str::to_string);
    let (manifest, environment) = manifest_for(&resolver, paths, "l_french", Vec::new());
    let inventory = resolver.playback_inventory(&manifest, environment).unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let resolver = resolver.with_content_admission(&admission).unwrap();
    let interpreter = ProductionContentInterpreter::new(&resolver, &admission, "l_french").unwrap();

    let tga_key = interpreter.content_key("gfx/interface/icon.tga").unwrap();
    let decoded = interpreter.decode_texture(&tga_key).unwrap();
    assert_eq!(decoded.provenance, vec![tga_key.clone()]);
    assert_eq!(
        decoded.value.image.pixels,
        vec![255, 0, 0, 255, 0, 255, 0, 128]
    );

    let png_key = interpreter.content_key("gfx/interface/photo.png").unwrap();
    assert!(interpreter.decode_texture(&png_key).is_err());

    fs::write(&tga_path, b"mutated after admission").unwrap();
    let decoded_after_mutation = interpreter.decode_texture(&tga_key).unwrap();
    assert_eq!(decoded_after_mutation.provenance, vec![tga_key]);
    assert_eq!(
        decoded_after_mutation.value.image.pixels,
        decoded.value.image.pixels
    );
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_ui_parsers_and_image_codecs_use_admitted_mod_overrides() {
    use game_ui::assets::AssetResolver;
    use game_ui::definition_admission::{load_effective_definition_catalog, ResolvedGuiDefinitionFile};
    let root = temp_root("modvfs-consumers");
    let game = root.join("game");
    let mods = root.join("mod");
    for (dir, name, value) in [(&game, "base_panel", 1u8), (&mods, "mod_panel", 7u8)] {
        write(dir, "interface/panel.gui", &format!("guiTypes = {{ containerWindowType = {{ name = {name} }} }}"));
        write(dir, "interface/icons.gfx", &format!("spriteTypes = {{ spriteType = {{ name = GFX_{name} texturefile = \"gfx/interface/icon.dds\" }} }}"));
        write(dir, "interface/core.gfx", &format!("bitmapfonts = {{ textcolors = {{ R = {{ {value} 8 9 }} }} }}"));
        let texture_dir = dir.join("gfx/interface");
        fs::create_dir_all(&texture_dir).unwrap();
        let mut dds = vec![0u8; 132];
        dds[..4].copy_from_slice(b"DDS ");
        for (offset, number) in [(4, 124u32), (12, 1), (16, 1), (28, 1), (76, 32),
            (80, 0x41), (88, 32), (92, 16711680), (96, 65280), (100, 255),
            (104, 4278190080), (108, 0x1000)] {
            dds[offset..offset + 4].copy_from_slice(&number.to_le_bytes());
        }
        dds[128..].copy_from_slice(&[9, 8, value, 255]);
        fs::write(texture_dir.join("icon.dds"), dds).unwrap();
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 32;
        tga[17] = 0x28;
        tga.extend_from_slice(&[9, 8, value, 255]);
        fs::write(texture_dir.join("icon.tga"), tga).unwrap();
        game_ui::png::write_png(&texture_dir.join("icon.png"), &game_ui::dds::RgbaImage {
            width: 1, height: 1, pixels: vec![value, 8, 9, 255],
        }).unwrap();
    }
    let raw = GameFileResolver::with_mod_paths(&game, vec![mods.clone()]).unwrap();
    let paths = raw.resolve_dir_files_recursive("", None).unwrap().into_iter().map(|(path, _)| path);
    let (manifest, environment) = manifest_for(&raw, paths, "l_english", vec![]);
    let admission = manifest.admit_playback(&raw.playback_inventory(&manifest, environment).unwrap()).unwrap();
    let resolver = raw.with_content_admission(&admission).unwrap();
    let assets = AssetResolver::from_content_admission(&resolver, &admission).unwrap();
    let gui = assets.resolve_file("interface/panel.gui").unwrap();
    let row = ResolvedGuiDefinitionFile::read(&assets, &gui).unwrap();
    let catalog = load_effective_definition_catalog(vec![row]).unwrap();
    assert_eq!(catalog.effective_roots[0].root.name, "mod_panel");
    let mut sprites = game_ui::gfx::SpriteRegistry::new();
    let gfx = assets.resolve_file("interface/icons.gfx").unwrap();
    sprites.parse_content(&assets.read_file(&gfx).unwrap(), &gfx.rel_path).unwrap();
    assert!(sprites.get("GFX_mod_panel").is_some());
    assert!(sprites.get("GFX_base_panel").is_none());
    let (probed, stats) = game_ui::probe::parse_gfx_files(&assets).unwrap();
    assert_eq!(stats.file_count, 2);
    assert!(probed.get("GFX_mod_panel").is_some());
    assert_eq!(game_ui::markup::TextColorTable::load(&assets).unwrap().get('R'), Some([7, 8, 9]));
    for relative in ["gfx/interface/icon.dds", "gfx/interface/icon.tga"] {
        assert_eq!(assets.decode_texture(relative).unwrap().image.pixels, [7, 8, 9, 255]);
    }
    let png = assets.resolve_file("gfx/interface/icon.png").unwrap();
    assert_eq!(game_ui::png::decode_bytes(&assets.read_file(&png).unwrap()).unwrap().image.pixels, [7, 8, 9, 255]);
    let dds = assets.resolve_file("gfx/interface/icon.dds").unwrap();
    assert_eq!(game_ui::dds::info_bytes(&assets.read_file(&dds).unwrap()).unwrap().width, 1);
    fs::write(mods.join("interface/panel.gui"), "invalid after admission").unwrap();
    let document = game_ui::paradox_text::parse_checked_bytes(&assets.read_file(&gui).unwrap(), &gui.rel_path).unwrap();
    let mut layouts = game_ui::gui::LayoutSet::default();
    layouts.add_document(&gui.rel_path, &document);
    assert_eq!(layouts.find_by_name("mod_panel").len(), 1);
    let foreign = game_ui::assets::ResolvedFile { rel_path: gui.rel_path.clone(), path: game.join("interface/panel.gui") };
    assert!(assets.read_file(&foreign).is_err());
    drop(assets);
    drop(resolver);
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_ui_directory_scan_ignores_unrecorded_gui_added_before_open() {
    let root = temp_root("modvfs-incomplete-namespace");
    let game = root.join("game");
    let mods = root.join("mod");
    write(&game, "interface/panel.gui", "guiTypes = {} ");
    write(&mods, "interface/panel.gui", "guiTypes = { containerWindowType = { name = mod_panel } }");
    let raw = GameFileResolver::with_mod_paths(&game, vec![mods.clone()]).unwrap();
    let (manifest, environment) = manifest_for(&raw, ["interface/panel.gui".into()], "l_english", vec![]);
    let admission = manifest.admit_playback(&raw.playback_inventory(&manifest, environment.clone()).unwrap()).unwrap();
    let before = raw.with_content_admission(&admission).unwrap();
    let before_assets = game_ui::assets::AssetResolver::from_content_admission(&before, &admission).unwrap();
    let expected = before_assets.list_files_recursive("interface", Some("gui")).unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].rel_path, "interface/panel.gui");
    let expected_bytes = before_assets.read_file(&expected[0]).unwrap();
    assert!(String::from_utf8(expected_bytes.clone()).unwrap().contains("mod_panel"));
    write(&game, "interface/base-extra.gui", "unused base GUI");
    write(&mods, "interface/extra.gui", "guiTypes = { containerWindowType = { name = mod_extra } }");
    let raw = GameFileResolver::with_mod_paths(&game, vec![mods]).unwrap();
    let reopened = manifest.admit_playback(&raw.playback_inventory(&manifest, environment).unwrap()).unwrap();
    assert_eq!(reopened, admission);
    let resolver = raw.with_content_admission(&reopened).unwrap();
    let assets = game_ui::assets::AssetResolver::from_content_admission(&resolver, &reopened).unwrap();
    let actual = assets.list_files_recursive("interface", Some("gui")).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(assets.read_file(&actual[0]).unwrap(), expected_bytes);
    assert!(assets.resolve_file("interface/extra.gui").is_err());
    drop(before_assets);
    drop(before);
    drop(assets);
    drop(resolver);
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}

#[test]
fn portable_manifest_rebinds_after_game_profile_and_replay_relocation() {
    let root = temp_root("relocated");
    let mut resolvers = Vec::new();
    for machine in ["machine-a", "machine-b"] {
        let machine_root = root.join(machine);
        let game = machine_root.join("different-game-location");
        let userdata = machine_root.join("different-profile-location");
        let mod_content = machine_root.join("different-mod-location");
        write_content_fixture(&game, &userdata, &mod_content);
        resolvers.push(
            GameFileResolver::from_recorded_content_layers(
                game,
                userdata,
                &[RecordedContentSource {
                    ordinal: 7,
                    source_id: "mod/portable.mod".into(),
                }],
            )
            .unwrap(),
        );
    }
    let (manifest, environment) = manifest_for(
        &resolvers[0],
        vec!["interface/panel.gui".into()],
        "l_french",
        Vec::new(),
    );
    let inventory = resolvers[1]
        .playback_inventory(&manifest, environment)
        .unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let resolver = resolvers
        .pop()
        .unwrap()
        .with_content_admission(&admission)
        .unwrap();
    let interpreter = ProductionContentInterpreter::new(&resolver, &admission, "l_french").unwrap();
    let key = interpreter.content_key("interface/panel.gui").unwrap();
    assert_eq!(
        interpreter.parse_gui(&key).unwrap().value[0].name,
        "mod_panel"
    );
    portable_test_fs_retry::remove_dir_all(root).unwrap();
}
