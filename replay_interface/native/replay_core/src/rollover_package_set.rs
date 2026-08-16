//! Admission and selection for recorder-published CID1 rollover package sets.
//!
//! A rollover set is not one timeline: each member starts with its own CID1
//! authority and its hour domain may move backwards relative to the preceding
//! member. Callers must therefore select one logical stream explicitly. This
//! module never concatenates members or chooses a member on the caller's behalf.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const ROLLOVER_PACKAGE_SET_SCHEMA: &str = "cid1-rollover-package-set-v1";
pub const ROLLOVER_PACKAGE_SET_FILE: &str = "logical-stream-package-set.json";
pub const ROLLOVER_SOURCE_SET_FILE: &str = "logical-streams/logical-stream-set.json";

const MAX_DESCRIPTOR_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECORDER_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LOGICAL_STREAMS: usize = 4096;
const MAX_RELATIVE_PATH_BYTES: usize = 1024;
const MIN_GAME_HOUR_RAW: i32 = 60_005_999;
const MAX_GAME_HOUR_RAW: i32 = 63_071_999;

/// Typed classification of a rollover-segment authentication failure, so the
/// product boundary can report the honest rejection class instead of
/// flattening every rollover failure into "not playable by this build".
///
/// * `Integrity` — bytes disagree with the digest/length the package-set
///   authority names for them (a tampered, corrupt, or substituted member).
/// * `ModsetMismatch` — the selected container's manifest modset disagrees
///   with its snapshot provenance (the same typed signal the single-file
///   retained-container branch preserves).
/// * `Unsupported` — "recognized but not playable by this build/request":
///   the request cannot name a playable member (no `--logical-stream` on a
///   set, an out-of-range stream, a recovery-only set, a direct recording),
///   or the descriptor declares a package-set schema this build does not
///   support. Not evidence the recording is broken.
/// * `Structure` — the descriptor, manifest, or package set is malformed or
///   internally inconsistent (container/schema class).
///
/// `Display` renders the inner message verbatim; the class lives in the type
/// so consumers never classify by matching error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolloverAuthenticationError {
    Integrity(String),
    ModsetMismatch(String),
    Unsupported(String),
    Structure(String),
}

impl RolloverAuthenticationError {
    pub fn message(&self) -> &str {
        match self {
            Self::Integrity(message)
            | Self::ModsetMismatch(message)
            | Self::Unsupported(message)
            | Self::Structure(message) => message,
        }
    }
}

impl std::fmt::Display for RolloverAuthenticationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for RolloverAuthenticationError {}

