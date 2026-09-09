//! Loader worker, spool thread, and restore/drain/self-unload lifecycle.

use crate::{
    hook::{Hook, Threads},
    sampling::{self, ACTIVE, SHARED, Shared},
    spooling::{Distribution, Processor},
};
use engine::{PINNED_BUILD, build::hash_executable, cgame_state, memory::LiveMemory, read};
use replay::{codec::Decoder, identity::SessionStart};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    ffi::{OsString, c_void},
    fs::{self, File},
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HMODULE, WAIT_OBJECT_0},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
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
    fs::write(&temp, serde_json::to_vec(status)?)?;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    if unsafe {
        MoveFileExW(
            wide(&temp).as_ptr(),
            wide(path).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct WorkerWork {
    buffer_service: Distribution,
    status_update: Distribution,
    queue_peak_observed: usize,
    started_qpc: i64,
    started_cpu_ns: Result<u64, String>,
}

impl WorkerWork {
    fn new() -> Self {
        Self {
            buffer_service: Distribution::default(),
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
    let rva = cgame_state::TICK_CANDIDATE_RVA;
    let sentinel = cgame_state::TICK_CANDIDATE_SENTINEL;
    if read::<[u8; 16]>(&memory, base + rva) != Some(sentinel) {
        return Err(format!("hourly boundary sentinel differs at RVA {rva:#x}"));
    }
    let mut hook =
        unsafe { Hook::new(base + rva, sampling::tick as *const () as usize, sentinel) }?;
    let shared = Arc::new(Shared::new(base, hook.trampoline(), frequency as u64));
    let spool_id = Arc::new(AtomicU32::new(0));
    if config.cancel_file.exists() || detach.signaled() {
        return Err("recording cancelled before hook installation".into());
    }
    let spool_shared = Arc::clone(&shared);
    let spool_thread_id = Arc::clone(&spool_id);
    let spool_status = status_path.clone();
    let session_id = session.id;
    let spool = std::thread::Builder::new()
        .name("hoi4-spool".into())
        .spawn(move || {
            spool_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            spool_loop(&spool_shared, &config, session, &spool_status)
        })
        .map_err(|e| e.to_string())?;
    SHARED.store(Arc::as_ptr(&shared).cast_mut(), Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut installation_error = None;
    loop {
        match hook.set_enabled(true) {
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
    while installation_error.is_none() && !detach.signaled() {
        std::thread::sleep(Duration::from_millis(20));
    }
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
    let ranges = [
        module as usize..module as usize + image_size,
        relay,
        trampoline,
    ];
    while spool_id.load(Ordering::Acquire) == 0 {
        std::thread::yield_now();
    }
    wait_quiet(
        &ranges,
        image_size,
        spool_id.load(Ordering::Acquire),
        &dll_path,
    );
    // Keep sampling until even callbacks already in the relay have returned.
    // A delayed restore therefore never silently stops recording hours.
    shared.recording.store(false, Ordering::Release);
    // Callback writers are now quiescent. The spool thread can take the final loss batch.
    shared.stop_spool.store(true, Ordering::Release);
    let spool_outcome = spool
        .join()
        .map_err(|_| "spool thread panicked".to_string())
        .and_then(|result| result);
    let spool_warning = spool_outcome.as_ref().ok().cloned().flatten();
    let mut spool_result = spool_outcome.map(|_| ());
    wait_quiet(&ranges, image_size, 0, &dll_path);
    SHARED.store(ptr::null_mut(), Ordering::Release);
    drop(hook);
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

fn wait_quiet(
    ranges: &[std::ops::Range<usize>],
    image_size: usize,
    excluded_thread: u32,
    dll_path: &Path,
) {
    let mut report = Instant::now();
    loop {
        let outcome = if image_size == 0 {
            Err("collector image size unreadable".to_string())
        } else if ACTIVE.load(Ordering::SeqCst) != 0 {
            Ok(false)
        } else {
            Threads::open_excluding(excluded_thread).and_then(|mut threads| {
                let quiet = threads.suspend().and_then(|()| {
                    if ACTIVE.load(Ordering::SeqCst) != 0 {
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

fn spool_loop(
    shared: &Shared,
    config: &Config,
    mut session: SessionStart,
    status_path: &Path,
) -> Result<Option<String>, String> {
    let mut processor: Option<Processor> = None;
    let mut capacities = [512, 20_000, 1024, 2048];
    let mut last_warning = None;
    let mut drain_result = Ok(());
    let mut startup_unknown = 0_u64;
    let mut work = WorkerWork::new();
    let mut reported = Instant::now() - Duration::from_secs(1);
    loop {
        while let Some(index) = shared.ready.pop() {
            let service_started = sampling::qpc();
            work.queue_peak_observed = work.queue_peak_observed.max(shared.ready.len() + 1);
            let mut elapsed = shared.completed[index].load(Ordering::Acquire);
            while elapsed == 0 {
                std::thread::yield_now();
                elapsed = shared.completed[index].load(Ordering::Acquire);
            }
            let frame = unsafe { &mut *shared.frames[index].get() };
            frame.total_ns = shared.ns(elapsed);
            let before_bytes = frame.allocated_bytes();
            if !shared.roster_ready.load(Ordering::Acquire) {
                // Any readable post-tick clock publishes the roster before its
                // buffer. These early callbacks have no attributable post-tick
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
                let _ = shared.free.push(index);
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
                if let Err(error) =
                    write_status(&config.output_dir.join("content.json"), &diagnostic)
                {
                    last_warning = Some(format!("cannot write content observation: {error}"));
                }
            }
            while processor.is_none() {
                match Processor::new(&config.output_dir, &config.replay_dir, session.clone()) {
                    Ok(created) => processor = Some(created),
                    Err(error) => {
                        last_warning = Some(error.to_string());
                        let _ = write_status(
                            status_path,
                            &json!({"state":"spool unavailable", "session":session.id, "warning":last_warning, "drain_complete":false}),
                        );
                        if shared.stop_spool.load(Ordering::Acquire) {
                            return Err(format!(
                                "cannot create spool at detach; queued observations remain unwritten: {error}"
                            ));
                        }
                        // Keep the current buffer and its preceding loss witnesses.
                        // Later callback overflows remain recorded in the fixed pool.
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            if let Some(processor) = processor.as_mut() {
                // Opening a campaign can read an existing replay or create a
                // directory. Retain this buffer and every preceding witness until
                // all those fallible operations succeed, before observing any hour.
                while let Err(error) = processor.prepare(frame) {
                    last_warning = Some(error.to_string());
                    let _ = write_status(
                        status_path,
                        &json!({
                            "state":"campaign spool unavailable", "session":session.id,
                            "warning":last_warning, "drain_complete":false,
                            "dropped_frames":shared.dropped.load(Ordering::Relaxed),
                        }),
                    );
                    if shared.stop_spool.load(Ordering::Acquire) {
                        let _ = processor.detach();
                        return Err(format!(
                            "cannot open campaign spool at detach; retained observations remain unwritten: {error}"
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
                if let Err(error) = processor.frame(frame) {
                    last_warning = Some(error.to_string());
                }
            }
            let previous_capacities = capacities;
            frame.grow(&mut capacities);
            let after_bytes = frame.allocated_bytes();
            if after_bytes >= before_bytes {
                shared
                    .pool_bytes
                    .fetch_add((after_bytes - before_bytes) as u64, Ordering::Relaxed);
            } else {
                shared
                    .pool_bytes
                    .fetch_sub((before_bytes - after_bytes) as u64, Ordering::Relaxed);
            }
            if previous_capacities != capacities {
                // Grow every idle buffer under its queue-index lease, so a FIFO of
                // old capacities does not repeat the same truncation for 256 hours.
                for _ in 0..shared.free.len() {
                    let Some(idle_index) = shared.free.pop() else {
                        break;
                    };
                    let idle = unsafe { &mut *shared.frames[idle_index].get() };
                    let old_bytes = idle.allocated_bytes();
                    idle.grow(&mut capacities);
                    shared.pool_bytes.fetch_add(
                        (idle.allocated_bytes() - old_bytes) as u64,
                        Ordering::Relaxed,
                    );
                    let _ = shared.free.push(idle_index);
                }
            }
            let _ = shared.free.push(index);
            work.buffer_service
                .record_ticks(sampling::qpc() - service_started, shared.frequency);
        }
        if let Some(processor) = processor.as_mut()
            && let Err(error) = processor.flush()
        {
            last_warning = Some(error.to_string());
        }
        if reported.elapsed() >= Duration::from_secs(1) || shared.stop_spool.load(Ordering::Acquire)
        {
            let status_started = sampling::qpc();
            let mut status = processor
                .as_ref()
                .map_or_else(|| json!({"hours":0}), Processor::status);
            status["state"] = json!(if shared.recording.load(Ordering::Acquire) {
                "recording"
            } else {
                "detaching"
            });
            status["session"] = json!(session.id);
            status["pool_bytes"] = json!(shared.pool_bytes.load(Ordering::Relaxed));
            status["dropped_frames"] = json!(shared.dropped.load(Ordering::Relaxed));
            status["startup_unattributed_callbacks"] = json!(startup_unknown);
            status["drain_complete"] = json!(false);
            if let Some(error) = &last_warning {
                status["warning"] = json!(error);
            }
            work.add_status(&mut status, shared.frequency);
            if let Err(error) = write_status(status_path, &status) {
                last_warning = Some(error.to_string());
            }
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
            if let Some(processor) = processor.as_mut() {
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
                status["dropped_frames"] = json!(shared.dropped.load(Ordering::Relaxed));
                status["drain_complete"] = json!(false);
                if let Some(error) = &last_warning {
                    status["warning"] = json!(error);
                }
                if let Err(error) = &drain_result {
                    status["error"] = json!(error);
                }
                work.add_status(&mut status, shared.frequency);
                if let Err(error) = write_status(status_path, &status) {
                    // The loader still publishes the authoritative detached
                    // status after joining us. Retain this diagnostic in memory
                    // so successful terminal publication can report recovery.
                    last_warning = Some(format!("cannot publish final spool status: {error}"));
                }
                work.status_update
                    .record_ticks(sampling::qpc() - status_started, shared.frequency);
            } else if pending.unknown_count != 0 || pending.iter().next().is_some() {
                add_terminal_error(
                    &mut drain_result,
                    "retained loss witnesses have no initialized spool at detach".into(),
                );
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
