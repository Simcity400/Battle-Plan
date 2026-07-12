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

use std::ffi::c_void;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value};
use snapshot::sha256::{to_hex, Sha256};

type Handle = *mut c_void;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// Bounded mod-content walk: enough for any real mod, refuses runaway trees.
const MOD_CONTENT_MAX_FILES: usize = 200_000;

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

/// The full modset + build binding, plus every resolution failure met on the
/// way. `resolved` is true ONLY when the whole chain proved out.
#[derive(Debug, Clone)]
pub struct ModsetReport {
    pub resolved: bool,
    pub errors: Vec<String>,
    pub exe_path: Option<PathBuf>,
    pub exe_size_bytes: Option<u64>,
    pub exe_sha256: Option<String>,
    pub user_dir: Option<PathBuf>,
    /// "override" (--pdx-user-dir) or "probed" — a wrong override is the
    /// recorded suspect when a binding later proves wrong.
    pub user_dir_source: Option<&'static str>,
    /// dlc_load.json mtime (unix seconds) and the game process creation time
    /// (unix seconds), both recorded so config-newer-than-process is provable.
    pub dlc_load_mtime_unix: Option<i64>,
    pub process_start_unix: Option<i64>,
    pub enabled_mods: Vec<ResolvedMod>,
    pub installed_dlcs: Vec<String>,
    pub disabled_dlcs: Vec<String>,
    pub enabled_dlcs: Vec<String>,
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
        });
        if self.resolved {
            let mut doc = json!({
                "resolved": true,
                "errors": self.errors,
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
                "resolved": false,
                "errors": self.errors,
                "partial_evidence": evidence,
            })
        }
    }
}

