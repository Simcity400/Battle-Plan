//! Reload / relaunch **stitching**: the deterministic supersede algorithm,
//! rollback-segment retention, and the stitched container/index model.
//!
//! This module is the production form of the reference algorithm in
//! `extractor/documents/STITCHING_DESIGN.md` (section 6), plus the segment
//! formation rules (section 5), the fail-closed campaign-identity / play-order
//! gate (sections 4 & 10), and the container/index layout (section 7).
//!
//! # Owner policy (decision of record)
//!
//! When a session reloads an older save and replays a span, the *later-played*
//! span **supersedes** the earlier one in the main continuity from the later
//! branch's cutover hour onward. Superseded history is **never deleted**: it is
//! retained as rollback data and remains addressable through the per-segment
//! indexes. `main` / `rollback` is a *span* property, not a whole-segment label.
//!
//! # Hard laws honoured here
//!
//! * **Zero silent data loss** — superseded coverage becomes [`SupersededSpan`]s;
//!   no hour is ever dropped, only reclassified with a `superseded_by` owner.
//! * **Fail closed** — any identity / ordering / epoch uncertainty yields
//!   [`StitchOutcome::Separate`] (keep sessions apart, surface why); never a
//!   guessed branch. Cross-session stitching is *disabled* while identity is
//!   unbound.
//! * **Determinism** — [`plan_stitch`] is a pure function of its ordered input;
//!   [`StitchedPlan::to_container_files`] serializes canonically so folding the
//!   same inputs twice yields byte-identical container payloads.
//!
//! # Campaign-identity seam
//!
//! [`CampaignIdentityResolution`] and [`AppendTicket`] are defined here **locally**
//! against the interface `STITCHING_DESIGN.md` section 4 specifies. The actual
//! identity *derivation* lives in `CAMPAIGN_IDENTITY_DESIGN.md` (a parallel lane).
//! Stitching treats `campaign_key` as opaque bytes and never inspects save
//! filenames, paths, timestamps, tags, mod/scenario names, or game dates. When the
//! parallel lane lands, its provider type is reconciled against these definitions;
//! the seam is recorded in `dossiers/W12C_STITCHING_IMPL.md`.

use std::collections::BTreeMap;

pub mod formation;
pub mod product;
pub mod publish;
pub mod runtime;

use serde_json::{json, Map, Value};
use snapshot::canon::write_canonical;
use snapshot::container::{canonical_stitch_input_digest, normalize_stitch_source_path};
use snapshot::sha256::Sha256;

/// The stitched container format version this module targets (design section 7/9).
pub const STITCH_CONTAINER_VERSION: u64 = 2;
/// The required-feature flag a stitched container declares (design section 9). Old
/// readers that do not know it fail closed rather than hide the rollback graph.
pub const STITCH_REQUIRED_FEATURE: &str = "timeline_segments_v1";
/// `index.json` schema tag for the stitched main-playback index (design section 7.1).
pub const STITCH_INDEX_SCHEMA: &str = "stitched-main-v1";
/// The only independently-verifiable v2 ZIP publication order.
pub const STITCH_ENTRY_ORDER: &str = "lexicographic-path-manifest-last-v1";
/// Selection rule serialized beside every repeated-hour observation set.
pub const SAME_HOUR_SELECTION_RULE: &str = "greatest_record_sequence_v1";

// ===========================================================================
// Section 4 — campaign identity & play-order interface (locally defined seam)
// ===========================================================================

/// One fail-closed identity result per verified save epoch (design section 4).
///
/// `campaign_key` is opaque. Two epochs are candidates for one campaign **only**
/// when both are `Verified`, their `scheme_major` matches, and their `campaign_key`
/// bytes are equal. `evidence_digest` is retained for audit but is never the
/// equality key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignIdentityResolution {
    Verified {
        scheme_major: u16,
        campaign_key: Vec<u8>,
        evidence_digest: [u8; 32],
    },
    Missing {
        reason: String,
    },
    Ambiguous {
        reason: String,
        candidate_count: u32,
    },
    Conflict {
        reason: String,
    },
}

/// Serialized-order token allocated per campaign participation (design section 4).
///
/// `play_order` is a strictly-increasing campaign-local append ordinal committed
/// under a campaign-local lock **before** the first campaign-bound byte is
/// accepted. It encodes the order sessions were actually played/observed — never
/// filesystem order, wall clock, zip order, or mtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendTicket {
    pub campaign_key: Vec<u8>,
    pub play_order: u64,
    pub session_nonce: String,
    pub process_start_identity: String,
}

impl AppendTicket {
    fn to_value(&self) -> Value {
        json!({
            "campaign_key_digest": Sha256::hex(&self.campaign_key),
            "play_order": self.play_order,
            "session_nonce": self.session_nonce,
            "process_start_identity": self.process_start_identity,
        })
    }
}

/// Every byte-affecting non-segment input required to build a deterministic
/// stitched publication. None of these values may be sampled by the stitcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StitchRequiredInputs {
    pub schema_version: String,
    pub fold_version: String,
    pub canonical_writer_version: String,
    pub packer_codec_version: String,
    pub created_utc: String,
    pub compression_method: String,
    pub compression_parameters: BTreeMap<String, String>,
    pub entry_order: String,
}

impl StitchRequiredInputs {
    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("schema_version", self.schema_version.as_str()),
            ("fold_version", self.fold_version.as_str()),
            (
                "canonical_writer_version",
                self.canonical_writer_version.as_str(),
            ),
            ("packer_codec_version", self.packer_codec_version.as_str()),
            ("created_utc", self.created_utc.as_str()),
            ("compression_method", self.compression_method.as_str()),
            ("entry_order", self.entry_order.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!("mandatory stitch input {name} is missing/empty"));
            }
        }
        if self.compression_parameters.is_empty()
            || self
                .compression_parameters
                .iter()
                .any(|(key, value)| key.is_empty() || value.is_empty())
        {
            return Err("mandatory stitch input compression_parameters is missing/empty".into());
        }
        Ok(())
    }
}

/// Where the byte-for-byte source is retained. A source containing more than one
/// campaign participation can only be standalone; embedding it under one campaign
/// would silently transfer foreign bytes into that campaign ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSourceStorage {
    Standalone,
    EmbeddedCampaign { campaign_key: Vec<u8> },
}

/// One exact campaign participation extent inside a session raw source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceParticipation {
    pub ticket: AppendTicket,
    pub scheme_major: u16,
    pub identity_evidence_digest: [u8; 32],
    /// Inclusive decoded record-sequence bounds.
    pub record_seq_bounds: (u64, u64),
    /// Half-open decoded byte bounds.
    pub byte_bounds: (u64, u64),
}

impl SourceParticipation {
    fn participation_id(&self) -> String {
        canon_hex(&self.binding_value())
    }

    fn binding_value(&self) -> Value {
        json!({
            "ticket": self.ticket.to_value(),
            "scheme_major": self.scheme_major,
            "identity_evidence_digest": hex(&self.identity_evidence_digest),
            "record_seq_bounds": [self.record_seq_bounds.0, self.record_seq_bounds.1],
            "byte_bounds": [self.byte_bounds.0, self.byte_bounds.1],
        })
    }

    fn to_value(&self) -> Value {
        let mut value = self.binding_value().as_object().unwrap().clone();
        value.insert("participation_id".into(), json!(self.participation_id()));
        Value::Object(value)
    }
}

/// Campaign-independent identity and complete participation ledger for one exact
/// session/process raw source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSourceDescriptor {
    pub session_nonce: String,
    pub process_start_identity: String,
    pub source_path: String,
    pub stored_digest: String,
    pub decoded_digest: String,
    pub record_seq_bounds: (u64, u64),
    pub byte_bounds: (u64, u64),
    pub storage: RawSourceStorage,
    pub participations: Vec<SourceParticipation>,
}

impl RawSourceDescriptor {
    /// The source key is deliberately campaign-independent.
    pub fn source_key(&self) -> String {
        let mut bytes = Vec::new();
        for value in [
            self.session_nonce.as_str(),
            self.process_start_identity.as_str(),
            self.stored_digest.as_str(),
            self.decoded_digest.as_str(),
        ] {
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(0);
        }
        Sha256::hex(&bytes)
    }

    fn storage_value(&self) -> Value {
        match &self.storage {
            RawSourceStorage::Standalone => json!({"kind":"standalone"}),
            RawSourceStorage::EmbeddedCampaign { campaign_key } => json!({
                "kind":"embedded_campaign",
                "campaign_key_digest": Sha256::hex(campaign_key),
            }),
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "source_key": self.source_key(),
            "session_nonce": self.session_nonce,
            "process_start_identity": self.process_start_identity,
            "session_path": self.source_path,
            "stored_digest": self.stored_digest,
            "decoded_digest": self.decoded_digest,
            "record_seq_bounds": [self.record_seq_bounds.0, self.record_seq_bounds.1],
            "byte_bounds": [self.byte_bounds.0, self.byte_bounds.1],
            "storage": self.storage_value(),
            "participations": self.participations.iter().map(SourceParticipation::to_value).collect::<Vec<_>>(),
        })
    }
}

