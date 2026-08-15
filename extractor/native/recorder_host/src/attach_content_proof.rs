//! KIND C attach-time content certification: what the engine's own effective
//! VFS exposed while the recorder was attaching, joined against every content
//! claim the packaged static artifacts make.
//!
//! # Why this exists
//!
//! The launch-time content epoch (`launch_time_static_bundle_v1`) is built from
//! a SUSPENDED process, before `ResumeThread`. The shipped `collector_record`
//! cannot take one: the client-driving actuator is compile-excluded
//! (`main.rs`'s `#[cfg(not(feature = "evidence-client-driving"))] let
//! early_launch: Option<EarlyLaunchCli> = None`), and the packaging scripts
//! assert it stays that way. So every production recording is a LATE ATTACH by
//! construction, and the launch-time epoch is a route the product will never
//! take. Without a second route, a production recording can never be certified
//! as content-bound at all.
//!
//! KIND C is that second route. It replaces "observed before the engine was
//! resumed" with "observed while the recorder was attaching", and it is
//! deliberately a stronger content claim than the layer identity the recorder
//! already sealed: `modset` names WHICH LAYERS are mounted, each pinned by
//! descriptor hash, but nothing in it states the bytes of the content corpus.
//! KIND C states them, by resolving every path the packaged artifacts claim
//! through the engine's effective VFS and requiring the winning layer's bytes to
//! hash to the declared digest.
//!
//! # The window, stated plainly
//!
//! Hashing ~1.8 GB across ~34,600 files takes an interval, so this is not an
//! attach *instant* — it is a bounded attach *window* `[observed_at_unix,
//! completed_at_unix]` (owner ruling 2026-08-15). Two things bound it:
//!
//! 1. **A world bracket.** The executable, `dlc_load.json`, and every mounted
//!    layer's descriptor file are hashed BEFORE the walk and re-hashed AFTER
//!    it. A difference means the world changed mid-window, so nothing observed
//!    inside it describes one install; the whole attempt is discarded and
//!    retried.
//! 2. **Consistency reads where they can say something.** A file whose bytes
//!    hash to the digest an artifact recorded BEFORE the capture agrees with an
//!    independent prior observation, so a passing join is self-bracketing — the
//!    only way it could be wrong is a file that changed to exactly the recorded
//!    content. A FAILING join is the ambiguous case ("really different" versus
//!    "changed while we were reading it"), so every failure is re-read once and
//!    only a failure that reproduces refuses its artifact by name. A failure
//!    that does not reproduce is churn, and churn retries.
//!
//! What this deliberately does NOT do is take a filesystem snapshot. The owner
//! ruled the bounded window is the contract; no snapshot machinery.
//!
//! # Never make the game wait
//!
//! The walk runs on recorder-owned worker threads spawned at attach. It touches
//! no engine thread, holds no engine lock, and the capture starts and runs
//! while it is still going. The result is joined at finalize, before the
//! manifest is written. Progress is published through the recorder's existing
//! progress receipt (`finalize_progress`), under its own phase, so the only
//! visible effect of attaching is a progress indication.
//!
//! # Retry, not an error
//!
//! Window drift auto-retries (owner ruling: "retry-until-clean; never surface
//! an error the user must act on"). Retries are bounded only so a machine whose
//! corpus is genuinely being rewritten — a game update mid-attach — cannot spin
//! forever; when the bound is reached the proof still seals, with
//! `window_settled = false` and every artifact named as unproven. Partial data
//! still seals; the certification then refuses those artifacts by name rather
//! than claiming them.

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

/// Manifest field naming WHICH content proof ran. Positively written by every
/// recording this recorder produces; a consumer never infers the proof from
/// which other fields happen to be present.
pub const CONTENT_PROOF_KIND_FIELD: &str =
    portable_content_manifest::attach_content_epoch::CONTENT_PROOF_KIND_FIELD;

/// Manifest field carrying the KIND C proof block itself.
pub const ATTACH_TIME_CONTENT_PROOF_FIELD: &str =
    portable_content_manifest::attach_content_epoch::ATTACH_TIME_CONTENT_PROOF_FIELD;

/// The launch-time epoch: a content manifest observed before `ResumeThread`.
pub const KIND_LAUNCH_TIME_STATIC_BUNDLE_V1: &str =
    portable_content_manifest::attach_content_epoch::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1;

