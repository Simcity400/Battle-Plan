
use replay_data::identity::IdentityCodec;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use replay::campaign::{Replay, Revision, SeekResult};
use replay_settings::packing::Budgeted;
use replay_recording::control::ControlRecord;
use replay_capture_spool::spool::{Recovered, Recovery};
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    BranchId, Hour, SessionId, format::FormatVersion, identity::Identity, identity::SessionStart,
    identity::Verification, rows::TableId,
    rows::tables,
};
use replay_data::{
    witness::TerminalWitness, witness::UncommittedRange, witness::WITNESS_BYTES,
    witness::WITNESS_FILE_NAME,
};
use replay_frame::{
    branches::BranchStart, branches::BranchTracker, branches::Gap, branches::Parentage,
    frame::DecodedFrame, frame::TableUpdate, frame::TableView,
};

use crate::windows;

mod ending;
mod session;

pub use ending::{
    finalize_session, finalize_session_ahead, finalize_session_outcome, finalize_session_outcome_with_progress,
    write_session_end,
};
pub use session::{
    Discarded, LOADS_FILE, Live, Load, RESTARTS_FILE, Unsaved, WRITING_FILE, discard_waiting, first_loaded_campaign, is_restart,
    is_unsaved_recording, journal_campaigns, locate, note_discard, noted_discards, recorded_before_load,
    reconcile_session, save_waiting, session_loads, unsaved, unsaved_bytes, waiting_name, written_campaigns,
};
use content_reader_localisation::localisation::Localisation;
use replay_settings::recorder_settings::RecorderSettings;
use replay_files::recording_files;
use serde_json::{Value, json};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Off,
    Elsewhere,
    Settings,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Elsewhere => "elsewhere",
            Self::Settings => "settings",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Self::Off),
            "elsewhere" => Some(Self::Elsewhere),
            "settings" => Some(Self::Settings),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Omission {
    pub campaign: String,
    pub load: Option<i64>,
    pub hours: u64,
    pub refusal: Option<Refusal>,
}

impl Omission {
    pub fn to_json(&self) -> Value {
        json!({
            "campaign": self.campaign,
            "load": self.load,
            "hours": self.hours,
            "reason": self.refusal.map_or("not_admitted", Refusal::as_str),
        })
    }
}

fn read_file(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    Budgeted::new(fs::File::open(path)?).read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_json_before_publish(path, value, |_| Ok(()))
}

fn write_json_before_publish(
    path: &Path,
    value: &Value,
    before_publish: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let (temporary, file) = json_temporary(path)?;
    let result = (|| {
        Budgeted::new(file)
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        fs::OpenOptions::new()
            .write(true)
            .open(&temporary)
            .and_then(|file| file.sync_all())
            .map_err(|error| failure(&temporary, error))?;
        before_publish(&temporary)?;
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
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn json_temporary(path: &Path) -> Result<(PathBuf, fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("json.{}.{sequence}.tmp", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(failure(&temporary, error)),
        }
    }
}

type Result<T> = std::result::Result<T, String>;

pub fn with_packing<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    if replay_settings::packing::Packing::active() {
        return work();
    }
    let packing = replay_settings::packing::Packing::new()
        .map_err(|error| format!("Packing could not start: {error}"))?;
    packing.promote();
    packing.run(work)
}

const ALLOCATION_GAP_REASON: &str = "collector allocation failed";

pub fn read_failure_witness(session_dir: &Path) -> Result<Option<TerminalWitness>> {
    Ok(read_failure_witnesses(session_dir)?.first().copied())
}

fn read_failure_witnesses(session_dir: &Path) -> Result<Vec<TerminalWitness>> {
    let path = session_dir.join(WITNESS_FILE_NAME);
    let config_path = session_dir.join("bin/collector.json");
    let modern = config_path.is_file();
    let bytes = match read_file(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if modern {
                return Err(failure(&path, "precreated terminal witness is missing"));
            }
            return Ok(Vec::new());
        }
        Err(error) => return Err(failure(&path, error)),
    };
    if bytes.len() < WITNESS_BYTES || !bytes.len().is_multiple_of(WITNESS_BYTES) {
        return Err(failure(&path, "terminal witness has the wrong length"));
    }
    if bytes.iter().all(|byte| *byte == 0) {
        if modern {
            return Err(failure(&path, "modern terminal witness is uninitialised"));
        }
        return Ok(Vec::new());
    }
    if let Ok(identity) = TerminalWitness::decode_prepared(&bytes) {
        if modern {
            let config: serde_json::Value = serde_json::from_slice(
                &read_file(&config_path).map_err(|error| failure(&config_path, error))?,
            )
            .map_err(|error| failure(&config_path, error))?;
            let session_id = config["session_id"].as_u64().ok_or_else(|| {
                failure(&config_path, "collector configuration has no session id")
            })?;
            let session_start: Vec<u8> = serde_json::from_value(config["session_start"].clone())
                .map_err(|error| failure(&config_path, error))?;
            let mut decoder = replay_data::codec::Decoder::new(&session_start);
            let session =
                SessionStart::decode(&mut decoder).map_err(|error| failure(&config_path, error))?;
            decoder
                .finish()
                .map_err(|error| failure(&config_path, error))?;
            if identity != (session_id, session.process_start_time) || session_id != session.id {
                return Err(failure(
                    &path,
                    "prepared witness belongs to a different process session",
                ));
            }
        }
        return Ok(Vec::new());
    }
    replay_data::witness::decode_bundle(&bytes).map_err(|error| failure(&path, error))
}

struct CampaignFailure {
    witness: TerminalWitness,
    ranges: Vec<UncommittedRange>,
    bundled: bool,
}

fn campaign_failure(
    session_dir: &Path,
    campaign: &[u8],
    primary: &TerminalWitness,
) -> Result<CampaignFailure> {
    let mut records = if session_dir.join(WITNESS_FILE_NAME).is_file() {
        read_failure_witnesses(session_dir)?
    } else {
        Vec::new()
    };
    if records.is_empty() {
        records.push(*primary);
    } else if records[0] != *primary {
        return Err(failure(
            session_dir,
            "terminal witness changed during finalization",
        ));
    }
    let mut ranges: Vec<UncommittedRange> = Vec::new();
    for range in records
        .iter()
        .filter_map(|record| record.uncommitted)
        .filter(|range| range.campaign_id.as_slice() == campaign)
    {
        if let Some(known) = ranges.iter_mut().find(|known| known.branch == range.branch) {
            if known.load_signal != range.load_signal {
                return Err(failure(
                    session_dir,
                    "conflicting terminal range load signals",
                ));
            }
            known.first_hour = known.first_hour.min(range.first_hour);
            known.last_hour = known.last_hour.max(range.last_hour);
        } else {
            ranges.push(range);
        }
    }
    ranges.sort_by_key(|range| range.branch);
    let mut projected = *primary;
    let mut prefix = records
        .iter()
        .filter(|record| {
            record
                .committed_campaign_id
                .as_ref()
                .map(|id| id.as_slice())
                == Some(campaign)
        })
        .filter_map(|record| {
            record
                .committed_segment
                .map(|segment| (segment, record.committed_bytes))
        })
        .max();
    if primary.pending_campaign_id.as_ref().map(|id| id.as_slice()) == Some(campaign) {
        let pending = primary
            .pending_segment
            .map(|segment| (segment, primary.pending_bytes));
        prefix = match (prefix, pending) {
            (Some(prefix), Some(pending)) => Some(prefix.min(pending)),
            _ => None,
        };
    }
    let empty_prefix = prefix.is_none() && !ranges.is_empty();
    if let Some((segment, bytes)) = prefix {
        projected.committed_campaign_id = Some(
            campaign
                .try_into()
                .map_err(|_| failure(session_dir, "invalid campaign identity"))?,
        );
        projected.committed_segment = Some(segment);
        projected.committed_bytes = bytes;
    } else if empty_prefix {
        projected.committed_campaign_id = None;
        projected.committed_segment = None;
        projected.committed_bytes = 0;
    }
    Ok(CampaignFailure {
        witness: projected,
        ranges,
        bundled: records.len() > 1,
    })
}

fn failure_session(session_dir: &Path, witness: &TerminalWitness) -> Result<SessionStart> {
    let config_path = session_dir.join("bin/collector.json");
    if config_path.is_file() {
        let config: serde_json::Value = serde_json::from_slice(
            &read_file(&config_path).map_err(|error| failure(&config_path, error))?,
        )
        .map_err(|error| failure(&config_path, error))?;
        let bytes: Vec<u8> = serde_json::from_value(config["session_start"].clone())
            .map_err(|error| failure(&config_path, error))?;
        let mut decoder = replay_data::codec::Decoder::new(&bytes);
        let session =
            SessionStart::decode(&mut decoder).map_err(|error| failure(&config_path, error))?;
        decoder
            .finish()
            .map_err(|error| failure(&config_path, error))?;
        if config["session_id"].as_u64() != Some(session.id)
            || session.id != witness.session
            || session.process_start_time != witness.process_start_time
        {
            return Err(failure(
                &config_path,
                "terminal witness does not match the prepared process session",
            ));
        }
        return Ok(session);
    }
    Err(failure(
        session_dir,
        "terminal ranges have no surviving process session identity",
    ))
}

