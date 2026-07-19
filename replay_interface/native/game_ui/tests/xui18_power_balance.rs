use game_ui::compose::{progress_minmax_uses_front, ComposeOptions};
use game_ui::decision_chrome::{DecisionAvailability, DecisionDef, DecisionSet};
use game_ui::power_balance::{
    admit_power_balance_screen, apply_power_balance_launcher_to_compose_options,
    apply_power_balance_to_compose_options, sample_panel_transition, ActiveModifierInput,
    AdmittedPowerBalanceScreen, BarState, ChangeCadence, PanelTransition,
    PowerBalanceAdmissionRequest, PowerBalanceDecisionInput, PowerBalanceDefinitionCatalog,
    PowerBalanceRejection, PowerBalanceRuntimeSupplement, PowerBalanceTrendInput,
    ResolvedPowerBalanceText, PANEL_HIDDEN_POSITION, PANEL_SHOWN_POSITION,
};
use replay_core::{Leaf, PowerBalanceRow, PowerBalanceView, ViewStatus};
use std::path::PathBuf;

fn frozen_catalog() -> PowerBalanceDefinitionCatalog {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extractor/game_defs/out/catalogs.json");
    let input = std::fs::read_to_string(path).expect("frozen game-def catalog");
    PowerBalanceDefinitionCatalog::from_catalogs_json(&input).expect("valid BOP catalog")
}

fn live_row(countries: &[&str], value: f64) -> PowerBalanceRow {
    PowerBalanceRow {
        template: Leaf::Measured("ITA_power_balance".into()),
        value: Leaf::Measured(value),
        left_side: Leaf::Measured("ITA_grand_council_side".into()),
        right_side: Leaf::Measured("ITA_mussolini_side".into()),
        trending_side: Leaf::Measured("ITA_grand_council_side".into()),
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
    PowerBalanceRuntimeSupplement {
        source_record_id: "d-bop:hour=42:country=ITA:id=ITA_power_balance".into(),
        country_tag: "ITA".into(),
        balance_id: "ITA_power_balance".into(),
        display_ordinal: 0,
        active_side_id: "ITA_grand_council_side".into(),
        active_range_id: "ITA_grand_council_medium_control_range".into(),
        effective_left_icon: "GFX_bop_ITA_grand_council_side".into(),
        effective_right_icon: "GFX_bop_ITA_mussolini_side".into(),
        launcher_visible: true,
        launcher_percentage_text: Some("32%".into()),
        launcher_levels_text: Some("3/5".into()),
        launcher_icon_sprite: Some("GFX_bop_ITA_grand_council_side".into()),
        launcher_glow_visible: true,
        bar_state: BarState::MovingLeft,
        range_indicator_frame: 0,
        range_indicator_top_left: [198, 101],
        trend: Some(PowerBalanceTrendInput {
            cadence: ChangeCadence::Daily,
            magnitude: 0.01,
            resolved_text: "1.00% per day".into(),
        }),
        active_modifiers_complete: true,
        active_modifiers: vec![ActiveModifierInput {
            modifier_id: "ITA_bop_lost_core".into(),
            source_record_id: "d-bop-mod:42:0".into(),
            resolved_text: "Grand Council drift: +1.00%".into(),
        }],
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
    }
}

#[test]
fn frozen_catalog_preserves_all_current_balance_ranges_and_dynamic_side_choices() {
    let catalog = frozen_catalog();
    assert_eq!(catalog.len(), 12);
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
    let catalog = frozen_catalog();
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
    let catalog = frozen_catalog();
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
fn all_irreducible_runtime_inputs_fail_closed() {
    let catalog = frozen_catalog();
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

    let mut incomplete = runtime();
    incomplete.active_modifiers_complete = false;
    assert_eq!(
        admit_power_balance_screen(PowerBalanceAdmissionRequest {
            country_tag: "ITA",
            view: &view,
            catalog: &catalog,
            decisions: &decision_defs,
            runtime: &[incomplete],
        }),
        Err(PowerBalanceRejection::IncompleteModifiers)
    );

    let mut invalid_range = runtime();
    invalid_range.active_range_id = "ITA_mussolini_low_control_range".into();
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
fn supplied_active_range_disambiguates_shared_threshold_boundaries() {
    let catalog = frozen_catalog();
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