/// Fail-closed decoder/build/modset/static-catalog compatibility result for one
/// campaign participation. `compatibility_class` is the decoder-proven target
/// class; source bindings may differ between segments, but every retained segment
/// must carry a non-empty proof for the same class and the proof must repeat its
/// exact source bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityResolution {
    Proven {
        evidence_id: String,
        compatibility_class: String,
        build_exe_sha256: String,
        modset_id: String,
        schema_version: String,
        static_catalog_digest: String,
    },
    Missing {
        reason: String,
    },
    Conflict {
        reason: String,
    },
    Uncertain {
        reason: String,
    },
}

impl CompatibilityResolution {
    fn to_value(&self) -> Value {
        match self {
            CompatibilityResolution::Proven {
                evidence_id,
                compatibility_class,
                build_exe_sha256,
                modset_id,
                schema_version,
                static_catalog_digest,
            } => json!({
                "kind": "proven", "evidence_id": evidence_id,
                "compatibility_class": compatibility_class,
                "build_exe_sha256": build_exe_sha256, "modset_id": modset_id,
                "schema_version": schema_version,
                "static_catalog_digest": static_catalog_digest,
            }),
            CompatibilityResolution::Missing { reason } => {
                json!({"kind":"missing","reason":reason})
            }
            CompatibilityResolution::Conflict { reason } => {
                json!({"kind":"conflict","reason":reason})
            }
            CompatibilityResolution::Uncertain { reason } => {
                json!({"kind":"uncertain","reason":reason})
            }
        }
    }
}

// ===========================================================================
// Section 3 — coverage dispositions & segment model
// ===========================================================================

/// The single coverage disposition of one complete observed hour within a segment
/// (design section 3). Every complete observed hour has exactly one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageDisposition {
    /// A published snapshot for the hour. `sha256` is the snapshot-body hash
    /// (hex); the *canonical container filename is always derived* from the
    /// segment's assigned global ordinal + hour via
    /// [`PlannedSegment::snapshot_file`] — never stored here, so it cannot drift
    /// from the ordinal the stitch actually assigns.
    Snapshot { sha256: String },
    /// The hour was observed complete but capture/fold failed. Carries a reason
    /// and an evidence id so the failure is auditable, never invented.
    CaptureFailed { reason: String, evidence_id: String },
    /// The hour's requested slot was coalesced into another observation.
    Coalesced { reason: String, evidence_id: String },
    /// An explicit, reason-coded coverage gap (e.g. a forward time jump or a
    /// cross-session gap). **Not** something the stitcher may invent — it must be
    /// present in the segment coverage before the supersede algorithm runs.
    Gap { reason: String, evidence_id: String },
}

impl CoverageDisposition {
    /// The kind tag used in the serialized coverage/index (`snapshot`,
    /// `capture_failed`, `coalesced`, `gap`).
    pub fn kind(&self) -> &'static str {
        match self {
            CoverageDisposition::Snapshot { .. } => "snapshot",
            CoverageDisposition::CaptureFailed { .. } => "capture_failed",
            CoverageDisposition::Coalesced { .. } => "coalesced",
            CoverageDisposition::Gap { .. } => "gap",
        }
    }
    /// A visible snapshot contributes a playback file; every other disposition is
    /// an explicit main-timeline gap (design section 6).
    pub fn is_snapshot(&self) -> bool {
        matches!(self, CoverageDisposition::Snapshot { .. })
    }

    fn to_value(&self) -> Value {
        match self {
            CoverageDisposition::Snapshot { sha256 } => {
                json!({ "kind": "snapshot", "sha256": sha256 })
            }
            CoverageDisposition::CaptureFailed {
                reason,
                evidence_id,
            } => json!({ "kind": "capture_failed", "reason": reason, "evidence_id": evidence_id }),
            CoverageDisposition::Coalesced {
                reason,
                evidence_id,
            } => json!({ "kind": "coalesced", "reason": reason, "evidence_id": evidence_id }),
            CoverageDisposition::Gap {
                reason,
                evidence_id,
            } => json!({ "kind": "gap", "reason": reason, "evidence_id": evidence_id }),
        }
    }
}

/// Exact, stable owner of one complete observation. The owner repeats the
/// authoritative ticket plus segment/epoch binding and is validated byte-for-byte;
/// it is never inferred later from a vector position or assigned global ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationOwner {
    pub ticket: AppendTicket,
    pub in_session_ordinal: u32,
    pub epoch_token: String,
}

impl ObservationOwner {
    fn to_value(&self) -> Value {
        json!({
            "campaign_key_digest": Sha256::hex(&self.ticket.campaign_key),
            "play_order": self.ticket.play_order,
            "session_nonce": self.ticket.session_nonce,
            "process_start_identity": self.ticket.process_start_identity,
            "in_session_ordinal": self.in_session_ordinal,
            "epoch_token": self.epoch_token,
        })
    }
}

/// One complete observation of one game hour, bound to exact decoded source
/// sequence/byte evidence. Repeated observations are all retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourObservation {
    pub observation_id: String,
    pub owner: ObservationOwner,
    /// Inclusive decoded record-sequence bounds of the complete logical frame.
    pub record_seq_bounds: (u64, u64),
    /// Half-open decoded byte bounds of the complete logical frame.
    pub byte_bounds: (u64, u64),
    pub disposition: CoverageDisposition,
}

impl HourObservation {
    fn to_value(&self) -> Value {
        let mut value = self.disposition.to_value().as_object().unwrap().clone();
        value.insert("observation_id".into(), json!(self.observation_id));
        value.insert("owner".into(), self.owner.to_value());
        value.insert(
            "record_seq_bounds".into(),
            json!([self.record_seq_bounds.0, self.record_seq_bounds.1]),
        );
        value.insert(
            "byte_bounds".into(),
            json!([self.byte_bounds.0, self.byte_bounds.1]),
        );
        Value::Object(value)
    }
}

/// Why a segment started / stopped (design section 5). Recorded as segment
/// provenance and surfaced in `segments.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentReason {
    /// First complete frame of a new session / process-start identity.
    SessionStart,
    /// A verified save-epoch token changed.
    EpochChange,
    /// The collector reported reload may have begun (uncertainty seal).
    ReloadUncertain,
    /// `frame.hour < current head` — a backward-time rollback cut.
    TimeRollback,
    /// The session/stream ended cleanly.
    SessionEnd,
    /// The segment is still open (used only pre-seal).
    Open,
}

impl SegmentReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SegmentReason::SessionStart => "session_start",
            SegmentReason::EpochChange => "epoch_change",
            SegmentReason::ReloadUncertain => "reload_uncertain",
            SegmentReason::TimeRollback => "time_rollback",
            SegmentReason::SessionEnd => "session_end",
            SegmentReason::Open => "open",
        }
    }
}

/// Immutable provenance a segment carries into the stitch (design sections 5 & 7.2).
///
/// This is segment *provenance*, never a substitute for campaign identity: build /
/// modset / schema / epoch bindings describe the bytes, they do not decide branch
/// selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentProvenance {
    pub session_nonce: String,
    pub process_start_identity: String,
    pub epoch_token: String,
    pub stream_major: u16,
    pub build_exe_sha256: Option<String>,
    pub modset_id: Option<String>,
    pub schema_version: Option<String>,
    pub static_catalog_digest: Option<String>,
    /// Byte-for-byte session source path (design section 7); one entry per session.
    pub source_path: Option<String>,
    /// SHA-256 of the *stored* (possibly compressed) session entry, hex.
    pub source_stored_digest: Option<String>,
    /// SHA-256 of the *decoded* stream bytes, hex. Never a compressed offset.
    pub source_decoded_digest: Option<String>,
    /// Decoded record-sequence bounds `[start, end]` of this segment in its source.
    pub record_seq_bounds: Option<(u64, u64)>,
    /// Decoded byte bounds `[start, end)` of this segment in its source.
    pub byte_bounds: Option<(u64, u64)>,
    pub start_reason: Option<SegmentReason>,
    pub stop_reason: Option<SegmentReason>,
}

/// A sealed segment ready for stitching: a maximal nondecreasing-hour run within
/// one session and one save epoch, with its coverage map and identity result.
///
/// Invariants enforced by [`SegmentInput::validate`]:
/// * coverage keys are the complete observed hours;
/// * `cutover` = min coverage hour, `head` = max coverage hour (both `None` for an
///   empty recovery segment);
/// * coverage is contiguous over `cutover..=head` (missing hour = corrupt input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentInput {
    /// Strictly-increasing campaign-local order in which sessions were played.
    /// Segments from one session share a `play_order`; `in_session_ordinal`
    /// breaks the tie deterministically.
    pub play_order: u64,
    /// Deterministic ordinal within one session (0-based, monotonic).
    pub in_session_ordinal: u32,
    /// Fail-closed identity of this segment's save epoch.
    pub identity: CampaignIdentityResolution,
    /// The append ticket bound to this segment's campaign participation. `None`
    /// means the bytes were observed while identity was unresolved.
    pub ticket: Option<AppendTicket>,
    /// Explicit fail-closed compatibility proof. Campaign identity is not a
    /// substitute for decoder/build/modset/static-catalog compatibility.
    pub compatibility: CompatibilityResolution,
    /// Coverage owned by this segment at a cross-session forward transition,
    /// before its first observed hour. This ledger is distinct from observed-hour
    /// coverage and must exactly cover any gap from the prior main head.
    pub transition_coverage: BTreeMap<i64, CoverageDisposition>,
    /// Segment-local coverage: complete observed hour -> disposition.
    pub coverage: BTreeMap<i64, CoverageDisposition>,
    /// Every complete observation for each hour, in decoded source-sequence
    /// evidence order after normalization. The selected disposition in coverage
    /// must equal the observation with the greatest record sequence.
    pub observations: BTreeMap<i64, Vec<HourObservation>>,
    /// Complete campaign-independent raw-source identity/participation ledger.
    /// None is representable only so corrupt/imported inputs can fail closed.
    pub raw_source: Option<RawSourceDescriptor>,
    /// Immutable provenance.
    pub provenance: SegmentProvenance,
}

