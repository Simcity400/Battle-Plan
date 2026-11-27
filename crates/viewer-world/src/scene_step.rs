use super::{BUILDING_TILE, BuildingHistory, ModelPass, UnitGates, hidden_map_frame_actors};
use viewer_scene_base::line_lod::wrap_offsets;
use viewer_scene_base::line_lod::MARGIN;
use std::{collections::BTreeSet, sync::Arc};
use viewer_gpu::camera::Camera;

struct Part {
    name: &'static str,
    began: Option<(std::time::Instant, Option<std::time::Duration>)>,
}

impl Part {
    fn new(name: &'static str) -> Self {
        let began = viewer_runtime::view_record::on()
            .then(|| (std::time::Instant::now(), viewer_runtime::profiling::current_thread_cpu_time()));
        Self { name, began }
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        let Some((wall, cpu)) = self.began else {
            return;
        };
        use viewer_runtime::view_record::add;
        add("presentation_wall_us", self.name, wall.elapsed().as_micros() as u64);
        if let (Some(cpu), Some(now)) = (cpu, viewer_runtime::profiling::current_thread_cpu_time()) {
            add("presentation_cpu_us", self.name, now.saturating_sub(cpu).as_micros() as u64);
        }
    }
}

pub(super) struct Presentation<'a> {
    pub graphics: &'a dashboard_core::protocol::MapGraphics,
    pub trains: Option<&'a mut (
        ModelPass,
        Arc<viewer_rail::trains::TrainAssets>,
        viewer_rail::trains::TrainPlan,
    )>,
    pub unit_history: &'a mut viewer_unit_history::unit_history::History,
    pub building_history: &'a mut BuildingHistory,
    pub map_buildings: Option<&'a mut ModelPass>,
    pub unit_models: Option<&'a mut ModelPass>,
    pub unit_gates: Option<UnitGates>,
    pub visibility: viewer_foundation::mode_visibility::Visibility,
    pub mesh_limit: i32,
    pub movement_arrows: Option<&'a mut crate::movement_arrows::Player>,
    pub animated_text: Option<&'a mut viewer_map_text::animated_map_text::Player>,
    pub strikes: &'a mut viewer_models::strike_effects::Presentation,
    pub missile_clock: &'a mut Option<f32>,
    pub map_frame: Option<&'a mut ModelPass>,
    pub ambient: Option<&'a mut crate::mapframe::Presentation>,
    pub transition_time: f32,
    pub transition_delta: f32,
    pub transition_reset: bool,
    pub scale_delta: Option<f32>,
    pub province_frame: Option<&'a viewer_gpu::province_schedule::ProvinceFrame>,
    pub pose_tick: bool,
}

