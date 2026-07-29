//! Production composition for reload/relaunch stitching.
//!
//! This is deliberately the only bridge from the activated CID1/rollover
//! recorder into the generic stitch planner.  It allocates an append ticket
//! before ordinary campaign bytes are admitted, records the ticket beside the
//! run, registers only fully verified standalone packages, then reconstructs
//! sealed [`SegmentInput`] values from those authenticated packages.  Directory
//! enumeration discovers candidates only; ticket rank is the sole ordering
//! authority.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::calendar::hour_index_from_raw;
use compact_contract::{
    parse_compact_record_header, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_FRAME_END,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snapshot::container::{Container, ExpectedIdentity, SpoolEntryReceipt};
use snapshot::sha256::{to_hex, Sha256};
use snapshot::zip::STREAMING_MAX_CONTAINER_ENTRY_BYTES;

use super::product::{
    publish_product_stitch_retained_with_rebuilt_wire, ProductStitchOutcome,
};
use super::publish::{
    PublishReport, RetainedStitchedPayloadSource, StitchedRebuiltWireProduct,
    StitchedRebuiltWireSource, VerifiedPayloadReceipt,
};
use super::runtime::DurableAppendLedger;
use super::{
    AppendTicket, CampaignIdentityResolution, CompatibilityResolution, CoverageDisposition,
    HourObservation, ObservationOwner, RawSourceDescriptor, RawSourceStorage, SegmentInput,
    SegmentProvenance, SegmentReason, SourceParticipation, StitchRequiredInputs,
    STITCH_ENTRY_ORDER,
};
use crate::duplex_session::rollover_fs::{
    DurableRolloverSetDescriptor, PACKAGE_SET_DESCRIPTOR_FILE, SET_DESCRIPTOR_FILE,
};

const TICKET_SCHEMA: &str = "product-stitch-ticket-v1";
const REGISTRY_SCHEMA: &str = "product-stitch-registry-entry-v1";
const MACHINE_SCOPE_FILE: &str = "machine-scope-v1";
const MAX_CONTROL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketReceipt {
    schema: String,
    logical_stream_id: u64,
    campaign_key_hex: String,
    identity_evidence_sha256: String,
    play_order: u64,
    session_nonce: String,
    process_start_identity: String,
    machine_scope_sha256: String,
}

impl TicketReceipt {
    fn ticket(&self) -> Result<AppendTicket, String> {
        Ok(AppendTicket {
            campaign_key: decode_sha256(&self.campaign_key_hex)?,
            play_order: self.play_order,
            session_nonce: self.session_nonce.clone(),
            process_start_identity: self.process_start_identity.clone(),
        })
    }

    fn evidence_digest(&self) -> Result<[u8; 32], String> {
        decode_sha256(&self.identity_evidence_sha256)?
            .try_into()
            .map_err(|_| "identity evidence digest is not 32 bytes".to_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryEntry {
    schema: String,
    ticket: TicketReceipt,
    package_path: String,
    package_sha256: String,
    raw_path: String,
    raw_sha256: String,
    manifest_path: String,
    manifest_sha256: String,
    start_reason: String,
    stop_reason: String,
}

/// Successful stitched publications created while closing one rollover set.
/// A campaign with fewer than two registered sealed segments is intentionally
/// absent: its standalone package remains the complete product artifact.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProductStitchReports {
    pub published: Vec<PublishReport>,
    pub kept_separate: Vec<String>,
}

/// Allocate and durably retain campaign append order before the first ordinary
/// record of `logical_stream_id` may be accepted.
pub fn allocate_product_append_ticket(
    product_root: &Path,
    run_dir: &Path,
    logical_stream_id: u64,
    identity: &ValidatedCid1,
    session_nonce: &str,
    process_start_identity: &str,
) -> Result<(), String> {
    require_direct_run(product_root, run_dir)?;
    let machine_scope_sha256 = machine_scope(product_root)?;
    let campaign_key = identity.campaign_id();
    let ticket = DurableAppendLedger::new(
        product_root.join("stitching").join("append-ledgers"),
    )
    .allocate(&campaign_key, session_nonce, process_start_identity)?;
    let receipt = TicketReceipt {
        schema: TICKET_SCHEMA.to_string(),
        logical_stream_id,
        campaign_key_hex: identity.campaign_id_hex(),
        identity_evidence_sha256: identity.payload_sha256_hex(),
        play_order: ticket.play_order,
        session_nonce: ticket.session_nonce,
        process_start_identity: ticket.process_start_identity,
        machine_scope_sha256,
    };
    validate_ticket_receipt(&receipt)?;
    let path = ticket_path(run_dir, logical_stream_id);
    write_immutable_json(&path, &receipt)
}

/// Register all verified standalone members from a completed rollover package
/// set and publish each campaign whose local registry now has at least two
/// authenticated segments.  Planner refusal is visible but does not invalidate
/// the independently verified standalone replays.
pub fn publish_registered_rollover_stitches(
    product_root: &Path,
    run_dir: &Path,
    descriptor: &DurableRolloverSetDescriptor,
) -> Result<ProductStitchReports, String> {
    require_direct_run(product_root, run_dir)?;
    if descriptor.schema != "cid1-rollover-set-v1" || descriptor.ordered_streams.is_empty() {
        return Err("stitch callsite received a malformed/empty rollover set".into());
    }
    let local_machine_scope = machine_scope(product_root)?;
    let package_set_path = run_dir.join(PACKAGE_SET_DESCRIPTOR_FILE);
    let package_set = read_strict_json(&package_set_path)?;
    let source_descriptor_path = run_dir.join("logical-streams").join(SET_DESCRIPTOR_FILE);
    let source_descriptor_bytes = fs::read(&source_descriptor_path)
        .map_err(|error| format!("read rollover source descriptor for stitch failed: {error}"))?;
    if source_descriptor_bytes.len() > MAX_CONTROL_BYTES {
        return Err("rollover source descriptor exceeds 16 MiB".into());
    }
    let source_descriptor = snapshot::parse_json_strict(&source_descriptor_bytes)
        .map_err(|error| format!("strict-decode rollover source descriptor failed: {error}"))?;
    let source_descriptor_sha256 = Sha256::hex(&source_descriptor_bytes);
    let expected_source_descriptor = serde_json::to_value(descriptor)
        .map_err(|error| format!("encode rollover source descriptor join failed: {error}"))?;
    let expected_source_relative = format!("logical-streams/{SET_DESCRIPTOR_FILE}");
    let expected_transitions = serde_json::to_value(&descriptor.transitions)
        .map_err(|error| format!("encode rollover transitions for stitch join failed: {error}"))?;
    require_exact_object_keys(
        &package_set,
        &[
            "schema",
            "source_descriptor",
            "source_descriptor_sha256",
            "ordered_streams",
            "transitions",
        ],
        "rollover package set",
    )?;
    if package_set.get("schema").and_then(Value::as_str)
        != Some("cid1-rollover-package-set-v1")
        || package_set.get("source_descriptor").and_then(Value::as_str)
            != Some(expected_source_relative.as_str())
        || package_set
            .get("source_descriptor_sha256")
            .and_then(Value::as_str)
            != Some(source_descriptor_sha256.as_str())
        || package_set.get("transitions") != Some(&expected_transitions)
        || source_descriptor != expected_source_descriptor
    {
        return Err("rollover package set authority disagrees with its durable descriptor".into());
    }
    let members = package_set
        .get("ordered_streams")
        .and_then(Value::as_array)
        .ok_or("rollover package set has no ordered_streams")?;
    if members.len() != descriptor.ordered_streams.len() {
        return Err("rollover package set member count differs from raw descriptor".into());
    }

    let mut touched_campaigns = BTreeSet::new();
    for (index, (raw, member)) in descriptor.ordered_streams.iter().zip(members).enumerate() {
        require_exact_object_keys(
            member,
            &[
                "logical_stream_id",
                "campaign_id_hex",
                "cid1_payload_sha256_hex",
                "raw_relative_path",
                "raw_bytes",
                "raw_sha256",
                "manifest_relative_path",
                "manifest_sha256",
                "package_relative_path",
                "package_sha256",
                "recovery",
            ],
            "rollover package member",
        )?;
        let expected_logical_stream_id = u64::try_from(index)
            .map_err(|_| "rollover package set has more than u64::MAX members")?;
        if raw.logical_stream_id != expected_logical_stream_id {
            return Err("durable rollover stream order is not dense from logical id zero".into());
        }
        let ticket = read_ticket(run_dir, raw.logical_stream_id)?;
        if ticket.campaign_key_hex != raw.campaign_id_hex
            || ticket.identity_evidence_sha256 != raw.cid1_payload_sha256_hex
            || ticket.machine_scope_sha256 != local_machine_scope
        {
            return Err(format!(
                "logical stream {} ticket disagrees with CID1/machine authority",
                raw.logical_stream_id
            ));
        }
        if member.get("logical_stream_id").and_then(Value::as_u64)
            != Some(raw.logical_stream_id)
            || member.get("campaign_id_hex").and_then(Value::as_str)
                != Some(raw.campaign_id_hex.as_str())
            || member
                .get("cid1_payload_sha256_hex")
                .and_then(Value::as_str)
                != Some(raw.cid1_payload_sha256_hex.as_str())
            || member.get("raw_bytes").and_then(Value::as_u64) != Some(raw.file_bytes)
            || member.get("raw_sha256").and_then(Value::as_str)
                != Some(raw.file_sha256_hex.as_str())
            || member.get("recovery").and_then(Value::as_bool) != Some(false)
        {
            return Err(format!(
                "logical stream {} is not a verified production package",
                raw.logical_stream_id
            ));
        }
        let package_path = product_relative_from_run(
            product_root,
            run_dir,
            &required_relative(member, "package_relative_path")?,
        )?;
        let manifest_path = product_relative_from_run(
            product_root,
            run_dir,
            &required_relative(member, "manifest_relative_path")?,
        )?;
        let raw_path = product_relative_from_run(
            product_root,
            run_dir,
            &required_relative(member, "raw_relative_path")?,
        )?;
        let entry = RegistryEntry {
            schema: REGISTRY_SCHEMA.to_string(),
            ticket: ticket.clone(),
            package_sha256: required_sha(member, "package_sha256")?,
            package_path,
            raw_sha256: required_sha(member, "raw_sha256")?,
            raw_path,
            manifest_sha256: required_sha(member, "manifest_sha256")?,
            manifest_path,
            start_reason: transition_reason(descriptor, raw.logical_stream_id, true),
            stop_reason: transition_reason(descriptor, raw.logical_stream_id, false),
        };
        validate_registry_entry(product_root, &entry)?;
        register_entry(product_root, &entry)?;
        touched_campaigns.insert(ticket.campaign_key_hex);
    }

    let mut reports = ProductStitchReports::default();
    for campaign in touched_campaigns {
        let entries = load_campaign_entries(product_root, &campaign, &local_machine_scope)?;
        if entries.len() < 2 {
            reports.kept_separate.push(format!(
                "campaign {campaign} has only one authenticated sealed segment"
            ));
            continue;
        }
        let destination = run_dir.join(format!("stitched-{}.battleplan", &campaign[..16]));
        match compose_and_publish(product_root, &campaign, entries, &destination)? {
            ProductStitchOutcome::Published(report) => reports.published.push(report),
            ProductStitchOutcome::KeptSeparate { reason } => reports
                .kept_separate
                .push(format!("campaign {campaign}: {reason}")),
        }
    }
    Ok(reports)
}

fn compose_and_publish(
    product_root: &Path,
    campaign_hex: &str,
    mut entries: Vec<RegistryEntry>,
    destination: &Path,
) -> Result<ProductStitchOutcome, String> {
    entries.sort_by_key(|entry| {
        (entry.ticket.play_order, entry.ticket.logical_stream_id)
    });
    let mut segments = Vec::with_capacity(entries.len());
    let mut payloads = RetainedProductPayloads::default();
    let mut created_utc = None;
    let mut wire_catalogs = Vec::with_capacity(entries.len());
    let mut wire_content_manifest = None;
    let mut wire_sources = Vec::with_capacity(entries.len());
    for (segment_id, entry) in entries.iter().enumerate() {
        let segment_id = u32::try_from(segment_id)
            .map_err(|_| "registered campaign has more than u32::MAX segments")?;
        let package_path = join_relative(product_root, &entry.package_path)?;
        let raw_path = join_relative(product_root, &entry.raw_path)?;
        let raw = inspect_registered_raw(&raw_path, entry)?;
        let (container, package_bytes, package_sha256) =
            Container::open_strict_with_file_receipt(
                &package_path,
                &ExpectedIdentity::default(),
            )
            .map_err(|error| format!("open registered standalone replay failed: {error}"))?;
        if package_bytes == 0 || package_sha256 != entry.package_sha256 {
            return Err("opened standalone replay differs from its registry receipt".into());
        }
        container
            .verify_all_streaming()
            .map_err(|error| format!("verify registered standalone replay failed: {error}"))?;
        let manifests = container.entries_by_role("recorder_manifest");
        if manifests.len() != 1 {
            return Err("registered replay does not have exactly one recorder manifest".into());
        }
        if manifests[0].bytes > MAX_CONTROL_BYTES as u64 {
            return Err("registered recorder manifest exceeds 16 MiB".into());
        }
        let manifest_bytes = container
            .read_entry_bytes(&manifests[0].path)
            .map_err(|error| format!("read registered recorder manifest failed: {error}"))?;
        if Sha256::hex(&manifest_bytes) != entry.manifest_sha256 {
            return Err("registered external/embedded manifest identity changed".into());
        }
        let manifest = snapshot::parse_json_strict(&manifest_bytes)
            .map_err(|error| format!("strict-decode registered manifest failed: {error}"))?;
        validate_manifest_identity(&manifest, entry)?;
        let identity_mode = registered_identity_mode(&manifest)?;
        let (content_manifest, province_catalog) =
            verify_registered_rebuilt_wire_authority(&container, &raw_path, identity_mode)?;
        let content_digest = content_manifest
            .sha256()
            .map_err(|error| format!("hash registered content manifest failed: {error}"))?;
        if let Some((expected_digest, _)) = wire_content_manifest.as_ref() {
            if expected_digest != &content_digest {
                return Err(
                    "registered stitch members use different content manifests".to_string(),
                );
            }
        } else {
            wire_content_manifest = Some((content_digest, content_manifest));
        }
        wire_catalogs.push(province_catalog);
        created_utc = Some(required_pointer_string(
            &manifest,
            "/replay_packaging/created_utc",
        )?);
        let built = build_segment(
            campaign_hex,
            entry,
            &container,
            &manifest,
            &raw,
            payloads.packages.len(),
        )?;
        if payloads
            .raw
            .insert(
                built.raw_container_path.clone(),
                RetainedRawPayload {
                    file: Mutex::new(raw.file),
                    receipt: raw.receipt,
                },
            )
            .is_some()
        {
            return Err("registered stitch maps multiple retained raws to one container path".into());
        }
        for (hour, snapshot) in built.snapshots {
            if payloads
                .snapshots
                .insert((segment_id, hour), snapshot)
                .is_some()
            {
                return Err("registered stitch repeats a retained snapshot locator".into());
            }
        }
        wire_sources.push(StitchedRebuiltWireSource::open(
            built.raw_container_path.clone(),
            raw_path,
            identity_mode,
        )?);
        payloads.packages.push(container);
        segments.push(built.segment);
    }
    add_cross_session_gaps(&mut segments)?;
    let game_id = format!("campaign-{}", &campaign_hex[..32]);
    let required_inputs = StitchRequiredInputs {
        schema_version: "1".to_string(),
        fold_version: "world-fold-schema-1".to_string(),
        canonical_writer_version: "snapshot-canonical-json-v1".to_string(),
        packer_codec_version: "snapshot-zip-store-v2".to_string(),
        created_utc: created_utc.ok_or("stitch has no immutable created_utc input")?,
        compression_method: "store".to_string(),
        compression_parameters: BTreeMap::from([("level".to_string(), "none".to_string())]),
        entry_order: STITCH_ENTRY_ORDER.to_string(),
    };
    let catalog = wire_product::AuthenticatedProvinceCatalog::union(&wire_catalogs)
        .map_err(|error| format!("union registered province catalogs failed: {error}"))?;
    let content_manifest = wire_content_manifest
        .map(|(_, manifest)| manifest)
        .ok_or_else(|| "registered stitch has no content manifest authority".to_string())?;
    publish_product_stitch_retained_with_rebuilt_wire(
        &game_id,
        required_inputs,
        segments,
        &payloads,
        destination,
        move |plan| {
            StitchedRebuiltWireProduct::build(
                plan,
                wire_product::BundleBuildConfig::default(),
                content_manifest,
                catalog,
                wire_sources,
            )
        },
    )
}

fn registered_identity_mode(
    manifest: &Value,
) -> Result<compact_contract::stream_validator::StreamIdentityMode, String> {
    match manifest
        .get("compact_stream_identity_mode")
        .and_then(Value::as_str)
    {
        Some("identity_v1") => {
            Ok(compact_contract::stream_validator::StreamIdentityMode::IdentityV1)
        }
        Some("legacy") => Ok(compact_contract::stream_validator::StreamIdentityMode::Legacy),
        other => Err(format!(
            "registered recorder manifest has unsupported compact_stream_identity_mode {other:?}"
        )),
    }
}

fn verify_registered_rebuilt_wire_authority(
    container: &Container,
    raw_path: &Path,
    identity_mode: compact_contract::stream_validator::StreamIdentityMode,
) -> Result<
    (
        portable_content_manifest::ContentManifest,
        wire_product::AuthenticatedProvinceCatalog,
    ),
    String,
> {
    let content_manifest = container
        .content_manifest()
        .cloned()
        .ok_or_else(|| "registered replay has no portable content manifest".to_string())?;
    let descriptors = registered_rebuilt_wire_descriptors(container)?;
    let read_entry = |path: &str| {
        let packed_path = format!("payload/{}/{path}", container.game_id());
        container.read_entry_bytes(&packed_path).map_err(|error| {
            format!("read registered rebuilt-wire entry {packed_path}: {error}")
        })
    };
    let verified = wire_product::verify_entry_source(&descriptors, read_entry)
        .map_err(|error| format!("registered rebuilt-wire bundle rejected: {error}"))?;
    let manifest_sha256: [u8; 32] = content_manifest
        .sha256()
        .map_err(|error| format!("hash registered content manifest failed: {error}"))
        .and_then(|digest| decode_sha256(&digest))?
        .try_into()
        .map_err(|_| "registered content-manifest digest is not 32 bytes".to_string())?;
    if verified.content_manifest_sha256 != manifest_sha256 {
        return Err(
            "registered rebuilt-wire bundle belongs to a different content manifest".to_string(),
        );
    }
    let catalog_entry = descriptors
        .iter()
        .filter(|entry| entry.role == wire_product::ArtifactRole::ProvinceCatalog)
        .collect::<Vec<_>>();
    if catalog_entry.len() != 1 {
        return Err(
            "registered rebuilt-wire bundle must have exactly one province catalog".to_string(),
        );
    }
    let content_manifest_json = content_manifest
        .encode_json()
        .map_err(|error| format!("encode registered content manifest failed: {error}"))?;
    let catalog_bytes = read_entry(&catalog_entry[0].path)?;
    let catalog = wire_product::AuthenticatedProvinceCatalog::from_artifact(
        &content_manifest_json,
        &catalog_bytes,
    )
    .map_err(|error| format!("reauthenticate registered province catalog failed: {error}"))?;
    let raw_file = File::open(raw_path).map_err(|error| {
        format!(
            "retain registered D81 source {} for semantic verification: {error}",
            raw_path.display()
        )
    })?;
    let session = wire_product::H4crSession {
        branch: 0,
        identity_mode,
        source: wire_product::H4crSource::RetainedFile(&raw_file),
    };
    let inspection = wire_product::inspect_d81_session(session)
        .map_err(|error| format!("inspect registered D81 source failed: {error}"))?;
    let branch = wire_product::BranchSpec {
        branch: 0,
        parent: None,
        first_tick: inspection.first_tick,
        session_fingerprint: inspection.decoded_sha256,
    };
    wire_product::verify_entry_source_against_h4cr(
        &descriptors,
        read_entry,
        wire_product::BundleBuildConfig::default(),
        &[branch],
        &catalog,
        &[session],
        &[],
    )
    .map_err(|error| format!("registered rebuilt-wire source parity failed: {error}"))?;
    Ok((content_manifest, catalog))
}

fn registered_rebuilt_wire_descriptors(
    container: &Container,
) -> Result<Vec<wire_product::BundleEntryDescriptor>, String> {
    let index_path = format!("payload/{}/index.json", container.game_id());
    let index_bytes = container
        .read_entry_bytes(&index_path)
        .map_err(|error| format!("read registered rebuilt-wire index failed: {error}"))?;
    let index = snapshot::parse_json_strict(&index_bytes)
        .map_err(|error| format!("strict-decode registered rebuilt-wire index failed: {error}"))?;
    let rebuilt = index
        .get("rebuilt_wire")
        .ok_or_else(|| "registered replay index has no rebuilt_wire discovery".to_string())?;
    require_exact_object_keys(
        rebuilt,
        &[
            "present",
            "root",
            "semantic_reopen_required_before_publication",
            "entries",
        ],
        "registered rebuilt-wire discovery",
    )?;
    if rebuilt.get("present").and_then(Value::as_bool) != Some(true)
        || rebuilt.get("root").and_then(Value::as_str) != Some("rebuilt_wire")
        || rebuilt
            .get("semantic_reopen_required_before_publication")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err("registered rebuilt-wire discovery is not production-authoritative".to_string());
    }
    let rows = rebuilt
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "registered rebuilt-wire discovery entries are not an array".to_string())?;
    if rows.is_empty() {
        return Err("registered rebuilt-wire discovery has no entries".to_string());
    }
    let mut descriptors = Vec::with_capacity(rows.len());
    for row in rows {
        require_exact_object_keys(
            row,
            &["path", "role", "bytes", "blake3"],
            "registered rebuilt-wire descriptor",
        )?;
        let path = required_pointer_string(row, "/path")?;
        if !path.starts_with("rebuilt_wire/") {
            return Err(format!(
                "registered rebuilt-wire descriptor has noncanonical path {path}"
            ));
        }
        let role_text = required_pointer_string(row, "/role")?;
        let role = wire_product::ArtifactRole::from_str(&role_text)
            .map_err(|error| format!("registered rebuilt-wire role rejected: {error}"))?;
        let len = row
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| "registered rebuilt-wire descriptor has invalid bytes".to_string())?;
        let blake3: [u8; 32] =
            decode_sha256(&required_pointer_string(row, "/blake3")?)?
            .try_into()
            .map_err(|_| "registered rebuilt-wire BLAKE3 is not 32 bytes".to_string())?;
        descriptors.push(wire_product::BundleEntryDescriptor {
            path,
            role,
            len,
            blake3,
        });
    }
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    if descriptors
        .windows(2)
        .any(|pair| pair[0].path == pair[1].path)
    {
        return Err("registered rebuilt-wire discovery repeats a path".to_string());
    }

    let prefix = format!("payload/{}/rebuilt_wire/", container.game_id());
    let mut packed = BTreeMap::new();
    for entry in container.entries() {
        let parsed_role = entry
            .role
            .as_deref()
            .and_then(|role| wire_product::ArtifactRole::from_str(role).ok());
        if !entry.path.starts_with(&prefix) {
            if parsed_role.is_some() {
                return Err(format!(
                    "registered rebuilt-wire role appears outside canonical prefix: {}",
                    entry.path
                ));
            }
            continue;
        }
        let relative = entry
            .path
            .strip_prefix(&format!("payload/{}/", container.game_id()))
            .ok_or_else(|| format!("registered rebuilt-wire path {} lost prefix", entry.path))?;
        let role_text = entry
            .role
            .as_deref()
            .ok_or_else(|| format!("registered rebuilt-wire entry {} has no role", entry.path))?;
        let role = wire_product::ArtifactRole::from_str(role_text).map_err(|error| {
            format!(
                "registered rebuilt-wire entry {} has invalid role: {error}",
                entry.path
            )
        })?;
        if packed
            .insert(relative.to_string(), (role, entry.bytes))
            .is_some()
        {
            return Err(format!("registered rebuilt-wire repeats entry {relative}"));
        }
    }
    if packed.len() != descriptors.len() {
        return Err(
            "registered rebuilt-wire discovery/container cardinality differs".to_string(),
        );
    }
    for descriptor in &descriptors {
        let (role, len) = packed.get(&descriptor.path).ok_or_else(|| {
            format!(
                "registered container is missing rebuilt-wire entry {}",
                descriptor.path
            )
        })?;
        if *role != descriptor.role || *len != descriptor.len {
            return Err(format!(
                "registered rebuilt-wire metadata differs for {}",
                descriptor.path
            ));
        }
    }
    Ok(descriptors)
}

