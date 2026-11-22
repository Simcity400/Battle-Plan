use std::{io, sync::Arc};

use crate::map_binding::MapBinding;
use replay::campaign::{HourCursor, Replay, SeekResult};
use replay_data::{BranchId, Hour, SessionId, rows::TableId};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use viewer_counter_products::map::province_tooltips::air_region::{RosterHistory, Sampler};

#[derive(Clone, Debug, PartialEq)]
pub enum Current {
    Frame(Arc<DecodedFrame>),
    Gap {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
    Refused {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
}

pub struct AirRosters(RosterHistory);

impl viewer_counters::map::counters::AirPasses for AirRosters {
    fn visited(&self, branch: BranchId, pass: Hour, slot: u32) -> Result<bool, String> {
        self.0.visited(branch, pass, slot)
    }
}

pub struct Playback {
    replay: Arc<Replay>,
    rosters: std::sync::Mutex<Option<(Arc<Sampler>, bool)>>,
    branch: BranchId,
    hour: Hour,
    current: Current,
    materialized: bool,
    cursor: Option<HourCursor>,
    map_binding: Option<Arc<MapBinding>>,
    map: Option<crate::map_source::MapSource>,
    map_failure: Option<String>,
    native_map: bool,
    source_path: Option<bool>,
}

impl Playback {
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(replay: Replay) -> io::Result<Self> {
        Self::shared(Arc::new(replay))
    }

    pub fn shared(replay: Arc<Replay>) -> io::Result<Self> {
        let mut playback = Self::unloaded(replay)?;
        playback.select_branch(playback.branch)?;
        Ok(playback)
    }

    pub fn unloaded(replay: Arc<Replay>) -> io::Result<Self> {
        let main = replay.timelines().map(|timelines| timelines.main);
        let first = replay
            .branches
            .iter()
            .find(|branch| Some(branch.id) == main)
            .or_else(|| replay.branches.first())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "replay contains no branches")
            })?;
        let branch = first.id;
        let hour = first.first_hour;
        Ok(Self {
            replay,
            rosters: std::sync::Mutex::new(None),
            branch,
            hour,
            current: gap(branch, hour, "hour has not been decoded".into()),
            materialized: false,
            cursor: None,
            map_binding: None,
            map: None,
            map_failure: None,
            native_map: false,
            source_path: None,
        })
    }

    pub fn replay(&self) -> &Replay {
        &self.replay
    }

    pub fn bind_map(&mut self, binding: Arc<MapBinding>) {
        debug_assert!(Arc::ptr_eq(&binding.replay_handle(), &self.replay));
        self.map_binding = Some(binding);
        self.map = None;
        self.map_failure = None;
        self.native_map = false;
        let held = match &self.current {
            Current::Frame(frame) if leaves_map_to_native(frame, self.replay.capture_mode) => {
                Some((frame.branch, frame.hour))
            }
            _ => None,
        };
        if let Some((branch, hour)) = held {
            self.current = gap(branch, hour, "hour has not been decoded".into());
            self.materialized = false;
        }
    }

    fn unindexed(&self, branch: BranchId, hour: Hour) -> Current {
        Current::Refused {
            branch,
            hour,
            reason: "this recording's map is held only in its native capture, and no map is                      bound to read it"
                .into(),
        }
    }

    fn refresh_map(&mut self) -> io::Result<()> {
        if self.map.is_some() || self.map_failure.is_some() {
            return Ok(());
        }
        let Some(binding) = &self.map_binding else {
            return Ok(());
        };
        let opened = binding
            .columnar()
            .and_then(|columnar| columnar.open(Arc::clone(&self.replay)));
        match opened {
            Ok(map) => self.map = Some(map),
            Err(error) if interrupted(&error) => return Err(error),
            Err(error) => {
                self.map_failure = Some(format!("this recording's map cannot be read: {error}"))
            }
        }
        Ok(())
    }

    fn note_source(&mut self, branch: BranchId, hour: Hour, indexed: bool) {
        let profiling = viewer_runtime::profile_writer::profiling();
        #[cfg(not(any(test, feature = "test-support")))]
        if !profiling {
            return;
        }
        if self.source_path == Some(indexed) {
            return;
        }
        self.source_path = Some(indexed);
        if profiling {
            let thread = std::thread::current();
            viewer_runtime::profile_writer::profile_print(source_path_line(
                thread.name().unwrap_or("unnamed"),
                branch,
                hour,
                indexed,
            ));
        }
    }

    pub fn replay_handle(&self) -> Arc<Replay> {
        Arc::clone(&self.replay)
    }

    pub fn air_roster_source(&self) -> Option<Arc<replay::columnar_natives::Natives>> {
        self.map.as_ref().map(|map| map.roster_source())
    }

    pub fn air_rosters(&self) -> Arc<AirRosters> {
        Arc::new(AirRosters(RosterHistory::of(self.roster_sampler())))
    }

    pub fn roster_sampler(&self) -> Arc<Sampler> {
        let source = self.air_roster_source();
        let mut held = self
            .rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((sampler, _)) = held.as_ref().filter(|(sampler, lanes)| {
            *lanes || sampler.columnar().map(Arc::as_ptr) == source.as_ref().map(Arc::as_ptr)
        }) {
            return Arc::clone(sampler);
        }
        let sampler = Arc::new(Sampler::today(Arc::clone(&self.replay), source));
        *held = Some((Arc::clone(&sampler), false));
        sampler
    }

    pub fn read_rosters_of(&self, entry: &crate::playhead_host::Entry) {
        if let Some(sampler) = entry.rosters(&self.replay) {
            *self
                .rosters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((sampler, true));
        }
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn frame(&self) -> Option<&DecodedFrame> {
        match &self.current {
            Current::Frame(frame) => Some(frame),
            Current::Gap { .. } | Current::Refused { .. } => None,
        }
    }

    pub fn present(&mut self, branch: BranchId, hour: Hour, current: Current) -> Current {
        self.branch = branch;
        self.hour = hour;
        self.materialized = true;
        std::mem::replace(&mut self.current, current)
    }

    pub fn release_source(&mut self) -> Option<impl Send + 'static + use<>> {
        let released = (self.cursor.take(), self.map.take());
        (released.0.is_some() || released.1.is_some()).then_some(released)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn holds_source(&self) -> bool {
        self.cursor.is_some()
    }

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
        let _cost = replay_frame::cpu::scope("viewer", Some("decode/hour-build"));
        let (first, _) = self.range(branch).ok_or_else(|| unknown_branch(branch))?;
        self.seek(branch, first)
    }

    pub fn seek(&mut self, branch: BranchId, hour: Hour) -> io::Result<()> {
        let _cost = replay_frame::cpu::scope("viewer", Some("decode/hour-build"));
        self.seek_with(branch, hour, |_| {})
    }

    pub fn seek_with(
        &mut self,
        branch: BranchId,
        hour: Hour,
        mut passed: impl FnMut(&DecodedFrame),
    ) -> io::Result<()> {
        self.seek_to(branch, hour, |frame| passed(frame), &|| false)?;
        self.materialized = true;
        Ok(())
    }

    fn seek_to(
        &mut self,
        branch: BranchId,
        hour: Hour,
        mut passed: impl FnMut(&Arc<DecodedFrame>),
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        let mode = self.replay.capture_mode;
        let mut native = false;
        let passed = |frame: &Arc<DecodedFrame>| {
            if leaves_map_to_native(frame, mode) {
                native = true;
            } else {
                passed(frame);
            }
        };
        self.seek_source(branch, hour, passed, cancelled)?;
        let held = native
            || matches!(&self.current, Current::Frame(frame) if leaves_map_to_native(frame, mode));
        if held && self.map.is_none() {
            self.native_map = true;
            self.current = self.unindexed(branch, hour);
            self.cursor = None;
        }
        Ok(())
    }

    fn seek_source(
        &mut self,
        branch: BranchId,
        hour: Hour,
        mut passed: impl FnMut(&Arc<DecodedFrame>),
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if !self.replay.branches.iter().any(|entry| entry.id == branch) {
            return Err(unknown_branch(branch));
        }
        self.refresh_map()?;
        let cached_output =
            self.branch == branch && self.hour == hour && self.cursor.is_some() && self.materialized;
        if !cached_output {
            self.current = gap(branch, hour, "hour has not been decoded".into());
            self.materialized = false;
        }
        if let Some(map) = &mut self.map {
            let current = map.read(branch, hour, true, cancelled)?;
            self.branch = branch;
            self.hour = hour;
            self.current = current;
            self.cursor = None;
            self.note_source(branch, hour, true);
            return Ok(());
        }
        if let Some(reason) = self.map_failure.clone() {
            self.branch = branch;
            self.hour = hour;
            self.current = Current::Refused {
                branch,
                hour,
                reason,
            };
            self.cursor = None;
            return Ok(());
        }
        if self.native_map {
            self.branch = branch;
            self.hour = hour;
            self.current = self.unindexed(branch, hour);
            return Ok(());
        }
        self.note_source(branch, hour, false);
        if cached_output {
            return Ok(());
        }
        self.branch = branch;
        self.hour = hour;
        if self.replay.block_at(branch, hour).is_none() {
            self.current = gap(branch, hour, self.gap_reason(branch, hour));
            return Ok(());
        }
        let replay = &self.replay;
        let cursor = self
            .cursor
            .get_or_insert_with(|| HourCursor::new(Arc::clone(replay), &TableId::ALL, false));
        if let Some((_, last)) = cursor.last().filter(|_| cursor.continues(branch, hour))
            && hour - last <= PASSED_HOURS
        {
            for passing in last + 1..hour {
                if cancelled() {
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                if let SeekResult::Frame(frame) = cursor.seek(branch, passing)? {
                    passed(&Arc::new(*frame));
                }
            }
        }
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.current = match self.cursor.as_mut().expect("a cursor").seek(branch, hour) {
            Ok(SeekResult::Frame(frame)) => Current::Frame(Arc::new(*frame)),
            Ok(SeekResult::Gap { reason, .. }) => gap(branch, hour, reason),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                self.cursor = None;
                Current::Refused {
                    branch,
                    hour,
                    reason: format!("the hour cannot be read: {error}"),
                }
            }
        };
        Ok(())
    }

    fn gap_reason(&self, branch: BranchId, hour: Hour) -> String {
        gap_reason(&self.replay, branch, hour)
    }
}

