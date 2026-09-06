use game_ui::assets::AssetResolver;
use game_ui::production_renderer::{
    admit_authenticated_product_hud_route, ProductionUiRenderer,
    OMITTED_AUTHORITY_BLOCKED_PACKETS, OMITTED_EXTENSION_PACKETS, OMITTED_PIXEL_CANVAS_ROUTES,
    PRODUCTION_UI_HEIGHT, PRODUCTION_UI_WIDTH, PROMOTED_EXTENSION_PACKETS,
    PROMOTED_EXTERNAL_DEPENDENCY_PACKETS, PROMOTED_XUI_PACKETS, REQUIRED_AUTHORED_HUD_SOURCES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use map_viewer::{GameFileResolver, PlaybackEnvironment};
use replay_core::{
    CommandAbilityEnvelope, CommandAbilityTimerKind, ContentLayerKind, ContentManifest,
    LayerStorageKind, RawFrame, RecordedContentLayer, ReplaySource, RequiredContent,
    RequiredContentKind, SourceError, SourceGameIdentity, SourceProfileIdentity, UsedContentKey,
    WorldTimeline,
};

fn installed_game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

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
        ("gfx/FX", None),
        ("common/abilities", None),
    ] {
        for (logical, _) in resolver
            .resolve_dir_files_recursive(dir, extension)
            .unwrap()
        {
            paths.insert(logical);
        }
    }
    production_renderer_from_paths(resolver, paths)
}

fn xui21_production_renderer(root: PathBuf) -> ProductionUiRenderer {
    let resolver = map_viewer::GameFileResolver::vanilla(root).unwrap();
    let paths = BTreeSet::from([
        "interface/topbar.gui".into(),
        "interface/minimap.gui".into(),
        "interface/mapmodes_interface.gui".into(),
        "interface/theatreselector.gui".into(),
        "interface/eventwindow.gui".into(),
        "interface/core.gfx".into(),
        "interface/eventwindow.gfx".into(),
        "interface/eventpictures.gfx".into(),
        "interface/countryintelligenceagencyview.gfx".into(),
        "interface/countrypoliticsview.gfx".into(),
        "gfx/interface/tiles/tiled_window_transparent.dds".into(),
        "gfx/interface/event_news_pic_overlay.dds".into(),
        "gfx/interface/event_news_bg.dds".into(),
        "gfx/interface/event_pic_clip.dds".into(),
        "gfx/interface/event_option_entry.dds".into(),
        "gfx/interface/event_report_tileable_midsection.dds".into(),
        "gfx/interface/event_report_top_win.dds".into(),
        "gfx/interface/event_report_bottom_win.dds".into(),
        "gfx/interface/event_report_bottom_win_2.dds".into(),
        "gfx/interface/event_button_minimize.dds".into(),
        "gfx/interface/events/clip.dds".into(),
        "gfx/interface/events/event_operative_bg.dds".into(),
        "gfx/event_pictures/report_event_001.dds".into(),
        "gfx/event_pictures/news_event_001.dds".into(),
        "gfx/interface/operatives/agencies/agency_logo_prc.dds".into(),
        "gfx/leaders/leader_unknown.dds".into(),
        "gfx/interface/event_leader_frame.dds".into(),
        "gfx/fonts/hoi_20bs.fnt".into(),
        "gfx/fonts/hoi_20bs.dds".into(),
        "gfx/fonts/hoi4_typewriter16.fnt".into(),
        "gfx/fonts/hoi4_typewriter16.dds".into(),
        "gfx/fonts/hoi4_typewriter22.fnt".into(),
        "gfx/fonts/hoi4_typewriter22.dds".into(),
        "events/LowCountries.txt".into(),
    ]);
    production_renderer_from_paths(resolver, paths)
}

fn xui23_production_renderer(root: PathBuf) -> ProductionUiRenderer {
    let resolver = map_viewer::GameFileResolver::vanilla(root).unwrap();
    let mut paths = BTreeSet::from([
        "interface/topbar.gui".to_string(),
        "interface/minimap.gui".to_string(),
        "interface/mapmodes_interface.gui".to_string(),
        "interface/theatreselector.gui".to_string(),
        "interface/abilitylist.gui".to_string(),
        "gfx/interface/focusview/titlebar/focus_completed_texture.dds".to_string(),
        "gfx/FX/buttonstate.shader".to_string(),
        "gfx/FX/buttonstate.fxh".to_string(),
        "gfx/FX/sprite_animation.fxh".to_string(),
        "gfx/FX/progress.shader".to_string(),
    ]);
    for (dir, extension) in [
        ("interface", Some("gfx")),
        ("localisation/english", Some("yml")),
        ("common/abilities", None),
        ("gfx/interface/abilitylist", None),
    ] {
        for (logical, _) in resolver
            .resolve_dir_files_recursive(dir, extension)
            .unwrap()
        {
            paths.insert(logical);
        }
    }
    production_renderer_from_paths(resolver, paths)
}

