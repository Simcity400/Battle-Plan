#![cfg(windows)]

use snapshot::Store;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::{mem::MaybeUninit, ptr::null_mut};
use std::process::Command;
use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, CreateSymbolicLinkW, FileAttributeTagInfo, FileIdInfo,
    GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
};

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "snapshot_namespace_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Creates a directory junction, retrying only when Windows failed to *start*
/// `cmd.exe` at all.
///
/// Under a full `cargo test --workspace` run, 200+ test binaries spawn
/// short-lived processes concurrently and process creation intermittently fails
/// with `STATUS_DLL_INIT_FAILED` (`0xC000_0142`) — the child is created but its
/// loader cannot initialise, so it dies before `mklink` ever runs. That is a
/// harness resource stall, not a product signal. The identical helper in
/// `recorder_host` was seen failing this way once in four full runs.
///
/// Only that exact code is retried. It is matched exactly rather than by the
/// NTSTATUS range `>= 0xC000_0000`, because that range is not exclusive to the
/// loader: a process may pass any 32-bit value to `ExitProcess`, `TerminateProcess`
/// may impose one, and an unhandled exception surfaces as its own status code. A
/// range test would therefore also retry an access violation or a fast-fail inside
/// the child. `mklink` reports its own refusals with exit code 1, which stays a
/// hard failure on the first attempt, so a genuinely broken junction still fails.
/// The post-condition keeps the callers falsifiable: if junction creation ever
/// silently produced a plain directory, this fires rather than letting the
/// reparse-point tests pass over a normal directory.
fn make_junction(link: &Path, target: &Path) {
    const MAX_ATTEMPTS: u32 = 5;
    const STATUS_DLL_INIT_FAILED: u32 = 0xC000_0142;
    for attempt in 1..=MAX_ATTEMPTS {
        let status = Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .expect("launch mklink /J");
        if status.success() {
            break;
        }
        let child_never_ran = status
            .code()
            .is_some_and(|code| code as u32 == STATUS_DLL_INIT_FAILED);
        assert!(
            child_never_ran && attempt < MAX_ATTEMPTS,
            "mklink /J failed with {status}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50 * u64::from(attempt)));
    }
    assert!(
        fs::symlink_metadata(link)
            .expect("junction was reported created")
            .file_type()
            .is_symlink(),
        "mklink /J reported success without leaving a reparse point at {}",
        link.display()
    );
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

const FILE_ATTRIBUTE_OFFLINE_RAW: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_REPARSE_POINT_RAW: u32 = 0x0000_0400;
const FILE_ATTRIBUTE_RECALL_ON_OPEN_RAW: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_RAW: u32 = 0x0040_0000;

fn carries_dehydrated_payload_metadata(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE_RAW
            | FILE_ATTRIBUTE_RECALL_ON_OPEN_RAW
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_RAW)
        != 0
}

#[derive(Debug, Eq, PartialEq)]
struct WindowsHandleIdentity {
    reparse_tag: u32,
    volume_serial_number: String,
    file_id_128: String,
}

fn windows_handle_identity(
    path: &Path,
    flags: u32,
) -> Result<WindowsHandleIdentity, String> {
    let path = wide(path);
    let raw = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(format!(
            "metadata-only CreateFileW failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut info = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileAttributeTagInfo,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(format!(
            "FileAttributeTagInfo failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let tag = unsafe { info.assume_init() }.ReparseTag;
    let mut file_id = MaybeUninit::<FILE_ID_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            file_id.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(format!(
            "FileIdInfo failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file_id = unsafe { file_id.assume_init() };
    if file_id.FileId.Identifier.iter().all(|byte| *byte == 0)
        || file_id.FileId.Identifier.iter().all(|byte| *byte == u8::MAX)
    {
        return Err("FileIdInfo returned an unavailable 128-bit identifier sentinel".into());
    }
    let file_id_128 = file_id
        .FileId
        .Identifier
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(WindowsHandleIdentity {
        reparse_tag: tag,
        volume_serial_number: format!("{:016x}", file_id.VolumeSerialNumber),
        file_id_128,
    })
}

fn cloud_files_identity(path: &Path) -> Result<WindowsHandleIdentity, String> {
    let identity = windows_handle_identity(path, FILE_FLAG_OPEN_REPARSE_POINT)?;
    if identity.reparse_tag & 0xffff_0fff != 0x9000_001a {
        return Err(format!(
            "selected reparse tag {:#010x} is not Cloud Files",
            identity.reparse_tag
        ));
    }
    Ok(identity)
}

fn require_expected_identity(
    identity: &WindowsHandleIdentity,
    volume_variable: &str,
    file_id_variable: &str,
    context: &str,
) {
    let expected_volume = std::env::var(volume_variable)
        .unwrap_or_else(|_| panic!("F3_ENV_GATE={volume_variable}_NOT_SET"));
    let expected_file_id = std::env::var(file_id_variable)
        .unwrap_or_else(|_| panic!("F3_ENV_GATE={file_id_variable}_NOT_SET"));
    assert_eq!(
        identity.volume_serial_number, expected_volume,
        "F3_ENV_GATE={context}_VOLUME_IDENTITY_CHANGED"
    );
    assert_eq!(
        identity.file_id_128, expected_file_id,
        "F3_ENV_GATE={context}_FILE_IDENTITY_CHANGED"
    );
}

/// Metadata-only preflight. This never opens a file for data access, so it
/// cannot accidentally hydrate the fixture merely while deciding whether the
/// ignored integration gate is valid to run.
fn dehydrated_replay_snapshot_candidates(
    root: &Path,
) -> Result<(usize, Vec<(String, String)>), String> {
    let mut pending = vec![root.to_path_buf()];
    let mut scanned_files = 0usize;
    let mut candidates = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!("enumerate metadata under {}: {error}", directory.display())
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("read metadata entry under {}: {error}", directory.display())
            })?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                format!(
                    "query link-safe metadata for {}: {error}",
                    entry.path().display()
                )
            })?;
            let attributes = metadata.file_attributes();
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                // The preflight does not resolve descendant reparse directories;
                // the real anchored loader owns tag classification and refusal.
                if attributes & FILE_ATTRIBUTE_REPARSE_POINT_RAW == 0 {
                    pending.push(entry.path());
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            scanned_files += 1;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if carries_dehydrated_payload_metadata(attributes)
                && attributes & FILE_ATTRIBUTE_REPARSE_POINT_RAW != 0
                && name.ends_with(".snapshot.json")
            {
                let relative = path.strip_prefix(root).map_err(|error| {
                    format!("derive replay-relative placeholder path: {error}")
                })?;
                let components = relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                if let [game, file] = components.as_slice() {
                    candidates.push((game.clone(), file.clone()));
                }
            }
        }
    }
    candidates.sort();
    Ok((scanned_files, candidates))
}

