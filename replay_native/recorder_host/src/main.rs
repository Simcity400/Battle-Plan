use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{c_char, c_void, OsStr};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Bool = i32;
type Dword = u32;
type Hwnd = *mut c_void;
type Hmodule = *mut c_void;
type Hhook = *mut c_void;
type Handle = *mut c_void;
type Lparam = isize;
type Wparam = usize;

const WH_GETMESSAGE: i32 = 3;
const WM_NULL: u32 = 0x0000;
const PIPE_ACCESS_INBOUND: Dword = 0x00000001;
const PIPE_TYPE_BYTE: Dword = 0x00000000;
const PIPE_READMODE_BYTE: Dword = 0x00000000;
const PIPE_WAIT: Dword = 0x00000000;
const ERROR_PIPE_CONNECTED: Dword = 535;
const GENERIC_READ_BUFFER: usize = 65_536;
const INVALID_HANDLE_VALUE: isize = -1;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(path: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const c_char) -> *const c_void;
    fn FreeLibrary(module: Hmodule) -> Bool;
    fn CloseHandle(handle: Handle) -> Bool;
    fn GetLastError() -> Dword;
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: Dword,
        pipe_mode: Dword,
        max_instances: Dword,
        out_buffer_size: Dword,
        in_buffer_size: Dword,
        default_timeout: Dword,
        security_attributes: *mut c_void,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> Bool;
    fn ReadFile(
        file: Handle,
        buffer: *mut c_void,
        bytes_to_read: Dword,
        bytes_read: *mut Dword,
        overlapped: *mut c_void,
    ) -> Bool;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(callback: extern "system" fn(Hwnd, Lparam) -> Bool, lparam: Lparam) -> Bool;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut Dword) -> Dword;
    fn IsWindowVisible(hwnd: Hwnd) -> Bool;
    fn SetWindowsHookExW(
        id_hook: i32,
        proc: *const c_void,
        module: Hmodule,
        thread_id: Dword,
    ) -> Hhook;
    fn UnhookWindowsHookEx(hook: Hhook) -> Bool;
    fn PostThreadMessageW(thread_id: Dword, msg: Dword, wparam: Wparam, lparam: Lparam) -> Bool;
}

#[derive(Debug)]
struct Config {
    pid: Dword,
    dll: PathBuf,
    out_dir: PathBuf,
    duration_ms: u32,
    sample_ms: u32,
    dwell_ms: u64,
    min_game_hours: i32,
    scheduled_domains: Vec<String>,
    lightweight_root_frame: bool,
    special_projects_scan: bool,
    compact_country_frame: bool,
    idea_group_filter: Option<String>,
    advisor_token_filter: Option<String>,
    session_id: String,
}

