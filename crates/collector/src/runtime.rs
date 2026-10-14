//! Loader worker, spool thread, and restore/drain/self-unload lifecycle.

use crate::{
    acquisition_jobs as acquisition, decode,
    history::PreparedCampaigns,
    hook::{Hook, Threads},
    sampling::{self, ACTIVE, SHARED, Shared},
    spool_phases::Phase,
    spooling::{Distribution, Processor, WallTotals},
};
use engine::{
    PINNED_BUILD, build::hash_executable, cgame_state, country_memory::SeedStatus,
    memory::LiveMemory, read,
};
use replay::{codec::Decoder, identity::SessionStart};

use serde::Deserialize;
#[cfg(test)]
use serde_json::{Value, json};
use std::{
    ffi::{OsString, c_void},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HMODULE, WAIT_OBJECT_0},
    Storage::FileSystem::{
        CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileSizeEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        MoveFileExW, OPEN_EXISTING,
    },
    System::{
        LibraryLoader::{FreeLibraryAndExitThread, GetModuleFileNameW, GetModuleHandleW},
        Performance::QueryPerformanceFrequency,
        Threading::*,
    },
};

// Both production refusal paths and the regression enter this boundary.
macro_rules! refuse_detach {
    ($reason:expr, $retain:expr) => {{
        let reason = $reason;
        crate::land_events::stop_observation();
        ($retain)(reason);
        return Err(reason);
    }};
}

#[cfg(test)]
pub(crate) fn refuse_land_detach_for_test(retain: impl FnOnce()) -> Result<(), &'static str> {
    refuse_detach!("synthetic land detach refused", |_reason| retain());
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default = "observer_default")]
    operation_outcomes: bool,
    #[serde(default = "observer_default")]
    politics_capture: bool,
    session_id: u64,
    output_dir: PathBuf,
    replay_dir: PathBuf,
    session_start: Vec<u8>,
    ready_event: String,
    detach_event: String,
    done_event: String,
    cancel_file: PathBuf,
    #[serde(default)]
    content_order_note: Option<String>,
    #[serde(default)]
    expected_mounts: Vec<String>,
    #[serde(default)]
    mod_descriptors: Vec<String>,
    #[serde(default)]
    alloc_fault: Option<AllocFault>,
    #[serde(default)]
    worker_sampler: Option<String>,
    #[serde(default = "game_score_enabled")]
    game_score: bool,
}
fn game_score_enabled() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllocFault {
    pub(crate) phase: String,
    pub(crate) ordinal: u64,
    #[serde(default)]
    pub(crate) after_hours: u64,
}
fn error_reason(error: io::Error, reason: &'static str) -> &'static str {
    crate::allocation::observe(&error);
    reason
}
pub(crate) fn configure_alloc_fault(fault: Option<&AllocFault>) -> Result<(), &'static str> {
    let Some(fault) = fault else {
        return Ok(());
    };
    #[cfg(feature = "alloc-fault-injection")]
    {
        let phase = crate::allocation::Phase::parse(&fault.phase)
            .ok_or("alloc_fault names an unsupported phase")?;
        if fault.ordinal == 0 {
            return Err("alloc_fault ordinal must be nonzero");
        }
        crate::allocation::inject_after(phase, fault.ordinal, fault.after_hours);
        Ok(())
    }
    #[cfg(not(feature = "alloc-fault-injection"))]
    {
        let _ = (&fault.phase, fault.ordinal, fault.after_hours);
        Err("alloc_fault requires collector feature alloc-fault-injection")
    }
}
fn observer_default() -> bool {
    true
}

#[cfg(not(feature = "worker_sampler"))]
fn refuse_worker_sampler(config: Option<&str>) -> Result<(), &'static str> {
    if config.is_some_and(|value| !value.is_empty()) {
        Err("worker_sampler requested but collector built without the worker_sampler feature")
    } else {
        Ok(())
    }
}

struct Event(HANDLE);
impl Event {
    fn create(name: &str) -> Result<Self, &'static str> {
        if name.is_empty() || name.contains('\0') {
            return Err("event name is empty or contains NUL");
        }
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, wide.as_ptr()) };
        if handle.is_null() {
            crate::allocation::native_error(&io::Error::last_os_error());
            Err("CreateEventW failed")
        } else {
            Ok(Self(handle))
        }
    }
    fn set(&self) {
        unsafe {
            SetEvent(self.0);
        }
    }
    fn signaled(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn module_path(module: HMODULE) -> Result<PathBuf, &'static str> {
    let mut buffer = vec![0u16; 32768];
    let len =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len == buffer.len() {
        return Err("GetModuleFileNameW failed or truncated");
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len])))
}
/// A failed installation's message, naming the family, kept for the
/// attachment: status and the terminal error borrow it until unload. Built
/// once, on the runtime thread, only when installation fails.
fn installation_message(message: String) -> &'static str {
    Box::leak(message.into_boxed_str())
}

pub unsafe extern "system" fn worker(module: *mut c_void) -> u32 {
    let _bootstrap = crate::allocation::enter(crate::allocation::Phase::Bootstrap);
    crate::allocation::install_hooks();
    let witness = bootstrap_witness(module);
    let result = match &witness {
        Ok((file, session, process_start)) => {
            crate::allocation::prepare(file, *session, *process_start);
            run(module, *session, *process_start)
        }
        Err(error) => Err(*error),
    };
    if let Err(error) = result
        && !crate::allocation::failed()
        && let Ok(path) = module_path(module)
    {
        let _ = fs::write(path.with_file_name("collector.error.txt"), error);
    }
    finish_worker(&SHARED, witness, |witness| {
        crate::allocation::clear_handle();
        engine::allocation::set_hooks(None, None);
        engine::allocation::set_stop_hook(None);
        drop(witness);
        unsafe {
            FreeLibraryAndExitThread(module, 0);
        }
    })
}

fn finish_worker<T>(shared: &AtomicPtr<Shared>, witness: T, unload: impl FnOnce(T) -> u32) -> u32 {
    if !shared.load(Ordering::Acquire).is_null() {
        std::mem::forget(witness);
        1
    } else {
        unload(witness)
    }
}
fn bootstrap_witness(module: HMODULE) -> Result<(File, u64, u64), &'static str> {
    let mut path = [0u16; 32768];
    let length =
        unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) } as usize;
    if length == 0 || length == path.len() {
        return Err("collector bootstrap module path unreadable");
    }
    bootstrap_witness_path(&mut path, length)
}
fn bootstrap_witness_path(
    path: &mut [u16; 32768],
    length: usize,
) -> Result<(File, u64, u64), &'static str> {
    let bin_separator = path[..length]
        .iter()
        .rposition(|unit| *unit == b'\\' as u16 || *unit == b'/' as u16)
        .and_then(|last| {
            path[..last]
                .iter()
                .rposition(|unit| *unit == b'\\' as u16 || *unit == b'/' as u16)
        })
        .ok_or("collector bootstrap has no session directory")?;
    let filename = replay::witness::WITNESS_FILE_NAME.as_bytes();
    let end = bin_separator + 1 + filename.len();
    if end >= path.len() {
        return Err("collector bootstrap witness path truncated");
    }
    for (output, byte) in path[bin_separator + 1..end].iter_mut().zip(filename) {
        *output = u16::from(*byte);
    }
    path[end] = 0;
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err("collector bootstrap witness open failed");
    }
    let mut file = unsafe { File::from_raw_handle(handle) };
    let mut length = 0_i64;
    if unsafe { GetFileSizeEx(file.as_raw_handle(), &mut length) } == 0 {
        return Err("collector bootstrap witness size unreadable");
    }
    if length != replay::witness::WITNESS_BYTES as i64 {
        return Err("collector bootstrap witness length invalid");
    }
    let mut bytes = [0u8; replay::witness::WITNESS_BYTES];
    let mut read = 0;
    while read < bytes.len() {
        match file.read(&mut bytes[read..]) {
            Ok(0) => return Err("collector bootstrap witness truncated"),
            Ok(count) => read += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err("collector bootstrap witness read failed"),
        }
    }
    file.seek(io::SeekFrom::Start(0))
        .map_err(|_| "collector bootstrap witness seek failed")?;
    let (session, process_start) = replay::witness::TerminalWitness::decode_prepared(&bytes)
        .map_err(|_| "collector bootstrap witness identity invalid")?;
    Ok((file, session, process_start))
}

#[cfg(test)]
mod bootstrap_witness_tests {
    use super::*;
    use std::io::Write;

    fn read(module: &[u16]) -> Result<(File, u64, u64), &'static str> {
        let mut path = [0_u16; 32768];
        path[..module.len()].copy_from_slice(module);
        bootstrap_witness_path(&mut path, module.len() - 1)
    }

    #[test]
    fn bootstrap_refuses_prepared_witness_with_trailing_byte() {
        let directory = std::env::temp_dir().join(format!(
            "hoi4-bootstrap-witness-length-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let witness = directory.join(replay::witness::WITNESS_FILE_NAME);
        fs::write(
            &witness,
            replay::witness::TerminalWitness::prepared(1, 1000),
        )
        .unwrap();
        let module: Vec<_> = directory
            .join("bin/collector.dll")
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let (file, session, start) = read(&module).unwrap();
        assert_eq!((session, start), (1, 1000));
        drop(file);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&witness)
            .unwrap()
            .write_all(&[0])
            .unwrap();
        assert_eq!(
            read(&module).err(),
            Some("collector bootstrap witness length invalid")
        );
        let _ = fs::remove_dir_all(&directory);
    }
}
fn write_status(path: &Path, status: &impl serde::Serialize) -> io::Result<()> {
    write_status_bytes(path, &crate::status::serialize(status)?)
}
pub(crate) fn write_status_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes)?;
    replace_status(&temp, path)
}

const STATUS_RENAME_RETRY_BUDGET: Duration = Duration::from_millis(30);

fn status_rename_retry_delay(error: &io::Error, elapsed: Duration) -> Option<Duration> {
    if !matches!(error.raw_os_error(), Some(5 | 32)) {
        return None;
    }
    STATUS_RENAME_RETRY_BUDGET
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| remaining.min(Duration::from_millis(1)))
}

/// Atomically replace `target` with `source` using POSIX rename semantics:
/// the new name takes effect even while a reader holds the old file open with
/// delete sharing, which a classic `MoveFileExW` replace refuses with
/// ERROR_ACCESS_DENIED. `Ok(false)`: the volume does not support it.
fn posix_replace(source: &[u16], target: &[u16]) -> io::Result<bool> {
    let handle = unsafe {
        CreateFileW(
            source.as_ptr(),
            DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_handle(handle) };
    replay::publication::posix_replace(&file, target)
}

/// Set once a status directory reports that POSIX rename is unsupported
/// (network shares, FAT): later writes go straight to the classic replace.
static POSIX_RENAME_UNSUPPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// One replace attempt: POSIX rename while supported, else the classic
/// `MoveFileExW` replace, which any open reader blocks.
fn replace_once(source: &[u16], target: &[u16]) -> io::Result<()> {
    if !POSIX_RENAME_UNSUPPORTED.load(Ordering::Relaxed) {
        match posix_replace(source, target) {
            Ok(true) => return Ok(()),
            Ok(false) => POSIX_RENAME_UNSUPPORTED.store(true, Ordering::Relaxed),
            Err(error) => return Err(error),
        }
    }
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn replace_status(temp: &Path, path: &Path) -> io::Result<()> {
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let source = wide(temp);
    let target = wide(path);
    let started = Instant::now();
    loop {
        let error = match replace_once(&source, &target) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let Some(delay) = status_rename_retry_delay(&error, started.elapsed()) else {
            return Err(error);
        };
        std::thread::sleep(delay);
        if started.elapsed() >= STATUS_RENAME_RETRY_BUDGET {
            return Err(error);
        }
    }
}

struct WorkerWork {
    buffer_service: Distribution,
    recycle_capacities_wait: WallTotals,
    sizing_capacities_hold: WallTotals,
    idle_flush_calls: u64,
    idle_flush_ns: u64,
    idle_flush_max_ns: u64,
    status_update: Distribution,
    queue_peak_observed: usize,
    started_qpc: i64,
    started_cpu_ns: Result<u64, io::ErrorKind>,
}

struct CpuWorkStatus {
    cpu_ns: Result<u64, io::ErrorKind>,
    hours: u64,
}

impl crate::status::Fields for CpuWorkStatus {
    fn len(&self) -> usize {
        if self.cpu_ns.is_ok() { 2 } else { 1 }
    }

    fn serialize<M: serde::ser::SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        match self.cpu_ns {
            Ok(cpu_ns) => {
                map.serialize_entry("cpu_total_ns", &cpu_ns)?;
                map.serialize_entry(
                    "cpu_ns_per_recorded_hour",
                    &if self.hours == 0 {
                        None
                    } else {
                        Some(cpu_ns as f64 / self.hours as f64)
                    },
                )
            }
            Err(_) => map.serialize_entry("cpu_error", "GetThreadTimes failed"),
        }
    }
}

impl WorkerWork {
    fn new() -> Self {
        Self {
            buffer_service: Distribution::default(),
            recycle_capacities_wait: WallTotals::default(),
            sizing_capacities_hold: WallTotals::default(),
            idle_flush_calls: 0,
            idle_flush_ns: 0,
            idle_flush_max_ns: 0,
            status_update: Distribution::default(),
            queue_peak_observed: 0,
            started_qpc: sampling::qpc(),
            started_cpu_ns: thread_cpu_ns().map_err(|error| error.kind()),
        }
    }

    fn recycle_capacities<'a>(
        &mut self,
        sizing: &'a SizingShared,
        frequency: u64,
    ) -> std::sync::MutexGuard<'a, BufferCapacities> {
        let started = sampling::qpc();
        let capacities = lock(&sizing.capacities);
        self.recycle_capacities_wait
            .record_ticks(sampling::qpc() - started, frequency);
        capacities
    }

    /// Once per status update. CPU time includes all work on the spool thread;
    /// buffer_service contains the conversion/encoding/write phases, not their sum.
    fn status(
        &self,
        hours: u64,
        frequency: u64,
    ) -> io::Result<crate::status::Object<impl crate::status::Fields>> {
        let buffer_service = self.buffer_service.status()?;
        let status_update = self.status_update.status()?;
        let elapsed_ns = ((sampling::qpc() - self.started_qpc).max(0) as u128 * 1_000_000_000
            / u128::from(frequency)) as u64;
        let cpu_ns = match self.started_cpu_ns {
            Ok(start) => thread_cpu_ns()
                .map(|now| now.saturating_sub(start))
                .map_err(|error| error.kind()),
            Err(error) => Err(error),
        };
        Ok(crate::status::status!({
            "buffer_service": buffer_service,
            "recycle_capacities_wait": self.recycle_capacities_wait.status(),
            "recycle_capacities_wait_scope": "spool-thread wait for the capture-capacity mutex while it holds the returning buffer; sizing holds that mutex for raises and idle-buffer growth, and re-reads live table and input hints there only when a concurrent raise changed the observed capacities",
            "sizing_capacities_hold": self.sizing_capacities_hold.status(),
            "sizing_capacities_hold_scope": "capacity-mutex hold wall for sizing raises, idle-buffer and combat-hint growth, live-hint capacity copies and publication, and live-hint re-reads after a concurrent raise; excludes the initial live-hint reads, lock acquisition and diagnostic hand-over; runs inline on the spool thread when the sizing thread cannot start",
            "idle_flush": {
                "calls": self.idle_flush_calls,
                "total_ns": self.idle_flush_ns,
                "max_ns": self.idle_flush_max_ns,
                "mean_ns": self.idle_flush_ns.checked_div(self.idle_flush_calls).unwrap_or(0),
            },
            "idle_flush_scope": "spool-thread wall in Processor::flush after draining ready buffers; includes periodic durable synchronization; separate from buffer_service and status_update",
            "status_update": status_update,
            "queue_peak_observed": self.queue_peak_observed,
            "elapsed_ns": elapsed_ns,
        }).join(crate::status::Object(CpuWorkStatus { cpu_ns, hours })))
    }
}

fn thread_cpu_ns() -> io::Result<u64> {
    let mut creation = unsafe { std::mem::zeroed() };
    let mut exit = unsafe { std::mem::zeroed() };
    let mut kernel = unsafe { std::mem::zeroed() };
    let mut user = unsafe { std::mem::zeroed() };
    if unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let ticks = |time: windows_sys::Win32::Foundation::FILETIME| {
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    };
    Ok(ticks(kernel)
        .saturating_add(ticks(user))
        .saturating_mul(100))
}

