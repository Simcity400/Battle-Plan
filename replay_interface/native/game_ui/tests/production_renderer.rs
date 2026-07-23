use game_ui::assets::AssetResolver;
use game_ui::production_renderer::{
    admit_authenticated_product_hud_route, ProductionUiRejection, ProductionUiRenderer,
    OMITTED_AUTHORITY_BLOCKED_PACKETS, OMITTED_EXTENSION_PACKETS, OMITTED_PIXEL_CANVAS_ROUTES,
    PRODUCTION_UI_HEIGHT, PRODUCTION_UI_WIDTH, PROMOTED_EXTENSION_PACKETS, PROMOTED_XUI_PACKETS,
    REQUIRED_AUTHORED_HUD_SOURCES,
};
use game_ui::variant_admission::InputVariant;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use map_viewer::{GameFileResolver, PlaybackEnvironment};
use replay_core::{
    ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer, RequiredContent,
    RequiredContentKind, SourceGameIdentity, SourceProfileIdentity, UsedContentKey,
};

fn production_renderer(root: PathBuf) -> ProductionUiRenderer {
    let resolver = map_viewer::GameFileResolver::vanilla(root).unwrap();
    let mut paths = BTreeSet::new();
    for (dir, extension) in [
        ("interface", None),
        ("localisation/english", Some("yml")),
        ("gfx/interface", None),
        ("gfx/minimap", None),
        ("gfx/texticons", None),
        ("gfx/fonts", None),
    ] {
        for (logical, _) in resolver
            .resolve_dir_files_recursive(dir, extension)
            .unwrap()
        {
            paths.insert(logical);
        }
    }
    let resolved = resolver.resolve_used_content(paths).unwrap();
    let used_ordinals = resolved
        .iter()
        .map(|row| row.source.ordinal)
        .collect::<BTreeSet<_>>();
    let layer_kind = |kind: &str| match kind {
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => ContentLayerKind::Base,
    };
    let effective_vfs_manifest_sha256 = "e".repeat(64);
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
        .map(|row| {
            let logical_path = row.logical_path.to_ascii_lowercase();
            let payload_sha256 = row.sha256.clone();
            let mut consumers = vec![format!("hud_role/v1/topbar/dependency/{}", payload_sha256)];
            let gui = match logical_path.as_str() {
                "interface/topbar.gui" => Some(("topbar", "topbar")),
                "interface/minimap.gui" => Some(("minimap", "mini_map_interface")),
                "interface/mapmodes_interface.gui" => {
                    Some(("mapmodes", "MapmodesInterface_Ingame"))
                }
                "interface/theatreselector.gui" => {
                    Some(("theatre_selector", "theatre_selection_window"))
                }
                _ => None,
            };
            if let Some((surface, root)) = gui {
                consumers.push(format!("hud_role/v1/{surface}/gui/{root}/{payload_sha256}"));
            }
            consumers.sort();
            consumers.dedup();
            RequiredContent {
                content_key: logical_path.clone(),
                kind: RequiredContentKind::Other,
                observation_count: 1,
                consumers,
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.clone(),
                    normalized_game_relative_path: logical_path,
                    winning_layer_ordinal: row.source.ordinal as u32,
                    layer_kind: layer_kind(&row.source.kind),
                    layer_id: row.source.source_id,
                    source_sha256: row
                        .source
                        .archive_sha256
                        .unwrap_or_else(|| row.sha256.clone()),
                    payload_sha256,
                    payload_bytes: row.byte_len,
                },
            }
        })
        .collect::<Vec<_>>();
    let source_game = SourceGameIdentity {
        version: "1.19.2".into(),
        build: "a729".into(),
        executable_sha256: "8".repeat(64),
        build_profile: "release".into(),
    };
    let source_profile = SourceProfileIdentity {
        profile_id: "test-profile".into(),
        playset_id: "test-playset".into(),
        selection_sha256: "0".repeat(64),
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "9".repeat(64),
        source_game: source_game.clone(),
        source_profile,
        effective_vfs_manifest_sha256,
        ordered_layers,
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    let manifest = manifest.with_computed_digest().unwrap();
    let inventory = resolver
        .playback_inventory(
            &manifest,
            PlaybackEnvironment {
                source_game,
                source_profile: manifest.source_profile.clone(),
                dlc_selection: Vec::new(),
                localisation_language: "l_english".into(),
                settings: BTreeMap::new(),
            },
        )
        .unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let resolver = resolver.with_content_admission(&admission).unwrap();
    let route = admit_authenticated_product_hud_route(&admission)
        .unwrap()
        .expect("synthetic production manifest carries the complete HUD role closure");
    let prefixes = route.consumer_prefixes().collect::<Vec<_>>();
    let assets =
        AssetResolver::from_content_consumer_prefixes(&resolver, &admission, &prefixes).unwrap();
    ProductionUiRenderer::new(assets, &admission).unwrap()
}