#[derive(Debug, Default)]
struct Summary {
    frame_count: usize,
    collector_started_count: usize,
    raw_event_count: usize,
    raw_delta_count: usize,
    domains_seen: BTreeSet<String>,
    raw_domains_seen: BTreeSet<String>,
    scheduled_domain_frames: BTreeMap<String, usize>,
    unique_hours: Vec<i32>,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
    skipped_hours: i32,
    parse_errors: Vec<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("collector_record: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cfg = parse_args(env::args().skip(1).collect())?;
    fs::create_dir_all(&cfg.out_dir).map_err(|e| format!("create out dir failed: {e}"))?;
    let raw_path = cfg.out_dir.join("collector_frames.jsonl");
    let manifest_path = cfg.out_dir.join("native_session_manifest.json");
    let pipe_token = pipe_token();
    write_control_file(&cfg, &pipe_token)?;

    let hook_hold_ms = u64::from(cfg.duration_ms)
        .saturating_add(cfg.dwell_ms)
        .saturating_add(1_000);
    let pipe_timeout_ms = hook_hold_ms.saturating_add(3_000);
    let pipe_name = format!(r"\\.\pipe\battle-plan_collector_{}_{}", cfg.pid, pipe_token);
    let pipe_thread = thread::spawn(move || read_pipe_bytes(&pipe_name, pipe_timeout_ms));
    attach_hook(cfg.pid, &cfg.dll, hook_hold_ms)?;
    let pipe_bytes = pipe_thread
        .join()
        .map_err(|_| "pipe thread panicked".to_string())??;

    fs::write(&raw_path, &pipe_bytes).map_err(|e| format!("write raw stream failed: {e}"))?;
    let summary = summarize_frames(&pipe_bytes);
    let hourly_met = summary.skipped_hours == 0
        && summary
            .first_hour
            .zip(summary.last_hour)
            .map(|(first, last)| last.saturating_sub(first) >= cfg.min_game_hours)
            .unwrap_or(false);
    let manifest = json!({
        "schema": "native_collector_session_v1",
        "pid": cfg.pid,
        "dll": cfg.dll.display().to_string(),
        "raw_stream": raw_path.file_name().and_then(|v| v.to_str()).unwrap_or("collector_frames.jsonl"),
        "session_id": cfg.session_id,
        "duration_ms": cfg.duration_ms,
        "sample_ms": cfg.sample_ms,
        "dwell_ms": cfg.dwell_ms,
        "scheduled_domains": cfg.scheduled_domains,
        "lightweight_root_frame": cfg.lightweight_root_frame,
        "special_projects_scan": cfg.special_projects_scan,
        "compact_country_frame": cfg.compact_country_frame,
        "idea_group_filter_rows": encoded_row_count(cfg.idea_group_filter.as_deref()),
        "advisor_token_filter_rows": encoded_row_count(cfg.advisor_token_filter.as_deref()),
        "frame_count": summary.frame_count,
        "collector_started_count": summary.collector_started_count,
        "raw_event_count": summary.raw_event_count,
        "raw_delta_count": summary.raw_delta_count,
        "domains_seen": summary.domains_seen.iter().collect::<Vec<_>>(),
        "raw_domains_seen": summary.raw_domains_seen.iter().collect::<Vec<_>>(),
        "scheduled_domain_frames": summary.scheduled_domain_frames,
        "first_game_hour_raw": summary.first_hour,
        "last_game_hour_raw": summary.last_hour,
        "unique_game_hour_count": summary.unique_hours.len(),
        "game_hours_advanced": summary.first_hour.zip(summary.last_hour).map(|(first, last)| last.saturating_sub(first)),
        "skipped_game_hours": summary.skipped_hours,
        "hourly_requirement": {
            "required": true,
            "requested_game_hours": cfg.min_game_hours,
            "met": hourly_met,
            "problems": if hourly_met { Vec::<String>::new() } else { vec!["hourly_requirement_not_met".to_string()] },
        },
        "parse_errors": summary.parse_errors,
        "production_path_note": "native host captured DLL pipe frames without Python in the live loop; replay container packing is still follow-up",
    });
    let pretty = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("serialize manifest failed: {e}"))?;
    fs::write(&manifest_path, pretty).map_err(|e| format!("write manifest failed: {e}"))?;
    if !hourly_met {
        return Err(format!(
            "hourly requirement not met; manifest written to {}",
            manifest_path.display()
        ));
    }
    println!("{}", manifest_path.display());
    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<Config, String> {
    let mut pid = None;
    let mut dll = None;
    let mut out_dir = None;
    let mut duration_ms = 180_000u32;
    let mut sample_ms = 10u32;
    let mut dwell_ms = 250u64;
    let mut min_game_hours = 12i32;
    let mut scheduled_domains = Vec::new();
    let mut lightweight_root_frame = false;
    let mut special_projects_scan = false;
    let mut compact_country_frame = false;
    let mut idea_group_filter = None;
    let mut advisor_token_filter = None;
    let mut session_id = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--pid" => {
                i += 1;
                pid = Some(parse_u32(args.get(i), "--pid")?);
            }
            "--dll" => {
                i += 1;
                dll = Some(PathBuf::from(required(args.get(i), "--dll")?));
            }
            "--out-dir" => {
                i += 1;
                out_dir = Some(PathBuf::from(required(args.get(i), "--out-dir")?));
            }
            "--duration-s" => {
                i += 1;
                duration_ms = (parse_f64(args.get(i), "--duration-s")? * 1000.0) as u32;
            }
            "--sample-ms" => {
                i += 1;
                sample_ms = parse_u32(args.get(i), "--sample-ms")?;
            }
            "--dwell-s" => {
                i += 1;
                dwell_ms = (parse_f64(args.get(i), "--dwell-s")? * 1000.0) as u64;
            }
            "--min-game-hours" => {
                i += 1;
                min_game_hours = parse_i32(args.get(i), "--min-game-hours")?;
            }
            "--scheduled-domain" => {
                i += 1;
                let domain = required(args.get(i), "--scheduled-domain")?;
                validate_domain(&domain)?;
                if !scheduled_domains.contains(&domain) {
                    scheduled_domains.push(domain);
                }
            }
            "--lightweight-root-frame" => lightweight_root_frame = true,
            "--special-projects-scan" => special_projects_scan = true,
            "--compact-country-frame" => compact_country_frame = true,
            "--idea-group-filter-file" => {
                i += 1;
                idea_group_filter =
                    Some(read_filter_file(args.get(i), "--idea-group-filter-file")?);
            }
            "--advisor-token-filter-file" => {
                i += 1;
                advisor_token_filter = Some(read_filter_file(
                    args.get(i),
                    "--advisor-token-filter-file",
                )?);
            }
            "--session-id" => {
                i += 1;
                session_id = Some(required(args.get(i), "--session-id")?);
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
        i += 1;
    }
    let dll = dll.ok_or_else(usage)?;
    let dll = fs::canonicalize(&dll).map_err(|e| format!("dll not found: {e}"))?;
    Ok(Config {
        pid: pid.ok_or_else(usage)?,
        dll,
        out_dir: out_dir.ok_or_else(usage)?,
        duration_ms,
        sample_ms,
        dwell_ms,
        min_game_hours,
        scheduled_domains,
        lightweight_root_frame,
        special_projects_scan,
        compact_country_frame,
        idea_group_filter,
        advisor_token_filter,
        session_id: session_id.unwrap_or_else(|| format!("native_{}", pipe_token())),
    })
}

