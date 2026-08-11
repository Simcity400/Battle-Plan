use map_viewer::{
    admit_map_authority_json, admit_map_authority_projection, map_authority_requirement_metadata,
    refuse_missing_xmap00_authority, refuse_product_map_until_xmap01,
    require_promoted_xmap01_renderer, MapAuthorityProjection, MapAuthorityRequirement,
    MapAuthorityRequirementDisposition, MAP_AUTHORITY_INPUT_SCHEMA, MAP_AUTHORITY_REQUIRED_INPUTS,
};

fn unresolved_requirement(input_id: &str) -> MapAuthorityRequirement {
    let metadata = map_authority_requirement_metadata(input_id).unwrap();
    assert_eq!(metadata.input_id, input_id);
    assert_eq!(metadata.payload_schema, None);
    assert_eq!(metadata.cadence, None);
    assert_eq!(metadata.identity_order, None);
    MapAuthorityRequirement {
        input_id: input_id.to_string(),
        payload_schema: None,
        cadence: None,
        identity_order: None,
        disposition: MapAuthorityRequirementDisposition::Unavailable,
        blocker: metadata.blocker.to_string(),
    }
}

fn unresolved_projection() -> MapAuthorityProjection {
    MapAuthorityProjection {
        schema: MAP_AUTHORITY_INPUT_SCHEMA.to_string(),
        hour_index: 42,
        content_manifest_digest_sha256: "1".repeat(64),
        product_modset_sha256: "2".repeat(64),
        effective_vfs_manifest_sha256: "3".repeat(64),
        requirements: MAP_AUTHORITY_REQUIRED_INPUTS
            .iter()
            .map(|input_id| unresolved_requirement(input_id))
            .collect(),
    }
}

#[test]
fn every_stable_family_has_only_explicit_unresolved_metadata_and_blocker() {
    for input_id in MAP_AUTHORITY_REQUIRED_INPUTS {
        let metadata = map_authority_requirement_metadata(input_id).unwrap();
        assert_eq!(metadata.input_id, *input_id);
        assert_eq!(metadata.payload_schema, None, "{input_id}");
        assert_eq!(metadata.cadence, None, "{input_id}");
        assert_eq!(metadata.identity_order, None, "{input_id}");
        assert!(!metadata.blocker.is_empty(), "{input_id}");
    }
    assert!(map_authority_requirement_metadata("recorder.selected_present").is_none());
}

#[test]
fn xmap00_can_only_return_a_complete_typed_gap_report() {
    let admission = admit_map_authority_projection(&unresolved_projection()).unwrap();
    assert_eq!(admission.gaps().len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
    assert_eq!(
        admission
            .gaps()
            .iter()
            .map(|gap| gap.input_id.as_str())
            .collect::<Vec<_>>(),
        MAP_AUTHORITY_REQUIRED_INPUTS
    );

    let refusal = refuse_product_map_until_xmap01(&admission);
    assert_eq!(refusal.code, "XMAP00_AUTHORITY_UNAVAILABLE");
    assert!(!refusal.authority_ready);
    assert_eq!(refusal.gap_ids.len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
    assert!(require_promoted_xmap01_renderer(&admission).is_err());

    let missing_frame = refuse_missing_xmap00_authority("world_frame");
    assert_eq!(missing_frame.code, "XMAP00_AUTHORITY_UNAVAILABLE");
    assert!(!missing_frame.authority_ready);
    assert_eq!(missing_frame.gap_ids, vec!["world_frame"]);
}

#[test]
fn caller_hashes_legacy_ready_rows_and_measured_empty_witnesses_cannot_admit() {
    let projection = unresolved_projection();
    for forbidden_field in [
        "payload_sha256",
        "ordering_sha256",
        "completeness_sha256",
        "provenance_receipt_sha256",
        "measured_empty_witness",
    ] {
        let mut json = serde_json::to_value(&projection).unwrap();
        json["requirements"][0][forbidden_field] = serde_json::json!("1".repeat(64));
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forbidden_field} was silently trusted"
        );
    }

    for forged_disposition in ["present", "measured_empty"] {
        let mut json = serde_json::to_value(&projection).unwrap();
        json["requirements"][0]["disposition"] = serde_json::json!(forged_disposition);
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forged_disposition} was admitted"
        );
    }

    let authority_source = include_str!("../src/map_authority.rs");
    assert!(!authority_source.contains("MapAuthorityReceipt"));
    assert!(!authority_source.contains("MapAuthorityMeasuredEmptyWitness"));
    assert!(!authority_source.contains("Ready {"));
    assert!(!authority_source.contains("fn is_ready"));
}

