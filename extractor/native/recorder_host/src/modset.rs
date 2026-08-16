//! Modset + game-build binding for the session manifest (M2 in
//! MOD_AND_UPDATE_ROBUSTNESS_PLAN.md).
//!
//! Every capture records WHICH game build, enabled mods, and enabled DLCs the
//! evidence was taken against, because all downstream joins (static catalogs,
//! validation bounds, RE'd offsets) are only valid against the recording's
//! modset — a modded recording silently joined against vanilla catalogs is the
//! worst-gap failure mode the plan names. Resolution is FAIL-CLOSED and
//! NEVER GUESSES: every step that cannot be proven records an error string and
//! drops `resolved` to false; the manifest then carries the partial evidence
//! plus the reasons, and downstream consumers must treat an unresolved modset
//! as "unknown", not "vanilla".
//!
//! G2 gate amendments (2026-07-11, REFUTED -> fix round):
//! - `dlc_load.json` fields must be PRESENT and WELL-TYPED: a missing or
//!   mistyped `enabled_mods`/`disabled_dlcs` fails closed (a `{}` document
//!   previously resolved as confident vanilla — the exact silent-vanilla trap).
//! - Every `disabled_dlcs` entry must match an INSTALLED census key exactly;
//!   an unmatched key is ambiguity (case/format drift could silently enable a
//!   disabled DLC) and fails closed.
//! - A mod descriptor must carry `path=`/`archive=` (and parse at least one
//!   recognized field) or the binding is incomplete -> fail closed.
//! - Mod CONTENT gets a fingerprint (file count + total bytes + max mtime,
//!   bounded walk) — a Workshop update that leaves the descriptor untouched
//!   changes the fingerprint. This is a fingerprint, not a proof; the
//!   descriptor sha stays the text binding.
//! - The exe hash records size+mtime BEFORE and re-checks AFTER hashing; a
//!   mid-hash mutation fails closed. (The pathname-vs-mapped-image race
//!   cannot be fully closed from user mode; the stability check plus recording
//!   the path narrows it and is recorded as a residual.)
//! - `dlc_load.json` mtime and the process start time are both recorded; a
//!   config file NEWER than the process start means the launcher state may not
//!   be the state the game was started with -> fail closed (evidence kept).
//! - `user_dir_source` records probed-vs-override provenance.
//! - When `resolved` is false the JSON nests all evidence under
//!   `partial_evidence` so a naive consumer reading `enabled_mods` at the top
//!   level finds NOTHING rather than a vanilla-looking answer.
//!
//! G2-regen amendments (2026-07-12, REFUTED 5 BLOCKER + 2 MAJOR -> this round):
//! - Duplicate-key detection compares DECODED key names (spelling one
//!   `enabled_mods` key with a backslash-u escape of the underscore previously
//!   bypassed the raw-slice comparison while serde kept the last, empty,
//!   value — silent vanilla).
//! - The chosen user dir (override AND probed) must be CORROBORATED against
//!   the attached process: the dir's `logs/` must have been written at/after
//!   process start (the running game writes its logs there continuously). A
//!   stale or wrong-but-well-formed directory no longer resolves.
//! - Mod content is BOUND by a SHA-256 over sorted relative paths + file
//!   bytes (double-pass stability bracket); the (count, bytes, max-mtime)
//!   tuple stays as cheap metadata but no longer carries the binding (rename /
//!   same-length edits / same-second edits defeated it).
//! - ALL mod/DLC disk evidence must PREDATE process start: a descriptor,
//!   content file, or installed-DLC descriptor written at/after launch means
//!   the disk no longer proves what the running process loaded (Workshop
//!   update between launch and attach) -> fail closed.
//! - `dlc_load.json` is read TWICE and must be byte-identical (an atomic
//!   same-mtime replacement — OneDrive sync — defeated the mtime-only
//!   bracket). Residual: a replacement completing before the first read with
//!   a preserved mtime is indistinguishable from user mode; recorded here.
//! - A descriptor carrying BOTH `path=` and `archive=`, or repeating either,
//!   is ambiguous (engine precedence unproven) -> fail closed.
//! - The repack path (main.rs) now refuses manifests without a current-schema
//!   `modset` object (legacy/absent bindings cannot be reconstructed).

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use snapshot::sha256::{to_hex, Sha256};

#[path = "auto_attach/modset.rs"]
mod auto_attach_modset;

/// How a path is spelled inside a resolution failure.
///
/// A diagnostic is the least load-bearing thing in the session manifest and it
/// used to be the one thing that could destroy a recording: the portable
/// projection reduces a string that IS a path, but a path EMBEDDED in prose
/// survived it and then tripped the seal-time rejector. Capture t55 lost 91
/// game hours to `$.modset.errors[0]` for that reason. Errors here therefore
/// name a file the way the container names one — by its final component —
/// using the sealer's own function so the two can never disagree.
///
/// The absolute path is not lost: `exe_path`, `pdx_user_dir` and the descriptor
/// paths are recorded as fields, and the RUN-DIRECTORY copy of the manifest
/// keeps those absolute for local recovery and audit.
fn portable_path(path: &Path) -> String {
    crate::portable_manifest::redact_absolute_path_string(&path.display().to_string())
}

/// A resolution failure: a STABLE CODE a consumer can branch on, plus prose for
/// a human.
///
/// The code exists because prose was already being parsed as a protocol —
/// same-process settings recovery matched on the English prefix
/// "settings.txt launch settings unresolved:", so rewording a message silently
/// disabled a recovery path. Nothing branches on `detail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModsetError {
    pub code: &'static str,
    pub detail: String,
}

impl ModsetError {
    /// Compose the failure, guaranteeing it can cross the container boundary.
    ///
    /// The per-site spelling above is the fix; this is the backstop for text
    /// this module did not compose itself — an `io::Error` that quotes a path,
    /// or a future edit that forgets. A detail that still cannot travel is
    /// dropped rather than shipped, because a diagnostic is never worth a
    /// refused seal.
    fn new(code: &'static str, detail: String) -> Self {
        let portable = detail
            .split_whitespace()
            .map(crate::portable_manifest::redact_absolute_path_string)
            .collect::<Vec<_>>()
            .join(" ");
        let detail = if crate::portable_manifest::reject_absolute_paths_in_json(
            &Value::String(portable.clone()),
            "$",
        )
        .is_ok()
        {
            portable
        } else {
            "detail withheld: not portable".to_string()
        };
        Self { code, detail }
    }
}

impl std::fmt::Display for ModsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

/// Stable failure codes. Adding a case adds a constant; nothing matches on
/// prose.
pub mod codes {
    pub const EXE_PATH_QUERY: &str = "exe_path_query_failed";
    pub const EXE_HASH: &str = "exe_hash_failed";
    pub const SUSPENDED_EXE_MISMATCH: &str = "suspended_executable_mismatch";
    pub const PROCESS_START_QUERY: &str = "process_start_query_failed";
    pub const USER_DIR: &str = "user_dir_unresolved";
    pub const DLC_LOAD_MTIME_AT_OR_AFTER_START: &str = "dlc_load_mtime_at_or_after_process_start";
    pub const DLC_LOAD_MTIME_UNAVAILABLE: &str = "dlc_load_mtime_unavailable";
    pub const DLC_LOAD_READ: &str = "dlc_load_read_failed";
    pub const DLC_LOAD_PARSE: &str = "dlc_load_parse_failed";
    pub const DLC_LOAD_UNSTABLE: &str = "dlc_load_unstable_while_read";
    pub const DLC_LOAD_DUPLICATE_KEYS: &str = "dlc_load_duplicate_top_level_keys";
    pub const DLC_LOAD_ENABLED_MODS: &str = "dlc_load_enabled_mods_unusable";
    pub const DLC_LOAD_DISABLED_DLCS: &str = "dlc_load_disabled_dlcs_unusable";
    pub const MOD_DESCRIPTOR_UNREADABLE: &str = "mod_descriptor_unreadable";
    pub const MOD_CONTENT_LOCATION_AMBIGUOUS: &str = "mod_content_location_ambiguous";
    pub const MOD_CONTENT_LOCATION_ABSENT: &str = "mod_content_location_absent";
    pub const MOD_CONTENT_NOT_HASHABLE: &str = "mod_content_not_hashable";
    pub const MOD_DESCRIPTOR_AT_OR_AFTER_START: &str = "mod_descriptor_at_or_after_process_start";
    pub const MOD_DESCRIPTOR_MTIME_UNAVAILABLE: &str = "mod_descriptor_mtime_unavailable";
    pub const MOD_CONTENT_AT_OR_AFTER_START: &str = "mod_content_at_or_after_process_start";
    pub const DLC_CENSUS: &str = "dlc_census_failed";
    pub const DLC_CENSUS_EMPTY: &str = "dlc_census_empty_or_timestampless";
    pub const DLC_CENSUS_AT_OR_AFTER_START: &str = "dlc_census_at_or_after_process_start";
    pub const DLC_CENSUS_UNSTABLE: &str = "dlc_census_unstable_between_passes";
    pub const DLC_DISABLED_UNMATCHED: &str = "disabled_dlc_matches_no_installed_dlc";
    pub const DLC_DISPLAY_NAME: &str = "dlc_display_name_unresolved";
    pub const EXE_NO_PARENT: &str = "exe_path_has_no_parent";

    // Settings binding (kept separate from the content binding above).
    pub const SETTINGS_UNREADABLE: &str = "settings_unreadable";
    pub const SETTINGS_LANGUAGE_ABSENT: &str = "settings_language_absent";
    pub const SETTINGS_NO_PROCESS_START: &str = "settings_process_start_unavailable";
    pub const SETTINGS_NO_USER_DIR: &str = "settings_user_dir_uncorroborated";
    pub const SETTINGS_ENDPOINT_UNREADABLE: &str = "settings_endpoint_read_failed";
    pub const SETTINGS_ENDPOINT_PROCESS_GONE: &str = "settings_endpoint_process_identity_lost";
}

type Handle = *mut c_void;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// Bounded mod-content walk: enough for any real mod, refuses runaway trees.
/// `pub` because the repack validator mirrors it: a resolved manifest
/// claiming a fingerprint above this bound is live-impossible.
pub const MOD_CONTENT_MAX_FILES: usize = 200_000;

extern "system" {
    fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        exe_name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
}

// --- Restart Manager (modset re-gate finding 1): which PIDs hold a file ---
const CCH_RM_SESSION_KEY: usize = 32;
const CCH_RM_MAX_APP_NAME: usize = 255;
const CCH_RM_MAX_SVC_NAME: usize = 63;
const ERROR_MORE_DATA: u32 = 234;

#[repr(C)]
#[derive(Clone, Copy)]
struct RmUniqueProcess {
    process_id: u32,
    start_time_low: u32,
    start_time_high: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RmProcessInfo {
    process: RmUniqueProcess,
    app_name: [u16; CCH_RM_MAX_APP_NAME + 1],
    service_short_name: [u16; CCH_RM_MAX_SVC_NAME + 1],
    application_type: u32,
    app_status: u32,
    ts_session_id: u32,
    restartable: i32,
}

#[link(name = "rstrtmgr")]
extern "system" {
    fn RmStartSession(session: *mut u32, flags: u32, session_key: *mut u16) -> u32;
    fn RmRegisterResources(
        session: u32,
        n_files: u32,
        file_names: *const *const u16,
        n_apps: u32,
        apps: *const c_void,
        n_services: u32,
        service_names: *const *const u16,
    ) -> u32;
    fn RmGetList(
        session: u32,
        needed: *mut u32,
        n_proc_info: *mut u32,
        proc_info: *mut RmProcessInfo,
        reasons: *mut u32,
    ) -> u32;
    fn RmEndSession(session: u32) -> u32;
}

/// True iff `pid` holds an open handle to `file` (Restart Manager query).
/// Errors are RM-session failures, distinct from a clean "not held".
fn file_held_by_pid(file: &Path, pid: u32) -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY + 1];
        let rc = RmStartSession(&mut session, 0, key.as_mut_ptr());
        if rc != 0 {
            return Err(format!("RmStartSession failed: {rc}"));
        }
        let result = (|| {
            let wide: Vec<u16> = file
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let names = [wide.as_ptr()];
            let rc = RmRegisterResources(
                session,
                1,
                names.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
            if rc != 0 {
                return Err(format!("RmRegisterResources failed: {rc}"));
            }
            let mut needed = 0u32;
            let mut count = 0u32;
            let mut reasons = 0u32;
            let rc = RmGetList(
                session,
                &mut needed,
                &mut count,
                std::ptr::null_mut(),
                &mut reasons,
            );
            if rc == 0 {
                return Ok(false); // nobody holds it
            }
            if rc != ERROR_MORE_DATA {
                return Err(format!("RmGetList size probe failed: {rc}"));
            }
            // Headroom for holders appearing between the two calls.
            let capacity = needed.saturating_add(4) as usize;
            let mut infos: Vec<RmProcessInfo> = vec![std::mem::zeroed(); capacity];
            let mut count = capacity as u32;
            let rc = RmGetList(
                session,
                &mut needed,
                &mut count,
                infos.as_mut_ptr(),
                &mut reasons,
            );
            if rc != 0 {
                return Err(format!("RmGetList failed: {rc}"));
            }
            infos.truncate(count as usize);
            Ok(infos.iter().any(|info| info.process.process_id == pid))
        })();
        RmEndSession(session);
        result
    }
}

/// One enabled mod, resolved from its launcher-listed `.mod` descriptor.
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// The descriptor path exactly as listed in `dlc_load.json`.
    pub descriptor_entry: String,
    /// The absolute descriptor path that was actually read (if found).
    pub descriptor_path: Option<PathBuf>,
    /// sha256 of the descriptor text — the "same mod text" binding.
    pub descriptor_sha256: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub remote_file_id: Option<String>,
    /// The mod content location the descriptor points at (`path=`/`archive=`).
    pub content_path: Option<String>,
    /// Cheap content metadata of the location `content_path` names:
    /// (file_count, total_bytes, max_mtime_unix). METADATA ONLY — the binding
    /// is `content_sha256` (G2-regen finding 3: this tuple is systematically
    /// non-unique). None when the content location could not be walked (which
    /// also fails resolution closed).
    pub content_fingerprint: Option<(u64, u64, i64)>,
    /// SHA-256 over the content: sorted relative paths + per-file length +
    /// file bytes (or the single archive file), computed under a double-pass
    /// stability bracket. THE content binding. None -> resolution fails closed.
    pub content_sha256: Option<String>,
    /// Newest mtime observed anywhere in the content walk (full precision,
    /// for the evidence-predates-launch check). None when unknown.
    pub content_max_mtime: Option<SystemTime>,
    /// Descriptor file mtime (full precision, same check).
    pub descriptor_mtime: Option<SystemTime>,
    /// True when the descriptor repeats `path=`/`archive=` or carries both —
    /// the effective content location is ambiguous (G2-regen finding 6).
    pub content_location_conflict: bool,
}

/// How well the recording knows the settings the running game is using.
///
/// SEPARATE FROM THE CONTENT BINDING ON PURPOSE. `settings.txt` contributes
/// only the localisation language and the display variant; the mods and DLC
/// come from `dlc_load.json` and the `dlc/` census. Folding both into one
/// `resolved` bit meant a settings failure erased a fully proven mod/DLC
/// binding — which is what happened on 2026-08-15: the game rewrites
/// `settings.txt` during load, so on any hand-launched game the pre-launch
/// mtime rule can never pass, the whole report went unresolved, and the
/// attach-and-play path could record nothing content-bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAuthority {
    /// The file was proven byte-stable and written BEFORE the process started,
    /// so the running game read exactly these bytes. The strongest statement,
    /// and the only one available when the launch is ours to arrange.
    PreLaunchBracketed,
    /// A previous recorder generation proved `PreLaunchBracketed` for this
    /// exact still-live process identity, and the file still says the same
    /// thing semantically.
    SameProcessAuthority,
    /// The file's mtime is at/after process start. The game itself rewrites
    /// `settings.txt` while loading, so this is the ORDINARY case for a game
    /// the player launched.
    ///
    /// DELIBERATELY NOT CALLED "the game's own rewrite": nothing here proves
    /// WHO wrote the file, and no cheap Windows call can attribute a write
    /// that has already happened. It also does not prove the values are the
    /// ones the process is using — a window can be resized, and the driver can
    /// negotiate a different mode, after the file was written. It is what the
    /// disk says, observed under the same stability bracket, and it is
    /// recorded as an observation.
    PostLaunchDiskObservation,
}

