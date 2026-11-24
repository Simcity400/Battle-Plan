#[cfg(test)]
use std::os::windows::ffi::OsStrExt;
mod history_requests;
use history_requests::{HistoryWorker, SpoolHistory};

use collector_capture::acquisition_jobs as acquisition;
use collector_capture::decode;
use collector_capture::sampling;
use collector_capture::sampling::Shared;
use collector_capture::sampling::{SHARED, SharedSlot};
use collector_hooks::hook::Hook;
use collector_hooks::hook::Threads;
use collector_primitives::capacity::CapacityMax;
use collector_spool::history::PreparedCampaigns;
#[cfg(test)]
use collector_spool::spooling::HistoryReply;
use collector_spool::spooling::Processor;
use collector_workers::spool_phases::Phase;
use engine_executable::build::hash_executable;
use engine_native::cgame_state;
use engine_native_memory::{memory::LiveMemory, read};
use engine_native_intelligence::country_memory::SeedStatus;
use replay_data::identity::IdentityCodec;
use replay_data::{codec::Decoder, identity::SessionStart};

use crate::status_file::{write_status, write_status_bytes};
use collector_capture::hourly_admission::{CaptureAdmission, HOURLY_ADMISSION};
use collector_workers::{
    event::Event,
    priority::{BACKGROUND_STACK_BYTES, OwnPriority},
};
use serde::Deserialize;
#[cfg(test)]
use serde_json::{Value, json};
use std::{
    ffi::{OsString, c_void},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek},
    os::windows::{
        ffi::OsStringExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
        HMODULE, SetLastError,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFileSizeEx, OPEN_EXISTING,
    },
    System::{
        LibraryLoader::{FreeLibraryAndExitThread, GetModuleFileNameW, GetModuleHandleW},
        Performance::QueryPerformanceFrequency,
        Threading::*,
    },
};

