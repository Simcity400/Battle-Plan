
use serde_json::{Map, Value};
use snapshot::{ExpectedIdentity, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const COMPACT_STREAM_TIMELINE_FORMAT: &str = "hoi4_compact_stream_timeline_v1";
const COMPACT_STREAM_ROLE: &str = snapshot::COMPACT_STREAM_ROLE;
pub const PRODUCT_COMPACT_STREAM_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn admit_product_compact_stream_bytes(bytes: u64) -> Result<(), SourceError> {
    if bytes > PRODUCT_COMPACT_STREAM_MAX_BYTES {
        return Err(SourceError::Unsupported(format!(
            "compact replay expands to {bytes} bytes; shipped playback limit is {} bytes (2 GiB)",
            PRODUCT_COMPACT_STREAM_MAX_BYTES
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecorderSessionBinding {
    active_dlcs: Vec<String>,
    localisation_language: String,
    display_variant: RecorderDisplayVariant,
    campaign_identity: Option<RecordedCampaignIdentity>,
    product_admission: RecorderProductAdmission,
    /// XMAP02 resolved foundation content: the recorded per-family effective
    /// VFS winners. Absent on recordings made before the XMAP02 producer
    /// existed - that is a named gap state, not an error.
    foundation_content: Option<portable_content_manifest::foundation_family::FoundationFamilyContentSet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProductionProfileGuarantee {
    ProductionProfileEnabled,
    FullProfileDomainSchedule,
    RequiredStaticArtifacts,
    ContentBoundStaticArtifacts,
}

impl ProductionProfileGuarantee {
    pub fn token(self) -> &'static str {
        match self {
            ProductionProfileGuarantee::ProductionProfileEnabled => "production_profile_enabled",
            ProductionProfileGuarantee::FullProfileDomainSchedule => {
                "full_profile_domain_schedule"
            }
            ProductionProfileGuarantee::RequiredStaticArtifacts => "required_static_artifacts",
            ProductionProfileGuarantee::ContentBoundStaticArtifacts => {
                "content_bound_static_artifacts"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionProfileAuthority {
    Unstated,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderProductAdmission {
    pub authority: ProductionProfileAuthority,
    pub guarantees_not_held: Vec<ProductionProfileGuarantee>,
    pub profile_domains_not_scheduled: Vec<String>,
    pub missing_required_static_artifacts: Vec<String>,
    pub static_artifacts_not_content_bound: Vec<String>,
}

impl RecorderProductAdmission {
    pub fn is_full_production_profile(&self) -> bool {
        self.guarantees_not_held.is_empty()
    }

    fn unstated() -> Self {
        RecorderProductAdmission {
            authority: ProductionProfileAuthority::Unstated,
            guarantees_not_held: vec![
                ProductionProfileGuarantee::ProductionProfileEnabled,
                ProductionProfileGuarantee::FullProfileDomainSchedule,
            ],
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: Vec::new(),
            static_artifacts_not_content_bound: Vec::new(),
        }
    }

    fn name_guarantee_not_held(&mut self, guarantee: ProductionProfileGuarantee) {
        self.guarantees_not_held.push(guarantee);
        self.guarantees_not_held.sort_unstable();
        self.guarantees_not_held.dedup();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCampaignIdentity {
    pub state: String,
    pub campaign_id: Option<String>,
    pub game_unique_id: Option<String>,
    pub game_unique_seed: Option<String>,
    pub source: Option<String>,
    pub source_build_profile: Option<String>,
    pub observed_game_hour_raw: Option<i32>,
    pub stream_header_sha256: Option<String>,
    pub resolution_note: Option<String>,
    pub witnesses: RecordedCampaignIdentityWitnesses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCampaignIdentityWitnesses {
    pub start_date_raw: Option<u32>,
    pub session: Option<u32>,
    pub scenario: Option<String>,
    pub initial_human_country_tags: Option<Vec<String>>,
    pub modset_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedDecisionCatalog {
    path: String,
    sha256: String,
    content_manifest_digest_sha256: String,
    bytes: Arc<[u8]>,
}

impl PackagedDecisionCatalog {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedStaticArtifact {
    name: String,
    path: String,
    sha256: String,
    content_manifest_digest_sha256: String,
    bytes: Arc<[u8]>,
}

impl PackagedStaticArtifact {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn content_manifest_digest_sha256(&self) -> &str {
        &self.content_manifest_digest_sha256
    }
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    #[cfg(feature = "test_support")]
    pub fn unauthenticated_for_consumer_tests(
        name: &str,
        path: &str,
        sha256: &str,
        content_manifest_digest_sha256: &str,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            sha256: sha256.to_string(),
            content_manifest_digest_sha256: content_manifest_digest_sha256.to_string(),
            bytes: Arc::from(bytes.into_boxed_slice()),
        }
    }
}

const PORTABILITY_EXCLUDED_VALUE: &str = "__portability_excluded__";

fn recorder_display_variant_value(settings_binding: &Value) -> Option<Value> {
    let mut display = settings_binding.get("display_variant")?.as_object()?.clone();
    let attach = settings_binding.get("attach_settings_sha256")?.as_str()?;
    display.insert(
        "settings_file_sha256".to_string(),
        Value::String(attach.to_string()),
    );
    Some(Value::Object(display))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecorderDisplayVariant {
    physical_width: u32,
    physical_height: u32,
    min_gui_width: u32,
    min_gui_height: u32,
    gui_scale_f32_bits: u32,
    refresh_rate_hz: u32,
    max_refresh_rate_hz: Option<u32>,
    fullscreen: bool,
    borderless: bool,
    display_mode: String,
    settings_file_sha256: String,
}

struct MetadataRoute {
    binding: Option<RecorderSessionBinding>,
    timeline: Option<Value>,
    recorder_manifest: Option<Map<String, Value>>,
}

impl MetadataRoute {
    fn admit_static_artifacts(
        &mut self,
        store: &Store,
        game_id: &str,
    ) -> Result<BTreeMap<String, PackagedStaticArtifact>, SourceError> {
        let claims_production_profile = self.binding.as_ref().is_some_and(|binding| {
            binding.product_admission.authority == ProductionProfileAuthority::Enabled
        });
        let roster = validate_packaged_static_artifact_roster(
            store,
            game_id,
            self.recorder_manifest.as_ref(),
            claims_production_profile,
        )?;
        if let Some(binding) = self.binding.as_mut() {
            if binding.product_admission.authority != ProductionProfileAuthority::Unstated {
                let declared = &binding.product_admission.missing_required_static_artifacts;
                let declared_set = declared.iter().cloned().collect::<BTreeSet<_>>();
                if declared_set.len() != declared.len() {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.missing_required_static_artifacts repeats a \
                         name ({declared:?})"
                    )));
                }
                if let Some(unknown) = declared_set.iter().find(|name| {
                    !REQUIRED_PACKAGED_STATIC_ARTIFACTS
                        .iter()
                        .any(|(required, _)| required == name)
                }) {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.missing_required_static_artifacts names \
                         {unknown:?}, which is not a required static artifact"
                    )));
                }
                let advertised = self
                    .recorder_manifest
                    .as_ref()
                    .and_then(|manifest| {
                        manifest
                            .get("production_profile")
                            .and_then(Value::as_object)
                            .and_then(|profile| profile.get("required_static_artifacts"))
                    })
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        SourceError::Open(
                            "recorder production_profile.required_static_artifacts is absent or \
                             is not an array, so what this recorder build required cannot be known"
                                .into(),
                        )
                    })?;
                let advertised_set = advertised
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        value.as_str().map(str::to_string).ok_or_else(|| {
                            SourceError::Open(format!(
                                "recorder production_profile.required_static_artifacts[{index}] \
                                 is not a string"
                            ))
                        })
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                if advertised_set.len() != advertised.len() {
                    return Err(SourceError::Open(
                        "recorder production_profile.required_static_artifacts repeats a name"
                            .into(),
                    ));
                }
                let canonical_current = REQUIRED_PACKAGED_STATIC_ARTIFACTS
                    .iter()
                    .map(|(name, _)| (*name).to_string())
                    .collect::<BTreeSet<_>>();
                let mut declarable_absent = roster.absent.clone();
                match roster.legacy_skipped {
                    None => {
                        if advertised_set != canonical_current {
                            return Err(SourceError::Open(format!(
                                "recorder production_profile.required_static_artifacts \
                                 {advertised_set:?} is not the canonical required set \
                                 {canonical_current:?}"
                            )));
                        }
                    }
                    Some(skipped) => {
                        let canonical_legacy = canonical_current
                            .iter()
                            .filter(|name| name.as_str() != skipped)
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        if advertised_set != canonical_legacy {
                            return Err(SourceError::Open(format!(
                                "static-artifact roster omits {skipped:?} but \
                                 production_profile.required_static_artifacts {advertised_set:?} \
                                 is not the canonical pre-{skipped} required set \
                                 {canonical_legacy:?}; only a recorder build that predates the \
                                 artifact is exempt from declaring it"
                            )));
                        }
                        declarable_absent.remove(skipped);
                    }
                }
                if binding.product_admission.authority == ProductionProfileAuthority::Enabled
                    && !declarable_absent.is_empty()
                {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.enabled is true but the verified package does \
                         not carry {declarable_absent:?}; a capture that claims the production \
                         profile carries every static artifact its recorder build required"
                    )));
                }
                if declared_set != declarable_absent {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.missing_required_static_artifacts \
                         {declared_set:?} disagrees with the artifacts the verified package does \
                         not carry {declarable_absent:?}"
                    )));
                }
            }
            binding.product_admission.missing_required_static_artifacts =
                REQUIRED_PACKAGED_STATIC_ARTIFACTS
                    .iter()
                    .map(|(name, _)| (*name).to_string())
                    .filter(|name| roster.absent.contains(name))
                    .collect();
            if !roster.absent.is_empty() {
                binding
                    .product_admission
                    .name_guarantee_not_held(ProductionProfileGuarantee::RequiredStaticArtifacts);
            }
            if !roster.not_content_bound.is_empty() {
                binding
                    .product_admission
                    .name_guarantee_not_held(ProductionProfileGuarantee::ContentBoundStaticArtifacts);
            }
            binding.product_admission.static_artifacts_not_content_bound =
                roster.not_content_bound;
        }
        Ok(roster.admitted)
    }
}

#[derive(Debug)]
pub enum SourceError {
    Open(String),
    Integrity(String),
    NotFound(String),
    ModsetMismatch(String),
    Unsupported(String),
    UnsupportedCompactStreamOnly(String),
    MultipleGames(Vec<String>),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Open(m)
            | SourceError::NotFound(m)
            | SourceError::ModsetMismatch(m)
            | SourceError::Integrity(m) => {
                write!(f, "open: {m}")
            }
            SourceError::Unsupported(m) | SourceError::UnsupportedCompactStreamOnly(m) => {
                write!(f, "unsupported recording: {m}")
            }
            SourceError::MultipleGames(games) => write!(
                f,
                "store holds {} games ({}); select one explicitly with open_recording_game",
                games.len(),
                games.join(", ")
            ),
        }
    }
}
impl std::error::Error for SourceError {}

fn is_integrity_failure(error: &snapshot::LoadError) -> bool {
    let snapshot::LoadError::Container(container) = error else {
        return false;
    };
    is_container_integrity_failure(container)
}

pub(crate) fn is_container_integrity_failure(container: &snapshot::ContainerError) -> bool {
    match container {
        snapshot::ContainerError::PayloadIntegrity(_) => true,
        snapshot::ContainerError::Zip(zip) => matches!(
            zip,
            snapshot::zip::ZipError::PayloadIntegrity(_)
                | snapshot::zip::ZipError::SizeMismatch { .. }
                | snapshot::zip::ZipError::Decompress(_)
        ),
        _ => false,
    }
}

fn source_error_from_load(error: &snapshot::LoadError, message: String) -> SourceError {
    match error {
        snapshot::LoadError::Container(snapshot::ContainerError::ModsetMismatch { .. }) => {
            SourceError::ModsetMismatch(message)
        }
        error if is_integrity_failure(error) => SourceError::Integrity(message),
        _ => SourceError::Open(message),
    }
}

fn source_error_from_container(error: &snapshot::ContainerError, message: String) -> SourceError {
    match error {
        snapshot::ContainerError::ModsetMismatch { .. } => SourceError::ModsetMismatch(message),
        error if is_container_integrity_failure(error) => SourceError::Integrity(message),
        _ => SourceError::Open(message),
    }
}

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub hour_index: i64,
    pub date: Option<String>,
    pub save_version: Option<String>,
    pub schema_version: Option<Value>,
    pub rest: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredCoverageGap {
    pub start_hour: i64,
    pub end_hour: i64,
    pub kind: String,
    pub reason: String,
    pub evidence_id: String,
    pub segment_id: Option<u32>,
    /// one. Opaque here: replay ingest never reinterprets recorder ownership.
    pub owner: Option<Value>,
    /// or not at all (`add_main_ownership_fields` in the stitcher), so a
    pub selection: Option<DeclaredCoverageSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredCoverageSelection {
    pub selection_rule: String,
    pub selected_observation_id: String,
    pub source_record_seq_bounds: (u64, u64),
    pub source_byte_bounds: (u64, u64),
}

impl DeclaredCoverageGap {
    pub fn covers(&self, hour: i64) -> bool {
        self.start_hour <= hour && hour <= self.end_hour
    }
    pub fn hour_count(&self) -> u64 {
        self.end_hour.abs_diff(self.start_hour).saturating_add(1)
    }
    pub fn affects_all_domains(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeclaredCoverage {
    pub index_schema: Option<String>,
    pub segments_file: Option<String>,
    pub main_head: Option<Value>,
    pub gaps: Vec<DeclaredCoverageGap>,
}

impl DeclaredCoverage {
    pub fn is_stitched(&self) -> bool {
        self.index_schema.as_deref() == Some(STITCHED_INDEX_SCHEMA)
    }
    pub fn gap_at(&self, hour: i64) -> Option<&DeclaredCoverageGap> {
        self.gaps.iter().find(|gap| gap.covers(hour))
    }
    pub fn is_empty(&self) -> bool {
        self.index_schema.is_none()
            && self.segments_file.is_none()
            && self.main_head.is_none()
            && self.gaps.is_empty()
    }
}

const STITCHED_INDEX_SCHEMA: &str = "stitched-main-v1";

const DECLARED_COVERAGE_KINDS: &[&str] = &["gap", "capture_failed", "coalesced"];

fn read_declared_coverage(
    game_id: &str,
    index: &snapshot::Index,
) -> Result<DeclaredCoverage, SourceError> {
    let is_stitched = index.is_stitched();
    let mut gaps: Vec<DeclaredCoverageGap> = Vec::with_capacity(index.coverage.len());
    for (ordinal, row) in index.coverage.iter().enumerate() {
        let context = format!("recording {game_id:?} index.coverage[{ordinal}]");
        let row = row.as_object().ok_or_else(|| {
            SourceError::Open(format!("{context} is not an object; refusing to drop it"))
        })?;
        let field_i64 = |name: &str| -> Result<i64, SourceError> {
            row.get(name).and_then(Value::as_i64).ok_or_else(|| {
                SourceError::Open(format!("{context}.{name} is missing or not an integer"))
            })
        };
        let field_text = |name: &str| -> Result<String, SourceError> {
            row.get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    SourceError::Open(format!("{context}.{name} is missing, blank, or not a string"))
                })
        };
        let field_u64_pair = |name: &str| -> Result<(u64, u64), SourceError> {
            let bad = || SourceError::Open(format!("{context}.{name} is not a [u64, u64] bound"));
            let values = row.get(name).and_then(Value::as_array).ok_or_else(bad)?;
            match values.as_slice() {
                [first, second] => Ok((
                    first.as_u64().ok_or_else(bad)?,
                    second.as_u64().ok_or_else(bad)?,
                )),
                _ => Err(bad()),
            }
        };
        let start_hour = field_i64("start_hour")?;
        let end_hour = field_i64("end_hour")?;
        if start_hour > end_hour {
            return Err(SourceError::Open(format!(
                "{context} spans {start_hour}..={end_hour} (end before start)"
            )));
        }
        let kind = field_text("kind")?;
        if kind == "snapshot" {
            return Err(SourceError::Open(format!(
                "{context} declares kind \"snapshot\"; a coverage row is an absence, not a frame"
            )));
        }
        if !DECLARED_COVERAGE_KINDS.contains(&kind.as_str()) {
            return Err(SourceError::Open(format!(
                "{context} declares unknown coverage kind {kind:?} (admitted kinds: {})",
                DECLARED_COVERAGE_KINDS.join(", ")
            )));
        }
        let segment_id = match row.get("segment_id") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        SourceError::Open(format!("{context}.segment_id is not a segment ordinal"))
                    })?,
            ),
        };
        let owner = match row.get("owner") {
            None | Some(Value::Null) => None,
            Some(value) if value.is_object() => Some(value.clone()),
            Some(_) => {
                return Err(SourceError::Open(format!(
                    "{context}.owner is present but is not an object"
                )))
            }
        };
        let selection_members = [
            "selection_rule",
            "selected_observation_id",
            "source_record_seq_bounds",
            "source_byte_bounds",
        ];
        let present = selection_members
            .iter()
            .filter(|name| row.get(**name).is_some_and(|value| !value.is_null()))
            .count();
        let selection = match present {
            0 => None,
            count if count == selection_members.len() => Some(DeclaredCoverageSelection {
                selection_rule: field_text("selection_rule")?,
                selected_observation_id: field_text("selected_observation_id")?,
                source_record_seq_bounds: field_u64_pair("source_record_seq_bounds")?,
                source_byte_bounds: field_u64_pair("source_byte_bounds")?,
            }),
            count => {
                return Err(SourceError::Open(format!(
                    "{context} carries {count} of {} selection-evidence members; a partial \
                     selection bundle is not admissible provenance",
                    selection_members.len()
                )))
            }
        };
        // (`add_main_ownership_fields` emits both unconditionally). It does NOT
        if is_stitched && (segment_id.is_none() || owner.is_none()) {
            return Err(SourceError::Open(format!(
                "{context} is a stitched-index coverage row but omits its owning segment or its \
                 observation owner; a stitched gap names who owns the hours it declares absent"
            )));
        }
        if let Some(previous) = gaps.last() {
            if start_hour <= previous.end_hour {
                return Err(SourceError::Open(format!(
                    "{context} starts at {start_hour}, at or before the previous declared gap end \
                     {} (coverage must be ascending and non-overlapping)",
                    previous.end_hour
                )));
            }
        }
        gaps.push(DeclaredCoverageGap {
            start_hour,
            end_hour,
            kind,
            reason: field_text("reason")?,
            evidence_id: field_text("evidence_id")?,
            segment_id,
            owner,
            selection,
        });
    }
    Ok(DeclaredCoverage {
        index_schema: index.index_schema.clone(),
        segments_file: index.segments_file.clone(),
        main_head: index.main_head.clone(),
        gaps,
    })
}

pub trait ReplaySource: Send + Sync {
    fn game_id(&self) -> &str;
    fn hours(&self) -> &[i64];
    fn warnings(&self) -> &[String];
    fn declared_coverage(&self) -> Option<&DeclaredCoverage> {
        None
    }
    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        None
    }
    /// Recorded XMAP02 resolved foundation content, when the recording's
    /// recorder session manifest carries it.
    fn foundation_content(
        &self,
    ) -> Option<&portable_content_manifest::foundation_family::FoundationFamilyContentSet>
    {
        None
    }
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        None
    }
    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        None
    }
    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        None
    }
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        None
    }
    fn static_artifact(&self, _name: &str) -> Option<&PackagedStaticArtifact> {
        None
    }
    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        None
    }
    fn date_at(&self, _hour_index: i64) -> Result<Option<String>, SourceError> {
        Ok(None)
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError>;

    fn set_scrub_interrupt(&self, _interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {}
}

pub fn open_recording(path: &Path) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, None, None)
}

pub fn open_recording_game(
    path: &Path,
    game_id: &str,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), None, None)
}

pub fn open_recording_with_decoder(
    path: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, Some(decoder), None)
}

pub fn open_recording_game_with_decoder(
    path: &Path,
    game_id: &str,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), Some(decoder), None)
}

/// and retains it only for the lifetime of the returned lazy source.
pub fn open_recording_with_spooled_decoder(
    path: &Path,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, None, Some((spool_base, decoder)))
}

pub fn open_container_with_spooled_decoder(
    container: snapshot::Container,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_store_impl(
        Store::from_container(container),
        "authenticated retained replay container",
        None,
        None,
        Some((spool_base, decoder)),
    )
}

pub fn open_recording_game_with_spooled_decoder(
    path: &Path,
    game_id: &str,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), None, Some((spool_base, decoder)))
}

pub fn open_prefolded_replay(
    replay: snapshot::PrefoldedReplay,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    validate_index_schema_version(Some(replay.index().schema_version()))?;
    Ok(Box::new(PrefoldedSource {
        replay,
        warnings: Vec::new(),
        store: None,
        recorder_binding: None,
        design_catalog: None,
        decision_catalog: None,
        static_artifacts: BTreeMap::new(),
        prefold_receipt: None,
    }))
}

fn open_recording_impl(
    path: &Path,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl_with_post_anchor(path, requested_game, decoder, spool_decoder, |_| {})
}

fn open_recording_impl_with_post_anchor<F>(
    path: &Path,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
    post_anchor: F,
) -> Result<Box<dyn ReplaySource>, SourceError>
where
    F: FnOnce(&Store),
{
    // Acquire the object's metadata ONCE and classify from it. `is_file()` /
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SourceError::NotFound(format!(
                "path does not exist (expected a .battleplan file or a snapshot store directory): {}",
                path.display()
            )))
        }
        Err(error) => {
            return Err(SourceError::Open(format!(
                "inspect recording path {}: {error}",
                path.display()
            )))
        }
    };
    let store = if metadata.is_file() {
        Store::open_replay(path, &ExpectedIdentity::default()).map_err(|e| {
            source_error_from_load(&e, format!("open .battleplan: {e}"))
        })?
    } else if metadata.is_dir() {
        Store::open_directory(path)
            .map_err(|e| source_error_from_load(&e, format!("anchor directory store: {e}")))?
    } else {
        return Err(SourceError::Open(format!(
            "recording path is neither a .battleplan file nor a snapshot store directory: {}",
            path.display()
        )));
    };
    post_anchor(&store);

    open_store_impl(
        store,
        &path.display().to_string(),
        requested_game,
        decoder,
        spool_decoder,
    )
}

fn open_store_impl(
    store: Store,
    source_label: &str,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
) -> Result<Box<dyn ReplaySource>, SourceError> {

    let games = store
        .games()
        .map_err(|e| SourceError::Open(format!("list games in store: {e}")))?;
    let game_id = match requested_game {
        Some(g) => {
            if !games.iter().any(|have| have == g) {
                return Err(SourceError::Open(format!(
                    "game {g:?} not found in store (available: {})",
                    if games.is_empty() {
                        "none".to_string()
                    } else {
                        games.join(", ")
                    }
                )));
            }
            g.to_string()
        }
        None => match games.len() {
            0 => {
                return Err(SourceError::Open(format!(
                    "no game found (no <game_id>/index.json) under {source_label}"
                )))
            }
            1 => games[0].clone(),
            _ => return Err(SourceError::MultipleGames(games)),
        },
    };

    let mut metadata = recorder_metadata_route(&store, &game_id)?;
    let index = store
        .index(&game_id)
        .map_err(|e| source_error_from_load(&e, format!("read index for game {game_id:?}: {e}")))?;
    validate_recorder_portable_container_contract(&store, &game_id, &index, &metadata)?;

    let declared_coverage = read_declared_coverage(&game_id, &index)?;

    if index.snapshots.is_empty() {
        if !declared_coverage.gaps.is_empty() {
            return Err(SourceError::Unsupported(format!(
                "recording {game_id:?} declares {} index coverage gap(s) but has no snapshot index \
                 entries; this build cannot carry declared coverage across the \
                 compact-stream/replay-cache routes",
                declared_coverage.gaps.len()
            )));
        }
        if packaged_replay_cache_roles_present(&store) {
            return open_packaged_replay_cache(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                spool_decoder.map(|(base, _)| base),
            );
        }
        if let Some((spool_base, spool_decoder)) = spool_decoder {
            return open_compact_stream_with_spooled_decoder(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                spool_base,
                spool_decoder,
            );
        }
        if let Some(decoder) = decoder {
            return open_compact_stream_with_decoder(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                decoder,
            );
        }
        return Err(empty_index_error(
            &store,
            metadata.timeline.as_ref(),
            &game_id,
        ));
    }
    validate_index_schema_version(index.schema_version.as_ref())?;

    let mut files_by_hour: BTreeMap<i64, String> = BTreeMap::new();
    let mut dates_by_hour: BTreeMap<i64, Option<String>> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    for entry in &index.snapshots {
        match entry.hour_index {
            None => warnings.push(format!(
                "index entry {:?} has no hour_index; skipped (never guessed)",
                entry.file
            )),
            Some(hour) => {
                if let Some(existing) = files_by_hour.get(&hour) {
                    return Err(SourceError::Open(format!(
                        "recording {game_id:?} index names hour_index {hour} twice \
                         ({existing:?} and {:?}); the index is the canonical authority for an \
                         hour and this build refuses to choose between two claims",
                        entry.file
                    )));
                }
                files_by_hour.insert(hour, entry.file.clone());
                dates_by_hour.insert(hour, entry.date.clone());
            }
        }
    }
    if index.is_stitched() {
        if let Some((hour, _)) = dates_by_hour
            .iter()
            .find(|(_, date)| date.as_deref().is_none_or(|date| date.trim().is_empty()))
        {
            return Err(SourceError::Open(format!(
                "stitched recording {game_id:?} visible snapshot hour {hour} carries no index \
                 date; a stitched index must date every visible hour (date navigation reads the \
                 index, never a frame body)"
            )));
        }
    }
    for gap in &declared_coverage.gaps {
        if let Some((hour, file)) = files_by_hour
            .range(gap.start_hour..=gap.end_hour)
            .next()
            .map(|(hour, file)| (*hour, file.clone()))
        {
            return Err(SourceError::Open(format!(
                "recording {game_id:?} declares hours {}..={} absent ({}: {}) yet its index also \
                 names a playable snapshot {file:?} at hour {hour}",
                gap.start_hour, gap.end_hour, gap.kind, gap.reason
            )));
        }
    }
    if files_by_hour.is_empty() {
        return Err(SourceError::Unsupported(format!(
            "recording {game_id:?} has {} snapshot index entries but none carries an \
             hour_index; nothing is playable (hour_index is the canonical key and is \
             never guessed)",
            index.snapshots.len()
        )));
    }

    let hours: Vec<i64> = files_by_hour.keys().copied().collect();
    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(StoreSource {
        store,
        game_id,
        hours,
        files_by_hour,
        dates_by_hour,
        declared_coverage,
        warnings,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
    }))
}

fn open_compact_stream_with_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    mut metadata: MetadataRoute,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let identity_mode = prefold_stream_identity_mode(&metadata);
    let container = store.container().ok_or_else(|| {
        SourceError::Unsupported(format!(
            "recording {game_id:?} is a compact-stream-only directory store; the portable decoder seam accepts authenticated .battleplan container role entries only"
        ))
    })?;
    let entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
    let entry = match entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(SourceError::Unsupported(
                match declared_raw_stream_packaging(metadata.recorder_manifest.as_ref()) {
                    Ok(snapshot::RawStreamPackaging::Omitted) => format!(
                        "recording {game_id:?} {}",
                        snapshot::raw_stream_unavailable_message(
                            "be folded through the raw-stream decoder seam"
                        ),
                    ),
                    _ => format!(
                        "recording {game_id:?} has an empty snapshot index but no manifest entry with role {:?}",
                        snapshot::COMPACT_STREAM_ROLE
                    ),
                },
            ))
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recording {game_id:?} has {} manifest entries with role {:?}; expected exactly one",
                entries.len(),
                snapshot::COMPACT_STREAM_ROLE
            )))
        }
    };
    validate_index_schema_version(declared_schema_version.as_ref())?;
    validate_compact_stream_entry_identity(&entry.path, &game_id, metadata.timeline.as_ref())?;
    validate_recorder_compact_stream_join(
        &entry.path,
        entry.bytes,
        &entry.sha256,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )?;
    admit_product_compact_stream_bytes(entry.bytes)?;
    let raw_h4cr = container.read_entry_bytes(&entry.path).map_err(|error| {
        let message = format!(
            "read verified compact stream role entry {:?}: {error}",
            entry.path
            );
        source_error_from_load(&snapshot::LoadError::Container(error), message)
    })?;
    let request = snapshot::CertifiedPrefoldRequest::try_new_with_identity_mode(
        game_id.clone(),
        raw_h4cr,
        identity_mode,
    )
    .map_err(|error| {
            SourceError::Open(format!(
                "compact stream role entry {:?} violates the portable pre-fold request: {error}",
                entry.path
            ))
        })?;
    let certified = snapshot::decode_certified_prefolded(decoder, request).map_err(|error| {
        SourceError::Open(format!(
            "portable decoder {:?} rejected compact stream role entry {:?}: {error}",
            decoder.decoder_id(),
            entry.path
        ))
    })?;
    validate_index_schema_version(Some(certified.index().schema_version()))?;
    if let Some(declared) = declared_schema_version.as_ref() {
        if !same_supported_schema_version(declared, certified.index().schema_version()) {
            return Err(SourceError::Open(format!(
                "compact-only index schema_version {declared} does not match decoder {:?} output schema_version {}",
                decoder.decoder_id(),
                certified.index().schema_version()
            )));
        }
    }
    validate_prefolded_container_identity(container, certified.replay())?;

    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        certified.replay().index().hours(),
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    let prefold_receipt = Some(certified.receipt().clone());
    Ok(Box::new(PrefoldedSource {
        replay: certified.into_replay(),
        warnings: Vec::new(),
        store: Some(store),
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
    }))
}

