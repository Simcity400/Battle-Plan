//! Passive process discovery, injection, detach and finalization.
use replay::identity::IdentityCodec;
use crate::{
    directories::{default_capture_directory, resolve_user_directory},
    replay_io,
    windows::{self, Event, Process},
};
use engine::{PINNED_BUILD, build::hash_executable};
use installed_content::content::{GatherOptions, gather};
use replay::{
    identity::SessionStart,
    witness::{TerminalWitness, WITNESS_FILE_NAME},
};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::System::{
    Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
    Performance::QueryPerformanceFrequency,
};

/// Stop and discard requests for a capture. The command line's console
/// handler owns one; the tray recorder owns one per capture.
#[derive(Debug, Default)]
pub struct Control {
    stop: AtomicBool,
    discard: AtomicBool,
}

impl Control {
    pub const fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            discard: AtomicBool::new(false),
        }
    }

    /// Ask the capture to detach. `discard` drops this session's capture
    /// instead of saving it; only an explicit user choice may set it.
    pub fn stop(&self, discard: bool) {
        // The choice is visible before the stop that acts on it.
        self.discard.store(discard, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub fn discarding(&self) -> bool {
        self.discard.load(Ordering::Acquire)
    }

    /// Clear both requests before the next capture.
    pub fn reset(&self) {
        self.stop.store(false, Ordering::Release);
        self.discard.store(false, Ordering::Release);
    }
}

/// What a capture is doing, for a controller that shows it.
#[derive(Debug)]
pub enum Stage<'a> {
    /// Verifying the game and its content, injecting and preparing history.
    Preparing,
    /// The collector is sampling into `session`. `status` is its latest
    /// readable publication, delivered about every two seconds.
    Recording {
        session: &'a Path,
        status: Option<&'a Value>,
    },
    /// Collector absence or game exit is proven and the spool is being
    /// finalized. `records` counts finalization work done so far.
    Saving { records: u64 },
}

/// The end of one capture.
#[derive(Debug, Default)]
pub struct CaptureOutcome {
    /// A stop was requested (by the controller or the collector), rather than
    /// the game exiting on its own.
    pub detach_requested: bool,
    pub process_exited: bool,
    /// Recordings written or appended by this session.
    pub replays: Vec<PathBuf>,
    /// The session was discarded at the user's request; nothing was published.
    pub discarded: bool,
    /// The session ended because the controller refused a load, not because
    /// of a stop or the game exiting.
    pub rerouted: bool,
    /// Loads the session left out of its recordings; their hours stay in the
    /// session folder.
    pub omitted: Vec<Omission>,
}

/// Asks whether a load of a campaign is recorded; the flag says this session's
/// spool already holds the campaign's hours from an earlier load.
pub type AdmitLoad<'a> = dyn Fn(&str, bool) -> Result<(), Refusal> + 'a;

use crate::replay_io::write_json;
pub use crate::replay_io::{Omission, Refusal};

static CONSOLE: Control = Control::new();
unsafe extern "system" fn console_control(kind: u32) -> i32 {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        CONSOLE.stop(false);
        1
    } else {
        0
    }
}

struct Options {
    output: PathBuf,
    user: Option<PathBuf>,
    once: bool,
    game_score: bool,
    operation_outcomes: bool,
    politics_capture: bool,
    #[cfg(feature = "alloc-fault-injection")]
    alloc_fault: Option<AllocationFault>,
}

#[cfg(feature = "alloc-fault-injection")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AllocationFault {
    phase: String,
    ordinal: u64,
    after_hours: u64,
}

#[cfg(feature = "alloc-fault-injection")]
fn allocation_fault(value: &OsStr) -> Result<AllocationFault, String> {
    let (phase, ordinal) = value
        .to_str()
        .and_then(|value| value.split_once(':'))
        .ok_or(
            "--alloc-fault requires <phase>:<positive allocation ordinal>[:<committed-hours>]",
        )?;
    let (ordinal, after_hours) = if let Some((ordinal, after_hours)) = ordinal.split_once(':') {
        let after_hours = after_hours
            .parse::<u64>()
            .map_err(|_| "--alloc-fault requires a nonnegative committed-hours count")?;
        (ordinal, after_hours)
    } else {
        (ordinal, 0)
    };
    let ordinal = ordinal
        .parse::<u64>()
        .ok()
        .filter(|ordinal| *ordinal != 0)
        .ok_or("--alloc-fault requires a positive allocation ordinal")?;
    if phase.is_empty() {
        return Err("--alloc-fault requires a phase name".into());
    }
    Ok(AllocationFault {
        phase: phase.into(),
        ordinal,
        after_hours,
    })
}

#[cfg(feature = "alloc-fault-injection")]
fn fault_config(mut config: Value, fault: Option<&AllocationFault>) -> Value {
    if let Some(fault) = fault {
        let mut requested = json!({"phase":fault.phase,"ordinal":fault.ordinal});
        if fault.after_hours != 0 {
            requested["after_hours"] = json!(fault.after_hours);
        }
        config["alloc_fault"] = requested;
    }
    config
}

/// Keep the sampler grammar opaque to the recorder: the feature-enabled DLL
/// owns parsing, while an ordinary DLL names and refuses a nonempty request.
fn worker_sampler_config(mut config: Value, requested: Option<String>) -> Value {
    if let Some(requested) = requested {
        config["worker_sampler"] = Value::String(requested);
    }
    config
}

fn usage() -> String {
    #[cfg(feature = "alloc-fault-injection")]
    let fault_option = " [--alloc-fault <phase>:<ordinal>[:<committed-hours>]]";
    #[cfg(not(feature = "alloc-fault-injection"))]
    let fault_option = "";
    format!(
        "recorder [record] [--out <directory>] [--user-dir <Paradox directory>] [--once] [--no-game-score] [--disable-operation-outcomes] [--disable-politics-capture]{fault_option}
recorder detach [--out <directory>]
recorder refresh [--out <directory>]
recorder finalize <session-directory> [--out <directory>]
recorder inspect <campaign.battleplan>
recorder inspect <campaign.battleplan> --characters <substring> --hour <n> --branch <id>
{}
{}
{}
{}
{}
Environment: BATTLE_PLAN_WORKER_SAMPLER (comma-separated off, interval_us=<n>)
  Forwarded verbatim to collector attach; a nonempty value requires collector feature worker_sampler.
Environment: BATTLE_PLAN_MAP_EVENTS (comma-separated map event family names, or none)
  The map event families the collector installs; every family when unset. The others record unavailable coverage.",
        crate::metrics::USAGE,
        crate::oracle::USAGE,
        crate::movement_oracle::USAGE,
        crate::sizes::USAGE,
        crate::derived::USAGE
    )
}

/// The map event families named in `families` (comma-separated; `none` for
/// none), passed to the collector, which installs only those.
fn map_event_config(mut config: serde_json::Value, families: &str) -> serde_json::Value {
    let selected = families
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty() && *family != "none")
        .collect::<Vec<_>>();
    config["map_events"] = json!(selected);
    config
}

fn parse_options(command: &str, args: Vec<OsString>) -> Result<Options, String> {
    let mut options = Options {
        output: default_capture_directory(),
        user: None,
        once: false,
        game_score: true,
        operation_outcomes: true,
        politics_capture: true,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--out") => options.output = PathBuf::from(args.next().ok_or_else(usage)?),
            Some("--user-dir") if command == "record" => {
                options.user = Some(PathBuf::from(args.next().ok_or_else(usage)?))
            }
            Some("--once") if command == "record" => options.once = true,
            Some("--no-game-score") if command == "record" => options.game_score = false,
            Some("--disable-operation-outcomes") if command == "record" => {
                options.operation_outcomes = false
            }
            Some("--disable-politics-capture") if command == "record" => {
                options.politics_capture = false
            }
            #[cfg(feature = "alloc-fault-injection")]
            Some("--alloc-fault") if command == "record" && options.alloc_fault.is_none() => {
                options.alloc_fault = Some(allocation_fault(&args.next().ok_or_else(usage)?)?);
            }
            #[cfg(not(feature = "alloc-fault-injection"))]
            Some("--alloc-fault") if command == "record" => {
                return Err("--alloc-fault requires recorder feature alloc-fault-injection".into());
            }
            _ => return Err(usage()),
        }
    }
    Ok(options)
}

pub fn command(mut args: Vec<OsString>) -> Result<(), String> {
    // bench.ps1 passes these four positionals directly to its MetricsTool.
    if args.first().is_some_and(|arg| {
        Path::new(arg)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("battleplan") || extension.eq_ignore_ascii_case("hoi4replay"))
    }) {
        return crate::metrics::command(args);
    }
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
    if command == "metrics" {
        return crate::metrics::command(args);
    }
    if command == "sizes" {
        return crate::sizes::command(args);
    }
    if command == "derived" {
        return crate::derived::command(args);
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
    let mut options = parse_options(&command, args)?;
    fs::create_dir_all(&options.output).map_err(|e| e.to_string())?;
    options.output = options.output.canonicalize().map_err(|e| e.to_string())?;
    match command.as_str() {
        "record" => record(options),
        "detach" => request_detach(&options.output),
        "refresh" => request_refresh(&options.output),
        "finalize" => {
            let session = session.ok_or_else(usage)?;
            if session_discarded(&session) {
                return Err(format!(
                    "{} was discarded; there is nothing to finalize",
                    session.display()
                ));
            }
            let (process_exited, expected) = ensure_finalizable(&session)?;
            let _lock = finalization_lock(&options.output)?;
            if let Some(witness) = replay_io::read_failure_witness(&session)? {
                if witness.session != expected.id
                    || witness.process_start_time != expected.process_start_time
                {
                    return Err("collector failure witness belongs to a different process session; spool retained".into());
                }
                return finalize_failed_session(
                    &session,
                    &options.output,
                    &witness,
                    process_exited,
                );
            }
            replay_io::finalize_session(&session, &options.output)?;
            Ok(())
        }
        _ => Err(usage()),
    }
}

