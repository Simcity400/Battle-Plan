//! Passive finalization and inspection use the same codecs as the collector.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use replay::{
    BranchId, Hour, SessionId,
    branches::{Gap, Parentage},
    container::{RecoveryNote, Replay, SeekResult},
    format::FormatVersion,
    frame::{DecodedFrame, Frame, FrameDecoder, TABLE_COUNT, TableUpdate, TableView},
    identity::{Identity, SessionStart, Verification, is_campaign_uuid},
    rows::{TableId, TableRows, tables},
    spool::{ControlRecord, Record, RecordReader, Recovery, recover_reader},
    witness::{TerminalWitness, WITNESS_BYTES, WITNESS_FILE_NAME},
};

use crate::recording::{Omission, Refusal};

type Result<T> = std::result::Result<T, String>;
/// The replay a campaign published, if any, and each load left out with its
/// load signal and recorded hours.
type CampaignOutcome = (Option<PathBuf>, Vec<(Option<i64>, u64)>);

const ALLOCATION_GAP_REASON: &str = "collector allocation failed";

// Witness and duplicate checks need exact hours and controls, not every hour's
// owning rows. Campaign and clock snapshots also support recovered history.
fn finalization_metadata(mut record: Record) -> std::io::Result<Record> {
    if let Record::Frame(frame) = &mut record {
        for table in TableId::ALL {
            if !matches!(table, TableId::Campaign | TableId::Clock) {
                frame.tables[table.index()] = TableUpdate::Unavailable {
                    reason: engine::allocation::string("finalization metadata only")?,
                };
            }
        }
    }
    Ok(record)
}

struct FinalizationFrames {
    paths: BTreeMap<u32, (PathBuf, usize)>,
    current: Option<(u32, RecordReader<fs::File>)>,
    /// Branches of loads that are not published; their frames are passed over.
    skipped: BTreeSet<BranchId>,
}

impl FinalizationFrames {
    fn new(
        paths: &[(u32, PathBuf)],
        segments: &[(u32, Recovery)],
        skipped: BTreeSet<BranchId>,
    ) -> Self {
        Self {
            paths: paths
                .iter()
                .filter_map(|(number, path)| {
                    segments
                        .iter()
                        .find(|(segment, _)| segment == number)
                        .map(|(_, recovery)| (*number, (path.clone(), recovery.valid_bytes)))
                })
                .collect(),
            current: None,
            skipped,
        }
    }

    fn read(&mut self, number: u32, expected: &Frame) -> std::io::Result<Frame> {
        if self
            .current
            .as_ref()
            .is_none_or(|(current, _)| *current != number)
        {
            self.current = None;
            let (path, length) = self
                .paths
                .get(&number)
                .ok_or_else(|| std::io::Error::other("recovered spool segment is missing"))?;
            self.current = Some((number, RecordReader::new(fs::File::open(path)?, *length)));
        }
        let reader = &mut self.current.as_mut().unwrap().1;
        while let Some(record) = reader.next_record()? {
            if let Record::Frame(frame) = record {
                if self.skipped.contains(&frame.branch) {
                    continue;
                }
                if (frame.branch, frame.hour, frame.keyframe)
                    != (expected.branch, expected.hour, expected.keyframe)
                {
                    return Err(replay::codec::invalid(
                        "spool frame differs from recovered metadata",
                    ));
                }
                return Ok(frame);
            }
        }
        Err(replay::codec::invalid("recovered spool frame is missing"))
    }
}

