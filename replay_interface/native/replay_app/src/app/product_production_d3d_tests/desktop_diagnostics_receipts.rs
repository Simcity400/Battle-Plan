//! CPU receipt fixtures exercise the actual finalization and wire producer.
//! These rows are local test values, never desktop playback qualification.
use super::*;
use crate::d3d::PresentOutcome;
use crate::desktop_diagnostics::DesktopDiagnostics;
use serde_json::json;

fn pending_fixture() -> ProductMapCompositionReceipt {
    let mut receipt = xmap22_all_present_receipt_fixture();
    let expected = xmap22_expected_family_execution_witnesses(&receipt).unwrap();
    for row in &mut receipt.mandatory_family_execution_census {
        row.execution = Xmap22FamilyExecutionDisposition::Executed {
            execution_sha256: expected[&row.family].clone(),
        };
    }
    receipt.mandatory_family_execution_census_sha256 = snapshot::sha256::Sha256::hex(
        &serde_json::to_vec(&receipt.mandatory_family_execution_census).unwrap(),
    );
    receipt.foundation_envelope_sha256 = xmap22_reconstructed_foundation_envelope(&receipt)
        .unwrap()
        .deterministic_sha256()
        .unwrap();
    receipt.presented_to_product_target = false;
    receipt.presentation_status = Xmap22PresentationStatus::WindowPendingPresent;
    receipt.seal().unwrap()
}

#[test]
fn actual_present_finalization_survives_diagnostics_without_status_or_content_substitution() {
    let pending = pending_fixture();
    let mut diagnostics = DesktopDiagnostics::with_enabled(true);
    for (ordinal, (disposition, status, positive)) in [
        (PresentOutcome::Presented, "window_presented", true),
        (PresentOutcome::Skipped, "window_skipped", false),
        (PresentOutcome::Occluded, "window_occluded", false),
    ]
    .into_iter()
    .enumerate()
    {
        let finalized = pending.clone().finish_window_present(disposition).unwrap();
        let row = diagnostics.present_row(Some(&finalized)).unwrap();
        assert_eq!(row["schema"], "hoi4.viewer.desktop.present.v1");
        assert_eq!(row["pid"], std::process::id());
        assert_eq!(row["present_seq"], ordinal * 2 + 1);
        assert_eq!(row["last_input_seq"], 0);
        assert_eq!(row["map"]["presentation_status"], status);
        assert_eq!(row["map"]["presented_to_product_target"], positive);
        assert_eq!(
            row["map"],
            json!(&finalized),
            "complete authenticated receipt must survive"
        );
        let roundtrip = ProductMapCompositionReceipt::decode_json(
            &serde_json::to_vec(&row["map"]).unwrap(),
            "CPU diagnostic fixture",
        )
        .unwrap();
        assert_eq!(
            roundtrip.deterministic_sha256,
            finalized.deterministic_sha256
        );
        // A repeated Present is evidence too; the production writer must not
        // reuse the app's feature-note deduplication path.
        let repeated = diagnostics.present_row(Some(&finalized)).unwrap();
        assert_eq!(repeated["map"], row["map"]);
        assert_eq!(repeated["present_seq"], ordinal * 2 + 2);
    }
}

#[test]
fn absent_error_and_disabled_observations_never_become_a_positive_present() {
    let mut enabled = DesktopDiagnostics::with_enabled(true);
    let absent = enabled.present_row(None).unwrap();
    assert!(absent["map"].is_null());
    for (disposition, reason) in [
        ("present_error", "DXGI fixture failure"),
        ("frame_error", "actual composition unavailable"),
        ("frame_skipped", "no window presentation was attempted"),
    ] {
        let row = enabled.failure_row(disposition, reason).unwrap();
        assert!(row["map"].is_null());
        assert_eq!(row["disposition"], disposition);
        assert_eq!(row["reason"], reason);
        assert_eq!(row["present_seq"], absent["present_seq"]);
    }
    let mut disabled = DesktopDiagnostics::with_enabled(false);
    assert!(disabled.present_row(Some(&pending_fixture())).is_none());
    assert!(disabled.failure_row("present_error", "fixture").is_none());
}

#[test]
fn shipping_hooks_observe_consumption_and_present_after_existing_finalization() {
    let app = source_scan::app_production();
    let run = &app[app.find("pub fn run(").unwrap()..];
    let reached = run
        .find("let desktop_pointer_target = product_observer_ui.authored_pointer_move(")
        .unwrap();
    let observed = run
        .find("input.pointer(desktop_pointer_target.as_ref())")
        .unwrap();
    let input_finished = run
        .find("desktop_diagnostics.finish_input(desktop_input)")
        .unwrap();
    let present = run
        .find("let present_outcome = match gfx.present()")
        .unwrap();
    let finalized = run
        .find("receipt.finish_window_present(present_outcome)")
        .unwrap();
    let map_row = run
        .find("desktop_diagnostics.present(xmap22_window_receipt.as_ref())")
        .unwrap();
    let ui_row = run.find("desktop_diagnostics.presented_ui(").unwrap();
    assert!(reached < observed && observed < input_finished && input_finished < present);
    assert!(present < finalized && finalized < map_row && map_row < ui_row);
    let successful = &run[map_row..ui_row];
    assert!(successful.contains("ObserverUiFrameDisposition::Presented,"));
    assert!(successful.contains(".authored_window_snapshot(viewer_clock_micros)?"));
    assert!(run.contains("desktop_diagnostics.failed(\"present_error\""));
    assert!(run.contains("desktop_diagnostics.failed(\"frame_error\""));
}
