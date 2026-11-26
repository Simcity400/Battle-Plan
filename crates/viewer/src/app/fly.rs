
use super::Viewer;
use crate::controls::flight::{self, Airframe, Flight, View};
use crate::controls::walker::{FALLBACK_FIGURE, FLY_PACE, VIEW_PITCH_START, Walker, World, free_speed_limits};
use crate::controls::{Frame, Pad};
use crate::document::Document;
use viewer_gpu::camera::{Camera, WATER_HEIGHT};
use viewer_gpu::province_map::HeightField;
use viewer_aircraft::aircraft::PlayerAircraft;
use viewer_models::ModelInstance;
use viewer_world::scene::PlayerModel;

pub(crate) const LABEL: &str = "flying aircraft";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scripted {
    pub position: [f32; 3],
    pub attitude: [f32; 3],
    pub view: View,
    pub free: bool,
    pub steer: [f32; 3],
}

impl Scripted {
    pub(crate) fn frame(&self, dt: f32) -> Frame {
        let [turn, climb, throttle] = self.steer;
        Frame {
            dt,
            pad: Pad {
                left: [turn, climb],
                triggers: [(-throttle).max(0.0), throttle.max(0.0)],
                ..Pad::default()
            },
            ..Frame::default()
        }
    }

    fn flight(&self) -> Flight {
        let [yaw, pitch, roll] = self.attitude;
        let mut flight = Flight::new(self.position, yaw, pitch, roll);
        flight.view = self.view;
        flight
    }
}

pub(super) fn placed(
    aircraft: Option<usize>,
    sight: Option<ModelInstance>,
) -> Option<(usize, ModelInstance)> {
    aircraft.zip(sight)
}

pub(super) fn default_free_speed(height: f32, ground: f32, ceiling: f32) -> f32 {
    let [smallest, largest] = free_speed_limits(ceiling);
    let above = (height - ground).max(4.0);
    (above * 0.25 / FLY_PACE).clamp(smallest, largest)
}

fn airframe(document: &Document, aircraft: Option<usize>) -> Airframe {
    let ground = document
        .aircraft
        .ground_contact()
        .unwrap_or(Airframe::NONE.ground);
    let Some(slot) = aircraft else {
        return Airframe {
            ground,
            ..Airframe::NONE
        };
    };
    let library = document.aircraft.library();
    Airframe {
        size: library
            .meshes
            .get(slot)
            .map(|mesh| mesh.radius)
            .filter(|radius| radius.is_finite() && *radius > 0.0)
            .unwrap_or(flight::DEFAULT_SIZE),
        explodes: library.has_state(slot, flight::EXPLODE),
        ground,
    }
}

impl Viewer {
    pub(crate) fn fly_aircraft(&self, document: &Document) -> Option<&PlayerAircraft> {
        self.fly_aircraft.ready(&document.aircraft)
    }

    pub(super) fn fly_refusals(&self, document: &Document) -> Vec<String> {
        let mut refusals = Vec::new();
        if let Some(reason) = self.fly_aircraft.refusal() {
            refusals.push(format!(
                "Fly flies the free camera instead of the chosen aircraft: {reason}"
            ));
        }
        if let Some(flight) = &self.flight
            && let flight::Phase::Wrecked { .. } = flight.phase
        {
            refusals.push(
                "The flying mode's aircraft cannot fly again: no point of the map has room for it \
                 under the content's CAMERA_MAX_HEIGHT"
                    .to_owned(),
            );
        }
        if let Err(reason) = document.aircraft.ground_contact() {
            refusals.push(format!(
                "The flying mode's aircraft meets the drawn ground itself and goes at once when it \
                 crashes: {reason}"
            ));
        }
        refusals
    }