/// The attach-time effective-VFS join this module produces.
pub const KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1: &str =
    portable_content_manifest::attach_content_epoch::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1;

/// Neither proof ran. Written positively so "no content proof" is a claim the
/// recording makes rather than a field a consumer finds missing.
pub const KIND_NONE: &str = portable_content_manifest::attach_content_epoch::KIND_NONE;

/// Schema of the proof block.
pub const SCHEMA: &str = portable_content_manifest::attach_content_epoch::ATTACH_PROOF_SCHEMA;

/// Progress phase published while the fingerprint runs.
pub const PHASE_CONTENT_FINGERPRINT: &str = "content_fingerprint";

/// The one playback-required content file the production roster consumes
/// DIRECTLY, rather than through a packaged catalog: D81 resolves the province
/// catalog as portable semantic identity.
///
/// Declared here so the recorder that joins it and the gate that requires it
/// read one spelling. It is an engine file name the production launch route
/// already resolves by the same literal, not a discovered content value.
pub const D81_PROVINCE_CATALOG_PATH: &str = "map/definition.csv";

/// Why D81 needs it, carried into the receipt so a refusal explains itself.
pub const D81_PROVINCE_CATALOG_PURPOSE: &str = "D81 province catalog";

/// Drop the calling thread below normal CPU priority for the rest of its life.
///
/// The content walk is a one-off ~1.8 GB read on a machine that must be assumed
/// to be a modest four-core box running the game at max speed. It never blocks
/// an engine thread — nothing here takes an engine lock or suspends the target —
/// but it does compete for cores with a capture whose sweep has a 48 ms
/// per-game-hour budget, so the walk is the work that yields.
///
/// # Why CPU priority and NOT Windows' background I/O mode
///
/// Windows' `THREAD_MODE_BACKGROUND_BEGIN` throttles I/O to the lowest band as
/// well as lowering CPU priority. It was implemented, and then MEASURED on a
/// live capture: the identical walk over the identical corpus took **349,732
/// ms** against **81,886 ms** at normal priority — 4.3x
/// (`runs/jb_kind_c_certification/FINGERPRINT_COST.md`).
///
/// That 4.3x is the one thing those runs establish, and even it is not
/// attributable to a mechanism: background mode changes CPU priority AND I/O
/// priority together, and the two captures differ in more than the setting, so
/// which half produced the slowdown is unknown. Zero engine-paused observation
/// windows is likewise not the same fact as zero slowdown.
///
/// What ships is therefore chosen on ONE mechanical property and no
/// performance claim at all: `THREAD_PRIORITY_BELOW_NORMAL` lowers CPU
/// scheduling priority and does not enter the background I/O band. Its actual
/// speed, and its actual effect on a capture, are UNKNOWN — see residual (e)
/// in `runs/jb_kind_c_certification/RESIDUALS.md` for the controlled
/// measurement that would settle them.
///
/// Best-effort by construction: a failure to lower priority is not a reason to
/// refuse a capture, and the walk is correct at any priority.
fn lower_this_thread_to_background_priority() {
    #[cfg(windows)]
    {
        // Declared locally rather than pulling in a Windows binding crate: two
        // symbols, both stable since Vista.
        extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        // SAFETY: both take a pseudo-handle to the calling thread and change
        // only its own scheduling. Neither reads or writes process memory.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}

/// Default number of attempts before the window is declared unsettled.
///
/// Each attempt is a full re-walk, so this is generous rather than tight: a
/// machine that drifts eight times in a row is one whose content is actively
/// being rewritten, and no number of further passes would settle it.
pub const DEFAULT_MAX_WINDOW_ATTEMPTS: u32 = 8;

/// How many refused paths an artifact's refusal reason names before it
/// summarizes. Bounded so one broken artifact cannot write megabytes of reason
/// text into every manifest.
const MAX_NAMED_REFUSED_PATHS: usize = 8;

/// Upper bound on claims per artifact. A ceiling, not a size expectation: it
/// exists so a malformed artifact cannot turn the walk into an unbounded one.
/// The real corpus is ~34,600 paths across six artifacts.
const MAX_CLAIMS_PER_ARTIFACT: usize = 1 << 20;

/// One packaged static artifact to certify.
#[derive(Clone, Debug)]
pub struct ArtifactRequest {
    /// Canonical role name, as it appears in
    /// `replay_packaging.static_artifacts[*].name`.
    pub role: String,
    /// Absolute path the recorder was given on the command line.
    pub path: PathBuf,
}

/// One playback-required content file the scheduled roster consumes directly
/// (as opposed to through a packaged artifact).
#[derive(Clone, Debug)]
pub struct RequiredContentRequest {
    /// Why this file is required, named so a refusal explains itself.
    pub purpose: String,
    /// Game-relative logical path, resolved through the effective VFS.
    pub logical_path: String,
}

/// What the certification was asked to prove.
#[derive(Clone, Debug)]
pub struct AttachContentProofRequest {
    pub artifacts: Vec<ArtifactRequest>,
    pub required_content: Vec<RequiredContentRequest>,
    /// Worker threads for the content walk.
    pub worker_threads: usize,
    pub max_window_attempts: u32,
}

impl AttachContentProofRequest {
    /// Worker count for a machine we must assume is a modest four-core box
    /// running the game at max speed.
    ///
    /// Half the reported parallelism, never more than four: the sweep budget is
    /// 48 ms per game hour and the capture's readers must not be starved by a
    /// one-off content walk. Hashing is I/O-bound in practice, so the marginal
    /// return above this is small anyway.
    pub fn default_worker_threads() -> usize {
        std::thread::available_parallelism()
            .map(|value| (value.get() / 2).clamp(1, 4))
            .unwrap_or(1)
    }
}

/// Live counters the progress publisher reads. Monotone within an attempt; a
/// retry resets the per-attempt totals and advances `attempt`.
#[derive(Debug, Default)]
pub struct AttachFingerprintProgress {
    pub attempt: AtomicU64,
    pub files_total: AtomicU64,
    pub files_done: AtomicU64,
    pub bytes_hashed: AtomicU64,
    pub finished: AtomicBool,
}

impl AttachFingerprintProgress {
    /// Work quantity for the progress receipt: bytes hashed so far.
    ///
    /// A retry legitimately makes this DECREASE, which is exactly why the
    /// receipt's contract treats only CHANGE as progress and never the
    /// direction.
    pub fn work_bytes(&self) -> u64 {
        self.bytes_hashed.load(Ordering::Relaxed)
    }
}

/// A running fingerprint. Owned by the recorder, joined at finalize.
pub struct AttachContentProofHandle {
    progress: Arc<AttachFingerprintProgress>,
    thread: JoinHandle<Result<AttachTimeContentProof, String>>,
}

impl AttachContentProofHandle {
    pub fn progress(&self) -> Arc<AttachFingerprintProgress> {
        Arc::clone(&self.progress)
    }

    /// Wait for the fingerprint and take its result.
    ///
    /// Called at finalize, when the capture is already over, so waiting here
    /// cannot delay a single game hour. A panicked worker is reported as a
    /// refusal rather than propagated: a capture that recorded real hours must
    /// never be destroyed by the certification thread.
    pub fn join(self) -> Result<AttachTimeContentProof, String> {
        match self.thread.join() {
            Ok(result) => result,
            Err(_) => Err("attach-time content fingerprint thread panicked".to_string()),
        }
    }
}

/// The sealed proof block.
#[derive(Clone, Debug)]
pub struct AttachTimeContentProof {
    /// When the ENGINE started, from the same resolved modset the layer stack
    /// comes from.
    ///
    /// Carried because it names what this proof does NOT cover. The window
    /// opens at the attach, which is later — sometimes much later, since
    /// capture is attach-and-play — so content the engine loaded at ITS launch
    /// and that changed before the window opened is outside every observation
    /// here. A consumer reads the gap rather than being left to assume there
    /// is none.
    pub game_process_start_unix: Option<i64>,
    pub observed_at_unix: i64,
    pub completed_at_unix: i64,
    /// Wall time of the whole fingerprint, measured on the monotonic clock.
    ///
    /// The two unix stamps are second-resolution and are the WINDOW; this is
    /// the COST, and the two are reported separately so a short window is not
    /// read as a cheap one.
    pub elapsed_ms: u64,
    /// Bytes and files actually read and hashed on the settling attempt. The
    /// price of the proof, stated by the recording that paid it.
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
    /// Digest of the artifact FILE as read at attach. The gate requires this to
    /// equal the digest of the bytes it admitted out of the container, so a
    /// receipt can never be read as covering some other build of the artifact.
    pub artifact_sha256: String,
    pub claimed_paths: u64,
    pub joined_bytes: u64,
    /// Paths the artifact declared LEGITIMATELY ABSENT and that the attach-time
    /// VFS confirmed are absent. A join that only re-proved present files would
    /// let a texture appear where the artifact said none exists, unnoticed.
    pub proven_absent_paths: u64,
    /// The localisation catalog this artifact declares it was joined against,
    /// reconciled against the packaged `localisation_join` artifact.
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

/// Digest over the ordered layer stack.
///
/// Taken over the same JSON rows the manifest carries, so a consumer
/// re-derives it from the block's own bytes rather than trusting a value it
/// cannot recompute. Order is load-bearing: load order decides which layer wins
/// a path, so a reordered stack is a different world and must produce a
/// different digest.
/// ONE authority, not two spellings: the rows are handed to
/// [`portable_content_manifest::attach_content_epoch::layer_stack_digest`],
/// which is also what the replay consumer re-derives the value with. A local
/// second canonicalization here is exactly the harness-authority drift that
/// makes a digest disagree with itself across a seam.
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

/// Start the fingerprint on recorder-owned threads and return immediately.
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

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

/// Everything outside the artifacts that decides what the effective VFS
/// exposes. Re-observed at the close of the window; a difference discards the
/// attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorldBracket {
    executable_sha256: String,
    executable_size_bytes: u64,
    dlc_load_sha256: String,
    /// `(layer_id, raw descriptor file digest)`, in mount order.
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

/// The whole proof, retried until the world bracket holds still.
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

    // Artifacts are read ONCE, outside the retry loop: they are the recorder's
    // own inputs, not engine content, and re-reading them per attempt would let
    // a retry silently certify a different build of the artifact than the one
    // the capture packages.
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
    progress
        .files_total
        .store(files_total, Ordering::Relaxed);

    let mut last: Option<WalkOutcome> = None;
    let mut window_settled = false;
    let mut attempts = 0u32;
    while attempts < max_attempts {
        attempts += 1;
        progress.attempt.store(u64::from(attempts), Ordering::Relaxed);
        progress.files_done.store(0, Ordering::Relaxed);
        progress.bytes_hashed.store(0, Ordering::Relaxed);

        let launch = PortableLaunchBinding::from_modset(report)?;
        let layers = ordered_content_layers_from_resolved_launch(&roots, &launch, report)?;
        let open = observe_world_bracket(&roots, &exe_path, &layers)?;

        let proof_layers = layers
            .iter()
            .map(|layer| ProofLayer {
                ordinal: layer.recorded.ordinal,
                kind: layer_kind_name(layer.recorded.layer_kind),
                layer_id: layer.recorded.layer_id.clone(),
                descriptor_sha256: layer.recorded.descriptor_sha256.clone(),
                descriptor_file_sha256: layer.descriptor_file_sha256.clone(),
                source_container_sha256: layer.recorded.source_container_sha256.clone(),
            })
            .collect::<Vec<_>>();
        let stack_sha256 = layer_stack_sha256(&proof_layers)?;
        let recorded = layers
            .iter()
            .map(|layer| layer.recorded.clone())
            .collect::<Vec<_>>();
        let vfs = effective_vfs_from_launch(&roots, &recorded, &stack_sha256)?;

        let mut outcome = walk_corpus(&vfs, &scans, request, progress);
        // INSIDE the window. Resolving this after the close observation would
        // put the one content file the roster reads directly outside the very
        // bracket that makes the rest of the proof a statement about one
        // install (Codex review round 1, finding 4).
        outcome.required_content = resolve_required_content(&vfs, &request.required_content);
        let close = observe_world_bracket(&roots, &exe_path, &layers)?;
        let bracket_held = open == close;
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
        // The window never settled, so nothing observed inside it is a
        // statement about one install. Every artifact is named as unproven
        // rather than reported with the results of a walk over a moving
        // corpus, and the recording still seals.
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

/// The wire spelling of a layer kind, back to the static token.
///
/// A verifier re-deriving `layer_stack_sha256` must rebuild the rows through
/// this rather than pass the string it read through: the digest is over the
/// CANONICAL spellings, so an unrecognized kind has to refuse instead of
/// hashing whatever the document happened to say.
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
    /// A mismatch that did NOT reproduce on its consistency read: the corpus
    /// moved under the walk, so the attempt is discarded.
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

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

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
                // Once per worker, before it reads anything.
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
                            let mut guard =
                                mismatches.lock().expect("attach proof mismatch lock");
                            guard.push((path.clone(), reason));
                        }
                    }
                }
            });
        }
    });

    let mismatches = mismatches.into_inner().expect("attach proof mismatch lock");
    if !mismatches.is_empty() {
        // THE CONSISTENCY READ. A mismatch is ambiguous — the file may really
        // differ, or it may have been rewritten while the walk was reading it.
        // Re-read only the failures: a reproduced mismatch is a real one and
        // refuses this artifact by name; one that does not reproduce is churn,
        // and churn retries the whole window rather than certifying a corpus
        // that moved under the observation.
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

    // The NEGATIVE claims. An artifact that says a texture is legitimately
    // absent is making an assertion about the corpus too, and a join that only
    // re-proved present files would let a file appear where the artifact said
    // none exists.
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

/// Resolve one claimed path and compare it against what the artifact declared.
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

// ---------------------------------------------------------------------------
// Claim extraction
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ContentClaim {
    sha256: String,
    bytes: Option<u64>,
}

#[derive(Debug)]
struct ArtifactScan {
    role: String,
    artifact_file_sha256: String,
    /// Normalized game-relative path -> the digest and length the artifact
    /// declares for it.
    claims: BTreeMap<String, ContentClaim>,
    /// Paths the artifact declares are legitimately absent.
    absent_claims: Vec<String>,
    localisation_source_sha256: Option<String>,
}

fn read_and_scan_artifact(request: &ArtifactRequest) -> Result<ArtifactScan, String> {
    let bytes = fs::read(&request.path)
        .map_err(|error| format!("read static artifact failed: {error}"))?;
    let artifact_file_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("static artifact is not JSON: {error}"))?;
    scan_artifact_document(&request.role, &artifact_file_sha256, &document)
}

