//! Loader worker, spool thread, and restore/drain/self-unload lifecycle.

use crate::{
    acquisition_jobs as acquisition, decode,
    history::PreparedCampaigns,
    hook::{Hook, Threads},
    sampling::{self, ACTIVE, SHARED, Shared},
    spooling::{Distribution, Processor},
};
use engine::{
    PINNED_BUILD, build::hash_executable, cgame_state, country_memory::SeedStatus,
    memory::LiveMemory, read,
};
use replay::{codec::Decoder, identity::SessionStart};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    ffi::{OsString, c_void},
    fs::{self, File},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HMODULE, WAIT_OBJECT_0},
    Storage::FileSystem::{
        CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_RENAME_INFO, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileRenameInfoEx, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING, SetFileInformationByHandle,
    },
    System::{
        LibraryLoader::{FreeLibraryAndExitThread, GetModuleFileNameW, GetModuleHandleW},
        Performance::QueryPerformanceFrequency,
        Threading::*,
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
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
}

struct Event(HANDLE);
impl Event {
    fn create(name: &str) -> Result<Self, String> {
        if name.is_empty() || name.contains('\0') {
            return Err("event name is empty or contains NUL".into());
        }
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, name.as_ptr()) };
        if handle.is_null() {
            Err(io::Error::last_os_error().to_string())
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

fn module_path(module: HMODULE) -> Result<PathBuf, String> {
    let mut buffer = vec![0; 32768];
    let len =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len == buffer.len() {
        return Err("GetModuleFileNameW failed or truncated".into());
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len])))
}

pub unsafe extern "system" fn worker(module: *mut c_void) -> u32 {
    let result = run(module);
    if let Err(error) = result
        && let Ok(path) = module_path(module)
    {
        let _ = fs::write(path.with_file_name("collector.error.txt"), error);
    }
    // Any retained shared pointer means code can still be executing through the hook.
    if !SHARED.load(Ordering::Acquire).is_null() {
        return 1;
    }
    unsafe {
        FreeLibraryAndExitThread(module, 0);
    }
}

fn write_status(path: &Path, status: &Value) -> io::Result<()> {
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(status).map_err(|error| {
        io::Error::other(format!(
            "cannot serialize collector status for {}: {error}",
            path.display()
        ))
    })?;
    fs::write(&temp, bytes).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot write status temporary file {} for {}: {error}",
                temp.display(),
                path.display()
            ),
        )
    })?;
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

/// `FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS`.
const RENAME_REPLACE_POSIX: u32 = 0x1 | 0x2;

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
    // The name excludes its terminator; the struct keeps one u16 for it.
    let name = &target[..target.len() - 1];
    let header = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = header + std::mem::size_of_val(name) + 2;
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    let result = unsafe {
        (*info).Anonymous.Flags = RENAME_REPLACE_POSIX;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = std::mem::size_of_val(name) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        let ok = SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), size as u32);
        let error = io::Error::last_os_error();
        CloseHandle(handle);
        if ok != 0 { Ok(true) } else { Err(error) }
    };
    match result {
        // Volumes without POSIX rename (FAT, older NTFS drivers) report these.
        Err(error) if matches!(error.raw_os_error(), Some(1 | 50 | 87)) => Ok(false),
        other => other,
    }
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
    let mut attempts = 0;
    // With POSIX rename a delete-sharing reader (the recorder, bench and smoke)
    // never blocks replacement. A reader without delete sharing still does, as
    // does any reader after the classic fallback, so keep the bounded retry.
    // It runs off the game thread; OS I/O and scheduling are not time-bounded.
    let error = loop {
        attempts += 1;
        let error = match replace_once(&source, &target) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let Some(delay) = status_rename_retry_delay(&error, started.elapsed()) else {
            break error;
        };
        std::thread::sleep(delay);
        if started.elapsed() >= STATUS_RENAME_RETRY_BUDGET {
            break error;
        }
    };
    Err(io::Error::new(
        error.kind(),
        format!(
            "cannot atomically replace status {} from {} after {attempts} attempts ({:.3} ms elapsed): {error}",
            path.display(),
            temp.display(),
            started.elapsed().as_secs_f64() * 1000.0
        ),
    ))
}

struct WorkerWork {
    buffer_service: Distribution,
    idle_flush_calls: u64,
    idle_flush_ns: u64,
    idle_flush_max_ns: u64,
    status_update: Distribution,
    queue_peak_observed: usize,
    started_qpc: i64,
    started_cpu_ns: Result<u64, String>,
}

impl WorkerWork {
    fn new() -> Self {
        Self {
            buffer_service: Distribution::default(),
            idle_flush_calls: 0,
            idle_flush_ns: 0,
            idle_flush_max_ns: 0,
            status_update: Distribution::default(),
            queue_peak_observed: 0,
            started_qpc: sampling::qpc(),
            started_cpu_ns: thread_cpu_ns().map_err(|error| error.to_string()),
        }
    }

