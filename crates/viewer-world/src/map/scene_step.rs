//! The presentation a map frame advances before it draws: model histories and placements,
//! train and aircraft plans, strikes, map-frame actors, movement arrows and world text. It needs
//! no GPU, so it can be stepped and compared without one.
//!
//! Display switches are not inputs to any of it. They only choose which placed actors draw
//! (each pass's hidden set), so a hidden layer keeps its admission, plans, random draws,
//! clocks, mesh budget and animation epochs, and shows exactly as if it had never been hidden.
use super::{BUILDING_TILE, BuildingHistory, ModelPass, UnitGates, hidden_map_frame_actors};
use crate::map::border_pass::wrap_offsets;
use crate::map::terrain::MARGIN;
use std::{collections::BTreeSet, sync::Arc};
use viewer_foundation::map::camera::Camera;

/// Every part of a scene that a frame advances, borrowed for one step.
pub(super) struct Presentation<'a> {
    pub graphics: &'a dashboard_core::protocol::MapGraphics,
    pub aircraft: Option<&'a mut (
        ModelPass,
        Arc<crate::map::aircraft::AircraftAssets>,
        crate::map::aircraft::AircraftPlan,
    )>,
    pub trains: Option<&'a mut (
        ModelPass,
        Arc<crate::map::trains::TrainAssets>,
        crate::map::trains::TrainPlan,
    )>,
    pub sprawl: Option<&'a mut ModelPass>,
    pub city_presentation: Option<&'a mut crate::map::sprawl::Presentation>,
    pub unit_history: &'a mut crate::map::unit_history::History,
    pub building_history: &'a mut BuildingHistory,
    pub map_buildings: Option<&'a mut ModelPass>,
    pub unit_models: Option<&'a mut ModelPass>,
    pub unit_gates: Option<UnitGates>,
    /// The map mode's layers: its land models and unit arrows.
    pub visibility: viewer_foundation::map::mode_visibility::Visibility,
    pub mesh_limit: i32,
    /// `None` only where no device built the arrow player (the GPU-free tests).
    pub movement_arrows: Option<&'a mut crate::map::movement_arrows::Player>,
    pub animated_text: Option<&'a mut crate::map::animated_map_text::Player>,
    pub strikes: &'a mut crate::map::strike_effects::Presentation,
    pub missile_clock: &'a mut Option<f32>,
    pub map_frame: Option<&'a mut ModelPass>,
    pub ambient: Option<&'a mut crate::map::mapframe::Presentation>,
    pub last_animation_time: Option<f32>,
    /// The hour builders' threads the step may lend its independent work.
    pub workers: &'a viewer_foundation::parallel::Workers,
}

