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
    Hourly => collector_hooks::hook::Hook,
    EquipmentDestruction => collector_equipment_destruction::equipment_destruction::Hooks,
    Land => collector_land_events::land_events::Hooks,
    Portrait => collector_characters::portrait_changes::Hooks,
    Opinion => collector_observers::opinion_changes::Hooks,
    Construction => collector_output::construction_ic::Hooks,
    LineOutput => collector_output::line_output::Hooks,
    CombatLogPrune => collector_combat_log::combat_log_prune::Hooks,
    Advisor => collector_characters::advisor_changes::Hooks,
    Relation => collector_air_relations::relation_changes::Hooks,
    History => collector_air_histories::history_changes::Hooks,
    Flags => collector_intel::flag_changes::Hooks,
    Buildings => collector_construction::building_changes::Hooks,
    AdvisorPools => collector_observers::advisor_pool_changes::Hooks,
    PartyLeaders => collector_governance::party_leader_changes::Hooks,
    Variables => collector_intel::variable_changes::Hooks,
    Parties => collector_governance::party_changes::Hooks,
    Wings => collector_observers::wing_changes::Hooks,
    Roster => collector_observers::roster_changes::Hooks,
    PoliticalStatus => collector_observers::political_status_changes::Hooks,
    Missions => collector_missions::mission_changes::Hooks,
    Rails => collector_observers::rail_changes::Hooks,
    Territory => collector_observers::territory_changes::Hooks,
    ArmyStats => collector_division_parts::army_stat_changes::Hooks,
    CountryInputs => collector_observers::country_input_changes::Hooks,
    Raid => collector_observers::raid_changes::Hooks,
    NavalTrees => collector_observers::naval_tree_changes::Hooks,
    UnitLeaders => collector_observers::unit_leader_changes::Hooks,
    Census => collector_observers::census_changes::Hooks,
    OrdersGroups => collector_observers::orders_group_changes::Hooks,
    OrderMembership => collector_observers::order_membership_changes::Hooks,
    WorldInputs => collector_observers::world_input_changes::Hooks,
    AirRecords => collector_air_records::air_record_changes::Hooks,
    LocalSubunits => collector_model_inputs::local_subunit_changes::Hooks,
    Templates => collector_division_templates::template_changes::Hooks,
    ShipModels => collector_ship_models::ship_model_changes::Hooks,
    ProvinceUnits => collector_divisions::province_unit_changes::Hooks,
    ProvinceCombats => collector_observers::province_combat_changes::Hooks,
    CombatEngagementRefreshes => collector_observers::combat_engagement_refreshes::Hooks,
    AirBaseCarry => collector_observers::air_base_carry_changes::Hooks,
    Experience => collector_experience_gains::experience_gains::Hooks,
    PlayerCommands => collector_player_commands::player_commands::Hooks,
    Fuel => collector_fuel_awards::fuel_awards::Hooks,
    OperationOutcomes => collector_operation_outcomes::operation_outcomes::Hooks,
    Politics => collector_politics::politics_changes::Hooks,
    Logistics => collector_logistics::logistics_balance::Hooks,
    Peace => collector_peace::peace_settlements::Hooks,
    NavalResultSources => collector_naval_results::naval_result_sources::Hooks,
    AirLosses => collector_observers::air_losses::Hooks,
    BuildingBombing => collector_observers::building_bombing::Hooks,
    NuclearStrikes => collector_observers::nuclear_strikes::Hooks,
    MapEvents => collector_map_events::hooks::Hooks,
    EventFeed => collector_observers::event_feed_observer::Hooks,
    Propaganda => collector_intel::propaganda_changes::Hooks,
    IntelSources => collector_intel::source_changes::Hooks,
    NetworkAuxiliary => collector_intel::network_auxiliary::Hooks,
    NavalManagement => collector_naval_management::changes::Hooks,
    WarParticipation => collector_war_overview::participation::Hooks,
    Designs => collector_production::production::design_changes::Hooks,
    FactionGoals => collector_factions::faction_goal_changes::Hooks,
    FactionCompanions => collector_factions::faction_companion_changes::Hooks,
    Market => collector_observers::market_changes::Hooks,
    CommandBatch => collector_capture::command_batch::Hooks,
    WorldLifetime => collector_lifetime::world_lifetime::Hooks,
}