fn packaged_replay_cache_roles_present(store: &Store) -> bool {
    store.container().is_some_and(|container| {
        !container
            .entries_by_role(snapshot::HOUR_INDEX_ROLE)
            .is_empty()
            || !container
                .entries_by_role(snapshot::CHECKPOINT_CACHE_ROLE)
                .is_empty()
    })
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn declared_raw_stream_packaging(
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<snapshot::RawStreamPackaging, SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(snapshot::RawStreamPackaging::Packaged);
    };
    snapshot::read_raw_stream_packaging_with_legacy_default(recorder_manifest).map_err(|error| {
        SourceError::Open(format!("recorder manifest states no usable composition: {error}"))
    })
}

fn validate_no_stream_path_is_claimed(
    recorder_manifest: Option<&Map<String, Value>>,
    timeline: Option<&Value>,
) -> Result<(), SourceError> {
    let claimed = |label: &str| {
        SourceError::Open(format!(
            "recording declares the {:?} composition but its {label} still names a packaged \
             stream path",
            snapshot::RawStreamPackaging::Omitted.as_str(),
        ))
    };
    if let Some(value) = recorder_manifest.and_then(|manifest| manifest.get("replay_entries")) {
        let entries = value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_entries authority is not an object".to_string())
        })?;
        if entries.contains_key("stream") {
            return Err(claimed("recorder manifest replay_entries"));
        }
    }
    if let Some(timeline) = timeline {
        let timeline = timeline.as_object().ok_or_else(|| {
            SourceError::Open("recorder-owned timeline is not an object".to_string())
        })?;
        if timeline.contains_key("stream") {
            return Err(claimed("timeline"));
        }
    }
    Ok(())
}

fn recorder_manifest_source_identity(
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<(u64, String), SourceError> {
    let missing = || {
        SourceError::Open(format!(
            "recording declares the {:?} composition, so its source identity must come from the \
             recorder manifest, but stream_storage.decoded_bytes/decoded_sha256 is missing or \
             malformed",
            snapshot::RawStreamPackaging::Omitted.as_str(),
        ))
    };
    let storage = recorder_manifest
        .ok_or_else(missing)?
        .get("stream_storage")
        .and_then(Value::as_object)
        .ok_or_else(missing)?;
    let bytes = storage
        .get("decoded_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(missing)?;
    let sha256 = storage
        .get("decoded_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_sha256(value))
        .ok_or_else(missing)?
        .to_string();
    Ok((bytes, sha256))
}

fn open_packaged_replay_cache(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    mut metadata: MetadataRoute,
    cache_spool_base: Option<&Path>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    validate_index_schema_version(declared_schema_version.as_ref())?;
    let identity_mode = prefold_stream_identity_mode(&metadata);
    let mut cache_spool: Option<PrefoldSpoolOwner> = None;
    let (cache, prefold_receipt, hours) = {
        let container = store.container().ok_or_else(|| {
            SourceError::Unsupported(
                "packaged replay_decoder artifacts require a portable container authority"
                    .to_string(),
            )
        })?;
        let select = |role: &str| -> Result<snapshot::container::ContainerEntry, SourceError> {
            match container.entries_by_role(role).as_slice() {
                [entry] => Ok(entry.clone()),
                [] => Err(SourceError::Open(format!(
                    "packaged replay_decoder container omits required role {role:?}"
                ))),
                entries => Err(SourceError::Open(format!(
                    "packaged replay_decoder container has {} entries for unique role {role:?}",
                    entries.len()
                ))),
            }
        };
        let index_entry = select(snapshot::HOUR_INDEX_ROLE)?;
        let cache_entry = select(snapshot::CHECKPOINT_CACHE_ROLE)?;
        let expected_prefix = format!("payload/{game_id}/");
        let expected_index_path = format!("{expected_prefix}{}", snapshot::HOUR_INDEX_FILE);
        let expected_cache_path =
            format!("{expected_prefix}{}", snapshot::CHECKPOINT_CACHE_FILE);
        if index_entry.path != expected_index_path || cache_entry.path != expected_cache_path {
            return Err(SourceError::Open(
                "packaged replay_decoder role paths disagree with the selected game namespace"
                    .to_string(),
            ));
        }
        let packaging = declared_raw_stream_packaging(metadata.recorder_manifest.as_ref())?;
        let stream_entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
        let source_identity = match (packaging, stream_entries.as_slice()) {
            (snapshot::RawStreamPackaging::Packaged, [source_entry]) => {
                validate_compact_stream_entry_identity(
                    &source_entry.path,
                    &game_id,
                    metadata.timeline.as_ref(),
                )?;
                validate_recorder_compact_stream_join(
                    &source_entry.path,
                    source_entry.bytes,
                    &source_entry.sha256,
                    &game_id,
                    metadata.recorder_manifest.as_ref(),
                )?;
                let source_receipt = container
                    .authenticate_entry(&source_entry.path)
                    .map_err(|error| {
                        SourceError::Open(format!(
                            "authenticate canonical H4CR role before replay-cache admission: {error}"
                        ))
                    })?;
                if source_receipt.uncompressed_len() != source_entry.bytes
                    || source_receipt.sha256() != source_entry.sha256
                {
                    return Err(SourceError::Open(
                        "canonical H4CR authentication receipt disagrees with its container role"
                            .to_string(),
                    ));
                }
                (source_entry.bytes, source_entry.sha256.clone())
            }
            (snapshot::RawStreamPackaging::Packaged, []) => {
                return Err(SourceError::Open(format!(
                    "packaged replay_decoder container omits required role {:?}, but the recorder \
                     manifest declares the {:?} composition",
                    snapshot::COMPACT_STREAM_ROLE,
                    snapshot::RawStreamPackaging::Packaged.as_str(),
                )))
            }
            (snapshot::RawStreamPackaging::Omitted, []) => {
                validate_no_stream_path_is_claimed(
                    metadata.recorder_manifest.as_ref(),
                    metadata.timeline.as_ref(),
                )?;
                recorder_manifest_source_identity(metadata.recorder_manifest.as_ref())?
            }
            (snapshot::RawStreamPackaging::Omitted, _) => {
                return Err(SourceError::Open(format!(
                    "recorder manifest declares the {:?} composition, but the container carries \
                     {} entries with role {:?}",
                    snapshot::RawStreamPackaging::Omitted.as_str(),
                    stream_entries.len(),
                    snapshot::COMPACT_STREAM_ROLE,
                )))
            }
            (_, entries) => {
                return Err(SourceError::Open(format!(
                    "packaged replay_decoder container has {} entries for unique role {:?}",
                    entries.len(),
                    snapshot::COMPACT_STREAM_ROLE,
                )))
            }
        };
        let (source_len, source_sha256) = source_identity;
        let index_bytes = container
            .read_entry_bytes(&index_entry.path)
            .map_err(|error| {
                SourceError::Open(format!(
                    "read certified replay_decoder hour index: {error}"
                ))
            })?;
        let (cache, copy_outcome) = match cache_spool_base {
            Some(base) => {
                let owner = PrefoldSpoolOwner::create(base)?;
                let spooled = owner.root().join("checkpoints.h4cp");
                let copy_outcome = {
                    let mut sink = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&spooled)
                        .map_err(|error| {
                            SourceError::Open(format!(
                                "create private checkpoint cache spool {}: {error}",
                                spooled.display()
                            ))
                        })?;
                    let outcome = container.copy_entry_to_unpublished_writer_bounded(
                        &cache_entry.path,
                        &mut sink,
                        snapshot::zip::STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                    );
                    sink.sync_all().map_err(|error| {
                        SourceError::Open(format!(
                            "sync private checkpoint cache spool {}: {error}",
                            spooled.display()
                        ))
                    })?;
                    outcome
                };
                if let Ok(receipt) = &copy_outcome {
                    if receipt.path() != cache_entry.path.as_str()
                        || receipt.uncompressed_len() != cache_entry.bytes
                        || receipt.sha256() != cache_entry.sha256
                    {
                        return Err(SourceError::Open(
                            "checkpoint cache spool receipt does not match its container role entry"
                                .to_string(),
                        ));
                    }
                }
                let file = open_private_prefold_spool_file(&spooled)?;
                let cache = snapshot::CertifiedReplayCache::open_file(
                    &index_bytes,
                    file,
                    source_len,
                    &source_sha256,
                )
                .map_err(|error| {
                    SourceError::Open(admission_refusal(&error, &copy_outcome))
                })?;
                cache_spool = Some(owner);
                (cache, copy_outcome)
            }
            None => {
                let mut cache_bytes = Vec::new();
                let copy_outcome = container.copy_entry_to_unpublished_writer_bounded(
                    &cache_entry.path,
                    &mut cache_bytes,
                    snapshot::zip::STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                );
                let cache = snapshot::CertifiedReplayCache::open(
                    &index_bytes,
                    cache_bytes,
                    source_len,
                    &source_sha256,
                )
                .map_err(|error| {
                    SourceError::Open(admission_refusal(&error, &copy_outcome))
                })?;
                (cache, copy_outcome)
            }
        };
        match (&copy_outcome, cache.confined_hours().is_empty()) {
            (Ok(_), true) => {}
            (Err(_), false) => {}
            (Ok(receipt), false) => {
                return Err(SourceError::Open(format!(
                    "the container verified {} against its own manifest digest {}, yet the \
                     sealed H4IX refuses {} of its hours by per-record digest; the two accounts \
                     of the same bytes disagree, so neither is trusted",
                    cache_entry.path,
                    receipt.sha256(),
                    cache.confined_hours().len(),
                )))
            }
            (Err(copy_error), true) => {
                return Err(SourceError::Open(format!(
                    "the container refused {} ({copy_error}), but every record in it matches the \
                     per-record digest the sealed H4IX publishes; the damage is in the \
                     container's own account of the entry, which serves no single game hour",
                    cache_entry.path,
                )))
            }
        }
        if cache.index().game_id() != game_id
            || cache.index().schema_version() != "1"
            || cache.index().identity_mode()
                != match identity_mode {
                    snapshot::PrefoldStreamIdentityMode::Legacy => {
                        snapshot::CacheIdentityMode::Legacy
                    }
                    snapshot::PrefoldStreamIdentityMode::IdentityV1 => {
                        snapshot::CacheIdentityMode::IdentityV1
                    }
                }
        {
            return Err(SourceError::Open(
                "packaged replay_decoder identity disagrees with container/recorder authority"
                    .to_string(),
            ));
        }
        validate_packaged_replay_cache_index_descriptor(
            &store,
            &game_id,
            cache.index(),
            &index_entry,
            &cache_entry,
            packaging,
        )?;
        let hours = cache
            .index()
            .hours()
            .iter()
            .map(snapshot::ReplayHourEntry::hour_index)
            .collect::<Vec<_>>();
        let receipt = snapshot::PrefoldSourceReceipt::try_new(
            "extractor.replay_decoder.binary_seek.v1",
            source_len,
            source_sha256,
            identity_mode,
        )
        .map_err(|error| SourceError::Open(error.to_string()))?;
        (cache, receipt, hours)
    };

    let first_world = cache
        .reconstruct_value(hours[0])
        .map_err(|error| SourceError::Open(format!("reconstruct first replay hour: {error}")))?
        .0;
    let first = snapshot::Snapshot::from_world_value(first_world).map_err(|error| {
        SourceError::Open(format!(
            "first binary replay reconstruction is not canonical snapshot JSON: {error}"
        ))
    })?;
    validate_snapshot_container_identity(
        store
            .container()
            .expect("packaged replay cache requires a container"),
        &first,
    )?;
    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(IndexedReplaySource {
        cache,
        cache_spool,
        hours,
        warnings: Vec::new(),
        store,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
    }))
}

fn validate_packaged_replay_cache_index_descriptor(
    store: &Store,
    game_id: &str,
    replay_index: &snapshot::ReplayHourIndex,
    index_entry: &snapshot::container::ContainerEntry,
    cache_entry: &snapshot::container::ContainerEntry,
    declared_packaging: snapshot::RawStreamPackaging,
) -> Result<(), SourceError> {
    let bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            let message = format!("read replay index descriptor: {error}");
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| SourceError::Open("replay index descriptor disappeared".to_string()))?;
    let value = snapshot::parse_json_strict(&bytes)
        .map_err(|error| SourceError::Open(format!("strict replay index descriptor: {error}")))?;
    let index_packaging = value
        .as_object()
        .ok_or_else(|| SourceError::Open("replay index descriptor is not an object".to_string()))
        .and_then(|object| {
            snapshot::read_raw_stream_packaging_with_legacy_default(object).map_err(|error| {
                SourceError::Open(format!("replay index descriptor composition: {error}"))
            })
        })?;
    if index_packaging != declared_packaging {
        return Err(SourceError::Open(format!(
            "replay index descriptor declares the {:?} composition but the recorder authority              declares {:?}",
            index_packaging.as_str(),
            declared_packaging.as_str(),
        )));
    }
    let no_legacy_snapshots = value
        .get("snapshots")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
        && value
            .get("production_json_snapshots_omitted")
            .and_then(Value::as_bool)
            == Some(true)
        && value
            .get("canonical_world_source")
            .and_then(Value::as_str)
            == Some(snapshot::COMPACT_STREAM_ROLE)
        && store
            .container()
            .is_some_and(|container| container.entries_by_role("snapshot").is_empty());
    let decoder = value
        .get("replay_decoder")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(
                "empty snapshot index has replay-cache roles but no replay_decoder descriptor"
                    .to_string(),
            )
        })?;
    let hour = decoder
        .get("hour_index")
        .and_then(Value::as_object)
        .ok_or_else(|| SourceError::Open("replay_decoder hour_index descriptor missing".into()))?;
    let checkpoints = decoder
        .get("checkpoints")
        .and_then(Value::as_object)
        .ok_or_else(|| SourceError::Open("replay_decoder checkpoints descriptor missing".into()))?;
    let exact = no_legacy_snapshots
        && decoder.get("schema").and_then(Value::as_str)
        == Some("hoi4.replay_decoder.binary_seek.v1")
        && decoder
            .get("exact_reconstruction_parity")
            .and_then(Value::as_bool)
            == Some(true)
        && decoder
            .get("bounded_seek_certified")
            .and_then(Value::as_bool)
            == Some(true)
        && hour.get("role").and_then(Value::as_str) == Some(snapshot::HOUR_INDEX_ROLE)
        && hour.get("path").and_then(Value::as_str) == Some(snapshot::HOUR_INDEX_FILE)
        && hour.get("bytes").and_then(Value::as_u64) == Some(index_entry.bytes)
        && hour.get("sha256").and_then(Value::as_str) == Some(index_entry.sha256.as_str())
        && hour.get("hours").and_then(Value::as_u64)
            == Some(replay_index.hours().len() as u64)
        && hour.get("source_bytes").and_then(Value::as_u64)
            == Some(replay_index.source_len())
        && hour.get("source_sha256").and_then(Value::as_str)
            == Some(replay_index.source_sha256())
        && checkpoints.get("role").and_then(Value::as_str)
            == Some(snapshot::CHECKPOINT_CACHE_ROLE)
        && checkpoints.get("path").and_then(Value::as_str)
            == Some(snapshot::CHECKPOINT_CACHE_FILE)
        && checkpoints.get("bytes").and_then(Value::as_u64) == Some(cache_entry.bytes)
        && checkpoints.get("sha256").and_then(Value::as_str)
            == Some(cache_entry.sha256.as_str())
        && checkpoints
            .get("checkpoint_interval_hours")
            .and_then(Value::as_u64)
            == Some(replay_index.checkpoint_interval_hours() as u64)
        && checkpoints
            .get("max_seek_deltas")
            .and_then(Value::as_u64)
            .is_some_and(|value| value < replay_index.checkpoint_interval_hours() as u64);
    if !exact {
        return Err(SourceError::Open(
            "replay_decoder index descriptor does not exactly join its certified roles or still carries production JSON snapshots"
                .to_string(),
        ));
    }
    Ok(())
}

fn open_compact_stream_with_spooled_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    mut metadata: MetadataRoute,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let identity_mode = prefold_stream_identity_mode(&metadata);
    let owner = PrefoldSpoolOwner::create(spool_base)?;
    let raw_path = owner.root().join("source.h4cr");
    let snapshot_dir = owner.snapshot_dir().to_path_buf();

    let source_receipt = {
        let container = store.container().ok_or_else(|| {
            SourceError::Unsupported(format!(
                "recording {game_id:?} is a compact-stream-only directory store; the bounded portable decoder seam accepts authenticated .battleplan role entries only"
            ))
        })?;
        let entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
        let entry = match entries.as_slice() {
            [entry] => entry,
            [] => {
                return Err(SourceError::Unsupported(
                    match declared_raw_stream_packaging(metadata.recorder_manifest.as_ref()) {
                        Ok(snapshot::RawStreamPackaging::Omitted) => format!(
                            "recording {game_id:?} {}",
                            snapshot::raw_stream_unavailable_message(
                                "be folded through the bounded raw-stream decoder seam"
                            ),
                        ),
                        _ => format!(
                            "recording {game_id:?} has an empty snapshot index but no manifest entry with role {:?}",
                            snapshot::COMPACT_STREAM_ROLE
                        ),
                    },
                ))
            }
            _ => {
                return Err(SourceError::Open(format!(
                    "recording {game_id:?} has {} manifest entries with role {:?}; expected exactly one",
                    entries.len(),
                    snapshot::COMPACT_STREAM_ROLE
                )))
            }
        };
        validate_index_schema_version(declared_schema_version.as_ref())?;
        validate_compact_stream_entry_identity(&entry.path, &game_id, metadata.timeline.as_ref())?;
        validate_recorder_compact_stream_join(
            &entry.path,
            entry.bytes,
            &entry.sha256,
            &game_id,
            metadata.recorder_manifest.as_ref(),
        )?;
        admit_product_compact_stream_bytes(entry.bytes)?;
        let receipt = container
            .spool_entry_to_file_transactionally_bounded(
                &entry.path,
                &raw_path,
                PRODUCT_COMPACT_STREAM_MAX_BYTES,
            )
            .map_err(|error| {
                let message = format!(
                    "authenticate compact stream role entry {:?} into private spool: {error}",
                    entry.path
                );
                source_error_from_container(&error, message)
            })?;
        if receipt.path() != entry.path.as_str() {
            return Err(SourceError::Open(
                "container spool receipt path does not match selected compact role entry"
                    .to_string(),
            ));
        }
        receipt
    };

    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new_with_identity_mode(
        game_id.clone(),
        raw_path,
        snapshot_dir.clone(),
        source_receipt.uncompressed_len(),
        source_receipt.sha256(),
        identity_mode,
    )
    .map_err(|error| {
        SourceError::Open(format!(
            "authenticated compact stream violates bounded pre-fold request: {error}"
        ))
    })?;
    let certified =
        snapshot::decode_certified_prefolded_spool(decoder, request).map_err(|error| {
            SourceError::Open(format!(
                "bounded portable decoder {:?} rejected authenticated compact stream: {error}",
                decoder.decoder_id()
            ))
        })?;
    validate_index_schema_version(Some(certified.spool().schema_version()))?;
    if let Some(declared) = declared_schema_version.as_ref() {
        if !same_supported_schema_version(declared, certified.spool().schema_version()) {
            return Err(SourceError::Open(format!(
                "compact-only index schema_version {declared} does not match bounded decoder {:?} output schema_version {}",
                decoder.decoder_id(),
                certified.spool().schema_version()
            )));
        }
    }
    if certified.snapshot_dir() != snapshot_dir {
        return Err(SourceError::Open(
            "bounded decoder result changed the caller-owned snapshot spool path".to_string(),
        ));
    }
    let (spool, certified_snapshot_dir, prefold_receipt) = certified.into_parts();
    if prefold_receipt.source_len() != source_receipt.uncompressed_len()
        || prefold_receipt.source_sha256() != source_receipt.sha256()
    {
        return Err(SourceError::Open(
            "bounded decoder receipt is not bound to the authenticated container entry".to_string(),
        ));
    }
    owner.remove_authenticated_source()?;

    let first_entry = spool.entries().first().expect("portable spool is nonempty");
    let first = read_spooled_hour(&owner, &certified_snapshot_dir, &spool, first_entry)?;
    validate_snapshot_container_identity(
        store
            .container()
            .expect("spooled compact route requires container"),
        &first,
    )?;

    let spool_hours = spool
        .entries()
        .iter()
        .map(|entry| entry.hour_index())
        .collect::<Vec<_>>();
    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &spool_hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(SpooledPrefoldedSource {
        spool,
        snapshot_dir: certified_snapshot_dir,
        owner,
        warnings: Vec::new(),
        store,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
    }))
}

fn prefold_stream_identity_mode(metadata: &MetadataRoute) -> snapshot::PrefoldStreamIdentityMode {
    prefold_stream_identity_mode_from_campaign(
        metadata
            .binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref()),
    )
}

fn prefold_stream_identity_mode_from_campaign(
    identity: Option<&RecordedCampaignIdentity>,
) -> snapshot::PrefoldStreamIdentityMode {
    match identity.map(|identity| identity.state.as_str()) {
        Some("known") => snapshot::PrefoldStreamIdentityMode::IdentityV1,
        _ => snapshot::PrefoldStreamIdentityMode::Legacy,
    }
}

fn validate_compact_stream_entry_identity(
    entry_path: &str,
    game_id: &str,
    timeline: Option<&Value>,
) -> Result<(), SourceError> {
    let game_prefix = format!("payload/{game_id}/");
    if !entry_path.starts_with(&game_prefix) {
        return Err(SourceError::Open(format!(
            "compact stream role entry {entry_path:?} is outside manifest game namespace {game_prefix:?}"
        )));
    }

    let Some(timeline) = timeline.and_then(Value::as_object) else {
        return Ok(());
    };
    let format = timeline.get("format").and_then(Value::as_str);
    let role = timeline.get("raw_stream_role").and_then(Value::as_str);
    let declares_compact =
        format == Some(COMPACT_STREAM_TIMELINE_FORMAT) || role == Some(COMPACT_STREAM_ROLE);
    if !declares_compact {
        return Ok(());
    }
    if format != Some(COMPACT_STREAM_TIMELINE_FORMAT) || role != Some(COMPACT_STREAM_ROLE) {
        return Err(SourceError::Open(format!(
            "compact timeline identity is incomplete: format={format:?}, raw_stream_role={role:?}"
        )));
    }
    let relative = timeline
        .get("stream")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            SourceError::Open("compact timeline declares no nonblank stream path".to_string())
        })?;
    if relative.starts_with('/')
        || relative.contains(char::from(92))
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || snapshot::zip::validate_path(relative).is_err()
    {
        return Err(SourceError::Open(format!(
            "compact timeline stream path is not a valid game-relative forward-slash path: {relative:?}"
        )));
    }
    let expected_path = format!("{game_prefix}{relative}");
    if entry_path != expected_path {
        return Err(SourceError::Open(format!(
            "compact stream role entry {entry_path:?} does not byte-match timeline stream {expected_path:?}"
        )));
    }
    Ok(())
}

fn validate_prefolded_container_identity(
    container: &snapshot::Container,
    replay: &snapshot::PrefoldedReplay,
) -> Result<(), SourceError> {
    for entry in replay.entries() {
        validate_snapshot_container_identity(container, entry.snapshot())?;
    }
    Ok(())
}

fn validate_snapshot_container_identity(
    container: &snapshot::Container,
    first: &snapshot::Snapshot,
) -> Result<(), SourceError> {
    let manifest = container.manifest();
    let provenance = first.rest.get("provenance");
    let manifest_modset = manifest
        .get("mod_identity")
        .and_then(|value| value.get("modset_id"))
        .and_then(Value::as_str);
    let snapshot_modset = provenance
        .and_then(|value| value.get("mod"))
        .and_then(|value| value.get("modset_id"))
        .and_then(Value::as_str);
    if let (Some(manifest), Some(snapshot_modset_id)) = (manifest_modset, snapshot_modset) {
        if manifest != snapshot_modset_id {
            let error = snapshot::ContainerError::ModsetMismatch {
                manifest: manifest.to_string(),
                snapshot: snapshot_modset_id.to_string(),
            };
            return Err(SourceError::ModsetMismatch(format!(
                "open .battleplan: {error}"
            )));
        }
    }

    let manifest_sha = manifest
        .get("build")
        .and_then(|value| value.get("exe_sha256"))
        .and_then(Value::as_str);
    for (label, snapshot_sha) in [
        (
            "collector",
            provenance
                .and_then(|value| value.get("collector"))
                .and_then(|value| value.get("exe_sha256"))
                .and_then(Value::as_str),
        ),
        (
            "memory",
            provenance
                .and_then(|value| value.get("memory"))
                .and_then(|value| value.get("exe_sha256"))
                .and_then(Value::as_str),
        ),
    ] {
        if let (Some(manifest), Some(snapshot)) = (manifest_sha, snapshot_sha) {
            if manifest != snapshot {
                return Err(SourceError::Open(format!(
                    "open .battleplan: container rejected: manifest build.exe_sha256 {manifest:?} disagrees with prefolded snapshot provenance.{label}.exe_sha256 {snapshot:?}"
                )));
            }
        }
    }
    Ok(())
}

fn read_portable_design_catalog(
    store: &Store,
    game_id: &str,
    hours: &[i64],
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<Arc<crate::design_catalog::PortableDesignCatalog>>, SourceError> {
    let recorder_row = recorder_manifest
        .and_then(|manifest| manifest.get("replay_packaging"))
        .and_then(|packaging| packaging.get("design_catalog_live"))
        .filter(|value| !value.is_null());

    if recorder_manifest.is_some() {
        let index_bytes = store
            .read_game_file_bytes(game_id, "index.json")
            .map_err(|error| {
                let message = format!(
                    "read authenticated index for A27 design-catalog join: {error}"
                );
                source_error_from_load(&error, message)
            })?
            .ok_or_else(|| {
                SourceError::Open(
                    "selected game index disappeared during A27 design-catalog join".into(),
                )
            })?;
        let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
            SourceError::Open(format!(
                "strict parse selected game index for A27 design-catalog join: {error}"
            ))
        })?;
        let index_row = index
            .get("design_catalog_live")
            .filter(|value| value.get("present").and_then(Value::as_bool) == Some(true));
        match (recorder_row, index_row) {
            (None, None) => {
                if store
                    .container()
                    .is_some_and(|container| {
                        !container.entries_by_role("design_catalog_live").is_empty()
                    })
                {
                    return Err(SourceError::Open(
                        "A27 design-catalog role exists without recorder/index authority".into(),
                    ));
                }
                return Ok(None);
            }
            (None, Some(_)) => {
                return Err(SourceError::Open(
                    "index records A27 design catalog present but recorder manifest omits it"
                        .into(),
                ))
            }
            (Some(_), None) => {
                return Err(SourceError::Open(
                    "recorder manifest records A27 design catalog but index omits it".into(),
                ))
            }
            (Some(recorder_row), Some(index_row)) => {
                let expected_path = crate::design_catalog::PORTABLE_ARTIFACT_PATH;
                let recorder_path = recorder_row.get("path").and_then(Value::as_str);
                let recorder_sha = recorder_row.get("sha256").and_then(Value::as_str);
                let recorder_len = recorder_row.get("bytes").and_then(Value::as_u64);
                let recorder_content_digest = recorder_row
                    .get("content_manifest_digest_sha256")
                    .and_then(Value::as_str);
                if recorder_path != Some(expected_path)
                    || recorder_sha.is_none_or(|value| !lower_sha256(value))
                    || recorder_len.is_none()
                    || recorder_content_digest.is_none_or(|value| !lower_sha256(value))
                {
                    return Err(SourceError::Open(
                        "recorder manifest A27 design-catalog identity is invalid".into(),
                    ));
                }
                let recorder_sha = recorder_sha.expect("validated A27 sha");
                let recorder_len = recorder_len.expect("validated A27 byte length");
                let recorder_content_digest =
                    recorder_content_digest.expect("validated A27 content digest");
                if index_row.get("role").and_then(Value::as_str)
                    != Some("design_catalog_live")
                    || index_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || index_row.get("sha256").and_then(Value::as_str) != Some(recorder_sha)
                    || index_row.get("bytes").and_then(Value::as_u64) != Some(recorder_len)
                    || index_row.get("source_sha256").and_then(Value::as_str)
                        != recorder_row.get("source_sha256").and_then(Value::as_str)
                {
                    return Err(SourceError::Open(
                        "index A27 design-catalog identity disagrees with recorder manifest"
                            .into(),
                    ));
                }

                let container = store.container().ok_or_else(|| {
                    SourceError::Open(
                        "A27 design catalog requires a portable container authority".into(),
                    )
                })?;
                let content_digest = container
                    .content_manifest()
                    .and_then(|manifest| manifest.manifest_digest_sha256.as_deref())
                    .ok_or_else(|| {
                        SourceError::Open(
                            "A27 design catalog has no finalized portable content-manifest epoch"
                                .into(),
                        )
                    })?;
                if content_digest != recorder_content_digest {
                    return Err(SourceError::Open(
                        "A27 design catalog is not bound to the selected content manifest".into(),
                    ));
                }
                let container_path = format!("payload/{game_id}/{expected_path}");
                let entries = container
                    .entries_by_role("design_catalog_live")
                    .into_iter()
                    .filter(|entry| entry.path == container_path)
                    .collect::<Vec<_>>();
                let entry = match entries.as_slice() {
                    [entry] => entry,
                    [] => {
                        return Err(SourceError::Open(
                            "A27 design-catalog index row has no matching dedicated role".into(),
                        ))
                    }
                    _ => {
                        return Err(SourceError::Open(
                            "A27 design catalog has duplicate dedicated role entries".into(),
                        ))
                    }
                };
                if entry.sha256 != recorder_sha || entry.bytes != recorder_len {
                    return Err(SourceError::Open(
                        "A27 design-catalog role entry disagrees with recorder/index hashes"
                            .into(),
                    ));
                }
            }
        }
    }

    if store.container().is_none() && recorder_row.is_none() {
        let present = store
            .read_game_file_bytes(game_id, crate::design_catalog::PORTABLE_ARTIFACT_PATH)
            .map_err(|error| {
                source_error_from_load(
                    &error,
                    format!(
                        "probe A27 design catalog artifact {:?}: {error}",
                        crate::design_catalog::PORTABLE_ARTIFACT_PATH
                    ),
                )
            })?
            .is_some();
        if present {
            return Err(SourceError::Open(format!(
                "A27 design catalog {:?} exists in a directory store with no container entry \
                 digest and no recorder-manifest sha256/length; this build refuses to admit an \
                 unauthenticated semantic catalog",
                crate::design_catalog::PORTABLE_ARTIFACT_PATH
            )));
        }
        return Ok(None);
    }

    let bytes = store
        .read_game_file_bytes(game_id, crate::design_catalog::PORTABLE_ARTIFACT_PATH)
        .map_err(|error| {
            source_error_from_load(
                &error,
                format!(
                    "read verified A27 design catalog artifact {:?}: {error}",
                    crate::design_catalog::PORTABLE_ARTIFACT_PATH
                ),
            )
        })?;
    bytes
        .map(|bytes| {
            if let Some(recorder_row) = recorder_row {
                let expected_sha = recorder_row
                    .get("sha256")
                    .and_then(Value::as_str)
                    .expect("recorder-owned A27 identity validated above");
                let expected_len = recorder_row
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .expect("recorder-owned A27 identity validated above");
                let actual_sha =
                    snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes));
                if bytes.len() as u64 != expected_len || actual_sha != expected_sha {
                    return Err(SourceError::Open(
                        "verified A27 design-catalog bytes disagree with package authorities"
                            .into(),
                    ));
                }
            }
            crate::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes)
                .and_then(|catalog| {
                    catalog.validate_replay_hours(hours)?;
                    Ok(catalog)
                })
                .map(Arc::new)
                .map_err(|error| {
                    SourceError::Open(format!("invalid A27 design catalog artifact: {error}"))
                })
        })
        .transpose()
}

