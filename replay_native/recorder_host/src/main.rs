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
const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
const COMPACT_RECORD_HEADER_LEN: usize = 32;
const COMPACT_RECORD_VERSION: u16 = 1;
const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;

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
    stream_format: StreamFormat,
    extraction_perf_gate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamFormat {
    Json,
    Compact,
}

impl StreamFormat {
    fn as_str(self) -> &'static str {
        match self {
            StreamFormat::Json => "json",
            StreamFormat::Compact => "compact",
        }
    }

    fn stream_file_name(self) -> &'static str {
        match self {
            StreamFormat::Json => "collector_frames.jsonl",
            StreamFormat::Compact => "collector_stream.bin",
        }
    }
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

#[derive(Debug, Default)]
struct CompactSummary {
    record_count: usize,
    domain_perf_record_count: usize,
    frame_end_record_count: usize,
    total_bytes: usize,
    total_payload_bytes: usize,
    max_payload_len: usize,
    max_record_len: usize,
    domains_seen: BTreeSet<u16>,
    per_domain: BTreeMap<u16, CompactDomainSummary>,
    frame_times_us: Vec<u64>,
    unique_hours: Vec<i32>,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
    skipped_hours: i32,
    ordering_errors: Vec<String>,
    parse_errors: Vec<String>,
}

#[derive(Debug, Default)]
struct CompactDomainSummary {
    record_count: usize,
    row_count_total: u64,
    changed_rows_total: u64,
    unit_count_total: u64,
    changed_unit_count_total: u64,
    read_times_us: Vec<u64>,
    max_read_time_us: u64,
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
    let raw_path = cfg.out_dir.join(cfg.stream_format.stream_file_name());
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
    let (manifest, hourly_met) = match cfg.stream_format {
        StreamFormat::Json => {
            let summary = summarize_frames(&pipe_bytes);
            let hourly_met = summary.skipped_hours == 0
                && summary
                    .first_hour
                    .zip(summary.last_hour)
                    .map(|(first, last)| last.saturating_sub(first) >= cfg.min_game_hours)
                    .unwrap_or(false);
            (
                json!({
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
                    "stream_format": cfg.stream_format.as_str(),
                    "extraction_perf_gate": cfg.extraction_perf_gate,
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
                    "production_path_note": "JSON is debug/proof visibility only; production feasibility must use the compact stream gate",
                }),
                hourly_met,
            )
        }
        StreamFormat::Compact => {
            let summary = summarize_compact_stream(&pipe_bytes);
            let decodable = summary.parse_errors.is_empty();
            let ordered = summary.ordering_errors.is_empty();
            let hourly_met = decodable
                && ordered
                && summary.skipped_hours == 0
                && summary
                    .first_hour
                    .zip(summary.last_hour)
                    .map(|(first, last)| last.saturating_sub(first) >= cfg.min_game_hours)
                    .unwrap_or(false);
            (
                json!({
                    "schema": "native_collector_session_v1",
                    "pid": cfg.pid,
                    "dll": cfg.dll.display().to_string(),
                    "raw_stream": raw_path.file_name().and_then(|v| v.to_str()).unwrap_or("collector_stream.bin"),
                    "session_id": cfg.session_id,
                    "duration_ms": cfg.duration_ms,
                    "sample_ms": cfg.sample_ms,
                    "dwell_ms": cfg.dwell_ms,
                    "scheduled_domains": cfg.scheduled_domains,
                    "lightweight_root_frame": cfg.lightweight_root_frame,
                    "special_projects_scan": cfg.special_projects_scan,
                    "compact_country_frame": cfg.compact_country_frame,
                    "stream_format": cfg.stream_format.as_str(),
                    "extraction_perf_gate": cfg.extraction_perf_gate,
                    "idea_group_filter_rows": encoded_row_count(cfg.idea_group_filter.as_deref()),
                    "advisor_token_filter_rows": encoded_row_count(cfg.advisor_token_filter.as_deref()),
                    "compact_stream": {
                        "record_magic": "H4CR",
                        "record_header_len": COMPACT_RECORD_HEADER_LEN,
                        "record_version": COMPACT_RECORD_VERSION,
                        "record_count": summary.record_count,
                        "domain_perf_record_count": summary.domain_perf_record_count,
                        "frame_end_record_count": summary.frame_end_record_count,
                        "total_bytes": summary.total_bytes,
                        "total_payload_bytes": summary.total_payload_bytes,
                        "max_payload_len": summary.max_payload_len,
                        "max_record_len": summary.max_record_len,
                        "decodable": decodable,
                        "ordered": ordered,
                        "domains_seen": compact_domain_names(&summary.domains_seen),
                        "per_domain": compact_domain_summaries_json(&summary),
                        "frame_time_us": timing_summary_json(&summary.frame_times_us),
                    },
                    "first_game_hour_raw": summary.first_hour,
                    "last_game_hour_raw": summary.last_hour,
                    "unique_game_hour_count": summary.unique_hours.len(),
                    "game_hours_advanced": summary.first_hour.zip(summary.last_hour).map(|(first, last)| last.saturating_sub(first)),
                    "skipped_game_hours": summary.skipped_hours,
                    "hourly_requirement": {
                        "required": true,
                        "requested_game_hours": cfg.min_game_hours,
                        "met": hourly_met,
                        "problems": compact_hourly_problems(&summary, cfg.min_game_hours),
                    },
                    "parse_errors": summary.parse_errors,
                    "ordering_errors": summary.ordering_errors,
                    "production_path_note": "compact DLL stream gate: no full JSON root frames in the live extraction path; replay container packing is still follow-up",
                }),
                hourly_met,
            )
        }
    };
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
    let mut stream_format = StreamFormat::Json;
    let mut extraction_perf_gate = false;
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
            "--stream-format" => {
                i += 1;
                stream_format = parse_stream_format(args.get(i), "--stream-format")?;
            }
            "--extraction-perf-gate" => extraction_perf_gate = true,
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
        stream_format,
        extraction_perf_gate,
    })
}

