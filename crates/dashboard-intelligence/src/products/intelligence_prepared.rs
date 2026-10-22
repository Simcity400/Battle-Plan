//! The intelligence products share one observer: decryption, operative
//! placements, agency comparison, the operatives roster and the espionage log.
//!
//! The family is prepared on its own ([`FAMILY`], [`State`]). A pre-pass
//! worker reads a block by the fields the observers read and keeps the frames
//! of the hours at which one of them, a row key or a table's availability
//! changes, or a table watched when read is read, with their change facts,
//! and for each run of hours between them which partially watched tables each
//! hour rebuilt. The fold makes every hour's observation on the carried
//! observer, as hourly preparation does: a kept hour with its frame and
//! facts, an hour between on the last kept frame (whose watched fields are,
//! by construction, still current), stamped with its own hour and told of no
//! change or read. The observers' own clocks (decryption's next daily phase,
//! the roster's next day rounding, espionage's daily pages) therefore fire at
//! the hours hourly preparation fires them, and those hours cost what an
//! unchanged hour costs there.
use dashboard_base::{
    catalogue,
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{self, Extract, Family, Folded},
    },
    source::Source,
};
use dashboard_core::config::LoadContext;
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    format::FormatVersion, rows::AutonomyRow, rows::CountryAppearanceRow, rows::CountryRow,
    rows::IntelligenceAgencyRow, rows::PoliticalPartyRow, rows::PoliticsRow,
    rows::RaidIntelInputRow, rows::TableId, rows::TableSchema, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::TableView};
use std::{
    any::Any,
    io,
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
};

pub(crate) mod first_views;
pub use first_views::query;

/// Fixed content is bound in pipeline configuration, including refusals. Recorded
/// inputs and their dated projections remain in the native observers.
pub struct Content {
    operative: Result<Option<std::sync::Arc<super::operative_content::OperativeContent>>, String>,
    catalogue: Result<Option<std::sync::Arc<catalogue::Catalogue>>, catalogue::ContentError>,
    defines: Result<dashboard_text::cryptology::Defines, String>,
    warnings: Vec<String>,
}
impl Content {
    pub fn load(source: &Source, context: &LoadContext) -> io::Result<Self> {
        let operative = match super::operative_content::content(source, context) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            result => result.map_err(|error| error.to_string()),
        };
        let catalogue = context
            .content
            .as_ref()
            .map(|options| catalogue::catalogue(source, options))
            .transpose()
            .map_err(catalogue::ContentError::from);
        if let Err(error) = &catalogue
            && error.error().kind() == io::ErrorKind::Interrupted
        {
            return Err(error.error());
        }
        if let Some(options) = &context.content {
            for result in [
                super::intelligence_content::agency_content(source, options),
                super::intelligence_content::intelligence_content(source, options),
            ] {
                if let Err(error) = result
                    && error.kind() == io::ErrorKind::Interrupted
                {
                    return Err(error);
                }
            }
        }
        let mut warnings = Vec::new();
        let defines = super::decryption::defines(source, context, &mut warnings);
        Ok(Self {
            operative,
            catalogue,
            defines,
            warnings,
        })
    }
}

