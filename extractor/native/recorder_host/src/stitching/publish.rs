use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::Sha256;
use snapshot::{
    pack_ordered_streaming_to_writer_with_options, BorrowedPackEntry, OrderedStreamingPackEntry,
    PackIdentity, PackOptions, VerifiedPackEntry,
};
#[cfg(test)]
use snapshot::{pack_with, PackEntry};

use super::{CoverageDisposition, PlannedSegment, RawSourceStorage, StitchedPlan};

pub struct StitchedRebuiltWireSource {
    pub source_path: String,
    file_path: PathBuf,
    file: File,
    pub identity_mode: compact_contract::stream_validator::StreamIdentityMode,
}

impl StitchedRebuiltWireSource {
    pub fn open(
        source_path: String,
        file_path: PathBuf,
        identity_mode: compact_contract::stream_validator::StreamIdentityMode,
    ) -> Result<Self, String> {
        let file = File::open(&file_path).map_err(|error| {
            crate::sharing_violation_holders::with_holder(
                format!(
                    "retain stitched rebuilt-wire source {}: {error}",
                    file_path.display()
                ),
                &error,
                &file_path,
            )
        })?;
        Ok(Self {
            source_path,
            file_path,
            file,
            identity_mode,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StitchedRebuiltWireRoute {
    branch: u32,
    segment_id: u32,
}

pub struct StitchedRebuiltWireProduct {
    stitch_input_digest: String,
    bundle: wire_product::PreparedBundle,
    config: wire_product::BundleBuildConfig,
    branches: Vec<wire_product::BranchSpec>,
    catalog: wire_product::AuthenticatedProvinceCatalog,
    sessions: Vec<OwnedStitchedWireSession>,
    leading_gaps: Vec<wire_product::D81LeadingGap>,
    content_manifest: portable_content_manifest::ContentManifest,
    routes: Vec<StitchedRebuiltWireRoute>,
    /// d81 DATA records whose `PSD1` envelope refused reconstruction inside
    /// any member session's selection. Their hours appear in the bundle as
    /// status-bound gaps; this count is the manifest-visible exclusion total.
    unreconstructed_records: u64,
}

struct OwnedStitchedWireSession {
    branch: u32,
    source_path: String,
    identity_mode: compact_contract::stream_validator::StreamIdentityMode,
    file_path: PathBuf,
    file: File,
    stored_sha256: [u8; 32],
    selection: wire_product::H4crSelection,
}

#[cfg(test)]
pub trait StitchedPayloadSource {
    fn snapshot_bytes(&self, segment_id: u32, hour_index: i64) -> Result<Vec<u8>, String>;
    fn raw_source_bytes(&self, source_path: &str) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPayloadReceipt {
    pub bytes: u64,
    pub sha256: String,
}

pub trait RetainedStitchedPayloadSource {
    fn snapshot_receipt(
        &self,
        segment_id: u32,
        hour_index: i64,
    ) -> Result<VerifiedPayloadReceipt, String>;
    fn copy_snapshot_to_unpublished(
        &self,
        segment_id: u32,
        hour_index: i64,
        writer: &mut dyn Write,
    ) -> Result<VerifiedPayloadReceipt, String>;
    fn raw_source_receipt(&self, source_path: &str) -> Result<VerifiedPayloadReceipt, String>;
    fn copy_raw_source_to_unpublished(
        &self,
        source_path: &str,
        writer: &mut dyn Write,
    ) -> Result<VerifiedPayloadReceipt, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub published_path: PathBuf,
    pub container_bytes: usize,
    pub container_sha256: String,
    pub visible_snapshot_hours: usize,
    pub segments: usize,
}

fn stitched_pack_options() -> PackOptions {
    PackOptions {
        container_version: Some(super::STITCH_CONTAINER_VERSION),
        required_features: Some(vec![super::STITCH_REQUIRED_FEATURE.to_string()]),
        ..Default::default()
    }
}

fn pack_identity(
    plan: &StitchedPlan,
    content_manifest: Option<portable_content_manifest::ContentManifest>,
) -> PackIdentity {
    let mod_identity = plan
        .segments
        .iter()
        .find_map(|segment| segment.input.provenance.modset_id.clone())
        .map(|modset_id| serde_json::json!({ "modset_id": modset_id }));
    PackIdentity {
        schema_version: Some(plan.required_inputs.schema_version.clone()),
        build_exe_sha256: plan
            .segments
            .iter()
            .find_map(|segment| segment.input.provenance.build_exe_sha256.clone()),
        build_pinned: false,
        mod_identity,
        created_utc: Some(plan.required_inputs.created_utc.clone()),
        content_manifest,
    }
}

pub fn derive_rebuilt_wire_branches(
    plan: &StitchedPlan,
) -> Result<Vec<wire_product::BranchSpec>, String> {
    derive_rebuilt_wire_plan(plan).map(|(branches, _)| branches)
}

fn derive_rebuilt_wire_plan(
    plan: &StitchedPlan,
) -> Result<(Vec<wire_product::BranchSpec>, Vec<StitchedRebuiltWireRoute>), String> {
    let data_segments = plan
        .segments
        .iter()
        .filter(|segment| !segment.input.is_empty_recovery())
        .collect::<Vec<_>>();
    if data_segments.is_empty() {
        return Err("rebuilt-wire branch table has no data-bearing root".to_string());
    }
    let branch_by_segment = data_segments
        .iter()
        .enumerate()
        .map(|(branch, segment)| {
            u32::try_from(branch)
                .map(|branch| (segment.segment_id, branch))
                .map_err(|_| "rebuilt-wire branch count exceeds u32".to_string())
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let mut branches = Vec::new();
    let mut routes = Vec::new();
    for segment in data_segments {
        let branch = branch_by_segment[&segment.segment_id];
        let cutover = segment
            .input
            .cutover()
            .ok_or_else(|| format!("wire branch {} has no cutover", segment.segment_id))?;
        let first_hour = segment
            .input
            .transition_coverage
            .keys()
            .next()
            .copied()
            .unwrap_or(cutover);
        let first_tick = u64::try_from(first_hour)
            .map_err(|_| format!("wire branch {branch} has negative origin hour {first_hour}"))?;
        let source = segment.input.raw_source.as_ref().ok_or_else(|| {
            format!(
                "wire branch {} has no authenticated raw source",
                segment.segment_id
            )
        })?;
        let provenance_digest = segment
            .input
            .provenance
            .source_decoded_digest
            .as_deref()
            .ok_or_else(|| {
                format!(
                    "wire branch {} has no decoded-source provenance digest",
                    segment.segment_id
                )
            })?;
        if provenance_digest != source.decoded_digest {
            return Err(format!(
                "wire branch {} decoded-source digest disagrees with raw-source ledger",
                segment.segment_id
            ));
        }
        let session_fingerprint = decode_sha256_hex(provenance_digest).map_err(|reason| {
            format!(
                "wire branch {} has invalid decoded-source digest: {reason}",
                segment.segment_id
            )
        })?;
        let parent = if branch == 0 {
            if segment.parent_segment.is_some() {
                return Err("wire root branch 0 unexpectedly has a parent".to_string());
            }
            None
        } else {
            let parent_segment = segment
                .parent_segment
                .ok_or_else(|| format!("wire branch {branch} has no planner-selected parent"))?;
            let parent_branch =
                branch_by_segment
                    .get(&parent_segment)
                    .copied()
                    .ok_or_else(|| {
                        format!(
                        "wire branch {branch} parent segment {parent_segment} is not data-bearing"
                    )
                    })?;
            if parent_branch >= branch {
                return Err(format!(
                    "wire branch {branch} has non-prior parent {parent_branch}"
                ));
            }
            let through_tick = first_tick.checked_sub(1).ok_or_else(|| {
                format!(
                    "wire branch {branch} forks at tick zero and cannot name an inclusive parent point"
                )
            })?;
            Some(wire_product::BranchParent {
                branch: parent_branch,
                through_tick,
            })
        };
        branches.push(wire_product::BranchSpec {
            branch,
            parent,
            first_tick,
            session_fingerprint,
        });
        routes.push(StitchedRebuiltWireRoute {
            branch,
            segment_id: segment.segment_id,
        });
    }
    Ok((branches, routes))
}

fn decode_sha256_hex(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("expected exactly 64 hexadecimal characters".to_string());
    }
    let mut out = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let text =
            std::str::from_utf8(chunk).map_err(|_| "SHA-256 digest is not ASCII".to_string())?;
        out[index] = u8::from_str_radix(text, 16)
            .map_err(|_| "SHA-256 digest contains invalid hexadecimal".to_string())?;
    }
    Ok(out)
}

fn hash_retained_file_sha256(file: &File, path: &Path) -> Result<(u64, [u8; 32]), String> {
    let mut file = file
        .try_clone()
        .map_err(|error| format!("clone stitched wire source {}: {error}", path.display()))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind stitched wire source {}: {error}", path.display()))?;
    let mut sha256 = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("hash stitched wire source {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(
                u64::try_from(read)
                    .map_err(|_| "stitched wire source read length does not fit u64".to_string())?,
            )
            .ok_or_else(|| "stitched wire source length overflows u64".to_string())?;
        sha256.update(&buffer[..read]);
    }
    Ok((bytes, sha256.finalize().into()))
}

fn verify_rebuilt_wire_source_receipts(
    sessions: &[OwnedStitchedWireSession],
) -> Result<(), String> {
    for session in sessions {
        let (_, actual) = hash_retained_file_sha256(&session.file, &session.file_path)?;
        if actual != session.stored_sha256 {
            return Err(format!(
                "stitched wire source {} ({}) stored SHA-256 differs from the final plan",
                session.source_path,
                session.file_path.display()
            ));
        }
    }
    Ok(())
}

fn inspect_rebuilt_wire_session_first_ticks(
    sessions: &[OwnedStitchedWireSession],
    catalog: Option<&wire_product::AuthenticatedProvinceCatalog>,
) -> Result<(std::collections::BTreeMap<u32, u64>, u64), String> {
    let mut first_ticks = std::collections::BTreeMap::new();
    let mut unreconstructed_records = 0u64;
    for session in sessions {
        let inspection = match catalog {
            Some(catalog) => wire_product::inspect_d81_session_with_catalog(
                wire_product::H4crSession {
                    branch: session.branch,
                    identity_mode: session.identity_mode,
                    source: wire_product::H4crSource::RetainedFileSelection(
                        &session.file,
                        session.selection,
                    ),
                },
                catalog,
            ),
            None => wire_product::inspect_d81_session(wire_product::H4crSession {
                branch: session.branch,
                identity_mode: session.identity_mode,
                source: wire_product::H4crSource::RetainedFileSelection(
                    &session.file,
                    session.selection,
                ),
            }),
        }
        .map_err(|error| {
            format!(
                "inspect retained stitched H4CR branch {} coverage: {error}",
                session.branch
            )
        })?;
        if first_ticks
            .insert(session.branch, inspection.first_tick)
            .is_some()
        {
            return Err(format!(
                "retained stitched H4CR repeats branch {}",
                session.branch
            ));
        }
        // THE STITCHED PRODUCT'S OWN NAMED EXCLUSION COUNT. Each branch is
        // scanned with the same consumer contract as a whole-stream product;
        // hours whose envelope refused reconstruction appear in this bundle as
        // status-bound gaps, and the count says how many without a reader
        // having to re-derive it from the gap records.
        unreconstructed_records += inspection.unreconstructed_records;
    }
    Ok((first_ticks, unreconstructed_records))
}

fn derive_rebuilt_wire_plan_from_sessions(
    plan: &StitchedPlan,
    catalog: Option<&wire_product::AuthenticatedProvinceCatalog>,
    sessions: &[OwnedStitchedWireSession],
) -> Result<
    (
        Vec<wire_product::BranchSpec>,
        Vec<StitchedRebuiltWireRoute>,
        std::collections::BTreeMap<u32, u64>,
        u64,
    ),
    String,
> {
    let (mut branches, routes) = derive_rebuilt_wire_plan(plan)?;
    let (first_ticks, unreconstructed_records) =
        inspect_rebuilt_wire_session_first_ticks(sessions, catalog)?;
    if first_ticks.len() != branches.len() {
        return Err(
            "retained stitched H4CR session count differs from rebuilt-wire branches".to_string(),
        );
    }
    for branch in &mut branches {
        let source_first_tick = first_ticks.get(&branch.branch).copied().ok_or_else(|| {
            format!(
                "rebuilt-wire branch {} has no retained H4CR coverage inspection",
                branch.branch
            )
        })?;
        branch.first_tick = branch.first_tick.min(source_first_tick);
        if let Some(parent) = branch.parent.as_mut() {
            parent.through_tick = branch.first_tick.checked_sub(1).ok_or_else(|| {
                format!(
                    "wire branch {} authenticated coverage begins at tick zero and cannot name an inclusive parent point",
                    branch.branch
                )
            })?;
        }
    }
    Ok((branches, routes, first_ticks, unreconstructed_records))
}

fn derive_rebuilt_wire_leading_gaps(
    plan: &StitchedPlan,
    routes: &[StitchedRebuiltWireRoute],
    source_first_ticks: &std::collections::BTreeMap<u32, u64>,
) -> Result<Vec<wire_product::D81LeadingGap>, String> {
    let mut gaps = Vec::new();
    for route in routes {
        let segment = plan
            .segments
            .iter()
            .find(|segment| segment.segment_id == route.segment_id)
            .ok_or_else(|| format!("wire gap route lost segment {}", route.segment_id))?;
        let mut open: Option<(u64, u64, wire_product::D81LeadingGapKind)> = None;
        for (hour, disposition) in &segment.input.transition_coverage {
            let tick = u64::try_from(*hour).map_err(|_| {
                format!(
                    "wire transition gap for segment {} is negative",
                    route.segment_id
                )
            })?;
            let source_first_tick =
                source_first_ticks
                    .get(&route.branch)
                    .copied()
                    .ok_or_else(|| {
                        format!(
                        "wire transition gap branch {} has no retained H4CR coverage inspection",
                        route.branch
                    )
                    })?;
            if tick >= source_first_tick {
                continue;
            }
            let kind = match disposition {
                CoverageDisposition::CaptureFailed { .. } => {
                    wire_product::D81LeadingGapKind::CaptureFailed
                }
                CoverageDisposition::Coalesced { .. } => wire_product::D81LeadingGapKind::Coalesced,
                CoverageDisposition::Gap { .. } => wire_product::D81LeadingGapKind::ExplicitGap,
                CoverageDisposition::Snapshot { .. } => {
                    return Err(format!(
                        "wire transition coverage for segment {} contains a snapshot",
                        route.segment_id
                    ))
                }
            };
            match open {
                Some((start, end, prior_kind)) if end == tick && prior_kind == kind => {
                    open = Some((start, tick + 1, kind));
                }
                Some((start, end, prior_kind)) => {
                    gaps.push(
                        wire_product::D81LeadingGap::new(route.branch, start, end, prior_kind)
                            .map_err(|error| {
                                format!("build wire transition gap failed: {error}")
                            })?,
                    );
                    open = Some((tick, tick + 1, kind));
                }
                None => open = Some((tick, tick + 1, kind)),
            }
        }
        if let Some((start, end, kind)) = open {
            gaps.push(
                wire_product::D81LeadingGap::new(route.branch, start, end, kind)
                    .map_err(|error| format!("build wire transition gap failed: {error}"))?,
            );
        }
    }
    Ok(gaps)
}

impl StitchedRebuiltWireProduct {
    pub fn build(
        plan: &StitchedPlan,
        config: wire_product::BundleBuildConfig,
        content_manifest: portable_content_manifest::ContentManifest,
        catalog: wire_product::AuthenticatedProvinceCatalog,
        sources: Vec<StitchedRebuiltWireSource>,
    ) -> Result<Self, String> {
        let manifest_sha256 = content_manifest
            .sha256()
            .map_err(|error| format!("hash stitched content manifest failed: {error}"))
            .and_then(|digest| decode_sha256_hex(&digest))?;
        if manifest_sha256 != catalog.content_manifest_sha256() {
            return Err(
                "stitched province catalog belongs to a different content manifest".to_string(),
            );
        }
        for segment in &plan.segments {
            if segment.input.is_empty_recovery() {
                continue;
            }
            let declared = segment
                .input
                .provenance
                .static_catalog_digest
                .as_deref()
                .ok_or_else(|| {
                    format!(
                        "segment {} has no static catalog digest",
                        segment.segment_id
                    )
                })?;
            if decode_sha256_hex(declared)? != manifest_sha256 {
                return Err(format!(
                    "segment {} static catalog differs from rebuilt-wire catalog",
                    segment.segment_id
                ));
            }
        }
        let (_, routes) = derive_rebuilt_wire_plan(plan)?;
        let source_count = sources.len();
        let source_by_path = sources
            .into_iter()
            .map(|source| (source.source_path.clone(), source))
            .collect::<std::collections::BTreeMap<_, _>>();
        if source_by_path.len() != source_count {
            return Err("stitched rebuilt-wire sources repeat a source path".to_string());
        }
        let mut used_paths = std::collections::BTreeSet::new();
        let mut sessions = Vec::with_capacity(routes.len());
        for route in &routes {
            let segment = plan
                .segments
                .iter()
                .find(|segment| segment.segment_id == route.segment_id)
                .ok_or_else(|| format!("wire route lost segment {}", route.segment_id))?;
            let raw =
                segment.input.raw_source.as_ref().ok_or_else(|| {
                    format!("wire segment {} has no raw source", route.segment_id)
                })?;
            let source = source_by_path.get(&raw.source_path).ok_or_else(|| {
                format!(
                    "wire segment {} has no retained source for {}",
                    route.segment_id, raw.source_path
                )
            })?;
            used_paths.insert(raw.source_path.as_str());
            let (first_sequence, last_sequence) =
                segment.input.provenance.record_seq_bounds.ok_or_else(|| {
                    format!("wire segment {} has no record bounds", route.segment_id)
                })?;
            let (start_byte, end_byte) =
                segment.input.provenance.byte_bounds.ok_or_else(|| {
                    format!("wire segment {} has no byte bounds", route.segment_id)
                })?;
            sessions.push(OwnedStitchedWireSession {
                branch: route.branch,
                source_path: raw.source_path.clone(),
                identity_mode: source.identity_mode,
                file_path: source.file_path.clone(),
                file: source.file.try_clone().map_err(|error| {
                    format!(
                        "clone retained wire source {} for branch {}: {error}",
                        source.file_path.display(),
                        route.branch
                    )
                })?,
                stored_sha256: decode_sha256_hex(&raw.stored_digest).map_err(|error| {
                    format!(
                        "wire segment {} stored source digest is invalid: {error}",
                        route.segment_id
                    )
                })?,
                selection: wire_product::H4crSelection {
                    first_sequence,
                    last_sequence,
                    start_byte,
                    end_byte,
                },
            });
        }
        if used_paths.len() != source_by_path.len() {
            return Err("stitched rebuilt-wire sources include an unreferenced path".to_string());
        }
        verify_rebuilt_wire_source_receipts(&sessions)?;
        let (branches, inspected_routes, source_first_ticks, unreconstructed_records) =
            derive_rebuilt_wire_plan_from_sessions(plan, Some(&catalog), &sessions)?;
        if inspected_routes != routes {
            return Err(
                "retained H4CR inspection changed the rebuilt-wire route table".to_string(),
            );
        }
        let leading_gaps = derive_rebuilt_wire_leading_gaps(plan, &routes, &source_first_ticks)?;
        let borrowed = borrowed_wire_sessions(&sessions);
        let bundle = wire_product::build_d81_bundle_with_leading_gaps(
            config,
            &branches,
            &catalog,
            &borrowed,
            &leading_gaps,
        )
        .map_err(|error| format!("build stitched rebuilt-wire product failed: {error}"))?;
        Ok(Self {
            stitch_input_digest: plan.stitch_input_digest.clone(),
            bundle,
            config,
            branches,
            catalog,
            sessions,
            leading_gaps,
            content_manifest,
            routes,
            unreconstructed_records,
        })
    }
}

fn borrowed_wire_sessions(
    sessions: &[OwnedStitchedWireSession],
) -> Vec<wire_product::H4crSession<'_>> {
    sessions
        .iter()
        .map(|session| wire_product::H4crSession {
            branch: session.branch,
            identity_mode: session.identity_mode,
            source: wire_product::H4crSource::RetainedFileSelection(
                &session.file,
                session.selection,
            ),
        })
        .collect()
}

#[cfg(test)]
pub fn build_stitched_entries<S: StitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
) -> Result<Vec<PackEntry>, String> {
    let game = plan.game_id.as_str();
    let mut entries: Vec<PackEntry> = Vec::new();

    for f in plan.to_container_files() {
        entries.push(PackEntry {
            path: format!("payload/{game}/{}", f.rel_path),
            bytes: f.bytes,
            role: match f.role {
                "index" => "index",
                "segments" => "segments",
                "segment_index" => "segment_index",
                _ => "aux",
            },
            hour_index: None,
        });
    }

    let mut raw_paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for segment in &plan.segments {
        let raw = segment.input.raw_source.as_ref().ok_or_else(|| {
            format!(
                "segment {} has no raw-source ledger; cannot publish",
                segment.segment_id
            )
        })?;
        raw_paths.insert(raw.source_path.clone());
    }
    for path in raw_paths {
        let descriptor = plan
            .segments
            .iter()
            .find_map(|segment| {
                segment
                    .input
                    .raw_source
                    .as_ref()
                    .filter(|raw| raw.source_path == path)
            })
            .ok_or_else(|| format!("no raw-source descriptor for {path}; cannot publish"))?;
        match &descriptor.storage {
            RawSourceStorage::Standalone => continue,
            RawSourceStorage::EmbeddedCampaign { .. } => {
                let bytes = source.raw_source_bytes(&path)?;
                let actual = Sha256::hex(&bytes);
                if actual != descriptor.stored_digest {
                    return Err(format!(
                        "raw source {path} stored bytes hash {actual} != recorded stored digest {}",
                        descriptor.stored_digest
                    ));
                }
                entries.push(PackEntry {
                    path: format!("payload/{game}/{path}"),
                    bytes,
                    role: "raw_source",
                    hour_index: None,
                });
            }
        }
    }

    for segment in &plan.segments {
        for (hour, disposition) in segment.input.coverage.iter() {
            if let CoverageDisposition::Snapshot { sha256, .. } = disposition {
                let bytes = source.snapshot_bytes(segment.segment_id, *hour)?;
                let actual = Sha256::hex(&bytes);
                if &actual != sha256 {
                    return Err(format!(
                        "segment {} hour {hour} snapshot hashes {actual} != recorded {sha256}",
                        segment.segment_id
                    ));
                }
                entries.push(PackEntry {
                    path: format!(
                        "payload/{game}/{}",
                        PlannedSegment::snapshot_file(segment.segment_id, *hour)
                    ),
                    bytes,
                    role: "snapshot",
                    hour_index: Some(*hour),
                });
            }
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

#[cfg(test)]
pub fn pack_and_validate<S: StitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
) -> Result<(Vec<u8>, PublishReport), String> {
    let entries = build_stitched_entries(plan, source)?;
    let bytes = pack_with(
        &plan.game_id,
        &entries,
        &pack_identity(plan, None),
        false,
        &stitched_pack_options(),
    )
    .map_err(|error| format!("pack stitched container failed: {error}"))?;

    let container = Container::from_bytes(bytes.clone(), &ExpectedIdentity::default())
        .map_err(|error| format!("stitched container failed to re-open for validation: {error}"))?;
    container
        .verify_all()
        .map_err(|error| format!("stitched container payload verification failed: {error}"))?;
    if !container.index().is_stitched() {
        return Err("packed container is not a stitched (stitched-main-v1) index".to_string());
    }
    let graph = container
        .segments()
        .map_err(|error| format!("stitched container graph failed to parse: {error}"))?
        .ok_or_else(|| "packed v2 container has no segments graph".to_string())?;

    let visible = plan.visible_snapshot_files();
    for (hour, file) in &visible {
        let resolved = container
            .index()
            .snapshots
            .iter()
            .find(|snapshot| snapshot.hour_index == Some(*hour))
            .ok_or_else(|| format!("visible hour {hour} missing from published index"))?;
        if &resolved.file != file {
            return Err(format!(
                "visible hour {hour} resolves to {} but the plan names {file}",
                resolved.file
            ));
        }
    }

    let report = PublishReport {
        published_path: PathBuf::new(),
        container_bytes: bytes.len(),
        container_sha256: Sha256::hex(&bytes),
        visible_snapshot_hours: visible.len(),
        segments: graph.segments.len(),
    };
    Ok((bytes, report))
}

#[derive(Clone)]
enum RetainedSourceKey {
    Snapshot { segment_id: u32, hour_index: i64 },
    Raw { source_path: String },
}

enum PackRetainedSource {
    Payload(RetainedSourceKey),
    File { file_path: PathBuf, file: File },
}

enum PreparedRetainedEntry {
    Memory {
        path: String,
        bytes: Vec<u8>,
        role: &'static str,
        hour_index: Option<i64>,
    },
    Source {
        path: String,
        source: RetainedSourceKey,
        receipt: VerifiedPayloadReceipt,
        role: &'static str,
        hour_index: Option<i64>,
    },
    File {
        path: String,
        file_path: PathBuf,
        file: File,
        receipt: VerifiedPayloadReceipt,
        role: &'static str,
        hour_index: Option<i64>,
    },
}

impl PreparedRetainedEntry {
    fn path(&self) -> &str {
        match self {
            Self::Memory { path, .. } | Self::Source { path, .. } | Self::File { path, .. } => path,
        }
    }
}

fn prepare_retained_entries<S: RetainedStitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
    rebuilt_wire: Option<&StitchedRebuiltWireProduct>,
) -> Result<Vec<PreparedRetainedEntry>, String> {
    let game = plan.game_id.as_str();
    let mut container_files = plan.to_container_files();
    if let Some(product) = rebuilt_wire {
        let index = container_files
            .iter_mut()
            .find(|file| file.rel_path == "index.json")
            .ok_or_else(|| {
                "stitched plan has no index.json for rebuilt-wire discovery".to_string()
            })?;
        index.bytes = rebuilt_wire_index_bytes(plan, product)?;
    }
    let mut entries = container_files
        .into_iter()
        .map(|file| PreparedRetainedEntry::Memory {
            path: format!("payload/{game}/{}", file.rel_path),
            bytes: file.bytes,
            role: match file.role {
                "index" => "index",
                "segments" => "segments",
                "segment_index" => "segment_index",
                _ => "aux",
            },
            hour_index: None,
        })
        .collect::<Vec<_>>();

    let mut raw_paths = std::collections::BTreeSet::new();
    for segment in &plan.segments {
        let raw = segment.input.raw_source.as_ref().ok_or_else(|| {
            format!(
                "segment {} has no raw-source ledger; cannot publish",
                segment.segment_id
            )
        })?;
        raw_paths.insert(raw.source_path.clone());
    }
    for source_path in raw_paths {
        let descriptor = plan
            .segments
            .iter()
            .find_map(|segment| {
                segment
                    .input
                    .raw_source
                    .as_ref()
                    .filter(|raw| raw.source_path == source_path)
            })
            .ok_or_else(|| format!("no raw-source descriptor for {source_path}"))?;
        if matches!(descriptor.storage, RawSourceStorage::Standalone) {
            continue;
        }
        let receipt = source.raw_source_receipt(&source_path)?;
        if receipt.sha256 != descriptor.stored_digest {
            return Err(format!(
                "raw source {source_path} receipt hashes {} != recorded {}",
                receipt.sha256, descriptor.stored_digest
            ));
        }
        entries.push(PreparedRetainedEntry::Source {
            path: format!("payload/{game}/{source_path}"),
            source: RetainedSourceKey::Raw { source_path },
            receipt,
            role: "raw_source",
            hour_index: None,
        });
    }

    for segment in &plan.segments {
        for (hour_index, disposition) in &segment.input.coverage {
            let CoverageDisposition::Snapshot { sha256, .. } = disposition else {
                continue;
            };
            let receipt = source.snapshot_receipt(segment.segment_id, *hour_index)?;
            if &receipt.sha256 != sha256 {
                return Err(format!(
                    "segment {} hour {hour_index} snapshot receipt hashes {} != recorded {sha256}",
                    segment.segment_id, receipt.sha256
                ));
            }
            entries.push(PreparedRetainedEntry::Source {
                path: format!(
                    "payload/{game}/{}",
                    PlannedSegment::snapshot_file(segment.segment_id, *hour_index)
                ),
                source: RetainedSourceKey::Snapshot {
                    segment_id: segment.segment_id,
                    hour_index: *hour_index,
                },
                receipt,
                role: "snapshot",
                hour_index: Some(*hour_index),
            });
        }
    }
    if let Some(product) = rebuilt_wire {
        wire_product::verify_prepared_bundle(&product.bundle)
            .map_err(|error| format!("prepared rebuilt-wire bundle is invalid: {error}"))?;
        for artifact in product.bundle.entries() {
            let file = File::open(&artifact.file_path).map_err(|error| {
                crate::sharing_violation_holders::with_holder(
                    format!(
                        "retain prepared rebuilt-wire artifact {}: {error}",
                        artifact.file_path.display()
                    ),
                    &error,
                    &artifact.file_path,
                )
            })?;
            let (bytes, sha256) = hash_retained_file_sha256(&file, &artifact.file_path)?;
            if bytes != artifact.len {
                return Err(format!(
                    "prepared rebuilt-wire artifact {} length changed after verification",
                    artifact.path
                ));
            }
            entries.push(PreparedRetainedEntry::File {
                path: format!("payload/{game}/{}", artifact.path),
                file_path: artifact.file_path.clone(),
                file,
                receipt: VerifiedPayloadReceipt {
                    bytes,
                    sha256: hex32(&sha256),
                },
                role: artifact.role.as_str(),
                hour_index: None,
            });
        }
    }
    entries.sort_by(|left, right| left.path().cmp(right.path()));
    for pair in entries.windows(2) {
        if pair[0].path() == pair[1].path() {
            return Err(format!(
                "duplicate stitched container entry path {}",
                pair[0].path()
            ));
        }
    }
    Ok(entries)
}

fn rebuilt_wire_index_bytes(
    plan: &StitchedPlan,
    product: &StitchedRebuiltWireProduct,
) -> Result<Vec<u8>, String> {
    let verified = wire_product::verify_prepared_bundle(&product.bundle)
        .map_err(|error| format!("prepare rebuilt-wire discovery index failed: {error}"))?;
    let mut index = plan.main_index_value();
    let object = index
        .as_object_mut()
        .ok_or_else(|| "stitched main index is not an object".to_string())?;
    object.insert(
        "rebuilt_wire".to_string(),
        serde_json::json!({
            "root": "rebuilt_wire",
            "stitch_input_digest": plan.stitch_input_digest,
            "content_manifest_sha256": hex32(&verified.content_manifest_sha256),
            "definition_csv_sha256": hex32(&verified.definition_csv_sha256),
            "d81_unreconstructed_records": product.unreconstructed_records,
            "branches": product.routes.iter().map(|route| serde_json::json!({
                "branch": route.branch,
                "segment_id": route.segment_id,
            })).collect::<Vec<_>>(),
            "entries": product.bundle.entries().iter().map(|entry| serde_json::json!({
                "path": entry.path,
                "role": entry.role.as_str(),
                "bytes": entry.len,
                "blake3": hex32(&entry.blake3),
            })).collect::<Vec<_>>(),
        }),
    );
    let mut text = String::new();
    snapshot::canon::write_canonical(&index, &mut text);
    Ok(text.into_bytes())
}

fn hex32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn validate_stitched_container(
    plan: &StitchedPlan,
    container: &Container,
) -> Result<usize, String> {
    if !container.index().is_stitched() {
        return Err("packed container is not a stitched (stitched-main-v1) index".to_string());
    }
    let graph = container
        .segments()
        .map_err(|error| format!("stitched container graph failed to parse: {error}"))?
        .ok_or_else(|| "packed v2 container has no segments graph".to_string())?;
    for (hour, file) in plan.visible_snapshot_files() {
        let resolved = container
            .index()
            .snapshots
            .iter()
            .find(|snapshot| snapshot.hour_index == Some(hour))
            .ok_or_else(|| format!("visible hour {hour} missing from published index"))?;
        if resolved.file != file {
            return Err(format!(
                "visible hour {hour} resolves to {} but the plan names {file}",
                resolved.file
            ));
        }
    }
    Ok(graph.segments.len())
}

fn verify_reopened_rebuilt_wire_bundle(
    plan: &StitchedPlan,
    product: &StitchedRebuiltWireProduct,
    container: &Container,
) -> Result<(), String> {
    verify_rebuilt_wire_source_receipts(&product.sessions)?;
    let index_path = format!("payload/{}/index.json", plan.game_id);
    let reopened_index = container
        .read_entry_bytes(&index_path)
        .map_err(|error| format!("read stitched rebuilt-wire discovery index failed: {error}"))?;
    let expected_index = rebuilt_wire_index_bytes(plan, product)?;
    if reopened_index != expected_index {
        return Err(
            "stitched rebuilt-wire discovery index differs from the final plan/product".to_string(),
        );
    }
    let opened_manifest = container
        .content_manifest()
        .ok_or_else(|| "stitched rebuilt-wire container has no content manifest".to_string())?;
    let opened_manifest_sha = opened_manifest
        .sha256()
        .map_err(|error| format!("hash reopened stitched content manifest failed: {error}"))
        .and_then(|digest| decode_sha256_hex(&digest))?;
    if opened_manifest_sha != product.catalog.content_manifest_sha256() {
        return Err(
            "stitched container content manifest differs from rebuilt-wire catalog".to_string(),
        );
    }
    let prefix = format!("payload/{}/rebuilt_wire/", plan.game_id);
    let mut descriptors = product
        .bundle
        .entries()
        .iter()
        .map(wire_product::BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let mut packed_entries = std::collections::BTreeMap::new();
    for entry in container.entries() {
        let role = entry.role.as_deref();
        let parsed_role = role.and_then(|value| wire_product::ArtifactRole::from_str(value).ok());
        if !entry.path.starts_with(&prefix) {
            if parsed_role.is_some() {
                return Err(format!(
                    "rebuilt-wire role {:?} appears outside canonical prefix {prefix}",
                    role.unwrap_or_default()
                ));
            }
            continue;
        }
        let role = role
            .ok_or_else(|| {
                format!(
                    "rebuilt-wire container entry {} has no manifest role",
                    entry.path
                )
            })
            .and_then(|role| {
                wire_product::ArtifactRole::from_str(role).map_err(|error| {
                    format!(
                        "rebuilt-wire container entry {} has invalid role: {error}",
                        entry.path
                    )
                })
            })?;
        let relative = entry
            .path
            .strip_prefix(&format!("payload/{}/", plan.game_id))
            .ok_or_else(|| format!("rebuilt-wire entry {} lost its game prefix", entry.path))?;
        if packed_entries
            .insert(relative.to_string(), (role, entry.bytes))
            .is_some()
        {
            return Err(format!("rebuilt-wire container repeats entry {relative}"));
        }
    }
    if packed_entries.len() != descriptors.len() {
        return Err(
            "reopened rebuilt-wire entry cardinality differs from the exact prepared bundle"
                .to_string(),
        );
    }
    for descriptor in &descriptors {
        let (role, bytes) = packed_entries.get(&descriptor.path).ok_or_else(|| {
            format!(
                "reopened stitched container is missing rebuilt-wire entry {}",
                descriptor.path
            )
        })?;
        if *role != descriptor.role || *bytes != descriptor.len {
            return Err(format!(
                "reopened rebuilt-wire metadata differs for {}",
                descriptor.path
            ));
        }
    }
    let sessions = borrowed_wire_sessions(&product.sessions);
    wire_product::verify_entry_source_against_h4cr(
        &descriptors,
        |path| {
            let packed_path = format!("payload/{}/{path}", plan.game_id);
            container
                .read_entry_bytes(&packed_path)
                .map_err(|error| format!("read stitched rebuilt-wire entry {packed_path}: {error}"))
        },
        product.config,
        &product.branches,
        &product.catalog,
        &sessions,
        &product.leading_gaps,
    )
    .map_err(|error| format!("stitched rebuilt-wire H4CR parity failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
pub fn publish_stitched_container_retained<S: RetainedStitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
    dest: &Path,
) -> Result<PublishReport, String> {
    publish_stitched_container_retained_impl(plan, source, None, dest)
}

pub fn publish_stitched_container_retained_with_rebuilt_wire<S: RetainedStitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
    rebuilt_wire: StitchedRebuiltWireProduct,
    dest: &Path,
) -> Result<PublishReport, String> {
    publish_stitched_container_retained_impl(plan, source, Some(&rebuilt_wire), dest)
}

fn publish_stitched_container_retained_impl<S: RetainedStitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
    rebuilt_wire: Option<&StitchedRebuiltWireProduct>,
    dest: &Path,
) -> Result<PublishReport, String> {
    if let Some(product) = rebuilt_wire {
        if product.stitch_input_digest != plan.stitch_input_digest {
            return Err("rebuilt-wire product belongs to a different stitched plan".to_string());
        }
        let (branches, routes, source_first_ticks, unreconstructed_records) =
            derive_rebuilt_wire_plan_from_sessions(plan, Some(&product.catalog), &product.sessions)?;
        if branches != product.branches || routes != product.routes {
            return Err(
                "rebuilt-wire product branch join differs from the final stitched plan".to_string(),
            );
        }
        if unreconstructed_records != product.unreconstructed_records {
            return Err(
                "rebuilt-wire product d81 exclusion count differs from the final stitched plan"
                    .to_string(),
            );
        }
        let leading_gaps = derive_rebuilt_wire_leading_gaps(plan, &routes, &source_first_ticks)?;
        if leading_gaps != product.leading_gaps {
            return Err(
                "rebuilt-wire product leading gaps differ from the final stitched plan".to_string(),
            );
        }
        verify_rebuilt_wire_source_receipts(&product.sessions)?;
    }
    let entries = prepare_retained_entries(plan, source, rebuilt_wire)?;
    let parent = dest.parent().filter(|path| !path.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|error| {
            crate::sharing_violation_holders::with_holder(
                format!("create output directory failed: {error}"),
                &error,
                parent,
            )
        })?;
    }
    let file_name = dest
        .file_name()
        .ok_or_else(|| "stitched publication destination has no file name".to_string())?;
    let anchor = parent.unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(anchor).map_err(|error| {
        crate::sharing_violation_holders::with_holder(
            format!("anchor stitched output directory failed: {error}"),
            &error,
            anchor,
        )
    })?;
    let anchored_dest = canonical_parent.join(file_name);
    reject_reparse_destination(&anchored_dest)?;
    let destination_guard = RetainedPublishDirectory::open(&canonical_parent)?;
    let (temp, mut file) = create_unique_sibling_temp(&anchored_dest)?;

    let publish_result = (|| -> Result<PublishReport, String> {
        let mut source_keys: Vec<(PackRetainedSource, VerifiedPayloadReceipt)> = Vec::new();
        let ordered = entries
            .iter()
            .map(|entry| -> Result<OrderedStreamingPackEntry<'_>, String> {
                Ok(match entry {
                    PreparedRetainedEntry::Memory {
                        path,
                        bytes,
                        role,
                        hour_index,
                    } => OrderedStreamingPackEntry::Borrowed(BorrowedPackEntry {
                        path: path.clone(),
                        bytes,
                        role: *role,
                        hour_index: *hour_index,
                    }),
                    PreparedRetainedEntry::Source {
                        path,
                        source,
                        receipt,
                        role,
                        hour_index,
                    } => {
                        let source_id = source_keys.len();
                        source_keys
                            .push((PackRetainedSource::Payload(source.clone()), receipt.clone()));
                        OrderedStreamingPackEntry::Verified(VerifiedPackEntry {
                            path: path.clone(),
                            source_id,
                            expected_len: receipt.bytes,
                            expected_sha256: receipt.sha256.clone(),
                            role: *role,
                            hour_index: *hour_index,
                        })
                    }
                    PreparedRetainedEntry::File {
                        path,
                        file_path,
                        file,
                        receipt,
                        role,
                        hour_index,
                    } => {
                        let source_id = source_keys.len();
                        source_keys.push((
                            PackRetainedSource::File {
                                file_path: file_path.clone(),
                                file: file.try_clone().map_err(|error| {
                                    format!(
                                        "clone prepared rebuilt-wire artifact {}: {error}",
                                        file_path.display()
                                    )
                                })?,
                            },
                            receipt.clone(),
                        ));
                        OrderedStreamingPackEntry::Verified(VerifiedPackEntry {
                            path: path.clone(),
                            source_id,
                            expected_len: receipt.bytes,
                            expected_sha256: receipt.sha256.clone(),
                            role: *role,
                            hour_index: *hour_index,
                        })
                    }
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        pack_ordered_streaming_to_writer_with_options(
            &plan.game_id,
            &ordered,
            &pack_identity(
                plan,
                rebuilt_wire.map(|product| product.content_manifest.clone()),
            ),
            false,
            &stitched_pack_options(),
            &mut file,
            |source_id, writer| {
                let (key, expected) = source_keys.get(source_id).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "stitched pack requested an unknown retained source",
                    )
                })?;
                let actual = match key {
                    PackRetainedSource::Payload(RetainedSourceKey::Snapshot {
                        segment_id,
                        hour_index,
                    }) => source
                        .copy_snapshot_to_unpublished(*segment_id, *hour_index, writer)
                        .map_err(|error| {
                            std::io::Error::new(std::io::ErrorKind::InvalidData, error)
                        }),
                    PackRetainedSource::Payload(RetainedSourceKey::Raw { source_path }) => source
                        .copy_raw_source_to_unpublished(source_path, writer)
                        .map_err(|error| {
                            std::io::Error::new(std::io::ErrorKind::InvalidData, error)
                        }),
                    PackRetainedSource::File { file_path, file } => {
                        let mut read = file.try_clone().map_err(|error| {
                            std::io::Error::new(
                                error.kind(),
                                format!(
                                    "clone rebuilt-wire artifact {}: {error}",
                                    file_path.display()
                                ),
                            )
                        })?;
                        read.seek(SeekFrom::Start(0)).map_err(|error| {
                            std::io::Error::new(
                                error.kind(),
                                format!(
                                    "rewind rebuilt-wire artifact {}: {error}",
                                    file_path.display()
                                ),
                            )
                        })?;
                        let bytes = std::io::copy(&mut read, writer).map_err(|error| {
                            std::io::Error::new(
                                error.kind(),
                                format!(
                                    "copy rebuilt-wire artifact {}: {error}",
                                    file_path.display()
                                ),
                            )
                        })?;
                        Ok(VerifiedPayloadReceipt {
                            bytes,
                            sha256: expected.sha256.clone(),
                        })
                    }
                }?;
                if &actual != expected {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "retained source copy receipt changed during stitched pack",
                    ));
                }
                Ok(())
            },
        )
        .map_err(|error| format!("pack stitched container failed: {error}"))?;
        file.flush().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let verify_handle = file.try_clone().map_err(|error| error.to_string())?;
        let (container, container_bytes, container_sha256) =
            Container::open_strict_with_file_receipt_from_file(
                verify_handle,
                &ExpectedIdentity::default(),
            )
            .map_err(|error| format!("stitched temp failed to re-open: {error}"))?;
        container
            .verify_all_streaming()
            .map_err(|error| format!("stitched temp payload verification failed: {error}"))?;
        if container
            .retained_file_receipt()
            .map_err(|error| format!("receipt stitched temp after verify failed: {error}"))?
            != (container_bytes, container_sha256.clone())
        {
            return Err("stitched temp changed during bounded verification".to_string());
        }
        let segments = validate_stitched_container(plan, &container)?;
        if let Some(product) = rebuilt_wire {
            verify_reopened_rebuilt_wire_bundle(plan, product, &container)?;
        }
        if container.retained_file_receipt().map_err(|error| {
            format!("receipt stitched temp after semantic verification failed: {error}")
        })? != (container_bytes, container_sha256.clone())
        {
            return Err(
                "stitched temp changed during rebuilt-wire semantic verification".to_string(),
            );
        }
        Ok(PublishReport {
            published_path: dest.to_path_buf(),
            container_bytes: usize::try_from(container_bytes)
                .map_err(|_| "stitched container length does not fit usize")?,
            container_sha256,
            visible_snapshot_hours: plan.visible_snapshot_files().len(),
            segments,
        })
    })();
    let report = match publish_result {
        Ok(report) => report,
        Err(error) => {
            drop(file);
            let cleanup = fs::remove_file(&temp);
            return Err(with_cleanup_note(
                format!("write stitched temp container failed: {error}"),
                &temp,
                cleanup,
            ));
        }
    };
    if let Err(error) = atomic_replace_retained(&file, &temp, &anchored_dest, &destination_guard) {
        drop(file);
        let cleanup = fs::remove_file(&temp);
        return Err(with_cleanup_note(
            crate::sharing_violation_holders::with_holder(
                format!("atomic publish (rename) failed: {error}"),
                &error,
                &anchored_dest,
            ),
            &temp,
            cleanup,
        ));
    }
    #[cfg(windows)]
    file.sync_all()
        .map_err(|error| format!("sync renamed stitched container failed: {error}"))?;
    drop(file);
    sync_published_parent(&anchored_dest)
        .map_err(|error| format!("sync stitched publication directory failed: {error}"))?;
    Ok(report)
}

struct RetainedPublishDirectory {
    #[cfg(windows)]
    _files: Vec<fs::File>,
}

impl RetainedPublishDirectory {
    fn open(path: &Path) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            use std::path::Component;

