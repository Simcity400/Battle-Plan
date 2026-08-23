//! Offline RC content-winner predicate tool. Mints a
//! `RawEffectiveVfsWinnerIndexBinding` from a sealed capture's source-inventory
//! v3 side-car plus the sealed session manifest, validates it, joins every
//! requested RC-family input path against both the minted winners and the
//! manifest's required-content keys, and evaluates the RC-CAP-LABELS predicate
//! where a NEGATIVE outcome is first-class and filed by name.
//!
//! Exit codes: 0 = binding valid, every requested family input bound and
//! agreeing with the manifest; 2 = named refusal(s), recorded in the report;
//! 1 = usage or IO error.

use portable_content_manifest::{
    normalize_game_relative_path, ContentLayerKind, ContentManifest, UsedContentKey,
};
use production_render_submit_live_lane::{RawEffectiveVfsOpenObservation, RawEffectiveVfsWinnerIndexBinding};
use recorder_host::production_vfs_winner_binding::build_effective_vfs_winner_binding_from_bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const REPORT_SCHEMA: &str = "extractor.rc_winner_binding_audit.v1";
const FAMILIES_SCHEMA: &str = "extractor.rc_family_inputs.v1";
const OBSERVATIONS_SCHEMA: &str = "extractor.rc_open_observations.v1";
const LABELS_SHADER_PATH: &str = "gfx/fx/mapname.shader";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationsDocument {
    schema: String,
    /// Pins this claim file to the exact binding being audited; a stale or
    /// mixed-up observations file refuses instead of silently joining.
    binding_rows_sha256: String,
    process_start_nonce: u64,
    observations: Vec<ObservedOpenRow>,
}

/// Field-for-field mirror of `RawEffectiveVfsOpenObservation`: the row carries
/// its own claimed winner identity and opened-payload facts, so nothing is
/// synthesized from the binding; `authorize_open` is a join check, and the
/// document-level binding pin keeps stale or mixed-up claim files out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedOpenRow {
    observation_sequence: u64,
    authority_manifest_sha256: String,
    canonical_logical_path: String,
    content: ObservedOpenContent,
    stream_handle: usize,
    data_ptr: Option<usize>,
    opened_payload_sha256: String,
    opened_payload_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedOpenContent {
    logical_path: String,
    winning_layer_ordinal: u32,
    layer_kind: String,
    layer_id: String,
    source_sha256: String,
    payload_sha256: String,
    payload_bytes: u64,
}

#[derive(Serialize)]
struct AuditReport {
    schema: &'static str,
    status: &'static str,
    refusals: Vec<String>,
    source_inventory_path: String,
    source_inventory_sha256: String,
    manifest_path: String,
    manifest_sha256: String,
    process_start_nonce: u64,
    authority_manifest_sha256: String,
    declared_row_count: u64,
    ordered_rows_sha256: String,
    winners: Vec<WinnerProjection>,
    families_input: Option<EvidenceInputBinding>,
    open_observations_input: Option<EvidenceInputBinding>,
    families: BTreeMap<String, FamilyOutcome>,
    rc_cap_labels: LabelsOutcome,
}

#[derive(Clone, Serialize)]
struct EvidenceInputBinding {
    path: String,
    sha256: String,
}

#[derive(Clone, Serialize)]
struct WinnerProjection {
    logical_path: String,
    winning_layer_ordinal: u32,
    layer_kind: String,
    layer_id: String,
    source_sha256: String,
    payload_sha256: String,
    payload_bytes: u64,
}

#[derive(Default, Serialize)]
struct FamilyOutcome {
    requested_paths: Vec<String>,
    bound_paths: usize,
    missing_winners: Vec<String>,
    missing_from_required_content: Vec<String>,
    winner_disagrees_with_required_content: Vec<String>,
}