    pub(super) fn fly(
        &mut self,
        document: &Document,
        map: Camera,
        start: Option<[f32; 2]>,
        wheel: f32,
        controls: &Frame,
    ) -> Camera {
        let field = HeightField {
            width: document.map.width,
            height: document.map.height,
            values: document.assets.heightmap_texels(),
        };
        let floor = |x: f32, z: f32| field.surface_height(x, z);
        let scripted = self.options.fly;
        let ready = self.fly_aircraft(document).map(|aircraft| aircraft.slot);
        let pilot_aircraft = match scripted {
            Some(scripted) => !scripted.free,
            None => ready.is_some(),
        };
        if !pilot_aircraft {
            self.flying = None;
            let world = World::of(&map, &floor);
            let free = self.free_cam.get_or_insert_with(|| {
                if let Some(scripted) = scripted {
                    let g = floor(scripted.position[0], scripted.position[2]).max(WATER_HEIGHT);
                    let base = default_free_speed(scripted.position[1], g, world.ceiling);
                    Walker::free_camera(
                        scripted.position,
                        scripted.attitude[0],
                        scripted.attitude[1],
                        base,
                        base,
                        &world,
                    )
                } else if let Some(flight) = self.flight.take() {
                    let g = floor(flight.position[0], flight.position[2]).max(WATER_HEIGHT);
                    let base = default_free_speed(flight.position[1], g, world.ceiling);
                    let speed = self.free_cam_speed.unwrap_or(base);
                    Walker::free_camera(flight.position, flight.yaw, 0.0, base, speed, &world)
                } else if let Some((eye, yaw, pitch)) = self.mode_handover.take() {
                    let g = floor(eye[0], eye[2]).max(WATER_HEIGHT);
                    let base = default_free_speed(eye[1], g, world.ceiling);
                    let speed = self.free_cam_speed.unwrap_or(base);
                    Walker::free_camera(eye, yaw, pitch, base, speed, &world)
                } else {
                    let start_flight = Flight::from_map(
                        &start.map_or(map, |center| Camera { center, ..map }),
                        &floor,
                        Airframe::NONE.size,
                    );
                    let g = floor(start_flight.position[0], start_flight.position[2])
                        .max(WATER_HEIGHT);
                    let base = default_free_speed(start_flight.position[1], g, world.ceiling);
                    let speed = self.free_cam_speed.unwrap_or(base);
                    Walker::free_camera(
                        start_flight.position,
                        start_flight.yaw,
                        VIEW_PITCH_START,
                        base,
                        speed,
                        &world,
                    )
                }
            });
            free.update(controls, wheel, FALLBACK_FIGURE, &world);
            self.free_cam_speed = Some(free.scale);
            return free.camera(map, FALLBACK_FIGURE, &world);
        }
        let airframe = airframe(document, ready);
        let preferred_view = self.fly_view;
        let flight = self.flight.get_or_insert_with(|| match scripted {
            Some(scripted) => scripted.flight(),
            None => {
                let mut flight = if let Some(free) = self.free_cam.take() {
                    Flight::new(free.feet, free.view_yaw, 0.0, 0.0)
                } else if let Some((eye, yaw, _)) = self.mode_handover.take() {
                    Flight::new(eye, yaw, 0.0, 0.0)
                } else {
                    Flight::from_map(
                        &start.map_or(map, |center| Camera { center, ..map }),
                        &floor,
                        airframe.size,
                    )
                };
                flight.view = preferred_view;
                flight
            }
        });
        let played = ready.and_then(|slot| {
            self.scene
                .as_ref()?
                .player_play_position(flight.animation_key(slot))
        });
        flight.update(
            controls,
            &map,
            &floor,
            &airframe,
            played,
            &mut self.fly_footprints,
        );
        self.fly_view = flight.view;
        let sight = flight.sight(map, &floor, &airframe);
        self.flying = placed(ready, sight.aircraft);
        sight.camera
    }

