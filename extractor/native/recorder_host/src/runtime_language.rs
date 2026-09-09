//! A bounded observation of the active language in one exact game incarnation.
//! It establishes the current language only; graphics settings and launch-time
//! settings are not inferred from this read.

use std::cell::Cell;
use std::ffi::c_void;
use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use theatre_selector_live_lane::engine_localisation_provenance::{
    read_engine_active_language, TargetImageMemory,
};

pub const SCHEMA: &str = "hoi4.runtime_language_observation.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLanguageObservation {
    pub schema: String,
    pub pid: u32,
    pub process_creation_filetime: u64,
    pub executable_sha256: String,
    pub observed_unix_ms: u64,
    pub game_hour: u32,
    pub language: String,
    pub double_stable: bool,
}

impl RuntimeLanguageObservation {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA
            || self.pid == 0
            || self.process_creation_filetime == 0
            || self.executable_sha256 != crate::native_design_catalog::PROGRAM_SHA256
            || !self.double_stable
            || self.language.trim().is_empty()
            || self.language.len() > 64
            || self
                .language
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
        {
            return Err(
                "runtime language observation has invalid source identity or language".into(),
            );
        }
        let creation_unix_ms = self
            .process_creation_filetime
            .checked_sub(116_444_736_000_000_000)
            .ok_or("runtime language process predates the Unix epoch")?
            / 10_000;
        if self.observed_unix_ms < creation_unix_ms {
            return Err("runtime language observation predates the process".into());
        }
        Ok(())
    }
}

fn observe_language_in_hour(
    memory: &dyn TargetImageMemory,
    image_base: u64,
) -> Result<(u32, String), String> {
    let read = |address, length| memory.read(address, length);
    let before = crate::native_design_catalog::observe_date(&read, image_base)
        .map_err(|error| format!("runtime language opening hour: {error:?}"))?;
    let language = read_engine_active_language(memory, image_base)?;
    let after = crate::native_design_catalog::observe_date(&read, image_base)
        .map_err(|error| format!("runtime language closing hour: {error:?}"))?;
    if before != after {
        return Err("runtime language read crossed a game hour".into());
    }
    let game_hour = before
        .checked_sub(crate::native_design_catalog::DATE_EPOCH)
        .ok_or("runtime language engine date predates the game epoch")?;
    Ok((game_hour, language))
}

type Handle = *mut c_void;

#[repr(C)]
struct ModuleEntry {
    size: u32,
    module_id: u32,
    process_id: u32,
    global_usage: u32,
    process_usage: u32,
    base: *mut u8,
    image_size: u32,
    module: Handle,
    name: [u16; 256],
    path: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetProcessId(process: Handle) -> u32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        path: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn ReadProcessMemory(
        process: Handle,
        address: *const c_void,
        bytes: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Module32FirstW(snapshot: Handle, entry: *mut ModuleEntry) -> i32;
    fn Module32NextW(snapshot: Handle, entry: *mut ModuleEntry) -> i32;
}

struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn require_incarnation(process: Handle, pid: u32, creation: u64) -> Result<PathBuf, String> {
    let (mut observed, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
    let mut path = vec![0u16; 32_768];
    let mut length = path.len() as u32;
    unsafe {
        if GetProcessId(process) != pid
            || GetProcessTimes(process, &mut observed, &mut exit, &mut kernel, &mut user) == 0
            || observed != creation
            || exit != 0
            || QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) == 0
        {
            return Err("runtime language process incarnation is unavailable or changed".into());
        }
    }
    fs::canonicalize(PathBuf::from(String::from_utf16_lossy(
        &path[..length as usize],
    )))
    .map_err(|error| format!("runtime language executable path: {error}"))
}

fn image_base(pid: u32, image: &Path) -> Result<u64, String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(0x8 | 0x10, pid) };
    if snapshot.is_null() || snapshot == -1isize as Handle {
        return Err("runtime language module snapshot unavailable".into());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry: ModuleEntry = unsafe { std::mem::zeroed() };
    entry.size = std::mem::size_of::<ModuleEntry>() as u32;
    let mut present = unsafe { Module32FirstW(snapshot.0, &mut entry) };
    for _ in 0..512 {
        if present == 0 {
            break;
        }
        let length = entry
            .path
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.path.len());
        let path = PathBuf::from(String::from_utf16_lossy(&entry.path[..length]));
        if entry.process_id == pid && fs::canonicalize(&path).ok().as_deref() == Some(image) {
            let base = entry.base as u64;
            if base == 0 || entry.image_size <= theatre_selector_live_lane::engine_localisation_provenance::LOCALISATION_ROOT_RVA as u32 {
                return Err("runtime language image does not contain its proven root".into());
            }
            return Ok(base);
        }
        present = unsafe { Module32NextW(snapshot.0, &mut entry) };
    }
    Err("runtime language exact executable module was not observed".into())
}

