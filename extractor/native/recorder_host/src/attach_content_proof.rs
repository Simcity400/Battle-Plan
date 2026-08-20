use crate::modset::ModsetReport;
use crate::portable_content_manifest::{PortableLaunchBinding, RuntimeContentRoots};
use crate::production_content_vfs::{
    effective_vfs_from_launch, ordered_content_layers_from_resolved_launch, BoundContentLayer,
};
use portable_content_manifest::effective_vfs::EffectiveVfs;
use portable_content_manifest::ContentLayerKind;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONTENT_PROOF_KIND_FIELD: &str =
    portable_content_manifest::attach_content_epoch::CONTENT_PROOF_KIND_FIELD;

pub const ATTACH_TIME_CONTENT_PROOF_FIELD: &str =
    portable_content_manifest::attach_content_epoch::ATTACH_TIME_CONTENT_PROOF_FIELD;

pub const KIND_LAUNCH_TIME_STATIC_BUNDLE_V1: &str =
    portable_content_manifest::attach_content_epoch::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1;

pub const KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1: &str =
    portable_content_manifest::attach_content_epoch::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1;

pub const KIND_NONE: &str = portable_content_manifest::attach_content_epoch::KIND_NONE;

pub const SCHEMA: &str = portable_content_manifest::attach_content_epoch::ATTACH_PROOF_SCHEMA;

pub const PHASE_CONTENT_FINGERPRINT: &str = "content_fingerprint";

pub const D81_PROVINCE_CATALOG_PATH: &str = "map/definition.csv";

pub const D81_PROVINCE_CATALOG_PURPOSE: &str = "D81 province catalog";