impl Presentation<'_> {
    #[expect(clippy::mutable_key_type, reason = "actor keys cache display data outside equality and ordering")]
    pub(super) fn step(
        self,
        camera: Camera,
        size: [u32; 2],
        animation_time: f32,
        rails_shown: bool,
        beside: impl FnOnce(f32) -> Option<Vec<viewer_models::strike_effects::Impact>>,
    ) -> BTreeSet<viewer_models::ActorAddress> {
        let Self {
            graphics,
            trains,
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
            transition_time,
            transition_delta,
            transition_reset,
            scale_delta,
            province_frame,
            pose_tick,
        } = self;
        let view_span = viewer_runtime::profiling::span("step_view");
        let visible = camera.visible_ground(size, MARGIN);
        let animation_offsets = wrap_offsets(visible, camera.map.x());
        drop(view_span);
        let walk = || {
            let _walk = Part::new("walk");
            if let Some((pass, assets, plan)) = trains
                && rails_shown
            {
                let _part = Part::new("trains");
                let _trains_span = viewer_runtime::profiling::span("step_trains");
                let field_span = viewer_runtime::profiling::span("train_field");
                pass.replace_field(&assets.library, plan.field(assets, animation_time));
                drop(field_span);
                if pose_tick {
                    let _animate_span = viewer_runtime::profiling::span("train_animate");
                    pass.animate(animation_time, visible, &animation_offsets);
                }
            }
            let mut gates = unit_gates.unwrap_or(UnitGates::OFF);
            if let Some(arrows) = &movement_arrows {
                gates.arrows = arrows.cutoff();
            }
            let gates = gates.for_mode(&visibility);
            let history_part = Part::new("history");
            let history_profile = viewer_runtime::profiling::thread_span("history_update");
            unit_history.set_transition_step(scale_delta, Some(transition_time));
            let models = unit_history.update_models_with_frame(
                camera,
                size,
                animation_time,
                gates,
                mesh_limit,
                building_history,
                true,
                province_frame,
            );
            drop(history_profile);
            drop(history_part);
            if let Some(arrows) = movement_arrows {
                let _part = Part::new("arrows");
                let _arrows_span = viewer_runtime::profiling::span("step_arrows");
                arrows.advance(
                    transition_delta,
                    transition_reset,
                    camera.height,
                    |province| unit_history.arrows_admitted(province),
                );
            }
            if let Some(text) = animated_text {
                let _part = Part::new("text");
                let _text_span = viewer_runtime::profiling::span("step_text");
                text.advance(camera, transition_time, province_frame);
            }
            if let Some(pass) = map_buildings {
                let _part = Part::new("buildings");
                let _buildings_span = viewer_runtime::profiling::span("step_buildings");
                if viewer_runtime::profile_writer::profiling() {
                    viewer_runtime::profiling::record_count(
                        "building_field_reused",
                        u64::from(models.buildings.is_none()),
                    );
                    viewer_runtime::profiling::record_count(
                        "building_field_updated",
                        u64::from(models.buildings.is_some()),
                    );
                }
                if let Some(placed) = models.buildings {
                    let _place_span = viewer_runtime::profiling::span("building_place");
                    pass.update_placed(placed, BUILDING_TILE);
                }
                if pose_tick {
                    let _animate_span = viewer_runtime::profiling::span("building_animate");
                    let _animate_profile =
                        viewer_runtime::profiling::thread_span("building_animate");
                    pass.animate(animation_time, visible, &animation_offsets);
                }
            }
            if let Some(pass) = unit_models {
                let _part = Part::new("units");
                let _units_span = viewer_runtime::profiling::span("step_units");
                let place_span = viewer_runtime::profiling::span("unit_place");
                let place_profile = viewer_runtime::profiling::thread_span("unit_place");
                viewer_runtime::view_record::add(
                    "unit walk",
                    "placed",
                    models.units.standing.as_ref().map_or(0, Vec::len) as u64,
                );
                pass.update_moving(models.units, viewer_unit_models::unit_models::UNIT_TILE);
                drop(place_profile);
                drop(place_span);
                let clocks_span = viewer_runtime::profiling::span("unit_clocks");
                pass.set_clocks(unit_history.combat_clocks());
                drop(clocks_span);
                if pose_tick {
                    let _animate_span = viewer_runtime::profiling::span("unit_animate");
                    let _animate_profile = viewer_runtime::profiling::thread_span("unit_animate");
                    pass.animate(animation_time, None, &[0.0]);
                }
            }
        };
        let joined = Part::new("join");
        walk();
        let explosions = beside(missile_clock.unwrap_or(animation_time));
        drop(joined);
        if let Some(explosions) = explosions {
            strikes.receive_explosions(&explosions, animation_time);
        }
        *missile_clock = Some(animation_time);
        let mut struck = BTreeSet::new();
        if let (Some(pass), Some(ambient)) = (map_frame, ambient) {
            let _part = Part::new("map_frame");
            let _map_frame_span = viewer_runtime::profiling::span("step_map_frame");
            let ambient_span = viewer_runtime::profiling::span("ambient_update");
            let mut placed = ambient.update(camera, size, animation_time);
            drop(ambient_span);
            let strike_span = viewer_runtime::profiling::span("strike_update");
            let (impacts, clocks) = strikes.update(camera, size, animation_time, |key| {
                pass.root_play_position(key)
            });
            drop(strike_span);
            let key = |(mesh, instance): &(usize, viewer_models::ModelInstance)| {
                let (seed, identity) = instance.animation_key();
                (*mesh, seed, identity)
            };
            let place_span = viewer_runtime::profiling::span("map_frame_place");
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
            drop(place_span);
            let clocks_span = viewer_runtime::profiling::span("map_frame_clocks");
            pass.set_clocks(clocks);
            drop(clocks_span);
            if pose_tick {
                let _animate_span = viewer_runtime::profiling::span("map_frame_animate");
                pass.animate(animation_time, None, &[0.]);
            }
            struck.extend(pass.clock_actor_addresses());
        }
        struck
    }
}

pub(super) struct Beside {
    pub graphics: dashboard_core::protocol::MapGraphics,
    pub aircraft: Option<(
        ModelPass,
        Arc<viewer_aircraft::aircraft::AircraftAssets>,
        viewer_aircraft::aircraft::AircraftPlan,
    )>,
    pub sprawl: Option<ModelPass>,
    pub city_presentation: Option<viewer_buildings::sprawl::Presentation>,
    pub camera: Camera,
    pub size: [u32; 2],
    pub animation_time: f32,
    pub pose_tick: bool,
    pub records: Option<viewer_runtime::profiling::ThreadRecords>,
}

