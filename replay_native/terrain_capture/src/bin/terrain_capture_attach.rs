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

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{
    CloseHandle, FreeLibrary, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM,
};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CreateRemoteThread, GetExitCodeThread, ResumeThread, TerminateProcess,
    WaitForSingleObject, CREATE_SUSPENDED, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, HOOKPROC, WH_GETMESSAGE, WM_NULL,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CONTROL_FILE: &str = "terrain_capture_control.json";
const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
const WATER_CAPTURE_MOMENT: &str = "water_pass";
const RIVERS_CAPTURE_MOMENT: &str = "rivers_pass";
const BORDERS_CAPTURE_MOMENT: &str = "borders_pass";
const TREES_CAPTURE_MOMENT: &str = "trees_pass";

fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    let source_dll = PathBuf::from(required_value(&args, "--dll")?);
    let dll = stage_runtime_dll_copy(&source_dll)?;
    let dwell_ms = value_after(&args, "--dwell-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(30_000);
    let pre_resume_timeout_ms = value_after(&args, "--pre-resume-timeout-ms")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(30_000);
    let capture_moments = capture_moments_from_args(&args)?;
    if let Some(shader_cache) = value_after(&args, "--shader-cache") {
        write_shader_cache_control(&dll, &PathBuf::from(shader_cache), &capture_moments)?;
    } else if !capture_moments.is_empty() {
        write_capture_moment_control(&dll, &capture_moments)?;
    }
    if let Some(exe) = value_after(&args, "--exe") {
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
        )?;
    } else {
        let pid = required_value(&args, "--pid")?.parse::<u32>()?;
        attach_terrain_capture(pid, &dll, dwell_ms)?;
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
) -> Result<()> {
    if !exe.is_file() {
        return Err(format!("the game executable does not exist: {}", exe.display()).into());
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
    clear_capture_event_log(dll)?;

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
    let remote_dll = inject_dll_by_load_library(process_handles.process, dll)?;
    if let Err(error) = wait_for_capture_hooks_installed(dll, pre_resume_timeout_ms) {
        unsafe {
            TerminateProcess(process_handles.process, 1);
        }
        return Err(format!(
            "terrain capture DLL loaded into pid {}, but hooks were not installed before resume: {}",
            process_handles.process_id, error
        )
        .into());
    }
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
        "launched the game pid {} suspended, injected {}, observed hooks installed, resumed primary thread; remote module handle 0x{:X}",
        process_handles.process_id,
        dll.display(),
        remote_dll
    );
    thread::sleep(Duration::from_millis(dwell_ms));
    println!(
        "early terrain capture dwell complete for pid {}; check terrain_capture\\terrain_capture_events.jsonl beside the DLL",
        process_handles.process_id
    );
    Ok(())
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

fn clear_capture_event_log(dll: &Path) -> Result<()> {
    let Some(module_dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    let events = module_dir
        .join("terrain_capture")
        .join("terrain_capture_events.jsonl");
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

fn wait_for_capture_hooks_installed(dll: &Path, timeout_ms: u64) -> Result<()> {
    let Some(module_dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    let events = module_dir
        .join("terrain_capture")
        .join("terrain_capture_events.jsonl");
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    loop {
        if let Ok(text) = fs::read_to_string(&events) {
            if text.contains("\"terrain_capture_hooks_installed\"") {
                return Ok(());
            }
            if text.contains("\"terrain_capture_blocked\"") {
                return Err(format!(
                    "capture worker reported blocked before hook install: {}",
                    last_nonempty_line(&text).unwrap_or("<no event line>")
                )
                .into());
            }
        }
        if start.elapsed() >= timeout {
            return Err(format!(
                "timed out after {timeout_ms}ms waiting for {}",
                events.display()
            )
            .into());
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

fn attach_terrain_capture(pid: u32, dll: &PathBuf, dwell_ms: u64) -> Result<()> {
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
        "terrain capture DLL attached to pid {pid} on thread {thread_id}; check terrain_capture_probe.jsonl beside the DLL"
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

fn write_shader_cache_control(
    dll: &Path,
    shader_cache: &Path,
    capture_moments: &[String],
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
    );
    let Some(dir) = dll.parent() else {
        return Err("terrain capture DLL path has no parent directory".into());
    };
    fs::write(dir.join(CONTROL_FILE), control)?;
    Ok(())
}

fn write_capture_moment_control(dll: &Path, capture_moments: &[String]) -> Result<()> {
    let capture_moments = normalized_capture_moments(capture_moments);
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
        ),
    )?;
    Ok(())
}

fn validate_capture_moment(capture_moment: &str) -> Result<()> {
    match capture_moment {
        TERRAIN_CAPTURE_MOMENT
        | WATER_CAPTURE_MOMENT
        | RIVERS_CAPTURE_MOMENT
        | BORDERS_CAPTURE_MOMENT
        | TREES_CAPTURE_MOMENT => Ok(()),
        other => Err(format!("unsupported capture moment {other}").into()),
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
    if capture_moments.is_empty() {
        vec![TERRAIN_CAPTURE_MOMENT.to_string()]
    } else {
        capture_moments.to_vec()
    }
}

fn terrain_capture_control_json(
    capture_moments: &[String],
    width: u32,
    height: u32,
    slots: &BTreeSet<u32>,
    paired_resource_slots: &BTreeSet<(u32, String, u32)>,
    shader_cache_signatures: &[ShaderCacheControlEntry],
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
    let capture_moments = normalized_capture_moments(capture_moments);
    let capture_moment = capture_moments
        .first()
        .map(String::as_str)
        .unwrap_or(TERRAIN_CAPTURE_MOMENT);
    let capture_moments_json = capture_moments
        .iter()
        .map(|moment| format!("\"{}\"", json_escape(moment)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        concat!(
            "{{\n",
            "  \"schema\": \"battle-plan-terrain-capture-control-v1\",\n",
            "  \"capture_moment\": \"{}\",\n",
            "  \"capture_moments\": [{}],\n",
            "  \"expected_width\": {},\n",
            "  \"expected_height\": {},\n",
            "  \"allowed_slots\": [{}],\n",
            "  \"paired_resource_slots\": [{}],\n",
            "  \"shader_cache_signatures\": [{}],\n",
            "  \"provenance\": \"shader_cache_scan_TerrainIDMap_Texture_register_tN\"\n",
            "}}\n"
        ),
        json_escape(capture_moment),
        capture_moments_json,
        width,
        height,
        allowed_slots,
        paired_slots,
        shader_cache_signatures
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
        "terrain_capture_attach --pid <the game pid> --dll <terrain_capture.dll> [--shader-cache <dir>] [--capture-moment terrain_active_pass|water_pass|rivers_pass|borders_pass|trees_pass] [--capture-moments terrain_active_pass,water_pass,rivers_pass,borders_pass,trees_pass] [--dwell-ms <ms, default 30000>]\n\
         terrain_capture_attach --exe <hoi4.exe> --dll <terrain_capture.dll> [--working-dir <game dir>] [--args <quoted args>] [--shader-cache <dir>] [--capture-moment terrain_active_pass|water_pass|rivers_pass|borders_pass|trees_pass] [--capture-moments terrain_active_pass,water_pass,rivers_pass,borders_pass,trees_pass] [--pre-resume-timeout-ms <ms, default 30000>] [--dwell-ms <ms, default 30000>]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
        );
        assert!(json.contains("\"capture_moment\": \"terrain_active_pass\""));
        assert!(json.contains("\"capture_moments\": [\"terrain_active_pass\"]"));
        assert!(json.contains("\"allowed_slots\": [3,5]"));
        assert!(json.contains(
            "\"paired_resource_slots\": [{\"terrain_id_srv_slot\":5,\"symbol\":\"SnowMudData\",\"shader_resource_slot\":7}]"
        ));
        assert!(json.contains("\"fow_pow2_y_milli\":250"));
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
        );
        assert!(json.contains("\"capture_moment\": \"water_pass\""));
        assert!(json.contains("\"allowed_slots\": []"));
        assert!(json.contains("\"shader_cache_signatures\": []"));
    }

    #[test]
    fn terrain_capture_control_can_request_map_session_moments() {
        let moments = capture_moments_from_args(&[
            "--capture-moments".to_string(),
            "terrain_active_pass,water_pass,rivers_pass,borders_pass,trees_pass".to_string(),
        ])
        .expect("parse capture moment sequence");

        assert_eq!(
            moments,
            vec![
                TERRAIN_CAPTURE_MOMENT.to_string(),
                WATER_CAPTURE_MOMENT.to_string(),
                RIVERS_CAPTURE_MOMENT.to_string(),
                BORDERS_CAPTURE_MOMENT.to_string(),
                TREES_CAPTURE_MOMENT.to_string()
            ]
        );
        let json =
            terrain_capture_control_json(&moments, 0, 0, &BTreeSet::new(), &BTreeSet::new(), &[]);
        assert!(json.contains("\"capture_moment\": \"terrain_active_pass\""));
        assert!(json.contains(
            "\"capture_moments\": [\"terrain_active_pass\",\"water_pass\",\"rivers_pass\",\"borders_pass\",\"trees_pass\"]"
        ));
    }

    #[test]
    fn capture_moment_validation_accepts_only_grounded_map_passes() {
        for moment in [
            TERRAIN_CAPTURE_MOMENT,
            WATER_CAPTURE_MOMENT,
            RIVERS_CAPTURE_MOMENT,
            BORDERS_CAPTURE_MOMENT,
            TREES_CAPTURE_MOMENT,
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
