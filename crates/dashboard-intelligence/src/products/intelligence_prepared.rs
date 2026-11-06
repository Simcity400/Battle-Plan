//! The intelligence products share one observer: decryption, operative
//! placements, agency comparison, the operatives roster and the espionage log.
//!
//! The family is prepared on the segment driver: each segment's operator reads
//! its tables as the observers read them and hands on what those reads changed
//! ([`facts`]), and each branch's ordered emit tells the observers every hour
//! from its reads as last handed on, publishing their pages where hourly
//! preparation did ([`emit`]).
use dashboard_base::{
    catalogue,
    prepared::frame_changes::FrameChanges,
    source::Source,
};
use dashboard_core::config::LoadContext;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{io, sync::atomic::AtomicBool};

#[cfg(test)]
mod columnar_tests;
pub mod delta;
pub mod emit;
pub mod facts;
mod query;
pub mod served;
pub use query::query;
pub(crate) use query::histories;

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

/// What the observers read of an hour beside its change facts and countries, as each segment's
/// operator hands it on ([`facts`]) and a branch's emit holds it.
#[derive(Default)]
pub struct Reads {
    pub(crate) cipher: super::decryption::CipherHeld,
    /// The census of the last membership read the placements resolve.
    pub(crate) placements: Option<super::operative_placements::PlacementRead>,
    pub(crate) agencies: super::agency_comparison::AgencyRead,
    pub(crate) roster: super::operatives_roster::RosterRead,
    /// The last operations census read of a complete table.
    pub(crate) operations: super::espionage_log::CensusHeld,
    /// The completions, each with its definition's name: none where incomplete.
    pub(crate) completions: Option<Vec<super::espionage_log::Completion>>,
    /// The completions as held before those last handed on.
    pub(crate) before: Option<Vec<super::espionage_log::Completion>>,
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
    /// Whether observing `frame` reads the espionage log's series as last committed
    /// ([`Self::observe`]'s `history`).
    #[cfg(test)]
    pub fn reads_history(&self, frame: &DecodedFrame, changes: &FrameChanges) -> bool {
        self.espionage.needs_history_changes(frame, changes)
    }

    /// Observes `frame`, whose change facts are `changes`, and the roster's where they differ
    /// (it reads fewer fields). `rebuilt` names the tables the hour changed in any field: the
    /// roster marks the inputs its next read joins afresh by them, read fields or not, as
    /// hourly preparation's change facts report a table rebuilt. `history` is the store the
    /// family last committed, given where [`Self::reads_history`].
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        source: &Source,
        content: &Content,
        frame: &DecodedFrame,
        (changes, roster): (&FrameChanges, Option<&FrameChanges>),
        rebuilt: &dyn Fn(TableId) -> bool,
        history: Option<(&store::Store, &store::SourceKey)>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:observe");
        self.roster.note_rebuilt(rebuilt);
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
        self.roster.observe_bound(
            frame,
            roster.unwrap_or(changes),
            &self.countries,
            &content.operative,
        )?;
        if self.espionage.needs_history_changes(frame, changes) {
            let (store, key) = history
                .ok_or_else(|| io::Error::other("The espionage log's history was not given"))?;
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

    /// Whether observing an hour whose change facts are `changes` from `reads` reads the
    /// espionage log's series as last committed ([`Self::observe_reads`]'s `history`).
    pub(crate) fn reads_history_of(&self, changes: &FrameChanges, reads: &Reads) -> bool {
        self.espionage
            .needs_history_read(changes, reads.completions.as_deref())
    }

    /// Observes `frame`'s hour from `reads`, as [`Self::observe`] observes a decoded frame:
    /// `frame` holds only the countries, `flags` what the hour read of the census, `touched`
    /// what the hour changed of the reads.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_reads(
        &mut self,
        source: &Source,
        content: &Content,
        frame: &DecodedFrame,
        (flags, reads, touched): (super::operatives_roster::Flags, &Reads, &facts::Touched),
        (changes, roster): (&FrameChanges, Option<&FrameChanges>),
        rebuilt: &dyn Fn(TableId) -> bool,
        history: Option<(&store::Store, &store::SourceKey)>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:observe");
        let hour = frame.hour;
        self.roster.note_rebuilt(rebuilt);
        self.countries.update(frame, Some(changes));
        self.decryption.observe_read(
            hour,
            &reads.cipher,
            &touched.cipher,
            content.defines.as_ref().copied().map_err(String::as_str),
            &content.warnings,
        )?;
        self.placements.observe_read(
            hour,
            super::operative_placements::Census {
                read: flags.membership_read,
                feature: flags.feature,
                complete: flags.memberships,
            },
            changes,
            &self.countries,
            &content.catalogue,
            reads.placements.as_ref(),
            &touched.placements,
        )?;
        self.agencies.observe_read(
            source,
            hour,
            changes,
            &self.countries,
            super::agency_comparison::Census {
                read: flags.membership_read,
                complete: flags.memberships,
            },
            &reads.agencies,
            &touched.agencies,
        )?;
        self.roster.observe_read(
            hour,
            flags,
            &reads.roster,
            roster.unwrap_or(changes),
            &self.countries,
            &content.operative,
            &touched.members,
        )?;
        let census = super::espionage_log::Census {
            complete: flags.operations,
            read: flags.operations_read,
        };
        let read = super::espionage_log::prepared::Given {
            census: &reads.operations,
            touched: &touched.census,
            completions: reads.completions.as_deref(),
            // Completions not handed on this hour are those of the hour before.
            before: if touched.completions {
                reads.before.as_deref()
            } else {
                reads.completions.as_deref()
            },
        };
        let at = (frame.branch, hour);
        if self.reads_history_of(changes, reads) {
            let (store, key) = history
                .ok_or_else(|| io::Error::other("The espionage log's history was not given"))?;
            let mut reader = dashboard_base::prepared::series::Reader::new(
                store,
                key,
                format!("espionage-log-{}-series", frame.branch),
                cancelled,
            )?;
            self.espionage.observe_read(
                source,
                at,
                census,
                read,
                changes,
                &self.countries,
                Some(&mut reader),
                cancelled,
            )
        } else {
            self.espionage.observe_read(
                source,
                at,
                census,
                read,
                changes,
                &self.countries,
                None,
                cancelled,
            )
        }
    }

    /// Finishes replay block `ordinal` of `branch` once its hours are observed: the espionage
    /// log's day pages it closes are written into `writer`.
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

    /// Writes the espionage log's days rendered so far into `writer`, for a flush partway
    /// through a replay block.
    pub fn publish_days(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.espionage
            .publish_days(source, writer, branch, cancelled)
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

    /// One branch's observer as its last flush saved it, each section read
    /// through `read`; None before that flush (no cipher continuation).
    pub fn read_saved(
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
                (&changes, None),
                &|_| false,
                Some((&fixture.store, &fixture.key)),
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
                (&changes, None),
                &|_| false,
                Some((&fixture.store, &fixture.key)),
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
                (&changes, None),
                &|_| false,
                Some((&fixture.store, &fixture.key)),
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
                    (&changes, None),
                    &|_| false,
                    Some((&fixture.store, &fixture.key)),
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
                        (changes, None),
                        &|_| false,
                        Some((&fixture.store, &fixture.key)),
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