impl Presentation<'_> {
    /// Advances everything to `animation_time` for `camera`. `rails_shown` is the camera's
    /// railway fade, below which trains run; display switches decide nothing here but which
    /// placed actors draw.
    /// Returns the players of shown strike impacts, whose sounds are combat sounds.
    pub(super) fn step(
        self,
        camera: Camera,
        size: [u32; 2],
        animation_time: f32,
        rails_shown: bool,
    ) -> BTreeSet<(usize, u32, u32)> {
        let Self {
            graphics,
            mut aircraft,
            trains,
            sprawl,
            city_presentation,
            unit_history,
            building_history,
            map_buildings,
            unit_models,
            unit_gates,
            visibility,
            mesh_limit,
            movement_arrows,
            animated_text,
            strikes,
            missile_clock,
            map_frame,
            ambient,
            last_animation_time,
            workers,
        } = self;
        let visible = camera.visible_ground(size, MARGIN);
        let animation_offsets = wrap_offsets(visible, camera.map.x());
        // Every mission is placed; one the display hides stays placed and only stops drawing.
        if let Some((pass, assets, plan)) = aircraft.as_deref_mut() {
            // The band reads the actors' players as the previous frame left them.
            let (mut field, hidden) = plan.field(
                assets,
                animation_time,
                &|mission| graphics.air_mission(mission),
                &|key| pass.root_play_position(key),
            );
            pass.replace_field(&assets.library(), &mut field);
            pass.set_hidden(hidden);
            pass.animate_with_workers(animation_time, None, &[0.], workers);
        }
        // Trains run below the railway fade, whether or not the display shows them.
        if let Some((pass, assets, plan)) = trains
            && rails_shown
        {
            pass.replace_field(&assets.library, plan.field(assets, animation_time));
            pass.animate_with_workers(animation_time, visible, &animation_offsets, workers);
        }
        // City actors are precreated outside the dynamic mesh creation budget. Each province's
        // damage inputs select their states.
        if let (Some(pass), Some(presentation)) = (sprawl, city_presentation) {
            if let Some(placed) = presentation.update(
                camera,
                size,
                crate::map::models::draws_city_sprawl(&camera),
                animation_time,
            ) {
                pass.update_placed(placed, crate::map::sprawl::SPRAWL_TILE);
            }
            pass.set_clocks(presentation.clocks(animation_time));
            pass.animate_with_workers(animation_time, None, &[0.0], workers);
        }
        let mut gates = unit_gates.unwrap_or(UnitGates::OFF);
        if let Some(arrows) = &movement_arrows {
            gates.arrows = arrows.cutoff();
        }
        // The raids map mode draws no land unit models (ships stay), and it and the operatives
        // mode no unit arrows, as the game's camera cutoffs would.
        let gates = gates.for_mode(&visibility);
        let models = unit_history.update_models(
            camera,
            size,
            animation_time,
            gates,
            mesh_limit,
            building_history,
            true,
        );
        if let Some(arrows) = movement_arrows {
            arrows.advance(
                last_animation_time.map_or(0.0, |last| (animation_time - last).max(0.0)),
                last_animation_time.is_some_and(|last| animation_time < last),
                camera.height,
                |province| unit_history.arrows_admitted(province),
            );
        }
        if let Some(text) = animated_text {
            text.advance(camera, animation_time);
        }
        if let Some(pass) = map_buildings {
            if let Some(placed) = models.buildings {
                pass.update_placed(placed, BUILDING_TILE);
            }
            pass.animate_with_workers(animation_time, visible, &animation_offsets, workers);
        }
        if let Some(pass) = unit_models {
            pass.update_placed(models.units, crate::map::unit_models::UNIT_TILE);
            pass.set_clocks(unit_history.combat_clocks());
            // Advance hidden entities too. History retires their players independently of
            // render culling.
            pass.animate_with_workers(animation_time, None, &[0.0], workers);
        }
        // Missile scenarios finished since the last frame start their explosions on the clock
        // the flying aircraft share. Their own clock mark moves with the start, so a frame that
        // fails later starts none of them again.
        if let Some((_, _, plan)) = aircraft {
            let from = missile_clock.unwrap_or(animation_time);
            let explosions = plan.missile_explosions(from, animation_time);
            strikes.receive_explosions(&explosions, animation_time);
        }
        *missile_clock = Some(animation_time);
        let mut struck = BTreeSet::new();
        if let (Some(pass), Some(ambient)) = (map_frame, ambient) {
            let mut placed = ambient.update(camera, size, animation_time);
            // Retirement reads the players the previous frame animated.
            let (impacts, clocks) = strikes.update(camera, size, animation_time, |key| {
                pass.root_play_position(key)
            });
            // The table and the impacts share this pass: a hidden one stays placed, so its
            // admission, clocks and retirement carry on, and only stops drawing.
            let key = |(mesh, instance): &(usize, crate::map::models::ModelInstance)| {
                let (seed, identity) = instance.animation_key();
                (*mesh, seed, identity)
            };
            pass.set_hidden(hidden_map_frame_actors(
                graphics,
                placed.iter().map(key),
                impacts
                    .iter()
                    .map(key)
                    .map(|key| (key, strikes.effect(key))),
            ));
            placed.extend(impacts);
            pass.update_placed(placed, 256.);
            struck.extend(clocks.keys().copied());
            pass.set_clocks(clocks);
            pass.animate_with_workers(animation_time, None, &[0.], workers);
        }
        struck
    }
}

