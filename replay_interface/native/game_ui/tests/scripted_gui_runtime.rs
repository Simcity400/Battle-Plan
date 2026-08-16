use game_ui::assets::AssetResolver;
use game_ui::scripted_gui_runtime::{
    apply_effect_preview, captured_inputs_for, evaluate_registered_gui, load_scripted_gui_registry,
    EffectApplyError, EvaluatedProperty, ScriptedGuiRuntimeState, Truth,
};
use replay_core::{EngineMaterialized, Leaf, ScriptedGuiRuntimeInputs, ScriptedGuiView, ViewStatus};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn temp_root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hoi4_xui22_{label}_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    if path.exists() {
        portable_test_fs_retry::remove_dir_all(&path).unwrap();
    }
    fs::create_dir_all(path.join("common/scripted_guis")).unwrap();
    path
}

fn view() -> ScriptedGuiView {
    let captured = ScriptedGuiRuntimeInputs {
        manager_order_index: Leaf::Absent,
        definition: Leaf::Absent,
        // Conflicts with the locally decidable visible block below. The exact
        // captured engine result is authoritative.
        visible: EngineMaterialized {
            value: Leaf::Measured(false),
            unavailable_reason: None,
        },
        enabled: EngineMaterialized {
            value: Leaf::Measured(true),
            unavailable_reason: None,
        },
        triggers: BTreeMap::from([
            ("score_visible".to_string(), EngineMaterialized {
                value: Leaf::Measured(false), unavailable_reason: None,
            }),
            ("increment_click_enabled".to_string(), EngineMaterialized {
                value: Leaf::Measured(true), unavailable_reason: None,
            }),
            ("disabled_click_enabled".to_string(), EngineMaterialized {
                value: Leaf::Measured(false), unavailable_reason: None,
            }),
            ("engine_visible".to_string(), EngineMaterialized {
                value: Leaf::Measured(true), unavailable_reason: None,
            }),
            ("engine_gate_click_enabled".to_string(), EngineMaterialized {
                value: Leaf::Measured(true), unavailable_reason: None,
            }),
        ]),
        properties: BTreeMap::from([
            (
                "portrait.image".to_string(),
                EngineMaterialized {
                    value: Leaf::Measured("GFX_exact_engine_portrait".to_string()),
                    unavailable_reason: None,
                },
            ),
            (
                "meter.frame".to_string(),
                EngineMaterialized {
                    value: Leaf::Measured("99".to_string()),
                    unavailable_reason: None,
                },
            ),
        ]),
        effect_enabled: BTreeMap::from([
            ("increment_click".to_string(), EngineMaterialized {
                value: Leaf::Measured(true), unavailable_reason: None,
            }),
            ("engine_only_click".to_string(), EngineMaterialized {
                value: Leaf::Measured(true), unavailable_reason: None,
            }),
            ("engine_gate_click".to_string(), EngineMaterialized {
                value: Leaf::Measured(true), unavailable_reason: None,
            }),
            ("disabled_click".to_string(), EngineMaterialized {
                value: Leaf::Measured(false), unavailable_reason: None,
            }),
        ]),
    };
    ScriptedGuiView {
        status: ViewStatus::Present,
        by_country: BTreeMap::from([(
            "TST".to_string(),
            Leaf::Measured(BTreeMap::from([("score".to_string(), 3.0)])),
        )]),
        by_country_raw_fixed5: BTreeMap::from([(
            "TST".to_string(),
            Leaf::Measured(BTreeMap::from([("score".to_string(), 300_000)])),
        )]),
        country_flags: BTreeMap::from([(
            "TST".to_string(),
            Leaf::Measured(vec!["ready".to_string()]),
        )]),
        country_flag_states: BTreeMap::new(),
        country_token_flag_states: BTreeMap::new(),
        by_state: BTreeMap::new(),
        by_state_raw_fixed5: BTreeMap::new(),
        state_flags: BTreeMap::new(),
        state_flag_states: BTreeMap::new(),
        state_token_flag_states: BTreeMap::new(),
        runtime_by_country: BTreeMap::from([(
            "TST".to_string(),
            Leaf::Measured(BTreeMap::from([("mod_panel".to_string(), captured)])),
        )]),
        runtime_authority: Leaf::Absent,
        world_variables: Leaf::Measured(BTreeMap::from([("phase".to_string(), 2.0)])),
        world_variables_raw_fixed5: Leaf::Measured(BTreeMap::from([(
            "phase".to_string(),
            200_000,
        )])),
        world_flags: Leaf::Measured(vec!["campaign_started".to_string()]),
        world_flag_states: Leaf::Absent,
        world_token_flag_states: Leaf::Absent,
    }
}