fn usage() -> String {
    "usage: collector_record --pid <pid> --dll <collector.dll> --out-dir <dir> [--duration-s N] [--sample-ms N] [--min-game-hours N] [--scheduled-domain name] [--stream-format json|compact] [--extraction-perf-gate] [--lightweight-root-frame] [--special-projects-scan] [--compact-country-frame] [--idea-group-filter-file path] [--advisor-token-filter-file path]".to_string()
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

fn parse_stream_format(value: Option<&String>, flag: &str) -> Result<StreamFormat, String> {
    match required(value, flag)?.as_str() {
        "json" => Ok(StreamFormat::Json),
        "compact" => Ok(StreamFormat::Compact),
        other => Err(format!(
            "invalid {flag}: expected json or compact, got {other:?}"
        )),
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
        "stream_format": cfg.stream_format.as_str(),
        "extraction_perf_gate": cfg.extraction_perf_gate,
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

fn summarize_compact_stream(raw: &[u8]) -> CompactSummary {
    let mut summary = CompactSummary {
        total_bytes: raw.len(),
        ..CompactSummary::default()
    };
    let mut hour_set = BTreeSet::new();
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: truncated header with {remaining} byte(s) remaining"
            ));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        if version != COMPACT_RECORD_VERSION {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported record version {version}"
            ));
            break;
        }
        let header_len = read_u16_le(header, 6) as usize;
        if header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported header length {header_len}"
            ));
            break;
        }
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];

        summary.record_count += 1;
        summary.total_payload_bytes = summary.total_payload_bytes.saturating_add(payload_len);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);
        summary.max_record_len = summary.max_record_len.max(record_len);
        summary.domains_seen.insert(domain_id);
        match record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                summary.domain_perf_record_count += 1;
                if payload.len() < 40 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: domain perf payload is {} byte(s), expected at least 40",
                        payload.len()
                    ));
                } else {
                    let read_time_us = read_u64_le(payload, 8);
                    let row_count = read_u32_le(payload, 16) as u64;
                    let changed_rows = read_u32_le(payload, 20) as u64;
                    let unit_count = read_u32_le(payload, 24) as u64;
                    let changed_unit_count = read_u32_le(payload, 28) as u64;
                    let domain = summary.per_domain.entry(domain_id).or_default();
                    domain.record_count += 1;
                    domain.row_count_total = domain.row_count_total.saturating_add(row_count);
                    domain.changed_rows_total =
                        domain.changed_rows_total.saturating_add(changed_rows);
                    domain.unit_count_total = domain.unit_count_total.saturating_add(unit_count);
                    domain.changed_unit_count_total = domain
                        .changed_unit_count_total
                        .saturating_add(changed_unit_count);
                    domain.max_read_time_us = domain.max_read_time_us.max(read_time_us);
                    domain.read_times_us.push(read_time_us);
                }
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count += 1;
                hour_set.insert(game_hour_raw);
                if payload.len() < 32 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end payload is {} byte(s), expected at least 32",
                        payload.len()
                    ));
                } else {
                    summary.frame_times_us.push(read_u64_le(payload, 8));
                }
            }
            other => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: unknown record type {other}"));
            }
        }
        offset = offset.saturating_add(record_len);
    }
    summary.unique_hours = hour_set.iter().copied().collect();
    summary.first_hour = summary.unique_hours.first().copied();
    summary.last_hour = summary.unique_hours.last().copied();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn compact_domain_names(domains: &BTreeSet<u16>) -> Vec<String> {
    domains
        .iter()
        .map(|domain| compact_domain_name(*domain).to_string())
        .collect()
}

