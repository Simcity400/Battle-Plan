#![recursion_limit = "256"]

use serde_json::{json, Value};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{c_char, c_void, OsStr};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Bool = i32;
type Dword = u32;
type Hwnd = *mut c_void;
type Hmodule = *mut c_void;
type Hhook = *mut c_void;
type Handle = *mut c_void;
type Lparam = isize;
type Wparam = usize;

#[repr(C)]
struct ThreadEntry32 {
    dw_size: Dword,
    cnt_usage: Dword,
    th32_thread_id: Dword,
    th32_owner_process_id: Dword,
    tp_base_pri: i32,
    tp_delta_pri: i32,
    dw_flags: Dword,
}

const WH_GETMESSAGE: i32 = 3;
const WM_NULL: u32 = 0x0000;
const PIPE_ACCESS_INBOUND: Dword = 0x00000001;
const PIPE_TYPE_BYTE: Dword = 0x00000000;
const PIPE_READMODE_BYTE: Dword = 0x00000000;
const PIPE_NOWAIT: Dword = 0x00000001;
const ERROR_PIPE_CONNECTED: Dword = 535;
const ERROR_BROKEN_PIPE: Dword = 109;
const ERROR_NO_DATA: Dword = 232;
const ERROR_PIPE_LISTENING: Dword = 536;
const GENERIC_READ_BUFFER: usize = 65_536;
const INVALID_HANDLE_VALUE: isize = -1;
const TH32CS_SNAPTHREAD: Dword = 0x00000004;
const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
const COMPACT_RECORD_HEADER_LEN: usize = 32;
const COMPACT_RECORD_VERSION: u16 = 1;
const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const COMPACT_STATUS_WORKER_PANIC: u32 = 1 << 6;
const COMPACT_STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
const COMPACT_STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
const COMPACT_STATUS_PARTIAL_DATA: u32 = 1 << 9;
const COMPACT_STATUS_DELTA_DATA: u32 = 1 << 10;
const COMPACT_FUEL_SCALE_RAW: u32 = 32768;
const COMPACT_PRODUCTION_SCALE_RAW: u32 = 100000;
const COMPACT_TECH_PROGRESS_SCALE_RAW: u32 = 1000;
const COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW: u32 = 100000;
const COMPACT_FIXED_SCALE_RAW: u32 = 100000;
const COMPACT_DIVISION_TYPE: u32 = 4713;
const COMPACT_DIVISION_ROW_LEN: usize = 88;
const COMPACT_MAX_DIVISION_PROVINCE_ID: u32 = 1_000_000;
const COMPACT_MAX_DIVISION_COMBAT_REF_COUNT: u32 = 1_000;
const COMPACT_MAX_DIVISION_ORGANISATION_RAW: u32 = 100_000_000;
const COMPACT_MAX_DIVISION_STRENGTH_RAW: u32 = 1_000_000_000;
const COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID: u32 = 2_048;
const COMPACT_MAX_ARMY_ID: u32 = i32::MAX as u32;
const COMPACT_MAX_DIVISION_EXPERIENCE_RAW: u32 = u32::MAX;
const COMPACT_MAX_DIVISION_SUPPLY_RAW: u32 = 100_000_000;
const COMPACT_MAX_DIVISION_SUPPLY_GAIN_RAW: u32 = 500_000;
const COMPACT_MAX_DIVISION_FUEL_RAW: u32 = u32::MAX;
const COMPACT_DIVISION_ROW_FLAG_DELETED: u32 = 1 << 12;
const COMPACT_DIVISION_ROW_FLAG_TEMPLATE: u32 = 1 << 13;
const COMPACT_MAX_DIVISION_TEMPLATE_ID: u32 = i32::MAX as u32;
const COMPACT_DIVISION_EQUIPMENT_ROW_LEN: usize = 24;
const COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN: usize = 16;
const COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE: u32 = 1;
const COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED: u32 = 1 << 1;
const COMPACT_MAX_DIVISION_EQUIPMENT_VARIANT_ID: u32 = i32::MAX as u32;
const COMPACT_MAX_DIVISION_EQUIPMENT_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
const COMPACT_LAND_COMBAT_ROW_LEN: usize = 40;
const COMPACT_LAND_COMBAT_PARTICIPANT_ROW_LEN: usize = 24;
const COMPACT_LAND_COMBAT_MAX_PROVINCE_ID: u32 = 1_000_000;
const COMPACT_LAND_COMBAT_MAX_AGE_DAYS: u32 = 100_000;
const COMPACT_ARMY_ORDER_ROW_LEN: usize = 48;
const COMPACT_ARMY_ORDER_TARGET_ROW_LEN: usize = 8;
const COMPACT_ARMY_ORDER_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES: u32 = 1 << 1;
const COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
const COMPACT_ARMY_ORDER_MAX_SLOT_INDEX: u32 = 64;
const COMPACT_ARMY_ORDER_MAX_STATE_ID: u32 = 65_535;
const COMPACT_OOB_ROW_LEN: usize = 80;
const COMPACT_OOB_MEMBER_ROW_LEN: usize = 8;
const COMPACT_OOB_KIND_ARMY: u32 = 1;
const COMPACT_OOB_KIND_ARMY_GROUP: u32 = 2;
const COMPACT_OOB_KIND_FLEET: u32 = 3;
const COMPACT_OOB_KIND_TASK_FORCE: u32 = 4;
const COMPACT_OOB_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_OOB_ROW_FLAG_PARENT: u32 = 1 << 1;
const COMPACT_OOB_ROW_FLAG_OWNER: u32 = 1 << 2;
const COMPACT_OOB_ROW_FLAG_LEADER: u32 = 1 << 3;
const COMPACT_OOB_ROW_FLAG_LEADER_SKILL: u32 = 1 << 4;
const COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
const COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
const COMPACT_OOB_ROW_FLAG_LOCATION: u32 = 1 << 7;
const COMPACT_OOB_MAX_ID: u32 = 4_000_000;
const COMPACT_OOB_MAX_LOCATION_PROVINCE_ID: u32 = 1_000_000;
const COMPACT_OOB_MAX_MEMBER_ROWS: usize = 250_000;
const COMPACT_OOB_MAX_LEADER_SKILL: u32 = 15;
const COMPACT_SHIP_ROW_BASE_LEN: usize = 40;
const COMPACT_SHIP_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_SHIP_ROW_FLAG_TASK_FORCE: u32 = 1 << 1;
const COMPACT_SHIP_ROW_FLAG_DEFINITION: u32 = 1 << 2;
const COMPACT_SHIP_ROW_FLAG_OWNER: u32 = 1 << 3;
const COMPACT_SHIP_MAX_ID: u32 = 4_000_000;
const COMPACT_SHIP_MAX_OWNER_TAG_DB_ID: u32 = 2_048;
const COMPACT_SHIP_MAX_CATEGORY: u32 = 128;
const COMPACT_SHIP_MAX_STRENGTH_RAW: u32 = 1_000_000_000;
const COMPACT_SHIP_MAX_ORGANISATION_RAW: u32 = 100_000_000;
const COMPACT_SHIP_MAX_EXPERIENCE_RAW: u32 = 100_000_000;
const COMPACT_AIR_WING_ROW_LEN: usize = 40;
const COMPACT_AIR_WING_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION: u32 = 1 << 1;
const COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID: u32 = 1_024;
const COMPACT_AIR_WING_MAX_ID: u32 = i32::MAX as u32;
const COMPACT_AIR_WING_MAX_PLANES: u32 = 10_000_000;
const COMPACT_AIR_WING_MAX_MISSION_REGION_ID: u32 = 65_535;
const COMPACT_AIR_REGION_ACTIVITY_ROW_LEN: usize = 32;
const COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_AIR_REGION_ACTIVITY_MAX_REGION_ID: u32 = 65_535;
const COMPACT_AIR_REGION_ACTIVITY_MAX_OWNER_TAG_DB_ID: u32 = 1_024;
const COMPACT_AIR_REGION_ACTIVITY_MAX_WING_COUNT: u32 = 20_000;
const COMPACT_AIR_REGION_ACTIVITY_MAX_PLANES: u32 = 10_000_000;
const COMPACT_AIR_WING_LOADOUT_ROW_LEN: usize = 32;
const COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN: usize = 16;
const COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_AIR_WING_LOADOUT_MAX_AIR_BASE_ID: u32 = i32::MAX as u32;
const COMPACT_AIR_WING_LOADOUT_MAX_VARIANT_ID: u32 = i32::MAX as u32;
const COMPACT_AIR_WING_LOADOUT_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
const COMPACT_AIR_VARIANT_DEF_ROW_LEN: usize = 24;
const COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME: u32 = 1;
const COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME: u32 = 1 << 1;
const COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES: u32 = 1 << 2;
const COMPACT_AIR_VARIANT_DEF_MAX_VARIANT_ID: u32 = i32::MAX as u32;
const COMPACT_AIR_VARIANT_DEF_MAX_SLOT_HANDLE: u32 = 1_000_000;
const COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN: usize = 160;
const COMPACT_AIR_VARIANT_STATS_ROW_LEN: usize = 64;
const COMPACT_AIR_VARIANT_STATS_MAX_VARIANT_ID: u32 = i32::MAX as u32;
const COMPACT_AIR_VARIANT_STATS_MAX_RAW: i32 = 1_000_000_000;
const COMPACT_AIR_BASE_STATUS_ROW_LEN: usize = 32;
const COMPACT_AIR_BASE_STATUS_ROW_FLAG_DELETED: u32 = 1;
const COMPACT_AIR_BASE_STATUS_MAX_AIR_BASE_ID: u32 = i32::MAX as u32;
const COMPACT_AIR_BASE_STATUS_MAX_STATE_ID: u32 = 65_535;
const COMPACT_AIR_BASE_STATUS_MAX_CAPACITY: u32 = 1_000_000;
const COMPACT_AIR_BASE_STATUS_MAX_BUILDING_LEVEL: u32 = 1_000;
const COMPACT_AIR_BASE_STATUS_MAX_WING_COUNT: u32 = 20_000;
const COMPACT_AIR_BASE_STATUS_MAX_PLANES: u32 = 10_000_000;
const COMPACT_MAP_STATE_ROW_LEN: usize = 16;
const COMPACT_MAP_PROVINCE_ROW_LEN: usize = 16;
const COMPACT_MAP_DETAIL_STATE_ROW_LEN: usize = 28;
const COMPACT_MAP_DETAIL_PROVINCE_ROW_LEN: usize = 20;
const COMPACT_MAP_DETAIL_ENTRY_ROW_LEN: usize = 12;
const COMPACT_WEATHER_REGION_ROW_LEN: usize = 32;
const COMPACT_MAP_MIN_STATE_ROWS: usize = 1_000;
const COMPACT_MAP_MIN_PROVINCE_ROWS: usize = 10_000;
const COMPACT_WEATHER_REGION_ROWS: usize = 305;
const COMPACT_MAP_MAX_STATE_ID: u32 = 65_535;
const COMPACT_MAP_MAX_PROVINCE_ID: u32 = 65_536;
const COMPACT_MAP_MAX_TAG_DB_ID: u32 = 1024;
const COMPACT_MAP_MAX_RESOURCE_AMOUNT_RAW: i32 = 100_000_000;
const COMPACT_MAP_MAX_BUILDING_LEVEL: u32 = 1_000;
const COMPACT_WEATHER_MAX_REGION_ID: u32 = 304;
const COMPACT_WEATHER_MAX_PHENOMENON: u32 = 16;
const COMPACT_WEATHER_MAX_TEMPERATURE_RAW: i32 = 10_000_000;
const COMPACT_WEATHER_MAX_ACTIVE_MODIFIERS: u32 = 4_096;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(path: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const c_char) -> *const c_void;
    fn FreeLibrary(module: Hmodule) -> Bool;
    fn CloseHandle(handle: Handle) -> Bool;
    fn CreateToolhelp32Snapshot(flags: Dword, process_id: Dword) -> Handle;
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
    fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry32) -> Bool;
    fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry32) -> Bool;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(callback: extern "system" fn(Hwnd, Lparam) -> Bool, lparam: Lparam) -> Bool;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut Dword) -> Dword;
    fn IsWindowVisible(hwnd: Hwnd) -> Bool;
    fn PostMessageW(hwnd: Hwnd, msg: Dword, wparam: Wparam, lparam: Lparam) -> Bool;
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
    division_body_bridge: Option<String>,
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
    domain_data_record_count: usize,
    frame_end_record_count: usize,
    total_bytes: usize,
    total_payload_bytes: usize,
    max_payload_len: usize,
    max_record_len: usize,
    domains_seen: BTreeSet<u16>,
    per_domain: BTreeMap<u16, CompactDomainSummary>,
    per_hour_domains: BTreeMap<i32, BTreeSet<u16>>,
    frame_times_us: Vec<u64>,
    status_flags_or: u32,
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
    data_record_count: usize,
    row_count_total: u64,
    changed_rows_total: u64,
    data_rows_total: u64,
    data_changed_rows_total: u64,
    data_units_total: BTreeMap<String, u64>,
    unit_count_total: u64,
    changed_unit_count_total: u64,
    read_times_us: Vec<u64>,
    max_read_time_us: u64,
    status_flags_or: u32,
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
    let replay_path = cfg.out_dir.join("native_session.battleplan");
    let replay_game_id = replay_game_id(&cfg.session_id);
    let pipe_token = pipe_token();
    write_control_file(&cfg, &pipe_token)?;

    let hook_hold_ms = u64::from(cfg.duration_ms)
        .saturating_add(cfg.dwell_ms)
        .saturating_add(native_division_root_prewarm_budget_ms(&cfg))
        .saturating_add(1_000);
    let pipe_timeout_ms = hook_hold_ms.saturating_add(3_000);
    let pipe_name = format!(r"\\.\pipe\battle-plan_collector_{}_{}", cfg.pid, pipe_token);
    let pipe_thread = thread::spawn(move || read_pipe_bytes(&pipe_name, pipe_timeout_ms));
    attach_hook(cfg.pid, &cfg.dll, hook_hold_ms)?;
    let pipe_bytes = pipe_thread
        .join()
        .map_err(|_| "pipe thread panicked".to_string())??;

    fs::write(&raw_path, &pipe_bytes).map_err(|e| format!("write raw stream failed: {e}"))?;
    let (mut manifest, hourly_met) = match cfg.stream_format {
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
                    "division_body_bridge_rows": encoded_row_count(cfg.division_body_bridge.as_deref()),
                    "division_root_source": "not_applicable_json_debug_stream",
                    "division_root_discovery_debt": cfg.division_body_bridge.as_ref().map(|_| "temporary bounded preflight/cache input; JSON is not the production compact proof path").unwrap_or("not_applicable_json_debug_stream"),
                    "division_body_bridge_preflight_debt": cfg.division_body_bridge.as_ref().map(|_| "temporary bounded preflight/cache input; JSON is not the production compact proof path").unwrap_or("not_applicable_json_debug_stream"),
                    "ship_root_source": "not_applicable_json_debug_stream",
                    "ship_root_discovery_debt": "not_applicable_json_debug_stream",
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
            let expected_domain_ids = compact_expected_domain_ids(&cfg.scheduled_domains);
            let coverage_errors = compact_domain_coverage_errors(&summary, &expected_domain_ids);
            let decodable = summary.parse_errors.is_empty();
            let ordered = summary.ordering_errors.is_empty();
            let worker_panic_seen = (summary.status_flags_or & COMPACT_STATUS_WORKER_PANIC) != 0;
            let bridge_unavailable_seen =
                (summary.status_flags_or & COMPACT_STATUS_BRIDGE_UNAVAILABLE) != 0;
            let source_unavailable_seen =
                (summary.status_flags_or & COMPACT_STATUS_SOURCE_UNAVAILABLE) != 0;
            let partial_data_seen = (summary.status_flags_or & COMPACT_STATUS_PARTIAL_DATA) != 0;
            let division_root_source = compact_division_root_source(&cfg, &summary);
            let division_root_discovery_debt =
                compact_division_root_discovery_debt(division_root_source);
            let ship_root_source = compact_ship_root_source(&cfg, &summary);
            let ship_root_discovery_debt = compact_ship_root_discovery_debt(ship_root_source);
            let hourly_met = decodable
                && ordered
                && !worker_panic_seen
                && !bridge_unavailable_seen
                && !source_unavailable_seen
                && !partial_data_seen
                && coverage_errors.is_empty()
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
                    "division_body_bridge_rows": encoded_row_count(cfg.division_body_bridge.as_deref()),
                    "division_root_source": division_root_source,
                    "division_root_discovery_debt": division_root_discovery_debt,
                    "division_body_bridge_preflight_debt": division_root_discovery_debt,
                    "ship_root_source": ship_root_source,
                    "ship_root_discovery_debt": ship_root_discovery_debt,
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
                        "domain_data_record_count": summary.domain_data_record_count,
                        "frame_end_record_count": summary.frame_end_record_count,
                        "total_bytes": summary.total_bytes,
                        "total_payload_bytes": summary.total_payload_bytes,
                        "max_payload_len": summary.max_payload_len,
                        "max_record_len": summary.max_record_len,
                        "decodable": decodable,
                        "ordered": ordered,
                        "status_flags_or": summary.status_flags_or,
                        "worker_panic_seen": worker_panic_seen,
                        "bridge_unavailable_seen": bridge_unavailable_seen,
                        "source_unavailable_seen": source_unavailable_seen,
                        "partial_data_seen": partial_data_seen,
                        "domains_seen": compact_domain_names(&summary.domains_seen),
                        "expected_domains": compact_domain_names_from_ids(&expected_domain_ids),
                        "domain_coverage": {
                            "ok": coverage_errors.is_empty(),
                            "expected_domain_count": expected_domain_ids.len(),
                            "errors": coverage_errors.clone(),
                        },
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
                        "problems": compact_hourly_problems(&summary, cfg.min_game_hours, &coverage_errors),
                    },
                    "parse_errors": summary.parse_errors,
                    "ordering_errors": summary.ordering_errors,
                    "production_path_note": "compact DLL stream gate: no full JSON root frames in the live extraction path; the Rust recorder packs the raw compact stream into .battleplan for durability",
                }),
                hourly_met,
            )
        }
    };
    if let Some(obj) = manifest.as_object_mut() {
        obj.insert(
            "replay_file".to_string(),
            json!(replay_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("native_session.battleplan")),
        );
        obj.insert("replay_game_id".to_string(), json!(replay_game_id));
        obj.insert(
            "replay_entries".to_string(),
            json!({
                "index": "index.json",
                "timeline": "timeline.json",
                "stream": format!("collector_raw/{}", cfg.stream_format.stream_file_name()),
                "stream_role": "collector_compact_stream",
                "session_manifest": "recorder/native_session_manifest.json",
                "session_manifest_role": "recorder_manifest",
            }),
        );
    }
    let pretty = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("serialize manifest failed: {e}"))?;
    fs::write(&manifest_path, pretty).map_err(|e| format!("write manifest failed: {e}"))?;
    write_native_replay_container(&cfg, &pipe_bytes, &manifest, &manifest_path, &replay_path)?;
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
    let mut division_body_bridge = None;
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
            "--division-body-bridge" => {
                i += 1;
                division_body_bridge = Some(required(args.get(i), "--division-body-bridge")?);
            }
            "--division-body-bridge-file" => {
                i += 1;
                division_body_bridge = Some(read_filter_file(
                    args.get(i),
                    "--division-body-bridge-file",
                )?);
            }
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
        division_body_bridge,
        idea_group_filter,
        advisor_token_filter,
        session_id: session_id.unwrap_or_else(|| format!("native_{}", pipe_token())),
        stream_format,
        extraction_perf_gate,
    })
}

fn usage() -> String {
    "usage: collector_record --pid <pid> --dll <collector.dll> --out-dir <dir> [--duration-s N] [--sample-ms N] [--min-game-hours N] [--scheduled-domain name] [--stream-format json|compact] [--extraction-perf-gate] [--lightweight-root-frame] [--special-projects-scan] [--compact-country-frame] [--division-body-bridge value|--division-body-bridge-file path] [--idea-group-filter-file path] [--advisor-token-filter-file path]".to_string()
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

fn replay_game_id(session_id: &str) -> String {
    let mut out = String::with_capacity(session_id.len().max(1));
    for ch in session_id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "native_session".to_string()
    } else {
        out
    }
}

fn created_utc_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    format!("unix_seconds:{secs}")
}

fn write_native_replay_container(
    cfg: &Config,
    stream_bytes: &[u8],
    manifest: &Value,
    manifest_path: &Path,
    replay_path: &Path,
) -> Result<(), String> {
    let game_id = manifest
        .get("replay_game_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| replay_game_id(&cfg.session_id));
    let stream_name = cfg.stream_format.stream_file_name();
    let manifest_name = manifest_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("native_session_manifest.json");
    let stream_payload_path = format!("collector_raw/{stream_name}");
    let manifest_payload_path = format!("recorder/{manifest_name}");
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id.clone(),
        "schema_version": "native_collector_session_v1",
        "snapshots": [],
    }))
    .map_err(|e| format!("serialize replay index failed: {e}"))?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": cfg.stream_format.as_str(),
        "stream": stream_payload_path.clone(),
        "session_manifest": manifest_payload_path.clone(),
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest",
    }))
    .map_err(|e| format!("serialize replay timeline failed: {e}"))?;
    let manifest_bytes =
        fs::read(manifest_path).map_err(|e| format!("read session manifest failed: {e}"))?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index_bytes,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/timeline.json"),
            bytes: timeline_bytes,
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{stream_payload_path}"),
            bytes: stream_bytes.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{manifest_payload_path}"),
            bytes: manifest_bytes,
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("native_collector_session_v1".to_string()),
        created_utc: Some(created_utc_label()),
        ..Default::default()
    };
    let packed = pack(&game_id, &entries, &identity, true)
        .map_err(|e| format!("pack replay container failed: {e}"))?;
    let tmp = replay_path.with_extension("hoi4replay.tmp");
    fs::write(&tmp, packed).map_err(|e| format!("write replay temp failed: {e}"))?;
    if replay_path.exists() {
        fs::remove_file(replay_path).map_err(|e| format!("replace existing replay failed: {e}"))?;
    }
    fs::rename(&tmp, replay_path).map_err(|e| format!("finalize replay failed: {e}"))?;

    let container = Container::open(replay_path, &ExpectedIdentity::default())
        .map_err(|e| format!("verify replay open failed: {e}"))?;
    container
        .verify_all()
        .map_err(|e| format!("verify replay entries failed: {e}"))?;
    let stream_entry = format!("payload/{game_id}/{stream_payload_path}");
    let verified_stream = container
        .read_entry_bytes(&stream_entry)
        .map_err(|e| format!("verify replay stream entry failed: {e}"))?;
    if verified_stream != stream_bytes {
        return Err("verify replay stream entry failed: compact bytes changed".to_string());
    }
    Ok(())
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
        "division_body_bridge": cfg.division_body_bridge.as_deref(),
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
    let dll_path = fs::canonicalize(dll).map_err(|e| {
        format!(
            "canonicalize collector dll path {} failed: {e}",
            dll.display()
        )
    })?;
    let dll_wide = wide(dll_path.as_os_str());
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
    let thread_ids = target_thread_ids(pid, thread_id);
    let mut hooks = Vec::new();
    for target_thread_id in thread_ids {
        let hook = unsafe { SetWindowsHookExW(WH_GETMESSAGE, proc, module, target_thread_id) };
        if !hook.is_null() {
            hooks.push((hook, target_thread_id));
        }
    }
    if hooks.is_empty() {
        unsafe {
            FreeLibrary(module);
        }
        return Err(format!(
            "SetWindowsHookExW failed for all target threads: {}",
            last_error()
        ));
    }
    let poke_ms = hold_ms.min(500);
    let poke_started = Instant::now();
    while duration_millis_u64(poke_started.elapsed()) <= poke_ms {
        for (_, target_thread_id) in &hooks {
            unsafe {
                PostThreadMessageW(*target_thread_id, WM_NULL, 0, 0);
            }
        }
        unsafe {
            PostMessageW(hwnd, WM_NULL, 0, 0);
        }
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(
        hold_ms.saturating_sub(duration_millis_u64(poke_started.elapsed())),
    ));
    unsafe {
        for (hook, _) in hooks {
            UnhookWindowsHookEx(hook);
        }
        FreeLibrary(module);
    }
    Ok(())
}