fn minimal_hud_route_admission(
    mapmodes_root: Option<&str>,
) -> replay_core::ValidatedContentAdmission {
    let root = std::env::temp_dir().join(format!(
        "game_ui_product_route_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let gui_rows = [
        ("interface/topbar.gui", "topbar", "topbar"),
        ("interface/minimap.gui", "minimap", "mini_map_interface"),
        (
            "interface/mapmodes_interface.gui",
            "mapmodes",
            mapmodes_root.unwrap_or(""),
        ),
        (
            "interface/theatreselector.gui",
            "theatre_selector",
            "theatre_selection_window",
        ),
    ];
    for (path, _, root_name) in gui_rows {
        let path = root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!("guiTypes = {{ containerWindowType = {{ name = \"{root_name}\" }} }}"),
        )
        .unwrap();
    }
    let resolver = GameFileResolver::vanilla(&root).unwrap();
    let resolved = resolver
        .resolve_used_content(gui_rows.into_iter().map(|(path, _, _)| path.to_string()))
        .unwrap();
    let vfs = "e".repeat(64);
    let required_content = resolved
        .into_iter()
        .map(|row| {
            let (surface, expected_root) = gui_rows
                .iter()
                .find_map(|(path, surface, root_name)| {
                    (*path == row.logical_path).then_some((*surface, *root_name))
                })
                .unwrap();
            let consumer = if expected_root.is_empty() {
                "fixture/non_hud/mapmodes".to_string()
            } else {
                format!("hud_role/v1/{surface}/gui/{expected_root}/{}", row.sha256)
            };
            RequiredContent {
                content_key: row.logical_path.clone(),
                kind: RequiredContentKind::GuiDefinition,
                observation_count: 1,
                consumers: vec![consumer],
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: vfs.clone(),
                    normalized_game_relative_path: row.logical_path,
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: row.sha256.clone(),
                    payload_sha256: row.sha256,
                    payload_bytes: row.byte_len,
                },
            }
        })
        .collect::<Vec<_>>();
    let source_game = SourceGameIdentity {
        version: "1.19.2".into(),
        build: "a729".into(),
        executable_sha256: "8".repeat(64),
        build_profile: "release".into(),
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "9".repeat(64),
        source_game: source_game.clone(),
        source_profile: SourceProfileIdentity {
            profile_id: "route-profile".into(),
            playset_id: "route-playset".into(),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: vfs,
        ordered_layers: vec![RecordedContentLayer {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            display_name: "Base".into(),
            layer_id: "base".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
            required_for_playback: true,
        }],
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest = manifest.with_computed_digest().unwrap();
    let inventory = resolver
        .playback_inventory(
            &manifest,
            PlaybackEnvironment {
                source_game,
                source_profile: manifest.source_profile.clone(),
                dlc_selection: Vec::new(),
                localisation_language: "l_english".into(),
                settings: BTreeMap::new(),
            },
        )
        .unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    admission
}

#[test]
fn product_packet_partition_is_exact_and_authority_blocked_surfaces_have_no_render_route() {
    assert_eq!(
        PROMOTED_XUI_PACKETS,
        ["XUI09", "XUI10", "XUI15", "XUI24", "XUI25"]
    );
    assert_eq!(OMITTED_AUTHORITY_BLOCKED_PACKETS.len(), 20);
    assert_eq!(OMITTED_PIXEL_CANVAS_ROUTES.len(), 1);
    assert_eq!(PROMOTED_EXTENSION_PACKETS, ["XUI27"]);
    assert!(OMITTED_EXTENSION_PACKETS.is_empty());
    for (packet, dependency) in OMITTED_AUTHORITY_BLOCKED_PACKETS {
        assert!(!PROMOTED_XUI_PACKETS.contains(packet));
        assert!(!dependency.trim().is_empty());
    }
    for (packet, dependency) in OMITTED_PIXEL_CANVAS_ROUTES {
        assert!(!PROMOTED_XUI_PACKETS.contains(packet));
        assert!(!dependency.trim().is_empty());
    }
    let classified = PROMOTED_XUI_PACKETS
        .iter()
        .copied()
        .chain(
            OMITTED_AUTHORITY_BLOCKED_PACKETS
                .iter()
                .map(|(packet, _)| *packet),
        )
        .chain(
            OMITTED_PIXEL_CANVAS_ROUTES
                .iter()
                .map(|(packet, _)| *packet),
        )
        .collect::<std::collections::BTreeSet<_>>();
    let expected = (1..=26)
        .map(|ordinal| format!("XUI{ordinal:02}"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        classified,
        expected.iter().map(String::as_str).collect(),
        "every XUI01-XUI26 packet must have exactly one product disposition"
    );
}

#[test]
fn production_ui_callers_have_no_frozen_or_extractor_staging_fallback() {
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let sources = [
        native.join("game_ui/src/production_renderer.rs"),
        native.join("replay_app/src/app.rs"),
        native.join("replay_app/src/ui_runtime.rs"),
        native.join("replay_app/src/main.rs"),
    ];
    let forbidden = [
        "load_frozen_definition_catalog",
        "frozen_frontend_source_files",
        "DEFAULT_GAME_ROOT",
        "from_runtime_userdir",
        "static_mod_staging_manifest",
        "ui14_engine_vfs",
        "ui_asset_resolver",
        "AssetResolver::new",
    ];
    for source in sources {
        let text = std::fs::read_to_string(&source).unwrap();
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "production source {} still contains forbidden fallback {needle}",
                source.display()
            );
        }
    }
}