fn usage() -> String {
    "usage: collector_record --pid <pid> --dll <collector.dll> --out-dir <dir> [--duration-s N] [--sample-ms N] [--min-game-hours N] [--scheduled-domain name] [--lightweight-root-frame] [--special-projects-scan] [--compact-country-frame] [--idea-group-filter-file path] [--advisor-token-filter-file path]".to_string()
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_u32(value: Option<&String>, flag: &str) -> Result<u32, String> {
    required(value, flag)?
        .parse::<u32>()
        .map_err(|e| format!("invalid {flag}: {e}"))
}

fn parse_i32(value: Option<&String>, flag: &str) -> Result<i32, String> {
    required(value, flag)?
        .parse::<i32>()
        .map_err(|e| format!("invalid {flag}: {e}"))
}

fn parse_f64(value: Option<&String>, flag: &str) -> Result<f64, String> {
    let parsed = required(value, flag)?
        .parse::<f64>()
        .map_err(|e| format!("invalid {flag}: {e}"))?;
    if parsed.is_finite() && parsed >= 0.0 {
        Ok(parsed)
    } else {
        Err(format!(
            "invalid {flag}: value must be finite and nonnegative"
        ))
    }
}

fn validate_domain(domain: &str) -> Result<(), String> {
    let mut chars = domain.chars();
    let Some(first) = chars.next() else {
        return Err("scheduled domain must not be empty".to_string());
    };
    if !(first.is_ascii_lowercase() || first == '_') {
        return Err(format!("invalid scheduled domain {domain:?}"));
    }
    if chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        Ok(())
    } else {
        Err(format!("invalid scheduled domain {domain:?}"))
    }
}