const REQUIRED_PACKAGED_STATIC_ARTIFACTS: &[(&str, &str)] = &[
    (
        "templates_equipment_catalog",
        "static/templates_equipment_catalog/domain59_static_catalog.json",
    ),
    (
        "localisation_join",
        "static/localisation_join/localisation_catalog.json",
    ),
    (
        "decisions_static_definitions",
        "static/decisions_static_definitions/decision_definitions_catalog.json",
    ),
    (
        "idea_tech_loc_xref_static",
        "static/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
    ),
    (
        "ship_definitions_static",
        "static/ship_definitions_static/ship_definitions_catalog.json",
    ),
    (
        "gfx_icon_asset_manifest_static",
        "static/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
    ),
    (
        "technology_sharing_static",
        "static/technology_sharing_static/technology_sharing_static.json",
    ),
    (
        "focus_defs_static",
        "static/focus_defs_static/focus_defs_static.json",
    ),
];
const LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT: usize =
    REQUIRED_PACKAGED_STATIC_ARTIFACTS.len() - 1;

struct PackagedStaticArtifactRoster {
    admitted: BTreeMap<String, PackagedStaticArtifact>,
    not_content_bound: Vec<String>,
    absent: BTreeSet<String>,
    legacy_skipped: Option<&'static str>,
}

enum RecordingContentEpoch {
    None,
    ContainerManifest(String),
    AttachTime(
        Box<Result<portable_content_manifest::attach_content_epoch::AttachContentEpoch, String>>,
    ),
}

impl RecordingContentEpoch {
    fn digest(&self) -> Option<&str> {
        match self {
            RecordingContentEpoch::None => Option::None,
            RecordingContentEpoch::ContainerManifest(digest) => Some(digest.as_str()),
            RecordingContentEpoch::AttachTime(epoch) => match epoch.as_ref() {
                Ok(epoch) => Some(epoch.digest()),
                Err(_) => Option::None,
            },
        }
    }

    fn derivation_error(&self) -> Option<&str> {
        match self {
            RecordingContentEpoch::AttachTime(epoch) => match epoch.as_ref() {
                Err(reason) => Some(reason.as_str()),
                Ok(_) => Option::None,
            },
            _ => Option::None,
        }
    }

    fn authorizes(&self, role: &str, artifact_sha256: &str) -> Result<(), String> {
        match self {
            RecordingContentEpoch::None => {
                Err("this recording states no content epoch".to_string())
            }
            RecordingContentEpoch::ContainerManifest(_) => Ok(()),
            RecordingContentEpoch::AttachTime(epoch) => match epoch.as_ref() {
                Ok(epoch) => epoch.bind(role, artifact_sha256).map(|_| ()),
                Err(reason) => Err(reason.clone()),
            },
        }
    }
}

fn recording_content_epoch(
    container: &snapshot::Container,
    recorder_manifest: &Map<String, Value>,
) -> Result<RecordingContentEpoch, SourceError> {
    use portable_content_manifest::attach_content_epoch as epoch_authority;

    let declared =
        epoch_authority::content_proof_kind(recorder_manifest).map_err(SourceError::Open)?;
    let container_digest = container
        .content_manifest()
        .and_then(|manifest| manifest.manifest_digest_sha256.as_deref());
    Ok(match declared {
        None => match container_digest {
            Some(digest) => RecordingContentEpoch::ContainerManifest(digest.to_string()),
            None => RecordingContentEpoch::None,
        },
        Some(kind) if kind == epoch_authority::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1 => {
            match container_digest {
                Some(digest) => RecordingContentEpoch::ContainerManifest(digest.to_string()),
                None => {
                    return Err(SourceError::Open(
                        "recording declares the launch-time content epoch but its container \
                         finalized no portable content manifest"
                            .to_string(),
                    ))
                }
            }
        }
        Some(kind) if kind == epoch_authority::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1 => {
            let block = recorder_manifest
                .get(epoch_authority::ATTACH_TIME_CONTENT_PROOF_FIELD)
                .filter(|value| !value.is_null())
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder manifest declares the attach-time content epoch but carries no \
                         attach_time_content_proof block"
                            .to_string(),
                    )
                })?;
            RecordingContentEpoch::AttachTime(Box::new(epoch_authority::epoch_from_proof_json(
                block,
            )))
        }
        Some(_) => RecordingContentEpoch::None,
    })
}

fn validate_packaged_static_artifact_roster(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
    claims_production_profile: bool,
) -> Result<PackagedStaticArtifactRoster, SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(PackagedStaticArtifactRoster {
            admitted: BTreeMap::new(),
            not_content_bound: Vec::new(),
            absent: BTreeSet::new(),
            legacy_skipped: None,
        });
    };
    let recorder_rows = recorder_manifest
        .get("replay_packaging")
        .and_then(|packaging| packaging.get("static_artifacts"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open(
                "recorder-owned replay has no static-artifact packaging roster".into(),
            )
        })?;
    {
        let mut seen = BTreeSet::new();
        for (index, row) in recorder_rows.iter().enumerate() {
            let name = row.get("name").and_then(Value::as_str).ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder manifest replay_packaging.static_artifacts[{index}] has no string name"
                ))
            })?;
            if !REQUIRED_PACKAGED_STATIC_ARTIFACTS
                .iter()
                .any(|(required, _)| *required == name)
            {
                return Err(SourceError::Open(format!(
                    "recorder manifest packages static artifact {name:?}, which is not a required \
                     static artifact"
                )));
            }
            if !seen.insert(name) {
                return Err(SourceError::Open(format!(
                    "recorder manifest packages static artifact {name:?} more than once"
                )));
            }
        }
    }
    let index_bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            let message = format!(
                "read authenticated index for static-artifact roster join: {error}"
            );
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| {
            SourceError::Open(
                "selected game index disappeared during static-artifact roster join".into(),
            )
        })?;
    let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse selected game index for static-artifact roster join: {error}"
        ))
    })?;
    let index_rows = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay index has no static-artifact roster".into())
        })?;
    let legacy_without_focus_defs =
        index_rows.len() == LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT;
    if !legacy_without_focus_defs
        && index_rows.len() != REQUIRED_PACKAGED_STATIC_ARTIFACTS.len()
    {
        return Err(SourceError::Open(format!(
            "static-artifact index roster has {} rows; expected current {} or legacy {}",
            index_rows.len(),
            REQUIRED_PACKAGED_STATIC_ARTIFACTS.len(),
            LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT,
        )));
    }
    let container = store.container().ok_or_else(|| {
        SourceError::Open("static-artifact roster requires a portable container authority".into())
    })?;
    let epoch_authority = recording_content_epoch(container, recorder_manifest)?;
    let content_digest = epoch_authority.digest();
    if claims_production_profile && content_digest.is_none() {
        return Err(SourceError::Open(
            "static-artifact roster has no finalized portable content-manifest epoch while \
             recorder production_profile.enabled is true; the production profile is what builds \
             that manifest, so a capture claiming it must carry one"
                .into(),
        ));
    }

    let mut expected_container_paths = BTreeSet::new();
    let mut admitted = BTreeMap::new();
    let mut not_content_bound = Vec::new();
    let mut absent = BTreeSet::new();
    let mut legacy_skipped = None;
    for &(name, expected_path) in REQUIRED_PACKAGED_STATIC_ARTIFACTS {
        let recorder_matches = recorder_rows
            .iter()
            .filter(|row| row.get("name").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>();
        if recorder_matches.len() > 1 {
            return Err(SourceError::Open(format!(
                "recorder manifest declares static artifact {name:?} more than once"
            )));
        }
        let index_matches = index_rows
            .iter()
            .filter(|row| row.get("name").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>();
        if legacy_without_focus_defs
            && name == "focus_defs_static"
            && recorder_matches.is_empty()
            && index_matches.is_empty()
        {
            absent.insert(name.to_string());
            legacy_skipped = Some(name);
            continue;
        }
        if index_matches.len() != 1 {
            return Err(SourceError::Open(format!(
                "index must declare static artifact {name:?} exactly once"
            )));
        }
        let index_row = index_matches[0];
        let present = index_row.get("present").and_then(Value::as_bool);
        match (present, recorder_matches.as_slice()) {
            (Some(false), []) => {
                absent.insert(name.to_string());
                continue;
            }
            (Some(false), [_]) => {
                return Err(SourceError::Open(format!(
                    "recorder manifest records {name:?} present while index records it absent"
                )))
            }
            (Some(true), [recorder_row]) => {
                let sha = recorder_row.get("sha256").and_then(Value::as_str);
                let bytes = recorder_row.get("bytes").and_then(Value::as_u64);
                let epoch = match recorder_row.get("content_manifest_digest_sha256") {
                    Some(Value::Null) => None,
                    Some(Value::String(value)) => Some(value.as_str()),
                    _ => {
                        return Err(SourceError::Open(format!(
                            "recorder manifest static artifact {name:?} \
                             content_manifest_digest_sha256 is absent or is neither a digest \
                             string nor null"
                        )))
                    }
                };
                if let (Some(reason), Some(_)) = (epoch_authority.derivation_error(), epoch) {
                    return Err(SourceError::Open(format!(
                        "recorder manifest static artifact {name:?} claims a content epoch its \
                         own attach-time proof does not yield: {reason}"
                    )));
                }
                let epoch_binds = match content_digest {
                    Some(digest) => epoch == Some(digest),
                    None => epoch.is_none(),
                };
                if recorder_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || sha.is_none_or(|value| !lower_sha256(value))
                    || bytes.is_none()
                    || !epoch_binds
                {
                    return Err(SourceError::Open(format!(
                        "recorder manifest static artifact {name:?} identity is invalid"
                    )));
                }
                let sha = sha.expect("validated static-artifact sha");
                let bytes = bytes.expect("validated static-artifact length");
                if index_row.get("role").and_then(Value::as_str) != Some("static_artifact")
                    || index_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || index_row.get("sha256").and_then(Value::as_str) != Some(sha)
                    || index_row.get("bytes").and_then(Value::as_u64) != Some(bytes)
                {
                    return Err(SourceError::Open(format!(
                        "index static artifact {name:?} disagrees with recorder manifest"
                    )));
                }
                let container_path = format!("payload/{game_id}/{expected_path}");
                let entries = container
                    .entries_by_role("static_artifact")
                    .into_iter()
                    .filter(|entry| entry.path == container_path)
                    .collect::<Vec<_>>();
                let entry = match entries.as_slice() {
                    [entry] => entry,
                    [] => {
                        return Err(SourceError::Open(format!(
                            "static artifact {name:?} has no matching container role"
                        )))
                    }
                    _ => {
                        return Err(SourceError::Open(format!(
                            "static artifact {name:?} has duplicate container roles"
                        )))
                    }
                };
                if entry.sha256 != sha || entry.bytes != bytes {
                    return Err(SourceError::Open(format!(
                        "container static artifact {name:?} disagrees with recorder/index hashes"
                    )));
                }
                let payload = store
                    .read_game_file_bytes(game_id, expected_path)
                    .map_err(|error| {
                        let message = format!(
                            "read verified static artifact {name:?}: {error}"
                        );
                        source_error_from_load(&error, message)
                    })?
                    .ok_or_else(|| {
                        SourceError::Open(format!(
                            "verified static artifact {name:?} payload is absent"
                        ))
                    })?;
                let actual_sha =
                    snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&payload));
                if payload.len() as u64 != bytes || actual_sha != sha {
                    return Err(SourceError::Open(format!(
                        "verified static artifact {name:?} bytes disagree with package authorities"
                    )));
                }
                if epoch_authority.authorizes(name, &actual_sha).is_err() {
                    not_content_bound.push(name.to_string());
                    expected_container_paths.insert(container_path);
                    continue;
                }
                match content_digest {
                    Some(digest) => {
                        admitted.insert(
                            name.to_string(),
                            PackagedStaticArtifact {
                                name: name.to_string(),
                                path: expected_path.to_string(),
                                sha256: actual_sha,
                                content_manifest_digest_sha256: digest.to_string(),
                                bytes: Arc::from(payload),
                            },
                        );
                    }
                    None => not_content_bound.push(name.to_string()),
                }
                expected_container_paths.insert(container_path);
            }
            (Some(true), []) => {
                return Err(SourceError::Open(format!(
                    "index records static artifact {name:?} present but recorder manifest omits it"
                )))
            }
            _ => {
                return Err(SourceError::Open(format!(
                    "index static artifact {name:?} has no boolean present disposition"
                )))
            }
        }
    }
    let actual_container_paths = container
        .entries_by_role("static_artifact")
        .into_iter()
        .map(|entry| entry.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if actual_container_paths != expected_container_paths {
        return Err(SourceError::Open(
            "container static-artifact roles do not exactly match the recorder/index roster"
                .into(),
        ));
    }
    Ok(PackagedStaticArtifactRoster {
        admitted,
        not_content_bound,
        absent,
        legacy_skipped,
    })
}

const DECISION_CATALOG_NAME: &str = "decisions_static_definitions";
const DECISION_CATALOG_PATH: &str =
    "static/decisions_static_definitions/decision_definitions_catalog.json";

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_packaged_decision_catalog(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<PackagedDecisionCatalog>, SourceError> {
    let recorder_rows = recorder_manifest
        .and_then(|manifest| manifest.get("replay_packaging"))
        .and_then(|packaging| packaging.get("static_artifacts"))
        .and_then(Value::as_array);
    let recorder_matches: Vec<&Value> = recorder_rows
        .into_iter()
        .flatten()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(DECISION_CATALOG_NAME))
        .collect();
    if recorder_matches.len() > 1 {
        return Err(SourceError::Open(
            "recorder manifest declares the A34 decision catalog more than once".into(),
        ));
    }

    let index_bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            let message = format!(
                "read authenticated index for A34 decision catalog join: {error}"
            );
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| {
            SourceError::Open(
                "selected game index disappeared during A34 decision catalog join".into(),
            )
        })?;
    let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse selected game index for A34 decision catalog join: {error}"
        ))
    })?;
    let index_matches: Vec<&Value> = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(DECISION_CATALOG_NAME))
        .collect();
    if index_matches.len() > 1 {
        return Err(SourceError::Open(
            "selected game index declares the A34 decision catalog more than once".into(),
        ));
    }
    let Some(index_row) = index_matches.first().copied() else {
        if recorder_matches.is_empty() {
            return Ok(None);
        }
        return Err(SourceError::Open(
            "recorder manifest declares A34 decision catalog but index roster omits it".into(),
        ));
    };
    if index_row.get("present").and_then(Value::as_bool) != Some(true) {
        if recorder_matches.is_empty() {
            return Ok(None);
        }
        return Err(SourceError::Open(
            "recorder manifest declares A34 decision catalog but index records it absent".into(),
        ));
    }
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(None);
    };
    let recorder_row = recorder_matches.first().copied().ok_or_else(|| {
        SourceError::Open(
            "index records A34 decision catalog present but recorder manifest omits it".into(),
        )
    })?;

    let recorder_path = recorder_row.get("path").and_then(Value::as_str);
    let recorder_sha = recorder_row.get("sha256").and_then(Value::as_str);
    let recorder_len = recorder_row.get("bytes").and_then(Value::as_u64);
    let recorder_content_digest = recorder_row
        .get("content_manifest_digest_sha256")
        .and_then(Value::as_str);
    // consulting a local generated catalog.
    let Some(recorder_content_digest) = recorder_content_digest else {
        return Ok(None);
    };
    if recorder_path != Some(DECISION_CATALOG_PATH)
        || recorder_sha.is_none_or(|value| !lower_sha256(value))
        || recorder_len.is_none()
        || !lower_sha256(recorder_content_digest)
    {
        return Err(SourceError::Open(
            "recorder manifest A34 decision catalog identity is invalid".into(),
        ));
    }
    let recorder_sha = recorder_sha.expect("validated recorder A34 sha");
    let recorder_len = recorder_len.expect("validated recorder A34 length");

    let index_path = index_row.get("path").and_then(Value::as_str);
    let index_sha = index_row.get("sha256").and_then(Value::as_str);
    let index_len = index_row.get("bytes").and_then(Value::as_u64);
    if index_row.get("role").and_then(Value::as_str) != Some("static_artifact")
        || index_path != Some(DECISION_CATALOG_PATH)
        || index_sha != Some(recorder_sha)
        || index_len != Some(recorder_len)
    {
        return Err(SourceError::Open(
            "index A34 decision catalog identity disagrees with recorder manifest".into(),
        ));
    }

    let container = store.container().ok_or_else(|| {
        SourceError::Open(
            "A34 decision catalog requires a portable container content-manifest authority"
                .into(),
        )
    })?;
    let epoch_authority = recording_content_epoch(container, recorder_manifest)?;
    let Some(content_digest) = epoch_authority.digest() else {
        return Ok(None);
    };
    if content_digest != recorder_content_digest {
        return Err(SourceError::Open(
            "A34 decision catalog recorder row is not bound to the selected content manifest"
                .into(),
        ));
    }
    if epoch_authority
        .authorizes(DECISION_CATALOG_NAME, recorder_sha)
        .is_err()
    {
        return Ok(None);
    }

    let container_path = format!("payload/{game_id}/{DECISION_CATALOG_PATH}");
    let entries: Vec<_> = container
        .entries_by_role("static_artifact")
        .into_iter()
        .filter(|entry| entry.path == container_path)
        .collect();
    let entry = match entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(SourceError::Open(
                "A34 decision catalog index row has no matching container-manifest entry".into(),
            ))
        }
        _ => {
            return Err(SourceError::Open(
                "A34 decision catalog has duplicate container-manifest entries".into(),
            ))
        }
    };
    if entry.sha256 != recorder_sha || entry.bytes != recorder_len {
        return Err(SourceError::Open(
            "A34 decision catalog container entry disagrees with recorder/index hashes".into(),
        ));
    }

    let bytes = store
        .read_game_file_bytes(game_id, DECISION_CATALOG_PATH)
        .map_err(|error| {
            let message = format!(
                "read verified A34 decision catalog payload: {error}"
            );
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| SourceError::Open("verified A34 decision catalog payload is absent".into()))?;
    let actual_sha =
        snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(bytes.as_slice()));
    if bytes.len() as u64 != recorder_len || actual_sha != recorder_sha {
        return Err(SourceError::Open(
            "verified A34 decision catalog bytes disagree with package authorities".into(),
        ));
    }

    Ok(Some(PackagedDecisionCatalog {
        path: DECISION_CATALOG_PATH.into(),
        sha256: actual_sha,
        content_manifest_digest_sha256: content_digest.into(),
        bytes: Arc::from(bytes),
    }))
}

fn validate_index_schema_version(version: Option<&Value>) -> Result<(), SourceError> {
    match version {
        None => Ok(()),
        Some(Value::String(value)) if value == "1" => Ok(()),
        Some(Value::Number(value)) if value.as_u64() == Some(1) => Ok(()),
        Some(value) => Err(SourceError::Unsupported(format!(
            "incompatible replay index schema_version {value}; this standalone replayer supports schema_version \"1\""
        ))),
    }
}

fn same_supported_schema_version(left: &Value, right: &Value) -> bool {
    fn is_v1(value: &Value) -> bool {
        matches!(value, Value::String(text) if text == "1")
            || matches!(value, Value::Number(number) if number.as_u64() == Some(1))
    }
    is_v1(left) && is_v1(right)
}

fn validate_recorder_content_manifest_join(
    recorder_session_manifest: &Map<String, Value>,
    container_manifest: Option<&portable_content_manifest::ContentManifest>,
) -> Result<(), SourceError> {
    let recorder_value = recorder_session_manifest.get("portable_content_manifest");
    match (recorder_value, container_manifest) {
        (None | Some(Value::Null), None) => Ok(()),
        (Some(Value::Object(_)), Some(container_manifest)) => {
            let recorder_manifest: portable_content_manifest::ContentManifest =
                serde_json::from_value(
                    recorder_value
                        .expect("object arm has a recorder content manifest")
                        .clone(),
                )
                .map_err(|error| {
                    SourceError::Open(format!(
                        "recorder portable_content_manifest is malformed: {error}"
                    ))
                })?;
            recorder_manifest.validate().map_err(|error| {
                SourceError::Open(format!(
                    "recorder portable_content_manifest failed validation: {error}"
                ))
            })?;
            if &recorder_manifest != container_manifest {
                return Err(SourceError::Open(
                    "recorder portable_content_manifest disagrees with container content_manifest"
                        .into(),
                ));
            }
            let modset = recorder_session_manifest
                .get("modset")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound modset authority".into(),
                    )
                })?;
            let recorder_executable = modset
                .get("game")
                .and_then(Value::as_object)
                .and_then(|game| game.get("exe_sha256"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound executable SHA-256".into(),
                    )
                })?;
            if recorder_executable != container_manifest.source_game.executable_sha256 {
                return Err(SourceError::Open(
                    "recorder modset executable disagrees with portable content source game"
                        .into(),
                ));
            }
            let settings_binding = modset.get("settings_binding").ok_or_else(|| {
                SourceError::Open(
                    "recorder portable content has no modset settings binding".into(),
                )
            })?;
            let recorder_language = settings_binding
                .get("language")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound localisation language"
                            .into(),
                    )
                })?;
            if recorder_language != container_manifest.localisation_language {
                return Err(SourceError::Open(
                    "recorder modset language disagrees with portable content language".into(),
                ));
            }
            let display: RecorderDisplayVariant = serde_json::from_value(
                recorder_display_variant_value(settings_binding).ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound display variant".into(),
                    )
                })?,
            )
            .map_err(|error| {
                SourceError::Open(format!(
                    "recorder portable-content display variant is malformed: {error}"
                ))
            })?;
            let expected_settings = BTreeMap::from([
                ("physical_width".into(), display.physical_width.to_string()),
                ("physical_height".into(), display.physical_height.to_string()),
                ("min_gui_width".into(), display.min_gui_width.to_string()),
                ("min_gui_height".into(), display.min_gui_height.to_string()),
                (
                    "gui_scale_f32_bits".into(),
                    display.gui_scale_f32_bits.to_string(),
                ),
                (
                    "refresh_rate_hz".into(),
                    display.refresh_rate_hz.to_string(),
                ),
                (
                    "max_refresh_rate_hz".into(),
                    display
                        .max_refresh_rate_hz
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".into()),
                ),
                ("fullscreen".into(), display.fullscreen.to_string()),
                ("borderless".into(), display.borderless.to_string()),
                ("display_mode".into(), display.display_mode),
                (
                    "settings_file_sha256".into(),
                    display.settings_file_sha256,
                ),
                (
                    "settings_authority".into(),
                    settings_binding
                        .get("authority")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            SourceError::Open(
                                "recorder portable content states no settings authority".into(),
                            )
                        })?
                        .to_string(),
                ),
            ]);
            if container_manifest.recorder_local_settings != expected_settings {
                return Err(SourceError::Open(
                    "recorder display variant disagrees with the portable content manifest's                      recorder-local settings"
                        .into(),
                ));
            }
            Ok(())
        }
        (None | Some(Value::Null), Some(_)) => Err(SourceError::Open(
            "container declares portable content but recorder manifest omits its matching authority"
                .into(),
        )),
        (Some(value), Some(_)) => Err(SourceError::Open(format!(
            "recorder portable_content_manifest must be an object when the container declares one; found {value}"
        ))),
        (Some(_), None) => Err(SourceError::Open(
            "recorder declares portable content but container manifest omits it".into(),
        )),
    }
}

