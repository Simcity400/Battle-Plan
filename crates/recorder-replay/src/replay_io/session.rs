//! A session's campaigns, each written into its own recording while it is recorded: the
//! captures of each campaign's journal in order, through one writer at a time
//! ([`replay::campaign::live`]). The journal is all that is read of what the collector wrote.
//!
//! A campaign's recording is `<folder>/.unsaved/<campaign>.battleplan` until its first save,
//! which moves it to its name; a campaign saved before is added to where it is. What a
//! session's campaign folder keeps beside its journal is the writer's resume state and one
//! note ([`WRITING_FILE`]): the recording's revision before the session's first write, and a
//! discard under way.

use super::{
    Admissions, Naming, Result, block_settings, failure, read_file, revision_json, revision_of,
    session_admissions, write_json,
};
use crate::recording_name::{self, Contents};
use replay::{
    campaign::{
        RecoveryNote, Replay, Revision, Summary,
        live::{Admit, Applied, Asked, Committed, FactsFile, Host, LiveWriter, Progress},
        writer,
    },
    columnar_frames::{Open, is_not_saved},
    columnar_journal::{Journal, Source},
    publication,
    recorder_settings::RecorderSettings,
    recording_files::{self, NamesLock},
};
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
    time::{Duration, Instant},
};

/// How long a campaign whose writer failed waits before it is tried again.
const RETRY: Duration = Duration::from_secs(10);

/// A session's campaign folder's note of the recording it writes into.
pub const WRITING_FILE: &str = "writing.json";
/// The writer's resume state in a session's campaign folder.
const STATE_FILE: &str = "recording.state";
/// The loads a session's journals named, in its folder.
pub const LOADS_FILE: &str = "loads.json";
/// In a session folder: each recorder that took the session over, the recorder that had
/// stopped, the campaign the collector was writing and after which hour.
pub const RESTARTS_FILE: &str = "recorder-restarts.json";
/// Starts the reason of the recovery note a recording holds for each recorder restart.
const RESTART_NOTE: &str = "the recorder restarted: ";

/// Whether a recovery note names a recorder restart rather than damage.
pub fn is_restart(note: &RecoveryNote) -> bool {
    note.reason.starts_with(RESTART_NOTE)
}

/// The reason of the recovery note for each restart the session's record lists of `campaign`.
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

/// Notes in `writer`'s recording each restart of the session while it wrote `campaign` that
/// the recording does not hold yet, unless its capture is finished.
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

/// What a session's campaign folder notes of the recording it writes into, before its first
/// write: the recording's newest revision then (none when the session creates it), a later
/// revision a discard of the session's hours keeps (what the user's Save made, or what was saved
/// when the waiting list's Discard came), and whether the session's hours of it are being
/// discarded.
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

/// The session folder's name, which names its captures in the recordings they are written into.
fn source_of(session_dir: &Path) -> Result<String> {
    session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| failure(session_dir, "session folder has no name"))
}

/// Whether `recording` is a folder's recording that was never saved.
pub fn is_unsaved_recording(recording: &Path) -> bool {
    recording.parent()
        == Some(&publication::unsaved_folder(
            recording
                .parent()
                .and_then(Path::parent)
                .unwrap_or(Path::new("")),
        ))
}

/// Where `campaign`'s recording is in `replay_dir`: its name once saved, the hidden folder
/// until then, nowhere before its first block. Both at once is refused, never guessed between.
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

/// One load a session's journals named: when its first hour or command was read, by the
/// session's performance counter, its campaign and its load signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Load {
    pub qpc: u64,
    pub campaign: String,
    pub load: i64,
}

/// Every load `session_dir`'s journals named so far, each once, in the order the writers met
/// them.
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

/// Whether the session already recorded hours of `campaign` under another load than `load`.
pub fn recorded_before_load(session_dir: &Path, campaign: &str, load: i64) -> bool {
    session_loads(session_dir)
        .iter()
        .any(|seen| seen.campaign == campaign && seen.load != load)
}

/// The campaign the session loaded first: the one whose first hour or command was read
/// earliest. Every campaign's are timed by the session's one clock.
pub fn first_loaded_campaign(session_dir: &Path) -> Option<String> {
    session_loads(session_dir)
        .into_iter()
        .filter(|load| is_campaign_uuid(load.campaign.as_bytes()))
        .min_by_key(|load| load.qpc)
        .map(|load| load.campaign)
}

