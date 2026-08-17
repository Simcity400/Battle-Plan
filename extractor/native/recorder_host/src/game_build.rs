//! WHICH the game BUILDS THIS RECORDER CAN RECORD — and the one
//! sentence a host reads when theirs is not one of them.
//!
//! # The defect this closes
//!
//! Every offset, RVA and vtable this recorder reads is derived against one
//! exact `hoi4.exe` (`1.19.2.0.a729`, sha256 `870ebf1f…c65ad`). A Paradox patch
//! moves them. Before this module the recorder had NO recorder-wide answer to
//! that: the pinned executable digest was first compared
//!
//! * inside the collector's own bootstrap hook installer, and
//! * inside the host's `CaptureReadyV2` validation,
//!
//! both of which run AFTER `VirtualAllocEx` / `WriteProcessMemory` /
//! `CreateRemoteThread(LoadLibraryW)` have already written the collector DLL
//! into the player's game. On patch day a host would therefore have had a
//! foreign DLL mapped into the game, five hook sites patched, and then an internal
//! sentence about "the exact a729 executable" and an attempt folder with no
//! recording in it.
//!
//! Owner ruling 2026-08-18: an unknown game build is ONE clear outcome — the
//! version the game reports, the versions this recorder supports, the words
//! "this game version (x) is not supported yet — an update to the recorder will
//! add it", a clean stop, and NOTHING patched into the game. No "try anyway".
//!
//! # Refuse before mutation
//!
//! Everything here works from files on disk, reached from the live process's
//! own image path. No process handle with write rights, no remote allocation,
//! no hook. That is what lets the check sit ahead of the first mutation in both
//! entry paths:
//!
//! * the resident product — `auto_attach::coordinator`, in `WaitingForIdentity`,
//!   before the candidate is ever staged or a worker spawned;
//! * the recorder worker / CLI — `main.rs`, before the target process is opened
//!   for injection.
//!
//! # How the game names its own version
//!
//! `hoi4.exe`'s Windows version resource says `1.0` — measured, on the pinned
//! build. It cannot name a the game version and is never read here. The engine's
//! own banner (`"Operation Postern v1.19.2.0.a729"`) lives in game memory at a
//! pinned RVA, which is exactly the thing a patch moves, so it is useless for
//! naming an UNKNOWN build and is not read here either.
//!
//! What does work is what the launcher itself shows the player, written beside
//! `hoi4.exe` by the install:
//!
//! * `launcher-settings.json` → `"version": "Operation Postern v1.19.2.0.a729 (d245)"`,
//!   `"rawVersion": "1.19.2.0"`;
//! * `hoi4_branch.txt` → `1.19.2.0/develop`;
//! * `hoi4_rev.txt` → `a729d47bd1c554…` (the `a729` in the build name).
//!
//! Those are ordinary installed content discovered from the running process's path —
//! no hardcoded install location, no engine constant, and a mod cannot move
//! them (they sit in the game directory, not the VFS). Every one of them is
//! best-effort: when none can be read the build is still refused, and the
//! message says the version is unknown rather than inventing one.
//!
//! # What decides support
//!
//! The sha256 of `hoi4.exe`, never the version text. Text is for the human; the
//! digest is the pin the offsets were derived against. A build whose text reads
//! `1.19.2.0.a729` but whose bytes differ is unsupported, and the message shows
//! both file fingerprints so that case is legible instead of self-contradictory.

use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// A the game build this recorder's offsets were derived against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupportedGameBuild {
    /// The version as the game's own launcher names it, for display only.
    pub display_version: &'static str,
    /// `hoi4.exe`'s sha256. THIS is what decides support.
    pub exe_sha256: [u8; 32],
}

/// Every build this recorder can record. Exactly one today; support for a new
/// one arrives as a recorder update, which is what the refusal tells the host
/// to wait for.
///
/// The digest is not spelled again here: it is the same constant the hook
/// contract, the CaptureReady validator and every pinned production hook
/// already refuse against, so a second spelling could drift away from the
/// thing it claims to name.
pub fn supported_game_builds() -> &'static [SupportedGameBuild] {
    &[SupportedGameBuild {
        display_version: "Operation Postern v1.19.2.0.a729 (d245)",
        exe_sha256: compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256,
    }]
}

