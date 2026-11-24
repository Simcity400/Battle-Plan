
use super::{
    Admissions, Naming, Result, block_settings, failure, read_file, revision_json, revision_of,
    session_admissions, write_json,
};
use crate::recording_name::{self, Contents};
use replay_recording::{records::RecoveryNote, columnar_frames::{Open, is_not_saved}};
use replay::campaign::{Replay, Revision, Summary, live::{Admit, Applied, Asked, Committed, FactsFile, Host, LiveWriter, Progress}, writer};
use replay_journal::columnar_journal::{Journal, Source};
use replay_settings::{publication, recorder_settings::RecorderSettings};
use replay_files::recording_files::{self, NamesLock};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::{
    BranchId, Hour, format::FormatVersion, identity::Identity, identity::is_campaign_uuid,
    rows::TableId, scope::CaptureMode,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const RETRY: Duration = Duration::from_secs(10);

pub const WRITING_FILE: &str = "writing.json";
const STATE_FILE: &str = "recording.state";
pub const LOADS_FILE: &str = "loads.json";
pub const RESTARTS_FILE: &str = "recorder-restarts.json";
const RESTART_NOTE: &str = "the recorder restarted: ";

pub fn is_restart(note: &RecoveryNote) -> bool {
    note.reason.starts_with(RESTART_NOTE)
}

fn restart_reasons(session_dir: &Path, campaign: &str) -> Vec<String> {
    let path = session_dir.join(RESTARTS_FILE);
    let restarts = match read_file(&path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Array(restarts)) => restarts,
            Ok(_) => Vec::new(),
            Err(error) => {
                eprintln!("{}", failure(&path, format!("the recorder restarts are not noted: {error}")));
                Vec::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            eprintln!("{}", failure(&path, format!("the recorder restarts are not noted: {error}")));
            Vec::new()
        }
    };
    restarts
        .iter()
        .filter(|restart| restart["campaign"] == campaign)
        .filter_map(|restart| {
            let recorder = restart["recorder_pid"].as_u64()?;
            let stopped = restart["stopped_pid"]
                .as_u64()
                .map_or_else(|| "a recorder".to_owned(), |pid| format!("recorder {pid}"));
            let at = restart["after_hour"]
                .as_i64()
                .map_or_else(|| "before its first hour was written".to_owned(), |hour| format!("after hour {hour}"));
            Some(format!(
                "{RESTART_NOTE}{stopped} stopped while the game ran and recorder {recorder} took the session up {at}; the collector's journal held every hour meanwhile"
            ))
        })
        .collect()
}