impl SegmentInput {
    /// The first observed (owned) hour, or `None` for an empty recovery segment.
    pub fn cutover(&self) -> Option<i64> {
        self.coverage.keys().next().copied()
    }
    /// The greatest observed hour, or `None` for an empty recovery segment.
    pub fn head(&self) -> Option<i64> {
        self.coverage.keys().next_back().copied()
    }
    /// True for a segment with no complete frame — retained in the container but
    /// excluded from the supersede calculation (design section 6).
    pub fn is_empty_recovery(&self) -> bool {
        self.coverage.is_empty()
    }

    fn observation_owner(&self) -> Result<ObservationOwner, String> {
        let ticket = self.ticket.clone().ok_or_else(|| {
            "retained segment lacks an AppendTicket (unbound observation owner)".to_string()
        })?;
        Ok(ObservationOwner {
            ticket,
            in_session_ordinal: self.in_session_ordinal,
            epoch_token: self.provenance.epoch_token.clone(),
        })
    }

    fn selected_observation(&self, hour: i64) -> Result<&HourObservation, String> {
        self.observations
            .get(&hour)
            .and_then(|observations| {
                observations
                    .iter()
                    .max_by_key(|observation| observation.record_seq_bounds.1)
            })
            .ok_or_else(|| format!("missing observation evidence for hour {hour}"))
    }

    fn normalize(&mut self) {
        for observations in self.observations.values_mut() {
            observations.sort_by(|left, right| {
                (
                    left.record_seq_bounds,
                    left.byte_bounds,
                    left.observation_id.as_str(),
                )
                    .cmp(&(
                        right.record_seq_bounds,
                        right.byte_bounds,
                        right.observation_id.as_str(),
                    ))
            });
        }
        if let Some(source) = self.raw_source.as_mut() {
            source.participations.sort_by(|left, right| {
                (
                    left.record_seq_bounds,
                    left.byte_bounds,
                    left.ticket.play_order,
                    left.ticket.campaign_key.as_slice(),
                )
                    .cmp(&(
                        right.record_seq_bounds,
                        right.byte_bounds,
                        right.ticket.play_order,
                        right.ticket.campaign_key.as_slice(),
                    ))
            });
        }
    }

    /// Structural validation independent of stitching. Returns the first problem.
    pub fn validate(&self) -> Result<(), String> {
        for (hour, disposition) in &self.transition_coverage {
            if disposition.is_snapshot() {
                return Err(format!(
                    "segment (play_order {}, ord {}) transition coverage cannot contain snapshots",
                    self.play_order, self.in_session_ordinal
                ));
            }
            validate_coverage_disposition(disposition, None).map_err(|reason| {
                format!(
                    "segment (play_order {}, ord {}) transition hour {hour} has invalid evidence: {reason}",
                    self.play_order, self.in_session_ordinal
                )
            })?;
        }
        if self.coverage.is_empty() {
            if !self.observations.is_empty() {
                return Err(format!(
                    "segment (play_order {}, ord {}) has observations without coverage",
                    self.play_order, self.in_session_ordinal
                ));
            }
            if !self.transition_coverage.is_empty() {
                return Err(format!(
                    "segment (play_order {}, ord {}) is empty recovery but carries transition coverage without a cutover",
                    self.play_order, self.in_session_ordinal
                ));
            }
            return Ok(()); // empty recovery segment is structurally legal
        }
        if self.coverage.keys().copied().collect::<Vec<_>>()
            != self.observations.keys().copied().collect::<Vec<_>>()
        {
            return Err(format!(
                "segment (play_order {}, ord {}) coverage/observation hour sets disagree",
                self.play_order, self.in_session_ordinal
            ));
        }
        let cutover = self.cutover().unwrap();
        let head = self.head().unwrap();
        // Contiguity is checked over retained keys instead of iterating the
        // attacker-controlled numeric span. This makes an i64::MIN..=i64::MAX
        // corrupt input a bounded rejection rather than a validation hang.
        let coverage_hours: Vec<i64> = self.coverage.keys().copied().collect();
        for adjacent in coverage_hours.windows(2) {
            if adjacent[0].checked_add(1) != Some(adjacent[1]) {
                return Err(format!(
                    "segment (play_order {}, ord {}) missing coverage between hours {} and {} in {cutover}..={head}",
                    self.play_order, self.in_session_ordinal,
                    adjacent[0], adjacent[1]
                ));
            }
        }
        for h in coverage_hours {
            let observations = self.observations.get(&h).ok_or_else(|| {
                format!(
                    "segment (play_order {}, ord {}) missing observation evidence for hour {h}",
                    self.play_order, self.in_session_ordinal
                )
            })?;
            if observations.is_empty() {
                return Err(format!("hour {h} has zero complete observations"));
            }
            let mut previous_record_end: Option<u64> = None;
            let mut previous_byte_end: Option<u64> = None;
            let mut evidence_ids = std::collections::BTreeSet::new();
            for observation in observations {
                if observation.observation_id.is_empty()
                    || !evidence_ids.insert(observation.observation_id.as_str())
                {
                    return Err(format!(
                        "hour {h} has missing/duplicate observation evidence id"
                    ));
                }
                if !is_evidence_id(&observation.observation_id) {
                    return Err(format!(
                        "hour {h} observation evidence id has invalid grammar"
                    ));
                }
                let (record_start, record_end) = observation.record_seq_bounds;
                let (byte_start, byte_end) = observation.byte_bounds;
                if record_start > record_end || byte_start >= byte_end {
                    return Err(format!("hour {h} observation has invalid source bounds"));
                }
                if previous_record_end.is_some_and(|end| record_start <= end)
                    || previous_byte_end.is_some_and(|end| byte_start < end)
                {
                    return Err(format!(
                        "hour {h} observations have ambiguous/overlapping source order"
                    ));
                }
                validate_coverage_disposition(
                    &observation.disposition,
                    Some(&observation.observation_id),
                )
                .map_err(|reason| format!("hour {h} disposition is invalid: {reason}"))?;
                previous_record_end = Some(record_end);
                previous_byte_end = Some(byte_end);
            }
            let selected = self.selected_observation(h)?;
            if self.coverage.get(&h) != Some(&selected.disposition) {
                return Err(format!(
                    "hour {h} selected disposition is not the last complete observation in source sequence"
                ));
            }
        }
        Ok(())
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_evidence_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 256
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn is_reason_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn validate_coverage_disposition(
    disposition: &CoverageDisposition,
    expected_evidence_id: Option<&str>,
) -> Result<(), String> {
    match disposition {
        CoverageDisposition::Snapshot { sha256 } => {
            if !is_sha256_hex(sha256) {
                return Err("snapshot digest is not canonical lowercase SHA-256".into());
            }
        }
        CoverageDisposition::CaptureFailed {
            reason,
            evidence_id,
        }
        | CoverageDisposition::Coalesced {
            reason,
            evidence_id,
        }
        | CoverageDisposition::Gap {
            reason,
            evidence_id,
        } => {
            if !is_reason_text(reason) {
                return Err("reason is empty, padded, oversized, or contains controls".into());
            }
            if !is_evidence_id(evidence_id) {
                return Err("evidence_id has invalid grammar".into());
            }
            if expected_evidence_id.is_some_and(|expected| expected != evidence_id) {
                return Err("evidence_id disagrees with observation evidence".into());
            }
        }
    }
    Ok(())
}

fn validate_raw_source_for_segment(seg: &SegmentInput) -> Result<(), String> {
    let source = seg
        .raw_source
        .as_ref()
        .ok_or_else(|| "mandatory raw_source descriptor is missing".to_string())?;
    let ticket = seg
        .ticket
        .as_ref()
        .ok_or_else(|| "retained segment lacks an AppendTicket".to_string())?;
    if source.session_nonce.is_empty()
        || source.process_start_identity.is_empty()
        || source.source_path.is_empty()
        || !is_sha256_hex(&source.stored_digest)
        || !is_sha256_hex(&source.decoded_digest)
    {
        return Err("raw source has missing/invalid mandatory identity or digest input".into());
    }
    if source.session_nonce != ticket.session_nonce
        || source.process_start_identity != ticket.process_start_identity
        || source.session_nonce != seg.provenance.session_nonce
        || source.process_start_identity != seg.provenance.process_start_identity
    {
        return Err("raw source session/process identity disagrees with exact ticket owner".into());
    }
    if source.record_seq_bounds.0 > source.record_seq_bounds.1
        || source.byte_bounds.0 >= source.byte_bounds.1
    {
        return Err("raw source has invalid record/byte bounds".into());
    }
    if source.participations.is_empty() {
        return Err("raw source participation ledger is missing".into());
    }
    let mut previous_record_end: Option<u64> = None;
    let mut previous_byte_end: Option<u64> = None;
    let mut participation_ids = std::collections::BTreeSet::new();
    for participation in &source.participations {
        if participation.ticket.session_nonce != source.session_nonce
            || participation.ticket.process_start_identity != source.process_start_identity
        {
            return Err(
                "raw source participation ticket has a foreign session/process owner".into(),
            );
        }
        let (record_start, record_end) = participation.record_seq_bounds;
        let (byte_start, byte_end) = participation.byte_bounds;
        if record_start > record_end
            || byte_start >= byte_end
            || record_start < source.record_seq_bounds.0
            || record_end > source.record_seq_bounds.1
            || byte_start < source.byte_bounds.0
            || byte_end > source.byte_bounds.1
        {
            return Err("raw source participation has invalid/out-of-source bounds".into());
        }
        match (previous_record_end, previous_byte_end) {
            (None, None) => {
                if record_start != source.record_seq_bounds.0 || byte_start != source.byte_bounds.0
                {
                    return Err(
                        "raw source participation ledger does not start at the source extent"
                            .into(),
                    );
                }
            }
            (Some(prior_record_end), Some(prior_byte_end)) => {
                if prior_record_end.checked_add(1) != Some(record_start)
                    || prior_byte_end != byte_start
                {
                    return Err(
                        "raw source participations do not form one exact contiguous partition"
                            .into(),
                    );
                }
            }
            _ => unreachable!("record and byte participation cursors advance together"),
        }
        if !participation_ids.insert(participation.participation_id()) {
            return Err("raw source has duplicate participation evidence".into());
        }
        previous_record_end = Some(record_end);
        previous_byte_end = Some(byte_end);
    }
    if previous_record_end != Some(source.record_seq_bounds.1)
        || previous_byte_end != Some(source.byte_bounds.1)
    {
        return Err("raw source participation ledger does not cover the source tail".into());
    }
    if let RawSourceStorage::EmbeddedCampaign { campaign_key } = &source.storage {
        if campaign_key.is_empty()
            || source
                .participations
                .iter()
                .any(|participation| participation.ticket.campaign_key != *campaign_key)
        {
            return Err(
                "embedded raw source contains cross-campaign participation; keep it standalone"
                    .into(),
            );
        }
        if campaign_key != &ticket.campaign_key {
            return Err("embedded raw source owner disagrees with stitched campaign".into());
        }
    }

    let matching: Vec<&SourceParticipation> = source
        .participations
        .iter()
        .filter(|participation| participation.ticket == *ticket)
        .collect();
    if matching.len() != 1 {
        return Err(format!(
            "segment ticket maps to {} raw-source participations; exact ownership requires one",
            matching.len()
        ));
    }
    let participation = matching[0];
    match &seg.identity {
        CampaignIdentityResolution::Verified {
            scheme_major,
            campaign_key,
            evidence_digest,
        } if participation.scheme_major == *scheme_major
            && participation.ticket.campaign_key == *campaign_key
            && participation.identity_evidence_digest == *evidence_digest => {}
        _ => {
            return Err(
                "raw-source participation identity evidence disagrees with segment identity".into(),
            );
        }
    }

    let record_bounds = seg
        .provenance
        .record_seq_bounds
        .ok_or_else(|| "mandatory segment record_seq_bounds input is missing".to_string())?;
    let byte_bounds = seg
        .provenance
        .byte_bounds
        .ok_or_else(|| "mandatory segment byte_bounds input is missing".to_string())?;
    if record_bounds.0 > record_bounds.1
        || byte_bounds.0 >= byte_bounds.1
        || record_bounds.0 < participation.record_seq_bounds.0
        || record_bounds.1 > participation.record_seq_bounds.1
        || byte_bounds.0 < participation.byte_bounds.0
        || byte_bounds.1 > participation.byte_bounds.1
    {
        return Err("segment source bounds escape its exact campaign participation".into());
    }
    if seg.provenance.source_path.as_deref() != Some(source.source_path.as_str())
        || seg.provenance.source_stored_digest.as_deref() != Some(source.stored_digest.as_str())
        || seg.provenance.source_decoded_digest.as_deref() != Some(source.decoded_digest.as_str())
    {
        return Err("segment source reference disagrees with raw-source descriptor".into());
    }
    let mut all_observation_bounds = Vec::new();
    for (hour, observations) in &seg.observations {
        for observation in observations {
            if observation.owner != seg.observation_owner()? {
                return Err(format!(
                    "hour {hour} observation owner disagrees with exact ticket/segment/epoch binding"
                ));
            }
            if observation.record_seq_bounds.0 < record_bounds.0
                || observation.record_seq_bounds.1 > record_bounds.1
                || observation.byte_bounds.0 < byte_bounds.0
                || observation.byte_bounds.1 > byte_bounds.1
            {
                return Err(format!(
                    "hour {hour} observation evidence escapes segment source bounds"
                ));
            }
            all_observation_bounds.push((
                observation.record_seq_bounds,
                observation.byte_bounds,
                *hour,
                observation.observation_id.as_str(),
            ));
        }
    }
    all_observation_bounds.sort_by(|left, right| {
        (left.0, left.1, left.2, left.3).cmp(&(right.0, right.1, right.2, right.3))
    });
    for adjacent in all_observation_bounds.windows(2) {
        if adjacent[1].0 .0 <= adjacent[0].0 .1 || adjacent[1].1 .0 < adjacent[0].1 .1 {
            return Err(format!(
                "observation evidence overlaps across hours {} and {}",
                adjacent[0].2, adjacent[1].2
            ));
        }
    }
    Ok(())
}

// ===========================================================================
// Section 6 — exact supersede & visibility algorithm
// ===========================================================================

/// Ownership of one main-timeline hour: which global segment owns it and its
/// disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainHour {
    pub segment_id: u32,
    pub disposition: CoverageDisposition,
    pub owner: ObservationOwner,
    /// Present for an observed hour; transition-only coverage has no logical
    /// frame observation but still carries the exact segment owner above.
    pub selected_observation: Option<HourObservation>,
}

/// A contiguous run of a segment's hours that a later segment removed from the
/// main timeline (design section 6). Retained, never deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededSpan {
    /// The segment whose hours were superseded.
    pub segment_id: u32,
    pub start_hour: i64,
    pub end_hour: i64,
    /// The later segment that first superseded this span.
    pub superseded_by: u32,
}

