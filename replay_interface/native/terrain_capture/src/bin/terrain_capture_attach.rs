use std::collections::BTreeSet;
use std::env;
use std::ffi::{CString, OsStr};
use std::fs;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{
    CloseHandle, FreeLibrary, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CreateRemoteThread, GetExitCodeProcess, GetExitCodeThread, OpenProcess,
    ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED, INFINITE,
    PROCESS_CREATE_THREAD, PROCESS_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION,
    PROCESS_VM_READ, PROCESS_VM_WRITE, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, HOOKPROC, WH_GETMESSAGE, WM_NULL,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CONTROL_FILE: &str = "terrain_capture_control.json";
const CAPTURE_DIR_NAME: &str = "terrain_capture";
const CAPTURE_EVENTS_FILE: &str = "terrain_capture_events.jsonl";
const CAPTURE_PROBE_FILE: &str = "terrain_capture_probe.jsonl";
const ONEDRIVE_MARKER: &str = ".849C9593-D756-4E56-8D6E-42412F2A707B";
const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
const TERRAIN_CHUNK_LIBRARY_MOMENT: &str = "terrain_chunk_library";
const WATER_CAPTURE_MOMENT: &str = "water_pass";
const RIVERS_CAPTURE_MOMENT: &str = "rivers_pass";
const BORDERS_CAPTURE_MOMENT: &str = "borders_pass";
const UNDERWATER_CAPTURE_MOMENT: &str = "underwater_pass";
const TREES_CAPTURE_MOMENT: &str = "trees_pass";
const RESTORE_CAPTURE_MOMENT: &str = "restore_pass";
const MAPNAME_CAPTURE_MOMENT: &str = "mapname_pass";
const UI_PASS_CAPTURE_MOMENT: &str = "ui_pass";
const DEFAULT_MAX_DRAWS_PER_PASS: u32 = 128;
const DEFAULT_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION: u64 = 1_073_741_824;
const DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_DRAWS_PER_PASS: u32 = 8_192;
const DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION: u64 = 2_147_483_648;
const DEFAULT_MAX_CAPTURE_WORK_MS_PER_FRAME: u64 = 1_000;
const DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_CAPTURE_WORK_MS_PER_FRAME: u64 = 2_000;
const DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES: u64 = 536_870_912;
const DEFAULT_UI_PASS_MAX_STATES: u32 = 64;
/// Capture sessions may run for up to 30 minutes so slow the game continue-save
/// loads still leave enough armed time for synchronized evidence collection.
const MAX_DWELL_MS: u64 = 1_800_000;

fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    let source_dll = PathBuf::from(required_value(&args, "--dll")?);
    let dll = stage_runtime_dll_copy(&source_dll)?;
    let harness_arg = value_after(&args, "--harness");
    let dwell_ms = value_after(&args, "--dwell-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(if harness_arg.is_some() {
            120_000
        } else {
            30_000
        });
    // Reject anything above the explicit 30-minute capture ceiling rather than
    // silently clamping. With Bug 1 fixed, dwell expiry is safe (the DLL
    // disarms; it never unhooks or unloads at runtime).
    validate_dwell_ms(dwell_ms)?;
    let settle_ms = value_after(&args, "--settle-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(8_000);
    let accept_probe_before_resume = args.iter().any(|arg| arg == "--accept-probe-before-resume");
    let pre_resume_timeout_ms = value_after(&args, "--pre-resume-timeout-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(30_000);
    let capture_out_dir_arg = value_after(&args, "--capture-out-dir");
    let capture_out_dir = resolved_capture_output_dir(&dll, capture_out_dir_arg.as_deref())?;
    clear_capture_event_log(&capture_out_dir)?;
    warn_if_synced_capture_output_dir(&capture_out_dir)?;
    let max_payload_writer_queue_bytes = value_after(&args, "--max-payload-writer-queue-bytes")
        .map(|value| value.parse::<u64>())
        .transpose()?;
    let max_total_payload_bytes_per_session =
        value_after(&args, "--max-total-payload-bytes-per-session")
            .map(|value| value.parse::<u64>())
            .transpose()?;
    let max_capture_work_ms_per_frame = value_after(&args, "--max-capture-work-ms-per-frame")
        .map(|value| value.parse::<u64>())
        .transpose()?;
    // CAPFIX3 — operator escape hatch for the per-frame map-frame draw cap
    // (default stays DEFAULT_MAX_DRAWS_PER_PASS / the chunk-library default).
    let max_draws_per_pass = parse_max_draws_per_pass_arg(&args)?;
    let ui_max_states = value_after(&args, "--ui-max-states")
        .map(|value| value.parse::<u32>())
        .transpose()?;
    let ui_cb_skip = value_after(&args, "--ui-cb-skip").unwrap_or_else(|| "hazard".to_string());
    validate_ui_cb_skip(&ui_cb_skip)?;
    let enable_map_frame_rounds = args.iter().any(|arg| arg == "--enable-map-frame-rounds");
    let census = args.iter().any(|arg| arg == "--census");
    // Persistent scene-CB0 sampler cadence (ms). Absent/0 => sampler fully off.
    let cb0_sample_ms = value_after(&args, "--cb0-sample-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .filter(|interval| *interval > 0);
    // CAPFIX3 — choreography: delay arming the moment sequence by N ms after the
    // map renderer first goes live, so the orchestrator can zoom/position the
    // camera before capture begins. Absent/0 => arm immediately (prior
    // behavior). This is what the never-firing-token workaround and the
    // zoom-race dance existed to fake tonight; a first-class delay removes both.
    let arm_delay_ms = value_after(&args, "--arm-delay-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .filter(|delay| *delay > 0);
    let trees_close_mode = value_after(&args, "--trees-close-mode");
    if let Some(mode) = trees_close_mode.as_deref() {
        if mode != "frame-repeat" {
            return Err(
                format!("unsupported --trees-close-mode {mode}; expected frame-repeat").into(),
            );
        }
    }
    let capture_moments = capture_moments_from_args(&args)?;
    let shader_cache_arg = value_after(&args, "--shader-cache");
    let harness_bundle_control = value_after(&args, "--harness-bundle-control").map(PathBuf::from);
    if shader_cache_arg.is_some() && harness_bundle_control.is_some() {
        return Err("pass either --shader-cache or --harness-bundle-control, not both".into());
    }
    if let Some(shader_cache) = shader_cache_arg {
        write_shader_cache_control(
            &dll,
            &PathBuf::from(shader_cache),
            &capture_moments,
            settle_ms,
            Some(dwell_ms),
            Some(&capture_out_dir),
            max_payload_writer_queue_bytes,
            max_total_payload_bytes_per_session,
            max_capture_work_ms_per_frame,
            max_draws_per_pass,
            ui_max_states,
            &ui_cb_skip,
            census,
            enable_map_frame_rounds,
            trees_close_mode.as_deref(),
            cb0_sample_ms,
            arm_delay_ms,
        )?;
    } else if let Some(bundle) = harness_bundle_control {
        write_harness_bundle_control(
            &dll,
            &bundle,
            &capture_moments,
            settle_ms,
            Some(dwell_ms),
            Some(&capture_out_dir),
            max_payload_writer_queue_bytes,
            max_total_payload_bytes_per_session,
            max_capture_work_ms_per_frame,
            max_draws_per_pass,
            ui_max_states,
            &ui_cb_skip,
            census,
            enable_map_frame_rounds,
            trees_close_mode.as_deref(),
            cb0_sample_ms,
            arm_delay_ms,
        )?;
    } else if !capture_moments.is_empty()
        || capture_out_dir_arg.is_some()
        || max_payload_writer_queue_bytes.is_some()
        || max_total_payload_bytes_per_session.is_some()
        || max_capture_work_ms_per_frame.is_some()
        || max_draws_per_pass.is_some()
        || ui_max_states.is_some()
        || ui_cb_skip != "hazard"
        || census
        || enable_map_frame_rounds
        || trees_close_mode.is_some()
        || cb0_sample_ms.is_some()
        || arm_delay_ms.is_some()
    {
        write_capture_moment_control(
            &dll,
            &capture_moments,
            settle_ms,
            Some(dwell_ms),
            Some(&capture_out_dir),
            max_payload_writer_queue_bytes,
            max_total_payload_bytes_per_session,
            max_capture_work_ms_per_frame,
            max_draws_per_pass,
            ui_max_states,
            &ui_cb_skip,
            census,
            enable_map_frame_rounds,
            trees_close_mode.as_deref(),
            cb0_sample_ms,
            arm_delay_ms,
        )?;
    }
    if let Some(harness) = harness_arg {
        let exe = PathBuf::from(harness);
        let launch_args = value_after(&args, "--args").unwrap_or_default();
        let working_dir = value_after(&args, "--working-dir")
            .map(PathBuf::from)
            .or_else(|| exe.parent().map(Path::to_path_buf))
            .ok_or("harness executable path has no parent; pass --working-dir")?;
        launch_with_early_terrain_capture(
            &exe,
            &dll,
            &working_dir,
            &launch_args,
            pre_resume_timeout_ms,
            dwell_ms,
            &capture_out_dir,
            LaunchMode::Harness,
            accept_probe_before_resume,
        )?;
    } else if let Some(exe) = value_after(&args, "--exe") {
        let exe = PathBuf::from(exe);
        let launch_args = value_after(&args, "--args").unwrap_or_default();
        let working_dir = value_after(&args, "--working-dir")
            .map(PathBuf::from)
            .or_else(|| exe.parent().map(Path::to_path_buf))
            .ok_or("launched executable path has no parent; pass --working-dir")?;
        launch_with_early_terrain_capture(
            &exe,
            &dll,
            &working_dir,
            &launch_args,
            pre_resume_timeout_ms,
            dwell_ms,
            &capture_out_dir,
            LaunchMode::DwellOnly,
            accept_probe_before_resume,
        )?;
    } else {
        let pid = required_value(&args, "--pid")?.parse::<u32>()?;
        if args.iter().any(|arg| arg == "--direct-load-library") {
            attach_terrain_capture_direct(pid, &dll, dwell_ms, &capture_out_dir)?;
        } else {
            attach_terrain_capture(pid, &dll, dwell_ms, &capture_out_dir)?;
        }
    }
    Ok(())
}

fn launch_with_early_terrain_capture(
    exe: &Path,
    dll: &Path,
    working_dir: &Path,
    launch_args: &str,
    pre_resume_timeout_ms: u64,
    dwell_ms: u64,
    capture_out_dir: &Path,
    mode: LaunchMode,
    accept_probe_before_resume: bool,
) -> Result<()> {
    if !exe.is_file() {
        return Err(format!("launched executable does not exist: {}", exe.display()).into());
    }
    if !dll.is_file() {
        return Err(format!("terrain capture DLL does not exist: {}", dll.display()).into());
    }
    if !working_dir.is_dir() {
        return Err(format!(
            "working directory does not exist: {}",
            working_dir.display()
        )
        .into());
    }
    if let Some(file_name) = dll.file_name().and_then(|name| name.to_str()) {
        env::set_var("TERRAIN_CAPTURE_DLL_NAME", file_name);
    }
    env::set_var("TERRAIN_CAPTURE_OUTPUT_DIR", capture_out_dir);
    if let Some(dir) = dll.parent() {
        env::set_var("TERRAIN_CAPTURE_CONTROL_PATH", dir.join(CONTROL_FILE));
    }
    let mut startup = STARTUPINFOW {
        cb: mem::size_of::<STARTUPINFOW>() as u32,
        ..STARTUPINFOW::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    let application = wide_null_path(exe);
    let current_directory = wide_null_path(working_dir);
    let mut command_line = wide_null_string(&command_line_for_launch(exe, launch_args));
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_SUSPENDED,
            ptr::null(),
            current_directory.as_ptr(),
            &mut startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(format!(
            "CreateProcessW suspended launch failed for {}: {}",
            exe.display(),
            std::io::Error::last_os_error()
        )
        .into());
    }
    let process_handles = ProcessHandles::new(process);
    if accept_probe_before_resume {
        clear_capture_probe_log(dll)?;
    }
    let remote_dll = inject_dll_by_load_library(process_handles.process, dll)?;
    let dll_dir = dll
        .parent()
        .ok_or("terrain capture DLL path has no parent directory")?;
    let ready_signal = match wait_for_capture_hooks_installed(
        capture_out_dir,
        dll_dir,
        pre_resume_timeout_ms,
        accept_probe_before_resume,
    ) {
        Ok(ready_signal) => ready_signal,
        Err(error) => {
            unsafe {
                TerminateProcess(process_handles.process, 1);
            }
            return Err(format!(
            "terrain capture DLL loaded into pid {}, but hooks were not installed before resume: {}",
            process_handles.process_id, error
        )
        .into());
        }
    };
    let resumed = unsafe { ResumeThread(process_handles.thread) };
    if resumed == u32::MAX {
        return Err(format!(
            "ResumeThread failed after injecting {}: {}",
            dll.display(),
            std::io::Error::last_os_error()
        )
        .into());
    }
    println!(
        "launched pid {} suspended, injected {}, observed {}, resumed primary thread; remote module handle 0x{:X}",
        process_handles.process_id,
        dll.display(),
        ready_signal,
        remote_dll
    );
    match mode {
        LaunchMode::DwellOnly => {
            thread::sleep(Duration::from_millis(dwell_ms));
            println!(
                "early terrain capture dwell complete for pid {}; check {}",
                process_handles.process_id,
                capture_out_dir.join(CAPTURE_EVENTS_FILE).display()
            );
        }
        LaunchMode::Harness => {
            let wait = unsafe { WaitForSingleObject(process_handles.process, dwell_ms as u32) };
            if wait == WAIT_TIMEOUT {
                unsafe {
                    TerminateProcess(process_handles.process, 2);
                }
                return Err(format!(
                    "capture harness pid {} timed out after {}ms; check {}",
                    process_handles.process_id,
                    dwell_ms,
                    capture_out_dir.join(CAPTURE_EVENTS_FILE).display()
                )
                .into());
            }
            if wait != WAIT_OBJECT_0 {
                return Err(format!(
                    "WaitForSingleObject on capture harness pid {} returned {}",
                    process_handles.process_id, wait
                )
                .into());
            }
            let mut exit_code = 0u32;
            let got_exit = unsafe { GetExitCodeProcess(process_handles.process, &mut exit_code) };
            if got_exit == 0 {
                return Err(format!(
                    "GetExitCodeProcess for capture harness pid {} failed: {}",
                    process_handles.process_id,
                    std::io::Error::last_os_error()
                )
                .into());
            }
            println!(
                "capture harness pid {} exited with code {}; check {}",
                process_handles.process_id,
                exit_code,
                capture_out_dir.join(CAPTURE_EVENTS_FILE).display()
            );
            if exit_code != 0 {
                return Err(format!("capture harness exited with code {exit_code}").into());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchMode {
    DwellOnly,
    Harness,
}

fn stage_runtime_dll_copy(dll: &Path) -> Result<PathBuf> {
    if !dll.is_file() {
        return Err(format!("terrain capture DLL does not exist: {}", dll.display()).into());
    }
    let dll_abs = fs::canonicalize(dll)?;
    let staged = runtime_dll_copy_path(&dll_abs, runtime_dll_nonce())?;
    fs::copy(&dll_abs, &staged).map_err(|error| {
        format!(
            "failed to stage runtime terrain capture DLL {} -> {}: {error}",
            dll_abs.display(),
            staged.display()
        )
    })?;
    Ok(staged)
}

fn runtime_dll_copy_path(dll_abs: &Path, nonce: u128) -> Result<PathBuf> {
    let parent = dll_abs
        .parent()
        .ok_or("terrain capture DLL path has no parent directory")?;
    let stem = dll_abs
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("terrain capture DLL path has no valid UTF-8 file stem")?;
    let extension = dll_abs
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("dll");
    let file_name = format!("{stem}_run_{}_{}.{}", std::process::id(), nonce, extension);
    Ok(parent.join(file_name))
}

fn runtime_dll_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn resolved_capture_output_dir(dll: &Path, capture_out_dir: Option<&str>) -> Result<PathBuf> {
    if let Some(dir) = capture_out_dir {
        let path = PathBuf::from(dir);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Ok(env::current_dir()?.join(path))
        };
    }
    let Some(module_dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    Ok(module_dir.join(CAPTURE_DIR_NAME))
}

fn warn_if_synced_capture_output_dir(capture_out_dir: &Path) -> Result<()> {
    let contains_onedrive = capture_out_dir
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("onedrive");
    let has_marker = path_has_sync_marker(capture_out_dir, ONEDRIVE_MARKER);
    if !contains_onedrive && !has_marker {
        return Ok(());
    }
    fs::create_dir_all(capture_out_dir)?;
    let reason = match (contains_onedrive, has_marker) {
        (true, true) => "path_contains_onedrive_and_sync_marker",
        (true, false) => "path_contains_onedrive",
        (false, true) => "sync_marker_detected",
        (false, false) => "none",
    };
    let message = format!(
        "capture output directory may be under synced storage; use fast non-synced storage for terrain captures: {}",
        capture_out_dir.display()
    );
    eprintln!("warning: {message}");
    write_helper_event(
        capture_out_dir,
        &format!(
            "{{\"type\":\"capture_output_dir_warning\",\"reason\":\"{}\",\"path\":\"{}\",\"message\":\"{}\"}}",
            reason,
            json_escape(&capture_out_dir.to_string_lossy()),
            json_escape(&message)
        ),
    );
    Ok(())
}

fn path_has_sync_marker(path: &Path, marker: &str) -> bool {
    path.ancestors()
        .any(|ancestor| ancestor.join(marker).exists())
}

fn write_helper_event(capture_out_dir: &Path, line: &str) {
    let _ = fs::create_dir_all(capture_out_dir);
    let path = capture_out_dir.join(CAPTURE_EVENTS_FILE);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write as _;
        let _ = writeln!(file, "{line}");
    }
}

fn clear_capture_event_log(capture_out_dir: &Path) -> Result<()> {
    let events = capture_out_dir.join(CAPTURE_EVENTS_FILE);
    match fs::remove_file(&events) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to clear stale capture event log {}: {error}",
            events.display()
        )
        .into()),
    }
}

fn clear_capture_probe_log(dll: &Path) -> Result<()> {
    let Some(module_dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    let probe = module_dir.join(CAPTURE_PROBE_FILE);
    match fs::remove_file(&probe) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to clear stale capture probe log {}: {error}",
            probe.display()
        )
        .into()),
    }
}

fn wait_for_capture_hooks_installed(
    capture_out_dir: &Path,
    module_dir: &Path,
    timeout_ms: u64,
    accept_probe_before_resume: bool,
) -> Result<&'static str> {
    let events = capture_out_dir.join(CAPTURE_EVENTS_FILE);
    let probe = module_dir.join(CAPTURE_PROBE_FILE);
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    loop {
        if let Ok(text) = fs::read_to_string(&events) {
            if text.contains("\"terrain_capture_hooks_installed\"") {
                return Ok("hooks installed");
            }
            if text.contains("\"terrain_capture_blocked\"") {
                return Err(format!(
                    "capture worker reported blocked before hook install: {}",
                    last_nonempty_line(&text).unwrap_or("<no event line>")
                )
                .into());
            }
        }
        if accept_probe_before_resume {
            if let Ok(text) = fs::read_to_string(&probe) {
                if text.contains("\"terrain_capture_started\"") {
                    return Ok("legacy probe start");
                }
            }
        }
        if start.elapsed() >= timeout {
            let wait_target = if accept_probe_before_resume {
                format!("{} or {}", events.display(), probe.display())
            } else {
                events.display().to_string()
            };
            return Err(format!("timed out after {timeout_ms}ms waiting for {wait_target}").into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn last_nonempty_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|line| !line.trim().is_empty())
}

fn inject_dll_by_load_library(process: HANDLE, dll: &Path) -> Result<usize> {
    let dll_abs = fs::canonicalize(dll)?;
    let dll_wide = wide_null_path(&dll_abs);
    let byte_len = dll_wide
        .len()
        .checked_mul(mem::size_of::<u16>())
        .ok_or("DLL path byte length overflowed")?;
    let remote_mem = unsafe {
        VirtualAllocEx(
            process,
            ptr::null(),
            byte_len,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote_mem.is_null() {
        return Err(format!(
            "VirtualAllocEx failed for remote DLL path: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }
    let remote_mem = RemoteAlloc {
        process,
        ptr: remote_mem,
    };
    let mut written = 0usize;
    let wrote = unsafe {
        WriteProcessMemory(
            process,
            remote_mem.ptr,
            dll_wide.as_ptr().cast(),
            byte_len,
            &mut written,
        )
    };
    if wrote == 0 || written != byte_len {
        return Err(format!(
            "WriteProcessMemory wrote {} of {} DLL-path bytes: {}",
            written,
            byte_len,
            std::io::Error::last_os_error()
        )
        .into());
    }

    let kernel32 = unsafe { GetModuleHandleW(wide_null_string("kernel32.dll").as_ptr()) };
    if kernel32.is_null() {
        return Err(format!(
            "GetModuleHandleW(kernel32.dll) failed: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }
    let proc_name = CString::new("LoadLibraryW")?;
    let load_library = unsafe { GetProcAddress(kernel32, proc_name.as_ptr().cast()) };
    let Some(load_library) = load_library else {
        return Err(format!(
            "GetProcAddress(LoadLibraryW) failed: {}",
            std::io::Error::last_os_error()
        )
        .into());
    };
    let start: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32 =
        unsafe { mem::transmute(load_library) };
    let thread = unsafe {
        CreateRemoteThread(
            process,
            ptr::null(),
            0,
            Some(start),
            remote_mem.ptr,
            0,
            ptr::null_mut(),
        )
    };
    if thread.is_null() {
        return Err(format!(
            "CreateRemoteThread(LoadLibraryW) failed: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }
    let thread = HandleGuard(thread);
    let wait = unsafe { WaitForSingleObject(thread.0, INFINITE) };
    if wait != WAIT_OBJECT_0 {
        return Err(format!("WaitForSingleObject on remote LoadLibraryW returned {wait}").into());
    }
    let mut exit_code = 0u32;
    let got_exit = unsafe { GetExitCodeThread(thread.0, &mut exit_code) };
    if got_exit == 0 {
        return Err(format!(
            "GetExitCodeThread for remote LoadLibraryW failed: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }
    if exit_code == 0 {
        return Err(format!(
            "remote LoadLibraryW returned null for {}",
            dll_abs.display()
        )
        .into());
    }
    Ok(exit_code as usize)
}

struct ProcessHandles {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
}

impl ProcessHandles {
    fn new(process: PROCESS_INFORMATION) -> Self {
        Self {
            process: process.hProcess,
            thread: process.hThread,
            process_id: process.dwProcessId,
        }
    }
}

impl Drop for ProcessHandles {
    fn drop(&mut self) {
        unsafe {
            if !self.thread.is_null() {
                CloseHandle(self.thread);
            }
            if !self.process.is_null() {
                CloseHandle(self.process);
            }
        }
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                CloseHandle(self.0);
            }
        }
    }
}

struct RemoteAlloc {
    process: HANDLE,
    ptr: *mut core::ffi::c_void,
}

impl Drop for RemoteAlloc {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                VirtualFreeEx(self.process, self.ptr, 0, MEM_RELEASE);
            }
        }
    }
}

fn attach_terrain_capture(
    pid: u32,
    dll: &PathBuf,
    dwell_ms: u64,
    capture_out_dir: &Path,
) -> Result<()> {
    if !dll.is_file() {
        return Err(format!("terrain capture DLL does not exist: {}", dll.display()).into());
    }
    let hwnd = find_main_window(pid).ok_or_else(|| {
        format!("no visible top-level window found for pid {pid}; the game must be running with a UI")
    })?;
    let mut window_pid = 0u32;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut window_pid) };
    if thread_id == 0 || window_pid != pid {
        return Err(format!("window thread lookup failed for pid {pid}").into());
    }

    let module = unsafe { LoadLibraryW(wide_null_path(dll).as_ptr()) };
    if module.is_null() {
        return Err(format!("LoadLibraryW failed for {}", dll.display()).into());
    }
    let proc_name = CString::new("Hoi4TerrainCaptureGetMsgProc")?;
    let proc = unsafe { GetProcAddress(module, proc_name.as_ptr() as *const u8) };
    if proc.is_none() {
        unsafe {
            FreeLibrary(module);
        }
        return Err("GetProcAddress(Hoi4TerrainCaptureGetMsgProc) failed".into());
    }
    let hook_proc: HOOKPROC = unsafe { mem::transmute(proc) };
    let hook: HHOOK = unsafe { SetWindowsHookExW(WH_GETMESSAGE, hook_proc, module, thread_id) };
    if hook.is_null() {
        unsafe {
            FreeLibrary(module);
        }
        return Err("SetWindowsHookExW failed".into());
    }
    unsafe {
        PostThreadMessageW(thread_id, WM_NULL, 0 as WPARAM, 0 as LPARAM);
    }
    thread::sleep(Duration::from_millis(dwell_ms));
    unsafe {
        UnhookWindowsHookEx(hook);
        FreeLibrary(module);
    }
    println!(
        "terrain capture DLL attached to pid {pid} on thread {thread_id}; check {}",
        capture_out_dir.join(CAPTURE_EVENTS_FILE).display()
    );
    Ok(())
}

fn attach_terrain_capture_direct(
    pid: u32,
    dll: &PathBuf,
    dwell_ms: u64,
    capture_out_dir: &Path,
) -> Result<()> {
    if !dll.is_file() {
        return Err(format!("terrain capture DLL does not exist: {}", dll.display()).into());
    }
    let access = PROCESS_CREATE_THREAD
        | PROCESS_QUERY_INFORMATION
        | PROCESS_VM_OPERATION
        | PROCESS_VM_WRITE
        | PROCESS_VM_READ;
    let process = unsafe { OpenProcess(access, 0, pid) };
    if process.is_null() {
        return Err(format!(
            "OpenProcess for direct one-shot injection into pid {pid} failed: {}",
            std::io::Error::last_os_error()
        )
        .into());
    }
    let process = HandleGuard(process);
    let remote_dll = inject_dll_by_load_library(process.0, dll)?;
    let dll_dir = dll
        .parent()
        .ok_or("terrain capture DLL path has no parent directory")?;
    let ready = wait_for_capture_hooks_installed(capture_out_dir, dll_dir, 30_000, false)?;
    println!(
        "terrain capture DLL directly loaded once into pid {pid}; observed {ready}; remote module handle low bits 0x{remote_dll:X}"
    );
    thread::sleep(Duration::from_millis(dwell_ms));
    println!(
        "direct terrain capture dwell complete for pid {pid}; check {}",
        capture_out_dir.join(CAPTURE_EVENTS_FILE).display()
    );
    Ok(())
}

fn find_main_window(pid: u32) -> Option<HWND> {
    #[repr(C)]
    struct Search {
        pid: u32,
        hwnd: HWND,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut window_pid = 0u32;
        let thread = unsafe { GetWindowThreadProcessId(hwnd, &mut window_pid) };
        if thread != 0 && window_pid == search.pid && unsafe { IsWindowVisible(hwnd) } != 0 {
            search.hwnd = hwnd;
            return 0;
        }
        1
    }

    let mut search = Search {
        pid,
        hwnd: ptr::null_mut(),
    };
    unsafe {
        EnumWindows(Some(enum_proc), &mut search as *mut Search as LPARAM);
    }
    (!search.hwnd.is_null()).then_some(search.hwnd)
}

fn required_value(args: &[String], flag: &str) -> Result<String> {
    value_after(args, flag).ok_or_else(|| format!("missing required {flag}").into())
}

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

/// CAPFIX3 — parse `--max-draws-per-pass`. Reject 0 loudly: a zero cap would
/// refuse the first draw of every pass and capture nothing (the DLL also clamps
/// to >=1 defensively, but a typo should surface to the operator, not be
/// silently neutered). Absent => None (moment-appropriate default applies).
fn parse_max_draws_per_pass_arg(args: &[String]) -> Result<Option<u32>> {
    let value = value_after(args, "--max-draws-per-pass")
        .map(|value| value.parse::<u32>())
        .transpose()?;
    if let Some(0) = value {
        return Err("--max-draws-per-pass must be >= 1 (0 would reject the first draw of every pass and capture nothing)".into());
    }
    Ok(value)
}

/// The attach helper rejects any `--dwell-ms` above [`MAX_DWELL_MS`] (30
/// minutes) with a clear error. Values at or under the cap
/// pass through unchanged.
fn validate_dwell_ms(dwell_ms: u64) -> Result<()> {
    if dwell_ms > MAX_DWELL_MS {
        return Err(format!(
            "--dwell-ms {dwell_ms} exceeds the {MAX_DWELL_MS} ms (30 minute) cap; \
             pass a value <= {MAX_DWELL_MS}"
        )
        .into());
    }
    Ok(())
}

fn wide_null_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn wide_null_string(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain([0]).collect()
}

fn command_line_for_launch(exe: &Path, args: &str) -> String {
    let exe = quote_windows_arg(&exe.as_os_str().to_string_lossy());
    if args.trim().is_empty() {
        exe
    } else {
        format!("{exe} {}", args.trim())
    }
}

fn quote_windows_arg(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"'));
    if !needs_quotes {
        return value.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat('\\').take(backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat('\\').take(backslashes * 2));
    quoted.push('"');
    quoted
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShaderCacheControlEntry {
    file: String,
    map_size_x: u32,
    map_size_y: u32,
    map_pow2_x_milli: u32,
    map_pow2_y_milli: u32,
    fow_pow2_x_milli: u32,
    fow_pow2_y_milli: u32,
    terrain_id_srv_slot: u32,
}

type ShaderCacheControlParts = (
    u32,
    u32,
    BTreeSet<u32>,
    BTreeSet<(u32, String, u32)>,
    Vec<ShaderCacheControlEntry>,
);

fn write_shader_cache_control(
    dll: &Path,
    shader_cache: &Path,
    capture_moments: &[String],
    settle_ms: u64,
    dwell_ms: Option<u64>,
    capture_out_dir: Option<&Path>,
    max_payload_writer_queue_bytes: Option<u64>,
    max_total_payload_bytes_per_session: Option<u64>,
    max_capture_work_ms_per_frame: Option<u64>,
    max_draws_per_pass: Option<u32>,
    ui_max_states: Option<u32>,
    ui_cb_skip: &str,
    census: bool,
    round_commands_enabled: bool,
    trees_close_mode: Option<&str>,
    cb0_sample_ms: Option<u64>,
    arm_delay_ms: Option<u64>,
) -> Result<()> {
    let capture_moments = normalized_capture_moments(capture_moments);
    let mut files = Vec::new();
    collect_scache_files(shader_cache, &mut files)?;
    files.sort();
    let mut slots = BTreeSet::new();
    let mut paired_resource_slots = BTreeSet::new();
    let mut constants = None;
    let mut shader_cache_signatures = Vec::new();
    for path in files {
        let text = String::from_utf8_lossy(&fs::read(&path)?).to_string();
        let Some(signature) = terrain_capture::shader_cache_signature(&text) else {
            continue;
        };
        let current = (
            signature.map_size_x,
            signature.map_size_y,
            signature.map_pow2_x_milli,
            signature.map_pow2_y_milli,
            signature.fow_pow2_x_milli,
            signature.fow_pow2_y_milli,
        );
        if let Some(previous) = constants {
            if previous != current {
                return Err(format!(
                    "shader cache TerrainIDMap MAP/FOW constants mismatch: {:?} vs {:?}",
                    previous, current
                )
                .into());
            }
        } else {
            constants = Some(current);
        }
        shader_cache_signatures.push(ShaderCacheControlEntry {
            file: shader_cache_relative_name(shader_cache, &path),
            map_size_x: signature.map_size_x,
            map_size_y: signature.map_size_y,
            map_pow2_x_milli: signature.map_pow2_x_milli,
            map_pow2_y_milli: signature.map_pow2_y_milli,
            fow_pow2_x_milli: signature.fow_pow2_x_milli,
            fow_pow2_y_milli: signature.fow_pow2_y_milli,
            terrain_id_srv_slot: signature.terrain_id_srv_slot,
        });
        slots.insert(signature.terrain_id_srv_slot);
        if let Some(snow_mud_slot) = terrain_capture::shader_resource_slot_from_shader_text(
            &text,
            terrain_capture::SNOW_MUD_DATA_RESOURCE_SYMBOL,
        ) {
            paired_resource_slots.insert((
                signature.terrain_id_srv_slot,
                terrain_capture::SNOW_MUD_DATA_RESOURCE_SYMBOL.to_string(),
                snow_mud_slot,
            ));
        }
    }
    let Some((width, height, _, _, _, _)) = constants else {
        return Err(format!(
            "no TerrainIDMap_Texture register(tN) shader cache found under {}",
            shader_cache.display()
        )
        .into());
    };
    let control = terrain_capture_control_json(
        &capture_moments,
        width,
        height,
        &slots,
        &paired_resource_slots,
        &shader_cache_signatures,
        settle_ms,
        dwell_ms,
        capture_out_dir,
        max_payload_writer_queue_bytes,
        max_total_payload_bytes_per_session,
        max_capture_work_ms_per_frame,
        max_draws_per_pass,
        ui_max_states,
        ui_cb_skip,
        census,
        round_commands_enabled,
        trees_close_mode,
        true,
        cb0_sample_ms,
        arm_delay_ms,
    );
    let Some(dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    fs::write(dir.join(CONTROL_FILE), control)?;
    Ok(())
}

fn write_harness_bundle_control(
    dll: &Path,
    bundle: &Path,
    capture_moments: &[String],
    settle_ms: u64,
    dwell_ms: Option<u64>,
    capture_out_dir: Option<&Path>,
    max_payload_writer_queue_bytes: Option<u64>,
    max_total_payload_bytes_per_session: Option<u64>,
    max_capture_work_ms_per_frame: Option<u64>,
    max_draws_per_pass: Option<u32>,
    ui_max_states: Option<u32>,
    ui_cb_skip: &str,
    census: bool,
    round_commands_enabled: bool,
    trees_close_mode: Option<&str>,
    cb0_sample_ms: Option<u64>,
    arm_delay_ms: Option<u64>,
) -> Result<()> {
    let capture_moments = normalized_capture_moments(capture_moments);
    let (width, height, slots, paired_resource_slots, shader_cache_signatures) =
        match harness_bundle_control_parts(bundle) {
            Ok(parts) => parts,
            Err(error)
                if !capture_moments_need_harness_shader_cache_constants(&capture_moments) =>
            {
                if error.to_string().contains("no shader_cache_constants") {
                    empty_harness_bundle_control_parts()
                } else {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        };
    let control = terrain_capture_control_json(
        &capture_moments,
        width,
        height,
        &slots,
        &paired_resource_slots,
        &shader_cache_signatures,
        settle_ms,
        dwell_ms,
        capture_out_dir,
        max_payload_writer_queue_bytes,
        max_total_payload_bytes_per_session,
        max_capture_work_ms_per_frame,
        max_draws_per_pass,
        ui_max_states,
        ui_cb_skip,
        census,
        round_commands_enabled,
        trees_close_mode,
        false,
        cb0_sample_ms,
        arm_delay_ms,
    );
    let Some(dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    fs::write(dir.join(CONTROL_FILE), control)?;
    Ok(())
}

fn capture_moments_need_harness_shader_cache_constants(capture_moments: &[String]) -> bool {
    normalized_capture_moments(capture_moments)
        .iter()
        .any(|moment| moment == TERRAIN_CAPTURE_MOMENT || moment == TERRAIN_CHUNK_LIBRARY_MOMENT)
}

fn empty_harness_bundle_control_parts() -> ShaderCacheControlParts {
    (
        0,
        0,
        BTreeSet::new(),
        BTreeSet::new(),
        Vec::<ShaderCacheControlEntry>::new(),
    )
}

fn harness_bundle_control_parts(bundle: &Path) -> Result<ShaderCacheControlParts> {
    let bundle_file = bundle.join("map_frame_bundle.json");
    let bytes = fs::read(&bundle_file)
        .map_err(|error| format!("failed to read {}: {error}", bundle_file.display()))?;
    let value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", bundle_file.display()))?;
    harness_bundle_control_parts_from_value(&value)
}

fn harness_bundle_control_parts_from_value(value: &Value) -> Result<ShaderCacheControlParts> {
    let constants = value
        .get("shader_cache_constants")
        .ok_or("map_frame_bundle.json has no shader_cache_constants")?;
    let width = json_number_u32(constants, "map_size_x")?;
    let height = json_number_u32(constants, "map_size_y")?;
    let map_pow2_x_milli = json_number_milli(constants, "map_pow2_x")?;
    let map_pow2_y_milli = json_number_milli(constants, "map_pow2_y")?;
    let fow_pow2_x_milli = json_number_milli(constants, "fow_pow2_x")?;
    let fow_pow2_y_milli = json_number_milli(constants, "fow_pow2_y")?;
    let terrain_id_srv_slot = json_number_u32(constants, "terrain_id_srv_slot")?;
    let mut slots = BTreeSet::new();
    slots.insert(terrain_id_srv_slot);

    let mut shader_cache_signatures = Vec::new();
    if let Some(cache_files) = constants.get("cache_files").and_then(Value::as_array) {
        for file in cache_files {
            let Some(file) = file.as_str() else {
                continue;
            };
            shader_cache_signatures.push(ShaderCacheControlEntry {
                file: file.to_string(),
                map_size_x: width,
                map_size_y: height,
                map_pow2_x_milli,
                map_pow2_y_milli,
                fow_pow2_x_milli,
                fow_pow2_y_milli,
                terrain_id_srv_slot,
            });
        }
    }
    if shader_cache_signatures.is_empty() {
        shader_cache_signatures.push(ShaderCacheControlEntry {
            file: "map_frame_bundle.json".to_string(),
            map_size_x: width,
            map_size_y: height,
            map_pow2_x_milli,
            map_pow2_y_milli,
            fow_pow2_x_milli,
            fow_pow2_y_milli,
            terrain_id_srv_slot,
        });
    }

    let mut paired_resource_slots = BTreeSet::new();
    if let Some(draws) = value.get("draws").and_then(Value::as_array) {
        for draw in draws {
            let Some(resources) = draw.get("shader_resources").and_then(Value::as_array) else {
                continue;
            };
            for resource in resources {
                let declared = resource
                    .get("declared_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if declared != terrain_capture::SNOW_MUD_DATA_RESOURCE_SYMBOL {
                    continue;
                }
                let Some(slot) = resource
                    .get("slot")
                    .and_then(Value::as_u64)
                    .and_then(|slot| u32::try_from(slot).ok())
                else {
                    continue;
                };
                paired_resource_slots.insert((
                    terrain_id_srv_slot,
                    terrain_capture::SNOW_MUD_DATA_RESOURCE_SYMBOL.to_string(),
                    slot,
                ));
            }
        }
    }

    Ok((
        width,
        height,
        slots,
        paired_resource_slots,
        shader_cache_signatures,
    ))
}

fn json_number_u32(value: &Value, key: &str) -> Result<u32> {
    let number = value
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("shader_cache_constants.{key} is missing or not numeric"))?;
    if !number.is_finite() || number < 0.0 || number > u32::MAX as f64 {
        return Err(format!("shader_cache_constants.{key} out of u32 range: {number}").into());
    }
    Ok(number.round() as u32)
}

fn json_number_milli(value: &Value, key: &str) -> Result<u32> {
    let number = value
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("shader_cache_constants.{key} is missing or not numeric"))?;
    let milli = number * 1000.0;
    if !milli.is_finite() || milli < 0.0 || milli > u32::MAX as f64 {
        return Err(format!("shader_cache_constants.{key} out of milli range: {number}").into());
    }
    Ok(milli.round() as u32)
}

fn write_capture_moment_control(
    dll: &Path,
    capture_moments: &[String],
    settle_ms: u64,
    dwell_ms: Option<u64>,
    capture_out_dir: Option<&Path>,
    max_payload_writer_queue_bytes: Option<u64>,
    max_total_payload_bytes_per_session: Option<u64>,
    max_capture_work_ms_per_frame: Option<u64>,
    max_draws_per_pass: Option<u32>,
    ui_max_states: Option<u32>,
    ui_cb_skip: &str,
    census: bool,
    round_commands_enabled: bool,
    trees_close_mode: Option<&str>,
    cb0_sample_ms: Option<u64>,
    arm_delay_ms: Option<u64>,
) -> Result<()> {
    let Some(dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    fs::write(
        dir.join(CONTROL_FILE),
        terrain_capture_control_json(
            &capture_moments,
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            settle_ms,
            dwell_ms,
            capture_out_dir,
            max_payload_writer_queue_bytes,
            max_total_payload_bytes_per_session,
            max_capture_work_ms_per_frame,
            max_draws_per_pass,
            ui_max_states,
            ui_cb_skip,
            census,
            round_commands_enabled,
            trees_close_mode,
            true,
            cb0_sample_ms,
            arm_delay_ms,
        ),
    )?;
    Ok(())
}

fn validate_capture_moment(capture_moment: &str) -> Result<()> {
    match capture_moment {
        TERRAIN_CAPTURE_MOMENT
        | TERRAIN_CHUNK_LIBRARY_MOMENT
        | WATER_CAPTURE_MOMENT
        | RIVERS_CAPTURE_MOMENT
        | BORDERS_CAPTURE_MOMENT
        | UNDERWATER_CAPTURE_MOMENT
        | TREES_CAPTURE_MOMENT
        | RESTORE_CAPTURE_MOMENT
        | MAPNAME_CAPTURE_MOMENT
        | UI_PASS_CAPTURE_MOMENT => Ok(()),
        other => Err(format!("unsupported capture moment {other}").into()),
    }
}

fn validate_ui_cb_skip(value: &str) -> Result<()> {
    match value {
        "hazard" | "all" | "none" => Ok(()),
        other => {
            Err(format!("unsupported --ui-cb-skip {other}; expected hazard, all, or none").into())
        }
    }
}

fn capture_moments_from_args(args: &[String]) -> Result<Vec<String>> {
    let single = value_after(args, "--capture-moment");
    let sequence = value_after(args, "--capture-moments");
    if single.is_some() && sequence.is_some() {
        return Err("pass either --capture-moment or --capture-moments, not both".into());
    }
    let raw = sequence
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|moment| !moment.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .or_else(|| single.map(|moment| vec![moment]))
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    for moment in &raw {
        validate_capture_moment(moment)?;
        if !seen.insert(moment.clone()) {
            return Err(format!("duplicate capture moment {moment}").into());
        }
    }
    Ok(raw)
}

fn normalized_capture_moments(capture_moments: &[String]) -> Vec<String> {
    let mut moments = if capture_moments.is_empty() {
        vec![TERRAIN_CAPTURE_MOMENT.to_string()]
    } else {
        capture_moments.to_vec()
    };
    move_ui_pass_to_sequence_tail(&mut moments);
    moments
}

fn move_ui_pass_to_sequence_tail(moments: &mut Vec<String>) {
    if !moments
        .iter()
        .any(|moment| moment == UI_PASS_CAPTURE_MOMENT)
    {
        return;
    }
    moments.retain(|moment| moment != UI_PASS_CAPTURE_MOMENT);
    moments.push(UI_PASS_CAPTURE_MOMENT.to_string());
}

fn terrain_capture_control_json(
    capture_moments: &[String],
    width: u32,
    height: u32,
    slots: &BTreeSet<u32>,
    paired_resource_slots: &BTreeSet<(u32, String, u32)>,
    shader_cache_signatures: &[ShaderCacheControlEntry],
    settle_ms: u64,
    dwell_ms: Option<u64>,
    capture_out_dir: Option<&Path>,
    max_payload_writer_queue_bytes: Option<u64>,
    max_total_payload_bytes_per_session: Option<u64>,
    max_capture_work_ms_per_frame: Option<u64>,
    max_draws_per_pass: Option<u32>,
    ui_max_states: Option<u32>,
    ui_cb_skip: &str,
    census: bool,
    round_commands_enabled: bool,
    trees_close_mode: Option<&str>,
    vanilla_source_evidence_required: bool,
    cb0_sample_ms: Option<u64>,
    arm_delay_ms: Option<u64>,
) -> String {
    let allowed_slots = slots
        .iter()
        .map(|slot| slot.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let paired_slots = paired_resource_slots
        .iter()
        .map(|(terrain_id_srv_slot, symbol, shader_resource_slot)| {
            format!(
                "{{\"terrain_id_srv_slot\":{},\"symbol\":\"{}\",\"shader_resource_slot\":{}}}",
                terrain_id_srv_slot, symbol, shader_resource_slot
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let shader_cache_signatures = shader_cache_signatures
        .iter()
        .map(|entry| {
            format!(
                concat!(
                    "{{\"file\":\"{}\",",
                    "\"map_size_x\":{},\"map_size_y\":{},",
                    "\"map_pow2_x_milli\":{},\"map_pow2_y_milli\":{},",
                    "\"fow_pow2_x_milli\":{},\"fow_pow2_y_milli\":{},",
                    "\"terrain_id_srv_slot\":{}}}"
                ),
                json_escape(&entry.file),
                entry.map_size_x,
                entry.map_size_y,
                entry.map_pow2_x_milli,
                entry.map_pow2_y_milli,
                entry.fow_pow2_x_milli,
                entry.fow_pow2_y_milli,
                entry.terrain_id_srv_slot
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let capture_out_dir_json = capture_out_dir
        .map(|dir| {
            format!(
                "  \"capture_out_dir\": \"{}\",\n",
                json_escape(&dir.to_string_lossy())
            )
        })
        .unwrap_or_default();
    let trees_close_mode_json = trees_close_mode
        .map(|mode| format!("  \"trees_close_mode\": \"{}\",\n", json_escape(mode)))
        .unwrap_or_default();
    let census_json = if census { "  \"census\": true,\n" } else { "" };
    let cb0_sample_ms_json = cb0_sample_ms
        .filter(|interval| *interval > 0)
        .map(|interval| format!("  \"cb0_sample_ms\": {},\n", interval))
        .unwrap_or_default();
    // CAPFIX3 — choreography: only emit arm_delay_ms when set, so an unset flag
    // leaves the field absent and the DLL's serde default (arm immediately)
    // applies. Zero is filtered out at parse time and behaves as absent.
    let arm_delay_ms_json = arm_delay_ms
        .filter(|delay| *delay > 0)
        .map(|delay| format!("  \"arm_delay_ms\": {},\n", delay))
        .unwrap_or_default();
    let map_frame_capture_enabled = !census || !capture_moments.is_empty();
    let capture_moments = if map_frame_capture_enabled {
        normalized_capture_moments(capture_moments)
    } else {
        Vec::new()
    };
    let capture_moment = capture_moments.first().map(String::as_str).unwrap_or("");
    let capture_moments_json = capture_moments
        .iter()
        .map(|moment| format!("\"{}\"", json_escape(moment)))
        .collect::<Vec<_>>()
        .join(",");
    let terrain_chunk_library = capture_moments
        .iter()
        .any(|moment| moment == TERRAIN_CHUNK_LIBRARY_MOMENT);
    // CAPFIX3 — honor the operator's --max-draws-per-pass override; fall back to
    // the moment-appropriate default. Clamp to >=1 (main() already rejects 0,
    // but keep the writer self-consistent with the DLL's own >=1 clamp).
    let max_draws_per_pass = max_draws_per_pass
        .unwrap_or(if terrain_chunk_library {
            DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_DRAWS_PER_PASS
        } else {
            DEFAULT_MAX_DRAWS_PER_PASS
        })
        .max(1);
    let max_total_payload_bytes_per_session =
        max_total_payload_bytes_per_session.unwrap_or(if terrain_chunk_library {
            DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION
        } else {
            DEFAULT_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION
        });
    let max_payload_writer_queue_bytes =
        max_payload_writer_queue_bytes.unwrap_or(DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES);
    let max_capture_work_ms_per_frame =
        max_capture_work_ms_per_frame.unwrap_or(if terrain_chunk_library {
            DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_CAPTURE_WORK_MS_PER_FRAME
        } else {
            DEFAULT_MAX_CAPTURE_WORK_MS_PER_FRAME
        });
    let ui_max_states = ui_max_states.unwrap_or(DEFAULT_UI_PASS_MAX_STATES).max(1);
    format!(
        concat!(
            "{{\n",
            "  \"schema\": \"battle-plan-terrain-capture-control-v1\",\n",
            "{}",
            "  \"capture_moment\": \"{}\",\n",
            "  \"capture_moments\": [{}],\n",
            "  \"map_frame_capture_enabled\": {},\n",
            "  \"terrain_chunk_library\": {},\n",
            "  \"expected_width\": {},\n",
            "  \"expected_height\": {},\n",
            "  \"allowed_slots\": [{}],\n",
            "  \"paired_resource_slots\": [{}],\n",
            "  \"shader_cache_signatures\": [{}],\n",
            "  \"settle_ms\": {},\n",
            "  \"dwell_ms\": {},\n",
            "  \"max_draws_per_pass\": {},\n",
            "  \"max_total_payload_bytes_per_session\": {},\n",
            "  \"max_capture_work_ms_per_frame\": {},\n",
            "  \"max_payload_writer_queue_bytes\": {},\n",
            "  \"ui_max_states\": {},\n",
            "  \"ui_cb_skip\": \"{}\",\n",
            "{}",
            "{}",
            "{}",
            "  \"round_commands_enabled\": {},\n",
            "{}",
            "  \"vanilla_source_evidence_required\": {},\n",
            "  \"provenance\": \"shader_cache_scan_TerrainIDMap_Texture_register_tN\"\n",
            "}}\n"
        ),
        capture_out_dir_json,
        json_escape(capture_moment),
        capture_moments_json,
        map_frame_capture_enabled,
        terrain_chunk_library,
        width,
        height,
        allowed_slots,
        paired_slots,
        shader_cache_signatures,
        settle_ms,
        dwell_ms.map_or_else(|| "null".to_string(), |value| value.to_string()),
        max_draws_per_pass,
        max_total_payload_bytes_per_session,
        max_capture_work_ms_per_frame,
        max_payload_writer_queue_bytes,
        ui_max_states,
        json_escape(ui_cb_skip),
        census_json,
        cb0_sample_ms_json,
        arm_delay_ms_json,
        round_commands_enabled,
        trees_close_mode_json,
        vanilla_source_evidence_required
    )
}

fn collect_scache_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_scache_files(&path, out)?;
            continue;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("scache"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn shader_cache_relative_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            _ => vec![ch],
        })
        .collect()
}

fn print_help() {
    eprintln!(
        "terrain_capture_attach --pid <the game pid> --dll <terrain_capture.dll> [--direct-load-library] [--capture-out-dir <dir>] [--shader-cache <dir>] [--capture-moment terrain_active_pass|terrain_chunk_library|water_pass|rivers_pass|borders_pass|underwater_pass|trees_pass|restore_pass|mapname_pass|ui_pass] [--capture-moments terrain_active_pass,terrain_chunk_library,water_pass,rivers_pass,borders_pass,underwater_pass,trees_pass,restore_pass,mapname_pass,ui_pass] [--census] [--enable-map-frame-rounds] [--trees-close-mode frame-repeat] [--max-payload-writer-queue-bytes <bytes>] [--max-total-payload-bytes-per-session <bytes>] [--ui-max-states <count>] [--ui-cb-skip hazard|all|none] [--dwell-ms <ms, default 30000>]\n\
         terrain_capture_attach --exe <hoi4.exe> --dll <terrain_capture.dll> [--capture-out-dir <dir>] [--working-dir <game dir>] [--args <quoted args>] [--shader-cache <dir>] [--capture-moment terrain_active_pass|terrain_chunk_library|water_pass|rivers_pass|borders_pass|underwater_pass|trees_pass|restore_pass|mapname_pass|ui_pass] [--capture-moments terrain_active_pass,terrain_chunk_library,water_pass,rivers_pass,borders_pass,underwater_pass,trees_pass,restore_pass,mapname_pass,ui_pass] [--census] [--enable-map-frame-rounds] [--trees-close-mode frame-repeat] [--max-payload-writer-queue-bytes <bytes>] [--max-total-payload-bytes-per-session <bytes>] [--ui-max-states <count>] [--ui-cb-skip hazard|all|none] [--pre-resume-timeout-ms <ms, default 30000>] [--dwell-ms <ms, default 30000>] [--settle-ms <ms, default 8000>]\n\
         terrain_capture_attach --harness <capture_harness.exe> --dll <terrain_capture.dll> [--capture-out-dir <dir>] [--working-dir <harness dir>] [--args <quoted harness args>] [--harness-bundle-control <session capture dir>] [--accept-probe-before-resume] [--capture-moments terrain_active_pass,terrain_chunk_library,water_pass,rivers_pass,borders_pass,underwater_pass,trees_pass,restore_pass,mapname_pass,ui_pass] [--census] [--enable-map-frame-rounds] [--trees-close-mode frame-repeat] [--max-payload-writer-queue-bytes <bytes>] [--max-total-payload-bytes-per-session <bytes>] [--ui-max-states <count>] [--ui-cb-skip hazard|all|none] [--pre-resume-timeout-ms <ms, default 30000>] [--dwell-ms <ms, default 120000>] [--settle-ms <ms, default 8000>]\n\
         [--cb0-sample-ms <ms>] arms the persistent scene-CB0 sampler: once the map renderer is live it appends one 416-byte TerrainSceneCB0 snapshot at most every <ms> ms to cb0_samples.jsonl for the WHOLE session (survives moment completion, round completion and the byte-budget quiesce; byte-budget exempt; capped at 100000 samples). Absent => sampler fully off.\n\
         [--max-draws-per-pass <n>] operator escape hatch for the PER-FRAME map-frame draw cap (default 128 for map moments, higher for the chunk library). A pass now closes at every clean frame boundary, so this bounds a SINGLE frame's draws; raise it only if one genuine frame legitimately issues more than the default. Must be >= 1 (0 is rejected).\n\
[--arm-delay-ms <ms>] choreography: delay arming the map-frame moment sequence until <ms> ms AFTER the map renderer first goes live, so the orchestrator can zoom/position the camera before any pass is captured. The CB0 sampler and the renderer-live trust latch are NOT delayed. Absent/0 => arm immediately. This is the first-class replacement for the never-firing-token workaround and the zoom-race dance.\n\
--direct-load-library is an explicit one-shot fallback for a live PID with no visible window. It uses the same remote LoadLibraryW path as suspended launch; never retry it while the target remains alive if module presence is uncertain.\n\
         KEEPALIVE PATTERN (no flag needed): include borders_pass in --capture-moments as a trailing moment to keep the session dwelling and the hooks live while you pan/zoom to set up a later moment. borders_pass only completes on a genuine inked border draw that changes the render target, so an idle camera leaves it pending and the dwell runs to its timeout, holding the renderer live for the CB0 sampler and any --arm-delay-ms window without capturing spurious bundles.\n\
         --dwell-ms is CAPPED at 1800000 (30 minutes); a larger value is rejected. Dwell expiry is safe: the capture DLL disarms its hooks and pins its module (it never unhooks or unloads at runtime); only process exit tears the hooks down."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dwell_cap_rejects_over_thirty_minutes() {
        // Slow continue-save loads need up to 20 minutes, so the helper permits
        // a 30-minute session but still rejects larger values fail-closed.
        assert_eq!(MAX_DWELL_MS, 1_800_000);
        assert!(validate_dwell_ms(0).is_ok());
        assert!(validate_dwell_ms(30_000).is_ok());
        assert!(
            validate_dwell_ms(1_800_000).is_ok(),
            "exactly the cap is allowed"
        );
        let err = validate_dwell_ms(1_800_001).expect_err("just over the cap must reject");
        let message = err.to_string();
        assert!(
            message.contains("1800001") && message.contains("cap"),
            "error must name the offending value and the cap: {message}"
        );
        assert!(validate_dwell_ms(3_600_000).is_err(), "an hour must reject");
    }

    #[test]
    fn terrain_capture_control_records_paired_snow_mud_slots() {
        let slots = BTreeSet::from([3, 5]);
        let paired = BTreeSet::from([(
            5,
            terrain_capture::SNOW_MUD_DATA_RESOURCE_SYMBOL.to_string(),
            7,
        )]);
        let signatures = [ShaderCacheControlEntry {
            file: "dx11_win32/ps_5_0/map.scache".to_string(),
            map_size_x: 5632,
            map_size_y: 2048,
            map_pow2_x_milli: 1000,
            map_pow2_y_milli: 1000,
            fow_pow2_x_milli: 500,
            fow_pow2_y_milli: 250,
            terrain_id_srv_slot: 5,
        }];
        let json = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            5632,
            2048,
            &slots,
            &paired,
            &signatures,
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_moment\": \"terrain_active_pass\""));
        assert!(json.contains("\"capture_moments\": [\"terrain_active_pass\"]"));
        assert!(json.contains("\"terrain_chunk_library\": false"));
        assert!(!json.contains("\"census\""));
        assert!(json.contains("\"allowed_slots\": [3,5]"));
        assert!(json.contains("\"settle_ms\": 8000"));
        assert!(json.contains("\"max_draws_per_pass\": 128"));
        assert!(json.contains("\"max_total_payload_bytes_per_session\": 1073741824"));
        assert!(json.contains("\"max_capture_work_ms_per_frame\": 1000"));
        assert!(json.contains("\"max_payload_writer_queue_bytes\": 536870912"));
        assert!(json.contains("\"ui_max_states\": 64"));
        assert!(json.contains("\"ui_cb_skip\": \"hazard\""));
        assert!(json.contains(
            "\"paired_resource_slots\": [{\"terrain_id_srv_slot\":5,\"symbol\":\"SnowMudData\",\"shader_resource_slot\":7}]"
        ));
        assert!(json.contains("\"fow_pow2_y_milli\":250"));

        let chunk_json = terrain_capture_control_json(
            &[
                TERRAIN_CAPTURE_MOMENT.to_string(),
                TERRAIN_CHUNK_LIBRARY_MOMENT.to_string(),
            ],
            5632,
            2048,
            &slots,
            &paired,
            &signatures,
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(chunk_json.contains("\"terrain_chunk_library\": true"));
        assert!(chunk_json.contains("\"max_capture_work_ms_per_frame\": 2000"));
    }

    #[test]
    fn harness_bundle_control_reads_captured_shader_cache_constants() {
        let value = serde_json::json!({
            "shader_cache_constants": {
                "cache_files": ["dx11_win32/ps_5_0/map.scache"],
                "map_size_x": 5632.0,
                "map_size_y": 2048.0,
                "map_pow2_x": 1.0,
                "map_pow2_y": 1.0,
                "fow_pow2_x": 1.0,
                "fow_pow2_y": 1.0,
                "terrain_id_srv_slot": 5
            },
            "draws": [{
                "shader_resources": [{
                    "slot": 7,
                    "declared_name": "SnowMudData"
                }]
            }]
        });

        let (width, height, slots, paired, signatures) =
            harness_bundle_control_parts_from_value(&value).expect("bundle control parts");

        assert_eq!((width, height), (5632, 2048));
        assert_eq!(slots, BTreeSet::from([5]));
        assert_eq!(
            paired,
            BTreeSet::from([(
                5,
                terrain_capture::SNOW_MUD_DATA_RESOURCE_SYMBOL.to_string(),
                7
            )])
        );
        assert_eq!(signatures.len(), 1);
        assert_eq!(signatures[0].file, "dx11_win32/ps_5_0/map.scache");
        assert_eq!(signatures[0].terrain_id_srv_slot, 5);
    }

    #[test]
    fn harness_bundle_control_requires_shader_constants_only_for_terrain_moments() {
        assert!(capture_moments_need_harness_shader_cache_constants(&[
            TERRAIN_CAPTURE_MOMENT.to_string()
        ]));
        assert!(capture_moments_need_harness_shader_cache_constants(&[
            RESTORE_CAPTURE_MOMENT.to_string(),
            TERRAIN_CHUNK_LIBRARY_MOMENT.to_string()
        ]));
        assert!(!capture_moments_need_harness_shader_cache_constants(&[
            RESTORE_CAPTURE_MOMENT.to_string(),
            UI_PASS_CAPTURE_MOMENT.to_string()
        ]));
        assert!(!capture_moments_need_harness_shader_cache_constants(&[
            MAPNAME_CAPTURE_MOMENT.to_string(),
            UI_PASS_CAPTURE_MOMENT.to_string()
        ]));

        let (width, height, slots, paired, signatures) = empty_harness_bundle_control_parts();
        assert_eq!((width, height), (0, 0));
        assert!(slots.is_empty());
        assert!(paired.is_empty());
        assert!(signatures.is_empty());
    }

    #[test]
    fn terrain_capture_control_can_request_water_moment_without_shader_cache() {
        let json = terrain_capture_control_json(
            &[WATER_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            12_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_moment\": \"water_pass\""));
        assert!(json.contains("\"allowed_slots\": []"));
        assert!(json.contains("\"shader_cache_signatures\": []"));
        assert!(json.contains("\"settle_ms\": 12000"));
    }

    #[test]
    fn terrain_capture_control_can_request_chunk_library_defaults() {
        let moments = capture_moments_from_args(&[
            "--capture-moments".to_string(),
            "terrain_active_pass,terrain_chunk_library".to_string(),
        ])
        .expect("parse chunk library request");

        let json = terrain_capture_control_json(
            &moments,
            5632,
            2048,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            180_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_moment\": \"terrain_active_pass\""));
        assert!(json
            .contains("\"capture_moments\": [\"terrain_active_pass\",\"terrain_chunk_library\"]"));
        assert!(json.contains("\"terrain_chunk_library\": true"));
        assert!(json.contains("\"max_draws_per_pass\": 8192"));
        assert!(json.contains("\"max_total_payload_bytes_per_session\": 2147483648"));
    }

    #[test]
    fn terrain_capture_control_can_request_map_session_moments() {
        let moments = capture_moments_from_args(&[
            "--capture-moments".to_string(),
            "terrain_active_pass,water_pass,rivers_pass,borders_pass,trees_pass,ui_pass"
                .to_string(),
        ])
        .expect("parse capture moment sequence");

        assert_eq!(
            moments,
            vec![
                TERRAIN_CAPTURE_MOMENT.to_string(),
                WATER_CAPTURE_MOMENT.to_string(),
                RIVERS_CAPTURE_MOMENT.to_string(),
                BORDERS_CAPTURE_MOMENT.to_string(),
                TREES_CAPTURE_MOMENT.to_string(),
                UI_PASS_CAPTURE_MOMENT.to_string()
            ]
        );
        let json = terrain_capture_control_json(
            &moments,
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_moment\": \"terrain_active_pass\""));
        assert!(json.contains(
            "\"capture_moments\": [\"terrain_active_pass\",\"water_pass\",\"rivers_pass\",\"borders_pass\",\"trees_pass\",\"ui_pass\"]"
        ));
    }

    /// CAPFIX1 step 4: the exact recommended next-session recipe — the planned
    /// base list with the fixed `borders_pass` and the new `underwater_pass`
    /// appended — must parse, pass `validate_capture_moment`, and inject verbatim.
    /// This is the end-to-end proof that the CLI accepts `underwater_pass` (the
    /// piece the inherited diff left unwired: the attach binary carries its own
    /// moment allowlist, separate from d3d11_runtime's profile table).
    #[test]
    fn capfix1_recommended_session_moment_string_parses_and_validates() {
        let moments = capture_moments_from_args(&[
            "--capture-moments".to_string(),
            "restore_pass,terrain_active_pass,water_pass,rivers_pass,mapname_pass,\
             borders_pass,underwater_pass"
                .to_string(),
        ])
        .expect("parse CAPFIX1 recommended capture moment sequence");
        assert_eq!(
            moments,
            vec![
                RESTORE_CAPTURE_MOMENT.to_string(),
                TERRAIN_CAPTURE_MOMENT.to_string(),
                WATER_CAPTURE_MOMENT.to_string(),
                RIVERS_CAPTURE_MOMENT.to_string(),
                MAPNAME_CAPTURE_MOMENT.to_string(),
                BORDERS_CAPTURE_MOMENT.to_string(),
                UNDERWATER_CAPTURE_MOMENT.to_string(),
            ]
        );
        for moment in &moments {
            validate_capture_moment(moment).expect("every recommended moment is grounded");
        }
    }

    #[test]
    fn terrain_capture_control_can_request_restore_pass_moment() {
        let moments = capture_moments_from_args(&[
            "--capture-moments".to_string(),
            "terrain_active_pass,restore_pass,ui_pass".to_string(),
        ])
        .expect("parse restore capture moment sequence");

        assert_eq!(
            moments,
            vec![
                TERRAIN_CAPTURE_MOMENT.to_string(),
                RESTORE_CAPTURE_MOMENT.to_string(),
                UI_PASS_CAPTURE_MOMENT.to_string()
            ]
        );
        let json = terrain_capture_control_json(
            &moments,
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_moment\": \"terrain_active_pass\""));
        assert!(json.contains(
            "\"capture_moments\": [\"terrain_active_pass\",\"restore_pass\",\"ui_pass\"]"
        ));
    }

    #[test]
    fn terrain_capture_control_records_output_dir_and_writer_queue_cap() {
        let json = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            Some(Path::new(r"C:\capture fast\terrain")),
            Some(123_456),
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_out_dir\": \"C:\\\\capture fast\\\\terrain\""));
        assert!(json.contains("\"max_payload_writer_queue_bytes\": 123456"));
    }

    // CAPFIX3 — --max-draws-per-pass override is written into the control file;
    // absent => the moment-appropriate default (128 for map moments).
    #[test]
    fn terrain_capture_control_honors_max_draws_per_pass_override() {
        let default_json = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None, // max_draws_per_pass absent
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(default_json.contains("\"max_draws_per_pass\": 128"));

        let override_json = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            Some(256), // max_draws_per_pass override
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(override_json.contains("\"max_draws_per_pass\": 256"));
    }

    // CAPFIX3 — --arm-delay-ms is emitted only when positive; absent/0 leaves the
    // field out so the DLL serde default (arm immediately) applies.
    #[test]
    fn terrain_capture_control_emits_arm_delay_ms_only_when_positive() {
        let with_delay = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            Some(2_500), // arm_delay_ms
        );
        assert!(with_delay.contains("\"arm_delay_ms\": 2500"));

        let without_delay = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            false,
            None,
            true,
            None,
            None, // arm_delay_ms absent
        );
        assert!(!without_delay.contains("arm_delay_ms"));
    }

    // CAPFIX3 — --max-draws-per-pass parse: valid value threads through, 0 is
    // rejected loudly, absent is None.
    #[test]
    fn parse_max_draws_per_pass_accepts_positive_and_rejects_zero() {
        let none_args: Vec<String> = vec!["--dll".into(), "x.dll".into()];
        assert_eq!(parse_max_draws_per_pass_arg(&none_args).unwrap(), None);

        let ok_args: Vec<String> = vec!["--max-draws-per-pass".into(), "64".into()];
        assert_eq!(parse_max_draws_per_pass_arg(&ok_args).unwrap(), Some(64));

        let zero_args: Vec<String> = vec!["--max-draws-per-pass".into(), "0".into()];
        let err = parse_max_draws_per_pass_arg(&zero_args)
            .expect_err("0 must be rejected")
            .to_string();
        assert!(err.contains("--max-draws-per-pass must be >= 1"));

        let bad_args: Vec<String> = vec!["--max-draws-per-pass".into(), "not-a-number".into()];
        assert!(parse_max_draws_per_pass_arg(&bad_args).is_err());
    }

    // CAPFIX3 — --arm-delay-ms parse mirrors the cb0-sample-ms pattern: positive
    // kept, 0 filtered to None (arm immediately), absent None.
    #[test]
    fn arm_delay_ms_arg_parses_positive_and_filters_zero() {
        // Mirror main()'s parse expression so the wiring stays covered.
        let parse = |args: &[String]| -> Option<u64> {
            value_after(args, "--arm-delay-ms")
                .map(|value| value.parse::<u64>())
                .transpose()
                .ok()
                .flatten()
                .filter(|delay| *delay > 0)
        };
        assert_eq!(parse(&["--dll".into(), "x".into()]), None);
        assert_eq!(parse(&["--arm-delay-ms".into(), "3000".into()]), Some(3000));
        assert_eq!(parse(&["--arm-delay-ms".into(), "0".into()]), None);
    }

    #[test]
    fn terrain_capture_control_records_rounds_and_trees_close_mode_when_enabled() {
        let json = terrain_capture_control_json(
            &[TREES_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            false,
            true,
            Some("frame-repeat"),
            true,
            None,
            None,
        );
        assert!(json.contains("\"round_commands_enabled\": true"));
        assert!(json.contains("\"trees_close_mode\": \"frame-repeat\""));
    }

    #[test]
    fn terrain_capture_control_records_census_when_enabled() {
        let json = terrain_capture_control_json(
            &[TERRAIN_CAPTURE_MOMENT.to_string()],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            true,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"census\": true"));
        assert!(json.contains("\"round_commands_enabled\": false"));
    }

    #[test]
    fn terrain_capture_control_can_request_census_without_map_frame_capture() {
        let json = terrain_capture_control_json(
            &[],
            0,
            0,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &[],
            8_000,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "hazard",
            true,
            false,
            None,
            true,
            None,
            None,
        );
        assert!(json.contains("\"capture_moment\": \"\""));
        assert!(json.contains("\"capture_moments\": []"));
        assert!(json.contains("\"map_frame_capture_enabled\": false"));
        assert!(json.contains("\"census\": true"));
    }

    #[test]
    fn capture_moment_validation_accepts_only_grounded_map_passes() {
        for moment in [
            TERRAIN_CAPTURE_MOMENT,
            TERRAIN_CHUNK_LIBRARY_MOMENT,
            WATER_CAPTURE_MOMENT,
            RIVERS_CAPTURE_MOMENT,
            BORDERS_CAPTURE_MOMENT,
            UNDERWATER_CAPTURE_MOMENT,
            TREES_CAPTURE_MOMENT,
            RESTORE_CAPTURE_MOMENT,
            MAPNAME_CAPTURE_MOMENT,
            UI_PASS_CAPTURE_MOMENT,
        ] {
            validate_capture_moment(moment).expect("grounded capture moment");
        }
        assert!(validate_capture_moment("cities_pass").is_err());
        assert!(validate_capture_moment("overlays_pass").is_err());
        let duplicate = capture_moments_from_args(&[
            "--capture-moments".to_string(),
            "terrain_active_pass,terrain_active_pass".to_string(),
        ])
        .expect_err("duplicate capture moments must fail before injection");
        assert!(duplicate.to_string().contains("duplicate capture moment"));
    }

    #[test]
    fn ui_cb_skip_validation_accepts_only_known_policies() {
        for value in ["hazard", "all", "none"] {
            validate_ui_cb_skip(value).expect("valid ui cb skip policy");
        }
        assert!(validate_ui_cb_skip("metadata").is_err());
        assert!(validate_ui_cb_skip("").is_err());
    }

    #[test]
    fn launch_command_line_quotes_executable_without_touching_passthrough_args() {
        let command_line =
            command_line_for_launch(Path::new(r"C:\Games\the game\hoi4.exe"), "-debug");
        assert_eq!(
            command_line,
            r#""C:\Games\the game\hoi4.exe" -debug"#
        );

        let bare = command_line_for_launch(Path::new(r"C:\Games\hoi4.exe"), "");
        assert_eq!(bare, r"C:\Games\hoi4.exe");
    }

    #[test]
    fn runtime_dll_copy_path_uses_unique_neighbor_name() {
        let staged =
            runtime_dll_copy_path(Path::new(r"C:\build\terrain_capture.dll"), 12345).unwrap();
        assert_eq!(
            staged,
            PathBuf::from(format!(
                r"C:\build\terrain_capture_run_{}_12345.dll",
                std::process::id()
            ))
        );
    }

    #[test]
    fn quote_windows_arg_escapes_embedded_quotes_and_terminal_backslashes() {
        assert_eq!(
            quote_windows_arg(r#"C:\plain\hoi4.exe"#),
            r#"C:\plain\hoi4.exe"#
        );
        assert_eq!(
            quote_windows_arg(r#"C:\path with spaces\"#),
            r#""C:\path with spaces\\""#,
        );
        assert_eq!(quote_windows_arg(r#"a"b"#), r#""a\"b""#);
    }
}