macro_rules! refuse_detach {
    ($reason:expr, $retain:expr) => {{
        let reason = $reason;
        // SAFETY: Session detach refusal owns teardown of land-event and combat-history observation.
        unsafe {
            collector_land_events::land_events::stop_observation();
            collector_combat_log::history_events::stop();
        }
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
    #[serde(default)]
    scripted_maps: collector_states::scripted_maps::Demand,
    #[serde(default)]
    without_history: Vec<String>,
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
    idle_event: String,
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
    #[serde(default)]
    map_events: Option<Vec<String>>,
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
    collector_memory::terminal::observe(&error);
    reason
}
pub(crate) fn configure_alloc_fault(fault: Option<&AllocFault>) -> Result<(), &'static str> {
    let Some(fault) = fault else {
        return Ok(());
    };
    #[cfg(feature = "alloc-fault-injection")]
    {
        let phase = collector_memory::terminal::Phase::parse(&fault.phase)
            .ok_or("alloc_fault names an unsupported phase")?;
        if fault.ordinal == 0 {
            return Err("alloc_fault ordinal must be nonzero");
        }
        collector_memory::terminal::inject_after(phase, fault.ordinal, fault.after_hours);
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

struct Owner<'a> {
    handle: HANDLE,
    shared: &'a SharedSlot,
}
impl<'a> Owner<'a> {
    fn claim(process_start: u64, shared: &'a SharedSlot) -> Result<Self, &'static str> {
        let name = format!("Local\\BattlePlan.Collector.{}.{process_start}", unsafe {
            GetCurrentProcessId()
        });
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe { SetLastError(0) };
        let handle = unsafe { CreateMutexW(ptr::null(), 0, wide.as_ptr()) };
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if handle.is_null() {
            collector_memory::terminal::native_error(&io::Error::last_os_error());
            return Err("CreateMutexW failed");
        }
        let owner = Self { handle, shared };
        if existed {
            return Err("another collector already runs in this game");
        }
        Ok(owner)
    }
}
impl Drop for Owner<'_> {
    fn drop(&mut self) {
        if !self.shared.is_published() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    #[test]
    fn a_second_collector_in_one_game_is_refused_until_the_first_ends() {
        let process_start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let shared = SharedSlot::default();
        let first = Owner::claim(process_start, &shared).unwrap();
        assert_eq!(
            Owner::claim(process_start, &shared).err(),
            Some("another collector already runs in this game")
        );
        assert!(
            Owner::claim(process_start + 1, &shared).is_ok(),
            "another game"
        );
        drop(first);
        assert!(Owner::claim(process_start, &shared).is_ok());
    }

    #[test]
    fn a_run_that_retains_its_hooks_keeps_the_game_claimed() {
        let process_start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            ^ 1;
        let retained = SharedSlot::retained_for_test();
        drop(Owner::claim(process_start, &retained).unwrap());
        assert_eq!(
            Owner::claim(process_start, &SharedSlot::default()).err(),
            Some("another collector already runs in this game")
        );
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
fn installation_deadline_message(family: &str, busy_sites: &[&str]) -> String {
    let detail = if busy_sites.is_empty() {
        String::new()
    } else {
        format!("/{}", busy_sites.join(", "))
    };
    format!(
        "{family}{detail} hooks were still not ready when the 15 s installation deadline expired"
    )
}

pub(crate) fn installation_message(message: impl Into<String>) -> &'static str {
    let message = message.into();
    Box::leak(message.into_boxed_str())
}

/// # Safety
/// Only the attach thread `DllMain` creates runs this, with `module` the collector's own handle.
pub unsafe extern "system" fn worker(module: *mut c_void) -> u32 {
    collector_workers::private_heap::off_hold_path();
    let _bootstrap =
        collector_memory::terminal::enter(collector_memory::terminal::Phase::Bootstrap);
    collector_memory::terminal::install_hooks();
    let witness = bootstrap_witness(module);
    let mut spool_final = false;
    let result = match &witness {
        Ok((file, session, process_start)) => {
            collector_memory::terminal::prepare(file, *session, *process_start);
            run(module, *session, *process_start, &mut spool_final)
        }
        Err(error) => Err(*error),
    };
    if let Err(error) = result
        && !spool_final
        && !collector_memory::terminal::failed()
        && let Ok(path) = module_path(module)
    {
        let _ = fs::write(path.with_file_name("collector.error.txt"), error);
    }
    finish_worker(&SHARED, witness, |witness| {
        let resident = resident_reason(collector_workers::module_threads::started_in(module));
        if let Some(reason) = &resident
            && let Ok(path) = module_path(module)
        {
            let _ = fs::write(path.with_file_name(RESIDENT_FILE), reason);
        }
        collector_memory::terminal::clear_handle();
        engine_allocation::set_hooks(None, None);
        engine_allocation::set_stop_hook(None);
        drop(witness);
        if resident.is_none() {
            unsafe {
                FreeLibraryAndExitThread(module, 0);
            }
        }
        0
    })
}

const RESIDENT_FILE: &str = "collector.resident.txt";

fn resident_reason(threads: Result<Vec<u32>, String>) -> Option<String> {
    match threads {
        Ok(threads) if threads.is_empty() => None,
        Ok(threads) => Some(format!(
            "collector stays loaded: {} of its threads are still running",
            threads.len()
        )),
        Err(reason) => Some(format!("collector stays loaded: {reason}")),
    }
}

fn finish_worker<T>(shared: &SharedSlot, witness: T, unload: impl FnOnce(T) -> u32) -> u32 {
    if shared.is_published() {
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
    let filename = replay_data::witness::WITNESS_FILE_NAME.as_bytes();
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
    if unsafe { GetFileSizeEx(file.as_raw_handle(), &raw mut length) } == 0 {
        return Err("collector bootstrap witness size unreadable");
    }
    if length != replay_data::witness::WITNESS_BYTES as i64 {
        return Err("collector bootstrap witness length invalid");
    }
    let mut bytes = [0u8; replay_data::witness::WITNESS_BYTES];
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
    let (session, process_start) = replay_data::witness::TerminalWitness::decode_prepared(&bytes)
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
        let witness = directory.join(replay_data::witness::WITNESS_FILE_NAME);
        fs::write(
            &witness,
            replay_data::witness::TerminalWitness::prepared(1, 1000),
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
fn run(
    module: HMODULE,
    prepared_session: u64,
    prepared_process_start: u64,
    spool_final: &mut bool,
) -> Result<(), &'static str> {
    engine_sites::production();
    collector_timing::hook_timing::start();
    // SAFETY: Only the current session's designated thread registers this role.
    unsafe {
        collector_workers::thread_cpu::register(collector_workers::thread_cpu::Role::Runtime)
    };
    #[cfg(feature = "worker_sampler")]
    // SAFETY: The caller owns the session and runs preparation before admitting consumers.
    unsafe {
        collector_workers::worker_sampler::prepare()
    };
    let dll_path = module_path(module)?;
    let config: Config = serde_json::from_slice(
        &fs::read(dll_path.with_file_name("collector.json"))
            .map_err(|e| error_reason(e, "collector config read failed"))?,
    )
    .map_err(|_| "collector JSON config invalid")?;
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
    let _allocation_details = collector_memory::terminal::prepare_details(&config.output_dir);
    let mut creation = unsafe { std::mem::zeroed() };
    let mut exit = unsafe { std::mem::zeroed() };
    let mut kernel = unsafe { std::mem::zeroed() };
    let mut user = unsafe { std::mem::zeroed() };
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
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
    let _owner = Owner::claim(process_start, &SHARED)?;
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&raw mut frequency) } == 0
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
    if session.identity.exe_hash != exe.sha256 || session.identity.exe_size != exe.size {
        return Err("collector config executable identity differs from running build");
    }
    collector_observers::territory_changes::scripted_capture(
        config.scripted_maps.victory_point,
        config.scripted_maps.all_controllers || !config.scripted_maps.controller_states.is_empty(),
    );
    collector_states::scripted_maps::configure(config.scripted_maps.clone());
    engine_research::research_modifiers::scripted_sharing_roster(
        config.scripted_maps.sharing_roster,
    );
    let tables = engine_executable::executable_tables::initialize_executable(&executable_path)
        .map_err(|error| error_reason(error, "collector executable tables preparation failed"))?;
    collector_primitives::build_gate::record(tables.recognised(), tables.build_warning());
    let dll = hash_executable(
        File::open(&dll_path).map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?,
    )
    .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    if session.identity.collector_hash != dll.sha256 {
        return Err("collector DLL hash differs from session identity");
    }
    let ready = Event::create(&config.ready_event)?;
    let detach = Event::create(&config.detach_event)?;
    let done = Event::create(&config.done_event)?;
    let idle = Event::create(&config.idle_event)?;
    let status_path = config.output_dir.join("status.json");
    collector_hook_failures::configure(
        &config.output_dir,
        session.id,
        process_start,
        write_status_bytes,
    );
    write_status(
        &status_path,
        &collector_status::status::status!({
            "state":"starting", "session":session.id, "hours":0,
            "build":collector_primitives::build_gate::status(),
        }),
    )
    .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    let progress = crate::progress::Progress::open(&config.output_dir);
    let worker_panic =
    // SAFETY: The runtime owns the process hook until all workers exit, or the fixture owns it in
    // isolation.
    unsafe { collector_workers::worker_panic::Guard::install(
        &config.output_dir.join(crate::progress::FILE),
        progress.opened(),
    ) };
    progress.step("attach_started", None);
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let memory = unsafe { LiveMemory::new() };
    let copy =
        |address: usize, bytes: &mut [u8]| engine_native_memory::Memory::copy(&memory, address, bytes);
    let image =
        engine_sites::Image::live(base, &copy).map_err(|_| "game executable headers unreadable")?;
    let located = engine_sites::install(engine_sites::Located::locate(
        &image,
        engine_sites::layout_checked(&exe.sha256),
    ));
    let sites = located.summary();
    progress.step(
        format_args!(
            "sites_located verified={} of={} functions={}/{} layouts={} us={}",
            sites.keys_verified,
            sites.keys,
            sites.functions_found,
            sites.functions,
            match (sites.layout_checked, sites.layout_verified) {
                (true, true) => "checked",
                (false, true) => "witnessed",
                (_, false) => "unverified",
            },
            sites.elapsed.as_micros(),
        ),
        None,
    );
    for (_, withheld) in located.withheld() {
        progress.step(format_args!("site {withheld}"), None);
    }
    progress.step(
        format_args!("sites_moved {}", located.moved().count()),
        None,
    );
    if engine_sites::layout_checked(&exe.sha256) {
        for (key, live) in located.moved() {
            progress.step(
                format_args!(
                    "site_moved {} {:#x} -> {live:#x}",
                    engine_sites::name(key),
                    key.0
                ),
                None,
            );
        }
    }
    collector_hook_failures::admit_essential(crate::hook_inventory::patches, installation_message)?;
    let hourly = engine_sites::hook(base, cgame_state::HOURLY_UPDATE_RVA)
        .and_then(|code| Ok((code.address, code.sentinel::<16>()?)));
    let (hourly_site, sentinel) = match hourly {
        Ok(site) => site,
        Err(cause) => {
            let _ = collector_hook_failures::refuse(
                crate::hook_inventory::patches,
                RestoreFamily::Hourly,
                "sentinel",
                cause,
            );
            return Err(cause);
        }
    };
    let output_lock = reserve_output(&config, &detach)?;
    progress.step("output_reserved", None);
    let mut history = PreparedCampaigns::load(&config.replay_dir, &session, |campaign| {
        if config.cancel_file.exists() || detach.signaled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        write_status(
            &status_path,
            &collector_status::status::status!({
                "state":"preparing campaign history", "session":session.id,
                "campaign":campaign, "hours":0,
                "build":collector_primitives::build_gate::status(),
            }),
        )
    })
    .map_err(|error| error_reason(error, "collector bootstrap preparation failed"))?;
    for campaign in &config.without_history {
        history.without_history(campaign);
    }
    // SAFETY: Attach owns restoring the politics observation epoch before enabling hooks.
    unsafe { collector_politics::politics_changes::restore_epoch(history.politics_epoch()) }?;
    collector_operation_outcomes::operation_outcomes::restore_sequence(
        history.completion_sequence(),
    )?;
    let (map_event_epoch, map_event_sequence) = history.map_events();
    collector_map_events::interval::begin(map_event_epoch, map_event_sequence)?;
    collector_naval_results::naval_result_sources::restore_sequence(
        history.naval_result_sequence(),
    )?;
    progress.step("history_loaded", None);
    let event_feed_sequence = history.event_feed_sequence();
    let event_feed_generation = history.event_feed_generation().unwrap_or(1).max(1);
    let event_feed_counts = history.event_feed_counts();
    let [
        air_loss_sequence,
        bombing_sequence,
        nuclear_sequence,
        market_sequence,
    ] = history.event_ledger_sequences();
    collector_observers::air_losses::restore_sequence(air_loss_sequence)?;
    collector_observers::building_bombing::restore_sequence(bombing_sequence)?;
    collector_observers::nuclear_strikes::restore_sequence(nuclear_sequence)?;
    collector_observers::market_changes::restore_sequence(market_sequence)?;
    let mut processor =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Processor::new(&config.output_dir, history, session.clone()) }
        .map_err(|error| error_reason(error, "collector bootstrap preparation failed"))?;
    let mut hook =
        unsafe { Hook::new(hourly_site, sampling::tick as *const () as usize, sentinel) }.map_err(
            |cause| {
                let _ = collector_hook_failures::refuse(
                    crate::hook_inventory::patches,
                    RestoreFamily::Hourly,
                    "construction",
                    &cause,
                );
                installation_message(format!("hourly hook construction failed: {cause}"))
            },
        )?;
    let mut batch_hooks = FamilyHooks::construct(base, || unsafe {
        collector_capture::command_batch::Hooks::new(&memory, base)
    })?;
    let mut portrait_hooks = FamilyHooks::construct(base, || unsafe {
        collector_characters::portrait_changes::Hooks::new(&memory, base)
    })?;
    let mut advisor_hooks = FamilyHooks::construct(base, || unsafe {
        collector_characters::advisor_changes::Hooks::new(&memory, base)
    })?;
    let mut relation_hooks = FamilyHooks::construct(base, || unsafe {
        collector_air_relations::relation_changes::Hooks::new(&memory, base)
    })?;
    let mut history_hooks = FamilyHooks::construct(base, || unsafe {
        collector_air_histories::history_changes::Hooks::new(&memory, base)
    })?;
    let mut flag_hooks = FamilyHooks::construct(base, || unsafe {
        collector_intel::flag_changes::Hooks::new(&memory, base)
    })?;
    let mut building_hooks = FamilyHooks::construct(base, || unsafe {
        collector_construction::building_changes::Hooks::new(&memory, base)
    })?;
    let mut advisor_pool_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::advisor_pool_changes::Hooks::new(&memory, base)
    })?;
    let mut party_leader_hooks = FamilyHooks::construct(base, || unsafe {
        collector_governance::party_leader_changes::Hooks::new(&memory, base)
    })?;
    let mut variable_hooks = FamilyHooks::construct(base, || unsafe {
        collector_intel::variable_changes::Hooks::new(&memory, base)
    })?;
    let mut party_hooks = FamilyHooks::construct(base, || unsafe {
        collector_governance::party_changes::Hooks::new(&memory, base)
    })?;
    let mut wing_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::wing_changes::Hooks::new(&memory, base)
    })?;
    let mut roster_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::roster_changes::Hooks::new(&memory, base)
    })?;
    let mut political_status_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::political_status_changes::Hooks::new(&memory, base)
    })?;
    let mut mission_hooks = FamilyHooks::construct(base, || unsafe {
        collector_missions::mission_changes::Hooks::new(&memory, base)
    })?;
    let mut rail_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::rail_changes::Hooks::new(&memory, base)
    })?;
    let mut territory_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::territory_changes::Hooks::new(&memory, base)
    })?;
    let mut army_stat_hooks = FamilyHooks::construct(base, || unsafe {
        collector_division_parts::army_stat_changes::Hooks::new(&memory, base)
    })?;
    let mut country_input_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::country_input_changes::Hooks::new(&memory, base)
    })?;
    let mut raid_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::raid_changes::Hooks::new(&memory, base)
    })?;
    let mut naval_tree_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::naval_tree_changes::Hooks::new(&memory, base)
    })?;
    let mut unit_leader_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::unit_leader_changes::Hooks::new(&memory, base)
    })?;
    let mut census_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::census_changes::Hooks::new(&memory, base)
    })?;
    let mut world_input_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::world_input_changes::Hooks::new(&memory, base)
    })?;
    let mut air_record_hooks = FamilyHooks::construct(base, || unsafe {
        collector_air_records::air_record_changes::Hooks::new(&memory, base)
    })?;
    let mut local_subunit_hooks = FamilyHooks::construct(base, || unsafe {
        collector_model_inputs::local_subunit_changes::Hooks::new(&memory, base)
    })?;
    let mut template_hooks = FamilyHooks::construct(base, || unsafe {
        collector_division_templates::template_changes::Hooks::new(&memory, base)
    })?;
    let mut membership_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::order_membership_changes::Hooks::new(&memory, base)
    })?;
    let mut orders_group_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::orders_group_changes::Hooks::new(&memory, base)
    })?;
    let mut ship_model_hooks = FamilyHooks::construct(base, || unsafe {
        collector_ship_models::ship_model_changes::Hooks::new(&memory, base)
    })?;
    let mut province_unit_hooks = FamilyHooks::construct(base, || unsafe {
        collector_divisions::province_unit_changes::Hooks::new(&memory, base)
    })?;
    let mut province_combat_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::province_combat_changes::Hooks::new(&memory, base)
    })?;
    let mut air_base_carry_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::air_base_carry_changes::Hooks::new(&memory, base)
    })?;

    let mut propaganda_hooks = FamilyHooks::construct(base, || unsafe {
        collector_intel::propaganda_changes::Hooks::new(
            &memory,
            base,
            collector_extended_support::outcomes::records_any(
                session.capture_mode,
                &[replay_data::rows::TableId::PropagandaEfforts],
            ),
        )
    })?;
    let mut intel_source_hooks = FamilyHooks::construct(base, || unsafe {
        collector_intel::source_changes::Hooks::new(
            &memory,
            base,
            collector_extended_support::outcomes::records_any(
                session.capture_mode,
                &[
                    replay_data::rows::TableId::IntelContributions,
                    replay_data::rows::TableId::IntelSources,
                ],
            ),
        )
    })?;
    let mut network_auxiliary_hooks = FamilyHooks::construct(base, || unsafe {
        collector_intel::network_auxiliary::Hooks::new(
            &memory,
            base,
            collector_extended_support::outcomes::records_any(
                session.capture_mode,
                &[replay_data::rows::TableId::IntelNetworkAuxiliaries],
            ),
        )
    })?;
    let naval_state = engine_sites::at(base, cgame_state::SINGLETON_RVA)
        .and_then(|at| read::<u64>(&memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .unwrap_or(0);
    let mut naval_management_hooks = FamilyHooks::construct(base, || unsafe {
        collector_naval_management::changes::Hooks::new(&memory, base, naval_state)
    })?;
    let mut participation_hooks = FamilyHooks::construct(base, || unsafe {
        collector_war_overview::participation::Hooks::new(&memory, base)
    })?;
    let mut design_hooks = FamilyHooks::construct(base, || unsafe {
        collector_production::production::design_changes::Hooks::new(&memory, base)
    })?;
    let mut faction_goal_hooks = FamilyHooks::construct(base, || unsafe {
        collector_factions::faction_goal_changes::Hooks::new(&memory, base)
    })?;
    let mut faction_companion_hooks = FamilyHooks::construct(base, || unsafe {
        collector_factions::faction_companion_changes::Hooks::new(&memory, base)
    })?;
    let _division_panel_forwarder = crate::division_panel_forwarder::attach();
    let mut opinion_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::opinion_changes::Hooks::new(&memory, base)
    })?;
    let mut destruction_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_equipment_destruction::equipment_destruction::Hooks::new(&memory, base)
        })
    })?;
    let mut naval_result_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_naval_results::naval_result_sources::Hooks::new(&memory, base)
        })
    })?;
    let mut experience_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_experience_gains::experience_gains::Hooks::new(&memory, base)
        })
    })?;
    let mut command_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_player_commands::player_commands::Hooks::new(
                &memory,
                base,
                sampling::sample_command_roster,
            )
        })
    })?;
    let mut air_loss_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe { collector_observers::air_losses::Hooks::new(&memory, base) })
    })?;
    let mut bombing_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_observers::building_bombing::Hooks::new(&memory, base)
        })
    })?;
    let mut engagement_refresh_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_observers::combat_engagement_refreshes::Hooks::new(&memory, base)
        })
    })?;
    let mut nuclear_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_observers::nuclear_strikes::Hooks::new(&memory, base)
        })
    })?;
    let mut map_event_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_map_events::hooks::Hooks::new(&memory, base, |family| {
                config
                    .map_events
                    .as_ref()
                    .is_none_or(|selected| selected.iter().any(|name| name == family.name()))
            })
        })
    })?;
    let mut market_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_observers::market_changes::Hooks::new(&memory, base)
        })
    })?;
    let events =
        collector_capture::country_scale::observe_production_output(&memory, base).unwrap_or(0);
    let mut line_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_output::line_output::Hooks::new(&memory, base, events)
        })
    })?;
    let mut world_hooks = FamilyHooks::construct(base, || unsafe {
        collector_lifetime::world_lifetime::Hooks::new(&memory, base, event_feed_generation)
    })?;
    let mut land_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_land_events::land_events::Hooks::new(&memory, base)
        })
    })?;
    let mut prune_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_combat_log::combat_log_prune::Hooks::new(&memory, base)
        })
    })?;
    let mut construction_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_output::construction_ic::Hooks::new(&memory, base)
        })
    })?;
    let logistics_rows = collector_logistics::logistics_balance::prepare(&memory, base)
        .map_err(|e| error_reason(e, "logistics balance staging allocation failed"))?;
    let mut logistics_hooks = FamilyHooks::construct(base, || unsafe {
        collector_logistics::logistics_balance::Hooks::new(&memory, base)
    })?;
    let mut fuel_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_fuel_awards::fuel_awards::Hooks::new(&memory, base)
        })
    })?;
    let operation_outcomes = config.operation_outcomes
        && collector_extended_support::outcomes::records_any(
            session.capture_mode,
            &collector_operation_outcomes::extended::operation_outcomes::TABLES,
        );
    let mut operation_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_operation_outcomes::operation_outcomes::Hooks::new(
                &memory,
                base,
                operation_outcomes,
            )
        })
    })?;
    let mut politics_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_politics::politics_changes::Hooks::new(&memory, base, config.politics_capture)
        })
    })?;
    progress.step("hooks_prepared", None);
    let slots = collector_capture::country_scale::observe(&memory, base).unwrap_or(0);
    collector_primitives::country_slots::note_slots(slots);
    collector_observers::wargoal_changes::note_countries(slots);
    collector_observers::wargoal_changes::presize();
    let _ = collector_weather::weather::observe_display_capacity(&memory, base);
    let mut event_feed_hooks = FamilyHooks::construct(base, || unsafe {
        collector_observers::event_feed_observer::Hooks::new(
            &memory,
            base,
            event_feed_sequence,
            event_feed_counts,
            slots,
            collector_extended_support::outcomes::records_any(
                session.capture_mode,
                &[replay_data::rows::TableId::EventFeedHistory],
            ),
        )
    })?;
    // SAFETY: Runtime attach owns the authenticated image and sizes politics staging before capture.
    unsafe {
        collector_politics::politics_changes::resize(
            collector_capture::country_scale::observe_politics_demand(
                &memory,
                base,
                slots,
                1,
            )
            .unwrap_or(slots)
            .min(8192),
            processor
                .observe_input_demand(engine_events::politics_sites::lexical_key_demand(
                    &memory, base,
                ))
                .unwrap_or(0),
        );
    }
    collector_observers::country_input_changes::presize_countries(slots);
    collector_air_relations::relation_changes::presize(slots);
    collector_observers::world_input_changes::presize(slots);
    collector_air_histories::history_changes::presize(slots, strategic_regions(&memory, base));
    let mut sizing = collector_capture::country_scale::Sizing::for_slots(slots);
    // SAFETY: Attach or the isolated fixture owns this authenticated census and its observer
    // demand.
    match unsafe { collector_capture::country_scale::source_demand::Demand::observe(&memory, base) }
    {
        Ok(Some(demand)) => demand.apply(&mut sizing),
        Ok(None) => {}
        Err(reason) => processor.observe_input_demand_error(reason),
    }
    collector_observers::occupation_changes::demand(sizing.core[3].saturating_mul(9));
    collector_observers::occupation_changes::prepare();
    if let Err(reason) =
        // SAFETY: The serialized sizing pass or isolated fixture owns the observer storage throughout.
        unsafe {
            collector_capture::country_scale::table_demand::presize(
                &memory,
                base,
                &mut sizing.map,
                &mut sizing.extended,
            )
        }
    {
        processor.observe_input_demand_error(reason);
    }
    if let Err(reason) =
        // SAFETY: The sizing owner prepares the session's live storage before callbacks or in its
        // sizing pass.
        unsafe { sizing.extended.presize_live_inputs(&memory, base) }
    {
        processor.observe_input_demand_error(reason);
    }
    // SAFETY: Runtime attach installs initial land-event storage before callback publication.
    unsafe { collector_land_events::land_events::install_offered() };
    sizing.extended.logistics.raise(logistics_rows);
    collector_fuel_awards::fuel_awards::presize(&memory, base);
    collector_fuel_awards::fuel_awards::install_offered();
    let (entries, names) = collector_fuel_awards::fuel_awards::drained_capacities();
    sizing.extended.fuel_awards = collector_fuel_awards::tables::Capacities { entries, names };
    sizing.extended.game_score.enabled = config.game_score;
    if config.game_score
        && let Some(observed) = collector_air::sizing::observe_game_score(&memory, base)
            .map_err(|error| error_reason(error, "game score sizing allocation failed"))?
    {
        sizing.extended.game_score.raise(observed.demand);
    }
    let _ = sizing.extended.combat_progress.presize(&memory, base);
    collector_experience_gains::experience_gains::prepare(slots);
    sizing.extended.equipment_destruction =
        collector_equipment_destruction::equipment_destruction::demand();
    collector_naval_results::naval_result_sources::prepare(&memory, base);
    sizing.extended.naval_result_sources = collector_naval_results::naval_result_sources::demand();
    // SAFETY: Runtime attach prepares live observer storage against the authenticated image before
    // callback publication.
    unsafe {
        collector_observers::air_losses::prepare(&memory, base, slots);
        collector_observers::building_bombing::prepare(&memory, base, slots);
        collector_observers::combat_engagement_refreshes::presize(slots.max(64));
    }
    collector_observers::nuclear_strikes::presize(slots.max(64));
    // SAFETY: Runtime attach presizes map-event lanes before callback publication.
    unsafe { collector_map_events::presize(slots) }?;
    sizing.extended.air_losses.events = collector_observers::air_losses::capacity();
    sizing.extended.building_bombing = collector_observers::building_bombing::capacity();
    sizing.extended.combat_engagement_refreshes =
        collector_observers::combat_engagement_refreshes::capacity();
    sizing.extended.nuclear_strikes = collector_observers::nuclear_strikes::capacity();
    // SAFETY: Runtime attach presizes market observer storage before callback publication.
    unsafe { collector_observers::market_changes::presize(slots, slots) };
    sizing.extended.market = collector_observers::market_changes::capacity();
    sizing.content = engine_dlc::cdlc_manager::observe_counts(&memory, base);
    if let Some(rows) = processor
        .observe_input_demand(collector_capture::country_scale::observe_country_intel(
            &memory, base,
        ))
        .flatten()
    {
        sizing.extended.raise_for_country_intel(rows);
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_air::sizing::observe_ace_names(&memory, base))
        .flatten()
    {
        sizing.extended.air.raise_for_names(demand);
    }
    if let Some(bytes) = processor
        .observe_input_demand(collector_air::sizing::observe_air_wing_names(&memory, base))
        .flatten()
    {
        collector_air::sizing::note_air_wing_name_bytes(bytes);
        // SAFETY: The sizing pass uses this session's measured name demand.
        unsafe { sizing.map.raise_for_air_wing_names(bytes) };
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_capture::country_scale::observe_railway_gun_names(
            &memory, base,
        ))
        .flatten()
    {
        collector_railway_guns::railway_guns::note_railway_gun_names(demand);
        sizing.extended.railway_guns.raise_for_names(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_capture::country_scale::observe_map(&memory, base))
        .flatten()
    {
        for reason in demand.map_refusals() {
            processor.observe_input_demand_error(reason);
        }
        if let Some(reason) = demand.navies.refusal() {
            processor.observe_input_demand_error(reason);
        }
        if let Ok(carrier) = demand.carrier {
            collector_air::sizing::note_carrier_demand(carrier);
        }
        sizing.raise_for_map(&demand);
        sizing
            .extended
            .country_counters
            .raise_for_physical_navies(demand.navies.capacities);
        // SAFETY: This attachment's sizing pass owns the live observer demand.
        unsafe { sizing.extended.naval_counters_mut() }
            .raise_for_names(demand.navies.name_rows, demand.navies.name_bytes);
        // SAFETY: This attachment's sizing pass owns the live observer demand.
        unsafe { sizing.extended.naval_counters_mut() }
            .raise_for_supply_origins(demand.navies.supply_origins);
        let _ = processor.observe_input_demand(
            // SAFETY: This attachment's sizing pass owns the live observer demand.
            unsafe { sizing.extended.naval_counters_mut() }.raise_for_port(demand.navies.port),
        );
    }
    if let Some(demand) = processor
        .observe_input_demand(
            collector_capture::country_scale::observe_presentation_census(&memory, base),
        )
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
        .observe_input_demand(collector_capture::country_scale::observe_raid_arenas(
            &memory, base,
        ))
        .flatten()
    {
        for reason in demand.failures.into_iter().flatten() {
            processor.observe_input_demand_error(reason);
        }
        sizing.extended.raise_for_raid_arenas(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_capture::country_scale::observe_saved_targets(
            &memory, base,
        ))
        .flatten()
    {
        sizing.extended.saved_event_targets.raise(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_capture::country_scale::observe_characters(
            &memory, base,
        ))
        .flatten()
    {
        sizing.extended.raise_for_characters(demand);
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_capture::country_scale::observe_politics(
            &memory, base,
        ))
        .flatten()
    {
        sizing
            .extended
            .country_counters
            .raise_for_pride_names(demand.pride_name_bytes);
        for reason in demand.failures() {
            processor.observe_input_demand_error(reason);
        }
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe { sizing.extended.raise_for_politics(demand) };
    }
    if let Some(slots) = processor
        .observe_input_demand(
            collector_capture::country_scale::observe_reserved_visibility(&memory, base),
        )
        .flatten()
    {
        sizing.extended.raise_reserved_visibility(slots);
    }
    if let Some(demand) = processor
        .observe_input_demand(collector_capture::country_scale::observe_fuel_unit_demand(
            &memory, base,
        ))
        .flatten()
    {
        // SAFETY: This attachment's sizing pass owns the live observer demand.
        unsafe { sizing.extended.economy_mut() }
            .demand
            .raise(demand);
    }
    // SAFETY: Runtime attach prepares operation-log storage before callback publication.
    unsafe { sizing.extended.operation_outcomes.prepare() };
    progress.step("inputs_sized", None);
    let combat_hints = CombatHints::observe(&memory, base);
    for reason in combat_hints.failures.into_iter().flatten() {
        processor.observe_input_demand_error(reason);
    }
    sizing.extended.line_output.entries = sizing.extended.line_output.entries.max(events);
    let peace_capacity = collector_peace::peace_settlements::capacity_for_world(&memory, base);
    sizing.extended.peace_settlements = peace_capacity;
    sizing.extended.peace_bids = peace_capacity;
    let mut peace_hooks = FamilyHooks::construct(base, || {
        Ok::<_, &'static str>(unsafe {
            collector_peace::peace_settlements::Hooks::new(&memory, base, peace_capacity)
        })
    })?;
    let pe_offset = read::<u32>(&memory, module as usize + 0x3c).unwrap_or(0) as usize;
    let image_size = read::<u32>(&memory, module as usize + pe_offset + 0x50).unwrap_or(0) as usize;
    let [relay, trampoline] = hook.executable_ranges();
    let mut ranges = vec![
        module as usize..module as usize + image_size,
        relay,
        trampoline,
    ];
    ranges.extend(batch_hooks.executable_ranges());
    ranges.extend(portrait_hooks.executable_ranges());
    ranges.extend(advisor_hooks.executable_ranges());
    ranges.extend(relation_hooks.executable_ranges());
    ranges.extend(history_hooks.executable_ranges());
    ranges.extend(flag_hooks.executable_ranges());
    ranges.extend(propaganda_hooks.executable_ranges());
    ranges.extend(intel_source_hooks.executable_ranges());
    ranges.extend(network_auxiliary_hooks.executable_ranges());
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
    ranges.extend(naval_management_hooks.executable_ranges());
    ranges.extend(census_hooks.executable_ranges());
    ranges.extend(world_input_hooks.executable_ranges());
    ranges.extend(air_record_hooks.executable_ranges());
    ranges.extend(participation_hooks.executable_ranges());
    ranges.extend(local_subunit_hooks.executable_ranges());
    ranges.extend(design_hooks.executable_ranges());
    ranges.extend(template_hooks.executable_ranges());
    ranges.extend(orders_group_hooks.executable_ranges());
    ranges.extend(membership_hooks.executable_ranges());
    ranges.extend(faction_goal_hooks.executable_ranges());
    ranges.extend(faction_companion_hooks.executable_ranges());
    ranges.extend(ship_model_hooks.executable_ranges());
    ranges.extend(province_unit_hooks.executable_ranges());
    ranges.extend(province_combat_hooks.executable_ranges());
    ranges.extend(air_base_carry_hooks.executable_ranges());
    ranges.extend(opinion_hooks.executable_ranges());
    ranges.extend(destruction_hooks.executable_ranges());
    ranges.extend(naval_result_hooks.executable_ranges());
    ranges.extend(experience_hooks.executable_ranges());
    ranges.extend(command_hooks.executable_ranges());
    ranges.extend(air_loss_hooks.executable_ranges());
    ranges.extend(bombing_hooks.executable_ranges());
    ranges.extend(engagement_refresh_hooks.executable_ranges());
    ranges.extend(nuclear_hooks.executable_ranges());
    ranges.extend(market_hooks.executable_ranges());
    ranges.extend(map_event_hooks.executable_ranges());
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
    let mut shared =
    // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
    // boundary or detach.
    unsafe { Shared::with_sizing(base, hook.trampoline(), frequency as u64, &sizing) }
        .map_err(|e| error_reason(e, "capture buffers allocation failed"))?;
    shared.capture_mode = session.capture_mode;

    // SAFETY: Attach initializes the idle event before publishing Shared.
    *unsafe { shared.tick_idle_mut() } = Some(idle);
    // SAFETY: This is the unpublished attachment's own roster.
    unsafe { shared.prepare_roster(&memory) };
    // SAFETY: This is unpublished attach state or the isolated fixture's private capture clock.
    unsafe { shared.prepare_clock_capture(&memory) };
    let shared = engine_allocation::shared(shared)
        .map_err(|e| error_reason(e, "collector state allocation failed"))?;
    processor.account_writer(
        // SAFETY: The writer accounts memory to this same session and drains before it is released.
        unsafe { shared.pool_bytes() }.clone(),
    );
    progress.step("shared_started", None);
    let mut spool_state = SpoolState {
        sizing,
        applied_combat_hints: AppliedHints::default(),
    };
    grow_combat_hints(
        &shared,
        &spool_state.sizing.extended,
        &combat_hints,
        &mut spool_state.applied_combat_hints,
    );
    let acquisition =
    // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
    unsafe { acquisition::Pool::start(
        collector_workers::acquisition::recommended_worker_count(),
        collector_workers::roles::MAX_JOBS,
    ) }?;

    // SAFETY: The writer accounts memory to this same session and drains before it is released.
    unsafe { shared.pool_bytes() }
        .fetch_add(acquisition.allocated_bytes() as u64, Ordering::Relaxed);

    // SAFETY: The runtime owns this attachment's worker pool from startup through join.
    unsafe { shared.acquisition() }
        .set(acquisition)
        .map_err(|_| "acquisition pool already initialized")?;
    let _payload_pools =
    // SAFETY: Attach owns payload storage through worker join. Fixture processes own their test
    // pools.
    unsafe { decode::prepare_payload_pools(config.game_score) }
        .map_err(|e| error_reason(e, "payload pools allocation failed"))?;
    if config.cancel_file.exists() || detach.signaled() {
        return Err("recording cancelled before hook installation");
    }
    if collector_memory::terminal::failed() {
        return Err("collector allocation failed before hooks");
    }
    let acquisition =
        // SAFETY: The runtime owns this attachment's worker pool from startup through join.
        unsafe { shared.acquisition() }.get().expect("initialized before hook");
    let mut owned_workers = acquisition.thread_ids();
    let mut priority_reconciler =
    // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
    unsafe { collector_workers::frame_priority::Reconciler::start(
        // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
         shared.frame_priority() .clone(),
    ) }?;
    owned_workers.push(priority_reconciler.thread_id());
    processor.attach_priority(
        // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
        unsafe { shared.frame_priority() }.clone(),
    );
    owned_workers.extend(processor.writer_thread_ids().into_iter().flatten());
    let (mut history_worker, history_handoff) = HistoryWorker::start(
        config.output_dir.clone(),
        config.replay_dir.clone(),
        Some(|| {
            // SAFETY: The session's history worker invokes this before the capture state is retired.
            unsafe { collector_capture::capture_point::settle_next() }
        }),
    )
    .map_err(|e| error_reason(e, "history worker could not start"))?;
    owned_workers.push(history_worker.thread_id());
    for index in 0..collector_workers::spool_helper::pool_size() {
        let Ok(helper) =
        // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
        (unsafe { collector_workers::spool_helper::SpoolHelper::start(Some((
            // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
             shared.frame_priority() .clone(),
            collector_workers::frame_priority::Role::Helper(index),
        ))) }) else {
            break;
        };
        let id = helper.thread_id();
        if !processor.set_helper(helper) {
            break;
        }
        owned_workers.push(id);
        // SAFETY: This is the live, owned helper at its assigned runtime index.
        unsafe { collector_workers::thread_cpu::register_helper(index, id) };
    }
    let sizing = SizingThread::start(shared.clone(), spool_state);
    if let Some(id) = sizing.thread_id() {
        owned_workers.push(id);
        // SAFETY: This updates the designated role of this runtime's owned worker.
        unsafe {
            collector_workers::thread_cpu::register_id(
                collector_workers::thread_cpu::Role::SpoolSizing,
                id,
            )
        };
    }
    if let Some(id) = sizing.builder_id() {
        owned_workers.push(id);
        // SAFETY: This updates the designated role of this runtime's owned worker.
        unsafe {
            collector_workers::thread_cpu::register_id(
                collector_workers::thread_cpu::Role::FrameBuilder,
                id,
            )
        };
    }
    let spool_id = Arc::new(AtomicU32::new(0));
    let spool_thread_id = Arc::clone(&spool_id);
    let spool_shared = shared.clone();
    let spool_status = status_path.clone();
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
            collector_workers::private_heap::off_hold_path();
            spool_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            // SAFETY: Only the current session's designated thread registers this role.
            unsafe {
                collector_workers::thread_cpu::register(collector_workers::thread_cpu::Role::Spool)
            };
            spool_loop(
                &spool_shared,
                &config,
                session,
                processor,
                &spool_status,
                sizing,
                spool_writer,
                &spool_teardown_status,
                history_handoff,
            )
        })
        .map_err(|e| error_reason(e, "collector bootstrap I/O failed"))?;
    let decode_panic_hook =
    // SAFETY: The runtime owns the process hook until all workers exit, or the fixture owns it in
    // isolation.
    unsafe { collector_workers::panic_failure::HookGuard::install() };
    let mut installation_error =
    // SAFETY: Attach starts decode once and retains its pools through detach.
    unsafe { decode::start_worker() }.err();
    while spool_id.load(Ordering::Acquire) == 0 && !spool.is_finished() {
        std::thread::yield_now();
    }
    owned_workers.extend([
        spool_id.load(Ordering::Acquire),
        collector_workers::thread_cpu::decode_worker(),
    ]);
    let mut capture_thread =
        // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
        unsafe { collector_capture::capture_point::Thread::start(BACKGROUND_STACK_BYTES) }.ok();
    owned_workers.extend(capture_thread.as_ref().map(|thread| thread.id()));
    collector_hook_failures::workers_started();
    progress.step("workers_started", None);
    #[cfg(feature = "worker_sampler")]
    let mut sampler = match
    // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
    unsafe { collector_workers::worker_sampler::start(
        sampler_config.as_deref(),
        &sampler_output,
        acquisition.thread_ids(),
    ) } {
        Ok(sampler) => {
            let id = collector_workers::worker_sampler::thread_id();
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
    let restoration = match collector_hooks::hook::restoration::Scratch::prepare(ranges.len()) {
        Ok(scratch) => Some(scratch),
        Err(error) => {
            installation_error.get_or_insert(error);
            None
        }
    };
    if collector_memory::terminal::failed() {
        installation_error = Some("collector allocation failed before hooks");
    }
    unsafe { SHARED.publish(&shared) };
    let mut deadline = Instant::now() + Duration::from_secs(15);
    let stage = crate::progress::Stage::new(&progress, "world_lifetime");
    while installation_error.is_none() && !collector_memory::terminal::failed() {
        stage.set("world_lifetime");
        match (|| {
            install_attach_batch(
                &stage,
                &mut [
                    ("world_lifetime", &mut || world_hooks.set_enabled(true)),
                    ("fuel_awards", &mut || fuel_hooks.set_enabled(true)),
                    ("advisor_changes", &mut || advisor_hooks.set_enabled(true)),
                    ("portrait_changes", &mut || portrait_hooks.set_enabled(true)),
                    ("relation_changes", &mut || relation_hooks.set_enabled(true)),
                    ("history_changes", &mut || history_hooks.set_enabled(true)),
                    ("flag_changes", &mut || flag_hooks.set_enabled(true)),
                    ("network_auxiliary", &mut || {
                        network_auxiliary_hooks.set_enabled(true)
                    }),
                    ("intel_sources", &mut || {
                        intel_source_hooks.set_enabled(true)
                    }),
                    ("propaganda_changes", &mut || {
                        propaganda_hooks.set_enabled(true)
                    }),
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
                    ("naval_management_changes", &mut || {
                        naval_management_hooks.set_enabled(true)
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
                    ("province_combat_changes", &mut || {
                        province_combat_hooks.set_enabled(true)
                    }),
                    ("war_participation", &mut || {
                        participation_hooks.set_enabled(true)
                    }),
                    ("air_record_changes", &mut || {
                        air_record_hooks.set_enabled(true)
                    }),
                    ("variant_changes", &mut || design_hooks.set_enabled(true)),
                    ("template_changes", &mut || template_hooks.set_enabled(true)),
                    ("order_membership_changes", &mut || {
                        membership_hooks.set_enabled(true)
                    }),
                    ("orders_group_changes", &mut || {
                        orders_group_hooks.set_enabled(true)
                    }),
                    ("faction_goals", &mut || {
                        faction_goal_hooks.set_enabled(true)
                    }),
                    ("faction_companions", &mut || {
                        faction_companion_hooks.set_enabled(true)
                    }),
                    ("ship_model_changes", &mut || {
                        ship_model_hooks.set_enabled(true)
                    }),
                    ("air_base_carry_changes", &mut || {
                        air_base_carry_hooks.set_enabled(true)
                    }),
                    ("opinion_changes", &mut || opinion_hooks.set_enabled(true)),
                    ("line_output", &mut || line_hooks.set_enabled(true)),
                    ("peace_settlements", &mut || peace_hooks.set_enabled(true)),
                    ("land_events", &mut || land_hooks.set_enabled(true)),
                    ("construction_ic", &mut || {
                        construction_hooks.set_enabled(true)
                    }),
                    ("logistics_balance", &mut || {
                        logistics_hooks.set_enabled(true)
                    }),
                    ("combat_log_prune", &mut || prune_hooks.set_enabled(true)),
                    ("equipment_destruction", &mut || {
                        destruction_hooks.set_enabled(true)
                    }),
                    ("event_feed_observer", &mut || {
                        event_feed_hooks.set_enabled(true)
                    }),
                    ("experience_gains", &mut || {
                        experience_hooks.set_enabled(true)
                    }),
                    ("player_commands", &mut || command_hooks.set_enabled(true)),
                    ("operation_outcomes", &mut || {
                        operation_hooks.set_enabled(true)
                    }),
                    ("politics_changes", &mut || politics_hooks.set_enabled(true)),
                    ("naval_result_sources", &mut || {
                        naval_result_hooks.set_enabled(true)
                    }),
                    ("air_losses", &mut || air_loss_hooks.set_enabled(true)),
                    ("building_bombing", &mut || bombing_hooks.set_enabled(true)),
                    ("combat_engagement_refreshes", &mut || {
                        engagement_refresh_hooks.set_enabled(true)
                    }),
                    ("nuclear_strikes", &mut || nuclear_hooks.set_enabled(true)),
                    ("market_changes", &mut || market_hooks.set_enabled(true)),
                    ("map_events", &mut || map_event_hooks.set_enabled(true)),
                    ("command_batch", &mut || batch_hooks.set_enabled(true)),
                    ("hourly", &mut || {
                        hook.set_enabled(true).map_err(installation_message)
                    }),
                ],
            )
        })() {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
            Ok(false) => {
                let busy_sites = match stage.get() {
                    "faction_goals" => faction_goal_hooks.busy_sites(),
                    "faction_companions" => faction_companion_hooks.busy_sites(),
                    _ => Vec::new(),
                };
                let reason = installation_deadline_message(stage.get(), &busy_sites);
                let family = RestoreFamily::ALL
                    .into_iter()
                    .find(|family| family.name() == stage.get())
                    .unwrap();
                if collector_hook_failures::failed(family) {
                    installation_error = Some(installation_message(format!(
                        "{} rollback failed: {reason}",
                        family.name()
                    )));
                    break;
                }
                match collector_hook_failures::refuse(
                    crate::hook_inventory::patches,
                    family,
                    "publication deadline",
                    &reason,
                ) {
                    Ok(()) => deadline = Instant::now() + Duration::from_secs(15),
                    Err(_) => {
                        installation_error =
                            Some(installation_message(format!("{}: {reason}", stage.get())));
                        break;
                    }
                }
            }
            Err(reason) => {
                let family = RestoreFamily::ALL
                    .into_iter()
                    .find(|family| family.name() == stage.get())
                    .unwrap();
                match collector_hook_failures::install_failed(
                    crate::hook_inventory::patches,
                    family,
                    reason,
                    installation_message,
                ) {
                    Ok(()) => deadline = Instant::now() + Duration::from_secs(15),
                    Err(error) => {
                        installation_error = Some(
                            collector_hook_failures::fatal_installation_error().unwrap_or(error),
                        );
                        break;
                    }
                }
            }
        }
    }
    stage.finish();
    #[cfg(feature = "worker_sampler")]
    if installation_error.is_none()
        && !collector_memory::terminal::failed()
        && let Some(sampler) = sampler.as_mut()
        && let Err(error) =
            // SAFETY: Attach activates its sole sampler after the target workers are ready.
            unsafe { sampler.activate() }
    {
        installation_error = Some(error);
    }
    if installation_error.is_none() && !collector_memory::terminal::failed() {
        // SAFETY: Attach has published Shared and is opening admission for this session.
        unsafe { HOURLY_ADMISSION.open() };
        // SAFETY: Attach selects the capture mode before hourly hook publication.
        unsafe {
            collector_capture::capture_point::set_beside(
                capture_thread.is_some() && collector_capture::command_batch::enabled(),
            )
        };
        progress.step("ready", None);
        // SAFETY: The tick exit owns the session's idle event and sets it after leaving the gate.
        unsafe { ready.set() };
    } else {
        progress.step("installation_refused", None);
    }
    while installation_error.is_none()
        && !detach.signaled()
        && !spool.is_finished()
        && !decode::worker_finished()
        && !priority_reconciler.is_finished()
        && !shared.acquisition_failed()
        && !collector_memory::terminal::failed()
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _teardown = collector_memory::terminal::enter(collector_memory::terminal::Phase::Teardown);
    let _restoration = restoration.map(collector_hooks::hook::restoration::Scratch::enter);
    collector_memory::terminal::attempted_hour(None);
    let decode_stopped_early = installation_error.is_none() && decode::worker_finished();
    let detach_started = Instant::now();
    progress.step("detach_requested", None);
    #[cfg(feature = "worker_sampler")]
    if let Some(sampler) = sampler.as_mut() {
        let id = collector_workers::worker_sampler::thread_id();
        let samples =
        // SAFETY: This is the owning runtime's shutdown, while the worker's shared state is still
        // live.
        unsafe { sampler.stop() };
        if id != 0 {
            owned_workers.retain(|worker| *worker != id);
        }
        progress.step("sampler_stopped", Some(samples));
    }
    macro_rules! retain_detach {
        ($reason:expr, $fault:expr, $workers:expr) => {{
            let reason: &'static str = $reason;
            let fault: Option<&'static str> = $fault;
            let error = fault
                .or_else(collector_workers::panic_failure::first)
                .or_else(collector_workers::worker_panic::first)
                .unwrap_or(reason);
            refuse_detach!(error, |_error| {

                    // SAFETY: This is the owning runtime's stop or an isolated fixture's recording flag.
                    unsafe { shared.recording() }.store(false, Ordering::Release);
                status_writer.finish();
                if !collector_memory::terminal::failed() {
                    let status = collector_status::status::status!({
                        "state":"detach_refused", "session":session_id,
                        "drain_complete":false, "error":error,
                        "build":collector_primitives::build_gate::status(),
                    });
                    #[cfg(feature = "worker_sampler")]
                    let status = status.join(collector_status::status::status!({
                        "worker_sampler":collector_workers::worker_sampler::status(),
                    }));
                    let _ = write_status(&status_path, &status);
                    let _ = fs::write(dll_path.with_file_name("collector.error.txt"), error);
                }
                std::mem::forget((
                    $workers, capture_thread, hook, batch_hooks, portrait_hooks, advisor_hooks, relation_hooks,
                    history_hooks, flag_hooks, network_auxiliary_hooks, intel_source_hooks, propaganda_hooks, building_hooks, advisor_pool_hooks, party_leader_hooks, variable_hooks, party_hooks, wing_hooks, roster_hooks, political_status_hooks, mission_hooks, rail_hooks, territory_hooks, army_stat_hooks, country_input_hooks, raid_hooks, naval_tree_hooks, air_record_hooks, participation_hooks, world_input_hooks, opinion_hooks, destruction_hooks, naval_result_hooks, local_subunit_hooks, design_hooks, template_hooks, orders_group_hooks, membership_hooks, faction_goal_hooks, faction_companion_hooks, ship_model_hooks, province_unit_hooks, province_combat_hooks, air_base_carry_hooks, unit_leader_hooks, naval_management_hooks, census_hooks,
                    air_loss_hooks, bombing_hooks, engagement_refresh_hooks, nuclear_hooks, market_hooks, map_event_hooks, line_hooks, land_hooks, prune_hooks,
                    construction_hooks, experience_hooks, command_hooks, event_feed_hooks, world_hooks,
                    fuel_hooks, operation_hooks, politics_hooks, logistics_hooks,
                    peace_hooks, shared, output_lock, _payload_pools, decode_panic_hook, worker_panic,
                ));
            });
        }};
    }

    let mut restored = [false; RestoreFamily::ALL.len()];
    let mut previous_errors = [false; RestoreFamily::ALL.len()];
    restore_hourly(
        &mut restored,
        &mut previous_errors,
        || detach_started.elapsed(),
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
        // SAFETY: The current runtime or its spool owns the session's terminal stop.
        unsafe { shared.recording() }.store(false, Ordering::Release);
    }
    let drained =
    // SAFETY: This is the detach drain after closing hourly admission, with capture state still
    // live.
    unsafe { collector_capture::capture_point::drain(Instant::now() + FINAL_HOUR_WAIT) };
    progress.step("final_hour_read", Some(u64::from(drained)));
    // SAFETY: Detach closes admission before waiting for admitted capture to complete.
    unsafe { HOURLY_ADMISSION.close() };
    if let Err(reason) = wait_acquisitions(&HOURLY_ADMISSION, || detach_started.elapsed()) {
        progress.step("acquisitions_refused", None);
        retain_detach!(
            reason,
            installation_error.or(Some(reason)),
            (spool, status_writer)
        );
    }
    if let Some(thread) = capture_thread.as_mut() {
        // SAFETY: This is the owning runtime's shutdown, while the worker's shared state is still
        // live.
        unsafe { thread.stop() };
        let id = thread.id();
        owned_workers.retain(|worker| *worker != id);
    }
    let unread =
    // SAFETY: Detach has drained capture and transfers the loss count to this session's spool.
    unsafe { collector_capture::capture_point::take_unread() };
    if unread != 0 {
        // SAFETY: no capture runs or can start.
        unsafe { sampling::record_lost(&shared, unread) };
        progress.step("unread_hours_lost", Some(u64::from(unread)));
    }
    progress.step("acquisitions_quiet", None);
    let mut first_restore_error = None;
    let mut restore_wait = RestoreWait::new(Instant::now());
    let restore_result = loop {
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
            RestoreFamily::CommandBatch => batch_hooks.set_enabled(false),
            RestoreFamily::Portrait => portrait_hooks.set_enabled(false),
            RestoreFamily::Opinion => opinion_hooks.set_enabled(false),
            RestoreFamily::Construction => construction_hooks.set_enabled(false),
            RestoreFamily::LineOutput => line_hooks.set_enabled(false),
            RestoreFamily::CombatLogPrune => prune_hooks.set_enabled(false),
            RestoreFamily::Advisor => advisor_hooks.set_enabled(false),
            RestoreFamily::Relation => relation_hooks.set_enabled(false),
            RestoreFamily::History => history_hooks.set_enabled(false),
            RestoreFamily::Flags => flag_hooks.set_enabled(false),
            RestoreFamily::Propaganda => propaganda_hooks.set_enabled(false),
            RestoreFamily::IntelSources => intel_source_hooks.set_enabled(false),
            RestoreFamily::NetworkAuxiliary => network_auxiliary_hooks.set_enabled(false),
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
            RestoreFamily::NavalManagement => naval_management_hooks.set_enabled(false),
            RestoreFamily::Census => census_hooks.set_enabled(false),
            RestoreFamily::WorldInputs => world_input_hooks.set_enabled(false),
            RestoreFamily::WarParticipation => participation_hooks.set_enabled(false),
            RestoreFamily::AirRecords => air_record_hooks.set_enabled(false),
            RestoreFamily::LocalSubunits => local_subunit_hooks.set_enabled(false),
            RestoreFamily::Designs => design_hooks.set_enabled(false),
            RestoreFamily::Templates => template_hooks.set_enabled(false),
            RestoreFamily::OrdersGroups => orders_group_hooks.set_enabled(false),
            RestoreFamily::OrderMembership => membership_hooks.set_enabled(false),
            RestoreFamily::FactionGoals => faction_goal_hooks.set_enabled(false),
            RestoreFamily::FactionCompanions => faction_companion_hooks.set_enabled(false),
            RestoreFamily::ShipModels => ship_model_hooks.set_enabled(false),
            RestoreFamily::ProvinceUnits => province_unit_hooks.set_enabled(false),
            RestoreFamily::ProvinceCombats => province_combat_hooks.set_enabled(false),
            RestoreFamily::AirBaseCarry => air_base_carry_hooks.set_enabled(false),
            RestoreFamily::Experience => experience_hooks.set_enabled(false),
            RestoreFamily::PlayerCommands => command_hooks.set_enabled(false),
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
            RestoreFamily::CombatEngagementRefreshes => engagement_refresh_hooks.set_enabled(false),
            RestoreFamily::NuclearStrikes => nuclear_hooks.set_enabled(false),
            RestoreFamily::Market => market_hooks.set_enabled(false),
            RestoreFamily::MapEvents => map_event_hooks.set_enabled(false),
            RestoreFamily::WorldLifetime => world_hooks.set_enabled(false),
        };
        let mut restore_round_shared = (!collector_hooks::hook::restoration::active())
            .then(collector_hooks::hook::shared::Round::collect)
            .flatten();
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
        let mut batch = collector_hooks::hook::restoration::Batch::collect();
        let mut batch_restored = 0;
        if let Some(batch) = batch.as_mut() {
            let world_ready = world_restore_ready(&restored, observers_active());
            for family in RestoreFamily::ALL {
                if !restored[family as usize]
                    && (family != RestoreFamily::WorldLifetime || world_ready)
                {
                    let _ = toggle(family);
                }
            }
            let begin = collector_primitives::clock::qpc();
            batch_restored = batch.publish();
            progress.step(
                format_args!("restore_batch {batch_restored} publications"),
                Some(ticks_to_micros(
                    collector_primitives::clock::qpc() - begin,
                    frequency,
                )),
            );
        }
        let restore = |family| {
            let (result, refusal) = collector_hooks::hook::restoration::attempt(|| toggle(family));
            progress.step(
                format_args!(
                    "restore {family:?} {}",
                    match result {
                        Ok(true) => "done",
                        Ok(false) => "busy",
                        Err(_) => "refused",
                    }
                ),
                collector_hooks::hook::shared::take_reported_ticks()
                    .map(|ticks| ticks_to_micros(ticks, frequency)),
            );
            if let Some(refusal) = refusal {
                progress.step(format_args!("restore {family:?}: {refusal}"), None);
            }
            if let Err(error) = result {
                progress.step(format_args!("restore {family:?}: {error}"), None);
                first_restore_error.get_or_insert(error);
            }
            result
        };
        let result = restore_wait.round(
            &mut restored,
            &mut previous_errors,
            observers_active,
            restore,
            Instant::now,
        );
        drop(restore_round_shared);
        drop(batch);
        match result {
            Ok(RestoreRound::Complete) => break Ok(()),
            Ok(RestoreRound::Retry(_)) if batch_restored != 0 => {}
            Ok(RestoreRound::Retry(delay)) => std::thread::sleep(delay),
            Ok(RestoreRound::Stalled) => {
                break Err(first_restore_error.unwrap_or_else(|| {
                    mutation_detach_refusal(detach_started.elapsed(), |family| {
                        family.observer_state().0
                    })
                }));
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
        // SAFETY: The current runtime or its spool owns the session's terminal stop.
        unsafe { shared.recording() }.store(false, Ordering::Release);
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
            Instant::now(),
        )
    {
        progress.step("observers_refused", None);
        retain_detach!(
            reason,
            installation_error.or(Some(reason)),
            (spool, status_writer)
        );
    }
    progress.step(
        if hooks.restored {
            "observers_quiet"
        } else {
            "observers_retained"
        },
        None,
    );

    // SAFETY: The current runtime or its spool owns the session's terminal stop.
    unsafe { shared.recording() }.store(false, Ordering::Release);
    if let Err(reason) =
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { acquisition.shutdown() }
    {
        progress.step("acquisition_refused", None);
        retain_detach!(
            reason,
            installation_error.or(Some(reason)),
            (spool, status_writer)
        );
    }
    progress.step("acquisition_joined", None);
    let mut logged = Instant::now();
    let decode_outcome =
    // SAFETY: Hourly admission is closed and capture is quiet before this decode drain.
    unsafe { decode::detach_drain(|count| {
        if logged.elapsed() >= Duration::from_secs(1) {
            logged = Instant::now();
            progress.step("decode_draining", Some(count));
        }
    }) };
    progress.step("decode_drained", None);

    // SAFETY: The runtime and sole spool coordinate this session's drain and exit.
    unsafe { shared.stop_spool() }.store(true, Ordering::Release);
    shared.wake_spool();
    let mut pending = usize::MAX;
    while !spool.is_finished() {
        let ready =
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }.ready_len();
        if ready != pending {
            pending = ready;
            progress.step("spool_draining", Some(ready as u64));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    progress.step("spool_finished", None);
    let spool_outcome = spool
        .join()
        .map_err(|_| collector_workers::worker_panic::first().unwrap_or("spool thread failed"))
        .and_then(|result| result);
    priority_reconciler.finish();
    if history_worker.finish().is_err() {
        progress.step("history_release_acknowledgement_failed", None);
    }
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
    if
    // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
    unsafe { shared.frame_priority() }.failed() {
        add_terminal_error(&mut spool_result, "frame priority reconciler failed");
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
            Instant::now(),
        )
    {
        progress.step("release_refused", None);
        retain_detach!(reason, spool_result.as_ref().err().copied(), ());
    }
    progress.step(
        if hooks.restored {
            "release_quiet"
        } else {
            "release_retained"
        },
        None,
    );
    if hooks.restored {
        if let Some(hooks) = experience_hooks.inner.as_mut() {
            hooks.release_after_quiet();
        }
        if let Some(hooks) = command_hooks.inner.as_mut() {
            // SAFETY: The owning hook handle has no published taps or live callbacks here.
            unsafe { hooks.release_after_quiet() };
        }
        let _ = match event_feed_hooks
            .inner
            .as_mut()
            .map_or(Ok(Default::default()), |hooks| hooks.release_after_quiet())
        {
            Ok(losses) => losses,
            Err(reason) => retain_detach!(reason, spool_result.as_ref().err().copied(), ()),
        };
        unsafe {
            collector_logistics::logistics_balance::release();
            collector_operation_outcomes::operation_outcomes::release();
            collector_politics::politics_changes::release();
            collector_map_events::lanes::release();
        }
        // SAFETY: Detach has disabled hooks and quiesced callbacks before releasing observer storage.
        if let Err(reason) = unsafe { collector_land_events::land_events::release() }
            .and_then(|()| unsafe { collector_combat_log::history_events::release_storage() })
            .and_then(|()| unsafe { collector_combat_log::combat_log_prune::release_storage() })
            .and_then(|()| collector_equipment_destruction::equipment_destruction::release())
            .and_then(|()| collector_naval_results::naval_result_sources::release())
            .and_then(|()| collector_fuel_awards::fuel_awards::release())
        {
            retain_detach!(reason, spool_result.as_ref().err().copied(), ());
        }
    }
    hooks.release(
        (
            hook,
            batch_hooks,
            portrait_hooks,
            advisor_hooks,
            relation_hooks,
            history_hooks,
            flag_hooks,
            propaganda_hooks,
            intel_source_hooks,
            network_auxiliary_hooks,
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
            naval_management_hooks,
            census_hooks,
            world_input_hooks,
            air_record_hooks,
            participation_hooks,
            local_subunit_hooks,
            design_hooks,
            template_hooks,
            orders_group_hooks,
            membership_hooks,
            faction_goal_hooks,
            faction_companion_hooks,
            ship_model_hooks,
            province_unit_hooks,
            province_combat_hooks,
            air_base_carry_hooks,
            opinion_hooks,
            destruction_hooks,
            naval_result_hooks,
            air_loss_hooks,
            bombing_hooks,
            engagement_refresh_hooks,
            nuclear_hooks,
            market_hooks,
            map_event_hooks,
            line_hooks,
            land_hooks,
            prune_hooks,
            construction_hooks,
            experience_hooks,
            command_hooks,
            event_feed_hooks,
            world_hooks,
            fuel_hooks,
            operation_hooks,
            politics_hooks,
            logistics_hooks,
            peace_hooks,
            shared,
            _payload_pools,
            decode_panic_hook,
            worker_panic,
        ),
        || {},
    );
    progress.step(
        if restore_result.is_ok() {
            "released"
        } else {
            "retained"
        },
        None,
    );
    drop(_restoration);
    let drain_complete = spool_result.is_ok();
    if let Some(error) = installation_error {
        add_terminal_error(&mut spool_result, error);
    }
    if !collector_memory::terminal::failed() {
        let pinned = restore_result.is_err();
        let status = collector_status::status::status!({
            "state":if pinned { "pinned" } else { "detached" },"session":session_id,
            "drain_complete":drain_complete,
            "error":spool_result.as_ref().err().copied(),
            "warning":restore_result.err(),
            "build":collector_primitives::build_gate::status(),
            "restoration_error":restore_result.err(),
        });
        #[cfg(feature = "worker_sampler")]
        let status = status.join(collector_status::status::status!({
            "worker_sampler":collector_workers::worker_sampler::status(),
        }));
        let write = if drain_complete {
            if let Some((mut bytes, directories)) = terminal_status {
                crate::status_writer::measure_bytes(&mut bytes, &directories)
                    .and_then(|()| write_status_bytes(&status_path, &bytes))
            } else {
                write_status(&status_path, &status)
            }
        } else {
            write_status(&status_path, &status)
        };
        if let Err(error) = write {
            collector_memory::terminal::observe(&error);
            add_terminal_error(&mut spool_result, "terminal status write failed");
        }
    }
    #[cfg(feature = "worker_sampler")]
    if restore_result.is_ok() {
        // SAFETY: Detach has stopped and joined the workers that use this state.
        unsafe { collector_workers::worker_sampler::release() };
    }
    if let Err(error) = spool_result
        && !collector_memory::terminal::failed()
    {
        let _ = fs::write(dll_path.with_file_name("collector.error.txt"), error);
    }
    progress.step("done", None);
    drop(progress);
    *spool_final = true;
    signal_done(output_lock, &done);
    spool_result
}

use crate::hook_failures::FamilyHooks;
use collector_hook_failures::Family as RestoreFamily;

trait ObserverState {
    fn observer_state(self) -> (bool, &'static str);
    fn shared(self) -> bool;
}

impl ObserverState for RestoreFamily {
    fn observer_state(self) -> (bool, &'static str) {
        match self {
            Self::PlayerCommands => (
                collector_player_commands::player_commands::active() != 0,
                "command observers did not quiesce; collector retained",
            ),
            Self::Hourly => (
                sampling::active() != 0,
                "hourly callbacks did not quiesce; collector retained",
            ),
            Self::EquipmentDestruction => (
                collector_equipment_destruction::equipment_destruction::active() != 0,
                "equipment destruction callbacks did not quiesce; collector retained",
            ),
            Self::Land => (
                collector_land_events::land_events::active() != 0,
                "land event callbacks did not quiesce; collector retained",
            ),
            Self::CommandBatch => (
                collector_capture::command_batch::active() != 0,
                "command batch callbacks did not quiesce; collector retained",
            ),
            Self::Portrait => (
                collector_characters::portrait_changes::ACTIVE.load(Ordering::SeqCst) != 0,
                "portrait callbacks did not quiesce; collector retained",
            ),
            Self::Opinion => (
                collector_observers::opinion_changes::active() != 0,
                "opinion callbacks did not quiesce; collector retained",
            ),
            Self::Construction => (
                collector_output::construction_ic::active() != 0,
                "construction callbacks did not quiesce; collector retained",
            ),
            Self::LineOutput => (
                collector_output::line_output::active() != 0
                    || collector_output::line_output::native_retention_required(),
                "production output callbacks did not quiesce; collector retained",
            ),
            Self::CombatLogPrune => (
                collector_combat_log::combat_log_prune::active() != 0
                    || collector_combat_log::history_events::active() != 0
                    || collector_combat_log::history_events::retention_required(),
                "combat log prune callbacks did not quiesce; collector retained",
            ),
            Self::Advisor => (
                collector_characters::advisor_changes::ACTIVE.load(Ordering::SeqCst) != 0
                    || collector_observers::character_changes::active() != 0,
                "advisor callbacks did not quiesce; collector retained",
            ),
            Self::Relation => (
                collector_air_relations::relation_changes::active() != 0,
                "relation callbacks did not quiesce; collector retained",
            ),
            Self::History => (
                collector_air_histories::history_changes::active() != 0,
                "history callbacks did not quiesce; collector retained",
            ),
            Self::NetworkAuxiliary => (
                collector_intel::network_auxiliary::active() != 0,
                "network auxiliary callbacks did not quiesce; collector retained",
            ),
            Self::IntelSources => (
                collector_intel::source_changes::active() != 0,
                "intel source callbacks did not quiesce; collector retained",
            ),
            Self::Propaganda => (
                collector_intel::propaganda_changes::active() != 0,
                "propaganda callbacks did not quiesce; collector retained",
            ),
            Self::Flags => (
                collector_intel::flag_changes::active() != 0,
                "script flag callbacks did not quiesce; collector retained",
            ),
            Self::Buildings => (
                collector_construction::building_changes::active() != 0,
                "building status callbacks did not quiesce; collector retained",
            ),
            Self::AdvisorPools => (
                collector_observers::advisor_pool_changes::active() != 0,
                "advisor pool callbacks did not quiesce; collector retained",
            ),
            Self::PartyLeaders => (
                collector_governance::party_leader_changes::active() != 0,
                "party leader callbacks did not quiesce; collector retained",
            ),
            Self::Variables => (
                collector_intel::variable_changes::active() != 0,
                "script variable callbacks did not quiesce; collector retained",
            ),
            Self::Parties => (
                collector_governance::party_changes::active() != 0,
                "political party callbacks did not quiesce; collector retained",
            ),
            Self::Wings => (
                collector_observers::wing_changes::active() != 0,
                "air wing equipment callbacks did not quiesce; collector retained",
            ),
            Self::Roster => (
                collector_observers::roster_changes::active() != 0,
                "character roster callbacks did not quiesce; collector retained",
            ),
            Self::PoliticalStatus => (
                collector_observers::political_status_changes::active() != 0,
                "political status callbacks did not quiesce; collector retained",
            ),
            Self::Missions => (
                collector_missions::mission_changes::active() != 0,
                "registered air mission callbacks did not quiesce; collector retained",
            ),
            Self::Rails => (
                collector_observers::rail_changes::active() != 0,
                "rail info callbacks did not quiesce; collector retained",
            ),
            Self::Territory => (
                collector_observers::territory_changes::active() != 0,
                "state and province callbacks did not quiesce; collector retained",
            ),
            Self::ArmyStats => (
                collector_division_parts::army_stat_changes::active() != 0,
                "division stat block callbacks did not quiesce; collector retained",
            ),
            Self::CountryInputs => (
                collector_observers::country_input_changes::active() != 0,
                "stability and research input callbacks did not quiesce; collector retained",
            ),
            Self::Raid => (
                collector_observers::raid_changes::active() != 0,
                "raid state input callbacks did not quiesce; collector retained",
            ),
            Self::NavalTrees => (
                collector_observers::naval_tree_changes::active()
                    + collector_observers::retreat_ban_changes::active()
                    + collector_observers::unit_leader_trait_changes::active()
                    != 0,
                "naval modifier tree callbacks did not quiesce; collector retained",
            ),
            Self::UnitLeaders => (
                collector_observers::unit_leader_changes::active() != 0,
                "unit leader callbacks did not quiesce; collector retained",
            ),
            Self::NavalManagement => (
                collector_naval_management::changes::active() != 0,
                "naval management callbacks did not quiesce; collector retained",
            ),
            Self::Census => (
                collector_observers::census_changes::active() != 0,
                "role census callbacks did not quiesce; collector retained",
            ),
            Self::OrderMembership => (
                collector_observers::order_membership_changes::active() != 0,
                "order membership callbacks did not quiesce; collector retained",
            ),
            Self::OrdersGroups => (
                collector_observers::orders_group_changes::active() != 0,
                "orders group callbacks did not quiesce; collector retained",
            ),
            Self::WorldInputs => (
                collector_observers::world_input_changes::active() != 0,
                "province name, region access and world tension callbacks did not quiesce; collector retained",
            ),
            Self::WarParticipation => (
                collector_war_overview::participation::active() != 0,
                "war participation callbacks did not quiesce; collector retained",
            ),
            Self::AirRecords => (
                collector_air_records::air_record_changes::active() != 0,
                "air combat record callbacks did not quiesce; collector retained",
            ),
            Self::LocalSubunits => (
                collector_model_inputs::local_subunit_changes::active() != 0,
                "local sub-unit clone callbacks did not quiesce; collector retained",
            ),
            Self::Designs => (
                collector_production::production::design_changes::active() != 0,
                "equipment design callbacks did not quiesce; collector retained",
            ),
            Self::FactionCompanions => (
                collector_factions::faction_companion_changes::active() != 0,
                "faction companion callbacks did not quiesce; collector retained",
            ),
            Self::FactionGoals => (
                collector_factions::faction_goal_changes::active() != 0,
                "faction goal callbacks did not quiesce; collector retained",
            ),
            Self::Templates => (
                collector_division_templates::template_changes::active() != 0,
                "division template name-key callbacks did not quiesce; collector retained",
            ),
            Self::ShipModels => (
                collector_ship_models::ship_model_changes::active() != 0,
                "ship model input callbacks did not quiesce; collector retained",
            ),
            Self::ProvinceUnits => (
                collector_divisions::province_unit_changes::active() != 0,
                "province occupant list callbacks did not quiesce; collector retained",
            ),
            Self::ProvinceCombats => (
                collector_observers::province_combat_changes::active() != 0,
                "province combat vector callbacks did not quiesce; collector retained",
            ),
            Self::AirBaseCarry => (
                collector_observers::air_base_carry_changes::active() != 0,
                "air base registration callbacks did not quiesce; collector retained",
            ),
            Self::Experience => (
                collector_experience_gains::experience_gains::active() != 0,
                "experience callbacks did not quiesce; collector retained",
            ),
            Self::Fuel => (
                collector_fuel_awards::fuel_awards::active() != 0,
                "fuel callbacks did not quiesce; collector retained",
            ),
            Self::OperationOutcomes => (
                collector_operation_outcomes::operation_outcomes::active() != 0,
                "operation completion callbacks did not quiesce; collector retained",
            ),
            Self::Politics => (
                collector_politics::politics_changes::active() != 0,
                "politics callbacks did not quiesce; collector retained",
            ),
            Self::Logistics => (
                collector_logistics::logistics_balance::active() != 0,
                "logistics callbacks did not quiesce; collector retained",
            ),
            Self::Peace => (
                collector_peace::peace_settlements::ACTIVE.load(Ordering::SeqCst) != 0,
                "peace settlement callbacks did not quiesce; collector retained",
            ),
            Self::NavalResultSources => (
                collector_naval_results::naval_result_sources::active() != 0,
                "naval result callbacks did not quiesce; collector retained",
            ),
            Self::AirLosses => (
                collector_observers::air_losses::active() != 0,
                "air loss callbacks did not quiesce; collector retained",
            ),
            Self::BuildingBombing => (
                collector_observers::building_bombing::active() != 0,
                "building bombing callbacks did not quiesce; collector retained",
            ),
            Self::CombatEngagementRefreshes => (
                collector_observers::combat_engagement_refreshes::active() != 0,
                "combat engagement refresh callbacks did not quiesce; collector retained",
            ),
            Self::NuclearStrikes => (
                collector_observers::nuclear_strikes::active() != 0,
                "nuclear strike callbacks did not quiesce; collector retained",
            ),
            Self::Market => (
                collector_observers::market_changes::active() != 0,
                "market callbacks did not quiesce; collector retained",
            ),
            Self::MapEvents => (
                collector_map_events::hooks::active() != 0,
                "map event callbacks did not quiesce; collector retained",
            ),
            Self::EventFeed => (
                collector_observers::event_feed_observer::active() != 0,
                "Event feed callbacks did not quiesce; collector retained",
            ),
            Self::WorldLifetime => (
                collector_lifetime::world_lifetime::ACTIVE.load(Ordering::SeqCst) != 0,
                "native world lifetime callbacks did not quiesce; collector retained",
            ),
        }
    }

    fn shared(self) -> bool {
        matches!(
            self,
            Self::Portrait
                | Self::CommandBatch
                | Self::Opinion
                | Self::Advisor
                | Self::Relation
                | Self::History
                | Self::Flags
                | Self::Propaganda
                | Self::IntelSources
                | Self::NetworkAuxiliary
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
                | Self::OrdersGroups
                | Self::OrderMembership
                | Self::WorldInputs
                | Self::AirRecords
                | Self::WarParticipation
                | Self::LocalSubunits
                | Self::Designs
                | Self::Templates
                | Self::FactionGoals
                | Self::FactionCompanions
                | Self::ShipModels
                | Self::ProvinceUnits
                | Self::ProvinceCombats
                | Self::AirBaseCarry
                | Self::AirLosses
                | Self::CombatEngagementRefreshes
                | Self::NuclearStrikes
                | Self::Market
        )
    }
}

#[cfg(test)]
pub(crate) fn hook_patch_families() -> impl Iterator<Item = &'static str> {
    RestoreFamily::ALL.into_iter().map(|family| match family {
        RestoreFamily::Hourly => "hourly",
        RestoreFamily::EquipmentDestruction => "equipment_destruction",
        RestoreFamily::Land => "land_events",
        RestoreFamily::Portrait => "portrait_changes",
        RestoreFamily::Opinion => "opinion_changes",
        RestoreFamily::Construction => "construction_ic",
        RestoreFamily::LineOutput => "line_output",
        RestoreFamily::CombatLogPrune => "combat_log_prune",
        RestoreFamily::Advisor => "advisor_changes",
        RestoreFamily::Relation => "relation_changes",
        RestoreFamily::History => "history_changes",
        RestoreFamily::Flags => "flag_changes",
        RestoreFamily::Buildings => "building_changes",
        RestoreFamily::AdvisorPools => "advisor_pool_changes",
        RestoreFamily::PartyLeaders => "party_leader_changes",
        RestoreFamily::Variables => "variable_changes",
        RestoreFamily::Parties => "party_changes",
        RestoreFamily::Wings => "wing_changes",
        RestoreFamily::Roster => "roster_changes",
        RestoreFamily::PoliticalStatus => "political_status_changes",
        RestoreFamily::Missions => "mission_changes",
        RestoreFamily::Rails => "rail_changes",
        RestoreFamily::Territory => "territory_changes",
        RestoreFamily::ArmyStats => "army_stat_changes",
        RestoreFamily::CountryInputs => "country_input_changes",
        RestoreFamily::Raid => "raid_changes",
        RestoreFamily::NavalTrees => "naval_tree_changes",
        RestoreFamily::UnitLeaders => "unit_leader_changes",
        RestoreFamily::Census => "census_changes",
        RestoreFamily::OrdersGroups => "orders_group_changes",
        RestoreFamily::OrderMembership => "order_membership_changes",
        RestoreFamily::WorldInputs => "world_input_changes",
        RestoreFamily::AirRecords => "air_record_changes",
        RestoreFamily::LocalSubunits => "local_subunit_changes",
        RestoreFamily::Templates => "template_changes",
        RestoreFamily::ShipModels => "ship_model_changes",
        RestoreFamily::ProvinceUnits => "province_unit_changes",
        RestoreFamily::ProvinceCombats => "province_combat_changes",
        RestoreFamily::AirBaseCarry => "air_base_carry_changes",
        RestoreFamily::Experience => "experience_gains",
        RestoreFamily::PlayerCommands => "player_commands",
        RestoreFamily::Fuel => "fuel_awards",
        RestoreFamily::OperationOutcomes => "operation_outcomes",
        RestoreFamily::Politics => "politics_changes",
        RestoreFamily::Logistics => "logistics_balance",
        RestoreFamily::Peace => "peace_settlements",
        RestoreFamily::NavalResultSources => "naval_result_sources",
        RestoreFamily::AirLosses => "air_losses",
        RestoreFamily::BuildingBombing => "building_bombing",
        RestoreFamily::CombatEngagementRefreshes => "combat_engagement_refreshes",
        RestoreFamily::NuclearStrikes => "nuclear_strikes",
        RestoreFamily::MapEvents => "map_events",
        RestoreFamily::EventFeed => "event_feed_observer",
        RestoreFamily::Propaganda => "propaganda_changes",
        RestoreFamily::IntelSources => "intel_sources",
        RestoreFamily::NetworkAuxiliary => "network_auxiliary",
        RestoreFamily::NavalManagement => "naval_management_changes",
        RestoreFamily::WarParticipation => "war_participation",
        RestoreFamily::Designs => "design_changes",
        RestoreFamily::FactionGoals => "faction_goals",
        RestoreFamily::FactionCompanions => "faction_companions",
        RestoreFamily::Market => "market_changes",
        RestoreFamily::CommandBatch => "command_batch",
        RestoreFamily::WorldLifetime => "world_lifetime",
    })
}

type AttachFamily<'a> = (
    &'static str,
    &'a mut dyn FnMut() -> Result<bool, &'static str>,
);

pub(crate) fn install_attach_batch(
    stage: &crate::progress::Stage<'_>,
    families: &mut [AttachFamily<'_>],
) -> Result<bool, &'static str> {
    let mut round = collector_hooks::hook::shared::Round::collect_attach();
    let mut collect_error = None;
    if let Some(round) = &mut round {
        for (index, (name, install)) in families.iter_mut().enumerate() {
            collector_hooks::hook::publication::select_family(Some(*name));
            let _ = collector_hooks::hook::shared::take_collect_allocation_failure();
            let outcome = install();
            if let Some(error) = collector_hook_failures::fatal_installation_error() {
                stage.set(name);
                return Err(error);
            }
            let allocation_failure =
                collector_hooks::hook::shared::take_collect_allocation_failure();
            if let Err(reason) = outcome {
                collect_error = Some((index, reason, allocation_failure));
                break;
            }
        }
        round.publish();
        let mut rollback_error = None;
        round.for_each_attach_rollback_failure(|family, failure| {
            if let Err(error) = collector_hook_failures::shared_rollback_failed(
                crate::hook_inventory::patches,
                family,
                failure,
                installation_message,
            ) && rollback_error.is_none()
            {
                stage.set(family.unwrap_or("unregistered shared rollback"));
                rollback_error = Some(error);
            }
        });
        if let Some(error) = rollback_error {
            return Err(error);
        }
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
        collector_hooks::hook::shared::prepare_refused_family(families[index].0);
        if allocation_failure {
            let _ = engine_allocation::oom();
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
    elapsed: impl Fn() -> Duration,
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
                if detach_retry_delay(elapsed(), Duration::from_millis(100)).is_none() {
                    return;
                }
                wait();
            }
            Err(_) => {
                previous_errors[index] = true;
                return;
            }
        }
    }
}

struct HookLifetime<'a> {
    restored: bool,
    shared: &'a SharedSlot,
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
            // SAFETY: The runtime has restored hooks and stopped users of this published state.
            // Fixtures clear only their own slot.
            unsafe { self.shared.clear() };
            drop(hooks);
        } else {
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

fn ticks_to_micros(ticks: i64, frequency: i64) -> u64 {
    u64::try_from(i128::from(ticks.max(0)) * 1_000_000 / i128::from(frequency.max(1)))
        .unwrap_or(u64::MAX)
}

fn signal_done(output_lock: File, done: &Event) {
    drop(output_lock);
    // SAFETY: The tick exit owns the session's idle event and sets it after leaving the gate.
    unsafe { done.set() };
}

fn wait_acquisitions(
    admission: &CaptureAdmission,
    elapsed: impl Fn() -> Duration,
) -> Result<(), &'static str> {
    while !admission.quiet() {
        if elapsed() >= DETACH_BUDGET {
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

struct DecodeStatus<T, U> {
    base: collector_status::status::Object<T>,
    shared: Option<collector_status::status::Object<U>>,
}

impl<T: collector_status::status::Fields, U: collector_status::status::Fields> serde::Serialize
    for DecodeStatus<T, U>
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let fields = self.base.0.field_count()
            + self
                .shared
                .as_ref()
                .map_or(0, |status| status.0.field_count());
        let mut map = serializer.serialize_map(Some(fields))?;
        self.base.0.serialize(&mut map)?;
        if let Some(status) = &self.shared {
            status.0.serialize(&mut map)?;
        }
        map.end()
    }
}

struct EdgesWithheldStatus;

impl serde::Serialize for EdgesWithheldStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        collector_land_events::land_events::with_edges_withheld(|reason| {
            reason.serialize(serializer)
        })
    }
}

fn decode_status() -> impl serde::Serialize {
    let shared = unsafe { SHARED.current() };
    decode_status_from(shared)
}

fn decode_status_from(shared: Option<&Shared>) -> impl serde::Serialize + '_ {
    use collector_status::status::status;
    let base = status!({
        "worker_panic": collector_workers::worker_panic::first(),
        "hook_timing": collector_timing::hook_timing::status(),
        "capture": collector_capture::capture_point::status(),
        "equipment_destruction": collector_equipment_destruction::equipment_destruction::status(),
        "sample_boundary": sampling::SAMPLE_BOUNDARY.name(),
        "order_membership_signals": collector_observers::order_membership_changes::status(),
        "player_commands": collector_player_commands::player_commands::status(),
        "propaganda_changes": collector_intel::propaganda_changes::status(),
        "intel_sources": collector_intel::source_changes::status(),
        "combat_log_history": collector_combat_log::history_events::refusals(),
        "politics_changes": collector_politics::politics_changes::refusals(),
        "variant_stat_witness": collector_production::production::design_changes::stat_witness_status(),
        "supply_truck_demand_witness": collector_supply_trains::supply_trains::truck_demand_witness_status(),
        "raid_completion": collector_projects::projects::raid_completion_data(),
        "event_windows": collector_observers::event_window_observer::status(),
        "land_combat_edges_withheld": EdgesWithheldStatus,
        "certificate_registries": collector_certificates::watch_registry::status(collector_division_parts::army_stat_changes::registries()),
    });
    #[cfg(feature = "order_membership_oracle")]
    let base = base.join(status!({
        "order_membership_oracle": engine_native::carmy::membership_oracle(),
        "order_membership_hook_spans": collector_observers::order_membership_changes::hook_reads(),
    }));
    #[cfg(feature = "acquisition_profile")]
    let base = base.join(status!({
        "native_acquisition": collector_memory::acquisition_profile::status(),
        "division_acquisition_phase_ticks": collector_divisions::extended::division_presentation::profile(),
    }));
    #[cfg(feature = "callback_profile")]
    let base = base.join(status!({
        "callback_profile": collector_workers::callback_profile::status(),
        "fuel_award_tags": collector_fuel_awards::fuel_awards::tag_profile(),
        "fuel_award_tags_scope": "diagnostic: every fuel tap callback since attach by tap (BEGIN, END, the five land receipts, naval, air, reset) as [calls, ns], TSC-timed inside the callback, so the tap bridge and relay lie outside",
    }));
    #[cfg(feature = "worker_sampler")]
    let base = base.join(status!({"worker_sampler": collector_workers::worker_sampler::status()}));
    #[cfg(feature = "phase_cpu")]
    let base = base.join(status!({"phase_cpu": collector_workers::spool_phases::status()}));
    #[cfg(feature = "advisor_profile")]
    let base = base.join(
        status!({"advisor_profile": collector_characters::extended::characters::advisors::profile::status()}),
    );
    let shared = shared.map(|shared| {
        status!({
            "native_tick_work": {
                "elapsed_ns":
                    // SAFETY: Status reads the counter maintained by this session's native callback.
                    unsafe { shared.native_tick_ns() }.load(Ordering::Relaxed),
                "calls":
                    // SAFETY: Status reads this session's counter and the fixture seeds only its own counter.
                    unsafe { shared.native_tick_calls() }.load(Ordering::Relaxed),
            },
            "acquisition_work": acquisition_work(shared),
            "hour_stages": collector_workers::hour_stages::status(&|ticks| shared.ns(ticks)),
            "collector_thread_cpu": collector_workers::thread_cpu::status(
                // SAFETY: This is the runtime's own acquisition pool, retained through worker join.
                unsafe { shared.acquisition() }
                    .get()
                    .map(|acquisition| acquisition.calibration().executor.thread_ids)
                    .unwrap_or_default()
                    .ids(),
            ),
        })
    });
    DecodeStatus { base, shared }
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
    use collector_test_support::with_allocations_denied;
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
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Shared::new(0, 0, 1) };
    let pool =
    // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
    unsafe { acquisition::Pool::start(1, collector_workers::roles::MAX_JOBS) }
        .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
    assert!(
        // SAFETY: This is the runtime's own acquisition pool, retained through worker join.
        unsafe { shared.acquisition() }.set(pool).is_ok()
    );

    // SAFETY: Status reads the live counter. The fixture writes only isolated state.
    unsafe { shared.native_tick_calls() }.store(3, Ordering::Relaxed);
    let decode = with_allocations_denied(|| decode_status_from(Some(&shared)));
    let value = collector_status::status::status!({"decode": decode});
    let mut count = Count(0);
    with_allocations_denied(|| serde_json::to_writer(&mut count, &value))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut bytes = Vec::new();
    engine_allocation::reserve_exact(&mut bytes, count.0)?;

    // SAFETY: Status reads the live counter. The fixture writes only isolated state.
    unsafe { shared.native_tick_calls() }.store(30, Ordering::Relaxed);
    with_allocations_denied(|| serde_json::to_writer(&mut bytes, &value))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    assert_eq!(bytes.len(), count.0);
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["decode"]["native_tick_work"]["calls"], 3);
    let families = status["decode"]["hook_timing"]["families"].as_object();
    assert_eq!(
        families.map(serde_json::Map::len),
        Some(collector_timing::hook_timing::FAMILY_COUNT)
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
    assert!(absent.get("native_tick_work").is_none());
    assert!(absent.get("collector_thread_cpu").is_none());
    assert!(absent["hook_timing"]["families"]["hourly"].is_object());
    for (family, reason) in [
        ("intel_sources", "first_loss"),
        ("politics_changes", "staging_error"),
    ] {
        assert!(absent[family].get(reason).is_some(), "{family}.{reason}");
    }
    assert!(absent["combat_log_history"].get("first_loss").is_some());

    // SAFETY: The runtime owns this attachment's worker pool through join. No submitted work is still borrowing its storage at
    // shutdown.
    unsafe { shared.acquisition().get().unwrap().shutdown() }
        .map_err(|_| io::Error::from(io::ErrorKind::Other))
}