#[test]
fn recorder_presentation_fields_are_rejected_not_ignored() {
    for forbidden in [
        "selected_present",
        "camera",
        "map_mode",
        "selection",
        "gui_state",
        "draw_geometry",
    ] {
        let mut json = serde_json::to_value(unresolved_projection()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert(forbidden.into(), serde_json::json!({}));
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forbidden} was silently accepted"
        );
    }
}

#[test]
fn family_identity_order_and_unresolved_metadata_are_fail_closed() {
    let mut missing = unresolved_projection();
    missing.requirements.pop();
    assert!(admit_map_authority_projection(&missing).is_err());

    let mut duplicate = unresolved_projection();
    duplicate.requirements[1].input_id = duplicate.requirements[0].input_id.clone();
    assert!(admit_map_authority_projection(&duplicate).is_err());

    let mut unsorted = unresolved_projection();
    unsorted.requirements.swap(0, 1);
    assert!(admit_map_authority_projection(&unsorted).is_err());

    for field in ["payload_schema", "cadence", "identity_order"] {
        let mut json = serde_json::to_value(unresolved_projection()).unwrap();
        json["requirements"][0][field] = serde_json::json!("caller-invented");
        let error = admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).unwrap_err();
        assert!(
            error.contains("unresolved map authority metadata"),
            "{error}"
        );
    }

    let mut changed_blocker = unresolved_projection();
    changed_blocker.requirements[0].blocker = "caller says complete".into();
    assert!(admit_map_authority_projection(&changed_blocker)
        .unwrap_err()
        .contains("canonical unresolved blocker"));
}

#[test]
fn absent_and_unavailable_require_distinct_canonical_blocker_forms() {
    let mut forged_absent = unresolved_projection();
    forged_absent.requirements[0].disposition = MapAuthorityRequirementDisposition::Absent;
    assert!(
        admit_map_authority_json(&serde_json::to_vec(&forged_absent).unwrap())
            .unwrap_err()
            .contains("canonical unresolved blocker")
    );

    let metadata =
        map_authority_requirement_metadata(&forged_absent.requirements[0].input_id).unwrap();
    forged_absent.requirements[0].blocker = format!(
        "{}; WorldFrame root status is DomainAbsent",
        metadata.blocker
    );
    let admitted = admit_map_authority_json(&serde_json::to_vec(&forged_absent).unwrap()).unwrap();
    assert_eq!(
        admitted.gaps()[0].disposition,
        MapAuthorityRequirementDisposition::Absent
    );
    assert_eq!(
        admitted.gaps()[0].reason,
        forged_absent.requirements[0].blocker
    );

    let mut forged_unavailable = unresolved_projection();
    let metadata =
        map_authority_requirement_metadata(&forged_unavailable.requirements[0].input_id).unwrap();
    forged_unavailable.requirements[0].blocker = format!(
        "{}; WorldFrame root status is DomainAbsent",
        metadata.blocker
    );
    assert!(
        admit_map_authority_json(&serde_json::to_vec(&forged_unavailable).unwrap())
            .unwrap_err()
            .contains("canonical unresolved blocker")
    );
}

fn shipping_performance_run(source: &str) -> &str {
    let start = source
        .find("pub(crate) fn run(")
        .expect("shipping performance run");
    let end = source[start..]
        .find("pub(crate) fn run_stress(")
        .expect("stress benchmark boundary");
    let bounded = &source[start..start + end];
    let function_end = bounded.rfind("\n}").expect("shipping performance end") + 2;
    &bounded[..function_end]
}

fn shipping_app_run(source: &str) -> &str {
    let start = source.find("pub fn run(").expect("shipping app run");
    let end = source[start..]
        .find("#[cfg(test)]")
        .expect("app test-module boundary");
    &source[start..start + end]
}

fn shipping_headless_dump(source: &str) -> &str {
    let start = source
        .find("fn dump_production_recording_frame(")
        .expect("shipping headless dump");
    let end = source[start..]
        .find("fn reject_production_dump_debug_inputs(")
        .expect("next headless function boundary");
    let bounded = &source[start..start + end];
    let function_end = bounded.rfind("\n}").expect("shipping headless end") + 2;
    &bounded[..function_end]
}

