//! The intelligence products share the pipeline's accepted-frame walk.
use dashboard_base::{catalogue, prepared::frame_changes::FrameChanges, source::Source};
use dashboard_core::config::LoadContext;
use replay::frame::DecodedFrame;
use std::{io, sync::atomic::AtomicBool};

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
        let section = format!("decryption-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| dashboard_core::invalid("Prepared cipher continuation is missing"))?;
        Ok(Self {
            countries: Default::default(),
            decryption: super::decryption::prepared::Accumulator::read(io::BufReader::new(file))?,
            placements: {
                let section = format!("operative-placements-{branch}");
                let file = match saved {
                    Some(previous) if previous != key => {
                        store.read_prefix(previous, key, &section, 0)?
                    }
                    _ => store.read(key, &section, 0)?,
                }
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared placement continuation is missing")
                })?;
                super::operative_placements::prepared::Prepared::read(io::BufReader::new(file))?
            },
            agencies: {
                let section = format!("agency-comparison-{branch}");
                let file = match saved {
                    Some(previous) if previous != key => {
                        store.read_prefix(previous, key, &section, 0)?
                    }
                    _ => store.read(key, &section, 0)?,
                }
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared agency continuation is missing")
                })?;
                super::agency_comparison::prepared::Prepared::read(io::BufReader::new(file))?
            },
            roster: {
                let section = format!("operatives-roster-{branch}");
                let file = match saved {
                    Some(previous) if previous != key => {
                        store.read_prefix(previous, key, &section, 0)?
                    }
                    _ => store.read(key, &section, 0)?,
                }
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared roster continuation is missing")
                })?;
                super::operatives_roster::prepared::Prepared::read(io::BufReader::new(file))?
            },
            espionage: {
                let section = format!("espionage-log-{branch}");
                let file = match saved {
                    Some(previous) if previous != key => {
                        store.read_prefix(previous, key, &section, 0)?
                    }
                    _ => store.read(key, &section, 0)?,
                }
                .ok_or_else(|| {
                    dashboard_core::invalid("Prepared espionage continuation is missing")
                })?;
                super::espionage_log::prepared::Prepared::read(io::BufReader::new(file))?
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::{
        prepared::{frame_changes::FrameChangeTracker, observer_tests, series::tests::Fixture},
        test_fixtures,
    };
    use replay::{
        container::Replay,
        frame::TableView,
        rows::{
            CountryLinkRow, CountryRow, IntelligenceAgencyRow, SimulationFeatureRow, TableId,
            TableRows,
        },
    };
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
        input.metrics.table_ns = [0; replay::frame::TABLE_COUNT];
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
        engine::executable_tables::initialize_for_tests()
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
                options: installed_content::ResolveOptions {
                    game_dir: folder.0.clone(),
                    user_dir: folder.0.join("user"),
                    cache_dir: folder.0.join("cache"),
                },
                allow_unverified: true,
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