fn target_thread_ids(pid: Dword, primary_thread_id: Dword) -> Vec<Dword> {
    let mut out = vec![primary_thread_id];
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot as isize == INVALID_HANDLE_VALUE {
        return out;
    }
    let mut entry = ThreadEntry32 {
        dw_size: core::mem::size_of::<ThreadEntry32>() as Dword,
        cnt_usage: 0,
        th32_thread_id: 0,
        th32_owner_process_id: 0,
        tp_base_pri: 0,
        tp_delta_pri: 0,
        dw_flags: 0,
    };
    let mut ok = unsafe { Thread32First(snapshot, &mut entry) };
    while ok != 0 {
        if entry.th32_owner_process_id == pid
            && entry.th32_thread_id != 0
            && !out.contains(&entry.th32_thread_id)
        {
            out.push(entry.th32_thread_id);
        }
        ok = unsafe { Thread32Next(snapshot, &mut entry) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    out
}

fn read_pipe_bytes(pipe_name: &str, timeout_ms: u64) -> Result<Vec<u8>, String> {
    let pipe_wide = wide(OsStr::new(pipe_name));
    let handle = unsafe {
        CreateNamedPipeW(
            pipe_wide.as_ptr(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT,
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
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let connected = unsafe { ConnectNamedPipe(handle, ptr::null_mut()) };
        if connected != 0 || last_error() == ERROR_PIPE_CONNECTED {
            break;
        }
        let err = last_error();
        if err != ERROR_PIPE_LISTENING {
            unsafe {
                CloseHandle(handle);
            }
            return Err(format!("ConnectNamedPipe failed: {err}"));
        }
        if Instant::now() >= deadline {
            unsafe {
                CloseHandle(handle);
            }
            return Err(format!("ConnectNamedPipe timed out after {timeout_ms} ms"));
        }
        thread::sleep(Duration::from_millis(25));
    }
    let mut out = Vec::new();
    let read_deadline = Instant::now() + Duration::from_millis(timeout_ms);
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
        if ok != 0 {
            if got == 0 {
                break;
            }
            out.extend_from_slice(&buf[..got as usize]);
            continue;
        }
        let err = last_error();
        if err == ERROR_BROKEN_PIPE {
            break;
        }
        if err == ERROR_NO_DATA {
            if Instant::now() >= read_deadline {
                break;
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        break;
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
                    let status_flags = read_u32_le(payload, 32);
                    summary.status_flags_or |= status_flags;
                    summary
                        .per_hour_domains
                        .entry(game_hour_raw)
                        .or_default()
                        .insert(domain_id);
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
                    domain.status_flags_or |= status_flags;
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                summary.domain_data_record_count += 1;
                let domain = summary.per_domain.entry(domain_id).or_default();
                domain.data_record_count += 1;
                match domain_id {
                    10 => decode_compact_fuel_data_payload(offset, payload, &mut summary),
                    11 => decode_compact_production_data_payload(offset, payload, &mut summary),
                    12 => decode_compact_focus_data_payload(offset, payload, &mut summary),
                    13 => decode_compact_tech_data_payload(offset, payload, &mut summary),
                    14 => decode_compact_politics_data_payload(offset, payload, &mut summary),
                    15 => decode_compact_politics_residuals_data_payload(
                        offset,
                        payload,
                        &mut summary,
                    ),
                    16 => decode_compact_decisions_data_payload(offset, payload, &mut summary),
                    17 => decode_compact_diplomacy_data_payload(offset, payload, &mut summary),
                    20 => decode_compact_divisions_data_payload(offset, payload, &mut summary),
                    21 => decode_compact_land_combats_data_payload(offset, payload, &mut summary),
                    22 => decode_compact_map_control_data_payload(offset, payload, &mut summary),
                    23 => decode_compact_map_detail_data_payload(offset, payload, &mut summary),
                    24 => decode_compact_weather_data_payload(offset, payload, &mut summary),
                    26 => decode_compact_division_equipment_data_payload(
                        offset,
                        payload,
                        &mut summary,
                    ),
                    27 => decode_compact_army_orders_data_payload(offset, payload, &mut summary),
                    28 => decode_compact_oob_command_data_payload(offset, payload, &mut summary),
                    29 => decode_compact_ships_data_payload(offset, payload, &mut summary),
                    30 => decode_compact_air_wings_data_payload(offset, payload, &mut summary),
                    31 => decode_compact_air_region_activity_data_payload(
                        offset,
                        payload,
                        &mut summary,
                    ),
                    32 => {
                        decode_compact_air_wing_loadouts_data_payload(offset, payload, &mut summary)
                    }
                    33 => {
                        decode_compact_air_variant_defs_data_payload(offset, payload, &mut summary)
                    }
                    34 => {
                        decode_compact_air_base_status_data_payload(offset, payload, &mut summary)
                    }
                    35 => {
                        decode_compact_air_variant_stats_data_payload(offset, payload, &mut summary)
                    }
                    other => summary.parse_errors.push(format!(
                        "offset {offset}: unsupported domain data payload for {}",
                        compact_domain_name(other)
                    )),
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
                    summary.status_flags_or |= read_u32_le(payload, 24);
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

fn decode_compact_fuel_data_payload(offset: usize, payload: &[u8], summary: &mut CompactSummary) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    summary.status_flags_or |= status_flags;
    if scale_raw != COMPACT_FUEL_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data scale {scale_raw} != expected {COMPACT_FUEL_SCALE_RAW}"
        ));
    }
    let expected_len = 24usize.saturating_add(row_count.saturating_mul(40));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data payload is {} byte(s), expected {expected_len} for {row_count} row(s)",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&10) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 10, "fuel_country_rows", row_count);
    let mut seen_tag_ids = BTreeSet::new();
    for row_index in 0..row_count {
        let row_offset = 24 + row_index * 40;
        let tag_db_id = read_u32_le(payload, row_offset);
        let row_flags = read_u32_le(payload, row_offset + 4);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} has zero tag_db_id"
            ));
        } else if !seen_tag_ids.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if row_flags & !1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} has unknown flags {row_flags}"
            ));
        }
        let current_raw = read_i64_le(payload, row_offset + 8);
        let max_raw = read_i64_le(payload, row_offset + 16);
        let gain_raw = read_i64_le(payload, row_offset + 24);
        let cost_raw = read_i64_le(payload, row_offset + 32);
        if current_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} current_raw is negative"
            ));
        }
        if max_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} max_raw is negative"
            ));
        }
        let _ = gain_raw;
        let _ = cost_raw;
    }
}

fn decode_compact_production_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: production data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let stockpile_count = read_u32_le(payload, 24) as usize;
    let military_count = read_u32_le(payload, 28) as usize;
    let naval_count = read_u32_le(payload, 32) as usize;
    let construction_count = read_u32_le(payload, 36) as usize;
    summary.status_flags_or |= status_flags;
    if scale_raw != COMPACT_PRODUCTION_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: production data scale {scale_raw} != expected {COMPACT_PRODUCTION_SCALE_RAW}"
        ));
    }
    let line_count = military_count.saturating_add(naval_count);
    let expected_len = 40usize
        .saturating_add(stockpile_count.saturating_mul(16))
        .saturating_add(line_count.saturating_mul(32))
        .saturating_add(construction_count.saturating_mul(32));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: production data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: production data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&11) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 11, "production_country_rows", country_count);
    add_compact_domain_data_unit(summary, 11, "stockpile_rows", stockpile_count);
    add_compact_domain_data_unit(summary, 11, "military_line_rows", military_count);
    add_compact_domain_data_unit(summary, 11, "naval_line_rows", naval_count);
    add_compact_domain_data_unit(summary, 11, "construction_queue_rows", construction_count);
    let mut cursor = 40usize;
    let mut seen_stockpile = BTreeSet::new();
    for row_index in 0..stockpile_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let variant_id = read_u32_le(payload, cursor + 4);
        let _amount_raw = read_i64_le(payload, cursor + 8);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: production stockpile row {row_index} has zero tag_db_id"
            ));
        }
        if !seen_stockpile.insert((tag_db_id, variant_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: production stockpile row {row_index} duplicates tag_db_id {tag_db_id} variant_id {variant_id}"
            ));
        }
        cursor += 16;
    }
    let mut seen_lines = BTreeSet::new();
    for row_index in 0..line_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let line_id = read_i32_le(payload, cursor + 8);
        let active_factories = read_i32_le(payload, cursor + 12);
        let _cost_raw = read_i32_le(payload, cursor + 16);
        let _speed_raw = read_i32_le(payload, cursor + 20);
        let _produced_raw = read_i32_le(payload, cursor + 24);
        let _amount = read_i32_le(payload, cursor + 28);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} has zero tag_db_id"
            ));
        }
        if kind != 1 && kind != 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} has invalid kind {kind}"
            ));
        }
        if !(0..=100_000).contains(&active_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} has invalid active_factories {active_factories}"
            ));
        }
        if !seen_lines.insert((tag_db_id, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
        cursor += 32;
    }
    let mut seen_construction = BTreeSet::new();
    for row_index in 0..construction_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let line_id = read_u32_le(payload, cursor + 4);
        let _template_id = read_u32_le(payload, cursor + 8);
        let active_factories = read_i32_le(payload, cursor + 12);
        let cost_raw = read_i32_le(payload, cursor + 16);
        let produced_raw = read_i32_le(payload, cursor + 20);
        let priority = read_u32_le(payload, cursor + 24);
        let reserved = read_u32_le(payload, cursor + 28);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} has zero tag_db_id"
            ));
        }
        if !(0..=100_000).contains(&active_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} has invalid active_factories {active_factories}"
            ));
        }
        if cost_raw < 0 || produced_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} has negative progress fields"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} reserved field is {reserved}"
            ));
        }
        if !seen_construction.insert((tag_db_id, line_id, priority)) {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} duplicates tag_db_id {tag_db_id} line_id {line_id} priority {priority}"
            ));
        }
        cursor += 32;
    }
}

fn decode_compact_focus_data_payload(offset: usize, payload: &[u8], summary: &mut CompactSummary) {
    if payload.len() < 28 {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data payload is {} byte(s), expected at least 28",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let completed_token_count = read_u32_le(payload, 24) as usize;
    summary.status_flags_or |= status_flags;
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&12) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 12, "focus_country_rows", country_count);
    add_compact_domain_data_unit(summary, 12, "completed_focus_tokens", completed_token_count);
    let mut cursor = 28usize;
    let mut seen_countries = BTreeSet::new();
    for row_index in 0..country_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 16, "focus country row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let completed_count = read_u32_le(payload, cursor + 8);
        let progress_raw = read_i32_le(payload, cursor + 12);
        cursor += 16;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11_1111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has unknown flags {flags}"
            ));
        }
        if completed_count != u32::MAX && completed_count > 4096 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has invalid completed_count {completed_count}"
            ));
        }
        if flags & (1 << 4) == 0 && progress_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has progress without flag"
            ));
        }
        for field in ["focus tree", "focus current", "focus current continuous"] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..completed_token_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "focus completed row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus completed row {row_index} has zero tag_db_id"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "focus completed token") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_tech_data_payload(offset: usize, payload: &[u8], summary: &mut CompactSummary) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: tech data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let progress_scale = read_u32_le(payload, 20);
    let live_scale = read_u32_le(payload, 24);
    let active_count = read_u32_le(payload, 28) as usize;
    summary.status_flags_or |= status_flags;
    if progress_scale != COMPACT_TECH_PROGRESS_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: tech progress scale {progress_scale} != expected {COMPACT_TECH_PROGRESS_SCALE_RAW}"
        ));
    }
    if live_scale != COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: tech live scale {live_scale} != expected {COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW}"
        ));
    }
    let expected_len = 32usize
        .saturating_add(country_count.saturating_mul(16))
        .saturating_add(active_count.saturating_mul(40));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: tech data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: tech data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&13) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 13, "tech_country_rows", country_count);
    add_compact_domain_data_unit(summary, 13, "active_research_slot_rows", active_count);
    let mut cursor = 32usize;
    let mut seen_countries = BTreeSet::new();
    for row_index in 0..country_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let _completed_count = read_u32_le(payload, cursor + 8);
        let slot_count = read_u32_le(payload, cursor + 12);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} has unknown flags {flags}"
            ));
        }
        if slot_count != u32::MAX && slot_count > 32 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} has invalid slot_count {slot_count}"
            ));
        }
        cursor += 16;
    }
    let mut seen_slots = BTreeSet::new();
    for row_index in 0..active_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let slot_index = read_u32_le(payload, cursor + 4);
        let _tech_handle_raw = read_u32_le(payload, cursor + 8);
        let _tech_def_index_raw = read_u32_le(payload, cursor + 12);
        let _progress_raw = read_u32_le(payload, cursor + 16);
        let flags = read_u32_le(payload, cursor + 20);
        let _live_progress_raw = read_i64_le(payload, cursor + 24);
        let _base_required_points_raw = read_i64_le(payload, cursor + 32);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has zero tag_db_id"
            ));
        }
        if slot_index > 32 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has invalid slot_index {slot_index}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has unknown flags {flags}"
            ));
        }
        if !seen_slots.insert((tag_db_id, slot_index)) {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} duplicates tag_db_id {tag_db_id} slot_index {slot_index}"
            ));
        }
        cursor += 40;
    }
}

fn decode_compact_politics_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let popularity_count = read_u32_le(payload, 24) as usize;
    let law_count = read_u32_le(payload, 28) as usize;
    let advisor_count = read_u32_le(payload, 32) as usize;
    let national_spirit_count = read_u32_le(payload, 36) as usize;
    summary.status_flags_or |= status_flags;
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&14) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 14, "politics_country_rows", country_count);
    add_compact_domain_data_unit(summary, 14, "popularity_rows", popularity_count);
    add_compact_domain_data_unit(summary, 14, "law_rows", law_count);
    add_compact_domain_data_unit(summary, 14, "advisor_rows", advisor_count);
    add_compact_domain_data_unit(summary, 14, "national_spirit_rows", national_spirit_count);
    let mut cursor = 40usize;
    let mut seen_countries = BTreeSet::new();
    for row_index in 0..country_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 28, "politics country row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let idea_count = read_u32_le(payload, cursor + 8);
        let election_frequency = read_u32_le(payload, cursor + 12);
        let party_count = read_u32_le(payload, cursor + 16);
        let low_confidence = read_u32_le(payload, cursor + 20);
        let _leader_char_id = read_u32_le(payload, cursor + 24);
        cursor += 28;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11_1111_1111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has unknown flags {flags}"
            ));
        }
        if idea_count != u32::MAX && idea_count > 4096 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid idea_count {idea_count}"
            ));
        }
        if election_frequency != u32::MAX && !(1..=600).contains(&election_frequency) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid election_frequency {election_frequency}"
            ));
        }
        if party_count != u32::MAX && party_count > 128 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid party_count {party_count}"
            ));
        }
        if low_confidence != u32::MAX && low_confidence > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid low_confidence {low_confidence}"
            ));
        }
        for field in [
            "politics ruling party",
            "politics leader name",
            "politics leader token",
            "politics leader ideology",
        ] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..popularity_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics popularity row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics popularity row {row_index} has zero tag_db_id"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "politics popularity ideology")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics popularity row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let raw = read_i32_le(payload, cursor);
        if !(0..=(COMPACT_FIXED_SCALE_RAW as i32 * 100)).contains(&raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics popularity row {row_index} has invalid raw value {raw}"
            ));
        }
        cursor += 4;
    }
    for row_index in 0..law_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics law row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics law row {row_index} has zero tag_db_id"
            ));
        }
        for field in ["politics law group", "politics law token"] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..advisor_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics advisor row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics advisor row {row_index} has zero tag_db_id"
            ));
        }
        for field in ["politics advisor group", "politics advisor token"] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..national_spirit_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 4, "politics national spirit row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics national spirit row {row_index} has zero tag_db_id"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "politics national spirit") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_politics_residuals_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let advisor_detail_count = read_u32_le(payload, 20) as usize;
    summary.status_flags_or |= status_flags;
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&15) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 15, "politics_residual_country_rows", country_count);
    add_compact_domain_data_unit(summary, 15, "advisor_detail_rows", advisor_detail_count);
    let mut cursor = 24usize;
    let mut seen_countries = BTreeSet::new();
    let mut declared_advisors = 0usize;
    for row_index in 0..country_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 16, "politics residuals country row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let elections_allowed = read_u32_le(payload, cursor + 8);
        let advisor_count = read_u32_le(payload, cursor + 12) as usize;
        cursor += 16;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} has unknown flags {flags}"
            ));
        }
        if elections_allowed != u32::MAX && elections_allowed > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} has invalid elections_allowed {elections_allowed}"
            ));
        }
        declared_advisors = declared_advisors.saturating_add(advisor_count);
        if let Err(err) = skip_compact_string(payload, &mut cursor, "politics last election") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if declared_advisors != advisor_detail_count {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals advisor_detail_count {advisor_detail_count} != declared country advisors {declared_advisors}"
        ));
    }
    for row_index in 0..advisor_detail_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 4, "politics residuals advisor row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals advisor row {row_index} has zero tag_db_id"
            ));
        }
        for field in [
            "politics advisor idea token",
            "politics advisor slot",
            "politics advisor ledger",
            "politics advisor character",
        ] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_decisions_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let timed_count = read_u32_le(payload, 20) as usize;
    summary.status_flags_or |= status_flags;
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&16) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 16, "decision_country_rows", country_count);
    add_compact_domain_data_unit(summary, 16, "timed_decision_rows", timed_count);
    let mut cursor = 24usize;
    let mut seen_countries = BTreeSet::new();
    let mut declared_timed = 0usize;
    for row_index in 0..country_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 32, "decisions country row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let timed_for_country = read_u32_le(payload, cursor + 28) as usize;
        cursor += 32;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions country row {row_index} has unknown flags {flags}"
            ));
        }
        declared_timed = declared_timed.saturating_add(timed_for_country);
    }
    if declared_timed != timed_count {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions timed_count {timed_count} != declared country timed decisions {declared_timed}"
        ));
    }
    for row_index in 0..timed_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 12, "decisions timed row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let days = read_i32_le(payload, cursor + 4);
        let state = read_i32_le(payload, cursor + 8);
        cursor += 12;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions timed row {row_index} has zero tag_db_id"
            ));
        }
        if !(0..=10_000).contains(&days) {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions timed row {row_index} has invalid days {days}"
            ));
        }
        if !(0..=16).contains(&state) {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions timed row {row_index} has invalid state {state}"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "decisions timed token") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_diplomacy_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let relation_count = read_u32_le(payload, 20) as usize;
    summary.status_flags_or |= status_flags;
    let expected_len = 24usize
        .saturating_add(country_count.saturating_mul(24))
        .saturating_add(relation_count.saturating_mul(16));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&17) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 17, "diplomacy_country_rows", country_count);
    add_compact_domain_data_unit(summary, 17, "at_war_relation_rows", relation_count);
    let mut cursor = 24usize;
    let mut seen_countries = BTreeSet::new();
    let mut declared_relations = 0usize;
    for row_index in 0..country_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let _faction_group = read_i32_le(payload, cursor + 8);
        let at_war_count = read_u32_le(payload, cursor + 12) as usize;
        let _casualties = read_i64_le(payload, cursor + 16);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy country row {row_index} has unknown flags {flags}"
            ));
        }
        declared_relations = declared_relations.saturating_add(at_war_count);
        cursor += 24;
    }
    if declared_relations != relation_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy relation_count {relation_count} != declared country relations {declared_relations}"
        ));
    }
    let mut seen_relations = BTreeSet::new();
    for row_index in 0..relation_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let token_len = read_u32_le(payload, cursor + 4) as usize;
        let token = &payload[cursor + 8..cursor + 16];
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has zero tag_db_id"
            ));
        }
        if token_len == 0 || token_len > 8 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has invalid token_len {token_len}"
            ));
        }
        if token_len <= 8
            && token[..token_len]
                .iter()
                .any(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-'))
        {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has non-token bytes"
            ));
        }
        if token_len < 8 && token[token_len..].iter().any(|byte| *byte != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has nonzero padding"
            ));
        }
        if !seen_relations.insert((tag_db_id, token.to_vec())) {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        cursor += 16;
    }
}