#[derive(Serialize)]
struct LabelsOutcome {
    status: &'static str,
    detail: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rc_winner_binding_audit: {error}");
        std::process::exit(1);
    }
}

fn usage() -> String {
    "usage: rc_winner_binding_audit --source-inventory <source_inventory_v3.json> \
     --manifest <native_session_manifest.json> --out <audit_report.json> \
     --process-start-nonce <u64> [--families <rc_family_inputs.json>] \
     [--open-observations <open_observations.json>]"
        .to_string()
}

struct Args {
    source_inventory: PathBuf,
    manifest: PathBuf,
    out: PathBuf,
    process_start_nonce: u64,
    families: Option<PathBuf>,
    open_observations: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut source_inventory = None;
    let mut manifest = None;
    let mut out = None;
    let mut process_start_nonce = 0u64;
    let mut families = None;
    let mut open_observations = None;
    let mut args = env::args_os().skip(1);
    while let Some(flag) = args.next() {
        let Some(flag) = flag.to_str().map(str::to_owned) else {
            return Err(format!("non-UTF-8 argument {flag:?}; {}", usage()));
        };
        let mut next_value = |flag: &str| -> Result<PathBuf, String> {
            args.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{flag} requires a value; {}", usage()))
        };
        match flag.as_str() {
            "--source-inventory" => source_inventory = Some(next_value(&flag)?),
            "--manifest" => manifest = Some(next_value(&flag)?),
            "--out" => out = Some(next_value(&flag)?),
            "--families" => families = Some(next_value(&flag)?),
            "--open-observations" => open_observations = Some(next_value(&flag)?),
            "--process-start-nonce" => {
                let value = next_value(&flag)?;
                process_start_nonce = value
                    .to_str()
                    .ok_or_else(|| format!("--process-start-nonce must be a decimal u64"))?
                    .parse::<u64>()
                    .map_err(|error| format!("--process-start-nonce: {error}"))?;
            }
            other => return Err(format!("unknown argument {other:?}; {}", usage())),
        }
    }
    Ok(Args {
        source_inventory: source_inventory.ok_or_else(usage)?,
        manifest: manifest.ok_or_else(usage)?,
        out: out.ok_or_else(usage)?,
        process_start_nonce,
        families,
        open_observations,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if args.process_start_nonce == 0 {
        return Err(format!(
            "--process-start-nonce must be nonzero (the binding law refuses nonce 0); {}",
            usage()
        ));
    }
    let inventory_bytes = stable_read(&args.source_inventory, "source inventory")?;
    let manifest_bytes = stable_read(&args.manifest, "session manifest")?;

    let manifest_document: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("decode session manifest: {error}"))?;
    let content_section = manifest_document
        .get("portable_content_manifest")
        .ok_or("session manifest carries no portable_content_manifest section")?;
    let content: ContentManifest = serde_json::from_value(content_section.clone())
        .map_err(|error| format!("decode portable_content_manifest: {error}"))?;
    if content.schema != portable_content_manifest::SCHEMA {
        return Err(format!(
            "portable_content_manifest schema is {} not {}",
            content.schema,
            portable_content_manifest::SCHEMA
        ));
    }
    content
        .validate()
        .map_err(|error| format!("portable_content_manifest validate: {error}"))?;

    // The builder consumes the exact stable-read bytes, so the report digest
    // and the folded input are the same slice by construction.
    let mut refusals = Vec::new();
    let binding = build_effective_vfs_winner_binding_from_bytes(
        &inventory_bytes,
        &content.ordered_layers,
        args.process_start_nonce,
        &content.effective_vfs_manifest_sha256,
    )?;
    binding.validate()?;

    let required_by_path = content
        .required_content
        .iter()
        .map(|row| (row.content.normalized_game_relative_path.clone(), &row.content))
        .collect::<BTreeMap<_, _>>();

    let mut families_input = None;
    let mut families = BTreeMap::new();
    if let Some(path) = &args.families {
        let families_bytes = stable_read(path, "families document")?;
        let document = parse_families(&families_bytes)?;
        for (family, paths) in document.families {
            let outcome =
                evaluate_family(&family, &paths, &binding, &required_by_path, &mut refusals);
            families.insert(family, outcome);
        }
        families_input = Some(EvidenceInputBinding {
            path: path.display().to_string(),
            sha256: hex_sha256(&families_bytes),
        });
    }

    let mut open_observations_input = None;
    let observations_bytes = match &args.open_observations {
        Some(path) => {
            let bytes = stable_read(path, "open observations")?;
            open_observations_input = Some(EvidenceInputBinding {
                path: path.display().to_string(),
                sha256: hex_sha256(&bytes),
            });
            Some(bytes)
        }
        None => None,
    };

    let rc_cap_labels = labels_outcome(observations_bytes.as_deref(), &binding, &mut refusals)?;

    let status = if refusals.is_empty() {
        "SEALED_OK"
    } else {
        "REFUSED"
    };
    let report = AuditReport {
        schema: REPORT_SCHEMA,
        status,
        refusals: refusals.clone(),
        source_inventory_path: args.source_inventory.display().to_string(),
        source_inventory_sha256: hex_sha256(&inventory_bytes),
        manifest_path: args.manifest.display().to_string(),
        manifest_sha256: hex_sha256(&manifest_bytes),
        process_start_nonce: args.process_start_nonce,
        authority_manifest_sha256: content.effective_vfs_manifest_sha256.clone(),
        declared_row_count: binding.declared_row_count,
        ordered_rows_sha256: binding.ordered_rows_sha256.clone(),
        winners: binding
            .ordered_rows
            .iter()
            .map(|row| WinnerProjection {
                logical_path: row.content.logical_path.clone(),
                winning_layer_ordinal: row.content.winning_layer_ordinal,
                layer_kind: row.content.layer_kind.clone(),
                layer_id: row.content.layer_id.clone(),
                source_sha256: row.content.source_sha256.clone(),
                payload_sha256: row.content.payload_sha256.clone(),
                payload_bytes: row.content.payload_bytes,
            })
            .collect(),
        families_input,
        open_observations_input,
        families,
        rc_cap_labels,
    };

    write_report(&args.out, &report)?;
    if status == "REFUSED" {
        for refusal in &refusals {
            eprintln!("rc_winner_binding_audit refusal: {refusal}");
        }
        eprintln!(
            "rc_winner_binding_audit: REFUSED with {} named reason(s); report at {}",
            refusals.len(),
            args.out.display()
        );
        std::process::exit(2);
    }
    println!(
        "rc_winner_binding_audit: OK {} winner row(s); report at {}",
        report.winners.len(),
        args.out.display()
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FamiliesDocument {
    schema: String,
    provenance: Option<String>,
    families: BTreeMap<String, Vec<String>>,
}

fn parse_families(bytes: &[u8]) -> Result<FamiliesDocument, String> {
    let mut document: FamiliesDocument =
        serde_json::from_slice(bytes).map_err(|error| format!("decode families: {error}"))?;
    if document.schema != FAMILIES_SCHEMA {
        return Err(format!("families document schema must be {FAMILIES_SCHEMA}"));
    }
    if document.families.is_empty() {
        return Err("families document names no families".into());
    }
    if document
        .families
        .values()
        .any(|paths| paths.is_empty())
    {
        return Err("families document carries an empty family".into());
    }
    for paths in document.families.values_mut() {
        let normalized = paths
            .iter()
            .map(|raw| {
                normalize_game_relative_path(raw)
                    .map_err(|error| format!("families path {raw:?}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        *paths = normalized;
    }
    Ok(document)
}

fn evaluate_family(
    family: &str,
    paths: &[String],
    binding: &RawEffectiveVfsWinnerIndexBinding,
    required_by_path: &BTreeMap<String, &UsedContentKey>,
    refusals: &mut Vec<String>,
) -> FamilyOutcome {
    let mut outcome = FamilyOutcome {
        requested_paths: paths.to_vec(),
        ..FamilyOutcome::default()
    };
    for path in paths {
        let winner_index = binding
            .ordered_rows
            .binary_search_by(|row| row.content.logical_path.as_str().cmp(path.as_str()));
        let Ok(winner_index) = winner_index else {
            let reason = format!("RC_FAMILY_INPUT_UNBOUND family={family} path={path}");
            refusals.push(reason.clone());
            outcome.missing_winners.push(reason);
            continue;
        };
        outcome.bound_paths += 1;
        let row = &binding.ordered_rows[winner_index];
        let Some(key) = required_by_path.get(path) else {
            let reason =
                format!("RC_FAMILY_PATH_ABSENT_FROM_REQUIRED_CONTENT family={family} path={path}");
            refusals.push(reason.clone());
            outcome.missing_from_required_content.push(reason);
            continue;
        };
        let agrees = key.winning_layer_ordinal == row.content.winning_layer_ordinal
            && layer_kind_name(key) == row.content.layer_kind
            && key.layer_id == row.content.layer_id
            && key.source_sha256 == row.content.source_sha256
            && key.payload_sha256 == row.content.payload_sha256
            && key.payload_bytes == row.content.payload_bytes;
        if !agrees {
            let reason = format!(
                "RC_FAMILY_WINNER_DISAGREES family={family} path={path} binding=({},{},{},{},{},\
                 {}b) manifest=({},{},{},{},{},{}b)",
                row.content.winning_layer_ordinal,
                row.content.layer_kind,
                row.content.layer_id,
                row.content.source_sha256,
                row.content.payload_sha256,
                row.content.payload_bytes,
                key.winning_layer_ordinal,
                layer_kind_name(key),
                key.layer_id,
                key.source_sha256,
                key.payload_sha256,
                key.payload_bytes,
            );
            outcome.winner_disagrees_with_required_content.push(reason.clone());
            refusals.push(reason);
        }
    }
    outcome
}

fn layer_kind_name(key: &UsedContentKey) -> String {
    match key.layer_kind {
        ContentLayerKind::Base => "base",
        ContentLayerKind::IntegratedDlc => "integrated_dlc",
        ContentLayerKind::Dlc => "dlc",
        ContentLayerKind::Mod => "mod",
    }
    .to_string()
}

fn labels_outcome(
    observations: Option<&[u8]>,
    binding: &RawEffectiveVfsWinnerIndexBinding,
    refusals: &mut Vec<String>,
) -> Result<LabelsOutcome, String> {
    let Some(bytes) = observations else {
        return Ok(LabelsOutcome {
            // First-class negative: the production capture path installs no
            // VFS-open observation producer, so an unobserved shader open is
            // the expected sealed-capture outcome and must be filed by name
            // (RC-CAP-LABELS residual, PROOF.md section 9).
            status: "NEGATIVE_SHADER_OPEN_BYPASSED_OBSERVATION",
            detail: "No RawEffectiveVfsOpenObservation producer exists on the production capture \
                     path, so no observation could be authorized against this binding; the \
                     live shader-open naming leg stays open by name."
                .into(),
        });
    };
    let document: ObservationsDocument =
        serde_json::from_slice(bytes).map_err(|error| format!("decode open observations: {error}"))?;
    if document.schema != OBSERVATIONS_SCHEMA {
        return Err(format!(
            "open observations document schema must be {OBSERVATIONS_SCHEMA}"
        ));
    }
    if document.binding_rows_sha256 != binding.ordered_rows_sha256
        || document.process_start_nonce != binding.process_start_nonce
    {
        return Err(
            "open observations document names another binding (rows sha256 / nonce disagree); \
             regenerate it against this audit's binding"
                .into(),
        );
    }
    let mut shader_opens = 0usize;
    let mut authorized = 0usize;
    for (row_index, row) in document.observations.iter().enumerate() {
        let normalized_logical = normalize_game_relative_path(&row.canonical_logical_path)
            .map_err(|error| {
                format!(
                    "observation {row_index} path {:?}: {error}",
                    row.canonical_logical_path
                )
            })?;
        if normalized_logical != LABELS_SHADER_PATH {
            continue;
        }
        shader_opens += 1;
        if row.observation_sequence == 0 || row.stream_handle == 0 {
            refusals.push(format!(
                "RC_LABELS_OPEN_INCOMPLETE path={normalized_logical} sequence={} handle={}",
                row.observation_sequence, row.stream_handle
            ));
            continue;
        }
        let observation = observation_from_row(row)
            .map_err(|error| format!("observation {row_index}: {error}"))?;
        match binding.authorize_open(&observation) {
            Ok(()) => authorized += 1,
            Err(error) => refusals.push(format!(
                "RC_LABELS_OPEN_REFUSED_BY_BINDING path={normalized_logical} \
                 sequence={}: {error}",
                row.observation_sequence
            )),
        }
    }
    let status = if shader_opens > 0 && authorized > 0 {
        "PASS"
    } else if shader_opens > 0 {
        "NEGATIVE_OPEN_REFUSED_BY_BINDING"
    } else {
        "NEGATIVE_NO_SHADER_OPEN_RECORDED"
    };
    Ok(LabelsOutcome {
        status,
        detail: format!(
            "observations={shader_opens} authorized={authorized} \
             authority_manifest_sha256={}",
            binding.authority_manifest_sha256
        ),
    })
}

fn observation_from_row(row: &ObservedOpenRow) -> Result<RawEffectiveVfsOpenObservation, String> {
    use production_render_submit_live_lane::RawVfsContentIdentity;
    Ok(RawEffectiveVfsOpenObservation {
        observation_sequence: row.observation_sequence,
        authority_manifest_sha256: row.authority_manifest_sha256.clone(),
        canonical_logical_path: row.canonical_logical_path.clone(),
        content: RawVfsContentIdentity {
            logical_path: row.content.logical_path.clone(),
            winning_layer_ordinal: row.content.winning_layer_ordinal,
            layer_kind: row.content.layer_kind.clone(),
            layer_id: row.content.layer_id.clone(),
            source_sha256: row.content.source_sha256.clone(),
            payload_sha256: row.content.payload_sha256.clone(),
            payload_bytes: row.content.payload_bytes,
        },
        stream_handle: row.stream_handle,
        data_ptr: row.data_ptr,
        opened_payload_sha256: row.opened_payload_sha256.clone(),
        opened_payload_bytes: row.opened_payload_bytes,
    })
}

fn stable_read(path: &Path, what: &str) -> Result<Vec<u8>, String> {
    let first = fs::read(path).map_err(|error| format!("read {what} {}: {error}", path.display()))?;
    let second =
        fs::read(path).map_err(|error| format!("re-read {what} {}: {error}", path.display()))?;
    if first != second {
        return Err(format!("{what} {} changed while being read", path.display()));
    }
    Ok(first)
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_report(out: &Path, report: &AuditReport) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(|error| format!("encode report: {error}"))?;
    bytes.push(b'\n');
    match fs::OpenOptions::new().write(true).create_new(true).open(out) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(&bytes)
                .map_err(|error| format!("write report {}: {error}", out.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                fs::read(out).map_err(|error| format!("read existing {}: {error}", out.display()))?;
            if existing == bytes {
                Ok(())
            } else {
                Err(format!(
                    "--out {} already exists with different content; this tool never overwrites \
                     evidence — choose a new path",
                    out.display()
                ))
            }
        }
        Err(error) => Err(format!("create report {}: {error}", out.display())),
    }
}