/// A contiguous run of a segment's hours that remain visible in the main timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSpan {
    pub segment_id: u32,
    pub start_hour: i64,
    pub end_hour: i64,
}

/// A main-branch slice attributed to one owning segment (ascending, coalesced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainSlice {
    pub segment_id: u32,
    pub start_hour: i64,
    pub end_hour: i64,
}

/// The deterministic result of the supersede algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersedeResult {
    /// The final main timeline, ascending by hour.
    pub main: BTreeMap<i64, MainHour>,
    /// Greatest hour on the main timeline (`None` if no segment has any hour).
    pub main_head: Option<i64>,
    /// Every superseded span, coalesced, in `(segment_id, start_hour)` order.
    pub superseded: Vec<SupersededSpan>,
    /// Every visible span, coalesced, in `(segment_id, start_hour)` order.
    pub visible: Vec<VisibleSpan>,
    /// Ordered main-branch slices by owning segment.
    pub main_slices: Vec<MainSlice>,
    /// Exact parent captured immediately before each segment's cutover mutation.
    pub insertion_parents: BTreeMap<u32, Option<u32>>,
}

/// Run the normative supersede algorithm (design section 6) over segments that are
/// **already ordered** by `(play_order, in_session_ordinal)` and carry global
/// ordinals equal to their index in that order.
///
/// Empty recovery segments must be excluded by the caller (they are retained in the
/// container but do not participate here).
///
/// Returns `Err` on corrupt input (a segment missing coverage for an owned hour) —
/// the stitcher never invents a gap (self-refutation #3).
fn run_supersede(ordered: &[(u32, &SegmentInput)]) -> Result<SupersedeResult, String> {
    // main: hour -> (owner segment id, disposition). We also track, per hour, the
    // owner so a later cutover can attribute the removed span to the superseding
    // segment.
    let mut main: BTreeMap<i64, MainHour> = BTreeMap::new();
    // superseded events recorded as (segment_id, hour, superseded_by); coalesced
    // into spans after the loop for determinism and compactness.
    let mut superseded_events: Vec<(u32, i64, u32)> = Vec::new();
    let mut main_head: Option<i64> = None;
    let mut insertion_parents = BTreeMap::new();

    for (gid, seg) in ordered {
        let cutover = match seg.cutover() {
            Some(c) => c,
            None => continue, // defensive: empty recovery excluded by caller
        };
        let head = seg.head().unwrap();

        // Parentage is insertion evidence, not a graph inferred from the final
        // branch. Capture it before transition insertion and cutover deletion.
        let parent = main
            .range(..cutover)
            .next_back()
            .map(|(_, main_hour)| main_hour.segment_id);
        insertion_parents.insert(*gid, parent);

        // A forward transition beyond the previous main head must be represented
        // exactly by the later segment's explicit transition ledger. Missing,
        // extra, or misplaced rows are a fail-closed accounting error.
        let required_transition = main_head.and_then(|previous_head| {
            (cutover > previous_head.saturating_add(1))
                .then_some((previous_head.saturating_add(1), cutover - 1))
        });
        match required_transition {
            Some((start, end)) => {
                let actual: Vec<i64> = seg.transition_coverage.keys().copied().collect();
                let expected: Vec<i64> = (start..=end).collect();
                if actual != expected {
                    return Err(format!(
                        "segment {gid}: transition coverage must exactly account for {start}..={end}"
                    ));
                }
                for h in start..=end {
                    let owner = seg.observation_owner()?;
                    main.insert(
                        h,
                        MainHour {
                            segment_id: *gid,
                            disposition: seg.transition_coverage[&h].clone(),
                            owner,
                            selected_observation: None,
                        },
                    );
                }
            }
            None if !seg.transition_coverage.is_empty() => {
                return Err(format!(
                    "segment {gid}: transition coverage present without a cross-session gap"
                ));
            }
            None => {}
        }

        // delete from main every hour h >= cutover(S), attributing each removed
        // hour to THIS segment as the first superseder.
        let removed: Vec<i64> = main.range(cutover..).map(|(h, _)| *h).collect();
        for h in removed {
            let owner = main.remove(&h).unwrap();
            superseded_events.push((owner.segment_id, h, *gid));
        }

        // add this segment's coverage over cutover..=head. A missing hour is
        // corrupt input.
        for h in cutover..=head {
            let disp = seg.coverage.get(&h).ok_or_else(|| {
                format!("segment {gid}: missing coverage for owned hour {h} (corrupt input)")
            })?;
            let selected = seg.selected_observation(h)?.clone();
            main.insert(
                h,
                MainHour {
                    segment_id: *gid,
                    disposition: disp.clone(),
                    owner: selected.owner.clone(),
                    selected_observation: Some(selected),
                },
            );
        }
        main_head = Some(head);
    }

    // final truncation: delete every hour h > main_head. With valid segments
    // (cutover <= head) the cutover-delete already removed these, so this is a
    // defensive no-op; if it ever fires, attribute to the last segment.
    if let (Some(mh), Some((last_gid, _))) = (main_head, ordered.last()) {
        let tail: Vec<i64> = main.range((mh + 1)..).map(|(h, _)| *h).collect();
        for h in tail {
            let owner = main.remove(&h).unwrap();
            superseded_events.push((owner.segment_id, h, *last_gid));
        }
    }

    let superseded = coalesce_superseded(&mut superseded_events);
    let visible = coalesce_visible(&main);
    let main_slices = coalesce_main_slices(&main);

    Ok(SupersedeResult {
        main,
        main_head,
        superseded,
        visible,
        main_slices,
        insertion_parents,
    })
}

