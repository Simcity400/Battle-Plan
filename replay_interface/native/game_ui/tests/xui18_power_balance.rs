use game_ui::compose::{progress_minmax_uses_front, ComposeOptions};
use game_ui::decision_chrome::{DecisionAvailability, DecisionDef, DecisionSet};
use game_ui::power_balance::{
    admit_power_balance_screen, apply_power_balance_launcher_to_compose_options,
    apply_power_balance_to_compose_options, sample_panel_transition,
    AdmittedPowerBalanceScreen, BarState, ChangeCadence, PanelTransition,
    PowerBalanceAdmissionRequest, PowerBalanceDecisionInput, PowerBalanceDefinitionCatalog,
    PowerBalanceDecisionTextRole, PowerBalanceRejection, PowerBalanceRuntimeProvenance,
    PowerBalanceRuntimeSupplement, PowerBalanceRuntimeTextBinding,
    PowerBalanceRuntimeTextProvenance, PowerBalanceRuntimeTextSlot, PowerBalanceTrendInput,
    ResolvedPowerBalanceText,
    PANEL_HIDDEN_POSITION, PANEL_SHOWN_POSITION,
};
use game_ui::power_balance_assets::{
    PowerBalanceAssetCatalog, PowerBalanceAssetError, PowerBalanceAssetRole,
};
use replay_core::{Leaf, PowerBalanceRow, PowerBalanceView, RuntimeTextIdentitySource, ViewStatus};
use std::path::PathBuf;
use snapshot::sha256::Sha256;

