use super::{
    Captured, Divisions, MISSING, active_abilities, air, army_icon_inputs, character_inputs,
    combat_details, construction, country_appearance, country_counters, country_stability,
    country_tactic_unlocks, division_templates, faction_theatres, fog, group_counters,
    hired_advisors, leader_paths, leader_portraits, map_facilities, map_names, model_inputs,
    modifier_definitions, naval_bases, naval_combat_sources, naval_counters, naval_details,
    naval_history, naval_mission_inputs, naval_result_details, projects, raid_launch_inputs,
    raid_state_inputs, raid_target_statuses, railway_guns, saved_event_targets, scatter,
    unpublished, visibility,
};
use crate::{division_census::UnitInputs, map_capture::Orders};
use engine::{Memory, walk::Outcome};
use replay::{frame::TABLE_COUNT, rows::TableId};
use std::{mem::size_of, sync::Arc};

impl Captured {
    /// Lend the ordinary packs to the pool: swap them with the spare set so
    /// nothing allocates on the game thread.
    pub(crate) fn take_ordinary_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
    ) -> crate::acquisition_jobs::OrdinaryRequest {
        self.ordinary_ticks = None;
        self.ordinary_failure = None;
        let mut packs = self.ordinary_spare.take().expect("ordinary spare present");
        self.swap_ordinary(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        packs.failure = None;
        crate::acquisition_jobs::OrdinaryRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            packs,
        }
    }

    /// Every terminal job installs a completion marker, including failure, so
    /// table assembly cannot retry an allocating walk on the callback.
    pub(crate) fn restore_ordinary_request(
        &mut self,
        request: crate::acquisition_jobs::OrdinaryRequest,
        failure: Option<&'static str>,
    ) {
        let mut packs = request.packs;
        if let Some(reason) = failure {
            packs.fail(reason);
        }
        self.ordinary_ticks = Some(packs.ticks);
        self.ordinary_failure = packs.failure;
        self.swap_ordinary(&mut packs);
        self.ordinary_spare = Some(packs);
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
        std::mem::swap(&mut self.naval_counters, &mut packs.naval_counters);
        std::mem::swap(&mut self.railway_guns, &mut packs.railway_guns);
        std::mem::swap(&mut self.active_abilities, &mut packs.active_abilities);
        std::mem::swap(&mut self.country_counters, &mut packs.country_counters);
        std::mem::swap(&mut self.group_counters, &mut packs.group_counters);
        std::mem::swap(
            &mut self.saved_event_targets,
            &mut packs.saved_event_targets,
        );
    }

    /// Loan the slow packs, with the raid families' fingerprint stores, to
    /// the Slow job.
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
            packs.fail(request.hour, request.force_full, "slow acquisition failed");
            for store in [&mut stores.stability, &mut stores.naval_missions]
                .into_iter()
                .flatten()
            {
                store.forget();
            }
        }
        if let Some(fingerprints) = stores.naval_missions.as_mut() {
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
        std::mem::swap(&mut self.country_stability, &mut packs.country_stability);
        std::mem::swap(
            &mut self.country_tactic_unlocks,
            &mut packs.country_tactic_unlocks,
        );
        std::mem::swap(
            &mut self.naval_mission_inputs,
            &mut packs.naval_mission_inputs,
        );
    }

    /// Loan the late packs and the walked order census to the job.
    /// `naval_raw`: the Naval raw job staged the order-of-battle sources.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_late_request(
        &mut self,
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
            inputs: self.unit_inputs.as_ref().map(Arc::clone),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
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
        let mut packs = request.packs;
        let mut air_scores = request.air_scores;
        if !completed {
            packs.fail(request.hour, request.force_full, "late acquisition failed");
            air_scores.invalidate();
        }
        self.late_ticks = Some(packs.ticks);
        self.swap_late(&mut packs, orders);
        self.late_spare = Some(packs);
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
    pub(crate) naval_counters: naval_counters::Captured,
    pub(crate) railway_guns: railway_guns::Captured,
    pub(crate) active_abilities: active_abilities::Captured,
    pub(crate) country_counters: country_counters::Captured,
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
        &naval_counters::TABLES,
        &railway_guns::TABLES,
        &active_abilities::TABLES,
        &country_counters::TABLES,
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
            + self.naval_counters.allocated_bytes()
            + self.railway_guns.allocated_bytes()
            + self.active_abilities.allocated_bytes()
            + self.country_counters.allocated_bytes()
            + self.group_counters.allocated_bytes()
            + self.saved_event_targets.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.combat_details.fail(reason);
        self.naval_details.reset();
        self.naval_result_details.reset();
        self.country_appearance.reset();
        self.naval_bases.reset();
        self.map_facilities.reset();
        self.naval_combat_sources.reset();
        self.naval_counters.reset();
        self.railway_guns.reset();
        self.active_abilities.reset();
        self.country_counters.reset();
        self.group_counters.reset();
        self.saved_event_targets.reset();
        self.ticks = [0; TABLE_COUNT];
        self.failure = Some(reason);
    }
    /// Placeholders the extended capture holds while the pool owns the real
    /// packs. Only the reusable box allocates; the hourly exchange is a swap.
    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
            combat_details: combat_details::Captured::vacant(),
            naval_details: naval_details::Captured::vacant(),
            naval_result_details: naval_result_details::Captured::vacant(),
            country_appearance: country_appearance::Captured::vacant(),
            naval_bases: naval_bases::Captured::vacant(),
            map_facilities: map_facilities::Captured::vacant(),
            naval_combat_sources: naval_combat_sources::Captured::vacant(),
            naval_counters: naval_counters::Captured::vacant(),
            railway_guns: railway_guns::Captured::vacant(),
            active_abilities: active_abilities::Captured::vacant(),
            country_counters: country_counters::Captured::vacant(),
            group_counters: group_counters::Captured::vacant(),
            saved_event_targets: saved_event_targets::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
            failure: None,
        })
    }

    pub(crate) fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! pack {
            ($($pack:ident),+ $(,)?) => {$(
                scatter(&$pack::TABLES, self.$pack.capture(memory, base, game_state), &mut ticks);
            )+};
        }
        pack!(
            combat_details,
            naval_details,
            naval_result_details,
            country_appearance,
            naval_bases,
            map_facilities,
            naval_combat_sources,
            naval_counters,
            railway_guns,
            active_abilities,
            country_counters,
            group_counters,
            saved_event_targets
        );
        self.ticks = ticks;
    }
}

