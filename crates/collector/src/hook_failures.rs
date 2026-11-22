use crate::runtime::installation_message;
use collector_hook_failures::{
    Family, admit, audit_report, failed, refuse_diagnostic, remember_failure,
};
#[cfg(test)]
use collector_hook_failures::{
    apply, configure, fatal_installation_error, install_failed, refuse, reset_failures_for_test,
    status,
};
#[cfg(test)]
use replay_data::rows::TableId;
#[cfg(test)]
use replay_frame::frame::{FrameInput, TableView};
use std::ops::Range;

pub(crate) trait RegisteredFamily {
    const FAMILY: Family;
}

macro_rules! runtime_families {
    ($($family:ident => $ty:ty),* $(,)?) => {
        $(impl RegisteredFamily for $ty { const FAMILY: Family = Family::$family; })*
        #[cfg(test)]
        pub(crate) fn runtime_type(family: Family) -> &'static str {
            match family { $(Family::$family => stringify!($ty)),* }
        }
    };
}
runtime_families! {
    Hourly => crate::hook::Hook,
    EquipmentDestruction => collector_equipment_destruction::equipment_destruction::Hooks,
    Land => collector_land_events::land_events::Hooks,
    Portrait => collector_characters::portrait_changes::Hooks,
    Opinion => crate::opinion_changes::Hooks,
    Construction => crate::construction_ic::Hooks,
    LineOutput => crate::line_output::Hooks,
    CombatLogPrune => crate::combat_log_prune::Hooks,
    Advisor => collector_characters::advisor_changes::Hooks,
    Relation => crate::relation_changes::Hooks,
    History => crate::history_changes::Hooks,
    Flags => crate::flag_changes::Hooks,
    Buildings => crate::building_changes::Hooks,
    AdvisorPools => crate::advisor_pool_changes::Hooks,
    PartyLeaders => crate::party_leader_changes::Hooks,
    Variables => crate::variable_changes::Hooks,
    Parties => crate::party_changes::Hooks,
    Wings => crate::wing_changes::Hooks,
    Roster => crate::roster_changes::Hooks,
    PoliticalStatus => crate::political_status_changes::Hooks,
    Missions => collector_missions::mission_changes::Hooks,
    Rails => crate::rail_changes::Hooks,
    Territory => crate::territory_changes::Hooks,
    ArmyStats => crate::army_stat_changes::Hooks,
    CountryInputs => crate::country_input_changes::Hooks,
    Raid => crate::raid_changes::Hooks,
    NavalTrees => crate::naval_tree_changes::Hooks,
    UnitLeaders => crate::unit_leader_changes::Hooks,
    Census => crate::census_changes::Hooks,
    OrdersGroups => crate::orders_group_changes::Hooks,
    OrderMembership => crate::order_membership_changes::Hooks,
    WorldInputs => crate::world_input_changes::Hooks,
    AirRecords => crate::air_record_changes::Hooks,
    LocalSubunits => crate::local_subunit_changes::Hooks,
    Templates => collector_division_templates::template_changes::Hooks,
    ShipModels => crate::ship_model_changes::Hooks,
    ProvinceUnits => collector_divisions::province_unit_changes::Hooks,
    ProvinceCombats => crate::province_combat_changes::Hooks,
    AirBaseCarry => crate::air_base_carry_changes::Hooks,
    Experience => crate::experience_gains::Hooks,
    PlayerCommands => crate::player_commands::Hooks,
    Fuel => crate::fuel_awards::Hooks,
    OperationOutcomes => collector_operation_outcomes::operation_outcomes::Hooks,
    Politics => crate::politics_changes::Hooks,
    Logistics => crate::logistics_balance::Hooks,
    Peace => crate::peace_settlements::Hooks,
    NavalResultSources => collector_naval_results::naval_result_sources::Hooks,
    AirLosses => crate::air_losses::Hooks,
    BuildingBombing => crate::building_bombing::Hooks,
    NuclearStrikes => crate::nuclear_strikes::Hooks,
    MapEvents => crate::map_events::hooks::Hooks,
    EventFeed => crate::event_feed_observer::Hooks,
    Propaganda => crate::propaganda_changes::Hooks,
    IntelSources => crate::intel_source_changes::Hooks,
    NetworkAuxiliary => crate::network_auxiliary::Hooks,
    NavalManagement => crate::naval_management_changes::Hooks,
    WarParticipation => crate::war_participation::Hooks,
    Designs => crate::design_changes::Hooks,
    FactionGoals => collector_factions::faction_goal_changes::Hooks,
    FactionCompanions => collector_factions::faction_companion_changes::Hooks,
    Market => crate::market_changes::Hooks,
    CommandBatch => crate::command_batch::Hooks,
    WorldLifetime => crate::world_lifetime::Hooks,
}