fn failure(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn segment_number(name: &str) -> Result<Option<u32>> {
    if !name.starts_with("spool.") || !name.ends_with(".h4s") {
        return Ok(None);
    }
    let number = &name[6..name.len() - 4];
    if number.len() < 4 || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid spool segment name: {name}"));
    }
    number
        .parse()
        .map(Some)
        .map_err(|_| format!("spool segment number exceeds u32: {name}"))
}

pub fn segment_paths(directory: &Path) -> Result<Vec<(u32, PathBuf)>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| failure(directory, error))? {
        let entry = entry.map_err(|error| failure(directory, error))?;
        if !entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_file()
        {
            continue;
        }
        if let Some(number) = segment_number(&entry.file_name().to_string_lossy())? {
            paths.push((number, entry.path()));
        }
    }
    paths.sort_unstable_by_key(|(number, _)| *number);
    if paths.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(failure(directory, "duplicate spool segment number"));
    }
    Ok(paths)
}

static ROSTERS: std::sync::Mutex<BTreeMap<Vec<u8>, (Revision, usize, bool)>> = std::sync::Mutex::new(BTreeMap::new());

fn roster(replay: &Replay, path: &Path) -> Result<bool> {
    let key = replay.campaign_id.to_ascii_lowercase();
    let known = ROSTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .copied()
        .filter(|(revision, blocks, _)| {
            *blocks <= replay.index.len() && replay.contains_revision(*revision).unwrap_or(false)
        });
    let (read, mut multiplayer) = known.map_or((0, false), |(_, blocks, multiplayer)| (blocks, multiplayer));
    if !multiplayer {
        multiplayer = crate::recording_name::multiplayer_after(replay, read).map_err(|error| failure(path, error))?;
    }
    ROSTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (replay.revision(), replay.index.len(), multiplayer));
    Ok(multiplayer)
}

fn spool_segments(campaign_dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(segment_paths(campaign_dir)?.into_iter().map(|(_, path)| path).collect())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Admissions {
    pub loads: BTreeMap<String, BTreeSet<i64>>,
    pub refused: BTreeMap<(String, i64), Refusal>,
}

pub const ADMISSIONS_FILE: &str = "admitted.json";
pub const ADMISSION_MARKER: &str = "recorder.json";

impl Admissions {
    fn to_json(&self, target: &Path) -> serde_json::Value {
        let loads: Vec<_> = self
            .loads
            .iter()
            .flat_map(|(campaign, loads)| {
                loads.iter().map(move |load| {
                    serde_json::json!({"campaign": campaign, "load": load, "target": target})
                })
            })
            .collect();
        let refused: Vec<_> = self
            .refused
            .iter()
            .map(|((campaign, load), refusal)| {
                serde_json::json!({"campaign": campaign, "load": load, "reason": refusal.as_str()})
            })
            .collect();
        serde_json::json!({ "loads": loads, "refused": refused })
    }

    pub fn admit(
        &mut self,
        session_dir: &Path,
        target: &Path,
        campaign: &str,
        load: i64,
    ) -> Result<()> {
        let mut next = self.clone();
        next.loads
            .entry(campaign.to_owned())
            .or_default()
            .insert(load);
        write_json(&session_dir.join(ADMISSIONS_FILE), &next.to_json(target))?;
        *self = next;
        Ok(())
    }

    pub fn refuse(
        &mut self,
        session_dir: &Path,
        target: &Path,
        campaign: &str,
        load: i64,
        refusal: Refusal,
    ) -> Result<()> {
        self.refused.insert((campaign.to_owned(), load), refusal);
        write_json(&session_dir.join(ADMISSIONS_FILE), &self.to_json(target))
    }

    fn omission(&self, campaign: &str, load: Option<i64>, hours: u64) -> Omission {
        Omission {
            campaign: campaign.to_owned(),
            load,
            hours,
            refusal: load.and_then(|load| self.refused.get(&(campaign.to_owned(), load)).copied()),
        }
    }
}

pub fn begin_admissions(session_dir: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(session_dir.join("bin")).map_err(|error| failure(session_dir, error))?;
    write_json(
        &session_dir.join(ADMISSIONS_FILE),
        &Admissions::default().to_json(target),
    )?;
    write_json(
        &session_dir.join("bin").join(ADMISSION_MARKER),
        &serde_json::json!({ "admission": ADMISSIONS_FILE }),
    )
}

pub fn session_admissions(session_dir: &Path) -> Result<Option<Admissions>> {
    if !session_dir.join("bin").join(ADMISSION_MARKER).is_file() {
        return Ok(None);
    }
    let path = session_dir.join(ADMISSIONS_FILE);
    let unreadable = || {
        failure(
            &path,
            "the admission record is missing or unreadable; nothing published; spool retained",
        )
    };
    let record: serde_json::Value = read_file(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(unreadable)?;
    let mut admissions = Admissions::default();
    for entry in record["loads"].as_array().ok_or_else(unreadable)? {
        let campaign = entry["campaign"].as_str().ok_or_else(unreadable)?;
        let load = entry["load"].as_i64().ok_or_else(unreadable)?;
        admissions
            .loads
            .entry(campaign.to_owned())
            .or_default()
            .insert(load);
    }
    if let Some(refused) = record.get("refused") {
        for entry in refused.as_array().ok_or_else(unreadable)? {
            let campaign = entry["campaign"].as_str().ok_or_else(unreadable)?;
            let load = entry["load"].as_i64().ok_or_else(unreadable)?;
            let refusal = entry["reason"]
                .as_str()
                .and_then(Refusal::parse)
                .ok_or_else(unreadable)?;
            admissions
                .refused
                .insert((campaign.to_owned(), load), refusal);
        }
    }
    Ok(Some(admissions))
}

#[derive(Debug, Default)]
pub struct Finalized {
    pub replays: Vec<PathBuf>,
    pub saved_campaigns: BTreeSet<String>,
    pub failed_campaigns: BTreeSet<String>,
    pub unsaved: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub omitted: Vec<Omission>,
    pub unrenamed: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct Naming<'a> {
    pub localisation: Option<&'a Localisation>,
    pub settings: &'a Path,
    pub report: Option<&'a (dyn Fn(Option<&str>) + Sync)>,
}

fn session_witness(session_dir: &Path) -> Result<Option<TerminalWitness>> {
    let witness = read_failure_witness(session_dir)?;
    if witness.is_none()
        && ["status.json", "completed.json"].iter().any(|name| {
            read_file(session_dir.join(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|status| status["capture_failed"] == true)
        })
    {
        return Err(failure(
            session_dir,
            "capture failed without a valid terminal witness; spool retained",
        ));
    }
    Ok(witness)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Recording,
    Unsaved,
    Neither,
}

fn revision_json(revision: Revision) -> Value {
    let root: String = revision.root.iter().map(|byte| format!("{byte:02x}")).collect();
    json!([revision.number, revision.length, root])
}

fn revision_of(value: &Value) -> Option<Revision> {
    let hex = value[2].as_str()?;
    if hex.len() != 32 {
        return None;
    }
    let mut root = [0; 16];
    for (at, byte) in root.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * at..2 * at + 2)?, 16).ok()?;
    }
    Some(Revision {
        number: value[0].as_u64()?,
        length: value[1].as_u64()?,
        root,
    })
}

fn block_settings() -> replay_settings::compression::Settings {
    RecorderSettings::load()
        .map(|settings| settings.compression)
        .unwrap_or_default()
        .block_settings()
}

fn recording_stem(replay: &Replay, naming: Naming<'_>, multiplayer: bool) -> Result<String> {
    let _phase = replay_settings::packing::Packing::phase(replay_settings::packing::Phase::Publication);
    let name = crate::recording_name::recording_name(replay, naming.localisation, multiplayer)
        .map_err(|error| error.to_string())?;
    if let Some(report) = naming.report {
        report(name.by_tag.as_deref());
    }
    Ok(name.stem())
}

fn keeps_name(
    recording: &Path,
    stem: &str,
    settings: &RecorderSettings,
    folder: &Path,
    campaign: &[u8],
) -> bool {
    let current = recording
        .file_stem()
        .and_then(|current| current.to_str())
        .unwrap_or_default();
    recording_files::stem_is(current, stem) || settings.name_chosen(folder, campaign)
}

fn admitted_branch(
    admitted: Option<&BTreeSet<i64>>,
    bootstrap: &BTreeSet<BranchId>,
    branch: Option<BranchId>,
    signal: Option<i64>,
) -> bool {
    let signal = if branch.is_some_and(|branch| bootstrap.contains(&branch)) {
        Some(0)
    } else {
        signal
    };
    admitted.is_none_or(|admitted| signal.is_none_or(|signal| admitted.contains(&signal)))
}