fn decode_compact_divisions_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let root_rows = read_u32_le(payload, 24) as usize;
    let reserved = read_u32_le(payload, 28);
    summary.status_flags_or |= status_flags;
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data reserved field is nonzero"
        ));
    }
    let expected_len = 32usize.saturating_add(row_count.saturating_mul(COMPACT_DIVISION_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions delta changed_rows {changed_rows} != emitted row_count {row_count}"
        ));
    }
    if !delta_data && row_count > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data row_count {row_count} > root_rows {root_rows}"
        ));
    }
    if (status_flags & COMPACT_STATUS_BRIDGE_UNAVAILABLE) != 0 && root_rows == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data root source unavailable"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&20) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 20, "division_rows", row_count);
    add_compact_domain_data_unit(summary, 20, "division_root_rows", root_rows);
    let mut cursor = 32usize;
    let mut seen_divisions = BTreeSet::new();
    let mut deleted_rows = 0usize;
    let mut moving_rows = 0usize;
    let mut combat_rows = 0usize;
    let mut template_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut army_rows = 0usize;
    let mut org_strength_rows = 0usize;
    for row_index in 0..row_count {
        let div_id = read_u32_le(payload, cursor);
        let div_type = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let location = read_u32_le(payload, cursor + 12);
        let move_target = read_u32_le(payload, cursor + 16);
        let move_source = read_u32_le(payload, cursor + 20);
        let movement_progress = read_u32_le(payload, cursor + 24);
        let progress_bucket = read_u32_le(payload, cursor + 28);
        let organisation_raw = read_u32_le(payload, cursor + 32);
        let _organisation_bucket = read_u32_le(payload, cursor + 36);
        let strength_raw = read_u32_le(payload, cursor + 40);
        let _strength_bucket = read_u32_le(payload, cursor + 44);
        let combat_ref_count = read_u32_le(payload, cursor + 48);
        let template_id = read_u32_le(payload, cursor + 52);
        let owner_tag_db_id = read_u32_le(payload, cursor + 56);
        let army_id = read_u32_le(payload, cursor + 60);
        let experience_raw = read_u32_le(payload, cursor + 64);
        let max_supply_raw = read_u32_le(payload, cursor + 68);
        let current_supply_raw = read_u32_le(payload, cursor + 72);
        let supply_gain_raw = read_u32_le(payload, cursor + 76);
        let fuel_raw = read_u32_le(payload, cursor + 80);
        let row_reserved2 = read_u32_le(payload, cursor + 84);
        cursor += COMPACT_DIVISION_ROW_LEN;

        if div_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero div_id"
            ));
        } else if !seen_divisions.insert(div_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} duplicates div_id {div_id}"
            ));
        }
        if div_type != COMPACT_DIVISION_TYPE {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has div_type {div_type}"
            ));
        }
        if flags
            & !(0b1111_1111_1111
                | COMPACT_DIVISION_ROW_FLAG_DELETED
                | COMPACT_DIVISION_ROW_FLAG_TEMPLATE)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = (flags & COMPACT_DIVISION_ROW_FLAG_DELETED) != 0;
        let moving = (flags & 1) != 0;
        let has_progress = (flags & (1 << 1)) != 0;
        let has_target = (flags & (1 << 2)) != 0;
        let has_source = (flags & (1 << 3)) != 0;
        let in_combat = (flags & (1 << 4)) != 0;
        let has_owner = (flags & (1 << 5)) != 0;
        let has_army = (flags & (1 << 6)) != 0;
        let has_experience = (flags & (1 << 7)) != 0;
        let has_max_supply = (flags & (1 << 8)) != 0;
        let has_current_supply = (flags & (1 << 9)) != 0;
        let has_supply_gain = (flags & (1 << 10)) != 0;
        let has_fuel = (flags & (1 << 11)) != 0;
        let has_template = (flags & COMPACT_DIVISION_ROW_FLAG_TEMPLATE) != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != COMPACT_DIVISION_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} has non-delete flags"
                ));
            }
            let zero_filled = location == 0
                && move_target == 0
                && move_source == 0
                && movement_progress == 0
                && progress_bucket == 0
                && organisation_raw == 0
                && _organisation_bucket == 0
                && strength_raw == 0
                && _strength_bucket == 0
                && combat_ref_count == 0
                && template_id == 0
                && owner_tag_db_id == 0
                && army_id == 0
                && experience_raw == 0
                && max_supply_raw == 0
                && current_supply_raw == 0
                && supply_gain_raw == 0
                && fuel_raw == 0
                && row_reserved2 == 0;
            if !zero_filled {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} is not zero-filled"
                ));
            }
            continue;
        }
        org_strength_rows = org_strength_rows.saturating_add(1);
        if moving {
            moving_rows = moving_rows.saturating_add(1);
        }
        if in_combat {
            combat_rows = combat_rows.saturating_add(1);
        }
        if has_template {
            template_rows = template_rows.saturating_add(1);
        }
        if has_owner {
            owner_rows = owner_rows.saturating_add(1);
        }
        if has_army {
            army_rows = army_rows.saturating_add(1);
        }
        if !valid_compact_province_id(location) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid location {location}"
            ));
        }
        if has_target && !valid_compact_province_id(move_target) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid move_target {move_target}"
            ));
        }
        if has_source && !valid_compact_province_id(move_source) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid move_source {move_source}"
            ));
        }
        if !moving && (has_progress || has_target || has_source) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has movement fields while not moving"
            ));
        }
        if has_progress && movement_progress == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero movement progress"
            ));
        }
        if !has_progress && movement_progress != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has progress without flag"
            ));
        }
        if !has_target && move_target != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has target without flag"
            ));
        }
        if !has_source && move_source != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has source without flag"
            ));
        }
        if !has_progress && progress_bucket != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has progress bucket without flag"
            ));
        }
        if organisation_raw > COMPACT_MAX_DIVISION_ORGANISATION_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} organisation_raw exceeds max"
            ));
        }
        if strength_raw > COMPACT_MAX_DIVISION_STRENGTH_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} strength_raw exceeds max"
            ));
        }
        if combat_ref_count > COMPACT_MAX_DIVISION_COMBAT_REF_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} combat_ref_count exceeds max"
            ));
        }
        if !in_combat && combat_ref_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has combat refs while not in combat"
            ));
        }
        if in_combat && combat_ref_count == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} is in combat without refs"
            ));
        }
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_template,
            template_id,
            1,
            COMPACT_MAX_DIVISION_TEMPLATE_ID,
            "template_id",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_owner,
            owner_tag_db_id,
            1,
            COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID,
            "owner_tag_db_id",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_army,
            army_id,
            1,
            COMPACT_MAX_ARMY_ID,
            "army_id",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_experience,
            experience_raw,
            0,
            COMPACT_MAX_DIVISION_EXPERIENCE_RAW,
            "experience_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_max_supply,
            max_supply_raw,
            0,
            COMPACT_MAX_DIVISION_SUPPLY_RAW,
            "max_supply_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_current_supply,
            current_supply_raw,
            0,
            COMPACT_MAX_DIVISION_SUPPLY_RAW,
            "current_supply_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_supply_gain,
            supply_gain_raw,
            0,
            COMPACT_MAX_DIVISION_SUPPLY_GAIN_RAW,
            "supply_gain_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_fuel,
            fuel_raw,
            0,
            COMPACT_MAX_DIVISION_FUEL_RAW,
            "fuel_raw",
        );
        if row_reserved2 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} second reserved field is nonzero"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 20, "division_deleted_rows", deleted_rows);
    add_compact_domain_data_unit(summary, 20, "division_moving_rows", moving_rows);
    add_compact_domain_data_unit(summary, 20, "division_in_combat_rows", combat_rows);
    add_compact_domain_data_unit(summary, 20, "division_template_rows", template_rows);
    add_compact_domain_data_unit(summary, 20, "division_owner_rows", owner_rows);
    add_compact_domain_data_unit(summary, 20, "division_army_rows", army_rows);
    add_compact_domain_data_unit(summary, 20, "division_org_strength_rows", org_strength_rows);
}

fn decode_compact_division_equipment_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let root_rows = read_u32_le(payload, 24) as usize;
    let observed_division_rows = read_u32_le(payload, 28) as usize;
    let equipment_entry_count = read_u32_le(payload, 32) as usize;
    let observed_equipment_rows = read_u32_le(payload, 36) as usize;
    summary.status_flags_or |= status_flags;
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    let expected_len = 40usize
        .saturating_add(row_count.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ROW_LEN))
        .saturating_add(
            equipment_entry_count.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN),
        );
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment delta changed_rows {changed_rows} != emitted row_count {row_count}"
        ));
    }
    if !delta_data && row_count != observed_division_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment full row_count {row_count} != observed_division_rows {observed_division_rows}"
        ));
    }
    if observed_division_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment observed_division_rows {observed_division_rows} > root_rows {root_rows}"
        ));
    }
    if !delta_data && equipment_entry_count != observed_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment full equipment_entry_count {equipment_entry_count} != observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if equipment_entry_count > observed_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment emitted entries {equipment_entry_count} > observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&26) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 26, "division_equipment_rows", row_count);
    add_compact_domain_data_unit(summary, 26, "division_equipment_root_rows", root_rows);
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_observed_division_rows",
        observed_division_rows,
    );
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_entry_rows",
        equipment_entry_count,
    );
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_observed_entry_rows",
        observed_equipment_rows,
    );

    let mut cursor = 40usize;
    let mut seen_divisions = BTreeSet::new();
    let mut declared_entries = 0usize;
    let mut deleted_rows = 0usize;
    let mut parent_ranges = Vec::with_capacity(row_count);
    for row_index in 0..row_count {
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let div_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let template_id = read_u32_le(payload, cursor + 12);
        let entry_first = read_u32_le(payload, cursor + 16) as usize;
        let entry_count = read_u32_le(payload, cursor + 20) as usize;
        cursor += COMPACT_DIVISION_EQUIPMENT_ROW_LEN;

        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if div_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} has zero div_id"
            ));
        } else if !seen_divisions.insert((owner_tag_db_id, div_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} duplicates owner/division {owner_tag_db_id}:{div_id}"
            ));
        }
        if flags
            & !(COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE
                | COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = (flags & COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED) != 0;
        let has_template = (flags & COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE) != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} has non-delete flags"
                ));
            }
            if template_id != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} has template_id"
                ));
            }
            if entry_count != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} has entries"
                ));
            }
            if entry_first > equipment_entry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} entry_first {entry_first} exceeds {equipment_entry_count}"
                ));
            }
        } else {
            validate_optional_compact_u32(
                summary,
                offset,
                row_index,
                has_template,
                template_id,
                1,
                COMPACT_MAX_DIVISION_TEMPLATE_ID,
                "template_id",
            );
            if entry_first.saturating_add(entry_count) > equipment_entry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment row {row_index} entry range {entry_first}+{entry_count} exceeds {equipment_entry_count}"
                ));
            }
        }
        declared_entries = declared_entries.saturating_add(entry_count);
        parent_ranges.push((
            row_index,
            owner_tag_db_id,
            div_id,
            entry_first,
            entry_count,
            deleted,
        ));
    }
    if declared_entries != equipment_entry_count {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment entry_count {equipment_entry_count} != declared {declared_entries}"
        ));
    }
    add_compact_domain_data_unit(summary, 26, "division_equipment_deleted_rows", deleted_rows);

    let mut seen_entries = BTreeSet::new();
    for (row_index, owner_tag_db_id, div_id, entry_first, entry_count, deleted) in parent_ranges {
        if deleted {
            continue;
        }
        for child_index in entry_first..entry_first.saturating_add(entry_count) {
            let child_offset = 40usize
                .saturating_add(row_count.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ROW_LEN))
                .saturating_add(
                    child_index.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN),
                );
            if child_offset.saturating_add(COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN) > payload.len()
            {
                continue;
            }
            let variant_id = read_u32_le(payload, child_offset);
            let reserved = read_u32_le(payload, child_offset + 4);
            let amount_raw = read_i64_le(payload, child_offset + 8);
            if variant_id == 0 || variant_id > COMPACT_MAX_DIVISION_EQUIPMENT_VARIANT_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment entry row {child_index} has invalid variant_id {variant_id}"
                ));
            }
            if amount_raw <= 0 || amount_raw > COMPACT_MAX_DIVISION_EQUIPMENT_AMOUNT_RAW {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment entry row {child_index} has invalid amount_raw {amount_raw}"
                ));
            }
            if reserved != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment entry row {child_index} reserved field is nonzero"
                ));
            }
            if !seen_entries.insert((owner_tag_db_id, div_id, variant_id)) {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment row {row_index} duplicates variant {variant_id}"
                ));
            }
        }
    }
}

fn validate_optional_compact_u32(
    summary: &mut CompactSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    min_value: u32,
    max_value: u32,
    field: &str,
) {
    if present {
        if value < min_value || value > max_value {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} {field} value {value} out of range"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} has {field} without flag"
        ));
    }
}

fn valid_compact_province_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_MAX_DIVISION_PROVINCE_ID
}

fn decode_compact_army_orders_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: army order data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let root_rows = read_u32_le(payload, 20) as usize;
    let observed_army_rows = read_u32_le(payload, 24) as usize;
    let observed_order_rows = read_u32_le(payload, 28) as usize;
    let target_state_count = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    summary.status_flags_or |= status_flags;
    let expected_len = 40usize
        .saturating_add(row_count.saturating_mul(COMPACT_ARMY_ORDER_ROW_LEN))
        .saturating_add(target_state_count.saturating_mul(COMPACT_ARMY_ORDER_TARGET_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: army order data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army order changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: army order observed_army_rows {observed_army_rows} > root_rows {root_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army order deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&27) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 27, "army_order_rows", row_count);
    add_compact_domain_data_unit(summary, 27, "army_order_root_rows", root_rows);
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_observed_armies",
        observed_army_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_observed_orders",
        observed_order_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_target_state_rows",
        target_state_count,
    );
    add_compact_domain_data_unit(summary, 27, "army_order_deleted_rows", deleted_rows);

    let mut cursor = 40usize;
    let mut seen_orders = BTreeSet::new();
    let mut declared_targets = 0usize;
    let mut exact_rows = 0usize;
    let mut unsupported_rows = 0usize;
    let mut overflow_rows = 0usize;
    let mut validation_failed_rows = 0usize;
    for row_index in 0..row_count {
        let army_id = read_u32_le(payload, cursor);
        let generation = read_u64_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 12);
        let slot_index = read_u32_le(payload, cursor + 16);
        let order_type = read_i32_le(payload, cursor + 20);
        let target_first = read_u32_le(payload, cursor + 24) as usize;
        let target_count = read_u32_le(payload, cursor + 28) as usize;
        let target_state_declared_count = read_u32_le(payload, cursor + 32) as usize;
        let target_status_code = read_u32_le(payload, cursor + 36);
        let row_reserved0 = read_u32_le(payload, cursor + 40);
        let row_reserved1 = read_u32_le(payload, cursor + 44);
        cursor += COMPACT_ARMY_ORDER_ROW_LEN;

        let deleted = (flags & COMPACT_ARMY_ORDER_ROW_FLAG_DELETED) != 0;
        let has_target_states = (flags & COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES) != 0;
        let has_target_state_count = (flags & COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT) != 0;
        if army_id == 0 || army_id > COMPACT_MAX_ARMY_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has invalid army_id {army_id}"
            ));
        } else if !seen_orders.insert((army_id, generation, slot_index, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} duplicates army {army_id}:{generation}:{slot_index}"
            ));
        }
        if flags
            & !(COMPACT_ARMY_ORDER_ROW_FLAG_DELETED
                | COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES
                | COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has unknown flags {flags}"
            ));
        }
        if slot_index >= COMPACT_ARMY_ORDER_MAX_SLOT_INDEX {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has invalid slot_index {slot_index}"
            ));
        }
        if order_type < -1_000 || order_type > 1_000 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has suspicious order_type {order_type}"
            ));
        }
        if deleted {
            if target_first != 0
                || target_count != 0
                || target_state_declared_count != 0
                || target_status_code != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order deleted row {row_index} has live target fields"
                ));
            }
        } else {
            match target_status_code {
                0 => {}
                1 => exact_rows = exact_rows.saturating_add(1),
                2 => unsupported_rows = unsupported_rows.saturating_add(1),
                3 => overflow_rows = overflow_rows.saturating_add(1),
                4 => validation_failed_rows = validation_failed_rows.saturating_add(1),
                other => summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} has unknown target_status_code {other}"
                )),
            }
        }
        if !has_target_states && target_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_count without target-states flag"
            ));
        }
        if has_target_states {
            let Some(target_end) = target_first.checked_add(target_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} target range overflows"
                ));
                continue;
            };
            if target_end > target_state_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} target range {target_first}..{target_end} exceeds {target_state_count}"
                ));
            }
            declared_targets = declared_targets.saturating_add(target_count);
        } else if target_first != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_first without target-states flag"
            ));
        }
        if !has_target_state_count && target_state_declared_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_state_count without flag"
            ));
        }
        if row_reserved0 != 0 || row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} reserved field is nonzero"
            ));
        }
    }
    if declared_targets != target_state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army order target_state_count {target_state_count} != declared {declared_targets}"
        ));
    }
    add_compact_domain_data_unit(summary, 27, "army_order_exact_target_rows", exact_rows);
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_unsupported_target_rows",
        unsupported_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_overflow_target_rows",
        overflow_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_validation_failed_target_rows",
        validation_failed_rows,
    );

    let mut seen_target_states = BTreeSet::new();
    for row_index in 0..target_state_count {
        let state_id = read_u32_le(payload, cursor);
        let row_reserved = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_ARMY_ORDER_TARGET_ROW_LEN;
        if !valid_compact_army_order_state_id(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order target row {row_index} has invalid state_id {state_id}"
            ));
        }
        if !seen_target_states.insert((row_index, state_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order target row {row_index} duplicates state_id {state_id}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order target row {row_index} reserved field is nonzero"
            ));
        }
    }
}

fn valid_compact_army_order_state_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_ARMY_ORDER_MAX_STATE_ID
}

fn decode_compact_oob_command_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let root_rows = read_u32_le(payload, 20) as usize;
    let observed_army_rows = read_u32_le(payload, 24) as usize;
    let observed_army_group_rows = read_u32_le(payload, 28) as usize;
    let member_count = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    let observed_fleet_rows = read_u32_le(payload, 40) as usize;
    let observed_task_force_rows = read_u32_le(payload, 44) as usize;
    summary.status_flags_or |= status_flags;
    if member_count > COMPACT_OOB_MAX_MEMBER_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} exceeds limit {COMPACT_OOB_MAX_MEMBER_ROWS}"
        ));
    }
    let expected_len = 48usize
        .saturating_add(row_count.saturating_mul(COMPACT_OOB_ROW_LEN))
        .saturating_add(member_count.saturating_mul(COMPACT_OOB_MEMBER_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command observed_army_rows {observed_army_rows} > root_rows {root_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&28) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 28, "oob_command_rows", row_count);
    add_compact_domain_data_unit(summary, 28, "oob_command_root_rows", root_rows);
    add_compact_domain_data_unit(summary, 28, "oob_observed_army_rows", observed_army_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_observed_army_group_rows",
        observed_army_group_rows,
    );
    add_compact_domain_data_unit(summary, 28, "oob_member_rows", member_count);
    add_compact_domain_data_unit(summary, 28, "oob_deleted_rows", deleted_rows);
    add_compact_domain_data_unit(summary, 28, "oob_observed_fleet_rows", observed_fleet_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_observed_task_force_rows",
        observed_task_force_rows,
    );

    let mut cursor = 48usize;
    let mut seen_rows = BTreeSet::new();
    let mut declared_members = 0usize;
    let mut emitted_army_rows = 0usize;
    let mut emitted_army_group_rows = 0usize;
    let mut emitted_fleet_rows = 0usize;
    let mut emitted_task_force_rows = 0usize;
    let mut member_division_rows = 0usize;
    let mut member_army_rows = 0usize;
    let mut member_task_force_rows = 0usize;
    let mut member_ship_rows = 0usize;
    let mut leader_rows = 0usize;
    let mut leader_skill_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut location_rows = 0usize;
    let mut engine_count_rows = 0usize;
    let mut missing_count_rows = 0usize;
    for row_index in 0..row_count {
        let kind = read_u32_le(payload, cursor);
        let object_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let parent_id = read_u32_le(payload, cursor + 12);
        let owner_tag_db_id = read_u32_le(payload, cursor + 16);
        let leader_id = read_u32_le(payload, cursor + 20);
        let leader_skill = read_u32_le(payload, cursor + 24);
        let member_first = read_u32_le(payload, cursor + 28) as usize;
        let row_member_count = read_u32_le(payload, cursor + 32) as usize;
        let logical_member_count = read_u32_le(payload, cursor + 36) as usize;
        let member_count_engine = read_u32_le(payload, cursor + 40);
        let missing_member_count = read_u32_le(payload, cursor + 44);
        let leader_trait_count = read_u32_le(payload, cursor + 48);
        let parent_status = read_u32_le(payload, cursor + 52);
        let member_status = read_u32_le(payload, cursor + 56);
        let owner_status = read_u32_le(payload, cursor + 60);
        let leader_status = read_u32_le(payload, cursor + 64);
        let leader_trait_status = read_u32_le(payload, cursor + 68);
        let location_province_id = read_u32_le(payload, cursor + 72);
        let row_reserved1 = read_u32_le(payload, cursor + 76);
        cursor += COMPACT_OOB_ROW_LEN;

        let deleted = (flags & COMPACT_OOB_ROW_FLAG_DELETED) != 0;
        if !matches!(
            kind,
            COMPACT_OOB_KIND_ARMY
                | COMPACT_OOB_KIND_ARMY_GROUP
                | COMPACT_OOB_KIND_FLEET
                | COMPACT_OOB_KIND_TASK_FORCE
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid kind {kind}"
            ));
        } else if kind == COMPACT_OOB_KIND_ARMY {
            emitted_army_rows = emitted_army_rows.saturating_add(1);
        } else if kind == COMPACT_OOB_KIND_ARMY_GROUP {
            emitted_army_group_rows = emitted_army_group_rows.saturating_add(1);
        } else if kind == COMPACT_OOB_KIND_FLEET {
            emitted_fleet_rows = emitted_fleet_rows.saturating_add(1);
        } else {
            emitted_task_force_rows = emitted_task_force_rows.saturating_add(1);
        }
        if !valid_compact_oob_id(object_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid object_id {object_id}"
            ));
        } else if !seen_rows.insert((kind, object_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} duplicates {kind}:{object_id}"
            ));
        }
        if flags
            & !(COMPACT_OOB_ROW_FLAG_DELETED
                | COMPACT_OOB_ROW_FLAG_PARENT
                | COMPACT_OOB_ROW_FLAG_OWNER
                | COMPACT_OOB_ROW_FLAG_LEADER
                | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT
                | COMPACT_OOB_ROW_FLAG_LOCATION)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has unknown flags {flags}"
            ));
        }
        if deleted {
            if parent_id != 0
                || owner_tag_db_id != 0
                || leader_id != 0
                || leader_skill != 0
                || row_member_count != 0
                || logical_member_count != 0
                || member_count_engine != 0
                || missing_member_count != 0
                || leader_trait_count != 0
                || location_province_id != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command deleted row {row_index} has live fields"
                ));
            }
        }
        validate_compact_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_PARENT) != 0,
            parent_id,
            "parent_id",
        );
        validate_compact_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0,
            owner_tag_db_id,
            "owner_tag_db_id",
        );
        validate_compact_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0,
            leader_id,
            "leader_id",
        );
        if (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0 {
            leader_rows = leader_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LOCATION) != 0 {
            location_rows = location_rows.saturating_add(1);
            if location_province_id == 0
                || location_province_id > COMPACT_OOB_MAX_LOCATION_PROVINCE_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid location_province_id {location_province_id}"
                ));
            }
        } else if location_province_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has location_province_id without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER_SKILL) != 0 {
            leader_skill_rows = leader_skill_rows.saturating_add(1);
            if leader_skill > COMPACT_OOB_MAX_LEADER_SKILL {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid leader_skill {leader_skill}"
                ));
            }
        } else if leader_skill != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has leader_skill without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE) != 0 {
            engine_count_rows = engine_count_rows.saturating_add(1);
        } else if member_count_engine != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has engine member count without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT) != 0 {
            missing_count_rows = missing_count_rows.saturating_add(1);
        } else if missing_member_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has missing member count without flag"
            ));
        }
        for (status_name, status_value) in [
            ("parent_status", parent_status),
            ("member_status", member_status),
            ("owner_status", owner_status),
            ("leader_status", leader_status),
            ("leader_trait_status", leader_trait_status),
        ] {
            if status_value > 5 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid {status_name} {status_value}"
                ));
            }
        }
        if row_member_count == 0 {
            if member_first != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has member_first without members"
                ));
            }
        } else {
            let Some(member_end) = member_first.checked_add(row_member_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range overflows"
                ));
                continue;
            };
            if member_end > member_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range {member_first}..{member_end} exceeds {member_count}"
                ));
            }
            declared_members = declared_members.saturating_add(row_member_count);
            if kind == COMPACT_OOB_KIND_ARMY {
                member_division_rows = member_division_rows.saturating_add(row_member_count);
            } else if kind == COMPACT_OOB_KIND_ARMY_GROUP {
                member_army_rows = member_army_rows.saturating_add(row_member_count);
            } else if kind == COMPACT_OOB_KIND_FLEET {
                member_task_force_rows = member_task_force_rows.saturating_add(row_member_count);
            } else if kind == COMPACT_OOB_KIND_TASK_FORCE {
                member_ship_rows = member_ship_rows.saturating_add(row_member_count);
            }
        }
        if member_status != 3 && row_member_count != logical_member_count {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} member_count {row_member_count} != logical_member_count {logical_member_count}"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} reserved field is nonzero"
            ));
        }
    }
    if declared_members != member_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} != declared {declared_members}"
        ));
    }
    add_compact_domain_data_unit(summary, 28, "oob_army_rows", emitted_army_rows);
    add_compact_domain_data_unit(summary, 28, "oob_army_group_rows", emitted_army_group_rows);
    add_compact_domain_data_unit(summary, 28, "oob_fleet_rows", emitted_fleet_rows);
    add_compact_domain_data_unit(summary, 28, "oob_task_force_rows", emitted_task_force_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_member_division_rows",
        member_division_rows,
    );
    add_compact_domain_data_unit(summary, 28, "oob_member_army_rows", member_army_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_member_task_force_rows",
        member_task_force_rows,
    );
    add_compact_domain_data_unit(summary, 28, "oob_member_ship_rows", member_ship_rows);
    add_compact_domain_data_unit(summary, 28, "oob_leader_rows", leader_rows);
    add_compact_domain_data_unit(summary, 28, "oob_leader_skill_rows", leader_skill_rows);
    add_compact_domain_data_unit(summary, 28, "oob_owner_rows", owner_rows);
    add_compact_domain_data_unit(summary, 28, "oob_location_rows", location_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_engine_member_count_rows",
        engine_count_rows,
    );
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_missing_member_count_rows",
        missing_count_rows,
    );

    let mut seen_members = BTreeSet::new();
    for row_index in 0..member_count {
        let member_id = read_u32_le(payload, cursor);
        let row_reserved = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_OOB_MEMBER_ROW_LEN;
        if !valid_compact_oob_id(member_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} has invalid member_id {member_id}"
            ));
        }
        if !seen_members.insert((row_index, member_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} duplicates member_id {member_id}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} reserved field is nonzero"
            ));
        }
    }
}