fn run(
    module: HMODULE,
    prepared_session: u64,
    prepared_process_start: u64,
) -> Result<(), &'static str> {
    // Calibrate hook timing over the whole attachment, before any hook exists.
    crate::hook_timing::start();
    crate::thread_cpu::register(crate::thread_cpu::Role::Runtime);
    // Arm early-exit tracking before creating any acquisition, decode, spool,
    // status, helper or sizing worker; sampler startup itself comes later.
    #[cfg(feature = "worker_sampler")]
    crate::worker_sampler::prepare();
    let dll_path = module_path(module)?;
    let config: Config = serde_json::from_slice(
        &fs::read(dll_path.with_file_name("collector.json"))
            .map_err(|e| error_reason(e, "collector config read failed"))?,
    )
    .map_err(|_| "collector JSON config invalid")?;
    // A diagnostic request must not silently become an ordinary capture. Refuse
    // before starting any acquisition, decode, spool or helper worker.
    #[cfg(not(feature = "worker_sampler"))]
    refuse_worker_sampler(config.worker_sampler.as_deref())?;
    let mut input = Decoder::new(&config.session_start);
    let session = SessionStart::decode(&mut input)
        .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    input
        .finish()
        .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    if session.id != config.session_id
        || session.id != prepared_session
        || session.process_id != unsafe { GetCurrentProcessId() }
    {
        return Err("collector config names another session or process");
    }
    let _allocation_details = crate::allocation::prepare_details(&config.output_dir);
    let mut creation = unsafe { std::mem::zeroed() };
    let mut exit = unsafe { std::mem::zeroed() };
    let mut kernel = unsafe { std::mem::zeroed() };
    let mut user = unsafe { std::mem::zeroed() };
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err("GetProcessTimes failed");
    }
    let process_start =
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    configure_alloc_fault(config.alloc_fault.as_ref())?;

    if session.process_start_time != process_start || prepared_process_start != process_start {
        return Err("collector config process creation time differs");
    }
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || frequency <= 0
        || session.qpc_frequency != frequency as u64
    {
        return Err("collector config QPC frequency differs");
    }
    if session.sample_boundary != sampling::SAMPLE_BOUNDARY {
        return Err("collector sample boundary differs from session");
    }
    let executable_path = module_path(ptr::null_mut())?;
    let exe = hash_executable(
        File::open(&executable_path)
            .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?,
    )
    .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    if !PINNED_BUILD.matches(&exe) {
        return Err("unsupported game build");
    }
    if session.identity.exe_hash != exe.sha256 || session.identity.exe_size != exe.size {
        return Err("collector config executable identity differs from running build");
    }
    // Prepare immutable metadata on this loader worker before publishing hooks.
    engine::executable_tables::initialize_executable(&executable_path)
        .map_err(|error| error_reason(error, "collector executable tables preparation failed"))?;
    let dll = hash_executable(
        File::open(&dll_path).map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?,
    )
    .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    if session.identity.collector_hash != dll.sha256 {
        return Err("collector DLL hash differs from session identity");
    }
    // The recorder creates the session directory before attachment.
    let ready = Event::create(&config.ready_event)?;
    let detach = Event::create(&config.detach_event)?;
    let done = Event::create(&config.done_event)?;
    let status_path = config.output_dir.join("status.json");
    write_status(
        &status_path,
        &crate::status::status!({"state":"starting", "session":session.id, "hours":0}),
    )
    .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    let progress = crate::progress::Progress::open(&config.output_dir);
    progress.step("attach_started", None);
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let memory = unsafe { LiveMemory::new() };
    let rva = cgame_state::HOURLY_UPDATE_RVA;
    let sentinel = cgame_state::HOURLY_UPDATE_SENTINEL;
    if read::<[u8; 16]>(&memory, base + rva) != Some(sentinel) {
        return Err("hourly boundary sentinel differs");
    }
    // The recorder uses this same exclusive handle before every finalization.
    // It protects both existing histories and campaigns with no replay yet.
    let output_lock = reserve_output(&config, &detach)?;
    progress.step("output_reserved", None);
    let history = PreparedCampaigns::load(&config.replay_dir, &session, |campaign| {
        if config.cancel_file.exists() || detach.signaled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        write_status(
            &status_path,
            &crate::status::status!({
                "state":"preparing campaign history", "session":session.id,
                "campaign":campaign, "hours":0,
            }),
        )
    })
    .map_err(|error| error_reason(error, "collector bootstrap preparation failed"))?;
    crate::politics_changes::restore_epoch(history.politics_epoch())?;
    crate::operation_outcomes::restore_sequence(history.completion_sequence())?;
    crate::naval_result_sources::restore_sequence(history.naval_result_sequence())?;
    progress.step("history_loaded", None);
    let event_feed_sequence = history.event_feed_sequence();
    let event_feed_generation = history.event_feed_generation().unwrap_or(1).max(1);
    let event_feed_counts = history.event_feed_counts();
    let [air_loss_sequence, bombing_sequence, nuclear_sequence] = history.event_ledger_sequences();
    crate::air_losses::restore_sequence(air_loss_sequence)?;
    crate::building_bombing::restore_sequence(bombing_sequence)?;
    crate::nuclear_strikes::restore_sequence(nuclear_sequence)?;
    let mut processor = Processor::new(&config.output_dir, history, session.clone())
        .map_err(|error| error_reason(error, "collector bootstrap preparation failed"))?;
    let mut hook = unsafe { Hook::new(base + rva, sampling::tick as *const () as usize, sentinel) }
        .map_err(|_| "hourly hook construction failed")?;
    let mut portrait_hooks = unsafe { crate::portrait_changes::Hooks::new(&memory, base) }?;
    let mut advisor_hooks = unsafe { crate::advisor_changes::Hooks::new(&memory, base) }?;
    let mut relation_hooks = unsafe { crate::relation_changes::Hooks::new(&memory, base) }?;
    let mut history_hooks = unsafe { crate::history_changes::Hooks::new(&memory, base) }?;
    let mut flag_hooks = unsafe { crate::flag_changes::Hooks::new(&memory, base) }?;
    let mut building_hooks = unsafe { crate::building_changes::Hooks::new(&memory, base) }?;
    let mut advisor_pool_hooks = unsafe { crate::advisor_pool_changes::Hooks::new(&memory, base) }?;
    let mut party_leader_hooks = unsafe { crate::party_leader_changes::Hooks::new(&memory, base) }?;
    let mut variable_hooks = unsafe { crate::variable_changes::Hooks::new(&memory, base) }?;
    let mut party_hooks = unsafe { crate::party_changes::Hooks::new(&memory, base) }?;
    let mut wing_hooks = unsafe { crate::wing_changes::Hooks::new(&memory, base) }?;
    let mut roster_hooks = unsafe { crate::roster_changes::Hooks::new(&memory, base) }?;
    let mut political_status_hooks =
        unsafe { crate::political_status_changes::Hooks::new(&memory, base) }?;
    let mut mission_hooks = unsafe { crate::mission_changes::Hooks::new(&memory, base) }?;
    let mut rail_hooks = unsafe { crate::rail_changes::Hooks::new(&memory, base) }?;
    let mut territory_hooks = unsafe { crate::territory_changes::Hooks::new(&memory, base) }?;
    let mut army_stat_hooks = unsafe { crate::army_stat_changes::Hooks::new(&memory, base) }?;
    let mut country_input_hooks =
        unsafe { crate::country_input_changes::Hooks::new(&memory, base) }?;
    let mut raid_hooks = unsafe { crate::raid_changes::Hooks::new(&memory, base) }?;
    let mut naval_tree_hooks = unsafe { crate::naval_tree_changes::Hooks::new(&memory, base) }?;
    let mut unit_leader_hooks = unsafe { crate::unit_leader_changes::Hooks::new(&memory, base) }?;
    let mut census_hooks = unsafe { crate::census_changes::Hooks::new(&memory, base) }?;
    let mut world_input_hooks = unsafe { crate::world_input_changes::Hooks::new(&memory, base) }?;
    let mut air_record_hooks = unsafe { crate::air_record_changes::Hooks::new(&memory, base) }?;
    let mut local_subunit_hooks =
        unsafe { crate::local_subunit_changes::Hooks::new(&memory, base) }?;
    let mut template_hooks = unsafe { crate::template_changes::Hooks::new(&memory, base) }?;
    let mut ship_model_hooks = unsafe { crate::ship_model_changes::Hooks::new(&memory, base) }?;
    let mut province_unit_hooks =
        unsafe { crate::province_unit_changes::Hooks::new(&memory, base) }?;
    let mut air_base_carry_hooks =
        unsafe { crate::air_base_carry_changes::Hooks::new(&memory, base) }?;
    // A family whose sentinels differ is left uninstalled; it reads as before.
    let mut opinion_hooks = unsafe { crate::opinion_changes::Hooks::new(&memory, base) }?;
    let mut destruction_hooks = unsafe { crate::equipment_destruction::Hooks::new(&memory, base) };
    let mut naval_result_hooks = unsafe { crate::naval_result_sources::Hooks::new(&memory, base) };
    let mut experience_hooks = unsafe { crate::experience_gains::Hooks::new(&memory, base) };
    let mut air_loss_hooks = unsafe { crate::air_losses::Hooks::new(&memory, base) };
    let mut bombing_hooks = unsafe { crate::building_bombing::Hooks::new(&memory, base) };
    let mut nuclear_hooks = unsafe { crate::nuclear_strikes::Hooks::new(&memory, base) };
    let events = crate::country_scale::observe_production_output(&memory, base).unwrap_or(0);
    let mut line_hooks = unsafe { crate::line_output::Hooks::new(&memory, base, events) };
    let mut world_hooks =
        unsafe { crate::world_lifetime::Hooks::new(&memory, base, event_feed_generation) }?;
    let mut land_hooks = unsafe { crate::land_events::Hooks::new(&memory, base) };
    let mut prune_hooks = unsafe { crate::combat_log_prune::Hooks::new(&memory, base) };
    let mut construction_hooks = unsafe { crate::construction_ic::Hooks::new(&memory, base) };
    let logistics_rows = crate::logistics_balance::prepare(&memory, base)
        .map_err(|e| error_reason(e, "logistics balance staging allocation failed"))?;
    let mut logistics_hooks = unsafe { crate::logistics_balance::Hooks::new(&memory, base) }?;
    let mut fuel_hooks = unsafe { crate::fuel_awards::Hooks::new(&memory, base) };
    let mut operation_hooks =
        unsafe { crate::operation_outcomes::Hooks::new(&memory, base, config.operation_outcomes) };
    let mut politics_hooks =
        unsafe { crate::politics_changes::Hooks::new(&memory, base, config.politics_capture) };
    progress.step("hooks_prepared", None);
    // Size every country-proportional buffer from the live registry before
    // the hook exists. At the main menu the registry may still be empty; the
    // spool thread then raises capacities as soon as a save fills it.
    let slots = crate::country_scale::observe(&memory, base).unwrap_or(0);
    crate::country_scale::note_slots(slots);
    // Size the live weather boundary before installing the hourly hook;
    // attachment after a native rebuild still captures its retained edges.
    let _ = crate::extended::weather::observe_display_capacity(&memory, base);
    let mut event_feed_hooks = unsafe {
        crate::event_feed_observer::Hooks::new(
            &memory,
            base,
            event_feed_sequence,
            event_feed_counts,
            slots,
        )
    }?;
    crate::politics_changes::resize(
        crate::country_scale::observe_politics_demand(&memory, base, slots).unwrap_or(slots),
        processor
            .observe_input_demand(engine::politics_sites::lexical_key_demand(&memory, base))
            .unwrap_or(0),
    );
    crate::relation_changes::presize(slots);
    crate::world_input_changes::presize(slots);
    crate::history_changes::presize(slots, strategic_regions(&memory, base));
    let mut sizing = crate::country_scale::Sizing::for_slots(slots);
    match crate::country_scale::source_demand::Demand::observe(&memory, base) {
        Ok(Some(demand)) => demand.apply(&mut sizing),
        Ok(None) => {}
        Err(reason) => processor.observe_input_demand_error(reason),
    }
    if let Err(reason) = crate::country_scale::presize_table_inputs(
        &memory,
        base,
        &mut sizing.map,
        &mut sizing.extended,
    ) {
        processor.observe_input_demand_error(reason);
    }
    if let Err(reason) = sizing.extended.presize_live_inputs(&memory, base) {
        processor.observe_input_demand_error(reason);
    }
    crate::land_events::install_offered();
    sizing.extended.logistics.raise(logistics_rows);
    crate::fuel_awards::presize(&memory, base);
    crate::fuel_awards::install_offered();
    let (entries, names) = crate::fuel_awards::capacities();
    sizing.extended.fuel_awards = crate::extended::fuel_awards::Capacities { entries, names };
    sizing.extended.game_score.enabled = config.game_score;
    if config.game_score
        && let Some(observed) = crate::country_scale::observe_game_score(&memory, base)
            .map_err(|error| error_reason(error, "game score sizing allocation failed"))?
    {
        sizing.extended.game_score.raise(observed.demand);
        processor
            .observe_game_score_sizing(&observed)
            .map_err(|error| error_reason(error, "game score sizing status allocation failed"))?;
    }
    let _ = sizing.extended.combat_progress.presize(&memory, base);
    crate::equipment_destruction::prepare(&memory, base, slots);
    crate::experience_gains::prepare(slots);
    sizing.extended.equipment_destruction = crate::equipment_destruction::capacity();
    crate::naval_result_sources::prepare(&memory, base);
    crate::naval_result_sources::install_offered();
    sizing.extended.naval_result_sources = crate::naval_result_sources::capacity();
    crate::air_losses::prepare(&memory, base, slots);
    crate::building_bombing::prepare(&memory, base, slots);
    crate::nuclear_strikes::presize(crate::building_bombing::buildings().saturating_add(slots));
    sizing.extended.air_losses.events = crate::air_losses::capacity();
    sizing.extended.building_bombing = crate::building_bombing::capacity();
    sizing.extended.nuclear_strikes = crate::nuclear_strikes::capacity();
    sizing.content = engine::cdlc_manager::observe_counts(&memory, base);
    if let Some(rows) = processor
        .observe_input_demand(crate::country_scale::observe_country_intel(&memory, base))
        .flatten()
    {
        sizing.extended.raise_for_country_intel(rows);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_ace_names(&memory, base))
        .flatten()
    {
        sizing.extended.air.raise_for_names(demand);
    }
    if let Some(bytes) = processor
        .observe_input_demand(crate::country_scale::observe_air_wing_names(&memory, base))
        .flatten()
    {
        crate::country_scale::note_air_wing_name_bytes(bytes);
        sizing.map.raise_for_air_wing_names(bytes);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_railway_gun_names(
            &memory, base,
        ))
        .flatten()
    {
        crate::country_scale::note_railway_gun_names(demand);
        sizing.extended.railway_guns.raise_for_names(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_map(&memory, base))
        .flatten()
    {
        for reason in demand.map_refusals() {
            processor.observe_input_demand_error(reason);
        }
        if let Some(reason) = demand.navies.refusal() {
            processor.observe_input_demand_error(reason);
        }
        if let Ok(carrier) = demand.carrier {
            crate::country_scale::note_carrier_demand(carrier);
        }
        sizing.raise_for_map(&demand);
        sizing
            .extended
            .country_counters
            .raise_for_physical_navies(demand.navies.capacities);
        sizing
            .extended
            .naval_counters
            .raise_for_names(demand.navies.name_rows, demand.navies.name_bytes);
        sizing
            .extended
            .naval_counters
            .raise_for_supply_origins(demand.navies.supply_origins);
        let _ = processor.observe_input_demand(
            sizing
                .extended
                .naval_counters
                .raise_for_port(demand.navies.port),
        );
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_presentation_census(
            &memory, base,
        ))
        .flatten()
    {
        if let Some(reason) = demand.failure {
            processor.observe_input_demand_error(reason);
        }
        sizing
            .extended
            .division_presentation
            .raise_for_census(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_raid_arenas(&memory, base))
        .flatten()
    {
        for reason in demand.failures.into_iter().flatten() {
            processor.observe_input_demand_error(reason);
        }
        sizing.extended.raise_for_raid_arenas(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_saved_targets(&memory, base))
        .flatten()
    {
        sizing.extended.saved_event_targets.raise(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_characters(&memory, base))
        .flatten()
    {
        sizing.extended.raise_for_characters(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_politics(&memory, base))
        .flatten()
    {
        sizing
            .extended
            .country_counters
            .raise_for_pride_names(demand.pride_name_bytes);
        for reason in demand.failures() {
            processor.observe_input_demand_error(reason);
        }
        sizing.extended.raise_for_politics(demand);
    }
    if let Some(slots) = processor
        .observe_input_demand(crate::country_scale::observe_reserved_visibility(
            &memory, base,
        ))
        .flatten()
    {
        sizing.extended.raise_reserved_visibility(slots);
    }
    if let Some(demand) = processor
        .observe_input_demand(crate::country_scale::observe_fuel_unit_demand(
            &memory, base,
        ))
        .flatten()
    {
        sizing.extended.economy.demand.raise(demand);
    }
    sizing.extended.operation_outcomes.prepare();
    progress.step("inputs_sized", None);
    let combat_hints = CombatHints::observe(&memory, base);
    for reason in combat_hints.failures.into_iter().flatten() {
        processor.observe_input_demand_error(reason);
    }
    sizing.extended.line_output.entries = sizing.extended.line_output.entries.max(events);
    let peace_capacity = crate::peace_settlements::capacity_for_world(&memory, base);
    sizing.extended.peace_settlements = peace_capacity;
    sizing.extended.peace_bids = peace_capacity;
    let mut peace_hooks =
        unsafe { crate::peace_settlements::Hooks::new(&memory, base, peace_capacity) };
    let pe_offset = read::<u32>(&memory, module as usize + 0x3c).unwrap_or(0) as usize;
    let image_size = read::<u32>(&memory, module as usize + pe_offset + 0x50).unwrap_or(0) as usize;
    let [relay, trampoline] = hook.executable_ranges();
    let mut ranges = vec![
        module as usize..module as usize + image_size,
        relay,
        trampoline,
    ];
    ranges.extend(portrait_hooks.executable_ranges());
    ranges.extend(advisor_hooks.executable_ranges());
    ranges.extend(relation_hooks.executable_ranges());
    ranges.extend(history_hooks.executable_ranges());
    ranges.extend(flag_hooks.executable_ranges());
    ranges.extend(building_hooks.executable_ranges());
    ranges.extend(advisor_pool_hooks.executable_ranges());
    ranges.extend(party_leader_hooks.executable_ranges());
    ranges.extend(variable_hooks.executable_ranges());
    ranges.extend(party_hooks.executable_ranges());
    ranges.extend(wing_hooks.executable_ranges());
    ranges.extend(roster_hooks.executable_ranges());
    ranges.extend(political_status_hooks.executable_ranges());
    ranges.extend(mission_hooks.executable_ranges());
    ranges.extend(rail_hooks.executable_ranges());
    ranges.extend(territory_hooks.executable_ranges());
    ranges.extend(army_stat_hooks.executable_ranges());
    ranges.extend(country_input_hooks.executable_ranges());
    ranges.extend(raid_hooks.executable_ranges());
    ranges.extend(naval_tree_hooks.executable_ranges());
    ranges.extend(unit_leader_hooks.executable_ranges());
    ranges.extend(census_hooks.executable_ranges());
    ranges.extend(world_input_hooks.executable_ranges());
    ranges.extend(air_record_hooks.executable_ranges());
    ranges.extend(local_subunit_hooks.executable_ranges());
    ranges.extend(template_hooks.executable_ranges());
    ranges.extend(ship_model_hooks.executable_ranges());
    ranges.extend(province_unit_hooks.executable_ranges());
    ranges.extend(air_base_carry_hooks.executable_ranges());
    ranges.extend(opinion_hooks.executable_ranges());
    ranges.extend(destruction_hooks.executable_ranges());
    ranges.extend(naval_result_hooks.executable_ranges());
    ranges.extend(experience_hooks.executable_ranges());
    ranges.extend(air_loss_hooks.executable_ranges());
    ranges.extend(bombing_hooks.executable_ranges());
    ranges.extend(nuclear_hooks.executable_ranges());
    ranges.extend(line_hooks.executable_ranges());
    ranges.extend(event_feed_hooks.executable_ranges());
    ranges.extend(world_hooks.executable_ranges());
    ranges.extend(peace_hooks.executable_ranges());
    ranges.extend(land_hooks.executable_ranges());
    ranges.extend(prune_hooks.executable_ranges());
    ranges.extend(construction_hooks.executable_ranges());
    ranges.extend(logistics_hooks.executable_ranges());
    ranges.extend(fuel_hooks.executable_ranges());
    ranges.extend(operation_hooks.executable_ranges());
    ranges.extend(politics_hooks.executable_ranges());
    let mut shared = Shared::with_sizing(base, hook.trampoline(), frequency as u64, &sizing)
        .map_err(|e| error_reason(e, "capture buffers allocation failed"))?;
    shared.prepare_roster(&memory);
    shared
        .prepare_clock_capture(&memory)
        .map_err(|_| "clock preparation allocation failed")?;
    let shared = Arc::new(shared);
    progress.step("shared_started", None);
    let mut spool_state = SpoolState {
        sizing,
        applied_combat_hints: [CombatHintState::default(); sampling::BUFFERS],
    };
    grow_combat_hints(
        &shared,
        &spool_state.sizing.extended,
        &combat_hints,
        &mut spool_state.applied_combat_hints,
    );
    let acquisition = acquisition::Pool::start(
        crate::acquisition::recommended_worker_count(),
        acquisition::MAX_JOBS,
    )?;
    shared
        .pool_bytes
        .fetch_add(acquisition.allocated_bytes() as u64, Ordering::Relaxed);
    shared
        .acquisition
        .set(acquisition)
        .map_err(|_| "acquisition pool already initialized")?;
    let _payload_pools = decode::prepare_payload_pools(config.game_score);
    if config.cancel_file.exists() || detach.signaled() {
        return Err("recording cancelled before hook installation");
    }
    if crate::allocation::failed() {
        return Err("collector allocation failed before hooks");
    }
    let acquisition = shared.acquisition.get().expect("initialized before hook");
    let mut owned_workers = acquisition.thread_ids();
    // The conversion helpers are collector workers for every quiescence check
    // and join with the processor when the spool thread returns. Without them
    // the spool thread converts every table itself, with identical output.
    for role in [
        crate::thread_cpu::Role::SpoolHelper,
        crate::thread_cpu::Role::SecondSpoolHelper,
    ] {
        if let Ok(helper) = crate::spool_helper::SpoolHelper::start() {
            owned_workers.push(helper.thread_id());
            crate::thread_cpu::register_id(role, helper.thread_id());
            processor.set_helper(helper);
        }
    }
    // The sizing thread is a collector worker too; the spool thread owns it
    // and joins it before returning. Without it the spool thread sizes.
    let sizing = SizingThread::start(Arc::clone(&shared), spool_state);
    if let Some(id) = sizing.thread_id() {
        owned_workers.push(id);
        crate::thread_cpu::register_id(crate::thread_cpu::Role::SpoolSizing, id);
    }
    let spool_id = Arc::new(AtomicU32::new(0));
    let spool_thread_id = Arc::clone(&spool_id);
    let spool_shared = Arc::clone(&shared);
    let spool_status = status_path.clone();
    // Owned here and finished only after the spool thread has returned and
    // callbacks are quiescent; the spool thread gets a hand-over handle.
    let mut status_writer = crate::status_writer::StatusWriter::start(write_status_bytes)
        .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    let spool_writer = status_writer.handle();
    let teardown_status = Arc::new(OnceLock::new());
    let spool_teardown_status = Arc::clone(&teardown_status);
    let session_id = session.id;
    #[cfg(feature = "worker_sampler")]
    let sampler_config = config.worker_sampler.clone();
    #[cfg(feature = "worker_sampler")]
    let sampler_output = config.output_dir.clone();
    let spool = std::thread::Builder::new()
        .name("hoi4-spool".into())
        .stack_size(BACKGROUND_STACK_BYTES)
        .spawn(move || {
            spool_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            crate::thread_cpu::register(crate::thread_cpu::Role::Spool);
            spool_loop(
                &spool_shared,
                &config,
                session,
                processor,
                &spool_status,
                sizing,
                spool_writer,
                &spool_teardown_status,
            )
        })
        .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    let mut installation_error = decode::start_worker().err();
    while spool_id.load(Ordering::Acquire) == 0 && !spool.is_finished() {
        std::thread::yield_now();
    }
    owned_workers.extend([spool_id.load(Ordering::Acquire), decode::worker_id()]);
    progress.step("workers_started", None);
    // The sampler starts paused: hook publication below suspends the process,
    // so no sampler suspension may overlap it. Activate only once every hook
    // is installed. Startup errors use the same drain as hook-install errors.
    #[cfg(feature = "worker_sampler")]
    let mut sampler = match crate::worker_sampler::start(
        sampler_config.as_deref(),
        &sampler_output,
        acquisition.thread_ids(),
    ) {
        Ok(sampler) => {
            let id = crate::worker_sampler::thread_id();
            if id != 0 {
                owned_workers.push(id);
            }
            Some(sampler)
        }
        Err(error) => {
            installation_error.get_or_insert_with(|| installation_message(error));
            None
        }
    };
    if crate::allocation::failed() {
        installation_error = Some("collector allocation failed before hooks");
    }
    SHARED.store(Arc::as_ptr(&shared).cast_mut(), Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(15);
    // The family each attempt reached: the one that failed, or that was
    // still not ready when the deadline expired.
    let stage = crate::progress::Stage::new(&progress, "world_lifetime");
    while installation_error.is_none() && !crate::allocation::failed() {
        stage.set("world_lifetime");
        match (|| {
            if !install_attach_batch(
                &stage,
                &mut [
                    ("world_lifetime", &mut || world_hooks.set_enabled(true)),
                    ("fuel_awards", &mut || fuel_hooks.set_enabled(true)),
                    ("advisor_changes", &mut || advisor_hooks.set_enabled(true)),
                    ("portrait_changes", &mut || portrait_hooks.set_enabled(true)),
                    ("relation_changes", &mut || relation_hooks.set_enabled(true)),
                    ("history_changes", &mut || history_hooks.set_enabled(true)),
                    ("flag_changes", &mut || flag_hooks.set_enabled(true)),
                    ("building_changes", &mut || building_hooks.set_enabled(true)),
                    ("advisor_pool_changes", &mut || {
                        advisor_pool_hooks.set_enabled(true)
                    }),
                    ("party_leader_changes", &mut || {
                        party_leader_hooks.set_enabled(true)
                    }),
                    ("variable_changes", &mut || variable_hooks.set_enabled(true)),
                    ("party_changes", &mut || party_hooks.set_enabled(true)),
                    ("wing_changes", &mut || wing_hooks.set_enabled(true)),
                    ("roster_changes", &mut || roster_hooks.set_enabled(true)),
                    ("political_status_changes", &mut || {
                        political_status_hooks.set_enabled(true)
                    }),
                    ("mission_changes", &mut || mission_hooks.set_enabled(true)),
                    ("rail_changes", &mut || rail_hooks.set_enabled(true)),
                    ("territory_changes", &mut || {
                        territory_hooks.set_enabled(true)
                    }),
                    ("army_stat_changes", &mut || {
                        army_stat_hooks.set_enabled(true)
                    }),
                    ("country_input_changes", &mut || {
                        country_input_hooks.set_enabled(true)
                    }),
                    ("raid_changes", &mut || raid_hooks.set_enabled(true)),
                    ("naval_tree_changes", &mut || {
                        naval_tree_hooks.set_enabled(true)
                    }),
                    ("unit_leader_changes", &mut || {
                        unit_leader_hooks.set_enabled(true)
                    }),
                    ("census_changes", &mut || census_hooks.set_enabled(true)),
                    ("world_input_changes", &mut || {
                        world_input_hooks.set_enabled(true)
                    }),
                    ("local_subunit_changes", &mut || {
                        local_subunit_hooks.set_enabled(true)
                    }),
                    ("province_unit_changes", &mut || {
                        province_unit_hooks.set_enabled(true)
                    }),
                    ("air_record_changes", &mut || {
                        air_record_hooks.set_enabled(true)
                    }),
                    ("template_changes", &mut || template_hooks.set_enabled(true)),
                    ("ship_model_changes", &mut || {
                        ship_model_hooks.set_enabled(true)
                    }),
                    ("air_base_carry_changes", &mut || {
                        air_base_carry_hooks.set_enabled(true)
                    }),
                    ("opinion_changes", &mut || opinion_hooks.set_enabled(true)),
                    ("line_output", &mut || line_hooks.set_enabled(true)),
                    ("peace_settlements", &mut || {
                        peace_hooks
                            .set_enabled(true)
                            .map_err(|_| "peace hook installation failed")
                    }),
                ],
            )? {
                return Ok(false);
            }
            stage.set("land_events");
            if !land_hooks
                .set_enabled(true)
                .map_err(|_| "land event hook installation failed")?
            {
                return Ok(false);
            }
            stage.set("construction_ic");
            if !construction_hooks.set_enabled(true)? {
                return Ok(false);
            }
            stage.set("logistics_balance");
            if !logistics_hooks.set_enabled(true)? {
                return Ok(false);
            }
            stage.set("combat_log_prune");
            if !prune_hooks.set_enabled(true)? {
                return Ok(false);
            }
            stage.set("equipment_destruction");
            if !destruction_hooks
                .set_enabled(true)
                .map_err(|_| "equipment destruction hook installation failed")?
            {
                return Ok(false);
            }
            if !install_attach_batch(
                &stage,
                &mut [
                    ("event_feed_observer", &mut || {
                        event_feed_hooks.set_enabled(true)
                    }),
                    ("experience_gains", &mut || {
                        experience_hooks.set_enabled(true)
                    }),
                    ("operation_outcomes", &mut || {
                        operation_hooks
                            .set_enabled(true)
                            .map_err(|_| "operation outcome hook installation failed")
                    }),
                    ("politics_changes", &mut || politics_hooks.set_enabled(true)),
                ],
            )? {
                return Ok(false);
            }
            stage.set("naval_result_sources");
            if !naval_result_hooks.set_enabled(true)? {
                return Ok(false);
            }
            if !install_attach_batch(
                &stage,
                &mut [
                    ("air_losses", &mut || air_loss_hooks.set_enabled(true)),
                    ("building_bombing", &mut || bombing_hooks.set_enabled(true)),
                    ("nuclear_strikes", &mut || nuclear_hooks.set_enabled(true)),
                ],
            )? {
                return Ok(false);
            }
            stage.set("hourly");
            hook.set_enabled(true)
                .map_err(|_| "hourly hook installation failed")
        })() {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            // Recorded in status and, as the terminal error, in
            // collector.error.txt, which the recorder prints to stderr.
            Ok(false) => {
                installation_error = Some(installation_message(format!(
                    "{} hooks were still not ready when the 15 s installation deadline expired",
                    stage.get()
                )));
                break;
            }
            Err(reason) => {
                installation_error = Some(installation_message(format!(
                    "{} hook installation failed: {reason}",
                    stage.get()
                )));
                break;
            }
        }
    }
    // Publication detail cannot affect the installation deadline decision.
    stage.finish();
    #[cfg(feature = "worker_sampler")]
    if installation_error.is_none()
        && !crate::allocation::failed()
        && let Some(sampler) = sampler.as_mut()
        && let Err(error) = sampler.activate()
    {
        installation_error = Some(error);
    }
    if installation_error.is_none() && !crate::allocation::failed() {
        progress.step("ready", None);
        ready.set();
    } else {
        progress.step("installation_refused", None);
    }
    while installation_error.is_none()
        && !detach.signaled()
        && !spool.is_finished()
        && !decode::worker_finished()
        && !shared.acquisition_failed()
        && !crate::allocation::failed()
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    // A callback may be waiting for a reusable frame. Cancel that wait before
    // restoring the hook and waiting for ACTIVE to reach zero; ordinary restore
    // keeps recording until every callback already in the relay has returned.
    let _teardown = crate::allocation::enter(crate::allocation::Phase::Teardown);
    crate::allocation::attempted_hour(None);
    shared.cancel_buffer_wait.store(true, Ordering::Release);
    let decode_stopped_early = installation_error.is_none() && decode::worker_finished();
    let detach_started = Instant::now();
    progress.step("detach_requested", None);
    // Join before any hook-restoration suspension or sampled worker join. This
    // also covers every attach failure after the sampler was started; no fallible
    // operation returns early between startup and this common teardown path.
    #[cfg(feature = "worker_sampler")]
    if let Some(sampler) = sampler.as_mut() {
        let id = crate::worker_sampler::thread_id();
        let samples = sampler.stop();
        // An exited sampler no longer qualifies for worker exclusions: Windows
        // may reuse its id before the observer quiescence census below.
        if id != 0 {
            owned_workers.retain(|worker| *worker != id);
        }
        progress.step("sampler_stopped", Some(samples));
    }
    let mut event_feed_detach = None;
    // Retain every callback, worker and frame owner on refusal. Keep an earlier
    // terminal fault separate from the new stop diagnostic.
    macro_rules! retain_detach {
        ($reason:expr, $fault:expr, $workers:expr) => {{
            let reason: &'static str = $reason;
            let fault: Option<&'static str> = $fault;
            let error = fault.unwrap_or(reason);
            refuse_detach!(error, |_error| {
                shared.recording.store(false, Ordering::Release);
                shared.cancel_buffer_wait.store(true, Ordering::Release);
                status_writer.finish();
                if !crate::allocation::failed() {
                    let status = crate::status::status!({
                        "state":"detach_refused", "session":session_id,
                        "drain_complete":false, "original_bytes_verified":false,
                        "error":error, "detach_refusal":reason,
                        "installation_error":installation_error,
                        "hook_publication":crate::hook::publication::status(),
                        "land_events":crate::land_events::status(),
                        "event_feed_detach":event_feed_detach,
                    });
                    #[cfg(feature = "worker_sampler")]
                    let status = status.join(crate::status::status!({
                        "worker_sampler":crate::worker_sampler::status(),
                    }));
                    let _ = write_status(&status_path, &status);
                    let _ = fs::write(dll_path.with_file_name("collector.error.txt"), error);
                }
                std::mem::forget((
                    $workers, hook, portrait_hooks, advisor_hooks, relation_hooks,
                    history_hooks, flag_hooks, building_hooks, advisor_pool_hooks, party_leader_hooks, variable_hooks, party_hooks, wing_hooks, roster_hooks, political_status_hooks, mission_hooks, rail_hooks, territory_hooks, army_stat_hooks, country_input_hooks, raid_hooks, naval_tree_hooks, air_record_hooks, world_input_hooks, opinion_hooks, destruction_hooks, naval_result_hooks, local_subunit_hooks, template_hooks, ship_model_hooks, province_unit_hooks, air_base_carry_hooks, unit_leader_hooks, census_hooks,
                    air_loss_hooks, bombing_hooks, nuclear_hooks, line_hooks, land_hooks, prune_hooks,
                    construction_hooks, experience_hooks, event_feed_hooks, world_hooks,
                    fuel_hooks, operation_hooks, politics_hooks, logistics_hooks,
                    peace_hooks, shared, output_lock, _payload_pools,
                ));
            });
        }};
    }

    // Keep observing until the hourly entry is restored. A hard refusal must
    // not prevent restoration of the independent writer families below.
    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    restore_hourly(
        &mut restored,
        &mut previous_errors,
        detach_started,
        || {
            hook.set_enabled(false)
                .map_err(|_| "hourly hook restoration failed")
        },
        || std::thread::sleep(Duration::from_millis(100)),
    );
    progress.step(
        "hourly_restored",
        Some(u64::from(restored[RestoreFamily::Hourly as usize])),
    );
    if !restored[RestoreFamily::Hourly as usize] {
        shared.recording.store(false, Ordering::Release);
    }
    // Closing admission, rather than waiting for native originals or mutation
    // observers, proves that no callback can submit another acquisition job.
    HOURLY_ADMISSION.close();
    if let Err(reason) = wait_acquisitions(&HOURLY_ADMISSION, detach_started) {
        progress.step("acquisitions_refused", None);
        retain_detach!(
            reason,
            installation_error.or(Some(reason)),
            (spool, status_writer)
        );
    }
    progress.step("acquisitions_quiet", None);
    // Keep mutation observation intact until the last acquisition has finished.
    // Every round visits all unfinished families even if an earlier one refuses.
    // A damaged enabled patch still prevents unloading its relay and this DLL.
    let mut first_restore_error = None;
    let restore_result = loop {
        // Keep invalidation published while any older native invocation can
        // still reach an observer post reader, even after its site was restored.
        let mut toggle = |family| match family {
            RestoreFamily::Hourly => hook
                .set_enabled(false)
                .map_err(|_| "hourly hook restoration failed"),
            RestoreFamily::EquipmentDestruction => destruction_hooks
                .set_enabled(false)
                .map_err(|_| "equipment destruction hook restoration failed"),
            RestoreFamily::Land => land_hooks
                .set_enabled(false)
                .map_err(|_| "land event hook restoration failed"),
            RestoreFamily::Portrait => portrait_hooks.set_enabled(false),
            RestoreFamily::Opinion => opinion_hooks.set_enabled(false),
            RestoreFamily::Construction => construction_hooks.set_enabled(false),
            RestoreFamily::LineOutput => line_hooks.set_enabled(false),
            RestoreFamily::CombatLogPrune => prune_hooks.set_enabled(false),
            RestoreFamily::Advisor => advisor_hooks.set_enabled(false),
            RestoreFamily::Relation => relation_hooks.set_enabled(false),
            RestoreFamily::History => history_hooks.set_enabled(false),
            RestoreFamily::Flags => flag_hooks.set_enabled(false),
            RestoreFamily::Buildings => building_hooks.set_enabled(false),
            RestoreFamily::AdvisorPools => advisor_pool_hooks.set_enabled(false),
            RestoreFamily::PartyLeaders => party_leader_hooks.set_enabled(false),
            RestoreFamily::Variables => variable_hooks.set_enabled(false),
            RestoreFamily::Parties => party_hooks.set_enabled(false),
            RestoreFamily::Wings => wing_hooks.set_enabled(false),
            RestoreFamily::Roster => roster_hooks.set_enabled(false),
            RestoreFamily::PoliticalStatus => political_status_hooks.set_enabled(false),
            RestoreFamily::Missions => mission_hooks.set_enabled(false),
            RestoreFamily::Rails => rail_hooks.set_enabled(false),
            RestoreFamily::Territory => territory_hooks.set_enabled(false),
            RestoreFamily::ArmyStats => army_stat_hooks.set_enabled(false),
            RestoreFamily::CountryInputs => country_input_hooks.set_enabled(false),
            RestoreFamily::Raid => raid_hooks.set_enabled(false),
            RestoreFamily::NavalTrees => naval_tree_hooks.set_enabled(false),
            RestoreFamily::UnitLeaders => unit_leader_hooks.set_enabled(false),
            RestoreFamily::Census => census_hooks.set_enabled(false),
            RestoreFamily::WorldInputs => world_input_hooks.set_enabled(false),
            RestoreFamily::AirRecords => air_record_hooks.set_enabled(false),
            RestoreFamily::LocalSubunits => local_subunit_hooks.set_enabled(false),
            RestoreFamily::Templates => template_hooks.set_enabled(false),
            RestoreFamily::ShipModels => ship_model_hooks.set_enabled(false),
            RestoreFamily::ProvinceUnits => province_unit_hooks.set_enabled(false),
            RestoreFamily::AirBaseCarry => air_base_carry_hooks.set_enabled(false),
            RestoreFamily::Experience => experience_hooks.set_enabled(false),
            RestoreFamily::EventFeed => event_feed_hooks.set_enabled(false),
            RestoreFamily::Fuel => fuel_hooks.set_enabled(false),
            RestoreFamily::OperationOutcomes => operation_hooks
                .set_enabled(false)
                .map_err(|_| "operation outcome hook restoration failed"),
            RestoreFamily::Politics => politics_hooks.set_enabled(false),
            RestoreFamily::Logistics => logistics_hooks.set_enabled(false),
            RestoreFamily::Peace => peace_hooks
                .set_enabled(false)
                .map_err(|_| "peace hook restoration failed"),
            RestoreFamily::NavalResultSources => naval_result_hooks.set_enabled(false),
            RestoreFamily::AirLosses => air_loss_hooks.set_enabled(false),
            RestoreFamily::BuildingBombing => bombing_hooks.set_enabled(false),
            RestoreFamily::NuclearStrikes => nuclear_hooks.set_enabled(false),
            RestoreFamily::WorldLifetime => world_hooks.set_enabled(false),
        };
        // Every unrestored family that takes part publishes under one census
        // and suspension (`hook::shared`); `restore_round` then applies and
        // logs each family's outcome, and publishes the others on their own.
        let mut restore_round_shared = crate::hook::shared::Round::collect();
        if let Some(round) = restore_round_shared.as_mut() {
            for family in RestoreFamily::ALL {
                if family.shared() && !restored[family as usize] {
                    let _ = toggle(family);
                }
            }
            let published = round.publish();
            progress.step(
                format_args!("restore_shared {} families", published.families),
                Some(ticks_to_micros(published.ticks, frequency)),
            );
        }
        let restore = |family| {
            let result = toggle(family);
            let result = if result == Ok(false) && detach_started.elapsed() >= DETACH_BUDGET {
                Err(mutation_detach_refusal(
                    detach_started.elapsed(),
                    |family| family.observer_state().0,
                ))
            } else {
                result
            };
            // Its count: the microseconds of its own checks and writes in the
            // shared suspension, when it took part (taken, so no other family
            // reads it).
            progress.step(
                format_args!(
                    "restore {family:?} {}",
                    match result {
                        Ok(true) => "done",
                        Ok(false) => "busy",
                        Err(_) => "refused",
                    }
                ),
                crate::hook::shared::take_reported_ticks()
                    .map(|ticks| ticks_to_micros(ticks, frequency)),
            );
            if let Err(error) = result {
                first_restore_error.get_or_insert(error);
            }
            result
        };
        let result = restore_round(
            &mut restored,
            &mut previous_errors,
            observers_active,
            restore,
        );
        drop(restore_round_shared);
        match result {
            Ok(true) => break Ok(()),
            Ok(false) => {
                let Some(delay) =
                    detach_retry_delay(detach_started.elapsed(), Duration::from_millis(100))
                else {
                    break Err(first_restore_error.unwrap_or_else(|| {
                        mutation_detach_refusal(detach_started.elapsed(), |family| {
                            family.observer_state().0
                        })
                    }));
                };
                std::thread::sleep(delay);
            }
            Err(error) => break Err(first_restore_error.unwrap_or(error)),
        }
    };
    progress.step(
        if restore_result.is_ok() {
            "restored"
        } else {
            "restoration_refused"
        },
        None,
    );
    let _ = teardown_status.set(TeardownStatus {
        restoration_error: restore_result.err(),
        installation_error,
    });
    if restore_result.is_err() {
        // An enabled patch can no longer be safely restored. Stop acquisition,
        // finish the owned spool, and retain everything a relay can still use.
        shared.recording.store(false, Ordering::Release);
    }
    let hooks = HookLifetime {
        restored: restore_result.is_ok(),
        shared: &SHARED,
    };
    if hooks.restored
        && let Err(reason) = wait_quiet(
            &ranges,
            image_size,
            &owned_workers,
            QuietPhase::Observers,
            &restored,
            detach_started,
        )
    {
        progress.step("observers_refused", None);
        retain_detach!(
            reason,
            installation_error.or(Some(reason)),
            (spool, status_writer)
        );
    }
    progress.step("observers_quiet", None);
    // Normal restoration also waits for callbacks and originals before release;
    // the pinned path retains their state after acquisition admission closes.
    shared.recording.store(false, Ordering::Release);
    // Closed admission proves every acquisition lease is complete; successful
    // restoration additionally proves callback and relay quiescence for unload.
    // Join the precreated readers before releasing any shared/frame storage.
    if let Err(reason) = acquisition.shutdown() {
        // Pool::shutdown reports an acquisition that escaped its runner guard.
        // A caller-owned Running marker can remain after the workers joined.
        progress.step("acquisition_refused", None);
        retain_detach!(
            reason,
            installation_error.or(Some(reason)),
            (spool, status_writer)
        );
    }
    progress.step("acquisition_joined", None);
    // Decode drain first: pending refreshes decode in FIFO hour order into
    // their pooled frames and hand off to the spool ready queue; the existing
    // spool drain below then consumes them in order. Undrainable items use the
    // existing LossBatch gap path with no new reason strings.
    // One step a second while queued refreshes decode.
    let mut logged = Instant::now();
    let decode_outcome = decode::detach_drain(|count| {
        if logged.elapsed() >= Duration::from_secs(1) {
            logged = Instant::now();
            progress.step("decode_draining", Some(count));
        }
    });
    progress.step("decode_drained", None);
    // Acquisition producers have stopped; the spool can take their final loss batch.
    shared.stop_spool.store(true, Ordering::Release);
    shared.wake_spool();
    // Each change of the ready queue's length is progress for the recorder.
    let mut pending = usize::MAX;
    while !spool.is_finished() {
        let ready = shared.ready.len();
        if ready != pending {
            pending = ready;
            progress.step("spool_draining", Some(ready as u64));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    progress.step("spool_finished", None);
    let spool_outcome = spool
        .join()
        .map_err(|_| "spool thread failed")
        .and_then(|result| result);
    status_writer.finish();
    let (mut spool_result, terminal_status) = match spool_outcome {
        Ok(outcome) => (
            outcome.result,
            Some((outcome.terminal_status, outcome.directories)),
        ),
        Err(error) => (Err(error), None),
    };
    if shared.acquisition_failed() {
        add_terminal_error(&mut spool_result, "game-thread acquisition failed");
    }
    if let Err(error) = decode_outcome {
        add_terminal_error(&mut spool_result, error);
    } else if decode_stopped_early {
        add_terminal_error(&mut spool_result, "decode worker stopped before detach");
    }
    if hooks.restored
        && let Err(reason) = wait_quiet(
            &ranges,
            image_size,
            &[],
            QuietPhase::Release,
            &restored,
            detach_started,
        )
    {
        progress.step("release_refused", None);
        retain_detach!(reason, spool_result.as_ref().err().copied(), ());
    }
    progress.step("release_quiet", None);
    if hooks.restored {
        // Spool/status readers joined and every family passed the final quiet check.
        experience_hooks.release_after_quiet();
        event_feed_detach = Some(match event_feed_hooks.release_after_quiet() {
            Ok(losses) => losses,
            Err(reason) => retain_detach!(reason, spool_result.as_ref().err().copied(), ()),
        });
        unsafe {
            crate::logistics_balance::release();
            crate::operation_outcomes::release();
            crate::politics_changes::release();
        }
        if let Err(reason) = crate::land_events::release()
            .and_then(|()| crate::equipment_destruction::release())
            .and_then(|()| crate::naval_result_sources::release())
            .and_then(|()| crate::fuel_awards::release())
        {
            retain_detach!(reason, spool_result.as_ref().err().copied(), ());
        }
    }
    hooks.release(
        (
            hook,
            portrait_hooks,
            advisor_hooks,
            relation_hooks,
            history_hooks,
            flag_hooks,
            building_hooks,
            advisor_pool_hooks,
            party_leader_hooks,
            variable_hooks,
            party_hooks,
            wing_hooks,
            roster_hooks,
            political_status_hooks,
            mission_hooks,
            rail_hooks,
            territory_hooks,
            army_stat_hooks,
            country_input_hooks,
            raid_hooks,
            naval_tree_hooks,
            unit_leader_hooks,
            census_hooks,
            world_input_hooks,
            air_record_hooks,
            local_subunit_hooks,
            template_hooks,
            ship_model_hooks,
            province_unit_hooks,
            air_base_carry_hooks,
            opinion_hooks,
            destruction_hooks,
            naval_result_hooks,
            air_loss_hooks,
            bombing_hooks,
            nuclear_hooks,
            line_hooks,
            land_hooks,
            prune_hooks,
            construction_hooks,
            experience_hooks,
            event_feed_hooks,
            world_hooks,
            fuel_hooks,
            operation_hooks,
            politics_hooks,
            logistics_hooks,
            peace_hooks,
            shared,
            _payload_pools,
        ),
        || {},
    );
    progress.step("released", None);
    let drain_complete = spool_result.is_ok();
    if let Some(error) = installation_error {
        add_terminal_error(&mut spool_result, error);
    }
    if let Err(error) = restore_result {
        add_terminal_error(&mut spool_result, error);
    }
    if !crate::allocation::failed() {
        let pinned = restore_result.is_err();
        let status = crate::status::status!({
            "state":if pinned { "pinned" } else { "detached" },"session":session_id,
            "process_start_time":process_start,"drain_complete":drain_complete,
            "error":spool_result.as_ref().err().copied(),
            "installation_error":installation_error,
            "hook_publication":crate::hook::publication::status(),
            "restoration_error":restore_result.err(),"collector_pinned":pinned,
            "recording_stopped":true,"spool_closed":true,
            "original_bytes_verified":!pinned,
            "active_callbacks":if pinned { None } else { Some(0_u64) },
            "event_feed_detach":event_feed_detach,
        });
        #[cfg(feature = "worker_sampler")]
        let status = status.join(crate::status::status!({
            "worker_sampler":crate::worker_sampler::status(),
        }));
        let write = if drain_complete {
            if let Some((mut bytes, directories)) = terminal_status {
                append_event_feed_detach_status(&mut bytes, event_feed_detach)
                    .and_then(|()| crate::status_writer::measure_bytes(&mut bytes, &directories))
                    .and_then(|()| write_status_bytes(&status_path, &bytes))
            } else {
                write_status(&status_path, &status)
            }
        } else {
            write_status(&status_path, &status)
        };
        if let Err(error) = write {
            crate::allocation::observe(&error);
            add_terminal_error(&mut spool_result, "terminal status write failed");
        }
    }
    #[cfg(feature = "worker_sampler")]
    if restore_result.is_ok() {
        crate::worker_sampler::release();
    }
    progress.step("done", None);
    // Closed before done, so the recorder may remove the session directory.
    drop(progress);
    signal_done(output_lock, &done);
    spool_result
}

fn append_event_feed_detach_status(
    bytes: &mut Vec<u8>,
    losses: Option<crate::event_feed_history::DetachLosses>,
) -> io::Result<()> {
    // The spool's final snapshot precedes receipt drain and store release.
    let suffix = crate::status::serialize(&crate::status::status!({"event_feed_detach":losses}))?;
    if bytes.last() != Some(&b'}') {
        return Err(io::ErrorKind::InvalidData.into());
    }
    engine::allocation::reserve(bytes, suffix.len())?;
    bytes.pop();
    if bytes.len() > 1 {
        bytes.push(b',');
    }
    bytes.extend_from_slice(&suffix[1..]);
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
enum RestoreFamily {
    Hourly,
    EquipmentDestruction,
    Land,
    Portrait,
    Opinion,
    Construction,
    LineOutput,
    CombatLogPrune,
    Advisor,
    Relation,
    History,
    Flags,
    Buildings,
    AdvisorPools,
    PartyLeaders,
    Variables,
    Parties,
    Wings,
    Roster,
    PoliticalStatus,
    Missions,
    Rails,
    Territory,
    ArmyStats,
    CountryInputs,
    Raid,
    NavalTrees,
    UnitLeaders,
    Census,
    WorldInputs,
    AirRecords,
    LocalSubunits,
    Templates,
    ShipModels,
    ProvinceUnits,
    AirBaseCarry,
    Experience,
    Fuel,
    OperationOutcomes,
    Politics,
    Logistics,
    Peace,
    NavalResultSources,
    AirLosses,
    BuildingBombing,
    NuclearStrikes,
    EventFeed,
    WorldLifetime,
}

impl RestoreFamily {
    fn observer_state(self) -> (bool, &'static str) {
        match self {
            Self::Hourly => (
                ACTIVE.load(Ordering::SeqCst) != 0,
                "hourly callbacks did not quiesce; collector retained",
            ),
            Self::EquipmentDestruction => (
                crate::equipment_destruction::active() != 0,
                "equipment destruction callbacks did not quiesce; collector retained",
            ),
            Self::Land => (
                crate::land_events::active() != 0,
                "land event callbacks did not quiesce; collector retained",
            ),
            Self::Portrait => (
                crate::portrait_changes::ACTIVE.load(Ordering::SeqCst) != 0,
                "portrait callbacks did not quiesce; collector retained",
            ),
            Self::Opinion => (
                crate::opinion_changes::ACTIVE.load(Ordering::SeqCst) != 0,
                "opinion callbacks did not quiesce; collector retained",
            ),
            Self::Construction => (
                crate::construction_ic::ACTIVE.load(Ordering::SeqCst) != 0,
                "construction callbacks did not quiesce; collector retained",
            ),
            Self::LineOutput => (
                crate::line_output::ACTIVE.load(Ordering::SeqCst) != 0
                    || crate::line_output::native_retention_required(),
                "production output callbacks did not quiesce; collector retained",
            ),
            Self::CombatLogPrune => (
                crate::combat_log_prune::ACTIVE.load(Ordering::SeqCst) != 0,
                "combat log prune callbacks did not quiesce; collector retained",
            ),
            Self::Advisor => (
                crate::advisor_changes::ACTIVE.load(Ordering::SeqCst) != 0
                    || crate::character_changes::active() != 0,
                "advisor callbacks did not quiesce; collector retained",
            ),
            Self::Relation => (
                crate::relation_changes::active() != 0,
                "relation callbacks did not quiesce; collector retained",
            ),
            Self::History => (
                crate::history_changes::active() != 0,
                "history callbacks did not quiesce; collector retained",
            ),
            Self::Flags => (
                crate::flag_changes::active() != 0,
                "script flag callbacks did not quiesce; collector retained",
            ),
            Self::Buildings => (
                crate::building_changes::active() != 0,
                "building status callbacks did not quiesce; collector retained",
            ),
            Self::AdvisorPools => (
                crate::advisor_pool_changes::active() != 0,
                "advisor pool callbacks did not quiesce; collector retained",
            ),
            Self::PartyLeaders => (
                crate::party_leader_changes::active() != 0,
                "party leader callbacks did not quiesce; collector retained",
            ),
            Self::Variables => (
                crate::variable_changes::active() != 0,
                "script variable callbacks did not quiesce; collector retained",
            ),
            Self::Parties => (
                crate::party_changes::active() != 0,
                "political party callbacks did not quiesce; collector retained",
            ),
            Self::Wings => (
                crate::wing_changes::active() != 0,
                "air wing equipment callbacks did not quiesce; collector retained",
            ),
            Self::Roster => (
                crate::roster_changes::active() != 0,
                "character roster callbacks did not quiesce; collector retained",
            ),
            Self::PoliticalStatus => (
                crate::political_status_changes::active() != 0,
                "political status callbacks did not quiesce; collector retained",
            ),
            Self::Missions => (
                crate::mission_changes::active() != 0,
                "registered air mission callbacks did not quiesce; collector retained",
            ),
            Self::Rails => (
                crate::rail_changes::active() != 0,
                "rail info callbacks did not quiesce; collector retained",
            ),
            Self::Territory => (
                crate::territory_changes::active() != 0,
                "state and province callbacks did not quiesce; collector retained",
            ),
            Self::ArmyStats => (
                crate::army_stat_changes::active() != 0,
                "division stat block callbacks did not quiesce; collector retained",
            ),
            Self::CountryInputs => (
                crate::country_input_changes::active() != 0,
                "stability and research input callbacks did not quiesce; collector retained",
            ),
            Self::Raid => (
                crate::raid_changes::active() != 0,
                "raid state input callbacks did not quiesce; collector retained",
            ),
            Self::NavalTrees => (
                crate::naval_tree_changes::active()
                    + crate::retreat_ban_changes::active()
                    + crate::unit_leader_trait_changes::active()
                    != 0,
                "naval modifier tree callbacks did not quiesce; collector retained",
            ),
            Self::UnitLeaders => (
                crate::unit_leader_changes::active() != 0,
                "unit leader callbacks did not quiesce; collector retained",
            ),
            Self::Census => (
                crate::census_changes::active() != 0,
                "role census callbacks did not quiesce; collector retained",
            ),
            Self::WorldInputs => (
                crate::world_input_changes::active() != 0,
                "province name, region access and world tension callbacks did not quiesce; collector retained",
            ),
            Self::AirRecords => (
                crate::air_record_changes::active() != 0,
                "air combat record callbacks did not quiesce; collector retained",
            ),
            Self::LocalSubunits => (
                crate::local_subunit_changes::active() != 0,
                "local sub-unit clone callbacks did not quiesce; collector retained",
            ),
            Self::Templates => (
                crate::template_changes::active() != 0,
                "division template name-key callbacks did not quiesce; collector retained",
            ),
            Self::ShipModels => (
                crate::ship_model_changes::active() != 0,
                "ship model input callbacks did not quiesce; collector retained",
            ),
            Self::ProvinceUnits => (
                crate::province_unit_changes::active() != 0,
                "province occupant list callbacks did not quiesce; collector retained",
            ),
            Self::AirBaseCarry => (
                crate::air_base_carry_changes::active() != 0,
                "air base registration callbacks did not quiesce; collector retained",
            ),
            Self::Experience => (
                crate::experience_gains::active() != 0,
                "experience callbacks did not quiesce; collector retained",
            ),
            Self::Fuel => (
                crate::fuel_awards::active() != 0,
                "fuel callbacks did not quiesce; collector retained",
            ),
            Self::OperationOutcomes => (
                crate::operation_outcomes::ACTIVE.load(Ordering::SeqCst) != 0,
                "operation completion callbacks did not quiesce; collector retained",
            ),
            Self::Politics => (
                crate::politics_changes::ACTIVE.load(Ordering::SeqCst) != 0,
                "politics callbacks did not quiesce; collector retained",
            ),
            Self::Logistics => (
                crate::logistics_balance::ACTIVE.load(Ordering::SeqCst) != 0,
                "logistics callbacks did not quiesce; collector retained",
            ),
            Self::Peace => (
                crate::peace_settlements::ACTIVE.load(Ordering::SeqCst) != 0,
                "peace settlement callbacks did not quiesce; collector retained",
            ),
            Self::NavalResultSources => (
                crate::naval_result_sources::active() != 0,
                "naval result callbacks did not quiesce; collector retained",
            ),
            Self::AirLosses => (
                crate::air_losses::active() != 0,
                "air loss callbacks did not quiesce; collector retained",
            ),
            Self::BuildingBombing => (
                crate::building_bombing::active() != 0,
                "building bombing callbacks did not quiesce; collector retained",
            ),
            Self::NuclearStrikes => (
                crate::nuclear_strikes::active() != 0,
                "nuclear strike callbacks did not quiesce; collector retained",
            ),
            Self::EventFeed => (
                crate::event_feed_observer::active() != 0,
                "Event feed callbacks did not quiesce; collector retained",
            ),
            Self::WorldLifetime => (
                crate::world_lifetime::ACTIVE.load(Ordering::SeqCst) != 0,
                "native world lifetime callbacks did not quiesce; collector retained",
            ),
        }
    }

    /// The families whose hooks publish through `hook::shared` rounds
    /// (`Hook::set_family_enabled_shared`): every plain family publication.
    /// The others (taps with unwind retention, split and multi-step
    /// publications, the hourly hook and the world lifetime family) publish
    /// on their own.
    const fn shared(self) -> bool {
        matches!(
            self,
            Self::Portrait
                | Self::Opinion
                | Self::Advisor
                | Self::Relation
                | Self::History
                | Self::Flags
                | Self::Buildings
                | Self::AdvisorPools
                | Self::PartyLeaders
                | Self::Variables
                | Self::Parties
                | Self::Wings
                | Self::Roster
                | Self::PoliticalStatus
                | Self::Missions
                | Self::Rails
                | Self::Territory
                | Self::ArmyStats
                | Self::CountryInputs
                | Self::Raid
                | Self::NavalTrees
                | Self::UnitLeaders
                | Self::Census
                | Self::WorldInputs
                | Self::AirRecords
                | Self::LocalSubunits
                | Self::Templates
                | Self::ShipModels
                | Self::ProvinceUnits
                | Self::AirBaseCarry
                | Self::AirLosses
                | Self::NuclearStrikes
        )
    }

    const ALL: [Self; 48] = [
        Self::Hourly,
        Self::EquipmentDestruction,
        Self::Land,
        Self::Portrait,
        Self::Opinion,
        Self::Construction,
        Self::LineOutput,
        Self::CombatLogPrune,
        Self::Advisor,
        Self::Relation,
        Self::History,
        Self::Flags,
        Self::Buildings,
        Self::AdvisorPools,
        Self::PartyLeaders,
        Self::Variables,
        Self::Parties,
        Self::Wings,
        Self::Roster,
        Self::PoliticalStatus,
        Self::Missions,
        Self::Rails,
        Self::Territory,
        Self::ArmyStats,
        Self::CountryInputs,
        Self::Raid,
        Self::NavalTrees,
        Self::UnitLeaders,
        Self::Census,
        Self::WorldInputs,
        Self::AirRecords,
        Self::LocalSubunits,
        Self::Templates,
        Self::ShipModels,
        Self::ProvinceUnits,
        Self::AirBaseCarry,
        Self::Experience,
        Self::Fuel,
        Self::OperationOutcomes,
        Self::Politics,
        Self::Logistics,
        Self::Peace,
        Self::NavalResultSources,
        Self::AirLosses,
        Self::BuildingBombing,
        Self::NuclearStrikes,
        Self::EventFeed,
        Self::WorldLifetime,
    ];
}

type AttachFamily<'a> = (
    &'static str,
    &'a mut dyn FnMut() -> Result<bool, &'static str>,
);

/// Collect one-publication families, then report their outcomes in the
/// original chain order. A collect error ends the candidate prefix; report
/// keeps each family's original refusal mapping and post-write bookkeeping.
fn install_attach_batch(
    stage: &crate::progress::Stage<'_>,
    families: &mut [AttachFamily<'_>],
) -> Result<bool, &'static str> {
    let mut round = crate::hook::shared::Round::collect_attach();
    let mut collect_error = None;
    if let Some(round) = &mut round {
        for (index, (name, install)) in families.iter_mut().enumerate() {
            crate::hook::publication::select_family(Some(*name));
            let _ = crate::hook::shared::take_collect_allocation_failure();
            let outcome = install();
            let allocation_failure = crate::hook::shared::take_collect_allocation_failure();
            if let Err(reason) = outcome {
                collect_error = Some((index, reason, allocation_failure));
                break;
            }
        }
        round.publish();
    }
    let report_count = collect_error.map_or(families.len(), |(index, _, _)| index);
    for (name, install) in families.iter_mut().take(report_count) {
        stage.set(name);
        if !install()? {
            return Ok(false);
        }
    }
    if let Some((index, reason, allocation_failure)) = collect_error {
        stage.set(families[index].0);
        crate::hook::shared::prepare_refused_family(families[index].0);
        if allocation_failure {
            let _ = engine::allocation::oom();
        }
        return Err(reason);
    }
    Ok(true)
}

fn world_restore_ready(restored: &[bool; RestoreFamily::ALL.len()], active: bool) -> bool {
    !active
        && restored[..RestoreFamily::WorldLifetime as usize]
            .iter()
            .all(|ready| *ready)
}

fn restore_hourly(
    restored: &mut [bool; RestoreFamily::ALL.len()],
    previous_errors: &mut [bool; RestoreFamily::ALL.len()],
    started: Instant,
    mut restore: impl FnMut() -> Result<bool, &'static str>,
    mut wait: impl FnMut(),
) {
    let index = RestoreFamily::Hourly as usize;
    loop {
        match restore() {
            Ok(true) => {
                restored[index] = true;
                return;
            }
            Ok(false) => {
                if detach_retry_delay(started.elapsed(), Duration::from_millis(100)).is_none() {
                    return;
                }
                wait();
            }
            Err(_) => {
                // This attempt precedes restore_round; its next Err is the
                // second consecutive failure, not a new first failure.
                previous_errors[index] = true;
                return;
            }
        }
    }
}

struct HookLifetime<'a> {
    restored: bool,
    shared: &'a AtomicPtr<Shared>,
}

impl HookLifetime<'_> {
    fn wait_quiet(&self, wait: impl FnOnce()) {
        if self.restored {
            wait();
        }
    }

    fn release<T>(self, hooks: T, wait: impl FnOnce()) {
        self.wait_quiet(wait);
        if self.restored {
            self.shared.store(ptr::null_mut(), Ordering::Release);
            drop(hooks);
        } else {
            // A live relay still needs its hooks and Shared; the nonnull
            // pointer also prevents finish_worker from unloading this DLL.
            std::mem::forget(hooks);
        }
    }
}

fn restore_round(
    restored: &mut [bool; RestoreFamily::ALL.len()],
    previous_errors: &mut [bool; RestoreFamily::ALL.len()],
    mut active: impl FnMut() -> bool,
    mut restore: impl FnMut(RestoreFamily) -> Result<bool, &'static str>,
) -> Result<bool, &'static str> {
    let mut persistent_error = None;
    for family in RestoreFamily::ALL {
        let index = family as usize;
        if !restored[index] {
            // World invalidation stays published
            // through other native returns. Recheck after this round's restores:
            // they can finish the last flight and consume the retry budget.
            if family == RestoreFamily::WorldLifetime && !world_restore_ready(restored, active()) {
                continue;
            }
            let previous_error = &mut previous_errors[index];
            match restore(family) {
                Ok(ready) => {
                    restored[index] = ready;
                    *previous_error = false;
                }
                Err(error) => {
                    if *previous_error {
                        persistent_error.get_or_insert(error);
                    }
                    *previous_error = true;
                }
            }
        }
    }
    match persistent_error {
        Some(error) => Err(error),
        None => Ok(restored.iter().all(|complete| *complete)),
    }
}

/// QPC ticks as microseconds, for progress steps.
fn ticks_to_micros(ticks: i64, frequency: i64) -> u64 {
    u64::try_from(i128::from(ticks.max(0)) * 1_000_000 / i128::from(frequency.max(1)))
        .unwrap_or(u64::MAX)
}

fn signal_done(output_lock: File, done: &Event) {
    // The recorder may begin replay finalization as soon as it observes done.
    drop(output_lock);
    done.set();
}

struct CaptureAdmission {
    closed: AtomicBool,
    active: AtomicUsize,
}

impl CaptureAdmission {
    const fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            active: AtomicUsize::new(0),
        }
    }

    fn enter(&self) -> Option<CaptureLease<'_>> {
        // SeqCst puts admission before or after close: an earlier admitted read
        // stays counted until its lease drops; a later entry cannot acquire.
        self.active.fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::SeqCst) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(CaptureLease(self))
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn quiet(&self) -> bool {
        self.active.load(Ordering::SeqCst) == 0
    }
}

pub(crate) struct CaptureLease<'a>(&'a CaptureAdmission);
impl Drop for CaptureLease<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