/// Coalesce per-hour superseded events into contiguous spans, deterministically
/// ordered by `(segment_id, start_hour)`. Adjacent hours coalesce only when the
/// owner segment AND the `superseded_by` owner match (design section 8 rule 6).
fn coalesce_superseded(events: &mut [(u32, i64, u32)]) -> Vec<SupersededSpan> {
    // sort by (segment_id, hour) so runs are contiguous and order is canonical.
    events.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    let mut out: Vec<SupersededSpan> = Vec::new();
    for &(seg, hour, by) in events.iter() {
        if let Some(last) = out.last_mut() {
            if last.segment_id == seg && last.superseded_by == by && last.end_hour + 1 == hour {
                last.end_hour = hour;
                continue;
            }
        }
        out.push(SupersededSpan {
            segment_id: seg,
            start_hour: hour,
            end_hour: hour,
            superseded_by: by,
        });
    }
    out
}

/// Coalesce the main map's visible ownership into per-segment contiguous spans.
fn coalesce_visible(main: &BTreeMap<i64, MainHour>) -> Vec<VisibleSpan> {
    // build owner runs in ascending hour, then re-sort by (segment_id, start_hour).
    let mut runs: Vec<VisibleSpan> = Vec::new();
    for (h, mh) in main.iter() {
        if let Some(last) = runs.last_mut() {
            if last.segment_id == mh.segment_id && last.end_hour + 1 == *h {
                last.end_hour = *h;
                continue;
            }
        }
        runs.push(VisibleSpan {
            segment_id: mh.segment_id,
            start_hour: *h,
            end_hour: *h,
        });
    }
    runs.sort_by(|a, b| (a.segment_id, a.start_hour).cmp(&(b.segment_id, b.start_hour)));
    runs
}

/// Ordered (ascending-hour) main-branch slices by owning segment.
fn coalesce_main_slices(main: &BTreeMap<i64, MainHour>) -> Vec<MainSlice> {
    let mut runs: Vec<MainSlice> = Vec::new();
    for (h, mh) in main.iter() {
        if let Some(last) = runs.last_mut() {
            if last.segment_id == mh.segment_id && last.end_hour + 1 == *h {
                last.end_hour = *h;
                continue;
            }
        }
        runs.push(MainSlice {
            segment_id: mh.segment_id,
            start_hour: *h,
            end_hour: *h,
        });
    }
    runs
}

// ===========================================================================
// Sections 4 & 10 — fail-closed identity / play-order gate + plan
// ===========================================================================

/// The outcome of a stitch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StitchOutcome {
    /// The segments form one verified campaign with a proven play order: a single
    /// stitched replay.
    Stitched(StitchedPlan),
    /// Fail-closed refusal: keep sessions separate. `reason` names why (unbound /
    /// missing / ambiguous / conflicting identity, unprovable order, mixed major).
    Separate { reason: String },
}

/// The immutable, deterministic stitch model: the main timeline plus the full
/// retained segment graph (design section 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StitchedPlan {
    pub game_id: String,
    pub required_inputs: StitchRequiredInputs,
    pub scheme_major: u16,
    /// Exact opaque campaign-key bytes needed for independent digest replay.
    pub campaign_key: Vec<u8>,
    pub campaign_key_digest: String,
    pub stream_major: u16,
    /// Digest binding every stitch input (play order + identity + coverage).
    pub stitch_input_digest: String,
    /// Digest of the ordered play-order ledger.
    pub play_order_ledger_digest: String,
    /// Segments in global-ordinal order (== `(play_order, in_session_ordinal)`),
    /// including empty recovery segments (retained, no visible span).
    pub segments: Vec<PlannedSegment>,
    pub supersede: SupersedeResult,
}

/// One segment as placed in the stitched plan, with its global ordinal and its
/// normalized visible / superseded spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSegment {
    pub segment_id: u32,
    pub input: SegmentInput,
    /// The segment that owns the greatest main-timeline hour below this segment's
    /// cutover at insertion time (design section 7.2). Explanatory only.
    pub parent_segment: Option<u32>,
    pub visible_spans: Vec<VisibleSpan>,
    pub superseded_spans: Vec<SupersededSpan>,
}

impl PlannedSegment {
    /// Canonical segment-prefixed snapshot filename (design section 7):
    /// `s<segment_id:08>.<hour:012>.snapshot.json`.
    pub fn snapshot_file(segment_id: u32, hour_index: i64) -> String {
        format!("s{segment_id:08}.{hour_index:012}.snapshot.json")
    }
    /// Per-segment index path (design section 7.3).
    pub fn index_path(segment_id: u32) -> String {
        format!("segments/s{segment_id:08}.index.json")
    }
}

