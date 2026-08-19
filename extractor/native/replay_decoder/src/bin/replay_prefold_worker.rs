
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

pub(crate) const INTERNAL_VERB: &str = "--internal-prefold-worker-v1";
pub(crate) const RECEIPT_SCHEMA: &str = "hoi4_prefold_worker_receipt_v2";
pub(crate) const DECODER_ID: &str = "extractor.replay_decoder.production_world_fold.worker.v1";

#[derive(Default)]
struct Args {
    game_id: Option<String>,
    source: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
    source_len: Option<u64>,
    source_sha256: Option<String>,
    identity_mode: Option<snapshot::PrefoldStreamIdentityMode>,
    worker_executable_sha256: Option<String>,
}

fn main() {
    if let Err(error) = run(std::env::args_os().skip(1)) {
        eprintln!("replay_prefold_worker: {error}");
        std::process::exit(2);
    }
}

pub(crate) fn is_internal_invocation(args: &[OsString]) -> bool {
    args.first().and_then(|value| value.to_str()) == Some(INTERNAL_VERB)
}

pub(crate) fn run(iter: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut iter = iter.into_iter();
    let first = iter.next().ok_or("missing worker arguments")?;
    let args = if first.to_str() == Some(INTERNAL_VERB) {
        parse_args(iter)?
    } else {
        parse_args(std::iter::once(first).chain(iter))?
    };
    let expected_worker_sha256 = args.worker_executable_sha256;
    let worker_executable_sha256 = expected_worker_sha256
        .as_deref()
        .map(authenticate_current_executable)
        .transpose()?;
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
    let spool = if worker_executable_sha256.is_some() {
        let source_file = duplicate_inherited_source_file()?;
        replay_decoder::fold_raw_h4cr_open_file_spooled(request, source_file)
    } else {
        replay_decoder::fold_raw_h4cr_file_spooled(request)
    }
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
        "worker_executable_sha256": worker_executable_sha256,
        "identity_mode": identity_mode_label,
        "game_id": spool.game_id(),
        "schema_version": spool.schema_version(),
        "entries": entries,
    });
    serde_json::to_writer(std::io::stdout().lock(), &receipt)
        .map_err(|error| format!("serialize worker receipt: {error}"))?;
    Ok(())
}

fn authenticate_current_executable(expected: &str) -> Result<String, String> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("--worker-executable-sha256 must be lowercase SHA-256".into());
    }
    let path = std::env::current_exe()
        .map_err(|error| format!("resolve recorder worker executable: {error}"))?;
    let mut file = open_current_executable(&path)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind recorder worker executable: {error}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("read recorder worker executable: {error}"))?;
    let actual = snapshot::sha256::Sha256::hex(&bytes);
    if actual != expected {
        return Err("internal prefold worker executable identity mismatch".into());
    }
    Ok(actual)
}

#[cfg(windows)]
fn open_current_executable(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| format!("retain recorder worker executable: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect recorder worker executable: {error}"))?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("recorder worker executable is not a regular non-reparse file".into());
    }
    Ok(file)
}

#[cfg(not(windows))]
fn open_current_executable(path: &Path) -> Result<File, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect recorder worker executable: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("recorder worker executable is not a regular non-symlink file".into());
    }
    File::open(path).map_err(|error| format!("retain recorder worker executable: {error}"))
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
    if source_handle.is_null() || source_handle as isize == -1 {
        return Err("internal prefold worker did not inherit a valid source handle".into());
    }
    let process = unsafe { GetCurrentProcess() };
    let mut duplicated = std::ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            source_handle,
            process,
            &mut duplicated,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(format!(
            "duplicate inherited prefold source handle: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_handle(duplicated) };
    file.metadata()
        .map_err(|error| format!("inspect inherited prefold source handle: {error}"))?;
    Ok(file)
}

#[cfg(not(windows))]
fn duplicate_inherited_source_file() -> Result<File, String> {
    Err("authenticated prefold source-handle inheritance is unsupported on this platform".into())
}

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
        return Err("snapshot spool path is empty".to_string());
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
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("resolve snapshot spool path: {error}"))?
            .join(path)
    };
    Ok(absolute)
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
        return Err("snapshot spool must not be a filesystem root".to_string());
    }
    Ok(handles)
}

fn parse_args(iter: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut iter = iter.into_iter();
    let mut parsed = Args::default();
    while let Some(flag) = iter.next() {
        let flag_text = flag
            .to_str()
            .ok_or_else(|| "worker option name is not Unicode".to_string())?;
        let value = iter
            .next()
            .ok_or_else(|| format!("missing value for {flag_text}"))?;
        match flag_text {
            "--game-id" => {
                if parsed.game_id.is_some() {
                    return Err("duplicate --game-id".into());
                }
                parsed.game_id = Some(
                    value
                        .into_string()
                        .map_err(|_| "--game-id is not Unicode".to_string())?,
                );
            }
            "--source" => {
                if parsed.source.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --source".into());
                }
            }
            "--snapshot-dir" => {
                if parsed.snapshot_dir.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --snapshot-dir".into());
                }
            }
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
            "--source-sha256" => {
                if parsed.source_sha256.is_some() {
                    return Err("duplicate --source-sha256".into());
                }
                parsed.source_sha256 = Some(
                    value
                        .into_string()
                        .map_err(|_| "--source-sha256 is not Unicode".to_string())?,
                );
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
                    _ => {
                        return Err(format!(
                            "invalid --identity-mode {text:?}; expected legacy or identity-v1"
                        ))
                    }
                });
            }
            "--worker-executable-sha256" => {
                if parsed.worker_executable_sha256.is_some() {
                    return Err("duplicate --worker-executable-sha256".into());
                }
                parsed.worker_executable_sha256 = Some(
                    value.into_string().map_err(|_| {
                        "--worker-executable-sha256 is not Unicode".to_string()
                    })?,
                );
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_rejects_unknown_and_duplicate_options() {
        assert!(parse_args([OsString::from("--unknown"), OsString::from("x")]).is_err());
        assert!(parse_args([
            OsString::from("--game-id"),
            OsString::from("g"),
            OsString::from("--game-id"),
            OsString::from("g2"),
        ])
        .is_err());
        assert!(parse_args([
            OsString::from("--identity-mode"),
            OsString::from("legacy"),
            OsString::from("--identity-mode"),
            OsString::from("identity-v1"),
        ])
        .is_err());
        assert!(parse_args([OsString::from("--identity-mode"), OsString::from("sniff"),]).is_err());
    }

    #[test]
    fn parser_accepts_both_explicit_identity_modes() {
        for (wire, expected) in [
            ("legacy", snapshot::PrefoldStreamIdentityMode::Legacy),
            (
                "identity-v1",
                snapshot::PrefoldStreamIdentityMode::IdentityV1,
            ),
        ] {
            let parsed =
                parse_args([OsString::from("--identity-mode"), OsString::from(wire)]).unwrap();
            assert_eq!(parsed.identity_mode, Some(expected));
        }
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
            "prefold_snapshot_guard_{}_{}",
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
        assert!(std::fs::rename(&snapshots, &moved).is_err());
        drop(guard);
        std::fs::rename(&snapshots, &moved).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