fn failure_tracker(
    campaign: &[u8; 36],
    segments: &[(u32, Recovery<Recovered>)],
    session: SessionId,
    held: Option<&Path>,
    contexts: &[UncommittedRange],
) -> Result<BranchTracker> {
    let spool = replay::campaign::CampaignHistory::from_spool(campaign, segments)
        .map_err(|error| error.to_string())?;
    if held.is_none()
        && let Some(found) = spool.sessions.iter().find(|known| known.id == session)
        && spool.session_tail(session).is_some()
        && contexts.is_empty()
    {
        return spool.tracker_for(found).map_err(|error| error.to_string());
    }
    let recorded = held
        .map(|path| {
            held_recording(path)?
                .into_history()
                .map_err(|error| failure(path, error))
        })
        .transpose()?;
    let saved = |id: BranchId| {
        recorded
            .as_ref()
            .is_some_and(|history| history.branches.iter().any(|known| known.start.id == id))
    };
    let mut tracker = BranchTracker::new();
    let mut missing_hours = BTreeMap::new();
    for branch in recorded.iter().flat_map(|history| &history.branches).chain(
        spool
            .branches
            .iter()
            .filter(|branch| !saved(branch.start.id)),
    ) {
        let mut hours = branch.hours.clone();
        let mut last_missing_hour = branch.last_missing_hour;
        if saved(branch.start.id)
            && let Some(tail) = spool
                .branches
                .iter()
                .find(|tail| tail.start.id == branch.start.id)
        {
            if tail.start != branch.start {
                return Err("spool branch conflicts with saved branch history".into());
            }
            hours.extend_from_slice(&tail.hours);
            hours.sort_unstable();
            hours.dedup();
            last_missing_hour = last_missing_hour.max(tail.last_missing_hour);
        }
        if let Some(hour) = last_missing_hour {
            missing_hours.insert(branch.start.id, hour);
        }
        tracker
            .seed_recorded_branch(branch.start.clone(), hours)
            .map_err(|error| error.to_string())?;
    }
    let saved_tail = recorded
        .as_ref()
        .and_then(|history| history.session_tail(session));
    let tail = spool
        .session_tail(session)
        .into_iter()
        .chain(saved_tail)
        .max_by_key(|tail| (tail.branch, tail.hour, tail.recorded));
    let latest = spool
        .branches
        .iter()
        .filter(|branch| branch.start.session == session)
        .max_by_key(|branch| branch.start.id);
    if let Some(branch) =
        latest.filter(|branch| tail.is_none_or(|tail| branch.start.id > tail.branch))
        && !contexts.is_empty()
    {
        let range = contexts
            .iter()
            .find(|range| range.branch == branch.start.id);
        let observed = segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Recovered::Gap(gap) if gap.branch == branch.start.id => Some(gap.last_hour),
                _ => None,
            })
            .max()
            .or_else(|| range.map(|range| range.last_hour))
            .ok_or("gap-only branch has no witnessed observation")?;
        tracker
            .resume_observation(
                session,
                branch.start.id,
                observed,
                range
                    .and_then(|range| range.load_signal)
                    .and_then(replay_recording::command_packet::load_marker),
            )
            .map_err(|error| error.to_string())?;
    } else {
        match tail {
            Some(tail) => tail.resume(
                &mut tracker,
                session,
                missing_hours.get(&tail.branch).copied(),
            ),
            None => tracker.begin_session(session),
        }
        .map_err(|error| error.to_string())?;
    }
    Ok(tracker)
}

fn held_signal(recording: &Replay, branch: BranchId) -> Result<Option<i64>> {
    let Some(block) = recording
        .index
        .iter()
        .rev()
        .find(|block| block.branch == branch)
    else {
        return Ok(recording
            .controls
            .iter()
            .rev()
            .find_map(|control| match control {
                ControlRecord::PlayerCommands { packet, .. } if packet.branch == branch => {
                    packet.phase.as_ref().map(|phase| phase.load_signal)
                }
                _ => None,
            }));
    };
    let mut signal = None;
    recording
        .read_block_alone(
            block,
            &[TableId::Clock],
            |_, decoded| {
                signal = decoded
                    .complete::<tables::Clock>()
                    .ok()
                    .and_then(|rows| rows.first())
                    .map(|clock| clock.play_session_started_at);
                Ok(())
            },
            || Ok(()),
        )
        .map_err(|error| error.to_string())?;
    Ok(signal)
}

#[derive(Clone, Copy)]
struct Held<'a> {
    recording: &'a Replay,
    path: &'a Path,
}

fn apply_failure_witness(
    segments: &mut [(u32, Recovery<Recovered>)],
    witness: &TerminalWitness,
    failing_campaign: bool,
    uncommitted: Option<&UncommittedRange>,
    held: Option<Held<'_>>,
) -> Result<()> {
    let (inserted, derived) =
        failure_records(segments, witness, failing_campaign, uncommitted, held)?;
    let target = segments
        .iter_mut()
        .rev()
        .find(|(_, recovery)| !recovery.records.is_empty())
        .ok_or("failure witness has no recoverable record prefix")?;
    let position = target
        .1
        .records
        .iter()
        .position(|record| matches!(record, Recovered::Control(ControlRecord::Detach { .. })))
        .unwrap_or(target.1.records.len());
    if derived
        && let Some(Recovered::Control(ControlRecord::Detach { last_hour, .. })) =
            target.1.records.get_mut(position)
    {
        *last_hour = None;
    }
    target
        .1
        .records
        .splice(position..position, inserted)
        .for_each(drop);
    Ok(())
}

const FAILURE_SEGMENT: u32 = u32::MAX;

fn failure_segment(
    held: Held<'_>,
    witness: &TerminalWitness,
    failing_campaign: bool,
    uncommitted: Option<&UncommittedRange>,
) -> Result<Option<(u32, Recovery<Recovered>)>> {
    if uncommitted.is_none() && !(failing_campaign && witness.failed_hour.is_some()) {
        return Ok(None);
    }
    let recording = held.recording;
    let Some(session) = recording
        .sessions
        .iter()
        .find(|session| session.id == witness.session)
    else {
        return Ok(None);
    };
    let partial = ControlRecord::Partial {
        session: session.id,
        reason: ALLOCATION_GAP_REASON.into(),
        failed_hour: witness.failed_hour.filter(|_| failing_campaign),
    };
    let gapped = |branch: Option<BranchId>, first: Hour, last: Hour| {
        let mut gaps: Vec<_> = recording
            .gaps
            .iter()
            .filter(|gap| {
                branch.is_none_or(|branch| gap.branch == branch)
                    && recording
                        .branches
                        .iter()
                        .any(|known| known.id == gap.branch && known.session == session.id)
            })
            .collect();
        gaps.sort_by_key(|gap| gap.first_hour);
        let mut next = i64::from(first);
        for gap in gaps {
            if i64::from(gap.first_hour) <= next && i64::from(gap.last_hour) >= next {
                next = i64::from(gap.last_hour) + 1;
            }
        }
        next > i64::from(last)
    };
    if recording.controls.contains(&partial)
        && uncommitted
            .is_none_or(|range| gapped(Some(range.branch), range.first_hour, range.last_hour))
        && witness
            .failed_hour
            .filter(|_| failing_campaign)
            .is_none_or(|hour| gapped(witness.branch, hour, hour))
    {
        return Ok(None);
    }
    let mut records = vec![Recovered::SessionStart(session.clone())];
    records.extend(
        recording
            .branches
            .iter()
            .rev()
            .find(|branch| branch.session == session.id)
            .cloned()
            .map(Recovered::BranchStart),
    );
    let segment = [(
        FAILURE_SEGMENT,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records,
            valid_bytes: 0,
            damage: None,
        },
    )];
    let (inserted, _) =
        failure_records(&segment, witness, failing_campaign, uncommitted, Some(held))?;
    if inserted.is_empty() {
        return Ok(None);
    }
    let [(number, mut recovery)] = segment;
    if matches!(inserted.first(), Some(Recovered::BranchStart(_))) {
        recovery.records.truncate(1);
    }
    recovery.records.extend(inserted);
    Ok(Some((number, recovery)))
}

fn failure_records(
    segments: &[(u32, Recovery<Recovered>)],
    witness: &TerminalWitness,
    failing_campaign: bool,
    uncommitted: Option<&UncommittedRange>,
    held: Option<Held<'_>>,
) -> Result<(Vec<Recovered>, bool)> {
    failure_records_inner(
        segments,
        witness,
        failing_campaign,
        uncommitted,
        held,
        None,
        None,
    )
}

fn failure_observation(
    tracker: &mut BranchTracker,
    hour: Hour,
    signal: Option<i64>,
    gaps: &[Gap],
    expected: Option<BranchId>,
    bundled: bool,
) -> Result<replay_frame::branches::Observation> {
    let signal = signal.and_then(replay_recording::command_packet::load_marker);
    let mut observation = tracker
        .observe(None, hour, signal, gaps)
        .map_err(|error| error.to_string())?;
    if bundled
        && expected.is_some()
        && observation.start.is_none()
        && tracker
            .branches()
            .last()
            .and_then(|branch| branch.id.checked_add(1))
            == expected
    {
        tracker.world_changed();
        observation = tracker
            .observe(None, hour, signal, &[])
            .map_err(|error| error.to_string())?;
    }
    Ok(observation)
}