fn validate_recorder_portable_container_contract(
    store: &Store,
    game_id: &str,
    index: &snapshot::Index,
    metadata: &MetadataRoute,
) -> Result<(), SourceError> {
    let Some(recorder_manifest) = metadata.recorder_manifest.as_ref() else {
        return Ok(());
    };
    let container = store.container().ok_or_else(|| {
        SourceError::Open(
            "recorder-owned replay metadata requires a portable container authority".into(),
        )
    })?;
    validate_recorder_content_manifest_join(recorder_manifest, container.content_manifest())?;
    let recorded_foundation = decode_recorded_foundation_content(
        recorder_manifest.get(FOUNDATION_CONTENT_FIELD),
        "recorder session manifest",
    )?;
    join_recorded_foundation_content(
        recorded_foundation.as_ref(),
        container.content_manifest(),
    )?;
    if let Some(declared_game_id) = recorder_manifest.get("replay_game_id") {
        if declared_game_id.as_str() != Some(game_id) {
            return Err(SourceError::Open(format!(
                "recorder replay_game_id {declared_game_id} disagrees with selected container game_id {game_id:?}"
            )));
        }
    }

    let manifest_schema = container
        .manifest()
        .get("schema_version")
        .ok_or_else(|| {
            SourceError::Open(
                "recorder-owned container manifest has no schema_version authority".into(),
            )
        })?;
    let index_schema = index.schema_version.as_ref().ok_or_else(|| {
        SourceError::Open("recorder-owned replay index has no schema_version authority".into())
    })?;
    if !same_supported_schema_version(manifest_schema, index_schema) {
        return Err(SourceError::Open(format!(
            "recorder-owned container manifest schema_version {manifest_schema} disagrees with index schema_version {index_schema}"
        )));
    }

    let canonical_index = format!("payload/{game_id}/index.json");
    let index_roles = container.entries_by_role("index");
    if index_roles.len() != 1 || index_roles[0].path != canonical_index {
        return Err(SourceError::Open(format!(
            "recorder-owned replay requires exactly one canonical index role {canonical_index:?}; found {:?}",
            index_roles
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>()
        )));
    }
    let index_bytes = container
        .read_entry_bytes(&canonical_index)
        .map_err(|error| {
            let message = format!(
                "read canonical index for recorder fold-disposition join: {error}"
            );
            source_error_from_container(&error, message)
        })?;
    let raw_index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse canonical index for recorder fold-disposition join: {error}"
        ))
    })?;
    let binary_replay_fold = raw_index
        .get("replay_decoder")
        .is_some_and(Value::is_object)
        && raw_index
            .get("production_json_snapshots_omitted")
            .and_then(Value::as_bool)
            == Some(true);
    if let Some(disposition) = raw_index.get("host_fold_skipped") {
        let skipped = disposition.as_bool().ok_or_else(|| {
            SourceError::Open(
                "recorder-owned index host_fold_skipped must be a boolean".into(),
            )
        })?;
        let expected_skipped = index.snapshots.is_empty() && !binary_replay_fold;
        if skipped != expected_skipped {
            return Err(SourceError::Open(format!(
                "recorder-owned index host_fold_skipped={skipped} contradicts snapshot count {} and binary replay fold={binary_replay_fold}",
                index.snapshots.len(),
            )));
        }
    }

    let snapshot_roles = container.entries_by_role("snapshot");
    if snapshot_roles.len() != index.snapshots.len() {
        return Err(SourceError::Open(format!(
            "recorder-owned replay index lists {} snapshots but container declares {} snapshot roles",
            index.snapshots.len(),
            snapshot_roles.len()
        )));
    }
    for indexed in &index.snapshots {
        let expected_path = format!("payload/{game_id}/{}", indexed.file);
        let matches = snapshot_roles
            .iter()
            .filter(|entry| entry.path == expected_path)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(SourceError::Open(format!(
                "indexed snapshot {:?} requires exactly one matching snapshot role; found {}",
                indexed.file,
                matches.len()
            )));
        }
        let snapshot_bytes = container
            .read_entry_bytes(&matches[0].path)
            .map_err(|error| {
                let message = format!(
                    "read indexed snapshot {:?} for hour binding: {error}",
                    matches[0].path
                );
                source_error_from_container(&error, message)
            })?;
        let snapshot_value = snapshot::parse_json_strict(&snapshot_bytes).map_err(|error| {
            SourceError::Open(format!(
                "strict parse indexed snapshot {:?} for hour binding: {error}",
                matches[0].path
            ))
        })?;
        let body_hour = snapshot_value.get("hour_index").and_then(Value::as_i64);
        if body_hour != indexed.hour_index {
            return Err(SourceError::Open(format!(
                "snapshot role {:?} hour_index {:?} disagrees with index hour_index {:?}",
                matches[0].path, body_hour, indexed.hour_index
            )));
        }
    }

    let timeline = metadata
        .timeline
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay has no timeline object".into())
        })?;
    for (field, expected) in [
        ("format", COMPACT_STREAM_TIMELINE_FORMAT),
        ("stream_format", "compact"),
        ("raw_stream_role", COMPACT_STREAM_ROLE),
        ("session_manifest_role", "recorder_manifest"),
    ] {
        if timeline.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(SourceError::Open(format!(
                "recorder-owned timeline {field} must be {expected:?}"
            )));
        }
    }
    let manifest_packaging = declared_raw_stream_packaging(Some(recorder_manifest))?;
    let timeline_packaging = snapshot::read_raw_stream_packaging_with_legacy_default(timeline)
        .map_err(|error| {
            SourceError::Open(format!("recorder-owned timeline composition: {error}"))
        })?;
    if manifest_packaging != timeline_packaging {
        return Err(SourceError::Open(format!(
            "recorder-owned timeline declares the {:?} composition but the recorder manifest \
             declares {:?}",
            timeline_packaging.as_str(),
            manifest_packaging.as_str(),
        )));
    }
    let index_packaging = raw_index
        .as_object()
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay index is not an object".to_string())
        })
        .and_then(|object| {
            snapshot::read_raw_stream_packaging_with_legacy_default(object).map_err(|error| {
                SourceError::Open(format!("recorder-owned index descriptor composition: {error}"))
            })
        })?;
    if index_packaging != manifest_packaging {
        return Err(SourceError::Open(format!(
            "recorder-owned index descriptor declares the {:?} composition but the recorder \
             manifest declares {:?}",
            index_packaging.as_str(),
            manifest_packaging.as_str(),
        )));
    }
    let stream_entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
    match (manifest_packaging, stream_entries.len()) {
        (snapshot::RawStreamPackaging::Packaged, 1) => {
            let entry = &stream_entries[0];
            validate_compact_stream_entry_identity(
                &entry.path,
                game_id,
                metadata.timeline.as_ref(),
            )?;
            validate_recorder_compact_stream_join(
                &entry.path,
                entry.bytes,
                &entry.sha256,
                game_id,
                Some(recorder_manifest),
            )?;
        }
        (snapshot::RawStreamPackaging::Omitted, 0) => {
            validate_no_stream_path_is_claimed(
                Some(recorder_manifest),
                metadata.timeline.as_ref(),
            )?;
        }
        (packaging, count) => {
            return Err(SourceError::Open(format!(
                "recorder-owned recording declares the {:?} composition but the container carries \
                 {count} entries with role {:?}",
                packaging.as_str(),
                snapshot::COMPACT_STREAM_ROLE,
            )))
        }
    }
    Ok(())
}

fn validate_recorder_compact_stream_join(
    entry_path: &str,
    entry_bytes: u64,
    entry_sha256: &str,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<(), SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(());
    };
    let relative_path = entry_path
        .strip_prefix(&format!("payload/{game_id}/"))
        .ok_or_else(|| {
            SourceError::Open(format!(
                "compact stream role entry {entry_path:?} is outside selected game {game_id:?}"
            ))
        })?;

    if let Some(storage_value) = recorder_manifest.get("stream_storage") {
        let storage = storage_value.as_object().ok_or_else(|| {
            SourceError::Open("recorder stream_storage authority is not an object".into())
        })?;
        if storage.get("decoded_bytes").and_then(Value::as_u64) != Some(entry_bytes)
            || storage.get("decoded_sha256").and_then(Value::as_str) != Some(entry_sha256)
        {
            return Err(SourceError::Open(
                "recorder stream_storage decoded identity disagrees with selected compact stream role"
                    .into(),
            ));
        }
    }

    if let Some(entries_value) = recorder_manifest.get("replay_entries") {
        let entries = entries_value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_entries authority is not an object".into())
        })?;
        if entries.get("stream").and_then(Value::as_str) != Some(relative_path)
            || entries.get("stream_role").and_then(Value::as_str)
                != Some(snapshot::COMPACT_STREAM_ROLE)
        {
            return Err(SourceError::Open(
                "recorder replay_entries stream binding disagrees with selected compact stream role"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn recorder_metadata_route(store: &Store, game_id: &str) -> Result<MetadataRoute, SourceError> {
    if let Some(container) = store.container() {
        let timeline_path = format!("payload/{game_id}/timeline.json");
        let timeline_roles = container.entries_by_role("timeline");
        if timeline_roles.len() > 1 {
            return Err(SourceError::Open(format!(
                "timeline role reconciliation found multiple entries: {:?}",
                timeline_roles
                    .iter()
                    .map(|entry| entry.path.as_str())
                    .collect::<Vec<_>>()
            )));
        }
        if let Some(entry) = timeline_roles.first() {
            if entry.path != timeline_path {
                return Err(SourceError::Open(format!(
                    "timeline role entry {:?} does not byte-match canonical {timeline_path:?}",
                    entry.path
                )));
            }
        }

        let recorder_roles = container.entries_by_role("recorder_manifest");
        let timeline_listed = container
            .entries()
            .iter()
            .any(|entry| entry.path == timeline_path);
        if !timeline_listed {
            if recorder_roles.is_empty() {
                return Ok(MetadataRoute {
                    binding: None,
                    timeline: None,
                    recorder_manifest: None,
                            });
            }
            return Err(SourceError::Open(format!(
                "recorder_manifest role entry exists but canonical timeline {timeline_path:?} is absent"
            )));
        }

        let timeline_bytes = container.read_entry_bytes(&timeline_path).map_err(|error| {
            {
                let message = format!(
                "listed canonical timeline {timeline_path:?} could not be read and verified: {error}"
            );
                source_error_from_container(&error, message)
            }
        })?;
        let timeline = parse_metadata_object(&timeline_bytes, "timeline.json")?;
        let reference = session_manifest_reference(&timeline)?;
        let Some(reference) = reference else {
            if recorder_roles.is_empty() {
                return Ok(MetadataRoute {
                    binding: None,
                    timeline: Some(Value::Object(timeline)),
                    recorder_manifest: None,
                            });
            }
            return Err(SourceError::Open(
                "timeline.json.session_manifest is absent but recorder_manifest role entry exists"
                    .into(),
            ));
        };

        if recorder_roles.len() != 1 {
            return Err(SourceError::Open(format!(
                "timeline.json.session_manifest {reference:?} requires exactly one recorder_manifest role entry; found {}",
                recorder_roles.len()
            )));
        }
        let expected_path = format!("payload/{game_id}/{reference}");
        let entry = &recorder_roles[0];
        if entry.path != expected_path {
            return Err(SourceError::Open(format!(
                "timeline.json.session_manifest {reference:?} expects recorder_manifest path {expected_path:?}, found {:?}",
                entry.path
            )));
        }
        let manifest_bytes = container.read_entry_bytes(&entry.path).map_err(|error| {
            {
                let message = format!(
                "read recorder_manifest {:?} referenced by timeline.json.session_manifest: {error}",
                entry.path
            );
                source_error_from_container(&error, message)
            }
        })?;
        let recorder_manifest =
            parse_metadata_object(&manifest_bytes, &format!("recorder manifest {reference:?}"))?;
        let product_admission = validate_recorder_product_admission(&recorder_manifest)?;
        let binding =
            recorder_manifest_binding_value(&recorder_manifest, &reference, product_admission)?;
        return Ok(MetadataRoute {
            binding: Some(binding),
            timeline: Some(Value::Object(timeline)),
            recorder_manifest: Some(recorder_manifest),
            });
    }

    directory_recorder_metadata_route_with_reader(|relative| {
        store
            .read_game_file_bytes(game_id, relative)
            .map_err(|error| {
                let message = format!(
                    "handle-relative read of {game_id:?}/{relative}: {error}"
                );
                source_error_from_load(&error, message)
            })
    })
}

fn directory_recorder_metadata_route_with_reader<F>(
    mut read_once: F,
) -> Result<MetadataRoute, SourceError>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>, SourceError>,
{
    let timeline =
        read_optional_metadata_object_once(|| read_once("timeline.json"), "timeline.json")?;
    let Some(timeline) = timeline else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: None,
            recorder_manifest: None,
            });
    };
    let Some(reference) = session_manifest_reference(&timeline)? else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: Some(Value::Object(timeline)),
            recorder_manifest: None,
            });
    };
    let manifest = read_optional_metadata_object_once(
        || read_once(&reference),
        &format!("recorder manifest {reference:?}"),
    )?
    .ok_or_else(|| {
        SourceError::Open(format!(
            "timeline.json.session_manifest references missing recorder manifest {reference:?}"
        ))
    })?;
    let product_admission = validate_recorder_product_admission(&manifest)?;
    let binding = recorder_manifest_binding_value(&manifest, &reference, product_admission)?;
    Ok(MetadataRoute {
        binding: Some(binding),
        timeline: Some(Value::Object(timeline)),
        recorder_manifest: Some(manifest),
    })
}

fn parse_metadata_object(bytes: &[u8], context: &str) -> Result<Map<String, Value>, SourceError> {
    let value = snapshot::parse_json_strict(bytes)
        .map_err(|error| SourceError::Open(format!("strict parse {context}: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| SourceError::Open(format!("{context} must decode as a JSON object")))
}

fn validate_recorder_product_admission(
    manifest: &Map<String, Value>,
) -> Result<RecorderProductAdmission, SourceError> {
    if let Some(value) = manifest.get("replay_artifact") {
        let artifact = value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_artifact authority is not an object".into())
        })?;
        if artifact.get("recovery").and_then(Value::as_bool) != Some(false)
            || artifact.get("reason").and_then(Value::as_str) != Some("gate_passed")
            || !artifact
                .get("gate_problems")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        {
            return Err(SourceError::Open(
                "recorder replay_artifact is recovery-only or did not positively pass its gate"
                    .into(),
            ));
        }
    }
    let Some(value) = manifest.get("production_profile") else {
        return Ok(RecorderProductAdmission::unstated());
    };
    let profile = value.as_object().ok_or_else(|| {
        SourceError::Open("recorder production_profile authority is not an object".into())
    })?;
    let enabled = profile
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            SourceError::Open(
                "recorder production_profile.enabled is absent or is not a boolean".into(),
            )
        })?;
    let read_list = |field: &str| -> Result<Vec<String>, SourceError> {
        let rows = profile.get(field).and_then(Value::as_array).ok_or_else(|| {
            SourceError::Open(format!(
                "recorder production_profile.{field} is absent or is not an array"
            ))
        })?;
        rows.iter()
            .enumerate()
            .map(|(index, row)| {
                row.as_str().map(str::to_string).ok_or_else(|| {
                    SourceError::Open(format!(
                        "recorder production_profile.{field}[{index}] is not a string"
                    ))
                })
            })
            .collect()
    };
    let mandatory_domains_not_production_complete =
        read_list("mandatory_domains_not_production_complete")?;
    let profile_domains_not_scheduled = read_list("profile_domains_not_scheduled")?;
    let missing_required_static_artifacts = read_list("missing_required_static_artifacts")?;

    // `state_ownership` resolves to the same domain as the profile's
    let unique_names = |rows: &Vec<String>, field: &str| -> Result<BTreeSet<String>, SourceError> {
        let set = rows.iter().cloned().collect::<BTreeSet<_>>();
        if set.len() != rows.len() {
            return Err(SourceError::Open(format!(
                "recorder {field} repeats a domain name ({rows:?})"
            )));
        }
        Ok(set)
    };
    let profile_domains = unique_names(
        &{
            let rows = profile.get("profile_domains").and_then(Value::as_array).ok_or_else(|| {
                SourceError::Open(
                    "recorder production_profile.profile_domains is absent or is not an array"
                        .into(),
                )
            })?;
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    row.as_str().map(str::to_string).ok_or_else(|| {
                        SourceError::Open(format!(
                            "recorder production_profile.profile_domains[{index}] is not a string"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        },
        "production_profile.profile_domains",
    )?;
    let scheduled_domains = unique_names(
        &{
            let rows = manifest
                .get("scheduled_domains")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder manifest scheduled_domains is absent or is not an array while a \
                         production_profile authority is present"
                            .into(),
                    )
                })?;
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    row.as_str().map(str::to_string).ok_or_else(|| {
                        SourceError::Open(format!(
                            "recorder manifest scheduled_domains[{index}] is not a string"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        },
        "manifest scheduled_domains",
    )?;
    let declared_not_scheduled = unique_names(
        &profile_domains_not_scheduled,
        "production_profile.profile_domains_not_scheduled",
    )?;
    if let Some(unknown) = declared_not_scheduled
        .iter()
        .find(|name| !profile_domains.contains(*name))
    {
        return Err(SourceError::Open(format!(
            "recorder production_profile.profile_domains_not_scheduled names {unknown:?}, which is \
             not in production_profile.profile_domains"
        )));
    }
    if let Some(contradicted) = declared_not_scheduled
        .intersection(&scheduled_domains)
        .next()
    {
        return Err(SourceError::Open(format!(
            "recorder production_profile.profile_domains_not_scheduled names {contradicted:?}, \
             which the manifest's own scheduled_domains carries"
        )));
    }

    if !mandatory_domains_not_production_complete.is_empty() {
        return Err(SourceError::Open(format!(
            "recorder production_profile.mandatory_domains_not_production_complete is nonempty \
             ({mandatory_domains_not_production_complete:?}); those domains' producers are not \
             production-complete in the recording's own recorder build, which no capture mode \
             explains and which the late-attach admission does not cover"
        )));
    }

    if enabled {
        for (field, rows) in [
            (
                "profile_domains_not_scheduled",
                &profile_domains_not_scheduled,
            ),
            (
                "missing_required_static_artifacts",
                &missing_required_static_artifacts,
            ),
        ] {
            if !rows.is_empty() {
                return Err(SourceError::Open(format!(
                    "recorder production_profile.{field} is nonempty ({rows:?}) while \
                     production_profile.enabled is true; a capture that claims the production \
                     profile must satisfy all of it"
                )));
            }
        }
        return Ok(RecorderProductAdmission {
            authority: ProductionProfileAuthority::Enabled,
            guarantees_not_held: Vec::new(),
            profile_domains_not_scheduled,
            missing_required_static_artifacts,
            static_artifacts_not_content_bound: Vec::new(),
        });
    }

    let mut guarantees_not_held = vec![ProductionProfileGuarantee::ProductionProfileEnabled];
    if !profile_domains_not_scheduled.is_empty() {
        guarantees_not_held.push(ProductionProfileGuarantee::FullProfileDomainSchedule);
    }
    guarantees_not_held.sort_unstable();
    guarantees_not_held.dedup();
    Ok(RecorderProductAdmission {
        authority: ProductionProfileAuthority::Disabled,
        guarantees_not_held,
        profile_domains_not_scheduled,
        missing_required_static_artifacts,
        static_artifacts_not_content_bound: Vec::new(),
    })
}

fn read_optional_metadata_object_once<F>(
    read_once: F,
    context: &str,
) -> Result<Option<Map<String, Value>>, SourceError>
where
    F: FnOnce() -> Result<Option<Vec<u8>>, SourceError>,
{
    read_once()?
        .map(|bytes| parse_metadata_object(&bytes, context))
        .transpose()
}

fn session_manifest_reference(
    timeline: &Map<String, Value>,
) -> Result<Option<String>, SourceError> {
    let Some(value) = timeline.get("session_manifest") else {
        return Ok(None);
    };
    let reference = value.as_str().ok_or_else(|| {
        SourceError::Open("timeline.json.session_manifest is present but is not a string".into())
    })?;
    if reference.trim().is_empty()
        || reference.starts_with('/')
        || reference.contains(char::from(92))
        || reference
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || snapshot::zip::validate_path(reference).is_err()
    {
        return Err(SourceError::Open(format!(
            "timeline.json.session_manifest is not a valid game-relative forward-slash path: {reference:?}"
        )));
    }
    Ok(Some(reference.to_string()))
}

fn recorder_manifest_binding_value(
    root: &Map<String, Value>,
    reference: &str,
    product_admission: RecorderProductAdmission,
) -> Result<RecorderSessionBinding, SourceError> {
    let campaign_identity = recorder_campaign_identity(&root, reference)?;
    let modset = root
        .get("modset")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset is missing or not an object"
            ))
        })?;

    match modset.get("resolved") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            let attributed = modset
                .get("errors")
                .and_then(Value::as_array)
                .map(|errors| {
                    errors
                        .iter()
                        .map(|error| {
                            error
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| format!("<non-string {error}>"))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["<missing or malformed errors>".into()]);
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} modset is unresolved; attributed errors: {attributed:?}"
            )));
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} modset.resolved is missing or not boolean"
            )))
        }
    }

    match modset.get("errors") {
        Some(Value::Array(errors)) if errors.is_empty() => {}
        Some(Value::Array(errors)) => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} resolved modset must have errors == []; found {errors:?}"
            )))
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} resolved modset errors is missing or not an array"
            )))
        }
    }

    let dlc = modset
        .get("dlc")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc is missing or not an object"
            ))
        })?;
    let enabled = dlc
        .get("enabled")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc.enabled is missing or not an array"
            ))
        })?;
    let enabled_paths: Vec<&str> = enabled
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder manifest {reference:?} modset.dlc.enabled[{index}] is not a string"
                ))
            })
        })
        .collect::<Result<_, _>>()?;

    let display_names = modset
        .get("dlc_display_names")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc_display_names is missing or not an object"
            ))
        })?;
    let mut active_dlcs = Vec::with_capacity(enabled_paths.len());
    for path in enabled_paths {
        let display = display_names
            .get(path)
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder metadata binding check: enabled DLC path {path:?} has no exactly-mapped nonblank display name"
                ))
            })?;
        if display == PORTABILITY_EXCLUDED_VALUE {
            return Err(SourceError::Open(format!(
                "recorder metadata binding check: enabled DLC path {path:?} has no display \
                 name in this container -- the seal excluded it as non-portable, and the \
                 portability_exclusions ledger names it"
            )));
        }
        active_dlcs.push(display.to_string());
    }
    let settings_binding = modset.get("settings_binding").ok_or_else(|| {
        SourceError::Open(format!(
            "recorder manifest {reference:?} modset.settings_binding is missing"
        ))
    })?;
    let localisation_language = settings_binding
        .get("language")
        .and_then(Value::as_str)
        .filter(|language| {
            !language.trim().is_empty() && *language != PORTABILITY_EXCLUDED_VALUE
        })
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.settings_binding.language is missing, \
                 blank, or excluded as non-portable"
            ))
        })?
        .to_string();
    let display_variant: RecorderDisplayVariant =
        serde_json::from_value(recorder_display_variant_value(settings_binding).ok_or_else(
            || {
                SourceError::Open(format!(
                    "recorder manifest {reference:?} modset.settings_binding.display_variant \
                     is missing"
                ))
            },
        )?)
        .map_err(|error| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.display_variant has invalid shape: {error}"
            ))
        })?;
    validate_display_variant(&display_variant, reference)?;

    let foundation_content =
        decode_recorded_foundation_content(root.get(FOUNDATION_CONTENT_FIELD), reference)?;

    Ok(RecorderSessionBinding {
        active_dlcs,
        localisation_language,
        display_variant,
        campaign_identity,
        product_admission,
        foundation_content,
    })
}

const FOUNDATION_CONTENT_FIELD: &str = "xmap02_foundation_content";

/// Decode the recorded XMAP02 resolved foundation content, if this recording
/// carries one. Three states: absent/null is the pre-producer gap state; a
/// typed gap marker (`xmap02-foundation-content-gap.v1`) is a durable named
/// gap the recorder wrote when production failed - both decode to absent.
/// Anything else must fully validate as the sealed set.
fn decode_recorded_foundation_content(
    value: Option<&Value>,
    reference: &str,
) -> Result<
    Option<portable_content_manifest::foundation_family::FoundationFamilyContentSet>,
    SourceError,
> {
    const GAP_SCHEMA: &str = "hoi4.replay.xmap02-foundation-content-gap.v1";
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) if value.get("schema").and_then(Value::as_str) == Some(GAP_SCHEMA) => {
            if value.as_object().is_some_and(|fields| {
                fields.len() == 2
                    && fields
                        .get("reason")
                        .and_then(Value::as_str)
                        .is_some_and(|reason| !reason.trim().is_empty())
            }) {
                Ok(None)
            } else {
                Err(SourceError::Open(format!(
                    "recorder manifest {reference:?} carries an XMAP02 foundation-content gap \
                     marker without exactly one reason"
                )))
            }
        }
        Some(value) => {
            let set: portable_content_manifest::foundation_family::FoundationFamilyContentSet =
                serde_json::from_value(value.clone()).map_err(|error| {
                    SourceError::Open(format!(
                        "recorder manifest {reference:?} {FOUNDATION_CONTENT_FIELD} is malformed: \
                         {error}"
                    ))
                })?;
            set.validate().map_err(|error| {
                SourceError::Open(format!(
                    "recorder manifest {reference:?} {FOUNDATION_CONTENT_FIELD} failed \
                     validation: {error}"
                ))
            })?;
            Ok(Some(set))
        }
    }
}

/// THE XMAP02 CONSUMER JOIN at open time: a recorded foundation-content set
/// must bind exactly the container's own portable content manifest - its
/// manifest digest must name the container's finalized digest - and it must
/// attest the classification re-derived from that same manifest with the
/// shared closures. Any divergence refuses the recording at open.
pub(crate) fn join_recorded_foundation_content(
    recorded: Option<
        &portable_content_manifest::foundation_family::FoundationFamilyContentSet,
    >,
    container_manifest: Option<&portable_content_manifest::ContentManifest>,
) -> Result<(), SourceError> {
    let Some(recorded) = recorded else {
        return Ok(());
    };
    let container_manifest = container_manifest.ok_or_else(|| {
        SourceError::Open(format!(
            "recorder manifest carries {FOUNDATION_CONTENT_FIELD} but the container has no \
             portable content manifest to join it against"
        ))
    })?;
    let declared_digest = container_manifest
        .manifest_digest_sha256
        .as_deref()
        .unwrap_or("");
    if recorded.manifest_digest_sha256 != declared_digest {
        return Err(SourceError::Open(
            "recorder xmap02_foundation_content disagrees with the container content manifest \
             authority"
                .into(),
        ));
    }
    // The whole join: re-derive the classification from THIS recording's own
    // manifest with the shared closures and require the producer's record to
    // attest exactly it. Any divergence - post-attach drift, tampering, a
    // rewritten record - refuses by name.
    let derived =
        portable_content_manifest::foundation_family::derive_foundation_family_classification(
            container_manifest,
        )
        .map_err(SourceError::Open)?;
    recorded
        .verify_against(&derived)
        .map_err(|error| SourceError::Open(format!(
            "recorder xmap02_foundation_content rejected: {error}"
        )))
}

fn recorder_campaign_identity(
    root: &Map<String, Value>,
    reference: &str,
) -> Result<Option<RecordedCampaignIdentity>, SourceError> {
    const OBJECT_KEYS: &[&str] = &[
        "schema",
        "state",
        "campaign_id",
        "game_unique_id",
        "game_unique_seed",
        "source",
        "source_build_profile",
        "observed_game_hour_raw",
        "stream_header_sha256",
        "resolution_note",
        "witnesses",
    ];
    const WITNESS_KEYS: &[&str] = &[
        "start_date_raw",
        "session",
        "scenario",
        "initial_human_country_tags",
        "modset_identity",
    ];
    let Some(value) = root.get("campaign_identity") else {
        return Ok(None);
    };
    let fail = |message: String| {
        SourceError::Open(format!(
            "recorder manifest {reference:?} campaign_identity {message}"
        ))
    };
    let object = value
        .as_object()
        .ok_or_else(|| fail("is not an object".into()))?;
    if object.len() != OBJECT_KEYS.len() || OBJECT_KEYS.iter().any(|key| !object.contains_key(*key))
    {
        return Err(fail("does not have the exact recorder-written key set".into()));
    }
    if object.get("schema").and_then(Value::as_str) != Some("hoi4.campaign_identity.v1") {
        return Err(fail("has an unsupported schema".into()));
    }
    let witnesses = object
        .get("witnesses")
        .and_then(Value::as_object)
        .ok_or_else(|| fail("witnesses is not an object".into()))?;
    if witnesses.len() != WITNESS_KEYS.len()
        || WITNESS_KEYS.iter().any(|key| !witnesses.contains_key(*key))
    {
        return Err(fail("witnesses does not have the exact recorder-written key set".into()));
    }
    for key in ["start_date_raw", "session"] {
        let field = &witnesses[key];
        if !field.is_null()
            && field
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .is_none()
        {
            return Err(fail(format!("witness {key} is not null or u32")));
        }
    }
    for key in ["scenario", "modset_identity"] {
        let field = &witnesses[key];
        if !field.is_null() && field.as_str().is_none() {
            return Err(fail(format!("witness {key} is not null or string")));
        }
    }
    let roster = &witnesses["initial_human_country_tags"];
    if !roster.is_null()
        && roster
            .as_array()
            .map_or(true, |rows| rows.iter().any(|row| row.as_str().is_none()))
    {
        return Err(fail(
            "witness initial_human_country_tags is not null or a string array".into(),
        ));
    }
    let recorded_witnesses = RecordedCampaignIdentityWitnesses {
        start_date_raw: witnesses["start_date_raw"]
            .as_u64()
            .map(|value| value as u32),
        session: witnesses["session"].as_u64().map(|value| value as u32),
        scenario: witnesses["scenario"].as_str().map(str::to_string),
        initial_human_country_tags: roster.as_array().map(|rows| {
            rows.iter()
                .map(|row| row.as_str().expect("witness roster validated").to_string())
                .collect()
        }),
        modset_identity: witnesses["modset_identity"].as_str().map(str::to_string),
    };

    let state = object
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("state is not a string".into()))?;
    let nullable_string = |key: &str| -> Result<Option<String>, SourceError> {
        let value = &object[key];
        if value.is_null() {
            Ok(None)
        } else {
            value
                .as_str()
                .map(|value| Some(value.to_string()))
                .ok_or_else(|| fail(format!("{key} is not null or string")))
        }
    };
    let campaign_id = nullable_string("campaign_id")?;
    let game_unique_id = nullable_string("game_unique_id")?;
    let game_unique_seed = nullable_string("game_unique_seed")?;
    let source = nullable_string("source")?;
    let source_build_profile = nullable_string("source_build_profile")?;
    let stream_header_sha256 = nullable_string("stream_header_sha256")?;
    let resolution_note = nullable_string("resolution_note")?;
    if source_build_profile
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        return Err(fail("source_build_profile violates its closed grammar".into()));
    }
    let observed_game_hour_raw = if object["observed_game_hour_raw"].is_null() {
        None
    } else {
        let raw = object["observed_game_hour_raw"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| fail("observed_game_hour_raw is not null or i32".into()))?;
        if !portable_admission::raw_date_in_range(raw) {
            return Err(fail("observed_game_hour_raw is outside the compact calendar".into()));
        }
        Some(raw)
    };
    let lower_hex64 = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };

    match state {
        "known" => {
            let campaign_id = campaign_id
                .as_deref()
                .filter(|value| lower_hex64(value))
                .ok_or_else(|| fail("known campaign_id is not lowercase SHA-256".into()))?;
            let uuid = game_unique_id
                .as_deref()
                .filter(|value| canonical_campaign_uuid(value))
                .ok_or_else(|| fail("known game_unique_id is not a canonical lowercase UUID".into()))?;
            let seed = game_unique_seed
                .as_deref()
                .and_then(|value| value.parse::<i32>().ok().map(|parsed| (value, parsed)))
                .filter(|(value, parsed)| *value == parsed.to_string())
                .map(|(value, _)| value)
                .ok_or_else(|| fail("known game_unique_seed is not canonical signed i32".into()))?;
            let mut preimage = b"hoi4-campaign-id-v1\0".to_vec();
            preimage.extend_from_slice(uuid.as_bytes());
            preimage.push(0);
            preimage.extend_from_slice(seed.as_bytes());
            let recomputed = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&preimage));
            if recomputed != campaign_id {
                return Err(fail("known campaign digest does not recompute".into()));
            }
            if source.as_deref() != Some("live_memory")
                || source_build_profile.is_none()
                || observed_game_hour_raw.is_none()
                || stream_header_sha256
                    .as_deref()
                    .map_or(true, |value| !lower_hex64(value))
                || resolution_note.is_some()
            {
                return Err(fail("known state fields are incoherent".into()));
            }
        }
        "unavailable" | "unbound" | "identity_component_conflict" => {
            if campaign_id.is_some()
                || game_unique_id.is_some()
                || game_unique_seed.is_some()
                || source.is_some()
                || observed_game_hour_raw.is_some()
                || stream_header_sha256.is_some()
                || resolution_note.as_deref().map_or(true, str::is_empty)
            {
                return Err(fail("non-known state contains positive identity authority".into()));
            }
        }
        _ => return Err(fail(format!("has unknown state {state:?}"))),
    }
    Ok(Some(RecordedCampaignIdentity {
        state: state.to_string(),
        campaign_id,
        game_unique_id,
        game_unique_seed,
        source,
        source_build_profile,
        observed_game_hour_raw,
        stream_header_sha256,
        resolution_note,
        witnesses: recorded_witnesses,
    }))
}

