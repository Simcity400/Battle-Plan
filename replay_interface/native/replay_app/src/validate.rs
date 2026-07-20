//! E4 recording open/validate UX.
//! Shipped opens enter through `crate::recording_open`, which statically selects
//! the bounded decoder capability and never retries ordinary or raw input.
//!
//! Opens a `.battleplan` through the SHARED validator — `replay_core::
//! open_recording`, the exact entry `replay_core/source.rs` and the timeline
//! use — and turns success/failure into a readable, category-tagged
//! diagnostic. There is NO parallel validation logic here: the underlying
//! precise error string (schema / container hash / modset / version) is
//! preserved verbatim inside the diagnostic message, so the product surface
//! shows the same specific rejection the ingest layer produced.

use std::path::Path;

use replay_core::{SourceError, WorldTimeline};

/// Category of a rejected recording, for a readable headline. The precise
/// underlying message is carried alongside in [`OpenOutcome::Rejected::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionKind {
    /// The path is not a `.battleplan` file or snapshot-store directory.
    NotFound,
    /// Container/manifest/schema/hash validation failed.
    ContainerOrSchema,
    /// Manifest modset disagrees with the snapshot provenance.
    Modset,
    /// Recognized but unplayable by this build (e.g. compact-stream-only).
    Unsupported,
    /// The store holds several games; one must be selected explicitly.
    MultipleGames,
    /// Opened, but the hour series is unusable (empty / not increasing).
    Timeline,
}

impl RejectionKind {
    pub fn headline(self) -> &'static str {
        match self {
            RejectionKind::NotFound => "recording not found",
            RejectionKind::ContainerOrSchema => "recording failed container/schema validation",
            RejectionKind::Modset => "recording modset mismatch",
            RejectionKind::Unsupported => "recording not playable by this build",
            RejectionKind::MultipleGames => "recording holds multiple games",
            RejectionKind::Timeline => "recording timeline unusable",
        }
    }
}

/// Summary of a successfully opened recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenReport {
    pub game_id: String,
    pub hour_count: usize,
    pub first_hour: i64,
    pub last_hour: i64,
    pub warnings: Vec<String>,
    pub prefold_decoder_id: Option<String>,
    pub prefold_source_len: Option<u64>,
    pub prefold_source_sha256: Option<String>,
}

/// The open/validate result surfaced to the UX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    Opened(OpenReport),
    Rejected {
        kind: RejectionKind,
        /// Readable message; always CONTAINS the underlying validator string
        /// (e.g. `sha256`, `required_features`, `empty snapshot index`).
        message: String,
    },
}

impl OpenOutcome {
    /// One-line human summary for logs / the diagnostic banner.
    pub fn summary(&self) -> String {
        match self {
            OpenOutcome::Opened(report) => format!(
                "opened {:?}: {} hour(s), {}..={}{}{}",
                report.game_id,
                report.hour_count,
                report.first_hour,
                report.last_hour,
                if report.warnings.is_empty() {
                    String::new()
                } else {
                    format!(" ({} warning(s))", report.warnings.len())
                },
                match (
                    report.prefold_decoder_id.as_deref(),
                    report.prefold_source_len,
                    report.prefold_source_sha256.as_deref(),
                ) {
                    (Some(decoder), Some(bytes), Some(sha256)) => format!(
                        " [compact source: decoder {decoder}, {bytes} bytes, sha256 {sha256}]"
                    ),
                    _ => String::new(),
                }
            ),
            OpenOutcome::Rejected { kind, message } => {
                format!("REJECTED — {}: {message}", kind.headline())
            }
        }
    }

    pub fn is_opened(&self) -> bool {
        matches!(self, OpenOutcome::Opened(_))
    }
}

/// Run the shared validator on `path` and classify the result. The whole
/// recording is opened (container + snapshot index + timeline), so this is the
/// same acceptance the playback path performs — a positive fixture that opens
/// here will open for playback, and a negative that is rejected here is
/// rejected for playback.
pub fn open_and_validate(path: &Path) -> OpenOutcome {
    match open_timeline(path) {
        Ok(timeline) => {
            let receipt = timeline.prefold_receipt();
            OpenOutcome::Opened(OpenReport {
            game_id: timeline.game_id().to_string(),
            hour_count: timeline.hours().len(),
            first_hour: timeline.first_hour(),
            last_hour: timeline.last_hour(),
            warnings: timeline.warnings().to_vec(),
            prefold_decoder_id: receipt.map(|value| value.decoder_id().to_string()),
            prefold_source_len: receipt.map(|value| value.source_len()),
            prefold_source_sha256: receipt.map(|value| value.source_sha256().to_string()),
        })
        }
        Err(outcome) => outcome,
    }
}