#[cfg(test)]
thread_local! {
    static WORK: std::cell::RefCell<dashboard_base::prepared::observer_tests::Work> = Default::default();
    static JOINS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(super) fn work(update: impl FnOnce(&mut dashboard_base::prepared::observer_tests::Work)) {
    WORK.with(|work| update(&mut work.borrow_mut()));
}
#[cfg(test)]
pub(super) fn measured() -> dashboard_base::prepared::observer_tests::Work {
    WORK.with(|work| *work.borrow())
}

#[cfg(test)]
pub(super) fn join_rebuilt() {
    JOINS.with(|joins| joins.set(joins.get() + 1));
}
#[cfg(test)]
pub(super) fn joins() -> usize {
    JOINS.with(std::cell::Cell::get)
}

#[derive(Default)]
pub struct Observer {
    decryption: super::decryption::prepared::Accumulator,
    placements: super::operative_placements::prepared::Prepared,
    agencies: super::agency_comparison::prepared::Prepared,
    roster: super::operatives_roster::prepared::Prepared,
    espionage: super::espionage_log::prepared::Prepared,
    // Observer is never serialized; joins are rebuilt by the first accepted frame.
    countries: super::operative_rows::CountryInputs,
}

impl Observer {
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        source: &Source,
        content: &Content,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        store: &store::Store,
        key: &store::SourceKey,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:observe");
        self.countries.update(frame, Some(changes));
        self.decryption.observe_bound(
            frame,
            changes,
            &self.countries,
            content.defines.as_ref().copied().map_err(String::as_str),
            &content.warnings,
        )?;
        self.placements
            .observe_bound(frame, changes, &self.countries, &content.catalogue)?;
        self.agencies
            .observe_changes(source, frame, changes, &self.countries)?;
        self.roster
            .observe_bound(frame, changes, &self.countries, &content.operative)?;
        if self.espionage.needs_history_changes(frame, changes) {
            prepass::note(HISTORY_READS);
            let mut reader = dashboard_base::prepared::series::Reader::new(
                store,
                key,
                format!("espionage-log-{}-series", frame.branch),
                cancelled,
            )?;
            self.espionage.observe_changes(
                source,
                frame,
                changes,
                &self.countries,
                Some(&mut reader),
                cancelled,
            )
        } else {
            self.espionage
                .observe_changes(source, frame, changes, &self.countries, None, cancelled)
        }
    }

    /// An hour at which nothing the family watches changed and no table it
    /// watches when read was read, on the last frame observed (stamped with
    /// the hour, reads cleared). `rebuilt` names the tables the hour's update
    /// rebuilt: hourly preparation's change facts report each of them as
    /// changed. Every observer reads only the watched fields of a partially
    /// watched table, so a rebuild there changed nothing it reads, and each
    /// treats such a rebuild as no change except where noted: its change
    /// facts here report only [`QUIET_REBUILDS`]' tables, and the roster is
    /// told of the rest for the marks it keeps until its next read.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_quiet(
        &mut self,
        source: &Source,
        content: &Content,
        frame: &DecodedFrame,
        rebuilt: &dyn Fn(TableId) -> bool,
        store: &store::Store,
        key: &store::SourceKey,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.roster.note_rebuilt(rebuilt);
        let changes = {
            let _cpu = ::replay_frame::cpu::scope("frame changes", None);
            let mut changes = FrameChanges {
                rows_changed: [false; TABLE_COUNT],
                table_read: [false; TABLE_COUNT],
                slot_read: [false; TABLE_COUNT],
                updates: None,
            };
            for &table in QUIET_REBUILDS {
                changes.rows_changed[table.index()] = rebuilt(table);
            }
            changes
        };
        self.observe(source, content, frame, &changes, store, key, cancelled)
    }

    pub fn finish_block(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.espionage
            .finish_block(source, writer, branch, ordinal, cancelled)
    }

    /// [`Self::finish_block`] with the block's day pages read and written
    /// through `pages`.
    fn finish_block_into(
        &mut self,
        source: &Source,
        pages: &mut impl super::espionage_log::prepared::DayPages,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.espionage
            .finish_block_into(source, pages, branch, ordinal, cancelled)
    }

    pub fn buffered_bytes(&self) -> usize {
        self.decryption.buffered_bytes()
            + self.placements.bytes()
            + self.agencies.bytes()
            + self.roster.bytes()
            + self.espionage.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.decryption.flush(writer, branch, cancelled)?;
        self.placements.flush(writer, branch, cancelled)?;
        self.agencies.flush(writer, branch, cancelled)?;
        self.roster.flush(writer, branch, cancelled)?;
        self.espionage.flush(writer, branch, cancelled)
    }

    pub fn load(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Self> {
        Self::read_saved(branch, &mut |section: &str| match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, section, 0),
            _ => store.read(key, section, 0),
        })?
        .ok_or_else(|| dashboard_core::invalid("Prepared cipher continuation is missing"))
    }

    /// One branch's observer as its last flush saved it, each section read
    /// through `read`; None before that flush (no cipher continuation).
    fn read_saved(
        branch: u64,
        read: &mut dyn FnMut(&str) -> io::Result<Option<store::Range>>,
    ) -> io::Result<Option<Self>> {
        let Some(file) = read(&format!("decryption-{branch}"))? else {
            return Ok(None);
        };
        let decryption = super::decryption::prepared::Accumulator::read(io::BufReader::new(file))?;
        let mut saved = |name: &str, missing: &str| -> io::Result<io::BufReader<store::Range>> {
            read(&format!("{name}-{branch}"))?
                .map(io::BufReader::new)
                .ok_or_else(|| dashboard_core::invalid(missing))
        };
        let placements = super::operative_placements::prepared::Prepared::read(saved(
            "operative-placements",
            "Prepared placement continuation is missing",
        )?)?;
        let agencies = super::agency_comparison::prepared::Prepared::read(saved(
            "agency-comparison",
            "Prepared agency continuation is missing",
        )?)?;
        let roster = super::operatives_roster::prepared::Prepared::read(saved(
            "operatives-roster",
            "Prepared roster continuation is missing",
        )?)?;
        let espionage = super::espionage_log::prepared::Prepared::read(saved(
            "espionage-log",
            "Prepared espionage continuation is missing",
        )?)?;
        Ok(Some(Self {
            countries: Default::default(),
            decryption,
            placements,
            agencies,
            roster,
            espionage,
        }))
    }
}