static HOURLY_ADMISSION: CaptureAdmission = CaptureAdmission::new();

pub(crate) fn admit_hourly_capture() -> Option<CaptureLease<'static>> {
    HOURLY_ADMISSION.enter()
}

fn wait_acquisitions(admission: &CaptureAdmission, started: Instant) -> Result<(), &'static str> {
    while !admission.quiet() {
        if started.elapsed() >= DETACH_BUDGET {
            return Err("hourly acquisitions did not quiesce; collector retained");
        }
        std::thread::yield_now();
    }
    Ok(())
}

fn reserve_output(config: &Config, detach: &Event) -> Result<File, &'static str> {
    let path = config.replay_dir.join("finalize.lock");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if config.cancel_file.exists() || detach.signaled() {
            return Err("recording cancelled while waiting for campaign output");
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) if error.raw_os_error() == Some(32) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => {
                return Err(error_reason(
                    error,
                    "campaign output is busy or unavailable",
                ));
            }
        }
    }
}
fn add_terminal_error(result: &mut Result<(), &'static str>, error: &'static str) {
    if result.is_ok() {
        *result = Err(error);
    }
}
#[cfg(test)]
fn set_drain_status(status: &mut Value, result: &Result<(), &'static str>) {
    let previous = status.as_object_mut().unwrap().remove("error");
    if result.is_ok()
        && status.get("warning").is_none()
        && let Some(previous) = previous
    {
        status["warning"] = previous;
    }
    status["drain_complete"] = json!(result.is_ok());
    if let Err(error) = result {
        status["error"] = json!(error);
    }
}

/// Additive decode-worker calibration for status. Existing keys survive; the
/// counters are monotonic since attach (see `crate::decode`). `table_ns`
/// for raw families is acquisition-only (workers may overlap); `decode_ns` lives here, so
/// the two are not comparable across threads (PLAN-appendix
/// apples-to-oranges).
/// Per-frame read counters and processed-frame totals remain untouched here;
/// asynchronously sampled live counters have a different observation cohort.
struct DecodeStatus<T, U, V> {
    base: crate::status::Object<T>,
    shared: Option<crate::status::Object<U>>,
    acquisition: Option<crate::status::Object<V>>,
}

impl<T: crate::status::Fields, U: crate::status::Fields, V: crate::status::Fields> serde::Serialize
    for DecodeStatus<T, U, V>
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let fields = self.base.0.len()
            + self.shared.as_ref().map_or(0, |status| status.0.len())
            + self.acquisition.as_ref().map_or(0, |status| status.0.len());
        let mut map = serializer.serialize_map(Some(fields))?;
        self.base.0.serialize(&mut map)?;
        if let Some(status) = &self.shared {
            status.0.serialize(&mut map)?;
        }
        if let Some(status) = &self.acquisition {
            status.0.serialize(&mut map)?;
        }
        map.end()
    }
}

fn decode_status() -> impl serde::Serialize {
    let shared_ptr = SHARED.load(Ordering::Acquire);
    let shared = if shared_ptr.is_null() {
        None
    } else {
        // Status is produced while runtime retains Shared and joins its spool.
        Some(unsafe { &*shared_ptr })
    };
    decode_status_from(shared)
}