/// The campaign folders of a session that hold a journal segment.
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
            replay::columnar_journal::segments(&path).map_err(|error| failure(&path, error))?;
        if !held.is_empty()
            && let Some(name) = entry.file_name().to_str()
        {
            campaigns.push(name.to_owned());
        }
    }
    campaigns.sort();
    Ok(campaigns)
}

/// The campaign folders of a session that hold a journal segment or note a recording the
/// session wrote into.
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

/// Deletes the campaign folder's row spool segments the collector has closed, unread: nothing
/// of the recorder's reads them. With `all`, its newest too, once no collector writes it.
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

/// What a session has seen of the hours it applied of one campaign.
#[derive(Default)]
struct Observed {
    /// Hours applied since the recording's last saved revision, and their span.
    unsaved: u64,
    unsaved_span: Option<(Hour, Hour)>,
    /// Every hour applied: their span, whether one held more than one human, and whether a
    /// roster could not be read.
    span: Option<(Hour, Hour)>,
    multiplayer: bool,
    roster_lost: bool,
    /// The branch and hour applied last.
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

/// Whether the hour `store` holds had more than one human; none when its roster is not whole
/// enough to tell.
fn roster(store: &Store) -> Option<bool> {
    let players = store.view(TableId::Players.index());
    let count = players.live().count();
    match players.status() {
        Status::Complete => Some(count > 1),
        Status::Truncated { .. } if count > 1 => Some(true),
        _ => None,
    }
}

/// The writer's host for one campaign of a session.
struct SessionHost<'a> {
    session_dir: &'a Path,
    campaign: &'a str,
    saves: bool,
    /// The loads the session admitted and refused, as read before this look; none for the
    /// command line, which records every load.
    admissions: &'a Option<Admissions>,
    /// The session ended: a load neither admitted nor refused by now is left out.
    last: bool,
    observed: &'a mut Observed,
    loads: &'a mut Vec<Load>,
    omitted: &'a mut Vec<(Option<i64>, u64)>,
    committed: &'a mut Vec<Committed>,
    /// The facts file was read and is not the session's: the writer asks before every hour
    /// until it has facts, and a recorder writes the file whole or not at all (a take-over
    /// packs it again for the same identity).
    facts_refused: bool,
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
            // Lost, the list is written whole again with the next load.
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
            // The session ended: nobody decides a load it never asked about.
            Some(_) if self.last => Admit::Skip,
            // No load names the branch, and nothing read later will.
            None if asked.last || (self.last && asked.hour) => Admit::Skip,
            _ => Admit::Wait,
        }
    }

    fn saves(&mut self) -> bool {
        self.saves
    }

    /// The pack at its place in the session's facts file: only the file's head is read here.
    fn facts_file(&mut self, identity: &Identity) -> Option<FactsFile> {
        if self.facts_refused {
            return None;
        }
        let mut file = fs::File::open(self.session_dir.join(replay::campaign::SESSION_FACTS_FILE)).ok()?;
        let place = replay::campaign::session_facts_place(&mut file, identity).ok().flatten();
        self.facts_refused = place.is_none();
        place.map(|(offset, length)| FactsFile { file, offset, length })
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

/// One campaign of a session as it is written.
#[derive(Default)]
struct Campaign {
    /// The capture being written: its number, writer and journal.
    writer: Option<(u32, LiveWriter, Journal)>,
    /// Captures the recording holds all it keeps of.
    done: BTreeSet<u32>,
    observed: Observed,
    /// The load its writer waits on.
    waiting: Option<Asked>,
    omitted: Vec<(Option<i64>, u64)>,
    committed: Vec<Committed>,
    /// Why its writer stopped, and when it is tried again; its end tries again too.
    error: Option<(String, Instant)>,
    /// Save was asked while its collector still held its journal: every revision of it is
    /// saved from then on, and its end saves it.
    saving: bool,
    /// What names the recording as it stood when the session first wrote to it.
    base: Option<Option<Contents>>,
    capture_mode: Option<CaptureMode>,
}

/// What one look at a campaign's journal got to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// Every record made so far is taken.
    Journal,
    Paused,
    Undecided,
}

