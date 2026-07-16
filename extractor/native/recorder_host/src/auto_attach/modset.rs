//! Product Paradox-user-directory discovery for automatic attach.
//!
//! Candidate *discovery* is deliberately separate from candidate *proof*.
//! Windows location APIs and the target process command line tell us where the
//! directory may be; the caller supplies the existing PID-held-log
//! corroboration callback that proves which physical directory the running
//! game actually owns.  Exactly one corroborated physical directory is
//! required.  Zero and multiple survivors both fail closed.

use std::collections::BTreeMap;
use std::ffi::{c_void, OsString};
use std::path::{Path, PathBuf};

const GAME_USER_DIR_SUFFIX: [&str; 2] = ["Paradox Interactive", "Hearts of Iron IV"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocatedUserDir {
    pub path: PathBuf,
    pub provenance: Vec<String>,
    pub explicit_override: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CandidateCorroboration {
    Owned,
    Rejected(String),
    /// The candidate could not be classified. A second owned candidate cannot
    /// be selected while this exists because that would convert unknown
    /// ambiguity into a false singleton.
    Indeterminate(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    path: PathBuf,
    provenance: Vec<String>,
}

/// All operating-system interactions used by discovery.  The resolver is
/// generic over this seam so ambiguity, unavailable APIs, custom `-userdir`,
/// redirected Documents, and multiple OneDrive accounts are deterministic
/// unit-test cases rather than machine-dependent integration tests.
pub(crate) trait DiscoveryEnvironment {
    fn process_command_line_args(&self, pid: u32) -> Result<Vec<OsString>, String>;
    fn known_documents_dir(&self) -> Result<PathBuf, String>;
    fn one_drive_roots(&self) -> Result<Vec<(String, PathBuf)>, String>;
    fn user_profile_dir(&self) -> Result<PathBuf, String>;
    fn contains_dlc_load(&self, dir: &Path) -> bool;
    fn canonicalize_dir(&self, dir: &Path) -> Result<PathBuf, String>;
}

/// Discover and prove the target process's Paradox user directory.
///
/// `explicit_override` is a support/legacy candidate. It never disables normal
/// discovery and never bypasses `contains_dlc_load`, canonicalization, or
/// `corroborate`, so an override cannot turn unresolved evidence into a
/// resolved modset.
pub(crate) fn locate_user_dir_with<E, F>(
    env: &E,
    pid: u32,
    explicit_override: Option<&Path>,
    mut corroborate: F,
) -> Result<LocatedUserDir, String>
where
    E: DiscoveryEnvironment,
    F: FnMut(&Path) -> CandidateCorroboration,
{
    if let Some(path) = explicit_override {
        if !env.contains_dlc_load(path) {
            return Err(format!(
                "explicit --pdx-user-dir {} does not contain dlc_load.json",
                path.display()
            ));
        }
    }
    let mut raw = discover_all_candidates(env, pid)?;
    if let Some(path) = explicit_override {
        // A support pin adds a candidate; it never narrows or disables normal
        // discovery. Otherwise an override could hide a second PID-owned
        // directory and turn ambiguity into a false singleton.
        raw.push(Candidate {
            path: path.to_path_buf(),
            provenance: vec!["support_override".to_string()],
        });
    }

    // Canonicalize and de-duplicate only candidates that actually carry the
    // launch configuration.  Multiple syntactic paths to the same physical
    // directory (Known Folder + OneDrive + legacy) are one candidate.
    let mut physical: BTreeMap<String, Candidate> = BTreeMap::new();
    let mut absent = Vec::new();
    let mut canonical_failures = Vec::new();
    for candidate in raw {
        if !env.contains_dlc_load(&candidate.path) {
            absent.push(candidate.path.display().to_string());
            continue;
        }
        let canonical = match env.canonicalize_dir(&candidate.path) {
            Ok(path) => path,
            Err(error) => {
                canonical_failures.push(format!("{}: {error}", candidate.path.display()));
                continue;
            }
        };
        let key = physical_path_key(&canonical);
        physical
            .entry(key)
            .and_modify(|existing| {
                for source in &candidate.provenance {
                    if !existing.provenance.contains(source) {
                        existing.provenance.push(source.clone());
                    }
                }
                existing.provenance.sort();
            })
            .or_insert_with(|| Candidate {
                path: canonical,
                provenance: candidate.provenance,
            });
    }

    // An existing `dlc_load.json` candidate that cannot be canonicalized may
    // be a second physical directory. Choosing another hit would therefore
    // turn an unknown into a false singleton.
    if !canonical_failures.is_empty() {
        canonical_failures.sort();
        return Err(format!(
            "Paradox user-directory discovery incomplete for pid {pid}: canonicalization failed for [{}]",
            canonical_failures.join(" | ")
        ));
    }

    let mut survivors = Vec::new();
    let mut rejected = Vec::new();
    for (_, candidate) in physical {
        match corroborate(&candidate.path) {
            CandidateCorroboration::Owned => survivors.push(candidate),
            CandidateCorroboration::Indeterminate(error) => {
                return Err(format!(
                    "Paradox user-directory proof indeterminate for pid {pid} at {}: {error}",
                    candidate.path.display()
                ));
            }
            CandidateCorroboration::Rejected(error) => {
                if candidate
                    .provenance
                    .iter()
                    .any(|source| source == "support_override")
                {
                    return Err(format!(
                        "explicit --pdx-user-dir {} was not corroborated against pid {pid}: {error}",
                        candidate.path.display()
                    ));
                }
                rejected.push(format!("{}: {error}", candidate.path.display()));
            }
        }
    }

    match survivors.len() {
        1 => {
            let candidate = survivors.pop().expect("length checked");
            let explicit_override = candidate
                .provenance
                .iter()
                .any(|source| source == "support_override");
            Ok(LocatedUserDir {
                path: candidate.path,
                provenance: candidate.provenance,
                explicit_override,
            })
        }
        0 => {
            absent.sort();
            canonical_failures.sort();
            rejected.sort();
            Err(format!(
                "no corroborated Paradox user directory for pid {pid}; missing dlc_load.json [{}]; canonicalization failures [{}]; corroboration failures [{}]",
                absent.join(", "),
                canonical_failures.join(" | "),
                rejected.join(" | "),
            ))
        }
        _ => {
            let paths = survivors
                .iter()
                .map(|candidate| candidate.path.display().to_string())
                .collect::<Vec<_>>();
            Err(format!(
                "AMBIGUOUS Paradox user directory for pid {pid}: multiple physical directories were corroborated ({})",
                paths.join(", ")
            ))
        }
    }
}

fn discover_all_candidates<E: DiscoveryEnvironment>(
    env: &E,
    pid: u32,
) -> Result<Vec<Candidate>, String> {
    // Every source is mandatory to query.  Silently ignoring an unavailable
    // source can make a later single hit look unambiguous while the real custom
    // directory was merely undiscoverable.
    let args = env
        .process_command_line_args(pid)
        .map_err(|e| format!("target process command line unavailable: {e}"))?;
    let command_line_user_dir = parse_userdir_args(&args)?;
    let known_documents = env
        .known_documents_dir()
        .map_err(|e| format!("FOLDERID_Documents unavailable: {e}"))?;
    let one_drive_roots = env
        .one_drive_roots()
        .map_err(|e| format!("runtime OneDrive roots unavailable: {e}"))?;
    let profile = env
        .user_profile_dir()
        .map_err(|e| format!("USERPROFILE unavailable: {e}"))?;

    let mut out = Vec::new();
    if let Some(path) = command_line_user_dir {
        out.push(Candidate {
            path,
            provenance: vec!["process_command_line:-userdir".to_string()],
        });
    }
    out.push(Candidate {
        path: append_game_dir(known_documents),
        provenance: vec!["known_folder:FOLDERID_Documents".to_string()],
    });
    for (source, root) in one_drive_roots {
        out.push(Candidate {
            path: append_game_dir(root.join("Documents")),
            provenance: vec![format!("onedrive:{source}")],
        });
    }
    out.push(Candidate {
        path: append_game_dir(profile.join("Documents")),
        provenance: vec!["legacy:USERPROFILE/Documents".to_string()],
    });
    out.push(Candidate {
        path: append_game_dir(profile.join("OneDrive").join("Documents")),
        provenance: vec!["legacy:USERPROFILE/OneDrive/Documents".to_string()],
    });
    Ok(out)
}

fn append_game_dir(mut documents: PathBuf) -> PathBuf {
    for component in GAME_USER_DIR_SUFFIX {
        documents.push(component);
    }
    documents
}

fn physical_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// Extract exactly one absolute `-userdir` value from already-tokenized Windows
/// command-line arguments.  Both `-userdir PATH` and `-userdir=PATH` are
/// accepted case-insensitively.  Repetition is ambiguity even if the strings
/// happen to match: engine precedence for repeated switches is not assumed.
fn parse_userdir_args(args: &[OsString]) -> Result<Option<PathBuf>, String> {
    let mut found: Option<PathBuf> = None;
    let mut index = 0usize;
    while index < args.len() {
        let value = args[index].to_string_lossy();
        let lower = value.to_ascii_lowercase();
        let parsed = if lower == "-userdir" {
            index += 1;
            let next = args
                .get(index)
                .ok_or_else(|| "process command line has -userdir without a value".to_string())?;
            Some(PathBuf::from(next.as_os_str()))
        } else if lower.starts_with("-userdir=") {
            Some(PathBuf::from(&value["-userdir=".len()..]))
        } else {
            None
        };
        if let Some(path) = parsed {
            if found.is_some() {
                return Err(
                    "process command line repeats -userdir; precedence is ambiguous".into(),
                );
            }
            if path.as_os_str().is_empty() {
                return Err("process command line has a blank -userdir value".into());
            }
            if !path.is_absolute() {
                return Err(format!(
                    "process command line -userdir is relative ({}); target current-directory semantics are unproven",
                    path.display()
                ));
            }
            found = Some(path);
        }
        index += 1;
    }
    Ok(found)
}

// =====================================================================
// Live Windows discovery environment
// =====================================================================

#[cfg(windows)]
pub(crate) struct WindowsDiscoveryEnvironment;

#[cfg(windows)]
impl DiscoveryEnvironment for WindowsDiscoveryEnvironment {
    fn process_command_line_args(&self, pid: u32) -> Result<Vec<OsString>, String> {
        windows::process_command_line_args(pid)
    }

    fn known_documents_dir(&self) -> Result<PathBuf, String> {
        windows::known_documents_dir()
    }

    fn one_drive_roots(&self) -> Result<Vec<(String, PathBuf)>, String> {
        windows::one_drive_roots()
    }

    fn user_profile_dir(&self) -> Result<PathBuf, String> {
        std::env::var_os("USERPROFILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| "USERPROFILE is unset or blank".to_string())
    }

    fn contains_dlc_load(&self, dir: &Path) -> bool {
        dir.join("dlc_load.json").is_file()
    }

    fn canonicalize_dir(&self, dir: &Path) -> Result<PathBuf, String> {
        std::fs::canonicalize(dir).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr;

    type Handle = *mut c_void;
    type HKey = *mut c_void;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
    const KEY_READ: u32 = 0x0002_0019;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_NO_MORE_ITEMS: i32 = 259;
    const RRF_RT_REG_SZ: u32 = 0x0000_0002;
    const RRF_RT_REG_EXPAND_SZ: u32 = 0x0000_0004;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *const u16,
    }

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const FOLDERID_DOCUMENTS: Guid = Guid {
        data1: 0xFDD3_9AD0,
        data2: 0x238F,
        data3: 0x46AF,
        data4: [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
    };

    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process: Handle,
            information_class: u32,
            information: *mut c_void,
            information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn CommandLineToArgvW(command_line: *const u16, count: *mut i32) -> *mut *mut u16;
        fn SHGetKnownFolderPath(
            folder_id: *const Guid,
            flags: u32,
            token: Handle,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            key: HKey,
            subkey: *const u16,
            options: u32,
            access: u32,
            result: *mut HKey,
        ) -> i32;
        fn RegEnumKeyExW(
            key: HKey,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_len: *mut u32,
            last_write: *mut u64,
        ) -> i32;
        fn RegGetValueW(
            key: HKey,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            value_type: *mut u32,
            data: *mut c_void,
            data_size: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: HKey) -> i32;
    }

    fn wide_nul(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        value
            .as_ref()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    unsafe fn wide_ptr_to_os_string(value: *const u16) -> Result<OsString, String> {
        if value.is_null() {
            return Err("Windows returned a null string".to_string());
        }
        let mut len = 0usize;
        while *value.add(len) != 0 {
            len = len
                .checked_add(1)
                .ok_or_else(|| "Windows string length overflow".to_string())?;
        }
        Ok(OsString::from_wide(std::slice::from_raw_parts(value, len)))
    }

    pub(super) fn process_command_line_args(pid: u32) -> Result<Vec<OsString>, String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err(format!("OpenProcess({pid}) failed"));
            }
            let result = (|| {
                let mut required = 0u32;
                let _ = NtQueryInformationProcess(
                    process,
                    PROCESS_COMMAND_LINE_INFORMATION,
                    ptr::null_mut(),
                    0,
                    &mut required,
                );
                if required < std::mem::size_of::<UnicodeString>() as u32 {
                    return Err(format!(
                        "NtQueryInformationProcess command-line size invalid: {required}"
                    ));
                }
                let mut buffer = vec![0u8; required as usize];
                let status = NtQueryInformationProcess(
                    process,
                    PROCESS_COMMAND_LINE_INFORMATION,
                    buffer.as_mut_ptr().cast(),
                    required,
                    &mut required,
                );
                if status != 0 {
                    return Err(format!(
                        "NtQueryInformationProcess command line failed: NTSTATUS 0x{:08x}",
                        status as u32
                    ));
                }
                let text = ptr::read_unaligned(buffer.as_ptr().cast::<UnicodeString>());
                if text.length == 0 || text.length % 2 != 0 || text.buffer.is_null() {
                    return Err("target command line is empty or malformed".to_string());
                }
                let start = buffer.as_ptr() as usize;
                let end = start
                    .checked_add(buffer.len())
                    .ok_or_else(|| "command-line buffer range overflow".to_string())?;
                let text_start = text.buffer as usize;
                let text_end = text_start
                    .checked_add(text.length as usize)
                    .ok_or_else(|| "command-line string range overflow".to_string())?;
                if text_start < start || text_end > end {
                    return Err("target command-line pointer lies outside returned buffer".into());
                }
                let command_line = OsString::from_wide(std::slice::from_raw_parts(
                    text.buffer,
                    text.length as usize / 2,
                ));
                let mut nul = command_line.encode_wide().collect::<Vec<_>>();
                nul.push(0);
                let mut argc = 0i32;
                let argv = CommandLineToArgvW(nul.as_ptr(), &mut argc);
                if argv.is_null() || argc <= 0 {
                    return Err("CommandLineToArgvW failed".to_string());
                }
                let mut args = Vec::with_capacity(argc as usize);
                for index in 0..argc as usize {
                    args.push(wide_ptr_to_os_string(*argv.add(index))?);
                }
                LocalFree(argv.cast());
                Ok(args)
            })();
            CloseHandle(process);
            result
        }
    }

    pub(super) fn known_documents_dir() -> Result<PathBuf, String> {
        unsafe {
            let mut raw: *mut u16 = ptr::null_mut();
            let result = SHGetKnownFolderPath(&FOLDERID_DOCUMENTS, 0, ptr::null_mut(), &mut raw);
            if result < 0 || raw.is_null() {
                return Err(format!(
                    "SHGetKnownFolderPath failed: HRESULT 0x{:08x}",
                    result as u32
                ));
            }
            let path = PathBuf::from(wide_ptr_to_os_string(raw)?);
            CoTaskMemFree(raw.cast());
            Ok(path)
        }
    }

    pub(super) fn one_drive_roots() -> Result<Vec<(String, PathBuf)>, String> {
        let mut roots = Vec::new();
        for variable in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(value) = std::env::var_os(variable).filter(|v| !v.is_empty()) {
                roots.push((format!("env:{variable}"), PathBuf::from(value)));
            }
        }
        roots.extend(registry_one_drive_roots()?);
        roots.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| physical_path_key(&a.1).cmp(&physical_path_key(&b.1)))
        });
        Ok(roots)
    }

    fn registry_one_drive_roots() -> Result<Vec<(String, PathBuf)>, String> {
        // HKEY_CURRENT_USER is the sign-extended predefined pseudo-handle
        // `(HKEY)(ULONG_PTR)((LONG)0x80000001)`, not a zero-extended address on
        // 64-bit Windows.
        let hkcu = (-2_147_483_647isize) as HKey;
        let accounts_path = wide_nul(r"Software\Microsoft\OneDrive\Accounts");
        unsafe {
            let mut accounts: HKey = ptr::null_mut();
            let rc = RegOpenKeyExW(hkcu, accounts_path.as_ptr(), 0, KEY_READ, &mut accounts);
            if rc == 2 {
                return Ok(Vec::new()); // ERROR_FILE_NOT_FOUND: OneDrive not installed/configured.
            }
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW OneDrive Accounts failed: {rc}"));
            }
            let result = (|| {
                let mut roots = Vec::new();
                for index in 0u32.. {
                    let mut name = vec![0u16; 512];
                    let mut name_len = (name.len() - 1) as u32;
                    let rc = RegEnumKeyExW(
                        accounts,
                        index,
                        name.as_mut_ptr(),
                        &mut name_len,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    );
                    if rc == ERROR_NO_MORE_ITEMS {
                        break;
                    }
                    if rc != ERROR_SUCCESS {
                        return Err(format!(
                            "RegEnumKeyExW OneDrive account {index} failed: {rc}"
                        ));
                    }
                    let account = OsString::from_wide(&name[..name_len as usize]);
                    let account_wide = wide_nul(&account);
                    let mut account_key: HKey = ptr::null_mut();
                    let rc = RegOpenKeyExW(
                        accounts,
                        account_wide.as_ptr(),
                        0,
                        KEY_READ,
                        &mut account_key,
                    );
                    if rc != ERROR_SUCCESS {
                        return Err(format!(
                            "RegOpenKeyExW OneDrive account {} failed: {rc}",
                            account.to_string_lossy()
                        ));
                    }
                    let root = read_registry_string(account_key, "UserFolder");
                    RegCloseKey(account_key);
                    match root {
                        Ok(Some(path)) => roots.push((
                            format!("registry:{}", account.to_string_lossy()),
                            PathBuf::from(path),
                        )),
                        Ok(None) => {}
                        Err(error) => return Err(error),
                    }
                }
                Ok(roots)
            })();
            RegCloseKey(accounts);
            result
        }
    }

    unsafe fn read_registry_string(
        key: HKey,
        value_name: &str,
    ) -> Result<Option<OsString>, String> {
        let value = wide_nul(value_name);
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
        let mut bytes = 0u32;
        let rc = RegGetValueW(
            key,
            ptr::null(),
            value.as_ptr(),
            flags,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut bytes,
        );
        if rc == 2 {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS || bytes < 2 || bytes % 2 != 0 {
            return Err(format!(
                "RegGetValueW {value_name} size failed: {rc}, bytes={bytes}"
            ));
        }
        let mut text = vec![0u16; bytes as usize / 2];
        let rc = RegGetValueW(
            key,
            ptr::null(),
            value.as_ptr(),
            flags,
            ptr::null_mut(),
            text.as_mut_ptr().cast(),
            &mut bytes,
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("RegGetValueW {value_name} failed: {rc}"));
        }
        while text.last() == Some(&0) {
            text.pop();
        }
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(OsString::from_wide(&text)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone)]
    struct FakeEnvironment {
        args: Result<Vec<OsString>, String>,
        documents: Result<PathBuf, String>,
        one_drive: Result<Vec<(String, PathBuf)>, String>,
        profile: Result<PathBuf, String>,
        present: BTreeSet<String>,
        canonical: BTreeMap<String, PathBuf>,
    }

    impl FakeEnvironment {
        fn standard() -> Self {
            Self {
                args: Ok(vec![OsString::from("hoi4.exe")]),
                documents: Ok(PathBuf::from(r"C:\Users\Example\Documents")),
                one_drive: Ok(Vec::new()),
                profile: Ok(PathBuf::from(r"C:\Users\Example")),
                present: BTreeSet::new(),
                canonical: BTreeMap::new(),
            }
        }

        fn add(&mut self, path: impl Into<PathBuf>, canonical: impl Into<PathBuf>) {
            let path = path.into();
            self.present.insert(physical_path_key(&path));
            self.canonical
                .insert(physical_path_key(&path), canonical.into());
        }
    }

    impl DiscoveryEnvironment for FakeEnvironment {
        fn process_command_line_args(&self, _pid: u32) -> Result<Vec<OsString>, String> {
            self.args.clone()
        }
        fn known_documents_dir(&self) -> Result<PathBuf, String> {
            self.documents.clone()
        }
        fn one_drive_roots(&self) -> Result<Vec<(String, PathBuf)>, String> {
            self.one_drive.clone()
        }
        fn user_profile_dir(&self) -> Result<PathBuf, String> {
            self.profile.clone()
        }
        fn contains_dlc_load(&self, dir: &Path) -> bool {
            self.present.contains(&physical_path_key(dir))
        }
        fn canonicalize_dir(&self, dir: &Path) -> Result<PathBuf, String> {
            self.canonical
                .get(&physical_path_key(dir))
                .cloned()
                .ok_or_else(|| "not canonicalizable".to_string())
        }
    }

    fn game_dir(documents: &str) -> PathBuf {
        append_game_dir(PathBuf::from(documents))
    }

    #[test]
    fn parses_separate_and_equals_userdir_and_rejects_ambiguous_forms() {
        assert_eq!(
            parse_userdir_args(&[
                OsString::from("hoi4.exe"),
                OsString::from("-USERDIR"),
                OsString::from(r"D:\Pdx"),
            ])
            .unwrap(),
            Some(PathBuf::from(r"D:\Pdx"))
        );
        assert_eq!(
            parse_userdir_args(&[
                OsString::from("hoi4.exe"),
                OsString::from(r"-userdir=D:\Pdx")
            ])
            .unwrap(),
            Some(PathBuf::from(r"D:\Pdx"))
        );
        assert!(parse_userdir_args(&[
            OsString::from("hoi4.exe"),
            OsString::from(r"-userdir=D:\A"),
            OsString::from(r"-userdir=D:\B"),
        ])
        .unwrap_err()
        .contains("repeats"));
        assert!(
            parse_userdir_args(&[OsString::from("hoi4.exe"), OsString::from("-userdir")]).is_err()
        );
        assert!(parse_userdir_args(&[
            OsString::from("hoi4.exe"),
            OsString::from("-userdir"),
            OsString::from("relative/path"),
        ])
        .unwrap_err()
        .contains("relative"));
    }

    #[test]
    fn known_folder_and_runtime_onedrive_candidates_are_all_considered() {
        let mut env = FakeEnvironment::standard();
        env.one_drive = Ok(vec![
            ("consumer".into(), PathBuf::from(r"D:\OneDrive")),
            ("business1".into(), PathBuf::from(r"E:\Corp")),
        ]);
        let expected = game_dir(r"E:\Corp\Documents");
        env.add(
            &expected,
            r"E:\Corp\Documents\Paradox Interactive\Hearts of Iron IV",
        );
        let located =
            locate_user_dir_with(&env, 42, None, |_| CandidateCorroboration::Owned).unwrap();
        assert_eq!(located.path, expected);
        assert_eq!(located.provenance, vec!["onedrive:business1"]);
        assert!(!located.explicit_override);
    }

    #[test]
    fn command_line_userdir_is_a_direct_candidate() {
        let mut env = FakeEnvironment::standard();
        env.args = Ok(vec![
            OsString::from("hoi4.exe"),
            OsString::from(r"-userdir=F:\Custom Pdx"),
        ]);
        env.add(r"F:\Custom Pdx", r"F:\Custom Pdx");
        let located =
            locate_user_dir_with(&env, 7, None, |_| CandidateCorroboration::Owned).unwrap();
        assert_eq!(located.path, PathBuf::from(r"F:\Custom Pdx"));
        assert_eq!(located.provenance, vec!["process_command_line:-userdir"]);
    }

    #[test]
    fn aliases_to_one_physical_directory_are_deduplicated() {
        let mut env = FakeEnvironment::standard();
        env.one_drive = Ok(vec![("consumer".into(), PathBuf::from(r"D:\OneDrive"))]);
        let known = game_dir(r"C:\Users\Example\Documents");
        let cloud = game_dir(r"D:\OneDrive\Documents");
        let physical =
            PathBuf::from(r"D:\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV");
        env.add(&known, &physical);
        env.add(&cloud, &physical);
        let located =
            locate_user_dir_with(&env, 1, None, |_| CandidateCorroboration::Owned).unwrap();
        assert_eq!(located.path, physical);
        assert_eq!(
            located.provenance,
            vec![
                "known_folder:FOLDERID_Documents",
                "legacy:USERPROFILE/Documents",
                "onedrive:consumer",
            ]
        );
    }

    #[test]
    fn multiple_corroborated_physical_directories_fail_deterministically() {
        let mut env = FakeEnvironment::standard();
        env.one_drive = Ok(vec![("consumer".into(), PathBuf::from(r"D:\OneDrive"))]);
        let a = game_dir(r"C:\Users\Example\Documents");
        let b = game_dir(r"D:\OneDrive\Documents");
        env.add(&a, &a);
        env.add(&b, &b);
        let error =
            locate_user_dir_with(&env, 99, None, |_| CandidateCorroboration::Owned).unwrap_err();
        assert!(error.contains("AMBIGUOUS"), "{error}");
        assert!(error.find("C:\\Users").unwrap() < error.find("D:\\OneDrive").unwrap());
    }

    #[test]
    fn corroboration_selects_the_only_pid_owned_candidate() {
        let mut env = FakeEnvironment::standard();
        env.one_drive = Ok(vec![("consumer".into(), PathBuf::from(r"D:\OneDrive"))]);
        let stale = game_dir(r"C:\Users\Example\Documents");
        let live = game_dir(r"D:\OneDrive\Documents");
        env.add(&stale, &stale);
        env.add(&live, &live);
        let located = locate_user_dir_with(&env, 12, None, |path| {
            if physical_path_key(path) == physical_path_key(&live) {
                CandidateCorroboration::Owned
            } else {
                CandidateCorroboration::Rejected("pid holds no log below this directory".into())
            }
        })
        .unwrap();
        assert_eq!(located.path, live);
    }

    #[test]
    fn indeterminate_candidate_blocks_a_false_singleton() {
        let mut env = FakeEnvironment::standard();
        env.one_drive = Ok(vec![("consumer".into(), PathBuf::from(r"D:\OneDrive"))]);
        let first = game_dir(r"C:\Users\Example\Documents");
        let second = game_dir(r"D:\OneDrive\Documents");
        env.add(&first, &first);
        env.add(&second, &second);
        let error = locate_user_dir_with(&env, 12, None, |path| {
            if physical_path_key(path) == physical_path_key(&first) {
                CandidateCorroboration::Indeterminate("Restart Manager denied".into())
            } else {
                CandidateCorroboration::Owned
            }
        })
        .unwrap_err();
        assert!(error.contains("indeterminate"), "{error}");
    }

    #[test]
    fn explicit_override_is_still_required_to_prove_out() {
        let mut env = FakeEnvironment::standard();
        let override_dir = PathBuf::from(r"G:\Pinned");
        env.add(&override_dir, &override_dir);
        let error = locate_user_dir_with(&env, 3, Some(&override_dir), |_| {
            CandidateCorroboration::Rejected("not held by pid".into())
        })
        .unwrap_err();
        assert!(error.contains("not held by pid"));

        let located = locate_user_dir_with(&env, 3, Some(&override_dir), |_| {
            CandidateCorroboration::Owned
        })
        .unwrap();
        assert_eq!(located.path, override_dir);
        assert!(located.explicit_override);
    }

    #[test]
    fn explicit_override_cannot_hide_a_second_pid_owned_directory() {
        let mut env = FakeEnvironment::standard();
        let discovered = game_dir(r"C:\Users\Example\Documents");
        let pinned = PathBuf::from(r"G:\Pinned");
        env.add(&discovered, &discovered);
        env.add(&pinned, &pinned);
        let error = locate_user_dir_with(&env, 3, Some(&pinned), |_| CandidateCorroboration::Owned)
            .unwrap_err();
        assert!(error.contains("AMBIGUOUS"), "{error}");
    }

    #[test]
    fn unavailable_required_discovery_source_fails_closed() {
        let mut env = FakeEnvironment::standard();
        env.args = Err("access denied".into());
        let error =
            locate_user_dir_with(&env, 4, None, |_| CandidateCorroboration::Owned).unwrap_err();
        assert!(error.contains("command line unavailable"), "{error}");
        env.add(r"G:\Pinned", r"G:\Pinned");
        let error = locate_user_dir_with(&env, 4, Some(Path::new(r"G:\Pinned")), |_| {
            CandidateCorroboration::Owned
        })
        .unwrap_err();
        assert!(
            error.contains("command line unavailable"),
            "support override must not bypass incomplete discovery: {error}"
        );

        let mut env = FakeEnvironment::standard();
        env.one_drive = Err("registry denied".into());
        let error =
            locate_user_dir_with(&env, 4, None, |_| CandidateCorroboration::Owned).unwrap_err();
        assert!(error.contains("OneDrive roots unavailable"), "{error}");
    }
}