fn decode_status_from(shared: Option<&Shared>) -> impl serde::Serialize + '_ {
    use crate::status::{DebugValue, map, sequence, status};
    let base = status!({
        "decode_work": decode::calibration_json(),
        "hook_timing": crate::hook_timing::status(),
        "hook_publication": crate::hook::publication::status(),
        "portrait_changes": crate::portrait_changes::status(),
        "advisor_changes": crate::advisor_changes::status(),
        "relation_changes": crate::relation_changes::status(),
        "history_changes": crate::history_changes::status(),
        "flag_changes": crate::flag_changes::status(),
        "building_changes": crate::building_changes::status(),
        "advisor_pool_changes": crate::advisor_pool_changes::status(),
        "party_leader_changes": crate::party_leader_changes::status(),
        "variable_changes": crate::variable_changes::status(),
        "party_changes": crate::party_changes::status(),
        "wing_changes": crate::wing_changes::status(),
        "roster_changes": crate::roster_changes::status(),
        "political_status_changes": crate::political_status_changes::status(),
        "war_changes": crate::war_changes::status(),
        "mission_changes": crate::mission_changes::status(),
        "navy_ledger_changes": crate::navy_ledger_changes::status(),
        "air_base_changes": crate::air_base_changes::status(),
        "air_record_changes": crate::air_record_changes::status(),
        "rail_changes": crate::rail_changes::status(),
        "territory_changes": crate::territory_changes::status(),
        "army_stat_changes": crate::army_stat_changes::status(),
        "country_input_changes": crate::country_input_changes::status(),
        "stability_batches": crate::extended::country_stability::status(),
        "raid_changes": crate::raid_changes::status(),
        "naval_tree_changes": crate::naval_tree_changes::status(),
        "raid_intel_changes": crate::extended::raid_intel_carry::status(),
        "unit_leader_changes": crate::unit_leader_changes::status(),
        "census_changes": crate::census_changes::status(),
        "unit_leader_trait_changes": crate::unit_leader_trait_changes::status(),
        "world_input_changes": crate::world_input_changes::status(),
        "retreat_ban_changes": crate::retreat_ban_changes::status(),
        "local_subunit_changes": crate::local_subunit_changes::status(),
        "province_unit_changes": crate::province_unit_changes::status(),
        "template_changes": crate::template_changes::status(),
        "ship_model_changes": crate::ship_model_changes::status(),
        "air_base_carry_changes": crate::air_base_carry_changes::status(),
        "area_changes": crate::area_changes::status(),
        "province_changes": crate::province_changes::status(),
        "character_changes": crate::character_changes::status(),
        "opinion_changes": crate::opinion_changes::status(),
        "equipment_destruction": crate::equipment_destruction::status(),
        "naval_result_sources": crate::naval_result_sources::status(),
        "experience_gains": crate::experience_gains::status(),
        "event_feed": crate::event_feed_observer::status(),
        "world_lifetime": crate::world_lifetime::status(),
        "operation_outcomes": crate::operation_outcomes::status(),
        "operative_capture": crate::extended::intel::operative_capture_diagnostics(),
        "agency_upgrade_capture": crate::extended::intel::agency_upgrade_capture_diagnostics(),
        "country_scale_demand": crate::country_scale::demand_diagnostics(),
        "air_losses": crate::air_losses::status(),
        "building_bombing": crate::building_bombing::status(),
        "nuclear_strikes": crate::nuclear_strikes::status(),
        "line_output": crate::line_output::status(),
        "fuel_awards": crate::fuel_awards::status(),
        "politics_capture": crate::politics_changes::status(),
        "peace_settlements": crate::peace_settlements::status(),
        "land_events": crate::land_events::status(),
        "combat_log_prune": crate::combat_log_prune::status(),
        "construction_ic": crate::construction_ic::status(),
        "logistics_balance": crate::logistics_balance::status(),
        "batch_fallbacks": map(engine::batch_fallback::counts()),
        "replay_format": replay::FORMAT_VERSION,
        "sample_boundary": sampling::SAMPLE_BOUNDARY.name(),
    });
    #[cfg(feature = "callback_profile")]
    let base = base.join(status!({
        "callback_profile": crate::callback_profile::status(),
        "fuel_award_tags": crate::fuel_awards::tag_profile(),
        "fuel_award_tags_scope": "diagnostic: every fuel tap callback since attach by tap (BEGIN, END, the five land receipts, naval, air, reset) as [calls, ns], TSC-timed inside the callback, so the tap bridge and relay lie outside",
    }));
    #[cfg(feature = "worker_sampler")]
    let base = base.join(status!({"worker_sampler": crate::worker_sampler::status()}));
    #[cfg(feature = "advisor_profile")]
    let base = base.join(
        status!({"advisor_profile": crate::extended::characters::advisors::profile::status()}),
    );
    let shared_status = shared.map(|shared| {
        let models = {
            let guard = shared.template_models.lock();
            guard.map(|stats| *stats).unwrap_or_default()
        };
        let load = |slot: &std::sync::atomic::AtomicU64| slot.load(Ordering::Relaxed);
        status!({
            "game_score_enabled": shared.game_score_enabled,
            "fixed_layout": crate::fixed_layout::status(shared.fixed_layout.get()),
            "naval_result_details": crate::extended::naval_result_details::status(),
            "land_reserve_speed": crate::extended::combat_details::status(),
            "native_tick_work": {
                "elapsed_ns": shared.native_tick_ns.load(Ordering::Relaxed),
                "calls": shared.native_tick_calls.load(Ordering::Relaxed),
                "scope": "asynchronously sampled cumulative original hourly-tick (trampoline) wall time since attach, including engine waits; excludes the entry read, the collect wait and all acquisition; not a recorder-free baseline or CPU time",
            },
            "template_models": {
                "hours": models.hours,
                "rows": models.rows,
                "first_reason": models.first_reason,
                "by_reason": map(models.reasons.into_iter().filter(|(_, rows)| *rows != 0)),
                "other_reasons": models.other,
                "missed_hours": shared.template_models_missed.load(Ordering::Relaxed),
                "scope": "cumulative DivisionTemplateIcons rows since attach whose 3D-model inputs could not be read: that row's model fields are empty and its counter fields stand; the viewer draws no model for its divisions. missed_hours counts hours not folded because this status write held the counters",
            },
            "background_priority": background_priority_status(),
            "collector_thread_cpu": crate::thread_cpu::status(
                shared
                    .acquisition
                    .get()
                    .map(|acquisition| acquisition.calibration().executor.thread_ids)
                    .unwrap_or_default()
                    .ids(),
            ),
            "refresh_requests_accepted": REFRESH_ACCEPTED.load(Ordering::Relaxed),
            "presentation_phases": {
                "hours": load(&shared.presentation.hours),
                "phase_ns": crate::status::map(
                    crate::extended::division_presentation::PHASE_NAMES
                        .into_iter()
                        .zip(shared.presentation.phase_ns.each_ref().map(load))
                ),
                "scope": "cumulative Presentation worker wall since attach inside the DivisionStatus acquisition span, split by pass; overlaps other jobs and the game-thread block and is not added to it",
            },
        })
    });
    let acquisition = shared.and_then(|shared| shared.acquisition.get().map(|acquisition| {
            let world = acquisition.world_calibration();
            let calibration = acquisition.calibration();
            let block_ns = shared.collect_wait_ns.load(Ordering::Relaxed);
            let block_calls = shared.native_tick_calls.load(Ordering::Relaxed);
            let phase_calls = shared.phase_calls.load(Ordering::Relaxed);
            let phase_mean = move |ns: u64| ns.checked_div(phase_calls).unwrap_or(0);
            let join_wait = shared.join_wait.each_ref().map(|wait| wait.load(Ordering::Relaxed));
            let join_wait_by_role = map(crate::acquisition_jobs::ROLES.into_iter()
                .zip(join_wait).filter(|(_, wait)| *wait > 0)
                .map(move |(role, wait)| (DebugValue(role), phase_mean(wait))));
            let segments = shared.phase_segments.each_ref().map(|segment| phase_mean(segment.load(Ordering::Relaxed)));
            let role_timeline = map(crate::acquisition_jobs::ROLES.into_iter()
                .zip(shared.role_timeline.each_ref())
                .map(|(role, timeline)| (role, timeline.means(), timeline.slow_joins()))
                .filter(|(_, (jobs, _), _)| *jobs > 0)
                .map(move |(role, (jobs, means), slow)| {
                    let ns = means.map(|ticks| shared.ns(i64::try_from(ticks).unwrap_or(i64::MAX)));
                    (DebugValue(role), status!({
                        "jobs": jobs,
                        "queued_ns": ns[0], "body_begin_ns": ns[1], "body_end_ns": ns[2],
                        "join_begin_ns": ns[3], "join_end_ns": ns[4],
                        "joins_waiting_over_250us": slow[0], "joins_waiting_over_1ms": slow[1],
                        "joins_waiting_over_4ms": slow[2],
                    }))
                }));
            status!({ "acquisition_work": {
                "collect_wait_ns": block_ns,
                "calls": block_calls,
                "mean_ns": block_ns.checked_div(block_calls).unwrap_or(0),
                "buffer_wait_ns": shared.buffer_wait_ns.load(Ordering::Relaxed),
                "buffer_wait_calls": shared.buffer_wait_calls.load(Ordering::Relaxed),
                "buffer_wait_max_ns": shared.buffer_wait_max_ns.load(Ordering::Relaxed),
                "buffer_wait_scope": "game-thread wall waiting for one of the four reusable capture buffers; included in collect_wait_ns and successful callback_work, not additional acquisition work; cancellation during detach or worker failure retains the existing explicit loss accounting",
                "collect_scope": "cumulative game-thread wall from taking a reusable buffer through live acquisition at tick entry, before the trampoline, including buffer waits and failed or cancelled captures; successful-capture percentiles are in callback_work",
                "block_phases": {
                    "calls": phase_calls,
                    "setup_mean_ns": phase_mean(shared.phase_setup_ns.load(Ordering::Relaxed)),
                    "parallel_mean_ns": phase_mean(shared.phase_parallel_ns.load(Ordering::Relaxed)),
                    "parallel_inline_walk_mean_ns": phase_mean(shared.phase_inline_walk_ns.load(Ordering::Relaxed)),
                    "parallel_join_mean_ns": phase_mean(shared.phase_join_ns.load(Ordering::Relaxed)),
                    "segments_mean_ns": sequence(segments.into_iter()),
                    "segments_scope": sampling::SEGMENTS_SCOPE,
                    "join_wait_by_role_mean_ns": join_wait_by_role,
                    "role_timeline": role_timeline,
                    "role_timeline_scope": "per pool role, its collected jobs (jobs, cumulative) and the mean offset in ns from their acquisition body's entry of when each was queued (a held first-burst job is queued at its submission: its release only wakes idle workers, and a worker already running may claim it before), when a pool participant began and ended its body (over the jobs that ran on one), and when the game thread began and ended its join. A join that begins after the body ended waits only on collection; one that begins before it waits on the unfinished body. joins_waiting_over_250us, _1ms and _4ms count the role's joins whose wait reached each threshold, so a mean wait from a few long hours shows apart from a steady one",
                    "residual_mean_ns": phase_mean(shared.phase_residual_ns.load(Ordering::Relaxed)),
                },
                "block_phases_scope": "disjoint game-thread wall means summing to the recorded acquisition body: setup = reusable-frame wait, frame reset and country authentication; parallel = shared input acquisition, request dispatch, result installation and sequential job joins; residual = completion bookkeeping before decode enqueue. parallel_inline_walk is a subset of parallel outside joins, measured over the collection span; parallel_join counts sequential completion spans and may include execution of the exact awaited job. Their sum leaves dispatch and other work outside that span. Worker durations overlap this game-thread wall and must not be added. Reusable-frame wait is separately reported but already included in setup, collect_wait_ns and callback_work",
                "game_thread": game_thread_status(shared, phase_calls),
                "panics": shared.acquisition_panics.load(Ordering::Relaxed),
                "workers": calibration,
                "dispatch_ns": shared.acquisition_dispatch_ns.load(Ordering::Relaxed),
                "join_wait_ns": shared.acquisition_wait_ns.load(Ordering::Relaxed),
                "join_scope": if cfg!(feature = "executor_join_help") {
                    "sequential game-thread completion spans including bookkeeping, exact-target caller execution of an eligible awaited queued job and helped caller-eligible queued jobs while the awaited job runs; help uses try_lock and skips contention, requires history for both roles and the awaited role mean minus elapsed body time at least twice the candidate role decaying maximum (max(sample, old - old/16)); each join has at most four attempts and spends at most half its first predicted remainder in cumulative candidate maximum costs; bodies have no preemption so this bound is a heuristic"
                } else {
                    "sequential game-thread completion spans including bookkeeping and possible exact-target caller execution when the awaited request is still queued"
                },
                "join_costs": join_costs_status(shared, phase_calls),
                "join_costs_scope": "per pool role, over its collected published jobs (published): those already published when their join began (ready_at_join); per acquired hour, the collector's first queue-lock wait only when its initial lock-free collection CAS failed (finish_lock_ns_per_hour; zero on the fast path), the stall from the later of join begin and atomic publication to collection (collect_stall_ns_per_hour: atomic handoff, address wake and rescheduling; optional helped bodies can also defer collection) and completion bookkeeping from body end to atomic publication (publish_wait_ns_per_hour; failures also stop/cancel under the queue mutex); and joins whose stall reached 0.25 and 1 ms (stalls_over_250us, stalls_over_1ms). With join help disabled, a role's join wait less its collect stall is time waiting on its unfinished body",
                "table_costs_overlap": true,
                "pool_bytes_scope": "reserved collector storage; executor bytes count its actual Arc shared allocation, payload cells, atomic words, metadata and ready-queue capacities, thread-id and join-handle capacities, boxed runner and enabled help history arrays",
                "country_snapshot_scope": "one immutable country authentication snapshot shared with acquisition workers; production-rest acquires fresh country inputs for its discovery builder and validates its current read plan; payload counted once in pool_bytes; preparation charged once to Countries; worker calibration reports the borrowed snapshot status",
                "world_country_cache": world.country_cache,
                "world_acquisition_ns": shared.ns(i64::try_from(world.acquisition_ticks).unwrap_or(i64::MAX)),
                "world_last_error": world.last_error,
                "world_cost_scope": "hourly owned World (production and construction rests) and Economy (economy rest) jobs together on any acquisition participant: world_acquisition_ns sums both, world_country_cache is observed once an hour by the Economy job, world_last_error names the job; includes staging attempts; shared country preparation charged once to Countries; typed fallback table costs exclude discarded staging attempts; overlaps other jobs and acquisition work; panicked/stopped worker cost is unavailable",
            } })
        }));
    DecodeStatus {
        base,
        shared: shared_status,
        acquisition,
    }
}

/// Each pool role's collection costs after its jobs finished
/// ([`sampling::JoinCosts`]), per acquired hour; roles with no published job
/// are left out.
fn join_costs_status(shared: &sampling::Shared, hours: u64) -> impl serde::Serialize + '_ {
    use crate::status::{DebugValue, map, status};
    let per_hour = move |ticks: u64| {
        shared
            .ns(i64::try_from(ticks).unwrap_or(i64::MAX))
            .checked_div(hours)
            .unwrap_or(0)
    };
    map(crate::acquisition_jobs::ROLES
        .into_iter()
        .zip(shared.join_costs.each_ref())
        .map(|(role, costs)| (role, costs.totals()))
        .filter(|(_, (published, ..))| *published > 0)
        .map(move |(role, (published, ready, ticks, stalls))| {
            (
                DebugValue(role),
                status!({
                    "published": published,
                    "ready_at_join": ready,
                    "finish_lock_ns_per_hour": per_hour(ticks[0]),
                    "collect_stall_ns_per_hour": per_hour(ticks[1]),
                    "publish_wait_ns_per_hour": per_hour(ticks[2]),
                    "stalls_over_250us": stalls[0],
                    "stalls_over_1ms": stalls[1],
                }),
            )
        }))
}

/// The game thread's own work per acquired hour, by kind (QPC wall means over
/// `hours`): what the parked hour costs besides waiting for workers.
fn game_thread_status(shared: &sampling::Shared, hours: u64) -> impl serde::Serialize + '_ {
    let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
    let phases = &shared.game_thread;
    let mean = move |ns: u64| ns.checked_div(hours).unwrap_or(0);
    let per_hour = move |count: u64| {
        if hours == 0 {
            0.0
        } else {
            count as f64 / hours as f64
        }
    };
    crate::status::status!({
        "hours": hours,
        "frame_reset_mean_ns": mean(load(&phases.frame_reset_ns)),
        "observer_boundaries_mean_ns": mean(load(&phases.boundaries_ns)),
        "country_snapshot_mean_ns": mean(load(&phases.country_snapshot_ns)),
        "dispatch_mean_ns": mean(load(&shared.acquisition_dispatch_ns)),
        "dispatched_jobs_per_hour": per_hour(load(&phases.dispatched_jobs)),
        "release_mean_ns": mean(load(&phases.release_ns)),
        "releases_per_hour": per_hour(load(&phases.releases)),
        "released_jobs_per_hour": per_hour(load(&phases.released_jobs)),
        "join_wait_mean_ns": mean(load(&shared.acquisition_wait_ns)),
        "map_sample_mean_ns": mean(load(&phases.map_sample_ns)),
        "table_assembly_mean_ns": mean(load(&phases.table_assembly_ns)),
        "scope": "game-thread QPC wall per acquired hour: frame reset (reused frame verdicts and decode slot), both observer boundary jobs, the country snapshot, job submission (dispatch, the jobs submitted and the wakes releasing held bursts), join waits for workers (including caller execution of a still-queued awaited job), the map sample's typed walk and table assembly after the joins. The rest of block_phases (installs and request building) is what these leave of setup and parallel; worker time overlaps and is not here",
    })
}

#[cfg(test)]
fn add_decode_status(status: &mut serde_json::Value) {
    if !status.is_object() {
        *status = serde_json::json!({});
    }
    let additions = serde_json::to_value(decode_status()).unwrap();
    status
        .as_object_mut()
        .unwrap()
        .extend(additions.as_object().unwrap().clone());
}

#[cfg(test)]
pub(crate) fn status_allocation_failure_exercise() -> io::Result<()> {
    use crate::allocation_failure_tests::with_allocations_denied;
    use std::io::Write;
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let shared = Shared::new(0, 0, 1);
    let pool = acquisition::Pool::start(1, acquisition::MAX_JOBS)
        .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
    assert!(shared.acquisition.set(pool).is_ok());
    shared.phase_calls.store(3, Ordering::Relaxed);
    shared.join_wait[0].store(42, Ordering::Relaxed);
    {
        let mut models = shared.template_models.lock().unwrap();
        models.reasons[0] = ("synthetic model reason", 2);
    }
    let work = with_allocations_denied(WorkerWork::new);
    let decode = with_allocations_denied(|| decode_status_from(Some(&shared)));
    let worker = with_allocations_denied(|| work.status(0, 1))?;
    let value = crate::status::status!({"decode": decode, "worker": worker});
    let mut count = Count(0);
    with_allocations_denied(|| serde_json::to_writer(&mut count, &value))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut bytes = Vec::new();
    engine::allocation::reserve_exact(&mut bytes, count.0)?;
    // Mutating the source between passes must not change the owned snapshot.
    shared.phase_calls.store(30, Ordering::Relaxed);
    shared.join_wait[0].store(420_000, Ordering::Relaxed);
    shared.template_models.try_lock().unwrap().reasons[0].1 = 200_000;
    with_allocations_denied(|| serde_json::to_writer(&mut bytes, &value))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    assert_eq!(bytes.len(), count.0);
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        status["decode"]["acquisition_work"]["block_phases"]["calls"],
        3
    );
    assert_eq!(
        status["decode"]["template_models"]["by_reason"]["synthetic model reason"],
        2
    );
    assert!(status["worker"]["cpu_ns_per_recorded_hour"].is_null());
    assert_eq!(
        status["decode"]["raid_intel_changes"]["configured"],
        cfg!(feature = "raid_intel_changes")
    );
    // Every hook family is published; thread CPU includes the started worker.
    let families = status["decode"]["hook_timing"]["families"].as_object();
    assert_eq!(
        families.map(serde_json::Map::len),
        Some(crate::hook_timing::FAMILY_COUNT)
    );
    let threads = status["decode"]["collector_thread_cpu"]["threads"].as_array();
    assert!(threads.is_some_and(|threads| {
        threads
            .iter()
            .any(|thread| thread["role"] == "acquisition" && thread["available"] == true)
    }));
    drop(value);
    let absent = with_allocations_denied(|| decode_status_from(None));
    let absent = serde_json::to_value(absent).unwrap();
    assert!(absent.get("fixed_layout").is_none());
    assert!(absent.get("acquisition_work").is_none());
    assert!(absent.get("collector_thread_cpu").is_none());
    assert!(absent["hook_timing"]["families"]["hourly"].is_object());
    shared
        .acquisition
        .get()
        .unwrap()
        .shutdown()
        .map_err(|_| io::Error::from(io::ErrorKind::Other))
}

// Both quiescence checks use the same predicate, including while the threads
// are suspended: a writer may be in its native original outside DLL code.
pub(crate) fn observers_active() -> bool {
    RestoreFamily::ALL
        .into_iter()
        .any(|family| family.observer_state().0)
}

const DETACH_BUDGET: Duration = Duration::from_secs(5);

fn observer_detach_refusal(
    elapsed: Duration,
    mut active: impl FnMut(RestoreFamily) -> bool,
) -> Option<&'static str> {
    if elapsed < DETACH_BUDGET {
        return None;
    }
    RestoreFamily::ALL
        .into_iter()
        .find(|family| active(*family))
        .map(|family| family.observer_state().1)
}

fn mutation_detach_refusal(
    elapsed: Duration,
    active: impl FnMut(RestoreFamily) -> bool,
) -> &'static str {
    // A busy patch or a deferred world entry need not have a native flight.
    observer_detach_refusal(elapsed, active)
        .unwrap_or("collector mutation entry restoration refused; collector retained")
}

// Native retry phases share the detach start and budget. Windows calls and
// owned worker joins remain separate obligations; timeout never permits release.

fn detach_retry_delay(elapsed: Duration, poll: Duration) -> Option<Duration> {
    DETACH_BUDGET
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| remaining.min(poll))
}

#[derive(Clone, Copy)]
enum QuietPhase {
    Observers,
    Release,
}
impl QuietPhase {
    fn entries_restored(self, restored: &[bool; RestoreFamily::ALL.len()]) -> bool {
        match self {
            Self::Observers | Self::Release => restored.iter().all(|restored| *restored),
        }
    }
    fn refusal(self) -> &'static str {
        match self {
            Self::Observers => "collector observer quiescence refused; collector retained",
            Self::Release => "collector final quiescence refused; collector retained",
        }
    }
}

fn wait_quiet(
    ranges: &[std::ops::Range<usize>],
    image_size: usize,
    excluded_threads: &[u32],
    phase: QuietPhase,
    restored: &[bool; RestoreFamily::ALL.len()],
    started: Instant,
) -> Result<(), &'static str> {
    if image_size == 0 || !phase.entries_restored(restored) {
        return Err("collector quiescence entries or image range unproved; collector retained");
    }
    crate::air_losses::mark_stranded();
    crate::building_bombing::mark_stranded();
    crate::nuclear_strikes::mark_stranded();
    loop {
        if let Some(reason) =
            observer_detach_refusal(started.elapsed(), |family| family.observer_state().0)
        {
            return Err(reason);
        }
        if !observers_active()
            && let Ok(mut threads) = Threads::open_excluding(excluded_threads)
        {
            // Snapshot/open may allocate. Only scalar counts and current IPs
            // are consulted after suspension; no owner drops in this window.
            let result = threads.suspend().and_then(|()| {
                if observers_active() {
                    Ok(false)
                } else {
                    threads.contains_ip(ranges).map(|present| !present)
                }
            });
            if threads.resume().is_err() {
                // Shared Threads::resume returns a failure only after every
                // owned suspension is actually resumed or proved exited.
                // Error history still refuses release; diagnostics now run
                // outside the suspended interval and all roots are retained.
                return Err("collector quiet thread resume refused; collector retained");
            }
            if result == Ok(true) {
                return Ok(());
            }
        }
        let Some(delay) = detach_retry_delay(started.elapsed(), Duration::from_millis(1)) else {
            return Err(phase.refusal());
        };
        std::thread::sleep(delay);
    }
}

#[cfg(test)]
mod detach_policy_tests {
    use super::*;

    #[test]
    fn restoration_refusal_still_visits_later_families_and_keeps_progress() {
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut visited = Vec::new();
        let mut previous_errors = [false; RestoreFamily::ALL.len()];
        assert_eq!(
            restore_round(
                &mut restored,
                &mut previous_errors,
                || false,
                |family| {
                    visited.push(family);
                    match family {
                        RestoreFamily::Hourly => Err("bytes differ"),
                        RestoreFamily::Portrait => Ok(false), // pending cache flush
                        _ => Ok(true),
                    }
                }
            ),
            Ok(false)
        );
        assert_eq!(
            visited,
            RestoreFamily::ALL[..RestoreFamily::WorldLifetime as usize]
        );
        assert!(!restored[RestoreFamily::Hourly as usize]);
        assert!(!restored[RestoreFamily::Portrait as usize]);
        assert!(restored[RestoreFamily::History as usize]);
        visited.clear();
        assert_eq!(
            restore_round(
                &mut restored,
                &mut previous_errors,
                || false,
                |family| {
                    visited.push(family);
                    Ok(true)
                }
            ),
            Ok(true)
        );
        assert_eq!(
            visited,
            [
                RestoreFamily::Hourly,
                RestoreFamily::Portrait,
                RestoreFamily::WorldLifetime
            ]
        );
    }

    #[test]
    fn detach_retry_budget_does_not_restart_for_last_short_poll() {
        let poll = Duration::from_millis(100);
        assert_eq!(detach_retry_delay(Duration::ZERO, poll), Some(poll));
        let last = Duration::from_nanos(1);
        assert_eq!(detach_retry_delay(DETACH_BUDGET - last, poll), Some(last));
        assert_eq!(detach_retry_delay(DETACH_BUDGET, poll), None);
        assert_eq!(detach_retry_delay(DETACH_BUDGET + last, poll), None);
    }

    #[test]
    fn later_quiet_phases_require_every_current_entry_restored() {
        let mut restored = [false; RestoreFamily::ALL.len()];
        restored[RestoreFamily::Hourly as usize] = true;
        assert!(!QuietPhase::Observers.entries_restored(&restored));
        assert!(!QuietPhase::Release.entries_restored(&restored));
        restored.fill(true);
        assert!(QuietPhase::Observers.entries_restored(&restored));
        assert!(QuietPhase::Release.entries_restored(&restored));
    }
}

#[cfg(test)]
mod detach_tests {
    use super::*;

    #[test]
    fn expired_detach_budget_does_not_restart_or_reset_acquisition_leases() {
        let started = Instant::now() - DETACH_BUDGET;
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut errors = [false; RestoreFamily::ALL.len()];
        restore_hourly(
            &mut restored,
            &mut errors,
            started,
            || Ok(false),
            || panic!("hourly restoration restarted the detach budget"),
        );
        assert!(!restored[RestoreFamily::Hourly as usize]);
        let admission = CaptureAdmission::new();
        let lease = admission.enter().unwrap();
        admission.close();
        assert_eq!(
            wait_acquisitions(&admission, started),
            Err("hourly acquisitions did not quiesce; collector retained"),
        );
        assert!(!admission.quiet());
        drop(lease);
        assert_eq!(wait_acquisitions(&admission, started), Ok(()));
    }

    #[test]
    fn world_signal_restores_only_after_other_hooks_and_native_returns_are_quiet() {
        let mut restored = [true; RestoreFamily::ALL.len()];
        restored[RestoreFamily::WorldLifetime as usize] = false;
        assert!(!world_restore_ready(&restored, true));
        assert!(world_restore_ready(&restored, false));
        restored[RestoreFamily::AirLosses as usize] = false;
        assert!(!world_restore_ready(&restored, false));
    }

    #[test]
    fn world_signal_restores_in_the_round_where_the_last_callback_returns() {
        let callbacks = CaptureAdmission::new();
        let mut native_return = Some(callbacks.enter().unwrap());
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut errors = [false; RestoreFamily::ALL.len()];
        let mut world_restores = 0;
        // The preceding restoration/return consumes the shared wait budget.
        // A quiet world must still get its first restoration attempt now.
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let result = restore_round(
            &mut restored,
            &mut errors,
            || !callbacks.quiet(),
            |family| match family {
                RestoreFamily::EventFeed => {
                    drop(native_return.take());
                    elapsed.set(DETACH_BUDGET);
                    Ok(true)
                }
                RestoreFamily::WorldLifetime => {
                    assert!(callbacks.quiet());
                    world_restores += 1;
                    Ok(true)
                }
                _ => Ok(true),
            },
        );
        assert!(callbacks.quiet());
        assert_eq!(
            detach_retry_delay(elapsed.get(), Duration::from_millis(100)),
            None
        );
        assert_eq!(result, Ok(true));
        assert_eq!(world_restores, 1);
        assert!(QuietPhase::Observers.entries_restored(&restored));
        assert!(QuietPhase::Release.entries_restored(&restored));
    }

    #[test]
    fn quiet_world_gets_its_first_restore_after_slow_predecessors() {
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut errors = [false; RestoreFamily::ALL.len()];
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let mut visited = Vec::new();
        assert_eq!(
            restore_round(
                &mut restored,
                &mut errors,
                || false,
                |family| {
                    visited.push(family);
                    elapsed.set(DETACH_BUDGET);
                    Ok(true)
                }
            ),
            Ok(true)
        );
        assert_eq!(visited, RestoreFamily::ALL);
        assert_eq!(
            detach_retry_delay(elapsed.get(), Duration::from_millis(100)),
            None
        );
        assert!(QuietPhase::Release.entries_restored(&restored));
    }

    #[test]
    fn world_signal_waits_for_a_native_return_after_its_entry_was_restored() {
        let callbacks = CaptureAdmission::new();
        let native_return = callbacks.enter().unwrap();
        let mut restored = [true; RestoreFamily::ALL.len()];
        restored[RestoreFamily::WorldLifetime as usize] = false;
        let mut errors = [false; RestoreFamily::ALL.len()];
        let world_restores = std::cell::Cell::new(0);
        let mut restore = |family| {
            assert_eq!(family, RestoreFamily::WorldLifetime);
            assert!(callbacks.quiet());
            world_restores.set(world_restores.get() + 1);
            Ok(true)
        };
        assert_eq!(
            restore_round(
                &mut restored,
                &mut errors,
                || !callbacks.quiet(),
                &mut restore
            ),
            Ok(false)
        );
        assert_eq!(world_restores.get(), 0);
        assert!(!QuietPhase::Release.entries_restored(&restored));
        assert!(!callbacks.quiet());
        drop(native_return);
        assert_eq!(
            restore_round(
                &mut restored,
                &mut errors,
                || !callbacks.quiet(),
                &mut restore
            ),
            Ok(true)
        );
        assert_eq!(world_restores.get(), 1);
    }

    #[test]
    fn every_observer_family_has_a_named_bounded_detach_refusal() {
        for family in RestoreFamily::ALL {
            let reason = family.observer_state().1;
            let active = |candidate| candidate == family;
            assert_eq!(observer_detach_refusal(DETACH_BUDGET, |_| false), None);
            assert_eq!(observer_detach_refusal(Duration::ZERO, active), None);
            assert_eq!(observer_detach_refusal(DETACH_BUDGET, active), Some(reason));
            assert!(reason.ends_with("collector retained"));
        }
    }

    #[test]
    fn expired_restoration_reports_callback_refusal_only_for_an_active_family() {
        let generic = "collector mutation entry restoration refused; collector retained";
        assert_eq!(mutation_detach_refusal(DETACH_BUDGET, |_| false), generic);
        for family in RestoreFamily::ALL {
            let active = |candidate| candidate == family;
            assert_eq!(mutation_detach_refusal(Duration::ZERO, active), generic);
            assert_eq!(
                mutation_detach_refusal(DETACH_BUDGET, active),
                family.observer_state().1
            );
        }
    }
}

/// Throttled observation of the live country registry for buffer sizing.
/// Header words only; nothing observed here is recorded.
#[derive(Default)]
struct RegistryWatch {
    last: Option<Instant>,
    slots: usize,
    regions: usize,
    last_intel: Option<Instant>,
    intel_rows: usize,
    last_ace_names: Option<Instant>,
    ace_names: crate::country_scale::AceNamesDemand,
    last_air_wing_names: Option<Instant>,
    air_wing_names: usize,
    last_railway_gun_names: Option<Instant>,
    railway_gun_names: engine::railway_gun_counter_inputs::NameDemand,
    last_map: Option<Instant>,
    last_presentation_census: Option<Instant>,
    presentation_census: engine::carmy::PresentationDemand,
    last_raids: Option<Instant>,
    raids: crate::country_scale::RaidArenas,
    last_characters: Option<Instant>,
    characters: crate::country_scale::CharacterDemand,
    last_saved_targets: Option<Instant>,
    saved_targets: engine::saved_event_targets::Demand,
    last_politics: Option<Instant>,
    politics: crate::country_scale::PoliticsDemand,
    last_reserved_visibility: Option<Instant>,
    reserved_visibility: usize,
    last_lines: Option<Instant>,
    last_logistics: Option<Instant>,
    last_game_score: Option<Instant>,
    last_combat_hints: Option<Instant>,
}

impl RegistryWatch {
    const INTERVAL: Duration = Duration::from_millis(50);
    /// The intel total walks every country's table header, so it is sampled
    /// less often than the three registry headers.
    const INTEL_INTERVAL: Duration = Duration::from_secs(1);