fn failure_records_inner(
    segments: &[(u32, Recovery<Recovered>)],
    witness: &TerminalWitness,
    failing_campaign: bool,
    uncommitted: Option<&UncommittedRange>,
    held: Option<Held<'_>>,
    partial_override: Option<&ControlRecord>,
    contexts: Option<&[UncommittedRange]>,
) -> Result<(Vec<Recovered>, bool)> {
    use replay_recording::command_packet::load_marker;
    let tail_branch = segments
        .iter()
        .rev()
        .find(|(_, recovery)| !recovery.records.is_empty())
        .and_then(|(_, recovery)| {
            recovery
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Recovered::BranchStart(start) => Some(start.id),
                    _ => None,
                })
        });
    let earlier_contexts = contexts.is_some();
    let mut current_branch: Option<BranchStart> = None;
    let mut current_signal = None;
    let mut current_observed = false;
    let mut matching_session = false;
    let mut recorded_branches = BTreeSet::new();
    let mut last_clock = None;
    let mut latest_phase: Option<&replay_recording::command_packet::Phase> = None;
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        match record {
            Recovered::SessionStart(session) => {
                if session.id != witness.session
                    || session.process_start_time != witness.process_start_time
                {
                    return Err("failure witness does not match the spool session".into());
                }
                matching_session = true;
            }
            Recovered::BranchStart(branch) => {
                if branch.session != witness.session {
                    return Err("failure witness does not match the spool branch session".into());
                }
                if current_branch.as_ref().map(|previous| previous.id) != Some(branch.id) {
                    current_signal = None;
                    current_observed = false;
                }
                recorded_branches.insert(branch.id);
                current_branch = Some(branch.clone());
            }
            Recovered::Frame(_) | Recovered::Gap(_) => current_observed = true,
            Recovered::Control(ControlRecord::ClockWitness {
                session,
                load_signal,
                qpc,
                ..
            }) => {
                if *session != witness.session {
                    return Err("failure witness does not match the spool clock session".into());
                }
                current_signal = load_signal.and_then(load_marker);
                current_observed = true;
                if let Some(branch) = &current_branch {
                    last_clock = Some((branch.id, *qpc));
                }
            }
            Recovered::Control(ControlRecord::PlayerCommands { session, packet }) => {
                if *session != witness.session {
                    return Err("failure witness does not match the command session".into());
                }
                if let Some(phase) = &packet.phase {
                    recorded_branches.insert(phase.start.id);
                    if latest_phase.is_none_or(|previous| previous.qpc < phase.qpc) {
                        latest_phase = Some(phase);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(phase) = latest_phase
        && (current_branch.is_none()
            || last_clock.is_some_and(|(branch, time)| {
                current_branch
                    .as_ref()
                    .is_some_and(|current| current.id == branch)
                    && phase.qpc > time
            }))
        && current_branch
            .as_ref()
            .is_none_or(|current| current.id != phase.start.id)
    {
        current_branch = Some(phase.start.clone());
        current_signal = phase.load_marker();
        current_observed = false;
    }
    if !matching_session {
        return Err("failure witness has no recorded session identity".into());
    }
    if let Some(held) = held {
        recorded_branches.extend(held.recording.branches.iter().map(|branch| branch.id));
        if let Some(current) = &current_branch
            && held
                .recording
                .index
                .iter()
                .any(|block| block.branch == current.id)
        {
            current_observed = true;
            if current_signal.is_none() {
                current_signal = held_signal(held.recording, current.id)?.and_then(load_marker);
            }
        }
        current_observed |= current_branch.as_ref().is_some_and(|current| {
            held.recording
                .gaps
                .iter()
                .any(|gap| gap.branch == current.id)
        });
    }
    let committed_hour = |branch: BranchId| {
        segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Recovered::Frame(frame) if frame.branch == branch => Some(frame.hour),
                _ => None,
            })
            .chain(
                held.iter()
                    .flat_map(|held| &held.recording.index)
                    .filter(|block| block.branch == branch)
                    .map(|block| block.last_hour),
            )
            .max()
    };
    let all_gaps = || {
        segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Recovered::Gap(gap) => Some(gap),
                _ => None,
            })
            .chain(held.iter().flat_map(|held| &held.recording.gaps))
    };
    let recorded_gaps = |branch: Option<BranchId>, before: Hour| -> Vec<Gap> {
        all_gaps()
            .filter(|gap| Some(gap.branch) == branch && gap.last_hour < before)
            .cloned()
            .collect()
    };
    let known_context = |id: BranchId| -> Result<Option<(BranchStart, Option<i64>)>> {
        let start = segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .find_map(|record| match record {
                Recovered::BranchStart(start) if start.id == id => Some(start.clone()),
                Recovered::Control(ControlRecord::PlayerCommands { packet, .. })
                    if packet.branch == id =>
                {
                    packet.phase.as_ref().map(|phase| phase.start.clone())
                }
                _ => None,
            })
            .or_else(|| {
                held.and_then(|held| {
                    held.recording
                        .branches
                        .iter()
                        .find(|start| start.id == id)
                        .cloned()
                })
            });
        let Some(start) = start else { return Ok(None) };
        if start.session != witness.session {
            return Err("failure witness does not match the saved branch session".into());
        }
        let mut active = None;
        let mut signal = None;
        for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
            match record {
                Recovered::BranchStart(start) => active = Some(start.id),
                Recovered::Control(ControlRecord::ClockWitness { load_signal, .. })
                    if active == Some(id) =>
                {
                    signal = load_signal.and_then(load_marker)
                }
                _ => {}
            }
        }
        if signal.is_none() {
            signal = segments
                .iter()
                .flat_map(|(_, recovery)| &recovery.records)
                .filter_map(|record| match record {
                    Recovered::Control(ControlRecord::PlayerCommands { packet, .. })
                        if packet.branch == id =>
                    {
                        packet.phase.as_ref()
                    }
                    _ => None,
                })
                .max_by_key(|phase| phase.qpc)
                .and_then(|phase| phase.load_marker());
        }
        if signal.is_none()
            && let Some(held) = held
        {
            signal = held_signal(held.recording, id)?.and_then(load_marker);
        }
        signal = signal.or_else(|| {
            contexts
                .and_then(|ranges| ranges.iter().find(|range| range.branch == id))
                .and_then(|range| range.load_signal)
                .and_then(load_marker)
        });
        Ok(Some((start, signal)))
    };
    if earlier_contexts
        && current_branch.is_none()
        && let Some(held) = held
    {
        current_branch = held
            .recording
            .branches
            .iter()
            .rev()
            .find(|branch| branch.session == witness.session)
            .cloned();
        if let Some(branch) = &current_branch {
            current_signal = held_signal(held.recording, branch.id)?.and_then(load_marker);
            current_observed = true;
        }
    }
    if earlier_contexts && current_signal.is_none() {
        current_signal = current_branch
            .as_ref()
            .and_then(|branch| {
                contexts.and_then(|ranges| ranges.iter().find(|range| range.branch == branch.id))
            })
            .and_then(|range| range.load_signal)
            .and_then(load_marker);
    }
    let mut tracker: Option<BranchTracker> = None;
    let mut range_start = None;
    let mut range_gap = None;
    if let Some(range) = uncommitted {
        if recorded_branches.contains(&range.branch) {
            if current_branch.as_ref().map(|branch| branch.id) != Some(range.branch) {
                if !earlier_contexts && current_branch.is_some() {
                    return Err("uncommitted range branch is not the last committed context".into());
                }
                let (start, signal) = known_context(range.branch)?
                    .ok_or("uncommitted range has no recorded branch")?;
                range_start = Some(start.clone());
                current_branch = Some(start);
                current_signal = signal;
            }
            if let (Some(expected), Some(recorded)) =
                (range.load_signal.and_then(load_marker), current_signal)
                && expected != recorded
            {
                return Err(
                    "uncommitted range load signal has no matching committed context".into(),
                );
            }
        } else {
            if tracker.is_none() {
                tracker = Some(failure_tracker(
                    &range.campaign_id,
                    segments,
                    witness.session,
                    held.map(|held| held.path),
                    contexts.unwrap_or_default(),
                )?);
            }
            let tracker = tracker.as_mut().unwrap();
            let known_gaps = recorded_gaps(tracker.current_branch(), range.first_hour);
            let observation = failure_observation(
                tracker,
                range.first_hour,
                range.load_signal,
                &known_gaps,
                Some(range.branch),
                earlier_contexts,
            )?;
            let start = observation
                .start
                .filter(|start| start.id == range.branch)
                .ok_or("uncommitted range branch differs from the witnessed clock context")?;
            current_branch = Some(start.clone());
            range_start = Some(start);
        }
        if tail_branch != Some(range.branch) {
            range_start = current_branch.clone();
        }
        let branch = current_branch.as_ref().unwrap();
        if range.first_hour < branch.first_hour {
            return Err("uncommitted range precedes the recorded branch context".into());
        }
        current_signal = range.load_signal.and_then(load_marker).or(current_signal);
        current_observed = true;
        let mut first_hour = Some(range.first_hour);
        if committed_hour(range.branch).is_some_and(|hour| hour >= range.first_hour) {
            return Err("uncommitted range overlaps or precedes a committed frame".into());
        }
        let gaps = recorded_gaps(Some(range.branch), Hour::MAX);
        while let Some(first) = first_hour
            && let Some(covering) = gaps
                .iter()
                .find(|gap| gap.first_hour <= first && gap.last_hour >= first)
        {
            first_hour = (covering.last_hour < range.last_hour).then(|| covering.last_hour + 1);
        }
        if let Some(first_hour) = first_hour {
            if gaps
                .iter()
                .any(|gap| gap.first_hour <= range.last_hour && gap.last_hour >= first_hour)
            {
                return Err("uncommitted range overlaps a recorded gap".into());
            }
            range_gap = Some(Gap {
                branch: range.branch,
                first_hour,
                last_hour: range.last_hour,
                reason: ALLOCATION_GAP_REASON.into(),
            });
        }
    }
    if failing_campaign
        && current_branch.is_none()
        && let Some(branch) = witness.branch
        && let Some((start, signal)) = known_context(branch)?
    {
        current_branch = Some(start);
        current_signal = signal;
        current_observed = held.is_some_and(|held| {
            held.recording
                .index
                .iter()
                .any(|block| block.branch == branch)
                || held.recording.gaps.iter().any(|gap| gap.branch == branch)
        });
    }
    let mut resolved_branch = witness.branch;
    let mut attempt_start = None;
    if earlier_contexts
        && failing_campaign
        && let Some(branch) = witness.branch
        && current_branch.as_ref().map(|current| current.id) != Some(branch)
        && let Some((start, signal)) = known_context(branch)?
    {
        attempt_start = Some(start.clone());
        current_branch = Some(start);
        current_signal = signal;
        current_observed = true;
    }
    let in_range = uncommitted.is_some_and(|range| {
        witness
            .failed_hour
            .is_some_and(|hour| (range.first_hour..=range.last_hour).contains(&hour))
            && witness.branch.is_none_or(|branch| branch == range.branch)
            && !matches!((witness.load_signal.and_then(load_marker), range.load_signal.and_then(load_marker)),
                (Some(attempted), Some(admitted)) if attempted != admitted)
    });
    if failing_campaign && in_range {
        resolved_branch = uncommitted.map(|range| range.branch);
        current_signal = current_signal.or(witness.load_signal.and_then(load_marker));
    } else if failing_campaign
        && let Some(failed_hour) = witness.failed_hour
        && witness
            .branch
            .is_none_or(|branch| current_branch.as_ref().map(|current| current.id) != Some(branch))
    {
        if tracker.is_none() {
            tracker = Some(failure_tracker(
                &witness.campaign_id.unwrap(),
                segments,
                witness.session,
                held.map(|held| held.path),
                contexts.unwrap_or_default(),
            )?);
        }
        let tracker = tracker.as_mut().unwrap();
        let mut known_gaps = recorded_gaps(tracker.current_branch(), failed_hour);
        if let Some(range) = uncommitted
            && Some(range.branch) == tracker.current_branch()
            && range.last_hour < failed_hour
        {
            known_gaps.push(Gap {
                branch: range.branch,
                first_hour: range.first_hour,
                last_hour: range.last_hour,
                reason: ALLOCATION_GAP_REASON.into(),
            });
        }
        let observation = failure_observation(
            tracker,
            failed_hour,
            witness.load_signal,
            &known_gaps,
            witness.branch,
            earlier_contexts,
        )?;
        if witness
            .branch
            .is_some_and(|expected| expected != observation.branch)
        {
            return Err("failure witness branch differs from the witnessed clock context".into());
        }
        resolved_branch = Some(observation.branch);
        current_signal = witness.load_signal.and_then(load_marker).or(current_signal);
        if let Some(start) = observation.start {
            current_branch = Some(start.clone());
            attempt_start = Some(start);
        }
    }
    if failing_campaign && tail_branch != resolved_branch {
        attempt_start = current_branch.clone();
    }
    let mut attempt_gap = None;
    if failing_campaign {
        if let Some(expected) = witness.branch
            && current_branch.as_ref().map(|branch| branch.id) != Some(expected)
        {
            return Err("failure witness branch has no matching committed context".into());
        }
        if let Some(expected) = witness.load_signal.and_then(load_marker)
            && current_signal != Some(expected)
        {
            let first_clock_failed = !current_observed
                && current_signal.is_none()
                && current_branch.as_ref().is_some_and(|branch| {
                    witness.branch == Some(branch.id)
                        && witness.failed_hour == Some(branch.first_hour)
                });
            if !first_clock_failed {
                return Err("failure witness load signal has no matching committed context".into());
            }
        }
        if let Some(hour) = witness.failed_hour {
            let branch = current_branch
                .as_ref()
                .filter(|branch| Some(branch.id) == resolved_branch)
                .ok_or("known failure hour has no unambiguous recorded branch context")?;
            if hour < branch.first_hour {
                return Err("failure hour precedes the recorded branch context".into());
            }
            if committed_hour(branch.id).is_some_and(|committed| committed >= hour) {
                return Err("failure hour overlaps or precedes a committed frame".into());
            }
            let mut already_gapped = false;
            for existing in all_gaps().filter(|existing| {
                existing.branch == branch.id
                    && existing.first_hour <= hour
                    && existing.last_hour >= hour
            }) {
                if existing.reason != ALLOCATION_GAP_REASON {
                    return Err(
                        "failure hour already belongs to a gap with a different reason".into(),
                    );
                }
                already_gapped = true;
            }
            if !already_gapped {
                attempt_gap = Some(Gap {
                    branch: branch.id,
                    first_hour: hour,
                    last_hour: hour,
                    reason: ALLOCATION_GAP_REASON.into(),
                });
            }
        }
    }
    let partial = partial_override.cloned().unwrap_or(ControlRecord::Partial {
        session: witness.session,
        reason: ALLOCATION_GAP_REASON.into(),
        failed_hour: if failing_campaign {
            witness.failed_hour
        } else {
            None
        },
    });
    let mut partial_exists = false;
    for control in segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .filter_map(|record| {
            if let Recovered::Control(control @ ControlRecord::Partial { .. }) = record {
                Some(control)
            } else {
                None
            }
        })
        .chain(
            held.iter()
                .flat_map(|held| &held.recording.controls)
                .filter(|control| {
                    matches!(control, ControlRecord::Partial { session, .. } if *session == witness.session)
                }),
        )
    {
        if control != &partial {
            return Err("recorded partial outcome differs from the failure witness".into());
        }
        partial_exists = true;
    }
    if range_gap.is_none() {
        range_start = None;
    }
    if attempt_gap.is_none() {
        attempt_start = None;
    }
    let mut inserted = Vec::new();
    let derived = range_start.is_some() || attempt_start.is_some();
    if let Some(start) = range_start {
        inserted.push(Recovered::BranchStart(start));
    }
    match (range_gap, attempt_gap) {
        (Some(range), Some(attempt)) if range.branch == attempt.branch => {
            if i64::from(attempt.first_hour) <= i64::from(range.last_hour) + 1
                && i64::from(range.first_hour) <= i64::from(attempt.last_hour) + 1
            {
                inserted.push(Recovered::Gap(Gap {
                    first_hour: range.first_hour.min(attempt.first_hour),
                    last_hour: range.last_hour.max(attempt.last_hour),
                    ..range
                }));
            } else if attempt.first_hour < range.first_hour {
                inserted.extend([Recovered::Gap(attempt), Recovered::Gap(range)]);
            } else {
                inserted.extend([Recovered::Gap(range), Recovered::Gap(attempt)]);
            }
        }
        (range, attempt) => {
            inserted.extend(range.map(Recovered::Gap));
            inserted.extend(attempt_start.map(Recovered::BranchStart));
            inserted.extend(attempt.map(Recovered::Gap));
        }
    }
    if !partial_exists {
        inserted.push(Recovered::Control(partial));
    }
    Ok((inserted, derived))
}