            const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
            const FILE_SHARE_READ: u32 = 0x0000_0001;
            const FILE_SHARE_WRITE: u32 = 0x0000_0002;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

            let mut current = PathBuf::new();
            let mut files = Vec::new();
            for component in path.components() {
                current.push(component.as_os_str());
                if matches!(component, Component::Prefix(_) | Component::RootDir) {
                    continue;
                }
                let file = OpenOptions::new()
                    .access_mode(FILE_READ_ATTRIBUTES)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&current)
                    .map_err(|error| {
                        crate::sharing_violation_holders::with_holder(
                            format!(
                                "retain stitched output directory {} failed: {error}",
                                current.display()
                            ),
                            &error,
                            &current,
                        )
                    })?;
                let metadata = file.metadata().map_err(|error| {
                    format!(
                        "inspect retained stitched directory {} failed: {error}",
                        current.display()
                    )
                })?;
                if !metadata.is_dir()
                    || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                {
                    return Err(format!(
                        "stitched output directory component is not a regular no-follow directory: {}",
                        current.display()
                    ));
                }
                files.push(file);
            }
            if files.is_empty() {
                return Err("stitched output directory may not be a filesystem root".into());
            }
            Ok(Self { _files: files })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Ok(Self {})
        }
    }
}

#[cfg(test)]
pub fn publish_stitched_container<S: StitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
    dest: &Path,
) -> Result<PublishReport, String> {
    let (bytes, mut report) = pack_and_validate(plan, source)?;

    let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory failed: {error}"))?;
    }
    let file_name = dest
        .file_name()
        .ok_or_else(|| "stitched publication destination has no file name".to_string())?;
    let canonical_parent = fs::canonicalize(parent.unwrap_or_else(|| Path::new(".")))
        .map_err(|error| format!("anchor stitched output directory failed: {error}"))?;
    let anchored_dest = canonical_parent.join(file_name);
    reject_reparse_destination(&anchored_dest)?;
    let (temp, mut file) = create_unique_sibling_temp(&anchored_dest)?;

    let write_result = (|| -> Result<(), String> {
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let mut persisted = Vec::with_capacity(bytes.len());
        file.read_to_end(&mut persisted)
            .map_err(|error| error.to_string())?;
        if persisted != bytes {
            return Err("stitched temp readback differs from packed bytes".to_string());
        }
        let persisted_container = Container::from_bytes(persisted, &ExpectedIdentity::default())
            .map_err(|error| format!("persisted stitched temp failed to re-open: {error}"))?;
        persisted_container.verify_all().map_err(|error| {
            format!("persisted stitched temp payload verification failed: {error}")
        })?;
        Ok(())
    })();
    if let Err(error) = write_result {
        drop(file);
        let cleanup = fs::remove_file(&temp);
        return Err(with_cleanup_note(
            format!("write stitched temp container failed: {error}"),
            &temp,
            cleanup,
        ));
    }
    drop(file);

    if let Err(error) = atomic_replace(&temp, &anchored_dest) {
        let cleanup = fs::remove_file(&temp);
        return Err(with_cleanup_note(
            format!("atomic publish (rename) failed: {error}"),
            &temp,
            cleanup,
        ));
    }
    sync_published_parent(&anchored_dest)
        .map_err(|error| format!("sync stitched publication directory failed: {error}"))?;

    let published = fs::read(&anchored_dest)
        .map_err(|error| format!("re-open published stitched container failed: {error}"))?;
    let published_sha256 = Sha256::hex(&published);
    if published_sha256 != report.container_sha256 || published.len() != bytes.len() {
        return Err(format!(
            "published stitched destination failed byte identity: expected {} bytes/{}, got {} bytes/{published_sha256}",
            bytes.len(), report.container_sha256, published.len()
        ));
    }
    let published_container = Container::from_bytes(published, &ExpectedIdentity::default())
        .map_err(|error| format!("published stitched destination failed to re-open: {error}"))?;
    published_container
        .verify_all()
        .map_err(|error| format!("published stitched destination verification failed: {error}"))?;

    report.published_path = dest.to_path_buf();
    report.container_sha256 = published_sha256;
    Ok(report)
}

