//! Dual-order automatic attach (roadmap task #36, design of record
//! `extractor/documents/AUTO_ATTACH_DESIGN.md`).
//!
//! Both launch orders attach with zero user precision (memory
//! `product-session-semantics`): APP-FIRST (the watcher sees the game launch)
//! and GAME-FIRST (startup enumeration finds an already-running game). The
//! resident [`coordinator::Coordinator`] drives every detected
//! `(pid, creation_filetime)` identity through the attach state machine, keeps
//! the SINGLE-INJECTION LAW via the [`reducer::CandidateRegistry`] no-double-
//! inject guard, and journals durably for crash recovery.
//!
//! ## What this lane ships vs. what it depends on
//!
//! This module (the coordinator core + identity/journal/policy/reducer/state +
//! the Toolhelp probe + FS stager) is complete and unit-tested with fakes. The
//! production end-to-end path additionally needs the design §9 changes that are
//! OUTSIDE this lane's fence — a real WMI `Win32_ProcessStartTrace` event source
//! (needs a COM binding in `Cargo.toml`), the collector heartbeat/first-frame
//! proof protocol and pipe-listening barrier (`collector/src/lib.rs`,
//! `main.rs`), and the `--until-process-exit` worker mode. Until those land, the
//! real adapters here degrade honestly: the event source runs snapshot-only
//! (the design's bounded fallback), and the worker handle gates on child-process
//! liveness/exit rather than a heartbeat it cannot yet observe. The dossier
//! (`extractor/documents/dossiers/W12C_AUTOATTACH_IMPL.md`) tracks every such
//! deviation and the live-validation matrix.

pub mod coordinator;
pub mod duplex_worker;
pub mod identity;
pub mod journal;
pub mod policy;
pub mod proof;
pub mod reducer;
#[cfg(windows)]
pub mod singleton;
pub mod sources;
pub mod staging;
pub mod state;
#[cfg(windows)]
pub mod windows_combined_backend;

pub use coordinator::{Coordinator, CoordinatorConfig, CoordinatorDeps};
pub use identity::{ProcessIdentity, ProcessKey};
pub use state::CandidateState;

/// Options for the resident supervisor (design §1, §7). No correctness policy is
/// user-tunable; these are location/identity facts resolved at boot.
#[cfg(windows)]
pub struct SupervisorOptions {
    /// Packaged collector DLL to stage per attempt (resolved relative to the
    /// packaged app executable/manifest — never this repo; design §3, §7).
    pub source_dll: std::path::PathBuf,
    /// Root under which per-attempt staging directories are created.
    pub staging_root: std::path::PathBuf,
    /// Root under which per-attempt run/output directories are created.
    pub out_root: std::path::PathBuf,
    /// Path to the `auto_attach_journal.jsonl` (design §8).
    pub journal_path: std::path::PathBuf,
    /// `collector_record` executable to spawn as the per-game worker (defaults
    /// to the current executable — the supervisor and worker are the same binary
    /// in different modes, mirroring the dev launcher).
    pub worker_exe: std::path::PathBuf,
    /// Extra worker args (e.g. static-catalog paths, profile). The `--pid`,
    /// `--dll`, and `--out-dir` are supplied per attempt.
    pub worker_extra_args: Vec<String>,
    /// Poll cadence for the supervisor loop.
    pub poll_ms: u64,
}

/// Run the resident coordinator until `should_stop` returns true (design §1).
/// Wires the real Windows adapters. This is the product entry point invoked from
/// `main.rs` under `--auto-attach`.
#[cfg(windows)]
pub fn run_supervisor(
    opts: SupervisorOptions,
    should_stop: impl Fn() -> bool,
) -> Result<(), String> {
    use sources::wmi::WmiProcessEventSource;
    use sources::{SystemClock, ToolhelpProbe};
    use staging::FsPayloadStager;

    let (own_session, own_sid_hash) = ToolhelpProbe::current_session_and_sid_hash()?;
    // Acquire and retain before journal recovery, WMI subscription, or process
    // enumeration so two supervisors cannot reach independent registries.
    let _singleton = singleton::SupervisorMutex::acquire(own_session, &own_sid_hash)?;
    let instance_id = live::coordinator_instance_id();
    let clock = SystemClock::new();
    let now0 = <SystemClock as sources::Clock>::now_ms(&clock);

    let config = CoordinatorConfig {
        coordinator_instance_id: instance_id.clone(),
        out_root: opts.out_root.clone(),
        own_session_id: own_session,
        own_sid_hash,
    };
    let deps = CoordinatorDeps {
        clock: Box::new(SystemClock::new()),
        probe: Box::new(ToolhelpProbe::new()),
        events: Box::new(WmiProcessEventSource::new()),
        env: Box::new(live::LiveAttachEnvironment::new()),
        stager: Box::new(FsPayloadStager::new(
            opts.source_dll,
            opts.staging_root,
            now0,
        )),
        spawner: Box::new(live::ProcessCommandSpawner::new(
            opts.worker_exe,
            opts.worker_extra_args,
        )),
    };

    let mut coord = Coordinator::new(config, deps, opts.journal_path)?;
    // Reconcile any prior journal before touching live processes (design §8).
    coord.recover_from_journal()?;

    while !should_stop() {
        coord.step()?;
        std::thread::sleep(std::time::Duration::from_millis(opts.poll_ms));
    }
    Ok(())
}