fn insert_failure_records(
    segments: &mut [(u32, Recovery<Recovered>)],
    mut records: Vec<Recovered>,
    branch: Option<BranchId>,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let mut place = None;
    if let Some(branch) = branch {
        for (segment, (_, recovery)) in segments.iter().enumerate() {
            let mut active = None;
            for (index, record) in recovery.records.iter().enumerate() {
                if matches!(
                    record,
                    Recovered::BranchStart(_) | Recovered::Control(ControlRecord::Detach { .. })
                ) && active == Some(branch)
                {
                    place = Some((segment, index));
                    if matches!(record, Recovered::Control(ControlRecord::Detach { .. })) {
                        active = None;
                    }
                }
                if let Recovered::BranchStart(start) = record {
                    active = Some(start.id);
                }
            }
            if active == Some(branch) {
                place = Some((segment, recovery.records.len()));
            }
        }
        if place.is_some()
            && matches!(records.first(), Some(Recovered::BranchStart(start)) if start.id == branch)
        {
            records.remove(0);
        }
        if place.is_none() {
            place = segments.iter().enumerate().find_map(|(segment, (_, recovery))| {
                recovery.records.iter().position(|record| matches!(record, Recovered::BranchStart(start) if start.id > branch)).map(|index| (segment, index))
            });
        }
    }
    let (segment, index) = place
        .or_else(|| {
            segments
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (_, recovery))| !recovery.records.is_empty())
                .map(|(segment, (_, recovery))| {
                    let index = recovery
                        .records
                        .iter()
                        .position(|record| {
                            matches!(record, Recovered::Control(ControlRecord::Detach { .. }))
                        })
                        .unwrap_or(recovery.records.len());
                    (segment, index)
                })
        })
        .ok_or("failure witness has no recoverable record prefix")?;
    if records
        .iter()
        .any(|record| matches!(record, Recovered::BranchStart(_)))
        && let Some(Recovered::Control(ControlRecord::Detach { last_hour, .. })) =
            segments[segment].1.records.get_mut(index)
    {
        *last_hour = None;
    }
    segments[segment]
        .1
        .records
        .splice(index..index, records)
        .for_each(drop);
    Ok(())
}