struct BuiltSegment {
    segment: SegmentInput,
    raw_container_path: String,
    snapshots: BTreeMap<i64, RetainedSnapshotPayload>,
}

fn build_segment(
    campaign_hex: &str,
    entry: &RegistryEntry,
    container: &Container,
    manifest: &Value,
    raw: &RawInspection,
    package_index: usize,
) -> Result<BuiltSegment, String> {
    let ticket = entry.ticket.ticket()?;
    let evidence_digest = entry.ticket.evidence_digest()?;
    if ticket.campaign_key != decode_sha256(campaign_hex)? {
        return Err("registered ticket belongs to a different campaign".into());
    }
    validate_raw_identity(&raw.first_record, entry)?;
    let frames = &raw.frames;
    let mut snapshots = BTreeMap::new();
    for indexed in &container.index().snapshots {
        let hour = indexed
            .hour_index
            .ok_or("standalone snapshot index entry has no hour")?;
        let path = format!("payload/{}/{}", container.game_id(), indexed.file);
        let receipt = container
            .authenticate_entry(&path)
            .map_err(|error| format!("authenticate standalone snapshot {hour} failed: {error}"))?;
        if snapshots
            .insert(
                hour,
                RetainedSnapshotPayload {
                    package_index,
                    entry_path: path,
                    receipt,
                },
            )
            .is_some()
        {
            return Err(format!("standalone replay has duplicate snapshot hour {hour}"));
        }
    }
    let frame_hours: Vec<i64> = frames.keys().copied().collect();
    let snapshot_hours: Vec<i64> = snapshots.keys().copied().collect();
    if frame_hours != snapshot_hours || snapshot_hours.is_empty() {
        return Err(
            "standalone frame/snapshot hour sets differ; refusing to invent stitch coverage"
                .into(),
        );
    }
    for adjacent in snapshot_hours.windows(2) {
        if adjacent[0].checked_add(1) != Some(adjacent[1]) {
            return Err(
                "standalone segment has an unmaterialized internal hour gap; keep separate"
                    .into(),
            );
        }
    }
    let owner = ObservationOwner {
        ticket: ticket.clone(),
        in_session_ordinal: u32::try_from(entry.ticket.logical_stream_id)
            .map_err(|_| "logical stream id exceeds stitching ordinal width")?,
        epoch_token: entry.ticket.identity_evidence_sha256.clone(),
    };
    let mut coverage = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for (hour, snapshot) in &snapshots {
        let disposition = CoverageDisposition::Snapshot {
            sha256: snapshot.receipt.sha256().to_string(),
        };
        let extent = frames
            .get(hour)
            .ok_or("standalone frame disappeared after hour-set validation")?;
        coverage.insert(*hour, disposition.clone());
        observations.insert(
            *hour,
            vec![HourObservation {
                observation_id: format!(
                    "frame-{}-{}-{}",
                    extent.first_sequence, extent.last_sequence, extent.start_byte
                ),
                owner: owner.clone(),
                record_seq_bounds: (extent.first_sequence, extent.last_sequence),
                byte_bounds: (extent.start_byte, extent.end_byte),
                disposition,
            }],
        );
    }
    let source_bounds = raw.source_bounds;
    let session_path_digest = structured_digest(
        b"product-stitch-session-path-v1",
        &[entry.ticket.session_nonce.as_bytes()],
    );
    let raw_container_path = format!(
        "raw/session-{}/logical-{:020}.h4cr",
        &session_path_digest[..16],
        entry.ticket.logical_stream_id
    );
    let participation = SourceParticipation {
        ticket: ticket.clone(),
        scheme_major: 1,
        identity_evidence_digest: evidence_digest,
        record_seq_bounds: source_bounds.0,
        byte_bounds: source_bounds.1,
    };
    let raw_source = RawSourceDescriptor {
        session_nonce: ticket.session_nonce.clone(),
        process_start_identity: ticket.process_start_identity.clone(),
        source_path: raw_container_path.clone(),
        stored_digest: entry.raw_sha256.clone(),
        decoded_digest: entry.raw_sha256.clone(),
        record_seq_bounds: source_bounds.0,
        byte_bounds: source_bounds.1,
        storage: RawSourceStorage::EmbeddedCampaign {
            campaign_key: ticket.campaign_key.clone(),
        },
        participations: vec![participation],
    };
    let build = required_pointer_sha(manifest, "/modset/game/exe_sha256")?;
    let modset = required_pointer_sha(manifest, "/static_bundle/product_modset_sha256")?;
    let catalog = required_pointer_sha(
        manifest,
        "/portable_content_manifest/manifest_digest_sha256",
    )?;
    let compatibility_id = structured_digest(
        b"product-stitch-compatibility-v1",
        &[
            build.as_bytes(),
            modset.as_bytes(),
            catalog.as_bytes(),
            b"snapshot-schema-1",
        ],
    );
    let compatibility_class = format!("snapshot-v1-{compatibility_id}");
    let start_reason = parse_reason(&entry.start_reason)?;
    let stop_reason = parse_reason(&entry.stop_reason)?;
    let ordinal = u32::try_from(entry.ticket.logical_stream_id)
        .map_err(|_| "logical stream id exceeds stitching ordinal width")?;
    let segment = SegmentInput {
        play_order: ticket.play_order,
        in_session_ordinal: ordinal,
        identity: CampaignIdentityResolution::Verified {
            scheme_major: 1,
            campaign_key: ticket.campaign_key.clone(),
            evidence_digest,
        },
        ticket: Some(ticket.clone()),
        compatibility: CompatibilityResolution::Proven {
            evidence_id: compatibility_id,
            compatibility_class,
            build_exe_sha256: build.clone(),
            modset_id: modset.clone(),
            schema_version: "1".to_string(),
            static_catalog_digest: catalog.clone(),
        },
        transition_coverage: BTreeMap::new(),
        coverage,
        observations,
        raw_source: Some(raw_source),
        provenance: SegmentProvenance {
            session_nonce: ticket.session_nonce,
            process_start_identity: ticket.process_start_identity,
            epoch_token: entry.ticket.identity_evidence_sha256.clone(),
            stream_major: 1,
            build_exe_sha256: Some(build),
            modset_id: Some(modset),
            schema_version: Some("1".to_string()),
            static_catalog_digest: Some(catalog),
            source_path: Some(raw_container_path.clone()),
            source_stored_digest: Some(entry.raw_sha256.clone()),
            source_decoded_digest: Some(entry.raw_sha256.clone()),
            record_seq_bounds: Some(source_bounds.0),
            byte_bounds: Some(source_bounds.1),
            start_reason: Some(start_reason),
            stop_reason: Some(stop_reason),
        },
    };
    segment.validate()?;
    Ok(BuiltSegment {
        segment,
        raw_container_path,
        snapshots,
    })
}