fn valid_compact_oob_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_OOB_MAX_ID
}

fn validate_compact_oob_optional_id(
    summary: &mut CompactSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if !valid_compact_oob_id(value) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid {field} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command row {row_index} has {field} without flag"
        ));
    }
}

fn decode_compact_ships_data_payload(offset: usize, payload: &[u8], summary: &mut CompactSummary) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_ship_rows = read_u32_le(payload, 20) as usize;
    let observed_task_force_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let reserved0 = read_u32_le(payload, 36);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_ship_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships row_count {row_count} > observed_ship_rows {observed_ship_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: ships scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships header reserved field is nonzero"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&29) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 29, "ship_rows", row_count);
    add_compact_domain_data_unit(summary, 29, "ship_observed_ship_rows", observed_ship_rows);
    add_compact_domain_data_unit(
        summary,
        29,
        "ship_observed_task_force_rows",
        observed_task_force_rows,
    );
    add_compact_domain_data_unit(summary, 29, "ship_deleted_rows", deleted_rows);

    let mut cursor = 40usize;
    let mut seen_rows = BTreeSet::new();
    let mut task_force_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut definition_rows = 0usize;
    for row_index in 0..row_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, COMPACT_SHIP_ROW_BASE_LEN, "ships row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let instance_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let task_force_id = read_u32_le(payload, cursor + 8);
        let category_enum = read_u32_le(payload, cursor + 12);
        let strength_raw = read_u32_le(payload, cursor + 16);
        let organisation_raw = read_u32_le(payload, cursor + 20);
        let experience_raw = read_u32_le(payload, cursor + 24);
        let sunk_convoys = read_u32_le(payload, cursor + 28);
        let owner_tag_db_id = read_u32_le(payload, cursor + 32);
        let row_reserved1 = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_SHIP_ROW_BASE_LEN;
        let definition_len = if cursor.saturating_add(2) <= payload.len() {
            read_u16_le(payload, cursor) as usize
        } else {
            0
        };
        if let Err(err) = skip_compact_string(payload, &mut cursor, "ships definition") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }

        let deleted = (flags & COMPACT_SHIP_ROW_FLAG_DELETED) != 0;
        if !valid_compact_ship_id(instance_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid instance_id {instance_id}"
            ));
        } else if !seen_rows.insert((instance_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} duplicates instance_id {instance_id}"
            ));
        }
        if flags
            & !(COMPACT_SHIP_ROW_FLAG_DELETED
                | COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                | COMPACT_SHIP_ROW_FLAG_DEFINITION
                | COMPACT_SHIP_ROW_FLAG_OWNER)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_TASK_FORCE) != 0 {
            task_force_rows = task_force_rows.saturating_add(1);
            if !valid_compact_oob_id(task_force_id) {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid task_force_id {task_force_id}"
                ));
            }
        } else if task_force_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has task_force_id without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_DEFINITION) != 0 {
            definition_rows = definition_rows.saturating_add(1);
            if definition_len == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has definition flag without definition"
                ));
            }
        } else if definition_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has definition without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
            if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_SHIP_MAX_OWNER_TAG_DB_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                ));
            }
        } else if owner_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has owner_tag_db_id without flag"
            ));
        }
        if category_enum > COMPACT_SHIP_MAX_CATEGORY {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid category_enum {category_enum}"
            ));
        }
        if strength_raw > COMPACT_SHIP_MAX_STRENGTH_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid strength_raw {strength_raw}"
            ));
        }
        if organisation_raw > COMPACT_SHIP_MAX_ORGANISATION_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid organisation_raw {organisation_raw}"
            ));
        }
        if experience_raw > COMPACT_SHIP_MAX_EXPERIENCE_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid experience_raw {experience_raw}"
            ));
        }
        if deleted
            && (task_force_id != 0
                || category_enum != 0
                || strength_raw != 0
                || organisation_raw != 0
                || experience_raw != 0
                || sunk_convoys != 0
                || owner_tag_db_id != 0
                || definition_len != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships deleted row {row_index} has live fields"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} reserved field is nonzero"
            ));
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(summary, 29, "ship_task_force_rows", task_force_rows);
    add_compact_domain_data_unit(summary, 29, "ship_owner_rows", owner_rows);
    add_compact_domain_data_unit(summary, 29, "ship_definition_rows", definition_rows);
}

fn valid_compact_ship_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_SHIP_MAX_ID
}

fn decode_compact_air_wings_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let deleted_rows = read_u32_le(payload, 24) as usize;
    let scale_raw = read_u32_le(payload, 28);
    let reserved0 = read_u32_le(payload, 32);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_air_wing_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings row_count {row_count} > observed_air_wing_rows {observed_air_wing_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings header reserved field is nonzero"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&30) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 30, "air_wing_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        30,
        "air_wing_observed_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(summary, 30, "air_wing_deleted_rows", deleted_rows);

    let mut cursor = 36usize;
    let mut seen_rows = BTreeSet::new();
    let mut mission_region_rows = 0usize;
    for row_index in 0..row_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, COMPACT_AIR_WING_ROW_LEN, "air wings row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let wing_id = read_u32_le(payload, cursor + 4);
        let generation = read_u64_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 16);
        let count = read_u32_le(payload, cursor + 20);
        let mission_mask = read_u32_le(payload, cursor + 24);
        let mission_region = read_u32_le(payload, cursor + 28);
        let effective_planes = read_u32_le(payload, cursor + 32);
        let reserved1 = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_AIR_WING_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_WING_ROW_FLAG_DELETED) != 0;
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if wing_id == 0 || wing_id > COMPACT_AIR_WING_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid wing_id {wing_id}"
            ));
        } else if !seen_rows.insert((owner_tag_db_id, wing_id, generation, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} duplicates owner/wing/generation"
            ));
        }
        if flags & !(COMPACT_AIR_WING_ROW_FLAG_DELETED | COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION) != 0 {
            mission_region_rows = mission_region_rows.saturating_add(1);
            if mission_region == 0 || mission_region > COMPACT_AIR_WING_MAX_MISSION_REGION_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wings row {row_index} has invalid mission_region {mission_region}"
                ));
            }
        } else if mission_region != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has mission_region without flag"
            ));
        }
        if count > COMPACT_AIR_WING_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid count {count}"
            ));
        }
        if effective_planes > COMPACT_AIR_WING_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid effective_planes {effective_planes}"
            ));
        }
        if deleted
            && (count != 0 || mission_mask != 0 || mission_region != 0 || effective_planes != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings deleted row {row_index} has live fields"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} reserved field is nonzero"
            ));
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        30,
        "air_wing_mission_region_rows",
        mission_region_rows,
    );
}

fn decode_compact_air_region_activity_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_bucket_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let reserved0 = read_u32_le(payload, 32);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_bucket_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity row_count {row_count} > observed_bucket_rows {observed_bucket_rows}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity header reserved field is nonzero"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&31) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 31, "air_region_activity_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_observed_bucket_rows",
        observed_bucket_rows,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_deleted_rows",
        deleted_rows,
    );

    let mut cursor = 36usize;
    let mut seen_rows = BTreeSet::new();
    let mut wing_count_sum = 0usize;
    let mut plane_count_sum = 0usize;
    let mut effective_planes_sum = 0usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_REGION_ACTIVITY_ROW_LEN,
            "air region activity row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let region_id = read_u32_le(payload, cursor);
        let owner_tag_db_id = read_u32_le(payload, cursor + 4);
        let mission_mask = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        let wing_count = read_u32_le(payload, cursor + 16);
        let plane_count = read_u32_le(payload, cursor + 20);
        let effective_planes = read_u32_le(payload, cursor + 24);
        let reserved1 = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_AIR_REGION_ACTIVITY_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED) != 0;
        if region_id == 0 || region_id > COMPACT_AIR_REGION_ACTIVITY_MAX_REGION_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid region_id {region_id}"
            ));
        }
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_REGION_ACTIVITY_MAX_OWNER_TAG_DB_ID
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if !seen_rows.insert((region_id, owner_tag_db_id, mission_mask)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} duplicates region/owner/mission"
            ));
        }
        if flags & !COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has unknown flags {flags}"
            ));
        }
        if wing_count > COMPACT_AIR_REGION_ACTIVITY_MAX_WING_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid wing_count {wing_count}"
            ));
        }
        if plane_count > COMPACT_AIR_REGION_ACTIVITY_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid plane_count {plane_count}"
            ));
        }
        if effective_planes > COMPACT_AIR_REGION_ACTIVITY_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid effective_planes {effective_planes}"
            ));
        }
        if deleted && (wing_count != 0 || plane_count != 0 || effective_planes != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity deleted row {row_index} has live counts"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} reserved field is nonzero"
            ));
        }
        wing_count_sum = wing_count_sum.saturating_add(wing_count as usize);
        plane_count_sum = plane_count_sum.saturating_add(plane_count as usize);
        effective_planes_sum = effective_planes_sum.saturating_add(effective_planes as usize);
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_wing_count_sum",
        wing_count_sum,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_plane_count_sum",
        plane_count_sum,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_effective_planes_sum",
        effective_planes_sum,
    );
}

fn decode_compact_air_wing_loadouts_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts data payload is {} byte(s), expected at least 44",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_equipment_rows = read_u32_le(payload, 24) as usize;
    let emitted_equipment_rows = read_u32_le(payload, 28) as usize;
    let deleted_rows = read_u32_le(payload, 32) as usize;
    let scale_raw = read_u32_le(payload, 36);
    let reserved0 = read_u32_le(payload, 40);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_air_wing_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts row_count {row_count} > observed_air_wing_rows {observed_air_wing_rows}"
        ));
    }
    if emitted_equipment_rows > observed_equipment_rows
        && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts emitted_equipment_rows {emitted_equipment_rows} > observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts header reserved field is nonzero"
        ));
    }
    let expected_min =
        44usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_WING_LOADOUT_ROW_LEN));
    if payload.len() < expected_min {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts data payload is {} byte(s), expected at least {expected_min}",
            payload.len()
        ));
        return;
    }
    let remaining_after_rows = payload.len().saturating_sub(expected_min);
    if remaining_after_rows % COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts equipment table has non-row trailing bytes"
        ));
        return;
    }
    let equipment_entry_count = remaining_after_rows / COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN;
    if equipment_entry_count != emitted_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts emitted_equipment_rows {emitted_equipment_rows} != payload entries {equipment_entry_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&32) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 32, "air_wing_loadout_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_observed_equipment_rows",
        observed_equipment_rows,
    );
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_equipment_rows",
        equipment_entry_count,
    );
    add_compact_domain_data_unit(summary, 32, "air_wing_loadout_deleted_rows", deleted_rows);

    let mut cursor = 44usize;
    let mut seen_rows = BTreeSet::new();
    let mut declared_entries = 0usize;
    let mut air_base_rows = 0usize;
    let mut equipment_rows_by_wing = BTreeMap::new();
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_WING_LOADOUT_ROW_LEN,
            "air wing loadouts row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let wing_id = read_u32_le(payload, cursor + 4);
        let generation = read_u64_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 16);
        let air_base_id = read_u32_le(payload, cursor + 20);
        let entry_first = read_u32_le(payload, cursor + 24) as usize;
        let entry_count = read_u32_le(payload, cursor + 28) as usize;
        cursor += COMPACT_AIR_WING_LOADOUT_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED) != 0;
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if wing_id == 0 || wing_id > COMPACT_AIR_WING_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} has invalid wing_id {wing_id}"
            ));
        } else if !seen_rows.insert((owner_tag_db_id, wing_id, generation, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} duplicates owner/wing/generation"
            ));
        }
        if flags & !COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} has unknown flags {flags}"
            ));
        }
        if deleted {
            if air_base_id != 0 || entry_first != 0 || entry_count != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts deleted row {row_index} has live fields"
                ));
            }
        } else {
            if air_base_id == 0 || air_base_id > COMPACT_AIR_WING_LOADOUT_MAX_AIR_BASE_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} has invalid air_base_id {air_base_id}"
                ));
            } else {
                air_base_rows = air_base_rows.saturating_add(1);
            }
            let Some(entry_end) = entry_first.checked_add(entry_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} equipment range overflows"
                ));
                continue;
            };
            if entry_end > equipment_entry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} equipment range {entry_first}+{entry_count} exceeds {equipment_entry_count}"
                ));
            }
            declared_entries = declared_entries.saturating_add(entry_count);
            equipment_rows_by_wing.insert(
                row_index,
                (
                    owner_tag_db_id,
                    wing_id,
                    generation,
                    entry_first,
                    entry_count,
                ),
            );
        }
    }
    if declared_entries != equipment_entry_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts equipment_entry_count {equipment_entry_count} != declared {declared_entries}"
        ));
    }
    add_compact_domain_data_unit(summary, 32, "air_wing_loadout_air_base_rows", air_base_rows);

    let entry_base =
        44usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_WING_LOADOUT_ROW_LEN));
    let mut seen_entries = BTreeSet::new();
    let mut amount_raw_sum = 0i128;
    for (row_index, (owner_tag_db_id, wing_id, generation, entry_first, entry_count)) in
        equipment_rows_by_wing
    {
        for child_index in entry_first..entry_first.saturating_add(entry_count) {
            let child_offset = entry_base
                .saturating_add(child_index.saturating_mul(COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN));
            if let Err(err) = require_compact_remaining(
                payload,
                child_offset,
                COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN,
                "air wing loadouts equipment row",
            ) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
            let variant_id = read_u32_le(payload, child_offset);
            let row_reserved = read_u32_le(payload, child_offset + 4);
            let amount_raw = read_i64_le(payload, child_offset + 8);
            if variant_id == 0 || variant_id > COMPACT_AIR_WING_LOADOUT_MAX_VARIANT_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts equipment row {child_index} has invalid variant_id {variant_id}"
                ));
            }
            if !(0..=COMPACT_AIR_WING_LOADOUT_MAX_AMOUNT_RAW).contains(&amount_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts equipment row {child_index} has invalid amount_raw {amount_raw}"
                ));
            }
            if row_reserved != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts equipment row {child_index} reserved field is nonzero"
                ));
            }
            if !seen_entries.insert((owner_tag_db_id, wing_id, generation, variant_id)) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} duplicates variant {variant_id}"
                ));
            }
            amount_raw_sum = amount_raw_sum.saturating_add(amount_raw as i128);
        }
    }
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_amount_raw_sum",
        amount_raw_sum.max(0).min(u64::MAX as i128) as usize,
    );
}

fn decode_compact_air_variant_defs_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs data payload is {} byte(s), expected at least 44",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let observed_module_rows = read_u32_le(payload, 24) as usize;
    let emitted_module_rows = read_u32_le(payload, 28) as usize;
    let invalid_variant_rows = read_u32_le(payload, 32) as usize;
    let reserved0 = read_u32_le(payload, 36);
    let reserved1 = read_u32_le(payload, 40);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_variant_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs row_count {row_count} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if emitted_module_rows > observed_module_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs emitted_module_rows {emitted_module_rows} > observed_module_rows {observed_module_rows}"
        ));
    }
    if invalid_variant_rows > observed_variant_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs invalid_variant_rows {invalid_variant_rows} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs header reserved field is nonzero"
        ));
    }
    let expected_rows_end =
        44usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_VARIANT_DEF_ROW_LEN));
    if payload.len() < expected_rows_end {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs data payload is {} byte(s), expected at least {expected_rows_end}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&33) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 33, "air_variant_def_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        33,
        "air_variant_def_observed_variant_rows",
        observed_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        33,
        "air_variant_def_observed_module_rows",
        observed_module_rows,
    );
    add_compact_domain_data_unit(
        summary,
        33,
        "air_variant_def_emitted_module_rows",
        emitted_module_rows,
    );
    add_compact_domain_data_unit(
        summary,
        33,
        "air_variant_def_invalid_variant_rows",
        invalid_variant_rows,
    );

    let mut cursor = 44usize;
    let mut rows = Vec::with_capacity(row_count);
    let mut seen_variants = BTreeSet::new();
    let mut declared_modules = 0usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_VARIANT_DEF_ROW_LEN,
            "air variant defs row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let module_first = read_u32_le(payload, cursor + 8) as usize;
        let module_count = read_u32_le(payload, cursor + 12) as usize;
        let row_reserved0 = read_u32_le(payload, cursor + 16);
        let row_reserved1 = read_u32_le(payload, cursor + 20);
        cursor += COMPACT_AIR_VARIANT_DEF_ROW_LEN;
        if variant_id == 0 || variant_id > COMPACT_AIR_VARIANT_DEF_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        let known_flags = COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME
            | COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME
            | COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved0 != 0 || row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} reserved fields are nonzero"
            ));
        }
        if module_count == 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has modules flag without modules"
            ));
        }
        if module_count > 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has modules without modules flag"
            ));
        }
        let Some(module_end) = module_first.checked_add(module_count) else {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} module range overflows"
            ));
            continue;
        };
        if module_end > emitted_module_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} module range {module_first}+{module_count} exceeds {emitted_module_rows}"
            ));
        }
        declared_modules = declared_modules.saturating_add(module_count);
        rows.push((flags, module_first, module_count));
    }
    if declared_modules != emitted_module_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs emitted_module_rows {emitted_module_rows} != declared {declared_modules}"
        ));
    }

    for (row_index, (flags, _, _)) in rows.iter().enumerate() {
        let label = "air variant defs display name";
        let display_len = match skip_compact_string_len(payload, &mut cursor, label) {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if display_len > COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} display name is too long ({display_len})"
            ));
        }
        if display_len == 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has display-name flag with empty string"
            ));
        }
        if display_len > 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has display name without flag"
            ));
        }
        let label = "air variant defs variant name";
        let variant_len = match skip_compact_string_len(payload, &mut cursor, label) {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if variant_len > COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} variant name is too long ({variant_len})"
            ));
        }
        if variant_len == 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has variant-name flag with empty string"
            ));
        }
        if variant_len > 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs row {row_index} has variant name without flag"
            ));
        }
    }

    let mut covered_modules = BTreeSet::new();
    for (row_index, (_, module_first, module_count)) in rows.iter().enumerate() {
        for module_index in *module_first..module_first.saturating_add(*module_count) {
            if !covered_modules.insert(module_index) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant defs row {row_index} overlaps module row {module_index}"
                ));
            }
        }
    }

    let mut nonempty_module_tokens = 0usize;
    for module_index in 0..emitted_module_rows {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 8, "air variant defs module row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let slot_handle = read_u32_le(payload, cursor);
        let module_reserved = read_u32_le(payload, cursor + 4);
        cursor += 8;
        if slot_handle == 0 || slot_handle > COMPACT_AIR_VARIANT_DEF_MAX_SLOT_HANDLE {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs module row {module_index} has invalid slot_handle {slot_handle}"
            ));
        }
        if module_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs module row {module_index} reserved field is nonzero"
            ));
        }
        let token_len =
            match skip_compact_string_len(payload, &mut cursor, "air variant defs module token") {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return;
                }
            };
        if token_len == 0 || token_len > COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant defs module row {module_index} has invalid token length {token_len}"
            ));
        } else {
            nonempty_module_tokens = nonempty_module_tokens.saturating_add(1);
        }
    }
    if covered_modules.len() != emitted_module_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs covered module rows {} != emitted_module_rows {emitted_module_rows}",
            covered_modules.len()
        ));
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant defs payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        33,
        "air_variant_def_module_token_rows",
        nonempty_module_tokens,
    );
}