impl Beside {
    pub(super) fn step(mut self) -> Self {
        let Self { graphics, camera, size, animation_time, pose_tick, .. } = self;
        if let Some((pass, assets, plan)) = self.aircraft.as_mut() {
            let _part = Part::new("aircraft");
            let _aircraft_span = viewer_runtime::profiling::span("step_aircraft");
            let field_span = viewer_runtime::profiling::span("aircraft_field");
            let (mut field, hidden) = plan.field(
                assets,
                animation_time,
                &|mission| graphics.air_mission(mission),
                &|key| pass.root_play_position(key),
            );
            drop(field_span);
            let place_span = viewer_runtime::profiling::span("aircraft_place");
            pass.replace_field(&assets.library(), &mut field);
            pass.set_hidden(hidden);
            drop(place_span);
            if pose_tick {
                let _animate_span = viewer_runtime::profiling::span("aircraft_animate");
                let _animate_profile = viewer_runtime::profiling::thread_span("aircraft_animate");
                pass.animate(animation_time, None, &[0.]);
            }
        }
        let (sprawl, city_presentation) = (self.sprawl.as_mut(), self.city_presentation.as_mut());
        if let (Some(pass), Some(presentation)) = (sprawl, city_presentation) {
            let _part = Part::new("cities");
            let _cities_span = viewer_runtime::profiling::span("step_cities");
            let update_span = viewer_runtime::profiling::span("city_update");
            let placed = presentation.update(
                camera,
                size,
                viewer_models::draws_city_sprawl(&camera),
                animation_time,
            );
            viewer_runtime::view_record::add(
                "city sprawl",
                if placed.is_some() { "field_rebuilt" } else { "field_kept" },
                1,
            );
            if let Some(placed) = placed {
                let _place_span = viewer_runtime::profiling::span("city_place");
                viewer_runtime::view_record::add("city sprawl", "placed", placed.len() as u64);
                pass.update_placed(placed, viewer_buildings::sprawl::SPRAWL_TILE);
            }
            drop(update_span);
            let clocks_span = viewer_runtime::profiling::span("city_clocks");
            pass.set_clock_records(presentation.clock_records(), animation_time);
            drop(clocks_span);
            if pose_tick {
                let _animate_span = viewer_runtime::profiling::span("city_animate");
                let _animate_profile = viewer_runtime::profiling::thread_span("city_animate");
                pass.animate(animation_time, None, &[0.0]);
            }
        }
        self.records = viewer_runtime::profiling::take_thread_records();
        self
    }
}

pub(super) struct EventSources<'a> {
    pub map_frame: Option<&'a ModelPass>,
    pub sprawl: Option<&'a ModelPass>,
    pub map_buildings: Option<&'a ModelPass>,
    pub unit_models: Option<&'a ModelPass>,
    pub aircraft: Option<&'a ModelPass>,
    pub trains: Option<&'a ModelPass>,
    pub player: Option<&'a ModelPass>,
}