    fn poll_reserved_visibility(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<usize>, &'static str> {
        if self
            .last_reserved_visibility
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_reserved_visibility = Some(Instant::now());
        let Some(slots) = crate::country_scale::observe_reserved_visibility(memory, base)? else {
            return Ok(None);
        };
        Ok((slots > self.reserved_visibility).then(|| {
            self.reserved_visibility = slots;
            slots
        }))
    }

    fn poll_game_score(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Option<io::Result<engine::game_score_inputs::ArenaSizing>> {
        if self
            .last_game_score
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_game_score = Some(Instant::now());
        match crate::country_scale::observe_game_score(memory, base) {
            Ok(Some(observed)) => Some(Ok(observed)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    /// Both combat families can grow without a country registry change.
    /// Failed hints are offered again; only actual capture latches capacities.
    fn poll_combat_hints(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Option<CombatHints> {
        if self
            .last_combat_hints
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_combat_hints = Some(Instant::now());
        Some(CombatHints::observe(memory, base))
    }

    /// Full logistics owner acquisition is bounded like the other country
    /// roster polls. Load/failure checks in the callbacks remain immediate.
    fn poll_logistics(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> io::Result<Option<usize>> {
        if self
            .last_logistics
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_logistics = Some(Instant::now());
        crate::logistics_balance::prepare(memory, base).map(Some)
    }

    /// Line rosters can grow without the country registry changing. Repeat
    /// the offer even without growth so a failed allocation can be retried.
    fn poll_lines(&mut self, memory: &impl engine::Memory, base: usize) -> Option<usize> {
        if self
            .last_lines
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_lines = Some(Instant::now());
        crate::country_scale::observe_production_output(memory, base)
    }

    /// The country slots and strategic regions when either is larger than
    /// any seen before, at most every `INTERVAL`.
    fn poll(&mut self, memory: &impl engine::Memory, base: usize) -> Option<(usize, usize)> {
        if self
            .last
            .is_some_and(|last| last.elapsed() < Self::INTERVAL)
        {
            return None;
        }
        self.last = Some(Instant::now());
        let slots = crate::country_scale::observe(memory, base)?;
        let regions = strategic_regions(memory, base);
        (slots > self.slots || regions > self.regions).then(|| {
            self.slots = self.slots.max(slots);
            self.regions = self.regions.max(regions);
            (self.slots, self.regions)
        })
    }

    /// A larger dense CountryIntel total than any seen, at most every
    /// `INTEL_INTERVAL`. Intel vectors fill after a save's registry does.
    fn poll_intel(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<usize>, &'static str> {
        if self
            .last_intel
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_intel = Some(Instant::now());
        let Some(rows) = crate::country_scale::observe_country_intel(memory, base)? else {
            return Ok(None);
        };
        Ok((rows > self.intel_rows).then(|| {
            self.intel_rows = rows;
            rows
        }))
    }

    fn poll_ace_names(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<crate::country_scale::AceNamesDemand>, &'static str> {
        if self
            .last_ace_names
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_ace_names = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_ace_names(memory, base)? else {
            return Ok(None);
        };
        let before = self.ace_names;
        self.ace_names.aces = self.ace_names.aces.max(demand.aces);
        self.ace_names.ace_bytes = self.ace_names.ace_bytes.max(demand.ace_bytes);
        Ok((self.ace_names != before).then_some(self.ace_names))
    }

    fn poll_air_wing_names(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<usize>, &'static str> {
        if self
            .last_air_wing_names
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_air_wing_names = Some(Instant::now());
        let Some(bytes) = crate::country_scale::observe_air_wing_names(memory, base)? else {
            return Ok(None);
        };
        Ok((bytes > self.air_wing_names).then(|| {
            self.air_wing_names = bytes;
            bytes
        }))
    }

    fn poll_railway_gun_names(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<engine::railway_gun_counter_inputs::NameDemand>, &'static str> {
        if self
            .last_railway_gun_names
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_railway_gun_names = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_railway_gun_names(memory, base)? else {
            return Ok(None);
        };
        Ok((demand.rows > self.railway_gun_names.rows
            || demand.bytes > self.railway_gun_names.bytes
            || demand.named_bonuses > self.railway_gun_names.named_bonuses
            || demand.supply_origins > self.railway_gun_names.supply_origins)
            .then(|| {
                self.railway_gun_names.rows = self.railway_gun_names.rows.max(demand.rows);
                self.railway_gun_names.bytes = self.railway_gun_names.bytes.max(demand.bytes);
                self.railway_gun_names.named_bonuses = self
                    .railway_gun_names
                    .named_bonuses
                    .max(demand.named_bonuses);
                self.railway_gun_names.supply_origins = self
                    .railway_gun_names
                    .supply_origins
                    .max(demand.supply_origins);
                self.railway_gun_names
            }))
    }

    /// Moving-owner counts, map headers and the physical navy pass can change
    /// independently of the country census. Failed hint components are retried
    /// independently; the applied capture capacities retain every earlier
    /// readable floor.
    fn poll_map(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<crate::country_scale::MapDemand>, &'static str> {
        if self
            .last_map
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_map = Some(Instant::now());
        crate::country_scale::observe_map(memory, base)
    }

    fn poll_presentation_census(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<engine::carmy::PresentationDemand>, &'static str> {
        if self
            .last_presentation_census
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_presentation_census = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_presentation_census(memory, base)? else {
            return Ok(None);
        };
        let raised = self.presentation_census.max(demand);
        if raised == self.presentation_census && demand.failure.is_none() {
            return Ok(None);
        }
        self.presentation_census = raised;
        Ok(Some(raised))
    }

    /// Larger raid arena demand than any seen, at most every
    /// `INTEL_INTERVAL`: the path memo, raid parents' type lists and convoy
    /// reservations fill after a save's registry does; a full hour reads all.
    fn poll_raids(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<crate::country_scale::RaidArenas>, &'static str> {
        if self
            .last_raids
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_raids = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_raid_arenas(memory, base)? else {
            return Ok(None);
        };
        let before = self.raids;
        self.raids = crate::country_scale::RaidArenas {
            type_tokens: before.type_tokens.max(demand.type_tokens),
            reservation_words: before.reservation_words.max(demand.reservation_words),
            path_rows: before.path_rows.max(demand.path_rows),
            facility_rows: before.facility_rows.max(demand.facility_rows),
            air_regions: before.air_regions.max(demand.air_regions),
            counteroperatives: before.counteroperatives.max(demand.counteroperatives),
            failures: demand.failures,
        };
        Ok(
            (self.raids != before || demand.failures.into_iter().any(|reason| reason.is_some()))
                .then_some(self.raids),
        )
    }

    /// Larger character or script flag demand than any seen, at most every
    /// `INTEL_INTERVAL`: a save's roster, template files and flags fill after
    /// its registry does, and a full hour must read them all.
    fn poll_characters(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<crate::country_scale::CharacterDemand>, &'static str> {
        if self
            .last_characters
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_characters = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_characters(memory, base)? else {
            return Ok(None);
        };
        let seen = &mut self.characters;
        let variables = seen.variables.max(demand.variables);
        let larger = demand.characters.rows > seen.characters.rows
            || demand.characters.bytes > seen.characters.bytes
            || demand.unit_leader_bytes > seen.unit_leader_bytes
            || demand.flags > seen.flags
            || variables != seen.variables;
        Ok(larger.then(|| {
            seen.characters.rows = seen.characters.rows.max(demand.characters.rows);
            seen.characters.bytes = seen.characters.bytes.max(demand.characters.bytes);
            seen.unit_leader_bytes = seen.unit_leader_bytes.max(demand.unit_leader_bytes);
            seen.flags = seen.flags.max(demand.flags);
            seen.variables = variables;
            *seen
        }))
    }

    /// Largest live saved-target row/name demand, at most once per second.
    /// Load and script growth are presized off-thread before a full capture.
    fn poll_saved_targets(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<engine::saved_event_targets::Demand>, &'static str> {
        if self
            .last_saved_targets
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_saved_targets = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_saved_targets(memory, base)? else {
            return Ok(None);
        };
        let raised = self.saved_targets.max(demand);
        if raised == self.saved_targets {
            return Ok(None);
        }
        self.saved_targets = raised;
        Ok(Some(raised))
    }

    /// Larger political table demand than any seen, at most every
    /// `INTEL_INTERVAL`: a save's character records, parties, civil wars and
    /// advisor lists fill after its registry does, and a full hour must read
    /// them all.
    fn poll_politics(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Result<Option<crate::country_scale::PoliticsDemand>, &'static str> {
        if self
            .last_politics
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_politics = Some(Instant::now());
        let Some(demand) = crate::country_scale::observe_politics(memory, base)? else {
            return Ok(None);
        };
        let changed = demand.exceeds(&self.politics) || demand.failures().next().is_some();
        self.politics = self.politics.max(demand);
        Ok(changed.then_some(self.politics))
    }
}

/// The loaded content's live strategic-region count, zero when unreadable:
/// the naval-patrol history watches are sized from it, never from a vanilla
/// count.
fn strategic_regions(memory: &impl engine::Memory, base: usize) -> usize {
    engine::cgame_state::read_strategic_region_count(memory, base).map_or(0, |count| count as usize)
}

struct CombatHints {
    land: Option<engine::cland_combat::DetailDemand>,
    naval: Option<engine::naval_combat_demand::Demand>,
    failures: [Option<&'static str>; 2],
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct CombatHintState {
    land: Option<crate::extended::combat_details::Capacities>,
    naval: Option<crate::extended::naval_details::Capacities>,
}

struct SpoolState {
    sizing: crate::country_scale::Sizing,
    applied_combat_hints: [CombatHintState; sampling::BUFFERS],
}

impl CombatHints {
    fn observe(memory: &impl engine::Memory, base: usize) -> Self {
        let land = crate::country_scale::observe_land_details(memory, base);
        let naval = crate::country_scale::observe_naval_details(memory, base);
        Self {
            failures: [land.as_ref().err().copied(), naval.as_ref().err().copied()],
            land: land.ok().flatten(),
            naval: naval.ok().flatten(),
        }
    }

    fn pending(
        &self,
        capacities: &crate::extended::Capacities,
        applied: &CombatHintState,
    ) -> CombatHintState {
        let mut pending = CombatHintState::default();
        if let Some(demand) = self.land {
            let mut capacity = applied.land.unwrap_or(capacities.combat_details);
            capacity.raise_for_live_details(demand);
            if applied.land != Some(capacity) {
                pending.land = Some(capacity);
            }
        }
        if let Some(demand) = self.naval {
            let mut capacity = applied.naval.unwrap_or(capacities.naval_details);
            capacity.raise_for_live_details(demand);
            if applied.naval != Some(capacity) {
                pending.naval = Some(capacity);
            }
        }
        pending
    }
}

/// Hint reserves are nonterminal and never raise the ordinary capture-driven
/// capacities. A failed attempt retains existing storage and is retried later.
fn grow_combat_hints(
    shared: &Shared,
    capacities: &crate::extended::Capacities,
    hints: &CombatHints,
    applied: &mut [CombatHintState; sampling::BUFFERS],
) {
    let pending: [_; sampling::BUFFERS] =
        std::array::from_fn(|index| hints.pending(capacities, &applied[index]));
    if pending
        .iter()
        .all(|hint| hint.land.is_none() && hint.naval.is_none())
    {
        return;
    }
    for _ in 0..shared.free.len() {
        let Some(index) = shared.free.pop() else {
            break;
        };
        let hint = pending[index];
        if hint.land.is_none() && hint.naval.is_none() {
            shared.recycle_frame_prepared(index);
            continue;
        }
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let before = frame.allocated_bytes();
        let success = frame
            .extended
            .apply_combat_hints(hint.land.as_ref(), hint.naval.as_ref());
        if success[0] {
            applied[index].land = hint.land;
        }
        if success[1] {
            applied[index].naval = hint.naval;
        }
        shared.pool_bytes.fetch_add(
            frame.allocated_bytes().saturating_sub(before) as u64,
            Ordering::Relaxed,
        );
        shared.recycle_frame_prepared(index);
    }
}

/// Grow every idle frame to the current capacities under its queue lease.
fn grow_idle_frames(
    shared: &Shared,
    capacities: &mut [usize; 12],
    map_capacities: &mut crate::map_capture::MapCapacities,
    extended_capacities: &mut crate::extended::Capacities,
) {
    let live = unsafe { LiveMemory::new() };
    let _ = extended_capacities
        .combat_progress
        .presize(&live, shared.base);
    for _ in 0..shared.free.len() {
        let Some(idle_index) = shared.free.pop() else {
            break;
        };
        let idle = unsafe { &mut *shared.frames[idle_index].load(Ordering::Acquire) };
        let old_bytes = idle.allocated_bytes() + shared.country_intel.allocated_bytes();
        idle.grow(capacities);
        idle.map.grow(map_capacities);
        extended_capacities.raise_for_states(capacities[3]);
        idle.extended.grow(extended_capacities);
        let new_bytes = idle.allocated_bytes() + shared.country_intel.allocated_bytes();
        if new_bytes >= old_bytes {
            shared
                .pool_bytes
                .fetch_add((new_bytes - old_bytes) as u64, Ordering::Relaxed);
        } else {
            shared
                .pool_bytes
                .fetch_sub((old_bytes - new_bytes) as u64, Ordering::Relaxed);
        }
        shared.recycle_frame_prepared(idle_index);
    }
}

struct SpoolOutcome {
    result: Result<(), &'static str>,
    terminal_status: Vec<u8>,
    directories: Vec<PathBuf>,
}

#[derive(Clone, Copy, Default)]
struct TeardownStatus {
    restoration_error: Option<&'static str>,
    installation_error: Option<&'static str>,
}

#[derive(serde::Serialize)]
struct SpoolStatus<P, D, W> {
    #[serde(flatten)]
    processor: P,
    #[serde(flatten)]
    decode: D,
    state: &'static str,
    session: u64,
    process_start_time: u64,
    pool_bytes: u64,
    pool_buffers: usize,
    dropped_frames: u64,
    startup_unattributed_callbacks: u64,
    drain_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_bytes_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_callbacks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collector_pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recording_stopped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spool_closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    restoration_error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    installation_error: Option<&'static str>,
    status_writer: W,
    combat_sizing_hints: crate::country_scale::CombatHintStatus,
    clock_misses: u64,
    clock_advisory_misses: u64,
    clock_last_failure: Option<&'static str>,
}
#[allow(clippy::too_many_arguments)]
fn spool_status<'a>(
    processor: &'a Processor,
    shared: &'a Shared,
    session: &'a SessionStart,
    work: &'a WorkerWork,
    writer: &'a crate::status_writer::StatusHandle,
    state: &'static str,
    startup_unknown: u64,
    warning: Option<&'static str>,
    error: Option<&'static str>,
    drain_complete: bool,
    terminal: bool,
    teardown: TeardownStatus,
) -> io::Result<impl serde::Serialize + 'a> {
    let pinned = terminal && teardown.restoration_error.is_some();
    let (clock_misses, clock_advisory_misses, clock_last_failure) = shared.clock_status();
    Ok(SpoolStatus {
        processor: processor
            .status_with_work(work.status(processor.recorded_hours(), shared.frequency)?)?,
        decode: decode_status(),
        state: if pinned { "pinned" } else { state },
        session: session.id,
        process_start_time: session.process_start_time,
        pool_bytes: shared.pool_bytes.load(Ordering::Relaxed),
        pool_buffers: sampling::BUFFERS,
        dropped_frames: shared.dropped.load(Ordering::Relaxed),
        startup_unattributed_callbacks: startup_unknown,
        drain_complete,
        warning,
        error: error
            .or(teardown.restoration_error)
            .or(teardown.installation_error),
        original_bytes_verified: terminal.then_some(!pinned),
        active_callbacks: (terminal && !pinned).then_some(0),
        collector_pinned: terminal.then_some(pinned),
        recording_stopped: terminal.then_some(!shared.recording.load(Ordering::Acquire)),
        spool_closed: terminal.then_some(true),
        restoration_error: teardown.restoration_error,
        installation_error: teardown.installation_error,
        status_writer: writer.status(),
        combat_sizing_hints: crate::country_scale::combat_hint_status(),
        clock_misses,
        clock_advisory_misses,
        clock_last_failure,
    })
}

pub(crate) fn wait_completed_callback(
    base: &std::sync::atomic::AtomicI64,
    tail: &std::sync::atomic::AtomicI64,
) -> Result<i64, &'static str> {
    loop {
        if crate::allocation::failed() {
            return Err("collector allocation failed");
        }
        if let Some(ticks) = sampling::completed_callback_ticks(base, tail) {
            return Ok(ticks);
        }
        std::thread::yield_now();
    }
}

#[allow(clippy::too_many_arguments)]
fn spool_loop(
    shared: &Shared,
    config: &Config,
    mut session: SessionStart,
    mut processor: Processor,
    status_path: &Path,
    sizing: SizingThread,
    // Status and content files are written on the loader's writer thread:
    // this thread only hands snapshots over and never waits on status I/O,
    // nor joins that thread (#94).
    writer: crate::status_writer::StatusHandle,
    teardown_status: &OnceLock<TeardownStatus>,
) -> Result<SpoolOutcome, &'static str> {
    // This local drops before the owned processor/sizing parameters, so an
    // early return or unwind joins the sampler before either owner joins helpers.
    #[cfg(feature = "worker_sampler")]
    let _sampler_exit = crate::thread_cpu::StopOnThreadExit;
    let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
    let live = unsafe { LiveMemory::new() };
    let mut last_warning = None;
    let mut drain_result = Ok(());
    let mut startup_unknown = 0_u64;
    let mut work = WorkerWork::new();
    let mut reported = Instant::now() - Duration::from_secs(1);
    let mut priority = OwnPriority::new();
    let mut refresh_requests = RefreshRequests::default();
    // Wake-ups replace polling: decode publication, a parked clock
    // preparation and the stop request unpark this thread.
    let _ = shared.spool_waker.set(std::thread::current());
    let phases = &crate::spool_phases::SPOOL;
    loop {
        if crate::allocation::failed() {
            return Err("collector allocation failed");
        }
        let span = phases.span(Phase::Observers);
        shared
            .prepare_clock_capture(&live)
            .map_err(|_| "clock preparation allocation failed")?;
        span.finish();
        priority.follow(shared.buffers_backlogged());
        while let Some(index) = shared.ready.pop() {
            if crate::allocation::failed() {
                return Err("collector allocation failed");
            }
            priority.follow(shared.buffers_backlogged());
            let span = phases.span(Phase::Receive);
            let service_started = sampling::qpc();
            work.queue_peak_observed = work.queue_peak_observed.max(shared.ready.len() + 1);
            let elapsed =
                wait_completed_callback(&shared.completed[index], &shared.callback_tail[index])?;
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.total_ns = shared.ns(elapsed);
            crate::allocation::set_context(crate::allocation::Context {
                campaign_id: frame
                    .clock
                    .as_ref()
                    .and_then(|clock| clock.campaign_id.as_bytes().try_into().ok()),
                branch: None,
                load_signal: frame
                    .clock
                    .as_ref()
                    .map(|clock| clock.play_session_started_at),
                failed_hour: frame.clock.as_ref().map(|clock| clock.hour),
            });
            // Only this frame is owned across conversion. Shared CountryIntel
            // growth is sampled under the capacity lock when it returns.
            let before_bytes = frame.allocated_bytes();
            if !shared.roster_ready.load(Ordering::Acquire) {
                // A readable entry clock publishes the roster before its
                // buffer. These early callbacks have no attributable
                // hour; retain their count until a session identity can be written.
                startup_unknown = startup_unknown
                    .saturating_add(1)
                    .saturating_add(frame.dropped_before.unknown_count)
                    .saturating_add(
                        frame
                            .dropped_before
                            .iter()
                            .map(|loss| loss.count)
                            .sum::<u64>(),
                    );
                shared.recycle_frame_prepared(index);
                work.buffer_service
                    .record_ticks(sampling::qpc() - service_started, shared.frequency);
                continue;
            }
            // Acquire above transfers the once-only roster from its callback
            // writer. Future callbacks see ready and never touch its allocation.
            if let Some(roster) = unsafe { (&mut *shared.roster.get()).take() } {
                shared
                    .pool_bytes
                    .fetch_sub(roster.allocated_bytes() as u64, Ordering::Relaxed);
                // Verification errors are recorded as Unverified, allowing an honest capture.
                roster
                    .verify(
                        &mut session.identity,
                        &config.expected_mounts,
                        &config.mod_descriptors,
                        config.content_order_note.as_deref(),
                    )
                    .map_err(|e| error_reason(e, "content verification failed"))?;
                let diagnostic =
                    roster.diagnostics(&config.expected_mounts, &session.identity.verification);
                writer
                    .publish(&config.output_dir.join("content.json"), diagnostic)
                    .map_err(|e| error_reason(e, "content status allocation failed"))?;
                processor.set_verified_session(
                    session
                        .try_clone()
                        .map_err(|e| error_reason(e, "session allocation failed"))?,
                );
            }
            // Campaign directories can still fail to open. Keep the buffer and
            // preceding witnesses until those operations succeed. History is ready.
            while let Err(error) = processor.prepare(frame) {
                crate::allocation::observe(&error);
                if crate::allocation::failed() {
                    return Err("collector allocation failed");
                }
                if error.kind() == io::ErrorKind::InvalidData {
                    // An immutable history or identity refusal cannot recover by
                    // retaining this buffer. Ending the worker requests restore
                    // through the loader's JoinHandle check and preserves the
                    // precise error in its terminal status.
                    let _ = processor.detach();
                    return Err("campaign recording refused; observations remain unwritten");
                }
                last_warning = Some(error_reason(error, "collector spool I/O failed"));
                writer
                    .publish(
                        status_path,
                        crate::status::status!({
                            "state":"campaign spool unavailable", "session":session.id,
                            "warning":last_warning, "drain_complete":false,
                            "dropped_frames":shared.dropped.load(Ordering::Relaxed),
                        }),
                    )
                    .map_err(|e| error_reason(e, "spool status allocation failed"))?;
                if shared.stop_spool.load(Ordering::Acquire) || shared.free.is_empty() {
                    // A persistent open failure cannot release a frame. Once
                    // all frames are occupied, end the worker so the loader
                    // cancels the callback wait and restores the hook.
                    let _ = processor.detach();
                    return Err("cannot open campaign spool; observations remain unwritten");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if startup_unknown != 0 {
                processor.unknown_loss(std::mem::take(&mut startup_unknown));
            }
            if frame.dropped_before.unknown_count != 0 {
                processor.unknown_loss(frame.dropped_before.unknown_count);
            }
            for loss in frame.dropped_before.iter() {
                if let Err(error) = processor.lost(loss) {
                    last_warning = Some(error_reason(error, "collector spool I/O failed"));
                }
            }
            span.finish();
            // Conversion owns every value needed by encoding and diagnostics.
            // Return the capture storage before potentially slow output, except
            // when a retry must be published before that storage can be reused.
            processor.set_backlogged(shared.buffers_backlogged());
            let pending_write = match processor.prepare_frame(frame) {
                Ok(prepared) if prepared.allows_early_recycle() => Some(prepared),
                Ok(prepared) => {
                    if let Err(error) = processor.write_prepared(prepared) {
                        crate::equipment_destruction::lost_drain();
                        crate::naval_result_sources::lost_drain();
                        crate::experience_gains::lost_drain();
                        crate::event_feed_observer::lost_drain();
                        crate::operation_outcomes::lost_drain();
                        crate::politics_changes::invalidate();
                        crate::land_events::fail("land prepared spool write failed");
                        crate::air_losses::lost_drain();
                        crate::building_bombing::lost_drain();
                        crate::nuclear_strikes::lost_drain();
                        last_warning = Some(error_reason(error, "collector spool I/O failed"));
                    }
                    None
                }
                Err(error) => {
                    // Conversion failed before any write; its exact batch is
                    // known unpublished, rather than an uncertain spool write.
                    frame.extended.discard_event_feed();
                    crate::equipment_destruction::lost_drain();
                    crate::naval_result_sources::lost_drain();
                    crate::experience_gains::lost_drain();
                    crate::event_feed_observer::lost_drain();
                    crate::operation_outcomes::lost_drain();
                    crate::politics_changes::invalidate();
                    crate::land_events::fail("land frame conversion failed");
                    crate::air_losses::lost_drain();
                    crate::building_bombing::lost_drain();
                    crate::nuclear_strikes::lost_drain();
                    last_warning = Some(error_reason(error, "collector spool I/O failed"));
                    None
                }
            };
            let span = phases.span(Phase::Recycle);
            if crate::allocation::failed() {
                return Err("collector allocation failed");
            }
            if processor.take_retry_full() {
                shared.request_retry_full();
            }
            let unpublished = processor.take_unpublished_families();
            if unpublished != 0 {
                shared
                    .forget_families
                    .fetch_or(unpublished, Ordering::Release);
            }
            if pending_write.is_none() {
                crate::allocation::attempted_hour(None);
            }
            sizing
                .sizing
                .recycle_frame(shared, index, before_bytes, &mut work);
            work.buffer_service
                .record_ticks(sampling::qpc() - service_started, shared.frequency);
            span.finish();
            // No reference to the recycled frame survives this point. Complete
            // this write before preparing another frame or flushing: comparison
            // tokens belong to the current encoder baseline.
            if let Some(prepared) = pending_write
                && let Err(error) = processor.write_prepared(prepared)
            {
                crate::equipment_destruction::lost_drain();
                crate::naval_result_sources::lost_drain();
                crate::experience_gains::lost_drain();
                crate::event_feed_observer::lost_drain();
                crate::operation_outcomes::lost_drain();
                crate::politics_changes::invalidate();
                crate::land_events::fail("land recycled spool write failed");
                crate::air_losses::lost_drain();
                crate::building_bombing::lost_drain();
                crate::nuclear_strikes::lost_drain();
                last_warning = Some(error_reason(error, "collector spool I/O failed"));
            }
            sizing.retire(processor.take_retired());
        }
        crate::allocation::attempted_hour(None);
        if crate::allocation::failed() {
            return Err("collector allocation failed");
        }
        sizing.spool_pass(shared, &mut processor, &mut work)?;
        let span = phases.span(Phase::Flush);
        if crate::allocation::failed() {
            return Err("collector allocation failed");
        }
        let flush_started = sampling::qpc();
        if let Err(error) = processor.flush() {
            crate::equipment_destruction::lost_drain();
            crate::naval_result_sources::lost_drain();
            crate::experience_gains::lost_drain();
            crate::event_feed_observer::lost_drain();
            crate::operation_outcomes::lost_drain();
            crate::politics_changes::invalidate();
            crate::land_events::fail("land final spool flush failed");
            crate::air_losses::lost_drain();
            crate::building_bombing::lost_drain();
            crate::nuclear_strikes::lost_drain();
            last_warning = Some(error_reason(error, "collector spool I/O failed"));
        }
        let flush_ns = shared.ns(sampling::qpc() - flush_started);
        work.idle_flush_calls += 1;
        work.idle_flush_ns = work.idle_flush_ns.saturating_add(flush_ns);
        work.idle_flush_max_ns = work.idle_flush_max_ns.max(flush_ns);
        span.finish();
        if reported.elapsed() >= Duration::from_secs(1) || shared.stop_spool.load(Ordering::Acquire)
        {
            let _span = phases.span(Phase::Status);
            crate::allocation::attempted_hour(None);
            let _status_phase = crate::allocation::enter(crate::allocation::Phase::Status);
            if let Err(error) = refresh_requests.poll(&config.output_dir, shared) {
                crate::allocation::observe(&error);
                last_warning = Some("refresh request failed");
            }
            let status_started = sampling::qpc();
            if let Some(error) = writer.take_error() {
                last_warning = Some(error);
            }
            let status = spool_status(
                &processor,
                shared,
                &session,
                &work,
                &writer,
                if shared.recording.load(Ordering::Acquire) {
                    "recording"
                } else {
                    "detaching"
                },
                startup_unknown,
                last_warning,
                None,
                false,
                false,
                TeardownStatus::default(),
            )
            .map_err(|e| error_reason(e, "status allocation failed"))?;
            writer
                .publish_measured(
                    status_path,
                    status,
                    processor
                        .spool_directories()
                        .map_err(|e| error_reason(e, "status directories allocation failed"))?,
                )
                .map_err(|e| error_reason(e, "status hand-over allocation failed"))?;
            work.status_update
                .record_ticks(sampling::qpc() - status_started, shared.frequency);
            reported = Instant::now();
        }
        if shared.stop_spool.load(Ordering::Acquire) {
            if !shared.ready.is_empty() {
                continue;
            }
            let _teardown = crate::allocation::enter(crate::allocation::Phase::Teardown);
            crate::allocation::attempted_hour(None);
            let pending = unsafe { std::mem::take(&mut *shared.pending.get()) };
            match processor.prepare_losses(&pending) {
                Err(error) => add_terminal_error(
                    &mut drain_result,
                    error_reason(
                        error,
                        "cannot open campaign spool for retained loss witnesses at detach",
                    ),
                ),
                Ok(()) => {
                    if pending.unknown_count != 0 {
                        processor.unknown_loss(pending.unknown_count);
                    }
                    for loss in pending.iter() {
                        if let Err(error) = processor.lost(loss) {
                            add_terminal_error(
                                &mut drain_result,
                                error_reason(error, "cannot drain retained loss witness"),
                            );
                        }
                    }
                }
            }
            if crate::allocation::failed() {
                return Err("collector allocation failed");
            }
            if let Err(error) = processor.detach() {
                add_terminal_error(
                    &mut drain_result,
                    error_reason(error, "cannot finish spool detach"),
                );
            }
            if startup_unknown != 0 {
                add_terminal_error(&mut drain_result, "startup callbacks had no readable clock");
            }
            if crate::allocation::failed() {
                return Err("collector allocation failed");
            }
            let terminal_status = crate::status::serialize(
                &spool_status(
                    &processor,
                    shared,
                    &session,
                    &work,
                    &writer,
                    "detached",
                    startup_unknown,
                    last_warning,
                    drain_result.as_ref().err().copied(),
                    drain_result.is_ok(),
                    true,
                    teardown_status.get().copied().unwrap_or_default(),
                )
                .map_err(|e| error_reason(e, "terminal status allocation failed"))?,
            )
            .map_err(|e| error_reason(e, "terminal status allocation failed"))?;
            return Ok(SpoolOutcome {
                result: drain_result,
                terminal_status,
                directories: processor
                    .spool_directories()
                    .map_err(|e| error_reason(e, "terminal directory allocation failed"))?,
            });
        }
        if shared.ready.is_empty() {
            let _span = phases.span(Phase::Wait);
            std::thread::park_timeout(SPOOL_IDLE_WAIT);
        }
    }
}

/// Capture-buffer capacities. The spool thread raises them as it returns a
/// buffer that ran short, and the sizing thread from live demand; whoever
/// raises them grows every idle buffer while holding the lock. Live table and
/// input hints read a copy outside it, published if unchanged or re-read under
/// the lock after a concurrent raise. The spool publishes its grown returning
/// buffer to the free queue under the same lock before growing idle buffers.
#[derive(Clone, Copy, PartialEq, Eq)]
struct BufferCapacities {
    core: [usize; 12],
    map: crate::map_capture::MapCapacities,
    extended: crate::extended::Capacities,
}

impl BufferCapacities {
    fn grow_idle(&mut self, shared: &Shared) {
        grow_idle_frames(shared, &mut self.core, &mut self.map, &mut self.extended);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What one sizing pass keeps between passes.
struct SizingState {
    registry: RegistryWatch,
    applied_combat_hints: [CombatHintState; sampling::BUFFERS],
    fuel_demand_at: Instant,
    /// The last pass that ran the once-a-second sizing reads.
    second: Instant,
}

/// Sizing observations the spool thread records in its status.
#[derive(Default)]
struct PendingSizing {
    errors: u64,
    last_error: Option<&'static str>,
    game_score: Vec<engine::game_score_inputs::ArenaSizing>,
    capacities_hold: WallTotals,
}

#[cfg(test)]
type RecycleIdleCheck = Box<dyn FnOnce(&SizingShared, &Shared) + Send>;

/// State the spool thread shares with its sizing thread.
struct SizingShared {
    capacities: Mutex<BufferCapacities>,
    #[cfg(test)]
    before_recycle_idle_growth: Mutex<Option<RecycleIdleCheck>>,
    stop: AtomicBool,
    /// A terminal sizing failure, which ends the spool thread.
    failure: OnceLock<&'static str>,
    pending: Mutex<PendingSizing>,
    /// The spool thread's allocation context between hours, which a terminal
    /// allocation failure on the sizing thread records.
    context: Mutex<crate::allocation::Context>,
    /// Held by the sizing thread, or by the spool thread when the sizing
    /// thread could not start.
    state: Mutex<Option<SizingState>>,
    /// What encoder commits handed back, freed by the sizing thread.
    retired: Mutex<Vec<replay::frame::Retired>>,
}

impl SizingShared {
    fn new(capacities: BufferCapacities, state: Option<SizingState>) -> Self {
        Self {
            capacities: Mutex::new(capacities),
            #[cfg(test)]
            before_recycle_idle_growth: Mutex::new(None),
            stop: AtomicBool::new(false),
            failure: OnceLock::new(),
            pending: Mutex::new(PendingSizing::default()),
            context: Mutex::new(crate::allocation::Context::default()),
            state: Mutex::new(state),
            retired: Mutex::new(Vec::new()),
        }
    }

    fn error(&self, reason: &'static str) {
        let mut pending = lock(&self.pending);
        pending.errors = pending.errors.saturating_add(1);
        pending.last_error = Some(reason);
    }

    fn demand<T>(&self, result: Result<T, &'static str>) -> Option<T> {
        result.map_err(|reason| self.error(reason)).ok()
    }

    /// Raise the capacities under their lock; grow every idle buffer when
    /// the step raised any.
    fn raise<R>(&self, shared: &Shared, step: impl FnOnce(&mut BufferCapacities) -> R) -> R {
        self.with_capacities(shared, |capacities| {
            let before = *capacities;
            let result = step(capacities);
            if *capacities != before {
                capacities.grow_idle(shared);
            }
            result
        })
    }

    /// Observe the once-a-second table and live-input hints outside the lock.
    /// Publish the proposal only if its starting capacities still hold;
    /// otherwise re-read once under the lock, using only that attempt's errors.
    /// A fallback repeats character roster, role, trait and advisor definition
    /// hints (monotonic maxima), advisor modifier hints and advisor-pool
    /// preparation (monotonic hints and owned pools), land-event preparation
    /// (monotonic word demand and checked pending storage), and DemandFailures
    /// updates in table_demand.rs and the live-input readers. Those diagnostics
    /// count read attempts, including the fallback, as their scope states.
    /// Session errors are reported once from the published attempt after release.
    fn raise_from_live_observation(
        &self,
        shared: &Shared,
        mut observe: impl FnMut(&mut BufferCapacities) -> [Option<&'static str>; 2],
    ) {
        let before = self.with_capacities(shared, |capacities| *capacities);
        let mut proposal = before;
        let failures = observe(&mut proposal);
        let failures = self.with_capacities(shared, |capacities| {
            let current = *capacities;
            let failures = if current == before {
                *capacities = proposal;
                failures
            } else {
                observe(capacities)
            };
            if *capacities != current {
                capacities.grow_idle(shared);
            }
            failures
        });
        for reason in failures.into_iter().flatten() {
            self.error(reason);
        }
    }

    /// Grow and publish the spool's returning buffer under one capacity hold,
    /// then grow idle buffers if its observed demand changed the capacities.
    fn recycle_frame(
        &self,
        shared: &Shared,
        index: usize,
        before_frame_bytes: usize,
        work: &mut WorkerWork,
    ) {
        let live = unsafe { LiveMemory::new() };
        let mut capacities = work.recycle_capacities(self, shared.frequency);
        // Idle growth may have already charged shared storage while this frame
        // converted. Count only its change during this capacity hold.
        let before_shared_bytes = shared.country_intel.allocated_bytes();
        let previous = *capacities;
        let _ = capacities
            .extended
            .combat_progress
            .presize(&live, shared.base);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.grow(&mut capacities.core);
        frame.map.grow(&mut capacities.map);
        let states = capacities.core[3];
        capacities.extended.raise_for_states(states);
        frame.extended.grow(&mut capacities.extended);
        let before = before_frame_bytes + before_shared_bytes;
        let after = frame.allocated_bytes() + shared.country_intel.allocated_bytes();
        if after >= before {
            shared
                .pool_bytes
                .fetch_add((after - before) as u64, Ordering::Relaxed);
        } else {
            shared
                .pool_bytes
                .fetch_sub((before - after) as u64, Ordering::Relaxed);
        }
        if let SeedStatus::Capacity { required_cells } = frame.country_cache_status {
            shared.offer_country_cache(required_cells);
        }
        shared.recycle_frame_prepared(index);
        if *capacities != previous {
            #[cfg(test)]
            {
                let check = lock(&self.before_recycle_idle_growth).take();
                if let Some(check) = check {
                    check(self, shared);
                }
            }
            capacities.grow_idle(shared);
        }
    }

    /// Time only sizing work under the capacity mutex, reporting after release.
    fn with_capacities<R>(
        &self,
        shared: &Shared,
        step: impl FnOnce(&mut BufferCapacities) -> R,
    ) -> R {
        let mut capacities = lock(&self.capacities);
        let started = sampling::qpc();
        let result = step(&mut capacities);
        let elapsed = sampling::qpc() - started;
        drop(capacities);
        lock(&self.pending)
            .capacities_hold
            .record_ticks(elapsed, shared.frequency);
        result
    }
}

/// The spool's sizing thread: live sizing reads, the capture-buffer growth
/// they request and between-tick observer preparation, in passes beside the
/// spool thread. Dropping it stops and joins the thread.
struct SizingThread {
    sizing: Arc<SizingShared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SizingThread {
    /// Start the thread with the attach-time capacities and sizing caches.
    /// Without a thread the spool thread runs each pass itself.
    fn start(shared: Arc<Shared>, state: SpoolState) -> Self {
        let sizing = Arc::new(SizingShared::new(
            BufferCapacities {
                core: state.sizing.core,
                map: state.sizing.map,
                extended: state.sizing.extended,
            },
            Some(SizingState {
                registry: RegistryWatch::default(),
                applied_combat_hints: state.applied_combat_hints,
                fuel_demand_at: Instant::now(),
                second: Instant::now() - Duration::from_secs(1),
            }),
        ));
        let worker = Arc::clone(&sizing);
        let thread = std::thread::Builder::new()
            .name("hoi4-spool-sizing".into())
            .stack_size(BACKGROUND_STACK_BYTES)
            .spawn(move || {
                crate::thread_cpu::register(crate::thread_cpu::Role::SpoolSizing);
                sizing_loop(&shared, &worker);
            })
            .ok();
        Self { sizing, thread }
    }

    fn thread_id(&self) -> Option<u32> {
        use std::os::windows::io::AsRawHandle;
        let thread = self.thread.as_ref()?;
        // SAFETY: the handle stays open while the JoinHandle lives.
        let id = unsafe {
            windows_sys::Win32::System::Threading::GetThreadId(thread.as_raw_handle() as _)
        };
        (id != 0).then_some(id)
    }

    /// Called by the spool thread once per pass: record what the sizing
    /// thread observed, hand it the current allocation context and run the
    /// pass here when the thread could not start.
    fn spool_pass(
        &self,
        shared: &Shared,
        processor: &mut Processor,
        work: &mut WorkerWork,
    ) -> Result<(), &'static str> {
        let span = crate::spool_phases::SPOOL.span(Phase::Sizing);
        if let Some(&failure) = self.sizing.failure.get() {
            return Err(failure);
        }
        let pending = std::mem::take(&mut *lock(&self.sizing.pending));
        work.sizing_capacities_hold.merge(pending.capacities_hold);
        if let Some(reason) = pending.last_error {
            processor.observe_input_demand_errors(pending.errors, reason);
        }
        for observed in &pending.game_score {
            processor
                .observe_game_score_sizing(observed)
                .map_err(|error| {
                    error_reason(error, "game score sizing status allocation failed")
                })?;
        }
        *lock(&self.sizing.context) = crate::allocation::context();
        span.finish();
        match &self.thread {
            Some(thread) => thread.thread().unpark(),
            None => {
                if let Some(state) = lock(&self.sizing.state).as_mut() {
                    sizing_pass(shared, &self.sizing, state, &crate::spool_phases::SPOOL)?;
                }
            }
        }
        Ok(())
    }
}

impl SizingThread {
    /// Free what a commit handed back on the sizing thread, or here without
    /// one, so the spool thread goes on to its next hour.
    fn retire(&self, retired: Vec<replay::frame::Retired>) {
        if retired.is_empty() {
            return;
        }
        let Some(thread) = &self.thread else {
            let _span = crate::spool_phases::SPOOL.span(Phase::Retire);
            drop(retired);
            return;
        };
        let mut held = lock(&self.sizing.retired);
        if held.is_empty() {
            *held = retired;
        } else if engine::allocation::reserve(&mut held, retired.len()).is_ok() {
            held.extend(retired);
        } else {
            drop(held);
            let _span = crate::spool_phases::SPOOL.span(Phase::Retire);
            drop(retired);
            return;
        }
        drop(held);
        thread.thread().unpark();
    }
}

impl Drop for SizingThread {
    fn drop(&mut self) {
        #[cfg(feature = "worker_sampler")]
        if self.thread.is_some() {
            crate::worker_sampler::stop_before_thread_exit();
        }
        self.sizing.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn sizing_loop(shared: &Shared, sizing: &SizingShared) {
    #[cfg(feature = "worker_sampler")]
    let _sampler_exit = crate::thread_cpu::StopOnThreadExit;
    let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
    let Some(mut state) = lock(&sizing.state).take() else {
        return;
    };
    let phases = &crate::spool_phases::SIZING;
    let mut priority = OwnPriority::new();
    while !sizing.stop.load(Ordering::Acquire) && !crate::allocation::failed() {
        priority.follow(shared.buffers_backlogged());
        let retired = std::mem::take(&mut *lock(&sizing.retired));
        if !retired.is_empty() {
            let _span = phases.span(Phase::Retire);
            drop(retired);
        }
        crate::allocation::set_context(*lock(&sizing.context));
        if let Err(failure) = sizing_pass(shared, sizing, &mut state, phases) {
            let _ = sizing.failure.set(failure);
            shared.wake_spool();
            return;
        }
        let _span = phases.span(Phase::Wait);
        std::thread::park_timeout(SPOOL_IDLE_WAIT);
    }
}

/// One pass of live sizing reads and between-tick observer preparation:
/// each step that raises capacities grows the idle capture buffers.
fn sizing_pass(
    shared: &Shared,
    sizing: &SizingShared,
    state: &mut SizingState,
    phases: &'static crate::spool_phases::Totals,
) -> Result<(), &'static str> {
    let live = unsafe { LiveMemory::new() };
    let base = shared.base;
    let registry = &mut state.registry;
    let once_a_second = state.second.elapsed() >= Duration::from_secs(1);
    let span = phases.span(Phase::Sizing);
    if let Some(logistics_rows) = registry
        .poll_logistics(&live, base)
        .map_err(|e| error_reason(e, "logistics balance staging allocation failed"))?
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.logistics.raise(logistics_rows)
        });
    }
    let span = span.switch(Phase::Observers);
    crate::relation_changes::collect();
    crate::world_input_changes::collect();
    crate::history_changes::collect();
    crate::flag_changes::presize();
    crate::flag_changes::collect();
    crate::building_changes::presize();
    crate::building_changes::collect();
    crate::advisor_pool_changes::presize();
    crate::advisor_pool_changes::collect();
    crate::party_leader_changes::presize();
    crate::party_leader_changes::collect();
    crate::variable_changes::presize();
    crate::variable_changes::collect();
    crate::party_changes::presize();
    crate::party_changes::collect();
    crate::wing_changes::presize();
    crate::wing_changes::collect();
    crate::mission_changes::presize();
    crate::mission_changes::collect();
    crate::rail_changes::presize();
    crate::rail_changes::collect();
    crate::territory_changes::presize();
    crate::territory_changes::collect();
    crate::army_stat_changes::presize();
    crate::army_stat_changes::collect();
    crate::country_input_changes::presize();
    crate::country_input_changes::collect();
    crate::raid_changes::presize();
    crate::raid_changes::collect();
    crate::naval_tree_changes::presize();
    crate::naval_tree_changes::collect();
    crate::unit_leader_changes::presize();
    crate::unit_leader_changes::collect();
    crate::opinion_changes::collect();
    crate::retreat_ban_changes::presize();
    crate::retreat_ban_changes::collect();
    crate::unit_leader_trait_changes::presize();
    crate::unit_leader_trait_changes::collect();
    crate::local_subunit_changes::presize();
    crate::local_subunit_changes::collect();
    crate::province_unit_changes::presize();
    crate::province_unit_changes::collect();
    crate::template_changes::presize();
    crate::template_changes::collect();
    crate::ship_model_changes::presize();
    crate::ship_model_changes::collect();
    crate::air_base_carry_changes::presize();
    crate::air_base_carry_changes::collect();
    crate::character_changes::presize(crate::extended::characters::census::initial_len());
    crate::character_changes::collect();
    crate::politics_changes::prepare();
    crate::air_losses::presize(0);
    crate::building_bombing::presize(0);
    crate::nuclear_strikes::presize(0);
    crate::land_events::prepare();
    let air_loss_capacity = crate::air_losses::capacity();
    let bombing_capacity = crate::building_bombing::capacity();
    let nuclear_capacity = crate::nuclear_strikes::capacity();
    let land_event_capacity = crate::land_events::capacity();
    sizing.raise(shared, |capacities| {
        let extended = &mut capacities.extended;
        extended.air_losses.events = extended.air_losses.events.max(air_loss_capacity);
        extended.building_bombing = extended.building_bombing.max(bombing_capacity);
        extended.nuclear_strikes = extended.nuclear_strikes.max(nuclear_capacity);
        extended.land_events.words = extended.land_events.words.max(land_event_capacity);
    });
    let operations =
        sizing.with_capacities(shared, |capacities| capacities.extended.operation_outcomes);
    operations.prepare();
    crate::experience_gains::prepare(registry.slots);
    crate::line_output::collect();
    crate::construction_ic::collect();
    crate::fuel_awards::collect();
    let span = span.switch(Phase::Sizing);
    if state.fuel_demand_at.elapsed() >= RegistryWatch::INTEL_INTERVAL {
        state.fuel_demand_at = Instant::now();
        if let Some(entries) =
            crate::country_scale::observe_politics_demand(&live, base, registry.slots)
        {
            crate::politics_changes::resize(entries, 0);
        }
        crate::fuel_awards::presize(&live, base);
        match crate::country_scale::observe_fuel_unit_demand(&live, base) {
            Ok(Some(demand)) => sizing.raise(shared, |capacities| {
                capacities.extended.economy.demand.raise(demand)
            }),
            Ok(None) => {}
            Err(reason) => sizing.error(reason),
        }
    }
    let (entries, names) = crate::fuel_awards::capacities();
    sizing.raise(shared, |capacities| {
        let awards = &mut capacities.extended.fuel_awards;
        awards.entries = awards.entries.max(entries);
        awards.names = awards.names.max(names);
    });
    let span = span.switch(Phase::Observers);
    crate::peace_settlements::collect();
    let span = span.switch(Phase::Sizing);
    if once_a_second {
        let peace = crate::peace_settlements::capacity_for_world(&live, base);
        sizing.raise(shared, |capacities| {
            let extended = &mut capacities.extended;
            if !extended.peace_settlements.contains(peace) || !extended.peace_bids.contains(peace) {
                extended.peace_settlements = extended.peace_settlements.merge(peace);
                extended.peace_bids = extended.peace_bids.merge(peace);
            }
        });
    }
    let span = span.switch(Phase::Observers);
    let peace = lock(&sizing.capacities).extended.peace_settlements;
    crate::peace_settlements::presize(peace);
    crate::event_feed_observer::presize(crate::country_scale::census_rows(0));
    let span = span.switch(Phase::Sizing);
    crate::naval_result_sources::prepare(&live, base);
    let naval_result_capacity = crate::naval_result_sources::capacity();
    sizing.raise(shared, |capacities| {
        let sources = &mut capacities.extended.naval_result_sources;
        *sources = (*sources).max(naval_result_capacity);
    });
    if let Some(lines) = registry.poll_lines(&live, base) {
        crate::line_output::presize(lines);
        sizing.raise(shared, |capacities| {
            let entries = &mut capacities.extended.line_output.entries;
            *entries = (*entries).max(lines);
        });
    }
    // A save load rebuilds the country registry before its first hourly
    // tick; presize every buffer while the game is still loading. Sizing
    // only grows, so a larger region roster alone repeats it harmlessly.
    if let Ok(bytes) = crate::extended::weather::observe_display_capacity(&live, base) {
        let changed = sizing.raise(shared, |capacities| {
            let display = &mut capacities.extended.weather.display_bytes;
            if bytes > *display {
                *display = bytes;
                true
            } else {
                false
            }
        });
        if changed {
            decode::presize_weather_jobs();
        }
    }
    if let Some((slots, regions)) = registry.poll(&live, base) {
        crate::country_scale::note_slots(slots);
        crate::operation_outcomes::resize(slots.max(1));
        // This census is RegistryWatch-throttled, including while idle.
        crate::politics_changes::resize(
            crate::country_scale::observe_politics_demand(&live, base, slots).unwrap_or(slots),
            sizing
                .demand(engine::politics_sites::lexical_key_demand(&live, base))
                .unwrap_or(0),
        );
        crate::relation_changes::presize(slots);
        crate::world_input_changes::presize(slots);
        crate::history_changes::presize(slots, regions);
        decode::presize_country_pools();
        let country_cells = sizing.raise(shared, |capacities| {
            let mut raised = crate::country_scale::Sizing {
                core: capacities.core,
                map: capacities.map,
                extended: capacities.extended,
                country_cells: 0,
                content: engine::cdlc_manager::Counts::default(),
            };
            raised.raise_for_slots(slots);
            raised.extended.raise_for_regions(regions);
            capacities.core = raised.core;
            capacities.map = raised.map;
            capacities.extended = raised.extended;
            raised.country_cells
        });
        shared.offer_country_cache(country_cells);
    }
    if once_a_second {
        if let Err(reason) = crate::construction_ic::presize_from_live(&live, base) {
            sizing.error(reason);
        }
        crate::air_losses::prepare(&live, base, registry.slots);
        crate::building_bombing::prepare(&live, base, registry.slots);
        crate::nuclear_strikes::presize(
            crate::building_bombing::buildings().saturating_add(registry.slots),
        );
        let source = crate::country_scale::source_demand::Demand::observe(&live, base);
        let air_loss_capacity = crate::air_losses::capacity();
        let bombing_capacity = crate::building_bombing::capacity();
        let nuclear_capacity = crate::nuclear_strikes::capacity();
        sizing.raise(shared, |capacities| {
            let extended = &mut capacities.extended;
            extended.air_losses.events = extended.air_losses.events.max(air_loss_capacity);
            extended.building_bombing = extended.building_bombing.max(bombing_capacity);
            extended.nuclear_strikes = extended.nuclear_strikes.max(nuclear_capacity);
            if let Ok(Some(demand)) = &source {
                demand.apply_parts(&mut capacities.core, &mut capacities.extended);
            }
        });
        if let Err(reason) = source {
            sizing.error(reason);
        }
        sizing.raise_from_live_observation(shared, |capacities| {
            [
                crate::country_scale::presize_table_inputs(
                    &live,
                    base,
                    &mut capacities.map,
                    &mut capacities.extended,
                )
                .err(),
                capacities.extended.presize_live_inputs(&live, base).err(),
            ]
        });
        // The live pool census sizes storage only; observe it once a second
        // rather than every idle pass.
        crate::equipment_destruction::prepare(&live, base, registry.slots);
        let destruction_capacity = crate::equipment_destruction::capacity();
        sizing.raise(shared, |capacities| {
            let destruction = &mut capacities.extended.equipment_destruction;
            *destruction = (*destruction).max(destruction_capacity);
        });
    }
    if let Some(rows) = sizing.demand(registry.poll_intel(&live, base)).flatten() {
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_for_country_intel(rows)
        });
    }
    if let Some(demand) = sizing
        .demand(registry.poll_ace_names(&live, base))
        .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.air.raise_for_names(demand)
        });
    }
    if let Some(bytes) = sizing
        .demand(registry.poll_air_wing_names(&live, base))
        .flatten()
    {
        crate::country_scale::note_air_wing_name_bytes(bytes);
        decode::presize_air_wing_names();
        sizing.raise(shared, |capacities| {
            capacities.map.raise_for_air_wing_names(bytes)
        });
    }
    if let Some(demand) = sizing
        .demand(registry.poll_railway_gun_names(&live, base))
        .flatten()
    {
        crate::country_scale::note_railway_gun_names(demand);
        sizing.raise(shared, |capacities| {
            capacities.extended.railway_guns.raise_for_names(demand)
        });
    }
    if let Some(demand) = sizing.demand(registry.poll_map(&live, base)).flatten() {
        if let Ok(carrier) = demand.carrier
            && crate::country_scale::note_carrier_demand(carrier)
        {
            decode::presize_country_pools();
        }
        for reason in demand.map_refusals() {
            sizing.error(reason);
        }
        let port_failure = sizing.raise(shared, |capacities| {
            let mut raised = crate::country_scale::Sizing {
                core: capacities.core,
                map: capacities.map,
                extended: capacities.extended,
                country_cells: 0,
                content: engine::cdlc_manager::Counts::default(),
            };
            raised.raise_for_map(&demand);
            raised
                .extended
                .country_counters
                .raise_for_physical_navies(demand.navies.capacities);
            raised
                .extended
                .naval_counters
                .raise_for_names(demand.navies.name_rows, demand.navies.name_bytes);
            raised
                .extended
                .naval_counters
                .raise_for_supply_origins(demand.navies.supply_origins);
            let failure = raised
                .extended
                .naval_counters
                .raise_for_port(demand.navies.port)
                .err();
            // The map hint raises the core and extended capacities.
            capacities.core = raised.core;
            capacities.extended = raised.extended;
            failure
        });
        for reason in [port_failure, demand.navies.refusal()]
            .into_iter()
            .flatten()
        {
            sizing.error(reason);
        }
    }
    if let Some(demand) = sizing
        .demand(registry.poll_presentation_census(&live, base))
        .flatten()
    {
        if let Some(reason) = demand.failure {
            sizing.error(reason);
        }
        sizing.raise(shared, |capacities| {
            capacities
                .extended
                .division_presentation
                .raise_for_census(demand)
        });
    }
    let game_score = lock(&sizing.capacities).extended.game_score.enabled;
    if game_score && let Some(observed) = registry.poll_game_score(&live, base) {
        let observed =
            observed.map_err(|error| error_reason(error, "game score sizing allocation failed"))?;
        let demand = observed.demand;
        {
            let mut pending = lock(&sizing.pending);
            engine::allocation::reserve(&mut pending.game_score, 1).map_err(|error| {
                error_reason(error, "game score sizing status allocation failed")
            })?;
            pending.game_score.push(observed);
        }
        if sizing.raise(shared, |capacities| {
            capacities.extended.game_score.raise(demand)
        }) {
            decode::presize_country_pools();
        }
    }
    if let Some(demand) = sizing.demand(registry.poll_raids(&live, base)).flatten() {
        for reason in demand.failures.into_iter().flatten() {
            sizing.error(reason);
        }
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_for_raid_arenas(demand)
        });
    }
    if let Some(demand) = sizing
        .demand(registry.poll_characters(&live, base))
        .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_for_characters(demand)
        });
    }
    if let Some(demand) = sizing
        .demand(registry.poll_saved_targets(&live, base))
        .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.saved_event_targets.raise(demand)
        });
    }
    if let Some(demand) = sizing.demand(registry.poll_politics(&live, base)).flatten() {
        for reason in demand.failures() {
            sizing.error(reason);
        }
        sizing.raise(shared, |capacities| {
            capacities
                .extended
                .country_counters
                .raise_for_pride_names(demand.pride_name_bytes);
            capacities.extended.raise_for_politics(demand);
        });
        let operations =
            sizing.with_capacities(shared, |capacities| capacities.extended.operation_outcomes);
        operations.prepare();
    }
    if let Some(slots) = sizing
        .demand(registry.poll_reserved_visibility(&live, base))
        .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_reserved_visibility(slots)
        });
    }
    if let Some(hints) = registry.poll_combat_hints(&live, base) {
        for reason in hints.failures.into_iter().flatten() {
            sizing.error(reason);
        }
        sizing.with_capacities(shared, |capacities| {
            grow_combat_hints(
                shared,
                &capacities.extended,
                &hints,
                &mut state.applied_combat_hints,
            );
        });
    }
    span.finish();
    if once_a_second {
        state.second = Instant::now();
    }
    Ok(())
}

