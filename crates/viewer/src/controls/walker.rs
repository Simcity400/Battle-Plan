
use super::{Frame, approach};
use dashboard_core::protocol::GamepadButton;
use eframe::egui::Key;
use std::f32::consts::{PI, TAU};
use viewer_gpu::camera::{Camera, FOV_Y, Free, Lens, Pose, WATER_HEIGHT};
use viewer_gpu::province_map::HEIGHT_SCALE;
use viewer_models::entity_animation::{IDLE, MOVE};
use viewer_models::ModelInstance;
use viewer_unit_history::unit_history::MESH_FORWARD_OFFSET;

pub const WALK_PACE: f32 = 1.0;
pub const FLY_PACE: f32 = 2.5;
pub const BOOST: f32 = 3.0;
pub const GRAVITY: f32 = 5.4;
pub const TERMINAL_SPEED: f32 = 30.0;
pub const EASE: f32 = 0.15;
pub const TURN: f32 = 0.1;
pub const STANDING: f32 = 0.2;
pub const DOUBLING: f32 = 1.0;
pub const MOUSE_LOOK: f32 = 0.0022;
pub const STICK_LOOK: [f32; 2] = [2.2, 1.5];
pub const VIEW_PITCH: [f32; 2] = [-1.45, 0.6];
pub const VIEW_PITCH_START: f32 = -0.25;
pub const VIEW_DISTANCE: [f32; 2] = [1.5, 12.0];
pub const VIEW_DISTANCE_START: f32 = 4.0;
pub const WHEEL_ZOOM: f32 = 0.002;
pub const BUMPER_ZOOM: f32 = 0.7;
pub const LOOK_AT: f32 = 0.75;
pub const NEAR_SHARE: f32 = 0.25;
pub const NEAREST_SHARE: f32 = 0.1;
pub const CLEARANCE: f32 = 2.0;
pub const FAR: f32 = 20_000.0;
pub const FALLBACK_FIGURE: f32 = 1.0;
pub const ANIMATION_SEED: u32 = 0x5741_4c4b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    Ground,
    Flying,
    Falling,
}

pub struct World<'a> {
    pub ground: &'a dyn Fn(f32, f32) -> f32,
    pub map: [f32; 2],
    pub ceiling: f32,
}

impl World<'_> {
    pub fn of<'a>(camera: &Camera, ground: &'a dyn Fn(f32, f32) -> f32) -> World<'a> {
        World {
            ground,
            map: [camera.map.x(), camera.map.y()],
            ceiling: camera.defines.heights[1],
        }
    }

    fn floor(&self, x: f32, z: f32) -> f32 {
        (self.ground)(x, z).max(WATER_HEIGHT)
    }
}

pub fn scale_limits(figure: f32, ceiling: f32) -> [f32; 2] {
    let figure = figure.max(f32::EPSILON);
    let smallest = HEIGHT_SCALE / figure;
    let largest = (ceiling - WATER_HEIGHT) / (figure * (1.0 + VIEW_DISTANCE[1]));
    [smallest, largest.max(smallest)]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    pub feet: [f32; 3],
    pub facing: f32,
    pub view_yaw: f32,
    pub view_pitch: f32,
    pub distance: f32,
    pub scale: f32,
    pub gait: Gait,
    velocity: [f32; 2],
    climb: f32,
    turn: [f32; 2],
}

impl Walker {
    pub fn standing([x, z]: [f32; 2], yaw: f32, scale: f32, world: &World<'_>) -> Self {
        Self {
            feet: [x, world.floor(x, z), z],
            facing: yaw,
            view_yaw: yaw,
            view_pitch: VIEW_PITCH_START,
            distance: VIEW_DISTANCE_START,
            scale,
            gait: Gait::Ground,
            velocity: [0.0; 2],
            climb: 0.0,
            turn: [0.0; 2],
        }
    }

    fn height(&self, figure: f32) -> f32 {
        figure * self.scale
    }