#[test]
fn effective_vfs_generic_evaluator_and_effect_preview_are_fail_closed() {
    let vanilla = temp_root("vanilla");
    let overlay = temp_root("overlay");
    fs::write(
        vanilla.join("common/scripted_guis/panel.txt"),
        r#"
scripted_gui = {
  mod_panel = {
    context_type = player_context
    window_name = vanilla_window
  }
}
"#,
    )
    .unwrap();
    fs::write(
        overlay.join("common/scripted_guis/panel.txt"),
        r#"
scripted_gui = {
  mod_panel = {
    context_type = player_context
    window_name = mod_window
    visible = {
      check_variable = { var = score value = 2 compare = greater_than_or_equals }
      has_country_flag = ready
    }
    enabled = { has_idea = engine_only_idea }
    triggers = {
      score_visible = { check_variable = { score > 2 } }
      increment_click_enabled = { check_variable = { score > 2 } }
      disabled_click_enabled = { check_variable = { score > 100 } }
      engine_visible = { has_idea = engine_only_idea }
      engine_gate_click_enabled = { has_idea = engine_only_idea }
    }
    effects = {
      increment_click = {
        add_to_variable = { var = score value = 2 }
        set_global_flag = changed
      }
      engine_only_click = { add_political_power = 5 }
      engine_gate_click = { add_to_variable = { var = score value = 1 } }
      disabled_click = { add_to_variable = { var = score value = 1000 } }
    }
    properties = {
      meter = { frame = "[?score?]" }
      portrait = { image = "[GET_EXACT_PORTRAIT]" }
      mixed = { text = "Score: [?score?]" }
    }
  }
}
"#,
    )
    .unwrap();

    // Higher-priority overlay root wins the same relative file, matching the
    // product AssetResolver path used by UI14/UI15.
    let registry =
        load_scripted_gui_registry(&AssetResolver::new(vec![overlay.clone(), vanilla.clone()]))
            .unwrap();
    portable_test_fs_retry::remove_dir_all(&overlay).unwrap();
    portable_test_fs_retry::remove_dir_all(&vanilla).unwrap();
    assert!(
        registry.production_admitted(),
        "{:?}",
        registry.source_issues
    );
    assert_eq!(registry.files, 1);
    let definition = &registry.definitions["mod_panel"];
    assert_eq!(definition.window_name.as_deref(), Some("mod_window"));

    let replay_view = view();
    let state = ScriptedGuiRuntimeState::from_view(&replay_view, "TST");
    let captured = captured_inputs_for(&replay_view, "TST", "mod_panel");
    let evaluated = evaluate_registered_gui(&registry, &replay_view, "TST", "mod_panel").unwrap();
    assert_eq!(evaluated.visible, Truth::False);
    assert_eq!(evaluated.enabled, Truth::True);
    assert_eq!(evaluated.triggers["score_visible"], Truth::False);
    assert_eq!(evaluated.triggers["engine_visible"], Truth::True);
    assert_eq!(evaluated.effect_enabled["increment_click"], Truth::True);
    assert_eq!(evaluated.effect_enabled["engine_gate_click"], Truth::True);
    assert_eq!(
        evaluated.properties["meter.frame"],
        EvaluatedProperty::String {
            value: "99".to_string(),
            captured: true,
        }
    );
    assert_eq!(
        evaluated.properties["portrait.image"],
        EvaluatedProperty::String {
            value: "GFX_exact_engine_portrait".to_string(),
            captured: true,
        }
    );
    assert_eq!(
        evaluated.properties["mixed.text"],
        EvaluatedProperty::Unavailable
    );

    let next = apply_effect_preview(definition, "increment_click", &state, captured).unwrap();
    assert_eq!(next.country_variables.unwrap()["score"], 5.0);
    assert!(next.world_flags.unwrap().contains("changed"));
    assert!(matches!(
        apply_effect_preview(definition, "engine_only_click", &state, captured),
        Err(EffectApplyError::EngineEvaluationRequired(_))
    ));
    assert!(matches!(
        apply_effect_preview(definition, "disabled_click", &state, captured),
        Err(EffectApplyError::Disabled)
    ));
    assert_eq!(state.country_variables.as_ref().unwrap()["score"], 3.0);

    // Exact captured engine values win over conflicting replay-local values.
    assert_eq!(
        evaluated.properties["meter.frame"],
        EvaluatedProperty::String {
            value: "99".to_string(),
            captured: true,
        }
    );

    // Removing local state cannot change an exact captured engine result.
    let mut missing_variable_view = view();
    missing_variable_view
        .by_country
        .insert("TST".to_string(), Leaf::Measured(BTreeMap::new()));
    let fallback =
        evaluate_registered_gui(&registry, &missing_variable_view, "TST", "mod_panel").unwrap();
    assert_eq!(
        fallback.properties["meter.frame"],
        EvaluatedProperty::String {
            value: "99".to_string(),
            captured: true,
        }
    );

    // Explicit engine unavailability remains unavailable even though the
    // replay-local authored predicate and variable expression are decidable.
    let mut unavailable_view = view();
    let Some(Leaf::Measured(guis)) = unavailable_view.runtime_by_country.get_mut("TST") else {
        panic!("captured GUI map missing")
    };
    let unavailable_inputs = guis.get_mut("mod_panel").unwrap();
    unavailable_inputs.visible.value = Leaf::Unobtained;
    unavailable_inputs
        .properties
        .get_mut("meter.frame")
        .unwrap()
        .value = Leaf::Unobtained;
    unavailable_inputs
        .triggers
        .get_mut("score_visible")
        .unwrap()
        .value = Leaf::Unobtained;
    unavailable_inputs
        .effect_enabled
        .get_mut("increment_click")
        .unwrap()
        .value = Leaf::Unobtained;
    let unavailable =
        evaluate_registered_gui(&registry, &unavailable_view, "TST", "mod_panel").unwrap();
    assert_eq!(unavailable.visible, Truth::Unavailable);
    assert_eq!(
        unavailable.properties["meter.frame"],
        EvaluatedProperty::Unavailable
    );
    assert_eq!(unavailable.triggers["score_visible"], Truth::Unavailable);
    assert_eq!(
        unavailable.effect_enabled["increment_click"],
        Truth::Unavailable
    );

    // Definition-level enabled participates in every effect gate. A captured
    // false therefore prevents even a locally supported variable preview.
    let mut disabled_view = view();
    let Some(Leaf::Measured(guis)) = disabled_view.runtime_by_country.get_mut("TST") else {
        panic!("captured GUI map missing")
    };
    guis.get_mut("mod_panel").unwrap().enabled.value = Leaf::Measured(false);
    let disabled_state = ScriptedGuiRuntimeState::from_view(&disabled_view, "TST");
    let disabled_capture = captured_inputs_for(&disabled_view, "TST", "mod_panel");
    let disabled = evaluate_registered_gui(&registry, &disabled_view, "TST", "mod_panel").unwrap();
    assert_eq!(disabled.enabled, Truth::False);
    assert_eq!(disabled.effect_enabled["increment_click"], Truth::False);
    assert!(matches!(
        apply_effect_preview(
            definition,
            "increment_click",
            &disabled_state,
            disabled_capture,
        ),
        Err(EffectApplyError::Disabled)
    ));
}