impl SettingsAuthority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreLaunchBracketed => "pre_launch_bracketed",
            Self::SameProcessAuthority => "same_process_authority",
            Self::PostLaunchDiskObservation => "post_launch_disk_observation",
        }
    }

    /// Whether the running process is PROVEN to have read these bytes.
    pub fn is_launch_proven(self) -> bool {
        matches!(self, Self::PreLaunchBracketed | Self::SameProcessAuthority)
    }
}

/// The settings half of the launch binding.
#[derive(Debug, Clone, Default)]
pub struct SettingsBinding {
    /// `None` when no settings could be read at all.
    pub authority: Option<SettingsAuthority>,
    pub errors: Vec<ModsetError>,
    /// `language=`. Never defaulted when the file or field is unavailable.
    pub language: Option<String>,
    /// Exact display inputs from the same stable read as `language`. Numeric
    /// scale identity is carried as IEEE-754 bits; no replay-side decimal
    /// re-rounding is permitted.
    pub display: Option<DisplayVariantSettings>,
    /// Digest of the settings file as read when the recorder attached.
    pub attach_sha256: Option<String>,
    /// Digest of the same file re-read as the capture window closed, while the
    /// process identity was still live.
    pub seal_sha256: Option<String>,
    /// Whether the attach-time and capture-close reads agreed SEMANTICALLY
    /// (the game reorders and reformats the file, so byte equality is not the
    /// question). `None` when the second observation could not be taken.
    ///
    /// This is an ENDPOINT comparison, not a claim about the whole window: a
    /// change made and undone between the two reads passes it.
    pub endpoint_semantics_equal: Option<bool>,
}

impl SettingsBinding {
    /// Record a settings failure from outside this module.
    pub fn fail_public(&mut self, code: &'static str, detail: String) {
        self.fail(code, detail);
    }

    fn fail(&mut self, code: &'static str, detail: String) {
        self.authority = None;
        self.language = None;
        self.display = None;
        self.errors.push(ModsetError::new(code, detail));
    }

    /// Language and display are both present under a stated authority.
    pub fn is_readable(&self) -> bool {
        self.authority.is_some() && self.language.is_some() && self.display.is_some()
    }

    pub fn as_json(&self) -> Value {
        json!({
            "authority": self.authority.map(SettingsAuthority::as_str),
            "launch_proven": self
                .authority
                .map(SettingsAuthority::is_launch_proven)
                .unwrap_or(false),
            "errors": self.errors.iter().map(ModsetError::to_string).collect::<Vec<_>>(),
            "language": self.language,
            "display_variant": self.display.as_ref().map(|display| json!({
                "physical_width": display.physical_width,
                "physical_height": display.physical_height,
                "min_gui_width": display.min_gui_width,
                "min_gui_height": display.min_gui_height,
                "gui_scale_f32_bits": display.gui_scale_f32_bits,
                "refresh_rate_hz": display.refresh_rate_hz,
                "max_refresh_rate_hz": display.max_refresh_rate_hz,
                "fullscreen": display.fullscreen,
                "borderless": display.borderless,
                "display_mode": display.display_mode,
            })),
            // BOTH endpoints, and neither one promoted to "the" settings hash.
            // The file can be reformatted between them while meaning the same
            // thing, and a single digest would then have to lie about one of
            // the two reads.
            "attach_settings_sha256": self.attach_sha256,
            "seal_settings_sha256": self.seal_sha256,
            "endpoint_semantics_equal": self.endpoint_semantics_equal,
        })
    }
}

/// The full modset + game-build binding, plus every resolution failure met on
/// the way.
///
/// `content_resolved` covers the executable, the corroborated user directory,
/// `dlc_load.json`, every enabled mod's descriptor and content, and the
/// installed-DLC census. It is true ONLY when that whole chain proved out. The
/// settings are a SEPARATE binding with its own state — see
/// [`SettingsBinding`].
#[derive(Debug, Clone)]
pub struct ModsetReport {
    pub content_resolved: bool,
    pub errors: Vec<ModsetError>,
    pub settings: SettingsBinding,
    pub exe_path: Option<PathBuf>,
    pub exe_size_bytes: Option<u64>,
    pub exe_sha256: Option<String>,
    pub user_dir: Option<PathBuf>,
    /// "override" (--pdx-user-dir) or "probed". A wrong override remains the
    /// recorded suspect when a binding later proves wrong.
    pub user_dir_source: Option<&'static str>,
    /// dlc_load.json mtime (unix seconds) and the game process creation time
    /// (unix seconds), both recorded so config-newer-than-process is provable.
    pub dlc_load_mtime_unix: Option<i64>,
    /// SHA-256 of the exact byte-identical `dlc_load.json` reads used below.
    /// This binds requested launcher selection evidence without another read.
    pub dlc_load_sha256: Option<String>,
    pub process_start_unix: Option<i64>,
    /// The same creation time at FULL PRECISION (FILETIME 100ns ticks).
    ///
    /// `process_start_unix` is the recorded, human-readable value and is
    /// floored to whole seconds; comparing identity with it would accept a PID
    /// reused by a process created inside the same second as the game.
    pub process_start_filetime: Option<u64>,
    pub enabled_mods: Vec<ResolvedMod>,
    pub installed_dlcs: Vec<String>,
    pub disabled_dlcs: Vec<String>,
    pub enabled_dlcs: Vec<String>,
    /// Installed descriptor path -> descriptor `name=` display value. A
    /// descriptor whose name cannot be read stays present with a null value;
    /// the corresponding resolution failure is recorded in `errors`.
    pub dlc_display_names: BTreeMap<String, Option<String>>,
    /// Installed descriptor path -> exact launch-bound descriptor evidence.
    /// The name and raw hash come from one byte-identical pre-start read.
    pub dlc_descriptor_evidence: BTreeMap<String, Option<DlcDescriptorEvidence>>,
}

impl ModsetReport {
    /// `language=` from settings.txt in the same corroborated Paradox user
    /// directory. Never defaulted when the file or field is unavailable.
    ///
    /// Read it together with `settings.authority`: an unproven authority means
    /// this is what the disk says, not what the process is proven to have read.
    pub fn localisation_language(&self) -> Option<&String> {
        self.settings.language.as_ref()
    }

    /// Exact display inputs from the same stable settings.txt read as
    /// [`Self::localisation_language`].
    pub fn display_variant(&self) -> Option<&DisplayVariantSettings> {
        self.settings.display.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DlcDescriptorEvidence {
    pub display_name: String,
    pub descriptor_file_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayVariantSettings {
    pub physical_width: u32,
    pub physical_height: u32,
    pub min_gui_width: u32,
    pub min_gui_height: u32,
    pub gui_scale_f32_bits: u32,
    pub refresh_rate_hz: u32,
    pub max_refresh_rate_hz: Option<u32>,
    pub fullscreen: bool,
    pub borderless: bool,
    pub display_mode: String,
    pub settings_file_sha256: String,
}

/// Exact launch-bound settings evidence retained across recorder generations
/// for one still-live process identity. the game rewrites `settings.txt` during
/// startup on some machines, so a recorder restart cannot re-prove the
/// pre-process mtime even though the first generation already did. This cache
/// is usable only for the identical PID+creation-FILETIME, executable hash,
/// canonical user directory, and semantically identical current settings.
/// the game may rewrite formatting/order after launch, so byte equality is not a
/// valid same-process reuse requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SameProcessSettingsAuthority {
    schema: String,
    pid: u32,
    process_creation_filetime: u64,
    exe_sha256: String,
    user_dir: String,
    localisation_language: String,
    display_variant: DisplayVariantSettings,
}

const SAME_PROCESS_SETTINGS_AUTHORITY_SCHEMA: &str =
    "hoi4_same_process_settings_authority_v1";

pub fn same_process_settings_authority_path(
    product_root: &Path,
    pid: u32,
    process_creation_filetime: u64,
) -> PathBuf {
    product_root
        .join("staging")
        .join("identity_commitments")
        .join(format!(
            "pid_{pid}_filetime_{process_creation_filetime}.settings.json"
        ))
}

/// Persist a newly proven launch-bound settings snapshot, or recover it after
/// a recorder-only restart when the ordinary resolver's sole failure is the
/// expected post-start `settings.txt` rewrite. Any mismatch leaves the report
/// unresolved.
pub fn apply_same_process_settings_authority(
    report: &mut ModsetReport,
    authority_path: &Path,
    pid: u32,
    process_creation_filetime: u64,
) -> Result<(), String> {
    // PUBLISH. A launch-proven settings read is the thing worth retaining
    // across a recorder restart, and it is now published on its own merits:
    // the CONTENT binding's state is a different proof and has no say in
    // whether these settings were proven.
    if report
        .settings
        .authority
        .is_some_and(SettingsAuthority::is_launch_proven)
    {
        let Some(exe_sha256) = report.exe_sha256.as_ref() else {
            return Err("launch-proven settings lack an executable hash".into());
        };
        let Some(user_dir) = report.user_dir.as_ref() else {
            return Err("launch-proven settings lack a canonical user directory".into());
        };
        let Some(localisation_language) = report.settings.language.as_ref() else {
            return Err("launch-proven settings lack a localisation language".into());
        };
        let Some(display_variant) = report.settings.display.as_ref() else {
            return Err("launch-proven settings lack a display variant".into());
        };
        let authority = SameProcessSettingsAuthority {
            schema: SAME_PROCESS_SETTINGS_AUTHORITY_SCHEMA.to_string(),
            pid,
            process_creation_filetime,
            exe_sha256: exe_sha256.clone(),
            user_dir: user_dir.display().to_string(),
            localisation_language: localisation_language.clone(),
            display_variant: display_variant.clone(),
        };
        return create_or_validate_same_process_settings_authority(authority_path, &authority);
    }

    // RECOVER. Only the post-launch observation is upgradable: it is the case
    // where an earlier generation may already have proven what this one cannot
    // re-prove. The match is on the settings binding's own state — it used to
    // be on the English prefix of an error string, so rewording a diagnostic
    // silently disabled recovery.
    if report.settings.authority != Some(SettingsAuthority::PostLaunchDiskObservation) {
        return Ok(());
    }
    let bytes = fs::read(authority_path).map_err(|error| {
        format!(
            "read same-process settings authority {}: {error}",
            portable_path(authority_path)
        )
    })?;
    let authority: SameProcessSettingsAuthority =
        serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "parse same-process settings authority {}: {error}",
                portable_path(authority_path)
            )
        })?;
    if authority.schema != SAME_PROCESS_SETTINGS_AUTHORITY_SCHEMA
        || authority.pid != pid
        || authority.process_creation_filetime != process_creation_filetime
    {
        return Err("same-process settings authority identity mismatch".into());
    }
    let exe_sha256 = report
        .exe_sha256
        .as_deref()
        .ok_or("same-process settings recovery lacks executable hash")?;
    if authority.exe_sha256 != exe_sha256 {
        return Err("same-process settings authority executable hash mismatch".into());
    }
    let user_dir = report
        .user_dir
        .as_ref()
        .ok_or("same-process settings recovery lacks user directory")?;
    if authority.user_dir != user_dir.display().to_string() {
        return Err("same-process settings authority user directory mismatch".into());
    }
    let settings_path = user_dir.join("settings.txt");
    let current_text = read_text_stable(&settings_path)
        .map_err(|error| format!("reread same-process settings authority: {error}"))?;
    let (current_language, current_display) = parse_settings_text(&current_text)
        .map_err(|error| format!("reparse same-process settings authority: {error}"))?;
    if current_language.as_deref() != Some(authority.localisation_language.as_str())
        || !same_display_semantics(&current_display, &authority.display_variant)
    {
        return Err("same-process settings authority current semantic settings mismatch".into());
    }
    report.settings.language = Some(authority.localisation_language);
    // THE ATTACH DIGEST FOLLOWS THE DISPLAY IT DESCRIBES. It means "the digest
    // of the read these display values came from", and the values now come from
    // the authority file rather than from this process's own attach read. the game
    // reorders and reformats settings.txt while meaning the same thing, so the
    // two digests genuinely differ on a semantic match -- which is the case
    // this recovery exists for. Leaving the newer digest in place published a
    // display whose digest described a different read, and every consumer that
    // reassembles the display from the attach digest refused the container.
    report.settings.attach_sha256 = Some(authority.display_variant.settings_file_sha256.clone());
    report.settings.display = Some(authority.display_variant);
    report.settings.authority = Some(SettingsAuthority::SameProcessAuthority);
    // The recovery upgraded the AUTHORITY, not the reading: the failures that
    // explain why the ordinary pre-launch proof was unavailable stay recorded.
    Ok(())
}

fn create_or_validate_same_process_settings_authority(
    authority_path: &Path,
    authority: &SameProcessSettingsAuthority,
) -> Result<(), String> {
    if authority_path.exists() {
        let bytes = fs::read(authority_path).map_err(|error| {
            format!(
                "read existing same-process settings authority {}: {error}",
                portable_path(authority_path)
            )
        })?;
        let existing: SameProcessSettingsAuthority =
            serde_json::from_slice(&bytes).map_err(|error| {
                format!(
                    "parse existing same-process settings authority {}: {error}",
                    portable_path(authority_path)
                )
            })?;
        if existing != *authority {
            return Err("existing same-process settings authority differs".into());
        }
        return Ok(());
    }
    let parent = authority_path
        .parent()
        .ok_or("same-process settings authority path has no parent")?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create same-process settings authority directory {}: {error}",
            parent.display()
        )
    })?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = match options.open(authority_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return create_or_validate_same_process_settings_authority(authority_path, authority)
        }
        Err(error) => {
            return Err(format!(
                "create same-process settings authority {}: {error}",
                portable_path(authority_path)
            ))
        }
    };
    serde_json::to_writer(&mut file, authority)
        .map_err(|error| format!("serialize same-process settings authority: {error}"))?;
    use std::io::Write as _;
    file.flush()
        .map_err(|error| format!("flush same-process settings authority: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync same-process settings authority: {error}"))
}

impl ModsetReport {
    pub fn as_json(&self) -> Value {
        let evidence = json!({
            "game": {
                "exe_path": self.exe_path.as_ref().map(|p| p.display().to_string()),
                "exe_size_bytes": self.exe_size_bytes,
                "exe_sha256": self.exe_sha256,
            },
            "pdx_user_dir": self.user_dir.as_ref().map(|p| p.display().to_string()),
            "user_dir_source": self.user_dir_source,
            "dlc_load_mtime_unix": self.dlc_load_mtime_unix,
            "dlc_load_sha256": self.dlc_load_sha256,
            "process_start_unix": self.process_start_unix,
            "enabled_mods": self.enabled_mods.iter().map(|m| json!({
                "descriptor_entry": m.descriptor_entry,
                "descriptor_path": m.descriptor_path.as_ref().map(|p| p.display().to_string()),
                "descriptor_sha256": m.descriptor_sha256,
                "name": m.name,
                "version": m.version,
                "remote_file_id": m.remote_file_id,
                "content_path": m.content_path,
                "content_sha256": m.content_sha256,
                "content_fingerprint": m.content_fingerprint.map(|(files, bytes, mtime)| json!({
                    "files": files, "bytes": bytes, "max_mtime_unix": mtime,
                })),
            })).collect::<Vec<_>>(),
            "dlc": {
                "installed": self.installed_dlcs,
                "disabled": self.disabled_dlcs,
                "enabled": self.enabled_dlcs,
            },
            "dlc_display_names": self.dlc_display_names,
            "dlc_descriptor_evidence": self.dlc_descriptor_evidence,
        });
        let errors = self
            .errors
            .iter()
            .map(ModsetError::to_string)
            .collect::<Vec<_>>();
        // The vocabulary is stated, and a reader that does not know it must
        // refuse rather than guess. `resolved` no longer answers "were the
        // settings proven" — that question moved to `settings_binding` — so a
        // v1 reader silently treating this as a v1 document would be reading a
        // stronger claim than the document makes.
        let mut doc = if self.content_resolved {
            let mut doc = json!({
                "schema": MODSET_BINDING_SCHEMA,
                "resolved": true,
                "errors": errors,
            });
            if let (Value::Object(doc_map), Value::Object(ev)) = (&mut doc, evidence) {
                for (k, v) in ev {
                    doc_map.insert(k, v);
                }
            }
            doc
        } else {
            // FAIL-CLOSED SHAPE (G2 finding 8): a naive consumer looking for
            // top-level enabled_mods/dlc on an UNRESOLVED report finds nothing
            // — partial evidence cannot be misread as a vanilla binding.
            json!({
                "schema": MODSET_BINDING_SCHEMA,
                "resolved": false,
                "errors": errors,
                "partial_evidence": evidence,
            })
        };
        doc.as_object_mut()
            .expect("modset binding is an object")
            .insert("settings_binding".to_string(), self.settings.as_json());
        doc
    }
}

