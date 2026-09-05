use game_ui::intelligence_presentation::{
    intelligence_tooltip_payloads_at, render_intelligence, IntelligenceInputRegion,
    IntelligenceViewerState, IntelligenceViewerSurface, AGENCY_GUI, LEDGER_GUI, OPERATION_GUI,
    OPERATIVE_GUI,
};
use game_ui::{assets::AssetResolver, gfx::SpriteRegistry};
use replay_core::{
    IntelligenceAgencyExact, IntelligenceAssignmentExact, IntelligenceAssignmentKindExact,
    IntelligenceCipherExact, IntelligenceCountryPresentationExact, IntelligenceLevelExact,
    IntelligenceNetworkExact, IntelligenceNetworkStateExact, IntelligenceOperationExact,
    IntelligenceOperationStatusExact, IntelligenceOperativeExact, IntelligenceTextExact,
    IntelligenceTooltipExact, IntelligenceUpgradeExact,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn viewer_country_surface_and_scroll_are_replay_owned() {
    let mut viewer = IntelligenceViewerState::default();
    viewer.toggle(Some("GER"));
    assert!(viewer.panel_open);
    assert!(viewer.country_tag.is("GER"));
    assert_eq!(viewer.surface, IntelligenceViewerSurface::Agency);

    let expected_cycle = [
        IntelligenceViewerSurface::Operations,
        IntelligenceViewerSurface::Cryptology,
        IntelligenceViewerSurface::IntelLedger,
        IntelligenceViewerSurface::OperationOverview,
        IntelligenceViewerSurface::Agency,
    ];
    let mut surface = IntelligenceViewerSurface::Agency;
    for expected in expected_cycle {
        surface = surface.next();
        assert_eq!(surface, expected);
    }

    viewer.select_surface(IntelligenceViewerSurface::IntelLedger);
    viewer.wheel(-3, 5);
    assert_eq!(viewer.row_scroll, 3);
    viewer.wheel(-99, 5);
    assert_eq!(viewer.row_scroll, 4);
    viewer.reconcile_country(&game_ui::stored_country_selection::StoredCountrySelection::of("FRA"));
    assert!(viewer.country_tag.is("FRA"));
    assert_eq!(viewer.surface, IntelligenceViewerSurface::Agency);
    assert_eq!(viewer.row_scroll, 0);
    viewer.toggle(Some("FRA"));
    assert!(!viewer.panel_open);
}

#[test]
fn authored_input_regions_are_half_open_and_all_four_roots_are_required() {
    let region = IntelligenceInputRegion {
        panel: [10, 20, 100, 200],
        close: Some([90, 20, 20, 20]),
        operations_tab: Some([10, 40, 40, 20]),
        crypto_tab: Some([50, 40, 40, 20]),
        row_count: 7,
    };
    assert!(region.panel_contains(10, 20));
    assert!(region.panel_contains(109, 219));
    assert!(!region.panel_contains(110, 220));
    assert!(region.close_contains(100, 30));
    assert!(region.operations_contains(20, 50));
    assert!(region.crypto_contains(60, 50));
    assert_eq!(
        [AGENCY_GUI, OPERATIVE_GUI, LEDGER_GUI, OPERATION_GUI],
        [
            "interface/countryintelligenceagencyview.gui",
            "interface/operativeleader.gui",
            "interface/countryintelledger.gui",
            "interface/operationoverview.gui",
        ]
    );
}

struct FixtureRoot(PathBuf);

impl FixtureRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "xui14-effective-vfs-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("interface")).expect("create fixture interface");
        fs::create_dir_all(root.join("gfx/interface")).expect("create fixture gfx");
        Self(root)
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, body: &str) {
    fs::write(path, body).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn write_effective_vfs(root: &Path) {
    write(
        &root.join("interface/core.gui"),
        r#"guiTypes = {
  textBoxType = { name = "xui14_tooltip" textureFile = "gfx/interface/xui14.tga" borderSize = { x = 1 y = 1 } font = "hoi_18mbs" maxWidth = 300 maxHeight = 300 }
}"#,
    );
    write(
        &root.join(AGENCY_GUI),
        r#"guiTypes = {
  containerWindowType = {
    name = "countryintelligenceagencyview" show_position = { x = 10 y = 10 } size = { width = 550 height = 560 }
    iconType = { name = "agency_logo" position = { x = 10 y = 10 } size = { width = 32 height = 32 } }
    instantTextboxType = { name = "agency_name" position = { x = 50 y = 10 } maxWidth = 180 maxHeight = 20 }
    instantTextboxType = { name = "agency_defense_level" position = { x = 50 y = 32 } maxWidth = 100 maxHeight = 20 }
    instantTextboxType = { name = "agency_crypto_defense_level" position = { x = 155 y = 32 } maxWidth = 100 maxHeight = 20 }
    instantTextboxType = { name = "total_operative_text" position = { x = 260 y = 32 } maxWidth = 100 maxHeight = 20 }
    buttonType = { name = "close_button" position = { x = 520 y = 5 } size = { width = 20 height = 20 } }
    buttonType = { name = "operations_tab_button" position = { x = 20 y = 60 } size = { width = 80 height = 20 } }
    buttonType = { name = "crypto_tab_button" position = { x = 110 y = 60 } size = { width = 80 height = 20 } }
    containerWindowType = { name = "agency_branches" position = { x = 10 y = 90 } size = { width = 520 height = 200 }
      gridBoxType = { name = "upgrades_list" position = { x = 0 y = 0 } size = { width = 500 height = 180 } slotsize = { width = 100 height = 70 } max_slots_horizontal = 4 format = UPPER_LEFT }
    }
    containerWindowType = { name = "agency_agents" position = { x = 10 y = 300 } size = { width = 520 height = 200 } }
    containerWindowType = { name = "agency_operations" position = { x = 10 y = 90 } size = { width = 520 height = 400 }
      gridBoxType = { name = "operations_grid" position = { x = 0 y = 0 } size = { width = 500 height = 380 } slotsize = { width = 220 height = 80 } max_slots_horizontal = 2 format = UPPER_LEFT }
    }
    containerWindowType = { name = "agency_crypto" position = { x = 10 y = 90 } size = { width = 520 height = 400 }
      gridBoxType = { name = "country_list" position = { x = 0 y = 0 } size = { width = 500 height = 380 } slotsize = { width = 240 height = 60 } max_slots_horizontal = 2 format = UPPER_LEFT }
    }
  }
  containerWindowType = { name = "upgrade_button" size = { width = 96 height = 64 }
    iconType = { name = "upgrade_icon" size = { width = 24 height = 24 } }
    instantTextboxType = { name = "agency_branches_title" position = { x = 26 y = 0 } maxWidth = 65 maxHeight = 40 }
  }
  containerWindowType = { name = "operation_view_entry" size = { width = 210 height = 70 }
    iconType = { name = "operation_bg" size = { width = 210 height = 70 } }
    iconType = { name = "operation_icon" position = { x = 4 y = 4 } size = { width = 28 height = 28 } }
    instantTextboxType = { name = "operation_name_text" position = { x = 36 y = 4 } maxWidth = 165 maxHeight = 18 }
    instantTextboxType = { name = "operation_duration_text" position = { x = 36 y = 24 } maxWidth = 165 maxHeight = 18 }
    instantTextboxType = { name = "network_strength_text" position = { x = 36 y = 44 } maxWidth = 165 maxHeight = 18 }
    containerWindowType = { name = "container_launch_operation" size = { width = 1 height = 1 } }
  }
  containerWindowType = { name = "cryptology_country_entry" size = { width = 230 height = 54 }
    iconType = { name = "bg" size = { width = 230 height = 54 } }
    iconType = { name = "flag" position = { x = 4 y = 4 } size = { width = 30 height = 20 } }
    instantTextboxType = { name = "name" position = { x = 38 y = 4 } maxWidth = 180 maxHeight = 18 }
    instantTextboxType = { name = "cryptology_state" position = { x = 38 y = 25 } maxWidth = 180 maxHeight = 18 }
    progressbarType = { name = "decrytion_bar" position = { x = 4 y = 45 } size = { width = 220 height = 5 } }
  }
}"#,
    );
    write(
        &root.join(OPERATIVE_GUI),
        r#"guiTypes = {
  containerWindowType = { name = "operative_leader_window" position = { x = 20 y = 330 } size = { width = 500 height = 180 }
    gridBoxType = { name = "operatives_box" size = { width = 480 height = 160 } slotsize = { width = 90 height = 90 } max_slots_horizontal = 5 format = UPPER_LEFT }
  }
  containerWindowType = { name = "operative_leader_portrait_window" size = { width = 80 height = 80 }
    iconType = { name = "portrait_btn" size = { width = 64 height = 64 } }
    iconType = { name = "leader_captured" size = { width = 12 height = 12 } }
    instantTextboxType = { name = "operative_skill_level" position = { x = 64 y = 0 } maxWidth = 12 maxHeight = 12 }
  }
}"#,
    );
    write(
        &root.join(LEDGER_GUI),
        r#"guiTypes = {
  containerWindowType = { name = "country_intel_ledger_view" show_position = { x = 20 y = 20 } size = { width = 520 height = 500 }
    containerWindowType = { name = "country_info" position = { x = 0 y = 50 } }
    containerWindowType = { name = "economy_panel" size = { width = 1 height = 1 } }
    containerWindowType = { name = "army_panel" size = { width = 1 height = 1 } }
    containerWindowType = { name = "navy_panel" size = { width = 1 height = 1 } }
    containerWindowType = { name = "air_panel" size = { width = 1 height = 1 } }
    containerWindowType = { name = "economy_tab_frame" position = { x = 10 y = 80 } size = { width = 120 height = 80 }
      buttonType = { name = "tab_button" size = { width = 1 height = 1 } }
      instantTextboxType = { name = "intel" maxWidth = 115 maxHeight = 20 }
    }
    containerWindowType = { name = "army_tab_frame" position = { x = 140 y = 80 } size = { width = 120 height = 80 }
      buttonType = { name = "tab_button" size = { width = 1 height = 1 } }
      instantTextboxType = { name = "intel" maxWidth = 115 maxHeight = 20 }
    }
    containerWindowType = { name = "navy_tab_frame" position = { x = 270 y = 80 } size = { width = 120 height = 80 }
      buttonType = { name = "tab_button" size = { width = 1 height = 1 } }
      instantTextboxType = { name = "intel" maxWidth = 115 maxHeight = 20 }
    }
    containerWindowType = { name = "air_tab_frame" position = { x = 400 y = 80 } size = { width = 110 height = 80 }
      buttonType = { name = "tab_button" size = { width = 1 height = 1 } }
      instantTextboxType = { name = "intel" maxWidth = 105 maxHeight = 20 }
    }
  }
}"#,
    );
    write(
        &root.join(OPERATION_GUI),
        r#"guiTypes = {
  containerWindowType = { name = "operation_overview_window" show_position = { x = 20 y = 20 } size = { width = 520 height = 500 }
    buttonType = { name = "btn_close" position = { x = 490 y = 5 } size = { width = 20 height = 20 } }
    iconType = { name = "operation_icon" position = { x = 10 y = 10 } size = { width = 40 height = 40 } }
    buttonType = { name = "btn_target_flag" position = { x = 60 y = 10 } size = { width = 40 height = 24 } }
    instantTextboxType = { name = "operation_name_text" position = { x = 110 y = 10 } maxWidth = 300 maxHeight = 20 }
    instantTextboxType = { name = "operation_progress_text" position = { x = 110 y = 35 } maxWidth = 300 maxHeight = 20 }
    instantTextboxType = { name = "operation_duration_text" position = { x = 110 y = 60 } maxWidth = 300 maxHeight = 20 }
    instantTextboxType = { name = "flag_name" position = { x = 60 y = 38 } maxWidth = 100 maxHeight = 20 }
    progressbarType = { name = "progressbar_content" position = { x = 10 y = 90 } size = { width = 480 height = 10 } }
    containerWindowType = { name = "container_target_info" size = { width = 1 height = 1 } }
    containerWindowType = { name = "duration_container" size = { width = 1 height = 1 } }
    containerWindowType = { name = "operatives_container" position = { x = 10 y = 120 } size = { width = 480 height = 160 }
      gridBoxType = { name = "operatives_grid" size = { width = 460 height = 140 } slotsize = { width = 90 height = 90 } max_slots_horizontal = 5 format = UPPER_LEFT }
    }
    containerWindowType = { name = "resources_grid_container" size = { width = 1 height = 1 } }
    containerWindowType = { name = "outlook_container" size = { width = 1 height = 1 } }
    containerWindowType = { name = "phases_container" size = { width = 1 height = 1 } }
    containerWindowType = { name = "container_bottom" size = { width = 1 height = 1 } }
    containerWindowType = { name = "reward_container" size = { width = 1 height = 1 } }
    containerWindowType = { name = "btn_location" size = { width = 1 height = 1 } }
  }
  containerWindowType = { name = "operation_operative_leader_portrait_window" size = { width = 80 height = 80 }
    iconType = { name = "portrait_btn" size = { width = 64 height = 64 } }
    iconType = { name = "leader_captured" size = { width = 12 height = 12 } }
  }
}"#,
    );
    let sprites = [
        "GFX_agency",
        "GFX_upgrade",
        "GFX_operative",
        "GFX_operation",
        "GFX_FRA",
        "GFX_ENG",
    ]
    .into_iter()
    .map(|name| {
        format!("spriteType = {{ name = \"{name}\" texturefile = \"gfx/interface/xui14.tga\" }}")
    })
    .collect::<Vec<_>>()
    .join("\n");
    write(
        &root.join("interface/xui14.gfx"),
        &format!("spriteTypes = {{\n{sprites}\n}}"),
    );
    let mut tga = vec![0u8; 18];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&1u16.to_le_bytes());
    tga[14..16].copy_from_slice(&1u16.to_le_bytes());
    tga[16] = 32;
    tga[17] = 0x28;
    tga.extend_from_slice(&[0, 80, 160, 255]);
    fs::write(root.join("gfx/interface/xui14.tga"), tga).expect("write fixture texture");
}

