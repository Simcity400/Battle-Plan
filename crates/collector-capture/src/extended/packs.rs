use super::{
    Captured, Divisions, MISSING, army_icon_inputs, battle_sounds, combat_details, combat_log,
    construction, country_appearance, country_counters, country_factories, country_stability,
    country_tactic_unlocks, economy, faction_theatres, hired_advisors, industrial_organisations,
    leader_paths, leader_portraits, logistics, map_facilities, map_names, model_inputs,
    modifier_definitions, naval_bases, naval_combat_sources, naval_counters, naval_details,
    naval_result_details, peace_bids, production, production_modifiers, projects,
    raid_launch_inputs, raid_state_inputs, raid_target_statuses, railway_guns, resource_map,
    saved_event_targets, scatter, session, state_building_inputs, unpublished, visibility,
    war_overview,
};
use collector_air::extended::{air, fog, game_score};
use collector_characters::extended::{active_abilities, character_inputs, characters};
use collector_division_templates::extended::group_counters;
use collector_divisions::{division_census::UnitInputs, orders::Orders};
use collector_naval_history::extended::naval_history;
use collector_naval_missions::extended::naval_mission_inputs;
use engine_native::{Memory, walk::Outcome};
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use std::mem::size_of;

impl Captured {
    pub(super) fn fail_missing_slow(&mut self, hour: i32, force_full: bool) {
        let mut packs = self.slow_spare.take().expect("slow spare present");
        self.swap_slow(&mut packs);
        packs.fail(hour, force_full, "slow acquisition not supplied");
        self.swap_slow(&mut packs);
        self.slow_spare = Some(packs);
        self.slow_refusal = Some("slow acquisition not supplied");
    }
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
    pub(super) fn fail_missing_production_daily(
        &mut self,
        hour: i32,
        force_full: bool,
        reason: &'static str,
    ) {
        let mut packs = self
            .production_daily_spare
            .take()
            .expect("production daily spare present");
        self.swap_production_daily(&mut packs);
        packs.fail(hour, force_full, reason);
        self.swap_production_daily(&mut packs);
        self.production_daily_spare = Some(packs);
    }
    pub(super) fn fail_missing_war_overview(
        &mut self,
        hour: i32,
        force_full: bool,
        reason: &'static str,
    ) {
        let mut packs = self
            .war_overview_spare
            .take()
            .expect("war overview spare present");
        self.swap_war_overview(&mut packs);
        packs.fail(hour, force_full, reason);
        self.swap_war_overview(&mut packs);
        self.war_overview_spare = Some(packs);
    }
    pub(super) fn fail_missing_stability(
        &mut self,
        hour: i32,
        force_full: bool,
        reason: &'static str,
    ) {
        let mut packs = self
            .stability_spare
            .take()
            .expect("stability spare present");
        self.swap_stability(&mut packs);
        packs.fail(hour, force_full, reason);
        self.swap_stability(&mut packs);
        self.stability_spare = Some(packs);
    }
    pub(super) fn fail_missing_raid_launch(&mut self, hour: i32, force_full: bool) {
        self.raid_launch_inputs.fail_capture(
            hour,
            force_full,
            "raid launch acquisition not supplied",
        );
    }
    pub(super) fn fail_missing_gated(&mut self, hour: i32, force_full: bool) {
        let mut packs = self.gated_spare.take().expect("gated spare present");
        self.swap_gated(&mut packs);
        packs.fail(hour, force_full, "gated acquisition not supplied");
        self.swap_gated(&mut packs);
        self.gated_spare = Some(packs);
    }
    pub(super) fn fail_missing_late(&mut self, hour: i32, force_full: bool, orders: &mut Orders) {
        let mut packs = self.late_spare.take().expect("late spare present");
        self.swap_late(&mut packs, orders);
        packs.fail(hour, force_full, "late acquisition not supplied");
        self.swap_late(&mut packs, orders);
        self.late_spare = Some(packs);
    }
    pub(super) fn fail_missing_raid_targets(
        &mut self,
        hour: i32,
        force_full: bool,
        reason: &'static str,
    ) {
        self.raid_target_statuses
            .fail_capture(hour, force_full, reason);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_countries_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        rows: &mut Vec<engine_native::ccountry::Row>,
        links: &mut Vec<engine_native::ccountry::LinkRow>,
        clock: (i32, bool),
        appearance_store: Option<Box<country_appearance::AppearanceStore>>,
    ) -> crate::acquisition_jobs::CountriesRequest {
        crate::acquisition_jobs::CountriesRequest::take_with_naming(
            countries,
            base,
            state,
            rows,
            links,
            &mut self.country_appearance,
            clock.0,
            appearance_store,
        )
    }

    pub(crate) fn restore_countries_request(
        &mut self,
        request: crate::acquisition_jobs::CountriesRequest,
        rows: &mut Vec<engine_native::ccountry::Row>,
        links: &mut Vec<engine_native::ccountry::LinkRow>,
        failure: Option<&'static str>,
    ) -> Option<Box<country_appearance::AppearanceStore>> {
        request.restore_with_naming(rows, links, &mut self.country_appearance, failure)
    }