    pub(super) fn fly_model<'a>(
        flying: Option<(usize, ModelInstance)>,
        library: &'a viewer_models::ModelLibrary,
    ) -> Option<PlayerModel<'a>> {
        flying.map(|(mesh, instance)| PlayerModel {
            library,
            label: LABEL,
            mesh,
            instance,
        })
    }

    pub(super) fn flight_line(&self, document: &Document) -> Option<String> {
        if let Some(free) = &self.free_cam {
            let [x, y, z] = free.feet;
            return Some(format!(
                "Flight: free camera at {x:.3},{y:.3},{z:.3}, heading {:.2}, pitch {:.2} degrees, \
                 speed x{:.2}",
                free.view_yaw.to_degrees(),
                free.view_pitch.to_degrees(),
                free.speed_multiplier(),
            ));
        }
        let flight = self.flight.as_ref()?;
        let [x, y, z] = flight.position;
        Some(format!(
            "Flight: {} at {x:.3},{y:.3},{z:.3}, heading {:.2}, pitch {:.2}, bank {:.2} degrees, \
             {:.2} units/s, {:?} view, {:?}, {} crashes",
            self.fly_aircraft(document)
                .map_or("no aircraft", |aircraft| aircraft.name.as_str()),
            flight.yaw.to_degrees(),
            flight.pitch.to_degrees(),
            flight.roll.to_degrees(),
            flight.speed,
            flight.view,
            flight.phase,
            flight.crashes,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
    fn the_host_s_controller_flies_the_aircraft_while_the_map_lacks_keyboard_focus()
    -> std::io::Result<()> {
        use crate::map::scene_failure_tests::Fixture;
        use dashboard_core::protocol::{GamepadButton, GamepadMessage, GamepadOwner, GamepadState};
        use eframe::egui;
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        let fixture = Fixture::load("fly-controller")?;
        let document = Document::from_shared_content(
            fixture.document.content.clone(),
            fixture.document.playback.replay_handle(),
            &crate::document::InitialHour {
                branch: Some(fixture.input.branch),
                hour: Some(fixture.input.hour),
                hour_fraction: 0.,
            },
        )?;
        let mut viewer = Viewer::create(
            Default::default(),
            Default::default(),
            None,
            Arc::new(Mutex::new(None)),
            None,
            Some((fixture.state.device.clone(), fixture.state.queue.clone())),
        );
        viewer.dashboard_map = true;
        viewer.diagnostics = false;
        viewer.graphics.samples = 1;
        viewer.install_map(document, fixture.scene()?, "fixture".into());
        let ctx = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        frame.wgpu_render_state = Some(fixture.state.clone());
        let target = super::super::render_failure_tests::target(&fixture);
        let draw = |viewer: &mut Viewer| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(256., 144.),
                    )),
                    focused: false,
                    ..Default::default()
                },
                |ui| viewer.map_ui(ui, &frame, &target),
            );
            output.textures_delta.clear();
        };
        let deadline = Instant::now() + Duration::from_secs(120);
        while viewer.rendered.is_none() {
            assert!(
                viewer.error.is_none() && viewer.map_error.is_none(),
                "{:?}",
                viewer.map_state(&target)
            );
            assert!(Instant::now() < deadline, "{:?}", viewer.map_state(&target));
            draw(&mut viewer);
            std::thread::sleep(Duration::from_millis(1));
        }
        viewer.set_camera_mode(dashboard_core::protocol::CameraMode::Fly, false, &ctx);
        let owner = GamepadOwner {
            viewer: 1,
            handoff: 1,
        };
        let mut state = GamepadState {
            sticks: [[0.0; 2], [0.0; 2]],
            triggers: [0.0; 2],
            buttons: 0,
        };
        viewer.input.gamepad(GamepadMessage::Take { owner, state });
        draw(&mut viewer);
        assert!(viewer.start_pick.choosing(), "{:?}", viewer.start_pick);
        state.buttons = GamepadButton::South.bit();
        viewer.input.gamepad(GamepadMessage::State {
            owner,
            state,
            pressed: 0,
        });
        draw(&mut viewer);
        assert!(viewer.flight.is_none(), "the flight waits for its start");
        assert!(viewer.start_pick.at().is_some(), "{:?}", viewer.start_pick);
        state = GamepadState {
            sticks: [[0.0, 1.0], [1.0, 0.0]],
            triggers: [0.0, 1.0],
            buttons: 0,
        };
        viewer.input.gamepad(GamepadMessage::State {
            owner,
            state,
            pressed: 0,
        });
        draw(&mut viewer);
        let start = viewer.free_cam.expect("the flying mode starts in the free camera");
        let begun = Instant::now();
        for _ in 0..10 {
            draw(&mut viewer);
        }
        while begun.elapsed() < Duration::from_millis(600) {
            draw(&mut viewer);
        }
        let flown = viewer.free_cam.expect("still flying in the free camera");
        let turned = (flown.view_yaw - start.view_yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        assert!(turned > 0.05, "{} -> {}", start.view_yaw, flown.view_yaw);
        assert!(flown.feet[1] > start.feet[1], "{} -> {}", start.feet[1], flown.feet[1]);
        Ok(())
    }
}
