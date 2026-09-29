//! Passive process discovery, injection, detach and finalization.
use crate::{
    replay_io,
    windows::{self, Event, Process},
};
use engine::{PINNED_BUILD, build::hash_executable};
use installed_content::content::{GatherOptions, gather};
use replay::identity::SessionStart;
use serde_json::{Value, json};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::{
        Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
        Environment::ExpandEnvironmentStringsW,
        Performance::QueryPerformanceFrequency,
        Registry::{
            HKEY_CURRENT_USER, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
        },
    },
};

static STOP: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn control(kind: u32) -> i32 {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        STOP.store(true, Ordering::Release);
        1
    } else {
        0
    }
}

struct Options {
    output: PathBuf,
    user: Option<PathBuf>,
    once: bool,
}

fn usage() -> String {
    format!(
        "recorder [record] [--out <directory>] [--user-dir <Paradox directory>] [--once]
recorder detach [--out <directory>]
recorder refresh [--out <directory>]
recorder finalize <session-directory> [--out <directory>]
recorder inspect <campaign.battleplan>
recorder inspect <campaign.battleplan> --characters <substring> --hour <n> --branch <id>
{}
{}",
        crate::oracle::USAGE,
        crate::movement_oracle::USAGE
    )
}

pub fn command(mut args: Vec<OsString>) -> Result<(), String> {
    let command = if args
        .first()
        .is_some_and(|arg| !arg.to_string_lossy().starts_with('-'))
    {
        args.remove(0).into_string().map_err(|_| usage())?
    } else {
        "record".into()
    };
    if command == "inspect" && args.len() == 1 {
        return replay_io::inspect(Path::new(&args[0]));
    }
    if command == "inspect" {
        return inspect_characters(&args);
    }
    if command == "oracle" {
        return crate::oracle::command(args).map_err(|error| error.to_string());
    }
    if command == "movement-oracle" {
        return crate::movement_oracle::command(args);
    }
    if command == "--help" || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return Ok(());
    }
    let session = if command == "finalize" && !args.is_empty() {
        Some(PathBuf::from(args.remove(0)))
    } else {
        None
    };
    let mut options = Options {
        output: PathBuf::from(r"captures"),
        user: None,
        once: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--out") => options.output = PathBuf::from(args.next().ok_or_else(usage)?),
            Some("--user-dir") if command == "record" => {
                options.user = Some(PathBuf::from(args.next().ok_or_else(usage)?))
            }
            Some("--once") if command == "record" => options.once = true,
            _ => return Err(usage()),
        }
    }
    fs::create_dir_all(&options.output).map_err(|e| e.to_string())?;
    options.output = options.output.canonicalize().map_err(|e| e.to_string())?;
    match command.as_str() {
        "record" => record(options),
        "detach" => request_detach(&options.output),
        "refresh" => request_refresh(&options.output),
        "finalize" => {
            let session = session.ok_or_else(usage)?;
            ensure_finalizable(&session)?;
            let _lock = finalization_lock(&options.output)?;
            replay_io::finalize_session(&session, &options.output)?;
            Ok(())
        }
        _ => Err(usage()),
    }
}

/// `inspect <replay> --characters <substring> --hour <n> --branch <id>`, the
/// options in any order, each once.
fn inspect_characters(args: &[OsString]) -> Result<(), String> {
    let (path, options) = args.split_first().ok_or_else(usage)?;
    let (mut needle, mut hour, mut branch) = (None, None, None);
    for pair in options.chunks(2) {
        let [option, value] = pair else {
            return Err(usage());
        };
        let value = value.to_str().ok_or_else(usage)?;
        let slot = match option.to_str() {
            Some("--characters") => &mut needle,
            Some("--hour") => &mut hour,
            Some("--branch") => &mut branch,
            _ => return Err(usage()),
        };
        if slot.replace(value).is_some() {
            return Err(usage());
        }
    }
    let (Some(needle), Some(hour), Some(branch)) = (needle, hour, branch) else {
        return Err(usage());
    };
    replay_io::inspect_characters(
        Path::new(path),
        needle,
        hour.parse()
            .map_err(|_| "--hour requires an engine i32 hour".to_string())?,
        branch
            .parse()
            .map_err(|_| "--branch requires a u64 branch id".to_string())?,
    )
}

