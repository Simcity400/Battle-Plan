//! Exact-hour playback through the shared replay decoder, with one cached block.

use std::{collections::BTreeMap, io, sync::Arc};

use crate::blocks::BlockReader;
use replay::{
    BranchId, Hour, SessionId,
    branches::Parentage,
    container::{Replay, SeekResult},
    frame::{DecodedFrame, Frame, FrameDecoder, TableView},
    rows::{TableId, tables},
};

/// What a playback holds for its hour: the decoded frame, shared so the
/// window can hand it between threads without a copy, or the gap it falls in.
#[derive(Clone, Debug, PartialEq)]
pub enum Current {
    Frame(Arc<DecodedFrame>),
    Gap {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
}

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
    cache: Option<CachedBlock>,
    /// Reads blocks ahead of this playback and frees passed ones elsewhere;
    /// without one, blocks are read and freed on the calling thread.
    reader: Option<Arc<BlockReader>>,
    parent_support: BTreeMap<BranchId, Option<bool>>,
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
            cache: None,
            reader: None,
            parent_support: BTreeMap::new(),
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
        std::mem::replace(&mut self.current, current)
    }

    /// The timeline includes explicitly lost hours at either end of a branch.
    pub fn range(&self, branch: BranchId) -> Option<(Hour, Hour)> {
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
    /// else the hour before the block's keyframe. `None` when no block holds it,
    /// or when the block failed at or before `hour`: that seek is a gap with no
    /// walk at all.
    pub fn walk_start(&self, branch: BranchId, hour: Hour) -> Option<Hour> {
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
        Some(continued.unwrap_or(self.replay.index[index].first_hour - 1))
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
        mut passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<()> {
        if !self.replay.branches.iter().any(|entry| entry.id == branch) {
            return Err(unknown_branch(branch));
        }
        if self.branch == branch && self.hour == hour && self.cache.is_some() {
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
        if cache.applied_hour.is_some_and(|applied| hour <= applied)
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
                self.current = Current::Frame(Arc::new(decoded.clone()));
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

    /// Check a probable parent's supporting hour separately from timeline seeks.
    /// Missing or damaged support does not turn a recorded claim into certainty.
    /// None means parentage is unknown; false means support is unavailable here.
    pub fn parent_support(&mut self, branch: BranchId) -> io::Result<Option<bool>> {
        if let Some(support) = self.parent_support.get(&branch) {
            return Ok(*support);
        }
        let start = self
            .replay
            .branches
            .iter()
            .find(|entry| entry.id == branch)
            .ok_or_else(|| unknown_branch(branch))?;
        let support = match start.parent {
            Parentage::Unknown => None,
            Parentage::Probable { branch, hour } => Some(matches!(
                self.replay.seek(branch, hour),
                Ok(SeekResult::Frame(_))
            )),
        };
        self.parent_support.insert(branch, support);
        Ok(support)
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use replay::{
        branches::{BranchReason, BranchStart, Gap},
        frame::{FrameEncoder, Metrics, TableUpdate},
        identity::{Identity, Roster, SessionStart, Verification},
        rows::{
            AirWingRow, ArmyOrderRow, CampaignRow, ClockRow, CountryRow, DivisionRow,
            FactionMemberRow, FactionRow, FleetRow, HumanControllerRow, LandCombatRow,
            NavalCombatParticipantRow, NavalCombatRow, PlayerRow, ProvinceRow, ShipRow, StateRow,
            TableRows, TaskForceRow, WarRelationRow,
        },
        spool::Record,
    };

    fn session(id: SessionId) -> SessionStart {
        SessionStart {
            id,
            process_id: id as u32,
            process_start_time: id,
            qpc_frequency: 10_000_000,
            sample_boundary: replay::identity::SampleBoundary::EndOfHour,
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
                        border_land_doctrine_level: replay::rows::Recorded::Value(None),
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
                        continent: replay::rows::Recorded::Value(Some(0)),
                        ..Default::default()
                    }])),
                    TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                        id: (1_u64 << 32) | 4713,
                        roster_owner: 1,
                        logical_owner: 1,
                        expeditionary_owner: 0,
                        province: 1,
                        template: (2_u64 << 32) | 52,
                        hit_points: 26_429_000,
                        organisation: 6_000_001,
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
                    }])),
                    TableView::Complete(TableRows::Fleets(vec![FleetRow {
                        id: (1_u64 << 32) | 61,
                        owner: 1,
                        home_base: 1,
                        name: replay::rows::Recorded::Value(Some(Vec::new())),
                        leader: replay::rows::Recorded::Value(Some(0)),
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
                        experience: i64::MIN,
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
                        name: replay::rows::Recorded::Value(Some(Vec::new())),
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
                // including singleton tables, are explicitly unavailable.
                let mut tables = std::array::from_fn(|_| TableView::Unavailable {
                    reason: "outside the map playback fixture".into(),
                });
                for (table, rows) in tables.iter_mut().zip(map_tables) {
                    *table = rows;
                }
                tables.map(Into::into)
            },
        }
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
        let replay = fixture();
        let first = &replay.index[0];
        let mut bytes = replay.bytes().to_vec();
        bytes[(first.offset + first.length - 1) as usize] ^= 0x40;
        let replay = Arc::new(Replay::open(bytes).unwrap());
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
    fn corrupt_blocks_are_cached_and_do_not_hide_the_next_keyframe() {
        let replay = fixture();
        let first = &replay.index[0];
        let mut bytes = replay.bytes().to_vec();
        bytes[(first.offset + first.length - 1) as usize] ^= 0x40;
        let mut playback = Playback::new(Replay::open(bytes).unwrap()).unwrap();
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
    fn parentage_stays_probable_when_its_support_is_missing() {
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
        assert_eq!(playback.parent_support(1).unwrap(), None);
        assert_eq!(playback.parent_support(2).unwrap(), Some(true));
        assert_eq!(playback.parent_support(3).unwrap(), Some(false));
        assert_eq!(playback.parent_support(3).unwrap(), Some(false));
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
}
