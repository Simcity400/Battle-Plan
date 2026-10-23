//! Exact-hour playback through the shared replay decoder, with one cached block.

use std::{io, ops::ControlFlow, sync::Arc};

use crate::blocks::BlockReader;
use replay::container::Replay;
use replay_data::{BranchId, Hour, SessionId, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::Frame, frame::FrameDecoder, frame::TableView};

pub use dashboard_base::prepared_map::Current;

struct CachedBlock {
    index: usize,
    frames: Result<Arc<Vec<Frame>>, String>,
    decoder: FrameDecoder,
    next: usize,
    applied_hour: Option<Hour>,
    failure: Option<(Hour, String)>,
}

impl CachedBlock {
    fn reset(&mut self) {
        self.decoder = FrameDecoder::new();
        self.next = 0;
        self.applied_hour = None;
        self.failure = None;
    }
}

pub struct Playback {
    /// Shared so a worker thread can walk the same file without a copy.
    replay: Arc<Replay>,
    branch: BranchId,
    hour: Hour,
    current: Current,
    materialized: bool,
    cache: Option<CachedBlock>,
    /// Reads blocks ahead of this playback and frees passed ones elsewhere;
    /// without one, blocks are read and freed on the calling thread.
    reader: Option<Arc<BlockReader>>,
    checkpoints: Option<crate::map_checkpoints::PreparedMap>,
    checkpoint_source: Option<Arc<crate::map_checkpoints::CheckpointSource>>,
    page_workers: usize,
    source_path: Option<bool>,
    #[cfg(test)]
    source_paths: Vec<(Hour, bool, Option<Hour>)>,
    #[cfg(test)]
    block_loads: usize,
}

impl Playback {
    #[cfg(test)]
    pub fn new(replay: Replay) -> io::Result<Self> {
        Self::shared(Arc::new(replay))
    }

    /// A playback of its own over a replay another playback already holds.
    pub fn shared(replay: Arc<Replay>) -> io::Result<Self> {
        let mut playback = Self::unloaded(replay)?;
        playback.select_branch(playback.branch)?;
        Ok(playback)
    }