fn checked_single_game() -> Result<Option<u32>, String> {
    match windows::games()?.as_slice() {
        [] => Ok(None),
        [pid] => Ok(Some(*pid)),
        _ => {
            Err("two hoi4.exe processes are unsupported; no second process will be recorded".into())
        }
    }
}

fn record(options: Options) -> Result<(), String> {
    unsafe {
        SetConsoleCtrlHandler(Some(control), 1);
    }
    let user = options
        .user
        .map(Ok)
        .unwrap_or_else(resolve_user_directory)?;
    let user = user.canonicalize().map_err(|e| {
        format!(
            "Paradox user folder {}: {e}; pass --user-dir",
            user.display()
        )
    })?;
    println!("Paradox user folder: {}", user.display());
    println!("Waiting for hoi4.exe. Recording is passive; Ctrl+C detaches and finalizes.");
    loop {
        if STOP.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(pid) = checked_single_game()? else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let detached = capture(pid, &options.output, &user)?;
        if detached || options.once || STOP.load(Ordering::Acquire) {
            return Ok(());
        }
        println!("Waiting for the next game process.");
    }
}

fn capture(pid: u32, output: &Path, user: &Path) -> Result<bool, String> {
    let process = Process::open(pid, true)?;
    let exe = hash_executable(File::open(&process.path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !PINNED_BUILD.matches(&exe) {
        return Err(format!(
            "unsupported game build; wanted {} / {} bytes / {}; found {} bytes / {}. Nothing recorded.",
            PINNED_BUILD.version,
            PINNED_BUILD.exe_size,
            hex(&PINNED_BUILD.exe_sha256),
            exe.size,
            hex(&exe.sha256)
        ));
    }
    if windows::modules(pid)?.iter().any(|module| {
        module
            .path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("collector.dll"))
    }) {
        return Err(
            "collector.dll is already loaded in this process; detach its recorder first".into(),
        );
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = output
        .join("sessions")
        .join(format!("{}-{nonce}", process.start_time));
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let dll = bin.join("collector.dll");
    let source = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("collector.dll");
    fs::copy(&source, &dll).map_err(|e| format!("copy {}: {e}", source.display()))?;
    let dll_hash = hash_executable(File::open(&dll).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        .sha256;
    let game_dir = process
        .path
        .parent()
        .ok_or("game executable has no folder")?
        .to_owned();
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is unset")?;
    let gather_options = GatherOptions {
        game_dir,
        user_dir: user.to_owned(),
        cache_dir: local.join("HOI4Replay").join("recorder"),
        dlc_order: None,
    };
    println!("Verifying installed content hashes in the background...");
    let hash_thread = std::thread::spawn(move || gather(&gather_options));
    let mut next_discovery = Instant::now();
    while !hash_thread.is_finished() {
        if process.handle.signaled()? {
            return Err("game exited while content identity was being gathered".into());
        }
        if Instant::now() >= next_discovery {
            if checked_single_game()? != Some(pid) {
                return Err("game changed while content identity was being gathered".into());
            }
            next_discovery = Instant::now() + Duration::from_secs(2);
        }
        if STOP.load(Ordering::Acquire) {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let content = hash_thread
        .join()
        .map_err(|_| "content hashing worker panicked")?
        .map_err(|e| e.to_string())?;
    println!(
        "Content: {} files, {:.1} MB covered, {} cache hits, {:.3} s.",
        content.stats.files,
        content.stats.bytes as f64 / 1e6,
        content.stats.cache_hits,
        content.stats.elapsed_ms as f64 / 1000.0
    );
    if let Some(note) = &content.order_note {
        println!("Content order is unverified: {note}");
    }
    let identity = content
        .identity(
            PINNED_BUILD.version.into(),
            exe.sha256,
            exe.size,
            env!("CARGO_PKG_VERSION").into(),
            dll_hash,
        )
        .map_err(|e| e.to_string())?;
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 || frequency <= 0 {
        return Err("QueryPerformanceFrequency failed".into());
    }
    let session = SessionStart {
        id: process.start_time,
        process_id: pid,
        process_start_time: process.start_time,
        qpc_frequency: frequency as u64,
        sample_boundary: replay::identity::SampleBoundary::EndOfHour,
        identity,
    };
    let mut session_bytes = Vec::new();
    session
        .encode(&mut session_bytes)
        .map_err(|e| e.to_string())?;
    let prefix = format!("Local\\HOI4Replay.{}.{}", process.start_time, nonce);
    let ready_name = format!("{prefix}.ready");
    let detach_name = format!("{prefix}.detach");
    let done_name = format!("{prefix}.done");
    let ready = Event::new(&ready_name)?;
    let detach = Event::new(&detach_name)?;
    let done = Event::new(&done_name)?;
    let cancel_file = bin.join("collector.cancel");
    let expected_mounts = &content.expected_mounts;
    let config = json!({"session_id":session.id,"output_dir":directory,"replay_dir":output,"session_start":session_bytes,"ready_event":ready_name,"detach_event":detach_name,"done_event":done_name,"cancel_file":cancel_file,"content_order_note":content.order_note,"expected_mounts":expected_mounts,"mod_descriptors":content.mod_descriptors});
    write_json(&bin.join("collector.json"), &config)?;
    struct DetachOnDrop<'a> {
        event: &'a Event,
        cancel: &'a Path,
    }
    impl Drop for DetachOnDrop<'_> {
        fn drop(&mut self) {
            let _ = fs::write(self.cancel, b"cancelled\n");
            let _ = self.event.set();
        }
    }
    let _detach_on_error = DetachOnDrop {
        event: &detach,
        cancel: &cancel_file,
    };
    checked_single_game()?
        .filter(|&found| found == pid)
        .ok_or("game changed before injection")?;
    process.inject(&dll, || STOP.load(Ordering::Acquire))?;
    let error_path = bin.join("collector.error.txt");
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut next_progress = Instant::now() + Duration::from_secs(10);
    while !ready.handle.signaled()? {
        if error_path.exists() {
            return Err(fs::read_to_string(&error_path).unwrap_or_else(|e| e.to_string()));
        }
        if process.handle.signaled()? {
            return Err("game exited before collector became ready".into());
        }
        if Instant::now() >= deadline {
            return Err("collector readiness timed out; detach requested; spool retained".into());
        }
        if STOP.load(Ordering::Acquire) {
            detach.set()?;
        }
        if Instant::now() >= next_progress {
            println!("Preparing collector and prior campaign history before hourly sampling...");
            next_progress = Instant::now() + Duration::from_secs(10);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let active = output.join("active.json");
    write_json(
        &active,
        &json!({"recorder_pid":std::process::id(),"game_pid":pid,"process_start_time":process.start_time,"session_dir":directory,"dll":dll,"detach_event":detach_name,"done_event":done_name}),
    )?;
    println!("Recording process {pid}. Spool: {}", directory.display());
    let mut next_status = Instant::now();
    let mut requested_detach = false;
    let mut second_reported = false;
    let mut next_discovery = Instant::now();
    loop {
        requested_detach |= STOP.load(Ordering::Acquire) || detach.handle.signaled()?;
        if process.handle.signaled()? {
            break;
        }
        if requested_detach {
            detach.set()?;
            break;
        }
        if done.handle.signaled()? {
            requested_detach = true;
            break;
        }
        if Instant::now() >= next_discovery {
            let second = windows::games()?.iter().any(|&other| other != pid);
            if second && !second_reported {
                eprintln!("A second hoi4.exe is unsupported and will not be recorded.");
            }
            second_reported = second;
            next_discovery = Instant::now() + Duration::from_secs(2);
        }
        if Instant::now() >= next_status {
            print_status(&directory.join("status.json"));
            next_status = Instant::now() + Duration::from_secs(60);
        }
        if error_path.exists() {
            detach.set()?;
            requested_detach = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !process.handle.signaled()? {
        detach.set()?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done.handle.signaled()? && !process.handle.signaled()? {
            if Instant::now() >= deadline {
                return Err("detach timed out; collector may still be loaded; spool retained and not finalized".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        while collector_present(&process, &dll)? {
            if Instant::now() >= deadline {
                return Err(
                    "collector signaled done but remains loaded; spool retained and not finalized"
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    // The collector can fail while draining, after signaling was requested. Its
    // files become terminal evidence only after unload or process termination.
    let process_exited = process.handle.signaled()?;
    requested_detach |= STOP.load(Ordering::Acquire) || detach.handle.signaled()?;
    let mut outcome = terminal_outcome(&directory, session.id, requested_detach, process_exited);
    if process_exited {
        println!("Game process exited; recovering the flushed spool.");
    } else {
        println!("Collector absence confirmed in the running game process.");
    }
    for warning in &outcome.warnings {
        eprintln!("Collector warning: {warning}");
    }
    let _lock = finalization_lock(output)?;
    let finalized = match replay_io::finalize_session(&directory, output) {
        Ok(paths) => paths,
        Err(error) => {
            outcome
                .issues
                .push(format!("replay finalization failed: {error}"));
            Vec::new()
        }
    };
    let completed = json!({
        "replays":finalized,
        "detach_requested":requested_detach,
        "detached":outcome.drain_complete == Some(true),
        "process_exited":process_exited,
        "drain_complete":outcome.drain_complete,
        "partial":!outcome.issues.is_empty(),
        "issues":outcome.issues,
        "warnings":outcome.warnings,
    });
    write_json(&directory.join("completed.json"), &completed)?;
    // The active file is this recorder's own control record, outside the repository.
    if fs::read(&active)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value["session_dir"] == json!(directory))
    {
        fs::remove_file(&active).map_err(|e| e.to_string())?;
    }
    if !outcome.issues.is_empty() {
        return Err(format!(
            "recording completion is partial: {}; recoverable spool retained at {}",
            outcome.issues.join("; "),
            directory.display()
        ));
    }
    Ok(requested_detach)
}

#[derive(Debug)]
struct TerminalOutcome {
    drain_complete: Option<bool>,
    issues: Vec<String>,
    warnings: Vec<String>,
}

fn classify_terminal(
    session: u64,
    requested_detach: bool,
    process_exited: bool,
    status: Option<&Value>,
    error_file: Option<&str>,
) -> TerminalOutcome {
    let mut outcome = TerminalOutcome {
        drain_complete: None,
        issues: Vec::new(),
        warnings: Vec::new(),
    };
    let mut issue = |reason: String| {
        if !outcome.issues.contains(&reason) {
            outcome.issues.push(reason);
        }
    };
    if let Some(error) = error_file.filter(|error| !error.trim().is_empty()) {
        issue(format!("collector terminal error: {}", error.trim()));
    }
    if let Some(status) = status {
        for warning in [status["warning"].as_str(), status["last_error"].as_str()]
            .into_iter()
            .flatten()
            .filter(|warning| !warning.trim().is_empty())
        {
            if !outcome.warnings.iter().any(|previous| previous == warning) {
                outcome.warnings.push(warning.to_owned());
            }
        }
        let correct_session = status["session"].as_u64() == Some(session);
        if !correct_session {
            let reason =
                "collector terminal status has a missing or different session id".to_owned();
            if requested_detach || !process_exited {
                issue(reason);
            } else {
                outcome.warnings.push(reason);
            }
        }
        if let Some(error) = status["error"]
            .as_str()
            .filter(|error| !error.trim().is_empty())
        {
            issue(format!("collector terminal error: {}", error.trim()));
        }
        if status["state"] == "detached" && correct_session {
            outcome.drain_complete = status["drain_complete"].as_bool();
            if outcome.drain_complete == Some(false) {
                issue("collector final queue drain or spool sync was incomplete".into());
            }
        }
    }
    if requested_detach && outcome.drain_complete != Some(true) {
        issue(if process_exited {
            "game exited after detach was requested without confirmation of a completed collector drain".into()
        } else {
            "collector unloaded without confirming a completed final queue drain and spool sync"
                .into()
        });
    }
    outcome
}

fn terminal_outcome(
    directory: &Path,
    session: u64,
    requested_detach: bool,
    process_exited: bool,
) -> TerminalOutcome {
    let mut status_read_issues = Vec::new();
    let mut error_read_issues = Vec::new();
    let status_path = directory.join("status.json");
    let status = match fs::read(&status_path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(status) => Some(status),
            Err(error) => {
                status_read_issues.push(format!(
                    "cannot parse terminal status {}: {error}",
                    status_path.display()
                ));
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            status_read_issues.push(format!(
                "cannot read terminal status {}: {error}",
                status_path.display()
            ));
            None
        }
    };
    let error_path = directory.join("bin/collector.error.txt");
    let error_file = match fs::read_to_string(&error_path) {
        Ok(error) => Some(error),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            error_read_issues.push(format!(
                "cannot read collector terminal error {}: {error}",
                error_path.display()
            ));
            None
        }
    };
    let mut outcome = classify_terminal(
        session,
        requested_detach,
        process_exited,
        status.as_ref(),
        error_file.as_deref(),
    );
    if requested_detach || !process_exited {
        outcome.issues.extend(status_read_issues);
    } else {
        outcome.warnings.extend(status_read_issues);
    }
    outcome.issues.extend(error_read_issues);
    outcome
}

/// Validate the recorder's terminal result before a second console claims success.
fn completed_detach(value: &Value) -> Result<bool, String> {
    let partial = value["partial"]
        .as_bool()
        .ok_or("completion has no partial outcome")?;
    let issues = value["issues"]
        .as_array()
        .ok_or("completion has no issue list")?;
    let issues: Vec<_> = issues
        .iter()
        .map(|issue| issue.as_str().ok_or("completion issue is not text"))
        .collect::<Result<_, _>>()?;
    if partial || !issues.is_empty() {
        return Err(format!(
            "recording completion is partial: {}",
            if issues.is_empty() {
                "no completion details were recorded".into()
            } else {
                issues.join("; ")
            }
        ));
    }
    let requested = value["detach_requested"]
        .as_bool()
        .ok_or("completion has no detach-request outcome")?;
    let detached = value["detached"]
        .as_bool()
        .ok_or("completion has no detached outcome")?;
    let exited = value["process_exited"]
        .as_bool()
        .ok_or("completion has no process-exit outcome")?;
    if (requested || detached) && value["drain_complete"].as_bool() != Some(true) {
        return Err("requested detach has no completed collector drain".into());
    }
    if !detached && !exited {
        return Err("completion confirms neither detach nor game process exit".into());
    }
    Ok(detached)
}

/// Ask the attached collector to read every scheduled table at the next
/// hourly sample (a full-refresh hour), for example before an oracle save.
/// The collector acknowledges within about a second; nothing touches the game.
fn request_refresh(output: &Path) -> Result<(), String> {
    let active: Value = serde_json::from_slice(
        &fs::read(output.join("active.json"))
            .map_err(|e| format!("no active recorder here: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let session_dir = PathBuf::from(
        active["session_dir"]
            .as_str()
            .ok_or("active recorder has no session directory")?,
    );
    let token = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let ack = session_dir.join("refresh.ack");
    let request = session_dir.join("refresh.request");
    let _ = fs::remove_file(&ack);
    // Write, then rename: the collector never sees a partial token.
    let temporary = session_dir.join(format!("refresh.request.{token}.tmp"));
    fs::write(&temporary, &token).map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&temporary, &request) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if fs::read_to_string(&ack).is_ok_and(|text| text.trim() == token) {
            println!("Full refresh requested: the next hourly sample reads every scheduled table.");
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Withdraw our own request so a later attach does not act on it.
            if fs::read_to_string(&request).is_ok_and(|text| text.trim() == token) {
                let _ = fs::remove_file(&request);
            }
            return Err("the collector did not acknowledge the refresh request".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn request_detach(output: &Path) -> Result<(), String> {
    let active: Value = serde_json::from_slice(
        &fs::read(output.join("active.json"))
            .map_err(|e| format!("no active recorder here: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let name = active["detach_event"]
        .as_str()
        .ok_or("active recorder has no detach event")?;
    let session_dir = PathBuf::from(
        active["session_dir"]
            .as_str()
            .ok_or("active recorder has no session directory")?,
    );
    Event::open(name)?.set()?;
    println!("Detach requested. Waiting for recorder finalization.");
    let deadline = Instant::now() + Duration::from_secs(120);
    while !session_dir.join("completed.json").exists() {
        if Instant::now() >= deadline {
            return Err("detach/finalization has not completed; inspect the recorder console and retained spool".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let completed: Value = serde_json::from_slice(
        &fs::read(session_dir.join("completed.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if completed_detach(&completed)? {
        println!("Detached and finalized: {}", session_dir.display());
    } else {
        println!(
            "Game exited and replay finalized: {}",
            session_dir.display()
        );
    }
    Ok(())
}

fn print_status(path: &Path) {
    if let Ok(bytes) = fs::read(path)
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
    {
        if let Some(error) = value["error"]
            .as_str()
            .or_else(|| value["last_error"].as_str())
            .or_else(|| value["warning"].as_str())
        {
            eprintln!("Collector: {error}");
        }
        if value["hours"].as_u64().unwrap_or(0) == 0 {
            println!(
                "{}; waiting for the first recorded game hour.",
                value["state"].as_str().unwrap_or("Attached")
            );
            return;
        }
        let number = |key: &str| value[key].as_f64().unwrap_or(0.0);
        println!(
            "{}: {:.0} hours, last {:.3} ms, p95 {:.3} ms, {:.2} h/s, spool {:.1} MB, {:.0} dropped, {:.0} truncated",
            value["state"].as_str().unwrap_or("recording"),
            number("hours"),
            number("total_ns") / 1e6,
            number("p95_ns") / 1e6,
            number("hours_per_second"),
            number("spool_bytes") / 1e6,
            number("dropped_frames"),
            number("truncated_tables")
        );
    } else {
        println!("Attached; waiting for the first recorded game hour.");
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let from = windows::wide(temporary.as_os_str());
    let to = windows::wide(path.as_os_str());
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn collector_present(process: &Process, dll: &Path) -> Result<bool, String> {
    if process.handle.signaled()? {
        return Ok(false);
    }
    match process.has_module(dll) {
        Ok(present) => Ok(present),
        Err(_) if process.handle.signaled()? => Ok(false),
        Err(error) => Err(error),
    }
}

fn ensure_finalizable(directory: &Path) -> Result<(), String> {
    let config: Value = serde_json::from_slice(
        &fs::read(directory.join("bin/collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let bytes: Vec<u8> =
        serde_json::from_value(config["session_start"].clone()).map_err(|e| e.to_string())?;
    let mut decoder = replay::codec::Decoder::new(&bytes);
    let session = SessionStart::decode(&mut decoder).map_err(|e| e.to_string())?;
    decoder.finish().map_err(|e| e.to_string())?;
    if !windows::games()?.contains(&session.process_id) {
        return Ok(());
    }
    let process = match Process::open(session.process_id, false) {
        Ok(process) => process,
        Err(_) if !windows::games()?.contains(&session.process_id) => return Ok(()),
        Err(error) => return Err(error),
    };
    if process.start_time != session.process_start_time {
        return Ok(());
    }
    let done = config["done_event"]
        .as_str()
        .ok_or("collector configuration has no done event")?;
    if collector_present(&process, &directory.join("bin/collector.dll"))? {
        return Err(
            "collector is still loaded; use recorder detach or quit the game before finalizing"
                .into(),
        );
    }
    if !process.handle.signaled()?
        && !Event::open(done)
            .and_then(|event| event.handle.signaled())
            .unwrap_or(false)
    {
        // A completed recorder has already observed done and module absence; the
        // named event ceases to exist when all process handles have closed.
        if !directory.join("completed.json").is_file() {
            return Err(
                "live process has no completed detach witness; wait for its recorder or game exit"
                    .into(),
            );
        }
    }
    Ok(())
}

fn finalization_lock(output: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let path = output.join("finalize.lock");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match fs::OpenOptions::new()
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
                return Err(format!(
                    "output is busy recording/finalizing or unavailable: {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

pub(crate) fn resolve_user_directory() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let key = windows::wide(OsStr::new(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
    ));
    let value = windows::wide(OsStr::new("Personal"));
    let mut buffer = vec![0_u16; 32768];
    let mut bytes = (buffer.len() * 2) as u32;
    if unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    } == 0
    {
        let mut expanded = vec![0_u16; 32768];
        let len = unsafe {
            ExpandEnvironmentStringsW(
                buffer.as_ptr(),
                expanded.as_mut_ptr(),
                expanded.len() as u32,
            )
        };
        if len > 0 && (len as usize) <= expanded.len() {
            let path = PathBuf::from(OsString::from_wide(&expanded[..len as usize - 1]))
                .join("Paradox Interactive")
                .join("Hearts of Iron IV");
            if path.is_dir() {
                return Ok(path);
            }
        }
    }
    for variable in ["OneDrive", "USERPROFILE"] {
        if let Some(root) = std::env::var_os(variable) {
            let path = PathBuf::from(root)
                .join("Documents")
                .join("Paradox Interactive")
                .join("Hearts of Iron IV");
            if path.is_dir() {
                return Ok(path);
            }
        }
    }
    Err("Paradox user directory was not found in the registry or Documents; pass --user-dir".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_request_is_renamed_whole_and_acknowledged() {
        let root = std::env::temp_dir().join(format!("hoi4-refresh-cli-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session = root.join("session");
        fs::create_dir_all(&session).unwrap();
        fs::write(
            root.join("active.json"),
            json!({"session_dir": session}).to_string(),
        )
        .unwrap();
        // A collector stand-in acknowledges the whole token it finds.
        let watcher = {
            let session = session.clone();
            std::thread::spawn(move || {
                for _ in 0..200 {
                    if let Ok(token) = fs::read_to_string(session.join("refresh.request")) {
                        assert!(!token.trim().is_empty());
                        fs::write(session.join("refresh.ack"), token.trim()).unwrap();
                        fs::remove_file(session.join("refresh.request")).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("no refresh request appeared");
            })
        };
        request_refresh(&root).unwrap();
        watcher.join().unwrap();
        // No temporary file is left behind.
        let leftovers: Vec<_> = fs::read_dir(&session)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        // Without an active recorder the request is refused.
        assert!(request_refresh(&session).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn late_terminal_drain_errors_cannot_be_reported_as_clean_detach() {
        let status = json!({"state":"detached","session":7,"drain_complete":false,"error":"final spool sync failed"});
        let outcome = classify_terminal(
            7,
            true,
            false,
            Some(&status),
            Some("final spool sync failed\n"),
        );
        assert_eq!(outcome.drain_complete, Some(false));
        assert!(
            outcome
                .issues
                .iter()
                .any(|issue| issue.contains("final spool sync failed"))
        );
        assert_eq!(
            outcome
                .issues
                .iter()
                .filter(|issue| issue.contains("final spool sync failed"))
                .count(),
            1
        );
        let completed = json!({"partial":true,"issues":outcome.issues,"detach_requested":true,"detached":false,"process_exited":false,"drain_complete":false});
        assert!(
            completed_detach(&completed)
                .unwrap_err()
                .contains("final spool sync failed")
        );
    }

    #[test]
    fn historical_recovered_warnings_do_not_make_a_completed_drain_partial() {
        let status = json!({"state":"detached","session":7,"drain_complete":true,"warning":"segment write retried","last_error":"segment write retried","write_errors":1});
        let outcome = classify_terminal(7, true, false, Some(&status), None);
        assert_eq!(outcome.drain_complete, Some(true));
        assert!(outcome.issues.is_empty());
        assert_eq!(outcome.warnings, ["segment write retried"]);
        let completed = json!({"partial":false,"issues":[],"detach_requested":true,"detached":true,"process_exited":false,"drain_complete":true});
        assert!(completed_detach(&completed).unwrap());
        let late_file = classify_terminal(
            7,
            true,
            false,
            Some(&status),
            Some("cannot publish terminal status"),
        );
        assert!(
            late_file
                .issues
                .iter()
                .any(|issue| issue.contains("cannot publish terminal status"))
        );
    }

    #[test]
    fn process_exit_recovery_does_not_require_a_detach_record_or_terminal_drain() {
        let running = json!({"state":"recording","session":7,"last_hour":100,"warning":"earlier segment retried"});
        assert!(
            classify_terminal(7, false, true, Some(&running), None)
                .issues
                .is_empty()
        );
        assert!(
            classify_terminal(7, false, true, None, None)
                .issues
                .is_empty()
        );
        let unavailable_status = classify_terminal(7, false, true, Some(&json!({})), None);
        assert!(unavailable_status.issues.is_empty());
        assert!(!unavailable_status.warnings.is_empty());
        let interrupted = classify_terminal(7, true, true, Some(&running), None);
        assert!(
            interrupted
                .issues
                .iter()
                .any(|issue| issue.contains("game exited after detach was requested"))
        );
        let completed = json!({"partial":false,"issues":[],"detach_requested":false,"detached":false,"process_exited":true,"drain_complete":null});
        assert!(!completed_detach(&completed).unwrap());
    }

    #[test]
    fn missing_foreign_or_inconsistent_terminal_results_refuse_clean_success() {
        let foreign = json!({"state":"detached","session":8,"drain_complete":true});
        let outcome = classify_terminal(7, true, false, Some(&foreign), None);
        assert_eq!(outcome.drain_complete, None);
        assert!(!outcome.issues.is_empty());
        assert!(
            !classify_terminal(7, true, false, None, None)
                .issues
                .is_empty()
        );
        assert!(completed_detach(&json!({"replays":["file"]})).is_err());
        assert!(completed_detach(&json!({"partial":false,"issues":["late failure"]})).is_err());
        assert!(completed_detach(&json!({"partial":false,"issues":[],"detach_requested":true,"detached":true,"process_exited":false,"drain_complete":false})).is_err());
    }
}