/// Where a display version came from, kept for the receipt so a support report
/// says which game file was believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionSource {
    /// `launcher-settings.json`'s `version` — what the launcher shows.
    LauncherSettingsVersion,
    /// `launcher-settings.json`'s `rawVersion`.
    LauncherSettingsRawVersion,
    /// `hoi4_branch.txt` (+ `hoi4_rev.txt`'s leading revision characters).
    BranchAndRevisionFiles,
    /// Nothing readable named a version.
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

/// The text used wherever no game file named a version. It is a plain word on
/// purpose: the host is told the recorder could not read the version, not given
/// a blank.
pub const UNKNOWN_VERSION_TEXT: &str = "unknown";

/// What the recorder observed about the game it found, entirely from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedGameBuild {
    /// The version the game names for itself, or [`UNKNOWN_VERSION_TEXT`].
    pub display_version: String,
    pub version_source: VersionSource,
    /// `hoi4.exe`'s sha256, or `None` when the file could not be read — which
    /// is itself a refusal, since an unreadable game cannot be a proven
    /// supported one.
    pub exe_sha256: Option<[u8; 32]>,
    /// Why the digest is absent, when it is.
    pub exe_read_error: Option<String>,
    pub exe_path: PathBuf,
}

/// The verdict. Two values, and the recorder may only proceed on the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameBuildVerdict {
    Supported {
        display_version: String,
        exe_sha256: [u8; 32],
    },
    Unsupported(UnsupportedGameBuild),
}

/// A refused build, and the single source of the sentence every surface shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedGameBuild {
    pub observed: ObservedGameBuild,
}

/// Twelve hex characters of a sha256 — enough for a host to quote in a support
/// report and short enough to read aloud. Never used to decide anything.
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
    /// THE message. Every surface — the window, the console, the receipt and
    /// the log — shows this exact text and nothing else about the refusal.
    ///
    /// Lines are separated with `\n`; the window converts to `\r\n` when it
    /// renders. No stack trace, no offset, no RVA, no module name, no file
    /// path: a host cannot act on any of those, and the developer's recovery
    /// procedure is a developer document that is deliberately not linked from
    /// anything a host sees.
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

    /// The stable machine name of this terminal outcome, shared by the journal
    /// reason code and the receipt.
    pub const TERMINAL_OUTCOME: &'static str = "unsupported_game_build";

    /// The receipt body. Carries the message verbatim so the file a host sends
    /// in says exactly what the window said, plus the full digests a developer
    /// needs to start the patch-day procedure.
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

/// The receipt's leaf name and schema. One file, written wherever the refusing
/// path has a directory to write into.
pub const UNSUPPORTED_GAME_BUILD_RECEIPT_FILE: &str = "GAME_BUILD_UNSUPPORTED_V1.json";
pub const UNSUPPORTED_GAME_BUILD_SCHEMA: &str = "hoi4-game-build-unsupported-v1";

/// The process exit code a recorder run ends with when it refused an unknown
/// game build. Distinct from the generic failure code 1 so a launcher or script
/// can tell "this build is not supported yet" from "something went wrong",
/// and from 2 (the internal prefold worker's failure code).
pub const UNSUPPORTED_GAME_BUILD_EXIT_CODE: i32 = 3;

/// Set once by the refusing path so `main` can exit with
/// [`UNSUPPORTED_GAME_BUILD_EXIT_CODE`] while the refusal still travels the
/// ordinary `Result` route — the error text reaches stderr and the window
/// exactly as every other fatal does, and every destructor still runs.
static REFUSAL_EXIT_CODE: OnceLock<i32> = OnceLock::new();

pub fn arm_refusal_exit_code() {
    let _ = REFUSAL_EXIT_CODE.set(UNSUPPORTED_GAME_BUILD_EXIT_CODE);
}

pub fn refusal_exit_code() -> Option<i32> {
    REFUSAL_EXIT_CODE.get().copied()
}

/// The refusal has already been put in front of the host by a window they saw
/// and dismissed.
///
/// Without this the resident recorder shows the sentence in its own window, the
/// host reads it and selects Close, and the same sentence immediately reappears
/// as a modal dialog — because a no-argument launch always routes a fatal to a
/// dialog. One outcome means one showing.
static REFUSAL_PRESENTED: AtomicBool = AtomicBool::new(false);

pub fn mark_refusal_presented() {
    REFUSAL_PRESENTED.store(true, Ordering::Release);
}