/// The tables the intelligence observers read.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Cryptology,
    TableId::RaidIntelInputs,
    TableId::SimulationFeatures,
    TableId::OperativeMemberships,
    TableId::CharacterOperatives,
    TableId::CharacterPortraits,
    TableId::IntelligenceAgencies,
    TableId::AgencyUpgradeLevels,
    TableId::LexicalTokens,
    TableId::Operations,
    TableId::OperationAssignments,
    TableId::OperationCompletions,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];

// An hour's rebuilt tables are kept as one bit per table.
const _: () = assert!(TABLES.len() <= 32);

/// The partially watched tables whose rebuild an hour between kept hours
/// reports as hourly preparation's change facts do. The agency scan prunes
/// rosters held for agencies no longer current at each hour its agencies are
/// rebuilt, and saves what it holds, so it sees those hours as hourly
/// preparation shows them. Whole-watched tables are never rebuilt between
/// kept hours: the pre-pass gives the extract a frame at each such rebuild.
const QUIET_REBUILDS: &[TableId] = &[TableId::IntelligenceAgencies];

pub static FAMILY: Family = Family {
    name: "intelligence",
    extract: "intelligence.extract",
    sequential: "intelligence.sequential",
    fallback: "intelligence.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// Test counts ([`prepass::note`]): hours a fold observed on a kept frame,
/// hours between kept hours it observed, and observations at which the
/// espionage log read its own committed series.
pub const KEPT_HOURS: &str = "intelligence.kept";
pub const QUIET_HOURS: &str = "intelligence.quiet";
pub const HISTORY_READS: &str = "intelligence.history";

/// The fields of the family's tables that its observers read. A partially
/// watched table's other fields may move every hour; no observer reads them
/// (the kept frames hold them only as defaults, [`BlockExtract::kept`]).
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // The country join reads the countries' keys: the id (the row key),
        // canonical slot and tag. Their balances move every hour.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // The country join compares the links whole.
        TableFields::all(TableId::CountryLinks),
        // Decryption reads every stored cipher operand (amount, countdown and
        // both flags); they move at the daily update and at activation.
        TableFields::all(TableId::Cryptology),
        // Decryption reads each target's cryptology strength; the defense,
        // strategic presence and counter-operatives are raid inputs.
        TableFields::named(TableId::RaidIntelInputs, &["crypto_strength"])?,
        // The agency feature bit. The debug byte and the runtime scalars,
        // whose sample hour moves every hour, are not read.
        TableFields::named(TableId::SimulationFeatures, &["dlc_mask"])?,
        // Placements, agencies and the roster compare memberships whole, and
        // each read dates them even when equal.
        TableFields::all(TableId::OperativeMemberships).when_read(),
        // The roster joins operatives to characters and portraits, and takes
        // any change of either for a new census input.
        TableFields::all(TableId::CharacterOperatives),
        TableFields::all(TableId::CharacterPortraits),
        // The agency scalars the comparison shows and compares; upgrade
        // progress and the admission ceiling are not read.
        TableFields::named(
            TableId::IntelligenceAgencies,
            &[
                "defense_raw",
                "active_count",
                "unknown_f0",
                "name",
                "created",
            ],
        )?,
        // Acquired upgrade levels, read whole, and the dictionary naming them,
        // operations and completions.
        TableFields::all(TableId::AgencyUpgradeLevels),
        TableFields::all(TableId::LexicalTokens),
        // The espionage log and the roster read every operation field the
        // daily read stores, and each read dates the census even when equal.
        TableFields::all(TableId::Operations).when_read(),
        TableFields::all(TableId::OperationAssignments),
        // Each hour's drained completions, an event table.
        TableFields::all(TableId::OperationCompletions),
        // The country naming operands the roster's capturer names read.
        TableFields::named(
            TableId::CountryAppearance,
            &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        )?,
        TableFields::named(
            TableId::ReservedCountryNaming,
            &[
                "tag",
                "name_tag",
                "cosmetic_tag",
                "leader_ideology",
                "group",
                "overlord",
                "autonomy",
                "built",
            ],
        )?,
        TableFields::named(TableId::Politics, &["ruling_party"])?,
        TableFields::named(TableId::PoliticalParties, &["token"])?,
        TableFields::named(TableId::Autonomy, &["state"])?,
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract::default())
}