#[test]
fn production_app_routes_only_authenticated_selected_present_ui() {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../replay_app/src/app.rs");
    let text = std::fs::read_to_string(app).unwrap();
    let ui_runtime = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../replay_app/src/ui_runtime.rs"),
    )
    .unwrap();
    assert!(!text.contains("ProductUiRuntime"));
    assert!(text.contains("ProductSelectedPresentRuntime::new"));
    assert!(text.contains("No authenticated selected-Present pass means no game HUD"));
    assert!(!text.contains("AssetResolver::from_content_admission"));
    assert!(!text.contains("admit_effective_window_manager"));
    assert!(!ui_runtime.contains("struct ProductUiRuntime"));
    assert!(ui_runtime.contains("exact_variant_admission_key"));
    assert!(!ui_runtime.contains("AssetResolver::from_content_admission"));
    assert!(!ui_runtime.contains("admit_effective_window_manager"));
    assert!(!text.contains("production_renderer::PRODUCTION_UI_WIDTH"));
    assert!(!text.contains("production_renderer::PRODUCTION_UI_HEIGHT"));
    assert!(text.contains("replay lacks authenticated initial display_variant"));
    assert!(text.contains("debug_overlay_enabled = capture_dir.is_some()"));
    assert!(text.contains("debug_overlay_enabled.then(|| Overlay::create"));
    assert!(!text.contains("let mut overlay = Overlay::create"));
    assert!(text.contains("if debug_overlay_enabled"));
    assert_eq!(
        PROMOTED_XUI_PACKETS,
        ["XUI09", "XUI10", "XUI15", "XUI24", "XUI25"]
    );
    assert_eq!(PROMOTED_EXTENSION_PACKETS, ["XUI27"]);
    for packet in ["XUI01", "XUI02", "XUI04", "XUI07", "XUI26"] {
        assert!(OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(candidate, _)| *candidate == packet));
    }
}