fn apply_failure_ranges(
    segments: &mut [(u32, Recovery<Recovered>)],
    witness: &TerminalWitness,
    failing_campaign: bool,
    ranges: &[UncommittedRange],
    bundled: bool,
    held: Option<Held<'_>>,
) -> Result<()> {
    if !bundled && ranges.len() <= 1 {
        return apply_failure_witness(segments, witness, failing_campaign, ranges.first(), held);
    }
    let partial = ControlRecord::Partial {
        session: witness.session,
        reason: ALLOCATION_GAP_REASON.into(),
        failed_hour: witness.failed_hour.filter(|_| failing_campaign),
    };
    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|range| range.branch);
    if failing_campaign && let Some(hour) = witness.failed_hour {
        for range in &mut ranges {
            if witness.branch == Some(range.branch)
                && witness.load_signal == range.load_signal
                && i64::from(hour) >= i64::from(range.first_hour) - 1
                && i64::from(hour) <= i64::from(range.last_hour) + 1
            {
                range.first_hour = range.first_hour.min(hour);
                range.last_hour = range.last_hour.max(hour);
                break;
            }
        }
    }
    for range in &ranges {
        let (mut inserted, _) = failure_records_inner(
            segments,
            witness,
            false,
            Some(range),
            held,
            Some(&partial),
            Some(&ranges),
        )?;
        inserted
            .retain(|record| !matches!(record, Recovered::Control(ControlRecord::Partial { .. })));
        insert_failure_records(segments, inserted, Some(range.branch))?;
    }
    let (inserted, _) = failure_records_inner(
        segments,
        witness,
        failing_campaign,
        None,
        held,
        Some(&partial),
        Some(&ranges),
    )?;
    insert_failure_records(
        segments,
        inserted,
        witness.branch.filter(|_| failing_campaign),
    )
}

fn failure_segment_ranges(
    held: Held<'_>,
    witness: &TerminalWitness,
    failing_campaign: bool,
    ranges: &[UncommittedRange],
    bundled: bool,
) -> Result<Option<(u32, Recovery<Recovered>)>> {
    if ranges.is_empty() && !(failing_campaign && witness.failed_hour.is_some()) {
        return Ok(None);
    }
    if !bundled && ranges.len() <= 1 {
        return failure_segment(held, witness, failing_campaign, ranges.first());
    }
    let Some(session) = held
        .recording
        .sessions
        .iter()
        .find(|session| session.id == witness.session)
    else {
        return Ok(None);
    };
    let records = vec![Recovered::SessionStart(session.clone())];
    let mut segments = [(
        FAILURE_SEGMENT,
        Recovery {
            format: Some(FormatVersion::CURRENT),
            records,
            valid_bytes: 0,
            damage: None,
        },
    )];
    let before = segments[0].1.records.len();
    apply_failure_ranges(
        &mut segments,
        witness,
        failing_campaign,
        ranges,
        bundled,
        Some(held),
    )?;
    if segments[0].1.records.len() == before {
        return Ok(None);
    }
    let [segment] = segments;
    Ok(Some(segment))
}

fn held_recording(path: &Path) -> Result<Replay> {
    Replay::open_newest(path).map_err(|error| failure(path, error))
}

fn print_script_events(branch: BranchId, state: &replay_data::event_feed_rows::ScriptEventState) {
    println!("script_events branch={branch} hour={:?}", state.hour());
    match state.pending() {
        Ok(rows) => {
            println!("pending_events count={}", rows.len());
            for row in rows {
                println!(
                    "pending_event instance={} event={} due_hour={} originator={:?} scopes={:?}",
                    row.instance,
                    String::from_utf8_lossy(&row.definition.token),
                    row.due_hour,
                    row.originator,
                    row.scopes
                );
            }
        }
        Err(reason) => println!("pending_events unavailable={reason}"),
    }
    match state.fired() {
        Ok(rows) => {
            println!("fired_event_names count={}", rows.len());
            for row in rows {
                println!(
                    "fired_event_name {} native={}",
                    String::from_utf8_lossy(&row.token),
                    row.native_event
                );
            }
        }
        Err(reason) => println!("fired_event_names unavailable={reason}"),
    }
    for (country, rows) in state.delayed_owners() {
        match rows {
            Ok(rows) => {
                println!("delayed_events country={country:?} count={}", rows.len());
                for row in rows {
                    println!(
                        "delayed_event instance={} event={} remaining={} originator={:?} scopes={:?}",
                        row.instance,
                        String::from_utf8_lossy(&row.definition.token),
                        row.remaining,
                        row.originator,
                        row.scopes
                    );
                }
            }
            Err(reason) => println!("delayed_events country={country:?} unavailable={reason}"),
        }
    }
}

fn print_owner_refusals(
    branch: BranchId,
    row: &replay_data::event_feed_rows::EventFeedHistoryRow,
) -> (u64, u64) {
    use replay_data::event_feed_rows::{EventFeedScriptChange as Change, decisions::Operation};
    let mut counts = (0, 0);
    for event in &row.events {
        let Some(script) = event.script else {
            continue;
        };
        let (registry, owner) = match script.change {
            Some(Change::DelayedRefused) => {
                counts.0 += 1;
                ("delayed", event.target)
            }
            Some(Change::Decision(change)) if change.operation == Operation::Refused => {
                counts.1 += 1;
                ("decisions", event.actor)
            }
            _ => continue,
        };
        let start = script.token_start as usize;
        let reason = row
            .script_tokens
            .get(start..start + script.token_len as usize)
            .unwrap_or_default();
        println!(
            "event_feed_owner_refusal branch={branch} hour={} registry={registry} owner={owner:?} reason={}",
            event.hour,
            String::from_utf8_lossy(reason)
        );
    }
    counts
}

fn stop_lines(replay: &Replay) -> Vec<String> {
    replay
        .stops
        .iter()
        .map(|stop| {
            format!(
                "stop spool={} capture={} branch={} last_hour={:?}: {}",
                stop.spool, stop.capture, stop.branch, stop.last_hour, stop.reason
            )
        })
        .collect()
}

pub fn inspect(path: &Path) -> Result<()> {
    let replay = held_recording(path)?;
    println!(
        "replay={} format={} bytes={} campaign={:?} revision={}",
        path.display(),
        replay.format.version(),
        replay.saved_length(),
        String::from_utf8_lossy(&replay.campaign_id),
        replay.revision().number
    );
    print_identity("header", &replay.identity);
    println!(
        "content_verified={} sessions={} branches={} blocks={}",
        replay.content_verified(),
        replay.sessions.len(),
        replay.branches.len(),
        replay.index.len()
    );
    for session in &replay.sessions {
        println!(
            "session={} pid={} process_start_filetime={} qpc_frequency={} sample_boundary={}",
            session.id,
            session.process_id,
            session.process_start_time,
            session.qpc_frequency,
            session.sample_boundary
        );
        print_identity(&format!("session {}", session.id), &session.identity);
    }
    let mut summary = Summary::default();
    let mut script_states = std::collections::BTreeMap::<
        BranchId,
        replay_data::event_feed_rows::ScriptEventState,
    >::new();
    let mut owner_refusals = std::collections::BTreeMap::<BranchId, (u64, u64)>::new();
    let blocks: Vec<_> = replay.index.iter().collect();
    let mut shown = None;
    replay
        .visit_blocks(
            &blocks,
            &TableId::ALL,
            |index, recorded, decoded| {
                if shown != Some((index.run, index.page)) {
                    shown = Some((index.run, index.page));
                    println!(
                        "block branch={} hours={}..={} run={} page={}",
                        index.branch, index.first_hour, index.last_hour, index.run, index.page
                    );
                }
                summary.observe(recorded, decoded).map_err(std::io::Error::other)?;
                match decoded.complete::<replay_data::rows::tables::EventFeedHistory>() {
                    Ok(rows) => {
                        for row in rows {
                            script_states
                                .entry(index.branch)
                                .or_default()
                                .observe(row)
                                .map_err(std::io::Error::other)?;
                            let (delayed, decisions) = print_owner_refusals(index.branch, row);
                            let counts = owner_refusals.entry(index.branch).or_default();
                            counts.0 += delayed;
                            counts.1 += decisions;
                        }
                    }
                    Err(reason) => {
                        script_states.remove(&index.branch);
                        println!(
                            "script_events branch={} hour={} unavailable={reason}",
                            index.branch, decoded.hour
                        );
                    }
                }
                Ok(true)
            },
            || Ok(()),
        )
        .map_err(|error| failure(path, error))?;
    for (branch, state) in &script_states {
        print_script_events(*branch, state);
    }
    for (branch, (delayed, decisions)) in &owner_refusals {
        println!(
            "event_feed_owner_refusals branch={branch} delayed={delayed} decisions={decisions}"
        );
    }
    for branch in &replay.branches {
        let parent = match branch.parent {
            Parentage::Unknown => "unknown".into(),
            Parentage::Probable { branch, hour } => format!(
                "probable({branch}, {hour}); supporting hour {}",
                if summary
                    .hours
                    .get(&branch)
                    .is_some_and(|hours| hours.contains(&hour))
                {
                    "present"
                } else {
                    "missing from recovered replay"
                }
            ),
        };
        println!(
            "branch={} session={} loaded_hour={:?} first_hour={} reason={:?} parent={parent}",
            branch.id, branch.session, branch.loaded_hour, branch.first_hour, branch.reason
        );
    }
    for gap in &replay.gaps {
        println!(
            "gap branch={} hours={}..={}: {}",
            gap.branch, gap.first_hour, gap.last_hour, gap.reason
        );
    }
    for branch in &replay.branches {
        if let Some(hours) = summary.hours.get(&branch.id) {
            for (first, last) in missing_ranges(branch.first_hour, hours) {
                println!(
                    "unrecorded branch={} hours={first}..={last}; see gap and recovery records for available reasons",
                    branch.id
                );
            }
        }
    }
    for note in &replay.recovery_notes {
        println!(
            "recovery session={:?} segment={} valid_bytes={}: {}",
            note.session, note.segment, note.valid_bytes, note.reason
        );
    }
    for control in &replay.controls {
        match control {
            ControlRecord::Detach { session, last_hour } => {
                println!("detach session={session} last_hour={last_hour:?}")
            }
            ControlRecord::GapSummary {
                session,
                dropped_frames,
                reason,
            } => println!(
                "gap_summary session={session} dropped_frames={dropped_frames} hour_range=unknown: {reason}"
            ),
            ControlRecord::Partial {
                session,
                reason,
                failed_hour,
            } => {
                println!("partial session={session} failed_hour={failed_hour:?}: {reason}")
            }
            ControlRecord::EventLoss {
                session,
                branch,
                after_hour,
                hour,
            } => println!(
                "event_loss session={session} branch={branch} between_reads={after_hour}..{hour}: the collector was detached for part of this interval; every event family's record of it is incomplete"
            ),
            ControlRecord::PlayerCommands { session, packet } => println!(
                "commands session={session} branch={} world={} packet={} lost_observations={}",
                packet.branch, packet.world, packet.ordinal, packet.lost
            ),
            ControlRecord::ClockWitness { .. } => {}
        }
    }
    for line in stop_lines(&replay) {
        println!("{line}");
    }
    summary.print();
    for issues in &summary.issues {
        for issue in issues {
            println!(
                "table_gap branch={} table={:?} hours={}..={}: {}",
                issue.branch, issue.table, issue.first, issue.last, issue.reason
            );
        }
    }
    print_throughput(&replay.sessions, replay.controls.iter().map(Some));
    print_map_events(path, &replay)?;
    println!(
        "oracle=not run; this command inspects recorded data without asserting save agreement"
    );
    Ok(())
}