fn production_renderer_from_paths(
    resolver: GameFileResolver,
    paths: BTreeSet<String>,
) -> ProductionUiRenderer {
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
            if logical_path.starts_with("events/") && logical_path.ends_with(".txt") {
                consumers
                    .push(game_ui::event_chrome::EVENT_DEFINITION_NAMESPACE_CONSUMER.to_string());
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
                    stored_game_relative_path: row.source_relative_path.clone(),
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
        executable_sha256: game_ui::active_abilities::XUI23_EXECUTABLE_SHA256.into(),
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
        recorder_local_settings: BTreeMap::new(),
        required_content,
        witnessed_content_namespaces: Vec::new(),
        witnessed_absent_content: Vec::new(),
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
    let vfs = "e".repeat(64);
    let sealed = resolver.observe_test_content_namespace(&vfs).unwrap();
    let required_content = sealed.winners().iter().cloned()
        .map(|content| {
            let (surface, expected_root) = gui_rows
                .iter()
                .find_map(|(path, surface, root_name)| {
                    (*path == content.normalized_game_relative_path).then_some((*surface, *root_name))
                })
                .unwrap();
            let consumer = if expected_root.is_empty() {
                "fixture/non_hud/mapmodes".to_string()
            } else {
                format!("hud_role/v1/{surface}/gui/{expected_root}/{}", content.payload_sha256)
            };
            RequiredContent {
                content_key: content.normalized_game_relative_path.clone(),
                kind: RequiredContentKind::GuiDefinition,
                observation_count: 1,
                consumers: vec![consumer],
                content,
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
        recorder_local_settings: BTreeMap::new(),
        required_content,
        witnessed_content_namespaces: Vec::new(),
        witnessed_absent_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.bind_observed_namespace(sealed).unwrap();
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
    portable_test_fs_retry::remove_dir_all(root).unwrap();
    admission
}

#[test]
fn xui23_consumer_is_promoted_with_exact_external_dependency() {
    let dependency = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI23")
        .expect("XUI23 external producer dependency");
    for required in [
        "EIR-NE-009",
        "exact-a729",
        "passive",
        "complete-D18",
        "all-country",
    ] {
        assert!(
            dependency.1.contains(required),
            "XUI23 dependency omits {required}"
        );
    }
    assert!(!OMITTED_PIXEL_CANVAS_ROUTES
        .iter()
        .any(|(packet, _)| *packet == "XUI23"));
    assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
        .iter()
        .any(|(packet, _)| *packet == "XUI23"));
    assert!(PROMOTED_XUI_PACKETS.contains(&"XUI23"));

    let renderer = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/production_renderer.rs"),
    )
    .unwrap();
    assert!(renderer.contains("let abilities = self.prepare_xui23_ability_route("));
    assert!(renderer.contains("reason: error.0"));
    let carrier = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/active_abilities.rs"),
    )
    .unwrap();
    assert!(carrier.contains("pub const XUI23_PASSIVE_CARRIER_UNAVAILABLE_REASON"));
    assert!(
        carrier.contains("ActiveAbilityRejection(XUI23_PASSIVE_CARRIER_UNAVAILABLE_REASON.into())")
    );
    let runtime = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../replay_app/src/ui_runtime.rs"),
    )
    .unwrap();
    assert!(runtime.contains(".compose_xui23_ability_surface("));
    assert!(runtime.contains("self.xui23_pending = Some(Xui23PresentedFrame"));
}

#[test]
fn legacy_a36_envelope_contract_still_refuses_engine_impossible_rows() {
    const CANONICAL: &str = include_str!(
        "../../../../re_evidence/rendering_laws/XUI23/canonical_active_abilities_envelope.json"
    );
    let envelope: CommandAbilityEnvelope =
        serde_json::from_str(CANONICAL).expect("canonical d88 envelope");
    assert_eq!(envelope.carrier.magic, "ABA2");
    assert_eq!(envelope.snapshot.magic, "ABL1");
    assert_eq!(
        envelope.carrier.executable_sha256,
        "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
    );
    assert!(envelope.validates_exact_authority());

    let mut empty = envelope.clone();
    empty.snapshot.row_count = 0;
    empty.snapshot.rows.clear();
    assert!(empty.snapshot.complete);
    assert!(empty.validates_exact_authority());

    let mut impossible = envelope;
    assert!(impossible.snapshot.rows[0].active);
    assert_eq!(
        impossible.snapshot.rows[0].duration_remaining.kind,
        CommandAbilityTimerKind::Hours
    );
    impossible.snapshot.rows[0].duration_remaining.hours = Some(0);
    assert!(!impossible.validates_exact_authority());
}