/// Adapt a container-open failure on the SELECTED package into the rollover
/// authentication class it belongs to, mirroring the typed preservation the
/// single-file retained-container branch performs.
fn rollover_error_from_container(
    error: &snapshot::ContainerError,
    message: String,
) -> RolloverAuthenticationError {
    match error {
        snapshot::ContainerError::ModsetMismatch { .. } => {
            RolloverAuthenticationError::ModsetMismatch(message)
        }
        error if crate::source::is_container_integrity_failure(error) => {
            RolloverAuthenticationError::Integrity(message)
        }
        _ => RolloverAuthenticationError::Structure(message),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverPackageSet {
    descriptor_path: PathBuf,
    source_descriptor: PathBuf,
    source_descriptor_sha256: String,
    ordered_streams: Vec<RolloverPackageSegment>,
    transitions: Vec<RolloverPackageTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloverPackageSegment {
    pub logical_stream_id: u64,
    pub campaign_id_hex: String,
    pub cid1_payload_sha256_hex: String,
    pub raw_relative_path: PathBuf,
    /// The member file on disk at `raw_relative_path`.
    pub raw_bytes: u64,
    pub raw_sha256: String,
    /// The canonical H4CR that file decodes to. Equal to the storage pair only
    /// when the member is stored raw; the packaged container embeds DECODED
    /// bytes, so its compact-stream entry is compared against this pair.
    pub raw_decoded_bytes: u64,
    pub raw_decoded_sha256: String,
    pub manifest_relative_path: PathBuf,
    pub manifest_sha256: String,
    pub package_relative_path: PathBuf,
    pub package_sha256: String,
    pub recovery: bool,
}

/// One descriptor-selected package whose whole-file receipt and internal role
/// joins were proven against the exact retained container handle. Consumers
/// must move the container into replay admission; reopening `path` would
/// discard this guarantee.
pub struct AuthenticatedRolloverPackage {
    path: PathBuf,
    package_sha256: String,
    container: snapshot::Container,
}

impl std::fmt::Debug for AuthenticatedRolloverPackage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatedRolloverPackage")
            .field("path", &self.path)
            .field("package_sha256", &self.package_sha256)
            .finish_non_exhaustive()
    }
}

impl AuthenticatedRolloverPackage {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn package_sha256(&self) -> &str {
        &self.package_sha256
    }

    pub fn into_container(self) -> snapshot::Container {
        self.container
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolloverPackageTransition {
    pub current_logical_stream_id: u64,
    pub proposed_logical_stream_id: u64,
    pub last_admitted_h4cr_sequence: u64,
    pub boundary_game_hour_raw: i32,
    pub reason: String,
    pub proposal_nonce: u64,
    pub current_campaign_id_hex: String,
    pub proposed_campaign_id_hex: String,
    pub current_cid1_payload_sha256_hex: String,
    pub proposed_cid1_payload_sha256_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePackageSet {
    schema: String,
    source_descriptor: String,
    source_descriptor_sha256: String,
    ordered_streams: Vec<WirePackageSegment>,
    transitions: Vec<RolloverPackageTransition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSourceSet {
    schema: String,
    ordered_streams: Vec<WireSourceStream>,
    transitions: Vec<RolloverPackageTransition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSourceStream {
    logical_stream_id: u64,
    relative_path: String,
    last_complete_sequence: u64,
    campaign_id_hex: String,
    cid1_payload_sha256_hex: String,
    file_bytes: u64,
    file_sha256_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePackageSegment {
    logical_stream_id: u64,
    campaign_id_hex: String,
    cid1_payload_sha256_hex: String,
    raw_relative_path: String,
    raw_bytes: u64,
    raw_sha256: String,
    raw_decoded_bytes: u64,
    raw_decoded_sha256: String,
    manifest_relative_path: String,
    manifest_sha256: String,
    package_relative_path: String,
    package_sha256: String,
    recovery: bool,
}

impl RolloverPackageSet {
    /// Read and validate an explicitly named descriptor, or discover the
    /// standard descriptor directly inside `path` when `path` is a directory.
    ///
    /// A well-formed descriptor that names a schema this build does not know
    /// is the `Unsupported` class ("recognized but not playable by this
    /// build"), checked BEFORE the strict v1 wire decode so a future schema
    /// is not misreported as a malformed v1 descriptor. Every other failure
    /// here is `Structure` (the descriptor is the root of trust; no digest
    /// authority exists above it to lose against).
    pub fn open(path: &Path) -> Result<Self, RolloverAuthenticationError> {
        let (descriptor_path, bytes) =
            Self::read_descriptor(path).map_err(RolloverAuthenticationError::Structure)?;
        let value = snapshot::parse_json_strict(&bytes).map_err(|error| {
            RolloverAuthenticationError::Structure(format!(
                "strict-decode rollover package-set descriptor: {error}"
            ))
        })?;
        if let Some(declared) = value.get("schema").and_then(|schema| schema.as_str()) {
            if declared != ROLLOVER_PACKAGE_SET_SCHEMA {
                return Err(RolloverAuthenticationError::Unsupported(format!(
                    "unsupported rollover package-set schema {declared:?}"
                )));
            }
        }
        Self::decode_v1_descriptor(descriptor_path, value)
            .map_err(RolloverAuthenticationError::Structure)
    }

    /// Locate and read the bounded descriptor bytes (symlink-refusing).
    fn read_descriptor(path: &Path) -> Result<(PathBuf, Vec<u8>), String> {
        let descriptor_path = if path.is_dir() {
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!("inspect rollover package-set root {}: {error}", path.display())
            })?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(format!(
                    "rollover package-set root is not a regular non-symlink directory: {}",
                    path.display()
                ));
            }
            path.join(ROLLOVER_PACKAGE_SET_FILE)
        } else {
            path.to_path_buf()
        };
        if descriptor_path.file_name().and_then(|value| value.to_str())
            != Some(ROLLOVER_PACKAGE_SET_FILE)
        {
            return Err(format!(
                "rollover package-set descriptor must be named {ROLLOVER_PACKAGE_SET_FILE:?}"
            ));
        }
        let descriptor_parent = descriptor_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent_metadata = fs::symlink_metadata(descriptor_parent).map_err(|error| {
            format!("inspect rollover package-set parent {}: {error}", descriptor_parent.display())
        })?;
        if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {
            return Err(format!(
                "rollover package-set parent is not a regular non-symlink directory: {}",
                descriptor_parent.display()
            ));
        }
        let bytes = read_regular_capped(&descriptor_path, MAX_DESCRIPTOR_BYTES, "package-set descriptor")?;
        Ok((descriptor_path, bytes))
    }

    /// Strict v1 wire decode + internal-consistency validation. The schema
    /// string has already been classified by [`Self::open`]; the equality
    /// check here is a retained structural invariant.
    fn decode_v1_descriptor(
        descriptor_path: PathBuf,
        value: serde_json::Value,
    ) -> Result<Self, String> {
        let wire: WirePackageSet = serde_json::from_value(value)
            .map_err(|error| format!("decode rollover package-set descriptor: {error}"))?;
        if wire.schema != ROLLOVER_PACKAGE_SET_SCHEMA {
            return Err(format!(
                "unsupported rollover package-set schema {:?}", wire.schema
            ));
        }
        if wire.ordered_streams.is_empty() {
            return Err("rollover package-set descriptor has no logical streams".to_string());
        }
        if wire.ordered_streams.len() > MAX_LOGICAL_STREAMS {
            return Err(format!(
                "rollover package-set has {} logical streams; limit is {MAX_LOGICAL_STREAMS}",
                wire.ordered_streams.len()
            ));
        }
        let source_descriptor = strict_relative_path(&wire.source_descriptor, "source_descriptor")?;
        if source_descriptor != Path::new(ROLLOVER_SOURCE_SET_FILE) {
            return Err(format!(
                "rollover package-set source_descriptor must be {ROLLOVER_SOURCE_SET_FILE:?}"
            ));
        }
        require_lower_hex(&wire.source_descriptor_sha256, 64, "source_descriptor_sha256")?;
        if is_zero_hex(&wire.source_descriptor_sha256) {
            return Err("rollover package-set has a zero source_descriptor_sha256".to_string());
        }

        let mut ordered_streams = Vec::with_capacity(wire.ordered_streams.len());
        let mut member_paths = BTreeSet::new();
        member_paths.insert(path_collision_key(&source_descriptor));
        member_paths.insert(ROLLOVER_PACKAGE_SET_FILE.to_ascii_lowercase());
        for (ordinal, stream) in wire.ordered_streams.into_iter().enumerate() {
            let expected_id = u64::try_from(ordinal)
                .map_err(|_| "rollover package-set logical-stream count overflow".to_string())?;
            if stream.logical_stream_id != expected_id {
                return Err(format!(
                    "rollover package-set logical streams must be contiguous from zero: expected {expected_id}, found {}",
                    stream.logical_stream_id
                ));
            }
            require_lower_hex(&stream.campaign_id_hex, 64, "campaign_id_hex")?;
            if stream.campaign_id_hex.bytes().all(|value| value == b'0') {
                return Err(format!("logical stream {expected_id} has a zero campaign_id_hex"));
            }
            require_lower_hex(
                &stream.cid1_payload_sha256_hex,
                64,
                "cid1_payload_sha256_hex",
            )?;
            require_lower_hex(&stream.raw_sha256, 64, "raw_sha256")?;
            require_lower_hex(&stream.manifest_sha256, 64, "manifest_sha256")?;
            require_lower_hex(&stream.package_sha256, 64, "package_sha256")?;
            for (value, field) in [
                (&stream.cid1_payload_sha256_hex, "cid1_payload_sha256_hex"),
                (&stream.raw_sha256, "raw_sha256"),
                (&stream.raw_decoded_sha256, "raw_decoded_sha256"),
                (&stream.manifest_sha256, "manifest_sha256"),
                (&stream.package_sha256, "package_sha256"),
            ] {
                if is_zero_hex(value) {
                    return Err(format!("logical stream {expected_id} has a zero {field}"));
                }
            }
            if stream.raw_bytes == 0 || stream.raw_decoded_bytes == 0 {
                return Err(format!("logical stream {expected_id} has an empty raw member"));
            }
            let raw_relative_path = strict_relative_path(
                &stream.raw_relative_path,
                "raw_relative_path",
            )?;
            let manifest_relative_path = strict_relative_path(
                &stream.manifest_relative_path,
                "manifest_relative_path",
            )?;
            let package_relative_path = strict_relative_path(
                &stream.package_relative_path,
                "package_relative_path",
            )?;
            if !member_paths.insert(path_collision_key(&raw_relative_path))
                || !member_paths.insert(path_collision_key(&manifest_relative_path))
                || !member_paths.insert(path_collision_key(&package_relative_path))
            {
                return Err(format!(
                    "logical stream {expected_id} reuses another member's raw, manifest, or package path"
                ));
            }
            ordered_streams.push(RolloverPackageSegment {
                logical_stream_id: stream.logical_stream_id,
                campaign_id_hex: stream.campaign_id_hex,
                cid1_payload_sha256_hex: stream.cid1_payload_sha256_hex,
                raw_relative_path,
                raw_bytes: stream.raw_bytes,
                raw_sha256: stream.raw_sha256,
                raw_decoded_bytes: stream.raw_decoded_bytes,
                raw_decoded_sha256: stream.raw_decoded_sha256,
                manifest_relative_path,
                manifest_sha256: stream.manifest_sha256,
                package_relative_path,
                package_sha256: stream.package_sha256,
                recovery: stream.recovery,
            });
        }
        validate_package_transitions(&ordered_streams, &wire.transitions)?;

        Ok(Self {
            descriptor_path,
            source_descriptor,
            source_descriptor_sha256: wire.source_descriptor_sha256,
            ordered_streams,
            transitions: wire.transitions,
        })
    }

    pub fn descriptor_path(&self) -> &Path {
        &self.descriptor_path
    }

    pub fn source_descriptor(&self) -> &Path {
        &self.source_descriptor
    }

    pub fn source_descriptor_sha256(&self) -> &str {
        &self.source_descriptor_sha256
    }

    pub fn ordered_streams(&self) -> &[RolloverPackageSegment] {
        &self.ordered_streams
    }

    pub fn transitions(&self) -> &[RolloverPackageTransition] {
        &self.transitions
    }

    /// Resolve and authenticate one logical-stream package. Both the durable
    /// source descriptor and the selected package/manifest pair are bound.
    /// Failures carry their typed rejection class
    /// ([`RolloverAuthenticationError`]): an authentication failure on a
    /// member is never reported as a build-capability limitation.
    pub fn select(
        &self,
        logical_stream_id: u64,
    ) -> Result<AuthenticatedRolloverPackage, RolloverAuthenticationError> {
        let index = usize::try_from(logical_stream_id).map_err(|_| {
            RolloverAuthenticationError::Unsupported(format!(
                "logical stream {logical_stream_id} is outside this platform's index range"
            ))
        })?;
        let segment = self.ordered_streams.get(index).ok_or_else(|| {
            RolloverAuthenticationError::Unsupported(format!(
                "logical stream {logical_stream_id} is not present; available logical streams are 0..{}",
                self.ordered_streams.len() - 1
            ))
        })?;
        if segment.logical_stream_id != logical_stream_id {
            return Err(RolloverAuthenticationError::Structure(
                "rollover package-set internal ordering invariant failed".to_string(),
            ));
        }
        if self.ordered_streams.iter().any(|member| member.recovery) {
            return Err(RolloverAuthenticationError::Unsupported(
                "rollover package set contains a recovery-only artifact and is not eligible for production playback"
                    .to_string(),
            ));
        }
        let root = self
            .descriptor_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let (_, source_descriptor_bytes) = read_bound_file(
            root,
            &self.source_descriptor,
            MAX_DESCRIPTOR_BYTES,
            None,
            &self.source_descriptor_sha256,
            "source descriptor",
        )?;
        validate_source_descriptor(&source_descriptor_bytes, self)
            .map_err(RolloverAuthenticationError::Structure)?;
        let (_, manifest_bytes) = read_bound_file(
            root,
            &segment.manifest_relative_path,
            MAX_RECORDER_MANIFEST_BYTES,
            None,
            &segment.manifest_sha256,
            "selected recorder manifest",
        )?;
        let incoming_transition = logical_stream_id
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.transitions.get(index));
        validate_selected_manifest(&manifest_bytes, segment, incoming_transition)
            .map_err(RolloverAuthenticationError::Structure)?;
        let package_path = resolve_bound_regular_file(
            root,
            &segment.package_relative_path,
            "selected replay package",
        )
        .map_err(RolloverAuthenticationError::Structure)?;
        let container = validate_selected_container(&package_path, segment, &manifest_bytes)?;
        Ok(AuthenticatedRolloverPackage {
            path: package_path,
            package_sha256: segment.package_sha256.clone(),
            container,
        })
    }
}

fn validate_package_transitions(
    streams: &[RolloverPackageSegment],
    transitions: &[RolloverPackageTransition],
) -> Result<(), String> {
    if transitions.len() != streams.len().saturating_sub(1) {
        return Err(format!(
            "rollover package-set has {} streams but {} transitions",
            streams.len(),
            transitions.len()
        ));
    }
    let mut previous_nonce = None;
    for (index, transition) in transitions.iter().enumerate() {
        let current = &streams[index];
        let proposed = &streams[index + 1];
        if transition.current_logical_stream_id != current.logical_stream_id
            || transition.proposed_logical_stream_id != proposed.logical_stream_id
        {
            return Err(format!("rollover transition {index} does not join adjacent logical streams"));
        }
        if transition.last_admitted_h4cr_sequence == u64::MAX {
            return Err(format!("rollover transition {index} has an invalid drained frontier"));
        }
        if transition.proposal_nonce == 0
            || transition.proposal_nonce == u64::MAX
            || previous_nonce.is_some_and(|previous| transition.proposal_nonce <= previous)
        {
            return Err(format!("rollover transition {index} proposal_nonce is not strictly increasing"));
        }
        previous_nonce = Some(transition.proposal_nonce);
        for (value, field) in [
            (&transition.current_campaign_id_hex, "current_campaign_id_hex"),
            (&transition.proposed_campaign_id_hex, "proposed_campaign_id_hex"),
            (&transition.current_cid1_payload_sha256_hex, "current_cid1_payload_sha256_hex"),
            (&transition.proposed_cid1_payload_sha256_hex, "proposed_cid1_payload_sha256_hex"),
        ] {
            require_lower_hex(value, 64, field)?;
            if is_zero_hex(value) {
                return Err(format!("rollover transition {index} has a zero {field}"));
            }
        }
        if transition.current_campaign_id_hex != current.campaign_id_hex
            || transition.proposed_campaign_id_hex != proposed.campaign_id_hex
            || transition.current_cid1_payload_sha256_hex != current.cid1_payload_sha256_hex
            || transition.proposed_cid1_payload_sha256_hex != proposed.cid1_payload_sha256_hex
        {
            return Err(format!("rollover transition {index} identity digests disagree with adjacent members"));
        }
        if transition.current_cid1_payload_sha256_hex
            == transition.proposed_cid1_payload_sha256_hex
        {
            return Err(format!("rollover transition {index} did not change CID1 payload identity"));
        }
        if !(MIN_GAME_HOUR_RAW..=MAX_GAME_HOUR_RAW).contains(&transition.boundary_game_hour_raw) {
            return Err(format!("rollover transition {index} has an invalid boundary game hour"));
        }
        match transition.reason.as_str() {
            "whole_pair_changed" if current.campaign_id_hex != proposed.campaign_id_hex => {}
            "same_pair_time_rollback" if current.campaign_id_hex == proposed.campaign_id_hex => {}
            "whole_pair_changed" | "same_pair_time_rollback" => {
                return Err(format!("rollover transition {index} reason contradicts its campaign identities"))
            }
            _ => return Err(format!("rollover transition {index} has an unknown reason")),
        }
    }
    Ok(())
}

fn validate_source_descriptor(bytes: &[u8], package: &RolloverPackageSet) -> Result<(), String> {
    let source: WireSourceSet = serde_json::from_value(
        snapshot::parse_json_strict(bytes)
            .map_err(|error| format!("strict-decode rollover source descriptor: {error}"))?,
    )
        .map_err(|error| format!("decode rollover source descriptor: {error}"))?;
    if source.schema != "cid1-rollover-set-v1"
        || source.ordered_streams.len() != package.ordered_streams.len()
        || source.transitions != package.transitions
    {
        return Err("rollover source descriptor disagrees with the package-set authority".to_string());
    }
    for (index, (source_stream, package_stream)) in source
        .ordered_streams
        .iter()
        .zip(&package.ordered_streams)
        .enumerate()
    {
        let expected_relative = package_stream
            .raw_relative_path
            .strip_prefix("logical-streams")
            .map_err(|_| "package raw path is outside logical-streams".to_string())?;
        if source_stream.logical_stream_id != package_stream.logical_stream_id
            || Path::new(&source_stream.relative_path) != expected_relative
            || source_stream.last_complete_sequence == u64::MAX
            || source_stream.campaign_id_hex != package_stream.campaign_id_hex
            || source_stream.cid1_payload_sha256_hex != package_stream.cid1_payload_sha256_hex
            || source_stream.file_bytes != package_stream.raw_bytes
            || source_stream.file_sha256_hex != package_stream.raw_sha256
        {
            return Err(format!("rollover source stream {index} disagrees with its packaged member"));
        }
        if let Some(transition) = package.transitions.get(index) {
            if transition.last_admitted_h4cr_sequence != source_stream.last_complete_sequence {
                return Err(format!("rollover transition {index} drained frontier disagrees with the sealed source stream"));
            }
        }
    }
    Ok(())
}

fn validate_selected_manifest(
    bytes: &[u8],
    segment: &RolloverPackageSegment,
    incoming_transition: Option<&RolloverPackageTransition>,
) -> Result<(), String> {
    let value = snapshot::parse_json_strict(bytes)
        .map_err(|error| format!("strict-decode selected recorder manifest: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "selected recorder manifest is not an object".to_string())?;
    if object
        .get("logical_stream_id")
        .and_then(serde_json::Value::as_u64)
        != Some(segment.logical_stream_id)
    {
        return Err("selected recorder manifest logical_stream_id disagrees with the package set".to_string());
    }
    if object
        .get("campaign_identity")
        .and_then(serde_json::Value::as_object)
        .and_then(|identity| identity.get("campaign_id"))
        .and_then(serde_json::Value::as_str)
        != Some(segment.campaign_id_hex.as_str())
    {
        return Err("selected recorder manifest campaign_id disagrees with the package set".to_string());
    }
    if object
        .get("campaign_identity")
        .and_then(serde_json::Value::as_object)
        .and_then(|identity| identity.get("stream_header_sha256"))
        .and_then(serde_json::Value::as_str)
        != Some(segment.cid1_payload_sha256_hex.as_str())
    {
        return Err("selected recorder manifest stream_header_sha256 disagrees with the package set".to_string());
    }
    let expected_raw = segment
        .raw_relative_path
        .to_str()
        .ok_or_else(|| "selected raw member path is not UTF-8".to_string())?;
    if object.get("raw_stream").and_then(serde_json::Value::as_str) != Some(expected_raw) {
        return Err("selected recorder manifest raw_stream disagrees with the package set".to_string());
    }
    let expected_replay_name = segment
        .package_relative_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "selected package file name is not UTF-8".to_string())?;
    if object.get("replay_file").and_then(serde_json::Value::as_str)
        != Some(expected_replay_name)
    {
        return Err("selected recorder manifest replay_file disagrees with the package set".to_string());
    }
    if let Some(transition) = incoming_transition {
        let observed_boundary = object
            .get("campaign_identity")
            .and_then(serde_json::Value::as_object)
            .and_then(|identity| identity.get("observed_game_hour_raw"))
            .and_then(serde_json::Value::as_i64);
        if observed_boundary != Some(i64::from(transition.boundary_game_hour_raw)) {
            return Err("selected recorder manifest CID1 boundary hour disagrees with its incoming rollover transition".to_string());
        }
    }
    Ok(())
}

/// A selected package that fails the retained-handle container open is
/// classified against the package-set's declared whole-file SHA-256: the
/// container parser may trip on a STRUCTURAL shape first (a truncated or
/// garbled ZIP), but when the on-disk bytes provably disagree with the digest
/// the descriptor names, the member was tampered with or substituted and the
/// failure is INTEGRITY. Only bytes that still match the declared digest fall
/// through to the container error's own class (the descriptor pinned a
/// malformed package). The open has already failed and stays failed either
/// way — this probe classifies, it never admits.
fn classify_selected_package_open_error(
    path: &Path,
    segment: &RolloverPackageSegment,
    error: &snapshot::ContainerError,
) -> RolloverAuthenticationError {
    let message = format!("open selected replay package through retained handle: {error}");
    if declared_package_sha256_matches(path, &segment.package_sha256) == Some(false) {
        return RolloverAuthenticationError::Integrity(format!(
            "{message}; selected replay package bytes disagree with the package-set SHA-256"
        ));
    }
    rollover_error_from_container(error, message)
}

/// Streaming whole-file SHA-256 probe over the descriptor-authorized package
/// path, used only to classify an already-failed open. `None` when the file
/// cannot be read (classification then stays with the container error).
fn declared_package_sha256_matches(path: &Path, expected_sha256: &str) -> Option<bool> {
    let file = fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut reader, &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Some(format!("{:x}", digest.finalize()) == expected_sha256)
}

fn validate_selected_container(
    path: &Path,
    segment: &RolloverPackageSegment,
    external_manifest_bytes: &[u8],
) -> Result<snapshot::Container, RolloverAuthenticationError> {
    let (container, package_bytes, package_sha256) =
        snapshot::Container::open_strict_with_file_receipt(
            path,
            &snapshot::ExpectedIdentity::default(),
        )
        .map_err(|error| classify_selected_package_open_error(path, segment, &error))?;
    if package_bytes == 0 || package_sha256 != segment.package_sha256 {
        return Err(RolloverAuthenticationError::Integrity(
            "selected replay package retained-handle receipt disagrees with the package set".to_string(),
        ));
    }
    let compact = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
    if compact.len() != 1 {
        return Err(RolloverAuthenticationError::Structure(format!(
            "selected replay package must contain exactly one {:?} entry; found {}",
            snapshot::COMPACT_STREAM_ROLE,
            compact.len()
        )));
    }
    let compact = &compact[0];
    // The container's compact-stream entry is canonical H4CR: the packer
    // decodes the member on the way in. Comparing it to the STORAGE pair holds
    // only while the member is stored raw, and silently rejects every
    // compressed member as a disagreement.
    if compact.bytes != segment.raw_decoded_bytes
        || compact.sha256.as_str() != segment.raw_decoded_sha256.as_str()
    {
        return Err(RolloverAuthenticationError::Integrity(
            "selected replay package compact-stream entry disagrees with the package set".to_string(),
        ));
    }
    let receipt = container
        .authenticate_entry(&compact.path)
        .map_err(|error| {
            rollover_error_from_container(
                &error,
                format!("authenticate selected compact-stream entry: {error}"),
            )
        })?;
    if receipt.uncompressed_len() != segment.raw_decoded_bytes
        || receipt.sha256() != segment.raw_decoded_sha256.as_str()
    {
        return Err(RolloverAuthenticationError::Integrity(
            "selected compact-stream authentication receipt disagrees with the package set".to_string(),
        ));
    }

    let manifests = container.entries_by_role("recorder_manifest");
    if manifests.len() != 1 {
        return Err(RolloverAuthenticationError::Structure(format!(
            "selected replay package must contain exactly one recorder_manifest entry; found {}",
            manifests.len()
        )));
    }
    if manifests[0].bytes != external_manifest_bytes.len() as u64
        || manifests[0].sha256.as_str() != segment.manifest_sha256.as_str()
    {
        return Err(RolloverAuthenticationError::Integrity(
            "selected replay package recorder_manifest metadata disagrees with the package set".to_string(),
        ));
    }
    let embedded = container
        .read_entry_bytes(&manifests[0].path)
        .map_err(|error| {
            rollover_error_from_container(
                &error,
                format!("authenticate embedded recorder manifest: {error}"),
            )
        })?;
    if embedded.as_slice() != external_manifest_bytes {
        return Err(RolloverAuthenticationError::Integrity(
            "selected replay package embedded recorder manifest disagrees with the package set".to_string(),
        ));
    }
    Ok(container)
}

/// Authenticate one explicitly selected rollover member and return its pinned
/// container capability. Direct recordings use the ordinary retained-container
/// open and are intentionally outside this API.
pub fn authenticate_rollover_segment(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<AuthenticatedRolloverPackage, RolloverAuthenticationError> {
    let explicitly_descriptor = path.file_name().and_then(|value| value.to_str())
        == Some(ROLLOVER_PACKAGE_SET_FILE);
    let discovered_descriptor = path.is_dir() && path.join(ROLLOVER_PACKAGE_SET_FILE).is_file();
    if explicitly_descriptor || discovered_descriptor {
        let set = RolloverPackageSet::open(path)?;
        let selected = logical_stream_id.ok_or_else(|| {
            RolloverAuthenticationError::Unsupported(format!(
                "recording is an ordered CID1 rollover package set with {} logical streams; select one explicitly with --logical-stream <0..{}>",
                set.ordered_streams.len(),
                set.ordered_streams.len() - 1
            ))
        })?;
        return set.select(selected);
    }
    if let Some(selected) = logical_stream_id {
        return Err(RolloverAuthenticationError::Unsupported(format!(
            "--logical-stream {selected} requires a {ROLLOVER_PACKAGE_SET_FILE} descriptor or a directory containing one"
        )));
    }
    Err(RolloverAuthenticationError::Unsupported(
        "direct recordings do not require rollover-segment resolution".to_string(),
    ))
}

fn strict_relative_path(value: &str, field: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_PATH_BYTES
        || value.contains('\\')
        || value.contains(':')
        || value.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.chars().any(char::is_control)
        })
    {
        return Err(format!("{field} is not a bounded portable relative path"));
    }
    let path = PathBuf::from(value);
    if path.components().any(|component| !matches!(component, Component::Normal(_))) {
        return Err(format!("{field} is not a strict relative path: {value:?}"));
    }
    Ok(path)
}

fn path_collision_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
}

fn require_lower_hex(value: &str, len: usize, field: &str) -> Result<(), String> {
    if value.len() != len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{field} must be exactly {len} lowercase hexadecimal characters"));
    }
    Ok(())
}

fn is_zero_hex(value: &str) -> bool {
    value.bytes().all(|byte| byte == b'0')
}

fn read_regular_capped(path: &Path, cap: u64, label: &str) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} is not a regular non-symlink file: {}", path.display()));
    }
    if metadata.len() > cap {
        return Err(format!("{label} is {} bytes; limit is {cap}", metadata.len()));
    }
    let file = fs::File::open(path)
        .map_err(|error| format!("open {label} {}: {error}", path.display()))?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label} {}: {error}", path.display()))?;
    if bytes.len() as u64 > cap {
        return Err(format!("{label} grew beyond the {cap}-byte limit while being read"));
    }
    Ok(bytes)
}

