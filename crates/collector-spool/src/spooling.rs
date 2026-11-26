use crate::spool_writer::{Schedule, Unrestricted};
use crate::history::PreparedCampaigns;
use collector_capture::pack_conversion::{HelperSplit, SpoolConversion};
use collector_capture::sampling::{Captured, LossBatch, LostObservation};
use collector_characters::extended::characters::{CharacterConversion, CharacterShadow};
use collector_intel::intel::{IntelConversion, IntelShadow, ScriptVariableComparison};
use collector_lexical::lexical::{LexicalConversion, LexicalConversionStats};
use collector_projection::projection::{
    Projection, ProjectionScratch, project_table, scalar_table,
};
#[cfg(test)]
use collector_projection::table_conversion::table_iter;
use collector_status::status::status;
use collector_workers::spool_phases::{HELPER, PhaseObserver, Steps, retry_cause};
use engine_native::cgame_state::Clock;
use engine_native_memory::walk::Outcome;
#[cfg(test)]
use replay_capture_spool::spool::SegmentedSpool;
use replay_recording::control::ControlRecord;
use replay_capture_spool::spool::{FileBackend, Record, SegmentBackend};
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    BranchId, codec::invalid_static as invalid, identity::SessionStart, identity::is_campaign_uuid,
    rows::BorderConflictRow, rows::CampaignRow, rows::ClockRow, rows::CountryLinkRow,
    rows::CountryRow, rows::DivisionRow, rows::FactionMemberRow, rows::FactionRow,
    rows::ProvinceRow, rows::StateRow, rows::TableId, rows::TableRows, rows::WarRelationRow,
    rows::tables,
};
use replay_frame::{
    branches::BranchStart, branches::BranchTracker, branches::Gap, frame::BaselineToken,
    frame::FrameEncoder, frame::FrameInput, frame::Metrics, frame::TableInput, frame::TableView,
};
use serde::Serialize;
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::{
    borrow::Cow,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

enum Merged {
    Rows(TableRows),
    Update(BaselineToken, replay_data::sliced::SliceUpdate),
}

struct TableCache<T>([Option<T>; TABLE_COUNT]);
impl<T> TableCache<T> {
    fn new() -> Self {
        Self(std::array::from_fn(|_| None))
    }
    fn get(&self, table: &TableId) -> Option<&T> {
        self.0[table.index()].as_ref()
    }
    fn insert(&mut self, table: TableId, value: T) -> Option<T> {
        self.0[table.index()].replace(value)
    }
    fn remove(&mut self, table: &TableId) -> Option<T> {
        self.0[table.index()].take()
    }
    fn clear(&mut self) {
        self.0.iter_mut().for_each(|slot| *slot = None);
    }
    #[cfg(test)]
    fn contains_key(&self, table: &TableId) -> bool {
        self.get(table).is_some()
    }
}

struct ReadHours([Vec<(u32, i32)>; TABLE_COUNT]);
impl ReadHours {
    fn new() -> Self {
        Self(std::array::from_fn(|_| Vec::new()))
    }
    fn clear(&mut self) {
        self.0.iter_mut().for_each(Vec::clear);
    }
    fn merge(
        &mut self,
        table: TableId,
        owners: &[u32],
        selection: &collector_certificates::slicing::SliceSelection,
        hour: i32,
    ) -> io::Result<()> {
        let mut ordered;
        let owners = if owners.windows(2).any(|pair| pair[0] >= pair[1]) {
            ordered = engine_allocation::vec(owners.len())?;
            ordered.extend_from_slice(owners);
            ordered.sort_unstable();
            ordered.dedup();
            ordered.as_slice()
        } else {
            owners
        };
        let previous = &self.0[table.index()];
        let capacity = owners
            .len()
            .checked_add(selection.read_owners().count())
            .and_then(|count| count.checked_add(previous.len()))
            .ok_or_else(engine_allocation::oom)?;
        let mut reads = engine_allocation::vec(capacity)?;
        let mut returned = owners.iter().copied().peekable();
        let mut selected = selection.read_owners().peekable();
        let mut old = previous.iter().copied().peekable();
        while let Some(owner) = returned
            .peek()
            .copied()
            .into_iter()
            .chain(selected.peek().copied())
            .chain(old.peek().map(|entry| entry.0))
            .min()
        {
            let returned = returned.next_if(|&value| value == owner).is_some();
            let selected = selected.next_if(|&value| value == owner).is_some();
            let previous = old.next_if(|entry| entry.0 == owner);
            let captured = selection.contains(owner);
            let read = if returned {
                if selection.force_full || captured {
                    Some(hour)
                } else {
                    previous.map(|entry| entry.1)
                }
            } else if selected {
                Some(hour)
            } else if !selection.force_full && !captured && selection.live(owner) {
                previous.map(|entry| entry.1)
            } else {
                None
            };
            if let Some(read) = read {
                reads.push((owner, read));
            }
        }
        self.0[table.index()] = reads;
        Ok(())
    }

    fn input(
        &self,
        frame_reads: &[Option<Vec<(u32, i32)>>; TABLE_COUNT],
        encoder: &FrameEncoder,
        branch: BranchId,
        hour: i32,
        force: bool,
    ) -> io::Result<TableInput> {
        let count = frame_reads
            .iter()
            .zip(&self.0)
            .try_fold(0_usize, |count, (view, base)| {
                count.checked_add(view.as_ref().unwrap_or(base).len())
            })
            .ok_or_else(engine_allocation::oom)?;
        let rows = || {
            frame_reads
                .iter()
                .zip(&self.0)
                .enumerate()
                .flat_map(|(table, (view, base))| {
                    view.as_ref()
                        .unwrap_or(base)
                        .iter()
                        .map(
                            move |&(owner, sample_hour)| replay_data::rows::SliceReadRow {
                                table: table as u32,
                                owner,
                                sample_hour,
                            },
                        )
                })
        };
        if let Some((token, baseline)) =
            encoder.comparison_baseline::<tables::SliceReads>(branch, hour, force)
            && token.allows_row_updates()
            && baseline.len() == count
        {
            let mut changed = 0;
            let same_keys = rows().zip(baseline).all(|(row, previous)| {
                changed += usize::from(row.sample_hour != previous.sample_hour);
                row.table == previous.table && row.owner == previous.owner
            });
            if same_keys {
                if changed == 0 {
                    return Ok(TableInput::ComparedEqual(token));
                }
                let mut indices = engine_allocation::vec(changed)?;
                let mut current = engine_allocation::vec(changed)?;
                for (index, (row, previous)) in rows().zip(baseline).enumerate() {
                    if row.sample_hour != previous.sample_hour {
                        indices.push(index);
                        current.push(row);
                    }
                }
                return Ok(TableInput::ComparedChanges {
                    token,
                    indices,
                    rows: TableRows::SliceReads(current),
                });
            }
        }
        let mut complete = engine_allocation::vec(count)?;
        complete.extend(rows());
        Ok(TableView::Complete(TableRows::SliceReads(complete)).into())
    }

    #[cfg(test)]
    fn get(&self, &(table, owner): &(u32, u32)) -> Option<&i32> {
        let rows = self.0.get(table as usize)?;
        rows.binary_search_by_key(&owner, |entry| entry.0)
            .ok()
            .map(|index| &rows[index].1)
    }
    #[cfg(test)]
    fn keys(&self) -> impl Iterator<Item = (u32, u32)> {
        self.0
            .iter()
            .enumerate()
            .flat_map(|(table, rows)| rows.iter().map(move |&(owner, _)| (table as u32, owner)))
    }
}

fn set_aside_in_context<S: Schedule>(
    id: &str,
    campaign: &mut Campaign<FileBackend, S>,
    storage: Option<crate::spool_writer::ReceiptStorage>,
) -> io::Result<()> {
    let caller = collector_memory::terminal::context();
    collector_memory::terminal::set_context(collector_memory::terminal::Context {
        campaign_id: id.as_bytes().try_into().ok(),
        branch: campaign.tracker.current_branch(),
        load_signal: None,
        failed_hour: None,
    });
    let result = campaign.set_aside(storage);
    collector_memory::terminal::set_context(caller);
    if result.is_err() {
        campaign.disk_branch = None;
        campaign.encoder = FrameEncoder::new();
    }
    result
}

struct Campaigns<S: Schedule>(Vec<(String, Campaign<FileBackend, S>)>);
impl<S: Schedule> Campaigns<S> {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn position(&self, id: &str) -> Result<usize, usize> {
        self.0.binary_search_by(|entry| entry.0.as_str().cmp(id))
    }
    fn contains_key(&self, id: &str) -> bool {
        self.position(id).is_ok()
    }
    fn get_mut(&mut self, id: &str) -> Option<&mut Campaign<FileBackend, S>> {
        let index = self.position(id).ok()?;
        Some(&mut self.0[index].1)
    }
    fn insert(&mut self, id: String, campaign: Campaign<FileBackend, S>) -> io::Result<()> {
        match self.position(&id) {
            Ok(index) => self.0[index].1 = campaign,
            Err(index) => {
                engine_allocation::reserve(&mut self.0, 1)?;
                self.0.insert(index, (id, campaign));
            }
        }
        Ok(())
    }
    fn values(&self) -> impl Clone + Iterator<Item = &Campaign<FileBackend, S>> {
        self.0.iter().map(|(_, campaign)| campaign)
    }
    fn keys(&self) -> impl DoubleEndedIterator<Item = &String> {
        self.0.iter().map(|(id, _)| id)
    }
}
impl<'a, S: Schedule> IntoIterator for &'a mut Campaigns<S> {
    type Item = (&'a String, &'a mut Campaign<FileBackend, S>);
    type IntoIter = std::iter::Map<
        std::slice::IterMut<'a, (String, Campaign<FileBackend, S>)>,
        fn(&mut (String, Campaign<FileBackend, S>)) -> (&String, &mut Campaign<FileBackend, S>),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut().map(|(id, campaign)| (&*id, campaign))
    }
}

fn error_reason(error: &io::Error) -> Cow<'static, str> {
    match error.kind() {
        io::ErrorKind::OutOfMemory => Cow::Borrowed("collector allocation failed"),
        io::ErrorKind::InvalidData => {
            replay_data::codec::try_format(format_args!("spool validation failed: {error}"))
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed("collector allocation failed"))
        }
        _ => Cow::Borrowed("spool I/O failed"),
    }
}

pub(crate) fn committed_spool_record(segment: u32, bytes: u64, complete_hour: bool) {
    collector_memory::terminal::committed_record(segment, bytes, complete_hour);
    if complete_hour {
        collector_memory::terminal::attempted_hour(None);
    }
}

pub(crate) use collector_extended_support::distribution::Distribution;

struct PendingBranch {
    start: BranchStart,
    gaps: Vec<Gap>,
    event_loss: Option<(i32, i32)>,
}

#[derive(Default)]
struct WorkSample {
    staged: Option<crate::spool_writer::StagedFrame>,
    validation_refused: bool,
    validation_families: u32,
    viewed: Option<ViewedFlag>,
    line_output_receipt: collector_output::line_tables::Receipt,
    construction_ic_receipt: collector_output::construction_tables::Receipt,
}

enum CachedTable {
    Complete(Arc<TableRows>),
    Truncated { rows: TableRows, missing: u32 },
    Unavailable { reason: String },
}

impl CachedTable {
    fn input(&self) -> io::Result<TableInput> {
        Ok(match self {
            Self::Complete(rows) => TableInput::SharedComplete(Arc::clone(rows)),
            Self::Truncated { rows, missing } => TableInput::Sampled(TableView::Truncated {
                rows: rows.try_clone()?,
                missing: *missing,
            }),
            Self::Unavailable { reason } => TableInput::Sampled(TableView::Unavailable {
                reason: engine_allocation::string(reason)?,
            }),
        })
    }
}

fn lexical_cache(
    cache: &TableCache<CachedTable>,
    cached_branch: Option<BranchId>,
    branch: BranchId,
) -> Option<&Arc<TableRows>> {
    if cached_branch != Some(branch) {
        return None;
    }
    match cache.get(&replay_data::rows::TableId::LexicalTokens) {
        Some(CachedTable::Complete(rows)) => Some(rows),
        _ => None,
    }
}

const SPOOL_TABLES: [TableId; 12] = [
    TableId::Clock,
    TableId::Campaign,
    TableId::Players,
    TableId::LocalPlayer,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::ReservedCountryNaming,
];

fn spooled(table: TableId) -> bool {
    #[cfg(test)]
    if KEEP_EVERY_TABLE.with(std::cell::Cell::get) {
        return true;
    }
    SPOOL_TABLES.contains(&table) || crate::history::reads(table)
}

fn baseline_retention(
    table: TableId,
    live_columnar: bool,
) -> replay_frame::frame::BaselineRetention {
    use replay_data::cadence::Cadence;
    use replay_frame::frame::BaselineRetention;
    if !live_columnar {
        return BaselineRetention::Full;
    }
    if matches!(
        table,
        TableId::DivisionTemplateIcons
            | TableId::CountryTemplateState
            | TableId::CharacterPortraits
            | TableId::HiredAdvisors
            | TableId::CountryCharacters
            | TableId::CountryRetiredCharacters
            | TableId::ShipModels
            | TableId::RailEdges
            | TableId::DivisionPanelStats
            | TableId::DivisionHistory
            | TableId::CharacterRoleTraits
            | TableId::StateResistance
    ) {
        return BaselineRetention::Meta;
    }
    if spooled(table)
        || table == TableId::SliceReads
        || collector_characters::extended::characters::SHADOW_TABLES.contains(&table)
        || matches!(
            table,
            TableId::CountryMarket
                | TableId::MarketContractTerms
                | TableId::LendLeaseAgreements
                | TableId::AdvisorTokens
                | TableId::DivisionTemplates
                | TableId::CombatLogObservers
                | TableId::RailCooldowns
                | TableId::VariantDefinitions
                | TableId::VariantModules
                | TableId::VariantStats
                | TableId::VariantUpgrades
                | TableId::WeatherProvinces
                | TableId::ScriptVariables
                | TableId::ScriptVariableNames
                | TableId::ScriptVariableValues
                | TableId::Provinces
                | TableId::States
        )
    {
        return BaselineRetention::Full;
    }
    if matches!(
        table,
        TableId::EventFeedHistory
            | TableId::LandCombatResults
            | TableId::LandCombatResultObservers
            | TableId::NavalResults
            | TableId::AirLossEvents
            | TableId::BuildingBombingDamage
            | TableId::CountryPowerChanges
            | TableId::GovernmentChanges
            | TableId::MarketEvents
            | TableId::EquipmentDestruction
            | TableId::OperationCompletions
            | TableId::NuclearStrikes
            | TableId::CombatEngagementRefreshes
            | TableId::CountryExperienceGains
            | TableId::DivisionEquipmentDeletions
            | TableId::PeaceBids
            | TableId::PeaceSettlements
            | TableId::VariantChanges
            | TableId::VariantBonusSnapshots
            | TableId::DesignCountryModifiers
            | TableId::DesignCountryTemplates
            | TableId::DesignObserverContext
            | TableId::DesignOrganisationLiveness
            | TableId::MapEvents
            | TableId::CountryPowerDaily
            | TableId::CountryPowerDailyCosts
    ) {
        return BaselineRetention::Drop;
    }
    if replay_data::cadence::cadence(table) == Cadence::Hourly {
        BaselineRetention::Full
    } else {
        BaselineRetention::Drop
    }
}

#[cfg(test)]
thread_local! {
    static KEEP_EVERY_TABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn keep_every_table() {
    KEEP_EVERY_TABLE.with(|keep| keep.set(true));
}

const REPUBLISHING: [(&str, TableId); 11] = [
    ("division_templates", TableId::DivisionTemplates),
    ("division_templates", TableId::DivisionTemplateIcons),
    ("division_templates", TableId::CountryTemplateState),
    ("character_portraits", TableId::CharacterPortraits),
    ("character_inputs", TableId::CharacterRoleTraits),
    ("hired_advisors", TableId::HiredAdvisors),
    ("hired_advisors", TableId::CountryCharacters),
    ("hired_advisors", TableId::AdvisorTokens),
    ("hired_advisors", TableId::CountryRetiredCharacters),
    ("ship_models", TableId::ShipModels),
    ("states", TableId::StateResistance),
];

fn character_cache(
    cache: &TableCache<CachedTable>,
    cached_branch: Option<BranchId>,
    branch: BranchId,
) -> [Option<&Arc<TableRows>>; 4] {
    collector_characters::extended::characters::SHADOW_TABLES.map(|table| {
        if cached_branch != Some(branch) {
            return None;
        }
        match cache.get(&table) {
            Some(CachedTable::Complete(rows)) => Some(rows),
            _ => None,
        }
    })
}

struct ValidationRefusal {
    table: String,
    column: Option<String>,
    reason: String,
}

#[derive(Clone, Default)]
pub(crate) struct ValidationRefusals(Arc<std::sync::Mutex<Vec<ValidationRefusal>>>);

fn validation_refusal(
    failures: &mut Vec<ValidationRefusal>,
    directory: &Path,
    table: TableId,
    hour: i32,
    error: io::Error,
) -> io::Result<String> {
    let detail = replay_data::codec::try_format(format_args!("{error}"))?;
    let (column, reason) = detail
        .split_once(" column ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .map_or(
            (
                None,
                detail.split_once(": ").map_or(detail.as_str(), |(_, r)| r),
            ),
            |(column, reason)| (Some(column), reason),
        );
    let table_name = replay_data::codec::try_format(format_args!("{table:?}"))?;
    if !failures
        .iter()
        .any(|f| f.table == table_name && f.column.as_deref() == column && f.reason == reason)
    {
        engine_allocation::reserve(failures, 1)?;
        failures.push(ValidationRefusal {
            table: table_name,
            column: column.map(engine_allocation::string).transpose()?,
            reason: engine_allocation::string(reason)?,
        });
        collector_hook_failures::log(
            directory.parent().unwrap_or(directory),
            format_args!("spool validation refused at hour {hour}: {detail}"),
        )?;
    }
    replay_data::codec::try_format(format_args!("spool validation failed: {detail}"))
}

struct Campaign<B: SegmentBackend = FileBackend, S: Schedule = Unrestricted> {
    validation_refusals: ValidationRefusals,
    world_generation: Option<u64>,
    directory: PathBuf,
    session: SessionStart,
    tracker: BranchTracker,
    resumed: Option<(BranchId, i32)>,
    encoder: FrameEncoder,
    spool: crate::spool_writer::Output<B, S>,
    disk_branch: Option<BranchId>,
    last_written: Option<i32>,
    pending: Vec<PendingBranch>,
    submitted_context: Vec<PendingBranch>,
    closing: Vec<(
        crate::spool_writer::CloseReceipt,
        Vec<PendingBranch>,
        Option<crate::spool_writer::ReceiptStorage>,
    )>,
    staggered_cache: TableCache<CachedTable>,
    staggered_complete: [bool; TABLE_COUNT],
    staggered_branch: Option<BranchId>,
    country_intel_sample: Option<replay_data::rows::ReservedCountryIntelSample>,
    country_intel_hour: Option<i32>,
    sliced_cache: TableCache<Arc<TableRows>>,
    sliced_owners: [Option<Vec<u32>>; TABLE_COUNT],
    sliced_reads: ReadHours,
    sliced_branch: Option<BranchId>,
    pending_slices: Vec<(TableId, Arc<replay_data::sliced::SliceUpdate>)>,
    retired: Vec<replay_frame::frame::Retired>,
    line_output: collector_output::line_tables::Totals,
    operation_outcomes: collector_operation_outcomes::extended::operation_outcomes::Ledger,
    map_events: collector_capture::extended::map_events::Ledger,
    politics: collector_politics::politics::Ledger,
    construction_ic: collector_output::construction_tables::Totals,
    fuel_awards: collector_fuel_awards::tables::Windows,
    variables_scratch: ScriptVariableComparison,
    character_shadow: CharacterShadow,
    intel_shadow: IntelShadow,
    projection_scratch: ProjectionScratch,
    lexical_conversion: LexicalConversionStats,
    history_lease: Option<File>,
    columnar: crate::columnar::Columnar,
}

impl<S: Schedule> Campaign<FileBackend, S> {
    fn open(
        root: &Path,
        history: &mut PreparedCampaigns,
        id: &str,
        session: &SessionStart,
        spool: crate::spool_writer::Output<FileBackend, S>,
        journal_thread: &std::sync::Arc<crate::columnar::JournalThread>,
    ) -> io::Result<Self> {
        let directory = root.join(id);
        if directory.exists()
            && fs::read_dir(&directory)
                .map_err(engine_allocation::allocation_error)?
                .any(|entry| {
                    entry.is_ok_and(|entry| {
                        entry.file_name().to_string_lossy().starts_with("spool.")
                    })
                })
        {
            return Err(invalid(
                "attach output already contains spool segments; use a fresh attach directory",
            ));
        }
        let world_generation = history.campaign_world_generation(id);
        let (tracker, history_file) = history.take(id, session)?;
        let columnar = crate::columnar::Columnar::open(&directory, id, session, journal_thread);
        Ok(Self {
            validation_refusals: ValidationRefusals::default(),
            directory,
            session: session.try_clone()?,
            resumed: tracker.current_branch().and_then(|branch| {
                tracker
                    .recorded_hours(branch)
                    .and_then(|hours| hours.last().map(|hour| (branch, *hour)))
            }),
            tracker,
            encoder: FrameEncoder::new(),
            spool,
            disk_branch: None,
            world_generation,
            last_written: None,
            pending: Vec::new(),
            submitted_context: Vec::new(),
            closing: Vec::new(),
            staggered_cache: TableCache::new(),
            staggered_complete: [false; TABLE_COUNT],
            staggered_branch: None,
            country_intel_sample: None,
            country_intel_hour: None,
            sliced_cache: TableCache::new(),
            sliced_owners: std::array::from_fn(|_| None),
            sliced_reads: ReadHours::new(),
            sliced_branch: None,
            pending_slices: Vec::new(),
            retired: Vec::new(),
            line_output: Default::default(),
            operation_outcomes: Default::default(),
            map_events: Default::default(),
            politics: Default::default(),
            construction_ic: Default::default(),
            fuel_awards: Default::default(),
            variables_scratch: ScriptVariableComparison::default(),
            character_shadow: CharacterShadow::default(),
            intel_shadow: IntelShadow::default(),
            projection_scratch: ProjectionScratch::default(),
            lexical_conversion: LexicalConversionStats::default(),
            history_lease: history_file,
            columnar,
        })
    }
}

impl<B: SegmentBackend, S: Schedule> Campaign<B, S> {
    fn reset_after_write_failure(&mut self) {
        self.columnar.interrupt();
        self.spool.invalidate();
        self.disk_branch = None;
        self.encoder = FrameEncoder::new();
        self.pending_slices.clear();
        self.staggered_cache.clear();
        self.staggered_complete.fill(false);
        self.staggered_branch = None;
        self.sliced_cache.clear();
        self.sliced_owners.fill(None);
        self.sliced_reads.clear();
        self.sliced_branch = None;
        self.country_intel_sample = None;
        self.country_intel_hour = None;
        self.character_shadow = CharacterShadow::default();
        self.intel_shadow = IntelShadow::default();
        self.projection_scratch = ProjectionScratch::default();
    }

    fn pending_branch(&mut self, branch: BranchId) -> io::Result<&mut PendingBranch> {
        let index = self
            .pending
            .partition_point(|pending| pending.start.id < branch);
        if self
            .pending
            .get(index)
            .is_none_or(|pending| pending.start.id != branch)
        {
            let start = self
                .tracker
                .branch_start(branch)
                .expect("observed branch exists")
                .clone();
            engine_allocation::reserve(&mut self.pending, 1)?;
            self.pending.insert(
                index,
                PendingBranch {
                    start,
                    gaps: Vec::new(),
                    event_loss: None,
                },
            );
        }
        Ok(&mut self.pending[index])
    }

    fn gap(
        &mut self,
        branch: BranchId,
        first_hour: i32,
        last_hour: i32,
        reason: String,
    ) -> io::Result<()> {
        let gaps = &mut self.pending_branch(branch)?.gaps;
        if let Some(previous) = gaps.last_mut()
            && previous.reason == reason
            && previous.last_hour.checked_add(1) == Some(first_hour)
        {
            previous.last_hour = last_hour;
        } else {
            engine_allocation::reserve(gaps, 1)?;
            gaps.push(Gap {
                branch,
                first_hour,
                last_hour,
                reason,
            });
        }
        Ok(())
    }

    fn observe(
        &mut self,
        loaded: Option<i32>,
        hour: i32,
        signal: Option<i64>,
    ) -> io::Result<BranchId> {
        let gaps = self
            .pending
            .last()
            .filter(|pending| Some(pending.start.id) == self.tracker.current_branch())
            .map_or(&[][..], |pending| pending.gaps.as_slice());
        let observation = self.tracker.observe(loaded, hour, signal, gaps)?;
        let event_loss = self
            .resumed
            .take()
            .filter(|&(branch, _)| observation.start.is_none() && observation.branch == branch)
            .map(|(_, after)| (after, hour));
        let pending = self.pending_branch(observation.branch)?;
        if event_loss.is_some() {
            pending.event_loss = event_loss;
        }
        Ok(observation.branch)
    }

    fn owner_sliced(&self, table: TableId) -> bool {
        replay_data::cadence::is_sliced(table)
            && !replay_data::sliced::is_keyed(table)
            && !spooled(table)
            && self.columnar.is_live()
    }

    fn try_merge_sliced_inputs<'a>(
        &mut self,
        source: &mut FrameInput,
        selection: impl Fn(TableId) -> Option<&'a collector_certificates::slicing::SliceSelection>,
        walked: impl Fn(TableId) -> bool,
        carried_owner: impl Fn(TableId, u32) -> bool,
    ) -> io::Result<(bool, u32)> {
        self.settle_failed_slices();
        if self.sliced_branch != Some(source.branch) {
            self.sliced_cache.clear();
            self.sliced_owners.fill(None);
            self.sliced_reads.clear();
            self.sliced_branch = Some(source.branch);
        }
        let force = !self.spool.is_open();
        let mut retry = false;
        let mut unpublished = 0_u32;
        let mut frame_reads: [Option<Vec<(u32, i32)>>; TABLE_COUNT] = std::array::from_fn(|_| None);
        let hour = source.hour;
        for table in TableId::ALL {
            if !replay_data::cadence::is_sliced(table) {
                continue;
            }
            let input = &mut source.tables[table.index()];
            if walked(table)
                && matches!(input, TableInput::Sampled(TableView::Unavailable { reason })
                    if *reason != replay_data::cadence::SKIPPED_REASON)
                && let Some(family) = collector_certificates::slicing::family_of(table)
            {
                unpublished |= 1 << family;
            }
            let Some(selection) = selection(table) else {
                *input = TableInput::Sampled(unavailable("sliced table has no owner selection"));
                continue;
            };
            let captured = |owner| selection.contains(owner);
            let live = |owner| selection.live(owner);
            match input {
                TableInput::Sampled(TableView::Complete(rows)) if self.owner_sliced(table) => {
                    let fresh = std::mem::replace(rows, TableRows::empty(table));
                    if selection.force_full || replay_data::sliced::replaces_whole(&fresh) {
                        let owners = replay_data::sliced::try_owners(&fresh)?;
                        self.sliced_reads.merge(table, &owners, selection, hour)?;
                        self.sliced_cache.remove(&table);
                        self.sliced_owners[table.index()] = Some(owners);
                        *input = TableInput::SharedComplete(Arc::new(fresh));
                    } else if let Some(previous) = self.sliced_cache.remove(&table) {
                        match replay_data::sliced::merge_slice(&previous, fresh, captured, live) {
                            Ok(merged) => {
                                let owners = replay_data::sliced::try_owners(&merged)?;
                                self.sliced_reads.merge(table, &owners, selection, hour)?;
                                self.sliced_owners[table.index()] = Some(owners);
                                *input = TableInput::SharedComplete(Arc::new(merged));
                            }
                            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                return Err(error);
                            }
                            Err(_) => {
                                *input =
                                    TableInput::Sampled(unavailable("sliced table merge failed"));
                                if let Some(family) =
                                    collector_certificates::slicing::family_of(table)
                                {
                                    unpublished |= 1 << family;
                                }
                                retry = true;
                            }
                        }
                    } else if let (Some(previous_owners), Some(token)) = (
                        self.sliced_owners[table.index()].as_deref(),
                        self.encoder
                            .untracked_slice_token(table, source.branch, hour, force),
                    ) {
                        let fresh_owners = replay_data::sliced::try_owners(&fresh)?;
                        match replay_data::sliced::SliceUpdate::for_owners_and_carried(
                            previous_owners,
                            fresh,
                            captured,
                            |owner| carried_owner(table, owner),
                            live,
                        ) {
                            Ok((update, next_owners)) => {
                                self.sliced_reads
                                    .merge(table, &fresh_owners, selection, hour)?;
                                self.sliced_owners[table.index()] = Some(next_owners);
                                *input = TableInput::SliceUpdate {
                                    token,
                                    update: Arc::new(update),
                                };
                            }
                            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                return Err(error);
                            }
                            Err(_) => {
                                *input =
                                    TableInput::Sampled(unavailable("sliced table merge failed"));
                                if let Some(family) =
                                    collector_certificates::slicing::family_of(table)
                                {
                                    unpublished |= 1 << family;
                                }
                                retry = true;
                            }
                        }
                    } else {
                        *input = TableInput::Sampled(unavailable(
                            "sliced table has no earlier complete rows on this branch; full refresh requested",
                        ));
                        retry = true;
                    }
                }
                TableInput::Sampled(TableView::Complete(rows)) => {
                    let fresh = std::mem::replace(rows, TableRows::empty(table));
                    let update = (!selection.force_full
                        && !replay_data::sliced::is_keyed(table)
                        && !replay_data::sliced::replaces_whole(&fresh))
                    .then(|| self.sliced_cache.get(&table))
                    .flatten()
                    .and_then(|previous| {
                        self.encoder
                            .slice_baseline(table, source.branch, hour, force)
                            .filter(|(_, baseline)| Arc::ptr_eq(baseline, previous))
                            .map(|(token, _)| (token, previous))
                    });
                    let mut fresh_owners = None;
                    let merged = if let Some((token, previous)) = update {
                        if !replay_data::sliced::is_keyed(table) {
                            fresh_owners = Some(replay_data::sliced::try_owners(&fresh)?);
                        }
                        match replay_data::sliced::SliceUpdate::diff_owners(
                            previous, fresh, captured, live,
                        ) {
                            Ok(update) => Merged::Update(token, update),
                            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                return Err(error);
                            }
                            Err(_) => {
                                *input =
                                    TableInput::Sampled(unavailable("sliced table merge failed"));
                                if let Some(family) =
                                    collector_certificates::slicing::family_of(table)
                                {
                                    unpublished |= 1 << family;
                                }
                                retry = true;
                                continue;
                            }
                        }
                    } else if selection.force_full || replay_data::sliced::replaces_whole(&fresh) {
                        Merged::Rows(fresh)
                    } else if let Some(previous) = self.sliced_cache.get(&table) {
                        match replay_data::sliced::merge_slice(previous, fresh, captured, live) {
                            Ok(merged) => Merged::Rows(merged),
                            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                return Err(error);
                            }
                            Err(_) => {
                                *input =
                                    TableInput::Sampled(unavailable("sliced table merge failed"));
                                if let Some(family) =
                                    collector_certificates::slicing::family_of(table)
                                {
                                    unpublished |= 1 << family;
                                }
                                retry = true;
                                continue;
                            }
                        }
                    } else {
                        *input = TableInput::Sampled(unavailable(
                            "sliced table has no earlier complete rows on this branch; full refresh requested",
                        ));
                        retry = true;
                        continue;
                    };
                    if !replay_data::sliced::is_keyed(table) {
                        let owners = match (&merged, fresh_owners) {
                            (_, Some(owners)) => owners,
                            (Merged::Rows(rows), None) => replay_data::sliced::try_owners(rows)?,
                            (Merged::Update(_, update), None) => {
                                replay_data::sliced::try_owners(update.rows())?
                            }
                        };
                        self.sliced_reads.merge(table, &owners, selection, hour)?;
                    }
                    match merged {
                        Merged::Rows(merged) => {
                            let merged = Arc::new(merged);
                            self.sliced_cache.insert(table, Arc::clone(&merged));
                            *input = TableInput::SharedComplete(merged);
                        }
                        Merged::Update(token, update) => {
                            let update = Arc::new(update);
                            engine_allocation::reserve(&mut self.pending_slices, 1)?;
                            self.pending_slices.push((table, Arc::clone(&update)));
                            *input = TableInput::SliceUpdate { token, update };
                        }
                    }
                }
                TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                    let missing = *missing;
                    let fresh = std::mem::replace(rows, TableRows::empty(table));
                    let returned = replay_data::sliced::try_owners(&fresh)?;
                    let replaced =
                        |owner: u32| captured(owner) && returned.binary_search(&owner).is_ok();
                    if !replay_data::sliced::is_keyed(table) {
                        let previous = &self.sliced_reads.0[table.index()];
                        let capacity = previous
                            .len()
                            .checked_add(returned.len())
                            .ok_or_else(engine_allocation::oom)?;
                        let mut reads = engine_allocation::vec(capacity)?;
                        let mut old = previous.iter().peekable();
                        for &owner in returned.iter().filter(|&&owner| captured(owner)) {
                            while old.peek().is_some_and(|entry| entry.0 < owner) {
                                reads.push(*old.next().unwrap());
                            }
                            if old.peek().is_some_and(|entry| entry.0 == owner) {
                                old.next();
                            }
                            reads.push((owner, hour));
                        }
                        reads.extend(old.copied());
                        frame_reads[table.index()] = Some(reads);
                    }
                    if self.sliced_cache.get(&table).is_none()
                        && self.owner_sliced(table)
                        && self.sliced_owners[table.index()].is_some()
                        && let Some(native_previous) =
                            self.columnar.materialize_complete(table, source.branch)?
                    {
                        self.sliced_cache.insert(table, Arc::new(native_previous));
                        retry = true;
                    }
                    if let Some(previous) = self.sliced_cache.get(&table) {
                        *input = TableInput::Sampled(
                            match replay_data::sliced::merge_slice(previous, fresh, replaced, live)
                            {
                                Ok(rows) => TableView::Truncated { rows, missing },
                                Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                    return Err(error);
                                }
                                Err(_) => {
                                    if let Some(family) =
                                        collector_certificates::slicing::family_of(table)
                                    {
                                        unpublished |= 1 << family;
                                    }
                                    unavailable("sliced table merge failed")
                                }
                            },
                        );
                    } else {
                        *input = TableInput::Sampled(unavailable(
                            "sliced table has no earlier complete rows on this branch; full refresh requested",
                        ));
                        retry = true;
                    }
                }
                _ => {}
            }
        }
        if unpublished != 0 {
            for table in TableId::ALL {
                if collector_certificates::slicing::family_of(table)
                    .is_some_and(|family| unpublished & (1 << family) != 0)
                {
                    self.sliced_cache.remove(&table);
                    self.sliced_owners[table.index()] = None;
                    self.pending_slices.retain(|(pending, _)| *pending != table);
                    let previous = std::mem::take(&mut self.sliced_reads.0[table.index()]);
                    if frame_reads[table.index()].is_none() {
                        frame_reads[table.index()] = Some(previous);
                    }
                }
            }
            retry = true;
        }
        source.tables[TableId::SliceReads.index()] =
            self.sliced_reads
                .input(&frame_reads, &self.encoder, source.branch, hour, force)?;
        Ok((retry, unpublished))
    }

    fn release_slices(&mut self) -> [bool; TABLE_COUNT] {
        let mut released = [false; TABLE_COUNT];
        for (table, _) in self.pending_slices.drain(..) {
            self.sliced_cache.remove(&table);
            released[table.index()] = true;
        }
        released
    }

    fn adopt_slices(&mut self, released: [bool; TABLE_COUNT]) {
        for table in TableId::ALL
            .into_iter()
            .filter(|table| released[table.index()])
        {
            if let Some(rows) = self.encoder.shared_baseline(table) {
                self.sliced_cache.insert(table, Arc::clone(rows));
            }
        }
    }

    fn settle_failed_slices(&mut self) {
        for (table, update) in std::mem::take(&mut self.pending_slices) {
            let Some(previous) = self.sliced_cache.get(&table) else {
                continue;
            };
            match update.merged(previous) {
                Ok(merged) => {
                    self.sliced_cache.insert(table, Arc::new(merged));
                }
                Err(_) => {
                    self.sliced_cache.remove(&table);
                }
            }
        }
    }

    #[cfg(test)]
    fn merge_sliced_inputs<'a>(
        &mut self,
        source: &mut FrameInput,
        selection: impl Fn(TableId) -> Option<&'a collector_certificates::slicing::SliceSelection>,
        walked: impl Fn(TableId) -> bool,
    ) -> (bool, u32) {
        self.try_merge_sliced_inputs(source, selection, walked, |_, _| false)
            .unwrap()
    }

    fn assemble_line_output(&mut self, source: &mut FrameInput, captured: &Captured) {
        self.line_output.assemble(
            source.branch,
            source.hour,
            captured.extended.line_output(),
            &mut source.tables,
        );
    }

    fn assemble_construction_ic(&mut self, source: &mut FrameInput, captured: &Captured) {
        self.construction_ic.assemble(
            source.branch,
            source.hour,
            captured.extended.construction_ic(),
            &mut source.tables,
        );
    }

    fn try_apply_staggered_inputs(
        &mut self,
        source: &mut FrameInput,
        intel_sample: Option<Result<replay_data::rows::ReservedCountryIntelSample, &'static str>>,
    ) -> io::Result<[bool; TABLE_COUNT]> {
        if self.staggered_branch != Some(source.branch) {
            self.staggered_cache.clear();
            self.staggered_complete.fill(false);
            self.country_intel_sample = None;
            self.country_intel_hour = None;
            self.staggered_branch = Some(source.branch);
        }
        let mut skipped = [false; TABLE_COUNT];
        for table in replay_data::rows::TableId::ALL {
            let index = table.index();
            let may_carry = replay_data::cadence::is_staggered(table);
            let conversion_cache = table == TableId::LexicalTokens
                || collector_characters::extended::characters::SHADOW_TABLES.contains(&table);
            if !may_carry && !conversion_cache {
                continue;
            }
            let input = &mut source.tables[index];
            let is_skip = matches!(
                &*input,
                TableInput::Sampled(TableView::Unavailable { reason })
                if reason == replay_data::cadence::SKIPPED_REASON
            );
            let cache_staggered = !self.columnar.is_live() || spooled(table) || conversion_cache;
            if is_skip {
                skipped[index] = true;
                if !may_carry {
                    *input = TableInput::Sampled(unavailable("hourly table was not acquired"));
                    self.staggered_cache.insert(
                        table,
                        CachedTable::Unavailable {
                            reason: engine_allocation::string("hourly table was not acquired")?,
                        },
                    );
                    continue;
                }
                if !cache_staggered {
                    if let Some(cached) = self.staggered_cache.get(&table) {
                        *input = cached.input()?;
                    } else if !self.staggered_complete[index]
                        || !self
                            .columnar
                            .has_complete(table, source.branch, source.hour)
                    {
                        *input = TableInput::Sampled(TableView::Unavailable {
                            reason: engine_allocation::string(
                                "staggered cache empty on new branch; full refresh required",
                            )?,
                        });
                    }
                    continue;
                }
                let restarting = !self.spool.is_open();
                let cached = self.staggered_cache.get(&table);
                if self
                    .encoder
                    .can_carry(table, source.branch, source.hour, restarting)
                {
                } else if let Some(cached @ CachedTable::Complete(_)) = cached {
                    *input = cached.input()?;
                } else if let Some(rows) =
                    self.encoder
                        .keyframe_carry(table, source.branch, source.hour, restarting)
                {
                    let rows = Arc::new(rows.try_clone()?);
                    self.staggered_cache
                        .insert(table, CachedTable::Complete(Arc::clone(&rows)));
                    *input = TableInput::SharedComplete(rows);
                } else if let Some(cached) = cached {
                    *input = cached.input()?;
                } else {
                    *input = TableInput::Sampled(TableView::Unavailable {
                        reason: engine_allocation::string(
                            "staggered cache empty on new branch; full refresh required",
                        )?,
                    });
                }
            } else {
                if !cache_staggered {
                    let compared_complete = matches!(
                        input,
                        TableInput::ComparedEqual(_) | TableInput::ComparedChanges { .. }
                    ) && self.staggered_complete[index]
                        && self
                            .columnar
                            .has_complete(table, source.branch, source.hour);
                    let complete = matches!(
                        input,
                        TableInput::Sampled(TableView::Complete(_))
                            | TableInput::SharedComplete(_)
                    ) || compared_complete;
                    if table == TableId::CountryIntel {
                        self.country_intel_hour = complete.then_some(source.hour);
                        self.country_intel_sample = if complete {
                            intel_sample
                                .and_then(Result::ok)
                                .filter(|sample| sample.sample_hour == source.hour)
                        } else {
                            None
                        };
                    }
                    match input {
                        TableInput::Sampled(TableView::Complete(_))
                        | TableInput::SharedComplete(_) => {
                            self.staggered_complete[index] = true;
                            self.staggered_cache.remove(&table);
                        }
                        TableInput::ComparedEqual(_) | TableInput::ComparedChanges { .. }
                            if compared_complete =>
                        {
                            self.staggered_complete[index] = true;
                            self.staggered_cache.remove(&table);
                        }
                        TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                            self.staggered_complete[index] = false;
                            self.staggered_cache.insert(
                                table,
                                CachedTable::Truncated {
                                    rows: rows.try_clone()?,
                                    missing: *missing,
                                },
                            );
                        }
                        TableInput::Sampled(TableView::Unavailable { reason }) => {
                            self.staggered_complete[index] = false;
                            self.staggered_cache.insert(
                                table,
                                CachedTable::Unavailable {
                                    reason: engine_allocation::string(reason)?,
                                },
                            );
                        }
                        TableInput::Sampled(TableView::NotRecorded { .. }) => {
                            self.staggered_complete[index] = false;
                            self.staggered_cache.remove(&table);
                        }
                        TableInput::ComparedEqual(_)
                        | TableInput::ComparedChanges { .. }
                        | TableInput::SliceUpdate { .. }
                        | TableInput::UniformOptionalI32 { .. } => {
                            self.staggered_complete[index] = false;
                            self.staggered_cache.insert(
                                table,
                                CachedTable::Unavailable {
                                    reason: engine_allocation::string(
                                        "a native comparison cannot supply a shared recovery snapshot",
                                    )?,
                                },
                            );
                        }
                    }
                    continue;
                }
                let cached = match input {
                    TableInput::Sampled(TableView::Complete(rows)) => {
                        let rows = Arc::new(std::mem::replace(rows, TableRows::empty(table)));
                        *input = TableInput::SharedComplete(Arc::clone(&rows));
                        CachedTable::Complete(rows)
                    }
                    TableInput::SharedComplete(rows) => CachedTable::Complete(Arc::clone(rows)),
                    TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                        CachedTable::Truncated {
                            rows: rows.try_clone()?,
                            missing: *missing,
                        }
                    }
                    TableInput::Sampled(TableView::Unavailable { reason }) => {
                        CachedTable::Unavailable {
                            reason: engine_allocation::string(reason)?,
                        }
                    }
                    TableInput::Sampled(TableView::NotRecorded { .. }) => continue,
                    TableInput::ComparedEqual(_)
                    | TableInput::ComparedChanges { .. }
                    | TableInput::SliceUpdate { .. }
                    | TableInput::UniformOptionalI32 { .. } => CachedTable::Unavailable {
                        reason: engine_allocation::string(
                            "a native comparison cannot supply a shared recovery snapshot",
                        )?,
                    },
                };
                if table == TableId::CountryIntel {
                    self.country_intel_hour =
                        matches!(&cached, CachedTable::Complete(_)).then_some(source.hour);
                    self.country_intel_sample = if matches!(&cached, CachedTable::Complete(_)) {
                        intel_sample
                            .and_then(Result::ok)
                            .filter(|sample| sample.sample_hour == source.hour)
                    } else {
                        None
                    };
                }
                self.staggered_cache.insert(table, cached);
            }
        }
        mark_reads(source, &skipped);
        Ok(skipped)
    }

    fn assemble_reserved_intel_sample(&self, source: &mut FrameInput) {
        let TableInput::Sampled(TableView::Complete(TableRows::ReservedCountryNaming(rows))) =
            &mut source.tables[TableId::ReservedCountryNaming.index()]
        else {
            return;
        };
        if let [row] = rows.as_mut_slice() {
            row.country_intel_sample =
                replay_data::rows::Recorded::Value(self.country_intel_sample);
        }
    }

    #[cfg(test)]
    fn apply_staggered_inputs(&mut self, source: &mut FrameInput) -> [bool; TABLE_COUNT] {
        self.try_apply_staggered_inputs(source, None).unwrap()
    }

    #[cfg(test)]
    fn apply_staggered_cache(
        &mut self,
        source: &mut replay_frame::frame::DecodedFrame,
    ) -> [bool; TABLE_COUNT] {
        let mut input = source.clone().into();
        let skipped = self.apply_staggered_inputs(&mut input);
        *source = sampled_frame(input);
        skipped
    }

    fn lost(&mut self, loss: &LostObservation, suppress_origin: bool) -> io::Result<()> {
        if loss.first_hour > loss.last_hour
            || i64::from(loss.last_hour) - i64::from(loss.first_hour) + 1
                != i64::try_from(loss.count).unwrap_or(-1)
        {
            return Err(invalid(
                "queue loss witness has inconsistent hours and count",
            ));
        }
        let origin = if suppress_origin || self.tracker.current_branch().is_none() {
            None
        } else {
            loss.loaded_hour
        };
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
            branch: None,
            load_signal: loss.load_signal,
            failed_hour: Some(loss.first_hour),
        });
        let branch = self.observe(origin, loss.first_hour, loss.load_signal)?;
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
            branch: Some(branch),
            load_signal: loss.load_signal,
            failed_hour: Some(loss.first_hour),
        });
        if loss.first_hour != loss.last_hour {
            self.gap(
                branch,
                loss.first_hour,
                loss.last_hour - 1,
                engine_allocation::string("collector queue overflow")?,
            )?;
            collector_memory::terminal::set_context(collector_memory::terminal::Context {
                campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                branch: None,
                load_signal: loss.load_signal,
                failed_hour: Some(loss.last_hour),
            });
            self.observe(None, loss.last_hour, loss.load_signal)?;
            collector_memory::terminal::set_context(collector_memory::terminal::Context {
                campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                branch: Some(branch),
                load_signal: loss.load_signal,
                failed_hour: Some(loss.last_hour),
            });
            self.gap(
                branch,
                loss.last_hour,
                loss.last_hour,
                engine_allocation::string("collector queue overflow")?,
            )?;
        } else {
            self.gap(
                branch,
                loss.first_hour,
                loss.last_hour,
                engine_allocation::string("collector queue overflow")?,
            )?;
        }
        Ok(())
    }

    fn context(&self, restarting: bool, extra: usize) -> io::Result<Vec<Record>> {
        let count = self
            .pending
            .iter()
            .try_fold(usize::from(restarting) + extra, |count, pending| {
                count
                    .checked_add(1)?
                    .checked_add(pending.gaps.len())?
                    .checked_add(usize::from(pending.event_loss.is_some()))
            })
            .ok_or_else(engine_allocation::oom)?;
        let mut records = engine_allocation::vec(count)?;
        let mut active = self.disk_branch;
        if restarting {
            records.push(Record::SessionStart(self.session.try_clone()?));
            active = None;
        }
        for pending in &self.pending {
            if active != Some(pending.start.id) {
                records.push(Record::BranchStart(pending.start.clone()));
                active = Some(pending.start.id);
            }
            for gap in &pending.gaps {
                records.push(Record::Gap(gap.try_clone()?));
            }
            if let Some((after_hour, hour)) = pending.event_loss {
                records.push(Record::Control(ControlRecord::EventLoss {
                    session: self.session.id,
                    branch: pending.start.id,
                    after_hour,
                    hour,
                }));
            }
        }
        Ok(records)
    }

    #[cfg(test)]
    fn write(
        &mut self,
        source: impl Into<FrameInput>,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
    ) -> io::Result<()> {
        let mut source = source.into();
        source.round_to_display();
        self.write_with(source, (qpc, signal), unknown, work, None, || {})
    }

    fn write_with(
        &mut self,
        mut source: FrameInput,
        witness: (i64, Option<i64>),
        unknown: u64,
        work: &mut WorkSample,
        helpers: Option<collector_workers::spool_helper::Helpers<'_>>,
        before_publication: impl FnOnce(),
    ) -> io::Result<()> {
        source.scope_to(self.session.capture_mode);
        let _details = replay_data::codec::error_details_scope();
        let (branch, hour) = (source.branch, source.hour);
        work.line_output_receipt = self
            .line_output
            .receipt(&source.tables[TableId::ProductionLineOutput.index()]);
        work.construction_ic_receipt = self
            .construction_ic
            .receipt(&source.tables[TableId::ConstructionLineOutput.index()]);
        let result = self.write_inner(source, witness, unknown, work, helpers, before_publication);
        if result.is_ok() && !self.spool.is_queued() {
            self.line_output.written(work.line_output_receipt);
            self.construction_ic.written(work.construction_ic_receipt);
        }
        if let Err(error) = &result {
            let _span =
            // SAFETY: This worker records phases on the counters assigned to its spool role.
            unsafe { collector_workers::spool_phases::SPOOL
                .span(collector_workers::spool_phases::Phase::Write) };
            let _ = self.spool.finish_segment();
            self.disk_branch = None;
            self.staggered_complete.fill(false);
            self.sliced_owners.fill(None);
            if !self.spool.is_queued()
                && error.kind() != io::ErrorKind::OutOfMemory
                && !collector_memory::terminal::failed()
            {
                self.encoder = FrameEncoder::new();
                self.columnar.interrupt();
                self.settle_failed_slices();
                self.gap(
                    branch,
                    hour,
                    hour,
                    engine_allocation::string("spool write failed")?,
                )?;
            }
        }
        result
    }

    fn write_inner(
        &mut self,
        source: FrameInput,
        witness: (i64, Option<i64>),
        unknown: u64,
        work: &mut WorkSample,
        helpers: Option<collector_workers::spool_helper::Helpers<'_>>,
        before_publication: impl FnOnce(),
    ) -> io::Result<()> {
        let (qpc, signal) = witness;
        let (branch, hour) = (source.branch, source.hour);
        let restarting = !self.spool.is_open();
        if restarting {
            self.encoder = FrameEncoder::new();
        }
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { collector_workers::spool_phases::SPOOL
            .span(collector_workers::spool_phases::Phase::Encode) };
        let frame = {
            let _phase =
                collector_memory::terminal::enter(collector_memory::terminal::Phase::Encode);
            let mut segments = match helpers {
                Some(helpers) => crate::spool_segments::encode_keeping(
                    &self.encoder,
                    &source,
                    (branch, hour, restarting),
                    spooled,
                    helpers,
                ),
                None => std::array::from_fn(|_| None),
            };
            self.encoder.prepare_rounded_contained_keeping(
                source,
                restarting,
                &mut segments,
                spooled,
                |table, error| {
                    let reason = validation_refusal(
                        &mut self
                            .validation_refusals
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                        &self.directory,
                        table,
                        hour,
                        error,
                    )?;
                    self.staggered_cache.remove(&table);
                    self.staggered_complete[table.index()] = false;
                    self.sliced_cache.remove(&table);
                    self.sliced_owners[table.index()] = None;
                    self.projection_scratch.forget(table);
                    self.intel_shadow.forget(table);
                    self.pending_slices.retain(|(id, _)| *id != table);
                    match table {
                        TableId::ProductionLineOutput => {
                            work.line_output_receipt = Default::default()
                        }
                        TableId::ConstructionLineOutput => {
                            work.construction_ic_receipt = Default::default()
                        }
                        _ => {}
                    }
                    work.validation_refused = true;
                    if let Some(family) = collector_certificates::slicing::family_of(table) {
                        work.validation_families |= 1 << family;
                    }
                    Ok(reason)
                })
        };
        let mut prepared = frame?;
        let span = span.switch(collector_workers::spool_phases::Phase::Columnar);
        let lanes = crate::columnar::Lanes::new(helpers);
        let row = replay_journal::native_frame::RowFrame {
            baselines: Some(&self.encoder),
            keyframe: prepared.frame().keyframe(),
            changed: prepared.changed_rows(),
        };
        let columnar = self.columnar.prepare(prepared.input(), row, &lanes)?;
        let mut span = span.switch(collector_workers::spool_phases::Phase::Write);
        let encoded = prepared.take_frame_keeping(spooled)?;
        let rotate = restarting || encoded.keyframe();
        let mut context = self.context(rotate, 2)?;
        if unknown != 0 {
            context.push(Record::Control(ControlRecord::GapSummary {
                session: self.session.id,
                dropped_frames: unknown,
                reason: engine_allocation::string(
                    "collector lost clock metadata; campaign and hour range are unknown",
                )?,
            }));
        }
        context.push(Record::Control(ControlRecord::ClockWitness {
            session: self.session.id,
            qpc: u64::try_from(qpc).map_err(|_| invalid("negative callback QPC"))?,
            hour,
            load_signal: signal,
        }));
        self.columnar.stage(&context);
        let queued = self.spool.is_queued();
        before_publication();
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        if queued {
            self.commit_baseline(prepared)?;
            if let Some(hour) = columnar {
                span = span.switch(collector_workers::spool_phases::Phase::Columnar);
                self.columnar.commit(hour, true, &lanes)?;
                span = span.switch(collector_workers::spool_phases::Phase::Write);
            }
            set_viewed(&mut work.viewed, viewed_flag(&self.encoder))?;
            work.staged = Some(crate::spool_writer::Output::<B, S>::stage_frame(
                context, encoded, rotate,
            )?);
            self.columnar.queued(branch, hour);
        } else {
            if let Some(hour) = columnar {
                span = span.switch(collector_workers::spool_phases::Phase::Columnar);
                self.columnar.commit(hour, true, &lanes)?;
                span = span.switch(collector_workers::spool_phases::Phase::Write);
            }
            let staged = crate::spool_writer::Output::<B, S>::stage_frame(context, encoded, rotate)?;
            self.spool.write_frame(staged, 0)?;
            self.columnar.written();
            if collector_memory::terminal::failed() {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            self.commit_baseline(prepared)?;
            self.columnar.complete(branch, hour);
            self.tracker.record_hour(branch, hour)?;
            #[cfg(feature = "alloc-fault-injection")]
            collector_memory::terminal::committed_hour();
        }
        let _span = span.switch(collector_workers::spool_phases::Phase::Commit);
        self.disk_branch = Some(branch);
        self.last_written = Some(hour);
        self.submitted_context = std::mem::take(&mut self.pending);
        if !queued {
            collector_memory::terminal::attempted_hour(None);
        }
        Ok(())
    }

    fn commit_baseline(&mut self, prepared: replay_frame::frame::PreparedFrame) -> io::Result<()> {
        engine_allocation::reserve(&mut self.retired, 1)?;
        let released = self.release_slices();
        let live_columnar = self.columnar.is_live();
        let retired = self
            .encoder
            .commit_retiring_mode(prepared, |table| baseline_retention(table, live_columnar))?;
        self.adopt_slices(released);
        if !retired.is_empty() {
            self.retired.push(retired);
        }
        Ok(())
    }

    fn detach(&mut self, unknown: u64) -> io::Result<()> {
        if !self.spool.is_open() {
            if self.pending.is_empty() && unknown == 0 {
                return Ok(());
            }
            if self
                .pending
                .last()
                .is_some_and(|pending| !pending.gaps.is_empty())
            {
                let mut context = self.context(true, usize::from(unknown != 0))?;
                if unknown != 0 {
                    context.push(Record::Control(ControlRecord::GapSummary {
                        session: self.session.id,
                        dropped_frames: unknown,
                        reason: engine_allocation::string(
                            "collector lost clock metadata; campaign and hour range are unknown",
                        )?,
                    }));
                }
                self.columnar.note(&context);
                self.spool.start_context(context)?;
                self.spool.finish_stopped_segment()?;
                self.pending.clear();
                return Ok(());
            }
            return Err(invalid(
                "spool unavailable at detach; pending context needs another observed keyframe",
            ));
        }
        let context = self.context(false, 0)?;
        self.columnar.note(&context);
        self.spool.append_context(context)?;
        if unknown != 0 {
            let summary = Record::Control(ControlRecord::GapSummary {
                session: self.session.id,
                dropped_frames: unknown,
                reason: engine_allocation::string(
                    "collector lost clock metadata; campaign and hour range are unknown",
                )?,
            });
            self.columnar.note(std::slice::from_ref(&summary));
            self.spool.append(summary)?;
        }
        let detach = Record::Control(ControlRecord::Detach {
            session: self.session.id,
            last_hour: self.last_written,
        });
        self.columnar.note(std::slice::from_ref(&detach));
        self.spool.append(detach)?;
        self.spool.finish_stopped_segment()?;
        self.pending.clear();
        Ok(())
    }

    fn close_storage_bytes(&self) -> usize {
        if self.spool.is_queued()
            && (self.spool.is_open()
                || self
                    .pending
                    .last()
                    .is_some_and(|pending| !pending.gaps.is_empty()))
        {
            pending_storage(&self.pending, self.pending.capacity())
        } else {
            0
        }
    }

    fn queue_close(
        &mut self,
        storage: Option<crate::spool_writer::ReceiptStorage>,
    ) -> io::Result<crate::spool_writer::CloseReceipt> {
        let context_submitted = self.spool.is_open()
            || self
                .pending
                .last()
                .is_some_and(|pending| !pending.gaps.is_empty());
        if context_submitted && !self.pending.is_empty() && self.spool.is_queued() {
            engine_allocation::reserve(&mut self.closing, 1)?;
        }
        if self.spool.is_open() {
            let context = self.context(false, 0)?;
            self.columnar.note(&context);
            self.spool.append_context(context)?;
        } else if self
            .pending
            .last()
            .is_some_and(|pending| !pending.gaps.is_empty())
        {
            let context = self.context(true, 0)?;
            self.columnar.note(&context);
            self.spool.start_context(context)?;
        }
        let receipt = self.spool.close_segment()?;
        if context_submitted && !self.pending.is_empty() {
            if self.spool.is_queued() {
                let pending = std::mem::take(&mut self.pending);
                self.closing.push((receipt.clone(), pending, storage));
            } else {
                self.pending.clear();
            }
        }
        Ok(receipt)
    }

    fn set_aside(
        &mut self,
        storage: Option<crate::spool_writer::ReceiptStorage>,
    ) -> io::Result<()> {
        if !self.spool.is_open()
            && self
                .pending
                .last()
                .is_none_or(|pending| pending.gaps.is_empty())
        {
            return Ok(());
        }
        self.queue_close(storage).map(|_| ())
    }

    fn restore_context(&mut self, context: Vec<PendingBranch>) -> io::Result<()> {
        for pending in context {
            let restored = self.pending_branch(pending.start.id)?;
            engine_allocation::reserve(&mut restored.gaps, pending.gaps.len())?;
            restored.gaps.splice(..0, pending.gaps);
            restored.event_loss = restored.event_loss.or(pending.event_loss);
        }
        Ok(())
    }

    fn lose_hour(&mut self, branch: BranchId, hour: i32) -> io::Result<()> {
        self.tracker.lose_hour(branch, hour)?;
        for pending in &mut self.pending {
            pending.start = self
                .tracker
                .branch_start(pending.start.id)
                .expect("pending branch exists")
                .clone();
        }
        Ok(())
    }
}

#[cfg(test)]
fn needs_retry(tables: &[TableInput; TABLE_COUNT]) -> bool {
    retry_table(tables).is_some()
}

pub(crate) const RETRY_SLICED: u16 = u16::MAX;
pub(crate) const RETRY_VALIDATION: u16 = u16::MAX - 1;

fn retry_table(tables: &[TableInput; TABLE_COUNT]) -> Option<TableId> {
    TableId::ALL.into_iter().find(|&table| {
        let input = &tables[table.index()];
        match replay_data::cadence::cadence(table) {
            replay_data::cadence::Cadence::Static => !matches!(
                input,
                TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_)
            ) && !matches!(input, TableInput::Sampled(TableView::Unavailable { reason }) if reason == replay_data::cadence::SKIPPED_REASON),
            replay_data::cadence::Cadence::Daily(_) => {
                matches!(input, TableInput::Sampled(TableView::Truncated { .. }))
            }
            replay_data::cadence::Cadence::Hourly | replay_data::cadence::Cadence::Sliced => false,
        }
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct ViewedFlag {
    tag: String,
    cosmetic: String,
    original: Option<String>,
    group: String,
}

impl ViewedFlag {
    fn same(&self, operands: &replay_data::country_flags::FlagOperands<'_>) -> bool {
        self.tag == operands.tag
            && self.cosmetic == operands.cosmetic
            && self.original.as_deref() == operands.original
            && self.group == operands.group
    }

    fn copy(operands: &replay_data::country_flags::FlagOperands<'_>) -> io::Result<Self> {
        Ok(Self {
            tag: engine_allocation::string(operands.tag)?,
            cosmetic: engine_allocation::string(operands.cosmetic)?,
            original: operands
                .original
                .map(engine_allocation::string)
                .transpose()?,
            group: engine_allocation::string(operands.group)?,
        })
    }
}

fn viewed_flag(encoder: &FrameEncoder) -> Option<replay_data::country_flags::FlagOperands<'_>> {
    replay_data::country_flags::viewed_country(
        encoder.committed::<tables::LocalPlayer>()?.1,
        encoder.committed::<tables::CountryLinks>()?.1,
        encoder.committed::<tables::Countries>()?.1,
        encoder.committed::<tables::CountryAppearance>()?.1,
        encoder.committed::<tables::Politics>()?.1,
        encoder.committed::<tables::PoliticalParties>()?.1,
    )
    .ok()
}

fn set_viewed(
    slot: &mut Option<ViewedFlag>,
    operands: Option<replay_data::country_flags::FlagOperands<'_>>,
) -> io::Result<()> {
    match operands {
        None => *slot = None,
        Some(operands) if slot.as_ref().is_some_and(|current| current.same(&operands)) => {}
        Some(operands) => *slot = Some(ViewedFlag::copy(&operands)?),
    }
    Ok(())
}

struct Statistics {
    validation_refusals: ValidationRefusals,
    hours: u64,
    campaign: Option<String>,
    last_hour: Option<i32>,
    player: Option<ViewedFlag>,
    total_ns: u64,
    callback: Distribution,
    callback_tail: crate::slow_hours::Recorder,
    conversion: collector_capture::pack_conversion::Conversion,
    dropped_frames: u64,
    truncated_tables: usize,
    pool_bytes: u64,
    previous: Option<(String, BranchId, i32, i64)>,
    interval_hours: u64,
    interval_qpc: u64,
    errors: u64,
    last_error: Option<Cow<'static, str>>,
    input_demand_errors: u64,
    input_demand_last_error: Option<&'static str>,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            validation_refusals: ValidationRefusals::default(),
            hours: 0,
            campaign: None,
            last_hour: None,
            player: None,
            total_ns: 0,
            callback: Distribution::default(),
            callback_tail: crate::slow_hours::Recorder::default(),
            conversion: collector_capture::pack_conversion::Conversion::default(),
            dropped_frames: 0,
            truncated_tables: 0,
            pool_bytes: 0,
            previous: None,
            interval_hours: 0,
            interval_qpc: 0,
            errors: 0,
            last_error: None,
            input_demand_errors: 0,
            input_demand_last_error: None,
        }
    }
}

#[must_use = "prepared observations must be written before processing another hour"]
pub struct PreparedFrame {
    writer_epoch: u64,
    source: FrameInput,
    id: String,
    clock: Clock,
    qpc: i64,
    unknown: u64,
    retry_full: bool,
    unpublished_families: u32,
    work: WorkSample,
    sampled: CapturedStatistics,
    operation_publication:
        Option<collector_operation_outcomes::extended::operation_outcomes::Publication>,
    map_event_publication: Option<collector_capture::extended::map_events::Publication>,
    event_feed_publication: Option<collector_observers::event_feed_history::Prepared>,
}

impl PreparedFrame {
    pub(crate) fn needs_retry(&self) -> bool {
        self.retry_full
    }

    pub fn allows_early_recycle(&self) -> bool {
        !self.needs_retry()
    }
}

struct CapturedStatistics {
    total_ns: u64,
    hour: Option<i32>,
    seq: u64,
    phases: collector_capture::sampling::HourPhases,
    tables: Option<[(TableId, u64); 4]>,
    pool_bytes: u64,
    dropped_frames: u64,
    truncated_tables: usize,
}

impl CapturedStatistics {
    fn capture(captured: &Captured) -> io::Result<Self> {
        let extended_outcomes = captured.extended.outcomes();
        let truncated_tables = [
            captured.countries_result,
            captured.links_result,
            captured.provinces_result,
            captured.provinces.conflict_walk.outcome,
            captured.states_result,
            captured.divisions_result,
            captured.wars_result,
            captured.factions_result,
            captured.faction_members_result,
        ]
        .into_iter()
        .chain(captured.map.outcomes())
        .chain(collector_capture::extended::PACK_TABLES.map(|table| {
            if table == TableId::StateResistance {
                captured.state_inputs.occupation.outcome()
            } else {
                extended_outcomes[table.index()]
            }
        }))
        .filter(|outcome| matches!(outcome, Outcome::Truncated { .. }))
        .count();
        #[cfg(feature = "phase_cpu")]
        // SAFETY: The sole spool publishes timing for the completed frame.
        unsafe {
            collector_workers::spool_phases::add_table_ns(&captured.table_ns)
        };
        Ok(Self {
            total_ns: captured.total_ns,
            hour: captured.clock.as_ref().map(|clock| clock.hour),
            seq: captured.capture_seq,
            phases: captured.phases,
            tables: crate::slow_hours::over_budget(captured.total_ns, &captured.phases)
                .then(|| crate::slow_hours::top_tables(&captured.table_ns)),
            pool_bytes: captured.pool_bytes,
            dropped_frames: captured.dropped_frames,
            truncated_tables,
        })
    }

    fn record(self, stats: &mut Statistics) -> io::Result<()> {
        stats.total_ns = self.total_ns;
        stats.callback.record_ns(self.total_ns);
        stats.callback_tail.record(
            self.total_ns,
            self.hour,
            self.seq,
            &self.phases,
            self.tables,
        )?;
        stats.pool_bytes = self.pool_bytes;
        stats.dropped_frames = self.dropped_frames;
        stats.truncated_tables = self.truncated_tables;
        Ok(())
    }
}

struct PublicationReceipt(Option<collector_observers::event_feed_history::Prepared>);
impl Drop for PublicationReceipt {
    fn drop(&mut self) {
        if let Some(publication) = &self.0 {
            // SAFETY: Dropping an uncommitted spool receipt discards its publication once.
            unsafe {
                collector_observers::event_feed_observer::discarded(
                    publication.publication(),
                    publication.row().generation,
                );
            }
        }
    }
}

struct QueuedFrame {
    storage: Option<crate::spool_writer::ReceiptStorage>,
    ticket: u64,
    context: Vec<PendingBranch>,
    unknown: u64,
    id: String,
    branch: BranchId,
    clock: Clock,
    qpc: i64,
    sampled: CapturedStatistics,
    operation_publication:
        Option<collector_operation_outcomes::extended::operation_outcomes::Publication>,
    map_event_publication: Option<collector_capture::extended::map_events::Publication>,
    event_feed_publication: PublicationReceipt,
    viewed: Option<ViewedFlag>,
    line_output_receipt: collector_output::line_tables::Receipt,
    construction_ic_receipt: collector_output::construction_tables::Receipt,
}

fn pending_storage(context: &[PendingBranch], capacity: usize) -> usize {
    capacity * size_of::<PendingBranch>()
        + context
            .iter()
            .map(|pending| {
                pending.gaps.capacity() * size_of::<Gap>()
                    + pending
                        .gaps
                        .iter()
                        .map(|gap| gap.reason.capacity())
                        .sum::<usize>()
            })
            .sum::<usize>()
}
impl QueuedFrame {
    fn storage_bytes(&self) -> usize {
        size_of::<Self>()
            + self.id.capacity()
            + self.clock.campaign_id.allocated_bytes()
            + pending_storage(&self.context, self.context.capacity())
            + self
                .operation_publication
                .as_ref()
                .map_or(0, |publication| publication.allocated_bytes())
            + self
                .map_event_publication
                .as_ref()
                .map_or(0, |publication| publication.allocated_bytes())
            + self
                .event_feed_publication
                .0
                .as_ref()
                .map_or(0, |publication| publication.allocated_bytes())
            + self.viewed.as_ref().map_or(0, |viewed| {
                viewed.tag.capacity()
                    + viewed.cosmetic.capacity()
                    + viewed.original.as_ref().map_or(0, String::capacity)
                    + viewed.group.capacity()
            })
    }
}

pub enum HistoryReply {
    Ready(String),
    Closing {
        token: String,
        receipt: crate::spool_writer::CloseReceipt,
    },
}

struct CommandWrite {
    id: String,
    world: u64,
    ordinal: u64,
    record: Record,
    attempted: bool,
    storage: Option<crate::spool_writer::ReceiptStorage>,
}

pub struct Processor<S: Schedule = Unrestricted> {
    root: PathBuf,
    history: PreparedCampaigns,
    session: SessionStart,
    session_verified: bool,
    campaigns: Campaigns<S>,
    stats: Statistics,
    unknown_pending: u64,
    suppress_origin: bool,
    retry_full: bool,
    unpublished_families: u32,
    helpers: Vec<collector_workers::spool_helper::SpoolHelper>,
    backlogged: bool,
    comparison: Box<crate::spool_compare::Counters>,
    writer: crate::spool_writer::Writer<S>,
    journal_thread: std::sync::Arc<crate::columnar::JournalThread>,
    queued: std::collections::VecDeque<Box<QueuedFrame>>,
    flight: Option<(String, BranchId)>,
    writer_error: Option<io::ErrorKind>,
    command_drain: collector_player_commands::player_commands::Drain,
    command_pending: Vec<collector_player_commands::player_commands::Drained>,
    command_contexts: std::collections::BTreeMap<u64, (String, BranchId)>,
    command_anchors: std::collections::BTreeMap<(u64, BranchId), (u64, i32, i64)>,
    command_ordinals: std::collections::BTreeMap<u64, u64>,
    command_definitions: std::collections::BTreeSet<(u64, BranchId)>,
    command_last_flush: std::time::Instant,
    command_ready: std::collections::BTreeMap<(u64, u64), CommandWrite>,
    command_inflight: std::collections::BTreeMap<u64, CommandWrite>,
}

impl<S: Schedule> Drop for Processor<S> {
    fn drop(&mut self) {
        self.writer.shutdown();
        while let Some(completion) = self.writer.completion() {
            if completion.frame
                && let Some(at) = self
                    .queued
                    .iter()
                    .position(|frame| frame.ticket == completion.ticket)
            {
                let mut frame = self.queued.remove(at).unwrap();
                if completion.committed
                    && let Some(publication) = frame.event_feed_publication.0.take()
                {
                    // SAFETY: Originating spool resolves a writer-confirmed publication receipt.
                    let _ = unsafe { collector_observers::event_feed_observer::accepted(&publication) };
                }
            }
        }
        self.queued.clear();
        self.writer.ledger_storage(0);
    }
}

impl Processor {
    /// # Safety
    /// Only the runtime's spool startup may construct the processor for this session's directory
    /// and history leases. It must join its writer before releasing those leases or unloading
    /// collector code.
    pub unsafe fn new(
        root: &Path,
        history: PreparedCampaigns,
        session: SessionStart,
    ) -> io::Result<Self> {
        Self::new_scheduled(root, history, session, crate::spool_writer::Writer::start)
    }
}

impl<S: Schedule> Processor<S> {
    pub fn account_writer(
        &self,
        counter: engine_allocation::SharedBox<std::sync::atomic::AtomicU64>,
    ) {
        self.writer.account_to(counter);
        self.account_ledger();
    }
    fn account_ledger(&self) {
        let bytes = self.queued.capacity() * size_of::<Box<QueuedFrame>>()
            + self
                .campaigns
                .values()
                .map(|campaign| {
                    campaign.closing.capacity()
                        * size_of::<(
                            crate::spool_writer::CloseReceipt,
                            Vec<PendingBranch>,
                            Option<crate::spool_writer::ReceiptStorage>,
                        )>()
                })
                .sum::<usize>();
        self.writer.ledger_storage(bytes);
    }
    fn close_storage(
        &mut self,
        id: &str,
    ) -> io::Result<Option<crate::spool_writer::ReceiptStorage>> {
        loop {
            self.poll_writer()?;
            let bytes = self
                .campaigns
                .get_mut(id)
                .map_or(0, |campaign| campaign.close_storage_bytes());
            if bytes == 0 {
                return Ok(None);
            }
            if let Some(storage) = self.writer.try_receipt_storage(
                bytes,
                size_of::<(
                    crate::spool_writer::CloseReceipt,
                    Vec<PendingBranch>,
                    Option<crate::spool_writer::ReceiptStorage>,
                )>(),
                0,
            )? {
                return Ok(Some(storage));
            }
            self.writer.wait_receipt_progress()?;
        }
    }

    fn set_aside_campaign(&mut self, id: &str) -> io::Result<()> {
        let storage = self.close_storage(id)?;
        if let Some(campaign) = self.campaigns.get_mut(id) {
            set_aside_in_context(id, campaign, storage)?;
        }
        self.account_ledger();
        Ok(())
    }

    fn receipt_storage(
        &mut self,
        bytes: usize,
        epoch: u64,
        write_bytes: usize,
    ) -> io::Result<crate::spool_writer::ReceiptStorage> {
        loop {
            self.poll_writer()?;
            if self.writer.failure_epoch() != epoch {
                return Err(io::Error::other(
                    "spool write failed during receipt admission",
                ));
            }
            if let Some(storage) = self.writer.try_receipt_storage(
                bytes,
                size_of::<Box<QueuedFrame>>(),
                write_bytes,
            )? {
                return Ok(storage);
            }
            self.writer.wait_receipt_progress()?;
        }
    }
    fn new_scheduled(
        root: &Path,
        history: PreparedCampaigns,
        session: SessionStart,
        make_writer: impl FnOnce(PathBuf) -> io::Result<crate::spool_writer::Writer<S>>,
    ) -> io::Result<Self> {
        let _details = replay_data::codec::error_details_scope();
        session.validate()?;
        fs::create_dir_all(root).map_err(engine_allocation::allocation_error)?;
        let writer = make_writer(root.into())?;
        let journal_thread = crate::columnar::JournalThread::start()?;
        Ok(Self {
            root: root.into(),
            history,
            session,
            session_verified: false,
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helpers: Vec::new(),
            backlogged: false,
            comparison: Box::default(),
            writer,
            journal_thread,
            queued: std::collections::VecDeque::new(),
            flight: None,
            writer_error: None,
            command_drain: Default::default(),
            command_pending: Vec::new(),
            command_contexts: Default::default(),
            command_anchors: Default::default(),
            command_ordinals: Default::default(),
            command_definitions: Default::default(),
            command_last_flush: std::time::Instant::now(),
            command_ready: Default::default(),
            command_inflight: Default::default(),
        })
    }

    pub fn set_helper(&mut self, helper: collector_workers::spool_helper::SpoolHelper) -> bool {
        if engine_allocation::reserve(&mut self.helpers, 1).is_err() {
            return false;
        }
        self.helpers.push(helper);
        true
    }

    pub fn attach_priority(
        &self,
        pipeline: engine_allocation::SharedBox<collector_workers::frame_priority::Pipeline>,
    ) {
        self.writer.attach_priority(pipeline.clone());
        self.journal_thread.attach_priority(pipeline);
    }

    pub fn writer_thread_ids(&self) -> [Option<u32>; 2] {
        [self.writer.thread_id(), self.journal_thread.thread_id()]
    }

    pub fn set_backlogged(&mut self, backlogged: bool) {
        self.backlogged = backlogged;
    }

    pub fn set_verified_session(&mut self, session: SessionStart) -> io::Result<()> {
        if !self.campaigns.is_empty()
            || !matches!(
                session.identity.verification,
                replay_data::identity::Verification::Verified { .. }
            )
        {
            return Err(invalid(
                "session verification must precede campaign publication",
            ));
        }
        self.session = session;
        self.session_verified = true;
        Ok(())
    }

    fn campaign(&mut self, id: &str) -> io::Result<&mut Campaign<FileBackend, S>> {
        if !self.campaigns.contains_key(id) {
            let campaign_id = id
                .as_bytes()
                .try_into()
                .map_err(|_| invalid("invalid campaign identity"))?;
            let output = self.writer.output(campaign_id);
            let mut campaign = Campaign::open(
                &self.root,
                &mut self.history,
                id,
                &self.session,
                output,
                &self.journal_thread,
            )?;
            campaign.validation_refusals = self.stats.validation_refusals.clone();
            self.campaigns
                .insert(engine_allocation::string(id)?, campaign)?;
        }
        Ok(self.campaigns.get_mut(id).unwrap())
    }

    fn error(&mut self, error: &io::Error) {
        self.stats.errors += 1;
        self.stats
            .last_error
            .get_or_insert_with(|| error_reason(error));
    }

    pub fn observe_input_demand_error(&mut self, reason: &'static str) {
        self.observe_input_demand_errors(1, reason);
    }

    pub fn observe_input_demand_errors(&mut self, count: u64, last: &'static str) {
        self.stats.input_demand_errors = self.stats.input_demand_errors.saturating_add(count);
        self.stats.input_demand_last_error = Some(last);
    }

    pub fn observe_input_demand<T>(&mut self, result: Result<T, &'static str>) -> Option<T> {
        match result {
            Ok(demand) => Some(demand),
            Err(reason) => {
                self.observe_input_demand_error(reason);
                None
            }
        }
    }

    /// # Safety
    /// Only the active spool may prepare its own capture frame. Use a frame from this session and
    /// complete preparation before recording it or returning its slot.
    pub unsafe fn prepare(&mut self, captured: &Captured) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        collector_memory::terminal::set_context(Default::default());
        let clock = captured.clock.as_ref();
        let ids = captured
            .dropped_before
            .iter()
            .map(|loss| {
                collector_memory::terminal::set_context(collector_memory::terminal::Context {
                    campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                    branch: None,
                    load_signal: loss.load_signal,
                    failed_hour: Some(loss.first_hour),
                });
                loss.campaign_id.as_bytes()
            })
            .chain(clock.into_iter().map(|clock| {
                collector_memory::terminal::set_context(collector_memory::terminal::Context {
                    campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
                    branch: None,
                    load_signal: Some(clock.play_session_started_at),
                    failed_hour: Some(clock.hour),
                });
                clock.campaign_id.as_bytes()
            }));
        let result = prepare_campaigns(ids, |id| self.campaign(id).map(|_| ()));
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    pub fn prepare_losses(&mut self, losses: &LossBatch) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Teardown);
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        collector_memory::terminal::set_context(Default::default());
        let result = prepare_campaigns(
            losses.iter().map(|loss| {
                collector_memory::terminal::set_context(collector_memory::terminal::Context {
                    campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                    branch: None,
                    load_signal: loss.load_signal,
                    failed_hour: Some(loss.first_hour),
                });
                loss.campaign_id.as_bytes()
            }),
            |id| self.campaign(id).map(|_| ()),
        );
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    /// # Safety
    /// Only the originating spool may submit a loss. The loss or count must come from that
    /// session's capture, and the spool must submit it in capture order and before releasing the
    /// recording.
    pub unsafe fn lost(&mut self, loss: &LostObservation) -> io::Result<()> {
        self.poll_writer()?;
        let _details = replay_data::codec::error_details_scope();
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
            branch: None,
            load_signal: loss.load_signal,
            failed_hour: Some(loss.first_hour),
        });
        let result = (|| {
            let id = match campaign_name(loss.campaign_id.as_bytes()) {
                Ok(id) => id,
                Err(error) => {
                    // SAFETY: The originating spool accounts for this session's own missing
                    // capture.
                    unsafe { self.unknown_loss(loss.count) };
                    return Err(error);
                }
            };
            let suppress = self.suppress_origin;
            self.campaign(&id)?.lost(loss, suppress)
        })();
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    /// # Safety
    /// Only the originating spool may submit a loss. The loss or count must come from that
    /// session's capture, and the spool must submit it in capture order and before releasing the
    /// recording.
    pub unsafe fn unknown_loss(&mut self, count: u64) {
        self.unknown_pending = self.unknown_pending.saturating_add(count);
        self.suppress_origin |= count != 0;
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Only the session's spool may submit a frame. It must be the matching hour's fully decoded
    /// frame, owned by this spool until conversion is complete.
    pub unsafe fn frame(&mut self, captured: &Captured) -> io::Result<()> {
        // SAFETY: The isolated fixture owns the decoded frame and processor.
        unsafe { self.frame_before_publication(captured, || {}) }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// The isolated fixture owns the decoded frame, its processor and any injected failure.
    pub unsafe fn frame_before_publication(
        &mut self,
        captured: &Captured,
        before_publication: impl FnOnce(),
    ) -> io::Result<()> {
        let prepared =
        // SAFETY: This is the decoded frame held by its originating spool.
        unsafe { self.prepare_frame(captured) }?;
        self.write_prepared_with(prepared, before_publication)?;
        self.drain_writer()?;
        match self.writer_error.take() {
            Some(kind) => Err(kind.into()),
            None => Ok(()),
        }
    }

    /// # Safety
    /// Only the session's spool may submit a frame. It must be the matching hour's fully decoded
    /// frame, owned by this spool until conversion is complete.
    pub unsafe fn prepare_frame(&mut self, captured: &Captured) -> io::Result<PreparedFrame> {
        self.poll_writer()?;
        if let Some(clock) = captured.clock.as_ref()
            && let Ok(id) = campaign_name(clock.campaign_id.as_bytes())
        {
            let boundary = self
                .flight
                .as_ref()
                .is_some_and(|(previous, _)| previous != &id)
                || self.campaigns.get_mut(&id).is_some_and(|campaign| {
                    !campaign.spool.is_open()
                        || campaign
                            .world_generation
                            .is_some_and(|previous| previous != captured.world_generation)
                        || campaign
                            .tracker
                            .last_load_signal()
                            .is_some_and(|load| load != clock.play_session_started_at)
                        || campaign
                            .tracker
                            .last_observation()
                            .is_some_and(|(_, hour)| hour.checked_add(1) != Some(clock.hour))
                });
            if boundary {
                self.set_aside_others(&id);
                if let Err(error) = self.set_aside_campaign(&id) {
                    self.error(&error);
                }
                self.poll_writer()?;
            }
        }
        let writer_epoch = loop {
            self.poll_writer()?;
            if let Some(epoch) = self.writer.ready_epoch()? {
                break epoch;
            }
        };
        let _details = replay_data::codec::error_details_scope();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { collector_workers::spool_phases::SPOOL
            .span(collector_workers::spool_phases::Phase::Observe) };
        if let Some(clock) = captured.clock.as_ref() {
            collector_memory::terminal::set_context(collector_memory::terminal::Context {
                campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
                load_signal: Some(clock.play_session_started_at),
                failed_hour: Some(clock.hour),
                ..Default::default()
            });
        } else {
            collector_memory::terminal::set_context(Default::default());
        }
        let mut branch = None;
        let result = self
            .prepare_frame_inner(captured, &mut branch, span)
            .map(|mut prepared| {
                prepared.writer_epoch = writer_epoch;
                prepared
            });
        self.finish_preparation(captured, branch, result)
    }

    fn finish_preparation(
        &mut self,
        captured: &Captured,
        branch: Option<BranchId>,
        result: io::Result<PreparedFrame>,
    ) -> io::Result<PreparedFrame> {
        if let Err(error) = &result {
            self.error(error);
            self.drain_writer()?;
            if error.kind() != io::ErrorKind::OutOfMemory
                && !collector_memory::terminal::failed()
                && let (Some(branch), Some(clock)) = (branch, &captured.clock)
            {
                let id = std::str::from_utf8(clock.campaign_id.as_bytes())
                    .map_err(|_| invalid("prepared campaign has invalid identity"))?;
                let campaign = self.campaigns.get_mut(id).unwrap();
                if campaign.spool.is_open() {
                    let _ = campaign.spool.finish_segment();
                }
                self.poll_writer()?;
                let campaign = self.campaigns.get_mut(id).unwrap();
                campaign.reset_after_write_failure();
                campaign.gap(
                    branch,
                    clock.hour,
                    clock.hour,
                    engine_allocation::string("spool frame preparation failed")?,
                )?;
                self.retry_full = true;
            }
        }
        result
    }

    fn prepare_frame_inner(
        &mut self,
        captured: &Captured,
        bound_branch: &mut Option<BranchId>,
        span: collector_workers::spool_phases::Span<'static>,
    ) -> io::Result<PreparedFrame> {
        let Some(clock) = captured.clock.as_ref() else {
            // SAFETY: The originating spool accounts for this session's own missing capture.
            unsafe { self.unknown_loss(1) };
            return Err(invalid(
                "entry clock unreadable; campaign and hour cannot be recorded",
            ));
        };
        let id = match campaign_name(clock.campaign_id.as_bytes()) {
            Ok(id) => id,
            Err(error) => {
                // SAFETY: The originating spool accounts for this session's own missing capture.
                unsafe { self.unknown_loss(1) };
                return Err(error);
            }
        };
        let suppress = self.suppress_origin;
        let unknown = self.unknown_pending;
        self.campaign(&id)?;
        let helper = (!self.helpers.is_empty()).then(|| HelperSplit {
            helpers: collector_workers::spool_helper::Helpers {
                pool: &self.helpers,
                backlogged: self.backlogged,
            },
            plan: self.stats.conversion.plan(self.helpers.len()),
        });
        let campaign = self
            .campaigns
            .get_mut(&id)
            .expect("an opened campaign remains admitted");
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            branch: None,
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(clock.hour),
        });
        let origin = if suppress || campaign.tracker.current_branch().is_none() {
            None
        } else {
            Some(clock.hour)
        };
        if campaign
            .world_generation
            .is_some_and(|previous| previous != captured.world_generation)
        {
            campaign.tracker.world_changed();
        }
        campaign.world_generation = Some(captured.world_generation);
        let branch = campaign.observe(origin, clock.hour, Some(clock.play_session_started_at))?;
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            branch: Some(branch),
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(clock.hour),
        });
        *bound_branch = Some(branch);
        self.command_contexts.insert(
            captured.world_generation,
            (engine_allocation::string(&id)?, branch),
        );
        if let Ok(qpc) = u64::try_from(captured.qpc)
            && qpc != 0
        {
            self.command_anchors.insert(
                (captured.world_generation, branch),
                (qpc, clock.hour, clock.play_session_started_at),
            );
        }
        let force = !campaign.spool.is_open();
        let variables_baseline = collector_intel::intel::VariableBaselines {
            scopes: campaign
                .encoder
                .slice_baseline(TableId::ScriptVariables, branch, clock.hour, force)
                .and_then(|(token, rows)| Some((token, rows.rows::<tables::ScriptVariables>()?))),
            names: campaign
                .encoder
                .slice_baseline(TableId::ScriptVariableNames, branch, clock.hour, force)
                .and_then(|(token, rows)| {
                    Some((token, rows.rows::<tables::ScriptVariableNames>()?))
                }),
            values: campaign
                .encoder
                .slice_baseline(TableId::ScriptVariableValues, branch, clock.hour, force)
                .and_then(|(token, rows)| {
                    Some((token, rows.rows::<tables::ScriptVariableValues>()?))
                }),
        };
        if captured.force_full {
            for (_, table) in REPUBLISHING {
                campaign.projection_scratch.forget(table);
            }
        }
        let sliced_skip = {
            let can_skip_carried = |table: TableId| {
                campaign.owner_sliced(table)
                    && campaign.sliced_branch == Some(branch)
                    && campaign.sliced_cache.get(&table).is_none()
                    && campaign.sliced_owners[table.index()].is_some()
                    && campaign.columnar.has_complete(table, branch, clock.hour)
                    && captured
                        .extended
                        .slice_selection(table)
                        .is_some_and(|selection| !selection.force_full)
                    && campaign
                        .encoder
                        .untracked_slice_token(table, branch, clock.hour, force)
                        .is_some()
            };
            let has_research_carried = captured.extended.research_carried_owners().is_some();
            let has_technology_carried = captured.extended.technology_carried_owners().is_some();
            collector_projection::projection::SlicedSkip {
                research_bonuses_carried: has_research_carried
                    && can_skip_carried(TableId::ResearchAppliedBonuses),
                research_modifiers_carried: has_research_carried
                    && can_skip_carried(TableId::ResearchModifiers),
                technologies_carried: has_technology_carried
                    && can_skip_carried(TableId::ResearchTechnologies),
            }
        };
        let mut characters = CharacterConversion::new(
            branch,
            &mut campaign.character_shadow,
            character_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
        );
        let mut lexical = LexicalConversion::new(
            lexical_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
            &mut campaign.lexical_conversion,
        );
        let mut intel = IntelConversion::new(branch, clock.hour, &mut campaign.intel_shadow);
        let mut projection = Projection::new(
            &campaign.encoder,
            branch,
            clock.hour,
            force,
            &campaign.projection_scratch,
        )
        .with_sliced_skip(sliced_skip);
        span.finish();
        let (mut source, mut costs) = decoded_with_costs(
            captured,
            clock.borrowed(),
            branch,
            &mut SpoolConversion {
                variables_baseline,
                variables_scratch: &mut campaign.variables_scratch,
                characters: Some(&mut characters),
                lexical: Some(&mut lexical),
                intel: Some(&mut intel),
                projection: Some(&mut projection),
                helper,
            },
        );
        costs.script_variables = campaign.variables_scratch.take_work();
        self.stats.conversion.record(&costs);
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { collector_workers::spool_phases::SPOOL
            .span(collector_workers::spool_phases::Phase::Merge) };
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let outcomes = captured.extended.outcomes();
        let (sliced_retry, unpublished_families) = campaign.try_merge_sliced_inputs(
            &mut source,
            |table| captured.extended.slice_selection(table),
            |table| {
                matches!(
                    outcomes[table.index()],
                    Outcome::Rows { .. } | Outcome::Truncated { .. }
                )
            },
            |table, owner| match table {
                TableId::ResearchAppliedBonuses => {
                    sliced_skip.research_bonuses_carried
                        && captured
                            .extended
                            .research_carried_owners()
                            .is_some_and(|owners| owners.binary_search(&owner).is_ok())
                }
                TableId::ResearchModifiers => {
                    sliced_skip.research_modifiers_carried
                        && captured
                            .extended
                            .research_carried_owners()
                            .is_some_and(|owners| owners.binary_search(&owner).is_ok())
                }
                TableId::ResearchTechnologies => {
                    sliced_skip.technologies_carried
                        && captured
                            .extended
                            .technology_carried_owners()
                            .is_some_and(|owners| owners.binary_search(&owner).is_ok())
                }
                _ => false,
            },
        )?;
        campaign.assemble_line_output(&mut source, captured);
        let event_feed_publication = captured.extended.event_feed().prepare_pack()?;
        if let Some(publication) = &event_feed_publication {
            let mut rows = engine_allocation::vec(1)?;
            rows.push(replay_data::rows::RowCodec::try_clone(publication.row())?);
            source.tables[TableId::EventFeedHistory.index()] = TableInput::Sampled(
                TableView::Complete(replay_data::rows::TableRows::EventFeedHistory(rows)),
            );
        }
        let operation_publication =
        // SAFETY: The spool assembles its own frame's interval while the originating observer is
        // live.
        unsafe { campaign
            .operation_outcomes
            .assemble_owned(
            // SAFETY: The spool borrows its own frame and keeps the operation log live to
            // publication.
             captured.extended.operation_outcomes() , &mut source.tables) };
        let map_event_publication =
        // SAFETY: The spool assembles its own frame's interval while the originating observer is
        // live.
        unsafe { campaign
            .map_events
            .assemble_owned(
            // SAFETY: The spool borrows its own frame for assembly before releasing event storage.
             captured.extended.map_events() , &mut source.tables) };
        if collector_memory::terminal::failed() {
            return Err(collector_memory::terminal::oom());
        }
        // SAFETY: The spool assembles its own frame's interval while the originating observer is
        // live.
        unsafe {
            campaign.politics.assemble_owned(
                source.branch,
                source.hour,
                // SAFETY: The spool borrows the political interval belonging to its decoded frame.
                captured.extended.politics(),
                &captured.countries,
                captured.countries_result,
                &mut source.tables,
            )
        }?;
        campaign.assemble_construction_ic(&mut source, captured);
        campaign.fuel_awards.assemble(
            source.branch,
            source.hour,
            captured.extended.fuel_awards(),
            &mut source.tables,
        )?;
        let retry = retry_table(&source.tables);
        if let Some(table) = retry {
            // SAFETY: The sole spool publishes the retry cause for its own frame.
            unsafe { retry_cause() }.store(
                table.index() as u16 + 1,
                std::sync::atomic::Ordering::Relaxed,
            );
        } else if sliced_retry {
            // SAFETY: The sole spool publishes the retry cause for its own frame.
            unsafe { retry_cause() }.store(RETRY_SLICED, std::sync::atomic::Ordering::Relaxed);
        }
        let retry_full = retry.is_some() || sliced_retry;
        campaign
            .try_apply_staggered_inputs(&mut source, captured.extended.country_intel_sample())?;
        campaign.assemble_reserved_intel_sample(&mut source);
        for table in [
            TableId::ProductionLineOutput,
            TableId::OperationCompletions,
            TableId::MapEvents,
            TableId::ConstructionLineOutput,
            TableId::CountryFuel,
            TableId::ReservedCountryNaming,
        ]
        .into_iter()
        .chain(collector_politics::politics::TABLES)
        {
            source.tables[table.index()].round_to_display();
        }
        if let Some(layout) = &captured.fixed_layout {
            collector_metadata::fixed_layout::apply(&mut source.tables, layout)?;
        }
        let work = WorkSample::default();
        let span = span.switch(collector_workers::spool_phases::Phase::Statistics);
        let sampled = CapturedStatistics::capture(captured)?;
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let clock = clock.copy_owned()?;
        span.finish();
        Ok(PreparedFrame {
            writer_epoch: 0,
            source,
            id,
            clock,
            qpc: captured.qpc,
            unknown,
            retry_full,
            unpublished_families,
            work,
            sampled,
            operation_publication,
            map_event_publication,
            event_feed_publication,
        })
    }

    /// # Safety
    /// Submit exactly once to the same processor that prepared the frame, in capture order and
    /// before the associated observer logs are released.
    pub unsafe fn write_prepared(&mut self, prepared: PreparedFrame) -> io::Result<()> {
        self.write_prepared_with(prepared, || {})
    }

    fn write_prepared_with(
        &mut self,
        prepared: PreparedFrame,
        before_publication: impl FnOnce(),
    ) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
        if collector_memory::terminal::failed() {
            if let Some(publication) = &prepared.event_feed_publication {
                // SAFETY: Originating spool discards an uncommitted publication on terminal OOM.
                unsafe {
                    collector_observers::event_feed_observer::discarded(
                    publication.publication(),
                    publication.row().generation,
                );
                }
            }
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let PreparedFrame {
            writer_epoch: epoch,
            mut source,
            id,
            clock,
            qpc,
            unknown,
            retry_full,
            unpublished_families,
            mut work,
            sampled,
            operation_publication,
            map_event_publication,
            event_feed_publication,
        } = prepared;
        let event_feed_publication = PublicationReceipt(event_feed_publication);
        self.unpublished_families |= unpublished_families;
        let branch = source.branch;
        self.queued
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        self.account_ledger();
        let flight_id = engine_allocation::string(&id)?;
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            branch: Some(branch),
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(clock.hour),
        });
        let campaign = self
            .campaigns
            .get_mut(&id)
            .expect("prepared campaign remains admitted");
        if let Some(reason) = campaign.columnar.halted() {
            let reason = engine_allocation::string(reason)?;
            campaign.gap(branch, clock.hour, clock.hour, reason)?;
            return self.drain_writer();
        }
        let helpers =
            (!self.helpers.is_empty()).then(|| collector_workers::spool_helper::Helpers {
                pool: &self.helpers,
                backlogged: self.backlogged,
            });
        collector_hook_failures::apply(&mut source)?;
        crate::site_withholding::apply(&mut source)?;
        crate::spool_compare::compare_frame(
            &campaign.encoder,
            crate::spool_compare::Target {
                branch,
                hour: source.hour,
                restarting: !campaign.spool.is_open(),
            },
            &mut source,
            helpers,
            &self.comparison,
        );
        let result = campaign.write_with(
            source,
            (qpc, Some(clock.play_session_started_at)),
            unknown,
            &mut work,
            helpers,
            before_publication,
        );
        let _span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { collector_workers::spool_phases::SPOOL
            .span(collector_workers::spool_phases::Phase::Commit) };

        if let Err(error) = &result {
            self.error(error);
        }
        if result.is_err() {
            if collector_memory::terminal::failed() {
                return result;
            }
            self.drain_writer()?;
            let campaign = self.campaigns.get_mut(&id).unwrap();
            campaign.reset_after_write_failure();
            if !collector_memory::terminal::failed() {
                campaign.gap(
                    branch,
                    clock.hour,
                    clock.hour,
                    engine_allocation::string("spool write failed")?,
                )?;
                self.retry_full = true;
            }
            return result;
        }
        let campaign = self.campaigns.get_mut(&id).unwrap();
        let mut frame = engine_allocation::boxed(QueuedFrame {
            storage: None,
            ticket: 0,
            context: std::mem::take(&mut campaign.submitted_context),
            unknown,
            id,
            branch,
            clock,
            qpc,
            sampled,
            operation_publication,
            map_event_publication,
            event_feed_publication,
            viewed: work.viewed.take(),
            line_output_receipt: work.line_output_receipt,
            construction_ic_receipt: work.construction_ic_receipt,
        })?;
        let receipt_bytes = frame.storage_bytes();
        let write_bytes = work
            .staged
            .as_ref()
            .expect("queued frame was staged")
            .admission_bytes()?;
        let result = (|| {
            frame.storage = Some(self.receipt_storage(receipt_bytes, epoch, write_bytes)?);
            let campaign = self.campaigns.get_mut(&frame.id).unwrap();
            campaign
                .tracker
                .admit_hour(frame.branch, frame.clock.hour)?;
            campaign.spool.write_frame(
                work.staged.take().expect("queued frame was staged"),
                receipt_bytes,
            )
        })();
        if let Err(error) = &result {
            if collector_memory::terminal::failed() {
                return result;
            }
            self.error(error);
            self.drain_writer()?;
            let campaign = self.campaigns.get_mut(&frame.id).unwrap();
            campaign.lose_hour(frame.branch, frame.clock.hour)?;
            campaign.restore_context(std::mem::take(&mut frame.context))?;
            campaign.reset_after_write_failure();
            campaign.gap(
                frame.branch,
                frame.clock.hour,
                frame.clock.hour,
                engine_allocation::string("spool write failed")?,
            )?;
            self.retry_full = true;
            return result;
        }
        collector_memory::terminal::attempted_hour(None);
        if work.validation_refused {
            // SAFETY: The sole spool publishes the retry cause for its own frame.
            unsafe { retry_cause() }.store(RETRY_VALIDATION, std::sync::atomic::Ordering::Relaxed);
        }
        self.retry_full |= retry_full || work.validation_refused;
        self.unpublished_families |= work.validation_families;
        self.unknown_pending = 0;
        self.suppress_origin = false;
        frame.ticket = self
            .campaigns
            .get_mut(&frame.id)
            .unwrap()
            .spool
            .last_ticket()
            .expect("processor always uses its collector writer");
        if let Some(publication) = &frame.operation_publication {
            // SAFETY: This is the spool's actual queue admission while its originating observer
            // remains live.
            unsafe { publication.admitted() };
        }
        if let Some(publication) = &frame.map_event_publication {
            // SAFETY: This is the spool's actual queue admission while its originating observer
            // remains live.
            unsafe { publication.admitted() };
        }
        self.queued.push_back(frame);
        self.flight = Some((flight_id, branch));
        Ok(())
    }

    fn complete_frame(&mut self, frame: Box<QueuedFrame>) -> io::Result<()> {
        let QueuedFrame {
            storage: _storage,
            id,
            branch,
            clock,
            qpc,
            sampled,
            operation_publication,
            map_event_publication,
            mut event_feed_publication,
            viewed,
            line_output_receipt,
            construction_ic_receipt,
            ..
        } = *frame;
        if let Some(publication) = event_feed_publication.0.take() {
            // SAFETY: Originating spool resolves a writer-confirmed event-feed publication receipt.
            unsafe { collector_observers::event_feed_observer::accepted(&publication)? };
        }
        if let Some(publication) = operation_publication {
            // SAFETY: The originating spool has the writer receipt and still owns the observers.
            unsafe { publication.commit() };
        }
        if let Some(publication) = map_event_publication {
            // SAFETY: The originating spool has the writer receipt and still owns the observers.
            unsafe { publication.commit() };
        }
        let campaign = self.campaigns.get_mut(&id).unwrap();
        campaign.line_output.written(line_output_receipt);
        campaign.construction_ic.written(construction_ic_receipt);
        campaign.tracker.record_hour(branch, clock.hour)?;
        campaign.columnar.complete(branch, clock.hour);
        #[cfg(feature = "alloc-fault-injection")]
        collector_memory::terminal::committed_hour();
        let stats = &mut self.stats;
        stats.hours += 1;
        stats.campaign = Some(engine_allocation::string(&id)?);
        stats.last_hour = Some(clock.hour);
        stats.player = viewed;
        sampled.record(stats)?;
        if let Some((old_id, old_branch, hour, previous_qpc)) = &stats.previous
            && old_id == &id
            && *old_branch == branch
            && hour.checked_add(1) == Some(clock.hour)
            && qpc > *previous_qpc
        {
            stats.interval_hours += 1;
            stats.interval_qpc += (qpc - previous_qpc) as u64;
        }
        stats.previous = Some((id, branch, clock.hour, qpc));
        Ok(())
    }

    fn poll_writer(&mut self) -> io::Result<()> {
        let caller = collector_memory::terminal::context();
        let result = self.poll_writer_inner();
        self.account_ledger();
        if result.is_err() && !collector_memory::terminal::failed() {
            collector_memory::terminal::fail(
                collector_memory::terminal::Phase::Spool,
                collector_memory::terminal::Code::Thread,
            );
        }
        collector_memory::terminal::set_context(caller);
        result
    }

    fn poll_writer_inner(&mut self) -> io::Result<()> {
        let mut lost = false;
        let mut failed = false;
        while let Some(completion) = self.writer.completion() {
            collector_memory::terminal::set_context(completion.context);
            if let Some(mut write) = self.command_inflight.remove(&completion.ticket) {
                if completion.result.is_ok() {
                    if let Some(campaign) = self.campaigns.get_mut(&write.id) {
                        campaign.columnar.note(std::slice::from_ref(&write.record));
                    }
                } else {
                    write.storage.take();
                    self.command_ready
                        .insert((write.world, write.ordinal), write);
                }
            }
            if completion.frame {
                if self
                    .queued
                    .front()
                    .is_none_or(|frame| frame.ticket != completion.ticket)
                {
                    return Err(invalid("writer completions are out of order"));
                }
                let mut frame = self.queued.pop_front().unwrap();
                if completion.committed {
                    collector_memory::terminal::attempted_hour(None);
                    self.complete_frame(frame)?;
                } else {
                    lost = true;
                    self.unknown_pending = self.unknown_pending.saturating_add(frame.unknown);
                    self.suppress_origin |= frame.unknown != 0;
                    let campaign = self.campaigns.get_mut(&frame.id).unwrap();
                    campaign.lose_hour(frame.branch, frame.clock.hour)?;
                    campaign.restore_context(std::mem::take(&mut frame.context))?;
                    campaign.gap(
                        frame.branch,
                        frame.clock.hour,
                        frame.clock.hour,
                        engine_allocation::string("spool write failed")?,
                    )?;
                }
            }
            if let Some(id) = completion.context.campaign_id {
                let id = std::str::from_utf8(&id)
                    .map_err(|_| invalid("writer completion has invalid campaign identity"))?;
                if let Some(campaign) = self.campaigns.get_mut(id)
                    && let Some(index) = campaign
                        .closing
                        .iter()
                        .position(|(receipt, _, _)| receipt.ticket == completion.ticket)
                {
                    let (_, context, _storage) = campaign.closing.remove(index);
                    if completion.result.is_err() {
                        campaign.restore_context(context)?;
                    }
                }
            }
            if let Err(error) = completion.result {
                failed = true;
                self.error(&error);
                self.writer_error.get_or_insert(error.kind());
                if let Some(id) = completion.context.campaign_id {
                    let id = std::str::from_utf8(&id)
                        .map_err(|_| invalid("writer completion has invalid campaign identity"))?;
                    if let Some(campaign) = self.campaigns.get_mut(id) {
                        campaign.reset_after_write_failure();
                    }
                }
                self.retry_full = true;
            }
        }
        if lost {
            // SAFETY: The sole spool has invalidated pending writes and still owns its observer
            // frontier.
            unsafe { collector_operation_outcomes::extended::operation_outcomes::reset_admitted() };
            // SAFETY: The sole spool has invalidated pending writes and still owns its observer
            // frontier.
            unsafe { collector_capture::extended::map_events::reset_admitted() };
        }
        if failed {
            self.writer.resume_after_failure();
        }
        Ok(())
    }

    fn drain_writer(&mut self) -> io::Result<()> {
        self.writer.drain()?;
        self.poll_writer()
    }

    fn set_aside_others(&mut self, current: &str) {
        if collector_memory::terminal::failed() {
            return;
        }
        let mut first_error = None;
        for index in 0..self.campaigns.0.len() {
            if self.campaigns.0[index].0 == current {
                continue;
            }
            let result = engine_allocation::string(&self.campaigns.0[index].0)
                .and_then(|id| self.set_aside_campaign(&id));
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        self.account_ledger();
        if let Some(error) = first_error {
            self.error(&error);
        }
    }

    #[cfg(test)]
    fn close_campaign(
        &mut self,
        id: &str,
        load: Option<i64>,
    ) -> Result<crate::spool_writer::CloseReceipt, &'static str> {
        if collector_memory::terminal::failed() {
            return Err("allocation failed");
        }
        let Some(campaign) = self.campaigns.get_mut(id) else {
            let receipt = crate::spool_writer::CloseReceipt::default();
            // SAFETY: This is the close result from its originating writer, or an isolated receipt
            // owned by the fixture.
            unsafe { receipt.complete(Ok(())) };
            return Ok(receipt);
        };
        if load.is_some_and(|load| {
            campaign
                .tracker
                .last_load_signal()
                .is_some_and(|now| now != load)
        }) {
            return Err("the campaign was loaded again");
        }
        let storage = self.close_storage(id).map_err(|error| {
            self.error(&error);
            "its queued context could not be reserved"
        })?;
        let campaign = self.campaigns.get_mut(id).unwrap();
        let caller = collector_memory::terminal::context();
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: id.as_bytes().try_into().ok(),
            branch: campaign.tracker.current_branch(),
            load_signal: campaign.tracker.last_load_signal(),
            failed_hour: None,
        });
        let result = campaign.queue_close(storage);
        collector_memory::terminal::set_context(caller);
        self.account_ledger();
        result.map_err(|error| {
            self.error(&error);
            "its segment could not be closed"
        })
    }

    /// # Safety
    /// Only the session's spool may move the lease for a campaign, after its matching history
    /// request is clear of capture and writer work.
    pub unsafe fn exchange_history_lease(
        &mut self,
        id: &str,
        mut file: Option<File>,
    ) -> Option<File> {
        match self.campaigns.get_mut(id) {
            Some(campaign) => {
                std::mem::swap(&mut campaign.history_lease, &mut file);
                file
            }
            None => self.history.exchange_lease(id, file),
        }
    }

    /// # Safety
    /// The sole spool must supply the originating session's history fence and matching pool slots.
    /// A close or release must wait for all captures through that fence and their publications.
    pub unsafe fn try_history_request(
        &mut self,
        request: &str,
    ) -> io::Result<Option<(HistoryReply, Option<File>)>> {
        let mut words = request.split_whitespace();
        let (Some(token), Some(action), Some(id)) = (words.next(), words.next(), words.next())
        else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        let id = campaign_name(id.as_bytes())?;
        let mut lease = None;
        let refusal = match action {
            "release"
                if self
                    .campaigns
                    .get_mut(&id)
                    .is_some_and(|campaign| campaign.spool.is_open()) =>
            {
                Some("the campaign is recording")
            }
            "release" | "lend" => {
                lease =
                // SAFETY: The originating spool has cleared this request through its capture and
                // writer fence.
                unsafe { self.exchange_history_lease(&id, None) };
                None
            }
            "close" => {
                let load = words.next().and_then(|word| word.parse::<i64>().ok());
                let receipt = match self.try_close_campaign(&id, load) {
                    Ok(Some(receipt)) => receipt,
                    Ok(None) => return Ok(None),
                    Err(error) => {
                        return Ok(Some((
                            HistoryReply::Ready(format!("{token} refused {error}")),
                            None,
                        )));
                    }
                };
                return Ok(Some((
                    HistoryReply::Closing {
                        token: token.to_owned(),
                        receipt,
                    },
                    None,
                )));
            }
            _ => Some("unknown request"),
        };
        let reply = match refusal {
            Some(reason) => format!("{token} refused {reason}"),
            None => format!("{token} ok"),
        };
        Ok(Some((HistoryReply::Ready(reply), lease)))
    }

    fn try_close_campaign(
        &mut self,
        id: &str,
        load: Option<i64>,
    ) -> io::Result<Option<crate::spool_writer::CloseReceipt>> {
        let Some(campaign) = self.campaigns.get_mut(id) else {
            let receipt = crate::spool_writer::CloseReceipt::default();
            // SAFETY: This is the close result from its originating writer, or an isolated receipt
            // owned by the fixture.
            unsafe { receipt.complete(Ok(())) };
            return Ok(Some(receipt));
        };
        if load.is_some_and(|load| {
            campaign
                .tracker
                .last_load_signal()
                .is_some_and(|now| now != load)
        }) {
            return Err(invalid("the campaign was loaded again"));
        }
        let storage = if campaign.close_storage_bytes() != 0 {
            let Some(storage) = self.writer.try_receipt_storage(
                campaign.close_storage_bytes(),
                size_of::<(
                    crate::spool_writer::CloseReceipt,
                    Vec<PendingBranch>,
                    Option<crate::spool_writer::ReceiptStorage>,
                )>(),
                0,
            )?
            else {
                return Ok(None);
            };
            Some(storage)
        } else {
            None
        };
        let restarting = !campaign.spool.is_open()
            && campaign
                .pending
                .last()
                .is_some_and(|pending| !pending.gaps.is_empty());
        let context_submitted = campaign.spool.is_open() || restarting;
        let context = if context_submitted {
            campaign.context(restarting, 0)?
        } else {
            Vec::new()
        };
        if context_submitted && !campaign.pending.is_empty() {
            engine_allocation::reserve(&mut campaign.closing, 1)?;
        }
        let caller = collector_memory::terminal::context();
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: id.as_bytes().try_into().ok(),
            branch: campaign.tracker.current_branch(),
            load_signal: campaign.tracker.last_load_signal(),
            failed_hour: None,
        });
        campaign.columnar.note(&context);
        let result = campaign.spool.try_close_context(context, restarting);
        collector_memory::terminal::set_context(caller);
        let receipt = match result {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            result => result?,
        };
        if context_submitted && !campaign.pending.is_empty() {
            campaign.closing.push((
                receipt.clone(),
                std::mem::take(&mut campaign.pending),
                storage,
            ));
        }
        self.account_ledger();
        Ok(Some(receipt))
    }

    #[cfg(test)]
    pub(crate) fn release_history(&mut self, id: &str) -> Result<(), &'static str> {
        match self.campaigns.get_mut(id) {
            Some(campaign) if campaign.spool.is_open() => return Err("the campaign is recording"),
            Some(campaign) => campaign.history_lease = None,
            None => self.history.release(id),
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn lend_history(&mut self, id: &str) {
        match self.campaigns.get_mut(id) {
            Some(campaign) => campaign.history_lease = None,
            None => self.history.release(id),
        }
    }

    #[cfg(test)]
    pub(crate) fn begin_history_request(&mut self, request: &str) -> HistoryReply {
        let mut words = request.split_whitespace();
        let (token, action, campaign) = (words.next(), words.next(), words.next());
        let load = words.next().and_then(|load| load.parse::<i64>().ok());
        let refusal = match (
            campaign.map(|campaign| campaign_name(campaign.as_bytes())),
            action,
        ) {
            (Some(Ok(campaign)), Some("release")) => self.release_history(&campaign).err(),
            (Some(Ok(campaign)), Some("lend")) => {
                self.lend_history(&campaign);
                None
            }
            (Some(Ok(campaign)), Some("close")) => match self.close_campaign(&campaign, load) {
                Ok(receipt) => {
                    return HistoryReply::Closing {
                        token: token.unwrap_or_default().to_owned(),
                        receipt,
                    };
                }
                Err(reason) => Some(reason),
            },
            (Some(Ok(campaign)), Some("retake")) => self
                .retake_history(&campaign)
                .err()
                .map(|_| "the history file could not be held"),
            _ => Some("unknown request"),
        };
        let token = token.unwrap_or_default();
        HistoryReply::Ready(match refusal {
            None => format!("{token} ok"),
            Some(reason) => format!("{token} refused {reason}"),
        })
    }

    #[cfg(test)]
    fn history_request(&mut self, request: &str) -> String {
        match self.begin_history_request(request) {
            HistoryReply::Ready(reply) => reply,
            HistoryReply::Closing { token, receipt } => {
                self.drain_writer().unwrap();
                match receipt.result().unwrap() {
                    Ok(()) => format!("{token} ok"),
                    Err(_) => format!("{token} refused its segment could not be closed"),
                }
            }
        }
    }

    #[cfg(test)]
    fn settle_journals(&self) {
        for campaign in self.campaigns.values() {
            campaign.columnar.settle();
        }
    }

    #[cfg(test)]
    pub(crate) fn retake_history(&mut self, id: &str) -> io::Result<()> {
        let lease = match self.history.history_path(id)? {
            Some(path) => Some(crate::history::lease(&path)?),
            None => None,
        };
        match self.campaigns.get_mut(id) {
            Some(campaign) => campaign.history_lease = lease,
            None => self.history.retake(id, lease),
        }
        Ok(())
    }

    /// # Safety
    /// Only the session's spool may drain its live command installation. Request the final drain
    /// only after hooks are restored and their callbacks are quiet.
    pub unsafe fn flush_commands(&mut self, final_drain: bool) -> io::Result<()> {
        if !final_drain
            && self.command_last_flush.elapsed() < std::time::Duration::from_secs(5)
            && !
            // SAFETY: The spool queries its command installation before runtime releases the
            // store.
            unsafe { collector_player_commands::player_commands::urgent() }
        {
            return Ok(());
        }
        // SAFETY: The sole spool drains its current installation, with hooks quiet for a final
        // drain.
        unsafe {
            self.command_drain
                .take_into(final_drain, &mut self.command_pending)
        }?;
        self.command_last_flush = std::time::Instant::now();
        if !self.session_verified {
            return if final_drain && !self.command_pending.is_empty() {
                Err(invalid("command capture session content was not verified"))
            } else {
                Ok(())
            };
        }
        let mut index = 0;
        while index < self.command_pending.len() {
            let pending = &self.command_pending[index];
            if !self.command_contexts.contains_key(&pending.world)
                && let Some(context) = pending.context
            {
                let world = pending.world;
                let id = campaign_name(&context.campaign)?;
                let campaign = self.campaign(&id)?;
                let branch = if campaign
                    .world_generation
                    .is_some_and(|current| current > world)
                {
                    campaign.tracker.past_commands(context.hour)?
                } else {
                    if campaign
                        .world_generation
                        .is_some_and(|previous| previous != world)
                    {
                        campaign.tracker.world_changed();
                    }
                    let branch = campaign
                        .tracker
                        .observe_commands(
                            context.hour,
                            replay_recording::command_packet::load_marker(context.load_signal),
                        )?
                        .branch;
                    campaign.world_generation = Some(world);
                    branch
                };
                self.command_contexts.insert(world, (id, branch));
                self.command_anchors.insert(
                    (world, branch),
                    (context.qpc, context.hour, context.load_signal),
                );
            }
            let drained = &mut self.command_pending[index];
            let Some((id, branch)) = self.command_contexts.get(&drained.world) else {
                index += 1;
                continue;
            };
            let campaign = self
                .campaigns
                .get_mut(id)
                .ok_or_else(|| invalid("command campaign context disappeared"))?;
            let ordinal = self.command_ordinals.entry(drained.world).or_default();
            if *ordinal == 0 {
                drained.batch.kinds =
                // SAFETY: This catalog belongs to the command installation the spool is still
                // draining.
                unsafe { collector_player_commands::player_commands::Drain::catalog() }?;
            }
            let mut packet = replay_recording::command_packet::Packet::pack(
                *branch,
                drained.capture,
                drained.world,
                *ordinal,
                drained.from,
                drained.to,
                drained.lost,
                &drained.batch,
            )?;
            if !self.command_definitions.contains(&(drained.world, *branch))
                && let Some(context) = drained.context
            {
                let (qpc, hour, load_signal) = self
                    .command_anchors
                    .get(&(drained.world, *branch))
                    .copied()
                    .unwrap_or((context.qpc, context.hour, context.load_signal));
                packet.phase = Some(replay_recording::command_packet::Phase {
                    start: campaign
                        .tracker
                        .branch_start(*branch)
                        .ok_or_else(|| invalid("command branch context is missing"))?
                        .clone(),
                    campaign: context.campaign,
                    qpc,
                    hour,
                    load_signal,
                });
            }
            let write = CommandWrite {
                id: engine_allocation::string(id)?,
                world: drained.world,
                ordinal: *ordinal,
                record: Record::Control(ControlRecord::PlayerCommands {
                    session: self.session.id,
                    packet: Box::new(packet),
                }),
                attempted: false,
                storage: None,
            };
            *ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("command packet ordinal exhausted"))?;
            self.command_ready
                .insert((write.world, write.ordinal), write);
            self.command_definitions.insert((drained.world, *branch));
            self.command_pending.remove(index);
        }
        if final_drain {
            self.drain_writer()?;
        }
        self.submit_commands()?;
        if final_drain {
            self.drain_writer()?;
            if !self.command_pending.is_empty()
                || !self.command_ready.is_empty()
                || !self.command_inflight.is_empty()
            {
                return Err(invalid(
                    "command capture could not publish every packet before detach",
                ));
            }
        }
        Ok(())
    }

    fn submit_commands(&mut self) -> io::Result<()> {
        let keys = engine_allocation::collect_vec(self.command_ready.keys().copied())?;
        for key in keys {
            let Some(write) = self.command_ready.get(&key) else {
                continue;
            };
            if !self.campaigns.contains_key(&write.id) {
                return Err(invalid("command campaign context disappeared"));
            }
            if !write.attempted
                && let Record::Control(ControlRecord::PlayerCommands { packet, .. }) = &write.record
                && let Some(phase) = &packet.phase
                && let Some(hour) = phase.start.loaded_hour
                && self.queued.iter().any(|frame| {
                    frame.id == write.id
                        && frame.branch < phase.start.id
                        && frame.clock.hour == hour
                })
            {
                continue;
            }
            let bytes = match &write.record {
                Record::Control(ControlRecord::PlayerCommands { packet, .. }) => {
                    packet.storage_bytes()
                }
                _ => unreachable!(),
            }
            .checked_add(size_of::<CommandWrite>())
            .and_then(|n| n.checked_add(write.id.capacity()))
            .ok_or_else(engine_allocation::oom)?;
            let storage = self.receipt_storage(bytes, self.writer.failure_epoch(), bytes)?;
            if self
                .command_ready
                .range(..key)
                .any(|(&(world, _), _)| world == key.0)
            {
                continue;
            }
            let write = self
                .command_ready
                .get(&key)
                .ok_or_else(|| invalid("command write disappeared"))?;
            collector_memory::terminal::set_context(collector_memory::terminal::Context {
                campaign_id: write.id.as_bytes().try_into().ok(),
                ..Default::default()
            });
            let campaign_id = engine_allocation::string(&write.id)?;
            let branch_start = if !write.attempted
                && let Record::Control(ControlRecord::PlayerCommands { packet, .. }) = &write.record
                && packet.phase.is_some()
            {
                Some(
                    self.campaigns
                        .get_mut(&write.id)
                        .ok_or_else(|| invalid("command campaign context disappeared"))?
                        .tracker
                        .branch_start(packet.branch)
                        .ok_or_else(|| invalid("command branch context is missing"))?
                        .clone(),
                )
            } else {
                None
            };
            let mut write = self
                .command_ready
                .remove(&key)
                .ok_or_else(|| invalid("command write disappeared"))?;
            let campaign = self.campaigns.get_mut(&write.id).unwrap();
            if !write.attempted {
                if let Some(start) = branch_start
                    && let Record::Control(ControlRecord::PlayerCommands { packet, .. }) =
                        &mut write.record
                    && let Some(phase) = &mut packet.phase
                {
                    phase.start = start;
                }
                write.attempted = true;
            }
            let standalone = !campaign.spool.is_open();
            let result = (|| {
                if standalone {
                    let mut context = engine_allocation::vec(1)?;
                    context.push(Record::SessionStart(campaign.session.try_clone()?));
                    campaign.spool.start_context(context)?;
                }
                campaign.spool.append_tracked(write.record.clone(), bytes)
            })();
            match result {
                Ok(Some(ticket)) => {
                    write.storage = Some(storage);
                    self.command_inflight.insert(ticket, write);
                }
                Ok(None) => self
                    .campaigns
                    .get_mut(&write.id)
                    .unwrap()
                    .columnar
                    .note(std::slice::from_ref(&write.record)),
                Err(error) => {
                    self.campaigns
                        .get_mut(&write.id)
                        .unwrap()
                        .reset_after_write_failure();
                    self.command_ready.insert(key, write);
                    return Err(error);
                }
            }
            if standalone {
                self.campaigns
                    .get_mut(&campaign_id)
                    .unwrap()
                    .spool
                    .finish_segment()?;
            }
        }
        Ok(())
    }

    /// # Safety
    /// Only the active spool or detach sequence may flush the originating writer and settle its
    /// publication receipts.
    pub unsafe fn flush(&mut self) -> io::Result<()> {
        self.poll_writer()?;
        // SAFETY: The session's spool owns this command drain and detachment has quieted callbacks
        // for the final drain.
        unsafe { self.flush_commands(false) }?;
        match self.writer_error.take() {
            Some(kind) => Err(kind.into()),
            None => Ok(()),
        }
    }

    /// # Safety
    /// Only the spool's detach sequence may close the processor, after draining frames and commands
    /// and before releasing their observer stores.
    pub unsafe fn detach(&mut self) -> io::Result<()> {
        self.drain_writer()?;
        let command_error =
        // SAFETY: The session's spool owns this command drain and detachment has quieted callbacks
        // for the final drain.
        unsafe { self.flush_commands(true) }.err();
        let _details = replay_data::codec::error_details_scope();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Teardown);
        collector_memory::terminal::attempted_hour(None);
        if collector_memory::terminal::failed() {
            for (_, campaign) in &mut self.campaigns {
                let _ = campaign.spool.finish_segment();
            }
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let mut first_error = command_error;
        let summary_campaign = self
            .stats
            .campaign
            .as_deref()
            .or_else(|| self.campaigns.keys().next_back().map(String::as_str))
            .and_then(|id| <[u8; 36]>::try_from(id.as_bytes()).ok());
        for (id, campaign) in &mut self.campaigns {
            collector_memory::terminal::set_context(collector_memory::terminal::Context {
                campaign_id: id.as_bytes().try_into().ok(),
                branch: campaign.tracker.current_branch(),
                load_signal: None,
                failed_hour: None,
            });
            let unknown = if summary_campaign
                .as_ref()
                .is_some_and(|summary| summary.as_slice() == id.as_bytes())
            {
                self.unknown_pending
            } else {
                0
            };
            let detached = campaign.detach(unknown);
            let exhausted = collector_memory::terminal::failed()
                || detached
                    .as_ref()
                    .is_err_and(|error| error.kind() == io::ErrorKind::OutOfMemory);
            if !exhausted {
                campaign.columnar.finish();
            }
            match detached {
                Ok(()) if unknown != 0 => self.unknown_pending = 0,
                Ok(()) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        self.drain_writer()?;
        if let Some(kind) = self.writer_error.take() {
            first_error.get_or_insert_with(|| io::Error::from(kind));
        }
        if self.unknown_pending != 0 {
            first_error
                .get_or_insert_with(|| invalid("unattributed clock losses remain at detach"));
        }
        if let Some(error) = first_error {
            self.error(&error);
            Err(error)
        } else {
            Ok(())
        }
    }

    pub fn take_retired(&mut self) -> Vec<replay_frame::frame::Retired> {
        let mut retired = Vec::new();
        for (_, campaign) in &mut self.campaigns {
            if retired.is_empty() {
                retired = std::mem::take(&mut campaign.retired);
            } else if engine_allocation::reserve(&mut retired, campaign.retired.len()).is_ok() {
                retired.append(&mut campaign.retired);
            } else {
                campaign.retired.clear();
            }
        }
        retired
    }

    pub fn end_spool_frame(
        &mut self,
        frame: Option<crate::slow_hours::SpoolFrame>,
    ) -> io::Result<()> {
        let conversion = self.stats.conversion.take_latest();
        match frame {
            Some(mut frame) => {
                if frame.hour.is_some() {
                    frame.conversion = conversion;
                }
                self.stats.callback_tail.record_spool(frame)
            }
            None => Ok(()),
        }
    }

    /// # Safety
    /// Only the originating spool may consume this recovery state, as part of handing the
    /// publication outcome back to capture.
    pub unsafe fn take_retry_full(&mut self) -> bool {
        std::mem::take(&mut self.retry_full)
    }

    /// # Safety
    /// Only the originating spool may consume this recovery state, as part of handing the
    /// publication outcome back to capture.
    pub unsafe fn take_unpublished_families(&mut self) -> u32 {
        std::mem::take(&mut self.unpublished_families)
    }

    pub fn spool_directories(&self) -> io::Result<Vec<std::path::PathBuf>> {
        let _details = replay_data::codec::error_details_scope();
        let mut directories = engine_allocation::vec(self.campaigns.0.len())?;
        for campaign in self.campaigns.values() {
            directories.push(campaign.directory.clone());
        }
        Ok(directories)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn status(&self) -> io::Result<impl Serialize + '_> {
        self.status_fields()
    }

    #[cfg(test)]
    pub(crate) fn recorded_hours(&self) -> u64 {
        self.stats.hours
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn allocation_failure_publication_state(
        &self,
        last_hour: i32,
    ) -> Option<(BaselineToken, (u32, u64))> {
        let campaign = self.campaigns.values().next()?;
        let branch = campaign.tracker.current_branch()?;
        let (baseline, _) = campaign.encoder.comparison_baseline::<tables::Campaign>(
            branch,
            last_hour.checked_add(1)?,
            false,
        )?;
        Some((baseline, campaign.spool.committed_watermark()?))
    }

    pub fn status_fields(&self) -> io::Result<impl Serialize + '_> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Status);
        collector_memory::terminal::attempted_hour(None);
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let stats = &self.stats;
        let callback = stats.callback.try_percentiles()?;
        let callback_work = stats.callback.status()?;
        let fields = status!({
            "campaign": &stats.campaign, "hours": stats.hours, "last_hour": stats.last_hour,
            "player": &stats.player,
            "total_ns": stats.total_ns, "p95_ns": callback.p95_ns,
            "callback_work": callback_work,
            "callback_tail": stats.callback_tail.status(),
            "hours_per_second": if stats.interval_qpc == 0 { 0.0 } else {
                stats.interval_hours as f64 * self.session.qpc_frequency as f64 / stats.interval_qpc as f64
            },
            "truncated_tables": stats.truncated_tables,
            "dropped_frames": stats.dropped_frames, "pool_bytes": stats.pool_bytes,
            "write_errors": stats.errors, "last_error": &stats.last_error,
            "build": collector_primitives::build_gate::status(),
            "spool_work": {
                "columnar": self.campaigns.values().filter_map(|campaign| campaign.columnar.status()).collect::<Vec<_>>(),
                "columnar_finished": crate::columnar::finished_status(),
            },
            "spool_conversion": stats.conversion.status(),
            "script_variables_conversion": stats.conversion.script_variables_status(),
        });
        #[cfg(feature = "conversion_check")]
        let fields = fields.join(status!({
            "conversion_check": collector_intel::intel::conversion_check::status(),
        }));
        Ok(fields)
    }

    #[cfg(test)]
    fn json(&self) -> Value {
        serde_json::to_value(self.status().unwrap()).unwrap()
    }
}

fn prepare_campaigns<'a>(
    ids: impl IntoIterator<Item = &'a [u8]>,
    mut open: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    let mut seen = Vec::new();
    for bytes in ids {
        let id = match campaign_name(bytes) {
            Ok(id) => id,
            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => return Err(error),
            Err(_) => continue,
        };
        if !seen.iter().any(|previous: &String| previous == &id) {
            open(&id)?;
            engine_allocation::reserve(&mut seen, 1)?;
            seen.push(id);
        }
    }
    Ok(())
}

pub fn campaign_name(bytes: &[u8]) -> io::Result<String> {
    if !is_campaign_uuid(bytes) {
        return Err(invalid(
            "campaign id is not a UUID; refusing an unsafe output path",
        ));
    }
    engine_allocation::string(std::str::from_utf8(bytes).expect("validated UUID is ASCII"))
}

pub(crate) use collector_projection::table_conversion::{table, unavailable};

fn area_node(
    nodes: &engine_native::cprovince::controller_areas::Output,
    province: &engine_native::cprovince::Row,
) -> Option<replay_data::rows::ControllerAreaNode> {
    let node = nodes.nodes.get(province.area_node? as usize)?;
    Some(replay_data::rows::ControllerAreaNode {
        first_land: node.first_land,
        neighbours: nodes.neighbours(node).map(<[u32]>::to_vec),
    })
}

fn area_node_equal(
    nodes: &engine_native::cprovince::controller_areas::Output,
    province: &engine_native::cprovince::Row,
    previous: &ProvinceRow,
) -> bool {
    let current = province
        .area_node
        .and_then(|index| nodes.nodes.get(index as usize));
    match (&previous.area_node, current) {
        (replay_data::rows::Recorded::Value(None), None) => true,
        (replay_data::rows::Recorded::Value(Some(previous)), Some(node)) => {
            previous.first_land == node.first_land
                && previous.neighbours.as_deref() == nodes.neighbours(node)
        }
        _ => false,
    }
}

fn state_table(
    captured: &Captured,
    projection: Option<&mut collector_projection::projection::Projection<'_>>,
) -> replay_frame::frame::TableInput {
    let copied = captured
        .state_names
        .get(..captured.state_names_len)
        .filter(|_| captured.states_result.len() <= captured.states.len());
    let Some(names) = copied else {
        return unavailable("state name arena length invalid").into();
    };
    let text = |span: engine_native::cstate::NameSpan| {
        span.start
            .checked_add(span.len)
            .and_then(|end| names.get(span.start..end))
    };
    if captured.states[..captured.states_result.len()]
        .iter()
        .any(|row| {
            text(row.custom_name).is_none()
                || text(row.owner_name).is_none()
                || row
                    .marked_owners
                    .is_some_and(|span| text(span).is_none_or(|words| words.len() % 4 != 0))
        })
    {
        return unavailable("state name lies outside the captured byte arena").into();
    }
    scalar_table::<tables::States, _>(
        projection,
        captured.states_result,
        &captured.states,
        |state| u128::from(state.id),
        |state| StateRow {
            infrastructure: replay_data::rows::Recorded::Value(state.infrastructure.map(|value| {
                replay_data::rows::StateInfrastructure {
                    building_type: value.building_type,
                    level: value.level,
                    maximum: value.maximum,
                }
            })),
            id: state.id,
            owner: state.owner,
            controller: state.controller,
            movement_infrastructure: state.movement_infrastructure,
            continent: replay_data::rows::Recorded::Value(state.continent),
            manpower_available: state.manpower_available,
            manpower_locked: replay_data::rows::Recorded::Value(state.manpower_locked),
            population: replay_data::rows::Recorded::Value(state.population),
            best_vp_index: replay_data::rows::Recorded::Value(state.best_vp_index),
            single_controller: replay_data::rows::Recorded::Value(
                state.single_controller.map(u32::from),
            ),
            category: replay_data::rows::Recorded::Value(state.category),
            railway_infrastructure: replay_data::rows::Recorded::Value(
                captured.state_inputs.fraction(state.id),
            ),
            custom_name: engine_allocation::owned_vec(engine_allocation::copy_vec(
                text(state.custom_name).unwrap_or_default(),
            )),
            owner_name: engine_allocation::owned_vec(engine_allocation::copy_vec(
                text(state.owner_name).unwrap_or_default(),
            )),
            marked_owners: replay_data::rows::Recorded::Value(
                state.marked_owners.and_then(text).map(|words| {
                    engine_allocation::owned_vec(engine_allocation::collect_vec(
                        words
                            .chunks_exact(4)
                            .map(|word| u32::from_le_bytes(word.try_into().unwrap())),
                    ))
                }),
            ),
        },
    )
}

fn faction_table(captured: &Captured) -> TableView {
    if captured.faction_names_len > captured.faction_names.len()
        || captured.factions_result.len() > captured.factions.len()
        || captured.factions[..captured.factions_result.len()]
            .iter()
            .any(|row| {
                row.name_start
                    .checked_add(row.name_len)
                    .is_none_or(|end| end > captured.faction_names_len)
            })
    {
        return unavailable("faction name lies outside the captured byte arena");
    }
    table::<tables::Factions, _>(captured.factions_result, &captured.factions, |faction| {
        FactionRow {
            id: faction.id,
            name: engine_allocation::owned_vec(engine_allocation::copy_vec(
                &captured.faction_names[faction.name_start..faction.name_start + faction.name_len],
            )),
            template_token: faction.template_token,
            color_red: faction.color[0],
            color_green: faction.color[1],
            color_blue: faction.color[2],
            color_alpha: faction.color[3],
            color_overridden: faction.color_overridden,
        }
    })
}

#[cfg(test)]
fn sampled_frame(source: FrameInput) -> replay_frame::frame::DecodedFrame {
    replay_frame::frame::DecodedFrame {
        branch: source.branch,
        hour: source.hour,
        metrics: source.metrics,
        tables: source
            .tables
            .map(|input| match input {
                TableInput::Sampled(view) => view,
                TableInput::SharedComplete(rows) => TableView::Complete((*rows).clone()),
                TableInput::ComparedEqual(_)
                | TableInput::ComparedChanges { .. }
                | TableInput::SliceUpdate { .. }
                | TableInput::UniformOptionalI32 { .. } => {
                    panic!("fixture expected ordinary sampled rows")
                }
            })
            .map(Into::into),
    }
}

#[cfg(test)]
fn decoded(
    captured: &Captured,
    clock: Clock,
    branch: BranchId,
) -> replay_frame::frame::DecodedFrame {
    replay_frame::frame::retire_absent(sampled_frame(
        decoded_with_costs(
            captured,
            clock.borrowed(),
            branch,
            &mut SpoolConversion {
                variables_baseline: Default::default(),
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: None,
                intel: None,
                projection: None,
                helper: None,
            },
        )
        .0,
    ))
}

fn mark_reads(source: &mut FrameInput, skipped: &[bool; TABLE_COUNT]) {
    for ((input, ns), &skipped) in source
        .tables
        .iter()
        .zip(&mut source.metrics.table_ns)
        .zip(skipped)
    {
        let read = !skipped
            && match input {
                TableInput::Sampled(
                    TableView::Unavailable { .. } | TableView::NotRecorded { .. },
                ) => *ns != 0,
                _ => true,
            };
        if !read {
            *ns = 0;
        } else if *ns == 0 {
            *ns = replay_frame::frame::READ_TIME_NOT_RECORDED;
        }
    }
}

fn decoded_with_costs(
    captured: &Captured,
    clock: Clock<&[u8]>,
    branch: BranchId,
    conversion: &mut SpoolConversion<'_, '_>,
) -> (FrameInput, collector_capture::pack_conversion::HourCosts) {
    decoded_with_helper_steps(captured, clock, branch, conversion, &HELPER)
}

fn decoded_with_helper_steps(
    captured: &Captured,
    clock: Clock<&[u8]>,
    branch: BranchId,
    conversion: &mut SpoolConversion<'_, '_>,
    helper_steps: &Steps<impl PhaseObserver + Sync>,
) -> (FrameInput, collector_capture::pack_conversion::HourCosts) {
    let mut costs = collector_capture::pack_conversion::HourCosts::EMPTY;
    let metrics = Metrics {
        total_ns: captured.total_ns,
        table_ns: captured.table_ns,
        pool_bytes: captured.pool_bytes,
        dropped_frames: captured.dropped_frames,
    };
    let failure = captured
        .clock
        .is_none()
        .then_some("clock unreadable as its capture began");
    let span =
    // SAFETY: This worker records phases on the counters assigned to its spool role.
    unsafe { collector_workers::spool_phases::SPOOL
        .span(collector_workers::spool_phases::Phase::ConvertCore) };
    let tables = if let Some(reason) = failure {
        unavailable_tables(reason)
    } else {
        let mut tables = unavailable_tables(NOT_ACQUIRED);
        match conversion.helper {
            None => {
                let started = std::time::Instant::now();
                let map = crate::map_spool::table_inputs(
                    &captured.map,
                    conversion.projection.as_deref_mut(),
                );
                let core = core_tables(captured, &clock, conversion.projection.as_deref_mut());
                for (destination, value) in tables.iter_mut().zip(core.into_iter().chain(map)) {
                    *destination = value;
                }
                tables[TableId::BorderConflicts.index()] = border_conflicts_table(captured).into();
                let _packs = span.switch(collector_workers::spool_phases::Phase::ConvertPacks);
                // SAFETY: The spool owns the frame and skips any packs on loan to its joined
                // helpers.
                unsafe {
                    captured.extended.insert_tables(
                        clock.hour,
                        &mut tables,
                        conversion,
                        false,
                        &mut costs,
                    )
                };
                costs.thread(started);
            }
            Some(split) => {
                convert_with_helper(
                    captured,
                    &clock,
                    conversion,
                    split,
                    &mut tables,
                    &mut costs,
                    (span, helper_steps),
                );
            }
        }
        tables[TableId::StateResistance.index()] = captured
            .state_inputs
            .occupation
            .input(conversion.projection.as_deref_mut());
        for (table, value) in collector_factions::faction_companions::TABLES
            .into_iter()
            .zip(captured.faction_companions.inputs())
        {
            tables[table.index()] = value;
        }
        for (table, value) in collector_factions::faction_goals::TABLES
            .into_iter()
            .zip(captured.faction_goals.inputs())
        {
            tables[table.index()] = value;
        }
        captured
            .map
            .naval
            .management
            .insert(collector_naval_management::forces::TABLES, &mut tables);
        tables
    };
    let mut input = FrameInput {
        branch,
        hour: clock.hour,
        tables,
        metrics,
    };
    input.round_to_display();
    (input, costs)
}

fn core_tables(
    captured: &Captured,
    clock: &Clock<&[u8]>,
    mut projection: Option<&mut collector_projection::projection::Projection<'_>>,
) -> [TableInput; 10] {
    let counters = captured.counters;
    [
        table::<tables::Clock, _>(
            Outcome::Rows { len: 1 },
            std::slice::from_ref(clock),
            |clock| ClockRow {
                hour: clock.hour,
                play_time: clock.play_time,
                play_session_started_at: clock.play_session_started_at,
                seed: clock.seed,
                speed: clock.speed,
                debug_current_ref_id: counters.and_then(|c| c.debug_current_ref_id),
                unit: counters.and_then(|c| c.unit),
                order_index: counters.and_then(|c| c.order_index),
                front_index: counters.and_then(|c| c.front_index),
                theatre_index: counters.and_then(|c| c.theatre_index),
                theater_group_index: counters.and_then(|c| c.theater_group_index),
                military_deployment_line_index: counters
                    .and_then(|c| c.military_deployment_line_index),
                military_deployment_conveyor_index: counters
                    .and_then(|c| c.military_deployment_conveyor_index),
                equipment_variant_index: counters.and_then(|c| c.equipment_variant_index),
                country_leader_index: counters.and_then(|c| c.country_leader_index),
                navy_id: counters.and_then(|c| c.navy_id),
                land_combat_id: counters.and_then(|c| c.land_combat_id),
                multiplayer_random_count: counters.and_then(|c| c.multiplayer_random_count),
            },
        )
        .into(),
        table::<tables::Campaign, _>(
            Outcome::Rows { len: 1 },
            std::slice::from_ref(clock),
            |clock| CampaignRow {
                id: engine_allocation::owned_vec(engine_allocation::copy_vec(clock.campaign_id)),
            },
        )
        .into(),
        project_table::<tables::Countries, _>(
            projection.as_deref_mut(),
            captured.countries_result,
            &captured.countries,
            |country| u128::from(country.id),
            |country, previous| {
                country.id == previous.id
                    && country.canonical_slot == previous.canonical_slot
                    && country.tag.as_bytes() == previous.tag
                    && country.political_power_raw == previous.political_power_raw
                    && country.command_power_raw == previous.command_power_raw
                    && country.stability_raw == previous.stability_raw
                    && country.war_support_raw == previous.war_support_raw
                    && country.convoys_raw == previous.convoys_raw
                    && country.manpower_residual == previous.manpower_residual
                    && replay_data::rows::Recorded::Value(country.border_land_doctrine_level)
                        == previous.border_land_doctrine_level
                    && country.peace_conference_participant == previous.peace_conference_participant
            },
            |country| CountryRow {
                id: country.id,
                canonical_slot: country.canonical_slot,
                tag: engine_allocation::owned_vec(engine_allocation::copy_vec(
                    country.tag.as_bytes(),
                )),
                political_power_raw: country.political_power_raw,
                command_power_raw: country.command_power_raw,
                stability_raw: country.stability_raw,
                war_support_raw: country.war_support_raw,
                convoys_raw: country.convoys_raw,
                peace_conference_participant: country.peace_conference_participant,
                manpower_residual: country.manpower_residual,
                border_land_doctrine_level: replay_data::rows::Recorded::Value(
                    country.border_land_doctrine_level,
                ),
            },
        ),
        scalar_table::<tables::CountryLinks, _>(
            projection.as_deref_mut(),
            captured.links_result,
            &captured.links,
            |link| u128::from(link.id),
            |link| CountryLinkRow {
                id: link.id,
                canonical_slot: link.canonical_slot,
            },
        ),
        project_table::<tables::Provinces, _>(
            projection.as_deref_mut(),
            captured.provinces_result,
            &captured.provinces.rows,
            |province| u128::from(province.id),
            |province, previous| {
                province.id == previous.id
                    && province.state == previous.state
                    && province.owner == previous.owner
                    && province.controller == previous.controller
                    && province.victory_points == previous.victory_points
                    && province.fort_positive == previous.fort_positive
                    && province.move_gate == previous.move_gate
                    && province.port_sea == previous.port_sea
                    && replay_data::rows::Recorded::Value(province.fort_bunker)
                        == previous.fort_bunker
                    && previous.selected_areas
                        == replay_data::rows::Recorded::Value(replay_data::rows::Retired)
                    && previous.attack
                        == replay_data::rows::Recorded::Value(replay_data::rows::Witnessed(None))
                    && previous.controller_area
                        == replay_data::rows::Recorded::Value(province.controller_area)
                    && previous.owner_area
                        == replay_data::rows::Recorded::Value(province.owner_area)
                    && area_node_equal(&captured.provinces.area_nodes, province, previous)
            },
            |province| ProvinceRow {
                id: province.id,
                state: province.state,
                owner: province.owner,
                controller: province.controller,
                victory_points: province.victory_points,
                fort_positive: province.fort_positive,
                move_gate: province.move_gate,
                port_sea: province.port_sea,
                fort_bunker: replay_data::rows::Recorded::Value(province.fort_bunker),
                selected_areas: replay_data::rows::Recorded::Value(replay_data::rows::Retired),
                attack: replay_data::rows::Recorded::Value(replay_data::rows::Witnessed(None)),
                controller_area: replay_data::rows::Recorded::Value(province.controller_area),
                owner_area: replay_data::rows::Recorded::Value(province.owner_area),
                area_node: replay_data::rows::Recorded::Value(area_node(
                    &captured.provinces.area_nodes,
                    province,
                )),
            },
        ),
        state_table(captured, projection.as_deref_mut()),
        scalar_table::<tables::Divisions, _>(
            projection.as_deref_mut(),
            captured.divisions_result,
            &captured.divisions,
            |division| u128::from(division.id),
            |division| DivisionRow {
                id: division.id,
                roster_owner: division.roster_owner,
                logical_owner: division.logical_owner,
                expeditionary_owner: division.expeditionary_owner,
                province: division.province,
                template: division.template,
                hit_points: division.hit_points,
                organisation: division.organisation,
                orders_group: division.orders_group,
                reveal_skip: division.reveal_skip,
                theatre: replay_data::rows::Recorded::Value(division.theatre),
            },
        ),
        scalar_table::<tables::WarRelations, _>(
            projection.as_deref_mut(),
            captured.wars_result,
            &captured.wars,
            |war| (u128::from(war.first_country) << 32) | u128::from(war.second_country),
            |war| WarRelationRow {
                first_country: war.first_country,
                second_country: war.second_country,
                start_hour: war.start_hour,
                end_hour: war.end_hour,
                cancel: war.cancel,
                first_casualties: war.first_casualties,
                second_casualties: war.second_casualties,
                first_unknown_casualties: war.first_unknown_casualties,
                second_unknown_casualties: war.second_unknown_casualties,
                first_was_instigator: war.first_was_instigator,
            },
        ),
        faction_table(captured).into(),
        scalar_table::<tables::FactionMembers, _>(
            projection,
            captured.faction_members_result,
            &captured.faction_members,
            |member| u128::from(member.country),
            |member| FactionMemberRow {
                country: member.country,
                faction: member.faction,
                ordinal: member.ordinal,
            },
        ),
    ]
}

fn convert_with_helper(
    captured: &Captured,
    clock: &Clock<&[u8]>,
    conversion: &mut SpoolConversion<'_, '_>,
    split: HelperSplit<'_>,
    tables: &mut [TableInput; TABLE_COUNT],
    costs: &mut collector_capture::pack_conversion::HourCosts,
    (span, helper_steps): (
        collector_workers::spool_phases::Span<'static>,
        &Steps<impl PhaseObserver + Sync>,
    ),
) {
    use collector_workers::spool_phases::Phase;
    struct Converted {
        map: Option<[TableInput; 10]>,
        packs: [Option<TableInput>; TABLE_COUNT],
        costs: collector_capture::pack_conversion::HourCosts,
    }
    fn take_in(
        converted: &std::sync::Mutex<Converted>,
        map: Option<[TableInput; 10]>,
        packs: &mut [Option<TableInput>; TABLE_COUNT],
        costs: Option<&collector_capture::pack_conversion::HourCosts>,
    ) {
        let mut converted = converted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.is_some() {
            converted.map = map;
        }
        for (destination, value) in converted.packs.iter_mut().zip(packs) {
            if let Some(value) = value.take() {
                *destination = Some(value);
            }
        }
        if let Some(costs) = costs {
            converted.costs.merge(costs);
        }
    }
    let projection = conversion
        .projection
        .as_ref()
        .map(|projection| **projection);
    let hour = clock.hour;
    let view =
    // SAFETY: The spool joins all converters before reusing the frame and gives each pack one
    // queue owner.
    unsafe { captured.extended.helper_view() };
    let map = &captured.map;
    let context = collector_memory::terminal::context();
    let SpoolConversion {
        variables_baseline,
        variables_scratch,
        characters,
        lexical,
        intel,
        projection: own_projection,
        helper: _,
    } = conversion;
    let lent = std::sync::Mutex::new(collector_capture::extended::LentCaches {
        characters: characters.take(),
        intel: intel.take(),
        variables_baseline: *variables_baseline,
        variables_scratch: Some(&mut **variables_scratch),
    });
    let mut spare_scratch = ScriptVariableComparison::default();
    let mut conversion = SpoolConversion {
        variables_baseline: Default::default(),
        variables_scratch: &mut spare_scratch,
        characters: None,
        lexical: lexical.as_deref_mut(),
        intel: None,
        projection: own_projection.as_deref_mut(),
        helper: None,
    };
    let converted = std::sync::Mutex::new(Converted {
        map: None,
        packs: [const { None }; TABLE_COUNT],
        costs: collector_capture::pack_conversion::HourCosts::EMPTY,
    });
    let queue = split.plan.queue();
    let helper = |index: usize| {
        let started = std::time::Instant::now();
        let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
        collector_memory::terminal::set_context(context);
        let mut projection = projection;
        let mut costs = collector_capture::pack_conversion::HourCosts::of(index + 1);
        let mut packs = [const { None }; TABLE_COUNT];
        let span =
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { helper_steps.span(Phase::ConvertCore) };
        let map = (index == 0).then(|| crate::map_spool::table_inputs(map, projection.as_mut()));
        let span = span.switch(Phase::ConvertPacks);
        // SAFETY: This conversion belongs to the joined spool round and uses its shared queue.
        unsafe {
            view.convert_queued(
                &queue,
                hour,
                projection.as_mut(),
                &lent,
                &mut packs,
                &mut costs,
            )
        };
        costs.thread(started);
        span.finish();
        take_in(&converted, map, &mut packs, Some(&costs));
        collector_workers::private_heap::PRIVATE_HEAP.maybe_flush_thread_cache();
    };
    let own = || {
        let started = std::time::Instant::now();
        let conversion = &mut conversion;
        let core = core_tables(captured, clock, conversion.projection.as_deref_mut());
        for (destination, value) in tables.iter_mut().zip(core) {
            *destination = value;
        }
        tables[TableId::BorderConflicts.index()] = border_conflicts_table(captured).into();
        let span = span.switch(Phase::ConvertPacks);
        // SAFETY: The spool owns the frame and skips any packs on loan to its joined helpers.
        unsafe {
            captured
                .extended
                .insert_tables(clock.hour, tables, conversion, true, costs)
        };
        let mut packs = [const { None }; TABLE_COUNT];
        // SAFETY: This conversion belongs to the joined spool round and uses its shared queue.
        unsafe {
            view.convert_queued(
                &queue,
                hour,
                conversion.projection.as_deref_mut(),
                &lent,
                &mut packs,
                costs,
            )
        };
        costs.thread(started);
        take_in(&converted, None, &mut packs, None);
        span.switch(Phase::HelperWait)
    };
    // SAFETY: The caller's single spool round owns each selected helper until it joins.
    unsafe { split.helpers.join_some(split.plan.helpers(), &helper, own) }.finish();
    let Converted {
        map,
        packs,
        costs: helper_costs,
    } = converted
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(map) = map {
        for (destination, value) in tables[TableId::Players.index()..].iter_mut().zip(map) {
            *destination = value;
        }
    }
    for (destination, value) in tables.iter_mut().zip(packs) {
        if let Some(value) = value {
            *destination = value;
        }
    }
    costs.merge(&helper_costs);
    captured.extended.refuse_failed_ordinary(tables);
}

fn border_conflicts_outcome(provinces: Outcome, conflicts: Outcome) -> Outcome {
    match (provinces, conflicts) {
        (Outcome::Unavailable { reason }, _) | (_, Outcome::Unavailable { reason }) => {
            Outcome::Unavailable { reason }
        }
        (Outcome::Truncated { unread_rows, .. }, conflicts) => Outcome::Truncated {
            len: conflicts.len(),
            unread_rows: unread_rows.max(1).saturating_add(match conflicts {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            }),
        },
        (Outcome::Rows { .. }, conflicts) => conflicts,
    }
}

fn border_conflicts_table(captured: &Captured) -> TableView {
    let province = &captured.provinces;
    let outcome =
        border_conflicts_outcome(captured.provinces_result, province.conflict_walk.outcome);
    let Some(groups) = province.groups.get(..province.conflict_walk.groups_len) else {
        return unavailable("border conflict group arena length invalid");
    };
    let list = |r: &engine_native::cprovince::ConflictRow| {
        r.groups_start
            .checked_add(r.groups_len)
            .and_then(|end| groups.get(r.groups_start..end))
    };
    let rows = &province.conflicts[..outcome.len().min(province.conflicts.len())];
    if rows.iter().any(|r| list(r).is_none()) {
        return unavailable("border conflict groups outside captured arena");
    }
    table::<tables::BorderConflicts, _>(outcome, &province.conflicts, |r| BorderConflictRow {
        province: r.province,
        groups: engine_allocation::owned_vec(engine_allocation::copy_vec(list(r).unwrap())),
    })
}

const NOT_ACQUIRED: &str = "table was not acquired";

fn unavailable_tables(reason: &'static str) -> [TableInput; TABLE_COUNT] {
    std::array::from_fn(|index| {
        TableInput::Sampled(match retired_reason(TableId::ALL[index]) {
            Some(retired) => unavailable(retired),
            None => unavailable(reason),
        })
    })
}

fn retired_reason(table: TableId) -> Option<&'static str> {
    static REASONS: std::sync::OnceLock<[Option<String>; TABLE_COUNT]> = std::sync::OnceLock::new();
    REASONS.get_or_init(|| {
        let current = replay_data::format::FormatVersion::CURRENT.version();
        std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            (!table.last_change().present(current))
                .then(|| replay_data::format::FormatVersion::CURRENT.table_refusal(table))
                .flatten()
        })
    })[table.index()]
    .as_deref()
}

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Run only in an isolated fixture with exclusive use of the process-wide recorder state.
pub unsafe fn spool_failure_fixture(root: &Path) -> io::Result<Processor> {
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { Processor::new(root, PreparedCampaigns::default(), session()) }
}

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Use only in the isolated allocation failure fixture with exclusive use of the spool state.
pub unsafe fn spool_allocation_fixture(
    root: &Path,
    lockstep: bool,
) -> io::Result<Processor<crate::spool_writer::FixtureSchedule>> {
    Processor::new_scheduled(root, PreparedCampaigns::default(), session(), |root| {
        crate::spool_writer::Writer::start_scheduled(
            root,
            crate::spool_writer::FixtureSchedule::new(lockstep),
        )
    })
}

#[cfg(any(test, feature = "test-support"))]
pub fn campaign_frame(campaign: &[u8], signal: i64, hour: i32) -> Captured {
    let mut frame = captured(hour);
    let clock = frame.clock.as_mut().unwrap();
    clock.campaign_id = engine_native_memory::capture_text::Owned {
        bytes: campaign.to_vec(),
        len: campaign.len(),
    };
    clock.play_session_started_at = signal;
    frame
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_frame(hour: i32) -> Captured {
    let mut frame = captured(hour);
    let clock = frame.clock.as_mut().unwrap();
    let uuid = b"01234567-89ab-cdef-0123-456789abcdef";
    clock.campaign_id.len = uuid.len();
    clock.campaign_id.bytes[..uuid.len()].copy_from_slice(uuid);
    engine_allocation::resize_default(&mut frame.map.land_combats.rows, 1).unwrap();
    engine_allocation::resize_default(&mut frame.map.land_participants, 2).unwrap();
    engine_allocation::resize_default(&mut frame.map.land_classes, 2).unwrap();
    frame.map.land_combats.rows[0] = engine_native::cland_combat::Row {
        id: 1,
        attacker_start: 0,
        attacker_len: 1,
        defender_start: 1,
        defender_len: 1,
        ..Default::default()
    };
    frame.map.land_combats.outcome = Outcome::Rows { len: 1 };
    frame.map.land_participants[..2].copy_from_slice(&[1, 2]);
    frame.map.land_participants_len = 2;
    frame
}

#[cfg(test)]
mod conversion_helper_tests;

#[cfg(test)]
#[path = "spooling/slice_reads_tests.rs"]
mod slice_reads_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "spooling/write_behind_tests.rs"]
pub mod write_behind_tests;

#[cfg(any(test, feature = "test-support"))]
const ID: &[u8] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

#[cfg(any(test, feature = "test-support"))]
fn owned_text(bytes: &[u8]) -> engine_native_memory::capture_text::Owned {
    engine_native_memory::capture_text::Owned {
        bytes: bytes.to_vec(),
        len: bytes.len(),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn clock(hour: i32) -> Clock {
    Clock {
        game_state: 4096,
        hour,
        play_time: u64::MAX,
        play_session_started_at: 1234,
        seed: i32::MIN,
        campaign_id: owned_text(ID),
        speed: 5,
        tick_busy: 0,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn captured(hour: i32) -> Captured {
    let mut captured =
    // SAFETY: The session or isolated fixture owns this frame's observer allocations and excludes
    // teardown.
    unsafe { Captured::with_capacities(
        &[1, 1, 1, 1, 2, 1, 1, 2, 6, 0, 0, 0],
        &collector_map_capture::map_capture::MapCapacities::default(),
        &collector_capture::extended::Capacities::default(),
    ) }
    .unwrap();
    captured.clock = Some(clock(hour));
    captured.qpc = 1000 + i64::from(hour);
    captured.countries_result = Outcome::Rows { len: 0 };
    captured.links_result = Outcome::Rows { len: 0 };
    captured.provinces_result = Outcome::Rows { len: 0 };
    captured.states_result = Outcome::Rows { len: 0 };
    captured.divisions_result = Outcome::Rows { len: 0 };
    captured.wars_result = Outcome::Rows { len: 0 };
    captured.factions_result = Outcome::Rows { len: 0 };
    captured.faction_members_result = Outcome::Rows { len: 0 };
    captured.map.players.outcome = Outcome::Rows { len: 0 };
    captured.map.human_controllers.outcome = Outcome::Rows { len: 0 };
    captured.map.land_combats.outcome = Outcome::Rows { len: 0 };
    captured.map.naval.fleets.outcome = Outcome::Rows { len: 0 };
    captured.map.naval.task_forces.outcome = Outcome::Rows { len: 0 };
    captured.map.naval.ships.outcome = Outcome::Rows { len: 0 };
    captured.map.naval_combats.outcome = Outcome::Rows { len: 0 };
    captured.map.naval_combat_participants.outcome = Outcome::Rows { len: 0 };
    captured.map.air_wings.outcome = Outcome::Rows { len: 0 };
    captured.map.orders.rows.outcome = Outcome::Rows { len: 0 };
    captured
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn session() -> SessionStart {
    use replay_data::identity::{Identity, Verification};
    SessionStart {
        id: 1,
        process_id: 100,
        process_start_time: 1000,
        qpc_frequency: 10_000_000,
        sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
        capture_mode: replay_data::identity::CaptureMode::Full,
        identity: Identity {
            game_version: "fixture".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "english".into(),
            recorder_version: "fixture".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{campaign::Replay, campaign::SeekResult};
    use replay_data::{rows::Recorded, rows::TableId};
    use replay_frame::frame::DecodedFrame;

    #[test]
    fn the_viewed_country_is_published_from_committed_rows_and_copied_only_on_change() {
        use replay_data::country_flags::FlagOperands;
        assert_eq!(viewed_flag(&FrameEncoder::new()), None);
        let germany = FlagOperands {
            tag: "mod-country-long-tag",
            cosmetic: "",
            original: Some("mod-country-long-tag"),
            group: "synthetic_group",
        };
        let mut slot = None;
        set_viewed(&mut slot, Some(germany)).unwrap();
        assert_eq!(
            serde_json::to_value(&slot).unwrap(),
            json!({"tag":"mod-country-long-tag","cosmetic":"","original":"mod-country-long-tag","group":"synthetic_group"})
        );
        let copied = slot.as_ref().unwrap().tag.as_ptr();
        set_viewed(&mut slot, Some(germany)).unwrap();
        assert_eq!(slot.as_ref().unwrap().tag.as_ptr(), copied);
        let changed = FlagOperands {
            cosmetic: "variants/red",
            original: None,
            ..germany
        };
        set_viewed(&mut slot, Some(changed)).unwrap();
        assert_eq!(slot.as_ref().unwrap().cosmetic, "variants/red");
        assert_eq!(
            serde_json::to_value(&slot).unwrap()["original"],
            Value::Null
        );
        set_viewed(&mut slot, None).unwrap();
        assert_eq!(slot, None);
    }

    #[test]
    fn compared_variables_never_override_a_missing_entry_clock() {
        let mut encoder = FrameEncoder::new();
        encoder
            .encode(staggered_frame(1, 10, false), false)
            .unwrap();
        let baseline = collector_intel::intel::VariableBaselines {
            scopes: encoder.comparison_baseline::<tables::ScriptVariables>(1, 11, false),
            names: encoder.comparison_baseline::<tables::ScriptVariableNames>(1, 11, false),
            values: encoder.comparison_baseline::<tables::ScriptVariableValues>(1, 11, false),
        };
        let mut capture = captured(11);
        capture.clock = None;
        let (input, _) = decoded_with_costs(
            &capture,
            clock(11).borrowed(),
            1,
            &mut SpoolConversion {
                variables_baseline: baseline,
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: None,
                intel: None,
                projection: None,
                helper: None,
            },
        );
        assert_eq!(
            input.tables,
            unavailable_tables("clock unreadable as its capture began")
        );
    }

    #[test]
    fn validation_refuses_only_bad_table_and_next_hour_rebaselines() {
        use replay_frame::frame::{FrameDecoder, TableUpdate};
        crate::spooling::keep_every_table();
        let directory =
            std::env::temp_dir().join(format!("hoi4-contain-table-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&captured(10)) }.unwrap();
        for hour in [11, 12] {
            let mut prepared =
            // SAFETY: This is the decoded frame held by its originating spool.
            unsafe { processor.prepare_frame(&captured(hour)) }.unwrap();
            prepared.source.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 64,
                    continent: Recorded::NotRecorded,
                    ..Default::default()
                }]))
                .into();
            prepared.source.tables[TableId::Campaign.index()] =
                TableView::Complete(TableRows::Campaign(Vec::new())).into();
            // SAFETY: This spool submits the prepared frame to its originating processor before
            // teardown.
            unsafe { processor.write_prepared(prepared) }.unwrap();
        }
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&captured(13)) }.unwrap();
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let status = processor.json();
        assert_eq!(status["hours"], 4);
        assert_eq!(status["write_errors"], 0);
        let log = fs::read_to_string(directory.join("recorder.log")).unwrap();
        assert_eq!(log.matches("States: row 64 column continent").count(), 1);
        let campaign = processor
            .campaigns
            .get_mut(std::str::from_utf8(ID).unwrap())
            .unwrap();
        assert!(campaign.pending.is_empty());
        let mut decoder = FrameDecoder::new();
        let mut frames = 0;
        for file in fs::read_dir(&campaign.directory).unwrap().flatten() {
            if file.file_name().to_string_lossy().starts_with("spool.") {
                for record in replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()).records
                {
                    if let Record::Frame(frame) = record {
                        let decoded = decoder.apply(&frame).unwrap();
                        frames += 1;
                        if [11, 12].contains(&frame.hour) {
                            assert!(
                                matches!(&*decoded.tables[TableId::States.index()], TableView::Unavailable { reason } if reason.contains("States: row 64 column continent"))
                            );
                            assert!(decoded.complete::<tables::Clock>().is_ok());
                        }
                        if frame.hour == 13 {
                            assert!(matches!(
                                frame.tables[TableId::States.index()],
                                TableUpdate::Snapshot(_)
                            ));
                        }
                    }
                }
            }
        }
        assert_eq!(frames, 4);
        drop(processor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn paused_commands_publish_both_formats_with_or_without_a_later_hour() {
        use replay_recording::command_packet::{Batch, Event, Origin, Span};
        for hourly in [None, Some(10), Some(11)] {
            let directory = std::env::temp_dir().join(format!(
                "hoi4-command-paused-{}-{}",
                std::process::id(),
                hourly.unwrap_or(-1)
            ));
            fs::create_dir_all(&directory).unwrap();
            let mut processor =
                // SAFETY: The caller owns the session or isolated fixture state required by this
                // constructor.
                unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
            processor
                .command_pending
                .push(collector_player_commands::player_commands::Drained {
                    capture: 1,
                    world: 1,
                    from: 100,
                    to: 200,
                    lost: 0,
                    context: Some(collector_player_commands::player_commands::WorldContext {
                        campaign: ID.try_into().unwrap(),
                        hour: 10,
                        load_signal: 1234,
                        qpc: 100,
                    }),
                    batch: Batch {
                        events: vec![Event {
                            paused: false,
                            qpc: 150,
                            hour: 10,
                            actor: 0,
                            player: 1,
                            kind: 0,
                            command_country: None,
                            origin: Origin::Unresolved,
                            sequence: 1,
                        }],
                        spans: vec![Span {
                            paused: false,
                            hour: 10,
                            from_qpc: 100,
                            to_qpc: 200,
                            complete: true,
                        }],
                        ..Default::default()
                    },
                });
            assert!(
                // SAFETY: The session's spool owns this command drain and detachment has quieted
                // callbacks for the final drain.
                unsafe { processor.flush_commands(true) }.is_err()
            );
            assert!(processor.campaigns.is_empty());
            let mut verified = session();
            verified.identity.verification = replay_data::identity::Verification::Verified {
                launcher: Default::default(),
                memory: Default::default(),
            };
            processor.set_verified_session(verified.clone()).unwrap();
            // SAFETY: The session's spool owns this command drain and detachment has quieted
            // callbacks for the final drain.
            unsafe { processor.flush_commands(true) }.unwrap();
            let command_branch = processor.command_contexts[&1].1;
            assert!(
                processor
                    .campaigns
                    .get_mut(std::str::from_utf8(ID).unwrap())
                    .unwrap()
                    .tracker
                    .recorded_hours(command_branch)
                    .unwrap()
                    .is_empty()
            );
            if let Some(hour) = hourly {
                let mut frame = captured(hour);
                frame.world_generation = 1;
                // SAFETY: This frame or slot belongs to the current admitted capture or the sole
                // spool consuming it.
                unsafe { processor.frame(&frame) }.unwrap();
                assert_eq!(processor.command_contexts[&1].1, command_branch);
            }
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
            drop(processor);
            let campaign = directory.join(std::str::from_utf8(ID).unwrap());
            let mut files: Vec<_> = fs::read_dir(&campaign)
                .unwrap()
                .flatten()
                .filter(|file| file.file_name().to_string_lossy().starts_with("spool."))
                .collect();
            files.sort_by_key(|file| file.file_name());
            let segments: Vec<_> = files
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    (
                        index as u32,
                        replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()),
                    )
                })
                .collect();
            let replay = replay::campaign::Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &segments).unwrap();
            let row_path = directory.join("paused.battleplan");
            replay.write_atomic(&row_path).unwrap();
            let history = replay.into_history().unwrap();
            let tracker = history.tracker_for(&verified).unwrap();
            assert_eq!(tracker.current_branch(), Some(command_branch));
            assert_eq!(
                tracker.recorded_hours(command_branch).unwrap().len(),
                usize::from(hourly.is_some())
            );
            let mut prepared = PreparedCampaigns::load(&directory, &verified, |_| Ok(())).unwrap();
            assert_eq!(
                prepared.campaign_world_generation(std::str::from_utf8(ID).unwrap()),
                Some(1)
            );
            let (resumed, lease) = prepared
                .take(std::str::from_utf8(ID).unwrap(), &verified)
                .unwrap();
            assert_eq!(resumed.current_branch(), Some(command_branch));
            drop((prepared, resumed, lease));
            for path in [row_path] {
                let recording = replay::campaign::commands::open_path(&path).unwrap();
                let commands: Vec<_> = recording
                    .controls
                    .iter()
                    .filter_map(|control| match control {
                        ControlRecord::PlayerCommands { packet, .. } => {
                            Some(packet.unpack().unwrap())
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    commands
                        .iter()
                        .map(|batch| batch.events.len())
                        .sum::<usize>(),
                    1
                );
                assert_eq!(commands[0].events[0].qpc, 150);
                assert_eq!(recording.branches.len(), 1);
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn same_world_reactivation_keeps_the_earliest_phase_in_both_formats() {
        let directory =
            std::env::temp_dir().join(format!("hoi4-command-reactivated-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let mut verified = session();
        verified.identity.verification = replay_data::identity::Verification::Verified {
            launcher: Default::default(),
            memory: Default::default(),
        };
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        processor.set_verified_session(verified).unwrap();
        processor.command_pending =
            collector_player_commands::player_commands::tests::reactivated_world();
        // SAFETY: The session's spool owns this command drain and detachment has quieted callbacks
        // for the final drain.
        unsafe { processor.flush_commands(true) }.unwrap();
        let branch = processor.command_contexts[&1].1;
        assert_eq!(processor.command_anchors[&(1, branch)], (100, 10, 5));
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        drop(processor);
        let campaign = directory.join(std::str::from_utf8(ID).unwrap());
        let mut files: Vec<_> = fs::read_dir(&campaign)
            .unwrap()
            .flatten()
            .filter(|file| file.file_name().to_string_lossy().starts_with("spool."))
            .collect();
        files.sort_by_key(|file| file.file_name());
        let segments: Vec<_> = files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                (
                    index as u32,
                    replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()),
                )
            })
            .collect();
        let replay = replay::campaign::Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &segments).unwrap();
        let row_path = directory.join("reactivated.battleplan");
        replay.write_atomic(&row_path).unwrap();
        for path in [row_path] {
            let recording = replay::campaign::commands::open_path(&path).unwrap();
            assert_eq!(recording.branches.len(), 1);
            let packets: Vec<_> = recording
                .controls
                .iter()
                .filter_map(|control| match control {
                    ControlRecord::PlayerCommands { packet, .. } => Some(packet),
                    _ => None,
                })
                .collect();
            assert_eq!(packets.len(), 1);
            let phase = packets[0].phase.as_ref().unwrap();
            phase.validate().unwrap();
            assert_eq!((phase.qpc, phase.hour, phase.load_signal), (100, 10, 5));
            assert_eq!(phase.start.first_hour, 10);
            let batch = packets[0].unpack().unwrap();
            assert_eq!(
                batch
                    .events
                    .iter()
                    .map(|event| (event.qpc, event.hour))
                    .collect::<Vec<_>>(),
                vec![(102, 10), (112, 12)]
            );
            assert_eq!(
                batch
                    .spans
                    .iter()
                    .map(|span| (span.from_qpc, span.to_qpc, span.hour))
                    .collect::<Vec<_>>(),
                vec![(100, 105, 10), (110, 115, 12)]
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn command_submit_allocation_failure_retains_packet_for_exact_once_retry() {
        use replay_recording::command_packet::{Batch, Event, Origin, Packet, Phase, Span};
        let directory = std::env::temp_dir().join(format!(
            "hoi4-command-submit-retry-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let mut verified = session();
        verified.identity.verification = replay_data::identity::Verification::Verified {
            launcher: Default::default(),
            memory: Default::default(),
        };
        // SAFETY: The isolated test owns this spool's directory and joins its writer at detach.
        let mut processor =
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        processor.set_verified_session(verified).unwrap();
        let campaign_id = std::str::from_utf8(ID).unwrap();
        let (branch, start) = {
            let campaign = processor.campaign(campaign_id).unwrap();
            let branch = campaign
                .tracker
                .observe_commands(10, replay_recording::command_packet::load_marker(1234))
                .unwrap()
                .branch;
            let start = campaign.tracker.branch_start(branch).unwrap().clone();
            (branch, start)
        };
        let batch = Batch {
            events: vec![Event {
                paused: false,
                qpc: 150,
                hour: 10,
                actor: 0,
                player: 1,
                kind: 0,
                command_country: None,
                origin: Origin::Unresolved,
                sequence: 1,
            }],
            spans: vec![Span {
                paused: false,
                hour: 10,
                from_qpc: 100,
                to_qpc: 200,
                complete: true,
            }],
            ..Default::default()
        };
        let mut packet = Packet::pack(branch, 1, 1, 0, 100, 200, 0, &batch).unwrap();
        packet.phase = Some(Phase {
            start,
            campaign: ID.try_into().unwrap(),
            qpc: 100,
            hour: 10,
            load_signal: 1234,
        });
        processor.command_ready.insert(
            (1, 0),
            CommandWrite {
                id: campaign_id.to_owned(),
                world: 1,
                ordinal: 0,
                record: Record::Control(ControlRecord::PlayerCommands {
                    session: processor.session.id,
                    packet: Box::new(packet),
                }),
                attempted: false,
                storage: None,
            },
        );
        let denied = {
            let _deny = collector_test_support::NullGuard::new(2);
            processor.submit_commands()
        };
        assert!(denied.is_err());
        assert!(processor.command_ready.contains_key(&(1, 0)));
        // SAFETY: The isolated test owns the spool and has no live command callbacks.
        unsafe { processor.flush_commands(true) }.unwrap();
        // SAFETY: The isolated test owns the spool through detach.
        unsafe { processor.detach() }.unwrap();
        drop(processor);
        let campaign = directory.join(campaign_id);
        let mut files: Vec<_> = fs::read_dir(&campaign)
            .unwrap()
            .flatten()
            .filter(|file| file.file_name().to_string_lossy().starts_with("spool."))
            .collect();
        files.sort_by_key(|file| file.file_name());
        let segments: Vec<_> = files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                (
                    index as u32,
                    replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()),
                )
            })
            .collect();
        let replay = replay::campaign::Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &segments).unwrap();
        let row_path = directory.join("submit-retry.battleplan");
        replay.write_atomic(&row_path).unwrap();
        let recording = replay::campaign::commands::open_path(&row_path).unwrap();
        let ordinals: Vec<_> = recording
            .controls
            .iter()
            .filter_map(|control| match control {
                ControlRecord::PlayerCommands { packet, .. } => Some(packet.ordinal),
                _ => None,
            })
            .collect();
        assert_eq!(ordinals, vec![0]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn delayed_command_world_declares_its_branch_without_interrupting_hourly_frames() {
        use replay_recording::command_packet::{Batch, Span};
        for later in [false, true] {
            let directory = std::env::temp_dir().join(format!(
                "hoi4-command-delayed-{}-{later}",
                std::process::id()
            ));
            let mut processor =
                // SAFETY: The caller owns the session or isolated fixture state required by this
                // constructor.
                unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
            let mut verified = session();
            verified.identity.verification = replay_data::identity::Verification::Verified {
                launcher: Default::default(),
                memory: Default::default(),
            };
            processor.set_verified_session(verified).unwrap();
            let mut frame = captured(11);
            frame.world_generation = 2;
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&frame) }.unwrap();
            let current = processor.command_contexts[&2].1;
            processor
                .command_pending
                .push(collector_player_commands::player_commands::Drained {
                    capture: 1,
                    world: 1,
                    from: 100,
                    to: 200,
                    lost: 0,
                    context: Some(collector_player_commands::player_commands::WorldContext {
                        campaign: ID.try_into().unwrap(),
                        hour: 10,
                        load_signal: 1234,
                        qpc: 100,
                    }),
                    batch: Batch {
                        spans: vec![Span {
                            paused: false,
                            hour: 10,
                            from_qpc: 100,
                            to_qpc: 200,
                            complete: true,
                        }],
                        ..Default::default()
                    },
                });
            // SAFETY: The session's spool owns this command drain and detachment has quieted
            // callbacks for the final drain.
            unsafe { processor.flush_commands(true) }.unwrap();
            let past = processor.command_contexts[&1].1;
            assert_ne!(past, current);
            assert_eq!(
                processor
                    .campaigns
                    .get_mut(std::str::from_utf8(ID).unwrap())
                    .unwrap()
                    .tracker
                    .current_branch(),
                Some(current)
            );
            if later {
                let mut frame = captured(12);
                frame.world_generation = 2;
                // SAFETY: This frame or slot belongs to the current admitted capture or the sole
                // spool consuming it.
                unsafe { processor.frame(&frame) }.unwrap();
            }
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
            drop(processor);
            let campaign = directory.join(std::str::from_utf8(ID).unwrap());
            let mut files: Vec<_> = fs::read_dir(&campaign)
                .unwrap()
                .flatten()
                .filter(|file| file.file_name().to_string_lossy().starts_with("spool."))
                .collect();
            files.sort_by_key(|file| file.file_name());
            let segments: Vec<_> = files
                .iter()
                .enumerate()
                .map(|(index, file)| {
                    (
                        index as u32,
                        replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()),
                    )
                })
                .collect();
            let row = Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &segments).unwrap();
            assert_eq!(row.branches.len(), 2);
            assert_eq!(
                row.recorded_hours(current).unwrap().len(),
                1 + usize::from(later)
            );
            assert!(row.recorded_hours(past).unwrap().is_empty());
            let history = row.into_history().unwrap();
            assert_eq!(history.session_tail(session().id).unwrap().branch, current);
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn a_close_request_closes_a_left_campaigns_segment_before_any_other_hour() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory =
            std::env::temp_dir().join(format!("hoi4-close-request-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&crate::spooling::campaign_frame(ID, 1234, 10)) }.unwrap();
        let name = std::str::from_utf8(ID).unwrap();
        let segment = fs::read_dir(directory.join(name))
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "h4s"))
            .unwrap();
        let settled = |path: &Path| {
            fs::OpenOptions::new()
                .read(true)
                .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
                .open(path)
                .is_ok()
        };
        assert!(!settled(&segment));
        assert_eq!(
            processor.history_request(&format!("1 close {name} 9999")),
            "1 refused the campaign was loaded again"
        );
        assert!(!settled(&segment));
        assert_eq!(
            processor.history_request(&format!("2 close {name} 1234")),
            "2 ok"
        );
        assert!(settled(&segment));
        drop(processor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn another_campaigns_hour_closes_the_left_campaigns_segment_for_good() {
        use std::os::windows::fs::OpenOptionsExt;
        const OTHER: &[u8] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a7";
        let directory = std::env::temp_dir().join(format!("hoi4-set-aside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        let load = |campaign: &[u8], signal: i64, hour: i32| {
            let mut source = captured(hour);
            let clock = source.clock.as_mut().unwrap();
            clock.campaign_id = owned_text(campaign);
            clock.play_session_started_at = signal;
            source
        };
        let first = directory.join(std::str::from_utf8(ID).unwrap());
        let segments = || {
            let mut paths: Vec<_> = fs::read_dir(&first)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "h4s"))
                .collect();
            paths.sort();
            paths
        };
        let settled = |path: &Path| {
            fs::OpenOptions::new()
                .read(true)
                .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
                .open(path)
                .is_ok()
        };
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&load(ID, 1234, 10)) }.unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&load(ID, 1234, 11)) }.unwrap();
        let [open] = segments().try_into().unwrap();
        assert!(!settled(&open), "the campaign being recorded keeps writing");
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&load(OTHER, 5678, 20)) }.unwrap();
        assert!(settled(&open), "the left campaign's segment is closed");
        let closed = fs::read(&open).unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&load(OTHER, 5678, 21)) }.unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&load(ID, 9999, 30)) }.unwrap();
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        assert_eq!(
            fs::read(&open).unwrap(),
            closed,
            "a closed segment never changes"
        );
        let [before, after] = segments().try_into().unwrap();
        assert_eq!(before, open);
        let records = |path: &Path| replay_capture_spool::spool::recover_prefix(&fs::read(path).unwrap()).records;
        let summary = |records: &[Record]| {
            records
                .iter()
                .map(|record| match record {
                    Record::SessionStart(_) => "session".to_owned(),
                    Record::BranchStart(_) => "branch".to_owned(),
                    Record::Frame(frame) => format!("hour {}", frame.hour),
                    Record::Control(ControlRecord::Detach { .. }) => "detach".to_owned(),
                    Record::Control(_) => "control".to_owned(),
                    other => format!("{other:?}"),
                })
                .filter(|kind| kind != "control")
                .collect::<Vec<_>>()
        };
        assert_eq!(
            summary(&records(&before)),
            ["session", "branch", "hour 10", "hour 11"]
        );
        assert_eq!(
            summary(&records(&after)),
            ["session", "branch", "hour 30", "detach"]
        );
        let replay = replay::campaign::Replay::from_recovered(
            recorder_derive::writers(),
            ID.to_vec(),
            &[
                (
                    0,
                    replay_capture_spool::spool::recover_prefix(&fs::read(&before).unwrap()),
                ),
                (1, replay_capture_spool::spool::recover_prefix(&fs::read(&after).unwrap())),
            ],
        )
        .unwrap();
        let hours: Vec<_> = replay
            .branches
            .iter()
            .flat_map(|branch| replay.recorded_hours(branch.id).unwrap())
            .collect();
        assert_eq!(hours, [10, 11, 30]);
        drop(processor);
        fs::remove_dir_all(directory).unwrap();
    }

    fn scratch_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hoi4-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("recordings")).unwrap();
        root
    }

    fn recorded_frames(path: &Path) -> Vec<(u64, i32)> {
        let replay = replay::campaign::Replay::open(fs::read(path).unwrap()).unwrap();
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| (frame.branch, frame.hour))
            .collect()
    }

    const OTHER: &[u8] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a7";

    #[test]
    fn a_campaign_recorded_again_after_a_restart_continues_its_unsaved_hours_and_both_save() {
        use recorder_replay::replay_io;
        let root = scratch_root("restart-unsaved");
        let target = root.join("recordings");
        let name = std::str::from_utf8(ID).unwrap();
        let unsaved = replay_settings::publication::unsaved_path(&target, name);
        let settings = root.join("settings.json");
        let naming = replay_io::Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        let recording = || replay_files::recording_files::recording_of(&target, ID).unwrap();
        let mut branches = Vec::new();
        for (process, signal, hours) in [(1, 1234, 10..=12), (2, 5678, 20..=21)] {
            let session = SessionStart {
                id: process,
                process_start_time: process * 1000,
                ..session()
            };
            let folder = root.join("sessions").join(process.to_string());
            let history = PreparedCampaigns::load(&target, &session, |_| Ok(())).unwrap();
            let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&folder, history, session) }.unwrap();
            for hour in hours {
                // SAFETY: This frame or slot belongs to the current admitted capture or the sole
                // spool consuming it.
                unsafe { processor.frame(&crate::spooling::campaign_frame(ID, signal, hour)) }
                    .unwrap();
            }
            branches.push(
                processor
                    .campaigns
                    .get_mut(name)
                    .unwrap()
                    .tracker
                    .current_branch(),
            );
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
            drop(processor);
            let ended = replay_io::finalize_session_ahead(
                &folder,
                &target,
                &|| Ok(()),
                replay_io::Live::default(),
                naming,
                &|_| replay_io::Destination::Unsaved,
            )
            .unwrap();
            assert!(ended.errors.is_empty(), "{:?}", ended.errors);
            assert_eq!(ended.unsaved, std::slice::from_ref(&unsaved));
            assert_eq!(recording(), None, "nothing is saved");
        }
        assert_ne!(branches[0], branches[1]);
        let saved = replay_io::save_waiting(&target, name, naming).unwrap();
        let recording = recording().unwrap();
        assert_eq!(saved, Some(recording.clone()));
        assert!(!unsaved.exists());
        let [first, second] = [branches[0].unwrap(), branches[1].unwrap()];
        assert_eq!(
            recorded_frames(&recording),
            [
                (first, 10),
                (first, 11),
                (first, 12),
                (second, 20),
                (second, 21)
            ]
        );
        let replay = replay::campaign::Replay::open(fs::read(&recording).unwrap()).unwrap();
        assert_eq!(
            replay
                .sessions
                .iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_campaign_the_game_left_is_saved_over_its_existing_recording_while_its_history_is_released()
    {
        use recorder_replay::replay_io;
        use std::os::windows::fs::OpenOptionsExt;
        let root = scratch_root("set-aside-lease");
        let target = root.join("recordings");
        let name = std::str::from_utf8(ID).unwrap();
        let other = std::str::from_utf8(OTHER).unwrap();
        let settings = root.join("settings.json");
        let naming = replay_io::Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        let end = |writers: replay_io::Live, folder: &Path| {
            let ended = replay_io::finalize_session_ahead(
                folder,
                &target,
                &|| Ok(()),
                writers,
                naming,
                &|_| replay_io::Destination::Recording,
            )
            .unwrap();
            assert!(ended.errors.is_empty(), "{:?}", ended.errors);
            ended.replays[0].clone()
        };
        let save =
            |writers: &mut replay_io::Live, folder: &Path| -> Result<Option<PathBuf>, String> {
                writers.retry();
                writers.advance(folder, &target, &|_| false, naming, &|| false)?;
                writers.save(folder, &target, name, naming)
            };
        let earlier = root.join("sessions").join("1");
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&earlier, PreparedCampaigns::default(), session()) }.unwrap();
        for hour in 10..=11 {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&crate::spooling::campaign_frame(ID, 1234, hour)) }.unwrap();
        }
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        drop(processor);
        let recording = end(replay_io::Live::default(), &earlier);
        let session = SessionStart {
            id: 2,
            process_start_time: 2000,
            ..session()
        };
        let folder = root.join("sessions").join("2");
        let history = PreparedCampaigns::load(&target, &session, |_| Ok(())).unwrap();
        let mut processor =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Processor::new(&folder, history, session) }.unwrap();
        for hour in 20..=21 {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&crate::spooling::campaign_frame(ID, 5678, hour)) }.unwrap();
        }
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&crate::spooling::campaign_frame(OTHER, 9012, 30)) }.unwrap();
        processor.settle_journals();
        let held = fs::read(&recording).unwrap();
        let conflict = fs::OpenOptions::new()
            .read(true)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(&recording)
            .unwrap();
        let mut writers = replay_io::Live::default();
        assert!(
            save(&mut writers, &folder).is_err(),
            "the restrictive reader keeps the writer out"
        );
        assert_eq!(fs::read(&recording).unwrap(), held);
        drop(conflict);
        assert_eq!(
            processor.history_request(&format!("1 release {other}")),
            "1 refused the campaign is recording"
        );
        assert_eq!(processor.history_request(&format!("5 lend {name}")), "5 ok");
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&recording)
                .is_ok(),
            "lent"
        );
        assert_eq!(
            processor.history_request(&format!("6 retake {name}")),
            "6 ok"
        );
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&recording)
                .is_err(),
            "held again after the checkpoint"
        );
        assert_eq!(
            processor.history_request(&format!("7 lend {other}")),
            "7 ok"
        );
        assert_eq!(
            processor.history_request(&format!("8 retake {other}")),
            "8 ok"
        );
        assert_eq!(
            processor.history_request(&format!("2 release {name}")),
            "2 ok"
        );
        let deferred = save(&mut writers, &folder).unwrap_err();
        assert!(
            deferred.contains("saved when recording stops"),
            "{deferred}"
        );
        assert_eq!(
            processor.history_request(&format!("3 retake {name}")),
            "3 ok"
        );
        let first = processor
            .campaigns
            .get_mut(name)
            .unwrap()
            .tracker
            .current_branch();
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&recording)
                .is_err(),
            "held again"
        );
        assert_eq!(
            processor.history_request("4 rename x"),
            "4 refused unknown request"
        );
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        drop(processor);
        assert_eq!(end(writers, &folder), recording);
        let frames = recorded_frames(&recording);
        assert_eq!(
            frames.iter().map(|(_, hour)| *hour).collect::<Vec<_>>(),
            [10, 11, 20, 21]
        );
        assert_eq!(frames[2].0, first.unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_halted_native_capture_gaps_every_later_hour_without_a_full_refresh() {
        let directory =
            std::env::temp_dir().join(format!("hoi4-native-halted-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&captured(10)) }.unwrap();
        // SAFETY: The originating spool consumes its own retry request for the recovery handoff.
        unsafe { processor.take_retry_full() };
        let campaign = processor
            .campaigns
            .get_mut(std::str::from_utf8(ID).unwrap())
            .unwrap();
        campaign.columnar = crate::columnar::Columnar::Unopened;
        for hour in [11, 12] {
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&captured(hour)) }.unwrap();
            assert!(
                !
            // SAFETY: This fixture consumes its own spool recovery state.
            unsafe { processor.take_retry_full() },
                "hour {hour}"
            );
        }
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();
        let campaign = processor
            .campaigns
            .get_mut(std::str::from_utf8(ID).unwrap())
            .unwrap();
        let (mut hours, mut gaps) = (Vec::new(), Vec::new());
        for file in fs::read_dir(&campaign.directory).unwrap().flatten() {
            if !file.file_name().to_string_lossy().starts_with("spool.") {
                continue;
            }
            for record in replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()).records {
                match record {
                    Record::Frame(frame) => hours.push(frame.hour),
                    Record::Gap(gap) => gaps.push((gap.first_hour, gap.last_hour, gap.reason)),
                    _ => {}
                }
            }
        }
        assert_eq!(hours, [10]);
        assert_eq!(
            gaps,
            [(
                11,
                12,
                "the columnar recording could not be opened".to_owned()
            )]
        );
        drop(processor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_session_spools_only_the_spooled_tables_its_mode_records() {
        use replay_data::scope::{self, CaptureMode};
        use replay_frame::frame::FrameDecoder;
        for mode in CaptureMode::ALL {
            let directory = std::env::temp_dir()
                .join(format!("hoi4-capture-mode-{mode}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&directory);
            let mut start = session();
            start.capture_mode = mode;
            let mut processor =
                // SAFETY: The caller owns the session or isolated fixture state required by this
                // constructor.
                unsafe { Processor::new(&directory, PreparedCampaigns::default(), start) }.unwrap();
            for hour in [10, 11] {
                // SAFETY: This frame or slot belongs to the current admitted capture or the sole
                // spool consuming it.
                unsafe { processor.frame(&captured(hour)) }.unwrap();
            }
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
            let campaign = processor
                .campaigns
                .get_mut(std::str::from_utf8(ID).unwrap())
                .unwrap();
            let mut decoder = FrameDecoder::new();
            let mut frames = 0;
            for file in fs::read_dir(&campaign.directory).unwrap().flatten() {
                if !file.file_name().to_string_lossy().starts_with("spool.") {
                    continue;
                }
                for record in replay_capture_spool::spool::recover_prefix(&fs::read(file.path()).unwrap()).records
                {
                    let Record::Frame(frame) = record else {
                        continue;
                    };
                    assert_eq!(frame.capture_mode().unwrap(), mode);
                    let decoded = decoder.apply(&frame).unwrap();
                    frames += 1;
                    for table in TableId::ALL {
                        let present = table.last_change().present(replay_data::FORMAT_VERSION);
                        let held = scope::records(mode, table) && super::spooled(table);
                        let view = &*decoded.tables[table.index()];
                        assert_eq!(
                            present && !held,
                            matches!(view, TableView::NotRecorded { mode: marked } if *marked == mode),
                            "{mode} {table:?}"
                        );
                    }
                    assert!(decoded.complete::<tables::Clock>().is_ok());
                }
            }
            assert_eq!(frames, 2, "{mode}");
            drop(processor);
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn a_map_session_holds_full_s_tables_for_every_table_map_records() {
        use collector_capture::sampling::capture_scope_test_kit::{self as kit, Fixture};
        use collector_workers::roles::Role;
        use replay_data::scope::{self, CaptureMode};
        use replay_frame::frame::FrameDecoder;
        crate::spooling::keep_every_table();
        const CHILD: &str = "BATTLEPLAN_MAP_DISPATCH_EQUIVALENCE";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "spooling::tests::a_map_session_holds_full_s_tables_for_every_table_map_records", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        synthetic_game::Executable::fixed_layout(engine_fixed_layout::fixed_layout::TOKEN_IDS)
            .enter();
        let exclusive_roles = [
            Role::World,
            Role::Presentation,
            Role::PresentationPrepare,
            Role::PresentationStatus,
            Role::PresentationPreparation,
            Role::PresentationSettle,
            Role::PresentationSpeedTerms,
            Role::RaidTargets,
            Role::Research,
            Role::Intel,
            Role::AirStats,
            Role::Sunk,
            Role::Projects,
            Role::Session,
            Role::Stability,
            Role::WarOverview,
            Role::IndustrialOrganisations,
            Role::ProductionDaily,
            Role::ProductionModifierParts,
            Role::SupplyTrains,
            Role::Fronts,
            Role::CountryLeaderRoles,
            Role::AdvisorRoles,
        ];
        #[cfg(feature = "acquisition_profile")]
        let exclusive_tables: Vec<_> = collector_capture::extended::mode_gated_walks()
            .into_iter()
            .flat_map(|(_, tables)| tables)
            .chain([TableId::ScriptFlags, TableId::ScriptVariables])
            .chain(collector_intel::intel::TABLES)
            .chain(collector_governance::governance::TABLES)
            .filter(|&table| {
                table != TableId::DivisionStatus && !scope::records(CaptureMode::Map, table)
            })
            .collect();
        let frames = |mode: CaptureMode, fixture: Fixture, refuse: bool| {
            let directory = scratch_root(&format!("capture-scope-{mode}-{fixture:?}-{refuse}"));
            let mut start = session();
            start.capture_mode = mode;
            let mut processor =
                // SAFETY: The caller owns the session or isolated fixture state required by this
                // constructor.
                unsafe { Processor::new(&directory, PreparedCampaigns::default(), start) }.unwrap();
            collector_map_events::interval::begin(0, 0).unwrap();
            // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
            unsafe {
                kit::capture(mode, fixture, refuse, |frame, trace, roles| {
                    let hour = frame.clock.as_ref().unwrap().hour;
                    if mode == CaptureMode::Map {
                        assert!(
                            roles.iter().all(|role| !exclusive_roles.contains(role)),
                            "{fixture:?} hour {hour}: {roles:?}"
                        );
                        assert!(trace.iter().all(|span| {
                            span.role
                                .is_none_or(|role| !exclusive_roles.contains(&role))
                        }));
                        #[cfg(feature = "acquisition_profile")]
                        for &table in &exclusive_tables {
                            assert_eq!(
                                frame.table_reads[table.index()],
                                engine_native_memory::memory::ReadCounts::ZERO,
                                "{fixture:?} hour {hour} {table:?}"
                            );
                        }
                    } else if hour == 0 {
                        for role in [Role::Research, Role::Intel, Role::AirStats] {
                            assert!(roles.contains(&role), "Full must exercise {role:?}");
                        }
                    }
                    for field in [
                        engine_native::cgame_state::GLOBAL_FLAGS,
                        engine_native::cgame_state::GLOBAL_VARIABLES,
                    ] {
                        let field = frame.clock.as_ref().unwrap().game_state as usize + field;
                        let touched = trace.iter().any(|span| {
                            span.address < field + 8
                                && field < span.address.saturating_add(span.len)
                        });
                        if mode == CaptureMode::Map {
                            assert!(
                                !touched,
                                "omitted script source read at hour {hour}: {trace:?}"
                            );
                        } else if hour == 0 {
                            assert!(touched, "Full did not exercise the script source");
                        }
                    }
                    if matches!(fixture, Fixture::Divisions) {
                        if refuse && matches!(hour, 2 | 13) {
                            assert!(matches!(
                                frame.divisions_result,
                                Outcome::Unavailable { .. }
                            ));
                        } else {
                            assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
                        }
                    }
                    processor.frame(frame).unwrap();
                })
            };
            // SAFETY: This spool's frame and command processing have finished before its processor
            // is closed.
            unsafe { processor.detach() }.unwrap();
            let campaign = processor
                .campaigns
                .get_mut(std::str::from_utf8(kit::ID).unwrap())
                .unwrap();
            let mut spools: Vec<_> = fs::read_dir(&campaign.directory)
                .unwrap()
                .flatten()
                .filter(|file| file.file_name().to_string_lossy().starts_with("spool."))
                .map(|file| file.path())
                .collect();
            spools.sort();
            let mut decoder = FrameDecoder::new();
            let mut frames = Vec::new();
            for spool in spools {
                for record in replay_capture_spool::spool::recover_prefix(&fs::read(spool).unwrap()).records {
                    let Record::Frame(frame) = record else {
                        continue;
                    };
                    assert_eq!(frame.capture_mode().unwrap(), mode);
                    let decoded = decoder.apply(&frame).unwrap();
                    let hour = frames.len();
                    match fixture {
                        Fixture::Divisions => {
                            let table = &*decoded.tables[TableId::DivisionEquipment.index()];
                            if refuse && (2..26).contains(&hour) {
                                assert!(
                                    matches!(table, TableView::Unavailable { .. }),
                                    "{mode} hour {hour}: {table:?}"
                                );
                            } else {
                                let TableView::Complete(TableRows::DivisionEquipment(rows)) = table
                                else {
                                    panic!(
                                        "{mode} hour {hour}: expected populated equipment: {table:?}"
                                    );
                                };
                                let sample_hour = if hour >= 26 {
                                    26
                                } else if hour >= 13 {
                                    13
                                } else if hour >= 2 {
                                    2
                                } else {
                                    0
                                };
                                let first = rows
                                    .iter()
                                    .find(|row| row.division == 101 && row.ordinal == 0)
                                    .unwrap();
                                assert_eq!(
                                    first.amount_raw,
                                    100 + sample_hour,
                                    "{mode} held equipment at hour {hour}"
                                );
                            }
                        }
                        Fixture::Wars if !refuse || !matches!(hour, 2 | 13) => {
                            let table = &*decoded.tables[TableId::WarRelations.index()];
                            let TableView::Complete(TableRows::WarRelations(rows)) = table else {
                                panic!("{mode} hour {hour}: expected populated wars: {table:?}");
                            };
                            let war = rows
                                .iter()
                                .find(|row| row.first_country == 3 && row.second_country == 1)
                                .unwrap();
                            assert_eq!(
                                war.first_casualties,
                                11 + hour as i64,
                                "{mode} hourly war at hour {hour}"
                            );
                        }
                        Fixture::Wars => {}
                    }
                    frames.push(
                        TableId::ALL
                            .map(|table| (*decoded.tables[table.index()]).clone())
                            .to_vec(),
                    );
                }
            }
            drop(processor);
            fs::remove_dir_all(directory).unwrap();
            frames
        };
        for fixture in [Fixture::Divisions, Fixture::Wars] {
            for refuse in [false, true] {
                let full = frames(CaptureMode::Full, fixture, refuse);
                let map = frames(CaptureMode::Map, fixture, refuse);
                assert_eq!((full.len(), map.len()), (27, 27));
                let mut omitted_slice_reads = 0;
                for (hour, (full, map)) in full.iter().zip(&map).enumerate() {
                    for table in TableId::ALL {
                        if !table.last_change().present(replay_data::FORMAT_VERSION) {
                            continue;
                        }
                        let index = table.index();
                        if table == TableId::SliceReads {
                            let (
                                TableView::Complete(TableRows::SliceReads(full)),
                                TableView::Complete(TableRows::SliceReads(map)),
                            ) = (&full[index], &map[index])
                            else {
                                panic!("SliceReads must be complete");
                            };
                            let kept: Vec<_> = full
                                .iter()
                                .filter(|row| {
                                    scope::records(
                                        CaptureMode::Map,
                                        TableId::ALL[row.table as usize],
                                    )
                                })
                                .cloned()
                                .collect();
                            omitted_slice_reads += full.len() - kept.len();
                            assert_eq!(
                                *map, kept,
                                "{fixture:?} refusal={refuse} hour {hour} SliceReads"
                            );
                        } else if scope::records(CaptureMode::Map, table) {
                            if scope::records(CaptureMode::Full, table) {
                                assert_eq!(
                                    map[index], full[index],
                                    "{fixture:?} refusal={refuse} hour {hour} {table:?}"
                                );
                            }
                        } else {
                            assert_eq!(
                                map[index],
                                TableView::NotRecorded {
                                    mode: CaptureMode::Map
                                },
                                "{table:?}"
                            );
                        }
                    }
                }
                if matches!(fixture, Fixture::Wars) {
                    assert!(
                        omitted_slice_reads > 0,
                        "exercise the omitted RelationOpinions witnesses"
                    );
                }
            }
        }
    }

    #[test]
    fn spool_validation_detail_survives_ordinary_failure_and_detach() {
        const TEST: &str =
            "spooling::tests::spool_validation_detail_survives_ordinary_failure_and_detach";
        const CHILD: &str = "BATTLEPLAN_SPOOL_VALIDATION_DETAIL_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        collector_memory::terminal::install_hooks();
        let directory = std::env::temp_dir().join(format!(
            "hoi4-spool-validation-detail-{}",
            std::process::id()
        ));
        let mut processor =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        let mut source = captured(10);
        // SAFETY: The caller owns the session and runs preparation before admitting consumers.
        unsafe { processor.prepare(&source) }.unwrap();
        // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
        // consuming it.
        unsafe { processor.frame(&source) }.unwrap();

        source = captured(11);
        let mut prepared =
        // SAFETY: This is the decoded frame held by its originating spool.
        unsafe { processor.prepare_frame(&source) }.unwrap();
        prepared.source.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: 12,
                ..Default::default()
            }]))
            .into();
        let detail = "Clock: clock row hour differs from frame hour";
        let expected = format!("spool validation failed: {detail}");
        assert_eq!(
            // SAFETY: This spool submits the prepared frame to its originating processor before
            // teardown.
            unsafe { processor.write_prepared(prepared) }
                .unwrap_err()
                .to_string(),
            detail
        );
        let campaign = processor
            .campaigns
            .get_mut(std::str::from_utf8(ID).unwrap())
            .unwrap();
        assert!(!campaign.spool.is_open());
        assert!(!campaign.pending.is_empty());
        let loss = LostObservation {
            campaign_id: owned_text(ID),
            first_hour: 12,
            last_hour: 12,
            count: 2,
            ..Default::default()
        };
        assert_eq!(
            // SAFETY: This isolated spool fixture submits its own captured loss.
            unsafe { processor.lost(&loss) }.unwrap_err().to_string(),
            "queue loss witness has inconsistent hours and count"
        );
        // SAFETY: This spool's frame and command processing have finished before its processor is
        // closed.
        unsafe { processor.detach() }.unwrap();

        let mut bytes = Vec::new();
        collector_status::status::serialize_into(&mut bytes, &processor.status().unwrap()).unwrap();
        let status_path = directory.join("status.json");
        fs::write(&status_path, bytes).unwrap();
        let persisted: Value = serde_json::from_slice(&fs::read(status_path).unwrap()).unwrap();
        assert_eq!(persisted["write_errors"], 2);
        assert_eq!(persisted["last_error"], expected);

        let mut preflight =
            // SAFETY: The caller owns the session or isolated fixture state required by this
            // constructor.
            unsafe { Processor::new(&directory, PreparedCampaigns::default(), session()) }.unwrap();
        assert_eq!(
            // SAFETY: The caller owns the session and runs preparation before admitting consumers.
            unsafe { preflight.prepare(&source) }
                .unwrap_err()
                .to_string(),
            "attach output already contains spool segments; use a fresh attach directory"
        );
        let loss = LostObservation {
            campaign_id: owned_text(ID),
            first_hour: 12,
            last_hour: 12,
            count: 2,
            ..Default::default()
        };
        assert_eq!(
            // SAFETY: This isolated spool fixture submits its own captured loss.
            unsafe { processor.lost(&loss) }.unwrap_err().to_string(),
            "queue loss witness has inconsistent hours and count"
        );
        assert_eq!(processor.json()["write_errors"], 3);
        assert_eq!(processor.json()["last_error"], expected);
        assert!(invalid("callback refusal").get_ref().is_none());
        drop(preflight);
        drop(processor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cached_callback_summary_does_not_freeze_status_errors_or_latest_cost() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            session_verified: false,
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helpers: Vec::new(),
            backlogged: false,
            comparison: Box::default(),
            writer: crate::spool_writer::Writer::start(PathBuf::new()).unwrap(),
            journal_thread: crate::columnar::JournalThread::start().unwrap(),
            queued: std::collections::VecDeque::new(),
            flight: None,
            writer_error: None,
            command_drain: Default::default(),
            command_pending: Vec::new(),
            command_contexts: Default::default(),
            command_anchors: Default::default(),
            command_ordinals: Default::default(),
            command_definitions: Default::default(),
            command_last_flush: std::time::Instant::now(),
            command_ready: Default::default(),
            command_inflight: Default::default(),
        };
        for ns in [100, 200, 300] {
            processor.stats.callback.record_ns(ns);
        }
        processor.stats.hours = 3;
        processor.stats.total_ns = 300;
        let first = processor.json();
        assert_eq!(first["p95_ns"], 300);
        assert_eq!(first["total_ns"], 300);
        assert_eq!(first["callback_work"]["calls"], 3);
        assert_eq!(first["callback_work"]["total_ns"], 600);
        let repeated = processor.json();
        for field in ["hours", "total_ns", "p95_ns", "callback_work"] {
            assert_eq!(repeated[field], first[field], "{field}");
        }

        processor.observe_input_demand_error("country roster header unreadable or invalid");
        processor.observe_input_demand_error("unit leader identity unreadable");
        assert_eq!(processor.stats.input_demand_errors, 2);
        assert_eq!(
            processor.stats.input_demand_last_error,
            Some("unit leader identity unreadable")
        );
        assert_eq!(processor.json()["write_errors"], 0);

        processor.error(&io::Error::other("new disk failure while paused"));
        let failed = processor.json();
        assert_eq!(failed["last_error"], "spool I/O failed");
        assert_eq!(failed["write_errors"], 1);
        assert_eq!(failed["p95_ns"], first["p95_ns"]);

        processor.stats.callback.record_ns(50);
        processor.stats.hours += 1;
        processor.stats.total_ns = 50;
        let resumed = processor.json();
        assert_eq!(resumed["hours"], 4);
        assert_eq!(resumed["total_ns"], 50);
        assert_eq!(resumed["callback_work"]["calls"], 4);
        assert_eq!(resumed["callback_work"]["total_ns"], 650);
        assert_eq!(resumed["p95_ns"], 300);
        assert_eq!(resumed["last_error"], failed["last_error"]);
    }

    use engine_native_memory::msvc_string::Text;
    use replay_capture_spool::spool::{SegmentSink, recover_prefix};
    use replay_frame::{branches::BranchReason, branches::Parentage};
    use std::{cell::RefCell, io::Write, rc::Rc};

    fn text<const N: usize>(value: &[u8]) -> Text<N> {
        let mut result = Text {
            len: value.len(),
            ..Text::default()
        };
        result.bytes[..value.len()].copy_from_slice(value);
        result
    }

    fn owned_prepared_fixture(captured: &Captured, branch: BranchId) -> PreparedFrame {
        let clock = captured.clock.as_ref().unwrap().copy_owned().unwrap();
        let source: FrameInput = decoded(captured, clock.clone(), branch).into();
        let retry_full = needs_retry(&source.tables);
        PreparedFrame {
            writer_epoch: 0,
            source,
            id: std::str::from_utf8(ID).unwrap().into(),
            clock,
            qpc: captured.qpc,
            unknown: 0,
            retry_full,
            unpublished_families: 0,
            work: WorkSample::default(),
            sampled: CapturedStatistics::capture(captured).unwrap(),
            operation_publication: None,
            map_event_publication: None,
            event_feed_publication: None,
        }
    }

    #[test]
    fn prepared_output_statistics_survive_capture_reuse() {
        let mut source = captured(10);
        source.total_ns = 123;
        source.pool_bytes = 789;
        source.dropped_frames = 2;
        let expected = decoded(&source, clock(10), 1);
        let prepared = owned_prepared_fixture(&source, 1);
        let _ = std::mem::replace(&mut source, captured(11));
        let PreparedFrame {
            source: owned,
            sampled,
            ..
        } = prepared;
        let mut encoder = FrameEncoder::new();
        let encoded = encoder.encode_input(owned, true).unwrap();
        let actual = replay_frame::frame::FrameDecoder::new()
            .apply(&encoded)
            .unwrap();
        assert_eq!(actual.tables, expected.tables);
        assert_eq!(actual.metrics, expected.metrics);
        let mut stats = Statistics::default();
        sampled.record(&mut stats).unwrap();
        assert_eq!(stats.total_ns, 123);
        assert_eq!(stats.pool_bytes, 789);
        assert_eq!(stats.dropped_frames, 2);
    }

    #[test]
    fn prepared_retry_verdict_retains_frame() {
        let source = captured(10);
        let mut prepared = owned_prepared_fixture(&source, 1);
        prepared.source.tables = TableId::ALL
            .map(|table| TableInput::Sampled(TableView::Complete(TableRows::empty(table))));
        prepared.retry_full = needs_retry(&prepared.source.tables);
        assert!(!prepared.needs_retry());
        assert!(prepared.allows_early_recycle());
        let static_table = TableId::ALL
            .into_iter()
            .find(|table| {
                replay_data::cadence::cadence(*table) == replay_data::cadence::Cadence::Static
            })
            .unwrap();
        prepared.source.tables[static_table.index()] =
            unavailable("synthetic static failure").into();
        prepared.retry_full = needs_retry(&prepared.source.tables);
        assert!(prepared.needs_retry());
        assert!(!prepared.allows_early_recycle());
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { collector_capture::sampling::Shared::new(0, 0, 1) };
        if prepared.needs_retry() {
            // SAFETY: The spool requests recovery after unpublished output in the current session.
            unsafe { shared.request_retry_full() };
        }
        assert!(
            // SAFETY: This is the isolated fixture's refresh flag.
            unsafe { shared.refresh_next() }.load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(!
            // SAFETY: This is the isolated fixture's recovery flag.
            unsafe { shared.force_next() }.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn conversion_keeps_native_values_aliases_and_raw_text() {
        let mut captured = captured(10);
        captured.countries[0] = engine_native::ccountry::Row {
            politics_witness: None,
            id: 7,
            canonical_slot: 1,
            tag: text(b"GER"),
            political_power_raw: None,
            command_power_raw: i64::MIN,
            stability_raw: 100_001,
            war_support_raw: -9_007_199_254_740_993,
            convoys_raw: Some(i64::MAX),
            peace_conference_participant: 1,
            manpower_residual: -19,
            border_land_doctrine_level: None,
        };
        captured.countries_result = Outcome::Rows { len: 1 };
        captured.links[0] = engine_native::ccountry::LinkRow {
            id: 2,
            canonical_slot: 1,
        };
        captured.links_result = Outcome::Rows { len: 1 };
        captured.provinces.rows[0] = engine_native::cprovince::Row {
            id: 6,
            state: 4,
            owner: 7,
            controller: 2,
            victory_points: i32::MIN,
            ..Default::default()
        };
        captured.provinces_result = Outcome::Rows { len: 1 };
        captured.states[0] = engine_native::cstate::Row {
            id: 4,
            owner: 7,
            controller: u32::MAX,
            ..Default::default()
        };
        captured.states_result = Outcome::Rows { len: 1 };
        captured.divisions[0] = engine_native::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 4713,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 6,
            template: u64::MAX,
            hit_points: i64::MIN,
            organisation: 9_007_199_254_740_993,
            orders_group: (57_u64 << 32) | 53,
            reveal_skip: 0,
            theatre: Some(0x0000_0011_0000_0043),
        };
        captured.divisions[1] = engine_native::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 51,
            ..captured.divisions[0]
        };
        captured.divisions_result = Outcome::Rows { len: 2 };
        let frame = decoded(&captured, clock(10), 1);
        assert_eq!(
            frame.tables[2],
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
                id: 7,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                political_power_raw: None,
                command_power_raw: i64::MIN,
                stability_raw: 100_001,
                war_support_raw: -9_007_199_254_740_993,
                convoys_raw: Some(i64::MAX),
                peace_conference_participant: 1,
                manpower_residual: -19,
            }]))
        );
        let TableView::Complete(divisions) = &*frame.tables[TableId::Divisions.index()] else {
            panic!("distinct full division handles must survive conversion");
        };
        assert_eq!(
            divisions
                .rows::<tables::Divisions>()
                .unwrap()
                .iter()
                .map(DivisionRow::key)
                .collect::<Vec<_>>(),
            vec![
                u128::from(captured.divisions[1].id),
                u128::from(captured.divisions[0].id)
            ]
        );
        for division in divisions.rows::<tables::Divisions>().unwrap() {
            assert_eq!(division.hit_points, i64::MIN);
            assert_eq!(division.organisation, 9_007_199_254_740_000);
            assert_eq!(division.template, u64::MAX);
            assert_eq!(division.expeditionary_owner, 0);
            assert_eq!(division.orders_group, (57_u64 << 32) | 53);
            assert_eq!(
                division.theatre,
                replay_data::rows::Recorded::Value(Some(0x0000_0011_0000_0043))
            );
        }
        assert_eq!(
            frame.tables[3],
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 2,
                canonical_slot: 1
            }]))
        );
        assert_eq!(
            frame.tables[4],
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 6,
                state: 4,
                owner: 7,
                controller: 2,
                victory_points: i32::MIN,
                fort_positive: 0,
                move_gate: 0,
                port_sea: 0,
                attack: Recorded::Value(replay_data::rows::Witnessed(None)),
                selected_areas: Default::default(),
                fort_bunker: Recorded::Value(None),
                controller_area: Default::default(),
                owner_area: Default::default(),
                area_node: Default::default(),
            }]))
        );
        let value = &frame.complete::<tables::Clock>().unwrap()[0];
        assert_eq!(value.play_time, u64::MAX);
        assert_eq!(
            frame.tables[TableId::States.index()],
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 4,
                owner: 7,
                controller: u32::MAX,
                movement_infrastructure: 0,
                railway_infrastructure: replay_data::rows::Recorded::Value(None),
                continent: replay_data::rows::Recorded::Value(None),
                ..Default::default()
            }]))
        );
    }

    #[test]
    fn failures_remain_local_and_missing_clock_invalidates_conversion() {
        let mut captured = captured(10);
        captured.countries_result = Outcome::Unavailable {
            reason: "country pointer unreadable",
        };
        captured.links_result = Outcome::Truncated {
            len: 0,
            unread_rows: 9,
        };
        captured.states_result = Outcome::Unavailable {
            reason: "state pointer unreadable",
        };
        captured.divisions_result = Outcome::Unavailable {
            reason: "division pointer unreadable",
        };
        let frame = decoded(&captured, clock(10), 1);
        assert_eq!(frame.tables[2], unavailable("country pointer unreadable"));
        assert_eq!(
            frame.tables[3],
            TableView::Truncated {
                rows: TableRows::CountryLinks(Vec::new()),
                missing: 9
            }
        );
        assert_eq!(
            frame.tables[4],
            TableView::Complete(TableRows::Provinces(Vec::new()))
        );
        captured.provinces_result = Outcome::Unavailable {
            reason: "province pointer unreadable",
        };
        assert_eq!(
            decoded(&captured, clock(10), 1).tables[4],
            unavailable("province pointer unreadable")
        );
        captured.provinces_result = Outcome::Rows { len: 0 };
        assert_eq!(
            decoded(&captured, clock(10), 1).tables[4],
            TableView::Complete(TableRows::Provinces(Vec::new()))
        );
        assert_eq!(
            frame.tables[TableId::States.index()],
            unavailable("state pointer unreadable")
        );
        assert_eq!(
            frame.tables[TableId::Divisions.index()],
            unavailable("division pointer unreadable")
        );
        captured.clock = None;
        let mut expected =
            unavailable_tables("clock unreadable as its capture began").map(|input| match input {
                TableInput::Sampled(view) => view,
                _ => unreachable!("fixture expected ordinary sampled rows"),
            });
        for table in TableId::ALL {
            if let Some(view) = replay_frame::frame::current_absence(table) {
                expected[table.index()] = view.into();
            }
        }
        assert_eq!(decoded(&captured, clock(10), 1).tables, expected);
    }

    #[test]
    fn border_conflicts_are_truncated_or_unavailable_with_their_provinces() {
        let rows = |len| Outcome::Rows { len };
        let truncated = |len, unread_rows| Outcome::Truncated { len, unread_rows };
        let failed = Outcome::Unavailable {
            reason: "conflicts",
        };
        let provinces = Outcome::Unavailable {
            reason: "provinces",
        };
        assert_eq!(border_conflicts_outcome(rows(9), rows(3)), rows(3));
        assert_eq!(border_conflicts_outcome(rows(9), failed), failed);
        assert_eq!(border_conflicts_outcome(provinces, rows(3)), provinces);
        assert_eq!(
            border_conflicts_outcome(truncated(9, 4), rows(3)),
            truncated(3, 4)
        );
        assert_eq!(
            border_conflicts_outcome(truncated(9, 4), truncated(3, 2)),
            truncated(3, 6)
        );
        let mut captured = captured(10);
        captured.provinces_result = truncated(0, 5);
        captured.provinces.conflict_walk.outcome = rows(0);
        let frame = decoded(&captured, clock(10), 1);
        assert!(matches!(
            *frame.tables[TableId::BorderConflicts.index()],
            TableView::Truncated { .. }
        ));
    }

    #[test]
    fn primary_failures_and_truncation_keep_independent_values() {
        let mut source = captured(10);
        source.states_result = Outcome::Unavailable {
            reason: "primary state pointer unreadable",
        };
        source.divisions_result = Outcome::Truncated {
            len: 0,
            unread_rows: 3,
        };
        source.factions[0] = engine_native::cfaction::Row {
            id: 88,
            name_start: 0,
            name_len: 3,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.faction_names[..3].copy_from_slice(&[b'N', 0, 255]);
        source.faction_names_len = 3;
        let frame = decoded(&source, clock(10), 1);
        for table in [
            TableId::CountryLinks,
            TableId::WarRelations,
            TableId::FactionMembers,
        ] {
            assert_eq!(
                frame.tables[table.index()],
                TableView::Complete(TableRows::empty(table))
            );
        }
        assert_eq!(
            frame.tables[TableId::States.index()],
            unavailable("primary state pointer unreadable")
        );
        assert_eq!(
            frame.tables[TableId::Divisions.index()],
            TableView::Truncated {
                rows: TableRows::Divisions(Vec::new()),
                missing: 3
            }
        );
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: 88,
                name: vec![b'N', 0, 255],
                ..FactionRow::default()
            }]))
        );
    }

    #[test]
    fn healthy_hours_restore_failed_baselines_and_keep_new_values() {
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};

        let mut source = captured(10);
        source.states_result = Outcome::Unavailable {
            reason: "state pointer unreadable",
        };
        source.factions_result = Outcome::Unavailable {
            reason: "faction pointer unreadable",
        };
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let failed = encoder
            .encode(decoded(&source, clock(10), 1), true)
            .unwrap();
        decoder.apply(&failed).unwrap();

        source.states_result = Outcome::Rows { len: 1 };
        source.states[0] = engine_native::cstate::Row {
            id: 4,
            owner: 7,
            controller: 7,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.factions[0] = engine_native::cfaction::Row {
            id: 88,
            name_start: 0,
            name_len: 3,
            ..Default::default()
        };
        source.faction_names_len = 3;
        for hour in [11, 12] {
            source.clock = Some(clock(hour));
            source.states[0].controller = hour as u32;
            source.faction_names[..3].copy_from_slice(if hour == 11 { b"one" } else { b"two" });
            let expected = decoded(&source, clock(hour), 1);
            let frame = encoder.encode(expected.clone(), false).unwrap();
            for table in [TableId::States, TableId::Factions] {
                if hour == 11 {
                    assert!(matches!(
                        frame.tables[table.index()],
                        TableUpdate::Snapshot(_)
                    ));
                } else {
                    assert!(matches!(
                        frame.tables[table.index()],
                        TableUpdate::Delta { .. }
                    ));
                }
            }
            assert_eq!(decoder.apply(&frame).unwrap().tables, expected.tables);
        }
    }

    #[test]
    fn relation_and_faction_conversion_preserves_native_fields_and_member_order() {
        let mut source = captured(10);
        source.wars[0] = engine_native::cwar_relation::Row {
            first_country: u32::MAX,
            second_country: 1,
            start_hour: i32::MIN,
            end_hour: i32::MAX,
            cancel: 255,
            first_casualties: i64::MAX,
            second_casualties: i64::MIN,
            first_unknown_casualties: -1,
            second_unknown_casualties: 1_i64 << 40,
            first_was_instigator: 254,
        };
        source.wars_result = Outcome::Rows { len: 1 };
        source.factions[0] = engine_native::cfaction::Row {
            id: u64::MAX,
            name_start: 2,
            name_len: 4,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.faction_names[..6].copy_from_slice(&[0, 0, b'A', 0, 255, b'B']);
        source.faction_names_len = 6;
        source.faction_members[0] = engine_native::cfaction::system::MemberRow {
            country: 9,
            faction: u64::MAX,
            ordinal: 0,
        };
        source.faction_members[1] = engine_native::cfaction::system::MemberRow {
            country: 1,
            faction: u64::MAX,
            ordinal: 1,
        };
        source.faction_members_result = Outcome::Rows { len: 2 };
        let frame = decoded(&source, clock(10), 1);
        let wars = frame.complete::<tables::WarRelations>().unwrap();
        let war = &wars[0];
        assert_eq!(wars[0].key(), (u128::from(u32::MAX) << 32) | 1);
        assert_eq!((war.cancel, war.first_was_instigator), (255, 254));
        assert_eq!(
            (
                war.first_casualties,
                war.second_casualties,
                war.second_unknown_casualties
            ),
            (i64::MAX, i64::MIN, 1_i64 << 40)
        );
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: u64::MAX,
                name: vec![b'A', 0, 255, b'B'],
                ..FactionRow::default()
            }]))
        );
        assert_eq!(
            frame.tables[TableId::FactionMembers.index()],
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 1,
                    faction: u64::MAX,
                    ordinal: 1
                },
                FactionMemberRow {
                    country: 9,
                    faction: u64::MAX,
                    ordinal: 0
                },
            ]))
        );
        let encoded = FrameEncoder::new().encode(frame.clone(), true).unwrap();
        let decoded = replay_frame::frame::FrameDecoder::new()
            .apply(&encoded)
            .unwrap();
        assert_eq!(decoded.tables, frame.tables);
    }

    #[test]
    fn faction_byte_ranges_and_primary_failures_cannot_publish_partial_values_as_complete() {
        let mut source = captured(10);
        source.factions[0] = engine_native::cfaction::Row {
            id: 88,
            name_start: usize::MAX,
            name_len: 1,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        assert_eq!(
            faction_table(&source),
            unavailable("faction name lies outside the captured byte arena")
        );
        source.faction_members_result = Outcome::Unavailable {
            reason: "faction member pointer unreadable",
        };
        let frame = decoded(&source, clock(10), 1);
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            unavailable("faction name lies outside the captured byte arena")
        );
        assert_eq!(
            frame.tables[TableId::FactionMembers.index()],
            unavailable("faction member pointer unreadable")
        );
    }

    #[test]
    fn table_conversion_rejects_duplicate_keys_and_path_validation_is_strict() {
        let source = [(2, i32::MIN), (1, i32::MAX)];
        let values = table_iter::<tables::Provinces, _>(
            Outcome::Truncated {
                len: 2,
                unread_rows: 3,
            },
            source.into_iter(),
            |(id, victory_points)| ProvinceRow {
                id,
                victory_points,
                ..Default::default()
            },
        );
        assert_eq!(
            values,
            TableView::Truncated {
                rows: TableRows::Provinces(vec![
                    ProvinceRow {
                        id: 1,
                        victory_points: i32::MAX,
                        ..Default::default()
                    },
                    ProvinceRow {
                        id: 2,
                        victory_points: i32::MIN,
                        ..Default::default()
                    },
                ]),
                missing: 3,
            }
        );
        assert_eq!(
            table_iter::<tables::Provinces, _>(
                Outcome::Rows { len: 1 },
                source.into_iter(),
                |(id, victory_points)| ProvinceRow {
                    id,
                    victory_points,
                    ..Default::default()
                }
            ),
            unavailable("walker row count exceeds allocated buffer"),
        );
        assert_eq!(
            table::<tables::Provinces, _>(Outcome::Rows { len: 2 }, &[1, 1], |id| ProvinceRow {
                id: *id,
                ..Default::default()
            }),
            unavailable("walker produced duplicate engine ids")
        );
        assert_eq!(
            table::<tables::Provinces, _>(Outcome::Rows { len: 3 }, &[1], |id| ProvinceRow {
                id: *id,
                ..Default::default()
            }),
            unavailable("walker row count exceeds allocated buffer")
        );
        assert_eq!(campaign_name(ID).unwrap().as_bytes(), ID);
        let mixed_case = b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6";
        assert_eq!(campaign_name(mixed_case).unwrap().as_bytes(), mixed_case);
        for id in [
            b"../outside".as_slice(),
            b"C:\\outside",
            b"21b7cc84/3028-4f65-8e0d-adf2f37ca1a6",
            b"",
        ] {
            assert!(campaign_name(id).is_err());
        }
    }

    #[derive(Default)]
    struct Store {
        segments: BTreeMap<u32, Vec<u8>>,
        fail_frame_flush: bool,
    }

    struct MemoryBackend(Rc<RefCell<Store>>);
    struct MemorySink {
        store: Rc<RefCell<Store>>,
        index: u32,
        last_tag: u8,
    }
    impl SegmentBackend for MemoryBackend {
        type Sink = MemorySink;
        fn create(&mut self, index: u32) -> io::Result<Self::Sink> {
            self.0.borrow_mut().segments.insert(index, Vec::new());
            Ok(MemorySink {
                store: self.0.clone(),
                index,
                last_tag: 255,
            })
        }
    }
    impl Write for MemorySink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.last_tag = bytes[replay_capture_spool::spool::RECORD_HEADER_BYTES];
            self.store
                .borrow_mut()
                .segments
                .get_mut(&self.index)
                .unwrap()
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            let mut store = self.store.borrow_mut();
            if store.fail_frame_flush && self.last_tag == 2 {
                store.fail_frame_flush = false;
                return Err(io::Error::other("injected flush failure"));
            }
            Ok(())
        }
    }
    impl SegmentSink for MemorySink {
        fn sync_all(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn campaign() -> (Campaign<MemoryBackend>, Rc<RefCell<Store>>) {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(session().id).unwrap();
        campaign_with(tracker)
    }

    fn campaign_with(tracker: BranchTracker) -> (Campaign<MemoryBackend>, Rc<RefCell<Store>>) {
        let store = Rc::new(RefCell::new(Store::default()));
        let session = session();
        (
            Campaign {
                validation_refusals: ValidationRefusals::default(),
                world_generation: None,
                directory: PathBuf::new(),
                session,
                resumed: tracker.last_observation(),
                tracker,
                encoder: FrameEncoder::new(),
                spool: crate::spool_writer::Output::Direct(SegmentedSpool::new(MemoryBackend(
                    store.clone(),
                ))),
                disk_branch: None,
                last_written: None,
                pending: Vec::new(),
                submitted_context: Vec::new(),
                closing: Vec::new(),
                staggered_cache: TableCache::new(),
                staggered_complete: [false; TABLE_COUNT],
                staggered_branch: None,
                country_intel_sample: None,
                country_intel_hour: None,
                sliced_cache: TableCache::new(),
                sliced_owners: std::array::from_fn(|_| None),
                sliced_reads: ReadHours::new(),
                sliced_branch: None,
                pending_slices: Vec::new(),
                retired: Vec::new(),
                line_output: Default::default(),
                operation_outcomes: Default::default(),
                map_events: Default::default(),
                politics: Default::default(),
                construction_ic: Default::default(),
                fuel_awards: Default::default(),
                variables_scratch: ScriptVariableComparison::default(),
                character_shadow: CharacterShadow::default(),
                intel_shadow: IntelShadow::default(),
                projection_scratch: ProjectionScratch::default(),
                lexical_conversion: LexicalConversionStats::default(),
                history_lease: None,
                columnar: crate::columnar::Columnar::off(),
            },
            store,
        )
    }

    fn publish(
        campaign: &mut Campaign<MemoryBackend>,
        hour: i32,
        signal: i64,
    ) -> io::Result<BranchId> {
        let branch = campaign.observe(Some(hour - 1), hour, Some(signal))?;
        let source = decoded(&captured(hour), clock(hour), branch);
        campaign.write(
            source,
            1000 + i64::from(hour),
            Some(signal),
            0,
            &mut WorkSample::default(),
        )?;
        Ok(branch)
    }

    #[test]
    fn uncertain_frame_is_not_retried_and_reload_losses_keep_their_branches() {
        let (mut campaign, store) = campaign();
        let first = publish(&mut campaign, 10, 1).unwrap();
        store.borrow_mut().fail_frame_flush = true;
        assert!(publish(&mut campaign, 11, 1).is_err());
        assert_eq!(
            campaign.tracker.recorded_hours(first).unwrap().to_vec(),
            vec![10]
        );
        campaign
            .lost(
                &LostObservation {
                    campaign_id: owned_text(ID),
                    first_hour: 5,
                    last_hour: 7,
                    loaded_hour: Some(4),
                    load_signal: Some(2),
                    count: 3,
                },
                false,
            )
            .unwrap();
        let second = campaign.tracker.current_branch().unwrap();
        let third = publish(&mut campaign, 11, 3).unwrap();
        assert_ne!(second, third);
        assert_eq!(
            campaign.tracker.branch_start(third).unwrap().parent,
            Parentage::Probable {
                branch: first,
                hour: 10
            }
        );
        assert_eq!(
            campaign.tracker.branch_start(second).unwrap().reason,
            BranchReason::LoadSignal
        );
        campaign.detach(0).unwrap();
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        let replay = Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &recovered).unwrap();
        assert_eq!(replay.recorded_hours(first).unwrap(), vec![10, 11]);
        assert!(matches!(
            replay.seek(first, 11).unwrap(),
            SeekResult::Frame(_)
        ));
        for hour in 5..=7 {
            assert!(
                matches!(replay.seek(second, hour).unwrap(), SeekResult::Gap { reason, .. } if reason == "collector queue overflow")
            );
        }
        assert_eq!(replay.branches.len(), 3);
        assert!(matches!(
            replay.seek(third, 11).unwrap(),
            SeekResult::Frame(_)
        ));
    }

    #[test]
    fn end_of_hour_reload_starts_at_the_loaded_hour_and_detach_invents_no_sample() {
        let (mut campaign, store) = campaign();
        let write = |campaign: &mut Campaign<MemoryBackend>, hour: i32, signal: i64| {
            let mut source = captured(hour);
            source.clock.as_mut().unwrap().play_session_started_at = signal;
            let current = source.clock.as_ref().unwrap().copy_owned().unwrap();
            let origin = campaign.tracker.current_branch().map(|_| current.hour);
            assert!(origin.is_none() || origin == Some(hour));
            let branch = campaign.observe(origin, hour, Some(signal)).unwrap();
            let frame = decoded(&source, current, branch);
            campaign
                .write(
                    frame,
                    1000 + i64::from(hour),
                    Some(signal),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            branch
        };
        let first = write(&mut campaign, 10, 1);
        assert_eq!(write(&mut campaign, 11, 1), first);
        assert_eq!(write(&mut campaign, 12, 1), first);
        let reloaded = write(&mut campaign, 11, 2);
        assert_ne!(reloaded, first);
        let start = campaign.tracker.branch_start(reloaded).unwrap();
        assert_eq!(start.reason, BranchReason::LoadSignal);
        assert_eq!(start.first_hour, 11);
        assert_eq!(start.loaded_hour, Some(11));
        assert_eq!(
            start.parent,
            Parentage::Probable {
                branch: first,
                hour: 11
            }
        );
        campaign.detach(0).unwrap();
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        let replay = Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &recovered).unwrap();
        assert_eq!(replay.recorded_hours(first).unwrap(), vec![10, 11, 12]);
        assert_eq!(replay.recorded_hours(reloaded).unwrap(), vec![11]);
        assert_eq!(
            replay.sessions[0].sample_boundary,
            replay_data::identity::SampleBoundary::EndOfHour
        );
        let last_detach = replay
            .controls
            .iter()
            .rev()
            .find_map(|control| match control {
                ControlRecord::Detach { last_hour, .. } => Some(*last_hour),
                _ => None,
            });
        assert_eq!(last_detach, Some(Some(11)));
    }

    #[test]
    fn a_failed_final_write_does_not_duplicate_an_hour_to_manufacture_detach() {
        let (mut campaign, store) = campaign();
        publish(&mut campaign, 10, 1).unwrap();
        store.borrow_mut().fail_frame_flush = true;
        assert!(publish(&mut campaign, 11, 1).is_err());
        campaign.detach(0).unwrap();
        let store = store.borrow();
        assert_eq!(store.segments.len(), 2);
        let records: Vec<_> = store
            .segments
            .values()
            .flat_map(|bytes| recover_prefix(bytes).records)
            .collect();
        assert_eq!(
            records
                .iter()
                .filter_map(|record| match record {
                    Record::Frame(frame) => Some(frame.hour),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [10, 11]
        );
        assert!(matches!(
            records.last(),
            Some(Record::Gap(gap)) if gap.first_hour == 11 && gap.last_hour == 11
        ));
        assert!(
            !records
                .iter()
                .any(|record| matches!(record, Record::Control(ControlRecord::Detach { .. })))
        );
    }

    #[test]
    fn unknown_tail_loss_is_written_at_detach_without_inventing_a_range() {
        let (mut campaign, store) = campaign();
        let branch = publish(&mut campaign, 10, 1).unwrap();
        campaign.detach(3).unwrap();
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        let replay = Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &recovered).unwrap();
        assert_eq!(replay.recorded_hours(branch).unwrap(), vec![10]);
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|record| matches!(record,
            ControlRecord::GapSummary { dropped_frames: 3, reason, .. } if reason.contains("hour range are unknown"))));
    }

    fn recovered(store: &Rc<RefCell<Store>>) -> Replay {
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        Replay::from_recovered(recorder_derive::writers(), ID.to_vec(), &recovered).unwrap()
    }

    fn event_losses(replay: &Replay) -> Vec<(BranchId, i32, i32)> {
        replay
            .controls
            .iter()
            .filter_map(|control| match *control {
                ControlRecord::EventLoss {
                    session: owner,
                    branch,
                    after_hour,
                    hour,
                } => {
                    assert_eq!(owner, session().id);
                    Some((branch, after_hour, hour))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_same_process_reattach_records_the_unobserved_event_interval_once() {
        let signal = clock(10).play_session_started_at;
        let (mut first, store) = campaign();
        let branch = publish(&mut first, 10, signal).unwrap();
        publish(&mut first, 11, signal).unwrap();
        first.detach(0).unwrap();
        let earlier = recovered(&store);
        assert!(event_losses(&earlier).is_empty());
        let reattach = || {
            let history = recovered(&store).into_history().unwrap();
            campaign_with(history.tracker_for(&session()).unwrap())
        };

        let (mut second, appended) = reattach();
        assert_eq!(publish(&mut second, 12, signal).unwrap(), branch);
        assert_eq!(publish(&mut second, 13, signal).unwrap(), branch);
        second.detach(0).unwrap();
        let resumed = recovered(&appended);
        assert_eq!(event_losses(&resumed), [(branch, 11, 12)]);
        assert_eq!(resumed.event_losses(branch).collect::<Vec<_>>(), [(11, 12)]);
        assert!(resumed.gaps.is_empty());
        assert_eq!(resumed.recorded_hours(branch).unwrap(), vec![12, 13]);

        let (mut moved, later) = reattach();
        assert_ne!(publish(&mut moved, 15, signal).unwrap(), branch);
        moved.detach(0).unwrap();
        assert!(event_losses(&recovered(&later)).is_empty());
    }

    #[test]
    fn a_reattach_whose_first_frame_is_lost_keeps_one_event_loss_and_the_gap() {
        let signal = clock(10).play_session_started_at;
        let (mut first, store) = campaign();
        let branch = publish(&mut first, 10, signal).unwrap();
        publish(&mut first, 11, signal).unwrap();
        first.detach(0).unwrap();
        let reattach = || {
            let history = recovered(&store).into_history().unwrap();
            campaign_with(history.tracker_for(&session()).unwrap())
        };

        let (mut failed, appended) = reattach();
        appended.borrow_mut().fail_frame_flush = true;
        assert!(publish(&mut failed, 12, signal).is_err());
        assert_eq!(publish(&mut failed, 13, signal).unwrap(), branch);
        failed.detach(0).unwrap();
        let retried = recovered(&appended);
        assert_eq!(event_losses(&retried), [(branch, 11, 12)]);
        assert!(
            retried
                .gaps
                .iter()
                .any(|gap| gap.first_hour == 12 && gap.last_hour == 12)
        );

        let (mut dropped, later) = reattach();
        dropped
            .lost(
                &LostObservation {
                    campaign_id: owned_text(ID),
                    first_hour: 12,
                    last_hour: 12,
                    loaded_hour: Some(11),
                    load_signal: Some(signal),
                    count: 1,
                },
                false,
            )
            .unwrap();
        assert_eq!(publish(&mut dropped, 13, signal).unwrap(), branch);
        dropped.detach(0).unwrap();
        let queued = recovered(&later);
        assert_eq!(event_losses(&queued), [(branch, 11, 12)]);
        assert_eq!(queued.recorded_hours(branch).unwrap(), vec![13]);
        assert!(
            queued
                .gaps
                .iter()
                .any(|gap| gap.first_hour == 12 && gap.last_hour == 12)
        );
    }

    #[test]
    fn campaign_preflight_retries_unopened_campaigns_before_consuming_any_witness() {
        let second = b"31b7cc84-3028-4f65-8e0d-adf2f37ca1a6".as_slice();
        let third = b"41b7cc84-3028-4f65-8e0d-adf2f37ca1a6".as_slice();
        let ids = [ID, b"../invalid", second, ID, third];
        let mut prepared = BTreeSet::new();
        let mut attempts = Vec::new();
        let mut fail_once = true;
        let mut open = |id: &str| -> io::Result<()> {
            if prepared.contains(id) {
                return Ok(());
            }
            attempts.push(id.to_owned());
            if id.as_bytes() == second && fail_once {
                fail_once = false;
                return Err(io::Error::other("temporary campaign directory failure"));
            }
            prepared.insert(id.to_owned());
            Ok(())
        };
        assert!(prepare_campaigns(ids, &mut open).is_err());
        prepare_campaigns(ids, &mut open).unwrap();
        assert_eq!(
            attempts.iter().map(String::as_bytes).collect::<Vec<_>>(),
            vec![ID, second, second, third]
        );
        assert_eq!(prepared.len(), 3);
        prepare_campaigns([b"malformed".as_slice()], |_| {
            panic!("invalid path was opened")
        })
        .unwrap();
    }

    #[test]
    fn malformed_campaign_witnesses_become_unknown_losses_without_filesystem_access() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            session_verified: false,
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helpers: Vec::new(),
            backlogged: false,
            comparison: Box::default(),
            writer: crate::spool_writer::Writer::start(PathBuf::new()).unwrap(),
            journal_thread: crate::columnar::JournalThread::start().unwrap(),
            queued: std::collections::VecDeque::new(),
            flight: None,
            writer_error: None,
            command_drain: Default::default(),
            command_pending: Vec::new(),
            command_contexts: Default::default(),
            command_anchors: Default::default(),
            command_ordinals: Default::default(),
            command_definitions: Default::default(),
            command_last_flush: std::time::Instant::now(),
            command_ready: Default::default(),
            command_inflight: Default::default(),
        };
        let loss = LostObservation {
            campaign_id: owned_text(b"../invalid"),
            count: 3,
            first_hour: 1,
            last_hour: 3,
            loaded_hour: Some(0),
            load_signal: Some(1),
        };
        assert!(
            // SAFETY: This isolated spool fixture submits its own captured loss.
            unsafe { processor.lost(&loss) }.is_err()
        );
        let mut captured = captured(4);
        captured.clock.as_mut().unwrap().campaign_id = owned_text(b"../invalid");
        // SAFETY: The caller owns the session and runs preparation before admitting consumers.
        unsafe { processor.prepare(&captured) }.unwrap();
        assert!(
            // SAFETY: This frame or slot belongs to the current admitted capture or the sole spool
            // consuming it.
            unsafe { processor.frame(&captured) }.is_err()
        );
        assert_eq!(processor.unknown_pending, 4);
        assert!(processor.suppress_origin);
        assert!(processor.campaigns.is_empty());
    }

    fn research_rows(rows: &[(u32, u32, i64, i32)]) -> TableRows {
        TableRows::ResearchInputs(
            rows.iter()
                .map(
                    |&(country, index, progress_raw, _read)| replay_data::rows::ResearchInputRow {
                        country,
                        index,
                        progress_raw,
                        ..Default::default()
                    },
                )
                .collect(),
        )
    }

    fn sliced_input(branch: BranchId, hour: i32, view: TableView) -> FrameInput {
        let mut input: FrameInput = staggered_frame(branch, hour, false).into();
        input.tables[TableId::ResearchInputs.index()] = TableInput::Sampled(view);
        input
    }

    fn merged(input: &FrameInput) -> &TableInput {
        &input.tables[TableId::ResearchInputs.index()]
    }

    fn reads(input: &FrameInput) -> Vec<(u32, i32)> {
        let TableInput::Sampled(TableView::Complete(TableRows::SliceReads(rows))) =
            &input.tables[TableId::SliceReads.index()]
        else {
            panic!("slice reads assembled")
        };
        rows.iter()
            .filter(|row| row.table == TableId::ResearchInputs.index() as u32)
            .map(|row| (row.owner, row.sample_hour))
            .collect()
    }

    fn merge_with(
        campaign: &mut Campaign<MemoryBackend>,
        input: &mut FrameInput,
        selection: collector_certificates::slicing::SliceSelection,
    ) -> bool {
        campaign
            .merge_sliced_inputs(input, |_| Some(&selection), |_| true)
            .0
    }

    #[test]
    fn a_walked_slice_that_fails_conversion_reports_its_family() {
        use collector_certificates::slicing::{RESEARCH, SliceSelection};
        let (mut campaign, _) = campaign();
        let selection = SliceSelection::for_test(1, false, &[]);
        let failed = || {
            sliced_input(
                1,
                1,
                TableView::Unavailable {
                    reason: "walker produced duplicate engine ids".into(),
                },
            )
        };
        let (retry, unpublished) =
            campaign.merge_sliced_inputs(&mut failed(), |_| Some(&selection), |_| true);
        assert_eq!(unpublished, 1 << RESEARCH);
        assert!(retry);
        let (_, unpublished) =
            campaign.merge_sliced_inputs(&mut failed(), |_| Some(&selection), |_| false);
        assert_eq!(unpublished, 0);
        let mut skipped = sliced_input(
            1,
            1,
            TableView::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON.into(),
            },
        );
        let (_, unpublished) =
            campaign.merge_sliced_inputs(&mut skipped, |_| Some(&selection), |_| true);
        assert_eq!(unpublished, 0);
    }

    #[test]
    fn a_first_full_hour_publishes_complete_sliced_rows_without_earlier_ones() {
        use collector_certificates::slicing::SliceSelection;
        let (mut first_campaign, _) = campaign();
        let rows = research_rows(&[(1, 0, 5, 0), (2, 0, 6, 0)]);
        let mut first = sliced_input(1, 0, TableView::Complete(rows.clone()));
        assert!(!merge_with(
            &mut first_campaign,
            &mut first,
            SliceSelection::for_test(0, true, &[]).with_census(&[1, 2])
        ));
        assert!(matches!(merged(&first), TableInput::SharedComplete(merged) if **merged == rows));
        assert_eq!(reads(&first), [(1, 0), (2, 0)]);
        let (mut fresh, _) = campaign();
        let mut truncated = sliced_input(
            1,
            0,
            TableView::Truncated {
                rows: research_rows(&[(1, 0, 5, 0)]),
                missing: 1,
            },
        );
        assert!(merge_with(
            &mut fresh,
            &mut truncated,
            SliceSelection::for_test(0, true, &[]).with_census(&[1, 2])
        ));
        assert!(matches!(
            merged(&truncated),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }

    #[test]
    fn no_hour_merges_rows_from_before_an_unpublished_read() {
        use collector_certificates::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let full = |hour| {
            sliced_input(
                1,
                hour,
                TableView::Complete(research_rows(&[(1, 0, 5, hour), (2, 0, 6, hour)])),
            )
        };
        assert!(!merge_with(
            &mut campaign,
            &mut full(0),
            SliceSelection::for_test(0, true, &[])
        ));
        let mut failed = sliced_input(
            1,
            1,
            TableView::Unavailable {
                reason: "walker produced duplicate engine ids".into(),
            },
        );
        let selection = SliceSelection::for_test(1, false, &[]);
        let (_, unpublished) =
            campaign.merge_sliced_inputs(&mut failed, |_| Some(&selection), |_| true);
        assert_ne!(unpublished, 0);
        assert_eq!(reads(&failed), [(1, 0), (2, 0)]);
        let mut slice = sliced_input(1, 2, TableView::Complete(research_rows(&[(2, 0, 7, 2)])));
        assert!(merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(2, false, &[])
        ));
        assert!(matches!(
            merged(&slice),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(reads(&slice).is_empty());
        let mut refreshed = full(3);
        assert!(!merge_with(
            &mut campaign,
            &mut refreshed,
            SliceSelection::for_test(3, true, &[])
        ));
        assert!(matches!(merged(&refreshed), TableInput::SharedComplete(_)));
    }

    fn failed_slice_merge_recovers(table: TableId, truncated: bool) {
        use collector_certificates::slicing::{
            FAMILIES, SliceFingerprints, SliceSelection, family_of,
        };

        let rows = |values: &[(u32, i64)]| match table {
            TableId::ResearchInputs => TableRows::ResearchInputs(
                values
                    .iter()
                    .map(
                        |&(country, progress_raw)| replay_data::rows::ResearchInputRow {
                            country,
                            progress_raw,
                            ..Default::default()
                        },
                    )
                    .collect(),
            ),
            TableId::StateResourceInputs => TableRows::StateResourceInputs(
                values
                    .iter()
                    .map(
                        |&(state, global_factor)| replay_data::rows::StateResourceInputRow {
                            state,
                            global_factor,
                            ..Default::default()
                        },
                    )
                    .collect(),
            ),
            _ => unreachable!(),
        };
        let frame = |hour, view| {
            let mut input: FrameInput = staggered_frame(1, hour, false).into();
            input.tables[table.index()] = TableInput::Sampled(view);
            input
        };
        let scan = |store: &mut SliceFingerprints, hour, full| {
            let mut selection = SliceSelection::for_test(hour, full, &[]);
            for owner in [1, 2, 3] {
                let print = u64::from(table == TableId::ResearchInputs && owner == 1 && hour > 0);
                selection.scan(store, owner, Some(print));
            }
            selection.finish_census(store);
            store.finish_scan();
            selection
        };
        let merge = |campaign: &mut Campaign<MemoryBackend>,
                     input: &mut FrameInput,
                     selection: &SliceSelection| {
            campaign.merge_sliced_inputs(input, |_| Some(selection), |_| true)
        };
        let family = family_of(table).unwrap();
        let (mut campaign, _) = campaign();
        let mut store = SliceFingerprints::new();
        let initial = rows(&[(1, 10), (2, 20), (3, 30)]);
        let mut first = frame(0, TableView::Complete(initial));
        assert_eq!(
            merge(&mut campaign, &mut first, &scan(&mut store, 0, true)),
            (false, 0)
        );

        let selection = scan(&mut store, 1, false);
        assert!(selection.contains(1));
        assert!(!selection.contains(2));
        let bad_rows = rows(&[(1, 11), (2, 20)]);
        let bad_view = if truncated {
            selection.invalidate(&mut store);
            TableView::Truncated {
                rows: bad_rows,
                missing: 1,
            }
        } else {
            TableView::Complete(bad_rows)
        };
        let mut failed = frame(1, bad_view);

        let in_flight: Vec<_> = (2..=3)
            .map(|hour| {
                let selection = scan(&mut store, hour, false);
                if !truncated {
                    assert!(
                        !selection.contains(1),
                        "the failed read's print is already held"
                    );
                }
                let values: Vec<_> = [(1, 11), (2, 22), (3, 33)]
                    .into_iter()
                    .filter(|&(owner, _)| selection.contains(owner))
                    .collect();
                (frame(hour, TableView::Complete(rows(&values))), selection)
            })
            .collect();
        let (retry, unpublished) = merge(&mut campaign, &mut failed, &selection);
        assert!(matches!(
            &failed.tables[table.index()],
            TableInput::Sampled(TableView::Unavailable { reason })
                if reason == "sliced table merge failed"
        ));
        for (mut input, selection) in in_flight {
            let (retry, _) = merge(&mut campaign, &mut input, &selection);
            assert!(
                matches!(
                    &input.tables[table.index()],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ),
                "hour {} reused rows predating the failed merge: {:?}",
                input.hour,
                input.tables[table.index()]
            );
            assert!(retry);
        }
        assert!(retry);
        assert_eq!(unpublished, 1 << family);
        for candidate in TableId::ALL
            .into_iter()
            .filter(|&id| replay_data::cadence::is_sliced(id))
        {
            let affected = FAMILIES[family].tables.contains(&candidate);
            assert_eq!(
                campaign.sliced_cache.contains_key(&candidate),
                !affected,
                "{candidate:?}"
            );
            assert_eq!(
                campaign
                    .sliced_reads
                    .keys()
                    .any(|(id, _)| id == candidate.index() as u32),
                !affected && !replay_data::sliced::is_keyed(candidate),
                "{candidate:?} read hours"
            );
        }

        store.forget();
        let selection = scan(&mut store, 4, false);
        assert_eq!(selection.read_owners().collect::<Vec<_>>(), [1, 2, 3]);
        let expected = rows(&[(1, 11), (2, 22), (3, 33)]);
        let mut full = frame(4, TableView::Complete(expected.clone()));
        assert_eq!(
            merge(&mut campaign, &mut full, &scan(&mut store, 4, true)),
            (false, 0)
        );
        for hour in 5..=6 {
            let mut later = frame(hour, TableView::Complete(rows(&[])));
            assert_eq!(
                merge(&mut campaign, &mut later, &scan(&mut store, hour, false)),
                (false, 0)
            );
            assert!(
                matches!(&later.tables[table.index()], TableInput::SharedComplete(held) if **held == expected)
            );
            for owner in [1, 2, 3] {
                assert_eq!(
                    campaign.sliced_reads.get(&(table.index() as u32, owner)),
                    Some(&4)
                );
            }
        }
    }

    #[test]
    fn failed_research_slice_merge_withholds_in_flight_frames_until_full_refresh() {
        for truncated in [false, true] {
            failed_slice_merge_recovers(TableId::ResearchInputs, truncated);
        }
    }

    #[test]
    fn failed_state_slice_merge_withholds_in_flight_frames_until_full_refresh() {
        for truncated in [false, true] {
            failed_slice_merge_recovers(TableId::StateResourceInputs, truncated);
        }
    }

    #[test]
    fn supply_trains_merge_each_countrys_nodes_and_inventory_at_its_read_hour() {
        use collector_certificates::slicing::SliceSelection;
        use replay_data::{rows::SupplyNodeRow, rows::TrainInventoryRow};
        const VERTEX: u32 = SupplyNodeRow::VERTEX;
        let nodes = |rows: &[(u32, u32, i64)]| {
            TableRows::SupplyNodes(
                rows.iter()
                    .map(|&(country, index, available)| SupplyNodeRow {
                        country,
                        index,
                        available,
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let trains = |rows: &[(u32, u32)]| {
            TableRows::TrainInventory(
                rows.iter()
                    .map(|&(country, node_count)| TrainInventoryRow {
                        country,
                        node_count,
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let input = |hour: i32, read_nodes: TableRows, read_trains: TableRows| {
            let mut input: FrameInput = staggered_frame(1, hour, false).into();
            input.tables[TableId::SupplyNodes.index()] =
                TableInput::Sampled(TableView::Complete(read_nodes));
            input.tables[TableId::TrainInventory.index()] =
                TableInput::Sampled(TableView::Complete(read_trains));
            input
        };
        let held = |input: &FrameInput, table: TableId| {
            let TableInput::SharedComplete(rows) = &input.tables[table.index()] else {
                panic!("{table:?} merged")
            };
            TableRows::clone(rows)
        };
        let read_hours = |input: &FrameInput, table: TableId| {
            let TableInput::Sampled(TableView::Complete(TableRows::SliceReads(rows))) =
                &input.tables[TableId::SliceReads.index()]
            else {
                panic!("slice reads assembled")
            };
            rows.iter()
                .filter(|row| row.table == table.index() as u32)
                .map(|row| (row.owner, row.sample_hour))
                .collect::<Vec<_>>()
        };
        let (mut campaign, _) = campaign();
        let mut full = input(
            0,
            nodes(&[(1, 0, 10), (1, VERTEX | 7, 0), (2, 0, 20), (25, 0, 250)]),
            trains(&[(1, 1), (2, 1), (25, 1)]),
        );
        assert!(!merge_with(
            &mut campaign,
            &mut full,
            SliceSelection::for_test(0, true, &[]).with_census(&[1, 2, 25])
        ));
        let mut slice = input(
            1,
            nodes(&[(1, 0, 11), (1, VERTEX | 8, 0), (25, 0, 251)]),
            trains(&[(1, 1), (25, 1)]),
        );
        assert!(!merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(1, false, &[]).with_census(&[1, 2, 25])
        ));
        assert_eq!(
            held(&slice, TableId::SupplyNodes),
            nodes(&[(1, 0, 11), (1, VERTEX | 8, 0), (2, 0, 20), (25, 0, 251)])
        );
        assert_eq!(
            held(&slice, TableId::TrainInventory),
            trains(&[(1, 1), (2, 1), (25, 1)])
        );
        for table in [TableId::SupplyNodes, TableId::TrainInventory] {
            assert_eq!(
                read_hours(&slice, table),
                [(1, 1), (2, 0), (25, 1)],
                "{table:?}"
            );
        }
        let mut quiet = input(3, nodes(&[]), trains(&[]));
        assert!(!merge_with(
            &mut campaign,
            &mut quiet,
            SliceSelection::for_test(3, false, &[]).with_census(&[1, 2, 25])
        ));
        assert_eq!(
            held(&quiet, TableId::SupplyNodes),
            held(&slice, TableId::SupplyNodes)
        );
        for table in [TableId::SupplyNodes, TableId::TrainInventory] {
            assert_eq!(
                read_hours(&quiet, table),
                [(1, 1), (2, 0), (25, 1)],
                "{table:?}"
            );
        }
        let mut vanished = input(4, nodes(&[]), trains(&[]));
        assert!(!merge_with(
            &mut campaign,
            &mut vanished,
            SliceSelection::for_test(4, false, &[]).with_census(&[1, 2])
        ));
        assert_eq!(
            held(&vanished, TableId::SupplyNodes),
            nodes(&[(1, 0, 11), (1, VERTEX | 8, 0), (2, 0, 20)])
        );
        assert_eq!(
            held(&vanished, TableId::TrainInventory),
            trains(&[(1, 1), (2, 1)])
        );
    }

    #[test]
    fn slices_replace_only_their_owners_and_keep_other_owners_rows_and_hours() {
        use collector_certificates::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let mut orphan = sliced_input(1, 1, TableView::Complete(research_rows(&[(1, 0, 5, 1)])));
        assert!(merge_with(
            &mut campaign,
            &mut orphan,
            SliceSelection::for_test(1, false, &[])
        ));
        assert!(matches!(
            merged(&orphan),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        let full_rows =
            research_rows(&[(1, 0, 10, 0), (1, 1, 11, 0), (3, 0, 30, 0), (25, 0, 250, 0)]);
        let mut full = sliced_input(1, 0, TableView::Complete(full_rows.clone()));
        assert!(!merge_with(
            &mut campaign,
            &mut full,
            SliceSelection::for_test(0, true, &[])
        ));
        let TableInput::SharedComplete(rows) = merged(&full) else {
            panic!("full read complete")
        };
        assert_eq!(**rows, full_rows);
        let mut slice = sliced_input(
            1,
            1,
            TableView::Complete(research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)])),
        );
        assert!(!merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(1, false, &[3])
        ));
        let TableInput::SharedComplete(rows) = merged(&slice) else {
            panic!("slice merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)]));
        assert_eq!(reads(&slice), [(1, 1), (3, 1)]);
        let mut quiet = sliced_input(1, 2, TableView::Complete(research_rows(&[])));
        assert!(!merge_with(
            &mut campaign,
            &mut quiet,
            SliceSelection::for_test(2, false, &[])
        ));
        let TableInput::SharedComplete(rows) = merged(&quiet) else {
            panic!("quiet merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)]));
        assert_eq!(reads(&quiet), [(1, 1), (3, 1)]);
        let mut failed = sliced_input(1, 3, unavailable("research acquisition panicked"));
        let selection = SliceSelection::for_test(3, false, &[]);
        assert!(
            !campaign
                .merge_sliced_inputs(&mut failed, |_| Some(&selection), |_| false)
                .0
        );
        assert!(matches!(
            merged(&failed),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        let mut partial = sliced_input(
            1,
            27,
            TableView::Truncated {
                rows: research_rows(&[(3, 0, 33, 27)]),
                missing: 1,
            },
        );
        assert!(!merge_with(
            &mut campaign,
            &mut partial,
            SliceSelection::for_test(27, false, &[1])
        ));
        let TableInput::Sampled(TableView::Truncated { rows, missing: 1 }) = merged(&partial)
        else {
            panic!("truncated slice merged")
        };
        assert_eq!(*rows, research_rows(&[(1, 0, 12, 1), (3, 0, 33, 27)]));
        assert_eq!(reads(&partial), [(1, 1), (3, 27)]);
        let mut later = sliced_input(1, 28, TableView::Complete(research_rows(&[])));
        assert!(!merge_with(
            &mut campaign,
            &mut later,
            SliceSelection::for_test(28, false, &[])
        ));
        let TableInput::SharedComplete(rows) = merged(&later) else {
            panic!("later merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)]));
        assert_eq!(reads(&later), [(1, 1), (3, 1)]);
        let mut vanished = sliced_input(1, 30, TableView::Complete(research_rows(&[])));
        assert!(!merge_with(
            &mut campaign,
            &mut vanished,
            SliceSelection::for_test(30, false, &[]).with_census(&[1])
        ));
        let TableInput::SharedComplete(rows) = merged(&vanished) else {
            panic!("census merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1)]));
        let mut other = sliced_input(2, 29, TableView::Complete(research_rows(&[])));
        assert!(merge_with(
            &mut campaign,
            &mut other,
            SliceSelection::for_test(29, false, &[])
        ));
    }

    #[test]
    fn a_tracking_walker_stamps_only_the_owners_it_read() {
        use collector_certificates::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let mut full = sliced_input(1, 0, TableView::Complete(research_rows(&[(2, 0, 20, 0)])));
        let mut selection = SliceSelection::for_test(0, true, &[]).with_census(&[2, 3, 26]);
        selection.track_reads();
        selection.mark_read(2);
        selection.mark_read(26);
        assert_eq!(selection.read_owners().collect::<Vec<_>>(), [2, 26]);
        merge_with(&mut campaign, &mut full, selection);
        assert_eq!(reads(&full), [(2, 0), (26, 0)]);
        let untracked = SliceSelection::for_test(0, true, &[]).with_census(&[2, 3, 26]);
        assert_eq!(untracked.read_owners().collect::<Vec<_>>(), [2, 3, 26]);
    }

    #[test]
    fn owners_read_without_rows_keep_their_read_hour() {
        use collector_certificates::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let mut full = sliced_input(1, 0, TableView::Complete(research_rows(&[(2, 0, 20, 0)])));
        merge_with(
            &mut campaign,
            &mut full,
            SliceSelection::for_test(0, true, &[]).with_census(&[2, 26]),
        );
        assert_eq!(reads(&full), [(2, 0), (26, 0)]);
        let mut slice = sliced_input(1, 2, TableView::Complete(research_rows(&[(2, 0, 21, 2)])));
        merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(2, false, &[]).with_census(&[2, 26]),
        );
        assert_eq!(reads(&slice), [(2, 2), (26, 2)]);
        let mut quiet = sliced_input(1, 3, TableView::Complete(research_rows(&[])));
        merge_with(
            &mut campaign,
            &mut quiet,
            SliceSelection::for_test(3, false, &[]).with_census(&[2, 26]),
        );
        assert_eq!(reads(&quiet), [(2, 2), (26, 2)]);
        let mut gone = sliced_input(1, 4, TableView::Complete(research_rows(&[])));
        merge_with(
            &mut campaign,
            &mut gone,
            SliceSelection::for_test(4, false, &[]).with_census(&[2]),
        );
        assert_eq!(reads(&gone), [(2, 2)]);
        let mut unscanned = sliced_input(1, 27, TableView::Complete(research_rows(&[])));
        merge_with(
            &mut campaign,
            &mut unscanned,
            SliceSelection::for_test(27, false, &[26]),
        );
        assert_eq!(reads(&unscanned), [(2, 2)]);
    }

    #[test]
    fn a_complete_definitions_read_drops_removed_occurrences_and_an_empty_one_keeps_all() {
        use collector_certificates::slicing::SliceSelection;
        let definitions = |handles: &[u32]| {
            TableRows::TechnologyDefinitions(
                handles
                    .iter()
                    .map(
                        |&technology_handle| replay_data::rows::TechnologyDefinitionRow {
                            technology_handle,
                            database_index: technology_handle as i32,
                            ..Default::default()
                        },
                    )
                    .collect(),
            )
        };
        let (mut campaign, _) = campaign();
        let mut frame = |hour: i32, rows: TableRows, full: bool| {
            let mut input: FrameInput = staggered_frame(1, hour, false).into();
            input.tables[TableId::TechnologyDefinitions.index()] =
                TableInput::Sampled(TableView::Complete(rows));
            assert!(!merge_with(
                &mut campaign,
                &mut input,
                SliceSelection::for_test(hour, full, &[])
            ));
            let TableInput::SharedComplete(rows) =
                &input.tables[TableId::TechnologyDefinitions.index()]
            else {
                panic!("definitions merged")
            };
            (**rows).clone()
        };
        assert_eq!(frame(0, definitions(&[1, 3]), true), definitions(&[1, 3]));
        assert_eq!(frame(1, definitions(&[1]), false), definitions(&[1]));
        assert_eq!(frame(2, definitions(&[]), false), definitions(&[1]));
    }

    #[test]
    fn periodic_keyframes_re_encode_held_sliced_rows_and_merging_continues_across_them() {
        use collector_certificates::slicing::SliceSelection;
        use replay_frame::frame::FrameDecoder;
        crate::spooling::keep_every_table();
        let (mut campaign, store) = campaign();
        let owners = 1..=48_u32;
        let slice = |hour: i32, full: bool| {
            research_rows(
                &owners
                    .clone()
                    .filter(|&owner| {
                        full || replay_data::cadence::slice_of(owner) == hour.rem_euclid(24) as u32
                    })
                    .map(|owner| (owner, 0, i64::from(hour), hour))
                    .collect::<Vec<_>>(),
            )
        };
        let mut expected = BTreeMap::new();
        let mut held: BTreeMap<u32, (i64, i32)> = BTreeMap::new();
        let mut allocation = None;
        for hour in 0..=745 {
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            let full = hour == 0;
            let fresh = slice(hour, full);
            for owner in replay_data::sliced::owners(&fresh) {
                held.insert(owner, (i64::from(hour), hour));
            }
            let mut input = sliced_input(branch, hour, TableView::Complete(fresh));
            assert!(!merge_with(
                &mut campaign,
                &mut input,
                SliceSelection::for_test(hour, full, &[])
            ));
            assert_eq!(
                matches!(merged(&input), TableInput::SliceUpdate { .. }),
                hour != 0 && hour != 720,
                "hour {hour}"
            );
            let want = research_rows(
                &held
                    .iter()
                    .map(|(&owner, &(progress, read))| (owner, 0, progress, read))
                    .collect::<Vec<_>>(),
            );
            expected.insert(hour, want);
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            let cached = campaign.sliced_cache.get(&TableId::ResearchInputs).unwrap();
            let shared = campaign
                .encoder
                .shared_baseline(TableId::ResearchInputs)
                .unwrap();
            assert!(Arc::ptr_eq(cached, shared), "hour {hour}");
            let current = Arc::as_ptr(cached);
            if hour != 0 && hour != 720 {
                assert_eq!(allocation, Some(current), "hour {hour}");
            }
            allocation = Some(current);
            assert_eq!(campaign.retired.len(), hour as usize + 1);
        }
        campaign.spool.finish_segment().unwrap();
        let frames: Vec<_> = store
            .borrow()
            .segments
            .values()
            .flat_map(|bytes| recover_prefix(bytes).records)
            .filter_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect();
        assert_eq!(frames.len(), 746);
        let mut decoder = FrameDecoder::new();
        let mut keyframes = Vec::new();
        for frame in &frames {
            let decoded = decoder.advance(frame).unwrap();
            let rows = &*decoded.tables[TableId::ResearchInputs.index()];
            assert_eq!(
                rows,
                &TableView::Complete(expected[&decoded.hour].clone()),
                "hour {}",
                decoded.hour
            );
            let TableView::Complete(TableRows::SliceReads(reads)) =
                &*decoded.tables[TableId::SliceReads.index()]
            else {
                panic!("slice reads decoded")
            };
            for row in reads {
                let slot = replay_data::cadence::slice_of(row.owner) as i32;
                let expected_read = (0..=decoded.hour)
                    .rev()
                    .find(|hour| hour % 24 == slot)
                    .unwrap_or(0);
                assert_eq!(row.sample_hour, expected_read, "owner {}", row.owner);
            }
            assert_eq!(reads.len(), 48);
            if frame.keyframe {
                keyframes.push(decoded.hour);
                let alone = FrameDecoder::new().advance(frame).unwrap().tables
                    [TableId::ResearchInputs.index()]
                .clone();
                assert_eq!(*alone, TableView::Complete(expected[&decoded.hour].clone()));
            }
        }
        assert_eq!(keyframes, [0, 720]);
        let counts: Vec<_> = store
            .borrow()
            .segments
            .values()
            .map(|bytes| {
                recover_prefix(bytes)
                    .records
                    .iter()
                    .filter(|record| matches!(record, Record::Frame(_)))
                    .count()
            })
            .collect();
        assert_eq!(counts, [720, 26]);
    }

    fn frames_of(store: &Rc<RefCell<Store>>) -> Vec<replay_frame::frame::Frame> {
        store
            .borrow()
            .segments
            .values()
            .flat_map(|bytes| recover_prefix(bytes).records)
            .filter_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_failed_slice_update_write_caches_the_merged_rows_of_its_hour() {
        use collector_certificates::slicing::SliceSelection;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        crate::spooling::keep_every_table();
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let write = |campaign: &mut Campaign<MemoryBackend>,
                         hour: i32,
                         rows: &[(u32, u32, i64, i32)],
                         full: bool,
                         fail: bool| {
                let branch = campaign.observe(None, hour, Some(1)).unwrap();
                let mut input =
                    sliced_input(branch, hour, TableView::Complete(research_rows(rows)));
                let early: Vec<u32> = rows.iter().map(|row| row.0).collect();
                assert!(!merge_with(
                    campaign,
                    &mut input,
                    SliceSelection::for_test(hour, full, &early)
                ));
                assert_eq!(
                    matches!(merged(&input), TableInput::SliceUpdate { .. }),
                    !full && campaign.spool.is_open(),
                    "hour {hour}"
                );
                if fail && fail_encoding {
                    input.tables[TableId::Clock.index()] =
                        TableView::Complete(TableRows::Clock(Vec::new())).into();
                }
                store.borrow_mut().fail_frame_flush = fail && !fail_encoding;
                let result = campaign.write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                );
                store.borrow_mut().fail_frame_flush = false;
                assert_eq!(result.is_err(), fail, "hour {hour}");
            };
            write(
                &mut campaign,
                0,
                &[(1, 0, 10, 0), (2, 0, 20, 0), (3, 0, 30, 0)],
                true,
                false,
            );
            write(&mut campaign, 1, &[(1, 0, 11, 1)], false, true);
            assert!(campaign.pending_slices.is_empty());
            assert_eq!(
                **campaign.sliced_cache.get(&TableId::ResearchInputs).unwrap(),
                research_rows(&[(1, 0, 11, 1), (2, 0, 20, 0), (3, 0, 30, 0)])
            );
            write(&mut campaign, 2, &[(2, 0, 22, 2)], false, false);
            let last = frames_of(&store).pop().unwrap();
            assert!(last.keyframe);
            assert_eq!(
                last.tables[TableId::ResearchInputs.index()],
                TableUpdate::Snapshot(research_rows(&[
                    (1, 0, 11, 1),
                    (2, 0, 22, 2),
                    (3, 0, 30, 0)
                ]))
            );
            write(&mut campaign, 3, &[(3, 0, 33, 3)], false, false);
            let mut decoder = FrameDecoder::new();
            let mut decoded = None;
            for frame in frames_of(&store).iter().filter(|frame| frame.hour >= 2) {
                decoded = Some(decoder.advance(frame).unwrap().clone());
            }
            assert_eq!(
                *decoded.unwrap().tables[TableId::ResearchInputs.index()],
                TableView::Complete(research_rows(&[
                    (1, 0, 11, 1),
                    (2, 0, 22, 2),
                    (3, 0, 33, 3)
                ]))
            );
        }
    }

    fn staggered_frame(branch: BranchId, hour: i32, skip: bool) -> DecodedFrame {
        use replay_data::{rows::CampaignRow, rows::ClockRow, rows::TableId, rows::TableRows};
        let mut tables = std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            if table.is_derived() {
                unavailable(super::NOT_ACQUIRED)
            } else if table.key_bits() == 0 {
                TableView::Complete(TableRows::one_default(table))
            } else {
                TableView::Complete(TableRows::empty(table))
            }
        });
        tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
            hour,
            ..Default::default()
        }]));
        tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: b"test".to_vec(),
            }]));
        tables[TableId::GameScoreMultipliers.index()] = TableView::Unavailable {
            reason: "score multipliers outside staggered fixture".into(),
        };
        tables[TableId::EventFeedHistory.index()] = TableView::Unavailable {
            reason: "Event feed outside staggered fixture".into(),
        };
        if skip {
            for table in TableId::ALL {
                if replay_data::cadence::is_staggered(table) {
                    tables[table.index()] = TableView::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
        }
        for table in TableId::ALL {
            if let Some(reason) = super::retired_reason(table) {
                tables[table.index()] = TableView::Unavailable {
                    reason: reason.into(),
                };
            }
        }
        replay_frame::frame::retire_absent(DecodedFrame {
            branch,
            hour,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        })
    }

    const DAILY: TableId = TableId::AirAces;

    fn daily_rows(value: u8) -> TableRows {
        assert!(replay_data::cadence::is_staggered(DAILY));
        TableRows::AirAces(vec![replay_data::rows::AirAceRow {
            id: 7,
            name: vec![0, 0xff, value],
            ..Default::default()
        }])
    }

    fn reserved_intel_rows(army: i64) -> TableRows {
        TableRows::CountryIntel(vec![
            replay_data::rows::CountryIntelRow {
                owner_country: 0,
                target_index: 0,
                army,
                ..Default::default()
            },
            replay_data::rows::CountryIntelRow {
                owner_country: 0,
                target_index: 2,
                ..Default::default()
            },
        ])
    }

    fn reserved_intel_sample(
        source: &FrameInput,
    ) -> Option<replay_data::rows::ReservedCountryIntelSample> {
        let TableInput::Sampled(TableView::Complete(TableRows::ReservedCountryNaming(rows))) =
            &source.tables[TableId::ReservedCountryNaming.index()]
        else {
            panic!("complete RCN fixture")
        };
        let replay_data::rows::Recorded::Value(sample) = rows[0].country_intel_sample else {
            panic!("assembled field")
        };
        sample
    }

    #[test]
    fn reserved_intel_certificate_carries_advances_equal_rows_and_clears_fresh_refusal() {
        use replay_data::rows::ReservedCountryIntelSample;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first: FrameInput = staggered_frame(branch, 0, false).into();
        first.tables[TableId::CountryIntel.index()] =
            TableView::Complete(reserved_intel_rows(17)).into();
        let initial = ReservedCountryIntelSample {
            sample_hour: 0,
            vector_count: 3,
        };
        campaign
            .try_apply_staggered_inputs(&mut first, Some(Ok(initial)))
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut first);
        assert_eq!(reserved_intel_sample(&first), Some(initial));

        let mut carried: FrameInput = staggered_frame(branch, 1, true).into();
        campaign
            .try_apply_staggered_inputs(&mut carried, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut carried);
        assert_eq!(reserved_intel_sample(&carried), Some(initial));

        let mut equal: FrameInput = staggered_frame(branch, 2, false).into();
        equal.tables[TableId::CountryIntel.index()] =
            TableView::Complete(reserved_intel_rows(17)).into();
        let latest = ReservedCountryIntelSample {
            sample_hour: 2,
            vector_count: 3,
        };
        campaign
            .try_apply_staggered_inputs(&mut equal, Some(Ok(latest)))
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut equal);
        assert_eq!(reserved_intel_sample(&equal), Some(latest));

        let mut refused: FrameInput = staggered_frame(branch, 3, false).into();
        refused.tables[TableId::CountryIntel.index()] =
            TableView::Complete(TableRows::CountryIntel(vec![
                replay_data::rows::CountryIntelRow {
                    owner_country: 1,
                    target_index: 2,
                    ..Default::default()
                },
            ]))
            .into();
        campaign
            .try_apply_staggered_inputs(&mut refused, Some(Err("reserved body fault")))
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut refused);
        assert_eq!(reserved_intel_sample(&refused), None);
        assert!(matches!(
            refused.tables[TableId::CountryIntel.index()],
            TableInput::SharedComplete(_)
        ));
        let mut after: FrameInput = staggered_frame(branch, 4, true).into();
        campaign
            .try_apply_staggered_inputs(&mut after, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut after);
        assert_eq!(reserved_intel_sample(&after), None);
    }

    #[test]
    fn reserved_intel_empty_certificate_prefix_refusal_and_branch_reset_are_distinct() {
        use replay_data::rows::ReservedCountryIntelSample;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let empty = ReservedCountryIntelSample {
            sample_hour: 0,
            vector_count: 0,
        };
        let mut first: FrameInput = staggered_frame(branch, 0, false).into();
        campaign
            .try_apply_staggered_inputs(&mut first, Some(Ok(empty)))
            .unwrap();
        first.tables[TableId::ReservedCountryNaming.index()] = unavailable("naming failed").into();
        campaign.assemble_reserved_intel_sample(&mut first);
        assert!(
            matches!(&first.tables[TableId::ReservedCountryNaming.index()],
            TableInput::Sampled(TableView::Unavailable { reason }) if reason == "naming failed")
        );
        assert_eq!(campaign.country_intel_sample, Some(empty));
        let mut next: FrameInput = staggered_frame(branch, 1, true).into();
        campaign
            .try_apply_staggered_inputs(&mut next, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut next);
        assert_eq!(reserved_intel_sample(&next), Some(empty));
        let other = branch + 1;
        let mut reset: FrameInput = staggered_frame(other, 2, true).into();
        campaign
            .try_apply_staggered_inputs(&mut reset, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut reset);
        assert_eq!(reserved_intel_sample(&reset), None);
        assert_eq!(campaign.country_intel_sample, None);
    }

    #[test]
    fn reserved_intel_noncomplete_sample_clears_previous_certificate() {
        for failed in [
            TableView::Truncated {
                rows: TableRows::CountryIntel(Vec::new()),
                missing: 1,
            },
            unavailable("positive intel table failed"),
        ] {
            let (mut campaign, _) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let mut first: FrameInput = staggered_frame(branch, 0, false).into();
            let sample = replay_data::rows::ReservedCountryIntelSample {
                sample_hour: 0,
                vector_count: 0,
            };
            campaign
                .try_apply_staggered_inputs(&mut first, Some(Ok(sample)))
                .unwrap();
            let mut refused: FrameInput = staggered_frame(branch, 1, false).into();
            refused.tables[TableId::CountryIntel.index()] = failed.into();
            campaign
                .try_apply_staggered_inputs(&mut refused, Some(Err("positive prerequisite failed")))
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut refused);
            assert_eq!(reserved_intel_sample(&refused), None);
        }
    }

    #[test]
    fn reserved_intel_latest_pair_survives_encoding_or_write_failure_recovery() {
        use replay_data::rows::ReservedCountryIntelSample;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        crate::spooling::keep_every_table();
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let mut first: FrameInput = staggered_frame(branch, 0, false).into();
            first.tables[TableId::CountryIntel.index()] =
                TableView::Complete(reserved_intel_rows(17)).into();
            campaign
                .try_apply_staggered_inputs(
                    &mut first,
                    Some(Ok(ReservedCountryIntelSample {
                        sample_hour: 0,
                        vector_count: 3,
                    })),
                )
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut first);
            campaign
                .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            campaign.observe(None, 1, Some(1)).unwrap();
            let mut failed: FrameInput = staggered_frame(branch, 1, false).into();
            failed.tables[TableId::CountryIntel.index()] =
                TableView::Complete(reserved_intel_rows(-29)).into();
            let latest = ReservedCountryIntelSample {
                sample_hour: 1,
                vector_count: 3,
            };
            campaign
                .try_apply_staggered_inputs(&mut failed, Some(Ok(latest)))
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut failed);
            if fail_encoding {
                failed.tables[TableId::Clock.index()] =
                    TableView::Complete(TableRows::Clock(Vec::new())).into();
            } else {
                store.borrow_mut().fail_frame_flush = true;
            }
            assert!(
                campaign
                    .write(failed, 1001, Some(1), 0, &mut WorkSample::default())
                    .is_err()
            );
            assert_eq!(campaign.country_intel_sample, Some(latest));
            campaign.observe(None, 2, Some(1)).unwrap();
            let mut restored: FrameInput = staggered_frame(branch, 2, true).into();
            campaign
                .try_apply_staggered_inputs(&mut restored, None)
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut restored);
            assert_eq!(reserved_intel_sample(&restored), Some(latest));
            campaign
                .write(restored, 1002, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
            let frame = recovered
                .records
                .iter()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                frame.tables[TableId::CountryIntel.index()],
                TableUpdate::Snapshot(reserved_intel_rows(-29))
            );
            let decoded = FrameDecoder::new().apply(frame).unwrap();
            let rcn = decoded
                .complete::<replay_data::rows::tables::ReservedCountryNaming>()
                .unwrap();
            assert_eq!(
                rcn[0].country_intel_sample,
                replay_data::rows::Recorded::Value(Some(latest))
            );
        }
    }

    #[test]
    fn fixed_layout_refusal_survives_daily_carry_and_independent_keyframes() {
        synthetic_game::Executable::fixed_layout(engine_fixed_layout::fixed_layout::TOKEN_IDS)
            .enter();
        use replay_frame::frame::FrameDecoder;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let key = engine_executable::modifier_keys::POWER_BALANCE_DAILY;
        let mut report = engine_fixed_layout::fixed_layout::Report {
            static_token_maximum: 0x6000,
            gap: Ok(()),
            modifiers: [Ok(()); engine_fixed_layout::fixed_layout::GAP as usize],
            tokens: [Ok(()); engine_fixed_layout::fixed_layout::TOKEN_IDS.len()],
        };
        report.modifiers[key as usize] = Err("definition token spells a different script name");
        let mut decoder = FrameDecoder::new();
        for (hour, skip, force_keyframe) in [(0, false, true), (1, true, false), (2, true, true)] {
            let mut source: FrameInput = staggered_frame(branch, hour, skip).into();
            let retry_before = needs_retry(&source.tables);
            campaign.apply_staggered_inputs(&mut source);
            collector_metadata::fixed_layout::apply(&mut source.tables, &Ok(report)).unwrap();
            assert!(!retry_before);
            let frame = campaign
                .encoder
                .encode_input(source, force_keyframe)
                .unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            for &table in replay_derive::fixed_layout::POWER_BALANCE.tables {
                assert!(
                    matches!(&*decoded.tables[table.index()], TableView::Unavailable { reason } if reason.contains(&format!("synthetic_modifier_{key}")))
                );
                if force_keyframe {
                    let alone = FrameDecoder::new().apply(&frame).unwrap();
                    assert_eq!(alone.tables[table.index()], decoded.tables[table.index()]);
                }
            }
            assert!(decoded.complete::<tables::Countries>().is_ok());
        }
    }

    #[test]
    fn line_output_totals_publish_with_the_census_and_carry_off_its_slot() {
        use collector_output::line_output::{Drained, Entry};
        use replay_data::{rows::ProductionLineRow, rows::TableRows};
        use replay_frame::frame::FrameDecoder;
        const LINE: u64 = 0x10_0000_0021;
        const VARIANT: u64 = 0x5_0000_0046;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut captured =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Captured::new() };
        let mut decoder = FrameDecoder::new();
        let mut published = Vec::new();
        for (hour, first, ticks, skip) in [(0, 10, 1, false), (1, 11, 2, true), (2, 13, 0, false)] {
            let entries = vec![
                Entry {
                    line: LINE,
                    country: 5001,
                    variant: VARIANT,
                    mio: 0,
                    kind: collector_output::line_output::OutputKind::Equipment,
                    units: 2,
                    credit: 2,
                    spending: None,
                    exact: true,
                    purpose: replay_data::rows::OutputPurpose::New,
                };
                ticks
            ];
            // SAFETY: The isolated fixture owns the production interval and its observer.
            unsafe { captured.extended.line_output_mut() }.set_drained_for_test(
                Ok(Drained {
                    epoch: 1,
                    first,
                    cursor: first + ticks as u64,
                    copied: ticks,
                    pending: ticks,
                }),
                &entries,
            );
            let mut source: FrameInput = staggered_frame(branch, hour, skip).into();
            if !skip {
                source.tables[TableId::ProductionLines.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(vec![
                        ProductionLineRow {
                            handle: LINE,
                            country: 5001,
                            variant: VARIANT,
                            ..Default::default()
                        },
                    ])));
            }
            campaign.assemble_line_output(&mut source, &captured);
            campaign.apply_staggered_inputs(&mut source);
            let frame = campaign.encoder.encode_input(source, hour == 0).unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            let rows = decoded.complete::<tables::ProductionLineOutput>().unwrap();
            assert_eq!(rows[0].country, 5001);
            published.push((rows[0].units, rows[0].mio_credit, rows[0].since_hour));
        }
        let retired = replay_data::rows::RetiredI64::default();
        assert_eq!(
            published,
            [(0, retired, 0), (0, retired, 0), (4, retired, 0)]
        );
    }

    #[test]
    fn construction_output_publishes_with_the_queues_and_carries_off_their_slot() {
        use collector_output::construction_ic::{Drained, Entry};
        use replay_data::{
            rows::BuildingConstructionRow, rows::RailwayConstructionRow, rows::TableRows,
        };
        use replay_frame::frame::FrameDecoder;
        const LINE: u64 = 0x10_0000_003b;
        const RAIL: u64 = 0x11_0000_1269;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut captured =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Captured::new() };
        let mut decoder = FrameDecoder::new();
        let mut published = Vec::new();
        for (hour, first, days, skip) in [(0, 10, 1, false), (1, 11, 2, true), (2, 13, 0, false)] {
            let entries: Vec<Entry> = (0..days)
                .map(|day| Entry {
                    line: if day == 0 { LINE } else { RAIL },
                    amount: 150_000,
                    bank: 20_000,
                    railway: day != 0,
                    ..Entry::default()
                })
                .collect();
            // SAFETY: The isolated fixture owns the construction interval and its observer.
            unsafe { captured.extended.construction_ic_mut() }.set_drained_for_test(
                Ok(Drained {
                    epoch: 1,
                    first,
                    cursor: first + days as u64,
                    copied: days,
                    pending: days,
                }),
                &entries,
            );
            let mut source: FrameInput = staggered_frame(branch, hour, skip).into();
            if !skip {
                source.tables[TableId::BuildingConstruction.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::BuildingConstruction(
                        vec![BuildingConstructionRow {
                            id: LINE,
                            country: 5001,
                            ..Default::default()
                        }],
                    )));
                source.tables[TableId::RailwayConstruction.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::RailwayConstruction(vec![
                        RailwayConstructionRow {
                            id: RAIL,
                            country: 5001,
                            ..Default::default()
                        },
                    ])));
            }
            campaign.assemble_construction_ic(&mut source, &captured);
            campaign.apply_staggered_inputs(&mut source);
            let frame = campaign.encoder.encode_input(source, hour == 0).unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            let rows = decoded
                .complete::<tables::ConstructionLineOutput>()
                .unwrap();
            published.push(
                rows.iter()
                    .map(|row| (row.line, row.railway, row.construction_raw, row.bank_raw))
                    .collect::<Vec<_>>(),
            );
        }
        let zero = vec![(LINE, false, 0, 0), (RAIL, true, 0, 0)];
        assert_eq!(
            published,
            [
                zero.clone(),
                zero,
                vec![
                    (LINE, false, 130_000, 20_000),
                    (RAIL, true, 130_000, 20_000)
                ]
            ]
        );
    }

    #[test]
    fn a_failed_write_keeps_the_final_totals_of_ended_lines() {
        use collector_output::line_output::{Drained, Entry};
        use replay_data::{rows::ProductionLineRow, rows::TableRows};
        use replay_frame::frame::FrameDecoder;
        crate::spooling::keep_every_table();
        const LINE_A: u64 = 0x10_0000_0021;
        const LINE_B: u64 = 0x11_0000_0021;
        const VARIANT: u64 = 0x5_0000_0046;
        let (mut campaign, store) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut captured =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { Captured::new() };
        for (hour, first, lines, ticks, fails) in [
            (0, 10, &[LINE_A, LINE_B][..], 0, false),
            (1, 10, &[LINE_A][..], 1, true),
            (2, 11, &[LINE_A][..], 0, false),
        ] {
            if hour != 0 {
                campaign.observe(None, hour, Some(1)).unwrap();
            }
            let entries = vec![
                Entry {
                    line: LINE_B,
                    country: 5001,
                    variant: VARIANT,
                    mio: 0,
                    kind: collector_output::line_output::OutputKind::Equipment,
                    units: 1,
                    credit: 1,
                    spending: None,
                    exact: true,
                    purpose: replay_data::rows::OutputPurpose::New,
                };
                ticks
            ];
            // SAFETY: The isolated fixture owns the production interval and its observer.
            unsafe { captured.extended.line_output_mut() }.set_drained_for_test(
                Ok(Drained {
                    epoch: 1,
                    first,
                    cursor: first + ticks as u64,
                    copied: ticks,
                    pending: ticks,
                }),
                &entries,
            );
            let mut source: FrameInput = staggered_frame(branch, hour, false).into();
            source.tables[TableId::ProductionLines.index()] =
                TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(
                    lines
                        .iter()
                        .map(|&handle| ProductionLineRow {
                            handle,
                            country: 5001,
                            variant: VARIANT,
                            ..Default::default()
                        })
                        .collect(),
                )));
            campaign.assemble_line_output(&mut source, &captured);
            campaign.apply_staggered_inputs(&mut source);
            store.borrow_mut().fail_frame_flush = fails;
            let written = campaign.write(
                source,
                1000 + i64::from(hour),
                Some(1),
                0,
                &mut WorkSample::default(),
            );
            assert_eq!(written.is_err(), fails, "hour {hour}");
        }
        let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
        let frame = recovered
            .records
            .iter()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        let decoded = FrameDecoder::new().apply(frame).unwrap();
        let rows = decoded.complete::<tables::ProductionLineOutput>().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (row.line, row.units, row.ended))
                .collect::<Vec<_>>(),
            [(LINE_A, 0, false), (LINE_B, 1, true)]
        );
    }

    fn conversion_reuse_frame(
        campaign: &mut Campaign<MemoryBackend>,
        branch: BranchId,
        hour: i32,
        value: u8,
        table: TableId,
    ) -> FrameInput {
        if table != TableId::LexicalTokens {
            return character_frame(campaign, branch, hour, value);
        }
        let native = collector_lexical::lexical::lexical_reuse_fixture(value);
        let mut reuse = LexicalConversion::new(
            lexical_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
            &mut campaign.lexical_conversion,
        );
        let [input] = native.table_inputs(Some(&mut reuse));
        let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
        frame.tables[table.index()] = input;
        frame
    }

    #[test]
    fn lexical_reuse_cache_eligibility_primary_failures_and_costs_stay_explicit() {
        let table = TableId::LexicalTokens;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first = conversion_reuse_frame(&mut campaign, branch, 0, 9, table);
        campaign.apply_staggered_inputs(&mut first);
        let cached = Arc::clone(cached_complete(&campaign, table));
        for cached_branch in [None, Some(branch + 1)] {
            assert!(lexical_cache(&campaign.staggered_cache, cached_branch, branch).is_none());
        }
        assert!(Arc::ptr_eq(
            lexical_cache(&campaign.staggered_cache, Some(branch), branch).unwrap(),
            &cached
        ));
        for invalid in [
            CachedTable::Unavailable {
                reason: "fresh lexical failure".into(),
            },
            CachedTable::Truncated {
                rows: (*cached).clone(),
                missing: 1,
            },
        ] {
            campaign.staggered_cache.insert(table, invalid);
            assert!(lexical_cache(&campaign.staggered_cache, Some(branch), branch).is_none());
            let current = conversion_reuse_frame(&mut campaign, branch, 1, 9, table);
            assert!(matches!(
                current.tables[table.index()],
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
        campaign
            .staggered_cache
            .insert(table, CachedTable::Complete(Arc::clone(&cached)));
        let mut current = conversion_reuse_frame(&mut campaign, branch, 1, 9, table);
        assert!(matches!(
            current.tables[table.index()],
            TableInput::SharedComplete(_)
        ));
        current.tables[table.index()] =
            TableInput::Sampled(unavailable("native dictionary unreadable"));
        assert_eq!(
            current.tables[table.index()],
            TableInput::Sampled(unavailable("native dictionary unreadable"))
        );
        campaign.apply_staggered_inputs(&mut current);
        assert!(lexical_cache(&campaign.staggered_cache, Some(branch), branch).is_none());

        let mut source = captured(1);
        source.clock = None;
        let before = campaign.lexical_conversion;
        let mut reuse = LexicalConversion::new(Some(&cached), &mut campaign.lexical_conversion);
        let (invalid, packs) = decoded_with_costs(
            &source,
            clock(1).borrowed(),
            branch,
            &mut SpoolConversion {
                variables_baseline: Default::default(),
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: Some(&mut reuse),
                intel: None,
                projection: None,
                helper: None,
            },
        );
        assert_eq!(
            invalid.tables,
            unavailable_tables("clock unreadable as its capture began")
        );
        assert!(packs.converted_by().all(|pack| pack.is_none()));
        assert_eq!(campaign.lexical_conversion, before);
    }

    fn character_frame(
        campaign: &mut Campaign<MemoryBackend>,
        branch: BranchId,
        hour: i32,
        progress: u8,
    ) -> FrameInput {
        let native = collector_characters::extended::characters::character_shadow_fixture(progress);
        let mut reuse = CharacterConversion::new(
            branch,
            &mut campaign.character_shadow,
            character_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
        );
        let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
        for (index, input) in native
            .table_inputs(Some(&mut reuse), None)
            .into_iter()
            .enumerate()
        {
            if matches!(index, 0 | 1 | 2 | 4) {
                frame.tables[collector_characters::extended::characters::TABLES[index].index()] =
                    input;
            }
        }
        frame
    }

    fn intel_frame(
        campaign: &mut Campaign<MemoryBackend>,
        branch: BranchId,
        hour: i32,
        value: i64,
    ) -> FrameInput {
        let native = collector_intel::intel::intel_shadow_fixture(value);
        let inputs = native.table_inputs(
            Default::default(),
            &mut ScriptVariableComparison::default(),
            Some(&mut IntelConversion::new(
                branch,
                hour,
                &mut campaign.intel_shadow,
            )),
        );
        let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
        frame.tables[TableId::ScriptFlags.index()] = inputs[7].clone();
        frame
    }

    fn intel_allocation(frame: &FrameInput, table: TableId) -> &Arc<TableRows> {
        let TableInput::SharedComplete(rows) = &frame.tables[table.index()] else {
            panic!("expected shared Intel rows")
        };
        rows
    }

    #[test]
    fn intel_shadow_reuses_exact_hourly_rows_through_keyframes_failed_writes_and_new_branches() {
        use replay_frame::frame::TableUpdate;
        crate::spooling::keep_every_table();
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let first = intel_frame(&mut campaign, branch, 0, 10);
            let original = Arc::clone(intel_allocation(&first, TableId::ScriptFlags));
            let mut reference = FrameEncoder::new();
            let expected_first = reference
                .encode(sampled_frame(first.clone()), false)
                .unwrap();
            campaign
                .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            campaign.observe(None, 1, Some(1)).unwrap();
            let second = intel_frame(&mut campaign, branch, 1, 10);
            assert!(Arc::ptr_eq(
                &original,
                intel_allocation(&second, TableId::ScriptFlags)
            ));
            let expected_second = reference
                .encode(sampled_frame(second.clone()), false)
                .unwrap();
            campaign
                .write(second, 1001, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let recovered = recover_prefix(store.borrow().segments.get(&0).unwrap());
            let frames: Vec<_> = recovered
                .records
                .iter()
                .filter_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .collect();
            assert_eq!(frames, vec![&expected_first, &expected_second]);

            campaign.observe(None, 2, Some(1)).unwrap();
            let mut failed = intel_frame(&mut campaign, branch, 2, 20);
            let latest = Arc::clone(intel_allocation(&failed, TableId::ScriptFlags));
            assert!(!Arc::ptr_eq(&original, &latest));
            if fail_encoding {
                failed.tables[TableId::Clock.index()] =
                    TableView::Complete(TableRows::Clock(Vec::new())).into();
            } else {
                store.borrow_mut().fail_frame_flush = true;
            }
            assert!(
                campaign
                    .write(failed, 1002, Some(1), 0, &mut WorkSample::default())
                    .is_err()
            );
            assert!(!campaign.spool.is_open());
            campaign.observe(None, 3, Some(1)).unwrap();
            let restored = intel_frame(&mut campaign, branch, 3, 20);
            assert!(Arc::ptr_eq(
                &latest,
                intel_allocation(&restored, TableId::ScriptFlags)
            ));
            campaign
                .write(restored, 1003, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
            let frame = recovered
                .records
                .iter()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert!(frame.keyframe);
            assert_eq!(
                frame.tables[TableId::ScriptFlags.index()],
                TableUpdate::Snapshot((*latest).clone())
            );

            let next_branch = campaign.observe(None, 4, Some(2)).unwrap();
            assert_ne!(branch, next_branch);
            let next = intel_frame(&mut campaign, next_branch, 4, 20);
            assert!(!Arc::ptr_eq(
                &latest,
                intel_allocation(&next, TableId::ScriptFlags)
            ));
            campaign
                .write(next, 1004, Some(2), 0, &mut WorkSample::default())
                .unwrap();
        }
    }

    #[test]
    fn intel_shadow_missing_clock_skips_conversion_and_preserves_failure() {
        let table = TableId::ScriptFlags;
        let (mut campaign, _) = campaign();
        let mut source = captured(0);
        source.extended.set_intel_shadow_fixture(10);
        let first = intel_frame(&mut campaign, 1, 0, 10);
        let old = Arc::clone(intel_allocation(&first, table));
        let before = campaign.intel_shadow.statistics();
        source.clock = None;
        let mut reuse = IntelConversion::new(1, 0, &mut campaign.intel_shadow);
        let (current, costs) = decoded_with_costs(
            &source,
            clock(0).borrowed(),
            1,
            &mut SpoolConversion {
                variables_baseline: Default::default(),
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: None,
                intel: Some(&mut reuse),
                projection: None,
                helper: None,
            },
        );
        assert_eq!(
            current.tables,
            unavailable_tables("clock unreadable as its capture began")
        );
        let after = campaign.intel_shadow.statistics();
        assert_eq!(
            (after.replay_capacity_bytes, after.hits, after.misses),
            (before.replay_capacity_bytes, before.hits, before.misses)
        );
        assert!(costs.converted_by().all(|pack| pack.is_none()));
        let next = intel_frame(&mut campaign, 1, 1, 10);
        assert!(Arc::ptr_eq(&old, intel_allocation(&next, table)));
    }

    #[test]
    fn character_shadow_reuse_never_overrides_primary_failure_or_missing_clock() {
        for table in collector_characters::extended::characters::SHADOW_TABLES {
            for failure in 0..2 {
                let (mut campaign, _) = campaign();
                let branch = campaign.observe(None, 0, Some(1)).unwrap();
                let mut first = character_frame(&mut campaign, branch, 0, 10);
                campaign.apply_staggered_inputs(&mut first);
                campaign
                    .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                    .unwrap();
                let previous = Arc::downgrade(cached_complete(&campaign, table));
                campaign.observe(None, 1, Some(1)).unwrap();
                let mut current = character_frame(&mut campaign, branch, 1, 10);
                let TableInput::SharedComplete(rows) = &current.tables[table.index()] else {
                    panic!()
                };
                assert!(Arc::ptr_eq(rows, &previous.upgrade().unwrap()));
                match failure {
                    0 => {
                        current.tables[table.index()] =
                            TableInput::Sampled(unavailable("native character unreadable"));
                    }
                    1 => {
                        let mut source = captured(1);
                        source.clock = None;
                        let mut reuse = CharacterConversion::new(
                            branch,
                            &mut campaign.character_shadow,
                            character_cache(
                                &campaign.staggered_cache,
                                campaign.staggered_branch,
                                branch,
                            ),
                        );
                        current = decoded_with_costs(
                            &source,
                            clock(1).borrowed(),
                            branch,
                            &mut SpoolConversion {
                                variables_baseline: Default::default(),
                                variables_scratch: &mut ScriptVariableComparison::default(),
                                characters: Some(&mut reuse),
                                lexical: None,
                                intel: None,
                                projection: None,
                                helper: None,
                            },
                        )
                        .0;
                        assert!(current.tables.iter().all(|input| matches!(
                            input,
                            TableInput::Sampled(TableView::Unavailable { .. })
                        )));
                    }
                    _ => unreachable!(),
                }
                assert!(matches!(
                    current.tables[table.index()],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ));
                campaign.apply_staggered_inputs(&mut current);
                campaign
                    .write(current, 1001, Some(1), 0, &mut WorkSample::default())
                    .unwrap();
                campaign.retired.clear();
                assert!(previous.upgrade().is_none());
                let next = character_frame(&mut campaign, branch, 2, 10);
                assert!(matches!(
                    next.tables[table.index()],
                    TableInput::SharedComplete(_)
                ));
            }
        }
    }

    fn cached_complete(campaign: &Campaign<MemoryBackend>, table: TableId) -> &Arc<TableRows> {
        let CachedTable::Complete(rows) = campaign.staggered_cache.get(&table).unwrap() else {
            panic!("fixture expected complete cache")
        };
        rows
    }

    #[test]
    fn scientist_daily_projection_survives_off_slot_keyframe_slot_and_following_delta() {
        use replay_frame::frame::{FrameDecoder, TableUpdate};
        crate::spooling::keep_every_table();
        let (mut campaign, store) = campaign();
        let mut decoder = FrameDecoder::new();
        for (hour, daily, ordinal) in [(3, false, 0), (4, true, 0), (5, false, 1)] {
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            let mut input: FrameInput = staggered_frame(branch, hour, false).into();
            for (id, table) in collector_projects::projects::TABLES.into_iter().zip(
                collector_projects::projects::scientist_spool_fixture(daily, ordinal),
            ) {
                if matches!(
                    id,
                    TableId::Scientists | TableId::ScientistSkills | TableId::ScientistDaily
                ) {
                    input.tables[id.index()] = table.into();
                }
            }
            let skipped = campaign.apply_staggered_inputs(&mut input);
            assert_eq!(skipped[TableId::ScientistDaily.index()], !daily);
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            assert_eq!(campaign.last_written, Some(hour));
            campaign.retired.clear();
            let recovered = recover_prefix(store.borrow().segments.values().last().unwrap());
            let encoded = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            if hour == 5 {
                assert!(matches!(
                    &encoded.tables[TableId::Scientists.index()],
                    TableUpdate::Delta { .. }
                ));
            }
            let decoded = decoder.apply(encoded).unwrap();
            assert_eq!(
                decoded.complete::<tables::Scientists>().unwrap()[0].ordinal,
                ordinal
            );
            if hour >= 4 {
                let daily = &decoded.complete::<tables::ScientistDaily>().unwrap()[0];
                assert_eq!(
                    daily.injury_days,
                    replay_data::rows::Recorded::Value(Some(8))
                );
                assert_eq!(
                    daily.template_visibility,
                    replay_data::rows::Recorded::Value(Some(1))
                );
                assert!(
                    matches!(&daily.template_origin, replay_data::rows::Recorded::Value(Some(origin)) if origin.line == 17)
                );
            } else {
                assert!(decoded.complete::<tables::ScientistDaily>().is_err());
            }
        }
    }

    #[test]
    fn staggered_complete_cache_and_encoder_share_refreshes_off_hours_and_keyframes() {
        crate::spooling::keep_every_table();
        let (mut campaign, store) = campaign();
        let mut reference = FrameEncoder::new();
        let table = DAILY;
        let mut value = 1;
        let mut previous_allocation = None;
        let mut previous_hour = None;
        for (hour, refresh) in [
            (0, Some(1)),
            (1, None),
            (3, Some(1)),
            (7, Some(2)),
            (8, None),
            (720, None),
        ] {
            if let Some(previous) = previous_hour
                && hour > previous + 1
            {
                campaign
                    .gap(
                        campaign.tracker.current_branch().unwrap(),
                        previous + 1,
                        hour - 1,
                        "fixture queue gap".into(),
                    )
                    .unwrap();
            }
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            previous_hour = Some(hour);
            let mut expected = staggered_frame(branch, hour, false);
            if let Some(next_value) = refresh {
                value = next_value;
            }
            *expected.tables[table.index()] = TableView::Complete(daily_rows(value));
            let mut input: FrameInput = if refresh.is_some() {
                expected.clone().into()
            } else {
                staggered_frame(branch, hour, true).into()
            };
            let sampled_literal = match &input.tables[table.index()] {
                TableInput::Sampled(TableView::Complete(TableRows::AirAces(rows))) => {
                    Some(rows[0].name.as_ptr())
                }
                _ => None,
            };
            let skipped = campaign.apply_staggered_inputs(&mut input);
            assert_eq!(skipped[table.index()], refresh.is_none());
            expected.metrics = input.metrics.clone();
            if let TableInput::SharedComplete(rows) = &input.tables[table.index()] {
                assert!(Arc::ptr_eq(rows, cached_complete(&campaign, table)));
                if let Some(pointer) = sampled_literal {
                    assert_eq!(
                        rows.rows::<tables::AirAces>().unwrap()[0].name.as_ptr(),
                        pointer
                    );
                }
            } else {
                assert!(
                    matches!(&input.tables[table.index()], TableInput::Sampled(TableView::Unavailable { reason }) if reason == replay_data::cadence::SKIPPED_REASON)
                );
            }
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            campaign.retired.clear();
            let rows = cached_complete(&campaign, table);
            assert_eq!(
                Arc::strong_count(rows),
                2,
                "only cache and encoder own the allocation"
            );
            if let Some(previous) = previous_allocation.take() {
                if refresh.is_some() {
                    assert!(
                        std::sync::Weak::upgrade(&previous).is_none(),
                        "equal refresh must replace the old allocation too"
                    );
                } else {
                    assert!(Arc::ptr_eq(&previous.upgrade().unwrap(), rows));
                }
            }
            previous_allocation = Some(Arc::downgrade(rows));
            let reference_frame = reference.encode(expected, false).unwrap();
            let recovered = recover_prefix(store.borrow().segments.values().last().unwrap());
            let actual = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert_eq!(actual, &reference_frame, "hour {hour}");
        }
    }

    #[test]
    fn a_compared_refresh_carries_its_rows_through_a_periodic_keyframe() {
        crate::spooling::keep_every_table();
        let (mut campaign, _) = campaign();
        let table = DAILY;
        let keyframe = replay_frame::KEYFRAME_INTERVAL as i32;
        for hour in 0..=keyframe + 1 {
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            let mut input: FrameInput = staggered_frame(branch, hour, hour > 1).into();
            if hour == 0 {
                input.tables[table.index()] = TableView::Complete(daily_rows(1)).into();
            } else if hour == 1 {
                let (token, _) = campaign
                    .encoder
                    .comparison_baseline::<tables::AirAces>(branch, hour, false)
                    .unwrap();
                input.tables[table.index()] = TableInput::ComparedEqual(token);
            }
            campaign.apply_staggered_inputs(&mut input);
            if hour == keyframe {
                let TableInput::SharedComplete(rows) = &input.tables[table.index()] else {
                    panic!("the keyframe carries the compared rows");
                };
                assert_eq!(**rows, daily_rows(1));
            }
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            if hour == keyframe + 1 {
                assert!(campaign.encoder.can_carry(table, branch, hour + 1, false));
            }
        }
    }

    #[test]
    fn latest_shared_sample_survives_encoding_or_write_failure_and_recovery_snapshots_it() {
        use replay_frame::frame::TableUpdate;
        crate::spooling::keep_every_table();
        let table = DAILY;
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let mut first: FrameInput = staggered_frame(branch, 0, false).into();
            first.tables[table.index()] = TableView::Complete(daily_rows(1)).into();
            campaign.apply_staggered_inputs(&mut first);
            campaign
                .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let previous = Arc::downgrade(cached_complete(&campaign, table));
            campaign.observe(None, 1, Some(1)).unwrap();
            let mut failed: FrameInput = staggered_frame(branch, 1, false).into();
            failed.tables[table.index()] = TableView::Complete(daily_rows(2)).into();
            if fail_encoding {
                failed.tables[TableId::Clock.index()] =
                    TableView::Complete(TableRows::Clock(Vec::new())).into();
            } else {
                store.borrow_mut().fail_frame_flush = true;
            }
            campaign.apply_staggered_inputs(&mut failed);
            let latest = Arc::downgrade(cached_complete(&campaign, table));
            assert!(
                campaign
                    .write(failed, 1001, Some(1), 0, &mut WorkSample::default())
                    .is_err()
            );
            assert!(previous.upgrade().is_none());
            assert_eq!(
                Arc::strong_count(cached_complete(&campaign, table)),
                1,
                "failed encoder has been discarded; latest sample remains cached"
            );
            assert!(!campaign.spool.is_open());
            assert!(!campaign.encoder.can_carry(table, branch, 2, false));
            campaign.observe(None, 2, Some(1)).unwrap();
            let mut restored: FrameInput = staggered_frame(branch, 2, true).into();
            campaign.apply_staggered_inputs(&mut restored);
            let TableInput::SharedComplete(rows) = &restored.tables[table.index()] else {
                panic!()
            };
            assert!(Arc::ptr_eq(rows, &latest.upgrade().unwrap()));
            campaign
                .write(restored, 1002, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            assert_eq!(Arc::strong_count(cached_complete(&campaign, table)), 2);
            let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
            let frame = recovered
                .records
                .iter()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert!(frame.keyframe);
            assert_eq!(frame.hour, 2);
            assert_eq!(
                frame.tables[table.index()],
                TableUpdate::Snapshot(daily_rows(2))
            );
            assert!(recovered.records.iter().any(|record| matches!(record, Record::Gap(gap) if gap.first_hour == 1 && gap.last_hour == 1)));
        }
    }

    #[test]
    fn open_spool_carries_skip_marker_but_restart_materializes_cached_rows() {
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first = staggered_frame(branch, 0, false);
        campaign.apply_staggered_cache(&mut first);
        let mut reference = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        decoder
            .apply(&reference.encode(first.clone(), true).unwrap())
            .unwrap();
        campaign
            .write(first.clone(), 1000, Some(1), 0, &mut WorkSample::default())
            .unwrap();
        assert!(campaign.spool.is_open());
        let mut second = staggered_frame(branch, 1, true);
        campaign.apply_staggered_cache(&mut second);
        assert!(
            matches!(&*second.tables[DAILY.index()], TableView::Unavailable { reason } if reason == replay_data::cadence::SKIPPED_REASON)
        );
        let encoded = campaign.encoder.encode(second, false).unwrap();
        assert_eq!(
            encoded.tables[DAILY.index()],
            TableUpdate::Unchanged { baseline: 0 }
        );
        assert_eq!(decoder.apply(&encoded).unwrap(), first.carried_to(1));
        campaign.spool.finish_segment().unwrap();
        let mut third = staggered_frame(branch, 2, true);
        campaign.apply_staggered_cache(&mut third);
        assert_eq!(third, first.carried_to(2));
        assert!(campaign.encoder.encode(third, true).unwrap().keyframe);
    }

    #[test]
    fn staggered_off_hours_emit_unchanged_and_hold_last_refresh() {
        use replay_data::rows::TableId;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        let (mut campaign, _) = campaign();
        let branch = 1;
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut first = staggered_frame(branch, 0, false);
        let skipped = campaign.apply_staggered_cache(&mut first);
        assert!(!skipped[DAILY.index()]);
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert!(matches!(
            frame.tables[DAILY.index()],
            TableUpdate::Snapshot(_)
        ));
        assert_eq!(decoder.apply(&frame).unwrap(), first);
        let mut second = staggered_frame(branch, 1, true);
        let skipped = campaign.apply_staggered_cache(&mut second);
        assert!(skipped[DAILY.index()]);
        assert!(!skipped[TableId::Clock.index()]);
        assert_eq!(second, first.carried_to(1));
        let frame = encoder.encode(second.clone(), false).unwrap();
        assert_eq!(
            frame.tables[DAILY.index()],
            TableUpdate::Unchanged { baseline: 0 }
        );
        assert_eq!(decoder.apply(&frame).unwrap(), second);
    }

    trait WithHour {
        fn carried_to(&self, hour: i32) -> Self;
    }

    impl WithHour for DecodedFrame {
        fn carried_to(&self, hour: i32) -> Self {
            let mut next = self.clone();
            next.hour = hour;
            if let TableView::Complete(TableRows::Clock(rows)) =
                &mut *next.tables[replay_data::rows::TableId::Clock.index()]
            {
                rows[0].hour = hour;
            }
            for table in replay_data::rows::TableId::ALL {
                if replay_data::cadence::is_staggered(table) {
                    next.metrics.table_ns[table.index()] = 0;
                }
            }
            next
        }
    }

    #[test]
    fn a_table_read_with_no_time_of_its_own_is_marked_read_and_a_skipped_one_is_not() {
        use replay_journal::native_frame::FrameWriter;
        use replay_data::rows::{CountryFuelRow, ProductionLineOutputRow, TableId};
        let (fuel, output, anchor) = (
            TableId::CountryFuel,
            TableId::ProductionLineOutput,
            TableId::Countries,
        );
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut writer = FrameWriter::new().unwrap();
        for (hour, skip) in [(0, false), (1, true), (2, false)] {
            let mut input: FrameInput = staggered_frame(branch, hour, skip).into();
            input.metrics.table_ns[anchor.index()] = 5;
            if skip {
                input.metrics.table_ns[fuel.index()] = 9;
            } else {
                input.tables[fuel.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(vec![
                        CountryFuelRow {
                            country: 7,
                            current_raw: i64::from(hour) + 1,
                            ..Default::default()
                        },
                    ])));
                input.tables[output.index()] = TableInput::Sampled(TableView::Complete(
                    TableRows::ProductionLineOutput(vec![ProductionLineOutputRow {
                        line: 3,
                        country: 7,
                        units: i64::from(hour),
                        ..Default::default()
                    }]),
                ));
            }
            campaign.apply_staggered_inputs(&mut input);
            for table in [fuel, output] {
                assert_eq!(
                    input.metrics.table_ns[table.index()] != 0,
                    !skip,
                    "{table:?} at hour {hour}"
                );
            }
            assert_eq!(input.metrics.table_ns[anchor.index()], 5);
            let prepared = writer.hour(&input).unwrap();
            let read = |table: TableId| {
                prepared.read()[table.index() / 64] >> (table.index() % 64) & 1 != 0
            };
            assert_eq!((read(fuel), read(output)), (!skip, !skip), "hour {hour}");
            writer.commit(prepared).unwrap();
            let sampled = if skip { 0 } else { hour };
            for table in [fuel, output] {
                assert_eq!(
                    writer.tables()[table.index()].acquisition().last_read,
                    Some(sampled),
                    "{table:?} at hour {hour}"
                );
            }
        }
    }

    #[test]
    fn refresh_hour_decode_equals_hourly_baseline() {
        use replay_data::rows::TableId;
        use replay_frame::frame::FrameDecoder;
        let branch = 7;
        let mut hourly_encoder = FrameEncoder::new();
        let mut hourly_decoder = FrameDecoder::new();
        let mut staggered_encoder = FrameEncoder::new();
        let mut staggered_decoder = FrameDecoder::new();
        let (mut campaign, _) = campaign();
        let mut expected_tables: [Option<TableView>; TABLE_COUNT] = std::array::from_fn(|_| None);
        for hour in 0..48 {
            let mut hourly = staggered_frame(branch, hour, false);
            *hourly.tables[DAILY.index()] = TableView::Complete(daily_rows(hour as u8));
            let hourly_frame = hourly_encoder.encode(hourly.clone(), false).unwrap();
            let hourly_decoded = hourly_decoder.apply(&hourly_frame).unwrap().clone();
            let mut staggered = hourly;
            for table in TableId::ALL {
                let index = table.index();
                let retired = super::retired_reason(table).is_some();
                if retired || replay_data::cadence::should_walk(table, hour, hour == 0) {
                    expected_tables[index] = Some((*hourly_decoded.tables[index]).clone());
                } else {
                    *staggered.tables[index] = TableView::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
            campaign.apply_staggered_cache(&mut staggered);
            let staggered_frame = staggered_encoder.encode(staggered.clone(), false).unwrap();
            let staggered_decoded = staggered_decoder.apply(&staggered_frame).unwrap().clone();
            for table in TableId::ALL {
                let index = table.index();
                assert_eq!(
                    &staggered_decoded.tables[index],
                    expected_tables[index].as_ref().unwrap(),
                    "{table:?} at hour {hour} must equal its last complete sample"
                );
            }
        }
    }

    #[test]
    fn branch_start_clears_staggered_cache() {
        let (mut campaign, _) = campaign();
        let mut first = staggered_frame(1, 0, false);
        campaign.apply_staggered_cache(&mut first);
        let mut second = staggered_frame(2, 1, true);
        let skipped = campaign.apply_staggered_cache(&mut second);
        assert!(skipped[DAILY.index()]);
        assert!(
            matches!(
                &*second.tables[DAILY.index()],
                TableView::Unavailable { reason } if reason.contains("full refresh required")
            ),
            "new branch must not carry stale rows"
        );
    }

    #[test]
    fn truncated_decode_forces_snapshot_next_refresh() {
        use replay_data::rows::TableId;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        let (mut campaign, _) = campaign();
        let branch = 3;
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut base = staggered_frame(branch, 10, false);
        campaign.apply_staggered_cache(&mut base);
        let base_frame = encoder.encode(base.clone(), false).unwrap();
        decoder.apply(&base_frame).unwrap();
        let mut first = staggered_frame(branch, 11, false);
        *first.tables[TableId::WorldTension.index()] = TableView::Truncated {
            rows: TableRows::WorldTension(vec![replay_data::rows::WorldTensionRow::default()]),
            missing: 2,
        };
        campaign.apply_staggered_cache(&mut first);
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert!(
            matches!(
                frame.tables[TableId::WorldTension.index()],
                TableUpdate::Truncated { .. }
            ),
            "truncated refresh must encode as truncated, got {:?}",
            frame.tables[TableId::WorldTension.index()]
        );
        assert_eq!(decoder.apply(&frame).unwrap(), first);
        let mut second = staggered_frame(branch, 12, false);
        campaign.apply_staggered_cache(&mut second);
        let frame = encoder.encode(second.clone(), false).unwrap();
        assert!(
            matches!(
                frame.tables[TableId::WorldTension.index()],
                TableUpdate::Snapshot(_)
            ),
            "post-truncation refresh must be a snapshot, not delta/unchanged"
        );
        assert_eq!(decoder.apply(&frame).unwrap(), second);
    }

    #[test]
    fn non_spooled_daily_tables_skip_staggered_cache_and_encoder_baselines() {
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let dir = TempDir(std::env::temp_dir().join(format!(
            "battleplan-non-spooled-daily-{}",
            std::process::id()
        )));
        let _ = fs::remove_dir_all(&dir.0);
        fs::create_dir_all(&dir.0).unwrap();
        let journal_thread = crate::columnar::JournalThread::start().unwrap();
        let (mut campaign, _) = campaign();
        campaign.columnar = crate::columnar::Columnar::open(
            &dir.0,
            "non-spooled-daily",
            &session(),
            &journal_thread,
        );
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first: FrameInput = staggered_frame(branch, 0, false).into();
        campaign.apply_staggered_inputs(&mut first);
        assert!(campaign.staggered_cache.get(&DAILY).is_none());
        campaign
            .write(first, 1000, Some(1), 0, &mut WorkSample::default())
            .unwrap();
        assert!(campaign.encoder.committed_rows(DAILY).is_none());
        assert!(campaign.encoder.committed_rows(TableId::Clock).is_some());
        for table in [
            TableId::DivisionPanelStats,
            TableId::DivisionHistory,
            TableId::CharacterRoleTraits,
            TableId::StateResistance,
        ] {
            assert_eq!(
                baseline_retention(table, true),
                replay_frame::frame::BaselineRetention::Meta,
                "{table:?}"
            );
        }
        for table in [
            TableId::DivisionStatus,
            TableId::DivisionPanelSources,
            TableId::WeatherProvinces,
            TableId::RailCooldowns,
            TableId::DivisionTemplates,
        ] {
            assert_eq!(
                baseline_retention(table, true),
                replay_frame::frame::BaselineRetention::Full,
                "{table:?}"
            );
        }
        campaign.columnar.finish();
    }

    #[test]
    fn truncated_owner_sliced_read_preserves_rows_and_missing_through_live_writer() {
        use collector_certificates::slicing::SliceSelection;
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let dir = TempDir(std::env::temp_dir().join(format!(
            "battleplan-h1-sliced-{}",
            std::process::id()
        )));
        let _ = fs::remove_dir_all(&dir.0);
        fs::create_dir_all(&dir.0).unwrap();
        let journal_thread = crate::columnar::JournalThread::start().unwrap();
        let (mut campaign, _) = campaign();
        campaign.columnar =
            crate::columnar::Columnar::open(&dir.0, "h1-sliced", &session(), &journal_thread);
        assert!(campaign.owner_sliced(TableId::ResearchInputs));

        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let initial = research_rows(&[(1, 0, 5, 0), (2, 0, 6, 0)]);
        let mut first = sliced_input(branch, 0, TableView::Complete(initial));
        let full_sel = SliceSelection::for_test(0, true, &[]).with_census(&[1, 2]);
        assert!(!merge_with(&mut campaign, &mut first, full_sel));
        assert!(campaign.sliced_cache.get(&TableId::ResearchInputs).is_none());
        assert!(campaign.sliced_owners[TableId::ResearchInputs.index()].is_some());
        campaign
            .write(first, 1000, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 1, Some(1)).unwrap();
        let partial = research_rows(&[(1, 0, 99, 1)]);
        let mut second = sliced_input(
            branch,
            1,
            TableView::Truncated {
                rows: partial,
                missing: 7,
            },
        );
        let partial_sel = SliceSelection::for_test(1, false, &[1]).with_census(&[1, 2]);
        assert!(merge_with(&mut campaign, &mut second, partial_sel));
        let expected = research_rows(&[(1, 0, 99, 1), (2, 0, 6, 0)]);
        assert!(
            matches!(
                merged(&second),
                TableInput::Sampled(TableView::Truncated { rows, missing: 7 }) if *rows == expected
            ),
            "truncated owner-sliced read must merge against native baseline and preserve missing count"
        );
        campaign
            .write(second, 1001, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 2, Some(1)).unwrap();
        let queued_slice = research_rows(&[(2, 0, 22, 2)]);
        let mut queued = sliced_input(branch, 2, TableView::Complete(queued_slice));
        assert!(!merge_with(
            &mut campaign,
            &mut queued,
            SliceSelection::for_test(2, false, &[2]).with_census(&[1, 2])
        ));
        let expected_after_queued = research_rows(&[(1, 0, 5, 0), (2, 0, 22, 2)]);
        assert!(
            matches!(
                merged(&queued),
                TableInput::SharedComplete(rows) if **rows == expected_after_queued
            ),
            "queued non-full owner slice after truncation must merge onto saved complete baseline"
        );
        assert_eq!(
            campaign.sliced_reads.0[TableId::ResearchInputs.index()],
            [(1, 0), (2, 2)]
        );
        assert!(campaign.sliced_cache.get(&TableId::ResearchInputs).is_none());
        campaign
            .write(queued, 1002, Some(1), 0, &mut WorkSample::default())
            .unwrap();
        assert_eq!(
            campaign.sliced_reads.0[TableId::ResearchInputs.index()],
            [(1, 0), (2, 2)]
        );

        let branch = campaign.observe(None, 3, Some(1)).unwrap();
        let refreshed = research_rows(&[(1, 0, 5, 3), (2, 0, 6, 3)]);
        let mut third = sliced_input(branch, 3, TableView::Complete(refreshed));
        assert!(!merge_with(
            &mut campaign,
            &mut third,
            SliceSelection::for_test(3, true, &[]).with_census(&[1, 2])
        ));
        assert!(campaign.sliced_cache.get(&TableId::ResearchInputs).is_none());
        campaign
            .write(third, 1003, Some(1), 0, &mut WorkSample::default())
            .unwrap();
        campaign.columnar.finish();
    }

    #[test]
    fn skipped_non_spooled_daily_table_preserves_failure_reason_and_truncated_rows() {
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let dir = TempDir(std::env::temp_dir().join(format!(
            "battleplan-h2-daily-{}",
            std::process::id()
        )));
        let _ = fs::remove_dir_all(&dir.0);
        fs::create_dir_all(&dir.0).unwrap();
        let journal_thread = crate::columnar::JournalThread::start().unwrap();
        let (mut campaign, _) = campaign();
        campaign.columnar =
            crate::columnar::Columnar::open(&dir.0, "h2-daily", &session(), &journal_thread);

        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut hour0: FrameInput = staggered_frame(branch, 0, false).into();
        hour0.tables[DAILY.index()] = TableInput::Sampled(TableView::Unavailable {
            reason: "native daily table failed".into(),
        });
        campaign.apply_staggered_inputs(&mut hour0);
        campaign
            .write(hour0, 1000, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 1, Some(1)).unwrap();
        let mut hour1: FrameInput = staggered_frame(branch, 1, true).into();
        campaign.apply_staggered_inputs(&mut hour1);
        assert!(
            matches!(
                &hour1.tables[DAILY.index()],
                TableInput::Sampled(TableView::Unavailable { reason })
                if reason == "native daily table failed"
            ),
            "skipped hour after failure must keep the real failure reason"
        );
        campaign
            .write(hour1, 1001, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 2, Some(1)).unwrap();
        let partial = daily_rows(9);
        let mut hour2: FrameInput = staggered_frame(branch, 2, false).into();
        hour2.tables[DAILY.index()] = TableInput::Sampled(TableView::Truncated {
            rows: partial.clone(),
            missing: 4,
        });
        campaign.apply_staggered_inputs(&mut hour2);
        campaign
            .write(hour2, 1002, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 3, Some(1)).unwrap();
        let mut hour3: FrameInput = staggered_frame(branch, 3, true).into();
        campaign.apply_staggered_inputs(&mut hour3);
        assert!(
            matches!(
                &hour3.tables[DAILY.index()],
                TableInput::Sampled(TableView::Truncated { rows, missing: 4 })
                if *rows == partial
            ),
            "skipped hour after truncation must keep partial rows and missing count"
        );
        campaign
            .write(hour3, 1003, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 4, Some(1)).unwrap();
        let mut hour4: FrameInput = staggered_frame(branch, 4, false).into();
        hour4.tables[DAILY.index()] = TableInput::Sampled(TableView::Complete(daily_rows(10)));
        hour4.tables[TableId::VariantDefinitions.index()] =
            TableInput::Sampled(TableView::Complete(TableRows::VariantDefinitions(vec![
                replay_data::rows::VariantDefinitionRow {
                    id: 1,
                    equipment_token: 2,
                    ..Default::default()
                },
            ])));
        campaign.apply_staggered_inputs(&mut hour4);
        assert!(campaign.staggered_cache.get(&DAILY).is_none());
        assert!(campaign.staggered_cache.get(&TableId::VariantDefinitions).is_none());
        campaign
            .write(hour4, 1004, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 5, Some(1)).unwrap();
        let mut hour5: FrameInput = staggered_frame(branch, 5, true).into();
        hour5.tables[TableId::VariantDefinitions.index()] =
            TableInput::Sampled(unavailable(replay_data::cadence::SKIPPED_REASON));
        campaign.apply_staggered_inputs(&mut hour5);
        assert!(matches!(
            &hour5.tables[DAILY.index()],
            TableInput::Sampled(TableView::Unavailable { reason })
            if reason == replay_data::cadence::SKIPPED_REASON
        ));
        assert!(matches!(
            &hour5.tables[TableId::VariantDefinitions.index()],
            TableInput::Sampled(TableView::Unavailable { reason })
            if reason == replay_data::cadence::SKIPPED_REASON
        ));
        campaign
            .write(hour5, 1005, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 6, Some(1)).unwrap();
        let (token, _) = campaign
            .encoder
            .observed_comparison_baseline::<tables::VariantDefinitions>(branch, 6, false)
            .unwrap();
        let mut hour6: FrameInput = staggered_frame(branch, 6, true).into();
        hour6.tables[TableId::VariantDefinitions.index()] = TableInput::ComparedEqual(token);
        campaign.apply_staggered_inputs(&mut hour6);
        assert!(campaign.staggered_cache.get(&TableId::VariantDefinitions).is_none());
        campaign
            .write(hour6, 1006, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 7, Some(1)).unwrap();
        let mut hour7: FrameInput = staggered_frame(branch, 7, true).into();
        hour7.tables[TableId::VariantDefinitions.index()] =
            TableInput::Sampled(unavailable(replay_data::cadence::SKIPPED_REASON));
        campaign.apply_staggered_inputs(&mut hour7);
        assert!(matches!(
            &hour7.tables[TableId::VariantDefinitions.index()],
            TableInput::Sampled(TableView::Unavailable { reason })
            if reason == replay_data::cadence::SKIPPED_REASON
        ));
        campaign
            .write(hour7, 1007, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 8, Some(1)).unwrap();
        let (token, _) = campaign
            .encoder
            .observed_comparison_baseline::<tables::VariantDefinitions>(branch, 8, false)
            .unwrap();
        let mut hour8: FrameInput = staggered_frame(branch, 8, true).into();
        hour8.tables[TableId::VariantDefinitions.index()] = TableInput::ComparedChanges {
            token,
            indices: vec![0],
            rows: TableRows::VariantDefinitions(vec![
                replay_data::rows::VariantDefinitionRow {
                    id: 1,
                    equipment_token: 3,
                    ..Default::default()
                },
            ]),
        };
        campaign.apply_staggered_inputs(&mut hour8);
        assert!(campaign.staggered_cache.get(&TableId::VariantDefinitions).is_none());
        campaign
            .write(hour8, 1008, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 9, Some(1)).unwrap();
        let mut hour9: FrameInput = staggered_frame(branch, 9, true).into();
        hour9.tables[TableId::VariantDefinitions.index()] =
            TableInput::Sampled(unavailable(replay_data::cadence::SKIPPED_REASON));
        campaign.apply_staggered_inputs(&mut hour9);
        assert!(matches!(
            &hour9.tables[TableId::VariantDefinitions.index()],
            TableInput::Sampled(TableView::Unavailable { reason })
            if reason == replay_data::cadence::SKIPPED_REASON
        ));
        campaign
            .write(hour9, 1009, Some(1), 0, &mut WorkSample::default())
            .unwrap();
        campaign.columnar.finish();
    }

    #[test]
    fn sparse_carried_research_updates_preserve_columnar_rows_and_slice_reads() {
        use collector_certificates::slicing::SliceSelection;
        use replay_data::rows::{ResearchAppliedBonusRow, ResearchTechnologyRow, RowCodec};
        struct TempDir(PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let dir = TempDir(std::env::temp_dir().join(format!(
            "battleplan-sparse-research-{}",
            std::process::id()
        )));
        let _ = fs::remove_dir_all(&dir.0);
        fs::create_dir_all(&dir.0).unwrap();
        let journal_thread = crate::columnar::JournalThread::start().unwrap();
        let (mut campaign, _) = campaign();
        campaign.columnar =
            crate::columnar::Columnar::open(&dir.0, "sparse-research", &session(), &journal_thread);

        let tech = |country, database_index, progress_raw| ResearchTechnologyRow {
            country,
            database_index,
            progress_raw,
            ..Default::default()
        };
        let bonus = |country, index, speed_bonus_raw| ResearchAppliedBonusRow {
            country,
            index,
            speed_bonus_raw,
            ..Default::default()
        };
        let frame_for = |branch, hour, technologies: TableInput, bonuses: TableInput| {
            let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
            frame.tables[TableId::ResearchTechnologies.index()] = technologies;
            frame.tables[TableId::ResearchAppliedBonuses.index()] = bonuses;
            frame
        };

        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut hour0 = frame_for(
            branch,
            0,
            TableView::Complete(TableRows::ResearchTechnologies(vec![
                tech(1, 10, 100),
                tech(1, 20, 200),
                tech(2, 10, 300),
                tech(2, 30, 400),
                tech(3, 10, 500),
            ]))
            .into(),
            TableView::Complete(TableRows::ResearchAppliedBonuses(vec![
                bonus(1, 0, 50),
                bonus(2, 0, 75),
                bonus(3, 0, 80),
            ]))
            .into(),
        );
        let full = SliceSelection::for_test(0, true, &[1, 2, 3]).with_census(&[1, 2, 3]);
        let (retry, unpub) = campaign
            .try_merge_sliced_inputs(
                &mut hour0,
                |table| {
                    matches!(
                        table,
                        TableId::ResearchTechnologies | TableId::ResearchAppliedBonuses
                    )
                    .then_some(&full)
                },
                |_| true,
                |_, _| false,
            )
            .unwrap();
        assert!(!retry && unpub == 0);
        campaign
            .write(hour0, 1000, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        let branch = campaign.observe(None, 1, Some(1)).unwrap();
        let changed = tech(1, 20, 250);
        let mut hour1 = frame_for(
            branch,
            1,
            TableView::Complete(TableRows::ResearchTechnologies(vec![
                changed.clone(),
                tech(2, 10, 350),
                tech(2, 40, 450),
            ]))
            .into(),
            TableView::Complete(TableRows::ResearchAppliedBonuses(vec![bonus(2, 0, 90)])).into(),
        );
        let mixed_selection = SliceSelection::for_test(1, false, &[1, 2]).with_census(&[1, 2]);
        let (retry, unpub) = campaign
            .try_merge_sliced_inputs(
                &mut hour1,
                |table| {
                    matches!(
                        table,
                        TableId::ResearchTechnologies | TableId::ResearchAppliedBonuses
                    )
                    .then_some(&mixed_selection)
                },
                |_| true,
                |table, owner| {
                    matches!(
                        table,
                        TableId::ResearchTechnologies | TableId::ResearchAppliedBonuses
                    ) && owner == 1
                },
            )
            .unwrap();
        assert!(!retry && unpub == 0);
        assert!(matches!(
            &hour1.tables[TableId::ResearchTechnologies.index()],
            TableInput::SliceUpdate { update, .. }
                if update.owners == vec![2, 3] && update.keys == vec![changed.key()]
        ));
        assert_eq!(
            campaign.sliced_reads.0[TableId::ResearchTechnologies.index()],
            vec![(1, 1), (2, 1)]
        );
        assert_eq!(
            campaign.sliced_reads.0[TableId::ResearchAppliedBonuses.index()],
            vec![(1, 1), (2, 1)]
        );
        campaign
            .write(hour1, 1001, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        assert_eq!(
            campaign
                .columnar
                .materialize_complete(TableId::ResearchTechnologies, branch)
                .unwrap()
                .unwrap(),
            TableRows::ResearchTechnologies(vec![
                tech(1, 10, 100),
                changed,
                tech(2, 10, 350),
                tech(2, 40, 450),
            ])
        );
        assert_eq!(
            campaign
                .columnar
                .materialize_complete(TableId::ResearchAppliedBonuses, branch)
                .unwrap()
                .unwrap(),
            TableRows::ResearchAppliedBonuses(vec![bonus(1, 0, 50), bonus(2, 0, 90)])
        );

        let branch = campaign.observe(None, 2, Some(1)).unwrap();
        let next_changed = tech(1, 20, 275);
        let mut hour2 = frame_for(
            branch,
            2,
            TableView::Complete(TableRows::ResearchTechnologies(vec![next_changed.clone()])).into(),
            TableView::Complete(TableRows::ResearchAppliedBonuses(Vec::new())).into(),
        );
        let all_carried = SliceSelection::for_test(2, false, &[1, 2]).with_census(&[1, 2]);
        let (retry, unpub) = campaign
            .try_merge_sliced_inputs(
                &mut hour2,
                |table| {
                    matches!(
                        table,
                        TableId::ResearchTechnologies | TableId::ResearchAppliedBonuses
                    )
                    .then_some(&all_carried)
                },
                |_| true,
                |table, owner| {
                    matches!(
                        table,
                        TableId::ResearchTechnologies | TableId::ResearchAppliedBonuses
                    ) && (owner == 1 || owner == 2)
                },
            )
            .unwrap();
        assert!(!retry && unpub == 0);
        assert!(matches!(
            &hour2.tables[TableId::ResearchTechnologies.index()],
            TableInput::SliceUpdate { update, .. }
                if update.owners.is_empty() && update.keys == vec![next_changed.key()]
        ));
        campaign
            .write(hour2, 1002, Some(1), 0, &mut WorkSample::default())
            .unwrap();

        assert_eq!(
            campaign
                .columnar
                .materialize_complete(TableId::ResearchTechnologies, branch)
                .unwrap()
                .unwrap(),
            TableRows::ResearchTechnologies(vec![
                tech(1, 10, 100),
                next_changed,
                tech(2, 10, 350),
                tech(2, 40, 450),
            ])
        );
        campaign.columnar.finish();
    }
}