fn leaves_map_to_native(frame: &DecodedFrame, mode: replay_data::scope::CaptureMode) -> bool {
    dashboard_base::map_tables::selected_tables().iter().any(|table| {
        replay_data::scope::records(mode, *table)
            && matches!(&*frame.tables[table.index()], TableView::NotRecorded { mode: marked } if *marked == mode)
    })
}

pub fn gap_reason(replay: &Replay, branch: BranchId, hour: Hour) -> String {
    let reason = replay
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

const PASSED_HOURS: Hour = 48;

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

fn source_path_line(thread: &str, branch: BranchId, hour: Hour, native: bool) -> String {
    format!(
        "PROFILE source_path thread={} branch={branch} hour={hour} path={}",
        thread.replace(char::is_whitespace, "_"),
        if native { "native" } else { "ordinary" },
    )
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[test]
    fn source_path_line_names_the_thread_and_source() {
        assert_eq!(
            source_path_line("map source", 1, 101, true),
            "PROFILE source_path thread=map_source branch=1 hour=101 path=native"
        );
        assert_eq!(
            source_path_line("map source decoder", 7, -3, false),
            "PROFILE source_path thread=map_source_decoder branch=7 hour=-3 path=ordinary"
        );
    }

    use super::*;
    use replay::{campaign::SeekResult, spool::Record};
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
        frame::FrameEncoder, frame::Metrics,
    };

    pub fn session(id: SessionId) -> SessionStart {
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

    pub fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
        replay_frame::frame::retire_absent(DecodedFrame {
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
                        hit_points: 26_420_000,
                        organisation: 6_000_000,
                        orders_group: (57_u64 << 32) | 53,
                        reveal_skip: 0,
                        theatre: Some(0).into(),
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
                        ordinal: 0.into(),
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
                for rows in resource_map_rows(hour) {
                    let table = rows.table();
                    tables[table.index()] = TableView::Complete(rows);
                }
                tables.map(Into::into)
            },
        })
    }

    pub fn resource_map_rows(hour: Hour) -> [TableRows; 8] {
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

    pub fn records(id: u64, hours: &[Hour]) -> Vec<Record> {
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

    pub fn fixture() -> Replay {
        written_fixture().0
    }

    fn written_fixture() -> (Replay, Vec<DecodedFrame>) {
        let mut records = records(1, &[100, 101, 104, 105, 820]);
        records.push(Record::Gap(Gap {
            branch: 1,
            first_hour: 102,
            last_hour: 104,
            reason: "flush failed".into(),
        }));
        Replay::from_records_derived(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap()
    }

    fn read_back(derived: &[DecodedFrame], branch: BranchId, hour: Hour) -> DecodedFrame {
        let held = derived
            .iter()
            .find(|frame| (frame.branch, frame.hour) == (branch, hour))
            .expect("the fixture writes the hour");
        let mut frame = source(branch, hour);
        for (index, view) in held.tables.iter().enumerate() {
            if !view.is_not_selected() {
                frame.tables[index] = view.clone();
                frame.metrics.table_ns[index] = held.metrics.table_ns[index];
            }
        }
        frame
    }

    fn corrupt_first_block(replay: &Replay) -> Replay {
        Replay::open(replay.damaged_block(&replay.index[0]).unwrap()).unwrap()
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
    fn seeks_match_the_shared_reader_through_holes() {
        let mut playback = Playback::new(fixture()).unwrap();
        for hour in [100, 101, 102, 103, 104, 105, 105, 100, 104, 103, 101] {
            assert_seek_matches_shared(&mut playback, 1, hour);
        }
        assert_seek_matches_shared(&mut playback, 1, 819);
        assert_seek_matches_shared(&mut playback, 1, 820);
        assert_eq!(playback.hour(), 820);
        assert_seek_matches_shared(&mut playback, 1, 105);
        assert!(playback.seek(999, 100).is_err());
        assert_eq!(
            (playback.branch(), playback.session(), playback.hour()),
            (1, 1, 105)
        );
    }

    #[test]
    fn seeks_show_table_failures_and_gaps_without_interpolation() {
        let (replay, derived) = written_fixture();
        let mut playback = Playback::new(replay).unwrap();
        playback.seek(1, 101).unwrap();
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
            playback.seek(1, hour).unwrap();
            assert_eq!(playback.hour(), hour);
            assert!(playback.frame().is_none());
            assert!(matches!(
                playback.current(),
                Current::Gap { reason, .. } if reason == "flush failed"
            ));
        }
        playback.seek(1, 104).unwrap();
        assert_eq!(playback.frame(), Some(&read_back(&derived, 1, 104)));
    }

    fn assert_unreadable(playback: &mut Playback, hour: Hour) {
        playback.seek(1, hour).unwrap();
        let error = playback.replay().seek(1, hour).unwrap_err().to_string();
        assert!(
            matches!(
                playback.current(),
                Current::Refused { branch: 1, hour: at, reason } if *at == hour && reason.contains(&error)
            ),
            "hour {hour}: {:?} does not carry {error}",
            playback.current()
        );
    }

    #[test]
    fn a_corrupt_block_is_its_hours_failure_and_does_not_hide_the_next_block() {
        let mut playback = Playback::new(corrupt_first_block(&fixture())).unwrap();
        for hour in [100, 101, 100] {
            assert_unreadable(&mut playback, hour);
        }
        for hour in [105, 820] {
            assert_seek_matches_shared(&mut playback, 1, hour);
            assert!(playback.frame().is_some());
        }
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
        let replay = Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[first, second, third]).unwrap();
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
    fn read_as_timelines_the_main_history_plays_through_each_reload_and_keeps_what_it_left() {
        let segments = [
            records(1, &[100, 101, 102, 103]),
            records(2, &[102, 103, 104]),
            records(3, &[105, 106]),
        ];
        let replay = Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &segments)
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let timelines = replay.timelines().unwrap();
        assert_eq!(timelines.main, 2);
        let pieces: Vec<_> = replay
            .pieces(2)
            .iter()
            .map(|piece| (piece.segment, piece.first_hour, piece.last_hour))
            .collect();
        assert_eq!(pieces, [(1, 100, 101), (2, 102, 104), (3, 105, 106)]);
        assert_eq!(replay.pieces(1).len(), 1);
        assert_eq!(
            timelines.parting(1),
            Some(replay::campaign::timelines::Parting::LeftBehind(102))
        );
        let mut playback = Playback::new(replay).unwrap();
        assert_eq!((playback.branch(), playback.hour()), (2, 100));
        assert_eq!(playback.range(2), Some((100, 106)));
        for hour in [100, 101, 102, 105, 106] {
            assert_seek_matches_shared(&mut playback, 2, hour);
            let Some(frame) = playback.frame() else {
                panic!("hour {hour} of the main history has a frame");
            };
            assert_eq!((frame.branch, frame.hour), (2, hour));
        }
        assert_seek_matches_shared(&mut playback, 1, 103);
        assert!(playback.frame().is_some());
        assert_eq!(playback.range(1), Some((100, 103)));
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
        let replay = Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap();
        let mut playback = Playback::new(replay).unwrap();
        assert_eq!(playback.range(1), Some((98, 103)));
        assert_eq!(playback.hour(), 98);
        assert!(playback.frame().is_none());
        playback.seek(1, 103).unwrap();
        assert!(playback.frame().is_none());
    }

    #[test]
    fn releasing_the_source_keeps_the_presented_hour_and_later_seeks_read_again() {
        let mut playback = Playback::new(fixture()).unwrap();
        playback.seek(1, 105).unwrap();
        let Current::Frame(shown) = playback.current().clone() else {
            panic!("hour 105 is recorded");
        };
        assert!(playback.holds_source());
        assert!(playback.release_source().is_some());
        assert!(!playback.holds_source());
        assert!(
            playback.release_source().is_none(),
            "nothing is left to release"
        );
        assert!(matches!(
            playback.current(),
            Current::Frame(presented) if Arc::ptr_eq(presented, &shown)
        ));
        assert_eq!((playback.branch(), playback.hour()), (1, 105));
        playback.seek(1, 105).unwrap();
        assert_eq!(playback.frame(), Some(&*shown));
        for hour in [100, 101, 102, 104, 820, 105] {
            assert_seek_matches_shared(&mut playback, 1, hour);
        }
    }

    #[test]
    fn an_unreadable_block_is_read_again_to_the_same_failure() {
        let mut playback = Playback::new(corrupt_first_block(&fixture())).unwrap();
        let unreadable = playback.current().clone();
        assert!(
            matches!(&unreadable, Current::Refused { .. }),
            "{unreadable:?}"
        );
        assert!(!playback.holds_source(), "a failed read is let go");
        playback.seek(1, 100).unwrap();
        assert_eq!(playback.current(), &unreadable);
        for hour in [101, 100] {
            assert_unreadable(&mut playback, hour);
        }
    }

    #[test]
    fn every_reader_opened_hands_out_its_playheads_one_lanes_roster_sampler() {
        use crate::layers::sounds::tests::{quick, record_runs, units};
        let replay = Arc::new(Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records(1, &[100, 101, 102])]).unwrap());
        let entry = crate::playhead_host::open(units(record_runs(&[(100, 3)], &[], |_, _| {}), "every reader hands out the lanes rosters"), quick()).unwrap();
        let lanes = entry.rosters(&replay).expect("the recording holds a block at the playhead's first hour");
        let campaign = Playback::unloaded(Arc::clone(&replay)).unwrap();
        let dashboard = Playback::unloaded(Arc::clone(&replay)).unwrap();
        for (name, playback) in [("the campaign map", &campaign), ("a dashboard map", &dashboard)] {
            assert!(!Arc::ptr_eq(&playback.roster_sampler(), &lanes), "{name} reads today's before a reader opens");
            playback.read_rosters_of(&entry);
            assert!(Arc::ptr_eq(&playback.roster_sampler(), &lanes), "{name} reads the lanes once its reader opened");
        }
        campaign.read_rosters_of(&entry);
        assert!(Arc::ptr_eq(&campaign.roster_sampler(), &lanes), "a reopened reader keeps the one sampler");
        assert!(Arc::ptr_eq(&entry.rosters(&replay).unwrap(), &lanes), "one sampler a recording and timeline");
    }
}

#[cfg(test)]
mod damaged_recording_tests {
    use super::*;
    use replay::campaign::SeekResult;

    #[test]
    fn a_damaged_block_opens_and_keeps_later_playback_exact() {
        let recording = tests::fixture();
        let expected = match recording.seek(1, 820).unwrap() {
            SeekResult::Frame(frame) => frame,
            other => panic!("{other:?}"),
        };
        let replay = Replay::open(recording.damaged_block(&recording.index[0]).unwrap()).unwrap();
        let mut playback = Playback::new(replay).unwrap();
        assert!(
            matches!(playback.current(), Current::Refused { .. }),
            "{:?}",
            playback.current()
        );
        playback.seek(1, 820).unwrap();
        assert_eq!(playback.frame().unwrap(), expected.as_ref());
    }
}

fn interrupted(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted
}