#[test]
fn authenticated_product_hud_reads_only_the_four_controller_bound_roots() {
    assert_eq!(
        REQUIRED_AUTHORED_HUD_SOURCES,
        [
            "interface/topbar.gui",
            "interface/minimap.gui",
            "interface/mapmodes_interface.gui",
            "interface/theatreselector.gui",
        ]
    );
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let renderer =
        std::fs::read_to_string(manifest_dir.join("src/production_renderer.rs")).unwrap();
    assert!(renderer.contains("compose_authenticated_hud("));
    assert!(!renderer.contains("compose_hud("));

    let hud = std::fs::read_to_string(manifest_dir.join("src/hud.rs")).unwrap();
    let product = hud
        .split_once("pub fn compose_authenticated_hud(")
        .unwrap()
        .1
        .split_once("pub fn compose_hud(")
        .unwrap()
        .0;
    for unbound in [
        "MENUBAR_GUI",
        "CORE_GUI",
        "zoom_in",
        "zoom_out",
        "outliner_row",
        "tooltip_shell",
        "compose_tooltip_into",
        "draw_outliner_rows",
    ] {
        assert!(
            !product.contains(unbound),
            "authenticated product HUD still reads controller-unbound fragment {unbound}"
        );
    }
}

#[test]
fn product_hud_route_omits_missing_or_incompatible_runtime_root_without_fallback() {
    let valid = minimal_hud_route_admission(Some("MapmodesInterface_Ingame"));
    assert!(admit_authenticated_product_hud_route(&valid)
        .unwrap()
        .is_some());

    let missing = minimal_hud_route_admission(None);
    assert!(admit_authenticated_product_hud_route(&missing)
        .unwrap()
        .is_none());

    let incompatible = minimal_hud_route_admission(Some("mod_mapmodes_controller"));
    assert!(admit_authenticated_product_hud_route(&incompatible)
        .unwrap()
        .is_none());
}

#[test]
fn admitted_ui_resolver_rejects_an_installed_but_unrecorded_path() {
    let Some(root) = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT")).map(PathBuf::from) else {
        return;
    };
    if !root.is_dir() {
        return;
    }
    let renderer = production_renderer(root);
    assert!(renderer
        .resolver()
        .resolve_optional_file("common/defines/00_defines.lua")
        .unwrap()
        .is_none());
}

#[test]
fn successor_xui12_data_stays_pixel_omitted_without_the_d71_item_grid_trace() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("positive")
        .join("fx_country_management.battleplan");
    let timeline =
        replay_core::WorldTimeline::open(replay_core::open_recording(&fixture).unwrap()).unwrap();
    let frame = timeline.state_at(16_959_368).unwrap();
    let production = frame
        .production("GER")
        .expect("successor fixture carries GER ProductionView");
    assert_eq!(production.stockpile_complete.measured(), Some(&true));
    assert_eq!(production.military_lines_complete.measured(), Some(&true));
    assert_eq!(production.naval_lines_complete.measured(), Some(&true));
    let military = production
        .military_lines
        .measured()
        .and_then(|rows| rows.first())
        .expect("successor fixture carries a military line");
    assert_eq!(
        military.line_subtype.measured().map(String::as_str),
        Some("military")
    );
    assert!(military.efficiency.measured().is_some());
    assert!(military.efficiency_cap.measured().is_some());
    assert!(military.efficiency_growth.measured().is_some());
    let naval = production
        .naval_lines
        .measured()
        .and_then(|rows| rows.first())
        .expect("successor fixture carries a naval line");
    assert_eq!(
        naval.line_subtype.measured().map(String::as_str),
        Some("ship_refit")
    );

    let rejection = game_ui::dynamic_collection::from_replay_frame(&frame)
        .expect_err("fixture intentionally has no authenticated d71 item-grid trace");
    assert!(
        rejection
            .to_string()
            .contains("dynamic collection trace is absent or unavailable"),
        "{rejection}"
    );
    let blocker = OMITTED_AUTHORITY_BLOCKED_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI12")
        .expect("XUI12 remains explicitly classified");
    assert!(blocker.1.contains("d71"));
    assert!(blocker.1.contains("materialized slot rectangles"));
    assert!(blocker.1.contains("clip"));
    assert!(blocker.1.contains("scroll"));
}