fn portable_asset_manifest() -> String {
    fn row(name: &str, kind: &str, frames: Option<u32>, slots: &[&str]) -> serde_json::Value {
        let texture_slots = slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                serde_json::json!({
                    "slot": slot,
                    "resolved_texturefile": format!(
                        "gfx/interface/fixture/{}_{}.dds",
                        name.to_ascii_lowercase(),
                        index
                    ),
                    "texture_layer": "vanilla",
                    "texture_layer_kind": "vanilla",
                    "texture_bytes": 128 + index,
                    "texture_sha256": format!("{:064x}", index + 1),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "name": name,
            "drawable_kind": kind,
            "source_file": "interface/powerbalanceview.gfx",
            "source_layer": "vanilla",
            "source_layer_kind": "vanilla",
            "frames": frames.map(|value| value.to_string()),
            "texture_slots": texture_slots,
            "resolved_texturefile": null,
            "texture_layer": null,
            "texture_layer_kind": null,
            "texture_bytes": null,
            "texture_sha256": null
        })
    }

    let names = [
        "GFX_tiled_plain_bg",
        "GFX_bop_background",
        "GFX_tiled_window2_1b_border",
        "GFX_header_bg",
        "GFX_closebutton",
        "GFX_tiled_decisions_bg_small",
        "GFX_bop_bar_frame",
        "GFX_bop_needle",
        "GFX_bop_splitter",
        "GFX_bop_ITA_grand_council_side",
        "GFX_bop_ITA_mussolini_side",
        "GFX_show_power_balance_button",
    ];
    let mut sprites = serde_json::Map::new();
    for name in names {
        sprites.insert(name.into(), row(name, "spriteType", None, &["texturefile"]));
    }
    sprites.insert(
        "GFX_power_balance_left_moving".into(),
        row(
            "GFX_power_balance_left_moving",
            "progressBarType",
            None,
            &["textureFile1", "textureFile2"],
        ),
    );
    sprites.insert(
        "GFX_bop_indicator".into(),
        row("GFX_bop_indicator", "spriteType", Some(2), &["texturefile"]),
    );
    sprites.insert(
        "GFX_bop_glow_effect".into(),
        row(
            "GFX_bop_glow_effect",
            "frameAnimatedSpriteType",
            Some(2),
            &["texturefile"],
        ),
    );
    fn gui_role(
        source_file: &str,
        hierarchy_path: &str,
        widget_type: &str,
        binding_source: &str,
        sprite_name: Option<&str>,
    ) -> serde_json::Value {
        let parent_hierarchy_path = hierarchy_path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        let widget_type_ordinal = match hierarchy_path {
            "range_bar"
            | "powerbalanceview/background"
            | "powerbalanceview/bop_bg"
            | "powerbalanceview/decision_container/Background"
            | "powerbalanceview/close"
            | "country_politics/power_balance_button/show_power_balance"
            | "country_politics/power_balance_button/show_power_balance_glow" => 0,
            "range_indicator"
            | "powerbalanceview/Background"
            | "powerbalanceview/power_balance_header_bg"
            | "country_politics/power_balance_button/power_balance_ICON" => 1,
            "powerbalanceview/power_balance_frame" => 2,
            "powerbalanceview/power_balance_value" => 3,
            "powerbalanceview/position_marker" => 4,
            "powerbalanceview/left_power_icon" => 5,
            "powerbalanceview/right_power_icon" => 6,
            _ => panic!("fixture role lacks authored sibling type ordinal"),
        };
        let authored_sprite_fields = sprite_name
            .map(|sprite| serde_json::json!([{"field":"spriteType","sprite_name":sprite}]))
            .unwrap_or_else(|| serde_json::json!([]));
        let mut row = serde_json::json!({
            "source_file": source_file,
            "source_layer": "fixture_mod",
            "source_layer_kind": "mod",
            "hierarchy_path": hierarchy_path,
            "parent_hierarchy_path": parent_hierarchy_path,
            "widget_type": widget_type,
            "widget_type_ordinal": widget_type_ordinal,
            "widget_name": hierarchy_path.rsplit('/').next().unwrap(),
            "hierarchy_match": "exact_case_sensitive",
            "binding_source": binding_source,
            "authored_sprite_fields": authored_sprite_fields,
        });
        if let Some(sprite) = sprite_name {
            row["sprite_name"] = serde_json::Value::String(sprite.to_string());
        }
        row
    }
    let panel = "interface/powerbalanceview.gui";
    let launcher = "interface/countrypoliticsview.gui";
    let roles = serde_json::json!({
        "panel_background": gui_role(panel, "powerbalanceview/background", "background", "authored", Some("GFX_tiled_plain_bg")),
        "panel_artwork": gui_role(panel, "powerbalanceview/bop_bg", "iconType", "authored", Some("GFX_bop_background")),
        "decision_container_frame": gui_role(panel, "powerbalanceview/decision_container/Background", "background", "authored", Some("GFX_tiled_window2_1b_border")),
        "header_background": gui_role(panel, "powerbalanceview/power_balance_header_bg", "iconType", "authored", Some("GFX_header_bg")),
        "close_button": gui_role(panel, "powerbalanceview/close", "buttonType", "authored", Some("GFX_closebutton")),
        "decision_background": gui_role(panel, "powerbalanceview/Background", "background", "authored", Some("GFX_tiled_decisions_bg_small")),
        "balance_frame": gui_role(panel, "powerbalanceview/power_balance_frame", "iconType", "authored", Some("GFX_bop_bar_frame")),
        "balance_value": gui_role(panel, "powerbalanceview/power_balance_value", "iconType", "engine", None),
        "position_marker": gui_role(panel, "powerbalanceview/position_marker", "iconType", "authored", Some("GFX_bop_needle")),
        "range_indicator": gui_role(panel, "range_indicator", "iconType", "authored", Some("GFX_bop_indicator")),
        "range_splitter": gui_role(panel, "range_bar", "iconType", "authored", Some("GFX_bop_splitter")),
        "left_side_icon": gui_role(panel, "powerbalanceview/left_power_icon", "iconType", "engine", None),
        "right_side_icon": gui_role(panel, "powerbalanceview/right_power_icon", "iconType", "engine", None),
        "launcher_button": gui_role(launcher, "country_politics/power_balance_button/show_power_balance", "buttonType", "authored", Some("GFX_show_power_balance_button")),
        "launcher_glow": gui_role(launcher, "country_politics/power_balance_button/show_power_balance_glow", "iconType", "authored_animation", Some("GFX_bop_glow_effect")),
        "launcher_icon": gui_role(launcher, "country_politics/power_balance_button/power_balance_ICON", "iconType", "engine", None),
    });
    let mut manifest = serde_json::json!({
        "schema": "gfx_icon_asset_manifest_static.static_artifact.v2",
        "power_balance_gui": {
            "schema": "gfx_icon_asset_manifest_static.power_balance_gui_roles.v2",
            "sources": {
                (panel): {"source_layer":"fixture_mod","source_layer_kind":"mod","bytes":321,"sha256":"a".repeat(64)},
                (launcher): {"source_layer":"fixture_mod","source_layer_kind":"mod","bytes":654,"sha256":"b".repeat(64)}
            },
            "roles": roles
        },
        "sprites": sprites,
    });
    let artifact = Sha256::hex(&serde_json::to_vec(&manifest).unwrap());
    manifest
        .as_object_mut()
        .unwrap()
        .insert("artifact_sha256".into(), serde_json::Value::String(artifact));
    serde_json::to_string(&manifest).unwrap()
}

fn committed_ita_catalog() -> PowerBalanceDefinitionCatalog {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xui18_power_balance/catalogs.json");
    let input = std::fs::read_to_string(path).expect("committed XUI18 ITA catalog fixture");
    PowerBalanceDefinitionCatalog::from_catalogs_json(&input)
        .expect("valid committed XUI18 ITA catalog fixture")
}

fn live_row(countries: &[&str], value: f64) -> PowerBalanceRow {
    live_row_trending(countries, value, "ITA_grand_council_side")
}