    pub(crate) fn take_templates_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        clock: (i32, bool),
    ) -> crate::acquisition_jobs::TemplatesRequest {
        crate::acquisition_jobs::TemplatesRequest::take(
            countries,
            base,
            state,
            &mut self.division_templates,
            clock.0,
            clock.1,
        )
    }

    pub(crate) fn restore_templates_request(
        &mut self,
        request: crate::acquisition_jobs::TemplatesRequest,
        failure: Option<&'static str>,
    ) {
        request.restore(&mut self.division_templates, failure);
    }

    pub(crate) fn take_ordinary_parts(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        fresh_modifier_guard: Result<(), &'static str>,
        naval_missions: NavalMissionLoan,
    ) -> [crate::acquisition_jobs::OrdinaryPartRequest; ORDINARY_PARTS] {
        self.ordinary_ticks = None;
        self.ordinary_failure = None;
        let mut naval_missions = Some(naval_missions);
        std::array::from_fn(|index| {
            let mut part = self.ordinary_part_spares[index]
                .take()
                .expect("ordinary part spare present");
            part.swap(self);
            part.begin(&mut naval_missions);
            crate::acquisition_jobs::OrdinaryPartRequest {
                countries: engine_allocation::SharedBox::clone(countries),
                base,
                game_state: state,
                hour,
                fresh_modifier_guard,
                part,
            }
        })
    }

    pub(crate) fn take_ordinary_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
    ) -> crate::acquisition_jobs::OrdinaryRequest {
        let mut packs = self.ordinary_spare.take().expect("ordinary spare present");
        self.swap_ordinary(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        packs.failure = None;
        crate::acquisition_jobs::OrdinaryRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            packs,
            counters: None,
        }
    }

    pub(crate) fn restore_ordinary_request(
        &mut self,
        request: crate::acquisition_jobs::OrdinaryRequest,
        failure: Option<&'static str>,
        parts: [(
            crate::acquisition_jobs::OrdinaryPartRequest,
            Option<&'static str>,
        ); ORDINARY_PARTS],
    ) -> NavalMissionLoan {
        let mut packs = request.packs;
        if let Some(reason) = failure {
            packs.fail(reason);
        }
        self.swap_ordinary(&mut packs);
        let mut ticks = packs.ticks;
        let mut combined = packs.failure;
        self.ordinary_spare = Some(packs);
        let mut naval_missions = None;
        for (index, (request, failure)) in parts.into_iter().enumerate() {
            let mut part = request.part;
            if let Some(reason) = failure {
                part.fail(reason);
            }
            combined = combined.or(part.failure());
            for (total, cost) in ticks.iter_mut().zip(part.ticks()) {
                *total += cost;
            }
            part.swap(self);
            if let Some(loan) = part.end() {
                naval_missions = Some(loan);
            }
            self.ordinary_part_spares[index] = Some(part);
        }
        let mut naval_missions = naval_missions.unwrap_or_default();
        if let Some(reason) = combined {
            self.fail_ordinary(request.hour, naval_missions.force_full, reason);
            ticks = [0; TABLE_COUNT];
        }
        if let Some(fingerprints) = naval_missions.fingerprints.as_mut() {
            if combined.is_some() {
                fingerprints.forget();
            } else {
                let outcomes = self.naval_mission_inputs.outcomes();
                if naval_mission_inputs::SLICED
                    .iter()
                    .any(|&index| !matches!(outcomes[index], Outcome::Rows { .. }))
                {
                    self.naval_mission_inputs
                        .selection()
                        .invalidate(fingerprints);
                }
            }
        }
        self.ordinary_ticks = Some(ticks);
        self.ordinary_failure = combined;
        naval_missions
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_ordinary_inline(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        fresh_modifier_guard: Result<(), &'static str>,
        naval_missions: NavalMissionLoan,
    ) -> NavalMissionLoan {
        let mut parts = self.take_ordinary_parts(
            countries,
            base,
            state,
            hour,
            fresh_modifier_guard,
            naval_missions,
        );
        let mut request = self.take_ordinary_request(countries, base, state, hour);
        for part in &mut parts {
            part.part
                .capture(memory, base, state, hour, part.fresh_modifier_guard);
        }
        request.packs.capture_at(memory, base, state, hour);
        let loan = self.restore_ordinary_request(request, None, parts.map(|part| (part, None)));
        self.combat_details.settle_owned();
        self.naval_bases.decode_owned();
        self.group_counters.decode_owned();
        self.naval_counters.decode_owned();
        self.naval_mission_inputs.decode_owned();
        self.railway_guns.decode_owned();
        loan
    }

    fn swap_ordinary(&mut self, packs: &mut OrdinaryPacks) {
        std::mem::swap(&mut self.country_appearance, &mut packs.country_appearance);
        std::mem::swap(&mut self.country_counters, &mut packs.country_counters);
    }

    fn fail_ordinary(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.combat_details.fail(reason);
        self.naval_details.reset();
        self.naval_result_details.reset();
        self.country_appearance.fail();
        self.naval_bases.reset();
        self.map_facilities.reset();
        self.naval_combat_sources.reset();
        self.naval_mission_inputs
            .fail_capture(hour, force_full, reason);
        self.naval_counters.reset();
        self.naval_counters.forget_censuses();
        self.railway_guns.reset();
        self.active_abilities.reset();
        self.country_counters.fail();
        self.game_score.fail(reason);
        self.group_counters.reset();
        self.saved_event_targets.reset();
        self.battle_sounds.reset();
    }

    pub(crate) fn take_slow_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
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
        packs.capture_mode = self.capture_mode;
        crate::acquisition_jobs::SlowRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            stores,
        }
    }

    pub(crate) fn restore_slow_request(
        &mut self,
        request: crate::acquisition_jobs::SlowRequest,
        completed: bool,
    ) -> SlowStores {
        let mut packs = request.packs;
        let mut stores = request.stores;
        if !completed {
            combat_log::forget_reads();
            packs.fail(request.hour, request.force_full, "slow acquisition failed");
            self.slow_refusal = Some("slow acquisition failed");
            if let Some(store) = stores.advisors.as_mut() {
                store.forget();
            }
            if let Some(store) = stores.available.as_mut() {
                store.forget();
            }
            if let Some(store) = stores.local_subunits.as_mut() {
                store.forget();
            }
            if let Some(store) = stores.doctrines.as_ref()
                && let Ok(mut store) = store.lock()
            {
                store.forget();
            }
        }
        let read = |outcome: &Outcome| {
            matches!(outcome, Outcome::Rows { .. })
                || matches!(outcome, Outcome::Unavailable { reason } if *reason == replay_data::cadence::SKIPPED_REASON)
        };
        if let Some(fingerprints) = stores.available.as_mut()
            && !read(&packs.hired_advisors.outcomes()[4])
        {
            packs
                .hired_advisors
                .available_selection()
                .invalidate(fingerprints);
        }
        self.slow_ticks = Some(packs.ticks);
        self.swap_slow(&mut packs);
        self.slow_spare = Some(packs);
        stores
    }

    pub(crate) fn take_stability_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        store: Option<Box<collector_certificates::slicing::SliceFingerprints>>,
    ) -> crate::acquisition_jobs::StabilityRequest {
        self.stability_ticks = None;
        let mut packs = self
            .stability_spare
            .take()
            .expect("stability spare present");
        self.swap_stability(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::StabilityRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            store,
        }
    }

    pub(crate) fn restore_stability_request(
        &mut self,
        request: crate::acquisition_jobs::StabilityRequest,
        completed: bool,
    ) -> Option<Box<collector_certificates::slicing::SliceFingerprints>> {
        let mut packs = request.packs;
        let mut store = request.store;
        if !completed {
            packs.fail(
                request.hour,
                request.force_full,
                "stability acquisition failed",
            );
            if let Some(store) = store.as_mut() {
                store.forget();
            }
        }
        if let Some(fingerprints) = store.as_mut()
            && packs.country_stability.outcomes().iter().any(|outcome| {
                !matches!(outcome, Outcome::Rows { .. })
                    && !matches!(outcome, Outcome::Unavailable { reason } if *reason == replay_data::cadence::SKIPPED_REASON)
            })
        {
            packs.country_stability.selection().invalidate(fingerprints);
        }
        self.stability_ticks = Some(packs.ticks);
        self.swap_stability(&mut packs);
        self.stability_spare = Some(packs);
        store
    }

    pub(crate) fn take_industrial_organisations_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::IndustrialOrganisationsRequest {
        let captured = std::mem::replace(
            &mut self.industrial_organisations,
            industrial_organisations::Captured::vacant(),
        );
        crate::acquisition_jobs::IndustrialOrganisationsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            captured,
        }
    }

    pub(crate) fn restore_industrial_organisations_request(
        &mut self,
        request: crate::acquisition_jobs::IndustrialOrganisationsRequest,
        completed: bool,
        ticks: [u64; industrial_organisations::N],
        packs: &mut ProductionDailyPacks,
    ) {
        packs.industrial_organisations = request.captured;
        packs.industrial_organisations_completed = Some(completed);
        scatter(&industrial_organisations::TABLES, ticks, &mut packs.ticks);
    }

    pub(crate) fn take_production_daily_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::ProductionDailyRequest {
        self.production_daily_ticks = None;
        let mut packs = self
            .production_daily_spare
            .take()
            .expect("production daily spare present");
        self.swap_production_daily(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        packs.industrial_organisations_completed = None;
        crate::acquisition_jobs::ProductionDailyRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

    pub(crate) fn restore_production_daily_request(
        &mut self,
        request: crate::acquisition_jobs::ProductionDailyRequest,
        completed: bool,
    ) {
        let mut packs = request.packs;
        if !completed || packs.industrial_organisations_completed == Some(false) {
            packs.fail(request.hour, request.force_full, "slow acquisition failed");
        } else if packs.industrial_organisations_completed.is_none() {
            packs.fail(
                request.hour,
                request.force_full,
                "slow acquisition not supplied",
            );
        }
        self.production_daily_ticks = Some(packs.ticks);
        self.swap_production_daily(&mut packs);
        self.production_daily_spare = Some(packs);
    }

    pub(crate) fn prepare_production_modifier_parts(
        &mut self,
        memory: &impl Memory,
        base: usize,
        starts: &[u32],
    ) -> usize {
        self.production_modifiers
            .prepare_parts(memory, base, starts)
    }

    pub(crate) fn take_production_modifier_part(
        &mut self,
        part: usize,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::ProductionModifierPartRequest {
        let (captured, slots, provided) = self.production_modifiers.take_part(part);
        crate::acquisition_jobs::ProductionModifierPartRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            part,
            slots,
            provided,
            captured,
        }
    }

    pub(crate) fn restore_production_modifier_part(
        &mut self,
        request: crate::acquisition_jobs::ProductionModifierPartRequest,
        completed: bool,
        ticks: u64,
    ) {
        self.production_modifiers.restore_part(
            request.part,
            request.captured,
            completed,
            request.hour,
            request.force_full,
        );
        if let Some(done) = &mut self.production_daily_ticks {
            done[production_modifiers::TABLES[0].index()] += ticks;
        }
    }

    pub(crate) fn production_modifier_starts(
        &self,
        previous: ([u32; production_modifiers::MAX_PARTS - 1], usize),
    ) -> ([u32; production_modifiers::MAX_PARTS - 1], usize) {
        self.production_modifiers.next_starts(previous)
    }

    fn swap_production_daily(&mut self, packs: &mut ProductionDailyPacks) {
        std::mem::swap(
            &mut self.industrial_organisations,
            &mut packs.industrial_organisations,
        );
        std::mem::swap(
            &mut self.production_modifiers,
            &mut packs.production_modifiers,
        );
    }

    pub(crate) fn take_war_overview_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::WarOverviewRequest {
        self.war_overview_ticks = None;
        let mut packs = self
            .war_overview_spare
            .take()
            .expect("war overview spare present");
        self.swap_war_overview(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::WarOverviewRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

    pub(crate) fn restore_war_overview_request(
        &mut self,
        request: crate::acquisition_jobs::WarOverviewRequest,
        completed: bool,
    ) {
        let mut packs = request.packs;
        if !completed {
            packs.fail(
                request.hour,
                request.force_full,
                "war overview acquisition failed",
            );
        }
        self.war_overview_ticks = Some(packs.ticks);
        self.swap_war_overview(&mut packs);
        self.war_overview_spare = Some(packs);
    }

    pub(crate) fn take_raid_targets_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
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
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

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

    pub(crate) fn take_raid_state_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
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
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            stores,
        }
    }

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
            stores.routes.forget();
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
        if unpublished(&self.resource_map.outcomes()[5..6]) {
            self.resource_map
                .route_selection()
                .invalidate(&mut stores.routes);
        }
        stores
    }

    pub(crate) fn take_raid_launch_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        store: Box<collector_certificates::slicing::SliceFingerprints>,
    ) -> crate::acquisition_jobs::RaidLaunchRequest {
        self.raid_launch_ticks = None;
        let mut packs = self
            .raid_launch_spare
            .take()
            .expect("raid launch spare present");
        std::mem::swap(&mut self.raid_launch_inputs, &mut packs.raid_launch_inputs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::RaidLaunchRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            store,
        }
    }

    pub(crate) fn restore_raid_launch_request(
        &mut self,
        request: crate::acquisition_jobs::RaidLaunchRequest,
        completed: bool,
    ) -> Box<collector_certificates::slicing::SliceFingerprints> {
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
        if unpublished(&launch[2..3]) {
            self.raid_launch_inputs.selection().invalidate(&mut store);
        }
        store
    }

    fn swap_raid_state(&mut self, packs: &mut RaidStatePacks) {
        std::mem::swap(&mut self.raid_state_inputs, &mut packs.raid_state_inputs);
        std::mem::swap(&mut self.resource_map, &mut packs.resource_map);
    }

    fn swap_stability(&mut self, packs: &mut StabilityPacks) {
        std::mem::swap(&mut self.country_stability, &mut packs.country_stability);
    }

    fn swap_war_overview(&mut self, packs: &mut WarOverviewPacks) {
        std::mem::swap(&mut self.war_overview, &mut packs.war_overview);
    }

    fn swap_slow(&mut self, packs: &mut SlowPacks) {
        std::mem::swap(&mut self.peace_bids, &mut packs.peace_bids);
        std::mem::swap(&mut self.map_names, &mut packs.map_names);
        std::mem::swap(&mut self.visibility, &mut packs.visibility);
        std::mem::swap(&mut self.faction_theatres, &mut packs.faction_theatres);
        std::mem::swap(&mut self.model_inputs, &mut packs.model_inputs);
        std::mem::swap(
            &mut self.modifier_definitions,
            &mut packs.modifier_definitions,
        );
        std::mem::swap(&mut self.hired_advisors, &mut packs.hired_advisors);
        std::mem::swap(&mut self.logistics, &mut packs.logistics);
        std::mem::swap(&mut self.combat_log, &mut packs.combat_log);
        std::mem::swap(&mut self.country_factories, &mut packs.country_factories);
        std::mem::swap(
            &mut self.state_building_inputs,
            &mut packs.state_building_inputs,
        );
        std::mem::swap(
            &mut self.country_tactic_unlocks,
            &mut packs.country_tactic_unlocks,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_late_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        orders: &mut Orders,
        naval_raw: bool,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::LateRequest {
        self.late_ticks = None;
        let mut packs = self.late_spare.take().expect("late spare present");
        self.swap_late(&mut packs, orders);
        packs.naval_raw = naval_raw;
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::LateRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            inputs: self.unit_inputs.clone(),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            plans: std::mem::replace(
                &mut self.plans,
                collector_plans::extended::plans::Captured::vacant(),
            ),
        }
    }

    pub(crate) fn restore_late_request(
        &mut self,
        request: crate::acquisition_jobs::LateRequest,
        completed: bool,
        orders: &mut Orders,
    ) {
        let mut plans = request.plans;
        let mut packs = request.packs;
        if !completed {
            packs.fail(request.hour, request.force_full, "late acquisition failed");
            plans.fail_unit_transports("late acquisition failed");
        }
        self.late_ticks = Some(packs.ticks);
        self.swap_late(&mut packs, orders);
        self.late_spare = Some(packs);
        self.plans = plans;
        self.install_reserved_land_certificate();
    }

    pub(crate) fn take_army_icons_request(
        &mut self,
        base: usize,
    ) -> crate::acquisition_jobs::ArmyIconsRequest {
        crate::acquisition_jobs::ArmyIconsRequest {
            base,
            inputs: self.unit_inputs.clone(),
            divisions: std::mem::replace(&mut self.divisions, super::Divisions::new(0)),
            icons: std::mem::replace(
                &mut self.army_icon_inputs,
                army_icon_inputs::Captured::vacant(),
            ),
            ticks: [0],
        }
    }

    pub(crate) fn restore_army_icons_request(
        &mut self,
        request: crate::acquisition_jobs::ArmyIconsRequest,
        completed: bool,
    ) {
        let mut icons = request.icons;
        let mut ticks = request.ticks[0];
        if !completed {
            icons.fail_capture("army icon acquisition failed");
            ticks = 0;
        }
        icons.set_early(ticks);
        self.divisions = request.divisions;
        self.army_icon_inputs = icons;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_scores_rails_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        air_scores: Box<engine_raid_inputs::raid_air_scores::Held>,
        rail_store: Option<Box<naval_history::RailStore>>,
    ) -> crate::acquisition_jobs::ScoresRailsRequest {
        crate::acquisition_jobs::ScoresRailsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            score_capacity: self.air.score_capacity(),
            parts: self
                .scores_rails_spare
                .take()
                .expect("scores and rails spare present"),
            air_scores,
            rail_store,
        }
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn take_air_supply_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
    ) -> crate::acquisition_jobs::AirSupplyRequest {
        crate::acquisition_jobs::AirSupplyRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            capture: self.air_supply_spare.take().unwrap_or_default(),
            ok: false,
        }
    }

    pub(crate) fn restore_air_supply_request(
        &mut self,
        request: crate::acquisition_jobs::AirSupplyRequest,
        completed: bool,
    ) {
        let mut capture = request.capture;
        self.air
            .install_supply(&mut capture, completed && request.ok);
        self.air_supply_spare = Some(capture);
    }

    pub(crate) fn restore_scores_rails_request(
        &mut self,
        request: crate::acquisition_jobs::ScoresRailsRequest,
        completed: bool,
    ) -> (
        Box<engine_raid_inputs::raid_air_scores::Held>,
        Option<Box<naval_history::RailStore>>,
    ) {
        let mut parts = request.parts;
        let mut air_scores = request.air_scores;
        let mut rail_store = request.rail_store;
        if !completed {
            let reason = "scores and rails acquisition failed";
            parts.scores.fail(request.hour, request.force_full, reason);
            parts.rails.fail(request.hour, request.force_full, reason);
            air_scores.invalidate();
            if let Some(store) = rail_store.as_deref_mut() {
                store.forget();
            }
        }
        self.air.install_scores(&mut parts.scores);
        self.naval_history.install_rails(&mut parts.rails);
        self.scores_rails_spare = Some(parts);
        (air_scores, rail_store)
    }

    pub(crate) fn install_order_transport(&mut self) {
        self.plans
            .install_order_transport(&mut self.division_presentation);
    }

    pub(crate) fn take_gated_request(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        project_fingerprints: Box<collector_certificates::slicing::SliceFingerprints>,
        countries: Option<engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>>,
    ) -> crate::acquisition_jobs::GatedRequest {
        self.gated_ticks = None;
        let mut packs = self.gated_spare.take().expect("gated spare present");
        self.swap_gated(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        packs.fog.stationed = collector_extended_support::outcomes::records_any(
            self.capture_mode,
            &[TableId::TheatreAreas, TableId::StationedUnits],
        );
        packs.projects_recorded =
            collector_extended_support::outcomes::records_any(self.capture_mode, &projects::TABLES);
        crate::acquisition_jobs::GatedRequest {
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            project_fingerprints: Some(project_fingerprints),
            countries,
        }
    }

    pub(crate) fn restore_gated_request(
        &mut self,
        request: crate::acquisition_jobs::GatedRequest,
        completed: bool,
    ) -> Box<collector_certificates::slicing::SliceFingerprints> {
        let mut packs = request.packs;
        let mut fingerprints = request.project_fingerprints.expect("projects job restored");
        if !completed {
            packs.fail(request.hour, request.force_full, "gated acquisition failed");
            fingerprints.forget();
        }
        let outcomes = packs.projects.outcomes();
        if projects::FAMILY
            .iter()
            .any(|&index| !matches!(outcomes[index], Outcome::Rows { .. }))
        {
            packs
                .projects
                .project_selection()
                .invalidate(&mut fingerprints);
        }
        self.gated_ticks = Some(packs.ticks);
        self.swap_gated(&mut packs);
        self.gated_spare = Some(packs);
        fingerprints
    }

    fn swap_gated(&mut self, packs: &mut GatedPacks) {
        std::mem::swap(&mut self.fog, &mut packs.fog);
        std::mem::swap(&mut self.projects, &mut packs.projects);
        std::mem::swap(&mut self.construction, &mut packs.construction);
    }

    pub(crate) fn take_character_inputs_request(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        roles: CharacterInputRoles,
    ) -> CharacterInputsRequest {
        self.character_inputs_ticks = None;
        CharacterInputsRequest {
            base,
            game_state: state,
            hour,
            force_full,
            captured: std::mem::replace(
                &mut self.character_inputs,
                character_inputs::Captured::vacant(),
            ),
            roles,
        }
    }

    pub(crate) fn restore_character_inputs_request(
        &mut self,
        request: CharacterInputsRequest,
        ticks: Option<[u64; character_inputs::N]>,
    ) -> Option<Box<character_inputs::RoleStore>> {
        let CharacterInputsRequest {
            hour,
            force_full,
            mut captured,
            roles: CharacterInputRoles { census, mut store },
            ..
        } = request;
        drop(census);
        if ticks.is_none() {
            captured.fail_capture(hour, force_full, "character inputs acquisition failed");
            if let Some(store) = store.as_mut() {
                store.forget();
            }
        }
        self.character_inputs = captured;
        self.character_inputs_ticks = Some(ticks.unwrap_or_default());
        store
    }

    pub(super) fn swap_late(&mut self, packs: &mut LatePacks, orders: &mut Orders) {
        std::mem::swap(&mut self.leader_portraits, &mut packs.leader_portraits);
        std::mem::swap(&mut self.leader_paths, &mut packs.leader_paths);
        std::mem::swap(&mut self.air, &mut packs.air);
        std::mem::swap(&mut self.naval_history, &mut packs.naval_history);
        std::mem::swap(&mut self.army_icon_inputs, &mut packs.army_icon_inputs);
        std::mem::swap(orders, &mut packs.orders);
    }
}