#[derive(Clone, Copy, Debug)]
struct FrameExtent {
    first_sequence: u64,
    last_sequence: u64,
    start_byte: u64,
    end_byte: u64,
}

struct RawInspection {
    file: File,
    receipt: VerifiedPayloadReceipt,
    first_record: Vec<u8>,
    frames: BTreeMap<i64, FrameExtent>,
    source_bounds: ((u64, u64), (u64, u64)),
}

fn open_retained_raw(path: &Path) -> Result<File, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|error| format!("open registered raw source failed: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("stat registered raw source failed: {error}"))?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("registered raw source is not a regular non-reparse file".into());
        }
        Ok(file)
    }
    #[cfg(not(windows))]
    {
        File::open(path).map_err(|error| format!("open registered raw source failed: {error}"))
    }
}

fn inspect_registered_raw(
    path: &Path,
    entry: &RegistryEntry,
) -> Result<RawInspection, String> {
    const SCAN_CHUNK: usize = 1024 * 1024;

    let mut file = open_retained_raw(path)?;
    let file_len = file
        .metadata()
        .map_err(|error| format!("stat registered raw source failed: {error}"))?
        .len();
    if file_len < COMPACT_RECORD_HEADER_LEN as u64 {
        return Err("registered raw source is empty/truncated".into());
    }
    let mut hasher = Sha256::new();
    let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
    let mut payload_buffer = vec![0u8; SCAN_CHUNK];
    let mut offset = 0u64;
    let mut previous_sequence: Option<u64> = None;
    let mut first_sequence = None;
    let mut last_sequence = 0u64;
    let mut open: Option<(i32, u64, u64)> = None;
    let mut frames = BTreeMap::new();
    let mut first_record = Vec::new();

    while offset < file_len {
        file.read_exact(&mut header_bytes)
            .map_err(|error| format!("read registered raw header at {offset} failed: {error}"))?;
        hasher.update(&header_bytes);
        let header = parse_compact_record_header(&header_bytes)
            .map_err(|error| format!("registered raw framing failed at {offset}: {error}"))?;
        let record_len = u64::try_from(header.record_len())
            .map_err(|_| "registered raw record length does not fit u64")?;
        let end = offset
            .checked_add(record_len)
            .filter(|end| *end <= file_len)
            .ok_or("registered raw record is truncated")?;
        if let Some(previous) = previous_sequence {
            if previous.checked_add(1) != Some(header.sequence) {
                return Err("registered raw sequence is not contiguous".into());
            }
        } else if header.sequence != 0 {
            return Err("IdentityV1 registered raw stream does not start at sequence zero".into());
        }
        previous_sequence = Some(header.sequence);
        first_sequence.get_or_insert(header.sequence);
        last_sequence = header.sequence;

        let payload_len = header
            .record_len()
            .checked_sub(COMPACT_RECORD_HEADER_LEN)
            .ok_or("registered raw record is shorter than its header")?;
        if offset == 0 {
            if header.record_len() > MAX_CONTROL_BYTES {
                return Err("registered raw CID1 record exceeds 16 MiB".into());
            }
            first_record.reserve_exact(header.record_len());
            first_record.extend_from_slice(&header_bytes);
        }
        let mut remaining = payload_len;
        while remaining != 0 {
            let take = remaining.min(payload_buffer.len());
            file.read_exact(&mut payload_buffer[..take]).map_err(|error| {
                format!("read registered raw payload at {offset} failed: {error}")
            })?;
            hasher.update(&payload_buffer[..take]);
            if offset == 0 {
                first_record.extend_from_slice(&payload_buffer[..take]);
            }
            remaining -= take;
        }

        if header.sequence != 0 {
            match open {
                None => open = Some((header.game_hour_raw, header.sequence, offset)),
                Some((hour, _, _)) if hour != header.game_hour_raw => {
                    return Err("registered raw stream interleaves logical hours".into())
                }
                _ => {}
            }
            if header.record_type == COMPACT_RECORD_TYPE_FRAME_END {
                let (hour, frame_first_sequence, start_byte) =
                    open.take().ok_or("FRAME_END has no open logical frame")?;
                let hour = hour_index_from_raw(hour).ok_or(
                    "registered raw FRAME_END carries an out-of-range game_hour_raw",
                )?;
                if frames
                    .insert(
                        hour,
                        FrameExtent {
                            first_sequence: frame_first_sequence,
                            last_sequence: header.sequence,
                            start_byte,
                            end_byte: end,
                        },
                    )
                    .is_some()
                {
                    return Err("registered raw stream repeats a completed hour".into());
                }
            }
        }
        offset = end;
    }
    if open.is_some() || offset != file_len {
        return Err("registered raw stream ends outside a complete frame boundary".into());
    }
    let sha256 = to_hex(&hasher.finalize());
    if sha256 != entry.raw_sha256 {
        return Err("opened raw source differs from its registry receipt".into());
    }
    validate_raw_identity(&first_record, entry)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind registered raw source failed: {error}"))?;
    Ok(RawInspection {
        file,
        receipt: VerifiedPayloadReceipt {
            bytes: file_len,
            sha256,
        },
        first_record,
        frames,
        source_bounds: (
            (
                first_sequence.ok_or("registered raw source has no records")?,
                last_sequence,
            ),
            (0, file_len),
        ),
    })
}