#[test]
fn intermediate_junction_is_name_surrogate_rejected() {
    let temp = Temp::new("intermediate_junction");
    let root = temp.0.join("store");
    let game = root.join("g");
    let outside = temp.0.join("outside_recorder");
    fs::create_dir_all(&game).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("manifest.json"), b"{}").unwrap();
    make_junction(&game.join("recorder"), &outside);

    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "recorder/manifest.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate"), "{error}");
    assert!(error.contains("recorder"), "{error}");
}

#[test]
fn game_component_junction_is_name_surrogate_rejected() {
    let temp = Temp::new("game_junction");
    let root = temp.0.join("store");
    let outside = temp.0.join("outside_game");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("timeline.json"), b"{}").unwrap();
    make_junction(&root.join("g"), &outside);

    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate"), "{error}");
    assert!(
        error.contains("component") && error.contains('g'),
        "{error}"
    );
}

#[test]
fn file_symlink_capability_and_rejection_are_mandatory() {
    let temp = Temp::new("file_symlink");
    let root = temp.0.join("store");
    let game = root.join("g");
    fs::create_dir_all(&game).unwrap();
    let target = game.join("target.json");
    let link = game.join("timeline.json");
    fs::write(&target, b"{}").unwrap();
    let link_wide = wide(&link);
    let target_wide = wide(&target);
    let created = unsafe {
        CreateSymbolicLinkW(
            link_wide.as_ptr(),
            target_wide.as_ptr(),
            SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
        )
    };
    if !created {
        let code = unsafe { GetLastError() };
        if code == 1314 {
            panic!(
                "F3_ENV_GATE=SYMLINK_PRIVILEGE_NOT_HELD: CreateSymbolicLinkW failed with Win32 1314; enable Windows Developer Mode (or supply SeCreateSymbolicLinkPrivilege), then rerun this exact gate"
            );
        }
        panic!(
            "F3_ENV_GATE=SYMLINK_PROVISION_UNEXPECTED_ERROR: CreateSymbolicLinkW failed with Win32 {code}; Developer Mode alone may not resolve this error"
        );
    }

    let store = Store::open_directory(&root).unwrap();
    let error = store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap_err()
        .to_string();
    assert!(error.contains("name-surrogate"), "{error}");
    assert!(error.contains("timeline.json"), "{error}");
}