/// Attempt to stitch `segments` into one replay, fail-closed per design sections 4
/// & 10.
///
/// The caller supplies segments in any order; this function establishes the
/// deterministic play order and assigns global ordinals. It refuses (returns
/// [`StitchOutcome::Separate`]) when:
///
/// * any participating (non-empty) segment's identity is not `Verified`
///   (unbound / missing / ambiguous / conflicting) — cross-session stitching is
///   disabled while identity is unbound;
/// * verified `scheme_major` values differ or `campaign_key` bytes differ;
/// * any participating segment lacks an [`AppendTicket`], or a ticket's
///   `campaign_key` disagrees with its identity;
/// * play-order ordinals collide or cannot form a proven total order;
/// * stream majors are mixed;
/// * structural validation fails (a segment missing coverage for an owned hour).
///
/// `game_id` / `schema_version` are recorded on the plan; they are not identity
/// heuristics.
pub fn plan_stitch(
    game_id: &str,
    required_inputs: StitchRequiredInputs,
    mut segments: Vec<SegmentInput>,
) -> StitchOutcome {
    if game_id.is_empty() {
        return StitchOutcome::Separate {
            reason: "mandatory stitch input game_id is missing/empty".into(),
        };
    }
    if game_id == "."
        || game_id == ".."
        || game_id.len() > 128
        || !game_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return StitchOutcome::Separate {
            reason: "mandatory stitch input game_id has invalid path-component grammar".into(),
        };
    }
    if let Err(reason) = required_inputs.validate() {
        return StitchOutcome::Separate { reason };
    }
    for segment in &mut segments {
        segment.normalize();
        if let Some(path) = segment.provenance.source_path.as_mut() {
            match normalize_stitch_source_path(path) {
                Ok(canonical) => *path = canonical,
                Err(reason) => {
                    return StitchOutcome::Separate {
                        reason: format!("raw source path rejected: {reason}"),
                    };
                }
            }
        }
        if let Some(source) = segment.raw_source.as_mut() {
            match normalize_stitch_source_path(&source.source_path) {
                Ok(canonical) => source.source_path = canonical,
                Err(reason) => {
                    return StitchOutcome::Separate {
                        reason: format!("raw source path rejected: {reason}"),
                    };
                }
            }
        }
    }
    // 0. structural validation (independent of identity).
    for seg in &segments {
        if let Err(e) = seg.validate() {
            return StitchOutcome::Separate {
                reason: format!("corrupt segment coverage: {e}"),
            };
        }
    }

    // Participating segments are those with at least one complete frame. Empty
    // recovery segments are retained and excluded only from visibility placement;
    // identity, ticket, compatibility, stream-major and ordering validation below
    // covers ALL retained segments.
    let participating: Vec<&SegmentInput> =
        segments.iter().filter(|s| !s.is_empty_recovery()).collect();

    // If there is nothing to place, there is nothing to stitch.
    if participating.is_empty() {
        return StitchOutcome::Separate {
            reason: "no non-empty segment to place on a main timeline".into(),
        };
    }

    // 1. every retained segment must be Verified with equal scheme/campaign,
    // uniform stream major, an exact AppendTicket binding, and an explicit Proven
    // compatibility result.
    let mut scheme_major: Option<u16> = None;
    let mut campaign_key: Option<Vec<u8>> = None;
    let mut stream_major: Option<u16> = None;
    let mut compatibility_class: Option<String> = None;
    let mut raw_sources: BTreeMap<String, RawSourceDescriptor> = BTreeMap::new();
    let mut raw_source_paths: BTreeMap<String, String> = BTreeMap::new();
    let mut segment_extents_by_source: BTreeMap<String, Vec<((u64, u64), (u64, u64), u64, u32)>> =
        BTreeMap::new();
    for seg in &segments {
        match &seg.identity {
            CampaignIdentityResolution::Verified {
                scheme_major: sm,
                campaign_key: ck,
                ..
            } => {
                if *sm == 0 || ck.is_empty() {
                    return StitchOutcome::Separate {
                        reason: "verified campaign identity has an empty/zero mandatory binding"
                            .into(),
                    };
                }
                match scheme_major {
                    None => scheme_major = Some(*sm),
                    Some(prev) if prev != *sm => {
                        return StitchOutcome::Separate {
                            reason: format!(
                                "mixed campaign scheme_major ({prev} vs {sm}); never merge across identity schemes"
                            ),
                        };
                    }
                    _ => {}
                }
                match &campaign_key {
                    None => campaign_key = Some(ck.clone()),
                    Some(prev) if prev != ck => {
                        return StitchOutcome::Separate {
                            reason: "verified campaign keys differ; separate replays".into(),
                        };
                    }
                    _ => {}
                }
            }
            CampaignIdentityResolution::Missing { reason } => {
                return StitchOutcome::Separate {
                    reason: format!("identity Missing ({reason}); cross-session stitch disabled"),
                };
            }
            CampaignIdentityResolution::Ambiguous {
                reason,
                candidate_count,
            } => {
                return StitchOutcome::Separate {
                    reason: format!(
                        "identity Ambiguous ({reason}, {candidate_count} candidates); no guessed branch"
                    ),
                };
            }
            CampaignIdentityResolution::Conflict { reason } => {
                return StitchOutcome::Separate {
                    reason: format!("identity Conflict ({reason}); separate replays"),
                };
            }
        }
        // stream major must be uniform (design section 9: no mixed majors).
        let sm = seg.provenance.stream_major;
        if sm == 0 {
            return StitchOutcome::Separate {
                reason: "segment has zero stream major".into(),
            };
        }
        match stream_major {
            None => stream_major = Some(sm),
            Some(prev) if prev != sm => {
                return StitchOutcome::Separate {
                    reason: format!("mixed stream majors ({prev} vs {sm}); reject stitch"),
                };
            }
            _ => {}
        }

        let ticket = match &seg.ticket {
            None => {
                return StitchOutcome::Separate {
                    reason: "retained segment lacks an AppendTicket (unbound play order)".into(),
                };
            }
            Some(ticket) => ticket,
        };
        if ticket.campaign_key != *campaign_key.as_ref().unwrap() {
            return StitchOutcome::Separate {
                reason: "AppendTicket campaign_key disagrees with verified identity".into(),
            };
        }
        if ticket.play_order != seg.play_order {
            return StitchOutcome::Separate {
                reason: format!(
                    "AppendTicket play_order {} disagrees with duplicated SegmentInput play_order {}",
                    ticket.play_order, seg.play_order
                ),
            };
        }
        if ticket.session_nonce != seg.provenance.session_nonce {
            return StitchOutcome::Separate {
                reason: "AppendTicket session_nonce disagrees with segment provenance".into(),
            };
        }
        if ticket.process_start_identity != seg.provenance.process_start_identity {
            return StitchOutcome::Separate {
                reason: "AppendTicket process_start_identity disagrees with segment provenance"
                    .into(),
            };
        }

        match &seg.compatibility {
            CompatibilityResolution::Proven {
                evidence_id,
                compatibility_class: class,
                build_exe_sha256,
                modset_id,
                schema_version,
                static_catalog_digest,
            } => {
                if evidence_id.is_empty()
                    || class.is_empty()
                    || build_exe_sha256.is_empty()
                    || modset_id.is_empty()
                    || schema_version.is_empty()
                    || static_catalog_digest.is_empty()
                {
                    return StitchOutcome::Separate {
                        reason: "compatibility Proven result has an empty mandatory binding".into(),
                    };
                }
                let exact = seg.provenance.build_exe_sha256.as_deref()
                    == Some(build_exe_sha256.as_str())
                    && seg.provenance.modset_id.as_deref() == Some(modset_id.as_str())
                    && seg.provenance.schema_version.as_deref() == Some(schema_version.as_str())
                    && seg.provenance.static_catalog_digest.as_deref()
                        == Some(static_catalog_digest.as_str());
                if !exact {
                    return StitchOutcome::Separate {
                        reason:
                            "compatibility Proven source bindings disagree with segment provenance"
                                .into(),
                    };
                }
                match &compatibility_class {
                    None => compatibility_class = Some(class.clone()),
                    Some(previous) if previous != class => {
                        return StitchOutcome::Separate {
                            reason: format!(
                                "conflicting proven compatibility classes {previous:?} and {class:?}"
                            ),
                        };
                    }
                    _ => {}
                }
            }
            CompatibilityResolution::Missing { reason } => {
                return StitchOutcome::Separate {
                    reason: format!("compatibility Missing ({reason}); stitch disabled"),
                };
            }
            CompatibilityResolution::Conflict { reason } => {
                return StitchOutcome::Separate {
                    reason: format!("compatibility Conflict ({reason}); separate replays"),
                };
            }
            CompatibilityResolution::Uncertain { reason } => {
                return StitchOutcome::Separate {
                    reason: format!("compatibility Uncertain ({reason}); stitch disabled"),
                };
            }
        }
    }
    let scheme_major = scheme_major.unwrap();
    let campaign_key = campaign_key.unwrap();
    let stream_major = stream_major.unwrap();

    // 2. prove a strict total segment order. Multiple segments from one session
    // may share one exact ticket/play_order, but their in-session ordinals must be
    // unique. Distinct tickets may never claim the same play_order.
    let mut seen_ranks: BTreeMap<(u64, u32), ()> = BTreeMap::new();
    let mut order_to_ticket: BTreeMap<u64, AppendTicket> = BTreeMap::new();
    for seg in &segments {
        let ticket = seg.ticket.as_ref().expect("ticket validated above");
        if seen_ranks
            .insert((ticket.play_order, seg.in_session_ordinal), ())
            .is_some()
        {
            return StitchOutcome::Separate {
                reason: format!(
                    "duplicate (play_order {}, in_session_ordinal {}); ordering unprovable",
                    ticket.play_order, seg.in_session_ordinal
                ),
            };
        }
        if let Some(previous) = order_to_ticket.insert(ticket.play_order, ticket.clone()) {
            if previous != *ticket {
                return StitchOutcome::Separate {
                    reason: format!(
                        "play_order {} maps to conflicting AppendTickets; unprovable order",
                        ticket.play_order
                    ),
                };
            }
        }
    }

    // Source ownership is validated only after the ticket/order authority has
    // rejected collisions, so a malformed nested source cannot obscure the
    // primary ordering-corruption evidence.
    for seg in &segments {
        let ticket = seg.ticket.as_ref().expect("ticket validated above");
        if let Err(reason) = validate_raw_source_for_segment(seg) {
            return StitchOutcome::Separate {
                reason: format!("raw source rejected: {reason}"),
            };
        }
        let source = seg.raw_source.as_ref().expect("raw source validated above");
        let source_key = source.source_key();
        if let Some(previous_key) =
            raw_source_paths.insert(source.source_path.clone(), source_key.clone())
        {
            if previous_key != source_key {
                return StitchOutcome::Separate {
                    reason: format!(
                        "raw source path {:?} aliases distinct source identities",
                        source.source_path
                    ),
                };
            }
        }
        if let Some(previous) = raw_sources.insert(source_key.clone(), source.clone()) {
            if previous != *source {
                return StitchOutcome::Separate {
                    reason: format!(
                        "raw source {source_key} has conflicting ownership/participation ledgers"
                    ),
                };
            }
        }
        segment_extents_by_source
            .entry(source_key)
            .or_default()
            .push((
                seg.provenance.record_seq_bounds.unwrap(),
                seg.provenance.byte_bounds.unwrap(),
                ticket.play_order,
                seg.in_session_ordinal,
            ));
    }
    for (source_key, extents) in &mut segment_extents_by_source {
        extents.sort();
        for adjacent in extents.windows(2) {
            if adjacent[1].0 .0 <= adjacent[0].0 .1 || adjacent[1].1 .0 < adjacent[0].1 .1 {
                return StitchOutcome::Separate {
                    reason: format!(
                        "raw source {source_key} has overlapping segment extents ({}, {}) and ({}, {})",
                        adjacent[0].2, adjacent[0].3, adjacent[1].2, adjacent[1].3
                    ),
                };
            }
        }
    }

    // 4. deterministic order: (play_order, in_session_ordinal). Global ordinals are
    //    assigned by scanning THIS order from zero — including empty recovery
    //    segments so their retained payloads get stable ids too.
    let mut ordered_all: Vec<SegmentInput> = segments;
    ordered_all.sort_by(|a, b| {
        let at = a.ticket.as_ref().expect("ticket validated above");
        let bt = b.ticket.as_ref().expect("ticket validated above");
        (at.play_order, a.in_session_ordinal).cmp(&(bt.play_order, b.in_session_ordinal))
    });

    // participating (non-empty) segments with their assigned global ordinal.
    let mut ordered_participating: Vec<(u32, &SegmentInput)> = Vec::new();
    for (gid, seg) in ordered_all.iter().enumerate() {
        if !seg.is_empty_recovery() {
            ordered_participating.push((gid as u32, seg));
        }
    }

    // 5. run the supersede algorithm.
    let supersede = match run_supersede(&ordered_participating) {
        Ok(r) => r,
        Err(e) => {
            return StitchOutcome::Separate {
                reason: format!("supersede rejected: {e}"),
            };
        }
    };

    // 6. build per-segment placement (parent, visible/superseded spans).
    let visible_by_seg = group_visible(&supersede.visible);
    let superseded_by_seg = group_superseded(&supersede.superseded);
    let mut planned: Vec<PlannedSegment> = Vec::with_capacity(ordered_all.len());
    for (gid, seg) in ordered_all.iter().enumerate() {
        let gid = gid as u32;
        let parent = supersede.insertion_parents.get(&gid).copied().flatten();
        planned.push(PlannedSegment {
            segment_id: gid,
            input: seg.clone(),
            parent_segment: parent,
            visible_spans: visible_by_seg.get(&gid).cloned().unwrap_or_default(),
            superseded_spans: superseded_by_seg.get(&gid).cloned().unwrap_or_default(),
        });
    }

    // 7. digests binding the deterministic input tuple (design section 8).
    let campaign_key_digest = Sha256::hex(&campaign_key);
    let play_order_ledger_digest = compute_play_order_ledger_digest(&ordered_all);

    let mut plan = StitchedPlan {
        game_id: game_id.to_string(),
        required_inputs,
        scheme_major,
        campaign_key,
        campaign_key_digest,
        stream_major,
        stitch_input_digest: String::new(),
        play_order_ledger_digest,
        segments: planned,
        supersede,
    };
    let serialized_segment_indexes: Vec<Value> = plan
        .segments
        .iter()
        .map(|segment| plan.segment_index_value(segment))
        .collect();
    plan.stitch_input_digest = canonical_stitch_input_digest(
        &plan.game_id,
        &plan.segments_value(),
        &serialized_segment_indexes,
    );
    StitchOutcome::Stitched(plan)
}