fn live_row_trending(
    countries: &[&str],
    value: f64,
    trending_side: &str,
) -> PowerBalanceRow {
    PowerBalanceRow {
        template: Leaf::Measured("ITA_power_balance".into()),
        value: Leaf::Measured(value),
        left_side: Leaf::Measured("ITA_grand_council_side".into()),
        right_side: Leaf::Measured("ITA_mussolini_side".into()),
        trending_side: Leaf::Measured(trending_side.into()),
        countries: Leaf::Measured(countries.iter().map(|tag| (*tag).into()).collect()),
    }
}

fn view(countries: &[&str], value: f64) -> PowerBalanceView {
    PowerBalanceView {
        status: ViewStatus::Present,
        balances: vec![live_row(countries, value)],
    }
}

fn decisions() -> DecisionSet {
    DecisionSet {
        decisions: vec![
            DecisionDef {
                id: "ITA_bop_decision_a".into(),
                category: "ITA_balance_of_power_category".into(),
                name_override: None,
                icon_token: Some("political_action".into()),
                template: "decision_item".into(),
                source_file: "common/decisions/ITA.txt".into(),
            },
            DecisionDef {
                id: "ITA_bop_decision_b".into(),
                category: "ITA_balance_of_power_category".into(),
                name_override: None,
                icon_token: None,
                template: "timed_decision_item".into(),
                source_file: "common/decisions/ITA.txt".into(),
            },
        ],
        ..DecisionSet::default()
    }
}

fn runtime() -> PowerBalanceRuntimeSupplement {
    let mut runtime = PowerBalanceRuntimeSupplement {
        provenance: PowerBalanceRuntimeProvenance {
            producer_manifest_sha256:
                "a540af1c030d9ef2fbb1cd290627087627771a196e7a60e4241cb5287b66e4e2"
                    .into(),
            completeness_record_identity: "d74-complete:42".into(),
            frame_hour: 42,
            materialization_qpc_ticks: 90,
            selected_present_candidate_id: 1,
            selected_present_ordinal: 0,
            selected_present_qpc_ticks: 100,
            selected_present_cutoff_qpc_ticks: 100,
            qpc_frequency_hz: 10_000_000,
            selected_present_receipt_sha256: "ab".repeat(32),
            runtime_text_contexts: Vec::new(),
            runtime_text_bindings: Vec::new(),
        },
        source_record_id: "d-bop:hour=42:country=ITA:id=ITA_power_balance".into(),
        country_tag: "ITA".into(),
        balance_id: "ITA_power_balance".into(),
        display_ordinal: 0,
        active_side_id: "ITA_grand_council_side".into(),
        trending_side_id: "ITA_grand_council_side".into(),
        active_range_id: "ITA_grand_council_medium_control_range".into(),
        effective_left_icon: "GFX_bop_ITA_grand_council_side".into(),
        effective_right_icon: "GFX_bop_ITA_mussolini_side".into(),
        launcher_visible: true,
        launcher_percentage_text: Some("32%".into()),
        launcher_levels_text: Some("3/5".into()),
        launcher_icon_sprite: Some("GFX_bop_ITA_grand_council_side".into()),
        launcher_glow_visible: true,
        panel_open: true,
        bar_state: BarState::MovingLeft,
        range_indicator_frame: 0,
        range_indicator_top_left: [198, 101],
        trend: Some(PowerBalanceTrendInput {
            cadence: ChangeCadence::Daily,
            magnitude: 0.01,
            resolved_text: "1.00% per day".into(),
        }),
        power_balance_modifier_desc: "Grand Council drift: +1.00%".into(),
        decisions_complete: true,
        decisions: vec![
            PowerBalanceDecisionInput {
                decision_id: "ITA_bop_decision_b".into(),
                source_record_id: "d45:42:ITA:b".into(),
                availability: DecisionAvailability::Active,
                resolved_name: "Decision B".into(),
                cost_text: Some("50".into()),
                timer_text: Some("12 days".into()),
                target_tag: None,
            },
            PowerBalanceDecisionInput {
                decision_id: "ITA_bop_decision_a".into(),
                source_record_id: "d45:42:ITA:a".into(),
                availability: DecisionAvailability::Available,
                resolved_name: "Decision A".into(),
                cost_text: Some("25".into()),
                timer_text: None,
                target_tag: None,
            },
        ],
        resolved_text: ResolvedPowerBalanceText {
            balance_name: "National Balance of Power".into(),
            left_side_name: "Grand Council".into(),
            right_side_name: "Duce".into(),
            active_range_name: "Medium Council Control".into(),
            balance_tooltip: "The balance trends toward the Grand Council.".into(),
            active_range_tooltip: "Active modifiers".into(),
        },
    };
    attach_runtime_text_provenance(&mut runtime);
    runtime
}

