use super::{
    hourly_observation_selected_present_sha256, mint_hourly_recorded_input_observation,
};
use crate::validate_hourly_recorded_input_admission;
use production_render_catalog_live::{authenticate_recorded_pointer_input, A729_EXECUTABLE_SHA256};

fn sealed_selected_present() -> portable_render_stream::SelectedPresentReceipt {
    portable_render_stream::SelectedPresentReceipt::sealed(
        3,
        "a".repeat(64),
        41,
        2,
        9_000,
        1_000_000,
        1_001_000,
    )
}

fn minted_observation(
    selected_present: &portable_render_stream::SelectedPresentReceipt,
) -> production_render_catalog_live::RawRecordedPointerInputObservation {
    mint_hourly_recorded_input_observation(
        A729_EXECUTABLE_SHA256,
        7,
        77,
        9,
        5,
        100,
        200,
        210,
        220,
        selected_present,
        [1920, 1080],
        [10, 20],
        [11, 20],
    )
    .unwrap()
}

// kills: carrier.production_frame.authority.recorded_input_observation — the producer
// stamped the RAW binary v6 selection receipt into the observation while the consumer
// joins it against the portable canonical-JSON construction. The mint computes the
// stamp itself from THIS hour's receipt object, so the raw value never enters the path:
// restoring any other construction fails this assert.
#[test]
fn hourly_observation_stamps_the_portable_selected_present_receipt() {
    let selected_present = sealed_selected_present();
    let portable_sha = hourly_observation_selected_present_sha256(&selected_present).unwrap();
    assert_eq!(portable_sha, selected_present.computed_sha256().unwrap());

    let observation = minted_observation(&selected_present);
    assert_eq!(
        observation.selected_present_receipt_sha256, portable_sha,
        "the observation must bind the hour's own portable receipt construction"
    );
    let authenticated = authenticate_recorded_pointer_input(observation).unwrap();
    assert_eq!(
        authenticated.computed_receipt_sha256().unwrap(),
        authenticated.observation_receipt_sha256
    );
}

fn admitted_receipt() -> production_render_catalog_live::AuthenticatedRecordedInputVariant {
    let selected_present = sealed_selected_present();
    authenticate_recorded_pointer_input(minted_observation(&selected_present)).unwrap()
}

#[test]
fn admission_accepts_a_same_launch_hourly_receipt() {
    let receipt = admitted_receipt();
    validate_hourly_recorded_input_admission(&receipt, 77, A729_EXECUTABLE_SHA256)
        .expect("a same-launch hourly receipt must pass its admission check");
}

// Mutations recompute the self-hash first so each case is refused by the EXPLICIT
// launch comparison under test, not by a stale receipt hash.
#[test]
fn admission_refuses_a_cross_launch_or_tampered_hourly_receipt() {
    let good = admitted_receipt();

    let mut foreign_nonce = good.clone();
    foreign_nonce.process_start_nonce = 78;
    foreign_nonce.observation_receipt_sha256 =
        foreign_nonce.computed_receipt_sha256().unwrap();
    assert!(validate_hourly_recorded_input_admission(
        &foreign_nonce,
        77,
        A729_EXECUTABLE_SHA256
    )
    .is_err());

    let mut foreign_executable = good.clone();
    foreign_executable.executable_sha256 = "8".repeat(64);
    foreign_executable.observation_receipt_sha256 =
        foreign_executable.computed_receipt_sha256().unwrap();
    assert!(validate_hourly_recorded_input_admission(
        &foreign_executable,
        77,
        A729_EXECUTABLE_SHA256
    )
    .is_err());

    let mut tampered_receipt = good;
    tampered_receipt.observation_sequence += 1;
    assert!(validate_hourly_recorded_input_admission(
        &tampered_receipt,
        77,
        A729_EXECUTABLE_SHA256
    )
    .is_err());
}