pub(super) const ORDINARY_TABLES: &[&[TableId]] = &[
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
    &battle_sounds::TABLES,
];

pub(crate) struct OrdinaryPacks {
    pub country_appearance: country_appearance::Captured,
    pub country_counters: country_counters::Captured,
    pub ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

impl OrdinaryPacks {
    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.country_appearance.allocated_bytes()
            + self.country_counters.allocated_bytes()
    }

    pub fn fail(&mut self, reason: &'static str) {
        self.ticks = [0; TABLE_COUNT];
        self.failure.get_or_insert(reason);
    }
    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            country_appearance: country_appearance::Captured::vacant(),
            country_counters: country_counters::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
            failure: None,
        })
        .ok()
    }

    #[cfg(test)]
    pub(crate) fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
    ) {
        self.capture_at_carried(memory, base, game_state, hour, None);
    }

    pub(crate) fn capture_at_carried(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        counters: Option<&mut country_counters::CounterStore>,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &country_appearance::TABLES,
            self.country_appearance
                .capture_at(memory, base, game_state, hour),
            &mut ticks,
        );
        let visibility = self.country_appearance.take_reserved_visibility();
        let prefix_ready = self.country_appearance.reserved_land_certificate_ready();
        scatter(
            &country_counters::TABLES,
            self.country_counters.capture_carried(
                memory,
                base,
                game_state,
                Some(super::reserved_visibility::Request {
                    hour: Some(hour),
                    observation: visibility,
                    prefix_ready,
                    capture: self.country_appearance.reverse_visibility_mut(),
                }),
                counters,
            ),
            &mut ticks,
        );
        ticks[TableId::ReservedCountryNaming.index()] = ticks
            [TableId::ReservedCountryNaming.index()]
        .saturating_add(self.country_appearance.reverse_visibility_ticks());
        self.ticks = ticks;
    }
}