fn decode_compact_air_variant_stats_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let invalid_variant_rows = read_u32_le(payload, 24) as usize;
    let fixed_scale_raw = read_u32_le(payload, 28);
    let range_scale_raw = read_u32_le(payload, 32);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_variant_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats row_count {row_count} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if invalid_variant_rows > observed_variant_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats invalid_variant_rows {invalid_variant_rows} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if fixed_scale_raw == 0 || range_scale_raw == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats scale fields must be nonzero"
        ));
    }
    let expected_len =
        36usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_VARIANT_STATS_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&35) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 35, "air_variant_stats_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_observed_variant_rows",
        observed_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_invalid_variant_rows",
        invalid_variant_rows,
    );

    let mut seen_variants = BTreeSet::new();
    let mut speed_cache_raw_sum = 0u128;
    let mut range_raw_sum = 0u128;
    let mut air_attack_raw_sum = 0u128;
    let mut agility_raw_sum = 0u128;
    let mut max_speed_raw_sum = 0u128;
    let mut reliability_raw_sum = 0u128;
    let mut air_defence_raw_sum = 0u128;
    let mut thrust_raw_sum = 0u128;
    let mut fuel_consumption_raw_sum = 0u128;
    let mut build_cost_ic_raw_sum = 0u128;
    let mut cursor = 36usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_VARIANT_STATS_ROW_LEN,
            "air variant stats row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let equipment_token_raw = read_u32_le(payload, cursor + 8);
        let equipment_type_token_raw = read_u32_le(payload, cursor + 12);
        let _interface_type_raw = read_u32_le(payload, cursor + 16);
        let speed_cache_raw = read_i32_le(payload, cursor + 20);
        let range_raw = read_i32_le(payload, cursor + 24);
        let air_attack_raw = read_i32_le(payload, cursor + 28);
        let agility_raw = read_i32_le(payload, cursor + 32);
        let max_speed_raw = read_i32_le(payload, cursor + 36);
        let reliability_raw = read_i32_le(payload, cursor + 40);
        let air_defence_raw = read_i32_le(payload, cursor + 44);
        let thrust_raw = read_i32_le(payload, cursor + 48);
        let fuel_consumption_raw = read_i32_le(payload, cursor + 52);
        let build_cost_ic_raw = read_i32_le(payload, cursor + 56);
        let row_reserved = read_u32_le(payload, cursor + 60);
        cursor += COMPACT_AIR_VARIANT_STATS_ROW_LEN;
        if variant_id == 0 || variant_id > COMPACT_AIR_VARIANT_STATS_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} has invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        if flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} reserved field is nonzero"
            ));
        }
        if equipment_token_raw == 0 || equipment_type_token_raw == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} has zero equipment token/type fields"
            ));
        }
        for (label, value) in [
            ("speed_cache_raw", speed_cache_raw),
            ("range_raw", range_raw),
            ("air_attack_raw", air_attack_raw),
            ("agility_raw", agility_raw),
            ("max_speed_raw", max_speed_raw),
            ("reliability_raw", reliability_raw),
            ("air_defence_raw", air_defence_raw),
            ("thrust_raw", thrust_raw),
            ("fuel_consumption_raw", fuel_consumption_raw),
            ("build_cost_ic_raw", build_cost_ic_raw),
        ] {
            if !(0..=COMPACT_AIR_VARIANT_STATS_MAX_RAW).contains(&value) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant stats row {row_index} has invalid {label} {value}"
                ));
            }
        }
        speed_cache_raw_sum = speed_cache_raw_sum.saturating_add(speed_cache_raw.max(0) as u128);
        range_raw_sum = range_raw_sum.saturating_add(range_raw.max(0) as u128);
        air_attack_raw_sum = air_attack_raw_sum.saturating_add(air_attack_raw.max(0) as u128);
        agility_raw_sum = agility_raw_sum.saturating_add(agility_raw.max(0) as u128);
        max_speed_raw_sum = max_speed_raw_sum.saturating_add(max_speed_raw.max(0) as u128);
        reliability_raw_sum = reliability_raw_sum.saturating_add(reliability_raw.max(0) as u128);
        air_defence_raw_sum = air_defence_raw_sum.saturating_add(air_defence_raw.max(0) as u128);
        thrust_raw_sum = thrust_raw_sum.saturating_add(thrust_raw.max(0) as u128);
        fuel_consumption_raw_sum =
            fuel_consumption_raw_sum.saturating_add(fuel_consumption_raw.max(0) as u128);
        build_cost_ic_raw_sum =
            build_cost_ic_raw_sum.saturating_add(build_cost_ic_raw.max(0) as u128);
    }
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_speed_cache_raw_sum",
        compact_sum_to_usize(speed_cache_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_range_raw_sum",
        compact_sum_to_usize(range_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_air_attack_raw_sum",
        compact_sum_to_usize(air_attack_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_agility_raw_sum",
        compact_sum_to_usize(agility_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_max_speed_raw_sum",
        compact_sum_to_usize(max_speed_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_reliability_raw_sum",
        compact_sum_to_usize(reliability_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_air_defence_raw_sum",
        compact_sum_to_usize(air_defence_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_thrust_raw_sum",
        compact_sum_to_usize(thrust_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_fuel_consumption_raw_sum",
        compact_sum_to_usize(fuel_consumption_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_build_cost_ic_raw_sum",
        compact_sum_to_usize(build_cost_ic_raw_sum),
    );
}

fn decode_compact_air_base_status_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_base_rows = read_u32_le(payload, 20) as usize;
    let observed_pool_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let reserved = read_u32_le(payload, 32);
    summary.status_flags_or |= status_flags;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_air_base_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status row_count {row_count} > observed_air_base_rows {observed_air_base_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status header reserved field is nonzero"
        ));
    }
    let expected_len =
        36usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_BASE_STATUS_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&34) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 34, "air_base_status_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_observed_air_base_rows",
        observed_air_base_rows,
    );
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_observed_pool_rows",
        observed_pool_rows,
    );
    add_compact_domain_data_unit(summary, 34, "air_base_status_deleted_rows", deleted_rows);

    let mut seen_air_bases = BTreeSet::new();
    let mut capacity_sum = 0usize;
    let mut based_wing_sum = 0usize;
    let mut based_plane_sum = 0usize;
    let mut cursor = 36usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_BASE_STATUS_ROW_LEN,
            "air base status row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let air_base_id = read_u32_le(payload, cursor);
        let state_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let capacity = read_u32_le(payload, cursor + 12);
        let building_level = read_u32_le(payload, cursor + 16);
        let based_wing_count = read_u32_le(payload, cursor + 20);
        let based_plane_count = read_u32_le(payload, cursor + 24);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_AIR_BASE_STATUS_ROW_LEN;
        let deleted = (flags & COMPACT_AIR_BASE_STATUS_ROW_FLAG_DELETED) != 0;
        if air_base_id == 0 || air_base_id > COMPACT_AIR_BASE_STATUS_MAX_AIR_BASE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid air_base_id {air_base_id}"
            ));
        } else if !seen_air_bases.insert(air_base_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} duplicates air_base_id {air_base_id}"
            ));
        }
        if state_id == 0 || state_id > COMPACT_AIR_BASE_STATUS_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid state_id {state_id}"
            ));
        }
        if flags & !COMPACT_AIR_BASE_STATUS_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} reserved field is nonzero"
            ));
        }
        if deleted {
            if capacity != 0
                || building_level != 0
                || based_wing_count != 0
                || based_plane_count != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: air base status deleted row {row_index} carries nonzero live fields"
                ));
            }
            continue;
        }
        if capacity > COMPACT_AIR_BASE_STATUS_MAX_CAPACITY {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid capacity {capacity}"
            ));
        }
        if building_level > COMPACT_AIR_BASE_STATUS_MAX_BUILDING_LEVEL {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid building_level {building_level}"
            ));
        }
        if based_wing_count > COMPACT_AIR_BASE_STATUS_MAX_WING_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid based_wing_count {based_wing_count}"
            ));
        }
        if based_plane_count > COMPACT_AIR_BASE_STATUS_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid based_plane_count {based_plane_count}"
            ));
        }
        if building_level == 0 && capacity != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has capacity {capacity} with building_level 0"
            ));
        }
        capacity_sum = capacity_sum.saturating_add(capacity as usize);
        based_wing_sum = based_wing_sum.saturating_add(based_wing_count as usize);
        based_plane_sum = based_plane_sum.saturating_add(based_plane_count as usize);
    }
    add_compact_domain_data_unit(summary, 34, "air_base_status_capacity_sum", capacity_sum);
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_based_wing_count_sum",
        based_wing_sum,
    );
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_based_plane_count_sum",
        based_plane_sum,
    );
}

fn decode_compact_land_combats_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let combat_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let participant_count = read_u32_le(payload, 20) as usize;
    let ended_count = read_u32_le(payload, 24) as usize;
    let reserved = read_u32_le(payload, 28);
    summary.status_flags_or |= status_flags;
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat data reserved field is nonzero"
        ));
    }
    let expected_len = 32usize
        .saturating_add(combat_count.saturating_mul(COMPACT_LAND_COMBAT_ROW_LEN))
        .saturating_add(participant_count.saturating_mul(COMPACT_LAND_COMBAT_PARTICIPANT_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > combat_count.saturating_add(ended_count) {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat changed_rows {changed_rows} > active+ended rows {}",
            combat_count.saturating_add(ended_count)
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&21) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(combat_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 21, "land_combat_rows", combat_count);
    add_compact_domain_data_unit(
        summary,
        21,
        "land_combat_participant_rows",
        participant_count,
    );
    add_compact_domain_data_unit(summary, 21, "land_combat_ended_rows", ended_count);
    let mut cursor = 32usize;
    let mut seen_combats = BTreeSet::new();
    let mut declared_participants = 0usize;
    let mut attacker_rows = 0usize;
    let mut defender_rows = 0usize;
    for row_index in 0..combat_count {
        let combat_id = read_u32_le(payload, cursor);
        let generation = read_u64_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 12);
        let location = read_u32_le(payload, cursor + 16);
        let day = read_u32_le(payload, cursor + 20);
        let duration = read_u32_le(payload, cursor + 24);
        let attacker_count = read_u32_le(payload, cursor + 28) as usize;
        let defender_count = read_u32_le(payload, cursor + 32) as usize;
        let row_reserved = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_LAND_COMBAT_ROW_LEN;

        if combat_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has zero combat_id"
            ));
        } else if !seen_combats.insert((combat_id, generation)) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} duplicates combat {combat_id}:{generation}"
            ));
        }
        if flags & !0b1_1111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) != 0 && !valid_compact_land_combat_province_id(location) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has invalid location {location}"
            ));
        }
        if (flags & 1) == 0 && location != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has location without flag"
            ));
        }
        if (flags & (1 << 1)) != 0 && day > COMPACT_LAND_COMBAT_MAX_AGE_DAYS {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has invalid day {day}"
            ));
        }
        if (flags & (1 << 1)) == 0 && day != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has day without flag"
            ));
        }
        if (flags & (1 << 2)) != 0 && duration > COMPACT_LAND_COMBAT_MAX_AGE_DAYS {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has invalid duration {duration}"
            ));
        }
        if (flags & (1 << 2)) == 0 && duration != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has duration without flag"
            ));
        }
        if (flags & (1 << 3)) == 0 && attacker_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has attackers without exact flag"
            ));
        }
        if (flags & (1 << 4)) == 0 && defender_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has defenders without exact flag"
            ));
        }
        declared_participants =
            declared_participants.saturating_add(attacker_count.saturating_add(defender_count));
        attacker_rows = attacker_rows.saturating_add(attacker_count);
        defender_rows = defender_rows.saturating_add(defender_count);
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} reserved field is nonzero"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 21, "land_combat_attacker_rows", attacker_rows);
    add_compact_domain_data_unit(summary, 21, "land_combat_defender_rows", defender_rows);
    if declared_participants != participant_count {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat participant_count {participant_count} != declared {declared_participants}"
        ));
    }
    let mut seen_participants = BTreeSet::new();
    for row_index in 0..participant_count {
        let combat_id = read_u32_le(payload, cursor);
        let generation = read_u64_le(payload, cursor + 4);
        let side = read_u32_le(payload, cursor + 12);
        let division_id = read_u32_le(payload, cursor + 16);
        let row_reserved = read_u32_le(payload, cursor + 20);
        cursor += COMPACT_LAND_COMBAT_PARTICIPANT_ROW_LEN;
        if !seen_combats.contains(&(combat_id, generation)) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} references unknown combat {combat_id}:{generation}"
            ));
        }
        if side != 1 && side != 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} has invalid side {side}"
            ));
        }
        if division_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} has zero division_id"
            ));
        }
        if !seen_participants.insert((combat_id, generation, side, division_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} duplicates participant"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} reserved field is nonzero"
            ));
        }
    }
}

fn decode_compact_map_control_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: map control data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let state_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let changed_state_rows = read_u32_le(payload, 24) as usize;
    let changed_province_rows = read_u32_le(payload, 28) as usize;
    summary.status_flags_or |= status_flags;
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    let expected_len = 32usize
        .saturating_add(state_count.saturating_mul(COMPACT_MAP_STATE_ROW_LEN))
        .saturating_add(province_count.saturating_mul(COMPACT_MAP_PROVINCE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: map control data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if !delta_data && state_count < COMPACT_MAP_MIN_STATE_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: map control state_count {state_count} < {COMPACT_MAP_MIN_STATE_ROWS}"
        ));
    }
    if !delta_data && province_count < COMPACT_MAP_MIN_PROVINCE_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: map control province_count {province_count} < {COMPACT_MAP_MIN_PROVINCE_ROWS}"
        ));
    }
    if changed_state_rows > state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control changed_state_rows {changed_state_rows} > state_count {state_count}"
        ));
    }
    if changed_province_rows > province_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control changed_province_rows {changed_province_rows} > province_count {province_count}"
        ));
    }
    if changed_rows != changed_state_rows.saturating_add(changed_province_rows) {
        summary.parse_errors.push(format!(
            "offset {offset}: map control changed_rows {changed_rows} != changed state+province rows {}",
            changed_state_rows.saturating_add(changed_province_rows)
        ));
    }
    if delta_data && changed_state_rows != state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control delta changed_state_rows {changed_state_rows} != emitted state_count {state_count}"
        ));
    }
    if delta_data && changed_province_rows != province_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control delta changed_province_rows {changed_province_rows} != emitted province_count {province_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&22) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(state_count.saturating_add(province_count) as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 22, "map_control_state_rows", state_count);
    add_compact_domain_data_unit(summary, 22, "map_control_province_rows", province_count);
    add_compact_domain_data_unit(
        summary,
        22,
        "map_control_changed_state_rows",
        changed_state_rows,
    );
    add_compact_domain_data_unit(
        summary,
        22,
        "map_control_changed_province_rows",
        changed_province_rows,
    );

    let mut cursor = 32usize;
    let mut seen_states = BTreeSet::new();
    for row_index in 0..state_count {
        let state_id = read_u32_le(payload, cursor);
        let owner_tag_db_id = read_u32_le(payload, cursor + 4);
        let controller_tag_db_id = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        cursor += COMPACT_MAP_STATE_ROW_LEN;
        if state_id == 0 || state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has invalid state_id {state_id}"
            ));
        } else if !seen_states.insert(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} duplicates state_id {state_id}"
            ));
        }
        if owner_tag_db_id > COMPACT_MAP_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has invalid owner tag {owner_tag_db_id}"
            ));
        }
        if controller_tag_db_id > COMPACT_MAP_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has invalid controller tag {controller_tag_db_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) == 0 && owner_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has owner without flag"
            ));
        }
        if (flags & 1) != 0 && owner_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has owner flag without owner"
            ));
        }
        if (flags & (1 << 1)) == 0 && controller_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has controller without flag"
            ));
        }
        if (flags & (1 << 1)) != 0 && controller_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has controller flag without controller"
            ));
        }
    }

    let mut seen_provinces = BTreeSet::new();
    for row_index in 0..province_count {
        let province_id = read_u32_le(payload, cursor);
        let controller_tag_db_id = read_u32_le(payload, cursor + 4);
        let state_id = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        cursor += COMPACT_MAP_PROVINCE_ROW_LEN;
        if province_id == 0 || province_id > COMPACT_MAP_MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has invalid province_id {province_id}"
            ));
        } else if !seen_provinces.insert(province_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if controller_tag_db_id > COMPACT_MAP_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has invalid controller tag {controller_tag_db_id}"
            ));
        }
        if state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has invalid state_id {state_id}"
            ));
        }
        if !delta_data && state_id != 0 && !seen_states.contains(&state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} references unknown state_id {state_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) == 0 && controller_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has controller without flag"
            ));
        }
        if (flags & 1) != 0 && controller_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has controller flag without controller"
            ));
        }
        if (flags & (1 << 1)) == 0 && state_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has state without flag"
            ));
        }
        if (flags & (1 << 1)) != 0 && state_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has state flag without state"
            ));
        }
    }
}

fn decode_compact_map_detail_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let state_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let observed_state_rows = read_u32_le(payload, 24) as usize;
    let observed_province_rows = read_u32_le(payload, 28) as usize;
    let resource_count = read_u32_le(payload, 32) as usize;
    let state_building_count = read_u32_le(payload, 36) as usize;
    let province_building_count = read_u32_le(payload, 40) as usize;
    let reserved = read_u32_le(payload, 44);
    summary.status_flags_or |= status_flags;
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    let expected_len = 48usize
        .saturating_add(state_count.saturating_mul(COMPACT_MAP_DETAIL_STATE_ROW_LEN))
        .saturating_add(province_count.saturating_mul(COMPACT_MAP_DETAIL_PROVINCE_ROW_LEN))
        .saturating_add(resource_count.saturating_mul(COMPACT_MAP_DETAIL_ENTRY_ROW_LEN))
        .saturating_add(state_building_count.saturating_mul(COMPACT_MAP_DETAIL_ENTRY_ROW_LEN))
        .saturating_add(province_building_count.saturating_mul(COMPACT_MAP_DETAIL_ENTRY_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail reserved field is {reserved}"
        ));
    }
    if !delta_data && state_count < COMPACT_MAP_MIN_STATE_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail state_count {state_count} < {COMPACT_MAP_MIN_STATE_ROWS}"
        ));
    }
    if province_count > 0 && !delta_data && province_count < COMPACT_MAP_MIN_PROVINCE_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail province_count {province_count} < {COMPACT_MAP_MIN_PROVINCE_ROWS}"
        ));
    }
    if state_count > observed_state_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail state_count {state_count} > observed_state_rows {observed_state_rows}"
        ));
    }
    if province_count > observed_province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail province_count {province_count} > observed_province_rows {observed_province_rows}"
        ));
    }
    let emitted_rows = state_count.saturating_add(province_count);
    if changed_rows > emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail changed_rows {changed_rows} > emitted rows {emitted_rows}"
        ));
    }
    if delta_data && changed_rows != emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail delta changed_rows {changed_rows} != emitted rows {emitted_rows}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&23) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(emitted_rows as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 23, "map_detail_state_rows", state_count);
    add_compact_domain_data_unit(summary, 23, "map_detail_province_rows", province_count);
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_observed_state_rows",
        observed_state_rows,
    );
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_observed_province_rows",
        observed_province_rows,
    );
    add_compact_domain_data_unit(summary, 23, "map_detail_resource_rows", resource_count);
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_state_building_rows",
        state_building_count,
    );
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_province_building_rows",
        province_building_count,
    );

    let mut cursor = 48usize;
    let mut seen_states = BTreeSet::new();
    for row_index in 0..state_count {
        let state_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let resource_first = read_u32_le(payload, cursor + 8) as usize;
        let resource_len = read_u32_le(payload, cursor + 12) as usize;
        let building_first = read_u32_le(payload, cursor + 16) as usize;
        let building_len = read_u32_le(payload, cursor + 20) as usize;
        let row_reserved = read_u32_le(payload, cursor + 24);
        cursor += COMPACT_MAP_DETAIL_STATE_ROW_LEN;
        if state_id == 0 || state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} has invalid state_id {state_id}"
            ));
        } else if !seen_states.insert(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} duplicates state_id {state_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} reserved field is {row_reserved}"
            ));
        }
        validate_compact_index_range(
            offset,
            summary,
            "map detail state resource",
            row_index,
            flags & 1 != 0,
            resource_first,
            resource_len,
            resource_count,
        );
        validate_compact_index_range(
            offset,
            summary,
            "map detail state building",
            row_index,
            flags & (1 << 1) != 0,
            building_first,
            building_len,
            state_building_count,
        );
    }

    let mut seen_provinces = BTreeSet::new();
    for row_index in 0..province_count {
        let province_id = read_u32_le(payload, cursor);
        let state_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let building_first = read_u32_le(payload, cursor + 12) as usize;
        let building_len = read_u32_le(payload, cursor + 16) as usize;
        cursor += COMPACT_MAP_DETAIL_PROVINCE_ROW_LEN;
        if province_id == 0 || province_id > COMPACT_MAP_MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has invalid province_id {province_id}"
            ));
        } else if !seen_provinces.insert(province_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has invalid state_id {state_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) == 0 && state_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has state without flag"
            ));
        }
        if (flags & 1) != 0 && state_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has state flag without state"
            ));
        }
        validate_compact_index_range(
            offset,
            summary,
            "map detail province building",
            row_index,
            flags & (1 << 1) != 0,
            building_first,
            building_len,
            province_building_count,
        );
    }

    for row_index in 0..resource_count {
        let resource_id = read_u32_le(payload, cursor);
        let amount_raw = read_i32_le(payload, cursor + 4);
        let entry_reserved = read_u32_le(payload, cursor + 8);
        cursor += COMPACT_MAP_DETAIL_ENTRY_ROW_LEN;
        if resource_id == 0 || resource_id == u32::MAX {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail resource row {row_index} has invalid resource_id {resource_id}"
            ));
        }
        if !(0..=COMPACT_MAP_MAX_RESOURCE_AMOUNT_RAW).contains(&amount_raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail resource row {row_index} has invalid amount_raw {amount_raw}"
            ));
        }
        if entry_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail resource row {row_index} reserved field is {entry_reserved}"
            ));
        }
    }
    for row_index in 0..state_building_count {
        decode_compact_map_detail_building_entry(
            offset,
            payload,
            &mut cursor,
            summary,
            "state",
            row_index,
        );
    }
    for row_index in 0..province_building_count {
        decode_compact_map_detail_building_entry(
            offset,
            payload,
            &mut cursor,
            summary,
            "province",
            row_index,
        );
    }
}