/// One run of a block's facts, in hour order.
#[allow(
    clippy::large_enum_variant,
    reason = "Block facts retain observed frames inline without a per-hour allocation."
)]
enum Hour {
    /// An hour observed on its own frame, with its change facts.
    Kept {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// Consecutive hours from `first` at which nothing watched changed and
    /// no table watched when read was read; each hour's bits name the
    /// [`TABLES`] its update rebuilt.
    Quiet { first: i32, rebuilt: Vec<u32> },
}

struct BlockFacts {
    hours: Vec<Hour>,
}

/// One block's extract: the kept frames, each partially watched table reduced
/// to the fields read and shared between kept hours while those are equal.
#[derive(Default)]
struct BlockExtract {
    hours: Vec<Hour>,
    /// The last reduced view of each reduced table, in [`BlockExtract::kept`]
    /// order.
    views: [Option<SharedView>; 7],
}

/// `kept`'s table `T` as `frame`'s rows reduced by `reduce`, at the view
/// `held` last kept while the reduced rows are equal. A truncated or
/// unavailable table is kept as it is.
fn reduced<T: TableSchema>(
    frame: &DecodedFrame,
    kept: &mut DecodedFrame,
    held: &mut Option<SharedView>,
    reduce: impl Fn(&T::Row) -> T::Row,
) where
    T::Row: PartialEq,
{
    let Ok(rows) = frame.complete::<T>() else {
        return;
    };
    let rows: Vec<T::Row> = rows.iter().map(reduce).collect();
    if let Some(view) = held.as_ref()
        && let TableView::Complete(previous) = &**view
        && T::borrow(previous) == Some(rows.as_slice())
    {
        kept.tables[T::ID.index()] = view.clone();
        return;
    }
    let view: SharedView = TableView::Complete(T::wrap(rows)).into();
    *held = Some(view.clone());
    kept.tables[T::ID.index()] = view;
}

impl BlockExtract {
    /// The frame as the observers read it. Large tables rebuilt for fields
    /// no observer reads keep only the fields read, so equal kept hours
    /// share one allocation; observers tell changes from the change facts,
    /// never from allocations.
    fn kept(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        let [
            countries,
            raid,
            agencies,
            appearance,
            politics,
            parties,
            autonomy,
        ] = &mut self.views;
        reduced::<tables::Countries>(frame, &mut kept, countries, |row| CountryRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
            tag: row.tag.clone(),
            ..Default::default()
        });
        reduced::<tables::RaidIntelInputs>(frame, &mut kept, raid, |row| RaidIntelInputRow {
            country: row.country,
            crypto_strength: row.crypto_strength,
            ..Default::default()
        });
        reduced::<tables::IntelligenceAgencies>(frame, &mut kept, agencies, |row| {
            IntelligenceAgencyRow {
                country: row.country,
                defense_raw: row.defense_raw,
                active_count: row.active_count,
                unknown_f0: row.unknown_f0,
                name: row.name.clone(),
                created: row.created,
                ..Default::default()
            }
        });
        reduced::<tables::CountryAppearance>(frame, &mut kept, appearance, |row| {
            CountryAppearanceRow {
                country: row.country,
                cosmetic_tag: row.cosmetic_tag.clone(),
                name_tag: row.name_tag.clone(),
                leader_ideology: row.leader_ideology.clone(),
                overlord: row.overlord,
                ..Default::default()
            }
        });
        reduced::<tables::Politics>(frame, &mut kept, politics, |row| PoliticsRow {
            country: row.country,
            ruling_party: row.ruling_party,
            ..Default::default()
        });
        reduced::<tables::PoliticalParties>(frame, &mut kept, parties, |row| PoliticalPartyRow {
            country: row.country,
            id: row.id,
            token: row.token.clone(),
            ..Default::default()
        });
        reduced::<tables::Autonomy>(frame, &mut kept, autonomy, |row| AutonomyRow {
            subject: row.subject,
            state: row.state.clone(),
            ..Default::default()
        });
        kept
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts and the hours kept between them.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            match hour {
                Hour::Kept { frame, changes } => {
                    held.frame(frame).changes(changes);
                }
                Hour::Quiet { rebuilt, .. } => {
                    held.add(rebuilt.capacity() * size_of::<u32>());
                }
            }
        }
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let frame = self.kept(frame);
        self.hours.push(Hour::Kept {
            frame,
            changes: FrameChanges {
                rows_changed: changes.rows_changed,
                table_read: changes.table_read,
                slot_read: changes.slot_read,
                updates: None,
            },
        });
        Ok(())
    }

    /// Hours between kept hours: the pre-pass splits spans at gaps, so they
    /// are consecutive. Each keeps the tables its update rebuilt.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let all = hours.hours();
        let Some(&first) = all.get(span.start) else {
            return Ok(());
        };
        let mut rebuilt = Vec::with_capacity(span.len());
        for (offset, position) in span.enumerate() {
            if i64::from(all[position]) != i64::from(first) + offset as i64 {
                return Err(io::Error::other(
                    "Intelligence quiet hours are not consecutive",
                ));
            }
            let mut bits = 0_u32;
            for (bit, &table) in TABLES.iter().enumerate() {
                if hours.rebuilt(position, table) {
                    bits |= 1 << bit;
                }
            }
            rebuilt.push(bits);
        }
        self.hours.push(Hour::Quiet { first, rebuilt });
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Intelligence block has no hours"));
        }
        Ok(Box::new(BlockFacts { hours: self.hours }))
    }
}