fn note_restarts(session_dir: &Path, campaign: &str, writer: &mut LiveWriter) -> Result<()> {
    if writer.finished() {
        return Ok(());
    }
    let reasons = restart_reasons(session_dir, campaign);
    if reasons.is_empty() {
        return Ok(());
    }
    let held = match writer.created() {
        true => Replay::open_newest(writer.path()).map_err(|error| failure(writer.path(), error))?.recovery_notes,
        false => Vec::new(),
    };
    let session = read_file(session_dir.join("bin/collector.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|config| config["session_id"].as_u64());
    for reason in reasons {
        if held.iter().any(|note| note.reason == reason) {
            continue;
        }
        let note = RecoveryNote {
            session,
            segment: writer.segments(),
            valid_bytes: 0,
            reason,
        };
        writer.note(note).map_err(|error| failure(writer.path(), error))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Writing {
    before: Option<Revision>,
    saved: Option<Revision>,
    discarding: bool,
}

fn read_writing(campaign_dir: &Path) -> Result<Option<Writing>> {
    let path = campaign_dir.join(WRITING_FILE);
    let bytes = match read_file(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        bytes => bytes.map_err(|error| failure(&path, error))?,
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| failure(&path, error))?;
    let revision = |key: &str| match value.get(key) {
        Some(Value::Null) => Some(None),
        Some(revision) => revision_of(revision).map(Some),
        None => None,
    };
    match (revision("before"), revision("saved"), value["discarding"].as_bool()) {
        (Some(before), Some(saved), Some(discarding)) => Ok(Some(Writing {
            before,
            saved,
            discarding,
        })),
        _ => Err(failure(
            &path,
            "invalid note of the recording being written; journal retained",
        )),
    }
}

fn write_writing(campaign_dir: &Path, writing: Writing) -> Result<()> {
    write_json(
        &campaign_dir.join(WRITING_FILE),
        &json!({
            "before": writing.before.map(revision_json),
            "saved": writing.saved.map(revision_json),
            "discarding": writing.discarding,
        }),
    )
}

fn source_of(session_dir: &Path) -> Result<String> {
    session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| failure(session_dir, "session folder has no name"))
}

pub fn is_unsaved_recording(recording: &Path) -> bool {
    recording.parent()
        == Some(&publication::unsaved_folder(
            recording
                .parent()
                .and_then(Path::parent)
                .unwrap_or(Path::new("")),
        ))
}

pub fn locate(replay_dir: &Path, campaign: &str) -> Result<Option<PathBuf>> {
    let unsaved = recording_files::unsaved_recording(replay_dir, campaign.as_bytes())
        .map_err(|error| failure(replay_dir, error))?;
    let saved = recording_files::recording_of(replay_dir, campaign.as_bytes())
        .map_err(|error| failure(replay_dir, error))?;
    match (unsaved, saved) {
        (Some(unsaved), Some(saved)) => Err(failure(
            &saved,
            format!(
                "a recording of this campaign is here while hours of it wait in {}; move one of them away",
                unsaved.display()
            ),
        )),
        (unsaved, saved) => Ok(unsaved.or(saved)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Load {
    pub qpc: u64,
    pub campaign: String,
    pub load: i64,
}

pub fn session_loads(session_dir: &Path) -> Vec<Load> {
    let Ok(bytes) = read_file(session_dir.join(LOADS_FILE)) else {
        return Vec::new();
    };
    let Ok(Value::Array(entries)) = serde_json::from_slice(&bytes) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            Some(Load {
                qpc: entry["qpc"].as_u64()?,
                campaign: entry["campaign"].as_str()?.to_owned(),
                load: entry["load"].as_i64()?,
            })
        })
        .collect()
}

pub fn recorded_before_load(session_dir: &Path, campaign: &str, load: i64) -> bool {
    session_loads(session_dir)
        .iter()
        .any(|seen| seen.campaign == campaign && seen.load != load)
}

pub fn first_loaded_campaign(session_dir: &Path) -> Option<String> {
    session_loads(session_dir)
        .into_iter()
        .filter(|load| is_campaign_uuid(load.campaign.as_bytes()))
        .min_by_key(|load| load.qpc)
        .map(|load| load.campaign)
}

pub fn journal_campaigns(session_dir: &Path) -> Result<Vec<String>> {
    let mut campaigns = Vec::new();
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| failure(&path, error))?
            .is_dir()
        {
            continue;
        }
        let held =
            replay_journal::columnar_journal::segments(&path).map_err(|error| failure(&path, error))?;
        if !held.is_empty()
            && let Some(name) = entry.file_name().to_str()
        {
            campaigns.push(name.to_owned());
        }
    }
    campaigns.sort();
    Ok(campaigns)
}

pub fn written_campaigns(session_dir: &Path) -> Result<Vec<String>> {
    let mut campaigns = journal_campaigns(session_dir)?;
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if entry.path().join(WRITING_FILE).is_file()
            && let Some(name) = entry.file_name().to_str()
            && !campaigns.iter().any(|known| known == name)
        {
            campaigns.push(name.to_owned());
        }
    }
    campaigns.sort();
    Ok(campaigns)
}

fn trim_spool(campaign_dir: &Path, all: bool) {
    let Ok(mut segments) = super::spool_segments(campaign_dir) else {
        return;
    };
    if !all {
        segments.pop();
    }
    for path in segments {
        let _ = fs::remove_file(path);
    }
}

#[derive(Default)]
struct Observed {
    unsaved: u64,
    unsaved_span: Option<(Hour, Hour)>,
    span: Option<(Hour, Hour)>,
    multiplayer: bool,
    roster_lost: bool,
    latest: Option<(BranchId, Hour)>,
}

fn widen(span: &mut Option<(Hour, Hour)>, hour: Hour) {
    *span = Some(span.map_or((hour, hour), |(first, last)| {
        (first.min(hour), last.max(hour))
    }));
}

impl Observed {
    fn hour(&mut self, hour: &Applied<'_>) {
        self.unsaved += 1;
        widen(&mut self.unsaved_span, hour.hour);
        widen(&mut self.span, hour.hour);
        self.latest = Some((hour.branch, hour.hour));
        match roster(hour.store) {
            Some(multiplayer) => self.multiplayer |= multiplayer,
            None => self.roster_lost = true,
        }
    }
}

fn roster(store: &Store) -> Option<bool> {
    let players = store.view(TableId::Players.index());
    let count = players.live().count();
    match players.status() {
        Status::Complete => Some(count > 1),
        Status::Truncated { .. } if count > 1 => Some(true),
        _ => None,
    }
}

struct SessionHost<'a> {
    session_dir: &'a Path,
    campaign: &'a str,
    saves: bool,
    admissions: &'a Option<Admissions>,
    last: bool,
    observed: &'a mut Observed,
    loads: &'a mut Vec<Load>,
    omitted: &'a mut Vec<(Option<i64>, u64)>,
    committed: &'a mut Vec<Committed>,
    facts_refused: bool,
    facts_packing: bool,
}

