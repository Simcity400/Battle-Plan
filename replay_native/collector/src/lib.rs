#![deny(unsafe_op_in_unsafe_fn)]

use core::{ptr, slice};

pub const ABI_VERSION: u32 = 1;
pub const COLLECTOR_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COLLECTOR_PROFILE_ID: &str = "vanilla_latest_2026_06_28";

const HEARTBEAT_JSON: &[u8] = br#"{"type":"heartbeat","backend":"in_process_collector","collector_version":"0.1.0","abi_version":1,"domains":[],"quality_status":"ok"}"#;

#[no_mangle]
pub extern "C" fn battle-plan_collector_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn battle-plan_collector_heartbeat_len() -> usize {
    HEARTBEAT_JSON.len()
}

/// Copy the static heartbeat JSON into `out`.
///
/// Returns the full required byte length even when `cap` is too small. The bytes
/// are not NUL-terminated; callers should use the returned length contract.
#[no_mangle]
pub unsafe extern "C" fn battle-plan_collector_heartbeat_json(out: *mut u8, cap: usize) -> usize {
    if !out.is_null() && cap != 0 {
        let n = HEARTBEAT_JSON.len().min(cap);
        unsafe {
            ptr::copy_nonoverlapping(HEARTBEAT_JSON.as_ptr(), out, n);
        }
    }
    HEARTBEAT_JSON.len()
}

pub fn heartbeat_json_for_tests() -> &'static [u8] {
    HEARTBEAT_JSON
}

/// Test helper mirroring the C ABI with a Rust slice.
pub fn copy_heartbeat_json(out: &mut [u8]) -> usize {
    let n = HEARTBEAT_JSON.len().min(out.len());
    out[..n].copy_from_slice(&HEARTBEAT_JSON[..n]);
    HEARTBEAT_JSON.len()
}

/// Validate that a raw pointer/length pair can be viewed as a caller-owned byte
/// slice. This is not exported yet; it is the shape future transport functions
/// will use for frame buffers.
unsafe fn _caller_buffer<'a>(ptr: *mut u8, len: usize) -> Option<&'a mut [u8]> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { slice::from_raw_parts_mut(ptr, len) })
}

#[cfg(windows)]
mod attach_log {
    use core::ffi::c_void;
    use core::ptr;
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::fs::{read_to_string, OpenOptions};
    use std::io::Write;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    const DLL_PROCESS_ATTACH: u32 = 1;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type Hmodule = *mut c_void;
    type Lpvoid = *mut c_void;
    type ThreadStart = Option<unsafe extern "system" fn(Lpvoid) -> Dword>;

    static mut DLL_MODULE: Hmodule = ptr::null_mut();

