//! Aircraft in flight: each region's mission scenarios, training patrols and base launch effects for
//! the viewing country, as today's plan generates them ([`AircraftPlan::make`]). Which wings fly
//! joins the whole air war (visit order, group caps, target scans over many tables), each region's
//! scenarios are drawn from one random sequence over its wings in order, and launch records are
//! shared across regions: so the plan is made whole on the layer's worker, from the shared rows at
//! the head at the step that changes a table it reads, once the plan before is made, or when the
//! pools its models' scripts decided decide otherwise ([`Build::stands`]). What does not change is
//! kept: each (variant, country)'s model by what choosing it read ([`KeptModels`]), and a scenario
//! whose placement reads the same as the build before's ([`Kept`]). It is drawn below the map
//! objects' camera cutoff and declares nothing above it. Of what the viewer sees the plan reads
//! only the viewing country, resolved from the same rows as today's vision resolves it, so no vision
//! is made. Each plan the layer takes goes to the scene's aircraft pass.

use super::rebuilt::{Build, Seen, Whole};
use crate::{document::Content, playhead_host::rows::Cadence};
use replay_data::rows::{TableId, TaskForceRow};
use replay_frame::frame::DecodedFrame;
use std::{
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::map::{counter_move::RouteWorld, presentation::Continuity, province_map::HeightField, task_force_visual::TaskForceMotions};
use viewer_world::map::{
    aircraft::{AircraftPlan, Kept, KeptModels, ModelsMade},
    equipment_graphics::{Decided, LimitValues, Limits},
};

/// The tables today's plan is known to read, declared from the start: a table read beyond them is
/// still learned as the builder reads it, and the worker's reader declares it. Each group by why
/// the plan reads it.
pub const TABLES: &[TableId] = &[
    // The plan before any script: wings, missions, patrols, carriers and the countries at war.
    TableId::AirWings,
    TableId::WingEquipment,
    TableId::AirRegionalMissions,
    TableId::AirNavalPatrolCombats,
    TableId::AirNavalPatrolHistories,
    TableId::NavalRegionalMines,
    TableId::NavalBases,
    TableId::Ships,
    TableId::TaskForces,
    TableId::TaskForceMapMotion,
    TableId::ProvinceUnitOrder,
    TableId::Provinces,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::AirCountryRelations,
    TableId::VolunteerHosts,
    TableId::SimulationFeatures,
    TableId::VariantDefinitions,
    // The viewing country, when the perspective follows the recorded player.
    TableId::LocalPlayer,
    // The base registry and the sortie sources: which bases fly what, with which modifiers.
    TableId::AirReferenceWitnesses,
    TableId::AirBases,
    TableId::AirMissionInputs,
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
    TableId::AirSortieModifierMetadata,
    // A mission's targets in reach: battles, divisions on the move, states and their buildings.
    TableId::States,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::Divisions,
    TableId::DivisionMovement,
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::NavalCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    // A wing's model, chosen by the content's scripts: the countries' politics, factions, subjects,
    // appearance and scripted values.
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Subjects,
    TableId::RelationLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::ScriptVariables,
    TableId::Characters,
    TableId::CountryCounterInputs,
    TableId::GlobalSavedEventTargets,
];

pub type Aircraft = Whole<Flights>;

impl Aircraft {
    /// The aircraft's limitation notes in today's words, from the tables' `statuses` at the head;
    /// none while hidden.
    pub fn notes(&self, statuses: &dyn viewer_foundation::map::tables::Statuses) -> Vec<String> {
        if !self.drawn() {
            return Vec::new();
        }
        let mut out = viewer_foundation::map::tables::Limitations::default();
        out.note_aircraft(statuses);
        out.into_lines()
    }

    /// The unit models' line today's window gives for the aircraft: the shown plan's refusal.
    pub fn warning(&self) -> Option<String> {
        self.refusal().filter(|_| self.shows()).map(|why| format!("aircraft: {why}"))
    }
}

/// One plan, the pools its models' scripts decided (what it read beyond the rows), and what its
/// models kept and chose. It reads as its plan.
pub struct Flown {
    pub plan: AircraftPlan,
    decided: Decided,
    models: ModelsMade,
}

impl std::ops::Deref for Flown {
    type Target = AircraftPlan;

    fn deref(&self) -> &AircraftPlan {
        &self.plan
    }
}

/// Today's aircraft plan builder over the recording's content, and what its worker keeps from one
/// build to the next: the scenarios' placements and the models.
pub struct Flights {
    content: Arc<Content>,
    kept: Mutex<(Kept, KeptModels)>,
}

impl Flights {
    /// The plan of `frame` for the viewer of `choice`, keeping from `kept` what reads the same.
    fn make(&self, frame: &DecodedFrame, choice: &viewer_foundation::map::perspective::Choice, (kept, models): &mut (Kept, KeptModels)) -> io::Result<Flown> {
        let content = &*self.content;
        let hour = crate::document::Hour::new(content, frame, 0.0);
        let ours = super::intel_icons::ours(frame, choice)?;
        let heights = HeightField { width: content.assets.heightmap.width, height: content.assets.heightmap.height, values: content.assets.heightmap_texels() };
        let movement = content.movement.as_deref();
        let world = movement.map(|movement| RouteWorld { map: &movement.map, heights: hour.heights(), width: content.map.width as f32 });
        let motions = TaskForceMotions::read(frame)?;
        let land = |province| {
            movement.and_then(|movement| movement.map.terrain(province)).map_or_else(
                || content.map.definitions.get(&province).is_some_and(|definition| definition.kind == installed_content_core::map::ProvinceKind::Land),
                |terrain| terrain.flags & 1 != 0,
            )
        };
        let origin = |force: &TaskForceRow| motions.settled_origin(force, movement, world.as_ref(), &|province, slot| content.counters.anchor(province, slot), &land);
        let evaluate = |items: &[installed_content_core::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
        let values = LimitValues::default();
        let (plan, decided, made) = models.session(&content.unit_models, frame, Limits::new(&values, &evaluate), |model| {
            AircraftPlan::make(frame, &content.aircraft, &content.counters.region_of, &content.province_geometry, &heights, ours, model, &origin, &|province| content.products.impact_place(province), kept)
        });
        Ok(Flown { plan: plan?, decided, models: made })
    }
}

impl Build for Flights {
    type Product = Flown;

    /// The limits its models' scripts decide may compare dates.
    fn hourly(&self) -> bool {
        true
    }

    fn tables(&self) -> Option<engine_executable::executable_tables::Tables> {
        self.content.tables
    }

    fn build(&self, frame: &DecodedFrame, (choice, _): &Seen) -> io::Result<Flown> {
        let mut kept = self.kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.make(frame, choice, &mut kept)
    }

    fn anew(&self, frame: &DecodedFrame, (choice, _): &Seen) -> io::Result<Flown> {
        self.make(frame, choice, &mut Default::default())
    }

    /// The rows are the same, so the plan is the same while every pool its models' limits decided
    /// decides the same at the new hour; never a plan with a diagnostic, whose words may name the
    /// hour it was made at.
    fn stands(&self, flown: &Flown, frame: &DecodedFrame, _: &Seen) -> bool {
        if flown.counts()[2] > 0 {
            return false;
        }
        let hour = crate::document::Hour::new(&self.content, frame, 0.0);
        let evaluate = |items: &[installed_content_core::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
        let values = LimitValues::default();
        Limits::new(&values, &evaluate).decide_the_same(&flown.decided)
    }

    fn parts(&self, flown: &Flown, each: &mut dyn FnMut(&'static str, u64)) {
        for (name, ns) in flown.marks.parts() {
            each(name, ns);
        }
    }

    fn counts(&self, flown: &Flown, each: &mut dyn FnMut(&'static str, u64)) {
        let marks = flown.marks;
        for (name, count) in [
            ("wings admitted", marks.wings),
            ("scenarios placed", marks.scenarios),
            ("scenarios kept", marks.kept),
            ("flights placed", marks.flights_placed),
            ("flights kept", marks.flights_kept),
            ("models kept", flown.models.kept),
            ("models chosen", flown.models.chosen),
        ] {
            each(name, count as u64);
        }
    }

    fn install(&self, gpu: &mut super::Gpu<'_>, flown: Option<&Flown>, continuity: Continuity) -> io::Result<Option<Flown>> {
        let plan = flown.map_or_else(AircraftPlan::default, |flown| flown.plan.clone());
        let replaced = gpu.scene.set_aircraft(Arc::clone(&gpu.content.aircraft), plan, continuity)?;
        gpu.scene.warm_added_aircraft(gpu.device)?;
        Ok(replaced.map(|plan| Flown { plan, decided: Decided::new(), models: ModelsMade::default() }))
    }
}

fn drawn(flights: &Flights, view: &super::View) -> bool {
    view.camera.is_none_or(|camera| camera.height < flights.content.map_defines.map_objects_cutoff)
}

/// The aircraft layer over `content`, reading the rows at the head at the step that changes a table
/// it reads (and ten times a second at most when steps move only the hour); `timed` as
/// [`Whole::new`].
pub fn new(content: Arc<Content>, timed: bool) -> io::Result<Aircraft> {
    Whole::new("aircraft", Flights { content, kept: Mutex::default() }, drawn, |flown| flown.plan.bytes(), TABLES, Cadence::Ready, false, timed)
}

/// What the aircraft's worker measured for the paced report: builds, how many a second and how much
/// of a core with the rows it found the shown plan stands at, what a build is made of (the maker's
/// parts; the rest is the layer's own reads before the maker), how far behind the head the rows of
/// each build were, and the shown plan's size.
pub fn summary(side: &crate::playhead_host::MapSide) -> serde_json::Value {
    let Some(layer) = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Aircraft>())) else { return serde_json::Value::Null };
    let builds = layer.rebuilt().builds();
    let count = builds.count.max(1) as f64;
    let seconds = builds.span.map(|(first, last)| last.duration_since(first).as_secs_f64()).filter(|&seconds| seconds > 0.0);
    let parts_ns: u64 = builds.parts.iter().map(|(_, ns)| ns).sum();
    let mut parts = serde_json::Map::new();
    for (name, ns) in builds.parts.iter().copied().chain([("rest", builds.total_ns.saturating_sub(parts_ns))]) {
        parts.insert(name.into(), serde_json::json!({"ms_mean": ns as f64 / 1e6 / count, "share": ns as f64 / builds.total_ns.max(1) as f64}));
    }
    let counts: serde_json::Map<String, serde_json::Value> = builds.counts.iter().map(|&(name, count)| (name.to_owned(), serde_json::json!({"total": count, "a_build": count as f64 / count_of(&builds)}))).collect();
    let behind: serde_json::Map<String, serde_json::Value> = super::rebuilt::BEHIND.iter().zip(builds.behind).map(|(bound, n)| (if *bound == u64::MAX { "more".to_owned() } else { format!("up to {bound}") }, n.into())).collect();
    let shown = layer.product().and_then(|built| built.product.as_ref().ok().map(|flown| (flown.counts(), flown.marks, built.hour, built.current())));
    serde_json::json!({
        "builds": builds.count,
        "builds_a_second": seconds.map(|seconds| builds.count.saturating_sub(1) as f64 / seconds),
        "build_ms": {"mean": builds.total_ns as f64 / 1e6 / count, "most": builds.most_ns as f64 / 1e6},
        "cores": seconds.map(|seconds| (builds.total_ns + builds.confirm_ns) as f64 / 1e9 / seconds),
        "stands": {"count": builds.confirmed, "ms_mean": builds.confirm_ns as f64 / 1e6 / builds.confirmed.max(1) as f64},
        "parts": parts,
        "counts": counts,
        "rows_behind_head_hours": {"buckets": behind, "mean": builds.behind_sum as f64 / count, "most": builds.behind_most},
        "passed_over": builds.skipped, "discovered": builds.discovered, "tables": layer.rebuilt().tables().len(),
        "shown": shown.map(|([flights, launches, diagnostics], marks, made, current)| serde_json::json!({"flights": flights, "launches": launches, "diagnostics": diagnostics, "wings_admitted": marks.wings, "scenarios_placed": marks.scenarios, "scenarios_kept": marks.kept, "made_at": made, "current_at": current})),
        "scope": "The aircraft worker over the run: its builds, made from the shared rows at the head when a table it reads changed, at the step, and the rows it found the shown plan stands at without a build (stands: the pools its scripts decided decide the same); cores counts both against the span from the first build to the last. Per part of a build the mean milliseconds and its share of build time (models: each wing's model, kept or chosen by the content's scripts; placement: routes and terrain of the scenarios placed anew; rest: the layer's reads before the maker); the wings, scenarios and models made anew and kept from the build before; the game hours the rows of each build were behind the map's head when it began; frames of rows published to the worker it never took.",
    })
}

fn count_of(builds: &super::rebuilt::Builds) -> f64 {
    builds.count.max(1) as f64
}