fn read_filter_file(value: Option<&String>, flag: &str) -> Result<String, String> {
    let path = PathBuf::from(required(value, flag)?);
    let text = fs::read_to_string(&path).map_err(|e| format!("read {flag} failed: {e}"))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Err(format!("{flag} must not be empty"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn encoded_row_count(value: Option<&str>) -> usize {
    value
        .map(|text| text.split(';').filter(|row| !row.is_empty()).count())
        .unwrap_or(0)
}

fn pipe_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_nanos();
    format!("{:x}_{:x}", std::process::id(), nanos)
}

fn write_control_file(cfg: &Config, pipe_token: &str) -> Result<(), String> {
    let control_path = cfg
        .dll
        .parent()
        .ok_or_else(|| "dll has no parent directory".to_string())?
        .join("collector_control.json");
    let scheduled_domains = if cfg.scheduled_domains.is_empty() {
        None
    } else {
        Some(cfg.scheduled_domains.join(";"))
    };
    let payload = json!({
        "mode": "hourly_poll",
        "duration_ms": cfg.duration_ms,
        "pipe_token": pipe_token,
        "sample_ms": cfg.sample_ms,
        "country_object_scan": false,
        "special_projects_scan": cfg.special_projects_scan,
        "lightweight_root_frame": cfg.lightweight_root_frame,
        "compact_country_frame": cfg.compact_country_frame,
        "scheduled_snapshot_step_hours": 1,
        "sidecar_log": false,
        "session_id": cfg.session_id,
        "scheduled_domains": scheduled_domains,
        "idea_group_filter": cfg.idea_group_filter.as_deref(),
        "advisor_token_filter": cfg.advisor_token_filter.as_deref(),
    });
    let mut bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("serialize collector control failed: {e}"))?;
    bytes.push(b'\n');
    fs::write(&control_path, bytes).map_err(|e| format!("write control failed: {e}"))
}

fn attach_hook(pid: Dword, dll: &Path, hold_ms: u64) -> Result<(), String> {
    let hwnd = find_main_window(pid).ok_or_else(|| format!("no visible window for pid {pid}"))?;
    let mut owner = 0u32;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
    if thread_id == 0 || owner != pid {
        return Err(format!(
            "GetWindowThreadProcessId failed or returned pid {owner}; last_error={}",
            last_error()
        ));
    }
    let dll_wide = wide(dll.as_os_str());
    let module = unsafe { LoadLibraryW(dll_wide.as_ptr()) };
    if module.is_null() {
        return Err(format!("LoadLibraryW failed: {}", last_error()));
    }
    let proc =
        unsafe { GetProcAddress(module, b"Hoi4ReplayGetMsgProc\0".as_ptr() as *const c_char) };
    if proc.is_null() {
        unsafe {
            FreeLibrary(module);
        }
        return Err(format!("GetProcAddress failed: {}", last_error()));
    }
    let hook = unsafe { SetWindowsHookExW(WH_GETMESSAGE, proc, module, thread_id) };
    if hook.is_null() {
        unsafe {
            FreeLibrary(module);
        }
        return Err(format!("SetWindowsHookExW failed: {}", last_error()));
    }
    unsafe {
        PostThreadMessageW(thread_id, WM_NULL, 0, 0);
    }
    thread::sleep(Duration::from_millis(hold_ms));
    unsafe {
        UnhookWindowsHookEx(hook);
        FreeLibrary(module);
    }
    Ok(())
}

fn read_pipe_bytes(pipe_name: &str, timeout_ms: u64) -> Result<Vec<u8>, String> {
    let pipe_wide = wide(OsStr::new(pipe_name));
    let handle = unsafe {
        CreateNamedPipeW(
            pipe_wide.as_ptr(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            65_536,
            65_536,
            timeout_ms.min(u64::from(u32::MAX)) as Dword,
            ptr::null_mut(),
        )
    };
    if handle as isize == INVALID_HANDLE_VALUE {
        return Err(format!("CreateNamedPipeW failed: {}", last_error()));
    }
    let connected = unsafe { ConnectNamedPipe(handle, ptr::null_mut()) };
    if connected == 0 && last_error() != ERROR_PIPE_CONNECTED {
        unsafe {
            CloseHandle(handle);
        }
        return Err(format!("ConnectNamedPipe failed: {}", last_error()));
    }
    let mut out = Vec::new();
    loop {
        let mut buf = [0u8; GENERIC_READ_BUFFER];
        let mut got = 0u32;
        let ok = unsafe {
            ReadFile(
                handle,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as Dword,
                &mut got,
                ptr::null_mut(),
            )
        };
        if ok == 0 || got == 0 {
            break;
        }
        out.extend_from_slice(&buf[..got as usize]);
    }
    unsafe {
        CloseHandle(handle);
    }
    if out.is_empty() {
        Err("collector pipe returned no bytes".to_string())
    } else {
        Ok(out)
    }
}

fn summarize_frames(raw: &[u8]) -> Summary {
    let mut summary = Summary::default();
    let mut hour_set = BTreeSet::new();
    for (idx, line) in raw.split(|b| *b == b'\n').enumerate() {
        let line = trim_ascii(line);
        if line.is_empty() {
            continue;
        }
        summary.frame_count += 1;
        let value: Value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(err) => {
                summary
                    .parse_errors
                    .push(format!("line {}: {}", idx + 1, err));
                continue;
            }
        };
        let frame_type = value.get("type").and_then(Value::as_str);
        if frame_type == Some("collector_started") {
            summary.collector_started_count += 1;
            add_domains(value.get("domains"), &mut summary.domains_seen);
            if let Some(root) = value.get("root_probe") {
                add_domains(root.get("domains"), &mut summary.domains_seen);
                if let Some(raw_hour) = root.get("current_game_date_raw").and_then(Value::as_i64) {
                    if raw_hour >= i64::from(i32::MIN) && raw_hour <= i64::from(i32::MAX) {
                        hour_set.insert(raw_hour as i32);
                    }
                }
            }
            if value.get("capture_boundary").and_then(Value::as_str)
                == Some("scheduled_hour_snapshot")
            {
                for domain in frame_domains(&value) {
                    *summary.scheduled_domain_frames.entry(domain).or_insert(0) += 1;
                }
            }
        } else if frame_type == Some("event") {
            summary.raw_event_count += 1;
            if let Some(domain) = value.get("domain").and_then(Value::as_str) {
                summary.raw_domains_seen.insert(domain.to_string());
            }
        } else if frame_type == Some("domain_delta") {
            summary.raw_delta_count += 1;
            if let Some(domain) = value.get("domain").and_then(Value::as_str) {
                summary.raw_domains_seen.insert(domain.to_string());
            }
        }
    }
    summary.unique_hours = hour_set.iter().copied().collect();
    summary.first_hour = summary.unique_hours.first().copied();
    summary.last_hour = summary.unique_hours.last().copied();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = bytes.len();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[start..end]
}

fn add_domains(value: Option<&Value>, out: &mut BTreeSet<String>) {
    if let Some(items) = value.and_then(Value::as_array) {
        for item in items {
            if let Some(domain) = item.as_str() {
                out.insert(domain.to_string());
            }
        }
    }
}

fn frame_domains(value: &Value) -> Vec<String> {
    let mut domains = BTreeSet::new();
    add_domains(value.get("domains"), &mut domains);
    if let Some(root) = value.get("root_probe") {
        add_domains(root.get("domains"), &mut domains);
    }
    domains.into_iter().collect()
}

fn skipped_hours(hours: &[i32]) -> i32 {
    hours
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]).saturating_sub(1).max(0))
        .sum()
}

