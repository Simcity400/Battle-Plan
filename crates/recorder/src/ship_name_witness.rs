use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
};

use engine_dlc::cdlc_manager;
use engine_executable::build::hash_executable;
use engine_native::{Memory, capture_text::Owned, cfleet, cgame_state, cship, read, walk::Outcome};
use content_reader_layers::content::{GatherOptions, gather};
use recorder_platform::games::{Discovery, choose_game};
use replay::campaign::{Replay, SeekResult};
use replay_data::{
    identity::{COVERAGE_POLICY_VERSION, Dlc, Identity, Mod, Verification},
    rows::{Recorded, TableId, TableRows},
};
use replay_frame::frame::TableView;
use serde::{Deserialize, Serialize};

use crate::{
    directories,
    windows::{self, Process, ProcessMemory},
};

pub const USAGE: &str = "recorder ship-name-witness --paused --hour <H> --branch <B> [--user-dir <directory>] [--report <file.json>]\nrecorder ship-name-witness --compare <report.json> <campaign.battleplan> --hour <H> --branch <B>";
const METHOD: &str = "guarded_retained_ship_display_cache";

struct Options {
    hour: i32,
    branch: u64,
    compare: Option<(PathBuf, PathBuf)>,
    user: Option<PathBuf>,
    report: Option<PathBuf>,
    paused: bool,
}

fn options(args: Vec<OsString>) -> Result<Options, String> {
    let (mut hour, mut branch, mut compare, mut user, mut report) = (None, None, None, None, None);
    let mut paused = false;
    let mut args = args.into_iter();
    while let Some(option) = args.next() {
        if option == "--paused" && !paused {
            paused = true;
            continue;
        }
        let value = args.next().ok_or(USAGE)?;
        match option.to_str() {
            Some("--hour") if hour.is_none() => {
                hour = Some(
                    value
                        .to_str()
                        .and_then(|v| v.parse().ok())
                        .ok_or("--hour requires an engine hour")?,
                );
            }
            Some("--branch") if branch.is_none() => {
                branch = Some(
                    value
                        .to_str()
                        .and_then(|v| v.parse().ok())
                        .filter(|v| *v != 0)
                        .ok_or("--branch requires a positive branch id")?,
                );
            }
            Some("--compare") if compare.is_none() => {
                compare = Some((
                    PathBuf::from(value),
                    PathBuf::from(args.next().ok_or(USAGE)?),
                ));
            }
            Some("--user-dir") if user.is_none() => user = Some(PathBuf::from(value)),
            Some("--report") if report.is_none() => report = Some(PathBuf::from(value)),
            _ => return Err(USAGE.into()),
        }
    }
    if compare.is_some() {
        if paused || user.is_some() || report.is_some() {
            return Err(USAGE.into());
        }
    } else if !paused {
        return Err("pause the game yourself and confirm with --paused; the witness never controls the game".into());
    }
    Ok(Options {
        hour: hour.ok_or(USAGE)?,
        branch: branch.ok_or(USAGE)?,
        compare,
        user,
        report,
        paused,
    })
}

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let options = options(args)?;
    if let Some((report, recording)) = &options.compare {
        let report: Report = serde_json::from_slice(&fs::read(report).map_err(|e| e.to_string())?)
            .map_err(|e| format!("ship-name witness report: {e}"))?;
        let replay = Replay::open_path(recording).map_err(|e| e.to_string())?;
        let compared = compare(&report, &replay, options.hour, options.branch)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&compared).map_err(|e| e.to_string())?
        );
        return compared.outcome();
    }
    capture(options)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Clock {
    game_state: u64,
    campaign: Vec<u8>,
    hour: i32,
    load_signal: i64,
    seed: i32,
    speed: i32,
    tick_busy: u8,
    tick_progress: u32,
}