/// The spool loop's longest idle wait. Ready buffers, parked clock
/// preparations and the stop request end it at once; the bound keeps the
/// pass that checks durability within this latency while no hour arrives;
/// the sizing thread waits as long between its passes.
const SPOOL_IDLE_WAIT: Duration = Duration::from_millis(10);

/// `recorder refresh` asks for one full-refresh hour: every scheduled table,
/// including daily, static and sliced ones, is read at the next hourly sample.
/// The request is a file in the session directory; the spool thread turns it
/// into `refresh_next` and acknowledges it. Observation remains continuous;
/// `force_next` is reserved for capture recovery. Nothing is sent to the
/// game. Oracles use this to compare every table at a chosen hour.
#[derive(Default)]
struct RefreshRequests {
    /// The last accepted token. A request whose acknowledgement or removal
    /// failed is still on disk; it must not force every later hour full.
    last: Option<String>,
}

pub(crate) const REFRESH_REQUEST: &str = "refresh.request";
pub(crate) const REFRESH_ACK: &str = "refresh.ack";

impl RefreshRequests {
    fn poll(&mut self, session: &Path, shared: &Shared) -> io::Result<()> {
        let request = session.join(REFRESH_REQUEST);
        let bytes = match fs::read(&request) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let token = std::str::from_utf8(&bytes)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?
            .trim();
        if token.is_empty() || self.last.as_deref() == Some(token) {
            return Ok(());
        }
        let owned = token.to_owned();
        shared.refresh_next.store(true, Ordering::Release);
        REFRESH_ACCEPTED.fetch_add(1, Ordering::Relaxed);
        self.last = Some(owned);
        if fs::write(session.join(REFRESH_ACK), token.as_bytes()).is_ok() {
            fs::remove_file(&request)?;
        }
        Ok(())
    }
}

static REFRESH_ACCEPTED: AtomicU64 = AtomicU64::new(0);

static PRIORITY_LOWERED: AtomicU64 = AtomicU64::new(0);
static PRIORITY_RAISED: AtomicU64 = AtomicU64::new(0);
static PRIORITY_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Reserved stack of each collector thread that decodes, converts, compares or
/// encodes whole frames and tables: decode, spool, its conversion helpers and
/// its sizing thread. std's 2 MiB default leaves little room beside the
/// several-hundred-KiB frames optimised builds give whole-frame code on these
/// threads, and an overflow ends the game process. Windows commits only the
/// pages a thread touches, so the reservation costs address space, not memory.
pub(crate) const BACKGROUND_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Priority of one collector-owned background thread (decode, spool). It runs
/// below normal so a woken game thread and the hour's acquisition workers find
/// a CPU first, except while the capture buffers are backlogged: then the game
/// thread may be waiting in `take_frame` for exactly this thread, so it runs at
/// normal priority until the backlog drains. Only the calling thread is
/// changed; game threads, their priorities and affinities are never touched.
/// A failed change keeps the previous priority, which is slower, never wrong.
pub(crate) struct OwnPriority {
    below_normal: Option<bool>,
}

impl OwnPriority {
    pub(crate) const fn new() -> Self {
        Self { below_normal: None }
    }