fn shipping_composition_start(source: &str) -> &str {
    let start = source
        .find("impl ProductMapCompositionSession {")
        .expect("XMAP22 composition implementation");
    let end = source[start..]
        .find("pub(crate) fn adapter_identity(")
        .expect("XMAP22 composition start boundary");
    &source[start..start + end]
}

fn assert_gate_sequence(name: &str, source: &str) -> usize {
    source
        .find("crate::validate::prepare_product_map_foundation(")
        .or_else(|| source.find("ProductMapCompositionSession::start_for_headless("))
        .or_else(|| source.find("ProductMapCompositionSession::start_for_benchmark("))
        .or_else(|| source.find("ProductMapCompositionSession::start_for_window_target("))
        .or_else(|| source.find("ProductMapFoundationSession::start_with_assigned_stage_schedule("))
        .or_else(|| source.find("ProductMapFoundationSession::start("))
        .unwrap_or_else(|| panic!("{name} does not enter the shared product foundation"))
}

fn assert_shared_foundation_sequence() {
    let source = include_str!("../../replay_app/src/validate.rs");
    for required_sequence in [
        &[
            "product_foundation_assigned_stage_plan()",
            "derive_bundle_content_authority(",
            "execute_foundation_content_preflight(",
            "product_feature_adapters_with_assigned_stage_schedule(",
        ][..],
        &[
            "map_authority_projection_from_world_frame(",
            "admit_map_authority_projection(",
            "prepare_with_xmap12_inputs(",
            "require_fresh_foundation_frame(",
        ][..],
    ] {
        let mut cursor = 0;
        for required in required_sequence {
            let offset = source[cursor..]
                .find(required)
                .unwrap_or_else(|| panic!("shared foundation omits {required}"));
            cursor += offset + required.len();
        }
    }
}

fn compact_rust_source(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

#[test]
fn shipping_product_gates_precede_all_presentation_and_device_prerequisites() {
    assert_shared_foundation_sequence();
    let public_api = compact_rust_source(include_str!("../src/lib.rs"));
    assert!(
        !public_api.contains("product_feature_adapters,"),
        "public map_viewer API must not expose the incomplete unscheduled adapter core"
    );
    let app = shipping_app_run(include_str!("../../replay_app/src/app.rs"));
    let headless = shipping_headless_dump(include_str!("../../replay_app/src/headless.rs"));
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));

    let app_gate = assert_gate_sequence("app", app);
    let headless_gate = assert_gate_sequence("headless", headless);
    let performance_gate = assert_gate_sequence("performance", performance);

    let app_prefix = &app[..app_gate];
    let headless_prefix = &headless[..headless_gate];
    let performance_prefix = &performance[..performance_gate];
    for (name, prefix) in [
        ("app", app_prefix),
        ("headless", headless_prefix),
        ("performance", performance_prefix),
    ] {
        for forbidden in [
            "WorkloadContract",
            "validate_contract(",
            "production_render_sha256",
            ".production_render(",
            ".production_render_status(",
            "selected_present_sha256",
            ".selected_present",
            "launch_epoch",
            "capture_receipt",
            "open_product_recording_segment_with_authority",
            "authority_sha256",
            "expected_map_operations",
            "expected_selected_present_operations",
            "map_operation_count",
            "selected_present_ui_operation_count",
            ".submit(",
            "ProductObserverUiRuntime::from_playback_content(",
            "Camera::fit(",
            "Window::create(",
            "Gfx::create(",
            "Gfx::create_offscreen(",
            "read_target(",
            "write_bmp(",
        ] {
            assert!(
                !prefix.contains(forbidden),
                "{name} consumes {forbidden} before XMAP refusal"
            );
        }
    }

    let app_condition = app[..app_gate]
        .rfind("if production_recording {")
        .expect("app production-only gate condition");
    assert!(app_condition < app_gate);
    assert!(app[app_gate..].contains("if observer_render_admitted && !production_recording"));

    for (name, source, gate) in [
        ("headless", headless, headless_gate),
        ("performance", performance, performance_gate),
    ] {
        let tail = &source[gate..];
        assert!(
            tail.contains("composition.execute("),
            "{name} does not execute the shared XMAP22 composition"
        );
        for forbidden in [
            "production_render",
            "selected_present",
            ".submit(",
            "ProductObserverUiRuntime",
        ] {
            assert!(
                !tail.contains(forbidden),
                "{name} retains success tail token {forbidden}"
            );
        }
    }
}