    const MEM_COMMIT: Dword = 0x1000;
    const PAGE_NOACCESS: Dword = 0x01;
    const PAGE_GUARD: Dword = 0x100;
    const GENERIC_WRITE: Dword = 0x40000000;
    const OPEN_EXISTING: Dword = 3;
    const FILE_ATTRIBUTE_NORMAL: Dword = 0x80;
    const C_GAME_STATE_DAILY_UPDATE_OFFSET: usize = 0x1D4540;
    const CG_STATE_DATE_OFFSET: usize = 0x33048C0;
    const COUNTRY_DATABASE_OFFSET: usize = 0x33063F8;
    const GET_LONG_STRING_OFFSET: usize = 0x5269B0;
    const COUNTRY_VTABLE_RVA: usize = 0x27C0E80;
    const GS_CURRENT_DATE_OFFSET: usize = 0x468;
    const GS_START_DATE_OFFSET: usize = 0x4A8;
    const DATE_EPOCH_CONST: i32 = 43_799_999;
    const HEAP_LO: usize = 0x10000000000;
    const HEAP_HI: usize = 0x7FF000000000;
    const OBJ_TAG_DB_ID: usize = 0x08;
    const OBJ_NAME: usize = 0x10;
    const COUNTRY_RECORD_OBJECT_PTR: usize = 0x08;
    const COUNTRY_RECORD_POLITICAL_POWER: usize = 0x20;
    const STATE_VTABLE_RVA: usize = 0x2910920;
    const STATE_OFF_SID: usize = 0x58;
    const STATE_OFF_OWNER: usize = 0xC8;
    const STATE_OFF_CTRL: usize = 0xCC;
    const OBJ_TEMPLATES_LOCKED: usize = 0x01B4;
    const OBJ_COMMAND_POWER: usize = 0x01F0;
    const OBJ_MANPOWER_POOL: usize = 0x0338;
    const OBJ_PROD_MANAGER: usize = 0x0F68;
    const OBJ_FUEL_STATUS: usize = 0x1580;
    const OBJ_ORIGINAL_CAPITAL: usize = 0x101C;
    const OBJ_STABILITY: usize = 0x10D0;
    const OBJ_WAR_SUPPORT: usize = 0x10D8;
    const OBJ_CONVOYS: usize = 0x1280;
    const OBJ_ACE_COUNT: usize = 0x12E4;
    const OBJ_ORIGINAL_TAG: usize = 0x130C;
    const OBJ_NUM_SHIPS: usize = 0x1330;
    const OBJ_CONVOYS_DESTROYED: usize = 0x1334;
    const OBJ_COSMETIC_TAG: usize = 0x1488;
    const OBJ_RESERVED_DYNAMIC_COUNTRY: usize = 0x145D;
    const OBJ_ACCIDENTS_SCORE: usize = 0x14E8;
    const FUEL_STATUS_CURRENT: usize = 0x08;
    const FUEL_STATUS_MAX: usize = 0x10;
    const FUEL_STATUS_GAIN: usize = 0x18;
    const FUEL_STATUS_COST: usize = 0x38;
    const FUEL_SCALE: f64 = 32768.0;
    const MAX_FUEL_MEASURE: f64 = 1_000_000_000.0;
    const VARIANT_VTABLE_RVA: usize = 0x292B218;
    const VARIANT_TYPE: u32 = 70;
    const STOCK_BEGIN: usize = 0x220;
    const STOCK_CAP: usize = 0x228;
    const STOCK_SIZE: usize = 0x22C;
    const STOCK_STRIDE: usize = 0x10;
    const STOCK_ENTRY_DEF: usize = 0x00;
    const STOCK_ENTRY_AMOUNT: usize = 0x08;
    const VARIANT_DEF_TYPE: usize = 0x08;
    const VARIANT_DEF_ID: usize = 0x0C;
    const LINE_BEGIN: usize = 0x58;
    const LINE_CAP: usize = 0x60;
    const LINE_SIZE: usize = 0x64;
    const LINE_STRIDE: usize = 0x08;
    const LINE_TYPE: usize = 0x08;
    const LINE_ID: usize = 0x0C;
    const LINE_ACTIVE_FACTORIES: usize = 0x18;
    const LINE_COST: usize = 0x28;
    const LINE_SPEED: usize = 0x30;
    const LINE_PRODUCED: usize = 0x38;
    const LINE_AMOUNT: usize = 0x48;
    const LINE_TYPE_MILITARY: u32 = 56;
    const LINE_TYPE_NAVAL: u32 = 57;
    const MAX_PRODUCTION_VECTOR_ENTRIES: u32 = 4096;
    const MAX_PRODUCTION_STOCKPILE_AMOUNT: f64 = 1_000_000_000_000.0;
    const MAX_PRODUCTION_LINE_FIXED: f64 = 1_000_000_000.0;
    const MAX_COUNTRY_OBJECT_BRIDGE_ROWS: usize = 512;
    const MAX_MAP_CONTROL_STATE_ROOT_ROWS: usize = 4096;
    const MAX_MAP_CONTROL_STATE_ROOT_PAYLOAD_BYTES: usize = 131_072;
    const MAX_MAP_CONTROL_STATE_ID: u32 = 65_535;
    const MAX_MAP_CONTROL_PROVINCE_BRIDGE_PAYLOAD_BYTES: usize = 512;
    const MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS: usize = 1024;
    const MAX_MAP_CONTROL_CONTROLLER_BRIDGE_PAYLOAD_BYTES: usize = 32_768;
    const MAX_MAP_CONTROL_PROVINCE_COUNT: u32 = 65_536;
    const MAX_MAP_CONTROL_PROVINCE_STRIDE: u32 = 64;
    const MAP_CONTROL_POINTER_WIDTH: u32 = 8;
    const MIN_USABLE_STATE_ROWS: usize = 1_000;
    const MIN_USABLE_PROVINCE_ROWS: usize = 10_000;
    const DIVISION_TYPE: u32 = 4713;
    const DIVISION_VTABLE_RVA: usize = 0x2933F00;
    const DIVISION_MOVE_ORDER_VTABLE_RVA: usize = 0x294AE78;
    const DIVISION_OFF_ID_TYPE: usize = 0x08;
    const DIVISION_OFF_ID_ID: usize = 0x0C;
    const DIVISION_OFF_LOCATION: usize = 0x18;
    const DIVISION_OFF_COMBAT_REF_COUNT: usize = 0x1A4;
    const DIVISION_OFF_MOVE_ORDER: usize = 0x1E8;
    const DIVISION_OFF_MOVE_PROGRESS: usize = 0x230;
    const DIVISION_OFF_STRENGTH: usize = 0x410;
    const DIVISION_OFF_ORGANISATION: usize = 0x418;
    const DIVISION_MOVE_ORDER_OFF_SOURCE: usize = 0xA4;
    const MAX_DIVISION_BODY_BRIDGE_ROWS: usize = 20_000;
    const MAX_DIVISION_BODY_BRIDGE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
    const MAX_DIVISION_BODY_BRIDGE_DIV_ID: u32 = i32::MAX as u32;
    const MAX_DIVISION_PROVINCE_ID: u32 = 1_000_000;
    const MAX_DIVISION_COMBAT_REF_COUNT: u32 = 1_000;
    const MAX_DIVISION_ORGANISATION_RAW: u32 = 100_000_000;
    const MAX_DIVISION_STRENGTH_RAW: u32 = 1_000_000_000;
    const DIVISION_DELTA_BUCKET_RAW: u32 = 1_000;
    const FP_SCALE: f64 = 100000.0;
    const MAX_COUNTRY_ACE_COUNT: u32 = 100_000;
    const MAX_COUNTRY_CONVOYS_DESTROYED: u32 = 100_000_000;
    const MAX_COUNTRY_ACCIDENTS_SCORE: f64 = 10.0;
    const MAX_COUNTRY_COSMETIC_TAG_LEN: usize = 96;
    const DAILY_UPDATE_SENTINEL: [u8; 16] = [
        0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D,
        0xAC,
    ];
    const GET_LONG_STRING_SENTINEL: [u8; 16] = [
        0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x57,
    ];
    static FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: Lpvoid,
        allocation_base: Lpvoid,
        allocation_protect: Dword,
        partition_id: u16,
        region_size: usize,
        state: Dword,
        protect: Dword,
        type_: Dword,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CloseHandle(handle: Handle) -> Bool;
        fn CreateFileW(
            filename: *const u16,
            desired_access: Dword,
            share_mode: Dword,
            security_attributes: Lpvoid,
            creation_disposition: Dword,
            flags_and_attributes: Dword,
            template_file: Handle,
        ) -> Handle;
        fn CreateThread(
            attrs: Lpvoid,
            stack_size: usize,
            start: ThreadStart,
            param: Lpvoid,
            flags: Dword,
            thread_id: *mut Dword,
        ) -> Handle;
        fn DisableThreadLibraryCalls(module: Hmodule) -> Bool;
        fn GetCurrentProcessId() -> Dword;
        fn GetModuleFileNameW(module: Hmodule, filename: *mut u16, size: Dword) -> Dword;
        fn GetModuleHandleW(module_name: *const u16) -> Hmodule;
        fn GetTickCount64() -> u64;
        fn VirtualQuery(
            address: *const c_void,
            buffer: *mut MemoryBasicInformation,
            length: usize,
        ) -> usize;
        fn WriteFile(
            file: Handle,
            buffer: *const c_void,
            bytes_to_write: Dword,
            bytes_written: *mut Dword,
            overlapped: Lpvoid,
        ) -> Bool;
    }

    pub unsafe fn dll_main(module: Hmodule, reason: Dword) -> Bool {
        if reason == DLL_PROCESS_ATTACH {
            unsafe {
                DLL_MODULE = module;
                DisableThreadLibraryCalls(module);
                let handle = CreateThread(
                    ptr::null_mut(),
                    0,
                    Some(attach_worker),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                );
                if !handle.is_null() {
                    CloseHandle(handle);
                }
            }
        }
        1
    }

    unsafe extern "system" fn attach_worker(_param: Lpvoid) -> Dword {
        write_attach_heartbeat();
        0
    }

    fn write_attach_heartbeat() {
        let Some(mut path) = module_path() else {
            return;
        };
        let control = if is_hoi4_process() {
            collector_control(&path)
        } else {
            None
        };
        let state = if is_hoi4_process() {
            Some(CollectorState::resolve(control.as_ref()))
        } else {
            None
        };
        let pid = unsafe { GetCurrentProcessId() };
        let sidecar_log = control
            .as_ref()
            .map(|control| control.sidecar_log)
            .unwrap_or(true);
        path.set_file_name("collector_attach.jsonl");
        let mut file = if sidecar_log {
            OpenOptions::new().create(true).append(true).open(path).ok()
        } else {
            None
        };
        let mut pipe = open_pipe(
            pid,
            control
                .as_ref()
                .and_then(|control| control.pipe_token.as_deref()),
        );
        if let Some(control) = control {
            let worker_started = Instant::now();
            let deadline = worker_started + Duration::from_millis(control.duration_ms as u64);
            let mut session_sequence = 1u64;
            let mut last_scheduled_game_hour: Option<i32> = None;
            let mut previous_division_movement: BTreeMap<u32, DivisionMovementSample> =
                BTreeMap::new();
            while Instant::now() <= deadline {
                let elapsed_wall_ms = duration_millis_u64(worker_started.elapsed());
                let capture_boundary = capture_boundary_for_frame(
                    state.as_ref(),
                    &control,
                    &mut last_scheduled_game_hour,
                );
                let line = build_frame_line(
                    pid,
                    state.as_ref(),
                    Some(&control),
                    Some(session_sequence),
                    Some(elapsed_wall_ms),
                    Some(capture_boundary),
                );
                write_sinks(file.as_mut(), pipe, line.as_bytes());
                session_sequence = session_sequence.saturating_add(1);
                if let Some(line) = build_clock_raw_domain_delta_line(
                    pid,
                    state.as_ref(),
                    &control,
                    Some(session_sequence),
                    Some(elapsed_wall_ms),
                    capture_boundary,
                ) {
                    write_sinks(file.as_mut(), pipe, line.as_bytes());
                    session_sequence = session_sequence.saturating_add(1);
                }
                for line in build_division_raw_domain_delta_lines(
                    pid,
                    state.as_ref(),
                    &control,
                    &mut previous_division_movement,
                    session_sequence,
                    Some(elapsed_wall_ms),
                    capture_boundary,
                ) {
                    write_sinks(file.as_mut(), pipe, line.as_bytes());
                    session_sequence = session_sequence.saturating_add(1);
                }
                thread::sleep(Duration::from_millis(control.sample_ms as u64));
            }
        } else {
            let line = build_frame_line(pid, state.as_ref(), None, None, None, None);
            write_sinks(file.as_mut(), pipe, line.as_bytes());
        }
        if let Some(handle) = pipe.take() {
            unsafe {
                CloseHandle(handle);
            }
        }
    }

    struct CollectorControl {
        duration_ms: u32,
        sample_ms: u32,
        country_object_scan: bool,
        country_object_bridge: Vec<CountryObjectBridgeEntry>,
        country_object_bridge_valid: bool,
        country_record_bridge: Vec<CountryObjectBridgeEntry>,
        country_record_bridge_valid: bool,
        division_body_bridge: Vec<DivisionBodyBridgeEntry>,
        division_body_bridge_valid: bool,
        map_control: MapControlContract,
        scheduled_snapshot_step_hours: u32,
        pipe_token: Option<String>,
        session_id: Option<String>,
        sidecar_log: bool,
    }

    #[derive(Clone, Copy)]
    struct CountryObjectBridgeEntry {
        tag_db_id: u32,
        object_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct DivisionBodyBridgeEntry {
        div_id: u32,
        body_ptr: usize,
    }

    #[derive(Clone)]
    struct MapControlContract {
        state_roots: Vec<MapControlStateRootEntry>,
        state_roots_valid: bool,
        province_bridge: Option<MapControlProvinceBridge>,
        province_bridge_valid: bool,
        controller_bridge: Vec<MapControlControllerBridgeEntry>,
        controller_bridge_valid: bool,
    }

    impl MapControlContract {
        fn empty() -> Self {
            Self {
                state_roots: Vec::new(),
                state_roots_valid: true,
                province_bridge: None,
                province_bridge_valid: true,
                controller_bridge: Vec::new(),
                controller_bridge_valid: true,
            }
        }

        fn state_roots_well_formed(&self) -> bool {
            if self.state_roots.len() > MAX_MAP_CONTROL_STATE_ROOT_ROWS {
                return false;
            }
            let mut seen_state_ids: BTreeMap<u32, ()> = BTreeMap::new();
            let mut seen_state_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
            for row in &self.state_roots {
                if row.state_id == 0 || row.state_id > MAX_MAP_CONTROL_STATE_ID {
                    return false;
                }
                if !is_map_control_heap_pointer(row.state_ptr) {
                    return false;
                }
                if seen_state_ids.insert(row.state_id, ()).is_some()
                    || seen_state_ptrs.insert(row.state_ptr, ()).is_some()
                {
                    return false;
                }
            }
            true
        }

        fn province_bridge_well_formed(&self) -> bool {
            match self.province_bridge.as_ref() {
                Some(bridge) => validate_map_control_province_bridge(bridge),
                None => true,
            }
        }

        fn controller_bridge_well_formed(&self) -> bool {
            if self.controller_bridge.len() > MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS {
                return false;
            }
            let mut seen_tag_ids: BTreeMap<u32, ()> = BTreeMap::new();
            let mut seen_controller_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
            for row in &self.controller_bridge {
                if row.tag_db_id == 0 || row.tag_db_id > 1024 {
                    return false;
                }
                if !is_map_control_heap_pointer(row.controller_ptr) {
                    return false;
                }
                if seen_tag_ids.insert(row.tag_db_id, ()).is_some()
                    || seen_controller_ptrs
                        .insert(row.controller_ptr, ())
                        .is_some()
                {
                    return false;
                }
            }
            true
        }
    }

    #[derive(Clone, Copy)]
    struct MapControlStateRootEntry {
        state_id: u32,
        state_ptr: usize,
    }

    #[derive(Clone, Copy)]
    struct MapControlProvinceBridge {
        province_array_ptr: usize,
        province_count: u32,
        province_stride: u32,
        controller_ptr_offset: u32,
    }

    #[derive(Clone, Copy)]
    struct MapControlControllerBridgeEntry {
        tag_db_id: u32,
        controller_ptr: usize,
    }

    fn collector_control(module_path: &PathBuf) -> Option<CollectorControl> {
        let mut path = module_path.clone();
        path.set_file_name("collector_control.json");
        let text = read_to_string(path).ok()?;
        if !text.contains("\"mode\"") || !text.contains("hourly_poll") {
            return None;
        }
        let duration_ms = parse_json_u32(&text, "duration_ms").unwrap_or(5_000);
        let sample_ms = parse_json_u32(&text, "sample_ms").unwrap_or(50);
        let scheduled_snapshot_step_hours =
            parse_json_u32(&text, "scheduled_snapshot_step_hours").unwrap_or(1);
        let country_object_scan = parse_json_bool(&text, "country_object_scan").unwrap_or(false);
        let (country_object_bridge, country_object_bridge_valid) =
            match parse_json_string(&text, "country_object_bridge") {
                Some(value) => match parse_country_object_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (country_record_bridge, country_record_bridge_valid) =
            match parse_json_string(&text, "country_record_bridge") {
                Some(value) => match parse_country_object_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let (division_body_bridge, division_body_bridge_valid) =
            match parse_json_string(&text, "division_body_bridge") {
                Some(value) => match parse_division_body_bridge(&value) {
                    Some(rows) => (rows, true),
                    None => (Vec::new(), false),
                },
                None => (Vec::new(), true),
            };
        let map_control = parse_map_control_contract(&text);
        let pipe_token = parse_json_string(&text, "pipe_token").and_then(clean_pipe_token);
        let session_id = parse_json_string(&text, "session_id").and_then(clean_session_id);
        let sidecar_log = parse_json_bool(&text, "sidecar_log").unwrap_or(false);
        Some(CollectorControl {
            duration_ms: duration_ms.clamp(100, 30_000),
            sample_ms: sample_ms.clamp(10, 5_000),
            country_object_scan,
            country_object_bridge,
            country_object_bridge_valid,
            country_record_bridge,
            country_record_bridge_valid,
            division_body_bridge,
            division_body_bridge_valid,
            map_control,
            scheduled_snapshot_step_hours: scheduled_snapshot_step_hours.clamp(1, 24),
            pipe_token,
            session_id,
            sidecar_log,
        })
    }

    fn parse_json_u32(text: &str, key: &str) -> Option<u32> {
        let marker = format!("\"{key}\"");
        let start = text.find(&marker)?;
        let rest = &text[start + marker.len()..];
        let colon = rest.find(':')?;
        let digits: String = rest[colon + 1..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    }

    fn parse_json_bool(text: &str, key: &str) -> Option<bool> {
        let marker = format!("\"{key}\"");
        let start = text.find(&marker)?;
        let rest = &text[start + marker.len()..];
        let colon = rest.find(':')?;
        let value: String = rest[colon + 1..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    fn parse_json_string(text: &str, key: &str) -> Option<String> {
        let marker = format!("\"{key}\"");
        let start = text.find(&marker)?;
        let rest = &text[start + marker.len()..];
        let colon = rest.find(':')?;
        let value = rest[colon + 1..].trim_start();
        let value = value.strip_prefix('"')?;
        let end = value.find('"')?;
        Some(value[..end].to_string())
    }

    fn clean_pipe_token(token: String) -> Option<String> {
        if token.is_empty() || token.len() > 64 {
            return None;
        }
        if token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            Some(token)
        } else {
            None
        }
    }

    fn clean_session_id(session_id: String) -> Option<String> {
        if session_id.is_empty() || session_id.len() > 96 {
            return None;
        }
        if session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            Some(session_id)
        } else {
            None
        }
    }

    fn parse_country_object_bridge(value: &str) -> Option<Vec<CountryObjectBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > 16_384 {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_tag_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_objects: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (tag_raw, ptr_raw) = raw.split_once(':')?;
            let tag_db_id = tag_raw.trim().parse::<u32>().ok()?;
            if tag_db_id == 0 || tag_db_id > 1024 {
                return None;
            }
            let object_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if object_ptr == 0 || object_ptr % 8 != 0 {
                return None;
            }
            if seen_tag_ids.insert(tag_db_id, ()).is_some()
                || seen_objects.insert(object_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(CountryObjectBridgeEntry {
                tag_db_id,
                object_ptr,
            });
            if entries.len() > MAX_COUNTRY_OBJECT_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn parse_division_body_bridge(value: &str) -> Option<Vec<DivisionBodyBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_DIVISION_BODY_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_div_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (div_raw, ptr_raw) = raw.split_once(':')?;
            let div_id = parse_bridge_u32(div_raw.trim())?;
            if div_id == 0 || div_id > MAX_DIVISION_BODY_BRIDGE_DIV_ID {
                return None;
            }
            let body_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(body_ptr) {
                return None;
            }
            if seen_div_ids.insert(div_id, ()).is_some()
                || seen_body_ptrs.insert(body_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(DivisionBodyBridgeEntry { div_id, body_ptr });
            if entries.len() > MAX_DIVISION_BODY_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn division_body_bridge_well_formed(rows: &[DivisionBodyBridgeEntry]) -> bool {
        if rows.len() > MAX_DIVISION_BODY_BRIDGE_ROWS {
            return false;
        }
        let mut seen_div_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_body_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for row in rows {
            if row.div_id == 0 || row.div_id > MAX_DIVISION_BODY_BRIDGE_DIV_ID {
                return false;
            }
            if !is_map_control_heap_pointer(row.body_ptr) {
                return false;
            }
            if seen_div_ids.insert(row.div_id, ()).is_some()
                || seen_body_ptrs.insert(row.body_ptr, ()).is_some()
            {
                return false;
            }
        }
        true
    }

    fn parse_map_control_contract(text: &str) -> MapControlContract {
        let mut contract = MapControlContract::empty();
        if let Some(value) = parse_json_string(text, "map_control_state_roots") {
            match parse_map_control_state_roots(&value) {
                Some(rows) => contract.state_roots = rows,
                None => contract.state_roots_valid = false,
            }
        }
        if let Some(value) = parse_json_string(text, "map_control_province_bridge") {
            if !value.trim().is_empty() {
                match parse_map_control_province_bridge(&value) {
                    Some(bridge) => contract.province_bridge = Some(bridge),
                    None => contract.province_bridge_valid = false,
                }
            }
        }
        if let Some(value) = parse_json_string(text, "map_control_controller_bridge") {
            match parse_map_control_controller_bridge(&value) {
                Some(rows) => contract.controller_bridge = rows,
                None => contract.controller_bridge_valid = false,
            }
        }
        contract
    }

    fn parse_map_control_state_roots(value: &str) -> Option<Vec<MapControlStateRootEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_MAP_CONTROL_STATE_ROOT_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_state_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_state_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (state_raw, ptr_raw) = raw.split_once(':')?;
            let state_id = parse_bridge_u32(state_raw.trim())?;
            if state_id == 0 || state_id > MAX_MAP_CONTROL_STATE_ID {
                return None;
            }
            let state_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(state_ptr) {
                return None;
            }
            if seen_state_ids.insert(state_id, ()).is_some()
                || seen_state_ptrs.insert(state_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(MapControlStateRootEntry {
                state_id,
                state_ptr,
            });
            if entries.len() > MAX_MAP_CONTROL_STATE_ROOT_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn parse_map_control_province_bridge(value: &str) -> Option<MapControlProvinceBridge> {
        if value.len() > MAX_MAP_CONTROL_PROVINCE_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut province_array_ptr = None;
        let mut province_count = None;
        let mut province_stride = None;
        let mut controller_ptr_offset = None;
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (key, raw_value) = raw.split_once('=')?;
            let key = key.trim();
            let raw_value = raw_value.trim();
            match key {
                "province_array" => {
                    if province_array_ptr.is_some() {
                        return None;
                    }
                    province_array_ptr = Some(parse_bridge_pointer(raw_value)?);
                }
                "province_count" => {
                    if province_count.is_some() {
                        return None;
                    }
                    province_count = Some(parse_bridge_u32(raw_value)?);
                }
                "province_stride" => {
                    if province_stride.is_some() {
                        return None;
                    }
                    province_stride = Some(parse_bridge_u32(raw_value)?);
                }
                "controller_ptr_offset" => {
                    if controller_ptr_offset.is_some() {
                        return None;
                    }
                    controller_ptr_offset = Some(parse_bridge_u32(raw_value)?);
                }
                _ => return None,
            }
        }
        let bridge = MapControlProvinceBridge {
            province_array_ptr: province_array_ptr?,
            province_count: province_count?,
            province_stride: province_stride?,
            controller_ptr_offset: controller_ptr_offset?,
        };
        validate_map_control_province_bridge(&bridge).then_some(bridge)
    }

    fn parse_map_control_controller_bridge(
        value: &str,
    ) -> Option<Vec<MapControlControllerBridgeEntry>> {
        if value.trim().is_empty() {
            return Some(Vec::new());
        }
        if value.len() > MAX_MAP_CONTROL_CONTROLLER_BRIDGE_PAYLOAD_BYTES {
            return None;
        }
        let mut entries = Vec::new();
        let mut seen_tag_ids: BTreeMap<u32, ()> = BTreeMap::new();
        let mut seen_controller_ptrs: BTreeMap<usize, ()> = BTreeMap::new();
        for raw in value.split(';') {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (tag_raw, ptr_raw) = raw.split_once(':')?;
            let tag_db_id = parse_bridge_u32(tag_raw.trim())?;
            if tag_db_id == 0 || tag_db_id > 1024 {
                return None;
            }
            let controller_ptr = parse_bridge_pointer(ptr_raw.trim())?;
            if !is_map_control_heap_pointer(controller_ptr) {
                return None;
            }
            if seen_tag_ids.insert(tag_db_id, ()).is_some()
                || seen_controller_ptrs.insert(controller_ptr, ()).is_some()
            {
                return None;
            }
            entries.push(MapControlControllerBridgeEntry {
                tag_db_id,
                controller_ptr,
            });
            if entries.len() > MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS {
                return None;
            }
        }
        Some(entries)
    }

    fn validate_map_control_province_bridge(bridge: &MapControlProvinceBridge) -> bool {
        if !is_map_control_heap_pointer(bridge.province_array_ptr) {
            return false;
        }
        if bridge.province_count == 0 || bridge.province_count > MAX_MAP_CONTROL_PROVINCE_COUNT {
            return false;
        }
        if bridge.province_stride < MAP_CONTROL_POINTER_WIDTH
            || bridge.province_stride > MAX_MAP_CONTROL_PROVINCE_STRIDE
            || bridge.province_stride % MAP_CONTROL_POINTER_WIDTH != 0
        {
            return false;
        }
        if bridge.controller_ptr_offset % MAP_CONTROL_POINTER_WIDTH != 0 {
            return false;
        }
        let Some(controller_end) = bridge
            .controller_ptr_offset
            .checked_add(MAP_CONTROL_POINTER_WIDTH)
        else {
            return false;
        };
        if controller_end > bridge.province_stride {
            return false;
        }
        let Some(span) =
            (bridge.province_count as usize).checked_mul(bridge.province_stride as usize)
        else {
            return false;
        };
        bridge
            .province_array_ptr
            .checked_add(span)
            .is_some_and(|end| end <= HEAP_HI)
    }

    fn is_map_control_heap_pointer(value: usize) -> bool {
        value >= HEAP_LO && value < HEAP_HI && value % (MAP_CONTROL_POINTER_WIDTH as usize) == 0
    }

    fn parse_bridge_u32(value: &str) -> Option<u32> {
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            value.parse::<u32>().ok()
        }
    }

    fn parse_bridge_pointer(value: &str) -> Option<usize> {
        if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            usize::from_str_radix(hex, 16).ok()
        } else {
            value.parse::<usize>().ok()
        }
    }

    fn build_frame_line(
        pid: Dword,
        state: Option<&CollectorState>,
        control: Option<&CollectorControl>,
        session_sequence: Option<u64>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: Option<&str>,
    ) -> String {
        let tick = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let scheduled_frame = capture_boundary == Some("scheduled_hour_snapshot");
        let include_scheduled_map = scheduled_frame;
        let include_scheduled_country_fuel = scheduled_frame;
        let include_scheduled_country_production = scheduled_frame;
        let state_rows = scheduled_state_rows_json(state, control, include_scheduled_map);
        let province_rows = scheduled_province_rows_json(state, control, include_scheduled_map);
        let (probe, domains) = root_probe_json(
            state,
            state_rows.as_deref(),
            province_rows.as_deref(),
            include_scheduled_country_fuel,
            include_scheduled_country_production,
        );
        let control_meta =
            control_metadata_json(control, session_sequence, elapsed_wall_ms, capture_boundary);
        let quality = if probe.contains("\"status\":\"verified\"") {
            "root_probe_verified"
        } else if probe.contains("\"status\":\"profile_mismatch\"") {
            "profile_mismatch"
        } else {
            "attach_noop"
        };
        format!(
            "{{\"type\":\"collector_started\",\"backend\":\"in_process_collector\",\
             \"collector_version\":\"{}\",\"abi_version\":{},\"process_id\":{},\
             \"tick_ms\":{},\"sequence\":{},\"profile_id\":\"{}\",\
             \"domains\":{},{}\"quality_status\":\"{}\",{}}}\n",
            super::COLLECTOR_VERSION,
            super::ABI_VERSION,
            pid,
            tick,
            sequence,
            super::COLLECTOR_PROFILE_ID,
            domains,
            control_meta,
            quality,
            probe
        )
    }

    const RAW_SOURCE: &str = "C";
    const RAW_CLOCK_DOMAIN: &str = "clock";
    const RAW_CLOCK_ENTITY_KEY: &str = "clock:game_date";
    const RAW_DIVISIONS_DOMAIN: &str = "divisions";
    const RAW_CONFIDENCE_SAMPLED: &str = "sampled";
    const RAW_SHORT_STRING_MAX_LENGTH: usize = 128;
    const RAW_STRING_MAX_LENGTH: usize = 512;
    const RAW_JSON_OBJECT_MAX_LENGTH: usize = 4096;

    struct ClockSample {
        date: String,
        hour_index: u64,
        raw_date: i32,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DivisionMovementSample {
        div_id: u32,
        moving: bool,
        raw_movement_progress: Option<u32>,
        progress_bucket: Option<u32>,
        location: u32,
        move_target_province: Option<u32>,
        movement_source_province: Option<u32>,
        raw_organisation: u32,
        organisation_bucket: u32,
        raw_strength: u32,
        strength_bucket: u32,
        combat_ref_count: u32,
        in_combat: bool,
    }

    struct RawDomainDeltaFrame<'a> {
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        date: &'a str,
        hour_index: u64,
        sample_ms: u32,
        duration_ms: Option<u32>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &'a str,
        domain: &'a str,
        entity_key: &'a str,
        confidence: &'a str,
        after_json: &'a str,
        changed_fields: &'a [&'a str],
    }

    fn build_clock_raw_domain_delta_line(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        session_sequence: Option<u64>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Option<String> {
        let sample = clock_sample_from_state(state)?;
        let tick_ms = unsafe { GetTickCount64() };
        let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        clock_raw_domain_delta_frame_line(
            pid,
            sequence,
            session_sequence,
            tick_ms,
            control,
            elapsed_wall_ms,
            capture_boundary,
            &sample,
        )
    }

    fn clock_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        sample: &ClockSample,
    ) -> Option<String> {
        let after_json = format!(
            "{{\"date\":{},\"hour_index\":{},\"raw_date\":{}}}",
            json_str(&sample.date),
            sample.hour_index,
            sample.raw_date
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &sample.date,
            hour_index: sample.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_CLOCK_DOMAIN,
            entity_key: RAW_CLOCK_ENTITY_KEY,
            confidence: RAW_CONFIDENCE_SAMPLED,
            after_json: &after_json,
            changed_fields: &["date", "hour_index", "raw_date"],
        })
    }

    fn clock_sample_from_state(state: Option<&CollectorState>) -> Option<ClockSample> {
        let raw_date = current_game_date_raw(state)?;
        let date = decode_datehour(raw_date)?;
        let hour_index = raw_date.checked_sub(DATE_EPOCH_CONST)? as u64;
        Some(ClockSample {
            date,
            hour_index,
            raw_date,
        })
    }

    fn build_division_raw_domain_delta_lines(
        pid: Dword,
        state: Option<&CollectorState>,
        control: &CollectorControl,
        previous: &mut BTreeMap<u32, DivisionMovementSample>,
        first_session_sequence: u64,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
    ) -> Vec<String> {
        if !(control.division_body_bridge_valid
            && division_body_bridge_well_formed(&control.division_body_bridge))
        {
            return Vec::new();
        }
        let Some(state) = state else {
            return Vec::new();
        };
        if control.division_body_bridge.is_empty() {
            return Vec::new();
        }
        let Some(clock) = clock_sample_from_state(Some(state)) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let mut session_sequence = first_session_sequence;
        for entry in &control.division_body_bridge {
            let Some(sample) = sample_division_movement(state.base, entry) else {
                continue;
            };
            let Some(changed_fields) = track_division_movement_sample(previous, &sample) else {
                continue;
            };
            let tick_ms = unsafe { GetTickCount64() };
            let sequence = FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            if let Some(line) = division_raw_domain_delta_frame_line(
                pid,
                sequence,
                Some(session_sequence),
                tick_ms,
                control,
                elapsed_wall_ms,
                capture_boundary,
                &clock,
                &sample,
                &changed_fields,
            ) {
                lines.push(line);
                session_sequence = session_sequence.saturating_add(1);
            }
        }
        lines
    }

    fn sample_division_movement(
        module_base: usize,
        entry: &DivisionBodyBridgeEntry,
    ) -> Option<DivisionMovementSample> {
        let body = entry.body_ptr;
        if read_u64(body)? as usize != module_base.checked_add(DIVISION_VTABLE_RVA)? {
            return None;
        }
        if read_u32(body.checked_add(DIVISION_OFF_ID_TYPE)?)? != DIVISION_TYPE {
            return None;
        }
        if read_u32(body.checked_add(DIVISION_OFF_ID_ID)?)? != entry.div_id {
            return None;
        }
        let location = read_u32(body.checked_add(DIVISION_OFF_LOCATION)?)?;
        if !valid_division_province_id(location) {
            return None;
        }
        let combat_ref_count = read_u32(body.checked_add(DIVISION_OFF_COMBAT_REF_COUNT)?)?;
        if combat_ref_count > MAX_DIVISION_COMBAT_REF_COUNT {
            return None;
        }
        let raw_strength = read_division_fixed_raw(
            body.checked_add(DIVISION_OFF_STRENGTH)?,
            MAX_DIVISION_STRENGTH_RAW,
        )?;
        let raw_organisation = read_division_fixed_raw(
            body.checked_add(DIVISION_OFF_ORGANISATION)?,
            MAX_DIVISION_ORGANISATION_RAW,
        )?;
        let raw_progress = read_u32(body.checked_add(DIVISION_OFF_MOVE_PROGRESS)?)?;
        let moving = raw_progress != 0;
        let movement_source_province = if moving {
            read_division_movement_source_province(module_base, body)
        } else {
            None
        };
        Some(DivisionMovementSample {
            div_id: entry.div_id,
            moving,
            raw_movement_progress: moving.then_some(raw_progress),
            progress_bucket: moving.then_some(division_delta_bucket(raw_progress)),
            location,
            move_target_province: moving.then_some(location),
            movement_source_province,
            raw_organisation,
            organisation_bucket: division_delta_bucket(raw_organisation),
            raw_strength,
            strength_bucket: division_delta_bucket(raw_strength),
            combat_ref_count,
            in_combat: combat_ref_count > 0,
        })
    }

    fn read_division_movement_source_province(module_base: usize, body: usize) -> Option<u32> {
        let move_order_ptr = read_u64(body.checked_add(DIVISION_OFF_MOVE_ORDER)?)? as usize;
        if !is_map_control_heap_pointer(move_order_ptr) {
            return None;
        }
        if read_u64(move_order_ptr)? as usize
            != module_base.checked_add(DIVISION_MOVE_ORDER_VTABLE_RVA)?
        {
            return None;
        }
        read_u32(move_order_ptr.checked_add(DIVISION_MOVE_ORDER_OFF_SOURCE)?)
            .filter(|province| valid_division_province_id(*province))
    }

    fn valid_division_province_id(value: u32) -> bool {
        value > 0 && value <= MAX_DIVISION_PROVINCE_ID
    }

    fn read_division_fixed_raw(addr: usize, max_raw: u32) -> Option<u32> {
        read_u32(addr).filter(|value| *value <= max_raw)
    }

    fn division_delta_bucket(raw: u32) -> u32 {
        raw / DIVISION_DELTA_BUCKET_RAW
    }

    fn division_fixed_value(raw: u32) -> f64 {
        raw as f64 / FP_SCALE
    }

    fn track_division_movement_sample(
        previous: &mut BTreeMap<u32, DivisionMovementSample>,
        sample: &DivisionMovementSample,
    ) -> Option<Vec<&'static str>> {
        let changed_fields = match previous.get(&sample.div_id) {
            None if sample.moving => Some(division_movement_all_fields()),
            None => None,
            Some(old) => division_movement_changed_fields(old, sample),
        };
        previous.insert(sample.div_id, sample.clone());
        changed_fields
    }

    fn division_movement_all_fields() -> Vec<&'static str> {
        vec![
            "div_uid",
            "div_id",
            "div_type",
            "moving",
            "movement_progress",
            "progress_bucket",
            "location",
            "move_target_province",
            "movement_source_province",
            "organisation",
            "strength",
            "combat_ref_count",
            "in_combat",
        ]
    }

    fn division_movement_changed_fields(
        old: &DivisionMovementSample,
        new: &DivisionMovementSample,
    ) -> Option<Vec<&'static str>> {
        let mut changed = Vec::new();
        if old.moving != new.moving {
            changed.push("moving");
        }
        if old.progress_bucket != new.progress_bucket {
            changed.push("movement_progress");
            changed.push("progress_bucket");
        }
        if old.location != new.location {
            changed.push("location");
        }
        if old.move_target_province != new.move_target_province {
            changed.push("move_target_province");
        }
        if old.movement_source_province != new.movement_source_province {
            changed.push("movement_source_province");
        }
        if old.organisation_bucket != new.organisation_bucket {
            changed.push("organisation");
        }
        if old.strength_bucket != new.strength_bucket {
            changed.push("strength");
        }
        if old.combat_ref_count != new.combat_ref_count {
            changed.push("combat_ref_count");
        }
        if old.in_combat != new.in_combat {
            changed.push("in_combat");
        }
        (!changed.is_empty()).then_some(changed)
    }

    fn division_raw_domain_delta_frame_line(
        pid: Dword,
        sequence: u64,
        session_sequence: Option<u64>,
        tick_ms: u64,
        control: &CollectorControl,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: &str,
        clock: &ClockSample,
        sample: &DivisionMovementSample,
        changed_fields: &[&str],
    ) -> Option<String> {
        let movement_progress = sample.raw_movement_progress.map(division_fixed_value);
        let organisation = division_fixed_value(sample.raw_organisation);
        let strength = division_fixed_value(sample.raw_strength);
        let div_uid = format!("{}:{}", sample.div_id, DIVISION_TYPE);
        let entity_key = format!("division:{div_uid}");
        let after_json = format!(
            "{{\"div_uid\":{},\"div_id\":{},\"div_type\":{},\
             \"moving\":{},\"movement_progress\":{},\"progress_bucket\":{},\
             \"location\":{},\"move_target_province\":{},\
             \"movement_source_province\":{},\"organisation\":{},\
             \"strength\":{},\"combat_ref_count\":{},\"in_combat\":{}}}",
            json_str(&div_uid),
            sample.div_id,
            DIVISION_TYPE,
            sample.moving,
            json_opt_f64(movement_progress),
            json_opt_u32(sample.progress_bucket),
            sample.location,
            json_opt_u32(sample.move_target_province),
            json_opt_u32(sample.movement_source_province),
            json_opt_f64(Some(organisation)),
            json_opt_f64(Some(strength)),
            sample.combat_ref_count,
            sample.in_combat
        );
        raw_domain_delta_frame_line(RawDomainDeltaFrame {
            pid,
            sequence,
            session_sequence,
            tick_ms,
            date: &clock.date,
            hour_index: clock.hour_index,
            sample_ms: control.sample_ms,
            duration_ms: Some(control.duration_ms),
            elapsed_wall_ms,
            capture_boundary,
            domain: RAW_DIVISIONS_DOMAIN,
            entity_key: &entity_key,
            confidence: RAW_CONFIDENCE_SAMPLED,
            after_json: &after_json,
            changed_fields,
        })
    }

    fn raw_domain_delta_frame_line(frame: RawDomainDeltaFrame<'_>) -> Option<String> {
        if !valid_raw_short_string(frame.date)
            || !valid_raw_short_string(frame.capture_boundary)
            || !valid_raw_domain(frame.domain)
            || !valid_raw_key(frame.entity_key)
            || !valid_raw_confidence(frame.confidence)
            || !valid_raw_object_json(frame.after_json)
            || frame.changed_fields.len() > 256
            || frame
                .changed_fields
                .iter()
                .any(|field| !valid_raw_short_string(field))
        {
            return None;
        }

        let changed_fields = frame
            .changed_fields
            .iter()
            .map(|field| json_str(field))
            .collect::<Vec<_>>()
            .join(",");
        let mut fields = vec![
            "\"type\":\"domain_delta\"".to_string(),
            "\"backend\":\"in_process_collector\"".to_string(),
            format!("\"collector_version\":\"{}\"", super::COLLECTOR_VERSION),
            format!("\"abi_version\":{}", super::ABI_VERSION),
            format!("\"process_id\":{}", frame.pid),
            format!("\"tick_ms\":{}", frame.tick_ms),
            format!("\"sequence\":{}", frame.sequence),
            format!("\"profile_id\":\"{}\"", super::COLLECTOR_PROFILE_ID),
            format!("\"date\":{}", json_str(frame.date)),
            format!("\"hour_index\":{}", frame.hour_index),
            format!("\"sample_ms\":{}", frame.sample_ms),
            format!("\"capture_boundary\":{}", json_str(frame.capture_boundary)),
            format!("\"domain\":{}", json_str(frame.domain)),
            format!("\"entity_key\":{}", json_str(frame.entity_key)),
            format!("\"source\":\"{}\"", RAW_SOURCE),
            format!("\"confidence\":{}", json_str(frame.confidence)),
        ];
        if let Some(session_sequence) = frame.session_sequence {
            fields.push(format!("\"session_sequence\":{session_sequence}"));
        }
        if let Some(duration_ms) = frame.duration_ms {
            fields.push(format!("\"duration_ms\":{duration_ms}"));
        }
        if let Some(elapsed_wall_ms) = frame.elapsed_wall_ms {
            fields.push(format!("\"elapsed_wall_ms\":{elapsed_wall_ms}"));
        }
        fields.push(format!(
            "\"payload\":{{\"op\":\"sample\",\"after\":{},\"changed_fields\":[{}]}}",
            frame.after_json, changed_fields
        ));
        Some(format!("{{{}}}\n", fields.join(",")))
    }

    fn valid_raw_short_string(value: &str) -> bool {
        !value.is_empty() && value.len() <= RAW_SHORT_STRING_MAX_LENGTH
    }

    fn valid_raw_key(value: &str) -> bool {
        !value.is_empty() && value.len() <= RAW_STRING_MAX_LENGTH && !looks_like_hex_pointer(value)
    }

    fn valid_raw_domain(value: &str) -> bool {
        matches!(value, RAW_CLOCK_DOMAIN | RAW_DIVISIONS_DOMAIN)
    }

    fn valid_raw_confidence(value: &str) -> bool {
        matches!(value, "exact" | "sampled" | "proxy" | "gap")
    }

    fn looks_like_hex_pointer(value: &str) -> bool {
        let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        else {
            return false;
        };
        !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn valid_raw_object_json(value: &str) -> bool {
        let trimmed = value.trim();
        !trimmed.is_empty()
            && trimmed.len() <= RAW_JSON_OBJECT_MAX_LENGTH
            && trimmed.starts_with('{')
            && trimmed.ends_with('}')
    }

    fn json_str(value: &str) -> String {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('"');
        for ch in value.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                ch if ch.is_control() => out.push_str(&format!("\\u{:04X}", ch as u32)),
                ch => out.push(ch),
            }
        }
        out.push('"');
        out
    }

    fn control_metadata_json(
        control: Option<&CollectorControl>,
        session_sequence: Option<u64>,
        elapsed_wall_ms: Option<u64>,
        capture_boundary: Option<&str>,
    ) -> String {
        let Some(control) = control else {
            return String::new();
        };
        let capture_boundary = capture_boundary.unwrap_or("worker_poll");
        let session = control
            .session_id
            .as_deref()
            .map(|id| format!("\"session_id\":\"{id}\","))
            .unwrap_or_default();
        let session_sequence = session_sequence
            .map(|n| format!("\"session_sequence\":{n},"))
            .unwrap_or_default();
        let elapsed_wall_ms = elapsed_wall_ms
            .map(|n| format!("\"elapsed_wall_ms\":{n},"))
            .unwrap_or_default();
        let division_body_bridge_valid = control.division_body_bridge_valid
            && division_body_bridge_well_formed(&control.division_body_bridge);
        let map_control_state_roots_valid =
            control.map_control.state_roots_valid && control.map_control.state_roots_well_formed();
        let map_control_province_bridge_valid = control.map_control.province_bridge_valid
            && control.map_control.province_bridge_well_formed();
        let map_control_controller_bridge_valid = control.map_control.controller_bridge_valid
            && control.map_control.controller_bridge_well_formed();
        format!(
            "{session}{session_sequence}{elapsed_wall_ms}\"capture_boundary\":\"{capture_boundary}\",\
             \"duration_ms\":{},\"sample_ms\":{},\
             \"scheduled_snapshot_step_hours\":{},\
             \"country_object_bridge_rows\":{},\
             \"country_object_bridge_valid\":{},\
             \"country_record_bridge_rows\":{},\
             \"country_record_bridge_valid\":{},\
             \"division_body_bridge_rows\":{},\
             \"division_body_bridge_valid\":{},\
             \"map_control_state_root_rows\":{},\
             \"map_control_state_roots_valid\":{},\
             \"map_control_province_bridge_present\":{},\
             \"map_control_province_bridge_valid\":{},\
             \"map_control_controller_bridge_rows\":{},\
             \"map_control_controller_bridge_valid\":{},",
            control.duration_ms,
            control.sample_ms,
            control.scheduled_snapshot_step_hours,
            control.country_object_bridge.len(),
            control.country_object_bridge_valid,
            control.country_record_bridge.len(),
            control.country_record_bridge_valid,
            control.division_body_bridge.len(),
            division_body_bridge_valid,
            control.map_control.state_roots.len(),
            map_control_state_roots_valid,
            control.map_control.province_bridge.is_some(),
            map_control_province_bridge_valid,
            control.map_control.controller_bridge.len(),
            map_control_controller_bridge_valid
        )
    }

    fn capture_boundary_for_frame(
        state: Option<&CollectorState>,
        control: &CollectorControl,
        last_scheduled_game_hour: &mut Option<i32>,
    ) -> &'static str {
        let Some(current_raw) = current_game_date_raw(state) else {
            return "worker_poll";
        };
        let step = control.scheduled_snapshot_step_hours.max(1) as i32;
        let should_schedule = match *last_scheduled_game_hour {
            None => true,
            Some(previous_raw) => {
                current_raw != previous_raw && current_raw.saturating_sub(previous_raw) >= step
            }
        };
        if should_schedule {
            *last_scheduled_game_hour = Some(current_raw);
            "scheduled_hour_snapshot"
        } else {
            "worker_poll"
        }
    }

    fn current_game_date_raw(state: Option<&CollectorState>) -> Option<i32> {
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        read_i32(state.date_ptr as usize + GS_CURRENT_DATE_OFFSET)
    }

    fn duration_millis_u64(duration: Duration) -> u64 {
        duration.as_millis().min(u128::from(u64::MAX)) as u64
    }

    struct CollectorState {
        base: usize,
        daily_addr: usize,
        long_string_addr: usize,
        daily_match: bool,
        long_string_match: bool,
        date_ptr: u64,
        country_database_ptr: u64,
        country_identity: CountryIdentityCache,
    }

    impl CollectorState {
        fn resolve(control: Option<&CollectorControl>) -> CollectorState {
            let base = unsafe { GetModuleHandleW(ptr::null()) as usize };
            let daily_addr = base.saturating_add(C_GAME_STATE_DAILY_UPDATE_OFFSET);
            let long_string_addr = base.saturating_add(GET_LONG_STRING_OFFSET);
            let daily_match = base != 0 && sentinel_matches(daily_addr, &DAILY_UPDATE_SENTINEL);
            let long_string_match =
                base != 0 && sentinel_matches(long_string_addr, &GET_LONG_STRING_SENTINEL);
            let date_ptr = if daily_match && long_string_match {
                read_u64(base + CG_STATE_DATE_OFFSET).unwrap_or(0)
            } else {
                0
            };
            let country_database_ptr = if daily_match && long_string_match {
                read_u64(base + COUNTRY_DATABASE_OFFSET).unwrap_or(0)
            } else {
                0
            };
            let country_identity = if daily_match
                && long_string_match
                && control
                    .map(|control| {
                        !control.country_object_bridge_valid || !control.country_record_bridge_valid
                    })
                    .unwrap_or(false)
            {
                CountryIdentityCache::not_attempted("invalid_control_bridge")
            } else if daily_match
                && long_string_match
                && control
                    .map(|control| !control.country_object_bridge.is_empty())
                    .unwrap_or(false)
            {
                let bridge = control
                    .map(|control| control.country_object_bridge.as_slice())
                    .unwrap_or(&[]);
                let record_bridge = control
                    .map(|control| control.country_record_bridge.as_slice())
                    .unwrap_or(&[]);
                CountryIdentityCache::from_control_bridge(
                    base,
                    country_database_ptr as usize,
                    bridge,
                    record_bridge,
                )
            } else if daily_match
                && long_string_match
                && control
                    .map(|control| control.country_object_scan)
                    .unwrap_or(false)
            {
                CountryIdentityCache::resolve(base, country_database_ptr as usize)
            } else if daily_match && long_string_match {
                CountryIdentityCache::not_attempted("disabled_speed_safe")
            } else {
                CountryIdentityCache::not_attempted("profile_mismatch")
            };
            CollectorState {
                base,
                daily_addr,
                long_string_addr,
                daily_match,
                long_string_match,
                date_ptr,
                country_database_ptr,
                country_identity,
            }
        }
    }

    #[derive(Clone)]
    struct CountryObjectRow {
        tag: String,
        tag_db_id: u32,
        name: String,
        object_ptr: usize,
        record_ptr: Option<usize>,
        original_capital: Option<i32>,
        original_tag: Option<String>,
        templates_locked: Option<bool>,
        reserved_dynamic_country: Option<bool>,
        stability: Option<f64>,
        war_support: Option<f64>,
        political_power: Option<f64>,
        command_power: Option<f64>,
        num_ships: Option<i32>,
        manpower_pool: Option<u32>,
        convoys: Option<f64>,
        misc: Option<CountryMiscRow>,
        fuel: Option<CountryFuelRow>,
        production: Option<CountryProductionRow>,
    }

    #[derive(Clone)]
    struct CountryMiscRow {
        convoys_destroyed: Option<u32>,
        accidents_score: Option<f64>,
        ace_count: Option<u32>,
        cosmetic_tag: Option<Option<String>>,
    }

    impl CountryMiscRow {
        fn has_any(&self) -> bool {
            self.convoys_destroyed.is_some()
                || self.accidents_score.is_some()
                || self.ace_count.is_some()
                || self.cosmetic_tag.is_some()
        }
    }

    #[derive(Clone, Copy)]
    struct CountryFuelRow {
        current: f64,
        max: f64,
        gain: f64,
        cost: f64,
    }

    #[derive(Clone)]
    struct CountryProductionRow {
        stockpile: Option<Vec<CountryStockpileRow>>,
        military_lines: Option<Vec<CountryProductionLineRow>>,
        naval_lines: Option<Vec<CountryProductionLineRow>>,
    }

    #[derive(Clone, Copy)]
    struct CountryStockpileRow {
        variant_id: u32,
        amount: f64,
    }

    #[derive(Clone, Copy)]
    struct CountryProductionLineRow {
        id: i32,
        active_factories: i32,
        cost: f64,
        speed: f64,
        produced: f64,
        amount: i32,
    }

    struct CountryIdentityCache {
        status: &'static str,
        scan_ms: u64,
        country_vtable: usize,
        bridge_rows_supplied: u32,
        record_rows_supplied: u32,
        rows: Vec<CountryObjectRow>,
        rejected_objects: u32,
        duplicate_tags: u32,
    }

    impl CountryIdentityCache {
        fn not_attempted(status: &'static str) -> CountryIdentityCache {
            CountryIdentityCache {
                status,
                scan_ms: 0,
                country_vtable: 0,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows: Vec::new(),
                rejected_objects: 0,
                duplicate_tags: 0,
            }
        }

        fn resolve(base: usize, country_database_ptr: usize) -> CountryIdentityCache {
            let started = unsafe { GetTickCount64() };
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let country_vtable = base + COUNTRY_VTABLE_RVA;
            let mut rows_by_tag: BTreeMap<String, CountryObjectRow> = BTreeMap::new();
            let mut rejected_objects = 0u32;
            let mut duplicate_tags = 0u32;
            for object_ptr in scan_qword_hits(country_vtable, HEAP_LO, HEAP_HI) {
                let Some(row) =
                    country_object_row(object_ptr, country_vtable, tag_table, None, false, false)
                else {
                    rejected_objects = rejected_objects.saturating_add(1);
                    continue;
                };
                if rows_by_tag.insert(row.tag.clone(), row).is_some() {
                    duplicate_tags = duplicate_tags.saturating_add(1);
                }
            }
            let scan_ms = unsafe { GetTickCount64() }.saturating_sub(started);
            let rows: Vec<CountryObjectRow> = rows_by_tag.into_values().collect();
            let status = if rows.len() >= 40 {
                "verified"
            } else {
                "insufficient_country_objects"
            };
            CountryIdentityCache {
                status,
                scan_ms,
                country_vtable,
                bridge_rows_supplied: 0,
                record_rows_supplied: 0,
                rows,
                rejected_objects,
                duplicate_tags,
            }
        }

        fn from_control_bridge(
            base: usize,
            country_database_ptr: usize,
            bridge: &[CountryObjectBridgeEntry],
            record_bridge: &[CountryObjectBridgeEntry],
        ) -> CountryIdentityCache {
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return CountryIdentityCache::not_attempted("tag_table_unavailable");
            };
            let country_vtable = base + COUNTRY_VTABLE_RVA;
            let mut records_by_tag_id: BTreeMap<u32, usize> = BTreeMap::new();
            for entry in record_bridge.iter().take(MAX_COUNTRY_OBJECT_BRIDGE_ROWS) {
                records_by_tag_id.insert(entry.tag_db_id, entry.object_ptr);
            }
            let mut rows_by_tag: BTreeMap<String, CountryObjectRow> = BTreeMap::new();
            let mut rejected_objects = 0u32;
            let mut duplicate_tags = 0u32;
            for entry in bridge.iter().take(MAX_COUNTRY_OBJECT_BRIDGE_ROWS) {
                let Some(row) = country_object_row_expected(
                    entry.object_ptr,
                    country_vtable,
                    tag_table,
                    entry.tag_db_id,
                    records_by_tag_id.get(&entry.tag_db_id).copied(),
                    false,
                    false,
                ) else {
                    rejected_objects = rejected_objects.saturating_add(1);
                    continue;
                };
                if rows_by_tag.insert(row.tag.clone(), row).is_some() {
                    duplicate_tags = duplicate_tags.saturating_add(1);
                }
            }
            let rows: Vec<CountryObjectRow> = rows_by_tag.into_values().collect();
            let status = if rows.len() >= 40 {
                "control_bridge_verified"
            } else {
                "insufficient_control_bridge"
            };
            CountryIdentityCache {
                status,
                scan_ms: 0,
                country_vtable,
                bridge_rows_supplied: bridge.len().min(u32::MAX as usize) as u32,
                record_rows_supplied: record_bridge.len().min(u32::MAX as usize) as u32,
                rows,
                rejected_objects,
                duplicate_tags,
            }
        }

        fn has_scalar_source(&self) -> bool {
            (self.status == "verified" || self.status == "control_bridge_verified")
                && self.country_vtable != 0
                && !self.rows.is_empty()
        }

        fn current_rows(
            &self,
            country_database_ptr: usize,
            include_scheduled_country_fuel: bool,
            include_scheduled_country_production: bool,
        ) -> Vec<CountryObjectRow> {
            if !self.has_scalar_source() {
                return Vec::new();
            }
            let Some(tag_table) = country_tag_table(country_database_ptr) else {
                return Vec::new();
            };
            let mut rows = Vec::new();
            for row in &self.rows {
                if let Some(fresh) = revalidated_country_object_row(
                    row,
                    self.country_vtable,
                    tag_table,
                    include_scheduled_country_fuel,
                    include_scheduled_country_production,
                ) {
                    rows.push(fresh);
                }
            }
            rows
        }

        fn domains_json_from_rows(
            &self,
            current_rows: &[CountryObjectRow],
            has_state_rows: bool,
            has_province_rows: bool,
            include_scheduled_country_fuel: bool,
            include_scheduled_country_production: bool,
        ) -> String {
            let mut domains = vec!["\"clock\"", "\"country_identity\""];
            if !current_rows.is_empty() {
                domains.push("\"country_scalars\"");
            }
            if current_rows.iter().any(|row| row.misc.is_some()) {
                domains.push("\"country_misc\"");
            }
            if include_scheduled_country_fuel && current_rows.iter().any(|row| row.fuel.is_some()) {
                domains.push("\"fuel_live\"");
            }
            if include_scheduled_country_production
                && current_rows.iter().any(|row| row.production.is_some())
            {
                domains.push("\"production_live\"");
            }
            if has_state_rows {
                domains.push("\"state_ownership\"");
                domains.push("\"state_control\"");
            }
            if has_province_rows {
                domains.push("\"province_control\"");
            }
            format!("[{}]", domains.join(","))
        }

        fn root_json(&self) -> String {
            format!(
                "\"country_object_identity\":{{\"status\":\"{}\",\
                 \"scan_ms\":{},\"bridge_rows_supplied\":{},\
                 \"record_rows_supplied\":{},\
                 \"object_count\":{},\"rejected_objects\":{},\
                 \"duplicate_tags\":{}}}",
                self.status,
                self.scan_ms,
                self.bridge_rows_supplied,
                self.record_rows_supplied,
                self.rows.len(),
                self.rejected_objects,
                self.duplicate_tags
            )
        }

        fn countries_json_from_rows(
            &self,
            country_database_ptr: usize,
            current_rows: &[CountryObjectRow],
            include_scheduled_country_fuel: bool,
            include_scheduled_country_production: bool,
        ) -> String {
            if !current_rows.is_empty() {
                let rows: Vec<String> = current_rows
                    .iter()
                    .map(|row| {
                        row_json(
                            row,
                            include_scheduled_country_fuel,
                            include_scheduled_country_production,
                        )
                    })
                    .collect();
                return format!("{{{}}}", rows.join(","));
            }
            country_rows_from_tag_table_json(country_database_ptr)
        }
    }

    fn write_sinks(file: Option<&mut std::fs::File>, pipe: Option<Handle>, bytes: &[u8]) {
        if let Some(file) = file {
            let _ = file.write_all(bytes);
        }
        if let Some(pipe) = pipe {
            write_handle(pipe, bytes);
        }
    }

    fn module_path() -> Option<PathBuf> {
        let mut buf = [0u16; 32768];
        let len = unsafe {
            let module = DLL_MODULE;
            GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as Dword)
        };
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }

    fn process_path() -> Option<PathBuf> {
        let mut buf = [0u16; 32768];
        let len =
            unsafe { GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), buf.len() as Dword) };
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }

    fn open_pipe(pid: Dword, pipe_token: Option<&str>) -> Option<Handle> {
        let pipe_name = if let Some(token) = pipe_token {
            format!(r"\\.\pipe\battle-plan_collector_{pid}_{token}")
        } else {
            format!(r"\\.\pipe\battle-plan_collector_{pid}")
        };
        let wide: Vec<u16> = OsStr::new(&pipe_name)
            .encode_wide()
            .chain(core::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                0,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if (handle as isize) == -1 {
            None
        } else {
            Some(handle)
        }
    }

    fn write_handle(handle: Handle, bytes: &[u8]) {
        let mut written: Dword = 0;
        unsafe {
            let _ = WriteFile(
                handle,
                bytes.as_ptr() as *const c_void,
                bytes.len().min(Dword::MAX as usize) as Dword,
                &mut written,
                ptr::null_mut(),
            );
        }
    }

    fn scheduled_state_rows_json(
        state: Option<&CollectorState>,
        control: Option<&CollectorControl>,
        include_scheduled_map: bool,
    ) -> Option<String> {
        if !include_scheduled_map {
            return None;
        }
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        let control = control?;
        if !(control.map_control.state_roots_valid && control.map_control.state_roots_well_formed())
        {
            return None;
        }
        if control.map_control.state_roots.len() < MIN_USABLE_STATE_ROWS {
            return None;
        }
        let tag_table = country_tag_table(state.country_database_ptr as usize)?;
        let state_vtable = state.base + STATE_VTABLE_RVA;
        let mut rows = Vec::new();
        for entry in &control.map_control.state_roots {
            if read_u64(entry.state_ptr)? as usize != state_vtable {
                return None;
            }
            let sid = read_u32(entry.state_ptr + STATE_OFF_SID)?;
            if sid != entry.state_id {
                return None;
            }
            let owner = read_u32(entry.state_ptr + STATE_OFF_OWNER).unwrap_or(0);
            let controller = read_u32(entry.state_ptr + STATE_OFF_CTRL).unwrap_or(0);
            let owner_tag = if owner == 0 {
                None
            } else {
                tag_from_table(tag_table, owner)
            };
            let controller_tag = if controller == 0 {
                None
            } else {
                tag_from_table(tag_table, controller)
            };
            rows.push(format!(
                "\"{}\":{{\"owner\":{},\"controller\":{}}}",
                entry.state_id,
                json_opt_str(owner_tag.as_deref()),
                json_opt_str(controller_tag.as_deref())
            ));
        }
        if rows.len() >= MIN_USABLE_STATE_ROWS {
            Some(format!("{{{}}}", rows.join(",")))
        } else {
            None
        }
    }

    fn scheduled_province_rows_json(
        state: Option<&CollectorState>,
        control: Option<&CollectorControl>,
        include_scheduled_map: bool,
    ) -> Option<String> {
        if !include_scheduled_map {
            return None;
        }
        let state = state?;
        if !(state.daily_match && state.long_string_match) {
            return None;
        }
        let control = control?;
        if !(control.map_control.province_bridge_valid
            && control.map_control.province_bridge_well_formed()
            && control.map_control.controller_bridge_valid
            && control.map_control.controller_bridge_well_formed())
        {
            return None;
        }
        let bridge = control.map_control.province_bridge?;
        if control.map_control.controller_bridge.is_empty() {
            return None;
        }
        let tag_table = country_tag_table(state.country_database_ptr as usize)?;
        let mut tags_by_controller_ptr: BTreeMap<usize, String> = BTreeMap::new();
        for entry in &control.map_control.controller_bridge {
            tags_by_controller_ptr.insert(
                entry.controller_ptr,
                tag_from_table(tag_table, entry.tag_db_id)?,
            );
        }

        let mut rows = Vec::new();
        for province_id in 1..bridge.province_count {
            let slot_offset =
                (province_id as usize).checked_mul(bridge.province_stride as usize)?;
            let entry_addr = bridge
                .province_array_ptr
                .checked_add(slot_offset)?
                .checked_add(bridge.controller_ptr_offset as usize)?;
            let controller_ptr = read_u64(entry_addr)? as usize;
            if controller_ptr == 0 {
                continue;
            }
            let controller_tag = tags_by_controller_ptr.get(&controller_ptr)?;
            rows.push(format!(
                "\"{}\":{{\"controller\":\"{}\"}}",
                province_id, controller_tag
            ));
        }

        if rows.len() >= MIN_USABLE_PROVINCE_ROWS {
            Some(format!("{{{}}}", rows.join(",")))
        } else {
            None
        }
    }

    fn root_probe_json(
        state: Option<&CollectorState>,
        scheduled_state_rows: Option<&str>,
        scheduled_province_rows: Option<&str>,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
    ) -> (String, String) {
        let Some(state) = state else {
            return ("\"root_probe\":null".to_string(), "[]".to_string());
        };
        let base = state.base;
        if state.base == 0 {
            return (
                "\"root_probe\":{\"status\":\"profile_mismatch\",\"reason\":\"no_image_base\"}"
                    .to_string(),
                "[]".to_string(),
            );
        }

        if !(state.daily_match && state.long_string_match) {
            return (
                format!(
                    "\"root_probe\":{{\"status\":\"profile_mismatch\",\
                     \"image_base\":\"0x{base:X}\",\
                     \"daily_update_sentinel_match\":{},\
                     \"long_string_sentinel_match\":{}}}",
                    state.daily_match, state.long_string_match
                ),
                "[]".to_string(),
            );
        }

        let date_ptr = state.date_ptr;
        let country_database_ptr = state.country_database_ptr;
        let country_probe = country_database_probe_json(country_database_ptr as usize);
        let current_rows = state.country_identity.current_rows(
            country_database_ptr as usize,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
        );
        let country_rows = state.country_identity.countries_json_from_rows(
            country_database_ptr as usize,
            &current_rows,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
        );
        let identity_probe = state.country_identity.root_json();
        let domains = state.country_identity.domains_json_from_rows(
            &current_rows,
            scheduled_state_rows.is_some(),
            scheduled_province_rows.is_some(),
            include_scheduled_country_fuel,
            include_scheduled_country_production,
        );
        let clock_probe = clock_probe_json(date_ptr as usize);
        let state_rows = scheduled_state_rows
            .map(|rows| format!(",\"states\":{rows}"))
            .unwrap_or_default();
        let province_rows = scheduled_province_rows
            .map(|rows| format!(",\"provinces\":{rows}"))
            .unwrap_or_default();
        (
            format!(
                "\"root_probe\":{{\"status\":\"verified\",\
                 \"profile\":\"{}\",\
                 \"image_base\":\"0x{base:X}\",\
                 \"daily_update\":\"0x{:X}\",\
                 \"get_long_string\":\"0x{:X}\",\
                 \"daily_update_sentinel_match\":true,\
                 \"long_string_sentinel_match\":true,\
                 \"date_root_ptr\":\"0x{date_ptr:X}\",\
                 \"country_database_root_ptr\":\"0x{country_database_ptr:X}\",\
                 {clock_probe},\
                 {country_probe},\
                 {identity_probe},\
                 \"domains\":{domains}}},\
                 \"countries\":{country_rows}{state_rows}{province_rows}",
                super::COLLECTOR_PROFILE_ID,
                state.daily_addr,
                state.long_string_addr
            ),
            domains,
        )
    }

    fn is_hoi4_process() -> bool {
        process_path()
            .and_then(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
            })
            .as_deref()
            == Some("hoi4.exe")
    }

    fn sentinel_matches(addr: usize, sentinel: &[u8]) -> bool {
        read_bytes(addr, sentinel.len())
            .map(|bytes| bytes == sentinel)
            .unwrap_or(false)
    }

    fn read_u64(addr: usize) -> Option<u64> {
        let bytes = read_bytes(addr, 8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Some(u64::from_le_bytes(arr))
    }

    fn read_u32(addr: usize) -> Option<u32> {
        let bytes = read_bytes(addr, 4)?;
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&bytes);
        Some(u32::from_le_bytes(arr))
    }

    fn read_u16(addr: usize) -> Option<u16> {
        let bytes = read_bytes(addr, 2)?;
        let mut arr = [0u8; 2];
        arr.copy_from_slice(&bytes);
        Some(u16::from_le_bytes(arr))
    }

    fn read_u8(addr: usize) -> Option<u8> {
        read_bytes(addr, 1).and_then(|bytes| bytes.first().copied())
    }

    fn country_database_probe_json(country_database_ptr: usize) -> String {
        if country_database_ptr == 0 {
            return "\"country_tag_count\":null,\"country_active_count\":null,\
                    \"tag_sample\":[]"
                .to_string();
        }
        let tag_table = read_u64(country_database_ptr).unwrap_or(0) as usize;
        let tag_count = read_u32(country_database_ptr + 8).unwrap_or(0);
        let active_count = read_u32(country_database_ptr + 12).unwrap_or(0);
        let sample = tag_sample_json(tag_table, tag_count.min(12));
        format!(
            "\"country_tag_count\":{tag_count},\
             \"country_active_count\":{active_count},\
             \"tag_sample\":{sample}"
        )
    }

    fn country_rows_from_tag_table_json(country_database_ptr: usize) -> String {
        if country_database_ptr == 0 {
            return "{}".to_string();
        }
        let tag_table = read_u64(country_database_ptr).unwrap_or(0) as usize;
        let tag_count = read_u32(country_database_ptr + 8).unwrap_or(0);
        let active_count = read_u32(country_database_ptr + 12).unwrap_or(0);
        if tag_table == 0 || tag_count == 0 || active_count == 0 {
            return "{}".to_string();
        }
        let row_count = tag_count.min(active_count).min(1024);
        let mut rows = Vec::new();
        for i in 0..row_count as usize {
            let Some(tag) = read_small_inline_string(tag_table + i * 32) else {
                continue;
            };
            if is_country_tag(&tag) {
                rows.push(format!(
                    "\"{tag}\":{{\"present\":true,\"identity\":{{\"tag_db_id\":{i}}}}}"
                ));
            }
        }
        format!("{{{}}}", rows.join(","))
    }

    fn country_tag_table(country_database_ptr: usize) -> Option<usize> {
        if country_database_ptr == 0 {
            return None;
        }
        let tag_table = read_u64(country_database_ptr)? as usize;
        let tag_count = read_u32(country_database_ptr + 8)?;
        if tag_table == 0 || tag_count < 40 || tag_count > 1024 {
            return None;
        }
        let prefix = ["---", "GER", "ENG", "SOV"];
        for (i, want) in prefix.iter().enumerate() {
            let got = read_small_inline_string(tag_table + i * 32)?;
            if got != *want {
                return None;
            }
        }
        Some(tag_table)
    }

    fn tag_from_table(tag_table: usize, tag_db_id: u32) -> Option<String> {
        if tag_db_id > 1024 {
            return None;
        }
        read_small_inline_string(tag_table + tag_db_id as usize * 32)
            .filter(|tag| is_country_tag(tag))
    }

    fn country_object_row(
        object_ptr: usize,
        country_vtable: usize,
        tag_table: usize,
        record_ptr: Option<usize>,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
    ) -> Option<CountryObjectRow> {
        if read_u64(object_ptr)? as usize != country_vtable {
            return None;
        }
        let name = read_msvc_string(object_ptr + OBJ_NAME, 96)?;
        if read_msvc_string(object_ptr + OBJ_NAME, 96)? != name {
            return None;
        }
        let tag_db_id = read_u32(object_ptr + OBJ_TAG_DB_ID)?;
        let tag = tag_from_table(tag_table, tag_db_id)?;
        let original_tag_id = read_u16(object_ptr + OBJ_ORIGINAL_TAG);
        let original_tag = original_tag_id.and_then(|id| {
            if id == 0 {
                None
            } else {
                tag_from_table(tag_table, id as u32)
            }
        });
        Some(CountryObjectRow {
            tag,
            tag_db_id,
            name,
            object_ptr,
            record_ptr,
            original_capital: read_i32(object_ptr + OBJ_ORIGINAL_CAPITAL).and_then(|v| {
                if v > 0 {
                    Some(v)
                } else {
                    None
                }
            }),
            original_tag,
            templates_locked: read_u8(object_ptr + OBJ_TEMPLATES_LOCKED).map(|v| v == 1),
            reserved_dynamic_country: read_u8(object_ptr + OBJ_RESERVED_DYNAMIC_COUNTRY)
                .map(|v| v == 1),
            stability: read_fixed_0_1(object_ptr + OBJ_STABILITY),
            war_support: read_fixed_0_1(object_ptr + OBJ_WAR_SUPPORT),
            political_power: record_ptr
                .and_then(|record_ptr| country_record_political_power(record_ptr, object_ptr)),
            command_power: read_fixed_nonnegative(object_ptr + OBJ_COMMAND_POWER, 1000.0),
            num_ships: read_i32(object_ptr + OBJ_NUM_SHIPS).and_then(|v| {
                if (0..=100_000).contains(&v) {
                    Some(v)
                } else {
                    None
                }
            }),
            manpower_pool: read_u32(object_ptr + OBJ_MANPOWER_POOL).and_then(|v| {
                if v <= 2_000_000_000 {
                    Some(v)
                } else {
                    None
                }
            }),
            convoys: read_fixed_nonnegative(object_ptr + OBJ_CONVOYS, 1_000_000.0),
            misc: read_country_misc(object_ptr),
            fuel: if include_scheduled_country_fuel {
                read_country_fuel(object_ptr)
            } else {
                None
            },
            production: if include_scheduled_country_production {
                read_country_production(object_ptr, country_vtable)
            } else {
                None
            },
        })
    }

    fn country_object_row_expected(
        object_ptr: usize,
        country_vtable: usize,
        tag_table: usize,
        expected_tag_db_id: u32,
        record_ptr: Option<usize>,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
    ) -> Option<CountryObjectRow> {
        let row = country_object_row(
            object_ptr,
            country_vtable,
            tag_table,
            record_ptr,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
        )?;
        if row.tag_db_id == expected_tag_db_id {
            Some(row)
        } else {
            None
        }
    }

    fn revalidated_country_object_row(
        expected: &CountryObjectRow,
        country_vtable: usize,
        tag_table: usize,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
    ) -> Option<CountryObjectRow> {
        let row = country_object_row_expected(
            expected.object_ptr,
            country_vtable,
            tag_table,
            expected.tag_db_id,
            expected.record_ptr,
            include_scheduled_country_fuel,
            include_scheduled_country_production,
        )?;
        if row.tag == expected.tag && row.name == expected.name {
            Some(row)
        } else {
            None
        }
    }

    fn read_country_misc(object_ptr: usize) -> Option<CountryMiscRow> {
        let row = CountryMiscRow {
            convoys_destroyed: read_u32_stable(object_ptr + OBJ_CONVOYS_DESTROYED)
                .filter(|v| *v <= MAX_COUNTRY_CONVOYS_DESTROYED),
            accidents_score: read_i32_stable(object_ptr + OBJ_ACCIDENTS_SCORE)
                .map(|v| v as f64 / FP_SCALE)
                .filter(|v| v.is_finite() && (0.0..=MAX_COUNTRY_ACCIDENTS_SCORE).contains(v)),
            ace_count: read_u32_stable(object_ptr + OBJ_ACE_COUNT)
                .filter(|v| *v <= MAX_COUNTRY_ACE_COUNT),
            cosmetic_tag: read_stable_msvc_token_nullable(
                object_ptr + OBJ_COSMETIC_TAG,
                MAX_COUNTRY_COSMETIC_TAG_LEN,
            ),
        };
        if row.has_any() {
            Some(row)
        } else {
            None
        }
    }

    fn country_record_political_power(record_ptr: usize, object_ptr: usize) -> Option<f64> {
        if read_u64(record_ptr + COUNTRY_RECORD_OBJECT_PTR)? as usize != object_ptr {
            return None;
        }
        let value = read_i32(record_ptr + COUNTRY_RECORD_POLITICAL_POWER)? as f64 / FP_SCALE;
        if (-1_000_000.0..=1_000_000.0).contains(&value) {
            Some(value)
        } else {
            None
        }
    }

    fn read_country_fuel(object_ptr: usize) -> Option<CountryFuelRow> {
        let fuel_status = read_u64(object_ptr + OBJ_FUEL_STATUS)? as usize;
        if !is_map_control_heap_pointer(fuel_status) {
            return None;
        }
        let current = read_i64(fuel_status + FUEL_STATUS_CURRENT)? as f64 / FUEL_SCALE;
        let max = read_i64(fuel_status + FUEL_STATUS_MAX)? as f64 / FUEL_SCALE;
        let gain = read_i64(fuel_status + FUEL_STATUS_GAIN)? as f64 / FUEL_SCALE;
        let cost = read_i64(fuel_status + FUEL_STATUS_COST)? as f64 / FUEL_SCALE;
        if valid_nonnegative_fuel_measure(current)
            && valid_nonnegative_fuel_measure(max)
            && valid_signed_fuel_measure(gain)
            && valid_signed_fuel_measure(cost)
        {
            Some(CountryFuelRow {
                current,
                max,
                gain,
                cost,
            })
        } else {
            None
        }
    }

    fn valid_nonnegative_fuel_measure(value: f64) -> bool {
        value.is_finite() && (0.0..=MAX_FUEL_MEASURE).contains(&value)
    }

    fn valid_signed_fuel_measure(value: f64) -> bool {
        value.is_finite() && value.abs() <= MAX_FUEL_MEASURE
    }

    fn read_country_production(
        object_ptr: usize,
        country_vtable: usize,
    ) -> Option<CountryProductionRow> {
        let base = country_vtable.checked_sub(COUNTRY_VTABLE_RVA)?;
        let variant_vtable = base.checked_add(VARIANT_VTABLE_RVA)?;
        let manager = read_u64(object_ptr + OBJ_PROD_MANAGER)? as usize;
        if !is_map_control_heap_pointer(manager) {
            return None;
        }
        let stockpile = read_production_stockpile(manager, variant_vtable);
        let lines = read_production_lines(manager);
        if stockpile.is_none() && lines.is_none() {
            return None;
        }
        let (military_lines, naval_lines) = match lines {
            Some((military_lines, naval_lines)) => (Some(military_lines), Some(naval_lines)),
            None => (None, None),
        };
        Some(CountryProductionRow {
            stockpile,
            military_lines,
            naval_lines,
        })
    }

    fn read_production_stockpile(
        manager: usize,
        variant_vtable: usize,
    ) -> Option<Vec<CountryStockpileRow>> {
        let begin = read_u64(manager + STOCK_BEGIN)? as usize;
        let cap = read_u32(manager + STOCK_CAP)?;
        let size = read_u32(manager + STOCK_SIZE)?;
        if size > cap || cap > MAX_PRODUCTION_VECTOR_ENTRIES {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        if !is_map_control_heap_pointer(begin) {
            return None;
        }
        let byte_len = (size as usize).checked_mul(STOCK_STRIDE)?;
        let raw = read_bytes(begin, byte_len)?;
        let mut rows_by_variant: BTreeMap<u32, f64> = BTreeMap::new();
        for k in (0..byte_len).step_by(STOCK_STRIDE) {
            let def_ptr = read_u64_from_slice(&raw, k + STOCK_ENTRY_DEF)? as usize;
            let amount_raw = read_i64_from_slice(&raw, k + STOCK_ENTRY_AMOUNT)?;
            if !is_map_control_heap_pointer(def_ptr) {
                continue;
            }
            let def_vtable = read_u64(def_ptr)? as usize;
            let def_type = read_u32(def_ptr + VARIANT_DEF_TYPE)?;
            if def_vtable != variant_vtable || def_type != VARIANT_TYPE {
                return None;
            }
            let variant_id = read_u32(def_ptr + VARIANT_DEF_ID)?;
            let amount = amount_raw as f64 / FP_SCALE;
            if !amount.is_finite() || amount.abs() > MAX_PRODUCTION_STOCKPILE_AMOUNT {
                return None;
            }
            if rows_by_variant.insert(variant_id, amount).is_some() {
                return None;
            }
        }
        Some(
            rows_by_variant
                .into_iter()
                .map(|(variant_id, amount)| CountryStockpileRow { variant_id, amount })
                .collect(),
        )
    }

    fn read_production_lines(
        manager: usize,
    ) -> Option<(Vec<CountryProductionLineRow>, Vec<CountryProductionLineRow>)> {
        let begin = read_u64(manager + LINE_BEGIN)? as usize;
        let cap = read_u32(manager + LINE_CAP)?;
        let size = read_u32(manager + LINE_SIZE)?;
        if size > cap || cap > MAX_PRODUCTION_VECTOR_ENTRIES {
            return None;
        }
        if size == 0 {
            return Some((Vec::new(), Vec::new()));
        }
        if !is_map_control_heap_pointer(begin) {
            return None;
        }
        let byte_len = (size as usize).checked_mul(LINE_STRIDE)?;
        let raw = read_bytes(begin, byte_len)?;
        let mut military = Vec::new();
        let mut naval = Vec::new();
        for k in (0..byte_len).step_by(LINE_STRIDE) {
            let line_ptr = read_u64_from_slice(&raw, k)? as usize;
            if !is_map_control_heap_pointer(line_ptr) {
                continue;
            }
            let line_type = read_u32(line_ptr + LINE_TYPE)?;
            if line_type != LINE_TYPE_MILITARY && line_type != LINE_TYPE_NAVAL {
                continue;
            }
            let active_factories = read_i32(line_ptr + LINE_ACTIVE_FACTORIES)?;
            if !(0..=100_000).contains(&active_factories) {
                continue;
            }
            let Some(cost) = read_production_fixed(line_ptr + LINE_COST) else {
                continue;
            };
            let Some(speed) = read_production_fixed(line_ptr + LINE_SPEED) else {
                continue;
            };
            let Some(produced) = read_production_fixed(line_ptr + LINE_PRODUCED) else {
                continue;
            };
            let line = CountryProductionLineRow {
                id: read_i32(line_ptr + LINE_ID)?,
                active_factories,
                cost,
                speed,
                produced,
                amount: read_i32(line_ptr + LINE_AMOUNT)?,
            };
            if line_type == LINE_TYPE_MILITARY {
                military.push(line);
            } else {
                naval.push(line);
            }
        }
        Some((military, naval))
    }

    fn read_production_fixed(addr: usize) -> Option<f64> {
        read_i32(addr)
            .map(|v| v as f64 / FP_SCALE)
            .filter(|v| v.is_finite() && v.abs() <= MAX_PRODUCTION_LINE_FIXED)
    }

    fn row_json(
        row: &CountryObjectRow,
        include_scheduled_country_fuel: bool,
        include_scheduled_country_production: bool,
    ) -> String {
        let original_capital = json_opt_i32(row.original_capital);
        let original_tag = json_opt_str(row.original_tag.as_deref());
        let templates_locked = json_opt_bool(row.templates_locked);
        let reserved_dynamic_country = json_opt_bool(row.reserved_dynamic_country);
        let stability = json_opt_f64(row.stability);
        let war_support = json_opt_f64(row.war_support);
        let political_power = json_opt_f64(row.political_power);
        let command_power = json_opt_f64(row.command_power);
        let num_ships = json_opt_i32(row.num_ships);
        let manpower_pool = json_opt_u32(row.manpower_pool);
        let convoys = json_opt_f64(row.convoys);
        let misc = row.misc.as_ref().map(country_misc_json).unwrap_or_default();
        let fuel = if include_scheduled_country_fuel {
            row.fuel
                .map(|fuel| {
                    format!(
                        ",\"fuel\":{{\"current\":{},\"max\":{},\"gain\":{},\"cost\":{}}}",
                        json_opt_f64(Some(fuel.current)),
                        json_opt_f64(Some(fuel.max)),
                        json_opt_f64(Some(fuel.gain)),
                        json_opt_f64(Some(fuel.cost))
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let production = if include_scheduled_country_production {
            row.production
                .as_ref()
                .map(country_production_json)
                .unwrap_or_default()
        } else {
            String::new()
        };
        format!(
            "\"{}\":{{\"present\":true,\
             \"collector_object\":\"0x{:X}\",\
             \"identity\":{{\"tag_db_id\":{},\
             \"original_capital\":{original_capital},\
             \"original_tag\":{original_tag},\
             \"reserved_dynamic_country\":{reserved_dynamic_country},\
             \"templates_locked\":{templates_locked}}},\
             \"scalars\":{{\"stability\":{stability},\
             \"war_support\":{war_support},\
             \"political_power\":{political_power},\
             \"command_power\":{command_power},\
             \"num_ships\":{num_ships},\
             \"manpower_pool\":{manpower_pool},\
             \"convoys\":{convoys}}}{misc}{fuel}{production}}}",
            row.tag, row.object_ptr, row.tag_db_id
        )
    }

    fn country_misc_json(misc: &CountryMiscRow) -> String {
        let mut fields = Vec::new();
        if let Some(value) = misc.convoys_destroyed {
            fields.push(format!("\"convoys_destroyed\":{value}"));
        }
        if let Some(value) = misc.accidents_score {
            fields.push(format!("\"accidents_score\":{}", json_opt_f64(Some(value))));
        }
        if let Some(value) = misc.ace_count {
            fields.push(format!("\"ace_count\":{value}"));
        }
        if let Some(value) = &misc.cosmetic_tag {
            fields.push(format!(
                "\"cosmetic_tag\":{}",
                json_opt_str(value.as_deref())
            ));
        }
        if fields.is_empty() {
            String::new()
        } else {
            format!(",\"misc\":{{{}}}", fields.join(","))
        }
    }

    fn country_production_json(production: &CountryProductionRow) -> String {
        let mut fields = Vec::new();
        if let Some(stockpile) = production.stockpile.as_deref() {
            let stockpile_total: f64 = stockpile.iter().map(|row| row.amount).sum();
            fields.push(format!(
                "\"stockpile\":{}",
                production_stockpile_json(stockpile)
            ));
            fields.push(format!(
                "\"stockpile_total\":{}",
                json_opt_f64(Some(stockpile_total))
            ));
        }
        if let Some(military_lines) = production.military_lines.as_deref() {
            let military_factories: i32 = military_lines
                .iter()
                .map(|line| line.active_factories)
                .sum();
            fields.push(format!(
                "\"military_lines\":{}",
                production_lines_json(military_lines)
            ));
            fields.push(format!("\"military_factories\":{military_factories}"));
        }
        if let Some(naval_lines) = production.naval_lines.as_deref() {
            let naval_factories: i32 = naval_lines.iter().map(|line| line.active_factories).sum();
            fields.push(format!(
                "\"naval_lines\":{}",
                production_lines_json(naval_lines)
            ));
            fields.push(format!("\"naval_factories\":{naval_factories}"));
        }
        format!(",\"production\":{{{}}}", fields.join(","))
    }

    fn production_stockpile_json(rows: &[CountryStockpileRow]) -> String {
        let parts: Vec<String> = rows
            .iter()
            .map(|row| format!("\"{}\":{}", row.variant_id, json_opt_f64(Some(row.amount))))
            .collect();
        format!("{{{}}}", parts.join(","))
    }

    fn production_lines_json(rows: &[CountryProductionLineRow]) -> String {
        let parts: Vec<String> = rows
            .iter()
            .map(|line| {
                format!(
                    "{{\"id\":{},\"active_factories\":{},\
                     \"cost\":{},\"speed\":{},\"produced\":{},\"amount\":{}}}",
                    line.id,
                    line.active_factories,
                    json_opt_f64(Some(line.cost)),
                    json_opt_f64(Some(line.speed)),
                    json_opt_f64(Some(line.produced)),
                    line.amount
                )
            })
            .collect();
        format!("[{}]", parts.join(","))
    }

    fn clock_probe_json(gamestate_ptr: usize) -> String {
        if gamestate_ptr == 0 {
            return "\"current_game_date\":null,\"current_game_date_raw\":null,\
                    \"start_date\":null,\"start_date_raw\":null"
                .to_string();
        }
        let current_raw = read_i32(gamestate_ptr + GS_CURRENT_DATE_OFFSET);
        let start_raw = read_i32(gamestate_ptr + GS_START_DATE_OFFSET);
        let current = current_raw.and_then(decode_datehour);
        let start = start_raw.and_then(decode_datehour);
        format!(
            "\"current_game_date\":{},\
             \"current_game_date_raw\":{},\
             \"start_date\":{},\
             \"start_date_raw\":{}",
            json_opt_str(current.as_deref()),
            json_opt_i32(current_raw),
            json_opt_str(start.as_deref()),
            json_opt_i32(start_raw)
        )
    }

    fn read_i32(addr: usize) -> Option<i32> {
        read_u32(addr).map(|v| v as i32)
    }

    fn read_i32_stable(addr: usize) -> Option<i32> {
        let first = read_i32(addr)?;
        let second = read_i32(addr)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_u32_stable(addr: usize) -> Option<u32> {
        let first = read_u32(addr)?;
        let second = read_u32(addr)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_i64(addr: usize) -> Option<i64> {
        let bytes = read_bytes(addr, 8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        Some(i64::from_le_bytes(arr))
    }

    fn read_u64_from_slice(bytes: &[u8], offset: usize) -> Option<u64> {
        let end = offset.checked_add(8)?;
        let slice = bytes.get(offset..end)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(slice);
        Some(u64::from_le_bytes(arr))
    }

    fn read_i64_from_slice(bytes: &[u8], offset: usize) -> Option<i64> {
        let end = offset.checked_add(8)?;
        let slice = bytes.get(offset..end)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(slice);
        Some(i64::from_le_bytes(arr))
    }

    fn read_fixed_0_1(addr: usize) -> Option<f64> {
        read_i32(addr)
            .map(|v| v as f64 / FP_SCALE)
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }

    fn read_fixed_nonnegative(addr: usize, max_value: f64) -> Option<f64> {
        read_i32(addr)
            .map(|v| v as f64 / FP_SCALE)
            .filter(|v| v.is_finite() && *v >= 0.0 && *v <= max_value)
    }

    fn decode_datehour(stored: i32) -> Option<String> {
        let min = (1850 * 365 * 24) + DATE_EPOCH_CONST;
        let max = (2200 * 365 * 24) + DATE_EPOCH_CONST;
        if stored < min || stored > max {
            return None;
        }
        let hrs = stored - DATE_EPOCH_CONST;
        let hour = hrs % 24;
        let rem_days = hrs / 24;
        let year = rem_days / 365;
        let mut doy = rem_days - year * 365;
        let month_lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month = 1;
        for len in month_lengths {
            if doy < len {
                break;
            }
            doy -= len;
            month += 1;
        }
        Some(format!("{year}.{month}.{}.{hour}", doy + 1))
    }

    fn json_opt_str(value: Option<&str>) -> String {
        match value {
            Some(v) => format!("\"{v}\""),
            None => "null".to_string(),
        }
    }

    fn json_opt_i32(value: Option<i32>) -> String {
        match value {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }

    fn json_opt_u32(value: Option<u32>) -> String {
        match value {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }

    fn json_opt_bool(value: Option<bool>) -> String {
        match value {
            Some(v) => v.to_string(),
            None => "null".to_string(),
        }
    }

    fn json_opt_f64(value: Option<f64>) -> String {
        match value {
            Some(v) if v.is_finite() => v.to_string(),
            _ => "null".to_string(),
        }
    }

    fn tag_sample_json(tag_table: usize, count: u32) -> String {
        if tag_table == 0 || count == 0 {
            return "[]".to_string();
        }
        let mut tags = Vec::new();
        for i in 0..count as usize {
            if let Some(tag) = read_small_inline_string(tag_table + i * 32) {
                tags.push(format!("\"{tag}\""));
            }
        }
        format!("[{}]", tags.join(","))
    }

    fn read_small_inline_string(addr: usize) -> Option<String> {
        let bytes = read_bytes(addr, 32)?;
        let len = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        if len == 0 || len > 7 {
            return None;
        }
        let raw = &bytes[..len];
        if !raw
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'-')
        {
            return None;
        }
        String::from_utf8(raw.to_vec()).ok()
    }

    fn read_msvc_string(addr: usize, max_len: usize) -> Option<String> {
        let size = read_u64(addr + 0x10)? as usize;
        let cap = read_u64(addr + 0x18)? as usize;
        if size == 0 || size > max_len || cap < size || cap > (1 << 20) {
            return None;
        }
        let chars_addr = if cap < 16 {
            addr
        } else {
            read_u64(addr)? as usize
        };
        let bytes = read_bytes(chars_addr, size + 1)?;
        if bytes.get(size).copied() != Some(0) {
            return None;
        }
        let raw = &bytes[..size];
        if !raw.iter().all(|b| (32..127).contains(b)) {
            return None;
        }
        String::from_utf8(raw.to_vec()).ok()
    }

    fn read_stable_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {
        let first = read_msvc_token_nullable(addr, max_len)?;
        let second = read_msvc_token_nullable(addr, max_len)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn read_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {
        let size = read_u64(addr + 0x10)? as usize;
        let cap = read_u64(addr + 0x18)? as usize;
        if size > max_len || cap < size || cap > (1 << 20) {
            return None;
        }
        if size == 0 {
            return if cap < 16 { Some(None) } else { None };
        }
        let chars_addr = if cap < 16 {
            addr
        } else {
            read_u64(addr)? as usize
        };
        let bytes = read_bytes(chars_addr, size + 1)?;
        if bytes.get(size).copied() != Some(0) {
            return None;
        }
        let raw = &bytes[..size];
        if !raw.iter().all(|b| is_cosmetic_token_byte(*b)) {
            return None;
        }
        Some(Some(String::from_utf8(raw.to_vec()).ok()?))
    }

    fn is_cosmetic_token_byte(value: u8) -> bool {
        value.is_ascii_alphanumeric() || value == b'_' || value == b'-'
    }

    fn is_country_tag(tag: &str) -> bool {
        tag.len() == 3
            && tag
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && tag.bytes().any(|b| b.is_ascii_alphabetic())
    }

    fn read_bytes(addr: usize, len: usize) -> Option<Vec<u8>> {
        if len == 0 || !is_readable_range(addr, len) {
            return None;
        }
        let mut out = vec![0u8; len];
        unsafe {
            ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), len);
        }
        Some(out)
    }

    fn scan_qword_hits(needle: usize, lo: usize, hi: usize) -> Vec<usize> {
        let needle_bytes = (needle as u64).to_le_bytes();
        let mut hits = Vec::new();
        let mut addr = lo;
        const CHUNK: usize = 16 << 20;
        const OVERLAP: usize = 16;
        while addr < hi {
            let Some((region_start, region_size)) = readable_region(addr) else {
                addr = addr.saturating_add(0x1000);
                continue;
            };
            let region_end = region_start.saturating_add(region_size).min(hi);
            let mut pos = region_start.max(addr);
            while pos < region_end {
                let want = CHUNK.min(region_end - pos);
                let Some(blob) = read_bytes(pos, want) else {
                    pos = pos.saturating_add(want.max(0x1000));
                    continue;
                };
                let mut i = 0usize;
                while i + 8 <= blob.len() {
                    if blob[i..i + 8] == needle_bytes[..] && (pos + i) % 8 == 0 {
                        hits.push(pos + i);
                    }
                    i += 1;
                }
                if pos + blob.len() >= region_end || blob.len() < want {
                    pos = pos.saturating_add(blob.len().max(0x1000));
                } else {
                    pos = pos.saturating_add(blob.len().saturating_sub(OVERLAP));
                }
            }
            addr = region_end.saturating_add(0x1000);
        }
        hits
    }

    fn readable_region(addr: usize) -> Option<(usize, usize)> {
        let mut mbi = MemoryBasicInformation {
            base_address: ptr::null_mut(),
            allocation_base: ptr::null_mut(),
            allocation_protect: 0,
            partition_id: 0,
            region_size: 0,
            state: 0,
            protect: 0,
            type_: 0,
        };
        let ok = unsafe {
            VirtualQuery(
                addr as *const c_void,
                &mut mbi,
                core::mem::size_of::<MemoryBasicInformation>(),
            )
        };
        if ok == 0 || mbi.state != MEM_COMMIT {
            return None;
        }
        if (mbi.protect & (PAGE_NOACCESS | PAGE_GUARD)) != 0 {
            return None;
        }
        Some((mbi.base_address as usize, mbi.region_size))
    }

    fn is_readable_range(addr: usize, len: usize) -> bool {
        let Some((region_start, region_size)) = readable_region(addr) else {
            return false;
        };
        let region_end = region_start.saturating_add(region_size);
        addr >= region_start && addr.saturating_add(len) <= region_end
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        fn raw_test_control() -> CollectorControl {
            CollectorControl {
                duration_ms: 8000,
                sample_ms: 10,
                country_object_scan: false,
                country_object_bridge: Vec::new(),
                country_object_bridge_valid: true,
                country_record_bridge: Vec::new(),
                country_record_bridge_valid: true,
                division_body_bridge: Vec::new(),
                division_body_bridge_valid: true,
                map_control: MapControlContract::empty(),
                scheduled_snapshot_step_hours: 1,
                pipe_token: Some("abc".to_string()),
                session_id: Some("run_1".to_string()),
                sidecar_log: false,
            }
        }

        fn stationary_division_sample() -> DivisionMovementSample {
            let raw_organisation = 5_000_000;
            let raw_strength = 100_000;
            DivisionMovementSample {
                div_id: 7,
                moving: false,
                raw_movement_progress: None,
                progress_bucket: None,
                location: 10,
                move_target_province: None,
                movement_source_province: None,
                raw_organisation,
                organisation_bucket: division_delta_bucket(raw_organisation),
                raw_strength,
                strength_bucket: division_delta_bucket(raw_strength),
                combat_ref_count: 0,
                in_combat: false,
            }
        }

        #[test]
        fn collector_control_accepts_sanitized_session_id() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"pipe_token":"abc-123","session_id":"run_1-abc","sidecar_log":false}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert_eq!(parsed.duration_ms, 8000);
            assert_eq!(parsed.sample_ms, 10);
            assert_eq!(parsed.pipe_token.as_deref(), Some("abc-123"));
            assert_eq!(parsed.session_id.as_deref(), Some("run_1-abc"));
            assert!(parsed.country_object_bridge.is_empty());
            assert!(parsed.country_object_bridge_valid);
            assert!(parsed.country_record_bridge.is_empty());
            assert!(parsed.country_record_bridge_valid);
            assert!(parsed.division_body_bridge.is_empty());
            assert!(parsed.division_body_bridge_valid);
            assert!(parsed.map_control.state_roots.is_empty());
            assert!(parsed.map_control.state_roots_valid);
            assert!(parsed.map_control.province_bridge.is_none());
            assert!(parsed.map_control.province_bridge_valid);
            assert!(parsed.map_control.controller_bridge.is_empty());
            assert!(parsed.map_control.controller_bridge_valid);
            assert!(!parsed.sidecar_log);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_country_object_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_scan":false,"country_object_bridge":"1:0x1000;2:8192"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(!parsed.country_object_scan);
            assert!(parsed.country_object_bridge_valid);
            assert_eq!(parsed.country_object_bridge.len(), 2);
            assert_eq!(parsed.country_object_bridge[0].tag_db_id, 1);
            assert_eq!(parsed.country_object_bridge[0].object_ptr, 0x1000);
            assert_eq!(parsed.country_object_bridge[1].tag_db_id, 2);
            assert_eq!(parsed.country_object_bridge[1].object_ptr, 8192);
            assert!(parsed.country_record_bridge.is_empty());
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_country_record_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_record_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1000","country_record_bridge":"1:0x3000"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_record_bridge_valid);
            assert_eq!(parsed.country_record_bridge.len(), 1);
            assert_eq!(parsed.country_record_bridge[0].tag_db_id, 1);
            assert_eq!(parsed.country_record_bridge[0].object_ptr, 0x3000);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_division_body_bridge() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_division_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"division_body_bridge":"7:0x{:x};8:{}"}}"#,
                    HEAP_LO,
                    HEAP_LO + 0x100
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.division_body_bridge_valid);
            assert_eq!(parsed.division_body_bridge.len(), 2);
            assert_eq!(parsed.division_body_bridge[0].div_id, 7);
            assert_eq!(parsed.division_body_bridge[0].body_ptr, HEAP_LO);
            assert_eq!(parsed.division_body_bridge[1].div_id, 8);
            assert_eq!(parsed.division_body_bridge[1].body_ptr, HEAP_LO + 0x100);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_accepts_map_control_contract() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_map_control_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            let state_roots = format!("1:0x{:x};1081:0x{:x}", HEAP_LO, HEAP_LO + 0x100);
            let province_bridge = format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=0",
                HEAP_LO + 0x1000
            );
            let controller_bridge =
                format!("1:0x{:x};2:0x{:x}", HEAP_LO + 0x2000, HEAP_LO + 0x3000);
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"map_control_state_roots":"{}","map_control_province_bridge":"{}","map_control_controller_bridge":"{}"}}"#,
                    state_roots, province_bridge, controller_bridge
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.map_control.state_roots_valid);
            assert_eq!(parsed.map_control.state_roots.len(), 2);
            assert_eq!(parsed.map_control.state_roots[0].state_id, 1);
            assert_eq!(parsed.map_control.state_roots[0].state_ptr, HEAP_LO);
            assert_eq!(parsed.map_control.state_roots[1].state_id, 1081);
            assert_eq!(parsed.map_control.state_roots[1].state_ptr, HEAP_LO + 0x100);
            assert!(parsed.map_control.province_bridge_valid);
            let bridge = parsed.map_control.province_bridge.unwrap();
            assert_eq!(bridge.province_array_ptr, HEAP_LO + 0x1000);
            assert_eq!(bridge.province_count, 13232);
            assert_eq!(bridge.province_stride, 8);
            assert_eq!(bridge.controller_ptr_offset, 0);
            assert!(parsed.map_control.controller_bridge_valid);
            assert_eq!(parsed.map_control.controller_bridge.len(), 2);
            assert_eq!(parsed.map_control.controller_bridge[0].tag_db_id, 1);
            assert_eq!(
                parsed.map_control.controller_bridge[0].controller_ptr,
                HEAP_LO + 0x2000
            );
            assert_eq!(parsed.map_control.controller_bridge[1].tag_db_id, 2);
            assert_eq!(
                parsed.map_control.controller_bridge[1].controller_ptr,
                HEAP_LO + 0x3000
            );
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_country_object_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_object_bridge.is_empty());
            assert!(!parsed.country_object_bridge_valid);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_division_body_bridge_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_division_bridge_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1000","division_body_bridge":"7:0x1001"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_object_bridge_valid);
            assert!(!parsed.country_object_bridge.is_empty());
            assert!(parsed.division_body_bridge.is_empty());
            assert!(!parsed.division_body_bridge_valid);
            let frame = build_frame_line(
                1234,
                None,
                Some(&parsed),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );
            assert!(frame.contains("\"division_body_bridge_rows\":0"));
            assert!(frame.contains("\"division_body_bridge_valid\":false"));
            assert!(!frame.contains("0x1001"));
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn collector_control_marks_bad_map_control_invalid() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_map_control_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            let state_roots = format!("1:0x{:x}", HEAP_LO + 1);
            let province_bridge = format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=8",
                HEAP_LO + 0x1000
            );
            let controller_bridge =
                format!("1:0x{:x};1:0x{:x}", HEAP_LO + 0x2000, HEAP_LO + 0x3000);
            fs::write(
                &control,
                format!(
                    r#"{{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"country_object_bridge":"1:0x1000","map_control_state_roots":"{}","map_control_province_bridge":"{}","map_control_controller_bridge":"{}"}}"#,
                    state_roots, province_bridge, controller_bridge
                ),
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert!(parsed.country_object_bridge_valid);
            assert!(!parsed.country_object_bridge.is_empty());
            assert!(parsed.map_control.state_roots.is_empty());
            assert!(!parsed.map_control.state_roots_valid);
            assert!(parsed.map_control.province_bridge.is_none());
            assert!(!parsed.map_control.province_bridge_valid);
            assert!(parsed.map_control.controller_bridge.is_empty());
            assert!(!parsed.map_control.controller_bridge_valid);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn country_object_bridge_parser_rejects_duplicates() {
            assert!(parse_country_object_bridge("1:0x1000;1:0x2000").is_none());
            assert!(parse_country_object_bridge("1:0x1000;2:0x1000").is_none());
        }

        #[test]
        fn division_body_bridge_parser_rejects_duplicates_ranges_and_size() {
            assert!(parse_division_body_bridge("").unwrap().is_empty());
            assert!(
                parse_division_body_bridge(&format!("1:0x{:x};2:{}", HEAP_LO, HEAP_LO + 8))
                    .is_some()
            );
            assert!(parse_division_body_bridge(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(
                parse_division_body_bridge(&format!("1:0x{:x};2:0x{:x}", HEAP_LO, HEAP_LO))
                    .is_none()
            );
            assert!(parse_division_body_bridge(&format!("0:0x{:x}", HEAP_LO)).is_none());
            assert!(parse_division_body_bridge(&format!(
                "{}:0x{:x}",
                i32::MAX as u32 + 1,
                HEAP_LO
            ))
            .is_none());
            assert!(parse_division_body_bridge("1:0x1000").is_none());
            assert!(parse_division_body_bridge(&format!("1:0x{:x}", HEAP_LO + 1)).is_none());
            assert!(parse_division_body_bridge(
                &"x".repeat(MAX_DIVISION_BODY_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_DIVISION_BODY_BRIDGE_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_division_body_bridge(&too_many).is_none());
        }

        #[test]
        fn map_control_state_roots_reject_duplicates_ranges_and_size() {
            assert!(parse_map_control_state_roots(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(
                parse_map_control_state_roots(&format!("1:0x{:x};2:0x{:x}", HEAP_LO, HEAP_LO))
                    .is_none()
            );
            assert!(parse_map_control_state_roots("0:0x10000000000").is_none());
            assert!(parse_map_control_state_roots("1:0x1000").is_none());
            assert!(parse_map_control_state_roots(
                &"x".repeat(MAX_MAP_CONTROL_STATE_ROOT_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_MAP_CONTROL_STATE_ROOT_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_map_control_state_roots(&too_many).is_none());
        }

        #[test]
        fn map_control_province_bridge_rejects_bad_fields() {
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=0",
                HEAP_LO
            ))
            .is_some());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_count=13233;province_stride=8;controller_ptr_offset=0",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count={};province_stride=8;controller_ptr_offset=0",
                HEAP_LO,
                MAX_MAP_CONTROL_PROVINCE_COUNT + 1
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=4;controller_ptr_offset=0",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=8",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(
                "province_array=0x1000;province_count=13232;province_stride=8;controller_ptr_offset=0"
            )
            .is_none());
            assert!(parse_map_control_province_bridge(&format!(
                "province_array=0x{:x};province_count=13232;province_stride=8;controller_ptr_offset=0;extra=1",
                HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_province_bridge(
                &"x".repeat(MAX_MAP_CONTROL_PROVINCE_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
        }

        #[test]
        fn map_control_controller_bridge_rejects_duplicates_ranges_and_size() {
            assert!(parse_map_control_controller_bridge(&format!(
                "1:0x{:x};1:0x{:x}",
                HEAP_LO,
                HEAP_LO + 8
            ))
            .is_none());
            assert!(parse_map_control_controller_bridge(&format!(
                "1:0x{:x};2:0x{:x}",
                HEAP_LO, HEAP_LO
            ))
            .is_none());
            assert!(parse_map_control_controller_bridge("0:0x10000000000").is_none());
            assert!(parse_map_control_controller_bridge("1:0x1000").is_none());
            assert!(parse_map_control_controller_bridge(
                &"x".repeat(MAX_MAP_CONTROL_CONTROLLER_BRIDGE_PAYLOAD_BYTES + 1)
            )
            .is_none());
            let too_many = (1..=MAX_MAP_CONTROL_CONTROLLER_BRIDGE_ROWS + 1)
                .map(|i| format!("{}:0x{:x}", i, HEAP_LO + (i * 8)))
                .collect::<Vec<_>>()
                .join(";");
            assert!(parse_map_control_controller_bridge(&too_many).is_none());
        }

        #[test]
        fn collector_control_rejects_unsafe_session_id() {
            let dir = std::env::temp_dir().join(format!(
                "hoi4_collector_control_bad_session_test_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let dll = dir.join("collector.dll");
            let control = dir.join("collector_control.json");
            fs::write(
                &control,
                r#"{"mode":"hourly_poll","duration_ms":8000,"sample_ms":10,"session_id":"bad path"}"#,
            )
            .unwrap();

            let parsed = collector_control(&dll).unwrap();

            assert_eq!(parsed.session_id, None);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn raw_clock_delta_frame_includes_bounded_contract_envelope() {
            let control = raw_test_control();
            let hour_index = ((1936 * 365) * 24 + 12) as u64;
            let sample = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index,
                raw_date: DATE_EPOCH_CONST + hour_index as i32,
            };

            let frame = clock_raw_domain_delta_frame_line(
                1234,
                11,
                Some(5),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &sample,
            )
            .unwrap();

            assert!(frame.ends_with('\n'));
            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"backend\":\"in_process_collector\""));
            assert!(frame.contains("\"collector_version\":\"0.1.0\""));
            assert!(frame.contains("\"abi_version\":1"));
            assert!(frame.contains("\"process_id\":1234"));
            assert!(frame.contains("\"tick_ms\":123456"));
            assert!(frame.contains("\"sequence\":11"));
            assert!(frame.contains("\"profile_id\":\"vanilla_latest_2026_06_28\""));
            assert!(frame.contains("\"session_sequence\":5"));
            assert!(frame.contains("\"duration_ms\":8000"));
            assert!(frame.contains("\"elapsed_wall_ms\":125"));
            assert!(frame.contains("\"date\":\"1936.1.1.12\""));
            assert!(frame.contains(&format!("\"hour_index\":{hour_index}")));
            assert!(frame.contains("\"sample_ms\":10"));
            assert!(frame.contains("\"capture_boundary\":\"worker_poll\""));
            assert!(frame.contains("\"domain\":\"clock\""));
            assert!(frame.contains("\"entity_key\":\"clock:game_date\""));
            assert!(frame.contains("\"source\":\"C\""));
            assert!(frame.contains("\"confidence\":\"sampled\""));
            assert!(frame.contains("\"payload\":{\"op\":\"sample\""));
            assert!(frame.contains("\"after\":{\"date\":\"1936.1.1.12\""));
            assert!(frame.contains(&format!(
                "\"raw_date\":{}",
                DATE_EPOCH_CONST + hour_index as i32
            )));
            assert!(frame.contains("\"changed_fields\":[\"date\",\"hour_index\",\"raw_date\"]"));
            assert!(!frame.contains("\"domains\""));
            assert!(!frame.contains("\"countries\""));
            assert!(!frame.contains("\"states\""));
            assert!(!frame.contains("\"provinces\""));
            assert!(!frame.contains("\"divisions\""));
        }

        #[test]
        fn raw_domain_delta_builder_rejects_unbounded_envelope_values() {
            let too_long = "x".repeat(RAW_SHORT_STRING_MAX_LENGTH + 1);
            let changed_fields = ["date"];

            let rejected_boundary = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: &too_long,
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "sampled",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_boundary.is_none());

            let rejected_pointer_key = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "0x1234",
                confidence: "sampled",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_pointer_key.is_none());

            let accepted_divisions_domain = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "divisions",
                entity_key: "clock:game_date",
                confidence: "sampled",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(accepted_divisions_domain.is_some());

            let rejected_unknown_domain = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "countries",
                entity_key: "clock:game_date",
                confidence: "sampled",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_unknown_domain.is_none());

            let rejected_confidence = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "certain",
                after_json: "{}",
                changed_fields: &changed_fields,
            });
            assert!(rejected_confidence.is_none());

            let rejected_after = raw_domain_delta_frame_line(RawDomainDeltaFrame {
                pid: 1234,
                sequence: 11,
                session_sequence: Some(5),
                tick_ms: 123456,
                date: "1936.1.1.12",
                hour_index: 1,
                sample_ms: 10,
                duration_ms: Some(8000),
                elapsed_wall_ms: Some(125),
                capture_boundary: "worker_poll",
                domain: "clock",
                entity_key: "clock:game_date",
                confidence: "sampled",
                after_json: "[]",
                changed_fields: &changed_fields,
            });
            assert!(rejected_after.is_none());
        }

        #[test]
        fn division_movement_tracker_emits_only_meaningful_changes() {
            let mut previous = BTreeMap::new();
            let stationary = stationary_division_sample();
            assert!(track_division_movement_sample(&mut previous, &stationary).is_none());

            let same_stationary_bucket = DivisionMovementSample {
                raw_organisation: 5_000_999,
                organisation_bucket: division_delta_bucket(5_000_999),
                raw_strength: 100_999,
                strength_bucket: division_delta_bucket(100_999),
                ..stationary.clone()
            };
            assert!(
                track_division_movement_sample(&mut previous, &same_stationary_bucket).is_none()
            );

            let org_bucket_changed = DivisionMovementSample {
                raw_organisation: 5_001_000,
                organisation_bucket: division_delta_bucket(5_001_000),
                ..same_stationary_bucket.clone()
            };
            let org_fields =
                track_division_movement_sample(&mut previous, &org_bucket_changed).unwrap();
            assert_eq!(org_fields, vec!["organisation"]);

            let strength_bucket_changed = DivisionMovementSample {
                raw_strength: 102_000,
                strength_bucket: division_delta_bucket(102_000),
                ..org_bucket_changed.clone()
            };
            let strength_fields =
                track_division_movement_sample(&mut previous, &strength_bucket_changed).unwrap();
            assert_eq!(strength_fields, vec!["strength"]);

            let combat_started = DivisionMovementSample {
                combat_ref_count: 1,
                in_combat: true,
                ..strength_bucket_changed.clone()
            };
            let combat_fields =
                track_division_movement_sample(&mut previous, &combat_started).unwrap();
            assert_eq!(combat_fields, vec!["combat_ref_count", "in_combat"]);

            let combat_ref_incremented = DivisionMovementSample {
                combat_ref_count: 2,
                ..combat_started.clone()
            };
            let combat_ref_fields =
                track_division_movement_sample(&mut previous, &combat_ref_incremented).unwrap();
            assert_eq!(combat_ref_fields, vec!["combat_ref_count"]);

            let moving_bucket_one = DivisionMovementSample {
                moving: true,
                raw_movement_progress: Some(1500),
                progress_bucket: Some(division_delta_bucket(1500)),
                move_target_province: Some(10),
                movement_source_province: Some(5),
                ..combat_ref_incremented.clone()
            };
            let start_fields =
                track_division_movement_sample(&mut previous, &moving_bucket_one).unwrap();
            assert!(start_fields.contains(&"moving"));
            assert!(start_fields.contains(&"movement_progress"));
            assert!(start_fields.contains(&"progress_bucket"));
            assert!(start_fields.contains(&"move_target_province"));
            assert!(start_fields.contains(&"movement_source_province"));

            let same_bucket = DivisionMovementSample {
                raw_movement_progress: Some(1900),
                progress_bucket: Some(division_delta_bucket(1900)),
                ..moving_bucket_one.clone()
            };
            assert!(track_division_movement_sample(&mut previous, &same_bucket).is_none());

            let next_bucket = DivisionMovementSample {
                raw_movement_progress: Some(2500),
                progress_bucket: Some(division_delta_bucket(2500)),
                ..same_bucket.clone()
            };
            let bucket_fields =
                track_division_movement_sample(&mut previous, &next_bucket).unwrap();
            assert_eq!(bucket_fields, vec!["movement_progress", "progress_bucket"]);

            let moved_location = DivisionMovementSample {
                location: 11,
                move_target_province: Some(11),
                ..next_bucket.clone()
            };
            let location_fields =
                track_division_movement_sample(&mut previous, &moved_location).unwrap();
            assert_eq!(location_fields, vec!["location", "move_target_province"]);

            let stopped = DivisionMovementSample {
                moving: false,
                raw_movement_progress: None,
                progress_bucket: None,
                location: 11,
                move_target_province: None,
                movement_source_province: None,
                ..moved_location.clone()
            };
            let stop_fields = track_division_movement_sample(&mut previous, &stopped).unwrap();
            assert!(stop_fields.contains(&"moving"));
            assert!(stop_fields.contains(&"movement_progress"));
            assert!(stop_fields.contains(&"progress_bucket"));
            assert!(stop_fields.contains(&"move_target_province"));
            assert!(stop_fields.contains(&"movement_source_province"));

            let mut fresh = BTreeMap::new();
            let first_moving =
                track_division_movement_sample(&mut fresh, &moving_bucket_one).unwrap();
            assert_eq!(first_moving, division_movement_all_fields());
        }

        #[test]
        fn division_movement_frame_builder_emits_sanitized_raw_delta() {
            let control = raw_test_control();
            let clock = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };
            let sample = DivisionMovementSample {
                div_id: 7,
                moving: true,
                raw_movement_progress: Some(12000),
                progress_bucket: Some(division_delta_bucket(12000)),
                location: 11597,
                move_target_province: Some(11597),
                movement_source_province: Some(11333),
                raw_organisation: 7_654_321,
                organisation_bucket: division_delta_bucket(7_654_321),
                raw_strength: 98_765,
                strength_bucket: division_delta_bucket(98_765),
                combat_ref_count: 2,
                in_combat: true,
            };
            let changed_fields = division_movement_all_fields();

            let frame = division_raw_domain_delta_frame_line(
                1234,
                11,
                Some(5),
                123456,
                &control,
                Some(125),
                "worker_poll",
                &clock,
                &sample,
                &changed_fields,
            )
            .unwrap();

            assert!(frame.contains("\"type\":\"domain_delta\""));
            assert!(frame.contains("\"domain\":\"divisions\""));
            assert!(frame.contains("\"entity_key\":\"division:7:4713\""));
            assert!(frame.contains("\"payload\":{\"op\":\"sample\""));
            assert!(frame.contains("\"div_uid\":\"7:4713\""));
            assert!(frame.contains("\"div_id\":7"));
            assert!(frame.contains("\"div_type\":4713"));
            assert!(frame.contains("\"moving\":true"));
            assert!(frame.contains("\"movement_progress\":0.12"));
            assert!(frame.contains("\"progress_bucket\":12"));
            assert!(frame.contains("\"location\":11597"));
            assert!(frame.contains("\"move_target_province\":11597"));
            assert!(frame.contains("\"movement_source_province\":11333"));
            assert!(frame.contains("\"organisation\":76.54321"));
            assert!(frame.contains("\"strength\":0.98765"));
            assert!(frame.contains("\"combat_ref_count\":2"));
            assert!(frame.contains("\"in_combat\":true"));
            assert!(frame.contains("\"changed_fields\":[\"div_uid\",\"div_id\",\"div_type\",\"moving\",\"movement_progress\",\"progress_bucket\",\"location\",\"move_target_province\",\"movement_source_province\",\"organisation\",\"strength\",\"combat_ref_count\",\"in_combat\"]"));
            assert!(!frame.contains("body_ptr"));
            assert!(!frame.contains("body_addr"));
            assert!(!frame.contains("tag_db_id"));
            assert!(!frame.contains("\"owner\""));
            assert!(!frame.contains("0x"));
        }

        #[test]
        fn raw_clock_delta_does_not_pollute_snapshot_domain_claims() {
            let control = raw_test_control();
            let sample = ClockSample {
                date: "1936.1.1.12".to_string(),
                hour_index: ((1936 * 365) * 24 + 12) as u64,
                raw_date: DATE_EPOCH_CONST + ((1936 * 365) * 24 + 12),
            };

            let raw = clock_raw_domain_delta_frame_line(
                1234,
                11,
                Some(5),
                123456,
                &control,
                Some(125),
                "scheduled_hour_snapshot",
                &sample,
            )
            .unwrap();
            assert!(raw.contains("\"domain\":\"clock\""));
            assert!(!raw.contains("\"domains\""));
            assert!(!raw.contains("\"root_probe\""));

            let snapshot = build_frame_line(
                1234,
                None,
                Some(&control),
                Some(7),
                Some(125),
                Some("scheduled_hour_snapshot"),
            );

            assert!(snapshot.contains("\"type\":\"collector_started\""));
            assert!(snapshot.contains("\"domains\":[]"));
            assert!(!snapshot.contains("\"domain\":\"clock\""));
            assert!(!snapshot.contains("\"payload\""));
        }

        #[test]
        fn worker_frame_without_state_includes_session_metadata_and_no_domains() {
            let control = CollectorControl {
                duration_ms: 8000,
                sample_ms: 10,
                country_object_scan: false,
                country_object_bridge: Vec::new(),
                country_object_bridge_valid: true,
                country_record_bridge: Vec::new(),
                country_record_bridge_valid: true,
                division_body_bridge: Vec::new(),
                division_body_bridge_valid: true,
                map_control: MapControlContract::empty(),
                scheduled_snapshot_step_hours: 1,
                pipe_token: Some("abc".to_string()),
                session_id: Some("run_1".to_string()),
                sidecar_log: false,
            };

            let frame = build_frame_line(
                1234,
                None,
                Some(&control),
                Some(7),
                Some(125),
                Some("worker_poll"),
            );

            assert!(frame.contains("\"type\":\"collector_started\""));
            assert!(frame.contains("\"session_id\":\"run_1\""));
            assert!(frame.contains("\"session_sequence\":7"));
            assert!(frame.contains("\"elapsed_wall_ms\":125"));
            assert!(frame.contains("\"capture_boundary\":\"worker_poll\""));
            assert!(frame.contains("\"duration_ms\":8000"));
            assert!(frame.contains("\"sample_ms\":10"));
            assert!(frame.contains("\"scheduled_snapshot_step_hours\":1"));
            assert!(frame.contains("\"map_control_state_root_rows\":0"));
            assert!(frame.contains("\"map_control_state_roots_valid\":true"));
            assert!(frame.contains("\"map_control_province_bridge_present\":false"));
            assert!(frame.contains("\"map_control_province_bridge_valid\":true"));
            assert!(frame.contains("\"map_control_controller_bridge_rows\":0"));
            assert!(frame.contains("\"map_control_controller_bridge_valid\":true"));
            assert!(frame.contains("\"division_body_bridge_rows\":0"));
            assert!(frame.contains("\"division_body_bridge_valid\":true"));
            assert!(frame.contains("\"profile_id\":\"vanilla_latest_2026_06_28\""));
            assert!(frame.contains("\"domains\":[]"));
            assert!(frame.contains("\"root_probe\":null"));
        }

        #[test]
        fn domain_list_claims_province_control_only_with_province_rows() {
            let cache = CountryIdentityCache::not_attempted("test");

            let without_provinces = cache.domains_json_from_rows(&[], true, false, false, false);
            assert!(without_provinces.contains("\"state_ownership\""));
            assert!(without_provinces.contains("\"state_control\""));
            assert!(!without_provinces.contains("\"province_control\""));

            let with_provinces = cache.domains_json_from_rows(&[], false, true, false, false);
            assert!(!with_provinces.contains("\"state_control\""));
            assert!(with_provinces.contains("\"province_control\""));
        }

        #[test]
        fn country_object_row_json_includes_tag_db_id() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: Some(0x5000),
                original_capital: Some(112),
                original_tag: None,
                templates_locked: Some(false),
                reserved_dynamic_country: Some(false),
                stability: Some(0.61),
                war_support: Some(0.42),
                political_power: Some(7.25),
                command_power: Some(12.5),
                num_ships: Some(7),
                manpower_pool: Some(123456),
                convoys: Some(99.0),
                misc: None,
                fuel: None,
                production: None,
            };

            let json = row_json(&row, false, false);

            assert!(json.contains("\"POR\""));
            assert!(json.contains("\"tag_db_id\":47"));
            assert!(json.contains("\"stability\":0.61"));
            assert!(json.contains("\"political_power\":7.25"));
            assert!(json.contains("\"manpower_pool\":123456"));
            assert!(!json.contains("\"fuel\""));
        }

        #[test]
        fn country_object_row_json_includes_country_misc_fields() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: Some(0x5000),
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: Some(CountryMiscRow {
                    convoys_destroyed: Some(12),
                    accidents_score: Some(0.5),
                    ace_count: Some(3),
                    cosmetic_tag: Some(None),
                }),
                fuel: None,
                production: None,
            };

            let json = row_json(&row, false, false);

            assert!(json.contains("\"misc\""));
            assert!(json.contains("\"convoys_destroyed\":12"));
            assert!(json.contains("\"accidents_score\":0.5"));
            assert!(json.contains("\"ace_count\":3"));
            assert!(json.contains("\"cosmetic_tag\":null"));
            let cache = CountryIdentityCache::not_attempted("test");
            let domains = cache.domains_json_from_rows(&[row], false, false, false, false);
            assert!(domains.contains("\"country_misc\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_fuel() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: Some(CountryFuelRow {
                    current: 123.5,
                    max: 456.25,
                    gain: 7.5,
                    cost: -2.25,
                }),
                production: None,
            };

            let scheduled = row_json(&row, true, false);
            let worker = row_json(&row, false, false);

            assert!(scheduled.contains(
                "\"fuel\":{\"current\":123.5,\"max\":456.25,\"gain\":7.5,\"cost\":-2.25}"
            ));
            assert!(!worker.contains("\"fuel\""));
        }

        #[test]
        fn country_object_row_json_includes_scheduled_production() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: Some(CountryProductionRow {
                    stockpile: Some(vec![
                        CountryStockpileRow {
                            variant_id: 70,
                            amount: 123.5,
                        },
                        CountryStockpileRow {
                            variant_id: 71,
                            amount: -0.25,
                        },
                    ]),
                    military_lines: Some(vec![CountryProductionLineRow {
                        id: 349,
                        active_factories: 3,
                        cost: 105.0,
                        speed: 31.05,
                        produced: 34.4,
                        amount: -1,
                    }]),
                    naval_lines: Some(Vec::new()),
                }),
            };

            let scheduled = row_json(&row, false, true);
            let worker = row_json(&row, false, false);

            assert!(scheduled.contains("\"production\""));
            assert!(scheduled.contains("\"stockpile\":{\"70\":123.5,\"71\":-0.25}"));
            assert!(scheduled.contains("\"stockpile_total\":123.25"));
            assert!(scheduled.contains("\"military_factories\":3"));
            assert!(scheduled.contains("\"naval_factories\":0"));
            assert!(scheduled.contains("\"amount\":-1"));
            assert!(!worker.contains("\"production\""));
        }

        #[test]
        fn country_object_row_json_omits_unavailable_production_leaves() {
            let row = CountryObjectRow {
                tag: "POR".to_string(),
                tag_db_id: 47,
                name: "Portugal".to_string(),
                object_ptr: 0x1234,
                record_ptr: None,
                original_capital: None,
                original_tag: None,
                templates_locked: None,
                reserved_dynamic_country: None,
                stability: None,
                war_support: None,
                political_power: None,
                command_power: None,
                num_ships: None,
                manpower_pool: None,
                convoys: None,
                misc: None,
                fuel: None,
                production: Some(CountryProductionRow {
                    stockpile: None,
                    military_lines: Some(vec![CountryProductionLineRow {
                        id: 349,
                        active_factories: 3,
                        cost: 105.0,
                        speed: 31.05,
                        produced: 34.4,
                        amount: -1,
                    }]),
                    naval_lines: Some(Vec::new()),
                }),
            };

            let scheduled = row_json(&row, false, true);

            assert!(scheduled.contains("\"production\""));
            assert!(!scheduled.contains("\"stockpile\""));
            assert!(!scheduled.contains("\"stockpile_total\""));
            assert!(scheduled.contains("\"military_lines\""));
            assert!(scheduled.contains("\"military_factories\":3"));
            assert!(scheduled.contains("\"naval_lines\":[]"));
            assert!(scheduled.contains("\"naval_factories\":0"));
        }
    }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    unsafe { attach_log::dll_main(module, reason) }
}

