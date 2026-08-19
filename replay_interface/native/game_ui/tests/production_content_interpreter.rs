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