#[test]
fn product_packet_partition_is_exact_and_authority_blocked_surfaces_have_no_render_route() {
    assert_eq!(
        PROMOTED_XUI_PACKETS,
        [
            "XUI01", "XUI02", "XUI03", "XUI04", "XUI05", "XUI06", "XUI07", "XUI08", "XUI09",
            "XUI10", "XUI11", "XUI12", "XUI13", "XUI14", "XUI15", "XUI16", "XUI17", "XUI18",
            "XUI19", "XUI20", "XUI21", "XUI22", "XUI23", "XUI24", "XUI25", "XUI26"
        ]
    );
    assert!(OMITTED_AUTHORITY_BLOCKED_PACKETS.is_empty());
    assert!(OMITTED_PIXEL_CANVAS_ROUTES.is_empty());
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
    for packet in [
        "XUI02", "XUI04", "XUI07", "XUI08", "XUI09", "XUI10", "XUI11", "XUI12", "XUI13", "XUI14",
        "XUI15", "XUI16", "XUI17", "XUI18", "XUI19", "XUI20", "XUI21", "XUI22", "XUI23", "XUI24",
        "XUI26",
    ] {
        assert!(PROMOTED_XUI_PACKETS.contains(&packet));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(candidate, _)| *candidate == packet));
        assert!(!OMITTED_PIXEL_CANVAS_ROUTES
            .iter()
            .any(|(candidate, _)| *candidate == packet));
    }
    let xui07 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI07")
        .expect("XUI07/XUI27 theatre consumer retains its canonical dependency");
    for required in ["EIR-NE-012", "THS2 v6", "D18/D98", "XUI07/XUI27"] {
        assert!(
            xui07.1.contains(required),
            "XUI07 dependency omits {required}"
        );
    }
    let xui14 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI14")
        .expect("XUI14 consumer retains one canonical external dependency");
    for required in [
        "EIR-NE-004",
        "passive",
        "UI-open-independent",
        "all-country",
    ] {
        assert!(xui14.1.contains(required), "XUI14 reason omits {required}");
    }
    let xui08 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI08")
        .expect("XUI08 consumer retains one canonical external dependency");
    for required in [
        "PCS3",
        "exact-a729",
        "passive",
        "UI-open-independent",
        "global",
    ] {
        assert!(xui08.1.contains(required), "XUI08 reason omits {required}");
    }
    let xui10 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI10")
        .expect("XUI10 consumer retains one canonical external dependency");
    for required in ["EIR-NE-002", "all-country", "national-focus"] {
        assert!(xui10.1.contains(required), "XUI10 reason omits {required}");
    }
    let xui09 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI09")
        .expect("XUI09 consumer retains its canonical external producer dependency");
    for required in [
        "exact-a729",
        "passive",
        "UI-open-independent",
        "POV-independent",
        "D18",
        "G4",
        "X9S1",
    ] {
        assert!(xui09.1.contains(required), "XUI09 reason omits {required}");
    }
    let xui11 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI11")
        .expect("XUI11 consumer retains one canonical external dependency");
    for required in ["EIR-NE-003", "all-country", "politics"] {
        assert!(xui11.1.contains(required), "XUI11 reason omits {required}");
    }
    let xui17 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI17")
        .expect("XUI17 consumer retains one closed external producer ledger");
    for required in [
        "EIR-NE-005",
        "exact-a729",
        "passive",
        "UI-open-independent",
        "recorder-POV-independent",
        "complete-D18",
        "all-country",
        "D47",
        "D28",
        "D46",
        "G8",
        "G11",
    ] {
        assert!(xui17.1.contains(required), "XUI17 reason omits {required}");
    }
    let xui18 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI18")
        .expect("XUI18 consumer retains one canonical external dependency");
    for required in [
        "exact-a729",
        "passive",
        "UI-open-independent",
        "POV-independent",
        "all-country",
        "d74",
    ] {
        assert!(xui18.1.contains(required), "XUI18 reason omits {required}");
    }
    let xui16 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI16")
        .expect("XUI16 consumer retains its exact occupation/garrison dependency");
    for required in [
        "EIR-NE-010",
        "D52/D66",
        "equipment identity/type/amount",
        "garrison-alert",
    ] {
        assert!(xui16.1.contains(required), "XUI16 reason omits {required}");
    }
    let xui20 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI20")
        .expect("XUI20 consumer retains one canonical external dependency");
    for required in [
        "EIR-CW-002",
        "exact-a729",
        "passive",
        "UI-open-independent",
        "global",
        "five-panel combat",
    ] {
        assert!(xui20.1.contains(required), "XUI20 reason omits {required}");
    }
    let xui21 = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI21")
        .expect("XUI21 consumer retains its canonical external producer dependency");
    for required in [
        "exact-a729",
        "passive",
        "UI-open-independent",
        "POV-independent",
        "all-country",
        "extended-d75",
        "leader/operative runtime bindings",
        "A25",
        "events/**/*.txt",
        "namespace",
    ] {
        assert!(xui21.1.contains(required), "XUI21 reason omits {required}");
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
fn retired_ui_carriers_have_no_replay_app_product_callers() {
    let app_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../replay_app/src");
    let ui_runtime = std::fs::read_to_string(app_root.join("ui_runtime.rs")).unwrap();
    let character_runtime = std::fs::read_to_string(app_root.join("character_runtime.rs")).unwrap();
    assert!(!app_root.join("selected_pov_runtime.rs").exists());
    assert!(!ui_runtime.contains(".admit_xui09_selected_present_route("));
    assert!(ui_runtime.contains("observer_research_from_complete_frame("));
    assert!(character_runtime.contains("admit_country_observer_character_ui("));
}

#[test]
fn xui19_is_promoted_with_only_the_exact_passive_producer_dependency() {
    let dependency = PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .find(|(packet, _)| *packet == "XUI19")
        .map(|(_, dependency)| *dependency)
        .expect("XUI19 has an explicit external producer dependency");
    assert!(PROMOTED_XUI_PACKETS.contains(&"XUI19"));
    assert!(!OMITTED_PIXEL_CANVAS_ROUTES
        .iter()
        .any(|(packet, _)| *packet == "XUI19"));
    assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
        .iter()
        .any(|(packet, _)| *packet == "XUI19"));
    assert!(dependency.contains("exact-a729"));
    assert!(dependency.contains("passive"));
    assert!(dependency.contains("UI-open-independent"));
    assert!(dependency.contains("recorder-POV-independent"));
    assert!(dependency.contains("complete-D18 all-country decision evaluation"));
}