fn attach_runtime_text_provenance(runtime: &mut PowerBalanceRuntimeSupplement) {
    let mut fields = vec![
        (PowerBalanceRuntimeTextSlot::LauncherPercentage, runtime.launcher_percentage_text.as_deref()),
        (PowerBalanceRuntimeTextSlot::LauncherLevels, runtime.launcher_levels_text.as_deref()),
        (PowerBalanceRuntimeTextSlot::Trend, runtime.trend.as_ref().map(|v| v.resolved_text.as_str())),
        (PowerBalanceRuntimeTextSlot::ModifierDescription, Some(runtime.power_balance_modifier_desc.as_str())),
        (PowerBalanceRuntimeTextSlot::BalanceName, Some(runtime.resolved_text.balance_name.as_str())),
        (PowerBalanceRuntimeTextSlot::LeftSideName, Some(runtime.resolved_text.left_side_name.as_str())),
        (PowerBalanceRuntimeTextSlot::RightSideName, Some(runtime.resolved_text.right_side_name.as_str())),
        (PowerBalanceRuntimeTextSlot::ActiveRangeName, Some(runtime.resolved_text.active_range_name.as_str())),
        (PowerBalanceRuntimeTextSlot::BalanceTooltip, Some(runtime.resolved_text.balance_tooltip.as_str())),
        (PowerBalanceRuntimeTextSlot::ActiveRangeTooltip, Some(runtime.resolved_text.active_range_tooltip.as_str())),
    ];
    for decision in &runtime.decisions {
        for (role, text) in [
            (PowerBalanceDecisionTextRole::Name, Some(decision.resolved_name.as_str())),
            (PowerBalanceDecisionTextRole::Cost, decision.cost_text.as_deref()),
            (PowerBalanceDecisionTextRole::Timer, decision.timer_text.as_deref()),
        ] {
            fields.push((PowerBalanceRuntimeTextSlot::Decision {
                decision_id: decision.decision_id.clone(),
                source_record_id: decision.source_record_id.clone(),
                role,
            }, text));
        }
    }
    for (ordinal, (slot, text)) in fields.into_iter().filter_map(|(slot, text)| text.map(|text| (slot, text))).enumerate() {
        let context_id = format!("a25:42:ITA:power-balance:{ordinal}");
        runtime.provenance.runtime_text_contexts.push(PowerBalanceRuntimeTextProvenance {
            context_id: context_id.clone(),
            frame_hour: 42,
            pov_country_or_global: "ITA".into(),
            view_id: "power_balance_view".into(),
            element_id: format!("power_balance_field_{ordinal}"),
            setter_entry_rva: replay_core::A729_TEXT_SETTER_RVA,
            setter_caller_rva: Some(0x01c4_baa0),
            formatter_entry_rva: Some(replay_core::A729_FORMATTER_RVA),
            identity_source: RuntimeTextIdentitySource::ProvenSetterCaller,
            final_text_sha256: Sha256::hex(text.as_bytes()),
        });
        runtime.provenance.runtime_text_bindings.push(PowerBalanceRuntimeTextBinding { slot, context_id });
    }
}

#[test]
fn committed_fixture_preserves_all_ita_balance_ranges_and_dynamic_side_choices() {
    let catalog = committed_ita_catalog();
    assert_eq!(catalog.len(), 1);
    let ita = catalog.get("ITA_power_balance").unwrap();
    assert_eq!(ita.sides.len(), 9);
    assert_eq!(ita.neutral_ranges.len(), 1);
    assert_eq!(
        ita.side("ITA_mussolini_side").unwrap().authored_icon,
        "GFX_bop_ITA_mussolini_side"
    );
    assert!(ita.side("ITA_balbo_side").is_some());
    assert_eq!(
        ita.range("ITA_grand_council_medium_control_range")
            .unwrap()
            .modifier
            .get("war_support_factor")
            .and_then(|value| value.as_str()),
        Some("-0.05")
    );
}