fn group_visible(spans: &[VisibleSpan]) -> BTreeMap<u32, Vec<VisibleSpan>> {
    let mut m: BTreeMap<u32, Vec<VisibleSpan>> = BTreeMap::new();
    for s in spans {
        m.entry(s.segment_id).or_default().push(s.clone());
    }
    m
}
fn group_superseded(spans: &[SupersededSpan]) -> BTreeMap<u32, Vec<SupersededSpan>> {
    let mut m: BTreeMap<u32, Vec<SupersededSpan>> = BTreeMap::new();
    for s in spans {
        m.entry(s.segment_id).or_default().push(s.clone());
    }
    m
}

/// Digest over the ordered play-order ledger only (design section 7.2).
fn compute_play_order_ledger_digest(ordered: &[SegmentInput]) -> String {
    let rows: Vec<Value> = ordered
        .iter()
        .map(|s| {
            let ticket = s.ticket.as_ref().expect("ticket validated before digest");
            json!({
                "campaign_key_digest": Sha256::hex(&ticket.campaign_key),
                "play_order": ticket.play_order,
                "in_session_ordinal": s.in_session_ordinal,
                "session_nonce": ticket.session_nonce,
                "process_start_identity": ticket.process_start_identity,
                "epoch_token": s.provenance.epoch_token,
            })
        })
        .collect();
    canon_hex(&Value::Array(rows))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// SHA-256 (hex) over the canonical bytes of a JSON value.
fn canon_hex(v: &Value) -> String {
    let mut s = String::new();
    write_canonical(v, &mut s);
    Sha256::hex(s.as_bytes())
}

// ===========================================================================
// Section 7 — container / index serialization (deterministic)
// ===========================================================================

/// One payload file the stitched container must contain: its container-relative
/// path (already `payload/<game>/`-rooted by the builder) and canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StitchedFile {
    /// Path relative to `payload/<game_id>/` (e.g. `index.json`,
    /// `segments.json`, `segments/s00000000.index.json`).
    pub rel_path: String,
    /// The `role` used in the container manifest.
    pub role: &'static str,
    pub bytes: Vec<u8>,
}

fn add_main_ownership_fields(row: &mut Map<String, Value>, main_hour: &MainHour) {
    row.insert("owner".into(), main_hour.owner.to_value());
    if let Some(observation) = &main_hour.selected_observation {
        row.insert("selection_rule".into(), json!(SAME_HOUR_SELECTION_RULE));
        row.insert(
            "selected_observation_id".into(),
            json!(observation.observation_id),
        );
        row.insert(
            "source_record_seq_bounds".into(),
            json!([
                observation.record_seq_bounds.0,
                observation.record_seq_bounds.1
            ]),
        );
        row.insert(
            "source_byte_bounds".into(),
            json!([observation.byte_bounds.0, observation.byte_bounds.1]),
        );
    }
}

fn source_ref_value(segment: &SegmentInput) -> Value {
    let source = segment
        .raw_source
        .as_ref()
        .expect("raw source validated before serialization");
    let ticket = segment
        .ticket
        .as_ref()
        .expect("ticket validated before serialization");
    let participation = source
        .participations
        .iter()
        .find(|participation| participation.ticket == *ticket)
        .expect("exact participation validated before serialization");
    json!({
        "source_key": source.source_key(),
        "participation_id": participation.participation_id(),
        "session_path": source.source_path,
        "stored_digest": source.stored_digest,
        "decoded_digest": source.decoded_digest,
        "record_seq_bounds": segment.provenance.record_seq_bounds.map(|(a,b)| vec![a,b]),
        "byte_bounds": segment.provenance.byte_bounds.map(|(a,b)| vec![a,b]),
    })
}

impl StitchedPlan {
    /// The canonical `index.json` body (stitched-main-v1; design section 7.1):
    /// exactly one entry per visible `Snapshot` hour, ascending, plus the visible
    /// coverage gaps. Never lists a superseded snapshot; never hides a main gap.
    pub fn main_index_value(&self) -> Value {
        let mut snapshots: Vec<Value> = Vec::new();
        let mut coverage: Vec<Value> = Vec::new();
        for (hour, mh) in self.supersede.main.iter() {
            match &mh.disposition {
                CoverageDisposition::Snapshot { sha256 } => {
                    let file = PlannedSegment::snapshot_file(mh.segment_id, *hour);
                    let mut row = Map::new();
                    row.insert("hour_index".into(), json!(hour));
                    row.insert("file".into(), json!(file));
                    row.insert("kind".into(), json!("snapshot"));
                    row.insert("sha256".into(), json!(sha256));
                    add_main_ownership_fields(&mut row, mh);
                    snapshots.push(Value::Object(row));
                }
                other => {
                    let mut row = other.to_value().as_object().unwrap().clone();
                    row.insert("start_hour".into(), json!(hour));
                    row.insert("end_hour".into(), json!(hour));
                    row.insert("segment_id".into(), json!(mh.segment_id));
                    add_main_ownership_fields(&mut row, mh);
                    coverage.push(Value::Object(row));
                }
            }
        }
        let main_head = self.supersede.main_head.map(|h| {
            let seg = self.supersede.main.get(&h).map(|mh| mh.segment_id);
            json!({ "segment_id": seg, "hour_index": h })
        });
        json!({
            "game_id": self.game_id,
            "schema_version": self.required_inputs.schema_version,
            "index_schema": STITCH_INDEX_SCHEMA,
            "segments_file": "segments.json",
            "main_head": main_head,
            "snapshots": snapshots,
            "coverage": coalesce_coverage_ranges(coverage),
        })
    }

