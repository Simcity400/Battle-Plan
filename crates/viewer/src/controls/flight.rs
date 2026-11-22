
use super::{Frame, approach};
use dashboard_core::protocol::GamepadButton;
use eframe::egui::Key;
use std::f32::consts::{PI, TAU};
use viewer_gpu::camera::{Camera, Free, Lens, PITCH_LIMIT, Pose, WATER_HEIGHT};
use viewer_aircraft::aircraft::GroundContact;
use viewer_models::ModelInstance;

pub const SLOWEST: f32 = 4.0;
pub const FASTEST: f32 = 80.0;
pub const START_THROTTLE: f32 = 0.2;
pub const THROTTLE_RATE: f32 = 0.5;
pub const BOOST: f32 = 3.0;
pub const SPEED_EASE: f32 = 0.8;

pub const MOUSE_AIM: f32 = 0.0025;
pub const STEER_RATE: [f32; 2] = [1.3, 1.0];
pub const MAX_LEAD: f32 = 1.4;
pub const MAX_PITCH: f32 = 1.2;

pub const TURN_RATE: f32 = 1.0;
pub const TURN_GAIN: f32 = 2.0;
pub const PITCH_RATE: f32 = 0.9;
pub const PITCH_GAIN: f32 = 3.0;
pub const MAX_BANK: f32 = 1.05;
pub const BANK_EASE: f32 = 0.25;

pub const CLEARANCE_PER_SIZE: f32 = 1.5;
pub const LEAST_CLEARANCE: f32 = 0.3;
pub const DEFAULT_SIZE: f32 = 1.0;
pub const LONGEST_SUBSTEP: f32 = 1.0 / 120.0;
pub const LONGEST_STRIDE: f32 = 0.5;

pub const RESPAWN_AFTER: f32 = 2.5;
pub const RESPAWN_AHEAD: f32 = 3.0;

pub const CHASE_DISTANCE: f32 = 4.0;
pub const CHASE_RISE: f32 = 1.0;
pub const CHASE_LEAD: f32 = 3.0;
pub const CHASE_EASE: f32 = 0.12;
pub const LOOK_RANGE: [f32; 2] = [2.6, 0.9];
pub const LOOK_EASE: f32 = 0.15;
pub const NOSE: f32 = 1.0;

pub const FOV_Y: f32 = 0.96;
pub const FAR: f32 = 20_000.0;
pub const NEAR_PER_SIZE: f32 = 0.2;
pub const NEAR_PER_HEIGHT: f32 = 0.05;
pub const NEAREST: f32 = 0.02;
pub const FARTHEST_NEAR: f32 = 2.0;
pub const CAMERA_CLEARANCE: f32 = 3.0;

pub const START_HEIGHT_SHARE: f32 = 0.25;
pub const START_CLEARANCES: f32 = 4.0;

pub const ANIMATION_SEED: u32 = 0x464c_5931;
pub const EXPLODE: &str = "explode";

pub type Floor<'a> = &'a dyn Fn(f32, f32) -> f32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Chase,
    Nose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Airframe {
    pub size: f32,
    pub explodes: bool,
    pub ground: GroundContact,
}