impl Host for SessionHost<'_> {
    fn admit(&mut self, asked: &Asked) -> Admit {
        if let (Some(load), Some(qpc)) = (asked.load, asked.qpc)
            && !self
                .loads
                .iter()
                .any(|seen| seen.campaign == self.campaign && seen.load == load)
        {
            self.loads.push(Load {
                qpc,
                campaign: self.campaign.to_owned(),
                load,
            });
            let listed: Vec<Value> = self
                .loads
                .iter()
                .map(|load| json!({"qpc": load.qpc, "campaign": load.campaign, "load": load.load}))
                .collect();
            let _ = write_json(&self.session_dir.join(LOADS_FILE), &Value::Array(listed));
        }
        let Some(admissions) = self.admissions else {
            return Admit::Write;
        };
        match asked.load {
            Some(load) if admitted(admissions, self.campaign, load) => Admit::Write,
            Some(load)
                if admissions
                    .refused
                    .contains_key(&(self.campaign.to_owned(), load)) =>
            {
                Admit::Skip
            }
            Some(_) if self.last => Admit::Skip,
            None if asked.last || (self.last && asked.hour) => Admit::Skip,
            _ => Admit::Wait,
        }
    }

    fn saves(&mut self) -> bool {
        self.saves
    }

    fn facts_file(&mut self, identity: &Identity) -> Option<FactsFile> {
        if self.facts_refused {
            return None;
        }
        let mut file = fs::File::open(self.session_dir.join(replay::campaign::SESSION_FACTS_FILE)).ok()?;
        let place = replay::campaign::session_facts_place(&mut file, identity).ok().flatten();
        self.facts_refused = place.is_none();
        place.map(|(offset, length)| FactsFile { file, offset, length })
    }

    fn facts_packing(&mut self) -> bool {
        self.facts_packing
    }

    fn hour(&mut self, hour: &Applied<'_>) {
        self.observed.hour(hour);
    }

    fn skipped(&mut self, asked: &Asked, hours: u64) {
        match self
            .omitted
            .iter_mut()
            .find(|(load, _)| *load == asked.load)
        {
            Some((_, total)) => *total += hours,
            None => self.omitted.push((asked.load, hours)),
        }
    }

    fn committed(&mut self, committed: &Committed) {
        if committed.saved {
            self.observed.unsaved = 0;
            self.observed.unsaved_span = None;
        }
        self.committed.push(*committed);
    }
}

fn admitted(admissions: &Admissions, campaign: &str, load: i64) -> bool {
    admissions
        .loads
        .get(campaign)
        .is_some_and(|loads| loads.contains(&load))
}

#[derive(Default)]
struct Campaign {
    writer: Option<(u32, LiveWriter, Journal)>,
    done: BTreeSet<u32>,
    observed: Observed,
    waiting: Option<Asked>,
    omitted: Vec<(Option<i64>, u64)>,
    committed: Vec<Committed>,
    error: Option<(String, Instant)>,
    saving: bool,
    base: Option<Option<Contents>>,
    capture_mode: Option<CaptureMode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reached {
    Journal,
    Paused,
    Undecided,
}

impl Campaign {
    fn advance(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        campaign: &str,
        loads: &mut Vec<Load>,
        saves: bool,
        ended: bool,
        released: bool,
        facts_packing: bool,
        stop: &dyn Fn() -> bool,
    ) -> Result<Reached> {
        let campaign_dir = session_dir.join(campaign);
        loop {
            if self.writer.is_none() {
                let captures = Journal::captures(&campaign_dir)
                    .map_err(|error| failure(&campaign_dir, error))?;
                let Some(capture) = captures
                    .into_iter()
                    .find(|capture| !self.done.contains(capture))
                else {
                    return Ok(Reached::Journal);
                };
                self.open(session_dir, replay_dir, campaign, capture)?;
            }
            let admissions = session_admissions(session_dir)?;
            let (capture, writer, journal) = self.writer.as_mut().expect("opened");
            let capture = *capture;
            self.capture_mode = writer.capture_mode().or(self.capture_mode);
            let mut host = SessionHost {
                session_dir,
                campaign,
                saves,
                admissions: &admissions,
                last: ended,
                observed: &mut self.observed,
                loads,
                omitted: &mut self.omitted,
                committed: &mut self.committed,
                facts_refused: false,
                facts_packing,
            };
            let path = writer.path().to_owned();
            let at = |error| failure(&path, error);
            let progress = if writer.finished() {
                journal.finish().map_err(at)?;
                Progress::Finished
            } else {
                writer.advance(journal, &mut host, stop).map_err(at)?
            };
            self.waiting = None;
            match progress {
                Progress::Finished => {}
                Progress::Paused => return Ok(Reached::Paused),
                Progress::AwaitsFacts => return Ok(Reached::Journal),
                Progress::Undecided(asked) => {
                    self.waiting = Some(asked);
                    return Ok(Reached::Undecided);
                }
                Progress::Waiting => {
                    let later = Journal::captures(&campaign_dir)
                        .map_err(|error| failure(&campaign_dir, error))?
                        .into_iter()
                        .any(|other| other > capture);
                    if !ended && !released && !later {
                        return Ok(Reached::Journal);
                    }
                    if !later
                        && journal
                            .producing()
                            .map_err(|error| failure(&campaign_dir, error))?
                    {
                        if !ended {
                            return Ok(Reached::Journal);
                        }
                        return Err(failure(
                            &campaign_dir,
                            "its journal is held open, so this ended session's hours wait for it; journal retained",
                        ));
                    }
                    match writer.advance(journal, &mut host, stop).map_err(at)? {
                        Progress::Finished => {}
                        Progress::Paused => return Ok(Reached::Paused),
                        Progress::AwaitsFacts => return Ok(Reached::Journal),
                        Progress::Undecided(asked) => {
                            self.waiting = Some(asked);
                            return Ok(Reached::Undecided);
                        }
                        Progress::Waiting => writer
                            .finish_stopped(
                                journal,
                                &mut host,
                                "the collector ended before finishing this file",
                            )
                            .map_err(at)?,
                    }
                }
            }
            self.done.insert(capture);
            self.writer = None;
        }
    }