fn validate_raw_identity(raw: &[u8], entry: &RegistryEntry) -> Result<(), String> {
    let header = parse_compact_record_header(raw)
        .map_err(|error| format!("registered raw CID1 header is invalid: {error}"))?;
    let end = header.record_len();
    if end > raw.len() {
        return Err("registered raw CID1 record is truncated".into());
    }
    let first = crate::duplex_session::identity_control::ValidatedFirstCompactRecord::identity_v1(
        entry.ticket.logical_stream_id,
        &raw[..end],
    )
    .map_err(|error| format!("registered raw CID1 validation failed: {error}"))?;
    let cid1 = first
        .validated_cid1()
        .ok_or("registered raw first record did not retain CID1")?;
    if cid1.campaign_id_hex() != entry.ticket.campaign_key_hex
        || cid1.payload_sha256_hex() != entry.ticket.identity_evidence_sha256
    {
        return Err("registered raw CID1 differs from its append ticket".into());
    }
    Ok(())
}

#[cfg(test)]
fn scan_complete_frames(raw: &[u8]) -> Result<BTreeMap<i64, FrameExtent>, String> {
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut open: Option<(i32, u64, u64)> = None;
    let mut frames = BTreeMap::new();
    while offset < raw.len() {
        let header = parse_compact_record_header(&raw[offset..])
            .map_err(|error| format!("registered raw framing failed at {offset}: {error}"))?;
        let end = offset
            .checked_add(header.record_len())
            .filter(|end| *end <= raw.len())
            .ok_or("registered raw record is truncated")?;
        if let Some(previous) = previous_sequence {
            if previous.checked_add(1) != Some(header.sequence) {
                return Err("registered raw sequence is not contiguous".into());
            }
        } else if header.sequence != 0 {
            return Err("IdentityV1 registered raw stream does not start at sequence zero".into());
        }
        previous_sequence = Some(header.sequence);
        if header.sequence != 0 {
            match open {
                None => open = Some((header.game_hour_raw, header.sequence, offset as u64)),
                Some((hour, _, _)) if hour != header.game_hour_raw => {
                    return Err("registered raw stream interleaves logical hours".into())
                }
                _ => {}
            }
            if header.record_type == COMPACT_RECORD_TYPE_FRAME_END {
                let (hour, first_sequence, start_byte) =
                    open.take().ok_or("FRAME_END has no open logical frame")?;
                let hour = hour_index_from_raw(hour).ok_or(
                    "registered raw FRAME_END carries an out-of-range game_hour_raw",
                )?;
                if frames
                    .insert(
                        hour,
                        FrameExtent {
                            first_sequence,
                            last_sequence: header.sequence,
                            start_byte,
                            end_byte: end as u64,
                        },
                    )
                    .is_some()
                {
                    return Err("registered raw stream repeats a completed hour".into());
                }
            }
        }
        offset = end;
    }
    if open.is_some() || offset != raw.len() {
        return Err("registered raw stream ends outside a complete frame boundary".into());
    }
    Ok(frames)
}