/// Open through the same shared container/timeline path while retaining the
/// timeline for the selected-installation compatibility gate.
pub fn open_timeline(path: &Path) -> Result<WorldTimeline, OpenOutcome> {
    let source = crate::recording_open::open_product_recording(path)
        .map_err(classify_source_error)?;
    let timeline = WorldTimeline::open(source).map_err(|err| OpenOutcome::Rejected {
        kind: RejectionKind::Timeline,
        message: format!("recording opened but its timeline is unusable: {err}"),
    })?;
    validate_all_frame_payloads(&timeline)?;
    Ok(timeline)
}

/// Verify every indexed snapshot payload before declaring a recording clean.
/// Container open validates structure/identity, but lazy entry verification
/// means a later SHA/CRC failure otherwise remains hidden until playback seeks
/// that hour. The explicit validation commands and startup compatibility gate
/// are certification surfaces, so they must reject the whole recording.
fn validate_all_frame_payloads(timeline: &WorldTimeline) -> Result<(), OpenOutcome> {
    for &hour in timeline.hours() {
        timeline.frame(hour).map_err(|error| OpenOutcome::Rejected {
            kind: RejectionKind::ContainerOrSchema,
            message: format!(
                "recording frame at canonical hour {hour} failed payload integrity/decode validation: {error}"
            ),
        })?;
    }
    Ok(())
}