    fn open(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        campaign: &str,
        capture: u32,
    ) -> Result<()> {
        let campaign_dir = session_dir.join(campaign);
        let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
        finish_discard(&campaign_dir, replay_dir, campaign)?;
        let located = locate(replay_dir, campaign)?;
        let path = match &located {
            Some(path) => path.clone(),
            None => {
                publication::create_unsaved_folder(replay_dir)
                    .map_err(|error| failure(replay_dir, error))?;
                publication::unsaved_path(replay_dir, campaign)
            }
        };
        let state_path = campaign_dir.join(STATE_FILE);
        let source = source_of(session_dir)?;
        let open = || {
            LiveWriter::open_with(
                &path,
                &state_path,
                &source,
                capture,
                block_settings(),
                Vec::new(),
                recorder_derive::writers(),
            )
        };
        let writing = read_writing(&campaign_dir)?;
        let opened = match open() {
            Err(error)
                if error.kind() == std::io::ErrorKind::InvalidData
                    && is_unsaved_recording(&path)
                    && writing.is_some_and(|writing| writing.before.is_none())
                    && no_revision(&path)
                    && replay_journal::columnar_journal::segments(&campaign_dir).is_ok_and(|held| {
                        held.iter()
                            .any(|(held, segment, _)| *held == capture && *segment == 0)
                    }) =>
            {
                fs::remove_file(&path).map_err(|error| failure(&path, error))?;
                open()
            }
            opened => opened,
        };
        let mut writer = opened.map_err(|error| failure(&path, error))?;
        if writing.is_none() {
            let before = match &located {
                Some(path) => Some(
                    Replay::open_newest(path)
                        .map_err(|error| failure(path, error))?
                        .revision(),
                ),
                None => None,
            };
            write_writing(
                &campaign_dir,
                Writing {
                    before,
                    saved: None,
                    discarding: false,
                },
            )?;
        }
        note_restarts(session_dir, campaign, &mut writer)?;
        let mut journal = Journal::open(&campaign_dir, capture);
        journal
            .start(writer.segments())
            .map_err(|error| failure(&campaign_dir, error))?;
        self.writer = Some((capture, writer, journal));
        Ok(())
    }