fn canonical_campaign_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}

fn validate_display_variant(
    display: &RecorderDisplayVariant,
    reference: &str,
) -> Result<(), SourceError> {
    let scale = f32::from_bits(display.gui_scale_f32_bits);
    let expected_mode = match (display.fullscreen, display.borderless) {
        (true, false) => "fullscreen",
        (false, true) => "borderless_fullscreen",
        (false, false) => "windowed",
        (true, true) => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} display_variant flags are ambiguous"
            )))
        }
    };
    let sha_ok = display.settings_file_sha256.len() == 64
        && display
            .settings_file_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if display.physical_width == 0
        || display.physical_height == 0
        || display.min_gui_width == 0
        || display.min_gui_height == 0
        || display.refresh_rate_hz == 0
        || !scale.is_finite()
        || scale <= 0.0
        || display.display_mode != expected_mode
        || !sha_ok
    {
        return Err(SourceError::Open(format!(
            "recorder manifest {reference:?} modset.display_variant violates the exact launch-settings contract"
        )));
    }
    Ok(())
}

fn empty_index_error(store: &Store, timeline: Option<&Value>, game_id: &str) -> SourceError {
    let contract = "playback requires certified replay_decoder hour-index/checkpoint \
                    roles, legacy per-hour snapshot entries, or an explicitly supplied \
                    certified portable pre-folder; ordinary product open does not guess \
                    compact record layouts";
    match compact_stream_evidence(store, timeline) {
        Some(evidence) => SourceError::UnsupportedCompactStreamOnly(format!(
            "recording {game_id:?} is a compact-stream-only production recording \
             (empty snapshot index; {evidence}): it carries only the raw compact \
             domain stream, and {contract}"
        )),
        None => SourceError::Unsupported(format!(
            "recording {game_id:?} has an empty snapshot index (\"snapshots\": []) \
             and no per-hour snapshots; {contract}"
        )),
    }
}

fn compact_stream_evidence(store: &Store, timeline: Option<&Value>) -> Option<String> {
    if let Some(container) = store.container() {
        let compact = container.entries_by_role(COMPACT_STREAM_ROLE);
        if let Some(entry) = compact.first() {
            return Some(format!(
                "manifest entry {:?} has role {COMPACT_STREAM_ROLE:?}",
                entry.path
            ));
        }
        return timeline.and_then(timeline_compact_evidence);
    }
    timeline.and_then(timeline_compact_evidence)
}

fn timeline_compact_evidence(timeline: &Value) -> Option<String> {
    let format = timeline.get("format").and_then(Value::as_str);
    let role = timeline.get("raw_stream_role").and_then(Value::as_str);
    if format == Some(COMPACT_STREAM_TIMELINE_FORMAT) || role == Some(COMPACT_STREAM_ROLE) {
        Some(format!(
            "timeline.json declares format {:?} / raw_stream_role {:?}",
            format.unwrap_or("<missing>"),
            role.unwrap_or("<missing>")
        ))
    } else {
        None
    }
}

static PREFOLD_SPOOL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const PREFOLD_SPOOL_MARKER: &str = ".hoi4-prefold-owner";

fn admission_refusal(
    error: &snapshot::ReplayCacheError,
    copy_outcome: &Result<snapshot::SpoolEntryReceipt, snapshot::ContainerError>,
) -> String {
    match copy_outcome {
        Ok(_) => format!("admit packaged replay_decoder binary artifacts: {error}"),
        Err(copy_error) => format!(
            "admit packaged replay_decoder binary artifacts: {error} (the container also \
             refused the entry: {copy_error})"
        ),
    }
}

/// RAII ownership for one private bounded pre-fold workspace. Cleanup is
fn open_private_prefold_spool_file(spool_path: &Path) -> Result<fs::File, SourceError> {
    let spool_file = fs::OpenOptions::new()
        .read(true)
        .open(spool_path)
        .map_err(|error| {
            SourceError::Open(format!(
                "open private checkpoint cache spool {}: {error}",
                spool_path.display()
            ))
        })?;
    Ok(spool_file)
}

struct PrefoldSpoolOwner {
    root: PathBuf,
    canonical_root: PathBuf,
    snapshot_dir: PathBuf,
    marker_value: String,
}

impl PrefoldSpoolOwner {
    fn create(base: &Path) -> Result<Self, SourceError> {
        let meta = fs::symlink_metadata(base).map_err(|error| {
            SourceError::Open(format!(
                "inspect prefold spool base {}: {error}",
                base.display()
            ))
        })?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(SourceError::Open(format!(
                "prefold spool base is not a regular non-symlink directory: {}",
                base.display()
            )));
        }
        let canonical_base = fs::canonicalize(base).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize prefold spool base {}: {error}",
                base.display()
            ))
        })?;
        let sequence = PREFOLD_SPOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| SourceError::Open(format!("prefold spool clock: {error}")))?
            .as_nanos();
        let marker_value = format!("{}-{now:032x}-{sequence:016x}", std::process::id());

        for attempt in 0..64u64 {
            let root = canonical_base.join(format!(
                ".hoi4-prefold-{}-{now:032x}-{:016x}",
                std::process::id(),
                sequence.wrapping_add(attempt)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let canonical_root = match fs::canonicalize(&root) {
                        Ok(path) if path.parent() == Some(canonical_base.as_path()) => path,
                        Ok(_) => {
                            let _ = fs::remove_dir(&root);
                            return Err(SourceError::Open(
                                "private prefold spool escaped its canonical base".to_string(),
                            ));
                        }
                        Err(error) => {
                            let _ = fs::remove_dir(&root);
                            return Err(SourceError::Open(format!(
                                "canonicalize private prefold spool {}: {error}",
                                root.display()
                            )));
                        }
                    };
                    let setup = (|| -> Result<PathBuf, SourceError> {
                        let marker_path = canonical_root.join(PREFOLD_SPOOL_MARKER);
                        let mut marker = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&marker_path)
                            .map_err(|error| {
                                SourceError::Open(format!(
                                    "create prefold ownership marker {}: {error}",
                                    marker_path.display()
                                ))
                            })?;
                        marker.write_all(marker_value.as_bytes()).map_err(|error| {
                            SourceError::Open(format!(
                                "write prefold ownership marker {}: {error}",
                                marker_path.display()
                            ))
                        })?;
                        marker.sync_all().map_err(|error| {
                            SourceError::Open(format!(
                                "sync prefold ownership marker {}: {error}",
                                marker_path.display()
                            ))
                        })?;
                        let snapshot_dir = canonical_root.join("snapshots");
                        fs::create_dir(&snapshot_dir).map_err(|error| {
                            SourceError::Open(format!(
                                "create private snapshot spool {}: {error}",
                                snapshot_dir.display()
                            ))
                        })?;
                        Ok(snapshot_dir)
                    })();
                    match setup {
                        Ok(snapshot_dir) => {
                            return Ok(Self {
                                root: canonical_root.clone(),
                                canonical_root,
                                snapshot_dir,
                                marker_value,
                            });
                        }
                        Err(error) => {
                            if fs::canonicalize(&root).ok().as_deref()
                                == Some(canonical_root.as_path())
                            {
                                let _ = fs::remove_dir_all(&root);
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(SourceError::Open(format!(
                        "create private prefold spool {}: {error}",
                        root.display()
                    )))
                }
            }
        }
        Err(SourceError::Open(
            "could not allocate a unique private prefold spool directory".to_string(),
        ))
    }

    fn root(&self) -> &Path {
        &self.root
    }
    fn snapshot_dir(&self) -> &Path {
        &self.snapshot_dir
    }

    fn identity_intact(&self) -> bool {
        let Ok(meta) = fs::symlink_metadata(&self.root) else {
            return false;
        };
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return false;
        }
        if fs::canonicalize(&self.root).ok().as_deref() != Some(self.canonical_root.as_path()) {
            return false;
        }
        let Ok(snapshot_meta) = fs::symlink_metadata(&self.snapshot_dir) else {
            return false;
        };
        if !snapshot_meta.file_type().is_dir() || snapshot_meta.file_type().is_symlink() {
            return false;
        }
        if fs::canonicalize(&self.snapshot_dir).ok().as_deref() != Some(self.snapshot_dir.as_path())
        {
            return false;
        }
        let marker = self.root.join(PREFOLD_SPOOL_MARKER);
        let Ok(marker_meta) = fs::symlink_metadata(&marker) else {
            return false;
        };
        if !marker_meta.file_type().is_file() || marker_meta.file_type().is_symlink() {
            return false;
        }
        if fs::read_to_string(marker).ok().as_deref() != Some(self.marker_value.as_str()) {
            return false;
        }
        let Ok(root_entries) = fs::read_dir(&self.root) else {
            return false;
        };
        for entry in root_entries {
            let Ok(entry) = entry else {
                return false;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            let spooled_artifact = name == "source.h4cr" || name == "checkpoints.h4cp";
            if name != PREFOLD_SPOOL_MARKER && !spooled_artifact && name != "snapshots" {
                return false;
            }
            if spooled_artifact {
                let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                    return false;
                };
                if !meta.file_type().is_file() || meta.file_type().is_symlink() {
                    return false;
                }
                let Ok(canonical) = fs::canonicalize(entry.path()) else {
                    return false;
                };
                if canonical.parent() != Some(self.root.as_path()) {
                    return false;
                }
            }
        }
        let Ok(snapshot_entries) = fs::read_dir(&self.snapshot_dir) else {
            return false;
        };
        for entry in snapshot_entries {
            let Ok(entry) = entry else {
                return false;
            };
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                return false;
            };
            if !meta.file_type().is_file() || meta.file_type().is_symlink() {
                return false;
            }
            let Ok(canonical) = fs::canonicalize(entry.path()) else {
                return false;
            };
            if canonical.parent() != Some(self.snapshot_dir.as_path()) {
                return false;
            }
        }
        true
    }

    fn require_identity(&self) -> Result<(), SourceError> {
        if self.identity_intact() {
            Ok(())
        } else {
            Err(SourceError::Open(
                "private prefold spool identity changed; refusing read and automatic cleanup"
                    .to_string(),
            ))
        }
    }

    fn remove_authenticated_source(&self) -> Result<(), SourceError> {
        self.require_identity()?;
        let path = self.root.join("source.h4cr");
        let meta = fs::symlink_metadata(&path).map_err(|error| {
            SourceError::Open(format!(
                "inspect authenticated source spool {}: {error}",
                path.display()
            ))
        })?;
        let canonical = fs::canonicalize(&path).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize authenticated source spool {}: {error}",
                path.display()
            ))
        })?;
        if !meta.file_type().is_file()
            || meta.file_type().is_symlink()
            || canonical.parent() != Some(self.root.as_path())
        {
            return Err(SourceError::Open(
                "authenticated source spool identity changed before cleanup".to_string(),
            ));
        }
        fs::remove_file(&path).map_err(|error| {
            SourceError::Open(format!(
                "remove authenticated source spool {}: {error}",
                path.display()
            ))
        })
    }
}

impl Drop for PrefoldSpoolOwner {
    fn drop(&mut self) {
        if self.identity_intact() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn read_spooled_hour(
    owner: &PrefoldSpoolOwner,
    snapshot_dir: &Path,
    spool: &snapshot::PrefoldedSpool,
    entry: &snapshot::PrefoldedSpoolEntry,
) -> Result<snapshot::Snapshot, SourceError> {
    owner.require_identity()?;
    if fs::canonicalize(snapshot_dir).ok().as_deref() != Some(owner.snapshot_dir()) {
        return Err(SourceError::Open(
            "private snapshot spool identity changed".to_string(),
        ));
    }
    let path = snapshot_dir.join(entry.file_name());
    let canonical = fs::canonicalize(&path).map_err(|error| {
        SourceError::Open(format!(
            "canonicalize spooled snapshot {}: {error}",
            path.display()
        ))
    })?;
    if canonical.parent() != Some(snapshot_dir) {
        return Err(SourceError::Open(format!(
            "spooled snapshot escaped private directory: {}",
            path.display()
        )));
    }
    let path_meta = fs::symlink_metadata(&path).map_err(|error| {
        SourceError::Open(format!(
            "inspect spooled snapshot {}: {error}",
            path.display()
        ))
    })?;
    if !path_meta.file_type().is_file()
        || path_meta.file_type().is_symlink()
        || path_meta.len() != entry.byte_len()
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot metadata changed for hour {}",
            entry.hour_index()
        )));
    }
    let file = fs::File::open(&canonical).map_err(|error| {
        SourceError::Open(format!(
            "open spooled snapshot {}: {error}",
            canonical.display()
        ))
    })?;
    if file
        .metadata()
        .map_err(|error| SourceError::Open(error.to_string()))?
        .len()
        != entry.byte_len()
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot length changed while opening hour {}",
            entry.hour_index()
        )));
    }
    let mut bytes = Vec::new();
    file.take(entry.byte_len().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            SourceError::Open(format!(
                "read spooled snapshot for hour {}: {error}",
                entry.hour_index()
            ))
        })?;
    if bytes.len() as u64 != entry.byte_len() {
        return Err(SourceError::Open(format!(
            "spooled snapshot byte count changed for hour {}",
            entry.hour_index()
        )));
    }
    let actual_sha = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes));
    if actual_sha != entry.sha256() {
        return Err(SourceError::Open(format!(
            "spooled snapshot SHA-256 changed for hour {}",
            entry.hour_index()
        )));
    }
    let strict = snapshot::parse_json_strict(&bytes).map_err(|error| {
        SourceError::Open(format!(
            "spooled hour {} is not strict snapshot JSON: {error}",
            entry.hour_index()
        ))
    })?;
    let snapshot: snapshot::Snapshot = serde_json::from_value(strict).map_err(|error| {
        SourceError::Open(format!(
            "spooled hour {} is not canonical snapshot JSON: {error}",
            entry.hour_index()
        ))
    })?;
    if snapshot.hour_index != Some(entry.hour_index())
        || snapshot.game_id.as_deref() != Some(spool.game_id())
        || snapshot.schema_version.as_ref() != Some(spool.schema_version())
        || snapshot.date.as_deref() != Some(entry.date())
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot identity disagrees with certified manifest at hour {}",
            entry.hour_index()
        )));
    }
    Ok(snapshot)
}

struct StoreSource {
    store: Store,
    game_id: String,
    hours: Vec<i64>,
    files_by_hour: BTreeMap<i64, String>,
    dates_by_hour: BTreeMap<i64, Option<String>>,
    declared_coverage: DeclaredCoverage,
    warnings: Vec<String>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
}

struct IndexedReplaySource {
    cache: snapshot::CertifiedReplayCache,
    #[allow(dead_code)]
    cache_spool: Option<PrefoldSpoolOwner>,
    hours: Vec<i64>,
    warnings: Vec<String>,
    store: Store,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: snapshot::PrefoldSourceReceipt,
}

struct SpooledPrefoldedSource {
    spool: snapshot::PrefoldedSpool,
    snapshot_dir: PathBuf,
    owner: PrefoldSpoolOwner,
    warnings: Vec<String>,
    store: Store,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: snapshot::PrefoldSourceReceipt,
}

struct PrefoldedSource {
    replay: snapshot::PrefoldedReplay,
    warnings: Vec<String>,
    store: Option<Store>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: Option<snapshot::PrefoldSourceReceipt>,
}

impl ReplaySource for IndexedReplaySource {
    fn game_id(&self) -> &str {
        self.cache.index().game_id()
    }

    fn set_scrub_interrupt(&self, interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {
        self.cache.set_scrub_interrupt(interrupt);
    }

    fn hours(&self) -> &[i64] {
        &self.hours
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }

    fn foundation_content(
        &self,
    ) -> Option<&portable_content_manifest::foundation_family::FoundationFamilyContentSet>
    {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.foundation_content.as_ref())
    }



    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        Some(&self.prefold_receipt)
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        self.cache
            .index()
            .entry(hour_index)
            .map(|entry| Some(entry.date().to_string()))
            .ok_or_else(|| {
                SourceError::Open(format!(
                    "no indexed replay_decoder hour {hour_index} in game {:?}",
                    self.game_id()
                ))
            })
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let (world, receipt) = self
            .cache
            .reconstruct_value(hour_index)
            .map_err(|error| {
                SourceError::Open(format!(
                    "reconstruct replay_decoder hour {hour_index}: {error}"
                ))
            })?;
        if receipt.applied_deltas > receipt.configured_bound {
            return Err(SourceError::Open(format!(
                "replay_decoder hour {hour_index} exceeded its certified seek bound"
            )));
        }
        let mut snapshot = snapshot::Snapshot::from_world_value(world).map_err(|error| {
            SourceError::Open(format!(
                "replay_decoder hour {hour_index} is not canonical snapshot JSON: {error}"
            ))
        })?;
        if snapshot.hour_index != Some(hour_index)
            || snapshot.game_id.as_deref() != Some(self.game_id())
        {
            return Err(SourceError::Open(format!(
                "replay_decoder reconstructed body identity disagrees at hour {hour_index}"
            )));
        }
        let mut rest = std::mem::take(&mut snapshot.rest);
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date,
            save_version: snapshot.save_version,
            schema_version: snapshot.schema_version,
            rest,
        })
    }
}

impl ReplaySource for SpooledPrefoldedSource {
    fn game_id(&self) -> &str {
        self.spool.game_id()
    }
    fn hours(&self) -> &[i64] {
        self.spool.hours()
    }
    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }

    fn foundation_content(
        &self,
    ) -> Option<&portable_content_manifest::foundation_family::FoundationFamilyContentSet>
    {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.foundation_content.as_ref())
    }



    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        Some(&self.prefold_receipt)
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        let entry = self.spool.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no spooled prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        Ok(Some(entry.date().to_string()))
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let entry = self.spool.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no spooled prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        let snapshot = read_spooled_hour(&self.owner, &self.snapshot_dir, &self.spool, entry)?;
        if let Some(container) = self.store.container() {
            validate_snapshot_container_identity(container, &snapshot)?;
        }
        let mut rest = snapshot.rest;
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date,
            save_version: snapshot.save_version,
            schema_version: snapshot.schema_version,
            rest,
        })
    }
}

impl ReplaySource for PrefoldedSource {
    fn game_id(&self) -> &str {
        self.replay.index().game_id()
    }

    fn hours(&self) -> &[i64] {
        self.replay.index().hours()
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .as_ref()
            .and_then(Store::container)
            .and_then(|container| container.content_manifest())
    }

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }

    fn foundation_content(
        &self,
    ) -> Option<&portable_content_manifest::foundation_family::FoundationFamilyContentSet>
    {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.foundation_content.as_ref())
    }



    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        self.prefold_receipt.as_ref()
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        let entry = self.replay.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        Ok(entry.snapshot().date.clone())
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let entry = self.replay.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        let snapshot = entry.snapshot();
        let mut rest = snapshot.rest.clone();
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date.clone(),
            save_version: snapshot.save_version.clone(),
            schema_version: snapshot.schema_version.clone(),
            rest,
        })
    }
}

impl ReplaySource for StoreSource {
    fn game_id(&self) -> &str {
        &self.game_id
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &self.warnings
    }
    fn declared_coverage(&self) -> Option<&DeclaredCoverage> {
        (!self.declared_coverage.is_empty()).then_some(&self.declared_coverage)
    }
    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }

    fn foundation_content(
        &self,
    ) -> Option<&portable_content_manifest::foundation_family::FoundationFamilyContentSet>
    {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.foundation_content.as_ref())
    }

    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }
    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }
    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        self.dates_by_hour.get(&hour_index).cloned().ok_or_else(|| {
            SourceError::Open(format!(
                "no snapshot for hour_index {hour_index} in game {:?}",
                self.game_id
            ))
        })
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let file = self.files_by_hour.get(&hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no snapshot for hour_index {hour_index} in game {:?}",
                self.game_id
            ))
        })?;
        let snap = self
            .store
            .load_file(&self.game_id, file)
            .map_err(|e| source_error_from_load(&e, format!("load snapshot {file:?}: {e}")))?;
        if let Some(body_hour) = snap.hour_index {
            if body_hour != hour_index {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body hour_index {body_hour} != index hour_index \
                     {hour_index} (index is the authority; refusing inconsistent recording)"
                )));
            }
        }
        if let Some(body_game) = snap.game_id.as_deref() {
            if body_game != self.game_id {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body game_id {body_game:?} != selected game {:?} \
                     (index is the authority; refusing inconsistent recording)",
                    self.game_id
                )));
            }
        }
        if let (Some(index_date), Some(body_date)) = (
            self.dates_by_hour.get(&hour_index).and_then(Option::as_deref),
            snap.date.as_deref(),
        ) {
            if index_date != body_date {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body date {body_date:?} != index date {index_date:?} at \
                     hour_index {hour_index} (index is the authority; refusing inconsistent \
                     recording)"
                )));
            }
        }
        let mut rest = snap.rest;
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snap.date,
            save_version: snap.save_version,
            schema_version: snap.schema_version,
            rest,
        })
    }
}

fn validate_existing_recorder_metadata(
    rest: &Map<String, Value>,
    binding: &RecorderSessionBinding,
) -> Result<(), SourceError> {
    if let Some(session_value) = rest.get("session") {
        let session = session_value
            .as_object()
            .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
        check_string_list(
            session.get("active_dlcs"),
            "session.active_dlcs",
            &binding.active_dlcs,
        )?;
        check_string_list(session.get("dlcs"), "session.dlcs", &binding.active_dlcs)?;
        check_string(
            session.get("localisation_language"),
            "session.localisation_language",
            &binding.localisation_language,
        )?;
        check_string(
            session.get("language"),
            "session.language",
            &binding.localisation_language,
        )?;
        check_exact_value(
            session.get("display_variant"),
            "session.display_variant",
            &serde_json::json!(binding.display_variant),
        )?;
    }

    if let Some(world_value) = rest.get("world") {
        let world = world_value
            .as_object()
            .ok_or_else(|| SourceError::Open("snapshot world is not an object".into()))?;
        if let Some(scenario_value) = world.get("scenario") {
            let scenario = scenario_value.as_object().ok_or_else(|| {
                SourceError::Open("snapshot world.scenario is not an object".into())
            })?;
            check_string_list(
                scenario.get("dlcs"),
                "world.scenario.dlcs",
                &binding.active_dlcs,
            )?;
            check_string(
                scenario.get("localisation_language"),
                "world.scenario.localisation_language",
                &binding.localisation_language,
            )?;
            check_string(
                scenario.get("language"),
                "world.scenario.language",
                &binding.localisation_language,
            )?;
        }
    }
    Ok(())
}

fn check_exact_value(
    value: Option<&Value>,
    context: &str,
    expected: &Value,
) -> Result<(), SourceError> {
    if value.is_some_and(|actual| actual != expected) {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict: {context} does not match launch-bound manifest metadata"
        )));
    }
    Ok(())
}

fn check_string_list(
    value: Option<&Value>,
    context: &str,
    expected: &[String],
) -> Result<(), SourceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let array = value.as_array().ok_or_else(|| {
        SourceError::Open(format!(
            "recorder metadata conflict: {context} is not a string array"
        ))
    })?;
    let actual: Vec<String> = array
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder metadata conflict: {context}[{index}] is not a string"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    if actual != expected {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict at {context}: snapshot {actual:?} != recorder binding {expected:?}"
        )));
    }
    Ok(())
}

fn check_string(value: Option<&Value>, context: &str, expected: &str) -> Result<(), SourceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let actual = value.as_str().ok_or_else(|| {
        SourceError::Open(format!(
            "recorder metadata conflict: {context} is not a string"
        ))
    })?;
    if actual != expected {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict at {context}: snapshot {actual:?} != recorder binding {expected:?}"
        )));
    }
    Ok(())
}

