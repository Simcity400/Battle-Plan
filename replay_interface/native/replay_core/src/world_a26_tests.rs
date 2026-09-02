use super::*;

fn selected() -> portable_render_stream::SelectedPresentReceipt {
    portable_render_stream::SelectedPresentReceipt::sealed(
        1,
        "a".repeat(64),
        7,
        0,
        43_800_899,
        100,
        100,
    )
}

fn exact_root(selected: &portable_render_stream::SelectedPresentReceipt) -> Value {
    serde_json::json!({
        "schema_version": 2,
        "authority": "exact_a729_tooltip_controller",
        "elapsed_wall_ms": 3,
        "status_flags": 0,
        "frame_hour": 900,
        "selected_present_candidate_id": 7,
        "selected_present_ordinal": 0,
        "selected_present_qpc_ticks": 100,
        "selected_present_receipt_sha256": selected.computed_sha256().unwrap(),
        "producer_manifest_sha256": "a23b63ccefc021eeced9a1a918588d3e2fbba07e5d1afb4d53f83cc7c1640177",
        "completeness_record_identity": "a26:selected-present=7:observation=3",
        "expected_instance_count": 1,
        "instances": [{
            "frame_hour": 900,
            "pov_country_or_global": "GER",
            "hovered_element_authored_name": "technology_slot",
            "tooltip_owner_authored_name": "ToolTip",
            "repeat_index": 0,
            "screen_rect": {"x": 1600, "y": 900, "width": 300, "height": 120},
            "viewport_width_px": 1920,
            "viewport_height_px": 1080,
            "producer_record_identity": "a26:observation=3:qpc=90",
            "final_resolved_text": "Exact first line\nExact second line"
        }]
    })
}

#[test]
fn tti2_preserves_final_blob_and_measured_empty() {
    let selected = selected();
    let mut rest = Map::new();
    rest.insert(
        "_r2_resolved_tooltip_instances_live".into(),
        exact_root(&selected),
    );
    let mut warnings = Vec::new();
    let view = build_resolved_tooltip_exact(
        &rest,
        900,
        Some(&selected),
        &BTreeSet::<String>::new(),
        &mut warnings,
    );
    assert_eq!(view.status, ViewStatus::Present);
    assert_eq!(
        view.frame.as_ref().unwrap().instances[0].final_resolved_text,
        "Exact first line\nExact second line"
    );

    rest["_r2_resolved_tooltip_instances_live"]["expected_instance_count"] = serde_json::json!(0);
    rest["_r2_resolved_tooltip_instances_live"]["instances"] = serde_json::json!([]);
    let view = build_resolved_tooltip_exact(
        &rest,
        900,
        Some(&selected),
        &BTreeSet::<String>::new(),
        &mut warnings,
    );
    assert_eq!(view.status, ViewStatus::Present);
    assert!(view.frame.unwrap().instances.is_empty());
}

#[test]
fn tti2_rejects_torn_selected_present_join() {
    let selected = selected();
    let mut root = exact_root(&selected);
    root["selected_present_candidate_id"] = serde_json::json!(8);
    let mut rest = Map::new();
    rest.insert("_r2_resolved_tooltip_instances_live".into(), root);
    let mut warnings = Vec::new();
    let view = build_resolved_tooltip_exact(
        &rest,
        900,
        Some(&selected),
        &BTreeSet::<String>::new(),
        &mut warnings,
    );
    assert_eq!(view.status, ViewStatus::DomainUnavailable);
}
