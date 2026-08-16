//! Authenticated internal process mode for compact-stream folding.
//!
//! The product deliberately executes the fold in a child process, but the
//! child is the exact `replay_app` image already admitted by the launcher. The
//! parent passes its retained-handle SHA-256 and the child refuses to fold
//! unless its own image matches that value.

// The production worker is the recorder executable already present in the
// two-executable offline package. Keep only the process-contract constants in
// the replay crate; the extractor-owned implementation below is retained as
// historical source until the sibling-worker move is committed atomically.
pub(crate) const INTERNAL_VERB: &str = "--internal-prefold-worker-v1";
pub(crate) const RECEIPT_SCHEMA: &str = "hoi4_prefold_worker_receipt_v2";
pub(crate) const DECODER_ID: &str = "extractor.replay_decoder.production_world_fold.worker.v1";
pub(crate) const WORKER_EXE_NAME: &str = "Battle Plan Recorder.exe";

#[cfg(any())]
mod retired_self_worker {

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

const INTERNAL_VERB: &str = "--internal-prefold-worker-v1";
const RECEIPT_SCHEMA: &str = "hoi4_prefold_worker_receipt_v2";
const DECODER_ID: &str = "extractor.replay_decoder.production_world_fold.worker.v1";

#[derive(Default)]
struct Args {
    game_id: Option<String>,
    source: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
    source_len: Option<u64>,
    source_sha256: Option<String>,
    identity_mode: Option<snapshot::PrefoldStreamIdentityMode>,
    executable_sha256: Option<String>,
}

pub(crate) fn is_internal_invocation(args: &[OsString]) -> bool {
    args.first().and_then(|value| value.to_str()) == Some(INTERNAL_VERB)
}

pub(crate) fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args = parse_args(args)?;
    let expected_executable_sha256 = args
        .executable_sha256
        .ok_or("missing --worker-executable-sha256")?;
    let executable = crate::recording_open::AuthenticatedCurrentExecutable::open()
        .map_err(|error| error.to_string())?;
    if executable.sha256() != expected_executable_sha256 {
        return Err("internal prefold worker executable identity mismatch".into());
    }

    let game_id = args.game_id.ok_or("missing --game-id")?;
    let source = args.source.ok_or("missing --source")?;
    let snapshot_dir = args.snapshot_dir.ok_or("missing --snapshot-dir")?;
    let snapshot_guard = SnapshotDirectoryGuard::acquire(&snapshot_dir)?;
    let source_len = args.source_len.ok_or("missing --source-len")?;
    let source_sha256 = args.source_sha256.ok_or("missing --source-sha256")?;
    let identity_mode = args.identity_mode.ok_or("missing --identity-mode")?;
    let identity_mode_label = match identity_mode {
        snapshot::PrefoldStreamIdentityMode::Legacy => "legacy",
        snapshot::PrefoldStreamIdentityMode::IdentityV1 => "identity-v1",
    };
    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new_with_identity_mode(
        game_id,
        source,
        snapshot_guard.path(),
        source_len,
        source_sha256,
        identity_mode,
    )
    .map_err(|error| error.to_string())?;
    // Parent stdin is a duplicated handle to the exact retained source object
    // it authenticated. Never reopen `source` by pathname in the worker.
    let source_file = duplicate_inherited_source_file()?;
    let spool = replay_decoder::fold_raw_h4cr_open_file_spooled(request, source_file)
        .map_err(|error| error.to_string())?;
    executable
        .verify_unchanged()
        .map_err(|error| error.to_string())?;
    let entries = spool
        .entries()
        .iter()
        .map(|entry| {
            serde_json::json!({
                "hour_index": entry.hour_index(),
                "date": entry.date(),
                "file_name": entry.file_name(),
                "byte_len": entry.byte_len(),
                "sha256": entry.sha256(),
            })
        })
        .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "schema": RECEIPT_SCHEMA,
        "decoder_id": DECODER_ID,
        "worker_executable_sha256": executable.sha256(),
        "identity_mode": identity_mode_label,
        "game_id": spool.game_id(),
        "schema_version": spool.schema_version(),
        "entries": entries,
    });
    serde_json::to_writer(std::io::stdout().lock(), &receipt)
        .map_err(|error| format!("serialize internal worker receipt: {error}"))?;
    Ok(())
}

