use super::{
    Captured, Divisions, MISSING, active_abilities, air, army_icon_inputs, character_inputs,
    characters, combat_details, combat_log, construction, country_appearance, country_counters,
    country_factories, country_stability, country_tactic_unlocks, division_templates, economy,
    faction_theatres, fog, game_score, group_counters, hired_advisors, industrial_organisations,
    leader_paths, leader_portraits, logistics, map_facilities, map_names, model_inputs,
    modifier_definitions, naval_bases, naval_combat_sources, naval_counters, naval_details,
    naval_history, naval_mission_inputs, naval_result_details, peace_bids, production,
    production_modifiers, projects, raid_launch_inputs, raid_state_inputs, raid_target_statuses,
    railway_guns, saved_event_targets, scatter, session, unpublished, visibility, war_overview,
};
use crate::{division_census::UnitInputs, map_capture::Orders, sampling::qpc};
use engine::{Memory, walk::Outcome};
use replay::{frame::TABLE_COUNT, rows::TableId};
use std::mem::size_of;
use std::sync::Arc;

impl Captured {
    #[cfg(not(test))]
    pub(super) fn fail_missing_slow(&mut self, hour: i32, force_full: bool) {
        let mut packs = self.slow_spare.take().expect("slow spare present");
        self.swap_slow(&mut packs);
        packs.fail(hour, force_full, "slow acquisition not supplied");
        self.swap_slow(&mut packs);
        self.slow_spare = Some(packs);
    }
    #[cfg(not(test))]
    pub(super) fn fail_missing_raid_state(&mut self, hour: i32, force_full: bool) {
        let mut packs = self
            .raid_state_spare
            .take()
            .expect("raid_state spare present");
        self.swap_raid_state(&mut packs);
        packs.fail(hour, force_full, "raid_state acquisition not supplied");
        self.swap_raid_state(&mut packs);
        self.raid_state_spare = Some(packs);
    }
    #[cfg(not(test))]
    pub(super) fn fail_missing_raid_launch(&mut self, hour: i32, force_full: bool) {
        self.raid_launch_inputs.fail_capture(
            hour,
            force_full,
            "raid launch acquisition not supplied",
        );
    }
    #[cfg(not(test))]
    pub(super) fn fail_missing_gated(&mut self, hour: i32, force_full: bool) {
        let mut packs = self.gated_spare.take().expect("gated spare present");
        self.swap_gated(&mut packs);
        packs.fail(hour, force_full, "gated acquisition not supplied");
        self.swap_gated(&mut packs);
        self.gated_spare = Some(packs);
    }
    #[cfg(not(test))]
    pub(super) fn fail_missing_late(&mut self, hour: i32, force_full: bool, orders: &mut Orders) {
        let mut packs = self.late_spare.take().expect("late spare present");
        self.swap_late(&mut packs, orders);
        packs.fail(hour, force_full, "late acquisition not supplied");
        self.swap_late(&mut packs, orders);
        self.late_spare = Some(packs);
    }
    #[cfg(not(test))]
    pub(super) fn fail_missing_raid_targets(&mut self, hour: i32, force_full: bool) {
        self.raid_target_statuses.fail_capture(
            hour,
            force_full,
            "raid target acquisition not supplied",
        );
    }

