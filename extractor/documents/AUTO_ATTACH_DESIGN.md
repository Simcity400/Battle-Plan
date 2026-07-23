# Dual-order automatic attach design

**Status (corrected 2026-07-22):** the auto-attach product code is merged. An ordinary no-argument
launch of the packaged recorder opens its normal desktop window and owns the resident supervisor;
`--auto-attach` remains the support/headless entry. The product has no tray process or service.
Closing the window or pressing **Stop and finish recordings** enters coordinator-owned graceful
shutdown. Duplex activation remains separately governed by P2.

**Owner decision:** 2026-07-12. This implements pillar 1 of
`PRODUCT_SESSION_SEMANTICS.md`: app-first and game-first must both attach without a
PID, path, or save setting (`PRODUCT_SESSION_SEMANTICS.md:16-26`). It does not claim
campaign identity or cross-session stitching exists; those remain separate
prerequisites (`PRODUCT_SESSION_SEMANTICS.md:34-52`).

## 1. Decision

Ship one resident **auto-attach coordinator**:

1. Acquire a per-Windows-session singleton, open a durable journal, and establish
   WMI process-start/process-stop subscriptions.
2. After subscribing, immediately enumerate current processes. This is game-first
   and closes the subscribe/startup race.
3. Treat WMI as the low-latency signal, not sole truth. Reconcile from a process
   snapshot every 30 seconds while WMI is healthy. If WMI fails, scan every 2
   seconds until it recovers. These are policy constants, not rig-fitted claims.
4. Key candidates by `(pid, process_creation_filetime)`, prove same-user,
   same-session `hoi4.exe`, and resolve executable/build/user-dir/modset evidence.
5. Wait through startup until the process has an injectable visible window and all
   identity evidence resolves. Do not time out while it lives; show the blocker.
6. Stage a fresh collector DLL directory for every attempt, prove the pipe is
   listening, then reuse the existing Windows-hook injector. Do not report the
   collector resident until a session-matching heartbeat arrives, or report
   recording until the first structurally valid session frame arrives.
7. Record until process exit or explicit app quit. Preserve/finalize every valid
   prefix. Failure creates a durable failed artifact and persistent red status.
8. Attach every proven simultaneous the game process independently. Never choose one
   silently or merge output merely because hours/modsets match.

The coordinator supervises one `collector_record` worker per game process. It is
started explicitly from the portable package; no installer-created sign-in task is
assumed. Closing its normal window or pressing **Stop and finish recordings**
performs graceful stop/finalization. There is no tray lifecycle.

## 2. Watcher: WMI plus bounded reconciliation