#[cfg(windows)]
fn duplicate_inherited_source_file() -> Result<File, String> {
    use std::ffi::c_void;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};

    const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn DuplicateHandle(
            source_process: *mut c_void,
            source_handle: *mut c_void,
            target_process: *mut c_void,
            target_handle: *mut *mut c_void,
            desired_access: u32,
            inherit_handle: i32,
            options: u32,
        ) -> i32;
    }

    let stdin = std::io::stdin();
    let source_handle = stdin.as_raw_handle();
    if source_handle.is_null() || source_handle == (-1isize as *mut c_void) {
        return Err("internal prefold worker did not inherit a valid source handle".into());
    }
    let process = unsafe { GetCurrentProcess() };
    let mut duplicate = std::ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            source_handle.cast(),
            process,
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(format!(
            "duplicate inherited compact-source handle: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_handle(duplicate) })
}

#[cfg(unix)]
fn duplicate_inherited_source_file() -> Result<File, String> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};

    extern "C" {
        fn dup(old_fd: i32) -> i32;
    }

    let stdin = std::io::stdin();
    let duplicate = unsafe { dup(stdin.as_raw_fd()) };
    if duplicate < 0 {
        return Err(format!(
            "duplicate inherited compact-source fd: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_fd(duplicate) })
}

#[cfg(not(any(windows, unix)))]
fn duplicate_inherited_source_file() -> Result<File, String> {
    Err("authenticated prefold source-handle inheritance is unsupported on this platform".into())
}

/// Retains every snapshot-directory component for the complete fold. On
/// Windows each handle denies delete sharing, so an accepted component cannot
/// be renamed or replaced by a junction before a later WorldFold child open.
struct SnapshotDirectoryGuard {
    path: PathBuf,
    #[cfg(windows)]
    _handles: Vec<std::fs::File>,
}

impl SnapshotDirectoryGuard {
    fn acquire(path: &Path) -> Result<Self, String> {
        let absolute = absolute_clean_path(path)?;
        #[cfg(windows)]
        let handles = windows_snapshot_directory_handles(&absolute)?;
        #[cfg(not(windows))]
        validate_snapshot_directory_components(&absolute)?;
        Ok(Self {
            path: absolute,
            #[cfg(windows)]
            _handles: handles,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn absolute_clean_path(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;

    if path.as_os_str().is_empty() {
        return Err("snapshot spool path is empty".into());
    }
    if path
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(format!(
            "snapshot spool path must not contain . or .. components: {}",
            path.display()
        ));
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("resolve snapshot spool path: {error}"))
    }
}

#[cfg(not(windows))]
fn validate_snapshot_directory_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if current.parent().is_none() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| format!("inspect snapshot spool {}: {error}", current.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "snapshot spool component is not a regular directory: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_snapshot_directory_handles(path: &Path) -> Result<Vec<std::fs::File>, String> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::path::Component;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let mut current = PathBuf::new();
    let mut handles = Vec::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let handle = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&current)
            .map_err(|error| {
                format!(
                    "lock snapshot spool component {} against replacement: {error}",
                    current.display()
                )
            })?;
        let metadata = handle.metadata().map_err(|error| {
            format!(
                "inspect locked snapshot spool component {}: {error}",
                current.display()
            )
        })?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!(
                "snapshot spool component is not a regular non-reparse directory: {}",
                current.display()
            ));
        }
        handles.push(handle);
    }
    if handles.is_empty() {
        return Err("snapshot spool must not be a filesystem root".into());
    }
    Ok(handles)
}