fn print_map_events(path: &Path, replay: &Replay) -> Result<()> {
    use replay_data::{rows::MapEventFamily, rows::MapEventPayload, rows::TableId, rows::tables};
    let selected = [TableId::MapEvents];
    let mut complete = [0_usize; MapEventFamily::COUNT];
    let mut incomplete: [BTreeMap<String, usize>; MapEventFamily::COUNT] = Default::default();
    let mut events = [0_usize; MapEventFamily::COUNT];
    let blocks: Vec<_> = replay.index.iter().collect();
    replay
        .visit_blocks(
            &blocks,
            &selected,
            |_, _, decoded| {
                let Ok([row]) = decoded.complete::<tables::MapEvents>() else {
                    return Ok(true);
                };
                    for family in MapEventFamily::ALL {
                        let health = row.health(family);
                        if health.complete {
                            complete[family.index()] += 1;
                        } else {
                            let faults = health
                                .faults
                                .iter()
                                .map(|fault| fault.name())
                                .collect::<Vec<_>>()
                                .join("+");
                            *incomplete[family.index()].entry(faults).or_default() += 1;
                        }
                    }
                    for event in &row.events {
                        events[event.payload.family().index()] += 1;
                        let detail = match &event.payload {
                            MapEventPayload::FakeDivisionRevealed(unit)
                            | MapEventPayload::RailwayGunOverrun(unit) => format!(
                                "owner={} province={} visibility_province={} fog={} intel_entries={}",
                                unit.owner,
                                unit.anchor_province,
                                unit.visibility_province,
                                unit.fog,
                                unit.intel.len()
                            ),
                            MapEventPayload::CriticalShoreBombardment(critical) => {
                                format!("province={}", critical.province)
                            }
                            MapEventPayload::GeneralCaptured(capture) => format!(
                                "owner={} captor={} province={}",
                                capture.owner, capture.captor, capture.province
                            ),
                            MapEventPayload::ParadropLanding(paradrop) => format!(
                                "province={} country={} lost={} fog={}",
                                paradrop.province, paradrop.country, paradrop.lost, paradrop.fog
                            ),
                            MapEventPayload::ResistanceActivity(activity) => format!(
                                "province={} controller={} overlord={} key={:?}",
                                activity.province,
                                activity.controller,
                                activity.overlord,
                                String::from_utf8_lossy(&activity.key)
                            ),
                            MapEventPayload::ScriptedMapRefresh(mode) => format!("scripted_mode={mode}"),
                            MapEventPayload::BuildingDamage(building) => format!(
                                "building_type={} province={} controller={} reason={:?}",
                                building.building_type,
                                building.province,
                                building.controller,
                                building.reason
                            ),
                        };
                        println!(
                            "map_event branch={} interval_hour={} sequence={} hour={} family={} {detail}",
                            decoded.branch,
                            decoded.hour,
                            event.sequence,
                            event.hour,
                            event.payload.family().name()
                        );
                    }
                Ok(true)
            },
            || Ok(()),
        )
        .map_err(|error| failure(path, error))?;
    for family in MapEventFamily::ALL {
        let index = family.index();
        let incomplete = incomplete[index]
            .iter()
            .map(|(faults, count)| format!("{faults}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "map_events family={} events={} complete_intervals={} incomplete_intervals={}",
            family.name(),
            events[index],
            complete[index],
            if incomplete.is_empty() {
                "0".into()
            } else {
                incomplete
            }
        );
    }
    Ok(())
}

pub fn inspect_characters(path: &Path, needle: &str, hour: Hour, branch: BranchId) -> Result<()> {
    let replay = held_recording(path)?;
    let frame = match replay
        .seek(branch, hour)
        .map_err(|error| failure(path, error))?
    {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => {
            return Err(format!("branch {branch}, hour {hour} is a gap: {reason}"));
        }
    };
    let lines = character_lines(&frame, needle.as_bytes())?;
    println!(
        "characters branch={branch} hour={hour} matching {needle:?}: {}",
        lines.len()
    );
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

fn character_lines(frame: &DecodedFrame, needle: &[u8]) -> Result<Vec<String>> {
    if needle.is_empty() {
        return Err("--characters needs a nonempty substring".into());
    }
    let contains = |haystack: &[u8]| haystack.windows(needle.len()).any(|part| part == needle);
    let rows = frame
        .complete::<tables::Characters>()
        .map_err(|error| error.to_string())?;
    let literals = frame
        .complete::<tables::LexicalTokens>()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.token, row.literal.as_slice()))
                .collect::<BTreeMap<_, _>>()
        })
        .map_err(|error| error.to_string());
    Ok(rows
        .iter()
        .filter(|row| contains(&row.token) || contains(&row.name))
        .map(|row| {
            let literal = match &literals {
                Ok(literals) => match literals.get(&row.template_tag) {
                    Some(literal) => format!("{:?}", String::from_utf8_lossy(literal)),
                    None => "not in LexicalTokens".into(),
                },
                Err(error) => format!("LexicalTokens unreadable: {error}"),
            };
            format!(
                "character id={} country={} token={:?} name={:?} template_present={} template_tag={} ({literal}) template_file={:?} template_line={} template_generated={}",
                row.id,
                row.country,
                String::from_utf8_lossy(&row.token),
                String::from_utf8_lossy(&row.name),
                row.template_present,
                row.template_tag,
                String::from_utf8_lossy(&row.template_file),
                row.template_line,
                row.template_generated
            )
        })
        .collect())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

fn missing_ranges(first_hour: Hour, hours: &BTreeSet<Hour>) -> Vec<(Hour, Hour)> {
    let mut missing = Vec::new();
    let mut next = i64::from(first_hour);
    for &hour in hours {
        if i64::from(hour) > next {
            missing.push((next as Hour, hour - 1));
        }
        next = i64::from(hour) + 1;
    }
    missing
}