impl Campaign {
    /// Takes what the campaign's journal holds, capture after capture. With `ended`, the
    /// session is over: a load not decided by now is left out, and a capture left unfinished
    /// ends where its journal does. With `released`, a capture left unfinished ends there too
    /// once its collector no longer holds its journal, as when the game left the campaign.
    #[allow(
        clippy::too_many_arguments,
        reason = "one campaign's whole context, passed once"
    )]
    fn advance(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        campaign: &str,
        loads: &mut Vec<Load>,
        saves: bool,
        ended: bool,
        released: bool,
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
            // Read before anything is decided by it: one that cannot be read leaves nothing out.
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
                Progress::Undecided(asked) => {
                    self.waiting = Some(asked);
                    return Ok(Reached::Undecided);
                }
                Progress::Waiting => {
                    // A later capture's journal means this one's collector gave it up.
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
                    // The collector let go of it: what it wrote before that is read first.
                    match writer.advance(journal, &mut host, stop).map_err(at)? {
                        Progress::Finished => {}
                        Progress::Paused => return Ok(Reached::Paused),
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

    /// Opens capture `capture`'s writer on the campaign's recording, noting first what the
    /// recording held before the session wrote to it.
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
            LiveWriter::open(
                &path,
                &state_path,
                &source,
                capture,
                block_settings(),
                Vec::new(),
            )
        };
        let writing = read_writing(&campaign_dir)?;
        let opened = match open() {
            // A recording the session created and was killed before the first revision of:
            // nothing reads as it, and its journal holds every hour of it.
            Err(error)
                if error.kind() == std::io::ErrorKind::InvalidData
                    && is_unsaved_recording(&path)
                    && writing.is_some_and(|writing| writing.before.is_none())
                    && no_revision(&path)
                    && replay::columnar_journal::segments(&campaign_dir).is_ok_and(|held| {
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

    /// The recording's path as its writer knows it.
    fn path(&self) -> Option<&Path> {
        self.writer.as_ref().map(|(_, writer, _)| writer.path())
    }
}

/// What a campaign's unsaved hours are, for a recording waiting to be saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unsaved {
    /// Admitted hours.
    pub hours: u64,
    pub first_hour: Option<Hour>,
    pub last_hour: Option<Hour>,
    /// Bytes they take on disk.
    pub bytes: u64,
    /// A load in them was never asked about, so what a save keeps is not known yet.
    pub undecided: bool,
}

/// A session's campaigns as they are written, for its end to take over.
#[derive(Default)]
pub struct Live {
    /// Boxed: a campaign's writer holds every table.
    campaigns: BTreeMap<String, Box<Campaign>>,
    loads: Vec<Load>,
}

impl Live {
    /// Takes what each campaign's journal added, until `stop`: each block goes into its
    /// campaign's recording as it closes, saved when `saves` says its campaign saves now; a
    /// recording saved for the first time takes the name `naming` gives it. Whether an hour
    /// was taken.
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
            let reached = campaign.advance(
                session_dir,
                replay_dir,
                &name,
                &mut self.loads,
                saves,
                false,
                false,
                stop,
            );
            grew |= before != campaign.observed.span.map(|_| campaign.observed.latest);
            match reached {
                Ok(_) => {
                    // Also one a recorder saved and was killed before naming.
                    place_saved(campaign, replay_dir, &name, naming);
                    if !campaign.committed.is_empty() {
                        trim_spool(&session_dir.join(&name), false);
                        campaign.committed.clear();
                    }
                }
                // Its writer goes on from the recording's newest revision when it is tried again.
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

    /// The loads the writers wait on, earliest first: its campaign, its load signal and when
    /// its first hour was read. A load nothing names is none of them.
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

    /// The hours of a campaign the session applied and has not saved; `None` before any.
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

    /// The campaigns the session wrote hours of that are not saved.
    pub fn unsaved_campaigns(&self) -> Vec<String> {
        self.campaigns
            .keys()
            .filter(|campaign| self.unsaved(campaign).is_some())
            .cloned()
            .collect()
    }

    /// Tries the campaigns whose writers failed again at the next look.
    pub fn retry(&mut self) {
        for campaign in self.campaigns.values_mut() {
            campaign.error = None;
        }
    }

    /// The file name a save would give each campaign's recording in `replay_dir` now, by
    /// campaign: what named it before the session, with the session's hours after. One not
    /// known, or whose loads are not all decided, is left out.
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

    /// Saves everything a campaign's recording holds: a capture its collector let go of
    /// without finishing ends where its journal does, then one more revision, then its name.
    /// While a collector still holds its journal, the hours since its last block are the
    /// collector's: the save is refused, and the campaign is saved as it is written from then
    /// on and at the session's end. Returns the recording, when the campaign has one.
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
        // Its captures ended: the recording holds every hour of them.
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

    /// Discards the session's hours of a campaign (`dropped` says which), and its journal goes.
    /// The capture's later hours, should the game load the campaign again, start a recording anew.
    ///
    /// While a collector still holds the campaign's journal the discard is refused: its capture
    /// goes on in that journal should the game load the campaign again.
    pub fn discard(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        campaign: &str,
        dropped: Discarded,
    ) -> Result<()> {
        // Its own reader lets go first: the probe below cannot tell its handle from the collector's.
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

/// Whether a collector still holds the newest journal of a session's campaign folder. A reader
/// of it in this process lets go first, or the probe takes its handle for the collector's.
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

/// Which of a session's hours of a campaign a discard drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discarded {
    /// Those not saved, as the waiting list's Discard drops them: what is saved stays.
    Unsaved,
    /// Every hour the session wrote, saved as it wrote them or not, as Stop's Discard drops
    /// them: the recording is as it was before the session, or as the user's Save left it since.
    Session,
}

/// The bytes a recording holds after its newest saved revision; all of it when none is saved.
fn waiting_bytes(path: Option<&Path>) -> u64 {
    let Some(path) = path else {
        return 0;
    };
    let Ok(newest) = newest_log(path) else {
        return 0;
    };
    newest.revision().length - newest.saved().map_or(0, |saved| saved.length)
}

/// The name a save would give `campaign`'s recording now.
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

/// The file name a save gives the recording of `campaign` in `replay_dir`, now at `existing`,
/// once it holds `contents`.
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

/// The file name [`save_waiting`] would give `campaign`'s recording in `replay_dir` with the
/// hours that wait in it; none while it is not known.
pub fn waiting_name(replay_dir: &Path, campaign: &str, naming: Naming<'_>) -> Option<String> {
    let path = locate(replay_dir, campaign).ok()??;
    let newest = Replay::open_newest(&path).ok()?;
    let contents = Contents::for_prediction(&newest).ok()?;
    name_of(&contents, replay_dir, campaign, Some(path), naming)
}

/// Moves a campaign's recording to its name once the session's writer committed a saved
/// revision of it: a failure leaves it where it waits, for the next save to place.
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

/// Gives the saved recording at `path` its name in `replay_dir`: one never saved before is
/// moved out of the hidden folder under the first free name its contents give; one saved before
/// keeps its name when its contents still give it or the user chose it, and is renamed
/// otherwise, never replacing another file. Returns the recording, and why it kept a name its
/// contents no longer give. The caller holds the folder's naming lock.
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
        // A new recording is named by its contents until the user names it.
        if settings.name_chosen(&folder, campaign.as_bytes()) {
            RecorderSettings::update_at(naming.settings, |settings| {
                settings.choose_name(&folder, campaign, false)
            })
            .map_err(|error| failure(naming.settings, error))?;
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

/// Saves everything `campaign`'s recording in `replay_dir` holds, no capture writing it: one
/// more revision when hours wait in it, then its name. Returns the recording and why it kept a
/// name its contents no longer give; none when the campaign has no recording. The caller holds
/// the folder's naming lock.
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

/// Saves a recording's waiting hours: everything its file holds is saved, and it takes its
/// name. Returns the recording, none when the campaign has none in `replay_dir`.
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

/// Drops a recording's waiting hours, every session's: its file is cut back to its newest
/// saved revision, and one never saved is deleted.
pub fn discard_waiting(replay_dir: &Path, campaign: &str) -> Result<()> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let Some(path) =
        locate(replay_dir, campaign).map_err(|error| format!("{error}; nothing discarded"))?
    else {
        return Ok(());
    };
    cut(&path, campaign, Keep::Saved)
}

/// What a cut keeps of a recording.
#[derive(Clone, Copy)]
enum Keep {
    /// Its newest saved revision: every session's waiting hours go.
    Saved,
    /// What it held before a session wrote to it, or the later revision the session's note
    /// keeps: the session's other hours go, saved as they were written or not.
    Session(Writing),
}

/// Cuts the recording at `path` back to what `keep` keeps; with nothing, it is deleted. A
/// recording that is that revision already is left as it is.
fn cut(path: &Path, campaign: &str, keep: Keep) -> Result<()> {
    // No revision of it was ever committed: there is nothing of it to keep.
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
            // A revision the recording no longer holds was itself discarded since: what is saved stays.
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

/// Notes that every hour the session wrote of a campaign is discarded, as Stop's Discard drops
/// them, without discarding them yet: whatever next ends or reads the session
/// ([`reconcile_session`]) carries the discard out.
pub fn note_discard(session_dir: &Path, replay_dir: &Path, campaign: &str) -> Result<()> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    note_discard_locked(&session_dir.join(campaign), replay_dir, campaign, Discarded::Session)
}

/// [`note_discard`] of the hours `dropped` names; the caller holds the folder's naming lock.
fn note_discard_locked(
    campaign_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    dropped: Discarded,
) -> Result<()> {
    let newest = || -> Result<Option<replay::columnar_frames::Recording>> {
        locate(replay_dir, campaign)?
            .map(|path| newest_log(&path).map_err(|error| failure(&path, error)))
            .transpose()
    };
    let writing = match read_writing(campaign_dir)? {
        Some(writing) => writing,
        // The session wrote nothing of it: only its journal goes.
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

/// The campaigns whose discard the session notes and has not finished.
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

/// Notes the revision the user's Save made of the recording at `path`, which a later discard of
/// the session's hours keeps. The caller holds the folder's naming lock.
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

/// Finishes the discard a session's campaign folder notes, when it notes one: the recording
/// cut back, then the writer's resume state, the journal and the row spool gone, then the note.
/// A journal a collector still holds stays, with the note, for whatever next takes the session
/// up once it lets go. Every step repeats safely. The caller holds the folder's naming lock.
fn finish_discard(campaign_dir: &Path, replay_dir: &Path, campaign: &str) -> Result<()> {
    let Some(writing) = read_writing(campaign_dir)?.filter(|writing| writing.discarding) else {
        return Ok(());
    };
    if let Some(path) = locate(replay_dir, campaign)? {
        cut(&path, campaign, Keep::Session(writing))?;
        // Hours the user's Save kept stay, with the game facts they depend on; without them the
        // recording takes its facts from the installation when it is next opened.
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
    for (_, _, path) in replay::columnar_journal::segments(campaign_dir)
        .map_err(|error| failure(campaign_dir, error))?
    {
        gone(&path)?;
    }
    for path in super::spool_segments(campaign_dir)? {
        gone(&path)?;
    }
    gone(&campaign_dir.join(WRITING_FILE))
}

/// Finishes every discard the session's campaign folders note.
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

/// The hours `campaign`'s recording in `replay_dir` holds that are not saved, every session's;
/// `None` when none wait.
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

/// The bytes a session's unsaved hours of `campaign` take on disk: its journal not yet written,
/// and what the campaign's recording in `replay_dir` holds after its newest saved revision.
/// None when either cannot be read.
pub fn unsaved_bytes(session_dir: &Path, replay_dir: &Path, campaign: &str) -> Option<u64> {
    let mut bytes = 0;
    for (_, _, path) in replay::columnar_journal::segments(&session_dir.join(campaign)).ok()? {
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

/// What ending one campaign of a session left.
#[derive(Debug, Default)]
pub(super) struct Ended {
    /// The recording, when the session's hours or its save changed it.
    pub recording: Option<PathBuf>,
    /// Whether everything it holds is saved.
    pub saved: bool,
    pub unrenamed: Option<String>,
    pub omitted: Vec<(Option<i64>, u64)>,
    /// Hours the session's writers added while ending it.
    pub hours: u64,
}

/// Ends one campaign of a session whose collector writes no more: what its journal still
/// holds is written, a capture left unfinished ends where its journal does, and with `save`
/// everything its recording holds is saved and it takes its name. `after` adds what the
/// session's failure, when it had one, leaves to say, once the journal is written.
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
        // A recording the session added nothing to is not its to change.
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
    // Nothing of the campaign is left to write: what the session kept beside its journal goes.
    trim_spool(&campaign_dir, true);
    for name in [STATE_FILE, WRITING_FILE] {
        let _ = fs::remove_file(campaign_dir.join(name));
    }
    Ok(ended)
}

/// Gives the recording at `path` the game facts `session_dir`'s recorder gathered, when it
/// carries none and they are its content's, in a revision that leaves what is saved as it was.
/// Whether it carries them now. No capture writes the recording; the caller holds the folder's
/// naming lock.
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

/// The newest revision of the recording at `path`, read for its root and records alone.
fn newest_log(path: &Path) -> std::io::Result<replay::columnar_frames::Recording> {
    replay::columnar_frames::Recording::open_path_as(path, Open::Newest)
}

/// Whether no footer of the file at `path` names a root that verifies: no revision of it was
/// ever committed.
fn no_revision(path: &Path) -> bool {
    fs::File::open(path).is_ok_and(|file| replay_columnar::container::locate(&file, Open::Newest).is_err())
}

#[cfg(test)]
mod tests;