/// The `modset` object's own vocabulary.
///
/// v2 split the settings binding out of `resolved`. The rename is deliberate
/// and the schema is checked where the document is re-read (fold-and-repack),
/// because in v1 `resolved:true` also asserted that the running process was
/// PROVEN to have read the recorded settings, and in v2 it does not.
pub const MODSET_BINDING_SCHEMA: &str = "hoi4_modset_binding_v2";

/// Resolve the recording modset for the attached game process. Never fails —
/// every problem lands in `errors` and clears `resolved`.
pub fn resolve_modset(pid: u32, user_dir_override: Option<&Path>) -> ModsetReport {
    resolve_modset_inner(pid, user_dir_override, None)
}

/// Resolve a process that is still held by this recorder's
/// `CREATE_SUSPENDED` authority. The caller has already selected both exact
/// paths and the command line contains this exact `-userdir`; requiring a
/// game-held log handle here would be impossible because no game instruction
/// has run yet. All file/directory stability and pre-process-time checks remain
/// identical to attached-process resolution.
pub(crate) fn resolve_suspended_launch_modset(
    pid: u32,
    expected_executable: &Path,
    exact_user_dir: &Path,
) -> ModsetReport {
    resolve_modset_inner(pid, Some(exact_user_dir), Some(expected_executable))
}

fn resolve_modset_inner(
    pid: u32,
    user_dir_override: Option<&Path>,
    suspended_expected_executable: Option<&Path>,
) -> ModsetReport {
    let mut report = ModsetReport {
        content_resolved: true,
        errors: Vec::new(),
        settings: SettingsBinding::default(),
        exe_path: None,
        exe_size_bytes: None,
        exe_sha256: None,
        user_dir: None,
        user_dir_source: None,
        dlc_load_mtime_unix: None,
        dlc_load_sha256: None,
        process_start_unix: None,
        process_start_filetime: None,
        enabled_mods: Vec::new(),
        installed_dlcs: Vec::new(),
        disabled_dlcs: Vec::new(),
        enabled_dlcs: Vec::new(),
        dlc_display_names: BTreeMap::new(),
        dlc_descriptor_evidence: BTreeMap::new(),
    };
    // A CONTENT resolution failure. Paths are spelled with `portable_path` so
    // the diagnostic can cross the container boundary — see `ModsetError`.
    macro_rules! fail {
        ($code:expr, $($arg:tt)*) => {{
            report.content_resolved = false;
            report.errors.push(ModsetError::new($code, format!($($arg)*)));
        }};
    }

    // --- game build binding ---
    match query_process_image_path(pid) {
        Ok(exe) => {
            if let Some(expected) = suspended_expected_executable {
                match (fs::canonicalize(&exe), fs::canonicalize(expected)) {
                    (Ok(observed), Ok(expected)) if observed == expected => {}
                    (Ok(observed), Ok(expected)) => fail!(
                        codes::SUSPENDED_EXE_MISMATCH,
                        "suspended process executable {} differs from selected {}",
                        portable_path(&observed),
                        portable_path(&expected)
                    ),
                    (observed, expected) => fail!(
                        codes::SUSPENDED_EXE_MISMATCH,
                        "canonicalize suspended executable binding failed: observed_ok={} expected_ok={}",
                        observed.is_ok(),
                        expected.is_ok()
                    ),
                }
            }
            match hash_file_streaming_stable(&exe) {
                Ok((size, sha)) => {
                    report.exe_size_bytes = Some(size);
                    report.exe_sha256 = Some(sha);
                }
                Err(e) => fail!(codes::EXE_HASH, "exe hash failed: {e}"),
            }
            report.exe_path = Some(exe);
        }
        Err(e) => fail!(codes::EXE_PATH_QUERY, "exe path query failed: {e}"),
    }
    let process_start: Option<SystemTime> = match query_process_start(pid) {
        Ok(start) => {
            report.process_start_unix = Some(system_time_unix(start));
            report.process_start_filetime = query_process_creation_filetime(pid).ok();
            Some(start)
        }
        Err(e) => {
            fail!(
                codes::PROCESS_START_QUERY,
                "process start time query failed: {e}"
            );
            None
        }
    };

    // --- Paradox user dir + dlc_load.json ---
    let located_user_dir = if suspended_expected_executable.is_some() {
        user_dir_override
            .ok_or_else(|| "suspended launch omitted its exact user directory".to_string())
            .and_then(|dir| {
                let dir = fs::canonicalize(dir)
                    .map_err(|e| format!("canonicalize suspended launch user dir: {e}"))?;
                if !dir.is_dir() || !dir.join("dlc_load.json").is_file() {
                    return Err(
                        "suspended launch user dir is not a directory with dlc_load.json".into(),
                    );
                }
                Ok((dir, "override"))
            })
    } else {
        locate_user_dir(pid, user_dir_override)
    };
    let user_dir_corroborated = match located_user_dir {
        Ok((dir, source)) => {
            // Discovery has already run the PID-held-log corroboration across
            // every physical candidate and required exactly one survivor.
            report.user_dir = Some(dir);
            report.user_dir_source = Some(source);
            true
        }
        Err(e) => {
            fail!(codes::USER_DIR, "{e}");
            false
        }
    };

    // --- settings binding (language + display) ---
    //
    // Evidence from the SAME user directory whose live log handle corroborates
    // dlc_load.json. A merely probed/overridden path is not sufficient
    // provenance for settings.txt.
    //
    // The mtime rule is a STATEMENT OF AUTHORITY, not an admission gate. It was
    // written to stop a THIRD PARTY's post-launch rewrite (a launcher, a
    // Workshop update) being read as the state the process launched with — the
    // same worry that still fails dlc_load.json closed below. But the game writes
    // settings.txt itself while loading, so on a game the player launched the
    // file is ALWAYS newer than the process, and refusing there took the whole
    // report down with it: the attach-and-play path could never resolve, never
    // prove content, and packaged none of its static artifacts. It also could
    // not seal, because the refusal's own text quoted the absolute path.
    //
    // So the read is admitted and the STRENGTH of what it proves is recorded.
    // Nothing here claims to know who wrote the file: `PostLaunchDiskObservation`
    // says only that this is what the disk held, read under the same stability
    // bracket. What binds it to the recording is the endpoint re-read at
    // capture close (`observe_settings_at_capture_close`) and, for the
    // language, the collector's independent refusal of any engine table whose
    // active language token disagrees.
    if !user_dir_corroborated {
        report.settings.fail(
            codes::SETTINGS_NO_USER_DIR,
            "settings.txt has no corroborated Paradox user directory; recorded null".to_string(),
        );
    } else if let Some(user_dir) = report.user_dir.as_deref() {
        match process_start {
            Some(start) => {
                let (authority, outcome) = match read_launch_settings(user_dir, start) {
                    Ok(read) => (SettingsAuthority::PreLaunchBracketed, Ok(read)),
                    Err(pre_launch_error) => match read_launch_settings_observed(user_dir) {
                        Ok(read) => (SettingsAuthority::PostLaunchDiskObservation, Ok(read)),
                        // Both failed: report the PRE-LAUNCH failure, which is
                        // the one that says why the strong proof is missing.
                        Err(_) => (
                            SettingsAuthority::PostLaunchDiskObservation,
                            Err(pre_launch_error),
                        ),
                    },
                };
                match outcome {
                    Ok((Some(language), display)) => {
                        report.settings.attach_sha256 = Some(display.settings_file_sha256.clone());
                        report.settings.language = Some(language);
                        report.settings.display = Some(display);
                        report.settings.authority = Some(authority);
                    }
                    Ok((None, display)) => {
                        report.settings.attach_sha256 = Some(display.settings_file_sha256.clone());
                        report.settings.fail(
                            codes::SETTINGS_LANGUAGE_ABSENT,
                            "settings.txt localisation language assignment is absent; \
                             recorded null"
                                .to_string(),
                        );
                    }
                    Err(e) => report.settings.fail(
                        codes::SETTINGS_UNREADABLE,
                        format!("settings.txt unreadable: {e}; recorded null"),
                    ),
                }
            }
            None => report.settings.fail(
                codes::SETTINGS_NO_PROCESS_START,
                "settings.txt launch binding unavailable without a process start time; \
                 recorded language/display as null"
                    .to_string(),
            ),
        }
    }

    let mut disabled_dlcs: Vec<String> = Vec::new();
    if let Some(user_dir) = report.user_dir.clone() {
        let dlc_load_path = user_dir.join("dlc_load.json");
        // Stability bracket (re-gate finding 4): capture mtime, read, re-check.
        let mtime_before = fs::metadata(&dlc_load_path).and_then(|m| m.modified()).ok();
        report.dlc_load_mtime_unix = mtime_before.map(system_time_unix);
        // Launcher-state provenance (re-gate finding 3+4): FULL-PRECISION
        // compare; an absent mtime or a config at/after process start both
        // fail closed (can't prove the running game used this config).
        match (mtime_before, process_start) {
            (Some(mtime), Some(start)) if mtime >= start => fail!(
                codes::DLC_LOAD_MTIME_AT_OR_AFTER_START,
                "dlc_load.json mtime is at/after the game process start; \
                 launcher state not provably the launched state"
            ),
            (None, _) => {
                fail!(
                    codes::DLC_LOAD_MTIME_UNAVAILABLE,
                    "dlc_load.json mtime unavailable; launcher-state provenance unprovable"
                )
            }
            _ => {}
        }
        match fs::read_to_string(&dlc_load_path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(doc) => {
                    // Re-check mtime AFTER the read: a launcher rewrite between
                    // the metadata read and the content read would otherwise
                    // pair new content with the old, accepted timestamp.
                    let mtime_after = fs::metadata(&dlc_load_path).and_then(|m| m.modified()).ok();
                    if mtime_after != mtime_before {
                        fail!(
                            codes::DLC_LOAD_UNSTABLE,
                            "dlc_load.json changed while being read (mtime moved)"
                        );
                    }
                    // G2-regen finding 5: an atomic replacement can PRESERVE
                    // the mtime (OneDrive sync, copied-file install), so the
                    // mtime bracket alone is insufficient. Read the file a
                    // second time and require byte-identical content. Residual
                    // (documented): a replacement completing before the FIRST
                    // read with a preserved mtime is indistinguishable from
                    // user mode; both reads then see the same (new) bytes.
                    match fs::read_to_string(&dlc_load_path) {
                        Ok(text_again) if text_again == text => {
                            report.dlc_load_sha256 = Some(Sha256::hex(text.as_bytes()));
                        }
                        Ok(_) => fail!(
                            codes::DLC_LOAD_UNSTABLE,
                            "dlc_load.json changed while being read (bytes differ between reads)"
                        ),
                        Err(e) => fail!(
                            codes::DLC_LOAD_READ,
                            "dlc_load.json second stability read failed: {e}"
                        ),
                    }
                    // Duplicate top-level keys (re-gate finding 2): serde_json
                    // silently keeps the last, so a lenient document can hide
                    // an ambiguous enabled_mods/disabled_dlcs behind an empty
                    // override. Reject any duplicated object key at depth 1.
                    if let Err(dup) = reject_duplicate_top_level_keys(&text) {
                        fail!(
                            codes::DLC_LOAD_DUPLICATE_KEYS,
                            "dlc_load.json top-level key scan rejected: {dup}"
                        );
                    }
                    // G2 finding 2: absent or mistyped fields are NOT empty
                    // lists — they are unproven state and fail closed.
                    match required_string_array(&doc, "enabled_mods") {
                        Ok(enabled_mods) => {
                            for entry in enabled_mods {
                                let resolved = resolve_mod_descriptor(&user_dir, &entry);
                                if resolved.descriptor_sha256.is_none() {
                                    fail!(
                                        codes::MOD_DESCRIPTOR_UNREADABLE,
                                        "mod descriptor unreadable: {entry:?}"
                                    );
                                } else if resolved.content_location_conflict {
                                    // G2-regen finding 6: repeated or competing
                                    // path=/archive= — the effective content
                                    // location is unproven engine semantics.
                                    fail!(
                                        codes::MOD_CONTENT_LOCATION_AMBIGUOUS,
                                        "mod descriptor content location ambiguous \
                                         (repeated or competing path=/archive=): {entry:?}"
                                    );
                                } else if resolved.content_path.is_none() {
                                    fail!(
                                        codes::MOD_CONTENT_LOCATION_ABSENT,
                                        "mod descriptor carries no path=/archive= \
                                         (binding incomplete): {entry:?}"
                                    );
                                } else if resolved.content_sha256.is_none() {
                                    fail!(
                                        codes::MOD_CONTENT_NOT_HASHABLE,
                                        "mod content not hashable: {entry:?} -> {:?}",
                                        resolved.content_path
                                    );
                                }
                                // G2-regen finding 4: descriptor or content
                                // written AT/AFTER process start means the disk
                                // no longer proves what the running process
                                // loaded (Workshop update post-launch).
                                if let Some(start) = process_start {
                                    match resolved.descriptor_mtime {
                                        Some(m) if m >= start => fail!(
                                            codes::MOD_DESCRIPTOR_AT_OR_AFTER_START,
                                            "mod descriptor written at/after process start \
                                             (post-launch update?): {entry:?}"
                                        ),
                                        None if resolved.descriptor_sha256.is_some() => {
                                            fail!(
                                                codes::MOD_DESCRIPTOR_MTIME_UNAVAILABLE,
                                                "mod descriptor mtime unavailable: {entry:?}"
                                            )
                                        }
                                        _ => {}
                                    }
                                    match resolved.content_max_mtime {
                                        Some(m) if m >= start => fail!(
                                            codes::MOD_CONTENT_AT_OR_AFTER_START,
                                            "mod content written at/after process start \
                                             (post-launch update?): {entry:?}"
                                        ),
                                        _ => {}
                                    }
                                }
                                report.enabled_mods.push(resolved);
                            }
                        }
                        Err(e) => fail!(codes::DLC_LOAD_ENABLED_MODS, "dlc_load.json enabled_mods: {e}"),
                    }
                    match required_string_array(&doc, "disabled_dlcs") {
                        Ok(list) => disabled_dlcs = list,
                        Err(e) => fail!(codes::DLC_LOAD_DISABLED_DLCS, "dlc_load.json disabled_dlcs: {e}"),
                    }
                }
                Err(e) => fail!(codes::DLC_LOAD_PARSE, "dlc_load.json parse failed: {e}"),
            },
            Err(e) => fail!(
                codes::DLC_LOAD_READ,
                "dlc_load.json read failed at {}: {e}",
                portable_path(&dlc_load_path)
            ),
        }
    }
    report.disabled_dlcs = disabled_dlcs.clone();

    // --- installed / enabled DLC census (game_root/dlc/<dir>/<name>.dlc) ---
    if let Some(exe) = report.exe_path.clone() {
        match exe.parent() {
            Some(game_root) => match census_installed_dlcs(game_root) {
                Ok((installed, census_max_mtime)) => {
                    // Modset re-gate finding 2 (empty-census arm): a real the game
                    // install always carries dlc/ descriptors; an EMPTY census
                    // (dlc/ removed/emptied — possibly post-launch) has no
                    // timestamp to order against process start and would
                    // resolve with vanilla-looking empty DLC arrays. Fail
                    // closed. Stability: a second census must reproduce the
                    // first exactly (same identity rule as dlc_load.json's
                    // double read).
                    if installed.is_empty() || census_max_mtime.is_none() {
                        fail!(
                            codes::DLC_CENSUS_EMPTY,
                            "installed-DLC census is empty/timestampless; cannot bind \
                             the DLC set the process loaded"
                        );
                    }
                    // G2-regen finding 4 (DLC arm): an installed-DLC descriptor
                    // written at/after process start means the census describes
                    // a different DLC set than the one the process loaded.
                    if let (Some(start), Some(newest)) = (process_start, census_max_mtime) {
                        if newest >= start {
                            fail!(
                                codes::DLC_CENSUS_AT_OR_AFTER_START,
                                "installed-DLC census changed at/after process start \
                                 (post-launch install/update?)"
                            );
                        }
                    }
                    // G2 finding 6: every disabled key must match an installed
                    // census key EXACTLY, else the subtraction is unproven
                    // (format/case drift could silently re-enable a DLC).
                    for disabled in &disabled_dlcs {
                        if !installed.iter().any(|rel| rel == disabled) {
                            fail!(
                                codes::DLC_DISABLED_UNMATCHED,
                                "disabled_dlcs entry matches no installed DLC \
                                 (format drift?): {disabled:?}"
                            );
                        }
                    }
                    report.enabled_dlcs = installed
                        .iter()
                        .filter(|rel| !disabled_dlcs.iter().any(|d| d == *rel))
                        .cloned()
                        .collect();
                    // Display names are read INSIDE the census bracket. Each
                    // descriptor is also double-read with a metadata bracket
                    // and must predate launch.
                    let (display_names, descriptor_evidence, display_name_errors) =
                        resolve_dlc_display_names(game_root, &installed, process_start);
                    report.dlc_display_names = display_names;
                    report.dlc_descriptor_evidence = descriptor_evidence;
                    for error in display_name_errors {
                        fail!(codes::DLC_DISPLAY_NAME, "{error}");
                    }
                    match census_installed_dlcs(game_root) {
                        Ok((again, again_mtime))
                            if again == installed && again_mtime == census_max_mtime => {}
                        Ok(_) => fail!(
                            codes::DLC_CENSUS_UNSTABLE,
                            "installed-DLC census changed while display names were being read \
                             (unstable between passes)"
                        ),
                        Err(e) => fail!(codes::DLC_CENSUS_UNSTABLE, "dlc census stability pass failed: {e}"),
                    }
                    report.installed_dlcs = installed;
                }
                Err(e) => fail!(codes::DLC_CENSUS, "dlc census failed: {e}"),
            },
            None => fail!(codes::EXE_NO_PARENT, "exe path has no parent directory"),
        }
    }

    report
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: SystemTime,
}