#[test]
fn linked_root_is_permitted_and_descendants_remain_anchored() {
    let temp = Temp::new("linked_root");
    let real = temp.0.join("real_store");
    let game = real.join("g");
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
    let linked = temp.0.join("linked_store");
    make_junction(&linked, &real);

    let store = Store::open_directory(&linked).unwrap();
    let bytes = store
        .read_game_file_bytes("g", "timeline.json")
        .unwrap()
        .unwrap();
    assert_eq!(bytes, br#"{"legacy":true}"#);
}

/// Must run on the primary OneDrive dev rig before ship. Point the environment
/// variable at a genuinely dehydrated directory replay store; this proves that
/// the non-surrogate reopen hydrates and returns logical bytes.
#[test]
#[ignore = "requires a real dehydrated OneDrive Files-On-Demand store on the dev rig"]
fn windows_onedrive_dehydrated_placeholder_store_must_pass_before_ship() {
    let provider_state = std::env::var("BATTLE_PLAN_ONEDRIVE_SYNC_PROVIDER_STATE").or_else(|_| std::env::var("HOI4_ONEDRIVE_SYNC_PROVIDER_STATE"))
        .expect("F3_ENV_GATE=ONEDRIVE_PROVIDER_STATE_NOT_SET: set enabled or disabled through the deployment acceptance runner");
    assert!(
        matches!(provider_state.as_str(), "enabled" | "disabled"),
        "F3_ENV_GATE=ONEDRIVE_PROVIDER_STATE_INVALID: expected enabled or disabled"
    );
    let path = std::env::var_os("BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE").or_else(|| std::env::var_os("HOI4_ONEDRIVE_DEHYDRATED_REPLAY_STORE"))
        .map(PathBuf::from)
        .expect("F3_ENV_GATE=ONEDRIVE_STORE_ENV_NOT_SET: set BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE to a genuinely dehydrated directory store");
    let (scanned_files, candidates) = dehydrated_replay_snapshot_candidates(&path)
        .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_METADATA_PROBE_FAILED: {error}"));
    assert!(
        !candidates.is_empty(),
        "F3_ENV_GATE=ONEDRIVE_REPLAY_SNAPSHOT_NOT_DEHYDRATED: metadata-only scan found no dehydrated <game>/*.snapshot.json payload among {scanned_files} files; refusing an unrelated-placeholder or hydrated-store pass"
    );
    let selected_path = std::env::var("BATTLE_PLAN_ONEDRIVE_SELECTED_REPLAY_SNAPSHOT").or_else(|_| std::env::var("HOI4_ONEDRIVE_SELECTED_REPLAY_SNAPSHOT"))
        .expect("F3_ENV_GATE=ONEDRIVE_SELECTED_SNAPSHOT_NOT_SET: deployment runner must select one exact game/snapshot.json placeholder");
    assert!(
        !selected_path.contains('\\') && selected_path.split('/').count() == 2,
        "F3_ENV_GATE=ONEDRIVE_SELECTED_SNAPSHOT_INVALID: expected game/file.snapshot.json"
    );
    let (game, file) = candidates
        .iter()
        .find(|(game, file)| selected_path == format!("{game}/{file}"))
        .expect("F3_ENV_GATE=ONEDRIVE_SELECTED_SNAPSHOT_NOT_DEHYDRATED: selected exact snapshot is absent from the fresh placeholder census");
    let expected_tag_text = std::env::var("BATTLE_PLAN_ONEDRIVE_SELECTED_REPARSE_TAG").or_else(|_| std::env::var("HOI4_ONEDRIVE_SELECTED_REPARSE_TAG"))
        .expect("F3_ENV_GATE=ONEDRIVE_SELECTED_REPARSE_TAG_NOT_SET");
    let expected_tag = u32::from_str_radix(
        expected_tag_text
            .strip_prefix("0x")
            .expect("selected reparse tag must use 0xhhhhhhhh form"),
        16,
    )
    .expect("selected reparse tag must be hexadecimal");
    let root_identity = windows_handle_identity(&path, FILE_FLAG_BACKUP_SEMANTICS)
        .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_STORE_IDENTITY_REJECTED: {error}"));
    require_expected_identity(
        &root_identity,
        "BATTLE_PLAN_ONEDRIVE_STORE_VOLUME_SERIAL_NUMBER",
        "BATTLE_PLAN_ONEDRIVE_STORE_FILE_ID_128",
        "ONEDRIVE_STORE",
    );
    let selected_identity = cloud_files_identity(&path.join(game).join(file))
        .unwrap_or_else(|error| panic!("F3_ENV_GATE=ONEDRIVE_CLOUD_TAG_REJECTED: {error}"));
    assert_eq!(
        selected_identity.reparse_tag, expected_tag,
        "F3_ENV_GATE=ONEDRIVE_SELECTED_REPARSE_TAG_CHANGED"
    );
    require_expected_identity(
        &selected_identity,
        "BATTLE_PLAN_ONEDRIVE_SELECTED_VOLUME_SERIAL_NUMBER",
        "BATTLE_PLAN_ONEDRIVE_SELECTED_FILE_ID_128",
        "ONEDRIVE_SELECTED",
    );
    let store = Store::open_directory(&path).unwrap();
    let index = store.index(game).unwrap();
    let selected = index
        .snapshots
        .iter()
        .find(|entry| entry.file.as_str() == file.as_str())
        .expect("dehydrated replay snapshot must be listed by its game index");
    let loaded = store.load_file(game, &selected.file);
    if provider_state == "disabled" {
        match loaded {
            Err(snapshot::LoadError::Directory(error)) => assert!(
                !error.trim().is_empty(),
                "disabled-provider directory refusal lost its diagnostic"
            ),
            Err(error) => panic!(
                "disabled provider produced a non-I/O/non-namespace failure that could mask a malformed fixture: {error}"
            ),
            Ok(_) => {
                panic!("disabled sync provider unexpectedly supplied logical placeholder bytes")
            }
        }
    } else {
        let snapshot = loaded.expect("enabled sync provider failed to hydrate exact snapshot");
        assert_eq!(snapshot.game_id.as_deref(), Some(game.as_str()));
    }
}

#[test]
fn dehydrated_placeholder_attribute_probe_is_exact_and_non_vacuous() {
    assert!(!carries_dehydrated_payload_metadata(0));
    assert!(!carries_dehydrated_payload_metadata(
        FILE_ATTRIBUTE_REPARSE_POINT_RAW
    ));
    for attribute in [
        FILE_ATTRIBUTE_OFFLINE_RAW,
        FILE_ATTRIBUTE_RECALL_ON_OPEN_RAW,
        FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_RAW,
    ] {
        assert!(carries_dehydrated_payload_metadata(attribute));
    }
}