    /// The canonical `segments.json` body (authoritative branch graph; section 7.2).
    pub fn segments_value(&self) -> Value {
        let segments: Vec<Value> = self
            .segments
            .iter()
            .map(|p| self.segment_value(p))
            .collect();
        let main_slices: Vec<Value> = self
            .supersede
            .main_slices
            .iter()
            .map(|s| json!({ "segment_id": s.segment_id, "start_hour": s.start_hour, "end_hour": s.end_hour }))
            .collect();
        let main_head = self.supersede.main_head.map(|h| {
            let seg = self.supersede.main.get(&h).map(|mh| mh.segment_id);
            json!({ "segment_id": seg, "hour_index": h })
        });
        let mut raw_sources_by_key = BTreeMap::new();
        for segment in &self.segments {
            let source = segment
                .input
                .raw_source
                .as_ref()
                .expect("raw source validated before serialization");
            raw_sources_by_key.insert(source.source_key(), source.to_value());
        }
        json!({
            "stitch_input_digest": self.stitch_input_digest,
            "play_order_ledger_digest": self.play_order_ledger_digest,
            "fold_version": self.required_inputs.fold_version,
            "schema_version": self.required_inputs.schema_version,
            "canonical_writer_version": self.required_inputs.canonical_writer_version,
            "packer_codec_version": self.required_inputs.packer_codec_version,
            "publication": {
                "created_utc": self.required_inputs.created_utc,
                "compression_method": self.required_inputs.compression_method,
                "compression_parameters": self.required_inputs.compression_parameters,
                "entry_order": self.required_inputs.entry_order,
            },
            "campaign": {
                "scheme_major": self.scheme_major,
                "campaign_key": hex(&self.campaign_key),
                "campaign_key_digest": self.campaign_key_digest,
            },
            "stream_major": self.stream_major,
            "raw_sources": raw_sources_by_key.into_values().collect::<Vec<_>>(),
            "segments": segments,
            "main_slices": main_slices,
            "main_head": main_head,
        })
    }

    fn segment_value(&self, p: &PlannedSegment) -> Value {
        let s = &p.input;
        let identity = match &s.identity {
            CampaignIdentityResolution::Verified {
                scheme_major,
                campaign_key,
                evidence_digest,
            } => json!({
                "scheme_major": scheme_major,
                "campaign_key_digest": Sha256::hex(campaign_key),
                "evidence_digest": hex(evidence_digest),
            }),
            _ => unreachable!("only verified identity can reach serialization"),
        };
        let visible: Vec<Value> = p
            .visible_spans
            .iter()
            .map(|v| json!({ "start_hour": v.start_hour, "end_hour": v.end_hour }))
            .collect();
        let superseded: Vec<Value> = p
            .superseded_spans
            .iter()
            .map(|v| json!({ "start_hour": v.start_hour, "end_hour": v.end_hour, "superseded_by": v.superseded_by }))
            .collect();
        let source_ref = source_ref_value(s);
        let transition_coverage: Vec<Value> = s
            .transition_coverage
            .iter()
            .map(|(h, d)| {
                let mut o = d.to_value().as_object().unwrap().clone();
                o.insert("hour_index".into(), json!(h));
                o.insert(
                    "owner".into(),
                    s.observation_owner()
                        .expect("owner validated before serialization")
                        .to_value(),
                );
                Value::Object(o)
            })
            .collect();
        json!({
            "segment_id": p.segment_id,
            "play_order": s.play_order,
            "in_session_ordinal": s.in_session_ordinal,
            "ticket": s.ticket.as_ref().map(AppendTicket::to_value),
            "identity": identity,
            "session_nonce": s.provenance.session_nonce,
            "process_start_identity": s.provenance.process_start_identity,
            "epoch_token": s.provenance.epoch_token,
            "stream_major": s.provenance.stream_major,
            "build_exe_sha256": s.provenance.build_exe_sha256,
            "modset_id": s.provenance.modset_id,
            "schema_version": s.provenance.schema_version,
            "static_catalog_digest": s.provenance.static_catalog_digest,
            "compatibility": s.compatibility.to_value(),
            "cutover_hour": s.cutover(),
            "head_hour": s.head(),
            "start_reason": s.provenance.start_reason.map(|r| r.as_str()),
            "stop_reason": s.provenance.stop_reason.map(|r| r.as_str()),
            "parent_segment": p.parent_segment,
            "empty_recovery": s.is_empty_recovery(),
            "index_file": PlannedSegment::index_path(p.segment_id),
            "source_ref": source_ref,
            "transition_coverage": transition_coverage,
            "visible_spans": visible,
            "superseded_spans": superseded,
        })
    }

    /// The canonical per-segment index body (design section 7.3): every complete
    /// observed hour + disposition, emitted snapshot file/hash, source bounds, and
    /// explicit gaps. Immutable once sealed.
    pub fn segment_index_value(&self, p: &PlannedSegment) -> Value {
        let s = &p.input;
        let hours: Vec<Value> = s
            .coverage
            .iter()
            .map(|(h, d)| {
                let mut o = Map::new();
                o.insert("hour_index".into(), json!(h));
                let selected = s
                    .selected_observation(*h)
                    .expect("observation selection validated before serialization");
                if d.is_snapshot() {
                    // the retained (possibly superseded) snapshot's canonical
                    // container file, derived from this segment's global ordinal.
                    o.insert(
                        "file".into(),
                        json!(PlannedSegment::snapshot_file(p.segment_id, *h)),
                    );
                }
                if let Value::Object(dv) = d.to_value() {
                    for (k, val) in dv {
                        o.insert(k, val);
                    }
                }
                o.insert("selection_rule".into(), json!(SAME_HOUR_SELECTION_RULE));
                o.insert(
                    "selected_observation_id".into(),
                    json!(selected.observation_id),
                );
                o.insert("owner".into(), selected.owner.to_value());
                o.insert(
                    "source_record_seq_bounds".into(),
                    json!([selected.record_seq_bounds.0, selected.record_seq_bounds.1]),
                );
                o.insert(
                    "source_byte_bounds".into(),
                    json!([selected.byte_bounds.0, selected.byte_bounds.1]),
                );
                let observations = s
                    .observations
                    .get(h)
                    .expect("observation set validated before serialization");
                o.insert("observation_count".into(), json!(observations.len()));
                o.insert(
                    "observations".into(),
                    Value::Array(observations.iter().map(HourObservation::to_value).collect()),
                );
                Value::Object(o)
            })
            .collect();
        json!({
            "segment_id": p.segment_id,
            "play_order": s.play_order,
            "in_session_ordinal": s.in_session_ordinal,
            "epoch_token": s.provenance.epoch_token,
            "cutover_hour": s.cutover(),
            "head_hour": s.head(),
            "source_ref": source_ref_value(s),
            "transition_coverage": s.transition_coverage.iter().map(|(h, d)| {
                let mut o = d.to_value().as_object().unwrap().clone();
                o.insert("hour_index".into(), json!(h));
                o.insert("owner".into(), s.observation_owner().expect("owner validated").to_value());
                Value::Object(o)
            }).collect::<Vec<_>>(),
            "hours": hours,
        })
    }

    /// All non-snapshot container files (index.json, segments.json, per-segment
    /// indexes) as canonical bytes, in deterministic path order. Snapshot payload
    /// files are supplied by the fold (byte-copied) and joined by the packer using
    /// [`PlannedSegment::snapshot_file`] names; they are not synthesized here.
    ///
    /// Determinism: canonical JSON + sorted paths ⇒ identical bytes for identical
    /// plans (design section 8).
    pub fn to_container_files(&self) -> Vec<StitchedFile> {
        let mut files: Vec<StitchedFile> = Vec::new();
        files.push(StitchedFile {
            rel_path: "index.json".into(),
            role: "index",
            bytes: canon_bytes(&self.main_index_value()),
        });
        files.push(StitchedFile {
            rel_path: "segments.json".into(),
            role: "segments",
            bytes: canon_bytes(&self.segments_value()),
        });
        for p in &self.segments {
            files.push(StitchedFile {
                rel_path: PlannedSegment::index_path(p.segment_id),
                role: "segment_index",
                bytes: canon_bytes(&self.segment_index_value(p)),
            });
        }
        files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        files
    }

    /// The set of visible main-timeline snapshot files (segment-prefixed names) in
    /// ascending hour order — the payloads the packer must include and the
    /// playback index resolves.
    pub fn visible_snapshot_files(&self) -> Vec<(i64, String)> {
        self.supersede
            .main
            .iter()
            .filter_map(|(h, mh)| match &mh.disposition {
                CoverageDisposition::Snapshot { .. } => {
                    Some((*h, PlannedSegment::snapshot_file(mh.segment_id, *h)))
                }
                _ => None,
            })
            .collect()
    }
}

/// Coalesce adjacent rows only when disposition, cause, evidence and owner all
/// match. Range endpoints are the only ignored fields.
fn coalesce_coverage_ranges(rows: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for row in rows {
        let start = row.get("start_hour").and_then(Value::as_i64).unwrap_or(0);
        let end = row.get("end_hour").and_then(Value::as_i64).unwrap_or(start);
        if let Some(last) = out.last_mut() {
            let lend = last.get("end_hour").and_then(Value::as_i64).unwrap_or(0);
            let mut left = last.clone();
            let mut right = row.clone();
            for value in [&mut left, &mut right] {
                let object = value.as_object_mut().unwrap();
                object.remove("start_hour");
                object.remove("end_hour");
            }
            if left == right && lend + 1 == start {
                last.as_object_mut()
                    .unwrap()
                    .insert("end_hour".into(), json!(end));
                continue;
            }
        }
        out.push(row);
    }
    out
}

/// Canonical bytes of a JSON value (RFC-8785-style; reuses the snapshot crate's
/// manifest canonicalizer). Deterministic regardless of insertion order.
fn canon_bytes(v: &Value) -> Vec<u8> {
    let mut s = String::new();
    write_canonical(v, &mut s);
    s.into_bytes()
}

#[cfg(test)]
mod tests;