fn reject_reparse_destination(dest: &Path) -> Result<(), String> {
    match fs::symlink_metadata(dest) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err("stitched publication refuses a symbolic-link destination".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err("stitched publication refuses a reparse-point destination".into());
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "inspect stitched publication destination failed: {error}"
        )),
    }
}

#[cfg(not(windows))]
fn create_unique_sibling_temp(dest: &Path) -> Result<(PathBuf, fs::File), String> {
    for attempt in 0..128u32 {
        let temp = sibling_temp_path(dest, attempt);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(crate::sharing_violation_holders::with_holder(
                    format!("create unique stitched temp failed: {error}"),
                    &error,
                    &temp,
                ))
            }
        }
    }
    Err("could not allocate a unique stitched temp after 128 attempts".into())
}

#[cfg(windows)]
fn create_unique_sibling_temp(dest: &Path) -> Result<(PathBuf, fs::File), String> {
    use std::os::windows::fs::OpenOptionsExt;

    const DELETE_ACCESS: u32 = 0x0001_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    for attempt in 0..128u32 {
        let temp = sibling_temp_path(dest, attempt);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE_ACCESS)
            .share_mode(FILE_SHARE_READ)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(crate::sharing_violation_holders::with_holder(
                    format!("create unique stitched temp failed: {error}"),
                    &error,
                    &temp,
                ))
            }
        }
    }
    Err("could not allocate a unique stitched temp after 128 attempts".into())
}