fn file_stamp(path: &Path) -> Result<FileStamp, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("{} metadata: {e}", portable_path(path)))?;
    let modified = metadata
        .modified()
        .map_err(|e| format!("{} mtime: {e}", portable_path(path)))?;
    Ok(FileStamp {
        len: metadata.len(),
        modified,
    })
}

fn validate_stable_text_read(
    path: &Path,
    process_start: SystemTime,
    before: FileStamp,
    first: Vec<u8>,
    second: Vec<u8>,
    after: FileStamp,
) -> Result<String, String> {
    if before.modified >= process_start {
        return Err(format!(
            "{} mtime is at/after game process start",
            portable_path(path)
        ));
    }
    if after != before {
        return Err(format!(
            "{} changed while being read (metadata moved)",
            portable_path(path)
        ));
    }
    if first != second {
        return Err(format!(
            "{} changed while being read (bytes differ between reads)",
            portable_path(path)
        ));
    }
    if first.len() as u64 != before.len {
        return Err(format!(
            "{} read length does not match bracketed metadata",
            portable_path(path)
        ));
    }
    String::from_utf8(first).map_err(|e| format!("{} is not UTF-8: {e}", portable_path(path)))
}

/// Read one launch-bound text file with both a metadata stability bracket and
/// two byte-identical reads. Equality with process start fails closed because
/// it cannot prove that the running process saw these bytes.
fn read_text_stable_before_start(path: &Path, process_start: SystemTime) -> Result<String, String> {
    let bytes = read_bytes_stable_before_start(path, process_start)?;
    String::from_utf8(bytes).map_err(|e| format!("{} is not UTF-8: {e}", portable_path(path)))
}

fn read_text_stable(path: &Path) -> Result<String, String> {
    let before = file_stamp(path)?;
    let first = fs::read(path).map_err(|e| format!("{} first read: {e}", portable_path(path)))?;
    let second = fs::read(path).map_err(|e| format!("{} second read: {e}", portable_path(path)))?;
    let after = file_stamp(path)?;
    if after != before {
        return Err(format!(
            "{} changed while being read (metadata moved)",
            portable_path(path)
        ));
    }
    if first != second {
        return Err(format!(
            "{} changed while being read (bytes differ between reads)",
            portable_path(path)
        ));
    }
    if first.len() as u64 != before.len {
        return Err(format!(
            "{} read length does not match bracketed metadata",
            portable_path(path)
        ));
    }
    String::from_utf8(first).map_err(|e| format!("{} is not UTF-8: {e}", portable_path(path)))
}

/// Return the exact bytes from the same pre-start, metadata-bracketed,
/// byte-identical read used for descriptor parsing and hashing.
fn read_bytes_stable_before_start(
    path: &Path,
    process_start: SystemTime,
) -> Result<Vec<u8>, String> {
    let before = file_stamp(path)?;
    let first = fs::read(path).map_err(|e| format!("{} first read: {e}", portable_path(path)))?;
    let second = fs::read(path).map_err(|e| format!("{} second read: {e}", portable_path(path)))?;
    let after = file_stamp(path)?;
    if before.modified >= process_start {
        return Err(format!(
            "{} mtime is at/after game process start",
            portable_path(path)
        ));
    }
    if after != before {
        return Err(format!(
            "{} changed while being read (metadata moved)",
            portable_path(path)
        ));
    }
    if first != second {
        return Err(format!(
            "{} changed while being read (bytes differ between reads)",
            portable_path(path)
        ));
    }
    if first.len() as u64 != before.len {
        return Err(format!(
            "{} read length does not match bracketed metadata",
            portable_path(path)
        ));
    }
    Ok(first)
}

/// Read all launch-bound variant inputs from one byte-identical settings.txt
/// snapshot. This prevents language and display metadata from being bound to
/// different file generations.
fn read_launch_settings(
    user_dir: &Path,
    process_start: SystemTime,
) -> Result<(Option<String>, DisplayVariantSettings), String> {
    let path = user_dir.join("settings.txt");
    let text = read_text_stable_before_start(&path, process_start)?;
    parse_settings_text(&text)
}

/// The same read WITHOUT the pre-process-start requirement.
///
/// Every other guarantee is identical — same metadata bracket, same two
/// byte-identical reads, same strict parse — so what this drops is exactly one
/// claim: that the running process is proven to have read these bytes. The
/// caller records that as [`SettingsAuthority::PostLaunchDiskObservation`] and
/// never as a launch proof.
///
/// It exists because the game rewrites `settings.txt` while it loads, which
/// makes the pre-start rule unsatisfiable on a game the player launched — and
/// the recorder attaching to a game the player launched is the product.
fn read_launch_settings_observed(
    user_dir: &Path,
) -> Result<(Option<String>, DisplayVariantSettings), String> {
    let path = user_dir.join("settings.txt");
    let text = read_text_stable(&path)?;
    parse_settings_text(&text)
}

/// Take the closing observation of `settings.txt`, while the recorded process
/// identity is still live.
///
/// WHAT THIS PROVES, EXACTLY: the settings read when the recorder attached and
/// the settings on disk as the capture window closed say the same thing
/// SEMANTICALLY. It does NOT prove the values held for the whole window — a
/// change made and undone between the two reads passes it — which is why the
/// recorded field is named `endpoint_semantics_equal` and not "stable".
///
/// It must be called while the game is still the process the report resolved
/// against. After the game exits, an exit-time rewrite would be compared as
/// though it described the recording, and the PID could even have been reused.
/// A dead or replaced process identity is recorded as a failure, never as a
/// disagreement.
pub fn observe_settings_at_capture_close(report: &mut ModsetReport, pid: u32) {
    let Some(attach_display) = report.settings.display.clone() else {
        return;
    };
    let Some(user_dir) = report.user_dir.clone() else {
        return;
    };
    // FULL-PRECISION identity. A whole-second comparison accepts a PID reused
    // by a process created inside the same second, and would then compare
    // another process's settings against this recording's.
    match (
        query_process_creation_filetime(pid),
        report.process_start_filetime,
    ) {
        (Ok(observed), Some(recorded)) if observed == recorded => {}
        _ => {
            report.settings.errors.push(ModsetError::new(
                codes::SETTINGS_ENDPOINT_PROCESS_GONE,
                "settings.txt closing observation skipped: the recorded process identity is \
                 no longer live, so a second read could describe another process"
                    .to_string(),
            ));
            return;
        }
    }
    match read_launch_settings_observed(&user_dir) {
        Ok((language, display)) => {
            report.settings.seal_sha256 = Some(display.settings_file_sha256.clone());
            report.settings.endpoint_semantics_equal = Some(
                language.as_deref() == report.settings.language.as_deref()
                    && same_display_semantics(&display, &attach_display),
            );
        }
        Err(e) => report.settings.errors.push(ModsetError::new(
            codes::SETTINGS_ENDPOINT_UNREADABLE,
            format!("settings.txt closing observation failed: {e}"),
        )),
    }
}

fn parse_settings_text(text: &str) -> Result<(Option<String>, DisplayVariantSettings), String> {
    let language = clausewitz_string_field(&text, "language")?;
    let graphics = clausewitz_object_field(&text, "graphics")
        .map_err(|e| format!("graphics object: {e}"))?
        .ok_or("settings.txt graphics object is absent")?;
    let size = clausewitz_object_field(graphics, "size")
        .map_err(|e| format!("graphics.size object: {e}"))?
        .ok_or("settings.txt graphics.size object is absent")?;
    let min_gui = clausewitz_object_field(graphics, "min_gui")
        .map_err(|e| format!("graphics.min_gui object: {e}"))?
        .ok_or("settings.txt graphics.min_gui object is absent")?;
    let physical_width = required_u32(size, "x", "graphics.size.x")?;
    let physical_height = required_u32(size, "y", "graphics.size.y")?;
    let min_gui_width = required_u32(min_gui, "x", "graphics.min_gui.x")?;
    let min_gui_height = required_u32(min_gui, "y", "graphics.min_gui.y")?;
    if physical_width == 0 || physical_height == 0 || min_gui_width == 0 || min_gui_height == 0 {
        return Err("settings.txt display extents must be nonzero".into());
    }
    let scale_text = clausewitz_atom_field(graphics, "gui_scale")?
        .ok_or("settings.txt graphics.gui_scale is absent")?;
    let scale: f32 = scale_text
        .parse()
        .map_err(|e| format!("settings.txt graphics.gui_scale is not f32: {e}"))?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("settings.txt graphics.gui_scale is not finite and positive".into());
    }
    let refresh_rate_hz = required_u32(graphics, "refreshRate", "graphics.refreshRate")?;
    if refresh_rate_hz == 0 {
        return Err("settings.txt graphics.refreshRate must be nonzero".into());
    }
    let max_refresh_rate_hz = clausewitz_atom_field(graphics, "max_refresh_rate")?
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|e| format!("settings.txt graphics.max_refresh_rate is not u32: {e}"))
        })
        .transpose()?;
    let fullscreen = required_yes_no(graphics, "fullScreen", "graphics.fullScreen")?;
    let borderless = required_yes_no(graphics, "borderless", "graphics.borderless")?;
    let display_mode = match (fullscreen, borderless) {
        (true, false) => "fullscreen",
        (false, true) => "borderless_fullscreen",
        (false, false) => "windowed",
        (true, true) => {
            return Err(
                "settings.txt graphics fullScreen=yes and borderless=yes is ambiguous".into(),
            )
        }
    };
    Ok((
        language,
        DisplayVariantSettings {
            physical_width,
            physical_height,
            min_gui_width,
            min_gui_height,
            gui_scale_f32_bits: scale.to_bits(),
            refresh_rate_hz,
            max_refresh_rate_hz,
            fullscreen,
            borderless,
            display_mode: display_mode.to_string(),
            settings_file_sha256: Sha256::hex(text.as_bytes()),
        },
    ))
}

fn same_display_semantics(
    left: &DisplayVariantSettings,
    right: &DisplayVariantSettings,
) -> bool {
    left.physical_width == right.physical_width
        && left.physical_height == right.physical_height
        && left.min_gui_width == right.min_gui_width
        && left.min_gui_height == right.min_gui_height
        && left.gui_scale_f32_bits == right.gui_scale_f32_bits
        && left.refresh_rate_hz == right.refresh_rate_hz
        && left.max_refresh_rate_hz == right.max_refresh_rate_hz
        && left.fullscreen == right.fullscreen
        && left.borderless == right.borderless
        && left.display_mode == right.display_mode
}

fn required_u32(text: &str, key: &str, context: &str) -> Result<u32, String> {
    clausewitz_atom_field(text, key)?
        .ok_or_else(|| format!("settings.txt {context} is absent"))?
        .parse::<u32>()
        .map_err(|e| format!("settings.txt {context} is not u32: {e}"))
}

fn required_yes_no(text: &str, key: &str, context: &str) -> Result<bool, String> {
    match clausewitz_atom_field(text, key)?
        .ok_or_else(|| format!("settings.txt {context} is absent"))?
    {
        "yes" => Ok(true),
        "no" => Ok(false),
        other => Err(format!(
            "settings.txt {context} is neither yes nor no: {other:?}"
        )),
    }
}

/// Return a unique object assignment's interior. The scanner tracks comments,
/// quoted strings and brace nesting, so same-named keys in nested objects do
/// not satisfy the requested scope.
fn clausewitz_object_field<'a>(text: &'a str, wanted: &str) -> Result<Option<&'a str>, String> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut found = None;
    while index < bytes.len() {
        skip_space_and_comments(bytes, &mut index);
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.checked_sub(1).ok_or("unbalanced closing brace")?;
                index += 1;
            }
            b'"' => skip_quoted(bytes, &mut index)?,
            byte if is_key_byte(byte) => {
                let start = index;
                while index < bytes.len() && is_key_byte(bytes[index]) {
                    index += 1;
                }
                let key = &text[start..index];
                let mut cursor = index;
                skip_space_and_comments(bytes, &mut cursor);
                if cursor >= bytes.len() || bytes[cursor] != b'=' {
                    index = cursor;
                    continue;
                }
                cursor += 1;
                skip_space_and_comments(bytes, &mut cursor);
                if depth == 0 && key == wanted {
                    if found.is_some() {
                        return Err(format!("duplicate {wanted}= object assignment"));
                    }
                    if cursor >= bytes.len() || bytes[cursor] != b'{' {
                        return Err(format!("{wanted}= value is not an object"));
                    }
                    let open = cursor;
                    let close = matching_brace(bytes, open)?;
                    found = Some(&text[open + 1..close]);
                    index = close + 1;
                } else {
                    index = cursor;
                }
            }
            _ => index += 1,
        }
    }
    if depth != 0 {
        return Err("unbalanced object brace".into());
    }
    Ok(found)
}

fn clausewitz_atom_field<'a>(text: &'a str, wanted: &str) -> Result<Option<&'a str>, String> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut found = None;
    while index < bytes.len() {
        skip_space_and_comments(bytes, &mut index);
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.checked_sub(1).ok_or("unbalanced closing brace")?;
                index += 1;
            }
            b'"' => skip_quoted(bytes, &mut index)?,
            byte if is_key_byte(byte) => {
                let start = index;
                while index < bytes.len() && is_key_byte(bytes[index]) {
                    index += 1;
                }
                let key = &text[start..index];
                let mut cursor = index;
                skip_space_and_comments(bytes, &mut cursor);
                if cursor >= bytes.len() || bytes[cursor] != b'=' {
                    index = cursor;
                    continue;
                }
                cursor += 1;
                skip_space_and_comments(bytes, &mut cursor);
                if depth == 0 && key == wanted {
                    if found.is_some() {
                        return Err(format!("duplicate {wanted}= assignment"));
                    }
                    let value_start = cursor;
                    while cursor < bytes.len()
                        && !bytes[cursor].is_ascii_whitespace()
                        && !matches!(bytes[cursor], b'{' | b'}' | b'#' | b'=')
                    {
                        cursor += 1;
                    }
                    if cursor == value_start {
                        return Err(format!("{wanted}= has no atom value"));
                    }
                    found = Some(&text[value_start..cursor]);
                    index = cursor;
                } else {
                    index = cursor;
                }
            }
            _ => index += 1,
        }
    }
    if depth != 0 {
        return Err("unbalanced object brace".into());
    }
    Ok(found)
}

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
}

fn skip_space_and_comments(bytes: &[u8], index: &mut usize) {
    loop {
        while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
            *index += 1;
        }
        if *index < bytes.len() && bytes[*index] == b'#' {
            while *index < bytes.len() && bytes[*index] != b'\n' {
                *index += 1;
            }
        } else {
            break;
        }
    }
}

fn skip_quoted(bytes: &[u8], index: &mut usize) -> Result<(), String> {
    *index += 1;
    while *index < bytes.len() {
        match bytes[*index] {
            b'\\' => *index += 2,
            b'"' => {
                *index += 1;
                return Ok(());
            }
            _ => *index += 1,
        }
    }
    Err("unbalanced quoted string".into())
}