fn classify_source_error(err: SourceError) -> OpenOutcome {
    let message = err.to_string();
    // Total match on the TYPED error — no `.contains()` on error text. The
    // wording of an underlying message can change freely without silently
    // reclassifying a rejection (the modset/not-found signals are typed all the
    // way from the snapshot crate).
    let kind = match &err {
        SourceError::NotFound(_) => RejectionKind::NotFound,
        SourceError::ModsetMismatch(_) => RejectionKind::Modset,
        SourceError::Open(_) => RejectionKind::ContainerOrSchema,
        SourceError::Unsupported(_) => RejectionKind::Unsupported,
        SourceError::MultipleGames(_) => RejectionKind::MultipleGames,
    };
    OpenOutcome::Rejected { kind, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        // CARGO_MANIFEST_DIR = .../native/replay_app → .../replay_interface/fixtures
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
    }

    fn manifest() -> Value {
        let path = fixture_root().join("manifest.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read fixture manifest {}: {err}", path.display()));
        serde_json::from_str(&text).expect("fixture manifest json")
    }

    #[test]
    fn all_positive_fixtures_open_through_shared_validator() {
        let root = fixture_root();
        let manifest = manifest();
        let positive = manifest["positive"].as_array().expect("positive list");
        assert_eq!(positive.len(), 19, "expected 19 positive fixtures");
        for entry in positive {
            let name = entry["name"].as_str().expect("name");
            let rel = entry["path"].as_str().expect("path");
            let outcome = open_and_validate(&root.join(rel));
            match outcome {
                OpenOutcome::Opened(report) => {
                    assert_eq!(report.game_id, name, "{name}: game id");
                    let expected_hours = entry["hours"].as_array().expect("hours").len();
                    assert_eq!(report.hour_count, expected_hours, "{name}: hour count");
                    assert!(report.warnings.is_empty(), "{name}: unexpected warnings");
                }
                OpenOutcome::Rejected { kind, message } => {
                    panic!("{name}: positive fixture rejected ({kind:?}): {message}")
                }
            }
        }
    }

    #[test]
    fn negative_fixtures_rejected_with_their_specific_error() {
        let root = fixture_root();
        let manifest = manifest();
        let negative = manifest["negative"].as_array().expect("negative list");
        assert_eq!(negative.len(), 4, "expected 4 negative fixtures");
        // The full rejection matrix: each negative → its specific surfaced
        // string AND a sensible category.
        for entry in negative {
            let name = entry["name"].as_str().expect("name");
            let rel = entry["path"].as_str().expect("path");
            let expected = entry["expected_error_contains"].as_str().expect("expected");
            let outcome = open_and_validate(&root.join(rel));
            match outcome {
                OpenOutcome::Opened(_) => panic!("{name}: negative fixture unexpectedly opened"),
                OpenOutcome::Rejected { kind, message } => {
                    assert!(
                        message.contains(expected),
                        "{name}: expected message to contain {expected:?}, got {message:?}"
                    );
                    // The summary the UX shows also carries the specific string.
                    assert!(outcome_summary_contains(name, &root, rel, expected));
                    // Category sanity per fixture.
                    let want_kind = match name {
                        "neg_empty_snapshot_index" => RejectionKind::Unsupported,
                        "neg_modset_mismatch" => RejectionKind::Modset,
                        _ => RejectionKind::ContainerOrSchema,
                    };
                    assert_eq!(kind, want_kind, "{name}: rejection category");
                }
            }
        }
    }

    fn outcome_summary_contains(_name: &str, root: &Path, rel: &str, needle: &str) -> bool {
        open_and_validate(&root.join(rel))
            .summary()
            .contains(needle)
    }

    #[test]
    fn classification_is_typed_not_string_matched() {
        // A malformed-modset MANIFEST message (produced by replay_core's
        // recorder-manifest validation) contains the word "modset" but is a
        // container/schema malformation, not a modset MISMATCH: only the typed
        // variant may classify as Modset.
        let malformed = classify_source_error(SourceError::Open(
            "recorder manifest \"session.json\" modset is missing or not an object".into(),
        ));
        match malformed {
            OpenOutcome::Rejected { kind, .. } => {
                assert_eq!(kind, RejectionKind::ContainerOrSchema)
            }
            OpenOutcome::Opened(_) => unreachable!(),
        }

        let mismatch = classify_source_error(SourceError::ModsetMismatch(
            "open .battleplan: container: container rejected: manifest modset_id \"a\" disagrees with snapshot provenance.mod \"b\"".into(),
        ));
        match mismatch {
            OpenOutcome::Rejected { kind, message } => {
                assert_eq!(kind, RejectionKind::Modset);
                assert!(message.contains("disagrees with snapshot provenance.mod"));
            }
            OpenOutcome::Opened(_) => unreachable!(),
        }

        let not_found = classify_source_error(SourceError::NotFound(
            "path does not exist (expected a .battleplan file or a snapshot store directory): x"
                .into(),
        ));
        match not_found {
            OpenOutcome::Rejected { kind, .. } => assert_eq!(kind, RejectionKind::NotFound),
            OpenOutcome::Opened(_) => unreachable!(),
        }
    }

    struct LazyIntegrityFailureSource {
        hours: Vec<i64>,
    }

    impl replay_core::ReplaySource for LazyIntegrityFailureSource {
        fn game_id(&self) -> &str {
            "lazy-integrity-failure"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(
            &self,
            hour_index: i64,
        ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
            if hour_index == 7 {
                return Err(replay_core::SourceError::Open(
                    "snapshot sha256 mismatch at lazy payload read".into(),
                ));
            }
            Ok(replay_core::RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: Default::default(),
            })
        }
    }

    #[test]
    fn exhaustive_validation_rejects_a_lazy_midstream_sha_mismatch() {
        let timeline = WorldTimeline::open(Box::new(LazyIntegrityFailureSource {
            hours: vec![5, 6, 7, 8],
        }))
        .expect("timeline index opens before lazy payload read");
        let outcome = match validate_all_frame_payloads(&timeline) {
            Err(outcome) => outcome,
            Ok(()) => panic!("lazy SHA mismatch must reject validation"),
        };
        match outcome {
            OpenOutcome::Rejected { kind, message } => {
                assert_eq!(kind, RejectionKind::ContainerOrSchema);
                assert!(message.contains("canonical hour 7"), "{message}");
                assert!(message.contains("sha256 mismatch"), "{message}");
            }
            OpenOutcome::Opened(_) => panic!("integrity failure cannot open"),
        }
    }

    #[test]
    fn missing_path_is_not_found() {
        let outcome = open_and_validate(Path::new("this/does/not/exist.battleplan"));
        match outcome {
            OpenOutcome::Rejected { kind, .. } => assert_eq!(kind, RejectionKind::NotFound),
            OpenOutcome::Opened(_) => panic!("missing path must be rejected"),
        }
    }

    #[test]
    fn relocated_replay_opens_without_source_machine_or_config_paths() {
        let source = fixture_root().join("positive/fx_owner_controller_flip.battleplan");
        let root = std::env::temp_dir().join(format!(
            "replay-app-relocated-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let moved = root
            .join("shared-by-another-user")
            .join("renamed-recording.battleplan");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::copy(&source, &moved).unwrap();
        let outcome = open_and_validate(&moved);
        assert!(outcome.is_opened(), "{}", outcome.summary());
        let _ = std::fs::remove_dir_all(root);
    }
}