#[test]
fn dynamic_lists_require_an_exact_ordered_runtime_carrier() {
    let root = temp_root("dynamic_list_refusal");
    fs::write(
        root.join("common/scripted_guis/list.txt"),
        r#"
scripted_gui = {
  list_panel = {
    context_type = decision_category
    window_name = list_window
    dynamic_lists = {
      rows = { array = runtime_rows value = row_value index = row_index }
    }
  }
}
"#,
    )
    .unwrap();
    let registry = load_scripted_gui_registry(&AssetResolver::new(vec![root.clone()])).unwrap();
    portable_test_fs_retry::remove_dir_all(&root).unwrap();
    assert!(!registry.production_admitted());
    assert!(registry
        .source_issues
        .iter()
        .any(|issue| issue.contains("authenticated ordered array/value/scope carrier")));
}

#[test]
fn only_one_outer_eof_closure_is_engine_tolerated() {
    let root = temp_root("outer_eof");
    fs::write(
        root.join("common/scripted_guis/outer_eof.txt"),
        "scripted_gui = { exact = { context_type = decision_category window_name = exact }",
    )
    .unwrap();
    let registry = load_scripted_gui_registry(&AssetResolver::new(vec![root.clone()])).unwrap();
    assert!(registry.production_admitted(), "{:?}", registry.source_issues);
    assert_eq!(registry.engine_tolerated_eof_closures.len(), 1);

    fs::write(
        root.join("common/scripted_guis/outer_eof.txt"),
        "scripted_gui = { broken = { context_type = decision_category",
    )
    .unwrap();
    let truncated = load_scripted_gui_registry(&AssetResolver::new(vec![root.clone()])).unwrap();
    portable_test_fs_retry::remove_dir_all(&root).unwrap();
    assert!(!truncated.production_admitted());
    assert!(truncated.engine_tolerated_eof_closures.is_empty());
    assert!(truncated.source_issues.len() >= 2);
}

#[test]
fn unavailable_recorded_scope_never_becomes_false_or_zero() {
    let mut view = view();
    view.by_country.insert("TST".to_string(), Leaf::Unobtained);
    let state = ScriptedGuiRuntimeState::from_view(&view, "TST");
    let predicate = game_ui::scripted_gui_runtime::Predicate::HasVariable {
        variable: game_ui::scripted_gui_runtime::VariableRef::parse("score"),
    };
    assert_eq!(predicate.evaluate(&state), Truth::Unavailable);
}

#[test]
fn malformed_named_blocks_and_dynamic_properties_fail_registry_admission() {
    let root = temp_root("malformed");
    fs::write(
        root.join("common/scripted_guis/bad.txt"),
        r#"
scripted_gui = {
  bad_panel = {
    triggers = bad
    effects = bad
    properties = { portrait = { image = { nested = value } } }
  }
}
"#,
    )
    .unwrap();
    let registry = load_scripted_gui_registry(&AssetResolver::new(vec![root.clone()])).unwrap();
    portable_test_fs_retry::remove_dir_all(&root).unwrap();
    assert!(!registry.production_admitted());
    assert_eq!(
        registry.source_issues.len(),
        3,
        "{:?}",
        registry.source_issues
    );
    assert!(registry.definitions["bad_panel"].properties.is_empty());
}