#[test]
fn headless_and_recording_benchmark_execute_shared_composition() {
    let headless = shipping_headless_dump(include_str!("../../replay_app/src/headless.rs"));
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));
    for (name, source) in [("headless", headless), ("performance", performance)] {
        let gate = assert_gate_sequence(name, source);
        let execution = source[gate..]
            .find("composition.execute(")
            .unwrap_or_else(|| panic!("{name} does not execute the XMAP22 composition"));
        assert!(
            execution > 0,
            "{name} executes before starting the XMAP22 composition"
        );
    }

    let composition = shipping_composition_start(include_str!("../../replay_app/src/app.rs"));
    let foundation = composition
        .find("ProductMapFoundationSession::start_with_assigned_stage_schedule(")
        .expect("XMAP22 composition does not enter the shared product foundation");
    let device = composition
        .find("Gfx::create_offscreen(")
        .expect("XMAP22 composition does not create its offscreen target");
    assert!(
        foundation < device,
        "XMAP22 composition creates a device before content/foundation admission"
    );
}

#[test]
fn shipping_help_and_source_truth_are_bound_to_all_three_foundation_gates() {
    let main = include_str!("../../replay_app/src/main.rs");
    let usage_start = main.find("const USAGE: &str").expect("shipping usage");
    let usage_end = main[usage_start..]
        .find("const SENSITIVE_GAME_FILE_PATHS")
        .expect("usage boundary");
    let usage = &main[usage_start..usage_start + usage_end];
    for required in [
        "executes and measures the complete packet-native map composition with GPU completion, and writes no benchmark receipt unless exhaustive map-family admission and native execution succeed",
        "executes the complete packet-native map composition, and emits no map pixels, BMP, or success receipt unless exhaustive map-family admission and native execution succeed",
    ] {
        assert!(compact_rust_source(usage).contains(&compact_rust_source(required)));
    }

    let headless_source = include_str!("../../replay_app/src/headless.rs");
    let headless_prelude = &headless_source[..headless_source
        .find("use std::io::Write;")
        .expect("headless module prelude boundary")];
    assert!(
        compact_rust_source(headless_prelude).contains(&compact_rust_source(
            "same XMAP22 packet-native composition used by windowed playback",
        ))
    );

    let status = include_str!("../../../documents/MAP_GRAPHICS_PRODUCT_STATUS.md");
    for required in [
        "Windowed playback",
        "headless frame export",
        "recording benchmark",
        "ProductMapCompositionSession::execute_on_target",
    ] {
        assert!(
            compact_rust_source(status).contains(&compact_rust_source(required)),
            "map product status omits current route token {required}"
        );
    }

    let app = shipping_app_run(include_str!("../../replay_app/src/app.rs"));
    let headless = shipping_headless_dump(headless_source);
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));
    assert_gate_sequence("app", app);
    assert_gate_sequence("headless", headless);
    assert_gate_sequence("performance", performance);
}

#[test]
fn content_manifest_absence_is_a_pre_admission_envelope_error() {
    for (name, source, message) in [
        (
            "app",
            shipping_app_run(include_str!("../../replay_app/src/app.rs")),
            "portable playback has no admitted content manifest after binding",
        ),
        (
            "headless",
            shipping_headless_dump(include_str!("../../replay_app/src/headless.rs")),
            "portable playback has no admitted content manifest after binding",
        ),
        (
            "performance",
            shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs")),
            "performance benchmark recording has no admitted content manifest",
        ),
    ] {
        let structural_error = source
            .find(message)
            .unwrap_or_else(|| panic!("{name} has no stable content-envelope error"));
        let projection = assert_gate_sequence(name, source);
        assert!(
            structural_error < projection,
            "{name} mislabeled a pre-admission content-envelope failure as a family gap"
        );
    }

    let shared_foundation = include_str!("../../replay_app/src/validate.rs");
    assert!(
        shared_foundation.contains("map_authority_projection_from_world_frame("),
        "shared product foundation has no XMAP00 projection"
    );
}
