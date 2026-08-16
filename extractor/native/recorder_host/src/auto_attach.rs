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
//! The coordinator, WMI+reconciliation discovery, identity/journal/policy,
//! restrictive paired H4CR/H4CT backend, heartbeat/first-frame authority, and
//! installed-payload resolution are composed in the live product. The child
//! publishes its attempt-bound status ladder to a recorder-created durable file;
//! H4CT remains the authority for every forwarded heartbeat. Explicit stop owns
//! the authenticated stop event and bounded H4CR drain/finalization path.

pub mod coordinator;
pub mod duplex_worker;
pub mod identity;
pub mod journal;
pub mod policy;
pub mod product_runtime;
pub mod proof;
pub mod reducer;
#[cfg(windows)]
pub mod singleton;
pub mod sources;
pub mod staging;
pub mod state;
#[cfg(windows)]
pub mod windows_combined_backend;

pub use coordinator::{Coordinator, CoordinatorConfig, CoordinatorDeps, SupervisorEvent};
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
    /// Retain the legacy visible-window readiness proof. The packaged headless
    /// product disables this and relies on the exact same-user process identity
    /// re-probe so injection precedes one-shot UI setup.
    pub require_visible_window: bool,
    /// Poll cadence for the supervisor loop.
    pub poll_ms: u64,
}

/// Run the resident coordinator until `should_stop` returns true (design §1).
/// Wires the real Windows discovery/identity adapters and process-isolated
/// worker invoked from `main.rs` under `--auto-attach`. Frozen activation keeps
/// the explicit Legacy path; the jointly gated product path selects the
/// duplex/combined backend and authenticated cooperative stop event.
#[cfg(windows)]
pub fn run_supervisor(
    opts: SupervisorOptions,
    should_stop: impl Fn() -> bool,
) -> Result<(), String> {
    run_supervisor_with_events(opts, should_stop, |_| {})
}

/// Run the resident coordinator and synchronously publish durable lifecycle
/// events to a product-facing observer. The callback is informational only.
#[cfg(windows)]
pub fn run_supervisor_with_events(
    opts: SupervisorOptions,
    should_stop: impl Fn() -> bool,
    mut on_event: impl FnMut(SupervisorEvent),
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
        require_visible_window: opts.require_visible_window,
    };
    let deps = CoordinatorDeps {
        clock: Box::new(SystemClock::new()),
        probe: Box::new(ToolhelpProbe::new()),
        events: Box::new(WmiProcessEventSource::new()),
        env: Box::new(live::LiveAttachEnvironment::new(opts.out_root.clone())),
        stager: Box::new(FsPayloadStager::new(
            opts.source_dll,
            opts.staging_root,
            now0,
        )),
        spawner: Box::new(live::ProductProcessCommandSpawner::new(
            opts.worker_exe,
            opts.worker_extra_args,
        )),
    };

    let mut coord = Coordinator::new(config, deps, opts.journal_path)?;
    // Reconcile any prior journal before touching live processes (design §8).
    coord.recover_from_journal()?;

    while !should_stop() {
        coord.step()?;
        for event in coord.drain_supervisor_events() {
            on_event(event);
        }
        std::thread::sleep(std::time::Duration::from_millis(opts.poll_ms));
    }
    coord.begin_explicit_stop()?;
    for event in coord.drain_supervisor_events() {
        on_event(event);
    }
    while !coord.drive_shutdown()? {
        for event in coord.drain_supervisor_events() {
            on_event(event);
        }
        std::thread::sleep(std::time::Duration::from_millis(opts.poll_ms));
    }
    for event in coord.drain_supervisor_events() {
        on_event(event);
    }
    Ok(())
}

