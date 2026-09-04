use game_ui::compose::ComposeOptions;
use game_ui::loc::Localisation;
use game_ui::text_law::{resolve_localisation, RuntimeTextContext, TextIssueKind};
use replay_core::{
    RuntimeTextIdentitySource, RuntimeTextLayout, RuntimeTextProducer, RuntimeTextRecord,
    RuntimeTextSource, A729_FORMATTER_RVA, A729_TEXT_SETTER_RVA,
};
use std::collections::BTreeMap;

#[test]
fn exact_formatted_carriers_are_required_and_admitted() {
    let loc = BTreeMap::from([(
        "ROW".to_string(),
        "§G$COUNT|+0$§! / [GetDuration]".to_string(),
    )]);
    let absent = resolve_localisation(&loc, "ROW", &RuntimeTextContext::default());
    assert!(!absent.admitted());
    assert!(absent.issues.iter().any(|issue| {
        issue.kind == TextIssueKind::FormattedVariableRequiresRuntimeValue
            && issue.token == "COUNT|+0"
    }));
    assert!(absent.issues.iter().any(|issue| {
        issue.kind == TextIssueKind::MissingDataFunction && issue.token == "GetDuration"
    }));

    let runtime = RuntimeTextContext {
        variables: BTreeMap::from([("COUNT|+0".to_string(), "+3".to_string())]),
        data_functions: BTreeMap::from([("GetDuration".to_string(), "12 days".to_string())]),
        ..RuntimeTextContext::default()
    };
    let admitted = resolve_localisation(&loc, "ROW", &runtime);
    assert!(admitted.admitted(), "{:?}", admitted.issues);
    assert_eq!(admitted.text, "§G+3§! / 12 days");
}

#[test]
fn localisation_container_can_supply_runtime_context_to_compositor() {
    let mut localisation = Localisation::default();
    localisation
        .entries
        .insert("VALUE".to_string(), "$VAL|%0$".to_string());
    let options = ComposeOptions::new(64, 32).with_localisation(&localisation);
    assert!(options.runtime_text.variables.is_empty());
}

#[test]
fn exact_a25_setter_result_is_identity_guarded_and_normalized() {
    let record = RuntimeTextRecord {
        context_id: "rt:losses".into(),
        frame_hour: 40,
        pov_country_or_global: "FRA".into(),
        view_id: "war_overview".into(),
        element_id: "attacker_losses".into(),
        source: RuntimeTextSource::LocalisationKey("WAR_OVERVIEW_LOSSES".into()),
        final_text_bytes: b"Losses: \x11R17.62M\x11!".to_vec(),
        final_text_utf8: "Losses: \u{11}R17.62M\u{11}!".into(),
        final_text_sha256: "fixture-construction-bypasses-wire-admission".into(),
        variables: BTreeMap::new(),
        data_functions: BTreeMap::new(),
        dynamic_colors: BTreeMap::new(),
        layout: RuntimeTextLayout::AuthoredReproducible,
        producer: RuntimeTextProducer {
            setter_entry_rva: A729_TEXT_SETTER_RVA,
            setter_caller_rva: Some(0x0189_A000),
            formatter_entry_rva: Some(A729_FORMATTER_RVA),
            identity_source: RuntimeTextIdentitySource::AuthoredWidgetRegistration,
        },
    };
    let runtime = RuntimeTextContext::from(&record);
    let loc = BTreeMap::from([(
        "WAR_OVERVIEW_LOSSES".to_string(),
        "Losses: $VALUE$".to_string(),
    )]);
    let exact = resolve_localisation(&loc, "WAR_OVERVIEW_LOSSES", &runtime);
    assert!(exact.admitted(), "{:?}", exact.issues);
    assert_eq!(exact.text, "Losses: §R17.62M§!");
    assert!(runtime.exact_for_element(Some("war_overview"), "attacker_losses"));
    assert!(!runtime.exact_for_element(Some("war_overview"), "defender_losses"));

    let mismatch = resolve_localisation(&loc, "OTHER_KEY", &runtime);
    assert_eq!(
        mismatch.issues[0].kind,
        TextIssueKind::RuntimeTextIdentityMismatch
    );
}