#[cfg(windows)]
mod hook_proc {
    use core::ffi::c_void;
    use core::ptr;

    #[link(name = "user32")]
    extern "system" {
        fn CallNextHookEx(hook: *mut c_void, code: i32, wparam: usize, lparam: isize) -> isize;
    }

    pub unsafe fn get_msg_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn Hoi4ReplayGetMsgProc(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe { hook_proc::get_msg_proc(code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_exports_expected_metadata() {
        let json = core::str::from_utf8(heartbeat_json_for_tests()).unwrap();
        assert!(json.contains("\"type\":\"heartbeat\""));
        assert!(json.contains("\"backend\":\"in_process_collector\""));
        assert!(json.contains("\"collector_version\":\"0.1.0\""));
        assert_eq!(battle-plan_collector_abi_version(), ABI_VERSION);
        assert_eq!(
            battle-plan_collector_heartbeat_len(),
            heartbeat_json_for_tests().len()
        );
    }

    #[test]
    fn heartbeat_copy_reports_required_length() {
        let mut buf = [0u8; 16];
        let required = copy_heartbeat_json(&mut buf);
        assert_eq!(required, heartbeat_json_for_tests().len());
        assert_eq!(&buf, &heartbeat_json_for_tests()[..16]);
    }

    #[test]
    fn heartbeat_abi_accepts_null_for_length_probe() {
        let required = unsafe { battle-plan_collector_heartbeat_json(ptr::null_mut(), 0) };
        assert_eq!(required, heartbeat_json_for_tests().len());
    }
}
