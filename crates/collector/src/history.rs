//! Prepare exact prior branch history before installing the hourly hook.

use crate::spooling::campaign_name;
use replay::{container::Replay, recording_files};
use replay_data::{
    codec::invalid, format::FormatVersion, identity::Identity, identity::SessionStart,
};
use replay_data::rows::TableId;
use replay_frame::branches::BranchTracker;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

struct Prepared {
    campaign_id: Vec<u8>,
    identity: Identity,
    previous_session: Option<SessionStart>,
    tracker: BranchTracker,
    politics_epoch: u64,
    completion_sequence: u64,
    naval_result_sequence: u64,
    event_ledger_sequences: [Option<u64>; 4],
    event_feed_sequence: Option<u64>,
    event_feed_generation: Option<u64>,
    event_feed_counts: replay_data::event_feed_rows::EventFeedCounts,
    map_events: (u64, u64),
    lease: Option<File>,
}

#[derive(Default)]
pub struct PreparedCampaigns {
    directory: PathBuf,
    entries: BTreeMap<String, Result<Prepared, String>>,
    completion_sequence: u64,
    naval_result_sequence: u64,
    event_ledger_sequences: [Option<u64>; 4],
    event_feed_sequence: Option<u64>,
    event_feed_generation: Option<u64>,
    event_feed_counts: replay_data::event_feed_rows::EventFeedCounts,
    map_events: (u64, u64),
    /// Recorded without their history: the recorder leaves their loads out.
    without_history: BTreeSet<String>,
}

impl PreparedCampaigns {
    /// Record a campaign without its history: the recorder leaves its loads out, so whatever its
    /// history holds, even an unreadable one, cannot stop the others recording.
    pub fn without_history(&mut self, id: &str) {
        let key = id.to_ascii_lowercase();
        self.entries.remove(&key);
        self.without_history.insert(key);
    }

    pub fn load(
        directory: &Path,
        session: &SessionStart,
        mut progress: impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Self> {
        // Each recording by the campaign its header names, whatever its file name.
        let mut paths = recordings(directory)?;
        let mut entries = BTreeMap::new();
        // An unsaved recording holds its saved one and more: it is the history, found as a save
        // finds it. One that is ambiguous leaves neither guessed to be.
        for (campaign, unsaved) in recording_files::unsaved_recordings(directory)? {
            let id = campaign_name(&campaign)?;
            let saved = paths
                .iter()
                .position(|(saved, _)| saved.eq_ignore_ascii_case(&id));
            match (unsaved, saved) {
                (Ok(path), Some(saved)) => paths[saved].1 = path,
                (Ok(path), None) => {
                    engine_allocation::reserve(&mut paths, 1)?;
                    paths.push((id, path));
                }
                (Err(reason), _) => {
                    paths.retain(|(saved, _)| !saved.eq_ignore_ascii_case(&id));
                    entries.insert(id, Err(reason));
                }
            }
        }
        paths.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut completion_sequence = 0;
        let mut naval_result_sequence = 0;
        let mut event_ledger_sequences = [None; 4];
        let mut event_feed_sequence = None;
        let mut event_feed_generation = None;
        let mut event_feed_counts = replay_data::event_feed_rows::EventFeedCounts::default();
        let mut map_events = (0, 0);
        for (id, path) in paths {
            let deadline = Instant::now() + Duration::from_secs(2);
            let prepared = loop {
                progress(&id)?;
                match Self::read(&path, &id, session).map_err(engine_allocation::allocation_error) {
                    Err(error)
                        if matches!(error.raw_os_error(), Some(5 | 32 | 33))
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) if error.kind() == io::ErrorKind::OutOfMemory => return Err(error),
                    result => break result.map_err(|error| error.to_string()),
                }
            };
            if let Ok(prepared) = &prepared {
                completion_sequence = completion_sequence.max(prepared.completion_sequence);
                map_events = (
                    map_events.0.max(prepared.map_events.0),
                    map_events.1.max(prepared.map_events.1),
                );
                naval_result_sequence = naval_result_sequence.max(prepared.naval_result_sequence);
                event_feed_sequence = event_feed_sequence.max(prepared.event_feed_sequence);
                event_feed_generation = event_feed_generation.max(prepared.event_feed_generation);
                merge_event_counts(&mut event_feed_counts, prepared.event_feed_counts);
                for (maximum, prior) in event_ledger_sequences
                    .iter_mut()
                    .zip(prepared.event_ledger_sequences)
                {
                    *maximum = (*maximum).max(prior);
                }
            }
            let key = id.to_ascii_lowercase();
            if entries.contains_key(&key) {
                // Neither recording is guessed to be the campaign's.
                entries.insert(
                    key,
                    Err("the replay folder holds more than one recording of this campaign".into()),
                );
                continue;
            }
            entries.insert(key, prepared);
        }
        Ok(Self {
            directory: directory.into(),
            entries,
            completion_sequence,
            naval_result_sequence,
            event_ledger_sequences,
            event_feed_sequence,
            event_feed_generation,
            event_feed_counts,
            map_events,
            without_history: BTreeSet::new(),
        })
    }