/// The Slow job's owner stores (`slicing::STABILITY`, `NAVAL_MISSIONS`). A
/// missing store only drops that family's early reads for the hour.
#[derive(Default)]
pub(crate) struct SlowStores {
    pub(crate) stability: Option<Box<crate::slicing::SliceFingerprints>>,
    pub(crate) naval_missions: Option<Box<crate::slicing::SliceFingerprints>>,
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
/// modifier definitions, hired advisors and character inputs). No defines are recorded: the
/// viewer reads the movement defines from the content's `NDefines`. Each
/// table keeps its own [`replay::cadence`]: the
/// template dictionary is daily, its counter inputs hourly.
pub(crate) struct SlowPacks {
    pub(crate) map_names: map_names::Captured,
    pub(crate) division_templates: division_templates::Captured,
    pub(crate) visibility: visibility::Captured,
    pub(crate) faction_theatres: faction_theatres::Captured,
    pub(crate) model_inputs: model_inputs::Captured,
    pub(crate) modifier_definitions: modifier_definitions::Captured,
    pub(crate) hired_advisors: hired_advisors::Captured,
    pub(crate) character_inputs: character_inputs::Captured,
    pub(crate) country_stability: country_stability::Captured,
    pub(crate) country_tactic_unlocks: country_tactic_unlocks::Captured,
    pub(crate) naval_mission_inputs: naval_mission_inputs::Captured,
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

    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
            raid_target_statuses: raid_target_statuses::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let mut ticks = [0; TABLE_COUNT];
        let cost = self
            .raid_target_statuses
            .capture(memory, base, game_state, hour, force_full);
        scatter(&raid_target_statuses::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

impl RaidStatePacks {
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.raid_state_inputs
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.raid_state_inputs.allocated_bytes()
    }

    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
            raid_state_inputs: raid_state_inputs::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
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

    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
            raid_launch_inputs: raid_launch_inputs::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
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
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
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
        self.country_stability
            .fail_capture(hour, force_full, reason);
        self.country_tactic_unlocks
            .fail_capture(hour, force_full, reason);
        self.naval_mission_inputs
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.map_names.allocated_bytes()
            + self.division_templates.allocated_bytes()
            + self.visibility.allocated_bytes()
            + self.faction_theatres.allocated_bytes()
            + self.model_inputs.allocated_bytes()
            + self.modifier_definitions.allocated_bytes()
            + self.hired_advisors.allocated_bytes()
            + self.character_inputs.allocated_bytes()
            + self.country_stability.allocated_bytes()
            + self.country_tactic_unlocks.allocated_bytes()
            + self.naval_mission_inputs.allocated_bytes()
    }

    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
            map_names: map_names::Captured::vacant(),
            division_templates: division_templates::Captured::vacant(),
            visibility: visibility::Captured::vacant(),
            faction_theatres: faction_theatres::Captured::vacant(),
            model_inputs: model_inputs::Captured::vacant(),
            modifier_definitions: modifier_definitions::Captured::vacant(),
            hired_advisors: hired_advisors::Captured::vacant(),
            character_inputs: character_inputs::Captured::vacant(),
            country_stability: country_stability::Captured::vacant(),
            country_tactic_unlocks: country_tactic_unlocks::Captured::vacant(),
            naval_mission_inputs: naval_mission_inputs::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
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
        let mut ticks = [0; TABLE_COUNT];
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
            map_names,
            division_templates,
            visibility,
            faction_theatres,
            model_inputs,
            modifier_definitions,
            hired_advisors,
            character_inputs,
            country_tactic_unlocks
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
        let cost = self.naval_mission_inputs.capture(
            memory,
            base,
            game_state,
            hour,
            force_full,
            stores.naval_missions.as_deref_mut(),
        );
        scatter(&naval_mission_inputs::TABLES, cost, &mut ticks);
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
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
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

    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
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
        })
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
                parent,
                &mut [],
                &crate::history_changes::Unobserved,
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
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
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

    pub(super) fn spare() -> Box<Self> {
        Box::new(Self {
            fog: fog::Captured::vacant(),
            projects: projects::Captured::vacant(),
            construction: construction::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
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
        scatter(
            &projects::TABLES,
            self.projects
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        self.ticks = ticks;
    }
}