pub(crate) fn observers_active() -> bool {
    RestoreFamily::ALL
        .into_iter()
        .any(|family| family.observer_state().0)
}

const DETACH_BUDGET: Duration = Duration::from_secs(5);
const FINAL_HOUR_WAIT: Duration = Duration::from_secs(1);

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
    observer_detach_refusal(elapsed, active)
        .unwrap_or("collector mutation entry restoration refused; collector retained")
}

#[derive(Debug, PartialEq, Eq)]
enum RestoreRound {
    Complete,
    Retry(Duration),
    Stalled,
}

struct RestoreWait {
    ready: usize,
    progressed: Instant,
}
impl RestoreWait {
    fn new(now: Instant) -> Self {
        Self {
            ready: 0,
            progressed: now,
        }
    }
    fn round(
        &mut self,
        restored: &mut [bool; RestoreFamily::ALL.len()],
        previous_errors: &mut [bool; RestoreFamily::ALL.len()],
        active: impl FnMut() -> bool,
        restore: impl FnMut(RestoreFamily) -> Result<bool, &'static str>,
        now: impl FnOnce() -> Instant,
    ) -> Result<RestoreRound, &'static str> {
        if restore_round(restored, previous_errors, active, restore)? {
            Ok(RestoreRound::Complete)
        } else {
            Ok(match self.retry(now(), restored) {
                Some(delay) => RestoreRound::Retry(delay),
                None => RestoreRound::Stalled,
            })
        }
    }
    fn retry(
        &mut self,
        now: Instant,
        restored: &[bool; RestoreFamily::ALL.len()],
    ) -> Option<Duration> {
        let ready = restored.iter().filter(|ready| **ready).count();
        if ready > self.ready {
            self.ready = ready;
            self.progressed = now;
        }
        detach_retry_delay(
            now.duration_since(self.progressed),
            Duration::from_millis(100),
        )
    }
}

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
    collector_observers::air_losses::mark_stranded();
    collector_observers::building_bombing::mark_stranded();
    collector_observers::combat_engagement_refreshes::mark_stranded();
    collector_observers::nuclear_strikes::mark_stranded();
    collector_observers::market_changes::mark_stranded();
    loop {
        if let Some(reason) =
            observer_detach_refusal(started.elapsed(), |family| family.observer_state().0)
        {
            return Err(reason);
        }
        if !observers_active() {
            if let Some(result) = collector_hooks::hook::restoration::quiet(
                ranges,
                excluded_threads,
                observers_active,
            ) {
                if result? {
                    return Ok(());
                }
            } else if let Ok(mut threads) = Threads::open_excluding(excluded_threads) {
                let result = threads.suspend().and_then(|()| {
                    if observers_active() {
                        Ok(false)
                    } else {
                        threads.contains_ip(ranges).map(|present| !present)
                    }
                });
                if threads.resume().is_err() {
                    return Err("collector quiet thread resume refused; collector retained");
                }
                if result == Ok(true) {
                    return Ok(());
                }
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
    fn persistent_restore_errors_are_not_hidden_by_progress_in_the_same_round() {
        let start = Instant::now();
        let mut wait = RestoreWait::new(start);
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut previous_errors = [false; RestoreFamily::ALL.len()];
        for (round, expected) in [
            Ok(RestoreRound::Retry(Duration::from_millis(100))),
            Err("bytes differ"),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                wait.round(
                    &mut restored,
                    &mut previous_errors,
                    || false,
                    |family| match family {
                        RestoreFamily::Hourly => Err("bytes differ"),
                        RestoreFamily::Portrait => Ok(round != 0),
                        _ => Ok(true),
                    },
                    || start + DETACH_BUDGET * (round as u32 + 1),
                ),
                expected
            );
        }
        assert!(restored[RestoreFamily::Portrait as usize]);
        assert!(previous_errors[RestoreFamily::Hourly as usize]);
    }

    #[test]
    fn terminal_equipment_restore_reaches_unload_with_persistent_allocator_refusal() {
        const CHILD: &str = "BATTLEPLAN_TERMINAL_EQUIPMENT_RESTORE";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "runtime::detach_policy_tests::terminal_equipment_restore_reaches_unload_with_persistent_allocator_refusal", "--nocapture"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "restore child failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        unsafe extern "system" fn target() -> usize {
            7
        }
        unsafe extern "system" fn callback() -> usize {
            19
        }
        let code = collector_hooks::hook::TestCode::new();
        let make = |offset| {
            let (site, bytes) = code.stub(offset, target as *const () as usize);
            unsafe { Hook::new(site, callback as *const () as usize, bytes) }.unwrap()
        };
        let hooks = collector_equipment_destruction::equipment_destruction::synthetic_restore_hooks(
            make(0),
            make(64),
        );
        let mut hooks =
            crate::hook_failures::FamilyHooks::new(RestoreFamily::EquipmentDestruction, Ok(hooks))
                .unwrap();
        assert_eq!(hooks.set_enabled(true), Ok(true));
        let ranges = hooks.executable_ranges().collect::<Vec<_>>();
        let scratch = collector_hooks::hook::restoration::Scratch::prepare(2)
            .unwrap()
            .enter();
        collector_memory::terminal::install_hooks();
        collector_memory::terminal::fail(
            collector_memory::terminal::Phase::Extended,
            collector_memory::terminal::Code::Reserve,
        );
        let shared = SharedSlot::retained_for_test();
        let unloaded = std::cell::Cell::new(false);
        let (restored, quiet, exit) = collector_test_support::with_allocations_denied(|| {
            let restored = hooks.set_enabled(false);
            let quiet = collector_hooks::hook::restoration::quiet(&ranges, &[], || false);
            if restored != Ok(true) || quiet != Some(Ok(true)) {
                std::mem::forget(hooks);
                return (restored, quiet, 1);
            }
            HookLifetime {
                restored: true,
                shared: &shared,
            }
            .release(hooks, || {});
            let exit = finish_worker(&shared, (), |()| {
                unloaded.set(true);
                0
            });
            drop(scratch);
            (restored, quiet, exit)
        });
        assert_eq!(restored, Ok(true));
        assert_eq!(quiet, Some(Ok(true)));
        assert_eq!(exit, 0);
        assert!(unloaded.get());
        assert!(!shared.is_published());
        assert!(collector_memory::terminal::failed());
        assert_eq!(collector_test_support::ATTEMPTS.load(Ordering::Relaxed), 0);
    }

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
                        RestoreFamily::Portrait => Ok(false),
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
    fn observers_active_tracks_in_flight_domain_callbacks() {
        if !crate::hook_failures::contract_tests::isolated(
            "runtime::detach_tests::observers_active_tracks_in_flight_domain_callbacks",
        ) {
            return;
        }

        assert!(!observers_active());
        collector_equipment_destruction::equipment_destruction::tests::with_active_observer(|| {
            assert!(observers_active());
        });
        assert!(!observers_active());
        collector_land_events::land_events::test_support::with_active_observer(|| {
            assert!(observers_active());
        });
        assert!(!observers_active());
        collector_naval_results::naval_result_sources::tests::with_active_observer(|| {
            assert!(observers_active());
        });
        assert!(!observers_active());
    }

    #[test]
    fn expired_detach_budget_does_not_restart_or_reset_acquisition_leases() {
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut errors = [false; RestoreFamily::ALL.len()];
        restore_hourly(
            &mut restored,
            &mut errors,
            || DETACH_BUDGET,
            || Ok(false),
            || panic!("hourly restoration restarted the detach budget"),
        );
        assert!(!restored[RestoreFamily::Hourly as usize]);
        let admission = CaptureAdmission::new();
        let lease =
        // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
        // hourly exit.
        unsafe { admission.enter() }.unwrap();
        // SAFETY: Detach closes admission before waiting for admitted capture to complete.
        unsafe { admission.close() };
        assert_eq!(
            wait_acquisitions(&admission, || DETACH_BUDGET),
            Err("hourly acquisitions did not quiesce; collector retained"),
        );
        assert!(!admission.quiet());
        drop(lease);
        assert_eq!(wait_acquisitions(&admission, || DETACH_BUDGET), Ok(()));
    }

    #[test]
    fn world_signal_restores_only_after_other_hooks_and_native_returns_are_quiet() {
        let mut restored = [true; RestoreFamily::ALL.len()];
        restored[RestoreFamily::WorldLifetime as usize] = false;
        assert!(!world_restore_ready(&restored, true));
        assert!(world_restore_ready(&restored, false));
        for family in RestoreFamily::ALL {
            if family == RestoreFamily::WorldLifetime {
                continue;
            }
            restored[family as usize] = false;
            assert!(
                !world_restore_ready(&restored, false),
                "{family:?} must restore before world lifetime"
            );
            restored[family as usize] = true;
        }
    }

    #[test]
    fn world_signal_restores_in_the_round_where_the_last_callback_returns() {
        let callbacks = CaptureAdmission::new();
        let mut native_return = Some(
            // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
            // hourly exit.
            unsafe { callbacks.enter() }.unwrap(),
        );
        let mut restored = [false; RestoreFamily::ALL.len()];
        let mut errors = [false; RestoreFamily::ALL.len()];
        let mut world_restores = 0;
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
        let native_return =
        // SAFETY: This is the admitted callback's live gate entry, paired with its native tick or
        // hourly exit.
        unsafe { callbacks.enter() }.unwrap();
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

#[derive(Default)]
struct RegistryWatch {
    last: Option<Instant>,
    slots: usize,
    regions: usize,
    last_intel: Option<Instant>,
    intel_rows: usize,
    last_ace_names: Option<Instant>,
    ace_names: collector_air::sizing::AceNamesDemand,
    last_air_wing_names: Option<Instant>,
    air_wing_names: usize,
    last_railway_gun_names: Option<Instant>,
    railway_gun_names: engine_railways::railway_gun_counter_inputs::NameDemand,
    last_map: Option<Instant>,
    last_presentation_census: Option<Instant>,
    presentation_census: engine_native::carmy::PresentationDemand,
    last_raids: Option<Instant>,
    raids: collector_capture::country_scale::RaidArenas,
    last_characters: Option<Instant>,
    characters: collector_capture::country_scale::CharacterDemand,
    last_saved_targets: Option<Instant>,
    saved_targets: engine_scripts::saved_event_targets::Demand,
    last_politics: Option<Instant>,
    politics: collector_capture::country_scale::PoliticsDemand,
    last_reserved_visibility: Option<Instant>,
    reserved_visibility: usize,
    last_lines: Option<Instant>,
    last_logistics: Option<Instant>,
    last_game_score: Option<Instant>,
    last_combat_hints: Option<Instant>,
}

impl RegistryWatch {
    const INTERVAL: Duration = Duration::from_millis(250);
    const INTEL_INTERVAL: Duration = Duration::from_secs(8);

    fn poll_reserved_visibility(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<usize>, &'static str> {
        if self
            .last_reserved_visibility
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_reserved_visibility = Some(Instant::now());
        let Some(slots) =
            collector_capture::country_scale::observe_reserved_visibility(memory, base)?
        else {
            return Ok(None);
        };
        Ok((slots > self.reserved_visibility).then(|| {
            self.reserved_visibility = slots;
            slots
        }))
    }

    fn poll_game_score(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Option<io::Result<engine_native_air::game_score_inputs::ArenaSizing>> {
        if self
            .last_game_score
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_game_score = Some(Instant::now());
        match collector_air::sizing::observe_game_score(memory, base) {
            Ok(Some(observed)) => Some(Ok(observed)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    fn poll_combat_hints(
        &mut self,
        memory: &impl engine_native_memory::Memory,
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

    fn poll_logistics(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        now: impl Fn() -> Instant,
    ) -> io::Result<Option<usize>> {
        if self
            .last_logistics
            .is_some_and(|last| now().saturating_duration_since(last) < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_logistics = Some(now());
        collector_logistics::logistics_balance::prepare(memory, base).map(Some)
    }

    fn poll_lines(&mut self, memory: &impl engine_native_memory::Memory, base: usize) -> Option<usize> {
        if self
            .last_lines
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return None;
        }
        self.last_lines = Some(Instant::now());
        collector_capture::country_scale::observe_production_output(memory, base)
    }

    fn poll(&mut self, memory: &impl engine_native_memory::Memory, base: usize) -> Option<(usize, usize)> {
        if self
            .last
            .is_some_and(|last| last.elapsed() < Self::INTERVAL)
        {
            return None;
        }
        self.last = Some(Instant::now());
        let slots = collector_capture::country_scale::observe(memory, base)?;
        let regions = strategic_regions(memory, base);
        (slots > self.slots || regions > self.regions).then(|| {
            self.slots = self.slots.max(slots);
            self.regions = self.regions.max(regions);
            (self.slots, self.regions)
        })
    }

    fn poll_intel(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<usize>, &'static str> {
        if self
            .last_intel
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_intel = Some(Instant::now());
        let Some(rows) = collector_capture::country_scale::observe_country_intel(memory, base)?
        else {
            return Ok(None);
        };
        Ok((rows > self.intel_rows).then(|| {
            self.intel_rows = rows;
            rows
        }))
    }

    fn poll_ace_names(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<collector_air::sizing::AceNamesDemand>, &'static str> {
        if self
            .last_ace_names
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_ace_names = Some(Instant::now());
        let Some(demand) = collector_air::sizing::observe_ace_names(memory, base)? else {
            return Ok(None);
        };
        let before = self.ace_names;
        self.ace_names.aces = self.ace_names.aces.max(demand.aces);
        self.ace_names.ace_bytes = self.ace_names.ace_bytes.max(demand.ace_bytes);
        Ok((self.ace_names != before).then_some(self.ace_names))
    }

    fn poll_air_wing_names(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<usize>, &'static str> {
        if self
            .last_air_wing_names
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_air_wing_names = Some(Instant::now());
        let Some(bytes) = collector_air::sizing::observe_air_wing_names(memory, base)? else {
            return Ok(None);
        };
        Ok((bytes > self.air_wing_names).then(|| {
            self.air_wing_names = bytes;
            bytes
        }))
    }

    fn poll_railway_gun_names(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<engine_railways::railway_gun_counter_inputs::NameDemand>, &'static str> {
        if self
            .last_railway_gun_names
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_railway_gun_names = Some(Instant::now());
        let Some(demand) =
            collector_capture::country_scale::observe_railway_gun_names(memory, base)?
        else {
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

    fn poll_map(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<collector_capture::country_scale::MapDemand>, &'static str> {
        if self
            .last_map
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_map = Some(Instant::now());
        collector_capture::country_scale::observe_map(memory, base)
    }

    fn poll_presentation_census(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<engine_native::carmy::PresentationDemand>, &'static str> {
        if self
            .last_presentation_census
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_presentation_census = Some(Instant::now());
        let Some(demand) =
            collector_capture::country_scale::observe_presentation_census(memory, base)?
        else {
            return Ok(None);
        };
        let raised = self.presentation_census.max(demand);
        if raised == self.presentation_census && demand.failure.is_none() {
            return Ok(None);
        }
        self.presentation_census = raised;
        Ok(Some(raised))
    }

    fn poll_raids(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<collector_capture::country_scale::RaidArenas>, &'static str> {
        if self
            .last_raids
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_raids = Some(Instant::now());
        let Some(demand) = collector_capture::country_scale::observe_raid_arenas(memory, base)?
        else {
            return Ok(None);
        };
        let before = self.raids;
        self.raids = collector_capture::country_scale::RaidArenas {
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

    fn poll_characters(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<collector_capture::country_scale::CharacterDemand>, &'static str> {
        if self
            .last_characters
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_characters = Some(Instant::now());
        let Some(demand) = collector_capture::country_scale::observe_characters(memory, base)?
        else {
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

    fn poll_saved_targets(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<engine_scripts::saved_event_targets::Demand>, &'static str> {
        if self
            .last_saved_targets
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_saved_targets = Some(Instant::now());
        let Some(demand) = collector_capture::country_scale::observe_saved_targets(memory, base)?
        else {
            return Ok(None);
        };
        let raised = self.saved_targets.max(demand);
        if raised == self.saved_targets {
            return Ok(None);
        }
        self.saved_targets = raised;
        Ok(Some(raised))
    }

    fn poll_politics(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Option<collector_capture::country_scale::PoliticsDemand>, &'static str> {
        if self
            .last_politics
            .is_some_and(|last| last.elapsed() < Self::INTEL_INTERVAL)
        {
            return Ok(None);
        }
        self.last_politics = Some(Instant::now());
        let Some(demand) = collector_capture::country_scale::observe_politics(memory, base)? else {
            return Ok(None);
        };
        let changed = demand.exceeds(&self.politics) || demand.failures().next().is_some();
        self.politics = self.politics.max(demand);
        Ok(changed.then_some(self.politics))
    }
}

fn strategic_regions(memory: &impl engine_native_memory::Memory, base: usize) -> usize {
    engine_native::cgame_state::read_strategic_region_count(memory, base)
        .map_or(0, |count| count as usize)
}

struct CombatHints {
    land: Option<engine_native_land::land_combat_details::DetailDemand>,
    naval: Option<engine_naval_history::naval_combat_demand::Demand>,
    failures: [Option<&'static str>; 2],
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct CombatHintState {
    land: Option<collector_combat::combat_details::Capacities>,
    naval: Option<collector_naval_details::naval_details::Capacities>,
}

#[derive(Clone, Default, PartialEq)]
struct AppliedHints(Vec<(u64, CombatHintState)>);

impl AppliedHints {
    fn get(&self, index: usize, generation: u64) -> CombatHintState {
        self.0
            .get(index)
            .filter(|(applied, _)| *applied == generation)
            .map_or_else(CombatHintState::default, |(_, state)| *state)
    }

    fn set(&mut self, index: usize, generation: u64, state: CombatHintState) {
        if self.0.len() <= index {
            self.0
                .resize(index + 1, (u64::MAX, CombatHintState::default()));
        }
        self.0[index] = (generation, state);
    }

    #[cfg(test)]
    fn iter(&self) -> impl Iterator<Item = &CombatHintState> {
        self.0.iter().map(|(_, state)| state)
    }
}

impl std::ops::Index<usize> for AppliedHints {
    type Output = CombatHintState;

    fn index(&self, index: usize) -> &CombatHintState {
        &self.0[index].1
    }
}

struct SpoolState {
    sizing: collector_capture::country_scale::Sizing,
    applied_combat_hints: AppliedHints,
}

impl CombatHints {
    fn observe(memory: &impl engine_native_memory::Memory, base: usize) -> Self {
        let land = collector_capture::country_scale::observe_land_details(memory, base);
        let naval = collector_capture::country_scale::observe_naval_details(memory, base);
        Self {
            failures: [land.as_ref().err().copied(), naval.as_ref().err().copied()],
            land: land.ok().flatten(),
            naval: naval.ok().flatten(),
        }
    }

    fn pending(
        &self,
        capacities: &collector_capture::extended::Capacities,
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

fn grow_combat_hints(
    shared: &Shared,
    capacities: &collector_capture::extended::Capacities,
    hints: &CombatHints,
    applied: &mut AppliedHints,
) -> bool {
    let generation = |index: usize|
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.slot(index).generation.load(Ordering::Acquire);
    let made =
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.made();
    if (0..made).all(|index| {
        let hint = hints.pending(capacities, &applied.get(index, generation(index)));
        hint.land.is_none() && hint.naval.is_none()
    }) {
        return false;
    }
    let mut refused = false;
    for index in 0..made {
        if !
        // SAFETY: The sizing owner reserves the idle slot and later returns it.
        unsafe { shared.take_idle_frame_for_growth(index) }
        {
            continue;
        }
        let generation = generation(index);
        let mut state = applied.get(index, generation);
        let hint = hints.pending(capacities, &state);
        if hint.land.is_none() && hint.naval.is_none() {
            // SAFETY: The sizing owner exclusively holds the idle slot and is returning it once.
            unsafe { shared.restore_idle_frame_prepared(index) };
            continue;
        }
        let frame = unsafe {
            // SAFETY: This is the runtime's frame pool for the current session.
            shared.frames()[index].captured_mut()
        };
        let success = frame
            .extended
            .apply_combat_hints(hint.land.as_ref(), hint.naval.as_ref());
        shared.set_frame_bytes(index, frame.allocated_bytes() as u64);
        refused |= (hint.land.is_some() && !success[0]) || (hint.naval.is_some() && !success[1]);
        if success[0] {
            state.land = hint.land;
        }
        if success[1] {
            state.naval = hint.naval;
        }
        applied.set(index, generation, state);
        // SAFETY: The sizing owner exclusively holds the idle slot and is returning it once.
        unsafe { shared.restore_idle_frame_prepared(index) };
    }
    refused
}

fn grow_idle_frames(shared: &Shared, sizing: &SizingShared, capacities: &mut BufferCapacities) {
    let live = unsafe { LiveMemory::new() };
    let _ = capacities
        .extended
        .combat_progress
        .presize(&live, shared.base);
    for idle_index in 0..
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.made()
    {
        if !
        // SAFETY: The sizing owner reserves the idle slot and later returns it.
        unsafe { shared.take_idle_frame_for_growth(idle_index) }
        {
            continue;
        }
        #[cfg(test)]
        if let Some(check) = lock(&sizing.before_idle_frame_growth).take() {
            check(sizing, shared);
        }
        let idle = unsafe {
            // SAFETY: This is the runtime's frame pool for the current session.
            shared.frames()[idle_index].captured_mut()
        };
        // SAFETY: Sizing holds exclusive `&mut` access to this idle frame while session observer
        // storage stays live and teardown is excluded; shared observers synchronize concurrent growth.
        unsafe { capacities.grow_frame(idle) };
        shared.set_frame_bytes(idle_index, idle.allocated_bytes() as u64);
        sizing.account_homes(shared);
        // SAFETY: The sizing owner exclusively holds the idle slot and is returning it once.
        unsafe { shared.restore_idle_frame_prepared(idle_index) };
    }
    shared.grow_parked_packs(&mut capacities.extended);
    // SAFETY: Sizing uses only the country intel home belonging to this live session.
    unsafe { shared.country_intel() }.grow(&mut capacities.extended.intel.rows[15]);
    sizing.account_homes(shared);
}

fn acquisition_work(shared: &Shared) -> impl serde::Serialize {
    let [heap_hours, heap_hours_ns, heap_hour_max_ns] = shared.heap_waits();
    let calls = |calls: collector_workers::private_heap::Calls| {
        collector_status::status::status!({
            "calls": calls.calls,
            "ns": shared.ns(calls.ticks as i64),
            "max_ns": shared.ns(calls.max_ticks as i64),
        })
    };
    let (hold, other) = (
        collector_workers::private_heap::PRIVATE_HEAP.hold(),
        collector_workers::private_heap::PRIVATE_HEAP.other(),
    );
    let frames =
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.gauge();
    collector_status::status::status!({
        "buffer_wait_calls": 0,
        "buffer_wait_ns": 0,
        "buffer_wait_max_ns": 0,
        "waits": {
            "allocator": {
                "hold": {
                    "hours": heap_hours, "ns": heap_hours_ns, "max_ns": heap_hour_max_ns,
                    "heap": calls(hold.heap),
                    "pages": calls(hold.pages),
                    "cause": "the game thread or an acquisition worker entered the private heap's lock (a refill the depot could not serve, an over-aligned block) or asked the system for pages; steady blocks come from its own cache",
                },
                "other": {
                    "heap": calls(other.heap),
                    "pages": calls(other.pages),
                    "cause": "collector threads that never hold the game entered the private heap's lock or asked the system for pages",
                },
            },
        },
        "frames": {
            "steady": collector_capture::frame_pool::STEADY,
            "live": frames.live,
            "free": frames.free,
            "in_flight": frames.in_flight,
            "in_flight_bytes": frames.in_flight_bytes,
            "live_bytes": frames.live_bytes,
            "grown": frames.grown,
            "grow_ns": frames.grow_ns,
            "grow_max_ns": frames.grow_max_ns,
            "ahead_ready": frames.ahead_ready,
            "built_ahead": frames.built_ahead,
            "ahead_taken": frames.ahead_taken,
            "freed": frames.freed,
            "dropped_spool_stalled": frames.dropped_stalled,
            "dropped_frame_limit": frames.dropped_limit,
        },
        "scope": "cumulative since attach: the game thread's waits outside the collector's own reading by cause, buffer_wait_* zero by construction (no frame or held store is ever waited on: an hour that finds the held division store still settling takes one of its own; while no frame is free, the frame builder thread builds one spare frame ahead, built_ahead, which hours take, ahead_taken; an hour that still finds no frame free builds one on the game thread, grown/grow_ns, unless the spool is stalled on an hour it cannot take, dropped_spool_stalled, or the live frame count or byte limit is reached, dropped_frame_limit; frames past the steady count are freed once steady frames are idle or after a sustained idle streak, freed); frames now: live and all their bytes, free, ahead_ready (built ahead and not yet taken), in_flight (taken for an hour and not yet returned by the spool) and their bytes",
    })
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
struct SpoolStatus<P, D> {
    #[serde(flatten)]
    processor: P,
    #[serde(flatten)]
    decode: D,
    state: &'static str,
    session: u64,
    pool_bytes: u64,
    dropped_frames: u64,
    dropped_spool_stalled: u64,
    dropped_frame_limit: u64,
    drain_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    restoration_error: Option<&'static str>,
}
#[expect(clippy::too_many_arguments, reason = "one status line reports every spool state field")]
fn spool_status<'a>(
    processor: &'a Processor,
    shared: &'a Shared,
    session: &'a SessionStart,
    state: &'static str,
    warning: Option<&'static str>,
    error: Option<&'static str>,
    drain_complete: bool,
    terminal: bool,
    teardown: TeardownStatus,
) -> io::Result<impl serde::Serialize + 'a> {
    let pinned = terminal && teardown.restoration_error.is_some();
    Ok(SpoolStatus {
        processor: processor.status_fields()?,
        decode: decode_status(),
        state: if pinned { "pinned" } else { state },
        session: session.id,
        pool_bytes: shared.pool_total(),
        dropped_frames:
            // SAFETY: This reads the counter maintained by the originating capture.
            unsafe { shared.dropped() }.load(Ordering::Relaxed),
        dropped_spool_stalled:
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }.stalled_drops(),
        dropped_frame_limit:
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }.limit_drops(),
        drain_complete,
        warning: warning.or(teardown.restoration_error),
        error: error.or(teardown.installation_error),
        restoration_error: teardown.restoration_error,
    })
}

pub(crate) fn wait_completed_callback(
    base: &std::sync::atomic::AtomicI64,
    tail: &std::sync::atomic::AtomicI64,
) -> Result<i64, &'static str> {
    loop {
        if collector_memory::terminal::failed() {
            return Err("collector allocation failed");
        }
        if let Some(ticks) = sampling::completed_callback_ticks(base, tail) {
            return Ok(ticks);
        }
        std::thread::yield_now();
    }
}

struct StopRecordingOnSpoolExit<'a> {
    recording: &'a AtomicBool,
}
impl StopRecordingOnSpoolExit<'_> {
    fn stop(&self) {
        self.recording.store(false, Ordering::Release);
    }
}
impl Drop for StopRecordingOnSpoolExit<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}

fn verify_content_roster(
    shared: &Shared,
    config: &Config,
    session: &mut SessionStart,
    processor: &mut Processor,
    writer: &crate::status_writer::StatusHandle,
) -> Result<(), &'static str> {
    if !
        // SAFETY: The spool checks readiness before taking this session's roster.
        unsafe { shared.roster_ready() }.load(Ordering::Acquire)
    {
        return Ok(());
    }
    if let Some(roster) = unsafe {
        (&mut *
        // SAFETY: The session's spool has observed roster readiness and alone takes this roster.
         shared.roster() .get())
            .take()
    } {
        // SAFETY: The writer accounts memory to this same session and drains before it is released.
        unsafe { shared.pool_bytes() }
            .fetch_sub(roster.allocated_bytes() as u64, Ordering::Relaxed);
        let verification = roster.verify(
            &mut session.identity,
            &config.expected_mounts,
            &config.mod_descriptors,
            config.content_order_note.as_deref(),
        );
        let diagnostic =
            roster.diagnostics(&config.expected_mounts, &session.identity.verification);
        writer
            .publish(&config.output_dir.join("content.json"), diagnostic)
            .map_err(|e| error_reason(e, "content status allocation failed"))?;
        verification.map_err(|e| error_reason(e, "content verification failed"))?;
        processor
            .set_verified_session(
                session
                    .try_clone()
                    .map_err(|e| error_reason(e, "session allocation failed"))?,
            )
            .map_err(|e| error_reason(e, "verified session publication failed"))?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the spool thread owns its session, processor, sizing, writer and history separately"
)]
fn spool_loop(
    shared: &Shared,
    config: &Config,
    mut session: SessionStart,
    mut processor: Processor,
    status_path: &Path,
    sizing: SizingThread,
    writer: crate::status_writer::StatusHandle,
    teardown_status: &OnceLock<TeardownStatus>,
    history_requests: SpoolHistory,
) -> Result<SpoolOutcome, &'static str> {
    #[cfg(feature = "worker_sampler")]
    // SAFETY: This runtime-owned worker keeps the guard until it exits.
    let _sampler_exit = unsafe { collector_workers::thread_cpu::StopOnThreadExit::new() };
    let stop_recording = StopRecordingOnSpoolExit {
        recording:
            // SAFETY: The current runtime or its spool owns the session's terminal stop.
            unsafe { shared.recording() },
    };
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
    let live = unsafe { LiveMemory::new() };
    let mut last_warning = None;
    let mut drain_result = Ok(());
    let mut startup_unknown = 0_u64;
    let mut reported = EverySecond::default();
    let mut priority =
    // SAFETY: This is the dedicated worker applying its own session's priority policy.
    unsafe { OwnPriority::frame_pipeline() };
    // SAFETY: This owned worker updates its priority from its session's own pipeline.
    unsafe {
        priority.follow_pipeline(
            // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
            shared.frame_priority(),
            collector_workers::frame_priority::Role::Spool,
        )
    };
    let _ =
        // SAFETY: The sole spool registers itself before handling frames.
        unsafe { shared.spool_waker() }.set(std::thread::current());
    let phases = &collector_workers::spool_phases::SPOOL;
    loop {
        if collector_memory::terminal::failed() {
            return Err("collector allocation failed");
        }
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { phases.span(Phase::Observers) };
        if !collector_workers::tick_gate::game_ticking() {
            // SAFETY: This session's spool prepares only slots owned by its sizing pass.
            unsafe { shared.prepare_frame_clocks(&live) };
        }
        span.finish();
        verify_content_roster(shared, config, &mut session, &mut processor, &writer)?;
        // SAFETY: This owned worker updates its priority from its session's own pipeline.
        unsafe {
            priority.follow_pipeline(
                // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
                shared.frame_priority(),
                collector_workers::frame_priority::Role::Spool,
            )
        };
        let mut received = false;
        while let Some(index) =
            // SAFETY: The sole spool consumer takes responsibility for the decoded slot until
            // recycle.
            unsafe { shared.frames().pop_ready() }
        {
            // SAFETY: This stage owns the installed frame slot.
            unsafe { shared.frame_stage(index, sampling::FrameStage::Spool) };
            let watch = collector_spool::slow_hours::SpoolWatch::start();
            let watched_hour = unsafe {
                // SAFETY: This is the runtime's frame pool for the current session.
                shared.frames()[index].captured()
            }
            .clock
            .as_ref()
            .map(|clock| clock.hour);
            received = true;
            if collector_memory::terminal::failed() {
                return Err("collector allocation failed");
            }
            // SAFETY: This owned worker updates its priority from its session's own pipeline.
            unsafe {
                priority.follow_pipeline(
                    // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
                    shared.frame_priority(),
                    collector_workers::frame_priority::Role::Spool,
                )
            };
            let span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { phases.span(Phase::Receive) };
            let elapsed = wait_completed_callback(
                &
                    // SAFETY: This is the runtime's frame pool for the current session.
                    unsafe { shared.frames() }.slot(index).completed,
                &
                    // SAFETY: This is the runtime's frame pool for the current session.
                    unsafe { shared.frames() }.slot(index).callback_tail,
            )?;
            let frame = unsafe {
                // SAFETY: This is the runtime's frame pool for the current session.
                shared.frames()[index].captured_mut()
            };
            frame.total_ns = shared.ns(elapsed);
            collector_memory::terminal::set_context(collector_memory::terminal::Context {
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
            if !
                // SAFETY: The spool checks readiness before taking this session's roster.
                unsafe { shared.roster_ready() }.load(Ordering::Acquire)
            {
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
                // SAFETY: This owner has finished with the installed slot and its helpers have
                // joined.
                unsafe { shared.recycle_frame_prepared(index) };
                continue;
            }
            verify_content_roster(shared, config, &mut session, &mut processor, &writer)?;
            let mut stalled = false;
            while let Err(error) =
                // SAFETY: The caller owns the session and runs preparation before admitting consumers.
                unsafe { processor.prepare(frame) }
            {
                if !stalled {
                    stalled = true;
                    // SAFETY: This is the sole spool consumer reporting its own stall state.
                    unsafe { shared.spool_stalled(true) };
                }
                collector_memory::terminal::observe(&error);
                if collector_memory::terminal::failed() {
                    return Err("collector allocation failed");
                }
                if error.kind() == io::ErrorKind::InvalidData {
                    stop_recording.stop();
                    let _ =
                    // SAFETY: This spool's frame and command processing have finished before its
                    // processor is closed.
                    unsafe { processor.detach() };
                    return Err("campaign recording refused; observations remain unwritten");
                }
                last_warning = Some(error_reason(error, "collector spool I/O failed"));
                writer
                    .publish(
                        status_path,
                        collector_status::status::status!({
                            "state":"campaign spool unavailable", "session":session.id,
                            "warning":last_warning, "drain_complete":false,
                            "dropped_frames":
                                // SAFETY: The spool or fixture reads the count maintained by its own capture.
                                unsafe { shared.dropped() }.load(Ordering::Relaxed),
                            "dropped_spool_stalled":
                                // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                                unsafe { shared.frames() }.stalled_drops(),
                        }),
                    )
                    .map_err(|e| error_reason(e, "spool status allocation failed"))?;
                if
                // SAFETY: The runtime and sole spool coordinate this session's drain and exit.
                unsafe { shared.stop_spool() }.load(Ordering::Acquire) {
                    stop_recording.stop();
                    let _ =
                    // SAFETY: This spool's frame and command processing have finished before its
                    // processor is closed.
                    unsafe { processor.detach() };
                    return Err("cannot open campaign spool; observations remain unwritten");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if stalled {
                // SAFETY: This is the sole spool consumer reporting its own stall state.
                unsafe { shared.spool_stalled(false) };
            }
            if startup_unknown != 0 {
                // SAFETY: The originating spool accounts for this session's own missing capture.
                unsafe { processor.unknown_loss(std::mem::take(&mut startup_unknown)) };
            }
            if frame.dropped_before.unknown_count != 0 {
                // SAFETY: The originating spool accounts for this session's own missing capture.
                unsafe { processor.unknown_loss(frame.dropped_before.unknown_count) };
            }
            for loss in frame.dropped_before.iter() {
                if let Err(error) =
                    // SAFETY: The originating spool is submitting a loss from its own capture or
                    // isolated fixture.
                    unsafe { processor.lost(loss) }
                {
                    last_warning = Some(error_reason(error, "collector spool I/O failed"));
                }
            }
            span.finish();
            processor.set_backlogged(shared.buffers_backlogged());
            let pending_write = match
            // SAFETY: This is the decoded frame held by its originating spool.
            unsafe { processor.prepare_frame(frame) } {
                Ok(prepared) if prepared.allows_early_recycle() => Some(prepared),
                Ok(prepared) => {
                    if let Err(error) =
                    // SAFETY: This spool submits the prepared frame to its originating processor
                    // before teardown.
                    unsafe { processor.write_prepared(prepared) } {
                        collector_equipment_destruction::equipment_destruction::lost_drain();
                        collector_naval_results::naval_result_sources::lost_drain();
                        collector_experience_gains::experience_gains::lost_drain();
                        // SAFETY: Spool publication failure owns invalidating live observer intervals.
                        unsafe {
                            collector_observers::event_feed_observer::lost_drain();
                            collector_operation_outcomes::operation_outcomes::lost_drain();
                            collector_politics::politics_changes::invalidate();
                            collector_land_events::land_events::fail();
                            collector_observers::air_losses::lost_drain();
                            collector_observers::building_bombing::lost_drain();
                            collector_observers::combat_engagement_refreshes::lost_drain();
                            collector_observers::nuclear_strikes::lost_drain();
                            collector_observers::market_changes::lost_drain();
                            collector_map_events::interval::lost_output();
                        }
                        last_warning = Some(error_reason(error, "collector spool I/O failed"));
                    }
                    None
                }
                Err(error) => {
                    // SAFETY: The spool settled this frame's event-feed handoff before returning
                    // the slot.
                    unsafe { frame.extended.discard_event_feed() };
                    collector_equipment_destruction::equipment_destruction::lost_drain();
                    collector_naval_results::naval_result_sources::lost_drain();
                    collector_experience_gains::experience_gains::lost_drain();
                    // SAFETY: Spool publication failure owns invalidating live observer intervals.
                    unsafe {
                        collector_observers::event_feed_observer::lost_drain();
                        collector_operation_outcomes::operation_outcomes::lost_drain();
                        collector_politics::politics_changes::invalidate();
                        collector_land_events::land_events::fail();
                        collector_observers::air_losses::lost_drain();
                        collector_observers::building_bombing::lost_drain();
                        collector_observers::combat_engagement_refreshes::lost_drain();
                        collector_observers::nuclear_strikes::lost_drain();
                        collector_observers::market_changes::lost_drain();
                        collector_map_events::interval::lost_output();
                    }
                    last_warning = Some(error_reason(error, "collector spool I/O failed"));
                    None
                }
            };
            let span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { phases.span(Phase::Recycle) };
            if collector_memory::terminal::failed() {
                return Err("collector allocation failed");
            }
            if
            // SAFETY: The originating spool consumes its own retry request for the recovery
            // handoff.
            unsafe { processor.take_retry_full() } {
                // SAFETY: The spool requests recovery after unpublished output in the current
                // session.
                unsafe { shared.request_retry_full() };
            }
            let unpublished =
            // SAFETY: The originating spool forwards the families from its own failed publication.
            unsafe { processor.take_unpublished_families() };
            if unpublished != 0 {
                // SAFETY: The sole spool requests forgetting only the families from a failed publication.
                unsafe { shared.forget_families() }.fetch_or(unpublished, Ordering::Release);
            }
            if pending_write.is_none() {
                collector_memory::terminal::attempted_hour(None);
            }
            sizing.sizing.recycle_frame(shared, index);
            if shared.needs_steady_park() && shared.detach_idle_steady_frame() {
                sizing.retire_parked(shared);
            }
            span.finish();
            if let Some(prepared) = pending_write
                && let Err(error) =
                    // SAFETY: This spool submits the prepared frame to its originating processor
                    // before teardown.
                    unsafe { processor.write_prepared(prepared) }
            {
                collector_equipment_destruction::equipment_destruction::lost_drain();
                collector_naval_results::naval_result_sources::lost_drain();
                collector_experience_gains::experience_gains::lost_drain();
                // SAFETY: Spool publication failure owns invalidating live observer intervals.
                unsafe {
                    collector_observers::event_feed_observer::lost_drain();
                    collector_operation_outcomes::operation_outcomes::lost_drain();
                    collector_politics::politics_changes::invalidate();
                    collector_land_events::land_events::fail();
                    collector_observers::air_losses::lost_drain();
                    collector_observers::building_bombing::lost_drain();
                    collector_observers::combat_engagement_refreshes::lost_drain();
                    collector_observers::nuclear_strikes::lost_drain();
                    collector_observers::market_changes::lost_drain();
                    collector_map_events::interval::lost_output();
                }
                last_warning = Some(error_reason(error, "collector spool I/O failed"));
            }
            sizing.retire(processor.take_retired());
            sizing.retire_parked(shared);
            if let Err(error) =
                // SAFETY: The session's spool owns this command drain and detachment has quieted
                // callbacks for the final drain.
                unsafe { processor.flush_commands(false) }
            {
                last_warning = Some(error_reason(error, "command capture publication failed"));
            }
            history_requests.poll(
                &mut processor,
                // SAFETY: This is the same session's fence and the request is checked against its own slots.
                unsafe { shared.history_frames() },
                // SAFETY: This is the runtime's frame pool for the current session.
                unsafe { shared.frames() }.slots().map(|slot| &slot.fence),
            );
            processor
                .end_spool_frame(watch.finish(watched_hour, |ticks| shared.ns(ticks)))
                .map_err(|_| "spool slow-frame allocation failed")?;
            if reported.due(Instant::now()) {
                break;
            }
        }
        let between = collector_spool::slow_hours::SpoolWatch::start();
        collector_memory::terminal::attempted_hour(None);
        if collector_memory::terminal::failed() {
            return Err("collector allocation failed");
        }
        sizing.spool_pass(shared, &mut processor, received)?;
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { phases.span(Phase::Flush) };
        if collector_memory::terminal::failed() {
            return Err("collector allocation failed");
        }
        if let Err(error) =
            // SAFETY: The originating spool flushes its own writer before releasing its observer
            // state.
            unsafe { processor.flush() }
        {
            collector_equipment_destruction::equipment_destruction::lost_drain();
            collector_naval_results::naval_result_sources::lost_drain();
            collector_experience_gains::experience_gains::lost_drain();
            // SAFETY: Spool flush failure owns invalidating live observer intervals.
            unsafe {
                collector_observers::event_feed_observer::lost_drain();
                collector_operation_outcomes::operation_outcomes::lost_drain();
                collector_politics::politics_changes::invalidate();
                collector_land_events::land_events::fail();
                collector_observers::air_losses::lost_drain();
                collector_observers::building_bombing::lost_drain();
                collector_observers::combat_engagement_refreshes::lost_drain();
                collector_observers::nuclear_strikes::lost_drain();
                collector_observers::market_changes::lost_drain();
                collector_map_events::interval::lost_output();
            }
            last_warning = Some(error_reason(error, "collector spool I/O failed"));
        }
        span.finish();
        history_requests.poll(
            &mut processor,
            // SAFETY: This is the same session's fence and the request is checked against its own slots.
            unsafe { shared.history_frames() },
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }.slots().map(|slot| &slot.fence),
        );
        if reported.due(Instant::now()) ||
            // SAFETY: The runtime and sole spool coordinate this session's drain and exit.
            unsafe { shared.stop_spool() }.load(Ordering::Acquire)
        {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { phases.span(Phase::Status) };
            collector_memory::terminal::attempted_hour(None);
            let _status_phase =
                collector_memory::terminal::enter(collector_memory::terminal::Phase::Status);
            if history_requests.take_refresh_failure() {
                last_warning = Some("refresh request failed");
            }
            if let Some(error) = writer.take_error() {
                last_warning = Some(error);
            }
            let status = spool_status(
                &processor,
                shared,
                &session,
                if
                // SAFETY: The current runtime or its spool owns the session's terminal stop.
                unsafe { shared.recording() }.load(Ordering::Acquire) {
                    "recording"
                } else {
                    "detaching"
                },
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
            reported.done(Instant::now());
        }
        processor
            .end_spool_frame(between.finish(None, |ticks| shared.ns(ticks)))
            .map_err(|_| "spool slow-frame allocation failed")?;
        if
        // SAFETY: The runtime and sole spool coordinate this session's drain and exit.
        unsafe { shared.stop_spool() }.load(Ordering::Acquire) {
            if !
                // SAFETY: This is the runtime's frame pool for the current session.
                unsafe { shared.frames() }.ready_is_empty()
            {
                continue;
            }
            let _teardown =
                collector_memory::terminal::enter(collector_memory::terminal::Phase::Teardown);
            verify_content_roster(shared, config, &mut session, &mut processor, &writer)?;
            collector_memory::terminal::attempted_hour(None);
            let pending = unsafe {
                std::mem::take(&mut *
                // SAFETY: Capture is quiescent before the spool takes pending losses. Tests use isolated state.
                 shared.pending() .get())
            };
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
                        // SAFETY: The originating spool accounts for this session's own missing
                        // capture.
                        unsafe { processor.unknown_loss(pending.unknown_count) };
                    }
                    for loss in pending.iter() {
                        if let Err(error) =
                            // SAFETY: The originating spool is submitting a loss from its own capture
                            // or isolated fixture.
                            unsafe { processor.lost(loss) }
                        {
                            add_terminal_error(
                                &mut drain_result,
                                error_reason(error, "cannot drain retained loss witness"),
                            );
                        }
                    }
                }
            }
            if collector_memory::terminal::failed() {
                return Err("collector allocation failed");
            }
            if let Err(error) =
                // SAFETY: This spool's frame and command processing have finished before its processor
                // is closed.
                unsafe { processor.detach() }
            {
                add_terminal_error(
                    &mut drain_result,
                    error_reason(error, "cannot finish spool detach"),
                );
            }
            if startup_unknown != 0 {
                add_terminal_error(&mut drain_result, "startup callbacks had no readable clock");
            }
            if collector_memory::terminal::failed() {
                return Err("collector allocation failed");
            }
            let terminal_status = collector_status::status::serialize(
                &spool_status(
                    &processor,
                    shared,
                    &session,
                    "detached",
                    last_warning,
                    drain_result.as_ref().err().copied(),
                    drain_result.is_ok(),
                    true,
                    teardown_status.get().copied().unwrap_or_default(),
                )
                .map_err(|e| error_reason(e, "terminal status allocation failed"))?,
            )
            .map_err(|e| error_reason(e, "terminal status allocation failed"))?;
            let directories = processor
                .spool_directories()
                .map_err(|e| error_reason(e, "terminal directory allocation failed"))?;
            drop(processor);
            return Ok(SpoolOutcome {
                result: drain_result,
                terminal_status,
                directories,
            });
        }
        if
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.ready_is_empty() {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { phases.span(Phase::Wait) };
            std::thread::park_timeout(SPOOL_IDLE_WAIT);
        }
    }
}

type BufferCapacities = sampling::FrameCapacities;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct SizingBackoff {
    last: Option<Instant>,
    interval: Duration,
    world: u64,
}

impl SizingBackoff {
    const FAST: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(5);
    const SETTLED: Duration = Duration::from_secs(60);

    fn new() -> Self {
        Self {
            last: None,
            interval: Self::FAST,
            world: 0,
        }
    }

    fn due(&mut self, now: Instant, world: u64, demand: bool) -> bool {
        if world != self.world || demand {
            self.reset();
        }
        self.world = world;
        self.last
            .is_none_or(|last| now.duration_since(last) >= self.interval)
    }

    fn reset(&mut self) {
        self.interval = Self::FAST;
    }

    fn observed(&mut self, now: Instant, demand: bool, settled: bool) {
        self.last = Some(now);
        self.interval = if demand {
            Self::FAST
        } else {
            (self.interval * 2).min(if settled { Self::SETTLED } else { Self::MAX })
        };
    }
}

trait Moment: Copy {
    fn since(self, earlier: Self) -> Duration;
}

impl Moment for Instant {
    fn since(self, earlier: Self) -> Duration {
        self.saturating_duration_since(earlier)
    }
}

struct EverySecond<T = Instant>(Option<T>);

impl<T> Default for EverySecond<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T: Moment> EverySecond<T> {
    fn due(&self, now: T) -> bool {
        self.due_after(now, Duration::from_secs(1))
    }

    fn due_after(&self, now: T, interval: Duration) -> bool {
        self.0.is_none_or(|last| now.since(last) >= interval)
    }

    fn done(&mut self, now: T) {
        self.0 = Some(now);
    }
}

struct SizingState {
    registry: RegistryWatch,
    applied_combat_hints: AppliedHints,
    fuel_demand_at: Instant,
    second: EverySecond,
    capacity_census: SizingBackoff,
}

#[derive(Default)]
struct PendingSizing {
    errors: u64,
    last_error: Option<&'static str>,
}

#[cfg(test)]
type SizingCheck = Box<dyn FnOnce(&SizingShared, &Shared) + Send>;

struct SizingShared {
    capacities: engine_allocation::SharedBox<collector_capture::frame_capacities::Capacities>,
    capacities_version: AtomicU64,
    idle_growth_requested: AtomicBool,
    home_accounted: Mutex<[usize; 3]>,
    pass_requested: AtomicBool,
    hint_demand: AtomicBool,
    settled_world: AtomicU64,
    #[cfg(test)]
    before_recycle_publish: Mutex<Option<SizingCheck>>,
    #[cfg(test)]
    before_idle_frame_growth: Mutex<Option<SizingCheck>>,
    stop: AtomicBool,
    failure: OnceLock<&'static str>,
    pending: Mutex<PendingSizing>,
    context: Mutex<collector_memory::terminal::Context>,
    state: Mutex<Option<SizingState>>,
    retired: Mutex<Vec<replay_frame::frame::Retired>>,
}

impl SizingShared {
    fn new(shared: &Shared, capacities: BufferCapacities, state: Option<SizingState>) -> Self {
        *
            // SAFETY: These are the owned capacity snapshots for this session's frames.
            unsafe { shared.frame_capacities() }.lock() = capacities;
        Self {
            capacities:
                // SAFETY: These are the owned capacity snapshots for this session's frames.
                unsafe { shared.frame_capacities() }.clone(),
            capacities_version: AtomicU64::new(0),
            idle_growth_requested: AtomicBool::new(false),
            home_accounted: Mutex::new([
                // SAFETY: Sizing and the test use only the home associated with these session frames.
                unsafe { shared.country_intel() }.allocated_bytes(),
                0,
                0,
            ]),
            pass_requested: AtomicBool::new(false),
            hint_demand: AtomicBool::new(false),
            settled_world: AtomicU64::new(u64::MAX),
            #[cfg(test)]
            before_recycle_publish: Mutex::new(None),
            #[cfg(test)]
            before_idle_frame_growth: Mutex::new(None),
            stop: AtomicBool::new(false),
            failure: OnceLock::new(),
            pending: Mutex::new(PendingSizing::default()),
            context: Mutex::new(collector_memory::terminal::Context::default()),
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

    fn merge_capacities(&self, capacities: &mut BufferCapacities, proposal: BufferCapacities) {
        let before = *capacities;
        capacities.merge_max(proposal);
        if *capacities != before {
            self.capacities_version.fetch_add(1, Ordering::Release);
            self.idle_growth_requested.store(true, Ordering::Release);
        }
    }

    fn raise<R>(&self, shared: &Shared, step: impl FnOnce(&mut BufferCapacities) -> R) -> R {
        let mut proposal = self.with_capacities(shared, |capacities| *capacities);
        let result = step(&mut proposal);
        self.with_capacities(shared, |capacities| {
            self.merge_capacities(capacities, proposal)
        });
        result
    }

    fn raise_from_live_observation(
        &self,
        shared: &Shared,
        observe: impl FnOnce(&mut BufferCapacities) -> [Option<&'static str>; 2],
    ) {
        for reason in self.raise(shared, observe).into_iter().flatten() {
            self.error(reason);
        }
    }

    fn grow_idle(&self, shared: &Shared) {
        if !self.idle_growth_requested.swap(false, Ordering::AcqRel) {
            return;
        }
        let mut proposal = self.with_capacities(shared, |capacities| *capacities);
        grow_idle_frames(shared, self, &mut proposal);
        self.with_capacities(shared, |capacities| {
            self.merge_capacities(capacities, proposal)
        });
    }

    fn account_homes(&self, shared: &Shared) {
        let mut accounted = lock(&self.home_accounted);
        let current = [
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes(),
            collector_division_templates::extended::division_templates::home_allocated_bytes(
                // SAFETY: The sizing pass uses its session's live shared storage.
                unsafe { shared.template_home() },
            ),
            collector_naval_details::naval_combat_sources::home_allocated_bytes(
                // SAFETY: The sizing pass uses its session's live shared storage.
                unsafe { shared.naval_combat_home() },
            ),
        ];
        for (slot, bytes) in accounted.iter_mut().zip(current) {
            let previous = *slot;
            if bytes != previous {
                *slot = bytes;
                // SAFETY: The writer accounts memory to this same session and drains before it is released.
                unsafe { shared.pool_bytes() }
                    .fetch_add((bytes as u64).wrapping_sub(previous as u64), Ordering::Relaxed);
            }
        }
    }

    fn recycle_frame(&self, shared: &Shared, index: usize) {
        self.recycle_frame_with(shared, index, |surplus| {
            // SAFETY: The frame was removed from its slot after every previous use ended.
            unsafe { shared.free_surplus(surplus) };
        });
    }

    fn recycle_frame_with(
        &self,
        shared: &Shared,
        index: usize,
        free: impl FnOnce(Option<Box<sampling::Captured>>),
    ) {
        // SAFETY: This stage owns the installed frame slot.
        unsafe { shared.frame_stage(index, sampling::FrameStage::Recycle) };
        let mut proposal = *self.capacities.lock();
        if !collector_workers::tick_gate::game_ticking() {
            let live = unsafe { LiveMemory::new() };
            let _ = proposal
                .extended
                .combat_progress
                .presize(&live, shared.base);
        }
        let admitted = {
            let frame = unsafe {
                // SAFETY: This is the runtime's frame pool for the current session.
                shared.frames()[index].captured()
            };
            frame.clock.is_some().then_some(frame.world_generation)
        };
        loop {
            {
                let frame = unsafe {
                    // SAFETY: This is the runtime's frame pool for the current session.
                    shared.frames()[index].captured_mut()
                };
                // SAFETY: Spool recycling holds exclusive `&mut` access to this frame while session
                // observer storage stays live and teardown is excluded; shared observers synchronize
                // concurrent growth.
                unsafe { proposal.grow_frame(frame) };
                if let SeedStatus::Capacity { required_cells } = frame.country_cache_status {
                    // SAFETY: This is the sizing pass for the active session's cache.
                    unsafe { shared.offer_country_cache(required_cells) };
                }
            }
            // SAFETY: This owner has finished using the frame and no other stage or helper borrows
            // it.
            unsafe { shared.prepare_frame_for_reuse(index) };
            self.account_homes(shared);
            #[cfg(test)]
            if let Some(check) = lock(&self.before_recycle_publish).take() {
                check(self, shared);
            }
            let mut capacities = self.capacities.lock();
            self.merge_capacities(&mut capacities, proposal);
            if *capacities == proposal {
                let surplus =
                // SAFETY: This is the last owner of the installed slot and all other uses have
                // ended.
                unsafe { shared.recycle_frame_unfreed(index) };
                if let Some(generation) = admitted
                    && collector_lifetime::world_lifetime::token() == Some(generation)
                {
                    self.settled_world.store(generation, Ordering::Release);
                }
                drop(capacities);
                free(surplus);
                return;
            }
            proposal = *capacities;
            drop(capacities);
        }
    }

    fn with_capacities<R>(
        &self,
        _shared: &Shared,
        step: impl FnOnce(&mut BufferCapacities) -> R,
    ) -> R {
        step(&mut self.capacities.lock())
    }
}

struct SizingThread {
    sizing: Arc<SizingShared>,
    thread: Option<std::thread::JoinHandle<()>>,
    builder: Option<std::thread::JoinHandle<()>>,
}

impl SizingThread {
    fn start(shared: engine_allocation::SharedBox<Shared>, state: SpoolState) -> Self {
        let sizing = Arc::new(SizingShared::new(
            &shared,
            BufferCapacities {
                core: state.sizing.core,
                map: state.sizing.map,
                extended: state.sizing.extended,
            },
            Some(SizingState {
                registry: RegistryWatch::default(),
                applied_combat_hints: state.applied_combat_hints,
                fuel_demand_at: Instant::now(),
                second: EverySecond::default(),
                capacity_census: SizingBackoff::new(),
            }),
        ));
        let builder = {
            let shared = shared.clone();
            let sizing = Arc::clone(&sizing);
            std::thread::Builder::new()
                .name("hoi4-frame-builder".into())
                .stack_size(BACKGROUND_STACK_BYTES)
                .spawn(move || {
                    collector_workers::private_heap::off_hold_path();
                    // SAFETY: Only the current session's designated thread registers this role.
                    unsafe {
                        collector_workers::thread_cpu::register(
                            collector_workers::thread_cpu::Role::FrameBuilder,
                        )
                    };
                    frame_builder(&shared, &sizing);
                })
                .ok()
        };
        let worker = Arc::clone(&sizing);
        let thread = std::thread::Builder::new()
            .name("hoi4-spool-sizing".into())
            .stack_size(BACKGROUND_STACK_BYTES)
            .spawn(move || {
                collector_workers::private_heap::off_hold_path();
                // SAFETY: Only the current session's designated thread registers this role.
                unsafe {
                    collector_workers::thread_cpu::register(
                        collector_workers::thread_cpu::Role::SpoolSizing,
                    )
                };
                sizing_loop(&shared, &worker);
            })
            .ok();
        Self {
            sizing,
            thread,
            builder,
        }
    }

    fn thread_id(&self) -> Option<u32> {
        handle_id(self.thread.as_ref()?)
    }

    fn builder_id(&self) -> Option<u32> {
        handle_id(self.builder.as_ref()?)
    }

    fn spool_pass(
        &self,
        shared: &Shared,
        processor: &mut Processor,
        received: bool,
    ) -> Result<(), &'static str> {
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { collector_workers::spool_phases::SPOOL.span(Phase::Sizing) };
        if let Some(&failure) = self.sizing.failure.get() {
            return Err(failure);
        }
        let pending = std::mem::take(&mut *lock(&self.sizing.pending));
        if let Some(reason) = pending.last_error {
            processor.observe_input_demand_errors(pending.errors, reason);
        }
        *lock(&self.sizing.context) = collector_memory::terminal::context();
        span.finish();
        match &self.thread {
            Some(thread) if received || collector_lifetime::world_lifetime::invalidating() => {
                if received && shared.needs_steady_park() {
                    shared.detach_idle_steady_frame();
                }
                self.sizing.pass_requested.store(true, Ordering::Release);
                if collector_lifetime::world_lifetime::invalidating()
                    || self.sizing.hint_demand.load(Ordering::Relaxed)
                    || self.sizing.idle_growth_requested.load(Ordering::Relaxed)
                    || shared.has_retired_parked()
                    || shared.needs_steady_park()
                {
                    thread.thread().unpark();
                }
            }
            Some(_) => {}
            None => {
                if let Some(state) = lock(&self.sizing.state).as_mut() {
                    let _gated = collector_workers::tick_gate::gated();
                    sizing_pass(
                        shared,
                        &self.sizing,
                        state,
                        &collector_workers::spool_phases::SPOOL,
                        false,
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl SizingThread {
    fn retire(&self, retired: Vec<replay_frame::frame::Retired>) {
        if retired.is_empty() {
            return;
        }
        let Some(thread) = &self.thread else {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { collector_workers::spool_phases::SPOOL.span(Phase::Retire) };
            drop(retired);
            return;
        };
        let mut held = lock(&self.sizing.retired);
        if held.is_empty() {
            *held = retired;
        } else if engine_allocation::reserve(&mut held, retired.len()).is_ok() {
            held.extend(retired);
        } else {
            drop(held);
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { collector_workers::spool_phases::SPOOL.span(Phase::Retire) };
            drop(retired);
            return;
        }
        let wake = held.len() >= 4
            || held
                .iter()
                .map(replay_frame::frame::Retired::len)
                .sum::<usize>()
                >= 64;
        drop(held);
        if wake {
            thread.thread().unpark();
        }
    }

    fn retire_parked(&self, shared: &Shared) {
        if !shared.has_retired_parked() {
            return;
        }
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        } else {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { collector_workers::spool_phases::SPOOL.span(Phase::Retire) };
            shared.drain_retired_parked();
        }
    }
}

impl Drop for SizingThread {
    fn drop(&mut self) {
        #[cfg(feature = "worker_sampler")]
        if self.thread.is_some() {
            // SAFETY: This owned sampled worker completes the exit handshake before leaving.
            unsafe { collector_workers::worker_sampler_state::stop_before_thread_exit() };
        }
        self.sizing.stop.store(true, Ordering::Release);
        for thread in [self.thread.take(), self.builder.take()]
            .into_iter()
            .flatten()
        {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn handle_id(thread: &std::thread::JoinHandle<()>) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    // SAFETY: the handle stays open while the JoinHandle lives.
    let id =
        unsafe { windows_sys::Win32::System::Threading::GetThreadId(thread.as_raw_handle().cast()) };
    (id != 0).then_some(id)
}

fn frame_builder(shared: &Shared, sizing: &SizingShared) {
    frame_builder_with_grace(shared, sizing, Duration::from_millis(6), |_| {});
}

#[cfg(test)]
fn frame_builder_observing(
    shared: &Shared,
    sizing: &SizingShared,
    observe: impl FnMut(sampling::BuildAhead),
) {
    frame_builder_with_grace(shared, sizing, Duration::ZERO, observe);
}

fn frame_builder_with_grace(
    shared: &Shared,
    sizing: &SizingShared,
    grace: Duration,
    observe: impl FnMut(sampling::BuildAhead),
) {
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
    let mut priority =
    // SAFETY: This is the dedicated worker applying its own session's priority policy.
    unsafe { OwnPriority::frame_pipeline() };
    let built =
    // SAFETY: This is the session's designated builder, which the runtime joins before releasing
    // Shared.
    unsafe { shared.build_ahead_until_with_grace(&sizing.stop, grace, || {
        priority.follow_pipeline(
            shared.frame_priority(),
            collector_workers::frame_priority::Role::Builder,
        ) ;
        collector_memory::terminal::set_context(*lock(&sizing.context));
    }, observe) };
    if let Err(error) = built {
        let _ = sizing
            .failure
            .set(error_reason(error, "frame allocation failed"));
        shared.wake_spool();
    }
}

fn sizing_loop(shared: &Shared, sizing: &SizingShared) {
    #[cfg(feature = "worker_sampler")]
    // SAFETY: This runtime-owned worker keeps the guard until it exits.
    let _sampler_exit = unsafe { collector_workers::thread_cpu::StopOnThreadExit::new() };
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
    let Some(mut state) = lock(&sizing.state).take() else {
        return;
    };
    let phases = &collector_workers::spool_phases::SIZING;
    let mut priority =
    // SAFETY: This is the dedicated worker applying its own session's priority policy.
    unsafe { OwnPriority::frame_pipeline() };
    // SAFETY: This owned worker updates its priority from its session's own pipeline.
    unsafe {
        priority.follow_pipeline(
            // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
            shared.frame_priority(),
            collector_workers::frame_priority::Role::Sizing,
        )
    };
    let mut next_pass = Instant::now();
    let mut last_pass = Instant::now() - SIZING_IDLE_WAIT;
    let mut last_reclaim = Instant::now();
    let mut followup_reclaims: u8 = 2;
    let mut last_freed =
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.freed_count();
    let mut steady_wait = SIZING_IDLE_WAIT;
    let mut seen_version = sizing.capacities_version.load(Ordering::Acquire);
    while !sizing.stop.load(Ordering::Acquire) && !collector_memory::terminal::failed() {
        // SAFETY: This owned worker updates its priority from its session's own pipeline.
        unsafe {
            priority.follow_pipeline(
                // SAFETY: The runtime owns this session's worker policy and keeps it live through join.
                shared.frame_priority(),
                collector_workers::frame_priority::Role::Sizing,
            )
        };
        collector_memory::terminal::set_context(*lock(&sizing.context));
        let mut reclaim_now = false;
        let retired = std::mem::take(&mut *lock(&sizing.retired));
        if !retired.is_empty() {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { phases.span(Phase::Retire) };
            drop(retired);
        }
        if shared.has_retired_parked() {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { phases.span(Phase::Retire) };
            reclaim_now |= shared.drain_retired_parked();
        }
        if shared.drain_surplus_parked() {
            reclaim_now = true;
        }
        if shared.needs_steady_park() && shared.park_idle_steady_frame() {
            reclaim_now = true;
        }
        let freed =
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }.freed_count();
        if freed != last_freed {
            last_freed = freed;
            reclaim_now = true;
        }
        let reclaim_interval = if followup_reclaims > 0 {
            Duration::from_millis(750)
        } else {
            Duration::from_secs(8)
        };
        if reclaim_now || last_reclaim.elapsed() >= reclaim_interval {
            last_reclaim = Instant::now();
            if reclaim_now {
                followup_reclaims = followup_reclaims.max(1);
            } else {
                followup_reclaims = followup_reclaims.saturating_sub(1);
            }
            collector_workers::private_heap::PRIVATE_HEAP.reclaim_empty_slabs();
        }
        let now = Instant::now();
        let due = now >= next_pass;
        let urgent = collector_lifetime::world_lifetime::invalidating()
            || sizing.hint_demand.load(Ordering::Acquire)
            || sizing.idle_growth_requested.load(Ordering::Acquire);
        let requested = sizing.pass_requested.load(Ordering::Acquire)
            && (urgent || last_pass.elapsed() >= steady_wait);
        if due || urgent || requested {
            sizing.pass_requested.store(false, Ordering::Release);
            last_pass = now;
            if let Err(failure) = sizing_pass(shared, sizing, &mut state, phases, true) {
                let _ = sizing.failure.set(failure);
                shared.wake_spool();
                return;
            }
            let version = sizing.capacities_version.load(Ordering::Acquire);
            let still_urgent = urgent
                || version != seen_version
                || collector_lifetime::world_lifetime::invalidating()
                || sizing.hint_demand.load(Ordering::Acquire)
                || sizing.idle_growth_requested.load(Ordering::Acquire);
            seen_version = version;
            steady_wait = if still_urgent {
                SIZING_IDLE_WAIT
            } else {
                (steady_wait * 2).min(SIZING_MAX_WAIT)
            };
            next_pass = Instant::now()
                + if collector_lifetime::world_lifetime::invalidating() {
                    SPOOL_IDLE_WAIT
                } else {
                    steady_wait
                };
        }
        let _span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { phases.span(Phase::Wait) };
        let next_reclaim = last_reclaim
            + if followup_reclaims > 0 {
                Duration::from_millis(750)
            } else {
                Duration::from_secs(3)
            };
        let wake_at = next_pass.min(next_reclaim);
        std::thread::park_timeout(wake_at.saturating_duration_since(Instant::now()));
    }
}

fn sizing_pass(
    shared: &Shared,
    sizing: &SizingShared,
    state: &mut SizingState,
    phases: &'static collector_workers::spool_phases::Steps,
    background: bool,
) -> Result<(), &'static str> {
    let live = unsafe { LiveMemory::new() };
    let base = shared.base;
    let registry = &mut state.registry;
    let loading = collector_lifetime::world_lifetime::invalidating();
    let hint_demand = sizing.hint_demand.swap(false, Ordering::AcqRel) || loading;
    let periodic_interval = if hint_demand {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(8)
    };
    let once_a_second = state.second.due_after(Instant::now(), periodic_interval);
    let capacity_census = state.capacity_census.due(
        Instant::now(),
        collector_lifetime::world_lifetime::generation(),
        hint_demand,
    ) && once_a_second;
    let span =
    // SAFETY: This worker records phases on the counters assigned to its spool role.
    unsafe { phases.span(Phase::Sizing) };
    if (capacity_census || registry.last_logistics.is_none())
        && let Some(logistics_rows) = registry
            .poll_logistics(&live, base, Instant::now)
            .map_err(|e| error_reason(e, "logistics balance staging allocation failed"))?
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.logistics.raise(logistics_rows)
        });
    }
    let span = span.switch(Phase::Observers);
    collector_air_relations::relation_changes::collect();
    collector_observers::world_input_changes::collect();
    collector_air_histories::history_changes::collect();
    collector_intel::flag_changes::presize();
    collector_intel::flag_changes::collect();
    collector_construction::building_changes::presize();
    collector_construction::building_changes::collect();
    // SAFETY: The session's sole serial sizing pass runs while observer storage stays live.
    unsafe { collector_observers::advisor_pool_changes::presize() };
    collector_observers::advisor_pool_changes::collect();
    collector_governance::party_leader_changes::presize();
    collector_governance::party_leader_changes::collect();
    collector_intel::variable_changes::presize();
    collector_intel::variable_changes::collect();
    collector_governance::party_changes::presize();
    collector_governance::party_changes::collect();
    collector_observers::wing_changes::presize();
    collector_observers::wing_changes::collect();
    // SAFETY: The session's sole serial sizing pass runs while mission watches stay live.
    unsafe { collector_missions::mission_changes::presize() };
    collector_missions::mission_changes::collect();
    collector_observers::rail_changes::presize();
    collector_observers::rail_changes::collect();
    collector_observers::territory_changes::presize();
    collector_observers::territory_changes::collect();
    collector_division_parts::army_stat_changes::presize();
    collector_division_parts::army_stat_changes::collect();
    collector_observers::country_input_changes::presize();
    collector_observers::country_input_changes::collect();
    collector_production::production::design_changes::presize();
    collector_observers::raid_changes::presize();
    collector_observers::raid_changes::collect();
    collector_observers::naval_tree_changes::presize();
    collector_observers::naval_tree_changes::collect();
    collector_observers::unit_leader_changes::presize();
    collector_observers::unit_leader_changes::collect();
    collector_observers::orders_group_changes::presize();
    collector_observers::orders_group_changes::collect();
    collector_naval_management::changes::collect();
    collector_observers::opinion_changes::collect();
    collector_observers::retreat_ban_changes::presize();
    collector_observers::retreat_ban_changes::collect();
    collector_observers::unit_leader_trait_changes::presize();
    collector_observers::unit_leader_trait_changes::collect();
    collector_model_inputs::local_subunit_changes::presize();
    collector_model_inputs::local_subunit_changes::collect();
    collector_divisions::province_unit_changes::presize();
    collector_divisions::province_unit_changes::collect();
    collector_observers::province_combat_changes::presize();
    collector_observers::province_combat_changes::collect();
    // SAFETY: The session's sole serial sizing pass offers watch storage while watches stay live.
    unsafe { collector_factions::faction_goal_changes::presize() };
    collector_factions::faction_goal_changes::collect();
    // SAFETY: The session's sole serial sizing pass offers watch storage while watches stay live.
    unsafe { collector_factions::faction_companion_changes::presize() };
    collector_factions::faction_companion_changes::collect();
    // SAFETY: The session's sole serial sizing pass offers watch storage while watches stay live.
    unsafe { collector_division_templates::template_changes::presize() };
    collector_division_templates::template_changes::collect();
    collector_ship_models::ship_model_changes::presize();
    collector_ship_models::ship_model_changes::collect();
    collector_observers::air_base_carry_changes::presize();
    collector_observers::air_base_carry_changes::collect();
    collector_observers::character_changes::presize(
        collector_characters::extended::characters::census::initial_len(),
    );
    collector_observers::character_changes::collect();
    // SAFETY: The session's sole serial sizing pass prepares live observer logs before detach.
    unsafe {
        collector_politics::politics_changes::prepare();
        collector_observers::air_losses::presize(0);
        collector_observers::building_bombing::presize(0);
        collector_observers::combat_engagement_refreshes::presize(0);
    }
    collector_observers::nuclear_strikes::presize(0);
    // SAFETY: The session's sole serial sizing pass prepares live observer logs before detach.
    unsafe {
        collector_observers::market_changes::presize(
            0,
            collector_primitives::country_slots::census_rows(0),
        );
    }
    // SAFETY: The session's sole serial sizing pass presizes live map-event lanes before detach.
    if let Err(reason) = unsafe {
        collector_map_events::presize(collector_primitives::country_slots::census_rows(0))
    } {
        sizing.error(reason);
    }
    // SAFETY: The session's sole serial sizing pass prepares live land and history-event logs.
    unsafe {
        collector_land_events::land_events::prepare();
        collector_combat_log::history_events::prepare(0);
    }
    let air_loss_capacity = collector_observers::air_losses::capacity();
    let bombing_capacity = collector_observers::building_bombing::capacity();
    let engagement_refresh_capacity = collector_observers::combat_engagement_refreshes::capacity();
    let nuclear_capacity = collector_observers::nuclear_strikes::capacity();
    let land_event_capacity = collector_land_events::land_events::capacity();
    sizing.raise(shared, |capacities| {
        let extended = &mut capacities.extended;
        extended.air_losses.events = extended.air_losses.events.max(air_loss_capacity);
        extended.building_bombing = extended.building_bombing.max(bombing_capacity);
        extended.combat_engagement_refreshes = extended
            .combat_engagement_refreshes
            .max(engagement_refresh_capacity);
        extended.nuclear_strikes = extended.nuclear_strikes.max(nuclear_capacity);
        extended.land_events.words = extended.land_events.words.max(land_event_capacity);
    });
    let operations =
        sizing.with_capacities(shared, |capacities| capacities.extended.operation_outcomes);
    // SAFETY: The session's sole serial sizing pass prepares live operation-log storage.
    unsafe { operations.prepare() };
    collector_experience_gains::experience_gains::prepare(registry.slots);
    collector_output::line_output::collect();
    collector_output::construction_ic::collect();
    collector_fuel_awards::fuel_awards::collect();
    let span = span.switch(Phase::Sizing);
    if state.fuel_demand_at.elapsed() >= RegistryWatch::INTEL_INTERVAL {
        state.fuel_demand_at = Instant::now();
        if let Some(entries) = collector_capture::country_scale::observe_politics_demand(
            &live,
            base,
            registry.slots,
            1,
        ) {
            // SAFETY: The session's sole serial sizing pass resizes live politics staging.
            unsafe { collector_politics::politics_changes::resize(entries.min(8192), 0) };
        }
        collector_fuel_awards::fuel_awards::presize(&live, base);
    }
    if capacity_census {
        match collector_capture::country_scale::observe_fuel_unit_demand(&live, base) {
            Ok(Some(demand)) => sizing.raise(shared, |capacities| {
                // SAFETY: This attachment's sizing pass owns the live observer demand.
                unsafe { capacities.extended.economy_mut() }
                    .demand
                    .raise(demand)
            }),
            Ok(None) => {}
            Err(reason) => sizing.error(reason),
        }
    }
    let (entries, names) = collector_fuel_awards::fuel_awards::drained_capacities();
    sizing.raise(shared, |capacities| {
        let awards = &mut capacities.extended.fuel_awards;
        awards.entries = awards.entries.max(entries);
        awards.names = awards.names.max(names);
    });
    let span = span.switch(Phase::Observers);
    collector_peace::peace_settlements::collect();
    let span = span.switch(Phase::Sizing);
    if once_a_second {
        let peace = collector_peace::peace_settlements::capacity_for_world(&live, base);
        sizing.raise(shared, |capacities| {
            let extended = &mut capacities.extended;
            if !extended.peace_settlements.contains(peace) || !extended.peace_bids.contains(peace) {
                extended.peace_settlements = extended.peace_settlements.merge(peace);
                extended.peace_bids = extended.peace_bids.merge(peace);
            }
        });
    }
    let span = span.switch(Phase::Observers);
    let peace = sizing.capacities.lock().extended.peace_settlements;
    collector_peace::peace_settlements::presize(peace);
    if collector_extended_support::outcomes::records_any(
        shared.capture_mode,
        &[replay_data::rows::TableId::EventFeedHistory],
    ) {
        collector_observers::event_feed_observer::presize(
            collector_primitives::country_slots::census_rows(0),
        );
    }
    let span = span.switch(Phase::Sizing);
    if once_a_second {
        collector_naval_results::naval_result_sources::prepare(&live, base);
        let naval_result_capacity = collector_naval_results::naval_result_sources::demand();
        sizing.raise(shared, |capacities| {
            let sources = &mut capacities.extended.naval_result_sources;
            *sources = (*sources).max(naval_result_capacity);
        });
    }
    if (capacity_census || registry.last_lines.is_none())
        && let Some(lines) = registry.poll_lines(&live, base)
    {
        collector_output::line_output::presize(lines);
        sizing.raise(shared, |capacities| {
            let entries = &mut capacities.extended.line_output.entries;
            *entries = (*entries).max(lines);
        });
    }
    if once_a_second
        && let Ok(bytes) = collector_weather::weather::observe_display_capacity(&live, base)
    {
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
            // SAFETY: This session's sizing owner is preparing live decode pools before teardown.
            unsafe { decode::presize_weather_jobs() };
        }
    }
    if let Some((slots, regions)) = registry.poll(&live, base) {
        collector_primitives::country_slots::note_slots(slots);
        // SAFETY: The session's sole serial sizing pass resizes live operation and politics logs.
        unsafe {
            collector_operation_outcomes::operation_outcomes::resize(slots.max(1));
            collector_politics::politics_changes::resize(
                collector_capture::country_scale::observe_politics_demand(
                    &live,
                    base,
                    slots,
                    1,
                )
                .unwrap_or(slots)
                .min(8192),
                sizing
                    .demand(engine_events::politics_sites::lexical_key_demand(
                        &live, base,
                    ))
                    .unwrap_or(0),
            );
        }
        collector_observers::country_input_changes::presize_countries(slots);
        collector_air_relations::relation_changes::presize(slots);
        collector_observers::world_input_changes::presize(slots);
        collector_air_histories::history_changes::presize(slots, regions);
        // SAFETY: This session's sizing owner is preparing live decode pools before teardown.
        unsafe { decode::presize_country_pools() };
        let country_cells = sizing.raise(shared, |capacities| {
            let mut raised = collector_capture::country_scale::Sizing {
                core: capacities.core,
                map: capacities.map,
                extended: capacities.extended,
                country_cells: 0,
                content: engine_dlc::cdlc_manager::Counts::default(),
            };
            raised.raise_for_slots(slots);
            raised.extended.raise_for_regions(regions);
            capacities.core = raised.core;
            capacities.map = raised.map;
            capacities.extended = raised.extended;
            raised.country_cells
        });
        // SAFETY: This is the sizing pass for the active session's cache.
        unsafe { shared.offer_country_cache(country_cells) };
    }
    if once_a_second {
        if let Err(reason) = collector_output::construction_ic::presize_from_live(&live, base) {
            sizing.error(reason);
        }
        // SAFETY: The session's sole serial sizing pass prepares live observer storage against the
        // authenticated session image before detach.
        unsafe {
            collector_observers::air_losses::prepare(&live, base, registry.slots);
            collector_observers::building_bombing::prepare(&live, base, registry.slots);
            collector_observers::combat_engagement_refreshes::presize(registry.slots.max(64));
        }
        collector_observers::nuclear_strikes::presize(registry.slots.max(64));
        let source =
        // SAFETY: Session sizing or the isolated fixture owns the census and its observer demand.
        unsafe { collector_capture::country_scale::source_demand::Demand::observe_periodic(
            &live,
            base,
            capacity_census,
        ) };
        let air_loss_capacity = collector_observers::air_losses::capacity();
        let bombing_capacity = collector_observers::building_bombing::capacity();
        let engagement_refresh_capacity =
            collector_observers::combat_engagement_refreshes::capacity();
        // SAFETY: The session's sole serial sizing pass presizes live market observer storage.
        unsafe { collector_observers::market_changes::presize(0, registry.slots) };
        let market_capacity = collector_observers::market_changes::capacity();
        let nuclear_capacity = collector_observers::nuclear_strikes::capacity();
        sizing.raise(shared, |capacities| {
            let extended = &mut capacities.extended;
            extended.air_losses.events = extended.air_losses.events.max(air_loss_capacity);
            extended.building_bombing = extended.building_bombing.max(bombing_capacity);
            extended.combat_engagement_refreshes = extended
                .combat_engagement_refreshes
                .max(engagement_refresh_capacity);
            extended.nuclear_strikes = extended.nuclear_strikes.max(nuclear_capacity);
            extended.market = extended.market.max(market_capacity);
            if let Ok(Some(demand)) = &source {
                if demand.refused() {
                    sizing.hint_demand.store(true, Ordering::Release);
                }
                demand.apply_parts(&mut capacities.core, &mut capacities.extended);
            }
        });
        if let Err(reason) = source {
            sizing.error(reason);
        }
        sizing.raise_from_live_observation(shared, |capacities| {
            [
                // SAFETY: The serialized sizing pass owns the observer storage and excludes
                // teardown.
                unsafe {
                    collector_capture::country_scale::table_demand::presize_periodic(
                        &live,
                        base,
                        &mut capacities.map,
                        &mut capacities.extended,
                        capacity_census,
                    )
                }
                .err(),
                // SAFETY: This is the session's sizing pass, with its observer storage still live.
                unsafe {
                    capacities
                        .extended
                        .presize_live_inputs_periodic(&live, base, capacity_census)
                }
                .err(),
            ]
        });
        if collector_lifetime::world_lifetime::token().is_some() {
            // SAFETY: Periodic sizing runs for the active session against the authenticated `base`
            // while naval-management watch storage stays live and teardown is excluded.
            unsafe { collector_naval_management::changes::presize(&live, base) };
        }
        let destruction_demand = collector_equipment_destruction::equipment_destruction::demand();
        sizing.raise(shared, |capacities| {
            let destruction = &mut capacities.extended.equipment_destruction;
            *destruction = (*destruction).max(destruction_demand);
        });
    }
    if capacity_census && let Some(rows) = sizing.demand(registry.poll_intel(&live, base)).flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_for_country_intel(rows)
        });
    }
    if capacity_census
        && let Some(demand) = sizing
            .demand(registry.poll_ace_names(&live, base))
            .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.air.raise_for_names(demand)
        });
    }
    if capacity_census
        && let Some(bytes) = sizing
            .demand(registry.poll_air_wing_names(&live, base))
            .flatten()
    {
        collector_air::sizing::note_air_wing_name_bytes(bytes);
        // SAFETY: This session's sizing owner is preparing live decode pools before teardown.
        unsafe { decode::presize_air_wing_names() };
        sizing.raise(shared, |capacities| {
            // SAFETY: The sizing pass uses this session's measured name demand.
            unsafe { capacities.map.raise_for_air_wing_names(bytes) }
        });
    }
    if capacity_census
        && let Some(demand) = sizing
            .demand(registry.poll_railway_gun_names(&live, base))
            .flatten()
    {
        collector_railway_guns::railway_guns::note_railway_gun_names(demand);
        sizing.raise(shared, |capacities| {
            capacities.extended.railway_guns.raise_for_names(demand)
        });
    }
    if (capacity_census || registry.last_map.is_none())
        && let Some(demand) = sizing.demand(registry.poll_map(&live, base)).flatten()
    {
        if let Ok(carrier) = demand.carrier
            && collector_air::sizing::note_carrier_demand(carrier)
        {
            // SAFETY: This session's sizing owner is preparing live decode pools before teardown.
            unsafe { decode::presize_country_pools() };
        }
        for reason in demand.map_refusals() {
            sizing.error(reason);
        }
        let port_failure = sizing.raise(shared, |capacities| {
            let mut raised = collector_capture::country_scale::Sizing {
                core: capacities.core,
                map: capacities.map,
                extended: capacities.extended,
                country_cells: 0,
                content: engine_dlc::cdlc_manager::Counts::default(),
            };
            raised.raise_for_map(&demand);
            raised
                .extended
                .country_counters
                .raise_for_physical_navies(demand.navies.capacities);
            // SAFETY: This attachment's sizing pass owns the live observer demand.
            unsafe { raised.extended.naval_counters_mut() }
                .raise_for_names(demand.navies.name_rows, demand.navies.name_bytes);
            // SAFETY: This attachment's sizing pass owns the live observer demand.
            unsafe { raised.extended.naval_counters_mut() }
                .raise_for_supply_origins(demand.navies.supply_origins);
            let failure =
            // SAFETY: This attachment's sizing pass owns the live observer demand.
            unsafe { raised.extended.naval_counters_mut() }
                .raise_for_port(demand.navies.port)
                .err();
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
    if capacity_census
        && let Some(demand) = sizing
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
    let game_score = sizing.capacities.lock().extended.game_score.enabled;
    if capacity_census
        && game_score
        && let Some(observed) = registry.poll_game_score(&live, base)
    {
        let observed =
            observed.map_err(|error| error_reason(error, "game score sizing allocation failed"))?;
        let demand = observed.demand;
        if sizing.raise(shared, |capacities| {
            capacities.extended.game_score.raise(demand)
        }) {
            // SAFETY: This session's sizing owner is preparing live decode pools before teardown.
            unsafe { decode::presize_country_pools() };
        }
    }
    if capacity_census
        && let Some(demand) = sizing.demand(registry.poll_raids(&live, base)).flatten()
    {
        for reason in demand.failures.into_iter().flatten() {
            sizing.error(reason);
        }
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_for_raid_arenas(demand)
        });
    }
    if (capacity_census || registry.last_characters.is_none())
        && let Some(demand) = sizing
            .demand(registry.poll_characters(&live, base))
            .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_for_characters(demand)
        });
    }
    if capacity_census
        && let Some(demand) = sizing
            .demand(registry.poll_saved_targets(&live, base))
            .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.saved_event_targets.raise(demand)
        });
    }
    if (capacity_census || registry.last_politics.is_none())
        && let Some(demand) = sizing.demand(registry.poll_politics(&live, base)).flatten()
    {
        for reason in demand.failures() {
            sizing.error(reason);
        }
        sizing.raise(shared, |capacities| {
            capacities
                .extended
                .country_counters
                .raise_for_pride_names(demand.pride_name_bytes);
            // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand
            // update.
            unsafe { capacities.extended.raise_for_politics(demand) };
        });
        let operations =
            sizing.with_capacities(shared, |capacities| capacities.extended.operation_outcomes);
        // SAFETY: Background sizing runs while attached with `operation_outcomes::release` excluded.
        unsafe { operations.prepare() };
    }
    if capacity_census
        && let Some(slots) = sizing
            .demand(registry.poll_reserved_visibility(&live, base))
            .flatten()
    {
        sizing.raise(shared, |capacities| {
            capacities.extended.raise_reserved_visibility(slots)
        });
    }
    if background {
        sizing.grow_idle(shared);
    }
    if background
        && capacity_census
        && let Some(hints) = registry.poll_combat_hints(&live, base)
    {
        for reason in hints.failures.into_iter().flatten() {
            sizing.error(reason);
        }
        let capacities = sizing.with_capacities(shared, |capacities| capacities.extended);
        let refused =
            grow_combat_hints(shared, &capacities, &hints, &mut state.applied_combat_hints);
        if refused {
            sizing.hint_demand.store(true, Ordering::Release);
        }
    }
    span.finish();
    let demand = sizing.hint_demand.swap(false, Ordering::AcqRel) || hint_demand;
    if capacity_census {
        let settled = collector_lifetime::world_lifetime::token()
            == Some(sizing.settled_world.load(Ordering::Acquire));
        state
            .capacity_census
            .observed(Instant::now(), demand, settled);
    } else if demand {
        state.capacity_census.reset();
    }
    if once_a_second {
        state.second.done(Instant::now());
    }
    Ok(())
}

const SPOOL_IDLE_WAIT: Duration = Duration::from_millis(10);
const SIZING_IDLE_WAIT: Duration = RegistryWatch::INTERVAL;
const SIZING_MAX_WAIT: Duration = Duration::from_secs(4);

#[derive(Default)]
struct RefreshRequests {
    accepted: Vec<(String, bool)>,
}

pub(crate) const REFRESH_REQUESTS: &str = "refresh-requests";

impl RefreshRequests {
    fn poll(&mut self, session: &Path, mut accept: impl FnMut()) -> io::Result<()> {
        let directory = session.join(REFRESH_REQUESTS);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.accepted.clear();
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let mut tokens = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension() == Some(std::ffi::OsStr::new("request")) {
                let token = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
                tokens.push(token.to_owned());
            }
        }
        self.accepted.retain(|(token, _)| tokens.contains(token));
        for token in tokens {
            let at = match self.accepted.iter().position(|(known, _)| *known == token) {
                Some(at) => at,
                None => {
                    accept();
                    REFRESH_ACCEPTED.fetch_add(1, Ordering::Relaxed);
                    self.accepted.push((token.clone(), false));
                    self.accepted.len() - 1
                }
            };
            if self.accepted[at].1 {
                continue;
            }
            fs::write(directory.join(format!("{token}.ack")), token.as_bytes())?;
            self.accepted[at].1 = true;
            match fs::remove_file(directory.join(format!("{token}.request"))) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }
}

static REFRESH_ACCEPTED: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[derive(Default)]
struct HistoryReplies {
    last: Option<String>,
    pending: Option<HistoryReply>,
}

#[cfg(test)]
impl HistoryReplies {
    fn poll_reply(
        &mut self,
        text: Option<&str>,
        begin: impl FnOnce(&str) -> HistoryReply,
        acknowledge: impl FnOnce(&str) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(text) = text {
            let token = text.split_whitespace().next().unwrap_or_default();
            if !token.is_empty() && self.last.as_deref() != Some(token) {
                self.pending = Some(begin(text));
                self.last = Some(token.to_owned());
                if matches!(self.pending.as_ref(), Some(HistoryReply::Closing { .. })) {
                    return Ok(());
                }
            }
        }
        let ack = match self.pending.as_ref() {
            Some(HistoryReply::Ready(ack)) => ack.clone(),
            Some(HistoryReply::Closing { token, receipt }) => match receipt.result() {
                None => return Ok(()),
                Some(Ok(())) => format!("{token} ok"),
                Some(Err(_)) => format!("{token} refused its segment could not be closed"),
            },
            None => return Ok(()),
        };
        acknowledge(&ack)?;
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod equipment_destruction_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_slow_spool_s_backlog_is_in_the_status_and_clears_when_it_catches_up() {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let status = |shared: &Shared| serde_json::to_value(acquisition_work(shared)).unwrap();
        let steady = collector_capture::frame_pool::STEADY as u64;
        let quiet = status(&shared);
        assert_eq!(quiet["frames"]["steady"], steady);
        assert_eq!(quiet["frames"]["in_flight"], 0);
        assert_eq!(quiet["frames"]["live"], steady);
        assert_eq!(quiet["buffer_wait_calls"], 0);
        assert!(quiet["waits"]["allocator"]["hold"]["heap"]["calls"].is_u64());
        assert!(quiet["waits"]["allocator"]["other"]["heap"]["calls"].is_u64());
        let hours = (2 * collector_capture::frame_pool::STEADY + 1).min(
            collector_capture::frame_pool::MAX_LIVE_FRAMES - collector_capture::frame_pool::AHEAD,
        );
        let taken: Vec<_> = (0..hours)
            .map(|_| {
                let index =
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared.take_frame(&mut 0) }.unwrap();

                // SAFETY: The producer owns this slot and its initialized decode job.
                unsafe { shared.frames().push_decode(index) };
                index
            })
            .collect();
        let behind = status(&shared);
        assert_eq!(behind["frames"]["in_flight"], hours as u64);
        assert_eq!(behind["frames"]["live"], hours as u64);
        assert_eq!(behind["frames"]["free"], 0);
        assert_eq!(behind["frames"]["grown"], hours as u64 - steady);
        let bytes: u64 = taken
            .iter()
            .map(|&index|
                // SAFETY: This is the runtime's frame pool for the current session.
                unsafe { shared.frames() }.slot(index).bytes.load(Ordering::Relaxed))
            .sum();
        assert!(bytes > 0);
        assert_eq!(behind["frames"]["in_flight_bytes"], bytes);
        assert_eq!(behind["frames"]["live_bytes"], bytes);
        assert_eq!(behind["frames"]["built_ahead"], 0);
        for _ in 0..collector_capture::frame_pool::AHEAD {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.build_frame_ahead() }.unwrap();
        }
        let ahead = status(&shared);
        let built = collector_capture::frame_pool::AHEAD as u64;
        assert_eq!(ahead["frames"]["built_ahead"], built);
        assert_eq!(ahead["frames"]["ahead_ready"], built);
        assert_eq!(ahead["frames"]["free"], built);
        let index =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap();

        // SAFETY: The producer owns this slot and its initialized decode job.
        unsafe { shared.frames().push_decode(index) };
        let taken_ahead = status(&shared);
        assert_eq!(taken_ahead["frames"]["ahead_taken"], 1);
        assert_eq!(taken_ahead["frames"]["ahead_ready"], built - 1);
        assert_eq!(taken_ahead["frames"]["grown"], hours as u64 - steady);
        while let Some(index) =
            // SAFETY: The sole decode consumer owns this queue and takes responsibility for the
            // slot.
            unsafe { shared.frames().pop_decode() }
        {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame(index) };
        }
        let caught_up = status(&shared);
        assert_eq!(caught_up["frames"]["in_flight"], 0);
        assert_eq!(caught_up["frames"]["in_flight_bytes"], 0);
        assert_eq!(caught_up["frames"]["freed"], hours as u64 + built - steady);
        assert_eq!(caught_up["frames"]["live"], steady);
        assert_eq!(caught_up["frames"]["free"], steady);
        assert_eq!(caught_up["frames"]["ahead_ready"], built - 1);
    }

    #[test]
    fn a_stalled_spool_drops_hours_instead_of_growing_frames() {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let frames = |shared: &Shared| {
            serde_json::to_value(acquisition_work(shared)).unwrap()["frames"].clone()
        };
        let steady = collector_capture::frame_pool::STEADY;
        // SAFETY: This is the sole spool consumer reporting its own stall state.
        unsafe { shared.spool_stalled(true) };
        let taken: Vec<_> = (0..2 * steady + 1)
            .map(|_|
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.take_frame(&mut 0) })
            .collect();
        assert!(
            taken[..steady].iter().all(Option::is_some),
            "the free frames are taken"
        );
        assert!(
            taken[steady..].iter().all(Option::is_none),
            "the rest are dropped"
        );
        assert!(!collector_memory::terminal::failed());
        assert!(!shared.acquisition_failed());
        let stalled = frames(&shared);
        assert_eq!(stalled["live"], steady as u64);
        assert_eq!(stalled["in_flight"], steady as u64);
        assert_eq!(stalled["grown"], 0);
        assert_eq!(stalled["built_ahead"], 0);
        assert_eq!(stalled["dropped_spool_stalled"], steady as u64 + 1);
        // SAFETY: This is the sole spool consumer reporting its own stall state.
        unsafe { shared.spool_stalled(false) };
        assert!(
            // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
            unsafe { shared.take_frame(&mut 0) }.is_some()
        );
        let resumed = frames(&shared);
        assert_eq!(resumed["grown"], 1);
        assert_eq!(resumed["dropped_spool_stalled"], steady as u64 + 1);
    }

    #[test]
    fn an_hour_building_its_own_frame_never_waits_on_the_held_frame_capacities() {
        let shared = Arc::new(
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Shared::new(0, 0, collector_test_support::qpc_frequency()) },
        );
        for _ in 0..collector_capture::frame_pool::STEADY {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.take_frame(&mut 0) }.unwrap();
        }
        let before =
            // SAFETY: These are the owned capacity snapshots for this session's frames.
            unsafe { shared.frame_capacities() }.published();
        let mut held =
            // SAFETY: These are the owned capacity snapshots for this session's frames.
            unsafe { shared.frame_capacities() }.lock();
        held.core[5] += 8;
        let hour = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut grow_ns = 0;
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared.take_frame(&mut grow_ns) }.map(|_| grow_ns)
            })
        };
        let grow_ns = hour.join().unwrap().expect("the hour's own frame");
        assert!(
            // SAFETY: This fixture reads the published capacities for its own frame pool.
            unsafe { shared.frame_capacities() }.published() == before,
            "a change still held is not read"
        );
        drop(held);
        assert_eq!(
            // SAFETY: This fixture reads the published capacities for its own frame pool.
            unsafe { shared.frame_capacities() }.published().core[5],
            before.core[5] + 8
        );
        assert!(grow_ns > 0, "an hour with no free frame builds its own");
        let frames = serde_json::to_value(acquisition_work(&shared)).unwrap()["frames"].clone();
        assert_eq!(frames["grown"], 1);
    }

    fn poll_history_reply(
        requests: &mut HistoryReplies,
        text: Option<&str>,
        reply: Option<HistoryReply>,
        acks: &mut Vec<String>,
    ) {
        requests
            .poll_reply(
                text,
                |_| reply.expect("only a new request begins processing"),
                |ack| {
                    acks.push(ack.to_owned());
                    Ok(())
                },
            )
            .unwrap();
    }

    #[test]
    fn history_close_acknowledgement_waits_for_its_receipt() {
        let mut requests = HistoryReplies::default();
        let receipt = collector_spool::spool_writer::CloseReceipt::pending();
        let mut acks = Vec::new();
        poll_history_reply(
            &mut requests,
            Some("original close campaign 42"),
            Some(HistoryReply::Closing {
                token: "original".to_owned(),
                receipt: receipt.clone(),
            }),
            &mut acks,
        );
        for _ in 0..3 {
            poll_history_reply(&mut requests, None, None, &mut acks);
            assert!(acks.is_empty());
            assert!(requests.pending.is_some());
        }
        // SAFETY: This is the close result from its originating writer, or an isolated receipt
        // owned by the fixture.
        unsafe { receipt.complete(Ok(())) };
        poll_history_reply(&mut requests, None, None, &mut acks);
        assert_eq!(acks, ["original ok"]);
        assert!(requests.pending.is_none());
        poll_history_reply(&mut requests, None, None, &mut acks);
        assert_eq!(acks, ["original ok"]);
    }

    #[test]
    fn history_close_failure_acknowledges_refusal_not_success() {
        let mut requests = HistoryReplies::default();
        let receipt = collector_spool::spool_writer::CloseReceipt::pending();
        let mut acks = Vec::new();
        poll_history_reply(
            &mut requests,
            Some("failed close campaign 42"),
            Some(HistoryReply::Closing {
                token: "failed".to_owned(),
                receipt: receipt.clone(),
            }),
            &mut acks,
        );
        // SAFETY: This is the close result from its originating writer, or an isolated receipt
        // owned by the fixture.
        unsafe { receipt.complete(Err(io::ErrorKind::WriteZero)) };
        poll_history_reply(&mut requests, None, None, &mut acks);
        assert_eq!(acks, ["failed refused its segment could not be closed"]);
        assert!(requests.pending.is_none());
    }

    #[test]
    fn duplicate_history_tokens_do_not_restart_or_discard_a_pending_close() {
        let mut requests = HistoryReplies::default();
        let receipt = collector_spool::spool_writer::CloseReceipt::pending();
        let mut acks = Vec::new();
        poll_history_reply(
            &mut requests,
            Some("same close campaign 42"),
            Some(HistoryReply::Closing {
                token: "same".to_owned(),
                receipt: receipt.clone(),
            }),
            &mut acks,
        );
        poll_history_reply(
            &mut requests,
            Some("same close campaign 42"),
            None,
            &mut acks,
        );
        assert!(acks.is_empty());
        // SAFETY: This is the close result from its originating writer, or an isolated receipt
        // owned by the fixture.
        unsafe { receipt.complete(Ok(())) };
        poll_history_reply(
            &mut requests,
            Some("same close campaign 42"),
            None,
            &mut acks,
        );
        assert_eq!(acks, ["same ok"]);
        poll_history_reply(
            &mut requests,
            Some("same close campaign 42"),
            None,
            &mut acks,
        );
        assert_eq!(acks, ["same ok"]);
    }

    #[test]
    fn a_newer_retake_supersedes_pending_and_completed_history_closes() {
        for result in [Ok(()), Err(io::ErrorKind::WriteZero)] {
            for complete_before_retake in [false, true] {
                let mut requests = HistoryReplies::default();
                let receipt = collector_spool::spool_writer::CloseReceipt::pending();
                let mut acks = Vec::new();
                poll_history_reply(
                    &mut requests,
                    Some("old close campaign 42"),
                    Some(HistoryReply::Closing {
                        token: "old".to_owned(),
                        receipt: receipt.clone(),
                    }),
                    &mut acks,
                );
                if complete_before_retake {
                    // SAFETY: This is the close result from its originating writer, or an isolated
                    // receipt owned by the fixture.
                    unsafe { receipt.complete(result) };
                }
                poll_history_reply(
                    &mut requests,
                    Some("new retake campaign"),
                    Some(HistoryReply::Ready("new ok".to_owned())),
                    &mut acks,
                );
                assert_eq!(acks, ["new ok"]);
                if !complete_before_retake {
                    // SAFETY: This is the close result from its originating writer, or an isolated
                    // receipt owned by the fixture.
                    unsafe { receipt.complete(result) };
                }
                poll_history_reply(&mut requests, None, None, &mut acks);
                assert_eq!(acks, ["new ok"]);
                assert_eq!(requests.last.as_deref(), Some("new"));
                assert!(requests.pending.is_none());
            }
        }
    }

    #[test]
    fn a_newer_history_close_keeps_its_own_receipt() {
        let mut requests = HistoryReplies::default();
        let old = collector_spool::spool_writer::CloseReceipt::pending();
        let new = collector_spool::spool_writer::CloseReceipt::pending();
        let mut acks = Vec::new();
        for (token, receipt) in [("old", &old), ("new", &new)] {
            poll_history_reply(
                &mut requests,
                Some(&format!("{token} close campaign 42")),
                Some(HistoryReply::Closing {
                    token: token.to_owned(),
                    receipt: receipt.clone(),
                }),
                &mut acks,
            );
        }
        // SAFETY: This is the close result from its originating writer, or an isolated receipt
        // owned by the fixture.
        unsafe { old.complete(Ok(())) };
        poll_history_reply(&mut requests, None, None, &mut acks);
        assert!(acks.is_empty());
        // SAFETY: This is the close result from its originating writer, or an isolated receipt
        // owned by the fixture.
        unsafe { new.complete(Ok(())) };
        poll_history_reply(&mut requests, None, None, &mut acks);
        assert_eq!(acks, ["new ok"]);
    }

    #[test]
    fn failed_history_acknowledgement_writes_retry_without_repeating_the_request() {
        for result in [Ok(()), Err(io::ErrorKind::WriteZero)] {
            let mut requests = HistoryReplies::default();
            let receipt = collector_spool::spool_writer::CloseReceipt::pending();
            let mut acks = Vec::new();
            poll_history_reply(
                &mut requests,
                Some("old close campaign 42"),
                Some(HistoryReply::Closing {
                    token: "old".to_owned(),
                    receipt: receipt.clone(),
                }),
                &mut acks,
            );
            // SAFETY: This is the close result from its originating writer, or an isolated receipt
            // owned by the fixture.
            unsafe { receipt.complete(result) };
            let expected = if result.is_ok() {
                "old ok"
            } else {
                "old refused its segment could not be closed"
            };
            let error = requests
                .poll_reply(
                    None,
                    |_| panic!("a pending reply must not repeat the request"),
                    |ack| {
                        assert_eq!(ack, expected);
                        Err(io::ErrorKind::PermissionDenied.into())
                    },
                )
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert!(requests.pending.is_some());
            poll_history_reply(&mut requests, None, None, &mut acks);
            assert_eq!(acks, [expected]);
            assert!(requests.pending.is_none());
        }
    }

    #[test]
    fn newer_ready_history_reply_retries_instead_of_an_older_close() {
        let mut requests = HistoryReplies::default();
        let receipt = collector_spool::spool_writer::CloseReceipt::pending();
        let mut acks = Vec::new();
        poll_history_reply(
            &mut requests,
            Some("old close campaign 42"),
            Some(HistoryReply::Closing {
                token: "old".to_owned(),
                receipt: receipt.clone(),
            }),
            &mut acks,
        );
        let error = requests
            .poll_reply(
                Some("new retake campaign"),
                |_| HistoryReply::Ready("new ok".to_owned()),
                |ack| {
                    assert_eq!(ack, "new ok");
                    Err(io::ErrorKind::PermissionDenied.into())
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        // SAFETY: This is the close result from its originating writer, or an isolated receipt
        // owned by the fixture.
        unsafe { receipt.complete(Ok(())) };
        poll_history_reply(&mut requests, Some("new retake campaign"), None, &mut acks);
        assert_eq!(acks, ["new ok"]);
        assert!(requests.pending.is_none());
    }

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
        code: &collector_hooks::hook::TestCode,
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
                    collector_hooks::hook::TapSafety {
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
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(!collector_memory::terminal::failed());
        {
            let _round = collector_hooks::hook::shared::Round::collect_attach().unwrap();
            collector_hooks::hook::publication::select_family(Some("runtime_classified_overflow"));
            let error =
                collector_hooks::hook::shared::publication_storage_attach::<u8>(usize::MAX, true)
                    .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
            assert!(collector_hooks::hook::shared::take_collect_allocation_failure());
            assert!(!collector_hooks::hook::shared::take_collect_allocation_failure());
            assert!(!collector_memory::terminal::failed());
        }
        let code = collector_hooks::hook::TestCode::new();
        let (_, _, ready_hook) = attach_fixture_hook(&code, 0, None);
        let (_, _, busy_hook) = attach_fixture_hook(&code, 64, Some(&FLIGHT));
        let mut ready = [ready_hook];
        let mut busy = [busy_hook];
        let later_calls = Cell::new(0);
        let progress = attach_test_progress();
        let stage = crate::progress::Stage::new(&progress, "runtime_allocation_ready");
        FLIGHT.store(1, Ordering::SeqCst);
        let suspensions = collector_hooks::hook::SUSPENSIONS.with(Cell::get);
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
                        collector_hooks::hook::shared::publication_storage_attach::<u8>(
                            usize::MAX,
                            true,
                        )
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
        assert!(!collector_memory::terminal::failed());
        assert!(!collector_hooks::hook::shared::take_collect_allocation_failure());
        assert_eq!(
            collector_hooks::hook::SUSPENSIONS.with(Cell::get) - suspensions,
            1
        );
        FLIGHT.store(0, Ordering::SeqCst);
        collector_hooks::hook::publication::select_family(None);
        assert_eq!(Hook::set_family_enabled(&mut ready, false), Ok(true));
    }

    #[test]
    fn attach_collect_refusal_reports_the_earlier_prefix_without_retrying_it() {
        use std::cell::Cell;

        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for prepared in &ATTACH_PREPARATIONS {
            prepared.store(0, Ordering::SeqCst);
        }
        let code = collector_hooks::hook::TestCode::new();
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
        let suspensions = collector_hooks::hook::SUSPENSIONS.with(Cell::get);
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
                        if !collector_hooks::hook::shared::defer_preparation(prepare_attach_middle)
                        {
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
        assert_eq!(first_posts.get(), 2);
        assert_eq!(second_posts.get(), 2);
        assert_eq!(ATTACH_PREPARATIONS[0].load(Ordering::SeqCst), 1);
        assert_eq!(ATTACH_PREPARATIONS[1].load(Ordering::SeqCst), 1);
        assert_eq!(ATTACH_PREPARATIONS[2].load(Ordering::SeqCst), 0);
        assert!(first[0].is_enabled() && second[0].is_enabled());
        assert_eq!(
            collector_hooks::hook::SUSPENSIONS.with(Cell::get) - suspensions,
            1
        );
        collector_hooks::hook::publication::select_family(None);
        for family in [&mut first, &mut second] {
            assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
        }
    }

    #[test]
    fn attach_busy_prefix_precedes_a_later_collect_refusal_and_retries_in_order() {
        use std::cell::Cell;

        static FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let code = collector_hooks::hook::TestCode::new();
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
        let suspensions = collector_hooks::hook::SUSPENSIONS.with(Cell::get);
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
        assert_eq!(later_posts.get(), 2);
        assert!(first[0].is_enabled() && middle[0].is_enabled() && later[0].is_enabled());
        assert_eq!(
            collector_hooks::hook::SUSPENSIONS.with(Cell::get) - suspensions,
            2
        );
        collector_hooks::hook::publication::select_family(None);
        for family in [&mut first, &mut middle, &mut later] {
            assert_eq!(Hook::set_family_enabled(family, false), Ok(true));
        }
    }

    #[test]
    fn attach_runtime_prepares_and_reports_each_family_once_under_one_suspension() {
        use std::cell::Cell;

        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for prepared in &ATTACH_PREPARATIONS {
            prepared.store(0, Ordering::SeqCst);
        }
        let code = collector_hooks::hook::TestCode::new();
        let (_, _, first_hook) = attach_fixture_hook(&code, 0, None);
        let (_, _, middle_hook) = attach_fixture_hook(&code, 64, None);
        let (_, _, later_hook) = attach_fixture_hook(&code, 128, None);
        let mut first = [first_hook];
        let mut middle = [middle_hook];
        let mut later = [later_hook];
        let posts = [Cell::new(0), Cell::new(0), Cell::new(0)];
        let progress = attach_test_progress();
        let stage = crate::progress::Stage::new(&progress, "runtime_prepare_first");
        let suspensions = collector_hooks::hook::SUSPENSIONS.with(Cell::get);
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
            assert_eq!(posted.get(), 2);
        }
        assert_eq!(
            collector_hooks::hook::SUSPENSIONS.with(Cell::get) - suspensions,
            1
        );
        collector_hooks::hook::publication::select_family(None);
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
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for busy in [false, true] {
            for prepared in &ATTACH_PREPARATIONS {
                prepared.store(0, Ordering::SeqCst);
            }
            let code = collector_hooks::hook::TestCode::new();
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
            let suspensions = collector_hooks::hook::SUSPENSIONS.with(Cell::get);
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
                assert_eq!(posts[index].get(), 1 + usize::from(index == 0 || !busy));
            }
            assert!(ready.iter().all(Hook::is_enabled));
            assert_eq!(
                collector_hooks::hook::SUSPENSIONS.with(Cell::get) - suspensions,
                usize::from(busy),
            );
            FLIGHT.store(0, Ordering::SeqCst);
            collector_hooks::hook::publication::select_family(None);
            assert_eq!(Hook::set_family_enabled(&mut ready, false), Ok(true));
            assert_eq!(Hook::set_family_enabled(&mut middle, false), Ok(true));
        }
    }

    impl Moment for Duration {
        fn since(self, earlier: Self) -> Duration {
            self.saturating_sub(earlier)
        }
    }

    fn sizing_fixture() -> (Shared, SizingThread) {
        let sizing = collector_capture::country_scale::Sizing::for_slots(0);
        let shared =
        // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
        // boundary or detach.
        unsafe { Shared::with_fixture_sizing(0, 0, 1, &sizing, 0) }.unwrap();
        let capacities = BufferCapacities {
            core: sizing.core,
            map: sizing.map,
            extended: sizing.extended,
        };
        let thread = SizingThread {
            sizing: Arc::new(SizingShared::new(&shared, capacities, None)),
            thread: None,
            builder: None,
        };
        (shared, thread)
    }

    #[test]
    fn capacity_raises_and_failed_observations_leave_the_census_backed_off() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let take_demand = || sizing.hint_demand.swap(false, Ordering::AcqRel);
        let before = sizing.with_capacities(&shared, |capacities| *capacities);
        sizing.raise(&shared, |capacities| capacities.core[5] += 1);
        sizing.raise_from_live_observation(&shared, |capacities| {
            capacities.core[6] += 1;
            [None; 2]
        });
        sizing.error("synthetic failed observation");
        assert!(!take_demand());
        let after = sizing.with_capacities(&shared, |capacities| *capacities);
        assert_eq!(after.core[5], before.core[5] + 1);
        assert_eq!(after.core[6], before.core[6] + 1);
        assert_eq!(lock(&sizing.pending).errors, 1);
    }

    #[test]
    fn the_spool_records_sizing_observations_once_and_ends_on_a_sizing_failure() {
        let directory =
            std::env::temp_dir().join(format!("hoi4-sizing-handoff-{}", std::process::id()));
        let mut processor =
        // SAFETY: The isolated test owns the spool's failure state.
        unsafe { collector_spool::spooling::spool_failure_fixture(&directory) }.unwrap();
        let (shared, sizing) = sizing_fixture();
        sizing.sizing.error("first sizing refusal");
        sizing.sizing.error("second sizing refusal");
        let context = collector_memory::terminal::Context {
            failed_hour: Some(7),
            ..Default::default()
        };
        collector_memory::terminal::set_context(context);
        sizing.spool_pass(&shared, &mut processor, true).unwrap();
        assert_eq!(lock(&sizing.sizing.context).failed_hour, Some(7));
        collector_memory::terminal::set_context(Default::default());
        sizing.spool_pass(&shared, &mut processor, true).unwrap();
        let _ = sizing.sizing.failure.set("synthetic sizing failure");
        assert_eq!(
            sizing.spool_pass(&shared, &mut processor, true).err(),
            Some("synthetic sizing failure")
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn only_a_spool_pass_that_received_an_hour_requests_sizing() {
        let directory =
            std::env::temp_dir().join(format!("hoi4-sizing-request-{}", std::process::id()));
        let mut processor =
        // SAFETY: The isolated test owns the spool's failure state.
        unsafe { collector_spool::spooling::spool_failure_fixture(&directory) }.unwrap();
        let (shared, mut sizing) = sizing_fixture();
        sizing.thread = Some(std::thread::spawn(std::thread::park));
        sizing.spool_pass(&shared, &mut processor, false).unwrap();
        assert!(!sizing.sizing.pass_requested.load(Ordering::Acquire));
        sizing.spool_pass(&shared, &mut processor, true).unwrap();
        assert!(sizing.sizing.pass_requested.load(Ordering::Acquire));
        drop(sizing);
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
        let capacities = *sizing.capacities.lock();
        assert!(capacities.extended.naval_result_sources >= 64);
        assert!(capacities.extended.equipment_destruction >= 64);
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS
        );
    }

    fn assert_idle_war_and_faction_capacities(shared: &Shared, sizing: &SizingShared) {
        let capacities = *sizing.capacities.lock();
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS
        );
        for slot in
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }.slots()
        {
            let frame = unsafe { slot.captured() };
            assert!(frame.wars.len() >= capacities.core[5]);
            assert!(frame.factions.len() >= capacities.core[6]);
        }
    }

    #[test]
    fn live_observation_releases_capacity_mutex_and_grows_idle_buffers() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let wars = sizing.capacities.lock().core[5] + 8;
        let mut reads = 0;
        sizing.raise_from_live_observation(&shared, |capacities| {
            reads += 1;
            assert!(sizing.capacities.try_lock().is_some());
            capacities.core[5] = wars;
            [None, None]
        });
        assert_eq!(reads, 1);
        assert_eq!(sizing.capacities.lock().core[5], wars);
        assert_eq!(lock(&sizing.pending).errors, 0);
        sizing.grow_idle(&shared);
        assert_idle_war_and_faction_capacities(&shared, sizing);
    }

    #[test]
    fn live_observation_merges_concurrent_raises_without_repeating_live_reads() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let initial = *sizing.capacities.lock();
        let wars = initial.core[5] + 8;
        let factions = initial.core[6] + 8;
        let mut reads = 0;
        sizing.raise_from_live_observation(&shared, |capacities| {
            reads += 1;
            assert!(sizing.capacities.try_lock().is_some());
            capacities.core[5] = wars;
            capacities.extended.naval_result_sources = 16;
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        sizing.raise(&shared, |capacities| {
                            capacities.core[5] = wars + 8;
                            capacities.core[6] = factions;
                            capacities.extended.naval_result_sources = 8;
                            capacities.extended.equipment_destruction = 24;
                        });
                    })
                    .join()
                    .unwrap();
            });
            [
                Some("synthetic table hint failure"),
                Some("synthetic live-input hint failure"),
            ]
        });
        assert_eq!(reads, 1);
        let published = *sizing.capacities.lock();
        assert_eq!(published.core[5], wars + 8);
        assert_eq!(published.core[6], factions);
        assert_eq!(published.extended.naval_result_sources, 16);
        assert_eq!(published.extended.equipment_destruction, 24);
        sizing.grow_idle(&shared);
        assert_idle_war_and_faction_capacities(&shared, sizing);
        let pending = lock(&sizing.pending);
        assert_eq!(pending.errors, 2);
        assert_eq!(
            pending.last_error,
            Some("synthetic live-input hint failure")
        );
    }

    #[test]
    fn recycle_merges_a_concurrent_raise_and_catches_up_before_publication() {
        let _demand = collector_equipment_destruction::equipment_destruction::tests::fresh_demand();
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let returning =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap();
        let initial = *sizing.capacities.lock();
        let wars = initial.core[5] + 8;
        let factions = initial.core[6] + 8;
        let frame = unsafe {
            // SAFETY: This is the runtime's frame pool for the current session.
            shared.frames()[returning].captured_mut()
        };
        frame.wars_result = engine_native_memory::walk::Outcome::Truncated {
            len: 0,
            unread_rows: wars as u32,
        };
        frame.factions_result = engine_native_memory::walk::Outcome::Truncated {
            len: 0,
            unread_rows: factions as u32,
        };
        let checked = Arc::new(AtomicBool::new(false));
        let check_seen = Arc::clone(&checked);
        *lock(&sizing.before_recycle_publish) = Some(Box::new(move |sizing, shared| {
            assert!(sizing.capacities.try_lock().is_some());
            assert_eq!(
                // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                unsafe { shared.frames() }.len(),
                sampling::BUFFERS - 1
            );
            let frame = unsafe {
                // SAFETY: This is the runtime's frame pool for the current session.
                shared.frames()[returning].captured()
            };
            assert!(frame.wars.len() >= wars);
            let grown_wars = frame.wars.len();
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        sizing.raise(shared, |capacities| {
                            capacities.core[5] = grown_wars + 8;
                            capacities.core[6] = initial.core[6] + 1;
                            capacities.extended.equipment_destruction = 24;
                        });
                    })
                    .join()
                    .unwrap();
            });
            check_seen.store(true, Ordering::Relaxed);
        }));
        sizing.recycle_frame(&shared, returning);
        assert!(checked.load(Ordering::Relaxed));
        assert!(!sizing.hint_demand.load(Ordering::Acquire));
        let capacities = *sizing.capacities.lock();
        assert!(capacities.core[5] > wars);
        assert!(capacities.core[6] >= factions);
        assert_eq!(capacities.extended.equipment_destruction, 24);
        let free: [_; sampling::BUFFERS] = std::array::from_fn(|_|
            // SAFETY: This consumer owns the queue removal for the named pool.
            unsafe {  shared.frames() .pop() }.unwrap());
        assert!(free.contains(&returning));
        for index in free {
            let frame = unsafe {
                // SAFETY: This is the runtime's frame pool for the current session.
                shared.frames()[index].captured()
            };
            if index == returning {
                assert!(frame.wars.len() >= capacities.core[5]);
                assert!(frame.factions.len() >= capacities.core[6]);
            } else {
                assert!(frame.wars.len() < wars);
            }
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame(index) };
        }
        sizing.grow_idle(&shared);
        assert_idle_war_and_faction_capacities(&shared, sizing);
    }

    #[test]
    fn idle_growth_keeps_the_next_hour_s_frame_and_its_spare_for_the_game() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let held: [_; sampling::BUFFERS - collector_capture::frame_pool::AHEAD] =
            std::array::from_fn(|_|
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.take_frame(&mut 0) }.unwrap());
        let idle: Vec<_> = (0..sampling::BUFFERS)
            .filter(|index| !held.contains(index))
            .collect();
        let frame = |index: usize| unsafe {
            // SAFETY: This is the runtime's frame pool for the current session.
            shared.frames()[index].captured()
        };
        let bytes = || {
            idle.iter()
                .map(|&index| frame(index).allocated_bytes())
                .collect::<Vec<_>>()
        };
        let before = bytes();
        let wars = frame(idle[0]).wars.len() + 8;
        sizing.raise(&shared, |capacities| capacities.core[5] = wars);
        *lock(&sizing.before_idle_frame_growth) = Some(Box::new(|_, _| {
            panic!("sizing leased the game's next frame or its spare for growth");
        }));
        let pool_before = shared.pool_owned();
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            collector_capture::frame_pool::AHEAD
        );
        sizing.grow_idle(&shared);
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            collector_capture::frame_pool::AHEAD
        );
        assert_eq!(shared.pool_owned(), pool_before);
        assert_eq!(bytes(), before);
        let mut taken: Vec<_> = (0..collector_capture::frame_pool::AHEAD)
            .map(|_| {
                let mut wait = u64::MAX;
                let index =
                // SAFETY: This fixture owns its frame and worker state and runs without a live
                // attachment.
                unsafe { shared.take_frame(&mut wait) }.unwrap();
                assert_eq!(wait, 0);
                index
            })
            .collect();
        taken.sort_unstable();
        assert_eq!(taken, idle);
        for &index in &idle {
            sizing.recycle_frame(&shared, index);
            assert!(frame(index).wars.len() >= wars);
        }
        for index in held {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame_prepared(index) };
        }
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS
        );
    }

    #[test]
    fn a_wake_during_a_sizing_pass_builds_the_spare_without_waiting_for_the_pass() {
        use std::sync::mpsc;
        let fixture = Arc::new(sizing_fixture());
        let (shared, thread) = &*fixture;
        let sizing = &*thread.sizing;
        let wars = sizing.capacities.lock().core[5] + 8;
        sizing.raise(shared, |capacities| capacities.core[5] = wars);
        let (in_pass, pass_entered) = mpsc::channel::<()>();
        let (release, resume) = mpsc::channel::<()>();
        *lock(&sizing.before_idle_frame_growth) = Some(Box::new(move |_, _| {
            let _ = in_pass.send(());
            let _ = resume.recv_timeout(collector_test_support::TEST_HANG_GUARD);
        }));
        let (built, spare) = mpsc::channel::<()>();
        let built = Mutex::new(built);
        let observe = move |at| {
            if at == sampling::BuildAhead::Free {
                let _ = built.lock().unwrap().send(());
            }
        };
        let watching = Arc::clone(&fixture);
        collector_test_support::watched(
            move || {
                format!(
                    "{:?}",
                    // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                    unsafe { watching.0.frames() }.gauge()
                )
            },
            move |steps| {
                let (shared, thread) = &*fixture;
                let sizing = &*thread.sizing;
                let builder = {
                    let fixture = Arc::clone(&fixture);
                    collector_test_support::spawn_ending(move || {
                        frame_builder_observing(&fixture.0, &fixture.1.sizing, observe)
                    })
                };
                struct End<'a>(
                    Option<mpsc::Sender<()>>,
                    &'a AtomicBool,
                    std::thread::Thread,
                );
                impl Drop for End<'_> {
                    fn drop(&mut self) {
                        drop(self.0.take());
                        self.1.store(true, Ordering::Release);
                        self.2.unpark();
                    }
                }
                let end = End(Some(release), &sizing.stop, builder.thread().clone());
                let pass = {
                    let fixture = Arc::clone(&fixture);
                    collector_test_support::spawn_ending(move || {
                        fixture.1.sizing.grow_idle(&fixture.0)
                    })
                };
                steps.at("waiting for the pass to lease an idle frame");
                pass_entered
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap_or_else(|_| panic!("the pass never leased an idle frame"));
                assert_eq!(
                    // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                    unsafe { shared.frames() }.len(),
                    sampling::BUFFERS - 1
                );
                let held: Vec<_> = (0..sampling::BUFFERS - collector_capture::frame_pool::AHEAD)
                    .map(|hour| {
                        steps.at(format_args!("hour {hour}: taking its frame"));
                        let mut grow_ns = u64::MAX;
                        let index =
                        // SAFETY: This fixture owns its frame and worker state and runs without a
                        // live attachment.
                        unsafe { shared.take_frame(&mut grow_ns) }.unwrap();
                        assert_eq!(
                            grow_ns,
                            0,
                            "hour {hour}: {:?}",
                            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                            unsafe { shared.frames() }.gauge()
                        );
                        index
                    })
                    .collect();
                steps.at("waiting for the spare while the pass is held");
                spare
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap_or_else(|_| {
                        panic!(
                            "the spare waited for the pass: {:?}",
                            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                            unsafe { shared.frames() }.gauge()
                        )
                    });
                assert!(
                    !pass.is_finished(),
                    "the pass ended before the spare was built"
                );
                let gauge =
                    // SAFETY: This is the runtime's frame pool for the current session.
                    unsafe { shared.frames() }.gauge();
                assert_eq!((gauge.grown, gauge.built_ahead), (0, 1), "{gauge:?}");
                drop(end);
                steps.at("the pass, released, and the builder, stopped");
                pass.end(|| "the sizing pass, released,".into());
                builder.end(|| "the frame builder, stopped,".into());
                assert!(thread.sizing.failure.get().is_none());
                for index in held {
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { shared.recycle_frame_prepared(index) };
                }
                let gauge =
                    // SAFETY: This is the runtime's frame pool for the current session.
                    unsafe { shared.frames() }.gauge();
                assert_eq!(
                    (gauge.live, gauge.free, gauge.in_flight, gauge.freed),
                    (sampling::BUFFERS, sampling::BUFFERS, 0, 1),
                    "{gauge:?}"
                );
            },
        );
    }

    #[test]
    fn recycling_and_game_reuse_finish_while_idle_growth_is_paused() {
        let fixture = Arc::new(sizing_fixture());
        let (shared, thread) = &*fixture;
        let sizing = &thread.sizing;
        let returning =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap();
        let primed: Vec<_> = (0..=collector_capture::frame_pool::AHEAD)
            .map(|_| {
                // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
                unsafe { shared.take_frame(&mut 0) }.unwrap()
            })
            .collect();
        for index in primed {
            // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
            unsafe { shared.restore_idle_frame_prepared(index) };
        }
        let wars = sizing.capacities.lock().core[5] + 8;
        sizing.raise(shared, |capacities| capacities.core[5] = wars);
        let (leased, lease_seen) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel::<()>();
        *lock(&sizing.before_idle_frame_growth) = Some(Box::new(move |sizing, shared| {
            assert!(sizing.capacities.try_lock().is_some());
            assert_eq!(
                // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                unsafe { shared.frames() }.len(),
                collector_capture::frame_pool::AHEAD
            );
            leased.send(()).unwrap();
            let _ = resume.recv_timeout(collector_test_support::TEST_HANG_GUARD);
        }));
        let watching = Arc::clone(&fixture);
        collector_test_support::watched(
            move || {
                format!(
                    "{:?}",
                    // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                    unsafe { watching.0.frames() }.gauge()
                )
            },
            move |steps| {
                let (shared, thread) = &*fixture;
                let sizing = &thread.sizing;
                let release = release;
                let worker = {
                    let fixture = Arc::clone(&fixture);
                    collector_test_support::spawn_ending(move || {
                        fixture.1.sizing.grow_idle(&fixture.0)
                    })
                };
                steps.at("waiting for idle growth to lease a frame");
                lease_seen
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap_or_else(|_| panic!("idle growth never leased a frame"));
                let others: Vec<_> = (0..collector_capture::frame_pool::AHEAD)
                    .map(|hour| {
                        steps.at(format_args!("hour {hour}: taking its frame"));
                        let mut wait = u64::MAX;
                        let index =
                        // SAFETY: This fixture owns its frame and worker state and runs without a
                        // live attachment.
                        unsafe { shared.take_frame(&mut wait) }.unwrap();
                        assert_eq!(wait, 0);
                        index
                    })
                    .collect();
                assert_eq!(
                    // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                    unsafe { shared.frames() }.len(),
                    0
                );
                steps.at("the spool recycling while idle growth is held");
                sizing.recycle_frame(shared, returning);
                steps.at("the game reusing the recycled frame");
                let mut wait = u64::MAX;
                assert_eq!(
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { shared.take_frame(&mut wait) },
                    Some(returning)
                );
                assert_eq!(wait, 0);
                drop(release);
                steps.at("idle growth, released");
                worker.end(|| "idle growth, released,".into());
                for index in others.into_iter().chain(Some(returning)) {
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { shared.recycle_frame_prepared(index) };
                }
                assert_eq!(
                    // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                    unsafe { shared.frames() }.len(),
                    sampling::BUFFERS
                );
            },
        );
    }

    #[test]
    fn an_hour_building_its_own_frame_never_waits_for_the_spool_freeing_a_surplus_one() {
        use sampling::BUFFERS;
        use std::sync::mpsc;
        let fixture = Arc::new(sizing_fixture());
        let (freeing, free_held) = mpsc::channel::<()>();
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let free = move |surplus: Option<Box<sampling::Captured>>| {
            if surplus.is_some() {
                let _ = freeing.send(());
                let _ = released
                    .lock()
                    .unwrap()
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD);
            }
            drop(surplus);
        };
        let watching = Arc::clone(&fixture);
        collector_test_support::watched(
            move || {
                format!(
                    "{:?}",
                    // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                    unsafe { watching.0.frames() }.gauge()
                )
            },
            move |steps| {
                let (shared, thread) = &*fixture;
                let release = release;
                let mut taken: Vec<_> = (0..=BUFFERS)
                    .map(|hour| {
                        steps.at(format_args!("hour {hour}: taking its frame"));
                        // SAFETY: This fixture owns its frame and worker state and runs without a
                        // live attachment.
                        unsafe { shared.take_frame(&mut 0) }.unwrap_or_else(|| {
                            panic!(
                                "hour {hour}: {:?}",
                                // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                                unsafe { shared.frames() }.gauge()
                            )
                        })
                    })
                    .collect();
                for _ in 0..BUFFERS {
                    let index = taken.pop().unwrap();
                    steps.at(format_args!("the spool returning frame {index}"));
                    thread.sizing.recycle_frame(shared, index);
                }
                let gauge =
                    // SAFETY: This is the runtime's frame pool for the current session.
                    unsafe { shared.frames() }.gauge();
                assert_eq!(
                    (gauge.live, gauge.free, gauge.grown),
                    (BUFFERS + 1, BUFFERS, 1),
                    "{gauge:?}"
                );
                let surplus = taken.pop().unwrap();
                let spool = {
                    let fixture = Arc::clone(&fixture);
                    collector_test_support::spawn_ending(move || {
                        fixture.1.sizing.recycle_frame_with(&fixture.0, surplus, free)
                    })
                };
                steps.at("waiting for the spool to free the surplus frame");
                free_held
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap_or_else(|_| {
                        panic!(
                            "the spool never freed a surplus frame: {:?}",
                            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                            unsafe { shared.frames() }.gauge()
                        )
                    });
                let hours = BUFFERS + 1;
                let game = {
                    let fixture = Arc::clone(&fixture);
                    collector_test_support::spawn_ending(move || {
                        (0..hours)
                            .map(|_| {
                                let mut grow_ns = u64::MAX;
                                (
                                    // SAFETY: This fixture owns its frame and worker state and runs
                                    // without a live attachment.
                                    unsafe { fixture.0.take_frame(&mut grow_ns) },
                                    grow_ns,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                };
                steps.at("the hours, while the spool's free is held");
                let hours_taken = game.end(|| {
                    format!(
                        "the hours, while the spool freed a surplus frame, ({:?})",
                        // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                        unsafe { shared.frames() }.gauge()
                    )
                });
                assert!(
                    !spool.is_finished(),
                    "the spool's free ended before the hours did"
                );
                for (hour, &(index, grow_ns)) in hours_taken.iter().enumerate() {
                    assert!(
                        index.is_some(),
                        "hour {hour}: {:?}",
                        // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
                        unsafe { shared.frames() }.gauge()
                    );
                    assert_eq!(
                        grow_ns > 0,
                        hour == hours - 1,
                        "hour {hour} grew {grow_ns} ns"
                    );
                }
                drop(release);
                steps.at("the spool's free, released");
                spool.end(|| "the spool's free, released,".into());
                let hours_held = hours_taken.into_iter().flat_map(|(index, _)| index);
                for index in taken.into_iter().chain(hours_held) {
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { shared.recycle_frame(index) };
                }
                let gauge =
                    // SAFETY: This is the runtime's frame pool for the current session.
                    unsafe { shared.frames() }.gauge();
                assert_eq!(
                    (gauge.live, gauge.free, gauge.in_flight, gauge.freed),
                    (BUFFERS, BUFFERS, 0, 2),
                    "{gauge:?}"
                );
            },
        );
    }

    #[test]
    fn recycle_counts_country_intel_growth_during_conversion_once() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let allocated_bytes = || {
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }
                .slots()
                .map(|slot| unsafe { slot.captured() }.allocated_bytes())
                .sum::<usize>()
                + shared.parked_bytes() as usize
                +
                    // SAFETY: Sizing and the test use only the home associated with these session frames.
                    unsafe { shared.country_intel() }.allocated_bytes()
                + collector_division_templates::extended::division_templates::home_allocated_bytes(
                    // SAFETY: The sizing pass uses its session's live shared storage.
                    unsafe { shared.template_home() },
                )
                + collector_naval_details::naval_combat_sources::home_allocated_bytes(
                    // SAFETY: The sizing pass uses its session's live shared storage.
                    unsafe { shared.naval_combat_home() },
                )
        };
        let returning =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap();
        let allocated_before = allocated_bytes();
        let pool_before = shared.pool_owned();
        let shared_before =
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes();
        let frame = unsafe {
            // SAFETY: This is the runtime's frame pool for the current session.
            shared.frames()[returning].captured()
        };
        let before_frame_bytes = frame.allocated_bytes();
        let initial = *sizing.capacities.lock();
        let wars = initial.core[5] + 8;
        let intel_rows = initial.extended.intel.rows[15] + 8;

        sizing.raise(&shared, |capacities| {
            capacities.core[5] = wars;
            capacities.extended.raise_for_country_intel(intel_rows);
        });
        sizing.grow_idle(&shared);
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS - 1
        );
        assert_eq!(frame.allocated_bytes(), before_frame_bytes);
        let shared_after_idle =
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes();
        assert!(shared_after_idle > shared_before);
        let allocated_after_idle = allocated_bytes();
        assert_eq!(
            allocated_after_idle - allocated_before,
            shared_after_idle - shared_before
        );
        let pool_after_idle = shared.pool_owned();
        assert_eq!(
            pool_after_idle - pool_before,
            (allocated_after_idle - allocated_before) as u64
        );

        sizing.recycle_frame(&shared, returning);
        assert_eq!(
            // SAFETY: This fixture reads the home associated with its own frame pool.
            unsafe { shared.country_intel() }.allocated_bytes(),
            0
        );
        let allocated_after_return = allocated_bytes();
        let pool_after_return = shared.pool_owned();
        assert_eq!(
            pool_after_return.wrapping_sub(pool_after_idle),
            (allocated_after_return as u64).wrapping_sub(allocated_after_idle as u64)
        );
        assert_eq!(
            pool_after_return.wrapping_sub(pool_before),
            (allocated_after_return as u64).wrapping_sub(allocated_before as u64)
        );

        sizing.idle_growth_requested.store(true, Ordering::Release);
        sizing.grow_idle(&shared);
        assert_eq!(
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes(),
            0
        );
        let allocated_after_second = allocated_bytes();
        let pool_after_second = shared.pool_owned();
        assert_eq!(
            pool_after_second.wrapping_sub(pool_before),
            (allocated_after_second as u64).wrapping_sub(allocated_before as u64)
        );
        assert_idle_war_and_faction_capacities(&shared, sizing);

        let mut loan = collector_intel::daily_intel::Loan::new(
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.clone(),
        );
        assert!(matches!(
            loan.borrow(true),
            collector_intel::daily_intel::Walk::Ready(_)
        ));
        drop(loan);
        let rehydrated =
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes();
        assert!(rehydrated >= shared_after_idle);
        sizing.account_homes(&shared);
        sizing.account_homes(&shared);
        let allocated_after_rehydrate = allocated_bytes();
        let pool_after_rehydrate = shared.pool_owned();
        assert_eq!(
            pool_after_rehydrate.wrapping_sub(pool_after_second),
            rehydrated as u64
        );
        assert_eq!(
            pool_after_rehydrate.wrapping_sub(pool_before),
            (allocated_after_rehydrate as u64).wrapping_sub(allocated_before as u64)
        );
    }

    #[test]
    fn concurrent_frame_accounting_charges_shared_country_intel_growth_once() {
        let (shared, thread) = sizing_fixture();
        let sizing = &thread.sizing;
        let held: [_; 2] = std::array::from_fn(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap());
        let before =
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes();
        let pool_before = shared.pool_owned();
        let mut rows = sizing.capacities.lock().extended.intel.rows[15] + 8;
        let mut loan = collector_intel::daily_intel::Loan::new(
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.clone(),
        );
        loan.grow(&mut rows);
        let added =
            // SAFETY: Sizing and the test use only the home associated with these session frames.
            unsafe { shared.country_intel() }.allocated_bytes() - before;
        assert!(added > 0);
        std::thread::scope(|scope| {
            for _ in held {
                let shared = &shared;
                scope.spawn(move || sizing.account_homes(shared));
            }
        });
        assert_eq!(shared.pool_owned() - pool_before, added as u64);
        for index in held {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame_prepared(index) };
        }
    }

    #[test]
    fn game_score_attachment_config_defaults_enabled_and_accepts_off_switch() {
        let mut value = json!({
            "session_id": 1, "output_dir": "synthetic/session", "replay_dir": "synthetic/replays",
            "session_start": [], "ready_event": "synthetic-ready", "detach_event": "synthetic-detach",
            "done_event": "synthetic-done", "idle_event": "synthetic-idle", "cancel_file": "synthetic/cancel",
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
            "done_event": "synthetic-done", "idle_event": "synthetic-idle", "cancel_file": "synthetic/cancel",
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
            "done_event": "synthetic-done", "idle_event": "synthetic-idle",
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
        let folder = session.join(REFRESH_REQUESTS);
        fs::create_dir_all(&folder).unwrap();
        let request = |token: &str| folder.join(format!("{token}.request"));
        let ack = |token: &str| fs::read_to_string(folder.join(format!("{token}.ack"))).unwrap();
        let accepted = std::cell::Cell::new(0);
        let accept = || accepted.set(accepted.get() + 1);
        let taken = || accepted.replace(0);
        let mut requests = RefreshRequests::default();
        requests.poll(&session, accept).unwrap();
        assert_eq!(taken(), 0);
        fs::write(request("token-7"), "token-7").unwrap();
        requests.poll(&session, accept).unwrap();
        assert_eq!(taken(), 1);
        assert_eq!(ack("token-7"), "token-7");
        assert!(!request("token-7").exists());
        requests.poll(&session, accept).unwrap();
        assert_eq!(taken(), 0);
        fs::write(request("token-8"), "token-8").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(request("token-8"))
            .unwrap();
        assert!(requests.poll(&session, accept).is_err());
        assert_eq!(taken(), 1);
        assert_eq!(ack("token-8"), "token-8");
        requests.poll(&session, accept).unwrap();
        assert_eq!(taken(), 0);
        drop(held);
        fs::remove_file(request("token-8")).unwrap();
        requests.poll(&session, accept).unwrap();
        assert!(requests.accepted.is_empty());
        fs::write(request("token-9"), "token-9").unwrap();
        fs::write(request("token-10"), "token-10").unwrap();
        fs::write(folder.join("token-11.tmp"), "token-11").unwrap();
        requests.poll(&session, accept).unwrap();
        assert_eq!(taken(), 2);
        for token in ["token-9", "token-10"] {
            assert_eq!(ack(token), token);
            assert!(!request(token).exists());
        }
        assert!(!folder.join("token-11.ack").exists());
        let _ = fs::remove_dir_all(&session);
    }

    #[test]
    fn a_failed_refresh_acknowledgement_is_retried_without_another_refresh() {
        let session = std::env::temp_dir().join(format!("hoi4-refresh-ack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session);
        let folder = session.join(REFRESH_REQUESTS);
        fs::create_dir_all(&folder).unwrap();
        let accepted = std::cell::Cell::new(0);
        let accept = || accepted.set(accepted.get() + 1);
        let taken = || accepted.replace(0);
        let mut requests = RefreshRequests::default();
        fs::create_dir(folder.join("token-9.ack")).unwrap();
        fs::write(folder.join("token-9.request"), "token-9").unwrap();
        assert!(requests.poll(&session, accept).is_err());
        assert_eq!(taken(), 1);
        assert!(requests.poll(&session, accept).is_err());
        assert_eq!(taken(), 0);
        fs::remove_dir(folder.join("token-9.ack")).unwrap();
        requests.poll(&session, accept).unwrap();
        assert_eq!(taken(), 0);
        assert_eq!(
            fs::read_to_string(folder.join("token-9.ack")).unwrap(),
            "token-9"
        );
        assert!(!folder.join("token-9.request").exists());
        let _ = fs::remove_dir_all(&session);
    }

    #[derive(Default)]
    struct Registry(std::cell::Cell<u32>, std::cell::Cell<u32>);

    impl engine_native_memory::Memory for Registry {
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
            if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
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
        let memory = Registry::default();
        let mut watch = RegistryWatch::default();
        assert_eq!(watch.poll(&memory, 0), None);
        memory.0.set(5_001);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), Some((5_001, 0)));
        assert_eq!(watch.poll(&memory, 0), None);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
        memory.1.set(2_500);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), Some((5_001, 2_500)));
        memory.0.set(400);
        memory.1.set(100);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
        memory.0.set(1 << 20);
        watch.last = None;
        assert_eq!(watch.poll(&memory, 0), None);
    }

    #[test]
    fn startup_sizing_retries_an_empty_world_and_presizes_regions_without_raids() {
        let memory = Registry::default();
        let mut watch = RegistryWatch::default();
        let mut capacities = collector_capture::extended::Capacities::default();
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
        assert!(
            // SAFETY: The sizing owner prepares the session's live storage before callbacks or in its
            // sizing pass.
            unsafe { capacities.presize_live_inputs(&memory, 0) }.is_err()
        );
        assert_eq!(capacities.air.scores.regions, 2_500);
    }

    #[test]
    fn land_detail_watch_presizes_growth_without_country_registry_changes() {
        use engine_native::{cland_combat as combat, cland_combatant as side, cprovince};
        use engine_native_parts::cterrain_type;

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
        impl engine_native_memory::Memory for Combats {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&0x1000u64.to_le_bytes());
                    return true;
                }
                engine_native_memory::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &self.0,
                }
                .copy(address, output)
            }
        }
        let mut memory = Combats(vec![0; 0x6000]);
        memory.array(0x1000 + cgame_state::COUNTRIES, 0x5800, 2);
        memory.word(0x2000, combat::VTABLE_RVA.0 as u64);
        memory.word(0x2000 + combat::BODY, combat::BODY_VTABLE_RVA.0 as u64);
        memory.word(
            0x2000 + combat::HANDLE,
            (1 << 32) | u64::from(combat::TYPE_WORD),
        );
        memory.word(0x2000 + combat::ATTACKER, 0x3000);
        memory.word(0x2000 + combat::DEFENDER, 0x4000);
        memory.word(0x2000 + combat::STORED_TERRAIN, 0x5000);
        memory.word(0x5000, cterrain_type::NULL_VTABLE_RVA.0 as u64);
        for (pointer, attacker) in [(0x3000, 1), (0x4000, 0)] {
            memory.word(pointer, side::VTABLE_RVA.0 as u64);
            memory.word(pointer + side::COMBAT, 0x2000);
            memory.put(pointer + side::IS_ATTACKER, &[attacker]);
        }
        memory.word(0x2000 + combat::PROVINCE, 0x5200);
        memory.word(0x5200, cprovince::VTABLE_RVA.0 as u64);
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
        watch.last_combat_hints = Some(Instant::now() + Duration::from_secs(3600));
        assert!(watch.poll_combat_hints(&memory, 0).is_none());
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
        memory.put(roster + 12, &72u32.to_le_bytes());
        watch.last_combat_hints = None;
        assert!(watch.poll_combat_hints(&memory, 0).unwrap().land.is_none());
    }

    #[test]
    fn naval_detail_watch_presizes_growth_without_country_registry_changes() {
        use collector_naval_details::naval_details::fixtures::{Fixture, IMAGE, STATE};

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
        watch.last_combat_hints = Some(Instant::now() + Duration::from_secs(3600));
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
        memory.u32(roster + 12, 2);
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
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let capacities = collector_capture::extended::Capacities::default();
        let mut applied = AppliedHints::default();
        let hints = CombatHints {
            land: Some(engine_native_land::land_combat_details::DetailDemand {
                rows: usize::MAX,
                ..Default::default()
            }),
            naval: Some(engine_naval_history::naval_combat_demand::Demand {
                rows: usize::MAX,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        let stopped = collector_memory::terminal::failed();
        assert!(grow_combat_hints(
            &shared,
            &capacities,
            &hints,
            &mut applied
        ));
        assert!(grow_combat_hints(
            &shared,
            &capacities,
            &hints,
            &mut applied
        ));
        assert!(
            applied
                .iter()
                .all(|state| *state == CombatHintState::default())
        );
        assert_eq!(collector_memory::terminal::failed(), stopped);
        assert!(capacities == collector_capture::extended::Capacities::default());
        let before = shared.pool_owned();
        let valid = CombatHints {
            land: Some(engine_native_land::land_combat_details::DetailDemand {
                rows: 600,
                ..Default::default()
            }),
            naval: Some(engine_naval_history::naval_combat_demand::Demand {
                rows: 1,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        assert!(!grow_combat_hints(
            &shared,
            &capacities,
            &valid,
            &mut applied
        ));
        assert!(
            applied
                .iter()
                .all(|state| state.land.is_some() && state.naval.is_some())
        );
        assert!(shared.pool_owned() > before);
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS
        );
    }

    #[test]
    fn combat_hints_keep_the_next_hour_s_frame_and_its_spare_for_the_game() {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let held: [_; sampling::BUFFERS - collector_capture::frame_pool::AHEAD] =
            std::array::from_fn(|_|
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.take_frame(&mut 0) }.unwrap());
        let idle: Vec<_> = (0..sampling::BUFFERS)
            .filter(|index| !held.contains(index))
            .collect();
        let capacities = collector_capture::extended::Capacities::default();
        let mut applied = AppliedHints::default();
        let hints = CombatHints {
            land: Some(engine_native_land::land_combat_details::DetailDemand {
                rows: 600,
                ..Default::default()
            }),
            naval: Some(engine_naval_history::naval_combat_demand::Demand {
                rows: 1,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        let bytes = || {
            idle.iter()
                .map(|&index| {
                    unsafe {
                        // SAFETY: This is the runtime's frame pool for the current session.
                        shared.frames()[index].captured()
                    }
                    .allocated_bytes()
                })
                .collect::<Vec<_>>()
        };
        let before = bytes();
        let pool_before = shared.pool_owned();
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            collector_capture::frame_pool::AHEAD
        );
        assert!(!grow_combat_hints(
            &shared,
            &capacities,
            &hints,
            &mut applied
        ));
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            collector_capture::frame_pool::AHEAD
        );
        assert!(
            applied
                .iter()
                .all(|state| *state == CombatHintState::default())
        );
        assert_eq!(shared.pool_owned(), pool_before);
        assert_eq!(bytes(), before);
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame_prepared(held[0]) };
        assert!(!grow_combat_hints(
            &shared,
            &capacities,
            &hints,
            &mut applied
        ));
        for index in idle.iter().copied().chain(Some(held[0])) {
            assert!(applied[index].land.is_some() && applied[index].naval.is_some());
        }
        for index in held.into_iter().skip(1) {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { shared.recycle_frame_prepared(index) };
        }
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS
        );
    }

    #[test]
    fn combat_hints_keep_applied_state_and_grow_directly_recycled_frames() {
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };
        let capacities = collector_capture::extended::Capacities::default();
        let mut applied = AppliedHints::default();
        let accounted = shared.pool_owned();
        let frame_bytes = || {
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }
                .slots()
                .map(|slot| unsafe { slot.captured() }.allocated_bytes())
                .sum::<usize>()
                + shared.parked_bytes() as usize
        };
        let bytes = frame_bytes();
        let check_accounting = || {
            assert_eq!(
                shared.pool_owned() - accounted,
                (frame_bytes() - bytes) as u64,
            )
        };
        let mut hints = CombatHints {
            land: Some(engine_native_land::land_combat_details::DetailDemand {
                rows: 600,
                ..Default::default()
            }),
            naval: Some(engine_naval_history::naval_combat_demand::Demand {
                rows: 1,
                ..Default::default()
            }),
            failures: [None; 2],
        };
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        let grown = applied.clone();
        assert!(
            grown
                .iter()
                .all(|state| state.land.is_some() && state.naval.is_some())
        );
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        assert!(applied == grown);
        hints.land.as_mut().unwrap().rows = 100;
        hints.naval.as_mut().unwrap().rows = 0;
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        assert!(applied == grown);

        let held =
            // SAFETY: This consumer owns the queue removal for the named pool.
            unsafe {  shared.frames() .pop() }.unwrap();
        let previous = applied[held];
        hints.land.as_mut().unwrap().rows = 1200;
        hints.naval.as_mut().unwrap().rows = 257;
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        assert!(applied[held] == previous);
        let grown = applied.clone();
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        assert!(applied == grown);
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame(held) };
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        assert!(applied[held].land != previous.land);
        assert!(applied[held].naval != previous.naval);
        let grown = applied.clone();
        grow_combat_hints(&shared, &capacities, &hints, &mut applied);
        check_accounting();
        assert!(applied == grown);
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.len(),
            sampling::BUFFERS
        );
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
            collector_capture::country_scale::PoliticsDemand::default()
        );
        assert_eq!(
            watch.characters,
            collector_capture::country_scale::CharacterDemand::default()
        );
        assert_eq!(watch.poll_politics(&memory, 0), Ok(None));
        assert_eq!(watch.poll_characters(&memory, 0), Ok(None));
    }

    #[test]
    fn raid_path_memo_growth_presizes_before_refresh() {
        use engine_native_memory::Memory;
        use engine_native_air::craid_system;
        use engine_raid_inputs::raid_path_memo;
        use std::cell::Cell;
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
                bytes[..8].copy_from_slice(&(craid_system::VTABLE_RVA.0 as u64).to_le_bytes());
                for memo in [raid_path_memo::LAND_MEMO, raid_path_memo::NAVAL_MEMO] {
                    bytes[memo + 8..memo + 16].copy_from_slice(&0x10_0000_u64.to_le_bytes());
                    bytes[memo + 0x14..memo + 0x18].copy_from_slice(&self.mask.get().to_le_bytes());
                    bytes[memo + 0x18] = (self.mask.get() + 1).ilog2() as u8 + 2;
                }
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
        let mut sizing = collector_capture::country_scale::Sizing::for_slots(5_001);
        assert_eq!(sizing.extended.projects.rows[12], 0);
        let initial = collector_capture::country_scale::observe_raid_arenas(&memory, 0)
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
        assert!(initial.failures.into_iter().any(|reason| reason.is_some()));
        memory.mask.set(8_191);
        assert_eq!(watch.poll_raids(&memory, 0).unwrap(), None);
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
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let raw = std::env::temp_dir().join(format!("hoi4-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&raw);
        fs::create_dir_all(&raw).unwrap();
        let dir = TempDir(fs::canonicalize(&raw).unwrap());
        assert!(dir.0.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        let path = dir.0.join("status.json");
        write_status(&path, &json!({"hours": 1})).unwrap();
        let shared = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        write_status(&path, &json!({"hours": 2})).unwrap();
        let current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current["hours"], 2);
        drop(shared);
        let exclusive = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let error = write_status(&path, &json!({"hours": 3})).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(5 | 32)));
        drop(exclusive);
        write_status(&path, &json!({"hours": 4})).unwrap();
        assert!(crate::status_file::posix_rename_supported_for_test());
    }

    #[test]
    fn racing_political_hint_counts_in_completed_status_without_a_session_warning() {
        use engine_native_memory::Memory;
        use engine_native::ccountry;
        use std::cell::Cell;

        struct RacingHint {
            bytes: Vec<u8>,
            roster_reads: Cell<usize>,
        }
        impl Memory for RacingHint {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
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
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            capture_mode: replay_data::identity::CaptureMode::Full,
            identity: replay_data::identity::Identity {
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
                verification: replay_data::identity::Verification::Unchecked,
            },
        };
        let directory = std::env::temp_dir()
            .join(format!("hoi4-political-hint-status-{}", std::process::id(),));
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session.clone()) }.unwrap();
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
            engine_native::carmy::PresentationDemand::default(),
        );
        assert!(
            processor
                .observe_input_demand(watch.poll_presentation_census(&memory, 0))
                .flatten()
                .is_none()
        );

        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };

        // SAFETY: The current runtime or its spool owns the session's terminal stop.
        unsafe { shared.recording() }.store(false, Ordering::Release);
        let writer = crate::status_writer::StatusWriter::start(|_, _| Ok(())).unwrap();
        let status = serde_json::to_value(
            spool_status(
                &processor,
                &shared,
                &session,
                "detached",
                None,
                None,
                true,
                true,
                TeardownStatus::default(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(status["write_errors"], 0);
        assert_eq!(status["drain_complete"], true);
        assert!(status.get("warning").is_none());
        assert!(status.get("error").is_none());
        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restoration_only_status_warns_without_hiding_real_drain_or_capture_errors() {
        let session = SessionStart {
            id: 1,
            process_id: 100,
            process_start_time: 1000,
            qpc_frequency: 1,
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            capture_mode: replay_data::identity::CaptureMode::Full,
            identity: replay_data::identity::Identity {
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
                verification: replay_data::identity::Verification::Unchecked,
            },
        };
        let directory = std::env::temp_dir().join(format!(
            "hoi4-restoration-warning-status-{}",
            std::process::id()
        ));
        let processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session.clone()) }.unwrap();
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Shared::new(0, 0, 1) };

        // SAFETY: The current runtime or its spool owns the session's terminal stop.
        unsafe { shared.recording() }.store(false, Ordering::Release);
        let writer = crate::status_writer::StatusWriter::start(|_, _| Ok(())).unwrap();
        for (warning, error, installation_error, drain_complete) in [
            (None, None, None, true),
            (Some("earlier write retried"), None, None, true),
            (None, Some("final spool sync failed"), None, false),
            (None, Some("game-thread acquisition failed"), None, false),
            (None, None, Some("hook installation failed"), true),
        ] {
            let status: Value = serde_json::from_slice(
                &collector_status::status::serialize(
                    &spool_status(
                        &processor,
                        &shared,
                        &session,
                        "detached",
                        warning,
                        error,
                        drain_complete,
                        true,
                        TeardownStatus {
                            restoration_error: Some("synthetic hook stayed busy"),
                            installation_error,
                        },
                    )
                    .unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(status["state"], "pinned");
            assert_eq!(status["drain_complete"], drain_complete);
            assert_eq!(status["restoration_error"], "synthetic hook stayed busy");
            assert_eq!(
                status["warning"],
                warning.unwrap_or("synthetic hook stayed busy")
            );
            assert_eq!(status["error"], json!(error.or(installation_error)));
        }
        drop(writer);
        drop(processor);
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
        let mut status = json!({"state": "recording", "hours": 3});
        add_decode_status(&mut status);
        assert_eq!(status["state"], "recording");
        assert_eq!(status["hours"], 3);
        assert_eq!(status["sample_boundary"], "after_hourly_update");
        assert!(status["hook_timing"]["families"]["hourly"].is_object());
        assert!(
            status["capture"]["placement"]
                .as_str()
                .is_some_and(|placement| placement.starts_with("holding the game"))
        );
    }
}