pub(crate) const ORDINARY_PARTS: usize = 5;

pub(crate) enum OrdinaryPart {
    Combat(Box<OrdinaryCombatPacks>),
    Naval(Box<OrdinaryNavalPacks>),
    Map(Box<OrdinaryMapPacks>),
    TaskForces(Box<OrdinaryTaskForcePacks>),
    Navies(Box<OrdinaryNavyPacks>),
}

pub(crate) struct OrdinaryCombatPacks {
    pub combat_details: combat_details::Captured,
    ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

pub(crate) struct OrdinaryNavalPacks {
    pub naval_details: naval_details::Captured,
    pub(crate) naval_result_details: naval_result_details::Captured,
    pub naval_combat_sources: naval_combat_sources::Captured,
    ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

pub(crate) struct OrdinaryMapPacks {
    pub(crate) naval_bases: naval_bases::Captured,
    pub(crate) map_facilities: map_facilities::Captured,
    pub railway_guns: railway_guns::Captured,
    pub(crate) active_abilities: active_abilities::Captured,
    pub game_score: game_score::Captured,
    pub group_counters: group_counters::Captured,
    pub saved_event_targets: saved_event_targets::Captured,
    pub(crate) battle_sounds: battle_sounds::Captured,
    ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

pub(crate) struct OrdinaryTaskForcePacks {
    pub naval_counters: naval_counters::Captured,
    ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

pub(crate) struct OrdinaryNavyPacks {
    pub naval_mission_inputs: naval_mission_inputs::Captured,
    navies: Option<country_counters::Navies>,
    launch: super::country_naval_movement_inputs::Launch,
    naval_missions: Option<NavalMissionLoan>,
    ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

impl OrdinaryPart {
    pub(super) fn spares() -> [Option<Self>; ORDINARY_PARTS] {
        if collector_memory::terminal::failed() {
            return [const { None }; ORDINARY_PARTS];
        }
        [
            engine_allocation::boxed(OrdinaryCombatPacks {
                combat_details: combat_details::Captured::vacant(),
                ticks: [0; TABLE_COUNT],
                failure: None,
            })
            .ok()
            .map(Self::Combat),
            engine_allocation::boxed(OrdinaryNavalPacks {
                naval_details: naval_details::Captured::vacant(),
                naval_result_details: naval_result_details::Captured::vacant(),
                naval_combat_sources: naval_combat_sources::Captured::vacant(),
                ticks: [0; TABLE_COUNT],
                failure: None,
            })
            .ok()
            .map(Self::Naval),
            engine_allocation::boxed(OrdinaryMapPacks {
                naval_bases: naval_bases::Captured::vacant(),
                map_facilities: map_facilities::Captured::vacant(),
                railway_guns: railway_guns::Captured::vacant(),
                active_abilities: active_abilities::Captured::vacant(),
                game_score: game_score::Captured::vacant(),
                group_counters: group_counters::Captured::vacant(),
                saved_event_targets: saved_event_targets::Captured::vacant(),
                battle_sounds: battle_sounds::Captured::vacant(),
                ticks: [0; TABLE_COUNT],
                failure: None,
            })
            .ok()
            .map(Self::Map),
            engine_allocation::boxed(OrdinaryTaskForcePacks {
                naval_counters: naval_counters::Captured::vacant(),
                ticks: [0; TABLE_COUNT],
                failure: None,
            })
            .ok()
            .map(Self::TaskForces),
            engine_allocation::boxed(OrdinaryNavyPacks {
                naval_mission_inputs: naval_mission_inputs::Captured::vacant(),
                navies: None,
                launch: super::country_naval_movement_inputs::Launch::new(Default::default()),
                naval_missions: None,
                ticks: [0; TABLE_COUNT],
                failure: None,
            })
            .ok()
            .map(Self::Navies),
        ]
    }