pub fn refusal_already_presented() -> bool {
    REFUSAL_PRESENTED.load(Ordering::Acquire)
}

/// Read the version the game names for itself out of the game directory.
///
/// Best-effort by construction and infallible by return type: an install with
/// none of these files still gets refused, just named `unknown`. Order is
/// most-human-readable first.
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
    // `hoi4_branch.txt` is `1.19.2.0/develop`; the build suffix `a729` is the
    // first four characters of `hoi4_rev.txt`. Together they spell the same
    // name the launcher shows, which is why they are the fallback rather than a
    // separate vocabulary.
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

/// Bytes of a game version file this reader will look at. These files are tens
/// to a few thousand bytes; the bound stops a corrupt or substituted file being
/// read into memory before it is judged.
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
    // A UTF-8 BOM is not part of the document. The build measured here has
    // none, but a future install writing one would otherwise make
    // `launcher-settings.json` unparseable and silently demote the version to
    // the branch/revision fallback.
    let bytes = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        Some(rest) => rest.to_vec(),
        None => bytes,
    };
    String::from_utf8(bytes).ok()
}

/// One line of display text: trimmed, first line only, bounded, and stripped of
/// anything that would corrupt a window, a console or a JSON receipt. A game
/// file is data, not a message, so nothing it contains may control a surface.
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

/// Hash `hoi4.exe`, refusing a file that changes underneath the read.
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

/// Observe the build of the game whose executable is at `exe_path`.
///
/// Never fails: an unreadable executable is an observation with no digest,
/// which [`classify`] refuses exactly like a digest that does not match.
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

/// Supported iff the observed executable digest is one this recorder's offsets
/// were derived against. Fail closed on anything else.
pub fn classify(observed: ObservedGameBuild) -> GameBuildVerdict {
    if let Some(digest) = observed.exe_sha256 {
        if let Some(build) = supported_game_builds()
            .iter()
            .find(|build| build.exe_sha256 == digest)
        {
            return GameBuildVerdict::Supported {
                // The supported build's own name, not the install's text: on a
                // proven-identical executable they describe the same thing and
                // the recorder's own name is the one its evidence is filed
                // under.
                display_version: build.display_version.to_string(),
                exe_sha256: digest,
            };
        }
    }
    GameBuildVerdict::Unsupported(UnsupportedGameBuild { observed })
}

/// The whole check, from a live process's image path to a verdict.
pub fn verdict_for_image_path(exe_path: &Path) -> GameBuildVerdict {
    classify(observe(exe_path))
}

/// A game executable held open in a way that stops it being rewritten or
/// replaced for as long as the lease is alive, together with the verdict on the
/// bytes that were actually read through it.
///
/// # Why a lease exists at all
///
/// The early-launch route judges a PATH and then asks Windows to create a
/// process from that path. Between those two steps the directory entry can be
/// repointed — Steam finishing an update is the ordinary way — and the recorder
/// would then create, suspend, and inject into a build it never judged. Judging
/// the file again afterwards is not enough: by then it has already been created
/// suspended, which is acting on it.
///
/// Holding the file open with only `FILE_SHARE_READ` denies writers and deleters
/// for the whole window. The image loader opens the executable for read, which
/// this permits, so `CreateProcessW` still works — this is the same shape the
/// collector DLL's own `RetainedDllAuthority` uses on its payload.
#[cfg(windows)]
pub struct RetainedGameExecutable {
    pub verdict: GameBuildVerdict,
    /// The path the lease was actually taken on: fully resolved ONCE, before
    /// the file was opened.
    ///
    /// Callers must launch from THIS, not from the path they asked about. A
    /// lease locks the file it opened; it does not lock a symlink or junction
    /// that pointed at it, and re-resolving the original name a second time —
    /// which `CreateProcessW`'s own canonicalisation would do — can arrive
    /// somewhere else entirely.
    pub resolved_path: PathBuf,
    _lease: fs::File,
}

/// Open `exe_path` denying write and delete, hash what is behind that handle,
/// and keep the handle alive inside the returned lease.
#[cfg(windows)]
pub fn retain_and_judge_executable(exe_path: &Path) -> Result<RetainedGameExecutable, String> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    // Resolved ONCE. Everything after this — the open, the hash, and the
    // caller's launch — uses the resolved path, so a symlink or junction in the
    // name the caller gave cannot be repointed between the judgement and the
    // launch. Locking the file does not lock the name that found it.
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
    // Hashed THROUGH the lease. A second `open` by path would be a second
    // lookup, and a second lookup is the race this exists to close.
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