    fn read(path: &Path, name: &str, session: &SessionStart) -> io::Result<Prepared> {
        // Freeze the initial copy; the callback subsequently uses only the resident snapshot.
        let mut snapshot = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)?;
        let mut bytes = Vec::new();
        let length =
            usize::try_from(snapshot.metadata()?.len()).map_err(|_| engine_allocation::oom())?;
        engine_allocation::reserve_exact(&mut bytes, length)?;
        let mut chunk = [0_u8; 8192];
        loop {
            let count = match snapshot.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            engine_allocation::reserve(&mut bytes, count)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        let lease = lease(path)?;
        drop(snapshot);
        let replay = Replay::open(bytes)?;
        if replay.format != FormatVersion::CURRENT {
            return Err(invalid(format!(
                "existing campaign replay is format {}; recording appends require format {}; move it aside or use another replay directory",
                replay.format.version(),
                FormatVersion::CURRENT.version()
            )));
        }
        let completion_sequence = completion_sequence(&replay)?;
        let naval_result_sequence = naval_result_sequence(&replay)?;
        let event_ledger_sequences = event_ledger_sequences(&replay)?;
        let (event_feed_sequence, event_feed_generation, event_feed_counts) =
            event_feed_history(&replay)?;
        let politics_epoch = politics_epoch(&replay)?;
        let map_events = map_event_history(&replay)?;
        let history = replay.into_history()?;
        if !history.campaign_id.eq_ignore_ascii_case(name.as_bytes()) {
            return Err(invalid("existing replay changed while it was read"));
        }
        let previous_session = history
            .sessions
            .iter()
            .find(|old| old.id == session.id)
            .cloned();
        // The callback has not verified the in-memory content roster yet. Seed
        // a resumed process using its old metadata, then compare the actual new
        // observation before consuming this prepared tracker.
        let tracker = history.tracker_for(previous_session.as_ref().unwrap_or(session))?;
        Ok(Prepared {
            campaign_id: history.campaign_id,
            identity: history.identity,
            previous_session,
            tracker,
            politics_epoch,
            completion_sequence,
            naval_result_sequence,
            event_ledger_sequences,
            event_feed_sequence,
            event_feed_generation,
            event_feed_counts,
            map_events,
            lease: Some(lease),
        })
    }

    /// Let go of a prepared campaign's history file while the recorder saves it.
    #[cfg(test)]
    pub fn release(&mut self, id: &str) {
        if let Some(Ok(prepared)) = self.entries.get_mut(&id.to_ascii_lowercase()) {
            prepared.lease = None;
        }
    }