/// `finalize` of a session whose collector failed: publish what recovery
/// allows, then record the partial completion. A Battle Plan Recorder session's
/// admission record is never rewritten here, so repeating it publishes the
/// same admitted loads.
fn finalize_failed_session(
    session: &Path,
    output: &Path,
    witness: &TerminalWitness,
    process_exited: bool,
) -> Result<(), String> {
    publish_failure_status(session, witness)?;
    let mut issues = vec!["collector allocation failed".to_owned()];
    let finalized = match replay_io::finalize_session_outcome(session, output) {
        Ok(finalized) => finalized,
        Err(error) => {
            issues.push(format!("replay finalization failed: {error}"));
            replay_io::Finalized::default()
        }
    };
    issues.extend(
        finalized
            .errors
            .iter()
            .map(|error| format!("replay finalization failed: {error}")),
    );
    let omitted: Vec<_> = finalized.omitted.iter().map(Omission::to_json).collect();
    write_json(
        &session.join("completed.json"),
        &json!({
            "replays":finalized.replays,"detach_requested":true,"detached":!process_exited,
            "process_exited":process_exited,"drain_complete":witness.drain_complete,
            "capture_failed":true,"failure_code":"allocation_failed",
            "failed_hour":witness.failed_hour,"partial":true,
            "issues":issues,"warnings":[],"omitted":omitted,
        }),
    )?;
    Err(format!(
        "recording completion is partial: {}; spool retained at {}",
        issues.join("; "),
        session.display()
    ))
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

pub(crate) const TWO_GAMES: &str =
    "two hoi4.exe processes are unsupported; no second process will be recorded";
pub(crate) const UNSUPPORTED_BUILD: &str = "unsupported game build";
pub(crate) const COLLECTOR_LOADED: &str =
    "collector.dll is already loaded in this process; detach its recorder first";

pub(crate) fn checked_single_game() -> Result<Option<u32>, String> {
    match windows::games()?.as_slice() {
        [] => Ok(None),
        [pid] => Ok(Some(*pid)),
        _ => Err(TWO_GAMES.into()),
    }
}

fn record(options: Options) -> Result<(), String> {
    unsafe {
        SetConsoleCtrlHandler(Some(console_control), 1);
    }
    let user = options
        .user
        .clone()
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
        if CONSOLE.stopping() {
            return Ok(());
        }
        let Some(pid) = checked_single_game()? else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let outcome = capture(pid, &cli_capture(&options, &user), &CONSOLE, &|_| {})?;
        if outcome.detach_requested || options.once || CONSOLE.stopping() {
            return Ok(());
        }
        println!("Waiting for the next game process.");
    }
}

/// Where and what one capture records.
pub(crate) struct CaptureOptions<'a> {
    /// The capture root: session folders and the active record.
    pub output: &'a Path,
    /// Where prior history is read and finished recordings are published.
    pub target: &'a Path,
    pub user: &'a Path,
    pub game_score: bool,
    pub operation_outcomes: bool,
    pub politics_capture: bool,
    /// A session that reached no spool (the game closed at its menu, say) ends
    /// quietly instead of as a failed finalization.
    pub nothing_recorded_is_ok: bool,
    /// Read the collector's status every two seconds for `Stage::Recording`
    /// and admission. The command line reads it only to print, once a minute.
    pub live_status: bool,
    /// Whether a load the game names may be recorded in this session, asked
    /// once per load, live and again at save for loads the spool identifies
    /// that were never asked about; the flag says this session's spool already
    /// holds the campaign's hours from an earlier load. Each admitted load is
    /// recorded in the session folder as it is admitted, and only admitted
    /// loads are ever published. A live refusal ends the session. `None` (the
    /// command line) publishes every load.
    pub admit: Option<&'a AdmitLoad<'a>>,
    /// The campaign the game has loaded right now and its load signal, read
    /// without the collector. Only these, the spool at save, and `opening`
    /// name loads for admission.
    pub loaded: Option<&'a dyn Fn() -> Option<(String, i64)>>,
    /// The load this session attaches for, already chosen by the controller:
    /// admitted before the collector starts.
    pub opening: Option<(&'a str, i64)>,
    #[cfg(feature = "alloc-fault-injection")]
    pub alloc_fault: Option<&'a AllocationFault>,
}

/// The command line's capture: the capture root is also the publication folder,
/// every campaign is recorded, and status is read only to print it.
fn cli_capture<'a>(options: &'a Options, user: &'a Path) -> CaptureOptions<'a> {
    CaptureOptions {
        output: &options.output,
        target: &options.output,
        user,
        game_score: options.game_score,
        operation_outcomes: options.operation_outcomes,
        politics_capture: options.politics_capture,
        nothing_recorded_is_ok: false,
        live_status: false,
        admit: None,
        loaded: None,
        opening: None,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault: options.alloc_fault.as_ref(),
    }
}

/// The loads a Battle Plan Recorder session admitted. Each load the game names is
/// asked about once and, when admitted, recorded in the session folder before
/// anything depends on it.
struct Admission<'a> {
    session: &'a Path,
    target: &'a Path,
    seen: BTreeSet<(String, i64)>,
    admitted: replay_io::Admissions,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Asked about before.
    Seen,
    /// Admitted and recorded.
    Admitted,
    /// Refused: the session ends and the load is never published.
    Refused,
}

impl<'a> Admission<'a> {
    fn new(session: &'a Path, target: &'a Path) -> Self {
        Self {
            session,
            target,
            seen: BTreeSet::new(),
            admitted: replay_io::Admissions::default(),
        }
    }

    /// The load the session attaches for, chosen by the controller: admitted
    /// without asking again, before the collector starts.
    fn admit_opening(&mut self, campaign: &str, load: i64) -> Result<(), String> {
        self.seen.insert((campaign.to_owned(), load));
        self.admitted
            .admit(self.session, self.target, campaign, load)
    }

    /// Ask once per load. `recorded` (asked only for a new load) says the
    /// session's spool already holds the campaign's hours from an earlier
    /// load. Fails when an admitted load cannot be recorded; it is then not
    /// published. A refusal's reason is kept only to report what was left out.
    fn check(
        &mut self,
        campaign: &str,
        load: i64,
        recorded: impl FnOnce() -> bool,
        admit: &dyn Fn(&str, bool) -> Result<(), Refusal>,
    ) -> Result<Decision, String> {
        if !self.seen.insert((campaign.to_owned(), load)) {
            return Ok(Decision::Seen);
        }
        match admit(campaign, recorded()) {
            Ok(()) => {
                self.admitted
                    .admit(self.session, self.target, campaign, load)?;
                Ok(Decision::Admitted)
            }
            Err(refusal) => {
                if let Err(error) =
                    self.admitted
                        .refuse(self.session, self.target, campaign, load, refusal)
                {
                    eprintln!(
                        "Load {load} of campaign {campaign}: its refusal could not be recorded: {error}"
                    );
                }
                Ok(Decision::Refused)
            }
        }
    }

    /// At save, once nothing is being captured: ask about each load the spool
    /// identifies that was never asked about (one that recorded and ended
    /// between polls, when the game closed or Stop was pressed), by the same rule
    /// and in spool order.
    fn check_spooled(
        &mut self,
        admit: &dyn Fn(&str, bool) -> Result<(), Refusal>,
    ) -> Result<(), String> {
        for (campaign, load) in replay_io::spooled_loads(self.session)? {
            let campaign_dir = self.session.join(&campaign);
            self.check(
                &campaign,
                load,
                || replay_io::recorded_before_load(&campaign_dir, load),
                admit,
            )?;
        }
        Ok(())
    }
}

/// When capture reads the collector's status: once a minute to print it, and
/// with `live` (Battle Plan Recorder) every two seconds for its stage callback and
/// admission. The command line is never live.
struct StatusReads {
    live: bool,
    next_print: Instant,
    next_live: Instant,
}

impl StatusReads {
    const PRINT: Duration = Duration::from_secs(60);
    const LIVE: Duration = Duration::from_secs(2);

    fn new(live: bool, now: Instant) -> Self {
        Self {
            live,
            next_print: now,
            next_live: now,
        }
    }

    /// Whether a printed read is due at `now`, scheduling the next one.
    fn print_due(&mut self, now: Instant) -> bool {
        let due = now >= self.next_print;
        if due {
            self.next_print = now + Self::PRINT;
        }
        due
    }

    /// Whether a live read is due at `now`.
    fn live_due(&self, now: Instant) -> bool {
        self.live && now >= self.next_live
    }

    /// A live read finished at `now`.
    fn live_read(&mut self, now: Instant) {
        self.next_live = now + Self::LIVE;
    }
}