#[test]
fn production_ui_callers_have_no_frozen_or_extractor_staging_fallback() {
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let sources = [
        (native.join("game_ui/src/production_renderer.rs"), "tests"),
        (
            native.join("replay_app/src/app.rs"),
            "product_production_d3d_tests",
        ),
        (native.join("replay_app/src/ui_runtime.rs"), "tests"),
        (native.join("replay_app/src/main.rs"), "tests"),
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
    for (source, test_module) in sources {
        let text = std::fs::read_to_string(&source).unwrap();
        let module_marker = format!("mod {test_module} {{");
        let module_start = text
            .find(&module_marker)
            .unwrap_or_else(|| panic!("missing test module marker in {}", source.display()));
        let test_boundary = text[..module_start]
            .rfind("#[cfg(test)]")
            .unwrap_or_else(|| panic!("missing test boundary in {}", source.display()));
        let text = &text[..test_boundary];
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
fn production_app_routes_observer_state_and_quarantines_selected_present() {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../replay_app/src/app.rs");
    let text = std::fs::read_to_string(app).unwrap();
    let text = &text[..text.find("#[cfg(test)]").expect("app test module marker")];
    let ui_runtime = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../replay_app/src/ui_runtime.rs"),
    )
    .unwrap();
    assert!(text.contains("ProductObserverUiRuntime::from_playback_content"));
    assert!(text.contains("product_observer_ui.execute("));
    assert!(text.contains("outcome.promoted_extension_packets"));
    assert!(!text.contains("recorded_ui_owner("));
    assert!(ui_runtime.contains("struct ProductObserverUiRuntime"));
    assert!(ui_runtime.contains(
        "let authored_observer_ui = crate::gui_definition_admission::AuthoredObserverUi::new("
    ));
    assert!(ui_runtime
        .contains("\"XUI01/XUI02/XUI04/XUI07/XUI24/XUI27:manifest-bound-effective-windows={}\""));
    assert!(!ui_runtime.contains("authored observer role index is unavailable"));
    assert!(ui_runtime.contains("XUI26:manifest-bound-effective-families={}"));
    assert!(ui_runtime.contains(
        "XUI26:family-scope:static_shipping={}:replay_payload_shipping={}:host_runtime_nonshipping={}:external_service_nonshipping={}"
    ));
    assert!(ui_runtime.contains(".frontend_family_dispositions(content_admission)?"));
    assert!(ui_runtime.contains("XUI26:family={}:window={}:source={}:slot={}:unavailable={}"));
    assert!(ui_runtime.contains("XUI26:family={}:window={}:source={}:instance={}"));
    assert!(ui_runtime.contains("panel.runtime_instance_token"));
    assert!(ui_runtime.contains("panel.frame_generation"));
    assert!(text.contains("product_observer_ui.cycle_frontend_panel("));
    assert!(!ui_runtime.contains("install_xui26_host_runtime_snapshot("));
    assert!(!ui_runtime.contains("clear_xui26_host_runtime_snapshot("));
    assert!(ui_runtime.contains("authored.reconcile_frontend_frame(world_frame)?"));
    assert!(ui_runtime.contains("XUI26 country reset failed closed"));
    assert!(text.contains("VK_U =>"));
    let xui01_gate = ui_runtime
        .find(".validate_composed_roots(&authored_source_windows)?")
        .expect("shipping XUI01 source/root gate");
    let hud_upload = ui_runtime[xui01_gate..]
        .find(".set_rgba(gfx, &framebuffer)")
        .map(|offset| xui01_gate + offset)
        .expect("shipping HUD upload");
    assert!(xui01_gate < hud_upload);
    let observer_start = ui_runtime
        .find("impl ProductObserverUiRuntime {")
        .expect("observer runtime");
    let observer_end = ui_runtime[observer_start..]
        .find("pub(crate) struct ProductSelectedPresentPerformanceRuntime")
        .map(|offset| observer_start + offset)
        .expect("post-observer runtime boundary");
    let observer = &ui_runtime[observer_start..observer_end];
    for forbidden in [
        "AuthenticatedUiPass",
        "resolved_tooltip_exact",
        "recorded_ui_owner",
        "market_trade_activation",
        "hud_surface_activation",
    ] {
        assert!(!observer.contains(forbidden));
    }
    assert!(!text.contains("production_renderer::PRODUCTION_UI_WIDTH"));
    assert!(!text.contains("production_renderer::PRODUCTION_UI_HEIGHT"));
    assert!(text.contains("debug_overlay_enabled = capture_dir.is_some()"));
    assert!(text.contains("debug_overlay_enabled.then(|| Overlay::create"));
    assert!(!text.contains("let mut overlay = Overlay::create"));
    assert!(text.contains("if debug_overlay_enabled"));
    assert_eq!(
        PROMOTED_XUI_PACKETS,
        [
            "XUI01", "XUI02", "XUI03", "XUI04", "XUI05", "XUI06", "XUI07", "XUI08", "XUI09",
            "XUI10", "XUI11", "XUI12", "XUI13", "XUI14", "XUI15", "XUI16", "XUI17", "XUI18",
            "XUI19", "XUI20", "XUI21", "XUI22", "XUI23", "XUI24", "XUI25", "XUI26"
        ]
    );
    assert_eq!(PROMOTED_EXTENSION_PACKETS, ["XUI27"]);
    for packet in [
        "XUI02", "XUI03", "XUI04", "XUI05", "XUI07", "XUI08", "XUI09", "XUI10", "XUI11", "XUI13",
        "XUI14", "XUI15", "XUI16", "XUI17", "XUI18", "XUI19", "XUI21", "XUI22", "XUI23", "XUI24",
        "XUI26",
    ] {
        assert!(PROMOTED_XUI_PACKETS.contains(&packet));
        assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .any(|(candidate, _)| *candidate == packet));
    }
    assert!(!OMITTED_PIXEL_CANVAS_ROUTES
        .iter()
        .any(|(packet, _)| *packet == "XUI12"));
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
    assert!(!REQUIRED_AUTHORED_HUD_SOURCES.contains(&"interface/core.gui"));
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
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install is required for exact effective XUI23 authored pixels"
)]
fn frame_owned_xui23_carrier_reaches_the_actual_production_renderer() {
    use game_ui::active_abilities::{
        ObserverAbilityAuthority, ObserverAbilityContext, ObserverAbilityFramePayload,
        ObserverAbilityLifecycle, ObserverAbilityRuntimeRow, ObserverAbilityRuntimeSnapshot,
        ObserverAbilityScope, ObserverAbilityScopeKind, XUI23_EXECUTABLE_SHA256,
        XUI23_PRODUCER_AUTHORITY_FIELD,
    };

    #[derive(Clone)]
    struct OneFrame {
        rest: serde_json::Map<String, serde_json::Value>,
    }
    impl ReplaySource for OneFrame {
        fn game_id(&self) -> &str {
            "xui23-production-renderer"
        }
        fn hours(&self) -> &[i64] {
            &[1_000]
        }
        fn warnings(&self) -> &[String] {
            &[]
        }
        fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: {
                    let mut rest = self.rest.clone();
                    if replay_core::COUNTRY_COMPOSED_MEMBERS
                        .iter()
                        .any(|member| rest.contains_key(*member))
                    {
                        rest.entry("tag_db_id_index").or_insert_with(
                            || serde_json::json!({"country_composition_version": 2}),
                        );
                    }
                    rest.into()
                },
            })
        }
    }
    let decode = |rest| {
        WorldTimeline::open(Box::new(OneFrame { rest }))
            .unwrap()
            .state_at(1_000)
            .unwrap()
    };

    let renderer = xui23_production_renderer(installed_game_root());
    for sprite_name in ["GFX_ability_active_bg", "GFX_ability_cooldown_bg"] {
        let sprite = renderer
            .registry()
            .get(sprite_name)
            .unwrap_or_else(|| panic!("installed a729 lacks {sprite_name}"));
        assert!(sprite.kind.eq_ignore_ascii_case("progressbarType"));
        assert_eq!(sprite.raw.first_scalar_ci("horizontal"), Some("no"));
    }
    let mut rest = serde_json::Map::new();
    rest.insert(
        "countries".into(),
        serde_json::json!({"GER": {"_r2_tag_db_id": 73}}),
    );
    let unsigned = decode(rest.clone());
    let context = ObserverAbilityContext {
        caster: Some(ObserverAbilityScope {
            kind: ObserverAbilityScopeKind::ArmyLeader,
            stable_id: 73,
        }),
        group_target: None,
    };
    let payload = ObserverAbilityFramePayload {
        executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
        producer_manifest_sha256: renderer.content_manifest_digest_sha256().into(),
        replay_frame_identity_sha256:
            game_ui::active_abilities::observer_ability_replay_frame_identity_sha256(
                unsigned.as_ref(),
            )
            .unwrap(),
        completeness_record_sha256: "ef".repeat(32),
        source_record_id: "xui23:test:production-renderer".into(),
        authority: ObserverAbilityAuthority {
            executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
            frame_hour: 1_000,
            country_roster_complete: true,
            country_count: 1,
        },
        snapshots: vec![ObserverAbilityRuntimeSnapshot {
            country_tag: "GER".into(),
            complete: true,
            context_count: 2,
            contexts: vec![
                ObserverAbilityContext {
                    caster: None,
                    group_target: None,
                },
                context.clone(),
            ],
            row_count: 1,
            rows: vec![ObserverAbilityRuntimeRow {
                ability_id: "force_attack".into(),
                caster: context.caster.clone(),
                group_target: None,
                definition_order: 2,
                target_kind_code: 1,
                available: false,
                lifecycle: ObserverAbilityLifecycle::Active,
                activation_hour: Some(900),
                authored_duration_hours: 168,
                authored_cooldown_hours: None,
                duration_remaining_hours: Some(68),
                cooldown_remaining_hours: None,
                effective_command_power_cost_fixed: -2_200_000,
                effective_command_power_cost_scale: 100_000,
                disabled_reason: None,
            }],
        }],
    };
    rest.insert(
        XUI23_PRODUCER_AUTHORITY_FIELD.into(),
        payload.replay_authority_record(),
    );
    let frame = decode(rest);
    let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
    viewer.selected_country =
        game_ui::stored_country_selection::StoredCountrySelection::of("GER");
    viewer.abilities.open = true;
    viewer.abilities.selected_context = Some(context);
    let route = renderer.prepare_xui23_ability_route(frame.as_ref(), &viewer);
    assert!(route.gaps.is_empty(), "route gaps: {:?}", route.gaps);
    let surface = renderer
        .compose_xui23_ability_surface(
            &route,
            &viewer.abilities,
            250_000,
            PRODUCTION_UI_WIDTH,
            PRODUCTION_UI_HEIGHT,
        )
        .unwrap()
        .expect("nonempty XUI23 production surface");
    assert_eq!(surface.authored_root, "abilitylist_window");
    assert_eq!(surface.button_regions.len(), 1);
    assert!(surface
        .input_region
        .contains(surface.button_regions[0].x, surface.button_regions[0].y));
    assert!(surface.pixels.report.missing_sprites.is_empty());
    assert!(
        surface.pixels.report.undecodable_sprites.is_empty(),
        "installed XUI23 sprite errors: {:?}",
        surface.pixels.report.undecodable_sprites
    );
}