    pub(crate) fn exchange_lease(&mut self, id: &str, mut file: Option<File>) -> Option<File> {
        if let Some(Ok(prepared)) = self.entries.get_mut(&id.to_ascii_lowercase()) {
            std::mem::swap(&mut prepared.lease, &mut file);
        }
        file
    }

    /// Hold a prepared campaign's history file again, as the recorder saved it.
    #[cfg(test)]
    pub fn retake(&mut self, id: &str, file: Option<File>) {
        if let Some(Ok(prepared)) = self.entries.get_mut(&id.to_ascii_lowercase()) {
            prepared.lease = file;
        }
    }

    /// The file that holds a campaign's history now: its unsaved recording, else its saved one
    /// found by its header.
    #[cfg(test)]
    pub fn history_path(&self, id: &str) -> io::Result<Option<PathBuf>> {
        match recording_files::unsaved_recording(&self.directory, id.as_bytes())? {
            Some(unsaved) => Ok(Some(unsaved)),
            None => recording_files::recording_of(&self.directory, id.as_bytes()),
        }
    }

    pub fn event_feed_sequence(&self) -> Option<u64> {
        self.event_feed_sequence
    }
    pub fn event_feed_generation(&self) -> Option<u64> {
        self.event_feed_generation
    }
    pub fn event_feed_counts(&self) -> replay_data::event_feed_rows::EventFeedCounts {
        self.event_feed_counts
    }
    pub fn campaign_world_generation(&self, id: &str) -> Option<u64> {
        self.entries
            .get(&id.to_ascii_lowercase())
            .and_then(|prepared| prepared.as_ref().ok())
            .and_then(|prepared| prepared.event_feed_generation)
    }

    pub fn naval_result_sequence(&self) -> u64 {
        self.naval_result_sequence
    }

    /// Air loss and building damage maxima, including retained partial prefixes.
    pub fn event_ledger_sequences(&self) -> [Option<u64>; 4] {
        self.event_ledger_sequences
    }

    pub fn completion_sequence(&self) -> u64 {
        self.completion_sequence
    }

    /// The latest map event epoch and the last sequence number any recorded
    /// interval reached, so a new session continues both.
    pub fn map_events(&self) -> (u64, u64) {
        self.map_events
    }

    pub fn politics_epoch(&self) -> u64 {
        self.entries
            .values()
            .filter_map(|entry| entry.as_ref().ok())
            .map(|entry| entry.politics_epoch)
            .max()
            .unwrap_or(0)
    }

    pub fn take(
        &mut self,
        id: &str,
        session: &SessionStart,
    ) -> io::Result<(BranchTracker, Option<File>)> {
        let key = id.to_ascii_lowercase();
        if self.without_history.contains(&key) {
            let mut tracker = BranchTracker::new();
            tracker.begin_session(session.id)?;
            return Ok((tracker, None));
        }
        match self.entries.get(&key) {
            Some(Err(reason)) => return Err(invalid(reason.clone())),
            Some(Ok(prepared)) => {
                if prepared.campaign_id != id.as_bytes() {
                    return Err(invalid("existing replay has a different campaign id"));
                }
                if !prepared.identity.same_content(&session.identity) {
                    return Err(invalid("existing campaign replay has different content"));
                }
                if prepared
                    .previous_session
                    .as_ref()
                    .is_some_and(|old| old != session)
                {
                    return Err(invalid(
                        "same process session has conflicting collector or content metadata",
                    ));
                }
            }
            None => {
                if !recording_files::recordings_of(&self.directory, id.as_bytes())?.is_empty()
                    || recording_files::unsaved_recording(&self.directory, id.as_bytes())?.is_some()
                {
                    return Err(invalid(
                        "campaign replay appeared after history preparation; restart the recorder",
                    ));
                }
                let mut tracker = BranchTracker::new();
                tracker.begin_session(session.id)?;
                return Ok((tracker, None));
            }
        }
        let prepared = self.entries.remove(&key).unwrap().map_err(invalid)?;
        Ok((prepared.tracker, prepared.lease))
    }
}