/// The identity a PE image carries in its own headers.
///
/// # Why this exists
///
/// Everything above judges the FILE at the running process's image path. Windows
/// lets that directory entry be replaced while the older image keeps running, so
/// a digest of the file is not, by itself, a statement about the process. In one
/// direction that is safe — a patched file on disk refuses a session whose
/// running image was fine, costing a recording rather than injecting into an
/// unknown build. In the other it is not: a host who patched, launched, and then
/// reverted the install would have the supported file on disk and an unsupported
/// image running, and a disk-only gate would admit it.
///
/// These four fields are mapped verbatim at the module base, so the same parser
/// reads them from the file we hashed and from the image the process is actually
/// executing. Equal fields mean the running image came from a file with these
/// headers; combined with "that file's sha256 is the pinned build", that is the
/// statement the injection gate needs. They are an IDENTITY CROSS-CHECK, never a
/// support decision on their own — support is still the sha256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeImageIdentity {
    pub time_date_stamp: u32,
    pub size_of_image: u32,
    pub check_sum: u32,
    pub address_of_entry_point: u32,
}

/// How many leading bytes of an image [`pe_image_identity`] needs. The DOS stub
/// plus the NT headers live well inside the first page on every PE Windows will
/// load.
pub const PE_HEADER_PROBE_BYTES: usize = 4096;