/// Live Windows adapters that don't belong in `sources.rs` (they compose the
/// existing `modset` resolver and spawn the existing worker binary).
#[cfg(windows)]
mod live {
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, TryRecvError};
    use std::sync::Arc;
    use std::ptr;

    use std::os::windows::io::AsRawHandle;
    use std::os::windows::ffi::OsStrExt;

    use super::identity::{ProcessIdentity, ProcessKey};
    use super::sources::{
        AttachEnvironment, ModsetResolution, ProcessProbe, ToolhelpProbe, WorkerHandle,
        WorkerSpawner, WorkerSpec, WorkerStatus,
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

    pub struct LiveAttachEnvironment {
        product_root: PathBuf,
    }

    impl LiveAttachEnvironment {
        pub fn new(product_root: PathBuf) -> Self {
            Self { product_root }
        }
    }

    impl AttachEnvironment for LiveAttachEnvironment {
        fn has_visible_window(&self, key: ProcessKey) -> bool {
            has_visible_window_for_process(key)
        }

        fn resolve_modset(&self, id: &ProcessIdentity) -> ModsetResolution {
            // Reuse the existing fail-closed resolver. NOTE (design §3, §9): the
            // resolver's user-dir CANDIDATE DISCOVERY still needs expansion
            // (command-line -userdir, FOLDERID_Documents, runtime OneDrive
            // roots) — a `modset.rs` change outside this lane's fence, tracked in
            // the dossier. Its PID-held-log corroboration and fail-closed
            // resolution are reused as-is.
            let probe = ToolhelpProbe::new();
            let before = match probe.identify(id.key.pid) {
                Ok(identity) if identity == *id => identity,
                Ok(_) => {
                    return ModsetResolution {
                        resolved: false,
                        reasons: vec![
                            "process identity changed before modset resolution".to_string(),
                        ],
                    }
                }
                Err(error) => {
                    return ModsetResolution {
                        resolved: false,
                        reasons: vec![format!(
                            "process identity unavailable before modset resolution: {error}"
                        )],
                    }
                }
            };
            let mut report = crate::modset::resolve_modset(before.key.pid, None);
            let settings_authority = crate::modset::same_process_settings_authority_path(
                &self.product_root,
                before.key.pid,
                before.key.creation_filetime.0,
            );
            if let Err(error) = crate::modset::apply_same_process_settings_authority(
                &mut report,
                &settings_authority,
                before.key.pid,
                before.key.creation_filetime.0,
            ) {
                // The authority is the SETTINGS half; a failure to publish or
                // recover it degrades the settings binding, and says nothing
                // about the executable, mods or DLC.
                report.settings.fail_public(
                    crate::modset::codes::SETTINGS_UNREADABLE,
                    format!("same-process settings authority unavailable: {error}"),
                );
            }
            match probe.identify(before.key.pid) {
                Ok(after) if after == before => {}
                Ok(_) => {
                    return ModsetResolution {
                        resolved: false,
                        reasons: vec![
                            "process identity changed during modset resolution".to_string(),
                        ],
                    }
                }
                Err(error) => {
                    return ModsetResolution {
                        resolved: false,
                        reasons: vec![format!(
                            "process identity unavailable after modset resolution: {error}"
                        )],
                    }
                }
            }
            ModsetResolution {
                // Auto-attach arms a capture against a game it did not launch,
                // so it asks the question it can act on: is the CONTENT bound.
                // The settings binding travels in the manifest with its own
                // authority; it is not a reason to refuse an attach.
                resolved: report.content_resolved,
                reasons: report
                    .errors
                    .iter()
                    .map(crate::modset::ModsetError::to_string)
                    .collect(),
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

    fn has_visible_window_for_process(key: ProcessKey) -> bool {
        let probe = ToolhelpProbe::new();
        if !matches!(probe.identify(key.pid), Ok(identity) if identity.key == key) {
            return false;
        }
        let mut search = WindowSearch {
            pid: key.pid,
            found: false,
        };
        unsafe {
            EnumWindows(enum_windows_proc, &mut search as *mut _ as isize);
        }
        search.found
            && matches!(probe.identify(key.pid), Ok(identity) if identity.key == key)
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

    // -- Passive worker ownership + cooperative stop --------------------

    type Handle = *mut std::ffi::c_void;
    type Bool = i32;
    type Dword = u32;

    const ERROR_ALREADY_EXISTS: Dword = 183;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *mut std::ffi::c_void,
            initial_owner: Bool,
            name: *const u16,
        ) -> Handle;
        fn CreateEventW(
            attributes: *mut std::ffi::c_void,
            manual_reset: Bool,
            initial_state: Bool,
            name: *const u16,
        ) -> Handle;
        fn SetEvent(event: Handle) -> Bool;
        fn GetProcessTimes(
            process: Handle,
            creation_time: *mut u64,
            exit_time: *mut u64,
            kernel_time: *mut u64,
            user_time: *mut u64,
        ) -> Bool;
        fn GetLastError() -> Dword;
        fn CloseHandle(handle: Handle) -> Bool;
    }

    /// A recorder-owned, handle-lifetime lease for one target identity.
    ///
    /// The named object is never applied to either the game process or the
    /// recorder worker. It provides passive duplicate-worker exclusion without
    /// termination, quota, suspension, or any other process-control capability.
    struct WorkerOwnershipLease(Handle);

    unsafe impl Send for WorkerOwnershipLease {}
    unsafe impl Sync for WorkerOwnershipLease {}

    impl WorkerOwnershipLease {
        fn acquire(spec: &WorkerSpec) -> Result<Self, String> {
            let name = format!(
                r"Local\battle-plan_worker_{}_{}",
                spec.key.pid, spec.key.creation_filetime.0
            );
            let mut wide: Vec<u16> = std::ffi::OsStr::new(&name).encode_wide().collect();
            wide.push(0);
            let handle = unsafe { CreateMutexW(ptr::null_mut(), 0, wide.as_ptr()) };
            if handle.is_null() {
                return Err(format!(
                    "create passive worker ownership lease failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                return Err(format!(
                    "a recorder worker already owns target identity ({}, {})",
                    spec.key.pid, spec.key.creation_filetime.0
                ));
            }
            Ok(Self(handle))
        }
    }

    impl Drop for WorkerOwnershipLease {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    struct SupervisorStopEvent {
        handle: Handle,
        name: String,
    }

    unsafe impl Send for SupervisorStopEvent {}

    impl SupervisorStopEvent {
        fn create(spec: &WorkerSpec) -> Result<Self, String> {
            // `pipe_token` is a canonical 128-bit CSPRNG secret already bound
            // to this attempt. Reusing it in a distinct Local object namespace
            // adds no second authority source and keeps the raw name out of the
            // journal.
            super::proof::decode_hex_16(&spec.payload.pipe_token)
                .map_err(|error| format!("invalid stop-event token: {error}"))?;
            let name = format!(
                r"Local\battle-plan_stop_{}_{}",
                spec.key.pid, spec.payload.pipe_token
            );
            let mut wide: Vec<u16> = std::ffi::OsStr::new(&name).encode_wide().collect();
            wide.push(0);
            let handle = unsafe { CreateEventW(ptr::null_mut(), 1, 0, wide.as_ptr()) };
            if handle.is_null() {
                return Err(format!(
                    "create cooperative stop event failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                return Err("cooperative stop event already existed".to_string());
            }
            Ok(Self { handle, name })
        }

        fn signal(&self) -> Result<(), String> {
            if unsafe { SetEvent(self.handle) } == 0 {
                Err(format!(
                    "signal cooperative stop event failed: {}",
                    std::io::Error::last_os_error()
                ))
            } else {
                Ok(())
            }
        }
    }

    impl Drop for SupervisorStopEvent {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe { CloseHandle(self.handle) };
            }
        }
    }

    fn retain_worker_until_exit(
        mut child: Child,
        ownership_lease: Arc<WorkerOwnershipLease>,
        cooperative_stop_event: Option<SupervisorStopEvent>,
        proof_stop: Arc<AtomicBool>,
        proof_join: Option<std::thread::JoinHandle<()>>,
    ) {
        let pid = child.id();
        let guardian_lease = ownership_lease.clone();
        if std::thread::Builder::new()
            .name(format!("auto-attach-passive-reaper-{pid}"))
            .spawn(move || {
                let _ownership_lease = guardian_lease;
                let _cooperative_stop_event = cooperative_stop_event;
                let _ = child.wait();
                proof_stop.store(true, Ordering::Release);
                if let Some(join) = proof_join {
                    let _ = join.join();
                }
            })
            .is_err()
        {
            // Thread creation failure must not make a still-running worker
            // appear unowned. Retain the passive lease until recorder-process
            // exit; the OS closes it without affecting either process.
            std::mem::forget(ownership_lease);
        }
    }

    // -- Child-process worker spawner ------------------------------------

    /// Spawns `collector_record --pid <target> --dll <staged> --out-dir <run>`
    /// as the per-game worker (design §5: reuse the existing hook injector; the
    /// coordinator never injects directly). This is the SAME interface the dev
    /// launcher uses (`Start-Hoi4ProductionSpeed5Capture.ps1`).
    /// Process-isolated worker adapter. A mintable product permit selects the child
    /// recorder's combined H4CR/H4CT backend plus authenticated stop event.
    pub struct ProductProcessCommandSpawner {
        exe: PathBuf,
        extra_args: Vec<String>,
    }

    impl ProductProcessCommandSpawner {
        pub fn new(exe: PathBuf, extra_args: Vec<String>) -> Self {
            Self { exe, extra_args }
        }
    }

    impl WorkerSpawner for ProductProcessCommandSpawner {
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
            let ownership_lease = Arc::new(WorkerOwnershipLease::acquire(spec)?);
            // The durable parent/child proof channel and live H4CT transport
            // coexist inside the worker process. Session capabilities are
            // process-global and single-claim, so the proof channel uses the
            // independent control challenge while H4CT exclusively owns the
            // pipe token.
            let control_session = spec.control_challenge;
            let proof_session_id =
                crate::control_transport::SessionId::from_bytes(control_session)
                    .map_err(|error| format!("invalid proof session id: {error}"))?;
            let expectation = super::proof::ProofExpectation {
                pid: spec.key.pid,
                creation_filetime: spec.key.creation_filetime.0,
                h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
                dll_sha256: super::proof::decode_hex_32(&spec.payload.dll_sha256)
                    .map_err(|e| format!("invalid staged DLL hash: {e}"))?,
                challenge: spec.control_challenge,
            };
            let cooperative_stop_event =
                if super::duplex_worker::auto_attach_identity_v1_enabled() {
                    Some(SupervisorStopEvent::create(spec)?)
                } else {
                    None
                };
            let proof_path = spec
                .out_dir
                .join(super::proof::WORKER_PROOF_FILE_NAME);
            let proof_file = super::proof::create_durable_proof_file(proof_path.clone())?;
            let proof_transcript_path = proof_path.clone();
            let stderr_path = spec.out_dir.join("worker_stderr.log");
            let worker_stderr = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stderr_path)
                .map_err(|error| format!("create worker stderr trace: {error}"))?;
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
                    "BATTLE_PLAN_AUTO_ATTACH_MODSET_AUTHORITY",
                    crate::modset::same_process_settings_authority_path(
                        spec.out_dir
                            .parent()
                            .ok_or("auto-attach run directory has no product root")?,
                        spec.key.pid,
                        spec.key.creation_filetime.0,
                    ),
                )
                .env(
                    "BATTLE_PLAN_AUTO_ATTACH_REARMED_TARGET_LEASE",
                    if spec.payload.rearmed_after_proven_unload {
                        "1"
                    } else {
                        "0"
                    },
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
                .env(super::proof::WORKER_PROOF_PATH_ENV, &proof_path);
            if let Some(stop_event) = cooperative_stop_event.as_ref() {
                cmd.env("BATTLE_PLAN_AUTO_ATTACH_STOP_EVENT", &stop_event.name);
            }
            cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(worker_stderr));
            let child = cmd.spawn().map_err(|e| format!("spawn worker: {e}"))?;
            let (status_tx, status_rx) = mpsc::channel();
            let proof_stop = Arc::new(AtomicBool::new(false));
            let proof_thread_stop = proof_stop.clone();
            let proof_join = match std::thread::Builder::new()
                .name(format!("auto-attach-proof-{}", spec.key.pid))
                .spawn(move || {
                    let result = (|| -> Result<(), String> {
                        let tail = super::proof::DurableProofTail::new(
                            proof_file,
                            proof_thread_stop.clone(),
                        );
                        let mut channel = super::proof::recorder_channel(tail, control_session)?;
                        let hello = channel.recv().map_err(|e| e.to_string())?;
                        if hello.kind != crate::control_transport::MessageKind::CollectorHello {
                            return Err(format!("expected CollectorHello, got {:?}", hello.kind));
                        }
                        super::proof::validate_collector_hello(&hello.payload, &expectation)?;
                        status_tx
                            .send(WorkerStatus::Attached)
                            .map_err(|_| "coordinator dropped proof receiver".to_string())?;
                        let mut last_sequence = None;
                        loop {
                            let proof = match channel.recv() {
                                Ok(proof) => proof,
                                Err(crate::control_transport::ControlError::Truncated)
                                    if last_sequence.is_some()
                                        && proof_thread_stop.load(Ordering::Acquire)
                                        && validate_complete_proof_transcript(
                                            &proof_transcript_path,
                                            proof_session_id,
                                        )
                                        .is_ok() =>
                                {
                                    break
                                }
                                Err(error) => return Err(error.to_string()),
                            };
                            if proof.kind != crate::control_transport::MessageKind::HeartbeatProof {
                                return Err(format!(
                                    "expected HeartbeatProof, got {:?}",
                                    proof.kind
                                ));
                            }
                            let sequence = super::proof::validate_heartbeat_proof(
                                &proof.payload,
                                &expectation,
                            )?;
                            if last_sequence.is_some_and(|previous| sequence < previous) {
                                return Err("worker heartbeat sequence regressed".to_string());
                            }
                            last_sequence = Some(sequence);
                            status_tx
                                .send(WorkerStatus::Recording {
                                    last_confirmed_hour: None,
                                })
                                .map_err(|_| "coordinator dropped proof receiver".to_string())?;
                        }
                        Ok(())
                    })();
                    if let Err(reason) = result {
                        let _ = status_tx.send(WorkerStatus::Failed { reason });
                    }
                })
            {
                Ok(join) => join,
                Err(e) => {
                    if let Some(stop_event) = cooperative_stop_event.as_ref() {
                        let _ = stop_event.signal();
                    }
                    retain_worker_until_exit(
                        child,
                        ownership_lease,
                        cooperative_stop_event,
                        Arc::new(AtomicBool::new(true)),
                        None,
                    );
                    return Err(format!("spawn proof reader: {e}"));
                }
            };
            Ok(Box::new(ChildWorkerHandle {
                child: Some(child),
                out_dir: spec.out_dir.clone(),
                stderr_path,
                stop_requested: false,
                shutdown_failure: None,
                ownership_lease: Some(ownership_lease),
                cooperative_stop_event,
                cooperative_stop_started: None,
                finalization_progress: None,
                finalization_last_progress_at: None,
                status_rx,
                proof_stop,
                proof_join: Some(proof_join),
                saw_attached: false,
                saw_recording: false,
            }))
        }
    }

    /// Handle over the process-isolated recorder worker. A retained, durable
    /// per-attempt file carries hello/first-world/recurring-heartbeat proofs;
    /// child exit is
    /// accepted as Complete only when that ladder succeeded and recorder-main's
    /// fold/pack/verify pipeline left a replay artifact.
    struct ChildWorkerHandle {
        child: Option<Child>,
        out_dir: PathBuf,
        stderr_path: PathBuf,
        stop_requested: bool,
        /// Set when cooperative finalization cannot be proven. The supervisor
        /// reports failure but never terminates the recorder worker.
        shutdown_failure: Option<String>,
        /// Recorder-owned duplicate-worker exclusion. If a worker outlives its
        /// handle, a passive reaper retains this lease until natural exit.
        ownership_lease: Option<Arc<WorkerOwnershipLease>>,
        cooperative_stop_event: Option<SupervisorStopEvent>,
        cooperative_stop_started: Option<std::time::Instant>,
        finalization_progress: Option<FinalizationProgress>,
        finalization_last_progress_at: Option<std::time::Instant>,
        status_rx: Receiver<WorkerStatus>,
        proof_stop: Arc<AtomicBool>,
        proof_join: Option<std::thread::JoinHandle<()>>,
        saw_attached: bool,
        saw_recording: bool,
    }

    impl WorkerHandle for ChildWorkerHandle {
        fn poll(&mut self) -> WorkerStatus {
            let now = std::time::Instant::now();
            self.refresh_finalization_progress(now);
            if self.finalization_deadman_expired(now) && self.shutdown_failure.is_none() {
                self.shutdown_failure = Some(
                    "worker finalization stopped making bounded progress; recorder left it running"
                        .to_string(),
                );
            }
            if let Some(reason) = self.shutdown_failure.as_ref() {
                return WorkerStatus::Failed {
                    reason: reason.clone(),
                };
            }
            match self.status_rx.try_recv() {
                Ok(WorkerStatus::Attached) => {
                    self.saw_attached = true;
                    return WorkerStatus::Attached;
                }
                Ok(status @ WorkerStatus::Recording { .. }) => {
                    self.saw_recording = true;
                    return status;
                }
                Ok(WorkerStatus::Failed { reason }) => {
                    if let Ok(Some(status)) =
                        self.child.as_mut().expect("worker child retained").try_wait()
                    {
                        if !status.success() {
                            return self.worker_exit_failure(status);
                        }
                    }
                    return WorkerStatus::Failed { reason };
                }
                Ok(status) => return status,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
            match self
                .child
                .as_mut()
                .expect("worker child retained")
                .try_wait()
            {
                Ok(None) if self.cooperative_stop_started.is_some() => WorkerStatus::Finalizing,
                Ok(None) if self.saw_recording => WorkerStatus::Recording {
                    last_confirmed_hour: None,
                },
                Ok(None) => WorkerStatus::Handshaking,
                Ok(Some(status)) => {
                    self.finish_proof_reader();
                    let mut proof_failure = None;
                    while let Ok(proof_status) = self.status_rx.try_recv() {
                        match proof_status {
                            WorkerStatus::Attached => self.saw_attached = true,
                            WorkerStatus::Recording { .. } => self.saw_recording = true,
                            WorkerStatus::Failed { reason } => proof_failure = Some(reason),
                            _ => {}
                        }
                    }
                    if !status.success() {
                        self.worker_exit_failure(status)
                    } else if let Some(reason) = proof_failure {
                        WorkerStatus::Failed { reason }
                    } else {
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
                    }
                }
                Err(e) => WorkerStatus::Failed {
                    reason: format!("try_wait failed: {e}"),
                },
            }
        }

        fn request_stop(&mut self) {
            if self.stop_requested {
                return;
            }
            self.stop_requested = true;
            if let Some(stop_event) = self.cooperative_stop_event.as_ref() {
                match stop_event.signal() {
                    Ok(()) => {
                        let now = std::time::Instant::now();
                        self.cooperative_stop_started = Some(now);
                        self.finalization_progress = Some(observe_finalization_progress(
                            self.child.as_ref().expect("worker child retained"),
                            &self.out_dir,
                        ));
                        self.finalization_last_progress_at = Some(now);
                        return;
                    }
                    Err(error) => {
                        self.shutdown_failure = Some(format!(
                            "cooperative stop signal failed; recorder left worker running: {error}"
                        ));
                        return;
                    }
                }
            }
            self.shutdown_failure = Some(
                "worker has no cooperative stop channel; recorder left it running".to_string(),
            );
        }
    }

    impl Drop for ChildWorkerHandle {
        fn drop(&mut self) {
            if !self.stop_requested {
                self.request_stop();
            }
            let exited = self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok())
                .flatten()
                .is_some();
            if exited {
                self.finish_proof_reader();
                self.child.take();
                self.ownership_lease.take();
                self.cooperative_stop_event.take();
                return;
            }

            // Dropping a Rust `Child` detaches without terminating it. Move the
            // live worker, its cooperative-stop event, proof reader, and passive
            // ownership lease to a recorder-owned reaper. The handle drop stays
            // bounded; duplicate-worker exclusion remains until natural exit.
            let child = self.child.take().expect("worker child retained");
            let ownership_lease = self
                .ownership_lease
                .take()
                .expect("worker ownership lease retained");
            let cooperative_stop_event = self.cooperative_stop_event.take();
            let proof_stop = self.proof_stop.clone();
            let proof_join = self.proof_join.take();
            retain_worker_until_exit(
                child,
                ownership_lease,
                cooperative_stop_event,
                proof_stop,
                proof_join,
            );
        }
    }

    impl ChildWorkerHandle {
        fn refresh_finalization_progress(&mut self, now: std::time::Instant) {
            if self.cooperative_stop_started.is_none() {
                return;
            }
            let observed = observe_finalization_progress(
                self.child.as_ref().expect("worker child retained"),
                &self.out_dir,
            );
            if self.finalization_progress != Some(observed) {
                self.finalization_progress = Some(observed);
                self.finalization_last_progress_at = Some(now);
            }
        }

        fn finalization_deadman_expired(&self, now: std::time::Instant) -> bool {
            let Some(started) = self.cooperative_stop_started else {
                return false;
            };
            let last_progress = self.finalization_last_progress_at.unwrap_or(started);
            finalization_deadman_expired(started, last_progress, now)
        }

        fn worker_exit_failure(&self, status: std::process::ExitStatus) -> WorkerStatus {
            const MAX_STDERR_TAIL_BYTES: usize = 8 * 1024;
            let detail = std::fs::read(&self.stderr_path).ok().and_then(|bytes| {
                let start = bytes.len().saturating_sub(MAX_STDERR_TAIL_BYTES);
                String::from_utf8_lossy(&bytes[start..])
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .map(|line| line.trim().to_string())
            });
            WorkerStatus::Failed {
                reason: detail.map_or_else(
                    || format!("worker exited with {status}"),
                    |detail| format!("worker exited with {status}: {detail}"),
                ),
            }
        }

        fn finish_proof_reader(&mut self) {
            self.proof_stop.store(true, Ordering::Release);
            if let Some(join) = self.proof_join.take() {
                let _ = join.join();
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct FinalizationProgress {
        cpu_time_100ns: u64,
        file_count: u64,
        total_file_bytes: u64,
    }

    fn observe_finalization_progress(
        child: &Child,
        out_dir: &std::path::Path,
    ) -> FinalizationProgress {
        let mut creation = 0u64;
        let mut exit = 0u64;
        let mut kernel = 0u64;
        let mut user = 0u64;
        let cpu_time_100ns = if unsafe {
            GetProcessTimes(
                child.as_raw_handle() as Handle,
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        } != 0
        {
            kernel.saturating_add(user)
        } else {
            0
        };

        let mut progress = FinalizationProgress {
            cpu_time_100ns,
            ..FinalizationProgress::default()
        };
        let mut pending = vec![out_dir.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    progress.file_count = progress.file_count.saturating_add(1);
                    if let Ok(metadata) = entry.metadata() {
                        progress.total_file_bytes =
                            progress.total_file_bytes.saturating_add(metadata.len());
                    }
                }
            }
        }
        progress
    }

    fn finalization_deadman_expired(
        started: std::time::Instant,
        last_progress: std::time::Instant,
        now: std::time::Instant,
    ) -> bool {
        now.saturating_duration_since(started)
            >= std::time::Duration::from_millis(
                super::policy::GRACEFUL_STOP_DEADLINE_MS,
            )
            || now.saturating_duration_since(last_progress)
                >= std::time::Duration::from_millis(
                    super::policy::GRACEFUL_STOP_STALL_DEADLINE_MS,
                )
    }

    /// A regular-file tail reports `UnexpectedEof` both at a clean frame
    /// boundary and after a torn append. The control codec deliberately maps
    /// both to `Truncated`, so child-exit handling must authenticate the exact
    /// durable transcript before treating the former as normal completion.
    fn validate_complete_proof_transcript(
        path: &std::path::Path,
        session_id: crate::control_transport::SessionId,
    ) -> Result<(), String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("read durable proof transcript failed: {error}"))?;
        if bytes.is_empty() {
            return Err("durable proof transcript is empty".to_string());
        }
        let mut cursor = 0usize;
        let mut sequence = 0u64;
        while cursor < bytes.len() {
            let remaining = bytes.len() - cursor;
            if remaining < crate::control_transport::CONTROL_HEADER_LEN {
                return Err(format!(
                    "durable proof transcript has {remaining} trailing header byte(s)"
                ));
            }
            let payload_len = u32::from_le_bytes(
                bytes[cursor + 12..cursor + 16]
                    .try_into()
                    .expect("fixed four-byte payload length"),
            ) as usize;
            let frame_len = crate::control_transport::CONTROL_HEADER_LEN
                .checked_add(payload_len)
                .ok_or("durable proof frame length overflow")?;
            let end = cursor
                .checked_add(frame_len)
                .ok_or("durable proof transcript cursor overflow")?;
            if end > bytes.len() {
                return Err(format!(
                    "durable proof transcript frame {sequence} is partial: needs {frame_len} byte(s), has {remaining}"
                ));
            }
            crate::control_transport::decode_frame(
                &bytes[cursor..end],
                session_id,
                sequence,
            )
            .map_err(|error| {
                format!("durable proof transcript frame {sequence} is invalid: {error}")
            })?;
            cursor = end;
            sequence = sequence
                .checked_add(1)
                .ok_or("durable proof sequence overflow")?;
        }
        Ok(())
    }

    fn find_verified_replay(out_dir: &std::path::Path) -> Option<String> {
        let package_set =
            out_dir.join(crate::duplex_session::rollover_fs::PACKAGE_SET_DESCRIPTOR_FILE);
        if std::fs::symlink_metadata(&package_set)
            .ok()
            .is_some_and(|metadata| {
                metadata.file_type().is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() > 0
            })
        {
            return Some(package_set.display().to_string());
        }
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
            let package_set =
                dir.join(crate::duplex_session::rollover_fs::PACKAGE_SET_DESCRIPTOR_FILE);
            std::fs::write(&package_set, b"{\"schema\":\"test\"}").unwrap();
            assert_eq!(
                find_verified_replay(&dir).as_deref(),
                Some(package_set.to_string_lossy().as_ref())
            );
            portable_test_fs_retry::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn finalization_progress_resets_stall_deadman_but_not_absolute_bound() {
            let started = std::time::Instant::now();
            let almost_stalled = started
                + std::time::Duration::from_millis(
                    super::super::policy::GRACEFUL_STOP_STALL_DEADLINE_MS - 1,
                );
            assert!(!finalization_deadman_expired(
                started,
                started,
                almost_stalled
            ));

            let progress = almost_stalled;
            let after_original_stall = started
                + std::time::Duration::from_millis(
                    super::super::policy::GRACEFUL_STOP_STALL_DEADLINE_MS + 1,
                );
            assert!(!finalization_deadman_expired(
                started,
                progress,
                after_original_stall
            ));

            let absolute = started
                + std::time::Duration::from_millis(
                    super::super::policy::GRACEFUL_STOP_DEADLINE_MS,
                );
            assert!(finalization_deadman_expired(
                started,
                absolute,
                absolute
            ));
        }

        #[test]
        fn proof_transcript_clean_eof_is_distinct_from_a_torn_frame() {
            let session =
                crate::control_transport::SessionId::from_bytes([0xA5; 16]).unwrap();
            let mut bytes = crate::control_transport::encode_frame(
                session,
                0,
                crate::control_transport::MessageKind::CollectorHello,
                b"hello",
            )
            .unwrap();
            bytes.extend_from_slice(
                &crate::control_transport::encode_frame(
                    session,
                    1,
                    crate::control_transport::MessageKind::HeartbeatProof,
                    b"proof",
                )
                .unwrap(),
            );
            let path = std::env::temp_dir().join(format!(
                "auto_attach_proof_transcript_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::write(&path, &bytes).unwrap();
            assert!(validate_complete_proof_transcript(&path, session).is_ok());
            std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
            assert!(validate_complete_proof_transcript(&path, session).is_err());
            portable_test_fs_retry::remove_file(path).unwrap();
        }

    }
}