fn compact_domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        1 => "clock",
        10 => "fuel_live",
        11 => "production_live",
        12 => "focus_live",
        13 => "tech_live",
        14 => "politics_live",
        15 => "politics_residuals",
        16 => "decisions_live",
        17 => "diplomacy_live",
        1000 => "frame",
        _ => "unknown",
    }
}

fn compact_domain_summaries_json(summary: &CompactSummary) -> Value {
    let mut out = serde_json::Map::new();
    for (domain_id, domain) in &summary.per_domain {
        out.insert(
            compact_domain_name(*domain_id).to_string(),
            json!({
                "domain_id": domain_id,
                "record_count": domain.record_count,
                "row_count_total": domain.row_count_total,
                "changed_rows_total": domain.changed_rows_total,
                "unit_count_total": domain.unit_count_total,
                "changed_unit_count_total": domain.changed_unit_count_total,
                "read_time_us": timing_summary_json(&domain.read_times_us),
            }),
        );
    }
    Value::Object(out)
}

fn timing_summary_json(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({
            "count": 0,
            "avg": null,
            "p95": null,
            "max": null,
        });
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let sum = sorted.iter().copied().sum::<u64>();
    let p95_index = ((sorted.len() * 95).saturating_add(99) / 100)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    json!({
        "count": sorted.len(),
        "avg": (sum as f64) / (sorted.len() as f64),
        "p95": sorted[p95_index],
        "max": sorted[sorted.len() - 1],
    })
}

fn compact_hourly_problems(summary: &CompactSummary, min_game_hours: i32) -> Vec<String> {
    let mut problems = Vec::new();
    if !summary.parse_errors.is_empty() {
        problems.push("compact_stream_not_decodable".to_string());
    }
    if !summary.ordering_errors.is_empty() {
        problems.push("compact_stream_sequence_order_failed".to_string());
    }
    if summary.skipped_hours != 0 {
        problems.push("skipped_game_hours_detected".to_string());
    }
    let advanced = summary
        .first_hour
        .zip(summary.last_hour)
        .map(|(first, last)| last.saturating_sub(first))
        .unwrap_or(0);
    if advanced < min_game_hours {
        problems.push("minimum_game_hours_not_reached".to_string());
    }
    problems
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
    fn compact_summary_decodes_ordered_records_and_domain_metrics() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(1_250, 355, 12, 44, 44),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );
        append_test_compact_record(
            &mut raw,
            3,
            101,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(1_500, 355, 10, 40, 40),
        );
        append_test_compact_record(
            &mut raw,
            4,
            101,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_250, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(summary.parse_errors.is_empty());
        assert!(summary.ordering_errors.is_empty());
        assert_eq!(summary.record_count, 4);
        assert_eq!(summary.domain_perf_record_count, 2);
        assert_eq!(summary.frame_end_record_count, 2);
        assert_eq!(summary.unique_hours, vec![100, 101]);
        assert_eq!(summary.skipped_hours, 0);
        assert_eq!(summary.total_bytes, raw.len());
        let fuel = summary.per_domain.get(&10).unwrap();
        assert_eq!(fuel.record_count, 2);
        assert_eq!(fuel.row_count_total, 710);
        assert_eq!(fuel.changed_rows_total, 22);
        assert_eq!(fuel.unit_count_total, 84);
        assert_eq!(fuel.max_read_time_us, 1_500);
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
            "--stream-format".to_string(),
            "compact".to_string(),
            "--extraction-perf-gate".to_string(),
        ])
        .unwrap();

        assert!(cfg.lightweight_root_frame);
        assert!(cfg.special_projects_scan);
        assert!(cfg.compact_country_frame);
        assert_eq!(cfg.stream_format, StreamFormat::Compact);
        assert!(cfg.extraction_perf_gate);
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

    fn test_domain_payload(
        read_time_us: u64,
        row_count: u32,
        changed_rows: u32,
        unit_count: u32,
        changed_unit_count: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&read_time_us.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&unit_count.to_le_bytes());
        payload.extend_from_slice(&changed_unit_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn test_frame_payload(frame_time_us: u64, emitted_domains: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&frame_time_us.to_le_bytes());
        payload.extend_from_slice(&emitted_domains.to_le_bytes());
        payload.extend_from_slice(&emitted_domains.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn append_test_compact_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }
}