#[cfg(test)]
fn scan_source_bounds(raw: &[u8]) -> Result<((u64, u64), (u64, u64)), String> {
    if raw.len() < COMPACT_RECORD_HEADER_LEN {
        return Err("registered raw source is empty/truncated".into());
    }
    let first = parse_compact_record_header(raw)
        .map_err(|error| format!("registered raw first record invalid: {error}"))?;
    let mut offset = 0usize;
    let mut last = first.sequence;
    while offset < raw.len() {
        let header = parse_compact_record_header(&raw[offset..])
            .map_err(|error| format!("registered raw record invalid: {error}"))?;
        offset = offset
            .checked_add(header.record_len())
            .filter(|end| *end <= raw.len())
            .ok_or("registered raw record extent overflow")?;
        last = header.sequence;
    }
    Ok(((first.sequence, last), (0, raw.len() as u64)))
}

fn add_cross_session_gaps(segments: &mut [SegmentInput]) -> Result<(), String> {
    segments.sort_by_key(|segment| (segment.play_order, segment.in_session_ordinal));
    for index in 1..segments.len() {
        let previous_head = segments[index - 1]
            .head()
            .ok_or("registered stitch contains an empty production segment")?;
        let cutover = segments[index]
            .cutover()
            .ok_or("registered stitch contains an empty production segment")?;
        let Some(first_gap) = previous_head.checked_add(1) else {
            return Err("previous segment head overflows hour range".into());
        };
        if cutover <= first_gap {
            continue;
        }
        for hour in first_gap..cutover {
            let evidence_id = format!(
                "inter-segment-gap-{}-{}",
                segments[index].play_order, hour
            );
            segments[index].transition_coverage.insert(
                hour,
                CoverageDisposition::Gap {
                    reason: "no authenticated capture exists between sealed segments".into(),
                    evidence_id,
                },
            );
        }
    }
    Ok(())
}