fn print_identity(label: &str, identity: &Identity) {
    println!(
        "{label}: game={:?} exe_sha256={} exe_bytes={} vanilla_sha256={} language={:?}",
        identity.game_version,
        hex(&identity.exe_hash),
        identity.exe_size,
        hex(&identity.vanilla_hash),
        identity.language
    );
    println!(
        "{label}: coverage_policy={} recorder={:?} collector_sha256={}",
        identity.coverage_policy,
        identity.recorder_version,
        hex(&identity.collector_hash)
    );
    if identity.coverage_policy == replay_data::identity::COVERAGE_POLICY_VERSION {
        println!(
            "coverage: descriptor (a mod's replace_path and dependencies only) plus map, common, history, events, localisation, interface, gfx/flags, gfx/interface, gfx/texticons, gfx/leaders, gfx/fonts, gfx/FX, gfx/posteffect_volumes.txt and every .asset and .gfx descriptor below gfx; integrated DLC and Tweaker UI roots are included in the vanilla hash under their install-relative paths; music, sound, video, loading screens, launcher, crash reporter and logs excluded"
        );
    } else {
        println!("coverage: this recorder does not recognize the policy; no coverage claim");
    }
    for (position, layer) in identity.dlc.iter().enumerate() {
        println!(
            "dlc[{position}] id={:?} sha256={} declared_checksum={:?}",
            layer.id,
            hex(&layer.hash),
            layer.declared_checksum
        );
    }
    for (position, layer) in identity.mods.iter().enumerate() {
        println!(
            "mod[{position}] id={:?} name={:?} declared_version={:?} sha256={}",
            layer.id,
            layer.display_name,
            layer.declared_version,
            hex(&layer.hash)
        );
    }
    match &identity.verification {
        Verification::Unchecked => println!("{label}: verification=unchecked"),
        Verification::Verified { launcher, memory } => {
            println!("{label}: verification=verified launcher={launcher:?} memory={memory:?}")
        }
        Verification::Unverified {
            reason,
            launcher,
            memory,
        } => println!(
            "{label}: verification=unverified reason={reason:?} launcher={launcher:?} memory={memory:?}"
        ),
    }
}

#[derive(Debug)]
struct Issue {
    branch: BranchId,
    table: TableId,
    first: Hour,
    last: Hour,
    reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateKind {
    Snapshot,
    Delta,
    Unchanged,
    Truncated,
    Unavailable,
    NotRecorded,
}

struct Summary {
    totals: Vec<u64>,
    table_times: [Vec<u64>; TABLE_COUNT],
    untimed_hours: u64,
    states: [[u64; 5]; TABLE_COUNT],
    max_pool: u64,
    max_dropped: u64,
    hours: BTreeMap<BranchId, BTreeSet<Hour>>,
    issues: [Vec<Issue>; TABLE_COUNT],
}

impl Default for Summary {
    fn default() -> Self {
        Self {
            totals: Vec::new(),
            table_times: std::array::from_fn(|_| Vec::new()),
            untimed_hours: 0,
            states: [[0; 5]; TABLE_COUNT],
            max_pool: 0,
            max_dropped: 0,
            hours: BTreeMap::new(),
            issues: std::array::from_fn(|_| Vec::new()),
        }
    }
}

impl Summary {
    fn observe(
        &mut self,
        recorded: &replay_frame::frame::RecordedFrame,
        decoded: &DecodedFrame,
    ) -> Result<()> {
        let metrics = &decoded.metrics;
        self.totals.push(metrics.total_ns);
        self.max_pool = self.max_pool.max(metrics.pool_bytes);
        self.max_dropped = self.max_dropped.max(metrics.dropped_frames);
        if !self.hours.entry(decoded.branch).or_default().insert(decoded.hour) {
            return Err(format!(
                "duplicate hour on branch {} at hour {}",
                decoded.branch, decoded.hour
            ));
        }
        let timed = metrics.table_times_recorded();
        self.untimed_hours += u64::from(!timed);
        for table in TableId::ALL {
            let i = table.index();
            let view = &*decoded.tables[i];
            let kind = match (&recorded.tables[i], view) {
                (_, TableView::NotRecorded { .. }) => UpdateKind::NotRecorded,
                (_, TableView::Unavailable { .. }) => UpdateKind::Unavailable,
                (_, TableView::Truncated { .. }) => UpdateKind::Truncated,
                (TableUpdate::Snapshot(_), _) => UpdateKind::Snapshot,
                (TableUpdate::Delta { .. }, _) => UpdateKind::Delta,
                _ => UpdateKind::Unchanged,
            };
            if kind == UpdateKind::NotRecorded {
                continue;
            }
            if timed {
                self.table_times[i].push(metrics.table_ns[i]);
            }
            let state = match kind {
                UpdateKind::Snapshot => 0,
                UpdateKind::Delta => 1,
                UpdateKind::Unchanged => 2,
                UpdateKind::Truncated => 3,
                UpdateKind::Unavailable => 4,
                UpdateKind::NotRecorded => unreachable!("skipped above"),
            };
            self.states[i][state] += 1;
            let reason = match view {
                TableView::Truncated { missing, .. } => format!("truncated; {missing} rows not read"),
                TableView::Unavailable { reason } => format!("unavailable; {reason}"),
                _ => continue,
            };
            let issues = &mut self.issues[i];
            if let Some(previous) = issues.last_mut()
                && previous.branch == decoded.branch
                && previous.last.checked_add(1) == Some(decoded.hour)
                && previous.reason == reason
            {
                previous.last = decoded.hour;
            } else {
                issues.push(Issue {
                    branch: decoded.branch,
                    table,
                    first: decoded.hour,
                    last: decoded.hour,
                    reason,
                });
            }
        }
        Ok(())
    }

    fn print(&mut self) {
        if let Err(error) = self.write(&mut std::io::stdout().lock()) {
            eprintln!("Summary could not be printed: {error}");
        }
    }

    fn write(&mut self, output: &mut impl std::io::Write) -> std::io::Result<()> {
        let hours = self.totals.len();
        if hours == 0 {
            writeln!(
                output,
                "hours=0 total_p50_ms=not_observed total_p95_ms=not_observed total_max_ms=not_observed pool_max_bytes=not_observed dropped_counter_max=not_observed"
            )?;
            return Ok(());
        }
        let [p50, p95, maximum] = percentiles(&mut self.totals);
        writeln!(
            output,
            "hours={hours} untimed_hours={} total_p50_ms={p50:.3} total_p95_ms={p95:.3} total_max_ms={maximum:.3} pool_max_bytes={} dropped_counter_max={}",
            self.untimed_hours, self.max_pool, self.max_dropped
        )?;
        writeln!(
            output,
            "table read_p50_ms read_p95_ms read_max_ms snapshots deltas unchanged truncated unavailable"
        )?;
        for table in TableId::ALL {
            let i = table.index();
            if self.states[i].iter().all(|&count| count == 0) {
                continue;
            }
            let times = self.read_times(table);
            let [snapshots, deltas, unchanged, truncated, unavailable] = self.states[i];
            writeln!(
                output,
                "{table:?} {times} {snapshots} {deltas} {unchanged} {truncated} {unavailable}"
            )?;
        }
        Ok(())
    }

    fn read_times(&mut self, table: TableId) -> String {
        let times = &mut self.table_times[table.index()];
        if times.is_empty() {
            return "not_recorded not_recorded not_recorded".into();
        }
        let [p50, p95, maximum] = percentiles(times);
        format!("{p50:.3} {p95:.3} {maximum:.3}")
    }
}

fn percentiles(values: &mut [u64]) -> [f64; 3] {
    crate::metrics::percentiles_ns(values)
        .unwrap_or([0; 3])
        .map(|ns| ns as f64 / 1_000_000.0)
}

#[derive(Default)]
struct Throughput {
    previous: BTreeMap<SessionId, (u64, Hour, Option<i64>)>,
    hours: u64,
    seconds: f64,
}

impl Throughput {
    fn witness(
        &mut self,
        session: SessionId,
        frequency: u64,
        qpc: u64,
        hour: Hour,
        signal: Option<i64>,
    ) {
        let previous = self.previous.insert(session, (qpc, hour, signal));
        if frequency != 0
            && let Some((last_qpc, last_hour, last_signal)) = previous
            && last_hour.checked_add(1) == Some(hour)
            && signal == last_signal
            && qpc > last_qpc
        {
            self.hours += 1;
            self.seconds += (qpc - last_qpc) as f64 / frequency as f64;
        }
    }
}

fn print_throughput<'a>(
    sessions: &[SessionStart],
    controls: impl Iterator<Item = Option<&'a ControlRecord>>,
) {
    let frequencies: BTreeMap<_, _> = sessions
        .iter()
        .map(|session| (session.id, session.qpc_frequency))
        .collect();
    let mut throughput = Throughput::default();
    for control in controls {
        match control {
            Some(ControlRecord::ClockWitness {
                session,
                qpc,
                hour,
                load_signal,
            }) => {
                throughput.witness(
                    *session,
                    frequencies.get(session).copied().unwrap_or(0),
                    *qpc,
                    *hour,
                    *load_signal,
                );
            }
            Some(
                ControlRecord::Detach { session, .. }
                | ControlRecord::GapSummary { session, .. }
                | ControlRecord::Partial { session, .. }
                | ControlRecord::EventLoss { session, .. },
            ) => {
                throughput.previous.remove(session);
            }
            Some(ControlRecord::PlayerCommands { .. }) => {}
            None => throughput.previous.clear(),
        }
    }
    if throughput.seconds > 0.0 {
        println!(
            "consecutive_witness_hours={} witness_seconds={:.3} game_hours_per_second={:.3}",
            throughput.hours,
            throughput.seconds,
            throughput.hours as f64 / throughput.seconds
        );
    } else {
        println!(
            "game_hours_per_second=unavailable (no consecutive clock witnesses with a valid frequency)"
        );
    }
}