`Win32_ProcessStartTrace` carries `ProcessID`, `ProcessName`, `SessionID`, SID, and
an event timestamp, fitting app-first discovery without walking all processes repeatedly
([Microsoft](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/krnlprov/win32-processstarttrace)).
`IWbemServices::ExecNotificationQuery` creates an event subscription that receives
events until its enumerator is released
([Microsoft](https://learn.microsoft.com/en-us/windows/win32/api/wbemcli/nf-wbemcli-iwbemservices-execnotificationquery)).
The dev launcher instead takes one `Get-Process -Name the game` snapshot and fails on
none or demands a PID on several (`Start-Hoi4ProductionSpeed5Capture.ps1:70-80`),
so it cannot be the product watcher.

WMI-only is rejected because provider failure must not silently disable attach.
Poll-only is rejected as the healthy default because it walks the process list
forever to learn nothing changed. The hybrid bounds work and failure:

- healthy: one startup snapshot plus at most 120 reconciliations/hour;
- degraded: at most 1,800 snapshots/hour, worst-case discovery delay 2 seconds;
- WMI loss/recovery is a journal and UI event;
- snapshots use `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` and
  `Process32First/Next`, Windows' documented read-only enumeration path
  ([snapshot](https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot),
  [enumeration](https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-process32first)).

Use blocking WMI enumerators on a dedicated thread with a cancellation handle.
Subscribe separately to `__InstanceCreationEvent WITHIN 1` and
`__InstanceDeletionEvent WITHIN 1`, restricted to `TargetInstance ISA
'Win32_Process'`, then filter `hoi4.exe` case-insensitively in-process. The
extrinsic `Win32_ProcessTrace` provider is not a standard-user contract: it
returns `WBEM_E_ACCESS_DENIED` on the supported host. The intrinsic event path
has a one-second provider sampling interval, still below the two-second degraded
reconciliation bound, and its live gate must prove both a real start and stop.

Exact startup order: open journal/singleton; establish both subscriptions;
enumerate a complete snapshot; drain queued events; reduce both sources through one
idempotent reducer; then start reconciliation. A game starting before subscription
appears in the snapshot; one starting after appears in WMI and may appear in both.
Full process identity deduplicates it. Reused PID/new FILETIME is a new identity.

## 3. Candidate proof and portable discovery

Retain full-precision creation FILETIME, canonical image path, stable executable
size/SHA-256, Windows session id, process-token user SID, and parent PID (telemetry
only). Existing code queries full image path via
`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`/`QueryFullProcessImageNameW`
(`modset.rs:602-616`) and creation time via `GetProcessTimes`
(`modset.rs:619-642`); extract and reuse those functions.

Accept only when basename is `hoi4.exe` (Windows case-insensitive), token SID and
process session equal the coordinator user/session, the identity re-read just
before injection still matches, and the modset is fully resolved. WMI SID/session
fields are hints; the live token/handle is authority.

Never scan Steam libraries or assume a drive. The live process supplies its path;
`resolve_modset` already hashes/records it (`modset.rs:353-376`). Resolve installed
payload relative to the packaged app executable or signed manifest, never this repo.

Reuse the strong user-dir proof:

- candidate contains `dlc_load.json`;
- attached PID holds an open handle below canonical `logs/`; Restart Manager errors
  fail closed (`modset.rs:790-829`);
- executable/config/mod/DLC evidence is stability-checked and ordered against
  process start (`modset.rs:328-420`, `modset.rs:913-948`).

Do **not** reuse its candidate list unchanged. It probes only
`%USERPROFILE%\\Documents` and `%USERPROFILE%\\OneDrive\\Documents`
(`modset.rs:951-998`). Product candidates, canonical-deduplicated, are the proven
process command line's `-userdir`, `FOLDERID_Documents` from
`SHGetKnownFolderPath`, runtime-discovered OneDrive consumer/business roots, and
the two legacy paths as compatibility candidates.

Run PID-held-log corroboration across every candidate containing `dlc_load.json`.
Exactly one physical directory may survive. Zero/multiple remains
`WaitingForIdentity`, listing all candidates/reasons. Normal product exposes no
`--pdx-user-dir`; a support override cannot turn unresolved evidence into resolved.
The recorder currently continues after unresolved modset evidence
(`main.rs:1164-1180`); auto-attach blocks injection while preserving errors.

## 4. State machine

| State | Exit / retry rule | Visible result |
|---|---|---|
| `Detected` | Immediately query process | the game detected |
| `WaitingForIdentity` | Retry 250 ms, 500 ms, 1 s, then every 2 s while alive | Exact missing/ambiguous proof |
| `WaitingForWindow` | Same capped backoff; process stop ends attempt | the game is still starting |
| `Preparing` | Unique paths, payload hash, atomic control, pipe | Preparing recorder |
| `Injecting` | Pipe-listening barrier satisfied; install hook once | Attaching |
| `Handshaking` | Require matching heartbeat; then enter `WaitingForWorld` | Waiting for collector proof |
| `WaitingForWorld` | Preserve status records until valid session frame | Attached; waiting for game/save |
| `Recording` | Continue to process exit/explicit stop | Current/last confirmed hour |
| `Finalizing` | Fold, pack, verify; serialize heavy work globally | Saving replay |
| `Complete` | Verified replay | Replay path and coverage |
| `Failed` | Persist artifacts; no uncertain reinjection | Persistent red reason/action |

Readiness retries are cheap probes. Cache a successful stable modset by full process
identity; do not re-hash all content every 2 seconds. Process-lifetime handshake
deadline is 30 seconds: a liveness policy, not measured the game startup; timeout is
loud and retains diagnostics.

## 5. Attach transaction

### Existing mechanism to reuse

The launcher passes `--pid`, `--dll`, output, profile, session, and static catalogs
to `collector_record` (`Start-Hoi4ProductionSpeed5Capture.ps1:307-326`). The host:

- writes `collector_control.json` beside the DLL (`main.rs:5150-5210`);
- creates a PID plus unique-token pipe (`main.rs:1181-1199`);
- finds a visible PID-owned window, resolves `Hoi4ReplayGetMsgProc`, and installs
  `WH_GETMESSAGE` hooks (`main.rs:5213-5258`);
- pokes target threads, then holds/removes hooks (`main.rs:5259-5281`).

The DLL starts one guarded worker from `DllMain`/hook callback
(`collector/src/lib.rs:1122-1163`, `collector/src/lib.rs:71063-71096`), reads control
beside its module, and connects to the PID/token pipe
(`collector/src/lib.rs:1184-1219`, `collector/src/lib.rs:2087-2103`,
`collector/src/lib.rs:45808-45833`). Auto-attach must reuse this, not add
`CreateRemoteThread` or a second collector.

### Required ordering

1. Atomically create a never-before-used attempt/staging directory.
2. Copy packaged DLL under a unique filename; journal source/copied hashes and
   refuse mismatch.
3. Write/flush `collector_control.json.tmp`, then atomically rename it.
4. Create the named pipe synchronously and signal an in-process listening barrier.
5. Re-read PID/creation FILETIME and window-owner PID.
6. Install the existing hook exactly once.
7. Require heartbeat with expected PID, pipe token, session id, ABI/version, and
   copied-DLL hash, then journal `collector_attached` and enter `WaitingForWorld`.
8. Journal `recording_started` only after a structurally valid frame proves a real
   session clock/root, every scheduled domain has explicit status, and no domain
   emits delta rows before its baseline. Failure-status domains remain explicit
   coverage failures; they are never omitted or relabelled clean.

The pipe barrier is mandatory. Today the host spawns the pipe thread and immediately
calls `attach_hook` (`main.rs:1193-1203`), while the DLL tries `CreateFileW` once and
returns `None` on failure (`collector/src/lib.rs:45808-45833`). That leaves an
avoidable connect-before-listen race.

Do not retry an uncertain hook/handshake outcome in the same live process. A second
unique DLL has independent module statics, so its worker guard does not prevent two
collectors. Retry is allowed only when the host proves the prior module is unloaded
and its worker lease is gone; otherwise retain `Failed` until a new process identity.

### Unique DLL path

Never overwrite, rename aside, or reuse a DLL path hooked into a live game. A live
incident isolated repeat second-window crashes to reuse of shared `collector.dll`;
unique per-window directories then passed three windows in one PID with every
liveness probe alive (`PRODUCTION_ENDGAME_PLAN.md:211-239`). Use a fresh directory
per attempt, not just per PID. Keep it until target exit and finalization; cleanup
failure is visible and never permits path reuse.

## 6. Startup, mid-session, and lifetime semantics

### Mid-load or menu

Process-start can precede the main window. The injector requires a visible
PID-owned window (`main.rs:5213-5221`), so `WaitingForWindow` is normal. Once window
and identity resolve, inject even if no campaign is loaded. Preserve heartbeat,
root-validation, and status records in `WaitingForWorld`; do not label menu/invalid
roots as campaign hours. A frame with no valid session clock/root cannot start
coverage. Once recording starts, partial/failure domains remain reason-tagged
attempts, while a delta-before-baseline domain is a hard stream error.

### Game-first in a campaign

Attach through the same proof sequence and start at the first full baseline actually
observed. Do not synthesize earlier hours or backdate the segment. Record:

- `segment_start_reason = mid_session_attach`;
- first observed hour and wall-clock time;
- `coverage_before_start = no_data`;
- process/build/modset evidence and attempt id.

The segment is replayable from that baseline. Joining it to an older segment depends
on unbuilt campaign identity/stitching. Canon says missing live spans are `no_data`
and uncertain continuity makes a boundary
(`documents/archive/DESIGN_legacy_full_2026-07-02.md:747-767`); the recording plan
requires time jumps/rollbacks to create segments and surface gaps
(`archive/plans/RECORDING_LAYER_COMPLETION_PLAN.md:1426-1443`).

Never merge on modset, date, PID, or wall clock. Without campaign lineage finalize
`stitching_status = campaign_identity_unresolved`. Later stitching may apply the
owner's later-played-supersedes rule while retaining old branches
(`PRODUCT_SESSION_SEMANTICS.md:53-61`); this layer only preserves evidence.

### Process-lifetime capture

Dev `FullGame` is a finite 21,600-second window
(`Start-Hoi4ProductionSpeed5Capture.ps1:182-198`). Product needs
`--until-process-exit`, mutually exclusive with `--duration-s`:

- host waits on target-process and per-attempt stop handles, not `hold_ms` sleep;
- control carries `stop_policy = process_exit_or_host_request` plus random stop
  event instead of finite duration;
- DLL checks stop between capture attempts; pipe write failure stops the worker and
  releases its module pin;
- game exit breaks the pipe; host already classifies `ERROR_BROKEN_PIPE` as clean
  EOF (`main.rs:5335-5341`);
- explicit quit signals stop, waits for acknowledgement/EOF, removes hooks, and
  finalizes;
- supervisor places workers in a kill-on-close Job Object. After supervisor crash,
  restart waits until the old unique module is proven unloaded before reinjection.

Raw pipe data is already streamed to disk in bounded memory
(`main.rs:1191-1203`, `main.rs:5345-5354`). A game crash finalizes a valid verified
prefix with abnormal termination. Truncated/unverifiable streams remain failed
artifacts and are never promoted.

## 7. Multi-instance and zero-settings behavior

Multiple same-user/same-session the game processes get independent state machines and
all attach. This avoids silently choosing one and dropping another. Each requires a
unique staging/DLL path, pipe/control/stop tokens, output directory, session id,
recorder child, raw stream, and later stitching decision. One global finalization
semaphore prevents folding one ended session from starving live capture on a weaker
PC. The proof code notes two simultaneous instances may share one user dir and the
same `dlc_load.json` (`modset.rs:790-798`); their streams remain distinct.

A named current-user/current-session singleton prevents duplicate coordinators. A
second app invocation reconnects/activates the first; it never starts another
watcher or injector.

Normal UI exposes no PID, Steam path, user-dir, DLL, duration, pipe, or output-path
setting. Resolve app payload from the packaged executable/manifest; staging,
recordings, and Documents through Windows Known Folder APIs; and game path from the
live process. Generate random session/attempt ids. Developer overrides may remain
for support, but no correctness policy is user-tunable.

Check free space before and during capture. Crossing the safe floor fails loudly
and preserves the prefix; never delete capture bytes to make room.

## 8. Loud failure and durable closure

The window and durable journal expose global watcher state plus per-process identity state: current
transition, exact reason, last confirmed hour, output path, and valid-prefix status.
WMI+fallback failure, identity/modset ambiguity, privilege mismatch, payload/pipe/
hook/handshake failure, pipe break while game lives, disk failure, pack/verify
failure, or unresolved stitching all require persistent red status and notification.

Append and flush one `auto_attach_journal.jsonl` record before each external action:

```text
schema, event_seq, observed_at_utc, coordinator_instance_id,
pid, process_creation_filetime, process_session_id, process_user_sid_hash,
exe_path, exe_sha256, attempt_id, state_from, state_to, reason_code,
reason_detail, staging_dir, run_dir, session_id, pipe_token_hash,
first_observed_hour, last_attempted_hour, last_confirmed_hour,
artifact_status, replay_path
```

Do not log raw SID or pipe tokens. On startup, replay the journal and reconcile every
nonterminal identity with live processes/worker leases, recording recovery. Every
detected identity must end `Complete`, `Failed`, or `ProcessExitedBeforeAttach`.

## 9. Exact implementation map (outside this lane's fence)

The implementing lane should make these changes; this lane intentionally does not.

### `extractor/native/recorder_host/src/lib.rs`

```diff
+ pub mod auto_attach;
  pub mod modset;
```

### `extractor/native/recorder_host/src/auto_attach.rs` (new)

```diff
+ Define ProcessKey, CandidateState/reasons, idempotent reducer, JSONL journal and
  recovery, WMI start/stop source, Toolhelp snapshot source, 30 s reconciliation /
  2 s degraded policy, singleton/worker leases, per-instance supervisor, UI events.
+ Abstract ProcessEventSource, ProcessProbe, Clock, PayloadStager, WorkerSpawner for
  deterministic race, PID-reuse, and WMI-loss tests.
```

### `extractor/native/recorder_host/src/modset.rs`

```diff
+ pub struct ProcessIdentity { pid, creation_filetime, exe_path, ... }
+ pub fn query_process_identity(pid) -> Result<ProcessIdentity, ...>
+ pub fn resolve_modset_for_identity(identity, discovered_dirs) -> ModsetReport
+ pub fn discover_user_dir_candidates(identity) -> Result<Vec<PathBuf>, ...>
~ Move existing image-path/start-time helpers behind ProcessIdentity.
~ Add command-line -userdir, FOLDERID_Documents, and runtime OneDrive roots;
  canonical-deduplicate before unchanged PID-held-log corroboration.
- Do not treat the two USERPROFILE string paths as complete product discovery.
```

### `extractor/native/recorder_host/src/main.rs`

```diff
+ Add --auto-attach supervisor and --until-process-exit worker modes.
+ Split pipe creation from reading; expose positive listening barrier.
+ Revalidate ProcessIdentity immediately before attach_hook.
+ Wait hook lifetime on process/stop handles in lifetime mode.
+ Validate heartbeat identity/ABI/hash before attached and the first structurally
  valid session frame/baseline ordering before recording.
+ Emit lifecycle events and abnormal-prefix termination reasons.
~ Keep attach_hook/WH_GETMESSAGE as the only injector.
- Do not continue product auto-attach when ModsetReport.resolved == false.
```

### `extractor/native/collector/src/lib.rs`

```diff
+ Parse process-lifetime stop event and session identity fields.
+ Put pid, session_id, pipe proof, ABI/version, and loaded-DLL hash in heartbeat.
+ Propagate pipe WriteFile failure; stop worker and release module pin.
+ Require first in-session domain emission to be a full baseline and expose
  reasoned WaitingForWorld before that.
- Do not ignore WriteFile result in the product pipe path.
```

### `extractor/native/recorder_host/Cargo.toml` and lockfile

```diff
+ Add one pinned Windows API binding dependency/features for WMI COM, Toolhelp,
  tokens, Known Folders, Job Objects, named synchronization, and process waits.
+ Keep APIs local-machine-only; no network discovery or admin service.
```

### Packaging/UI entry point

```diff
+ Launch the resident supervisor from the recorder's ordinary no-argument entry.
+ Keep the normal desktop window open while watching/recording.
+ Make window close and the explicit stop button stop-and-finalize operations.
- Do not install a tray icon, background service, sign-in task, or updater.
- Remove PID/path/duration controls from normal product UI.
```

Keep `Start-Hoi4ProductionSpeed5Capture.ps1` as dev/live-validation tooling; do not
turn its single-PID finite-duration interface into the product watcher.

## 10. Verification gates

### Deterministic gates

1. **Dual order:** startup-snapshot process and later WMI start both attach.
2. **Subscribe race:** process starts before subscription, between subscription and
   snapshot, and after snapshot; each identity creates exactly one attempt.
3. **PID reuse:** same PID/new FILETIME creates a new attempt with no old state.
4. **WMI loss:** failure is journaled, 2-second scan discovers, recovery returns to
   30-second reconciliation without duplicate attach.
5. **Identity fail-close:** wrong SID/session, path error, ambiguous dirs, unstable
   hash, or unresolved modset never reaches injection.
6. **Pipe barrier:** delayed listener proves no early injection; missing heartbeat
   never reaches `WaitingForWorld`, and missing valid session frame never reaches
   `Recording`.
7. **Injection uncertainty:** handshake timeout creates one failure and no second DLL.
8. **Unique staging:** 1,000 generated attempt paths do not collide; copied-hash
   mismatch and existing destination fail closed.
9. **Multi-instance:** two identities have separate paths/tokens/workers/streams and
   terminals; serialized finalization does not stop the surviving stream.
10. **Crash recovery:** kill GUI, supervisor, recorder, and target at every state;
    all detected identities close and all written bytes remain retained.
11. **Mid-session:** first valid frame is baseline, prior coverage is `no_data`, and
    no merge occurs without campaign lineage.
12. **No silent write failure:** injected pipe/disk/fold/pack failures produce a
    visible reason and retained failed artifact.

### Live Windows matrix

Run on the normal dev PC and at least one weaker target PC: app-first; menu-first;
loaded-campaign-first; game relaunch while app remains; non-default Steam drive;
Known-Folder/OneDrive Documents; WMI unavailable; two simultaneous games; access
denied/different session; explicit app quit; game crash; supervisor crash/restart
while the game remains.

Retain timestamps for observation, injection, heartbeat, first baseline, and
terminal. Require zero missing candidate closures, duplicate attachments, shared
DLL/control paths, or discarded bytes, and either a verified replay or explicit
failed artifact for every attempt.

### Idle-cost gate

Measure 30 minutes with no game for app-without-watcher, healthy WMI, and forced
degraded scanning. Record snapshot count, CPU time, wakeups/context switches,
private bytes, and WMI reconnects. Healthy WMI permits at most 61 snapshots
(startup plus 30-second reconciliation); degraded permits at most 900. Unbounded
growth, busy loop, or count above bound fails. Report CPU/memory deltas on both
machines; do not label them acceptable until weaker-PC evidence is owner-approved.

## 11. Self-refutation

- **WMI alone is reliable and waste-free:** refuted; provider failure can silently
  miss launches. Surviving policy is event-first plus visible bounded fallback.
- **Process-start means injectable:** refuted by the visible-window requirement.
  Surviving policy is indefinite, reasoned readiness wait.
- **Hook success means attached:** refuted by the current pipe race and lack of
  collector proof. Surviving policy is listening barrier, heartbeat for residency,
  then structurally valid session frame for recording.
- **Reuse modset resolver unchanged on arbitrary machines:** refuted; its PID-held
  log proof survives, but its two candidate paths do not. Expand discovery first.
- **Fresh DLL path makes automatic retry safe:** refuted; loaded copies have
  independent static worker guards. Prove old module/lease absent or do not retry.
- **Multiple games require a PID picker:** refuted; independent outputs remove the
  choice. Attach all proven same-user/session instances without merging.
- **Auto-attach makes mid-campaign capture complete:** refuted; earlier hours were
  unobserved and stitching is unbuilt. Start baseline now and record leading no-data.
- **Dev six-hour FullGame satisfies end-of-game output:** refuted; it can expire
  first. Product capture follows process lifetime and has graceful stop.

## 12. Explicit non-closures

This design does not close campaign/save identity, cross-session stitching/rollback
selection, or the live WMI/multi-instance/crash-recovery/weaker-PC proof. The
packaging/UI implementation is now present, but none of those separate acceptance
rows may be reported closed merely because this design specifies them.