    /// Presentation state without synchronous decoding. The hour coordinator
    /// supplies exact frames through `present` before this state is rendered.
    pub fn unloaded(replay: Arc<Replay>) -> io::Result<Self> {
        let first = replay.branches.first().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "replay contains no branches")
        })?;
        let branch = first.id;
        let hour = first.first_hour;
        Ok(Self {
            replay,
            branch,
            hour,
            current: gap(branch, hour, "hour has not been decoded".into()),
            materialized: false,
            cache: None,
            reader: None,
            checkpoints: None,
            checkpoint_source: None,
            page_workers: 1,
            source_path: None,
            #[cfg(test)]
            source_paths: Vec::new(),
            #[cfg(test)]
            block_loads: 0,
        })
    }

    pub fn replay(&self) -> &Replay {
        &self.replay
    }

    /// Reads blocks through `reader`, which must read this playback's replay:
    /// the next block of the branch is read ahead whenever a block is loaded.
    pub fn with_reader(mut self, reader: Arc<BlockReader>) -> Self {
        debug_assert!(Arc::ptr_eq(reader.replay(), &self.replay));
        self.reader = Some(reader);
        self
    }

    /// Uses prepared selected-table checkpoints on the hour-builder thread.
    /// Hours outside the accepted prefix retain the ordinary replay reader.
    pub fn with_checkpoints(
        mut self,
        mut checkpoints: crate::map_checkpoints::PreparedMap,
    ) -> Self {
        debug_assert!(Arc::ptr_eq(checkpoints.replay_handle(), &self.replay));
        checkpoints.set_page_workers(self.page_workers);
        self.checkpoints = Some(checkpoints);
        self
    }

    pub fn with_page_workers(mut self, workers: usize) -> Self {
        self.page_workers = workers;
        if let Some(map) = &mut self.checkpoints {
            map.set_page_workers(workers);
        }
        self
    }

    pub fn with_checkpoint_source(
        mut self,
        source: Arc<crate::map_checkpoints::CheckpointSource>,
    ) -> Self {
        self.bind_checkpoint_source(source);
        self
    }

    pub fn bind_checkpoint_source(
        &mut self,
        source: Arc<crate::map_checkpoints::CheckpointSource>,
    ) {
        debug_assert!(Arc::ptr_eq(&source.replay_handle(), &self.replay));
        self.checkpoint_source = Some(source);
    }

    pub fn checkpoint_source(&self) -> Option<Arc<crate::map_checkpoints::CheckpointSource>> {
        self.checkpoint_source.clone()
    }

    #[cfg(test)]
    pub(crate) fn checkpoint_metrics(&self) -> Option<(u64, u64, u64)> {
        self.checkpoints
            .as_ref()
            .map(|map| (map.applied_frames, map.checkpoint_reads, map.source_reads))
    }

    fn refresh_checkpoints(&mut self, branch: BranchId, hour: Hour) {
        if self
            .checkpoints
            .as_ref()
            .is_some_and(|map| map.covers(branch, hour))
        {
            return;
        }
        let Some(source) = &self.checkpoint_source else {
            return;
        };
        match source.open() {
            Ok(Some(mut checkpoints)) => {
                checkpoints.set_page_workers(self.page_workers);
                self.checkpoints = Some(checkpoints);
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("Map checkpoint binding unavailable: {error}; using the replay reader")
            }
        }
    }

    fn note_source(&mut self, branch: BranchId, hour: Hour, prepared: bool) {
        let profiling = viewer_foundation::profile_writer::profiling();
        // Tests follow the source without printing it.
        #[cfg(not(test))]
        if !profiling {
            return;
        }
        if self.source_path == Some(prepared) {
            return;
        }
        self.source_path = Some(prepared);
        let through = self
            .checkpoints
            .as_ref()
            .and_then(|map| prepared_through(map, branch));
        #[cfg(test)]
        self.source_paths.push((hour, prepared, through));
        if profiling {
            let thread = std::thread::current();
            viewer_foundation::profile_writer::profile_print(source_path_line(
                thread.name().unwrap_or("unnamed"),
                branch,
                hour,
                prepared,
                through,
            ));
        }
    }

    /// Each change of source so far: hour, prepared or not, accepted end.
    #[cfg(test)]
    pub(crate) fn source_paths(&self) -> &[(Hour, bool, Option<Hour>)] {
        &self.source_paths
    }

    /// A handle to the same replay for a worker thread.
    pub fn replay_handle(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    pub fn branch(&self) -> BranchId {
        self.branch
    }

    pub fn session(&self) -> SessionId {
        self.replay
            .branches
            .iter()
            .find(|branch| branch.id == self.branch)
            .expect("selected branch belongs to replay")
            .session
    }

    pub fn hour(&self) -> Hour {
        self.hour
    }

    pub fn current(&self) -> &Current {
        &self.current
    }

    #[cfg(test)]
    pub fn frame(&self) -> Option<&DecodedFrame> {
        match &self.current {
            Current::Frame(frame) => Some(frame),
            Current::Gap { .. } => None,
        }
    }

    /// Shows an hour another playback decoded, without decoding anything.
    ///
    /// The window's hour builders decode ahead of the playhead on their own
    /// thread (`crate::hours`); this moves the position the rest of the window
    /// reads to the hour they built, in the frame that draws it.
    /// Returns the hour it replaces, for the caller to free where it likes.
    pub fn present(&mut self, branch: BranchId, hour: Hour, current: Current) -> Current {
        self.branch = branch;
        self.hour = hour;
        self.materialized = true;
        std::mem::replace(&mut self.current, current)
    }

    /// The timeline includes explicitly lost hours at either end of a branch.
    pub fn range(&self, branch: BranchId) -> Option<(Hour, Hour)> {
        if let Some(range) = self.checkpoints.as_ref().and_then(|map| map.range(branch)) {
            return Some(range);
        }
        let start = self
            .replay
            .branches
            .iter()
            .find(|entry| entry.id == branch)?;
        let mut first = start.first_hour;
        let mut last = first;
        for block in self
            .replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
        {
            first = first.min(block.first_hour);
            last = last.max(block.last_hour);
        }
        for gap in self.replay.gaps.iter().filter(|gap| gap.branch == branch) {
            first = first.min(gap.first_hour);
            last = last.max(gap.last_hour);
        }
        Some((first, last))
    }

    pub fn select_branch(&mut self, branch: BranchId) -> io::Result<()> {
        let (first, _) = self.range(branch).ok_or_else(|| unknown_branch(branch))?;
        self.seek(branch, first)
    }

    /// The hour a seek to `hour` of `branch` would walk from: the last hour the
    /// decoder applied in that hour's block when it is still before `hour`,
    /// else keyframe - 1 (i64 permits Hour::MIN - 1). None when no block holds it,
    /// or when the block failed at or before `hour`: that seek is a gap with no
    /// walk at all.
    pub fn walk_start(&self, branch: BranchId, hour: Hour) -> Option<i64> {
        if let Some(checkpoints) = &self.checkpoints
            && checkpoints.covers(branch, hour)
        {
            return checkpoints
                .walk_start(branch, hour)
                .ok()
                .flatten()
                .map(i64::from);
        }
        let index = self.replay.index.iter().position(|index| {
            index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
        })?;
        if self.cache.as_ref().is_some_and(|cache| {
            cache.index == index
                && (cache.frames.is_err()
                    || cache
                        .failure
                        .as_ref()
                        .is_some_and(|(failed, _)| *failed <= hour))
        }) {
            return None;
        }
        let continued = self
            .cache
            .as_ref()
            .filter(|cache| cache.index == index && cache.failure.is_none())
            .and_then(|cache| cache.applied_hour)
            .filter(|applied| *applied < hour);
        Some(continued.map_or_else(
            || i64::from(self.replay.index[index].first_hour) - 1,
            i64::from,
        ))
    }

    /// Seek to exactly this hour. A hole never borrows the last displayed rows.
    /// Only the target block may be decompressed; backward seeks reuse its frames.
    /// Intermediate hours advance the baseline without copying their decoded rows.
    pub fn seek(&mut self, branch: BranchId, hour: Hour) -> io::Result<()> {
        self.seek_with(branch, hour, |_| {})
    }

    /// [`Playback::seek`], handing `passed` every earlier hour of the block it
    /// decodes on the way, so a caller that also wants those hours need not
    /// seek back and decode the block again.
    pub fn seek_with(
        &mut self,
        branch: BranchId,
        hour: Hour,
        passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<()> {
        self.seek_to(branch, hour, passed, true)?;
        self.materialized = true;
        Ok(())
    }

    /// Applies a cancellation walk's endpoint without keeping an owned output
    /// alive across the next native update. Earlier passed hours stay borrowed.
    pub fn advance_without_output(&mut self, branch: BranchId, hour: Hour) -> io::Result<()> {
        self.seek_to(branch, hour, |_| {}, false)?;
        self.materialized = false;
        Ok(())
    }

    pub fn page_entry(&self, branch: BranchId, from: Hour, limit: Hour) -> Option<Hour> {
        self.checkpoints
            .as_ref()
            .filter(|map| map.covers(branch, limit))
            .and_then(|map| map.page_entry(branch, from, limit))
    }

    #[cfg(test)]
    pub fn page_fallbacks(&self) -> u64 {
        self.checkpoints
            .as_ref()
            .map_or(0, |map| map.page_fallbacks)
    }

    pub fn advance_pages(
        &mut self,
        branch: BranchId,
        hour: Hour,
        visit_from: Hour,
        mut wanted: impl FnMut() -> bool,
    ) -> ControlFlow<(), Option<Hour>> {
        if !wanted() {
            return ControlFlow::Break(());
        }
        self.refresh_checkpoints(branch, hour);
        let Some(checkpoints) = self.checkpoints.as_mut() else {
            return ControlFlow::Continue(None);
        };
        let reached = match checkpoints.advance_pages(branch, hour, visit_from, &mut wanted) {
            Ok(ControlFlow::Break(())) => return ControlFlow::Break(()),
            Ok(ControlFlow::Continue(Some(reached))) => {
                self.branch = branch;
                self.hour = reached;
                self.current = gap(branch, reached, "hour has not been decoded".into());
                self.materialized = false;
                self.cache = None;
                self.note_source(branch, reached, true);
                Some(reached)
            }
            Ok(ControlFlow::Continue(None)) => None,
            Err(_) if !wanted() => return ControlFlow::Break(()),
            Err(error) => {
                // Prepared files are disposable, as on every other route.
                eprintln!("Map checkpoint unavailable: {error}; using the replay reader");
                self.checkpoints = None;
                None
            }
        };
        ControlFlow::Continue(reached)
    }

    fn seek_to(
        &mut self,
        branch: BranchId,
        hour: Hour,
        mut passed: impl FnMut(&DecodedFrame),
        materialize: bool,
    ) -> io::Result<()> {
        if !self.replay.branches.iter().any(|entry| entry.id == branch) {
            return Err(unknown_branch(branch));
        }
        self.refresh_checkpoints(branch, hour);
        let cached_output =
            self.branch == branch && self.hour == hour && self.cache.is_some() && self.materialized;
        if !cached_output || !materialize {
            // This decode lane's old output is no longer needed. Displayed
            // hours retained by other owners stay immutable; without one the
            // prepared decoder can apply deltas in its existing row storage.
            self.current = gap(branch, hour, "hour has not been decoded".into());
            self.materialized = false;
        }
        if let Some(checkpoints) = &mut self.checkpoints {
            let prepared = if !materialize {
                checkpoints
                    .advance_without_output(branch, hour, &mut passed)
                    .map(|handled| {
                        handled.then(|| gap(branch, hour, "hour has not been decoded".into()))
                    })
            } else if self.branch == branch && self.hour <= hour {
                checkpoints.advance(branch, hour, &mut passed)
            } else {
                checkpoints.seek_with(branch, hour, &mut passed)
            };
            match prepared {
                Ok(Some(current)) => {
                    self.branch = branch;
                    self.hour = hour;
                    self.current = current;
                    self.cache = None;
                    self.note_source(branch, hour, true);
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) => {
                    // Prepared files are disposable. Their disappearance or a
                    // rejected cache page cannot refuse a readable recording.
                    eprintln!("Map checkpoint unavailable: {error}; using the replay reader");
                    self.checkpoints = None;
                }
            }
        }
        self.note_source(branch, hour, false);
        if cached_output && materialize {
            return Ok(());
        }
        self.branch = branch;
        self.hour = hour;
        let Some(index) = self.replay.index.iter().position(|index| {
            index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
        }) else {
            self.current = gap(branch, hour, self.gap_reason(branch, hour));
            return Ok(());
        };

        if self.cache.as_ref().is_none_or(|cache| cache.index != index) {
            let frames = match &self.reader {
                Some(reader) => {
                    let frames = reader.block(index);
                    // The branch's next block, read while this one plays.
                    if self
                        .replay
                        .index
                        .get(index + 1)
                        .is_some_and(|next| next.branch == branch)
                    {
                        reader.prefetch(index + 1);
                    }
                    frames
                }
                None => self
                    .replay
                    .read_block(&self.replay.index[index])
                    .map(Arc::new)
                    .map_err(|error| format!("unreadable block: {error}")),
            };
            let passed = self.cache.replace(CachedBlock {
                index,
                frames,
                decoder: FrameDecoder::new(),
                next: 0,
                applied_hour: None,
                failure: None,
            });
            if let Some(reader) = &self.reader {
                reader.drop_later(passed);
            }
            #[cfg(test)]
            {
                self.block_loads += 1;
            }
        }

        let cache = self.cache.as_mut().expect("target block was cached");
        if cache.applied_hour.is_some_and(|applied| hour < applied)
            || cache
                .failure
                .as_ref()
                .is_some_and(|(failed, _)| hour < *failed)
        {
            cache.reset();
        }
        let frames = match &cache.frames {
            Ok(frames) => frames,
            Err(reason) => {
                self.current = gap(branch, hour, reason.clone());
                return Ok(());
            }
        };
        if let Some((_, reason)) = &cache.failure {
            self.current = gap(branch, hour, reason.clone());
            return Ok(());
        }
        if let Some(decoded) = cache
            .decoder
            .current()
            .filter(|decoded| decoded.hour == hour)
        {
            if materialize {
                self.current = Current::Frame(Arc::new(decoded.clone()));
            }
            return Ok(());
        }
        while let Some(frame) = frames.get(cache.next).filter(|frame| frame.hour <= hour) {
            let decoded = match cache.decoder.advance(frame) {
                Ok(decoded) => decoded,
                Err(error) => {
                    let reason = format!("invalid frame baseline at hour {}: {error}", frame.hour);
                    cache.failure = Some((frame.hour, reason.clone()));
                    self.current = gap(branch, hour, reason);
                    return Ok(());
                }
            };
            // read_block validates the wire representation. Like Replay::seek,
            // validate identity after every applied frame, including the prefix.
            if let TableView::Complete(rows) = &*decoded.tables[TableId::Campaign.index()]
                && rows
                    .rows::<tables::Campaign>()
                    .is_none_or(|rows| rows.len() != 1 || rows[0].id != self.replay.campaign_id)
            {
                let reason = "frame campaign identity differs from replay header".to_owned();
                cache.failure = Some((frame.hour, reason.clone()));
                self.current = gap(branch, hour, reason);
                return Ok(());
            }
            cache.next += 1;
            cache.applied_hour = Some(frame.hour);
            if frame.hour == hour {
                // The decoder shares every table this hour left unchanged, so
                // the copy is one reference per table, not the rows.
                if materialize {
                    self.current = Current::Frame(Arc::new(decoded.clone()));
                }
                return Ok(());
            }
            passed(decoded);
        }
        self.current = gap(branch, hour, self.gap_reason(branch, hour));
        Ok(())
    }

    /// Advance one absolute game hour, including a gap. Stop at this branch's end.
    ///
    /// The window advances through the hours its builders finish now
    /// (`crate::hours`); this survives for the tests that pin the stepping rule.
    #[cfg(test)]
    pub fn step(&mut self) -> io::Result<bool> {
        let (_, last) = self.range(self.branch).expect("selected branch exists");
        let Some(next) = self.hour.checked_add(1).filter(|next| *next <= last) else {
            return Ok(false);
        };
        self.seek(self.branch, next)?;
        Ok(true)
    }

    fn gap_reason(&self, branch: BranchId, hour: Hour) -> String {
        let reason = self
            .replay
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
            .map(|gap| gap.reason.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        if reason.is_empty() {
            "hour was not recorded".into()
        } else {
            reason
        }
    }
}

fn gap(branch: BranchId, hour: Hour, reason: String) -> Current {
    Current::Gap {
        branch,
        hour,
        reason,
    }
}

fn unknown_branch(branch: BranchId) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("unknown branch {branch}"),
    )
}