#[test]
fn unsupported_physical_viewport_rejects_before_any_pixel_resampling() {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .map(PathBuf::from);
    let Some(root) = root else {
        return;
    };
    if !root.is_dir() {
        return;
    }
    let renderer = production_renderer(root);
    assert_eq!(
        renderer.compose_hud_frame(None, None, 0, 1280, 720),
        Err(ProductionUiRejection::UnsupportedDisplayVariant {
            width: 1280,
            height: 720,
        })
    );
}

#[test]
fn installed_a729_ui_rejects_without_the_portable_xui25_carrier() {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .map(PathBuf::from);
    let Some(root) = root else {
        return;
    };
    if !root.is_dir() {
        return;
    }
    let renderer = production_renderer(root);
    assert_eq!(
        renderer.compose_hud_frame(None, None, 0, PRODUCTION_UI_WIDTH, PRODUCTION_UI_HEIGHT,),
        Err(ProductionUiRejection::MissingDisplayVariantCarrier(
            "WorldFrame"
        ))
    );
}

#[test]
fn recorded_topbar_frame_reaches_the_production_compositor_without_raw_snapshot_access() {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .map(PathBuf::from);
    let Some(root) = root else {
        return;
    };
    if !root.is_dir() {
        return;
    }
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("positive")
        .join("fx_topbar_scalars_sweep.battleplan");
    let source = replay_core::open_recording(&fixture).unwrap();
    let timeline = replay_core::WorldTimeline::open(source).unwrap();
    let frame = timeline.state_at(timeline.first_hour()).unwrap();
    let pov = frame
        .topbar_scalars_by_country()
        .keys()
        .next()
        .expect("fixture has a measured topbar country")
        .clone();
    let renderer = production_renderer(root);
    let first = renderer
        .compose_hud_frame_for_input(
            Some(&frame),
            Some(&pov),
            2,
            Some(InputVariant::MouseKeyboard),
            PRODUCTION_UI_WIDTH,
            PRODUCTION_UI_HEIGHT,
        )
        .unwrap();
    let second = renderer
        .compose_hud_frame_for_input(
            Some(&frame),
            Some(&pov),
            2,
            Some(InputVariant::MouseKeyboard),
            PRODUCTION_UI_WIDTH,
            PRODUCTION_UI_HEIGHT,
        )
        .unwrap();
    assert_eq!(first.image, second.image);
    assert_eq!(first.report, second.report);
    assert_eq!(first.report.pov_tag.as_deref(), Some(pov.as_str()));
}

#[test]
fn relocated_portable_replay_file_still_drives_the_same_production_pixels() {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .map(PathBuf::from);
    let Some(root) = root else {
        return;
    };
    if !root.is_dir() {
        return;
    }
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("positive")
        .join("fx_topbar_scalars_sweep.battleplan");
    let relocated = std::env::temp_dir().join(format!(
        "battle-plan-production-ui-relocated-{}.battleplan",
        std::process::id()
    ));
    std::fs::copy(&source_path, &relocated).unwrap();
    let renderer = production_renderer(root);
    let render = |path: &std::path::Path| {
        let source = replay_core::open_recording(path).unwrap();
        let timeline = replay_core::WorldTimeline::open(source).unwrap();
        let frame = timeline.state_at(timeline.first_hour()).unwrap();
        let pov = frame
            .topbar_scalars_by_country()
            .keys()
            .next()
            .unwrap()
            .clone();
        renderer
            .compose_hud_frame_for_input(
                Some(&frame),
                Some(&pov),
                2,
                Some(InputVariant::MouseKeyboard),
                PRODUCTION_UI_WIDTH,
                PRODUCTION_UI_HEIGHT,
            )
            .unwrap()
    };
    let canonical = render(&source_path);
    let moved = render(&relocated);
    std::fs::remove_file(&relocated).unwrap();
    assert_eq!(canonical.image, moved.image);
    assert_eq!(canonical.report, moved.report);
}