#[test]
fn exact_live_static_runtime_join_preserves_geometry_decisions_and_modifier_order() {
    let catalog = committed_ita_catalog();
    let view = view(&["ITA"], -0.35);
    let decisions = decisions();
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &view,
        catalog: &catalog,
        decisions: &decisions,
        runtime: &[runtime()],
    })
    .unwrap();
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("active measured balance must be visible")
    };
    let row = &rows[0];
    assert_eq!(row.normalized_value, 0.325);
    assert_eq!(row.marker_center_x, 212.0);
    assert_eq!(row.marker_top_left, [200.0, 119.0]);
    assert_eq!(row.bar_sprite, "GFX_power_balance_left_moving");
    assert_eq!(row.active_range_modifier["war_support_factor"], "-0.05");
    assert_eq!(
        row.decisions
            .iter()
            .map(|decision| decision.card.decision_id.as_str())
            .collect::<Vec<_>>(),
        ["ITA_bop_decision_b", "ITA_bop_decision_a"]
    );
    assert_eq!(
        row.range_boundaries
            .iter()
            .map(|boundary| boundary.value)
            .collect::<Vec<_>>(),
        [-0.9, -0.6, -0.3, -0.1, 0.1, 0.3, 0.6, 0.9]
    );

    let mut options = ComposeOptions::new(1920, 1080);
    apply_power_balance_to_compose_options(row, &mut options);
    assert_eq!(
        options.sprite_name_overrides["power_balance_value"],
        "GFX_power_balance_left_moving"
    );
    assert_eq!(options.progress_overrides["power_balance_value"], 0.325);
    assert_eq!(options.frame_overrides["range_indicator"], 0);
    assert_eq!(options.position_overrides["position_marker"], [200, 119]);
    assert_eq!(options.position_overrides["range_indicator"], [198, 101]);
    assert_eq!(
        options.text_overrides["active_range_name"],
        "Medium Council Control"
    );

    let mut launcher = ComposeOptions::new(1920, 1080);
    apply_power_balance_launcher_to_compose_options(row, &mut launcher);
    assert_eq!(launcher.text_overrides["power_balance_percentage"], "32%");
    assert_eq!(launcher.text_overrides["power_balance_levels"], "3/5");
    assert_eq!(
        launcher.sprite_name_overrides["power_balance_ICON"],
        "GFX_bop_ITA_grand_council_side"
    );
    assert!(!launcher.hidden_elements.contains("show_power_balance_glow"));
}

#[test]
fn base_feature_visibility_is_active_balance_membership_not_dlc_entitlement() {
    let catalog = committed_ita_catalog();
    let view = view(&[], -0.35);
    let decision_defs = decisions();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[],
        }),
        Ok(AdmittedPowerBalanceScreen::HiddenNoActiveBalance)
    );
}

#[test]
fn portable_asset_join_keeps_every_a37_role_and_both_progress_textures() {
    let catalog = committed_ita_catalog();
    let live = view(&["ITA"], -0.35);
    let decision_defs = decisions();
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &live,
        catalog: &catalog,
        decisions: &decision_defs,
        runtime: &[runtime()],
    })
    .unwrap();
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("active measured balance must be visible")
    };
    let assets = PowerBalanceAssetCatalog::from_static_manifest_json(
        &portable_asset_manifest(),
    )
    .unwrap()
    .resolve(&rows[0])
    .unwrap();

    assert_eq!(assets.bindings.len(), 17);
    assert!(assets.bindings.iter().all(|binding| {
        binding.gui_source_layer == "fixture_mod"
            && binding.gui_source_layer_kind == "mod"
            && binding.gui_source_sha256.len() == 64
            && binding.gui_source_bytes > 0
    }));
    let value = assets
        .bindings
        .iter()
        .filter(|binding| binding.role == PowerBalanceAssetRole::BalanceValue)
        .collect::<Vec<_>>();
    assert_eq!(value.len(), 2);
    assert_eq!(value[0].texture_slot, "textureFile1");
    assert_eq!(value[1].texture_slot, "textureFile2");
    assert!(value.iter().all(|binding| {
        binding.sprite_name == "GFX_power_balance_left_moving"
            && binding.drawable_kind == "progressBarType"
            && binding.selected_frame == Some(0)
            && binding.texture_path.starts_with("gfx/interface/")
            && binding.texture_sha256.len() == 64
    }));

    let indicator = assets
        .bindings
        .iter()
        .find(|binding| binding.role == PowerBalanceAssetRole::RangeIndicator)
        .unwrap();
    assert_eq!(indicator.selected_frame, Some(0));
    let glow = assets
        .bindings
        .iter()
        .find(|binding| binding.role == PowerBalanceAssetRole::LauncherGlow)
        .unwrap();
    assert_eq!(glow.selected_frame, None);
    assert_eq!(glow.drawable_kind, "frameAnimatedSpriteType");

    let left = assets
        .bindings
        .iter()
        .find(|binding| binding.role == PowerBalanceAssetRole::LeftSideIcon)
        .unwrap();
    let launcher = assets
        .bindings
        .iter()
        .find(|binding| binding.role == PowerBalanceAssetRole::LauncherIcon)
        .unwrap();
    assert_eq!(left.texture_sha256, launcher.texture_sha256);
    assert_ne!(left.role, launcher.role);
}

