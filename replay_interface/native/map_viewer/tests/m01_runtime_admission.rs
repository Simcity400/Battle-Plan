
use map_viewer::{
    admit_m01_runtime_rows, parse_and_admit_m01_runtime_rows, M01A14ControlState,
    M01AdmissionError, M01AirBaseProvinceIfWing, M01NegativeRuntimeControl, M01PositiveRuntimeRow,
    M01RuntimeRowPair, M01UnitKind, M01VisibilityOrIntelState, M01_RUNTIME_INPUT_SCHEMA,
};
use serde_json::{json, Value};

fn pair(id: &str, clock: u32, base_event: u64) -> M01RuntimeRowPair {
    M01RuntimeRowPair {
        positive: M01PositiveRuntimeRow {
            clock,
            stable_unit_id: id.to_string(),
            unit_kind: M01UnitKind::Division,
            a7_selected_entity: "infantry_entity".to_string(),
            a7_selected_model: "infantry_mesh".to_string(),
            a7_air_base_province_if_wing: M01AirBaseProvinceIfWing::NotWing,
            a14_pov_tag: "GER".to_string(),
            a14_visibility_or_intel_state: M01VisibilityOrIntelState::Visible,
            a14_spotted_decision: true,
            render_included: true,
            immutable_ib_hash64: format!("{base_event:016x}"),
            world_matrix_le_hex_64_bytes: format!("{:0128x}", base_event + 1),
            pre_color_draw_event: base_event + 10,
            shadow_draw_event: base_event + 30,
            post_color_draw_event: base_event + 50,
        },
        negative_control: M01NegativeRuntimeControl {
            same_stable_unit_id: id.to_string(),
            changed_a14_pov_or_spotted_state: M01A14ControlState {
                pov_tag: "ENG".to_string(),
                visibility_or_intel_state: M01VisibilityOrIntelState::Hidden,
                spotted_decision: false,
            },
            render_included_false: false,
            explicit_exclusion_reason: "not visible to selected POV".to_string(),
            zero_draw_memberships_for_that_clock: true,
        },
    }
}

fn envelope_json(rows: &[M01RuntimeRowPair]) -> Value {
    json!({"schema": M01_RUNTIME_INPUT_SCHEMA, "rows": rows})
}

#[test]
fn admits_complete_synchronized_rows_and_exposes_only_opaque_output() {
    let rows = vec![pair("division-a", 42, 100), pair("division-b", 42, 200)];
    let admitted = admit_m01_runtime_rows(Some(&rows)).expect("complete input must admit");
    assert_eq!(admitted.len(), 2);
    assert!(!admitted.is_empty());
    assert_eq!(
        admitted
            .render_rows()
            .map(|row| row.stable_unit_id.as_str())
            .collect::<Vec<_>>(),
        ["division-a", "division-b"]
    );
    assert_eq!(admitted.controls().len(), 2);

    let json = serde_json::to_string(&envelope_json(&rows)).unwrap();
    assert_eq!(parse_and_admit_m01_runtime_rows(&json).unwrap(), admitted);
}

#[test]
fn rejects_absent_empty_unknown_or_wrong_schema_input() {
    assert_eq!(
        admit_m01_runtime_rows(None),
        Err(M01AdmissionError::MissingInput)
    );
    assert_eq!(
        admit_m01_runtime_rows(Some(&[])),
        Err(M01AdmissionError::EmptyRows)
    );

    let rows = vec![pair("division-a", 42, 100)];
    let mut unknown = envelope_json(&rows);
    unknown["unexpected"] = json!(true);
    assert!(matches!(
        parse_and_admit_m01_runtime_rows(&unknown.to_string()),
        Err(M01AdmissionError::InvalidJson(_))
    ));

    let mut wrong_schema = envelope_json(&rows);
    wrong_schema["schema"] = json!("hoi4-m01-runtime-input-v0");
    assert!(matches!(
        parse_and_admit_m01_runtime_rows(&wrong_schema.to_string()),
        Err(M01AdmissionError::UnsupportedSchema(_))
    ));
}