#[derive(Default)]
struct RetainedProductPayloads {
    packages: Vec<Container>,
    snapshots: BTreeMap<(u32, i64), RetainedSnapshotPayload>,
    raw: BTreeMap<String, RetainedRawPayload>,
}

struct RetainedSnapshotPayload {
    package_index: usize,
    entry_path: String,
    receipt: SpoolEntryReceipt,
}

struct RetainedRawPayload {
    file: Mutex<File>,
    receipt: VerifiedPayloadReceipt,
}

fn verified_snapshot_receipt(receipt: &SpoolEntryReceipt) -> VerifiedPayloadReceipt {
    VerifiedPayloadReceipt {
        bytes: receipt.uncompressed_len(),
        sha256: receipt.sha256().to_string(),
    }
}

impl RetainedStitchedPayloadSource for RetainedProductPayloads {
    fn snapshot_receipt(
        &self,
        segment_id: u32,
        hour_index: i64,
    ) -> Result<VerifiedPayloadReceipt, String> {
        self.snapshots
            .get(&(segment_id, hour_index))
            .map(|snapshot| verified_snapshot_receipt(&snapshot.receipt))
            .ok_or_else(|| format!("missing registered snapshot {segment_id}/{hour_index}"))
    }

    fn copy_snapshot_to_unpublished(
        &self,
        segment_id: u32,
        hour_index: i64,
        writer: &mut dyn Write,
    ) -> Result<VerifiedPayloadReceipt, String> {
        let snapshot = self
            .snapshots
            .get(&(segment_id, hour_index))
            .ok_or_else(|| format!("missing registered snapshot {segment_id}/{hour_index}"))?;
        let package = self
            .packages
            .get(snapshot.package_index)
            .ok_or("registered snapshot names an unknown retained package")?;
        let mut sink = writer;
        let receipt = package
            .copy_entry_to_unpublished_writer_bounded(
                &snapshot.entry_path,
                &mut sink,
                STREAMING_MAX_CONTAINER_ENTRY_BYTES,
            )
            .map_err(|error| format!("copy retained snapshot failed: {error}"))?;
        Ok(verified_snapshot_receipt(&receipt))
    }

    fn raw_source_receipt(&self, source_path: &str) -> Result<VerifiedPayloadReceipt, String> {
        self.raw
            .get(source_path)
            .map(|raw| raw.receipt.clone())
            .ok_or_else(|| format!("missing registered raw source {source_path}"))
    }

    fn copy_raw_source_to_unpublished(
        &self,
        source_path: &str,
        writer: &mut dyn Write,
    ) -> Result<VerifiedPayloadReceipt, String> {
        const COPY_CHUNK: usize = 1024 * 1024;

        let raw = self
            .raw
            .get(source_path)
            .ok_or_else(|| format!("missing registered raw source {source_path}"))?;
        let mut file = raw
            .file
            .lock()
            .map_err(|_| "registered raw source lock poisoned".to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| format!("rewind retained raw source failed: {error}"))?;
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = vec![0u8; COPY_CHUNK];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("read retained raw source failed: {error}"))?;
            if read == 0 {
                break;
            }
            writer
                .write_all(&buffer[..read])
                .map_err(|error| format!("copy retained raw source failed: {error}"))?;
            hasher.update(&buffer[..read]);
            bytes = bytes
                .checked_add(read as u64)
                .ok_or("retained raw source length overflow")?;
        }
        Ok(VerifiedPayloadReceipt {
            bytes,
            sha256: to_hex(&hasher.finalize()),
        })
    }
}

fn transition_reason(
    descriptor: &DurableRolloverSetDescriptor,
    logical_stream_id: u64,
    start: bool,
) -> String {
    for transition in &descriptor.transitions {
        let matches = if start {
            transition.proposed_logical_stream_id == logical_stream_id
        } else {
            transition.current_logical_stream_id == logical_stream_id
        };
        if matches {
            return if transition.reason == "same_pair_time_rollback" {
                "time_rollback"
            } else {
                "epoch_change"
            }
            .to_string();
        }
    }
    if start {
        "session_start".to_string()
    } else {
        "session_end".to_string()
    }
}

fn parse_reason(value: &str) -> Result<SegmentReason, String> {
    match value {
        "session_start" => Ok(SegmentReason::SessionStart),
        "epoch_change" => Ok(SegmentReason::EpochChange),
        "time_rollback" => Ok(SegmentReason::TimeRollback),
        "session_end" => Ok(SegmentReason::SessionEnd),
        _ => Err(format!("registered segment reason {value:?} is invalid")),
    }
}

fn register_entry(product_root: &Path, entry: &RegistryEntry) -> Result<(), String> {
    let logical_stream_bytes = entry.ticket.logical_stream_id.to_le_bytes();
    let stem_digest = structured_digest(
        b"product-stitch-registry-stem-v1",
        &[
            entry.ticket.session_nonce.as_bytes(),
            entry.ticket.process_start_identity.as_bytes(),
            &logical_stream_bytes,
        ],
    );
    let path = campaign_registry_dir(product_root, &entry.ticket.campaign_key_hex)
        .join(format!(
            "{:020}-{}.json",
            entry.ticket.play_order,
            &stem_digest[..24]
        ));
    write_immutable_json(&path, entry)
}

fn load_campaign_entries(
    product_root: &Path,
    campaign_hex: &str,
    machine_scope: &str,
) -> Result<Vec<RegistryEntry>, String> {
    let dir = campaign_registry_dir(product_root, campaign_hex);
    let mut entries = Vec::new();
    for candidate in fs::read_dir(&dir)
        .map_err(|error| format!("read campaign stitch registry failed: {error}"))?
    {
        let candidate = candidate.map_err(|error| format!("read registry entry failed: {error}"))?;
        let metadata = candidate
            .file_type()
            .map_err(|error| format!("inspect registry entry failed: {error}"))?;
        let file_name = candidate.file_name();
        let file_name = file_name
            .to_str()
            .ok_or("campaign stitch registry contains a non-UTF-8 entry")?;
        if metadata.is_file()
            && file_name.starts_with('.')
            && file_name.contains(".incoming-")
        {
            continue;
        }
        if !metadata.is_file() || candidate.path().extension().and_then(|v| v.to_str()) != Some("json") {
            return Err("campaign stitch registry contains a non-JSON/non-file entry".into());
        }
        let value: RegistryEntry = serde_json::from_value(read_strict_json(&candidate.path())?)
            .map_err(|error| format!("decode campaign registry entry failed: {error}"))?;
        if value.ticket.campaign_key_hex != campaign_hex
            || value.ticket.machine_scope_sha256 != machine_scope
        {
            return Err("campaign registry contains foreign campaign/machine provenance".into());
        }
        validate_registry_entry(product_root, &value)?;
        entries.push(value);
    }
    let mut ranks = BTreeSet::new();
    let mut play_order_owners = BTreeMap::new();
    for entry in &entries {
        let owner = (
            entry.ticket.session_nonce.as_str(),
            entry.ticket.process_start_identity.as_str(),
        );
        if play_order_owners
            .insert(entry.ticket.play_order, owner)
            .is_some_and(|existing| existing != owner)
        {
            return Err(
                "campaign registry maps one append-ticket rank to multiple sessions".into(),
            );
        }
        let rank = (entry.ticket.play_order, entry.ticket.logical_stream_id);
        if !ranks.insert(rank) {
            return Err("campaign registry contains a duplicate ticket/segment rank".into());
        }
    }
    Ok(entries)
}