pub(crate) fn capture(
    pid: u32,
    options: &CaptureOptions<'_>,
    control: &Control,
    stage: &dyn Fn(Stage<'_>),
) -> Result<CaptureOutcome, String> {
    let CaptureOptions {
        output,
        target,
        user,
        game_score,
        operation_outcomes,
        politics_capture,
        nothing_recorded_is_ok,
        live_status,
        admit,
        loaded,
        opening,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault,
    } = *options;
    stage(Stage::Preparing);
    fs::create_dir_all(target).map_err(|error| failure_text(target, error))?;
    let process = Process::open(pid, true)?;
    let mut executable = File::open(&process.path).map_err(|e| e.to_string())?;
    let exe = hash_executable(&mut executable).map_err(|e| e.to_string())?;
    if !PINNED_BUILD.matches(&exe) {
        return Err(format!(
            "{UNSUPPORTED_BUILD}; wanted {} bytes / {}; found {} bytes / {}. Nothing recorded.",
            PINNED_BUILD.exe_size,
            hex(&PINNED_BUILD.exe_sha256),
            exe.size,
            hex(&exe.sha256)
        ));
    }
    engine::executable_tables::initialize_executable(&process.path)
        .map_err(|error| format!("Cannot prepare game tables: {error}"))?;
    let game_version = PINNED_BUILD
        .read_version_banner(&mut executable)
        .map_err(|e| format!("game version text: {e}"))?;
    if windows::modules(pid)?.iter().any(|module| {
        module
            .path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("collector.dll"))
    }) {
        return Err(COLLECTOR_LOADED.into());
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
        if control.stopping() {
            return Ok(CaptureOutcome {
                detach_requested: true,
                ..CaptureOutcome::default()
            });
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
            game_version,
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
    create_failure_witness(&directory, session.id, session.process_start_time)?;
    let mut admission = Admission::new(&directory, target);
    if admit.is_some() {
        // Before the collector starts: a Battle Plan Recorder session publishes only
        // the loads this record will list, starting with the one it attaches for.
        replay_io::begin_admissions(&directory, target)?;
        if let Some((campaign, load)) = opening {
            admission.admit_opening(campaign, load)?;
        }
    }
    let config = json!({"session_id":session.id,"output_dir":directory,"replay_dir":target,"session_start":session_bytes,"ready_event":ready_name,"detach_event":detach_name,"done_event":done_name,"cancel_file":cancel_file,"content_order_note":content.order_note,"expected_mounts":expected_mounts,"mod_descriptors":content.mod_descriptors,"game_score":game_score,"operation_outcomes":operation_outcomes,"politics_capture":politics_capture});
    let sampler = match std::env::var("BATTLE_PLAN_WORKER_SAMPLER").or_else(|_| std::env::var("HOI4_WORKER_SAMPLER")) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("BATTLE_PLAN_WORKER_SAMPLER is not valid Unicode".into());
        }
    };
    let config = worker_sampler_config(config, sampler);
    let config = match std::env::var("BATTLE_PLAN_MAP_EVENTS").or_else(|_| std::env::var("HOI4_MAP_EVENTS")) {
        Ok(families) => map_event_config(config, &families),
        Err(std::env::VarError::NotPresent) => config,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("BATTLE_PLAN_MAP_EVENTS is not valid Unicode".into());
        }
    };
    #[cfg(feature = "alloc-fault-injection")]
    let config = fault_config(config, alloc_fault);
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
    process.inject(&dll, || control.stopping())?;
    let error_path = bin.join("collector.error.txt");
    let mut wait = CollectorWait::new(Instant::now(), READY_STALL);
    let mut next_progress = Instant::now() + Duration::from_secs(10);
    while !ready.handle.signaled()? {
        if let Some(witness) = live_failure(&directory, session.id, session.process_start_time)? {
            publish_failure_status(&directory, &witness)?;
            detach.set()?;
            break;
        }
        if error_path.exists() {
            return Err(fs::read_to_string(&error_path).unwrap_or_else(|e| e.to_string()));
        }
        if process.handle.signaled()? {
            return Err("game exited before collector became ready".into());
        }
        if let Some(end) = wait.ended(Instant::now(), collector_stamp(&directory)) {
            return Err(format!(
                "collector readiness timed out {}; detach requested; spool retained",
                end.describe(&directory)
            ));
        }
        if control.stopping() {
            detach.set()?;
        }
        if Instant::now() >= next_progress {
            println!("Preparing collector and prior campaign history before hourly sampling...");
            next_progress = Instant::now() + Duration::from_secs(10);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(summary) = step_summary(&directory, "attach_started", "ready") {
        println!("Collector attach {summary}");
    }
    let active = output.join("active.json");
    write_json(
        &active,
        &json!({"recorder_pid":std::process::id(),"game_pid":pid,"session_id":session.id,"process_start_time":process.start_time,"session_dir":directory,"dll":dll,"detach_event":detach_name,"done_event":done_name}),
    )?;
    println!("Recording process {pid}. Spool: {}", directory.display());
    let status_path = directory.join("status.json");
    stage(Stage::Recording {
        session: &directory,
        status: None,
    });
    let mut status_reads = StatusReads::new(live_status, Instant::now());
    let mut admission_failures = Vec::new();
    let mut rerouted = false;
    let mut requested_detach = false;
    let mut second_reported = false;
    let mut next_discovery = Instant::now();
    loop {
        requested_detach |= control.stopping() || detach.handle.signaled()?;
        if process.handle.signaled()? {
            break;
        }
        if let Some(witness) = live_failure(&directory, session.id, session.process_start_time)? {
            publish_failure_status(&directory, &witness)?;
            eprintln!(
                "Collector capture failed at hour {:?} (phase {}, code {}); detaching and recovering the committed prefix.",
                witness.failed_hour, witness.phase, witness.code
            );
            detach.set()?;
            requested_detach = true;
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
        if status_reads.print_due(Instant::now()) {
            print_status(&status_path);
        }
        if status_reads.live_due(Instant::now()) {
            let status = read_status(&status_path);
            // Only a load the game names (campaign and load signal) is asked
            // about, usually while it is still paused; the collector's status
            // naming a campaign never is. Hours of a load not admitted, asked
            // about too late or never, are never published.
            if let Some(admit) = admit
                && let Some((campaign, load)) = loaded.and_then(|loaded| loaded())
            {
                let recorded = || replay_io::recorded_before_load(&directory.join(&campaign), load);
                match admission.check(&campaign, load, recorded, admit) {
                    Ok(Decision::Refused) => {
                        println!(
                            "Load {load} of campaign {campaign} is not recorded in this session; detaching."
                        );
                        requested_detach = true;
                        rerouted = true;
                    }
                    Ok(Decision::Admitted | Decision::Seen) => {}
                    Err(error) => {
                        eprintln!(
                            "Load {load} of campaign {campaign} could not be admitted: {error}"
                        );
                        admission_failures.push(error);
                        requested_detach = true;
                    }
                }
                if requested_detach {
                    detach.set()?;
                    break;
                }
            }
            stage(Stage::Recording {
                session: &directory,
                status: status.as_ref(),
            });
            status_reads.live_read(Instant::now());
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
        let mut wait = CollectorWait::new(Instant::now(), DONE_STALL);
        while !done.handle.signaled()? && !process.handle.signaled()? {
            if let Some(end) = wait.ended(Instant::now(), collector_stamp(&directory)) {
                return Err(format!(
                    "detach timed out {}; collector may still be loaded; spool retained and not finalized",
                    end.describe(&directory)
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        while collector_present(&process, &dll)? {
            if let Some(end) = wait.ended(Instant::now(), collector_stamp(&directory)) {
                return Err(format!(
                    "collector signaled done but remains loaded {}; spool retained and not finalized",
                    end.describe(&directory)
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if let Some(summary) = step_summary(&directory, "detach_requested", "done") {
            println!("Collector detach {summary}");
        }
    }
    // The collector can fail while draining, after signaling was requested. Its
    // files become terminal evidence only after unload or process termination.
    let process_exited = process.handle.signaled()?;
    print_status(&status_path);
    requested_detach |= control.stopping() || detach.handle.signaled()?;
    if control.discarding() {
        // The user chose Discard. Nothing from this session is finalized, so a
        // recording it would have continued keeps exactly its earlier contents.
        if process_exited {
            println!("Game process exited; discarding this session.");
        } else {
            println!(
                "Collector absence confirmed in the running game process; discarding this session."
            );
        }
        write_json(
            &directory.join("completed.json"),
            &json!({
                "replays":[],"discarded":true,"detach_requested":requested_detach,
                "detached":!process_exited,"process_exited":process_exited,
                "partial":false,"issues":[],"warnings":[],
            }),
        )?;
        remove_active(&active, &directory)?;
        discard_session(&directory)?;
        return Ok(CaptureOutcome {
            detach_requested: requested_detach,
            process_exited,
            replays: Vec::new(),
            discarded: true,
            rerouted,
            omitted: Vec::new(),
        });
    }
    // Saving, with the game no longer captured: a load that recorded and ended
    // between polls (the game closing, or Stop) is asked about now by the same
    // rule, so a wanted load is never left out for being brief.
    if let Some(admit) = admit
        && let Err(error) = admission.check_spooled(admit)
    {
        admission_failures.push(error);
    }
    let mut outcome = terminal_outcome(
        &directory,
        session.id,
        session.process_start_time,
        requested_detach,
        process_exited,
    );
    outcome.issues.extend(
        admission_failures
            .into_iter()
            .map(|error| format!("a load could not be admitted, so it is not published: {error}")),
    );
    if process_exited {
        println!("Game process exited; recovering the flushed spool.");
    } else {
        println!("Collector absence confirmed in the running game process.");
    }
    for warning in &outcome.warnings {
        eprintln!("Collector warning: {warning}");
    }
    if let Some(witness) = &outcome.witness
        && let Err(error) = publish_failure_status(&directory, witness)
    {
        outcome
            .issues
            .push(format!("terminal status publication failed: {error}"));
    }
    if !outcome.recovery_allowed
        && let Err(error) = write_json(
            &directory.join("status.json"),
            &json!({
                "state":"incomplete","session":session.id,"partial":true,
                "capture_failed":outcome.capture_failed,"failure_code":null,"failed_hour":null,
                "drain_complete":outcome.drain_complete,"error":outcome.issues.join("; "),
            }),
        )
    {
        outcome
            .issues
            .push(format!("incomplete status publication failed: {error}"));
    }
    // Nothing recorded means no spooled hours at all. Hours left out of the
    // recordings are reported by finalization, never as nothing recorded.
    if nothing_recorded_is_ok
        && outcome.recovery_allowed
        && outcome.issues.is_empty()
        && outcome.witness.is_none()
        && matches!(replay_io::has_campaign_spools(&directory), Ok(false))
    {
        println!("Nothing was recorded in this session.");
        write_json(
            &directory.join("completed.json"),
            &json!({
                "replays":[],"nothing_recorded":true,"detach_requested":requested_detach,
                "detached":!process_exited,"process_exited":process_exited,
                "drain_complete":outcome.drain_complete,"capture_failed":false,
                "partial":false,"issues":[],"warnings":outcome.warnings,
            }),
        )?;
        remove_active(&active, &directory)?;
        return Ok(CaptureOutcome {
            detach_requested: requested_detach,
            process_exited,
            replays: Vec::new(),
            discarded: false,
            rerouted,
            omitted: Vec::new(),
        });
    }
    let _lock = finalization_lock(target)?;
    stage(Stage::Saving { records: 0 });
    let saving = |records| stage(Stage::Saving { records });
    let progress = FinalizationProgress::new(&directory, &saving)?;
    let (finalized, omitted) = match outcome.recovery_allowed.then(|| {
        replay_io::finalize_session_outcome_with_progress(&directory, target, &|| {
            progress.advance()
        })
    }) {
        Some(Ok(done)) => {
            outcome.issues.extend(
                done.errors
                    .into_iter()
                    .map(|error| format!("replay finalization failed: {error}")),
            );
            (done.replays, done.omitted)
        }
        Some(Err(error)) => {
            outcome
                .issues
                .push(format!("replay finalization failed: {error}"));
            (Vec::new(), Vec::new())
        }
        None => (Vec::new(), Vec::new()),
    };
    let completed = json!({
        "replays":finalized,
        "omitted":omitted.iter().map(Omission::to_json).collect::<Vec<_>>(),
        "detach_requested":requested_detach,
        "detached":!process_exited,
        "process_exited":process_exited,
        "drain_complete":outcome.drain_complete,
        "capture_failed":outcome.capture_failed,
        "failure_code":outcome.witness.as_ref().map(|_| "allocation_failed"),
        "failed_hour":outcome.witness.as_ref().and_then(|witness| witness.failed_hour),
        "partial":!outcome.issues.is_empty(),
        "issues":outcome.issues,
        "warnings":outcome.warnings,
    });
    write_json(&directory.join("completed.json"), &completed)?;
    remove_active(&active, &directory)?;
    if !outcome.issues.is_empty() {
        return Err(format!(
            "recording completion is partial: {}; recoverable spool retained at {}",
            outcome.issues.join("; "),
            directory.display()
        ));
    }
    Ok(CaptureOutcome {
        detach_requested: requested_detach,
        process_exited,
        replays: finalized,
        discarded: false,
        rerouted,
        omitted,
    })
}

/// The active file is this recorder's own control record, outside the repository.
fn remove_active(active: &Path, directory: &Path) -> Result<(), String> {
    if fs::read(active)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value["session_dir"] == json!(directory))
    {
        fs::remove_file(active).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Remove a discarded session's spool, copied collector and evidence, keeping
/// only its completion record. Call only after collector absence or game exit.
fn discard_session(directory: &Path) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| failure_text(directory, error))? {
        let entry = entry.map_err(|error| failure_text(directory, error))?;
        if entry.file_name() == "completed.json" {
            continue;
        }
        let path = entry.path();
        let removed = if entry
            .file_type()
            .map_err(|error| failure_text(&path, error))?
            .is_dir()
        {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(|error| format!("discard {}: {error}", path.display()))?;
    }
    Ok(())
}

fn failure_text(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// Whether the session's completion record says the user discarded it.
fn session_discarded(directory: &Path) -> bool {
    read_status(&directory.join("completed.json")).is_some_and(|value| value["discarded"] == true)
}

/// A JSON publication, when it exists and parses.
fn read_status(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

#[derive(Debug)]
struct TerminalOutcome {
    drain_complete: Option<bool>,
    issues: Vec<String>,
    warnings: Vec<String>,
    witness: Option<TerminalWitness>,
    recovery_allowed: bool,
    capture_failed: bool,
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
        witness: None,
        recovery_allowed: true,
        capture_failed: status.is_some_and(|status| status["capture_failed"] == true),
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
        if matches!(
            status["state"].as_str(),
            Some("detached" | "capture_failed")
        ) && correct_session
        {
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
    process_start_time: u64,
    requested_detach: bool,
    process_exited: bool,
) -> TerminalOutcome {
    let mut status_read_issues = Vec::new();
    let mut error_read_issues = Vec::new();
    let status_path = directory.join("status.json");
    let mut status = match fs::read(&status_path) {
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
    let witness = replay_io::read_failure_witness(directory).and_then(|witness| {
        if witness.as_ref().is_some_and(|witness| {
            witness.session != session || witness.process_start_time != process_start_time
        }) {
            Err("collector failure witness belongs to a different process session".into())
        } else {
            Ok(witness)
        }
    });
    if let Ok(Some(witness)) = &witness {
        status = Some(failure_status(status, witness));
    }
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
    match witness {
        Ok(Some(witness)) => outcome.witness = Some(witness),
        Ok(None) => {
            if status
                .as_ref()
                .is_some_and(|status| status["capture_failed"] == true)
            {
                outcome.issues.push("collector capture failed without a valid terminal witness; spool retained and not finalized".into());
                outcome.recovery_allowed = false;
            } else if requested_detach && outcome.drain_complete != Some(true) {
                outcome.recovery_allowed = false;
            }
        }
        Err(error) => {
            outcome.issues.push(format!(
                "invalid collector failure witness: {error}; spool retained and not finalized"
            ));
            outcome.recovery_allowed = false;
        }
    }
    outcome
}

fn create_failure_witness(
    directory: &Path,
    session: u64,
    process_start_time: u64,
) -> Result<(), String> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(WITNESS_FILE_NAME))
        .map_err(|error| error.to_string())?;
    file.write_all(&TerminalWitness::prepared(session, process_start_time))
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

fn failure_status(status: Option<Value>, witness: &TerminalWitness) -> Value {
    let mut status = status
        .filter(|status| status.is_object() && status["session"].as_u64() == Some(witness.session))
        .unwrap_or_else(|| json!({}));
    status["state"] = json!("capture_failed");
    status["session"] = json!(witness.session);
    status["drain_complete"] = json!(witness.drain_complete);
    status["capture_failed"] = json!(true);
    status["failure_code"] = json!("allocation_failed");
    status["failure_phase"] = json!(witness.phase);
    status["failure_detail"] = json!(witness.code);
    status["failed_hour"] = json!(witness.failed_hour);
    status["partial"] = json!(true);
    status["error"] = json!("collector allocation failed");
    status
}

fn publish_failure_status(directory: &Path, witness: &TerminalWitness) -> Result<(), String> {
    let path = directory.join("status.json");
    let previous = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let mut status = failure_status(previous, witness);
    if let Ok(bytes) = fs::read(directory.join("collector-failure-detail.json"))
        && let Ok(detail) = serde_json::from_slice::<Value>(&bytes)
        && detail["session"].as_u64() == Some(witness.session)
        && detail["process_start_time"].as_u64() == Some(witness.process_start_time)
        && detail["family"].is_string()
        && detail["requested_bytes"].is_u64()
    {
        status["allocation_family"] = detail["family"].clone();
        status["allocation_requested_bytes"] = detail["requested_bytes"].clone();
    }
    write_json(&path, &status)
}

/// A complete first-failure witness is authoritative while capture is loaded.
/// A concurrent fixed-record write may not yet have a valid checksum; terminal
/// recovery still validates all evidence strictly after collector absence.
fn live_failure(
    directory: &Path,
    session: u64,
    process_start: u64,
) -> Result<Option<TerminalWitness>, String> {
    let Ok(Some(witness)) = replay_io::read_failure_witness(directory) else {
        return Ok(None);
    };
    if witness.session != session || witness.process_start_time != process_start {
        return Err(
            "collector failure witness belongs to a different process session; spool retained"
                .into(),
        );
    }
    Ok(Some(witness))
}

fn refuse_failed_refresh(directory: &Path, active: &Value) -> Result<(), String> {
    let process_start = active["process_start_time"]
        .as_u64()
        .ok_or("active recorder has no process start time")?;
    let session = active["session_id"].as_u64().unwrap_or(process_start);
    if let Some(witness) = live_failure(directory, session, process_start)? {
        publish_failure_status(directory, &witness)?;
        return Err(format!(
            "collector capture failed at hour {:?} (phase {}, code {}); refresh unavailable; detach recovers the committed prefix",
            witness.failed_hour, witness.phase, witness.code
        ));
    }
    Ok(())
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
    refuse_failed_refresh(&session_dir, &active)?;
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
        if let Err(error) = refuse_failed_refresh(&session_dir, &active) {
            if fs::read_to_string(&request).is_ok_and(|text| text.trim() == token) {
                let _ = fs::remove_file(&request);
            }
            return Err(error);
        }
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
    let recorder_pid = active["recorder_pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or("active recorder has no valid recorder pid")?;
    let recorder = Process::open(recorder_pid, false)
        .map_err(|error| format!("recording controller unavailable: {error}"))?;
    Event::open(name)?.set()?;
    println!("Detach requested. Waiting for recorder finalization.");
    let mut watch = ProgressWatch::new(Instant::now());
    while !session_dir.join("completed.json").exists() {
        if recorder.handle.signaled()? {
            // Completion publication and controller exit can coincide.
            if session_dir.join("completed.json").exists() {
                break;
            }
            return Err(
                "recording controller exited before detach/finalization completed; spool retained"
                    .into(),
            );
        }
        if watch.stalled(Instant::now(), progress_stamp(&session_dir)) {
            return Err("detach/finalization stopped making progress; inspect the recorder console and retained spool".into());
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

// This is an inactivity timeout, never a limit on total drain/finalization time.
// While the collector detaches it follows the collector's own steps, so it
// allows what the recording controller allows (DONE_STALL).
const DETACH_STALL_TIMEOUT: Duration = DONE_STALL;
/// The detach command's watch over the session's status and finalization.
type ProgressWatch = StampWatch<(Option<SystemTime>, Option<SystemTime>)>;
struct StampWatch<S> {
    last_progress: Instant,
    stamp: S,
    stall: Duration,
}
impl<S: Default + PartialEq> StampWatch<S> {
    fn new(now: Instant) -> Self {
        Self::with_stall(now, DETACH_STALL_TIMEOUT)
    }
    fn with_stall(now: Instant, stall: Duration) -> Self {
        Self {
            last_progress: now,
            stamp: S::default(),
            stall,
        }
    }
    fn stalled(&mut self, now: Instant, stamp: S) -> bool {
        if stamp != self.stamp {
            self.stamp = stamp;
            self.last_progress = now;
        }
        now.duration_since(self.last_progress) >= self.stall
    }
}
fn progress_stamp(directory: &Path) -> (Option<SystemTime>, Option<SystemTime>) {
    let modified = |name| {
        fs::metadata(directory.join(name))
            .ok()
            .and_then(|metadata| metadata.modified().ok())
    };
    // After unload, the controller reports actual metadata/frame work. Earlier
    // collector status publications act as its drain heartbeat.
    let finalization = modified("finalization-progress.json");
    (
        if finalization.is_none() {
            modified("status.json").max(modified(COLLECTOR_PROGRESS))
        } else {
            None
        },
        finalization,
    )
}

/// The collector's step log in its session directory (`progress` in the
/// collector), one line per attach or detach step: milliseconds since attach
/// began, the step and an optional count.
const COLLECTOR_PROGRESS: &str = "progress";
/// Readiness and detach wait while the collector takes steps (loading a
/// campaign history; installing, restoring and quiescing each hook family;
/// joining its workers; draining its queues), up to this long for one step.
/// A hook publication suspends every game thread and cannot log while they
/// are held; on a loaded machine one took 43.9 s (2d1ede901c 1936 burn-in,
/// where the game's working set was being paged out), so the limit leaves
/// room for several times that.
const READY_STALL: Duration = Duration::from_secs(180);
const DONE_STALL: Duration = Duration::from_secs(180);
/// No attach or detach waits longer in total, progressing or not.
const COLLECTOR_WAIT_CAP: Duration = Duration::from_secs(15 * 60);

/// The collector's progress: its step log's length and its status's time.
type CollectorStamp = (Option<u64>, Option<SystemTime>);

fn collector_stamp(directory: &Path) -> CollectorStamp {
    (
        fs::metadata(directory.join(COLLECTOR_PROGRESS))
            .ok()
            .map(|metadata| metadata.len()),
        fs::metadata(directory.join("status.json"))
            .ok()
            .and_then(|metadata| metadata.modified().ok()),
    )
}

/// A controller wait on the collector: it continues while the collector's
/// stamp changes within `stall`, up to [`COLLECTOR_WAIT_CAP`] in total.
struct CollectorWait {
    started: Instant,
    watch: StampWatch<CollectorStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitEnd {
    /// No collector step for this long.
    Stalled(Duration),
    /// The total cap.
    Capped,
}

impl WaitEnd {
    /// "after …", naming the collector's last step.
    fn describe(self, directory: &Path) -> String {
        let step = fs::read_to_string(directory.join(COLLECTOR_PROGRESS))
            .ok()
            .and_then(|text| last_step(&text))
            .unwrap_or_else(|| "none recorded".into());
        match self {
            Self::Stalled(stall) => format!(
                "after {} s without collector progress (last step: {step})",
                stall.as_secs()
            ),
            Self::Capped => format!(
                "after the {} s cap (last step: {step})",
                COLLECTOR_WAIT_CAP.as_secs()
            ),
        }
    }
}

impl CollectorWait {
    fn new(now: Instant, stall: Duration) -> Self {
        Self {
            started: now,
            watch: StampWatch::with_stall(now, stall),
        }
    }

    fn ended(&mut self, now: Instant, stamp: CollectorStamp) -> Option<WaitEnd> {
        if self.watch.stalled(now, stamp) {
            Some(WaitEnd::Stalled(self.watch.stall))
        } else if now.duration_since(self.started) >= COLLECTOR_WAIT_CAP {
            Some(WaitEnd::Capped)
        } else {
            None
        }
    }
}

/// `(milliseconds, step)` of each step-log line.
fn steps(text: &str) -> impl Iterator<Item = (u64, &str)> {
    text.lines().filter_map(|line| {
        let (millis, step) = line.split_once(' ')?;
        Some((millis.parse().ok()?, step))
    })
}

/// The last step, with its time.
fn last_step(text: &str) -> Option<String> {
    let (millis, step) = steps(text).last()?;
    Some(format!("{step} at {:.1} s", millis as f64 / 1000.0))
}

/// "took N s; slowest steps: …" over the steps from the last `from` to the
/// first `to` after it, each step timed from the one before it.
fn steps_between(text: &str, from: &str, to: &str) -> Option<String> {
    let all: Vec<(u64, &str)> = steps(text).collect();
    let first = all.iter().rposition(|(_, step)| *step == from)?;
    let last = first + all[first..].iter().position(|(_, step)| *step == to)?;
    let span = &all[first..=last];
    let mut durations: Vec<(u64, &str)> = span
        .windows(2)
        .map(|pair| (pair[1].0.saturating_sub(pair[0].0), pair[1].1))
        .collect();
    durations.sort_by_key(|&(millis, _)| std::cmp::Reverse(millis));
    let slowest: Vec<String> = durations
        .iter()
        .take(5)
        .map(|(millis, step)| format!("{step} {:.1} s", *millis as f64 / 1000.0))
        .collect();
    Some(format!(
        "took {:.1} s; slowest steps: {}",
        (span[span.len() - 1].0 - span[0].0) as f64 / 1000.0,
        slowest.join(", ")
    ))
}

fn step_summary(directory: &Path, from: &str, to: &str) -> Option<String> {
    steps_between(
        &fs::read_to_string(directory.join(COLLECTOR_PROGRESS)).ok()?,
        from,
        to,
    )
}
struct FinalizationProgress<'a> {
    directory: &'a Path,
    /// Told the record count whenever it is published.
    observe: &'a dyn Fn(u64),
    records: Cell<u64>,
    last_write: Cell<Instant>,
}
impl<'a> FinalizationProgress<'a> {
    fn new(directory: &'a Path, observe: &'a dyn Fn(u64)) -> Result<Self, String> {
        write_json(
            &directory.join("finalization-progress.json"),
            &json!({"recorder_pid": std::process::id(), "records": 0u64}),
        )?;
        Ok(Self {
            directory,
            observe,
            records: Cell::new(0),
            last_write: Cell::new(Instant::now()),
        })
    }
    fn advance(&self) -> std::io::Result<()> {
        self.records.set(self.records.get().saturating_add(1));
        let now = Instant::now();
        if now.duration_since(self.last_write.get()) >= Duration::from_secs(1) {
            write_json(
                &self.directory.join("finalization-progress.json"),
                &json!({"recorder_pid": std::process::id(), "records": self.records.get()}),
            )
            .map_err(std::io::Error::other)?;
            self.last_write.set(now);
            (self.observe)(self.records.get());
        }
        Ok(())
    }
}

fn print_status(path: &Path) {
    if let Ok(bytes) = fs::read(path)
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
    {
        if let Some((site, reason)) = destruction_refusal(&value) {
            eprintln!("EquipmentDestruction refusal site={site}: {reason}");
        }
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

fn destruction_refusal(status: &Value) -> Option<(u64, &str)> {
    let refusal = &status["equipment_destruction"]["refusal"];
    Some((refusal["site_index"].as_u64()?, refusal["reason"].as_str()?))
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

/// Returns whether the original process has exited, after proving DLL absence.
fn ensure_finalizable(directory: &Path) -> Result<(bool, SessionStart), String> {
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
        return Ok((true, session));
    }
    let process = match Process::open(session.process_id, false) {
        Ok(process) => process,
        Err(_) if !windows::games()?.contains(&session.process_id) => return Ok((true, session)),
        Err(error) => return Err(error),
    };
    if process.start_time != session.process_start_time {
        return Ok((true, session));
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
        if !directory.join("completed.json").is_file()
            && !replay_io::read_failure_witness(directory)?.is_some_and(|witness| {
                witness.session == session.id
                    && witness.process_start_time == session.process_start_time
            })
        {
            return Err(
                "live process has no completed detach witness; wait for its recorder or game exit"
                    .into(),
            );
        }
    }
    Ok((process.handle.signaled()?, session))
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

#[cfg(test)]
mod tests {
    use super::*;
    use replay::container::Replay;
    use std::{cell::RefCell, collections::BTreeMap};

    #[test]
    fn detach_wait_follows_progress_past_the_old_total_deadline_and_detects_stalls() {
        let start = Instant::now();
        let mut watch = ProgressWatch::new(start);
        for seconds in [0, 60, 119, 127, 240, 1000] {
            let stamp = Some(UNIX_EPOCH + Duration::from_secs(seconds));
            assert!(!watch.stalled(start + Duration::from_secs(seconds), (None, stamp)));
        }
        // The detach command allows the controller's 180 s without collector
        // progress: 150 s passes (the old 120 s limit failed it), 180 s ends.
        let stamp = (None, Some(UNIX_EPOCH + Duration::from_secs(1000)));
        assert!(!watch.stalled(start + Duration::from_secs(1150), stamp));
        assert!(!watch.stalled(start + Duration::from_secs(1179), stamp));
        assert!(watch.stalled(start + Duration::from_secs(1180), stamp));
    }

    /// The 48da4248f1 detach: the collector took steps for 92 s before done.
    /// The controller's old fixed 60 s deadline gave up at 60 s with the
    /// collector still loaded; the progress wait follows the steps, still
    /// ends a collector that stops taking them, and caps the total.
    #[test]
    fn detach_wait_follows_collector_steps_and_ends_a_stall() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut wait = CollectorWait::new(start, DONE_STALL);
        let mut length = 0;
        for second in 0..=92 {
            if second % 5 == 0 {
                length += 40;
            }
            assert_eq!(
                wait.ended(at(second), (Some(length), None)),
                None,
                "{second} s"
            );
        }
        // One step of 120 s (a hook publication on a loaded machine, like
        // the 2d1ede901c 1936 burn-in's 43.9 s Opinion restore and worse)
        // continues; a collector that stops is ended 180 s after its last
        // step, not before. The old 60 s limit ended both.
        let mut wait = CollectorWait::new(start, DONE_STALL);
        assert_eq!(wait.ended(at(10), (Some(40), None)), None);
        assert_eq!(wait.ended(at(130), (Some(80), None)), None, "a 120 s step");
        assert_eq!(wait.ended(at(309), (Some(80), None)), None);
        assert_eq!(
            wait.ended(at(310), (Some(80), None)),
            Some(WaitEnd::Stalled(Duration::from_secs(180)))
        );
        let mut ready = CollectorWait::new(start, READY_STALL);
        // Readiness: the step seen at 10 s, unchanged at 160 s (the old
        // 120 s limit ended it) and ended at 190 s.
        assert_eq!(ready.ended(at(10), (Some(40), None)), None);
        assert_eq!(
            ready.ended(at(160), (Some(40), None)),
            None,
            "a 150 s attach step"
        );
        assert_eq!(
            ready.ended(at(190), (Some(40), None)),
            Some(WaitEnd::Stalled(Duration::from_secs(180)))
        );
        // Status writes are progress too; the cap ends any wait.
        let mut wait = CollectorWait::new(start, DONE_STALL);
        let cap = COLLECTOR_WAIT_CAP.as_secs();
        for second in (0..cap).step_by(30) {
            let stamp = (None, Some(UNIX_EPOCH + Duration::from_secs(second)));
            assert_eq!(wait.ended(at(second), stamp), None);
        }
        let stamp = (None, Some(UNIX_EPOCH + Duration::from_secs(cap)));
        assert_eq!(wait.ended(at(cap), stamp), Some(WaitEnd::Capped));
    }

    #[test]
    fn step_log_names_the_last_step_and_times_the_slowest() {
        let log = "0 attach_started\n15 ready\n90000 detach_requested\n90100 restore Hourly done\n\
                   120100 restore Raid done\n121000 spool_draining 3\n182400 done\n";
        assert_eq!(last_step(log).unwrap(), "done at 182.4 s");
        assert_eq!(
            steps_between(log, "detach_requested", "done").unwrap(),
            "took 92.4 s; slowest steps: done 61.4 s, restore Raid done 30.0 s, \
             spool_draining 3 0.9 s, restore Hourly done 0.1 s"
        );
        assert_eq!(
            steps_between(log, "attach_started", "ready").unwrap(),
            "took 0.0 s; slowest steps: ready 0.0 s"
        );
        assert_eq!(steps_between(log, "detach_requested", "missing"), None);
        assert_eq!(last_step(""), None);
    }

    #[test]
    fn politics_capture_defaults_on_and_can_only_be_disabled_for_recording() {
        assert!(parse_options("record", vec![]).unwrap().politics_capture);
        let option: OsString = "--disable-politics-capture".into();
        assert!(
            !parse_options("record", vec![option.clone()])
                .unwrap()
                .politics_capture
        );
        for command in ["detach", "refresh", "finalize"] {
            assert!(parse_options(command, vec![option.clone()]).is_err());
        }
        #[cfg(feature = "alloc-fault-injection")]
        {
            let options = parse_options(
                "record",
                vec![option, "--alloc-fault".into(), "callback:1".into()],
            )
            .unwrap();
            assert!(!options.politics_capture);
            assert_eq!(options.alloc_fault.unwrap().ordinal, 1);
        }
    }

    #[test]
    fn game_score_cli_defaults_enabled_and_disables_at_attach() {
        assert!(parse_options("record", Vec::new()).unwrap().game_score);
        let options = parse_options(
            "record",
            vec![
                "--once".into(),
                "--no-game-score".into(),
                "--out".into(),
                "output".into(),
                "--user-dir".into(),
                "user".into(),
            ],
        )
        .unwrap();
        assert!(!options.game_score);
        assert!(options.once);
        assert_eq!(options.output, PathBuf::from("output"));
        assert_eq!(options.user, Some(PathBuf::from("user")));
        assert!(usage().contains("[--no-game-score]"));
    }

    #[test]
    fn worker_sampler_config_preserves_environment_text_verbatim() {
        let original = json!({"game_score": true, "output_dir": "synthetic/session"});
        assert_eq!(worker_sampler_config(original.clone(), None), original);
        for requested in ["", "off", r"interval_us=50"] {
            let config = worker_sampler_config(original.clone(), Some(requested.into()));
            assert_eq!(config["worker_sampler"], requested);
            assert_eq!(config["game_score"], original["game_score"]);
            assert_eq!(config["output_dir"], original["output_dir"]);
        }
        assert!(usage().contains("BATTLE_PLAN_WORKER_SAMPLER"));
    }

    #[test]
    fn game_score_cli_refuses_disable_for_other_commands() {
        for command in ["detach", "refresh", "finalize"] {
            assert!(parse_options(command, vec!["--no-game-score".into()]).is_err());
        }
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    fn allocation_fault_cli_requires_a_named_phase_and_positive_ordinal() {
        assert_eq!(
            allocation_fault(OsStr::new("callback:17")).unwrap(),
            AllocationFault {
                phase: "callback".into(),
                ordinal: 17,
                after_hours: 0,
            }
        );
        assert_eq!(
            allocation_fault(OsStr::new("encode:1:3")).unwrap(),
            AllocationFault {
                phase: "encode".into(),
                ordinal: 1,
                after_hours: 3,
            }
        );
        assert_eq!(
            allocation_fault(OsStr::new("teardown:1:0"))
                .unwrap()
                .after_hours,
            0
        );
        for value in [
            "callback",
            "callback:0",
            "callback:-1",
            ":1",
            "callback:1:",
            "callback:1:-1",
            "callback:1:2:3",
            "callback:1:18446744073709551616",
        ] {
            assert!(allocation_fault(OsStr::new(value)).is_err(), "{value}");
        }
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    fn allocation_fault_json_is_omitted_until_requested() {
        let healthy = json!({"session_id":7});
        assert_eq!(fault_config(healthy.clone(), None), healthy);
        let fault = allocation_fault(OsStr::new("spool:540")).unwrap();
        assert_eq!(
            fault_config(healthy.clone(), Some(&fault)),
            json!({"session_id":7,"alloc_fault":{"phase":"spool","ordinal":540}})
        );
        let fault = allocation_fault(OsStr::new("encode:1:3")).unwrap();
        assert_eq!(
            fault_config(healthy, Some(&fault)),
            json!({"session_id":7,"alloc_fault":{"phase":"encode","ordinal":1,"after_hours":3}})
        );
    }

    #[cfg(not(feature = "alloc-fault-injection"))]
    #[test]
    fn allocation_fault_cli_refuses_feature_option_in_default_build() {
        assert!(!usage().contains("--alloc-fault"));
        assert_eq!(
            command(vec![
                "record".into(),
                "--alloc-fault".into(),
                "spool:1".into()
            ])
            .unwrap_err(),
            "--alloc-fault requires recorder feature alloc-fault-injection"
        );
    }

    #[test]
    fn destruction_console_diagnostic_retains_the_site_and_reason() {
        for site in 0..4 {
            let status = json!({"equipment_destruction": {
                "refusal": {"site_index": site, "reason": "synthetic pool increase"}
            }});
            assert_eq!(
                destruction_refusal(&status),
                Some((site, "synthetic pool increase"))
            );
        }
        for status in [
            json!({}),
            json!({"equipment_destruction": {"refusal": null}}),
        ] {
            assert_eq!(destruction_refusal(&status), None);
        }
        let status = json!({"equipment_destruction": {
            "refusal": {"site_index": null, "reason": "buffer allocation failed"}
        }});
        assert_eq!(destruction_refusal(&status), None);
    }

    #[test]
    fn allocation_failure_witness_overrides_stale_success_and_publishes_terminal_status() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let directory = allocation_test_directory("terminal-status");
        create_failure_witness(&directory, 7, 13).unwrap();
        assert!(
            replay_io::read_failure_witness(&directory)
                .unwrap()
                .is_none()
        );
        assert!(create_failure_witness(&directory, 7, 13).is_err());
        write_json(
            &directory.join("status.json"),
            &json!({
                "session":7,"state":"detached","drain_complete":true,"hours":3,
            }),
        )
        .unwrap();
        let witness = allocation_witness(1);
        fs::write(directory.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let outcome = terminal_outcome(&directory, 7, 13, true, false);
        assert_eq!(outcome.witness, Some(witness));
        assert_eq!(outcome.drain_complete, Some(false));
        assert!(outcome.recovery_allowed);
        assert!(
            outcome
                .issues
                .iter()
                .any(|issue| issue.contains("collector allocation failed"))
        );
        publish_failure_status(&directory, &witness).unwrap();
        let status: Value =
            serde_json::from_slice(&fs::read(directory.join("status.json")).unwrap()).unwrap();
        assert_eq!(status["state"], "capture_failed");
        assert_eq!(status["capture_failed"], true);
        assert_eq!(status["failure_code"], "allocation_failed");
        assert_eq!(status["failed_hour"], 99);
        assert_eq!(status["partial"], true);
        assert_eq!(status["drain_complete"], false);
        assert_eq!(status["hours"], 3);
        let completed = json!({"partial":true,"capture_failed":true,"failure_code":"allocation_failed","failed_hour":99,"issues":outcome.issues});
        assert!(completed_detach(&completed).is_err());
    }

    #[test]
    fn live_failure_refuses_refresh_and_authenticates_request_diagnostics() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let directory = allocation_test_directory("live-failure-refresh");
        create_failure_witness(&directory, 7, 13).unwrap();
        let active = json!({"session_id":7,"process_start_time":13});
        assert_eq!(live_failure(&directory, 7, 13).unwrap(), None);
        assert!(refuse_failed_refresh(&directory, &active).is_ok());
        let witness = allocation_witness(1);
        fs::write(directory.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        write_json(
            &directory.join("status.json"),
            &json!({"session":7,"state":"recording","hours":16}),
        )
        .unwrap();
        for session in [8, 7] {
            write_json(&directory.join("collector-failure-detail.json"), &json!({
                "session":session,"process_start_time":13,"family":"raw bytes","requested_bytes":322536576u64
            })).unwrap();
            assert_eq!(live_failure(&directory, 7, 13).unwrap(), Some(witness));
            assert!(live_failure(&directory, 7, 14).is_err());
            let error = refuse_failed_refresh(&directory, &active).unwrap_err();
            assert!(error.contains("capture failed") && error.contains("refresh unavailable"));
            let status: Value =
                serde_json::from_slice(&fs::read(directory.join("status.json")).unwrap()).unwrap();
            assert_eq!(status["state"], "capture_failed");
            assert_eq!(status["hours"], 16);
            if session == 7 {
                assert_eq!(status["allocation_family"], "raw bytes");
                assert_eq!(status["allocation_requested_bytes"], 322536576u64);
            } else {
                assert!(status.get("allocation_family").is_none());
            }
        }
    }

    #[test]
    fn allocation_failure_missing_invalid_or_foreign_witness_never_reports_success() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let directory = allocation_test_directory("terminal-refusals");
        write_json(
            &directory.join("status.json"),
            &json!({
                "session":7,"state":"detached","drain_complete":true,"capture_failed":true,
            }),
        )
        .unwrap();
        let missing = terminal_outcome(&directory, 7, 13, true, false);
        assert!(!missing.recovery_allowed);
        assert!(!missing.issues.is_empty());
        fs::create_dir_all(directory.join("bin")).unwrap();
        fs::write(directory.join("bin/collector.json"), b"configured").unwrap();
        write_json(
            &directory.join("status.json"),
            &json!({"session":7,"state":"detached","drain_complete":true}),
        )
        .unwrap();
        let missing = terminal_outcome(&directory, 7, 13, true, false);
        assert!(!missing.recovery_allowed);
        assert!(
            missing
                .issues
                .iter()
                .any(|issue| issue.contains("precreated terminal witness is missing"))
        );
        let witness = allocation_witness(1);
        fs::write(directory.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let foreign = terminal_outcome(&directory, 7, 14, true, false);
        assert!(!foreign.recovery_allowed);
        assert!(foreign.witness.is_none());
        let mut invalid = witness.encode();
        invalid[31] ^= 1;
        fs::write(directory.join(WITNESS_FILE_NAME), invalid).unwrap();
        let invalid = terminal_outcome(&directory, 7, 13, true, false);
        assert!(!invalid.recovery_allowed);
        assert!(invalid.witness.is_none());
    }

    #[test]
    fn refresh_request_is_renamed_whole_and_acknowledged() {
        let root = std::env::temp_dir().join(format!("hoi4-refresh-cli-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session = root.join("session");
        fs::create_dir_all(&session).unwrap();
        fs::write(
            root.join("active.json"),
            json!({"session_dir": session,"session_id":7,"process_start_time":13}).to_string(),
        )
        .unwrap();
        // A collector stand-in acknowledges the whole token it finds.
        let watcher = {
            let session = session.clone();
            std::thread::spawn(move || {
                // Only a hang guard: a busy machine can delay the request.
                let deadline = Instant::now() + Duration::from_secs(120);
                while Instant::now() < deadline {
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
    fn control_requests_discard_only_when_asked_and_resets_between_captures() {
        let control = Control::new();
        assert!(!control.stopping() && !control.discarding());
        control.stop(false);
        assert!(control.stopping() && !control.discarding());
        control.reset();
        assert!(!control.stopping() && !control.discarding());
        control.stop(true);
        assert!(control.stopping() && control.discarding());
        control.reset();
        assert!(!control.stopping() && !control.discarding());
    }

    fn published_hours(path: &Path) -> Vec<(replay::BranchId, replay::Hour)> {
        let replay = Replay::open(fs::read(path).unwrap()).unwrap();
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| (frame.branch, frame.hour))
            .collect()
    }

    #[test]
    fn admission_asks_once_per_load_and_records_each_decision_on_disk() {
        use crate::test_support::allocation_test_directory;
        const KEPT: &str = "00000000-0000-4000-8000-000000000001";
        const REFUSED: &str = "00000000-0000-4000-8000-000000000002";
        let root = allocation_test_directory("admission-record");
        let session = root.join("session");
        let target = root.join("target");
        replay_io::begin_admissions(&session, &target).unwrap();
        let on_disk = || replay_io::session_admissions(&session).unwrap().unwrap();
        let calls = Cell::new(0);
        let refuse_reload = Cell::new(false);
        let admit = |campaign: &str, _recorded: bool| -> Result<(), Refusal> {
            calls.set(calls.get() + 1);
            if campaign == REFUSED {
                Err(Refusal::Off)
            } else if refuse_reload.get() {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let mut admission = Admission::new(&session, &target);
        assert_eq!(
            admission.check(KEPT, 1, || false, &admit),
            Ok(Decision::Admitted)
        );
        // On disk the moment it is admitted.
        let first = BTreeMap::from([(KEPT.to_owned(), BTreeSet::from([1]))]);
        assert_eq!(on_disk().loads, first);
        assert_eq!(
            admission.check(REFUSED, 1, || false, &admit),
            Ok(Decision::Refused)
        );
        // Each load is asked once: a later change of choice does not end a
        // session that already admitted it, and a refusal is not repeated.
        assert_eq!(
            admission.check(REFUSED, 1, || unreachable!(), &admit),
            Ok(Decision::Seen)
        );
        assert_eq!(
            admission.check(KEPT, 1, || unreachable!(), &admit),
            Ok(Decision::Seen)
        );
        assert_eq!(calls.get(), 2);
        // Another load of the admitted campaign is asked again; refused, it is
        // never admitted, so never published, and its reason is kept.
        refuse_reload.set(true);
        assert_eq!(
            admission.check(KEPT, 2, || true, &admit),
            Ok(Decision::Refused)
        );
        let record = on_disk();
        assert_eq!(record.loads, first);
        assert_eq!(
            record.refused,
            BTreeMap::from([
                ((REFUSED.to_owned(), 1), Refusal::Off),
                ((KEPT.to_owned(), 2), Refusal::Elsewhere),
            ])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_opening_load_is_admitted_before_the_collector_starts() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        let root = allocation_test_directory("opening-load");
        let session = root.join("session");
        let target = root.join("target");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        replay_io::begin_admissions(&session, &target).unwrap();
        let mut admission = Admission::new(&session, &target);
        admission.admit_opening(campaign, FIRST_LOAD).unwrap();
        // On disk before any hour is spooled, and never asked about again.
        assert_eq!(
            replay_io::session_admissions(&session)
                .unwrap()
                .unwrap()
                .loads,
            BTreeMap::from([(campaign.to_owned(), BTreeSet::from([FIRST_LOAD]))])
        );
        let ask = |_: &str, _: bool| -> Result<(), Refusal> { unreachable!() };
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || unreachable!(), &ask),
            Ok(Decision::Seen)
        );
        // the game closed before the first poll: the load still publishes.
        write_campaign_spool(&session, &allocation_records());
        admission.check_spooled(&ask).unwrap();
        let paths = replay_io::finalize_session(&session, &target).unwrap();
        assert_eq!(published_hours(&paths[0]), [(9, 2), (9, 3), (9, 4)]);
        fs::remove_dir_all(root).unwrap();
    }

    /// Attached at the game's menu with automatic recording on, a load
    /// records and the game closes (or Stop is pressed) before the first probe.
    #[test]
    fn a_brief_load_never_probed_is_admitted_at_save_and_published() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        let root = allocation_test_directory("brief-load");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        write_campaign_spool(&session, &allocation_records());
        let asked = RefCell::new(Vec::new());
        let admit = |campaign: &str, recorded: bool| -> Result<(), Refusal> {
            asked.borrow_mut().push((campaign.to_owned(), recorded));
            Ok(())
        };
        admission.check_spooled(&admit).unwrap();
        assert_eq!(asked.into_inner(), [(campaign.to_owned(), false)]);
        assert_eq!(
            replay_io::session_admissions(&session)
                .unwrap()
                .unwrap()
                .loads,
            BTreeMap::from([(campaign.to_owned(), BTreeSet::from([FIRST_LOAD]))])
        );
        let finalized =
            replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                .unwrap();
        assert!(finalized.errors.is_empty() && finalized.omitted.is_empty());
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// A is admitted, then reloads and records hours, and the game closes before
    /// the next probe. At save the reload is asked by the same rule: with
    /// automatic recording turned off for A it is left out, and after New
    /// file it belongs in a new file. Either way it is reported, with why.
    #[test]
    fn a_reload_that_ends_before_its_probe_is_asked_at_save_by_the_same_rule() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, two_load_records,
            write_campaign_spool,
        };
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        for refusal in [Refusal::Off, Refusal::Elsewhere] {
            let root = allocation_test_directory(&format!("reload-at-save-{}", refusal.as_str()));
            let session = root.join("session");
            let output = root.join("replays");
            replay_io::begin_admissions(&session, &output).unwrap();
            let mut admission = Admission::new(&session, &output);
            admission.admit_opening(campaign, FIRST_LOAD).unwrap();
            write_campaign_spool(&session, &two_load_records());
            let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
                if recorded { Err(refusal) } else { Ok(()) }
            };
            admission.check_spooled(&admit).unwrap();
            let finalized =
                replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                    .unwrap();
            assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
            assert_eq!(
                published_hours(&finalized.replays[0]),
                [(9, 2), (9, 3), (9, 4)]
            );
            assert_eq!(
                finalized.omitted,
                [Omission {
                    campaign: campaign.to_owned(),
                    load: Some(RELOAD),
                    hours: 2,
                    refusal: Some(refusal),
                }]
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// Record pressed at the menu with automatic recording off. Campaign A
    /// loads and records first, then campaign B, and the game closes before any
    /// probe; B's UUID sorts before A's. Save-time decisions follow load
    /// order, so Record belongs to A.
    #[test]
    fn save_time_decisions_follow_load_order_across_campaigns() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        use replay::spool::{ControlRecord, Record};
        const LATER: &str = "00000000-0000-4000-8000-000000000001";
        let first = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        assert!(LATER < first, "the later campaign's UUID sorts first");
        let root = allocation_test_directory("load-order");
        let session = root.join("session");
        let output = root.join("replays");
        write_campaign_spool(&session, &allocation_records());
        // B: its own branch and load, witnessed after all of A's hours.
        let later: Vec<_> = allocation_records()
            .into_iter()
            .map(|mut record| {
                match &mut record {
                    Record::BranchStart(branch) => branch.id = 12,
                    Record::Frame(frame) => frame.branch = 12,
                    Record::Control(ControlRecord::ClockWitness {
                        qpc, load_signal, ..
                    }) => {
                        *qpc += 10_000;
                        *load_signal = Some(31);
                    }
                    _ => {}
                }
                record
            })
            .collect();
        let spool: Vec<u8> = later
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        fs::create_dir_all(session.join(LATER)).unwrap();
        fs::write(session.join(LATER).join("spool.0000.h4s"), spool).unwrap();
        assert_eq!(
            replay_io::first_spooled_campaign(&session).as_deref(),
            Some(first)
        );
        // Record pressed: the first campaign asked about records; others are off.
        let chosen: RefCell<Option<String>> = RefCell::new(None);
        let admit = |campaign: &str, _: bool| -> Result<(), Refusal> {
            let mut chosen = chosen.borrow_mut();
            let pressed = chosen.get_or_insert_with(|| campaign.to_owned()).as_str() == campaign;
            if pressed { Ok(()) } else { Err(Refusal::Off) }
        };
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        admission.check_spooled(&admit).unwrap();
        assert_eq!(chosen.borrow().as_deref(), Some(first));
        let finalized =
            replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                .unwrap();
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        assert_eq!(
            finalized.omitted,
            [Omission {
                campaign: LATER.to_owned(),
                load: Some(31),
                hours: 3,
                refusal: Some(Refusal::Off),
            }]
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// New file is pending and the campaign's first recording is still only
    /// this session's spool. The reload, unpaused before the check, is refused;
    /// then detach times out (or the recorder crashes), so only a later
    /// `finalize` publishes.
    #[test]
    fn a_refused_reload_stays_unpublished_when_the_session_never_completes() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_records, allocation_test_directory,
            two_load_records, write_campaign_spool,
        };
        let root = allocation_test_directory("refused-reload-unfinished");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        // New file starts a new recording for a load that follows recorded hours.
        let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
            if recorded {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let recorded = |load| replay_io::recorded_before_load(&session.join(campaign), load);
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        write_campaign_spool(&session, &allocation_records());
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Admitted)
        );
        write_campaign_spool(&session, &two_load_records());
        assert_eq!(
            admission.check(campaign, RELOAD, || recorded(RELOAD), &admit),
            Ok(Decision::Refused)
        );
        assert!(!session.join("completed.json").exists());
        let paths = replay_io::finalize_session(&session, &output).unwrap();
        assert_eq!(published_hours(&paths[0]), [(9, 2), (9, 3), (9, 4)]);
        fs::remove_dir_all(root).unwrap();
    }

    /// Attached to a paused campaign, the probe admits its load before the
    /// collector names the campaign, and New file is chosen once recording
    /// starts. Polls name only loads, so nothing uses the choice up until the
    /// reload, which starts the new recording; the first load publishes alone.
    #[test]
    fn new_file_chosen_after_the_first_load_waits_for_the_reload() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_records, allocation_test_directory,
            two_load_records, write_campaign_spool,
        };
        let root = allocation_test_directory("new-file-after-first-load");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        // New file as the tray applies it: used up by the next load asked
        // about, which starts a new recording when it follows recorded hours.
        let new_file = Cell::new(false);
        let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
            if new_file.replace(false) && recorded {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let recorded = |load| replay_io::recorded_before_load(&session.join(campaign), load);
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Admitted)
        );
        write_campaign_spool(&session, &allocation_records());
        new_file.set(true);
        // Later polls name the same load: nothing is asked, so New file waits.
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Seen)
        );
        assert!(new_file.get());
        write_campaign_spool(&session, &two_load_records());
        assert_eq!(
            admission.check(campaign, RELOAD, || recorded(RELOAD), &admit),
            Ok(Decision::Refused)
        );
        assert!(!new_file.get());
        let finalized =
            replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                .unwrap();
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_finalization_of_a_failed_session_publishes_exactly_its_admitted_loads() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, allocation_witness,
            two_load_records, write_campaign_spool,
        };
        let root = allocation_test_directory("failed-session-admissions");
        let session = root.join("session");
        let output = root.join("replays");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = write_campaign_spool(&session, &two_load_records());
        let committed = fs::metadata(campaign.join("spool.0000.h4s")).unwrap().len();
        // The collector failed during the reload, which was never admitted.
        let mut witness = allocation_witness(committed);
        witness.branch = Some(10);
        witness.load_signal = Some(RELOAD);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        replay_io::begin_admissions(&session, &output).unwrap();
        replay_io::Admissions::default()
            .admit(&session, &output, name, FIRST_LOAD)
            .unwrap();
        // Each `finalize` rewrites the completion record; the admitted load,
        // and only it, publishes every time, unchanged, and the record keeps
        // the load it left out.
        let recording = output.join(format!("{name}.battleplan"));
        let mut published: Option<Vec<u8>> = None;
        for _ in 0..2 {
            let error = finalize_failed_session(&session, &output, &witness, true).unwrap_err();
            assert!(error.contains("partial"), "{error}");
            assert_eq!(published_hours(&recording), [(9, 2), (9, 3), (9, 4)]);
            let bytes = fs::read(&recording).unwrap();
            assert!(
                published
                    .replace(bytes.clone())
                    .is_none_or(|previous| previous == bytes)
            );
            let completed: Value =
                serde_json::from_slice(&fs::read(session.join("completed.json")).unwrap()).unwrap();
            assert_eq!(
                completed["omitted"],
                json!([{"campaign": name, "load": RELOAD, "hours": 2, "reason": "not_admitted"}])
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_command_line_reads_status_once_a_minute_and_the_tray_every_two_seconds() {
        let start = Instant::now();
        let reads = |live| {
            let mut schedule = StatusReads::new(live, start);
            let (mut printed, mut live_reads) = (0, 0);
            // Two minutes of the capture loop's 100 ms polls.
            for step in 0..1_200 {
                let now = start + Duration::from_millis(100 * step);
                printed += u32::from(schedule.print_due(now));
                if schedule.live_due(now) {
                    live_reads += 1;
                    schedule.live_read(now);
                }
            }
            (printed, live_reads)
        };
        assert_eq!(reads(false), (2, 0));
        assert_eq!(reads(true), (2, 60));
        let options = parse_options("record", vec!["--out".into(), "captures".into()]).unwrap();
        let user = PathBuf::from("user");
        let capture = cli_capture(&options, &user);
        assert!(!capture.live_status);
        assert!(capture.admit.is_none() && capture.loaded.is_none());
        assert_eq!(capture.target, capture.output);
    }

    #[test]
    fn discarded_session_keeps_only_its_completion_record_and_refuses_finalization() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-discard-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let session = root.join("sessions").join("7-1");
        let campaign = session.join("00000000-0000-4000-8000-000000000000");
        fs::create_dir_all(session.join("bin")).unwrap();
        fs::create_dir_all(&campaign).unwrap();
        fs::write(session.join("bin/collector.dll"), b"synthetic").unwrap();
        fs::write(campaign.join("spool.0001.h4s"), b"synthetic").unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), b"synthetic").unwrap();
        write_json(&session.join("status.json"), &json!({"session":7})).unwrap();
        // An earlier recording of the campaign sits beside the sessions folder.
        let earlier = root.join("00000000-0000-4000-8000-000000000000.battleplan");
        fs::write(&earlier, b"earlier recording").unwrap();
        let active = root.join("active.json");
        write_json(&active, &json!({"session_dir": session})).unwrap();
        assert!(!session_discarded(&session));
        write_json(
            &session.join("completed.json"),
            &json!({"replays":[],"discarded":true,"partial":false,"issues":[]}),
        )
        .unwrap();
        remove_active(&active, &session).unwrap();
        discard_session(&session).unwrap();
        let left: Vec<_> = fs::read_dir(&session)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["completed.json"]);
        assert!(!active.exists());
        assert_eq!(fs::read(&earlier).unwrap(), b"earlier recording");
        assert!(session_discarded(&session));
        let error = command(vec![
            "finalize".into(),
            session.clone().into_os_string(),
            "--out".into(),
            root.clone().into_os_string(),
        ])
        .unwrap_err();
        assert!(error.contains("discarded"), "{error}");
        fs::remove_dir_all(root).unwrap();
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