#[cfg(not(windows))]
fn sync_published_parent(dest: &Path) -> std::io::Result<()> {
    fs::File::open(dest.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(windows)]
fn sync_published_parent(_dest: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, dest: &Path) -> std::io::Result<()> {
    fs::rename(temp, dest)
}

#[cfg(not(windows))]
fn atomic_replace_retained(
    _verified_file: &fs::File,
    _temp: &Path,
    _dest: &Path,
    _destination_guard: &RetainedPublishDirectory,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "exact retained-handle replacement is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn atomic_replace_retained(
    verified_file: &fs::File,
    _temp: &Path,
    dest: &Path,
    _destination_guard: &RetainedPublishDirectory,
) -> std::io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileRenameInfo {
        replace_if_exists: u32,
        root_directory: *mut c_void,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            file: *mut c_void,
            information_class: i32,
            information: *const c_void,
            information_bytes: u32,
        ) -> i32;
    }

    const FILE_RENAME_INFO_CLASS: i32 = 3;
    let wide = dest.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.is_empty() || wide.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic publish destination is empty or contains NUL",
        ));
    }
    let name_bytes = wide
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "atomic publish destination is too long",
            )
        })?;
    let info_bytes = std::mem::size_of::<FileRenameInfo>()
        .checked_add((wide.len() - 1) * std::mem::size_of::<u16>())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "atomic publish rename buffer overflow",
            )
        })?;
    let info_bytes_u32 = u32::try_from(info_bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic publish rename buffer is too large",
        )
    })?;
    let word_bytes = std::mem::size_of::<usize>();
    let word_count = info_bytes.checked_add(word_bytes - 1).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic publish rename allocation overflow",
        )
    })? / word_bytes;
    let mut storage = vec![0usize; word_count];
    let info = storage.as_mut_ptr().cast::<FileRenameInfo>();
    let ok = unsafe {
        (*info).replace_if_exists = 1;
        (*info).root_directory = std::ptr::null_mut();
        (*info).file_name_length = name_bytes;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), (*info).file_name.as_mut_ptr(), wide.len());
        SetFileInformationByHandle(
            verified_file.as_raw_handle(),
            FILE_RENAME_INFO_CLASS,
            info.cast(),
            info_bytes_u32,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn atomic_replace(temp: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    fn wide(path: &Path) -> std::io::Result<Vec<u16>> {
        let mut out: Vec<u16> = path.as_os_str().encode_wide().collect();
        if out.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "atomic publish path contains an embedded NUL",
            ));
        }
        out.push(0);
        Ok(out)
    }

    let existing = wide(temp)?;
    let replacement = wide(dest)?;
    let ok = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn sibling_temp_path(dest: &Path, attempt: u32) -> PathBuf {
    let nonce = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let file_name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stitched".to_string());
    let temp_name = format!(".{file_name}.stitch-tmp-{nonce}.{attempt}");
    match dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent.join(temp_name),
        None => PathBuf::from(temp_name),
    }
}