    pub fn update(&mut self, frame: &Frame, wheel: f32, figure: f32, world: &World<'_>) {
        let dt = frame.dt;
        let pad = &frame.pad;
        if frame.double_tapped(Key::Space) || pad.double_tapped(GamepadButton::South) {
            self.gait = match self.gait {
                Gait::Ground | Gait::Falling => Gait::Flying,
                Gait::Flying => Gait::Falling,
            };
            self.climb = 0.0;
        }
        let grow = f32::from(
            i8::from(frame.held(Key::Plus) || frame.held(Key::Equals))
                - i8::from(frame.held(Key::Minus)),
        ) + f32::from(
            i8::from(pad.held(GamepadButton::DpadUp)) - i8::from(pad.held(GamepadButton::DpadDown)),
        );
        let [smallest, largest] = scale_limits(figure, world.ceiling);
        self.scale =
            (self.scale * (grow.clamp(-1.0, 1.0) * dt / DOUBLING).exp2()).clamp(smallest, largest);
        self.turn = [0, 1].map(|axis| {
            let rate = approach(
                self.turn[axis],
                pad.right[axis] * STICK_LOOK[axis],
                EASE,
                dt,
            );
            if rate.abs() < 1e-4 { 0.0 } else { rate }
        });
        self.view_yaw = wrap_angle(self.view_yaw + frame.look[0] * MOUSE_LOOK + self.turn[0] * dt);
        self.view_pitch = (self.view_pitch - frame.look[1] * MOUSE_LOOK + self.turn[1] * dt)
            .clamp(VIEW_PITCH[0], VIEW_PITCH[1]);
        if pad.pressed & GamepadButton::RightStick.bit() != 0 {
            self.view_yaw = self.facing;
            self.view_pitch = VIEW_PITCH_START;
        }
        let bumpers = f32::from(
            i8::from(pad.held(GamepadButton::RightBumper))
                - i8::from(pad.held(GamepadButton::LeftBumper)),
        );
        self.distance = (self.distance * (bumpers * BUMPER_ZOOM * dt - wheel * WHEEL_ZOOM).exp())
            .clamp(VIEW_DISTANCE[0], VIEW_DISTANCE[1]);
        let height = self.height(figure);
        let across = (frame.axis(Key::A, Key::D) + pad.left[0]).clamp(-1.0, 1.0);
        let ahead = (frame.axis(Key::S, Key::W) + pad.left[1]).clamp(-1.0, 1.0);
        let mut pace = match self.gait {
            Gait::Ground | Gait::Falling => WALK_PACE,
            Gait::Flying => FLY_PACE,
        } * height;
        if frame.modifiers.ctrl || pad.held(GamepadButton::LeftStick) {
            pace *= BOOST;
        }
        let (sin, cos) = self.view_yaw.sin_cos();
        let target = [
            (across * cos + ahead * sin) * pace,
            (-across * sin + ahead * cos) * pace,
        ];
        self.velocity = [0, 1].map(|axis| {
            let value = approach(self.velocity[axis], target[axis], EASE, dt);
            if value.abs() < 1e-3 * height {
                0.0
            } else {
                value
            }
        });
        if across != 0.0 || ahead != 0.0 {
            let toward = target[0].atan2(target[1]);
            self.facing =
                wrap_angle(self.facing + approach(0.0, wrap_angle(toward - self.facing), TURN, dt));
        }
        self.feet[0] = (self.feet[0] + self.velocity[0] * dt).rem_euclid(world.map[0].max(1.0));
        self.feet[2] = (self.feet[2] + self.velocity[1] * dt).clamp(0.0, world.map[1]);
        let floor = world.floor(self.feet[0], self.feet[2]);
        match self.gait {
            Gait::Ground => {
                self.climb = 0.0;
                self.feet[1] = floor;
            }
            Gait::Flying => {
                let up = (f32::from(
                    i8::from(frame.held(Key::Space) || pad.held(GamepadButton::South))
                        - i8::from(frame.modifiers.shift || pad.held(GamepadButton::East)),
                ) + pad.triggers[1]
                    - pad.triggers[0])
                    .clamp(-1.0, 1.0);
                self.climb = approach(self.climb, up * pace, EASE, dt);
                self.feet[1] += self.climb * dt;
                if self.feet[1] <= floor {
                    self.feet[1] = floor;
                    if up < 0.0 {
                        self.gait = Gait::Ground;
                        self.climb = 0.0;
                    }
                }
            }
            Gait::Falling => {
                self.climb = (self.climb - GRAVITY * height * dt).max(-TERMINAL_SPEED * height);
                self.feet[1] += self.climb * dt;
                if self.feet[1] <= floor {
                    self.feet[1] = floor;
                    self.gait = Gait::Ground;
                    self.climb = 0.0;
                }
            }
        }
        let top = (world.ceiling - height * (1.0 + self.distance)).max(floor);
        if self.feet[1] > top {
            self.feet[1] = top;
            self.climb = self.climb.min(0.0);
        }
    }

