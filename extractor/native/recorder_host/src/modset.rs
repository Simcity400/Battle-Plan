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
//! Sources of truth read here:
//! - hoi4.exe image path via the attached PID (`QueryFullProcessImageNameW`) —
//!   never a hardcoded install path; the exe sha256 is the build binding.
//! - `<pdx_user_dir>/dlc_load.json` — the launcher-written enabled-mods +
//!   disabled-DLCs document (observed schema:
//!   `{"enabled_mods":[".mod paths"],"disabled_dlcs":["dlc/.../dlcNNN.dlc"]}`).
//! - each enabled mod's `.mod` descriptor (name/version/remote_file_id/path),
//!   content-hashed so two captures can prove they ran the same mod text.
//! - `<game_root>/dlc/*/​*.dlc` descriptors for the installed-DLC census;
//!   enabled = installed minus `disabled_dlcs`.
//!
//! The Paradox user dir is probed (`Documents` and OneDrive-redirected
//! `OneDrive/Documents` under `%USERPROFILE%`) unless `--pdx-user-dir` pins it.
//! If MORE THAN ONE probe candidate carries a `dlc_load.json` the resolution is
//! AMBIGUOUS and fails closed (both candidates recorded) — pass the flag.

use std::ffi::c_void;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use snapshot::sha256::{to_hex, Sha256};

type Handle = *mut c_void;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

extern "system" {
    fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        exe_name: *mut u16,
        size: *mut u32,
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
    pub enabled_mods: Vec<ResolvedMod>,
    pub installed_dlcs: Vec<String>,
    pub disabled_dlcs: Vec<String>,
    pub enabled_dlcs: Vec<String>,
}

impl ModsetReport {
    pub fn as_json(&self) -> Value {
        json!({
            "resolved": self.resolved,
            "errors": self.errors,
            "game": {
                "exe_path": self.exe_path.as_ref().map(|p| p.display().to_string()),
                "exe_size_bytes": self.exe_size_bytes,
                "exe_sha256": self.exe_sha256,
            },
            "pdx_user_dir": self.user_dir.as_ref().map(|p| p.display().to_string()),
            "enabled_mods": self.enabled_mods.iter().map(|m| json!({
                "descriptor_entry": m.descriptor_entry,
                "descriptor_path": m.descriptor_path.as_ref().map(|p| p.display().to_string()),
                "descriptor_sha256": m.descriptor_sha256,
                "name": m.name,
                "version": m.version,
                "remote_file_id": m.remote_file_id,
                "content_path": m.content_path,
            })).collect::<Vec<_>>(),
            "dlc": {
                "installed": self.installed_dlcs,
                "disabled": self.disabled_dlcs,
                "enabled": self.enabled_dlcs,
            },
        })
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
        enabled_mods: Vec::new(),
        installed_dlcs: Vec::new(),
        disabled_dlcs: Vec::new(),
        enabled_dlcs: Vec::new(),
    };

    // --- game build binding ---
    match query_process_image_path(pid) {
        Ok(exe) => {
            match hash_file_streaming(&exe) {
                Ok((size, sha)) => {
                    report.exe_size_bytes = Some(size);
                    report.exe_sha256 = Some(sha);
                }
                Err(e) => {
                    report.resolved = false;
                    report.errors.push(format!("exe hash failed: {e}"));
                }
            }
            report.exe_path = Some(exe);
        }
        Err(e) => {
            report.resolved = false;
            report.errors.push(format!("exe path query failed: {e}"));
        }
    }

    // --- Paradox user dir + dlc_load.json ---
    let user_dir = match locate_user_dir(user_dir_override) {
        Ok(dir) => Some(dir),
        Err(e) => {
            report.resolved = false;
            report.errors.push(e);
            None
        }
    };
    report.user_dir = user_dir.clone();

    let mut disabled_dlcs: Vec<String> = Vec::new();
    if let Some(user_dir) = &user_dir {
        let dlc_load_path = user_dir.join("dlc_load.json");
        match fs::read_to_string(&dlc_load_path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(doc) => {
                    let enabled_mods = string_array(&doc, "enabled_mods");
                    disabled_dlcs = string_array(&doc, "disabled_dlcs");
                    for entry in enabled_mods {
                        let resolved = resolve_mod_descriptor(user_dir, &entry);
                        if resolved.descriptor_sha256.is_none() {
                            report.resolved = false;
                            report.errors.push(format!(
                                "mod descriptor unreadable: {entry:?}"
                            ));
                        }
                        report.enabled_mods.push(resolved);
                    }
                }
                Err(e) => {
                    report.resolved = false;
                    report
                        .errors
                        .push(format!("dlc_load.json parse failed: {e}"));
                }
            },
            Err(e) => {
                report.resolved = false;
                report.errors.push(format!(
                    "dlc_load.json read failed at {}: {e}",
                    dlc_load_path.display()
                ));
            }
        }
    }
    report.disabled_dlcs = disabled_dlcs.clone();

    // --- installed / enabled DLC census (game_root/dlc/<dir>/<name>.dlc) ---
    if let Some(exe) = &report.exe_path {
        match exe.parent() {
            Some(game_root) => match census_installed_dlcs(game_root) {
                Ok(installed) => {
                    report.enabled_dlcs = installed
                        .iter()
                        .filter(|rel| !disabled_dlcs.iter().any(|d| d == *rel))
                        .cloned()
                        .collect();
                    report.installed_dlcs = installed;
                }
                Err(e) => {
                    report.resolved = false;
                    report.errors.push(format!("dlc census failed: {e}"));
                }
            },
            None => {
                report.resolved = false;
                report
                    .errors
                    .push("exe path has no parent directory".to_string());
            }
        }
    }

    report
}