impl Airframe {
    pub const NONE: Self = Self {
        size: DEFAULT_SIZE,
        explodes: false,
        ground: GroundContact {
            offset: 0.0,
            explosion: 0.0,
        },
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Flying,
    Crashed {
        seconds: f32,
        eye: [f32; 3],
        exploded: bool,
    },
    Wrecked {
        eye: [f32; 3],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Flight {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub aim: [f32; 2],
    pub speed: f32,
    pub throttle: f32,
    pub view: View,
    pub phase: Phase,
    pub crashes: u32,
    look: [f32; 2],
    chase: Option<[f32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    pub camera: Camera,
    pub aircraft: Option<ModelInstance>,
}

impl Flight {
    pub fn new(position: [f32; 3], yaw: f32, pitch: f32, roll: f32) -> Self {
        let throttle = START_THROTTLE;
        Self {
            position,
            yaw: wrap(yaw),
            pitch: pitch.clamp(-MAX_PITCH, MAX_PITCH),
            roll,
            aim: [wrap(yaw), pitch.clamp(-MAX_PITCH, MAX_PITCH)],
            speed: cruise(throttle),
            throttle,
            view: View::Chase,
            phase: Phase::Flying,
            crashes: 0,
            look: [0.0; 2],
            chase: None,
        }
    }

    pub fn from_map(map: &Camera, floor: Floor<'_>, size: f32) -> Self {
        let forward = map.look_direction();
        let target = match map.free() {
            Some(_) => map.eye(),
            None => map.look_at(),
        };
        let [x, z] = [target[0], target[2]];
        let ground = ground(floor, map.map.x(), x, z);
        let above = (map.eye()[1] - ground) * START_HEIGHT_SHARE;
        let low = ground + clearance(size);
        let height = (ground + above.max(clearance(size) * START_CLEARANCES))
            .min(map.defines.heights[1].max(low));
        Self::new(
            [
                x.rem_euclid(map.map.x().max(1.0)),
                height,
                z.clamp(0.0, map.map.y()),
            ],
            forward[0].atan2(forward[2]),
            0.0,
            0.0,
        )
    }

    pub fn update(
        &mut self,
        frame: &Frame,
        map: &Camera,
        floor: Floor<'_>,
        airframe: &Airframe,
        played: Option<Option<(f32, f32)>>,
        footprints: &mut Footprints,
    ) {
        let dt = frame.dt;
        let width = map.map.x().max(1.0);
        match &mut self.phase {
            Phase::Flying => {}
            Phase::Crashed {
                seconds, exploded, ..
            } => {
                *seconds += dt;
                *exploded = *exploded || !airframe.explodes || airframe.ground.exploded(played);
                if *exploded && *seconds >= RESPAWN_AFTER {
                    self.respawn(map, floor, airframe, footprints);
                }
                return;
            }
            Phase::Wrecked { .. } => return,
        }
        let size = airframe.size;
        let pad = &frame.pad;
        if frame.pressed.contains(&Key::V) || pad.pressed(GamepadButton::North) {
            self.view = match self.view {
                View::Chase => View::Nose,
                View::Nose => View::Chase,
            };
        }
        let free_look = frame.held(Key::C);
        let mouse = if free_look { [0.0; 2] } else { frame.look };
        let turn = (frame.axis(Key::A, Key::D)
            + frame.axis(Key::ArrowLeft, Key::ArrowRight)
            + pad.left[0])
            .clamp(-1.0, 1.0);
        let climb = (frame.axis(Key::ArrowDown, Key::ArrowUp) + pad.left[1]).clamp(-1.0, 1.0);
        let lead = wrap(self.aim[0] - self.yaw) + turn * STEER_RATE[0] * dt + mouse[0] * MOUSE_AIM;
        self.aim[0] = wrap(self.yaw + lead.clamp(-MAX_LEAD, MAX_LEAD));
        self.aim[1] = (self.aim[1] + climb * STEER_RATE[1] * dt - mouse[1] * MOUSE_AIM)
            .clamp(-MAX_PITCH, MAX_PITCH);
        let throttle =
            (frame.axis(Key::S, Key::W) + pad.triggers[1] - pad.triggers[0]).clamp(-1.0, 1.0);
        self.throttle = (self.throttle + throttle * THROTTLE_RATE * dt).clamp(0.0, 1.0);
        let boost = frame.modifiers.ctrl
            || pad.held(GamepadButton::South)
            || pad.held(GamepadButton::LeftStick);
        let target_speed = cruise(self.throttle) * if boost { BOOST } else { 1.0 };
        if free_look {
            self.look[0] = (self.look[0] + frame.look[0] * MOUSE_AIM).clamp(-PI, PI);
            self.look[1] =
                (self.look[1] - frame.look[1] * MOUSE_AIM).clamp(-LOOK_RANGE[1], LOOK_RANGE[1]);
        } else {
            self.look = [0, 1].map(|axis| {
                approach(
                    self.look[axis],
                    pad.right[axis] * LOOK_RANGE[axis],
                    LOOK_EASE,
                    dt,
                )
            });
        }
        let target = self.chase_offset(size);
        self.chase = Some(match self.chase {
            Some(offset) if dt > 0.0 => {
                [0, 1, 2].map(|axis| approach(offset[axis], target[axis], CHASE_EASE, dt))
            }
            Some(offset) => offset,
            None => target,
        });
        let rows = map.map.y();
        let ceiling = map.defines.heights[1];
        let reach = self.speed.max(target_speed) * dt;
        let steps = (dt / LONGEST_SUBSTEP)
            .max(reach / LONGEST_STRIDE)
            .ceil()
            .max(1.0) as usize;
        let step = dt / steps as f32;
        for _ in 0..steps {
            if step <= 0.0 {
                break;
            }
            let before = self.probes(airframe.size);
            let rate = (wrap(self.aim[0] - self.yaw) * TURN_GAIN).clamp(-TURN_RATE, TURN_RATE);
            self.yaw = wrap(self.yaw + rate * step);
            let mut pitch = self.aim[1];
            if self.position[1] >= ceiling && pitch > 0.0 {
                pitch = 0.0;
            }
            let change = ((pitch - self.pitch) * PITCH_GAIN).clamp(-PITCH_RATE, PITCH_RATE);
            self.pitch = (self.pitch + change * step).clamp(-MAX_PITCH, MAX_PITCH);
            self.roll = approach(self.roll, rate / TURN_RATE * MAX_BANK, BANK_EASE, step);
            self.speed = approach(self.speed, target_speed, SPEED_EASE, step).max(SLOWEST);
            let forward = direction(self.yaw, self.pitch);
            for (axis, value) in self.position.iter_mut().enumerate() {
                *value += forward[axis] * self.speed * step;
            }
            self.position[2] = self.position[2].clamp(0.0, rows);
            self.position[1] = self.position[1].min(ceiling);
            let after = self.probes(airframe.size);
            let struck = before
                .iter()
                .zip(&after)
                .any(|(from, to)| sweeps_into(floor, width, *from, *to, airframe.ground.offset));
            self.position[0] = self.position[0].rem_euclid(width);
            if struck {
                self.phase = Phase::Crashed {
                    seconds: 0.0,
                    eye: std::array::from_fn(|axis| {
                        self.position[axis] + self.chase.unwrap_or(target)[axis]
                    }),
                    exploded: !airframe.explodes,
                };
                return;
            }
        }
    }

    fn probes(&self, size: f32) -> [[f32; 3]; 5] {
        let [right, _, forward] = self.attitude().axes();
        let reach = size.max(0.0);
        [
            [0.0; 3],
            forward,
            forward.map(|v| -v),
            right,
            right.map(|v| -v),
        ]
        .map(|axis| std::array::from_fn(|i| self.position[i] + axis[i] * reach))
    }

    #[cfg(test)]
    pub fn step(&mut self, frame: &Frame, map: &Camera, floor: Floor<'_>, airframe: &Airframe) {
        let played = match self.phase {
            Phase::Crashed { seconds, .. } => Some(Some((seconds, f32::INFINITY))),
            _ => None,
        };
        self.update(
            frame,
            map,
            floor,
            airframe,
            played,
            &mut Footprints::default(),
        );
    }

    #[cfg(test)]
    pub fn strikes(&self, floor: Floor<'_>, width: f32, airframe: &Airframe) -> bool {
        self.probes(airframe.size).iter().any(|point| {
            point[1] <= ground(floor, width, point[0], point[2]) + airframe.ground.offset
        })
    }

    fn respawn(
        &mut self,
        map: &Camera,
        floor: Floor<'_>,
        airframe: &Airframe,
        footprints: &mut Footprints,
    ) {
        let width = map.map.x().max(1.0);
        let ceiling = map.defines.heights[1];
        let reach = airframe.size.max(0.0);
        let lift = clearance(airframe.size) * START_CLEARANCES + airframe.ground.offset.max(0.0);
        let speed = cruise(self.throttle);
        let (sin, cos) = self.yaw.sin_cos();
        let ahead = speed * RESPAWN_AHEAD;
        let [x, z] = [self.position[0], self.position[2]];
        let here = highest(
            floor,
            width,
            [x, z],
            [x + sin * ahead, z + cos * ahead],
            reach,
        ) + lift;
        let found = if here <= ceiling {
            Some([x.rem_euclid(width), here, z])
        } else {
            footprints.ensure(floor, map, reach);
            footprints
                .nearest([x, z], [sin, cos], ceiling - lift)
                .map(|[x, top, z]| [x, top + lift, z])
        };
        let Some(position) = found else {
            if let Phase::Crashed { eye, .. } = self.phase {
                self.phase = Phase::Wrecked { eye };
            }
            return;
        };
        *self = Self {
            view: self.view,
            throttle: self.throttle,
            crashes: self.crashes + 1,
            ..Self::new(position, self.yaw, 0.0, 0.0)
        };
        self.speed = speed;
    }

    fn chase_offset(&self, size: f32) -> [f32; 3] {
        let size = size.max(LEAST_CLEARANCE);
        let toward = direction(
            self.aim[0] + self.look[0],
            (self.aim[1] + self.look[1]).clamp(-PITCH_LIMIT, PITCH_LIMIT),
        );
        [
            -toward[0] * size * CHASE_DISTANCE,
            -toward[1] * size * CHASE_DISTANCE + size * CHASE_RISE,
            -toward[2] * size * CHASE_DISTANCE,
        ]
    }

    fn attitude(&self) -> Free {
        Free {
            yaw: self.yaw,
            pitch: self.pitch,
            roll: self.roll,
            lens: Lens::MAP,
        }
    }

    pub fn sight(&self, base: Camera, floor: Floor<'_>, airframe: &Airframe) -> Sight {
        let width = base.map.x().max(1.0);
        let size = airframe.size.max(LEAST_CLEARANCE);
        let attitude = self.attitude();
        let chase_lens = Lens {
            fov_y: FOV_Y,
            near: (size * NEAR_PER_SIZE).clamp(NEAREST, FARTHEST_NEAR),
            far: FAR,
        };
        let (eye, pose) = match (self.phase, self.view) {
            (Phase::Crashed { eye, .. } | Phase::Wrecked { eye }, _) => {
                (eye, Free::looking_at(eye, self.position, chase_lens))
            }
            (Phase::Flying, View::Nose) => {
                let forward = attitude.forward();
                let eye: [f32; 3] =
                    std::array::from_fn(|axis| self.position[axis] + forward[axis] * size * NOSE);
                let above = eye[1] - ground(floor, width, eye[0], eye[2]);
                let lens = Lens {
                    near: (above * NEAR_PER_HEIGHT).clamp(NEAREST, FARTHEST_NEAR),
                    ..chase_lens
                };
                (eye, Free { lens, ..attitude })
            }
            (Phase::Flying, View::Chase) => {
                let offset = self.chase.unwrap_or_else(|| self.chase_offset(size));
                let eye: [f32; 3] = std::array::from_fn(|axis| self.position[axis] + offset[axis]);
                let toward = direction(
                    self.aim[0] + self.look[0],
                    (self.aim[1] + self.look[1]).clamp(-PITCH_LIMIT, PITCH_LIMIT),
                );
                let target: [f32; 3] = std::array::from_fn(|axis| {
                    self.position[axis] + toward[axis] * size * CHASE_LEAD
                });
                (eye, Free::looking_at(eye, target, chase_lens))
            }
        };
        let mut camera = Camera {
            center: [eye[0], eye[2]],
            height: eye[1],
            pose: Pose::Free(pose),
            ..base
        };
        let lifted = |x: f32, z: f32| ground(floor, width, x, z);
        camera.hold_free(&lifted, pose.lens.near * CAMERA_CLEARANCE);
        let shift = ((camera.center[0] - eye[0]) / width).round() * width;
        let position = [self.position[0] + shift, self.position[1], self.position[2]];
        let aircraft = match self.phase {
            Phase::Flying => (self.view == View::Chase).then(|| self.instance(position)),
            Phase::Crashed { exploded, .. } => (!exploded).then(|| ModelInstance {
                animation: viewer_models::entity_animation::named_request(EXPLODE),
                ..self.instance(position)
            }),
            Phase::Wrecked { .. } => None,
        };
        Sight { camera, aircraft }
    }

    pub fn animation_key(&self, mesh: usize) -> (usize, u32, u32) {
        (mesh, ANIMATION_SEED, self.crashes + 1)
    }

    fn instance(&self, position: [f32; 3]) -> ModelInstance {
        let [right, up, forward] = self.attitude().axes();
        let basis = [right.map(|v| -v), up, forward.map(|v| -v)];
        ModelInstance {
            position,
            rotation: (-basis[2][0]).atan2(basis[2][2]),
            pitch: (-basis[2][1]).clamp(-1.0, 1.0).asin(),
            roll: basis[0][1].atan2(basis[1][1]),
            scale: 1.0,
            native_flags: 4,
            native_basis: basis,
            animation_seed: ANIMATION_SEED,
            animation_identity: self.crashes + 1,
            ..ModelInstance::default()
        }
    }
}

pub fn sweeps_into(
    floor: Floor<'_>,
    width: f32,
    from: [f32; 3],
    to: [f32; 3],
    offset: f32,
) -> bool {
    let meets = |t: f32| {
        let point: [f32; 3] = std::array::from_fn(|i| from[i] + (to[i] - from[i]) * t);
        point[1] <= ground(floor, width, point[0], point[2]) + offset
    };
    if meets(0.0) || meets(1.0) {
        return true;
    }
    let lines = [
        (from[0], to[0]),
        (from[2], to[2]),
        (from[0] - from[2], to[0] - to[2]),
        (from[0] + from[2], to[0] + to[2]),
    ];
    lines.into_iter().any(|(start, end)| {
        if start == end || !(start.is_finite() && end.is_finite()) {
            return false;
        }
        let (low, high) = (start.min(end), start.max(end));
        let mut line = low.ceil();
        while line <= high {
            if meets((line - start) / (end - start)) {
                return true;
            }
            line += 1.0;
        }
        false
    })
}

pub fn highest(floor: Floor<'_>, width: f32, from: [f32; 2], to: [f32; 2], radius: f32) -> f32 {
    let margin = radius.max(0.0) + std::f32::consts::SQRT_2;
    let length = (to[0] - from[0]).hypot(to[1] - from[1]);
    let steps = length.ceil().max(1.0) as usize;
    let mut top = WATER_HEIGHT;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let [x, z] = [
            from[0] + (to[0] - from[0]) * t,
            from[1] + (to[1] - from[1]) * t,
        ];
        let [x0, x1] = [(x - margin).floor(), (x + margin).ceil()];
        let [z0, z1] = [(z - margin).floor(), (z + margin).ceil()];
        let mut column = x0;
        while column <= x1 {
            let mut row = z0;
            while row <= z1 {
                top = top.max(ground(floor, width, column, row));
                row += 1.0;
            }
            column += 1.0;
        }
    }
    top
}

#[derive(Debug, Default)]
pub struct Footprints {
    key: Option<(usize, usize, usize)>,
    width: usize,
    heights: Vec<f32>,
}

impl Footprints {
    pub fn radius(reach: f32) -> usize {
        (reach.max(0.0) + std::f32::consts::SQRT_2).ceil() as usize
    }

    pub fn ensure(&mut self, floor: Floor<'_>, map: &Camera, reach: f32) {
        let width = map.map.width.max(1) as usize;
        let rows = map.map.height.max(1) as usize;
        let radius = Self::radius(reach);
        if self.key == Some((width, rows, radius)) {
            return;
        }
        let mut heights = vec![WATER_HEIGHT; width * rows];
        for row in 0..rows {
            for column in 0..width {
                heights[row * width + column] =
                    ground(floor, width as f32, column as f32, row as f32);
            }
        }
        let mut across = vec![WATER_HEIGHT; width * rows];
        for row in 0..rows {
            for column in 0..width {
                let mut top = WATER_HEIGHT;
                for step in -(radius as isize)..=radius as isize {
                    let other = (column as isize + step).rem_euclid(width as isize) as usize;
                    top = top.max(heights[row * width + other]);
                }
                across[row * width + column] = top;
            }
        }
        for row in 0..rows {
            let [first, last] = [row.saturating_sub(radius), (row + radius).min(rows - 1)];
            for column in 0..width {
                heights[row * width + column] = (first..=last)
                    .map(|other| across[other * width + column])
                    .fold(WATER_HEIGHT, f32::max);
            }
        }
        *self = Self {
            key: Some((width, rows, radius)),
            width,
            heights,
        };
    }

    pub fn nearest(&self, from: [f32; 2], toward: [f32; 2], limit: f32) -> Option<[f32; 3]> {
        let width = self.width as f32;
        let mut best: Option<(f32, f32, [f32; 3])> = None;
        for (index, top) in self.heights.iter().enumerate() {
            if *top > limit {
                continue;
            }
            let [column, row] = [(index % self.width) as f32, (index / self.width) as f32];
            let across = (column - from[0] + width * 0.5).rem_euclid(width) - width * 0.5;
            let along = row - from[1];
            let distance = across * across + along * along;
            let side = -(across * toward[0] + along * toward[1]);
            if best.is_none_or(|(best_distance, best_side, _)| {
                (distance, side) < (best_distance, best_side)
            }) {
                best = Some((distance, side, [column, *top, row]));
            }
        }
        best.map(|(.., vertex)| vertex)
    }
}

pub fn clearance(size: f32) -> f32 {
    (size * CLEARANCE_PER_SIZE).max(LEAST_CLEARANCE)
}

pub fn cruise(throttle: f32) -> f32 {
    SLOWEST + (FASTEST - SLOWEST) * throttle.clamp(0.0, 1.0)
}

fn ground(floor: Floor<'_>, width: f32, x: f32, z: f32) -> f32 {
    floor(x.rem_euclid(width), z).max(WATER_HEIGHT)
}

fn direction(yaw: f32, pitch: f32) -> [f32; 3] {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch]
}

fn wrap(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests;