    /// Call between work items with the current backlog verdict.
    pub(crate) fn follow(&mut self, backlogged: bool) {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, SetThreadPriorityBoost,
            THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_NORMAL,
        };
        let below_normal = !backlogged;
        if self.below_normal == Some(below_normal) {
            return;
        }
        if self.below_normal.is_none() {
            // No dynamic boost: a wake-up never lifts this thread above its
            // base priority, so even the backlog's normal priority only shares
            // CPUs with game threads and never preempts them.
            // SAFETY: the pseudo-handle names only the calling thread.
            if unsafe { SetThreadPriorityBoost(GetCurrentThread(), 1) } == 0 {
                PRIORITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            }
        }
        let priority = if below_normal {
            THREAD_PRIORITY_BELOW_NORMAL
        } else {
            THREAD_PRIORITY_NORMAL
        };
        // SAFETY: the pseudo-handle names only the calling thread and needs no close.
        if unsafe { SetThreadPriority(GetCurrentThread(), priority) } == 0 {
            PRIORITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if below_normal {
            PRIORITY_LOWERED.fetch_add(1, Ordering::Relaxed);
        } else {
            PRIORITY_RAISED.fetch_add(1, Ordering::Relaxed);
        }
        self.below_normal = Some(below_normal);
    }
}

fn background_priority_status() -> impl serde::Serialize {
    crate::status::status!({
        "lowered": PRIORITY_LOWERED.load(Ordering::Relaxed),
        "raised_for_backlog": PRIORITY_RAISED.load(Ordering::Relaxed),
        "failures": PRIORITY_FAILURES.load(Ordering::Relaxed),
        "scope": "cumulative priority changes of the decode, spool, spool helper and sizing threads: below normal while idle or keeping up, normal with at most two of the four reusable buffers free (the game thread may then be waiting for them); dynamic wake-up boosts are disabled, so none runs above that base priority; failures keep the previous priority",
    })
}

#[cfg(test)]
mod equipment_destruction_tests;

#[cfg(test)]
mod tests {
    use super::*;

