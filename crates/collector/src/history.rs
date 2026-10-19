//! Prepare exact prior branch history before installing the hourly hook.

use crate::spooling::campaign_name;
use replay::container::Replay;
use replay_data::{
    codec::invalid, format::FormatVersion, identity::Identity, identity::SessionStart,
};
use replay_frame::branches::BranchTracker;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

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
    lease: File,
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
}

impl PreparedCampaigns {
    pub fn load(
        directory: &Path,
        session: &SessionStart,
        mut progress: impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Self> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file()
                || !path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("battleplan") || ext.eq_ignore_ascii_case("hoi4replay"))
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if let Ok(id) = campaign_name(stem.as_bytes()) {
                engine_allocation::reserve(&mut paths, 1)?;
                paths.push((id, path));
            }
        }
        paths.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut entries = BTreeMap::new();
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
            if entries.insert(id.to_ascii_lowercase(), prepared).is_some() {
                return Err(invalid("duplicate campaign replay filename"));
            }
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
        })
    }

    fn read(path: &Path, name: &str, session: &SessionStart) -> io::Result<Prepared> {
        // Readers remain allowed. Keep this handle until detach/process exit so
        // the history used for parentage cannot change beneath the collector.
        let mut lease = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)?;
        let mut bytes = Vec::new();
        let length =
            usize::try_from(lease.metadata()?.len()).map_err(|_| engine_allocation::oom())?;
        engine_allocation::reserve_exact(&mut bytes, length)?;
        let mut chunk = [0_u8; 8192];
        loop {
            let count = match lease.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            engine_allocation::reserve(&mut bytes, count)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
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
            return Err(invalid(
                "existing replay filename differs from its campaign id",
            ));
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
            lease,
        })
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
                if self
                    .directory
                    .join(format!("{id}.battleplan"))
                    .try_exists()?
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
        Ok((prepared.tracker, Some(prepared.lease)))
    }
}

fn completion_sequence(replay: &Replay) -> io::Result<u64> {
    use replay_data::{rows::TableId, rows::tables};
    use replay_frame::frame::FrameDecoder;
    let selected = [TableId::OperationCompletions];
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
    use replay_data::{rows::TableId, rows::tables};
    use replay_frame::frame::FrameDecoder;
    let selected = [TableId::MapEvents];
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
    use replay_data::rows::TableId;
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
                    replay_frame::frame::TableView::Unavailable { .. } => continue,
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
    use replay_data::{rows::TableId, rows::tables};
    use replay_frame::{frame::FrameDecoder, frame::TableView};
    let selected = [TableId::NavalResultSources];
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
    use replay_data::{rows::TableId, rows::tables};
    use replay_frame::{frame::FrameDecoder, frame::TableView};
    let selected = [
        TableId::AirLossEvents,
        TableId::BuildingBombingDamage,
        TableId::NuclearStrikes,
        TableId::MarketEvents,
    ];
    let mut maximum = [None; 4];
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            let decoded = decoder.advance(&frame)?;
            for (index, table) in selected.into_iter().enumerate() {
                let rows = match &*decoded.tables[table.index()] {
                    TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
                    TableView::Unavailable { .. } => continue,
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
    use replay_data::{rows::TableId, rows::tables};
    use replay_frame::{frame::FrameDecoder, frame::TableView};
    let selected = [TableId::EventFeedHistory];
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