    #[cfg(test)]
    pub(super) fn spare_box_bytes() -> usize {
        size_of::<OrdinaryCombatPacks>()
            + size_of::<OrdinaryNavalPacks>()
            + size_of::<OrdinaryMapPacks>()
            + size_of::<OrdinaryTaskForcePacks>()
            + size_of::<OrdinaryNavyPacks>()
    }

    pub fn allocated_bytes(&self) -> usize {
        match self {
            Self::Combat(packs) => {
                size_of::<OrdinaryCombatPacks>() + packs.combat_details.allocated_bytes()
            }
            Self::Naval(packs) => {
                size_of::<OrdinaryNavalPacks>()
                    + packs.naval_details.allocated_bytes()
                    + packs.naval_result_details.allocated_bytes()
                    + packs.naval_combat_sources.allocated_bytes()
            }
            Self::Map(packs) => {
                size_of::<OrdinaryMapPacks>()
                    + packs.naval_bases.allocated_bytes()
                    + packs.map_facilities.allocated_bytes()
                    + packs.railway_guns.allocated_bytes()
                    + packs.active_abilities.allocated_bytes()
                    + packs.game_score.allocated_bytes()
                    + packs.group_counters.allocated_bytes()
                    + packs.saved_event_targets.allocated_bytes()
                    + packs.battle_sounds.allocated_bytes()
            }
            Self::TaskForces(packs) => {
                size_of::<OrdinaryTaskForcePacks>() + packs.naval_counters.allocated_bytes()
            }
            Self::Navies(packs) => {
                size_of::<OrdinaryNavyPacks>()
                    + packs.naval_mission_inputs.allocated_bytes()
                    + packs
                        .navies
                        .as_ref()
                        .map_or(0, country_counters::Navies::allocated_bytes)
                    + packs.launch.allocated_bytes()
            }
        }
    }

    fn swap(&mut self, captured: &mut Captured) {
        match self {
            Self::Combat(packs) => {
                std::mem::swap(&mut captured.combat_details, &mut packs.combat_details);
            }
            Self::Naval(packs) => {
                std::mem::swap(&mut captured.naval_details, &mut packs.naval_details);
                std::mem::swap(
                    &mut captured.naval_result_details,
                    &mut packs.naval_result_details,
                );
                std::mem::swap(
                    &mut captured.naval_combat_sources,
                    &mut packs.naval_combat_sources,
                );
            }
            Self::Map(packs) => {
                std::mem::swap(&mut captured.naval_bases, &mut packs.naval_bases);
                std::mem::swap(&mut captured.map_facilities, &mut packs.map_facilities);
                std::mem::swap(&mut captured.railway_guns, &mut packs.railway_guns);
                std::mem::swap(&mut captured.active_abilities, &mut packs.active_abilities);
                captured.game_score.swap_country(&mut packs.game_score);
                std::mem::swap(&mut captured.group_counters, &mut packs.group_counters);
                std::mem::swap(
                    &mut captured.saved_event_targets,
                    &mut packs.saved_event_targets,
                );
                std::mem::swap(&mut captured.battle_sounds, &mut packs.battle_sounds);
            }
            Self::TaskForces(packs) => {
                std::mem::swap(&mut captured.naval_counters, &mut packs.naval_counters);
            }
            Self::Navies(packs) => {
                std::mem::swap(
                    &mut captured.naval_mission_inputs,
                    &mut packs.naval_mission_inputs,
                );
                match packs.navies.take() {
                    None => packs.navies = Some(captured.country_counters.take_navies()),
                    Some(navies) => {
                        captured.country_counters.restore_navies(navies);
                        std::mem::swap(
                            captured.country_appearance.launch_sources_mut(),
                            &mut packs.launch,
                        );
                    }
                }
            }
        }
    }

    fn begin(&mut self, naval_missions: &mut Option<NavalMissionLoan>) {
        let (ticks, failure) = self.state();
        *ticks = [0; TABLE_COUNT];
        *failure = None;
        if let Self::Navies(packs) = self {
            packs.naval_missions = naval_missions.take();
        }
    }

    fn end(&mut self) -> Option<NavalMissionLoan> {
        match self {
            Self::Navies(packs) => packs.naval_missions.take(),
            _ => None,
        }
    }

    fn state(&mut self) -> (&mut [u64; TABLE_COUNT], &mut Option<&'static str>) {
        match self {
            Self::Combat(packs) => (&mut packs.ticks, &mut packs.failure),
            Self::Naval(packs) => (&mut packs.ticks, &mut packs.failure),
            Self::Map(packs) => (&mut packs.ticks, &mut packs.failure),
            Self::TaskForces(packs) => (&mut packs.ticks, &mut packs.failure),
            Self::Navies(packs) => (&mut packs.ticks, &mut packs.failure),
        }
    }

    fn ticks(&mut self) -> [u64; TABLE_COUNT] {
        *self.state().0
    }

    fn failure(&mut self) -> Option<&'static str> {
        *self.state().1
    }

    pub fn fail(&mut self, reason: &'static str) {
        let (ticks, failure) = self.state();
        *ticks = [0; TABLE_COUNT];
        failure.get_or_insert(reason);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        fresh_modifier_guard: Result<(), &'static str>,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! pack {
            ($packs:ident: $($pack:ident),+ $(,)?) => {$(
                scatter(&$pack::TABLES, $packs.$pack.capture(memory, base, game_state), &mut ticks);
            )+};
        }
        match self {
            Self::Combat(packs) => {
                scatter(
                    &combat_details::TABLES,
                    packs
                        .combat_details
                        .capture_deferred(memory, base, game_state),
                    &mut ticks,
                );
            }
            Self::Naval(packs) => {
                pack!(packs: naval_details, naval_result_details, naval_combat_sources);
            }
            Self::Map(packs) => {
                scatter(
                    &naval_bases::TABLES,
                    // SAFETY: Admitted naval-base capture reads the live image and settles or
                    // discards its owned management loan before frame reuse or detach.
                    unsafe {
                        packs.naval_bases.acquire(
                            memory,
                            base,
                            game_state,
                            collector_workers::panic_failure::owned_failed,
                        )
                    },
                    &mut ticks,
                );
                pack!(packs: map_facilities);
                scatter(
                    &railway_guns::TABLES,
                    packs
                        .railway_guns
                        .acquire_with_hour(memory, base, game_state, hour),
                    &mut ticks,
                );
                pack!(packs: active_abilities, game_score);
                scatter(
                    &group_counters::TABLES,
                    // SAFETY: Admitted hourly capture records HQ-group demand for the active session.
                    unsafe { packs.group_counters.acquire(memory, base, game_state) },
                    &mut ticks,
                );
                pack!(packs: saved_event_targets, battle_sounds);
            }
            Self::TaskForces(packs) => {
                scatter(
                    &naval_counters::TABLES,
                    // SAFETY: Admitted naval capture runs under the active session with live watches.
                    unsafe {
                        packs.naval_counters.acquire_with_hour(
                            memory,
                            base,
                            game_state,
                            hour,
                            fresh_modifier_guard,
                            naval_counters::Holds {
                                world: collector_lifetime::world_lifetime::token().filter(|_| {
                                    collector_observers::world_input_changes::map_content_stable()
                                }),
                                fleets: |fleet| {
                                    use collector_ship_models::ship_model_changes::Changes;
                                    collector_ship_models::ship_model_changes::Observer
                                        .sequence(fleet)
                                },
                            },
                        )
                    },
                    &mut ticks,
                );
                packs.naval_counters.clear_fresh_owner_demands();
            }
            Self::Navies(packs) => {
                let navies = packs
                    .navies
                    .as_mut()
                    .expect("the navy part holds the physical navy pass");
                let naval_missions = packs.naval_missions.get_or_insert_with(Default::default);
                let mut naval_reader =
                    navies.begin(memory, base, game_state, hour, &mut packs.launch);
                scatter(
                    &naval_mission_inputs::TABLES,
                    packs.naval_mission_inputs.acquire_with_navies(
                        memory,
                        base,
                        game_state,
                        hour,
                        naval_missions.force_full,
                        naval_missions.fingerprints.as_deref_mut(),
                        navies.holder(),
                        collector_missions::navy_ledger_changes::Skip::current(),
                    ),
                    &mut ticks,
                );
                collector_missions::navy_ledger_changes::record(
                    packs.naval_mission_inputs.ledger_counts(),
                );
                naval_reader.clear();
                navies.finish();
                ticks[country_counters::TABLES[0].index()] += navies.ticks();
            }
        }
        *self.state().0 = ticks;
    }
}

#[derive(Default)]
pub(crate) struct NavalMissionLoan {
    pub force_full: bool,
    pub(crate) fingerprints: Option<Box<collector_certificates::slicing::SliceFingerprints>>,
}

#[derive(Default)]
pub(crate) struct SlowStores {
    pub(crate) doctrine_bytes: Option<u64>,
    pub doctrines: Option<
        engine_allocation::SharedBox<std::sync::Mutex<collector_slow_tables::doctrines::Store>>,
    >,
    pub available: Option<Box<collector_certificates::slicing::SliceFingerprints>>,
    pub advisors: Option<Box<hired_advisors::AdvisorStore>>,
    pub(crate) local_subunits: Option<Box<model_inputs::LocalStore>>,
}

pub(crate) struct RaidStateStores {
    pub states: Box<collector_certificates::slicing::SliceFingerprints>,
    pub buildings: Box<collector_certificates::slicing::SliceFingerprints>,
    pub routes: Box<collector_certificates::slicing::SliceFingerprints>,
}

impl RaidStateStores {
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            states: Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            buildings: Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            routes: Box::new(collector_certificates::slicing::SliceFingerprints::new()),
        }
    }
}

pub(crate) struct SlowPacks {
    pub peace_bids: peace_bids::Captured,
    pub map_names: map_names::Captured,
    pub visibility: visibility::Captured,
    pub(crate) faction_theatres: faction_theatres::Captured,
    pub model_inputs: model_inputs::Captured,
    pub(crate) modifier_definitions: modifier_definitions::Captured,
    pub hired_advisors: hired_advisors::Captured,
    pub logistics: logistics::Captured,
    pub combat_log: combat_log::Captured,
    pub(crate) country_factories: country_factories::Captured,
    pub(crate) state_building_inputs: state_building_inputs::Captured,
    pub(crate) country_tactic_unlocks: country_tactic_unlocks::Captured,
    pub capture_mode: replay_data::scope::CaptureMode,
    pub ticks: [u64; TABLE_COUNT],
}

pub(crate) struct StabilityPacks {
    pub country_stability: country_stability::Captured,
    pub ticks: [u64; TABLE_COUNT],
}

impl StabilityPacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.country_stability
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.country_stability.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            country_stability: country_stability::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: Option<&mut collector_certificates::slicing::SliceFingerprints>,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self
            .country_stability
            .capture(memory, base, game_state, hour, force_full, store);
        scatter(&country_stability::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

pub(crate) struct WarOverviewPacks {
    pub war_overview: war_overview::Captured,
    pub ticks: [u64; TABLE_COUNT],
}

impl WarOverviewPacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.war_overview.fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.war_overview.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            war_overview: war_overview::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &war_overview::TABLES,
            self.war_overview
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        self.ticks = ticks;
    }
}