struct ProcessMemory {
    process: Handle,
    bytes_remaining: Cell<usize>,
}

impl TargetImageMemory for ProcessMemory {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let remaining = self
            .bytes_remaining
            .get()
            .checked_sub(length)
            .ok_or("runtime language read exceeded its 1024-byte budget")?;
        if address == 0 || length == 0 || address.checked_add(length as u64).is_none() {
            return Err("runtime language read has an invalid range".into());
        }
        self.bytes_remaining.set(remaining);
        let mut bytes = vec![0; length];
        let mut read = 0;
        let ok = unsafe {
            ReadProcessMemory(
                self.process,
                address as *const c_void,
                bytes.as_mut_ptr().cast(),
                length,
                &mut read,
            )
        };
        if ok == 0 || read != length {
            return Err("runtime language memory was unreadable or partially read".into());
        }
        Ok(bytes)
    }
}

pub fn observe(
    pid: u32,
    creation: u64,
    expected_image: &Path,
    expected_sha256: &str,
) -> Result<RuntimeLanguageObservation, String> {
    if expected_sha256 != crate::native_design_catalog::PROGRAM_SHA256 || pid == 0 || creation == 0
    {
        return Err(
            "runtime language requires the proven executable and exact process incarnation".into(),
        );
    }
    let image = fs::canonicalize(expected_image)
        .map_err(|error| format!("runtime language executable identity: {error}"))?;
    // Keep the executable bytes fixed through hashing and process observation.
    let _image_lease = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&image)
        .map_err(|error| format!("runtime language executable lease: {error}"))?;
    if crate::modset::hash_file_streaming_stable(&image)?.1 != expected_sha256 {
        return Err("runtime language executable hash differs from the admitted build".into());
    }
    let process = unsafe { OpenProcess(0x1000 | 0x10, 0, pid) };
    if process.is_null() {
        return Err("runtime language process is unavailable".into());
    }
    let process = OwnedHandle(process);
    if require_incarnation(process.0, pid, creation)? != image {
        return Err("runtime language process executable differs from the admitted image".into());
    }
    let base = image_base(pid, &image)?;
    let memory = ProcessMemory {
        process: process.0,
        bytes_remaining: Cell::new(1024),
    };
    let (game_hour, language) = observe_language_in_hour(&memory, base)?;
    if require_incarnation(process.0, pid, creation)? != image || image_base(pid, &image)? != base {
        return Err("runtime language process or module changed across the observation".into());
    }
    let observation = RuntimeLanguageObservation {
        schema: SCHEMA.into(),
        pid,
        process_creation_filetime: creation,
        executable_sha256: expected_sha256.into(),
        observed_unix_ms: u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "runtime language observation clock predates Unix epoch")?
                .as_millis(),
        )
        .map_err(|_| "runtime language observation clock overflows")?,
        game_hour,
        language,
        double_stable: true,
    };
    observation.validate()?;
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const BASE: u64 = 0x140000000;
    const STATE: u64 = 0x20000000000;
    const ROOT: u64 = 0x20000010000;
    const STRING: u64 = 0x20000020000;

    struct Memory {
        regions: BTreeMap<u64, Vec<u8>>,
        date_reads: Cell<usize>,
        cross_hour: bool,
    }

    impl TargetImageMemory for Memory {
        fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
            if address == STATE + 0x468 {
                let count = self.date_reads.get();
                self.date_reads.set(count + 1);
                // Each existing observe_date seals its readset. Change between
                // those complete observations, not within either readset.
                if self.cross_hour && count >= 2 {
                    return Ok(60_830_442u32.to_le_bytes().to_vec());
                }
            }
            let (start, bytes) = self
                .regions
                .range(..=address)
                .next_back()
                .ok_or("unmapped fixture address")?;
            let offset = usize::try_from(address - start).map_err(|_| "fixture offset")?;
            bytes
                .get(offset..offset.checked_add(length).ok_or("fixture overflow")?)
                .map(<[u8]>::to_vec)
                .ok_or("unmapped fixture range".into())
        }
    }

    fn memory(language: &str) -> Memory {
        let mut header = vec![0; 32];
        header[16..24].copy_from_slice(&(language.len() as u64).to_le_bytes());
        let mut regions = BTreeMap::from([
            (BASE + 0x3316e90, STATE.to_le_bytes().to_vec()),
            (STATE + 0x468, 60_830_441u32.to_le_bytes().to_vec()),
            (BASE + theatre_selector_live_lane::engine_localisation_provenance::LOCALISATION_ROOT_RVA,
                ROOT.to_le_bytes().to_vec()),
            (ROOT, STRING.to_le_bytes().to_vec()),
        ]);
        if language.len() <= 15 {
            header[..language.len()].copy_from_slice(language.as_bytes());
            header[24..32].copy_from_slice(&15u64.to_le_bytes());
        } else {
            header[..8].copy_from_slice(&(STRING + 0x1000).to_le_bytes());
            header[24..32].copy_from_slice(&(language.len() as u64).to_le_bytes());
            regions.insert(STRING + 0x1000, language.as_bytes().to_vec());
        }
        regions.insert(STRING, header);
        Memory {
            regions,
            date_reads: Cell::new(0),
            cross_hour: false,
        }
    }

    pub(crate) fn observation() -> RuntimeLanguageObservation {
        RuntimeLanguageObservation {
            schema: SCHEMA.into(),
            pid: 640,
            process_creation_filetime: 134_333_846_092_118_857,
            executable_sha256: crate::native_design_catalog::PROGRAM_SHA256.into(),
            observed_unix_ms: 1_788_911_010_000,
            game_hour: 17_030_442,
            language: "l_english".into(),
            double_stable: true,
        }
    }

    #[test]
    fn current_language_reads_sso_and_heap_with_a_stable_hour_without_tables_or_settings() {
        for language in ["l_english", "l_long_fixture_language"] {
            let memory = memory(language);
            assert_eq!(
                observe_language_in_hour(&memory, BASE).unwrap(),
                (17_030_442, language.into())
            );
            assert_eq!(memory.date_reads.get(), 4);
        }
    }

    #[test]
    fn current_language_refuses_crossed_hours_unreadable_and_unbounded_strings() {
        let mut crossed = memory("l_english");
        crossed.cross_hour = true;
        assert!(observe_language_in_hour(&crossed, BASE)
            .unwrap_err()
            .contains("crossed a game hour"));
        let mut unreadable = memory("l_english");
        unreadable.regions.remove(&STRING);
        assert!(observe_language_in_hour(&unreadable, BASE).is_err());
        assert!(observe_language_in_hour(&memory(&"x".repeat(65)), BASE).is_err());
        assert!(observe_language_in_hour(&memory("l_en/guess"), BASE).is_err());
    }

    #[test]
    fn proof_requires_the_known_build_incarnation_stability_and_current_time() {
        assert_eq!(
            crate::native_design_catalog::PROGRAM_SHA256,
            portable_engine_evidence::runtime_language::EXECUTABLE_SHA256
        );
        assert_eq!(
            theatre_selector_live_lane::engine_localisation_provenance::LOCALISATION_ROOT_RVA,
            u64::from(portable_engine_evidence::runtime_language::LOCALISATION_ROOT_RVA)
        );
        let valid = observation();
        valid.validate().unwrap();
        let encoded = serde_json::to_value(&valid).unwrap();
        for (field, value) in [
            ("schema", serde_json::json!("other")),
            ("pid", serde_json::json!(0)),
            ("process_creation_filetime", serde_json::json!(0)),
            ("executable_sha256", serde_json::json!("ab".repeat(32))),
            ("double_stable", serde_json::json!(false)),
            ("observed_unix_ms", serde_json::json!(1)),
            ("language", serde_json::json!("")),
        ] {
            let mut changed = encoded.clone();
            changed[field] = value;
            let changed: RuntimeLanguageObservation = serde_json::from_value(changed).unwrap();
            assert!(changed.validate().is_err(), "accepted invalid {field}");
        }
        let mut extra = encoded;
        extra["launch_proven"] = serde_json::json!(true);
        assert!(serde_json::from_value::<RuntimeLanguageObservation>(extra).is_err());
    }

    #[test]
    fn invalid_identity_ranges_and_exhausted_budget_refuse_before_native_access() {
        let memory = ProcessMemory {
            process: std::ptr::null_mut(),
            bytes_remaining: Cell::new(1024),
        };
        assert!(memory.read(1, 1025).unwrap_err().contains("budget"));
        assert!(memory.read(0, 8).unwrap_err().contains("invalid range"));
        assert!(memory
            .read(u64::MAX, 8)
            .unwrap_err()
            .contains("invalid range"));
        assert_eq!(memory.bytes_remaining.get(), 1024);
        for (pid, creation, hash) in [
            (0, 1, crate::native_design_catalog::PROGRAM_SHA256),
            (1, 0, crate::native_design_catalog::PROGRAM_SHA256),
            (1, 1, "unknown build"),
        ] {
            assert!(observe(pid, creation, Path::new("must-not-open"), hash)
                .unwrap_err()
                .contains("proven executable and exact process incarnation"));
        }
    }
}