    /// The Countries worker owns the original naming buffers until its early
    /// join. Ordinary and Slow can take them only after this loan is restored.
    pub(crate) fn take_countries_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        rows: &mut Vec<engine::ccountry::Row>,
        links: &mut Vec<engine::ccountry::LinkRow>,
        clock: (i32, bool),
    ) -> crate::acquisition_jobs::CountriesRequest {
        crate::acquisition_jobs::CountriesRequest::take_with_naming(
            countries,
            base,
            state,
            rows,
            links,
            &mut self.country_appearance,
            &mut self.division_templates,
            clock.0,
            clock.1,
        )
    }

    pub(crate) fn restore_countries_request(
        &mut self,
        request: crate::acquisition_jobs::CountriesRequest,
        rows: &mut Vec<engine::ccountry::Row>,
        links: &mut Vec<engine::ccountry::LinkRow>,
        failure: Option<&'static str>,
    ) {
        request.restore_with_naming(
            rows,
            links,
            &mut self.country_appearance,
            &mut self.division_templates,
            failure,
        );
    }

    pub(crate) fn naval_breakdown_status(&self) -> naval_details::BreakdownStatus {
        let mut status = self.naval_details.breakdown_status();
        if let Some(reason) = self.ordinary_failure {
            status.outcome = Outcome::Unavailable { reason };
        }
        status
    }

    /// Lend the ordinary packs to the pool: swap them with the spare set so
    /// nothing allocates on the game thread.
    pub(crate) fn take_ordinary_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        fresh_modifier_guard: Result<(), &'static str>,
        naval_missions: NavalMissionLoan,
    ) -> crate::acquisition_jobs::OrdinaryRequest {
        self.ordinary_ticks = None;
        self.ordinary_failure = None;
        let mut packs = self.ordinary_spare.take().expect("ordinary spare present");
        self.swap_ordinary(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        packs.failure = None;
        let models = packs
            .country_appearance
            .transport_missions_mut()
            .prepare_transport_models(hour);
        if self
            .ship_models
            .lend_transport_models(models, base, state, hour)
            .is_err()
        {
            models.fail();
        }
        crate::acquisition_jobs::OrdinaryRequest {
            countries: Arc::clone(countries),
            unit_inputs: self.unit_inputs.as_ref().map(Arc::clone),
            base,
            game_state: state,
            hour,
            fresh_modifier_guard,
            transport_admission: crate::transport_admission::current(base, state, hour),
            transport_modifier_guard: fresh_modifier_guard,
            naval_missions,
            packs,
        }
    }

    /// Every terminal job installs a completion marker, including failure, so
    /// table assembly cannot retry an allocating walk on the callback.
    pub(crate) fn restore_ordinary_request(
        &mut self,
        request: crate::acquisition_jobs::OrdinaryRequest,
        failure: Option<&'static str>,
    ) -> NavalMissionLoan {
        let mut packs = request.packs;
        let mut naval_missions = request.naval_missions;
        if let Some(reason) = failure {
            packs.fail(request.hour, naval_missions.force_full, reason);
        }
        if let Some(fingerprints) = naval_missions.fingerprints.as_mut() {
            if packs.failure.is_some() {
                fingerprints.forget();
            } else {
                let outcomes = packs.naval_mission_inputs.outcomes();
                if naval_mission_inputs::SLICED
                    .iter()
                    .any(|&index| !matches!(outcomes[index], Outcome::Rows { .. }))
                {
                    packs
                        .naval_mission_inputs
                        .selection()
                        .invalidate(fingerprints);
                }
            }
        }
        self.ordinary_ticks = Some(packs.ticks);
        self.ordinary_failure = packs.failure;
        self.swap_ordinary(&mut packs);
        self.ordinary_spare = Some(packs);
        naval_missions
    }

    fn swap_ordinary(&mut self, packs: &mut OrdinaryPacks) {
        std::mem::swap(&mut self.combat_details, &mut packs.combat_details);
        std::mem::swap(&mut self.naval_details, &mut packs.naval_details);
        std::mem::swap(
            &mut self.naval_result_details,
            &mut packs.naval_result_details,
        );
        std::mem::swap(&mut self.country_appearance, &mut packs.country_appearance);
        std::mem::swap(&mut self.naval_bases, &mut packs.naval_bases);
        std::mem::swap(&mut self.map_facilities, &mut packs.map_facilities);
        std::mem::swap(
            &mut self.naval_combat_sources,
            &mut packs.naval_combat_sources,
        );
        std::mem::swap(
            &mut self.naval_mission_inputs,
            &mut packs.naval_mission_inputs,
        );
        std::mem::swap(&mut self.naval_counters, &mut packs.naval_counters);
        std::mem::swap(&mut self.railway_guns, &mut packs.railway_guns);
        std::mem::swap(&mut self.active_abilities, &mut packs.active_abilities);
        std::mem::swap(&mut self.country_counters, &mut packs.country_counters);
        // The map's typed air fallback runs before Ordinary joins.
        self.game_score.swap_country(&mut packs.game_score);
        std::mem::swap(&mut self.group_counters, &mut packs.group_counters);
        std::mem::swap(
            &mut self.saved_event_targets,
            &mut packs.saved_event_targets,
        );
    }

    /// Loan the slow packs with the stability family's fingerprint store.
    pub(crate) fn take_slow_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        stores: SlowStores,
    ) -> crate::acquisition_jobs::SlowRequest {
        self.slow_ticks = None;
        let mut packs = self.slow_spare.take().expect("slow spare present");
        self.swap_slow(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::SlowRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            directed_query_admission: crate::transport_admission::current(base, state, hour),
            packs,
            stores,
        }
    }

    /// Returns the Slow job's fingerprint stores: forgotten if the job
    /// failed, and with this hour's selection invalidated where a family's
    /// table was not read completely, so its countries are read again at the
    /// next hour.
    pub(crate) fn restore_slow_request(
        &mut self,
        request: crate::acquisition_jobs::SlowRequest,
        completed: bool,
    ) -> SlowStores {
        let mut packs = request.packs;
        let mut stores = request.stores;
        if !completed {
            // The Combat Log read may have advanced its countries' last
            // complete read before failing.
            combat_log::forget_reads();
            packs.fail(request.hour, request.force_full, "slow acquisition failed");
            if let Some(store) = stores.stability.as_mut() {
                store.forget();
            }
        }
        if let Some(fingerprints) = stores.stability.as_mut()
            && packs.country_stability.outcomes().iter().any(|outcome| {
                !matches!(outcome, Outcome::Rows { .. })
                    && !matches!(outcome, Outcome::Unavailable { reason } if *reason == replay::cadence::SKIPPED_REASON)
            })
        {
            packs.country_stability.selection().invalidate(fingerprints);
        }
        self.slow_ticks = Some(packs.ticks);
        self.swap_slow(&mut packs);
        self.slow_spare = Some(packs);
        stores
    }

    /// Loan the raid target statuses to their own pool job, so they run
    /// beside the sliced raid families rather than after them.
    pub(crate) fn take_raid_targets_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::RaidTargetsRequest {
        self.raid_targets_ticks = None;
        let mut packs = self
            .raid_targets_spare
            .take()
            .expect("raid targets spare present");
        std::mem::swap(
            &mut self.raid_target_statuses,
            &mut packs.raid_target_statuses,
        );
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::RaidTargetsRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

    /// Restore the raid target statuses.
    pub(crate) fn restore_raid_targets_request(
        &mut self,
        request: crate::acquisition_jobs::RaidTargetsRequest,
        completed: bool,
    ) {
        let mut packs = request.packs;
        if !completed {
            packs.raid_target_statuses.fail_capture(
                request.hour,
                request.force_full,
                "raid target acquisition failed",
            );
            packs.ticks = [0; TABLE_COUNT];
        }
        self.raid_targets_ticks = Some(packs.ticks);
        std::mem::swap(
            &mut self.raid_target_statuses,
            &mut packs.raid_target_statuses,
        );
        self.raid_targets_spare = Some(packs);
    }

    /// Loan the sliced state and building inputs, with their fingerprint
    /// stores, to their own pool job.
    pub(crate) fn take_raid_state_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        stores: RaidStateStores,
    ) -> crate::acquisition_jobs::RaidStateRequest {
        self.raid_state_ticks = None;
        let mut packs = self
            .raid_state_spare
            .take()
            .expect("raid state spare present");
        self.swap_raid_state(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::RaidStateRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            stores,
        }
    }

    /// Restore the raid state packs and return their stores: forgotten if the
    /// job was interrupted (every owner is then new and read at the next
    /// hour), and with a family's selection invalidated if any of its tables
    /// was not read completely, so its owners are read again at the next hour.
    pub(crate) fn restore_raid_state_request(
        &mut self,
        request: crate::acquisition_jobs::RaidStateRequest,
        completed: bool,
    ) -> RaidStateStores {
        let mut packs = request.packs;
        let mut stores = request.stores;
        if !completed {
            packs.fail(
                request.hour,
                request.force_full,
                "raid state acquisition failed",
            );
            stores.states.forget();
            stores.buildings.forget();
        }
        self.raid_state_ticks = Some(packs.ticks);
        self.swap_raid_state(&mut packs);
        self.raid_state_spare = Some(packs);
        let state = self.raid_state_inputs.outcomes();
        if unpublished(&raid_state_inputs::STATE_TABLES.map(|index| state[index])) {
            self.raid_state_inputs
                .state_selection()
                .invalidate(&mut stores.states);
        }
        if unpublished(&state[3..4]) {
            self.raid_state_inputs
                .country_selection()
                .invalidate(&mut stores.buildings);
        }
        stores
    }

    /// Loan the sliced raid launch inputs and their store to their own job.
    pub(crate) fn take_raid_launch_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        store: Box<crate::slicing::SliceFingerprints>,
    ) -> crate::acquisition_jobs::RaidLaunchRequest {
        self.raid_launch_ticks = None;
        let mut packs = self
            .raid_launch_spare
            .take()
            .expect("raid launch spare present");
        std::mem::swap(&mut self.raid_launch_inputs, &mut packs.raid_launch_inputs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::RaidLaunchRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            store,
        }
    }

    /// Restore the raid launch pack and return its store, forgotten or
    /// invalidated as [`Self::restore_raid_state_request`] does.
    pub(crate) fn restore_raid_launch_request(
        &mut self,
        request: crate::acquisition_jobs::RaidLaunchRequest,
        completed: bool,
    ) -> Box<crate::slicing::SliceFingerprints> {
        let mut packs = request.packs;
        let mut store = request.store;
        if !completed {
            packs.raid_launch_inputs.fail_capture(
                request.hour,
                request.force_full,
                "raid launch acquisition failed",
            );
            packs.ticks = [0; TABLE_COUNT];
            store.forget();
        }
        self.raid_launch_ticks = Some(packs.ticks);
        std::mem::swap(&mut self.raid_launch_inputs, &mut packs.raid_launch_inputs);
        self.raid_launch_spare = Some(packs);
        let launch = self.raid_launch_inputs.outcomes();
        if unpublished(&[launch[0], launch[2], launch[3]]) {
            self.raid_launch_inputs.selection().invalidate(&mut store);
        }
        store
    }

    fn swap_raid_state(&mut self, packs: &mut RaidStatePacks) {
        std::mem::swap(&mut self.raid_state_inputs, &mut packs.raid_state_inputs);
    }

    fn swap_slow(&mut self, packs: &mut SlowPacks) {
        std::mem::swap(&mut self.peace_bids, &mut packs.peace_bids);
        std::mem::swap(&mut self.map_names, &mut packs.map_names);
        std::mem::swap(&mut self.division_templates, &mut packs.division_templates);
        std::mem::swap(&mut self.visibility, &mut packs.visibility);
        std::mem::swap(&mut self.faction_theatres, &mut packs.faction_theatres);
        std::mem::swap(&mut self.model_inputs, &mut packs.model_inputs);
        std::mem::swap(
            &mut self.modifier_definitions,
            &mut packs.modifier_definitions,
        );
        std::mem::swap(&mut self.hired_advisors, &mut packs.hired_advisors);
        std::mem::swap(&mut self.character_inputs, &mut packs.character_inputs);
        std::mem::swap(&mut self.logistics, &mut packs.logistics);
        std::mem::swap(&mut self.combat_log, &mut packs.combat_log);
        std::mem::swap(&mut self.country_factories, &mut packs.country_factories);
        std::mem::swap(&mut self.war_overview, &mut packs.war_overview);
        std::mem::swap(
            &mut self.industrial_organisations,
            &mut packs.industrial_organisations,
        );
        std::mem::swap(
            &mut self.production_modifiers,
            &mut packs.production_modifiers,
        );
        std::mem::swap(&mut self.country_stability, &mut packs.country_stability);
        std::mem::swap(
            &mut self.country_tactic_unlocks,
            &mut packs.country_tactic_unlocks,
        );
    }

    /// Loan the late packs and the walked order census to the job.
    /// `naval_raw`: the Naval raw job staged the order-of-battle sources.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_late_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        orders: &mut Orders,
        naval_raw: bool,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        air_scores: Box<engine::raid_air_scores::Held>,
    ) -> crate::acquisition_jobs::LateRequest {
        self.late_ticks = None;
        let mut packs = self.late_spare.take().expect("late spare present");
        self.swap_late(&mut packs, orders);
        packs.naval_raw = naval_raw;
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::LateRequest {
            countries: Arc::clone(countries),
            inputs: self.unit_inputs.clone(),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            plans: std::mem::replace(&mut self.plans, super::plans::Captured::vacant()),
            air_scores,
        }
    }

    /// Restore the late packs and return the held RaidAirScores table, which
    /// a failed job leaves to be read in full.
    pub(crate) fn restore_late_request(
        &mut self,
        request: crate::acquisition_jobs::LateRequest,
        completed: bool,
        orders: &mut Orders,
    ) -> Box<engine::raid_air_scores::Held> {
        let mut plans = request.plans;
        let mut packs = request.packs;
        let mut air_scores = request.air_scores;
        if !completed {
            packs.fail(request.hour, request.force_full, "late acquisition failed");
            air_scores.invalidate();
            plans.fail_unit_transports("late acquisition failed");
        }
        self.late_ticks = Some(packs.ticks);
        self.swap_late(&mut packs, orders);
        self.late_spare = Some(packs);
        self.plans = plans;
        self.plans
            .install_order_transport(&mut self.division_presentation);
        self.install_reserved_land_certificate();
        air_scores
    }

    /// Loan the raw-gated packs once every raw-copy verdict is in.
    pub(crate) fn take_gated_request(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::GatedRequest {
        self.gated_ticks = None;
        let mut packs = self.gated_spare.take().expect("gated spare present");
        self.swap_gated(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::GatedRequest {
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

    pub(crate) fn restore_gated_request(
        &mut self,
        request: crate::acquisition_jobs::GatedRequest,
        completed: bool,
    ) {
        let mut packs = request.packs;
        if !completed {
            packs.fail(request.hour, request.force_full, "gated acquisition failed");
        }
        self.gated_ticks = Some(packs.ticks);
        self.swap_gated(&mut packs);
        self.gated_spare = Some(packs);
    }

    fn swap_gated(&mut self, packs: &mut GatedPacks) {
        std::mem::swap(&mut self.fog, &mut packs.fog);
        std::mem::swap(&mut self.projects, &mut packs.projects);
        std::mem::swap(&mut self.construction, &mut packs.construction);
    }

    pub(super) fn swap_late(&mut self, packs: &mut LatePacks, orders: &mut Orders) {
        std::mem::swap(&mut self.leader_portraits, &mut packs.leader_portraits);
        std::mem::swap(&mut self.leader_paths, &mut packs.leader_paths);
        std::mem::swap(&mut self.air, &mut packs.air);
        std::mem::swap(&mut self.naval_history, &mut packs.naval_history);
        std::mem::swap(&mut self.army_icon_inputs, &mut packs.army_icon_inputs);
        std::mem::swap(&mut self.divisions, &mut packs.divisions);
        std::mem::swap(&mut self.air_root, &mut packs.air_root);
        std::mem::swap(orders, &mut packs.orders);
    }
}

/// The map counter inputs that need only live memory: one pool job from the
/// first dispatch burst.
pub(crate) struct OrdinaryPacks {
    pub(crate) combat_details: combat_details::Captured,
    pub(crate) naval_details: naval_details::Captured,
    pub(crate) naval_result_details: naval_result_details::Captured,
    pub(crate) country_appearance: country_appearance::Captured,
    pub(crate) naval_bases: naval_bases::Captured,
    pub(crate) map_facilities: map_facilities::Captured,
    pub(crate) naval_combat_sources: naval_combat_sources::Captured,
    pub(crate) naval_mission_inputs: naval_mission_inputs::Captured,
    pub(crate) naval_counters: naval_counters::Captured,
    pub(crate) railway_guns: railway_guns::Captured,
    pub(crate) active_abilities: active_abilities::Captured,
    pub(crate) country_counters: country_counters::Captured,
    pub(crate) game_score: game_score::Captured,
    pub(crate) group_counters: group_counters::Captured,
    pub(crate) saved_event_targets: saved_event_targets::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

impl OrdinaryPacks {
    pub(super) const TABLES: &'static [&'static [TableId]] = &[
        &combat_details::TABLES,
        &naval_details::TABLES,
        &naval_result_details::TABLES,
        &country_appearance::TABLES,
        &naval_bases::TABLES,
        &map_facilities::TABLES,
        &naval_combat_sources::TABLES,
        &naval_mission_inputs::TABLES,
        &naval_counters::TABLES,
        &railway_guns::TABLES,
        &active_abilities::TABLES,
        &country_counters::TABLES,
        &game_score::COUNTRY_TABLES,
        &group_counters::TABLES,
        &saved_event_targets::TABLES,
    ];

    pub(crate) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.combat_details.allocated_bytes()
            + self.naval_details.allocated_bytes()
            + self.naval_result_details.allocated_bytes()
            + self.country_appearance.allocated_bytes()
            + self.naval_bases.allocated_bytes()
            + self.map_facilities.allocated_bytes()
            + self.naval_combat_sources.allocated_bytes()
            + self.naval_mission_inputs.allocated_bytes()
            + self.naval_counters.allocated_bytes()
            + self.railway_guns.allocated_bytes()
            + self.active_abilities.allocated_bytes()
            + self.country_counters.allocated_bytes()
            + self.game_score.allocated_bytes()
            + self.group_counters.allocated_bytes()
            + self.saved_event_targets.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.combat_details.fail(reason);
        self.naval_details.reset();
        self.naval_result_details.reset();
        self.country_appearance.fail(reason);
        self.naval_bases.reset();
        self.map_facilities.reset();
        self.naval_combat_sources.reset();
        self.naval_mission_inputs
            .fail_capture(hour, force_full, reason);
        self.naval_counters.reset();
        self.railway_guns.reset();
        self.active_abilities.reset();
        self.country_counters.fail();
        self.game_score.fail(reason);
        self.group_counters.reset();
        self.saved_event_targets.reset();
        self.ticks = [0; TABLE_COUNT];
        self.failure = Some(reason);
    }
    /// Placeholders the extended capture holds while the pool owns the real
    /// packs. Only the reusable box allocates; the hourly exchange is a swap.
    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            combat_details: combat_details::Captured::vacant(),
            naval_details: naval_details::Captured::vacant(),
            naval_result_details: naval_result_details::Captured::vacant(),
            country_appearance: country_appearance::Captured::vacant(),
            naval_bases: naval_bases::Captured::vacant(),
            map_facilities: map_facilities::Captured::vacant(),
            naval_combat_sources: naval_combat_sources::Captured::vacant(),
            naval_mission_inputs: naval_mission_inputs::Captured::vacant(),
            naval_counters: naval_counters::Captured::vacant(),
            railway_guns: railway_guns::Captured::vacant(),
            active_abilities: active_abilities::Captured::vacant(),
            country_counters: country_counters::Captured::vacant(),
            game_score: game_score::Captured::vacant(),
            group_counters: group_counters::Captured::vacant(),
            saved_event_targets: saved_event_targets::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
            failure: None,
        }))
    }

    #[cfg(test)]
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        unit_inputs: Option<&UnitInputs>,
        clock_and_missions: (i32, &mut NavalMissionLoan),
        fresh_modifier_guard: Result<(), &'static str>,
    ) {
        self.capture_with_transport_admission(
            memory,
            base,
            game_state,
            unit_inputs,
            clock_and_missions,
            fresh_modifier_guard,
            Ok(()),
            fresh_modifier_guard,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_with_transport_admission(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        unit_inputs: Option<&UnitInputs>,
        clock_and_missions: (i32, &mut NavalMissionLoan),
        fresh_modifier_guard: Result<(), &'static str>,
        transport_admission: Result<(), &'static str>,
        transport_modifier_guard: Result<(), &'static str>,
    ) {
        let (hour, naval_missions) = clock_and_missions;
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! pack {
            ($($pack:ident),+ $(,)?) => {$(
                scatter(&$pack::TABLES, self.$pack.capture(memory, base, game_state), &mut ticks);
            )+};
        }
        pack!(combat_details, naval_details, naval_result_details);
        scatter(
            &country_appearance::TABLES,
            self.country_appearance
                .capture_at(memory, base, game_state, hour),
            &mut ticks,
        );
        pack!(naval_bases, map_facilities, naval_combat_sources);
        let started = qpc();
        let mut naval_reader = self.country_counters.begin_naval(
            memory,
            base,
            game_state,
            hour,
            self.country_appearance.naval_movement_mut(),
        );
        ticks[country_counters::TABLES[0].index()] += qpc().saturating_sub(started).max(0) as u64;
        let cost = match transport_admission {
            Ok(()) => self.naval_mission_inputs.capture_with_transport_growing(
                memory,
                base,
                game_state,
                hour,
                naval_missions.force_full,
                naval_missions.fingerprints.as_deref_mut(),
                self.country_counters.naval_holder(),
                self.country_appearance.transport_missions_mut(),
            ),
            Err(reason) => {
                let shared = self.country_appearance.transport_missions_mut();
                let required = shared.required();
                shared.reset();
                shared.retain_required(required);
                shared.retain_required(super::transport_mission_shared::Capacities {
                    navies: self
                        .country_counters
                        .naval_holder()
                        .physical_count()
                        .unwrap_or(0),
                    access_modes: self.country_counters.naval_holder().known_access_modes(),
                    ..Default::default()
                });
                shared.fail(reason);
                self.naval_mission_inputs.capture_growing(
                    memory,
                    base,
                    game_state,
                    hour,
                    naval_missions.force_full,
                    naval_missions.fingerprints.as_deref_mut(),
                )
            }
        };
        scatter(&naval_mission_inputs::TABLES, cost, &mut ticks);
        scatter(
            &naval_counters::TABLES,
            if transport_admission.is_ok() {
                self.naval_counters.capture_with_transport(
                    memory,
                    base,
                    game_state,
                    hour,
                    fresh_modifier_guard,
                    naval_counters::TransportCapture {
                        guard: transport_modifier_guard,
                        receivers: &mut self.naval_mission_inputs.transport,
                        shared: self.country_appearance.transport_missions_mut(),
                    },
                )
            } else {
                self.naval_counters.capture_with_hour(
                    memory,
                    base,
                    game_state,
                    hour,
                    fresh_modifier_guard,
                )
            },
            &mut ticks,
        );
        scatter(
            &railway_guns::TABLES,
            self.railway_guns
                .capture_with_hour(memory, base, game_state, hour),
            &mut ticks,
        );
        pack!(active_abilities);
        // Take the completed scalar handoff even after a required naming
        // refusal. It never survives Ordinary into worker conversion.
        let naval_begin_ticks = ticks[country_counters::TABLES[0].index()];
        let visibility = self.country_appearance.take_reserved_visibility();
        let prefix_ready = self.country_appearance.reserved_land_certificate_ready();
        scatter(
            &country_counters::TABLES,
            self.country_counters.capture_with_movement_and_reverse(
                memory,
                base,
                game_state,
                super::production::Sources {
                    rows: unit_inputs.map_or(&[], UnitInputs::seeds),
                    outcome: unit_inputs.map_or(MISSING, UnitInputs::outcome),
                },
                self.railway_guns.movement_sources(),
                Some(super::reserved_visibility::Request {
                    hour: Some(hour),
                    observation: visibility,
                    prefix_ready,
                    capture: self.country_appearance.reverse_visibility_mut(),
                }),
            ),
            &mut ticks,
        );
        ticks[country_counters::TABLES[0].index()] += naval_begin_ticks;
        ticks[TableId::ReservedCountryNaming.index()] = ticks
            [TableId::ReservedCountryNaming.index()]
        .saturating_add(self.country_appearance.reverse_visibility_ticks());
        pack!(game_score, group_counters, saved_event_targets);
        let owner_failure = match self.naval_counters.outcomes()[2] {
            Outcome::Rows { .. } => None,
            Outcome::Truncated { .. } => Some("task-force movement owner census truncated"),
            Outcome::Unavailable { reason } => Some(reason),
        };
        ticks[country_counters::TABLES[0].index()] += self.country_counters.finish_naval(
            memory,
            base,
            game_state,
            super::country_naval_movement_inputs::OwnerCensus {
                owners: self
                    .naval_counters
                    .fresh_owner_demands()
                    .map(|(_, owner)| owner),
                failure: owner_failure,
            },
            self.country_appearance.naval_movement_mut(),
            &naval_reader,
        );
        naval_reader.clear();
        self.naval_counters.clear_fresh_owner_demands();
        self.ticks = ticks;
    }
}

/// The original hourly naval mission pass and sliced-family store travel
/// together through Ordinary. No other job may read or publish this loan.
#[derive(Default)]
pub(crate) struct NavalMissionLoan {
    pub(crate) force_full: bool,
    pub(crate) fingerprints: Option<Box<crate::slicing::SliceFingerprints>>,
}

/// The Slow job's stability store. Absence drops only its early reads.
#[derive(Default)]
pub(crate) struct SlowStores {
    pub(crate) stability: Option<Box<crate::slicing::SliceFingerprints>>,
}

/// The raid state job's owner stores (`slicing::RAID_STATE`, `RAID_BUILDINGS`).
pub(crate) struct RaidStateStores {
    pub(crate) states: Box<crate::slicing::SliceFingerprints>,
    pub(crate) buildings: Box<crate::slicing::SliceFingerprints>,
}

impl RaidStateStores {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            states: Box::new(crate::slicing::SliceFingerprints::new()),
            buildings: Box::new(crate::slicing::SliceFingerprints::new()),
        }
    }
}