pub(super) fn frame_events(
    sources: EventSources<'_>,
    graphics: &dashboard_core::protocol::MapGraphics,
    camera: &Camera,
    animation_time: f32,
    rails_shown: bool,
) -> Vec<viewer_models::entity_animation::Event> {
    let mut events = Vec::new();
    if animation_time <= 0.0 {
        return events;
    }
    let mut add =
        |pass: Option<&ModelPass>, layer_hidden: bool| {
            if let Some(pass) = pass {
                events.extend(pass.events.iter().map(|event| {
                    viewer_models::entity_animation::Event {
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
    if viewer_aircraft::aircraft::draws(camera) {
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
    use viewer_rail::trains::{TrainAssets, TrainPlan};
    use dashboard_core::protocol::MapGraphics;
    use viewer_gpu::camera::{MapDefines, MapSize, Placement};
    use viewer_runtime::presentation::Continuity;

    struct World {
        trains: (ModelPass, Arc<TrainAssets>, TrainPlan),
        unit_models: ModelPass,
        simulations: viewer_weather_effects::weather_effects::EntitySimulations,
        events: Vec<viewer_models::entity_animation::Event>,
        unit_history: viewer_unit_history::unit_history::History,
        building_history: BuildingHistory,
        strikes: viewer_models::strike_effects::Presentation,
        missile_clock: Option<f32>,
        last: Option<f32>,
    }

    impl World {
        fn new() -> Self {
            use viewer_unit_history::unit_history::{Kind, test_support::target};
            let assets = Arc::new(viewer_rail::trains::test_support::assets());
            let mut rig = viewer_models::entity_animation::stage_rig(false);
            rig.nodes[0].clips[0].animation = None;
            rig.nodes[0].clips[0]
                .state
                .events
                .push(content_reader_mesh::mesh::EntityEvent {
                    particle: Some("wake".into()),
                    keep_particle: true,
                    ..Default::default()
                });
            let bounds = Arc::new(viewer_gpu::province_admission::edge_boxes());
            let mut unit_history = viewer_unit_history::unit_history::History::default();
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
                    viewer_rail::trains::test_support::plan(),
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
            let transition_delta = self.last.map_or(0.0, |last| (time - last).max(0.0));
            let transition_reset = self.last.is_some_and(|last| time < last);
            Presentation {
                graphics,
                trains: Some(&mut self.trains),
                unit_history: &mut self.unit_history,
                building_history: &mut self.building_history,
                map_buildings: None,
                unit_models: Some(&mut self.unit_models),
                unit_gates: Some(viewer_unit_history::unit_history::test_support::GATES),
                visibility: viewer_foundation::mode_visibility::Visibility::POLITICAL,
                mesh_limit: i32::MAX,
                movement_arrows: None,
                animated_text: None,
                strikes: &mut self.strikes,
                missile_clock: &mut self.missile_clock,
                map_frame: None,
                ambient: None,
                transition_time: time,
                transition_delta,
                transition_reset,
                scale_delta: None,
                province_frame: None,
                pose_tick: true,
            }
            .step(camera, [1024, 768], time, true, |_| None);
            self.last = Some(time);
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
            let emitter = viewer_weather_effects::weather_effects::test_support::long_lived_test_emitter();
            let entities = std::collections::BTreeMap::from([("wake".into(), (1., vec![emitter]))]);
            let (events, _) = viewer_weather_effects::weather_effects::advance_entity_effects(
                &mut self.simulations,
                &entities,
                &viewer_weather_effects::entity_effects::Lights::default(),
                events,
            )
            .unwrap();
            self.events = events;
        }

        fn next_hour(&mut self) {
            let mut next = viewer_rail::trains::test_support::next_hour_plan();
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

    #[test]
    fn coarse_animation_scopes_are_profile_only_and_preserve_state() {
        const ARM: &str = "BATTLEPLAN_TEST_ANIMATION_PROFILE_ARM";
        let Some(arm) = std::env::var_os(ARM) else {
            let name = concat!(
                module_path!(),
                "::coarse_animation_scopes_are_profile_only_and_preserve_state"
            );
            let mut states = Vec::new();
            for enabled in [false, true] {
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child
                    .args(["--exact", name.split_once("::").unwrap().1, "--nocapture"])
                    .env(ARM, if enabled { "1" } else { "0" })
                    .env_remove("BATTLEPLAN_VIEWER_PROFILE")
                    .env_remove("BATTLEPLAN_ANIMATION_PROFILE");
                if enabled {
                    child.env("BATTLEPLAN_VIEWER_PROFILE", "1");
                }
                let output = child.output().unwrap();
                assert!(output.status.success(), "{output:?}");
                let stdout = String::from_utf8(output.stdout).unwrap();
                let arm_states: Vec<_> = stdout
                    .lines()
                    .filter_map(|line| line.strip_prefix("ANIMATION_PROFILE_STATE "))
                    .map(str::to_owned)
                    .collect();
                assert_eq!(arm_states.len(), 4, "{stdout}");
                states.push(arm_states);
            }
            assert_eq!(states[0], states[1]);
            return;
        };
        let enabled = arm == "1";
        assert_eq!(viewer_runtime::profile_writer::profiling(), enabled);
        let mut world = World::new();
        for frame in 0..4 {
            viewer_runtime::profiling::begin_thread_frame();
            world.step(&MapGraphics::default(), frame as f32 / 10.0);
            let line = viewer_runtime::profiling::take_thread_spans();
            for stage in ["history_update", "unit_place", "unit_animate"] {
                let calls = format!("mt_{stage}_calls=1");
                assert_eq!(line.split_whitespace().any(|field| field == calls), enabled);
                for key in [format!("mt_{stage}="), format!("mt_{stage}_cpu=")] {
                    let value = line.split_whitespace().find_map(|field| field.strip_prefix(&key));
                    if enabled {
                        let value = value.unwrap();
                        if value != "unavailable" {
                            let value: f64 = value.parse().unwrap();
                            assert!(value.is_finite() && value >= 0.0);
                        }
                    } else {
                        assert!(value.is_none());
                    }
                }
            }
            for stage in ["aircraft_animate", "building_animate", "city_animate", "lights"] {
                assert!(!line.contains(&format!("mt_{stage}=")));
            }
            assert!(!line.contains("building_field_reused="));
            println!("ANIMATION_PROFILE_STATE {}", world.state());
        }
    }

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
            if frame == 200 {
                drawn.next_hour();
                undrawn.next_hour();
            }
            drawn.step(&shown, time);
            undrawn.step(&hidden, time);
            assert_eq!(drawn.state(), undrawn.state(), "frame {frame}");
            trains_ran |= drawn.trains.2.live_trains() > 0;
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