    fn path(&self) -> Option<&Path> {
        self.writer.as_ref().map(|(_, writer, _)| writer.path())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unsaved {
    pub hours: u64,
    pub first_hour: Option<Hour>,
    pub last_hour: Option<Hour>,
    pub bytes: u64,
    pub undecided: bool,
}

#[derive(Default)]
pub struct Live {
    campaigns: BTreeMap<String, Box<Campaign>>,
    loads: Vec<Load>,
    facts_packing: Option<Arc<AtomicBool>>,
}

impl Live {
    pub fn wait_for_facts(&mut self, packing: Arc<AtomicBool>) {
        self.facts_packing = Some(packing);
    }

    pub fn advance(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        saves: &dyn Fn(&str) -> bool,
        naming: Naming<'_>,
        stop: &dyn Fn() -> bool,
    ) -> Result<bool> {
        if self.loads.is_empty() {
            self.loads = session_loads(session_dir);
        }
        let mut grew = false;
        for name in journal_campaigns(session_dir)? {
            if stop() {
                break;
            }
            let campaign = self.campaigns.entry(name.clone()).or_default();
            if campaign
                .error
                .as_ref()
                .is_some_and(|(_, retry)| Instant::now() < *retry)
            {
                continue;
            }
            let failed = campaign.error.take().map(|(error, _)| error);
            let before = campaign.observed.span.map(|_| campaign.observed.latest);
            let saves = campaign.saving || saves(&name);
            let packing = self.facts_packing.as_ref().is_some_and(|packing| packing.load(Ordering::Acquire));
            let reached = campaign.advance(
                session_dir,
                replay_dir,
                &name,
                &mut self.loads,
                saves,
                false,
                false,
                packing,
                stop,
            );
            grew |= before != campaign.observed.span.map(|_| campaign.observed.latest);
            match reached {
                Ok(_) => {
                    place_saved(campaign, replay_dir, &name, naming);
                    if !campaign.committed.is_empty() {
                        trim_spool(&session_dir.join(&name), false);
                        campaign.committed.clear();
                    }
                }
                Err(error) => {
                    if failed.as_ref() != Some(&error) {
                        eprintln!(
                            "Campaign {name} is not being written; it is tried again: {error}"
                        );
                    }
                    campaign.writer = None;
                    campaign.error = Some((error, Instant::now() + RETRY));
                }
            }
        }
        Ok(grew)
    }

    pub fn undecided(&self) -> Vec<Load> {
        let mut waiting: Vec<Load> = self
            .campaigns
            .iter()
            .filter_map(|(name, campaign)| {
                let asked = campaign.waiting?;
                Some(Load {
                    qpc: asked.qpc?,
                    campaign: name.clone(),
                    load: asked.load?,
                })
            })
            .collect();
        waiting.sort_by_key(|load| load.qpc);
        waiting
    }

    pub fn unsaved(&self, campaign: &str) -> Option<Unsaved> {
        let held = self.campaigns.get(campaign)?;
        let undecided = held.waiting.is_some();
        if held.observed.unsaved == 0 && !undecided {
            return None;
        }
        Some(Unsaved {
            hours: held.observed.unsaved,
            first_hour: held.observed.unsaved_span.map(|(first, _)| first),
            last_hour: held.observed.unsaved_span.map(|(_, last)| last),
            bytes: waiting_bytes(held.path()),
            undecided,
        })
    }

    pub fn unsaved_campaigns(&self) -> Vec<String> {
        self.campaigns
            .keys()
            .filter(|campaign| self.unsaved(campaign).is_some())
            .cloned()
            .collect()
    }

    pub fn retry(&mut self) {
        for campaign in self.campaigns.values_mut() {
            campaign.error = None;
        }
    }

    pub fn names(&mut self, replay_dir: &Path, naming: Naming<'_>) -> BTreeMap<String, String> {
        let mut names = BTreeMap::new();
        for (name, campaign) in &mut self.campaigns {
            if campaign.waiting.is_some() || campaign.error.is_some() {
                continue;
            }
            if let Some(file) = predicted(campaign, replay_dir, name, naming) {
                names.insert(name.clone(), file);
            }
        }
        names
    }

    pub fn save(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        campaign: &str,
        naming: Naming<'_>,
    ) -> Result<Option<PathBuf>> {
        let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
        finish_discard(&session_dir.join(campaign), replay_dir, campaign)?;
        let held = self.campaigns.entry(campaign.to_owned()).or_default();
        if let Some((error, _)) = &held.error {
            return Err(format!(
                "{error}; its hours are not written yet, so nothing was saved"
            ));
        }
        if held.writer.is_some() {
            let waits = |why: &str| Err(failure(&session_dir.join(campaign), why));
            match held.advance(
                session_dir,
                replay_dir,
                campaign,
                &mut self.loads,
                true,
                false,
                true,
                false,
                &|| false,
            )? {
                Reached::Undecided => {
                    return waits(
                        "a load of it is not decided; it can be saved once recording stops",
                    );
                }
                Reached::Paused => {
                    return waits("its hours are still being written; save it again");
                }
                Reached::Journal if held.writer.is_some() => {
                    held.saving = true;
                    return waits(
                        "the game still holds its last hours; it is saved when recording stops",
                    );
                }
                Reached::Journal => {}
            }
        }
        let saved = save_locked(replay_dir, campaign, naming)?;
        if let Some((path, _)) = &saved {
            note_saved(&session_dir.join(campaign), path)?;
        }
        held.observed.unsaved = 0;
        held.observed.unsaved_span = None;
        Ok(saved.map(|(path, unrenamed)| {
            if let Some(unrenamed) = unrenamed {
                println!("{unrenamed}");
            }
            path
        }))
    }

    pub fn forget_save(&mut self, campaign: &str) {
        if let Some(held) = self.campaigns.get_mut(campaign) {
            held.saving = false;
        }
    }

    pub fn discard(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        campaign: &str,
        dropped: Discarded,
    ) -> Result<()> {
        if let Some(held) = self.campaigns.get_mut(campaign) {
            held.writer = None;
        }
        let campaign_dir = session_dir.join(campaign);
        if producing(&campaign_dir)? {
            return Err(failure(
                &campaign_dir,
                "the game still holds it; discard it once recording stops",
            ));
        }
        self.campaigns.remove(campaign);
        let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
        note_discard_locked(&campaign_dir, replay_dir, campaign, dropped)?;
        finish_discard(&campaign_dir, replay_dir, campaign)
    }
}

fn producing(campaign_dir: &Path) -> Result<bool> {
    let newest = Journal::captures(campaign_dir)
        .map_err(|error| failure(campaign_dir, error))?
        .into_iter()
        .next_back();
    match newest {
        Some(capture) => Journal::open(campaign_dir, capture)
            .producing()
            .map_err(|error| failure(campaign_dir, error)),
        None => Ok(false),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discarded {
    Unsaved,
    Session,
}

fn waiting_bytes(path: Option<&Path>) -> u64 {
    let Some(path) = path else {
        return 0;
    };
    let Ok(newest) = newest_log(path) else {
        return 0;
    };
    newest.revision().length - newest.saved().map_or(0, |saved| saved.length)
}

fn predicted(
    held: &mut Campaign,
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Option<String> {
    let (_, writer, _) = held.writer.as_ref()?;
    let existing = locate(replay_dir, campaign).ok()?;
    let base = held.base.get_or_insert_with(|| {
        let replay = Replay::open_newest(existing.as_deref()?).ok()?;
        Contents::for_prediction(&replay).ok()
    });
    let added = match (held.observed.span, held.observed.latest) {
        (Some(span), Some((branch, hour))) => {
            if held.observed.roster_lost && !held.observed.multiplayer {
                return None;
            }
            let mode = held.capture_mode?;
            let store = writer.store();
            let frame = replay_frame::frame::stored_frame(
                recording_name::name_tables().iter().map(|&table| (table, store.view(table.index()))),
                (branch, hour),
                mode,
                FormatVersion::CURRENT.version(),
            )
            .ok()?;
            Some(Contents::recording(span, held.observed.multiplayer, &frame))
        }
        _ => None,
    };
    let contents = match (base.clone(), added) {
        (Some(base), Some(added)) => base.then(added),
        (base, added) => base.or(added)?,
    };
    name_of(&contents, replay_dir, campaign, existing, naming)
}

fn name_of(
    contents: &Contents,
    replay_dir: &Path,
    campaign: &str,
    existing: Option<PathBuf>,
    naming: Naming<'_>,
) -> Option<String> {
    let predicted = contents.predicted(naming.localisation)?;
    if let Some(report) = naming.report {
        report(predicted.by_tag.as_deref());
    }
    let stem = predicted.stem();
    let folder = fs::canonicalize(replay_dir).ok()?;
    let settings = RecorderSettings::load_from(naming.settings).ok()?;
    let saved = existing.filter(|path| !is_unsaved_recording(path));
    let path = match saved {
        Some(saved)
            if super::keeps_name(&saved, &stem, &settings, &folder, campaign.as_bytes()) =>
        {
            saved
        }
        saved => recording_files::unique_path(replay_dir, &stem, saved.as_deref()).ok()?,
    };
    path.file_stem()?.to_str().map(str::to_owned)
}

pub fn waiting_name(replay_dir: &Path, campaign: &str, naming: Naming<'_>) -> Option<String> {
    let path = locate(replay_dir, campaign).ok()??;
    let newest = Replay::open_newest(&path).ok()?;
    let contents = Contents::for_prediction(&newest).ok()?;
    name_of(&contents, replay_dir, campaign, Some(path), naming)
}

fn place_saved(held: &mut Campaign, replay_dir: &Path, campaign: &str, naming: Naming<'_>) {
    let Some((_, writer, _)) = &mut held.writer else {
        return;
    };
    if !is_unsaved_recording(writer.path()) || writer.saved().is_none() {
        return;
    }
    let placed = NamesLock::wait(replay_dir)
        .map_err(|error| failure(replay_dir, error))
        .and_then(|_lock| place(replay_dir, campaign, writer.path(), naming));
    match placed {
        Ok((path, _)) => writer.moved(&path),
        Err(error) => eprintln!("Campaign {campaign} keeps its place until its next save: {error}"),
    }
}

fn place(
    replay_dir: &Path,
    campaign: &str,
    path: &Path,
    naming: Naming<'_>,
) -> Result<(PathBuf, Option<String>)> {
    let replay = Replay::open_path(path).map_err(|error| failure(path, error))?;
    let multiplayer = super::roster(&replay, path)?;
    let stem = super::recording_stem(&replay, naming, multiplayer)
        .map_err(|error| failure(path, error))?;
    drop(replay);
    let folder = fs::canonicalize(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let settings = RecorderSettings::load_from(naming.settings)
        .map_err(|error| failure(naming.settings, error))?;
    if is_unsaved_recording(path) {
        if settings.name_chosen(&folder, campaign.as_bytes()) {
            drop(
                RecorderSettings::update_at(naming.settings, |settings| settings.choose_name(&folder, campaign, false))
                    .map_err(|error| failure(naming.settings, error))?
                    .logged("The recording’s chosen name"),
            );
        }
        let placed = recording_files::place_new(path, replay_dir, &stem)
            .map_err(|error| failure(path, error))?;
        return Ok((placed, None));
    }
    if super::keeps_name(path, &stem, &settings, &folder, campaign.as_bytes()) {
        return Ok((path.to_owned(), None));
    }
    Ok(match recording_files::rename(path, &stem) {
        Ok(renamed) => (renamed, None),
        Err(error) => (
            path.to_owned(),
            Some(format!(
                "{}: saved, but not renamed to {stem} ({error}); the next save tries again",
                path.display()
            )),
        ),
    })
}

fn save_locked(
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Result<Option<(PathBuf, Option<String>)>> {
    let Some(path) = locate(replay_dir, campaign)? else {
        return Ok(None);
    };
    let newest = newest_log(&path).map_err(|error| failure(&path, error))?;
    if !newest.root().identity.campaign.eq_ignore_ascii_case(campaign.as_bytes()) {
        return Err(failure(
            &path,
            "existing replay belongs to a different campaign",
        ));
    }
    if newest.saved() != Some(newest.revision()) {
        writer::revise(
            &path,
            &newest,
            &writer::Addition::default(),
            true,
            block_settings().level,
        )
        .map_err(|error| failure(&path, format!("{error}; nothing saved")))?;
    }
    drop(newest);
    place(replay_dir, campaign, &path, naming).map(Some)
}

pub fn save_waiting(
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Result<Option<PathBuf>> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    Ok(
        save_locked(replay_dir, campaign, naming)?.map(|(path, unrenamed)| {
            if let Some(unrenamed) = unrenamed {
                println!("{unrenamed}");
            }
            path
        }),
    )
}

pub fn discard_waiting(replay_dir: &Path, campaign: &str) -> Result<()> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let Some(path) =
        locate(replay_dir, campaign).map_err(|error| format!("{error}; nothing discarded"))?
    else {
        return Ok(());
    };
    cut(&path, campaign, Keep::Saved)
}

#[derive(Clone, Copy)]
enum Keep {
    Saved,
    Session(Writing),
}

fn cut(path: &Path, campaign: &str, keep: Keep) -> Result<()> {
    if is_unsaved_recording(path) && no_revision(path) {
        return publication::posix_delete(path).map_err(|error| failure(path, error));
    }
    let newest = newest_log(path).map_err(|error| failure(path, error))?;
    if !newest.root().identity.campaign.eq_ignore_ascii_case(campaign.as_bytes()) {
        return Err(failure(
            path,
            "the recording belongs to a different campaign; nothing discarded",
        ));
    }
    let held = |revision: Option<Revision>| {
        revision.filter(|revision| writer::holds_revision(path, *revision).unwrap_or(false))
    };
    let kept = match keep {
        Keep::Saved => newest.saved(),
        Keep::Session(writing) => {
            let lost = writing.before.is_some() && held(writing.before).is_none();
            [held(writing.before), held(writing.saved), newest.saved().filter(|_| lost)]
                .into_iter()
                .flatten()
                .max_by_key(|revision| revision.number)
        }
    };
    let at = newest.revision();
    drop(newest);
    match kept {
        None => publication::posix_delete(path).map_err(|error| failure(path, error)),
        Some(kept) if kept == at => Ok(()),
        Some(kept) => writer::cut_back(path, kept).map_err(|error| failure(path, error)),
    }
}

pub fn note_discard(session_dir: &Path, replay_dir: &Path, campaign: &str) -> Result<()> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    note_discard_locked(&session_dir.join(campaign), replay_dir, campaign, Discarded::Session)
}

fn note_discard_locked(
    campaign_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    dropped: Discarded,
) -> Result<()> {
    let newest = || -> Result<Option<replay_recording::columnar_frames::Recording>> {
        locate(replay_dir, campaign)?
            .map(|path| newest_log(&path).map_err(|error| failure(&path, error)))
            .transpose()
    };
    let writing = match read_writing(campaign_dir)? {
        Some(writing) => writing,
        None if campaign_dir.is_dir() => Writing {
            before: newest()?.map(|newest| newest.revision()),
            saved: None,
            discarding: false,
        },
        None => return Ok(()),
    };
    let saved = match dropped {
        Discarded::Session => writing.saved,
        Discarded::Unsaved => latest(writing.saved, newest()?.and_then(|newest| newest.saved())),
    };
    write_writing(
        campaign_dir,
        Writing {
            saved,
            discarding: true,
            ..writing
        },
    )
}

fn latest(one: Option<Revision>, other: Option<Revision>) -> Option<Revision> {
    [one, other].into_iter().flatten().max_by_key(|revision| revision.number)
}

pub fn noted_discards(session_dir: &Path) -> Result<Vec<String>> {
    let mut campaigns = Vec::new();
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if entry.path().join(WRITING_FILE).is_file()
            && let Some(campaign) = entry.file_name().to_str()
            && read_writing(&entry.path())?.is_some_and(|writing| writing.discarding)
        {
            campaigns.push(campaign.to_owned());
        }
    }
    campaigns.sort();
    Ok(campaigns)
}

fn note_saved(campaign_dir: &Path, path: &Path) -> Result<()> {
    let Some(writing) = read_writing(campaign_dir)? else {
        return Ok(());
    };
    let saved = newest_log(path).map_err(|error| failure(path, error))?.revision();
    write_writing(
        campaign_dir,
        Writing {
            saved: latest(writing.saved, Some(saved)),
            ..writing
        },
    )
}

fn finish_discard(campaign_dir: &Path, replay_dir: &Path, campaign: &str) -> Result<()> {
    let Some(writing) = read_writing(campaign_dir)?.filter(|writing| writing.discarding) else {
        return Ok(());
    };
    if let Some(path) = locate(replay_dir, campaign)? {
        cut(&path, campaign, Keep::Session(writing))?;
        if let (Some(session_dir), Ok(newest)) = (campaign_dir.parent(), newest_log(&path))
            && Some(newest.revision()) != writing.before
        {
            drop(newest);
            let _ = attach_facts(session_dir, &path);
        }
    }
    if producing(campaign_dir)? {
        return Ok(());
    }
    let gone = |path: &Path| match publication::posix_delete(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(failure(path, error)),
        _ => Ok(()),
    };
    gone(&campaign_dir.join(STATE_FILE))?;
    for (_, _, path) in replay_journal::columnar_journal::segments(campaign_dir)
        .map_err(|error| failure(campaign_dir, error))?
    {
        gone(&path)?;
    }
    for path in super::spool_segments(campaign_dir)? {
        gone(&path)?;
    }
    gone(&campaign_dir.join(WRITING_FILE))
}

pub fn reconcile_session(session_dir: &Path, replay_dir: &Path) -> Result<()> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if entry.path().join(WRITING_FILE).is_file()
            && let Some(campaign) = entry.file_name().to_str()
        {
            finish_discard(&entry.path(), replay_dir, campaign)?;
        }
    }
    Ok(())
}

pub fn unsaved(replay_dir: &Path, campaign: &str) -> Result<Option<Unsaved>> {
    let Some(path) = locate(replay_dir, campaign)? else {
        return Ok(None);
    };
    let newest = Replay::open_newest(&path).map_err(|error| failure(&path, error))?;
    if !newest.unsaved() {
        return Ok(None);
    }
    let saved: BTreeSet<(u32, u32)> = match newest.saved_revision() {
        None => BTreeSet::new(),
        Some(_) => Replay::open_path(&path)
            .map_err(|error| failure(&path, error))?
            .index
            .iter()
            .map(|block| (block.run, block.page))
            .collect(),
    };
    let mut summary = Unsaved {
        bytes: newest.revision().length - newest.saved_revision().map_or(0, |saved| saved.length),
        ..Unsaved::default()
    };
    let mut span = None;
    for block in newest
        .index
        .iter()
        .filter(|block| !saved.contains(&(block.run, block.page)))
    {
        summary.hours += (block.last_hour - block.first_hour + 1) as u64;
        widen(&mut span, block.first_hour);
        widen(&mut span, block.last_hour);
    }
    summary.first_hour = span.map(|(first, _)| first);
    summary.last_hour = span.map(|(_, last)| last);
    Ok(Some(summary))
}

pub fn unsaved_bytes(session_dir: &Path, replay_dir: &Path, campaign: &str) -> Option<u64> {
    let mut bytes = 0;
    for (_, _, path) in replay_journal::columnar_journal::segments(&session_dir.join(campaign)).ok()? {
        bytes += fs::metadata(&path).ok()?.len();
    }
    if let Some(path) = locate(replay_dir, campaign).ok()? {
        let newest = Summary::read_as(&path, Open::Newest).ok()?.revision.length;
        let saved = match Summary::read(&path) {
            Ok(saved) => saved.revision.length,
            Err(error) if is_not_saved(&error) => 0,
            Err(_) => return None,
        };
        bytes += newest - saved;
    }
    Some(bytes)
}

#[derive(Debug, Default)]
pub(super) struct Ended {
    pub recording: Option<PathBuf>,
    pub saved: bool,
    pub unrenamed: Option<String>,
    pub omitted: Vec<(Option<i64>, u64)>,
    pub hours: u64,
}

pub(super) fn end_campaign(
    live: &mut Live,
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    save: bool,
    naming: Naming<'_>,
    progress: &dyn Fn() -> std::io::Result<()>,
    after: &dyn Fn(Option<&Path>) -> Result<Option<PathBuf>>,
) -> Result<Ended> {
    let campaign_dir = session_dir.join(campaign);
    let mut held = live.campaigns.remove(campaign).unwrap_or_default();
    held.error = None;
    let save = save || held.saving;
    let before = held.committed.len();
    let stop = || progress().is_err();
    let reached = held.advance(
        session_dir,
        replay_dir,
        campaign,
        &mut live.loads,
        save,
        true,
        true,
        false,
        &stop,
    );
    progress().map_err(|error| failure(&campaign_dir, error))?;
    match reached? {
        Reached::Journal => {}
        Reached::Paused => {
            return Err(failure(
                &campaign_dir,
                "the save was interrupted; journal retained",
            ));
        }
        Reached::Undecided => {
            return Err(failure(
                &campaign_dir,
                "a load of it was never decided; journal retained",
            ));
        }
    }
    let hours = held.committed[before.min(held.committed.len())..]
        .iter()
        .map(|committed| committed.hours)
        .sum();
    let omitted = std::mem::take(&mut held.omitted);
    drop(held);
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let writing = read_writing(&campaign_dir)?;
    let located = locate(replay_dir, campaign)?;
    let created = after(located.as_deref())?;
    let mut ended = Ended {
        omitted,
        hours,
        ..Ended::default()
    };
    if let Some(path) = created.as_ref().or(located.as_ref()) {
        let revision = |path: &Path| {
            newest_log(path)
                .map(|newest| (newest.revision(), newest.saved() == Some(newest.revision())))
                .map_err(|error| failure(path, error))
        };
        let (written, _) = revision(path)?;
        let wrote =
            created.is_some() || writing.is_some_and(|writing| writing.before != Some(written));
        if wrote {
            attach_facts(session_dir, path)?;
        }
        let path = if save {
            let (path, unrenamed) = save_locked(replay_dir, campaign, naming)?
                .ok_or_else(|| failure(path, "the recording is gone"))?;
            ended.unrenamed = unrenamed;
            path
        } else {
            path.clone()
        };
        let (left, saved) = revision(&path)?;
        if wrote || left != written {
            ended.saved = saved;
            ended.recording = Some(path);
        }
    }
    trim_spool(&campaign_dir, true);
    for name in [STATE_FILE, WRITING_FILE] {
        let _ = fs::remove_file(campaign_dir.join(name));
    }
    Ok(ended)
}

pub(super) fn attach_facts(session_dir: &Path, path: &Path) -> Result<bool> {
    let facts = session_dir.join(replay::campaign::SESSION_FACTS_FILE);
    let bytes = match fs::read(&facts) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(failure(&facts, format!("{error}; the game facts are kept"))),
    };
    let newest = newest_log(path).map_err(|error| failure(path, error))?;
    if !newest.facts_extents().is_empty() {
        return Ok(true);
    }
    let identity = newest.identity().map_err(|error| failure(path, error))?;
    let Some(pack) = replay::campaign::session_facts_for(&bytes, &identity) else {
        return Ok(false);
    };
    let addition = writer::Addition {
        facts: Some(pack),
        ..writer::Addition::default()
    };
    let saved = newest.saved() == Some(newest.revision());
    writer::revise(path, &newest, &addition, saved, block_settings().level)
        .map_err(|error| failure(path, error))?;
    Ok(true)
}

fn newest_log(path: &Path) -> std::io::Result<replay_recording::columnar_frames::Recording> {
    replay_recording::columnar_frames::Recording::open_path_as(path, Open::Newest)
}

fn no_revision(path: &Path) -> bool {
    fs::File::open(path).is_ok_and(|file| replay_columnar::container::locate(&file, Open::Newest).is_err())
}

#[cfg(test)]
mod tests;