fn resolve_bound_regular_file(
    root: &Path,
    relative: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root)
        .map_err(|error| format!("canonicalize rollover package root {}: {error}", root.display()))?;
    let candidate = root.join(relative);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("inspect {label} {}: {error}", candidate.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} is not a regular non-symlink file: {}", candidate.display()));
    }
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| format!("canonicalize {label} {}: {error}", candidate.display()))?;
    if !canonical.starts_with(&root) || canonical == root {
        return Err(format!("{label} escapes the rollover package root"));
    }
    Ok(canonical)
}

fn read_bound_file(
    root: &Path,
    relative: &Path,
    cap: u64,
    expected_len: Option<u64>,
    expected_sha256: &str,
    label: &str,
) -> Result<(PathBuf, Vec<u8>), RolloverAuthenticationError> {
    // Disagreement between the read bytes and the digest/length the
    // package-set authority declares for them is the INTEGRITY class; every
    // filesystem/shape failure before that comparison is structural.
    let canonical = resolve_bound_regular_file(root, relative, label)
        .map_err(RolloverAuthenticationError::Structure)?;
    let mut file = fs::File::open(&canonical).map_err(|error| {
        RolloverAuthenticationError::Structure(format!(
            "open {label} {}: {error}",
            canonical.display()
        ))
    })?;
    let handle_len = file
        .metadata()
        .map_err(|error| {
            RolloverAuthenticationError::Structure(format!(
                "inspect opened {label} {}: {error}",
                canonical.display()
            ))
        })?
        .len();
    if handle_len > cap {
        return Err(RolloverAuthenticationError::Structure(format!(
            "{label} is {handle_len} bytes; limit is {cap}"
        )));
    }
    if expected_len.is_some_and(|expected| handle_len != expected) {
        return Err(RolloverAuthenticationError::Integrity(format!(
            "{label} byte length differs from the package-set descriptor"
        )));
    }
    let mut digest = Sha256::new();
    let mut bytes = Vec::with_capacity(usize::try_from(handle_len).unwrap_or(0));
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            RolloverAuthenticationError::Structure(format!(
                "read {label} {}: {error}",
                canonical.display()
            ))
        })?;
    if bytes.len() as u64 != handle_len || bytes.len() as u64 > cap {
        return Err(RolloverAuthenticationError::Integrity(format!(
            "{label} changed length while being read"
        )));
    }
    digest.update(&bytes);
    let actual = format!("{:x}", digest.finalize());
    if actual != expected_sha256 {
        return Err(RolloverAuthenticationError::Integrity(format!(
            "{label} SHA-256 differs from the package-set descriptor"
        )));
    }
    Ok((canonical, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use snapshot::{
        pack, CertifiedPrefoldSpoolDecoder, CertifiedPrefoldSpoolRequest, PackEntry,
        PackIdentity, PrefoldDecodeError, PrefoldedSpool, PrefoldedSpoolEntry,
    };
    use std::collections::BTreeMap;

    struct FixtureSpoolDecoder;

    impl CertifiedPrefoldSpoolDecoder for FixtureSpoolDecoder {
        fn decoder_id(&self) -> &'static str {
            "test.rollover.retained-container"
        }

        fn decode_certified_spool(
            &self,
            request: CertifiedPrefoldSpoolRequest,
        ) -> Result<PrefoldedSpool, PrefoldDecodeError> {
            let (game_id, _raw_path, snapshot_dir, _len, _sha) = request.into_parts();
            let bytes = serde_json::to_vec(&json!({
                "game_id": game_id,
                "schema_version": "1",
                "hour_index": 7,
                "date": "1936.1.1.7",
                "retained_object_marker": "RETAINED_OBJECT"
            }))
            .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            let file_name = "000000000007.snapshot.json";
            fs::write(snapshot_dir.join(file_name), &bytes)
                .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            let entry = PrefoldedSpoolEntry::try_new(
                7,
                "1936.1.1.7",
                file_name,
                bytes.len() as u64,
                snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes)),
            )?;
            PrefoldedSpool::try_new(
                game_id,
                serde_json::Value::String("1".to_string()),
                vec![entry],
            )
        }
    }

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn portable_content_manifest() -> portable_content_manifest::ContentManifest {
        let mut manifest = portable_content_manifest::ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: "33".repeat(32),
            source_game: portable_content_manifest::SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "44".repeat(32),
                build_profile: "hoi4-a729".into(),
            },
            source_profile: portable_content_manifest::SourceProfileIdentity {
                profile_id: "rollover-fixture-profile".into(),
                playset_id: "rollover-fixture-playset".into(),
                selection_sha256: "00".repeat(32),
            },
            effective_vfs_manifest_sha256: "55".repeat(32),
            ordered_layers: Vec::new(),
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::from([
                ("physical_width".into(), "1920".into()),
                ("physical_height".into(), "1080".into()),
                ("min_gui_width".into(), "1280".into()),
                ("min_gui_height".into(), "720".into()),
                ("gui_scale_f32_bits".into(), 1.0f32.to_bits().to_string()),
                ("refresh_rate_hz".into(), "60".into()),
                ("max_refresh_rate_hz".into(), "60".into()),
                ("fullscreen".into(), "false".into()),
                ("borderless".into(), "true".into()),
                ("display_mode".into(), "borderless_fullscreen".into()),
                ("settings_file_sha256".into(), "aa".repeat(32)),
                // The authority that proved these settings travels in the
                // same map, so a replay cannot reproduce them without knowing
                // how well they were proven.
                ("settings_authority".into(), "pre_launch_bracketed".into()),
            ]),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    /// A written-out rollover package tree, with the scanner guards that hold
    /// it open CARRIED AS A FIELD.
    ///
    /// This used to be a `(PathBuf, PathBuf, Vec<File>)` tuple, and that was the
    /// weakest of the three retention shapes in use in this repository:
    /// `#[must_use]` does not reach through a destructuring pattern, so
    /// `let (root, descriptor, _) = fixture(..)` dropped the guards at that
    /// semicolon and compiled silently. Every call site happened to bind them,
    /// by convention and by the doc comment asking for it -- which is not an
    /// enforcement, and a new test written the obvious way would not have.
    ///
    /// As a field there is nothing for a caller to remember: the binding a test
    /// needs anyway is what holds the tree open, and a test added later is
    /// covered by construction. `#[must_use]` now also catches the one
    /// remaining way to get it wrong, calling `fixture(..)` for its side effect
    /// and discarding the value.
    /// The fields are PRIVATE TO THIS MODULE and reached only through the two
    /// borrowed accessors, which is the difference between a convention and an
    /// enforcement. A plain public-field struct is not "covered by
    /// construction": `let RolloverFixture { root, descriptor, .. } =
    /// fixture(..)` consumes the `#[must_use]` value and drops the guards at
    /// that statement, reproducing the exact tuple defect this replaced. A
    /// reviewer pointed that out, and it was right. With the fields private,
    /// that pattern does not compile outside this module.
    mod fixture_guard {
        use std::path::{Path, PathBuf};

        #[must_use]
        pub(super) struct RolloverFixture {
            root: PathBuf,
            descriptor: PathBuf,
            /// Held for the lifetime of the fixture; see the parent docs.
            _scanner_guards: Vec<std::fs::File>,
        }

        impl RolloverFixture {
            pub(super) fn new(
                root: PathBuf,
                descriptor: PathBuf,
                scanner_guards: Vec<std::fs::File>,
            ) -> Self {
                Self {
                    root,
                    descriptor,
                    _scanner_guards: scanner_guards,
                }
            }

            pub(super) fn root(&self) -> &Path {
                &self.root
            }

            pub(super) fn descriptor(&self) -> &Path {
                &self.descriptor
            }
        }
    }

    use fixture_guard::RolloverFixture;

    fn fixture(tag: &str) -> RolloverFixture {
        fixture_with_storage(tag, None)
    }

    /// `storage` overrides the member's STORAGE identity (bytes, digest) so it
    /// differs from the decoded identity, which is what a compressed member
    /// looks like. `None` keeps them equal, which is what a raw member is.
    fn fixture_with_storage(tag: &str, storage: Option<(u64, String)>) -> RolloverFixture {
        let root = std::env::temp_dir().join(format!(
            "replay_core_rollover_set_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("logical-streams/packages/00000000000000000000")).unwrap();
        // Same minimal raw-H4CR fixture accepted by replay_core's certified
        // prefolder tests. The fixture decoder owns semantic prefolding; this
        // boundary still requires the canonical compact-stream magic.
        let raw = b"H4CRfixture-stream";
        let source = serde_json::to_vec(&json!({
            "schema": "cid1-rollover-set-v1",
            "ordered_streams": [{
                "logical_stream_id": 0,
                "relative_path": "logical-stream-0.sealed.h4cr",
                "last_complete_sequence": 1,
                "campaign_id_hex": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
                "cid1_payload_sha256_hex": "22".repeat(32),
                "file_bytes": storage.as_ref().map_or(raw.len() as u64, |(bytes, _)| *bytes),
                "file_sha256_hex": storage
                    .as_ref()
                    .map_or_else(|| hash(raw), |(_, digest)| digest.clone())
            }],
            "transitions": []
        }))
        .unwrap();
        let content_manifest = portable_content_manifest();
        let manifest = serde_json::to_vec(&json!({
            "logical_stream_id": 0,
            "campaign_identity": {
                "schema": "hoi4.campaign_identity.v1",
                "state": "known",
                "campaign_id": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
                "game_unique_id": "a4b17fed-ed12-4361-9064-fb6576638110",
                "game_unique_seed": "1026046527",
                "source": "live_memory",
                "source_build_profile": "hoi4-a729",
                "observed_game_hour_raw": 60759359,
                "stream_header_sha256": "22".repeat(32),
                "resolution_note": null,
                "witnesses": {
                    "start_date_raw": null,
                    "session": null,
                    "scenario": null,
                    "initial_human_country_tags": null,
                    "modset_identity": null
                }
            },
            "portable_content_manifest": serde_json::to_value(&content_manifest).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content_manifest.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": "l_english",
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1280,
                        "min_gui_height": 720,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": 60,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "aa".repeat(32),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            },
            "raw_stream": "logical-streams/logical-stream-0.sealed.h4cr",
            "replay_file": "native_session.battleplan",
            "replay_packaging": {"static_artifacts": []}
        }))
        .unwrap();
        fs::write(root.join("logical-streams/logical-stream-set.json"), &source).unwrap();
        fs::write(root.join("native_session_manifest.json"), &manifest).unwrap();
        let game = "rollover-fixture";
        let stream_reference = "collector_raw/collector_stream.bin";
        let stream_path = format!("payload/{game}/{stream_reference}");
        let manifest_reference = "recorder/native_session_manifest.json";
        let manifest_path = format!("payload/{game}/{manifest_reference}");
        let package = pack(
            game,
            &[
                PackEntry {
                    path: format!("payload/{game}/index.json"),
                    bytes: serde_json::to_vec(&json!({
                        "game_id": game,
                        "schema_version": "1",
                        "snapshots": [],
                        "static_artifacts": [
                            {"name": "templates_equipment_catalog", "present": false},
                            {"name": "localisation_join", "present": false},
                            {"name": "decisions_static_definitions", "present": false},
                            {"name": "idea_tech_loc_xref_static", "present": false},
                            {"name": "ship_definitions_static", "present": false},
                            {"name": "gfx_icon_asset_manifest_static", "present": false},
                            {"name": "technology_sharing_static", "present": false},
                            {"name": "focus_defs_static", "present": false}
                        ]
                    }))
                    .unwrap(),
                    role: "index",
                    hour_index: None,
                },
                PackEntry {
                    path: format!("payload/{game}/timeline.json"),
                    bytes: serde_json::to_vec(&json!({
                        "format": "hoi4_compact_stream_timeline_v1",
                        "stream_format": "compact",
                        "stream": stream_reference,
                        "session_manifest": manifest_reference,
                        "raw_stream_role": "collector_compact_stream",
                        "session_manifest_role": "recorder_manifest"
                    }))
                    .unwrap(),
                    role: "timeline",
                    hour_index: None,
                },
                PackEntry {
                    path: stream_path,
                    bytes: raw.to_vec(),
                    role: snapshot::COMPACT_STREAM_ROLE,
                    hour_index: None,
                },
                PackEntry {
                    path: manifest_path,
                    bytes: manifest.clone(),
                    role: "recorder_manifest",
                    hour_index: None,
                },
            ],
            &PackIdentity {
                schema_version: Some("1".to_string()),
                content_manifest: Some(content_manifest),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        fs::write(root.join("native_session.battleplan"), &package).unwrap();
        let value = json!({
            "schema": ROLLOVER_PACKAGE_SET_SCHEMA,
            "source_descriptor": "logical-streams/logical-stream-set.json",
            "source_descriptor_sha256": hash(&source),
            "ordered_streams": [{
                "logical_stream_id": 0,
                "campaign_id_hex": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
                "cid1_payload_sha256_hex": "22".repeat(32),
                "raw_relative_path": "logical-streams/logical-stream-0.sealed.h4cr",
                "raw_bytes": storage.as_ref().map_or(raw.len() as u64, |(bytes, _)| *bytes),
                "raw_sha256": storage
                    .as_ref()
                    .map_or_else(|| hash(raw), |(_, digest)| digest.clone()),
                "raw_decoded_bytes": raw.len(),
                "raw_decoded_sha256": hash(raw),
                "manifest_relative_path": "native_session_manifest.json",
                "manifest_sha256": hash(&manifest),
                "package_relative_path": "native_session.battleplan",
                "package_sha256": hash(&package),
                "recovery": false
            }],
            "transitions": []
        });
        let descriptor = root.join(ROLLOVER_PACKAGE_SET_FILE);
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        // Everything under `root` was written moments ago, and `open` plus
        // `select` read the descriptor, the manifests and the packages. The
        // guards go back to the caller rather than being leaked: this binary
        // runs every test in the crate, and handles held for the process
        // lifetime would accumulate across all of them.
        let guards = portable_test_fs_retry::tree_read_guards(&root)
            .expect("hold the package tree open across the scanner's window");
        RolloverFixture::new(root, descriptor, guards)
    }

    #[test]
    fn explicit_selection_authenticates_one_independent_package() {
        let fixture = fixture("select");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let set = RolloverPackageSet::open(&descriptor).unwrap();
        assert_eq!(set.ordered_streams()[0].logical_stream_id, 0);
        let selected = set.select(0).unwrap();
        assert_eq!(selected.path(), fs::canonicalize(root.join("native_session.battleplan")).unwrap());
        drop(selected);
        drop(set);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pathname_replacement_after_selection_cannot_change_replay_source() {
        let fixture = fixture("replace_after_admission");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let selected = RolloverPackageSet::open(&descriptor).unwrap().select(0).unwrap();
        let selected_path = selected.path().to_path_buf();
        #[cfg(windows)]
        fs::rename(&selected_path, root.join("selected-object-retained.battleplan"))
            .expect_err("the retained authenticated handle must deny pathname replacement");
        #[cfg(not(windows))]
        {
            fs::rename(&selected_path, root.join("selected-object-retained.battleplan")).unwrap();
            fs::write(&selected_path, b"replacement is not the selected replay").unwrap();
        }
        let spool = root.join("spool");
        fs::create_dir(&spool).unwrap();

        let source = crate::source::open_container_with_spooled_decoder(
            selected.into_container(),
            &spool,
            &FixtureSpoolDecoder,
        )
        .expect("replay source must consume the retained selected container");
        let frame = source.frame(7).unwrap();
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.7"));
        assert_eq!(
            frame.rest.get("retained_object_marker").and_then(serde_json::Value::as_str),
            Some("RETAINED_OBJECT")
        );
        drop(source);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// The packaged container holds DECODED H4CR, so admission joins on the
    /// decoded pair.
    ///
    /// A member whose spool was compressed has a storage length and digest that
    /// are nothing like its decoded ones. Joining the container's compact-stream
    /// entry to the storage pair does not corrupt anything — it rejects every
    /// such member as a disagreement, which reads as a tampered package rather
    /// than as the wrong field being compared.
    #[test]
    fn admission_joins_the_container_entry_to_the_decoded_member_identity() {
        let fixture = fixture_with_storage(
            "compressed_storage",
            Some((11, "5c".repeat(32))),
        );
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let set = RolloverPackageSet::open(&descriptor).unwrap();
        let segment = &set.ordered_streams()[0];
        assert_ne!(
            segment.raw_sha256, segment.raw_decoded_sha256,
            "fixture must present a member whose two identities differ"
        );
        set.select(0)
            .expect("a member whose storage identity differs from its decoded identity");
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// And the decoded pair is still checked: a package set that misstates it
    /// is refused rather than accepted because some other field matched.
    #[test]
    fn a_misstated_decoded_identity_is_still_refused() {
        let fixture = fixture("decoded_tamper");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let mut value: serde_json::Value =
            serde_json::from_slice(&portable_test_fs_retry::read(&descriptor).unwrap()).unwrap();
        value["ordered_streams"][0]["raw_decoded_sha256"] = json!("6d".repeat(32));
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = portable_test_fs_retry::with_read_guard(&descriptor, || {
            RolloverPackageSet::open(&descriptor)
        })
        .expect("hold the rewritten descriptor open across the scanner's window")
            .unwrap()
            .select(0)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("compact-stream entry disagrees"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn descriptor_never_defaults_to_segment_zero() {
        let fixture = fixture("explicit");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let error = authenticate_rollover_segment(&descriptor, None).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Unsupported(_)),
            "missing explicit selection is a usage refusal, not an integrity failure: {error:?}"
        );
        assert!(error.to_string().contains("select one explicitly"), "{error}");
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selected_package_hash_is_checked_before_replay_open() {
        let fixture = fixture("package_tamper");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let set = RolloverPackageSet::open(&descriptor).unwrap();
        let replacement_game = "valid-but-unselected-package";
        let replacement = pack(
            replacement_game,
            &[PackEntry {
                path: format!("payload/{replacement_game}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": replacement_game,
                    "schema_version": "1",
                    "snapshots": []
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            }],
            &PackIdentity {
                schema_version: Some("1".to_string()),
                ..Default::default()
            },
            true,
        )
        .unwrap();
        fs::write(root.join("native_session.battleplan"), replacement).unwrap();
        let error = set.select(0).unwrap_err();
        // R7 F1 regression: a SUBSTITUTED member is an integrity failure, not
        // "recognized but unplayable by this build".
        assert!(
            matches!(error, RolloverAuthenticationError::Integrity(_)),
            "substituted package member must classify as Integrity: {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("selected replay package retained-handle receipt disagrees"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn post_pack_external_manifest_mutation_cannot_become_rollover_authority() {
        let fixture = fixture("external_manifest_mutation");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let manifest_path = root.join("native_session_manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&portable_test_fs_retry::read(&manifest_path).unwrap()).unwrap();
        manifest["host_finalize"] = json!({"post_pack_only": true});
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        fs::write(&manifest_path, &manifest_bytes).unwrap();

        // Even a descriptor rewritten to trust the post-pack file cannot make
        // those bytes equal the recorder_manifest already sealed in the
        // selected container.
        let mut value: serde_json::Value =
            serde_json::from_slice(&portable_test_fs_retry::read(&descriptor).unwrap()).unwrap();
        value["ordered_streams"][0]["manifest_sha256"] = json!(hash(&manifest_bytes));
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = portable_test_fs_retry::with_read_guard(&descriptor, || {
            RolloverPackageSet::open(&descriptor)
        })
        .expect("hold the rewritten descriptor open across the scanner's window")
            .unwrap()
            .select(0)
            .unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Integrity(_)),
            "post-pack manifest substitution must classify as Integrity: {error:?}"
        );
        assert!(
            error.to_string().contains("recorder_manifest metadata disagrees"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_member_is_never_admitted_to_product_playback() {
        let fixture = fixture("recovery");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let mut value: serde_json::Value = serde_json::from_slice(&portable_test_fs_retry::read(
            &descriptor
        ).unwrap()).unwrap();
        value["ordered_streams"][0]["recovery"] = json!(true);
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        let set = portable_test_fs_retry::with_read_guard(&descriptor, || RolloverPackageSet::open(&descriptor))
            .expect("hold the rewritten descriptor open across the scanner's window")
            .unwrap();
        let error = set.select(0).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Unsupported(_)),
            "a recovery-only set is a selection refusal, not a broken recording: {error:?}"
        );
        assert!(error.to_string().contains("recovery-only artifact"), "{error}");
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_fields_and_non_contiguous_ids_fail_closed() {
        let fixture = fixture("strict");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let mut value: serde_json::Value = serde_json::from_slice(&portable_test_fs_retry::read(
            &descriptor
        ).unwrap()).unwrap();
        value["unexpected"] = json!(true);
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(portable_test_fs_retry::with_read_guard(&descriptor, || RolloverPackageSet::open(&descriptor))
            .expect("hold the rewritten descriptor open across the scanner's window")
            .is_err());
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_recording_rejects_irrelevant_segment_selector() {
        let error = authenticate_rollover_segment(Path::new("ordinary.battleplan"), Some(0)).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Unsupported(_)),
            "an irrelevant segment selector is a usage refusal: {error:?}"
        );
        assert!(
            error.to_string().contains("requires a logical-stream-package-set.json"),
            "{error}"
        );
    }

    /// R7 F1 regression: a TAMPERED rollover member (bytes disagree with the
    /// SHA-256 the package-set authority names for them) must classify as an
    /// integrity failure — never as a selection/capability refusal that the
    /// product boundary would headline "recording not playable by this build".
    #[test]
    fn tampered_manifest_member_classifies_as_integrity() {
        let fixture = fixture("manifest_tamper");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let manifest_path = root.join("native_session_manifest.json");
        let mut bytes = portable_test_fs_retry::read(&manifest_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&manifest_path, &bytes).unwrap();
        let error = authenticate_rollover_segment(&descriptor, Some(0)).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Integrity(_)),
            "tampered manifest bytes must classify as Integrity: {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("selected recorder manifest SHA-256 differs"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// R7 F1 regression: a tampered durable source descriptor (its SHA-256 is
    /// pinned by the package-set descriptor) is likewise an integrity failure.
    #[test]
    fn tampered_source_descriptor_classifies_as_integrity() {
        let fixture = fixture("source_tamper");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let source_path = root.join("logical-streams/logical-stream-set.json");
        let mut bytes = portable_test_fs_retry::read(&source_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&source_path, &bytes).unwrap();
        let error = authenticate_rollover_segment(&descriptor, Some(0)).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Integrity(_)),
            "tampered source descriptor must classify as Integrity: {error:?}"
        );
        assert!(
            error.to_string().contains("source descriptor SHA-256 differs"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// Codex round-1 catch: a well-formed descriptor naming a schema this
    /// build does not know is "recognized but not playable by this build" —
    /// the Unsupported class, exactly as the pre-typed boundary reported it —
    /// never a malformed-descriptor Structure error.
    #[test]
    fn unknown_package_set_schema_classifies_as_unsupported() {
        let fixture = fixture("future_schema");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        let mut value: serde_json::Value =
            serde_json::from_slice(&portable_test_fs_retry::read(&descriptor).unwrap()).unwrap();
        value["schema"] = json!("cid1-rollover-package-set-v2");
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = authenticate_rollover_segment(&descriptor, Some(0)).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Unsupported(_)),
            "an unknown package-set schema is a build-capability refusal: {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("unsupported rollover package-set schema"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// Codex round-1 catch: a package member tampered so badly its ZIP no
    /// longer parses fails the container open STRUCTURALLY before the
    /// whole-file receipt exists — but the bytes still lost against the
    /// package-set's declared SHA-256, so the classification must be
    /// Integrity, not Structure.
    #[test]
    fn unparseable_tampered_package_member_classifies_as_integrity() {
        let fixture = fixture("zip_structure_tamper");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        fs::write(
            root.join("native_session.battleplan"),
            b"these bytes are not a zip container at all",
        )
        .unwrap();
        let error = authenticate_rollover_segment(&descriptor, Some(0)).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Integrity(_)),
            "tampered member bytes must classify as Integrity even when the \
             container parser fails structurally first: {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("bytes disagree with the package-set SHA-256"),
            "{error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// A malformed package-set descriptor stays in the container/schema class:
    /// integrity is reserved for bytes that lost against a declared digest.
    #[test]
    fn malformed_descriptor_classifies_as_structure() {
        let fixture = fixture("malformed_descriptor");
        let (root, descriptor) = (fixture.root(), fixture.descriptor());
        fs::write(&descriptor, b"{ not json").unwrap();
        let error = authenticate_rollover_segment(&descriptor, Some(0)).unwrap_err();
        assert!(
            matches!(error, RolloverAuthenticationError::Structure(_)),
            "malformed descriptor must classify as Structure: {error:?}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transition_reason_and_adjacent_identity_join_are_exact() {
        let segment = |id: u64, payload: &str| RolloverPackageSegment {
            logical_stream_id: id,
            campaign_id_hex: "11".repeat(32),
            cid1_payload_sha256_hex: payload.repeat(32),
            raw_relative_path: format!("logical-streams/logical-stream-{id}.sealed.h4cr").into(),
            raw_bytes: 1,
            raw_sha256: "33".repeat(32),
            raw_decoded_bytes: 1,
            raw_decoded_sha256: "33".repeat(32),
            manifest_relative_path: format!("manifests/{id}.json").into(),
            manifest_sha256: "44".repeat(32),
            package_relative_path: format!("packages/{id}.battleplan").into(),
            package_sha256: "55".repeat(32),
            recovery: false,
        };
        let streams = vec![segment(0, "22"), segment(1, "66")];
        let mut transition = RolloverPackageTransition {
            current_logical_stream_id: 0,
            proposed_logical_stream_id: 1,
            last_admitted_h4cr_sequence: 7,
            boundary_game_hour_raw: MIN_GAME_HOUR_RAW,
            reason: "same_pair_time_rollback".to_string(),
            proposal_nonce: 1,
            current_campaign_id_hex: streams[0].campaign_id_hex.clone(),
            proposed_campaign_id_hex: streams[1].campaign_id_hex.clone(),
            current_cid1_payload_sha256_hex: streams[0].cid1_payload_sha256_hex.clone(),
            proposed_cid1_payload_sha256_hex: streams[1].cid1_payload_sha256_hex.clone(),
        };
        validate_package_transitions(&streams, &[transition.clone()]).unwrap();
        transition.reason = "whole_pair_changed".to_string();
        assert!(validate_package_transitions(&streams, &[transition]).is_err());
    }
}
