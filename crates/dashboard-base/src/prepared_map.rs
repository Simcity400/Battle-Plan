//! Selected map checkpoints prepared alongside dashboard products.
//!
//! Checkpoint headers are binary-searched on disk; only one selected decoded
//! hour and one selected source block remain in memory. Hourly updates are read
//! from the recording; prepared storage contains only sparse native checkpoints.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Seek, SeekFrom},
    num::NonZeroU32,
    sync::Arc,
};

use replay::{
    BranchId, Hour,
    codec::Decoder,
    frame::{DecodedFrame, Frame, FrameDecoder, TableView},
    rows::TableId,
};
use store::{SourceKey, Store, Writer};

mod reader;
pub use reader::Reader;

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

const MAGIC: &[u8; 8] = b"HOIMAP04";
/// Production seek bound in recorded updates, independent of source blocks.
pub const CHECKPOINT_STRIDE: u32 = 720;
const GAP_MAGIC: &[u8; 8] = b"HOIGAP01";
const GAP_RECORD_BYTES: u64 = 24;

/// Native map inputs plus the existing authored-trigger operand kit.
/// Both the producer and the viewer use this one selection.
pub fn selected_tables() -> &'static [TableId] {
    static SELECTED: std::sync::LazyLock<Vec<TableId>> = std::sync::LazyLock::new(|| {
        const NATIVE: &[TableId] = &[
            TableId::AceRangeInputs,
            TableId::AgencyOperationBuffers,
            TableId::ActiveAbilities,
            TableId::AirBaseRegistrations,
            TableId::AirBases,
            TableId::AirCountryRelations,
            TableId::AirMissionInputs,
            TableId::AirNavalPatrolCombats,
            TableId::AirNavalPatrolHistories,
            TableId::AirReferenceWitnesses,
            TableId::AirRegionCombat,
            TableId::AirRegionalMissions,
            TableId::AirSortieBaseCountries,
            TableId::AirSortieModifierMetadata,
            TableId::AirSortieModifiers,
            TableId::AirSortieSpecialBases,
            TableId::AirWings,
            TableId::ArmyGroupMembers,
            TableId::ArmyIconInputs,
            TableId::AvailableEquipmentVariants,
            TableId::BattleSounds,
            TableId::BorderConflicts,
            TableId::Campaign,
            TableId::CharacterPortraits,
            TableId::Clock,
            TableId::CountryCommandPower,
            TableId::CountryIntel,
            TableId::CountryLocalSubunits,
            TableId::CountryStabilityInputs,
            TableId::CountryTacticUnlocks,
            TableId::CountryVisibility,
            TableId::Cryptology,
            TableId::DivisionEquipment,
            TableId::DivisionMovement,
            TableId::DivisionRetreatBans,
            TableId::Divisions,
            TableId::FacilityMapFlags,
            TableId::FactionTheatres,
            TableId::FleetCounterInputs,
            TableId::Fleets,
            TableId::FloatingHarborDeployments,
            TableId::Fronts,
            TableId::HumanControllers,
            TableId::IntelNetworks,
            TableId::LandCombatDetails,
            TableId::LandCombatDivisionModifiers,
            TableId::LandCombatDivisions,
            TableId::LandCombatModifiers,
            TableId::LandCombatSides,
            TableId::LandCombats,
            TableId::LeaderPortraitPaths,
            TableId::LeaderPortraits,
            TableId::MapEvents,
            TableId::NavalBases,
            TableId::NavalCombatAirBaseWingHandles,
            TableId::NavalCombatDetails,
            TableId::NavalCombatParticipants,
            TableId::NavalCombatResultDetails,
            TableId::NavalCombatShips,
            TableId::NavalCombatTransfers,
            TableId::NavalCombats,
            TableId::NavalMineLevels,
            TableId::NavalMissionCommanders,
            TableId::NavalMissionCountries,
            TableId::NavalMissionLedgers,
            TableId::NavalMissionTaskForces,
            TableId::NavalModifierTrees,
            TableId::NavalProvinceCombats,
            TableId::NavalRegionAccess,
            TableId::NavalResultCountries,
            TableId::NavalResultMembers,
            TableId::NavalResults,
            TableId::NavalSupremacy,
            TableId::NavalTransferUnits,
            TableId::NewDivisionEquipment,
            TableId::NuclearStrikes,
            TableId::OrdersGroupCounterInputs,
            TableId::OrdersGroupMembers,
            TableId::OperativeMissions,
            TableId::Operations,
            TableId::Players,
            TableId::ProvinceSupply,
            TableId::ProvinceUnitOrder,
            TableId::RadarCoverage,
            TableId::RaidAirScores,
            TableId::RaidAirSites,
            TableId::RaidCooldowns,
            TableId::RaidCountries,
            TableId::RaidCountryInputs,
            TableId::RaidEquipment,
            TableId::RaidFacilities,
            TableId::RaidIntelInputs,
            TableId::RaidPathDistances,
            TableId::RaidPlayerAllowedTypes,
            TableId::RaidSpeedInputs,
            TableId::RaidTargetStatuses,
            TableId::RaidTargets,
            TableId::RailCooldowns,
            TableId::RailEdges,
            TableId::RailwayGunCounterInputs,
            TableId::RelationLinks,
            TableId::RelationTuples,
            TableId::SentVolunteers,
            TableId::ShipModels,
            TableId::Ships,
            TableId::SliceReads,
            TableId::StateFlags,
            TableId::StateVisibility,
            TableId::StationedUnits,
            TableId::StockpileArchetypes,
            TableId::Stockpiles,
            TableId::StrategicOperatives,
            TableId::SupplyNodes,
            TableId::TaskForceCounterInputs,
            TableId::TaskForceMapMotion,
            TableId::TaskForceMissions,
            TableId::TaskForces,
            TableId::TheatreAreas,
            TableId::TrainInventory,
            TableId::UnitIntel,
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantStats,
            TableId::VariantUpgrades,
            TableId::VolunteerHosts,
            TableId::WeatherProvinces,
            TableId::WeatherRegions,
            TableId::WingEquipment,
            // The map modes' layers (viewer_world::map::map_mode); Capitulation holds each
            // country's owned war goals.
            TableId::Capitulation,
            TableId::FactionMembers,
            TableId::Factions,
            TableId::PoliticalParties,
            TableId::Politics,
            TableId::RelationOpinions,
            TableId::SimulationFeatures,
            TableId::StateResistance,
            TableId::Subjects,
            // The resources map mode's origins, icons and hover
            // (viewer_foundation::resource_origins) and the trade routes and their convoys
            // (viewer_world::map::trade_routes).
            TableId::ConvoyClients,
            TableId::CountryOilFactors,
            TableId::RegionalConvoyEfficiency,
            TableId::ResourceExchanges,
            TableId::ResourceOrigins,
            TableId::ResourceRights,
            TableId::StateTemporaryResources,
            TableId::TradeRoutes,
        ];
        TableId::ALL
            .into_iter()
            .filter(|table| {
                NATIVE.contains(table) || crate::triggers::RECORDED_TABLES.contains(table)
            })
            .collect()
    });
    &SELECTED
}