/// The live-memory packs run as a second pool job from the first burst
/// (map names, templates, visibility, faction theatres, model inputs,
/// modifier definitions, hired advisors, character inputs, logistics, the Combat
/// Log, country factories and industrial organisations). No defines are recorded:
/// the viewer reads the movement defines from the content's `NDefines`. Each
/// table keeps its own [`replay::cadence`]: the
/// template dictionary is daily, its counter inputs hourly.
pub(crate) struct SlowPacks {
    pub(crate) peace_bids: peace_bids::Captured,
    pub(crate) map_names: map_names::Captured,
    pub(crate) division_templates: division_templates::Captured,
    pub(crate) visibility: visibility::Captured,
    pub(crate) faction_theatres: faction_theatres::Captured,
    pub(crate) model_inputs: model_inputs::Captured,
    pub(crate) modifier_definitions: modifier_definitions::Captured,
    pub(crate) hired_advisors: hired_advisors::Captured,
    pub(crate) character_inputs: character_inputs::Captured,
    pub(crate) logistics: logistics::Captured,
    pub(crate) combat_log: combat_log::Captured,
    pub(crate) country_factories: country_factories::Captured,
    pub(crate) war_overview: war_overview::Captured,
    pub(crate) industrial_organisations: industrial_organisations::Captured,
    pub(crate) production_modifiers: production_modifiers::Captured,
    pub(crate) country_stability: country_stability::Captured,
    pub(crate) country_tactic_unlocks: country_tactic_unlocks::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

/// The sliced state and building-resource inputs: their own pool job of the
/// first burst, so they run beside Slow.
pub(crate) struct RaidStatePacks {
    pub(crate) raid_state_inputs: raid_state_inputs::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

/// Raid target statuses: their own pool job of the first burst, so they run
/// beside the sliced raid families.
pub(crate) struct RaidTargetPacks {
    pub(crate) raid_target_statuses: raid_target_statuses::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl RaidTargetPacks {
    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.raid_target_statuses.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            raid_target_statuses: raid_target_statuses::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        }))
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self
            .raid_target_statuses
            .capture(memory, base, game_state, hour, force_full);
        scatter(&raid_target_statuses::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

impl RaidStatePacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.raid_state_inputs
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.raid_state_inputs.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            raid_state_inputs: raid_state_inputs::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        }))
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: &mut RaidStateStores,
    ) {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self.raid_state_inputs.capture_sliced_growing(
            memory,
            base,
            game_state,
            hour,
            force_full,
            Some(raid_state_inputs::Stores {
                states: &mut stores.states,
                countries: &mut stores.buildings,
            }),
        );
        scatter(&raid_state_inputs::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

/// The sliced raid launch inputs: their own pool job of the first burst.
pub(crate) struct RaidLaunchPacks {
    pub(crate) raid_launch_inputs: raid_launch_inputs::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl RaidLaunchPacks {
    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.raid_launch_inputs.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            raid_launch_inputs: raid_launch_inputs::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        }))
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: &mut crate::slicing::SliceFingerprints,
    ) {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self.raid_launch_inputs.capture_sliced(
            memory,
            base,
            game_state,
            hour,
            force_full,
            Some(store),
        );
        scatter(&raid_launch_inputs::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

impl SlowPacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.peace_bids.fail_capture(hour, force_full, reason);
        self.map_names.fail_capture(hour, force_full, reason);
        self.division_templates
            .fail_capture(hour, force_full, reason);
        self.visibility.fail_capture(hour, force_full, reason);
        self.faction_theatres.fail_capture(hour, force_full, reason);
        self.model_inputs.fail_capture(hour, force_full, reason);
        self.modifier_definitions
            .fail_capture(hour, force_full, reason);
        self.hired_advisors.fail_capture(hour, force_full, reason);
        self.character_inputs.fail_capture(hour, force_full, reason);
        self.logistics.fail_capture(hour, force_full, reason);
        self.combat_log.fail_capture(hour, force_full, reason);
        self.country_factories
            .fail_capture(hour, force_full, reason);
        self.war_overview.fail_capture(hour, force_full, reason);
        self.industrial_organisations
            .fail_capture(hour, force_full, reason);
        self.production_modifiers
            .fail_capture(hour, force_full, reason);
        self.country_stability
            .fail_capture(hour, force_full, reason);
        self.country_tactic_unlocks
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.peace_bids.allocated_bytes()
            + self.map_names.allocated_bytes()
            + self.division_templates.allocated_bytes()
            + self.visibility.allocated_bytes()
            + self.faction_theatres.allocated_bytes()
            + self.model_inputs.allocated_bytes()
            + self.modifier_definitions.allocated_bytes()
            + self.hired_advisors.allocated_bytes()
            + self.character_inputs.allocated_bytes()
            + self.logistics.allocated_bytes()
            + self.combat_log.allocated_bytes()
            + self.country_factories.allocated_bytes()
            + self.war_overview.allocated_bytes()
            + self.industrial_organisations.allocated_bytes()
            + self.production_modifiers.allocated_bytes()
            + self.country_stability.allocated_bytes()
            + self.country_tactic_unlocks.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            peace_bids: peace_bids::Captured::vacant(),
            map_names: map_names::Captured::vacant(),
            division_templates: division_templates::Captured::vacant(),
            visibility: visibility::Captured::vacant(),
            faction_theatres: faction_theatres::Captured::vacant(),
            model_inputs: model_inputs::Captured::vacant(),
            modifier_definitions: modifier_definitions::Captured::vacant(),
            hired_advisors: hired_advisors::Captured::vacant(),
            character_inputs: character_inputs::Captured::vacant(),
            logistics: logistics::Captured::vacant(),
            combat_log: combat_log::Captured::vacant(),
            country_factories: country_factories::Captured::vacant(),
            war_overview: war_overview::Captured::vacant(),
            industrial_organisations: industrial_organisations::Captured::vacant(),
            production_modifiers: production_modifiers::Captured::vacant(),
            country_stability: country_stability::Captured::vacant(),
            country_tactic_unlocks: country_tactic_unlocks::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        }))
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: &mut SlowStores,
    ) {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &super::visibility::TABLES,
            self.visibility
                .capture_growing(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                scatter(
                    &$pack::TABLES,
                    self.$pack.capture(memory, base, game_state, hour, force_full),
                    &mut ticks,
                );
            )+};
        }
        staggered!(
            peace_bids,
            map_names,
            division_templates,
            faction_theatres,
            model_inputs,
            modifier_definitions,
            hired_advisors,
            character_inputs,
            country_tactic_unlocks,
            logistics,
            combat_log,
            country_factories,
            war_overview,
            industrial_organisations,
            production_modifiers
        );
        let cost = self.country_stability.capture(
            memory,
            base,
            game_state,
            hour,
            force_full,
            stores.stability.as_deref_mut(),
        );
        scatter(&country_stability::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

/// The typed walks seeded by the map sample (wings, order sources, HQ portraits) and the
/// division census, moved off the game thread as a second pool job dispatched
/// once the map sample has run.
pub(crate) struct LatePacks {
    pub(crate) leader_portraits: leader_portraits::Captured,
    pub(crate) leader_paths: leader_paths::Captured,
    pub(crate) air: air::Captured,
    pub(crate) naval_history: naval_history::Captured,
    pub(crate) army_icon_inputs: army_icon_inputs::Captured,
    pub(crate) divisions: Divisions,
    pub(crate) air_root: Option<Outcome>,
    /// The walked order census; the naval order sources read its roots.
    pub(crate) orders: Orders,
    /// The Naval raw job staged the order-of-battle sources itself.
    pub(crate) naval_raw: bool,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl LatePacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.leader_portraits.fail_capture(hour, force_full, reason);
        self.leader_paths.fail_capture(hour, force_full, reason);
        self.air.fail_late_capture(hour, force_full, reason);
        self.naval_history
            .fail_late_capture(hour, force_full, reason);
        self.army_icon_inputs.fail_capture(reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.leader_portraits.allocated_bytes()
            + self.leader_paths.allocated_bytes()
            + self.air.allocated_bytes()
            + self.naval_history.allocated_bytes()
            + self.army_icon_inputs.allocated_bytes()
            + self.divisions.allocated_bytes()
            + self.orders.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            leader_portraits: leader_portraits::Captured::vacant(),
            leader_paths: leader_paths::Captured::vacant(),
            air: air::Captured::vacant(),
            naval_history: naval_history::Captured::vacant(),
            army_icon_inputs: army_icon_inputs::Captured::vacant(),
            divisions: Divisions::new(0),
            air_root: None,
            orders: Orders::vacant(),
            naval_raw: false,
            ticks: [0; TABLE_COUNT],
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        inputs: Option<&UnitInputs>,
        air_scores: &mut engine::raid_air_scores::Held,
    ) {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self.leader_portraits.capture(
            memory,
            base,
            self.orders.roots(),
            self.orders.root_outcome,
        );
        scatter(&leader_portraits::TABLES, cost, &mut ticks);
        let cost =
            self.leader_paths
                .capture(memory, base, game_state, self.leader_portraits.sources());
        scatter(&leader_paths::TABLES, cost, &mut ticks);
        if !self.naval_raw {
            self.naval_history.set_order_sources(
                memory,
                base,
                self.orders.roots(),
                self.orders.root_outcome,
            );
        }
        let parent = self.air_root.unwrap_or(MISSING);
        self.air.finish_wings(parent);
        // Without the AirMissions job the history parts get no regions.
        if !self.air.missions_captured() {
            self.air.capture_missions(
                memory,
                base,
                game_state,
                air::MissionSources {
                    parent,
                    histories: &mut [],
                    changes: &crate::history_changes::Unobserved,
                    transport_regions: Err(
                        "transport original region loan unavailable to legacy air capture",
                    ),
                    transport_guard: Err(
                        "transport air modifier guard unavailable to legacy air capture",
                    ),
                    transport_admission: Err(
                        "transport original callback admission unavailable to legacy air capture",
                    ),
                },
            );
        }
        let cost = self
            .air
            .capture(memory, base, game_state, hour, force_full, air_scores);
        scatter(&air::TABLES, cost, &mut ticks);
        let cost = self
            .naval_history
            .capture(memory, base, game_state, hour, force_full);
        scatter(&naval_history::TABLES, cost, &mut ticks);
        let seeds = inputs.map(UnitInputs::seeds);
        let cost = self
            .army_icon_inputs
            .capture(memory, base, self.divisions.sources(), seeds);
        scatter(&army_icon_inputs::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

/// The packs whose typed walks wait for the raw-copy verdicts (radar, intel,
/// registrations, projects, construction rest): one pool job dispatched after
/// the mid-block joins and joined before capture_tables.
pub(crate) struct GatedPacks {
    pub(crate) fog: fog::Captured,
    pub(crate) projects: projects::Captured,
    pub(crate) construction: construction::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl GatedPacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.fog.fail_gated_capture(hour, force_full, reason);
        self.projects.fail_gated_capture(hour, force_full, reason);
        self.construction
            .fail_gated_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.fog.allocated_bytes()
            + self.projects.allocated_bytes()
            + self.construction.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if crate::allocation::failed() {
            return None;
        }
        Some(Box::new(Self {
            fog: fog::Captured::vacant(),
            projects: projects::Captured::vacant(),
            construction: construction::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        }))
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &fog::TABLES,
            self.fog
                .capture_growing(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        scatter(
            &construction::TABLES,
            self.construction
                .capture_growing(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        scatter(
            &projects::TABLES,
            self.projects
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        self.ticks = ticks;
    }
}

/// Final original borrowers, moved together after their source jobs join.
#[derive(Clone, Copy, Default)]
pub(crate) struct FinalInputsReport {
    pub snapshots: [u64; 3],
    pub production: [u64; production::N],
    pub economy: [u64; economy::N],
    pub session: [u64; session::N],
    pub characters: [u64; characters::N],
    pub destruction: [u64; super::equipment_destruction::N],
    pub naval_sources: [u64; super::naval_result_sources::N],
    pub air_losses: [u64; super::air_losses::N],
    pub bombing: [u64; super::building_bombing::N],
    pub intel_delegated: [u64; super::intel::N],
}

pub(crate) struct FinalInputsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    base: usize,
    state: usize,
    hour: i32,
    force_full: bool,
    guard: Result<(), &'static str>,
    pub provinces: Vec<engine::cprovince::Row>,
    province_outcome: Outcome,
    inputs: Option<Arc<UnitInputs>>,
    map: super::attack_inputs::Map,
    presentation: super::division_presentation::Captured,
    groups: super::group_counters::Captured,
    journal: engine::raw_country_observations::Journal,
    divisions: super::Divisions,
    new_divisions: super::new_divisions::Captured,
    experience: super::experience_gains::Captured,
    production: production::Captured,
    economy: economy::Captured,
    session: session::Captured,
    characters: characters::Captured,
    destruction: super::equipment_destruction::Captured,
    naval_sources: super::naval_result_sources::Captured,
    air_losses: super::air_losses::Captured,
    bombing: super::building_bombing::Captured,
    intel: super::intel::Captured,
    finished: [bool; 12],
}
impl FinalInputsRequest {
    pub(crate) fn allocated_bytes(&self) -> u64 {
        (self.provinces.capacity() * size_of::<engine::cprovince::Row>()
            + self.map.allocated_bytes()
            + self.presentation.allocated_bytes()
            + self.groups.allocated_bytes()
            + self.journal.allocated_bytes()
            + self.divisions.allocated_bytes()
            + self.new_divisions.allocated_bytes()
            + self.experience.allocated_bytes()
            + self.production.allocated_bytes()
            + self.economy.allocated_bytes()
            + self.session.allocated_bytes()
            + self.characters.allocated_bytes()
            + self.destruction.allocated_bytes()
            + self.naval_sources.allocated_bytes()
            + self.air_losses.allocated_bytes()
            + self.bombing.allocated_bytes()
            + self.intel.allocated_bytes()) as u64
    }

    pub(crate) fn capture(&mut self, memory: &impl Memory) -> FinalInputsReport {
        let started = qpc();
        let _reads = crate::read_accounting::table(TableId::CountryCounterInputs);
        let sources = super::unit_sources(&self.inputs);
        self.map.capture_with_observations(
            memory,
            self.base,
            self.state,
            self.hour,
            sources.rows,
            sources.outcome,
            &self.provinces,
            self.province_outcome,
            self.guard,
            &mut self.journal,
        );
        self.presentation
            .capture_group_attack(memory, self.base, self.hour, &mut self.groups);
        self.journal.finish_native();
        super::attack_inputs::retire_province_states(&mut self.provinces);
        self.presentation.retire_attack_native();
        self.finished[0] = true;
        let attack = qpc().saturating_sub(started).max(0) as u64;
        let sources = self.divisions.sources();
        let new_divisions =
            self.new_divisions
                .capture(memory, self.base, sources.rows, sources.outcome)[0];
        self.finished[1] = true;
        let experience = self.experience.capture(memory, self.base, self.state)[0];
        self.finished[2] = true;
        let production = self.production.acquire(
            memory,
            self.base,
            self.state,
            self.divisions.sources(),
            self.hour,
            self.force_full,
        );
        self.finished[3] = true;
        let economy =
            self.economy
                .capture_growing(memory, self.base, self.state, self.hour, self.force_full);
        self.finished[4] = true;
        let session =
            self.session
                .capture_growing(memory, self.base, self.state, self.hour, self.force_full);
        self.finished[5] = true;
        let characters = self.characters.capture_growing(
            memory,
            self.base,
            self.state,
            self.hour,
            self.force_full,
        );
        self.finished[6] = true;
        let destruction = self
            .destruction
            .capture_growing(memory, self.base, self.state);
        self.finished[7] = true;
        let naval_sources = self
            .naval_sources
            .capture_growing(memory, self.base, self.state);
        self.finished[8] = true;
        let air_losses = self
            .air_losses
            .capture_growing(memory, self.base, self.state);
        self.finished[9] = true;
        let bombing = self.bombing.capture_growing(memory, self.base, self.state);
        self.finished[10] = true;
        let intel_delegated = self
            .intel
            .capture_delegated(
                memory,
                super::intel::CaptureContext {
                    base: self.base,
                    game_state: self.state,
                    hour: self.hour,
                    force_full: self.force_full,
                },
            )
            .ticks;
        self.finished[11] = true;
        FinalInputsReport {
            snapshots: [attack, new_divisions, experience],
            production,
            economy,
            session,
            characters,
            destruction,
            naval_sources,
            air_losses,
            bombing,
            intel_delegated,
        }
    }
}
impl Captured {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_final_inputs_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        provinces: &mut Vec<engine::cprovince::Row>,
        province_outcome: Outcome,
    ) -> FinalInputsRequest {
        FinalInputsRequest {
            countries: Arc::clone(countries),
            base,
            state,
            hour,
            force_full,
            guard: self.attack_guard,
            provinces: std::mem::take(provinces),
            province_outcome,
            inputs: self.unit_inputs.clone(),
            map: std::mem::replace(
                &mut self.attack_map,
                super::attack_inputs::Map::new(super::attack_inputs::MapCapacities(
                    engine::selected_attack::MapCapacity {
                        countries: 0,
                        links: 0,
                        provinces: 0,
                    },
                )),
            ),
            presentation: std::mem::replace(
                &mut self.division_presentation,
                super::division_presentation::Captured::vacant(),
            ),
            groups: std::mem::replace(
                &mut self.group_counters,
                super::group_counters::Captured::vacant(),
            ),
            journal: std::mem::replace(
                &mut self.country_appearance.raw,
                engine::raw_country_observations::Journal::vacant(),
            ),
            divisions: std::mem::replace(&mut self.divisions, super::Divisions::new(0)),
            new_divisions: std::mem::replace(
                &mut self.new_divisions,
                super::new_divisions::Captured::vacant(),
            ),
            experience: std::mem::replace(
                &mut self.experience_gains,
                super::experience_gains::Captured::vacant(),
            ),
            production: std::mem::replace(&mut self.production, production::Captured::vacant()),
            economy: std::mem::replace(&mut self.economy, economy::Captured::vacant()),
            session: std::mem::replace(&mut self.session, session::Captured::vacant()),
            characters: std::mem::replace(&mut self.characters, characters::Captured::vacant()),
            destruction: std::mem::replace(
                &mut self.equipment_destruction,
                super::equipment_destruction::Captured::vacant(),
            ),
            naval_sources: std::mem::replace(
                &mut self.naval_result_sources,
                super::naval_result_sources::Captured::vacant(),
            ),
            air_losses: std::mem::replace(
                &mut self.air_losses,
                super::air_losses::Captured::vacant(),
            ),
            bombing: std::mem::replace(
                &mut self.building_bombing,
                super::building_bombing::Captured::vacant(),
            ),
            intel: std::mem::replace(&mut self.intel, super::intel::Captured::vacant()),
            finished: [false; 12],
        }
    }
    pub(crate) fn restore_final_inputs_request(
        &mut self,
        mut request: FinalInputsRequest,
        ticks: FinalInputsReport,
        failure: Option<&'static str>,
        provinces: &mut Vec<engine::cprovince::Row>,
    ) {
        if let Some(reason) = failure {
            if !request.finished[0] {
                request.map.fail(reason);
                request.groups.attack.packet.fail(reason);
                request.presentation.fail_attack_epoch(request.hour, reason);
            }
            request.presentation.retire_attack_native();
            request.journal.finish_native();
            super::attack_inputs::retire_province_states(&mut request.provinces);
            if !request.finished[1] {
                request.new_divisions.fail(reason);
            }
            if !request.finished[2] {
                request.experience.fail(reason);
            }
            if !request.finished[3] {
                request
                    .production
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[4] {
                request
                    .economy
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[5] {
                request
                    .session
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[6] {
                request
                    .characters
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[7] {
                request.destruction.fail(reason);
            }
            if !request.finished[8] {
                request.naval_sources.fail(reason);
            }
            if !request.finished[9] {
                request.air_losses.fail(reason);
            }
            if !request.finished[10] {
                request.bombing.fail(reason);
            }
            if !request.finished[11] {
                request.intel.fail_delegated_capture(
                    super::intel::CaptureContext {
                        base: request.base,
                        game_state: request.state,
                        hour: request.hour,
                        force_full: request.force_full,
                    },
                    reason,
                );
            }
        }
        self.attack_map = request.map;
        self.division_presentation = request.presentation;
        self.group_counters = request.groups;
        self.country_appearance.raw = request.journal;
        self.divisions = request.divisions;
        self.new_divisions = request.new_divisions;
        self.experience_gains = request.experience;
        self.production = request.production;
        self.economy = request.economy;
        self.session = request.session;
        self.characters = request.characters;
        self.equipment_destruction = request.destruction;
        self.naval_result_sources = request.naval_sources;
        self.air_losses = request.air_losses;
        self.building_bombing = request.bombing;
        self.intel = request.intel;
        *provinces = request.provinces;
        self.final_inputs_ticks = Some(ticks);
    }
}