pub(crate) struct ProductionDailyPacks {
    pub industrial_organisations: industrial_organisations::Captured,
    industrial_organisations_completed: Option<bool>,
    pub(crate) production_modifiers: production_modifiers::Captured,
    pub ticks: [u64; TABLE_COUNT],
}

impl ProductionDailyPacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.industrial_organisations
            .fail_capture(hour, force_full, reason);
        self.production_modifiers
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.industrial_organisations.allocated_bytes()
            + self.production_modifiers.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            industrial_organisations: industrial_organisations::Captured::vacant(),
            industrial_organisations_completed: None,
            production_modifiers: production_modifiers::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn due(hour: i32, force_full: bool) -> bool {
        replay_data::cadence::should_walk(industrial_organisations::TABLES[0], hour, force_full)
            || replay_data::cadence::should_walk(production_modifiers::TABLES[0], hour, force_full)
    }

    pub fn recorded(mode: replay_data::scope::CaptureMode) -> bool {
        collector_extended_support::outcomes::records_any(mode, &industrial_organisations::TABLES)
            || collector_extended_support::outcomes::records_any(
                mode,
                &production_modifiers::TABLES,
            )
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &production_modifiers::TABLES,
            self.production_modifiers
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        self.ticks = ticks;
    }
}

pub(crate) struct RaidStatePacks {
    pub(crate) raid_state_inputs: raid_state_inputs::Captured,
    pub(crate) resource_map: resource_map::Captured,
    pub ticks: [u64; TABLE_COUNT],
}

pub(crate) struct RaidTargetPacks {
    pub(crate) raid_target_statuses: raid_target_statuses::Captured,
    pub ticks: [u64; TABLE_COUNT],
}

impl RaidTargetPacks {
    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.raid_target_statuses.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            raid_target_statuses: raid_target_statuses::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
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
        self.resource_map.fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.raid_state_inputs.allocated_bytes()
            + self.resource_map.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            raid_state_inputs: raid_state_inputs::Captured::vacant(),
            resource_map: resource_map::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: &mut RaidStateStores,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self.raid_state_inputs.capture_sliced(
            memory,
            base,
            game_state,
            hour,
            force_full,
            Some(raid_state_inputs::Stores {
                states: &mut stores.states,
                countries: &mut stores.buildings,
                changes: &collector_observers::raid_changes::Observer,
            }),
        );
        scatter(&raid_state_inputs::TABLES, cost, &mut ticks);
        let cost = self.resource_map.capture_deferred(
            memory,
            base,
            game_state,
            hour,
            force_full,
            Some(resource_map::Stores {
                routes: &mut stores.routes,
                changes: &collector_observers::raid_changes::Observer,
            }),
        );
        scatter(&resource_map::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

pub(crate) struct RaidLaunchPacks {
    pub(crate) raid_launch_inputs: raid_launch_inputs::Captured,
    pub ticks: [u64; TABLE_COUNT],
}

impl RaidLaunchPacks {
    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.raid_launch_inputs.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            raid_launch_inputs: raid_launch_inputs::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: &mut collector_certificates::slicing::SliceFingerprints,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        let cost = self.raid_launch_inputs.capture_sliced(
            memory,
            base,
            game_state,
            hour,
            force_full,
            Some(raid_launch_inputs::Owners {
                store,
                changes: &collector_observers::raid_changes::Observer,
            }),
        );
        scatter(&raid_launch_inputs::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

impl SlowPacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.peace_bids.fail_capture(hour, force_full, reason);
        self.map_names.fail_capture(hour, force_full, reason);
        self.visibility.fail_capture(hour, force_full, reason);
        self.faction_theatres.fail_capture(hour, force_full, reason);
        self.model_inputs.fail_capture(hour, force_full, reason);
        self.modifier_definitions
            .fail_capture(hour, force_full, reason);
        self.hired_advisors.fail_capture(hour, force_full, reason);
        self.logistics.fail_capture(hour, force_full, reason);
        self.combat_log.fail_capture(hour, force_full, reason);
        self.country_factories
            .fail_capture(hour, force_full, reason);
        self.state_building_inputs
            .fail_capture(hour, force_full, reason);
        self.country_tactic_unlocks
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.peace_bids.allocated_bytes()
            + self.map_names.allocated_bytes()
            + self.visibility.allocated_bytes()
            + self.faction_theatres.allocated_bytes()
            + self.model_inputs.allocated_bytes()
            + self.modifier_definitions.allocated_bytes()
            + self.hired_advisors.allocated_bytes()
            + self.logistics.allocated_bytes()
            + self.combat_log.allocated_bytes()
            + self.country_factories.allocated_bytes()
            + self.state_building_inputs.allocated_bytes()
            + self.country_tactic_unlocks.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            peace_bids: peace_bids::Captured::vacant(),
            map_names: map_names::Captured::vacant(),
            visibility: visibility::Captured::vacant(),
            faction_theatres: faction_theatres::Captured::vacant(),
            model_inputs: model_inputs::Captured::vacant(),
            modifier_definitions: modifier_definitions::Captured::vacant(),
            hired_advisors: hired_advisors::Captured::vacant(),
            logistics: logistics::Captured::vacant(),
            combat_log: combat_log::Captured::vacant(),
            country_factories: country_factories::Captured::vacant(),
            state_building_inputs: state_building_inputs::Captured::vacant(),
            country_tactic_unlocks: country_tactic_unlocks::Captured::vacant(),
            capture_mode: replay_data::scope::CaptureMode::Full,
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: &mut SlowStores,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &super::visibility::TABLES,
            self.visibility
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        let recorded = |tables: &[TableId]| {
            collector_extended_support::outcomes::records_any(self.capture_mode, tables)
        };
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                if recorded(&$pack::TABLES) {
                    scatter(
                        &$pack::TABLES,
                        self.$pack.capture(memory, base, game_state, hour, force_full),
                        &mut ticks,
                    );
                } else {
                    self.$pack.fail_capture(hour, force_full, collector_extended_support::outcomes::NOT_RECORDED);
                }
            )+};
        }
        staggered!(
            peace_bids,
            map_names,
            faction_theatres,
            modifier_definitions,
            logistics
        );
        scatter(
            &combat_log::TABLES,
            // SAFETY: Admitted hourly capture owns this hour's combat-log lane while attached.
            unsafe {
                self.combat_log
                    .capture(memory, base, game_state, hour, force_full)
            },
            &mut ticks,
        );
        staggered!(country_factories, state_building_inputs);
        scatter(
            &country_tactic_unlocks::TABLES,
            self.country_tactic_unlocks.capture_carried(
                memory,
                base,
                game_state,
                hour,
                force_full,
                stores
                    .doctrines
                    .as_deref()
                    .filter(|_| recorded(&collector_slow_tables::doctrines::TABLES)),
            ),
            &mut ticks,
        );
        stores.doctrine_bytes = self
            .country_tactic_unlocks
            .doctrine_store_bytes()
            .map(|bytes| bytes as u64);
        scatter(
            &model_inputs::TABLES,
            self.model_inputs.capture_carried(
                memory,
                base,
                game_state,
                hour,
                force_full,
                stores.local_subunits.as_deref_mut(),
            ),
            &mut ticks,
        );
        if recorded(&hired_advisors::TABLES) {
            scatter(
                &hired_advisors::TABLES,
                // SAFETY: Admitted hourly capture updates advisor-pool demand for the active session.
                unsafe {
                    self.hired_advisors.capture_carried(
                        memory,
                        base,
                        game_state,
                        hour,
                        force_full,
                        stores.advisors.as_deref_mut(),
                        stores.available.as_deref_mut(),
                    )
                },
                &mut ticks,
            );
        } else {
            self.hired_advisors.fail_capture(
                hour,
                force_full,
                collector_extended_support::outcomes::NOT_RECORDED,
            );
        }
        self.ticks = ticks;
    }
}