/// The recordings directly in a folder, by the campaign their header names.
fn recordings(folder: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || !recording_files::is_recording(&path) {
            continue;
        }
        // One that cannot be read may be any campaign's: preparation fails.
        if let Some(id) =
            recording_files::campaign_of(&path)?.and_then(|id| campaign_name(&id).ok())
        {
            engine_allocation::reserve(&mut paths, 1)?;
            paths.push((id, path));
        }
    }
    Ok(paths)
}

const COMPLETIONS: [TableId; 1] = [TableId::OperationCompletions];
const MAP_EVENTS: [TableId; 1] = [TableId::MapEvents];
const NAVAL_RESULTS: [TableId; 1] = [TableId::NavalResultSources];
const EVENT_LEDGERS: [TableId; 4] = [
    TableId::AirLossEvents,
    TableId::BuildingBombingDamage,
    TableId::NuclearStrikes,
    TableId::MarketEvents,
];
const EVENT_FEED: [TableId; 1] = [TableId::EventFeedHistory];

/// Whether preparing a campaign's history reads `table` back from its recording's rows.
pub(crate) fn reads(table: TableId) -> bool {
    [
        &COMPLETIONS[..],
        &MAP_EVENTS,
        &crate::extended::politics::TABLES,
        &NAVAL_RESULTS,
        &EVENT_LEDGERS,
        &EVENT_FEED,
    ]
    .iter()
    .any(|tables| tables.contains(&table))
}

/// The collector owns a resident history snapshot; append and rollback never change that snapshot.
pub(crate) fn lease(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

fn completion_sequence(replay: &Replay) -> io::Result<u64> {
    use replay_data::rows::tables;
    use replay_frame::frame::FrameDecoder;
    let selected = COMPLETIONS;
    let mut maximum = 0;
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            if let Ok(rows) = decoded.complete::<tables::OperationCompletions>() {
                maximum = maximum.max(rows.iter().map(|row| row.sequence).max().unwrap_or(0));
            }
        }
    }
    Ok(maximum)
}

fn map_event_history(replay: &Replay) -> io::Result<(u64, u64)> {
    use replay_data::rows::tables;
    use replay_frame::frame::FrameDecoder;
    let selected = MAP_EVENTS;
    let mut latest = (0, 0);
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            if let Ok([row]) = decoded.complete::<tables::MapEvents>() {
                latest = (
                    latest.0.max(row.epoch),
                    latest.1.max(row.next_sequence.saturating_sub(1)),
                );
            }
        }
    }
    Ok(latest)
}

fn politics_epoch(replay: &Replay) -> io::Result<u64> {
    use replay_frame::frame::FrameDecoder;
    let selected = crate::extended::politics::TABLES;
    let mut latest = 0;
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            for table in selected {
                let rows = match &*decoded.tables[table.index()] {
                    replay_frame::frame::TableView::Complete(rows)
                    | replay_frame::frame::TableView::Truncated { rows, .. } => rows,
                    replay_frame::frame::TableView::Unavailable { .. }
                    | replay_frame::frame::TableView::NotRecorded { .. } => continue,
                };
                // Every observed row carries the epoch, including refused census intervals.
                macro_rules! maximum {
                    ($ty:ident) => {
                        for row in rows.rows::<replay_data::rows::tables::$ty>().unwrap() {
                            latest = latest.max(row.epoch);
                        }
                    };
                }
                match table {
                    TableId::CountryPowerDaily => maximum!(CountryPowerDaily),
                    TableId::CountryPowerChanges => maximum!(CountryPowerChanges),
                    TableId::CountryPowerDailyCosts => maximum!(CountryPowerDailyCosts),
                    TableId::GovernmentChanges => maximum!(GovernmentChanges),
                    TableId::PoliticsCaptureCoverage => maximum!(PoliticsCaptureCoverage),
                    _ => unreachable!("politics selection"),
                }
            }
        }
    }
    Ok(latest)
}