fn text(identity: &str, value: &str) -> IntelligenceTextExact {
    IntelligenceTextExact {
        authored_identity: identity.into(),
        final_text: value.into(),
        final_text_sha256: "0".repeat(64),
    }
}

fn tooltip(identity: &str, primary: &str) -> IntelligenceTooltipExact {
    IntelligenceTooltipExact {
        primary: text(&format!("{identity}/primary"), primary),
        delayed: Some(text(
            &format!("{identity}/delayed"),
            "Exact delayed owner text",
        )),
    }
}

fn presentation(with_network: bool) -> IntelligenceCountryPresentationExact {
    let operation = IntelligenceOperationExact {
        source_ordinal: 0,
        stable_item_id: "operation:1".into(),
        operation_id: "operation:1".into(),
        definition_id: "steal_blueprints".into(),
        target_country: Some("FRA".into()),
        target_state_id: None,
        status: IntelligenceOperationStatusExact::Running,
        phase_index: 1,
        phase_count: 2,
        progress_raw: 50,
        progress_denominator_raw: 100,
        start_hour: 4,
        duration_hours: 48,
        assigned_operative_ids: vec!["operative:1".into()],
        icon_sprite: "GFX_operation".into(),
        display: text("operation/display", "Steal Blueprints"),
        status_text: text("operation/status", "In progress"),
        target_text: text("operation/target", "France"),
        tooltip: tooltip("operation", "Steal Blueprints in France"),
    };
    let levels = [
        ("FRA", 12u32, "France", "GFX_FRA"),
        ("ENG", 13u32, "United Kingdom", "GFX_ENG"),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, (tag, tag_id, name, flag))| IntelligenceLevelExact {
            source_ordinal: index as u32,
            stable_item_id: format!("intel:{tag}"),
            target_country: tag.into(),
            target_country_tag_db_id: tag_id,
            civilian_raw: 10,
            army_raw: 20,
            navy_raw: 30,
            air_raw: 40,
            scale_raw: 100,
            target_text: text(&format!("intel/{tag}/target"), name),
            civilian_text: text(&format!("intel/{tag}/civilian"), "Civilian 10%"),
            army_text: text(&format!("intel/{tag}/army"), "Army 20%"),
            navy_text: text(&format!("intel/{tag}/navy"), "Navy 30%"),
            air_text: text(&format!("intel/{tag}/air"), "Air 40%"),
            flag_sprite: flag.into(),
            tooltip: tooltip(
                &format!("intel/{tag}"),
                &format!("{name} intelligence ledger"),
            ),
        },
    )
    .collect::<Vec<_>>();
    IntelligenceCountryPresentationExact {
        source_ordinal: 0,
        country_tag: "GER".into(),
        tag_db_id: 47,
        source_record_identity_sha256: "1".repeat(64),
        effective_controller_identity_sha256: "2".repeat(64),
        effective_root_identity_sha256: "3".repeat(64),
        expected_upgrade_count: 1,
        expected_operative_count: 1,
        expected_operation_count: 1,
        expected_cipher_count: 1,
        expected_intel_level_count: 2,
        expected_network_count: u32::from(with_network),
        agency: IntelligenceAgencyExact {
            agency_instance_id: "agency:GER".into(),
            created: true,
            name: text("agency/name", "Abwehr"),
            icon_sprite: "GFX_agency".into(),
            upgrade_progress_raw: 25,
            upgrade_progress_denominator_raw: 100,
            current_upgrade_id: Some("upgrade:1".into()),
            max_operatives: 2,
            usable_slots: 1,
            defense_raw: 50,
            defense_denominator_raw: 100,
            capacity_text: text("agency/capacity", "1 / 2"),
            defense_text: text("agency/defense", "50%"),
            tooltip: tooltip("agency", "German intelligence agency"),
        },
        upgrades: vec![IntelligenceUpgradeExact {
            source_ordinal: 0,
            stable_item_id: "upgrade:1".into(),
            upgrade_id: "upgrade:1".into(),
            branch_id: "branch:1".into(),
            level: 1,
            icon_sprite: "GFX_upgrade".into(),
            display: text("upgrade/display", "Invisible Ink"),
            tooltip: tooltip("upgrade", "Invisible Ink"),
        }],
        operatives: vec![IntelligenceOperativeExact {
            source_ordinal: 0,
            stable_item_id: "operative:1".into(),
            operative_id: "operative:1".into(),
            name: text("operative/name", "Ada"),
            portrait_sprite: "GFX_operative".into(),
            skill: 1,
            experience_raw: 20,
            experience_denominator_raw: 100,
            experience_text: text("operative/experience", "20%"),
            trait_ids: vec!["commando".into()],
            trait_texts: vec![text("operative/trait", "Commando")],
            nationality_tags: vec!["GER".into()],
            nationality_texts: vec![text("operative/nationality", "German")],
            captured: false,
            captured_by: None,
            captured_by_tag_db_id: None,
            capture_end_date_raw: None,
            recruitable_pool_member: false,
            assignment: IntelligenceAssignmentExact {
                kind: IntelligenceAssignmentKindExact::Operation,
                assignment_identity: Some("operation:1".into()),
                action_identity: None,
                target_country: Some("FRA".into()),
                target_country_tag_db_id: Some(12),
                target_state_id: None,
                authored_subject_id: None,
                final_text: text("operative/assignment", "Assigned"),
                tooltip: tooltip("operative/assignment", "Assigned to operation"),
            },
            tooltip: tooltip("operative", "Ada operative"),
        }],
        operations: vec![operation],
        ciphers: vec![IntelligenceCipherExact {
            source_ordinal: 0,
            stable_item_id: "cipher:FRA".into(),
            target_country: "FRA".into(),
            target_country_tag_db_id: 12,
            decryption_points_raw: 40,
            progress_denominator_raw: 100,
            decrypting: true,
            cipher_broken: false,
            target_text: text("cipher/target", "France"),
            progress_text: text("cipher/progress", "40%"),
            flag_sprite: "GFX_FRA".into(),
            tooltip: tooltip("cipher", "French cipher"),
        }],
        intel_levels: levels,
        networks: if with_network {
            vec![IntelligenceNetworkExact {
                source_ordinal: 0,
                stable_item_id: "network:FRA".into(),
                network_id: "network:FRA".into(),
                target_country: "FRA".into(),
                target_country_tag_db_id: 12,
                operative_ids: vec!["operative:1".into()],
                state_strengths: vec![IntelligenceNetworkStateExact {
                    state_id: 28,
                    strength_raw: 50,
                    strength_denominator_raw: 100,
                }],
                national_coverage_raw: 50,
                national_coverage_denominator_raw: 100,
                target_text: text("network/target", "France"),
                coverage_text: text("network/coverage", "50%"),
                flag_sprite: "GFX_FRA".into(),
                tooltip: tooltip("network", "Network in France"),
            }]
        } else {
            Vec::new()
        },
    }
}