/// A prepared identity record means no collector failure was published.
pub(crate) fn read_failure_witness(session_dir: &Path) -> Result<Option<TerminalWitness>> {
    let path = session_dir.join(WITNESS_FILE_NAME);
    let config_path = session_dir.join("bin/collector.json");
    let modern = config_path.is_file();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if modern {
                return Err(failure(&path, "precreated terminal witness is missing"));
            }
            return Ok(None);
        }
        Err(error) => return Err(failure(&path, error)),
    };
    if bytes.len() != WITNESS_BYTES {
        return Err(failure(&path, "terminal witness has the wrong length"));
    }
    if bytes.iter().all(|byte| *byte == 0) {
        if modern {
            return Err(failure(&path, "modern terminal witness is uninitialised"));
        }
        return Ok(None);
    }
    if let Ok(identity) = TerminalWitness::decode_prepared(&bytes) {
        if modern {
            let config: serde_json::Value = serde_json::from_slice(
                &fs::read(&config_path).map_err(|error| failure(&config_path, error))?,
            )
            .map_err(|error| failure(&config_path, error))?;
            let session_id = config["session_id"].as_u64().ok_or_else(|| {
                failure(&config_path, "collector configuration has no session id")
            })?;
            let session_start: Vec<u8> = serde_json::from_value(config["session_start"].clone())
                .map_err(|error| failure(&config_path, error))?;
            let mut decoder = replay::codec::Decoder::new(&session_start);
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
        return Ok(None);
    }
    TerminalWitness::decode(&bytes)
        .map(Some)
        .map_err(|error| failure(&path, error))
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

fn segment_paths(directory: &Path) -> Result<Vec<(u32, PathBuf)>> {
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

fn campaign_id(segments: &[(u32, Recovery)]) -> Result<Vec<u8>> {
    segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .find_map(|record| match record {
            Record::Frame(frame) => match &frame.tables[TableId::Campaign.index()] {
                TableUpdate::Snapshot(TableRows::Campaign(rows)) => match rows.as_slice() {
                    [campaign] if !campaign.id.is_empty() => Some(campaign.id.clone()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(|| "no complete campaign identity survived in the spool; directory names cannot establish identity".into())
}

fn campaign_filename(id: &[u8]) -> Result<&str> {
    if !is_campaign_uuid(id) {
        return Err(
            "recorded campaign identity is not a UUID; refusing to use it as a file name".into(),
        );
    }
    std::str::from_utf8(id).map_err(|error| error.to_string())
}

/// The campaign folders of a session that hold spool segments.
pub(crate) fn spooled_campaigns(session_dir: &Path) -> Result<Vec<String>> {
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
    campaigns.sort();
    Ok(campaigns)
}

/// Whether any campaign folder of a session holds a spool segment.
pub(crate) fn has_campaign_spools(session_dir: &Path) -> Result<bool> {
    Ok(!spooled_campaigns(session_dir)?.is_empty())
}

/// The loads a Battle Plan Recorder session admitted, by campaign and the collector's
/// load signal: the only hours the session publishes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Admissions {
    pub loads: BTreeMap<String, BTreeSet<i64>>,
    /// Loads refused, with why: kept to report what was left out, never to
    /// decide what is published.
    pub refused: BTreeMap<(String, i64), Refusal>,
}

/// A Battle Plan Recorder session's admission record in its folder: each admitted
/// campaign and load signal with the folder the session publishes into, and
/// each refused load with why, rewritten atomically as each load is decided.
pub(crate) const ADMISSIONS_FILE: &str = "admitted.json";
/// In the session's `bin` folder, written before the collector starts: it
/// marks a Battle Plan Recorder session, so a missing admission record publishes
/// nothing rather than everything.
pub(crate) const ADMISSION_MARKER: &str = "recorder.json";

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
    pub(crate) fn admit(
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
        crate::recording::write_json(&session_dir.join(ADMISSIONS_FILE), &next.to_json(target))?;
        *self = next;
        Ok(())
    }

    /// Note why a load was refused, for reporting. It is never published
    /// either way; a failed write only loses the reason.
    pub(crate) fn refuse(
        &mut self,
        session_dir: &Path,
        target: &Path,
        campaign: &str,
        load: i64,
        refusal: Refusal,
    ) -> Result<()> {
        self.refused.insert((campaign.to_owned(), load), refusal);
        crate::recording::write_json(&session_dir.join(ADMISSIONS_FILE), &self.to_json(target))
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
pub(crate) fn begin_admissions(session_dir: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(session_dir.join("bin")).map_err(|error| failure(session_dir, error))?;
    crate::recording::write_json(
        &session_dir.join(ADMISSIONS_FILE),
        &Admissions::default().to_json(target),
    )?;
    crate::recording::write_json(
        &session_dir.join("bin").join(ADMISSION_MARKER),
        &serde_json::json!({ "admission": ADMISSIONS_FILE }),
    )
}

/// The loads a session admitted. `None`: not a Battle Plan Recorder session (the
/// command line), so every load publishes as it always has. A Battle Plan Recorder
/// session whose record is missing or unreadable refuses finalization, so
/// nothing is published.
pub(crate) fn session_admissions(session_dir: &Path) -> Result<Option<Admissions>> {
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
    let record: serde_json::Value = fs::read(&path)
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
    let mut reader = RecordReader::new(file, length);
    while let Ok(Some(record)) = reader.next_record() {
        if let Record::Control(ControlRecord::ClockWitness {
            qpc, load_signal, ..
        }) = record
        {
            return Some((qpc, load_signal));
        }
    }
    None
}

/// Whether this session's spool for a campaign already holds hours from before
/// load `load`: its first clock witness carries another load signal (or none).
pub(crate) fn recorded_before_load(campaign_dir: &Path, load: i64) -> bool {
    first_witness(campaign_dir).is_some_and(|(_, signal)| signal != Some(load))
}

/// The campaign this session loaded first, as its spool shows: the one whose
/// first clock witness is earliest. Every campaign's witnesses share the
/// session's clock. Reads only the first records of each campaign's spool.
pub(crate) fn first_spooled_campaign(session_dir: &Path) -> Option<String> {
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
    order: Vec<BranchId>,
    loads: BTreeMap<BranchId, i64>,
    first_qpc: BTreeMap<BranchId, u64>,
    hours: BTreeMap<BranchId, u64>,
    current: Option<BranchId>,
}

impl BranchLoads {
    fn observe(&mut self, record: &Record) {
        match record {
            Record::SessionStart(_) => self.current = None,
            Record::BranchStart(branch) => {
                if !self.order.contains(&branch.id) {
                    self.order.push(branch.id);
                }
                self.current = Some(branch.id);
            }
            Record::Control(ControlRecord::ClockWitness {
                qpc, load_signal, ..
            }) => {
                if let Some(branch) = self.current {
                    self.first_qpc.entry(branch).or_insert(*qpc);
                    if let Some(signal) = load_signal {
                        self.loads.entry(branch).or_insert(*signal);
                    }
                }
            }
            Record::Frame(frame) => *self.hours.entry(frame.branch).or_default() += 1,
            _ => {}
        }
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
        let mut reader = RecordReader::new(file, length);
        branches.current = None;
        loop {
            match reader.next_record() {
                Ok(Some(record)) => branches.observe(&record),
                Ok(None) => break,
                // Damage ends a segment's readable prefix.
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(failure(&path, error)),
            }
        }
    }
    Ok(branches)
}

/// Each load in a campaign's spool, in spool order, with the hours it recorded.
pub(crate) fn spool_loads(campaign_dir: &Path) -> Result<Vec<(Option<i64>, u64)>> {
    let branches = read_branch_loads(campaign_dir)?;
    Ok(branches.by_load(branches.order.iter().copied()))
}

/// Every load this session's spool identifies, in the order they were loaded
/// across campaigns: by each load's first clock witness, since every
/// campaign's witnesses share the session's clock.
pub(crate) fn spooled_loads(session_dir: &Path) -> Result<Vec<(String, i64)>> {
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

/// Keep only the branches of a campaign's recovered spool whose load is
/// admitted. Each load starts its own branch, and a branch's load is the first
/// load signal its clock witnesses carry; a branch with none is not admitted.
/// `known` gives the load of the branch a validated failure witness names, for
/// a branch that failed before its first clock witness. Dropping a branch
/// drops every record in its context (its start, gaps, frames, clock witnesses
/// and the controls after them), including its start repeated at the head of
/// a later segment. Returns the dropped branches, and each left-out load with
/// the hours it recorded.
fn keep_admitted_loads(
    segments: &mut [(u32, Recovery)],
    admitted: &BTreeSet<i64>,
    known: Option<(BranchId, i64)>,
) -> (BTreeSet<BranchId>, Vec<(Option<i64>, u64)>) {
    let mut branches = BranchLoads::default();
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        branches.observe(record);
    }
    if let Some((branch, load)) = known {
        branches.loads.entry(branch).or_insert(load);
    }
    let dropped: BTreeSet<BranchId> = branches
        .order
        .iter()
        .copied()
        .filter(|branch| {
            !branches
                .loads
                .get(branch)
                .is_some_and(|load| admitted.contains(load))
        })
        .collect();
    for (_, recovery) in segments.iter_mut() {
        let mut dropping = false;
        recovery.records.retain(|record| {
            match record {
                Record::SessionStart(_) => dropping = false,
                Record::BranchStart(branch) => dropping = dropped.contains(&branch.id),
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
pub(crate) struct Finalized {
    pub replays: Vec<PathBuf>,
    /// Campaigns that could not be published.
    pub errors: Vec<String>,
    /// Loads a Battle Plan Recorder session left out, with their hours and why.
    pub omitted: Vec<Omission>,
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
pub(crate) fn finalize_session_outcome(session_dir: &Path, replay_dir: &Path) -> Result<Finalized> {
    finalize_session_outcome_with_progress(session_dir, replay_dir, &|| Ok(()))
}

pub(crate) fn finalize_session_outcome_with_progress(
    session_dir: &Path,
    replay_dir: &Path,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<Finalized> {
    progress().map_err(|error| failure(session_dir, error))?;
    let admissions = session_admissions(session_dir)?;
    let mut finalized = Finalized::default();
    let witness = read_failure_witness(session_dir)?;
    if witness.is_none()
        && ["status.json", "completed.json"].iter().any(|name| {
            fs::read(session_dir.join(name))
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
    fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let mut campaigns = Vec::new();
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_dir()
        {
            let segments = segment_paths(&entry.path())?;
            if segments.is_empty() {
                continue;
            }
            let name = entry.file_name().to_str().map(str::to_owned);
            // A Battle Plan Recorder session publishes nothing of a campaign without
            // an admitted load; its hours are reported as left out.
            let admitted = match &admissions {
                None => None,
                Some(admissions) => {
                    match name.as_deref().and_then(|name| admissions.loads.get(name)) {
                        Some(loads) => Some(loads),
                        None => {
                            if let Some(name) = &name {
                                match spool_loads(&entry.path()) {
                                    Ok(loads) => finalized.omitted.extend(loads.into_iter().map(
                                        |(load, hours)| admissions.omission(name, load, hours),
                                    )),
                                    Err(error) => finalized.errors.push(error),
                                }
                            }
                            continue;
                        }
                    }
                }
            };
            campaigns.push((entry.path(), name, segments, admitted));
        }
    }
    campaigns.sort_by(|left, right| left.0.cmp(&right.0));
    if campaigns.is_empty() && admissions.is_none() {
        return Err(failure(session_dir, "no campaign spool segments found"));
    }
    let mut process_context = None;
    for (campaign_dir, name, paths, admitted) in campaigns {
        match finalize_campaign(
            &campaign_dir,
            &paths,
            replay_dir,
            &mut process_context,
            witness.as_ref(),
            admitted,
            progress,
        ) {
            Ok((path, omitted)) => {
                finalized.replays.extend(path);
                if let (Some(admissions), Some(name)) = (&admissions, &name) {
                    finalized.omitted.extend(
                        omitted
                            .into_iter()
                            .map(|(load, hours)| admissions.omission(name, load, hours)),
                    );
                }
            }
            Err(error) => finalized.errors.push(error),
        }
    }
    if admissions.is_some() && finalized.replays.is_empty() && finalized.errors.is_empty() {
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
    Ok(finalized)
}

/// Publish one campaign's spool: every load, or with `admitted` (a the game
/// Recorder session) only those loads. Returns the recording (`None` when no
/// admitted load recorded an hour) and each load left out with its hours.
fn finalize_campaign(
    campaign_dir: &Path,
    paths: &[(u32, PathBuf)],
    replay_dir: &Path,
    process_context: &mut Option<(SessionId, u32, u64, u64)>,
    witness: Option<&TerminalWitness>,
    admitted: Option<&BTreeSet<i64>>,
    progress: &dyn Fn() -> std::io::Result<()>,
) -> Result<CampaignOutcome> {
    finalize_campaign_with_publisher(
        campaign_dir,
        paths,
        replay_dir,
        process_context,
        witness,
        admitted,
        progress,
        |replay, destination| replay.write_atomic_with(destination, |_| progress()),
    )
}

#[allow(clippy::too_many_arguments)]
fn finalize_campaign_with_publisher(
    campaign_dir: &Path,
    paths: &[(u32, PathBuf)],
    replay_dir: &Path,
    process_context: &mut Option<(SessionId, u32, u64, u64)>,
    witness: Option<&TerminalWitness>,
    admitted: Option<&BTreeSet<i64>>,
    progress: &dyn Fn() -> std::io::Result<()>,
    publish: impl FnOnce(&Replay, &Path) -> std::io::Result<()>,
) -> Result<CampaignOutcome> {
    let started = Instant::now();
    let mut spool_bytes = 0_u64;
    let mut segments = Vec::with_capacity(paths.len());
    let failing_campaign = witness.is_some_and(|witness| {
        witness.campaign_id.as_ref().is_some_and(|campaign| {
            campaign_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::as_bytes)
                == Some(campaign.as_slice())
        })
    });
    let committed_campaign = witness.is_some_and(|witness| {
        witness
            .committed_campaign_id
            .as_ref()
            .is_some_and(|campaign| {
                campaign_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::as_bytes)
                    == Some(campaign.as_slice())
            })
    });
    let pending_campaign = witness.is_some_and(|witness| {
        witness
            .pending_campaign_id
            .as_ref()
            .is_some_and(|campaign| {
                campaign_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::as_bytes)
                    == Some(campaign.as_slice())
            })
    });
    if witness.is_some_and(|witness| witness.committed_campaign_id.is_none())
        || (pending_campaign && witness.unwrap().pending_segment.is_none())
    {
        return Err(failure(
            campaign_dir,
            "failure witness has no committed spool watermark",
        ));
    }
    let watermark = witness.and_then(|witness| {
        let committed = committed_campaign
            .then(|| (witness.committed_segment.unwrap(), witness.committed_bytes));
        let pending =
            pending_campaign.then(|| (witness.pending_segment.unwrap(), witness.pending_bytes));
        match (committed, pending) {
            (Some(committed), Some(pending)) => Some(committed.min(pending)),
            (committed, pending) => committed.or(pending),
        }
    });
    let mut watermark_found = watermark.is_none();
    for (number, path) in paths {
        let file = fs::File::open(path).map_err(|error| failure(path, error))?;
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
        let recovered = recover_reader(file, prefix_length, |record| {
            let record = finalization_metadata(record)?;
            progress()?;
            Ok(record)
        })
        .map_err(|error| failure(path, error))?;
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
        segments.push((*number, recovered));
    }
    if !watermark_found {
        return Err(failure(
            campaign_dir,
            "failure witness committed segment is missing",
        ));
    }
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        if let Record::SessionStart(session) = record {
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
    let id = campaign_id(&segments).map_err(|error| failure(campaign_dir, error))?;
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
    let mut dropped = BTreeSet::new();
    let mut omitted = Vec::new();
    let mut failure_published = true;
    if let Some(admitted) = admitted {
        let last_branch = segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Record::BranchStart(branch) => Some(branch.id),
                _ => None,
            })
            .next_back();
        let known = witness
            .filter(|_| failing_campaign)
            .and_then(|witness| Some((witness.branch?, witness.load_signal?)));
        (dropped, omitted) = keep_admitted_loads(&mut segments, admitted, known);
        if !segments.iter().any(|(_, recovery)| {
            recovery
                .records
                .iter()
                .any(|record| matches!(record, Record::Frame(_)))
        }) {
            println!(
                "{}: no admitted load recorded an hour; nothing published",
                campaign_dir.display()
            );
            return Ok((None, omitted));
        }
        failure_published = last_branch.is_none_or(|branch| !dropped.contains(&branch))
            && (!failing_campaign
                || witness
                    .and_then(|witness| witness.load_signal)
                    .is_none_or(|signal| admitted.contains(&signal)));
    }
    if let Some(witness) = witness
        && failure_published
    {
        apply_failure_witness(&mut segments, witness, failing_campaign)
            .map_err(|error| failure(campaign_dir, error))?;
    }
    let destination = replay_dir.join(format!("{name}.battleplan"));
    let mut unchanged = false;
    let mut summary = Summary::default();
    let mut frames = FinalizationFrames::new(paths, &segments, dropped);
    let observe = |frame: &Frame, decoded: &DecodedFrame, sizes| {
        progress()?;
        summary
            .observe_decoded(frame, decoded, sizes)
            .map_err(std::io::Error::other)
    };
    let replay = if destination.exists() {
        let previous = open(&destination)?;
        previous
            .require_undamaged()
            .map_err(|error| failure(&destination, error))?;
        if previous.campaign_id != id {
            return Err(failure(
                &destination,
                "existing replay belongs to a different campaign",
            ));
        }
        if already_finalized_with_reader(&previous, &segments, |number, frame| {
            progress()?;
            frames.read(number, frame)
        })? {
            unchanged = true;
            previous
        } else {
            frames.current = None;
            previous
                .append_recovered_with_reader(
                    &segments,
                    |number, frame| {
                        progress()?;
                        frames.read(number, frame)
                    },
                    observe,
                )
                .map_err(|error| failure(&destination, error))?
        }
    } else {
        Replay::from_recovered_with_reader(
            id,
            &segments,
            |number, frame| {
                progress()?;
                frames.read(number, frame)
            },
            observe,
        )
        .map_err(|error| failure(&destination, error))?
    };
    if unchanged {
        // Matching bytes can survive a rename whose durability flush failed.
        replay::publication::sync_file(&destination)
            .map_err(|error| failure(&destination, error))?;
    } else {
        publish(&replay, &destination).map_err(|error| failure(&destination, error))?;
    }
    progress().map_err(|error| failure(&destination, error))?;
    let ready_seconds = started.elapsed().as_secs_f64();
    println!(
        "{}{}",
        destination.display(),
        if unchanged {
            " (already finalized; unchanged)"
        } else {
            ""
        }
    );
    println!(
        "finalize_s={ready_seconds:.3} replay_bytes={} spool_bytes={spool_bytes}",
        replay.bytes().len()
    );

    // New captures were summarized during validation and encoding. A repeated
    // finalization skipped those steps, so only that case needs a reporting pass.
    let records: Vec<_> = segments
        .iter()
        .flat_map(|(_, recovered)| &recovered.records)
        .collect();
    if unchanged {
        frames.current = None;
        for (number, recovery) in &segments {
            for record in &recovery.records {
                if let Record::Frame(frame) = record {
                    let frame = frames
                        .read(*number, frame)
                        .map_err(|error| error.to_string())?;
                    summary.observe(&frame)?;
                }
            }
        }
    }
    summary.print(spool_bytes);
    print_throughput(
        &replay.sessions,
        records.iter().filter_map(|record| match record {
            Record::Control(control) => Some(Some(control)),
            Record::Gap(_) | Record::BranchStart(_) => Some(None),
            _ => None,
        }),
    );
    let gap_count = records
        .iter()
        .filter(|record| matches!(record, Record::Gap(_)))
        .count();
    println!(
        "gaps={gap_count} damaged_segments={}",
        segments
            .iter()
            .filter(|(_, recovered)| recovered.damage.is_some())
            .count()
    );
    for record in records {
        if let Record::Control(ControlRecord::GapSummary {
            session,
            dropped_frames,
            reason,
        }) = record
        {
            println!(
                "gap_summary session={session} dropped_frames={dropped_frames} hour_range=unknown: {reason}"
            );
        }
    }
    Ok((Some(destination), omitted))
}

fn apply_failure_witness(
    segments: &mut [(u32, Recovery)],
    witness: &TerminalWitness,
    failing_campaign: bool,
) -> Result<()> {
    let mut current_branch = None;
    let mut current_signal = None;
    let mut current_observed = false;
    let mut matching_session = false;
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        match record {
            Record::SessionStart(session) => {
                if session.id != witness.session
                    || session.process_start_time != witness.process_start_time
                {
                    return Err("failure witness does not match the spool session".into());
                }
                matching_session = true;
            }
            Record::BranchStart(branch) => {
                if branch.session != witness.session {
                    return Err("failure witness does not match the spool branch session".into());
                }
                if current_branch.map(|previous: &replay::branches::BranchStart| previous.id)
                    != Some(branch.id)
                {
                    current_signal = None;
                    current_observed = false;
                }
                current_branch = Some(branch);
            }
            Record::Frame(_) | Record::Gap(_) => current_observed = true,
            Record::Control(ControlRecord::ClockWitness {
                session,
                load_signal,
                ..
            }) => {
                if *session != witness.session {
                    return Err("failure witness does not match the spool clock session".into());
                }
                current_signal = *load_signal;
                current_observed = true;
            }
            _ => {}
        }
    }
    if !matching_session {
        return Err("failure witness has no recorded session identity".into());
    }
    let mut resolved_branch = witness.branch;
    let recovered_branch = if failing_campaign
        && let Some(failed_hour) = witness.failed_hour
        && witness
            .branch
            .is_none_or(|branch| current_branch.map(|current| current.id) != Some(branch))
    {
        let history = Replay::from_recovered(witness.campaign_id.unwrap().to_vec(), segments)
            .and_then(Replay::into_history)
            .map_err(|error| error.to_string())?;
        let session = history
            .sessions
            .iter()
            .find(|session| session.id == witness.session)
            .ok_or("failure witness has no matching history session")?;
        let mut tracker = history
            .tracker_for(session)
            .map_err(|error| error.to_string())?;
        let known_gaps: Vec<_> = segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Record::Gap(gap)
                    if Some(gap.branch) == tracker.current_branch()
                        && gap.last_hour < failed_hour =>
                {
                    Some(gap.clone())
                }
                _ => None,
            })
            .collect();
        let observation = tracker
            .observe(None, failed_hour, witness.load_signal, &known_gaps)
            .map_err(|error| error.to_string())?;
        if witness
            .branch
            .is_some_and(|expected| expected != observation.branch)
        {
            return Err("failure witness branch differs from the witnessed clock context".into());
        }
        resolved_branch = Some(observation.branch);
        current_signal = witness.load_signal.or(current_signal);
        observation.start
    } else {
        None
    };
    if let Some(branch) = recovered_branch.as_ref() {
        current_branch = Some(branch);
    }
    let mut gap = None;
    if failing_campaign {
        if let Some(expected) = witness.branch
            && current_branch.map(|branch| branch.id) != Some(expected)
        {
            return Err("failure witness branch has no matching committed context".into());
        }
        if let Some(expected) = witness.load_signal
            && current_signal != Some(expected)
        {
            // A context record can commit before the new branch's first clock.
            let first_clock_failed = !current_observed
                && current_signal.is_none()
                && current_branch.is_some_and(|branch| {
                    witness.branch == Some(branch.id)
                        && witness.failed_hour == Some(branch.first_hour)
                });
            if !first_clock_failed {
                return Err("failure witness load signal has no matching committed context".into());
            }
        }
        if let Some(hour) = witness.failed_hour {
            let branch = current_branch
                .filter(|branch| Some(branch.id) == resolved_branch)
                .ok_or("known failure hour has no unambiguous recorded branch context")?;
            if hour < branch.first_hour {
                return Err("failure hour precedes the recorded branch context".into());
            }
            let mut already_gapped = false;
            for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
                match record {
                    Record::Frame(frame) if frame.branch == branch.id && frame.hour >= hour => {
                        return Err("failure hour overlaps or precedes a committed frame".into());
                    }
                    Record::Gap(existing)
                        if existing.branch == branch.id
                            && existing.first_hour <= hour
                            && existing.last_hour >= hour =>
                    {
                        if existing.reason != ALLOCATION_GAP_REASON {
                            return Err(
                                "failure hour already belongs to a gap with a different reason"
                                    .into(),
                            );
                        }
                        already_gapped = true;
                    }
                    _ => {}
                }
            }
            if !already_gapped {
                gap = Some(Gap {
                    branch: branch.id,
                    first_hour: hour,
                    last_hour: hour,
                    reason: ALLOCATION_GAP_REASON.into(),
                });
            }
        }
    }
    let partial = ControlRecord::Partial {
        session: witness.session,
        reason: ALLOCATION_GAP_REASON.into(),
        failed_hour: if failing_campaign {
            witness.failed_hour
        } else {
            None
        },
    };
    let mut partial_exists = false;
    for control in segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .filter_map(|record| {
            if let Record::Control(control @ ControlRecord::Partial { .. }) = record {
                Some(control)
            } else {
                None
            }
        })
    {
        if control != &partial {
            return Err("recorded partial outcome differs from the failure witness".into());
        }
        partial_exists = true;
    }
    let target = segments
        .iter_mut()
        .rev()
        .find(|(_, recovery)| !recovery.records.is_empty())
        .ok_or("failure witness has no recoverable record prefix")?;
    let mut position = target
        .1
        .records
        .iter()
        .position(|record| matches!(record, Record::Control(ControlRecord::Detach { .. })))
        .unwrap_or(target.1.records.len());
    if let Some(branch) = recovered_branch {
        target
            .1
            .records
            .insert(position, Record::BranchStart(branch));
        position += 1;
        if let Some(Record::Control(ControlRecord::Detach { last_hour, .. })) =
            target.1.records.get_mut(position)
        {
            *last_hour = None;
        }
    }
    if let Some(gap) = gap {
        target.1.records.insert(position, Record::Gap(gap));
        position += 1;
    }
    if !partial_exists {
        target.1.records.insert(position, Record::Control(partial));
    }
    Ok(())
}

fn open(path: &Path) -> Result<Replay> {
    Replay::open(fs::read(path).map_err(|error| failure(path, error))?)
        .map_err(|error| failure(path, error))
}

/// A repeated request is harmless only when its complete recovered contents
/// already exist. Matching hours alone must never hide different row values.
#[cfg(test)]
fn already_finalized(previous: &Replay, segments: &[(u32, Recovery)]) -> Result<bool> {
    already_finalized_with_reader(previous, segments, |_, frame| Ok(frame.clone()))
}

fn already_finalized_with_reader(
    previous: &Replay,
    segments: &[(u32, Recovery)],
    mut read: impl FnMut(u32, &Frame) -> std::io::Result<Frame>,
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
            if let Record::Frame(frame) = record
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
            Record::SessionStart(session) if !previous.sessions.contains(session) => {
                return Err(conflict());
            }
            Record::BranchStart(branch) if !previous.branches.contains(branch) => {
                return Err(conflict());
            }
            Record::Gap(gap) if !previous.gaps.contains(gap) => return Err(conflict()),
            _ => {}
        }
    }
    // Finalization keeps a retried event loss once; compare it once too.
    let mut controls: Vec<&ControlRecord> = Vec::new();
    for record in &records {
        if let Record::Control(control) = record
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
                Record::SessionStart(session) => Some(session.id),
                Record::BranchStart(branch) => Some(branch.session),
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

pub fn inspect(path: &Path) -> Result<()> {
    let replay = open(path)?;
    println!(
        "replay={} format={} bytes={} campaign={:?} keyframe_interval={}",
        path.display(),
        replay.format.version(),
        replay.bytes().len(),
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
    for index in &replay.index {
        let layout = match replay
            .block_sections(index)
            .map_err(|error| failure(path, error))?
        {
            Some(sections) => format!("grouped sections={}", sections.len()),
            None => "interleaved".into(),
        };
        println!(
            "block branch={} hours={}..={} offset={} compressed_bytes={} raw_bytes={} layout={layout}",
            index.branch,
            index.first_hour,
            index.last_hour,
            index.offset,
            index.length,
            index.raw_length
        );
        for frame in replay
            .read_block(index)
            .map_err(|error| failure(path, error))?
        {
            summary.observe(&frame)?;
        }
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
    println!(
        "oracle=not run; this command inspects recorded data without asserting save agreement"
    );
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
    if identity.coverage_policy == replay::identity::COVERAGE_POLICY_VERSION {
        println!(
            "coverage: descriptor plus map, common, history, events, localisation, interface, gfx/flags, gfx/interface, gfx/texticons, gfx/leaders, gfx/fonts, gfx/FX, gfx/posteffect_volumes.txt and every .asset and .gfx descriptor below gfx; integrated DLC and Tweaker UI roots are included in the vanilla hash under their install-relative paths; music, sound, video, loading screens, launcher, crash reporter and logs excluded"
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
        }
    }
}

impl Summary {
    fn observe(&mut self, frame: &Frame) -> Result<()> {
        let decoded = self
            .decoder
            .apply(frame)
            .map_err(|error| error.to_string())?;
        let sizes = frame
            .table_encoded_sizes()
            .map_err(|error| error.to_string())?;
        self.observe_decoded(frame, &decoded, sizes)
    }

    fn observe_decoded(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        sizes: [usize; TABLE_COUNT],
    ) -> Result<()> {
        self.totals.push(frame.metrics.total_ns);
        self.max_pool = self.max_pool.max(frame.metrics.pool_bytes);
        self.max_dropped = self.max_dropped.max(frame.metrics.dropped_frames);
        if !self
            .hours
            .entry(frame.branch)
            .or_default()
            .insert(frame.hour)
        {
            return Err(format!(
                "duplicate frame on branch {} at hour {}",
                frame.branch, frame.hour
            ));
        }
        let timed = frame.metrics.table_times_recorded();
        self.untimed_hours += u64::from(!timed);
        for table in TableId::ALL {
            if !self.recorded(table) {
                continue;
            }
            let i = table.index();
            self.table_bytes[i] += sizes[i] as u64;
            if timed {
                self.table_times[i].push(frame.metrics.table_ns[i]);
            }
            let state = match &frame.tables[i] {
                TableUpdate::Snapshot(_) => 0,
                TableUpdate::Delta { .. } => 1,
                TableUpdate::Unchanged { .. } => 2,
                TableUpdate::Truncated { .. } => 3,
                _ => 4,
            };
            self.states[i][state] += 1;
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } =
                &*decoded.tables[i]
            {
                for (field, count) in rows.field_gaps() {
                    let name = if field.contains('.') {
                        field.to_owned()
                    } else {
                        format!("{table:?}.{field}")
                    };
                    println!(
                        "Compatibility field gap: {name}: {count} occurrences not recorded at hour {}",
                        frame.hour
                    );
                }
            }
            let reason = match &*decoded.tables[i] {
                TableView::Complete(_) => continue,
                TableView::Truncated { missing, .. } => {
                    format!("truncated; {missing} rows not read")
                }
                TableView::Unavailable { reason } => format!("unavailable; {reason}"),
            };
            if let Some(previous) = self.issues[i].last_mut()
                && previous.branch == frame.branch
                && previous.last.checked_add(1) == Some(frame.hour)
                && previous.reason == reason
            {
                previous.last = frame.hour;
            } else {
                self.issues[i].push(Issue {
                    branch: frame.branch,
                    table,
                    first: frame.hour,
                    last: frame.hour,
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
        let hours = self.totals.len();
        let [p50, p95, maximum] = percentiles(&mut self.totals);
        println!(
            "hours={hours} total_p50_ms={p50:.3} total_p95_ms={p95:.3} total_max_ms={maximum:.3} pool_max_bytes={} dropped_counter_max={}",
            self.max_pool, self.max_dropped
        );
        if spool_bytes != 0 && hours != 0 {
            println!(
                "spool_bytes_per_hour={:.3}",
                spool_bytes as f64 / hours as f64
            );
        }
        println!(
            "table raw_encoded_bytes bytes_per_hour read_p50_ms read_p95_ms read_max_ms snapshots deltas unchanged truncated unavailable"
        );
        for table in TableId::ALL {
            if !self.recorded(table) {
                continue;
            }
            let i = table.index();
            let times = self.read_times(table);
            let [snapshots, deltas, unchanged, truncated, unavailable] = self.states[i];
            let per_hour = if hours == 0 {
                0.0
            } else {
                self.table_bytes[i] as f64 / hours as f64
            };
            println!(
                "{table:?} {} {per_hour:.3} {times} {snapshots} {deltas} {unchanged} {truncated} {unavailable}",
                self.table_bytes[i]
            );
        }
    }

    /// A table's read p50, p95 and maximum in milliseconds over the hours
    /// whose read times are recorded; replay blocks keep only which tables
    /// were read, so a replay without timed hours reports them not recorded.
    fn read_times(&mut self, table: TableId) -> String {
        let times = &mut self.table_times[table.index()];
        if times.is_empty() && self.untimed_hours != 0 {
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
    use replay::branches::{BranchReason, BranchStart};
    use replay::spool::recover_prefix;

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

    fn long_finalization_records(promote_hour: Option<i32>) -> Vec<Record> {
        use replay::{
            frame::FrameEncoder,
            rows::{ClockRow, LexicalTokenRow},
        };
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
        assert!(fs::read_dir(&output).unwrap().next().is_none());
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
        let nothing = |output: &Path| {
            fs::read_dir(output).map_or(true, |mut entries| entries.next().is_none())
        };
        // Nothing admitted: nothing published, no error, and the load reported left out.
        let finalized = finalize_session_outcome(&session, &output).unwrap();
        assert!(finalized.replays.is_empty() && finalized.errors.is_empty());
        assert_eq!(finalized.omitted.len(), 1);
        assert!(nothing(&output));
        // The record missing or unreadable: finalization refuses and publishes nothing.
        fs::remove_file(session.join(ADMISSIONS_FILE)).unwrap();
        assert!(session_admissions(&session).is_err());
        assert!(finalize_session(&session, &output).is_err());
        assert!(nothing(&output));
        fs::write(session.join(ADMISSIONS_FILE), b"{\"loads\":").unwrap();
        assert!(finalize_session(&session, &output).is_err());
        assert!(nothing(&output));
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
    fn repeat_finalization_retries_post_rename_durability_without_rewriting() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
        };
        use replay::container::AtomicPublisher;
        use std::{io, os::windows::ffi::OsStrExt, os::windows::fs::OpenOptionsExt};
        use windows_sys::Win32::{
            Foundation::GENERIC_WRITE,
            Storage::FileSystem::{
                DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
                FILE_SHARE_WRITE,
            },
        };

        // Keep the real recording writes, temp sync and POSIX rename. Fail at
        // the first post-rename flush, leaving the complete revision visible.
        struct FailPostRenameFlush<'a> {
            destination: &'a Path,
            temporary: PathBuf,
        }
        impl AtomicPublisher for FailPostRenameFlush<'_> {
            type Writer = fs::File;

            fn create(&mut self) -> io::Result<fs::File> {
                fs::File::create(&self.temporary)
            }

            fn sync(&mut self, writer: &mut fs::File) -> io::Result<()> {
                writer.sync_all()
            }

            fn replace(&mut self, writer: fs::File) -> io::Result<()> {
                drop(writer);
                let file = fs::OpenOptions::new()
                    .access_mode(DELETE | GENERIC_WRITE)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .open(&self.temporary)?;
                let target: Vec<_> = std::path::absolute(self.destination)?
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect();
                assert!(replay::publication::posix_replace(&file, &target)?);
                Err(io::Error::other("injected post-rename flush failure"))
            }
        }

        let directory = allocation_test_directory("post-rename-durability");
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
        let mut temporary = destination.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let error = finalize_campaign_with_publisher(
            &campaign,
            &segment_paths(&campaign).unwrap(),
            &output,
            &mut None,
            None,
            None,
            &|| Ok(()),
            |replay, path| {
                replay.publish_with(
                    &mut FailPostRenameFlush {
                        destination: path,
                        temporary: temporary.clone(),
                    },
                    |_| Ok(()),
                )
            },
        )
        .unwrap_err();
        assert!(error.contains("injected post-rename flush failure"));
        let bytes = fs::read(&destination).unwrap();
        let expected = old.append(&[records]).unwrap();
        assert_eq!(bytes, expected.bytes());
        assert_ne!(bytes, old.bytes());
        assert!(!temporary.exists());
        let modified = fs::metadata(&destination).unwrap().modified().unwrap();

        // Either unavailable flush handle must prevent recovery from reporting
        // success. The old matching-revision path succeeds under both locks.
        for path in [&destination, &output] {
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .unwrap();
            assert!(finalize_session(&session, &output).is_err());
            assert_eq!(fs::read(&destination).unwrap(), bytes);
            assert_eq!(
                fs::metadata(&destination).unwrap().modified().unwrap(),
                modified
            );
            assert!(!temporary.exists());
            drop(held);
        }
        assert_eq!(
            finalize_session(&session, &output).unwrap(),
            vec![destination.clone()],
        );
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&destination).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(fs::read(&spool_path).unwrap(), spool);
        assert!(!temporary.exists());
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
            finalization_metadata,
        )
        .unwrap();
        assert_eq!(metadata.valid_bytes, spool.len());
        assert!(metadata.damage.is_none());
        assert_eq!(
            metadata
                .records
                .iter()
                .filter(|record| matches!(record, Record::Frame(_)))
                .count(),
            20
        );
        for record in &metadata.records {
            if let Record::Frame(frame) = record {
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
                replay::rows::LexicalTokenRow {
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
                assert!(fs::read_dir(&output).unwrap().next().is_none());
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
        assert!(fs::read_dir(output).unwrap().next().is_none());
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
                format: Some(replay::format::FormatVersion::CURRENT),
                records,
                valid_bytes: 1,
                damage: None,
            },
        )];
        let witness = allocation_witness(1);
        apply_failure_witness(&mut segments, &witness, true).unwrap();
        apply_failure_witness(&mut segments, &witness, true).unwrap();
        let records = &segments[0].1.records;
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, Record::Gap(_)))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, Record::Control(ControlRecord::Partial { .. })))
                .count(),
            1
        );
        assert!(matches!(
            records[records.len() - 1],
            Record::Control(ControlRecord::Detach { .. })
        ));
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
        ] {
            fs::write(session.join(WITNESS_FILE_NAME), bad.encode()).unwrap();
            assert!(finalize_session(&session, &output).is_err(), "{bad:?}");
            assert!(fs::read_dir(&output).unwrap().next().is_none());
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
        use replay::{
            frame::Metrics,
            rows::{CharacterRow, LexicalTokenRow},
        };
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
        use replay::{
            branches::{BranchReason, BranchStart},
            frame::Metrics,
            rows::{CampaignRow, ClockRow, SunkShipsRow, WorldTensionRow},
        };
        let id = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6".to_vec();
        let session = Record::SessionStart(SessionStart {
            id: 1,
            process_id: 2,
            process_start_time: 3,
            qpc_frequency: 100,
            sample_boundary: replay::identity::SampleBoundary::EndOfHour,
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
                    TableId::SunkShips => {
                        TableUpdate::Snapshot(TableRows::SunkShips(vec![SunkShipsRow::default()]))
                    }
                    TableId::WorldTension => TableUpdate::Snapshot(TableRows::WorldTension(vec![
                        WorldTensionRow::default(),
                    ])),
                    TableId::LocalPlayer => {
                        TableUpdate::Snapshot(TableRows::LocalPlayer(vec![Default::default()]))
                    }
                    TableId::SimulationFeatures => {
                        TableUpdate::Snapshot(TableRows::SimulationFeatures(vec![
                            Default::default(),
                        ]))
                    }
                    TableId::CombatLogState => {
                        TableUpdate::Snapshot(TableRows::CombatLogState(vec![Default::default()]))
                    }
                    TableId::AirSortieModifierMetadata => {
                        TableUpdate::Snapshot(TableRows::AirSortieModifierMetadata(vec![
                            Default::default(),
                        ]))
                    }
                    TableId::ReservedCountryNaming => {
                        TableUpdate::Snapshot(TableRows::ReservedCountryNaming(vec![
                            Default::default(),
                        ]))
                    }
                    TableId::GameScoreMultipliers => TableUpdate::Unavailable {
                        reason: "score multipliers outside finalization fixture".into(),
                    },
                    TableId::EventFeedHistory => {
                        TableUpdate::Snapshot(TableRows::EventFeedHistory(vec![
                            replay::rows::EventFeedHistoryRow {
                                read_hour: hour,
                                coverage: replay::event_feed_rows::EventFeedCoverage::Complete,
                                counts: Some(Default::default()),
                                ..Default::default()
                            },
                        ]))
                    }
                    // A current spool holds no retired table; decoding names it.
                    table => match FormatVersion::CURRENT.table_refusal(table) {
                        Some(reason) => TableUpdate::Unavailable { reason },
                        None => TableUpdate::Snapshot(TableRows::empty(table)),
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
                    format: Some(replay::format::FormatVersion::CURRENT),
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