fn matching_brace(bytes: &[u8], open: usize) -> Result<usize, String> {
    let mut index = open + 1;
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'"' => skip_quoted(bytes, &mut index)?,
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    Err("unbalanced object brace".into())
}

/// Parse a single line-oriented Clausewitz `key = "value"` assignment.
/// Duplicate target keys and any malformed target assignment are ambiguous and
/// fail closed. Only explicit quoted-string escapes are accepted.
fn clausewitz_string_field(text: &str, wanted: &str) -> Result<Option<String>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut found: Option<String> = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        if key.trim() != wanted {
            continue;
        }
        if found.is_some() {
            return Err(format!("duplicate {wanted}= assignment"));
        }
        let raw = value.trim();
        let Some(body) = raw.strip_prefix('"') else {
            return Err(format!("{wanted}= value is not a quoted string"));
        };
        let mut parsed = String::new();
        let mut chars = body.char_indices();
        let mut closing = None;
        while let Some((offset, ch)) = chars.next() {
            match ch {
                '"' => {
                    closing = Some(offset);
                    break;
                }
                '\\' => {
                    let Some((_, escaped)) = chars.next() else {
                        return Err(format!("{wanted}= has a trailing escape"));
                    };
                    parsed.push(match escaped {
                        '\\' => '\\',
                        '"' => '"',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => return Err(format!("{wanted}= has malformed escape \\{escaped}")),
                    });
                }
                _ => parsed.push(ch),
            }
        }
        let Some(closing) = closing else {
            return Err(format!("{wanted}= has an unbalanced quoted string"));
        };
        let trailing = body[closing + 1..].trim();
        if !trailing.is_empty() && !trailing.starts_with('#') {
            return Err(format!(
                "{wanted}= has trailing tokens after the quoted string"
            ));
        }
        if parsed.trim().is_empty() {
            return Err(format!("{wanted}= value is whitespace-only"));
        }
        found = Some(parsed);
    }
    Ok(found)
}

/// Resolve display names without inventing a fallback from directory/file
/// names. Every installed path gets a map entry; unreadable or nameless
/// descriptors carry null plus a fail-closed resolution note.
fn resolve_dlc_display_names(
    game_root: &Path,
    installed: &[String],
    process_start: Option<SystemTime>,
) -> (
    BTreeMap<String, Option<String>>,
    BTreeMap<String, Option<DlcDescriptorEvidence>>,
    Vec<String>,
) {
    let mut names = BTreeMap::new();
    let mut evidence = BTreeMap::new();
    let mut errors = Vec::new();
    for relative in installed {
        let path = game_root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let row = match process_start {
            None => {
                errors.push(format!(
                    "DLC descriptor launch binding unavailable: {relative:?}; recorded null"
                ));
                None
            }
            Some(start) => match read_bytes_stable_before_start(&path, start) {
                Ok(bytes) => {
                    let text = bytes
                        .iter()
                        .map(|byte| char::from(*byte))
                        .collect::<String>();
                    match clausewitz_string_field(&text, "name") {
                        Ok(Some(name)) => Some(DlcDescriptorEvidence {
                            display_name: name,
                            descriptor_file_sha256: Sha256::hex(&bytes),
                        }),
                        Ok(None) => {
                            errors.push(format!(
                            "DLC descriptor has no name= display value: {relative:?}; recorded null"
                        ));
                            None
                        }
                        Err(e) => {
                            errors.push(format!(
                            "DLC descriptor display name malformed: {relative:?}: {e}; recorded null"
                        ));
                            None
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!(
                        "DLC descriptor display name not launch-bound: {relative:?}: {e}; recorded null"
                    ));
                    None
                }
            },
        };
        names.insert(
            relative.clone(),
            row.as_ref().map(|value| value.display_name.clone()),
        );
        evidence.insert(relative.clone(), row);
    }
    (names, evidence, errors)
}

/// A field that MUST exist as an array of strings. Anything else is unproven
/// launcher state, not an empty list.
fn required_string_array(doc: &Value, key: &str) -> Result<Vec<String>, String> {
    let field = doc
        .get(key)
        .ok_or_else(|| format!("field {key:?} absent (unproven, not empty)"))?;
    let entries = field
        .as_array()
        .ok_or_else(|| format!("field {key:?} is not an array"))?;
    entries
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("field {key:?} carries a non-string entry"))
        })
        .collect()
}

/// The final component of a live process's executable image.
///
/// Exposed because "is that pid one of US" is a question outside this module —
/// crash recovery has to distinguish a finalizer still working in a run
/// directory from any other process that happens to hold the same pid — and it
/// must be answered with the same call the modset binding already trusts rather
/// than a second opinion.
pub fn process_image_file_name(pid: u32) -> Result<String, String> {
    let image = query_process_image_path(pid)?;
    image
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| format!("process {pid} image path has no file name"))
}

fn query_process_image_path(pid: u32) -> Result<PathBuf, String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return Err(format!("OpenProcess({pid}) failed"));
        }
        let mut buf = vec![0u16; 32 * 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return Err(format!("QueryFullProcessImageNameW({pid}) failed"));
        }
        Ok(PathBuf::from(String::from_utf16_lossy(
            &buf[..len as usize],
        )))
    }
}

/// Process creation time as a full-precision `SystemTime` (FILETIME 100ns
/// ticks -> unix epoch). Full precision matters: the launcher-state ordering
/// check must not floor two sub-second-apart events into one equal second
/// (re-gate finding 3).
/// The process creation FILETIME exactly as Windows reports it — 100ns ticks,
/// no floor, no epoch conversion.
///
/// This is the process IDENTITY half of (pid, creation time). Anything that
/// asks "is that still the same process" must compare this, not a whole-second
/// stamp: PIDs are reused, and a reuse inside the same second is exactly the
/// case a floored comparison cannot see.
pub fn query_process_creation_filetime(pid: u32) -> Result<u64, String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return Err(format!("OpenProcess({pid}) failed"));
        }
        let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 {
            return Err(format!("GetProcessTimes({pid}) failed"));
        }
        Ok(creation)
    }
}

fn query_process_start(pid: u32) -> Result<SystemTime, String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return Err(format!("OpenProcess({pid}) failed"));
        }
        let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 {
            return Err(format!("GetProcessTimes({pid}) failed"));
        }
        // FILETIME: 100ns ticks since 1601-01-01. Unix epoch offset in ticks:
        const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
        if creation < UNIX_EPOCH_TICKS {
            return Err("process creation time precedes the unix epoch".to_string());
        }
        let nanos = (creation - UNIX_EPOCH_TICKS).saturating_mul(100);
        Ok(SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(nanos))
    }
}

/// Whole-second unix stamp of a `SystemTime`, for human-readable manifest
/// fields ONLY — never for the ordering comparison (that uses full precision).
fn system_time_unix(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Reject a JSON document that repeats any object key at the TOP level.
/// `serde_json` silently keeps the last value for a duplicate key, so a
/// lenient `{"enabled_mods":[...],"enabled_mods":[]}` could resolve as an
/// empty (vanilla-looking) modset. A minimal depth-aware, string-aware scan
/// collects depth-1 keys and fails on the first repeat. Input is already
/// proven valid JSON by the caller's parse.
fn reject_duplicate_top_level_keys(text: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut expecting_key = false;
    let mut seen: Vec<String> = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            // Non-key strings (values, nested keys) are skipped whole here;
            // top-level keys are captured at the opening-quote branch below.
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                // String opening. If at depth 1 and a key is expected, read it
                // whole here (escape-aware) so we can record it.
                if depth == 1 && expecting_key {
                    let start = i + 1;
                    let mut j = start;
                    let mut esc = false;
                    while j < bytes.len() {
                        match bytes[j] {
                            b'\\' if !esc => esc = true,
                            b'"' if !esc => break,
                            _ => esc = false,
                        }
                        j += 1;
                    }
                    if j >= bytes.len() {
                        return Ok(()); // malformed; caller's parse already vouched, be lenient
                    }
                    // G2-regen finding 1: compare DECODED key names. serde_json
                    // decodes backslash-u escapes inside key names and keeps
                    // the last value, so a raw-slice comparison lets an escaped
                    // duplicate smuggle an empty (vanilla-looking) override
                    // past the scanner. An undecodable key fails closed.
                    let key = decode_json_string(&text[start..j])
                        .map_err(|e| format!("undecodable key {:?}: {e}", &text[start..j]))?;
                    if seen.iter().any(|k| k == &key) {
                        return Err(format!("{key:?}"));
                    }
                    seen.push(key);
                    expecting_key = false;
                    i = j + 1;
                    continue;
                }
                in_string = true;
            }
            b'{' => {
                depth += 1;
                if depth == 1 {
                    expecting_key = true;
                }
            }
            b'}' => depth -= 1,
            b'[' => depth += 1,
            b']' => depth -= 1,
            b',' if depth == 1 => expecting_key = true,
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

/// Decode a raw JSON string body (the text between the quotes) to its actual
/// character content: `\uXXXX` (incl. surrogate pairs) and the short escapes.
/// Duplicate-key detection MUST compare decoded names (G2-regen finding 1) —
/// serde_json compares decoded names when it keeps-last.
fn decode_json_string(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let esc = chars.next().ok_or("dangling escape")?;
        match esc {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{0008}'),
            'f' => out.push('\u{000C}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let read_u16 = |chars: &mut std::str::Chars| -> Result<u32, String> {
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() != 4 {
                        return Err("truncated \\u escape".to_string());
                    }
                    u32::from_str_radix(&hex, 16).map_err(|e| format!("bad \\u escape: {e}"))
                };
                let first = read_u16(&mut chars)?;
                let code = if (0xD800..0xDC00).contains(&first) {
                    // High surrogate: the low half must follow as `\uXXXX`.
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err("unpaired high surrogate".to_string());
                    }
                    let second = read_u16(&mut chars)?;
                    if !(0xDC00..0xE000).contains(&second) {
                        return Err("invalid low surrogate".to_string());
                    }
                    0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                } else if (0xDC00..0xE000).contains(&first) {
                    return Err("unpaired low surrogate".to_string());
                } else {
                    first
                };
                out.push(char::from_u32(code).ok_or("invalid code point")?);
            }
            other => return Err(format!("unknown escape \\{other}")),
        }
    }
    Ok(out)
}

/// Corroborate that the chosen Paradox user dir is the one the ATTACHED
/// process actually uses (G2-regen finding 2): a running the game writes its logs
/// under `<user_dir>/logs/` continuously, so a directory whose newest log
/// predates process start was NOT written by this process — a stale override
/// or a stale probed default while the game runs elsewhere. Fail closed on a
/// missing/unreadable/never-written logs dir. Bounded residual (documented):
/// a SECOND simultaneously-running the game instance sharing the same user dir
/// would corroborate it too — but a shared dir shares dlc_load.json, so the
/// binding evidence is the same either way.
fn corroborate_user_dir_live(user_dir: &Path, pid: u32) -> Result<(), String> {
    // Modset re-gate finding 1: mtime liveness was NOT ownership — ANY writer
    // touching logs/ post-launch (OneDrive, another app, the user) passed the
    // old check for a wrong-but-well-formed directory. Ownership proof: the
    // attached PID must HOLD an open handle to a file under <user_dir>/logs
    // (the game keeps game.log/error.log open for the whole session —
    // live-verified 2026-07-12: exclusive open fails on both while the game
    // runs). Restart Manager names the holding PIDs per file. No held file =
    // this is not the attached process's user dir = fail closed. RM failures
    // fail closed too (never downgrade to the weaker mtime signal).
    let logs = user_dir.join("logs");
    // Fix-round re-gate finding 1 (junction bypass): `logs` — or a file
    // inside it — could be a junction/symlink into the REAL user dir's logs.
    // RM would then truthfully name the attached PID on the TARGET and the
    // proof would corroborate a WRONG directory holding a copied
    // dlc_load.json. The ownership invariant is that the corroborated logs
    // and the dlc_load.json we read live in the SAME physical directory:
    // canonicalize (resolves every junction/symlink in the chain) and require
    // logs to still be <canonical user_dir>/logs. A user_dir that is ITSELF
    // an alias of the real dir still corroborates — both evidence sources
    // then resolve inside the same physical directory, so the binding is that
    // directory's either way. Deliberately NOT keyed on the raw
    // FILE_ATTRIBUTE_REPARSE_POINT: cloud-sync files-on-demand placeholders
    // (OneDrive, Dropbox, ...) are reparse points, and Documents redirected
    // into OneDrive — putting the whole user dir under one — is a stock
    // Windows configuration on end-user machines; those must keep passing.
    // canonicalize resolves such placeholders to themselves. Bounded
    // residual (documented): a same-volume HARDLINK to a held log is not a
    // reparse point and survives canonicalization — creating one needs write
    // access to the live log and is outside the stale-override/wrong-probe
    // misconfiguration threat model this check exists for.
    let canonical_user_dir = fs::canonicalize(user_dir).map_err(|e| {
        format!(
            "{} canonicalize failed ({e}); cannot prove the ownership probe \
             targets this dir",
            portable_path(user_dir)
        )
    })?;
    let canonical_logs = fs::canonicalize(&logs).map_err(|e| {
        format!(
            "{} canonicalize failed ({e}); cannot prove the ownership probe \
             targets this dir",
            portable_path(&logs)
        )
    })?;
    let expected_logs = canonical_user_dir.join("logs");
    // Case-insensitive: NTFS is; canonicalize returns on-disk casing which
    // may differ from the literal "logs" we joined.
    if !canonical_logs
        .as_os_str()
        .eq_ignore_ascii_case(expected_logs.as_os_str())
    {
        return Err(format!(
            "{} resolves to {}, not {} — a junction/symlink redirects the \
             ownership probe away from this dir; refusing to corroborate \
             through an alias",
            portable_path(&logs),
            portable_path(&canonical_logs),
            portable_path(&expected_logs)
        ));
    }
    let entries = fs::read_dir(&logs).map_err(|e| {
        format!(
            "{} unreadable ({e}); cannot prove the attached process owns this dir",
            portable_path(&logs)
        )
    })?;
    let mut probed = 0usize;
    let mut rm_errors: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", portable_path(&logs)))?;
        // `file_type()` does NOT follow links: a symlinked/junction entry is
        // visible as such and refused (same finding — a symlinked game.log
        // pointing at the real one would corroborate the wrong dir).
        let file_type = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", portable_path(&entry.path())))?;
        if file_type.is_symlink() {
            return Err(format!(
                "{} is a symlink/junction; refusing to corroborate through an alias",
                portable_path(&entry.path())
            ));
        }
        if file_type.is_dir() {
            continue;
        }
        probed += 1;
        if probed > 64 {
            break; // bounded: a real logs dir has a handful of files
        }
        match file_held_by_pid(&entry.path(), pid) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => rm_errors.push(format!("{}: {e}", portable_path(&entry.path()))),
        }
    }
    if probed == 0 {
        return Err(format!(
            "{} contains no files; cannot prove the attached process owns this dir",
            portable_path(&logs)
        ));
    }
    Err(format!(
        "pid {pid} holds no open handle to any of the {probed} probed file(s) under {} - \
         this is not the attached process's user dir{}",
        portable_path(&logs),
        if rm_errors.is_empty() {
            String::new()
        } else {
            format!(" (restart-manager errors: {})", rm_errors.join("; "))
        }
    ))
}

/// Streaming sha256 with a size+mtime stability bracket (G2 finding 4): the
/// file's metadata is captured before hashing and re-checked after; any
/// mid-hash mutation (game update mid-capture, partial write) fails closed
/// instead of recording a mixed hash as the build binding.
fn hash_file_streaming_stable(path: &Path) -> Result<(u64, String), String> {
    let meta_before = fs::metadata(path).map_err(|e| format!("{}: {e}", portable_path(path)))?;
    let (size_before, mtime_before) = (meta_before.len(), meta_before.modified().ok());
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", portable_path(path)))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut chunk)
            .map_err(|e| format!("{}: {e}", portable_path(path)))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&chunk[..n]);
    }
    let meta_after = fs::metadata(path).map_err(|e| format!("{}: {e}", portable_path(path)))?;
    if total != size_before
        || meta_after.len() != size_before
        || meta_after.modified().ok() != mtime_before
    {
        return Err(format!(
            "{} changed while hashing (size {}->{} read {}, mtime moved: {})",
            portable_path(path),
            size_before,
            meta_after.len(),
            total,
            meta_after.modified().ok() != mtime_before,
        ));
    }
    Ok((total, to_hex(&hasher.finalize())))
}