fn validate_registry_entry(product_root: &Path, entry: &RegistryEntry) -> Result<(), String> {
    if entry.schema != REGISTRY_SCHEMA {
        return Err("unsupported product stitch registry schema".into());
    }
    validate_ticket_receipt(&entry.ticket)?;
    for digest in [
        &entry.package_sha256,
        &entry.raw_sha256,
        &entry.manifest_sha256,
    ] {
        require_sha(digest)?;
    }
    verify_file_sha(
        &join_relative(product_root, &entry.package_path)?,
        &entry.package_sha256,
    )?;
    verify_file_sha(
        &join_relative(product_root, &entry.raw_path)?,
        &entry.raw_sha256,
    )?;
    verify_file_sha(
        &join_relative(product_root, &entry.manifest_path)?,
        &entry.manifest_sha256,
    )?;
    let _ = parse_reason(&entry.start_reason)?;
    let _ = parse_reason(&entry.stop_reason)?;
    Ok(())
}

fn validate_manifest_identity(manifest: &Value, entry: &RegistryEntry) -> Result<(), String> {
    if required_pointer_string(manifest, "/campaign_identity/state")? != "known"
        || required_pointer_sha(manifest, "/campaign_identity/campaign_id")?
            != entry.ticket.campaign_key_hex
        || required_pointer_sha(manifest, "/campaign_identity/stream_header_sha256")?
            != entry.ticket.identity_evidence_sha256
        || required_pointer_sha(manifest, "/stream_storage/decoded_sha256")?
            != entry.raw_sha256
        || manifest.get("logical_stream_id").and_then(Value::as_u64)
            != Some(entry.ticket.logical_stream_id)
    {
        return Err("registered manifest disagrees with ticket/CID1 authority".into());
    }
    Ok(())
}

fn validate_ticket_receipt(receipt: &TicketReceipt) -> Result<(), String> {
    if receipt.schema != TICKET_SCHEMA {
        return Err("unsupported product stitch ticket schema".into());
    }
    require_sha(&receipt.campaign_key_hex)?;
    require_sha(&receipt.identity_evidence_sha256)?;
    require_sha(&receipt.machine_scope_sha256)?;
    let ticket = receipt.ticket()?;
    if ticket.session_nonce.trim() != ticket.session_nonce
        || ticket.process_start_identity.trim() != ticket.process_start_identity
        || ticket.session_nonce.is_empty()
        || ticket.process_start_identity.is_empty()
        || ticket.session_nonce.chars().any(char::is_control)
        || ticket.process_start_identity.chars().any(char::is_control)
    {
        return Err("product stitch ticket text authority is invalid".into());
    }
    Ok(())
}

fn read_ticket(run_dir: &Path, logical_stream_id: u64) -> Result<TicketReceipt, String> {
    serde_json::from_value(read_strict_json(&ticket_path(run_dir, logical_stream_id))?)
        .map_err(|error| format!("decode product stitch ticket failed: {error}"))
        .and_then(|ticket: TicketReceipt| {
            validate_ticket_receipt(&ticket)?;
            if ticket.logical_stream_id != logical_stream_id {
                return Err("product stitch ticket logical id disagrees with its path".into());
            }
            Ok(ticket)
        })
}

fn ticket_path(run_dir: &Path, logical_stream_id: u64) -> PathBuf {
    run_dir
        .join("stitching")
        .join("tickets")
        .join(format!("{logical_stream_id:020}.json"))
}

fn campaign_registry_dir(product_root: &Path, campaign_hex: &str) -> PathBuf {
    product_root
        .join("stitching")
        .join("campaigns")
        .join(campaign_hex)
        .join("entries")
}

fn machine_scope(product_root: &Path) -> Result<String, String> {
    let root = product_root.join("stitching");
    fs::create_dir_all(&root)
        .map_err(|error| format!("create product stitch root failed: {error}"))?;
    let path = root.join(MACHINE_SCOPE_FILE);
    match fs::read(&path) {
        Ok(bytes) => validate_machine_scope(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let secret = crate::auto_attach::staging::generate_secret_128()?;
            write_immutable_bytes(&path, &secret)?;
            validate_machine_scope(&fs::read(&path).map_err(|error| {
                format!("read persisted machine stitch scope failed: {error}")
            })?)
        }
        Err(error) => Err(format!("read machine stitch scope failed: {error}")),
    }
}

fn validate_machine_scope(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() != 16 || bytes.iter().all(|byte| *byte == 0) {
        return Err("machine stitch scope is corrupt".into());
    }
    Ok(Sha256::hex(bytes))
}

fn require_direct_run(product_root: &Path, run_dir: &Path) -> Result<(), String> {
    if !product_root.is_absolute()
        || !run_dir.is_absolute()
        || run_dir.parent() != Some(product_root)
    {
        return Err("product stitch run directory is not a direct child of product root".into());
    }
    Ok(())
}

fn required_relative(value: &Value, field: &str) -> Result<String, String> {
    let path = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("rollover package member lacks {field}"))?;
    validate_relative(path)?;
    Ok(path.to_string())
}

fn require_exact_object_keys(
    value: &Value,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} is not an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{context} has an unknown or missing field"));
    }
    Ok(())
}

fn product_relative_from_run(
    product_root: &Path,
    run_dir: &Path,
    run_relative: &str,
) -> Result<String, String> {
    validate_relative(run_relative)?;
    let run_name = run_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("product run directory name is not UTF-8")?;
    validate_relative(run_name)?;
    let relative = format!("{run_name}/{run_relative}");
    let resolved = join_relative(product_root, &relative)?;
    if !resolved.starts_with(run_dir) {
        return Err("product-relative stitch input escaped its run directory".into());
    }
    Ok(relative)
}

fn join_relative(root: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_relative(relative)?;
    Ok(root.join(relative))
}

fn validate_relative(relative: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe product-relative stitch path {relative:?}"));
    }
    Ok(())
}

fn required_sha(value: &Value, field: &str) -> Result<String, String> {
    let digest = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("rollover package member lacks {field}"))?;
    require_sha(digest)?;
    Ok(digest.to_string())
}

fn required_pointer_sha(value: &Value, pointer: &str) -> Result<String, String> {
    let digest = required_pointer_string(value, pointer)?;
    require_sha(&digest)?;
    Ok(digest)
}

fn required_pointer_string(value: &Value, pointer: &str) -> Result<String, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.trim() == *value)
        .map(str::to_string)
        .ok_or_else(|| format!("registered manifest lacks valid {pointer}"))
}

fn require_sha(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("stitch authority digest is not canonical lowercase SHA-256".into())
    }
}

fn decode_sha256(value: &str) -> Result<Vec<u8>, String> {
    require_sha(value)?;
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(32);
    for pair in bytes.chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn structured_digest(domain: &[u8], fields: &[&[u8]]) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(domain.len() as u64).to_le_bytes());
    bytes.extend_from_slice(domain);
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_le_bytes());
        bytes.extend_from_slice(field);
    }
    Sha256::hex(&bytes)
}

fn hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("invalid lowercase hex".into()),
    }
}

fn verify_file_sha(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| {
        format!("read registered stitch input {} failed: {error}", path.display())
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            format!("read registered stitch input {} failed: {error}", path.display())
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if to_hex(&hasher.finalize()) != expected {
        return Err(format!("registered stitch input {} changed", path.display()));
    }
    Ok(())
}

fn read_strict_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("read stitch control {} failed: {error}", path.display()))?;
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(format!("stitch control {} exceeds 16 MiB", path.display()));
    }
    snapshot::parse_json_strict(&bytes)
        .map_err(|error| format!("strict-decode stitch control {} failed: {error}", path.display()))
}

fn write_immutable_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("encode immutable stitch control failed: {error}"))?;
    write_immutable_bytes(path, &bytes)
}

fn write_immutable_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create immutable stitch directory failed: {error}"))?;
    }
    if path.exists() {
        return compare_immutable(path, bytes);
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("immutable stitch control has no UTF-8 file name")?;
    for attempt in 0..128u32 {
        let temp = path.with_file_name(format!(
            ".{file_name}.incoming-{}-{attempt}",
            std::process::id()
        ));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("create immutable stitch temp failed: {error}"))
            }
        };
        if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&temp);
            return Err(format!("persist immutable stitch temp failed: {error}"));
        }
        drop(file);
        match rename_new_durable(&temp, path) {
            Ok(()) => return Ok(()),
            Err(_) if path.exists() => {
                let _ = fs::remove_file(&temp);
                return compare_immutable(path, bytes);
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(format!("publish immutable stitch control failed: {error}"));
            }
        }
    }
    Err("could not allocate an immutable stitch temp after 128 attempts".into())
}