#[test]
fn rejects_every_missing_frozen_positive_or_negative_field() {
    let rows = vec![pair("division-a", 42, 100)];
    let base = envelope_json(&rows);
    let positive_fields = [
        "clock",
        "stable_unit_id",
        "unit_kind",
        "a7_selected_entity",
        "a7_selected_model",
        "a7_air_base_province_if_wing",
        "a14_pov_tag",
        "a14_visibility_or_intel_state",
        "a14_spotted_decision",
        "render_included",
        "immutable_ib_hash64",
        "world_matrix_le_hex_64_bytes",
        "pre_color_draw_event",
        "shadow_draw_event",
        "post_color_draw_event",
    ];
    for field in positive_fields {
        let mut incomplete = base.clone();
        incomplete["rows"][0]["positive"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            matches!(
                parse_and_admit_m01_runtime_rows(&incomplete.to_string()),
                Err(M01AdmissionError::InvalidJson(_))
            ),
            "missing positive field {field} was accepted"
        );
    }

    let negative_fields = [
        "same_stable_unit_id",
        "changed_a14_pov_or_spotted_state",
        "render_included_false",
        "explicit_exclusion_reason",
        "zero_draw_memberships_for_that_clock",
    ];
    for field in negative_fields {
        let mut incomplete = base.clone();
        incomplete["rows"][0]["negative_control"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            matches!(
                parse_and_admit_m01_runtime_rows(&incomplete.to_string()),
                Err(M01AdmissionError::InvalidJson(_))
            ),
            "missing negative field {field} was accepted"
        );
    }
}

#[test]
fn rejects_semantically_inconsistent_rows() {
    let mut mismatch = pair("division-a", 42, 100);
    mismatch.negative_control.same_stable_unit_id = "division-b".to_string();
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[mismatch])),
        Err(M01AdmissionError::StableUnitMismatch { .. })
    ));

    let mut unchanged = pair("division-a", 42, 100);
    unchanged.negative_control.changed_a14_pov_or_spotted_state = M01A14ControlState {
        pov_tag: "GER".to_string(),
        visibility_or_intel_state: M01VisibilityOrIntelState::Visible,
        spotted_decision: true,
    };
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[unchanged])),
        Err(M01AdmissionError::UnchangedA14Control { .. })
    ));

    let mut included_control = pair("division-a", 42, 100);
    included_control.negative_control.render_included_false = true;
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[included_control])),
        Err(M01AdmissionError::InvalidField {
            field: "render_included_false",
            ..
        })
    ));

    let mut wing_without_base = pair("wing-a", 42, 100);
    wing_without_base.positive.unit_kind = M01UnitKind::Wing;
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[wing_without_base])),
        Err(M01AdmissionError::InvalidField {
            field: "a7_air_base_province_if_wing",
            ..
        })
    ));

    let mut invalid_hash = pair("division-a", 42, 100);
    invalid_hash.positive.immutable_ib_hash64 = "GPU-OBJECT-ID".to_string();
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[invalid_hash])),
        Err(M01AdmissionError::InvalidField {
            field: "immutable_ib_hash64",
            ..
        })
    ));
}

#[test]
fn rejects_duplicate_identity_gpu_event_and_cross_pass_order() {
    let duplicate_id = vec![pair("division-a", 42, 100), pair("division-a", 42, 200)];
    assert!(matches!(
        admit_m01_runtime_rows(Some(&duplicate_id)),
        Err(M01AdmissionError::DuplicateStableUnitAtClock { .. })
    ));

    let first = pair("division-a", 42, 100);
    let mut gpu_collision = pair("division-b", 42, 200);
    gpu_collision.positive.immutable_ib_hash64 = first.positive.immutable_ib_hash64.clone();
    gpu_collision.positive.world_matrix_le_hex_64_bytes =
        first.positive.world_matrix_le_hex_64_bytes.clone();
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[first.clone(), gpu_collision])),
        Err(M01AdmissionError::DuplicateGpuKeyAtClock { .. })
    ));

    let mut event_collision = pair("division-b", 42, 200);
    event_collision.positive.shadow_draw_event = first.positive.shadow_draw_event;
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[first.clone(), event_collision])),
        Err(M01AdmissionError::DuplicateDrawEventAtClock { .. })
    ));

    let mut reversed_shadow = pair("division-b", 42, 200);
    reversed_shadow.positive.shadow_draw_event = 120;
    assert!(matches!(
        admit_m01_runtime_rows(Some(&[first, reversed_shadow])),
        Err(M01AdmissionError::PassSequenceMismatch { clock: 42 })
    ));
}