fn with_cleanup_note(primary: String, temp: &Path, cleanup: std::io::Result<()>) -> String {
    match cleanup {
        Ok(()) => primary,
        Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => primary,
        Err(remove_error) => format!(
            "{primary}; ALSO failed to remove temp artifact {}: {remove_error} \
             (a partial temp file remains and must be cleaned up manually)",
            temp.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::stitching::{
        plan_stitch, AppendTicket, CampaignIdentityResolution, CompatibilityResolution,
        CoverageDisposition, HourObservation, ObservationOwner, RawSourceDescriptor,
        RawSourceStorage, SegmentInput, SegmentProvenance, SourceParticipation, StitchOutcome,
        StitchRequiredInputs, STITCH_ENTRY_ORDER,
    };

    const GAME: &str = "g";
    const MODSET: &str = "MS";
    const K: &[u8] = b"campaign-A";

    fn snap_payload(play_order: u64, hour: i64) -> Vec<u8> {
        format!(
            r#"{{"schema_version":"1","game_id":"{GAME}","hour_index":{hour},"provenance":{{"mod":{{"modset_id":"{MODSET}"}}}},"po":{play_order},"countries":{{}},"states":{{}}}}"#
        )
        .into_bytes()
    }

    fn raw_payload(play_order: u64, len: usize) -> Vec<u8> {
        vec![(play_order as u8).wrapping_add(1); len]
    }

    fn stitched_wire_test_catalog() -> wire_product::AuthenticatedProvinceCatalog {
        use portable_content_manifest::{
            ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer,
            RequiredContent, RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
            UsedContentKey,
        };

        let definition = b"1;1;2;3;land;false;plains;1\n".to_vec();
        let definition_sha = Sha256::hex(&definition);
        let vfs_sha = "f".repeat(64);
        let mut manifest = ContentManifest {
            schema: portable_content_manifest::SCHEMA.to_string(),
            product_modset_sha256: "9".repeat(64),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "a".repeat(64),
                build_profile: "a729-test".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "test-profile".into(),
                playset_id: "test-playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: vfs_sha.clone(),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest.required_content.push(RequiredContent {
            content_key: "map/definition.csv".into(),
            kind: RequiredContentKind::Definition,
            content: UsedContentKey {
                effective_vfs_manifest_sha256: vfs_sha,
                normalized_game_relative_path: "map/definition.csv".into(),
                winning_layer_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: definition_sha.clone(),
                payload_sha256: definition_sha,
                payload_bytes: definition.len() as u64,
            },
            observation_count: 1,
            consumers: vec!["wire_product.d81".into()],
        });
        manifest = manifest.with_computed_digest().unwrap();
        wire_product::AuthenticatedProvinceCatalog::from_content_manifest(
            &manifest.encode_json().unwrap(),
            &definition,
        )
        .unwrap()
    }

    fn push_stitched_wire_record(
        out: &mut Vec<u8>,
        sequence: u64,
        tick: i32,
        domain: u16,
        record_type: u16,
        payload: &[u8],
    ) -> (u64, u64) {
        let start = out.len() as u64;
        out.extend_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&tick.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        (start, out.len() as u64)
    }

    fn stitched_wire_frame_end_payload(emitted: u32, status: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn append_stitched_d81_hour(
        out: &mut Vec<u8>,
        sequence: &mut u64,
        tick: i32,
    ) -> ((u64, u64), (u64, u64)) {
        append_stitched_d81_hour_fixture(out, sequence, tick, false)
    }

    fn append_stitched_d81_hour_with_country_roster(
        out: &mut Vec<u8>,
        sequence: &mut u64,
        tick: i32,
    ) -> ((u64, u64), (u64, u64)) {
        append_stitched_d81_hour_fixture(out, sequence, tick, true)
    }

    fn append_stitched_d81_hour_fixture(
        out: &mut Vec<u8>,
        sequence: &mut u64,
        tick: i32,
        include_country_roster: bool,
    ) -> ((u64, u64), (u64, u64)) {
        let mut perf = vec![0u8; 40];
        perf[32..36].copy_from_slice(&0u32.to_le_bytes());
        let mut first = None;
        if include_country_roster {
            first = Some(push_stitched_wire_record(
                out,
                *sequence,
                tick,
                18,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &perf,
            ));
            *sequence += 1;
            let mut countries = Vec::new();
            countries.extend_from_slice(&0u64.to_le_bytes());
            countries.extend_from_slice(&1u32.to_le_bytes());
            countries.extend_from_slice(&1u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            compact_contract::country_name_inputs::encode_domain_version(18, &mut countries);
            countries.extend_from_slice(&1u32.to_le_bytes());
            countries.extend_from_slice(&3u16.to_le_bytes());
            countries.extend_from_slice(b"TST");
            countries.push(1);
            countries.extend_from_slice(&[0, 1]);
            countries.extend_from_slice(compact_contract::country_identity_membership::CIM1_MAGIC);
            countries.extend_from_slice(
                &compact_contract::country_identity_membership::CIM1_VERSION.to_le_bytes(),
            );
            countries.extend_from_slice(
                &(compact_contract::country_identity_membership::CIM1_HEADER_LEN as u16)
                    .to_le_bytes(),
            );
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&1u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&0u32.to_le_bytes());
            countries.extend_from_slice(&1u32.to_le_bytes());
            countries.push(0);
            countries.extend_from_slice(&[0, 0, 0]);
            push_stitched_wire_record(
                out,
                *sequence,
                tick,
                18,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &countries,
            );
            *sequence += 1;
        }
        let d81_first = push_stitched_wire_record(
            out,
            *sequence,
            tick,
            compact_contract::rail_replay::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf,
        );
        let first = first.unwrap_or(d81_first);
        *sequence += 1;
        let province = compact_contract::rail_replay::RailProvinceState {
            province_id: 1,
            active_terminal: true,
            anchor_variant: 0,
            anchor_bits: [1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits()],
            neighbors: Vec::new(),
        };
        let data = compact_contract::rail_replay::encode_payload_with_hub_simulation(
            &[province],
            &[compact_contract::rail_replay::RailHubSimulationState {
                pov_country_id: 1,
                province_id: 1,
                province_owner_country_id: 1,
                subtype: 0,
                route_class: 2,
                node_kind: 1,
                node_flag: 0,
                available_raw: 1_000_000,
                consumed_raw: 250_000,
                foreign_max_raw: 1_000_000,
                foreign_route: false,
                fallback_record: false,
            }],
            1_000,
            0,
            1,
            0,
        )
        .unwrap();
        push_stitched_wire_record(
            out,
            *sequence,
            tick,
            compact_contract::rail_replay::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data,
        );
        *sequence += 1;
        let last = push_stitched_wire_record(
            out,
            *sequence,
            tick,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            &stitched_wire_frame_end_payload(if include_country_roster { 2 } else { 1 }, 0),
        );
        *sequence += 1;
        (first, last)
    }

    fn append_stitched_d81_gap(
        out: &mut Vec<u8>,
        sequence: &mut u64,
        gap: compact_contract::DomainGapRecordV1,
    ) -> ((u64, u64), (u64, u64)) {
        let tick = gap.target_hour_raw;
        let first = push_stitched_wire_record(
            out,
            *sequence,
            tick,
            compact_contract::rail_replay::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
            &gap.encode().unwrap(),
        );
        *sequence += 1;
        let last = push_stitched_wire_record(
            out,
            *sequence,
            tick,
            compact_contract::COMPACT_DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            &stitched_wire_frame_end_payload(1, 0),
        );
        *sequence += 1;
        (first, last)
    }

    fn stitched_valid_hour(offset: i32) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24 + offset
    }

    fn stitch_inputs() -> StitchRequiredInputs {
        StitchRequiredInputs {
            schema_version: "1".into(),
            fold_version: "wf-v1".into(),
            canonical_writer_version: "canon-v1".into(),
            packer_codec_version: "zip-v1".into(),
            created_utc: "2026-07-12T00:00:00Z".into(),
            compression_method: "store".into(),
            compression_parameters: BTreeMap::from([("level".into(), "none".into())]),
            entry_order: STITCH_ENTRY_ORDER.into(),
        }
    }

    fn snap_seg(play_order: u64, range: std::ops::RangeInclusive<i64>) -> SegmentInput {
        let hours: Vec<i64> = range.collect();
        let ticket = AppendTicket {
            campaign_key: K.to_vec(),
            play_order,
            session_nonce: format!("nonce-{play_order}"),
            process_start_identity: format!("pid-{play_order}"),
        };
        let epoch_token = format!("epoch-{play_order}");
        let owner = ObservationOwner {
            ticket: ticket.clone(),
            in_session_ordinal: 0,
            epoch_token: epoch_token.clone(),
        };
        let mut coverage = BTreeMap::new();
        let mut observations = BTreeMap::new();
        for (index, h) in hours.iter().copied().enumerate() {
            let disposition = CoverageDisposition::Snapshot {
                sha256: Sha256::hex(&snap_payload(play_order, h)),
                date: Some(format!("1936.1.1.{h}")),
            };
            coverage.insert(h, disposition.clone());
            observations.insert(
                h,
                vec![HourObservation {
                    observation_id: format!("obs-{play_order}-{h}"),
                    owner: owner.clone(),
                    record_seq_bounds: ((index as u64) * 2, (index as u64) * 2 + 1),
                    byte_bounds: ((index as u64) * 100, (index as u64 + 1) * 100),
                    disposition,
                }],
            );
        }
        let record_end = (hours.len() as u64) * 2 - 1;
        let byte_end = (hours.len() as u64) * 100;
        let source_path = format!("sessions/p{play_order:08}/collector_stream.h4cr");
        let source_digest = Sha256::hex(&raw_payload(play_order, byte_end as usize));
        SegmentInput {
            play_order,
            in_session_ordinal: 0,
            identity: CampaignIdentityResolution::Verified {
                scheme_major: 1,
                campaign_key: K.to_vec(),
                evidence_digest: [0u8; 32],
            },
            ticket: Some(ticket.clone()),
            compatibility: CompatibilityResolution::Proven {
                evidence_id: format!("compat-{play_order}"),
                compatibility_class: "decoder-v1".into(),
                build_exe_sha256: "ab".repeat(32),
                modset_id: MODSET.into(),
                schema_version: "1".into(),
                static_catalog_digest: "cd".repeat(32),
            },
            transition_coverage: BTreeMap::new(),
            coverage,
            observations,
            raw_source: Some(RawSourceDescriptor {
                session_nonce: format!("nonce-{play_order}"),
                process_start_identity: format!("pid-{play_order}"),
                source_path: source_path.clone(),
                stored_digest: source_digest.clone(),
                decoded_digest: source_digest.clone(),
                record_seq_bounds: (0, record_end),
                byte_bounds: (0, byte_end),
                storage: RawSourceStorage::EmbeddedCampaign {
                    campaign_key: K.to_vec(),
                },
                participations: vec![SourceParticipation {
                    ticket,
                    scheme_major: 1,
                    identity_evidence_digest: [0u8; 32],
                    record_seq_bounds: (0, record_end),
                    byte_bounds: (0, byte_end),
                }],
            }),
            provenance: SegmentProvenance {
                session_nonce: format!("nonce-{play_order}"),
                process_start_identity: format!("pid-{play_order}"),
                epoch_token,
                stream_major: 2,
                build_exe_sha256: Some("ab".repeat(32)),
                modset_id: Some(MODSET.into()),
                schema_version: Some("1".into()),
                static_catalog_digest: Some("cd".repeat(32)),
                source_path: Some(source_path),
                source_stored_digest: Some(source_digest.clone()),
                source_decoded_digest: Some(source_digest),
                record_seq_bounds: Some((0, record_end)),
                byte_bounds: Some((0, byte_end)),
                ..Default::default()
            },
        }
    }

    fn rollback_plan() -> StitchedPlan {
        match plan_stitch(
            GAME,
            stitch_inputs(),
            vec![snap_seg(0, 0..=3), snap_seg(1, 2..=2)],
        ) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        }
    }

    #[test]
    fn rebuilt_wire_branches_come_only_from_final_plan_evidence() {
        let plan = rollback_plan();
        let branches = derive_rebuilt_wire_branches(&plan).unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].branch, 0);
        assert_eq!(branches[0].first_tick, 0);
        assert_eq!(branches[0].parent, None);
        assert_eq!(
            branches[0].session_fingerprint,
            decode_sha256_hex(
                plan.segments[0]
                    .input
                    .raw_source
                    .as_ref()
                    .unwrap()
                    .decoded_digest
                    .as_str()
            )
            .unwrap()
        );
        assert_eq!(branches[1].branch, 1);
        assert_eq!(branches[1].first_tick, 2);
        assert_eq!(
            branches[1].parent,
            Some(wire_product::BranchParent {
                branch: 0,
                through_tick: 1,
            })
        );
    }

    #[test]
    fn rebuilt_wire_branch_derivation_skips_empty_ordinals_and_rejects_digest_drift() {
        let mut leading_empty = rollback_plan();
        leading_empty.segments[0].input.coverage.clear();
        leading_empty.segments[0].input.observations.clear();
        leading_empty.segments[1].parent_segment = None;
        let (branches, routes) = derive_rebuilt_wire_plan(&leading_empty).unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].branch, 0);
        assert_eq!(routes[0].segment_id, 1);

        let mut drifted = rollback_plan();
        drifted.segments[1].input.provenance.source_decoded_digest = Some("00".repeat(32));
        let error = derive_rebuilt_wire_branches(&drifted)
            .err()
            .expect("source digest disagreement must fail");
        assert!(error.contains("disagrees"), "{error}");
    }

    #[test]
    fn rebuilt_wire_branch_origin_includes_transition_coverage() {
        let first = snap_seg(0, 0..=0);
        let mut second = snap_seg(1, 2..=2);
        second.transition_coverage.insert(
            1,
            CoverageDisposition::Gap {
                reason: "cross-session forward hour unavailable".into(),
                evidence_id: "transition-gap-1".into(),
            },
        );
        let plan = match plan_stitch(GAME, stitch_inputs(), vec![first, second]) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        };
        let branches = derive_rebuilt_wire_branches(&plan).unwrap();
        assert_eq!(branches[1].first_tick, 1);
        assert_eq!(
            branches[1].parent,
            Some(wire_product::BranchParent {
                branch: 0,
                through_tick: 0,
            })
        );
    }

    struct MemorySource {
        snapshots: BTreeMap<(u32, i64), Vec<u8>>,
        raw: BTreeMap<String, Vec<u8>>,
    }

    impl MemorySource {
        fn for_plan(plan: &StitchedPlan) -> MemorySource {
            let mut snapshots = BTreeMap::new();
            let mut raw = BTreeMap::new();
            for segment in &plan.segments {
                let po = segment.input.play_order;
                for (hour, disposition) in segment.input.coverage.iter() {
                    if disposition.is_snapshot() {
                        snapshots.insert((segment.segment_id, *hour), snap_payload(po, *hour));
                    }
                }
                let source = segment.input.raw_source.as_ref().unwrap();
                let len = (source.byte_bounds.1 - source.byte_bounds.0) as usize;
                raw.insert(source.source_path.clone(), raw_payload(po, len));
            }
            MemorySource { snapshots, raw }
        }
    }

    impl StitchedPayloadSource for MemorySource {
        fn snapshot_bytes(&self, segment_id: u32, hour_index: i64) -> Result<Vec<u8>, String> {
            self.snapshots
                .get(&(segment_id, hour_index))
                .cloned()
                .ok_or_else(|| format!("no snapshot for segment {segment_id} hour {hour_index}"))
        }
        fn raw_source_bytes(&self, source_path: &str) -> Result<Vec<u8>, String> {
            self.raw
                .get(source_path)
                .cloned()
                .ok_or_else(|| format!("no raw source {source_path}"))
        }
    }

    impl RetainedStitchedPayloadSource for MemorySource {
        fn snapshot_receipt(
            &self,
            segment_id: u32,
            hour_index: i64,
        ) -> Result<VerifiedPayloadReceipt, String> {
            let bytes = self
                .snapshots
                .get(&(segment_id, hour_index))
                .ok_or_else(|| format!("no snapshot for segment {segment_id} hour {hour_index}"))?;
            Ok(VerifiedPayloadReceipt {
                bytes: bytes.len() as u64,
                sha256: Sha256::hex(bytes),
            })
        }

        fn copy_snapshot_to_unpublished(
            &self,
            segment_id: u32,
            hour_index: i64,
            writer: &mut dyn Write,
        ) -> Result<VerifiedPayloadReceipt, String> {
            let bytes = self
                .snapshots
                .get(&(segment_id, hour_index))
                .ok_or_else(|| format!("no snapshot for segment {segment_id} hour {hour_index}"))?;
            writer.write_all(bytes).map_err(|error| error.to_string())?;
            self.snapshot_receipt(segment_id, hour_index)
        }

        fn raw_source_receipt(&self, source_path: &str) -> Result<VerifiedPayloadReceipt, String> {
            let bytes = self
                .raw
                .get(source_path)
                .ok_or_else(|| format!("no raw source {source_path}"))?;
            Ok(VerifiedPayloadReceipt {
                bytes: bytes.len() as u64,
                sha256: Sha256::hex(bytes),
            })
        }

        fn copy_raw_source_to_unpublished(
            &self,
            source_path: &str,
            writer: &mut dyn Write,
        ) -> Result<VerifiedPayloadReceipt, String> {
            let bytes = self
                .raw
                .get(source_path)
                .ok_or_else(|| format!("no raw source {source_path}"))?;
            writer.write_all(bytes).map_err(|error| error.to_string())?;
            self.raw_source_receipt(source_path)
        }
    }

    struct DriftingRetainedSource(MemorySource);

    impl RetainedStitchedPayloadSource for DriftingRetainedSource {
        fn snapshot_receipt(
            &self,
            segment_id: u32,
            hour_index: i64,
        ) -> Result<VerifiedPayloadReceipt, String> {
            self.0.snapshot_receipt(segment_id, hour_index)
        }

        fn copy_snapshot_to_unpublished(
            &self,
            segment_id: u32,
            hour_index: i64,
            writer: &mut dyn Write,
        ) -> Result<VerifiedPayloadReceipt, String> {
            writer
                .write_all(b"payload-drifted-after-receipt")
                .map_err(|error| error.to_string())?;
            self.0.snapshot_receipt(segment_id, hour_index)
        }

        fn raw_source_receipt(&self, source_path: &str) -> Result<VerifiedPayloadReceipt, String> {
            self.0.raw_source_receipt(source_path)
        }

        fn copy_raw_source_to_unpublished(
            &self,
            source_path: &str,
            writer: &mut dyn Write,
        ) -> Result<VerifiedPayloadReceipt, String> {
            self.0.copy_raw_source_to_unpublished(source_path, writer)
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "hoi4_stitch_publish_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rebuilt_wire_source_receipt_rejects_path_drift() {
        let dir = TempDir::new("wire_source_receipt");
        let path = dir.0.join("collector_stream.bin");
        let original = b"exact retained stored bytes";
        fs::write(&path, original).unwrap();
        let session = OwnedStitchedWireSession {
            branch: 0,
            source_path: "raw/session.h4cr".to_string(),
            identity_mode: compact_contract::stream_validator::StreamIdentityMode::Legacy,
            file_path: path.clone(),
            file: portable_test_fs_retry::open(&path).unwrap(),
            stored_sha256: decode_sha256_hex(&Sha256::hex(original)).unwrap(),
            selection: wire_product::H4crSelection {
                first_sequence: 0,
                last_sequence: 0,
                start_byte: 0,
                end_byte: 0,
            },
        };
        verify_rebuilt_wire_source_receipts(std::slice::from_ref(&session)).unwrap();

        fs::write(&path, b"different stored bytes").unwrap();
        assert!(verify_rebuilt_wire_source_receipts(&[session])
            .unwrap_err()
            .contains("stored SHA-256 differs"));
    }

    #[test]
    fn stitched_branch_origin_uses_initial_authenticated_h4dg_coverage() {
        let h0 = stitched_valid_hour(0);
        let h2 = stitched_valid_hour(2);
        let h3 = stitched_valid_hour(3);
        let h2_width = u64::try_from(h2 - h0 + 1).unwrap();
        let mut stream = Vec::new();
        let mut sequence = 1u64;
        let h2_first_sequence = sequence;
        let (h2_first, h2_last) = append_stitched_d81_gap(
            &mut stream,
            &mut sequence,
            compact_contract::DomainGapRecordV1 {
                part: compact_contract::DomainGapPart::State,
                reason: compact_contract::DomainGapReason::Overrun,
                target_hour_raw: h2,
                range_start_hour_raw: h0,
                range_end_hour_raw: h2,
                tick_before: 0,
                tick_after: h2_width,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: 0,
            },
        );
        let h2_last_sequence = sequence - 1;
        let h3_first_sequence = sequence;
        // The country roster rides WITH the hour: the identity-portability
        // oracle requires every d81 country id to be vouched by a walked D18
        // roster, and this synthetic branch's only roster is its own.
        let (h3_first, h3_last) =
            append_stitched_d81_hour_with_country_roster(&mut stream, &mut sequence, h3);
        let h3_last_sequence = sequence - 1;
        let digest = Sha256::hex(&stream);

        let mut segment = snap_seg(0, i64::from(h2)..=i64::from(h3));
        let h2_observation = segment
            .observations
            .get_mut(&i64::from(h2))
            .unwrap()
            .first_mut()
            .unwrap();
        h2_observation.record_seq_bounds = (h2_first_sequence, h2_last_sequence);
        h2_observation.byte_bounds = (h2_first.0, h2_last.1);
        let h3_observation = segment
            .observations
            .get_mut(&i64::from(h3))
            .unwrap()
            .first_mut()
            .unwrap();
        h3_observation.record_seq_bounds = (h3_first_sequence, h3_last_sequence);
        h3_observation.byte_bounds = (h3_first.0, h3_last.1);
        segment.provenance.source_stored_digest = Some(digest.clone());
        segment.provenance.source_decoded_digest = Some(digest.clone());
        segment.provenance.record_seq_bounds = Some((1, sequence - 1));
        segment.provenance.byte_bounds = Some((0, stream.len() as u64));
        let raw = segment.raw_source.as_mut().unwrap();
        raw.stored_digest = digest.clone();
        raw.decoded_digest = digest;
        raw.record_seq_bounds = (1, sequence - 1);
        raw.byte_bounds = (0, stream.len() as u64);
        raw.participations[0].record_seq_bounds = raw.record_seq_bounds;
        raw.participations[0].byte_bounds = raw.byte_bounds;
        let plan = match plan_stitch(GAME, stitch_inputs(), vec![segment]) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        };

        let dir = TempDir::new("wire_initial_coalesced_gap");
        let path = dir.0.join("initial-gap.h4cr");
        fs::write(&path, &stream).unwrap();
        let session = OwnedStitchedWireSession {
            branch: 0,
            source_path: plan.segments[0]
                .input
                .raw_source
                .as_ref()
                .unwrap()
                .source_path
                .clone(),
            identity_mode: compact_contract::stream_validator::StreamIdentityMode::Legacy,
            file_path: path.clone(),
            file: portable_test_fs_retry::open(&path).unwrap(),
            stored_sha256: decode_sha256_hex(&Sha256::hex(&stream)).unwrap(),
            selection: wire_product::H4crSelection {
                first_sequence: 1,
                last_sequence: sequence - 1,
                start_byte: 0,
                end_byte: stream.len() as u64,
            },
        };
        let (branches, routes, first_ticks, _unreconstructed_records) =
            derive_rebuilt_wire_plan_from_sessions(&plan, None, &[session]).unwrap();
        assert_eq!(branches[0].first_tick, u64::try_from(h0).unwrap());
        assert_eq!(first_ticks.get(&0), Some(&u64::try_from(h0).unwrap()));
        assert!(
            derive_rebuilt_wire_leading_gaps(&plan, &routes, &first_ticks)
                .unwrap()
                .is_empty(),
            "authenticated H4DG coverage must not be duplicated as a transition gap"
        );
    }

    #[test]
    fn stitched_retained_selection_directly_refolds_exact_descriptors_and_gaps() {
        let h9 = stitched_valid_hour(9);
        let h10 = stitched_valid_hour(10);
        let h11 = stitched_valid_hour(11);
        let h12 = stitched_valid_hour(12);
        let mut stream = Vec::new();
        let mut sequence = 1u64;
        // Roster rides with the hour (identity-portability oracle).
        append_stitched_d81_hour_with_country_roster(&mut stream, &mut sequence, h9);
        let (first, _) =
            append_stitched_d81_hour_with_country_roster(&mut stream, &mut sequence, h10);
        append_stitched_d81_gap(
            &mut stream,
            &mut sequence,
            compact_contract::DomainGapRecordV1 {
                part: compact_contract::DomainGapPart::State,
                reason: compact_contract::DomainGapReason::ClockUnreadable,
                target_hour_raw: h11,
                range_start_hour_raw: h11,
                range_end_hour_raw: h11,
                tick_before: 0,
                tick_after: 0,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: 0,
            },
        );
        let (_, last) = append_stitched_d81_gap(
            &mut stream,
            &mut sequence,
            compact_contract::DomainGapRecordV1 {
                part: compact_contract::DomainGapPart::State,
                reason: compact_contract::DomainGapReason::WorkerPanic,
                target_hour_raw: h12,
                range_start_hour_raw: h12,
                range_end_hour_raw: h12,
                tick_before: 0,
                tick_after: 0,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: 1,
            },
        );
        let selection = wire_product::H4crSelection {
            // The rostered hours carry two extra D18 records each, so h10's
            // first record is sequence 6 (Codex review round 31, defect 2).
            first_sequence: 6,
            last_sequence: sequence - 1,
            start_byte: first.0,
            end_byte: last.1,
        };
        assert_eq!((selection.first_sequence, selection.last_sequence), (6, 14));
        assert!(selection.start_byte < selection.end_byte);
        let dir = TempDir::new("wire_selected_refold");
        let source_path = dir.0.join("selected.h4cr");
        fs::write(&source_path, &stream).unwrap();
        let session = OwnedStitchedWireSession {
            branch: 0,
            source_path: "raw/selected.h4cr".into(),
            identity_mode: compact_contract::stream_validator::StreamIdentityMode::Legacy,
            file_path: source_path.clone(),
            file: portable_test_fs_retry::open(&source_path).unwrap(),
            stored_sha256: decode_sha256_hex(&Sha256::hex(&stream)).unwrap(),
            selection,
        };
        let branches = [wire_product::BranchSpec {
            branch: 0,
            parent: None,
            first_tick: u64::try_from(h9).unwrap(),
            session_fingerprint: session.stored_sha256,
        }];
        let leading = [wire_product::D81LeadingGap::new(
            0,
            u64::try_from(h9).unwrap(),
            u64::try_from(h10).unwrap(),
            wire_product::D81LeadingGapKind::ExplicitGap,
        )
        .unwrap()];
        let catalog = stitched_wire_test_catalog();
        let sessions = borrowed_wire_sessions(std::slice::from_ref(&session));
        let bundle = wire_product::build_d81_bundle_with_leading_gaps(
            wire_product::BundleBuildConfig::default(),
            &branches,
            &catalog,
            &sessions,
            &leading,
        )
        .unwrap();
        let mut descriptors = bundle
            .entries()
            .iter()
            .map(wire_product::BundleEntryDescriptor::from)
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
        let artifacts = bundle
            .entries()
            .iter()
            .map(|entry| (entry.path.clone(), entry.file_path.clone()))
            .collect::<BTreeMap<_, _>>();
        let verified = wire_product::verify_entry_source_against_h4cr(
            &descriptors,
            |path| {
                portable_test_fs_retry::read(
                    artifacts
                        .get(path)
                        .ok_or_else(|| format!("missing stitched artifact {path}"))?,
                )
                .map_err(|error| error.to_string())
            },
            wire_product::BundleBuildConfig::default(),
            &branches,
            &catalog,
            &sessions,
            &leading,
        )
        .unwrap();
        assert_eq!(verified.branches[0].state_observations, 1);
        assert_eq!(verified.branches[0].gap_hours, 3);
        assert_eq!(verified.branches[0].last_tick, u64::try_from(h12).unwrap());

        let bad_session = OwnedStitchedWireSession {
            selection: wire_product::H4crSelection {
                last_sequence: selection.last_sequence - 1,
                ..selection
            },
            file: session.file.try_clone().unwrap(),
            branch: session.branch,
            source_path: session.source_path.clone(),
            identity_mode: session.identity_mode,
            file_path: session.file_path.clone(),
            stored_sha256: session.stored_sha256,
        };
        let bad_sessions = borrowed_wire_sessions(std::slice::from_ref(&bad_session));
        let error = wire_product::verify_entry_source_against_h4cr(
            &descriptors,
            |path| {
                portable_test_fs_retry::read(artifacts.get(path).unwrap())
                    .map_err(|error| error.to_string())
            },
            wire_product::BundleBuildConfig::default(),
            &branches,
            &catalog,
            &bad_sessions,
            &leading,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "H4CR selection byte end disagrees with last sequence"
        );
    }

    #[cfg(windows)]
    #[test]
    fn publishes_multi_segment_session_and_reopens() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("roundtrip");
        let dest = dir.0.join("replay.battleplan");

        let report = publish_stitched_container_retained(&plan, &source, &dest).unwrap();
        assert_eq!(report.published_path, dest);
        assert!(dest.exists());
        assert_eq!(report.visible_snapshot_hours, 3);
        assert_eq!(report.segments, 2);
        let leftovers: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp artifact leaked on success");

        let bytes = portable_test_fs_retry::read(&dest).unwrap();
        let container = Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap();
        assert!(container.index().is_stitched());
        let h2 = container
            .index()
            .snapshots
            .iter()
            .find(|s| s.hour_index == Some(2))
            .unwrap();
        assert_eq!(h2.file, "s00000001.000000000002.snapshot.json");
        assert_eq!(
            container
                .load(2)
                .unwrap()
                .rest
                .get("po")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert!(container
            .load_file("s00000000.000000000002.snapshot.json")
            .is_ok());
        assert!(container
            .load_file("s00000000.000000000003.snapshot.json")
            .is_ok());
        container.verify_all().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn retained_publication_is_byte_deterministic_and_leaves_no_temp() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("retained_roundtrip");
        let first_path = dir.0.join("first.battleplan");
        let second_path = dir.0.join("second.battleplan");

        let first = publish_stitched_container_retained(&plan, &source, &first_path).unwrap();
        let second = publish_stitched_container_retained(&plan, &source, &second_path).unwrap();
        assert_eq!(first.container_sha256, second.container_sha256);
        assert_eq!(first.container_bytes, second.container_bytes);
        // This test is about the published bytes and the temps left behind,
        // not about the retained open's path sharing; the handle keeps it off
        // a race it does not control.
        let published = fs::File::open(&first_path).expect("open the published container");
        let (container, _, _) =
            Container::open_strict_with_file_receipt_from_file(published, &ExpectedIdentity::default())
                .unwrap();
        container.verify_all_streaming().unwrap();
        assert!(container.index().is_stitched());
        let leftovers = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "retained publication leaked temp files"
        );
    }

    #[cfg(windows)]
    #[test]
    fn retained_source_drift_preserves_prior_replay_and_leaves_no_temp() {
        let plan = rollback_plan();
        let dir = TempDir::new("retained_drift");
        let dest = dir.0.join("replay.battleplan");
        let source = MemorySource::for_plan(&plan);
        publish_stitched_container_retained(&plan, &source, &dest).unwrap();
        let prior = portable_test_fs_retry::read(&dest).unwrap();

        let drifting = DriftingRetainedSource(MemorySource::for_plan(&plan));
        let error = publish_stitched_container_retained(&plan, &drifting, &dest).unwrap_err();
        assert!(
            error.contains("length") || error.contains("sha256") || error.contains("hash"),
            "unexpected drift failure: {error}"
        );
        assert_eq!(
            portable_test_fs_retry::read(&dest).unwrap(),
            prior,
            "prior replay was replaced"
        );
        let leftovers = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty(), "retained failure leaked temp files");
    }

    #[cfg(not(windows))]
    #[test]
    fn retained_publication_fails_closed_without_handle_replace() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("retained_unsupported");
        let dest = dir.0.join("replay.battleplan");
        fs::write(&dest, b"prior-replay-sentinel").unwrap();

        let error = publish_stitched_container_retained(&plan, &source, &dest).unwrap_err();
        assert!(error.contains("retained-handle replacement"), "{error}");
        assert_eq!(
            portable_test_fs_retry::read(&dest).unwrap(),
            b"prior-replay-sentinel"
        );
        let leftovers = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "unsupported publication leaked temp files"
        );
    }

    #[cfg(windows)]
    #[test]
    fn atomic_publish_replaces_an_existing_verified_destination() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("replace_existing");
        let dest = dir.0.join("replay.battleplan");

        publish_stitched_container_retained(&plan, &source, &dest).unwrap();
        let first = portable_test_fs_retry::read(&dest).unwrap();
        publish_stitched_container_retained(&plan, &source, &dest).unwrap();
        let second = portable_test_fs_retry::read(&dest).unwrap();
        assert_eq!(second, first);
        assert!(Container::from_bytes(second, &ExpectedIdentity::default()).is_ok());
    }

    #[test]
    fn packing_is_byte_deterministic() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let (a, _) = pack_and_validate(&plan, &source).unwrap();
        let (b, _) = pack_and_validate(&plan, &source).unwrap();
        assert_eq!(a, b, "same plan + payloads -> byte-identical container");
    }

    #[test]
    fn tampered_snapshot_payload_aborts_before_publish() {
        let plan = rollback_plan();
        let mut source = MemorySource::for_plan(&plan);
        let key = *source.snapshots.keys().next().unwrap();
        source.snapshots.insert(key, b"tampered".to_vec());
        let error = build_stitched_entries(&plan, &source)
            .err()
            .expect("tampered snapshot must abort");
        assert!(error.contains("hashes"), "{error}");
    }

    #[test]
    fn tampered_raw_source_aborts_before_publish() {
        let plan = rollback_plan();
        let mut source = MemorySource::for_plan(&plan);
        let path = source.raw.keys().next().unwrap().clone();
        source
            .raw
            .insert(path, b"different-length-and-content".to_vec());
        let error = build_stitched_entries(&plan, &source)
            .err()
            .expect("tampered raw source must abort");
        assert!(error.contains("stored digest"), "{error}");
    }

    #[test]
    fn failed_publish_preserves_prior_replay_and_leaves_no_temp() {
        let dir = TempDir::new("crash");
        let dest = dir.0.join("replay.battleplan");
        let good = rollback_plan();
        publish_stitched_container(&good, &MemorySource::for_plan(&good), &dest).unwrap();
        let prior_bytes = portable_test_fs_retry::read(&dest).unwrap();

        let plan = rollback_plan();
        let mut broken = MemorySource::for_plan(&plan);
        broken.snapshots.clear();
        let error = publish_stitched_container(&plan, &broken, &dest).unwrap_err();
        assert!(error.contains("no snapshot"), "{error}");

        assert_eq!(
            portable_test_fs_retry::read(&dest).unwrap(),
            prior_bytes,
            "prior replay was disturbed"
        );
        let leftovers: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp artifact leaked on failure");
    }

    fn compressed_source_plan() -> StitchedPlan {
        let mut seg = snap_seg(0, 0..=2);
        let decoded = Sha256::hex(b"decoded-plaintext-distinct-from-the-stored-bytes");
        seg.raw_source.as_mut().unwrap().decoded_digest = decoded.clone();
        seg.provenance.source_decoded_digest = Some(decoded);
        match plan_stitch(GAME, stitch_inputs(), vec![seg]) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        }
    }

    #[test]
    fn compressed_source_validates_stored_digest_and_reopens() {
        let plan = compressed_source_plan();
        let source = MemorySource::for_plan(&plan);
        let (_bytes, report) = pack_and_validate(&plan, &source).unwrap();
        assert_eq!(report.visible_snapshot_hours, 3);
    }

    fn standalone_source_plan() -> StitchedPlan {
        let mut seg = snap_seg(0, 0..=2);
        seg.raw_source.as_mut().unwrap().storage = RawSourceStorage::Standalone;
        match plan_stitch(GAME, stitch_inputs(), vec![seg]) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn standalone_source_is_referenced_not_embedded() {
        let plan = standalone_source_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("standalone");
        let dest = dir.0.join("replay.battleplan");

        let report = publish_stitched_container_retained(&plan, &source, &dest).unwrap();
        assert_eq!(report.segments, 1);
        assert_eq!(report.visible_snapshot_hours, 3);

        let bytes = portable_test_fs_retry::read(&dest).unwrap();
        let container = Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap();
        assert!(
            container.entries_by_role("raw_source").is_empty(),
            "standalone raw source must not be embedded in the container"
        );
        let raw_path = &plan.segments[0]
            .input
            .raw_source
            .as_ref()
            .unwrap()
            .source_path;
        assert!(
            container
                .load_file(&format!("payload/{GAME}/{raw_path}"))
                .is_err(),
            "standalone source payload must be absent from the container"
        );
    }

    #[test]
    fn temp_cleanup_failure_is_surfaced_not_swallowed() {
        let temp = PathBuf::from(r"C:\replay\.replay.battleplan.stitch-tmp-123");
        let primary = "atomic publish (rename) failed: disk full".to_string();

        let ok = with_cleanup_note(primary.clone(), &temp, Ok(()));
        assert_eq!(ok, primary, "successful cleanup adds no note");

        let gone = with_cleanup_note(
            primary.clone(),
            &temp,
            Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        );
        assert_eq!(gone, primary, "an already-gone temp satisfies the contract");

        let denied = with_cleanup_note(
            primary.clone(),
            &temp,
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        );
        assert!(
            denied.starts_with(&primary),
            "primary error kept as the cause"
        );
        assert!(
            denied.contains("failed to remove temp artifact")
                && denied.contains("partial temp file remains"),
            "cleanup failure must be surfaced: {denied}"
        );
    }
}