    static ATTACH_PREPARATIONS: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];

    fn prepare_attach_first() {
        ATTACH_PREPARATIONS[0].fetch_add(1, Ordering::SeqCst);
    }

    fn prepare_attach_middle() {
        ATTACH_PREPARATIONS[1].fetch_add(1, Ordering::SeqCst);
    }

    fn prepare_attach_later() {
        ATTACH_PREPARATIONS[2].fetch_add(1, Ordering::SeqCst);
    }

    unsafe extern "system" fn attach_original() -> usize {
        7
    }

    unsafe extern "system" fn attach_callback() -> usize {
        19
    }

    fn attach_fixture_hook(
        code: &crate::hook::TestCode,
        offset: usize,
        flight: Option<&'static AtomicUsize>,
    ) -> (usize, [u8; 16], Hook) {
        let (site, bytes) = code.stub(offset, attach_original as *const () as usize);
        let callback = attach_callback as *const () as usize;
        let hook = if let Some(in_flight) = flight {
            unsafe {
                Hook::new_with_safety(
                    site,
                    callback,
                    bytes,
                    crate::hook::TapSafety {
                        in_flight,
                        stub: callback..callback + 1,
                    },
                )
            }
        } else {
            unsafe { Hook::new(site, callback, bytes) }
        }
        .unwrap();
        (site, bytes, hook)
    }

    fn attach_test_progress() -> crate::progress::Progress {
        // An absent directory gives this stage no file, without creating
        // capture or progress files in the worktree.
        let absent = std::env::temp_dir().join(format!(
            "hoi4-attach-stage-{}-{:?}/absent",
            std::process::id(),
            std::thread::current().id(),
        ));
        crate::progress::Progress::open(&absent)
    }

    #[test]
    fn attach_speculative_storage_failure_stays_nonterminal_behind_a_busy_family() {
        use std::cell::Cell;

        static FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!crate::allocation::failed());
        {
            let _round = crate::hook::shared::Round::collect_attach().unwrap();
            crate::hook::publication::select_family(Some("runtime_classified_overflow"));
            // Capacity overflow exercises classification without changing
            // allocator hooks or attempting a large allocation.
            let error = crate::hook::shared::publication_storage_attach::<u8>(usize::MAX, true)
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
            assert!(crate::hook::shared::take_collect_allocation_failure());
            assert!(!crate::hook::shared::take_collect_allocation_failure());
            assert!(!crate::allocation::failed());
        }
        let code = crate::hook::TestCode::new();
        let (_, _, ready_hook) = attach_fixture_hook(&code, 0, None);
        let (_, _, busy_hook) = attach_fixture_hook(&code, 64, Some(&FLIGHT));
        let mut ready = [ready_hook];
        let mut busy = [busy_hook];
        let later_calls = Cell::new(0);
        let progress = attach_test_progress();
        let stage = crate::progress::Stage::new(&progress, "runtime_allocation_ready");
        FLIGHT.store(1, Ordering::SeqCst);
        let suspensions = crate::hook::SUSPENSIONS.with(Cell::get);
        assert_eq!(
            install_attach_batch(
                &stage,
                &mut [
                    ("runtime_allocation_ready", &mut || {
                        Hook::set_family_enabled_shared(&mut ready, true)
                            .map_err(|_| "ready publication failed")
                    }),
                    ("runtime_allocation_busy", &mut || {
                        Hook::set_family_enabled_shared(&mut busy, true)
                            .map_err(|_| "busy publication failed")
                    }),
                    ("runtime_allocation_later", &mut || {
                        later_calls.set(later_calls.get() + 1);
                        crate::hook::shared::publication_storage_attach::<u8>(usize::MAX, true)
                            .map_err(|_| "later publication storage failed")?;
                        Ok(true)
                    }),
                ],
            ),
            Ok(false),
        );
        assert_eq!(stage.get(), "runtime_allocation_busy");
        assert_eq!(later_calls.get(), 1);
        assert!(ready[0].is_enabled());
        assert!(!busy[0].is_enabled());
        assert!(!crate::allocation::failed());
        assert!(!crate::hook::shared::take_collect_allocation_failure());
        assert_eq!(crate::hook::SUSPENSIONS.with(Cell::get) - suspensions, 1);
        FLIGHT.store(0, Ordering::SeqCst);
        crate::hook::publication::select_family(None);
        assert_eq!(Hook::set_family_enabled(&mut ready, false), Ok(true));
    }

    #[test]
    fn attach_collect_refusal_reports_the_earlier_prefix_without_retrying_it() {
        use std::cell::Cell;

        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for prepared in &ATTACH_PREPARATIONS {
            prepared.store(0, Ordering::SeqCst);
        }
        let code = crate::hook::TestCode::new();
        let (_, _, first_hook) = attach_fixture_hook(&code, 0, None);
        let (_, _, second_hook) = attach_fixture_hook(&code, 64, None);
        let mut first = [first_hook];
        let mut second = [second_hook];
        let first_posts = Cell::new(0);
        let second_posts = Cell::new(0);
        let refused_calls = Cell::new(0);
        let later_calls = Cell::new(0);
        let progress = attach_test_progress();
        let stage = crate::progress::Stage::new(&progress, "runtime_collect_first");
        let suspensions = crate::hook::SUSPENSIONS.with(Cell::get);
        assert_eq!(
            install_attach_batch(
                &stage,
                &mut [
                    ("runtime_collect_first", &mut || {
                        let ready = Hook::set_family_enabled_shared_prepared(
                            &mut first,
                            true,
                            prepare_attach_first,
                        )
                        .map_err(|_| "first publication failed")?;
                        if ready {
                            first_posts.set(first_posts.get() + 1);
                        }
                        Ok(ready)
                    }),
                    ("runtime_collect_second", &mut || {
                        let ready = Hook::set_family_enabled_shared(&mut second, true)
                            .map_err(|_| "second publication failed")?;
                        if ready {
                            second_posts.set(second_posts.get() + 1);
                        }
                        Ok(ready)
                    }),
                    ("runtime_collect_refused", &mut || {
                        if !crate::hook::shared::defer_preparation(prepare_attach_middle) {
                            prepare_attach_middle();
                        }
                        refused_calls.set(refused_calls.get() + 1);
                        if refused_calls.get() == 1 {
                            Err("synthetic preparation refused")
                        } else {
                            Ok(true)
                        }
                    }),
                    ("runtime_collect_later", &mut || {
                        later_calls.set(later_calls.get() + 1);
                        Ok(true)
                    }),
                ],
            ),
            Err("synthetic preparation refused"),
        );
        assert_eq!(stage.get(), "runtime_collect_refused");
        assert_eq!(
            refused_calls.get(),
            1,
            "the refusal cannot be retried in report"
        );
        assert_eq!(later_calls.get(), 0);
        assert_eq!(first_posts.get(), 1);
        assert_eq!(second_posts.get(), 1);
        assert_eq!(ATTACH_PREPARATIONS[0].load(Ordering::SeqCst), 1);
        assert_eq!(ATTACH_PREPARATIONS[1].load(Ordering::SeqCst), 1);
        assert_eq!(ATTACH_PREPARATIONS[2].load(Ordering::SeqCst), 0);
        assert!(first[0].is_enabled() && second[0].is_enabled());
        assert_eq!(crate::hook::SUSPENSIONS.with(Cell::get) - suspensions, 1);
        crate::hook::publication::select_family(None);
        for family in [&mut first, &mut second] {
            assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
        }
    }

    #[test]
    fn attach_busy_prefix_precedes_a_later_collect_refusal_and_retries_in_order() {
        use std::cell::Cell;

        static FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let code = crate::hook::TestCode::new();
        let (_, _, first_hook) = attach_fixture_hook(&code, 0, None);
        let (middle_site, middle_bytes, middle_hook) =
            attach_fixture_hook(&code, 64, Some(&FLIGHT));
        let (later_site, later_bytes, later_hook) = attach_fixture_hook(&code, 128, None);
        let mut first = [first_hook];
        let mut middle = [middle_hook];
        let mut later = [later_hook];
        let collect_refused = Cell::new(true);
        let refused_calls = Cell::new(0);
        let later_posts = Cell::new(0);
        let progress = attach_test_progress();
        let stage = crate::progress::Stage::new(&progress, "runtime_busy_first");
        FLIGHT.store(1, Ordering::SeqCst);
        let suspensions = crate::hook::SUSPENSIONS.with(Cell::get);
        let mut install = || {
            install_attach_batch(
                &stage,
                &mut [
                    ("runtime_busy_first", &mut || {
                        Hook::set_family_enabled_shared(&mut first, true)
                            .map_err(|_| "first publication failed")
                    }),
                    ("runtime_busy_middle", &mut || {
                        Hook::set_family_enabled_shared(&mut middle, true)
                            .map_err(|_| "middle publication failed")
                    }),
                    ("runtime_busy_refused", &mut || {
                        refused_calls.set(refused_calls.get() + 1);
                        if collect_refused.get() {
                            Err("later preparation refused")
                        } else {
                            Ok(true)
                        }
                    }),
                    ("runtime_busy_later", &mut || {
                        let ready = Hook::set_family_enabled_shared(&mut later, true)
                            .map_err(|_| "later publication failed")?;
                        if ready {
                            later_posts.set(later_posts.get() + 1);
                        }
                        Ok(ready)
                    }),
                ],
            )
        };
        assert_eq!(install(), Ok(false));
        assert_eq!(stage.get(), "runtime_busy_middle");
        assert_eq!(refused_calls.get(), 1);
        assert_eq!(later_posts.get(), 0);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(middle_site as *const u8, 16) },
            middle_bytes,
        );
        assert_eq!(
            unsafe { std::slice::from_raw_parts(later_site as *const u8, 16) },
            later_bytes,
        );
        FLIGHT.store(0, Ordering::SeqCst);
        collect_refused.set(false);
        assert_eq!(install(), Ok(true));
        assert_eq!(stage.get(), "runtime_busy_later");
        assert_eq!(later_posts.get(), 1);
        assert!(first[0].is_enabled() && middle[0].is_enabled() && later[0].is_enabled());
        assert_eq!(crate::hook::SUSPENSIONS.with(Cell::get) - suspensions, 2);
        crate::hook::publication::select_family(None);
        for family in [&mut first, &mut middle, &mut later] {
            assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
        }
    }

    #[test]
    fn attach_runtime_prepares_and_reports_each_family_once_under_one_suspension() {
        use std::cell::Cell;

        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for prepared in &ATTACH_PREPARATIONS {
            prepared.store(0, Ordering::SeqCst);
        }
        let code = crate::hook::TestCode::new();
        let (_, _, first_hook) = attach_fixture_hook(&code, 0, None);
        let (_, _, middle_hook) = attach_fixture_hook(&code, 64, None);
        let (_, _, later_hook) = attach_fixture_hook(&code, 128, None);
        let mut first = [first_hook];
        let mut middle = [middle_hook];
        let mut later = [later_hook];
        let posts = [Cell::new(0), Cell::new(0), Cell::new(0)];
        let progress = attach_test_progress();
        let stage = crate::progress::Stage::new(&progress, "runtime_prepare_first");
        let suspensions = crate::hook::SUSPENSIONS.with(Cell::get);
        assert_eq!(
            install_attach_batch(
                &stage,
                &mut [
                    ("runtime_prepare_first", &mut || {
                        let ready = Hook::set_family_enabled_shared_prepared(
                            &mut first,
                            true,
                            prepare_attach_first,
                        )
                        .map_err(|_| "first publication failed")?;
                        if ready {
                            posts[0].set(posts[0].get() + 1);
                        }
                        Ok(ready)
                    }),
                    ("runtime_prepare_middle", &mut || {
                        let ready = Hook::set_family_enabled_shared_prepared(
                            &mut middle,
                            true,
                            prepare_attach_middle,
                        )
                        .map_err(|_| "middle publication failed")?;
                        if ready {
                            posts[1].set(posts[1].get() + 1);
                        }
                        Ok(ready)
                    }),
                    ("runtime_prepare_later", &mut || {
                        let ready = Hook::set_family_enabled_shared_prepared(
                            &mut later,
                            true,
                            prepare_attach_later,
                        )
                        .map_err(|_| "later publication failed")?;
                        if ready {
                            posts[2].set(posts[2].get() + 1);
                        }
                        Ok(ready)
                    }),
                ],
            ),
            Ok(true),
        );
        assert_eq!(stage.get(), "runtime_prepare_later");
        for (prepared, posted) in ATTACH_PREPARATIONS.iter().zip(&posts) {
            assert_eq!(prepared.load(Ordering::SeqCst), 1);
            assert_eq!(posted.get(), 1);
        }
        assert_eq!(crate::hook::SUSPENSIONS.with(Cell::get) - suspensions, 1);
        crate::hook::publication::select_family(None);
        for family in [&mut first, &mut middle, &mut later] {
            assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
        }
    }

    #[test]
    fn attach_empty_prepared_families_run_only_when_reached_without_extra_suspensions() {
        use std::cell::Cell;

        fn install_prepared(
            hooks: &mut [Hook],
            prepare: fn(),
            posts: &Cell<usize>,
        ) -> Result<bool, &'static str> {
            let ready = Hook::set_family_enabled_shared_prepared(hooks, true, prepare)
                .map_err(|_| "synthetic publication failed")?;
            if ready {
                posts.set(posts.get() + 1);
            }
            Ok(ready)
        }

        static FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for busy in [false, true] {
            for prepared in &ATTACH_PREPARATIONS {
                prepared.store(0, Ordering::SeqCst);
            }
            let code = crate::hook::TestCode::new();
            let mut ready = if busy {
                vec![attach_fixture_hook(&code, 0, None).2]
            } else {
                Vec::new()
            };
            let mut first = Vec::new();
            let mut middle = if busy {
                vec![attach_fixture_hook(&code, 64, Some(&FLIGHT)).2]
            } else {
                Vec::new()
            };
            let mut later = Vec::new();
            let posts = [Cell::new(0), Cell::new(0), Cell::new(0)];
            let progress = attach_test_progress();
            let stage = crate::progress::Stage::new(&progress, "runtime_empty_ready");
            FLIGHT.store(usize::from(busy), Ordering::SeqCst);
            let suspensions = crate::hook::SUSPENSIONS.with(Cell::get);
            assert_eq!(
                install_attach_batch(
                    &stage,
                    &mut [
                        ("runtime_empty_ready", &mut || {
                            Hook::set_family_enabled_shared(&mut ready, true)
                                .map_err(|_| "ready publication failed")
                        }),
                        ("runtime_empty_first", &mut || {
                            install_prepared(&mut first, prepare_attach_first, &posts[0])
                        }),
                        ("runtime_empty_middle", &mut || {
                            install_prepared(&mut middle, prepare_attach_middle, &posts[1])
                        }),
                        ("runtime_empty_later", &mut || {
                            install_prepared(&mut later, prepare_attach_later, &posts[2])
                        }),
                    ],
                ),
                Ok(!busy),
            );
            assert_eq!(
                stage.get(),
                if busy {
                    "runtime_empty_middle"
                } else {
                    "runtime_empty_later"
                },
            );
            for (index, prepared) in ATTACH_PREPARATIONS.iter().enumerate() {
                assert_eq!(
                    prepared.load(Ordering::SeqCst),
                    usize::from(index < 2 || !busy)
                );
                assert_eq!(posts[index].get(), usize::from(index == 0 || !busy));
            }
            assert!(ready.iter().all(Hook::is_enabled));
            assert_eq!(
                crate::hook::SUSPENSIONS.with(Cell::get) - suspensions,
                usize::from(busy),
            );
            FLIGHT.store(0, Ordering::SeqCst);
            crate::hook::publication::select_family(None);
            assert_eq!(Hook::set_family_enabled(&mut ready, false), Ok(true));
            assert_eq!(Hook::set_family_enabled(&mut middle, false), Ok(true));
        }
    }

    fn sizing_fixture() -> (Shared, SizingThread) {
        let sizing = crate::country_scale::Sizing::for_slots(0);
        let shared = Shared::with_sizing(0, 0, 1, &sizing).unwrap();
        let capacities = BufferCapacities {
            core: sizing.core,
            map: sizing.map,
            extended: sizing.extended,
        };
        // No thread and no pass state: the spool's share of each pass only.
        let thread = SizingThread {
            sizing: Arc::new(SizingShared::new(capacities, None)),
            thread: None,
        };
        (shared, thread)
    }

    #[test]
    fn capacity_mutex_costs_reach_spool_status_once_per_call() {
        let directory =
            std::env::temp_dir().join(format!("hoi4-capacity-mutex-status-{}", std::process::id()));
        let mut processor = crate::spooling::spool_failure_fixture(&directory).unwrap();
        let (shared, sizing) = sizing_fixture();
        let mut work = WorkerWork::new();
        let returning = shared.take_frame(&mut 0).unwrap();
        let frame = unsafe { &*shared.frames[returning].load(Ordering::Acquire) };
        let before = frame.allocated_bytes();
        sizing
            .sizing
            .recycle_frame(&shared, returning, before, &mut work);
        let recycle_calls =
            serde_json::to_value(work.recycle_capacities_wait.status()).unwrap()["calls"]
                .as_u64()
                .unwrap();
        assert_eq!(recycle_calls, 1);
        for _ in 0..2 {
            sizing.sizing.raise(&shared, |_| {});
        }
        sizing.sizing.with_capacities(&shared, |_| {});
        for _ in 0..2 {
            sizing
                .spool_pass(&shared, &mut processor, &mut work)
                .unwrap();
            let status = serde_json::to_value(
                processor
                    .status_with_work(work.status(0, shared.frequency).unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                status["spool_work"]["recycle_capacities_wait"]["calls"],
                recycle_calls
            );
            assert_eq!(status["spool_work"]["sizing_capacities_hold"]["calls"], 3);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_spool_records_sizing_observations_once_and_ends_on_a_sizing_failure() {
        let directory =
            std::env::temp_dir().join(format!("hoi4-sizing-handoff-{}", std::process::id()));
        let mut processor = crate::spooling::spool_failure_fixture(&directory).unwrap();
        let (shared, sizing) = sizing_fixture();
        let mut work = WorkerWork::new();
        sizing.sizing.error("first sizing refusal");
        sizing.sizing.error("second sizing refusal");
        lock(&sizing.sizing.pending)
            .game_score
            .push(engine::game_score_inputs::ArenaSizing::default());
        let context = crate::allocation::Context {
            failed_hour: Some(7),
            ..Default::default()
        };
        crate::allocation::set_context(context);
        sizing
            .spool_pass(&shared, &mut processor, &mut work)
            .unwrap();
        // A terminal allocation failure on the sizing thread names the
        // spool thread's latest campaign context.
        assert_eq!(lock(&sizing.sizing.context).failed_hour, Some(7));
        crate::allocation::set_context(Default::default());
        for pass in 0..2 {
            if pass != 0 {
                sizing
                    .spool_pass(&shared, &mut processor, &mut work)
                    .unwrap();
            }
            let status = serde_json::to_value(processor.status().unwrap()).unwrap();
            assert_eq!(status["input_demand_errors"], 2);
            assert_eq!(status["input_demand_last_error"], "second sizing refusal");
            assert_eq!(status["game_score"]["sizing_observations"], 1);
        }
        let _ = sizing.sizing.failure.set("synthetic sizing failure");
        assert_eq!(
            sizing.spool_pass(&shared, &mut processor, &mut work).err(),
            Some("synthetic sizing failure")
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn spool_and_sizing_raises_of_the_buffer_capacities_both_survive() {
        let (shared, sizing) = sizing_fixture();
        let sizing = &sizing.sizing;
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for rows in 1..=64 {
                    sizing.raise(&shared, |capacities| {
                        let sources = &mut capacities.extended.naval_result_sources;
                        *sources = (*sources).max(rows);
                    });
                }
            });
            for rows in 1..=64 {
                sizing.raise(&shared, |capacities| {
                    let destruction = &mut capacities.extended.equipment_destruction;
                    *destruction = (*destruction).max(rows);
                });
            }
        });
        let capacities = *lock(&sizing.capacities);
        assert!(capacities.extended.naval_result_sources >= 64);
        assert!(capacities.extended.equipment_destruction >= 64);
        // Every idle buffer was returned after growing.
        assert_eq!(shared.free.len(), sampling::BUFFERS);
    }

    fn assert_idle_war_and_faction_capacities(shared: &Shared, sizing: &SizingShared) {
        let capacities = *lock(&sizing.capacities);
        assert_eq!(shared.free.len(), sampling::BUFFERS);
        for frame in &shared.frames {
            let frame = unsafe { &*frame.load(Ordering::Acquire) };
            assert!(frame.wars.len() >= capacities.core[5]);
            assert!(frame.factions.len() >= capacities.core[6]);
        }
    }

    #[test]
    fn live_observation_releases_capacity_mutex_and_grows_idle_buffers() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let wars = lock(&sizing.capacities).core[5] + 8;
        let mut reads = 0;
        sizing.raise_from_live_observation(&shared, |capacities| {
            reads += 1;
            assert!(sizing.capacities.try_lock().is_ok());
            capacities.core[5] = wars;
            [None, None]
        });
        assert_eq!(reads, 1);
        assert_eq!(lock(&sizing.capacities).core[5], wars);
        assert_eq!(lock(&sizing.pending).errors, 0);
        assert_idle_war_and_faction_capacities(&shared, sizing);
    }

    #[test]
    fn live_observation_rereads_under_lock_and_reports_only_published_failures() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let initial = *lock(&sizing.capacities);
        let wars = initial.core[5] + 8;
        let factions = initial.core[6] + 8;
        let mut reads = 0;
        sizing.raise_from_live_observation(&shared, |capacities| {
            reads += 1;
            assert_eq!(lock(&sizing.pending).errors, 0);
            if reads == 1 {
                assert!(sizing.capacities.try_lock().is_ok());
                capacities.core[5] = wars + 8;
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            sizing.raise(&shared, |capacities| {
                                capacities.core[6] = factions;
                            });
                        })
                        .join()
                        .unwrap();
                });
                [Some("unpublished table hint failure"), None]
            } else {
                assert_eq!(reads, 2);
                assert!(matches!(
                    sizing.capacities.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                assert_eq!(capacities.core[5], initial.core[5]);
                assert_eq!(capacities.core[6], factions);
                capacities.core[5] = wars;
                [None, Some("published live-input hint failure")]
            }
        });
        assert_eq!(reads, 2);
        let published = *lock(&sizing.capacities);
        assert_eq!(published.core[5], wars);
        assert_eq!(published.core[6], factions);
        assert_idle_war_and_faction_capacities(&shared, sizing);
        let pending = lock(&sizing.pending);
        assert_eq!(pending.errors, 1);
        assert_eq!(
            pending.last_error,
            Some("published live-input hint failure")
        );
    }

    #[test]
    fn recycle_publishes_grown_returning_buffer_before_idle_growth() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let returning = shared.take_frame(&mut 0).unwrap();
        let wars = lock(&sizing.capacities).core[5] + 8;
        let frame = unsafe { &mut *shared.frames[returning].load(Ordering::Acquire) };
        frame.wars_result = engine::walk::Outcome::Truncated {
            len: 0,
            unread_rows: wars as u32,
        };
        let before = frame.allocated_bytes();
        let checked = Arc::new(AtomicBool::new(false));
        let check_seen = Arc::clone(&checked);
        *lock(&sizing.before_recycle_idle_growth) = Some(Box::new(move |sizing, shared| {
            assert!(matches!(
                sizing.capacities.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            assert_eq!(shared.free.len(), sampling::BUFFERS);
            let free: [_; sampling::BUFFERS] = std::array::from_fn(|_| shared.free.pop().unwrap());
            assert!(free.contains(&returning));
            for index in free {
                let frame = unsafe { &*shared.frames[index].load(Ordering::Acquire) };
                if index == returning {
                    assert!(frame.wars.len() >= wars);
                } else {
                    assert!(frame.wars.len() < wars);
                }
                shared.recycle_frame(index);
            }
            check_seen.store(true, Ordering::Relaxed);
        }));
        sizing.recycle_frame(&shared, returning, before, &mut WorkerWork::new());
        assert!(checked.load(Ordering::Relaxed));
        assert!(lock(&sizing.capacities).core[5] >= wars);
        assert_idle_war_and_faction_capacities(&shared, sizing);
    }

    #[test]
    fn recycle_counts_country_intel_growth_during_conversion_once() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        // This fixture has no workers. Include every frame's actual allocation
        // and the one shared Home, independently of the charged pool total.
        let allocated_bytes = || {
            shared
                .frames
                .iter()
                .map(|frame| unsafe { &*frame.load(Ordering::Acquire) }.allocated_bytes())
                .sum::<usize>()
                + shared.country_intel.allocated_bytes()
        };
        let allocated_before = allocated_bytes();
        let pool_before = shared.pool_bytes.load(Ordering::Relaxed);
        let shared_before = shared.country_intel.allocated_bytes();
        let returning = shared.take_frame(&mut 0).unwrap();
        let frame = unsafe { &*shared.frames[returning].load(Ordering::Acquire) };
        // A fresh frame has no CountryIntel loan: Home is free for idle growth.
        let before_frame_bytes = frame.allocated_bytes();
        let initial = *lock(&sizing.capacities);
        let wars = initial.core[5] + 8;
        let intel_rows = initial.extended.intel.rows[15] + 8;

        // Model a sizing raise during A's conversion with the real growth path.
        sizing.raise(&shared, |capacities| {
            capacities.core[5] = wars;
            capacities.extended.raise_for_country_intel(intel_rows);
        });
        assert_eq!(shared.free.len(), sampling::BUFFERS - 1);
        assert_eq!(frame.allocated_bytes(), before_frame_bytes);
        let shared_after_idle = shared.country_intel.allocated_bytes();
        assert!(shared_after_idle > shared_before);
        let allocated_after_idle = allocated_bytes();
        assert!(allocated_after_idle - allocated_before > shared_after_idle - shared_before);
        let pool_after_idle = shared.pool_bytes.load(Ordering::Relaxed);
        assert_eq!(
            pool_after_idle - pool_before,
            (allocated_after_idle - allocated_before) as u64
        );

        sizing.recycle_frame(
            &shared,
            returning,
            before_frame_bytes,
            &mut WorkerWork::new(),
        );
        assert_eq!(shared.country_intel.allocated_bytes(), shared_after_idle);
        let allocated_after_return = allocated_bytes();
        assert!(allocated_after_return > allocated_after_idle);
        let pool_after_return = shared.pool_bytes.load(Ordering::Relaxed);
        assert_eq!(
            pool_after_return - pool_after_idle,
            (allocated_after_return - allocated_after_idle) as u64
        );
        assert_eq!(
            pool_after_return - pool_before,
            (allocated_after_return - allocated_before) as u64
        );
        assert_idle_war_and_faction_capacities(&shared, sizing);
    }

    #[test]
    fn game_score_attachment_config_defaults_enabled_and_accepts_off_switch() {
        let mut value = json!({
            "session_id": 1, "output_dir": "synthetic/session", "replay_dir": "synthetic/replays",
            "session_start": [], "ready_event": "synthetic-ready", "detach_event": "synthetic-detach",
            "done_event": "synthetic-done", "cancel_file": "synthetic/cancel",
        });
        assert!(
            serde_json::from_value::<Config>(value.clone())
                .unwrap()
                .game_score
        );
        value["game_score"] = json!(false);
        assert!(
            !serde_json::from_value::<Config>(value.clone())
                .unwrap()
                .game_score
        );
        value["game_score"] = json!("false");
        assert!(serde_json::from_value::<Config>(value).is_err());
    }

    #[cfg(not(feature = "worker_sampler"))]
    #[test]
    fn worker_sampler_config_refuses_nonempty_requests_without_feature() {
        let mut value = json!({
            "session_id": 1, "output_dir": "synthetic/session", "replay_dir": "synthetic/replays",
            "session_start": [], "ready_event": "synthetic-ready", "detach_event": "synthetic-detach",
            "done_event": "synthetic-done", "cancel_file": "synthetic/cancel",
        });
        let absent: Config = serde_json::from_value(value.clone()).unwrap();
        assert!(absent.worker_sampler.is_none());
        assert_eq!(
            refuse_worker_sampler(absent.worker_sampler.as_deref()),
            Ok(())
        );
        value["worker_sampler"] = json!("");
        let empty: Config = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            refuse_worker_sampler(empty.worker_sampler.as_deref()),
            Ok(())
        );
        for requested in ["off", "interval_us=250", " "] {
            value["worker_sampler"] = json!(requested);
            let config: Config = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(config.worker_sampler.as_deref(), Some(requested));
            assert_eq!(
                refuse_worker_sampler(config.worker_sampler.as_deref()),
                Err(
                    "worker_sampler requested but collector built without the worker_sampler feature"
                ),
            );
        }
    }

    #[test]
    fn allocation_fault_config_keeps_healthy_json_unchanged_and_refuses_invalid_faults() {
        let value = json!({
            "session_id": 1,
            "output_dir": "synthetic/session",
            "replay_dir": "synthetic/replays",
            "session_start": [],
            "ready_event": "synthetic-ready",
            "detach_event": "synthetic-detach",
            "done_event": "synthetic-done",
            "cancel_file": "synthetic/cancel",
        });
        let healthy: Config = serde_json::from_value(value.clone()).unwrap();
        assert!(healthy.alloc_fault.is_none());
        assert_eq!(configure_alloc_fault(None), Ok(()));
        let mut requested = value;
        requested["alloc_fault"] = json!({"phase": "spool", "ordinal": 0});
        let config: Config = serde_json::from_value(requested).unwrap();
        assert_eq!(config.alloc_fault.as_ref().unwrap().after_hours, 0);
        let refused = configure_alloc_fault(config.alloc_fault.as_ref());
        #[cfg(feature = "alloc-fault-injection")]
        assert_eq!(refused, Err("alloc_fault ordinal must be nonzero"));
        #[cfg(not(feature = "alloc-fault-injection"))]
        assert_eq!(
            refused,
            Err("alloc_fault requires collector feature alloc-fault-injection")
        );
        let fault: AllocFault = serde_json::from_value(json!({
            "phase": "encode", "ordinal": 1, "after_hours": 3,
        }))
        .unwrap();
        assert_eq!((fault.ordinal, fault.after_hours), (1, 3));
    }

    #[test]
    fn a_refresh_request_preserves_observation_and_is_acknowledged_once() {
        let session = std::env::temp_dir().join(format!("hoi4-refresh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session);
        fs::create_dir_all(&session).unwrap();
        let shared = sampling::Shared::new(0, 0, 1);
        let mut requests = RefreshRequests::default();
        // No request: nothing forced.
        requests.poll(&session, &shared).unwrap();
        assert!(!shared.force_next.load(Ordering::Acquire));
        assert!(!shared.refresh_next.load(Ordering::Acquire));
        fs::write(session.join(REFRESH_REQUEST), "token-7\n").unwrap();
        requests.poll(&session, &shared).unwrap();
        assert!(shared.refresh_next.swap(false, Ordering::AcqRel));
        assert!(!shared.force_next.load(Ordering::Acquire));
        assert_eq!(
            fs::read_to_string(session.join(REFRESH_ACK)).unwrap(),
            "token-7"
        );
        assert!(!session.join(REFRESH_REQUEST).exists());
        // Consumed: a later poll forces nothing more.
        requests.poll(&session, &shared).unwrap();
        assert!(!shared.refresh_next.load(Ordering::Acquire));
        // A request that could not be removed is accepted once, not every hour.
        fs::write(session.join(REFRESH_REQUEST), "token-7").unwrap();
        requests.poll(&session, &shared).unwrap();
        assert!(!shared.refresh_next.load(Ordering::Acquire));
        // An empty token is not a request; a new token is.
        fs::write(session.join(REFRESH_REQUEST), " \n").unwrap();
        requests.poll(&session, &shared).unwrap();
        assert!(!shared.refresh_next.load(Ordering::Acquire));
        fs::write(session.join(REFRESH_REQUEST), "token-8").unwrap();
        requests.poll(&session, &shared).unwrap();
        shared.force_next.store(true, Ordering::Release);
        assert!(shared.refresh_next.swap(false, Ordering::AcqRel));
        assert!(shared.force_next.swap(false, Ordering::AcqRel));
        assert_eq!(requests.last.as_deref(), Some("token-8"));
        let _ = fs::remove_dir_all(&session);
    }

    /// A registry whose three country headers report the first count, and
    /// whose strategic-region roster the second.
    #[derive(Default)]
    struct Registry(std::cell::Cell<u32>, std::cell::Cell<u32>);

    impl engine::Memory for Registry {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            const STATE: usize = 0x1000;
            let mut bytes = [0_u8; 0x400];
            for (field, count) in [
                (cgame_state::COUNTRIES, &self.0),
                (cgame_state::COUNTRY_LINKS, &self.0),
                (cgame_state::COUNTRY_TAGS, &self.0),
                (cgame_state::STRATEGIC_REGIONS, &self.1),
            ] {
                bytes[field..field + 8].copy_from_slice(&0x9000_u64.to_le_bytes());
                bytes[field + 8..field + 12].copy_from_slice(&count.get().to_le_bytes());
                bytes[field + 12..field + 16].copy_from_slice(&count.get().to_le_bytes());
            }
            if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&(STATE as u64).to_le_bytes());
                return true;
            }
            let Some(offset) = address.checked_sub(STATE) else {
                return false;
            };
            match bytes.get(offset..offset + output.len()) {
                Some(source) => {
                    output.copy_from_slice(source);
                    true
                }
                None => false,
            }
        }
    }

    #[test]
    fn logistics_owner_poll_does_no_native_reads_between_bounded_observations() {
        struct Reads(std::cell::Cell<usize>);
        impl engine::Memory for Reads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        let memory = Reads(std::cell::Cell::new(0));
        let mut watch = RegistryWatch::default();
        assert_eq!(watch.poll_logistics(&memory, 0).unwrap(), Some(0));
        let first = memory.0.get();
        assert!(first > 0);
        assert_eq!(watch.poll_logistics(&memory, 0).unwrap(), None);
        assert_eq!(memory.0.get(), first);
        watch.last_logistics = Some(Instant::now() - RegistryWatch::INTEL_INTERVAL);
        assert_eq!(watch.poll_logistics(&memory, 0).unwrap(), Some(0));
        assert!(memory.0.get() > first);
    }
    #[test]
    fn registry_watch_reports_growth_once_and_ignores_torn_or_smaller_counts() {
        let memory = Registry::default();
        let mut watch = RegistryWatch::default();
        // The main menu has an empty registry: nothing to presize.
        assert_eq!(watch.poll(&memory, 0), None);
        // A save loads 5,001 slots; the next poll after the interval sees it.
        memory.0.set(5_001);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), Some((5_001, 0)));
        // Throttled, then unchanged.
        assert_eq!(watch.poll(&memory, 0), None);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
        // The region roster fills later: the slots are reported with it.
        memory.1.set(2_500);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), Some((5_001, 2_500)));
        // A smaller registry or roster never shrinks capacities.
        memory.0.set(400);
        memory.1.set(100);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
        // A count past the engine's u16 slot ceiling is a torn read, not a size.
        memory.0.set(1 << 20);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
    }

    #[test]
    fn startup_sizing_retries_an_empty_world_and_presizes_regions_without_raids() {
        let memory = Registry::default();
        let mut watch = RegistryWatch::default();
        let mut capacities = crate::extended::Capacities::default();
        for _ in 0..24 {
            watch.last_game_score = None;
            watch.last_lines = None;
            assert!(watch.poll_game_score(&memory, 0).is_none());
            assert!(watch.poll_lines(&memory, 0).is_none());
        }
        memory.0.set(5_001);
        memory.1.set(2_500);
        watch.last_game_score = None;
        assert!(watch.poll_game_score(&memory, 0).is_some());
        // No character, production or raid system exists in this fixture.
        assert!(capacities.presize_live_inputs(&memory, 0).is_err());
        assert_eq!(capacities.air.scores.regions, 2_500);
    }

    #[test]
    fn land_detail_watch_presizes_growth_without_country_registry_changes() {
        use engine::{cland_combat as combat, cland_combatant as side, cprovince, cterrain_type};

        struct Combats(Vec<u8>);
        impl Combats {
            fn put(&mut self, at: usize, bytes: &[u8]) {
                self.0[at - 0x1000..at - 0x1000 + bytes.len()].copy_from_slice(bytes);
            }
            fn word(&mut self, at: usize, value: u64) {
                self.put(at, &value.to_le_bytes());
            }
            fn array(&mut self, at: usize, data: u64, count: u32) {
                self.word(at, data);
                self.put(at + 8, &count.to_le_bytes());
                self.put(at + 12, &count.to_le_bytes());
            }
        }
        impl engine::Memory for Combats {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
                    output.copy_from_slice(&0x1000u64.to_le_bytes());
                    return true;
                }
                engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &self.0,
                }
                .copy(address, output)
            }
        }
        let mut memory = Combats(vec![0; 0x6000]);
        memory.array(0x1000 + cgame_state::COUNTRIES, 0x5800, 2);
        memory.word(0x2000, combat::VTABLE_RVA as u64);
        memory.word(0x2000 + combat::BODY, combat::BODY_VTABLE_RVA as u64);
        memory.word(
            0x2000 + combat::HANDLE,
            (1 << 32) | u64::from(combat::TYPE_WORD),
        );
        memory.word(0x2000 + combat::ATTACKER, 0x3000);
        memory.word(0x2000 + combat::DEFENDER, 0x4000);
        memory.word(0x2000 + combat::STORED_TERRAIN, 0x5000);
        memory.word(0x5000, cterrain_type::NULL_VTABLE_RVA as u64);
        for (pointer, attacker) in [(0x3000, 1), (0x4000, 0)] {
            memory.word(pointer, side::VTABLE_RVA as u64);
            memory.word(pointer + side::COMBAT, 0x2000);
            memory.put(pointer + side::IS_ATTACKER, &[attacker]);
        }
        memory.word(0x2000 + combat::PROVINCE, 0x5200);
        memory.word(0x5200, cprovince::VTABLE_RVA as u64);
        memory.put(0x5200 + cprovince::ID, &1u32.to_le_bytes());
        memory.word(0x5408, 0x5200);
        memory.array(0x1000 + cgame_state::PROVINCES, 0x5400, 2);
        for index in 0..71 {
            memory.word(0x6000 + index * 8, 0x2000);
        }
        let roster = 0x1000 + cgame_state::COMBATS + 8;
        memory.array(roster, 0x6000, 66);
        let mut watch = RegistryWatch::default();
        let initial = watch
            .poll_combat_hints(&memory, 0)
            .and_then(|hints| hints.land)
            .unwrap();
        assert_eq!(initial.rows, 66);
        memory.array(roster, 0x6000, 71);
        assert!(watch.poll_combat_hints(&memory, 0).is_none()); // throttled
        watch.last_combat_hints = None;
        assert_eq!(
            watch
                .poll_combat_hints(&memory, 0)
                .unwrap()
                .land
                .unwrap()
                .rows,
            71
        );
        // An unreadable air payload contributes no hint; a valid one grows it.
        memory.array(0x3000 + side::AIR_SUPPORT, 0x7fff_0000, 300);
        watch.last_combat_hints = None;
        assert!(watch.poll_combat_hints(&memory, 0).unwrap().land.is_none());
        for index in 0..300 {
            memory.word(0x5600 + index * 8, 0x5500);
        }
        memory.array(0x3000 + side::AIR_SUPPORT, 0x5600, 300);
        watch.last_combat_hints = None;
        let raised = watch
            .poll_combat_hints(&memory, 0)
            .and_then(|hints| hints.land)
            .unwrap();
        assert_eq!(raised.arenas.air, 71 * 300);
        memory.array(roster, 0x6000, 1);
        watch.last_combat_hints = None;
        assert_eq!(
            watch
                .poll_combat_hints(&memory, 0)
                .unwrap()
                .land
                .unwrap()
                .rows,
            1
        );
        memory.put(roster + 12, &72u32.to_le_bytes()); // invalid count/capacity
        watch.last_combat_hints = None;
        assert!(watch.poll_combat_hints(&memory, 0).unwrap().land.is_none());
    }

    #[test]
    fn naval_detail_watch_presizes_growth_without_country_registry_changes() {
        use crate::extended::naval_details::fixtures::{Fixture, IMAGE, STATE};

        let mut memory = Fixture::new(2);
        let roster = STATE + cgame_state::COMBATS + 8;
        memory.array(roster, 0x4000, 1);
        let mut watch = RegistryWatch::default();
        let initial = watch
            .poll_combat_hints(&memory, IMAGE)
            .and_then(|hints| hints.naval)
            .unwrap();
        assert_eq!(initial.rows, 1);
        assert!(initial.ships > 0);
        memory.array(roster, 0x4000, 2);
        assert!(watch.poll_combat_hints(&memory, IMAGE).is_none());
        watch.last_combat_hints = None;
        let raised = watch
            .poll_combat_hints(&memory, IMAGE)
            .and_then(|hints| hints.naval)
            .unwrap();
        assert_eq!(raised.rows, 2);
        assert!(raised.ships > initial.ships);
        memory.array(roster, 0x4000, 1);
        watch.last_combat_hints = None;
        assert_eq!(
            watch
                .poll_combat_hints(&memory, IMAGE)
                .unwrap()
                .naval
                .unwrap()
                .rows,
            1
        );
        memory.u32(roster + 12, 2); // count exceeds capacity
        watch.last_combat_hints = None;
        assert!(
            watch
                .poll_combat_hints(&memory, IMAGE)
                .unwrap()
                .naval
                .is_none()
        );
    }

    #[test]
    fn combat_hints_skip_failed_reserves_without_latching_demand() {
        let shared = Shared::new(0, 0, 1);
        let capacities = crate::extended::Capacities::default();
        let mut applied = [CombatHintState::default(); sampling::BUFFERS];
        let before = crate::country_scale::combat_hint_status();
        let hints = CombatHints {
            land: Some(engine::cland_combat::DetailDemand {
                rows: usize::MAX,
                ..Default::default()
            }),
            naval: Some(engine::naval_combat_demand::Demand {
                rows: usize::MAX,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        let stopped = crate::allocation::failed();
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied == [CombatHintState::default(); sampling::BUFFERS]);
        assert_eq!(crate::allocation::failed(), stopped);
        let after = crate::country_scale::combat_hint_status();
        let before = serde_json::to_value(before).unwrap();
        let after = serde_json::to_value(after).unwrap();
        for name in ["land_skipped", "naval_skipped"] {
            assert!(
                after[name].as_u64().unwrap()
                    >= before[name].as_u64().unwrap() + 2 * sampling::BUFFERS as u64
            );
        }
        assert!(capacities == crate::extended::Capacities::default());
        // A later valid observation is retried, without the failed huge peak.
        let before = shared.pool_bytes.load(Ordering::Relaxed);
        let valid = CombatHints {
            land: Some(engine::cland_combat::DetailDemand {
                rows: 600,
                ..Default::default()
            }),
            naval: Some(engine::naval_combat_demand::Demand {
                rows: 1,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        grow_combat_hints(&shared, &capacities, &valid, &mut applied);
        assert!(
            applied
                .iter()
                .all(|state| state.land.is_some() && state.naval.is_some())
        );
        assert!(shared.pool_bytes.load(Ordering::Relaxed) > before);
        assert_eq!(shared.free.len(), sampling::BUFFERS);
    }

    #[test]
    fn combat_hints_keep_applied_state_and_grow_directly_recycled_frames() {
        let shared = Shared::new(0, 0, 1);
        let capacities = crate::extended::Capacities::default();
        let mut applied = [CombatHintState::default(); sampling::BUFFERS];
        let mut hints = CombatHints {
            land: Some(engine::cland_combat::DetailDemand {
                rows: 600,
                ..Default::default()
            }),
            naval: Some(engine::naval_combat_demand::Demand {
                rows: 1,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        let grown = applied;
        assert!(
            grown
                .iter()
                .all(|state| state.land.is_some() && state.naval.is_some())
        );
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied == grown);
        hints.land.as_mut().unwrap().rows = 100;
        hints.naval.as_mut().unwrap().rows = 0;
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied == grown);

        // A leased frame must still receive a later hint when it returns.
        let held = shared.free.pop().unwrap();
        let previous = applied[held];
        hints.land.as_mut().unwrap().rows = 1200;
        hints.naval.as_mut().unwrap().rows = 257;
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied[held] == previous);
        let grown = applied;
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied == grown);
        // A queue-full drop returns directly to free, without a ready pop.
        shared.recycle_frame(held);
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied[held].land != previous.land);
        assert!(applied[held].naval != previous.naval);
        let grown = applied;
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        assert!(applied == grown);
        assert_eq!(shared.free.len(), sampling::BUFFERS);
    }

    #[test]
    fn demand_polling_keeps_named_failures_and_does_not_raise_seen_demand() {
        let memory = Registry::default();
        let mut watch = RegistryWatch::default();
        assert_eq!(watch.poll_politics(&memory, 0), Ok(None));
        assert_eq!(watch.poll_characters(&memory, 0), Ok(None));
        memory.0.set(5_001);
        watch.last_politics = None;
        watch.last_characters = None;
        assert_eq!(
            watch.poll_politics(&memory, 0),
            Err("character manager unreadable or null")
        );
        assert!(watch.poll_characters(&memory, 0).is_err());
        assert_eq!(
            watch.politics,
            crate::country_scale::PoliticsDemand::default()
        );
        assert_eq!(
            watch.characters,
            crate::country_scale::CharacterDemand::default()
        );
        // The interval limits observations, without turning a failure into zero.
        assert_eq!(watch.poll_politics(&memory, 0), Ok(None));
        assert_eq!(watch.poll_characters(&memory, 0), Ok(None));
    }

    #[test]
    fn raid_path_memo_growth_presizes_before_refresh() {
        use engine::{Memory, craid_system, raid_path_memo};
        use std::cell::Cell;
        // Five thousand null country slots, with no raid-parent payloads.
        // The memo buckets are deliberately unreadable: sizing needs headers.
        struct Memo {
            registry: Registry,
            mask: Cell<u32>,
        }
        impl Memory for Memo {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                const STATE: usize = 0x1000;
                const SYSTEM: usize = 0x8000;
                if address == STATE + cgame_state::RAID_SYSTEM && output.len() == 8 {
                    output.copy_from_slice(&(SYSTEM as u64).to_le_bytes());
                    return true;
                }
                if (0x9008..0x9000 + 5_001 * 8).contains(&address) && output.len() == 8 {
                    output.fill(0);
                    return true;
                }
                let mut bytes = [0_u8; 0x300];
                bytes[..8].copy_from_slice(&(craid_system::VTABLE_RVA as u64).to_le_bytes());
                let memo = raid_path_memo::MEMO;
                bytes[memo + 8..memo + 16].copy_from_slice(&0x10_0000_u64.to_le_bytes());
                bytes[memo + 0x14..memo + 0x18].copy_from_slice(&self.mask.get().to_le_bytes());
                bytes[memo + 0x18] = (self.mask.get() + 1).ilog2() as u8 + 2;
                if let Some(source) = address
                    .checked_sub(SYSTEM)
                    .and_then(|offset| bytes.get(offset..offset + output.len()))
                {
                    output.copy_from_slice(source);
                    return true;
                }
                self.registry.copy(address, output)
            }
        }
        let memory = Memo {
            registry: Registry(Cell::new(5_001), Cell::new(0)),
            mask: Cell::new(2_047),
        };
        let mut sizing = crate::country_scale::Sizing::for_slots(5_001);
        assert_eq!(sizing.extended.projects.rows[12], 0);
        let initial = crate::country_scale::observe_raid_arenas(&memory, 0)
            .unwrap()
            .unwrap();
        assert_eq!(initial.path_rows, 2_048 + 13);
        sizing.extended.raise_for_raid_arenas(initial);
        assert!(sizing.extended.projects.rows[12] >= initial.path_rows);
        let mut watch = RegistryWatch {
            raids: initial,
            ..Default::default()
        };
        assert_eq!(watch.poll_raids(&memory, 0).unwrap(), Some(initial));
        // Optional unreadable hints remain reportable on each unthrottled poll.
        assert!(initial.failures.into_iter().any(|reason| reason.is_some()));
        // Rehash alone grows: tokens and reservations remain unchanged.
        memory.mask.set(8_191);
        assert_eq!(watch.poll_raids(&memory, 0).unwrap(), None); // throttled
        watch.last_raids = None;
        let demand = watch.poll_raids(&memory, 0).unwrap().unwrap();
        assert_eq!(demand.path_rows, 8_192 + 15);
        assert_eq!(demand.type_tokens, initial.type_tokens);
        assert_eq!(demand.reservation_words, initial.reservation_words);
        sizing.extended.raise_for_raid_arenas(demand);
        assert!(sizing.extended.projects.rows[12] >= demand.path_rows);
        watch.last_raids = None;
        assert_eq!(watch.poll_raids(&memory, 0).unwrap(), Some(demand));
        memory.mask.set(1_023);
        watch.last_raids = None;
        assert_eq!(watch.poll_raids(&memory, 0).unwrap(), Some(demand));
        assert_eq!(watch.raids.path_rows, demand.path_rows);
    }

    #[test]
    fn status_replaces_while_a_delete_sharing_reader_holds_it_open() {
        use std::os::windows::fs::OpenOptionsExt;
        /// Removes the directory even when an assertion fails.
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let raw = std::env::temp_dir().join(format!("hoi4-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&raw);
        fs::create_dir_all(&raw).unwrap();
        // Production writes through canonical \\?\ paths.
        let dir = TempDir(fs::canonicalize(&raw).unwrap());
        assert!(dir.0.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        let path = dir.0.join("status.json");
        write_status(&path, &json!({"hours": 1})).unwrap();
        // A reader that shares delete, as the recorder, bench and smoke do.
        let shared = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        write_status(&path, &json!({"hours": 2})).unwrap();
        let current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current["hours"], 2);
        drop(shared);
        // A reader without delete sharing still blocks it, within the bounded retry.
        let exclusive = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let error = write_status(&path, &json!({"hours": 3})).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(5 | 32)));
        drop(exclusive);
        write_status(&path, &json!({"hours": 4})).unwrap();
        // This local NTFS directory supports POSIX rename.
        assert!(!POSIX_RENAME_UNSUPPORTED.load(Ordering::Relaxed));
    }

    #[test]
    fn status_rename_retries_only_transient_errors_before_deadline() {
        for code in [5, 32] {
            let error = io::Error::from_raw_os_error(code);
            assert_eq!(
                status_rename_retry_delay(&error, Duration::ZERO),
                Some(Duration::from_millis(1))
            );
            assert_eq!(
                status_rename_retry_delay(&error, Duration::from_micros(29_500)),
                Some(Duration::from_micros(500))
            );
            assert_eq!(
                status_rename_retry_delay(&error, STATUS_RENAME_RETRY_BUDGET),
                None
            );
            assert_eq!(
                status_rename_retry_delay(&error, Duration::from_millis(31)),
                None
            );
        }
        for code in [2, 3, 87, 112] {
            assert_eq!(
                status_rename_retry_delay(&io::Error::from_raw_os_error(code), Duration::ZERO),
                None
            );
        }
        assert_eq!(
            status_rename_retry_delay(
                &io::Error::from(io::ErrorKind::PermissionDenied),
                Duration::ZERO
            ),
            None
        );
    }

    #[test]
    fn racing_political_hint_counts_in_completed_status_without_a_session_warning() {
        use engine::{Memory, ccountry};
        use std::cell::Cell;

        struct RacingHint {
            bytes: Vec<u8>,
            roster_reads: Cell<usize>,
        }
        impl Memory for RacingHint {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
                    output.copy_from_slice(&0x1000_u64.to_le_bytes());
                    return true;
                }
                let Some(source) = address
                    .checked_add(output.len())
                    .and_then(|end| self.bytes.get(address..end))
                else {
                    return false;
                };
                output.copy_from_slice(source);
                if address == 0x4000 + ccountry::DIVISIONS && output.len() == 16 {
                    let reads = self.roster_reads.get() + 1;
                    self.roster_reads.set(reads);
                    // The count and seed passes see the same empty roster;
                    // native growth races its final selecting-header check.
                    if reads >= 3 {
                        output[8..12].copy_from_slice(&1_u32.to_le_bytes());
                    }
                }
                true
            }
        }
        fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
            bytes[at..at + value.len()].copy_from_slice(value);
        }
        let mut memory = RacingHint {
            bytes: vec![0; 0x6000],
            roster_reads: Cell::new(0),
        };
        for (field, data) in [
            (cgame_state::COUNTRIES, 0x3000_u64),
            (cgame_state::COUNTRY_LINKS, 0x3100),
            (cgame_state::PROVINCES, 0x3200),
        ] {
            put(&mut memory.bytes, 0x1000 + field, &data.to_le_bytes());
            put(&mut memory.bytes, 0x1000 + field + 8, &2_u32.to_le_bytes());
            put(&mut memory.bytes, 0x1000 + field + 12, &2_u32.to_le_bytes());
        }
        put(&mut memory.bytes, 0x3008, &0x4000_u64.to_le_bytes());
        put(&mut memory.bytes, 0x3104, &1_u32.to_le_bytes());
        put(
            &mut memory.bytes,
            0x4000,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut memory.bytes,
            0x4000 + ccountry::ID,
            &1_u32.to_le_bytes(),
        );

        let session = SessionStart {
            id: 1,
            process_id: 100,
            process_start_time: 1000,
            qpc_frequency: 1,
            sample_boundary: replay::identity::SampleBoundary::EndOfHour,
            identity: replay::identity::Identity {
                game_version: "fixture".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "fixture".into(),
                collector_hash: [3; 32],
                verification: replay::identity::Verification::Unchecked,
            },
        };
        let directory = std::env::temp_dir()
            .join(format!("hoi4-political-hint-status-{}", std::process::id(),));
        let mut processor =
            Processor::new(&directory, PreparedCampaigns::default(), session.clone()).unwrap();
        let mut watch = RegistryWatch::default();
        assert!(
            processor
                .observe_input_demand(watch.poll_presentation_census(&memory, 0))
                .flatten()
                .is_none()
        );
        assert_eq!(memory.roster_reads.get(), 3);
        assert_eq!(
            watch.presentation_census,
            engine::carmy::PresentationDemand::default(),
        );
        // Throttling is not another failure observation.
        assert!(
            processor
                .observe_input_demand(watch.poll_presentation_census(&memory, 0))
                .flatten()
                .is_none()
        );

        let shared = Shared::new(0, 0, 1);
        shared.recording.store(false, Ordering::Release);
        let writer = crate::status_writer::StatusWriter::start(|_, _| Ok(())).unwrap();
        let work = WorkerWork::new();
        let handle = writer.handle();
        let status = serde_json::to_value(
            spool_status(
                &processor,
                &shared,
                &session,
                &work,
                &handle,
                "detached",
                0,
                None,
                None,
                true,
                true,
                TeardownStatus::default(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(status["input_demand_errors"], 1);
        assert_eq!(
            status["input_demand_last_error"],
            "division presentation demand selecting header changed",
        );
        assert_eq!(status["write_errors"], 0);
        assert_eq!(status["drain_complete"], true);
        assert!(status.get("warning").is_none());
        assert!(status.get("error").is_none());
        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovered_warnings_do_not_become_terminal_drain_failures() {
        let mut status = json!({"error":"old transient error", "warning":"recovered write failure", "last_error":"recovered write failure", "write_errors":1});
        set_drain_status(&mut status, &Ok(()));
        assert_eq!(status["drain_complete"], true);
        assert!(status.get("error").is_none());
        assert_eq!(status["warning"], "recovered write failure");
        assert_eq!(status["write_errors"], 1);
    }

    #[test]
    fn final_spool_status_preserves_metadata_and_adds_event_feed_detach_losses() {
        let mut bytes =
            br#"{"spool_closed":true,"hours":2,"decode":{"event_feed":{"native_flights":0}}}"#
                .to_vec();
        append_event_feed_detach_status(
            &mut bytes,
            Some(crate::event_feed_history::DetachLosses {
                unrecorded_tail_events: 1,
                unrecorded_tail_revisions: 2,
                failed_batches: 3,
            }),
        )
        .unwrap();
        let status: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status["spool_closed"], true);
        assert_eq!(status["hours"], 2);
        assert_eq!(status["decode"]["event_feed"]["native_flights"], 0);
        assert_eq!(status["event_feed_detach"]["unrecorded_tail_events"], 1);
        assert_eq!(status["event_feed_detach"]["unrecorded_tail_revisions"], 2);
        assert_eq!(status["event_feed_detach"]["failed_batches"], 3);
    }

    #[test]
    fn terminal_failures_remain_explicit_even_with_recovered_history() {
        let mut result = Ok(());
        add_terminal_error(&mut result, "final sync failed");
        add_terminal_error(&mut result, "terminal status unavailable");
        let mut status = json!({"warning":"earlier failure recovered"});
        set_drain_status(&mut status, &result);
        assert_eq!(status["drain_complete"], false);
        assert_eq!(status["error"], "final sync failed");
        assert_eq!(status["warning"], "earlier failure recovered");
    }

    #[test]
    fn decode_status_merge_is_additive_and_keeps_existing_keys() {
        let mut status = json!({
            "state": "recording", "hours": 3,
            "table_reads_recorded": true, "table_reads_frames": 3,
            "table_reads": [{"bytes": 42}], "table_total_reads": [{"bytes": 123}],
        });
        add_decode_status(&mut status);
        assert_eq!(status["state"], "recording");
        assert_eq!(status["hours"], 3);
        assert_eq!(status["replay_format"], replay::FORMAT_VERSION);
        assert_eq!(status["table_reads_recorded"], true);
        assert_eq!(status["table_reads_frames"], 3);
        assert_eq!(status["table_reads"][0]["bytes"], 42);
        assert_eq!(status["table_total_reads"][0]["bytes"], 123);
        assert_eq!(
            status["decode_work"]["relations_tables"][0],
            "RelationOpinions"
        );
        assert_eq!(
            status["decode_work"]["relations_tables"][1],
            "RelationTuples"
        );
        assert!(status.get("research_decode").is_none());
    }
}