#[test]
#[ignore = "XUI21 installed shipping-renderer Evidence: requires the game install and is intentionally excluded from the deterministic default tier"]
fn installed_xui21_shipping_renderer_admits_complete_event_namespace_and_composes_popup() {
    use game_ui::event_chrome::{
        AdmittedEventOption, AdmittedEventPopup, AdmittedFiredEvent, EventPopupSubtype,
        EventRuntimeImageIdentity,
    };
    use game_ui::production_renderer::ObserverEventRoute;
    use game_ui::text_law::{ExactRuntimeText, RuntimeTextContext};
    use replay_core::{RuntimeTextLayout, RuntimeTextSource};

    let renderer = xui21_production_renderer(installed_game_root());
    let definitions = renderer
        .event_definitions()
        .expect("complete authenticated event namespace reaches shipping renderer");
    assert_eq!(definitions.len(), 1);
    assert!(definitions.contains_key("low.1"));

    let operative_text = |element: &str, text: &str| RuntimeTextContext {
        exact: Some(ExactRuntimeText {
            view_id: "EventWindow_Operative".into(),
            element_id: element.into(),
            source: RuntimeTextSource::SetterCallerRva(0x0122_5b30),
            engine_utf8: text.into(),
            layout: RuntimeTextLayout::AuthoredReproducible,
        }),
        ..Default::default()
    };
    let events = [
        EventPopupSubtype::Country,
        EventPopupSubtype::State,
        EventPopupSubtype::News,
        EventPopupSubtype::UnitLeader,
        EventPopupSubtype::OperativeLeader,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, subtype)| {
        let leader = matches!(
            subtype,
            EventPopupSubtype::UnitLeader | EventPopupSubtype::OperativeLeader
        );
        let operative = subtype == EventPopupSubtype::OperativeLeader;
        AdmittedFiredEvent {
            source_file: "events/LowCountries.txt".into(),
            event_id: "low.1".into(),
            scope_country_tag_db_id: 1,
            recipient_tag_db_id: Some(1),
            recipient_tag: Some("HOL".into()),
            fire_date: "1936.1.1".into(),
            fire_hour: 100,
            fire_tick: 100,
            event_sequence: index as i64 + 1,
            definition_hidden: false,
            chosen_option_ordinal: None,
            effective_timeout_hours: 24,
            timeout_tick: 124,
            resolved_tick: None,
            resolution_reason: "pending".into(),
            runtime_title_text_context_id: Some(format!("ctx:title:{index}")),
            runtime_description_text_context_id: Some(format!("ctx:description:{index}")),
            runtime_option_text_context_ids: BTreeMap::from([(0, format!("ctx:option:{index}:0"))]),
            popup: Some(AdmittedEventPopup {
                localisation_misses: Default::default(),
                subtype,
                window_name: subtype.window_name(),
                title: format!("Replay event {index}"),
                title_runtime_text: RuntimeTextContext::default(),
                description: "Exact positive production-compositor evidence.".into(),
                description_runtime_text: RuntimeTextContext::default(),
                selected_desc_key: "low.1.d".into(),
                picture_gfx: None,
                leader_portrait: leader
                    .then(|| EventRuntimeImageIdentity::GfxName("GFX_leader_unknown".into())),
                leader_fullname: operative.then(|| {
                    (
                        "Jane Doe".into(),
                        operative_text("leader_fullname", "Jane Doe"),
                    )
                }),
                leader_codename: operative.then(|| {
                    (
                        "Sparrow".into(),
                        operative_text("leader_codename", "Sparrow"),
                    )
                }),
                agency_insignia: operative.then(|| {
                    EventRuntimeImageIdentity::FilePath(
                        "gfx/interface/operatives/agencies/agency_logo_prc.dds".into(),
                    )
                }),
                options: vec![AdmittedEventOption {
                    definition_ordinal: 0,
                    text: "Observe".into(),
                    runtime_text: RuntimeTextContext::default(),
                    selected: false,
                }],
            }),
            popup_suppression: None,
        }
    })
    .collect::<Vec<_>>();
    // The identity has to come FROM A FRAME: there is no constructor that
    // takes an id, so this route names HOL only because this frame does.
    #[derive(Clone)]
    struct HolFrame;
    impl ReplaySource for HolFrame {
        fn game_id(&self) -> &str {
            "xui21-shipping-compositor"
        }
        fn hours(&self) -> &[i64] {
            &[100]
        }
        fn warnings(&self) -> &[String] {
            &[]
        }
        fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: serde_json::json!({
                    "countries": { "HOL": { "_r2_tag_db_id": 1 } },
                    "tag_db_id_index": { "country_composition_version": 2 },
                })
                .as_object()
                .expect("frame object")
                .clone()
                .into(),
            })
        }
    }
    let timeline = WorldTimeline::open(Box::new(HolFrame)).expect("timeline");
    let frame = timeline.frame(100).expect("frame");
    let mut route = ObserverEventRoute::authenticated_for_selection(
        &frame,
        &game_ui::stored_country_selection::StoredCountrySelection::of("HOL"),
    );
    assert_eq!(
        route.country_tag_db_id(),
        Some(1),
        "the frame authenticates HOL as country 1"
    );
    route.admitted_history = Some(events);
    route.event_tick = Some(100);
    route.visible_sequences = vec![1, 2, 3, 4, 5];
    route.focused_sequence = Some(5);

    let outputs = renderer
        .compose_xui21_event_stack(&route, PRODUCTION_UI_WIDTH, PRODUCTION_UI_HEIGHT)
        .expect("the shipping renderer composes an admitted positive XUI21 popup");
    assert_eq!(outputs.len(), 5);
    assert_eq!(
        outputs
            .iter()
            .map(|output| output.root_window.as_str())
            .collect::<Vec<_>>(),
        vec![
            "EventWindow",
            "EventWindow",
            "EventWindow_News",
            "EventWindow_leader",
            "EventWindow_Operative",
        ]
    );
    for (index, output) in outputs.iter().enumerate() {
        assert_eq!(output.event_sequence, index as i64 + 1);
        assert_eq!(output.option_targets.len(), 1);
        assert!(output.root_region.width > 0 && output.root_region.height > 0);
        assert!(output.image.pixels.iter().any(|pixel| *pixel != 0));
    }
}

