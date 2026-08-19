use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupportedGameBuild {
    pub display_version: &'static str,
    pub exe_sha256: [u8; 32],
}

pub fn supported_game_builds() -> &'static [SupportedGameBuild] {
    &[SupportedGameBuild {
        display_version: "Operation Postern v1.19.2.0.a729 (d245)",
        exe_sha256: compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256,
    }]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionSource {
    LauncherSettingsVersion,
    LauncherSettingsRawVersion,
    BranchAndRevisionFiles,
    Unknown,
}

impl VersionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LauncherSettingsVersion => "launcher-settings.json:version",
            Self::LauncherSettingsRawVersion => "launcher-settings.json:rawVersion",
            Self::BranchAndRevisionFiles => "hoi4_branch.txt+hoi4_rev.txt",
            Self::Unknown => "unknown",
        }
    }
}

pub const UNKNOWN_VERSION_TEXT: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedGameBuild {
    pub display_version: String,
    pub version_source: VersionSource,
    pub exe_sha256: Option<[u8; 32]>,
    pub exe_read_error: Option<String>,
    pub exe_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameBuildVerdict {
    Supported {
        display_version: String,
        exe_sha256: [u8; 32],
    },
    Unsupported(UnsupportedGameBuild),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedGameBuild {
    pub observed: ObservedGameBuild,
}

pub fn short_fingerprint(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(12);
    for byte in &digest[..6] {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn full_hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

impl UnsupportedGameBuild {
    pub fn message(&self) -> String {
        let observed_version = self.observed.display_version.as_str();
        let supported = supported_game_builds();
        let supported_versions = supported
            .iter()
            .map(|build| {
                format!(
                    "{} (game file {})",
                    build.display_version,
                    short_fingerprint(&build.exe_sha256)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let observed_file = match self.observed.exe_sha256 {
            Some(digest) => format!("game file {}", short_fingerprint(&digest)),
            None => "game file could not be read".to_string(),
        };
        format!(
            "This game version ({observed_version}) is not supported yet \u{2014} an update to the \
             recorder will add it.\n\
             Nothing was changed in your game, and no recording was started.\n\
             What to do: wait for a recorder update that adds this version.\n\
             This recorder supports: {supported_versions}.\n\
             Your game reports: {observed_version} ({observed_file})."
        )
    }

    pub const TERMINAL_OUTCOME: &'static str = "unsupported_game_build";

    pub fn receipt_json(&self) -> Value {
        json!({
            "schema": UNSUPPORTED_GAME_BUILD_SCHEMA,
            "terminal_outcome": Self::TERMINAL_OUTCOME,
            "message": self.message(),
            "recording_started": false,
            "game_modified": false,
            "exit_code": UNSUPPORTED_GAME_BUILD_EXIT_CODE,
            "observed": {
                "display_version": self.observed.display_version,
                "version_source": self.observed.version_source.as_str(),
                "exe_path": self.observed.exe_path.display().to_string(),
                "exe_sha256": self.observed.exe_sha256.as_ref().map(full_hex),
                "exe_read_error": self.observed.exe_read_error,
            },
            "supported": supported_game_builds()
                .iter()
                .map(|build| json!({
                    "display_version": build.display_version,
                    "exe_sha256": full_hex(&build.exe_sha256),
                }))
                .collect::<Vec<_>>(),
        })
    }
}

pub const UNSUPPORTED_GAME_BUILD_RECEIPT_FILE: &str = "GAME_BUILD_UNSUPPORTED_V1.json";
pub const UNSUPPORTED_GAME_BUILD_SCHEMA: &str = "hoi4-game-build-unsupported-v1";

pub const UNSUPPORTED_GAME_BUILD_EXIT_CODE: i32 = 3;

static REFUSAL_EXIT_CODE: OnceLock<i32> = OnceLock::new();

pub fn arm_refusal_exit_code() {
    let _ = REFUSAL_EXIT_CODE.set(UNSUPPORTED_GAME_BUILD_EXIT_CODE);
}

pub fn refusal_exit_code() -> Option<i32> {
    REFUSAL_EXIT_CODE.get().copied()
}

static REFUSAL_PRESENTED: AtomicBool = AtomicBool::new(false);

pub fn mark_refusal_presented() {
    REFUSAL_PRESENTED.store(true, Ordering::Release);
}

pub fn refusal_already_presented() -> bool {
    REFUSAL_PRESENTED.load(Ordering::Acquire)
}

pub fn read_display_version(game_dir: &Path) -> (String, VersionSource) {
    if let Some(text) = read_bounded(&game_dir.join("launcher-settings.json")) {
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) {
            for (key, source) in [
                ("version", VersionSource::LauncherSettingsVersion),
                ("rawVersion", VersionSource::LauncherSettingsRawVersion),
            ] {
                if let Some(Value::String(value)) = map.get(key) {
                    let value = sanitize_version(value);
                    if !value.is_empty() {
                        return (value, source);
                    }
                }
            }
        }
    }
    if let Some(branch) = read_bounded(&game_dir.join("hoi4_branch.txt")) {
        let branch = sanitize_version(branch.split('/').next().unwrap_or(""));
        if !branch.is_empty() {
            let revision = read_bounded(&game_dir.join("hoi4_rev.txt"))
                .map(|rev| sanitize_version(&rev))
                .filter(|rev| rev.chars().all(|c| c.is_ascii_hexdigit()) && rev.len() >= 4)
                .map(|rev| rev[..4].to_string());
            return (
                match revision {
                    Some(revision) => format!("{branch}.{revision}"),
                    None => branch,
                },
                VersionSource::BranchAndRevisionFiles,
            );
        }
    }
    (UNKNOWN_VERSION_TEXT.to_string(), VersionSource::Unknown)
}

const MAX_VERSION_FILE_BYTES: u64 = 256 * 1024;

fn read_bounded(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_VERSION_FILE_BYTES {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_VERSION_FILE_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let bytes = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        Some(rest) => rest.to_vec(),
        None => bytes,
    };
    String::from_utf8(bytes).ok()
}

fn sanitize_version(value: &str) -> String {
    const MAX_VERSION_CHARS: usize = 120;
    value
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_VERSION_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn hash_exe(path: &Path) -> Result<[u8; 32], String> {
    let before = fs::metadata(path).map_err(|error| error.to_string())?;
    let (size, mtime) = (before.len(), before.modified().ok());
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&chunk[..read]);
    }
    let after = fs::metadata(path).map_err(|error| error.to_string())?;
    if total != size || after.len() != size || after.modified().ok() != mtime {
        return Err("the game executable changed while it was being read".to_string());
    }
    Ok(hasher.finalize().into())
}

pub fn observe(exe_path: &Path) -> ObservedGameBuild {
    let game_dir = exe_path.parent().unwrap_or_else(|| Path::new("."));
    let (display_version, version_source) = read_display_version(game_dir);
    let (exe_sha256, exe_read_error) = match hash_exe(exe_path) {
        Ok(digest) => (Some(digest), None),
        Err(error) => (None, Some(error)),
    };
    ObservedGameBuild {
        display_version,
        version_source,
        exe_sha256,
        exe_read_error,
        exe_path: exe_path.to_path_buf(),
    }
}

pub fn classify(observed: ObservedGameBuild) -> GameBuildVerdict {
    if let Some(digest) = observed.exe_sha256 {
        if let Some(build) = supported_game_builds()
            .iter()
            .find(|build| build.exe_sha256 == digest)
        {
            return GameBuildVerdict::Supported {
                display_version: build.display_version.to_string(),
                exe_sha256: digest,
            };
        }
    }
    GameBuildVerdict::Unsupported(UnsupportedGameBuild { observed })
}

pub fn verdict_for_image_path(exe_path: &Path) -> GameBuildVerdict {
    classify(observe(exe_path))
}

#[cfg(windows)]
pub struct RetainedGameExecutable {
    pub verdict: GameBuildVerdict,
    pub resolved_path: PathBuf,
    _lease: fs::File,
}

#[cfg(windows)]
pub fn retain_and_judge_executable(exe_path: &Path) -> Result<RetainedGameExecutable, String> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    let resolved_path = fs::canonicalize(exe_path).map_err(|error| {
        format!(
            "resolve the game executable {} failed: {error}",
            exe_path.display()
        )
    })?;
    let mut lease = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&resolved_path)
        .map_err(|error| {
            format!(
                "hold the game executable {} against replacement failed: {error}",
                resolved_path.display()
            )
        })?;
    let exe_path = resolved_path.as_path();
    let game_dir = exe_path.parent().unwrap_or_else(|| Path::new("."));
    let (display_version, version_source) = read_display_version(game_dir);
    let (exe_sha256, exe_read_error) = match hash_open_file(&mut lease) {
        Ok(digest) => (Some(digest), None),
        Err(error) => (None, Some(error)),
    };
    Ok(RetainedGameExecutable {
        verdict: classify(ObservedGameBuild {
            display_version,
            version_source,
            exe_sha256,
            exe_read_error,
            exe_path: resolved_path.clone(),
        }),
        resolved_path,
        _lease: lease,
    })
}

fn hash_open_file(file: &mut fs::File) -> Result<[u8; 32], String> {
    use std::io::Seek;
    file.rewind().map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hasher.finalize().into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeImageIdentity {
    pub time_date_stamp: u32,
    pub size_of_image: u32,
    pub check_sum: u32,
    pub address_of_entry_point: u32,
}

pub const PE_HEADER_PROBE_BYTES: usize = 4096;

fn read_u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub fn pe_image_identity(bytes: &[u8]) -> Result<PeImageIdentity, String> {
    if read_u16_at(bytes, 0) != Some(0x5A4D) {
        return Err("not a PE image: no DOS signature".to_string());
    }
    let nt =
        read_u32_at(bytes, 0x3C).ok_or("PE image is truncated before its header offset")? as usize;
    if read_u32_at(bytes, nt) != Some(0x0000_4550) {
        return Err("not a PE image: no NT signature".to_string());
    }
    let time_date_stamp =
        read_u32_at(bytes, nt + 8).ok_or("PE image is truncated before its timestamp")?;
    let optional = nt + 24;
    let address_of_entry_point =
        read_u32_at(bytes, optional + 16).ok_or("PE image is truncated before its entry point")?;
    let magic = read_u16_at(bytes, optional).ok_or("PE image is truncated before its magic")?;
    if magic != 0x10B && magic != 0x20B {
        return Err("not a PE image: unknown optional-header magic".to_string());
    }
    let size_of_image =
        read_u32_at(bytes, optional + 56).ok_or("PE image is truncated before its image size")?;
    let check_sum =
        read_u32_at(bytes, optional + 64).ok_or("PE image is truncated before its checksum")?;
    Ok(PeImageIdentity {
        time_date_stamp,
        size_of_image,
        check_sum,
        address_of_entry_point,
    })
}

pub fn pe_image_identity_of_file(path: &Path) -> Result<PeImageIdentity, String> {
    let mut file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut bytes = vec![0u8; PE_HEADER_PROBE_BYTES];
    let mut filled = 0usize;
    while filled < bytes.len() {
        match file.read(&mut bytes[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    bytes.truncate(filled);
    pe_image_identity(&bytes)
}

pub fn running_image_mismatch(exe_path: &Path) -> UnsupportedGameBuild {
    let game_dir = exe_path.parent().unwrap_or_else(|| Path::new("."));
    let (display_version, version_source) = read_display_version(game_dir);
    UnsupportedGameBuild {
        observed: ObservedGameBuild {
            display_version,
            version_source,
            exe_sha256: None,
            exe_read_error: Some(
                "the running game is not the copy of the game on disk, so its build could not be \
                 proved"
                    .to_string(),
            ),
            exe_path: exe_path.to_path_buf(),
        },
    }
}

pub const REFUSAL_MESSAGE_PREFIX: &str = "This game version";

pub fn is_refusal_message(text: &str) -> bool {
    let text = text.trim();
    text.starts_with(REFUSAL_MESSAGE_PREFIX)
        && text.contains("is not supported yet")
        && text.contains("wait for a recorder update")
}

pub fn extract_refusal_message(text: &str) -> Option<String> {
    let mut start: Option<usize> = None;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with(REFUSAL_MESSAGE_PREFIX) {
            start = Some(offset + (line.len() - line.trim_start().len()));
        }
        offset += line.len();
    }
    let suffix = text[start?..].trim_end();
    is_refusal_message(suffix).then(|| suffix.to_string())
}

pub fn unreadable_refusal_message() -> String {
    let supported = supported_game_builds()
        .iter()
        .map(|build| build.display_version)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "This game version is not supported yet \u{2014} an update to the recorder will add it.\n\
         Nothing was changed in your game, and no recording was started.\n\
         What to do: wait for a recorder update that adds this version.\n\
         This recorder supports: {supported}."
    )
}

pub fn refusal_message_from_receipt(dir: &Path) -> Option<String> {
    let path = dir.join(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE);
    let mut file = fs::File::open(&path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_VERSION_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_VERSION_FILE_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    if value.get("schema")?.as_str()? != UNSUPPORTED_GAME_BUILD_SCHEMA {
        return None;
    }
    if value.get("terminal_outcome")?.as_str()? != UnsupportedGameBuild::TERMINAL_OUTCOME {
        return None;
    }
    let message = value.get("message")?.as_str()?;
    (!message.trim().is_empty()).then(|| message.to_string())
}

pub fn write_refusal_receipt(
    dir: &Path,
    refusal: &UnsupportedGameBuild,
) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let path = dir.join(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE);
    let body = serde_json::to_vec_pretty(&refusal.receipt_json())
        .map_err(|error| format!("{}: {error}", path.display()))?;
    fs::write(&path, body).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mk_game_build_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn fake_install(dir: &Path, bytes: &[u8], files: &[(&str, &str)]) -> PathBuf {
        let exe = dir.join("hoi4.exe");
        fs::write(&exe, bytes).expect("write exe");
        for (name, body) in files {
            fs::write(dir.join(name), body).expect("write version file");
        }
        exe
    }

    #[test]
    fn a_patched_executable_is_unsupported_even_when_it_claims_the_supported_version() {
        let dir = temp_dir("claims_supported");
        let exe = fake_install(
            &dir,
            b"not the pinned hoi4.exe",
            &[(
                "launcher-settings.json",
                r#"{"version":"Operation Postern v1.19.2.0.a729 (d245)"}"#,
            )],
        );
        let verdict = verdict_for_image_path(&exe);
        let GameBuildVerdict::Unsupported(refusal) = verdict else {
            panic!("a foreign executable must never be supported");
        };
        let message = refusal.message();
        assert!(
            message.contains("is not supported yet"),
            "message was: {message}"
        );
        assert!(
            message.contains(&short_fingerprint(
                &compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256
            )),
            "message must name the supported game file: {message}"
        );
        assert!(
            message.contains(&short_fingerprint(&refusal.observed.exe_sha256.unwrap())),
            "message must name the observed game file: {message}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_message_is_exactly_the_owner_sentence_and_carries_no_jargon() {
        let dir = temp_dir("exact_message");
        let exe = fake_install(
            &dir,
            b"a future hoi4.exe",
            &[(
                "launcher-settings.json",
                r#"{"version":"Operation Bombers v1.20.0.b100 (d300)","rawVersion":"1.20.0.0"}"#,
            )],
        );
        let GameBuildVerdict::Unsupported(refusal) = verdict_for_image_path(&exe) else {
            panic!("expected refusal");
        };
        let observed = short_fingerprint(&refusal.observed.exe_sha256.unwrap());
        let supported = short_fingerprint(
            &compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256,
        );
        let expected = [
            "This game version (Operation Bombers v1.20.0.b100 (d300)) is not supported yet \u{2014} an update to the recorder will add it.",
            "Nothing was changed in your game, and no recording was started.",
            "What to do: wait for a recorder update that adds this version.",
            &format!(
                "This recorder supports: Operation Postern v1.19.2.0.a729 (d245) (game file {supported})."
            ),
            &format!(
                "Your game reports: Operation Bombers v1.20.0.b100 (d300) (game file {observed})."
            ),
        ]
        .join("\n");
        assert_eq!(refusal.message(), expected);
        for jargon in [
            "0x", "RVA", "rva", "vtable", "offset", "sha256", "panic", "hoi4.exe", "unwrap",
        ] {
            assert!(
                !refusal.message().contains(jargon),
                "message must not contain {jargon:?}: {}",
                refusal.message()
            );
        }
        assert!(
            !refusal.message().contains(&dir.display().to_string()),
            "message must not contain a file path"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_install_that_names_no_version_is_still_refused_and_reads_as_unknown() {
        let dir = temp_dir("no_version_files");
        let exe = fake_install(&dir, b"nameless build", &[]);
        let GameBuildVerdict::Unsupported(refusal) = verdict_for_image_path(&exe) else {
            panic!("expected refusal");
        };
        assert_eq!(refusal.observed.version_source, VersionSource::Unknown);
        assert!(refusal
            .message()
            .starts_with("This game version (unknown) is not supported yet"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn branch_and_revision_files_name_the_build_when_launcher_settings_is_absent() {
        let dir = temp_dir("branch_fallback");
        fs::write(dir.join("hoi4_branch.txt"), "1.19.2.0/develop\n").expect("branch");
        fs::write(
            dir.join("hoi4_rev.txt"),
            "a729d47bd1c55457e6886b6eeb2fcdef4ac05057\n",
        )
        .expect("rev");
        let (version, source) = read_display_version(&dir);
        assert_eq!(version, "1.19.2.0.a729");
        assert_eq!(source, VersionSource::BranchAndRevisionFiles);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_version() {
        let dir = temp_dir("bom");
        let mut body = vec![0xEF, 0xBB, 0xBF];
        body.extend_from_slice(br#"{"version":"Operation Postern v1.19.2.0.a729 (d245)"}"#);
        fs::write(dir.join("launcher-settings.json"), body).expect("settings");
        let (version, source) = read_display_version(&dir);
        assert_eq!(version, "Operation Postern v1.19.2.0.a729 (d245)");
        assert_eq!(source, VersionSource::LauncherSettingsVersion);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hostile_version_string_cannot_control_a_surface() {
        let dir = temp_dir("hostile_version");
        let hostile = format!("{}{}", "A\r\nis not supported\u{7}", "B".repeat(500));
        let body = json!({ "version": hostile }).to_string();
        fs::write(dir.join("launcher-settings.json"), body).expect("settings");
        let (version, source) = read_display_version(&dir);
        assert_eq!(source, VersionSource::LauncherSettingsVersion);
        assert!(!version.contains('\r') && !version.contains('\n'));
        assert!(!version.chars().any(char::is_control));
        assert!(version.chars().count() <= 120, "version was {version:?}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_executable_is_unsupported() {
        let dir = temp_dir("unreadable_exe");
        let GameBuildVerdict::Unsupported(refusal) = verdict_for_image_path(&dir.join("hoi4.exe"))
        else {
            panic!("expected refusal");
        };
        assert!(refusal.observed.exe_sha256.is_none());
        assert!(refusal.observed.exe_read_error.is_some());
        assert!(refusal.message().contains("game file could not be read"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_receipt_repeats_the_message_and_names_the_terminal_outcome() {
        let dir = temp_dir("receipt");
        let exe = fake_install(&dir, b"another build", &[]);
        let GameBuildVerdict::Unsupported(refusal) = verdict_for_image_path(&exe) else {
            panic!("expected refusal");
        };
        let path = write_refusal_receipt(&dir, &refusal).expect("receipt written");
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE)
        );
        let value: Value =
            serde_json::from_slice(&fs::read(&path).expect("read receipt")).expect("parse receipt");
        assert_eq!(value["schema"], UNSUPPORTED_GAME_BUILD_SCHEMA);
        assert_eq!(
            value["terminal_outcome"],
            UnsupportedGameBuild::TERMINAL_OUTCOME
        );
        assert_eq!(value["message"], refusal.message());
        assert_eq!(value["recording_started"], false);
        assert_eq!(value["game_modified"], false);
        assert_eq!(value["exit_code"], UNSUPPORTED_GAME_BUILD_EXIT_CODE);
        assert_eq!(
            value["supported"][0]["exe_sha256"],
            full_hex(&compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256)
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_pinned_build_is_supported_and_takes_the_normal_path() {
        let pinned = compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256;
        let verdict = classify(ObservedGameBuild {
            display_version: "Operation Postern v1.19.2.0.a729 (d245)".to_string(),
            version_source: VersionSource::LauncherSettingsVersion,
            exe_sha256: Some(pinned),
            exe_read_error: None,
            exe_path: PathBuf::from("hoi4.exe"),
        });
        assert_eq!(
            verdict,
            GameBuildVerdict::Supported {
                display_version: "Operation Postern v1.19.2.0.a729 (d245)".to_string(),
                exe_sha256: pinned,
            }
        );
        let verdict = classify(ObservedGameBuild {
            display_version: UNKNOWN_VERSION_TEXT.to_string(),
            version_source: VersionSource::Unknown,
            exe_sha256: Some(pinned),
            exe_read_error: None,
            exe_path: PathBuf::from("hoi4.exe"),
        });
        assert!(matches!(verdict, GameBuildVerdict::Supported { .. }));
    }

    #[test]
    fn the_supported_roster_is_the_same_pin_the_hook_contract_uses() {
        let builds = supported_game_builds();
        assert_eq!(builds.len(), 1);
        assert_eq!(
            builds[0].exe_sha256,
            compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256
        );
    }

    #[test]
    fn the_supervisor_recovers_the_workers_sentence_from_the_receipt() {
        let dir = temp_dir("receipt_readback");
        assert_eq!(refusal_message_from_receipt(&dir), None, "no receipt yet");

        let exe = fake_install(&dir, b"a future build", &[]);
        let GameBuildVerdict::Unsupported(refusal) = verdict_for_image_path(&exe) else {
            panic!("expected refusal");
        };
        write_refusal_receipt(&dir, &refusal).expect("receipt written");
        assert_eq!(
            refusal_message_from_receipt(&dir).as_deref(),
            Some(refusal.message().as_str()),
            "the sentence must come back whole, not as its last line"
        );

        for corrupt in [
            json!({ "schema": "something-else", "terminal_outcome": UnsupportedGameBuild::TERMINAL_OUTCOME, "message": "x" }),
            json!({ "schema": UNSUPPORTED_GAME_BUILD_SCHEMA, "terminal_outcome": "other", "message": "x" }),
            json!({ "schema": UNSUPPORTED_GAME_BUILD_SCHEMA, "terminal_outcome": UnsupportedGameBuild::TERMINAL_OUTCOME }),
            json!({ "schema": UNSUPPORTED_GAME_BUILD_SCHEMA, "terminal_outcome": UnsupportedGameBuild::TERMINAL_OUTCOME, "message": "  " }),
        ] {
            fs::write(
                dir.join(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE),
                corrupt.to_string(),
            )
            .expect("write corrupt receipt");
            assert_eq!(refusal_message_from_receipt(&dir), None, "{corrupt}");
        }
        fs::write(dir.join(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE), b"not json").expect("write");
        assert_eq!(refusal_message_from_receipt(&dir), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_refusal_whose_words_were_lost_still_speaks() {
        let message = unreadable_refusal_message();
        assert!(message.starts_with(
            "This game version is not supported yet \u{2014} an update to the recorder will add it."
        ));
        assert!(message.contains("Nothing was changed in your game"));
        assert!(message.contains("wait for a recorder update"));
        assert!(message.contains("Operation Postern v1.19.2.0.a729 (d245)"));
        for jargon in ["0x", "RVA", "sha256", "panic", "exit"] {
            assert!(!message.contains(jargon), "{message}");
        }
    }

    #[test]
    fn an_oversized_receipt_is_refused_by_the_read_not_by_a_stale_stat() {
        let dir = temp_dir("oversize_receipt");
        let mut body = json!({
            "schema": UNSUPPORTED_GAME_BUILD_SCHEMA,
            "terminal_outcome": UnsupportedGameBuild::TERMINAL_OUTCOME,
            "message": "x",
            "padding": "p".repeat((MAX_VERSION_FILE_BYTES + 4096) as usize),
        })
        .to_string();
        body.push('\n');
        assert!(body.len() as u64 > MAX_VERSION_FILE_BYTES);
        fs::write(dir.join(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE), &body).expect("write");
        assert_eq!(refusal_message_from_receipt(&dir), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn a_leased_executable_is_judged_through_the_handle_that_holds_it() {
        let dir = temp_dir("lease");
        let exe = fake_install(
            &dir,
            b"a future hoi4.exe",
            &[(
                "launcher-settings.json",
                r#"{"version":"Operation Bombers v1.20.0.b100 (d300)"}"#,
            )],
        );
        let retained = retain_and_judge_executable(&exe).expect("the lease opens");
        let GameBuildVerdict::Unsupported(refusal) = &retained.verdict else {
            panic!("expected refusal");
        };
        assert!(refusal
            .message()
            .starts_with("This game version (Operation Bombers v1.20.0.b100 (d300))"));
        assert!(
            fs::OpenOptions::new().write(true).open(&exe).is_err(),
            "the lease must deny writers"
        );
        assert!(
            fs::remove_file(&exe).is_err(),
            "the lease must deny deleters"
        );
        drop(retained);
        assert!(
            fs::OpenOptions::new().write(true).open(&exe).is_ok(),
            "and must let go when it is dropped"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_text_shaped_like_a_refusal_is_shown_as_one() {
        let real = UnsupportedGameBuild {
            observed: ObservedGameBuild {
                display_version: "Operation Bombers v1.20.0.b100 (d300)".to_string(),
                version_source: VersionSource::LauncherSettingsVersion,
                exe_sha256: Some([0x5a; 32]),
                exe_read_error: None,
                exe_path: PathBuf::from("hoi4.exe"),
            },
        }
        .message();
        assert!(is_refusal_message(&real));
        assert!(is_refusal_message(unreadable_refusal_message().as_str()));
        for foreign in [
            "",
            "worker exited with exit code: 3",
            "recorder: unsupported-build receipt not written: The disk is full. (os error 112)",
            "This game version (x) is odd",
            &real[..40],
        ] {
            assert!(
                !is_refusal_message(foreign),
                "must not be shown as a refusal: {foreign:?}"
            );
            assert_eq!(
                extract_refusal_message(foreign),
                None,
                "nothing message-shaped is in {foreign:?}"
            );
        }

        let contaminated = format!(
            "recorder: unsupported-build receipt not written: The disk is full. (os error 112)\n{real}"
        );
        assert!(!is_refusal_message(&contaminated));
        assert_eq!(
            extract_refusal_message(&contaminated).as_deref(),
            Some(real.as_str()),
            "the sentence must be lifted out of the noise above it"
        );
        assert_eq!(
            extract_refusal_message(&format!("recorder: something\n{}", &real[..40])),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_lease_reports_the_resolved_path_it_was_taken_on() {
        let dir = temp_dir("lease_resolves");
        let exe = fake_install(&dir, b"a future hoi4.exe", &[]);
        let retained = retain_and_judge_executable(&exe).expect("the lease opens");
        assert_eq!(
            retained.resolved_path,
            fs::canonicalize(&exe).expect("canonical"),
            "the lease must publish the path it actually opened"
        );
        let GameBuildVerdict::Unsupported(refusal) = &retained.verdict else {
            panic!("expected refusal");
        };
        assert_eq!(refusal.observed.exe_path, retained.resolved_path);
        drop(retained);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_host_facing_surface_points_at_the_developer_runbook() {
        let refusal = UnsupportedGameBuild {
            observed: ObservedGameBuild {
                display_version: "Operation Bombers v1.20.0.b100 (d300)".to_string(),
                version_source: VersionSource::LauncherSettingsVersion,
                exe_sha256: Some([0x5a; 32]),
                exe_read_error: None,
                exe_path: PathBuf::from("hoi4.exe"),
            },
        };
        let message = refusal.message();
        assert!(message.contains("wait for a recorder update"));
        for pointer in [
            "RUNBOOK",
            "runbook",
            "http",
            ".md",
            "documents/",
            "\\documents",
        ] {
            assert!(
                !message.contains(pointer),
                "the message must not point anywhere: {message}"
            );
        }
        let mut scanned = 0usize;
        let mut sources: Vec<(String, String)> = Vec::new();
        let crate_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![crate_src];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read crate sources") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                    let text = fs::read_to_string(&path).expect("read source");
                    let production = text
                        .rsplit_once("\n#[cfg(test)]")
                        .map(|(before, _)| before.to_string())
                        .unwrap_or(text);
                    sources.push((path.display().to_string(), production));
                }
            }
        }
        sources.push((
            "New-PortableProductPackage.ps1".to_string(),
            include_str!("../../scripts/New-PortableProductPackage.ps1").to_string(),
        ));
        for (what, source) in &sources {
            scanned += 1;
            let lowered = source.to_ascii_lowercase();
            for pointer in [
                "r4_patch_day_runbook",
                "patch_day_runbook",
                "patch-day runbook",
            ] {
                assert!(
                    !lowered.contains(pointer),
                    "{what} names the developer runbook ({pointer:?}); the host's whole \
                     instruction is the refusal message"
                );
            }
        }
        assert!(
            scanned > 20,
            "the host-facing closure went stale: only {scanned} file(s) were scanned"
        );
    }

    fn fake_pe(
        time_date_stamp: u32,
        entry_point: u32,
        size_of_image: u32,
        check_sum: u32,
    ) -> Vec<u8> {
        let mut bytes = vec![0u8; 512];
        bytes[0..2].copy_from_slice(&0x5A4Du16.to_le_bytes());
        let nt = 0xC0usize;
        bytes[0x3C..0x40].copy_from_slice(&(nt as u32).to_le_bytes());
        bytes[nt..nt + 4].copy_from_slice(&0x0000_4550u32.to_le_bytes());
        bytes[nt + 8..nt + 12].copy_from_slice(&time_date_stamp.to_le_bytes());
        let optional = nt + 24;
        bytes[optional..optional + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        bytes[optional + 16..optional + 20].copy_from_slice(&entry_point.to_le_bytes());
        bytes[optional + 56..optional + 60].copy_from_slice(&size_of_image.to_le_bytes());
        bytes[optional + 64..optional + 68].copy_from_slice(&check_sum.to_le_bytes());
        bytes
    }

    #[test]
    fn the_pe_identity_reads_the_fields_that_bind_an_image_to_its_file() {
        let bytes = fake_pe(0x6612_3456, 0x1000, 0x0400_0000, 0x00AB_CDEF);
        let identity = pe_image_identity(&bytes).expect("a well-formed PE");
        assert_eq!(
            identity,
            PeImageIdentity {
                time_date_stamp: 0x6612_3456,
                size_of_image: 0x0400_0000,
                check_sum: 0x00AB_CDEF,
                address_of_entry_point: 0x1000,
            }
        );
        let other = pe_image_identity(&fake_pe(0x6612_3457, 0x1000, 0x0400_0000, 0x00AB_CDEF))
            .expect("a well-formed PE");
        assert_ne!(identity, other);
    }

    #[test]
    fn a_malformed_image_is_refused_and_never_panics() {
        for bad in [
            vec![],
            vec![0u8; 4],
            vec![0x4Du8, 0x5A],
            fake_pe(1, 1, 1, 1)[..0x40].to_vec(),
            {
                let mut bytes = fake_pe(1, 1, 1, 1);
                bytes[0] = 0;
                bytes
            },
            {
                let mut bytes = fake_pe(1, 1, 1, 1);
                bytes[0xC0] = 0;
                bytes
            },
            {
                let mut bytes = fake_pe(1, 1, 1, 1);
                bytes[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
                bytes
            },
            { fake_pe(1, 1, 1, 1)[..0xE0].to_vec() },
        ] {
            assert!(
                pe_image_identity(&bad).is_err(),
                "a malformed image must be refused, not parsed"
            );
        }
    }

    #[test]
    fn a_running_image_mismatch_speaks_the_same_sentence() {
        let dir = temp_dir("running_mismatch");
        fs::write(
            dir.join("launcher-settings.json"),
            r#"{"version":"Operation Bombers v1.20.0.b100 (d300)"}"#,
        )
        .expect("settings");
        let refusal = running_image_mismatch(&dir.join("hoi4.exe"));
        assert!(refusal.message().starts_with(
            "This game version (Operation Bombers v1.20.0.b100 (d300)) is not supported yet"
        ));
        assert!(refusal.message().contains("game file could not be read"));
        assert_eq!(
            refusal.receipt_json()["terminal_outcome"],
            UnsupportedGameBuild::TERMINAL_OUTCOME
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_refusal_exit_code_is_distinct() {
        assert_eq!(UNSUPPORTED_GAME_BUILD_EXIT_CODE, 3);
        assert_ne!(UNSUPPORTED_GAME_BUILD_EXIT_CODE, 1);
        assert_ne!(UNSUPPORTED_GAME_BUILD_EXIT_CODE, 2);
    }
}