fn string_array(doc: &Value, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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

/// Streaming sha256 so hashing the ~60MB exe never spikes the recorder's peak
/// private commit (the G5 bounded-memory headline reads from the same manifest).
fn hash_file_streaming(path: &Path) -> Result<(u64, String), String> {
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
    Ok((total, to_hex(&hasher.finalize())))
}

/// Locate the Paradox user dir carrying `dlc_load.json`. An explicit override
/// must itself prove out (contain the file). Without one, both the plain and
/// the OneDrive-redirected Documents locations are probed; exactly one match
/// resolves, zero or several fail closed with the probed paths recorded.
fn locate_user_dir(user_dir_override: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(dir) = user_dir_override {
        if dir.join("dlc_load.json").is_file() {
            return Ok(dir.to_path_buf());
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
        [only] => Ok((*only).clone()),
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
/// stay None (recorded, never fabricated).
fn resolve_mod_descriptor(user_dir: &Path, entry: &str) -> ResolvedMod {
    let mut resolved = ResolvedMod {
        descriptor_entry: entry.to_string(),
        descriptor_path: None,
        descriptor_sha256: None,
        name: None,
        version: None,
        remote_file_id: None,
        content_path: None,
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
    resolved.descriptor_sha256 = Some(Sha256::hex(text.as_bytes()));
    resolved.descriptor_path = Some(candidate);
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "name" => resolved.name = Some(value),
            "version" => resolved.version = Some(value),
            "remote_file_id" => resolved.remote_file_id = Some(value),
            "path" | "archive" => resolved.content_path = Some(value),
            _ => {}
        }
    }
    resolved
}

/// Enumerate installed DLC descriptors as game-root-relative
/// forward-slash paths (`dlc/dlc023_man_the_guns/dlc023.dlc`) — the exact key
/// format `dlc_load.json.disabled_dlcs` uses, so the subtraction is a string
/// match with no normalization guesswork.
fn census_installed_dlcs(game_root: &Path) -> Result<Vec<String>, String> {
    let dlc_root = game_root.join("dlc");
    let entries = fs::read_dir(&dlc_root).map_err(|e| format!("{}: {e}", dlc_root.display()))?;
    let mut installed = Vec::new();
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
            if path.extension().and_then(|e| e.to_str()) == Some("dlc") {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    installed.push(format!("dlc/{dir_name}/{file_name}"));
                }
            }
        }
    }
    installed.sort();
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_descriptor_fields_parse_and_hash() {
        let dir = std::env::temp_dir().join(format!("modset_test_{}", std::process::id()));
        fs::create_dir_all(dir.join("mod")).unwrap();
        let descriptor = "version=\"1.2\"\nname=\"Test Mod\"\nremote_file_id=\"12345\"\npath=\"mod/test\"\n";
        fs::write(dir.join("mod").join("test.mod"), descriptor).unwrap();

        let resolved = resolve_mod_descriptor(&dir, "mod/test.mod");
        assert_eq!(resolved.name.as_deref(), Some("Test Mod"));
        assert_eq!(resolved.version.as_deref(), Some("1.2"));
        assert_eq!(resolved.remote_file_id.as_deref(), Some("12345"));
        assert_eq!(resolved.content_path.as_deref(), Some("mod/test"));
        assert_eq!(
            resolved.descriptor_sha256.as_deref(),
            Some(Sha256::hex(descriptor.as_bytes()).as_str())
        );

        // A missing descriptor stays evidence-free (caller records the error).
        let missing = resolve_mod_descriptor(&dir, "mod/absent.mod");
        assert!(missing.descriptor_sha256.is_none());
        assert!(missing.descriptor_path.is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dlc_census_emits_dlc_load_relative_keys_and_subtraction_matches() {
        let root = std::env::temp_dir().join(format!("modset_dlc_test_{}", std::process::id()));
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

        let installed = census_installed_dlcs(&root).unwrap();
        assert_eq!(
            installed,
            vec![
                "dlc/dlc013_sabaton/dlc013.dlc".to_string(),
                "dlc/dlc023_man_the_guns/dlc023.dlc".to_string(),
            ]
        );
        // The disabled_dlcs entries observed in a real dlc_load.json use this
        // exact relative form — subtraction must be a plain string match.
        let disabled = vec!["dlc/dlc013_sabaton/dlc013.dlc".to_string()];
        let enabled: Vec<&String> = installed
            .iter()
            .filter(|rel| !disabled.iter().any(|d| d == *rel))
            .collect();
        assert_eq!(enabled, vec!["dlc/dlc023_man_the_guns/dlc023.dlc"]);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ambiguous_or_missing_user_dir_fails_closed() {
        let dir = std::env::temp_dir().join(format!("modset_override_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // Override without dlc_load.json inside: must FAIL, never fall back to
        // probing (an explicit pin that does not prove out is an error).
        let err = locate_user_dir(Some(&dir)).unwrap_err();
        assert!(err.contains("does not contain dlc_load.json"));

        fs::write(dir.join("dlc_load.json"), "{}").unwrap();
        assert_eq!(locate_user_dir(Some(&dir)).unwrap(), dir);

        fs::remove_dir_all(&dir).ok();
    }
}
