
use std::path::Path;

use replay_core::{SourceError, WorldTimeline};

#[cfg(all(test, windows))]
#[path = "combat_measurement.rs"]
mod combat_measurement;

#[cfg(all(test, windows))]
#[path = "particle_functional.rs"]
mod particle_functional;

#[cfg(windows)]
pub(crate) struct ProductMapFoundationSession {
    content_receipt: map_viewer::FoundationContentExecutionReceipt,
    adapters: map_viewer::ProductionMapFeatureAdapters<'static>,
    playback: map_viewer::FoundationPlaybackState,
    /// Recorded BEFORE playback admission rules on it: a refusing hour never
    /// gets past that admission, so this is the only place its per-family
    /// account survives.
    last_candidate_envelope: Option<map_viewer::FoundationRenderEnvelope>,
    /// Recorded beside it, for the same reason.
    last_family_costs: Vec<map_viewer::FamilyPreparationCost>,
    /// What the XMAP12 payload carried, recorded beside them.
    last_xmap12_payload: Option<map_viewer::Xmap12PayloadSummary>,
}

#[cfg(windows)]
pub(crate) fn product_foundation_assigned_stage_plan() -> crate::Result<(
    map_viewer::AssignedStageTopology,
    map_viewer::AssignedStageSchedule,
)> {
    let topology = map_viewer::product_foundation_assigned_stage_topology()?;
    let schedule = map_viewer::AssignedStageSchedule::from_topology(&topology)
        .map_err(|error| format!("product foundation assigned-stage schedule: {error}"))?;
    Ok((topology, schedule))
}

#[cfg(windows)]
impl ProductMapFoundationSession {
    pub(crate) fn start(
        resolver: &map_viewer::GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        terrain_options: map_viewer::TerrainLodCullOptions,
        recorded_foundation_content: Option<
            &portable_content_manifest::foundation_family::FoundationFamilyContentSet,
        >,
    ) -> crate::Result<Self> {
        let (topology, schedule) = product_foundation_assigned_stage_plan()?;
        Self::start_with_assigned_stage_schedule(
            resolver,
            content_admission,
            manifest,
            terrain_options,
            recorded_foundation_content,
            &topology,
            schedule,
        )
    }