fn prepared_through(map: &crate::map_checkpoints::PreparedMap, branch: BranchId) -> Option<Hour> {
    let (first, last) = map.range(branch)?;
    if !map.covers(branch, first) {
        return None;
    }
    let (mut covered, mut beyond) = (i64::from(first), i64::from(last) + 1);
    while beyond - covered > 1 {
        let middle = covered + (beyond - covered) / 2;
        let hour = Hour::try_from(middle).expect("between two hours");
        if map.covers(branch, hour) {
            covered = middle;
        } else {
            beyond = middle;
        }
    }
    Some(Hour::try_from(covered).expect("an hour"))
}

fn source_path_line(
    thread: &str,
    branch: BranchId,
    hour: Hour,
    prepared: bool,
    through: Option<Hour>,
) -> String {
    format!(
        "PROFILE source_path thread={} branch={branch} hour={hour} path={} prepared_through={}",
        thread.replace(char::is_whitespace, "_"),
        if prepared { "prepared" } else { "ordinary" },
        through.map_or_else(|| "none".to_owned(), |hour| hour.to_string()),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn source_path_line_names_the_thread_source_and_accepted_end() {
        assert_eq!(
            source_path_line("hour decoder", 1, 101, true, Some(101)),
            "PROFILE source_path thread=hour_decoder branch=1 hour=101 path=prepared prepared_through=101"
        );
        assert_eq!(
            source_path_line("map source decoder", 7, -3, false, None),
            "PROFILE source_path thread=map_source_decoder branch=7 hour=-3 path=ordinary prepared_through=none"
        );
    }

    use super::*;
    use replay::{container::SeekResult, spool::Record};
    use replay_data::{
        identity::Identity, identity::Roster, identity::SessionStart, identity::Verification,
        rows::AirWingRow, rows::ArmyOrderRow, rows::CampaignRow, rows::ClockRow,
        rows::ConvoyClientRow, rows::CountryOilFactorRow, rows::CountryRow, rows::DivisionRow,
        rows::FactionMemberRow, rows::FactionRow, rows::FleetRow, rows::HumanControllerRow,
        rows::LandCombatRow, rows::NavalCombatParticipantRow, rows::NavalCombatRow,
        rows::PlayerRow, rows::ProvinceRow, rows::RegionalConvoyEfficiencyRow,
        rows::ResourceExchangeRow, rows::ResourceOriginRow, rows::ResourceRightRow, rows::ShipRow,
        rows::StateRow, rows::StateTemporaryResourceRow, rows::TableRows, rows::TaskForceRow,
        rows::TradeRouteRow, rows::WarRelationRow,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Gap, branches::Parentage,
        frame::FrameEncoder, frame::Metrics, frame::TableUpdate,
    };

    fn session(id: SessionId) -> SessionStart {
        SessionStart {
            id,
            process_id: id as u32,
            process_start_time: id,
            qpc_frequency: 10_000_000,
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            capture_mode: replay_data::identity::CaptureMode::Full,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        }
    }

    fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
        DecodedFrame {
            branch,
            hour,
            metrics: Metrics::default(),
            tables: {
                let map_tables = [
                    TableView::Complete(TableRows::Clock(vec![ClockRow {
                        hour,
                        ..ClockRow::default()
                    }])),
                    TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                        id: b"campaign".to_vec(),
                    }])),
                    TableView::Complete(TableRows::Countries(vec![CountryRow {
                        id: 1,
                        canonical_slot: 1,
                        tag: b"GER".to_vec(),
                        political_power_raw: Some(i64::MAX - i64::from(hour)),
                        command_power_raw: 100_001,
                        stability_raw: 70_000,
                        war_support_raw: 30_000,
                        convoys_raw: None,
                        peace_conference_participant: 0,
                        manpower_residual: 0,
                        border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
                    }])),
                    TableView::Complete(TableRows::CountryLinks(Vec::new())),
                    TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                        id: 1,
                        state: 1,
                        owner: 1,
                        controller: 1,
                        victory_points: hour - 103,
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::States(vec![StateRow {
                        id: 1,
                        owner: 1,
                        controller: 1,
                        continent: replay_data::rows::Recorded::Value(Some(0)),
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                        id: (1_u64 << 32) | 4713,
                        roster_owner: 1,
                        logical_owner: 1,
                        expeditionary_owner: 0,
                        province: 1,
                        template: (2_u64 << 32) | 52,
                        // Whole display steps (replay_data::display), as recorded.
                        hit_points: 26_420_000,
                        organisation: 6_000_000,
                        orders_group: (57_u64 << 32) | 53,
                        reveal_skip: 0,
                    }])),
                    TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                        first_country: 1,
                        second_country: 2,
                        start_hour: 99,
                        end_hour: 0,
                        cancel: 0,
                        first_casualties: i64::from(hour),
                        second_casualties: 0,
                        first_unknown_casualties: 0,
                        second_unknown_casualties: 0,
                        first_was_instigator: 1,
                    }])),
                    TableView::Complete(TableRows::Factions(vec![FactionRow {
                        id: 1_u64 << 32 | 42,
                        name: Vec::new(),
                        ..FactionRow::default()
                    }])),
                    TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                        country: 1,
                        faction: 1_u64 << 32 | 42,
                        ordinal: 0,
                    }])),
                    TableView::Complete(TableRows::Players(vec![PlayerRow {
                        id: i32::MIN,
                        country: 1,
                        flags: 0x81,
                        user: b"synthetic user".to_vec(),
                        name: b"synthetic name".to_vec(),
                    }])),
                    TableView::Complete(TableRows::HumanControllers(vec![HumanControllerRow {
                        country: 1,
                        count: i8::MIN,
                    }])),
                    TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                        id: (9_u64 << 32) | 62,
                        province: 1,
                        duration: hour,
                        day: -1,
                        attackers: vec![(1_u64 << 32) | 4713],
                        defenders: vec![(2_u64 << 32) | 4713],
                        attacker_classes: vec![0],
                        defender_classes: vec![0],
                    }])),
                    TableView::Complete(TableRows::Fleets(vec![FleetRow {
                        id: (1_u64 << 32) | 61,
                        owner: 1,
                        home_base: 1,
                        name: replay_data::rows::Recorded::Value(Some(Vec::new())),
                        leader: replay_data::rows::Recorded::Value(Some(0)),
                    }])),
                    TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                        id: (2_u64 << 32) | 51,
                        fleet: (1_u64 << 32) | 61,
                        roster_owner: 1,
                        logical_owner: 1,
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::Ships(vec![ShipRow {
                        id: (3_u64 << 32) | 51,
                        task_force: (2_u64 << 32) | 51,
                        strength: i64::from(hour),
                        organisation: i64::MAX,
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::NavalCombats(vec![NavalCombatRow {
                        id: u64::MAX,
                        province: 1,
                        attacker_positioning: i64::MIN,
                        defender_anti_air: i64::MAX,
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::NavalCombatParticipants(vec![
                        NavalCombatParticipantRow {
                            combat: u64::MAX,
                            side: 1,
                            battle_slot: u32::MAX,
                            ship: (3_u64 << 32) | 51,
                            owner: 1,
                            state: 0x8000_0020,
                        },
                    ])),
                    TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                        id: (4_u64 << 32) | 69,
                        owner: 1,
                        air_base: 65,
                        base_override: (5_u64 << 32) | 65,
                        planes: hour,
                        mission_region: Some(u32::MAX),
                        name: replay_data::rows::Recorded::Value(Some(Vec::new())),
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::ArmyOrders(vec![ArmyOrderRow {
                        owner_group: u64::MAX,
                        instance: u32::MAX,
                        country: 1,
                        order_type: 4,
                        active: 0x80,
                        path: vec![1, hour as u32, 1],
                        target_states: vec![u32::MAX, 1],
                        paradrop_target: 7,
                    }])),
                ];
                // This fixture exercises the twenty map tables. Other domains,
                // including singleton tables, are explicitly unavailable, and a
                // retired table reads as the decoder refuses it by name.
                let mut tables = std::array::from_fn(|index| {
                    let table = TableId::ALL[index];
                    TableView::Unavailable {
                        reason: match table.last_change().retired {
                            Some(retired) => format!(
                                "{table:?} retired at format {retired}; this recording is format {}",
                                replay_data::FORMAT_VERSION
                            ),
                            None => "outside the map playback fixture".into(),
                        },
                    }
                });
                for (table, rows) in tables.iter_mut().zip(map_tables) {
                    *table = rows;
                }
                // Hourly nuclear impacts: one strike at hour 105, none otherwise.
                tables[TableId::NuclearStrikes.index()] =
                    TableView::Complete(TableRows::NuclearStrikes(if hour == 105 {
                        vec![replay_data::rows::NuclearStrikeRow {
                            id: 7,
                            hour,
                            province: 1,
                            nuke_type: 1,
                            ..Default::default()
                        }]
                    } else {
                        Vec::new()
                    }));
                // The resources map mode's and the trade routes' tables.
                for rows in resource_map_rows(hour) {
                    let table = rows.table();
                    tables[table.index()] = TableView::Complete(rows);
                }
                tables.map(Into::into)
            },
        }
    }

    /// The resources map mode's and the trade routes' hourly tables, one row each and
    /// changing every hour.
    pub(crate) fn resource_map_rows(hour: Hour) -> [TableRows; 8] {
        let efficiency = i64::from(hour) * 1_000;
        [
            TableRows::ResourceExchanges(vec![ResourceExchangeRow {
                id: 9_u64 << 32,
                resource: 7,
                exporter: 2,
                importer: 1,
                efficiency_a: efficiency,
                ..Default::default()
            }]),
            TableRows::ResourceOrigins(vec![ResourceOriginRow {
                ordinal: hour as u32,
                state: 1,
                country: 1,
                efficiency,
                convoys_assigned: 2,
                convoys_required: 3,
                ..Default::default()
            }]),
            TableRows::ResourceRights(vec![ResourceRightRow {
                ordinal: hour as u32,
                state: 1,
                resource: 1,
                holder: 2,
            }]),
            TableRows::CountryOilFactors(vec![CountryOilFactorRow {
                country: 1,
                factor: efficiency,
            }]),
            TableRows::StateTemporaryResources(vec![StateTemporaryResourceRow {
                state: 1,
                resource: 7,
                amount: efficiency,
            }]),
            TableRows::ConvoyClients(vec![ConvoyClientRow {
                owner_ordinal: Some(hour as u32),
                other_ordinal: Some(hour as u32 + 1),
                handle: 9,
                category: 2,
                owner: 1,
                other: 2,
                route_class: 1,
                route_country: 2,
                route_other: 1,
                convoys_assigned: 1,
                convoys_required: 1,
                ..Default::default()
            }]),
            TableRows::TradeRoutes(vec![TradeRouteRow {
                country: 2,
                class: 1,
                other: 1,
                kind: 2,
                from: 1,
                to: 1,
                path: vec![hour as u32],
                source_state: Some(hour as u32),
                destination_state: Some(hour as u32 + 1),
            }]),
            TableRows::RegionalConvoyEfficiency(vec![RegionalConvoyEfficiencyRow {
                country: 1,
                region: hour as u32,
                efficiency,
            }]),
        ]
    }

    pub(crate) fn records(id: u64, hours: &[Hour]) -> Vec<Record> {
        let mut records = vec![
            Record::SessionStart(session(id)),
            Record::BranchStart(BranchStart {
                id,
                session: id,
                loaded_hour: Some(hours[0] - 1),
                first_hour: hours[0],
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        let mut encoder = FrameEncoder::new();
        for &hour in hours {
            let mut frame = source(id, hour);
            if hour == 101 {
                *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
                    reason: "country read failed".into(),
                };
                *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
                    rows: TableRows::Provinces(Vec::new()),
                    missing: 1,
                };
            }
            records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        records
    }

    pub(crate) fn fixture() -> Replay {
        let mut records = records(1, &[100, 101, 104, 105, 820]);
        records.push(Record::Gap(Gap {
            branch: 1,
            first_hour: 102,
            last_hour: 104,
            reason: "flush failed".into(),
        }));
        Replay::from_records(b"campaign".to_vec(), &[records]).unwrap()
    }

    /// The fixture with its first block's last section of later entries
    /// damaged: the block is unreadable, while the next block's keyframes,
    /// which anchor only to earlier keyframe entries, stay readable.
    fn corrupt_first_block(replay: &Replay) -> Replay {
        let first = &replay.index[0];
        let sections = replay.block_sections(first).unwrap().unwrap();
        let last = sections
            .iter()
            .rposition(|section| matches!(section.section, replay::container::Section::Later(_)))
            .unwrap();
        let after: u64 = sections[last + 1..]
            .iter()
            .map(|section| section.compressed)
            .sum();
        let mut bytes = replay.bytes().to_vec();
        bytes[(first.offset + first.length - after - 1) as usize] ^= 0x40;
        Replay::open(bytes).unwrap()
    }

    fn assert_seek_matches_shared(playback: &mut Playback, branch: BranchId, hour: Hour) {
        let expected = playback.replay().seek(branch, hour).unwrap();
        playback.seek(branch, hour).unwrap();
        match (&expected, playback.current()) {
            (SeekResult::Frame(expected), Current::Frame(actual)) => {
                assert_eq!(**expected, **actual);
            }
            (
                SeekResult::Gap {
                    branch: eb,
                    hour: eh,
                    reason: er,
                },
                Current::Gap {
                    branch: ab,
                    hour: ah,
                    reason: ar,
                },
            ) => assert_eq!((eb, eh, er), (ab, ah, ar)),
            _ => panic!("expected {expected:?}, found {:?}", playback.current()),
        }
    }

    #[test]
    fn seeks_share_one_block_and_match_the_shared_reader_through_holes() {
        let mut playback = Playback::new(fixture()).unwrap();
        assert_eq!(playback.block_loads, 1);
        for hour in [100, 101, 102, 103, 104, 105, 105, 100, 104, 103, 101] {
            assert_seek_matches_shared(&mut playback, 1, hour);
            assert_eq!(playback.block_loads, 1);
        }
        assert_seek_matches_shared(&mut playback, 1, 819);
        assert_eq!(playback.block_loads, 1);
        assert_seek_matches_shared(&mut playback, 1, 820);
        assert_eq!(playback.block_loads, 2);
        assert!(!playback.step().unwrap());
        assert_eq!(playback.hour(), 820);
        assert_seek_matches_shared(&mut playback, 1, 105);
        assert_eq!(playback.block_loads, 3);
        assert!(playback.seek(999, 100).is_err());
        assert_eq!(
            (playback.branch(), playback.session(), playback.hour()),
            (1, 1, 105)
        );
    }

    #[test]
    fn steps_show_table_failures_and_gaps_without_interpolation() {
        let mut playback = Playback::new(fixture()).unwrap();
        assert!(playback.step().unwrap());
        let frame = playback.frame().unwrap();
        assert!(matches!(
            &*frame.tables[TableId::Countries.index()],
            TableView::Unavailable { reason } if reason == "country read failed"
        ));
        assert!(matches!(
            *frame.tables[TableId::Provinces.index()],
            TableView::Truncated { missing: 1, .. }
        ));
        for hour in [102, 103] {
            assert!(playback.step().unwrap());
            assert_eq!(playback.hour(), hour);
            assert!(playback.frame().is_none());
            assert!(matches!(
                playback.current(),
                Current::Gap { reason, .. } if reason == "flush failed"
            ));
        }
        assert!(playback.step().unwrap());
        assert_eq!(playback.frame(), Some(&source(1, 104)));
        assert_eq!(playback.block_loads, 1);
    }

    #[test]
    fn a_reader_reads_the_next_block_ahead_and_matches_the_shared_reader() {
        let replay = Arc::new(fixture());
        let reader = BlockReader::new(
            Arc::clone(&replay),
            Arc::new(crate::hours::Dropper::new().unwrap()),
        );
        let mut playback = Playback::unloaded(Arc::clone(&replay))
            .unwrap()
            .with_reader(Arc::clone(&reader));
        let reads = || reader.reads.load(std::sync::atomic::Ordering::SeqCst);
        for hour in [100, 101, 102, 104, 105, 100] {
            assert_seek_matches_shared(&mut playback, 1, hour);
        }
        // The first block, then the second read ahead of the playhead.
        reader.settle();
        assert_eq!(reads(), 2);
        assert_seek_matches_shared(&mut playback, 1, 820);
        assert_eq!(reads(), 2, "the read-ahead block served the boundary");
        // The second block is current now, with no block after it.
        assert_eq!(reader.kept(), (Some(1), None));
        // Back to the first block: read again, and the second read ahead of
        // it is the one already kept.
        assert_seek_matches_shared(&mut playback, 1, 105);
        reader.settle();
        assert_eq!(reads(), 3);
        assert_eq!(reader.kept(), (Some(0), Some(1)));
    }

    #[test]
    fn a_reader_reports_a_corrupt_block_as_the_plain_reader_does() {
        let replay = Arc::new(corrupt_first_block(&fixture()));
        let mut playback = Playback::unloaded(Arc::clone(&replay))
            .unwrap()
            .with_reader(BlockReader::new(
                Arc::clone(&replay),
                Arc::new(crate::hours::Dropper::new().unwrap()),
            ));
        for hour in [100, 105, 820] {
            assert_seek_matches_shared(&mut playback, 1, hour);
        }
        assert!(playback.frame().is_some());
    }

    #[test]
    fn a_failed_baseline_is_not_walked_again() {
        // Opening the branch decodes its first hour, 100.
        let mut playback = Playback::new(fixture()).unwrap();
        assert_eq!(playback.walk_start(1, 105), Some(100));
        assert_eq!(
            playback.walk_start(1, 100),
            Some(99),
            "a step back restarts"
        );
        playback.seek(1, 101).unwrap();
        assert_eq!(playback.walk_start(1, 105), Some(101), "continues from 101");
        // A baseline that failed at 104 makes every later hour of the block a
        // gap with no walk, and leaves an earlier hour's walk from the keyframe.
        let cache = playback.cache.as_mut().unwrap();
        cache.failure = Some((104, "invalid frame baseline".into()));
        assert_eq!(playback.walk_start(1, 104), None);
        assert_eq!(playback.walk_start(1, 105), None);
        // Before it, a seek restarts the block (Playback::seek resets on a
        // failure after the hour), so the walk is from the keyframe.
        assert_eq!(playback.walk_start(1, 101), Some(99));
        assert_eq!(playback.walk_start(1, 102), Some(99));
    }

    #[test]
    fn a_block_at_the_earliest_hour_walks_from_before_its_keyframe() {
        let first = Hour::MIN;
        let mut records = vec![
            Record::SessionStart(session(1)),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: None,
                first_hour: first,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        let mut encoder = FrameEncoder::new();
        for hour in [first, first + 1] {
            let mut tables: [TableView; TableId::ALL.len()] =
                std::array::from_fn(|_| TableView::Unavailable {
                    reason: "outside the fixture".into(),
                });
            tables[TableId::Campaign.index()] =
                TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                    id: b"campaign".to_vec(),
                }]));
            let frame = DecodedFrame {
                branch: 1,
                hour,
                tables: tables.map(Into::into),
                metrics: Metrics::default(),
            };
            records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        }
        let replay = Replay::from_records(b"campaign".to_vec(), &[records]).unwrap();
        let playback = Playback::unloaded(Arc::new(replay)).unwrap();
        assert_eq!(
            playback.walk_start(1, first + 1),
            Some(i64::from(first) - 1)
        );
    }

    #[test]
    fn corrupt_blocks_are_cached_and_do_not_hide_the_next_keyframe() {
        let mut playback = Playback::new(corrupt_first_block(&fixture())).unwrap();
        for hour in [100, 101, 105, 100] {
            assert_seek_matches_shared(&mut playback, 1, hour);
            assert!(matches!(
                playback.current(),
                Current::Gap { reason, .. } if reason.starts_with("unreadable block:")
            ));
            assert_eq!(playback.block_loads, 1);
        }
        assert_seek_matches_shared(&mut playback, 1, 820);
        assert!(playback.frame().is_some());
        assert_eq!(playback.block_loads, 2);
    }

    #[test]
    fn a_bad_campaign_prefix_cannot_be_hidden_by_a_later_valid_snapshot() {
        let mut playback = Playback::new(fixture()).unwrap();
        let cache = playback.cache.as_mut().unwrap();
        let frames = Arc::get_mut(cache.frames.as_mut().unwrap()).unwrap();
        frames[1].tables[TableId::Campaign.index()] =
            TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                id: b"other".to_vec(),
            }]));
        frames[2].tables[TableId::Campaign.index()] =
            TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                id: b"campaign".to_vec(),
            }]));
        for hour in [104, 105, 101, 104] {
            playback.seek(1, hour).unwrap();
            assert!(matches!(
                playback.current(),
                Current::Gap { reason, .. } if reason == "frame campaign identity differs from replay header"
            ));
        }
        playback.seek(1, 100).unwrap();
        assert_eq!(playback.frame(), Some(&source(1, 100)));
        playback.seek(1, 820).unwrap();
        assert_eq!(playback.frame(), Some(&source(1, 820)));
    }

    #[test]
    fn parentage_stays_probable_and_its_branch_selects_its_own_range() {
        let first = records(1, &[100, 101]);
        let mut second = records(2, &[101, 102]);
        let mut third = records(3, &[201]);
        if let Record::BranchStart(branch) = &mut second[1] {
            branch.parent = Parentage::Probable {
                branch: 1,
                hour: 100,
            };
        }
        if let Record::BranchStart(branch) = &mut third[1] {
            branch.parent = Parentage::Probable {
                branch: 999,
                hour: 200,
            };
        }
        let replay = Replay::from_records(b"campaign".to_vec(), &[first, second, third]).unwrap();
        let mut playback = Playback::new(replay).unwrap();
        assert!(matches!(
            playback.replay().branches[2].parent,
            Parentage::Probable { branch: 999, .. }
        ));
        playback.select_branch(2).unwrap();
        assert_eq!(
            (playback.branch(), playback.session(), playback.hour()),
            (2, 2, 101)
        );
        assert_eq!(playback.range(2), Some((101, 102)));
    }

    #[test]
    fn lost_initial_and_trailing_hours_remain_on_the_timeline() {
        let mut records = records(1, &[100, 101]);
        if let Record::BranchStart(branch) = &mut records[1] {
            branch.first_hour = 98;
            branch.loaded_hour = Some(97);
        }
        for (first_hour, last_hour) in [(98, 99), (102, 103)] {
            records.push(Record::Gap(Gap {
                branch: 1,
                first_hour,
                last_hour,
                reason: "queue full".into(),
            }));
        }
        let replay = Replay::from_records(b"campaign".to_vec(), &[records]).unwrap();
        let mut playback = Playback::new(replay).unwrap();
        assert_eq!(playback.range(1), Some((98, 103)));
        assert_eq!(playback.hour(), 98);
        assert!(playback.frame().is_none());
        assert_eq!(playback.block_loads, 0);
        playback.seek(1, 103).unwrap();
        assert!(playback.frame().is_none());
        assert!(!playback.step().unwrap());
    }

    #[test]
    fn cancellation_walks_do_not_hold_endpoints_and_can_export_the_same_hour_afterwards() {
        let replay = Arc::new(fixture());
        let mut playback = Playback::unloaded(Arc::clone(&replay)).unwrap();
        playback.advance_without_output(1, 104).unwrap();
        assert!(!playback.materialized);
        assert!(matches!(playback.current(), Current::Gap { .. }));
        let before = playback
            .cache
            .as_ref()
            .unwrap()
            .decoder
            .current()
            .unwrap()
            .complete::<tables::Provinces>()
            .unwrap()
            .as_ptr();
        playback.advance_without_output(1, 105).unwrap();
        let after = playback
            .cache
            .as_ref()
            .unwrap()
            .decoder
            .current()
            .unwrap()
            .complete::<tables::Provinces>()
            .unwrap()
            .as_ptr();
        assert_eq!(
            before, after,
            "the private row allocation survives the next native edit"
        );
        assert_seek_matches_shared(&mut playback, 1, 105);
        let held = playback.current().clone();
        for hour in [820, 102, 104, 100] {
            playback.advance_without_output(1, hour).unwrap();
            assert_seek_matches_shared(&mut playback, 1, hour);
        }
        let Current::Frame(held) = held else {
            panic!("held frame")
        };
        assert_eq!(held.as_ref(), &source(1, 105));
    }
}

#[cfg(test)]
mod damaged_recording_tests {
    use super::*;
    use replay::container::SeekResult;

    #[test]
    fn modified_trailer_and_damaged_blocks_open_and_keep_later_playback_exact() {
        let recording = tests::fixture();
        let expected = match recording.seek(1, 820).unwrap() {
            SeekResult::Frame(frame) => frame,
            other => panic!("{other:?}"),
        };
        for corrupt_block in [false, true] {
            let mut bytes = recording.bytes().to_vec();
            *bytes.last_mut().unwrap() ^= 1;
            if corrupt_block {
                let first = &recording.index[0];
                bytes[(first.offset + first.length - 1) as usize] ^= 1;
            }
            let replay = Replay::open(bytes).unwrap();
            assert!(
                replay
                    .warnings
                    .iter()
                    .any(|reason| reason.contains("trailer CRC mismatch"))
            );
            let mut playback = Playback::new(replay).unwrap();
            if corrupt_block {
                assert!(matches!(playback.current(), Current::Gap { .. }));
            }
            playback.seek(1, 820).unwrap();
            assert_eq!(playback.frame().unwrap(), expected.as_ref());
        }
    }
}