struct WindowSearch {
    pid: Dword,
    hwnd: Hwnd,
}

fn find_main_window(pid: Dword) -> Option<Hwnd> {
    let mut search = WindowSearch {
        pid,
        hwnd: ptr::null_mut(),
    };
    unsafe {
        EnumWindows(enum_window_proc, &mut search as *mut WindowSearch as Lparam);
    }
    if search.hwnd.is_null() {
        None
    } else {
        Some(search.hwnd)
    }
}

extern "system" fn enum_window_proc(hwnd: Hwnd, lparam: Lparam) -> Bool {
    let search = unsafe { &mut *(lparam as *mut WindowSearch) };
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }
    let mut owner = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut owner);
    }
    if owner == search.pid {
        search.hwnd = hwnd;
        0
    } else {
        1
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn last_error() -> Dword {
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_detects_contiguous_hours_and_domains() {
        let raw = br#"{"type":"collector_started","capture_boundary":"scheduled_hour_snapshot","domains":["clock"],"root_probe":{"domains":["clock","projects_live"],"current_game_date_raw":10}}
{"type":"domain_delta","domain":"land_combats"}
{"type":"collector_started","capture_boundary":"scheduled_hour_snapshot","domains":["clock"],"root_probe":{"domains":["clock","projects_live"],"current_game_date_raw":11}}
"#;
        let summary = summarize_frames(raw);
        assert_eq!(summary.frame_count, 3);
        assert_eq!(summary.collector_started_count, 2);
        assert_eq!(summary.raw_delta_count, 1);
        assert_eq!(summary.skipped_hours, 0);
        assert_eq!(summary.first_hour, Some(10));
        assert_eq!(summary.last_hour, Some(11));
        assert!(summary.domains_seen.contains("projects_live"));
        assert_eq!(
            summary
                .scheduled_domain_frames
                .get("projects_live")
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn summary_detects_skipped_hours() {
        let raw = br#"{"type":"collector_started","capture_boundary":"scheduled_hour_snapshot","root_probe":{"domains":["clock"],"current_game_date_raw":20}}
{"type":"collector_started","capture_boundary":"scheduled_hour_snapshot","root_probe":{"domains":["clock"],"current_game_date_raw":23}}
"#;
        let summary = summarize_frames(raw);
        assert_eq!(summary.unique_hours, vec![20, 23]);
        assert_eq!(summary.skipped_hours, 2);
    }

    #[test]
    fn parse_args_accepts_special_projects_scan() {
        let dir =
            std::env::temp_dir().join(format!("battle-plan-recorder_host_args_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dll = dir.join("collector.dll");
        fs::write(&dll, b"").unwrap();
        let idea_groups = dir.join("idea_groups.txt");
        let advisor_tokens = dir.join("advisor_tokens.txt");
        fs::write(&idea_groups, b"war_economy=economy\n").unwrap();
        fs::write(
            &advisor_tokens,
            b"POR_salazar=political_advisor|political|POR_salazar\n",
        )
        .unwrap();

        let cfg = parse_args(vec![
            "--pid".to_string(),
            "1234".to_string(),
            "--dll".to_string(),
            dll.display().to_string(),
            "--out-dir".to_string(),
            dir.join("out").display().to_string(),
            "--scheduled-domain".to_string(),
            "projects_live".to_string(),
            "--lightweight-root-frame".to_string(),
            "--special-projects-scan".to_string(),
            "--compact-country-frame".to_string(),
            "--idea-group-filter-file".to_string(),
            idea_groups.display().to_string(),
            "--advisor-token-filter-file".to_string(),
            advisor_tokens.display().to_string(),
        ])
        .unwrap();

        assert!(cfg.lightweight_root_frame);
        assert!(cfg.special_projects_scan);
        assert!(cfg.compact_country_frame);
        assert_eq!(cfg.scheduled_domains, vec!["projects_live".to_string()]);
        assert_eq!(
            cfg.idea_group_filter.as_deref(),
            Some("war_economy=economy")
        );
        assert_eq!(
            cfg.advisor_token_filter.as_deref(),
            Some("POR_salazar=political_advisor|political|POR_salazar")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
