//! Passive finalization and inspection use the same codecs as the collector.

use replay_data::identity::IdentityCodec;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Seek,
    path::{Path, PathBuf},
    time::Instant,
};

use replay::{
    container::BlockIndex, container::Budgeted, container::Incomplete, container::Observed,
    container::RecoveryNote, container::Replay, container::Revision, container::SeekResult,
    container::UpdateKind, spool::ControlRecord, spool::FrameMetadata, spool::Record,
    spool::RecordReader, spool::RecordRef, spool::Recovered, spool::Recovery, spool::SpoolRecord,
    spool::Tail, spool::read_frame_at, spool::read_frame_payload_at, spool::recover_reader,
};
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    BranchId, Hour, SessionId, format::FormatVersion, identity::Identity, identity::SessionStart,
    identity::Verification, identity::is_campaign_uuid, rows::TableId, rows::TableRows,
    rows::tables,
};
use replay_data::{
    witness::TerminalWitness, witness::UncommittedRange, witness::WITNESS_BYTES,
    witness::WITNESS_FILE_NAME,
};
use replay_frame::{
    branches::BranchStart, branches::BranchTracker, branches::Gap, branches::Parentage,
    frame::DecodedFrame, frame::Frame, frame::FrameDecoder, frame::TableUpdate, frame::TableView,
};

use crate::{recording_name::Contents, recording_name::SpoolName, windows};
use installed_content_localisation::localisation::Localisation;
use replay::{
    recorder_settings::RecorderSettings, recording_files, recording_files::NamesLock,
    spool::Placement,
};
use serde_json::{Value, json};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

/// Why a Battle Plan Recorder session does not record a load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The campaign does not record automatically, and Record was not pressed for it.
    Off,
    /// The load belongs in another recording: its campaign's own folder, or a new file.
    Elsewhere,
    /// The recorder settings could not be read or updated, so neither whether
    /// the campaign records nor its folder is known.
    Settings,
    /// Earlier hours of the campaign wait to be saved and could not be staged for this session to
    /// continue: its loads are left out, and other campaigns record.
    Waiting,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Elsewhere => "elsewhere",
            Self::Settings => "settings",
            Self::Waiting => "waiting",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Self::Off),
            "elsewhere" => Some(Self::Elsewhere),
            "settings" => Some(Self::Settings),
            "waiting" => Some(Self::Waiting),
            _ => None,
        }
    }
}

/// A load a Battle Plan Recorder session left out of its recordings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Omission {
    pub campaign: String,
    /// The collector's load signal; `None` for hours whose load no clock
    /// witness identified.
    pub load: Option<i64>,
    /// Hours it recorded.
    pub hours: u64,
    /// Why; `None` when it was never admitted (never asked about, or unidentified).
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

/// Run `work` packing at the persisted compression, unless a packing job already chose it.
pub fn with_compression<T>(settings: &Path, work: impl FnOnce() -> Result<T>) -> Result<T> {
    if replay::container::Packing::active() {
        return work();
    }
    let packing = replay::container::Packing::from_settings(settings)
        .map_err(|error| failure(settings, error))?;
    packing.promote();
    packing.run(work)
}

/// What finalizing one campaign published and left out.
#[derive(Debug, Default)]
struct CampaignOutcome {
    /// The saved or published recording, if any.
    replay: Option<PathBuf>,
    /// Why the recording kept a name its contents no longer give.
    unrenamed: Option<String>,
    /// Each load left out, with its load signal and recorded hours.
    omitted: Vec<(Option<i64>, u64)>,
    /// Each segment the recording now holds that the save read: its length, admitted hours and
    /// their span.
    covered: BTreeMap<u32, (u64, u64, Option<(Hour, Hour)>)>,
    incomplete: BTreeSet<u32>,
    report: Option<Box<FinalizationReport>>,
}

/// Each recording a save or checkpoint added blocks to, once they are durable and read back,
/// with the blocks it added in order: the seam for work that follows a recording as it grows.
pub type Committed<'a> = dyn FnMut(&Path, &Replay, &[BlockIndex]) + Send + 'a;

const ALLOCATION_GAP_REASON: &str = "collector allocation failed";

/// Each recovered segment through the handle it was recovered with, which no one may write or delete.
struct FinalizationFrames {
    segments: BTreeMap<u32, (Segment, usize)>,
}

enum Segment {
    Held(std::sync::Mutex<fs::File>),
    /// Still written by the collector, so opened for each read.
    Growing(PathBuf),
}

/// The file at `path` opened to read and write, shared with no one; none while another
/// handle holds it, as its collector's does until it finishes the file.
fn open_unwritten(path: &Path) -> std::io::Result<Option<fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) => Ok(None),
        Err(error) => Err(error),
    }
}

/// A segment opened so that no one else may write, truncate, replace or delete it while held.
pub fn open_held(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

/// A session's spool recovered and its blocks written while it records, for finalization to take over.
#[derive(Default)]
pub struct Ahead {
    /// Boxed with its summary: inline, map nodes and moves overflow the main thread's stack.
    campaigns: BTreeMap<PathBuf, Box<CampaignAhead>>,
    /// The game speed index the last frame read recorded.
    speed: Option<i32>,
    committed: Option<Box<Committed<'static>>>,
    // Checkpoint metrics outlive the disposable spool tails, whose table times recordings omit.
    checkpoint_reports: BTreeMap<PathBuf, Box<FinalizationReport>>,
}

/// Known work in one scheduling class, not a promise that blocked work can drain.
#[derive(Clone, Copy, Debug, Default)]
pub struct PackingDebt {
    pub work: replay::container::Outstanding,
    pub unwritten_hours: usize,
    pub closed_blocks: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PackingBacklog {
    pub runnable: PackingDebt,
    pub blocked: PackingDebt,
}

#[derive(Default)]
struct CampaignAhead {
    command_bootstrap: BTreeSet<BranchId>,
    /// Segments an earlier save of the session holds, never read again.
    saved: BTreeSet<u32>,
    tails: BTreeMap<u32, (PathBuf, Tail<Recovered>)>,
    clock: Option<FrameDecoder>,
    written: Option<(replay::container::Ahead, Box<Summary>)>,
    /// It could not go on; finalization writes every block.
    stopped: bool,
    /// Naming inputs read with the spool, independently of load decisions.
    naming: std::sync::Mutex<CampaignNaming>,
    /// A checkpoint waits for the segments through this one: later ones are left unread.
    through: Option<u32>,
}

#[derive(Default)]
struct CampaignNaming {
    spools: BTreeMap<BranchId, (SpoolName, (u32, usize))>,
}

impl CampaignNaming {
    fn observe(&mut self, segment: u32, placement: Placement, frame: &Frame) {
        let (spool, written) = self.spools.entry(frame.branch).or_default();
        if spool.last_hour().is_none_or(|last| frame.hour >= last) {
            *written = (segment, placement.offset);
        }
        spool.observe(segment, placement, frame);
    }

    fn contents(
        &mut self,
        dropped: &BTreeSet<BranchId>,
        read: impl FnMut(u32, Placement) -> std::io::Result<Frame>,
    ) -> Option<Contents> {
        let mut combined: Option<Contents> = None;
        let mut latest = None;
        for (branch, (spool, written)) in &self.spools {
            if dropped.contains(branch) {
                continue;
            }
            let summary = spool.summary()?;
            combined = Some(match combined {
                Some(known) => known.then(summary),
                None => summary,
            });
            let key = (spool.last_hour()?, *written);
            if latest.is_none_or(|(known, _)| key >= known) {
                latest = Some((key, *branch));
            }
        }
        let country = self.spools.get_mut(&latest?.1)?.0.contents(read)?;
        Some(combined?.then(country))
    }
}

impl Ahead {
    /// Reads what each campaign's spool added; whether a frame was added.
    pub fn read(&mut self, session_dir: &Path, replay_dir: &Path) -> Result<bool> {
        let mut grew = false;
        for name in spooled_campaigns(session_dir)? {
            let campaign_dir = session_dir.join(&name);
            let campaign = &mut **self
                .campaigns
                .entry(campaign_dir.clone())
                .or_insert_with(|| {
                    let sessions = spool_session(&segment_paths(&campaign_dir).unwrap_or_default());
                    let base = save_base(session_dir, replay_dir, &name, &sessions)
                        .ok()
                        .flatten();
                    Box::new(CampaignAhead {
                        saved: read_saved(&campaign_dir)
                            .filter(|saved| Some(&saved.replay) == base.as_ref())
                            .map(|saved| saved.segments.into_keys().collect())
                            .unwrap_or_default(),
                        ..CampaignAhead::default()
                    })
                });
            campaign.command_bootstrap = read_saved(&campaign_dir)
                .map(|saved| saved.command_bootstrap)
                .unwrap_or_default();
            for (number, path) in segment_paths(&campaign_dir)? {
                if campaign.saved.contains(&number)
                    || campaign.through.is_some_and(|through| number > through)
                {
                    continue;
                }
                let (_, tail) = campaign
                    .tails
                    .entry(number)
                    .or_insert_with(|| (path.clone(), Tail::default()));
                let file = fs::File::open(&path).map_err(|error| failure(&path, error))?;
                let length = file
                    .metadata()
                    .map_err(|error| failure(&path, error))?
                    .len();
                let length = usize::try_from(length).map_err(|error| failure(&path, error))?;
                let read = tail.records().len();
                let pending = std::sync::Mutex::new(None::<(Placement, Frame)>);
                let grow = tail.grow(
                    &mut std::io::BufReader::new(Budgeted::new(file)),
                    length,
                    |record, placement| {
                        let mut pending = pending
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        // grow accepts the previous record before asking keep for the next one.
                        if let Some((placement, frame)) = pending.take() {
                            campaign
                                .naming
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .observe(number, placement, &frame);
                        }
                        match record {
                            Record::Frame(frame) => {
                                let kept = Recovered::Frame(FrameMetadata {
                                    branch: frame.branch,
                                    hour: frame.hour,
                                    keyframe: frame.keyframe,
                                    placement,
                                    campaign: frame.tables[TableId::Campaign.index()].clone(),
                                    clock: frame.tables[TableId::Clock.index()].clone(),
                                });
                                *pending = Some((placement, frame));
                                kept
                            }
                            record => Recovered::keep(record, placement),
                        }
                    },
                );
                if let Some((placement, frame)) = pending
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    && matches!(tail.records().last(), Some(Recovered::Frame(kept)) if kept.placement == placement)
                {
                    campaign
                        .naming
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .observe(number, placement, &frame);
                }
                grow.map_err(|error| failure(&path, error))?;
                let clock = campaign
                    .clock
                    .get_or_insert_with(|| FrameDecoder::for_tables(&[TableId::Clock]));
                for record in &tail.records()[read..] {
                    let Recovered::Frame(metadata) = record else {
                        continue;
                    };
                    grew = true;
                    self.speed = metadata.frame().ok().and_then(|frame| {
                        match clock
                            .advance(&frame)
                            .ok()
                            .map(|hour| &*hour.tables[TableId::Clock.index()])
                        {
                            Some(TableView::Complete(TableRows::Clock(rows))) => {
                                rows.first().map(|row| row.speed)
                            }
                            _ => None,
                        }
                    });
                }
            }
        }
        Ok(grew)
    }

    /// The file name a save would give each campaign's recording in `replay_dir` now, by spool
    /// folder: its spool's hours appended to what they add to. One not known is left out.
    pub fn names(
        &self,
        session_dir: &Path,
        replay_dir: &Path,
        naming: Naming<'_>,
    ) -> BTreeMap<String, String> {
        let Ok(admissions) = session_admissions(session_dir) else {
            return BTreeMap::new();
        };
        self.campaigns
            .iter()
            .filter_map(|(directory, campaign)| {
                let name = directory.file_name()?.to_str()?.to_owned();
                // A campaign none of whose loads was admitted saves nothing.
                if admissions.as_ref().is_some_and(|admissions| {
                    admissions.loads.get(&name).is_none_or(BTreeSet::is_empty)
                }) {
                    return None;
                }
                let sessions = spool_session(&segment_paths(directory).ok()?);
                if !campaign.saved.is_empty() {
                    let base = save_base(session_dir, replay_dir, &name, &sessions).ok()??;
                    let saved = read_saved(directory)?;
                    if !Replay::open_path(&base)
                        .ok()?
                        .contains_revision(saved.revision)
                        .ok()?
                    {
                        return None;
                    }
                }
                let mut branches = BranchLoads::default();
                for (_, tail) in campaign.tails.values() {
                    for record in tail.records() {
                        branches.observe(record.view());
                    }
                }
                branches.restore_bootstrap(&campaign.command_bootstrap);
                if branches.undecided(admissions.as_ref(), &name) {
                    return None;
                }
                let dropped = admissions
                    .as_ref()
                    .map(|admissions| {
                        branches.dropped(admissions.loads.get(&name).unwrap_or(&BTreeSet::new()))
                    })
                    .unwrap_or_default();
                let read = |segment: u32, placement: Placement| {
                    let (path, tail) = campaign
                        .tails
                        .get(&segment)
                        .ok_or_else(|| std::io::Error::other("spool segment is gone"))?;
                    read_frame_at(
                        &mut Budgeted::new(fs::File::open(path)?),
                        placement,
                        tail.valid_bytes(),
                    )
                };
                let mut known = campaign
                    .naming
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let added = if known.spools.is_empty() {
                    None
                } else {
                    Some(known.contents(&dropped, read)?)
                };
                let file =
                    predicted_name(session_dir, replay_dir, &name, &sessions, added, naming)?;
                Some((name, file))
            })
            .collect()
    }

    pub fn speed(&self) -> Option<i32> {
        self.speed
    }

    /// Drop what was read of a campaign whose spool changed under it.
    pub fn forget(&mut self, campaign_dir: &Path) {
        self.campaigns.remove(campaign_dir);
    }

    /// Hand each recording a save adds blocks to, and those blocks, to `committed` from now on.
    pub fn on_committed(&mut self, committed: Box<Committed<'static>>) {
        self.committed = Some(committed);
    }

    /// The hours and spool bytes of a campaign read and not yet saved into the recording it adds
    /// to; `None` before any is read.
    pub fn unsaved(&self, campaign_dir: &Path) -> Option<(usize, u64)> {
        let campaign = self.campaigns.get(campaign_dir)?;
        let hours = campaign
            .tails
            .values()
            .flat_map(|(_, tail)| tail.records())
            .filter(|record| matches!(record, Recovered::Frame(_)))
            .count();
        let bytes = campaign
            .tails
            .values()
            .map(|(_, tail)| tail.valid_bytes() as u64)
            .sum();
        Some((hours, bytes))
    }

    /// Only closed, wholly admitted, already packed segments can become a durable watermark.
    pub fn completed_through(&self, campaign_dir: &Path) -> Option<u32> {
        let campaign = self.campaigns.get(campaign_dir)?;
        let (written, _) = campaign.written.as_ref()?;
        let mut closed = Vec::new();
        for (number, (path, tail)) in &campaign.tails {
            let Ok(file) = open_held(path) else { break };
            if file.metadata().ok()?.len() != tail.valid_bytes() as u64 {
                break;
            }
            closed.push((*number, tail.records()));
        }
        written.completed_through(&closed)
    }

    /// Read no segment of the campaign after `through` until its checkpoint takes what was read.
    pub fn hold_through(&mut self, campaign_dir: &Path, through: u32) {
        if let Some(campaign) = self.campaigns.get_mut(campaign_dir) {
            if campaign.tails.keys().any(|number| *number > through) {
                self.campaigns.remove(campaign_dir);
            } else {
                campaign.through = Some(through);
            }
        }
    }

    /// Frames read from the spools.
    pub fn frames_read(&self) -> usize {
        self.campaigns
            .values()
            .flat_map(|campaign| campaign.tails.values())
            .flat_map(|(_, tail)| tail.records())
            .filter(|record| matches!(record, Recovered::Frame(_)))
            .count()
    }

    /// Writes and compresses the blocks of what was read, until `stop`.
    pub fn write(
        &mut self,
        session_dir: &Path,
        replay_dir: &Path,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        with_compression(&RecorderSettings::path(), || {
            let admissions = session_admissions(session_dir)?;
            for (campaign_dir, campaign) in &mut self.campaigns {
                let Some(name) = campaign_dir.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let name = name.to_owned();
                if campaign.stopped || stop() {
                    continue;
                }
                let admitted = admissions
                    .as_ref()
                    .map(|admissions| admissions.loads.get(&name).cloned().unwrap_or_default());
                let segments = admitted_prefix(
                    &campaign.tails,
                    admitted.as_ref(),
                    &campaign.command_bootstrap,
                );
                let Ok(id) = campaign_id(segments.iter().map(|(_, records)| *records)) else {
                    continue;
                };
                if campaign_filename(&id).ok() != Some(name.as_str()) {
                    campaign.stopped = true;
                    continue;
                }
                if campaign.written.is_none() {
                    let sessions = spooled_sessions(segments.iter().flat_map(|(_, records)| *records));
                    let existing = match save_base(session_dir, replay_dir, &name, &sessions) {
                        Ok(None) => None,
                        Ok(Some(destination)) => match held_recording(&destination) {
                            Ok(previous)
                                if previous.require_undamaged().is_ok()
                                    && previous.campaign_id == id =>
                            {
                                Some(previous)
                            }
                            _ => {
                                campaign.stopped = true;
                                continue;
                            }
                        },
                        Err(_) => {
                            campaign.stopped = true;
                            continue;
                        }
                    };
                    let ahead = replay::container::Ahead::new(existing)
                        .map_err(|error| failure(replay_dir, error))?;
                    campaign.written = Some((ahead, Box::default()));
                }
                let frames = FinalizationFrames {
                    segments: campaign
                        .tails
                        .iter()
                        .map(|(number, (path, tail))| {
                            (
                                *number,
                                (Segment::Growing(path.clone()), tail.valid_bytes()),
                            )
                        })
                        .collect(),
                };
                let (ahead, summary) = campaign.written.as_mut().unwrap();
                let _phase = replay::container::Packing::phase(replay::container::Phase::Pack);
                let advanced = ahead.advance(
                    &id,
                    &segments,
                    |number, frame| frames.read(number, frame),
                    |hour| summary.observe_hour(hour).map_err(std::io::Error::other),
                    stop,
                );
                if !matches!(advanced, Ok(true)) {
                    campaign.written = None;
                    campaign.stopped = true;
                }
            }
            Ok(())
        })
    }

    /// All recovered debt, including work background packing cannot admit.
    pub fn outstanding(&self) -> replay::container::Outstanding {
        let mut total = replay::container::Outstanding::default();
        for campaign in self.campaigns.values() {
            let segments = admitted_prefix(&campaign.tails, None, &campaign.command_bootstrap);
            let work = campaign.debt(&segments).work;
            total.rows += work.rows;
            total.compression += work.compression;
            total.readback += work.readback;
            total.open += work.open;
        }
        total
    }

    /// All recovered closed blocks, including those outside the admitted prefix.
    pub fn pending_blocks(&self) -> usize {
        self.campaigns
            .values()
            .map(|campaign| {
                campaign
                    .debt(&admitted_prefix(
                        &campaign.tails,
                        None,
                        &campaign.command_bootstrap,
                    ))
                    .closed_blocks
            })
            .sum()
    }

    /// Only the prefix write can consume drives scheduling; everything else stays visible as blocked.
    pub fn backlog(&self, session_dir: &Path) -> Result<PackingBacklog> {
        let admissions = session_admissions(session_dir)?;
        let mut backlog = PackingBacklog::default();
        for (directory, campaign) in &self.campaigns {
            let all = campaign.debt(&admitted_prefix(
                &campaign.tails,
                None,
                &campaign.command_bootstrap,
            ));
            let runnable = directory
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| {
                    if campaign.stopped {
                        return None;
                    }
                    let admitted = admissions
                        .as_ref()
                        .map(|admissions| admissions.loads.get(name).cloned().unwrap_or_default());
                    let segments = admitted_prefix(
                        &campaign.tails,
                        admitted.as_ref(),
                        &campaign.command_bootstrap,
                    );
                    let id = campaign_id(segments.iter().map(|(_, records)| *records)).ok()?;
                    if campaign_filename(&id).ok() != Some(name)
                        || campaign
                            .written
                            .as_ref()
                            .is_some_and(|(written, _)| !written.continues(&segments))
                    {
                        return None;
                    }
                    Some(campaign.debt(&segments))
                })
                .unwrap_or_default();
            backlog.runnable.work.rows += runnable.work.rows;
            backlog.runnable.work.compression += runnable.work.compression;
            backlog.runnable.work.readback += runnable.work.readback;
            backlog.runnable.work.open += runnable.work.open;
            backlog.runnable.unwritten_hours += runnable.unwritten_hours;
            backlog.runnable.closed_blocks += runnable.closed_blocks;
            backlog.blocked.work.rows += all.work.rows - runnable.work.rows;
            backlog.blocked.work.compression += all.work.compression - runnable.work.compression;
            backlog.blocked.work.readback += all.work.readback - runnable.work.readback;
            backlog.blocked.work.open += all.work.open - runnable.work.open;
            backlog.blocked.unwritten_hours += all.unwritten_hours - runnable.unwritten_hours;
            backlog.blocked.closed_blocks += all.closed_blocks - runnable.closed_blocks;
        }
        Ok(backlog)
    }

    /// Hours written ahead, whose blocks are packed.
    pub fn hours(&self) -> usize {
        self.campaigns
            .values()
            .filter_map(|campaign| campaign.written.as_ref())
            .map(|(ahead, _)| ahead.hours())
            .sum()
    }

    /// Hours whose frames are written ahead, packed or not.
    pub fn written(&self) -> usize {
        self.campaigns
            .values()
            .filter_map(|campaign| campaign.written.as_ref())
            .map(|(ahead, _)| ahead.written())
            .sum()
    }
}

impl CampaignAhead {
    fn debt(&self, segments: &[(u32, &[Recovered])]) -> PackingDebt {
        let (work, written, packed) = self.written.as_ref().map_or(
            (replay::container::Outstanding::default(), 0, 0),
            |(written, _)| (written.outstanding(), written.written(), written.hours()),
        );
        let mut debt = PackingDebt {
            work,
            ..Default::default()
        };
        debt.work.rows = 0;
        let mut read = 0;
        for record in segments.iter().flat_map(|(_, records)| *records) {
            if let Recovered::Frame(frame) = record {
                if frame.keyframe && read > packed {
                    debt.closed_blocks += 1;
                }
                if read >= written {
                    debt.work.rows += frame.placement.length as u64;
                    debt.unwritten_hours += 1;
                }
                read += 1;
            }
        }
        debt
    }
}

/// The records before the first branch whose load is not admitted (yet); every record without admissions.
fn admitted_prefix<'a>(
    tails: &'a BTreeMap<u32, (PathBuf, Tail<Recovered>)>,
    admitted: Option<&BTreeSet<i64>>,
    command_bootstrap: &BTreeSet<BranchId>,
) -> Vec<(u32, &'a [Recovered])> {
    let all: Vec<(u32, &[Recovered])> = tails
        .iter()
        .map(|(number, (_, tail))| (*number, tail.records()))
        .collect();
    let Some(admitted) = admitted else {
        return all;
    };
    let mut branches = BranchLoads::default();
    for record in all.iter().flat_map(|(_, records)| records.iter()) {
        branches.observe(record.view());
    }
    branches.restore_bootstrap(command_bootstrap);
    let mut prefix = Vec::new();
    for (number, records) in all {
        let mut current = None;
        for (at, record) in records.iter().enumerate() {
            match record {
                Recovered::SessionStart(_) => current = None,
                Recovered::BranchStart(branch) => current = Some(branch.id),
                _ => {}
            }
            if current.is_some_and(|branch| {
                !branches
                    .loads
                    .get(&branch)
                    .is_some_and(|load| admitted.contains(load))
            }) {
                prefix.push((number, &records[..at]));
                return prefix;
            }
        }
        prefix.push((number, records));
    }
    prefix
}

impl FinalizationFrames {
    fn new(held: BTreeMap<u32, fs::File>, segments: &[(u32, Recovery<Recovered>)]) -> Self {
        Self {
            segments: held
                .into_iter()
                .filter_map(|(number, file)| {
                    segments
                        .iter()
                        .find(|(segment, _)| *segment == number)
                        .map(|(_, recovery)| {
                            let file = Segment::Held(std::sync::Mutex::new(file));
                            (number, (file, recovery.valid_bytes))
                        })
                })
                .collect(),
        }
    }

    fn read(&self, number: u32, expected: &FrameMetadata) -> std::io::Result<Frame> {
        let (segment, length) = self
            .segments
            .get(&number)
            .ok_or_else(|| std::io::Error::other("recovered spool segment is missing"))?;
        let frame = match segment {
            Segment::Held(file) => {
                let payload = read_frame_payload_at(
                    &mut Budgeted::new(
                        &mut *file
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    ),
                    expected.placement,
                    *length,
                )?;
                payload.decode()?
            }
            Segment::Growing(path) => read_frame_at(
                &mut Budgeted::new(fs::File::open(path)?),
                expected.placement,
                *length,
            )?,
        };
        if (frame.branch, frame.hour, frame.keyframe)
            != (expected.branch, expected.hour, expected.keyframe)
        {
            return Err(replay_data::codec::invalid(
                "spool frame differs from recovered metadata",
            ));
        }
        Ok(frame)
    }
}

/// A prepared identity record means no collector failure was published.
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
    empty_prefix: bool,
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
        empty_prefix,
        bundled: records.len() > 1,
    })
}

/// Where a projected failure witness ends `campaign_dir`'s committed spool; refused when it
/// names no end for it.
fn committed_watermark(
    campaign_dir: &Path,
    projected: &CampaignFailure,
) -> Result<Option<(u32, u64)>> {
    let witness = &projected.witness;
    let folder = campaign_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::as_bytes);
    let committed_campaign = witness
        .committed_campaign_id
        .as_ref()
        .is_some_and(|campaign| folder == Some(campaign.as_slice()));
    let pending_campaign = witness
        .pending_campaign_id
        .as_ref()
        .is_some_and(|campaign| folder == Some(campaign.as_slice()));
    if !projected.empty_prefix
        && (witness.committed_campaign_id.is_none()
            || (pending_campaign && witness.pending_segment.is_none()))
    {
        return Err(failure(
            campaign_dir,
            "failure witness has no committed spool watermark",
        ));
    }
    let committed = witness
        .committed_segment
        .filter(|_| committed_campaign)
        .map(|segment| (segment, witness.committed_bytes));
    let pending = witness
        .pending_segment
        .filter(|_| pending_campaign)
        .map(|segment| (segment, witness.pending_bytes));
    Ok(match (committed, pending) {
        (Some(committed), Some(pending)) => Some(committed.min(pending)),
        (committed, pending) => committed.or(pending),
    })
}

/// How much of a campaign's spool its session's capture failure leaves publishable; the bytes
/// past it are kept only as evidence of the failure.
#[derive(Clone, Copy)]
enum SpoolEnd {
    Whole,
    /// Up to this byte of this segment.
    At(u32, u64),
    /// None of it.
    Empty,
}

impl SpoolEnd {
    fn of(campaign_dir: &Path) -> Result<Self> {
        let session_dir = campaign_dir.parent().unwrap_or(campaign_dir);
        let Some(witness) = session_witness(session_dir)? else {
            return Ok(Self::Whole);
        };
        let folder = campaign_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| failure(campaign_dir, "campaign folder has no name"))?;
        let projected = campaign_failure(session_dir, folder.as_bytes(), &witness)?;
        Ok(match committed_watermark(campaign_dir, &projected)? {
            _ if projected.empty_prefix => Self::Empty,
            Some((segment, bytes)) => Self::At(segment, bytes),
            None => Self::Whole,
        })
    }

    /// The bytes of segment `number` at `path`, `length` long with its first `saved` bytes
    /// saved, that no save took and the failure leaves publishable; `None` when none wait.
    fn unsaved(
        self,
        path: &Path,
        number: u32,
        length: u64,
        saved: Option<u64>,
    ) -> Result<Option<std::ops::Range<u64>>> {
        if saved == Some(length) {
            return Ok(None);
        }
        let end = match self {
            Self::At(segment, bytes) if number == segment => {
                if bytes > length {
                    return Err(failure(
                        path,
                        "failure witness watermark exceeds the spool length",
                    ));
                }
                bytes
            }
            Self::At(segment, _) if number > segment => 0,
            Self::Empty => 0,
            Self::Whole | Self::At(..) => length,
        };
        let start = saved.filter(|saved| *saved <= length).unwrap_or(0);
        // A segment saved up to its watermark holds only failure evidence past it.
        Ok((start < end || end == length).then_some(start..end))
    }
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
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if !entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_dir()
        {
            continue;
        }
        for (_, path) in segment_paths(&entry.path())? {
            let file = fs::File::open(&path).map_err(|error| failure(&path, error))?;
            let length = usize::try_from(
                file.metadata()
                    .map_err(|error| failure(&path, error))?
                    .len(),
            )
            .map_err(|error| failure(&path, error))?;
            if let Ok(Some(Record::SessionStart(session))) =
                RecordReader::new(Budgeted::new(file), length).next_record()
                && session.id == witness.session
                && session.process_start_time == witness.process_start_time
            {
                return Ok(session);
            }
        }
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

fn campaign_id<'a>(segments: impl IntoIterator<Item = &'a [Recovered]>) -> Result<Vec<u8>> {
    let mut identity: Option<Vec<u8>> = None;
    for record in segments.into_iter().flatten() {
        let observed = match record {
            Recovered::Frame(frame) => match &frame.campaign {
                TableUpdate::Snapshot(TableRows::Campaign(rows)) => match rows.as_slice() {
                    [campaign] if !campaign.id.is_empty() => Some(campaign.id.as_slice()),
                    _ => None,
                },
                _ => None,
            },
            Recovered::Control(ControlRecord::PlayerCommands { packet, .. }) => {
                packet.phase.as_ref().map(|phase| phase.campaign.as_slice())
            }
            _ => None,
        };
        if let Some(observed) = observed {
            if identity
                .as_ref()
                .is_some_and(|known| !known.eq_ignore_ascii_case(observed))
            {
                return Err("conflicting campaign identity in spool observations".into());
            }
            identity.get_or_insert_with(|| observed.to_vec());
        }
    }
    identity.ok_or_else(|| "no complete campaign identity survived in the spool; directory names cannot establish identity".into())
}

fn campaign_filename(id: &[u8]) -> Result<&str> {
    if !is_campaign_uuid(id) {
        return Err(
            "recorded campaign identity is not a UUID; refusing to use it as a file name".into(),
        );
    }
    std::str::from_utf8(id).map_err(|error| error.to_string())
}

/// The recordings a session may continue, as their folder held them when it started.
pub const RECORDINGS_FILE: &str = "recordings.json";

/// Noted before the collector starts; one that cannot be read refuses the session.
pub fn select_recordings(session_dir: &Path, replay_dir: &Path) -> Result<()> {
    // Created now, so the folder noted is the one a save canonicalises.
    fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let found = recording_files::recordings(replay_dir).map_err(|error| error.to_string())?;
    // One that cannot be read continues nothing, so only this session's own sessions count.
    let staged = recording_files::recordings(&replay::publication::unsaved_folder(replay_dir))
        .unwrap_or_default();
    write_selection(
        session_dir,
        &Selection {
            folder: folder_key(replay_dir).map_err(|error| failure(replay_dir, error))?,
            recordings: found
                .into_iter()
                .map(|(campaign, paths)| (String::from_utf8_lossy(&campaign).into_owned(), paths))
                .collect(),
            continues: staged
                .into_iter()
                .map(|(campaign, paths)| {
                    let sessions = paths
                        .iter()
                        .filter_map(|path| Replay::read_session_ids(path).ok())
                        .flatten()
                        .collect();
                    (String::from_utf8_lossy(&campaign).into_owned(), sessions)
                })
                .collect(),
        },
    )
}

struct Selection {
    /// The folder the recordings were noted in; another folder ignores them.
    folder: PathBuf,
    recordings: BTreeMap<String, Vec<PathBuf>>,
    /// The sessions whose unsaved hours of each campaign this session continues.
    continues: BTreeMap<String, Vec<u64>>,
}

fn folder_key(folder: &Path) -> std::io::Result<PathBuf> {
    match fs::canonicalize(folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(folder.to_path_buf()),
        canonical => canonical,
    }
}

fn write_selection(session_dir: &Path, selection: &Selection) -> Result<()> {
    write_json(
        &session_dir.join(RECORDINGS_FILE),
        &json!({
            "folder": selection.folder,
            "recordings": selection.recordings,
            "continues": selection.continues,
        }),
    )
}

fn selected_recordings(session_dir: &Path) -> Result<Option<Selection>> {
    let path = session_dir.join(RECORDINGS_FILE);
    let bytes = match read_file(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        bytes => bytes.map_err(|error| failure(&path, error))?,
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| failure(&path, error))?;
    Ok(Some(Selection {
        folder: serde_json::from_value(value["folder"].clone())
            .map_err(|error| failure(&path, error))?,
        recordings: serde_json::from_value(value["recordings"].clone())
            .map_err(|error| failure(&path, error))?,
        // Noted only when unsaved hours waited.
        continues: match &value["continues"] {
            Value::Null => BTreeMap::new(),
            continues => {
                serde_json::from_value(continues.clone()).map_err(|error| failure(&path, error))?
            }
        },
    }))
}

/// The ids of the sessions a campaign's spool records.
fn spooled_sessions<'a>(records: impl IntoIterator<Item = &'a Recovered>) -> Vec<u64> {
    records
        .into_iter()
        .filter_map(|record| match record {
            Recovered::SessionStart(session) => Some(session.id),
            _ => None,
        })
        .collect()
}

/// The recording this session continues, found again if renamed, within `replay_dir` only.
fn continued_recording(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &[u8],
    sessions: &[u64],
) -> Result<Option<PathBuf>> {
    let look = || {
        recording_files::recording_of(replay_dir, campaign)
            .map_err(|error| failure(replay_dir, error))
    };
    let folder = folder_key(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let Some(selection) = selected_recordings(session_dir)?
        .filter(|selection| folder_key(&selection.folder).is_ok_and(|selected| selected == folder))
    else {
        return look();
    };
    let key = String::from_utf8_lossy(campaign).to_ascii_lowercase();
    match selection.recordings.get(&key).map(Vec::as_slice) {
        // A recording that appeared since is this session's own only if it holds one of its
        // sessions, continued unsaved sessions, or a verified revision this session saved.
        None | Some([]) => match look()? {
            None => Ok(None),
            Some(path) => {
                let continued = selection.continues.get(&key).map_or(&[][..], Vec::as_slice);
                let found =
                    Replay::read_session_ids(&path).map_err(|error| failure(&path, error))?;
                let holds_session = found
                    .iter()
                    .any(|session| sessions.contains(session) || continued.contains(session));
                let saved_ancestor = if holds_session {
                    false
                } else if let Some(saved) = read_saved(&session_dir.join(&key)) {
                    let previous = held_recording(&path)?;
                    previous
                        .require_undamaged()
                        .map_err(|error| failure(&path, error))?;
                    saved_ancestry(&saved, replay_dir, campaign, &path, &previous)?
                } else {
                    false
                };
                if holds_session || saved_ancestor {
                    Ok(Some(path))
                } else {
                    Err(failure(
                        &path,
                        "a recording of this campaign appeared after the session started; move it, then save again; spool retained",
                    ))
                }
            }
        },
        Some([path]) => {
            // Only a file really in the locked folder: a link repointed since cannot lead elsewhere.
            let within = match fs::canonicalize(path) {
                Ok(real) if real.parent() == Some(folder.as_path()) => {
                    real.file_name().map(|name| replay_dir.join(name))
                }
                Ok(_) => None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(failure(path, error)),
            };
            let found = within
                .as_deref()
                .map(recording_files::campaign_of)
                .transpose();
            match found {
                Ok(Some(Some(found))) if found.eq_ignore_ascii_case(campaign) => Ok(within),
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(failure(path, error))
                }
                _ => look()?.map(Some).ok_or_else(|| {
                    failure(
                        path,
                        "the recording this session continues left its folder; nothing published; spool retained",
                    )
                }),
            }
        }
        // Several were there when it started: it continues the one of them still there alone.
        Some(paths) => match look()? {
            Some(kept) if paths.iter().any(|path| path.file_name() == kept.file_name()) => {
                Ok(Some(kept))
            }
            _ => Err(failure(
                replay_dir,
                format!(
                    "{} recordings of this campaign were there when the session started; keep one of them there under its name and move the others",
                    paths.len()
                ),
            )),
        },
    }
}

/// Each saved file's multiplayer hour by its size and time, so its rosters are read once.
static ROSTERS: std::sync::Mutex<BTreeMap<PathBuf, (u64, std::time::SystemTime, bool)>> =
    std::sync::Mutex::new(BTreeMap::new());

fn roster_stamp(path: &Path) -> std::io::Result<(PathBuf, u64, std::time::SystemTime)> {
    let metadata = fs::metadata(path)?;
    Ok((
        fs::canonicalize(path)?,
        metadata.len(),
        metadata.modified()?,
    ))
}

fn known_roster(path: &Path) -> std::io::Result<Option<bool>> {
    let (path, length, modified) = roster_stamp(path)?;
    let rosters = ROSTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(rosters
        .get(&path)
        .filter(|(known_length, known_modified, _)| {
            (*known_length, *known_modified) == (length, modified)
        })
        .map(|(_, _, multiplayer)| *multiplayer))
}

fn remember_roster(path: &Path, multiplayer: bool) -> std::io::Result<()> {
    let (path, length, modified) = roster_stamp(path)?;
    ROSTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path, (length, modified, multiplayer));
    Ok(())
}

/// Note the recording a save published, so saving the session again continues it.
fn record_recording(session_dir: &Path, campaign: &[u8], path: &Path) -> Result<()> {
    let Some(mut selection) = selected_recordings(session_dir)? else {
        return Ok(());
    };
    let noted = folder_key(&selection.folder).map_err(|error| failure(path, error))?;
    let published = path.parent().map(folder_key).transpose();
    if published.map_err(|error| failure(path, error))? != Some(noted) {
        return Ok(());
    }
    selection.recordings.insert(
        String::from_utf8_lossy(campaign).to_ascii_lowercase(),
        vec![path.to_path_buf()],
    );
    write_selection(session_dir, &selection)
}

/// The campaign folders of a session that hold spool segments.
pub fn spooled_campaigns(session_dir: &Path) -> Result<Vec<String>> {
    let mut campaigns = Vec::new();
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if !entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_dir()
            || segment_paths(&entry.path())?.is_empty()
        {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            campaigns.push(name.to_owned());
        }
    }
    for range in read_failure_witnesses(session_dir)?
        .iter()
        .filter_map(|record| record.uncommitted)
    {
        let campaign = String::from_utf8(range.campaign_id.to_vec())
            .map_err(|error| failure(session_dir, error))?;
        if !campaigns.contains(&campaign) {
            campaigns.push(campaign);
        }
    }
    campaigns.sort();
    Ok(campaigns)
}

/// Whether any campaign folder of a session holds a spool segment.
pub fn has_campaign_spools(session_dir: &Path) -> Result<bool> {
    Ok(!spooled_campaigns(session_dir)?.is_empty())
}

/// The loads a Battle Plan Recorder session admitted, by campaign and the collector's
/// load signal: the only hours the session publishes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Admissions {
    pub loads: BTreeMap<String, BTreeSet<i64>>,
    /// Loads refused, with why: kept to report what was left out, never to
    /// decide what is published.
    pub refused: BTreeMap<(String, i64), Refusal>,
}

/// A Battle Plan Recorder session's admission record in its folder: each admitted
/// campaign and load signal with the folder the session publishes into, and
/// each refused load with why, rewritten atomically as each load is decided.
pub const ADMISSIONS_FILE: &str = "admitted.json";
/// In the session's `bin` folder, written before the collector starts: it
/// marks a Battle Plan Recorder session, so a missing admission record publishes
/// nothing rather than everything.
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

    /// Admit a load: it is on disk before this returns, so a crash or an
    /// unfinished session still publishes it, and nothing else.
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

    /// Note why a load was refused, for reporting. It is never published
    /// either way; a failed write only loses the reason.
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

    /// A left-out load as reported: refused with its reason, or never admitted.
    fn omission(&self, campaign: &str, load: Option<i64>, hours: u64) -> Omission {
        Omission {
            campaign: campaign.to_owned(),
            load,
            hours,
            refusal: load.and_then(|load| self.refused.get(&(campaign.to_owned(), load)).copied()),
        }
    }
}

/// Start a Battle Plan Recorder session's admission record, with no load admitted,
/// then its marker. Call before the collector starts.
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

/// The loads a session admitted. `None`: not a Battle Plan Recorder session (the
/// command line), so every load publishes as it always has. A Battle Plan Recorder
/// session whose record is missing or unreadable refuses finalization, so
/// nothing is published.
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

/// The first clock witness of a campaign's spool (its time and load signal).
/// Reads only the first records of the first segment, so it is safe while the
/// collector is still writing.
fn first_witness(campaign_dir: &Path) -> Option<(u64, Option<i64>)> {
    let (_, first) = segment_paths(campaign_dir).ok()?.into_iter().next()?;
    let file = fs::File::open(&first).ok()?;
    let length = usize::try_from(file.metadata().ok()?.len()).ok()?;
    let mut reader = RecordReader::new(Budgeted::new(file), length);
    let mut first = None;
    while let Ok(Some(record)) = reader.next_record() {
        let observed = match record {
            Record::Control(ControlRecord::ClockWitness {
                qpc, load_signal, ..
            }) => Some((qpc, load_signal)),
            Record::Control(ControlRecord::PlayerCommands { packet, .. }) => packet
                .phase
                .map(|phase| (phase.qpc, Some(phase.load_signal))),
            _ => None,
        };
        if let Some(observed) = observed
            && first.is_none_or(|(time, _)| observed.0 < time)
        {
            first = Some(observed);
        }
    }
    first
}

/// The capture mode a campaign's spool records, from its first segment's session.
pub fn spool_capture_mode(campaign_dir: &Path) -> Option<replay_data::scope::CaptureMode> {
    let (_, first) = segment_paths(campaign_dir).ok()?.into_iter().next()?;
    let file = fs::File::open(&first).ok()?;
    let length = usize::try_from(file.metadata().ok()?.len()).ok()?;
    match RecordReader::new(Budgeted::new(file), length).next_record() {
        Ok(Some(Record::SessionStart(session))) => Some(session.capture_mode),
        _ => None,
    }
}

/// Whether this session's spool for a campaign already holds hours from before
/// load `load`: its first clock witness carries another load signal (or none).
pub fn recorded_before_load(campaign_dir: &Path, load: i64) -> bool {
    first_witness(campaign_dir).is_some_and(|(_, signal)| signal != Some(load))
}

/// The campaign this session loaded first, as its spool shows: the one whose
/// first clock witness is earliest. Every campaign's witnesses share the
/// session's clock. Reads only the first records of each campaign's spool.
pub fn first_spooled_campaign(session_dir: &Path) -> Option<String> {
    spooled_campaigns(session_dir)
        .ok()?
        .into_iter()
        .filter(|campaign| is_campaign_uuid(campaign.as_bytes()))
        .filter_map(|campaign| Some((first_witness(&session_dir.join(&campaign))?.0, campaign)))
        .min_by_key(|(qpc, _)| *qpc)
        .map(|(_, campaign)| campaign)
}

/// Each spool branch's load, first clock witness time and hours, read from
/// its records: a branch's load is the first load signal its clock witnesses
/// carry. A branch continuing into a later segment repeats its start there.
#[derive(Default)]
struct BranchLoads {
    command_bootstrap: BTreeSet<BranchId>,
    order: Vec<BranchId>,
    loads: BTreeMap<BranchId, i64>,
    sessions: BTreeMap<BranchId, SessionId>,
    first_qpc: BTreeMap<BranchId, u64>,
    hours: BTreeMap<BranchId, u64>,
    current: Option<BranchId>,
}

impl BranchLoads {
    fn observe(&mut self, record: RecordRef<'_>) {
        match record {
            RecordRef::SessionStart(_) => self.current = None,
            RecordRef::BranchStart(branch) => {
                self.sessions.entry(branch.id).or_insert(branch.session);
                if !self.order.contains(&branch.id) {
                    self.order.push(branch.id);
                }
                self.current = Some(branch.id);
            }
            RecordRef::Control(ControlRecord::ClockWitness {
                qpc, load_signal, ..
            }) => {
                if let Some(branch) = self.current {
                    self.first_qpc.entry(branch).or_insert(*qpc);
                    if let Some(signal) = load_signal {
                        self.loads.entry(branch).or_insert(*signal);
                    }
                }
            }
            RecordRef::Control(ControlRecord::PlayerCommands { session, packet }) => {
                self.sessions.entry(packet.branch).or_insert(*session);
                if !self.order.contains(&packet.branch) {
                    self.order.push(packet.branch);
                }
                if let Some(phase) = &packet.phase {
                    self.first_qpc.entry(packet.branch).or_insert(phase.qpc);
                    if phase.load_signal == 0 {
                        self.command_bootstrap.insert(packet.branch);
                        self.loads.insert(packet.branch, 0);
                    } else {
                        self.loads.entry(packet.branch).or_insert(phase.load_signal);
                    }
                }
            }
            RecordRef::Frame(_) | RecordRef::Metadata(_) => {
                let (branch, _, _) = record.frame().unwrap();
                *self.hours.entry(branch).or_default() += 1;
            }
            _ => {}
        }
    }

    fn restore_bootstrap(&mut self, branches: &BTreeSet<BranchId>) {
        self.command_bootstrap.extend(branches.iter().copied());
        for &branch in &self.command_bootstrap {
            self.loads.insert(branch, 0);
        }
    }

    fn dropped(&self, admitted: &BTreeSet<i64>) -> BTreeSet<BranchId> {
        self.order
            .iter()
            .copied()
            .filter(|branch| {
                !self
                    .loads
                    .get(branch)
                    .is_some_and(|load| admitted.contains(load))
            })
            .collect()
    }

    fn undecided(&self, admissions: Option<&Admissions>, campaign: &str) -> bool {
        let Some(admissions) = admissions else {
            return false;
        };
        self.order.iter().any(|branch| {
            self.loads.get(branch).is_none_or(|load| {
                !admissions
                    .loads
                    .get(campaign)
                    .is_some_and(|loads| loads.contains(load))
                    && !admissions
                        .refused
                        .contains_key(&(campaign.to_owned(), *load))
            })
        })
    }

    /// The loads of `branches`, in spool order, with the hours each recorded;
    /// hours of branches with no identified load count under `None`.
    fn by_load(&self, branches: impl Iterator<Item = BranchId>) -> Vec<(Option<i64>, u64)> {
        let mut result: Vec<(Option<i64>, u64)> = Vec::new();
        for branch in branches {
            let load = self.loads.get(&branch).copied();
            let hours = self.hours.get(&branch).copied().unwrap_or(0);
            match result.iter_mut().find(|(known, _)| *known == load) {
                Some((_, total)) => *total += hours,
                None => result.push((load, hours)),
            }
        }
        result
    }
}

/// A campaign's spool branches, read from each segment's valid prefix, as
/// recovery reads it.
fn read_branch_loads(campaign_dir: &Path) -> Result<BranchLoads> {
    let mut branches = BranchLoads::default();
    for (_, path) in segment_paths(campaign_dir)? {
        let file = fs::File::open(&path).map_err(|error| failure(&path, error))?;
        let length = file
            .metadata()
            .map_err(|error| failure(&path, error))?
            .len();
        let length = usize::try_from(length).map_err(|error| failure(&path, error))?;
        branches.current = None;
        // Damage ends a segment's readable prefix.
        recover_reader(
            std::io::BufReader::new(Budgeted::new(file)),
            length,
            Recovered::keep,
            |record| {
                branches.observe(record.view());
                Ok(())
            },
        )
        .map_err(|error| failure(&path, error))?;
    }
    if let Some(saved) = read_saved(campaign_dir) {
        branches.restore_bootstrap(&saved.command_bootstrap);
    }
    Ok(branches)
}

/// Each load in a campaign's spool, in spool order, with the hours it recorded.
pub fn spool_loads(campaign_dir: &Path) -> Result<Vec<(Option<i64>, u64)>> {
    let branches = read_branch_loads(campaign_dir)?;
    Ok(branches.by_load(branches.order.iter().copied()))
}

/// Every load this session's spool identifies, in the order they were loaded
/// across campaigns: by each load's first clock witness, since every
/// campaign's witnesses share the session's clock.
pub fn spooled_loads(session_dir: &Path) -> Result<Vec<(String, i64)>> {
    let mut loads: Vec<(u64, String, i64)> = Vec::new();
    for campaign in spooled_campaigns(session_dir)? {
        if !is_campaign_uuid(campaign.as_bytes()) {
            continue;
        }
        let branches = read_branch_loads(&session_dir.join(&campaign))?;
        for branch in &branches.order {
            if let (Some(load), Some(qpc)) =
                (branches.loads.get(branch), branches.first_qpc.get(branch))
            {
                loads.push((*qpc, campaign.clone(), *load));
            }
        }
    }
    loads.sort_by_key(|(qpc, _, _)| *qpc);
    let mut seen = BTreeSet::new();
    Ok(loads
        .into_iter()
        .filter(|(_, campaign, load)| seen.insert((campaign.clone(), *load)))
        .map(|(_, campaign, load)| (campaign, load))
        .collect())
}

/// Keep admitted branches; checked witness or saved-recording evidence can supply a missing clock.
fn keep_admitted_loads(
    segments: &mut [(u32, Recovery<Recovered>)],
    admitted: &BTreeSet<i64>,
    known: &[(BranchId, i64)],
    command_bootstrap: &BTreeSet<BranchId>,
) -> (BTreeSet<BranchId>, Vec<(Option<i64>, u64)>) {
    let mut branches = BranchLoads::default();
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        branches.observe(record.view());
    }
    for (branch, load) in known {
        branches.loads.entry(*branch).or_insert(*load);
    }
    branches.restore_bootstrap(command_bootstrap);
    let dropped = branches.dropped(admitted);
    for (_, recovery) in segments.iter_mut() {
        let mut dropping = false;
        recovery.records.retain(|record| {
            if let Recovered::Control(ControlRecord::PlayerCommands { packet, .. }) = record {
                return !dropped.contains(&packet.branch);
            }
            match record {
                Recovered::SessionStart(_) => dropping = false,
                Recovered::BranchStart(branch) => dropping = dropped.contains(&branch.id),
                _ => {}
            }
            !dropping
        });
    }
    let omitted = branches.by_load(
        branches
            .order
            .iter()
            .copied()
            .filter(|branch| dropped.contains(branch)),
    );
    (dropped, omitted)
}

/// What one finalization published and left out.
#[derive(Debug, Default)]
pub struct Finalized {
    pub replays: Vec<PathBuf>,
    pub saved_campaigns: BTreeSet<String>,
    pub failed_campaigns: BTreeSet<String>,
    /// Unsaved recordings written.
    pub unsaved: Vec<PathBuf>,
    /// Campaigns that could not be published.
    pub errors: Vec<String>,
    /// Loads a Battle Plan Recorder session left out, with their hours and why.
    pub omitted: Vec<Omission>,
    /// Recordings saved under their old name because renaming failed.
    pub unrenamed: Vec<String>,
}

#[cfg(test)]
impl Finalized {
    /// The recordings and errors, for tests of sessions that leave nothing out.
    fn parts(self) -> (Vec<PathBuf>, Vec<String>) {
        assert!(self.omitted.is_empty(), "{:?}", self.omitted);
        (self.replays, self.errors)
    }
}

/// Call only after detach and module-absence confirmation, or after process exit.
/// Gives `recording` the game facts this session's recorder gathered, as a save of its own,
/// when it carries none and they are its content's: a recording saved before the facts were
/// packed, with no hours added since, gets them here. An unsaved recording, or one an unsaved
/// recording continues, is left unchanged so the continuation still holds it: the facts wait
/// beside the unsaved recording, which carries them when saved ([`publish_unsaved`]) and hands
/// them back when discarded ([`discard_unsaved_recording`]). Whether it now carries them.
pub fn attach_session_facts(session_dir: &Path, recording: &Path) -> Result<bool> {
    let Some(bytes) = session_facts(session_dir)? else {
        return Ok(false);
    };
    let folder = match recording.parent() {
        Some(folder) if is_unsaved_recording(recording) => folder.parent().unwrap_or(folder),
        Some(folder) => folder,
        None => Path::new("."),
    };
    let _lock = NamesLock::wait(folder).map_err(|error| failure(folder, error))?;
    let mut replay =
        Replay::open_appendable(recording).map_err(|error| failure(recording, error))?;
    if !replay.fact_packs.is_empty() {
        return Ok(true);
    }
    let Some(pack) = replay::container::session_facts_for(&bytes, &replay.identity) else {
        return Ok(false);
    };
    let staged = if is_unsaved_recording(recording) {
        Some(recording.to_owned())
    } else {
        match std::str::from_utf8(&replay.campaign_id) {
            Ok(campaign) => staged_recording(folder, campaign)?,
            Err(_) => None,
        }
    };
    if let Some(staged) = staged {
        drop(replay);
        keep_facts_beside(&staged, &bytes)?;
        return Ok(false);
    }
    replay
        .attach_facts_in_place(pack)
        .map_err(|error| failure(recording, error))?;
    Ok(true)
}

pub fn finalize_session(session_dir: &Path, replay_dir: &Path) -> Result<Vec<PathBuf>> {
    let finalized = finalize_session_outcome(session_dir, replay_dir)?;
    if finalized.errors.is_empty() {
        Ok(finalized.replays)
    } else {
        Err(finalized.errors.join("\n"))
    }
}

/// Keep successfully written campaigns visible when another campaign is
/// incomplete. A Battle Plan Recorder session publishes only the loads it admitted.
pub fn finalize_session_outcome(session_dir: &Path, replay_dir: &Path) -> Result<Finalized> {
    finalize_session_outcome_with_progress(session_dir, replay_dir, &|| Ok(()))
}

pub fn finalize_session_outcome_with_progress(
    session_dir: &Path,
    replay_dir: &Path,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<Finalized> {
    let settings = RecorderSettings::path();
    let naming = Naming {
        localisation: None,
        settings: &settings,
        report: None,
    };
    finalize_session_ahead(
        session_dir,
        replay_dir,
        progress,
        Ahead::default(),
        naming,
        &|_| Destination::Recording,
    )
}

/// How a save names recordings.
#[derive(Clone, Copy)]
pub struct Naming<'a> {
    /// The recording's own localisation; without it countries are named by tag.
    pub localisation: Option<&'a Localisation>,
    /// The recorder settings that keep the names users chose.
    pub settings: &'a Path,
    /// The country-name refusal of each actual save, including a successful name clearing it.
    pub report: Option<&'a (dyn Fn(Option<&str>) + Sync)>,
}

/// A session's failure witness, refusing a failed capture without one.
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

/// What a session's save does with one of its campaigns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Recording,
    /// The unsaved recording, which the next session continues and Save publishes.
    Unsaved,
    /// Left in the session folder as it is.
    Neither,
}

/// Finalizes taking over the recovery and blocks written ahead where they still hold.
pub fn finalize_session_ahead(
    session_dir: &Path,
    replay_dir: &Path,
    progress: &dyn Fn() -> std::io::Result<()>,
    mut ahead: Ahead,
    naming: Naming<'_>,
    destination: &dyn Fn(&str) -> Destination,
) -> Result<Finalized> {
    with_compression(naming.settings, || {
        progress().map_err(|error| failure(session_dir, error))?;
        reconcile_session_checkpoints(session_dir, replay_dir)?;
        let admissions = session_admissions(session_dir)?;
        let mut finalized = Finalized::default();
        let witness = session_witness(session_dir)?;
        fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
        let mut campaigns = Vec::new();
        let mut campaign_names = spooled_campaigns(session_dir)?;
        if session_ended(session_dir) {
            for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
                let entry = entry.map_err(|error| failure(session_dir, error))?;
                if entry
                    .file_type()
                    .map_err(|error| failure(&entry.path(), error))?
                    .is_dir()
                    && read_saved(&entry.path()).is_some()
                    && let Some(name) = entry.file_name().to_str()
                    && !campaign_names.iter().any(|known| known == name)
                {
                    campaign_names.push(name.to_owned());
                }
            }
        }
        for campaign in campaign_names {
            let campaign_dir = session_dir.join(&campaign);
            fs::create_dir_all(&campaign_dir).map_err(|error| failure(&campaign_dir, error))?;
            let segments = segment_paths(&campaign_dir)?;
            let name = Some(campaign);
            let unsaved = match name.as_deref().map_or(Destination::Recording, destination) {
                Destination::Recording => false,
                Destination::Unsaved => true,
                Destination::Neither => continue,
            };
            // A Battle Plan Recorder session publishes nothing of a campaign without
            // an admitted load; its hours are reported as left out.
            let admitted = match &admissions {
                None => None,
                Some(admissions) => {
                    match name.as_deref().and_then(|name| admissions.loads.get(name)) {
                        Some(loads) => Some(loads),
                        None => {
                            if let Some(name) = &name {
                                match spool_loads(&campaign_dir) {
                                    Ok(loads) => finalized.omitted.extend(loads.into_iter().map(
                                        |(load, hours)| admissions.omission(name, load, hours),
                                    )),
                                    Err(error) => {
                                        finalized.failed_campaigns.insert(name.clone());
                                        finalized.errors.push(error);
                                    }
                                }
                            }
                            continue;
                        }
                    }
                }
            };
            campaigns.push((campaign_dir, name, segments, admitted, unsaved));
        }
        campaigns.sort_by(|left, right| left.0.cmp(&right.0));
        if campaigns.is_empty() && admissions.is_none() {
            return Err(failure(session_dir, "no campaign spool segments found"));
        }
        let mut process_context = None;
        let mut committed = ahead.committed.take();
        for (campaign_dir, name, paths, admitted, unsaved) in campaigns {
            let campaign_ahead = ahead.campaigns.remove(&campaign_dir);
            match finalize_campaign(
                &campaign_dir,
                &paths,
                replay_dir,
                unsaved,
                true,
                &mut process_context,
                witness.as_ref(),
                admitted,
                progress,
                campaign_ahead,
                ahead.checkpoint_reports.remove(&campaign_dir),
                naming,
                &mut committed,
            ) {
                Ok(outcome) => {
                    if unsaved {
                        finalized.unsaved.extend(outcome.replay);
                    } else {
                        if outcome.replay.is_some() {
                            finalized.saved_campaigns.extend(name.clone());
                        }
                        finalized.replays.extend(outcome.replay);
                    }
                    finalized.unrenamed.extend(outcome.unrenamed);
                    if let (Some(admissions), Some(name)) = (&admissions, &name) {
                        finalized.omitted.extend(
                            outcome
                                .omitted
                                .into_iter()
                                .map(|(load, hours)| admissions.omission(name, load, hours)),
                        );
                    }
                }
                Err(error) => {
                    finalized.failed_campaigns.extend(name);
                    finalized.errors.push(error);
                }
            }
        }
        if admissions.is_some()
            && finalized.replays.is_empty()
            && finalized.unsaved.is_empty()
            && finalized.errors.is_empty()
        {
            println!("No admitted load recorded an hour; nothing published.");
        }
        // By campaign; each campaign's loads stay in spool order.
        finalized
            .omitted
            .sort_by(|left, right| left.campaign.cmp(&right.campaign));
        for omission in &finalized.omitted {
            println!(
                "{}: {} hours of load {} left out ({}); kept in the session folder",
                omission.campaign,
                omission.hours,
                omission
                    .load
                    .map_or_else(|| "unidentified".to_owned(), |load| load.to_string()),
                omission.refusal.map_or("not admitted", Refusal::as_str)
            );
        }
        for unrenamed in &finalized.unrenamed {
            println!("{unrenamed}");
        }
        finalized
            .errors
            .extend(attach_published_facts(session_dir, &finalized.replays));
        finalized
            .errors
            .extend(keep_unsaved_facts(session_dir, &finalized.unsaved));
        Ok(finalized)
    })
}

/// The game facts this session's recorder gathered, or none when it gathered none; one that
/// cannot be read is an error, so nothing that waits on them is let go.
fn session_facts(session_dir: &Path) -> Result<Option<Vec<u8>>> {
    let path = session_dir.join(replay::container::SESSION_FACTS_FILE);
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failure(&path, format!("{error}; the game facts are kept"))),
    }
}

/// Where an unsaved recording's game facts wait beside it until it is saved: the staged file
/// itself is never changed, so its checkpoint journal stays true.
fn unsaved_facts(staged: &Path) -> PathBuf {
    let mut name = staged.file_name().unwrap_or_default().to_owned();
    name.push(".facts");
    staged.with_file_name(name)
}

/// Keeps this session's facts beside each unsaved recording that lacks them and is its
/// content's; what could not be kept, with why.
fn keep_unsaved_facts(session_dir: &Path, unsaved: &[PathBuf]) -> Vec<String> {
    let bytes = match session_facts(session_dir) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Vec::new(),
        Err(error) => return vec![error],
    };
    unsaved
        .iter()
        .filter_map(|staged| keep_facts_beside(staged, &bytes).err())
        .collect()
}

/// Keeps a session's facts (`bytes`, [`replay::container::SESSION_FACTS_FILE`]) beside the
/// unsaved recording `staged` when it lacks them, they are its content's and none wait there.
fn keep_facts_beside(staged: &Path, bytes: &[u8]) -> Result<()> {
    let sidecar = unsaved_facts(staged);
    let Ok(replay) = held_recording(staged) else {
        return Ok(());
    };
    if !replay.fact_packs.is_empty()
        || sidecar.is_file()
        || replay::container::session_facts_for(bytes, &replay.identity).is_none()
    {
        return Ok(());
    }
    let staging = sidecar.with_extension("facts.tmp");
    fs::write(&staging, bytes)
        .and_then(|()| fs::rename(&staging, &sidecar))
        .map_err(|error| format!("game facts were not kept for {}: {error}", staged.display()))
}

/// Gives every recording this session published, or noted as its own, the session's game facts
/// when it still lacks them ([`attach_session_facts`]); what could not be given them, with why.
pub fn attach_published_facts(session_dir: &Path, published: &[PathBuf]) -> Vec<String> {
    let mut recordings: BTreeSet<PathBuf> = published.iter().cloned().collect();
    if let Ok(Some(selection)) = selected_recordings(session_dir) {
        recordings.extend(selection.recordings.into_values().flatten());
    }
    recordings
        .into_iter()
        .filter(|recording| recording.is_file())
        .filter_map(|recording| {
            attach_session_facts(session_dir, &recording)
                .err()
                .map(|error| format!("game facts were not added: {error}"))
        })
        .collect()
}

/// Save (or with `unsaved`, stage) a campaign's closed segments; its loads must be decided.
pub fn save_campaign(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    unsaved: bool,
    progress: &dyn Fn() -> std::io::Result<()>,
    ahead: &mut Ahead,
    naming: Naming<'_>,
) -> Result<(Option<PathBuf>, Vec<Omission>)> {
    with_compression(naming.settings, || {
        reconcile_session_checkpoints(session_dir, replay_dir)?;
        let admissions = session_admissions(session_dir)?;
        let witness = session_witness(session_dir)?;
        fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
        let campaign_dir = session_dir.join(campaign);
        let admitted = admissions
            .as_ref()
            .map(|admissions| admissions.loads.get(campaign).cloned().unwrap_or_default());
        let outcome = finalize_campaign(
            &campaign_dir,
            &segment_paths(&campaign_dir)?,
            replay_dir,
            unsaved,
            session_ended(session_dir),
            &mut None,
            witness.as_ref(),
            admitted.as_ref(),
            progress,
            ahead.campaigns.remove(&campaign_dir),
            ahead.checkpoint_reports.remove(&campaign_dir),
            naming,
            &mut ahead.committed,
        )?;
        if let Some(unrenamed) = &outcome.unrenamed {
            println!("{unrenamed}");
        }
        let omitted = admissions.map_or_else(Vec::new, |admissions| {
            outcome
                .omitted
                .into_iter()
                .map(|(load, hours)| admissions.omission(campaign, load, hours))
                .collect()
        });
        Ok((outcome.replay, omitted))
    })
}

/// What one checkpoint saved.
#[derive(Debug)]
pub struct Checkpoint {
    /// The recording it added to.
    pub recording: PathBuf,
    /// Admitted hours it saved.
    pub hours: u64,
    /// Spool segments it deleted, now the recording holds them.
    pub deleted: usize,
}

/// One rollback point for a running session's unsaved copy of a campaign.
const CHECKPOINT_FILE: &str = "checkpoint.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckpointStep {
    Preparing,
    Prepared,
    Written,
    Verified,
    Journaled,
    Watermarked,
    Handed,
    Trimmed,
    Discarded,
    Completed,
    Publishing,
    Published,
}

#[cfg(test)]
thread_local! {
    static CHECKPOINT_STEP: std::cell::RefCell<Option<Box<dyn FnMut(CheckpointStep, &Path) -> std::io::Result<()>>>> = const { std::cell::RefCell::new(None) };
}

fn checkpoint_step(_step: CheckpointStep, _path: &Path) -> Result<()> {
    #[cfg(test)]
    CHECKPOINT_STEP.with(|hook| match hook.borrow_mut().as_mut() {
        Some(hook) => hook(_step, _path).map_err(|error| failure(_path, error)),
        None => Ok(()),
    })?;
    Ok(())
}

struct Checkpointed {
    campaign: String,
    recording: PathBuf,
    before: Option<Revision>,
    after: Option<Revision>,
    source: Revision,
    intended: Revision,
    pending: bool,
    discarding: bool,
    publishing: bool,
}

fn revision_json(revision: Revision) -> Value {
    json!([
        revision.end,
        revision.trailer,
        revision.length,
        revision.crc
    ])
}

fn revision_of(value: &Value) -> Option<Revision> {
    Some(Revision {
        end: value[0].as_u64()?,
        trailer: value[1].as_u64()?,
        length: value[2].as_u64()?,
        crc: u32::try_from(value[3].as_u64()?).ok()?,
    })
}

fn read_checkpointed(campaign_dir: &Path) -> Result<Option<Checkpointed>> {
    let path = campaign_dir.join(CHECKPOINT_FILE);
    let bytes = match read_file(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        bytes => bytes.map_err(|error| failure(&path, error))?,
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| failure(&path, error))?;
    let parse = || -> Option<Checkpointed> {
        let revision = |value: &Value| match value {
            Value::Null => Some(None),
            value => revision_of(value).map(Some),
        };
        Some(Checkpointed {
            campaign: value["campaign"].as_str()?.to_owned(),
            recording: PathBuf::from(value["recording"].as_str()?),
            before: revision(value.get("before")?)?,
            after: revision(value.get("after")?)?,
            source: revision_of(&value["source"])?,
            intended: revision_of(&value["intended"])?,
            pending: value["pending"].as_bool()?,
            discarding: value["discarding"].as_bool()?,
            publishing: value["publishing"].as_bool()?,
        })
    };
    parse()
        .filter(|journal| {
            is_campaign_uuid(journal.campaign.as_bytes())
                && campaign_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(&journal.campaign))
                && is_unsaved_recording(&journal.recording)
                && journal
                    .recording
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(&journal.campaign))
                && (journal.before.is_none() || journal.after.is_some())
                && (!journal.publishing
                    || (!journal.pending && !journal.discarding && journal.after.is_some()))
        })
        .map(Some)
        .ok_or_else(|| failure(&path, "invalid checkpoint journal; spool retained"))
}

fn write_checkpointed(campaign_dir: &Path, checkpointed: &Checkpointed) -> Result<()> {
    use std::io::Write;
    let path = campaign_dir.join(CHECKPOINT_FILE);
    let temporary = path.with_extension("json.tmp");
    let value = json!({
        "campaign": checkpointed.campaign,
        "recording": checkpointed.recording,
        "before": checkpointed.before.map(revision_json),
        "after": checkpointed.after.map(revision_json),
        "source": revision_json(checkpointed.source),
        "intended": revision_json(checkpointed.intended),
        "pending": checkpointed.pending,
        "discarding": checkpointed.discarding,
        "publishing": checkpointed.publishing,
    });
    let bytes = serde_json::to_vec_pretty(&value).map_err(|error| failure(&path, error))?;
    remove_stale_copy(&temporary).map_err(|error| failure(&temporary, error))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| failure(&temporary, error))?;
    Budgeted::new(&mut file)
        .write_all(&bytes)
        .map_err(|error| failure(&temporary, error))?;
    file.sync_all()
        .map_err(|error| failure(&temporary, error))?;
    drop(file);
    replay::publication::replace_file(&temporary, &path).map_err(|error| failure(&path, error))
}

fn checkpoint_folder(recording: &Path) -> Result<&Path> {
    let parent = recording
        .parent()
        .ok_or_else(|| failure(recording, "recording has no folder"))?;
    if is_unsaved_recording(recording) {
        parent
            .parent()
            .ok_or_else(|| failure(recording, "unsaved recording has no folder"))
    } else {
        Ok(parent)
    }
}

fn checkpoint_identity_error(
    path: &Path,
    campaign: &str,
    reason: impl std::fmt::Display,
) -> String {
    failure(
        path,
        format!(
            "checkpoint recording identity for campaign {campaign}: {reason}; spool and journal retained; nothing discarded"
        ),
    )
}

// All callers hold the recording folder's naming lock.
fn resolve_checkpointed(checkpointed: &Checkpointed, locked_folder: &Path) -> Result<()> {
    let original = &checkpointed.recording;
    let identity_error =
        |reason| checkpoint_identity_error(original, &checkpointed.campaign, reason);
    let root = checkpoint_folder(original)?;
    if folder_key(root).map_err(|error| identity_error(error.to_string()))?
        != folder_key(locked_folder).map_err(|error| identity_error(error.to_string()))?
    {
        return Err(identity_error(
            "the recording left the locked folder".into(),
        ));
    }
    let folder = original.parent().unwrap();
    let found = recording_files::unsaved_recording(root, checkpointed.campaign.as_bytes())
        .map_err(|error| identity_error(error.to_string()))?;
    let Some(path) = found else {
        if checkpointed.after.is_some()
            && !(checkpointed.discarding && checkpointed.before.is_none())
        {
            return Err(identity_error("the expected recording is missing".into()));
        }
        return match fs::symlink_metadata(original) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(identity_error(error.to_string())),
            Ok(_) => Err(identity_error(
                "the destination belongs to another recording".into(),
            )),
        };
    };
    if path.parent() != Some(folder) {
        return Err(identity_error(
            "the recording left the unsaved folder".into(),
        ));
    }
    let found = Replay::open_path(&path).map_err(|error| identity_error(error.to_string()))?;
    let revision = Some(found.revision());
    let expected = revision == checkpointed.after
        || (checkpointed.pending
            && [checkpointed.source, checkpointed.intended].contains(&found.revision()))
        || (checkpointed.discarding && revision == checkpointed.before);
    if !expected
        || !found
            .campaign_id
            .eq_ignore_ascii_case(checkpointed.campaign.as_bytes())
    {
        return Err(identity_error(
            "the recording changed since its checkpoint".into(),
        ));
    }
    Ok(())
}

fn session_ended(session_dir: &Path) -> bool {
    ["completed.json", "waiting.json"]
        .iter()
        .any(|name| session_dir.join(name).is_file())
}

fn cleanup_ended_checkpoints(session_dir: &Path, replay_dir: &Path) -> Result<Vec<String>> {
    let mut directories = Vec::new();
    let mut errors = Vec::new();
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        match entry.and_then(|entry| Ok((entry.file_type()?, entry.path()))) {
            Ok((kind, path)) if kind.is_dir() => directories.push(path),
            Ok(_) => {}
            Err(error) => errors.push(failure(session_dir, error)),
        }
    }
    directories.sort();
    let lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    for directory in &directories {
        if let Err(error) = accept_checkpoints(directory) {
            errors.push(error);
        }
    }
    drop(lock);
    for directory in &directories {
        if let Err(error) = drop_checkpointed(directory) {
            errors.push(error);
        }
    }
    Ok(errors)
}

/// End checkpoint ownership only after recovery and the session's durable waiting or completion record.
pub fn write_session_end(
    session_dir: &Path,
    replay_dir: &Path,
    name: &str,
    value: &Value,
) -> Result<()> {
    if !["completed.json", "waiting.json"].contains(&name) {
        return Err(failure(session_dir, "not a session end record"));
    }
    reconcile_session_checkpoints(session_dir, replay_dir)?;
    write_json(&session_dir.join(name), value)?;
    checkpoint_step(CheckpointStep::Completed, session_dir)?;
    reconcile_session_checkpoints(session_dir, replay_dir)
}

// All journal changes and recovery hold the recording folder's naming lock.
fn reconcile_checkpoints(campaign_dir: &Path, replay_dir: &Path) -> Result<()> {
    if campaign_dir.parent().is_some_and(session_ended) {
        return accept_checkpoints(campaign_dir);
    }
    let Some(mut checkpointed) = read_checkpointed(campaign_dir)? else {
        return Ok(());
    };
    if checkpointed.publishing && !checkpointed.recording.exists() {
        let published = recording_files::recording_of(replay_dir, checkpointed.campaign.as_bytes())
            .map_err(|error| {
                checkpoint_identity_error(&checkpointed.recording, &checkpointed.campaign, error)
            })?;
        if let Some(published) = published {
            let found = held_recording(&published)?;
            if Some(found.revision()) == checkpointed.after
                && found
                    .campaign_id
                    .eq_ignore_ascii_case(checkpointed.campaign.as_bytes())
                && found.require_undamaged().is_ok()
            {
                record_recording(
                    campaign_dir.parent().unwrap(),
                    &found.campaign_id,
                    &published,
                )?;
                if let Some(mut saved) = read_saved(campaign_dir) {
                    saved.replay = published;
                    write_saved(campaign_dir, &saved)?;
                }
                return accept_checkpoints(campaign_dir);
            }
        }
    }
    resolve_checkpointed(&checkpointed, replay_dir)?;
    if checkpointed.discarding {
        return finish_discard(campaign_dir, &checkpointed);
    }
    if checkpointed.pending {
        match checkpointed.after {
            Some(after) => Replay::roll_back(&checkpointed.recording, after),
            None => match fs::symlink_metadata(&checkpointed.recording) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
                Ok(_) => replay::publication::posix_delete(&checkpointed.recording),
            },
        }
        .map_err(|error| {
            checkpoint_identity_error(&checkpointed.recording, &checkpointed.campaign, error)
        })?;
        let temporary = stage_copy_path(&checkpointed.recording);
        remove_stale_copy(&temporary).map_err(|error| failure(&temporary, error))?;
        checkpointed.pending = false;
        write_checkpointed(campaign_dir, &checkpointed)?;
    }
    Ok(())
}

pub fn reconcile_session_checkpoints(session_dir: &Path, replay_dir: &Path) -> Result<()> {
    if session_ended(session_dir) {
        for error in cleanup_ended_checkpoints(session_dir, replay_dir)? {
            eprintln!("Saved spool cleanup will be retried: {error}");
        }
        return Ok(());
    }
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if !entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_dir()
        {
            continue;
        }
        let directory = entry.path();
        if let Some(checkpointed) = read_checkpointed(&directory)? {
            let folder = folder_key(replay_dir).map_err(|error| failure(replay_dir, error))?;
            if checkpoint_folder(&checkpointed.recording)
                .and_then(|path| folder_key(path).map_err(|error| failure(path, error)))
                .is_ok_and(|path| path == folder)
            {
                reconcile_checkpoints(&directory, replay_dir)?;
            }
        }
    }
    Ok(())
}

fn begin_checkpoint(
    campaign_dir: &Path,
    recording: &Path,
    before: Option<Revision>,
    source: &Replay,
) -> Result<()> {
    let mut checkpointed = match read_checkpointed(campaign_dir)? {
        Some(checkpointed) => {
            resolve_checkpointed(&checkpointed, checkpoint_folder(recording)?)?;
            if checkpointed.pending
                || checkpointed.after != before
                || checkpointed.recording != recording
            {
                return Err(checkpoint_identity_error(
                    recording,
                    &checkpointed.campaign,
                    "the recording changed since its checkpoint",
                ));
            }
            checkpointed
        }
        None => Checkpointed {
            campaign: String::from_utf8_lossy(&source.campaign_id).into_owned(),
            recording: recording.to_owned(),
            before,
            after: before,
            source: source.revision(),
            intended: source.revision(),
            pending: false,
            discarding: false,
            publishing: false,
        },
    };
    if !is_unsaved_recording(recording)
        || !source
            .campaign_id
            .eq_ignore_ascii_case(checkpointed.campaign.as_bytes())
    {
        return Err(checkpoint_identity_error(
            recording,
            &checkpointed.campaign,
            "source campaign or unsaved destination differs",
        ));
    }
    checkpointed.source = source.revision();
    checkpointed.intended = source.revision();
    checkpointed.pending = true;
    checkpointed.publishing = false;
    checkpoint_step(CheckpointStep::Preparing, recording)?;
    write_checkpointed(campaign_dir, &checkpointed)?;
    checkpoint_step(CheckpointStep::Prepared, recording)
}

fn intend_checkpoint(campaign_dir: &Path, recording: &Path, revision: Revision) -> Result<()> {
    let mut checkpointed = read_checkpointed(campaign_dir)?.ok_or_else(|| {
        failure(
            campaign_dir,
            "checkpoint journal is missing; spool retained",
        )
    })?;
    if checkpointed.recording != recording || !checkpointed.pending {
        return Err(failure(
            recording,
            "pending checkpoint destination is missing; spool retained",
        ));
    }
    checkpointed.intended = revision;
    write_checkpointed(campaign_dir, &checkpointed)
}

fn finish_checkpoint(campaign_dir: &Path, recording: &Path, replay: &Replay) -> Result<()> {
    let mut checkpointed = read_checkpointed(campaign_dir)?.ok_or_else(|| {
        failure(
            campaign_dir,
            "checkpoint journal is missing; spool retained",
        )
    })?;
    if checkpointed.recording != recording
        || !checkpointed.pending
        || checkpointed.intended != replay.revision()
        || !replay
            .campaign_id
            .eq_ignore_ascii_case(checkpointed.campaign.as_bytes())
    {
        return Err(checkpoint_identity_error(
            recording,
            &checkpointed.campaign,
            "the published revision differs",
        ));
    }
    checkpointed.after = Some(replay.revision());
    checkpointed.pending = false;
    write_checkpointed(campaign_dir, &checkpointed)?;
    checkpoint_step(CheckpointStep::Journaled, recording)
}

/// Save the closed segments of a campaign the session records, through segment `through`, into
/// the recording a save at the end would add them to, then delete the segments it now holds.
/// Without `auto_save` they go into the unsaved copy; with it, any waiting copy is published. The newest segment stays until a newer one follows it, so the
/// campaign keeps its spool folder. `None` when there was nothing to save.
#[allow(clippy::too_many_arguments)]
pub fn checkpoint_campaign(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    through: u32,
    auto_save: bool,
    ahead: &mut Ahead,
    naming: Naming<'_>,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<Option<Checkpoint>> {
    with_compression(naming.settings, || {
        reconcile_session_checkpoints(session_dir, replay_dir)?;
        if session_witness(session_dir)?.is_some() {
            return Err(failure(
                session_dir,
                "the capture failed; nothing checkpointed",
            ));
        }
        let campaign_dir = session_dir.join(campaign);
        let admissions = session_admissions(session_dir)?;
        let admitted = admissions
            .as_ref()
            .map(|admissions| admissions.loads.get(campaign).cloned().unwrap_or_default());
        let paths: Vec<_> = segment_paths(&campaign_dir)?
            .into_iter()
            .filter(|(number, _)| *number <= through)
            .collect();
        if paths.is_empty() {
            return Ok(None);
        }
        fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
        let unsaved = !auto_save;
        let mut retained = None;
        let taken = match ahead.campaigns.remove(&campaign_dir) {
            Some(mut campaign)
                if campaign.written.is_some()
                    && campaign.tails.keys().any(|number| *number > through) =>
            {
                let prefix = campaign
                    .written
                    .as_mut()
                    .map(|(written, _)| written.take_completed(through))
                    .transpose()
                    .map_err(|error| failure(&campaign_dir, error))?
                    .flatten();
                let Some(prefix) = prefix else {
                    ahead.campaigns.insert(campaign_dir.clone(), campaign);
                    return Ok(None);
                };
                let tails = campaign
                    .tails
                    .range(..=through)
                    .map(|(number, tail)| (*number, tail.clone()))
                    .collect();
                let mut summary = Box::<Summary>::default();
                summary.multiplayer = prefix.multiplayer();
                let taken = Box::new(CampaignAhead {
                    saved: campaign.saved.clone(),
                    tails,
                    written: Some((prefix, summary)),
                    ..Default::default()
                });
                retained = Some(campaign);
                Some(taken)
            }
            taken => taken,
        };
        let committed = &mut ahead.committed;
        let mut hand = |recording: &Path, replay: &Replay, blocks: &[BlockIndex]| {
            if let Some(committed) = committed {
                committed(recording, replay, blocks);
            }
        };
        let result = finalize_campaign_with_publisher(
            &campaign_dir,
            &paths,
            replay_dir,
            unsaved,
            &mut None,
            None,
            admitted.as_ref(),
            progress,
            taken,
            naming,
            true,
            |replay, destination| replay.write_atomic_with(destination, |_| progress()),
            // The collector holds the recording until the session ends; the save then names it.
            |recording, _| Ok(recording.to_owned()),
            &mut hand,
        );
        if let Some(mut campaign) = retained {
            let must_adopt = result
                .as_ref()
                .is_ok_and(|outcome| outcome.replay.is_some());
            let recording = result
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.replay.clone())
                .or_else(|| read_saved(&campaign_dir).map(|saved| saved.replay));
            let adopted = (|| {
                let recording = recording.ok_or_else(|| {
                    failure(&campaign_dir, "commit has no durable spool watermark")
                })?;
                let replay = held_recording(&recording)?;
                let saved = read_saved(&campaign_dir).ok_or_else(|| {
                    failure(
                        &campaign_dir,
                        "verified commit has no durable spool watermark",
                    )
                })?;
                if saved.revision != replay.revision() {
                    return Err(failure(
                        &campaign_dir,
                        "spool watermark differs from verified revision",
                    ));
                }
                if let Some((written, _)) = &mut campaign.written {
                    written
                        .committed(replay, saved.revision)
                        .map_err(|error| failure(&campaign_dir, error))?;
                }
                campaign.saved.extend(saved.segments.keys().copied());
                campaign.command_bootstrap = saved.command_bootstrap;
                campaign.tails.retain(|number, _| *number > through);
                campaign.through = None;
                Ok(())
            })();
            if adopted.is_err()
                && let Some((written, _)) = &mut campaign.written
            {
                written.abort_commit();
            }
            ahead.campaigns.insert(campaign_dir.clone(), campaign);
            if must_adopt {
                adopted?;
            }
        }
        let mut outcome = result?;
        let Some(recording) = outcome.replay else {
            return Ok(None);
        };
        let hours = outcome.covered.values().map(|(_, hours, _)| hours).sum();
        if let Some(report) = merge_reports(
            ahead.checkpoint_reports.remove(&campaign_dir),
            outcome.report.take(),
            &held_recording(&recording)?,
        )? {
            ahead
                .checkpoint_reports
                .insert(campaign_dir.clone(), report);
        }
        let deleted = drop_checkpointed(&campaign_dir)?;
        Ok(Some(Checkpoint {
            recording,
            hours,
            deleted,
        }))
    })
}

/// Delete verified saved segments, keeping the newest while live and every waiting segment once ended.
pub fn drop_checkpointed(campaign_dir: &Path) -> Result<usize> {
    let Some(saved) = read_saved(campaign_dir) else {
        return Ok(0);
    };
    let ended = campaign_dir.parent().is_some_and(session_ended);
    if ended && (is_unsaved_recording(&saved.replay) || !finished_campaign(campaign_dir)?) {
        return Ok(0);
    }
    let folder = checkpoint_folder(&saved.replay)?;
    let _lock = NamesLock::wait(folder).map_err(|error| failure(folder, error))?;
    if ended {
        if campaign_dir
            .join(CHECKPOINT_FILE)
            .try_exists()
            .map_err(|error| failure(campaign_dir, error))?
        {
            return Ok(0);
        }
    } else {
        if let Some(checkpointed) = read_checkpointed(campaign_dir)? {
            if checkpointed.pending {
                return Ok(0);
            }
            resolve_checkpointed(&checkpointed, folder)?;
        }
    }
    let paths = segment_paths(campaign_dir)?;
    let newest = paths.last().map(|(number, _)| *number);
    let saved_paths: Vec<_> = paths
        .iter()
        .filter(|(number, path)| {
            (ended || Some(*number) != newest)
                && saved.segments.get(number).is_some_and(|bytes| {
                    fs::metadata(path).is_ok_and(|metadata| metadata.len() == *bytes)
                })
        })
        .collect();
    let campaign = campaign_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let recording = if is_unsaved_recording(&saved.replay) {
        recording_files::unsaved_recording(folder, campaign.as_bytes())
    } else {
        recording_files::recording_of(folder, campaign.as_bytes())
    }
    .map_err(|error| failure(&saved.replay, error))?;
    if saved_paths.is_empty()
        || recording
            .as_deref()
            .and_then(|path| Replay::open_path(path).ok())
            .is_none_or(|recording| {
                !recording.contains_revision(saved.revision).unwrap_or(false)
                    || !recording
                        .campaign_id
                        .eq_ignore_ascii_case(campaign.as_bytes())
                    || recording.require_undamaged().is_err()
            })
    {
        return Ok(0);
    }
    for (_, path) in &saved_paths {
        fs::remove_file(path).map_err(|error| failure(path, error))?;
        checkpoint_step(CheckpointStep::Trimmed, path)?;
    }
    Ok(saved_paths.len())
}

fn finished_campaign(campaign_dir: &Path) -> Result<bool> {
    let session = campaign_dir.parent().unwrap();
    let Some(path) = ["completed.json", "waiting.json"]
        .into_iter()
        .map(|name| session.join(name))
        .find(|path| path.is_file())
    else {
        return Ok(false);
    };
    let ended: Value =
        serde_json::from_slice(&read_file(&path).map_err(|error| failure(&path, error))?)
            .map_err(|error| failure(&path, error))?;
    let campaign = campaign_dir.file_name().and_then(|name| name.to_str());
    Ok(ended.is_object()
        && match ended.get("waiting") {
            Some(waiting) => waiting.as_array().is_some_and(|waiting| {
                waiting.iter().all(|entry| {
                    entry["campaign"]
                        .as_str()
                        .is_some_and(|waiting| Some(waiting) != campaign)
                })
            }),
            None => path
                .file_name()
                .is_some_and(|name| name == "completed.json"),
        })
}

/// Roll back only this running session's unsaved copy; published hours are final.
pub fn discard_checkpoints(session_dir: &Path, campaign: &str) -> Result<()> {
    let campaign_dir = session_dir.join(campaign);
    if session_ended(session_dir) {
        return accept_checkpoints(&campaign_dir);
    }
    let Some(checkpointed) = read_checkpointed(&campaign_dir)? else {
        return Ok(());
    };
    let folder = checkpoint_folder(&checkpointed.recording)?;
    let _lock = NamesLock::wait(folder).map_err(|error| failure(folder, error))?;
    reconcile_checkpoints(&campaign_dir, folder)?;
    let Some(mut checkpointed) = read_checkpointed(&campaign_dir)? else {
        return Ok(());
    };
    checkpointed.publishing = false;
    checkpointed.discarding = true;
    write_checkpointed(&campaign_dir, &checkpointed)?;
    finish_discard(&campaign_dir, &checkpointed)
}

fn accept_checkpoints(campaign_dir: &Path) -> Result<()> {
    let path = campaign_dir.join(CHECKPOINT_FILE);
    match replay::publication::posix_delete(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(failure(&path, error)),
        _ => Ok(()),
    }
}

fn finish_discard(campaign_dir: &Path, checkpointed: &Checkpointed) -> Result<()> {
    let campaign = &checkpointed.campaign;
    let recording = &checkpointed.recording;
    match Replay::open_path(recording) {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound && checkpointed.before.is_none() => {}
        Err(error) => return Err(checkpoint_identity_error(recording, campaign, error)),
        Ok(found) => {
            if (Some(found.revision()) != checkpointed.after
                && Some(found.revision()) != checkpointed.before)
                || !found.campaign_id.eq_ignore_ascii_case(campaign.as_bytes())
            {
                return Err(checkpoint_identity_error(
                    recording,
                    campaign,
                    "the recording changed since this session saved into it",
                ));
            }
            drop(found);
            match checkpointed.before {
                Some(before) => Replay::roll_back(recording, before),
                None => replay::publication::posix_delete(recording),
            }
            .map_err(|error| failure(recording, error))?;
            checkpoint_step(CheckpointStep::Discarded, recording)?;
        }
    }
    // Keep Discard's intent until no spool can resurrect the rolled-back hours after a crash.
    for (_, path) in segment_paths(campaign_dir)? {
        replay::publication::posix_delete(&path).map_err(|error| failure(&path, error))?;
        checkpoint_step(CheckpointStep::Trimmed, &path)?;
    }
    let saved = campaign_dir.join(SAVED_FILE);
    match replay::publication::posix_delete(&saved) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(failure(&saved, error));
        }
        _ => {}
    }
    accept_checkpoints(campaign_dir)
}

/// Publish one campaign's spool (with `admitted`, only those loads) into its recording and name
/// it, or with `unsaved`, into its unsaved recording.
#[allow(clippy::too_many_arguments)]
fn finalize_campaign(
    campaign_dir: &Path,
    paths: &[(u32, PathBuf)],
    replay_dir: &Path,
    unsaved: bool,
    ended: bool,
    process_context: &mut Option<(SessionId, u32, u64, u64)>,
    witness: Option<&TerminalWitness>,
    admitted: Option<&BTreeSet<i64>>,
    progress: &dyn Fn() -> std::io::Result<()>,
    ahead: Option<Box<CampaignAhead>>,
    checkpointed: Option<Box<FinalizationReport>>,
    naming: Naming<'_>,
    committed: &mut Option<Box<Committed<'static>>>,
) -> Result<CampaignOutcome> {
    // Before the recording that records their sessions' ends, so a reader finds them with it.
    if !unsaved {
        publish_columnar(campaign_dir, replay_dir, admitted, ended)?;
    }
    let mut hand = |recording: &Path, replay: &Replay, blocks: &[BlockIndex]| {
        if let Some(committed) = committed {
            committed(recording, replay, blocks);
        }
    };
    let mut outcome = finalize_campaign_with_publisher(
        campaign_dir,
        paths,
        replay_dir,
        unsaved,
        process_context,
        witness,
        admitted,
        progress,
        ahead,
        naming,
        false,
        |replay, destination| replay.write_atomic_with(destination, |_| progress()),
        recording_files::rename,
        &mut hand,
    )?;
    if let Some(recording) = &outcome.replay {
        let replay = held_recording(recording)?;
        let mut report =
            merge_reports(checkpointed, outcome.report.take(), &replay)?.unwrap_or_default();
        println!(
            "Finalization evidence: {} known hours; historical timings not reconstructed.",
            report
                .summary
                .hours
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
        );
        let mut lengths =
            read_saved(campaign_dir).map_or_else(BTreeMap::new, |saved| saved.segments);
        for (number, path) in paths {
            lengths.insert(
                *number,
                fs::metadata(path)
                    .map_err(|error| failure(path, error))?
                    .len(),
            );
        }
        report.spool_bytes = lengths.values().sum();
        emit_finalization(&report.text(replay.saved_length())?);
        println!(
            "gaps={} damaged_segments={}",
            replay.gaps.len(),
            replay.recovery_notes.len()
        );
        println!(
            "Detailed capture statistics and gap controls are available through inspect, not scanned at Stop."
        );
    }
    if !unsaved && campaign_dir.parent().is_some_and(session_ended) {
        if let Err(error) = drop_checkpointed(campaign_dir) {
            eprintln!("Saved spool cleanup will be retried: {error}");
        }
    }
    Ok(outcome)
}

/// Publishes the campaign's finished columnar recordings, one per capture, in capture order:
/// each as its session's next in the campaign's folder of `replay_dir` ([`campaign_folder`]).
/// A partial one no writer holds is finished from its last revision first; one with none is
/// reported and left, and one its collector still writes holds back the rest. With `admitted`,
/// the loads a Battle Plan Recorder session admitted, each holds only the branches of loads it did not
/// refuse, as the save holds them ([`keep_admitted`]); one holding none is left unpublished. Any
/// other failure stops there, so a session's captures publish in order, and fails the save: its
/// recording would end sessions whose captures a reader cannot find. A retry resumes. With
/// `ended`, no collector of the session writes any more, so one held open fails the save too.
///
/// [`campaign_folder`]: replay::columnar_file::campaign_folder
fn publish_columnar(
    campaign_dir: &Path,
    replay_dir: &Path,
    admitted: Option<&BTreeSet<i64>>,
    ended: bool,
) -> Result<()> {
    let held = publish_captures(campaign_dir, replay_dir, admitted).map_err(|error| {
        failure(
            campaign_dir,
            format!("columnar recordings not published: {error}; spool retained"),
        )
    })?;
    match held {
        Some(partial) if ended => Err(failure(
            &partial,
            "held open, so this ended session's save waits for it; spool retained",
        )),
        _ => Ok(()),
    }
}

/// Whether a campaign's spool still holds native captures its recording has not published.
pub fn unpublished_captures(campaign_dir: &Path) -> Result<bool> {
    let entries = match fs::read_dir(campaign_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        entries => entries.map_err(|error| failure(campaign_dir, error))?,
    };
    for entry in entries {
        let entry = entry.map_err(|error| failure(campaign_dir, error))?;
        if replay::columnar_file::capture_of(&entry.file_name()).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn publish_captures(
    campaign_dir: &Path,
    replay_dir: &Path,
    admitted: Option<&BTreeSet<i64>>,
) -> std::io::Result<Option<PathBuf>> {
    use replay::columnar_file::{
        CAPTURE_NAME_BYTES, campaign_folder, capture_name, capture_of, published_name,
        same_recording,
    };
    let (Some(session_dir), Some(campaign)) = (campaign_dir.parent(), campaign_dir.file_name())
    else {
        return Ok(None);
    };
    let Some(session) = session_dir.file_name() else {
        return Ok(None);
    };
    let entries = match fs::read_dir(campaign_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        entries => entries?,
    };
    let mut captures = Vec::new();
    for entry in entries {
        if let Some(capture) = capture_of(&entry?.file_name()) {
            captures.push(capture);
        }
    }
    if captures.is_empty() {
        return Ok(None);
    }
    captures.sort_unstable();
    let spooled = admitted
        .map(|_| read_branch_loads(campaign_dir))
        .transpose()
        .map_err(std::io::Error::other)?;
    let folder = campaign_folder(replay_dir, campaign.as_encoded_bytes());
    let (published, mut next) = published_captures(&folder, session)?;
    let mut name = [0; CAPTURE_NAME_BYTES];
    'captures: for (n, partial) in captures {
        let finished = campaign_dir.join(capture_name(n, false, &mut name));
        if partial {
            let partial = campaign_dir.join(capture_name(n, true, &mut name));
            match finish_unfinished_columnar(&partial, &finished) {
                Ok(true) => {}
                Ok(false) => return Ok(Some(partial)),
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    eprintln!(
                        "Columnar recording {} not recovered: {error}",
                        partial.display()
                    );
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        if let (Some(admitted), Some(spooled)) = (admitted, spooled.as_ref()) {
            match keep_admitted(&finished, campaign_dir, replay_dir, spooled, admitted) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    eprintln!(
                        "Columnar recording {} not published without its refused loads: {error}",
                        finished.display()
                    );
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        // A move across volumes stopped after its copy landed: the capture is published.
        for copy in &published {
            if same_recording(copy, &finished)? {
                fs::remove_file(&finished)?;
                continue 'captures;
            }
        }
        fs::create_dir_all(&folder)?;
        let target = folder.join(published_name(session, next));
        move_published(&finished, &target)?;
        println!("Columnar recording: {}", target.display());
        next = next
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("a session's captures pass 2^32"))?;
    }
    Ok(None)
}

/// The captures of `session` that `folder` holds, and the number after every one.
fn published_captures(
    folder: &Path,
    session: &std::ffi::OsStr,
) -> std::io::Result<(Vec<PathBuf>, u32)> {
    let entries = match fs::read_dir(folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        entries => entries?,
    };
    let (mut published, mut next) = (Vec::new(), 0_u32);
    for entry in entries {
        let entry = entry?;
        if let Some((held, number)) = replay::columnar_file::published_of(&entry.file_name())
            && session.to_str() == Some(held)
        {
            next = next.max(
                number
                    .checked_add(1)
                    .ok_or_else(|| std::io::Error::other("a session's captures pass 2^32"))?,
            );
            published.push(entry.path());
        }
    }
    Ok((published, next))
}

/// Moves `source` to `target`; across volumes, copies it beside `target`, then publishes it by
/// name.
fn move_published(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target).or_else(|_| {
        let mut staged = target.as_os_str().to_os_string();
        staged.push(".partial");
        let staged = PathBuf::from(staged);
        let copied = fs::copy(source, &staged)
            .and_then(|_| replay::publication::sync_file(&staged))
            .and_then(|_| fs::rename(&staged, target));
        if copied.is_err() {
            let _ = fs::remove_file(&staged);
        }
        copied.and_then(|_| fs::remove_file(source))
    })
}

/// Finishes the columnar recording at `partial` an ended collector left, at the last revision
/// it published, the capture recording that its hours stop there, and names it `finished`; not
/// while its collector still writes it. One with no revision holding a capture and an hour or
/// command is refused as invalid data.
fn finish_unfinished_columnar(partial: &Path, finished: &Path) -> std::io::Result<bool> {
    use replay::columnar_file::{Settings, Stop, Unfinished};
    let Some(mut file) = open_unwritten(partial)? else {
        return Ok(false);
    };
    let unfinished = Unfinished::read(&mut file)?;
    let Some(mut capture) = unfinished.capture.clone() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "its last revision holds no capture",
        ));
    };
    let mut branch = unfinished.last.map(|(branch, _)| branch);
    if branch.is_none() {
        for bytes in &capture.controls {
            if let Record::Control(ControlRecord::PlayerCommands { packet, .. }) =
                replay::spool::decode_record(bytes)?
            {
                branch = Some(packet.branch);
            }
        }
    }
    let branch = branch.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "its last revision holds no hours or commands",
        )
    })?;
    let last_hour = unfinished.last.map(|(_, hour)| hour);
    capture.stop = Some(Stop {
        branch,
        last_hour,
        reason: "the collector ended before finishing this file".into(),
    });
    unfinished.finish(
        &mut file,
        |file, length| file.set_len(length),
        |file| file.sync_data(),
        &capture,
        Settings::RECORDING,
    )?;
    drop(file);
    fs::rename(partial, finished)?;
    if let Some(hour) = last_hour {
        println!("Columnar recording recovered through hour {hour}");
    } else {
        println!("Columnar command recording recovered");
    }
    Ok(true)
}

/// Checkpointed phase and saved-recording evidence still establish a capture's admitted loads.
fn keep_admitted(
    finished: &Path,
    campaign_dir: &Path,
    replay_dir: &Path,
    spooled: &BranchLoads,
    admitted: &BTreeSet<i64>,
) -> std::io::Result<bool> {
    use replay::columnar_frames::{Limits, Recording};
    let native = Recording::open_path(finished, Limits::default())?;
    let refused = capture_refused_loads(&native, campaign_dir, replay_dir, spooled, admitted)?;
    if refused.is_empty() {
        return Ok(true);
    }
    let staged = replay::publication::temporary_beside(finished, "admitted");
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&staged)?;
    let kept = replay::columnar_file::keep_branches(
        &native,
        |branch| !refused.contains(&branch),
        file,
        replay::columnar_file::Settings::RECORDING,
        |file| file.sync_data(),
    );
    drop(native);
    let placed = match kept {
        Ok(Some(written)) => {
            let synced = written.file.sync_all();
            drop(written);
            synced
                .and_then(|()| fs::rename(&staged, finished))
                .map(|()| true)
        }
        Ok(None) => Ok(false),
        Err(error) => Err(error),
    };
    if !matches!(placed, Ok(true)) {
        let _ = fs::remove_file(&staged);
    }
    placed
}

fn capture_refused_loads(
    native: &replay::columnar_frames::Recording,
    campaign_dir: &Path,
    replay_dir: &Path,
    spooled: &BranchLoads,
    admitted: &BTreeSet<i64>,
) -> std::io::Result<BTreeSet<BranchId>> {
    let invalid = |reason| std::io::Error::new(std::io::ErrorKind::InvalidData, reason);
    let capture = native
        .capture()?
        .ok_or_else(|| invalid("columnar capture metadata is absent"))?;
    let root = native.root_identity();
    if !root.campaign.eq_ignore_ascii_case(&capture.campaign_id)
        || campaign_dir
            .file_name()
            .is_none_or(|name| !root.campaign.eq_ignore_ascii_case(name.as_encoded_bytes()))
        || root.content != capture.content
        || root.capture_mode != capture.capture_mode
        || capture.run_sessions.len() != native.run_count() as usize
    {
        return Err(invalid(
            "columnar capture identity differs from its recording",
        ));
    }
    let run_sessions = engine_allocation::copy_vec(&capture.run_sessions)?;
    let metadata = replay::commands::Recording::from_columnar(capture)?;
    let mut branches: BTreeMap<_, _> = metadata
        .branches
        .iter()
        .map(|start| (start.id, start.session))
        .collect();
    for run in 0..native.run_count() {
        let session = metadata
            .sessions
            .get(run_sessions[run as usize] as usize)
            .ok_or_else(|| invalid("columnar run session is absent"))?
            .id;
        let branch = native.run(run)?.branch;
        if branches
            .insert(branch, session)
            .is_some_and(|previous| previous != session)
        {
            return Err(invalid("columnar branch has conflicting sessions"));
        }
    }
    let mut known = BranchLoads::default();
    for control in &metadata.controls {
        known.observe(RecordRef::Control(control));
    }
    known.restore_bootstrap(&spooled.command_bootstrap);
    for (&branch, &session) in &branches {
        if spooled
            .sessions
            .get(&branch)
            .is_some_and(|previous| *previous != session)
        {
            return Err(invalid("columnar and spool branch sessions differ"));
        }
        if let Some(&load) = spooled.loads.get(&branch) {
            if known
                .loads
                .insert(branch, load)
                .is_some_and(|previous| previous != load)
            {
                return Err(invalid("columnar and spool branch loads differ"));
            }
        }
    }
    let missing: Vec<_> = branches
        .keys()
        .filter(|branch| !known.loads.contains_key(*branch))
        .copied()
        .collect();
    if !missing.is_empty() {
        let _lock = NamesLock::wait(replay_dir).map_err(std::io::Error::other)?;
        let name = campaign_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("campaign folder has no name"))?;
        let sessions: Vec<_> = metadata.sessions.iter().map(|session| session.id).collect();
        let base = save_base(
            campaign_dir.parent().unwrap_or(campaign_dir),
            replay_dir,
            name,
            &sessions,
        )
        .map_err(std::io::Error::other)?;
        if let Some(base) = base {
            let saved = held_recording(&base).map_err(std::io::Error::other)?;
            saved.require_undamaged()?;
            if !saved.campaign_id.eq_ignore_ascii_case(&root.campaign)
                || metadata
                    .sessions
                    .iter()
                    .any(|session| !saved.identity.same_content(&session.identity))
            {
                return Err(invalid("columnar and saved recording content differs"));
            }
            for branch in missing {
                if let Some(start) = saved.branches.iter().find(|start| start.id == branch) {
                    if start.session != branches[&branch]
                        || metadata
                            .sessions
                            .iter()
                            .filter(|session| session.id == start.session)
                            .any(|session| !saved.sessions.contains(session))
                    {
                        return Err(invalid("columnar and saved branch sessions differ"));
                    }
                    if let Some(load) =
                        held_signal(&saved, branch).map_err(std::io::Error::other)?
                    {
                        known.loads.insert(branch, load);
                    }
                }
            }
        }
    }
    Ok(branches
        .keys()
        .filter(|branch| {
            !known
                .loads
                .get(*branch)
                .is_some_and(|load| admitted.contains(load))
        })
        .copied()
        .collect())
}

/// The campaign's unsaved recording, found as the collector finds its history; an ambiguous one is refused.
fn staged_recording(replay_dir: &Path, campaign: &str) -> Result<Option<PathBuf>> {
    let staged = replay::publication::unsaved_path(replay_dir, campaign);
    match recording_files::unsaved_recording(replay_dir, campaign.as_bytes()) {
        Ok(found) => Ok(found.map(|_| staged)),
        Err(error) => Err(failure(
            replay_dir,
            format!("{error}; nothing saved; spool retained"),
        )),
    }
}

/// What a save adds to: the campaign's unsaved recording when it has one, else the recording it continues.
fn save_base(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    sessions: &[u64],
) -> Result<Option<PathBuf>> {
    match staged_recording(replay_dir, campaign)? {
        Some(staged) => Ok(Some(staged)),
        None => continued_recording(session_dir, replay_dir, campaign.as_bytes(), sessions),
    }
}

/// The session a campaign's spool records: every segment begins with its context.
fn spool_session(paths: &[(u32, PathBuf)]) -> Vec<u64> {
    let first = || {
        let (_, first) = paths.first()?;
        let file = fs::File::open(first).ok()?;
        let length = usize::try_from(file.metadata().ok()?.len()).ok()?;
        match RecordReader::new(Budgeted::new(file), length).next_record() {
            Ok(Some(Record::SessionStart(session))) => Some(session.id),
            Ok(Some(Record::BranchStart(branch))) => Some(branch.session),
            _ => None,
        }
    };
    first().into_iter().collect()
}

/// The name a save gives a recording of these contents.
fn recording_stem(replay: &Replay, naming: Naming<'_>, multiplayer: bool) -> Result<String> {
    let _phase = replay::container::Packing::phase(replay::container::Phase::Publication);
    let name = crate::recording_name::recording_name(replay, naming.localisation, multiplayer)
        .map_err(|error| error.to_string())?;
    if let Some(report) = naming.report {
        report(name.by_tag.as_deref());
    }
    Ok(name.stem())
}

/// Whether a save keeps the name of the recording it continues: its contents give it, or the user chose it.
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

/// Each recording a save adds to, by its size and time, and what names it.
static CONTENTS: std::sync::Mutex<BTreeMap<PathBuf, (u64, std::time::SystemTime, Contents)>> =
    std::sync::Mutex::new(BTreeMap::new());

/// What names the recording a save adds to, read once per version of it without loading it whole.
fn saved_contents(path: &Path, campaign: &str) -> std::io::Result<Contents> {
    let stamp = roster_stamp(path)?;
    let (key, length, modified) = stamp.clone();
    let known = CONTENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .filter(|(known_length, known_modified, _)| {
            (*known_length, *known_modified) == (length, modified)
        })
        .map(|(_, _, contents)| contents.clone());
    if let Some(contents) = known {
        return Ok(contents);
    }
    let replay = Replay::open_path(path)?;
    replay.require_undamaged()?;
    if !replay.campaign_id.eq_ignore_ascii_case(campaign.as_bytes()) {
        return Err(std::io::Error::other(
            "existing replay belongs to a different campaign",
        ));
    }
    let contents = Contents::for_prediction(&replay)?;
    drop(replay);
    // Replaced while read, it is read again next time.
    if roster_stamp(path)? != stamp {
        return Err(std::io::Error::other(
            "the recording changed while it was read",
        ));
    }
    CONTENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (length, modified, contents.clone()));
    Ok(contents)
}

/// The file name a save gives the campaign's recording, found and placed as the save does, with
/// `added` appended; `None` when that save would fail or part of its name cannot be read.
fn predicted_name(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    sessions: &[u64],
    added: Option<Contents>,
    naming: Naming<'_>,
) -> Option<String> {
    let existing =
        continued_recording(session_dir, replay_dir, campaign.as_bytes(), sessions).ok()?;
    let staged = staged_recording(replay_dir, campaign).ok()?;
    if let (Some(staged), Some(existing)) = (&staged, &existing)
        && !staged_holds(staged, existing)
    {
        return None;
    }
    let contents = match staged.or_else(|| existing.clone()) {
        Some(base) => {
            let base = saved_contents(&base, campaign).ok()?;
            match added {
                Some(added) => base.then(added),
                None => base,
            }
        }
        None => added?,
    };
    let predicted = contents.predicted(naming.localisation)?;
    if let Some(report) = naming.report {
        report(predicted.by_tag.as_deref());
    }
    let stem = predicted.stem();
    let folder = fs::canonicalize(replay_dir).ok()?;
    let settings = RecorderSettings::load_from(naming.settings).ok()?;
    let path = match existing {
        Some(existing) if keeps_name(&existing, &stem, &settings, &folder, campaign.as_bytes()) => {
            existing
        }
        existing => recording_files::unique_path(replay_dir, &stem, existing.as_deref()).ok()?,
    };
    path.file_stem()?.to_str().map(str::to_owned)
}

type FileStamp = (PathBuf, u64, std::time::SystemTime);

static CONTAINMENT: std::sync::Mutex<BTreeMap<PathBuf, (FileStamp, FileStamp, bool)>> =
    std::sync::Mutex::new(BTreeMap::new());

fn staged_holds(staged: &Path, existing: &Path) -> bool {
    let proof = || -> std::io::Result<bool> {
        let staged_stamp = roster_stamp(staged)?;
        let existing_stamp = roster_stamp(existing)?;
        let key = staged_stamp.0.clone();
        if let Some((_, _, holds)) = CONTAINMENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .filter(|(a, b, _)| *a == staged_stamp && *b == existing_stamp)
        {
            return Ok(*holds);
        }
        let holds = Replay::open_path(staged)?.holds(existing)?;
        if roster_stamp(staged)? != staged_stamp || roster_stamp(existing)? != existing_stamp {
            return Ok(false);
        }
        CONTAINMENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, (staged_stamp, existing_stamp, holds));
        Ok(holds)
    };
    proof().unwrap_or(false)
}

#[derive(Clone, PartialEq, Eq)]
struct StagedStamp {
    staged: FileStamp,
    journal: FileStamp,
    admissions: Option<FileStamp>,
    marker: Option<FileStamp>,
    witness: Option<FileStamp>,
    segments: Vec<(u32, FileStamp)>,
}

fn staged_stamp(campaign_dir: &Path, staged: &Path) -> Option<StagedStamp> {
    let session = campaign_dir.parent()?;
    Some(StagedStamp {
        staged: roster_stamp(staged).ok()?,
        journal: roster_stamp(&campaign_dir.join(SAVED_FILE)).ok()?,
        admissions: roster_stamp(&session.join(ADMISSIONS_FILE)).ok(),
        marker: roster_stamp(&session.join("bin").join(ADMISSION_MARKER)).ok(),
        witness: roster_stamp(&session.join(WITNESS_FILE_NAME)).ok(),
        segments: segment_paths(campaign_dir)
            .ok()?
            .into_iter()
            .map(|(number, path)| Some((number, roster_stamp(&path).ok()?)))
            .collect::<Option<_>>()?,
    })
}

static STAGED: std::sync::Mutex<BTreeMap<PathBuf, (StagedStamp, bool)>> =
    std::sync::Mutex::new(BTreeMap::new());

/// Verify the current staged revision as Save does, including exact contents after later appends.
fn staged_whole(campaign_dir: &Path, staged: &Path) -> bool {
    let Some(stamp) = staged_stamp(campaign_dir, staged) else {
        return false;
    };
    let Ok(key) = fs::canonicalize(campaign_dir) else {
        return false;
    };
    if let Some((_, whole)) = STAGED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .filter(|(known, _)| *known == stamp)
    {
        return *whole;
    }
    let whole = verify_staged(campaign_dir, staged, &stamp).unwrap_or(false);
    if staged_stamp(campaign_dir, staged).as_ref() != Some(&stamp) {
        return false;
    }
    STAGED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (stamp, whole));
    whole
}

fn verify_staged(campaign_dir: &Path, staged: &Path, stamp: &StagedStamp) -> Option<bool> {
    let saved = read_saved(campaign_dir)?;
    let end = SpoolEnd::of(campaign_dir).ok()?;
    if !replay::recorder_settings::same_path(&saved.replay, staged)
        || stamp.segments.iter().any(|(number, (path, length, _))| {
            !matches!(
                end.unsaved(path, *number, *length, saved.segments.get(number).copied()),
                Ok(None)
            )
        })
    {
        return Some(false);
    }
    let campaign = campaign_dir.file_name()?.to_str()?;
    let previous = Replay::open_path(staged).ok()?;
    if !previous
        .campaign_id
        .eq_ignore_ascii_case(campaign.as_bytes())
    {
        return Some(false);
    }
    previous.contains_revision(saved.revision).ok()
}

/// The file name saving the hours ended `sessions` (oldest first) left waiting gives the recording;
/// known only while its unsaved recording holds them all, so their spools add nothing.
pub fn waiting_name(
    sessions: &[PathBuf],
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Option<String> {
    let staged = staged_recording(replay_dir, campaign).ok()??;
    if !sessions
        .iter()
        .all(|session| staged_whole(&session.join(campaign), &staged))
    {
        return None;
    }
    let first = sessions.first()?;
    let spooled = spool_session(&segment_paths(&first.join(campaign)).ok()?);
    predicted_name(first, replay_dir, campaign, &spooled, None, naming)
}

/// Where a saved recording's bytes are before they become its file.
#[derive(Clone, Copy)]
enum Bytes<'a> {
    /// In memory only.
    New,
    /// The unsaved recording holds them.
    Staged(&'a Path),
    /// The recording it continues holds them, added to in place.
    Unchanged,
}

/// Make `replay` the campaign's recording: the one it continues, renamed once durable unless the
/// user chose its name, or a new one under the first free name, never replacing another file.
/// Returns the recording, and why it kept a name its contents no longer give.
#[allow(clippy::too_many_arguments)]
fn save_recording_file(
    replay: &Replay,
    bytes: Bytes<'_>,
    replay_dir: &Path,
    existing: Option<&Path>,
    stem: &str,
    naming: Naming<'_>,
    publish: impl FnOnce(&Replay, &Path) -> std::io::Result<()>,
    rename: impl FnOnce(&Path, &str) -> std::io::Result<PathBuf>,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<(PathBuf, Option<String>)> {
    let id = &replay.campaign_id;
    let folder = fs::canonicalize(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let settings = RecorderSettings::load_from(naming.settings)
        .map_err(|error| failure(naming.settings, error))?;
    let Some(destination) = existing else {
        // A new recording is named by its contents until the user names it.
        if settings.name_chosen(&folder, id) {
            RecorderSettings::update_at(naming.settings, |settings| {
                settings.choose_name(&folder, &String::from_utf8_lossy(id), false)
            })
            .map_err(|error| failure(naming.settings, error))?;
        }
        let published = match bytes {
            Bytes::New | Bytes::Unchanged => {
                recording_files::publish_new(replay, replay_dir, stem, |_| progress())
            }
            Bytes::Staged(staged) => recording_files::place_new(staged, replay_dir, stem),
        }
        .map_err(|error| failure(replay_dir, error))?;
        progress().map_err(|error| failure(&published, error))?;
        return Ok((published, None));
    };
    // Only a recording it holds whole: one game process gives each capture it makes the same session.
    if let Bytes::Staged(_) = bytes
        && !replay.holds(destination).map_err(|error| {
            failure(
                destination,
                format!("{error}; nothing saved; spool retained"),
            )
        })?
    {
        return Err(failure(
            destination,
            "the unsaved recording does not hold all of this recording unchanged; nothing saved; spool retained",
        ));
    }
    match bytes {
        Bytes::New => publish(replay, destination),
        // Added to where it waits, then moved: a crash never leaves it older than the saved recording.
        Bytes::Staged(staged) => {
            replay::publication::replace_file(staged, destination).map_err(std::io::Error::from)
        }
        // Matching bytes can survive a rename whose durability flush failed.
        Bytes::Unchanged => replay::publication::sync_file(destination),
    }
    .map_err(|error| failure(destination, error))?;
    progress().map_err(|error| failure(destination, error))?;
    if keeps_name(destination, stem, &settings, &folder, id) {
        return Ok((destination.to_owned(), None));
    }
    // Renamed once its contents are durable; a failure keeps the old name.
    Ok(match rename(destination, stem) {
        Ok(renamed) => (renamed, None),
        Err(error) => (
            destination.to_owned(),
            Some(format!(
                "{}: saved, but not renamed to {stem} ({error}); the next save tries again",
                destination.display()
            )),
        ),
    })
}

fn command_bootstrap_branch(record: &Recovered) -> Option<BranchId> {
    match record {
        Recovered::Control(ControlRecord::PlayerCommands { packet, .. })
            if packet
                .phase
                .as_ref()
                .is_some_and(|phase| phase.load_signal == 0) =>
        {
            Some(packet.branch)
        }
        _ => None,
    }
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

#[allow(clippy::too_many_arguments)]
fn finalize_campaign_with_publisher(
    campaign_dir: &Path,
    paths: &[(u32, PathBuf)],
    replay_dir: &Path,
    unsaved: bool,
    process_context: &mut Option<(SessionId, u32, u64, u64)>,
    witness: Option<&TerminalWitness>,
    admitted: Option<&BTreeSet<i64>>,
    progress: &dyn Fn() -> std::io::Result<()>,
    ahead: Option<Box<CampaignAhead>>,
    naming: Naming<'_>,
    checkpoint: bool,
    publish: impl FnOnce(&Replay, &Path) -> std::io::Result<()>,
    rename: impl FnOnce(&Path, &str) -> std::io::Result<PathBuf>,
    committed: &mut Committed<'_>,
) -> Result<CampaignOutcome> {
    let (mut tails, written) =
        ahead.map_or_else(Default::default, |ahead| (ahead.tails, ahead.written));
    let started = Instant::now();
    let folder = campaign_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| failure(campaign_dir, "campaign folder has no name"))?;
    let session_dir = campaign_dir.parent().unwrap_or(campaign_dir);
    // The game facts this session's recorder gathered, which every save adds once.
    let session_facts = fs::read(session_dir.join(replay::container::SESSION_FACTS_FILE)).ok();
    let facts_for = |identity: &replay_data::identity::Identity| {
        session_facts
            .as_deref()
            .and_then(|bytes| replay::container::session_facts_for(bytes, identity))
    };
    // Held until the recording has its name, so no user rename slips in between.
    let lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    reconcile_checkpoints(campaign_dir, replay_dir)?;
    let journaled = unsaved && !session_ended(session_dir);
    let promoted = if unsaved {
        None
    } else {
        publish_checkpointed_locked(campaign_dir, replay_dir, folder, naming)?
    };
    let staged = replay::publication::unsaved_path(replay_dir, folder);
    let existing = continued_recording(
        session_dir,
        replay_dir,
        folder.as_bytes(),
        &spool_session(paths),
    )?;
    let base = staged_recording(replay_dir, folder)?.or_else(|| existing.clone());
    let previous = match &base {
        Some(base) => {
            let previous = held_recording(base)?;
            previous
                .require_undamaged()
                .map_err(|error| failure(base, error))?;
            Some(previous)
        }
        None => None,
    };
    let base_blocks = previous.as_ref().map_or(0, |previous| previous.index.len());
    let base_roster = match &base {
        Some(base) => known_roster(base).map_err(|error| failure(base, error))?,
        None => Some(false),
    };
    let projected = witness
        .map(|witness| campaign_failure(session_dir, folder.as_bytes(), witness))
        .transpose()?;
    let witness = projected.as_ref().map(|failure| &failure.witness);
    let empty_prefix = projected
        .as_ref()
        .is_some_and(|failure| failure.empty_prefix);
    let bundled = projected.as_ref().is_some_and(|failure| failure.bundled);
    // Appends and publication preserve earlier saves even when their path or newest revision changes.
    let saved_record =
        verified_saved(campaign_dir, replay_dir, base.as_deref(), previous.as_ref())?;
    let mut bootstrap = saved_record
        .as_ref()
        .map(|saved| saved.command_bootstrap.clone())
        .unwrap_or_default();
    bootstrap.extend(
        tails
            .values()
            .flat_map(|(_, tail)| tail.records())
            .filter_map(command_bootstrap_branch),
    );
    let terminal_session = if paths.is_empty()
        && previous.is_none()
        && empty_prefix
        && !campaign_dir
            .join(SAVED_FILE)
            .try_exists()
            .map_err(|error| failure(campaign_dir, error))?
    {
        Some(failure_session(session_dir, witness.unwrap())?)
    } else {
        None
    };
    if paths.is_empty() && saved_record.is_none() && terminal_session.is_none() {
        return Err(failure(
            campaign_dir,
            "saved revision cannot be verified; spool retained",
        ));
    }
    let mut covered_saved = saved_record
        .as_ref()
        .filter(|saved| unsaved && is_unsaved_recording(&saved.replay))
        .map(|saved| saved.covered.clone())
        .unwrap_or_default();
    let (mut saved, mut omitted) = saved_record
        .map(|saved| (saved.segments, Vec::new()))
        .unwrap_or_default();
    let watermark = projected
        .as_ref()
        .map(|projected| committed_watermark(campaign_dir, projected))
        .transpose()?
        .flatten();
    let mut watermark_found =
        watermark.is_none_or(|(segment, bytes)| saved.get(&segment) == Some(&bytes));
    if !watermark_found
        && let Some((segment, bytes)) = watermark
        && let Some((_, path)) = paths.iter().find(|(number, path)| {
            *number == segment
                && saved.get(number).is_some_and(|length| {
                    fs::metadata(path).is_ok_and(|metadata| metadata.len() == *length)
                })
        })
    {
        // A saved file length does not certify a shorter terminal watermark inside it.
        let file = open_held(path).map_err(|error| failure(path, error))?;
        let length = file.metadata().map_err(|error| failure(path, error))?.len();
        let end = usize::try_from(bytes)
            .ok()
            .filter(|_| bytes <= length)
            .ok_or_else(|| failure(path, "failure witness watermark exceeds the spool length"))?;
        let recovered = recover_reader(
            std::io::BufReader::new(Budgeted::new(file)),
            end,
            Recovered::keep,
            |_| progress(),
        )
        .map_err(|error| failure(path, error))?;
        if recovered.valid_bytes != end {
            return Err(failure(
                path,
                "spool damage precedes the committed failure watermark",
            ));
        }
        watermark_found = true;
    }
    let paths: Vec<_> = paths
        .iter()
        .filter(|(number, path)| {
            saved.get(number).is_none_or(|bytes| {
                // Saved and verified up to its watermark: what follows is only failure evidence.
                watermark != Some((*number, *bytes))
                    && fs::metadata(path).map_or(true, |metadata| metadata.len() != *bytes)
            })
        })
        .collect();
    let mut ranges = projected
        .as_ref()
        .map(|failure| failure.ranges.clone())
        .unwrap_or_default();
    let failing_campaign = witness.is_some_and(|witness| {
        witness.campaign_id.as_ref().is_some_and(|campaign| {
            campaign_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::as_bytes)
                == Some(campaign.as_slice())
        })
    });
    let lost_hours = !ranges.is_empty()
        || (failing_campaign && witness.is_some_and(|witness| witness.failed_hour.is_some()));
    let mut failure_only = None;
    if paths.is_empty()
        && let (Some(witness), Some(recording), Some(path)) = (witness, &previous, &base)
    {
        failure_only = failure_segment_ranges(
            Held { recording, path },
            witness,
            failing_campaign
                && admitted_branch(admitted, &bootstrap, witness.branch, witness.load_signal),
            &ranges
                .iter()
                .copied()
                .filter(|range| {
                    admitted_branch(admitted, &bootstrap, Some(range.branch), range.load_signal)
                })
                .collect::<Vec<_>>(),
            bundled,
        )
        .map_err(|error| failure(campaign_dir, error))?;
    }
    if paths.is_empty() && failure_only.is_none() && !lost_hours {
        // Nothing new: the base already holds every hour.
        let (Some(base), Some(previous)) = (base, previous) else {
            return Ok(CampaignOutcome {
                replay: promoted,
                omitted,
                ..CampaignOutcome::default()
            });
        };
        let (destination, unrenamed) = if unsaved {
            if base != staged {
                // The saved recording holds it all; nothing waits.
                return Ok(CampaignOutcome {
                    replay: promoted,
                    omitted,
                    ..CampaignOutcome::default()
                });
            }
            replay::publication::sync_file(&staged).map_err(|error| failure(&staged, error))?;
            (staged, None)
        } else {
            let multiplayer = match base_roster {
                Some(known) => known,
                None => crate::recording_name::multiplayer(&previous, base_blocks)
                    .map_err(|error| failure(&base, error))?,
            };
            let stem = recording_stem(&previous, naming, multiplayer)
                .map_err(|error| failure(&base, error))?;
            let bytes = if base == staged {
                Bytes::Staged(&staged)
            } else {
                Bytes::Unchanged
            };
            let placed = save_recording_file(
                &previous,
                bytes,
                replay_dir,
                existing.as_deref(),
                &stem,
                naming,
                publish,
                rename,
                progress,
            )?;
            record_recording(session_dir, &previous.campaign_id, &placed.0)?;
            remember_roster(&placed.0, multiplayer).map_err(|error| failure(&placed.0, error))?;
            if base == staged {
                write_saved_record(
                    campaign_dir,
                    &placed.0,
                    &previous,
                    saved,
                    covered_saved,
                    &omitted,
                    &bootstrap,
                )?;
            }
            placed
        };
        if !journaled {
            accept_checkpoints(campaign_dir)?;
        }
        drop(lock);
        println!("{} (already finalized; unchanged)", destination.display());
        return Ok(CampaignOutcome {
            replay: Some(destination),
            unrenamed,
            omitted,
            ..CampaignOutcome::default()
        });
    }
    let drain_phase = replay::container::Packing::phase(replay::container::Phase::Drain);
    let mut spool_bytes = 0_u64;
    let mut segments = Vec::with_capacity(paths.len());
    let mut held = BTreeMap::new();
    let mut lengths = BTreeMap::new();
    for (number, path) in paths {
        if empty_prefix {
            continue;
        }
        let file = open_held(path).map_err(|error| failure(path, error))?;
        let length = file.metadata().map_err(|error| failure(path, error))?.len();
        spool_bytes += length;
        let mut prefix_length = usize::try_from(length).map_err(|error| failure(path, error))?;
        if let Some((committed_segment, committed_bytes)) = watermark {
            if *number > committed_segment {
                continue;
            }
            if *number == committed_segment {
                let end = usize::try_from(committed_bytes)
                    .ok()
                    .filter(|end| *end <= prefix_length)
                    .ok_or_else(|| {
                        failure(path, "failure witness watermark exceeds the spool length")
                    })?;
                prefix_length = end;
                watermark_found = true;
            }
        }
        let recovered = {
            let mut input = std::io::BufReader::new(Budgeted::new(&file));
            let continued = match tails.remove(number) {
                Some((tail_path, tail)) if tail_path == *path => tail
                    .finish(&mut input, prefix_length, Recovered::keep, |_| progress())
                    .map_err(|error| failure(path, error))?,
                _ => None,
            };
            match continued {
                Some(recovered) => recovered,
                None => {
                    input
                        .seek(std::io::SeekFrom::Start(0))
                        .map_err(|error| failure(path, error))?;
                    recover_reader(input, prefix_length, Recovered::keep, |_| progress())
                        .map_err(|error| failure(path, error))?
                }
            }
        };
        held.insert(*number, file);
        if watermark.is_some_and(|(segment, _)| segment == *number)
            && recovered.valid_bytes != prefix_length
        {
            return Err(failure(
                path,
                "spool damage precedes the committed failure watermark",
            ));
        }
        if let Some(reason) = &recovered.damage {
            eprintln!(
                "{}: recovered {} valid bytes; {reason}",
                path.display(),
                recovered.valid_bytes
            );
        }
        // Only the bytes read are saved: cleanup keeps any tail past a failure watermark.
        lengths.insert(*number, prefix_length as u64);
        segments.push((*number, recovered));
    }
    drop(drain_phase);
    if !watermark_found {
        return Err(failure(
            campaign_dir,
            "failure witness committed segment is missing",
        ));
    }
    if segments.is_empty() && lost_hours {
        let witness = witness.unwrap();
        let session = terminal_session
            .or_else(|| {
                previous.as_ref().and_then(|recording| {
                    recording
                        .sessions
                        .iter()
                        .find(|session| session.id == witness.session)
                        .cloned()
                })
            })
            .map(Ok)
            .unwrap_or_else(|| failure_session(session_dir, witness))?;
        segments.push((
            FAILURE_SEGMENT,
            Recovery {
                format: Some(FormatVersion::CURRENT),
                records: vec![Recovered::SessionStart(session)],
                valid_bytes: 0,
                damage: None,
            },
        ));
    }
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        if let Recovered::SessionStart(session) = record {
            if witness.is_some_and(|witness| {
                session.id != witness.session
                    || session.process_start_time != witness.process_start_time
            }) {
                return Err(failure(
                    campaign_dir,
                    "failure witness does not match the spool process session",
                ));
            }
            let context = (
                session.id,
                session.process_id,
                session.process_start_time,
                session.qpc_frequency,
            );
            if process_context.is_some_and(|previous| previous != context) {
                return Err(failure(
                    campaign_dir,
                    "campaign spools in one session directory describe different game processes",
                ));
            }
            *process_context = Some(context);
        }
    }
    bootstrap.extend(
        segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(command_bootstrap_branch),
    );
    let frameless = !segments.iter().any(|(_, recovery)| {
        recovery
            .records
            .iter()
            .any(|record| matches!(record, Recovered::Frame(_)))
    });
    let commands = segments.iter().any(|(_, recovery)| {
        recovery.records.iter().any(|record| {
            matches!(
                record,
                Recovered::Control(ControlRecord::PlayerCommands { .. })
            )
        })
    });
    let mut explicit_loss = witness.is_some()
        && previous.is_some()
        && frameless
        && segments.iter().any(|(_, recovery)| {
            recovery
                .records
                .iter()
                .any(|record| matches!(record, Recovered::Gap(_)))
        });
    if witness.is_none() && !saved.is_empty() && frameless {
        if admitted.is_some()
            && let Some(previous) = previous
                .as_ref()
                .filter(|previous| unchanged_context(previous, &segments))
        {
            let spool_bytes = lengths.values().sum();
            keep_saved_tail(
                campaign_dir,
                base.as_ref().unwrap(),
                previous.revision(),
                saved,
                covered_saved,
                lengths,
                &omitted,
                &bootstrap,
                journaled,
            )?;
            return Ok(CampaignOutcome {
                replay: promoted.or_else(|| base.filter(|base| !unsaved || *base == staged)),
                omitted,
                report: Some(Box::new(FinalizationReport {
                    revision: Some(previous.revision()),
                    spool_bytes,
                    ..Default::default()
                })),
                ..CampaignOutcome::default()
            });
        }
        if !(commands
            && segments
                .iter()
                .all(|(_, recovery)| recovery.damage.is_none()))
        {
            match previous
                .as_ref()
                .and_then(|previous| loss_tail(previous, &segments))
            {
                Some(loss) => {
                    segments = loss;
                    explicit_loss = true;
                }
                None if admitted.is_some() => {
                    return Err(failure(
                        campaign_dir,
                        "frameless spool has damage or metadata not held by its saved recording; spool retained",
                    ));
                }
                None => {}
            }
        }
    }
    let id = match (
        campaign_id(
            segments
                .iter()
                .map(|(_, recovery)| recovery.records.as_slice()),
        ),
        &previous,
    ) {
        // Checkpoints can leave a failed session only spool context; its recording names it.
        (Err(_), Some(previous)) if frameless && (lost_hours || explicit_loss || commands) => {
            previous.campaign_id.clone()
        }
        (Err(_), None) if frameless && !ranges.is_empty() => ranges[0].campaign_id.to_vec(),
        (id, _) => id.map_err(|error| failure(campaign_dir, error))?,
    };
    let name = campaign_filename(&id).map_err(|error| failure(campaign_dir, error))?;
    if campaign_dir.file_name().and_then(|name| name.to_str()) != Some(name) {
        return Err(failure(
            campaign_dir,
            format!("directory differs from recorded campaign {name}"),
        ));
    }
    // A Battle Plan Recorder session publishes only its admitted loads. A failure
    // witness describes the end of the committed spool; when that end (or the
    // load that failed) is not admitted, the failure is not this recording's
    // and the published hours end cleanly before it. A branch that failed
    // before its first clock witness takes its load from the validated witness.
    let mut incomplete = BTreeSet::new();
    let mut failure_published = true;
    if let Some(admitted) = admitted {
        let last_branch = segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Recovered::BranchStart(branch) => Some(branch.id),
                _ => None,
            })
            .next_back();
        let mut known: Vec<_> = ranges
            .iter()
            .filter_map(|range| Some((range.branch, range.load_signal?)))
            .chain(
                witness
                    .filter(|_| failing_campaign)
                    .and_then(|witness| Some((witness.branch?, witness.load_signal?))),
            )
            .collect();
        // A saved branch's loss has its saved hours' load; a new branch needs its own clock.
        if (explicit_loss || witness.is_some() || commands)
            && let Some(previous) = &previous
        {
            for branch in previous.branches.iter().filter(|branch| {
                segments
                    .iter()
                    .flat_map(|(_, recovery)| &recovery.records)
                    .any(|record| {
                        matches!(record, Recovered::BranchStart(start) if start == *branch)
                            || matches!(record, Recovered::Control(ControlRecord::PlayerCommands { packet, .. }) if packet.branch == branch.id)
                    })
            }) {
                if known.iter().any(|(id, _)| *id == branch.id) {
                    continue;
                }
                if let Some(signal) = held_signal(previous, branch.id)
                    .map_err(|error| failure(campaign_dir, error))?
                {
                    known.push((branch.id, signal));
                }
            }
        }
        // Explicit loss of a load left out keeps its segment for a later admission.
        let counted = |record: &&Recovered| {
            matches!(record, Recovered::Frame(_))
                || matches!(
                    record,
                    Recovered::Control(ControlRecord::PlayerCommands { .. })
                )
                || explicit_loss && matches!(record, Recovered::Gap(_))
        };
        let frames_before: BTreeMap<_, _> = segments
            .iter()
            .map(|(number, recovery)| (*number, recovery.records.iter().filter(counted).count()))
            .collect();
        let (dropped, left_out) = keep_admitted_loads(&mut segments, admitted, &known, &bootstrap);
        incomplete.extend(segments.iter().filter_map(|(number, recovery)| {
            let kept = recovery.records.iter().filter(counted).count();
            (frames_before[number] != kept).then_some(*number)
        }));
        for (load, hours) in left_out {
            match omitted.iter_mut().find(|(known, _)| *known == load) {
                Some((_, total)) => *total += hours,
                None => omitted.push((load, hours)),
            }
        }
        // A branch its saved recording holds was admitted, even with only context left in the spool.
        let kept = |branch: &BranchId| {
            !dropped.contains(branch)
                || previous.as_ref().is_some_and(|previous| {
                    previous.branches.iter().any(|known| known.id == *branch)
                })
        };
        failure_published = last_branch.is_none_or(|branch| kept(&branch))
            && (!failing_campaign
                || witness.is_none_or(|witness| {
                    admitted_branch(
                        Some(admitted),
                        &bootstrap,
                        witness.branch,
                        witness.load_signal,
                    )
                }));
        // Admitted hours of a load left out are not this recording's to gap.
        ranges.retain(|range| {
            kept(&range.branch)
                && admitted_branch(
                    Some(admitted),
                    &bootstrap,
                    Some(range.branch),
                    range.load_signal,
                )
        });
        let lost_hours = failure_only.is_some()
            || !ranges.is_empty()
            || (failing_campaign
                && failure_published
                && witness.is_some_and(|witness| witness.failed_hour.is_some()));
        if !lost_hours
            && !segments.iter().any(|(_, recovery)| {
                recovery.records.iter().any(|record| {
                    matches!(record, Recovered::Frame(_))
                        || matches!(
                            record,
                            Recovered::Control(ControlRecord::PlayerCommands { .. })
                        )
                        || explicit_loss && matches!(record, Recovered::Gap(_))
                })
            })
        {
            if explicit_loss && let (Some(base), Some(previous)) = (&base, &previous) {
                keep_saved_tail(
                    campaign_dir,
                    base,
                    previous.revision(),
                    saved,
                    covered_saved,
                    lengths
                        .into_iter()
                        .filter(|(number, _)| !incomplete.contains(number))
                        .collect(),
                    &omitted,
                    &bootstrap,
                    journaled,
                )?;
                return Ok(CampaignOutcome {
                    replay: promoted
                        .or_else(|| (!unsaved || *base == staged).then(|| base.clone())),
                    omitted,
                    ..CampaignOutcome::default()
                });
            }
            if promoted.is_none() {
                println!(
                    "{}: no admitted load recorded an hour; nothing published",
                    campaign_dir.display()
                );
            }
            return Ok(CampaignOutcome {
                replay: promoted,
                omitted,
                ..CampaignOutcome::default()
            });
        }
    }
    if let (Some(base), Some(previous)) = (&base, &previous)
        && previous.campaign_id != id
    {
        return Err(failure(
            base,
            "existing replay belongs to a different campaign",
        ));
    }
    let mut covered = BTreeMap::new();
    for (number, recovery) in &segments {
        let mut hours = 0;
        let mut span: Option<(Hour, Hour)> = None;
        for record in &recovery.records {
            if let Recovered::Frame(frame) = record {
                hours += 1;
                span = Some(span.map_or((frame.hour, frame.hour), |(first, last)| {
                    (first.min(frame.hour), last.max(frame.hour))
                }));
            }
        }
        if let Some(length) = lengths.get(number) {
            covered.insert(*number, (*length, hours, span));
        }
    }
    let frames = FinalizationFrames::new(held, &segments);
    if let Some(previous) = &previous {
        remove_held_branches(previous, &mut segments, |number, frame| {
            progress()?;
            frames.read(number, frame)
        })?;
    }
    let prefix = match &previous {
        Some(previous) => published_prefix(previous, &segments, |number, frame| {
            progress()?;
            frames.read(number, frame)
        })?,
        None => 0,
    };
    let saved_recording = previous
        .as_ref()
        .zip(base.as_deref())
        .map(|(recording, path)| Held { recording, path });
    if prefix == segments.len()
        && failure_only.is_none()
        && let (Some(witness), Some(held)) = (witness, saved_recording)
    {
        // The recording holds every committed hour: only the failure's lost hours are new.
        failure_only = failure_segment_ranges(
            held,
            witness,
            failing_campaign && failure_published,
            &ranges,
            bundled,
        )
        .map_err(|error| failure(campaign_dir, error))?;
    }
    // Saved before a failure elsewhere: it stays as it was saved.
    if let Some(witness) = witness
        && (failure_published || !ranges.is_empty())
        && prefix != segments.len()
    {
        apply_failure_ranges(
            &mut segments,
            witness,
            failing_campaign && failure_published,
            &ranges,
            bundled,
            saved_recording,
        )
        .map_err(|error| failure(campaign_dir, error))?;
    }
    segments.extend(failure_only);
    let unchanged = prefix == segments.len();
    let mut sorted: Vec<_> = segments[prefix..]
        .iter()
        .map(|(number, recovery)| (*number, recovery.records.as_slice()))
        .collect();
    sorted.sort_unstable_by_key(|(number, _)| *number);
    let written = written.map(|(mut ahead, summary)| {
        if prefix != 0
            && let Some(previous) = &previous
        {
            // published_prefix already compared these frames exactly after an interrupted save.
            let _ = ahead.accept_verified_prefix(segments[prefix - 1].0, previous);
        }
        (ahead, summary)
    });
    let (ahead, mut summary) = match written {
        Some((ahead, summary))
            if ahead.continues(&sorted)
                && match (ahead.existing(), &previous) {
                    (None, None) => true,
                    (Some(before), Some(previous)) => before.same_revision(previous),
                    _ => false,
                } =>
        {
            (Some(ahead), summary)
        }
        _ => (None, Box::default()),
    };
    let read = |number, frame: &FrameMetadata| frames.read(number, frame);
    let observe = |hour: &Observed| {
        progress()?;
        summary.observe_hour(hour).map_err(std::io::Error::other)
    };
    let replay = match (previous, &base) {
        (Some(previous), _) if unchanged => previous,
        (Some(previous), Some(base)) => {
            // Added to in place: the base itself, or for hours that wait unsaved, a copy of the
            // recording they continue put there first.
            let target = if unsaved { &staged } else { base };
            if journaled {
                let before = (target == base).then(|| previous.revision());
                begin_checkpoint(campaign_dir, target, before, &previous)?;
            }
            let target = if unsaved && *base != staged {
                stage_copy(&previous, &staged)?;
                &staged
            } else {
                base
            };
            let mut appending =
                Replay::open_appendable(target).map_err(|error| failure(target, error))?;
            if !appending.same_revision(&previous) {
                return Err(failure(
                    target,
                    "the recording changed while it was being saved; nothing saved; spool retained",
                ));
            }
            drop(previous);
            let facts = facts_for(&appending.identity);
            appending
                .append_in_place(
                    &segments[prefix..],
                    read,
                    progress,
                    observe,
                    ahead,
                    facts,
                    |revision| {
                        if journaled {
                            intend_checkpoint(campaign_dir, target, revision)
                                .map_err(std::io::Error::other)?;
                        }
                        Ok(())
                    },
                )
                .map_err(|error| failure(target, error))?;
            // Read on through a handle that lets the save move or rename it.
            let revision = appending.revision();
            drop(appending);
            let saved = held_recording(target)?;
            if saved.revision() != revision {
                return Err(failure(
                    target,
                    "the recording changed after it was saved; spool retained",
                ));
            }
            saved
        }
        (Some(_), None) => unreachable!("a recording is added to only with a base"),
        (None, _) => Replay::from_recovered_with_reader(
            id.clone(),
            &segments,
            read,
            progress,
            observe,
            ahead,
        )
        .and_then(|replay| match facts_for(&replay.identity) {
            Some(pack) => replay.with_facts(pack),
            None => Ok(replay),
        })
        .map_err(|error| failure(campaign_dir, error))?,
    };
    let mut outcome = CampaignOutcome {
        omitted: omitted.clone(),
        covered,
        incomplete,
        ..CampaignOutcome::default()
    };
    let mut published_roster = None;
    let destination = if journaled && base.is_none() {
        replay::publication::create_unsaved_folder(replay_dir)
            .map_err(|error| failure(replay_dir, error))?;
        begin_checkpoint(campaign_dir, &staged, None, &replay)?;
        publish(&replay, &staged).map_err(|error| failure(&staged, error))?;
        staged.clone()
    } else if unsaved {
        if !unchanged && base.is_none() {
            replay::publication::create_unsaved_folder(replay_dir)
                .map_err(|error| failure(replay_dir, error))?;
            publish(&replay, &staged).map_err(|error| failure(&staged, error))?;
        } else if unchanged && base.as_ref() != Some(&staged) {
            // The saved recording holds it all; nothing waits.
            return Ok(CampaignOutcome {
                replay: promoted,
                omitted,
                ..CampaignOutcome::default()
            });
        } else {
            replay::publication::sync_file(&staged).map_err(|error| failure(&staged, error))?;
        }
        staged.clone()
    } else {
        let checked = if unchanged {
            replay.index.len()
        } else {
            base_blocks
        };
        let multiplayer = summary.multiplayer
            || match base_roster {
                Some(known) => known,
                None => crate::recording_name::multiplayer(&replay, checked)
                    .map_err(|error| failure(campaign_dir, error))?,
            };
        let stem = recording_stem(&replay, naming, multiplayer)
            .map_err(|error| failure(campaign_dir, error))?;
        let bytes = match &base {
            Some(base) if *base == staged => Bytes::Staged(&staged),
            Some(_) => Bytes::Unchanged,
            None => Bytes::New,
        };
        let (destination, unrenamed) = save_recording_file(
            &replay,
            bytes,
            replay_dir,
            existing.as_deref(),
            &stem,
            naming,
            publish,
            rename,
            progress,
        )?;
        outcome.unrenamed = unrenamed;
        published_roster = Some(multiplayer);
        destination
    };
    if (checkpoint || journaled) && !unchanged {
        checkpoint_step(CheckpointStep::Written, &destination)?;
    }
    let replay = if base.is_none() {
        verify_publication(&replay, &destination, progress)?
    } else {
        replay
    };
    if (checkpoint || journaled) && !unchanged {
        checkpoint_step(CheckpointStep::Verified, &destination)?;
    }
    let publication_phase =
        replay::container::Packing::phase(replay::container::Phase::Publication);
    if unchanged {
        println!("{} (already finalized; unchanged)", destination.display());
    }
    if journaled && !unchanged {
        finish_checkpoint(campaign_dir, &destination, &replay)?;
    }
    if let Some(multiplayer) = published_roster {
        record_recording(session_dir, &id, &destination)?;
        remember_roster(&destination, multiplayer).map_err(|error| failure(&destination, error))?;
    } else if let Some(multiplayer) = base_roster
        .map(|known| known || summary.multiplayer)
        .or_else(|| summary.multiplayer.then_some(true))
    {
        remember_roster(&destination, multiplayer).map_err(|error| failure(&destination, error))?;
    }
    saved.extend(
        lengths
            .into_iter()
            .filter(|(number, _)| !outcome.incomplete.contains(number)),
    );
    covered_saved.extend(
        outcome
            .covered
            .iter()
            .filter(|(number, _)| !outcome.incomplete.contains(number))
            .map(|(number, covered)| (*number, *covered)),
    );
    write_saved_record(
        campaign_dir,
        &destination,
        &replay,
        saved,
        covered_saved,
        &omitted,
        &bootstrap,
    )?;
    if checkpoint && !unchanged {
        checkpoint_step(CheckpointStep::Watermarked, &destination)?;
    }
    if !unchanged {
        committed(&destination, &replay, &replay.index[base_blocks..]);
        if checkpoint {
            checkpoint_step(CheckpointStep::Handed, &destination)?;
        }
    }
    if !journaled {
        accept_checkpoints(campaign_dir)?;
    }
    drop(lock);
    progress().map_err(|error| failure(&destination, error))?;
    drop(publication_phase);
    let ready_seconds = started.elapsed().as_secs_f64();
    if !unchanged {
        println!("{}", destination.display());
    }
    if checkpoint {
        emit_finalization(&format!(
            "checkpoint_s={ready_seconds:.3} replay_bytes={} spool_bytes={spool_bytes} hours={}\n",
            replay.saved_length(),
            outcome
                .covered
                .values()
                .map(|(_, hours, _)| hours)
                .sum::<u64>()
        ));
    }
    Ok(CampaignOutcome {
        replay: Some(destination),
        report: Some(Box::new(FinalizationReport {
            revision: Some(replay.revision()),
            summary,
            spool_bytes,
            seconds: Some(ready_seconds),
        })),
        ..outcome
    })
}

/// Note the segments a save put into `destination`; without the note the next save compares them
/// with the recording instead.
fn write_saved_record(
    campaign_dir: &Path,
    destination: &Path,
    replay: &Replay,
    segments: BTreeMap<u32, u64>,
    covered: BTreeMap<u32, (u64, u64, Option<(Hour, Hour)>)>,
    omitted: &[(Option<i64>, u64)],
    command_bootstrap: &BTreeSet<BranchId>,
) -> Result<()> {
    write_saved(
        campaign_dir,
        &Saved {
            command_bootstrap: command_bootstrap.clone(),
            replay: destination.to_owned(),
            revision: replay.revision(),
            segments,
            covered,
            omitted: omitted.to_vec(),
        },
    )
}

/// The branch tracker after the committed history, for a branch whose start records were torn.
fn failure_tracker(
    campaign: &[u8; 36],
    segments: &[(u32, Recovery<Recovered>)],
    session: SessionId,
    held: Option<&Path>,
    contexts: &[UncommittedRange],
) -> Result<BranchTracker> {
    let spool = Replay::from_recovered(campaign.to_vec(), segments)
        .and_then(Replay::into_history)
        .map_err(|error| error.to_string())?;
    if held.is_none()
        && let Some(found) = spool.sessions.iter().find(|known| known.id == session)
        && spool.session_tail(session).is_some()
        && contexts.is_empty()
    {
        return spool.tracker_for(found).map_err(|error| error.to_string());
    }
    // Checkpoints moved the session's earlier hours from the spool into the saved recording.
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
                    .and_then(replay::commands::load_marker),
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

/// The load signal of `branch`'s last saved frame, read from its last block alone.
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
    let mut decoder = FrameDecoder::for_tables(&[TableId::Clock]);
    let mut signal = None;
    for frame in recording
        .read_block(block)
        .map_err(|error| error.to_string())?
    {
        let decoded = decoder.advance(&frame).map_err(|error| error.to_string())?;
        signal = decoded
            .complete::<tables::Clock>()
            .ok()
            .and_then(|rows| rows.first())
            .map(|clock| clock.play_session_started_at);
    }
    Ok(signal)
}

/// A saved recording of the campaign: checkpoints may have moved committed records into it.
#[derive(Clone, Copy)]
struct Held<'a> {
    recording: &'a Replay,
    path: &'a Path,
}

/// `uncommitted` is the witnessed range only when it names this campaign.
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

/// No spool segment holds this number: a failure-only segment has no file.
const FAILURE_SEGMENT: u32 = u32::MAX;

/// Append only lost hours a checkpoint does not already hold.
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
    // An earlier save already added this failure: its outcome and every lost hour are recorded.
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
    // A new branch follows the session directly: its context branch has no keyframe here.
    if matches!(inserted.first(), Some(Recovered::BranchStart(_))) {
        recovery.records.truncate(1);
    }
    recovery.records.extend(inserted);
    Ok(Some((number, recovery)))
}

/// The records a failure adds after the committed prefix, and whether it derives a branch.
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
    let signal = signal.and_then(replay::commands::load_marker);
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
        // A continuous clock with the next witnessed branch proves native invalidation.
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
    use replay::commands::load_marker;
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
    let mut latest_phase: Option<&replay::commands::Phase> = None;
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
    // The last committed hour of each branch, in the spool or saved by a checkpoint.
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
    // The admitted range precedes the attempt that failed: resolve it first.
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
        // Hours a committed gap already accounts for lead the range, never follow it.
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
            // A context record can commit before the new branch's first clock.
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
    // The range and the attempt on one branch become one gap when they touch.
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

fn open(path: &Path) -> Result<Replay> {
    Replay::open(read_file(path).map_err(|error| failure(path, error))?)
        .map_err(|error| failure(path, error))
}

fn verify_publication(
    expected: &Replay,
    path: &Path,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<Replay> {
    let _phase = replay::container::Packing::phase(replay::container::Phase::Readback);
    let saved = held_recording(path)?;
    saved
        .require_undamaged()
        .map_err(|error| failure(path, error))?;
    if !saved.same_revision(expected) {
        return Err(failure(
            path,
            "the published recording differs from the save; spool retained",
        ));
    }
    for block in &saved.index {
        progress().map_err(|error| failure(path, error))?;
        saved
            .read_block_parallel(
                block,
                &replay::container::Parallel {
                    workers: replay::container::Packing::workers(),
                    on_worker: &|| {},
                },
            )
            .map_err(|error| failure(path, error))?;
    }
    Ok(saved)
}

/// A recording read through its file, holding only its metadata.
fn held_recording(path: &Path) -> Result<Replay> {
    Replay::open_path(path).map_err(|error| failure(path, error))
}

/// Put a copy of `recording`'s saves at `staged`, the unsaved recording, never replacing one.
fn stage_copy(recording: &Replay, staged: &Path) -> Result<()> {
    replay::publication::create_unsaved_folder(
        staged
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| failure(staged, "the unsaved recording has no folder"))?,
    )
    .map_err(|error| failure(staged, error))?;
    let temporary = stage_copy_path(staged);
    remove_stale_copy(&temporary).map_err(|error| failure(&temporary, error))?;
    recording
        .copy_saved(&temporary)
        .and_then(|()| replay::publication::rename_new(&temporary, staged))
        .map_err(|error| {
            let _ = fs::remove_file(&temporary);
            failure(staged, error)
        })
}

fn stage_copy_path(staged: &Path) -> PathBuf {
    let mut path = staged.as_os_str().to_owned();
    path.push(".copy.tmp");
    PathBuf::from(path)
}

fn remove_stale_copy(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(metadata) => {
            use std::os::windows::fs::MetadataExt;
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err(std::io::Error::other(
                    "staging temporary is not an ordinary file",
                ));
            }
            fs::remove_file(path)
        }
    }
}

/// The segments a save put into a recording, tied to its exact committed revision.
const SAVED_FILE: &str = "saved.json";

struct Saved {
    command_bootstrap: BTreeSet<BranchId>,
    replay: PathBuf,
    revision: Revision,
    /// Segment number to its length when saved.
    segments: BTreeMap<u32, u64>,
    covered: BTreeMap<u32, (u64, u64, Option<(Hour, Hour)>)>,
    /// Loads those segments left out, with their hours.
    omitted: Vec<(Option<i64>, u64)>,
}

fn read_saved(campaign_dir: &Path) -> Option<Saved> {
    let value: Value =
        serde_json::from_slice(&read_file(campaign_dir.join(SAVED_FILE)).ok()?).ok()?;
    Some(Saved {
        command_bootstrap: value["command_bootstrap"]
            .as_array()?
            .iter()
            .map(Value::as_u64)
            .collect::<Option<_>>()?,
        replay: PathBuf::from(value["replay"].as_str()?),
        revision: revision_of(&value["revision"])?,
        segments: value["segments"]
            .as_array()?
            .iter()
            .map(|entry| Some((u32::try_from(entry[0].as_u64()?).ok()?, entry[1].as_u64()?)))
            .collect::<Option<_>>()?,
        covered: value["covered"]
            .as_array()?
            .iter()
            .map(|entry| {
                let hour =
                    |value: &Value| value.as_i64().and_then(|hour| Hour::try_from(hour).ok());
                let span = hour(&entry[3]).zip(hour(&entry[4]));
                Some((
                    u32::try_from(entry[0].as_u64()?).ok()?,
                    (entry[1].as_u64()?, entry[2].as_u64()?, span),
                ))
            })
            .collect::<Option<_>>()?,
        omitted: value["omitted"]
            .as_array()?
            .iter()
            .map(|entry| {
                let load = match &entry[0] {
                    Value::Null => None,
                    load => Some(load.as_i64()?),
                };
                Some((load, entry[1].as_u64()?))
            })
            .collect::<Option<_>>()?,
    })
}

fn saved_ancestry(
    saved: &Saved,
    replay_dir: &Path,
    campaign: &[u8],
    base: &Path,
    previous: &Replay,
) -> Result<bool> {
    let saved_folder = checkpoint_folder(&saved.replay)?;
    Ok(
        folder_key(saved_folder).map_err(|error| failure(saved_folder, error))?
            == folder_key(replay_dir).map_err(|error| failure(replay_dir, error))?
            && previous.campaign_id.eq_ignore_ascii_case(campaign)
            && previous
                .contains_revision(saved.revision)
                .map_err(|error| failure(base, error))?,
    )
}

fn verified_saved(
    campaign_dir: &Path,
    replay_dir: &Path,
    base: Option<&Path>,
    previous: Option<&Replay>,
) -> Result<Option<Saved>> {
    let (Some(mut saved), Some(base), Some(previous)) = (read_saved(campaign_dir), base, previous)
    else {
        return Ok(None);
    };
    let campaign = campaign_dir.file_name().unwrap().as_encoded_bytes();
    if !saved_ancestry(&saved, replay_dir, campaign, base, previous)? {
        return Ok(None);
    }
    if saved.replay != base || saved.revision != previous.revision() {
        if !is_unsaved_recording(&saved.replay) && is_unsaved_recording(base) {
            saved.covered.clear();
        }
        saved.replay = base.to_owned();
        saved.revision = previous.revision();
        write_saved(campaign_dir, &saved)?;
    }
    Ok(Some(saved))
}

fn write_saved(campaign_dir: &Path, saved: &Saved) -> Result<()> {
    write_json(
        &campaign_dir.join(SAVED_FILE),
        &json!({
            "command_bootstrap": saved.command_bootstrap,
            "replay": saved.replay,
            "revision": revision_json(saved.revision),
            "segments": saved.segments.iter().map(|(number, bytes)| json!([number, bytes])).collect::<Vec<_>>(),
            "covered": saved.covered.iter().map(|(number, (bytes, hours, span))| {
                json!([number, bytes, hours, span.map(|span| span.0), span.map(|span| span.1)])
            }).collect::<Vec<_>>(),
            "omitted": saved.omitted.iter().map(|(load, hours)| json!([load, hours])).collect::<Vec<_>>(),
        }),
    )
}

/// Whether `recording` is a folder's unsaved recording.
fn is_unsaved_recording(recording: &Path) -> bool {
    recording.parent()
        == Some(&replay::publication::unsaved_folder(
            recording
                .parent()
                .and_then(Path::parent)
                .unwrap_or(Path::new("")),
        ))
}

/// By the saved record and failure watermark: for showing and discarding, never for publishing.
pub fn unsaved_segments(campaign_dir: &Path) -> Result<Vec<(u32, PathBuf, u64)>> {
    Ok(unsaved_spans(campaign_dir)?
        .into_iter()
        .map(|(number, path, bytes, _)| (number, path, bytes))
        .collect())
}

/// Each segment with bytes waiting to be saved: its number, path, length and those bytes.
fn unsaved_spans(campaign_dir: &Path) -> Result<Vec<(u32, PathBuf, u64, std::ops::Range<u64>)>> {
    // Hours staged into an unsaved recording still wait.
    let saved = read_saved(campaign_dir)
        .filter(|saved| !is_unsaved_recording(&saved.replay))
        .map(|saved| saved.segments)
        .unwrap_or_default();
    let end = SpoolEnd::of(campaign_dir)?;
    let mut unsaved = Vec::new();
    for (number, path) in segment_paths(campaign_dir)? {
        let bytes = fs::metadata(&path)
            .map_err(|error| failure(&path, error))?
            .len();
        if let Some(span) = end.unsaved(&path, number, bytes, saved.get(&number).copied())? {
            unsaved.push((number, path, bytes, span));
        }
    }
    Ok(unsaved)
}

/// What a campaign's unsaved spool holds, for a recording waiting to be saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unsaved {
    /// Admitted hours.
    pub hours: u64,
    pub first_hour: Option<Hour>,
    pub last_hour: Option<Hour>,
    /// Spool bytes on disk.
    pub bytes: u64,
    /// A load in them was never asked about, so what a save keeps is not known yet.
    pub undecided: bool,
}

/// The admitted hours of a campaign's unsaved segments; `None` when nothing waits.
pub fn unsaved(session_dir: &Path, campaign: &str) -> Result<Option<Unsaved>> {
    let admissions = session_admissions(session_dir)?;
    let campaign_dir = session_dir.join(campaign);
    let admitted = admissions
        .as_ref()
        .map(|admissions| admissions.loads.get(campaign).cloned().unwrap_or_default());
    let decided = |load: i64| {
        admissions.as_ref().is_none_or(|admissions| {
            admitted
                .as_ref()
                .is_some_and(|admitted| admitted.contains(&load))
                || admissions
                    .refused
                    .contains_key(&(campaign.to_owned(), load))
        })
    };
    let mut summary = Unsaved::default();
    // Hours checkpoints put into the unsaved recording from segments since deleted still wait.
    if let Some(saved) =
        read_saved(&campaign_dir).filter(|saved| is_unsaved_recording(&saved.replay))
    {
        for (number, (_, hours, span)) in &saved.covered {
            if campaign_dir.join(format!("spool.{number:04}.h4s")).exists() {
                continue;
            }
            summary.hours += hours;
            if let Some((first, last)) = span {
                summary.first_hour =
                    Some(summary.first_hour.map_or(*first, |hour| hour.min(*first)));
                summary.last_hour = Some(summary.last_hour.map_or(*last, |hour| hour.max(*last)));
            }
        }
    }
    for (_, path, bytes, span) in unsaved_spans(&campaign_dir)? {
        summary.bytes += bytes;
        let file = fs::File::open(&path).map_err(|error| failure(&path, error))?;
        let recovery = recover_reader(
            std::io::BufReader::new(Budgeted::new(file)),
            usize::try_from(span.end).map_err(|error| failure(&path, error))?,
            Recovered::keep,
            |_| Ok(()),
        )
        .map_err(|error| failure(&path, error))?;
        let mut branches = BranchLoads::default();
        for record in &recovery.records {
            branches.observe(record.view());
        }
        if let Some(saved) = read_saved(&campaign_dir) {
            branches.restore_bootstrap(&saved.command_bootstrap);
        }
        summary.undecided |= branches.loads.values().any(|load| !decided(*load));
        for record in &recovery.records {
            let Recovered::Frame(frame) = record else {
                continue;
            };
            let placement = frame.placement;
            let end = placement.offset + replay::spool::RECORD_HEADER_BYTES + placement.length;
            // The recording already holds the frames of the segment's saved prefix.
            if end as u64 <= span.start {
                continue;
            }
            if admitted.as_ref().is_some_and(|admitted| {
                !branches
                    .loads
                    .get(&frame.branch)
                    .is_some_and(|load| admitted.contains(load))
            }) {
                continue;
            }
            let hour = frame.hour;
            summary.hours += 1;
            summary.first_hour = Some(summary.first_hour.map_or(hour, |first| first.min(hour)));
            summary.last_hour = Some(summary.last_hour.map_or(hour, |last| last.max(hour)));
        }
    }
    Ok((summary.hours > 0 || summary.undecided).then_some(summary))
}

/// Publish a live checkpoint's copy before another session can promote that shared recording.
pub fn publish_checkpointed(
    session_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Result<Option<PathBuf>> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let campaign_dir = session_dir.join(campaign);
    reconcile_checkpoints(&campaign_dir, replay_dir)?;
    if read_checkpointed(&campaign_dir)?.is_none() {
        return Ok(None);
    }
    publish_checkpointed_locked(&campaign_dir, replay_dir, campaign, naming)
}

fn publish_checkpointed_locked(
    campaign_dir: &Path,
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Result<Option<PathBuf>> {
    if staged_recording(replay_dir, campaign)?.is_none() {
        return Ok(None);
    }
    if let Some(mut checkpointed) = read_checkpointed(campaign_dir)? {
        checkpointed.publishing = true;
        write_checkpointed(campaign_dir, &checkpointed)?;
    }
    checkpoint_step(
        CheckpointStep::Publishing,
        &replay::publication::unsaved_path(replay_dir, campaign),
    )?;
    let published = publish_unsaved_locked(replay_dir, campaign, naming)?;
    if let Some(published) = &published {
        checkpoint_step(CheckpointStep::Published, published)?;
        let replay = held_recording(published)?;
        record_recording(
            campaign_dir.parent().unwrap(),
            &replay.campaign_id,
            published,
        )?;
        if let Some(mut saved) = read_saved(campaign_dir) {
            saved.replay = published.clone();
            write_saved(campaign_dir, &saved)?;
        }
    }
    accept_checkpoints(campaign_dir)?;
    Ok(published)
}

/// Save a recording's unsaved hours: its unsaved recording becomes it, named as any save names it.
pub fn publish_unsaved(
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Result<Option<PathBuf>> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    publish_unsaved_locked(replay_dir, campaign, naming)
}

fn publish_unsaved_locked(
    replay_dir: &Path,
    campaign: &str,
    naming: Naming<'_>,
) -> Result<Option<PathBuf>> {
    let Some(unsaved) = staged_recording(replay_dir, campaign)? else {
        // Facts a save left waiting once its unsaved recording was gone go to what it saved.
        hand_over_waiting_facts(replay_dir, campaign, None)?;
        return Ok(None);
    };
    let existing = recording_files::recording_of(replay_dir, campaign.as_bytes())
        .map_err(|error| failure(replay_dir, error))?;
    let sidecar = unsaved_facts(&unsaved);
    // The facts its session kept for it; one that cannot be read keeps them waiting.
    let waiting = match fs::read(&sidecar) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(failure(
                &sidecar,
                format!("{error}; nothing saved, and the game facts waiting for it are kept"),
            ));
        }
    };
    // A save that ended before its staged file was removed: the recording holds it already.
    if let Some(existing) = &existing
        && waiting.is_some()
        && held_recording(existing).is_ok_and(|saved| saved.holds(&unsaved).unwrap_or(false))
    {
        replay::publication::posix_delete(&unsaved).map_err(|error| failure(&unsaved, error))?;
        hand_over_waiting_facts(replay_dir, campaign, Some(&unsaved))?;
        return Ok(Some(existing.clone()));
    }
    let staged = held_recording(&unsaved)?;
    staged
        .require_undamaged()
        .map_err(|error| failure(&unsaved, error))?;
    if !staged.campaign_id.eq_ignore_ascii_case(campaign.as_bytes()) {
        return Err(failure(
            &unsaved,
            "existing replay belongs to a different campaign",
        ));
    }
    // The facts its session kept for it are in the copy that is saved; the staged file, which
    // its checkpoint journal names, is only removed once that copy is saved.
    let pack = waiting.as_deref().and_then(|bytes| {
        replay::container::session_facts_for(bytes, &staged.identity).map(<[u8]>::to_vec)
    });
    let with_facts = pack.filter(|_| staged.fact_packs.is_empty()).map(|pack| {
        let mut name = unsaved.file_name().unwrap_or_default().to_owned();
        name.push(".facts.tmp");
        (unsaved.with_file_name(name), pack)
    });
    let source = match &with_facts {
        Some((copy, pack)) => {
            let _ = fs::remove_file(copy);
            staged
                .write_with_facts(copy, pack)
                .map_err(|error| failure(copy, error))?;
            copy.clone()
        }
        None => unsaved.clone(),
    };
    let saved = (|| {
        let replay = held_recording(&source)?;
        let multiplayer = match known_roster(&unsaved).map_err(|error| failure(&unsaved, error))? {
            Some(known) => known,
            None => crate::recording_name::multiplayer(&replay, replay.index.len())
                .map_err(|error| failure(&unsaved, error))?,
        };
        let stem = recording_stem(&replay, naming, multiplayer)
            .map_err(|error| failure(&unsaved, error))?;
        let (recording, unrenamed) = save_recording_file(
            &replay,
            Bytes::Staged(&source),
            replay_dir,
            existing.as_deref(),
            &stem,
            naming,
            |_: &Replay, _: &Path| Ok(()),
            recording_files::rename,
            &|| Ok(()),
        )?;
        if let Some(unrenamed) = unrenamed {
            println!("{unrenamed}");
        }
        remember_roster(&recording, multiplayer).map_err(|error| failure(&recording, error))?;
        Ok(recording)
    })();
    if with_facts.is_some() {
        match &saved {
            Ok(_) => {
                drop(staged);
                replay::publication::posix_delete(&unsaved)
                    .map_err(|error| failure(&unsaved, error))?;
            }
            Err(_) => {
                let _ = fs::remove_file(&source);
            }
        }
    }
    if saved.is_ok() {
        let _ = fs::remove_file(&sidecar);
    }
    saved.map(Some)
}

/// Drop a recording's unsaved hours, keeping what it saved. An ambiguous unsaved recording, one
/// that may hold another campaign's hours, is refused and every file kept.
pub fn discard_unsaved_recording(replay_dir: &Path, campaign: &str) -> Result<()> {
    let _lock = NamesLock::wait(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let unsaved = recording_files::unsaved_recording(replay_dir, campaign.as_bytes())
        .map_err(|error| failure(replay_dir, format!("{error}; nothing discarded")))?;
    // Removed first, so the recording it continued changes only once nothing must hold it.
    if let Some(unsaved) = &unsaved {
        replay::publication::posix_delete(unsaved).map_err(|error| failure(unsaved, error))?;
    }
    hand_over_waiting_facts(replay_dir, campaign, unsaved.as_deref())
}

/// Gives the facts that waited beside the campaign's unsaved recording, at `staged` or where it
/// would be, now removed, to the recording it continued. They stay waiting until it takes them,
/// so a later save or discard retries.
fn hand_over_waiting_facts(replay_dir: &Path, campaign: &str, staged: Option<&Path>) -> Result<()> {
    let sidecar = unsaved_facts(&staged.map_or_else(
        || replay::publication::unsaved_path(replay_dir, campaign),
        Path::to_owned,
    ));
    let bytes = match fs::read(&sidecar) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(failure(
                &sidecar,
                format!("{error}; the game facts waiting for these hours are kept"),
            ));
        }
    };
    if let Some(saved) = recording_files::recording_of(replay_dir, campaign.as_bytes())
        .map_err(|error| failure(replay_dir, error))?
    {
        attach_facts_bytes(&saved, &bytes)
            .map_err(|error| format!("{error}; the game facts waiting for these hours are kept"))?;
    }
    fs::remove_file(&sidecar).map_err(|error| failure(&sidecar, error))
}

/// Adds the session facts `bytes` to `recording` in place when it lacks them and they are its
/// content's.
fn attach_facts_bytes(recording: &Path, bytes: &[u8]) -> Result<()> {
    let mut replay =
        Replay::open_appendable(recording).map_err(|error| failure(recording, error))?;
    if !replay.fact_packs.is_empty() {
        return Ok(());
    }
    let Some(pack) = replay::container::session_facts_for(bytes, &replay.identity) else {
        return Ok(());
    };
    replay
        .attach_facts_in_place(pack)
        .map_err(|error| failure(recording, error))
}

/// Call only once no one writes them. Hours checkpoints put into the unsaved recording go with it,
/// which the caller deletes or takes back.
pub fn discard_unsaved(session_dir: &Path, campaign: &str) -> Result<()> {
    let campaign_dir = session_dir.join(campaign);
    discard_checkpoints(session_dir, campaign)?;
    for (_, path) in segment_paths(&campaign_dir)? {
        replay::publication::posix_delete(&path)
            .map_err(|error| format!("discard {}: {error}", path.display()))?;
    }
    if read_saved(&campaign_dir).is_some_and(|saved| is_unsaved_recording(&saved.replay)) {
        let path = campaign_dir.join(SAVED_FILE);
        replay::publication::posix_delete(&path).map_err(|error| failure(&path, error))?;
    }
    Ok(())
}

// A later load decision can admit another branch of a segment kept after a partial save.
fn remove_held_branches(
    previous: &Replay,
    segments: &mut [(u32, Recovery<Recovered>)],
    mut read: impl FnMut(u32, &FrameMetadata) -> std::io::Result<Frame>,
) -> Result<()> {
    let mut ranges: BTreeMap<BranchId, Vec<(Hour, Hour)>> = BTreeMap::new();
    for index in &previous.index {
        ranges
            .entry(index.branch)
            .or_default()
            .push((index.first_hour, index.last_hour));
    }
    let mut coverage: BTreeMap<BranchId, (bool, bool)> = BTreeMap::new();
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        if let Recovered::Frame(frame) = record {
            let held = ranges.get(&frame.branch).is_some_and(|ranges| {
                let at = ranges.partition_point(|(first, _)| *first <= frame.hour);
                at > 0 && ranges[at - 1].1 >= frame.hour
            });
            let entry = coverage.entry(frame.branch).or_default();
            if held {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
        }
    }
    let held: BTreeSet<_> = coverage
        .iter()
        .filter(|(_, (inside, outside))| *inside && !*outside)
        .map(|(branch, _)| *branch)
        .collect();
    if held.is_empty() {
        return Ok(());
    }
    for branch in &held {
        let mut matched = Vec::new();
        for (number, recovery) in segments.iter() {
            let mut taking = false;
            let mut session = None;
            let mut records = Vec::new();
            for record in &recovery.records {
                if let Recovered::Control(control @ ControlRecord::PlayerCommands { .. }) = record
                    && !previous.controls.contains(control)
                {
                    continue;
                }
                match record {
                    Recovered::SessionStart(_) => {
                        session = Some(record);
                        taking = false;
                    }
                    Recovered::BranchStart(start) => {
                        taking = start.id == *branch;
                        if taking && let Some(session) = session {
                            records.push(session.clone());
                        }
                    }
                    _ => {}
                }
                if taking {
                    records.push(record.clone());
                }
            }
            if !records.is_empty() {
                matched.push((
                    *number,
                    Recovery {
                        format: recovery.format,
                        records,
                        valid_bytes: recovery.valid_bytes,
                        damage: recovery.damage.clone(),
                    },
                ));
            }
        }
        if !already_finalized_with_reader(previous, &matched, &mut read)? {
            return Err("saved branch differs from its retained spool; spool retained".into());
        }
    }
    for (_, recovery) in segments {
        let mut taking = true;
        recovery.records.retain(|record| {
            if let Recovered::Control(control @ ControlRecord::PlayerCommands { .. }) = record {
                return !previous.controls.contains(control);
            }
            match record {
                Recovered::SessionStart(_) => taking = true,
                Recovered::BranchStart(branch) => taking = !held.contains(&branch.id),
                _ => {}
            }
            taking
        });
    }
    Ok(())
}

fn unchanged_context(previous: &Replay, segments: &[(u32, Recovery<Recovered>)]) -> bool {
    let mut controls = Vec::new();
    for (_, recovery) in segments {
        if recovery.damage.is_some() {
            return false;
        }
        for record in &recovery.records {
            match record {
                Recovered::Frame(_) => return false,
                Recovered::SessionStart(session) if !previous.sessions.contains(session) => {
                    return false;
                }
                Recovered::BranchStart(branch) if !previous.branches.contains(branch) => {
                    return false;
                }
                Recovered::Gap(gap) if !previous.gaps.contains(gap) => return false,
                Recovered::Control(control) => {
                    if !(matches!(control, ControlRecord::EventLoss { .. })
                        && controls.contains(&control))
                    {
                        controls.push(control);
                    }
                }
                _ => {}
            }
        }
    }
    is_subsequence(previous.controls.iter(), controls.into_iter())
}

/// Note a frameless tail whose saved recording already holds all it adds.
#[allow(clippy::too_many_arguments)]
fn keep_saved_tail(
    campaign_dir: &Path,
    replay: &Path,
    revision: Revision,
    mut saved: BTreeMap<u32, u64>,
    mut covered: BTreeMap<u32, (u64, u64, Option<(Hour, Hour)>)>,
    lengths: BTreeMap<u32, u64>,
    omitted: &[(Option<i64>, u64)],
    command_bootstrap: &BTreeSet<BranchId>,
    journaled: bool,
) -> Result<()> {
    covered.extend(
        lengths
            .iter()
            .map(|(number, length)| (*number, (*length, 0, None))),
    );
    saved.extend(lengths);
    write_saved(
        campaign_dir,
        &Saved {
            command_bootstrap: command_bootstrap.clone(),
            replay: replay.to_owned(),
            revision,
            segments: saved,
            covered,
            omitted: omitted.to_vec(),
        },
    )?;
    if !journaled {
        accept_checkpoints(campaign_dir)?;
    }
    Ok(())
}

/// A frameless tail that adds only complete explicit loss, less what the recording holds.
fn loss_tail(
    previous: &Replay,
    segments: &[(u32, Recovery<Recovered>)],
) -> Option<Vec<(u32, Recovery<Recovered>)>> {
    let mut recorded = previous.gaps.clone();
    let mut held = Vec::new();
    let mut clocks = Vec::new();
    let mut added = false;
    let mut tail = Vec::with_capacity(segments.len());
    for (number, recovery) in segments {
        if recovery.damage.is_some() {
            return None;
        }
        let mut records = Vec::new();
        let mut session = None;
        let mut branch = None;
        let mut section = Vec::new();
        let mut lost = false;
        for record in &recovery.records {
            match record {
                Recovered::Frame(_) => return None,
                Recovered::SessionStart(start) => {
                    if !previous.sessions.contains(start) {
                        return None;
                    }
                    session = Some(start);
                }
                Recovered::BranchStart(start) => {
                    if !previous.branches.contains(start)
                        && previous.branches.iter().any(|known| known.id == start.id)
                    {
                        return None;
                    }
                    loss_section(&mut records, session, branch, &mut section, lost, false)?;
                    branch = Some(start);
                    lost = false;
                }
                Recovered::Gap(gap) if previous.gaps.contains(gap) => {}
                Recovered::Gap(gap) => {
                    lost = true;
                    // Hours an earlier gap already accounts for lead this one, never follow it.
                    let mut first = Some(gap.first_hour);
                    while let Some(hour) = first
                        && let Some(covering) = recorded.iter().find(|known| {
                            known.branch == gap.branch
                                && known.first_hour <= hour
                                && known.last_hour >= hour
                        })
                    {
                        first =
                            (covering.last_hour < gap.last_hour).then(|| covering.last_hour + 1);
                    }
                    let Some(first) = first else {
                        continue;
                    };
                    if recorded.iter().any(|known| {
                        known.branch == gap.branch
                            && known.first_hour <= gap.last_hour
                            && first <= known.last_hour
                    }) || previous.index.iter().any(|block| {
                        block.branch == gap.branch
                            && block.first_hour <= gap.last_hour
                            && first <= block.last_hour
                    }) {
                        return None;
                    }
                    let gap = Gap {
                        first_hour: first,
                        ..gap.clone()
                    };
                    recorded.push(gap.clone());
                    section.push(Recovered::Gap(gap));
                    added = true;
                }
                Recovered::Control(control) if previous.controls.contains(control) => {
                    if !(matches!(control, ControlRecord::EventLoss { .. })
                        && held.contains(&control))
                    {
                        held.push(control);
                    }
                }
                Recovered::Control(control) => {
                    match control {
                        ControlRecord::ClockWitness { hour, .. } => {
                            clocks.push((branch?.id, *hour));
                        }
                        ControlRecord::EventLoss { .. } | ControlRecord::GapSummary { .. } => {}
                        _ => return None,
                    }
                    section.push(record.clone());
                }
            }
        }
        loss_section(&mut records, session, branch, &mut section, lost, true)?;
        tail.push((
            *number,
            Recovery {
                format: recovery.format,
                records,
                valid_bytes: recovery.valid_bytes,
                damage: None,
            },
        ));
    }
    // A clock of an hour no frame holds must be explained by a gap.
    let gapped = clocks.iter().all(|(branch, hour)| {
        recorded
            .iter()
            .any(|gap| gap.branch == *branch && gap.first_hour <= *hour && gap.last_hour >= *hour)
    });
    (added && gapped && is_subsequence(previous.controls.iter(), held.into_iter())).then_some(tail)
}

/// One branch's new loss records after its context; `lost` says the spool gapped it.
fn loss_section(
    records: &mut Vec<Recovered>,
    session: Option<&SessionStart>,
    branch: Option<&BranchStart>,
    section: &mut Vec<Recovered>,
    lost: bool,
    last: bool,
) -> Option<()> {
    if section.is_empty() {
        return Some(());
    }
    // A branch without a gap may only end its segment, as the spool's own rules allow.
    if !lost
        || (!last
            && !section
                .iter()
                .any(|record| matches!(record, Recovered::Gap(_))))
    {
        return None;
    }
    if records.is_empty() {
        records.push(Recovered::SessionStart(session?.clone()));
    }
    records.push(Recovered::BranchStart(branch?.clone()));
    records.append(section);
    Some(())
}

/// How many leading segments the recording holds exactly; the rest must share no hour with it.
fn published_prefix(
    previous: &Replay,
    segments: &[(u32, Recovery<Recovered>)],
    read: impl FnMut(u32, &FrameMetadata) -> std::io::Result<Frame>,
) -> Result<usize> {
    let conflict = || {
        "spool overlaps a finalized session but differs from it; refusing to append duplicate or conflicting hours".to_string()
    };
    let mut blocks: BTreeMap<BranchId, Vec<(Hour, Hour)>> = BTreeMap::new();
    for index in &previous.index {
        blocks
            .entry(index.branch)
            .or_default()
            .push((index.first_hour, index.last_hour));
    }
    let covered = |branch: BranchId, hour: Hour| {
        blocks.get(&branch).is_some_and(|ranges| {
            let at = ranges.partition_point(|(first, _)| *first <= hour);
            at > 0 && ranges[at - 1].1 >= hour
        })
    };
    let mut published = 0;
    let mut frames = false;
    for (at, (_, recovery)) in segments.iter().enumerate() {
        let (mut inside, mut outside) = (0, 0);
        for record in &recovery.records {
            if let Recovered::Frame(frame) = record {
                if covered(frame.branch, frame.hour) {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        let held = if inside + outside == 0 {
            recovery.records.iter().all(|record| match record {
                Recovered::SessionStart(session) => previous.sessions.contains(session),
                Recovered::BranchStart(branch) => previous.branches.contains(branch),
                Recovered::Gap(gap) => previous.gaps.contains(gap),
                Recovered::Control(control) => previous.controls.contains(control),
                _ => true,
            })
        } else {
            outside == 0
        };
        if inside > 0 && (outside > 0 || published != at) {
            return Err(conflict());
        }
        if held && published == at {
            published += 1;
            frames |= inside > 0;
        }
    }
    if frames && !already_finalized_with_reader(previous, &segments[..published], read)? {
        return Err(conflict());
    }
    Ok(published)
}

/// A repeated request is harmless only when its complete recovered contents
/// already exist. Matching hours alone must never hide different row values.
fn already_finalized_with_reader(
    previous: &Replay,
    segments: &[(u32, Recovery<Recovered>)],
    mut read: impl FnMut(u32, &FrameMetadata) -> std::io::Result<Frame>,
) -> Result<bool> {
    let records: Vec<_> = segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .collect();
    let conflict = || {
        "spool overlaps a finalized session but differs from it; refusing to append duplicate or conflicting hours".to_string()
    };
    let mut expected = BTreeMap::new();
    for (number, recovery) in segments {
        for record in &recovery.records {
            if let Recovered::Frame(frame) = record
                && expected
                    .insert((frame.branch, frame.hour), (*number, frame))
                    .is_some()
            {
                return Err(conflict());
            }
        }
    }
    let mut matched = 0;
    let mut source_decoder = FrameDecoder::new();
    let mut source_segment = None;
    for index in &previous.index {
        if expected
            .range((index.branch, index.first_hour)..=(index.branch, index.last_hour))
            .next()
            .is_none()
        {
            continue;
        }
        let mut replay_decoder = FrameDecoder::new();
        for frame in previous
            .read_block(index)
            .map_err(|error| error.to_string())?
        {
            let decoded = replay_decoder
                .advance(&frame)
                .map_err(|error| error.to_string())?;
            if let Some((number, expected_frame)) = expected.remove(&(frame.branch, frame.hour)) {
                if source_segment != Some(number) {
                    source_decoder = FrameDecoder::new();
                    source_segment = Some(number);
                }
                let expected_frame =
                    read(number, expected_frame).map_err(|error| error.to_string())?;
                let expected_decoded = source_decoder
                    .advance(&expected_frame)
                    .map_err(|error| error.to_string())?;
                // Size-driven keyframes change the wire representation, while
                // every value, availability, origin and metric must stay exact,
                // with table read times as blocks store them.
                if expected_decoded.branch != decoded.branch
                    || expected_decoded.hour != decoded.hour
                    || expected_decoded.tables != decoded.tables
                    || expected_decoded.metrics.stored() != decoded.metrics.stored()
                {
                    return Err(conflict());
                }
                matched += 1;
            }
        }
    }
    if matched == 0 {
        return Ok(false);
    }
    if !expected.is_empty() {
        return Err(conflict());
    }
    for record in &records {
        match record {
            Recovered::SessionStart(session) if !previous.sessions.contains(session) => {
                return Err(conflict());
            }
            Recovered::BranchStart(branch) if !previous.branches.contains(branch) => {
                return Err(conflict());
            }
            Recovered::Gap(gap) if !previous.gaps.contains(gap) => return Err(conflict()),
            _ => {}
        }
    }
    // Finalization keeps a retried event loss once; compare it once too.
    let mut controls: Vec<&ControlRecord> = Vec::new();
    for record in &records {
        if let Recovered::Control(control) = record
            && !(matches!(control, ControlRecord::EventLoss { .. }) && controls.contains(&control))
        {
            controls.push(control);
        }
    }
    if !is_subsequence(previous.controls.iter(), controls.into_iter()) {
        return Err(conflict());
    }
    for (number, recovery) in segments {
        if let Some(reason) = &recovery.damage {
            let session = recovery.records.first().and_then(|record| match record {
                Recovered::SessionStart(session) => Some(session.id),
                Recovered::BranchStart(branch) => Some(branch.session),
                _ => None,
            });
            if !previous.recovery_notes.contains(&RecoveryNote {
                session,
                segment: *number,
                valid_bytes: recovery.valid_bytes as u64,
                reason: reason.clone(),
            }) {
                return Err(conflict());
            }
        }
    }
    Ok(true)
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

/// Print one interval row's owner refusal markers; returns its delayed and
/// decision refusal counts.
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

pub fn inspect(path: &Path) -> Result<()> {
    let replay = held_recording(path)?;
    println!(
        "replay={} format={} bytes={} campaign={:?} keyframe_interval={}",
        path.display(),
        replay.format.version(),
        replay.saved_length(),
        String::from_utf8_lossy(&replay.campaign_id),
        replay.keyframe_interval
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
    for index in &replay.index {
        let sections = replay
            .block_sections(index)
            .map_err(|error| failure(path, error))?
            .ok_or_else(|| failure(path, std::io::Error::other("missing grouped sections")))?;
        let layout = format!("grouped sections={}", sections.len());
        println!(
            "block branch={} hours={}..={} offset={} compressed_bytes={} raw_bytes={} layout={layout}",
            index.branch,
            index.first_hour,
            index.last_hour,
            index.offset,
            index.length,
            index.raw_length
        );
        let mut script_decoder = replay_frame::frame::FrameDecoder::for_tables(&[
            replay_data::rows::TableId::EventFeedHistory,
        ]);
        for frame in replay
            .read_block(index)
            .map_err(|error| failure(path, error))?
        {
            summary.observe(&frame)?;
            let decoded = script_decoder
                .advance(&frame)
                .map_err(|error| failure(path, error))?;
            match decoded.complete::<replay_data::rows::tables::EventFeedHistory>() {
                Ok(rows) => {
                    for row in rows {
                        script_states
                            .entry(index.branch)
                            .or_default()
                            .observe(row)
                            .map_err(|error| failure(path, error))?;
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
                        index.branch, frame.hour
                    );
                }
            }
        }
    }
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
    summary.print(0);
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

/// Every recorded map event, and each family's intervals by coverage.
fn print_map_events(path: &Path, replay: &Replay) -> Result<()> {
    use replay_data::{rows::MapEventFamily, rows::MapEventPayload, rows::TableId, rows::tables};
    let selected = [TableId::MapEvents];
    let mut complete = [0_usize; MapEventFamily::COUNT];
    let mut incomplete: [BTreeMap<String, usize>; MapEventFamily::COUNT] = Default::default();
    let mut events = [0_usize; MapEventFamily::COUNT];
    for index in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay
            .read_block_selected(index, &selected)
            .map_err(|error| failure(path, error))?
        {
            let decoded = decoder
                .advance(&frame)
                .map_err(|error| failure(path, error))?;
            let Ok([row]) = decoded.complete::<tables::MapEvents>() else {
                continue;
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
        }
    }
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

/// `recorder inspect <replay> --characters <substring> --hour <h> --branch <id>`:
/// the recorded Characters rows at that frame whose token or name contains
/// the substring, with their template columns.
pub fn inspect_characters(path: &Path, needle: &str, hour: Hour, branch: BranchId) -> Result<()> {
    let replay = open(path)?;
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

/// One line per Characters row whose token or name contains `needle`, in id
/// order. `template_tag` is shown with its literal from the frame's
/// LexicalTokens when that table is complete and holds the token.
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

fn is_subsequence<'a, T: PartialEq + 'a>(
    mut existing: impl Iterator<Item = &'a T>,
    incoming: impl Iterator<Item = &'a T>,
) -> bool {
    incoming
        .into_iter()
        .all(|wanted| existing.any(|value| value == wanted))
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

#[derive(Default)]
struct FinalizationReport {
    revision: Option<Revision>,
    summary: Box<Summary>,
    spool_bytes: u64,
    seconds: Option<f64>,
}

impl std::fmt::Debug for FinalizationReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FinalizationReport")
            .field("hours", &self.summary.totals.len())
            .field("spool_bytes", &self.spool_bytes)
            .finish()
    }
}

fn merge_reports(
    earlier: Option<Box<FinalizationReport>>,
    later: Option<Box<FinalizationReport>>,
    replay: &Replay,
) -> Result<Option<Box<FinalizationReport>>> {
    let earlier = earlier.filter(|report| {
        report
            .revision
            .is_some_and(|revision| replay.contains_revision(revision).unwrap_or(false))
    });
    match (earlier, later) {
        (Some(mut earlier), Some(later)) => {
            earlier.summary.merge(later.summary)?;
            earlier.revision = later.revision.or(earlier.revision);
            earlier.spool_bytes += later.spool_bytes;
            earlier.seconds = match (earlier.seconds, later.seconds) {
                (Some(earlier), Some(later)) => Some(earlier + later),
                (earlier, later) => earlier.or(later),
            };
            Ok(Some(earlier))
        }
        (earlier, later) => Ok(earlier.or(later)),
    }
}

impl FinalizationReport {
    fn text(&mut self, replay_bytes: u64) -> Result<String> {
        use std::io::Write;
        let mut output = Vec::new();
        writeln!(
            output,
            "finalize_s={} replay_bytes={replay_bytes} spool_bytes={}",
            self.seconds
                .map(|seconds| format!("{seconds:.3}"))
                .unwrap_or_else(|| "not_observed".into()),
            self.spool_bytes
        )
        .map_err(|error| error.to_string())?;
        self.summary
            .write_known(&mut output)
            .map_err(|error| error.to_string())?;
        String::from_utf8(output).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
thread_local! {
    static FINALIZATION_OUTPUT: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

fn emit_finalization(text: &str) {
    #[cfg(test)]
    FINALIZATION_OUTPUT.with_borrow_mut(|output| {
        if let Some(output) = output {
            output.push(text.to_owned());
        }
    });
    print!("{text}");
}

struct Summary {
    decoder: FrameDecoder,
    totals: Vec<u64>,
    table_times: [Vec<u64>; TABLE_COUNT],
    /// Hours whose recording keeps which tables were read but not their times.
    untimed_hours: u64,
    table_bytes: [u64; TABLE_COUNT],
    states: [[u64; 5]; TABLE_COUNT],
    max_pool: u64,
    max_dropped: u64,
    hours: BTreeMap<BranchId, BTreeSet<Hour>>,
    issues: [Vec<Issue>; TABLE_COUNT],
    /// The hour [`Summary::observe`] observed last.
    previous: Option<Observed>,
    /// An observed hour's roster held more than one human.
    multiplayer: bool,
}

impl Default for Summary {
    fn default() -> Self {
        Self {
            decoder: FrameDecoder::default(),
            totals: Vec::new(),
            table_times: std::array::from_fn(|_| Vec::new()),
            untimed_hours: 0,
            table_bytes: [0; TABLE_COUNT],
            states: [[0; 5]; TABLE_COUNT],
            max_pool: 0,
            max_dropped: 0,
            hours: BTreeMap::new(),
            issues: std::array::from_fn(|_| Vec::new()),
            previous: None,
            multiplayer: false,
        }
    }
}

impl Summary {
    fn merge(&mut self, mut later: Box<Summary>) -> Result<()> {
        if self.totals.is_empty() && self.hours.is_empty() {
            let multiplayer = self.multiplayer;
            *self = *later;
            self.multiplayer |= multiplayer;
            return Ok(());
        }
        for (branch, hours) in &later.hours {
            if self
                .hours
                .get(branch)
                .is_some_and(|earlier| !earlier.is_disjoint(hours))
            {
                return Err(format!(
                    "checkpoint summary repeats hours on branch {branch}"
                ));
            }
        }
        self.totals.append(&mut later.totals);
        self.untimed_hours += later.untimed_hours;
        self.max_pool = self.max_pool.max(later.max_pool);
        self.max_dropped = self.max_dropped.max(later.max_dropped);
        self.multiplayer |= later.multiplayer;
        for (branch, hours) in std::mem::take(&mut later.hours) {
            self.hours.entry(branch).or_default().extend(hours);
        }
        for i in 0..TABLE_COUNT {
            self.table_times[i].append(&mut later.table_times[i]);
            self.table_bytes[i] += later.table_bytes[i];
            for state in 0..5 {
                self.states[i][state] += later.states[i][state];
            }
            self.issues[i].append(&mut later.issues[i]);
        }
        Ok(())
    }

    fn observe(&mut self, frame: &Frame) -> Result<()> {
        let decoded = self
            .decoder
            .apply(frame)
            .map_err(|error| error.to_string())?;
        let sizes = frame
            .table_encoded_sizes()
            .map_err(|error| error.to_string())?;
        let hour = Observed::new(frame, &decoded, sizes, self.previous.as_ref())
            .map_err(|error| error.to_string())?;
        self.observe_hour(&hour)?;
        self.previous = Some(hour);
        Ok(())
    }

    fn observe_hour(&mut self, hour: &Observed) -> Result<()> {
        self.multiplayer |= hour.players > 1;
        self.totals.push(hour.metrics.total_ns);
        self.max_pool = self.max_pool.max(hour.metrics.pool_bytes);
        self.max_dropped = self.max_dropped.max(hour.metrics.dropped_frames);
        if !self.hours.entry(hour.branch).or_default().insert(hour.hour) {
            return Err(format!(
                "duplicate frame on branch {} at hour {}",
                hour.branch, hour.hour
            ));
        }
        let timed = hour.metrics.table_times_recorded();
        self.untimed_hours += u64::from(!timed);
        for table in TableId::ALL {
            let i = table.index();
            if !self.recorded(table) || hour.updates[i] == UpdateKind::NotRecorded {
                continue;
            }
            self.table_bytes[i] += hour.sizes[i] as u64;
            if timed {
                self.table_times[i].push(hour.metrics.table_ns[i]);
            }
            let state = match hour.updates[i] {
                UpdateKind::Snapshot => 0,
                UpdateKind::Delta => 1,
                UpdateKind::Unchanged => 2,
                UpdateKind::Truncated => 3,
                UpdateKind::Unavailable | UpdateKind::Incompatible => 4,
                UpdateKind::NotRecorded => unreachable!("skipped above"),
            };
            self.states[i][state] += 1;
        }
        for (table, field, count) in &hour.field_gaps {
            let name = if field.contains('.') {
                (*field).to_owned()
            } else {
                format!("{table:?}.{field}")
            };
            println!(
                "Compatibility field gap: {name}: {count} occurrences not recorded at hour {}",
                hour.hour
            );
        }
        for (table, incomplete) in &hour.incomplete {
            let reason = match incomplete {
                Incomplete::Truncated { missing } => {
                    format!("truncated; {missing} rows not read")
                }
                Incomplete::Unavailable { reason } => format!("unavailable; {reason}"),
            };
            let issues = &mut self.issues[table.index()];
            if let Some(previous) = issues.last_mut()
                && previous.branch == hour.branch
                && previous.last.checked_add(1) == Some(hour.hour)
                && previous.reason == reason
            {
                previous.last = hour.hour;
            } else {
                issues.push(Issue {
                    branch: hour.branch,
                    table: *table,
                    first: hour.hour,
                    last: hour.hour,
                    reason,
                });
            }
        }
        Ok(())
    }

    fn recorded(&self, table: TableId) -> bool {
        FormatVersion::CURRENT.table_refusal(table).is_none()
    }

    fn print(&mut self, spool_bytes: u64) {
        if let Err(error) = self.write(spool_bytes, &mut std::io::stdout().lock()) {
            eprintln!("Summary could not be printed: {error}");
        }
    }

    fn write_known(&self, output: &mut impl std::io::Write) -> std::io::Result<()> {
        let hours = self.totals.len();
        if hours == 0 {
            return writeln!(
                output,
                "hours=0 total_p50_ms=not_observed total_p95_ms=not_observed total_max_ms=not_observed pool_max_bytes=not_observed dropped_counter_max=not_observed"
            );
        }
        writeln!(
            output,
            "hours={hours} total_p50_ms=not_summarized total_p95_ms=not_summarized total_max_ms=not_summarized pool_max_bytes={} dropped_counter_max={}",
            self.max_pool, self.max_dropped
        )
    }

    fn write(&mut self, spool_bytes: u64, output: &mut impl std::io::Write) -> std::io::Result<()> {
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
            "hours={hours} total_p50_ms={p50:.3} total_p95_ms={p95:.3} total_max_ms={maximum:.3} pool_max_bytes={} dropped_counter_max={}",
            self.max_pool, self.max_dropped
        )?;
        if spool_bytes != 0 && hours != 0 {
            writeln!(
                output,
                "spool_bytes_per_hour={:.3}",
                spool_bytes as f64 / hours as f64
            )?;
        }
        writeln!(
            output,
            "table raw_encoded_bytes bytes_per_hour read_p50_ms read_p95_ms read_max_ms snapshots deltas unchanged truncated unavailable"
        )?;
        for table in TableId::ALL {
            let i = table.index();
            // Every hour counts each table but one its capture mode leaves out: no row, not zeros.
            if !self.recorded(table)
                || (hours != 0 && self.states[i].iter().all(|&count| count == 0))
            {
                continue;
            }
            let times = self.read_times(table);
            let [snapshots, deltas, unchanged, truncated, unavailable] = self.states[i];
            let per_hour = if hours == 0 {
                0.0
            } else {
                self.table_bytes[i] as f64 / hours as f64
            };
            writeln!(
                output,
                "{table:?} {} {per_hour:.3} {times} {snapshots} {deltas} {unchanged} {truncated} {unavailable}",
                self.table_bytes[i]
            )?;
        }
        Ok(())
    }

    /// A table's read p50, p95 and maximum in milliseconds over the hours
    /// whose read times are recorded; replay blocks keep only which tables
    /// were read, so a replay without timed hours reports them not recorded.
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

// A None marks a branch or gap boundary in a recovered spool's record stream.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_json_publications_own_distinct_temporary_files() {
        let directory = recorder_test_support::test_support::allocation_test_directory(
            "json-publication-interleaving",
        );
        let path = directory.join("completed.json");
        write_json_before_publish(&path, &json!({"writer":"first"}), |temporary| {
            let first = fs::read(temporary).unwrap();
            write_json_before_publish(&path, &json!({"writer":"second"}), |second| {
                assert_ne!(temporary, second);
                assert_eq!(fs::read(temporary).unwrap(), first);
                Ok(())
            })?;
            let published: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(published["writer"], "second");
            assert_eq!(fs::read(temporary).unwrap(), first);
            Ok(())
        })
        .unwrap();
        let published: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(published["writer"], "first");
        assert!(
            write_json_before_publish(&path, &json!({"writer":"aborted"}), |_| {
                Err("synthetic publication failure".into())
            })
            .is_err()
        );
        let published: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(published["writer"], "first");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    /// A native recording of `runs`, each a branch, its first hour and its hours, of the
    /// allocation campaign, every hour of every table unread.
    pub(super) fn native_recording(runs: &[(u64, i32, i32)]) -> Vec<u8> {
        native_recording_with_controls(runs, &[])
    }

    fn native_recording_with_controls(runs: &[(u64, i32, i32)], controls: &[Record]) -> Vec<u8> {
        use replay::{
            columnar_file::{Capture, FileWriter, Hour, Settings},
            columnar_frames::schema,
            native_frame::FrameWriter,
        };
        use replay_data::identity::IdentityCodec;
        use replay_data::{identity::CaptureMode, rows::TABLE_COUNT};
        use replay_frame::frame::{FrameInput, Metrics, TableInput};
        let Record::SessionStart(session) = crate::test_support::allocation_records().remove(0)
        else {
            unreachable!()
        };
        let mut session_bytes = Vec::new();
        session.encode(&mut session_bytes).unwrap();
        let mut content = Vec::new();
        session.identity.encode(&mut content).unwrap();
        let campaign = recorder_test_support::test_support::ALLOCATION_CAMPAIGN.to_vec();
        let schema = schema().unwrap();
        let mut file = FileWriter::create(
            std::io::Cursor::new(Vec::new()),
            replay_columnar::envelope::Identity {
                schema: replay_columnar::envelope::schema_digest(&schema),
                campaign: campaign.clone(),
                content: content.clone(),
                capture_mode: CaptureMode::Full.byte(),
            },
            schema.iter().map(Vec::len).collect(),
            Settings::RECORDING,
        )
        .unwrap();
        for &(branch, first, hours) in runs {
            let mut writer = FrameWriter::new().unwrap();
            file.start_run(branch, replay_columnar::envelope::epoch::LOAD, first)
                .unwrap();
            for hour in first..first + hours {
                let input = FrameInput {
                    branch,
                    hour,
                    tables: std::array::from_fn(|index| {
                        TableInput::Sampled(
                            if replay_data::scope::records(CaptureMode::Full, TableId::ALL[index]) {
                                TableView::Unavailable {
                                    reason: "not in this test".into(),
                                }
                            } else {
                                TableView::NotRecorded {
                                    mode: CaptureMode::Full,
                                }
                            },
                        )
                    }),
                    metrics: Metrics {
                        total_ns: 1,
                        table_ns: [0; TABLE_COUNT],
                        pool_bytes: 0,
                        dropped_frames: 0,
                    },
                };
                let prepared = writer.hour(&input).unwrap();
                let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
                file.hour(Hour {
                    hour,
                    manifest: &prepared.manifest,
                    sections: &sections,
                    statuses: prepared.statuses(),
                    read: prepared.read(),
                })
                .unwrap();
                drop(sections);
                writer.commit(prepared).unwrap();
            }
        }
        file.finish_with_capture(
            &Capture {
                campaign_id: campaign,
                content,
                capture_mode: CaptureMode::Full.byte(),
                sessions: vec![session_bytes],
                run_sessions: vec![0; runs.len()],
                stop: None,
                format: replay_data::format::FormatVersion::CURRENT.version(),
                controls: controls
                    .iter()
                    .map(|record| replay::spool::encode_record(record).unwrap())
                    .collect(),
            },
            |_| Ok(()),
        )
        .unwrap()
        .file
        .into_inner()
    }

    /// Names by tag, with no name chosen.
    fn unnamed() -> Naming<'static> {
        static SETTINGS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        Naming {
            localisation: None,
            report: None,
            settings: SETTINGS.get_or_init(|| {
                std::env::temp_dir().join(format!(
                    "hoi4-unwritten-settings-{}.json",
                    std::process::id()
                ))
            }),
        }
    }
    use replay::spool::recover_prefix;
    use replay_frame::{branches::BranchReason, branches::BranchStart};

    #[test]
    fn a_session_without_campaign_segments_has_no_spool() {
        let session = std::env::temp_dir().join(format!(
            "hoi4-empty-session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(session.join("bin")).unwrap();
        fs::write(session.join("bin/collector.dll"), b"synthetic").unwrap();
        fs::write(session.join("status.json"), b"{}").unwrap();
        let campaign = session.join("00000000-0000-4000-8000-000000000000");
        fs::create_dir_all(&campaign).unwrap();
        assert!(!has_campaign_spools(&session).unwrap());
        fs::write(campaign.join("spool.0001.h4s"), b"synthetic").unwrap();
        assert!(has_campaign_spools(&session).unwrap());
        fs::remove_dir_all(session).unwrap();
    }

    /// A campaign's finished captures publish in capture order, each as its session's next in
    /// the campaign's folder. A partial one its collector still holds holds back the later ones,
    /// one released with no revision to finish it from is left, and one whose move landed its
    /// copy but kept its source is not published again.
    #[test]
    fn captures_publish_in_order_as_their_sessions_next() {
        let root =
            recorder_test_support::test_support::allocation_test_directory("native-captures");
        let campaign = root.join("spool").join("session-1").join("Campaign-A");
        let replays = root.join("replays");
        let folder = replay::columnar_file::campaign_folder(&replays, b"campaign-a");
        fs::create_dir_all(&campaign).unwrap();
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("session-1.0.h4col"), b"published before").unwrap();
        fs::write(folder.join("session-2.4.h4col"), b"another session's").unwrap();
        fs::write(campaign.join("columnar.0.h4col"), b"first").unwrap();
        let partial = campaign.join("columnar.1.h4col.partial");
        fs::write(&partial, b"still written").unwrap();
        fs::write(campaign.join("columnar.2.h4col"), b"third").unwrap();
        let written = fs::OpenOptions::new().write(true).open(&partial).unwrap();
        publish_columnar(&campaign, &replays, None, false).unwrap();
        assert_eq!(fs::read(folder.join("session-1.1.h4col")).unwrap(), b"first");
        assert!(campaign.join("columnar.2.h4col").exists());
        drop(written);
        publish_columnar(&campaign, &replays, None, false).unwrap();
        assert!(partial.exists());
        assert_eq!(fs::read(folder.join("session-1.2.h4col")).unwrap(), b"third");
        fs::write(campaign.join("columnar.3.h4col"), b"third").unwrap();
        publish_columnar(&campaign, &replays, None, false).unwrap();
        assert_eq!(fs::read_dir(&campaign).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 4);
        fs::remove_dir_all(root).unwrap();
    }

    /// A capture that cannot be published fails its campaign's save and stays in the spool.
    #[test]
    fn unpublished_captures_fail_the_save() {
        let root =
            recorder_test_support::test_support::allocation_test_directory("native-unpublished");
        let campaign = root.join("spool").join("session-1").join("Campaign-A");
        let replays = root.join("replays");
        fs::create_dir_all(&campaign).unwrap();
        fs::create_dir_all(&replays).unwrap();
        // A file where the campaign's capture folder belongs.
        fs::write(replay::columnar_file::campaign_folder(&replays, b"campaign-a"), b"").unwrap();
        fs::write(campaign.join("columnar.0.h4col"), b"first").unwrap();
        let error = publish_columnar(&campaign, &replays, None, false).unwrap_err();
        assert!(error.contains("columnar recordings not published"), "{error}");
        assert_eq!(fs::read(campaign.join("columnar.0.h4col")).unwrap(), b"first");
        fs::remove_dir_all(root).unwrap();
    }

    /// A capture held open waits while its session records, and fails the save of one ended.
    #[test]
    fn held_captures_fail_an_ended_session_save() {
        let root = recorder_test_support::test_support::allocation_test_directory("native-held");
        let campaign = root.join("spool").join("session-1").join("Campaign-A");
        let replays = root.join("replays");
        fs::create_dir_all(&campaign).unwrap();
        let partial = campaign.join("columnar.0.h4col.partial");
        fs::write(&partial, b"still held").unwrap();
        let held = fs::OpenOptions::new().write(true).open(&partial).unwrap();
        publish_columnar(&campaign, &replays, None, false).unwrap();
        let error = publish_columnar(&campaign, &replays, None, true).unwrap_err();
        assert!(error.contains("held open"), "{error}");
        drop(held);
        assert_eq!(fs::read(&partial).unwrap(), b"still held");
        fs::remove_dir_all(root).unwrap();
    }

    pub(super) fn long_finalization_records(promote_hour: Option<i32>) -> Vec<Record> {
        use replay_data::{rows::ClockRow, rows::LexicalTokenRow};
        use replay_frame::frame::FrameEncoder;
        let mut base = crate::test_support::allocation_records();
        let Record::Frame(first) = &base[2] else {
            unreachable!()
        };
        let template = FrameDecoder::new().apply(first).unwrap();
        base.truncate(2);
        let mut encoder = FrameEncoder::new();
        for hour in 2..=21 {
            let mut decoded = template.clone();
            decoded.hour = hour;
            *decoded.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    ..Default::default()
                }]));
            *decoded.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 0,
                    literal: vec![hour as u8; 64 * 1024],
                }]));
            base.push(Record::Frame(
                encoder.encode(decoded, promote_hour == Some(hour)).unwrap(),
            ));
        }
        base.push(Record::Control(ControlRecord::Detach {
            session: 7,
            last_hour: Some(21),
        }));
        base
    }

    fn published_frames(path: &Path) -> Vec<(BranchId, Hour)> {
        let replay = open(path).unwrap();
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| (frame.branch, frame.hour))
            .collect()
    }

    #[test]
    fn command_resume_preserves_failed_frames_from_saved_and_spooled_history() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_records};
        use replay::commands::{Batch, Packet, Phase};

        let directory = std::env::temp_dir()
            .join(format!("hoi4-command-gap-resume-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let original = allocation_records();
        let Record::SessionStart(session) = &original[0] else {
            unreachable!()
        };
        let Record::BranchStart(mut start) = original[1].clone() else {
            unreachable!()
        };
        start.loaded_hour = None;
        let hour = start.first_hour;
        let mut packet = Packet::pack(start.id, 1, 1, 0, 100, 200, 0, &Batch::default()).unwrap();
        packet.phase = Some(Phase {
            start: start.clone(),
            campaign: *ALLOCATION_CAMPAIGN,
            qpc: 100,
            hour,
            load_signal: 19,
        });
        let base = vec![
            Record::SessionStart(session.clone()),
            Record::Control(ControlRecord::PlayerCommands {
                session: session.id,
                packet,
            }),
        ];
        let gap = Record::Gap(Gap {
            branch: start.id,
            first_hour: hour,
            last_hour: hour,
            reason: "synthetic failed frame".into(),
        });
        let with_gap = |mut records: Vec<Record>| {
            records.push(Record::BranchStart(start.clone()));
            records.push(gap.clone());
            records
        };
        let recover = |records: &[Record]| -> Recovery<Recovered> {
            let mut bytes = Vec::new();
            for record in records {
                bytes.extend(replay::spool::encode_record(record).unwrap());
            }
            let recovered = recover_reader(
                std::io::Cursor::new(&bytes),
                bytes.len(),
                Recovered::keep,
                |_| Ok(()),
            )
            .unwrap();
            assert!(recovered.damage.is_none(), "{:?}", recovered.damage);
            assert_eq!(recovered.valid_bytes, bytes.len());
            assert_eq!(recovered.records.len(), records.len());
            for record in records {
                if let Record::Gap(gap) = record {
                    assert!(recovered.records.contains(&Recovered::Gap(gap.clone())));
                }
            }
            recovered
        };
        let failed = with_gap(base.clone());
        let history = Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[failed.clone()])
            .unwrap()
            .into_history()
            .unwrap();
        assert!(history.branches[0].hours.is_empty());
        assert_eq!(history.branches[0].last_missing_hour, Some(hour));
        let mut live = BranchTracker::new();
        live.seed_recorded_branch(start.clone(), std::iter::empty::<Hour>())
            .unwrap();
        live.resume_commands(session.id, start.id, hour, Some(19))
            .unwrap();
        assert_eq!(
            live.observe(None, hour, Some(19), &[]).unwrap().branch,
            start.id
        );
        live.lose_hour(start.id, hour).unwrap();
        live.observe_commands(hour, Some(19)).unwrap();
        let repeated = live.observe(None, hour, Some(19), &[]).unwrap().branch;
        assert_ne!(repeated, start.id);
        let mut restored = history.tracker_for(session).unwrap();
        restored.observe_commands(hour, Some(19)).unwrap();
        assert_eq!(
            restored.observe(None, hour, Some(19), &[]).unwrap().branch,
            repeated
        );
        assert!(restored.recorded_hours(start.id).unwrap().is_empty());
        let mut later = history.tracker_for(session).unwrap();
        assert_eq!(
            later
                .observe(Some(hour + 1), hour + 1, Some(19), &[])
                .unwrap()
                .branch,
            start.id
        );
        assert!(later.recorded_hours(start.id).unwrap().is_empty());

        let mut newer_command = base.clone();
        let Record::Control(ControlRecord::PlayerCommands { packet, .. }) = &mut newer_command[1]
        else {
            unreachable!()
        };
        packet.phase.as_mut().unwrap().hour = hour + 1;
        let newer_command = with_gap(newer_command);
        let newer = Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[newer_command])
            .unwrap()
            .into_history()
            .unwrap();
        let mut later = newer.tracker_for(session).unwrap();
        assert_eq!(
            later.observe(None, hour + 1, Some(19), &[]).unwrap().branch,
            start.id
        );

        for (spool_gap, saved_gap) in [(true, None), (false, Some(true)), (true, Some(false))] {
            let mut records = base.clone();
            if spool_gap {
                records = with_gap(records);
            }
            let segments = [(0, recover(&records))];
            let saved = saved_gap.map(|saved_gap| {
                let mut records = base.clone();
                if saved_gap {
                    records = with_gap(records);
                }
                let path = directory.join("saved.battleplan");
                Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[records])
                    .unwrap()
                    .write_atomic(&path)
                    .unwrap();
                path
            });
            for with_context in [false, true] {
                let contexts = if with_context {
                    vec![UncommittedRange {
                        campaign_id: *ALLOCATION_CAMPAIGN,
                        branch: start.id,
                        load_signal: Some(19),
                        first_hour: hour + 1,
                        last_hour: hour + 1,
                    }]
                } else {
                    Vec::new()
                };
                let mut restored = failure_tracker(
                    ALLOCATION_CAMPAIGN,
                    &segments,
                    session.id,
                    saved.as_deref(),
                    &contexts,
                )
                .unwrap();
                restored.observe_commands(hour, Some(19)).unwrap();
                assert_eq!(
                    restored.observe(None, hour, Some(19), &[]).unwrap().branch,
                    repeated
                );
                assert!(restored.recorded_hours(start.id).unwrap().is_empty());
            }
        }
        let mut recorded = original.clone();
        recorded[1] = Record::BranchStart(start.clone());
        let saved = directory.join("saved.battleplan");
        Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[recorded])
            .unwrap()
            .write_atomic(&saved)
            .unwrap();
        let segments = [(0, recover(&base))];
        let tracker = failure_tracker(
            ALLOCATION_CAMPAIGN,
            &segments,
            session.id,
            Some(&saved),
            &[],
        )
        .unwrap();
        assert_eq!(tracker.last_observation(), Some((start.id, 4)));
        assert_eq!(tracker.recorded_hours(start.id).unwrap(), [2, 3, 4]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn command_only_loads_finalize_filter_and_resume_through_the_recorder() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_records, write_campaign_spool};
        use replay::{
            columnar_file::{Capture, FileWriter, Settings},
            commands::{Batch, Event, Origin, Packet, Phase},
        };
        use replay_data::identity::IdentityCodec;
        for (prior_hours, bootstrap_load) in [(false, 0), (false, 19), (true, 19)] {
            let root = std::env::temp_dir()
                .join(format!(
                    "hoi4-command-finalize-{}-{prior_hours}-{bootstrap_load}",
                    std::process::id()
                ));
            let session_dir = root.join("session");
            let target = root.join("output");
            let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
            let mut original = allocation_records();
            let Record::SessionStart(session) = &mut original[0] else {
                unreachable!()
            };
            session.identity.verification = Verification::Verified {
                launcher: Default::default(),
                memory: Default::default(),
            };
            let session = session.clone();
            let Record::BranchStart(mut start) = original[1].clone() else {
                unreachable!()
            };
            let campaign = session_dir.join(name);
            fs::create_dir_all(&campaign).unwrap();
            begin_admissions(&session_dir, &target).unwrap();
            let mut admissions = Admissions::default();
            admissions
                .admit(&session_dir, &target, name, bootstrap_load)
                .unwrap();
            if prior_hours {
                write_campaign_spool(&session_dir, &original);
                let initial = finalize_session_outcome(&session_dir, &target).unwrap();
                assert!(initial.errors.is_empty(), "{:?}", initial.errors);
                assert_eq!(initial.replays.len(), 1);
            } else {
                start.loaded_hour = None;
            }
            let from = if prior_hours { 500 } else { 100 };
            let mut controls = Vec::new();
            for (branch, load, qpc) in [(9, bootstrap_load, from), (10, 23, from + 200)] {
                let mut branch_start = start.clone();
                branch_start.id = branch;
                if branch != 9 {
                    branch_start.reason = BranchReason::LoadSignal;
                }
                let batch = Batch {
                    events: vec![Event {
                        paused: false,
                        qpc: qpc + 1,
                        hour: if prior_hours { 5 } else { 2 },
                        actor: 0,
                        player: 7,
                        kind: 0,
                        command_country: None,
                        origin: Origin::Unresolved,
                        sequence: branch as u32,
                    }],
                    ..Default::default()
                };
                let mut packet =
                    Packet::pack(branch, branch, branch, 0, qpc, qpc + 100, 0, &batch).unwrap();
                packet.phase = Some(Phase {
                    start: branch_start,
                    campaign: *ALLOCATION_CAMPAIGN,
                    qpc,
                    hour: if prior_hours { 5 } else { 2 },
                    load_signal: load,
                });
                controls.push(ControlRecord::PlayerCommands {
                    session: session.id,
                    packet,
                });
            }
            let mut records = vec![Record::SessionStart(session.clone())];
            records.extend(controls.iter().cloned().map(Record::Control));
            let bytes: Vec<_> = records
                .iter()
                .flat_map(|record| replay::spool::encode_record(record).unwrap())
                .collect();
            fs::write(
                campaign.join(if prior_hours {
                    "spool.0001.h4s"
                } else {
                    "spool.0000.h4s"
                }),
                bytes,
            )
            .unwrap();
            let mut session_bytes = Vec::new();
            session.encode(&mut session_bytes).unwrap();
            let mut content = Vec::new();
            session.identity.encode(&mut content).unwrap();
            let schema = replay::columnar_frames::schema().unwrap();
            let capture = Capture {
                campaign_id: ALLOCATION_CAMPAIGN.to_vec(),
                content: content.clone(),
                capture_mode: session.capture_mode.byte(),
                sessions: vec![session_bytes],
                run_sessions: Vec::new(),
                stop: None,
                format: replay_data::FORMAT_VERSION,
                controls: controls
                    .iter()
                    .map(|control| {
                        replay::spool::encode_record(&Record::Control(control.clone())).unwrap()
                    })
                    .collect(),
            };
            let writer = FileWriter::create(
                fs::File::create(campaign.join("columnar.0.h4col")).unwrap(),
                replay_columnar::envelope::Identity {
                    schema: replay_columnar::envelope::schema_digest(&schema),
                    campaign: ALLOCATION_CAMPAIGN.to_vec(),
                    content,
                    capture_mode: session.capture_mode.byte(),
                },
                schema.iter().map(Vec::len).collect(),
                Settings::RECORDING,
            )
            .unwrap();
            let finished = writer
                .finish_with_capture(&capture, |file| file.sync_data())
                .unwrap();
            drop(finished);
            assert_eq!(
                spool_loads(&campaign).unwrap(),
                if prior_hours {
                    vec![(Some(19), 3), (Some(23), 0)]
                } else {
                    vec![(Some(bootstrap_load), 0), (Some(23), 0)]
                }
            );
            let finalized = finalize_session_outcome(&session_dir, &target).unwrap();
            assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
            assert_eq!(finalized.replays.len(), 1);
            let path = &finalized.replays[0];
            let replay = Replay::open_path(path).unwrap();
            let packets: Vec<_> = replay
                .controls
                .iter()
                .filter_map(|control| match control {
                    ControlRecord::PlayerCommands { packet, .. } => Some(packet),
                    _ => None,
                })
                .collect();
            assert_eq!(packets.len(), 1);
            assert_eq!(packets[0].branch, 9);
            assert_eq!(
                packets[0].phase.as_ref().unwrap().load_signal,
                bootstrap_load
            );
            assert_eq!(held_signal(&replay, 9).unwrap(), Some(bootstrap_load));
            if bootstrap_load == 0 {
                let range = UncommittedRange {
                    campaign_id: *ALLOCATION_CAMPAIGN,
                    branch: 9,
                    load_signal: Some(19),
                    first_hour: 2,
                    last_hour: 2,
                };
                let witness = TerminalWitness {
                    branch: Some(9),
                    load_signal: Some(19),
                    failed_hour: Some(3),
                    uncommitted: Some(range),
                    ..crate::test_support::allocation_witness(0)
                };
                let segments = [(
                    0,
                    Recovery {
                        format: Some(FormatVersion::CURRENT),
                        records: vec![Recovered::SessionStart(session.clone())],
                        valid_bytes: 0,
                        damage: None,
                    },
                )];
                let (failed, _) = failure_records_inner(
                    &segments,
                    &witness,
                    true,
                    Some(&range),
                    Some(Held {
                        recording: &replay,
                        path,
                    }),
                    None,
                    Some(&[range]),
                )
                .unwrap();
                let gaps: Vec<_> = failed.iter().filter_map(|record| match record {
                    Recovered::Gap(gap) => Some((gap.branch, gap.first_hour, gap.last_hour)),
                    _ => None,
                }).collect();
                assert_eq!(gaps, [(9, 2, 3)]);
            }
            let history = replay.into_history().unwrap();
            assert_eq!(history.session_tail(session.id).unwrap().hour, if prior_hours { 5 } else { 2 });
            assert!(!history.session_tail(session.id).unwrap().recorded);
            let tracker = history.tracker_for(&session).unwrap();
            assert_eq!(tracker.current_branch(), Some(9));
            assert_eq!(
                tracker.recorded_hours(9).unwrap().len(),
                if prior_hours { 3 } else { 0 }
            );
            let columns = replay::columnar_file::campaign_folder(&target, ALLOCATION_CAMPAIGN);
            let paths: Vec<_> = fs::read_dir(columns)
                .unwrap()
                .flatten()
                .filter(|file| {
                    file.path()
                        .extension()
                        .is_some_and(|extension| extension == "h4col")
                })
                .collect();
            assert_eq!(paths.len(), 1);
            let recording = replay::commands::Recording::open_path(&paths[0].path()).unwrap();
            let packets: Vec<_> = recording
                .controls
                .iter()
                .filter_map(|control| match control {
                    ControlRecord::PlayerCommands { packet, .. } => Some(packet),
                    _ => None,
                })
                .collect();
            assert_eq!(packets.len(), 1);
            assert_eq!(packets[0].branch, 9);
            let saved = fs::read(path).unwrap();
            let repeated = finalize_session_outcome(&session_dir, &target).unwrap();
            assert!(repeated.errors.is_empty(), "{:?}", repeated.errors);
            assert_eq!(fs::read(path).unwrap(), saved);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn bootstrap_failure_keeps_admitted_hours_with_and_without_a_spool_tail() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_witness, write_campaign_spool,
        };
        use replay::commands::{Batch, Packet, Phase};
        for shape in [0, 1, 2] {
            let root = std::env::temp_dir().join(format!(
                "hoi4-command-bootstrap-failure-{}-{shape}",
                std::process::id()
            ));
            let session_dir = root.join("session");
            let target = root.join("output");
            let original = allocation_records();
            let Record::SessionStart(session) = &original[0] else {
                unreachable!()
            };
            let Record::BranchStart(mut start) = original[1].clone() else {
                unreachable!()
            };
            start.loaded_hour = None;
            let mut packet = Packet::pack(9, 1, 1, 0, 100, 150, 0, &Batch::default()).unwrap();
            packet.phase = Some(Phase {
                start,
                campaign: *ALLOCATION_CAMPAIGN,
                qpc: 100,
                hour: 2,
                load_signal: 0,
            });
            let records = vec![
                original[0].clone(),
                Record::Control(ControlRecord::PlayerCommands { session: 7, packet }),
            ];
            let campaign = write_campaign_spool(&session_dir, &records);
            begin_admissions(&session_dir, &target).unwrap();
            Admissions::default()
                .admit(
                    &session_dir,
                    &target,
                    std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap(),
                    0,
                )
                .unwrap();
            let first = finalize_session_outcome(&session_dir, &target).unwrap();
            assert!(first.errors.is_empty(), "{:?}", first.errors);
            assert_eq!(first.replays.len(), 1);
            assert!(
                read_saved(&campaign)
                    .unwrap()
                    .command_bootstrap
                    .contains(&9)
            );
            let spool = campaign.join("spool.0000.h4s");
            let mut committed_bytes = fs::metadata(&spool).unwrap().len();
            if shape != 0 {
                let packet = Packet::pack(9, 1, 1, 1, 150, 200, 0, &Batch::default()).unwrap();
                let bytes =
                    replay::spool::encode_record(&Record::Control(ControlRecord::PlayerCommands {
                        session: 7,
                        packet,
                    }))
                    .unwrap();
                if shape == 1 {
                    use std::io::Write;
                    fs::OpenOptions::new()
                        .append(true)
                        .open(&spool)
                        .unwrap()
                        .write_all(&bytes)
                        .unwrap();
                    committed_bytes += bytes.len() as u64;
                } else {
                    let mut prefix = replay::spool::encode_record(&original[0]).unwrap();
                    prefix.extend_from_slice(&bytes);
                    committed_bytes = prefix.len() as u64;
                    fs::write(campaign.join("spool.0001.h4s"), prefix).unwrap();
                    fs::remove_file(&spool).unwrap();
                }
            } else {
                fs::remove_file(&spool).unwrap();
            }
            let range = UncommittedRange {
                campaign_id: *ALLOCATION_CAMPAIGN,
                branch: 9,
                load_signal: Some(19),
                first_hour: 2,
                last_hour: 2,
            };
            let witness = TerminalWitness {
                failed_hour: Some(if shape == 2 { 2 } else { 3 }),
                uncommitted: (shape != 2).then_some(range),
                committed_segment: Some(if shape == 2 { 1 } else { 0 }),
                ..allocation_witness(committed_bytes)
            };
            assert_eq!(witness.process_start_time, session.process_start_time);
            fs::write(session_dir.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
            let finalized = finalize_session_outcome(&session_dir, &target).unwrap();
            assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
            assert!(finalized.omitted.is_empty(), "{:?}", finalized.omitted);
            let replay = Replay::open_path(&finalized.replays[0]).unwrap();
            assert_eq!(
                replay
                    .gaps
                    .iter()
                    .map(|gap| (gap.branch, gap.first_hour, gap.last_hour))
                    .collect::<Vec<_>>(),
                [(9, 2, if shape == 2 { 2 } else { 3 })]
            );
            assert!(
                replay
                    .controls
                    .iter()
                    .any(|control| matches!(control, ControlRecord::Partial { .. }))
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn checkpoint_trim_keeps_admitted_columnar_commands_from_phase_or_saved_evidence() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_frame, allocation_records, write_campaign_spool,
        };
        use replay::commands::{Batch, Category, Event, Kind, Origin, Packet, Phase, Span};
        use replay_frame::branches::BranchReason;
        for (own_phase, bootstrap_load) in [(false, 0), (true, 0), (false, 19), (true, 19)] {
            let root = std::env::temp_dir()
                .join(format!(
                    "hoi4-command-checkpoint-evidence-{}-{own_phase}-{bootstrap_load}",
                    std::process::id()
                ));
            let session_dir = root.join("session");
            let target = root.join("output");
            let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
            let mut original = allocation_records();
            let session = original[0].clone();
            let Record::BranchStart(first) = original[1].clone() else {
                unreachable!()
            };
            let command = |branch, world, ordinal, time, hour, load, start: Option<BranchStart>| {
                let batch = Batch {
                    kinds: if ordinal == 0 {
                        vec![Kind {
                            id: 0,
                            token: 0,
                            name: "synthetic command".into(),
                            category: Category::Other,
                        }]
                    } else {
                        Vec::new()
                    },
                    events: vec![Event {
                        paused: false,
                        qpc: time + 1,
                        hour,
                        actor: 0,
                        player: 7,
                        kind: 0,
                        command_country: None,
                        origin: Origin::Unresolved,
                        sequence: ordinal as u32 + 1,
                    }],
                    spans: vec![Span {
                        paused: false,
                        hour,
                        from_qpc: time,
                        to_qpc: time + 50,
                        complete: true,
                    }],
                    ..Default::default()
                };
                let mut packet =
                    Packet::pack(branch, 1, world, ordinal, time, time + 50, 0, &batch).unwrap();
                packet.phase = start.map(|start| Phase {
                    start,
                    campaign: *ALLOCATION_CAMPAIGN,
                    qpc: time,
                    hour,
                    load_signal: load,
                });
                Record::Control(ControlRecord::PlayerCommands { session: 7, packet })
            };
            let first_packet = command(9, 1, 0, 100, 2, bootstrap_load, Some(first.clone()));
            original.pop();
            original.insert(2, first_packet.clone());
            let campaign = write_campaign_spool(&session_dir, &original);
            let second = BranchStart {
                id: 10,
                session: 7,
                loaded_hour: Some(4),
                first_hour: 6,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            };
            let later = vec![
                session,
                Record::BranchStart(first.clone()),
                Record::Frame(allocation_frame(9, 5, 19)),
                Record::Control(ControlRecord::ClockWitness {
                    session: 7,
                    qpc: 500,
                    hour: 5,
                    load_signal: Some(19),
                }),
                Record::BranchStart(second.clone()),
                Record::Frame(allocation_frame(10, 6, 23)),
                Record::Control(ControlRecord::ClockWitness {
                    session: 7,
                    qpc: 600,
                    hour: 6,
                    load_signal: Some(23),
                }),
            ];
            let bytes: Vec<_> = later
                .iter()
                .flat_map(|record| replay::spool::encode_record(record).unwrap())
                .collect();
            let suffix = campaign.join("spool.0001.h4s");
            fs::write(&suffix, bytes).unwrap();
            begin_admissions(&session_dir, &target).unwrap();
            let mut admissions = Admissions::default();
            admissions
                .admit(&session_dir, &target, name, bootstrap_load)
                .unwrap();
            let checkpoint = checkpoint_campaign(
                &session_dir,
                &target,
                name,
                0,
                false,
                &mut Ahead::default(),
                unnamed(),
                &|| Ok(()),
            )
            .unwrap()
            .unwrap();
            assert_eq!(checkpoint.deleted, 1);
            assert!(!campaign.join("spool.0000.h4s").exists());
            let late = command(9, 1, 1, 400, 4, 19, None);
            let refused = command(10, 2, 0, 600, 6, 23, Some(second.clone()));
            let mut file = fs::OpenOptions::new().append(true).open(&suffix).unwrap();
            use std::io::Write;
            for record in [&refused, &late] {
                file.write_all(&replay::spool::encode_record(record).unwrap())
                    .unwrap();
            }
            drop(file);
            assert_eq!(
                read_branch_loads(&campaign).unwrap().loads.get(&9),
                Some(&bootstrap_load)
            );
            let mut resumed = Ahead::default();
            resumed.read(&session_dir, &target).unwrap();
            assert_eq!(
                resumed
                    .backlog(&session_dir)
                    .unwrap()
                    .runnable
                    .unwritten_hours,
                1
            );
            if bootstrap_load == 0 {
                assert!(
                    read_saved(&campaign)
                        .unwrap()
                        .command_bootstrap
                        .contains(&9)
                );
            }
            let mut captured_first = first_packet;
            if !own_phase {
                let Record::Control(ControlRecord::PlayerCommands { packet, .. }) =
                    &mut captured_first
                else {
                    unreachable!()
                };
                packet.phase = None;
            }
            let unknown = BranchStart {
                id: 11,
                session: 7,
                loaded_hour: None,
                first_hour: 7,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            };
            let controls = vec![
                Record::BranchStart(first),
                captured_first,
                Record::BranchStart(second),
                refused,
                late,
                Record::BranchStart(unknown),
            ];
            fs::write(
                campaign.join("columnar.0.h4col"),
                native_recording_with_controls(&[(9, 2, 4), (10, 6, 1), (11, 7, 1)], &controls),
            )
            .unwrap();
            let finalized = finalize_session_outcome(&session_dir, &target).unwrap();
            assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
            assert_eq!(finalized.replays.len(), 1);
            let rows = Replay::open_path(&finalized.replays[0]).unwrap();
            let expected = replay::commands::analyze(&rows.sessions, &rows.controls, 9).unwrap();
            let paths: Vec<_> = fs::read_dir(replay::columnar_file::campaign_folder(
                &target,
                ALLOCATION_CAMPAIGN,
            ))
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "h4col")
            })
            .collect();
            assert_eq!(paths.len(), 1);
            let columns =
                replay::columnar_frames::Recording::open_path(&paths[0].path(), Default::default())
                    .unwrap();
            assert_eq!(columns.hours(), 4);
            assert_eq!(columns.run_count(), 1);
            assert_eq!(columns.run(0).unwrap().branch, 9);
            drop(columns);
            let columns = replay::commands::Recording::open_path(&paths[0].path()).unwrap();
            assert_eq!(
                columns
                    .branches
                    .iter()
                    .map(|branch| branch.id)
                    .collect::<Vec<_>>(),
                [9]
            );
            let actual =
                replay::commands::analyze(&columns.sessions, &columns.controls, 9).unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
            drop((rows, columns));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_crash_between_admission_and_save_publishes_exactly_the_admitted_loads() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, THIRD_LOAD, allocation_test_directory,
            three_load_records, write_campaign_spool,
        };
        let directory = allocation_test_directory("admitted-after-crash");
        let session = directory.join("session");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = write_campaign_spool(&session, &three_load_records());
        let spool = fs::read(campaign.join("spool.0000.h4s")).unwrap();
        assert_eq!(recover_prefix(&spool).valid_bytes, spool.len());

        // A command line session publishes every load, as before.
        let everything = directory.join("everything");
        let paths = finalize_session(&session, &everything).unwrap();
        assert_eq!(
            published_frames(&paths[0]),
            [(9, 2), (9, 3), (9, 4), (10, 3), (10, 4), (11, 5), (11, 6)]
        );

        // A Battle Plan Recorder session admitted its first and third loads, each on
        // disk as it was admitted; the second was never probed. The recorder
        // then stopped before saving, so no completion record exists.
        let target = directory.join("target");
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        for load in [FIRST_LOAD, THIRD_LOAD] {
            admissions.admit(&session, &target, name, load).unwrap();
        }
        assert_eq!(session_admissions(&session).unwrap(), Some(admissions));
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(session.join(ADMISSIONS_FILE)).unwrap()).unwrap();
        assert_eq!(record["loads"][1]["target"], target.to_str().unwrap());
        let output = directory.join("output");
        let paths = finalize_session(&session, &output).unwrap();
        assert_eq!(
            published_frames(&paths[0]),
            [(9, 2), (9, 3), (9, 4), (11, 5), (11, 6)]
        );
        assert_eq!(
            fs::read(campaign.join("spool.0000.h4s")).unwrap(),
            spool,
            "the evidence stays"
        );
        // Repeated, it publishes exactly the same, and reports the load left out.
        let published = fs::read(&paths[0]).unwrap();
        let again = finalize_session_outcome(&session, &output).unwrap();
        assert_eq!(again.replays, paths);
        assert_eq!(
            again.omitted,
            [Omission {
                campaign: name.to_owned(),
                load: Some(crate::test_support::RELOAD),
                hours: 2,
                refusal: None,
            }]
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), published);
        fs::remove_dir_all(directory).unwrap();
    }

    use crate::test_support::write_load_segment as set_aside_segment;

    #[test]
    fn a_campaign_saved_while_the_session_records_on_appends_exactly_what_follows() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory,
        };
        let directory = allocation_test_directory("set-aside-save");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        admissions
            .admit(&session, &target, name, FIRST_LOAD)
            .unwrap();
        // The game left the campaign: its segment is closed and saved at once.
        set_aside_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        let mut ahead = Ahead::default();
        ahead.read(&session, &target).unwrap();
        let (saved, omitted) = save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut ahead,
            unnamed(),
        )
        .unwrap();
        let saved = saved.unwrap();
        assert!(omitted.is_empty());
        assert_eq!(published_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
        assert!(unsaved_segments(&campaign).unwrap().is_empty());
        assert_eq!(unsaved(&session, name).unwrap(), None);
        // Loaded again in the same session: a new load in a new segment.
        admissions.admit(&session, &target, name, RELOAD).unwrap();
        let second = set_aside_segment(&campaign, 1, 10, 3..=5, RELOAD);
        assert_eq!(
            unsaved(&session, name).unwrap(),
            Some(Unsaved {
                hours: 3,
                first_hour: Some(3),
                last_hour: Some(5),
                bytes: fs::metadata(&second).unwrap().len(),
                undecided: false,
            })
        );
        ahead.read(&session, &target).unwrap();
        save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut ahead,
            unnamed(),
        )
        .unwrap();
        let both = [(9, 2), (9, 3), (9, 4), (10, 3), (10, 4), (10, 5)];
        assert_eq!(published_frames(&saved), both);
        let replay = open(&saved).unwrap();
        assert_eq!(
            replay
                .branches
                .iter()
                .map(|branch| branch.id)
                .collect::<Vec<_>>(),
            [9, 10]
        );
        assert_eq!(replay.sessions.len(), 1);
        // The same as saving the whole spool at once.
        let whole = finalize_session(&session, &directory.join("whole")).unwrap();
        assert_eq!(published_frames(&whole[0]), both);
        assert_eq!(open(&whole[0]).unwrap().branches, replay.branches);
        // Saving again, even without its saved record, changes nothing.
        let bytes = fs::read(&saved).unwrap();
        save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut Ahead::default(),
            unnamed(),
        )
        .unwrap();
        assert_eq!(
            finalize_session(&session, &target).unwrap(),
            [saved.clone()]
        );
        fs::remove_file(campaign.join(SAVED_FILE)).unwrap();
        save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut Ahead::default(),
            unnamed(),
        )
        .unwrap();
        assert_eq!(fs::read(&saved).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_save_continues_the_unsaved_recording_and_takes_its_place() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory,
        };
        let directory = allocation_test_directory("unsaved-base");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        // Its hours name no country or date.
        let recording = target.join("Campaign.battleplan");
        let unsaved = replay::publication::unsaved_path(&target, name);
        let campaign = session.join(name);
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        for load in [FIRST_LOAD, RELOAD] {
            admissions.admit(&session, &target, name, load).unwrap();
        }
        let save = |stage| {
            save_campaign(
                &session,
                &target,
                name,
                stage,
                &|| Ok(()),
                &mut Ahead::default(),
                unnamed(),
            )
            .unwrap()
            .0
        };
        set_aside_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        assert_eq!(save(true), Some(unsaved.clone()));
        // Staged hours still wait, for showing and discarding.
        assert_eq!(unsaved_segments(&campaign).unwrap().len(), 1);
        set_aside_segment(&campaign, 1, 10, 5..=6, RELOAD);
        assert_eq!(save(false), Some(recording.clone()));
        assert!(!unsaved.exists());
        assert_eq!(
            published_frames(&recording),
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
        );
        assert!(unsaved_segments(&campaign).unwrap().is_empty());
        // Nothing more: staging finds nothing waiting.
        assert_eq!(save(true), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_save_never_replaces_a_recording_its_unsaved_recording_does_not_hold() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
        };
        let directory = allocation_test_directory("unsaved-not-held");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let begin = |session: &Path, target: &Path, load: i64| {
            begin_admissions(session, target).unwrap();
            select_recordings(session, target).unwrap();
            Admissions::default()
                .admit(session, target, name, load)
                .unwrap();
        };
        let save = |session: &Path, target: &Path, stage: bool| {
            save_campaign(
                session,
                target,
                name,
                stage,
                &|| Ok(()),
                &mut Ahead::default(),
                unnamed(),
            )
        };
        // Hours 2 to 4, saved.
        let first = directory.join("first");
        let first_session = directory.join("first-session");
        begin(&first_session, &first, FIRST_LOAD);
        set_aside_segment(&first_session.join(name), 0, 9, 2..=4, FIRST_LOAD);
        let saved = save(&first_session, &first, false).unwrap().0.unwrap();
        let published = fs::read(&saved).unwrap();
        // New file: the same game process captures hours 5 and 6 into another folder; they wait.
        let second = directory.join("second");
        let second_session = directory.join("second-session");
        begin(&second_session, &second, RELOAD);
        set_aside_segment(&second_session.join(name), 0, 10, 5..=6, RELOAD);
        let unsaved = save(&second_session, &second, true).unwrap().0.unwrap();
        assert_eq!(unsaved, replay::publication::unsaved_path(&second, name));
        // The first recording is moved in before Save; its session is the waiting hours' own.
        let moved = second.join(saved.file_name().unwrap());
        fs::rename(&saved, &moved).unwrap();
        assert_eq!(
            Replay::read_session_ids(&moved).unwrap(),
            Replay::read_session_ids(&unsaved).unwrap()
        );
        // The unsaved recording holds hours 5 and 6 alone: no save replaces the first with it.
        let error = save(&second_session, &second, false).unwrap_err();
        assert!(error.contains("nothing saved; spool retained"), "{error}");
        let error = publish_unsaved(&second, name, unnamed()).unwrap_err();
        assert!(error.contains("nothing saved; spool retained"), "{error}");
        assert_eq!(fs::read(&moved).unwrap(), published);
        assert_eq!(published_frames(&unsaved), [(10, 5), (10, 6)]);
        // Moved back out, the waiting hours save as a recording of their own.
        fs::rename(&moved, &saved).unwrap();
        let placed = save(&second_session, &second, false).unwrap().0.unwrap();
        assert_eq!(published_frames(&placed), [(10, 5), (10, 6)]);
        assert_eq!(fs::read(&saved).unwrap(), published);
        // Hours staged onto the first recording hold it whole, so they take its place.
        let third_session = directory.join("third-session");
        begin(&third_session, &first, THIRD_LOAD);
        set_aside_segment(&third_session.join(name), 0, 11, 7..=8, THIRD_LOAD);
        save(&third_session, &first, true).unwrap();
        let replaced = save(&third_session, &first, false).unwrap().0.unwrap();
        assert_eq!(
            published_frames(&replaced),
            [(9, 2), (9, 3), (9, 4), (11, 7), (11, 8)]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_session_continues_the_one_recording_kept_of_several_it_started_with() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
        };
        let directory = allocation_test_directory("several-at-start");
        let (session, target) = (directory.join("session"), directory.join("target"));
        fs::create_dir_all(&session).unwrap();
        fs::create_dir_all(&target).unwrap();
        let replay = Replay::from_records(
            ALLOCATION_CAMPAIGN.to_vec(),
            &[vec![allocation_records().remove(0)]],
        )
        .unwrap();
        let kept = target.join("Kept.battleplan");
        let moved = target.join("Moved.battleplan");
        fs::write(&kept, replay.bytes()).unwrap();
        fs::write(&moved, replay.bytes()).unwrap();
        select_recordings(&session, &target).unwrap();
        let continued = || continued_recording(&session, &target, ALLOCATION_CAMPAIGN, &[]);
        assert!(continued().is_err());
        // The others moved out since, it continues the one kept.
        fs::rename(&moved, directory.join("Moved.battleplan")).unwrap();
        assert_eq!(continued().unwrap(), Some(kept.clone()));
        // Renamed, it can no longer be told from one that appeared since.
        fs::rename(&kept, target.join("Renamed.battleplan")).unwrap();
        assert!(continued().is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discard_keeps_an_unsaved_recording_named_for_another_campaign() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_test_directory};
        const OTHER: &str = "00000000-0000-4000-8000-000000000001";
        let directory = allocation_test_directory("discard-ambiguous");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let (session, target) = (directory.join("session"), directory.join("target"));
        begin_admissions(&session, &target).unwrap();
        select_recordings(&session, &target).unwrap();
        Admissions::default()
            .admit(&session, &target, name, FIRST_LOAD)
            .unwrap();
        set_aside_segment(&session.join(name), 0, 9, 2..=4, FIRST_LOAD);
        let staged = save_campaign(
            &session,
            &target,
            name,
            true,
            &|| Ok(()),
            &mut Ahead::default(),
            unnamed(),
        )
        .unwrap()
        .0
        .unwrap();
        // Its hours under another campaign's name: neither campaign's Discard may delete them.
        let misnamed = replay::publication::unsaved_path(&target, OTHER);
        fs::rename(&staged, &misnamed).unwrap();
        let bytes = fs::read(&misnamed).unwrap();
        for campaign in [OTHER, name] {
            let error = discard_unsaved_recording(&target, campaign).unwrap_err();
            assert!(error.contains("nothing discarded"), "{error}");
        }
        assert_eq!(fs::read(&misnamed).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_saved_segment_that_differs_from_the_recording_is_refused() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_test_directory};
        let directory = allocation_test_directory("set-aside-conflict");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        begin_admissions(&session, &target).unwrap();
        Admissions::default()
            .admit(&session, &target, name, FIRST_LOAD)
            .unwrap();
        set_aside_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        let saved = save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut Ahead::default(),
            unnamed(),
        )
        .unwrap()
        .0
        .unwrap();
        let bytes = fs::read(&saved).unwrap();
        // A segment holding saved hours and a new one, and nothing saying what was saved.
        fs::remove_file(campaign.join(SAVED_FILE)).unwrap();
        set_aside_segment(&campaign, 0, 9, 2..=5, FIRST_LOAD);
        let error = save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut Ahead::default(),
            unnamed(),
        )
        .unwrap_err();
        assert!(
            error.contains("refusing to append duplicate or conflicting hours"),
            "{error}"
        );
        assert_eq!(fs::read(&saved).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discarding_waiting_hours_keeps_what_was_saved_and_undecided_loads_are_told() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
        };
        let directory = allocation_test_directory("set-aside-discard");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        admissions
            .admit(&session, &target, name, FIRST_LOAD)
            .unwrap();
        set_aside_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        let saved = save_campaign(
            &session,
            &target,
            name,
            false,
            &|| Ok(()),
            &mut Ahead::default(),
            unnamed(),
        )
        .unwrap()
        .0
        .unwrap();
        let bytes = fs::read(&saved).unwrap();
        // A refused load and one never asked about wait after it.
        admissions
            .refuse(&session, &target, name, RELOAD, Refusal::Off)
            .unwrap();
        set_aside_segment(&campaign, 1, 10, 5..=6, RELOAD);
        let refused = unsaved(&session, name).unwrap();
        assert_eq!(
            refused, None,
            "a refused load is never saved, so nothing waits"
        );
        set_aside_segment(&campaign, 2, 11, 7..=8, THIRD_LOAD);
        let waiting = unsaved(&session, name).unwrap().unwrap();
        assert!(waiting.undecided);
        assert_eq!((waiting.hours, waiting.first_hour), (0, None));
        discard_unsaved(&session, name).unwrap();
        assert!(segment_paths(&campaign).unwrap().is_empty());
        assert_eq!(fs::read(&saved).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_full_spool_with_nothing_admitted_reports_every_load_left_out() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, two_load_records,
            write_campaign_spool,
        };
        let directory = allocation_test_directory("nothing-admitted");
        let session = directory.join("session");
        let output = directory.join("output");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        write_campaign_spool(&session, &two_load_records());
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        admissions
            .refuse(&session, &target, name, FIRST_LOAD, Refusal::Off)
            .unwrap();
        assert_eq!(
            session_admissions(&session).unwrap().unwrap().refused,
            BTreeMap::from([((name.to_owned(), FIRST_LOAD), Refusal::Off)])
        );
        // The session recorded hours, so it is not "nothing recorded".
        assert!(has_campaign_spools(&session).unwrap());
        let finalized = finalize_session_outcome(&session, &output).unwrap();
        assert!(finalized.replays.is_empty() && finalized.errors.is_empty());
        assert_eq!(
            finalized.omitted,
            [
                Omission {
                    campaign: name.to_owned(),
                    load: Some(FIRST_LOAD),
                    hours: 3,
                    refusal: Some(Refusal::Off),
                },
                Omission {
                    campaign: name.to_owned(),
                    load: Some(RELOAD),
                    hours: 2,
                    refusal: None,
                },
            ]
        );
        assert_only_naming_lock(&output);
        fs::remove_dir_all(directory).unwrap();
    }

    /// A Battle Plan Recorder session's load that failed after committing its branch
    /// start and before its first clock witness keeps its gap and partial
    /// outcome, exactly as an unmarked session does.
    #[test]
    fn a_marked_session_keeps_the_failure_of_an_admitted_load_before_its_first_clock() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("marked-declared-no-clock");
        let campaign = std::str::from_utf8(&witness.campaign_id.unwrap())
            .unwrap()
            .to_owned();
        let mut records = allocation_records();
        records.pop();
        records.push(Record::BranchStart(BranchStart {
            id: 10,
            session: witness.session,
            loaded_hour: Some(20),
            first_hour: 21,
            parent: Parentage::Unknown,
            reason: BranchReason::LoadSignal,
        }));
        let mut spool = Vec::new();
        for record in &records {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.committed_bytes = spool.len() as u64;
        witness.branch = Some(10);
        witness.failed_hour = Some(21);
        witness.load_signal = Some(21);
        spool.extend_from_slice(b"torn bytes after declared branch");
        fs::write(session.join(&campaign).join("spool.0000.h4s"), &spool).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        // The command line's recording of this spool.
        let unmarked = finalize_session(&session, &output.join("unmarked")).unwrap();
        let expected = fs::read(&unmarked[0]).unwrap();
        let replay = Replay::open(expected.clone()).unwrap();
        assert_eq!(replay.branches.len(), 2);
        assert_eq!(replay.gaps.len(), 1);
        // Both loads admitted: the same recording, gap and partial outcome included.
        let target = output.join("marked");
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        for load in [19, 21] {
            admissions
                .admit(&session, &target, &campaign, load)
                .unwrap();
        }
        let marked = finalize_session(&session, &target).unwrap();
        assert_eq!(fs::read(&marked[0]).unwrap(), expected);
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: Some(21),
                ..
            }
        )));
        // The failing load not admitted: its failure is not this recording's.
        let clean = output.join("clean");
        begin_admissions(&session, &clean).unwrap();
        let mut admissions = Admissions::default();
        admissions.admit(&session, &clean, &campaign, 19).unwrap();
        let finalized = finalize_session_outcome(&session, &clean).unwrap();
        let replay = open(&finalized.replays[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert!(replay.gaps.is_empty());
        assert_eq!(
            finalized.omitted,
            [Omission {
                campaign,
                load: Some(21),
                hours: 0,
                refusal: None,
            }]
        );
    }

    #[test]
    fn a_tray_session_without_its_admission_record_publishes_nothing() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let directory = allocation_test_directory("admission-record-missing");
        let session = directory.join("session");
        let output = directory.join("output");
        write_campaign_spool(&session, &allocation_records());
        begin_admissions(&session, &directory.join("target")).unwrap();
        // Nothing admitted: nothing published, no error, and the load reported left out.
        let finalized = finalize_session_outcome(&session, &output).unwrap();
        assert!(finalized.replays.is_empty() && finalized.errors.is_empty());
        assert_eq!(finalized.omitted.len(), 1);
        assert_only_naming_lock(&output);
        // The record missing or unreadable: finalization refuses and publishes nothing.
        fs::remove_file(session.join(ADMISSIONS_FILE)).unwrap();
        assert!(session_admissions(&session).is_err());
        assert!(finalize_session(&session, &output).is_err());
        assert_only_naming_lock(&output);
        fs::write(session.join(ADMISSIONS_FILE), b"{\"loads\":").unwrap();
        assert!(finalize_session(&session, &output).is_err());
        assert_only_naming_lock(&output);
        // Without the marker it is a command line session, which publishes every load.
        fs::remove_file(session.join("bin").join(ADMISSION_MARKER)).unwrap();
        assert_eq!(finalize_session(&session, &output).unwrap().len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_campaigns_earlier_hours_in_the_spool_count_as_recorded_before_a_reload() {
        use crate::test_support::{
            FIRST_LOAD, RELOAD, allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let directory = allocation_test_directory("recorded-before-load");
        let campaign = directory.join("session").join("missing");
        // No spool yet: nothing recorded.
        assert!(!recorded_before_load(&campaign, RELOAD));
        let campaign = write_campaign_spool(&directory.join("session"), &allocation_records());
        // Hours of the first load are in the spool: a reload has earlier hours,
        // the first load itself does not.
        assert!(recorded_before_load(&campaign, RELOAD));
        assert!(!recorded_before_load(&campaign, FIRST_LOAD));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn finalization_refuses_recovered_destination_and_preserves_spool_and_file() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
        };
        let directory = allocation_test_directory("damaged-destination");
        let session = directory.join("session");
        let output = directory.join("output");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        fs::create_dir_all(&campaign).unwrap();
        let spool: Vec<_> = allocation_records()
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, &spool).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let mut damaged = fs::read(&paths[0]).unwrap();
        *damaged.last_mut().unwrap() ^= 1;
        fs::write(&paths[0], &damaged).unwrap();
        assert!(open(&paths[0]).unwrap().needed_recovery());
        let error = finalize_session(&session, &output).unwrap_err();
        assert!(
            error.contains("This recording is damaged, so it can't be added to; start a new file.")
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), damaged);
        assert_eq!(fs::read(path).unwrap(), spool);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_save_that_cannot_add_to_its_recording_leaves_it_and_the_next_save_adds_to_it() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
        };
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

        let directory = allocation_test_directory("in-place-refused");
        let session = directory.join("session");
        let output = directory.join("output");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        fs::create_dir_all(&campaign).unwrap();
        fs::create_dir(&output).unwrap();
        let records = allocation_records();
        let spool: Vec<_> = records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        let spool_path = campaign.join("spool.0000.h4s");
        fs::write(&spool_path, &spool).unwrap();
        let destination = output.join(format!("{name}.battleplan"));
        let mut old_records = records[..4].to_vec();
        for record in &mut old_records {
            match record {
                Record::SessionStart(session) => session.id = 6,
                Record::BranchStart(branch) => {
                    branch.id = 8;
                    branch.session = 6;
                }
                Record::Frame(frame) => frame.branch = 8,
                Record::Control(ControlRecord::ClockWitness { session, .. }) => *session = 6,
                _ => unreachable!(),
            }
        }
        let old = Replay::from_records(ALLOCATION_CAMPAIGN.to_vec(), &[old_records]).unwrap();
        old.write_atomic(&destination).unwrap();

        // Another program reads it and lets no one write it: the save cannot add to it.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(&destination)
            .unwrap();
        assert!(finalize_session(&session, &output).is_err());
        assert_eq!(fs::read(&destination).unwrap(), old.bytes());
        assert_eq!(fs::read(&spool_path).unwrap(), spool);
        drop(held);

        // The next save adds the session to it in place, as an append in memory would write
        // it, and names it as its contents do.
        let named = output.join("Campaign.battleplan");
        assert_eq!(
            finalize_session(&session, &output).unwrap(),
            vec![named.clone()],
        );
        assert!(!destination.exists());
        let bytes = fs::read(&named).unwrap();
        assert_eq!(bytes, old.append(&[records]).unwrap().bytes());
        let modified = fs::metadata(&named).unwrap().modified().unwrap();
        // Saved again, it is unchanged.
        assert_eq!(
            finalize_session(&session, &output).unwrap(),
            vec![named.clone()],
        );
        assert_eq!(fs::read(&named).unwrap(), bytes);
        assert_eq!(fs::metadata(&named).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read(&spool_path).unwrap(), spool);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn streaming_finalization_keeps_rows_out_of_metadata_and_repeats_exactly() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_test_directory};
        let directory = allocation_test_directory("streaming-finalization");
        let session = directory.join("session");
        let output = directory.join("output");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        fs::create_dir_all(&campaign).unwrap();
        let mut records = long_finalization_records(None);
        let spool: Vec<_> = records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, &spool).unwrap();
        let metadata = recover_reader(
            fs::File::open(&path).unwrap(),
            spool.len(),
            Recovered::keep,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(metadata.valid_bytes, spool.len());
        assert!(metadata.damage.is_none());
        assert_eq!(
            metadata
                .records
                .iter()
                .filter(|record| matches!(record, Recovered::Frame(_)))
                .count(),
            20
        );
        for record in &metadata.records {
            if let Recovered::Frame(frame) = record {
                let frame = frame.frame().unwrap();
                assert!(
                    matches!(&frame.tables[TableId::LexicalTokens.index()], TableUpdate::Unavailable { reason } if reason == "finalization metadata only")
                );
            }
        }
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(
            replay
                .index
                .iter()
                .map(|block| replay.read_block(block).unwrap().len())
                .sum::<usize>(),
            20
        );
        let bytes = replay.bytes().to_vec();
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        assert_eq!(fs::read(&path).unwrap(), spool);

        let Record::Frame(changed) = &mut records[10] else {
            unreachable!()
        };
        changed.tables[TableId::LexicalTokens.index()] =
            TableUpdate::Snapshot(TableRows::LexicalTokens(vec![
                replay_data::rows::LexicalTokenRow {
                    token: 0,
                    literal: b"different captured value".to_vec(),
                },
            ]));
        let changed_spool: Vec<_> = records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        fs::write(&path, changed_spool).unwrap();
        assert!(
            finalize_session(&session, &output)
                .unwrap_err()
                .contains("differs from it")
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    /// Each frame kept as metadata with its position as offset.
    fn already_finalized(previous: &Replay, segments: &[(u32, Recovery)]) -> Result<bool> {
        let kept: Vec<_> = segments
            .iter()
            .map(|(number, recovery)| {
                (
                    *number,
                    Recovery {
                        format: recovery.format,
                        records: recovery
                            .records
                            .iter()
                            .enumerate()
                            .map(|(at, record)| {
                                Recovered::keep(
                                    record.clone(),
                                    replay::spool::Placement {
                                        offset: at,
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect(),
                        valid_bytes: recovery.valid_bytes,
                        damage: recovery.damage.clone(),
                    },
                )
            })
            .collect();
        already_finalized_with_reader(previous, &kept, |number, frame| {
            let (_, recovery) = segments
                .iter()
                .find(|(segment, _)| *segment == number)
                .unwrap();
            match &recovery.records[frame.placement.offset] {
                Record::Frame(frame) => Ok(frame.clone()),
                _ => unreachable!("a frame's position"),
            }
        })
    }

    #[test]
    fn runnable_backlog_stops_at_the_first_unadmitted_load_even_after_a_later_admission() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            three_load_records, write_campaign_spool,
        };
        let directory = allocation_test_directory("admitted-packing-backlog");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        write_campaign_spool(&session, &three_load_records());
        begin_admissions(&session, &target).unwrap();
        let mut admissions = Admissions::default();
        admissions.admit(&session, &target, name, FIRST_LOAD).unwrap();
        let mut ahead = Ahead::default();
        ahead.read(&session, &target).unwrap();
        let before = ahead.backlog(&session).unwrap();
        assert_eq!((before.runnable.unwritten_hours, before.blocked.unwritten_hours), (3, 4));
        assert_eq!((before.runnable.closed_blocks, before.blocked.closed_blocks), (2, 4));
        assert_eq!(before.runnable.work.rows + before.blocked.work.rows, ahead.outstanding().rows);
        ahead.write(&session, &target, &|| false).unwrap();
        let packed = ahead.backlog(&session).unwrap();
        assert_eq!(ahead.written(), 3);
        assert_eq!((packed.runnable.unwritten_hours, packed.runnable.closed_blocks), (0, 0));
        assert_eq!(packed.blocked.work.rows, before.blocked.work.rows);
        admissions.refuse(&session, &target, name, RELOAD, Refusal::Off).unwrap();
        admissions.admit(&session, &target, name, THIRD_LOAD).unwrap();
        ahead.write(&session, &target, &|| false).unwrap();
        let refused = ahead.backlog(&session).unwrap();
        assert_eq!(ahead.written(), 3);
        assert_eq!(refused.runnable.work.rows, 0);
        assert_eq!(refused.runnable.closed_blocks, 0);
        assert_eq!(refused.blocked.unwritten_hours, 4);
        assert_eq!(refused.blocked.closed_blocks, 4);
        admissions.admit(&session, &target, name, RELOAD).unwrap();
        let admitted = ahead.backlog(&session).unwrap();
        assert_eq!((admitted.runnable.unwritten_hours, admitted.blocked.unwritten_hours), (4, 0));
        assert_eq!(admitted.runnable.closed_blocks, 4);
        ahead.write(&session, &target, &|| false).unwrap();
        let drained = ahead.backlog(&session).unwrap();
        assert_eq!(ahead.written(), 7);
        assert_eq!(drained.runnable.work.rows, 0);
        assert_eq!(drained.runnable.closed_blocks, 0);
        assert_eq!(drained.blocked.work.rows, 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn blocked_refused_and_stopped_campaigns_do_not_add_to_a_healthy_campaigns_runnable_debt() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_records,
            allocation_test_directory, three_load_records, write_campaign_spool,
        };
        const HEALTHY: &str = "00000000-0000-4000-8000-000000000001";
        for state in ["unadmitted", "refused", "stopped"] {
            let directory = allocation_test_directory(&format!("blocked-packing-backlog-{state}"));
            let session = directory.join("session");
            let target = directory.join("target");
            let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
            let records = three_load_records();
            let campaign = write_campaign_spool(&session, &records);
            begin_admissions(&session, &target).unwrap();
            let mut admissions = Admissions::default();
            let mut ahead = Ahead::default();
            if state == "stopped" {
                admissions.admit(&session, &target, name, FIRST_LOAD).unwrap();
                ahead.read(&session, &target).unwrap();
                ahead.write(&session, &target, &|| false).unwrap();
                assert!(ahead.campaigns[&campaign].written.is_some());
                for load in [RELOAD, THIRD_LOAD] {
                    admissions.admit(&session, &target, name, load).unwrap();
                }
                ahead.campaigns.get_mut(&campaign).unwrap().stopped = true;
            } else if state == "refused" {
                admissions.refuse(&session, &target, name, FIRST_LOAD, Refusal::Elsewhere).unwrap();
            }
            let mut healthy = allocation_records();
            for record in &mut healthy {
                if let Record::Frame(frame) = record {
                    frame.tables[TableId::Campaign.index()] =
                        TableUpdate::Snapshot(TableRows::Campaign(vec![
                            replay_data::rows::CampaignRow {
                                id: HEALTHY.as_bytes().to_vec(),
                            },
                        ]));
                }
            }
            let healthy_dir = session.join(HEALTHY);
            fs::create_dir_all(&healthy_dir).unwrap();
            fs::write(
                healthy_dir.join("spool.0000.h4s"),
                healthy
                    .iter()
                    .flat_map(|record| replay::spool::encode_record(record).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            admissions
                .admit(&session, &target, HEALTHY, FIRST_LOAD)
                .unwrap();
            ahead.read(&session, &target).unwrap();
            // Inflate only blocked metadata, not files or allocations, past the row escalation threshold.
            let spool: Vec<_> = records.iter()
                .flat_map(|record| replay::spool::encode_record(record).unwrap()).collect();
            let mut tail = Tail::default();
            tail.grow(
                &mut std::io::Cursor::new(&spool),
                spool.len(),
                |record, placement| {
                    let mut record = Recovered::keep(record, placement);
                    if let Recovered::Frame(frame) = &mut record {
                        frame.placement.length = 100_000_000;
                    }
                    record
                },
            )
            .unwrap();
            ahead
                .campaigns
                .get_mut(&campaign)
                .unwrap()
                .tails
                .get_mut(&0)
                .unwrap()
                .1 = tail;
            let before = ahead.backlog(&session).unwrap();
            assert_eq!(before.runnable.unwritten_hours, 3, "{state}");
            assert_eq!(before.runnable.closed_blocks, 2, "{state}");
            assert!(before.blocked.work.rows >= 100_000_000, "{state}");
            assert!(before.blocked.closed_blocks >= 2, "{state}");
            if state == "stopped" {
                assert!(before.blocked.work.readback > 0);
                assert!(before.blocked.work.open > 0);
            }
            ahead.write(&session, &target, &|| false).unwrap();
            let drained = ahead.backlog(&session).unwrap();
            assert_eq!(drained.runnable.unwritten_hours, 0, "{state}");
            assert_eq!(drained.runnable.work.rows, 0, "{state}");
            assert_eq!(drained.runnable.closed_blocks, 0, "{state}");
            assert_eq!(drained.blocked.work.rows, before.blocked.work.rows, "{state}");
            assert_eq!(drained.blocked.closed_blocks, before.blocked.closed_blocks, "{state}");
            assert_eq!(drained.runnable.work.rows + drained.blocked.work.rows, ahead.outstanding().rows);
            assert_eq!(drained.runnable.closed_blocks + drained.blocked.closed_blocks, ahead.pending_blocks());
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn an_encoder_outside_the_current_admitted_prefix_is_blocked_not_runnable() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        let directory = allocation_test_directory("withdrawn-packing-prefix");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        write_campaign_spool(&session, &allocation_records());
        begin_admissions(&session, &target).unwrap();
        Admissions::default().admit(&session, &target, name, FIRST_LOAD).unwrap();
        let mut ahead = Ahead::default();
        ahead.read(&session, &target).unwrap();
        ahead.write(&session, &target, &|| false).unwrap();
        assert!(ahead.backlog(&session).unwrap().runnable.work.readback > 0);
        begin_admissions(&session, &target).unwrap();
        let backlog = ahead.backlog(&session).unwrap();
        assert_eq!(backlog.runnable.work.readback, 0);
        assert_eq!(backlog.runnable.work.open, 0);
        assert!(backlog.blocked.work.readback > 0);
        assert!(backlog.blocked.work.open > 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unreadable_admissions_leave_scheduling_debt_unavailable() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let directory = allocation_test_directory("unknown-packing-backlog");
        let session = directory.join("session");
        let target = directory.join("target");
        write_campaign_spool(&session, &allocation_records());
        begin_admissions(&session, &target).unwrap();
        let mut ahead = Ahead::default();
        ahead.read(&session, &target).unwrap();
        fs::write(session.join(ADMISSIONS_FILE), b"not json").unwrap();
        assert!(ahead.backlog(&session).is_err());
        assert!(ahead.outstanding().rows > 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn backlog_includes_newly_read_frames_before_the_encoder_accepts_them() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_test_directory};
        let directory = allocation_test_directory("fresh-packing-backlog");
        let session = directory.join("session");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        fs::create_dir_all(&campaign).unwrap();
        let path = campaign.join("spool.0000.h4s");
        let output = directory.join("output");
        let records = long_finalization_records(Some(12));
        let split = records
            .iter()
            .position(|record| matches!(record, Record::Frame(frame) if frame.hour == 12))
            .unwrap();
        let prefix: Vec<_> = records[..split]
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        fs::write(&path, prefix).unwrap();
        let mut ahead = Ahead::default();
        ahead.read(&session, &output).unwrap();
        assert_eq!(ahead.frames_read(), 10);
        assert_eq!(ahead.written(), 0);
        assert!(ahead.outstanding().rows > 0);
        assert_eq!(ahead.pending_blocks(), 0);
        ahead.write(&session, &output, &|| false).unwrap();
        assert_eq!(ahead.written(), 10);
        assert_eq!(ahead.outstanding().rows, 0);
        let bytes: Vec<_> = records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        fs::write(&path, &bytes).unwrap();
        ahead.read(&session, &output).unwrap();
        assert_eq!(
            (ahead.frames_read(), ahead.written(), ahead.hours()),
            (20, 10, 0)
        );
        assert_eq!(ahead.pending_blocks(), 1);
        let expected: u64 = ahead
            .campaigns
            .values()
            .flat_map(|campaign| campaign.tails.values())
            .flat_map(|(_, tail)| tail.records())
            .filter_map(|record| match record {
                Recovered::Frame(frame) if frame.hour >= 12 => Some(frame.placement.length as u64),
                _ => None,
            })
            .sum();
        assert_eq!(ahead.outstanding().rows, expected);
        assert!(expected > 0);
        ahead.write(&session, &output, &|| false).unwrap();
        assert_eq!(
            (ahead.frames_read(), ahead.written(), ahead.hours()),
            (20, 20, 10)
        );
        assert_eq!(ahead.pending_blocks(), 0);
        assert_eq!(ahead.outstanding().rows, 0);
        ahead.forget(&campaign);
        assert_eq!(
            (ahead.frames_read(), ahead.written(), ahead.hours()),
            (0, 0, 0)
        );
        assert_eq!(ahead.pending_blocks(), 0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_spool_finalized_ahead_while_it_grows_publishes_the_same_recording() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_test_directory};
        let directory = allocation_test_directory("finalized-ahead");
        let session = directory.join("session");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        fs::create_dir_all(&campaign).unwrap();
        let path = campaign.join("spool.0000.h4s");
        let ahead_dir = directory.join("ahead");
        let mut ahead = Ahead::default();
        let mut written = Vec::new();
        for (at, record) in long_finalization_records(Some(12)).iter().enumerate() {
            let record = replay::spool::encode_record(record).unwrap();
            written.extend_from_slice(&record);
            // The collector's write of a record may not be whole yet.
            fs::write(&path, &written[..written.len() - record.len() / 2]).unwrap();
            ahead.read(&session, &ahead_dir).unwrap();
            ahead.write(&session, &ahead_dir, &|| false).unwrap();
            fs::write(&path, &written).unwrap();
            if at % 3 == 0 {
                ahead.read(&session, &ahead_dir).unwrap();
                ahead.write(&session, &ahead_dir, &|| false).unwrap();
            }
        }
        // The first block, hours 2 to 11, is packed; the second is still open.
        assert_eq!(ahead.hours(), 10);
        assert_eq!(ahead.frames_read(), 20);
        assert_eq!(ahead.speed(), Some(0));
        let with =
            finalize_session_ahead(&session, &ahead_dir, &|| Ok(()), ahead, unnamed(), &|_| {
                Destination::Recording
            })
            .unwrap();
        let without = finalize_session(&session, &directory.join("plain")).unwrap();
        assert_eq!(
            fs::read(&with.replays[0]).unwrap(),
            fs::read(&without[0]).unwrap()
        );
        // Records that differ from those written ahead are finalized from the spool.
        let mut ahead = Ahead::default();
        ahead.read(&session, &directory.join("changed")).unwrap();
        ahead
            .write(&session, &directory.join("changed"), &|| false)
            .unwrap();
        let mut records = long_finalization_records(Some(12));
        records.remove(records.len() - 2);
        let spool: Vec<_> = records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        fs::write(&path, spool).unwrap();
        let changed = finalize_session_ahead(
            &session,
            &directory.join("changed"),
            &|| Ok(()),
            ahead,
            unnamed(),
            &|_| Destination::Recording,
        )
        .unwrap();
        let plain = finalize_session(&session, &directory.join("changed-plain")).unwrap();
        assert_eq!(
            fs::read(&changed.replays[0]).unwrap(),
            fs::read(&plain[0]).unwrap()
        );
        // A spool cut short since it was read ahead is finalized from what it still holds.
        let mut ahead = Ahead::default();
        ahead.read(&session, &directory.join("truncated")).unwrap();
        ahead
            .write(&session, &directory.join("truncated"), &|| false)
            .unwrap();
        assert!(ahead.written() > 0);
        let spool = fs::read(&path).unwrap();
        fs::write(&path, &spool[..spool.len() * 2 / 3]).unwrap();
        let truncated = finalize_session_ahead(
            &session,
            &directory.join("truncated"),
            &|| Ok(()),
            ahead,
            unnamed(),
            &|_| Destination::Recording,
        )
        .unwrap();
        let plain = finalize_session(&session, &directory.join("truncated-plain")).unwrap();
        assert_eq!(
            fs::read(&truncated.replays[0]).unwrap(),
            fs::read(&plain[0]).unwrap()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn blocks_are_written_ahead_only_up_to_a_load_not_yet_admitted() {
        let records = crate::test_support::two_load_records();
        let spool: Vec<_> = records
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        let mut tail = Tail::default();
        tail.grow(
            &mut std::io::Cursor::new(&spool),
            spool.len(),
            Recovered::keep,
        )
        .unwrap();
        let tails = BTreeMap::from([(0, (PathBuf::new(), tail))]);
        let count = |admitted: Option<&BTreeSet<i64>>| -> usize {
            admitted_prefix(&tails, admitted, &BTreeSet::new())
                .iter()
                .map(|(_, records)| records.len())
                .sum()
        };
        let second = records
            .iter()
            .position(|record| matches!(record, Record::BranchStart(branch) if branch.id == 10))
            .unwrap();
        let first_load = records
            .iter()
            .find_map(|record| match record {
                Record::Control(ControlRecord::ClockWitness { load_signal, .. }) => *load_signal,
                _ => None,
            })
            .unwrap();
        let second_load = records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Control(ControlRecord::ClockWitness { load_signal, .. }) => *load_signal,
                _ => None,
            })
            .unwrap();
        assert_eq!(count(None), records.len());
        assert_eq!(count(Some(&BTreeSet::from([first_load]))), second);
        assert_eq!(
            count(Some(&BTreeSet::from([first_load, second_load]))),
            records.len()
        );
        // Nothing past the session start before its first load is admitted.
        assert_eq!(count(Some(&BTreeSet::new())), 1);
    }

    #[test]
    fn the_summary_observes_the_gaps_a_full_scan_counts() {
        use replay_data::rows::{ClockRow, DivisionStatusRow, Recorded};
        use replay_frame::frame::FrameEncoder;
        let records = crate::test_support::allocation_records();
        let Record::Frame(first) = &records[2] else {
            unreachable!()
        };
        let template = FrameDecoder::new().apply(first).unwrap();
        let mut encoder = FrameEncoder::new();
        let (mut summary, mut decoder) = (Summary::default(), FrameDecoder::new());
        let mut unchanged = 0;
        // Changed, unchanged after a change and after the keyframe, and across a missing hour.
        for (hour, missing) in [(2, 2), (3, 2), (4, 3), (5, 3), (7, 3), (8, 0), (9, 0)] {
            let mut decoded = template.clone();
            decoded.hour = hour;
            *decoded.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    ..Default::default()
                }]));
            *decoded.tables[TableId::DivisionStatus.index()] =
                TableView::Complete(TableRows::DivisionStatus(
                    (0..4_u64)
                        .map(|id| DivisionStatusRow {
                            division: id << 32 | 1,
                            supply: if id < missing {
                                Recorded::NotRecorded
                            } else {
                                Default::default()
                            },
                            ..Default::default()
                        })
                        .collect(),
                ));
            let frame = encoder.encode(decoded, hour == 2).unwrap();
            unchanged += usize::from(matches!(
                frame.tables[TableId::DivisionStatus.index()],
                TableUpdate::Unchanged { .. }
            ));
            let scan = Observed::new(
                &frame,
                decoder.advance(&frame).unwrap(),
                [0; TABLE_COUNT],
                None,
            )
            .unwrap();
            summary.observe(&frame).unwrap();
            let observed = summary.previous.as_ref().unwrap();
            assert_eq!(observed.field_gaps, scan.field_gaps, "hour {hour}");
            assert_eq!(observed.field_gaps.is_empty(), missing == 0, "hour {hour}");
        }
        assert_eq!(unchanged, 4);
    }

    #[test]
    fn spool_segments_cannot_change_between_recovery_and_reread() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_test_directory};
        let directory = allocation_test_directory("held-segments");
        let session = directory.join("session");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        fs::create_dir_all(&campaign).unwrap();
        let encode = |records: &[Record]| -> Vec<u8> {
            records
                .iter()
                .flat_map(|record| replay::spool::encode_record(record).unwrap())
                .collect()
        };
        let records = long_finalization_records(Some(12));
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, encode(&records)).unwrap();
        let expected =
            fs::read(&finalize_session(&session, &directory.join("plain")).unwrap()[0]).unwrap();
        // The same records with one hour's rows changed, each with a valid checksum.
        let mut changed = records.clone();
        let Record::Frame(frame) = &mut changed[10] else {
            unreachable!()
        };
        frame.tables[TableId::LexicalTokens.index()] =
            TableUpdate::Snapshot(TableRows::LexicalTokens(vec![
                replay_data::rows::LexicalTokenRow {
                    token: 0,
                    literal: b"replaced".to_vec(),
                },
            ]));
        let other = directory.join("other.h4s");
        fs::write(&other, encode(&changed)).unwrap();
        let calls = std::cell::Cell::new(0);
        let attempts = std::cell::RefCell::new(Vec::new());
        let finalized =
            finalize_session_outcome_with_progress(&session, &directory.join("held"), &|| {
                calls.set(calls.get() + 1);
                // The first call after recovery read every record.
                if calls.get() == records.len() + 2 {
                    attempts.borrow_mut().extend([
                        fs::OpenOptions::new().write(true).open(&path).is_ok(),
                        fs::rename(&other, &path).is_ok(),
                        fs::remove_file(&path).is_ok(),
                    ]);
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(*attempts.borrow(), [false, false, false]);
        assert_eq!(fs::read(&finalized.replays[0]).unwrap(), expected);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn repeat_finalization_accepts_added_keyframes_but_checks_values_and_metrics() {
        let original = long_finalization_records(None);
        let replay = Replay::from_records(
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            &[long_finalization_records(Some(8))],
        )
        .unwrap();
        let mut segments = [(
            0,
            Recovery {
                format: Some(FormatVersion::CURRENT),
                records: original,
                valid_bytes: 0,
                damage: None,
            },
        )];
        assert!(already_finalized(&replay, &segments).unwrap());
        let Record::Frame(changed) = &mut segments[0].1.records[8] else {
            unreachable!()
        };
        changed.metrics.total_ns = 99;
        assert!(
            already_finalized(&replay, &segments)
                .unwrap_err()
                .contains("differs from it")
        );
    }

    #[test]
    fn inspect_reports_read_times_a_replay_does_not_record() {
        let mut records = crate::test_support::allocation_records();
        for record in &mut records {
            if let Record::Frame(frame) = record {
                frame.metrics.table_ns[TableId::Clock.index()] = 2_500_000;
            }
        }
        let replay = Replay::from_records(
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            std::slice::from_ref(&records),
        )
        .unwrap();
        let mut from_replay = Summary::default();
        for index in &replay.index {
            for frame in replay.read_block(index).unwrap() {
                assert_ne!(frame.metrics.table_ns[TableId::Clock.index()], 0);
                from_replay.observe(&frame).unwrap();
            }
        }
        assert_eq!(
            from_replay.read_times(TableId::Clock),
            "not_recorded not_recorded not_recorded"
        );
        // Finalization summarizes spool frames, which keep nanoseconds.
        let mut from_spool = Summary::default();
        for record in &records {
            if let Record::Frame(frame) = record {
                from_spool.observe(frame).unwrap();
            }
        }
        assert_eq!(from_spool.read_times(TableId::Clock), "2.500 2.500 2.500");
    }

    #[test]
    fn allocation_failure_json_configuration_requires_prepared_identity() {
        use crate::test_support::{allocation_records, allocation_test_directory};
        let directory = allocation_test_directory("prepared-identity");
        let Record::SessionStart(session) = &allocation_records()[0] else {
            unreachable!()
        };
        let mut session_start = Vec::new();
        session.encode(&mut session_start).unwrap();
        let config = serde_json::json!({"session_id":session.id,"session_start":session_start});
        fs::create_dir_all(directory.join("bin")).unwrap();
        fs::write(
            directory.join("bin/collector.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let path = directory.join(WITNESS_FILE_NAME);
        fs::write(&path, TerminalWitness::prepared(7, 13)).unwrap();
        assert!(read_failure_witness(&directory).unwrap().is_none());
        fs::write(&path, TerminalWitness::prepared(7, 14)).unwrap();
        assert!(read_failure_witness(&directory).is_err());
        fs::write(&path, [0; WITNESS_BYTES]).unwrap();
        assert!(read_failure_witness(&directory).is_err());
    }

    #[test]
    #[ignore = "launched by collector allocation-failure sweeps"]
    fn allocation_fixture_finalization_child() {
        let session = PathBuf::from(std::env::var_os("BATTLE_PLAN_ALLOCATION_FIXTURE").or_else(|| std::env::var_os("HOI4_ALLOCATION_FIXTURE")).unwrap());
        let witness = read_failure_witness(&session).unwrap().unwrap();
        let mut expected = BTreeMap::new();
        let mut sources = Vec::new();
        let mut refused = BTreeSet::new();
        for campaign in fs::read_dir(&session).unwrap() {
            let campaign = campaign.unwrap();
            if !campaign.file_type().unwrap().is_dir() {
                continue;
            }
            for (number, path) in segment_paths(&campaign.path()).unwrap() {
                let bytes = fs::read(&path).unwrap();
                let mut prefix = bytes.as_slice();
                let committed_campaign = witness.committed_campaign_id.is_some_and(|id| {
                    campaign.file_name().to_str().map(str::as_bytes) == Some(id.as_slice())
                });
                let pending_campaign = witness.pending_campaign_id.is_some_and(|id| {
                    campaign.file_name().to_str().map(str::as_bytes) == Some(id.as_slice())
                });
                if witness.committed_campaign_id.is_none()
                    || (pending_campaign && witness.pending_segment.is_none())
                {
                    refused.insert(campaign.file_name());
                    prefix = &[];
                } else {
                    let committed = committed_campaign
                        .then(|| (witness.committed_segment.unwrap(), witness.committed_bytes));
                    let pending = pending_campaign
                        .then(|| (witness.pending_segment.unwrap(), witness.pending_bytes));
                    if let Some((segment, committed_bytes)) =
                        [committed, pending].into_iter().flatten().min()
                    {
                        if number > segment {
                            prefix = &[];
                        } else if number == segment {
                            prefix = &bytes[..usize::try_from(committed_bytes).unwrap()];
                        }
                    }
                }
                let recovery = recover_prefix(prefix);
                for record in recovery.records {
                    if let Record::Frame(frame) = record {
                        assert!(
                            expected
                                .insert((campaign.file_name(), frame.branch, frame.hour), frame)
                                .is_none()
                        );
                    }
                }
                sources.push((path, bytes));
            }
        }
        let output = session.join("replays");
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert_eq!(issues.len(), refused.len(), "{issues:?}");
        assert!(
            issues
                .iter()
                .all(|issue| issue.contains("no committed spool watermark"))
        );
        assert_eq!(paths.is_empty(), expected.is_empty());
        let mut gap_found = false;
        let mut gap_expected = false;
        let mut allocation_gap_count = 0;
        let mut before = Vec::new();
        for path in &paths {
            let bytes = fs::read(path).unwrap();
            let replay = Replay::open(bytes.clone()).unwrap();
            let expected_failed_hour = if witness.campaign_id.as_ref().map(|id| id.as_slice())
                == Some(replay.campaign_id.as_slice())
            {
                witness.failed_hour
            } else {
                None
            };
            gap_expected |= expected_failed_hour.is_some();
            allocation_gap_count += replay
                .gaps
                .iter()
                .filter(|gap| gap.reason == ALLOCATION_GAP_REASON)
                .count();
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { session, reason, failed_hour }
                if *session == witness.session && reason == ALLOCATION_GAP_REASON
                    && *failed_hour == expected_failed_hour)));
            if let Some(detach) = replay
                .controls
                .iter()
                .position(|control| matches!(control, ControlRecord::Detach { .. }))
            {
                let partial = replay
                    .controls
                    .iter()
                    .position(|control| matches!(control, ControlRecord::Partial { .. }))
                    .unwrap();
                assert!(partial < detach);
            }
            // Finalization's SegmentState rejects a gap after Detach.
            for block in &replay.index {
                let frames = replay.read_block(block).unwrap();
                let mut decoder = FrameDecoder::new();
                for frame in frames {
                    let key = (
                        std::ffi::OsString::from(std::str::from_utf8(&replay.campaign_id).unwrap()),
                        frame.branch,
                        frame.hour,
                    );
                    let expected_frame = expected
                        .remove(&key)
                        .expect("replay contains an unrecorded frame");
                    assert_eq!(frame, expected_frame);
                    decoder.apply(&frame).unwrap();
                    assert!(matches!(
                        replay.seek(frame.branch, frame.hour).unwrap(),
                        SeekResult::Frame(_)
                    ));
                }
            }
            if let Some(hour) = expected_failed_hour {
                let gap = replay
                    .gaps
                    .iter()
                    .find(|gap| {
                        gap.first_hour <= hour
                            && gap.last_hour >= hour
                            && gap.reason == ALLOCATION_GAP_REASON
                    })
                    .expect("known attempted hour has no explicit allocation gap");
                assert!(
                    matches!(replay.seek(gap.branch, hour).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
                );
                gap_found = true;
            } else {
                assert!(
                    !replay
                        .gaps
                        .iter()
                        .any(|gap| gap.reason == ALLOCATION_GAP_REASON)
                );
            }
            before.push(bytes);
        }
        assert!(expected.is_empty(), "committed prefix frames were lost");
        assert_eq!(gap_found, gap_expected);
        assert_eq!(allocation_gap_count, usize::from(gap_expected));
        let (again, repeated_issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert_eq!(repeated_issues, issues);
        assert_eq!(again, paths);
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        for (path, bytes) in sources {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(read_failure_witness(&session).unwrap(), Some(witness));
    }

    fn allocation_session(name: &str) -> (PathBuf, PathBuf, TerminalWitness) {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory, allocation_witness,
        };
        let root = allocation_test_directory(name);
        let session = root.join("session");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        let output = root.join("replays");
        fs::create_dir_all(&campaign).unwrap();
        let mut spool = Vec::new();
        for record in allocation_records() {
            spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let witness = allocation_witness(spool.len() as u64);
        // A writer can leave a torn record after the last committed watermark.
        spool.extend_from_slice(b"torn write beyond the committed prefix");
        fs::write(campaign.join("spool.0000.h4s"), spool).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        (session, output, witness)
    }

    #[test]
    fn allocation_failure_finalizes_verified_prefix_with_exact_gap_and_is_idempotent() {
        let (session, output, witness) = allocation_session("partial-prefix");
        let paths = finalize_session(&session, &output).unwrap();
        assert_eq!(paths.len(), 1);
        let bytes = fs::read(&paths[0]).unwrap();
        let replay = Replay::open(bytes.clone()).unwrap();
        assert_eq!(
            replay
                .index
                .iter()
                .map(|index| replay.read_block(index).unwrap().len())
                .sum::<usize>(),
            3
        );
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 9,
                first_hour: 99,
                last_hour: 99,
                reason: ALLOCATION_GAP_REASON.into(),
            }]
        );
        assert!(
            matches!(replay.seek(9, 99).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
        );
        let partial = replay.controls.iter().position(|control| matches!(control,
            ControlRecord::Partial { session: 7, failed_hour: Some(99), reason } if reason.contains(ALLOCATION_GAP_REASON))).unwrap();
        let detach = replay
            .controls
            .iter()
            .position(|control| matches!(control, ControlRecord::Detach { .. }))
            .unwrap();
        assert!(partial < detach);
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        assert!(session.join(WITNESS_FILE_NAME).is_file());
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        assert!(campaign.join("spool.0000.h4s").is_file());
    }

    #[test]
    fn a_saved_segment_length_never_certifies_a_torn_terminal_watermark() {
        let (session, output, mut witness) = allocation_session("saved-torn-watermark");
        let paths = finalize_session(&session, &output).unwrap();
        let before = fs::read(&paths[0]).unwrap();
        witness.committed_bytes -= 1;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let error = finalize_session(&session, &output).unwrap_err();
        assert!(
            error.contains("spool damage precedes the committed failure watermark"),
            "{error}"
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
    }

    #[test]
    fn allocation_failure_unknown_hour_does_not_invent_a_tail_gap() {
        let (session, output, mut witness) = allocation_session("partial-idle");
        witness.failed_hour = None;
        witness.drain_complete = true;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
    }

    #[test]
    fn allocation_failure_crc_valid_record_after_watermark_is_not_committed() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("uncommitted-frame");
        let mut records = allocation_records();
        records.pop();
        let mut spool = Vec::new();
        for record in &records {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.committed_bytes = spool.len() as u64;
        witness.failed_hour = Some(5);
        let mut frame = records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        frame.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut frame.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        spool.extend(replay::spool::encode_record(&Record::Frame(frame)).unwrap());
        assert_eq!(recover_prefix(&spool).valid_bytes, spool.len());
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, &spool).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
        assert!(matches!(replay.seek(9, 5).unwrap(), SeekResult::Gap { .. }));
        assert_eq!(fs::read(path).unwrap(), spool);
    }

    #[test]
    fn a_failed_capture_saved_to_its_watermark_waits_for_nothing_and_keeps_its_evidence() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("saved-failure-evidence");
        let mut records = allocation_records();
        records.pop();
        let mut spool = Vec::new();
        for record in &records {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.committed_bytes = spool.len() as u64;
        witness.failed_hour = Some(5);
        let mut late = records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        late.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        spool.extend(replay::spool::encode_record(&Record::Frame(late)).unwrap());
        spool.extend_from_slice(b"torn write after the uncommitted frame");
        let id = witness.campaign_id.unwrap();
        let name = std::str::from_utf8(&id).unwrap();
        let campaign = session.join(name);
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, &spool).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        // The CRC-valid frame past the watermark is failure evidence, never an hour waiting.
        let waiting = unsaved(&session, name).unwrap().unwrap();
        assert_eq!(
            (waiting.hours, waiting.first_hour, waiting.last_hour),
            (3, Some(2), Some(4))
        );
        let paths = finalize_session(&session, &output).unwrap();
        let bytes = fs::read(&paths[0]).unwrap();
        assert_eq!(
            read_saved(&campaign).unwrap().segments[&0],
            witness.committed_bytes
        );
        assert!(unsaved_segments(&campaign).unwrap().is_empty());
        assert_eq!(unsaved(&session, name).unwrap(), None);
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        assert_eq!(fs::read(&path).unwrap(), spool);
    }

    #[test]
    fn allocation_failure_callback_without_spool_branch_resolves_existing_load_context() {
        let (session, output, mut witness) = allocation_session("callback-branch");
        witness.branch = None;
        witness.failed_hour = Some(5);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 5,
                reason: ALLOCATION_GAP_REASON.into(),
            }]
        );
    }

    #[test]
    fn allocation_failure_declared_branch_before_initial_clock_preserves_exact_prefix() {
        use crate::test_support::allocation_records;
        for (name, signal, failed_hour, recoverable) in [
            ("declared-no-clock", None, 21, true),
            ("declared-matching-clock", Some(Some(21)), 21, true),
            ("declared-wrong-hour", None, 22, false),
            ("declared-wrong-clock", Some(Some(22)), 21, false),
            ("declared-unknown-clock", Some(None), 21, false),
        ] {
            let (session, output, mut witness) = allocation_session(name);
            let mut records = allocation_records();
            records.pop();
            let expected_frames: Vec<_> = records
                .iter()
                .filter_map(|record| match record {
                    Record::Frame(frame) => Some(frame.clone()),
                    _ => None,
                })
                .collect();
            let branch = BranchStart {
                id: 10,
                session: witness.session,
                loaded_hour: Some(20),
                first_hour: 21,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            };
            records.push(Record::BranchStart(branch.clone()));
            if let Some(load_signal) = signal {
                records.push(Record::Control(ControlRecord::ClockWitness {
                    session: witness.session,
                    qpc: 2100,
                    hour: 21,
                    load_signal,
                }));
            }
            let mut spool = Vec::new();
            for record in &records {
                spool.extend(replay::spool::encode_record(record).unwrap());
            }
            assert_eq!(recover_prefix(&spool).valid_bytes, spool.len());
            witness.committed_bytes = spool.len() as u64;
            witness.branch = Some(branch.id);
            witness.failed_hour = Some(failed_hour);
            witness.load_signal = Some(21);
            spool.extend_from_slice(b"torn bytes after declared branch");
            let campaign =
                session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
            let spool_path = campaign.join("spool.0000.h4s");
            let witness_path = session.join(WITNESS_FILE_NAME);
            fs::write(&spool_path, &spool).unwrap();
            fs::write(&witness_path, witness.encode()).unwrap();

            let result = finalize_session(&session, &output);
            if recoverable {
                let paths = result.unwrap();
                let bytes = fs::read(&paths[0]).unwrap();
                let replay = Replay::open(bytes.clone()).unwrap();
                assert_eq!(replay.branches[1], branch);
                assert_eq!(replay.branches.len(), 2);
                let frames: Vec<_> = replay
                    .index
                    .iter()
                    .flat_map(|index| replay.read_block(index).unwrap())
                    .collect();
                assert_eq!(frames, expected_frames);
                assert_eq!(
                    replay.gaps,
                    [Gap {
                        branch: 10,
                        first_hour: 21,
                        last_hour: 21,
                        reason: ALLOCATION_GAP_REASON.into(),
                    }]
                );
                assert!(matches!(
                    replay.seek(10, 21).unwrap(),
                    SeekResult::Gap { .. }
                ));
                assert_eq!(
                    replay
                        .controls
                        .iter()
                        .filter(|control| matches!(
                            control,
                            ControlRecord::ClockWitness { hour: 21, .. }
                        ))
                        .count(),
                    usize::from(signal.is_some())
                );
                assert!(replay.controls.iter().any(|control| matches!(control,
                    ControlRecord::Partial { failed_hour: Some(21), reason, .. }
                        if reason == ALLOCATION_GAP_REASON)));
                assert_eq!(finalize_session(&session, &output).unwrap(), paths);
                assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .contains("load signal has no matching committed context")
                );
                assert_only_naming_lock(&output);
            }
            assert_eq!(fs::read(&spool_path).unwrap(), spool);
            assert_eq!(fs::read(&witness_path).unwrap(), witness.encode());
        }
    }

    #[test]
    fn allocation_failure_known_hour_without_campaign_keeps_prefix_without_a_gap() {
        let (session, output, mut witness) = allocation_session("unknown-attempted-campaign");
        witness.campaign_id = None;
        witness.branch = None;
        witness.load_signal = Some(77);
        witness.failed_hour = Some(21);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
        assert_eq!(replay.branches.len(), 1);
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
        assert_eq!(read_failure_witness(&session).unwrap(), Some(witness));
    }

    #[test]
    fn allocation_failure_same_signal_hour_discontinuity_uses_native_branch_semantics() {
        let (session, output, mut witness) = allocation_session("callback-hour-discontinuity");
        witness.branch = None;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let before = fs::read(&paths[0]).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches[1].reason, BranchReason::HourDiscontinuity);
        assert_eq!(replay.branches[1].loaded_hour, None);
        assert_eq!(replay.branches[1].parent, Parentage::Unknown);
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 10,
                first_hour: 99,
                last_hour: 99,
                reason: ALLOCATION_GAP_REASON.into()
            }]
        );
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
    }

    #[test]
    fn allocation_failure_unknown_load_read_retains_last_native_signal() {
        let (session, output, mut witness) = allocation_session("callback-unknown-load-read");
        witness.branch = None;
        witness.load_signal = None;
        witness.failed_hour = Some(5);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 5,
                reason: ALLOCATION_GAP_REASON.into()
            }]
        );
    }

    #[test]
    fn allocation_failure_existing_gaps_preserve_native_branch_continuity() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("callback-known-gaps");
        let mut records = allocation_records();
        records.insert(
            records.len() - 1,
            Record::Gap(Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 98,
                reason: "synthetic witnessed missing hours".into(),
            }),
        );
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend(replay::spool::encode_record(&record).unwrap());
        }
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        witness.branch = None;
        witness.committed_bytes = bytes.len() as u64;
        fs::write(campaign.join("spool.0000.h4s"), bytes).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert_eq!(replay.gaps.len(), 2);
        assert_eq!(
            replay.gaps[1],
            Gap {
                branch: 9,
                first_hour: 99,
                last_hour: 99,
                reason: ALLOCATION_GAP_REASON.into(),
            }
        );
    }

    #[test]
    fn allocation_failure_changed_load_signal_recovers_a_gap_only_branch_idempotently() {
        let (session, output, mut witness) = allocation_session("callback-load-failure");
        witness.branch = None;
        witness.load_signal = Some(20);
        witness.failed_hour = Some(1);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let bytes = fs::read(&paths[0]).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 2);
        assert_eq!(
            replay.branches[1],
            BranchStart {
                id: 10,
                session: 7,
                loaded_hour: None,
                first_hour: 1,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }
        );
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 10,
                first_hour: 1,
                last_hour: 1,
                reason: ALLOCATION_GAP_REASON.into(),
            }]
        );
        assert!(
            matches!(replay.seek(10, 1).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
        );
        assert!(matches!(
            replay.controls.last().unwrap(),
            ControlRecord::Detach {
                last_hour: None,
                ..
            }
        ));
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
    }

    #[test]
    fn allocation_failure_new_campaign_without_prefix_preserves_previous_campaign() {
        let (session, output, mut witness) = allocation_session("new-campaign-failure");
        witness.campaign_id = Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
        witness.branch = None;
        witness.load_signal = Some(20);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
    }

    #[test]
    fn allocation_failure_mixed_campaign_completion_keeps_successful_outputs_in_result() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("mixed-campaign-failure");
        witness.campaign_id = Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
        witness.pending_campaign_id = witness.campaign_id;
        witness.branch = None;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let failed = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        fs::create_dir_all(&failed).unwrap();
        let context = allocation_records();
        let mut spool = Vec::new();
        for record in &context[..2] {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        fs::write(failed.join("spool.0000.h4s"), spool).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert_eq!(paths.len(), 1);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no committed spool watermark"));
        assert!(
            open(&paths[0])
                .unwrap()
                .controls
                .iter()
                .any(|control| matches!(
                    control,
                    ControlRecord::Partial {
                        failed_hour: None,
                        ..
                    }
                ))
        );
    }

    #[test]
    fn allocation_failure_new_campaign_caps_late_crc_valid_previous_campaign_frame() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("mixed-late-frame");
        let committed = witness.committed_campaign_id.unwrap();
        let old = session.join(std::str::from_utf8(&committed).unwrap());
        let mut records = allocation_records();
        records.pop();
        let mut old_spool = Vec::new();
        for record in &records {
            old_spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.committed_bytes = old_spool.len() as u64;
        let mut late = records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        late.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        old_spool.extend(replay::spool::encode_record(&Record::Frame(late.clone())).unwrap());
        assert_eq!(recover_prefix(&old_spool).valid_bytes, old_spool.len());
        let old_path = old.join("spool.0000.h4s");
        fs::write(&old_path, &old_spool).unwrap();

        let attempted = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        witness.campaign_id = Some(attempted);
        witness.pending_campaign_id = Some(attempted);
        witness.branch = None;
        witness.load_signal = Some(20);
        witness.failed_hour = Some(99);
        let failed = session.join(std::str::from_utf8(&attempted).unwrap());
        fs::create_dir_all(&failed).unwrap();
        let mut failed_spool = replay::spool::encode_record(&records[0]).unwrap();
        failed_spool.extend(
            replay::spool::encode_record(&Record::BranchStart(BranchStart {
                id: 10,
                session: 7,
                loaded_hour: Some(98),
                first_hour: 99,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }))
            .unwrap(),
        );
        late.branch = 10;
        late.hour = 99;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 99;
        rows[0].play_session_started_at = 20;
        let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
            &mut late.tables[TableId::Campaign.index()]
        else {
            unreachable!()
        };
        rows[0].id = attempted.to_vec();
        failed_spool.extend(replay::spool::encode_record(&Record::Frame(late)).unwrap());
        assert_eq!(
            recover_prefix(&failed_spool).valid_bytes,
            failed_spool.len()
        );
        let failed_path = failed.join("spool.0000.h4s");
        fs::write(&failed_path, &failed_spool).unwrap();
        let witness_bytes = witness.encode();
        fs::write(session.join(WITNESS_FILE_NAME), witness_bytes).unwrap();

        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert_eq!(paths.len(), 1);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no committed spool watermark"));
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.campaign_id, committed);
        assert!(replay.index.iter().all(|block| block.last_hour <= 4));
        assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
        assert!(
            !output
                .join(format!(
                    "{}.battleplan",
                    std::str::from_utf8(&attempted).unwrap()
                ))
                .exists()
        );
        let before = fs::read(&paths[0]).unwrap();
        assert_eq!(
            finalize_session_outcome(&session, &output).unwrap().parts(),
            (paths.clone(), issues)
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
        assert_eq!(fs::read(old_path).unwrap(), old_spool);
        assert_eq!(fs::read(failed_path).unwrap(), failed_spool);
        assert_eq!(
            fs::read(session.join(WITNESS_FILE_NAME)).unwrap(),
            witness_bytes
        );
    }

    fn assert_only_naming_lock(output: &Path) {
        assert!(replay::recording_files::listing(output).unwrap().is_empty());
        let names: Vec<_> = fs::read_dir(output)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                assert!(entry.file_type().unwrap().is_file());
                entry.file_name()
            })
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("recording-names.lock")]);
    }

    #[test]
    fn allocation_failure_without_global_watermark_refuses_raw_crc_valid_frames() {
        let (session, output, mut witness) = allocation_session("uncertified-prefix");
        witness.campaign_id = None;
        witness.branch = None;
        witness.load_signal = None;
        witness.failed_hour = None;
        witness.committed_campaign_id = None;
        witness.committed_segment = None;
        witness.committed_bytes = 0;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert!(paths.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no committed spool watermark"));
        assert_only_naming_lock(&output);
    }

    #[test]
    fn allocation_failure_pending_old_campaign_caps_late_frame_with_other_global_watermark() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("pending-other-campaign");
        let pending = witness.committed_campaign_id.unwrap();
        let mut old_records = allocation_records();
        old_records.pop();
        let mut old_spool = Vec::new();
        for record in &old_records {
            old_spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.pending_campaign_id = Some(pending);
        witness.pending_segment = Some(0);
        witness.pending_bytes = old_spool.len() as u64;
        let mut late = old_records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        late.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        old_spool.extend(replay::spool::encode_record(&Record::Frame(late)).unwrap());
        assert_eq!(recover_prefix(&old_spool).valid_bytes, old_spool.len());
        let old_path = session
            .join(std::str::from_utf8(&pending).unwrap())
            .join("spool.0000.h4s");
        fs::write(&old_path, &old_spool).unwrap();

        let committed = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let mut current_spool = Vec::new();
        for mut record in old_records {
            if let Record::Frame(frame) = &mut record {
                let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
                    &mut frame.tables[TableId::Campaign.index()]
                else {
                    unreachable!()
                };
                rows[0].id = committed.to_vec();
            }
            current_spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let current = session.join(std::str::from_utf8(&committed).unwrap());
        fs::create_dir_all(&current).unwrap();
        let current_path = current.join("spool.0000.h4s");
        fs::write(&current_path, &current_spool).unwrap();
        witness.campaign_id = Some(committed);
        witness.committed_campaign_id = Some(committed);
        witness.committed_bytes = current_spool.len() as u64;
        let witness_bytes = witness.encode();
        fs::write(session.join(WITNESS_FILE_NAME), witness_bytes).unwrap();

        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(paths.len(), 2);
        let mut before = Vec::new();
        for path in &paths {
            let replay = open(path).unwrap();
            assert!(replay.index.iter().all(|block| block.last_hour <= 4));
            assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
            let failed_hour = if replay.campaign_id == committed {
                Some(99)
            } else {
                None
            };
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { failed_hour: actual, .. } if *actual == failed_hour)));
            assert_eq!(replay.gaps.len(), usize::from(failed_hour.is_some()));
            before.push(fs::read(path).unwrap());
        }
        assert_eq!(
            finalize_session_outcome(&session, &output).unwrap().parts(),
            (paths.clone(), Vec::new())
        );
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(fs::read(old_path).unwrap(), old_spool);
        assert_eq!(fs::read(current_path).unwrap(), current_spool);
        assert_eq!(
            fs::read(session.join(WITNESS_FILE_NAME)).unwrap(),
            witness_bytes
        );
    }

    #[test]
    fn allocation_failure_stable_failed_campaign_keeps_prefix_with_other_global_watermark() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("stable-failed-campaign");
        let committed = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let mut spool = Vec::new();
        for mut record in allocation_records() {
            if let Record::Frame(frame) = &mut record {
                let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
                    &mut frame.tables[TableId::Campaign.index()]
                else {
                    unreachable!()
                };
                rows[0].id = committed.to_vec();
            }
            spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let current = session.join(std::str::from_utf8(&committed).unwrap());
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("spool.0000.h4s"), &spool).unwrap();
        witness.committed_campaign_id = Some(committed);
        witness.committed_bytes = spool.len() as u64;
        witness.failed_hour = Some(5);
        witness.branch = None;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(paths.len(), 2);
        for path in paths {
            let replay = open(&path).unwrap();
            assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
            assert_eq!(
                replay.gaps.len(),
                usize::from(replay.campaign_id != committed)
            );
            if replay.campaign_id != committed {
                assert!(
                    matches!(replay.seek(9, 5).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
                );
            }
        }
    }

    #[test]
    fn allocation_failure_same_campaign_pending_cap_uses_earlier_watermark() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("pending-earlier-watermark");
        let mut prior = Vec::new();
        for record in allocation_records() {
            if matches!(&record, Record::Frame(frame) if frame.hour == 4) {
                break;
            }
            prior.extend(replay::spool::encode_record(&record).unwrap());
        }
        witness.pending_campaign_id = witness.committed_campaign_id;
        witness.pending_segment = Some(0);
        witness.pending_bytes = prior.len() as u64;
        witness.failed_hour = Some(4);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(replay.index.iter().all(|block| block.last_hour <= 3));
        assert!(matches!(replay.seek(9, 3).unwrap(), SeekResult::Frame(_)));
        assert!(
            matches!(replay.seek(9, 4).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
        );
    }

    #[test]
    fn allocation_failure_reuses_an_existing_gap_before_detach() {
        use crate::test_support::{allocation_records, allocation_witness};
        let mut records = allocation_records();
        let gap = Gap {
            branch: 9,
            first_hour: 99,
            last_hour: 99,
            reason: ALLOCATION_GAP_REASON.into(),
        };
        records.insert(records.len() - 1, Record::Gap(gap));
        let mut segments = vec![(
            0,
            Recovery {
                format: Some(replay_data::format::FormatVersion::CURRENT),
                records: records
                    .into_iter()
                    .enumerate()
                    .map(|(at, record)| {
                        Recovered::keep(
                            record,
                            replay::spool::Placement {
                                offset: at,
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
                valid_bytes: 1,
                damage: None,
            },
        )];
        let witness = allocation_witness(1);
        apply_failure_witness(&mut segments, &witness, true, None, None).unwrap();
        apply_failure_witness(&mut segments, &witness, true, None, None).unwrap();
        let records = &segments[0].1.records;
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, Recovered::Gap(_)))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(
                    record,
                    Recovered::Control(ControlRecord::Partial { .. })
                ))
                .count(),
            1
        );
        assert!(matches!(
            records[records.len() - 1],
            Recovered::Control(ControlRecord::Detach { .. })
        ));
    }

    fn write_allocation_spool(session: &Path, records: &[Record]) -> (PathBuf, Vec<u8>) {
        let campaign =
            session.join(std::str::from_utf8(crate::test_support::ALLOCATION_CAMPAIGN).unwrap());
        let mut spool = Vec::new();
        for record in records {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, &spool).unwrap();
        (path, spool)
    }

    fn allocation_gaps(branch: BranchId, hours: &[(Hour, Hour)]) -> Vec<Gap> {
        hours
            .iter()
            .map(|&(first_hour, last_hour)| Gap {
                branch,
                first_hour,
                last_hour,
                reason: ALLOCATION_GAP_REASON.into(),
            })
            .collect()
    }

    #[test]
    fn allocation_failure_gaps_the_whole_uncommitted_range_beside_the_attempt_idempotently() {
        use crate::test_support::allocation_range;
        for (name, failed_hour, expected) in [
            ("range-adjacent-attempt", Some(8), vec![(5, 8)]),
            ("range-inner-attempt", Some(6), vec![(5, 7)]),
            ("range-idle", None, vec![(5, 7)]),
            ("range-later-attempt", Some(99), vec![(5, 7), (99, 99)]),
        ] {
            let (session, output, mut witness) = allocation_session(name);
            witness.failed_hour = failed_hour;
            witness.uncommitted = Some(allocation_range(9, Some(19), 5..=7));
            fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
            let paths = finalize_session(&session, &output).unwrap();
            let bytes = fs::read(&paths[0]).unwrap();
            let replay = Replay::open(bytes.clone()).unwrap();
            assert_eq!(replay.gaps, allocation_gaps(9, &expected), "{name}");
            assert_eq!(replay.branches.len(), 1);
            assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
            for hour in 5..=7 {
                assert!(
                    matches!(replay.seek(9, hour).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
                );
            }
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { failed_hour: actual, .. } if *actual == failed_hour)));
            assert_eq!(finalize_session(&session, &output).unwrap(), paths);
            assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
            assert_eq!(read_failure_witness(&session).unwrap(), Some(witness));
        }
    }

    #[test]
    fn allocation_failure_uncommitted_range_gaps_only_hours_no_commit_covers() {
        use crate::test_support::{allocation_frame, allocation_range, allocation_records};
        // A frame written past the watermark is not committed; one inside it is never gapped.
        for (name, committed_late, range, expected) in [
            ("range-written-uncommitted", false, 5..=6, (5, 6)),
            ("range-clipped-committed", true, 6..=6, (6, 6)),
        ] {
            let (session, output, mut witness) = allocation_session(name);
            let mut records = allocation_records();
            records.pop();
            let late = || {
                [
                    Record::Frame(allocation_frame(9, 5, 19)),
                    Record::Control(ControlRecord::ClockWitness {
                        session: 7,
                        qpc: 500,
                        hour: 5,
                        load_signal: Some(19),
                    }),
                ]
            };
            if committed_late {
                records.extend(late());
            }
            let (path, mut spool) = write_allocation_spool(&session, &records);
            witness.committed_bytes = spool.len() as u64;
            witness.pending_campaign_id = witness.committed_campaign_id;
            witness.pending_segment = Some(0);
            witness.pending_bytes = spool.len() as u64;
            witness.failed_hour = None;
            witness.uncommitted = Some(allocation_range(9, Some(19), range));
            if !committed_late {
                for record in &late() {
                    spool.extend(replay::spool::encode_record(record).unwrap());
                }
                assert_eq!(recover_prefix(&spool).valid_bytes, spool.len());
                fs::write(&path, &spool).unwrap();
            }
            fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
            let paths = finalize_session(&session, &output).unwrap();
            let replay = open(&paths[0]).unwrap();
            assert_eq!(replay.gaps, allocation_gaps(9, &[expected]), "{name}");
            let last_frame = if committed_late { 5 } else { 4 };
            assert_eq!(
                crate::test_support::recorded_frames(&paths[0]),
                (2..=last_frame).map(|hour| (9, hour)).collect::<Vec<_>>()
            );
            assert!(matches!(replay.seek(9, 6).unwrap(), SeekResult::Gap { .. }));
            assert_eq!(fs::read(&path).unwrap(), spool);
        }
    }

    #[test]
    fn allocation_failure_uncommitted_range_follows_recorded_gaps_and_refuses_inner_ones() {
        use crate::test_support::{allocation_range, allocation_records};
        let overflow = |first_hour, last_hour| Gap {
            branch: 9,
            first_hour,
            last_hour,
            reason: "collector queue overflow".into(),
        };
        for (name, recorded, expected) in [
            (
                "range-after-overflow",
                overflow(5, 5),
                Some(vec![
                    overflow(5, 5),
                    allocation_gaps(9, &[(6, 7)]).remove(0),
                ]),
            ),
            (
                "range-already-gapped",
                allocation_gaps(9, &[(5, 7)]).remove(0),
                Some(allocation_gaps(9, &[(5, 7)])),
            ),
            ("range-inner-gap", overflow(6, 6), None),
        ] {
            let (session, output, mut witness) = allocation_session(name);
            let mut records = allocation_records();
            records.insert(records.len() - 1, Record::Gap(recorded));
            let (path, spool) = write_allocation_spool(&session, &records);
            witness.committed_bytes = spool.len() as u64;
            witness.failed_hour = None;
            witness.uncommitted = Some(allocation_range(9, Some(19), 5..=7));
            fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
            let result = finalize_session(&session, &output);
            match expected {
                Some(gaps) => assert_eq!(open(&result.unwrap()[0]).unwrap().gaps, gaps, "{name}"),
                None => {
                    assert!(result.unwrap_err().contains("overlaps a recorded gap"));
                    assert_only_naming_lock(&output);
                }
            }
            assert_eq!(fs::read(&path).unwrap(), spool);
        }
    }

    #[test]
    fn allocation_failure_uncommitted_range_derives_its_torn_branch_start() {
        use crate::test_support::allocation_range;
        for (name, failed_hour, expected) in [
            ("range-torn-branch", None, (21, 22)),
            ("range-torn-branch-attempt", Some(23), (21, 23)),
        ] {
            let (session, output, mut witness) = allocation_session(name);
            witness.branch = None;
            witness.failed_hour = failed_hour;
            witness.load_signal = Some(20);
            if failed_hour.is_none() {
                // Failed elsewhere: only the queue names this campaign's lost hours.
                witness.campaign_id = None;
                witness.load_signal = None;
            }
            witness.uncommitted = Some(allocation_range(10, Some(20), 21..=22));
            fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
            let paths = finalize_session(&session, &output).unwrap();
            let bytes = fs::read(&paths[0]).unwrap();
            let replay = Replay::open(bytes.clone()).unwrap();
            assert_eq!(replay.branches.len(), 2);
            assert_eq!(
                replay.branches[1],
                BranchStart {
                    id: 10,
                    session: 7,
                    loaded_hour: None,
                    first_hour: 21,
                    parent: Parentage::Unknown,
                    reason: BranchReason::LoadSignal,
                }
            );
            assert_eq!(replay.gaps, allocation_gaps(10, &[expected]), "{name}");
            assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { failed_hour: actual, .. } if *actual == failed_hour)));
            assert!(matches!(
                replay.controls.last().unwrap(),
                ControlRecord::Detach {
                    last_hour: None,
                    ..
                }
            ));
            assert_eq!(finalize_session(&session, &output).unwrap(), paths);
            assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        }
    }

    #[test]
    fn allocation_failure_uncommitted_range_in_another_campaign_keeps_both_gaps() {
        use crate::test_support::{allocation_range, allocation_records};
        let (session, output, mut witness) = allocation_session("range-other-campaign");
        let other = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let mut spool = Vec::new();
        for mut record in allocation_records() {
            if let Record::Frame(frame) = &mut record {
                let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
                    &mut frame.tables[TableId::Campaign.index()]
                else {
                    unreachable!()
                };
                rows[0].id = other.to_vec();
            }
            spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let current = session.join(std::str::from_utf8(&other).unwrap());
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("spool.0000.h4s"), &spool).unwrap();
        witness.committed_campaign_id = Some(other);
        witness.committed_bytes = spool.len() as u64;
        witness.uncommitted = Some(UncommittedRange {
            campaign_id: other,
            ..allocation_range(9, Some(19), 5..=6)
        });
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap().parts();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(paths.len(), 2);
        let mut before = Vec::new();
        for path in &paths {
            let replay = open(path).unwrap();
            let (gaps, failed_hour) = if replay.campaign_id == other {
                (allocation_gaps(9, &[(5, 6)]), None)
            } else {
                (allocation_gaps(9, &[(99, 99)]), Some(99))
            };
            assert_eq!(replay.gaps, gaps);
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { failed_hour: actual, .. } if *actual == failed_hour)));
            before.push(fs::read(path).unwrap());
        }
        assert_eq!(
            finalize_session_outcome(&session, &output).unwrap().parts(),
            (paths.clone(), Vec::new())
        );
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn allocation_failure_reapplied_uncommitted_range_reuses_its_gap() {
        use crate::test_support::{allocation_range, allocation_records, allocation_witness};
        let mut segments = vec![(
            0,
            Recovery {
                format: Some(replay_data::format::FormatVersion::CURRENT),
                records: allocation_records()
                    .into_iter()
                    .enumerate()
                    .map(|(at, record)| {
                        Recovered::keep(
                            record,
                            replay::spool::Placement {
                                offset: at,
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
                valid_bytes: 1,
                damage: None,
            },
        )];
        let witness = TerminalWitness {
            failed_hour: None,
            uncommitted: Some(allocation_range(9, Some(19), 5..=7)),
            ..allocation_witness(1)
        };
        for _ in 0..2 {
            apply_failure_witness(
                &mut segments,
                &witness,
                true,
                witness.uncommitted.as_ref(),
                None,
            )
            .unwrap();
        }
        let records = &segments[0].1.records;
        let gaps: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                Recovered::Gap(gap) => Some(gap.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(gaps, allocation_gaps(9, &[(5, 7)]));
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(
                    record,
                    Recovered::Control(ControlRecord::Partial { .. })
                ))
                .count(),
            1
        );
        assert!(matches!(
            records[records.len() - 1],
            Recovered::Control(ControlRecord::Detach { .. })
        ));
    }

    /// A checkpoint followed by a failed capture, optionally with a context-only segment.
    fn checkpointed_failure(
        name: &str,
        auto_save: bool,
        context_segment: bool,
        witness: impl FnOnce(&mut TerminalWitness),
    ) -> (PathBuf, PathBuf, PathBuf, Vec<u8>) {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_test_directory, allocation_witness, write_load_segment,
        };
        let root = allocation_test_directory(name);
        let session = root.join("session");
        let output = root.join("replays");
        let campaign_name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(campaign_name);
        let first = write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let mut committed = (0, fs::metadata(&first).unwrap().len());
        if context_segment {
            let context: Vec<u8> = recover_prefix(&fs::read(&first).unwrap())
                .records
                .iter()
                .take(2)
                .flat_map(|record| replay::spool::encode_record(record).unwrap())
                .collect();
            fs::write(campaign.join("spool.0001.h4s"), &context).unwrap();
            committed = (1, context.len() as u64);
        }
        let checkpoint = checkpoint_campaign(
            &session,
            &output,
            campaign_name,
            0,
            auto_save,
            &mut Ahead::default(),
            unnamed(),
            &|| Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(checkpoint.hours, 3);
        assert_eq!(checkpoint.deleted, usize::from(context_segment));
        let mut failure = TerminalWitness {
            committed_segment: Some(committed.0),
            committed_bytes: committed.1,
            failed_hour: None,
            ..allocation_witness(0)
        };
        witness(&mut failure);
        fs::write(session.join(WITNESS_FILE_NAME), failure.encode()).unwrap();
        let bytes = fs::read(&checkpoint.recording).unwrap();
        (session, output, checkpoint.recording, bytes)
    }

    /// `after` adds to `before` in place: every byte but the commit slot is kept.
    fn assert_added_after(before: &[u8], after: &[u8]) {
        let slot = Replay::open(before.to_vec()).unwrap().index[0].offset as usize - 32;
        assert!(after.len() > before.len());
        assert_eq!(after[..slot], before[..slot]);
        assert_eq!(after[slot + 32..before.len()], before[slot + 32..]);
    }

    fn has_partial(replay: &Replay, failed_hour: Option<Hour>) -> bool {
        replay.controls.iter().any(|control| {
            matches!(control, ControlRecord::Partial { session: 7, failed_hour: actual, reason }
                if *actual == failed_hour && reason == ALLOCATION_GAP_REASON)
        })
    }

    #[test]
    fn allocation_failure_after_an_auto_save_checkpoint_adds_only_its_lost_hours() {
        use crate::test_support::{allocation_range, recorded_frames};
        for context_segment in [false, true] {
            for (failed_hour, range, expected) in [
                (Some(8), Some(5..=7), (5, 8)),
                (Some(5), None, (5, 5)),
                (None, Some(5..=6), (5, 6)),
            ] {
                let name = format!("checkpoint-failure-saved-{context_segment}-{}", expected.1);
                let (session, output, _, checkpointed) =
                    checkpointed_failure(&name, true, context_segment, |witness| {
                        witness.failed_hour = failed_hour;
                        witness.uncommitted =
                            range.map(|hours| allocation_range(9, Some(19), hours));
                    });
                let paths = finalize_session(&session, &output).unwrap();
                assert_eq!(paths.len(), 1);
                let bytes = fs::read(&paths[0]).unwrap();
                assert_added_after(&checkpointed, &bytes);
                let replay = Replay::open(bytes.clone()).unwrap();
                assert_eq!(recorded_frames(&paths[0]), [(9, 2), (9, 3), (9, 4)]);
                assert_eq!(replay.gaps, allocation_gaps(9, &[expected]), "{name}");
                assert!(has_partial(&replay, failed_hour));
                assert_eq!(finalize_session(&session, &output).unwrap(), paths);
                assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
            }
        }
    }

    #[test]
    fn allocation_failure_elsewhere_leaves_an_auto_save_checkpoint_exactly_as_saved() {
        let (session, output, _, checkpointed) =
            checkpointed_failure("checkpoint-failure-elsewhere", true, false, |witness| {
                witness.campaign_id = Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
                witness.branch = None;
                witness.failed_hour = Some(5);
            });
        let paths = finalize_session(&session, &output).unwrap();
        assert_eq!(fs::read(&paths[0]).unwrap(), checkpointed);
    }

    #[test]
    fn allocation_failure_after_an_unsaved_checkpoint_waits_and_discards_with_it() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_range};
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        for context_segment in [false, true] {
            let (session, output, staged, checkpointed) = checkpointed_failure(
                &format!("checkpoint-failure-unsaved-{context_segment}"),
                false,
                context_segment,
                |witness| witness.uncommitted = Some(allocation_range(9, Some(19), 5..=7)),
            );
            let save = || {
                save_campaign(
                    &session,
                    &output,
                    name,
                    true,
                    &|| Ok(()),
                    &mut Ahead::default(),
                    unnamed(),
                )
                .unwrap()
                .0
            };
            assert_eq!(save(), Some(staged.clone()));
            let bytes = fs::read(&staged).unwrap();
            assert_added_after(&checkpointed, &bytes);
            let replay = Replay::open(bytes.clone()).unwrap();
            assert_eq!(replay.gaps, allocation_gaps(9, &[(5, 7)]));
            assert!(has_partial(&replay, None));
            assert_eq!(
                recording_files::recording_of(&output, ALLOCATION_CAMPAIGN).unwrap(),
                None
            );
            let campaign = session.join(name);
            let journal = read_checkpointed(&campaign).unwrap().unwrap();
            assert_eq!(journal.before, None);
            assert_eq!(journal.after, Some(replay.revision()));
            assert!(!journal.pending);
            assert_eq!(save(), Some(staged.clone()));
            assert_eq!(fs::read(&staged).unwrap(), bytes);
            // Discard takes back the failure's hours with the checkpoint they follow.
            discard_checkpoints(&session, name).unwrap();
            assert!(!staged.exists());
            assert!(segment_paths(&campaign).unwrap().is_empty());
            assert!(!campaign.join(CHECKPOINT_FILE).exists());
        }
    }

    #[test]
    fn allocation_failure_after_a_checkpoint_derives_a_torn_branch_from_the_saved_history() {
        use crate::test_support::allocation_range;
        for context_segment in [false, true] {
            let (session, output, _, checkpointed) = checkpointed_failure(
                &format!("checkpoint-failure-torn-{context_segment}"),
                true,
                context_segment,
                |witness| {
                    witness.campaign_id = None;
                    witness.branch = None;
                    witness.load_signal = None;
                    witness.uncommitted = Some(allocation_range(10, Some(20), 21..=22));
                },
            );
            let paths = finalize_session(&session, &output).unwrap();
            let bytes = fs::read(&paths[0]).unwrap();
            assert_added_after(&checkpointed, &bytes);
            let replay = Replay::open(bytes.clone()).unwrap();
            assert_eq!(
                replay.branches,
                [
                    Replay::open(checkpointed.clone()).unwrap().branches[0].clone(),
                    BranchStart {
                        id: 10,
                        session: 7,
                        loaded_hour: None,
                        first_hour: 21,
                        parent: Parentage::Unknown,
                        reason: BranchReason::LoadSignal,
                    }
                ]
            );
            assert_eq!(replay.gaps, allocation_gaps(10, &[(21, 22)]));
            assert!(has_partial(&replay, None));
            assert_eq!(finalize_session(&session, &output).unwrap(), paths);
            assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        }
    }

    #[test]
    fn allocation_failure_refuses_invalid_identity_context_and_watermarks() {
        let (session, output, witness) = allocation_session("partial-refusals");
        for bad in [
            TerminalWitness {
                session: 8,
                ..witness
            },
            TerminalWitness {
                process_start_time: 14,
                ..witness
            },
            TerminalWitness {
                branch: Some(100),
                ..witness
            },
            TerminalWitness {
                load_signal: Some(20),
                ..witness
            },
            TerminalWitness {
                failed_hour: Some(4),
                ..witness
            },
            TerminalWitness {
                committed_segment: Some(1),
                ..witness
            },
            TerminalWitness {
                committed_bytes: witness.committed_bytes - 1,
                ..witness
            },
            TerminalWitness {
                committed_bytes: u64::MAX,
                ..witness
            },
            TerminalWitness {
                uncommitted: Some(crate::test_support::allocation_range(9, Some(19), 4..=5)),
                ..witness
            },
            TerminalWitness {
                uncommitted: Some(crate::test_support::allocation_range(9, Some(20), 5..=5)),
                ..witness
            },
            TerminalWitness {
                uncommitted: Some(crate::test_support::allocation_range(100, Some(19), 5..=5)),
                ..witness
            },
            TerminalWitness {
                uncommitted: Some(crate::test_support::allocation_range(9, Some(19), 1..=1)),
                ..witness
            },
        ] {
            fs::write(session.join(WITNESS_FILE_NAME), bad.encode()).unwrap();
            assert!(finalize_session(&session, &output).is_err(), "{bad:?}");
            assert_only_naming_lock(&output);
        }
        let mut torn = witness.encode();
        torn[31] ^= 1;
        fs::write(session.join(WITNESS_FILE_NAME), torn).unwrap();
        assert!(finalize_session(&session, &output).is_err());
        fs::write(session.join(WITNESS_FILE_NAME), &torn[..WITNESS_BYTES - 1]).unwrap();
        assert!(finalize_session(&session, &output).is_err());
        fs::remove_file(session.join(WITNESS_FILE_NAME)).unwrap();
        fs::write(session.join("status.json"), br#"{"capture_failed":true}"#).unwrap();
        assert!(finalize_session(&session, &output).is_err());
    }

    #[test]
    fn allocation_failure_before_first_frame_retains_evidence_without_a_replay() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let session = allocation_test_directory("bootstrap-failure");
        let witness = TerminalWitness {
            campaign_id: None,
            branch: None,
            load_signal: None,
            failed_hour: None,
            committed_campaign_id: None,
            committed_segment: None,
            committed_bytes: 0,
            ..allocation_witness(0)
        };
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let output = session.join("output");
        assert!(
            finalize_session(&session, &output)
                .unwrap_err()
                .contains("no campaign spool")
        );
        assert!(read_failure_witness(&session).unwrap().is_some());
    }

    #[test]
    fn allocation_failure_empty_witness_preserves_healthy_finalization_bytes() {
        let (session, output, witness) = allocation_session("healthy-witness");
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        fs::OpenOptions::new()
            .write(true)
            .open(campaign.join("spool.0000.h4s"))
            .unwrap()
            .set_len(witness.committed_bytes)
            .unwrap();
        fs::remove_file(session.join(WITNESS_FILE_NAME)).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let before = fs::read(&paths[0]).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), [0; WITNESS_BYTES]).unwrap();
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
        let replay = Replay::open(before).unwrap();
        assert!(replay.gaps.is_empty());
        assert!(
            !replay
                .controls
                .iter()
                .any(|control| matches!(control, ControlRecord::Partial { .. }))
        );
    }

    #[test]
    fn character_lines_filter_by_token_or_name_and_resolve_the_template_tag() {
        use replay_data::{rows::CharacterRow, rows::LexicalTokenRow};
        use replay_frame::frame::Metrics;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        let row = |id, token: &[u8], name: &[u8], tag, line| CharacterRow {
            id,
            token: token.to_vec(),
            name: name.to_vec(),
            template_present: 1,
            template_tag: tag,
            template_file: b"common/characters/ARG.txt".to_vec(),
            template_line: line,
            ..Default::default()
        };
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![
                row(1, b"ARG_pedro_pablo_ramirez", b"", 7, 263),
                row(2, b"ARG_other", b"ARG_pedro_pablo_ramirez", 8, 329),
                row(3, b"GER_other", b"", 7, 1),
            ]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 7,
                literal: b"ARG_pedro_pablo_ramirez".to_vec(),
            }]));
        let lines = character_lines(&frame, b"pedro_pablo").unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("character id=1 "));
        assert!(lines[0].contains(
            "template_tag=7 (\"ARG_pedro_pablo_ramirez\") template_file=\"common/characters/ARG.txt\" template_line=263"
        ));
        assert!(lines[1].contains("template_tag=8 (not in LexicalTokens)"));
        assert!(character_lines(&frame, b"").is_err());
        *frame.tables[TableId::Characters.index()] = TableView::Unavailable {
            reason: "test".into(),
        };
        assert!(character_lines(&frame, b"pedro").is_err());
    }

    #[test]
    fn file_names_are_validated_without_rewriting_campaign_identity() {
        let mixed_case = "21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6";
        assert_eq!(
            campaign_filename(mixed_case.as_bytes()).unwrap(),
            mixed_case
        );
        assert_eq!(
            campaign_filename(b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6").unwrap(),
            "21b7cc84-3028-4f65-8e0d-adf2f37ca1a6"
        );
        for id in [
            b"../campaign".as_slice(),
            b"21b7cc84/3028-4f65-8e0d-adf2f37ca1a6",
            b"21b7cc84-3028-4f65-8e0d-adf2f37ca1ag",
        ] {
            assert!(campaign_filename(id).is_err());
        }
        assert_eq!(segment_number("spool.10000.h4s").unwrap(), Some(10000));
        assert_eq!(segment_number("spool.0009.h4s").unwrap(), Some(9));
        assert_eq!(segment_number("status.json").unwrap(), None);
        assert!(segment_number("spool.-001.h4s").is_err());
    }

    #[test]
    fn percentiles_use_nearest_rank_without_rounding_native_samples() {
        let mut samples: Vec<_> = (1..=100).rev().map(|n| n * 1_000_000).collect();
        assert_eq!(percentiles(&mut samples), [50.0, 95.0, 100.0]);
        assert_eq!(percentiles(&mut [1]), [0.000001; 3]);
        assert_eq!(percentiles(&mut []), [0.0; 3]);
    }

    #[test]
    fn throughput_excludes_reload_gaps_and_process_boundaries() {
        let mut result = Throughput::default();
        result.witness(1, 100, 100, 50, Some(1));
        result.witness(1, 100, 120, 51, Some(1));
        result.witness(1, 100, 150, 52, Some(2)); // next-hour reload
        result.witness(1, 100, 160, 70, Some(2)); // unexplained jump
        result.witness(1, 100, 180, 71, Some(2));
        result.witness(2, 100, 200, 72, Some(2)); // new process
        result.witness(2, 100, 190, 73, Some(2)); // invalid QPC ordering
        result.witness(3, 0, 100, 1, None);
        result.witness(3, 0, 200, 2, None);
        assert_eq!(result.hours, 2);
        assert!((result.seconds - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn repeated_controls_require_order_and_multiplicity() {
        assert!(is_subsequence([1, 2, 2, 3].iter(), [2, 2].iter()));
        assert!(!is_subsequence([1, 2, 3].iter(), [2, 2].iter()));
        assert!(!is_subsequence([1, 2, 3].iter(), [3, 2].iter()));
    }

    #[test]
    fn repeat_finalization_compares_values_and_allows_a_fresh_continuation() {
        use replay_data::{rows::CampaignRow, rows::ClockRow};
        use replay_frame::{branches::BranchReason, branches::BranchStart, frame::Metrics};
        let id = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6".to_vec();
        let session = Record::SessionStart(SessionStart {
            id: 1,
            process_id: 2,
            process_start_time: 3,
            qpc_frequency: 100,
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            capture_mode: replay_data::identity::CaptureMode::Full,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        });
        let branch = Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(1),
            first_hour: 2,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        });
        let frame = |hour, seed| {
            Record::Frame(Frame {
                branch: 1,
                hour,
                keyframe: true,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|index| match TableId::ALL[index] {
                    // Each hour's map-event interval runs from the previous hour
                    // to this frame's hour, so consecutive frames continue it.
                    TableId::MapEvents => TableUpdate::Snapshot(TableRows::MapEvents(vec![
                        replay_data::rows::MapEventsRow {
                            from_hour: hour - 1,
                            to_hour: hour,
                            ..Default::default()
                        },
                    ])),
                    TableId::Clock => TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow {
                        hour,
                        seed,
                        ..Default::default()
                    }])),
                    TableId::Campaign => {
                        TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                            id: id.clone(),
                        }]))
                    }
                    TableId::GameScoreMultipliers => TableUpdate::Unavailable {
                        reason: "score multipliers outside finalization fixture".into(),
                    },
                    TableId::EventFeedHistory => {
                        TableUpdate::Snapshot(TableRows::EventFeedHistory(vec![
                            replay_data::rows::EventFeedHistoryRow {
                                read_hour: hour,
                                coverage: replay_data::event_feed_rows::EventFeedCoverage::Complete,
                                counts: Some(Default::default()),
                                ..Default::default()
                            },
                        ]))
                    }
                    table => match replay_frame::frame::current_absence(table) {
                        Some(TableView::NotRecorded { mode }) => TableUpdate::NotRecorded { mode },
                        Some(TableView::Unavailable { reason }) => {
                            TableUpdate::Unavailable { reason }
                        }
                        None if table.key_bits() == 0 => {
                            TableUpdate::Snapshot(TableRows::one_default(table))
                        }
                        None => TableUpdate::Snapshot(TableRows::empty(table)),
                        _ => unreachable!("absence is an omission or a named refusal"),
                    },
                }),
            })
        };
        let witness = Record::Control(ControlRecord::ClockWitness {
            session: 1,
            qpc: 200,
            hour: 2,
            load_signal: None,
        });
        let first = vec![
            session.clone(),
            branch.clone(),
            frame(2, 10),
            witness.clone(),
        ];
        let previous = Replay::from_records(id.clone(), std::slice::from_ref(&first)).unwrap();
        let recovered = |records| {
            vec![(
                0,
                Recovery {
                    format: Some(replay_data::format::FormatVersion::CURRENT),
                    records,
                    valid_bytes: 0,
                    damage: None,
                },
            )]
        };
        assert!(already_finalized(&previous, &recovered(first)).unwrap());
        assert!(
            already_finalized(
                &previous,
                &recovered(vec![
                    session.clone(),
                    branch.clone(),
                    frame(2, 10),
                    witness.clone(),
                    witness,
                ])
            )
            .is_err()
        );
        assert!(
            already_finalized(
                &previous,
                &recovered(vec![session.clone(), branch.clone(), frame(2, 11)])
            )
            .is_err()
        );
        let continuation = recovered(vec![session.clone(), branch.clone(), frame(3, 10)]);
        assert!(!already_finalized(&previous, &continuation).unwrap());
        assert!(previous.append_recovered(&continuation).is_ok());
        assert!(
            already_finalized(
                &previous,
                &recovered(vec![session, branch, frame(2, 10), frame(3, 10)])
            )
            .is_err()
        );
    }

    #[test]
    fn missing_ranges_use_exact_recorded_hours_and_do_not_invent_a_tail() {
        let hours = [12, 13, 17, 19].into_iter().collect();
        assert_eq!(missing_ranges(10, &hours), [(10, 11), (14, 16), (18, 18)]);
        assert!(missing_ranges(10, &BTreeSet::new()).is_empty());
        assert!(missing_ranges(Hour::MAX, &[Hour::MAX].into_iter().collect()).is_empty());
    }
}

#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod naming_tests;

#[cfg(test)]
mod multi_context_tests;