/// The family's observation of one hour's outcome: `Ok(true)` while it goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// One branch of the intelligence family prepared on its own: the observer
/// hourly preparation keeps in its branch state, the content bound for it,
/// and the espionage day pages its blocks finished since its last flush.
pub struct State {
    observer: Box<Observer>,
    content: Option<Content>,
    pages: super::espionage_log::prepared::StagedPages,
}

impl State {
    /// The branch's observer as its last flush saved it, each section read
    /// through `read`; new before that flush.
    pub fn load(
        read: &mut dyn FnMut(&str) -> io::Result<Option<store::Range>>,
        branch: u64,
    ) -> io::Result<Self> {
        Ok(Self {
            observer: Box::new(Observer::read_saved(branch, read)?.unwrap_or_default()),
            content: None,
            pages: Default::default(),
        })
    }

    /// The content hourly preparation binds before its first frame.
    fn bind(&mut self, source: &Source, context: &LoadContext) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        if self.content.is_none() {
            self.content = Some(Content::load(source, context)?);
        }
        Ok(())
    }

    /// Fold one block's facts: every hour observed on the carried observer,
    /// then the block finished. Returns how it folded and the last hour
    /// reached.
    #[allow(clippy::too_many_arguments)]
    pub fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let facts = {
            let _cpu = ::replay_frame::cpu::scope("fold facts", None);
            facts
                .downcast::<BlockFacts>()
                .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
        };
        self.bind(source, context)?;
        // The espionage log reads its own committed series during
        // observation: the family commits where hourly preparation does, so
        // it reads the prefix hourly preparation reads.
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        let cancelled = AtomicBool::new(false);
        let Self {
            observer,
            content,
            pages,
        } = self;
        let content = content.as_ref().expect("bound intelligence content");
        let mut last: Option<DecodedFrame> = None;
        let mut reached = None;
        for hour in facts.hours {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            check()?;
            match hour {
                Hour::Kept { frame, changes } => {
                    prepass::note(KEPT_HOURS);
                    reached = Some(frame.hour);
                    let result = {
                        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                        observer
                            .observe(source, content, &frame, &changes, &store, &key, &cancelled)
                    };
                    if !observed(frame.hour, result)? {
                        return Ok((Folded::Fast, reached));
                    }
                    last = Some(frame);
                }
                Hour::Quiet { first, rebuilt } => {
                    let mut frame = {
                        let _cpu = ::replay_frame::cpu::scope("fold frame", None);
                        last.clone().ok_or_else(|| {
                            io::Error::other(
                                "Intelligence hours were quiet before the block's first",
                            )
                        })?
                    };
                    // No table was read: a read of a table watched when read
                    // keeps its hour.
                    frame.metrics.table_ns = [0; TABLE_COUNT];
                    for (offset, bits) in rebuilt.into_iter().enumerate() {
                        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                        check()?;
                        let hour = i32::try_from(offset)
                            .ok()
                            .and_then(|offset| first.checked_add(offset))
                            .ok_or_else(|| io::Error::other("Intelligence hour overflow"))?;
                        frame.hour = hour;
                        reached = Some(hour);
                        prepass::note(QUIET_HOURS);
                        let rebuilt = |table: TableId| {
                            TABLES
                                .iter()
                                .position(|&watched| watched == table)
                                .is_some_and(|bit| bits & (1 << bit) != 0)
                        };
                        let result = {
                            let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                            observer.observe_quiet(
                                source, content, &frame, &rebuilt, &store, &key, &cancelled,
                            )
                        };
                        if !observed(hour, result)? {
                            return Ok((Folded::Fast, reached));
                        }
                    }
                }
            }
        }
        finish_block(
            observer,
            pages,
            replay,
            index,
            source,
            &store,
            &key,
            reached.unwrap_or(index.last_hour),
            observed,
        )?;
        Ok((Folded::Fast, reached))
    }

    /// Observe the block hour by hour with the change facts of the block
    /// alone, as hourly preparation does, then finish it. Returns the last
    /// hour observed.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_block(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.bind(source, context)?;
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        let cancelled = AtomicBool::new(false);
        let Self {
            observer,
            content,
            pages,
        } = self;
        let content = content.as_ref().expect("bound intelligence content");
        let mut tracker = FrameChangeTracker::new();
        let mut reached = None;
        let mut stopped = false;
        let mut failure = None;
        let read = replay.read_block_alone(
            index,
            TABLES,
            |raw, frame| {
                let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                if stopped {
                    return Ok(());
                }
                reached = Some(frame.hour);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    observer.observe(source, content, frame, &facts, &store, &key, &cancelled)
                };
                match observed(frame.hour, result) {
                    Ok(live) => {
                        stopped = !live;
                        Ok(())
                    }
                    Err(error) => {
                        failure = Some(error);
                        Err(io::Error::other("Intelligence observation stopped"))
                    }
                }
            },
            check,
        );
        if let Some(error) = failure {
            return Err(error);
        }
        read?;
        if !stopped {
            finish_block(
                observer,
                pages,
                replay,
                index,
                source,
                &store,
                &key,
                reached.unwrap_or(index.last_hour),
                observed,
            )?;
        }
        Ok(reached)
    }

    /// Bytes held for the next commit, against the batch budget.
    pub fn pending(&self) -> usize {
        self.observer.buffered_bytes() + self.pages.bytes()
    }

    /// The espionage day pages its blocks finished since the last commit,
    /// which hourly preparation writes as each block finishes: at every
    /// commit, before the flush, also once the branch was refused later in
    /// the batch.
    pub fn publish(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        self.pages.write(writer)
    }

    /// The flush hourly preparation makes at a batch's end.
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.observer.flush(writer, branch, cancelled)
    }
}