pub(crate) struct FamilyHooks<T> {
    pub(crate) inner: Option<T>,
    family: Family,
    base: usize,
}
impl<T: RegisteredFamily> FamilyHooks<T> {
    pub(crate) fn construct<E: AsRef<str>>(
        base: usize,
        construct: impl FnOnce() -> Result<T, E>,
    ) -> Result<Self, &'static str> {
        let family = T::FAMILY;
        if !admit(family, installation_message)? {
            return Ok(Self {
                inner: None,
                family,
                base,
            });
        }
        let (result, report) = crate::hook::installation::capture(|| {
            let result = construct();
            if let Err(cause) = &result {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    cause.as_ref(),
                );
            }
            result
        });
        let diagnostic = audit_report(family, base, report)?;
        let hooks = Self {
            inner: result.ok(),
            family,
            base,
        };
        if let Some(diagnostic) = diagnostic {
            refuse_diagnostic(family, diagnostic, installation_message)?;
        } else if hooks.inner.is_none() {
            return Err("hook constructor failed without an installation diagnostic");
        }
        Ok(hooks)
    }
}
impl<T> FamilyHooks<T> {
    #[cfg(test)]
    pub(crate) fn new(
        family: Family,
        hooks: Result<T, &'static str>,
    ) -> Result<Self, &'static str> {
        let inner = match hooks {
            Ok(hooks) => Some(hooks),
            Err(cause) => {
                refuse(family, "construction", cause)?;
                None
            }
        };
        Ok(Self {
            inner,
            family,
            base: 0,
        })
    }
    fn toggle(
        &mut self,
        enabled: bool,
        mut toggle: impl FnMut(&mut T, bool) -> Result<bool, std::borrow::Cow<'static, str>>,
    ) -> Result<bool, &'static str> {
        let requested = enabled;
        let enabled = enabled && !failed(self.family);
        let Some(hooks) = self.inner.as_mut() else {
            return Ok(true);
        };
        if !requested {
            return toggle(hooks, false).map_err(|cause| match cause {
                std::borrow::Cow::Borrowed(cause) => cause,
                std::borrow::Cow::Owned(_) => "hook restoration failed; collector retained",
            });
        }
        let (result, report) = crate::hook::installation::capture(|| {
            let result = toggle(hooks, enabled);
            if requested && let Err(cause) = &result {
                crate::hook::installation::refused(
                    if enabled {
                        crate::hook::installation::Kind::Publication
                    } else {
                        crate::hook::installation::Kind::Rollback
                    },
                    None,
                    cause.as_ref(),
                );
            }
            result
        });
        let refused = if enabled {
            "hook publication refused"
        } else {
            "hook restoration failed; collector retained"
        };
        if report.failure.is_some() || !report.sites.is_empty() {
            if let Some(diagnostic) = audit_report(self.family, self.base, report)? {
                remember_failure(self.family, diagnostic, installation_message);
                return Err(refused);
            }
        }
        result.map_err(|_| refused)
    }
}
macro_rules! family_hooks {
    ($( $($module:ident)::+ ),* $(,)?) => {$ (
        impl FamilyHooks<$($module)::+::Hooks> {
            pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
                self.toggle(enabled, |hooks, enabled| hooks.set_enabled(enabled).map_err(|cause|
                    if enabled {
                        cause.into()
                    } else {
                        std::borrow::Cow::Borrowed("hook restoration failed; collector retained")
                    }))
            }
            pub(crate) fn executable_ranges(&self) -> impl Iterator<Item=Range<usize>> + '_ {
                self.inner.iter().flat_map(|hooks| hooks.executable_ranges().into_iter())
            }
        }
    )*};
}
family_hooks!(
    crate::propaganda_changes,
    crate::intel_source_changes,
    crate::network_auxiliary,
    crate::naval_management_changes,
    crate::war_participation,
    crate::design_changes,
    collector_factions::faction_goal_changes,
    collector_factions::faction_companion_changes,
    crate::market_changes,
    crate::command_batch,
    collector_equipment_destruction::equipment_destruction,
    collector_land_events::land_events,
    collector_characters::portrait_changes,
    crate::opinion_changes,
    crate::construction_ic,
    crate::line_output,
    crate::combat_log_prune,
    collector_characters::advisor_changes,
    crate::relation_changes,
    crate::history_changes,
    crate::flag_changes,
    crate::building_changes,
    crate::advisor_pool_changes,
    crate::party_leader_changes,
    crate::variable_changes,
    crate::party_changes,
    crate::wing_changes,
    crate::roster_changes,
    crate::political_status_changes,
    collector_missions::mission_changes,
    crate::rail_changes,
    crate::territory_changes,
    crate::army_stat_changes,
    crate::country_input_changes,
    crate::raid_changes,
    crate::naval_tree_changes,
    crate::unit_leader_changes,
    crate::census_changes,
    crate::orders_group_changes,
    crate::order_membership_changes,
    crate::world_input_changes,
    crate::air_record_changes,
    crate::local_subunit_changes,
    collector_division_templates::template_changes,
    crate::ship_model_changes,
    collector_divisions::province_unit_changes,
    crate::province_combat_changes,
    crate::air_base_carry_changes,
    crate::experience_gains,
    crate::player_commands,
    crate::fuel_awards,
    collector_operation_outcomes::operation_outcomes,
    crate::politics_changes,
    crate::logistics_balance,
    crate::peace_settlements,
    collector_naval_results::naval_result_sources,
    crate::air_losses,
    crate::building_bombing,
    crate::nuclear_strikes,
    crate::event_feed_observer,
    crate::world_lifetime
);
impl FamilyHooks<crate::map_events::hooks::Hooks> {
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        self.toggle(enabled, |hooks, enabled| {
            hooks.set_enabled(enabled).map_err(Into::into)
        })
    }
    pub(crate) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.inner
            .iter()
            .flat_map(|hooks| hooks.executable_ranges())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::frame::{FrameInput, Metrics, TABLE_COUNT};

    #[test]
    fn every_runtime_family_contains_failures_and_restores_safely() {
        const CHILD: &str = "BATTLEPLAN_HOOK_CONTAINMENT_TEST";
        const TEST: &str =
            "hook_failures::tests::every_runtime_family_contains_failures_and_restores_safely";
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
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let directory =
            std::env::temp_dir().join(format!("hoi4-hooks-contained-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        configure(&directory, 1, 1000, crate::status_file::write_status_bytes);
        for family in Family::ALL {
            reset_failures_for_test();
            if family.essential().is_some() {
                assert!(
                    FamilyHooks::<bool>::new(family, Err("synthetic sentinel differs")).is_err()
                );
                assert!(refuse(family, "publication", "synthetic publication failure").is_err());
                continue;
            }
            if matches!(
                family,
                Family::PlayerCommands | Family::OrderMembership | Family::CommandBatch
            ) {
                assert!(family.tables().is_empty());
            } else {
                assert!(
                    !family.tables().is_empty(),
                    "{} needs a refusal contract",
                    family.name()
                );
            }
            let mut broken = FamilyHooks::new(family, Ok(false)).unwrap();
            assert_eq!(
                broken.toggle(true, |_, _| Ok(false)),
                Ok(false),
                "busy is retryable"
            );
            let mut other = FamilyHooks::new(Family::Hourly, Ok(false)).unwrap();
            let progress = crate::progress::Progress::open(&directory);
            let stage = crate::progress::Stage::new(&progress, family.name());
            let mut attempts = 0;
            loop {
                attempts += 1;
                assert!(attempts <= 2, "failed family retried");
                let result = crate::runtime::install_attach_batch(
                    &stage,
                    &mut [
                        (family.name(), &mut || {
                            broken.toggle(true, |installed, enabled| {
                                *installed = enabled;
                                if enabled {
                                    Err("synthetic publication failure".into())
                                } else {
                                    Ok(true)
                                }
                            })
                        }),
                        ("hourly", &mut || {
                            other.toggle(true, |installed, enabled| {
                                *installed = enabled;
                                Ok(true)
                            })
                        }),
                    ],
                );
                match result {
                    Ok(true) => break,
                    Err(cause) => {
                        assert_eq!(stage.get(), family.name());
                        install_failed(family, cause, installation_message).unwrap();
                    }
                    Ok(false) => panic!("synthetic families are not busy"),
                }
            }
            assert_eq!(attempts, 2);
            assert_eq!(
                broken.inner,
                Some(false),
                "partial publication must roll back"
            );
            assert_eq!(
                other.inner,
                Some(true),
                "independent family was not installed"
            );
            let mut source = FrameInput {
                branch: 1,
                hour: 10,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|index| {
                    TableView::Complete(replay_data::rows::TableRows::empty(TableId::ALL[index]))
                        .into()
                }),
            };
            apply(&mut source).unwrap();
            for table in super::contract_tests::expected_tables(family) {
                if family == Family::Templates {
                    assert!(matches!(
                        &source.tables[table.index()],
                        replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
                    ));
                    continue;
                }
                assert!(
                    matches!(&source.tables[table.index()], replay_frame::frame::TableInput::Sampled(TableView::Unavailable { reason })
                    if reason.starts_with("hook family ") && reason.contains("not installed:") && reason.contains("synthetic publication failure")),
                    "{} {table:?}",
                    family.name()
                );
            }
            for table in family.tables() {
                if !super::contract_tests::expected_tables(family).contains(table) {
                    assert!(
                        matches!(
                            &source.tables[table.index()],
                            replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
                        ),
                        "{} {table:?} discarded fresh rows",
                        family.name()
                    );
                }
            }
            assert!(matches!(
                &source.tables[TableId::Clock.index()],
                replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
            ));
            let persisted: serde_json::Value =
                serde_json::from_slice(&std::fs::read(directory.join("status.json")).unwrap())
                    .unwrap();
            assert!(
                persisted["failed_hook_families"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["family"] == family.name())
            );
            let status = serde_json::to_value(status()).unwrap();
            let failure = status
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["family"] == family.name())
                .unwrap();
            assert_eq!(failure["stage"], "publication");
            assert!(!failure["site"].as_str().unwrap().is_empty());
            if family == Family::PlayerCommands {
                let memory = engine_native::memory::SliceMemory {
                    base: 0,
                    bytes: &[],
                };
                let mut hooks = FamilyHooks::construct(0, || {
                    Ok::<_, &'static str>(unsafe { crate::player_commands::Hooks::new(&memory, 0) })
                })
                .unwrap();
                for enabled in [true, false] {
                    assert_eq!(hooks.set_enabled(enabled), Ok(true));
                    assert_eq!(hooks.executable_ranges().count(), 0);
                }
                let mut drain = crate::player_commands::Drain::default();
                for final_drain in [false, true] {
                    assert!(drain.take(final_drain).unwrap().is_empty());
                }
                assert!(source.tables.iter().all(|table| matches!(
                    table,
                    replay_frame::frame::TableInput::Sampled(TableView::Complete(_))
                )));
            }
            let mut restored = false;
            broken
                .toggle(false, |_, enabled| {
                    assert!(!enabled);
                    restored = true;
                    Ok(true)
                })
                .unwrap();
            assert!(restored);
            other
                .toggle(false, |installed, enabled| {
                    *installed = enabled;
                    Ok(true)
                })
                .unwrap();
            assert_eq!(other.inner, Some(false));
            let mut absent = FamilyHooks::<bool>::new(family, Err("constructor failed")).unwrap();
            absent
                .toggle(false, |_, _| panic!("unconstructed family restored"))
                .unwrap();
            assert_eq!(source.tables.len(), TABLE_COUNT);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

impl FamilyHooks<collector_factions::faction_goal_changes::Hooks> {
    pub(crate) fn busy_sites(&mut self) -> Vec<&'static str> {
        self.inner
            .as_mut()
            .map_or_else(Vec::new, |hooks| hooks.busy_sites())
    }
}

impl FamilyHooks<collector_factions::faction_companion_changes::Hooks> {
    pub(crate) fn busy_sites(&mut self) -> Vec<&'static str> {
        self.inner
            .as_mut()
            .map_or_else(Vec::new, |hooks| hooks.busy_sites())
    }
}

#[cfg(test)]
#[path = "hook_contract_tests.rs"]
pub(crate) mod contract_tests;

#[cfg(test)]
#[path = "hook_fresh_reads_tests.rs"]
mod fresh_reads_tests;

#[cfg(test)]
#[path = "hook_fresh_producer_tests.rs"]
mod fresh_producer_tests;

#[cfg(test)]
#[path = "hook_constructor_gate.rs"]
mod constructor_gate;