pub struct FamilyHooks<T> {
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
        if !admit(crate::hook_inventory::patches, family, installation_message)? {
            return Ok(Self {
                inner: None,
                family,
                base,
            });
        }
        let (result, report) = collector_hooks::hook::installation::capture(|| {
            let result = construct();
            if let Err(cause) = &result {
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Construction,
                    None,
                    cause.as_ref(),
                );
            }
            result
        });
        let diagnostic = audit_report(crate::hook_inventory::patches, family, base, report)?;
        let hooks = Self {
            inner: result.ok(),
            family,
            base,
        };
        if let Some(diagnostic) = diagnostic {
            refuse_diagnostic(
                crate::hook_inventory::patches,
                family,
                diagnostic,
                installation_message,
            )?;
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
                refuse(
                    crate::hook_inventory::patches,
                    family,
                    "construction",
                    cause,
                )?;
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
        let (result, report) = collector_hooks::hook::installation::capture(|| {
            let result = toggle(hooks, enabled);
            if requested && let Err(cause) = &result {
                collector_hooks::hook::installation::refused(
                    if enabled {
                        collector_hooks::hook::installation::Kind::Publication
                    } else {
                        collector_hooks::hook::installation::Kind::Rollback
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
            if let Some(diagnostic) = audit_report(
                crate::hook_inventory::patches,
                self.family,
                self.base,
                report,
            )? {
                remember_failure(self.family, diagnostic, installation_message);
                return Err(refused);
            }
        }
        result.map_err(|_| refused)
    }
}
macro_rules! family_hooks {
    (@set_enabled $hooks:ident, $enabled:ident, unsafe) => {
        // SAFETY: The runtime's publication round owns these taps and retains them through
        // publication and native callback quiescence.
        unsafe { $hooks.set_enabled($enabled) }
    };
    (@set_enabled $hooks:ident, $enabled:ident) => {
        $hooks.set_enabled($enabled)
    };
    ($( $($module:ident)::+ $(; $safety:ident)? ),* $(,)?) => {$ (
        impl FamilyHooks<$($module)::+::Hooks> {
            pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
                self.toggle(enabled, |hooks, enabled|
                family_hooks!(@set_enabled hooks, enabled $(, $safety)?).map_err(|cause|
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
    collector_intel::propaganda_changes,
    collector_intel::source_changes,
    collector_intel::network_auxiliary,
    collector_naval_management::changes,
    collector_war_overview::participation,
    collector_production::production::design_changes,
    collector_factions::faction_goal_changes,
    collector_factions::faction_companion_changes,
    collector_observers::market_changes,
    collector_capture::command_batch; unsafe,
    collector_equipment_destruction::equipment_destruction,
    collector_land_events::land_events,
    collector_characters::portrait_changes,
    collector_observers::opinion_changes,
    collector_output::construction_ic,
    collector_output::line_output,
    collector_combat_log::combat_log_prune,
    collector_characters::advisor_changes,
    collector_air_relations::relation_changes,
    collector_air_histories::history_changes,
    collector_intel::flag_changes,
    collector_construction::building_changes,
    collector_observers::advisor_pool_changes,
    collector_governance::party_leader_changes,
    collector_intel::variable_changes,
    collector_governance::party_changes,
    collector_observers::wing_changes,
    collector_observers::roster_changes,
    collector_observers::political_status_changes,
    collector_missions::mission_changes,
    collector_observers::rail_changes,
    collector_observers::territory_changes,
    collector_division_parts::army_stat_changes,
    collector_observers::country_input_changes,
    collector_observers::raid_changes,
    collector_observers::naval_tree_changes,
    collector_observers::unit_leader_changes,
    collector_observers::census_changes,
    collector_observers::orders_group_changes,
    collector_observers::order_membership_changes,
    collector_observers::world_input_changes,
    collector_air_records::air_record_changes,
    collector_model_inputs::local_subunit_changes,
    collector_division_templates::template_changes,
    collector_ship_models::ship_model_changes,
    collector_divisions::province_unit_changes,
    collector_observers::province_combat_changes,
    collector_observers::combat_engagement_refreshes,
    collector_observers::air_base_carry_changes,
    collector_experience_gains::experience_gains,
    collector_player_commands::player_commands; unsafe,
    collector_fuel_awards::fuel_awards,
    collector_operation_outcomes::operation_outcomes,
    collector_politics::politics_changes,
    collector_logistics::logistics_balance,
    collector_peace::peace_settlements,
    collector_naval_results::naval_result_sources,
    collector_observers::air_losses,
    collector_observers::building_bombing,
    collector_observers::nuclear_strikes,
    collector_observers::event_feed_observer,
    collector_lifetime::world_lifetime
);
impl FamilyHooks<collector_map_events::hooks::Hooks> {
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
                assert!(
                    refuse(
                        crate::hook_inventory::patches,
                        family,
                        "publication",
                        "synthetic publication failure"
                    )
                    .is_err()
                );
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
                        install_failed(
                            crate::hook_inventory::patches,
                            family,
                            cause,
                            installation_message,
                        )
                        .unwrap();
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
                    Ok::<_, &'static str>(unsafe {
                        collector_player_commands::player_commands::Hooks::new(
                            &memory,
                            0,
                            collector_capture::sampling::sample_command_roster,
                        )
                    })
                })
                .unwrap();
                for enabled in [true, false] {
                    assert_eq!(hooks.set_enabled(enabled), Ok(true));
                    assert_eq!(hooks.executable_ranges().count(), 0);
                }
                let mut drain = collector_player_commands::player_commands::Drain::default();
                for final_drain in [false, true] {
                    assert!(
                        // SAFETY: The sole spool drains its current installation, with hooks quiet for
                        // a final drain.
                        unsafe { drain.take(final_drain) }.unwrap().is_empty()
                    );
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