/// Finish a block as hourly preparation does after observing it, its day
/// pages staged for the next commit; the outcome is the block's last hour's.
#[allow(clippy::too_many_arguments)]
fn finish_block(
    observer: &mut Observer,
    pages: &mut super::espionage_log::prepared::StagedPages,
    replay: &Replay,
    index: &BlockIndex,
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    hour: i32,
    observed: &mut Observed<'_>,
) -> io::Result<()> {
    // Day pages are named by the block's position within its own branch.
    let position = replay
        .index
        .iter()
        .position(|block| std::ptr::eq(block, index))
        .ok_or_else(|| io::Error::other("Intelligence block is not in its recording"))?;
    let ordinal = replay.index[..position]
        .iter()
        .filter(|block| block.branch == index.branch)
        .count();
    let cancelled = AtomicBool::new(false);
    let result = observer.finish_block_into(
        source,
        &mut pages.through(store, key),
        index.branch,
        ordinal,
        &cancelled,
    );
    observed(hour, result)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::{
        prepared::{frame_changes::FrameChangeTracker, observer_tests, series::tests::Fixture},
        test_fixtures,
    };
    use replay::container::Replay;
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::IntelligenceAgencyRow,
        rows::SimulationFeatureRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    use std::sync::Arc;

    fn frame() -> DecodedFrame {
        let mut input = test_fixtures::source(1, 100, None);
        *input.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![
                CountryRow {
                    id: 7,
                    canonical_slot: 1,
                    tag: b"modA".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 9,
                    canonical_slot: 2,
                    tag: b"modB".to_vec(),
                    ..Default::default()
                },
            ]));
        *input.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 9,
                    canonical_slot: 2,
                },
            ]));
        *input.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: 1 << 5,
                ..Default::default()
            }]));
        *input.tables[TableId::OperativeMemberships.index()] =
            TableView::Complete(TableRows::OperativeMemberships(Vec::new()));
        *input.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![
                IntelligenceAgencyRow {
                    country: 7,
                    ..Default::default()
                },
            ]));
        *input.tables[TableId::AgencyUpgradeLevels.index()] =
            TableView::Complete(TableRows::AgencyUpgradeLevels(Vec::new()));
        *input.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        *input.tables[TableId::OperationCompletions.index()] =
            TableView::Complete(TableRows::OperationCompletions(Vec::new()));
        input.metrics.table_ns = [0; replay_data::rows::TABLE_COUNT];
        input.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
        input
    }
    fn source(input: &DecodedFrame) -> Source {
        Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "intel-shared-driver".into(),
                revision: "synthetic".into(),
            },
            "fixtures/intel.battleplan".into(),
            Arc::new(
                Replay::from_records(
                    test_fixtures::CAMPAIGN.into(),
                    &[test_fixtures::records(1, vec![input.clone()])],
                )
                .unwrap(),
            ),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn intel_shared_driver_one_changed_table_reuses_the_country_join_and_balances_do_not_invalidate()
     {
        let mut input = frame();
        let source = source(&input);
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let content = Content::load(&source, &LoadContext { content: None }).unwrap();
        let mut observer = Observer::default();
        let mut tracker = FrameChangeTracker::new();
        let changes = tracker.observe(&input, source.replay_ref().format);
        observer
            .observe(
                &source,
                &content,
                &input,
                &changes,
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap();
        let joins_before = joins();
        input.hour += 1;
        input.metrics.table_ns[TableId::OperativeMemberships.index()] = 0;
        let TableView::Complete(TableRows::IntelligenceAgencies(rows)) =
            &mut *input.tables[TableId::IntelligenceAgencies.index()]
        else {
            unreachable!()
        };
        rows[0].defense_raw += 1;
        let changes = tracker.observe(&input, source.replay_ref().format);
        work(|work| *work = Default::default());
        observer
            .observe(
                &source,
                &content,
                &input,
                &changes,
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap();
        assert_eq!(joins(), joins_before);
        assert_eq!(measured().row_visits, 1);
        assert_eq!(measured().emitted, 1);
        input.hour += 1;
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *input.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        rows[0].command_power_raw += 1;
        let changes = tracker.observe(&input, source.replay_ref().format);
        work(|work| *work = Default::default());
        observer
            .observe(
                &source,
                &content,
                &input,
                &changes,
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap();
        assert_eq!(joins(), joins_before);
        assert_eq!(measured(), observer_tests::Work::default());
    }

    #[test]
    fn intel_shared_driver_database_success_and_failure_bind_before_observation() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for metadata tables");
        for success in [true, false] {
            let files = if success {
                vec![(
                    "common/defines/00_fixture.lua",
                    "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture\" } } }",
                )]
            } else {
                Vec::new()
            };
            let (catalogue, folder) = catalogue::tests::folder_catalogue(&files);
            let input = frame();
            let source = source(&input);
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let options = dashboard_core::config::ContentOptions {
                options: installed_content_core::ResolveOptions {
                    game_dir: folder.0.clone(),
                    user_dir: folder.0.join("user"),
                    cache_dir: folder.0.join("cache"),
                },
                allow_unverified: true,
                presentation: Default::default(),
            };
            catalogue::set_test_catalogue(&source, &options, Arc::new(catalogue));
            let context = LoadContext {
                content: Some(options),
            };
            let content = Content::load(&source, &context).unwrap();
            assert_eq!(content.operative.is_ok(), success);
            let mut observer = Observer::default();
            let changes = FrameChangeTracker::new().observe(&input, source.replay_ref().format);
            observer
                .observe(
                    &source,
                    &content,
                    &input,
                    &changes,
                    &fixture.store,
                    &fixture.key,
                    &flag,
                )
                .unwrap();
            let baseline = measured();
            let delta = observer_tests::unchanged_frames(
                baseline,
                &input,
                8,
                source.replay_ref().format,
                |input, changes| {
                    observer.observe(
                        &source,
                        &content,
                        input,
                        changes,
                        &fixture.store,
                        &fixture.key,
                        &flag,
                    )?;
                    Ok(measured())
                },
            )
            .unwrap();
            assert_eq!(delta.content_loads, 0);
            // The existing public loader shares the same failed binding too.
            let second = super::super::operative_content::content(&source, &context);
            assert_eq!(second.is_ok(), success);
            assert_eq!(measured().content_loads, baseline.content_loads);
        }
    }
}