/// The model passes whose animation events feed the frame's particle simulations and lights.
pub(super) struct EventSources<'a> {
    pub map_frame: Option<&'a ModelPass>,
    pub sprawl: Option<&'a ModelPass>,
    pub map_buildings: Option<&'a ModelPass>,
    pub unit_models: Option<&'a ModelPass>,
    pub aircraft: Option<&'a ModelPass>,
    pub trains: Option<&'a ModelPass>,
    /// The model a free camera mode's user steers, which every camera and switch shows.
    pub player: Option<&'a ModelPass>,
}

/// Every event the frame's passes emitted, in submission order. A hidden layer's or actor's
/// events are kept and marked hidden, so every simulation, its retention included, advances on
/// them, and only their particles and light go unsubmitted. Only the camera decides which layers
/// emit at all, as before: aircraft above their draw height and trains above the railway fade
/// do not.
pub(super) fn frame_events(
    sources: EventSources<'_>,
    graphics: &dashboard_core::protocol::MapGraphics,
    camera: &Camera,
    animation_time: f32,
    rails_shown: bool,
) -> Vec<crate::map::entity_animation::Event> {
    let mut events = Vec::new();
    if animation_time <= 0.0 {
        return events;
    }
    let mut add =
        |pass: Option<&ModelPass>, layer_hidden: bool| {
            if let Some(pass) = pass {
                events.extend(pass.events.iter().map(|event| {
                    crate::map::entity_animation::Event {
                        hidden: layer_hidden || pass.hides(event.key.actor()),
                        ..event.clone()
                    }
                }));
            }
        };
    add(sources.map_frame, false);
    add(sources.sprawl, !graphics.city_sprawl);
    add(sources.map_buildings, !graphics.buildings);
    add(sources.unit_models, !graphics.unit_figures());
    if crate::map::aircraft::draws(camera) {
        add(sources.aircraft, !graphics.aircraft());
    }
    if rails_shown {
        add(sources.trains, !graphics.supply_trains());
    }
    add(sources.player, false);
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::trains::{TrainAssets, TrainPlan};
    use dashboard_core::protocol::MapGraphics;
    use viewer_foundation::map::camera::{MapDefines, MapSize, Placement};
    use viewer_foundation::map::presentation::Continuity;

    /// The GPU-free parts of a scene that a frame advances, with the entity particle
    /// simulations its events feed.
    struct World {
        trains: (ModelPass, Arc<TrainAssets>, TrainPlan),
        unit_models: ModelPass,
        simulations: crate::map::weather_effects::EntitySimulations,
        events: Vec<crate::map::entity_animation::Event>,
        unit_history: crate::map::unit_history::History,
        building_history: BuildingHistory,
        strikes: crate::map::strike_effects::Presentation,
        missile_clock: Option<f32>,
        last: Option<f32>,
    }

    impl World {
        fn new() -> Self {
            use crate::map::unit_history::{Kind, test_support::target};
            let assets = Arc::new(crate::map::trains::tests::assets());
            // Every unit figure plays a clip whose event emits kept particles.
            let mut rig = crate::map::entity_animation::stage_rig(false);
            rig.nodes[0].clips[0].animation = None;
            rig.nodes[0].clips[0]
                .state
                .events
                .push(installed_content_core::mesh::EntityEvent {
                    particle: Some("wake".into()),
                    keep_particle: true,
                    ..Default::default()
                });
            let bounds = Arc::new(viewer_foundation::map::province_admission::edge_boxes());
            let mut unit_history = crate::map::unit_history::History::default();
            unit_history.set(
                (1..=6)
                    .map(|id| target(id, if id == 2 { Kind::Ship } else { Kind::Land }))
                    .collect(),
                bounds,
                true,
            );
            Self {
                trains: (
                    ModelPass::new(&assets.library, "supply trains"),
                    assets,
                    crate::map::trains::tests::plan(),
                ),
                unit_models: ModelPass::with_rigs_for_tests(vec![Arc::new(rig)], "unit models"),
                simulations: Default::default(),
                events: Vec::new(),
                unit_history,
                building_history: Default::default(),
                strikes: Default::default(),
                missile_clock: None,
                last: None,
            }
        }

        fn step(&mut self, graphics: &MapGraphics, time: f32) {
            let camera =
                Placement::map([3000.0, 1000.0], 50.0).on(MapSize::TEST, MapDefines::test());
            self.unit_history.advance_schedule(camera, [1024, 768]);
            Presentation {
                graphics,
                aircraft: None,
                trains: Some(&mut self.trains),
                sprawl: None,
                city_presentation: None,
                unit_history: &mut self.unit_history,
                building_history: &mut self.building_history,
                map_buildings: None,
                unit_models: Some(&mut self.unit_models),
                unit_gates: Some(crate::map::unit_history::test_support::GATES),
                visibility: viewer_foundation::map::mode_visibility::Visibility::POLITICAL,
                mesh_limit: i32::MAX,
                movement_arrows: None,
                animated_text: None,
                strikes: &mut self.strikes,
                missile_clock: &mut self.missile_clock,
                map_frame: None,
                ambient: None,
                last_animation_time: self.last,
                workers: &Default::default(),
            }
            .step(camera, [1024, 768], time, true);
            self.last = Some(time);
            // The frame's events reach the particle simulations as the scene hands them over.
            let events = frame_events(
                EventSources {
                    map_frame: None,
                    sprawl: None,
                    map_buildings: None,
                    unit_models: Some(&self.unit_models),
                    aircraft: None,
                    trains: Some(&self.trains.0),
                    player: None,
                },
                graphics,
                &camera,
                time,
                true,
            );
            let emitter = crate::map::weather_effects::tests::long_lived_test_emitter();
            let entities = std::collections::BTreeMap::from([("wake".into(), (1., vec![emitter]))]);
            let (events, _) = crate::map::weather_effects::advance_entity_effects(
                &mut self.simulations,
                &entities,
                &crate::map::entity_effects::Lights::default(),
                events,
            )
            .unwrap();
            self.events = events;
        }

        /// The next hour's plan, with a changed allocation, installed as the scene does.
        fn next_hour(&mut self) {
            let mut next = crate::map::trains::tests::next_hour_plan();
            next.carry_presentation(&mut self.trains.2, Continuity::Continue);
            self.trains.2 = next;
        }

        fn state(&self) -> String {
            format!(
                "{:?}",
                (
                    self.trains.0.presentation_state(),
                    self.trains.2.presentation_state(),
                    self.unit_models.presentation_state(),
                    self.simulations.presentation_state(),
                    &self.strikes,
                    self.missile_clock,
                )
            )
        }
    }

    /// Every boolean choice off: every show/hide switch, main and part.
    fn everything_hidden() -> MapGraphics {
        let mut value = serde_json::to_value(MapGraphics::default()).unwrap();
        for choice in value.as_object_mut().unwrap().values_mut() {
            if choice.is_boolean() {
                *choice = false.into();
            }
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn hiding_every_layer_leaves_every_plan_history_and_random_stream_as_shown() {
        let shown = MapGraphics::default();
        let hidden = everything_hidden();
        assert!(!hidden.supply_trains() && !hidden.aircraft() && !hidden.unit_figures());
        let (mut drawn, mut undrawn) = (World::new(), World::new());
        let mut trains_ran = false;
        let mut particles_ran = false;
        for frame in 0..=400 {
            let time = frame as f32 / 10.0;
            // The hour changes, with another allocation, while one scene hides everything.
            if frame == 200 {
                drawn.next_hour();
                undrawn.next_hour();
            }
            drawn.step(&shown, time);
            undrawn.step(&hidden, time);
            assert_eq!(drawn.state(), undrawn.state(), "frame {frame}");
            trains_ran |= drawn.trains.2.live_trains() > 0;
            // The same events reach both scenes' simulations; only the hidden marks differ.
            let keys = |world: &World| {
                world
                    .events
                    .iter()
                    .map(|event| event.key.without_pass())
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(&drawn), keys(&undrawn), "frame {frame}");
            assert!(drawn.events.iter().all(|event| !event.hidden));
            assert!(undrawn.events.iter().all(|event| event.hidden));
            particles_ran |= !drawn.events.is_empty()
                && drawn
                    .simulations
                    .presentation_state()
                    .contains("particles=[(");
        }
        assert!(
            trains_ran,
            "the fixture runs no train, so it proves nothing"
        );
        assert!(
            particles_ran,
            "the fixture emits no particle, so it proves nothing"
        );
    }
}