    /// Once per status update. CPU time includes all work on the spool thread;
    /// buffer_service contains the conversion/encoding/write phases, not their sum.
    fn add_status(&self, status: &mut Value, frequency: u64) {
        let hours = status.get("hours").and_then(Value::as_u64).unwrap_or(0);
        if !status["spool_work"].is_object() {
            status["spool_work"] = json!({});
        }
        let fields = &mut status["spool_work"];
        fields["buffer_service"] = self.buffer_service.json();
        fields["idle_flush"] = json!({
            "calls": self.idle_flush_calls,
            "total_ns": self.idle_flush_ns,
            "max_ns": self.idle_flush_max_ns,
            "mean_ns": self.idle_flush_ns.checked_div(self.idle_flush_calls).unwrap_or(0),
        });
        fields["idle_flush_scope"] = json!(
            "spool-thread wall in Processor::flush after draining ready buffers; includes periodic durable synchronization; separate from buffer_service and status_update"
        );
        fields["status_update"] = self.status_update.json();
        fields["queue_peak_observed"] = json!(self.queue_peak_observed);
        let elapsed_ns = ((sampling::qpc() - self.started_qpc).max(0) as u128 * 1_000_000_000
            / u128::from(frequency)) as u64;
        fields["elapsed_ns"] = json!(elapsed_ns);
        match (&self.started_cpu_ns, thread_cpu_ns()) {
            (Ok(start), Ok(now)) => {
                let cpu_ns = now.saturating_sub(*start);
                fields["cpu_total_ns"] = json!(cpu_ns);
                fields["cpu_ns_per_recorded_hour"] = json!(if hours == 0 {
                    None
                } else {
                    Some(cpu_ns as f64 / hours as f64)
                });
            }
            (Err(error), _) => fields["cpu_error"] = json!(error),
            (_, Err(error)) => fields["cpu_error"] = json!(error.to_string()),
        }
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

fn run(module: HMODULE) -> Result<(), String> {
    let dll_path = module_path(module)?;
    let config: Config = serde_json::from_slice(
        &fs::read(dll_path.with_file_name("collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut input = Decoder::new(&config.session_start);
    let session = SessionStart::decode(&mut input).map_err(|e| e.to_string())?;
    input.finish().map_err(|e| e.to_string())?;
    if session.id != config.session_id || session.process_id != unsafe { GetCurrentProcessId() } {
        return Err("collector config names another session or process".into());
    }
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
        return Err(io::Error::last_os_error().to_string());
    }
    let process_start =
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    if session.process_start_time != process_start {
        return Err("collector config process creation time differs".into());
    }
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || frequency <= 0
        || session.qpc_frequency != frequency as u64
    {
        return Err("collector config QPC frequency differs".into());
    }
    if session.sample_boundary != sampling::SAMPLE_BOUNDARY {
        return Err(format!(
            "collector samples at the {} boundary; the session declares {}",
            sampling::SAMPLE_BOUNDARY,
            session.sample_boundary
        ));
    }
    let exe =
        hash_executable(File::open(module_path(ptr::null_mut())?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if !PINNED_BUILD.matches(&exe) {
        return Err(format!(
            "unsupported game build; wanted {} ({} bytes, {:02x?}), got {} bytes, {:02x?}",
            PINNED_BUILD.version,
            PINNED_BUILD.exe_size,
            PINNED_BUILD.exe_sha256,
            exe.size,
            exe.sha256
        ));
    }
    if session.identity.exe_hash != exe.sha256 || session.identity.exe_size != exe.size {
        return Err("collector config executable identity differs from running build".into());
    }
    let dll = hash_executable(File::open(&dll_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if session.identity.collector_hash != dll.sha256 {
        return Err("collector DLL hash differs from session identity".into());
    }
    fs::create_dir_all(&config.output_dir).map_err(|e| e.to_string())?;
    let ready = Event::create(&config.ready_event)?;
    let detach = Event::create(&config.detach_event)?;
    let done = Event::create(&config.done_event)?;
    let status_path = config.output_dir.join("status.json");
    write_status(
        &status_path,
        &json!({"state":"starting", "session":session.id, "hours":0}),
    )
    .map_err(|e| e.to_string())?;
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let memory = unsafe { LiveMemory::new() };
    let rva = cgame_state::HOURLY_UPDATE_RVA;
    let sentinel = cgame_state::HOURLY_UPDATE_SENTINEL;
    if read::<[u8; 16]>(&memory, base + rva) != Some(sentinel) {
        return Err(format!("hourly boundary sentinel differs at RVA {rva:#x}"));
    }
    // The recorder uses this same exclusive handle before every finalization.
    // It protects both existing histories and campaigns with no replay yet.
    let _output_lock = reserve_output(&config, &detach)?;
    let history = PreparedCampaigns::load(&config.replay_dir, &session, |campaign| {
        if config.cancel_file.exists() || detach.signaled() {
            return Err(io::Error::other(
                "recording cancelled during history preparation",
            ));
        }
        write_status(
            &status_path,
            &json!({
                "state":"preparing campaign history", "session":session.id,
                "campaign":campaign, "hours":0,
            }),
        )
    })
    .map_err(|error| error.to_string())?;
    let processor = Processor::new(&config.output_dir, history, session.clone())
        .map_err(|error| error.to_string())?;
    let mut hook =
        unsafe { Hook::new(base + rva, sampling::tick as *const () as usize, sentinel) }?;
    let mut portrait_hooks = unsafe { crate::portrait_changes::Hooks::new(&memory, base) }?;
    let mut advisor_hooks = unsafe { crate::advisor_changes::Hooks::new(&memory, base) }?;
    // A family whose sentinels differ is left uninstalled; it reads as before.
    let mut opinion_hooks = unsafe { crate::opinion_changes::Hooks::new(&memory, base) }?;
    // Size every country-proportional buffer from the live registry before
    // the hook exists. At the main menu the registry may still be empty; the
    // spool thread then raises capacities as soon as a save fills it.
    let slots = crate::country_scale::observe(&memory, base).unwrap_or(0);
    crate::country_scale::note_slots(slots);
    let mut sizing = crate::country_scale::Sizing::for_slots(slots);
    sizing.content = engine::cdlc_manager::observe_counts(&memory, base);
    if let Some(rows) = crate::country_scale::observe_country_intel(&memory, base) {
        sizing.extended.raise_for_country_intel(rows);
    }
    if let Some(demand) = crate::country_scale::observe_raid_arenas(&memory, base) {
        sizing.extended.raise_for_raid_arenas(demand);
    }
    if let Some(demand) = crate::country_scale::observe_characters(&memory, base) {
        sizing.extended.raise_for_characters(demand);
    }
    if let Some(demand) = crate::country_scale::observe_politics(&memory, base) {
        sizing.extended.raise_for_politics(demand);
    }
    let shared = Arc::new(Shared::with_sizing(
        base,
        hook.trampoline(),
        frequency as u64,
        &sizing,
    ));
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
        .map_err(|_| "acquisition pool already initialized".to_string())?;
    let _payload_pools = decode::prepare_payload_pools();
    let spool_id = Arc::new(AtomicU32::new(0));
    if config.cancel_file.exists() || detach.signaled() {
        return Err("recording cancelled before hook installation".into());
    }
    let spool_shared = Arc::clone(&shared);
    let spool_thread_id = Arc::clone(&spool_id);
    let spool_status = status_path.clone();
    // Owned here and finished only after the spool thread has returned and
    // callbacks are quiescent; the spool thread gets a hand-over handle.
    let mut status_writer =
        crate::status_writer::StatusWriter::start(write_status).map_err(|e| e.to_string())?;
    let spool_writer = status_writer.handle();
    let session_id = session.id;
    let spool = std::thread::Builder::new()
        .name("hoi4-spool".into())
        .spawn(move || {
            spool_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            spool_loop(
                &spool_shared,
                &config,
                session,
                processor,
                &spool_status,
                sizing,
                spool_writer,
            )
        })
        .map_err(|e| e.to_string())?;
    let mut installation_error = decode::start_worker().err();
    SHARED.store(Arc::as_ptr(&shared).cast_mut(), Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(15);
    while installation_error.is_none() {
        match advisor_hooks
            .set_enabled(true)
            .and_then(|ready| {
                if ready {
                    portrait_hooks.set_enabled(true)
                } else {
                    Ok(false)
                }
            })
            .and_then(|ready| {
                if ready {
                    opinion_hooks.set_enabled(true)
                } else {
                    Ok(false)
                }
            })
            .and_then(|ready| {
                if ready {
                    hook.set_enabled(true)
                } else {
                    Ok(false)
                }
            }) {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            error => {
                installation_error = Some(format!("hook installation failed: {error:?}"));
                break;
            }
        }
    }
    if installation_error.is_none() {
        ready.set();
    }
    while installation_error.is_none()
        && !detach.signaled()
        && !spool.is_finished()
        && !decode::worker_finished()
        && !shared.acquisition_failed()
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    // A callback may be waiting for a reusable frame. Cancel that wait before
    // restoring the hook and waiting for ACTIVE to reach zero; recording stays
    // enabled until every callback already in the relay has returned.
    shared.cancel_buffer_wait.store(true, Ordering::Release);
    let decode_stopped_early = installation_error.is_none() && decode::worker_finished();
    // Never unwind through an enabled or partially published detour.
    loop {
        match hook.set_enabled(false) {
            Ok(true) => break,
            result => {
                let _ = fs::write(
                    dll_path.with_file_name("collector.error.txt"),
                    format!("detach pending: {result:?}"),
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
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
    ranges.extend(opinion_hooks.executable_ranges());
    while spool_id.load(Ordering::Acquire) == 0 {
        std::thread::yield_now();
    }
    let acquisition = shared.acquisition.get().expect("initialized before hook");
    let mut owned_workers = acquisition.thread_ids();
    owned_workers.extend([spool_id.load(Ordering::Acquire), decode::worker_id()]);
    wait_quiet(&ranges, image_size, &owned_workers, &dll_path);
    // Keep mutation observation intact until the last acquisition has finished.
    // Then restore its hooks and drain callbacks that entered during restoration.
    loop {
        match portrait_hooks.set_enabled(false) {
            Ok(true) => break,
            result => {
                let _ = fs::write(
                    dll_path.with_file_name("collector.error.txt"),
                    format!("portrait hook detach pending: {result:?}"),
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    loop {
        match opinion_hooks.set_enabled(false) {
            Ok(true) => break,
            result => {
                let _ = fs::write(
                    dll_path.with_file_name("collector.error.txt"),
                    format!("opinion hook detach pending: {result:?}"),
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    loop {
        match advisor_hooks.set_enabled(false) {
            Ok(true) => break,
            result => {
                let _ = fs::write(
                    dll_path.with_file_name("collector.error.txt"),
                    format!("advisor hook detach pending: {result:?}"),
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    wait_quiet(&ranges, image_size, &owned_workers, &dll_path);
    // Keep sampling until even callbacks already in the relay have returned.
    // A delayed restore therefore never silently stops recording hours.
    shared.recording.store(false, Ordering::Release);
    // ACTIVE and relay quiescence prove every acquisition lease is complete.
    // Join the precreated readers before releasing any shared/frame storage.
    let acquisition_outcome = acquisition.shutdown();
    // Decode drain first: pending refreshes decode in FIFO hour order into
    // their pooled frames and hand off to the spool ready queue; the existing
    // spool drain below then consumes them in order. Undrainable items use the
    // existing LossBatch gap path with no new reason strings.
    let decode_outcome = decode::detach_drain();
    // Callback writers are now quiescent. The spool thread can take the final loss batch.
    shared.stop_spool.store(true, Ordering::Release);
    let spool_outcome = spool
        .join()
        .map_err(|_| "spool thread panicked".to_string())
        .and_then(|result| result);
    let mut spool_warning = spool_outcome.as_ref().ok().cloned().flatten();
    // The spool thread has returned and every callback is quiescent: waiting
    // on status I/O here blocks no game thread. The last spool status is on
    // disk (or its failure is known) before it is read below.
    status_writer.finish();
    if let Some(error) = status_writer.take_error() {
        // The loader still publishes the authoritative detached status. Keep
        // the diagnostic so successful terminal publication reports recovery.
        let error = format!("cannot publish spool status: {error}");
        spool_warning = Some(match spool_warning {
            Some(warning) => format!("{warning}; {error}"),
            None => error,
        });
    }
    let mut spool_result = spool_outcome.map(|_| ());
    if let Err(error) = acquisition_outcome {
        add_terminal_error(&mut spool_result, error);
    }
    if shared.acquisition_failed() {
        add_terminal_error(
            &mut spool_result,
            "game-thread acquisition panicked; recording stopped after draining completed captures"
                .into(),
        );
    }
    if let Err(error) = decode_outcome {
        add_terminal_error(&mut spool_result, error);
    } else if decode_stopped_early {
        add_terminal_error(
            &mut spool_result,
            "decode worker stopped before detach".into(),
        );
    }
    wait_quiet(&ranges, image_size, &[], &dll_path);
    SHARED.store(ptr::null_mut(), Ordering::Release);
    drop(hook);
    drop(portrait_hooks);
    drop(advisor_hooks);
    drop(opinion_hooks);
    drop(shared);
    if let Some(error) = installation_error {
        add_terminal_error(&mut spool_result, error);
    }
    let detach_warning = fs::read_to_string(dll_path.with_file_name("collector.error.txt"))
        .ok()
        .filter(|message| !message.trim().is_empty());
    if spool_result.is_ok() {
        // Restore retries may have left a transient diagnostic. After successful
        // drain it must not masquerade as a fatal error after module absence.
        match fs::remove_file(dll_path.with_file_name("collector.error.txt")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => add_terminal_error(
                &mut spool_result,
                format!("cannot clear recovered detach diagnostic: {error}"),
            ),
        }
    }
    let mut final_status = fs::read(&status_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| json!({}));
    set_drain_status(&mut final_status, &spool_result);
    if let Some(warning) = spool_warning {
        final_status["warning"] = json!(warning);
    }
    if let Some(warning) = detach_warning {
        let previous = final_status["warning"].as_str().unwrap_or_default();
        final_status["warning"] = json!(if previous.is_empty() {
            warning
        } else {
            format!("{previous}; {warning}")
        });
    }
    final_status["state"] = json!("detached");
    final_status["session"] = json!(session_id);
    final_status["original_bytes_verified"] = json!(true);
    final_status["active_callbacks"] = json!(0);
    if let Err(error) = write_status(&status_path, &final_status) {
        // The worker writes this terminal failure to collector.error.txt before
        // unloading. The recorder reads it only after confirming module absence.
        add_terminal_error(
            &mut spool_result,
            format!("cannot publish terminal collector status: {error}"),
        );
    }
    done.set();
    spool_result
}

fn reserve_output(config: &Config, detach: &Event) -> Result<File, String> {
    let path = config.replay_dir.join("finalize.lock");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if config.cancel_file.exists() || detach.signaled() {
            return Err("recording cancelled while waiting for campaign output".into());
        }
        match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) if error.raw_os_error() == Some(32) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(format!(
                    "campaign output is busy or unavailable: {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

fn add_terminal_error(result: &mut Result<(), String>, error: String) {
    match result {
        Ok(()) => *result = Err(error),
        Err(previous) => {
            previous.push_str("; ");
            previous.push_str(&error);
        }
    }
}

fn set_drain_status(status: &mut Value, result: &Result<(), String>) {
    if !status.is_object() {
        *status = json!({});
    }
    let previous_error = status.as_object_mut().unwrap().remove("error");
    if result.is_ok()
        && status.get("warning").is_none()
        && let Some(previous_error) = previous_error
    {
        status["warning"] = previous_error;
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
fn add_decode_status(status: &mut Value) {
    if !status.is_object() {
        *status = json!({});
    }
    status["decode_work"] = decode::calibration_json();
    status["portrait_changes"] = crate::portrait_changes::status();
    status["advisor_changes"] = crate::advisor_changes::status();
    status["opinion_changes"] = crate::opinion_changes::status();
    #[cfg(feature = "callback_profile")]
    {
        status["callback_profile"] = crate::callback_profile::status();
    }
    #[cfg(feature = "advisor_profile")]
    {
        status["advisor_profile"] = crate::extended::characters::advisors::profile::status();
    }
    status["replay_format"] = json!(replay::FORMAT_VERSION);
    status["sample_boundary"] = json!(sampling::SAMPLE_BOUNDARY.name());
    let shared_ptr = SHARED.load(Ordering::Acquire);
    if !shared_ptr.is_null() {
        // Status is produced while runtime retains Shared and joins its spool.
        let shared = unsafe { &*shared_ptr };
        status["fixed_layout"] = crate::fixed_layout::status(shared.fixed_layout.get());
        status["native_tick_work"] = json!({
            "elapsed_ns": shared.native_tick_ns.load(Ordering::Relaxed),
            "calls": shared.native_tick_calls.load(Ordering::Relaxed),
            "scope": "asynchronously sampled cumulative original hourly-tick (trampoline) wall time since attach, including engine waits; excludes the entry read, the collect wait and all acquisition; not a recorder-free baseline or CPU time",
        });
        let models = shared
            .template_models
            .lock()
            .map(|stats| *stats)
            .unwrap_or_default();
        status["template_models"] = json!({
            "hours": models.hours,
            "rows": models.rows,
            "first_reason": models.first_reason,
            "by_reason": models
                .reasons
                .iter()
                .filter(|(_, rows)| *rows != 0)
                .map(|(reason, rows)| (reason.to_string(), json!(rows)))
                .collect::<serde_json::Map<_, _>>(),
            "other_reasons": models.other,
            "missed_hours": shared.template_models_missed.load(Ordering::Relaxed),
            "scope": "cumulative DivisionTemplateIcons rows since attach whose 3D-model inputs could not be read: that row's model fields are empty and its counter fields stand; the viewer draws no model for its divisions. missed_hours counts hours not folded because this status write held the counters",
        });
        let eta = &shared.eta_inputs;
        let load = |slot: &std::sync::atomic::AtomicU64| slot.load(Ordering::Relaxed);
        let eta_hours = load(&eta.hours);
        let eta_mean = |value: u64| value.checked_div(eta_hours).unwrap_or(0);
        status["background_priority"] = background_priority_status();
        status["refresh_requests_accepted"] = json!(REFRESH_ACCEPTED.load(Ordering::Relaxed));
        status["eta_inputs"] = json!({
            "hours": eta_hours,
            "walks": load(&eta.walks),
            "divisions": load(&eta.divisions),
            "legs": load(&eta.legs),
            "graph_ns": load(&eta.graph_ns),
            "scan_ns": load(&eta.scan_ns),
            "other_ns": load(&eta.other_ns),
            "graph_mean_ns": eta_mean(load(&eta.graph_ns)),
            "scan_mean_ns": eta_mean(load(&eta.scan_ns)),
            "other_mean_ns": eta_mean(load(&eta.other_ns)),
            "divisions_mean": eta_mean(load(&eta.divisions)),
            "legs_mean": eta_mean(load(&eta.legs)),
            "geometry_hits": load(&eta.geometry_hits),
            "geometry_misses": load(&eta.geometry_misses),
            "static_hits": load(&eta.static_hits),
            "static_misses": load(&eta.static_misses),
            "static_cleared": load(&eta.static_cleared),
            "state_modifier_requests": load(&eta.state_modifier_requests),
            "state_modifier_hits": load(&eta.state_modifier_hits),
            "air_defense_requests": load(&eta.air_defense_requests),
            "air_defense_hits": load(&eta.air_defense_hits),
            "scope": "cumulative DivisionEtaInputs acquisition since attach, one observation per sampled hour. graph = the per-division composed speed graph, scan = the per-leg route scan, other = the route copy and the prepared speed tail; they sum to the acquisition wall inside its owning worker, not to game-thread block. geometry hits/misses count province/adjacency lookups answered from the pass memo versus read from memory; static hits/misses are the session map memo (distance, crossing, flags, terrain), and static_cleared counts hours that dropped it after a reload or a new census",
        });
        if let Some(acquisition) = shared.acquisition.get() {
            let world = acquisition.world_calibration();
            let calibration = acquisition.calibration();
            let block_ns = shared.collect_wait_ns.load(Ordering::Relaxed);
            let block_calls = shared.native_tick_calls.load(Ordering::Relaxed);
            let phase_calls = shared.phase_calls.load(Ordering::Relaxed);
            let phase_mean = |ns: u64| ns.checked_div(phase_calls).unwrap_or(0);
            let join_wait_by_role = crate::acquisition_jobs::ROLES
                .iter()
                .zip(&shared.join_wait)
                .filter_map(|(role, wait)| {
                    let wait = wait.load(Ordering::Relaxed);
                    (wait > 0).then(|| (format!("{role:?}"), json!(phase_mean(wait))))
                })
                .collect::<serde_json::Map<_, _>>();
            status["acquisition_work"] = json!({
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
                    "segments_mean_ns": shared.phase_segments.iter().map(|segment| phase_mean(segment.load(Ordering::Relaxed))).collect::<Vec<_>>(),
                    "segments_scope": sampling::SEGMENTS_SCOPE,
                    "join_wait_by_role_mean_ns": join_wait_by_role,
                    "residual_mean_ns": phase_mean(shared.phase_residual_ns.load(Ordering::Relaxed)),
                },
                "block_phases_scope": "disjoint game-thread wall means summing to the recorded acquisition body: setup = reusable-frame wait, frame reset and country authentication; parallel = shared input acquisition, request dispatch, result installation and sequential job joins; residual = completion bookkeeping before decode enqueue. parallel_inline_walk is a subset of parallel outside joins, measured over the collection span; parallel_join counts sequential completion spans and may include execution of the exact awaited job. Their sum leaves dispatch and other work outside that span. Worker durations overlap this game-thread wall and must not be added. Reusable-frame wait is separately reported but already included in setup, collect_wait_ns and callback_work",
                "panics": shared.acquisition_panics.load(Ordering::Relaxed),
                "workers": calibration,
                "dispatch_ns": shared.acquisition_dispatch_ns.load(Ordering::Relaxed),
                "join_wait_ns": shared.acquisition_wait_ns.load(Ordering::Relaxed),
                "join_scope": "sequential game-thread completion spans including bookkeeping and possible exact-target caller execution when the awaited request is still queued; no unrelated queued job is executed by a join",
                "table_costs_overlap": true,
                "country_snapshot_scope": "one immutable country authentication snapshot shared with acquisition workers; production-rest acquires fresh country inputs for its discovery builder and validates its current read plan; payload counted once in pool_bytes; preparation charged once to Countries; worker calibration reports the borrowed snapshot status",
                "world_country_cache": world.country_cache,
                "world_acquisition_ns": shared.ns(i64::try_from(world.acquisition_ticks).unwrap_or(i64::MAX)),
                "world_last_error": world.last_error,
                "world_cost_scope": "hourly owned World job on any acquisition participant; includes staging attempts; shared country preparation charged once to Countries; typed fallback table costs exclude discarded staging attempts; overlaps other jobs and acquisition work; panicked/stopped worker cost is unavailable",
            });
        }
    }
}

fn wait_quiet(
    ranges: &[std::ops::Range<usize>],
    image_size: usize,
    excluded_threads: &[u32],
    dll_path: &Path,
) {
    let mut report = Instant::now();
    loop {
        let outcome = if image_size == 0 {
            Err("collector image size unreadable".to_string())
        } else if ACTIVE.load(Ordering::SeqCst) != 0
            || crate::portrait_changes::ACTIVE.load(Ordering::SeqCst) != 0
            || crate::advisor_changes::ACTIVE.load(Ordering::SeqCst) != 0
            || crate::opinion_changes::ACTIVE.load(Ordering::SeqCst) != 0
        {
            Ok(false)
        } else {
            Threads::open_excluding(excluded_threads).and_then(|mut threads| {
                let quiet = threads.suspend().and_then(|()| {
                    if ACTIVE.load(Ordering::SeqCst) != 0
                        || crate::portrait_changes::ACTIVE.load(Ordering::SeqCst) != 0
                        || crate::advisor_changes::ACTIVE.load(Ordering::SeqCst) != 0
                        || crate::opinion_changes::ACTIVE.load(Ordering::SeqCst) != 0
                    {
                        Ok(false)
                    } else {
                        threads.contains_ip(ranges).map(|present| !present)
                    }
                });
                let resumed = threads.resume();
                drop(threads);
                resumed.map_err(|code| format!("ResumeThread failed: {code}"))?;
                quiet.map_err(|code| format!("thread context check failed: {code}"))
            })
        };
        if outcome == Ok(true) {
            break;
        }
        if report.elapsed() >= Duration::from_secs(1) {
            let _ = fs::write(
                dll_path.with_file_name("collector.error.txt"),
                format!(
                    "unload pending: {outcome:?}; active {}",
                    ACTIVE.load(Ordering::SeqCst)
                ),
            );
            report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Throttled observation of the live country registry for buffer sizing.
/// Header words only; nothing observed here is recorded.
#[derive(Default)]
struct RegistryWatch {
    last: Option<Instant>,
    slots: usize,
    last_intel: Option<Instant>,
    intel_rows: usize,
    last_raids: Option<Instant>,
    raids: crate::country_scale::RaidArenas,
    last_characters: Option<Instant>,
    characters: crate::country_scale::CharacterDemand,
    last_politics: Option<Instant>,
    politics: crate::country_scale::PoliticsDemand,
}

impl RegistryWatch {
    const INTERVAL: Duration = Duration::from_millis(50);
    /// The intel total walks every country's table header, so it is sampled
    /// less often than the three registry headers.
    const INTEL_INTERVAL: Duration = Duration::from_secs(1);

    /// A larger registry than any seen before, at most every `INTERVAL`.
    fn poll(&mut self, memory: &impl engine::Memory, base: usize) -> Option<usize> {
        if self
            .last
            .is_some_and(|last| last.elapsed() < Self::INTERVAL)
        {
            return None;
        }
        self.last = Some(Instant::now());
        let slots = crate::country_scale::observe(memory, base)?;
        (slots > self.slots).then(|| {
            self.slots = slots;
            slots
        })
    }

    /// A larger dense CountryIntel total than any seen, at most every
    /// `INTEL_INTERVAL`. Intel vectors fill after a save's registry does.
    fn poll_intel(&mut self, memory: &impl engine::Memory, base: usize) -> Option<usize> {
        if self
            .last_intel
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_intel = Some(Instant::now());
        let rows = crate::country_scale::observe_country_intel(memory, base)?;
        (rows > self.intel_rows).then(|| {
            self.intel_rows = rows;
            rows
        })
    }

    /// Larger raid arena demand than any seen, at most every
    /// `INTEL_INTERVAL`: raid parents' type lists and convoy reservations
    /// fill after a save's registry does, and a full hour must read them all.
    fn poll_raids(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Option<crate::country_scale::RaidArenas> {
        if self
            .last_raids
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_raids = Some(Instant::now());
        let demand = crate::country_scale::observe_raid_arenas(memory, base)?;
        (demand.type_tokens > self.raids.type_tokens
            || demand.reservation_words > self.raids.reservation_words)
            .then(|| {
                self.raids.type_tokens = self.raids.type_tokens.max(demand.type_tokens);
                self.raids.reservation_words =
                    self.raids.reservation_words.max(demand.reservation_words);
                self.raids
            })
    }

    /// Larger character or script flag demand than any seen, at most every
    /// `INTEL_INTERVAL`: a save's roster, template files and flags fill after
    /// its registry does, and a full hour must read them all.
    fn poll_characters(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Option<crate::country_scale::CharacterDemand> {
        if self
            .last_characters
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_characters = Some(Instant::now());
        let demand = crate::country_scale::observe_characters(memory, base)?;
        let seen = &mut self.characters;
        (demand.characters.rows > seen.characters.rows
            || demand.characters.bytes > seen.characters.bytes
            || demand.flags > seen.flags)
            .then(|| {
                seen.characters.rows = seen.characters.rows.max(demand.characters.rows);
                seen.characters.bytes = seen.characters.bytes.max(demand.characters.bytes);
                seen.flags = seen.flags.max(demand.flags);
                *seen
            })
    }

    /// Larger political table demand than any seen, at most every
    /// `INTEL_INTERVAL`: a save's character records, parties, civil wars and
    /// advisor lists fill after its registry does, and a full hour must read
    /// them all.
    fn poll_politics(
        &mut self,
        memory: &impl engine::Memory,
        base: usize,
    ) -> Option<crate::country_scale::PoliticsDemand> {
        if self
            .last_politics
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_politics = Some(Instant::now());
        let demand = crate::country_scale::observe_politics(memory, base)?;
        demand.exceeds(&self.politics).then(|| {
            self.politics = self.politics.max(demand);
            self.politics
        })
    }
}

/// Grow every idle frame to the current capacities under its queue lease.
fn grow_idle_frames(
    shared: &Shared,
    capacities: &mut [usize; 11],
    map_capacities: &mut crate::map_capture::MapCapacities,
    extended_capacities: &mut crate::extended::Capacities,
) {
    for _ in 0..shared.free.len() {
        let Some(idle_index) = shared.free.pop() else {
            break;
        };
        let idle = unsafe { &mut *shared.frames[idle_index].load(Ordering::Acquire) };
        let old_bytes = idle.allocated_bytes() + shared.country_intel.allocated_bytes();
        idle.grow(capacities);
        idle.map.grow(map_capacities);
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
        shared.recycle_frame(idle_index);
    }
}

fn spool_loop(
    shared: &Shared,
    config: &Config,
    mut session: SessionStart,
    mut processor: Processor,
    status_path: &Path,
    sizing: crate::country_scale::Sizing,
    // Status and content files are written on the loader's writer thread:
    // this thread only hands snapshots over and never waits on status I/O,
    // nor joins that thread (#94).
    writer: crate::status_writer::StatusHandle,
) -> Result<Option<String>, String> {
    let mut capacities = sizing.core;
    let mut map_capacities = sizing.map;
    let mut extended_capacities = sizing.extended;
    let live = unsafe { LiveMemory::new() };
    let mut registry = RegistryWatch::default();
    let mut last_warning = None;
    let mut drain_result = Ok(());
    let mut startup_unknown = 0_u64;
    let mut work = WorkerWork::new();
    let mut reported = Instant::now() - Duration::from_secs(1);
    let mut priority = OwnPriority::new();
    let mut refresh_requests = RefreshRequests::default();
    loop {
        priority.follow(shared.buffers_backlogged());
        while let Some(index) = shared.ready.pop() {
            priority.follow(shared.buffers_backlogged());
            let service_started = sampling::qpc();
            work.queue_peak_observed = work.queue_peak_observed.max(shared.ready.len() + 1);
            let mut elapsed = sampling::completed_callback_ticks(
                &shared.completed[index],
                &shared.callback_tail[index],
            );
            while elapsed.is_none() {
                std::thread::yield_now();
                elapsed = sampling::completed_callback_ticks(
                    &shared.completed[index],
                    &shared.callback_tail[index],
                );
            }
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.total_ns = shared.ns(elapsed.unwrap());
            // The shared CountryIntel buffer grows through whichever frame
            // returns it, so its bytes are part of every growth delta.
            let before_bytes = frame.allocated_bytes() + shared.country_intel.allocated_bytes();
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
                shared.recycle_frame(index);
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
                let _ = roster.verify(
                    &mut session.identity,
                    &config.expected_mounts,
                    config.content_order_note.as_deref(),
                );
                let diagnostic =
                    roster.diagnostics(&config.expected_mounts, &session.identity.verification);
                writer.publish(&config.output_dir.join("content.json"), diagnostic);
                processor.set_verified_session(session.clone());
            }
            // Campaign directories can still fail to open. Keep the buffer and
            // preceding witnesses until those operations succeed. History is ready.
            while let Err(error) = processor.prepare(frame) {
                if error.kind() == io::ErrorKind::InvalidData {
                    // An immutable history or identity refusal cannot recover by
                    // retaining this buffer. Ending the worker requests restore
                    // through the loader's JoinHandle check and preserves the
                    // precise error in its terminal status.
                    let drain = processor.detach().err();
                    return Err(format!(
                        "campaign recording refused: {error}; current and {} queued observations remain unwritten{}",
                        shared.ready.len(),
                        drain.map_or_else(String::new, |error| format!(
                            "; prior spool detach: {error}"
                        )),
                    ));
                }
                last_warning = Some(error.to_string());
                writer.publish(
                    status_path,
                    json!({
                        "state":"campaign spool unavailable", "session":session.id,
                        "warning":last_warning, "drain_complete":false,
                        "dropped_frames":shared.dropped.load(Ordering::Relaxed),
                    }),
                );
                if shared.stop_spool.load(Ordering::Acquire) || shared.free.is_empty() {
                    // A persistent open failure cannot release a frame. Once
                    // all frames are occupied, end the worker so the loader
                    // cancels the callback wait and restores the hook.
                    let _ = processor.detach();
                    return Err(format!(
                        "cannot open campaign spool at detach or exhausted frame pool; retained observations remain unwritten: {error}"
                    ));
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
                    last_warning = Some(error.to_string());
                }
            }
            // Conversion owns every value needed by encoding and diagnostics.
            // Return the capture storage before potentially slow output, except
            // when a retry must be published before that storage can be reused.
            let pending_write = match processor.prepare_frame(frame) {
                Ok(prepared) if prepared.allows_early_recycle() => Some(prepared),
                Ok(prepared) => {
                    if let Err(error) = processor.write_prepared(prepared) {
                        last_warning = Some(error.to_string());
                    }
                    None
                }
                Err(error) => {
                    last_warning = Some(error.to_string());
                    None
                }
            };
            if processor.take_retry_full() {
                shared.force_next.store(true, Ordering::Release);
            }
            let unpublished = processor.take_unpublished_families();
            if unpublished != 0 {
                shared
                    .forget_families
                    .fetch_or(unpublished, Ordering::Release);
            }
            let previous_capacities = capacities;
            let previous_map_capacities = map_capacities;
            let previous_extended_capacities = extended_capacities;
            frame.grow(&mut capacities);
            frame.map.grow(&mut map_capacities);
            frame.extended.grow(&mut extended_capacities);
            let after_bytes = frame.allocated_bytes() + shared.country_intel.allocated_bytes();
            if after_bytes >= before_bytes {
                shared
                    .pool_bytes
                    .fetch_add((after_bytes - before_bytes) as u64, Ordering::Relaxed);
            } else {
                shared
                    .pool_bytes
                    .fetch_sub((before_bytes - after_bytes) as u64, Ordering::Relaxed);
            }
            if previous_capacities != capacities
                || previous_map_capacities != map_capacities
                || previous_extended_capacities != extended_capacities
            {
                // Grow every idle buffer under its queue-index lease so old
                // capacities do not repeat the same truncation on later hours.
                grow_idle_frames(
                    shared,
                    &mut capacities,
                    &mut map_capacities,
                    &mut extended_capacities,
                );
            }
            if let SeedStatus::Capacity { required_cells } = frame.country_cache_status {
                shared.offer_country_cache(required_cells);
            }
            shared.recycle_frame(index);
            work.buffer_service
                .record_ticks(sampling::qpc() - service_started, shared.frequency);
            // No reference to the recycled frame survives this point. Complete
            // this write before preparing another frame or flushing: comparison
            // tokens belong to the current encoder baseline.
            if let Some(prepared) = pending_write
                && let Err(error) = processor.write_prepared(prepared)
            {
                last_warning = Some(error.to_string());
            }
        }
        // A save load rebuilds the country registry before its first hourly
        // tick; presize every buffer while the game is still loading.
        if let Some(slots) = registry.poll(&live, shared.base) {
            crate::country_scale::note_slots(slots);
            decode::presize_country_pools();
            let mut raised = crate::country_scale::Sizing {
                core: capacities,
                map: map_capacities,
                extended: extended_capacities,
                country_cells: 0,
                content: engine::cdlc_manager::Counts::default(),
            };
            raised.raise_for_slots(slots);
            shared.offer_country_cache(raised.country_cells);
            if raised.core != capacities
                || raised.map != map_capacities
                || raised.extended != extended_capacities
            {
                capacities = raised.core;
                map_capacities = raised.map;
                extended_capacities = raised.extended;
                grow_idle_frames(
                    shared,
                    &mut capacities,
                    &mut map_capacities,
                    &mut extended_capacities,
                );
            }
        }
        if let Some(rows) = registry.poll_intel(&live, shared.base) {
            let before = extended_capacities;
            extended_capacities.raise_for_country_intel(rows);
            if extended_capacities != before {
                grow_idle_frames(
                    shared,
                    &mut capacities,
                    &mut map_capacities,
                    &mut extended_capacities,
                );
            }
        }
        if let Some(demand) = registry.poll_raids(&live, shared.base) {
            let before = extended_capacities;
            extended_capacities.raise_for_raid_arenas(demand);
            if extended_capacities != before {
                grow_idle_frames(
                    shared,
                    &mut capacities,
                    &mut map_capacities,
                    &mut extended_capacities,
                );
            }
        }
        if let Some(demand) = registry.poll_characters(&live, shared.base) {
            let before = extended_capacities;
            extended_capacities.raise_for_characters(demand);
            if extended_capacities != before {
                grow_idle_frames(
                    shared,
                    &mut capacities,
                    &mut map_capacities,
                    &mut extended_capacities,
                );
            }
        }
        if let Some(demand) = registry.poll_politics(&live, shared.base) {
            let before = extended_capacities;
            extended_capacities.raise_for_politics(demand);
            if extended_capacities != before {
                grow_idle_frames(
                    shared,
                    &mut capacities,
                    &mut map_capacities,
                    &mut extended_capacities,
                );
            }
        }
        let flush_started = sampling::qpc();
        if let Err(error) = processor.flush() {
            last_warning = Some(error.to_string());
        }
        let flush_ns = shared.ns(sampling::qpc() - flush_started);
        work.idle_flush_calls += 1;
        work.idle_flush_ns = work.idle_flush_ns.saturating_add(flush_ns);
        work.idle_flush_max_ns = work.idle_flush_max_ns.max(flush_ns);
        if reported.elapsed() >= Duration::from_secs(1) || shared.stop_spool.load(Ordering::Acquire)
        {
            refresh_requests.poll(&config.output_dir, shared);
            let status_started = sampling::qpc();
            if let Some(error) = writer.take_error() {
                last_warning = Some(error);
            }
            let mut status = processor.status();
            status["state"] = json!(if shared.recording.load(Ordering::Acquire) {
                "recording"
            } else {
                "detaching"
            });
            status["session"] = json!(session.id);
            status["pool_bytes"] = json!(shared.pool_bytes.load(Ordering::Relaxed));
            status["pool_buffers"] = json!(sampling::BUFFERS);
            status["dropped_frames"] = json!(shared.dropped.load(Ordering::Relaxed));
            status["startup_unattributed_callbacks"] = json!(startup_unknown);
            status["drain_complete"] = json!(false);
            if let Some(error) = &last_warning {
                status["warning"] = json!(error);
            }
            add_decode_status(&mut status);
            work.add_status(&mut status, shared.frequency);
            status["status_writer"] = writer.json();
            writer.publish_measured(status_path, status, processor.spool_directories());
            work.status_update
                .record_ticks(sampling::qpc() - status_started, shared.frequency);
            reported = Instant::now();
        }
        if shared.stop_spool.load(Ordering::Acquire) {
            // The last callback may have published after this iteration's initial
            // empty pop but before the loader published quiescence. Drain it first.
            if !shared.ready.is_empty() {
                continue;
            }
            // The loader worker has restored the hook and proved every callback quiescent.
            let pending = unsafe { std::mem::take(&mut *shared.pending.get()) };
            match processor.prepare_losses(&pending) {
                Err(error) => {
                    add_terminal_error(
                        &mut drain_result,
                        format!(
                            "cannot open campaign spool for retained loss witnesses at detach: {error}"
                        ),
                    );
                }
                Ok(()) => {
                    if pending.unknown_count != 0 {
                        processor.unknown_loss(pending.unknown_count);
                    }
                    for loss in pending.iter() {
                        if let Err(error) = processor.lost(loss) {
                            add_terminal_error(
                                &mut drain_result,
                                format!("cannot drain retained loss witness: {error}"),
                            );
                        }
                    }
                }
            }
            if let Err(error) = processor.detach() {
                add_terminal_error(
                    &mut drain_result,
                    format!("cannot finish spool detach: {error}"),
                );
            }
            let status_started = sampling::qpc();
            let mut status = processor.status();
            status["state"] = json!("detaching");
            status["session"] = json!(session.id);
            status["pool_bytes"] = json!(shared.pool_bytes.load(Ordering::Relaxed));
            status["pool_buffers"] = json!(sampling::BUFFERS);
            status["dropped_frames"] = json!(shared.dropped.load(Ordering::Relaxed));
            status["drain_complete"] = json!(false);
            if let Some(error) = &last_warning {
                status["warning"] = json!(error);
            }
            if let Err(error) = &drain_result {
                status["error"] = json!(error);
            }
            add_decode_status(&mut status);
            work.add_status(&mut status, shared.frequency);
            status["status_writer"] = writer.json();
            writer.publish_measured(status_path, status, processor.spool_directories());
            work.status_update
                .record_ticks(sampling::qpc() - status_started, shared.frequency);
            // The loader finishes the writer after joining this thread and
            // reads this status then; a failed write becomes its warning.
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if startup_unknown != 0 {
        add_terminal_error(
            &mut drain_result,
            format!(
                "{startup_unknown} startup callbacks had no readable clock; no content roster could be sampled"
            ),
        );
    }
    drain_result.map(|()| last_warning)
}

/// `recorder refresh` asks for one full-refresh hour: every scheduled table,
/// including daily, static and sliced ones, is read at the next hourly sample.
/// The request is a file in the session directory; the spool thread turns it
/// into the existing `force_next` and acknowledges it. Nothing is sent to the
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
    fn poll(&mut self, session: &Path, shared: &Shared) {
        let request = session.join(REFRESH_REQUEST);
        let Ok(token) = fs::read_to_string(&request) else {
            return;
        };
        // The requester writes a temporary file and renames it, so a token is
        // whole when present; an empty one is not a request.
        let token = token.trim();
        if token.is_empty() || self.last.as_deref() == Some(token) {
            return;
        }
        shared.force_next.store(true, Ordering::Release);
        REFRESH_ACCEPTED.fetch_add(1, Ordering::Relaxed);
        self.last = Some(token.to_owned());
        // Acknowledge before removing the request: the requester waits for
        // its token here.
        if fs::write(session.join(REFRESH_ACK), token).is_ok() {
            let _ = fs::remove_file(&request);
        }
    }
}

static REFRESH_ACCEPTED: AtomicU64 = AtomicU64::new(0);

static PRIORITY_LOWERED: AtomicU64 = AtomicU64::new(0);
static PRIORITY_RAISED: AtomicU64 = AtomicU64::new(0);
static PRIORITY_FAILURES: AtomicU64 = AtomicU64::new(0);

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
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
            THREAD_PRIORITY_NORMAL,
        };
        let below_normal = !backlogged;
        if self.below_normal == Some(below_normal) {
            return;
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

fn background_priority_status() -> serde_json::Value {
    json!({
        "lowered": PRIORITY_LOWERED.load(Ordering::Relaxed),
        "raised_for_backlog": PRIORITY_RAISED.load(Ordering::Relaxed),
        "failures": PRIORITY_FAILURES.load(Ordering::Relaxed),
        "scope": "cumulative priority changes of the decode and spool threads: below normal while idle or keeping up, normal while every reusable capture buffer but at most one is in flight (the game thread may then be waiting for them); failures keep the previous priority",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refresh_request_sets_force_next_once_and_is_acknowledged() {
        let session = std::env::temp_dir().join(format!("hoi4-refresh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session);
        fs::create_dir_all(&session).unwrap();
        let shared = sampling::Shared::new(0, 0, 1);
        let mut requests = RefreshRequests::default();
        // No request: nothing forced.
        requests.poll(&session, &shared);
        assert!(!shared.force_next.load(Ordering::Acquire));
        fs::write(session.join(REFRESH_REQUEST), "token-7\n").unwrap();
        requests.poll(&session, &shared);
        assert!(shared.force_next.swap(false, Ordering::AcqRel));
        assert_eq!(
            fs::read_to_string(session.join(REFRESH_ACK)).unwrap(),
            "token-7"
        );
        assert!(!session.join(REFRESH_REQUEST).exists());
        // Consumed: a later poll forces nothing more.
        requests.poll(&session, &shared);
        assert!(!shared.force_next.load(Ordering::Acquire));
        // A request that could not be removed is accepted once, not every hour.
        fs::write(session.join(REFRESH_REQUEST), "token-7").unwrap();
        requests.poll(&session, &shared);
        assert!(!shared.force_next.load(Ordering::Acquire));
        // An empty token is not a request; a new token is.
        fs::write(session.join(REFRESH_REQUEST), " \n").unwrap();
        requests.poll(&session, &shared);
        assert!(!shared.force_next.load(Ordering::Acquire));
        fs::write(session.join(REFRESH_REQUEST), "token-8").unwrap();
        requests.poll(&session, &shared);
        assert!(shared.force_next.swap(false, Ordering::AcqRel));
        assert_eq!(requests.last.as_deref(), Some("token-8"));
        let _ = fs::remove_dir_all(&session);
    }

    /// A registry whose three headers report `count` elements.
    struct Registry(std::cell::Cell<u32>);

    impl engine::Memory for Registry {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            const STATE: usize = 0x1000;
            let mut bytes = [0_u8; 0x400];
            for field in [
                cgame_state::COUNTRIES,
                cgame_state::COUNTRY_LINKS,
                cgame_state::COUNTRY_TAGS,
            ] {
                bytes[field..field + 8].copy_from_slice(&0x9000_u64.to_le_bytes());
                bytes[field + 8..field + 12].copy_from_slice(&self.0.get().to_le_bytes());
                bytes[field + 12..field + 16].copy_from_slice(&self.0.get().to_le_bytes());
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
    fn registry_watch_reports_growth_once_and_ignores_torn_or_smaller_counts() {
        let memory = Registry(std::cell::Cell::new(0));
        let mut watch = RegistryWatch::default();
        // The main menu has an empty registry: nothing to presize.
        assert_eq!(watch.poll(&memory, 0), None);
        // A save loads 5,001 slots; the next poll after the interval sees it.
        memory.0.set(5_001);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), Some(5_001));
        // Throttled, then unchanged.
        assert_eq!(watch.poll(&memory, 0), None);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
        // A smaller registry never shrinks capacities.
        memory.0.set(400);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
        // A count past the engine's u16 slot ceiling is a torn read, not a size.
        memory.0.set(1 << 20);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
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
        assert!(
            error
                .to_string()
                .contains("cannot atomically replace status")
        );
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
    fn recovered_warnings_do_not_become_terminal_drain_failures() {
        let mut status = json!({"error":"old transient error", "warning":"recovered write failure", "last_error":"recovered write failure", "write_errors":1});
        set_drain_status(&mut status, &Ok(()));
        assert_eq!(status["drain_complete"], true);
        assert!(status.get("error").is_none());
        assert_eq!(status["warning"], "recovered write failure");
        assert_eq!(status["write_errors"], 1);
    }

    #[test]
    fn terminal_failures_remain_explicit_even_with_recovered_history() {
        let mut result = Ok(());
        add_terminal_error(&mut result, "final sync failed".into());
        add_terminal_error(&mut result, "terminal status unavailable".into());
        let mut status = json!({"warning":"earlier failure recovered"});
        set_drain_status(&mut status, &result);
        assert_eq!(status["drain_complete"], false);
        assert_eq!(
            status["error"],
            "final sync failed; terminal status unavailable"
        );
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