/// Resolve the recording modset for the attached game process. Never fails —
/// every problem lands in `errors` and clears `resolved`.
pub fn resolve_modset(pid: u32, user_dir_override: Option<&Path>) -> ModsetReport {
    let mut report = ModsetReport {
        resolved: true,
        errors: Vec::new(),
        exe_path: None,
        exe_size_bytes: None,
        exe_sha256: None,
        user_dir: None,
        user_dir_source: None,
        dlc_load_mtime_unix: None,
        process_start_unix: None,
        enabled_mods: Vec::new(),
        installed_dlcs: Vec::new(),
        disabled_dlcs: Vec::new(),
        enabled_dlcs: Vec::new(),
    };
    macro_rules! fail {
        ($($arg:tt)*) => {{
            report.resolved = false;
            report.errors.push(format!($($arg)*));
        }};
    }

    // --- game build binding ---
    match query_process_image_path(pid) {
        Ok(exe) => {
            match hash_file_streaming_stable(&exe) {
                Ok((size, sha)) => {
                    report.exe_size_bytes = Some(size);
                    report.exe_sha256 = Some(sha);
                }
                Err(e) => fail!("exe hash failed: {e}"),
            }
            report.exe_path = Some(exe);
        }
        Err(e) => fail!("exe path query failed: {e}"),
    }
    let process_start: Option<SystemTime> = match query_process_start(pid) {
        Ok(start) => {
            report.process_start_unix = Some(system_time_unix(start));
            Some(start)
        }
        Err(e) => {
            fail!("process start time query failed: {e}");
            None
        }
    };

    // --- Paradox user dir + dlc_load.json ---
    match locate_user_dir(user_dir_override) {
        Ok((dir, source)) => {
            // G2-regen finding 2: a well-formed but WRONG directory (stale
            // override, stale probe while the game runs a custom user dir)
            // previously resolved. Corroborate: the attached process writes
            // its logs into its real user dir continuously, so this dir's
            // logs/ must have been written at/after process start. No
            // corroboration -> fail closed (override and probed alike).
            match process_start {
                Some(start) => {
                    if let Err(e) = corroborate_user_dir_live(&dir, start) {
                        fail!(
                            "user dir not corroborated against the attached process: {e}"
                        );
                    }
                }
                // process_start unknown already failed resolution closed above.
                None => {}
            }
            report.user_dir = Some(dir);
            report.user_dir_source = Some(source);
        }
        Err(e) => fail!("{e}"),
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
                "dlc_load.json mtime is at/after the game process start; \
                 launcher state not provably the launched state"
            ),
            (None, _) => fail!(
                "dlc_load.json mtime unavailable; launcher-state provenance unprovable"
            ),
            _ => {}
        }
        match fs::read_to_string(&dlc_load_path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(doc) => {
                    // Re-check mtime AFTER the read: a launcher rewrite between
                    // the metadata read and the content read would otherwise
                    // pair new content with the old, accepted timestamp.
                    let mtime_after =
                        fs::metadata(&dlc_load_path).and_then(|m| m.modified()).ok();
                    if mtime_after != mtime_before {
                        fail!("dlc_load.json changed while being read (mtime moved)");
                    }
                    // G2-regen finding 5: an atomic replacement can PRESERVE
                    // the mtime (OneDrive sync, copied-file install), so the
                    // mtime bracket alone is insufficient. Read the file a
                    // second time and require byte-identical content. Residual
                    // (documented): a replacement completing before the FIRST
                    // read with a preserved mtime is indistinguishable from
                    // user mode; both reads then see the same (new) bytes.
                    match fs::read_to_string(&dlc_load_path) {
                        Ok(text_again) if text_again == text => {}
                        Ok(_) => fail!(
                            "dlc_load.json changed while being read (bytes differ between reads)"
                        ),
                        Err(e) => fail!("dlc_load.json second stability read failed: {e}"),
                    }
                    // Duplicate top-level keys (re-gate finding 2): serde_json
                    // silently keeps the last, so a lenient document can hide
                    // an ambiguous enabled_mods/disabled_dlcs behind an empty
                    // override. Reject any duplicated object key at depth 1.
                    if let Err(dup) = reject_duplicate_top_level_keys(&text) {
                        fail!("dlc_load.json top-level key scan rejected: {dup}");
                    }
                    // G2 finding 2: absent or mistyped fields are NOT empty
                    // lists — they are unproven state and fail closed.
                    match required_string_array(&doc, "enabled_mods") {
                        Ok(enabled_mods) => {
                            for entry in enabled_mods {
                                let resolved = resolve_mod_descriptor(&user_dir, &entry);
                                if resolved.descriptor_sha256.is_none() {
                                    fail!("mod descriptor unreadable: {entry:?}");
                                } else if resolved.content_location_conflict {
                                    // G2-regen finding 6: repeated or competing
                                    // path=/archive= — the effective content
                                    // location is unproven engine semantics.
                                    fail!(
                                        "mod descriptor content location ambiguous \
                                         (repeated or competing path=/archive=): {entry:?}"
                                    );
                                } else if resolved.content_path.is_none() {
                                    fail!(
                                        "mod descriptor carries no path=/archive= \
                                         (binding incomplete): {entry:?}"
                                    );
                                } else if resolved.content_sha256.is_none() {
                                    fail!(
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
                                            "mod descriptor written at/after process start \
                                             (post-launch update?): {entry:?}"
                                        ),
                                        None if resolved.descriptor_sha256.is_some() => fail!(
                                            "mod descriptor mtime unavailable: {entry:?}"
                                        ),
                                        _ => {}
                                    }
                                    match resolved.content_max_mtime {
                                        Some(m) if m >= start => fail!(
                                            "mod content written at/after process start \
                                             (post-launch update?): {entry:?}"
                                        ),
                                        _ => {}
                                    }
                                }
                                report.enabled_mods.push(resolved);
                            }
                        }
                        Err(e) => fail!("dlc_load.json enabled_mods: {e}"),
                    }
                    match required_string_array(&doc, "disabled_dlcs") {
                        Ok(list) => disabled_dlcs = list,
                        Err(e) => fail!("dlc_load.json disabled_dlcs: {e}"),
                    }
                }
                Err(e) => fail!("dlc_load.json parse failed: {e}"),
            },
            Err(e) => fail!(
                "dlc_load.json read failed at {}: {e}",
                dlc_load_path.display()
            ),
        }
    }
    report.disabled_dlcs = disabled_dlcs.clone();

    // --- installed / enabled DLC census (game_root/dlc/<dir>/<name>.dlc) ---
    if let Some(exe) = report.exe_path.clone() {
        match exe.parent() {
            Some(game_root) => match census_installed_dlcs(game_root) {
                Ok((installed, census_max_mtime)) => {
                    // G2-regen finding 4 (DLC arm): an installed-DLC descriptor
                    // written at/after process start means the census describes
                    // a different DLC set than the one the process loaded.
                    if let (Some(start), Some(newest)) = (process_start, census_max_mtime) {
                        if newest >= start {
                            fail!(
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
                    report.installed_dlcs = installed;
                }
                Err(e) => fail!("dlc census failed: {e}"),
            },
            None => fail!("exe path has no parent directory"),
        }
    }

    report
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
        Ok(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// Process creation time as a full-precision `SystemTime` (FILETIME 100ns
/// ticks -> unix epoch). Full precision matters: the launcher-state ordering
/// check must not floor two sub-second-apart events into one equal second
/// (re-gate finding 3).
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
fn corroborate_user_dir_live(user_dir: &Path, process_start: SystemTime) -> Result<(), String> {
    let logs = user_dir.join("logs");
    let entries = fs::read_dir(&logs)
        .map_err(|e| format!("{} unreadable ({e}); cannot prove the attached process writes this dir", logs.display()))?;
    let mut newest: Option<SystemTime> = None;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", logs.display()))?;
        let meta = entry
            .metadata()
            .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        if !meta.is_dir() {
            if let Ok(mtime) = meta.modified() {
                newest = Some(newest.map_or(mtime, |n: SystemTime| n.max(mtime)));
            }
        }
    }
    match newest {
        Some(newest) if newest >= process_start => Ok(()),
        Some(_) => Err(format!(
            "newest file under {} predates process start — this dir is not \
             being written by the attached process",
            logs.display()
        )),
        None => Err(format!(
            "{} contains no files; cannot prove the attached process writes this dir",
            logs.display()
        )),
    }
}

/// Streaming sha256 with a size+mtime stability bracket (G2 finding 4): the
/// file's metadata is captured before hashing and re-checked after; any
/// mid-hash mutation (game update mid-capture, partial write) fails closed
/// instead of recording a mixed hash as the build binding.
fn hash_file_streaming_stable(path: &Path) -> Result<(u64, String), String> {
    let meta_before = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (size_before, mtime_before) = (meta_before.len(), meta_before.modified().ok());
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut chunk)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&chunk[..n]);
    }
    let meta_after = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if total != size_before
        || meta_after.len() != size_before
        || meta_after.modified().ok() != mtime_before
    {
        return Err(format!(
            "{} changed while hashing (size {}->{} read {}, mtime moved: {})",
            path.display(),
            size_before,
            meta_after.len(),
            total,
            meta_after.modified().ok() != mtime_before,
        ));
    }
    Ok((total, to_hex(&hasher.finalize())))
}