pub(crate) struct ScoresRailsParts {
    pub scores: air::ScorePart,
    pub(crate) rails: naval_history::RailPart,
}

impl ScoresRailsParts {
    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            scores: air::ScorePart::vacant(),
            rails: naval_history::RailPart::vacant(),
        })
        .ok()
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.scores.allocated_bytes() + self.rails.allocated_bytes()
    }
}

pub(crate) struct LatePacks {
    pub leader_portraits: leader_portraits::Captured,
    pub leader_paths: leader_paths::Captured,
    pub air: air::Captured,
    pub naval_history: naval_history::Captured,
    pub(crate) army_icon_inputs: army_icon_inputs::Captured,
    pub orders: Orders,
    pub(crate) naval_raw: bool,
    pub ticks: [u64; TABLE_COUNT],
}

impl LatePacks {
    pub(super) fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.leader_portraits.fail_capture(hour, force_full, reason);
        self.leader_paths.fail_capture(hour, force_full, reason);
        self.air.fail_late_capture(hour, force_full, reason);
        self.naval_history
            .fail_late_capture(hour, force_full, reason);
        self.army_icon_inputs.fail_late(reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.leader_portraits.allocated_bytes()
            + self.leader_paths.allocated_bytes()
            + self.air.allocated_bytes()
            + self.naval_history.allocated_bytes()
            + self.army_icon_inputs.allocated_bytes()
            + self.orders.allocated_bytes()
    }