fn bind_or_match(
    object: &mut Map<String, Value>,
    key: &str,
    binding: Value,
) -> Result<(), SourceError> {
    match object.get(key) {
        None => {
            object.insert(key.to_string(), binding);
            Ok(())
        }
        Some(existing) if *existing == binding => Ok(()),
        Some(existing) => Err(SourceError::Open(format!(
            "recorder metadata bind_or_match conflict at session.{key}: snapshot {existing} != recorder binding {binding}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "replay_source_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn integrity_is_a_typed_sibling_of_open_with_an_identical_rendering() {
        let message = "entry payload/g/h.json: sha256 aa != manifest bb";
        assert_eq!(
            SourceError::Integrity(message.into()).to_string(),
            SourceError::Open(message.into()).to_string()
        );
        assert!(matches!(
            SourceError::Integrity(message.into()),
            SourceError::Integrity(_)
        ));
    }

    #[test]
    fn load_failures_are_classified_by_type_not_by_message_text() {
        let integrity: Vec<snapshot::LoadError> = vec![
            snapshot::LoadError::Container(snapshot::ContainerError::PayloadIntegrity(
                "entry x: sha256 aa != manifest bb".into(),
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::PayloadIntegrity("entry x: payload crc32 mismatch".into()),
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::SizeMismatch {
                    name: "x".into(),
                    declared: 10,
                    got: 9,
                },
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::Decompress("inflate failed".into()),
            )),
        ];
        for error in &integrity {
            assert!(
                matches!(
                    source_error_from_load(error, "m".into()),
                    SourceError::Integrity(_)
                ),
                "{error} must be the integrity class"
            );
        }

        let not_integrity: Vec<snapshot::LoadError> = vec![
            snapshot::LoadError::Container(snapshot::ContainerError::Reject(
                "manifest missing required_features".into(),
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::NoManifest),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::Truncated("central directory"),
            )),
            snapshot::LoadError::Directory("anchor escaped its root".into()),
            snapshot::LoadError::Container(snapshot::ContainerError::Reject(
                "entry x: sha256 field is not lowercase hex".into(),
            )),
        ];
        for error in &not_integrity {
            assert!(
                matches!(
                    source_error_from_load(error, "m".into()),
                    SourceError::Open(_)
                ),
                "{error} must stay the ordinary open class"
            );
        }

        assert!(matches!(
            source_error_from_load(
                &snapshot::LoadError::Container(snapshot::ContainerError::ModsetMismatch {
                    manifest: "a".into(),
                    snapshot: "b".into(),
                }),
                "m".into()
            ),
            SourceError::ModsetMismatch(_)
        ));
    }

    fn function_body(name: &str) -> String {
        let source = include_str!("source.rs").replace("\r\n", "\n");
        let source = source.as_str();
        let start = source
            .find(&format!("\nfn {name}("))
            .unwrap_or_else(|| panic!("{name} must exist in this module"));
        let open = source[start..]
            .find(" {\n")
            .unwrap_or_else(|| panic!("{name} must have a body"))
            + start;
        let bytes = source.as_bytes();
        let mut depth = 0usize;
        for (offset, byte) in bytes[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return source[open..open + offset + 1].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("{name} body is unbalanced");
    }

    #[test]
    fn both_compact_routes_admit_the_product_ceiling_and_every_prefold_route_proves_identity() {
        for route in [
            "open_compact_stream_with_decoder",
            "open_compact_stream_with_spooled_decoder",
        ] {
            assert!(
                function_body(route).contains("admit_product_compact_stream_bytes("),
                "{route} must apply the shipped compact-playback ceiling before materializing \
                 or spooling the stream"
            );
        }
        let eager = function_body("validate_prefolded_container_identity");
        assert!(
            eager.contains("for entry in replay.entries()"),
            "the eager prefold route must prove identity for EVERY hour, not just the first"
        );
        let lazy = include_str!("source.rs");
        let lazy_frame = lazy
            .find("impl ReplaySource for SpooledPrefoldedSource")
            .expect("the lazy spooled source must exist");
        let lazy_body = &lazy[lazy_frame..];
        let lazy_end = lazy_body
            .find("\nimpl ")
            .map(|offset| offset + 1)
            .unwrap_or(lazy_body.len());
        assert!(
            lazy_body[..lazy_end].contains("validate_snapshot_container_identity(container,"),
            "the lazy spooled source must re-prove container identity for each hour it serves"
        );
    }

    #[test]
    fn shipped_compact_stream_budget_is_closed_at_two_gibibytes() {
        assert!(admit_product_compact_stream_bytes(PRODUCT_COMPACT_STREAM_MAX_BYTES).is_ok());
        let error = admit_product_compact_stream_bytes(PRODUCT_COMPACT_STREAM_MAX_BYTES + 1)
            .expect_err("one byte beyond the product budget must fail before spooling");
        assert!(error.to_string().contains("2 GiB"));
        assert!(PRODUCT_COMPACT_STREAM_MAX_BYTES < 64 * 1024 * 1024 * 1024);
    }

    #[test]
    fn recorder_recovery_and_incomplete_production_profiles_are_not_playable() {
        let admitted = json!({
            "scheduled_domains": ["fuel_live"],
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
            "production_profile": {
                "enabled": true,
                "profile_domains": ["fuel_live"],
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        let full = validate_recorder_product_admission(admitted.as_object().unwrap()).unwrap();
        assert!(
            full.is_full_production_profile(),
            "an enabled profile with three empty lists holds every guarantee"
        );

        let mut recovery = admitted.clone();
        recovery["replay_artifact"]["recovery"] = json!(true);
        recovery["replay_artifact"]["reason"] = json!("gate_failed");
        recovery["replay_artifact"]["gate_problems"] = json!(["domain_missing"]);
        let error = validate_recorder_product_admission(recovery.as_object().unwrap())
            .expect_err("recovery replay must never enter production playback");
        assert!(error.to_string().contains("recovery-only"));

        let mut incomplete = admitted;
        incomplete["production_profile"]["mandatory_domains_not_production_complete"] =
            json!(["weather_live"]);
        let error = validate_recorder_product_admission(incomplete.as_object().unwrap())
            .expect_err("non-complete mandatory production domain must fail closed");
        assert!(
            error
                .to_string()
                .contains("mandatory_domains_not_production_complete")
        );

        let mut incomplete = json!({
            "scheduled_domains": ["fuel_live"],
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
            "production_profile": {
                "enabled": true,
                "profile_domains": ["fuel_live", "technology_mio_live"],
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        incomplete["production_profile"]["profile_domains_not_scheduled"] =
            json!(["technology_mio_live"]);
        let error = validate_recorder_product_admission(incomplete.as_object().unwrap())
            .expect_err("incomplete production profile must fail closed");
        assert!(error.to_string().contains("profile_domains_not_scheduled"));

        let missing_positive_authority = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
            },
            "production_profile": {
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        assert!(
            validate_recorder_product_admission(
                missing_positive_authority.as_object().unwrap()
            )
            .is_err(),
            "present admission authorities must be complete and positive"
        );
    }

    #[test]
    fn late_attach_captures_are_admitted_with_their_shortfall_named() {
        let late_attach = |mandatory: Value, domains: Value, artifacts: Value, enabled: bool| {
            let scheduled = ["fuel_live", "production_live"];
            let mut profile_domains = scheduled.iter().map(|d| json!(d)).collect::<Vec<_>>();
            profile_domains.extend(domains.as_array().expect("domain list").iter().cloned());
            json!({
                "scheduled_domains": scheduled,
                "replay_artifact": {
                    "recovery": false,
                    "reason": "gate_passed",
                    "gate_problems": [],
                },
                "production_profile": {
                    "enabled": enabled,
                    "profile_domains": profile_domains,
                    "mandatory_domains_not_production_complete": mandatory,
                    "profile_domains_not_scheduled": domains,
                    "missing_required_static_artifacts": artifacts,
                },
            })
        };

        let product = late_attach(json!([]), json!([]), json!(["focus_defs_static"]), false);
        let admission =
            validate_recorder_product_admission(product.as_object().unwrap()).expect(
                "a late-attach capture must open; --profile production is early-launch-only",
            );
        assert!(!admission.is_full_production_profile());
        assert_eq!(
            admission.guarantees_not_held,
            vec![ProductionProfileGuarantee::ProductionProfileEnabled]
        );
        assert_eq!(
            admission.missing_required_static_artifacts,
            vec!["focus_defs_static".to_string()],
            "the declaration is read verbatim here; the roster join replaces it with the \
             package's own answer after proving the two agree"
        );
        assert!(admission.profile_domains_not_scheduled.is_empty());

        let probe = late_attach(
            json!([]),
            json!(["decision_panel_live"]),
            json!(["focus_defs_static"]),
            false,
        );
        let admission =
            validate_recorder_product_admission(probe.as_object().unwrap()).unwrap();
        assert_eq!(
            admission.guarantees_not_held,
            vec![
                ProductionProfileGuarantee::ProductionProfileEnabled,
                ProductionProfileGuarantee::FullProfileDomainSchedule,
            ]
        );
        assert_eq!(
            admission.profile_domains_not_scheduled,
            vec!["decision_panel_live".to_string()]
        );

        let admission =
            validate_recorder_product_admission(
                late_attach(json!([]), json!([]), json!([]), false)
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            admission.guarantees_not_held,
            vec![ProductionProfileGuarantee::ProductionProfileEnabled]
        );

        let error = validate_recorder_product_admission(
            late_attach(json!(["weather_live"]), json!([]), json!([]), false)
                .as_object()
                .unwrap(),
        )
        .expect_err("a not-production-complete producer refuses whether or not the profile ran");
        assert!(error
            .to_string()
            .contains("mandatory_domains_not_production_complete"));

        let error = validate_recorder_product_admission(
            late_attach(json!([]), json!([]), json!(["focus_defs_static"]), true)
                .as_object()
                .unwrap(),
        )
        .expect_err("a capture claiming the production profile must satisfy all of it");
        assert!(error
            .to_string()
            .contains("missing_required_static_artifacts"));

        let mut absent = late_attach(json!([]), json!([]), json!([]), false);
        absent["production_profile"]
            .as_object_mut()
            .unwrap()
            .remove("enabled");
        let error = validate_recorder_product_admission(absent.as_object().unwrap())
            .expect_err("an absent enabled is a malformed authority, not a false one");
        assert!(error
            .to_string()
            .contains("enabled is absent or is not a boolean"));

        for broken in [json!(null), json!("false"), json!(0)] {
            let mut manifest = late_attach(json!([]), json!([]), json!([]), false);
            manifest["production_profile"]["enabled"] = broken.clone();
            let error = validate_recorder_product_admission(manifest.as_object().unwrap())
                .expect_err("a non-boolean enabled is a malformed authority");
            assert!(
                error.to_string().contains("enabled is absent or is not a boolean"),
                "{broken} was not refused as a malformed authority: {error}"
            );
        }

        let mut manifest = late_attach(json!([]), json!([]), json!([]), false);
        manifest["production_profile"]["profile_domains_not_scheduled"] = json!([7]);
        let error = validate_recorder_product_admission(manifest.as_object().unwrap())
            .expect_err("a non-string list element is a malformed authority");
        assert!(error
            .to_string()
            .contains("profile_domains_not_scheduled[0] is not a string"));

        let mut overstated = late_attach(json!([]), json!(["fuel_live"]), json!([]), false);
        overstated["production_profile"]["profile_domains"] =
            json!(["fuel_live", "production_live"]);
        let error = validate_recorder_product_admission(overstated.as_object().unwrap())
            .expect_err("a scheduled domain may not be declared unscheduled");
        assert!(error
            .to_string()
            .contains("which the manifest's own scheduled_domains carries"));

        // registry resolves (`state_ownership` is the same domain as the
        let mut understated = late_attach(json!([]), json!([]), json!([]), false);
        understated["production_profile"]["profile_domains"] =
            json!(["fuel_live", "production_live", "decision_panel_live"]);
        let admission = validate_recorder_product_admission(understated.as_object().unwrap())
            .expect("under-declaration is not computable here; see the function docs");
        assert_eq!(
            admission.guarantees_not_held,
            vec![ProductionProfileGuarantee::ProductionProfileEnabled],
            "if this ever starts naming FullProfileDomainSchedule, the difference became \
             computable and the comment above is stale"
        );

        let mut aliased = late_attach(json!([]), json!([]), json!([]), true);
        aliased["production_profile"]["profile_domains"] = json!(["map_control"]);
        aliased["scheduled_domains"] = json!(["state_ownership"]);
        validate_recorder_product_admission(aliased.as_object().unwrap())
            .expect("a legitimate full-profile capture scheduling a legacy alias must open");

        let mut foreign = late_attach(json!([]), json!([]), json!([]), false);
        foreign["production_profile"]["profile_domains_not_scheduled"] =
            json!(["not_a_profile_domain"]);
        let error = validate_recorder_product_admission(foreign.as_object().unwrap())
            .expect_err("a name outside the profile roster must refuse");
        assert!(error
            .to_string()
            .contains("is not in production_profile.profile_domains"));

        let mut repeated = late_attach(json!([]), json!([]), json!([]), false);
        repeated["scheduled_domains"] = json!(["fuel_live", "fuel_live", "production_live"]);
        let error = validate_recorder_product_admission(repeated.as_object().unwrap())
            .expect_err("a repeated scheduled domain is malformed");
        assert!(error.to_string().contains("repeats a domain name"));

        let unstated = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
        });
        let admission =
            validate_recorder_product_admission(unstated.as_object().unwrap()).unwrap();
        assert_eq!(admission, RecorderProductAdmission::unstated());
        assert_eq!(admission.authority, ProductionProfileAuthority::Unstated);
        assert_eq!(
            admission.guarantees_not_held,
            vec![
                ProductionProfileGuarantee::ProductionProfileEnabled,
                ProductionProfileGuarantee::FullProfileDomainSchedule,
            ]
        );

        assert_eq!(
            validate_recorder_product_admission(
                late_attach(json!([]), json!([]), json!([]), false)
                    .as_object()
                    .unwrap(),
            )
            .unwrap()
            .authority,
            ProductionProfileAuthority::Disabled
        );
        assert_eq!(
            validate_recorder_product_admission(
                late_attach(json!([]), json!([]), json!([]), true)
                    .as_object()
                    .unwrap(),
            )
            .unwrap()
            .authority,
            ProductionProfileAuthority::Enabled
        );
    }

    #[test]
    fn recorder_compact_stream_identity_must_match_selected_role() {
        let manifest = json!({
            "stream_storage": {
                "decoded_bytes": 4096,
                "decoded_sha256": "a".repeat(64),
            },
            "replay_entries": {
                "stream": "collector_raw/collector_stream.bin",
                "stream_role": "collector_compact_stream",
            },
        });
        let manifest = manifest.as_object().unwrap();
        validate_recorder_compact_stream_join(
            "payload/game/collector_raw/collector_stream.bin",
            4096,
            &"a".repeat(64),
            "game",
            Some(manifest),
        )
        .unwrap();

        let error = validate_recorder_compact_stream_join(
            "payload/game/collector_raw/collector_stream.bin",
            4097,
            &"a".repeat(64),
            "game",
            Some(manifest),
        )
        .expect_err("different role bytes must not enter prefolding");
        assert!(error.to_string().contains("decoded identity"));
    }

    fn portable_content_manifest_fixture() -> portable_content_manifest::ContentManifest {
        let mut manifest = portable_content_manifest::ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: "a".repeat(64),
            source_game: portable_content_manifest::SourceGameIdentity {
                version: "1.0".into(),
                build: "fixture".into(),
                executable_sha256: "b".repeat(64),
                build_profile: "fixture-profile".into(),
            },
            source_profile: portable_content_manifest::SourceProfileIdentity {
                profile_id: "fixture-profile".into(),
                playset_id: "fixture-playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: "c".repeat(64),
            ordered_layers: Vec::new(),
            dlc_selection: Vec::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::from([
                ("physical_width".into(), "1920".into()),
                ("physical_height".into(), "1080".into()),
                ("min_gui_width".into(), "1920".into()),
                ("min_gui_height".into(), "1080".into()),
                ("gui_scale_f32_bits".into(), 1.0f32.to_bits().to_string()),
                ("refresh_rate_hz".into(), "60".into()),
                ("max_refresh_rate_hz".into(), "unavailable".into()),
                ("fullscreen".into(), "false".into()),
                ("borderless".into(), "true".into()),
                ("display_mode".into(), "borderless_fullscreen".into()),
                ("settings_file_sha256".into(), "e".repeat(64)),
                ("settings_authority".into(), "pre_launch_bracketed".into()),
            ]),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    #[test]
    fn recorder_and_container_content_authorities_must_join_exactly() {
        let container = portable_content_manifest_fixture();
        let recorder = json!({
            "portable_content_manifest": serde_json::to_value(&container).unwrap(),
            "modset": {
                "game": {"exe_sha256": container.source_game.executable_sha256.clone()},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": container.localisation_language.clone(),
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1920,
                        "min_gui_height": 1080,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": null,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "e".repeat(64),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            }
        })
        .as_object()
        .unwrap()
        .clone();
        validate_recorder_content_manifest_join(&recorder, Some(&container)).unwrap();

        let mut wrong_launch = recorder.clone();
        wrong_launch
            .get_mut("modset")
            .unwrap()
            .get_mut("game")
            .unwrap()["exe_sha256"] = json!("d".repeat(64));
        let error = validate_recorder_content_manifest_join(&wrong_launch, Some(&container))
            .expect_err("content authority must join the recorded launch executable");
        assert!(error.to_string().contains("executable disagrees"));

        let mut wrong_display = recorder.clone();
        wrong_display
            .get_mut("modset")
            .unwrap()
            .get_mut("settings_binding")
            .unwrap()
            .get_mut("display_variant")
            .unwrap()["physical_width"] = json!(2560);
        let error = validate_recorder_content_manifest_join(&wrong_display, Some(&container))
            .expect_err("content settings must join the recorded display variant");
        assert!(error.to_string().contains("display variant disagrees"));

        let missing = Map::new();
        let error = validate_recorder_content_manifest_join(&missing, Some(&container))
            .expect_err("container-only authority must fail closed");
        assert!(error.to_string().contains("recorder manifest omits"));

        let mut different = container.clone();
        different.product_modset_sha256 = "d".repeat(64);
        different.manifest_digest_sha256 = None;
        different = different.with_computed_digest().unwrap();
        let error = validate_recorder_content_manifest_join(&recorder, Some(&different))
            .expect_err("different valid authorities must fail closed");
        assert!(error.to_string().contains("disagrees"));

        let error = validate_recorder_content_manifest_join(&recorder, None)
            .expect_err("recorder-only authority must fail closed");
        assert!(error.to_string().contains("container manifest omits"));
    }

    #[test]
    fn private_prefold_spool_cleanup_is_identity_gated() {
        let base = temp_root("prefold_spool_owner");
        let owner = PrefoldSpoolOwner::create(&base).unwrap();
        let owned_root = owner.root().to_path_buf();
        drop(owner);
        assert!(!owned_root.exists(), "intact private spool must be cleaned");

        let owner = PrefoldSpoolOwner::create(&base).unwrap();
        let preserved_root = owner.root().to_path_buf();
        fs::write(preserved_root.join(PREFOLD_SPOOL_MARKER), "different-owner").unwrap();
        drop(owner);
        assert!(
            preserved_root.exists(),
            "identity mismatch must preserve evidence instead of recursive deletion"
        );
        portable_test_fs_retry::remove_dir_all(&preserved_root).unwrap();
        portable_test_fs_retry::remove_dir_all(&base).unwrap();
    }

    fn write_playable_store(root: &Path, game: &str, recorder: bool, label: &str) {
        let game_dir = root.join(game);
        fs::create_dir_all(game_dir.join("recorder")).unwrap();
        fs::write(
            game_dir.join("index.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            game_dir.join("000000000001.snapshot.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "hour_index": 1,
                "marker": label
            }))
            .unwrap(),
        )
        .unwrap();
        if recorder {
            fs::write(
                game_dir.join("timeline.json"),
                br#"{"session_manifest":"recorder/native_session_manifest.json"}"#,
            )
            .unwrap();
            fs::write(
                game_dir
                    .join("recorder")
                    .join("native_session_manifest.json"),
                serde_json::to_vec(&json!({
                    "modset": {
                        "resolved": true,
                        "errors": [],
                        "dlc": {"enabled": ["dlc/a.dlc"]},
                        "dlc_display_names": {"dlc/a.dlc": "DLC A"},
                        "settings_binding": {
                            "authority": "pre_launch_bracketed",
                            "launch_proven": true,
                            "errors": [],
                            "language": "l_english",
                            "display_variant": {
                                "physical_width": 2752, "physical_height": 1152,
                                "min_gui_width": 1280, "min_gui_height": 720,
                                "gui_scale_f32_bits": 1.0f32.to_bits(),
                                "refresh_rate_hz": 60, "max_refresh_rate_hz": 75,
                                "fullscreen": false, "borderless": true,
                                "display_mode": "borderless_fullscreen",
                            },
                            "attach_settings_sha256": "aa".repeat(32),
                            "seal_settings_sha256": null,
                            "endpoint_semantics_equal": null
                        },
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        } else {
            fs::write(game_dir.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
        }
    }

    fn write_portable_recorder_fixture(
        path: &Path,
        game: &str,
        label: &str,
        campaign_identity: Option<Value>,
    ) {
        let content = portable_content_manifest_fixture();
        let absent_static_artifacts = REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, path)| {
                json!({
                    "name": name,
                    "path": path,
                    "present": false,
                })
            })
            .collect::<Vec<_>>();
        let mut recorder_manifest = json!({
            "replay_game_id": game,
            "replay_packaging": {
                "static_artifacts": [],
            },
            snapshot::RAW_STREAM_PACKAGING_FIELD:
                snapshot::RawStreamPackaging::Omitted.as_str(),
            "portable_content_manifest": serde_json::to_value(&content).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": content.localisation_language.clone(),
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1920,
                        "min_gui_height": 1080,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": null,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "e".repeat(64),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            },
        });
        if let Some(campaign_identity) = campaign_identity {
            recorder_manifest["campaign_identity"] = campaign_identity;
        }
        let entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "host_fold_skipped": false,
                    "static_artifacts": absent_static_artifacts,
                    snapshot::RAW_STREAM_PACKAGING_FIELD:
                        snapshot::RawStreamPackaging::Omitted.as_str(),
                    "snapshots": [{
                        "hour_index": 1,
                        "file": "000000000001.snapshot.json",
                    }],
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: serde_json::to_vec(&json!({
                    "format": COMPACT_STREAM_TIMELINE_FORMAT,
                    "stream_format": "compact",
                    "raw_stream_role": COMPACT_STREAM_ROLE,
                    "session_manifest": "recorder/native_session_manifest.json",
                    "session_manifest_role": "recorder_manifest",
                    snapshot::RAW_STREAM_PACKAGING_FIELD:
                        snapshot::RawStreamPackaging::Omitted.as_str(),
                }))
                .unwrap(),
                role: "timeline",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
                role: "recorder_manifest",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/000000000001.snapshot.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "hour_index": 1,
                    "marker": label,
                }))
                .unwrap(),
                role: "snapshot",
                hour_index: Some(1),
            },
        ];
        let packed = snapshot::pack(
            game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(path, packed).unwrap();
    }

    #[must_use = "the scanner guard must be held until the recording is opened"]
    fn write_portable_binary_replay_fixture(
        path: &Path,
        valid_checkpoint_role: bool,
    ) -> std::fs::File {
        write_portable_binary_replay_fixture_with_composition(
            path,
            valid_checkpoint_role,
            snapshot::RawStreamPackaging::Packaged,
        )
    }

    #[must_use = "the scanner guard must be held until the recording is opened"]
    fn write_portable_binary_replay_fixture_with_composition(
        path: &Path,
        valid_checkpoint_role: bool,
        packaging: snapshot::RawStreamPackaging,
    ) -> std::fs::File {
        let game = "binary-game";
        let source_bytes = b"H4CRcanonical-world-source";
        let source_sha256 = snapshot::sha256::Sha256::hex(source_bytes);
        let worlds = [
            serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "hour_index": 10,
                "date": "1936.1.1.10",
                "countries": {"FRA": {"stability": 0.50}},
                "states": {},
            }))
            .unwrap(),
            serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "hour_index": 11,
                "date": "1936.1.1.11",
                "countries": {"FRA": {"stability": 0.55}},
                "states": {},
            }))
            .unwrap(),
        ];
        let first_end = 8;
        let mut encoder =
            snapshot::ReplayCacheEncoder::new(Vec::new(), &source_sha256, 2).unwrap();
        encoder
            .push(snapshot::CacheBuildHour {
                hour_index: 10,
                date: "1936.1.1.10".into(),
                h4cr_start_offset: 0,
                h4cr_end_offset: first_end,
                world_json: worlds[0].clone(),
            })
            .unwrap();
        encoder
            .push(snapshot::CacheBuildHour {
                hour_index: 11,
                date: "1936.1.1.11".into(),
                h4cr_start_offset: first_end,
                h4cr_end_offset: source_bytes.len() as u64,
                world_json: worlds[1].clone(),
            })
            .unwrap();
        let (checkpoint_bytes, replay_index) = encoder
            .finish(
                game,
                "1",
                snapshot::CacheIdentityMode::Legacy,
                source_bytes.len() as u64,
            )
            .unwrap();
        let hour_index_bytes = replay_index.to_bytes().unwrap();
        let hour_index_sha256 = snapshot::sha256::Sha256::hex(&hour_index_bytes);
        let checkpoint_sha256 = snapshot::sha256::Sha256::hex(&checkpoint_bytes);
        let absent_static_artifacts = REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, artifact_path)| {
                json!({
                    "name": name,
                    "path": artifact_path,
                    "present": false,
                })
            })
            .collect::<Vec<_>>();
        let index_bytes = serde_json::to_vec(&json!({
            "game_id": game,
            "schema_version": "1",
            "host_fold_skipped": false,
            "production_json_snapshots_omitted": true,
            "canonical_world_source": snapshot::COMPACT_STREAM_ROLE,
            snapshot::RAW_STREAM_PACKAGING_FIELD: packaging.as_str(),
            "snapshots": [],
            "static_artifacts": absent_static_artifacts,
            "replay_decoder": {
                "schema": "hoi4.replay_decoder.binary_seek.v1",
                "hour_index": {
                    "role": snapshot::HOUR_INDEX_ROLE,
                    "path": snapshot::HOUR_INDEX_FILE,
                    "bytes": hour_index_bytes.len(),
                    "sha256": hour_index_sha256,
                    "hours": 2,
                    "source_bytes": source_bytes.len(),
                    "source_sha256": source_sha256,
                },
                "checkpoints": {
                    "role": snapshot::CHECKPOINT_CACHE_ROLE,
                    "path": snapshot::CHECKPOINT_CACHE_FILE,
                    "bytes": checkpoint_bytes.len(),
                    "sha256": checkpoint_sha256,
                    "checkpoint_count": 1,
                    "checkpoint_interval_hours": 2,
                    "max_seek_deltas": 1,
                },
                "legacy_json_bytes_avoided": worlds.iter().map(Vec::len).sum::<usize>(),
                "exact_reconstruction_parity": true,
                "bounded_seek_certified": true,
            },
        }))
        .unwrap();
        let content = portable_content_manifest_fixture();
        let mut replay_entries = json!({
            "stream_role": snapshot::COMPACT_STREAM_ROLE,
        });
        if packaging.requires_stream_entry() {
            replay_entries.as_object_mut().unwrap().insert(
                "stream".to_string(),
                json!("collector_raw/collector_stream.bin"),
            );
        }
        let recorder_manifest = serde_json::to_vec(&json!({
            "replay_game_id": game,
            "replay_packaging": {"static_artifacts": []},
            snapshot::RAW_STREAM_PACKAGING_FIELD: packaging.as_str(),
            "replay_entries": replay_entries,
            "stream_storage": {
                "decoded_bytes": source_bytes.len(),
                "decoded_sha256": source_sha256,
            },
            "portable_content_manifest": serde_json::to_value(&content).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": content.localisation_language.clone(),
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1920,
                        "min_gui_height": 1080,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": null,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "e".repeat(64),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            },
        }))
        .unwrap();
        let mut entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: index_bytes,
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: {
                    let mut timeline_value = json!({
                        "format": COMPACT_STREAM_TIMELINE_FORMAT,
                        "stream_format": "compact",
                        "raw_stream_role": snapshot::COMPACT_STREAM_ROLE,
                        "session_manifest": "recorder/native_session_manifest.json",
                        "session_manifest_role": "recorder_manifest",
                        snapshot::RAW_STREAM_PACKAGING_FIELD: packaging.as_str(),
                    });
                    if packaging.requires_stream_entry() {
                        timeline_value.as_object_mut().unwrap().insert(
                            "stream".to_string(),
                            json!("collector_raw/collector_stream.bin"),
                        );
                    }
                    serde_json::to_vec(&timeline_value).unwrap()
                },
                role: "timeline",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: recorder_manifest,
                role: "recorder_manifest",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/{}", snapshot::HOUR_INDEX_FILE),
                bytes: hour_index_bytes,
                role: snapshot::HOUR_INDEX_ROLE,
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/{}", snapshot::CHECKPOINT_CACHE_FILE),
                bytes: checkpoint_bytes,
                role: if valid_checkpoint_role {
                    snapshot::CHECKPOINT_CACHE_ROLE
                } else {
                    "wrong_checkpoint_role"
                },
                hour_index: None,
            },
        ];
        if packaging.requires_stream_entry() {
            entries.push(snapshot::PackEntry {
                path: format!("payload/{game}/collector_raw/collector_stream.bin"),
                bytes: source_bytes.to_vec(),
                role: snapshot::COMPACT_STREAM_ROLE,
                hour_index: None,
            });
        }
        let packed = snapshot::pack(
            game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(path, packed).unwrap();
        portable_test_fs_retry::open(path).expect("hold the fixture open across the scanner's window")
    }

    #[test]
    fn packaged_binary_replay_reconstructs_exact_hours_without_json_snapshots() {
        let root = temp_root("binary_replay");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture(&recording, true);

        let source = open_recording(&recording).unwrap();
        assert_eq!(source.game_id(), "binary-game");
        assert_eq!(source.hours(), &[10, 11]);
        assert_eq!(
            source
                .prefold_receipt()
                .expect("binary replay must retain its H4CR receipt")
                .source_len(),
            b"H4CRcanonical-world-source".len() as u64
        );
        let frame = source.frame(11).unwrap();
        assert_eq!(frame.hour_index, 11);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.11"));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.55));
        assert!(source.frame(12).is_err());
        drop(source);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_only_recording_replays_every_hour_without_the_raw_stream() {
        let root = temp_root("binary_replay_cache_only");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Omitted,
        );

        let container =
            snapshot::Container::open(&recording, &snapshot::ExpectedIdentity::default()).unwrap();
        assert!(container
            .entries_by_role(snapshot::COMPACT_STREAM_ROLE)
            .is_empty());
        drop(container);

        let source = open_recording(&recording).unwrap();
        assert_eq!(source.game_id(), "binary-game");
        assert_eq!(source.hours(), &[10, 11]);
        assert_eq!(
            source
                .prefold_receipt()
                .expect("a cache-only recording still carries its H4CR source receipt")
                .source_len(),
            b"H4CRcanonical-world-source".len() as u64
        );
        let frame = source.frame(11).unwrap();
        assert_eq!(frame.hour_index, 11);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.11"));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.55));
        assert!(source.frame(12).is_err());
        drop(source);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    struct UnusedSpoolDecoder;

    impl snapshot::CertifiedPrefoldSpoolDecoder for UnusedSpoolDecoder {
        fn decoder_id(&self) -> &'static str {
            "test.packaged-cache.never-decodes"
        }

        fn decode_certified_spool(
            &self,
            _request: snapshot::CertifiedPrefoldSpoolRequest,
        ) -> Result<snapshot::PrefoldedSpool, snapshot::PrefoldDecodeError> {
            panic!("a packaged certified cache must never be folded from the raw stream");
        }
    }

    #[test]
    fn a_packaged_cache_opened_through_a_spool_base_matches_the_resident_read() {
        let root = temp_root("binary_replay_cache_spooled");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Omitted,
        );
        let spool_base = root.join("spool");
        fs::create_dir(&spool_base).unwrap();

        let resident = open_recording(&recording).unwrap();
        let spooled =
            open_recording_with_spooled_decoder(&recording, &spool_base, &UnusedSpoolDecoder)
                .unwrap();
        assert_eq!(spooled.game_id(), resident.game_id());
        assert_eq!(spooled.hours(), resident.hours());
        for hour in resident.hours().to_vec() {
            let expected = resident.frame(hour).unwrap();
            let observed = spooled.frame(hour).unwrap();
            assert_eq!(observed.hour_index, expected.hour_index);
            assert_eq!(observed.date, expected.date);
            assert_eq!(observed.rest, expected.rest, "hour {hour} differs by route");
        }

        let spooled_caches: Vec<PathBuf> = fs::read_dir(&spool_base)
            .unwrap()
            .map(|entry| entry.unwrap().path().join("checkpoints.h4cp"))
            .filter(|path| path.is_file())
            .collect();
        assert_eq!(
            spooled_caches.len(),
            1,
            "the spooled route must materialize exactly one private cache"
        );

        drop(spooled);
        assert_eq!(
            fs::read_dir(&spool_base).unwrap().count(),
            0,
            "dropping the source must remove its private spool"
        );
        drop(resident);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    fn rot_sealed_cache_byte_in_recording(recording: &Path, cache_bytes: &[u8], offset: usize) {
        let mut packed = fs::read(recording).unwrap();
        let positions: Vec<usize> = packed
            .windows(cache_bytes.len())
            .enumerate()
            .filter(|(_, window)| *window == cache_bytes)
            .map(|(position, _)| position)
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "the sealed cache must appear exactly once in the packed container"
        );
        packed[positions[0] + offset] ^= 0xff;
        fs::write(recording, &packed).unwrap();
    }

    #[test]
    fn a_rotted_packaged_cache_entry_confines_its_hours_instead_of_the_recording() {
        let root = temp_root("binary_replay_rotted_cache");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Omitted,
        );

        let (cache_bytes, last_byte_of_hour_11) = {
            let container =
                snapshot::Container::open(&recording, &snapshot::ExpectedIdentity::default())
                    .unwrap();
            let index_bytes = container
                .read_entry_bytes(&format!(
                    "payload/binary-game/{}",
                    snapshot::HOUR_INDEX_FILE
                ))
                .unwrap();
            let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).unwrap();
            let row = &index.hours()[1];
            assert_eq!(row.hour_index(), 11, "position 1 serves hour 11");
            assert_eq!(index.hours()[0].hour_index(), 10);
            let offset = (row.cache_record_offset() + row.cache_record_len() - 1) as usize;
            let cache = container
                .read_entry_bytes(&format!(
                    "payload/binary-game/{}",
                    snapshot::CHECKPOINT_CACHE_FILE
                ))
                .unwrap();
            (cache, offset)
        };
        rot_sealed_cache_byte_in_recording(&recording, &cache_bytes, last_byte_of_hour_11);

        let resident = open_recording(&recording).unwrap();
        assert_eq!(resident.hours(), &[10, 11]);
        let frame = resident
            .frame(10)
            .expect("hour 10's record still hashes to the digest the seal published for it");
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.50));
        let refusal = resident.frame(11).unwrap_err().to_string();
        assert!(
            refusal.contains("game hour 11"),
            "the confined hour is refused by name: {refusal}"
        );

        let spool_base = root.join("spool");
        fs::create_dir(&spool_base).unwrap();
        let spooled =
            open_recording_with_spooled_decoder(&recording, &spool_base, &UnusedSpoolDecoder)
                .unwrap();
        assert_eq!(spooled.hours(), resident.hours());
        assert_eq!(
            spooled.frame(10).unwrap().rest,
            resident.frame(10).unwrap().rest,
            "the served hour is byte-identical by route"
        );
        assert!(
            spooled.frame(11).is_err(),
            "the confined hour is confined by route too"
        );

        drop(spooled);
        drop(resident);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn declared_cache_only_recording_rejects_a_smuggled_stream_entry() {
        let root = temp_root("binary_replay_smuggled_stream");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Packaged,
        );
        let repacked = repack_recording_with_manifest_mutation(&recording, |document| {
            document.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            );
            document.remove("stream");
            if let Some(entries) = document
                .get_mut("replay_entries")
                .and_then(Value::as_object_mut)
            {
                entries.remove("stream");
            }
        });
        fs::write(&recording, repacked).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("a declared cache-only recording carrying a stream must fail closed");
        let text = error.to_string();
        assert!(
            text.contains("omitted") && text.contains(snapshot::COMPACT_STREAM_ROLE),
            "the refusal must name the contradiction, got {text}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unassigned_composition_spelling_rejects_at_replay_admission() {
        let root = temp_root("binary_replay_bad_composition");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Packaged,
        );
        let repacked = repack_recording_with_manifest_mutation(&recording, |manifest| {
            manifest.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!("maybe"),
            );
        });
        fs::write(&recording, repacked).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("an unassigned composition spelling must reject");
        let text = error.to_string();
        assert!(
            text.contains("unassigned") && text.contains("maybe"),
            "the rejection must name the unassigned value, got {text}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn index_descriptor_composition_must_agree_with_the_recorder_authority() {
        let root = temp_root("binary_replay_index_composition");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Packaged,
        );
        let repacked = repack_recording_mutating_roles(&recording, &["index"], |index| {
            index.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            );
        });
        fs::write(&recording, repacked).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("an index descriptor that disagrees about the composition must reject");
        let text = error.to_string();
        assert!(
            text.contains("index descriptor") && text.contains("omitted"),
            "the refusal must name the disagreeing document, got {text}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    fn repack_recording_with_manifest_mutation(
        path: &Path,
        mutate: impl Fn(&mut serde_json::Map<String, Value>),
    ) -> Vec<u8> {
        repack_recording_mutating_roles(path, &["recorder_manifest", "timeline", "index"], mutate)
    }

    fn repack_recording_mutating_roles(
        path: &Path,
        roles: &[&str],
        mutate: impl Fn(&mut serde_json::Map<String, Value>),
    ) -> Vec<u8> {
        let container =
            snapshot::Container::open(path, &snapshot::ExpectedIdentity::default()).unwrap();
        let game = container.game_id().to_string();
        let content = portable_content_manifest_fixture();
        let mut entries = Vec::new();
        for entry in container.entries() {
            let mut bytes = container.read_entry_bytes(&entry.path).unwrap();
            if entry
                .role
                .as_deref()
                .is_some_and(|role| roles.contains(&role))
            {
                let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                mutate(value.as_object_mut().unwrap());
                bytes = serde_json::to_vec(&value).unwrap();
            }
            entries.push(snapshot::PackEntry {
                path: entry.path.clone(),
                bytes,
                role: Box::leak(
                    entry
                        .role
                        .clone()
                        .unwrap_or_default()
                        .into_boxed_str(),
                ),
                hour_index: None,
            });
        }
        snapshot::pack(
            &game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap()
    }

    #[test]
    fn packaged_binary_replay_rejects_an_incomplete_role_graph() {
        let root = temp_root("binary_replay_role");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture(&recording, false);

        let error = match open_recording(&recording) {
            Ok(_) => panic!("binary replay with a mismatched checkpoint role must fail closed"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(snapshot::CHECKPOINT_CACHE_ROLE));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    fn known_campaign_identity_value() -> Value {
        json!({
            "schema": "hoi4.campaign_identity.v1",
            "state": "known",
            "campaign_id": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
            "game_unique_id": "a4b17fed-ed12-4361-9064-fb6576638110",
            "game_unique_seed": "1026046527",
            "source": "live_memory",
            "source_build_profile": "hoi4-a729",
            "observed_game_hour_raw": 60759359,
            "stream_header_sha256": "cc".repeat(32),
            "resolution_note": null,
            "witnesses": {
                "start_date_raw": null,
                "session": null,
                "scenario": null,
                "initial_human_country_tags": null,
                "modset_identity": null
            }
        })
    }

    #[test]
    fn recorder_campaign_identity_is_recomputed_before_replay_exposure() {
        let mut root = Map::new();
        root.insert(
            "campaign_identity".to_string(),
            known_campaign_identity_value(),
        );
        let identity = recorder_campaign_identity(&root, "test manifest")
            .unwrap()
            .expect("known identity must be exposed");
        assert_eq!(identity.state, "known");
        assert_eq!(
            identity.game_unique_id.as_deref(),
            Some("a4b17fed-ed12-4361-9064-fb6576638110")
        );
        assert_eq!(identity.game_unique_seed.as_deref(), Some("1026046527"));
        assert_eq!(
            identity.campaign_id.as_deref(),
            Some("b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f")
        );
        assert_eq!(identity.source.as_deref(), Some("live_memory"));
        assert_eq!(identity.resolution_note, None);
        assert_eq!(identity.witnesses.session, None);

        root.get_mut("campaign_identity")
            .unwrap()["game_unique_seed"] = json!("1026046528");
        assert!(recorder_campaign_identity(&root, "tampered manifest").is_err());
    }

    #[test]
    fn validated_known_manifest_selects_identity_v1_without_stream_sniffing() {
        let mut root = Map::new();
        root.insert(
            "campaign_identity".to_string(),
            known_campaign_identity_value(),
        );
        let known = recorder_campaign_identity(&root, "known mode")
            .unwrap()
            .expect("known identity");
        assert_eq!(
            prefold_stream_identity_mode_from_campaign(Some(&known)),
            snapshot::PrefoldStreamIdentityMode::IdentityV1
        );
        assert_eq!(
            prefold_stream_identity_mode_from_campaign(None),
            snapshot::PrefoldStreamIdentityMode::Legacy
        );
    }

    #[test]
    fn recorder_campaign_identity_rejects_extra_or_positive_non_known_authority() {
        let mut value = known_campaign_identity_value();
        value["unexpected"] = json!(true);
        let mut root = Map::new();
        root.insert("campaign_identity".to_string(), value);
        assert!(recorder_campaign_identity(&root, "extra key").is_err());

        let mut unavailable = known_campaign_identity_value();
        unavailable["state"] = json!("unavailable");
        unavailable["resolution_note"] = json!("live reader unavailable");
        unavailable["campaign_id"] = Value::Null;
        unavailable["game_unique_id"] = Value::Null;
        unavailable["game_unique_seed"] = Value::Null;
        unavailable["source"] = Value::Null;
        unavailable["observed_game_hour_raw"] = Value::Null;
        unavailable["stream_header_sha256"] = Value::Null;
        let mut root = Map::new();
        root.insert("campaign_identity".to_string(), unavailable.clone());
        let unavailable_identity = recorder_campaign_identity(&root, "unavailable")
            .unwrap()
            .expect("non-known metadata must remain inspectable");
        assert_eq!(
            unavailable_identity.resolution_note.as_deref(),
            Some("live reader unavailable")
        );

        unavailable["game_unique_seed"] = json!("1");
        root.insert("campaign_identity".to_string(), unavailable);
        assert!(recorder_campaign_identity(&root, "fabricated unavailable").is_err());
    }

    #[test]
    fn packaged_snapshot_recording_binds_its_stream_entry_to_the_recorder_receipt() {
        let root = temp_root("snapshot_stream_binding");
        let recording = root.join("snapshot.battleplan");
        write_portable_recorder_fixture(&recording, "g", "SNAPSHOT", None);

        let stream_bytes = b"H4CRsnapshot-route-stream".to_vec();
        let container =
            snapshot::Container::open(&recording, &snapshot::ExpectedIdentity::default()).unwrap();
        let content = portable_content_manifest_fixture();
        let mut entries = Vec::new();
        for entry in container.entries() {
            let mut bytes = container.read_entry_bytes(&entry.path).unwrap();
            match entry.role.as_deref() {
                Some("recorder_manifest") => {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    let object = value.as_object_mut().unwrap();
                    object.insert(
                        snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                        json!(snapshot::RawStreamPackaging::Packaged.as_str()),
                    );
                    object.insert(
                        "replay_entries".to_string(),
                        json!({
                            "stream": "collector_raw/collector_stream.bin",
                            "stream_role": snapshot::COMPACT_STREAM_ROLE,
                        }),
                    );
                    object.insert(
                        "stream_storage".to_string(),
                        json!({
                            "decoded_bytes": stream_bytes.len() as u64 + 1,
                            "decoded_sha256": "a".repeat(64),
                        }),
                    );
                    bytes = serde_json::to_vec(&value).unwrap();
                }
                Some("timeline") | Some("index") => {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    let object = value.as_object_mut().unwrap();
                    object.insert(
                        snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                        json!(snapshot::RawStreamPackaging::Packaged.as_str()),
                    );
                    if entry.role.as_deref() == Some("timeline") {
                        object.insert(
                            "stream".to_string(),
                            json!("collector_raw/collector_stream.bin"),
                        );
                    }
                    bytes = serde_json::to_vec(&value).unwrap();
                }
                _ => {}
            }
            entries.push(snapshot::PackEntry {
                path: entry.path.clone(),
                bytes,
                role: Box::leak(entry.role.clone().unwrap_or_default().into_boxed_str()),
                hour_index: if entry.role.as_deref() == Some("snapshot") {
                    Some(1)
                } else {
                    None
                },
            });
        }
        entries.push(snapshot::PackEntry {
            path: "payload/g/collector_raw/collector_stream.bin".to_string(),
            bytes: stream_bytes,
            role: snapshot::COMPACT_STREAM_ROLE,
            hour_index: None,
        });
        drop(container);
        let packed = snapshot::pack(
            "g",
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(&recording, packed).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("a packaged snapshot recording must be bound to its stream entry");
        assert!(
            error.to_string().contains("stream_storage"),
            "the refusal must name the disagreeing authority, got {error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opened_recording_exposes_only_the_validated_recorder_campaign_identity() {
        let root = temp_root("campaign_identity_exposure");
        let recording = root.join("campaign.battleplan");
        write_portable_recorder_fixture(
            &recording,
            "g",
            "CAMPAIGN",
            Some(known_campaign_identity_value()),
        );

        let source = open_recording(&recording).unwrap();
        let identity = source
            .campaign_identity()
            .expect("valid recorder identity must survive source admission");
        assert_eq!(identity.state, "known");
        assert_eq!(identity.observed_game_hour_raw, Some(60759359));
        assert_eq!(
            identity.stream_header_sha256.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
        drop(source);
        let _ = fs::remove_dir_all(root);
    }

    fn prefolded_entry(hour: i64, schema: &str) -> snapshot::PrefoldedHourEntry {
        let bytes = serde_json::to_vec(&json!({
            "game_id": "prefolded-game",
            "schema_version": schema,
            "hour_index": hour,
            "date": "1936.1.1.12",
            "save_version": "a729",
            "countries": {"FRA": {"stability": 0.5}}
        }))
        .unwrap();
        snapshot::PrefoldedHourEntry::from_json(hour, &bytes).unwrap()
    }

    #[test]
    fn neutral_prefolded_hours_are_admitted_through_the_existing_frame_boundary() {
        let replay = snapshot::PrefoldedReplay::try_new(
            "prefolded-game",
            Value::String("1".into()),
            vec![prefolded_entry(10, "1"), prefolded_entry(12, "1")],
        )
        .unwrap();
        let source = open_prefolded_replay(replay).unwrap();

        assert_eq!(source.game_id(), "prefolded-game");
        assert_eq!(source.hours(), &[10, 12]);
        assert!(source.warnings().is_empty());
        let frame = source.frame(12).unwrap();
        assert_eq!(frame.hour_index, 12);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.12"));
        assert_eq!(frame.save_version.as_deref(), Some("a729"));
        assert_eq!(frame.schema_version, Some(Value::String("1".into())));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.5));
        assert!(source.frame(11).is_err());
    }

    #[test]
    fn prefolded_admission_rejects_an_unsupported_schema_before_exposing_a_source() {
        let replay = snapshot::PrefoldedReplay::try_new(
            "prefolded-game",
            Value::String("2".into()),
            vec![prefolded_entry(10, "2")],
        )
        .unwrap();
        assert!(matches!(
            open_prefolded_replay(replay),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn raw_h4cr_compact_package_remains_fail_closed_at_the_file_boundary() {
        let root = temp_root("raw_h4cr_stays_closed");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
        )
        .unwrap();
        fs::write(
            game.join("timeline.json"),
            br#"{"format":"hoi4_compact_stream_timeline_v1","raw_stream_role":"collector_compact_stream"}"#,
        )
        .unwrap();
        fs::write(game.join("collector_stream.bin"), b"H4CR").unwrap();

        let error = match open_recording(&root) {
            Err(error) => error,
            Ok(_) => panic!("raw H4CR package must not enter replay_core directly"),
        };
        assert!(matches!(
            error,
            SourceError::UnsupportedCompactStreamOnly(_)
        ));
        let message = error.to_string();
        assert!(message.starts_with("unsupported recording: "), "{message}");
        assert!(message.contains("compact-stream-only"), "{message}");
        assert!(message.contains("collector_compact_stream"), "{message}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_index_without_compact_evidence_stays_plain_unsupported() {
        let root = temp_root("empty_index_no_evidence");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
        )
        .unwrap();
        let error = match open_recording(&root) {
            Err(error) => error,
            Ok(_) => panic!("an empty snapshot index must not open"),
        };
        assert!(matches!(error, SourceError::Unsupported(_)));
        assert!(error.to_string().contains("empty snapshot index"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn standalone_replay_manifest_has_no_extractor_dependency() {
        let manifest = include_str!("../Cargo.toml").replace('\\', "/");
        assert!(!manifest.contains("extractor/"), "{manifest}");
    }

    #[test]
    fn root_swap_full_lifecycle_stays_in_anchored_graph() {
        let parent = temp_root("root_swap");
        let recording = parent.join("store.battleplan");
        write_portable_recorder_fixture(&recording, "g", "ANCHORED", None);
        let moved = parent.join("anchored_object.battleplan");
        let source = open_recording_impl_with_post_anchor(&recording, None, None, None, |_| {
            #[cfg(windows)]
            {
                fs::rename(&recording, &moved)
                    .expect_err("the retained container handle must deny pathname replacement");
                assert!(recording.is_file());
                assert!(!moved.exists());
            }
            #[cfg(not(windows))]
            {
                fs::rename(&recording, &moved).unwrap();
                write_portable_recorder_fixture(&recording, "g", "IMPOSTOR", None);
            }
        })
        .unwrap();

        let frame = source.frame(1).unwrap();
        assert_eq!(frame.rest["marker"], "ANCHORED");
        assert_eq!(frame.rest["session"]["active_dlcs"], json!([]));
        assert_eq!(frame.rest["session"]["localisation_language"], "english");
        assert_eq!(
            frame.rest["session"]["display_variant"]["physical_width"],
            1920
        );
        drop(source);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn empty_index_evidence_after_root_swap_uses_anchored_timeline() {
        let parent = temp_root("empty_root_swap");
        let root = parent.join("store");
        let game = "g";
        let game_dir = root.join(game);
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(
            game_dir.join("index.json"),
            serde_json::to_vec(&json!({"game_id": game, "snapshots": []})).unwrap(),
        )
        .unwrap();
        fs::write(
            game_dir.join("timeline.json"),
            br#"{"format":"hoi4_compact_stream_timeline_v1"}"#,
        )
        .unwrap();
        let moved = parent.join("anchored_object");
        let result = open_recording_impl_with_post_anchor(&root, None, None, None, |_| {
            fs::rename(&root, &moved).unwrap();
            let impostor = root.join(game);
            fs::create_dir_all(&impostor).unwrap();
            fs::write(
                impostor.join("index.json"),
                serde_json::to_vec(&json!({"game_id": game, "snapshots": []})).unwrap(),
            )
            .unwrap();
            fs::write(impostor.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("empty index must be unsupported"),
        };
        let message = error.to_string();
        assert!(
            message.contains("hoi4_compact_stream_timeline_v1"),
            "{message}"
        );
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn directory_route_reads_each_metadata_object_once_and_uses_first_buffer() {
        let mut calls = BTreeMap::<String, usize>::new();
        let route = directory_recorder_metadata_route_with_reader(|relative| {
            let count = calls.entry(relative.to_string()).or_default();
            *count += 1;
            if *count > 1 {
                return Ok(Some(
                    br#"{"legacy":true,"second_read_must_not_bind":true}"#.to_vec(),
                ));
            }
            let bytes = match relative {
                "timeline.json" => {
                    br#"{"session_manifest":"recorder/native_session_manifest.json"}"#.as_slice()
                }
                "recorder/native_session_manifest.json" => br#"{"modset":{"resolved":true,"errors":[],"dlc":{"enabled":["dlc/a.dlc"]},"dlc_display_names":{"dlc/a.dlc":"DLC A"},"settings_binding":{"authority":"pre_launch_bracketed","launch_proven":true,"errors":[],"language":"l_english","display_variant":{"physical_width":2752,"physical_height":1152,"min_gui_width":1280,"min_gui_height":720,"gui_scale_f32_bits":1065353216,"refresh_rate_hz":60,"max_refresh_rate_hz":75,"fullscreen":false,"borderless":true,"display_mode":"borderless_fullscreen"},"attach_settings_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","seal_settings_sha256":null,"endpoint_semantics_equal":null}}}"#,
                other => panic!("unexpected metadata read {other:?}"),
            };
            Ok(Some(bytes.to_vec()))
        })
        .unwrap();

        let binding = route.binding.expect("first buffers must produce a binding");
        assert_eq!(binding.active_dlcs, vec!["DLC A"]);
        assert_eq!(binding.localisation_language, "l_english");
        assert_eq!(binding.display_variant.physical_width, 2752);
        assert_eq!(
            calls.get("timeline.json").copied(),
            Some(1),
            "timeline final object must be opened/read exactly once"
        );
        assert_eq!(
            calls.get("recorder/native_session_manifest.json").copied(),
            Some(1),
            "recorder-manifest final object must be opened/read exactly once"
        );
    }

    #[test]
    fn selected_game_directory_swap_before_frame_is_rejected_without_cross_object_mix() {
        let parent = temp_root("selected_game_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        write_playable_store(&root, "g", false, "OBJECT_A");
        let source = open_recording(&root).unwrap();

        let original = root.join("g");
        let moved = root.join("game_object_a");
        fs::rename(&original, &moved).unwrap();
        write_playable_store(&root, "g", false, "OBJECT_B");

        let error = source
            .frame(1)
            .expect_err("a selected-game object swap must fail closed")
            .to_string();
        assert!(
            error.contains("identity") || error.contains("namespace"),
            "{error}"
        );
        drop(source);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn timeline_to_manifest_game_parent_swap_is_rejected() {
        let parent = temp_root("timeline_manifest_parent_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        write_playable_store(&root, "g", true, "OBJECT_A");
        let store = Store::open_directory(&root).unwrap();
        let mut swapped = false;

        let result = directory_recorder_metadata_route_with_reader(|relative| {
            let bytes = store
                .read_game_file_bytes("g", relative)
                .map_err(|error| SourceError::Open(error.to_string()))?;
            if relative == "timeline.json" && !swapped {
                fs::rename(root.join("g"), root.join("game_object_a")).unwrap();
                write_playable_store(&root, "g", true, "OBJECT_B");
                swapped = true;
            }
            Ok(bytes)
        });

        let error = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("timeline and referenced manifest must share one game parent object"),
        };
        assert!(
            error.contains("identity") || error.contains("namespace"),
            "{error}"
        );
        drop(store);
        let _ = fs::remove_dir_all(parent);
    }

    #[derive(Default)]
    struct FilesystemAliases {
        modules: Vec<String>,
        files: Vec<String>,
        open_options: Vec<String>,
    }

    fn collect_aliases(compact: &str, markers: &[&str]) -> Vec<String> {
        let mut aliases = Vec::new();
        for marker in markers {
            let mut cursor = 0;
            while let Some(relative) = compact[cursor..].find(marker) {
                let start = cursor + relative + marker.len();
                let len = compact[start..]
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                if len == 0 {
                    cursor = start;
                    continue;
                }
                let alias = &compact[start..start + len];
                if !aliases.iter().any(|existing| existing == alias) {
                    aliases.push(alias.to_string());
                }
                cursor = start + len;
            }
        }
        aliases
    }

    fn filesystem_aliases(source: &str) -> FilesystemAliases {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        FilesystemAliases {
            modules: collect_aliases(
                &compact,
                &["usestd::fsas", "usestd::{fsas", "usestd::fs::{selfas"],
            ),
            files: collect_aliases(&compact, &["usestd::fs::Fileas", "usestd::fs::{Fileas"]),
            open_options: collect_aliases(
                &compact,
                &["usestd::fs::OpenOptionsas", "usestd::fs::{OpenOptionsas"],
            ),
        }
    }

    fn source_references_file_access(source: &str, aliases: &FilesystemAliases) -> bool {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        [
            "std::fs",
            "std::{fs",
            "File::",
            "OpenOptions",
            ".open(",
            "windows_sys::Win32::Storage::FileSystem",
            "windows_sys::Wdk::Storage::FileSystem",
            "::Storage::FileSystem::",
            "CreateFile(",
            "CreateFile2",
            "CreateFileA",
            "CreateFileW",
            "NtCreateFile",
            "ZwCreateFile",
            "NtOpenFile",
            "ZwOpenFile",
            "OpenFile(",
            "OpenFileById",
            "ReOpenFile",
        ]
        .iter()
        .any(|token| compact.contains(token))
            || aliases
                .modules
                .iter()
                .any(|alias| compact.contains(&format!("{alias}::")))
            || aliases
                .files
                .iter()
                .any(|alias| compact.contains(&format!("{alias}::")))
            || aliases
                .open_options
                .iter()
                .any(|alias| compact.contains(alias))
    }

    fn pathbuf_value_names(source: &str) -> Vec<String> {
        let mut names = Vec::new();
        for line in source.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("let ") else {
                continue;
            };
            let rest = rest.strip_prefix("mut ").unwrap_or(rest);
            let Some((name, value)) = rest.split_once('=') else {
                continue;
            };
            let name = name.trim().split(':').next().unwrap_or("").trim();
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && (value.contains("PathBuf::")
                    || value.contains(".join(")
                    || value.contains(".to_path_buf("))
                && !names.iter().any(|existing| existing == name)
            {
                names.push(name.to_string());
            }
        }
        names
    }

    fn file_capability_import_lines(source: &str) -> Vec<bool> {
        let mut import_lines = vec![false; source.lines().count()];
        let mut in_import = false;
        for (index, line) in source.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if !in_import
                && (compact.contains("std::fs::{")
                    || compact.contains("std::{fs::{")
                    || (compact.contains("windows_sys::")
                        && compact.contains("::Storage::FileSystem::{")))
            {
                in_import = true;
            }
            if in_import {
                import_lines[index] = true;
                if compact.contains("};") {
                    in_import = false;
                }
            }
        }
        import_lines
    }

    const WINDOWS_OPEN_PRIMITIVES: &[&str] = &[
        "CreateFile",
        "CreateFile2",
        "CreateFileA",
        "CreateFileW",
        "CreateFile2FromAppW",
        "CreateFileFromAppW",
        "CreateFileTransactedA",
        "CreateFileTransactedW",
        "NtCreateFile",
        "ZwCreateFile",
        "NtOpenFile",
        "ZwOpenFile",
        "OpenFile",
        "OpenFileById",
        "ReOpenFile",
    ];

    fn windows_open_primitive_symbol(symbol: &str) -> bool {
        WINDOWS_OPEN_PRIMITIVES.contains(&symbol)
    }

    fn windows_open_primitive_candidate(line: &str) -> bool {
        WINDOWS_OPEN_PRIMITIVES.iter().any(|primitive| {
            line.match_indices(primitive).any(|(start, matched)| {
                let end = start + matched.len();
                let identifier_boundary = |byte: u8| !byte.is_ascii_alphanumeric() && byte != b'_';
                let starts_at_boundary = start == 0
                    || line
                        .as_bytes()
                        .get(start - 1)
                        .copied()
                        .is_some_and(identifier_boundary);
                let ends_at_boundary = end == line.len()
                    || line
                        .as_bytes()
                        .get(end)
                        .copied()
                        .is_some_and(identifier_boundary);
                if !starts_at_boundary || !ends_at_boundary {
                    return false;
                }
                let suffix = line[end..].trim_start();
                suffix.is_empty()
                    || suffix.starts_with('(')
                    || suffix.starts_with(',')
                    || suffix.starts_with(';')
                    || suffix.starts_with(')')
                    || suffix.starts_with('}')
                    || suffix.starts_with("as ")
            })
        })
    }

    fn link_name_open_primitive_candidate(line: &str) -> bool {
        let compact = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let quote = char::from(34);
        let prefix = format!("#[link_name={quote}");
        let Some(attribute) = compact
            .find(&prefix)
            .map(|start| &compact[start + prefix.len()..])
        else {
            return false;
        };
        let Some(end) = attribute.find('"') else {
            return false;
        };
        windows_open_primitive_symbol(&attribute[..end])
    }

    fn ffi_open_declaration_candidate(line: &str) -> bool {
        link_name_open_primitive_candidate(line)
            || (line.contains("extern") && windows_open_primitive_candidate(line))
    }

    fn loader_pathname_candidate(
        line: &str,
        aliases: &FilesystemAliases,
        file_access_capable: bool,
        pathbuf_values: &[String],
    ) -> bool {
        let direct_file_access = [
            "std::fs",
            "std::{fs",
            "fs::",
            "File::",
            "OpenOptions",
            ".open(",
            ".is_file(",
            ".is_dir(",
            ".try_exists(",
            "windows_sys::Win32::Storage::FileSystem",
            "windows_sys::Wdk::Storage::FileSystem",
            "::Storage::FileSystem::",
        ]
        .iter()
        .any(|token| line.contains(token));
        let path_construction = file_access_capable
            && (["std::path", "Path::", "PathBuf::", ".join("]
                .iter()
                .any(|token| line.contains(token))
                || pathbuf_values
                    .iter()
                    .any(|name| line.contains(&format!("{name}.push("))));
        direct_file_access
            || windows_open_primitive_candidate(line)
            || ffi_open_declaration_candidate(line)
            || path_construction
            || aliases
                .modules
                .iter()
                .any(|alias| line.contains(&format!("{alias}::")))
            || aliases
                .files
                .iter()
                .any(|alias| line.contains(&format!("{alias}::open")))
            || aliases
                .open_options
                .iter()
                .any(|alias| line.contains(alias))
    }

    fn discover_rust_sources(label: &str, root: &Path) -> Result<Vec<(String, String)>, String> {
        fn walk(
            label: &str,
            root: &Path,
            directory: &Path,
            sources: &mut Vec<(String, String)>,
        ) -> Result<(), String> {
            let mut entries = fs::read_dir(directory)
                .map_err(|error| format!("discover {}: {error}", directory.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("enumerate {}: {error}", directory.display()))?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                let file_type = entry.file_type().map_err(|error| {
                    format!("classify discovered {}: {error}", entry.path().display())
                })?;
                if file_type.is_symlink() {
                    return Err(format!(
                        "source discovery refuses symlink {}",
                        entry.path().display()
                    ));
                }
                if file_type.is_dir() {
                    walk(label, root, &entry.path(), sources)?;
                    continue;
                }
                if !file_type.is_file() {
                    return Err(format!(
                        "source discovery found unsupported object {}",
                        entry.path().display()
                    ));
                }
                if entry.path().extension().and_then(|value| value.to_str()) != Some("rs") {
                    continue;
                }
                let path = entry.path();
                let relative = path.strip_prefix(root).map_err(|_| {
                    format!(
                        "discovered source {} escaped root {}",
                        path.display(),
                        root.display()
                    )
                })?;
                let relative = relative
                    .to_str()
                    .ok_or_else(|| format!("source path is not Unicode: {}", path.display()))?
                    .replace(char::from(92), "/");
                let source = fs::read_to_string(&path).map_err(|error| {
                    format!("read discovered source {}: {error}", path.display())
                })?;
                sources.push((format!("{label}/{relative}"), source));
            }
            Ok(())
        }

        let root = fs::canonicalize(root)
            .map_err(|error| format!("canonicalize source root {}: {error}", root.display()))?;
        if !root.is_dir() {
            return Err(format!(
                "source root is not a directory: {}",
                root.display()
            ));
        }
        let mut sources = Vec::new();
        walk(label, &root, &root, &mut sources)?;
        if sources.is_empty() {
            return Err(format!(
                "source discovery found no Rust files under {label}"
            ));
        }
        Ok(sources)
    }

    fn classified_root_or_handle_site(module: &str, line: &str) -> bool {
        matches!(
            (module, line),
            ("replay_core/source.rs", "games.join(\", \")")
                | (
                    "replay_core/source.rs",
                    "DECLARED_COVERAGE_KINDS.join(\", \")"
                )
                | (
                    "replay_core/source.rs",
                    "let metadata = match fs::metadata(path) {"
                )
                | (
                    "replay_core/source.rs",
                    "let store = if metadata.is_file() {"
                )
                | ("replay_core/source.rs", "} else if metadata.is_dir() {")
                | ("snapshot/lib.rs", "if path.is_file() {")
                | (
                    "snapshot/container.rs",
                    "let data = std::fs::read(path.as_ref())?;"
                )
                | ("snapshot/dir_store/linux.rs", "use std::fs::File;")
                | ("snapshot/dir_store/linux.rs", "use std::path::Path;")
                | (
                    "snapshot/dir_store/linux.rs",
                    "let mut file = File::from(opened.fd);"
                )
                | ("snapshot/dir_store/windows.rs", "use std::fs::File;")
                | ("snapshot/dir_store/windows.rs", "use std::path::Path;")
                | (
                    "snapshot/dir_store/windows.rs",
                    "use windows_sys::Wdk::Storage::FileSystem::{"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "use windows_sys::Win32::Storage::FileSystem::{"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,"
                )
                | ("snapshot/dir_store/windows.rs", "};")
                | (
                    "snapshot/dir_store/windows.rs",
                    "CreateFileW, FileAttributeTagInfo, FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FileIdInfo, FileNormalizedNameInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_GENERIC_READ, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_NAME_INFO, FILE_SHARE_DELETE,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,"
                )
                | ("snapshot/dir_store/windows.rs", "CreateFileW(")
                | (
                    "snapshot/dir_store/windows.rs",
                    "let mut file = File::from(opened.handle);"
                )
                | ("snapshot/dir_store/windows.rs", "NtCreateFile(")
                | ("snapshot/stream_verify.rs", "use std::fs::File;")
                | ("snapshot/stream_verify.rs", "use std::path::Path;")
                | (
                    "snapshot/stream_verify.rs",
                    "File::open(path).map_err(|e| format!(\"open container {}: {e}\", path.display()))?;"
                )
        )
    }

    /// failure until its ownership and handle-identity contract is reviewed.
    fn classified_snapshot_container_retained_io_site(module: &str, line: &str) -> bool {
        module == "snapshot/container.rs"
            && matches!(
                line,
                "use std::fs::{self, File, OpenOptions};"
                    | "use std::path::{Path, PathBuf};"
                    | "let joined = components.join(\"/\");"
                    | "use std::os::windows::fs::{MetadataExt, OpenOptionsExt};"
                    | "use std::path::{Component, Prefix};"
                    | "std::env::current_dir()?.join(path)"
                    | "let mut absolute = PathBuf::from(&prefix);"
                    | "absolute.push(\"\\\\\");"
                    | "absolute.push(name);"
                    | "let mut cursor = PathBuf::from(prefix);"
                    | "cursor.push(\"\\\\\");"
                    | "cursor.push(component.as_os_str());"
                    | "let directory = OpenOptions::new()"
                    | ".open(&cursor)?;"
                    | "if !metadata.is_dir()"
                    | "let file = OpenOptions::new()"
                    | ".open(&absolute)?;"
                    | "if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {"
                    | "File::open(path)"
                    // Authenticated transactional output-spool ownership and
                    | "use std::path::Component;"
                    | "std::env::current_dir()?.join(destination)"
                    | "let mut current = PathBuf::new();"
                    | "current.push(component.as_os_str());"
                    | "let metadata = fs::symlink_metadata(&current)?;"
                    | "if metadata.file_type().is_symlink() || !metadata.is_dir() {"
                    | "use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};"
                    | "let handle = OpenOptions::new()"
                    | ".open(&current)?;"
                    | "if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {"
                    // The debloat reformat moved the opening brace of the
                    // ancestor no-follow check onto its own line; the
                    // brace-free spelling is the same guard.
                    | "if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0"
                    | "let temp = guard.parent.join(temp_name);"
                    | "use std::os::windows::fs::OpenOptionsExt;"
                    | "OpenOptions::new()"
                    | ".open(path)"
                    | "use std::os::unix::fs::MetadataExt;"
                    | "if !metadata.is_file() {"
                    | "fs::hard_link(&temp, &destination).map_err(|error| {"
                    | "match fs::remove_file(&destination) {"
                    | "let _ = fs::remove_file(&temp);"
                    | "match fs::remove_file(path) {"
            )
    }

    fn classified_rollover_descriptor_discovery_site(module: &str, line: &str) -> bool {
        module == "replay_core/rollover_package_set.rs"
            && matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Component, Path, PathBuf};"
                    | "let descriptor_path = if path.is_dir() {"
                    | "let metadata = fs::symlink_metadata(path).map_err(|error| {"
                    | "if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {"
                    | "path.join(ROLLOVER_PACKAGE_SET_FILE)"
                    | ".unwrap_or_else(|| Path::new(\".\"));"
                    | "let parent_metadata = fs::symlink_metadata(descriptor_parent).map_err(|error| {"
                    | "if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {"
                    | "if source_descriptor != Path::new(ROLLOVER_SOURCE_SET_FILE) {"
                    | "|| Path::new(&source_stream.relative_path) != expected_relative"
                    | "let discovered_descriptor = path.is_dir() && path.join(ROLLOVER_PACKAGE_SET_FILE).is_file();"
                    | "let path = PathBuf::from(value);"
                    | "let metadata = fs::symlink_metadata(path)"
                    | "if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {"
                    | "let file = fs::File::open(path)"
            )
    }

    fn classified_rollover_retained_handle_site(module: &str, line: &str) -> bool {
        module == "replay_core/rollover_package_set.rs"
            && matches!(
                line,
                "let root = fs::canonicalize(root)"
                    | "let candidate = root.join(relative);"
                    | "let metadata = fs::symlink_metadata(&candidate)"
                    | "if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {"
                    | "let canonical = fs::canonicalize(&candidate)"
                    | "let mut file = fs::File::open(&canonical).map_err(|error| {"
                    | "let file = fs::File::open(path).ok()?;"
            )
    }

    fn classified_non_loader_site(module: &str, line: &str) -> bool {
        match module {
            "replay_core/world_xmap19_tests.rs" => matches!(
                line,
                "let path = std::path::PathBuf::from(env!(\"CARGO_MANIFEST_DIR\"))"
                    | ".join(\"../../fixtures\")"
                    | ".join(\"xmap19_weather_visual_lifecycle_joint_certification.json\");"
                    | "let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {"
            ),
            "snapshot/bin/pack_replay.rs" => matches!(
                line,
                "use std::fs::{self, OpenOptions};"
                    | "use std::io::{Read, Seek, SeekFrom, Write};"
                    | "use std::path::{Path, PathBuf};"
                    | "(Some(a), Some(b), Some(c)) => (PathBuf::from(a), b.clone(), PathBuf::from(c)),"
                    | "let gdir = root.join(&game);"
                    | "let index_path = gdir.join(\"index.json\");"
                    | "let index_bytes = match fs::read(&index_path) {"
                    | "let p = gdir.join(file);"
                    | "let bytes = match fs::read(&p) {"
                    | "let tpath = gdir.join(\"timeline.json\");"
                    | "if let Ok(tb) = fs::read(&tpath) {"
                    | "fs::create_dir_all(parent)"
                    // Debloat-reformat spelling of the same output-directory
                    // creation, wrapped with its map_err on one line.
                    | "fs::create_dir_all(parent).map_err(|error| format!(\"create output directory: {error}\"))?;"
                    | "let canonical_parent = fs::canonicalize(parent.unwrap_or_else(|| Path::new(\".\")))"
                    | "let anchored_out = canonical_parent.join(file_name);"
                    | "let published = fs::read(&anchored_out)"
                    // The debloat reformat wrapped the published-container
                    // re-read (the packer's byte-identity check on its own
                    // just-written output) onto a continuation line.
                    | "fs::read(&anchored_out).map_err(|error| format!(\"read published container: {error}\"))?;"
                    | "fn create_unique_temp(out: &Path) -> Result<(PathBuf, fs::File), String> {"
                    | "let parent = out.parent().unwrap_or_else(|| Path::new(\".\"));"
                    | "let temp = parent.join(format!(\".{name}.pack-tmp-{nonce}.{attempt}\"));"
                    | "match OpenOptions::new()"
                    | ".open(&temp)"
                    | "match fs::symlink_metadata(out) {"
                    | "use std::os::windows::fs::MetadataExt;"
                    | "let cleanup = fs::remove_file(&temp);"
                    | "fs::File::open(out.parent().unwrap_or_else(|| Path::new(\".\")))?.sync_all()"
                    | "fs::rename(temp, out)"
                    | "let gdir = Path::new(root).join(game);"
                    | "let orig = gdir.join(&s.file);"
                    | "let orig_bytes = match fs::read(&orig) {"
                    | "Some(p) => PathBuf::from(p),"
            ),
            "snapshot/pack.rs" => matches!(
                line,
                "use std::fs;"
                    | "use std::path::Path;"
                    | "return Ok(fs::metadata(entry.file_path)?.len());"
                    | "let mut file = fs::File::open(path)?;"
            ),
            "replay_core/bin/make_demo_recording.rs" => matches!(
                line,
                "use std::path::{Path, PathBuf};"
                    | "bundle_dir = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));"
                    | "out_path = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));"
                    | "std::fs::write(&out_path, packed).map_err(|e| format!(\"write {}: {e}\", out_path.display()))?;"
                    | "let path = bundle_dir.join(\"state_owner_1936.json\");"
                    | "std::fs::read_to_string(&path).map_err(|e| format!(\"read {}: {e}\", path.display()))?;"
                    | "let path = bundle_dir.join(\"province_to_state.json\");"
            ),
            "replay_core/bin/replay_dump.rs" => matches!(
                line,
                "coverage.domains.join(\", \")" | ".join(\",\")" | ".join(\", \")"
            ),
            "replay_core/world.rs" => matches!(
                line,
                "path[..=i].join(\".\"),"
                    | "path.join(\".\"),"
                    | "let ectx = format!(\"{ctx}.{}[{i}]\", path.join(\".\"));"
                    | "let rctx = format!(\"{ctx}.{}[{i}]\", path.join(\".\"));"
                    | "path.join(\".\")"
            ),
            "replay_core/lib.rs" => matches!(
                line,
                "pub use static_focus_defs::{"
            ),
            _ => false,
        }
    }

    /// exactly that fact, and the module cannot acquire a descendant-pathname
    fn classified_certified_cache_handle_site(module: &str, line: &str) -> bool {
        module == "snapshot/replay_cache.rs"
            && matches!(
                line,
                "use std::fs::File;" | "let cache = CertifiedCacheFile::new(cache)?;"
            )
    }

    fn classified_private_prefold_spool_site(module: &str, line: &str) -> bool {
        module == "replay_core/source.rs"
            && matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Path, PathBuf};"
                    | "let raw_path = owner.root().join(\"source.h4cr\");"
                    | "let spooled = owner.root().join(\"checkpoints.h4cp\");"
                    | "let mut sink = fs::OpenOptions::new()"
                    | ".open(&spooled)"
                    | "fn open_private_prefold_spool_file(spool_path: &Path) -> Result<fs::File, SourceError> {"
                    | "let spool_file = fs::OpenOptions::new()"
                    | ".open(spool_path)"
                    | "let file = fs::OpenOptions::new()"
                    | ".open(&raw_path)"
                    | "let meta = fs::symlink_metadata(base).map_err(|error| {"
                    | "if !meta.file_type().is_dir() || meta.file_type().is_symlink() {"
                    | "let canonical_base = fs::canonicalize(base).map_err(|error| {"
                    | "let root = canonical_base.join(format!("
                    | "match fs::create_dir(&root) {"
                    | "let canonical_root = match fs::canonicalize(&root) {"
                    | "let _ = fs::remove_dir(&root);"
                    | "let _ = fs::remove_dir_all(&root);"
                    | "let marker_path = canonical_root.join(PREFOLD_SPOOL_MARKER);"
                    | "let mut marker = fs::OpenOptions::new()"
                    | ".open(&marker_path)"
                    | "let snapshot_dir = canonical_root.join(\"snapshots\");"
                    | "fs::create_dir(&snapshot_dir).map_err(|error| {"
                    | "if fs::canonicalize(&root).ok().as_deref()"
                    | "let Ok(meta) = fs::symlink_metadata(&self.root) else {"
                    | "if fs::canonicalize(&self.root).ok().as_deref() != Some(self.canonical_root.as_path()) {"
                    | "let Ok(snapshot_meta) = fs::symlink_metadata(&self.snapshot_dir) else {"
                    | "if !snapshot_meta.file_type().is_dir() || snapshot_meta.file_type().is_symlink() {"
                    | "if fs::canonicalize(&self.snapshot_dir).ok().as_deref() != Some(self.snapshot_dir.as_path())"
                    | "let marker = self.root.join(PREFOLD_SPOOL_MARKER);"
                    | "let Ok(marker_meta) = fs::symlink_metadata(&marker) else {"
                    | "if !marker_meta.file_type().is_file() || marker_meta.file_type().is_symlink() {"
                    | "if fs::read_to_string(marker).ok().as_deref() != Some(self.marker_value.as_str()) {"
                    | "let Ok(root_entries) = fs::read_dir(&self.root) else {"
                    | "let Ok(meta) = fs::symlink_metadata(entry.path()) else {"
                    | "if !meta.file_type().is_file() || meta.file_type().is_symlink() {"
                    | "let Ok(canonical) = fs::canonicalize(entry.path()) else {"
                    | "let Ok(snapshot_entries) = fs::read_dir(&self.snapshot_dir) else {"
                    | "let path = self.root.join(\"source.h4cr\");"
                    | "let meta = fs::symlink_metadata(&path).map_err(|error| {"
                    | "let canonical = fs::canonicalize(&path).map_err(|error| {"
                    | "if !meta.file_type().is_file()"
                    | "fs::remove_file(&path).map_err(|error| {"
                    | "let _ = fs::remove_dir_all(&self.root);"
                    | "if fs::canonicalize(snapshot_dir).ok().as_deref() != Some(owner.snapshot_dir()) {"
                    | "let path = snapshot_dir.join(entry.file_name());"
                    | "let path_meta = fs::symlink_metadata(&path).map_err(|error| {"
                    | "if !path_meta.file_type().is_file()"
                    | "let file = fs::File::open(&canonical).map_err(|error| {"
            )
    }

    fn cfg_test_item_end(source: &str, mut cursor: usize) -> Result<usize, String> {
        #[derive(Clone, Copy)]
        enum State {
            Code,
            LineComment,
            BlockComment(usize),
            String(bool),
            RawString(usize),
        }

        fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
            let mut cursor = match (bytes.get(index), bytes.get(index + 1)) {
                (Some(b'r'), _) => index + 1,
                (Some(b'b'), Some(b'r')) => index + 2,
                _ => return None,
            };
            let mut hashes = 0;
            while bytes.get(cursor) == Some(&b'#') {
                hashes += 1;
                cursor += 1;
            }
            (bytes.get(cursor) == Some(&b'"')).then_some((cursor + 1, hashes))
        }

        let bytes = source.as_bytes();
        let mut state = State::Code;
        let mut depth = 0usize;
        let mut opened = false;
        while cursor < bytes.len() {
            match state {
                State::Code => {
                    if bytes[cursor..].starts_with(b"//") {
                        state = State::LineComment;
                        cursor += 2;
                        continue;
                    }
                    if bytes[cursor..].starts_with(b"/*") {
                        state = State::BlockComment(1);
                        cursor += 2;
                        continue;
                    }
                    if let Some((next, hashes)) = raw_string_start(bytes, cursor) {
                        state = State::RawString(hashes);
                        cursor = next;
                        continue;
                    }
                    if bytes[cursor] == b'"' {
                        state = State::String(false);
                        cursor += 1;
                        continue;
                    }
                    if bytes[cursor] == b'\'' {
                        let mut end = cursor + 1;
                        if bytes.get(end) == Some(&b'\\') {
                            end += 2;
                        } else if let Some(character) = source[end..].chars().next() {
                            end += character.len_utf8();
                        }
                        if bytes.get(end) == Some(&b'\'') {
                            cursor = end + 1;
                            continue;
                        }
                    }
                    match bytes[cursor] {
                        b'{' => {
                            opened = true;
                            depth += 1;
                        }
                        b'}' if opened => {
                            depth = depth.checked_sub(1).ok_or_else(|| {
                                "cfg(test) item brace depth underflow".to_string()
                            })?;
                            if depth == 0 {
                                return Ok(cursor + 1);
                            }
                        }
                        b';' if !opened => return Ok(cursor + 1),
                        _ => {}
                    }
                    cursor += 1;
                }
                State::LineComment => {
                    if bytes[cursor] == b'\n' {
                        state = State::Code;
                    }
                    cursor += 1;
                }
                State::BlockComment(comment_depth) => {
                    if bytes[cursor..].starts_with(b"/*") {
                        state = State::BlockComment(comment_depth + 1);
                        cursor += 2;
                    } else if bytes[cursor..].starts_with(b"*/") {
                        if comment_depth == 1 {
                            state = State::Code;
                        } else {
                            state = State::BlockComment(comment_depth - 1);
                        }
                        cursor += 2;
                    } else {
                        cursor += 1;
                    }
                }
                State::String(escaped) => {
                    if escaped {
                        state = State::String(false);
                    } else if bytes[cursor] == b'\\' {
                        state = State::String(true);
                    } else if bytes[cursor] == b'"' {
                        state = State::Code;
                    }
                    cursor += 1;
                }
                State::RawString(hashes) => {
                    if bytes[cursor] == b'"'
                        && bytes
                            .get(cursor + 1..cursor + 1 + hashes)
                            .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                    {
                        state = State::Code;
                        cursor += 1 + hashes;
                    } else {
                        cursor += 1;
                    }
                }
            }
        }
        Err("cfg(test) item did not have a complete body".into())
    }

    fn mask_cfg_test_items(source: &str) -> Result<String, String> {
        const ATTRIBUTE: &str = "#[cfg(test)]";
        let mut masked = source.as_bytes().to_vec();
        let mut cursor = 0;
        while let Some(relative) = source[cursor..].find(ATTRIBUTE) {
            let start = cursor + relative;
            let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
            let line_end = source[start..]
                .find('\n')
                .map_or(source.len(), |index| start + index);
            if source[line_start..line_end].trim() != ATTRIBUTE {
                cursor = start + ATTRIBUTE.len();
                continue;
            }
            let end = cfg_test_item_end(source, start + ATTRIBUTE.len())?;
            for byte in &mut masked[start..end] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            cursor = end;
        }
        String::from_utf8(masked).map_err(|_| "masked Rust source stopped being UTF-8".into())
    }

    /// This is a development-time, best-effort honest-mistake lint, not a runtime
    /// security boundary. It cheaply recognizes direct APIs and explicit
    /// link_name FFI declarations. Renamed FFI beyond that check,
    /// macro/proc-macro expansion, and generated/include! code remain known,
    /// ACCEPTED blind spots; the runtime boundary is the handle-relative
    /// loader implementation and its identity/namespace checks.
    fn loader_pathname_audit(module: &str, source: &str) -> Result<(usize, Vec<String>), String> {
        let source = mask_cfg_test_items(source)?;
        let aliases = filesystem_aliases(&source);
        let file_access_capable = source_references_file_access(&source, &aliases);
        let pathbuf_values = pathbuf_value_names(&source);
        let import_lines = file_capability_import_lines(&source);
        let mut classified = 0;
        let mut candidates = 0;
        let mut violations = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("//")
                || (!import_lines[index]
                    && !loader_pathname_candidate(
                        line,
                        &aliases,
                        file_access_capable,
                        &pathbuf_values,
                    ))
            {
                continue;
            }
            candidates += 1;
            if classified_root_or_handle_site(module, line)
                || classified_snapshot_container_retained_io_site(module, line)
                || classified_rollover_descriptor_discovery_site(module, line)
                || classified_rollover_retained_handle_site(module, line)
                || classified_non_loader_site(module, line)
                || classified_private_prefold_spool_site(module, line)
                || classified_certified_cache_handle_site(module, line)
            {
                classified += 1;
            } else {
                violations.push(format!("{module}:{}: {line}", index + 1));
            }
        }
        if file_access_capable && candidates == 0 {
            violations.push(format!(
                "{module}: file-access capability yielded zero recognized source sites"
            ));
        }
        Ok((classified, violations))
    }

    fn loader_pathname_violations(module: &str, source: &str) -> Vec<String> {
        loader_pathname_audit(module, source)
            .expect("pathname audit source must be structurally unambiguous")
            .1
    }

    #[test]
    fn cfg_test_masking_survives_brace_bearing_nested_block_comments() {
        // Kills: a block-comment scanner that closes on the wrong token (the
        // debloat-corrupted `starts_with(b"")` was always true, ending every
        // comment after two bytes) or that lacks the nested `/*` arm (the
        // first `*/` would then drop to Code). Either way the UNBALANCED
        // stray `}` below leaks out of the comment and ends the cfg(test)
        // item inside it, leaving `MASKED_BODY` unmasked — the fixture
        // deliberately carries no `{` in the comment, so the leak cannot
        // re-balance (Codex 01a00f9e b2 R5 finding 1).
        let source = "#[cfg(test)]\nfn masked() {\n    /* outer /* inner */ } popped-early */\n    let secret = \"MASKED_BODY\";\n}\nfn production() { let keep = \"KEPT_PRODUCTION\"; }\n";
        let masked = mask_cfg_test_items(source).expect("scanner must close the cfg(test) item");
        assert!(
            !masked.contains("MASKED_BODY"),
            "cfg(test) body escaped masking: {masked}"
        );
        assert!(
            masked.contains("KEPT_PRODUCTION"),
            "production code after the cfg(test) item was masked: {masked}"
        );
    }

    #[test]
    fn f4_gate_catches_planted_file_and_open_options_descendant_opens() {
        let planted = r#"
use std::fs::{File, OpenOptions};
use std::{fs as aliased_fs, path::Path};
use std::fs as disk;
fn bad(root: &std::path::Path) {
    let _ = File::open(root.join("timeline.json"));
    let _ = OpenOptions::new().read(true).open(root.join("index.json"));
    let _ = aliased_fs::read(root.join("snapshot.json"));
    let _ = disk::read(root.join("aliased.json"));
}
"#;
        let violations = loader_pathname_violations("snapshot/container.rs", planted);
        assert!(
            violations.iter().any(|site| site.contains("File::open")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("OpenOptions")),
            "{violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|site| site.contains("aliased_fs::read")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("disk::read")),
            "{violations:?}"
        );
    }

    #[test]
    fn f4_gate_catches_create_file2_and_pathbuf_push_open_sites() {
        for primitive in [
            "CreateFile2",
            "CreateFileA",
            "CreateFileW",
            "NtCreateFile",
            "ZwCreateFile",
            "NtOpenFile",
            "ZwOpenFile",
            "OpenFileById",
            "ReOpenFile",
        ] {
            let planted = format!(
                "use windows_sys::Win32::Storage::FileSystem::{primitive};\n\
                 fn bad() {{ let _ = {primitive}(0); }}\n"
            );
            let violations =
                loader_pathname_violations(&format!("fixture/{primitive}.rs"), &planted);
            assert!(
                violations.iter().any(|site| site.contains(primitive)),
                "{primitive} must never be an invisible zero-site module: {violations:?}"
            );
        }

        let split_primitive = r#"
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
fn bad() {
    let _ = CreateFileW
        (0);
}
"#;
        let violations =
            loader_pathname_violations("fixture/split_create_file.rs", split_primitive);
        assert!(
            violations
                .iter()
                .any(|site| site.contains("let _ = CreateFileW")),
            "a whitespace-split primitive name must still be classified: {violations:?}"
        );

        let future_windows_api = r#"
use windows_sys::Win32::Storage::FileSystem::{
    FutureFileOpenPrimitive,
};
"#;
        let violations =
            loader_pathname_violations("fixture/future_windows_api.rs", future_windows_api);
        assert!(
            violations
                .iter()
                .any(|site| site.contains("FutureFileOpenPrimitive")),
            "an unknown windows-sys FileSystem capability must fail closed: {violations:?}"
        );

        let pathbuf_push_open = r#"
use std::fs::File;
use std::path::PathBuf;
fn bad(root: &std::path::Path) {
    let mut child = PathBuf::from(root);
    child.push("descendant.json");
    let _ = File::open(child);
}
"#;
        let violations =
            loader_pathname_violations("fixture/pathbuf_push_open.rs", pathbuf_push_open);
        assert!(
            violations.iter().any(|site| site.contains(".push(")),
            "PathBuf::push in a file-open-capable module must be classified: {violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("File::open")),
            "the paired file open must be classified: {violations:?}"
        );
    }

    #[test]
    fn f4_gate_recognizes_link_name_file_open_ffi_declaration() {
        let planted = r#"
#[link(name = "kernel32")]
extern "system" {
    #[link_name = "CreateFileW"]
    fn renamed_open();
}
"#;
        let violations = loader_pathname_violations("fixture/ffi_alias.rs", planted);
        assert!(
            violations.iter().any(|site| site.contains("#[link_name")),
            "the FFI capability declaration itself must be a recognized site: {violations:?}"
        );
    }

    #[test]
    fn f4_gate_discovers_new_module_and_scans_production_after_test_item() {
        let root = temp_root("f4_closed_world");
        fs::write(root.join("existing.rs"), "fn existing() {}\n").unwrap();
        fs::write(
            root.join("newly_added_loader_helper.rs"),
            r#"
use std::fs::File;
fn bad(root: &std::path::Path) {
    let _ = File::open(root.join("new-module.json"));
}
"#,
        )
        .unwrap();
        let discovered = discover_rust_sources("fixture", &root).unwrap();
        let (_, new_module) = discovered
            .iter()
            .find(|(module, _)| module == "fixture/newly_added_loader_helper.rs")
            .expect("new module is discovered without a hand-maintained module list");
        let new_module_violations =
            loader_pathname_violations("fixture/newly_added_loader_helper.rs", new_module);
        assert!(
            new_module_violations
                .iter()
                .any(|site| site.contains("File::open")),
            "{new_module_violations:?}"
        );

        let after_test = r#"
#[cfg(test)]
mod tests {
    fn test_only(root: &std::path::Path) {
        let _ = std::fs::File::open(root.join("test-only.json"));
    }
}

fn production(root: &std::path::Path) {
    let _ = std::fs::File::open(root.join("after-test.json"));
}
"#;
        let after_test_violations = loader_pathname_violations("fixture/after_test.rs", after_test);
        assert!(
            after_test_violations
                .iter()
                .any(|site| site.contains("after-test.json")),
            "{after_test_violations:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_sources_have_no_descendant_pathname_read() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut modules = discover_rust_sources("replay_core", &manifest_dir.join("src"))
            .expect("discover every replay_core production source");
        modules.extend(
            discover_rust_sources(
                "snapshot",
                &manifest_dir.join("../../../portable_replay/native/snapshot/src"),
            )
            .expect("discover every portable snapshot production source"),
        );
        let discovered = modules.len();
        let mut classified = 0;
        let mut violations = Vec::new();
        for (module, source) in modules {
            let (module_classified, mut module_violations) =
                loader_pathname_audit(&module, &source).unwrap_or_else(|error| {
                    panic!("pathname audit could not classify {module}: {error}")
                });
            classified += module_classified;
            violations.append(&mut module_violations);
        }
        assert!(
            violations.is_empty(),
            "unclassified or unrecognized file-access sites across {discovered} discovered modules ({classified} classified): {violations:#?}"
        );
    }
}