    pub fn walking(&self, figure: f32) -> bool {
        self.gait == Gait::Ground
            && self.velocity[0].hypot(self.velocity[1]) > STANDING * WALK_PACE * self.height(figure)
    }

    pub fn instance(&self, figure: f32) -> ModelInstance {
        let walking = self.walking(figure);
        let (east, north) = self.facing.sin_cos();
        ModelInstance {
            position: self.feet,
            rotation: north.atan2(east) + MESH_FORWARD_OFFSET,
            scale: self.scale,
            native_flags: 1 | (u32::from(walking) << 1),
            animation: if walking { MOVE } else { IDLE },
            animation_seed: ANIMATION_SEED,
            animation_identity: ANIMATION_SEED,
            ..ModelInstance::default()
        }
    }

    pub fn camera(&self, base: Camera, figure: f32, world: &World<'_>) -> Camera {
        let height = self.height(figure);
        let target = [self.feet[0], self.feet[1] + LOOK_AT * height, self.feet[2]];
        let reach = self.distance * height;
        let forward = Free::new(self.view_yaw, self.view_pitch, Lens::MAP).forward();
        let mut eye = std::array::from_fn::<f32, 3, _>(|axis| target[axis] - forward[axis] * reach);
        let edge = eye[2].clamp(0.0, world.map[1]);
        if edge != eye[2] {
            eye[2] = edge;
            let across = (eye[0] - target[0]).hypot(eye[2] - target[2]);
            let up = (reach * reach - across * across).max(0.0).sqrt();
            eye[1] = target[1] + if forward[1] > 0.0 { -up } else { up };
        }
        let ground = world.floor(eye[0], eye[2]);
        let near = NEAR_SHARE * (eye[1] - ground).clamp(NEAREST_SHARE * reach, reach);
        let low = ground + near * CLEARANCE;
        eye[1] = eye[1].max(low).min(world.ceiling.max(low));
        let lens = Lens {
            fov_y: FOV_Y,
            near,
            far: FAR,
        };
        let mut camera = Camera {
            center: [eye[0], eye[2]],
            height: eye[1],
            pose: Pose::Free(Free::looking_at(eye, target, lens)),
            ..base
        };
        camera.hold_free(world.ground, near * CLEARANCE);
        camera
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Start {
    pub at: [f32; 2],
    pub facing: f32,
    pub scale: f32,
    pub flying: bool,
    pub held: Vec<Key>,
    pub modifiers: eframe::egui::Modifiers,
}

impl Start {
    pub fn parse(place: &str, keys: Option<&str>, flying: bool) -> Result<Self, String> {
        const USAGE: &str = "--walk takes x,z,heading[,scale]: world x and z, the heading in \
                             degrees clockwise from north and a positive scale (default 1)";
        let values = place
            .split(',')
            .map(|value| value.trim().parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| USAGE.to_owned())?;
        if !(3..=4).contains(&values.len()) || !values.iter().all(|value| value.is_finite()) {
            return Err(USAGE.into());
        }
        let scale = values.get(3).copied().unwrap_or(1.0);
        if scale <= 0.0 {
            return Err(USAGE.into());
        }
        let mut start = Self {
            at: [values[0], values[1]],
            facing: values[2].to_radians(),
            scale,
            flying,
            ..Self::default()
        };
        for name in keys.into_iter().flat_map(|keys| keys.split(',')) {
            match name.trim().to_ascii_lowercase().as_str() {
                "w" => start.held.push(Key::W),
                "a" => start.held.push(Key::A),
                "s" => start.held.push(Key::S),
                "d" => start.held.push(Key::D),
                "space" => start.held.push(Key::Space),
                "plus" => start.held.push(Key::Plus),
                "minus" => start.held.push(Key::Minus),
                "shift" => start.modifiers.shift = true,
                "ctrl" => start.modifiers.ctrl = true,
                other => {
                    return Err(format!(
                        "--walk-keys takes w, a, s, d, space, shift, ctrl, plus and minus, not `{other}`"
                    ));
                }
            }
        }
        Ok(start)
    }

    pub fn frame(&self, dt: f32) -> Frame {
        Frame {
            dt,
            held: self.held.iter().copied().collect(),
            modifiers: self.modifiers,
            ..Frame::default()
        }
    }

    pub fn walker(&self, world: &World<'_>) -> Walker {
        let mut walker = Walker::standing(self.at, self.facing, self.scale, world);
        if self.flying {
            walker.gait = Gait::Flying;
        }
        walker
    }
}

fn wrap_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests;