/// Locate and prove the Paradox user directory for `pid`.
///
/// Product discovery includes the target command line's `-userdir`, the real
/// `FOLDERID_Documents`, all runtime OneDrive consumer/business account roots,
/// and both legacy profile paths. Candidate aliases are canonical-deduplicated,
/// then the existing PID-held-log proof is run against every physical hit.
/// Exactly one survivor is required. A support override adds a candidate but
/// never narrows discovery or bypasses any proof.
fn locate_user_dir(
    pid: u32,
    user_dir_override: Option<&Path>,
) -> Result<(PathBuf, &'static str), String> {
    #[cfg(windows)]
    {
        let env = auto_attach_modset::WindowsDiscoveryEnvironment;
        let located =
            auto_attach_modset::locate_user_dir_with(&env, pid, user_dir_override, |dir| {
                match corroborate_user_dir_live(dir, pid) {
                    Ok(()) => auto_attach_modset::CandidateCorroboration::Owned,
                    Err(error) if error.contains("restart-manager errors:") => {
                        auto_attach_modset::CandidateCorroboration::Indeterminate(error)
                    }
                    Err(error) => auto_attach_modset::CandidateCorroboration::Rejected(error),
                }
            })?;
        // Preserve the manifest schema consumed by fold-and-repack. Detailed
        // discovery provenance remains internal to the auto-attach resolver;
        // the stable public value is still `probed` for every non-override hit.
        let source = if located.explicit_override {
            "override"
        } else {
            "probed"
        };
        return Ok((located.path, source));
    }

    #[cfg(not(windows))]
    {
        let _ = (pid, user_dir_override);
        Err("Paradox user-directory discovery is Windows-only".to_string())
    }
}

/// Resolve one `dlc_load.json` `enabled_mods` entry to its descriptor content.
/// Entries are usually user-dir-relative (`mod/ugc_123.mod`); absolute entries
/// are honored as-is. Field extraction is line-based on the Clausewitz
/// `key="value"` descriptor grammar — unknown lines are ignored, absent fields
/// stay None (recorded, never fabricated). The CALLER enforces the
/// completeness invariant (readable + content_path + fingerprint) and fails
/// resolution closed when any part is missing.
fn resolve_mod_descriptor(user_dir: &Path, entry: &str) -> ResolvedMod {
    let mut resolved = ResolvedMod {
        descriptor_entry: entry.to_string(),
        descriptor_path: None,
        descriptor_sha256: None,
        name: None,
        version: None,
        remote_file_id: None,
        content_path: None,
        content_fingerprint: None,
        content_sha256: None,
        content_max_mtime: None,
        descriptor_mtime: None,
        content_location_conflict: false,
    };
    let candidate = {
        let as_path = Path::new(entry);
        if as_path.is_absolute() {
            as_path.to_path_buf()
        } else {
            user_dir.join(entry)
        }
    };
    let Ok(text) = fs::read_to_string(&candidate) else {
        return resolved;
    };
    resolved.descriptor_mtime = fs::metadata(&candidate).and_then(|m| m.modified()).ok();
    resolved.descriptor_sha256 = Some(Sha256::hex(text.as_bytes()));
    // Strip a UTF-8 BOM so `\u{feff}name=...` still parses (G2 finding 7).
    let text_body = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut content_location_keys = 0u32;
    for line in text_body.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "name" => resolved.name = Some(value),
            "version" => resolved.version = Some(value),
            "remote_file_id" => resolved.remote_file_id = Some(value),
            // Re-gate finding 5: an EMPTY path=/archive= names no content
            // location (joining "" with user_dir would fingerprint the whole
            // user dir). Leave content_path None so the caller fails closed.
            // G2-regen finding 6: a SECOND content-location key (repeated
            // path=, repeated archive=, or both present — empty or not) makes
            // the effective location ambiguous (engine precedence unproven);
            // the caller fails resolution closed on the conflict flag.
            "path" | "archive" => {
                content_location_keys += 1;
                if content_location_keys > 1 {
                    resolved.content_location_conflict = true;
                }
                if !value.is_empty() {
                    resolved.content_path = Some(value);
                }
            }
            _ => {}
        }
    }
    if resolved.content_location_conflict {
        // Don't bind ambiguous content; the caller records the failure.
        resolved.content_path = None;
    }
    if let Some(content) = resolved.content_path.as_deref() {
        let content_abs = {
            let as_path = Path::new(content);
            if as_path.is_absolute() {
                as_path.to_path_buf()
            } else {
                user_dir.join(content)
            }
        };
        if let Some(hashed) = hash_content_stable(&content_abs) {
            resolved.content_fingerprint = Some((
                hashed.files,
                hashed.bytes,
                system_time_unix(hashed.max_mtime),
            ));
            resolved.content_sha256 = Some(hashed.sha256);
            resolved.content_max_mtime = Some(hashed.max_mtime);
        }
    }
    resolved.descriptor_path = Some(candidate);
    resolved
}

/// The content binding for one mod: an exact SHA-256 plus cheap metadata.
struct HashedContent {
    files: u64,
    bytes: u64,
    /// Newest mtime anywhere in the walk, full precision (for the
    /// evidence-predates-launch check).
    max_mtime: SystemTime,
    /// SHA-256 over sorted relative paths + per-file length + file bytes.
    sha256: String,
}

/// Metadata pass over a content location: (files, bytes, max_mtime) plus the
/// sorted relative file list. Any unreadable entry/metadata/mtime fails the
/// pass closed (re-gate finding 6). The bounded walk REFUSES (None) past
/// `MOD_CONTENT_MAX_FILES` rather than silently truncating.
/// Bounded directory count for a mod content walk (modset re-gate finding 5:
/// only FILES were bounded, so an empty directory reparse cycle looped
/// forever without ever failing closed).
const MOD_CONTENT_MAX_DIRS: usize = 50_000;

fn walk_content_metadata(root: &Path) -> Option<(u64, u64, SystemTime, Vec<PathBuf>)> {
    if root.is_file() {
        let meta = fs::metadata(root).ok()?;
        return Some((1, meta.len(), meta.modified().ok()?, vec![PathBuf::new()]));
    }
    if !root.is_dir() {
        return None;
    }
    let mut files = 0u64;
    let mut dirs = 0usize;
    let mut bytes = 0u64;
    // Modset re-gate finding 2: DIRECTORY mtimes join the maximum. An NTFS
    // rename preserves the file's own mtime but updates the parent directory,
    // so a post-launch renamed tree was invisible to the file-only maximum
    // (both hash passes already saw the new path; the launch-order check did
    // not). The root and every traversed directory now contribute.
    let mut max_mtime: Option<SystemTime> = fs::metadata(root).ok()?.modified().ok();
    let mut rel_paths: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            let path = entry.path();
            // Modset re-gate finding 5: refuse symlinks/reparse points
            // outright (fail closed) — following them let a junction cycle
            // walk forever (no file ever incremented the bound), and a link
            // escaping the content root would bind foreign content.
            // `DirEntry::metadata()` does not follow symlinks, so file_type
            // here is the LINK's own type.
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if meta.is_dir() {
                dirs += 1;
                if dirs > MOD_CONTENT_MAX_DIRS {
                    return None;
                }
                let mtime = meta.modified().ok()?;
                max_mtime = Some(max_mtime.map_or(mtime, |m: SystemTime| m.max(mtime)));
                stack.push(path);
            } else {
                files += 1;
                if files as usize > MOD_CONTENT_MAX_FILES {
                    return None;
                }
                bytes = bytes.saturating_add(meta.len());
                let mtime = meta.modified().ok()?;
                max_mtime = Some(max_mtime.map_or(mtime, |m: SystemTime| m.max(mtime)));
                rel_paths.push(path.strip_prefix(root).ok()?.to_path_buf());
            }
        }
    }
    if files == 0 {
        // An empty content dir binds nothing — treat as not bindable.
        return None;
    }
    rel_paths.sort();
    Some((files, bytes, max_mtime?, rel_paths))
}

/// Exact content binding under a double-pass stability bracket (G2-regen
/// finding 3: the old (count, bytes, max-mtime) tuple was systematically
/// non-unique — renames, same-length edits, and same-second edits all
/// collided). SHA-256 over: for each file in sorted relative-path order, the
/// path (forward-slash, utf-8) + 0x00 + the file length (LE u64) + the file
/// bytes. The metadata walk runs BEFORE and AFTER hashing and must agree, and
/// each file's streamed byte count must match its listed length — an
/// incoherent walk (content changing underneath) fails closed.
fn hash_content_stable(root: &Path) -> Option<HashedContent> {
    let (files, bytes, max_mtime, rel_paths) = walk_content_metadata(root)?;
    let mut hasher = Sha256::new();
    for rel in &rel_paths {
        let abs = if rel.as_os_str().is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        let rel_key = rel.to_str()?.replace('\\', "/");
        hasher.update(rel_key.as_bytes());
        hasher.update(&[0u8]);
        let mut file = fs::File::open(&abs).ok()?;
        let listed_len = fs::metadata(&abs).ok()?.len();
        hasher.update(&listed_len.to_le_bytes());
        let mut streamed = 0u64;
        let mut chunk = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            streamed += n as u64;
            hasher.update(&chunk[..n]);
        }
        if streamed != listed_len {
            return None;
        }
    }
    // Stability bracket: the metadata pass must reproduce exactly.
    let (files_after, bytes_after, max_mtime_after, rel_after) = walk_content_metadata(root)?;
    if (files, bytes, max_mtime) != (files_after, bytes_after, max_mtime_after)
        || rel_paths != rel_after
    {
        return None;
    }
    Some(HashedContent {
        files,
        bytes,
        max_mtime,
        sha256: to_hex(&hasher.finalize()),
    })
}