#[test]
fn xui12_shipping_consumer_requires_the_detached_passive_all_country_carrier() {
    struct LegacyProductionSource {
        rest: serde_json::Map<String, serde_json::Value>,
    }
    impl ReplaySource for LegacyProductionSource {
        fn game_id(&self) -> &str {
            "xui12-legacy-production"
        }

        fn hours(&self) -> &[i64] {
            &[16_959_368]
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: {
                    let mut rest = self.rest.clone();
                    if replay_core::COUNTRY_COMPOSED_MEMBERS
                        .iter()
                        .any(|member| rest.contains_key(*member))
                    {
                        rest.entry("tag_db_id_index").or_insert_with(
                            || serde_json::json!({"country_composition_version": 2}),
                        );
                    }
                    rest.into()
                },
            })
        }
    }

    let mut rest = serde_json::Map::new();
    rest.insert(
        "countries".into(),
        serde_json::json!({
            "GER": {
                "_r2_tag_db_id": 1,
                "production": {
                    "stockpile_complete": true,
                    "military_lines_complete": true,
                    "naval_lines_complete": true,
                    "military_lines": [{
                        "id": 41,
                        "line_subtype": "military",
                        "efficiency": 0.72,
                        "efficiency_cap": 0.85,
                        "efficiency_growth": 0.003
                    }],
                    "naval_lines": [{
                        "id": 72,
                        "line_subtype": "ship_refit"
                    }]
                }
            }
        }),
    );
    let timeline = WorldTimeline::open(Box::new(LegacyProductionSource { rest })).unwrap();
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

    assert!(PROMOTED_XUI_PACKETS.contains(&"XUI12"));
    assert!(!OMITTED_PIXEL_CANVAS_ROUTES
        .iter()
        .any(|(packet, _)| *packet == "XUI12"));
    assert_eq!(
        game_ui::production_lines::xui12_producer_payload_from_frame(frame.as_ref()),
        Ok(None),
        "legacy selected-country production data must not impersonate the detached XUI12 carrier"
    );
    assert!(PROMOTED_EXTERNAL_DEPENDENCY_PACKETS
        .iter()
        .any(|(packet, requirement)| *packet == "XUI12"
            && requirement.contains("passive UI-open-independent")
            && requirement.contains("complete-D18")));
    assert!(!OMITTED_AUTHORITY_BLOCKED_PACKETS
        .iter()
        .any(|(packet, _)| *packet == "XUI12"));
}