fn decode_compact_weather_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let region_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let observed_region_rows = read_u32_le(payload, 24) as usize;
    let observed_province_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let active_region_count = read_u32_le(payload, 36) as usize;
    let reserved0 = read_u32_le(payload, 40);
    let reserved1 = read_u32_le(payload, 44);
    summary.status_flags_or |= status_flags;
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    let expected_len =
        48usize.saturating_add(region_count.saturating_mul(COMPACT_WEATHER_REGION_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: weather data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if province_count != 0 || observed_province_rows != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province rows are not supported in this schema ({province_count}/{observed_province_rows})"
        ));
    }
    if observed_region_rows != COMPACT_WEATHER_REGION_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: weather observed_region_rows {observed_region_rows} != {COMPACT_WEATHER_REGION_ROWS}"
        ));
    }
    if !delta_data && region_count != COMPACT_WEATHER_REGION_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: weather full snapshot region_count {region_count} != {COMPACT_WEATHER_REGION_ROWS}"
        ));
    }
    if region_count > observed_region_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region_count {region_count} > observed_region_rows {observed_region_rows}"
        ));
    }
    if changed_rows > region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: weather changed_rows {changed_rows} > region_count {region_count}"
        ));
    }
    if delta_data && changed_rows != region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: weather delta changed_rows {changed_rows} != emitted region_count {region_count}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: weather scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if active_region_count > region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: weather active_region_count {active_region_count} > region_count {region_count}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather reserved fields are nonzero ({reserved0}, {reserved1})"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&24) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(region_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 24, "weather_region_rows", region_count);
    add_compact_domain_data_unit(
        summary,
        24,
        "weather_observed_region_rows",
        observed_region_rows,
    );
    add_compact_domain_data_unit(
        summary,
        24,
        "weather_active_region_rows",
        active_region_count,
    );

    let mut cursor = 48usize;
    let mut seen_regions = BTreeSet::new();
    let mut computed_active_regions = 0usize;
    for row_index in 0..region_count {
        let region_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let temperature_raw = read_i32_le(payload, cursor + 8);
        let phenomenon = read_u32_le(payload, cursor + 12);
        let _next_weather_change_raw = read_u32_le(payload, cursor + 16);
        let active_modifier_count = read_u32_le(payload, cursor + 20);
        let intensity = read_u32_le(payload, cursor + 24);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_WEATHER_REGION_ROW_LEN;
        if region_id > COMPACT_WEATHER_MAX_REGION_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid region_id {region_id}"
            ));
        } else if !seen_regions.insert(region_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} duplicates region_id {region_id}"
            ));
        }
        if flags & !0x7f != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has unknown flags {flags}"
            ));
        }
        let active_flag = (flags & (1 << 5)) != 0;
        let phenomenon_flag = (flags & (1 << 6)) != 0;
        if active_flag != phenomenon_flag {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has mismatched active flags"
            ));
        }
        if phenomenon >= 1 {
            computed_active_regions = computed_active_regions.saturating_add(1);
            if !active_flag {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather row {row_index} has active phenomenon without active flag"
                ));
            }
        } else if active_flag {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has active flag without phenomenon"
            ));
        }
        if phenomenon > COMPACT_WEATHER_MAX_PHENOMENON {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid phenomenon {phenomenon}"
            ));
        }
        if !(-COMPACT_WEATHER_MAX_TEMPERATURE_RAW..=COMPACT_WEATHER_MAX_TEMPERATURE_RAW)
            .contains(&temperature_raw)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid temperature_raw {temperature_raw}"
            ));
        }
        if active_modifier_count > COMPACT_WEATHER_MAX_ACTIVE_MODIFIERS {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid active_modifier_count {active_modifier_count}"
            ));
        }
        if intensity > u8::MAX as u32 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid intensity {intensity}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} reserved field is {row_reserved}"
            ));
        }
    }
    if computed_active_regions != active_region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: weather active_region_count {active_region_count} != computed {computed_active_regions}"
        ));
    }
}

fn validate_compact_index_range(
    offset: usize,
    summary: &mut CompactSummary,
    label: &str,
    row_index: usize,
    present: bool,
    first: usize,
    len: usize,
    total: usize,
) {
    if !present {
        if first != 0 || len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has range without presence flag"
            ));
        }
        return;
    }
    if first.saturating_add(len) > total {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} row {row_index} range {first}+{len} exceeds {total}"
        ));
    }
}

fn decode_compact_map_detail_building_entry(
    offset: usize,
    payload: &[u8],
    cursor: &mut usize,
    summary: &mut CompactSummary,
    scope: &str,
    row_index: usize,
) {
    let type_id = read_u32_le(payload, *cursor);
    let level = read_u32_le(payload, *cursor + 4);
    let entry_reserved = read_u32_le(payload, *cursor + 8);
    *cursor += COMPACT_MAP_DETAIL_ENTRY_ROW_LEN;
    if type_id == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail {scope} building row {row_index} has zero type_id"
        ));
    }
    if level > COMPACT_MAP_MAX_BUILDING_LEVEL {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail {scope} building row {row_index} has invalid level {level}"
        ));
    }
    if entry_reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail {scope} building row {row_index} reserved field is {entry_reserved}"
        ));
    }
}

fn valid_compact_land_combat_province_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_LAND_COMBAT_MAX_PROVINCE_ID
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn require_compact_remaining(
    bytes: &[u8],
    cursor: usize,
    needed: usize,
    label: &str,
) -> Result<(), String> {
    if cursor.saturating_add(needed) > bytes.len() {
        Err(format!(
            "{label} overruns payload at cursor {cursor}: need {needed}, remaining {}",
            bytes.len().saturating_sub(cursor)
        ))
    } else {
        Ok(())
    }
}

fn skip_compact_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    require_compact_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_compact_remaining(bytes, *cursor, len, label)?;
    let start = *cursor;
    *cursor += len;
    Ok(start)
}

fn skip_compact_string_len(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    require_compact_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_compact_remaining(bytes, *cursor, len, label)?;
    *cursor += len;
    Ok(len)
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

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes([
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

fn compact_domain_names_from_ids(domains: &[u16]) -> Vec<String> {
    domains
        .iter()
        .map(|domain| compact_domain_name(*domain).to_string())
        .collect()
}

fn compact_expected_domain_ids(scheduled_domains: &[String]) -> Vec<u16> {
    let mut ids = if scheduled_domains.is_empty() {
        vec![10, 11, 12, 13, 14, 15, 16, 17]
    } else {
        scheduled_domains
            .iter()
            .filter_map(|domain| compact_domain_id(domain))
            .collect::<Vec<_>>()
    };
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn compact_domain_id(domain: &str) -> Option<u16> {
    match domain {
        "fuel_live" => Some(10),
        "production_live" => Some(11),
        "focus_live" => Some(12),
        "tech_live" => Some(13),
        "politics_live" => Some(14),
        "politics_residuals" => Some(15),
        "decisions_live" => Some(16),
        "diplomacy_live" => Some(17),
        "divisions" => Some(20),
        "land_combats" => Some(21),
        "map_control" | "state_ownership" | "state_control" | "province_control" => Some(22),
        "map_detail" | "state_detail" | "buildings_live" => Some(23),
        "weather_live" => Some(24),
        "division_equipment" => Some(26),
        "army_orders" => Some(27),
        "oob_command" => Some(28),
        "ships" => Some(29),
        "air_wings" => Some(30),
        "air_region_activity" => Some(31),
        "air_wing_loadouts" => Some(32),
        "air_variant_defs" => Some(33),
        "air_base_status" => Some(34),
        "air_variant_stats" => Some(35),
        _ => None,
    }
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
        20 => "divisions",
        21 => "land_combats",
        22 => "map_control",
        23 => "map_detail",
        24 => "weather_live",
        26 => "division_equipment",
        27 => "army_orders",
        28 => "oob_command",
        29 => "ships",
        30 => "air_wings",
        31 => "air_region_activity",
        32 => "air_wing_loadouts",
        33 => "air_variant_defs",
        34 => "air_base_status",
        35 => "air_variant_stats",
        1000 => "frame",
        _ => "unknown",
    }
}

fn compact_domain_coverage_errors(
    summary: &CompactSummary,
    expected_domains: &[u16],
) -> Vec<String> {
    if expected_domains.is_empty() {
        return Vec::new();
    }
    let mut errors = Vec::new();
    for hour in &summary.unique_hours {
        let present = summary.per_hour_domains.get(hour);
        for expected in expected_domains {
            if !present
                .map(|domains| domains.contains(expected))
                .unwrap_or(false)
            {
                errors.push(format!(
                    "game_hour_raw {hour}: missing compact domain {}",
                    compact_domain_name(*expected)
                ));
            }
        }
    }
    for expected in expected_domains {
        let record_count = summary
            .per_domain
            .get(expected)
            .map(|domain| domain.record_count)
            .unwrap_or(0);
        if record_count != summary.frame_end_record_count {
            errors.push(format!(
                "domain {} record_count {} != frame_end_record_count {}",
                compact_domain_name(*expected),
                record_count,
                summary.frame_end_record_count
            ));
        }
        if compact_domain_requires_data_record(*expected) {
            let data_record_count = summary
                .per_domain
                .get(expected)
                .map(|domain| domain.data_record_count)
                .unwrap_or(0);
            if data_record_count != summary.frame_end_record_count {
                errors.push(format!(
                    "domain {} data_record_count {} != frame_end_record_count {}",
                    compact_domain_name(*expected),
                    data_record_count,
                    summary.frame_end_record_count
                ));
            }
        }
    }
    errors
}

fn compact_domain_requires_data_record(domain_id: u16) -> bool {
    matches!(
        domain_id,
        10 | 11
            | 12
            | 13
            | 14
            | 15
            | 16
            | 17
            | 20
            | 21
            | 22
            | 23
            | 24
            | 26
            | 27
            | 28
            | 29
            | 30
            | 31
            | 32
            | 33
            | 34
            | 35
    )
}

fn native_division_root_prewarm_budget_ms(_cfg: &Config) -> u64 {
    0
}

fn duration_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn compact_division_root_source(cfg: &Config, summary: &CompactSummary) -> &'static str {
    if cfg.division_body_bridge.is_some() {
        return "python_bridge";
    }
    if !cfg.scheduled_domains.iter().any(|domain| {
        domain == "divisions"
            || domain == "division_equipment"
            || domain == "army_orders"
            || domain == "oob_command"
    }) {
        return "not_scheduled";
    }
    let Some(domain) = summary
        .per_domain
        .get(&20)
        .or_else(|| summary.per_domain.get(&26))
        .or_else(|| summary.per_domain.get(&27))
        .or_else(|| summary.per_domain.get(&28))
    else {
        return "unseen";
    };
    let blocked = (domain.status_flags_or
        & (COMPACT_STATUS_BRIDGE_UNAVAILABLE
            | COMPACT_STATUS_SOURCE_UNAVAILABLE
            | COMPACT_STATUS_WORKER_PANIC))
        != 0;
    if !blocked && domain.data_rows_total > 0 {
        "dll_native"
    } else {
        "unresolved"
    }
}

fn compact_division_root_discovery_debt(source: &str) -> &'static str {
    match source {
        "dll_native" => {
            "none; compact stream used a DLL-native anchored division root"
        }
        "python_bridge" => {
            "temporary bounded preflight/cache input; not DLL-complete root discovery"
        }
        "not_scheduled" => "not_scheduled",
        "unseen" => "divisions domain was scheduled but no compact division records were seen",
        _ => "anchored division root/container did not resolve; heap scans are excluded from production capture",
    }
}

fn compact_ship_root_source(cfg: &Config, summary: &CompactSummary) -> &'static str {
    if !cfg.scheduled_domains.iter().any(|domain| domain == "ships") {
        return "not_scheduled";
    }
    let Some(domain) = summary.per_domain.get(&29) else {
        return "unseen";
    };
    let blocked = (domain.status_flags_or
        & (COMPACT_STATUS_BRIDGE_UNAVAILABLE
            | COMPACT_STATUS_SOURCE_UNAVAILABLE
            | COMPACT_STATUS_PARTIAL_DATA
            | COMPACT_STATUS_WORKER_PANIC))
        != 0;
    if !blocked && domain.data_rows_total > 0 {
        "dll_native"
    } else {
        "unresolved"
    }
}

fn compact_ship_root_discovery_debt(source: &str) -> &'static str {
    match source {
        "dll_native" => "none; compact stream used a DLL-native anchored CTaskForce/CShip root",
        "not_scheduled" => "not_scheduled",
        "unseen" => "ships domain was scheduled but no compact ship records were seen",
        _ => "anchored CTaskForce/CShip root/container did not resolve; ship bridges and heap scans are excluded from production capture",
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
                "data_record_count": domain.data_record_count,
                "row_count_total": domain.row_count_total,
                "changed_rows_total": domain.changed_rows_total,
                "data_rows_total": domain.data_rows_total,
                "data_changed_rows_total": domain.data_changed_rows_total,
                "data_units_total": domain.data_units_total,
                "unit_count_total": domain.unit_count_total,
                "changed_unit_count_total": domain.changed_unit_count_total,
                "status_flags_or": domain.status_flags_or,
                "read_time_us": timing_summary_json(&domain.read_times_us),
            }),
        );
    }
    Value::Object(out)
}

fn add_compact_domain_data_unit(
    summary: &mut CompactSummary,
    domain_id: u16,
    name: &str,
    count: usize,
) {
    if count == 0 {
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&domain_id) {
        let entry = domain.data_units_total.entry(name.to_string()).or_default();
        *entry = entry.saturating_add(count as u64);
    }
}