#[cfg(not(windows))]
fn rename_new_durable(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::hard_link(source, destination)?;
    fs::remove_file(source)?;
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "immutable stitch destination has no parent",
        )
    })?;
    fs::File::open(parent)?.sync_all()
}

#[cfg(windows)]
fn rename_new_durable(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let encode = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let source = encode(source);
    let destination = encode(destination);
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn compare_immutable(path: &Path, expected: &[u8]) -> Result<(), String> {
    let existing = fs::read(path)
        .map_err(|error| format!("read immutable stitch control failed: {error}"))?;
    if existing == expected {
        Ok(())
    } else {
        Err(format!(
            "immutable stitch control {} already exists with different bytes",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapshot::{pack_with, PackEntry, PackIdentity, PackOptions};

    fn record(sequence: u64, hour: i32, record_type: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        bytes.extend_from_slice(b"H4CR");
        bytes.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        bytes.extend_from_slice(&sequence.to_le_bytes());
        bytes.extend_from_slice(&hour.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&record_type.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn descriptor_container(
        declared_bundle_len: u64,
        smuggle_role_outside_prefix: bool,
        extra_wire_entry: bool,
    ) -> Container {
        let game = "descriptor-test";
        const BUNDLE: &str = "rebuilt_wire/bundle.bin";
        const PROVINCES: &str = "rebuilt_wire/provinces.bin";
        let bundle_bytes = b"bundle-body".to_vec();
        let province_bytes = b"province-body".to_vec();
        let bundle_descriptor = wire_product::BundleEntry::from_bytes(
            BUNDLE,
            wire_product::ArtifactRole::BundleMetadata,
            bundle_bytes.clone(),
        )
        .unwrap();
        let province_descriptor = wire_product::BundleEntry::from_bytes(
            PROVINCES,
            wire_product::ArtifactRole::ProvinceCatalog,
            province_bytes.clone(),
        )
        .unwrap();
        let descriptors = vec![
            serde_json::json!({
                "path": BUNDLE,
                "role": wire_product::ArtifactRole::BundleMetadata.as_str(),
                "bytes": declared_bundle_len,
                "blake3": to_hex(&bundle_descriptor.blake3),
            }),
            serde_json::json!({
                "path": PROVINCES,
                "role": wire_product::ArtifactRole::ProvinceCatalog.as_str(),
                "bytes": province_bytes.len(),
                "blake3": to_hex(&province_descriptor.blake3),
            }),
        ];
        let index = serde_json::to_vec(&serde_json::json!({
            "game_id": game,
            "schema_version": "test",
            "snapshots": [],
            "rebuilt_wire": {
                "present": true,
                "root": "rebuilt_wire",
                "semantic_reopen_required_before_publication": true,
                "entries": descriptors,
            },
        }))
        .unwrap();
        let mut entries = vec![
            PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: index,
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/{BUNDLE}"),
                bytes: bundle_bytes,
                role: wire_product::ArtifactRole::BundleMetadata.as_str(),
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/{PROVINCES}"),
                bytes: province_bytes,
                role: wire_product::ArtifactRole::ProvinceCatalog.as_str(),
                hour_index: None,
            },
        ];
        if smuggle_role_outside_prefix {
            entries.push(PackEntry {
                path: format!("payload/{game}/decoy.bin"),
                bytes: vec![1],
                role: wire_product::ArtifactRole::StateBlock.as_str(),
                hour_index: None,
            });
        }
        if extra_wire_entry {
            entries.push(PackEntry {
                path: format!("payload/{game}/rebuilt_wire/unlisted-gap.bin"),
                bytes: vec![2],
                role: wire_product::ArtifactRole::GapRecord.as_str(),
                hour_index: None,
            });
        }
        let bytes = pack_with(
            game,
            &entries,
            &PackIdentity::default(),
            false,
            &PackOptions::default(),
        )
        .unwrap();
        Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap()
    }

    #[test]
    fn registered_frame_scan_retains_exact_source_extents() {
        let first_raw_hour = compact_contract::calendar::DATE_EPOCH_CONST
            + ((1936 * 365 + 10) * 24 + 5);
        let first_hour_index = compact_contract::calendar::hour_index_from_raw(first_raw_hour)
            .expect("fixture raw hour");
        let mut raw = record(0, first_raw_hour, 5, b"cid1");
        let first_start = raw.len() as u64;
        raw.extend(record(1, first_raw_hour, 1, b"world"));
        raw.extend(record(
            2,
            first_raw_hour,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0; 32],
        ));
        let second_start = raw.len() as u64;
        raw.extend(record(3, first_raw_hour + 1, 1, b"world-2"));
        raw.extend(record(
            4,
            first_raw_hour + 1,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0; 32],
        ));

        let frames = scan_complete_frames(&raw).unwrap();
        assert_eq!(frames.len(), 2);
        let first = frames.get(&first_hour_index).unwrap();
        assert_eq!((first.first_sequence, first.last_sequence), (1, 2));
        assert_eq!(first.start_byte, first_start);
        assert_eq!(first.end_byte, second_start);
        let second = frames.get(&(first_hour_index + 1)).unwrap();
        assert_eq!((second.first_sequence, second.last_sequence), (3, 4));
        assert_eq!(second.start_byte, second_start);
        assert_eq!(second.end_byte, raw.len() as u64);
        assert_eq!(scan_source_bounds(&raw).unwrap(), ((0, 4), (0, raw.len() as u64)));
    }

    #[test]
    fn registered_frame_scan_rejects_sequence_and_hour_ambiguity() {
        let raw_hour = compact_contract::calendar::DATE_EPOCH_CONST
            + ((1936 * 365 + 10) * 24 + 5);
        let mut gap = record(0, raw_hour, 5, b"cid1");
        gap.extend(record(2, raw_hour, 1, b"world"));
        assert!(scan_complete_frames(&gap)
            .unwrap_err()
            .contains("sequence"));

        let mut interleaved = record(0, raw_hour, 5, b"cid1");
        interleaved.extend(record(1, raw_hour, 1, b"world"));
        interleaved.extend(record(
            2,
            raw_hour + 1,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0; 32],
        ));
        assert!(scan_complete_frames(&interleaved)
            .unwrap_err()
            .contains("interleaves"));
    }

    #[test]
    fn product_relative_paths_never_escape_the_run() {
        assert!(validate_relative("run/logical-streams/member.h4cr").is_ok());
        for invalid in [
            "../escape",
            "run/../escape",
            "/absolute",
            r"run\windows-alias",
            "",
        ] {
            assert!(validate_relative(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn package_control_objects_reject_schema_drift() {
        let exact = serde_json::json!({"schema":"v1", "ordered_streams":[]});
        assert!(require_exact_object_keys(
            &exact,
            &["schema", "ordered_streams"],
            "fixture",
        )
        .is_ok());

        let unknown = serde_json::json!({
            "schema":"v1",
            "ordered_streams":[],
            "unreviewed":true,
        });
        assert!(require_exact_object_keys(
            &unknown,
            &["schema", "ordered_streams"],
            "fixture",
        )
        .is_err());
    }

    #[test]
    fn registered_rebuilt_wire_discovery_joins_metadata_without_loading_bodies() {
        let exact = descriptor_container(b"bundle-body".len() as u64, false, false);
        let descriptors = registered_rebuilt_wire_descriptors(&exact).unwrap();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].path, "rebuilt_wire/bundle.bin");
        assert_eq!(descriptors[1].path, "rebuilt_wire/provinces.bin");

        let wrong_length = descriptor_container(999, false, false);
        assert!(registered_rebuilt_wire_descriptors(&wrong_length)
            .unwrap_err()
            .contains("metadata differs"));

        let smuggled = descriptor_container(b"bundle-body".len() as u64, true, false);
        assert!(registered_rebuilt_wire_descriptors(&smuggled)
            .unwrap_err()
            .contains("outside canonical prefix"));

        let extra = descriptor_container(b"bundle-body".len() as u64, false, true);
        assert!(registered_rebuilt_wire_descriptors(&extra)
            .unwrap_err()
            .contains("cardinality differs"));
    }
}