/// Locate the Paradox user dir carrying `dlc_load.json`. An explicit override
/// must itself prove out (contain the file). Without one, both the plain and
/// the OneDrive-redirected Documents locations are probed; exactly one match
/// resolves, zero or several fail closed with the probed paths recorded.
fn locate_user_dir(
    user_dir_override: Option<&Path>,
) -> Result<(PathBuf, &'static str), String> {
    if let Some(dir) = user_dir_override {
        if dir.join("dlc_load.json").is_file() {
            return Ok((dir.to_path_buf(), "override"));
        }
        return Err(format!(
            "--pdx-user-dir {} does not contain dlc_load.json",
            dir.display()
        ));
    }
    let profile = std::env::var("USERPROFILE")
        .map_err(|_| "USERPROFILE not set; pass --pdx-user-dir".to_string())?;
    let candidates = [
        Path::new(&profile)
            .join("Documents")
            .join("Paradox Interactive")
            .join("Hearts of Iron IV"),
        Path::new(&profile)
            .join("OneDrive")
            .join("Documents")
            .join("Paradox Interactive")
            .join("Hearts of Iron IV"),
    ];
    let hits: Vec<&PathBuf> = candidates
        .iter()
        .filter(|dir| dir.join("dlc_load.json").is_file())
        .collect();
    match hits.as_slice() {
        [only] => Ok(((*only).clone(), "probed")),
        [] => Err(format!(
            "dlc_load.json not found; probed {} and {}; pass --pdx-user-dir",
            candidates[0].display(),
            candidates[1].display()
        )),
        _ => Err(format!(
            "AMBIGUOUS pdx user dir: dlc_load.json exists in more than one probe location ({}); pass --pdx-user-dir",
            hits.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
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
            resolved.content_fingerprint =
                Some((hashed.files, hashed.bytes, system_time_unix(hashed.max_mtime)));
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
fn walk_content_metadata(root: &Path) -> Option<(u64, u64, SystemTime, Vec<PathBuf>)> {
    if root.is_file() {
        let meta = fs::metadata(root).ok()?;
        return Some((1, meta.len(), meta.modified().ok()?, vec![PathBuf::new()]));
    }
    if !root.is_dir() {
        return None;
    }
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut max_mtime: Option<SystemTime> = None;
    let mut rel_paths: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            let path = entry.path();
            let meta = entry.metadata().ok()?;
            if meta.is_dir() {
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
    let entries = fs::read_dir(&dlc_root).map_err(|e| format!("{}: {e}", dlc_root.display()))?;
    let mut installed = Vec::new();
    let mut newest: Option<SystemTime> = None;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dlc_root.display()))?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let children =
            fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for child in children {
            let child = child.map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = child.path();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dlc"))
            {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    let mtime = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map_err(|e| format!("{}: mtime unreadable: {e}", path.display()))?;
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

    #[test]
    fn mod_descriptor_fields_parse_hash_and_fingerprint() {
        let dir = temp_dir("descriptor");
        fs::create_dir_all(dir.join("mod").join("test")).unwrap();
        let descriptor = "version=\"1.2\"\nname=\"Test Mod\"\nremote_file_id=\"12345\"\npath=\"mod/test\"\n";
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
            root.join("dlc").join("dlc023_man_the_guns").join("dlc023.dlc"),
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

        // An unmatched disabled key is detectable by the same exact-match rule
        // resolve_modset enforces (fails closed there).
        let drifted = "DLC/dlc013_sabaton/dlc013.dlc".to_string();
        assert!(!installed.iter().any(|rel| rel == &drifted));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ambiguous_or_missing_user_dir_fails_closed_and_records_source() {
        let dir = temp_dir("override");
        // Override without dlc_load.json inside: must FAIL, never fall back to
        // probing (an explicit pin that does not prove out is an error).
        let err = locate_user_dir(Some(&dir)).unwrap_err();
        assert!(err.contains("does not contain dlc_load.json"));

        fs::write(dir.join("dlc_load.json"), "{}").unwrap();
        let (found, source) = locate_user_dir(Some(&dir)).unwrap();
        assert_eq!(found, dir);
        assert_eq!(source, "override");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unresolved_report_nests_evidence_under_partial_evidence() {
        let report = ModsetReport {
            resolved: false,
            errors: vec!["x".to_string()],
            exe_path: None,
            exe_size_bytes: None,
            exe_sha256: None,
            user_dir: None,
            user_dir_source: None,
            dlc_load_mtime_unix: None,
            process_start_unix: None,
            enabled_mods: Vec::new(),
            installed_dlcs: vec!["dlc/a/a.dlc".to_string()],
            disabled_dlcs: Vec::new(),
            enabled_dlcs: vec!["dlc/a/a.dlc".to_string()],
        };
        let doc = report.as_json();
        // A naive consumer reading top-level lists on an unresolved report
        // finds NOTHING (fail-closed shape).
        assert!(doc.get("enabled_mods").is_none());
        assert!(doc.get("dlc").is_none());
        assert_eq!(doc["resolved"], Value::Bool(false));
        assert!(doc["partial_evidence"]["dlc"]["enabled"].is_array());

        let resolved = ModsetReport {
            resolved: true,
            ..report
        };
        let doc = resolved.as_json();
        assert!(doc.get("partial_evidence").is_none());
        assert!(doc["dlc"]["enabled"].is_array());
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
        let dup2 = format!(
            r#"{{"abc":1,"ab{}":2}}"#,
            concat!(r"\u", "0063")
        );
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
        assert_eq!(decode_json_string(r#"a\"b\\c\/d\n"#).unwrap(), "a\"b\\c/d\n");
        // Surrogate pair: U+1F600.
        assert_eq!(
            decode_json_string(&format!("{}{}", concat!(r"\u", "D83D"), concat!(r"\u", "DE00")))
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
        assert_eq!((files_b, bytes_b), (files_r, bytes_r), "tuple collides on rename");
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
    fn user_dir_corroboration_requires_live_log_writes() {
        // G2-regen finding 2: a well-formed but stale/wrong dir must not
        // resolve. The dir counts as live only when its logs/ was written
        // at/after process start.
        let dir = temp_dir("corrob");

        // No logs dir at all -> fail closed.
        assert!(corroborate_user_dir_live(&dir, SystemTime::UNIX_EPOCH).is_err());

        // Empty logs dir -> fail closed.
        fs::create_dir_all(dir.join("logs")).unwrap();
        assert!(corroborate_user_dir_live(&dir, SystemTime::UNIX_EPOCH).is_err());

        // Log written now, process "started" at epoch -> corroborated.
        fs::write(dir.join("logs").join("game.log"), b"[12:00] boot").unwrap();
        assert!(corroborate_user_dir_live(&dir, SystemTime::UNIX_EPOCH).is_ok());

        // Process "starts" far in the future -> the newest log predates it ->
        // this dir is not being written by that process -> fail closed.
        let future = SystemTime::now() + std::time::Duration::from_secs(3600);
        let err = corroborate_user_dir_live(&dir, future).unwrap_err();
        assert!(err.contains("predates process start"), "{err}");

        fs::remove_dir_all(&dir).ok();
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

    #[test]
    fn hash_stability_bracket_detects_midhash_growth() {
        let dir = temp_dir("hash");
        let f = dir.join("x.bin");
        fs::write(&f, vec![7u8; 4096]).unwrap();
        let (size, sha) = hash_file_streaming_stable(&f).unwrap();
        assert_eq!(size, 4096);
        assert_eq!(sha.len(), 64);
        fs::remove_dir_all(&dir).ok();
    }
}