#[test]
fn portable_asset_join_accepts_authenticated_mod_sprite_overrides_from_gui_roles() {
    let mut manifest: serde_json::Value = serde_json::from_str(&portable_asset_manifest()).unwrap();
    let mut modded_sprite = manifest["sprites"]["GFX_tiled_plain_bg"].clone();
    modded_sprite["name"] = serde_json::json!("GFX_total_conversion_bop_panel");
    modded_sprite["source_layer"] = serde_json::json!("fixture_mod");
    modded_sprite["source_layer_kind"] = serde_json::json!("mod");
    manifest["sprites"]["GFX_total_conversion_bop_panel"] = modded_sprite;
    let role = &mut manifest["power_balance_gui"]["roles"]["panel_background"];
    role["sprite_name"] = serde_json::json!("GFX_total_conversion_bop_panel");
    role["authored_sprite_fields"][0]["sprite_name"] =
        serde_json::json!("GFX_total_conversion_bop_panel");
    manifest.as_object_mut().unwrap().remove("artifact_sha256");
    let digest = Sha256::hex(&serde_json::to_vec(&manifest).unwrap());
    manifest["artifact_sha256"] = serde_json::Value::String(digest);

    let definition_catalog = committed_ita_catalog();
    let live = view(&["ITA"], -0.35);
    let decision_defs = decisions();
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &live,
        catalog: &definition_catalog,
        decisions: &decision_defs,
        runtime: &[runtime()],
    })
    .unwrap();
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("active measured balance must be visible")
    };
    let assets = PowerBalanceAssetCatalog::from_static_manifest_json(
        &serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap()
    .resolve(&rows[0])
    .unwrap();
    assert_eq!(
        assets
            .bindings
            .iter()
            .find(|binding| binding.role == PowerBalanceAssetRole::PanelBackground)
            .unwrap()
            .sprite_name,
        "GFX_total_conversion_bop_panel"
    );
}

#[test]
fn portable_asset_join_refuses_manifest_tampering_and_incomplete_progress_roles() {
    let mut tampered: serde_json::Value =
        serde_json::from_str(&portable_asset_manifest()).unwrap();
    tampered["sprites"]["GFX_bop_indicator"]["texture_slots"][0]
        ["texture_bytes"] = serde_json::json!(999);
    assert_eq!(
        PowerBalanceAssetCatalog::from_static_manifest_json(
            &serde_json::to_string(&tampered).unwrap()
        )
        .unwrap_err(),
        PowerBalanceAssetError::InvalidManifestHash
    );

    let mut incomplete: serde_json::Value =
        serde_json::from_str(&portable_asset_manifest()).unwrap();
    incomplete
        ["sprites"]
        ["GFX_power_balance_left_moving"]
        ["texture_slots"]
        .as_array_mut()
        .unwrap()
        .pop();
    incomplete.as_object_mut().unwrap().remove("artifact_sha256");
    let digest = Sha256::hex(&serde_json::to_vec(&incomplete).unwrap());
    incomplete.as_object_mut().unwrap().insert(
        "artifact_sha256".into(),
        serde_json::Value::String(digest),
    );
    let asset_catalog = PowerBalanceAssetCatalog::from_static_manifest_json(
        &serde_json::to_string(&incomplete).unwrap(),
    )
    .unwrap();
    let definition_catalog = committed_ita_catalog();
    let live = view(&["ITA"], -0.35);
    let decision_defs = decisions();
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &live,
        catalog: &definition_catalog,
        decisions: &decision_defs,
        runtime: &[runtime()],
    })
    .unwrap();
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("active measured balance must be visible")
    };
    assert_eq!(
        asset_catalog.resolve(&rows[0]),
        Err(PowerBalanceAssetError::InvalidSprite(
            "GFX_power_balance_left_moving".into()
        ))
    );

    let mut missing_role: serde_json::Value =
        serde_json::from_str(&portable_asset_manifest()).unwrap();
    missing_role["power_balance_gui"]["roles"]
        .as_object_mut()
        .unwrap()
        .remove("panel_background");
    missing_role.as_object_mut().unwrap().remove("artifact_sha256");
    let digest = Sha256::hex(&serde_json::to_vec(&missing_role).unwrap());
    missing_role["artifact_sha256"] = serde_json::Value::String(digest);
    assert_eq!(
        PowerBalanceAssetCatalog::from_static_manifest_json(
            &serde_json::to_string(&missing_role).unwrap()
        )
        .unwrap_err(),
        PowerBalanceAssetError::InvalidGuiManifest
    );

    let mut case_folded: serde_json::Value =
        serde_json::from_str(&portable_asset_manifest()).unwrap();
    case_folded["power_balance_gui"]["roles"]["panel_background"]
        ["hierarchy_path"] = serde_json::json!("powerbalanceview/Background");
    case_folded["power_balance_gui"]["roles"]["panel_background"]
        ["widget_name"] = serde_json::json!("Background");
    case_folded.as_object_mut().unwrap().remove("artifact_sha256");
    let digest = Sha256::hex(&serde_json::to_vec(&case_folded).unwrap());
    case_folded["artifact_sha256"] = serde_json::Value::String(digest);
    assert_eq!(
        PowerBalanceAssetCatalog::from_static_manifest_json(
            &serde_json::to_string(&case_folded).unwrap()
        )
        .unwrap_err(),
        PowerBalanceAssetError::InvalidGuiManifest
    );
}