    pub(super) fn spare() -> Option<Box<Self>> {
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            leader_portraits: leader_portraits::Captured::vacant(),
            leader_paths: leader_paths::Captured::vacant(),
            air: air::Captured::vacant(),
            naval_history: naval_history::Captured::vacant(),
            army_icon_inputs: army_icon_inputs::Captured::vacant(),
            orders: Orders::vacant(),
            naval_raw: false,
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        inputs: Option<&UnitInputs>,
        divisions: Option<&Divisions>,
        air_scores: Option<&mut engine_raid_inputs::raid_air_scores::Held>,
        rails: Option<&mut naval_history::RailStore>,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
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
        let parent = MISSING;
        self.air.finish_wings(parent);
        if !self.air.missions_captured() {
            // SAFETY: Admitted hourly capture runs without carried mission watches here.
            unsafe {
                self.air.capture_missions(
                    memory,
                    base,
                    game_state,
                    air::MissionSources {
                        parent,
                        histories: &mut [],
                        changes: &collector_air_histories::history_changes::Unobserved,
                        carry: None,
                    },
                );
            }
        }
        let cost = self
            .air
            .capture(memory, base, game_state, hour, force_full, air_scores);
        scatter(&air::TABLES, cost, &mut ticks);
        let cost = self
            .naval_history
            .capture(memory, base, game_state, hour, force_full, rails);
        scatter(&naval_history::TABLES, cost, &mut ticks);
        let cost = match self.army_icon_inputs.take_early() {
            Some(ticks) => [ticks],
            None => match divisions {
                Some(divisions) => {
                    let seeds = inputs.map(UnitInputs::seeds);
                    self.army_icon_inputs
                        .capture(memory, base, divisions.sources(), seeds)
                }
                None => {
                    self.army_icon_inputs
                        .fail_capture("army icon acquisition not supplied");
                    [0]
                }
            },
        };
        scatter(&army_icon_inputs::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

pub(crate) struct GatedPacks {
    pub fog: fog::Captured,
    pub projects: projects::Captured,
    pub construction: construction::Captured,
    pub(crate) projects_recorded: bool,
    pub ticks: [u64; TABLE_COUNT],
}

#[derive(Default)]
pub(crate) struct CharacterInputRoles {
    pub census: Option<
        engine_allocation::SharedBox<
            collector_characters::extended::characters::census::RoleCensus,
        >,
    >,
    pub store: Option<Box<collector_characters::extended::character_inputs::RoleStore>>,
}

pub(crate) struct CharacterInputsRequest {
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
    captured: character_inputs::Captured,
    roles: CharacterInputRoles,
}

impl CharacterInputsRequest {
    pub fn capture(&mut self, memory: &impl Memory) -> [u64; character_inputs::N] {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let census = self
            .roles
            .census
            .as_deref()
            .and_then(|census| census.admitted(self.base, self.game_state));
        self.captured.capture_census(
            memory,
            self.base,
            self.game_state,
            self.hour,
            self.force_full,
            census,
            self.roles.store.as_deref_mut(),
        )
    }
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
        if collector_memory::terminal::failed() {
            return None;
        }
        engine_allocation::boxed(Self {
            fog: fog::Captured::vacant(),
            projects: projects::Captured::vacant(),
            construction: construction::Captured::vacant(),
            projects_recorded: true,
            ticks: [0; TABLE_COUNT],
        })
        .ok()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &fog::TABLES,
            self.fog.capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        scatter(
            &construction::TABLES,
            self.construction
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        self.ticks = ticks;
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct FinalInputsReport {
    pub snapshots: [u64; 2],
    pub production: [u64; production::N],
    pub economy: [u64; economy::N],
    pub session: [u64; session::N],
    pub characters: [u64; characters::N],
    pub destruction: [u64; collector_equipment_destruction::extended::equipment_destruction::N],
    pub(crate) naval_sources: [u64; collector_naval_results::extended::naval_result_sources::N],
    pub air_losses: [u64; super::air_losses::N],
    pub air_records: [u64; super::air_records::N],
    pub bombing: [u64; super::building_bombing::N],
    pub engagement_refreshes: [u64; super::combat_engagement_refreshes::N],
    pub nuclear: [u64; super::nuclear_strikes::N],
    pub market: [u64; super::market::N],
    pub(crate) intel_delegated: [u64; super::intel::N],
}

pub(crate) struct FinalInputsRequest {
    pub countries: engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
    base: usize,
    state: usize,
    hour: i32,
    force_full: bool,
    divisions: super::Divisions,
    new_divisions: super::new_divisions::Captured,
    experience: super::experience_gains::Captured,
    production: production::Captured,
    economy: economy::Captured,
    session: session::Captured,
    characters: characters::Captured,
    destruction: collector_equipment_destruction::extended::equipment_destruction::Captured,
    naval_sources: collector_naval_results::extended::naval_result_sources::Captured,
    air_losses: super::air_losses::Captured,
    air_records: super::air_records::Captured,
    bombing: super::building_bombing::Captured,
    engagement_refreshes: super::combat_engagement_refreshes::Captured,
    nuclear: super::nuclear_strikes::Captured,
    market: super::market::Captured,
    intel: super::intel::Captured,
    finished: [bool; 15],
}
impl FinalInputsRequest {
    #[cfg(test)]
    pub fn allocated_bytes(&self) -> u64 {
        (self.divisions.allocated_bytes()
            + self.new_divisions.allocated_bytes()
            + self.experience.allocated_bytes()
            + self.production.allocated_bytes()
            + self.economy.allocated_bytes()
            + self.session.allocated_bytes()
            + self.characters.allocated_bytes()
            + self.destruction.allocated_bytes()
            + self.naval_sources.allocated_bytes()
            + self.air_losses.allocated_bytes()
            + self.air_records.allocated_bytes()
            + self.bombing.allocated_bytes()
            + self.engagement_refreshes.allocated_bytes()
            + self.nuclear.allocated_bytes()
            + self.market.allocated_bytes()
            + self.intel.allocated_bytes()) as u64
    }

    pub fn capture(&mut self, memory: &impl Memory) -> FinalInputsReport {
        let sources = self.divisions.sources();
        let new_divisions =
            self.new_divisions
                .capture(memory, self.base, sources.rows, sources.outcome)[0];
        self.finished[0] = true;
        let experience = self.experience.capture(memory, self.base, self.state)[0];
        self.finished[1] = true;
        let production = self.production.acquire(
            memory,
            self.base,
            self.state,
            self.divisions.sources(),
            self.hour,
            self.force_full,
        );
        self.finished[2] = true;
        let economy =
            self.economy
                .capture(memory, self.base, self.state, self.hour, self.force_full);
        self.finished[3] = true;
        let session =
            self.session
                .capture(memory, self.base, self.state, self.hour, self.force_full);
        self.finished[4] = true;
        let characters = self.characters.capture_growing(
            memory,
            self.base,
            self.state,
            self.hour,
            self.force_full,
        );
        self.finished[5] = true;
        let destruction = self.destruction.capture(memory, self.base, self.state);
        self.finished[6] = true;
        let naval_sources = self.naval_sources.capture(memory, self.base, self.state);
        self.finished[7] = true;
        // SAFETY: Admitted hour's sole capture pass drains air-loss and building-bombing ledgers.
        let air_losses = unsafe { self.air_losses.capture(memory, self.base, self.state) };
        self.finished[8] = true;
        // SAFETY: Admitted hour's sole capture pass drains air-loss and building-bombing ledgers.
        let bombing = unsafe { self.bombing.capture(memory, self.base, self.state) };
        self.finished[9] = true;
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
        self.finished[10] = true;
        let air_records =
            self.air_records
                .capture(memory, self.base, self.state, self.hour, self.force_full);
        self.finished[11] = true;
        // SAFETY: Admitted hour's sole capture pass drains nuclear-strike and market ledgers.
        let nuclear = unsafe { self.nuclear.capture(memory, self.base, self.state) };
        self.finished[12] = true;
        // SAFETY: Admitted hour's sole capture pass drains nuclear-strike and market ledgers.
        let market = unsafe {
            self.market
                .capture(memory, self.base, self.state, self.hour, self.force_full)
        };
        self.finished[13] = true;
        // SAFETY: Admitted hour's sole capture pass drains combat-engagement-refresh ledger.
        let engagement_refreshes = unsafe {
            self.engagement_refreshes
                .capture(memory, self.base, self.state)
        };
        self.finished[14] = true;
        FinalInputsReport {
            snapshots: [new_divisions, experience],
            production,
            economy,
            session,
            characters,
            destruction,
            naval_sources,
            air_losses,
            air_records,
            bombing,
            engagement_refreshes,
            nuclear,
            market,
            intel_delegated,
        }
    }
}
impl Captured {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_final_inputs_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> FinalInputsRequest {
        let variants = self.production.take_variants();
        let changes = self.production.take_design_changes();
        let production = std::mem::replace(&mut self.production, production::Captured::vacant());
        self.production.restore_variants(variants);
        self.production.restore_design_changes(changes);
        FinalInputsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            state,
            hour,
            force_full,
            divisions: std::mem::replace(&mut self.divisions, super::Divisions::new(0)),
            new_divisions: std::mem::replace(
                &mut self.new_divisions,
                super::new_divisions::Captured::vacant(),
            ),
            experience: std::mem::replace(
                &mut self.experience_gains,
                super::experience_gains::Captured::vacant(),
            ),
            production,
            economy: std::mem::replace(&mut self.economy, economy::Captured::vacant()),
            session: std::mem::replace(&mut self.session, session::Captured::vacant()),
            characters: std::mem::replace(&mut self.characters, characters::Captured::vacant()),
            destruction: std::mem::replace(
                &mut self.equipment_destruction,
                collector_equipment_destruction::extended::equipment_destruction::Captured::vacant(
                ),
            ),
            naval_sources: std::mem::replace(
                &mut self.naval_result_sources,
                collector_naval_results::extended::naval_result_sources::Captured::vacant(),
            ),
            air_losses: std::mem::replace(
                &mut self.air_losses,
                super::air_losses::Captured::vacant(),
            ),
            air_records: std::mem::replace(
                &mut self.air_records,
                super::air_records::Captured::vacant(),
            ),
            bombing: std::mem::replace(
                &mut self.building_bombing,
                super::building_bombing::Captured::vacant(),
            ),
            engagement_refreshes: std::mem::replace(
                &mut self.combat_engagement_refreshes,
                super::combat_engagement_refreshes::Captured::vacant(),
            ),
            nuclear: std::mem::replace(
                &mut self.nuclear_strikes,
                super::nuclear_strikes::Captured::vacant(),
            ),
            intel: std::mem::replace(&mut self.intel, super::intel::Captured::vacant()),
            market: std::mem::replace(&mut self.market, super::market::Captured::vacant()),
            finished: [false; 15],
        }
    }
    pub(crate) fn return_final_inputs_request(&mut self, request: FinalInputsRequest) {
        self.restore_final_inputs_request(request, FinalInputsReport::default(), None);
        self.final_inputs_ticks = None;
    }
    pub(crate) fn restore_final_inputs_request(
        &mut self,
        mut request: FinalInputsRequest,
        ticks: FinalInputsReport,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure {
            if !request.finished[0] {
                request.new_divisions.fail(reason);
            }
            if !request.finished[1] {
                request.experience.fail(reason);
            }
            if !request.finished[2] {
                request
                    .production
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[3] {
                request
                    .economy
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[4] {
                request
                    .session
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[5] {
                request
                    .characters
                    .fail_final_capture(request.hour, request.force_full, reason);
            }
            if !request.finished[6] {
                request.destruction.fail(reason);
            }
            if !request.finished[7] {
                request.naval_sources.fail(reason);
            }
            if !request.finished[8] {
                request.air_losses.fail(reason);
            }
            if !request.finished[9] {
                request.bombing.fail(reason);
            }
            if !request.finished[10] {
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
            if !request.finished[11] {
                request.air_records.fail(reason);
            }
            if !request.finished[13] {
                request.market.fail(reason);
            }
            if !request.finished[12] {
                request.nuclear.fail(reason);
            }
            if !request.finished[14] {
                // SAFETY: Final-inputs failure recovery owns invalidating the uncaptured refresh drain.
                unsafe { request.engagement_refreshes.fail(reason) };
            }
        }
        self.divisions = request.divisions;
        self.new_divisions = request.new_divisions;
        self.experience_gains = request.experience;
        request
            .production
            .restore_variants(self.production.take_variants());
        request
            .production
            .restore_design_changes(self.production.take_design_changes());
        self.production = request.production;
        self.economy = request.economy;
        self.session = request.session;
        self.characters = request.characters;
        self.equipment_destruction = request.destruction;
        self.naval_result_sources = request.naval_sources;
        self.air_losses = request.air_losses;
        self.air_records = request.air_records;
        self.building_bombing = request.bombing;
        self.combat_engagement_refreshes = request.engagement_refreshes;
        self.nuclear_strikes = request.nuclear;
        self.market = request.market;
        self.intel = request.intel;
        self.final_inputs_ticks = Some(ticks);
    }
}