fn clock(memory: &impl Memory, base: usize) -> Result<Clock, String> {
    let clock = cgame_state::read_clock(memory, base).ok_or("game clock unreadable")?;
    let state = usize::try_from(clock.game_state).map_err(|_| "game state pointer invalid")?;
    let tick_progress = state
        .checked_add(cgame_state::TICK_PROGRESS)
        .and_then(|address| read(memory, address))
        .ok_or("tick progress unreadable")?;
    Ok(Clock {
        game_state: clock.game_state,
        campaign: clock.campaign_id.as_bytes().to_vec(),
        hour: clock.hour,
        load_signal: clock.play_session_started_at,
        seed: clock.seed,
        speed: clock.speed,
        tick_busy: clock.tick_busy,
        tick_progress,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Cache {
    Read { bytes: Vec<u8> },
    Refused { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ship {
    reference: u64,
    task_force: u64,
    cache: Cache,
}

fn ship(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    reference: u64,
    task_force: u64,
) -> Ship {
    let mut bytes = Vec::new();
    let (mut len, mut required) = (0, 0);
    let copied = cship::cached_name_growing(
        memory,
        base,
        pointer,
        reference,
        &mut bytes,
        &mut len,
        &mut required,
    );
    let cache = match copied {
        Ok(Some((0, count))) if count <= bytes.len() => {
            bytes.truncate(count);
            Cache::Read { bytes }
        }
        Ok(Some(_)) => Cache::Refused {
            reason: "ship cache span invalid".into(),
        },
        Ok(None) => Cache::Refused {
            reason: "ship cache storage unavailable".into(),
        },
        Err(reason) => Cache::Refused {
            reason: reason.into(),
        },
    };
    Ship {
        reference,
        task_force,
        cache,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Census {
    Complete { rows: usize },
    Truncated { rows: usize, unread: u32 },
    Refused { reason: String },
}

fn ships(memory: &impl Memory, base: usize, state: usize) -> (Census, Vec<Ship>) {
    let (mut fleets, mut tasks, mut rows, mut names, mut ships) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let walked = cfleet::walk_timed_with_names(
        memory,
        base,
        state,
        &mut fleets,
        &mut tasks,
        &mut rows,
        |_| 0,
        |pointer, row| ships.push(ship(memory, base, pointer, row.id, row.task_force)),
        &mut names,
    );
    let census = match walked.capture.ships {
        Outcome::Rows { len } => Census::Complete { rows: len },
        Outcome::Truncated { len, unread_rows } => Census::Truncated {
            rows: len,
            unread: unread_rows,
        },
        Outcome::Unavailable { reason } => Census::Refused {
            reason: reason.into(),
        },
    };
    (census, ships)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentIdentity {
    game_version: String,
    exe_hash: [u8; 32],
    exe_size: u64,
    vanilla_hash: [u8; 32],
    dlc: Vec<DlcIdentity>,
    mods: Vec<ModIdentity>,
    coverage_policy: u32,
    language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DlcIdentity {
    id: String,
    hash: [u8; 32],
    declared_checksum: Option<String>,
}

impl From<&Dlc> for DlcIdentity {
    fn from(dlc: &Dlc) -> Self {
        Self {
            id: dlc.id.clone(),
            hash: dlc.hash,
            declared_checksum: dlc.declared_checksum.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModIdentity {
    id: String,
    display_name: String,
    declared_version: String,
    hash: [u8; 32],
}

impl From<&Mod> for ModIdentity {
    fn from(layer: &Mod) -> Self {
        Self {
            id: layer.id.clone(),
            display_name: layer.display_name.clone(),
            declared_version: layer.declared_version.clone(),
            hash: layer.hash,
        }
    }
}

impl From<&Identity> for ContentIdentity {
    fn from(identity: &Identity) -> Self {
        Self {
            game_version: identity.game_version.clone(),
            exe_hash: identity.exe_hash,
            exe_size: identity.exe_size,
            vanilla_hash: identity.vanilla_hash,
            dlc: identity.dlc.iter().map(Into::into).collect(),
            mods: identity.mods.iter().map(Into::into).collect(),
            coverage_policy: identity.coverage_policy,
            language: identity.language.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    method: String,
    branch: u64,
    process_id: u32,
    process_start_time: u64,
    executable: PathBuf,
    identity: ContentIdentity,
    recorder_version: String,
    recorder_executable: PathBuf,
    recorder_hash: [u8; 32],
    recorder_size: u64,
    expected_mounts: Vec<String>,
    observed_mounts: Vec<Vec<u8>>,
    paused_by_operator: bool,
    before: Clock,
    after: Option<Clock>,
    census: Census,
    ships: Vec<Ship>,
    refusals: Vec<String>,
}

fn mounts(
    memory: &impl Memory,
    base: usize,
    expected: &[String],
    note: Option<&str>,
) -> Result<Vec<Vec<u8>>, String> {
    if let Some(note) = note {
        return Err(format!("content load order unverified: {note}"));
    }
    let mut observed = vec![Owned::default(); cdlc_manager::observe_counts(memory, base).mounts];
    match cdlc_manager::observe_search_path_owned(memory, base, &mut observed, true) {
        Outcome::Rows { len } if cdlc_manager::mounts_match(&observed[..len], expected) => {
            Ok(observed[..len]
                .iter()
                .map(|mount| mount.as_bytes().to_vec())
                .collect())
        }
        outcome => Err(format!(
            "content load order unverified: successful mounts {outcome:?} do not prove expected membership and priority"
        )),
    }
}

fn capture(options: Options) -> Result<(), String> {
    let process = loop {
        let game = match choose_game(&windows::game_processes()?) {
            Discovery::One(game) => game,
            Discovery::None => {
                return Err("no hoi4.exe is running; load and pause a game first".into());
            }
            Discovery::Several(several) => return Err(several.to_string()),
        };
        if let Some(process) = Process::open_selected(&game, false)? {
            break process;
        }
    };
    let pid = process.pid;
    let tables = engine_executable::executable_tables::initialize_executable(&process.path)
        .map_err(|e| e.to_string())?;
    let exe = tables.identity().clone();
    let version = tables.version.clone().map_err(|e| e.to_string())?;
    let base = crate::game_sites::module_base(&process, pid)?;
    crate::game_sites::locate(&process.path, base)?
        .layouts()
        .map_err(|withheld| format!("the ship-name witness is withheld: {withheld}"))?;
    let memory = ProcessMemory(&process.handle);
    let before = clock(&memory, base)?;
    if before.tick_busy != 0
        || before.hour != options.hour
        || !replay_data::identity::is_campaign_uuid(&before.campaign)
    {
        return Err("load and pause the intended campaign at the requested engine hour before taking the witness".into());
    }
    let user = match options.user {
        Some(user) => user,
        None => directories::resolve_user_directory()?,
    };
    let content = gather(&GatherOptions {
        game_dir: process
            .path
            .parent()
            .ok_or("game executable has no folder")?
            .to_owned(),
        user_dir: user,
        cache_dir: replay_settings::recorder_settings::directory(),
        dlc_order: None,
    })
    .map_err(|e| e.to_string())?;
    let observed_mounts = mounts(
        &memory,
        base,
        &content.expected_mounts,
        content.order_note.as_deref(),
    )?;
    let recorder_executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let recorder_build =
        hash_executable(File::open(&recorder_executable).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let identity = ContentIdentity {
        game_version: version,
        exe_hash: exe.sha256,
        exe_size: exe.size,
        vanilla_hash: content.vanilla_hash,
        dlc: content.dlc.iter().map(Into::into).collect(),
        mods: content.mods.iter().map(Into::into).collect(),
        coverage_policy: COVERAGE_POLICY_VERSION,
        language: content.language.clone(),
    };
    if clock(&memory, base)? != before {
        return Err(
            "campaign/load/hour/tick state changed while proving content; pause and repeat".into(),
        );
    }
    let (census, ships) = ships(
        &memory,
        base,
        usize::try_from(before.game_state).map_err(|_| "game state pointer invalid")?,
    );
    let mut report = Report {
        method: METHOD.into(),
        branch: options.branch,
        process_id: pid,
        process_start_time: process.start_time,
        executable: process.path.clone(),
        identity,
        recorder_version: crate::recording::RECORDER_VERSION.into(),
        recorder_executable,
        recorder_hash: recorder_build.sha256,
        recorder_size: recorder_build.size,
        expected_mounts: content.expected_mounts,
        observed_mounts,
        paused_by_operator: options.paused,
        before,
        after: None,
        census,
        ships,
        refusals: Vec::new(),
    };
    match clock(&memory, base) {
        Ok(after) => report.after = Some(after),
        Err(reason) => report.refusals.push(reason),
    }
    if let Err(reason) = mounts(
        &memory,
        base,
        &report.expected_mounts,
        content.order_note.as_deref(),
    ) {
        report.refusals.push(reason);
    }
    match process.handle.signaled() {
        Ok(false) => {}
        Ok(true) => report
            .refusals
            .push("game exited during the witness".into()),
        Err(reason) => report.refusals.push(reason),
    }
    let refusals = report_refusals(&report);
    let refused = report
        .ships
        .iter()
        .filter(|ship| matches!(ship.cache, Cache::Refused { .. }))
        .count();
    let path = options.report.unwrap_or_else(|| {
        directories::default_capture_directory()
            .join("ship_name_witness")
            .join(format!(
                "ship-names-{}-{}-{}.json",
                process.start_time, report.branch, report.before.hour
            ))
    });
    write_report(&path, &report)?;
    println!(
        "Ship-name witness: {} ships, {refused} refused; {} report refusals. Report: {}",
        report.ships.len(),
        refusals.len(),
        path.display()
    );
    if refused != 0 || !refusals.is_empty() {
        return Err(format!(
            "ship-name witness incomplete: {refused} refused ships; {}",
            refusals.join("; ")
        ));
    }
    Ok(())
}

fn write_report(path: &Path, report: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::to_writer_pretty(&file, report).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn mount_evidence_matches(observed: &[Vec<u8>], expected: &[String]) -> bool {
    let observed: Vec<_> = observed
        .iter()
        .map(|bytes| Owned {
            bytes: bytes.clone(),
            len: bytes.len(),
        })
        .collect();
    cdlc_manager::mounts_match(&observed, expected)
}
fn report_refusals(report: &Report) -> Vec<String> {
    let mut refusals = report.refusals.clone();
    if report.method != METHOD {
        refusals.push("unrecognized witness method".into());
    }
    if !report.paused_by_operator {
        refusals.push("operator pause confirmation missing".into());
    }
    if report.before.tick_busy != 0 || report.after.as_ref() != Some(&report.before) {
        refusals
            .push("idle campaign/load/hour/tick state was not stable across the witness".into());
    }
    if !replay_data::identity::is_campaign_uuid(&report.before.campaign) {
        refusals.push("loaded campaign identity invalid".into());
    }
    if !mount_evidence_matches(&report.observed_mounts, &report.expected_mounts) {
        refusals.push("content mount membership or priority unproved".into());
    }
    let mut references = BTreeSet::new();
    if report
        .ships
        .iter()
        .any(|ship| !references.insert(ship.reference))
    {
        refusals.push("duplicate full ship references in native census".into());
    }
    match &report.census {
        Census::Complete { rows } if *rows == report.ships.len() && *rows != 0 => {}
        _ => refusals.push("complete nonempty ship census missing".into()),
    }
    refusals
}

#[derive(Debug, Default, Serialize)]
struct Comparison {
    witness_ships: usize,
    replay_ships: Option<usize>,
    matched: usize,
    missing_from_witness: usize,
    missing_from_replay: usize,
    witness_refused: usize,
    replay_refused: usize,
    mismatched: usize,
    task_force_mismatched: usize,
    duplicate_references: usize,
    invalid_references: usize,
    refusals: Vec<String>,
    ships: BTreeMap<u64, Vec<String>>,
}

impl Comparison {
    fn outcome(&self) -> Result<(), String> {
        if self.matched != 0 && self.refusals.is_empty() && self.ships.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "ship-name witness comparison failed: {} matched, {} missing, {} refused, {} mismatched; {} report refusals",
                self.matched,
                self.missing_from_witness + self.missing_from_replay,
                self.witness_refused + self.replay_refused,
                self.mismatched + self.task_force_mismatched,
                self.refusals.len()
            ))
        }
    }
    fn ship(&mut self, reference: u64, reason: &str) {
        self.ships.entry(reference).or_default().push(reason.into());
    }
}

fn compare(report: &Report, replay: &Replay, hour: i32, branch: u64) -> Result<Comparison, String> {
    let mut result = Comparison {
        witness_ships: report.ships.len(),
        refusals: report_refusals(report),
        ..Default::default()
    };
    let mut witnessed = BTreeSet::new();
    for ship in &report.ships {
        if !witnessed.insert(ship.reference) {
            result.duplicate_references += 1;
            result.ship(ship.reference, "duplicate witness reference");
        }
        if ship.reference as u32 != cship::HANDLE_TYPE {
            result.invalid_references += 1;
            result.ship(ship.reference, "invalid full ship reference");
        }
        if let Cache::Refused { reason } = &ship.cache {
            result.witness_refused += 1;
            result.ship(ship.reference, &format!("native cache refused: {reason}"));
        }
    }
    if report.before.hour != hour
        || report.branch != branch
        || report.before.campaign != replay.campaign_id
    {
        result
            .refusals
            .push("witness campaign, branch or engine hour differs from request/recording".into());
    }
    if report.identity != ContentIdentity::from(&replay.identity) {
        result.refusals.push(
            "witness build, language or ordered content identity differs from recording".into(),
        );
    }
    if !matches!(replay.identity.verification, Verification::Verified { .. }) {
        result
            .refusals
            .push("recording content identity is unverified".into());
    }
    let session = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .and_then(|start| {
            replay
                .sessions
                .iter()
                .find(|session| session.id == start.session)
        });
    if !session.is_some_and(|session| {
        session.process_id == report.process_id
            && session.process_start_time == report.process_start_time
    }) {
        result
            .refusals
            .push("recording process session differs from witness".into());
    }
    let damage_notes = replay.recovery_notes.iter().filter(|note| !recorder_replay::replay_io::is_restart(note)).count();
    if replay.damaged_parts != 0 || damage_notes != 0 || !replay.warnings.is_empty()
    {
        result
            .refusals
            .push("recording has recovery or damage diagnostics".into());
    }
    let frame = match replay
        .seek_selected(branch, hour, Some(&[TableId::Clock, TableId::Ships]))
        .map_err(|e| e.to_string())?
    {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => {
            result
                .refusals
                .push(format!("requested supplying hour unavailable: {reason}"));
            return Ok(result);
        }
    };
    match &*frame.tables[TableId::Clock.index()] {
        TableView::Complete(TableRows::Clock(rows))
            if rows.len() == 1
                && rows[0].hour == hour
                && rows[0].play_session_started_at == report.before.load_signal => {}
        _ => result
            .refusals
            .push("recorded load signal/hour missing or different".into()),
    }
    let table = &frame.tables[TableId::Ships.index()];
    if table.sampled_at() != Some(hour) {
        result
            .refusals
            .push("Ships has no fresh supplying read at the requested hour".into());
    }
    let rows = match &**table {
        TableView::Complete(TableRows::Ships(rows)) => rows,
        _ => {
            result
                .refusals
                .push("Ships census unavailable or truncated".into());
            return Ok(result);
        }
    };
    result.replay_ships = Some(rows.len());
    for row in rows {
        if matches!(row.name, Recorded::Value(None) | Recorded::NotRecorded) {
            result.replay_refused += 1;
            result.ship(row.id, "recorded ship name unavailable");
        }
    }
    let recorded: BTreeMap<_, _> = rows.iter().map(|row| (row.id, row)).collect();
    for ship in &report.ships {
        let Some(row) = recorded.get(&ship.reference) else {
            result.missing_from_replay += 1;
            result.ship(ship.reference, "ship missing from replay");
            continue;
        };
        if ship.task_force != row.task_force {
            result.task_force_mismatched += 1;
            result.ship(ship.reference, "containing task force differs");
        }
        match (&ship.cache, &row.name) {
            (Cache::Read { bytes }, Recorded::Value(Some(name))) if bytes == name => {
                result.matched += 1
            }
            (Cache::Read { .. }, Recorded::Value(Some(_))) => {
                result.mismatched += 1;
                result.ship(ship.reference, "retained cache bytes differ");
            }
            (_, Recorded::Value(None) | Recorded::NotRecorded) => {}
            (Cache::Refused { .. }, _) => {}
        }
    }
    for row in rows {
        if !witnessed.contains(&row.id) {
            result.missing_from_witness += 1;
            result.ship(row.id, "ship missing from witness");
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
