//! Fail-closed product composition for sealed stitching inputs.
//!
//! Live capture owns segment formation and sealing. Once it has immutable
//! [`SegmentInput`] values and an exact payload source, this module is the one
//! production routing seam: plan, refuse uncertain inputs without publishing,
//! or atomically publish the validated v2 container. Keeping the seam here also
//! gives offline tests the exact call graph without enabling auto-attach or
//! launching the game.

use std::path::Path;

use super::publish::{publish_stitched_container, PublishReport, StitchedPayloadSource};
use super::{plan_stitch, SegmentInput, StitchOutcome, StitchRequiredInputs, StitchedPlan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductStitchOutcome {
    /// Inputs could not prove one campaign/order. No destination was touched.
    KeptSeparate { reason: String },
    /// A v2 stitched container was fully validated and atomically published.
    Published(PublishReport),
}

/// Plan and publish one sealed product stitch.
///
/// This function deliberately accepts no defaults: caller-supplied deterministic
/// inputs, sealed segments, payload authority, and an explicit `.battleplan`
/// destination are all required. A planner refusal is a successful fail-closed
/// outcome, not an I/O error, and never calls the publisher.
pub fn publish_product_stitch<S: StitchedPayloadSource>(
    game_id: &str,
    required_inputs: StitchRequiredInputs,
    segments: Vec<SegmentInput>,
    source: &S,
    destination: &Path,
) -> Result<ProductStitchOutcome, String> {
    publish_product_stitch_with(
        game_id,
        required_inputs,
        segments,
        destination,
        |plan, destination| publish_stitched_container(plan, source, destination),
    )
}

fn publish_product_stitch_with<F>(
    game_id: &str,
    required_inputs: StitchRequiredInputs,
    segments: Vec<SegmentInput>,
    destination: &Path,
    publish: F,
) -> Result<ProductStitchOutcome, String>
where
    F: FnOnce(&StitchedPlan, &Path) -> Result<PublishReport, String>,
{
    validate_destination(destination)?;
    if segments.len() < 2 {
        return Ok(ProductStitchOutcome::KeptSeparate {
            reason: "product stitching requires at least two sealed segments".to_string(),
        });
    }
    match plan_stitch(game_id, required_inputs, segments) {
        StitchOutcome::Separate { reason } => Ok(ProductStitchOutcome::KeptSeparate { reason }),
        StitchOutcome::Stitched(plan) => {
            publish(&plan, destination).map(ProductStitchOutcome::Published)
        }
    }
}

fn validate_destination(destination: &Path) -> Result<(), String> {
    if destination.file_name().is_none() {
        return Err("stitched product destination has no file name".to_string());
    }
    if destination.extension().and_then(|value| value.to_str()) != Some("hoi4replay") {
        return Err("stitched product destination must end in .battleplan".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stitching::{
        AppendTicket, CampaignIdentityResolution, CompatibilityResolution, CoverageDisposition,
        HourObservation, ObservationOwner, RawSourceDescriptor, RawSourceStorage,
        SegmentProvenance, SourceParticipation, STITCH_ENTRY_ORDER,
    };
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const KEY: &[u8] = b"product-campaign";

    fn required_inputs() -> StitchRequiredInputs {
        StitchRequiredInputs {
            schema_version: "schema-product-v1".into(),
            fold_version: "fold-product-v1".into(),
            canonical_writer_version: "canon-product-v1".into(),
            packer_codec_version: "zip-product-v1".into(),
            created_utc: "2026-07-19T00:00:00Z".into(),
            compression_method: "store".into(),
            compression_parameters: BTreeMap::from([("level".into(), "none".into())]),
            entry_order: STITCH_ENTRY_ORDER.into(),
        }
    }

    fn segment(play_order: u64, hour: i64) -> SegmentInput {
        let ticket = AppendTicket {
            campaign_key: KEY.to_vec(),
            play_order,
            session_nonce: format!("session-{play_order}"),
            process_start_identity: format!("process-{play_order}"),
        };
        let epoch_token = format!("epoch-{play_order}");
        let owner = ObservationOwner {
            ticket: ticket.clone(),
            in_session_ordinal: 0,
            epoch_token: epoch_token.clone(),
        };
        let disposition = CoverageDisposition::Snapshot {
            sha256: format!("{hour:064x}"),
        };
        let source_path = format!("sessions/{play_order}/collector_stream.h4cr");
        let stored_digest = format!("{:064x}", play_order + 1);
        let decoded_digest = format!("{:064x}", play_order + 100);
        let participation = SourceParticipation {
            ticket: ticket.clone(),
            scheme_major: 1,
            identity_evidence_digest: [7; 32],
            record_seq_bounds: (0, 1),
            byte_bounds: (0, 100),
        };
        SegmentInput {
            play_order,
            in_session_ordinal: 0,
            identity: CampaignIdentityResolution::Verified {
                scheme_major: 1,
                campaign_key: KEY.to_vec(),
                evidence_digest: [7; 32],
            },
            ticket: Some(ticket),
            compatibility: CompatibilityResolution::Proven {
                evidence_id: format!("compat-{play_order}"),
                compatibility_class: "a729-product".into(),
                build_exe_sha256: "build-a729".into(),
                modset_id: "modset-product".into(),
                schema_version: "schema-product-v1".into(),
                static_catalog_digest: "catalog-product".into(),
            },
            transition_coverage: BTreeMap::new(),
            coverage: BTreeMap::from([(hour, disposition.clone())]),
            observations: BTreeMap::from([(
                hour,
                vec![HourObservation {
                    observation_id: format!("observation-{play_order}"),
                    owner,
                    record_seq_bounds: (0, 1),
                    byte_bounds: (0, 100),
                    disposition,
                }],
            )]),
            raw_source: Some(RawSourceDescriptor {
                session_nonce: format!("session-{play_order}"),
                process_start_identity: format!("process-{play_order}"),
                source_path: source_path.clone(),
                stored_digest: stored_digest.clone(),
                decoded_digest: decoded_digest.clone(),
                record_seq_bounds: (0, 1),
                byte_bounds: (0, 100),
                storage: RawSourceStorage::Standalone,
                participations: vec![participation],
            }),
            provenance: SegmentProvenance {
                session_nonce: format!("session-{play_order}"),
                process_start_identity: format!("process-{play_order}"),
                epoch_token,
                stream_major: 2,
                build_exe_sha256: Some("build-a729".into()),
                modset_id: Some("modset-product".into()),
                schema_version: Some("schema-product-v1".into()),
                static_catalog_digest: Some("catalog-product".into()),
                source_path: Some(source_path),
                source_stored_digest: Some(stored_digest),
                source_decoded_digest: Some(decoded_digest),
                record_seq_bounds: Some((0, 1)),
                byte_bounds: Some((0, 100)),
                ..Default::default()
            },
        }
    }

    fn destination() -> PathBuf {
        std::env::temp_dir().join("offline-product-stitch.battleplan")
    }

    #[test]
    fn proven_segments_reach_the_one_publisher_call() {
        let called = Cell::new(0);
        let destination = destination();
        let result = publish_product_stitch_with(
            "the game",
            required_inputs(),
            vec![segment(0, 10), segment(1, 10)],
            &destination,
            |plan, observed_destination| {
                called.set(called.get() + 1);
                assert_eq!(plan.segments.len(), 2);
                assert_eq!(observed_destination, destination);
                Ok(PublishReport {
                    published_path: observed_destination.to_path_buf(),
                    container_bytes: 123,
                    container_sha256: "0".repeat(64),
                    visible_snapshot_hours: 1,
                    segments: 2,
                })
            },
        )
        .unwrap();
        assert_eq!(called.get(), 1);
        assert!(matches!(result, ProductStitchOutcome::Published(_)));
    }

    #[test]
    fn planner_refusal_never_calls_publisher_or_touches_destination() {
        let called = Cell::new(false);
        let destination = destination();
        let mut conflicting = segment(1, 10);
        conflicting.identity = CampaignIdentityResolution::Conflict {
            reason: "fixture conflict".into(),
        };
        let result = publish_product_stitch_with(
            "the game",
            required_inputs(),
            vec![segment(0, 10), conflicting],
            &destination,
            |_, _| {
                called.set(true);
                unreachable!("publisher must not run for refused inputs")
            },
        )
        .unwrap();
        assert!(!called.get());
        assert!(matches!(result, ProductStitchOutcome::KeptSeparate { .. }));
    }

    #[test]
    fn single_segment_and_wrong_extension_fail_before_publish() {
        let called = Cell::new(false);
        let single = publish_product_stitch_with(
            "the game",
            required_inputs(),
            vec![segment(0, 10)],
            &destination(),
            |_, _| {
                called.set(true);
                unreachable!()
            },
        )
        .unwrap();
        assert!(matches!(single, ProductStitchOutcome::KeptSeparate { .. }));
        assert!(!called.get());

        let error = publish_product_stitch_with(
            "the game",
            required_inputs(),
            vec![segment(0, 10), segment(1, 10)],
            Path::new("bad.zip"),
            |_, _| unreachable!(),
        )
        .unwrap_err();
        assert!(error.contains(".battleplan"));
    }
}