/// Enumerate installed DLC descriptors as game-root-relative
/// forward-slash paths (`dlc/dlc023_man_the_guns/dlc023.dlc`) — the exact key
/// format `dlc_load.json.disabled_dlcs` uses, so the subtraction is a string
/// match with no normalization guesswork (and `resolve_modset` REFUSES any
/// disabled key that matches no installed entry).
/// Also returns the newest `.dlc` descriptor mtime (full precision) so the
/// caller can prove the census predates process start (G2-regen finding 4).
fn census_installed_dlcs(game_root: &Path) -> Result<(Vec<String>, Option<SystemTime>), String> {
    let dlc_root = game_root.join("dlc");
    let entries = fs::read_dir(&dlc_root).map_err(|e| format!("{}: {e}", portable_path(&dlc_root)))?;
    let mut installed = Vec::new();
    let mut newest: Option<SystemTime> = None;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", portable_path(&dlc_root)))?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let children = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", portable_path(&dir)))?;
        for child in children {
            let child = child.map_err(|e| format!("{}: {e}", portable_path(&dir)))?;
            let path = child.path();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dlc"))
            {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    let mtime = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map_err(|e| format!("{}: mtime unreadable: {e}", portable_path(&path)))?;
                    newest = Some(newest.map_or(mtime, |n: SystemTime| n.max(mtime)));
                    installed.push(format!("dlc/{dir_name}/{file_name}"));
                }
            }
        }
    }
    installed.sort();
    Ok((installed, newest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "modset_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn authority_report(user_dir: &Path, settings_sha256: String) -> ModsetReport {
        ModsetReport {
            content_resolved: true,
            errors: Vec::new(),
            settings: SettingsBinding {
                authority: Some(SettingsAuthority::PreLaunchBracketed),
                errors: Vec::new(),
                language: Some("english".into()),
                display: Some(DisplayVariantSettings {
                    physical_width: 1920,
                    physical_height: 1080,
                    min_gui_width: 1280,
                    min_gui_height: 720,
                    gui_scale_f32_bits: 1.0f32.to_bits(),
                    refresh_rate_hz: 60,
                    max_refresh_rate_hz: Some(75),
                    fullscreen: false,
                    borderless: true,
                    display_mode: "borderless_fullscreen".into(),
                    settings_file_sha256: settings_sha256.clone(),
                }),
                attach_sha256: Some(settings_sha256),
                seal_sha256: None,
                endpoint_semantics_equal: None,
            },
            exe_path: Some(PathBuf::from(r"F:\game\hoi4.exe")),
            exe_size_bytes: Some(7),
            exe_sha256: Some("ab".repeat(32)),
            user_dir: Some(user_dir.to_path_buf()),
            user_dir_source: Some("probed"),
            dlc_load_mtime_unix: Some(1),
            dlc_load_sha256: Some("cd".repeat(32)),
            process_start_unix: Some(2),
            process_start_filetime: None,
            enabled_mods: Vec::new(),
            installed_dlcs: vec!["dlc/a/a.dlc".into()],
            disabled_dlcs: Vec::new(),
            enabled_dlcs: vec!["dlc/a/a.dlc".into()],
            dlc_display_names: BTreeMap::new(),
            dlc_descriptor_evidence: BTreeMap::new(),
        }
    }

    #[test]
    fn same_process_settings_authority_recovers_only_matching_identity_and_semantics() {
        let dir = temp_dir("same_process_settings_authority");
        let initial = "language=\"english\"\ngraphics={\n\
            size={ x=1920 y=1080 }\nmin_gui={ x=1280 y=720 }\n\
            gui_scale=1.0\nrefreshRate=60\nmax_refresh_rate=75\n\
            fullScreen=no\nborderless=yes\n}\n";
        fs::write(dir.join("settings.txt"), initial).unwrap();
        let settings_sha256 = Sha256::hex(initial.as_bytes());
        let authority_path = dir.join("authority.json");
        let mut proven = authority_report(&dir, settings_sha256);
        apply_same_process_settings_authority(&mut proven, &authority_path, 77, 88).unwrap();

        // the game may rewrite the same effective settings with different spacing
        // and field order after launch. The immutable launch authority remains
        // valid for the identical live process.
        let rewritten_text = "graphics={ borderless=yes fullScreen=no \
            max_refresh_rate=75 refreshRate=60 gui_scale=1.000000 \
            min_gui={y=720 x=1280} size={y=1080 x=1920} }\n\
            language=\"english\"\n";
        fs::write(dir.join("settings.txt"), rewritten_text).unwrap();
        let mut rewritten = proven.clone();
        rewritten.settings.authority = Some(SettingsAuthority::PostLaunchDiskObservation);
        rewritten.settings.language = None;
        rewritten.settings.display = None;
        // This process read the REWRITTEN file, so its attach digest is that
        // file's. The recovery is about to publish the ORIGINAL display.
        rewritten.settings.attach_sha256 = Some(Sha256::hex(rewritten_text.as_bytes()));
        apply_same_process_settings_authority(&mut rewritten, &authority_path, 77, 88).unwrap();
        // The CONTENT binding was never in question here, and the recovery
        // upgrades the SETTINGS authority rather than a single global bit.
        assert!(rewritten.content_resolved);
        assert_eq!(
            rewritten.settings.authority,
            Some(SettingsAuthority::SameProcessAuthority)
        );
        assert!(rewritten.settings.authority.unwrap().is_launch_proven());
        assert_eq!(
            rewritten.localisation_language().map(String::as_str),
            Some("english")
        );
        assert_eq!(
            rewritten.display_variant().unwrap().settings_file_sha256,
            Sha256::hex(initial.as_bytes())
        );
        // THE ATTACH DIGEST FOLLOWS THE DISPLAY IT DESCRIBES. It means "the
        // digest of the read these display values came from", and the recovery
        // has just replaced those values with the authority's. Leaving this
        // process's own digest in place published a display whose digest named
        // a different read of the file -- and every consumer reassembles the
        // display from this digest, so all of them refused the container. The
        // two digests genuinely differ here: that semantic-match-despite-a-
        // rewrite is the whole case this recovery exists for.
        assert_ne!(
            Sha256::hex(rewritten_text.as_bytes()),
            Sha256::hex(initial.as_bytes())
        );
        assert_eq!(
            rewritten.settings.attach_sha256.as_deref(),
            Some(Sha256::hex(initial.as_bytes()).as_str())
        );

        fs::write(
            dir.join("settings.txt"),
            rewritten_text.replace("x=1920", "x=1600"),
        )
        .unwrap();
        let mut changed = proven.clone();
        changed.settings.authority = Some(SettingsAuthority::PostLaunchDiskObservation);
        changed.settings.language = None;
        changed.settings.display = None;
        assert!(apply_same_process_settings_authority(
            &mut changed,
            &authority_path,
            77,
            88
        )
        .unwrap_err()
        .contains("current semantic settings mismatch"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mod_descriptor_fields_parse_hash_and_fingerprint() {
        let dir = temp_dir("descriptor");
        fs::create_dir_all(dir.join("mod").join("test")).unwrap();
        let descriptor =
            "version=\"1.2\"\nname=\"Test Mod\"\nremote_file_id=\"12345\"\npath=\"mod/test\"\n";
        fs::write(dir.join("mod").join("test.mod"), descriptor).unwrap();
        fs::write(dir.join("mod").join("test").join("a.txt"), b"hello").unwrap();

        let resolved = resolve_mod_descriptor(&dir, "mod/test.mod");
        assert_eq!(resolved.name.as_deref(), Some("Test Mod"));
        assert_eq!(resolved.version.as_deref(), Some("1.2"));
        assert_eq!(resolved.remote_file_id.as_deref(), Some("12345"));
        assert_eq!(resolved.content_path.as_deref(), Some("mod/test"));
        assert_eq!(
            resolved.descriptor_sha256.as_deref(),
            Some(Sha256::hex(descriptor.as_bytes()).as_str())
        );
        let (files, bytes, _mtime) = resolved.content_fingerprint.expect("fingerprint");
        assert_eq!((files, bytes), (1, 5));

        // Content change WITHOUT a descriptor change moves the fingerprint.
        fs::write(dir.join("mod").join("test").join("b.txt"), b"world!").unwrap();
        let again = resolve_mod_descriptor(&dir, "mod/test.mod");
        assert_eq!(again.descriptor_sha256, resolved.descriptor_sha256);
        assert_ne!(again.content_fingerprint, resolved.content_fingerprint);

        // A missing descriptor stays evidence-free (caller records the error).
        let missing = resolve_mod_descriptor(&dir, "mod/absent.mod");
        assert!(missing.descriptor_sha256.is_none());
        assert!(missing.descriptor_path.is_none());

        // A descriptor without path=/archive= parses but carries no content
        // binding — the caller fails resolution closed on that.
        fs::write(dir.join("mod").join("nopath.mod"), "name=\"X\"\n").unwrap();
        let nopath = resolve_mod_descriptor(&dir, "mod/nopath.mod");
        assert!(nopath.descriptor_sha256.is_some());
        assert!(nopath.content_path.is_none());

        // BOM-prefixed first key still parses (finding 7).
        fs::write(
            dir.join("mod").join("bom.mod"),
            "\u{feff}name=\"Bom\"\npath=\"mod/test\"\n",
        )
        .unwrap();
        let bom = resolve_mod_descriptor(&dir, "mod/bom.mod");
        assert_eq!(bom.name.as_deref(), Some("Bom"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_or_mistyped_dlc_load_fields_fail_closed() {
        // `{}`: both fields absent -> both errors, never "vanilla".
        let empty: Value = serde_json::from_str("{}").unwrap();
        assert!(required_string_array(&empty, "enabled_mods").is_err());
        assert!(required_string_array(&empty, "disabled_dlcs").is_err());

        // Wrong types fail closed.
        let corrupt: Value =
            serde_json::from_str(r#"{"enabled_mods":"corrupt","disabled_dlcs":[1]}"#).unwrap();
        assert!(required_string_array(&corrupt, "enabled_mods").is_err());
        assert!(required_string_array(&corrupt, "disabled_dlcs").is_err());

        // The real shape passes.
        let ok: Value = serde_json::from_str(
            r#"{"enabled_mods":["mod/a.mod"],"disabled_dlcs":["dlc/d/d.dlc"]}"#,
        )
        .unwrap();
        assert_eq!(
            required_string_array(&ok, "enabled_mods").unwrap(),
            vec!["mod/a.mod".to_string()]
        );
    }

    #[test]
    fn dlc_census_emits_dlc_load_relative_keys_and_subtraction_matches() {
        let root = temp_dir("dlc");
        fs::create_dir_all(root.join("dlc").join("dlc023_man_the_guns")).unwrap();
        fs::create_dir_all(root.join("dlc").join("dlc013_sabaton")).unwrap();
        fs::write(
            root.join("dlc")
                .join("dlc023_man_the_guns")
                .join("dlc023.dlc"),
            "name=\"Man the Guns\"",
        )
        .unwrap();
        fs::write(
            root.join("dlc").join("dlc013_sabaton").join("dlc013.dlc"),
            "name=\"Sabaton\"",
        )
        .unwrap();

        let (installed, census_max_mtime) = census_installed_dlcs(&root).unwrap();
        assert_eq!(
            installed,
            vec![
                "dlc/dlc013_sabaton/dlc013.dlc".to_string(),
                "dlc/dlc023_man_the_guns/dlc023.dlc".to_string(),
            ]
        );
        // Freshly-written census files carry a real newest mtime — the
        // evidence-predates-launch check compares against this.
        assert!(census_max_mtime.is_some());
        let disabled = vec!["dlc/dlc013_sabaton/dlc013.dlc".to_string()];
        let enabled: Vec<&String> = installed
            .iter()
            .filter(|rel| !disabled.iter().any(|d| d == *rel))
            .collect();
        assert_eq!(enabled, vec!["dlc/dlc023_man_the_guns/dlc023.dlc"]);

        let launch = SystemTime::now() + std::time::Duration::from_secs(60);
        let (display_names, descriptor_evidence, display_errors) =
            resolve_dlc_display_names(&root, &installed, Some(launch));
        assert!(display_errors.is_empty());
        assert_eq!(descriptor_evidence.len(), installed.len());
        assert_eq!(
            display_names["dlc/dlc013_sabaton/dlc013.dlc"].as_deref(),
            Some("Sabaton")
        );
        assert_eq!(
            display_names["dlc/dlc023_man_the_guns/dlc023.dlc"].as_deref(),
            Some("Man the Guns")
        );

        let (post_launch, post_launch_evidence, post_launch_errors) =
            resolve_dlc_display_names(&root, &installed, Some(std::time::UNIX_EPOCH));
        assert!(post_launch.values().all(Option::is_none));
        assert!(post_launch_evidence.values().all(Option::is_none));
        assert_eq!(post_launch_errors.len(), installed.len());
        assert!(post_launch_errors
            .iter()
            .all(|error| error.contains("recorded null") && error.contains("at/after")));

        // Every installed path remains present when its descriptor becomes
        // unreadable; the value is null and the failure note is explicit.
        portable_test_fs_retry::remove_file(root.join("dlc").join("dlc013_sabaton").join("dlc013.dlc")).unwrap();
        let (after_remove, after_remove_evidence, errors) =
            resolve_dlc_display_names(&root, &installed, Some(launch));
        assert!(after_remove["dlc/dlc013_sabaton/dlc013.dlc"].is_none());
        assert!(after_remove_evidence["dlc/dlc013_sabaton/dlc013.dlc"].is_none());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("recorded null"));

        // An unmatched disabled key is detectable by the same exact-match rule
        // resolve_modset enforces (fails closed there).
        let drifted = "DLC/dlc013_sabaton/dlc013.dlc".to_string();
        assert!(!installed.iter().any(|rel| rel == &drifted));

        fs::remove_dir_all(&root).ok();
    }

    /// THE t55 DEFECT, as a unit.
    ///
    /// the game rewrites `settings.txt` while it loads, so on a game the player
    /// launched the file is always newer than the process. That used to void
    /// the WHOLE report — mods, DLC and executable included — which refused the
    /// content proof, unpackaged every static artifact, and (because the
    /// refusal quoted the absolute path) refused the seal.
    ///
    /// The settings must degrade to an OBSERVATION and take nothing else with
    /// them.
    ///
    /// Production change that would make this fail: restoring the pre-launch
    /// mtime requirement as an admission gate for settings, or folding the
    /// settings state back into `content_resolved`.
    #[test]
    fn a_post_launch_settings_rewrite_degrades_the_settings_and_nothing_else() {
        let dir = temp_dir("post_launch_settings");
        let text = "language=\"l_english\"\ngraphics={\n\
             size={ x=2752 y=1152 }\nmin_gui={ x=1280 y=720 }\n\
             gui_scale=1.000000\nrefreshRate=60\nmax_refresh_rate=75\n\
             fullScreen=no\nborderless=yes\n}\n";
        fs::write(dir.join("settings.txt"), text).unwrap();

        // The file exists and parses, but it was written AFTER the process
        // started — the ordinary case for a hand-launched game.
        let launched_before_the_write = std::time::UNIX_EPOCH;
        assert!(read_launch_settings(&dir, SystemTime::now() + std::time::Duration::from_secs(60))
            .is_ok());
        assert!(read_launch_settings(&dir, launched_before_the_write).is_err());

        let (language, display) = read_launch_settings_observed(&dir).expect("observed read");
        assert_eq!(language.as_deref(), Some("l_english"));
        assert_eq!(display.physical_width, 2752);
        assert_eq!(display.settings_file_sha256, Sha256::hex(text.as_bytes()));

        // And the observation is never dressed up as a launch proof.
        assert!(!SettingsAuthority::PostLaunchDiskObservation.is_launch_proven());
        assert!(SettingsAuthority::PreLaunchBracketed.is_launch_proven());
        assert!(SettingsAuthority::SameProcessAuthority.is_launch_proven());
        assert_eq!(
            SettingsAuthority::PostLaunchDiskObservation.as_str(),
            "post_launch_disk_observation"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// A resolution failure must be able to cross the container boundary. This
    /// is the exact string capture t55 died on.
    ///
    /// Production change that would make this fail: spelling a path with
    /// `Path::display()` in a diagnostic again, or dropping the backstop in
    /// `ModsetError::new`.
    #[test]
    fn a_resolution_failure_carries_no_absolute_path() {
        let spelled = ModsetError::new(
            codes::SETTINGS_UNREADABLE,
            format!(
                "settings.txt unreadable: {} mtime is at/after game process start",
                portable_path(Path::new(
                    r"\\?\C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\settings.txt"
                ))
            ),
        );
        assert_eq!(
            spelled.to_string(),
            "settings_unreadable: settings.txt unreadable: settings.txt mtime is at/after \
             game process start"
        );
        crate::portable_manifest::reject_absolute_paths_in_json(
            &Value::String(spelled.to_string()),
            "$",
        )
        .expect("a diagnostic must be portable");

        // The backstop catches text this module did not compose itself.
        let leaked = ModsetError::new(
            codes::DLC_LOAD_READ,
            r"dlc_load.json read failed at C:\Users\Example\dlc_load.json: denied".to_string(),
        );
        crate::portable_manifest::reject_absolute_paths_in_json(
            &Value::String(leaked.to_string()),
            "$",
        )
        .expect("the backstop must make an unforeseen path portable");
        assert!(!leaked.detail.contains("someone"), "{}", leaked.detail);
    }

    /// The closing observation must refuse to compare against a process that is
    /// no longer the one that was recorded. An exit-time rewrite — or a reused
    /// PID — would otherwise be reported as a settings disagreement inside the
    /// recording.
    ///
    /// Production change that would make this fail: dropping the process
    /// identity check from `observe_settings_at_capture_close`.
    #[test]
    fn the_closing_settings_observation_refuses_a_lost_process_identity() {
        let dir = temp_dir("closing_observation");
        let text = "language=\"english\"\ngraphics={\n\
             size={ x=1920 y=1080 }\nmin_gui={ x=1280 y=720 }\n\
             gui_scale=1.0\nrefreshRate=60\nmax_refresh_rate=75\n\
             fullScreen=no\nborderless=yes\n}\n";
        fs::write(dir.join("settings.txt"), text).unwrap();
        let mut report = authority_report(&dir, Sha256::hex(text.as_bytes()));

        // This process is alive, but it did not start when the report says the
        // game did, so the identity does not match. The comparison is on the
        // FULL-PRECISION creation time: a whole-second stamp would accept a pid
        // reused inside the same second as the game.
        report.process_start_filetime = Some(1);
        observe_settings_at_capture_close(&mut report, std::process::id());
        assert!(report.settings.endpoint_semantics_equal.is_none());
        assert_eq!(
            report.settings.errors.last().map(|error| error.code),
            Some(codes::SETTINGS_ENDPOINT_PROCESS_GONE)
        );

        // With the identity intact the endpoints are compared, and a semantic
        // rewrite between them is not a disagreement.
        report.settings.errors.clear();
        report.process_start_filetime =
            Some(query_process_creation_filetime(std::process::id()).unwrap());
        // Same second, different process: the identity must still be refused.
        let same_second_other_identity = report.process_start_filetime.unwrap() + 1;
        let mut reused_pid = report.clone();
        reused_pid.process_start_filetime = Some(same_second_other_identity);
        observe_settings_at_capture_close(&mut reused_pid, std::process::id());
        assert!(reused_pid.settings.endpoint_semantics_equal.is_none());
        assert_eq!(
            reused_pid.settings.errors.last().map(|error| error.code),
            Some(codes::SETTINGS_ENDPOINT_PROCESS_GONE)
        );
        fs::write(
            dir.join("settings.txt"),
            "graphics={ borderless=yes fullScreen=no max_refresh_rate=75 \
             refreshRate=60 gui_scale=1.000000 min_gui={y=720 x=1280} \
             size={y=1080 x=1920} }\nlanguage=\"english\"\n",
        )
        .unwrap();
        observe_settings_at_capture_close(&mut report, std::process::id());
        assert_eq!(report.settings.endpoint_semantics_equal, Some(true));
        assert!(report.settings.seal_sha256.is_some());
        assert_ne!(report.settings.seal_sha256, report.settings.attach_sha256);

        // A real change IS a disagreement.
        fs::write(
            dir.join("settings.txt"),
            text.replace("x=1920", "x=1600").as_str(),
        )
        .unwrap();
        observe_settings_at_capture_close(&mut report, std::process::id());
        assert_eq!(report.settings.endpoint_semantics_equal, Some(false));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn launch_language_and_display_read_one_stable_settings_snapshot() {
        let dir = temp_dir("language");
        fs::write(
            dir.join("settings.txt"),
            "\u{feff}language=\"l_english\"\ngraphics={\n\
             size={ x=2752 y=1152 }\nmin_gui={ x=1280 y=720 }\n\
             gui_scale=1.000000\nrefreshRate=60\nmax_refresh_rate=75\n\
             fullScreen=no\nborderless=yes\n}\n",
        )
        .unwrap();
        let (language, display) =
            read_launch_settings(&dir, SystemTime::now() + std::time::Duration::from_secs(60))
                .unwrap();
        assert_eq!(language.as_deref(), Some("l_english"));
        assert_eq!(
            (display.physical_width, display.physical_height),
            (2752, 1152)
        );
        assert_eq!((display.min_gui_width, display.min_gui_height), (1280, 720));
        assert_eq!(display.gui_scale_f32_bits, 1.0f32.to_bits());
        assert_eq!(display.display_mode, "borderless_fullscreen");
        assert_eq!(display.settings_file_sha256.len(), 64);

        fs::write(dir.join("settings.txt"), "volume=0.5\n").unwrap();
        assert!(
            read_launch_settings(&dir, SystemTime::now() + std::time::Duration::from_secs(60),)
                .is_err()
        );
        assert!(read_launch_settings(&dir, std::time::UNIX_EPOCH).is_err());
        portable_test_fs_retry::remove_file(dir.join("settings.txt")).unwrap();
        assert!(
            read_launch_settings(&dir, SystemTime::now() + std::time::Duration::from_secs(60),)
                .is_err()
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clausewitz_string_field_rejects_torn_escaped_duplicate_and_blank_assignments() {
        for (label, text) in [
            ("unclosed quote", "language=\"l_english\n"),
            ("malformed escape", "language=\"l_eng\\qish\"\n"),
            (
                "duplicate",
                "language=\"l_english\"\nlanguage=\"l_french\"\n",
            ),
            ("whitespace only", "language=\"   \"\n"),
        ] {
            assert!(
                clausewitz_string_field(text, "language").is_err(),
                "{label} assignment must fail closed"
            );
        }
        assert_eq!(
            clausewitz_string_field("language=\"l_eng\\nlish\" # bound\n", "language")
                .unwrap()
                .as_deref(),
            Some("l_eng\nlish")
        );
        assert!(clausewitz_string_field("volume=\"0.5\"\n", "language")
            .unwrap()
            .is_none());
    }

    #[test]
    fn stable_text_validation_rejects_post_launch_torn_and_metadata_drift() {
        let path = Path::new("settings.txt");
        let before = FileStamp {
            len: 4,
            modified: std::time::UNIX_EPOCH + std::time::Duration::from_secs(10),
        };
        let launch = std::time::UNIX_EPOCH + std::time::Duration::from_secs(20);
        assert!(validate_stable_text_read(
            path,
            launch,
            before,
            b"same".to_vec(),
            b"torn".to_vec(),
            before,
        )
        .unwrap_err()
        .contains("bytes differ"));
        let moved = FileStamp {
            modified: std::time::UNIX_EPOCH + std::time::Duration::from_secs(11),
            ..before
        };
        assert!(validate_stable_text_read(
            path,
            launch,
            before,
            b"same".to_vec(),
            b"same".to_vec(),
            moved,
        )
        .unwrap_err()
        .contains("metadata moved"));
        assert!(validate_stable_text_read(
            path,
            before.modified,
            before,
            b"same".to_vec(),
            b"same".to_vec(),
            before,
        )
        .unwrap_err()
        .contains("at/after"));
    }

    #[test]
    fn ambiguous_or_missing_user_dir_fails_closed_and_records_source() {
        let dir = temp_dir("override");
        // Override without dlc_load.json inside: must FAIL, never fall back to
        // probing (an explicit pin that does not prove out is an error).
        let err = locate_user_dir(std::process::id(), Some(&dir)).unwrap_err();
        assert!(err.contains("does not contain dlc_load.json"), "{err}");

        fs::write(dir.join("dlc_load.json"), "{}").unwrap();
        fs::create_dir_all(dir.join("logs")).unwrap();
        let log = dir.join("logs").join("game.log");
        fs::write(&log, "held").unwrap();
        let held =
            portable_test_fs_retry::open_with(fs::OpenOptions::new().append(true), &log).unwrap();
        let (found, source) = locate_user_dir(std::process::id(), Some(&dir)).unwrap();
        assert_eq!(found, fs::canonicalize(&dir).unwrap());
        assert_eq!(source, "override");
        drop(held);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unresolved_report_nests_evidence_under_partial_evidence() {
        let report = ModsetReport {
            content_resolved: false,
            errors: vec![ModsetError::new(codes::DLC_CENSUS, "x".to_string())],
            settings: SettingsBinding::default(),
            exe_path: None,
            exe_size_bytes: None,
            exe_sha256: None,
            user_dir: None,
            user_dir_source: None,
            dlc_load_mtime_unix: None,
            dlc_load_sha256: None,
            process_start_unix: None,
            process_start_filetime: None,
            enabled_mods: Vec::new(),
            installed_dlcs: vec!["dlc/a/a.dlc".to_string()],
            disabled_dlcs: Vec::new(),
            enabled_dlcs: vec!["dlc/a/a.dlc".to_string()],
            dlc_display_names: BTreeMap::from([("dlc/a/a.dlc".to_string(), Some("A".to_string()))]),
            dlc_descriptor_evidence: BTreeMap::new(),
        };
        let doc = report.as_json();
        // A naive consumer reading top-level lists on an unresolved report
        // finds NOTHING (fail-closed shape).
        assert!(doc.get("enabled_mods").is_none());
        assert!(doc.get("dlc").is_none());
        assert_eq!(doc["resolved"], Value::Bool(false));
        assert_eq!(doc["schema"], MODSET_BINDING_SCHEMA);
        assert_eq!(doc["errors"][0], "dlc_census_failed: x");
        assert!(doc["partial_evidence"]["dlc"]["enabled"].is_array());
        assert_eq!(
            doc["partial_evidence"]["dlc_display_names"]["dlc/a/a.dlc"],
            "A"
        );
        // The settings binding is its own claim, present in BOTH shapes, and
        // an unavailable authority is stated rather than implied by an absent
        // language.
        assert!(doc["settings_binding"]["authority"].is_null());
        assert_eq!(doc["settings_binding"]["launch_proven"], Value::Bool(false));
        assert!(doc["settings_binding"]["language"].is_null());
        assert!(doc["settings_binding"]["display_variant"].is_null());

        let resolved = ModsetReport {
            content_resolved: true,
            ..report
        };
        let doc = resolved.as_json();
        assert!(doc.get("partial_evidence").is_none());
        assert!(doc["dlc"]["enabled"].is_array());
        assert_eq!(doc["dlc_display_names"]["dlc/a/a.dlc"], "A");
        // A resolved CONTENT chain does NOT assert anything about settings:
        // this is the split that lets a hand-launched game record a proven
        // mod/DLC binding while its settings are only observed.
        assert!(doc["settings_binding"]["language"].is_null());
        assert!(doc.get("localisation_language").is_none());
    }

    #[test]
    fn duplicate_top_level_keys_are_rejected() {
        // serde_json accepts this (keeps the last), so the structural scan is
        // the only line of defense against the silent-vanilla override.
        let dup = r#"{"enabled_mods":["mod/a.mod"],"enabled_mods":[],"disabled_dlcs":[]}"#;
        assert!(serde_json::from_str::<Value>(dup).is_ok());
        let err = reject_duplicate_top_level_keys(dup).unwrap_err();
        assert!(err.contains("enabled_mods"));

        // A NESTED repeat of the same name is fine (different object).
        let nested =
            r#"{"enabled_mods":["mod/a.mod"],"meta":{"enabled_mods":1},"disabled_dlcs":[]}"#;
        assert!(reject_duplicate_top_level_keys(nested).is_ok());

        // The name appearing inside a string VALUE is not a key.
        let in_value = r#"{"note":"enabled_mods twice","enabled_mods":["x"]}"#;
        assert!(reject_duplicate_top_level_keys(in_value).is_ok());

        // Escaped quote inside a value does not desync the scanner.
        let escaped = r#"{"a":"he said \"enabled_mods\"","enabled_mods":[]}"#;
        assert!(reject_duplicate_top_level_keys(escaped).is_ok());
    }

    #[test]
    fn unicode_escaped_duplicate_keys_are_rejected() {
        // G2-regen finding 1: serde_json decodes an escaped-underscore
        // spelling of enabled_mods to the same key and keeps the LAST (empty)
        // value; a raw-slice comparison saw two distinct strings and passed.
        // The scanner must compare DECODED names.
        // `\ u 0 0 5 f` assembled via concat! so the escape sequence exists
        // only in the JSON under test, never in this source file.
        const ESCAPED_UNDERSCORE: &str = concat!(r"\u", "005f");
        let dup = format!(
            r#"{{"enabled_mods":["mod/a.mod"],"enabled{}mods":[],"disabled_dlcs":[]}}"#,
            ESCAPED_UNDERSCORE
        );
        let parsed: Value = serde_json::from_str(&dup).unwrap();
        // Confirm the trap is real: serde sees ONE key holding the empty list.
        assert_eq!(parsed["enabled_mods"], serde_json::json!([]));
        let err = reject_duplicate_top_level_keys(&dup).unwrap_err();
        assert!(err.contains("enabled_mods"), "{err}");

        // Mixed escape forms of the same name are also duplicates.
        let dup2 = format!(r#"{{"abc":1,"ab{}":2}}"#, concat!(r"\u", "0063"));
        assert!(reject_duplicate_top_level_keys(&dup2).is_err());

        // Distinct decoded names stay accepted.
        let ok = r#"{"enabled_mods":[],"disabled_dlcs":[]}"#;
        assert!(reject_duplicate_top_level_keys(ok).is_ok());
    }

    #[test]
    fn decode_json_string_handles_escapes_and_surrogates() {
        assert_eq!(decode_json_string("plain").unwrap(), "plain");
        assert_eq!(
            decode_json_string(&format!("enabled{}mods", concat!(r"\u", "005f"))).unwrap(),
            "enabled_mods"
        );
        assert_eq!(
            decode_json_string(r#"a\"b\\c\/d\n"#).unwrap(),
            "a\"b\\c/d\n"
        );
        // Surrogate pair: U+1F600.
        assert_eq!(
            decode_json_string(&format!(
                "{}{}",
                concat!(r"\u", "D83D"),
                concat!(r"\u", "DE00")
            ))
            .unwrap(),
            "\u{1F600}"
        );
        // Unpaired surrogate fails closed.
        assert!(decode_json_string(r"\uD83D").is_err());
        assert!(decode_json_string(r"\uZZZZ").is_err());
        assert!(decode_json_string(r"dangling\").is_err());
    }

    #[test]
    fn competing_or_repeated_content_locations_are_conflicts() {
        // G2-regen finding 6: last-wins content selection is unproven engine
        // semantics — both-present and repeated keys must fail closed.
        let dir = temp_dir("conflict");
        fs::create_dir_all(dir.join("mod").join("a")).unwrap();
        fs::write(dir.join("mod").join("a").join("f.txt"), b"x").unwrap();

        fs::write(
            dir.join("mod").join("both.mod"),
            "name=\"B\"\npath=\"mod/a\"\narchive=\"mod/b.zip\"\n",
        )
        .unwrap();
        let both = resolve_mod_descriptor(&dir, "mod/both.mod");
        assert!(both.content_location_conflict);
        assert!(both.content_path.is_none());
        assert!(both.content_sha256.is_none());

        fs::write(
            dir.join("mod").join("repeat.mod"),
            "path=\"mod/a\"\npath=\"mod/a\"\n",
        )
        .unwrap();
        let repeat = resolve_mod_descriptor(&dir, "mod/repeat.mod");
        assert!(repeat.content_location_conflict);
        assert!(repeat.content_path.is_none());

        // A single location stays bound.
        fs::write(dir.join("mod").join("one.mod"), "path=\"mod/a\"\n").unwrap();
        let one = resolve_mod_descriptor(&dir, "mod/one.mod");
        assert!(!one.content_location_conflict);
        assert_eq!(one.content_path.as_deref(), Some("mod/a"));
        assert!(one.content_sha256.is_some());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn content_sha256_catches_what_the_tuple_fingerprint_misses() {
        // G2-regen finding 3: (count, bytes, max-mtime) collides on renames
        // and same-length edits. The sha256 binding must not.
        let dir = temp_dir("sha");
        let content = dir.join("mod").join("m");
        fs::create_dir_all(&content).unwrap();
        fs::write(content.join("a.txt"), b"hello").unwrap();
        fs::write(content.join("keep.txt"), b"zzzz").unwrap();
        fs::write(dir.join("mod").join("m.mod"), "path=\"mod/m\"\n").unwrap();

        let before = resolve_mod_descriptor(&dir, "mod/m.mod");
        let sha_before = before.content_sha256.clone().expect("sha");
        let (files_b, bytes_b, _) = before.content_fingerprint.expect("tuple");

        // RENAME: file mtime is preserved, count and bytes unchanged — the
        // old tuple is blind to this; the sha must move.
        fs::rename(content.join("a.txt"), content.join("b.txt")).unwrap();
        let renamed = resolve_mod_descriptor(&dir, "mod/m.mod");
        let (files_r, bytes_r, _) = renamed.content_fingerprint.expect("tuple");
        assert_eq!(
            (files_b, bytes_b),
            (files_r, bytes_r),
            "tuple collides on rename"
        );
        assert_ne!(
            renamed.content_sha256.as_deref(),
            Some(sha_before.as_str()),
            "sha256 must catch a rename"
        );

        // SAME-LENGTH EDIT: count/bytes unchanged; sha must move.
        let sha_renamed = renamed.content_sha256.clone().unwrap();
        fs::write(content.join("b.txt"), b"HELLO").unwrap();
        let edited = resolve_mod_descriptor(&dir, "mod/m.mod");
        let (files_e, bytes_e, _) = edited.content_fingerprint.expect("tuple");
        assert_eq!((files_r, bytes_r), (files_e, bytes_e));
        assert_ne!(edited.content_sha256.as_deref(), Some(sha_renamed.as_str()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn user_dir_corroboration_requires_pid_held_log_handle() {
        // Modset re-gate finding 1: mtime liveness was not ownership (any
        // writer touching logs/ passed). Ownership = the attached PID holds
        // an open handle to a file under logs/ (Restart Manager query).
        let dir = temp_dir("corrob");
        let my_pid = std::process::id();

        // No logs dir at all -> fail closed.
        assert!(corroborate_user_dir_live(&dir, my_pid).is_err());

        // Empty logs dir -> fail closed.
        fs::create_dir_all(dir.join("logs")).unwrap();
        assert!(corroborate_user_dir_live(&dir, my_pid).is_err());

        // A log file EXISTS but nobody holds it open -> mere presence (or a
        // third-party write) is NOT ownership -> fail closed.
        let log = dir.join("logs").join("game.log");
        fs::write(&log, b"[12:00] boot").unwrap();
        let err = corroborate_user_dir_live(&dir, my_pid).unwrap_err();
        assert!(err.contains("holds no open handle"), "{err}");

        // THIS process holds the handle -> corroborated for OUR pid...
        let held =
            portable_test_fs_retry::open_with(fs::OpenOptions::new().append(true), &log).unwrap();
        assert!(corroborate_user_dir_live(&dir, my_pid).is_ok());
        // ...but NOT for a different pid (the wrong-process case).
        assert!(corroborate_user_dir_live(&dir, my_pid.wrapping_add(7919)).is_err());
        drop(held);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn user_dir_corroboration_refuses_junction_redirected_logs() {
        // Fix-round re-gate finding 1: a WRONG user dir whose logs/ is a
        // JUNCTION into the real logs dir must NOT corroborate — Restart
        // Manager would truthfully report the attached PID holding the
        // junction TARGET, corroborating a directory whose dlc_load.json the
        // process never read. Junction creation (mklink /J) needs no
        // privileges or Developer Mode, unlike file symlinks.
        let real = temp_dir("junct_real");
        let fake = temp_dir("junct_fake");
        let my_pid = std::process::id();
        let real_logs = real.join("logs");
        fs::create_dir_all(&real_logs).unwrap();
        let log = real_logs.join("game.log");
        fs::write(&log, b"[12:00] boot").unwrap();
        let held =
            portable_test_fs_retry::open_with(fs::OpenOptions::new().append(true), &log).unwrap();
        // Sanity: the REAL dir corroborates while we hold the handle.
        corroborate_user_dir_live(&real, my_pid).unwrap();

        let fake_logs = fake.join("logs");
        // Retried only when Windows failed to *start* `cmd.exe`: under a full
        // `--workspace` run, concurrent process creation intermittently dies
        // with `STATUS_DLL_INIT_FAILED` (`0xC000_0142`) before `mklink` ever
        // runs. `mklink`'s own refusals use exit code 1 and still fail on the
        // first attempt, so this cannot mask a junction that was not created —
        // and the `is_file` assertion below independently proves the junction
        // really resolves before the refusal being tested is asserted. The code
        // is matched exactly rather than by the NTSTATUS range `>= 0xC000_0000`,
        // which is not exclusive to the loader: a child may pass any 32-bit value
        // to `ExitProcess`, `TerminateProcess` may impose one, and an unhandled
        // exception surfaces as its own status code.
        const MAX_ATTEMPTS: u32 = 5;
        const STATUS_DLL_INIT_FAILED: u32 = 0xC000_0142;
        for attempt in 1..=MAX_ATTEMPTS {
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(&fake_logs)
                .arg(&real_logs)
                .status()
                .expect("spawn cmd mklink /J");
            if status.success() {
                break;
            }
            let child_never_ran = status
                .code()
                .is_some_and(|code| code as u32 == STATUS_DLL_INIT_FAILED);
            assert!(
                child_never_ran && attempt < MAX_ATTEMPTS,
                "mklink /J failed: {status}"
            );
            std::thread::sleep(std::time::Duration::from_millis(50 * u64::from(attempt)));
        }
        // The junction genuinely reaches the held file — exactly the state
        // that fooled the pre-fix check...
        assert!(fake_logs.join("game.log").is_file());
        // ...but corroboration refuses the alias.
        let err = corroborate_user_dir_live(&fake, my_pid).unwrap_err();
        assert!(
            err.contains("refusing to corroborate through an alias"),
            "{err}"
        );
        drop(held);

        fs::remove_dir_all(&fake).ok();
        fs::remove_dir_all(&real).ok();
    }

    #[test]
    fn empty_content_path_leaves_binding_incomplete() {
        let dir = temp_dir("emptypath");
        fs::create_dir_all(dir.join("mod")).unwrap();
        fs::write(dir.join("mod").join("e.mod"), "name=\"E\"\npath=\"\"\n").unwrap();
        let resolved = resolve_mod_descriptor(&dir, "mod/e.mod");
        // An empty path= names no content location -> None -> caller fails closed.
        assert!(resolved.content_path.is_none());
        assert!(resolved.content_fingerprint.is_none());
        fs::remove_dir_all(&dir).ok();
    }

}