fn read_u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Parse the four identity fields out of a PE image's leading bytes.
///
/// Bounds-checked throughout: the input may be a truncated file, a foreign
/// binary, or bytes read out of a process that is not what we think it is, and
/// none of those may panic.
pub fn pe_image_identity(bytes: &[u8]) -> Result<PeImageIdentity, String> {
    if read_u16_at(bytes, 0) != Some(0x5A4D) {
        return Err("not a PE image: no DOS signature".to_string());
    }
    let nt = read_u32_at(bytes, 0x3C).ok_or("PE image is truncated before its header offset")?
        as usize;
    if read_u32_at(bytes, nt) != Some(0x0000_4550) {
        return Err("not a PE image: no NT signature".to_string());
    }
    // COFF header is 20 bytes after the 4-byte signature; the optional header
    // follows. The four fields below sit at fixed offsets that are the same for
    // PE32 and PE32+ (all four precede the first size-dependent field).
    let time_date_stamp =
        read_u32_at(bytes, nt + 8).ok_or("PE image is truncated before its timestamp")?;
    let optional = nt + 24;
    let address_of_entry_point =
        read_u32_at(bytes, optional + 16).ok_or("PE image is truncated before its entry point")?;
    let magic = read_u16_at(bytes, optional).ok_or("PE image is truncated before its magic")?;
    // `SizeOfImage` and `CheckSum` sit at 56 and 64 in both PE32 and PE32+
    // optional headers; the layouts only diverge after `SizeOfHeaders`.
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

/// [`pe_image_identity`] over the leading bytes of a file.
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

/// The sentence for a running image that does not match the file that was
/// judged. It is the ORDINARY unsupported message: from the host's side this is
/// the same situation — the game running right now is not one this recorder
/// knows — and giving it a second vocabulary would give the same event two
/// different explanations.
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

/// The first words of every refusal sentence. Its own shape check: text that
/// does not open this way is not one of ours, whatever produced it.
pub const REFUSAL_MESSAGE_PREFIX: &str = "This game version";

/// Is this text a refusal message this recorder wrote?
///
/// The supervisor recovers a worker's sentence from the worker's log when the
/// receipt is gone, and a log is a place other things also write — the very
/// line explaining that the receipt could not be written is printed
/// immediately before it. Anything that does not have the shape of the message
/// is discarded rather than shown to a host as if it were one.
pub fn is_refusal_message(text: &str) -> bool {
    let text = text.trim();
    text.starts_with(REFUSAL_MESSAGE_PREFIX)
        && text.contains("is not supported yet")
        && text.contains("wait for a recorder update")
}

/// Pull a refusal message out of text that may have other lines above it.
///
/// The worker prints "recorder: unsupported-build receipt not written: …"
/// immediately before the message, with no blank line, so the trailing block of
/// its log is the diagnostic AND the message. REJECTING that block outright cost
/// the host the version — the sentence was right there, one line down. This
/// takes the suffix beginning at the LAST line that opens a message, and returns
/// it only if that suffix is one.
///
/// `None` means nothing message-shaped is in there, and the caller should say
/// less rather than say something else.
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

/// The sentence for a refusal whose own words could not be recovered.
///
/// Reached only when a worker exited on [`UNSUPPORTED_GAME_BUILD_EXIT_CODE`] and
/// neither its receipt nor any message-shaped text in its log survived. It says less — it cannot name the
/// version, because nothing that knew it is readable — but it is true, it is the
/// same outcome, and it stops the recorder. An exit code shown to a host is not.
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

/// Read back the host-facing sentence a refusal receipt in `dir` carries.
///
/// The supervisor uses this to recover what its WORKER told the host, verbatim,
/// rather than re-deriving it or scraping the worker's last stderr line. `None`
/// for an absent, oversized, unparseable or foreign-schema file: a receipt that
/// cannot be read is not evidence that a build was refused.
pub fn refusal_message_from_receipt(dir: &Path) -> Option<String> {
    let path = dir.join(UNSUPPORTED_GAME_BUILD_RECEIPT_FILE);
    // Bounded from the OPEN HANDLE, not from a separate `metadata` call: a
    // stat-then-read pair lets the file grow between the two, and the bound
    // this documents would then be a bound on a file nobody read.
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

/// Publish the refusal receipt into `dir`, best-effort.
///
/// A receipt that cannot be written never changes the outcome — the refusal
/// already happened and the message already reached the window and the log.
/// The path is returned so a caller can name it; the error is returned so a
/// caller can log it, and neither is a reason to record anything.
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

    /// A fake install: an executable with `bytes`, plus whichever version files
    /// the case wants beside it.
    fn fake_install(dir: &Path, bytes: &[u8], files: &[(&str, &str)]) -> PathBuf {
        let exe = dir.join("hoi4.exe");
        fs::write(&exe, bytes).expect("write exe");
        for (name, body) in files {
            fs::write(dir.join(name), body).expect("write version file");
        }
        exe
    }

    /// kills: deleting the digest comparison in `classify`, or replacing it
    /// with a version-string comparison. An executable whose bytes are not the
    /// pinned build must never be Supported, no matter what the installed content say
    /// about themselves.
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
        // Both fingerprints appear, so "supported: X / your game: X" cannot read
        // as a contradiction.
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

    /// kills: dropping any clause of the owner's sentence, reordering it, or
    /// letting a surface print a path/offset/module name. The message is
    /// asserted whole, in one piece, from the one function that produces it.
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

    /// kills: making the version reader fallible, or letting an install with no
    /// readable version files pass the gate. An install that names nothing is
    /// still refused, and the message says `unknown` rather than a blank.
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

    /// kills: dropping the `hoi4_branch.txt` / `hoi4_rev.txt` fallback, or
    /// spelling the revision suffix differently from the launcher's own build
    /// name.
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

    /// kills: dropping the BOM strip. A future install writing `launcher-
    /// settings.json` with a UTF-8 BOM would otherwise fail to parse and
    /// silently demote the host's version text to the branch/revision fallback.
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

    /// kills: reading `launcher-settings.json` as raw text into a surface. A
    /// game file is data: control characters, extra lines and unbounded length
    /// must never reach a window, a console or a JSON receipt.
    #[test]
    fn a_hostile_version_string_cannot_control_a_surface() {
        let dir = temp_dir("hostile_version");
        let hostile = format!(
            "{}{}",
            "A\r\nis not supported\u{7}",
            "B".repeat(500)
        );
        let body = json!({ "version": hostile }).to_string();
        fs::write(dir.join("launcher-settings.json"), body).expect("settings");
        let (version, source) = read_display_version(&dir);
        assert_eq!(source, VersionSource::LauncherSettingsVersion);
        assert!(!version.contains('\r') && !version.contains('\n'));
        assert!(!version.chars().any(char::is_control));
        assert!(version.chars().count() <= 120, "version was {version:?}");
        fs::remove_dir_all(&dir).ok();
    }

    /// kills: letting an unreadable executable through. No digest is not
    /// "assume it is fine"; it is an unknown build.
    #[test]
    fn an_unreadable_executable_is_unsupported() {
        let dir = temp_dir("unreadable_exe");
        let GameBuildVerdict::Unsupported(refusal) =
            verdict_for_image_path(&dir.join("hoi4.exe"))
        else {
            panic!("expected refusal");
        };
        assert!(refusal.observed.exe_sha256.is_none());
        assert!(refusal.observed.exe_read_error.is_some());
        assert!(refusal
            .message()
            .contains("game file could not be read"));
        fs::remove_dir_all(&dir).ok();
    }

    /// kills: writing a receipt that disagrees with what the host was shown, or
    /// dropping the named terminal outcome / exit code from it.
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

    /// kills: making the gate refuse the build it exists to admit. The pinned
    /// executable — the only one every offset in this recorder was derived
    /// against — must classify Supported, carrying the recorder's own name for
    /// it, so the ordinary path is unchanged for the host whose game matches.
    /// (Asserted on the digest rather than on a real 2 GB `hoi4.exe`, because
    /// the digest is the entire input `classify` reads.)
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
        // ...and the recorder's own name wins even when the install's files say
        // nothing, so an unreadable `launcher-settings.json` on a proven-correct
        // executable does not degrade what the evidence is filed under.
        let verdict = classify(ObservedGameBuild {
            display_version: UNKNOWN_VERSION_TEXT.to_string(),
            version_source: VersionSource::Unknown,
            exe_sha256: Some(pinned),
            exe_read_error: None,
            exe_path: PathBuf::from("hoi4.exe"),
        });
        assert!(matches!(verdict, GameBuildVerdict::Supported { .. }));
    }

    /// kills: pointing the supported roster at some other digest than the one
    /// the hook contract and the CaptureReady validator refuse against. Two
    /// spellings of "the build we support" is the drift this asserts away.
    #[test]
    fn the_supported_roster_is_the_same_pin_the_hook_contract_uses() {
        let builds = supported_game_builds();
        assert_eq!(builds.len(), 1);
        assert_eq!(
            builds[0].exe_sha256,
            compact_contract::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256
        );
    }

    /// kills: recovering the worker's refusal from its stderr tail instead of
    /// its receipt (which keeps one line of a five-line sentence), or admitting
    /// a receipt that names a different schema/outcome — the leftovers of an
    /// earlier attempt in a reused directory.
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

        // A foreign or superseded document is not evidence that a build was
        // refused.
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

    /// kills: deleting the last-resort sentence, or letting a refusal whose own
    /// words were lost reach a host as an exit code. It says less — nothing that
    /// knew the version is readable — but it is the same outcome, in the same
    /// words, and it still tells the host what to do.
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

    /// kills: bounding the receipt read with a separate `metadata` call. A
    /// stat-then-read pair lets the file grow between the two, and the bound
    /// this documents would be a bound on a file nobody read.
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

    /// kills: judging the early-launch executable by a second lookup instead of
    /// through the handle that is held across `CreateProcessW`. The lease is
    /// what stops the directory entry being repointed between the judgement and
    /// the launch.
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
        // ...and while it is held the file cannot be replaced or removed, which
        // is the whole point.
        assert!(
            fs::OpenOptions::new().write(true).open(&exe).is_err(),
            "the lease must deny writers"
        );
        assert!(fs::remove_file(&exe).is_err(), "the lease must deny deleters");
        drop(retained);
        assert!(
            fs::OpenOptions::new().write(true).open(&exe).is_ok(),
            "and must let go when it is dropped"
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// kills: dropping the shape check on the log fallback, or applying it to
    /// the trailing block WHOLE instead of extracting the message out of it. The worker prints
    /// "recorder: unsupported-build receipt not written: …" immediately before
    /// the message and with no blank line between them, so the trailing block of
    /// its log is the diagnostic AND the message. Handing that to a host puts
    /// disk-error jargon at the top of the one sentence that is supposed to be
    /// plain.
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

        // THE CASE THAT MATTERS, and the one a whole-block check got wrong: the
        // worker's own receipt-write diagnostic sits directly above the message
        // with no blank line between them. The version is right there — keeping
        // it is the difference between telling the host WHICH build they have
        // and telling them only that it is not this one.
        let contaminated = format!(
            "recorder: unsupported-build receipt not written: The disk is full. (os error 112)\n{real}"
        );
        assert!(!is_refusal_message(&contaminated));
        assert_eq!(
            extract_refusal_message(&contaminated).as_deref(),
            Some(real.as_str()),
            "the sentence must be lifted out of the noise above it"
        );
        // ...and a TRUNCATED message under a diagnostic is still not one.
        assert_eq!(
            extract_refusal_message(&format!("recorder: something\n{}", &real[..40])),
            None
        );
    }

    /// kills: launching from the name the caller gave instead of the path the
    /// lease was taken on. A lease locks a FILE; it does not lock a symlink or
    /// junction that pointed at it, and resolving the original name a second
    /// time — which the launcher's own canonicalisation does — can arrive
    /// somewhere else entirely.
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
        // ...and the observation is filed under that same resolved path, so a
        // receipt names the file that was judged.
        let GameBuildVerdict::Unsupported(refusal) = &retained.verdict else {
            panic!("expected refusal");
        };
        assert_eq!(refusal.observed.exe_path, retained.resolved_path);
        drop(retained);
        fs::remove_dir_all(&dir).ok();
    }

    /// kills: pointing a host at the developer's patch-day runbook — from the
    /// message, from the window, or from the README that ships in the ZIP. The
    /// host's whole instruction is "wait for a recorder update"; the recovery
    /// procedure is a developer's document and stays one (owner ruling
    /// 2026-08-18).
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
        // The host is never sent to a document, a page, or a support address.
        for pointer in [
            "RUNBOOK", "runbook", "http", ".md", "documents/", "\\documents",
        ] {
            assert!(
                !message.contains(pointer),
                "the message must not point anywhere: {message}"
            );
        }
        // ...nor may anything that can put words in front of a host. That
        // closure is: every source file of this crate (the window, the console
        // lines, the journal, the receipts) and the script that generates the
        // README shipped inside the product ZIP. Matched case-insensitively and
        // on the runbook's stem as well as its file name, so a rename or a
        // differently-cased reference does not slip through.
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
                    // The PRODUCTION half only. A test may name the runbook —
                    // this one does, in the very list below — and a test is not
                    // a surface a host can read.
                    //
                    // Split at the LAST `#[cfg(test)]`, not the first. Several
                    // files in this crate open with a test-only `use` (the
                    // coordinator's is line 13), and splitting at the first
                    // attribute scanned twelve lines of the coordinator and
                    // called the whole file clean.
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
            for pointer in ["r4_patch_day_runbook", "patch_day_runbook", "patch-day runbook"] {
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

    /// A minimal PE64 header block: DOS signature, `e_lfanew`, NT signature,
    /// COFF timestamp, and the four optional-header fields the identity reads.
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

    /// kills: deleting the running-image cross-check, or reading a field the
    /// mapped image does not carry at the same place. The injection gate proves
    /// two things — that the FILE is the pinned build, and that the IMAGE the
    /// process is running came from that file — because Windows lets the file be
    /// replaced under a running process.
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
        // A rebuilt game differs in the timestamp alone; that is enough.
        let other = pe_image_identity(&fake_pe(0x6612_3457, 0x1000, 0x0400_0000, 0x00AB_CDEF))
            .expect("a well-formed PE");
        assert_ne!(identity, other);
    }

    /// kills: parsing a hostile or truncated image without bounds. These bytes
    /// can come out of a process that is not what the recorder thinks it is, so
    /// none of them may panic.
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
                // `e_lfanew` past the end of the buffer.
                let mut bytes = fake_pe(1, 1, 1, 1);
                bytes[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
                bytes
            },
            {
                // A real PE header truncated mid optional-header.
                fake_pe(1, 1, 1, 1)[..0xE0].to_vec()
            },
        ] {
            assert!(
                pe_image_identity(&bad).is_err(),
                "a malformed image must be refused, not parsed"
            );
        }
    }

    /// kills: giving a running-image mismatch its own vocabulary. From the
    /// host's side it is the same event — the game running right now is not one
    /// this recorder knows — and two explanations for one event is the defect.
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

    /// kills: reusing the generic failure exit code, which would make "your
    /// game is newer than this recorder" indistinguishable from a crash.
    #[test]
    fn the_refusal_exit_code_is_distinct() {
        assert_eq!(UNSUPPORTED_GAME_BUILD_EXIT_CODE, 3);
        assert_ne!(UNSUPPORTED_GAME_BUILD_EXIT_CODE, 1);
        assert_ne!(UNSUPPORTED_GAME_BUILD_EXIT_CODE, 2);
    }
}