#[test]
fn all_irreducible_runtime_inputs_fail_closed() {
    let catalog = committed_ita_catalog();
    let view = view(&["ITA"], -0.35);
    let decision_defs = decisions();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[],
        }),
        Err(PowerBalanceRejection::RuntimeSetMismatch)
    );

    let mut invalid_provenance = runtime();
    invalid_provenance
        .provenance
        .completeness_record_identity
        .clear();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_provenance],
        }),
        Err(PowerBalanceRejection::MissingRuntimeProvenance)
    );

    let mut mismatched_trending_side = runtime();
    mismatched_trending_side.trending_side_id = "ITA_mussolini_side".into();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[mismatched_trending_side],
        }),
        Err(PowerBalanceRejection::InvalidTrend)
    );

    let mut invalid_active_side = runtime();
    invalid_active_side.active_side_id = "ITA_mussolini_side".into();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_active_side],
        }),
        Err(PowerBalanceRejection::InvalidActiveSide)
    );

    let mut invalid_stable_bar = runtime();
    invalid_stable_bar.trend = None;
    invalid_stable_bar.bar_state = BarState::StableRight;
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_stable_bar],
        }),
        Err(PowerBalanceRejection::InvalidBarState)
    );

    let mut invalid_range = runtime();
    invalid_range.active_range_id = "ITA_missing_range".into();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_range],
        }),
        Err(PowerBalanceRejection::InvalidActiveRange)
    );

    let mut invalid_icon = runtime();
    invalid_icon.effective_left_icon.clear();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_icon],
        }),
        Err(PowerBalanceRejection::InvalidRuntimeIcon)
    );

    let mut invalid_trend = runtime();
    invalid_trend.trend.as_mut().unwrap().magnitude = 0.0;
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_trend],
        }),
        Err(PowerBalanceRejection::InvalidTrend)
    );

    let mut invalid_launcher = runtime();
    invalid_launcher.launcher_percentage_text = Some(String::new());
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_launcher],
        }),
        Err(PowerBalanceRejection::InvalidLauncher)
    );

    let mut unknown_decision = runtime();
    unknown_decision.decisions[0].availability = DecisionAvailability::Unknown;
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[unknown_decision],
        }),
        Err(PowerBalanceRejection::InvalidDecision)
    );

    let mut invalid_text = runtime();
    invalid_text.resolved_text.balance_tooltip.clear();
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[invalid_text],
        }),
        Err(PowerBalanceRejection::InvalidResolvedText)
    );

    let unavailable = PowerBalanceView {
        status: ViewStatus::DomainUnavailable,
        balances: Vec::new(),
    };
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &unavailable,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[],
        }),
        Err(PowerBalanceRejection::DomainUnavailable)
    );
}

#[test]
fn runtime_text_field_bindings_are_exact_and_hash_committed() {
    let catalog = committed_ita_catalog();
    let view = view(&["ITA"], -0.35);
    let decision_defs = decisions();
    let reject = |runtime| {
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[runtime],
        })
    };

    let mut missing = runtime();
    missing.provenance.runtime_text_bindings.pop();
    assert_eq!(
        reject(missing),
        Err(PowerBalanceRejection::InvalidRuntimeTextBinding)
    );

    let mut duplicate = runtime();
    let duplicate_binding = duplicate.provenance.runtime_text_bindings[0].clone();
    duplicate
        .provenance
        .runtime_text_bindings
        .push(duplicate_binding);
    assert_eq!(
        reject(duplicate),
        Err(PowerBalanceRejection::InvalidRuntimeTextBinding)
    );

    let mut changed_text = runtime();
    changed_text.resolved_text.balance_name.push('!');
    assert_eq!(
        reject(changed_text),
        Err(PowerBalanceRejection::InvalidRuntimeTextBinding)
    );

    let mut orphan = runtime();
    let mut extra = orphan.provenance.runtime_text_contexts[0].clone();
    extra.context_id.push_str(":orphan");
    orphan.provenance.runtime_text_contexts.push(extra);
    assert_eq!(
        reject(orphan),
        Err(PowerBalanceRejection::InvalidRuntimeTextBinding)
    );
}