    pub(crate) fn start_with_assigned_stage_schedule(
        resolver: &map_viewer::GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        manifest: &replay_core::ContentManifest,
        terrain_options: map_viewer::TerrainLodCullOptions,
        recorded_foundation_content: Option<
            &portable_content_manifest::foundation_family::FoundationFamilyContentSet,
        >,
        topology: &map_viewer::AssignedStageTopology,
        schedule: map_viewer::AssignedStageSchedule,
    ) -> crate::Result<Self> {
        let content_authority = crate::content_compatibility::derive_bundle_content_authority(
            manifest,
            resolver,
            content_admission,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let content_receipt = map_viewer::execute_foundation_content_preflight(
            resolver,
            content_admission,
            &content_authority.authority_sha256,
        )?;
        crate::content_compatibility::join_recorded_xmap02_foundation_families(
            recorded_foundation_content,
            content_admission,
            manifest,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Ok(Self {
            content_receipt,
            adapters: map_viewer::product_feature_adapters_with_assigned_stage_schedule(
                resolver,
                content_admission,
                terrain_options,
                &topology,
                schedule,
                &manifest.localisation_language,
            )?,
            playback: map_viewer::FoundationPlaybackState::default(),
            last_candidate_envelope: None,
            last_family_costs: Vec::new(),
            last_xmap12_payload: None,
        })
    }

    pub(crate) fn last_candidate_envelope(
        &self,
    ) -> Option<&map_viewer::FoundationRenderEnvelope> {
        self.last_candidate_envelope.as_ref()
    }

    /// Per-family wall time and footprint of the last hour this session
    /// prepared, refused or not.
    pub(crate) fn last_family_costs(&self) -> &[map_viewer::FamilyPreparationCost] {
        &self.last_family_costs
    }

    /// Distinct meshes the XMAP12 adapter holds packed and their resident bytes.
    pub(crate) fn xmap12_resident_geometry(&self) -> Option<map_viewer::Result<(usize, usize)>> {
        self.adapters.xmap12_resident_geometry()
    }

    /// What the XMAP12 payload of the last prepared hour carried.
    pub(crate) fn last_xmap12_payload(&self) -> Option<map_viewer::Xmap12PayloadSummary> {
        self.last_xmap12_payload
    }

    pub(crate) fn prepare_with_xmap12_inputs(
        &mut self,
        frame: &replay_core::WorldFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        manifest: &replay_core::ContentManifest,
        building_visual: Option<&map_viewer::Xmap12BuildingVisualFrame>,
        fixed_runtime: Option<&map_viewer::Xmap12FixedRuntimeState>,
        snow_mud_phase: &map_viewer::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample,
    ) -> crate::Result<map_viewer::PreparedFoundationFrame> {
        // Cleared FIRST: a failure before the family loop must not leave the
        // previous hour's account standing as this one's.
        self.last_candidate_envelope = None;
        self.last_family_costs.clear();
        self.last_xmap12_payload = None;
        // The map authority is derived inside the preparation, from this frame
        // and manifest: there is no admission to hand it beside them.
        let candidate = self.adapters.prepare_with_xmap12_inputs(
            frame,
            viewer,
            manifest,
            &self.content_receipt,
            building_visual,
            fixed_runtime,
            Some(snow_mud_phase),
        )?;
        self.last_candidate_envelope = Some(candidate.envelope.clone());
        self.last_family_costs = candidate.family_costs.clone();
        self.last_xmap12_payload = candidate.xmap12_payload_summary();
        let fresh = map_viewer::require_fresh_foundation_frame(
            self.playback
                .admit_allowing_xmap16_unavailable(candidate.envelope.clone())?,
        )?;
        if fresh != candidate.envelope {
            return Err("foundation playback admitted a different terrain frame envelope".into());
        }
        let _ = candidate.xmap16_rail_supply_state()?;
        Ok(candidate)
    }

    pub(crate) fn prepare_with_xmap12_and_xmap15_inputs(
        &mut self,
        frame: &replay_core::WorldFrame,
        viewer: &map_viewer::ViewerOwnedMapState,
        manifest: &replay_core::ContentManifest,
        building_visual: Option<&map_viewer::Xmap12BuildingVisualFrame>,
        fixed_runtime: Option<&map_viewer::Xmap12FixedRuntimeState>,
        selection: &map_viewer::Xmap15ViewerSelection,
        snow_mud_phase: &map_viewer::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample,
    ) -> crate::Result<map_viewer::PreparedFoundationFrame> {
        // Cleared FIRST: a failure before the family loop must not leave the
        // previous hour's account standing as this one's.
        self.last_candidate_envelope = None;
        self.last_family_costs.clear();
        self.last_xmap12_payload = None;
        // As above: the preparation derives its own map authority.
        let candidate = self.adapters.prepare_with_xmap12_and_xmap15_inputs(
            frame,
            viewer,
            manifest,
            &self.content_receipt,
            building_visual,
            fixed_runtime,
            selection,
            Some(snow_mud_phase),
        )?;
        self.last_candidate_envelope = Some(candidate.envelope.clone());
        self.last_family_costs = candidate.family_costs.clone();
        self.last_xmap12_payload = candidate.xmap12_payload_summary();
        let fresh = map_viewer::require_fresh_foundation_frame(
            self.playback
                .admit_allowing_xmap16_unavailable(candidate.envelope.clone())?,
        )?;
        if fresh != candidate.envelope {
            return Err("foundation playback admitted a different terrain frame envelope".into());
        }
        let _ = candidate.xmap16_rail_supply_state()?;
        Ok(candidate)
    }
}

// XMAP12 reads NO world input. Both halves of the family are derived.
//
// The per-placement building visual is DERIVED, never carried: lane OJ proved the family
// owes zero wire bytes, and lane OO landed the derivation (shared-spawn-point occupant law
// plus the destroyed-mesh selector). There is no `xmap12_building_visual` world input.
//
// The ambient/fixed lifecycle is derived too, and used not to be: this file read it out of
// `WorldFrame::domain("xmap12_fixed_runtime")` as a recorded world input while XMAP12's own
// authority (`re_evidence/rendering_laws/XMAP12/authority.json`,
// `production_contract.population`) assigned it to the viewer and `map_authority.rs` refused a
// recorded carrier for the rows it feeds. No recorder ever emitted the key, and one that did
// would be publishing one client's frame-rate-local presentation as world state: the lifecycle
// advances on the viewer's own renderer clock. The mint now lives in
// `map_viewer::mint_xmap12_fixed_runtime`, so there is no world input to authenticate and
// no `AuthenticatedXmap12FrameInputs` to carry one.

#[cfg(windows)]
pub(crate) fn prepare_product_map_foundation(
    frame: &replay_core::WorldFrame,
    viewer: &map_viewer::ViewerOwnedMapState,
    resolver: &map_viewer::GameFileResolver,
    content_admission: &replay_core::ValidatedContentAdmission,
    manifest: &replay_core::ContentManifest,
    terrain_options: map_viewer::TerrainLodCullOptions,
    snow_mud_phase: &map_viewer::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample,
) -> crate::Result<map_viewer::PreparedFoundationFrame> {
    // A single-frame entry point has no previous frame, which is the engine's own cold start
    // for a freshly created map-building render slot.
    ProductMapFoundationSession::start(
        resolver,
        content_admission,
        manifest,
        terrain_options,
        None,
    )?
    .prepare_with_xmap12_inputs(frame, viewer, manifest, None, None, snow_mud_phase)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionKind {
    NotFound,
    ContainerOrSchema,
    Integrity,
    Modset,
    Unsupported,
    MultipleGames,
    Timeline,
}

impl RejectionKind {
    pub fn headline(self) -> &'static str {
        match self {
            RejectionKind::NotFound => "recording not found",
            RejectionKind::ContainerOrSchema => "recording failed container/schema validation",
            RejectionKind::Integrity => "recording failed payload integrity validation",
            RejectionKind::Modset => "recording modset mismatch",
            RejectionKind::Unsupported => "recording not playable by this build",
            RejectionKind::MultipleGames => "recording holds multiple games",
            RejectionKind::Timeline => "recording timeline unusable",
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    Opened(OpenReport),
    Rejected {
        kind: RejectionKind,
        message: String,
    },
}

impl OpenOutcome {
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

pub fn open_and_validate(path: &Path) -> OpenOutcome {
    open_and_validate_segment(path, None)
}

pub fn open_and_validate_segment(path: &Path, logical_stream_id: Option<u64>) -> OpenOutcome {
    open_and_validate_segment_timed(path, logical_stream_id).0
}

pub fn open_and_validate_segment_timed(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> (OpenOutcome, Option<(f64, DecodePassTiming)>) {
    let open_started = std::time::Instant::now();
    let timeline = match open_timeline_segment_unvalidated(path, logical_stream_id) {
        Ok(timeline) => timeline,
        Err(outcome) => return (outcome, None),
    };
    let open_ms = open_started.elapsed().as_secs_f64() * 1e3;
    let timing = match validate_all_frame_payloads(&timeline, &mut |_, _| Ok(())) {
        Ok(timing) => timing,
        Err(outcome) => return (outcome, None),
    };
    let receipt = timeline.prefold_receipt();
    let opened = OpenOutcome::Opened(OpenReport {
        game_id: timeline.game_id().to_string(),
        hour_count: timeline.hours().len(),
        first_hour: timeline.first_hour(),
        last_hour: timeline.last_hour(),
        warnings: timeline.warnings().to_vec(),
        prefold_decoder_id: receipt.map(|value| value.decoder_id().to_string()),
        prefold_source_len: receipt.map(|value| value.source_len()),
        prefold_source_sha256: receipt.map(|value| value.source_sha256().to_string()),
    });
    (opened, Some((open_ms, timing)))
}

pub fn open_timeline(path: &Path) -> Result<WorldTimeline, OpenOutcome> {
    open_timeline_segment(path, None)
}

pub fn open_timeline_segment(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<WorldTimeline, OpenOutcome> {
    let timeline = open_timeline_segment_unvalidated(path, logical_stream_id)?;
    validate_all_frame_payloads(&timeline, &mut |_, _| Ok(()))?;
    Ok(timeline)
}

pub fn open_timeline_segment_unvalidated(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<WorldTimeline, OpenOutcome> {
    let source = match logical_stream_id {
        Some(id) => crate::recording_open::open_product_recording_segment(path, Some(id)),
        None => crate::recording_open::open_product_recording(path),
    }
    .map_err(classify_source_error)?;
    // A validation pass reads each hour once; a frame cache would only hold
    // finished hours resident.
    let config = replay_core::TimelineConfig {
        frame_cache_entries: 1,
        ..replay_core::TimelineConfig::default()
    };
    WorldTimeline::open_with_config(source, config).map_err(|err| OpenOutcome::Rejected {
        kind: RejectionKind::Timeline,
        message: format!("recording opened but its timeline is unusable: {err}"),
    })
}

/// Wall time of the decode pass, printed by the validation commands.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DecodePassTiming {
    pub hours: usize,
    pub total_ms: f64,
    pub max_hour_ms: f64,
    pub max_hour: i64,
}

impl DecodePassTiming {
    pub fn per_hour_ms(&self) -> f64 {
        if self.hours == 0 {
            0.0
        } else {
            self.total_ms / self.hours as f64
        }
    }
}

/// Every hour decodes exactly once: `on_frame` sees each validated frame in
/// hour order, so a caller that needs every frame does not decode them again.
pub fn validate_all_frame_payloads(
    timeline: &WorldTimeline,
    on_frame: &mut dyn FnMut(i64, &replay_core::WorldFrame) -> Result<(), String>,
) -> Result<DecodePassTiming, OpenOutcome> {
    let mut timing = DecodePassTiming::default();
    for &hour in timeline.hours() {
        let started = std::time::Instant::now();
        let frame = timeline.frame(hour).map_err(|error| {
            let kind = match &error {
                replay_core::WorldError::Source(source) => rejection_kind(source),
                _ => RejectionKind::ContainerOrSchema,
            };
            OpenOutcome::Rejected {
                kind,
                message: format!(
                    "recording frame at canonical hour {hour} failed payload integrity/decode validation: {error}"
                ),
            }
        })?;
        on_frame(hour, &frame).map_err(|message| OpenOutcome::Rejected {
            kind: RejectionKind::ContainerOrSchema,
            message,
        })?;
        let hour_ms = started.elapsed().as_secs_f64() * 1e3;
        timing.hours += 1;
        timing.total_ms += hour_ms;
        if hour_ms > timing.max_hour_ms {
            timing.max_hour_ms = hour_ms;
            timing.max_hour = hour;
        }
    }
    Ok(timing)
}

fn rejection_kind(err: &SourceError) -> RejectionKind {
    match err {
        SourceError::NotFound(_) => RejectionKind::NotFound,
        SourceError::ModsetMismatch(_) => RejectionKind::Modset,
        SourceError::Integrity(_) => RejectionKind::Integrity,
        SourceError::Open(_) => RejectionKind::ContainerOrSchema,
        SourceError::Unsupported(_) | SourceError::UnsupportedCompactStreamOnly(_) => {
            RejectionKind::Unsupported
        }
        SourceError::MultipleGames(_) => RejectionKind::MultipleGames,
    }
}

fn classify_source_error(err: SourceError) -> OpenOutcome {
    let message = err.to_string();
    let kind = rejection_kind(&err);
    OpenOutcome::Rejected { kind, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
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
                    assert!(outcome_summary_contains(name, &root, rel, expected));
                    let want_kind = match name {
                        "neg_empty_snapshot_index" => RejectionKind::Unsupported,
                        "neg_modset_mismatch" => RejectionKind::Modset,
                        "neg_bad_snapshot_hash" => RejectionKind::Integrity,
                        _ => RejectionKind::ContainerOrSchema,
                    };
                    assert_eq!(
                        kind, want_kind,
                        "{name}: rejection category; message={message:?}"
                    );
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
        timeline
            .frame(5)
            .expect("hour before the corrupt payload plays");
        timeline
            .frame(6)
            .expect("hour before the corrupt payload plays");
        assert!(
            timeline.frame(7).is_err(),
            "playback refuses fail-closed at the corrupt hour"
        );
        let outcome = match validate_all_frame_payloads(&timeline, &mut |_, _| Ok(())) {
            Err(outcome) => outcome,
            Ok(_) => panic!("lazy SHA mismatch must reject validation"),
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