/// Turn one artifact document into the set of content claims it makes.
///
/// # The rule, and why it is shaped this way
///
/// An earlier design walked `source_files` alone. That covers 8% of the corpus:
/// the gfx artifact carries 23,123 further hashed texture paths in a DIFFERENT
/// row shape under `sprites[*].texture_slots[*]`, the templates artifact
/// carries 2,655 `(source, source_sha256)` rows, and the gfx artifact carries
/// two more as MAP KEYS under `power_balance_gui.sources`. A walker that knows
/// the surfaces by name is a walker that silently misses the next one.
///
/// So the scan is over the WHOLE document and is fail-closed on anything it
/// cannot account for: every 64-hex digest in the document must be consumed by
/// one of the claim shapes below or be one of the two named non-content
/// digests, and anything else refuses the artifact by name. A new claim surface
/// therefore announces itself instead of being quietly skipped.
///
/// The claim shapes, all decided on SIBLING KEYS rather than on position:
///
/// * `{relpath, sha256, bytes?}` — `source_files[*]`, `flags[*]`
/// * `{resolved_texturefile, texture_sha256, texture_bytes?}` — sprites and
///   their texture slots; both fields null together means the slot has no
///   texture and is covered by `allowed_missing_textures`
/// * `{source, source_sha256}` — the templates catalog's per-row provenance
/// * an object carrying `sha256` and `bytes` and NO path sibling, whose own key
///   in the parent map is the path — `power_balance_gui.sources`
///
/// The two non-content digests:
///
/// * top-level `artifact_sha256` — the document's self-digest, which the gate
///   already recomputes independently
/// * `localisation_source.sha256` — the localisation catalog this artifact was
///   BUILT against, reconciled against the packaged `localisation_join`
///   artifact rather than resolved as game content
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
    };
    let root = document
        .as_object()
        .ok_or_else(|| "static artifact root is not a JSON object".to_string())?;

    // THE TWO NON-CONTENT DIGESTS ARE REMOVED FROM THE DOCUMENT, not exempted
    // from the sweep by name.
    //
    // An allowlist keyed on where a digest SITS would let a future document put
    // an unrelated digest at the same key path and inherit the exemption — and,
    // worse, `localisation_source` would still be walked, so if it ever grew a
    // `bytes` field the map-keyed claim shape below would fire on it and turn a
    // build-input digest into a bogus content "proof" for a path named
    // `localisation_source`. Taking both out of the walked copy makes the rule
    // absolute: EVERY digest in what is walked must be a content claim.
    let mut walked = document.clone();
    let walked_root = walked
        .as_object_mut()
        .ok_or_else(|| "static artifact root is not a JSON object".to_string())?;
    let self_digest = walked_root
        .remove("artifact_sha256")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| "the artifact carries no artifact_sha256 self-digest".to_string())?;
    require_sha256(&self_digest, "artifact_sha256")?;
    // The localisation catalog this artifact was BUILT against. It is a repo
    // path plus a digest, never engine content, so it is reconciled against the
    // packaged localisation catalog rather than resolved through the VFS.
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
        // inside it is pinned: exactly one digest, the one just read. A future
        // `localisation_source` that grew a second digest would otherwise be
        // exempted along with it (Codex review round 1, finding 3).
        let mut other = Vec::new();
        let mut discarded = ArtifactScan {
            role: String::new(),
            artifact_file_sha256: String::new(),
            claims: BTreeMap::new(),
            absent_claims: Vec::new(),
            localisation_source_sha256: None,
        };
        collect_claims(
            &source,
            None,
            &mut vec!["localisation_source".to_string()],
            &mut discarded,
            &mut other,
        )?;
        other.retain(|site| site != "localisation_source.sha256");
        // A claim shape inside the removed object would be dropped on the floor
        // rather than flagged, so it is counted here too.
        other.extend(discarded.claims.into_keys().map(|path| {
            format!("localisation_source declares a content claim for {path}")
        }));
        if !other.is_empty() {
            return Err(format!(
                "localisation_source carries {} digest site(s) beyond the catalog digest, \n                 which the walk cannot account for: {}",
                other.len(),
                other.join("; ")
            ));
        }
        scan.localisation_source_sha256 = Some(sha256.to_string());
    }

    // A non-empty `unresolved_textures` is the artifact itself saying it could
    // not account for content it references. The generating gate refuses to
    // pass with any (`gfx_icon_manifest_gate.py`'s `"unresolved_textures":
    // meta["unresolved_textures"] == 0`), so one here means the artifact was
    // not built by a passing gate and nothing about it should be certified.
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
                        let bytes = byte_key.and_then(|key| object.get(key)).and_then(Value::as_u64);
                        insert_claim(scan, path, digest, bytes, breadcrumb)?;
                    }
                    // Both null together is the artifact saying this slot has no
                    // texture. It is not a claim, and it is not an omission
                    // either: `allowed_missing_textures` carries the matching
                    // negative claim, which IS proved.
                    (Value::Null, Value::Null) => {}
                    _ => {
                        return Err(format!(
                            "claim row at {} pairs {path_key}/{digest_key} inconsistently",
                            breadcrumb_text(breadcrumb)
                        ))
                    }
                }
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
                // A digest-shaped KEY is a digest site too. Nothing in the
                // shipped artifacts is keyed by digest, which is exactly why a
                // future claim surface that WAS would otherwise be skipped
                // rather than refused (Codex review round 2, finding 3).
                if is_digest_shaped(key) {
                    breadcrumb.push(key.clone());
                    unaccounted.push(format!("{} (digest-shaped key)", breadcrumb_text(breadcrumb)));
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
                // A BARE digest as an array element is a digest site too, and
                // the sweep must see it. It has no sibling path and no key of
                // its own, so it can never be a claim — which makes it exactly
                // the shape that would otherwise be silently skipped (Codex
                // review round 1, finding 3).
                if let Value::String(text) = value {
                    if is_digest_shaped(text) {
                        unaccounted.push(breadcrumb_text(breadcrumb));
                    }
                }
                // An array element has no key of its own, so the map-keyed
                // shape cannot apply to it; passing the array's key down would
                // make every element claim the array's name as its path.
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
        // The same path claimed twice must be claimed identically. Two
        // different digests for one path is the artifact contradicting itself,
        // and a join could then be "passed" by whichever copy the walker
        // happened to keep.
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

/// A digest a claim may be ADMITTED under: exactly 64 lowercase hex.
fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Anything that LOOKS like a digest, for the fail-closed sweep only.
///
/// Deliberately wider than [`is_sha256`], and the asymmetry is the point
/// (Codex review round 3, finding 1): admission stays lowercase-only, because
/// that is the spelling every artifact and every join receipt uses, while the
/// sweep has to notice a digest whatever its case — otherwise a future claim
/// surface written in uppercase hex would be silently skipped by the very rule
/// that exists to refuse the surfaces nobody anticipated.
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

/// Every artifact that names the localisation catalog it was joined against
/// must name the SAME one, and it must be the catalog this recording packages.
///
/// Without this the recording could package localisation catalog A while five
/// of its artifacts were built against catalog B, and every content join would
/// still pass: the joins prove each artifact agrees with the ENGINE, not that
/// the artifacts agree with each other.
fn reconcile_localisation_sources(scans: &[ArtifactScan], refused: &mut Vec<RefusedArtifact>) {
    let Some(packaged) = scans
        .iter()
        .find(|scan| scan.role == compact_contract::STATIC_ARTIFACT_LOCALISATION_JOIN)
        .map(|scan| scan.artifact_file_sha256.clone())
    else {
        // No localisation catalog is packaged, so there is nothing to reconcile
        // against. Any artifact that declares one is refused rather than
        // certified against a catalog this recording does not carry.
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
        // One document carrying all four shapes at once: the `source_files`
        // row, the texture-slot row, the `(source, source_sha256)` row, and the
        // map-keyed row. If a shape stops being recognized this count drops.
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
        // The fail-closed half. A future artifact that grows a new claim
        // surface must announce itself; the defect this closes is a walker that
        // silently certified 8% of a corpus.
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
        // `bytes` is present deliberately. The two non-content digests are
        // REMOVED from the walked document rather than exempted by key path, so
        // a `localisation_source` that grows a length field cannot fall into
        // the map-keyed claim shape and turn a build-input digest into a
        // "proof" for a path called `localisation_source`.
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

    #[test]
    fn an_artifact_without_a_self_digest_is_refused() {
        // The self-digest is what the gate independently recomputes over the
        // rest of the document. An artifact that carries none has nothing
        // binding the claims below to the bytes the container ships.
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
        assert!(error.contains("two different contents for a.txt"), "{error}");
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
        // The sweep used to look at object VALUES only, so a digest sitting in
        // an array as a plain string reached the wildcard arm and was silently
        // skipped — the exact shape the fail-closed rule exists to catch
        // (Codex review round 1, finding 3).
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
        // The map-keyed shape reads a path from OUTSIDE the object, so it only
        // applies when the key IS a content path. Without that guard any object
        // that grew `sha256` beside `bytes` would turn its parent's name into a
        // path and a build-input digest into a bogus content "proof".
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
        // Admission is lowercase-only, but the SWEEP must be case-blind or a
        // future claim surface written in uppercase hex would slip through the
        // rule that exists to catch the surfaces nobody anticipated (Codex
        // review round 3, finding 1).
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

    /// The map-keyed shape reads a path from OUTSIDE the object, so a key that
    /// merely LOOKS path-shaped — `build.v1`, `vendor/thing` — does become a
    /// claim. That is deliberate and it is FAIL-CLOSED, not a false pass: the
    /// claim then has to resolve through the engine's effective VFS and match
    /// its digest, and a name that is not a content path does not resolve, so
    /// the artifact is refused BY NAME.
    ///
    /// This test proves the second half against a real mounted VFS rather than
    /// asserting it in a comment (Codex review round 2, finding 4).
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

        // A real base-only effective VFS over a scratch install.
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
        // And a real path whose bytes disagree with the claim is refused for
        // the RIGHT reason, so the test above is not passing on "everything
        // fails".
        let real = check_claim(&vfs, "interface/real.gui", &scanned.claims["interface/real.gui"])
            .expect_err("a real file whose digest disagrees must not join");
        assert!(real.starts_with("sha256 "), "{real}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_array_element_never_inherits_its_arrays_key_as_a_path() {
        // The map-keyed shape is the one rule that reads a path from OUTSIDE
        // the object. Applied to array elements it would give every row of, say,
        // `flags` the logical path `flags`, and the join would then "prove" one
        // arbitrary file over and over.
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
            },
            ArtifactScan {
                role: "ship_definitions_static".into(),
                artifact_file_sha256: "a".repeat(64),
                claims: BTreeMap::new(),
                absent_claims: Vec::new(),
                localisation_source_sha256: Some("b".repeat(64)),
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
}
