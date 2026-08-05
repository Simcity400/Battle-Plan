//! Process boundary for the canonical Python static-bundle verifier.
//!
//! Rust deliberately does not duplicate the bundle schema. The Python verifier
//! remains the single semantic authority; this module supplies bounded process
//! execution, captures its verified evidence, and detects byte/binding drift by
//! requiring an identical second verification before publication. Recorder
//! admission is explicitly the pre-injection phase: only a current, deferred
//! bundle is accepted here. Final token closure remains a separate post-capture
//! Python finalization/publication gate with exact capture provenance.

use serde_json::{json, Value};
use snapshot::sha256::Sha256;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_VERIFIER_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticBundleRequest {
    pub python: OsString,
    pub verifier: PathBuf,
    pub manifest: PathBuf,
    pub exe: PathBuf,
    pub mod_state: PathBuf,
    pub dlc_state: PathBuf,
    pub load_order: PathBuf,
    pub source_inventory: PathBuf,
    pub live_resolution_report: PathBuf,
    pub expected_process_id: u32,
    pub expected_process_creation_filetime_utc: u64,
    /// A selected product-modset corpus is necessarily acquired from an
    /// earlier process. A recorder-owned suspended launch may rebind only the
    /// process identity; Python still re-hashes every source, and the early
    /// launch path independently joins the selected runtime mod/DLC/VFS
    /// content before COMMIT. Late attach must leave this false.
    pub allow_prepared_process_identity_rebind: bool,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizedStaticBundleRequest {
    pub python: OsString,
    pub verifier: PathBuf,
    pub manifest: PathBuf,
    pub exe: PathBuf,
    pub mod_state: PathBuf,
    pub dlc_state: PathBuf,
    pub load_order: PathBuf,
    pub source_inventory: PathBuf,
    pub live_resolution_report: PathBuf,
    pub expected_process_id: u32,
    pub expected_process_creation_filetime_utc: u64,
    pub allow_prepared_process_identity_rebind: bool,
    pub expected_capture_id: String,
    pub expected_session_id: String,
    pub expected_stream_sha256: String,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedArtifact {
    pub role: String,
    pub path: PathBuf,
    pub file: String,
    pub sha256: String,
    pub artifact_sha256: String,
    pub source_gate_report_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedStaticBundle {
    pub bundle_id: String,
    pub product_modset_sha256: String,
    pub artifact_count: u64,
    pub artifacts: Vec<VerifiedArtifact>,
    pub evidence: Value,
}

impl VerifiedStaticBundle {
    pub fn as_json(&self) -> Value {
        self.evidence.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInvocation {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessResult {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait VerifierProcess {
    fn run(&self, invocation: &ProcessInvocation) -> Result<ProcessResult, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CommandVerifierProcess;

impl VerifierProcess for CommandVerifierProcess {
    fn run(&self, invocation: &ProcessInvocation) -> Result<ProcessResult, String> {
        if invocation.timeout.is_zero() {
            return Err("static bundle verifier timeout must be nonzero".to_string());
        }
        let mut child = Command::new(&invocation.program)
            .args(&invocation.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start static bundle verifier failed: {error}"))?;
        let started = Instant::now();
        loop {
            match child
                .try_wait()
                .map_err(|error| format!("poll static bundle verifier failed: {error}"))?
            {
                Some(_) => break,
                None if started.elapsed() >= invocation.timeout => {
                    let child_pid = child.id();
                    if let Err(error) = thread::Builder::new()
                        .name(format!("static-verifier-passive-reaper-{child_pid}"))
                        .spawn(move || {
                            let _ = child.wait_with_output();
                        })
                    {
                        eprintln!(
                            "failed to start passive static-verifier reaper after timeout: {error}"
                        );
                    }
                    return Err(format!(
                        "static bundle verifier timed out after {} ms",
                        invocation.timeout.as_millis()
                    ));
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("collect static bundle verifier output failed: {error}"))?;
        if output.stdout.len() > MAX_VERIFIER_OUTPUT_BYTES
            || output.stderr.len() > MAX_VERIFIER_OUTPUT_BYTES
        {
            return Err("static bundle verifier output exceeded 1 MiB bound".to_string());
        }
        Ok(ProcessResult {
            success: output.status.success(),
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

pub fn verify(request: &StaticBundleRequest) -> Result<VerifiedStaticBundle, String> {
    verify_with_process(request, &CommandVerifierProcess)
}

pub fn verify_unchanged(
    request: &StaticBundleRequest,
    admitted: &VerifiedStaticBundle,
) -> Result<VerifiedStaticBundle, String> {
    verify_unchanged_with_process(request, admitted, &CommandVerifierProcess)
}

pub fn verify_unchanged_with_process(
    request: &StaticBundleRequest,
    admitted: &VerifiedStaticBundle,
    process: &dyn VerifierProcess,
) -> Result<VerifiedStaticBundle, String> {
    let current = verify_with_process(request, process)?;
    if &current != admitted {
        return Err(
            "static bundle or executable/mod/DLC/load-order/source-inventory/live-resolution binding changed after admission"
                .to_string(),
        );
    }
    Ok(current)
}

pub fn finalize(
    admitted: &StaticBundleRequest,
    output_dir: &Path,
    emitted_tokens: &Path,
    capture_id: &str,
    session_id: &str,
    stream_sha256: &str,
) -> Result<(FinalizedStaticBundleRequest, VerifiedStaticBundle), String> {
    finalize_with_process(
        admitted,
        output_dir,
        emitted_tokens,
        capture_id,
        session_id,
        stream_sha256,
        &CommandVerifierProcess,
    )
}

pub fn finalize_with_process(
    admitted: &StaticBundleRequest,
    output_dir: &Path,
    emitted_tokens: &Path,
    capture_id: &str,
    session_id: &str,
    stream_sha256: &str,
    process: &dyn VerifierProcess,
) -> Result<(FinalizedStaticBundleRequest, VerifiedStaticBundle), String> {
    required_nonempty(capture_id, "capture_id")?;
    required_nonempty(session_id, "session_id")?;
    validate_sha(stream_sha256, "stream_sha256")?;
    if output_dir.exists() {
        return Err(format!(
            "finalized static bundle output already exists: {}",
            output_dir.display()
        ));
    }
    let verifier = canonical_file(&admitted.verifier, "static bundle verifier")?;
    let manifest = canonical_file(&admitted.manifest, "static bundle manifest")?;
    let bundle_dir = manifest
        .parent()
        .ok_or_else(|| "static bundle manifest has no parent directory".to_string())?
        .to_path_buf();
    let emitted_tokens = canonical_file(emitted_tokens, "emitted token inventory")?;
    let exe = canonical_file(&admitted.exe, "static bundle executable binding")?;
    let mod_state = canonical_file(&admitted.mod_state, "static bundle mod-state binding")?;
    let dlc_state = canonical_file(&admitted.dlc_state, "static bundle DLC-state binding")?;
    let load_order = canonical_file(&admitted.load_order, "static bundle load-order binding")?;
    let source_inventory = canonical_file(
        &admitted.source_inventory,
        "static bundle source-inventory binding",
    )?;
    let live_resolution_report = canonical_file(
        &admitted.live_resolution_report,
        "static bundle live-resolution-report binding",
    )?;
    let invocation = ProcessInvocation {
        program: admitted.python.clone(),
        args: vec![
            verifier.clone().into_os_string(),
            "finalize".into(),
            "--bundle".into(),
            bundle_dir.into_os_string(),
            "--out".into(),
            output_dir.as_os_str().to_os_string(),
            "--exe".into(),
            exe.clone().into_os_string(),
            "--mod-state".into(),
            mod_state.clone().into_os_string(),
            "--dlc-state".into(),
            dlc_state.clone().into_os_string(),
            "--load-order".into(),
            load_order.clone().into_os_string(),
            "--source-inventory".into(),
            source_inventory.clone().into_os_string(),
            "--live-resolution-report".into(),
            live_resolution_report.clone().into_os_string(),
            "--emitted-tokens".into(),
            emitted_tokens.into_os_string(),
            "--expected-capture-id".into(),
            capture_id.into(),
            "--expected-session-id".into(),
            session_id.into(),
            "--expected-stream-sha256".into(),
            stream_sha256.into(),
        ],
        timeout: admitted.timeout,
    };
    require_finalized_result(process.run(&invocation)?, "finalization")?;
    let request = FinalizedStaticBundleRequest {
        python: admitted.python.clone(),
        verifier,
        manifest: output_dir.join("static_bundle_manifest.json"),
        exe,
        mod_state,
        dlc_state,
        load_order,
        source_inventory,
        live_resolution_report,
        expected_process_id: admitted.expected_process_id,
        expected_process_creation_filetime_utc: admitted.expected_process_creation_filetime_utc,
        allow_prepared_process_identity_rebind: admitted
            .allow_prepared_process_identity_rebind,
        expected_capture_id: capture_id.to_string(),
        expected_session_id: session_id.to_string(),
        expected_stream_sha256: stream_sha256.to_string(),
        timeout: admitted.timeout,
    };
    let verified = verify_finalized_with_process(&request, process)?;
    Ok((request, verified))
}

pub fn verify_finalized(
    request: &FinalizedStaticBundleRequest,
) -> Result<VerifiedStaticBundle, String> {
    verify_finalized_with_process(request, &CommandVerifierProcess)
}

pub fn verify_finalized_unchanged(
    request: &FinalizedStaticBundleRequest,
    admitted: &VerifiedStaticBundle,
) -> Result<VerifiedStaticBundle, String> {
    let current = verify_finalized(request)?;
    if &current != admitted {
        return Err(
            "finalized static bundle or capture binding changed after finalization".to_string(),
        );
    }
    Ok(current)
}

pub fn verify_finalized_with_process(
    request: &FinalizedStaticBundleRequest,
    process: &dyn VerifierProcess,
) -> Result<VerifiedStaticBundle, String> {
    required_nonempty(&request.expected_capture_id, "expected_capture_id")?;
    required_nonempty(&request.expected_session_id, "expected_session_id")?;
    validate_sha(&request.expected_stream_sha256, "expected_stream_sha256")?;
    let verifier = canonical_file(&request.verifier, "static bundle verifier")?;
    let manifest = canonical_file(&request.manifest, "finalized static bundle manifest")?;
    if manifest.file_name().and_then(|name| name.to_str()) != Some("static_bundle_manifest.json") {
        return Err(
            "finalized static bundle manifest must be named static_bundle_manifest.json"
                .to_string(),
        );
    }
    let bundle_dir = manifest
        .parent()
        .ok_or_else(|| "finalized static bundle manifest has no parent directory".to_string())?
        .to_path_buf();
    let bindings = canonical_bindings(
        &request.exe,
        &request.mod_state,
        &request.dlc_state,
        &request.load_order,
        &request.source_inventory,
        &request.live_resolution_report,
    )?;
    let process_identity = verify_live_resolution_process_identity(
        &bindings.5,
        request.expected_process_id,
        request.expected_process_creation_filetime_utc,
        request.allow_prepared_process_identity_rebind,
    )?;
    let invocation = ProcessInvocation {
        program: request.python.clone(),
        args: vec![
            verifier.clone().into_os_string(),
            "verify".into(),
            "--bundle".into(),
            bundle_dir.clone().into_os_string(),
            "--exe".into(),
            bindings.0.clone().into_os_string(),
            "--mod-state".into(),
            bindings.1.clone().into_os_string(),
            "--dlc-state".into(),
            bindings.2.clone().into_os_string(),
            "--load-order".into(),
            bindings.3.clone().into_os_string(),
            "--source-inventory".into(),
            bindings.4.clone().into_os_string(),
            "--live-resolution-report".into(),
            bindings.5.clone().into_os_string(),
            "--expected-capture-id".into(),
            request.expected_capture_id.clone().into(),
            "--expected-session-id".into(),
            request.expected_session_id.clone().into(),
            "--expected-stream-sha256".into(),
            request.expected_stream_sha256.clone().into(),
        ],
        timeout: request.timeout,
    };
    let result = require_finalized_result(process.run(&invocation)?, "verification")?;
    let mut verified = verified_bundle_from_manifest(
        &manifest,
        &bundle_dir,
        &verifier,
        &bindings,
        &result,
        "post_capture_finalized",
        "passed",
    )?;
    verified
        .evidence
        .as_object_mut()
        .ok_or_else(|| "finalized static bundle evidence is not an object".to_string())?
        .insert(
            "capture_provenance".to_string(),
            json!({
                "capture_id": request.expected_capture_id,
                "session_id": request.expected_session_id,
                "stream_sha256": request.expected_stream_sha256,
            }),
        );
    verified
        .evidence
        .as_object_mut()
        .ok_or_else(|| "finalized static bundle evidence is not an object".to_string())?
        .insert("process_identity".to_string(), process_identity);
    Ok(verified)
}

pub fn verify_with_process(
    request: &StaticBundleRequest,
    process: &dyn VerifierProcess,
) -> Result<VerifiedStaticBundle, String> {
    let verifier = canonical_file(&request.verifier, "static bundle verifier")?;
    let manifest = canonical_file(&request.manifest, "static bundle manifest")?;
    if manifest.file_name().and_then(|name| name.to_str()) != Some("static_bundle_manifest.json") {
        return Err("static bundle manifest must be named static_bundle_manifest.json".to_string());
    }
    let bundle_dir = manifest
        .parent()
        .ok_or_else(|| "static bundle manifest has no parent directory".to_string())?
        .to_path_buf();
    let exe = canonical_file(&request.exe, "static bundle executable binding")?;
    let mod_state = canonical_file(&request.mod_state, "static bundle mod-state binding")?;
    let dlc_state = canonical_file(&request.dlc_state, "static bundle DLC-state binding")?;
    let load_order = canonical_file(&request.load_order, "static bundle load-order binding")?;
    let source_inventory = canonical_file(
        &request.source_inventory,
        "static bundle source-inventory binding",
    )?;
    let live_resolution_report = canonical_file(
        &request.live_resolution_report,
        "static bundle live-resolution-report binding",
    )?;
    let process_identity = verify_live_resolution_process_identity(
        &live_resolution_report,
        request.expected_process_id,
        request.expected_process_creation_filetime_utc,
        request.allow_prepared_process_identity_rebind,
    )?;

    let invocation = ProcessInvocation {
        program: request.python.clone(),
        args: vec![
            verifier.clone().into_os_string(),
            "verify".into(),
            "--bundle".into(),
            bundle_dir.clone().into_os_string(),
            "--exe".into(),
            exe.clone().into_os_string(),
            "--mod-state".into(),
            mod_state.clone().into_os_string(),
            "--dlc-state".into(),
            dlc_state.clone().into_os_string(),
            "--load-order".into(),
            load_order.clone().into_os_string(),
            "--source-inventory".into(),
            source_inventory.clone().into_os_string(),
            "--live-resolution-report".into(),
            live_resolution_report.clone().into_os_string(),
            "--allow-deferred-token-gate".into(),
        ],
        timeout: request.timeout,
    };
    let output = process.run(&invocation)?;
    if !output.success {
        return Err(format!(
            "static bundle verifier failed with code {:?}: {}",
            output.code,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let result: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("static bundle verifier returned invalid JSON: {error}"))?;
    if result.get("passed").and_then(Value::as_bool) != Some(true) {
        return Err("static bundle verifier did not return passed=true".to_string());
    }
    let bundle_id = required_sha(result.get("bundle_id"), "verifier bundle_id")?;
    let product_modset_sha256 = required_sha(
        result.get("product_modset_sha256"),
        "verifier product_modset_sha256",
    )?;
    let artifact_count = result
        .get("artifact_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "static bundle verifier omitted artifact_count".to_string())?;
    if result.get("token_join_status").and_then(Value::as_str) != Some("deferred") {
        return Err(
            "pre-injection static bundle verifier did not admit a deferred token gate".to_string(),
        );
    }
    if result.get("verification_phase").and_then(Value::as_str)
        != Some("pre_injection_deferred_allowed")
    {
        return Err("static bundle verifier returned the wrong admission phase".to_string());
    }

    // Semantic validity belongs exclusively to static_bundle.py. After it
    // passes, retain the exact verified roster and hashes for manifest evidence
    // and a byte-for-byte second-verification comparison.
    let manifest_bytes = read_bounded(&manifest, "static bundle manifest")?;
    let manifest_json: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("read verified static bundle evidence failed: {error}"))?;
    if manifest_json.get("bundle_id").and_then(Value::as_str) != Some(bundle_id.as_str()) {
        return Err("verifier result bundle_id differs from verified manifest".to_string());
    }
    let rows = manifest_json
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "verified static bundle manifest omitted artifact roster".to_string())?;
    if u64::try_from(rows.len()).ok() != Some(artifact_count) {
        return Err("verifier artifact_count differs from manifest roster".to_string());
    }
    let mut artifacts = Vec::with_capacity(rows.len());
    for row in rows {
        let role = required_string(row.get("role"), "artifact role")?;
        let file = required_string(row.get("file"), "artifact file")?;
        let path = canonical_file(&bundle_dir.join(&file), "bundled static artifact")?;
        if !path.starts_with(&bundle_dir) {
            return Err(format!(
                "bundled static artifact path escapes bundle: {file}"
            ));
        }
        artifacts.push(VerifiedArtifact {
            role,
            path,
            file,
            sha256: required_sha(row.get("sha256"), "artifact sha256")?,
            artifact_sha256: required_sha(row.get("artifact_sha256"), "artifact self sha256")?,
            source_gate_report_sha256: required_sha(
                row.get("source_gate_report_sha256"),
                "artifact source gate report sha256",
            )?,
        });
    }
    artifacts.sort_by(|left, right| left.role.cmp(&right.role));

    let roster = artifacts
        .iter()
        .map(|artifact| {
            json!({
                "role": artifact.role,
                "file": artifact.file,
                "sha256": artifact.sha256,
                "artifact_sha256": artifact.artifact_sha256,
                "source_gate_report_sha256": artifact.source_gate_report_sha256,
            })
        })
        .collect::<Vec<_>>();
    let evidence = json!({
        "admission": "passed",
        "bundle_id": bundle_id,
        "product_modset_sha256": product_modset_sha256,
        "artifact_count": artifact_count,
        "manifest": {
            "path": manifest.display().to_string(),
            "sha256": Sha256::hex(&manifest_bytes),
        },
        "artifacts": roster,
        "bindings": {
            "executable": file_evidence(&exe)?,
            "mod_state": file_evidence(&mod_state)?,
            "dlc_state": file_evidence(&dlc_state)?,
            "load_order": file_evidence(&load_order)?,
            "source_inventory": file_evidence(&source_inventory)?,
            "live_resolution_report": file_evidence(&live_resolution_report)?,
        },
        "process_identity": process_identity,
        "verifier": {
            "path": verifier.display().to_string(),
            "sha256": sha256_file(&verifier)?,
        },
        "verification_phase": "pre_injection_deferred_allowed",
        "token_join_status": "deferred",
        "token_gate_required": true,
        "deferred_token_gate_explicitly_allowed": true,
        "final_token_gate_required_before_validated_publication": true,
    });
    Ok(VerifiedStaticBundle {
        bundle_id,
        product_modset_sha256,
        artifact_count,
        artifacts,
        evidence,
    })
}

type CanonicalBindings = (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf);

fn canonical_bindings(
    exe: &Path,
    mod_state: &Path,
    dlc_state: &Path,
    load_order: &Path,
    source_inventory: &Path,
    live_resolution_report: &Path,
) -> Result<CanonicalBindings, String> {
    Ok((
        canonical_file(exe, "static bundle executable binding")?,
        canonical_file(mod_state, "static bundle mod-state binding")?,
        canonical_file(dlc_state, "static bundle DLC-state binding")?,
        canonical_file(load_order, "static bundle load-order binding")?,
        canonical_file(source_inventory, "static bundle source-inventory binding")?,
        canonical_file(
            live_resolution_report,
            "static bundle live-resolution-report binding",
        )?,
    ))
}

fn verify_live_resolution_process_identity(
    report: &Path,
    expected_process_id: u32,
    expected_creation_filetime_utc: u64,
    allow_prepared_process_identity_rebind: bool,
) -> Result<Value, String> {
    if expected_process_id == 0 || expected_creation_filetime_utc == 0 {
        return Err("static bundle expected process identity is incomplete".to_string());
    }
    let bytes = read_bounded(report, "static bundle live-resolution-report binding")?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!("static bundle live-resolution report is not strict JSON: {error}")
    })?;
    let process = document
        .get("process")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "static bundle live-resolution report has no process identity".to_string()
        })?;
    let observed_process_id = process
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or_else(|| "static bundle live-resolution report process id is invalid".to_string())?;
    let observed_creation = process
        .get("creation_filetime_utc")
        .and_then(Value::as_u64)
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            "static bundle live-resolution report process creation identity is invalid".to_string()
        })?;
    let exact_target = observed_process_id == expected_process_id
        && observed_creation == expected_creation_filetime_utc;
    if !exact_target && !allow_prepared_process_identity_rebind {
        return Err(format!(
            "static bundle live-resolution process identity ({observed_process_id}, {observed_creation}) does not match capture target ({expected_process_id}, {expected_creation_filetime_utc})"
        ));
    }
    Ok(json!({
        "mode": if exact_target { "exact_capture_target" } else { "prepared_corpus_rebound_to_suspended_target" },
        "prepared_process_id": observed_process_id,
        "prepared_process_creation_filetime_utc": observed_creation,
        "capture_process_id": expected_process_id,
        "capture_process_creation_filetime_utc": expected_creation_filetime_utc,
        "process_identity_rebound": !exact_target,
    }))
}

fn require_finalized_result(output: ProcessResult, action: &str) -> Result<Value, String> {
    if !output.success {
        return Err(format!(
            "static bundle {action} failed with code {:?}: {}",
            output.code,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let result: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("static bundle {action} returned invalid JSON: {error}"))?;
    if result.get("passed").and_then(Value::as_bool) != Some(true)
        || result.get("token_join_status").and_then(Value::as_str) != Some("passed")
        || result.get("verification_phase").and_then(Value::as_str)
            != Some("post_capture_finalized")
    {
        return Err(format!(
            "static bundle {action} did not return the exact finalized passed phase"
        ));
    }
    Ok(result)
}

fn verified_bundle_from_manifest(
    manifest: &Path,
    bundle_dir: &Path,
    verifier: &Path,
    bindings: &CanonicalBindings,
    result: &Value,
    verification_phase: &str,
    token_join_status: &str,
) -> Result<VerifiedStaticBundle, String> {
    let bundle_id = required_sha(result.get("bundle_id"), "verifier bundle_id")?;
    let product_modset_sha256 = required_sha(
        result.get("product_modset_sha256"),
        "verifier product_modset_sha256",
    )?;
    let artifact_count = result
        .get("artifact_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "static bundle verifier omitted artifact_count".to_string())?;
    let manifest_bytes = read_bounded(manifest, "static bundle manifest")?;
    let manifest_json: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("read verified static bundle evidence failed: {error}"))?;
    if manifest_json.get("bundle_id").and_then(Value::as_str) != Some(bundle_id.as_str()) {
        return Err("verifier result bundle_id differs from verified manifest".to_string());
    }
    let rows = manifest_json
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "verified static bundle manifest omitted artifact roster".to_string())?;
    if u64::try_from(rows.len()).ok() != Some(artifact_count) {
        return Err("verifier artifact_count differs from manifest roster".to_string());
    }
    let mut artifacts = Vec::with_capacity(rows.len());
    for row in rows {
        let role = required_string(row.get("role"), "artifact role")?;
        let file = required_string(row.get("file"), "artifact file")?;
        let path = canonical_file(&bundle_dir.join(&file), "bundled static artifact")?;
        if !path.starts_with(bundle_dir) {
            return Err(format!(
                "bundled static artifact path escapes bundle: {file}"
            ));
        }
        artifacts.push(VerifiedArtifact {
            role,
            path,
            file,
            sha256: required_sha(row.get("sha256"), "artifact sha256")?,
            artifact_sha256: required_sha(row.get("artifact_sha256"), "artifact self sha256")?,
            source_gate_report_sha256: required_sha(
                row.get("source_gate_report_sha256"),
                "artifact source gate report sha256",
            )?,
        });
    }
    artifacts.sort_by(|left, right| left.role.cmp(&right.role));
    let roster = artifacts
        .iter()
        .map(|artifact| {
            json!({
                "role": artifact.role,
                "file": artifact.file,
                "sha256": artifact.sha256,
                "artifact_sha256": artifact.artifact_sha256,
                "source_gate_report_sha256": artifact.source_gate_report_sha256,
            })
        })
        .collect::<Vec<_>>();
    let evidence = json!({
        "admission": "passed",
        "bundle_id": bundle_id,
        "product_modset_sha256": product_modset_sha256,
        "artifact_count": artifact_count,
        "manifest": {
            "path": manifest.display().to_string(),
            "sha256": Sha256::hex(&manifest_bytes),
        },
        "artifacts": roster,
        "bindings": {
            "executable": file_evidence(&bindings.0)?,
            "mod_state": file_evidence(&bindings.1)?,
            "dlc_state": file_evidence(&bindings.2)?,
            "load_order": file_evidence(&bindings.3)?,
            "source_inventory": file_evidence(&bindings.4)?,
            "live_resolution_report": file_evidence(&bindings.5)?,
        },
        "verifier": {
            "path": verifier.display().to_string(),
            "sha256": sha256_file(verifier)?,
        },
        "verification_phase": verification_phase,
        "token_join_status": token_join_status,
        "token_gate_required": true,
        "final_token_gate_satisfied": true,
    });
    Ok(VerifiedStaticBundle {
        bundle_id,
        product_modset_sha256,
        artifact_count,
        artifacts,
        evidence,
    })
}

fn required_nonempty(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_sha(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{label} is not canonical lowercase SHA-256"))
    }
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = fs::canonicalize(path).map_err(|error| format!("{label} not found: {error}"))?;
    if !path.is_file() {
        return Err(format!("{label} is not a file: {}", path.display()));
    }
    Ok(path)
}

fn required_string(value: Option<&Value>, label: &str) -> Result<String, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("verified static bundle omitted {label}"))
}

fn required_sha(value: Option<&Value>, label: &str) -> Result<String, String> {
    let value = required_string(value, label)?;
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value)
    } else {
        Err(format!("verified static bundle has invalid {label}"))
    }
}

fn read_bounded(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|error| format!("stat {label} failed: {error}"))?;
    if meta.len() > 16 * 1024 * 1024 {
        return Err(format!("{label} exceeds 16 MiB bound"));
    }
    fs::read(path).map_err(|error| format!("read {label} failed: {error}"))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("open {} for hash failed: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|error| format!("read {} for hash failed: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    Ok(snapshot::sha256::to_hex(&hasher.finalize()))
}

fn file_evidence(path: &Path) -> Result<Value, String> {
    Ok(json!({
        "path": path.display().to_string(),
        "bytes": fs::metadata(path)
            .map_err(|error| format!("stat binding {} failed: {error}", path.display()))?
            .len(),
        "sha256": sha256_file(path)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MockProcess {
        result: Mutex<Option<Result<ProcessResult, String>>>,
    }

    impl VerifierProcess for MockProcess {
        fn run(&self, _invocation: &ProcessInvocation) -> Result<ProcessResult, String> {
            self.result.lock().unwrap().take().unwrap()
        }
    }

    struct CapturingProcess {
        result: Mutex<Option<Result<ProcessResult, String>>>,
        invocation: Mutex<Option<ProcessInvocation>>,
    }

    impl VerifierProcess for CapturingProcess {
        fn run(&self, invocation: &ProcessInvocation) -> Result<ProcessResult, String> {
            *self.invocation.lock().unwrap() = Some(invocation.clone());
            self.result.lock().unwrap().take().unwrap()
        }
    }

    #[test]
    fn command_runner_rejects_zero_timeout_before_spawn() {
        let error = CommandVerifierProcess
            .run(&ProcessInvocation {
                program: "definitely-not-a-real-program".into(),
                args: Vec::new(),
                timeout: Duration::ZERO,
            })
            .unwrap_err();
        assert!(error.contains("timeout must be nonzero"));
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "recorder_static_bundle_{label}_{}_{}",
                std::process::id(),
                nonce
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_request(root: &Path) -> StaticBundleRequest {
        let roles = compact_contract::REQUIRED_STATIC_ARTIFACTS;
        let artifacts = roles
            .iter()
            .enumerate()
            .map(|(index, role)| {
                let file = format!("artifact_{index}.json");
                fs::write(root.join(&file), format!("{{\"role\":\"{role}\"}}")).unwrap();
                json!({
                    "role": role,
                    "file": file,
                    "sha256": format!("{:064x}", index + 1),
                    "artifact_sha256": format!("{:064x}", index + 11),
                    "source_gate_report_sha256": format!("{:064x}", index + 21),
                })
            })
            .collect::<Vec<_>>();
        fs::write(root.join("verifier.py"), b"# mock verifier boundary\n").unwrap();
        fs::write(root.join("game.exe"), b"bound executable").unwrap();
        fs::write(root.join("mod_state.json"), b"{\"mods\":[]}").unwrap();
        fs::write(root.join("dlc_state.json"), b"{\"dlc\":[]}").unwrap();
        fs::write(root.join("load_order.json"), b"{\"entries\":[]}").unwrap();
        fs::write(
            root.join("source_inventory.json"),
            b"{\"schema\":\"extractor.live_binding_source_inventory.v3\"}",
        )
        .unwrap();
        fs::write(
            root.join("live_resolution_report.json"),
            b"{\"passed\":true,\"process\":{\"pid\":4242,\"creation_filetime_utc\":9001}}",
        )
        .unwrap();
        fs::write(
            root.join("static_bundle_manifest.json"),
            serde_json::to_vec(&json!({
                "bundle_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "artifacts": artifacts,
            }))
            .unwrap(),
        )
        .unwrap();
        StaticBundleRequest {
            python: "python".into(),
            verifier: root.join("verifier.py"),
            manifest: root.join("static_bundle_manifest.json"),
            exe: root.join("game.exe"),
            mod_state: root.join("mod_state.json"),
            dlc_state: root.join("dlc_state.json"),
            load_order: root.join("load_order.json"),
            source_inventory: root.join("source_inventory.json"),
            live_resolution_report: root.join("live_resolution_report.json"),
            expected_process_id: 4242,
            expected_process_creation_filetime_utc: 9001,
            allow_prepared_process_identity_rebind: false,
            timeout: Duration::from_secs(5),
        }
    }

    fn passing_result() -> ProcessResult {
        ProcessResult {
            success: true,
            code: Some(0),
            stdout: serde_json::to_vec(&json!({
                "passed": true,
                "bundle_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "product_modset_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "artifact_count": 8,
                "token_join_status": "deferred",
                "verification_phase": "pre_injection_deferred_allowed",
            }))
            .unwrap(),
            stderr: Vec::new(),
        }
    }

    fn finalized_result() -> ProcessResult {
        ProcessResult {
            success: true,
            code: Some(0),
            stdout: serde_json::to_vec(&json!({
                "passed": true,
                "bundle_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "product_modset_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "artifact_count": 8,
                "token_join_status": "passed",
                "verification_phase": "post_capture_finalized",
            }))
            .unwrap(),
            stderr: Vec::new(),
        }
    }

    fn finalized_request(request: &StaticBundleRequest) -> FinalizedStaticBundleRequest {
        FinalizedStaticBundleRequest {
            python: request.python.clone(),
            verifier: request.verifier.clone(),
            manifest: request.manifest.clone(),
            exe: request.exe.clone(),
            mod_state: request.mod_state.clone(),
            dlc_state: request.dlc_state.clone(),
            load_order: request.load_order.clone(),
            source_inventory: request.source_inventory.clone(),
            live_resolution_report: request.live_resolution_report.clone(),
            expected_process_id: request.expected_process_id,
            expected_process_creation_filetime_utc: request.expected_process_creation_filetime_utc,
            allow_prepared_process_identity_rebind: request
                .allow_prepared_process_identity_rebind,
            expected_capture_id: "capture-1".to_string(),
            expected_session_id: "00000000-0000-4000-8000-000000000001".to_string(),
            expected_stream_sha256: "b".repeat(64),
            timeout: request.timeout,
        }
    }

    #[test]
    fn mocked_canonical_verifier_admits_and_records_exact_evidence() {
        let root = TempDir::new("admit");
        let request = fixture_request(&root.0);
        let process = CapturingProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
            invocation: Mutex::new(None),
        };
        let verified = verify_with_process(&request, &process).unwrap();
        assert_eq!(verified.artifact_count, 8);
        assert_eq!(verified.artifacts.len(), 8);
        assert_eq!(verified.evidence["admission"], "passed");
        assert_eq!(
            verified.evidence["verification_phase"],
            "pre_injection_deferred_allowed"
        );
        assert_eq!(verified.evidence["token_join_status"], "deferred");
        assert_eq!(
            verified.evidence["process_identity"]["capture_process_id"],
            4242
        );
        assert_eq!(
            verified.evidence["process_identity"]["capture_process_creation_filetime_utc"],
            9001
        );
        assert_eq!(
            verified.evidence["process_identity"]["mode"],
            "exact_capture_target"
        );
        assert_eq!(
            verified.evidence["deferred_token_gate_explicitly_allowed"],
            true
        );
        assert_eq!(
            verified.evidence["final_token_gate_required_before_validated_publication"],
            true
        );
        assert_eq!(verified.evidence["artifacts"].as_array().unwrap().len(), 8);
        assert_eq!(verified.evidence["bindings"]["executable"]["bytes"], 16);
        assert_eq!(
            verified.evidence["bindings"]["source_inventory"]["bytes"],
            55
        );
        assert_eq!(
            verified.evidence["bindings"]["live_resolution_report"]["bytes"],
            67
        );
        let invocation = process.invocation.lock().unwrap().clone().unwrap();
        let args = invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--source-inventory"
                && Path::new(&pair[1]) == fs::canonicalize(&request.source_inventory).unwrap()
        }));
        assert!(args.iter().any(|arg| arg == "--allow-deferred-token-gate"));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--live-resolution-report"
                && Path::new(&pair[1]) == fs::canonicalize(&request.live_resolution_report).unwrap()
        }));
    }

    #[test]
    fn pre_injection_gate_rejects_cross_process_resolution_report() {
        let root = TempDir::new("cross_process");
        let mut request = fixture_request(&root.0);
        request.expected_process_id += 1;
        let process = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let error = verify_with_process(&request, &process).unwrap_err();
        assert!(error.contains("does not match capture target"), "{error}");
    }

    #[test]
    fn suspended_launch_records_prepared_corpus_process_rebind() {
        let root = TempDir::new("prepared_process_rebind");
        let mut request = fixture_request(&root.0);
        request.expected_process_id += 1;
        request.expected_process_creation_filetime_utc += 1;
        request.allow_prepared_process_identity_rebind = true;
        let process = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let verified = verify_with_process(&request, &process).unwrap();
        assert_eq!(
            verified.evidence["process_identity"]["mode"],
            "prepared_corpus_rebound_to_suspended_target"
        );
        assert_eq!(
            verified.evidence["process_identity"]["prepared_process_id"],
            4242
        );
        assert_eq!(
            verified.evidence["process_identity"]["capture_process_id"],
            4243
        );
        assert_eq!(
            verified.evidence["process_identity"]["process_identity_rebound"],
            true
        );
    }

    #[test]
    fn pre_publication_gate_rejects_process_creation_rollover() {
        let root = TempDir::new("creation_rollover");
        let mut request = fixture_request(&root.0);
        let first = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let admitted = verify_with_process(&request, &first).unwrap();
        request.expected_process_creation_filetime_utc += 1;
        let second = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let error = verify_unchanged_with_process(&request, &admitted, &second).unwrap_err();
        assert!(error.contains("does not match capture target"), "{error}");
    }

    #[test]
    fn final_reverification_detects_binding_byte_drift() {
        let root = TempDir::new("drift");
        let request = fixture_request(&root.0);
        let first = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let admitted = verify_with_process(&request, &first).unwrap();
        fs::write(
            &request.live_resolution_report,
            b"{\"passed\":false,\"process\":{\"pid\":4242,\"creation_filetime_utc\":9001}}",
        )
        .unwrap();
        let second = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let error = verify_unchanged_with_process(&request, &admitted, &second).unwrap_err();
        assert!(error.contains("binding changed after admission"), "{error}");
    }

    #[test]
    fn verifier_result_must_match_manifest_identity_and_roster() {
        let root = TempDir::new("mismatch");
        let request = fixture_request(&root.0);
        let bad = ProcessResult {
            stdout: serde_json::to_vec(&json!({
                "passed": true,
                "bundle_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "product_modset_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "artifact_count": 6,
                "token_join_status": "deferred",
                "verification_phase": "pre_injection_deferred_allowed",
            }))
            .unwrap(),
            ..passing_result()
        };
        let process = MockProcess {
            result: Mutex::new(Some(Ok(bad))),
        };
        let error = verify_with_process(&request, &process).unwrap_err();
        assert!(error.contains("bundle_id differs"), "{error}");
    }

    #[test]
    fn pre_injection_gate_rejects_a_passed_or_wrong_phase_token_join() {
        let root = TempDir::new("wrong_phase");
        let request = fixture_request(&root.0);
        let mut wrong_status = passing_result();
        wrong_status.stdout = serde_json::to_vec(&json!({
            "passed": true,
            "bundle_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "product_modset_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "artifact_count": 8,
            "token_join_status": "passed",
            "verification_phase": "post_capture_finalized",
        }))
        .unwrap();
        let process = MockProcess {
            result: Mutex::new(Some(Ok(wrong_status))),
        };
        let error = verify_with_process(&request, &process).unwrap_err();
        assert!(
            error.contains("did not admit a deferred token gate"),
            "{error}"
        );
    }

    #[test]
    fn finalized_verification_binds_capture_provenance_and_omits_deferred_flag() {
        let root = TempDir::new("finalized");
        let admitted = fixture_request(&root.0);
        let request = finalized_request(&admitted);
        let process = CapturingProcess {
            result: Mutex::new(Some(Ok(finalized_result()))),
            invocation: Mutex::new(None),
        };
        let verified = verify_finalized_with_process(&request, &process).unwrap();
        assert_eq!(verified.evidence["token_join_status"], "passed");
        assert_eq!(
            verified.evidence["capture_provenance"]["capture_id"],
            "capture-1"
        );
        assert_eq!(
            verified.evidence["capture_provenance"]["stream_sha256"],
            "b".repeat(64)
        );
        let invocation = process.invocation.lock().unwrap().clone().unwrap();
        let args = invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!args.iter().any(|arg| arg == "--allow-deferred-token-gate"));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0] == "--expected-capture-id" && pair[1] == "capture-1" }));
        assert!(args
            .windows(2)
            .any(|pair| { pair[0] == "--expected-stream-sha256" && pair[1] == "b".repeat(64) }));
    }

    #[test]
    fn finalized_verification_rejects_deferred_result() {
        let root = TempDir::new("finalized_wrong_phase");
        let admitted = fixture_request(&root.0);
        let request = finalized_request(&admitted);
        let process = MockProcess {
            result: Mutex::new(Some(Ok(passing_result()))),
        };
        let error = verify_finalized_with_process(&request, &process).unwrap_err();
        assert!(error.contains("exact finalized passed phase"), "{error}");
    }

    #[test]
    fn real_canonical_verifier_process_rejects_invalid_bundle() {
        let root = TempDir::new("real_negative");
        let mut request = fixture_request(&root.0);
        request.verifier =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game_defs/static_bundle.py");
        // This is the one test that spawns the real Python verifier; interpreter
        // startup on a loaded machine can exceed the fixture's 5 s. The assertion
        // is about rejection, not latency, so give the subprocess ample time.
        request.timeout = Duration::from_secs(120);
        fs::write(&request.manifest, b"{}\n").unwrap();
        let error = verify(&request).unwrap_err();
        assert!(
            error.contains("static bundle verifier failed"),
            "canonical verifier was not actually invoked: {error}"
        );
    }
}