/// Ordinals following an accepted decoded hour in its source block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub block: usize,
    pub next_frame: usize,
}

/// Small preparation accumulator. The host persists this alongside its source
/// cursor, and publishes it only with the matching store manifest.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    pub stride: u32,
    pub selected: Vec<u16>,
    pub branches: BTreeMap<BranchId, BranchProgress>,
    pub resume_range: Option<u64>,
    pub gaps: BTreeMap<BranchId, GapPage>,
    pub ranges: BTreeMap<BranchId, (Hour, Hour)>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct BranchProgress {
    pub checkpoints: u64,
    pub since_checkpoint: u32,
    pub last_hour: Option<Hour>,
    /// Last accepted source cursor, used by the pipeline's coverage publication.
    pub last_block: Option<usize>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GapPage {
    pub range: u64,
    pub intervals: u64,
}

pub struct Builder {
    stride: NonZeroU32,
    selected: Vec<TableId>,
    progress: Progress,
}

impl Builder {
    pub fn new(
        stride: NonZeroU32,
        selected: &[TableId],
        mut progress: Progress,
    ) -> io::Result<Self> {
        validate_selection(selected)?;
        let mut selection: Vec<u16> = selected.iter().map(|table| *table as u16).collect();
        selection.sort_unstable();
        if !progress.selected.is_empty() && progress.selected != selection {
            return Err(invalid(
                "map checkpoint table selection differs from accepted progress",
            ));
        }
        progress.selected = selection;
        if progress.stride != 0 && progress.stride != stride.get() {
            return Err(invalid(
                "map checkpoint stride differs from accepted progress",
            ));
        }
        if progress
            .branches
            .values()
            .any(|branch| branch.since_checkpoint >= stride.get())
        {
            return Err(invalid("map checkpoint distance exceeds stride"));
        }
        progress.stride = stride.get();
        Ok(Self {
            stride,
            selected: selected.to_vec(),
            progress,
        })
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Full source timeline bounds, including an unprepared tail. Only source
    /// metadata is visited; hourly table decoding remains the shared pipeline's.
    pub fn set_ranges(&mut self, replay: &replay::container::Replay) {
        let mut ranges: BTreeMap<_, _> = replay
            .branches
            .iter()
            .map(|branch| (branch.id, (branch.first_hour, branch.first_hour)))
            .collect();
        for index in &replay.index {
            if let Some((first, last)) = ranges.get_mut(&index.branch) {
                *first = (*first).min(index.first_hour);
                *last = (*last).max(index.last_hour);
            }
        }
        for gap in &replay.gaps {
            if let Some((first, last)) = ranges.get_mut(&gap.branch) {
                *first = (*first).min(gap.first_hour);
                *last = (*last).max(gap.last_hour);
            }
        }
        self.progress.ranges = ranges;
    }

    /// Called once for every accepted hour. A checkpoint every `stride` frames
    /// bounds replay work by recorded frames, including sparse clock histories.
    /// The caller writes `save_resume` before committing accepted progress.
    pub fn accept(
        &mut self,
        writer: &mut Writer,
        source: &Frame,
        frame: &DecodedFrame,
        position: Position,
    ) -> io::Result<()> {
        if source.branch != frame.branch || source.hour != frame.hour {
            return Err(invalid("map update differs from its accepted decoded hour"));
        }
        let mut branch = self
            .progress
            .branches
            .get(&frame.branch)
            .cloned()
            .unwrap_or_default();
        if branch.last_hour.is_some_and(|last| last >= frame.hour) {
            return Err(invalid("map preparation hour did not advance"));
        }
        let checkpoint =
            branch.checkpoints == 0 || branch.since_checkpoint >= self.stride.get() - 1;
        if checkpoint {
            let bytes = encode_checkpoint(frame, position, &self.selected)?;
            let range = writer.write(&section(frame.branch), bytes.as_slice())?;
            if range != branch.checkpoints {
                return Err(invalid("map checkpoint ordinal differs from progress"));
            }
            branch.checkpoints += 1;
            branch.since_checkpoint = 0;
        } else {
            branch.since_checkpoint += 1;
        }
        branch.last_block = Some(position.block);
        branch.last_hour = Some(frame.hour);
        self.progress.branches.insert(frame.branch, branch);
        Ok(())
    }

    /// Accept an hour whose block the caller already admitted, reading map
    /// rows only when this hour writes a checkpoint. A checkpoint on the
    /// block's first frame reads its keyframe sections alone; any other
    /// reads and reconstructs the block's prefix through that exact hour.
    /// Progress between checkpoints needs only the accepted hour and cursor.
    pub fn accept_sparse(
        &mut self,
        writer: &mut Writer,
        replay: &replay::container::Replay,
        source: &Frame,
        frame: &DecodedFrame,
        position: Position,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if source.branch != frame.branch || source.hour != frame.hour {
            return Err(invalid("map update differs from its accepted decoded hour"));
        }
        let checkpoint = self
            .progress
            .branches
            .get(&frame.branch)
            .is_none_or(|branch| {
                branch.checkpoints == 0 || branch.since_checkpoint >= self.stride.get() - 1
            });
        if !checkpoint {
            return self.accept(writer, source, frame, position);
        }
        check()?;
        let index = replay
            .index
            .get(position.block)
            .filter(|index| {
                index.branch == frame.branch
                    && index.first_hour <= frame.hour
                    && frame.hour <= index.last_hour
            })
            .ok_or_else(|| invalid("map checkpoint cursor differs from its accepted hour"))?;
        let frames = if frame.hour == index.first_hour {
            vec![replay.read_keyframe_selected(index, &self.selected)?]
        } else {
            replay.read_block_selected_through(index, Some(&self.selected), frame.hour, None)?
        };
        if frames.len() != position.next_frame {
            return Err(invalid(
                "map checkpoint cursor differs from its accepted hour",
            ));
        }
        let mut decoder = FrameDecoder::for_tables(&self.selected);
        for update in frames {
            check()?;
            let decoded = decoder.advance(&update)?;
            crate::metadata::validate_campaign(decoded, replay)?;
        }
        let decoded = decoder
            .current()
            .filter(|decoded| decoded.branch == frame.branch && decoded.hour == frame.hour)
            .ok_or_else(|| invalid("map checkpoint frame is missing"))?;
        self.accept(writer, source, decoded, position)
    }

    /// Retains precisely the accepted current state, including table freshness,
    /// partial censuses and refusals. Appended preparation starts after it.
    pub fn save_resume(
        &mut self,
        writer: &mut Writer,
        frame: &DecodedFrame,
        position: Position,
        selected: &[TableId],
    ) -> io::Result<()> {
        if self
            .progress
            .branches
            .get(&frame.branch)
            .and_then(|p| p.last_hour)
            != Some(frame.hour)
        {
            return Err(invalid(
                "map resume state is not the accepted preparation hour",
            ));
        }
        validate_selection(selected)?;
        let bytes = encode_checkpoint(frame, position, selected)?;
        writer.write_at("map.resume", 0, bytes.as_slice())?;
        self.progress.resume_range = Some(0);
        Ok(())
    }

    /// Prepare explicit gap metadata separately from hourly decoding. Only
    /// callers observing changed source gap metadata need to publish this page.
    /// Overlaps retain every reason in the source's original occurrence order.
    pub fn save_gaps(
        &mut self,
        writer: &mut Writer,
        gaps: &[replay::branches::Gap],
    ) -> io::Result<()> {
        let mut branches = BTreeMap::<BranchId, Vec<&replay::branches::Gap>>::new();
        for gap in gaps {
            branches.entry(gap.branch).or_default().push(gap);
        }
        let mut pages = BTreeMap::new();
        for (branch, gaps) in branches {
            let intervals = gap_intervals(&gaps)?;
            let mut bytes = Vec::new();
            bytes.extend_from_slice(GAP_MAGIC);
            bytes.extend_from_slice(&(intervals.len() as u64).to_le_bytes());
            let mut offset = 16u64 + intervals.len() as u64 * GAP_RECORD_BYTES;
            for (first, last, reason) in &intervals {
                bytes.extend_from_slice(&first.to_le_bytes());
                bytes.extend_from_slice(&last.to_le_bytes());
                bytes.extend_from_slice(&offset.to_le_bytes());
                bytes.extend_from_slice(&(reason.len() as u64).to_le_bytes());
                offset += reason.len() as u64;
            }
            for (_, _, reason) in &intervals {
                bytes.extend_from_slice(reason.as_bytes());
            }
            writer.write_at(&format!("map.gaps.{branch}"), 0, bytes.as_slice())?;
            pages.insert(
                branch,
                GapPage {
                    range: 0,
                    intervals: intervals.len() as u64,
                },
            );
        }
        self.progress.gaps = pages;
        Ok(())
    }
}

fn gap_intervals(gaps: &[&replay::branches::Gap]) -> io::Result<Vec<(Hour, Hour, String)>> {
    let mut edges = BTreeMap::<i64, Vec<(usize, bool)>>::new();
    for (ordinal, gap) in gaps.iter().enumerate() {
        if gap.first_hour > gap.last_hour {
            return Err(invalid("map gap interval is reversed"));
        }
        edges
            .entry(i64::from(gap.first_hour))
            .or_default()
            .push((ordinal, true));
        edges
            .entry(i64::from(gap.last_hour) + 1)
            .or_default()
            .push((ordinal, false));
    }
    let mut active = BTreeSet::<usize>::new();
    let mut previous = None;
    let mut intervals = Vec::new();
    for (edge, changes) in edges {
        if let Some(first) = previous
            && first < edge
            && !active.is_empty()
        {
            let reason = active
                .iter()
                .map(|&ordinal| gaps[ordinal].reason.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            intervals.push((first as Hour, (edge - 1) as Hour, reason));
        }
        for (ordinal, start) in changes {
            if start {
                active.insert(ordinal);
            } else {
                active.remove(&ordinal);
            }
        }
        previous = Some(edge);
    }
    Ok(intervals)
}

pub fn load_resume(
    store: &Store,
    key: &SourceKey,
    progress: &Progress,
    selected: &[TableId],
) -> io::Result<Option<(DecodedFrame, Position)>> {
    let Some(range) = progress.resume_range else {
        return Ok(None);
    };
    let file = store
        .read(key, "map.resume", range)?
        .ok_or_else(|| invalid("map resume page is missing"))?;
    resume_from(file, progress, selected)
}

/// Restore accepted map state after the caller authorizes recording growth
/// using input stamps and the block-index/semantic metadata prefix policy.
/// This reader does not reread or hash old compressed blocks.
pub fn load_resume_prefix(
    store: &Store,
    previous: &SourceKey,
    current: &SourceKey,
    progress: &Progress,
    selected: &[TableId],
) -> io::Result<Option<(DecodedFrame, Position)>> {
    let Some(range) = progress.resume_range else {
        return Ok(None);
    };
    let file = store
        .read_prefix(previous, current, "map.resume", range)?
        .ok_or_else(|| invalid("map resume prefix page is missing"))?;
    resume_from(file, progress, selected)
}

fn resume_from(
    file: impl Read,
    progress: &Progress,
    selected: &[TableId],
) -> io::Result<Option<(DecodedFrame, Position)>> {
    let checkpoint = decode_checkpoint(file, selected)?;
    if progress
        .branches
        .get(&checkpoint.frame.branch)
        .and_then(|p| p.last_hour)
        != Some(checkpoint.frame.hour)
    {
        return Err(invalid("map resume page differs from accepted progress"));
    }
    Ok(Some((checkpoint.frame, checkpoint.position)))
}

pub struct Checkpoint {
    pub frame: DecodedFrame,
    pub position: Position,
    selected: [bool; TableId::ALL.len()],
}

impl Checkpoint {
    /// Whether the producer selected a table, independently of whether its
    /// recorded read was complete, truncated or unavailable.
    pub fn selects(&self, table: TableId) -> bool {
        self.selected[table.index()]
    }
}

pub fn section(branch: BranchId) -> String {
    format!("map.checkpoints.{branch}")
}

fn compress(payload: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    let mut compressor = zstd::bulk::Compressor::new(9)?;
    compressor.set_parameter(zstd::zstd_safe::CParameter::ChecksumFlag(true))?;
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&compressor.compress(payload)?);
    Ok(())
}

fn inflate(mut file: impl Read) -> io::Result<Vec<u8>> {
    let mut length = [0; 8];
    file.read_exact(&mut length)?;
    let length = usize::try_from(u64::from_le_bytes(length))
        .map_err(|_| invalid("map page exceeds address space"))?;
    let mut compressed = Vec::new();
    file.read_to_end(&mut compressed)?;
    if zstd::zstd_safe::find_frame_compressed_size(&compressed)
        .map_err(|_| invalid("invalid compressed map page"))?
        != compressed.len()
    {
        return Err(invalid("map page contains trailing compressed bytes"));
    }
    let mut bytes = engine::allocation::vec(length)?;
    bytes.resize(length, 0);
    if zstd::bulk::decompress_to_buffer(&compressed, &mut bytes)? != length {
        return Err(invalid("map page decoded length differs"));
    }
    Ok(bytes)
}

pub fn validate_selection(selected: &[TableId]) -> io::Result<()> {
    if !selected.contains(&TableId::Campaign) {
        return Err(invalid("map checkpoints require Campaign identity"));
    }
    let mut sorted = selected.to_vec();
    sorted.sort_by_key(|table| table.index());
    if sorted.windows(2).any(|p| p[0] == p[1]) {
        return Err(invalid("repeated selected map table"));
    }
    Ok(())
}

fn encode_checkpoint(
    frame: &DecodedFrame,
    position: Position,
    selected: &[TableId],
) -> io::Result<Vec<u8>> {
    let packet = DecodedFrame {
        branch: frame.branch,
        hour: frame.hour,
        metrics: frame.metrics.clone(),
        tables: std::array::from_fn(|index| {
            if selected.contains(&TableId::ALL[index]) {
                frame.tables[index].clone()
            } else {
                TableView::Unavailable {
                    reason: "Table not selected for reconstruction".into(),
                }
                .into()
            }
        }),
    };
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&frame.branch.to_le_bytes());
    out.extend_from_slice(&frame.hour.to_le_bytes());
    out.extend_from_slice(&(position.block as u64).to_le_bytes());
    out.extend_from_slice(&(position.next_frame as u64).to_le_bytes());
    out.extend_from_slice(&replay::FORMAT_VERSION.to_le_bytes());
    for table in TableId::ALL {
        out.push(u8::from(selected.contains(&table)));
    }
    let mut payload = Vec::new();
    packet.encode_checkpoint(&mut payload)?;
    compress(&payload, &mut out)?;
    Ok(out)
}

pub fn header(mut input: impl Read) -> io::Result<(BranchId, Hour)> {
    let mut bytes = [0u8; 20];
    input.read_exact(&mut bytes)?;
    if &bytes[..8] != MAGIC {
        return Err(invalid("unsupported map checkpoint page"));
    }
    Ok((
        u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    ))
}

pub fn decode_checkpoint(file: impl Read, selected: &[TableId]) -> io::Result<Checkpoint> {
    decode_selected_checkpoint(file, Some(selected))
}

/// Read a shared resume page with its stored selection. Consumers of a
/// family's baseline check that the page contains their inputs and hour;
/// the walk itself restores with its exact selection through `load_resume`.
pub fn decode_resume(file: impl Read) -> io::Result<Checkpoint> {
    decode_selected_checkpoint(file, None)
}

fn decode_selected_checkpoint(
    mut file: impl Read,
    expected: Option<&[TableId]>,
) -> io::Result<Checkpoint> {
    let mut bytes = vec![0; 38 + TableId::ALL.len()];
    file.read_exact(&mut bytes)?;
    bytes.extend_from_slice(&inflate(file)?);
    let mut input = Decoder::new(&bytes);
    if input.take(8)? != MAGIC {
        return Err(invalid("unsupported map checkpoint page"));
    }
    let branch = input.u64()?;
    let hour = input.i32()?;
    let position = Position {
        block: usize::try_from(input.u64()?)
            .map_err(|_| invalid("map block ordinal exceeds address space"))?,
        next_frame: usize::try_from(input.u64()?)
            .map_err(|_| invalid("map frame ordinal exceeds address space"))?,
    };
    if input.u16()? != replay::FORMAT_VERSION {
        return Err(invalid("map checkpoint row contract differs"));
    }
    let mut selected = [false; TableId::ALL.len()];
    for table in TableId::ALL {
        let present = input.bool()?;
        if expected.is_some_and(|expected| present != expected.contains(&table)) {
            return Err(invalid("map checkpoint table selection differs"));
        }
        selected[table.index()] = present;
    }
    let frame = DecodedFrame::decode_checkpoint(&mut input)?;
    input.finish()?;
    if frame.branch != branch || frame.hour != hour {
        return Err(invalid("map checkpoint identity differs from its header"));
    }
    for table in TableId::ALL {
        if !selected[table.index()]
            && (!matches!(&*frame.tables[table.index()], TableView::Unavailable { reason } if reason == "Table not selected for reconstruction")
                || frame.tables[table.index()].sampled_at().is_some())
        {
            return Err(invalid("map checkpoint contains an unselected table"));
        }
    }
    // The native checkpoint decoder has validated typed rows, singleton and
    // partial-census laws and supplying hours. Resume will consume this state.
    Ok(Checkpoint {
        frame,
        position,
        selected,
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn gap_reason(
    store: &Store,
    key: &SourceKey,
    progress: &Progress,
    branch: BranchId,
    hour: Hour,
) -> io::Result<String> {
    let mut reason = String::new();
    if let Some(page) = progress.gaps.get(&branch) {
        let mut file = store
            .read(key, &format!("map.gaps.{branch}"), page.range)?
            .ok_or_else(|| invalid("map gap page is missing"))?;
        let mut header = [0; 16];
        file.read_exact(&mut header)?;
        if &header[..8] != GAP_MAGIC
            || u64::from_le_bytes(header[8..].try_into().unwrap()) != page.intervals
        {
            return Err(invalid("map gap page differs from progress"));
        }
        let mut low = 0;
        let mut high = page.intervals;
        while low < high {
            let mid = low + (high - low) / 2;
            let (first, _, _, _) = gap_record(&mut file, mid)?;
            if first <= hour {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        if let Some(range) = low.checked_sub(1) {
            let (_, last, offset, len) = gap_record(&mut file, range)?;
            if hour <= last {
                let mut bytes = vec![
                    0;
                    usize::try_from(len).map_err(|_| invalid(
                        "map gap reason exceeds address space"
                    ))?
                ];
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(&mut bytes)?;
                reason =
                    String::from_utf8(bytes).map_err(|_| invalid("map gap reason is not UTF-8"))?;
            }
        }
    }
    Ok(if reason.is_empty() {
        "hour was not recorded".into()
    } else {
        reason
    })
}
fn gap_record(file: &mut store::Range, ordinal: u64) -> io::Result<(Hour, Hour, u64, u64)> {
    let mut bytes = [0; GAP_RECORD_BYTES as usize];
    let offset = ordinal
        .checked_mul(GAP_RECORD_BYTES)
        .and_then(|bytes| bytes.checked_add(16))
        .ok_or_else(|| invalid("map gap index overflow"))?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut bytes)?;
    let first = i32::from_le_bytes(bytes[..4].try_into().unwrap());
    let last = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let offset = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let len = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    if first > last || offset.checked_add(len).is_none_or(|end| end > file.len()) {
        return Err(invalid("map gap record exceeds its page"));
    }
    Ok((first, last, offset, len))
}

#[cfg(test)]
mod tests;