/// Live Windows adapters that don't belong in `sources.rs` (they compose the
/// existing `modset` resolver and spawn the existing worker binary).
#[cfg(windows)]
mod live {
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, TryRecvError};

    use super::identity::ProcessIdentity;
    use super::sources::{
        AttachEnvironment, ModsetResolution, WorkerHandle, WorkerSpawner, WorkerSpec, WorkerStatus,
    };

    /// A per-boot coordinator instance id (design §8 `coordinator_instance_id`).
    pub fn coordinator_instance_id() -> String {
        use snapshot::sha256::{to_hex, Sha256};
        let mut h = Sha256::new();
        h.update(&std::process::id().to_le_bytes());
        if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            h.update(&t.as_nanos().to_le_bytes());
        }
        to_hex(&h.finalize())[..16].to_string()
    }

    // -- Visible-window + modset environment -----------------------------

    #[derive(Default)]
    pub struct LiveAttachEnvironment;

    impl LiveAttachEnvironment {
        pub fn new() -> Self {
            Self
        }
    }

    impl AttachEnvironment for LiveAttachEnvironment {
        fn has_visible_window(&self, pid: u32) -> bool {
            has_visible_window_for_pid(pid)
        }

        fn resolve_modset(&self, id: &ProcessIdentity) -> ModsetResolution {
            // Reuse the existing fail-closed resolver. NOTE (design §3, §9): the
            // resolver's user-dir CANDIDATE DISCOVERY still needs expansion
            // (command-line -userdir, FOLDERID_Documents, runtime OneDrive
            // roots) — a `modset.rs` change outside this lane's fence, tracked in
            // the dossier. Its PID-held-log corroboration and fail-closed
            // resolution are reused as-is.
            let report = crate::modset::resolve_modset(id.key.pid, None);
            ModsetResolution {
                resolved: report.resolved,
                reasons: report.errors,
            }
        }
    }

    struct WindowSearch {
        pid: u32,
        found: bool,
    }

    extern "system" fn enum_windows_proc(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
        unsafe {
            let search = &mut *(lparam as *mut WindowSearch);
            let mut owner_pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut owner_pid);
            if owner_pid == search.pid && IsWindowVisible(hwnd) != 0 {
                search.found = true;
                return 0; // stop enumeration
            }
            1 // continue
        }
    }

    fn has_visible_window_for_pid(pid: u32) -> bool {
        let mut search = WindowSearch { pid, found: false };
        unsafe {
            EnumWindows(enum_windows_proc, &mut search as *mut _ as isize);
        }
        search.found
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(
            proc: extern "system" fn(*mut std::ffi::c_void, isize) -> i32,
            lparam: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *mut std::ffi::c_void) -> i32;
    }

    // -- Child-process worker spawner ------------------------------------

    /// Spawns `collector_record --pid <target> --dll <staged> --out-dir <run>`
    /// as the per-game worker (design §5: reuse the existing hook injector; the
    /// coordinator never injects directly). This is the SAME interface the dev
    /// launcher uses (`Start-Hoi4ProductionSpeed5Capture.ps1`).
    pub struct ProcessCommandSpawner {
        exe: PathBuf,
        extra_args: Vec<String>,
    }

    impl ProcessCommandSpawner {
        pub fn new(exe: PathBuf, extra_args: Vec<String>) -> Self {
            Self { exe, extra_args }
        }
    }

    impl WorkerSpawner for ProcessCommandSpawner {
        fn spawn(&self, spec: &WorkerSpec) -> Result<Box<dyn WorkerHandle>, String> {
            std::fs::create_dir_all(&spec.out_dir).map_err(|e| format!("create out dir: {e}"))?;
            const RESERVED: &[&str] = &[
                "--pid",
                "--dll",
                "--out-dir",
                "--session-id",
                "--capture-id",
                "--stream-format",
            ];
            if let Some(arg) = self
                .extra_args
                .iter()
                .find(|arg| RESERVED.contains(&arg.as_str()))
            {
                return Err(format!(
                    "auto-attach worker arg {arg:?} would override coordinator authority"
                ));
            }
            let control_session = super::proof::decode_hex_16(&spec.payload.pipe_token)
                .map_err(|e| format!("invalid coordinator pipe token: {e}"))?;
            let expectation = super::proof::ProofExpectation {
                pid: spec.key.pid,
                creation_filetime: spec.key.creation_filetime.0,
                h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
                dll_sha256: super::proof::decode_hex_32(&spec.payload.dll_sha256)
                    .map_err(|e| format!("invalid staged DLL hash: {e}"))?,
                challenge: spec.control_challenge,
            };
            let mut cmd = Command::new(&self.exe);
            cmd.args(&self.extra_args)
                .arg("--pid")
                .arg(spec.key.pid.to_string())
                .arg("--dll")
                .arg(&spec.payload.dll_path)
                .arg("--out-dir")
                .arg(&spec.out_dir)
                .arg("--session-id")
                .arg(&spec.session_id)
                .arg("--capture-id")
                .arg(&spec.attempt_id)
                .arg("--stream-format")
                .arg("compact")
                .env("BATTLE_PLAN_AUTO_ATTACH_PIPE_TOKEN", &spec.payload.pipe_token)
                .env(
                    "BATTLE_PLAN_AUTO_ATTACH_EXPECTED_FILETIME",
                    spec.key.creation_filetime.0.to_string(),
                )
                .env(
                    "BATTLE_PLAN_AUTO_ATTACH_EXPECTED_DLL_SHA256",
                    &spec.payload.dll_sha256,
                )
                .env(
                    "BATTLE_PLAN_AUTO_ATTACH_CONTROL_CHALLENGE",
                    super::proof::encode_hex(&spec.control_challenge),
                )
                .env("BATTLE_PLAN_AUTO_ATTACH_ATTEMPT_ID", &spec.attempt_id)
                .env(
                    "BATTLE_PLAN_AUTO_ATTACH_SOURCE_DLL_SHA256",
                    &spec.payload.source_sha256,
                )
                .stdin(Stdio::null())
                .stdout(Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| format!("spawn worker: {e}"))?;
            let stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("spawned worker has no proof pipe".to_string());
                }
            };
            let (status_tx, status_rx) = mpsc::channel();
            if let Err(e) = std::thread::Builder::new()
                .name(format!("auto-attach-proof-{}", spec.key.pid))
                .spawn(move || {
                    let result = (|| -> Result<(), String> {
                        let mut channel = super::proof::recorder_channel(stdout, control_session)?;
                        let hello = channel.recv().map_err(|e| e.to_string())?;
                        if hello.kind != crate::control_transport::MessageKind::CollectorHello {
                            return Err(format!("expected CollectorHello, got {:?}", hello.kind));
                        }
                        super::proof::validate_collector_hello(&hello.payload, &expectation)?;
                        status_tx
                            .send(WorkerStatus::Attached)
                            .map_err(|_| "coordinator dropped proof receiver".to_string())?;
                        let proof = channel.recv().map_err(|e| e.to_string())?;
                        if proof.kind != crate::control_transport::MessageKind::HeartbeatProof {
                            return Err(format!("expected HeartbeatProof, got {:?}", proof.kind));
                        }
                        super::proof::validate_heartbeat_proof(&proof.payload, &expectation)?;
                        status_tx
                            .send(WorkerStatus::Recording {
                                last_confirmed_hour: None,
                            })
                            .map_err(|_| "coordinator dropped proof receiver".to_string())?;
                        Ok(())
                    })();
                    if let Err(reason) = result {
                        let _ = status_tx.send(WorkerStatus::Failed { reason });
                    }
                })
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("spawn proof reader: {e}"));
            }
            Ok(Box::new(ChildWorkerHandle {
                child,
                out_dir: spec.out_dir.clone(),
                stop_requested: false,
                status_rx,
                saw_attached: false,
                saw_recording: false,
                #[cfg(test)]
                test_reaped: None,
            }))
        }
    }

    /// Handle over a spawned worker child. HONEST LIMITATION (design §9, tracked
    /// in the dossier): without the collector heartbeat/first-frame proof
    /// protocol, this handle cannot observe residency or a valid session frame,
    /// so it gates on child liveness/exit only:
    /// - child alive        => `Handshaking` (never falsely claims Attached);
    /// - clean exit + replay => `Complete`;
    /// - clean exit, no replay, or non-zero exit => `Failed`.
    /// When the collector begins writing an attach-status sidecar (§9 work),
    /// map it here to unlock the full residency/recording ladder.
    struct ChildWorkerHandle {
        child: Child,
        out_dir: PathBuf,
        stop_requested: bool,
        status_rx: Receiver<WorkerStatus>,
        saw_attached: bool,
        saw_recording: bool,
        #[cfg(test)]
        test_reaped: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }

    impl WorkerHandle for ChildWorkerHandle {
        fn poll(&mut self) -> WorkerStatus {
            match self.status_rx.try_recv() {
                Ok(WorkerStatus::Attached) => {
                    self.saw_attached = true;
                    return WorkerStatus::Attached;
                }
                Ok(status @ WorkerStatus::Recording { .. }) => {
                    self.saw_recording = true;
                    return status;
                }
                Ok(status) => return status,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
            match self.child.try_wait() {
                Ok(None) => WorkerStatus::Handshaking,
                Ok(Some(status)) => {
                    if status.success() {
                        if !self.saw_attached || !self.saw_recording {
                            WorkerStatus::Failed {
                                reason: "worker exited without matched hello and complete first-frame proof"
                                    .into(),
                            }
                        } else if let Some(path) = find_verified_replay(&self.out_dir) {
                            WorkerStatus::Complete { replay_path: path }
                        } else {
                            WorkerStatus::Failed {
                                reason: "worker exited 0 but produced no replay artifact".into(),
                            }
                        }
                    } else {
                        WorkerStatus::Failed {
                            reason: format!("worker exited with {status}"),
                        }
                    }
                }
                Err(e) => WorkerStatus::Failed {
                    reason: format!("try_wait failed: {e}"),
                },
            }
        }

        fn request_stop(&mut self) {
            self.stop_requested = true;
            // Best-effort graceful stop is a §9 collector feature; today we can
            // only hard-stop the child if it outlives an explicit quit.
            let _ = self.child.kill();
        }
    }

    impl Drop for ChildWorkerHandle {
        fn drop(&mut self) {
            if !self.stop_requested {
                self.request_stop();
            }
            // Rust's `Child::drop` neither terminates nor waits. Reap here so a
            // coordinator error or normal supervisor shutdown cannot orphan the
            // injector/capture worker.
            let _waited = self.child.wait().is_ok();
            #[cfg(test)]
            if _waited {
                if let Some(reaped) = &self.test_reaped {
                    reaped.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    }

    fn find_verified_replay(out_dir: &std::path::Path) -> Option<String> {
        let rd = std::fs::read_dir(out_dir).ok()?;
        for entry in rd {
            let entry = entry.ok()?;
            let p = entry.path();
            let file_type = entry.file_type().ok()?;
            let metadata = entry.metadata().ok()?;
            if p.extension().and_then(|e| e.to_str()) == Some("hoi4replay")
                && file_type.is_file()
                && metadata.len() > 0
            {
                return Some(p.display().to_string());
            }
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn retained_artifact_must_be_nonempty_regular_replay_file() {
            let dir = std::env::temp_dir().join(format!(
                "auto_attach_artifact_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&dir).unwrap();
            let replay = dir.join("session.battleplan");
            std::fs::write(&replay, []).unwrap();
            assert!(find_verified_replay(&dir).is_none());
            std::fs::write(&replay, b"verified-worker-output").unwrap();
            assert_eq!(
                find_verified_replay(&dir).as_deref(),
                Some(replay.to_string_lossy().as_ref())
            );
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        #[ignore = "subprocess helper for ChildWorkerHandle drop test"]
        fn child_worker_drop_wait_helper() {
            let ready = std::env::var_os("BATTLE_PLAN_TEST_CHILD_READY").or_else(|| std::env::var_os("HOI4_TEST_CHILD_READY")).expect("ready path");
            std::fs::write(ready, b"ready").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(30));
        }

        #[test]
        fn child_worker_drop_stops_and_waits_live_process() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;

            let ready = std::env::temp_dir().join(format!(
                "auto_attach_child_ready_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "auto_attach::live::tests::child_worker_drop_wait_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("BATTLE_PLAN_TEST_CHILD_READY", &ready)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !ready.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(ready.exists(), "helper child did not become live");

            let (_tx, status_rx) = mpsc::channel();
            let reaped = Arc::new(AtomicBool::new(false));
            let handle = ChildWorkerHandle {
                child,
                out_dir: std::env::temp_dir(),
                stop_requested: false,
                status_rx,
                saw_attached: false,
                saw_recording: false,
                test_reaped: Some(reaped.clone()),
            };
            drop(handle);
            assert!(
                reaped.load(Ordering::SeqCst),
                "drop did not wait/reap child"
            );
            let _ = std::fs::remove_file(ready);
        }
    }
}