fn lower_this_thread_to_background_priority() {
    #[cfg(windows)]
    {
        extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        // SAFETY: both take a pseudo-handle to the calling thread and change
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}

pub const DEFAULT_MAX_WINDOW_ATTEMPTS: u32 = 8;

const MAX_NAMED_REFUSED_PATHS: usize = 8;

const MAX_CLAIMS_PER_ARTIFACT: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct ArtifactRequest {
    pub role: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct RequiredContentRequest {
    pub purpose: String,
    pub logical_path: String,
}

#[derive(Clone, Debug)]
pub struct AttachContentProofRequest {
    pub artifacts: Vec<ArtifactRequest>,
    pub required_content: Vec<RequiredContentRequest>,
    pub worker_threads: usize,
    pub max_window_attempts: u32,
}

impl AttachContentProofRequest {
    pub fn default_worker_threads() -> usize {
        std::thread::available_parallelism()
            .map(|value| (value.get() / 2).clamp(1, 4))
            .unwrap_or(1)
    }
}

#[derive(Debug, Default)]
pub struct AttachFingerprintProgress {
    pub attempt: AtomicU64,
    pub files_total: AtomicU64,
    pub files_done: AtomicU64,
    pub bytes_hashed: AtomicU64,
    pub finished: AtomicBool,
}

impl AttachFingerprintProgress {
    pub fn work_bytes(&self) -> u64 {
        self.bytes_hashed.load(Ordering::Relaxed)
    }
}

pub struct AttachContentProofHandle {
    progress: Arc<AttachFingerprintProgress>,
    thread: JoinHandle<Result<AttachTimeContentProof, String>>,
}

impl AttachContentProofHandle {
    pub fn progress(&self) -> Arc<AttachFingerprintProgress> {
        Arc::clone(&self.progress)
    }

    pub fn join(self) -> Result<AttachTimeContentProof, String> {
        match self.thread.join() {
            Ok(result) => result,
            Err(_) => Err("attach-time content fingerprint thread panicked".to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AttachTimeContentProof {
    pub game_process_start_unix: Option<i64>,
    pub observed_at_unix: i64,
    pub completed_at_unix: i64,
    pub elapsed_ms: u64,
    pub hashed_bytes: u64,
    pub hashed_files: u64,
    pub window_attempts: u32,
    pub window_settled: bool,
    pub executable_sha256: String,
    pub executable_size_bytes: u64,
    pub dlc_load_sha256: String,
    pub layer_stack: Vec<ProofLayer>,
    pub layer_stack_sha256: String,
    pub artifact_joins: Vec<ArtifactJoin>,
    pub refused_artifacts: Vec<RefusedArtifact>,
    pub required_content: Vec<RequiredContentJoin>,
    pub refused_required_content: Vec<RefusedRequiredContent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofLayer {
    pub ordinal: u32,
    pub kind: &'static str,
    pub layer_id: String,
    pub descriptor_sha256: Option<String>,
    pub descriptor_file_sha256: Option<String>,
    pub source_container_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ArtifactJoin {
    pub role: String,
    pub artifact_sha256: String,
    pub claimed_paths: u64,
    pub joined_bytes: u64,
    pub proven_absent_paths: u64,
    pub localisation_source_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RefusedArtifact {
    pub role: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct RequiredContentJoin {
    pub purpose: String,
    pub logical_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub winning_layer_id: String,
}

#[derive(Clone, Debug)]
pub struct RefusedRequiredContent {
    pub purpose: String,
    pub logical_path: String,
    pub reason: String,
}

impl AttachTimeContentProof {
    pub fn as_json(&self) -> Value {
        json!({
            "schema": SCHEMA,
            "kind": KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
            "game_process_start_unix": self.game_process_start_unix,
            "observed_at_unix": self.observed_at_unix,
            "completed_at_unix": self.completed_at_unix,
            "elapsed_ms": self.elapsed_ms,
            "hashed_bytes": self.hashed_bytes,
            "hashed_files": self.hashed_files,
            "window_attempts": self.window_attempts,
            "window_settled": self.window_settled,
            "executable_sha256": self.executable_sha256,
            "executable_size_bytes": self.executable_size_bytes,
            "dlc_load_sha256": self.dlc_load_sha256,
            "layer_stack": self
                .layer_stack
                .iter()
                .map(|layer| json!({
                    "ordinal": layer.ordinal,
                    "kind": layer.kind,
                    "layer_id": layer.layer_id,
                    "descriptor_sha256": layer.descriptor_sha256,
                    "descriptor_file_sha256": layer.descriptor_file_sha256,
                    "source_container_sha256": layer.source_container_sha256,
                }))
                .collect::<Vec<_>>(),
            "layer_stack_sha256": self.layer_stack_sha256,
            "artifact_joins": self
                .artifact_joins
                .iter()
                .map(|join| json!({
                    "role": join.role,
                    "artifact_sha256": join.artifact_sha256,
                    "claimed_paths": join.claimed_paths,
                    "joined_bytes": join.joined_bytes,
                    "proven_absent_paths": join.proven_absent_paths,
                    "localisation_source_sha256": join.localisation_source_sha256,
                }))
                .collect::<Vec<_>>(),
            "refused_artifacts": self
                .refused_artifacts
                .iter()
                .map(|refused| json!({ "role": refused.role, "reason": refused.reason }))
                .collect::<Vec<_>>(),
            "required_content": self
                .required_content
                .iter()
                .map(|row| json!({
                    "purpose": row.purpose,
                    "logical_path": row.logical_path,
                    "sha256": row.sha256,
                    "bytes": row.bytes,
                    "winning_layer_id": row.winning_layer_id,
                }))
                .collect::<Vec<_>>(),
            "refused_required_content": self
                .refused_required_content
                .iter()
                .map(|row| json!({
                    "purpose": row.purpose,
                    "logical_path": row.logical_path,
                    "reason": row.reason,
                }))
                .collect::<Vec<_>>(),
        })
    }
}

pub fn proof_layers_from_bound(layers: &[BoundContentLayer]) -> Vec<ProofLayer> {
    layers
        .iter()
        .map(|layer| ProofLayer {
            ordinal: layer.recorded.ordinal,
            kind: layer_kind_name(layer.recorded.layer_kind),
            layer_id: layer.recorded.layer_id.clone(),
            descriptor_sha256: layer.recorded.descriptor_sha256.clone(),
            descriptor_file_sha256: layer.descriptor_file_sha256.clone(),
            source_container_sha256: layer.recorded.source_container_sha256.clone(),
        })
        .collect()
}

pub fn require_certified_bracket(
    roots: &RuntimeContentRoots,
    exe_path: &Path,
    layers: &[BoundContentLayer],
    proof: &AttachTimeContentProof,
    phase: &str,
) -> Result<(), String> {
    let bracket = observe_world_bracket(roots, exe_path, layers)?;
    if bracket.executable_sha256 != proof.executable_sha256
        || bracket.executable_size_bytes != proof.executable_size_bytes
    {
        return Err(format!(
            "the executable {phase} is {} ({} bytes), not the certified {} ({} bytes)",
            bracket.executable_sha256,
            bracket.executable_size_bytes,
            proof.executable_sha256,
            proof.executable_size_bytes
        ));
    }
    if bracket.dlc_load_sha256 != proof.dlc_load_sha256 {
        return Err(format!(
            "dlc_load.json {phase} is {}, not the certified {}",
            bracket.dlc_load_sha256, proof.dlc_load_sha256
        ));
    }
    require_bracket_descriptors_match_certified(&bracket, proof, phase)?;
    let observed = layer_stack_sha256(&proof_layers_from_bound(layers))?;
    if observed != proof.layer_stack_sha256 {
        return Err(format!(
            "the layer stack {phase} is {observed}, not the certified {}",
            proof.layer_stack_sha256
        ));
    }
    Ok(())
}

fn published_layers_match_bracket(published: &[ProofLayer], bracket: &WorldBracket) -> bool {
    let rows = published
        .iter()
        .filter_map(|layer| {
            layer
                .descriptor_file_sha256
                .as_ref()
                .map(|sha| (layer.layer_id.clone(), sha.clone()))
        })
        .collect::<Vec<_>>();
    rows == bracket.layer_descriptors
}

fn require_bracket_descriptors_match_certified(
    bracket: &WorldBracket,
    proof: &AttachTimeContentProof,
    phase: &str,
) -> Result<(), String> {
    let certified = proof
        .layer_stack
        .iter()
        .filter_map(|layer| {
            layer
                .descriptor_file_sha256
                .as_ref()
                .map(|sha| (layer.layer_id.clone(), sha.clone()))
        })
        .collect::<Vec<_>>();
    if bracket.layer_descriptors != certified {
        return Err(format!(
            "the layer descriptors {phase} are {:?}, not the certified {certified:?}",
            bracket.layer_descriptors
        ));
    }
    Ok(())
}

pub fn layer_stack_sha256(layers: &[ProofLayer]) -> Result<String, String> {
    let rows = layers
        .iter()
        .map(|layer| {
            json!({
                "ordinal": layer.ordinal,
                "kind": layer.kind,
                "layer_id": layer.layer_id,
                "descriptor_sha256": layer.descriptor_sha256,
                "descriptor_file_sha256": layer.descriptor_file_sha256,
                "source_container_sha256": layer.source_container_sha256,
            })
        })
        .collect::<Vec<_>>();
    portable_content_manifest::attach_content_epoch::layer_stack_digest(&rows)
}

pub fn spawn(
    request: AttachContentProofRequest,
    report: ModsetReport,
) -> Result<AttachContentProofHandle, String> {
    if request.artifacts.is_empty() {
        return Err("attach-time content proof was asked to certify no artifacts".into());
    }
    let progress = Arc::new(AttachFingerprintProgress::default());
    let thread_progress = Arc::clone(&progress);
    let thread = std::thread::Builder::new()
        .name("recorder-attach-content-proof".to_string())
        .spawn(move || certify(&request, &report, &thread_progress))
        .map_err(|error| format!("spawn attach content proof thread failed: {error}"))?;
    Ok(AttachContentProofHandle { progress, thread })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorldBracket {
    executable_sha256: String,
    executable_size_bytes: u64,
    dlc_load_sha256: String,
    layer_descriptors: Vec<(String, String)>,
}

fn observe_world_bracket(
    roots: &RuntimeContentRoots,
    exe_path: &Path,
    layers: &[BoundContentLayer],
) -> Result<WorldBracket, String> {
    let (executable_sha256, executable_size_bytes) = hash_file(exe_path)
        .map_err(|error| format!("attach window executable bracket failed: {error}"))?;
    let dlc_load_path = roots.user_dir.join("dlc_load.json");
    let (dlc_load_sha256, _) = hash_file(&dlc_load_path)
        .map_err(|error| format!("attach window dlc_load.json bracket failed: {error}"))?;
    let mut layer_descriptors = Vec::new();
    for layer in layers {
        let Some(path) = layer.descriptor_path.as_ref() else {
            continue;
        };
        let (sha256, _) = hash_file(path).map_err(|error| {
            format!(
                "attach window descriptor bracket failed for {}: {error}",
                layer.recorded.layer_id
            )
        })?;
        layer_descriptors.push((layer.recorded.layer_id.clone(), sha256));
    }
    Ok(WorldBracket {
        executable_sha256,
        executable_size_bytes,
        dlc_load_sha256,
        layer_descriptors,
    })
}

fn hash_file(path: &Path) -> Result<(String, u64), String> {
    use std::io::Read as _;
    let mut file =
        fs::File::open(path).map_err(|error| format!("open {} failed: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {} failed: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

fn certify(
    request: &AttachContentProofRequest,
    report: &ModsetReport,
    progress: &Arc<AttachFingerprintProgress>,
) -> Result<AttachTimeContentProof, String> {
    let observed_at_unix = now_unix();
    let started = std::time::Instant::now();
    let roots = RuntimeContentRoots::from_modset(report)?;
    let exe_path = report
        .exe_path
        .clone()
        .ok_or("attach-time content proof requires a resolved executable path")?;
    let max_attempts = request.max_window_attempts.max(1);

    let mut scans = Vec::new();
    let mut refused_artifacts = Vec::new();
    for artifact in &request.artifacts {
        match read_and_scan_artifact(artifact) {
            Ok(scan) => scans.push(scan),
            Err(reason) => refused_artifacts.push(RefusedArtifact {
                role: artifact.role.clone(),
                reason,
            }),
        }
    }
    reconcile_localisation_sources(&scans, &mut refused_artifacts);
    scans.retain(|scan| {
        !refused_artifacts
            .iter()
            .any(|refused| refused.role == scan.role)
    });

    let files_total: u64 = scans.iter().map(|scan| scan.claims.len() as u64).sum();
    progress.files_total.store(files_total, Ordering::Relaxed);

    let mut last: Option<WalkOutcome> = None;
    let mut window_settled = false;
    let mut attempts = 0u32;
    while attempts < max_attempts {
        attempts += 1;
        progress
            .attempt
            .store(u64::from(attempts), Ordering::Relaxed);
        progress.files_done.store(0, Ordering::Relaxed);
        progress.bytes_hashed.store(0, Ordering::Relaxed);

        let launch = PortableLaunchBinding::from_modset(report)?;
        let layers = ordered_content_layers_from_resolved_launch(&roots, &launch, report)?;
        let open = observe_world_bracket(&roots, &exe_path, &layers)?;

        let proof_layers = proof_layers_from_bound(&layers);
        let stack_sha256 = layer_stack_sha256(&proof_layers)?;
        let recorded = layers
            .iter()
            .map(|layer| layer.recorded.clone())
            .collect::<Vec<_>>();
        let vfs = effective_vfs_from_launch(&roots, &recorded, &stack_sha256)?;

        let mut outcome = walk_corpus(&vfs, &scans, request, progress);
        outcome.required_content = resolve_required_content(&vfs, &request.required_content);
        let close = observe_world_bracket(&roots, &exe_path, &layers)?;
        let published_matches_bracket = published_layers_match_bracket(&proof_layers, &open);
        let bracket_held = open == close && published_matches_bracket;
        last = Some(WalkOutcome {
            bracket: open,
            proof_layers,
            stack_sha256,
            ..outcome
        });
        if bracket_held && !last.as_ref().expect("walk outcome").churned {
            window_settled = true;
            break;
        }
    }

    let outcome = last.ok_or("attach-time content proof produced no attempt")?;
    let mut refused = refused_artifacts;
    if window_settled {
        refused.extend(outcome.refused);
    } else {
        for scan in &scans {
            refused.push(RefusedArtifact {
                role: scan.role.clone(),
                reason: format!(
                    "the attach window never settled: the install changed during all {attempts} \
                     fingerprint attempts, so no observation inside it describes one install"
                ),
            });
        }
    }
    refused.sort_by(|left, right| left.role.cmp(&right.role));
    let mut joins = if window_settled {
        outcome.joins
    } else {
        Vec::new()
    };
    joins.sort_by(|left, right| left.role.cmp(&right.role));
    let (required_content, refused_required_content) = if window_settled {
        outcome.required_content
    } else {
        (
            Vec::new(),
            request
                .required_content
                .iter()
                .map(|row| RefusedRequiredContent {
                    purpose: row.purpose.clone(),
                    logical_path: row.logical_path.clone(),
                    reason: "the attach window never settled".to_string(),
                })
                .collect(),
        )
    };

    progress.finished.store(true, Ordering::Relaxed);
    Ok(AttachTimeContentProof {
        game_process_start_unix: report.process_start_unix,
        observed_at_unix,
        completed_at_unix: now_unix(),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        hashed_bytes: progress.bytes_hashed.load(Ordering::Relaxed),
        hashed_files: progress.files_done.load(Ordering::Relaxed),
        window_attempts: attempts,
        window_settled,
        executable_sha256: outcome.bracket.executable_sha256,
        executable_size_bytes: outcome.bracket.executable_size_bytes,
        dlc_load_sha256: outcome.bracket.dlc_load_sha256,
        layer_stack: outcome.proof_layers,
        layer_stack_sha256: outcome.stack_sha256,
        artifact_joins: joins,
        refused_artifacts: refused,
        required_content,
        refused_required_content,
    })
}

fn layer_kind_name(kind: ContentLayerKind) -> &'static str {
    match kind {
        ContentLayerKind::Base => "base",
        ContentLayerKind::IntegratedDlc => "integrated_dlc",
        ContentLayerKind::Dlc => "dlc",
        ContentLayerKind::Mod => "mod",
    }
}

pub fn layer_kind_from_wire(value: &str) -> Option<&'static str> {
    [
        layer_kind_name(ContentLayerKind::Base),
        layer_kind_name(ContentLayerKind::IntegratedDlc),
        layer_kind_name(ContentLayerKind::Dlc),
        layer_kind_name(ContentLayerKind::Mod),
    ]
    .into_iter()
    .find(|known| *known == value)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[derive(Default)]
struct WalkOutcome {
    joins: Vec<ArtifactJoin>,
    refused: Vec<RefusedArtifact>,
    churned: bool,
    bracket: WorldBracket,
    proof_layers: Vec<ProofLayer>,
    stack_sha256: String,
    required_content: (Vec<RequiredContentJoin>, Vec<RefusedRequiredContent>),
}

impl Default for WorldBracket {
    fn default() -> Self {
        Self {
            executable_sha256: String::new(),
            executable_size_bytes: 0,
            dlc_load_sha256: String::new(),
            layer_descriptors: Vec::new(),
        }
    }
}

fn walk_corpus(
    vfs: &EffectiveVfs,
    scans: &[ArtifactScan],
    request: &AttachContentProofRequest,
    progress: &Arc<AttachFingerprintProgress>,
) -> WalkOutcome {
    let mut outcome = WalkOutcome::default();
    for scan in scans {
        match join_artifact(vfs, scan, request.worker_threads.max(1), progress) {
            ArtifactVerdict::Joined(join) => outcome.joins.push(join),
            ArtifactVerdict::Refused(reason) => outcome.refused.push(RefusedArtifact {
                role: scan.role.clone(),
                reason,
            }),
            ArtifactVerdict::Churned => {
                outcome.churned = true;
                return outcome;
            }
        }
    }
    outcome
}

enum ArtifactVerdict {
    Joined(ArtifactJoin),
    Refused(String),
    Churned,
}

fn join_artifact(
    vfs: &EffectiveVfs,
    scan: &ArtifactScan,
    workers: usize,
    progress: &Arc<AttachFingerprintProgress>,
) -> ArtifactVerdict {
    let claims: Vec<(&String, &ContentClaim)> = scan.claims.iter().collect();
    let cursor = AtomicU64::new(0);
    let mismatches: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let joined_bytes = AtomicU64::new(0);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                lower_this_thread_to_background_priority();
                loop {
                    let index = cursor.fetch_add(1, Ordering::Relaxed) as usize;
                    let Some((path, claim)) = claims.get(index).copied() else {
                        return;
                    };
                    let verdict = check_claim(vfs, path.as_str(), claim);
                    progress.files_done.fetch_add(1, Ordering::Relaxed);
                    match verdict {
                        Ok(bytes) => {
                            joined_bytes.fetch_add(bytes, Ordering::Relaxed);
                            progress.bytes_hashed.fetch_add(bytes, Ordering::Relaxed);
                        }
                        Err(reason) => {
                            let mut guard = mismatches.lock().expect("attach proof mismatch lock");
                            guard.push((path.clone(), reason));
                        }
                    }
                }
            });
        }
    });

    let mismatches = mismatches.into_inner().expect("attach proof mismatch lock");
    if !mismatches.is_empty() {
        let mut stable = Vec::new();
        for (path, reason) in &mismatches {
            let claim = match scan.claims.get(path) {
                Some(claim) => claim,
                None => return ArtifactVerdict::Refused(format!("lost claim for {path}")),
            };
            match check_claim(vfs, path.as_str(), claim) {
                Ok(_) => return ArtifactVerdict::Churned,
                Err(second) if &second != reason => return ArtifactVerdict::Churned,
                Err(second) => stable.push(format!("{path}: {second}")),
            }
        }
        stable.sort();
        let shown = stable
            .iter()
            .take(MAX_NAMED_REFUSED_PATHS)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        return ArtifactVerdict::Refused(format!(
            "{} of {} claimed paths did not match the attach-time effective VFS: {shown}{}",
            stable.len(),
            claims.len(),
            if stable.len() > MAX_NAMED_REFUSED_PATHS {
                format!(" (+{} more)", stable.len() - MAX_NAMED_REFUSED_PATHS)
            } else {
                String::new()
            }
        ));
    }

    let mut present_but_declared_absent = Vec::new();
    for path in &scan.absent_claims {
        match vfs.resolve(path) {
            Ok(None) => {}
            Ok(Some(_)) => present_but_declared_absent.push(path.clone()),
            Err(error) => present_but_declared_absent.push(format!("{path}: {error}")),
        }
    }
    if !present_but_declared_absent.is_empty() {
        present_but_declared_absent.sort();
        return ArtifactVerdict::Refused(format!(
            "{} path(s) the artifact declared legitimately absent resolve through the attach-time \
             effective VFS: {}",
            present_but_declared_absent.len(),
            present_but_declared_absent
                .iter()
                .take(MAX_NAMED_REFUSED_PATHS)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    ArtifactVerdict::Joined(ArtifactJoin {
        role: scan.role.clone(),
        artifact_sha256: scan.artifact_file_sha256.clone(),
        claimed_paths: claims.len() as u64,
        joined_bytes: joined_bytes.load(Ordering::Relaxed),
        proven_absent_paths: scan.absent_claims.len() as u64,
        localisation_source_sha256: scan.localisation_source_sha256.clone(),
    })
}

fn check_claim(vfs: &EffectiveVfs, path: &str, claim: &ContentClaim) -> Result<u64, String> {
    let resolved = match vfs.resolve(path) {
        Ok(Some(resolved)) => resolved,
        Ok(None) => return Err("absent from the effective VFS".to_string()),
        Err(error) => return Err(format!("effective VFS refused the path: {error}")),
    };
    if resolved.content.payload_sha256 != claim.sha256 {
        return Err(format!(
            "sha256 {} != declared {}",
            resolved.content.payload_sha256, claim.sha256
        ));
    }
    if let Some(bytes) = claim.bytes {
        if resolved.content.payload_bytes != bytes {
            return Err(format!(
                "bytes {} != declared {bytes}",
                resolved.content.payload_bytes
            ));
        }
    }
    Ok(resolved.content.payload_bytes)
}

fn resolve_required_content(
    vfs: &EffectiveVfs,
    requests: &[RequiredContentRequest],
) -> (Vec<RequiredContentJoin>, Vec<RefusedRequiredContent>) {
    let mut joined = Vec::new();
    let mut refused = Vec::new();
    for request in requests {
        match vfs.resolve(&request.logical_path) {
            Ok(Some(file)) => joined.push(RequiredContentJoin {
                purpose: request.purpose.clone(),
                logical_path: file.content.normalized_game_relative_path.clone(),
                sha256: file.content.payload_sha256.clone(),
                bytes: file.content.payload_bytes,
                winning_layer_id: file.content.layer_id.clone(),
            }),
            Ok(None) => refused.push(RefusedRequiredContent {
                purpose: request.purpose.clone(),
                logical_path: request.logical_path.clone(),
                reason: "absent from the attach-time effective VFS".to_string(),
            }),
            Err(error) => refused.push(RefusedRequiredContent {
                purpose: request.purpose.clone(),
                logical_path: request.logical_path.clone(),
                reason: format!("effective VFS refused the path: {error}"),
            }),
        }
    }
    (joined, refused)
}

#[derive(Clone, Debug)]
struct ContentClaim {
    sha256: String,
    bytes: Option<u64>,
}

#[derive(Debug)]
struct ArtifactScan {
    role: String,
    artifact_file_sha256: String,
    claims: BTreeMap<String, ContentClaim>,
    absent_claims: Vec<String>,
    localisation_source_sha256: Option<String>,
    /// `{source_file, source_sha256}` rows: RESTATEMENTS of a claim the artifact
    /// makes elsewhere, never a claim of their own. See
    /// [`reconcile_restated_claims`].
    restated_claims: Vec<RestatedClaim>,
}

/// One `{source_file, source_sha256}` row, held until the whole document has
/// been walked.
#[derive(Debug)]
struct RestatedClaim {
    normalized_path: String,
    sha256: String,
    breadcrumb: String,
}

fn read_and_scan_artifact(request: &ArtifactRequest) -> Result<ArtifactScan, String> {
    let bytes =
        fs::read(&request.path).map_err(|error| format!("read static artifact failed: {error}"))?;
    let artifact_file_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("static artifact is not JSON: {error}"))?;
    scan_artifact_document(&request.role, &artifact_file_sha256, &document)
}

fn scan_artifact_document(
    role: &str,
    artifact_file_sha256: &str,
    document: &Value,
) -> Result<ArtifactScan, String> {
    let mut scan = ArtifactScan {
        role: role.to_string(),
        artifact_file_sha256: artifact_file_sha256.to_string(),
        claims: BTreeMap::new(),
        absent_claims: Vec::new(),
        localisation_source_sha256: None,
        restated_claims: Vec::new(),
    };
    let root = document
        .as_object()
        .ok_or_else(|| "static artifact root is not a JSON object".to_string())?;

    let mut walked = document.clone();
    let walked_root = walked
        .as_object_mut()
        .ok_or_else(|| "static artifact root is not a JSON object".to_string())?;
    let self_digest = walked_root
        .remove("artifact_sha256")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| "the artifact carries no artifact_sha256 self-digest".to_string())?;
    require_sha256(&self_digest, "artifact_sha256")?;
    if let Some(source) = walked_root.remove("localisation_source") {
        let object = source
            .as_object()
            .ok_or_else(|| "localisation_source is not an object".to_string())?;
        let sha256 = object
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| "localisation_source names no sha256".to_string())?;
        require_sha256(sha256, "localisation_source.sha256")?;
        // Removing the whole object exempts everything inside it, so what is
        // inside it is pinned: a CLOSED, NAMED set of build-provenance digests
        // and nothing else. A future `localisation_source` that grew a digest
        // outside that set is still refused (Codex review round 1, finding 3).
        //
        // The set is three, not one. `technology_sharing_static` records which
        // effective layers and which localisation source files its catalog was
        // built from, and digests each of those two canonical-JSON structures:
        //
        //   layer_binding_sha256                  the ordered mounted layers
        //   localisation_source_inventory_sha256  the localisation file inventory
        //
        // Neither is the digest of any single file at any VFS path, so neither
        // can be a content claim and neither can be resolved through the
        // effective VFS. They are pinned by name and required to be well-formed
        // digests; refusing them made the artifact uncertifiable while proving
        // nothing (`focus_defs_static`/`technology_sharing_static` were both
        // refused for this, lane MU's ledger item 9).
        for name in LOCALISATION_SOURCE_PROVENANCE_DIGESTS {
            let Some(value) = object.get(name) else {
                continue;
            };
            let text = value
                .as_str()
                .ok_or_else(|| format!("localisation_source.{name} is not a string"))?;
            require_sha256(text, &format!("localisation_source.{name}"))?;
        }
        let mut other = Vec::new();
        let mut discarded = ArtifactScan {
            role: String::new(),
            artifact_file_sha256: String::new(),
            claims: BTreeMap::new(),
            absent_claims: Vec::new(),
            localisation_source_sha256: None,
            restated_claims: Vec::new(),
        };
        collect_claims(
            &source,
            None,
            &mut vec!["localisation_source".to_string()],
            &mut discarded,
            &mut other,
        )?;
        other.retain(|site| {
            !LOCALISATION_SOURCE_PROVENANCE_DIGESTS
                .iter()
                .any(|name| *site == format!("localisation_source.{name}"))
        });
        other.extend(
            discarded
                .claims
                .into_keys()
                .map(|path| format!("localisation_source declares a content claim for {path}")),
        );
        // A RESTATEMENT inside the removed object would reach neither the sweep
        // (its digest is marked consumed) nor `reconcile_restated_claims` (it
        // lands in this discarded scan), so it would be the one shape that
        // could carry an unaccounted digest through the pin.
        other.extend(discarded.restated_claims.into_iter().map(|row| {
            format!(
                "localisation_source restates a content claim for {}",
                row.normalized_path
            )
        }));
        if !other.is_empty() {
            return Err(format!(
                "localisation_source carries {} digest site(s) beyond its named build-provenance \
                 digests, which the walk cannot account for: {}",
                other.len(),
                other.join("; ")
            ));
        }
        scan.localisation_source_sha256 = Some(sha256.to_string());
    }

    if let Some(rows) = root.get("unresolved_textures").and_then(Value::as_array) {
        if !rows.is_empty() {
            return Err(format!(
                "the artifact declares {} unresolved texture reference(s); a passing gate emits none",
                rows.len()
            ));
        }
    }
    for row in root
        .get("allowed_missing_textures")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let path = row
            .get("texturefile")
            .and_then(Value::as_str)
            .ok_or_else(|| "allowed_missing_textures row names no texturefile".to_string())?;
        scan.absent_claims.push(path.to_string());
    }
    scan.absent_claims.sort();
    scan.absent_claims.dedup();

    let mut unaccounted = Vec::new();
    collect_claims(&walked, None, &mut Vec::new(), &mut scan, &mut unaccounted)?;
    if !unaccounted.is_empty() {
        unaccounted.sort();
        unaccounted.dedup();
        return Err(format!(
            "{} digest site(s) in the artifact are not content claims, so the corpus this \
             artifact claims cannot be enumerated completely: {}",
            unaccounted.len(),
            unaccounted
                .iter()
                .take(MAX_NAMED_REFUSED_PATHS)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    reconcile_restated_claims(&mut scan)?;
    if scan.claims.is_empty() {
        return Err("the artifact makes no content claims at all".to_string());
    }
    if scan.claims.len() > MAX_CLAIMS_PER_ARTIFACT {
        return Err(format!(
            "the artifact claims {} paths, above the {MAX_CLAIMS_PER_ARTIFACT} ceiling",
            scan.claims.len()
        ));
    }
    Ok(scan)
}

/// The digests `localisation_source` may carry. Each states the provenance of
/// the localisation catalog the artifact was BUILT against — never the bytes of
/// a game file — so none of them is a content claim and none is resolvable
/// through the effective VFS. Closed by construction: any other digest inside
/// `localisation_source` still refuses the artifact by name.
const LOCALISATION_SOURCE_PROVENANCE_DIGESTS: [&str; 3] = [
    "sha256",
    "layer_binding_sha256",
    "localisation_source_inventory_sha256",
];

/// One recognized `(path key, digest key, byte key)` triple.
const CLAIM_SHAPES: [(&str, &str, Option<&str>); 3] = [
    ("relpath", "sha256", Some("bytes")),
    (
        "resolved_texturefile",
        "texture_sha256",
        Some("texture_bytes"),
    ),
    ("source", "source_sha256", None),
];

/// The RESTATEMENT shape: `{source_file, source_sha256}`.
///
/// `focus_defs_static`'s `meta.source_compatibility[]` rows spell a
/// game-relative content path and its digest with a different path key than the
/// `source_files[]` row that states the SAME path with the SAME digest. Before
/// this was recognized the digest reached the fail-closed sweep and refused the
/// whole artifact.
///
/// It is deliberately NOT a claim shape. A claim shape puts a NEW path into the
/// proved corpus, and this one would then admit any path the row happens to
/// carry — including a repo-relative build input, which
/// `normalize_game_relative_path` accepts and which a layer could then satisfy.
/// A restatement can only ever REPEAT a claim the artifact already makes: it is
/// held until the whole document has been walked and then required to match an
/// existing claim exactly. A row naming a path no other shape claimed, or the
/// same path with a different digest, refuses the artifact by name. So it can
/// never widen the corpus, only contradict it.
const RESTATED_CLAIM_PATH_KEY: &str = "source_file";
const RESTATED_CLAIM_DIGEST_KEY: &str = "source_sha256";

/// Require every restatement to repeat a claim the artifact already makes.
///
/// Run after the WHOLE document has been walked, because a restatement can sit
/// anywhere relative to the claim it repeats — `focus_defs_static` puts them
/// under `meta`, which serializes before `source_files`. Nothing here inserts
/// into `scan.claims`, so a restatement can never widen the proved corpus.
fn reconcile_restated_claims(scan: &mut ArtifactScan) -> Result<(), String> {
    let restated = std::mem::take(&mut scan.restated_claims);
    for row in restated {
        match scan.claims.get(&row.normalized_path) {
            Some(claim) if claim.sha256 == row.sha256 => {}
            Some(_) => {
                return Err(format!(
                    "the artifact claims two different contents for {}",
                    row.normalized_path
                ))
            }
            None => {
                return Err(format!(
                    "the restatement at {} names {}, which no content claim in this artifact \
                     covers, so it is a digest site the walk cannot account for",
                    row.breadcrumb, row.normalized_path
                ))
            }
        }
    }
    Ok(())
}

fn collect_claims(
    node: &Value,
    parent_key: Option<&str>,
    breadcrumb: &mut Vec<String>,
    scan: &mut ArtifactScan,
    unaccounted: &mut Vec<String>,
) -> Result<(), String> {
    match node {
        Value::Object(object) => {
            let mut consumed: Vec<&str> = Vec::new();
            for (path_key, digest_key, byte_key) in CLAIM_SHAPES {
                let (Some(path), Some(digest)) = (object.get(path_key), object.get(digest_key))
                else {
                    continue;
                };
                consumed.push(digest_key);
                match (path, digest) {
                    (Value::String(path), Value::String(digest)) => {
                        let bytes = byte_key
                            .and_then(|key| object.get(key))
                            .and_then(Value::as_u64);
                        insert_claim(scan, path, digest, bytes, breadcrumb)?;
                    }
                    (Value::Null, Value::Null) => {}
                    _ => {
                        return Err(format!(
                            "claim row at {} pairs {path_key}/{digest_key} inconsistently",
                            breadcrumb_text(breadcrumb)
                        ))
                    }
                }
            }
            // The restatement shape. It is checked AFTER the claim shapes so a
            // row carrying both a claim path key and `source_file` beside one
            // digest is caught rather than quietly resolved twice.
            if let (Some(path), Some(digest)) = (
                object.get(RESTATED_CLAIM_PATH_KEY),
                object.get(RESTATED_CLAIM_DIGEST_KEY),
            ) {
                if consumed.contains(&RESTATED_CLAIM_DIGEST_KEY) {
                    return Err(format!(
                        "row at {} spells one {RESTATED_CLAIM_DIGEST_KEY} for two different \
                         paths",
                        breadcrumb_text(breadcrumb)
                    ));
                }
                consumed.push(RESTATED_CLAIM_DIGEST_KEY);
                let (Value::String(path), Value::String(digest)) = (path, digest) else {
                    return Err(format!(
                        "restatement row at {} pairs {RESTATED_CLAIM_PATH_KEY}/\
                         {RESTATED_CLAIM_DIGEST_KEY} inconsistently",
                        breadcrumb_text(breadcrumb)
                    ));
                };
                require_sha256(digest, &breadcrumb_text(breadcrumb))?;
                let normalized_path = portable_content_manifest::normalize_game_relative_path(path)
                    .map_err(|error| {
                        format!("restatement at {}: {error}", breadcrumb_text(breadcrumb))
                    })?;
                scan.restated_claims.push(RestatedClaim {
                    normalized_path,
                    sha256: digest.clone(),
                    breadcrumb: breadcrumb_text(breadcrumb),
                });
            }
            // The map-keyed shape: the object carries a digest and a length but
            // names no path, so its own key in the parent map is the path.
            //
            // The key must LOOK like a content path — it has to carry a
            // directory separator or an extension — or the shape does not
            // apply and the digest falls through to the fail-closed sweep
            // below. Without that guard, any object anywhere that grew a
            // `sha256` beside a `bytes` would have its parent key read as a
            // path, and a build-input digest under a name like
            // `localisation_source` could become a bogus content "claim"
            // (Codex review round 1, finding 3).
            if !consumed.contains(&"sha256") {
                if let (Some(Value::String(digest)), Some(bytes), Some(key)) = (
                    object.get("sha256"),
                    object.get("bytes").and_then(Value::as_u64),
                    parent_key.filter(|key| key.contains('/') || key.contains('.')),
                ) {
                    consumed.push("sha256");
                    insert_claim(scan, key, digest, Some(bytes), breadcrumb)?;
                }
            }
            for (key, value) in object {
                if is_digest_shaped(key) {
                    breadcrumb.push(key.clone());
                    unaccounted.push(format!(
                        "{} (digest-shaped key)",
                        breadcrumb_text(breadcrumb)
                    ));
                    breadcrumb.pop();
                }
                if let Value::String(text) = value {
                    if is_digest_shaped(text) && !consumed.contains(&key.as_str()) {
                        breadcrumb.push(key.clone());
                        unaccounted.push(breadcrumb_text(breadcrumb));
                        breadcrumb.pop();
                    }
                }
                breadcrumb.push(key.clone());
                collect_claims(value, Some(key), breadcrumb, scan, unaccounted)?;
                breadcrumb.pop();
            }
        }
        Value::Array(rows) => {
            for (index, value) in rows.iter().enumerate() {
                breadcrumb.push(format!("[{index}]"));
                if let Value::String(text) = value {
                    if is_digest_shaped(text) {
                        unaccounted.push(breadcrumb_text(breadcrumb));
                    }
                }
                collect_claims(value, None, breadcrumb, scan, unaccounted)?;
                breadcrumb.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

fn insert_claim(
    scan: &mut ArtifactScan,
    path: &str,
    digest: &str,
    bytes: Option<u64>,
    breadcrumb: &[String],
) -> Result<(), String> {
    require_sha256(digest, &breadcrumb_text(breadcrumb))?;
    let normalized = portable_content_manifest::normalize_game_relative_path(path)
        .map_err(|error| format!("claim at {}: {error}", breadcrumb_text(breadcrumb)))?;
    let claim = ContentClaim {
        sha256: digest.to_string(),
        bytes,
    };
    match scan.claims.get(&normalized) {
        Some(existing)
            if existing.sha256 != claim.sha256
                || (existing.bytes.is_some()
                    && claim.bytes.is_some()
                    && existing.bytes != claim.bytes) =>
        {
            Err(format!(
                "the artifact claims two different contents for {normalized}"
            ))
        }
        Some(existing) if existing.bytes.is_some() => Ok(()),
        _ => {
            scan.claims.insert(normalized, claim);
            Ok(())
        }
    }
}

fn breadcrumb_text(breadcrumb: &[String]) -> String {
    if breadcrumb.is_empty() {
        String::new()
    } else {
        breadcrumb.join(".")
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_digest_shaped(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    if is_sha256(value) {
        Ok(())
    } else {
        Err(format!("{label} is not a lowercase sha256"))
    }
}

fn reconcile_localisation_sources(scans: &[ArtifactScan], refused: &mut Vec<RefusedArtifact>) {
    let Some(packaged) = scans
        .iter()
        .find(|scan| scan.role == compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN)
        .map(|scan| scan.artifact_file_sha256.clone())
    else {
        for scan in scans {
            if let Some(declared) = scan.localisation_source_sha256.as_ref() {
                refused.push(RefusedArtifact {
                    role: scan.role.clone(),
                    reason: format!(
                        "the artifact was built against localisation catalog {declared}, which \
                         this recording does not package"
                    ),
                });
            }
        }
        return;
    };
    for scan in scans {
        let Some(declared) = scan.localisation_source_sha256.as_ref() else {
            continue;
        };
        if declared != &packaged {
            refused.push(RefusedArtifact {
                role: scan.role.clone(),
                reason: format!(
                    "the artifact was built against localisation catalog {declared}, but this \
                     recording packages {packaged}"
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(document: Value) -> Result<ArtifactScan, String> {
        scan_artifact_document("fixture_role", &"a".repeat(64), &document)
    }

    #[test]
    fn every_claim_shape_in_the_shipped_artifacts_is_collected() {
        let scanned = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": {
                "scanned": [{ "relpath": "common/units/a.txt", "sha256": "1".repeat(64), "bytes": 7 }]
            },
            "sprites": {
                "GFX_a": {
                    "resolved_texturefile": "gfx/a.dds",
                    "texture_sha256": "2".repeat(64),
                    "texture_bytes": 9,
                    "texture_slots": [{
                        "resolved_texturefile": "gfx/a.dds",
                        "texture_sha256": "2".repeat(64),
                        "texture_bytes": 9,
                        "slot": "texturefile"
                    }]
                }
            },
            "catalog": { "row": { "source": "common/names/b.txt", "source_sha256": "3".repeat(64) } },
            "power_balance_gui": {
                "sources": {
                    "interface/c.gui": { "sha256": "4".repeat(64), "bytes": 11 }
                }
            }
        }))
        .expect("all four shapes scan");
        assert_eq!(
            scanned.claims.keys().collect::<Vec<_>>(),
            vec![
                "common/names/b.txt",
                "common/units/a.txt",
                "gfx/a.dds",
                "interface/c.gui",
            ]
        );
        assert_eq!(scanned.claims["gfx/a.dds"].bytes, Some(9));
        assert_eq!(scanned.claims["common/names/b.txt"].bytes, None);
    }

    #[test]
    fn a_digest_the_scan_cannot_account_for_refuses_the_artifact() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] },
            "new_surface": { "rows": [{ "asset": "gfx/new.dds", "asset_sha256": "5".repeat(64) }] }
        }))
        .expect_err("an unaccounted digest must refuse");
        assert!(
            error.contains("new_surface.rows.[0].asset_sha256"),
            "the refusal must name the site: {error}"
        );
    }

    #[test]
    fn the_document_self_digest_and_the_localisation_source_are_not_content_claims() {
        let scanned = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "localisation_source": {
                "path": "extractor/game_defs/out/localisation_gate/x/localisation_catalog.json",
                "sha256": "6".repeat(64),
                "bytes": 4096,
                "key_count": 3
            },
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] }
        }))
        .expect("the two non-content digests never become content claims");
        assert_eq!(scanned.claims.keys().collect::<Vec<_>>(), vec!["a.txt"]);
        assert_eq!(scanned.localisation_source_sha256, Some("6".repeat(64)));
    }

    /// `focus_defs_static` states one file twice, under two path keys.
    ///
    /// `meta.source_compatibility[]` names a game-relative path as
    /// `source_file` with the file's digest as `source_sha256`, while
    /// `source_files[]` names the SAME path as `relpath` with the same digest.
    /// The second spelling used to reach the fail-closed sweep and refuse the
    /// whole artifact for carrying "a digest site that is not a content claim".
    ///
    /// It is admitted as a RESTATEMENT, never as a claim: it must repeat a claim
    /// the artifact already makes, so it can never put a new path into the
    /// proved corpus. That matters because `normalize_game_relative_path`
    /// accepts a repo-relative build-input path perfectly well, and a claim
    /// shape would then let a build-input digest become content a layer could
    /// satisfy.
    ///
    /// kills: deleting the `RESTATED_CLAIM_*` branch in `collect_claims` makes
    /// case 1 refuse with `meta.source_compatibility.[0].source_sha256`, and the
    /// artifact uncertifiable. Making it call `insert_claim` instead of
    /// `scan.restated_claims.push` makes case 3 pass — a build input silently
    /// entering the corpus. Dropping the `consumed.contains` guard makes case 4
    /// resolve one digest against two paths. Deleting the
    /// `discarded.restated_claims` extension in the `localisation_source` pin
    /// makes case 3b pass, which is the one route a restatement could take
    /// without ever being reconciled.
    #[test]
    fn a_source_file_row_restates_an_existing_claim_and_can_never_widen_the_corpus() {
        // 1. agrees with source_files -> exactly one claim, and the claim keeps
        //    the length the real claim shape supplied.
        let scanned = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "meta": {
                "source_compatibility": [{
                    "compatibility": "a729_exact_trailing_root_closer",
                    "source_file": "common/national_focus/joint.txt",
                    "source_sha256": "4".repeat(64)
                }]
            },
            "source_files": [
                { "relpath": "common/national_focus/joint.txt", "sha256": "4".repeat(64), "bytes": 7360 }
            ]
        }))
        .expect("a compatibility row that agrees with source_files is one claim");
        assert_eq!(
            scanned.claims.keys().collect::<Vec<_>>(),
            vec!["common/national_focus/joint.txt"]
        );
        assert_eq!(
            scanned.claims["common/national_focus/joint.txt"].bytes,
            Some(7360)
        );

        // 2. same path, different digest -> the artifact contradicts itself.
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "meta": {
                "source_compatibility": [{
                    "source_file": "common/national_focus/joint.txt",
                    "source_sha256": "5".repeat(64)
                }]
            },
            "source_files": [
                { "relpath": "common/national_focus/joint.txt", "sha256": "4".repeat(64) }
            ]
        }))
        .expect_err("a compatibility row that disagrees must refuse the artifact");
        assert!(
            error.contains("two different contents for common/national_focus/joint.txt"),
            "{error}"
        );

        // 3. a path NO claim covers -- here a repo-relative build input, which
        //    path normalization accepts. It must refuse, not be proved.
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "meta": {
                "source_compatibility": [{
                    "source_file": "extractor/game_defs/out/build_input.json",
                    "source_sha256": "9".repeat(64)
                }]
            },
            "source_files": [
                { "relpath": "common/national_focus/joint.txt", "sha256": "4".repeat(64) }
            ]
        }))
        .expect_err("a restatement of a path nothing claims must refuse");
        assert!(
            error.contains("extractor/game_defs/out/build_input.json")
                && error.contains("no content claim in this artifact covers"),
            "{error}"
        );

        // 3b. the same shape INSIDE localisation_source, which is removed from
        //     the walked document. Its digest is marked consumed by the
        //     restatement branch and the row lands in a discarded scan, so
        //     without the explicit check it would reach neither the sweep nor
        //     reconcile_restated_claims.
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "localisation_source": {
                "path": "x/localisation_catalog.json",
                "sha256": "6".repeat(64),
                "source_file": "common/national_focus/joint.txt",
                "source_sha256": "9".repeat(64)
            },
            "source_files": [
                { "relpath": "common/national_focus/joint.txt", "sha256": "4".repeat(64) }
            ]
        }))
        .expect_err("a restatement inside localisation_source must refuse");
        assert!(
            error.contains("localisation_source restates a content claim for"),
            "{error}"
        );

        // 4. one digest, two competing path siblings.
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "meta": {
                "source_compatibility": [{
                    "source": "common/national_focus/a.txt",
                    "source_file": "common/national_focus/b.txt",
                    "source_sha256": "4".repeat(64)
                }]
            },
            "source_files": [
                { "relpath": "common/national_focus/joint.txt", "sha256": "4".repeat(64) }
            ]
        }))
        .expect_err("one digest for two paths must refuse");
        assert!(
            error.contains("one source_sha256 for two different paths"),
            "{error}"
        );
    }

    /// `technology_sharing_static` records WHICH effective layers and WHICH
    /// localisation files its catalog was built from, and digests each of those
    /// two canonical-JSON structures. Neither is any file's bytes at any VFS
    /// path, so neither can be resolved or claimed — but refusing them made the
    /// artifact uncertifiable while proving nothing.
    ///
    /// kills: dropping `layer_binding_sha256` /
    /// `localisation_source_inventory_sha256` from
    /// `LOCALISATION_SOURCE_PROVENANCE_DIGESTS` makes the first case refuse
    /// again. Replacing the named set with a blanket exemption of everything
    /// inside `localisation_source` makes the second and third cases pass.
    #[test]
    fn localisation_source_carries_named_build_provenance_digests_and_nothing_else() {
        let scanned = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "localisation_source": {
                "source": "static_localisation_join_r3",
                "path": "extractor/game_defs/out/localisation_gate/x/localisation_catalog.json",
                "sha256": "6".repeat(64),
                "compatibility": "verified_current_effective_layers_and_files",
                "layer_binding_sha256": "7".repeat(64),
                "localisation_source_inventory_sha256": "8".repeat(64),
                "localisation_source_file_count": 206,
                "key_count": 129087
            },
            "source_files": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }]
        }))
        .expect("the three named provenance digests are accounted for");
        assert_eq!(scanned.claims.keys().collect::<Vec<_>>(), vec!["a.txt"]);
        assert_eq!(scanned.localisation_source_sha256, Some("6".repeat(64)));

        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "localisation_source": {
                "path": "x/localisation_catalog.json",
                "sha256": "6".repeat(64),
                "some_future_sha256": "9".repeat(64)
            },
            "source_files": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }]
        }))
        .expect_err("an unnamed digest inside localisation_source must still refuse");
        assert!(error.contains("localisation_source.some_future_sha256"), "{error}");

        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "localisation_source": {
                "path": "x/localisation_catalog.json",
                "sha256": "6".repeat(64),
                "layer_binding_sha256": "not-a-digest"
            },
            "source_files": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }]
        }))
        .expect_err("a named provenance digest must still be a well-formed digest");
        assert!(error.contains("localisation_source.layer_binding_sha256"), "{error}");
    }

    #[test]
    fn an_artifact_without_a_self_digest_is_refused() {
        let error = scan(json!({
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] }
        }))
        .expect_err("an artifact with no self-digest must refuse");
        assert!(error.contains("carries no artifact_sha256"), "{error}");
    }

    #[test]
    fn a_null_texture_slot_is_not_a_claim_but_its_absence_is_still_proved() {
        let scanned = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "sprites": {
                "GFX_missing": { "resolved_texturefile": null, "texture_sha256": null }
            },
            "allowed_missing_textures": [
                { "texturefile": "gfx/leaders/missing.dds", "sprite": "GFX_missing" }
            ],
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] }
        }))
        .expect("a null slot scans");
        assert_eq!(scanned.claims.len(), 1);
        assert_eq!(scanned.absent_claims, vec!["gfx/leaders/missing.dds"]);
    }

    #[test]
    fn two_different_contents_for_one_path_refuse_the_artifact() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": {
                "a": [{ "relpath": "a.txt", "sha256": "1".repeat(64), "bytes": 1 }],
                "b": [{ "relpath": "a.txt", "sha256": "2".repeat(64), "bytes": 1 }]
            }
        }))
        .expect_err("a self-contradicting artifact must refuse");
        assert!(
            error.contains("two different contents for a.txt"),
            "{error}"
        );
    }

    #[test]
    fn an_artifact_declaring_unresolved_textures_is_refused() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "unresolved_textures": [{ "texturefile": "gfx/x.dds" }],
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] }
        }))
        .expect_err("an artifact its own gate would fail must not be certified");
        assert!(error.contains("unresolved texture reference"), "{error}");
    }

    #[test]
    fn a_bare_digest_inside_an_array_is_still_swept() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] },
            "bare_digests": ["9".repeat(64)]
        }))
        .expect_err("a bare digest in an array must refuse");
        assert!(error.contains("bare_digests.[0]"), "{error}");
    }

    #[test]
    fn a_digest_beside_a_length_under_a_non_path_key_is_not_a_claim() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] },
            "buildinfo": { "sha256": "9".repeat(64), "bytes": 12 }
        }))
        .expect_err("a digest under a non-path key must refuse, not become a claim");
        assert!(error.contains("buildinfo.sha256"), "{error}");
    }

    #[test]
    fn an_uppercase_digest_is_swept_as_a_key_and_as_a_value() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] },
            "future_surface": { "asset_sha256": "AB".repeat(32) }
        }))
        .expect_err("an uppercase digest VALUE must refuse");
        assert!(error.contains("future_surface.asset_sha256"), "{error}");

        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] },
            "by_digest": { "CD".repeat(32): { "relpath": "b.txt" } }
        }))
        .expect_err("an uppercase digest KEY must refuse");
        assert!(error.contains("digest-shaped key"), "{error}");
    }

    #[test]
    fn an_uppercase_digest_in_a_claim_row_refuses_rather_than_being_admitted() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "AB".repeat(32) }] }
        }))
        .expect_err("an uppercase claim digest must refuse, never be admitted");
        assert!(error.contains("is not a lowercase sha256"), "{error}");
    }

    #[test]
    fn a_digest_shaped_object_key_is_swept_too() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "a.txt", "sha256": "1".repeat(64) }] },
            "by_digest": { "7".repeat(64): { "relpath": "b.txt" } }
        }))
        .expect_err("a digest-shaped key must refuse");
        assert!(error.contains("digest-shaped key"), "{error}");
    }

    #[test]
    fn a_path_shaped_key_that_is_not_content_refuses_at_the_join_rather_than_passing() {
        let scanned = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "source_files": { "s": [{ "relpath": "interface/real.gui", "sha256": "1".repeat(64) }] },
            "build.v1": { "sha256": "9".repeat(64), "bytes": 12 }
        }))
        .expect("a path-shaped key becomes a claim");
        assert!(
            scanned.claims.contains_key("build.v1"),
            "the map-keyed shape is lexical, and this documents it: {:?}",
            scanned.claims.keys().collect::<Vec<_>>()
        );

        let root = std::env::temp_dir().join(format!(
            "jb_attach_join_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let install = root.join("game");
        fs::create_dir_all(install.join("interface")).unwrap();
        fs::write(install.join("interface/real.gui"), b"real").unwrap();
        let vfs = effective_vfs_from_launch(
            &RuntimeContentRoots {
                install_root: install,
                user_dir: root.join("profile"),
            },
            &[portable_content_manifest::RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: portable_content_manifest::LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            }],
            &"a".repeat(64),
        )
        .unwrap();

        let bogus = check_claim(&vfs, "build.v1", &scanned.claims["build.v1"])
            .expect_err("a name that is not content must not join");
        assert!(bogus.contains("absent from the effective VFS"), "{bogus}");
        let real = check_claim(
            &vfs,
            "interface/real.gui",
            &scanned.claims["interface/real.gui"],
        )
        .expect_err("a real file whose digest disagrees must not join");
        assert!(real.starts_with("sha256 "), "{real}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_array_element_never_inherits_its_arrays_key_as_a_path() {
        let error = scan(json!({
            "artifact_sha256": "0".repeat(64),
            "flags": [
                { "sha256": "1".repeat(64), "bytes": 3 },
                { "sha256": "2".repeat(64), "bytes": 4 }
            ]
        }))
        .expect_err("a keyless digest-bearing row must refuse, not borrow a name");
        assert!(error.contains("flags.[0].sha256"), "{error}");
    }

    #[test]
    fn a_localisation_source_naming_a_catalog_this_recording_does_not_package_refuses() {
        let scans = vec![ArtifactScan {
            role: "decisions_static_definitions".into(),
            artifact_file_sha256: "a".repeat(64),
            claims: BTreeMap::new(),
            absent_claims: Vec::new(),
            localisation_source_sha256: Some("b".repeat(64)),
            restated_claims: Vec::new(),
        }];
        let mut refused = Vec::new();
        reconcile_localisation_sources(&scans, &mut refused);
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0].reason.contains("does not package"),
            "{}",
            refused[0].reason
        );
    }

    #[test]
    fn a_localisation_source_disagreeing_with_the_packaged_catalog_refuses() {
        let scans = vec![
            ArtifactScan {
                role: compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN.into(),
                artifact_file_sha256: "c".repeat(64),
                claims: BTreeMap::new(),
                absent_claims: Vec::new(),
                localisation_source_sha256: None,
                restated_claims: Vec::new(),
            },
            ArtifactScan {
                role: "ship_definitions_static".into(),
                artifact_file_sha256: "a".repeat(64),
                claims: BTreeMap::new(),
                absent_claims: Vec::new(),
                localisation_source_sha256: Some("b".repeat(64)),
                restated_claims: Vec::new(),
            },
        ];
        let mut refused = Vec::new();
        reconcile_localisation_sources(&scans, &mut refused);
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].role, "ship_definitions_static");
    }

    #[test]
    fn the_layer_stack_digest_moves_when_the_stack_is_reordered() {
        let a = ProofLayer {
            ordinal: 0,
            kind: "base",
            layer_id: "base".into(),
            descriptor_sha256: None,
            descriptor_file_sha256: None,
            source_container_sha256: None,
        };
        let b = ProofLayer {
            ordinal: 1,
            kind: "mod",
            layer_id: "mod/x.mod".into(),
            descriptor_sha256: Some("1".repeat(64)),
            descriptor_file_sha256: Some("2".repeat(64)),
            source_container_sha256: None,
        };
        let forward = layer_stack_sha256(&[a.clone(), b.clone()]).unwrap();
        let reversed = layer_stack_sha256(&[b, a]).unwrap();
        assert_ne!(
            forward, reversed,
            "load order decides which layer wins a path, so a reordered stack is a different world"
        );
    }

    fn published_layer(layer_id: &str, descriptor_file_sha256: Option<&str>) -> ProofLayer {
        ProofLayer {
            ordinal: 0,
            kind: "mod",
            layer_id: layer_id.to_string(),
            descriptor_sha256: Some("s".repeat(64)),
            descriptor_file_sha256: descriptor_file_sha256.map(str::to_string),
            source_container_sha256: None,
        }
    }

    fn bracket(layer_descriptors: &[(&str, &str)]) -> WorldBracket {
        WorldBracket {
            executable_sha256: "e".repeat(64),
            executable_size_bytes: 1,
            dlc_load_sha256: "d".repeat(64),
            layer_descriptors: layer_descriptors
                .iter()
                .map(|(id, sha)| ((*id).to_string(), (*sha).to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_descriptor_that_changed_before_the_bracket_opened_does_not_settle() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let published = vec![
            published_layer("base", None),
            published_layer("mod/one.mod", Some(&a)),
        ];

        assert!(
            published_layers_match_bracket(&published, &bracket(&[("mod/one.mod", &a)])),
            "an unchanged descriptor agrees, and the base layer is exempt"
        );
        assert!(
            !published_layers_match_bracket(&published, &bracket(&[("mod/one.mod", &b)])),
            "a descriptor rewritten between resolution and the open observation must not settle"
        );
        assert!(
            !published_layers_match_bracket(&published, &bracket(&[])),
            "a descriptor the bracket never saw must not settle"
        );
        assert!(
            !published_layers_match_bracket(
                &published,
                &bracket(&[("mod/one.mod", &a), ("mod/two.mod", &b)])
            ),
            "a layer the published rows do not name must not settle"
        );
    }

    #[test]
    fn the_census_compares_the_descriptors_it_just_observed() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let mut proof = AttachTimeContentProof {
            game_process_start_unix: None,
            observed_at_unix: 0,
            completed_at_unix: 0,
            elapsed_ms: 0,
            hashed_bytes: 0,
            hashed_files: 0,
            window_attempts: 1,
            window_settled: true,
            executable_sha256: "e".repeat(64),
            executable_size_bytes: 1,
            dlc_load_sha256: "d".repeat(64),
            layer_stack: vec![
                published_layer("base", None),
                published_layer("mod/one.mod", Some(&a)),
            ],
            layer_stack_sha256: "f".repeat(64),
            artifact_joins: Vec::new(),
            refused_artifacts: Vec::new(),
            required_content: Vec::new(),
            refused_required_content: Vec::new(),
        };

        require_bracket_descriptors_match_certified(
            &bracket(&[("mod/one.mod", &a)]),
            &proof,
            "before",
        )
        .expect("an unchanged descriptor is the certified one");

        let error = require_bracket_descriptors_match_certified(
            &bracket(&[("mod/one.mod", &b)]),
            &proof,
            "after",
        )
        .expect_err("a rewritten descriptor is not the certified one");
        assert!(error.contains("after"), "the phase is named: {error}");

        proof.layer_stack = vec![published_layer("base", None)];
        require_bracket_descriptors_match_certified(&bracket(&[]), &proof, "before")
            .expect("a stack of nothing but base observes no descriptors");
    }
}