#[test]
fn effective_vfs_compositor_executes_all_surfaces_and_preserves_exact_owners() {
    let fixture = FixtureRoot::new();
    write_effective_vfs(&fixture.0);
    let resolver = AssetResolver::new(vec![fixture.0.clone()]);
    let mut sprites = SpriteRegistry::new();
    sprites
        .parse_content(&map_viewer::GameFileResolver::vanilla(&fixture.0).unwrap().read_required_bytes("interface/xui14.gfx", "fixture content").unwrap(), "interface/xui14.gfx")
        .expect("parse fixture sprites");
    let producer_manifest = "4".repeat(64);

    let mut empty_agency = presentation(false);
    empty_agency.agency.created = false;
    empty_agency.upgrades.clear();
    empty_agency.operatives.clear();
    empty_agency.operations.clear();
    empty_agency.ciphers.clear();
    empty_agency.intel_levels.clear();
    let empty = render_intelligence(
        &resolver,
        &sprites,
        &empty_agency,
        &producer_manifest,
        &IntelligenceViewerState {
            panel_open: true,
            country_tag: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
            surface: IntelligenceViewerSurface::Agency,
            row_scroll: 0,
        },
        800,
        600,
    )
    .expect("compose authenticated empty/not-created agency");
    assert!(empty.displayed_row_ids.is_empty());
    assert_eq!(empty.tooltip_targets.len(), 1);
    assert_eq!(
        empty.tooltip_targets[0].payloads.primary.rows()[0].text,
        "German intelligence agency"
    );

    for with_network in [false, true] {
        let presentation = presentation(with_network);
        for surface in [
            IntelligenceViewerSurface::Agency,
            IntelligenceViewerSurface::Operations,
            IntelligenceViewerSurface::Cryptology,
            IntelligenceViewerSurface::IntelLedger,
            IntelligenceViewerSurface::OperationOverview,
        ] {
            let mut viewer = IntelligenceViewerState {
                panel_open: true,
                country_tag: game_ui::stored_country_selection::StoredCountrySelection::of("GER"),
                surface,
                row_scroll: 0,
            };
            if surface == IntelligenceViewerSurface::IntelLedger {
                viewer.wheel(-1, presentation.intel_levels.len());
                assert_eq!(viewer.row_scroll, 1);
            }
            let composed = render_intelligence(
                &resolver,
                &sprites,
                &presentation,
                &producer_manifest,
                &viewer,
                800,
                600,
            )
            .unwrap_or_else(|error| panic!("compose {surface:?}, network={with_network}: {error}"));
            assert!(composed.output.report.missing_sprites.is_empty());
            assert!(composed.output.report.undecodable_sprites.is_empty());
            assert!(composed.output.report.localisation_misses.is_empty());
            assert!(composed.output.report.unresolved_text_inputs.is_empty());
            assert!(composed
                .tooltip_targets
                .iter()
                .flat_map(|target| target.payloads.primary.rows())
                .all(|row| !row.text.contains("Network in France")));
            for target in &composed.tooltip_targets {
                assert_eq!(target.payloads.primary.rows().len(), 1);
                let source = target.payloads.primary.owner().source_file();
                match surface {
                    IntelligenceViewerSurface::Agency => {
                        assert!(source == AGENCY_GUI || source == OPERATIVE_GUI)
                    }
                    IntelligenceViewerSurface::Operations
                    | IntelligenceViewerSurface::Cryptology => assert_eq!(source, AGENCY_GUI),
                    IntelligenceViewerSurface::IntelLedger => {
                        panic!("the size-less ledger context hook must expose no hover owner")
                    }
                    IntelligenceViewerSurface::OperationOverview => {
                        assert_eq!(source, OPERATION_GUI)
                    }
                }
                let point = (target.rect.x, target.rect.y);
                assert!(
                    intelligence_tooltip_payloads_at(&composed.tooltip_targets, point).is_some()
                );
            }
            if surface == IntelligenceViewerSurface::IntelLedger {
                assert_eq!(composed.displayed_row_ids, ["intel:ENG"]);
                assert_eq!(composed.input_region.row_count, 2);
                assert!(composed.tooltip_targets.is_empty());
                assert!(
                    intelligence_tooltip_payloads_at(&composed.tooltip_targets, (0, 0)).is_none()
                );
                viewer.wheel(1, presentation.intel_levels.len());
                assert_eq!(viewer.row_scroll, 0);
            }
            if surface == IntelligenceViewerSurface::Operations {
                assert!(composed
                    .tooltip_targets
                    .iter()
                    .flat_map(|target| target.payloads.primary.rows())
                    .any(|row| row.text == "Steal Blueprints in France"));
            }
        }
    }
}