fn compact_sum_to_usize(value: u128) -> usize {
    value.min(usize::MAX as u128) as usize
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

fn compact_hourly_problems(
    summary: &CompactSummary,
    min_game_hours: i32,
    coverage_errors: &[String],
) -> Vec<String> {
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
    if (summary.status_flags_or & COMPACT_STATUS_WORKER_PANIC) != 0 {
        problems.push("compact_worker_panic_seen".to_string());
    }
    if (summary.status_flags_or & COMPACT_STATUS_BRIDGE_UNAVAILABLE) != 0 {
        problems.push("compact_bridge_unavailable".to_string());
    }
    if (summary.status_flags_or & COMPACT_STATUS_SOURCE_UNAVAILABLE) != 0 {
        problems.push("compact_source_unavailable".to_string());
    }
    if (summary.status_flags_or & COMPACT_STATUS_PARTIAL_DATA) != 0 {
        problems.push("compact_partial_data".to_string());
    }
    if !coverage_errors.is_empty() {
        problems.push("scheduled_domain_coverage_failed".to_string());
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
            10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_fuel_data_payload(&[(47, 4_046_848, 14_950_400, 245_760, -73_728)]),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );
        append_test_compact_record(
            &mut raw,
            4,
            101,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(1_500, 355, 10, 40, 40),
        );
        append_test_compact_record(
            &mut raw,
            5,
            101,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_fuel_data_payload(&[(47, 3_932_160, 14_950_400, 65_536, -32_768)]),
        );
        append_test_compact_record(
            &mut raw,
            6,
            101,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_250, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(summary.parse_errors.is_empty());
        assert!(summary.ordering_errors.is_empty());
        assert_eq!(summary.record_count, 6);
        assert_eq!(summary.domain_perf_record_count, 2);
        assert_eq!(summary.domain_data_record_count, 2);
        assert_eq!(summary.frame_end_record_count, 2);
        assert_eq!(summary.unique_hours, vec![100, 101]);
        assert_eq!(summary.skipped_hours, 0);
        assert_eq!(summary.total_bytes, raw.len());
        assert!(compact_domain_coverage_errors(&summary, &[10]).is_empty());
        let fuel = summary.per_domain.get(&10).unwrap();
        assert_eq!(fuel.record_count, 2);
        assert_eq!(fuel.row_count_total, 710);
        assert_eq!(fuel.changed_rows_total, 22);
        assert_eq!(fuel.data_record_count, 2);
        assert_eq!(fuel.data_rows_total, 2);
        assert_eq!(fuel.data_changed_rows_total, 2);
        assert_eq!(fuel.unit_count_total, 84);
        assert_eq!(fuel.max_read_time_us, 1_500);
    }

    #[test]
    fn compact_coverage_rejects_alternating_domain_hours() {
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
            11,
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
        let errors = compact_domain_coverage_errors(&summary, &[10, 11]);

        assert_eq!(summary.skipped_hours, 0);
        assert!(errors
            .iter()
            .any(|err| err.contains("missing compact domain production_live")));
        assert!(errors
            .iter()
            .any(|err| err.contains("missing compact domain fuel_live")));
    }

    #[test]
    fn compact_summary_surfaces_worker_panic_status() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(0, 0, 0, 0, 0, COMPACT_STATUS_WORKER_PANIC),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert_ne!(summary.status_flags_or & COMPACT_STATUS_WORKER_PANIC, 0);
        assert_ne!(
            summary.per_domain.get(&10).unwrap().status_flags_or & COMPACT_STATUS_WORKER_PANIC,
            0
        );
    }

    #[test]
    fn compact_coverage_requires_fuel_data_record_per_frame() {
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

        let summary = summarize_compact_stream(&raw);
        let errors = compact_domain_coverage_errors(&summary, &[10]);

        assert!(errors
            .iter()
            .any(|err| err.contains("fuel_live data_record_count")));
    }

    #[test]
    fn compact_coverage_requires_division_equipment_data_record_per_frame() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            26,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(900, 2, 2, 3, 3),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);
        let errors = compact_domain_coverage_errors(&summary, &[26]);

        assert!(errors
            .iter()
            .any(|err| err.contains("division_equipment data_record_count")));
    }

    #[test]
    fn compact_fuel_data_rejects_duplicate_tag_ids() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_fuel_data_payload(&[
                (47, 4_046_848, 14_950_400, 245_760, -73_728),
                (47, 3_932_160, 14_950_400, 65_536, -32_768),
            ]),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("duplicates tag_db_id 47")));
    }

    #[test]
    fn compact_politics_data_accepts_popularity_only_rows() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            14,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_politics_popularity_delta_payload(),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let politics = summary.per_domain.get(&14).unwrap();
        assert_eq!(politics.data_record_count, 1);
        assert_eq!(politics.data_rows_total, 1);
        assert_eq!(politics.data_changed_rows_total, 1);
    }

    #[test]
    fn compact_summary_decodes_widened_country_data_records() {
        let mut raw = Vec::new();
        let mut sequence = 1u64;
        for (domain_id, payload) in [
            (
                10u16,
                test_fuel_data_payload(&[(47, 4_046_848, 14_950_400, 245_760, -73_728)]),
            ),
            (11u16, test_production_data_payload()),
            (12u16, test_focus_data_payload()),
            (13u16, test_tech_data_payload()),
            (14u16, test_politics_data_payload()),
            (15u16, test_politics_residuals_data_payload()),
            (16u16, test_decisions_data_payload()),
            (17u16, test_diplomacy_data_payload()),
        ] {
            append_test_compact_record(
                &mut raw,
                sequence,
                100,
                domain_id,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &test_domain_payload(1_250, 355, 1, 1, 1),
            );
            sequence += 1;
            append_test_compact_record(
                &mut raw,
                sequence,
                100,
                domain_id,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            sequence += 1;
        }
        append_test_compact_record(
            &mut raw,
            sequence,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 8),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(
            compact_domain_coverage_errors(&summary, &[10, 11, 12, 13, 14, 15, 16, 17]).is_empty()
        );
        assert_eq!(summary.domain_data_record_count, 8);
        for domain_id in [10u16, 11, 12, 13, 14, 15, 16, 17] {
            assert_eq!(
                summary
                    .per_domain
                    .get(&domain_id)
                    .unwrap()
                    .data_record_count,
                1
            );
            assert_eq!(
                summary.per_domain.get(&domain_id).unwrap().data_rows_total,
                1
            );
        }
        let production_units = &summary.per_domain.get(&11).unwrap().data_units_total;
        assert_eq!(production_units.get("stockpile_rows").copied(), Some(1));
        assert_eq!(production_units.get("military_line_rows").copied(), Some(1));
        assert_eq!(production_units.get("naval_line_rows").copied(), Some(1));
        assert_eq!(
            production_units.get("construction_queue_rows").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .per_domain
                .get(&12)
                .unwrap()
                .data_units_total
                .get("completed_focus_tokens")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .per_domain
                .get(&13)
                .unwrap()
                .data_units_total
                .get("active_research_slot_rows")
                .copied(),
            Some(1)
        );
        let politics_units = &summary.per_domain.get(&14).unwrap().data_units_total;
        assert_eq!(politics_units.get("popularity_rows").copied(), Some(2));
        assert_eq!(politics_units.get("law_rows").copied(), Some(1));
        assert_eq!(politics_units.get("advisor_rows").copied(), Some(1));
        assert_eq!(politics_units.get("national_spirit_rows").copied(), Some(1));
        assert_eq!(
            summary
                .per_domain
                .get(&15)
                .unwrap()
                .data_units_total
                .get("advisor_detail_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .per_domain
                .get(&16)
                .unwrap()
                .data_units_total
                .get("timed_decision_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .per_domain
                .get(&17)
                .unwrap()
                .data_units_total
                .get("at_war_relation_rows")
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn compact_summary_decodes_divisions_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(1_500, 2, 2, 2, 2),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_divisions_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[20]).is_empty());
        let divisions = summary.per_domain.get(&20).unwrap();
        assert_eq!(divisions.data_record_count, 1);
        assert_eq!(divisions.data_rows_total, 2);
        assert_eq!(divisions.data_changed_rows_total, 2);
        assert_eq!(
            divisions
                .data_units_total
                .get("division_org_strength_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            divisions
                .data_units_total
                .get("division_moving_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            divisions
                .data_units_total
                .get("division_in_combat_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn compact_summary_decodes_division_equipment_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            26,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(900, 2, 2, 3, 3),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            26,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_division_equipment_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_100, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[26]).is_empty());
        let equipment = summary.per_domain.get(&26).unwrap();
        assert_eq!(equipment.data_record_count, 1);
        assert_eq!(equipment.data_rows_total, 2);
        assert_eq!(equipment.data_changed_rows_total, 2);
        assert_eq!(equipment.unit_count_total, 3);
        assert_eq!(
            equipment
                .data_units_total
                .get("division_equipment_entry_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(compact_domain_id("division_equipment"), Some(26));
        assert_eq!(compact_domain_name(26), "division_equipment");
    }

    #[test]
    fn compact_summary_decodes_army_orders_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            27,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(700, 2, 2, 3, 2),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            27,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_army_orders_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_100, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[27]).is_empty());
        let orders = summary.per_domain.get(&27).unwrap();
        assert_eq!(orders.data_record_count, 1);
        assert_eq!(orders.data_rows_total, 2);
        assert_eq!(orders.data_changed_rows_total, 2);
        assert_eq!(orders.unit_count_total, 3);
        assert_eq!(
            orders
                .data_units_total
                .get("army_order_observed_armies")
                .copied(),
            Some(2)
        );
        assert_eq!(
            orders
                .data_units_total
                .get("army_order_observed_orders")
                .copied(),
            Some(3)
        );
        assert_eq!(
            orders
                .data_units_total
                .get("army_order_target_state_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            orders
                .data_units_total
                .get("army_order_deleted_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(compact_domain_id("army_orders"), Some(27));
        assert_eq!(compact_domain_name(27), "army_orders");
    }

    #[test]
    fn compact_summary_decodes_oob_command_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            28,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(650, 4, 4, 6, 4),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            28,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_100, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[28]).is_empty());
        let oob = summary.per_domain.get(&28).unwrap();
        assert_eq!(oob.data_record_count, 1);
        assert_eq!(oob.data_rows_total, 4);
        assert_eq!(oob.data_changed_rows_total, 4);
        assert_eq!(oob.unit_count_total, 6);
        assert_eq!(
            oob.data_units_total.get("oob_observed_army_rows").copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total
                .get("oob_observed_army_group_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total.get("oob_observed_fleet_rows").copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total
                .get("oob_observed_task_force_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total.get("oob_member_rows").copied(),
            Some(6)
        );
        assert_eq!(
            oob.data_units_total
                .get("oob_member_division_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            oob.data_units_total.get("oob_member_army_rows").copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total
                .get("oob_member_task_force_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total.get("oob_member_ship_rows").copied(),
            Some(2)
        );
        assert_eq!(oob.data_units_total.get("oob_fleet_rows").copied(), Some(1));
        assert_eq!(
            oob.data_units_total.get("oob_task_force_rows").copied(),
            Some(1)
        );
        assert_eq!(
            oob.data_units_total.get("oob_location_rows").copied(),
            Some(1)
        );
        assert_eq!(compact_domain_id("oob_command"), Some(28));
        assert_eq!(compact_domain_name(28), "oob_command");
    }

    #[test]
    fn compact_summary_decodes_ships_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            29,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(700, 2, 2, 1, 2),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_100, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[29]).is_empty());
        let ships = summary.per_domain.get(&29).unwrap();
        assert_eq!(ships.data_record_count, 1);
        assert_eq!(ships.data_rows_total, 2);
        assert_eq!(ships.data_changed_rows_total, 2);
        assert_eq!(ships.unit_count_total, 1);
        assert_eq!(
            ships
                .data_units_total
                .get("ship_observed_ship_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            ships
                .data_units_total
                .get("ship_observed_task_force_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            ships.data_units_total.get("ship_task_force_rows").copied(),
            Some(1)
        );
        assert_eq!(
            ships.data_units_total.get("ship_owner_rows").copied(),
            Some(1)
        );
        assert_eq!(
            ships.data_units_total.get("ship_definition_rows").copied(),
            Some(1)
        );
        assert_eq!(compact_domain_id("ships"), Some(29));
        assert_eq!(compact_domain_name(29), "ships");
    }

    #[test]
    fn compact_ship_root_source_marks_unresolved_blocked_native_rows() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            29,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(500, 0, 0, 0, 0, COMPACT_STATUS_SOURCE_UNAVAILABLE),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            29,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_empty_ships_data_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_100, 1),
        );

        let summary = summarize_compact_stream(&raw);
        let cfg = test_config(vec!["ships"], None);
        let source = compact_ship_root_source(&cfg, &summary);

        assert_eq!(source, "unresolved");
        assert_eq!(
            compact_ship_root_discovery_debt(source),
            "anchored CTaskForce/CShip root/container did not resolve; ship bridges and heap scans are excluded from production capture"
        );
    }

    #[test]
    fn compact_summary_decodes_air_wings_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            30,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(700, 2, 2, 2, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_air_wings_data_payload(COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[30]).is_empty());
        let air_wings = summary.per_domain.get(&30).unwrap();
        assert_eq!(air_wings.row_count_total, 2);
        assert_eq!(air_wings.data_record_count, 1);
        assert_eq!(air_wings.data_rows_total, 2);
        assert_eq!(air_wings.data_changed_rows_total, 2);
        assert_eq!(
            air_wings.data_units_total.get("air_wing_rows").copied(),
            Some(2)
        );
        assert_eq!(
            air_wings
                .data_units_total
                .get("air_wing_observed_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            air_wings
                .data_units_total
                .get("air_wing_deleted_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            air_wings
                .data_units_total
                .get("air_wing_mission_region_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(compact_domain_id("air_wings"), Some(30));
        assert_eq!(compact_domain_name(30), "air_wings");
    }

    #[test]
    fn compact_summary_decodes_air_region_activity_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            31,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(800, 2, 2, 3, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            31,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_air_region_activity_data_payload(COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[31]).is_empty());
        let activity = summary.per_domain.get(&31).unwrap();
        assert_eq!(activity.row_count_total, 2);
        assert_eq!(activity.unit_count_total, 3);
        assert_eq!(activity.data_record_count, 1);
        assert_eq!(activity.data_rows_total, 2);
        assert_eq!(activity.data_changed_rows_total, 2);
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_observed_wing_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_observed_bucket_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_deleted_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_wing_count_sum")
                .copied(),
            Some(2)
        );
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_plane_count_sum")
                .copied(),
            Some(200)
        );
        assert_eq!(
            activity
                .data_units_total
                .get("air_region_activity_effective_planes_sum")
                .copied(),
            Some(180)
        );
        assert_eq!(compact_domain_id("air_region_activity"), Some(31));
        assert_eq!(compact_domain_name(31), "air_region_activity");
    }

    #[test]
    fn compact_summary_decodes_air_wing_loadouts_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            32,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(900, 2, 2, 2, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_air_wing_loadouts_data_payload(COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[32]).is_empty());
        let loadouts = summary.per_domain.get(&32).unwrap();
        assert_eq!(loadouts.row_count_total, 2);
        assert_eq!(loadouts.unit_count_total, 2);
        assert_eq!(loadouts.data_record_count, 1);
        assert_eq!(loadouts.data_rows_total, 2);
        assert_eq!(loadouts.data_changed_rows_total, 2);
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_observed_wing_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_observed_equipment_rows")
                .copied(),
            Some(4)
        );
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_equipment_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_deleted_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_air_base_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            loadouts
                .data_units_total
                .get("air_wing_loadout_amount_raw_sum")
                .copied(),
            Some(10_000_000)
        );
        assert_eq!(compact_domain_id("air_wing_loadouts"), Some(32));
        assert_eq!(compact_domain_name(32), "air_wing_loadouts");
    }

    #[test]
    fn compact_summary_decodes_air_variant_defs_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            33,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(700, 2, 2, 413, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            33,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_air_variant_defs_data_payload(COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[33]).is_empty());
        let defs = summary.per_domain.get(&33).unwrap();
        assert_eq!(defs.row_count_total, 2);
        assert_eq!(defs.unit_count_total, 413);
        assert_eq!(defs.data_record_count, 1);
        assert_eq!(defs.data_rows_total, 2);
        assert_eq!(defs.data_changed_rows_total, 2);
        assert_eq!(
            defs.data_units_total.get("air_variant_def_rows").copied(),
            Some(2)
        );
        assert_eq!(
            defs.data_units_total
                .get("air_variant_def_observed_variant_rows")
                .copied(),
            Some(140)
        );
        assert_eq!(
            defs.data_units_total
                .get("air_variant_def_observed_module_rows")
                .copied(),
            Some(413)
        );
        assert_eq!(
            defs.data_units_total
                .get("air_variant_def_emitted_module_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            defs.data_units_total
                .get("air_variant_def_invalid_variant_rows")
                .copied(),
            None
        );
        assert_eq!(
            defs.data_units_total
                .get("air_variant_def_module_token_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(compact_domain_id("air_variant_defs"), Some(33));
        assert_eq!(compact_domain_name(33), "air_variant_defs");
    }

    #[test]
    fn compact_summary_decodes_air_variant_stats_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            35,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(550, 2, 2, 140, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_air_variant_stats_data_payload(COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[35]).is_empty());
        let stats = summary.per_domain.get(&35).unwrap();
        assert_eq!(stats.row_count_total, 2);
        assert_eq!(stats.unit_count_total, 140);
        assert_eq!(stats.data_record_count, 1);
        assert_eq!(stats.data_rows_total, 2);
        assert_eq!(stats.data_changed_rows_total, 2);
        assert_eq!(
            stats
                .data_units_total
                .get("air_variant_stats_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            stats
                .data_units_total
                .get("air_variant_stats_observed_variant_rows")
                .copied(),
            Some(140)
        );
        assert_eq!(
            stats
                .data_units_total
                .get("air_variant_stats_range_raw_sum")
                .copied(),
            Some(2_200_000)
        );
        assert_eq!(
            stats
                .data_units_total
                .get("air_variant_stats_air_attack_raw_sum")
                .copied(),
            Some(600_000)
        );
        assert_eq!(
            stats
                .data_units_total
                .get("air_variant_stats_max_speed_raw_sum")
                .copied(),
            Some(60_000_000)
        );
        assert_eq!(
            stats
                .data_units_total
                .get("air_variant_stats_build_cost_ic_raw_sum")
                .copied(),
            Some(5_082_500)
        );
        assert_eq!(compact_domain_id("air_variant_stats"), Some(35));
        assert_eq!(compact_domain_name(35), "air_variant_stats");
    }

    #[test]
    fn compact_summary_decodes_air_base_status_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            34,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(650, 2, 2, 5, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_air_base_status_data_payload(COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[34]).is_empty());
        let bases = summary.per_domain.get(&34).unwrap();
        assert_eq!(bases.row_count_total, 2);
        assert_eq!(bases.unit_count_total, 5);
        assert_eq!(bases.data_record_count, 1);
        assert_eq!(bases.data_rows_total, 2);
        assert_eq!(bases.data_changed_rows_total, 2);
        assert_eq!(
            bases.data_units_total.get("air_base_status_rows").copied(),
            Some(2)
        );
        assert_eq!(
            bases
                .data_units_total
                .get("air_base_status_observed_air_base_rows")
                .copied(),
            Some(367)
        );
        assert_eq!(
            bases
                .data_units_total
                .get("air_base_status_observed_pool_rows")
                .copied(),
            Some(5)
        );
        assert_eq!(
            bases
                .data_units_total
                .get("air_base_status_capacity_sum")
                .copied(),
            Some(2400)
        );
        assert_eq!(
            bases
                .data_units_total
                .get("air_base_status_based_wing_count_sum")
                .copied(),
            Some(4)
        );
        assert_eq!(
            bases
                .data_units_total
                .get("air_base_status_based_plane_count_sum")
                .copied(),
            Some(378)
        );
        assert_eq!(compact_domain_id("air_base_status"), Some(34));
        assert_eq!(compact_domain_name(34), "air_base_status");
    }

    #[test]
    fn compact_summary_decodes_division_delta_delete_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(1_500, 4, 2, 4, 2, COMPACT_STATUS_DELTA_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_divisions_delta_data_payload(),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[20]).is_empty());
        let divisions = summary.per_domain.get(&20).unwrap();
        assert_eq!(divisions.row_count_total, 4);
        assert_eq!(divisions.data_record_count, 1);
        assert_eq!(divisions.data_rows_total, 2);
        assert_eq!(divisions.data_changed_rows_total, 2);
        assert_ne!(divisions.status_flags_or & COMPACT_STATUS_DELTA_DATA, 0);
    }

    #[test]
    fn compact_division_root_source_marks_dll_native_success() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(1_500, 2, 2, 2, 2),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_divisions_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );

        let cfg = test_config(vec!["divisions"], None);
        let summary = summarize_compact_stream(&raw);

        let source = compact_division_root_source(&cfg, &summary);
        assert_eq!(source, "dll_native");
        assert_eq!(
            compact_division_root_discovery_debt(source),
            "none; compact stream used a DLL-native anchored division root"
        );
        assert_eq!(native_division_root_prewarm_budget_ms(&cfg), 0);
    }

    #[test]
    fn compact_division_root_source_marks_python_bridge_without_prewarm() {
        let cfg = test_config(vec!["divisions"], Some("1:0x10000000000"));
        let summary = CompactSummary::default();

        assert_eq!(
            compact_division_root_source(&cfg, &summary),
            "python_bridge"
        );
        assert_eq!(native_division_root_prewarm_budget_ms(&cfg), 0);
    }

    #[test]
    fn compact_division_root_source_marks_unresolved_blocked_native_rows() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(500, 2, 2, 2, 2, COMPACT_STATUS_SOURCE_UNAVAILABLE),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_divisions_data_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );

        let cfg = test_config(vec!["divisions"], None);
        let summary = summarize_compact_stream(&raw);

        let source = compact_division_root_source(&cfg, &summary);
        assert_eq!(source, "unresolved");
        assert_eq!(
            compact_division_root_discovery_debt(source),
            "anchored division root/container did not resolve; heap scans are excluded from production capture"
        );
    }

    #[test]
    fn compact_divisions_bridge_unavailable_fails_hourly_problem_list() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(500, 0, 0, 0, 0, COMPACT_STATUS_BRIDGE_UNAVAILABLE),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_divisions_data_payload(COMPACT_STATUS_BRIDGE_UNAVAILABLE),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );

        let summary = summarize_compact_stream(&raw);
        let coverage_errors = compact_domain_coverage_errors(&summary, &[20]);
        let problems = compact_hourly_problems(&summary, 0, &coverage_errors);

        assert!(problems.contains(&"compact_bridge_unavailable".to_string()));
    }

    #[test]
    fn compact_summary_decodes_land_combat_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(800, 2, 2, 3, 2),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_land_combats_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_200, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[21]).is_empty());
        let land = summary.per_domain.get(&21).unwrap();
        assert_eq!(land.data_record_count, 1);
        assert_eq!(land.data_rows_total, 2);
        assert_eq!(land.unit_count_total, 3);
        assert_eq!(
            land.data_units_total
                .get("land_combat_participant_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(
            land.data_units_total
                .get("land_combat_attacker_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            land.data_units_total
                .get("land_combat_defender_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn compact_summary_decodes_map_control_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(7_500, 11_200, 11_200, 10_100, 10_100),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_map_control_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(8_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[22]).is_empty());
        let map = summary.per_domain.get(&22).unwrap();
        assert_eq!(map.data_record_count, 1);
        assert_eq!(map.data_rows_total, 11_200);
        assert_eq!(map.data_changed_rows_total, 11_200);
        assert_eq!(map.unit_count_total, 10_100);
        assert_eq!(
            map.data_units_total.get("map_control_state_rows").copied(),
            Some(1_100)
        );
        assert_eq!(
            map.data_units_total
                .get("map_control_province_rows")
                .copied(),
            Some(10_100)
        );
    }

    #[test]
    fn compact_summary_decodes_empty_map_control_delta_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(
                1_000,
                11_200,
                0,
                10_100,
                0,
                COMPACT_STATUS_DELTA_DATA,
            ),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_map_control_delta_data_payload(0, 0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_100, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[22]).is_empty());
        let map = summary.per_domain.get(&22).unwrap();
        assert_eq!(map.data_record_count, 1);
        assert_eq!(map.row_count_total, 11_200);
        assert_eq!(map.data_rows_total, 0);
        assert_eq!(map.data_changed_rows_total, 0);
        assert_eq!(map.unit_count_total, 10_100);
        assert_ne!(map.status_flags_or & COMPACT_STATUS_DELTA_DATA, 0);
    }

    #[test]
    fn compact_summary_decodes_map_detail_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            23,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(7_500, 11_200, 11_200, 11_200, 11_200),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_map_detail_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(8_000, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[23]).is_empty());
        let map = summary.per_domain.get(&23).unwrap();
        assert_eq!(map.data_record_count, 1);
        assert_eq!(map.data_rows_total, 11_200);
        assert_eq!(map.data_changed_rows_total, 11_200);
        assert_eq!(map.unit_count_total, 11_200);
        assert_eq!(
            map.data_units_total
                .get("map_detail_resource_rows")
                .copied(),
            Some(1_100)
        );
        assert_eq!(
            map.data_units_total
                .get("map_detail_state_building_rows")
                .copied(),
            Some(1_100)
        );
        assert_eq!(
            map.data_units_total
                .get("map_detail_province_building_rows")
                .copied(),
            Some(10_100)
        );
        assert_eq!(compact_domain_id("state_detail"), Some(23));
        assert_eq!(compact_domain_id("buildings_live"), Some(23));
        assert_eq!(compact_domain_name(23), "map_detail");
    }

    #[test]
    fn compact_summary_decodes_weather_data_record() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            24,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload(1_500, 305, 305, 305, 305),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            24,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_weather_data_payload(0),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_800, 1),
        );

        let summary = summarize_compact_stream(&raw);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(compact_domain_coverage_errors(&summary, &[24]).is_empty());
        let weather = summary.per_domain.get(&24).unwrap();
        assert_eq!(weather.data_record_count, 1);
        assert_eq!(weather.data_rows_total, 305);
        assert_eq!(weather.data_changed_rows_total, 305);
        assert_eq!(weather.unit_count_total, 305);
        assert_eq!(
            weather
                .data_units_total
                .get("weather_active_region_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(compact_domain_id("weather_live"), Some(24));
        assert_eq!(compact_domain_name(24), "weather_live");
    }

    #[test]
    fn compact_land_combat_partial_data_fails_hourly_problem_list() {
        let mut raw = Vec::new();
        append_test_compact_record(
            &mut raw,
            1,
            100,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_payload_with_status(500, 0, 0, 0, 0, COMPACT_STATUS_PARTIAL_DATA),
        );
        append_test_compact_record(
            &mut raw,
            2,
            100,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_land_combats_data_payload(COMPACT_STATUS_PARTIAL_DATA),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(1_200, 1),
        );

        let summary = summarize_compact_stream(&raw);
        let coverage_errors = compact_domain_coverage_errors(&summary, &[21]);
        let problems = compact_hourly_problems(&summary, 0, &coverage_errors);

        assert!(problems.contains(&"compact_partial_data".to_string()));
    }

    #[test]
    fn replay_container_preserves_compact_stream_bytes() {
        let dir =
            std::env::temp_dir().join(format!("battle-plan_pack_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("native_session_manifest.json");
        let replay_path = dir.join("native_session.battleplan");
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
            10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_fuel_data_payload(&[(47, 4_046_848, 14_950_400, 245_760, -73_728)]),
        );
        append_test_compact_record(
            &mut raw,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_payload(2_000, 1),
        );
        let manifest = json!({
            "schema": "native_collector_session_v1",
            "session_id": "pack test/session",
            "replay_game_id": replay_game_id("pack test/session"),
            "stream_format": "compact",
        });
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let cfg = Config {
            pid: 1,
            dll: dir.join("collector.dll"),
            out_dir: dir.clone(),
            duration_ms: 1000,
            sample_ms: 0,
            dwell_ms: 0,
            min_game_hours: 0,
            scheduled_domains: vec!["fuel_live".to_string()],
            lightweight_root_frame: true,
            special_projects_scan: false,
            compact_country_frame: true,
            division_body_bridge: None,
            idea_group_filter: None,
            advisor_token_filter: None,
            session_id: "pack test/session".to_string(),
            stream_format: StreamFormat::Compact,
            extraction_perf_gate: true,
        };

        write_native_replay_container(&cfg, &raw, &manifest, &manifest_path, &replay_path).unwrap();

        let container = Container::open(&replay_path, &ExpectedIdentity::default()).unwrap();
        container.verify_all().unwrap();
        let game_id = replay_game_id("pack test/session");
        let stored = container
            .read_entry_bytes(&format!(
                "payload/{game_id}/collector_raw/collector_stream.bin"
            ))
            .unwrap();
        assert_eq!(stored, raw);
        let _ = fs::remove_dir_all(&dir);
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
        let division_bridge = dir.join("division_bridge.txt");
        fs::write(&idea_groups, b"war_economy=economy\n").unwrap();
        fs::write(
            &advisor_tokens,
            b"POR_salazar=political_advisor|political|POR_salazar\n",
        )
        .unwrap();
        fs::write(&division_bridge, b"17:0x10000000000\n").unwrap();

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
            "--division-body-bridge-file".to_string(),
            division_bridge.display().to_string(),
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
        assert_eq!(
            cfg.division_body_bridge.as_deref(),
            Some("17:0x10000000000")
        );
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
        test_domain_payload_with_status(
            read_time_us,
            row_count,
            changed_rows,
            unit_count,
            changed_unit_count,
            0,
        )
    }

    fn test_config(scheduled_domains: Vec<&str>, division_body_bridge: Option<&str>) -> Config {
        Config {
            pid: 1,
            dll: PathBuf::from("collector.dll"),
            out_dir: PathBuf::from("."),
            duration_ms: 1_000,
            sample_ms: 0,
            dwell_ms: 0,
            min_game_hours: 0,
            scheduled_domains: scheduled_domains
                .into_iter()
                .map(|domain| domain.to_string())
                .collect(),
            lightweight_root_frame: true,
            special_projects_scan: false,
            compact_country_frame: true,
            division_body_bridge: division_body_bridge.map(str::to_string),
            idea_group_filter: None,
            advisor_token_filter: None,
            session_id: "test-session".to_string(),
            stream_format: StreamFormat::Compact,
            extraction_perf_gate: true,
        }
    }

    fn test_domain_payload_with_status(
        read_time_us: u64,
        row_count: u32,
        changed_rows: u32,
        unit_count: u32,
        changed_unit_count: u32,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&read_time_us.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&unit_count.to_le_bytes());
        payload.extend_from_slice(&changed_unit_count.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
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

    fn test_fuel_data_payload(rows: &[(u32, i64, i64, i64, i64)]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FUEL_SCALE_RAW.to_le_bytes());
        for (tag_db_id, current, max, gain, cost) in rows {
            payload.extend_from_slice(&tag_db_id.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&current.to_le_bytes());
            payload.extend_from_slice(&max.to_le_bytes());
            payload.extend_from_slice(&gain.to_le_bytes());
            payload.extend_from_slice(&cost.to_le_bytes());
        }
        payload
    }

    fn test_production_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_PRODUCTION_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes()); // stockpile
        payload.extend_from_slice(&1u32.to_le_bytes()); // military
        payload.extend_from_slice(&1u32.to_le_bytes()); // naval
        payload.extend_from_slice(&1u32.to_le_bytes()); // construction
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&70u32.to_le_bytes());
        payload.extend_from_slice(&12_350_000i64.to_le_bytes());
        for (kind, id, factories, cost, speed, produced, amount) in [
            (
                1u32,
                349i32,
                3i32,
                10_500_000i32,
                3_105_000i32,
                3_440_000i32,
                -1i32,
            ),
            (
                2u32,
                350i32,
                2i32,
                5_500_000i32,
                2_105_000i32,
                1_440_000i32,
                1i32,
            ),
        ] {
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&kind.to_le_bytes());
            payload.extend_from_slice(&id.to_le_bytes());
            payload.extend_from_slice(&factories.to_le_bytes());
            payload.extend_from_slice(&cost.to_le_bytes());
            payload.extend_from_slice(&speed.to_le_bytes());
            payload.extend_from_slice(&produced.to_le_bytes());
            payload.extend_from_slice(&amount.to_le_bytes());
        }
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&9001u32.to_le_bytes());
        payload.extend_from_slice(&20013u32.to_le_bytes());
        payload.extend_from_slice(&6i32.to_le_bytes());
        payload.extend_from_slice(&1_080_000_000i32.to_le_bytes());
        payload.extend_from_slice(&12_550_000i32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn test_focus_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b11_1111u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1_350_000i32.to_le_bytes());
        append_test_compact_string(&mut payload, "portugal_focus");
        append_test_compact_string(&mut payload, "POR_industrial_focus");
        append_test_compact_string(&mut payload, "POR_continuous_focus");
        payload.extend_from_slice(&47u32.to_le_bytes());
        append_test_compact_string(&mut payload, "POR_completed_focus");
        payload
    }

    fn test_tech_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_TECH_PROGRESS_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b111u32.to_le_bytes());
        payload.extend_from_slice(&17u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0x2A4Eu32.to_le_bytes());
        payload.extend_from_slice(&717u32.to_le_bytes());
        payload.extend_from_slice(&250u32.to_le_bytes());
        payload.extend_from_slice(&0b11u32.to_le_bytes());
        payload.extend_from_slice(&125_000i64.to_le_bytes());
        payload.extend_from_slice(&300_000i64.to_le_bytes());
        payload
    }

    fn test_politics_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b11_1111_1111u32.to_le_bytes());
        payload.extend_from_slice(&9u32.to_le_bytes());
        payload.extend_from_slice(&48u32.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&123u32.to_le_bytes());
        append_test_compact_string(&mut payload, "neutrality");
        append_test_compact_string(&mut payload, "Antonio");
        append_test_compact_string(&mut payload, "POR_antonio");
        append_test_compact_string(&mut payload, "neutrality");
        for (ideology, raw) in [("democratic", 3_500_000i32), ("neutrality", 6_500_000i32)] {
            payload.extend_from_slice(&47u32.to_le_bytes());
            append_test_compact_string(&mut payload, ideology);
            payload.extend_from_slice(&raw.to_le_bytes());
        }
        payload.extend_from_slice(&47u32.to_le_bytes());
        append_test_compact_string(&mut payload, "economy");
        append_test_compact_string(&mut payload, "partial_economic_mobilisation");
        payload.extend_from_slice(&47u32.to_le_bytes());
        append_test_compact_string(&mut payload, "political_advisor");
        append_test_compact_string(&mut payload, "POR_salazar");
        payload.extend_from_slice(&47u32.to_le_bytes());
        append_test_compact_string(&mut payload, "estado_novo");
        payload
    }

    fn test_politics_popularity_delta_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&(1u32 | (1 << 8) | (1 << 9)).to_le_bytes());
        payload.extend_from_slice(&u32::MAX.to_le_bytes());
        payload.extend_from_slice(&u32::MAX.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..4 {
            append_test_compact_string(&mut payload, "");
        }
        for (ideology, raw) in [("democratic", 3_600_000i32), ("neutrality", 6_400_000i32)] {
            payload.extend_from_slice(&47u32.to_le_bytes());
            append_test_compact_string(&mut payload, ideology);
            payload.extend_from_slice(&raw.to_le_bytes());
        }
        payload
    }

    fn test_politics_residuals_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b111u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        append_test_compact_string(&mut payload, "1936.1.1.1");
        payload.extend_from_slice(&47u32.to_le_bytes());
        append_test_compact_string(&mut payload, "POR_salazar");
        append_test_compact_string(&mut payload, "political_advisor");
        append_test_compact_string(&mut payload, "political");
        append_test_compact_string(&mut payload, "POR_antonio");
        payload
    }

    fn test_decisions_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b11u32.to_le_bytes());
        for count in [1u32, 0, 0, 2, 0] {
            payload.extend_from_slice(&count.to_le_bytes());
        }
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&35i32.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
        append_test_compact_string(&mut payload, "POR_countdown_mission");
        payload
    }

    fn test_diplomacy_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b11u32.to_le_bytes());
        payload.extend_from_slice(&42i32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1234i64.to_le_bytes());
        for tag in [b"GER", b"ITA"] {
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&3u32.to_le_bytes());
            let mut fixed = [0u8; 8];
            fixed[..3].copy_from_slice(tag);
            payload.extend_from_slice(&fixed);
        }
        payload
    }

    fn test_divisions_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for (
            div_id,
            flags,
            location,
            target,
            source,
            progress,
            progress_bucket,
            org,
            org_bucket,
            strength,
            strength_bucket,
            combat_refs,
            template_id,
            owner,
            army,
            experience,
            max_supply,
            current_supply,
            supply_gain,
            fuel,
        ) in [
            (
                17u32,
                0b10_1111_1111_1111u32,
                118u32,
                118u32,
                117u32,
                42_000u32,
                42u32,
                8_500_000u32,
                8_500u32,
                96_250_000u32,
                96_250u32,
                1u32,
                3101u32,
                47u32,
                2001u32,
                333_000u32,
                1_600_000u32,
                1_250_000u32,
                25_000u32,
                450_000u32,
            ),
            (
                18u32,
                0b1111_1010_0000u32,
                119u32,
                0u32,
                0u32,
                0u32,
                0u32,
                10_000_000u32,
                10_000u32,
                100_000_000u32,
                100_000u32,
                0u32,
                0u32,
                48u32,
                0u32,
                0u32,
                1_200_000u32,
                1_200_000u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&div_id.to_le_bytes());
            payload.extend_from_slice(&COMPACT_DIVISION_TYPE.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&location.to_le_bytes());
            payload.extend_from_slice(&target.to_le_bytes());
            payload.extend_from_slice(&source.to_le_bytes());
            payload.extend_from_slice(&progress.to_le_bytes());
            payload.extend_from_slice(&progress_bucket.to_le_bytes());
            payload.extend_from_slice(&org.to_le_bytes());
            payload.extend_from_slice(&org_bucket.to_le_bytes());
            payload.extend_from_slice(&strength.to_le_bytes());
            payload.extend_from_slice(&strength_bucket.to_le_bytes());
            payload.extend_from_slice(&combat_refs.to_le_bytes());
            payload.extend_from_slice(&template_id.to_le_bytes());
            payload.extend_from_slice(&owner.to_le_bytes());
            payload.extend_from_slice(&army.to_le_bytes());
            payload.extend_from_slice(&experience.to_le_bytes());
            payload.extend_from_slice(&max_supply.to_le_bytes());
            payload.extend_from_slice(&current_supply.to_le_bytes());
            payload.extend_from_slice(&supply_gain.to_le_bytes());
            payload.extend_from_slice(&fuel.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_divisions_delta_data_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            div_id,
            flags,
            location,
            target,
            source,
            progress,
            progress_bucket,
            org,
            org_bucket,
            strength,
            strength_bucket,
            combat_refs,
            template_id,
            owner,
            army,
            experience,
            max_supply,
            current_supply,
            supply_gain,
            fuel,
        ) in [
            (
                17u32,
                0b10_1111_1111_1111u32,
                118u32,
                118u32,
                117u32,
                42_000u32,
                42u32,
                8_500_000u32,
                8_500u32,
                96_250_000u32,
                96_250u32,
                1u32,
                3101u32,
                47u32,
                2001u32,
                333_000u32,
                1_600_000u32,
                1_250_000u32,
                25_000u32,
                450_000u32,
            ),
            (
                18u32,
                COMPACT_DIVISION_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&div_id.to_le_bytes());
            payload.extend_from_slice(&COMPACT_DIVISION_TYPE.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&location.to_le_bytes());
            payload.extend_from_slice(&target.to_le_bytes());
            payload.extend_from_slice(&source.to_le_bytes());
            payload.extend_from_slice(&progress.to_le_bytes());
            payload.extend_from_slice(&progress_bucket.to_le_bytes());
            payload.extend_from_slice(&org.to_le_bytes());
            payload.extend_from_slice(&org_bucket.to_le_bytes());
            payload.extend_from_slice(&strength.to_le_bytes());
            payload.extend_from_slice(&strength_bucket.to_le_bytes());
            payload.extend_from_slice(&combat_refs.to_le_bytes());
            payload.extend_from_slice(&template_id.to_le_bytes());
            payload.extend_from_slice(&owner.to_le_bytes());
            payload.extend_from_slice(&army.to_le_bytes());
            payload.extend_from_slice(&experience.to_le_bytes());
            payload.extend_from_slice(&max_supply.to_le_bytes());
            payload.extend_from_slice(&current_supply.to_le_bytes());
            payload.extend_from_slice(&supply_gain.to_le_bytes());
            payload.extend_from_slice(&fuel.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_division_equipment_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());

        for (owner, div_id, flags, template_id, entry_first, entry_count) in [
            (
                47u32,
                17u32,
                COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE,
                3101u32,
                0u32,
                2u32,
            ),
            (48u32, 18u32, 0u32, 0u32, 2u32, 1u32),
        ] {
            payload.extend_from_slice(&owner.to_le_bytes());
            payload.extend_from_slice(&div_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&template_id.to_le_bytes());
            payload.extend_from_slice(&entry_first.to_le_bytes());
            payload.extend_from_slice(&entry_count.to_le_bytes());
        }

        for (variant_id, amount_raw) in [
            (101u32, 11_300_000i64),
            (202u32, 500_000i64),
            (303u32, 7_500_000i64),
        ] {
            payload.extend_from_slice(&variant_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&amount_raw.to_le_bytes());
        }
        payload
    }

    fn test_army_orders_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&20u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());

        for (
            army_id,
            generation,
            flags,
            slot_index,
            order_type,
            target_first,
            target_count,
            target_state_count,
            target_status_code,
        ) in [
            (
                17u32,
                3u64,
                COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES
                    | COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT,
                2u32,
                5i32,
                0u32,
                2u32,
                2u32,
                1u32,
            ),
            (
                18u32,
                4u64,
                COMPACT_ARMY_ORDER_ROW_FLAG_DELETED,
                0u32,
                2i32,
                0u32,
                0u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&army_id.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&slot_index.to_le_bytes());
            payload.extend_from_slice(&order_type.to_le_bytes());
            payload.extend_from_slice(&target_first.to_le_bytes());
            payload.extend_from_slice(&target_count.to_le_bytes());
            payload.extend_from_slice(&target_state_count.to_le_bytes());
            payload.extend_from_slice(&target_status_code.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for state_id in [118u32, 119u32] {
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_oob_command_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&22u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&6u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());

        for (
            kind,
            object_id,
            flags,
            parent_id,
            owner_tag_db_id,
            leader_id,
            leader_skill,
            member_first,
            row_member_count,
            logical_member_count,
            member_count_engine,
            missing_member_count,
            leader_trait_count,
            location_province_id,
            parent_status,
            member_status,
            owner_status,
            leader_status,
            leader_trait_status,
        ) in [
            (
                COMPACT_OOB_KIND_ARMY,
                17u32,
                COMPACT_OOB_ROW_FLAG_PARENT
                    | COMPACT_OOB_ROW_FLAG_OWNER
                    | COMPACT_OOB_ROW_FLAG_LEADER
                    | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                    | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                    | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT,
                301u32,
                47u32,
                9001u32,
                4u32,
                0u32,
                2u32,
                2u32,
                3u32,
                1u32,
                3u32,
                0u32,
                1u32,
                1u32,
                1u32,
                1u32,
                1u32,
            ),
            (
                COMPACT_OOB_KIND_ARMY_GROUP,
                301u32,
                COMPACT_OOB_ROW_FLAG_OWNER,
                0u32,
                47u32,
                0u32,
                0u32,
                2u32,
                1u32,
                1u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                1u32,
                1u32,
                0u32,
                0u32,
            ),
            (
                COMPACT_OOB_KIND_FLEET,
                615u32,
                COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE,
                0u32,
                0u32,
                0u32,
                0u32,
                3u32,
                1u32,
                1u32,
                1u32,
                0u32,
                0u32,
                0u32,
                0u32,
                1u32,
                2u32,
                0u32,
                0u32,
            ),
            (
                COMPACT_OOB_KIND_TASK_FORCE,
                617u32,
                COMPACT_OOB_ROW_FLAG_PARENT
                    | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                    | COMPACT_OOB_ROW_FLAG_LOCATION,
                615u32,
                0u32,
                0u32,
                0u32,
                4u32,
                2u32,
                2u32,
                2u32,
                0u32,
                0u32,
                12389u32,
                1u32,
                1u32,
                2u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&kind.to_le_bytes());
            payload.extend_from_slice(&object_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&parent_id.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&leader_id.to_le_bytes());
            payload.extend_from_slice(&leader_skill.to_le_bytes());
            payload.extend_from_slice(&member_first.to_le_bytes());
            payload.extend_from_slice(&row_member_count.to_le_bytes());
            payload.extend_from_slice(&logical_member_count.to_le_bytes());
            payload.extend_from_slice(&member_count_engine.to_le_bytes());
            payload.extend_from_slice(&missing_member_count.to_le_bytes());
            payload.extend_from_slice(&leader_trait_count.to_le_bytes());
            payload.extend_from_slice(&parent_status.to_le_bytes());
            payload.extend_from_slice(&member_status.to_le_bytes());
            payload.extend_from_slice(&owner_status.to_le_bytes());
            payload.extend_from_slice(&leader_status.to_le_bytes());
            payload.extend_from_slice(&leader_trait_status.to_le_bytes());
            payload.extend_from_slice(&location_province_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for member_id in [101u32, 102u32, 17u32, 617u32, 1001u32, 1002u32] {
            payload.extend_from_slice(&member_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_ships_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            instance_id,
            flags,
            task_force_id,
            owner_tag_db_id,
            category_enum,
            strength_raw,
            organisation_raw,
            experience_raw,
            sunk_convoys,
            definition,
        ) in [
            (
                1001u32,
                COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                    | COMPACT_SHIP_ROW_FLAG_DEFINITION
                    | COMPACT_SHIP_ROW_FLAG_OWNER,
                617u32,
                47u32,
                3u32,
                10_000_000u32,
                9_000_000u32,
                500_000u32,
                2u32,
                "hull",
            ),
            (
                1002u32,
                COMPACT_SHIP_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                "",
            ),
        ] {
            payload.extend_from_slice(&instance_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&task_force_id.to_le_bytes());
            payload.extend_from_slice(&category_enum.to_le_bytes());
            payload.extend_from_slice(&strength_raw.to_le_bytes());
            payload.extend_from_slice(&organisation_raw.to_le_bytes());
            payload.extend_from_slice(&experience_raw.to_le_bytes());
            payload.extend_from_slice(&sunk_convoys.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            append_test_compact_string(&mut payload, definition);
        }
        payload
    }

    fn test_empty_ships_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn test_air_wings_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            owner_tag_db_id,
            wing_id,
            generation,
            flags,
            count,
            mission_mask,
            mission_region,
            effective_planes,
        ) in [
            (
                1u32,
                4u32,
                0u64,
                COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION,
                24u32,
                8u32,
                1u32,
                22u32,
            ),
            (
                1u32,
                12u32,
                1u64,
                COMPACT_AIR_WING_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&wing_id.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&count.to_le_bytes());
            payload.extend_from_slice(&mission_mask.to_le_bytes());
            payload.extend_from_slice(&mission_region.to_le_bytes());
            payload.extend_from_slice(&effective_planes.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_air_region_activity_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            region_id,
            owner_tag_db_id,
            mission_mask,
            flags,
            wing_count,
            plane_count,
            effective_planes,
        ) in [
            (154u32, 1u32, 1u32, 0u32, 2u32, 200u32, 180u32),
            (
                164u32,
                2u32,
                4u32,
                COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&region_id.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&mission_mask.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&wing_count.to_le_bytes());
            payload.extend_from_slice(&plane_count.to_le_bytes());
            payload.extend_from_slice(&effective_planes.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_air_wing_loadouts_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (owner_tag_db_id, wing_id, generation, flags, air_base_id, entry_first, entry_count) in [
            (1u32, 4u32, 7u64, 0u32, 154u32, 0u32, 2u32),
            (
                1u32,
                5u32,
                8u64,
                COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&wing_id.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&air_base_id.to_le_bytes());
            payload.extend_from_slice(&entry_first.to_le_bytes());
            payload.extend_from_slice(&entry_count.to_le_bytes());
        }

        for (variant_id, amount_raw) in [(1375u32, 8_400_000i64), (5578u32, 1_600_000i64)] {
            payload.extend_from_slice(&variant_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&amount_raw.to_le_bytes());
        }
        payload
    }

    fn test_air_variant_defs_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&140u32.to_le_bytes());
        payload.extend_from_slice(&413u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (variant_id, flags, module_first, module_count) in [
            (
                1371u32,
                COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME
                    | COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME
                    | COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES,
                0u32,
                2u32,
            ),
            (1325u32, 0u32, 2u32, 0u32),
        ] {
            payload.extend_from_slice(&variant_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&module_first.to_le_bytes());
            payload.extend_from_slice(&module_count.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }

        append_test_compact_string(&mut payload, "Do 17");
        append_test_compact_string(&mut payload, "A");
        append_test_compact_string(&mut payload, "");
        append_test_compact_string(&mut payload, "");

        for (slot_handle, token) in [(28689u32, "medium_bomb_bay"), (28691u32, "engine_1_2x")] {
            payload.extend_from_slice(&slot_handle.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            append_test_compact_string(&mut payload, token);
        }
        payload
    }

    fn test_air_variant_stats_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&140u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&100_000u32.to_le_bytes());
        payload.extend_from_slice(&1_000u32.to_le_bytes());

        for (
            variant_id,
            equipment_token_raw,
            equipment_type_token_raw,
            interface_type_raw,
            speed_cache_raw,
            range_raw,
            air_attack_raw,
            agility_raw,
            max_speed_raw,
            reliability_raw,
            air_defence_raw,
            thrust_raw,
            fuel_consumption_raw,
            build_cost_ic_raw,
        ) in [
            (
                1371u32,
                18_204u32,
                70u32,
                7u32,
                11_000i32,
                700_000i32,
                500_000i32,
                3_850_000i32,
                40_000_000i32,
                80_000i32,
                400_000i32,
                1_100_000i32,
                21_000i32,
                1_572_500i32,
            ),
            (
                1375u32,
                18_208u32,
                70u32,
                7u32,
                16_000i32,
                1_500_000i32,
                100_000i32,
                3_000_000i32,
                20_000_000i32,
                80_000i32,
                900_000i32,
                2_500_000i32,
                32_000i32,
                3_510_000i32,
            ),
        ] {
            payload.extend_from_slice(&variant_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&equipment_token_raw.to_le_bytes());
            payload.extend_from_slice(&equipment_type_token_raw.to_le_bytes());
            payload.extend_from_slice(&interface_type_raw.to_le_bytes());
            payload.extend_from_slice(&speed_cache_raw.to_le_bytes());
            payload.extend_from_slice(&range_raw.to_le_bytes());
            payload.extend_from_slice(&air_attack_raw.to_le_bytes());
            payload.extend_from_slice(&agility_raw.to_le_bytes());
            payload.extend_from_slice(&max_speed_raw.to_le_bytes());
            payload.extend_from_slice(&reliability_raw.to_le_bytes());
            payload.extend_from_slice(&air_defence_raw.to_le_bytes());
            payload.extend_from_slice(&thrust_raw.to_le_bytes());
            payload.extend_from_slice(&fuel_consumption_raw.to_le_bytes());
            payload.extend_from_slice(&build_cost_ic_raw.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_air_base_status_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&367u32.to_le_bytes());
        payload.extend_from_slice(&5u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            air_base_id,
            state_id,
            flags,
            capacity,
            building_level,
            based_wing_count,
            based_plane_count,
        ) in [
            (154u32, 2u32, 0u32, 1600u32, 8u32, 3u32, 278u32),
            (176u32, 3u32, 0u32, 800u32, 4u32, 1u32, 100u32),
        ] {
            payload.extend_from_slice(&air_base_id.to_le_bytes());
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&capacity.to_le_bytes());
            payload.extend_from_slice(&building_level.to_le_bytes());
            payload.extend_from_slice(&based_wing_count.to_le_bytes());
            payload.extend_from_slice(&based_plane_count.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_land_combats_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for (
            combat_id,
            generation,
            flags,
            location,
            day,
            duration,
            attacker_count,
            defender_count,
        ) in [
            (271u32, 3u64, 0b1_1111u32, 118u32, 12u32, 34u32, 2u32, 1u32),
            (272u32, 4u64, 0b1_1111u32, 119u32, 2u32, 3u32, 0u32, 0u32),
        ] {
            payload.extend_from_slice(&combat_id.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&location.to_le_bytes());
            payload.extend_from_slice(&day.to_le_bytes());
            payload.extend_from_slice(&duration.to_le_bytes());
            payload.extend_from_slice(&attacker_count.to_le_bytes());
            payload.extend_from_slice(&defender_count.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for (combat_id, generation, side, division_id) in [
            (271u32, 3u64, 1u32, 1053u32),
            (271u32, 3u64, 1u32, 2837u32),
            (271u32, 3u64, 2u32, 465u32),
        ] {
            payload.extend_from_slice(&combat_id.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(&side.to_le_bytes());
            payload.extend_from_slice(&division_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_map_control_data_payload(status_flags: u32) -> Vec<u8> {
        let state_count = 1_100u32;
        let province_count = 10_100u32;
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&(state_count + province_count).to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        for state_id in 1..=state_count {
            let owner_tag_db_id = (state_id % 100) + 1;
            let controller_tag_db_id = owner_tag_db_id;
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&controller_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&0b11u32.to_le_bytes());
        }
        for province_id in 1..=province_count {
            let controller_tag_db_id = (province_id % 100) + 1;
            let state_id = ((province_id - 1) % state_count) + 1;
            payload.extend_from_slice(&province_id.to_le_bytes());
            payload.extend_from_slice(&controller_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&0b11u32.to_le_bytes());
        }
        payload
    }

    fn test_map_control_delta_data_payload(state_count: u32, province_count: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&(state_count + province_count).to_le_bytes());
        payload.extend_from_slice(&COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        payload
    }

    fn test_map_detail_data_payload(status_flags: u32) -> Vec<u8> {
        let state_count = 1_100u32;
        let province_count = 10_100u32;
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&(state_count + province_count).to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&state_count.to_le_bytes());
        payload.extend_from_slice(&province_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for state_id in 1..=state_count {
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&0b11u32.to_le_bytes());
            payload.extend_from_slice(&(state_id - 1).to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&(state_id - 1).to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for province_id in 1..=province_count {
            let state_id = ((province_id - 1) % state_count) + 1;
            payload.extend_from_slice(&province_id.to_le_bytes());
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&0b11u32.to_le_bytes());
            payload.extend_from_slice(&(province_id - 1).to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
        }
        for _ in 0..state_count {
            payload.extend_from_slice(&0x3E03u32.to_le_bytes());
            payload.extend_from_slice(&700_000i32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for _ in 0..state_count {
            payload.extend_from_slice(&0x2FABu32.to_le_bytes());
            payload.extend_from_slice(&5u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for _ in 0..province_count {
            payload.extend_from_slice(&0x31F6u32.to_le_bytes());
            payload.extend_from_slice(&10u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_weather_data_payload(status_flags: u32) -> Vec<u8> {
        let region_count = COMPACT_WEATHER_REGION_ROWS as u32;
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&region_count.to_le_bytes());
        payload.extend_from_slice(&region_count.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&region_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for region_id in 0..region_count {
            let active = region_id == 31;
            let flags = if active {
                1u32 | (1u32 << 5) | (1u32 << 6)
            } else {
                0
            };
            let phenomenon = if active { 1u32 } else { 0u32 };
            let intensity = if active { 2u32 } else { 0u32 };
            payload.extend_from_slice(&region_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&1_000_000i32.to_le_bytes());
            payload.extend_from_slice(&phenomenon.to_le_bytes());
            payload.extend_from_slice(&43_817_520u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&intensity.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn append_test_compact_string(payload: &mut Vec<u8>, value: &str) {
        let bytes = value.as_bytes();
        payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        payload.extend_from_slice(bytes);
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