#[test]
fn active_side_moving_direction_and_trending_identity_remain_independent() {
    let catalog = committed_ita_catalog();
    let decision_defs = decisions();

    let right_active = PowerBalanceView {
        status: ViewStatus::Present,
        balances: vec![live_row_trending(
            &["ITA"],
            0.35,
            "ITA_mussolini_side",
        )],
    };
    let mut moving_left_with_right_trend_identity = runtime();
    moving_left_with_right_trend_identity.active_side_id = "ITA_mussolini_side".into();
    moving_left_with_right_trend_identity.active_range_id =
        "ITA_mussolini_medium_control_range".into();
    moving_left_with_right_trend_identity.trending_side_id = "ITA_mussolini_side".into();
    moving_left_with_right_trend_identity.bar_state = BarState::MovingLeft;
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &right_active,
        catalog: &catalog,
        decisions: &decision_defs,
        runtime: &[moving_left_with_right_trend_identity],
    })
    .expect("right-active balance may move left with an independent right trend identity");
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("selected balance must remain visible")
    };
    assert_eq!(rows[0].active_side_id, "ITA_mussolini_side");
    assert_eq!(rows[0].trending_side_id, "ITA_mussolini_side");
    assert_eq!(rows[0].bar_sprite, "GFX_power_balance_left_moving");

    let left_active = PowerBalanceView {
        status: ViewStatus::Present,
        balances: vec![live_row_trending(&["ITA"], -0.35, "ITA_grand_council_side")],
    };
    let mut moving_right_with_left_trend_identity = runtime();
    moving_right_with_left_trend_identity.trending_side_id = "ITA_grand_council_side".into();
    moving_right_with_left_trend_identity.bar_state = BarState::MovingRight;
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &left_active,
        catalog: &catalog,
        decisions: &decision_defs,
        runtime: &[moving_right_with_left_trend_identity],
    })
    .expect("left-active balance may move right with an independent left trend identity");
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("selected balance must remain visible")
    };
    assert_eq!(rows[0].active_side_id, "ITA_grand_council_side");
    assert_eq!(rows[0].trending_side_id, "ITA_grand_council_side");
    assert_eq!(rows[0].bar_sprite, "GFX_power_balance_right_moving");
}

#[test]
fn controller_selected_identity_excludes_other_applicable_balances() {
    let catalog = committed_ita_catalog();
    let decision_defs = decisions();
    let mut later = live_row(&["ITA"], -0.1);
    later.template = Leaf::Measured("ITA_unrendered_second_balance".into());
    let view = PowerBalanceView {
        status: ViewStatus::Present,
        // Save/world order is not controller-selection authority. The exact
        // runtime balance id identifies the engine's first matching record.
        balances: vec![later, live_row(&["ITA"], -0.35)],
    };
    let mut measured = runtime();
    measured.power_balance_modifier_desc.clear();
    measured.provenance.runtime_text_contexts.clear();
    measured.provenance.runtime_text_bindings.clear();
    attach_runtime_text_provenance(&mut measured);
    let admitted = admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag: "ITA",
        view: &view,
        catalog: &catalog,
        decisions: &decision_defs,
        runtime: &[measured],
    })
    .expect("only the exact controller-selected engine row is presented");
    let AdmittedPowerBalanceScreen::Visible(rows) = admitted else {
        panic!("first matching row must be visible")
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].balance_id, "ITA_power_balance");
    assert_eq!(
        rows[0].provenance.completeness_record_identity,
        "d74-complete:42"
    );
    assert!(rows[0]
        .provenance
        .runtime_text_bindings
        .iter()
        .any(|binding| binding.slot == PowerBalanceRuntimeTextSlot::ModifierDescription));
    assert!(rows[0].power_balance_modifier_desc.is_empty());
}

#[test]
fn supplied_active_range_disambiguates_shared_threshold_boundaries() {
    let catalog = committed_ita_catalog();
    let view = view(&["ITA"], -0.3);
    let decision_defs = decisions();
    for range in [
        "ITA_grand_council_low_control_range",
        "ITA_grand_council_medium_control_range",
    ] {
        let mut runtime = runtime();
        runtime.active_range_id = range.into();
        assert!(admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[runtime],
        })
        .is_ok());
    }
}

#[test]
fn progress_minmax_shader_interval_and_panel_animation_are_exact() {
    assert!(!progress_minmax_uses_front(0.2, 0.35));
    assert!(progress_minmax_uses_front(0.4, 0.35));
    assert!(progress_minmax_uses_front(0.5, 0.65));
    assert!(progress_minmax_uses_front(0.6, 0.65));
    assert!(!progress_minmax_uses_front(0.8, 0.65));

    let show_start = sample_panel_transition(PanelTransition::Show, 0.0, 0.0).unwrap();
    assert_eq!(show_start.position, PANEL_HIDDEN_POSITION);
    let show_end = sample_panel_transition(PanelTransition::Show, 0.0, 301.0).unwrap();
    assert_eq!(show_end.position, PANEL_SHOWN_POSITION);
    assert!(show_end.complete);

    let hide_start = sample_panel_transition(PanelTransition::Hide, 0.0, 0.0).unwrap();
    assert_eq!(hide_start.position, PANEL_SHOWN_POSITION);
    assert!(!hide_start.perform_deferred_hide);
    let hide_end = sample_panel_transition(PanelTransition::Hide, 0.0, 301.0).unwrap();
    assert_eq!(hide_end.position, PANEL_HIDDEN_POSITION);
    assert!(hide_end.perform_deferred_hide);
}