fn naval_result_sequence(replay: &Replay) -> io::Result<u64> {
    use replay_data::rows::tables;
    use replay_frame::{frame::FrameDecoder, frame::TableView};
    let selected = NAVAL_RESULTS;
    let mut maximum = 0;
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } =
                &*decoded.tables[TableId::NavalResultSources.index()]
            {
                let rows = rows
                    .rows::<tables::NavalResultSources>()
                    .ok_or_else(|| invalid("naval result source history contains another table"))?;
                maximum = maximum.max(rows.iter().map(|row| row.id).max().unwrap_or(0));
            }
        }
    }
    Ok(maximum)
}

fn event_ledger_sequences(replay: &Replay) -> io::Result<[Option<u64>; 4]> {
    use replay_data::rows::tables;
    use replay_frame::{frame::FrameDecoder, frame::TableView};
    let selected = EVENT_LEDGERS;
    let mut maximum = [None; 4];
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            for (index, table) in selected.into_iter().enumerate() {
                let rows = match &*decoded.tables[table.index()] {
                    TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
                    TableView::Unavailable { .. } | TableView::NotRecorded { .. } => continue,
                };
                let last = match table {
                    TableId::AirLossEvents => rows
                        .rows::<tables::AirLossEvents>()
                        .ok_or_else(|| invalid("air loss history contains another table"))?
                        .iter()
                        .map(|row| row.id)
                        .max(),
                    TableId::BuildingBombingDamage => rows
                        .rows::<tables::BuildingBombingDamage>()
                        .ok_or_else(|| invalid("building bombing history contains another table"))?
                        .iter()
                        .map(|row| row.id)
                        .max(),
                    TableId::NuclearStrikes => rows
                        .rows::<tables::NuclearStrikes>()
                        .ok_or_else(|| invalid("nuclear strike history contains another table"))?
                        .iter()
                        .map(|row| row.id)
                        .max(),
                    TableId::MarketEvents => rows
                        .rows::<tables::MarketEvents>()
                        .ok_or_else(|| invalid("market event history contains another table"))?
                        .iter()
                        .map(|row| row.id)
                        .max(),
                    _ => unreachable!("event ledger history selection"),
                };
                maximum[index] = maximum[index].max(last);
            }
        }
    }
    Ok(maximum)
}

fn merge_event_counts(
    into: &mut replay_data::event_feed_rows::EventFeedCounts,
    counts: replay_data::event_feed_rows::EventFeedCounts,
) {
    into.acquired = into.acquired.max(counts.acquired);
    into.settled = into.settled.max(counts.settled);
    into.failed = into.failed.max(counts.failed);
    into.rejected = into.rejected.max(counts.rejected);
}
fn event_feed_history(
    replay: &Replay,
) -> io::Result<(
    Option<u64>,
    Option<u64>,
    replay_data::event_feed_rows::EventFeedCounts,
)> {
    use replay_data::rows::tables;
    use replay_frame::{frame::FrameDecoder, frame::TableView};
    let selected = EVENT_FEED;
    let mut maximum = None;
    let mut generation = None;
    let mut counts = replay_data::event_feed_rows::EventFeedCounts::default();
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            if let TableView::Complete(rows) | TableView::Truncated { rows, .. } =
                &*decoded.tables[TableId::EventFeedHistory.index()]
            {
                for row in rows
                    .rows::<tables::EventFeedHistory>()
                    .ok_or_else(|| invalid("Event feed history contains another table"))?
                {
                    row.validate()?;
                    maximum = maximum.max(row.events.iter().map(|event| event.sequence).max());
                    generation = generation.max(Some(row.generation));
                    if let Some(previous) = row.counts {
                        merge_event_counts(&mut counts, previous);
                    }
                }
            }
        }
    }
    Ok((maximum, generation, counts))
}

#[cfg(test)]
mod tests;