fn parse_args(iter: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut iter = iter.into_iter();
    let verb = iter.next().ok_or("missing internal worker verb")?;
    if verb.to_str() != Some(INTERNAL_VERB) {
        return Err("invalid internal worker verb".into());
    }
    let mut parsed = Args::default();
    while let Some(flag) = iter.next() {
        let flag_text = flag
            .to_str()
            .ok_or_else(|| "worker option name is not Unicode".to_string())?;
        let value = iter
            .next()
            .ok_or_else(|| format!("missing value for {flag_text}"))?;
        match flag_text {
            "--game-id" => set_string(&mut parsed.game_id, value, flag_text)?,
            "--source" => set_path(&mut parsed.source, value, flag_text)?,
            "--snapshot-dir" => set_path(&mut parsed.snapshot_dir, value, flag_text)?,
            "--source-len" => {
                if parsed.source_len.is_some() {
                    return Err("duplicate --source-len".into());
                }
                let text = value
                    .into_string()
                    .map_err(|_| "--source-len is not Unicode".to_string())?;
                parsed.source_len = Some(
                    text.parse::<u64>()
                        .map_err(|error| format!("invalid --source-len: {error}"))?,
                );
            }
            "--source-sha256" => set_string(&mut parsed.source_sha256, value, flag_text)?,
            "--worker-executable-sha256" => {
                set_string(&mut parsed.executable_sha256, value, flag_text)?
            }
            "--identity-mode" => {
                if parsed.identity_mode.is_some() {
                    return Err("duplicate --identity-mode".into());
                }
                let text = value
                    .into_string()
                    .map_err(|_| "--identity-mode is not Unicode".to_string())?;
                parsed.identity_mode = Some(match text.as_str() {
                    "legacy" => snapshot::PrefoldStreamIdentityMode::Legacy,
                    "identity-v1" => snapshot::PrefoldStreamIdentityMode::IdentityV1,
                    _ => return Err(format!("invalid --identity-mode {text:?}")),
                });
            }
            other => return Err(format!("unknown internal worker option {other}")),
        }
    }
    Ok(parsed)
}

fn set_string(slot: &mut Option<String>, value: OsString, flag: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate {flag}"));
    }
    *slot = Some(
        value
            .into_string()
            .map_err(|_| format!("{flag} is not Unicode"))?,
    );
    Ok(())
}

fn set_path(slot: &mut Option<PathBuf>, value: OsString, flag: &str) -> Result<(), String> {
    if slot.replace(PathBuf::from(value)).is_some() {
        return Err(format!("duplicate {flag}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn open_directory_for_delete(path: &Path) -> std::io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt as _;

        const DELETE: u32 = 0x0001_0000;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

        std::fs::OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    #[test]
    fn internal_mode_requires_exact_leading_verb() {
        assert!(is_internal_invocation(&[OsString::from(INTERNAL_VERB)]));
        assert!(!is_internal_invocation(&[OsString::from("--recording")]));
        assert!(!is_internal_invocation(&[
            OsString::from("--recording"),
            OsString::from(INTERNAL_VERB),
        ]));
    }

    #[test]
    fn parser_requires_single_executable_identity() {
        let base = [
            OsString::from(INTERNAL_VERB),
            OsString::from("--worker-executable-sha256"),
            OsString::from("11".repeat(32)),
        ];
        assert_eq!(
            parse_args(base.clone()).unwrap().executable_sha256,
            Some("11".repeat(32))
        );
        assert!(parse_args(base.into_iter().chain([
            OsString::from("--worker-executable-sha256"),
            OsString::from("22".repeat(32)),
        ]))
        .is_err());
    }

    #[test]
    fn parser_rejects_unknown_and_duplicate_options() {
        assert!(parse_args([
            OsString::from(INTERNAL_VERB),
            OsString::from("--unknown"),
            OsString::from("x"),
        ])
        .is_err());
        assert!(parse_args([
            OsString::from(INTERNAL_VERB),
            OsString::from("--identity-mode"),
            OsString::from("legacy"),
            OsString::from("--identity-mode"),
            OsString::from("identity-v1"),
        ])
        .is_err());
    }

    #[test]
    fn snapshot_spool_paths_reject_lexical_aliases() {
        assert!(absolute_clean_path(Path::new("snapshots/../outside")).is_err());
        assert!(absolute_clean_path(Path::new("./snapshots")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn snapshot_spool_guard_retains_directory_against_replacement() {
        let root = std::env::temp_dir().join(format!(
            "replay_app_prefold_snapshot_guard_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let snapshots = root.join("snapshots");
        let moved = root.join("moved");
        std::fs::create_dir_all(&snapshots).unwrap();
        let guard = SnapshotDirectoryGuard::acquire(&snapshots).unwrap();
        // Renaming a Windows directory requires DELETE access. Probe that
        // capability directly instead of relying on std::fs::rename, whose
        // Windows implementation may select different rename APIs and POSIX
        // fallback behavior across Rust releases.
        assert!(open_directory_for_delete(&snapshots).is_err());
        drop(guard);
        open_directory_for_delete(&snapshots).unwrap();
        std::fs::rename(&snapshots, &moved).unwrap();
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }
}

}
