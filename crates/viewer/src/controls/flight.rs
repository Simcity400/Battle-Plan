//! The aircraft the flying mode flies, and the camera that follows it.
//!
//! Arcade flight: the pilot points where to go and the aircraft follows, banking into its turns.
//! It cannot stall: it never flies slower than [`SLOWEST`], whatever its attitude. It looks ahead
//! along its heading and climbs over the ground it would meet, never climbs past the content's
//! `CAMERA_MAX_HEIGHT` and slides along the map's top and bottom edges; across the east and west
//! edges it flies on round the world. With nothing held it levels its wings at once and its nose
//! after a moment.
//!
//! It can crash. Every step sweeps its origin, nose, tail and wingtips (its model's bounding radius
//! from its origin, along its attitude) along their paths against the drawn ground or water raised
//! by the content's `AIRPLANES_GROUND_COLLISION_OFFSET_Y`, exactly: the drawn surface is planar
//! between its cell edges and diagonals, so each path is tested where it crosses them
//! ([`sweeps_into`]). Where it strikes, the controls stop, the camera holds, and the aircraft
//! plays its entity's `explode` state and goes once its own player has played the content's
//! `AIRPLANES_GROUND_EXPLOSION_TIME_DELAY` of it, read as the map's aircraft read it
//! ([`GroundContact::exploded`]), or at once without that state. An explosion is over, too, when
//! the aircraft shown gives way to one without that state, or to none, before it has played:
//! nothing is left to play it. When the explosion has gone,
//! and at least [`RESPAWN_AFTER`] seconds after the crash, it flies again level on the same
//! heading at the same throttle: over the wreck, [`START_CLEARANCES`] clearances above the
//! highest ground within its bounding radius of the next [`RESPAWN_AHEAD`] seconds of flight
//! ([`highest`]), when that height fits under the content's `CAMERA_MAX_HEIGHT`; otherwise at the
//! vertex of the whole map nearest the wreck, across its east-west seam too, whose own
//! footprint's height fits ([`Footprints`]). What it then flies into, it may crash into again. A
//! map with no such vertex leaves the wreck where it lies.
//!
//! | Action | Keyboard and mouse | Controller |
//! |---|---|---|
//! | Steer | Mouse, while the map holds the pointer; A / D or Left / Right turn; Up / Down climb and dive | Left stick |
//! | Throttle up / down | W / S | Right / left trigger |
//! | Faster | Ctrl | South face button or left stick pressed in |
//! | Look around (chase view) | C held, with the mouse | Right stick |
//! | Chase or nose view | V | North face button |
//!
//! The chase view sits behind and above the aircraft, looking where it is steered from a distance
//! set by the model's size. The nose view looks out from the aircraft's nose along its attitude,
//! banking with it; no cockpit and no part of the aircraft is drawn in it. A crash is watched from
//! where the chase view stood, in either view.
//!
//! Time is the input layer's wall-clock frame time ([`Frame::dt`]), integrated in steps of at
//! most [`LONGEST_SUBSTEP`] and [`LONGEST_STRIDE`].

use super::{Frame, approach};
use dashboard::protocol::GamepadButton;
use eframe::egui::Key;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use viewer_foundation::map::camera::{Camera, Free, Lens, PITCH_LIMIT, Pose, WATER_HEIGHT};
use viewer_world::map::aircraft::GroundContact;
use viewer_world::map::models::ModelInstance;

/// The slowest the aircraft flies, in world units (map pixels) per second: it never stalls.
pub const SLOWEST: f32 = 4.0;
/// The speed at full throttle.
pub const FASTEST: f32 = 80.0;
/// The throttle a flight starts at.
pub const START_THROTTLE: f32 = 0.2;
/// How much of the throttle's range a held control moves per second.
pub const THROTTLE_RATE: f32 = 0.5;
/// The faster control multiplies the throttle's speed by this.
pub const BOOST: f32 = 3.0;
/// Seconds the airspeed takes to follow the throttle.
pub const SPEED_EASE: f32 = 0.8;

/// Radians the steering point moves per pixel of pointer motion.
pub const MOUSE_AIM: f32 = 0.0025;
/// Radians per second full keys or stick move the steering point across and up or down.
pub const STEER_RATE: [f32; 2] = [1.3, 1.0];
/// How far the steering point may lead the aircraft's heading, so it cannot point behind it.
pub const MAX_LEAD: f32 = 1.4;
/// The steepest climb or dive.
pub const MAX_PITCH: f32 = 1.2;
/// Seconds without climbing or diving after which the steering point drifts back to level, and
/// the time that drift takes.
pub const LEVEL_DELAY: f32 = 0.8;
pub const LEVEL_EASE: f32 = 1.5;

/// The fastest turn, in radians per second, and the turn rate per radian the heading is off.
pub const TURN_RATE: f32 = 1.0;
pub const TURN_GAIN: f32 = 2.0;
/// The fastest pitch change and the rate per radian the pitch is off.
pub const PITCH_RATE: f32 = 0.9;
pub const PITCH_GAIN: f32 = 3.0;
/// The bank of the fastest turn, and the seconds the bank takes to follow the turn.
pub const MAX_BANK: f32 = 1.05;
pub const BANK_EASE: f32 = 0.25;

/// Seconds ahead the ground is checked at, along the heading at the current speed.
pub const LOOKAHEAD: [f32; 3] = [0.5, 1.0, 2.0];
/// Clearances above the ground ahead the aircraft climbs to keep.
pub const AVOID_MARGIN: f32 = 2.0;
/// The clearance the ground checks keep, in model sizes, and its least value.
pub const CLEARANCE_PER_SIZE: f32 = 1.5;
pub const LEAST_CLEARANCE: f32 = 0.3;
/// The size flown with when no model is drawn, in world units.
pub const DEFAULT_SIZE: f32 = 1.0;
/// The longest integration step, in seconds.
pub const LONGEST_SUBSTEP: f32 = 1.0 / 120.0;
/// The farthest the aircraft moves between two ground tests: half a height-map cell, so no
/// ridge of the drawn surface fits between them.
pub const LONGEST_STRIDE: f32 = 0.5;

/// The least seconds from a crash until the aircraft flies again; an explosion still playing
/// plays out first.
pub const RESPAWN_AFTER: f32 = 2.5;
/// Seconds of flight ahead whose highest ground a respawned aircraft clears.
pub const RESPAWN_AHEAD: f32 = 3.0;

/// The chase camera's distance behind the aircraft, height above it and the distance ahead of
/// it that it looks at, in model sizes, and the seconds it takes to follow.
pub const CHASE_DISTANCE: f32 = 4.0;
pub const CHASE_RISE: f32 = 1.0;
pub const CHASE_LEAD: f32 = 3.0;
pub const CHASE_EASE: f32 = 0.12;
/// How far the look-around control turns the chase camera across and up or down, and the
/// seconds it takes to turn there and back.
pub const LOOK_RANGE: [f32; 2] = [2.6, 0.9];
pub const LOOK_EASE: f32 = 0.15;
/// The nose view's eye, in model sizes ahead of the aircraft's origin.
pub const NOSE: f32 = 1.0;

/// The flying lens's vertical field of view, its far plane, its near plane per model size in the
/// chase view and per unit of height above the ground in the nose view, and the near plane's
/// limits.
pub const FOV_Y: f32 = 0.96;
pub const FAR: f32 = 20_000.0;
pub const NEAR_PER_SIZE: f32 = 0.2;
pub const NEAR_PER_HEIGHT: f32 = 0.05;
pub const NEAREST: f32 = 0.02;
pub const FARTHEST_NEAR: f32 = 2.0;
/// The camera keeps at least this many near planes above the ground.
pub const CAMERA_CLEARANCE: f32 = 3.0;

/// How high above the ground a flight starts, as a share of the map camera's height above it,
/// and the least start height in clearances (also a respawn's height).
pub const START_HEIGHT_SHARE: f32 = 0.25;
pub const START_CLEARANCES: f32 = 4.0;

/// The player aircraft's animation identity, apart from every map model's.
pub const ANIMATION_SEED: u32 = 0x464c_5931;
/// The entity state a crashing aircraft plays.
pub const EXPLODE: &str = "explode";

/// The height of the drawn ground or water under a world `(x, z)`.
pub type Floor<'a> = &'a dyn Fn(f32, f32) -> f32;

/// Where the camera stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// Behind and above the aircraft, which is drawn.
    #[default]
    Chase,
    /// At the aircraft's nose, which is not drawn.
    Nose,
}

/// What a flight knows of the aircraft it flies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Airframe {
    /// The model's bounding radius in world units ([`DEFAULT_SIZE`] without a model): its nose,
    /// tail and wingtips are tested this far from its origin, and the cameras and clearances
    /// scale with it.
    pub size: f32,
    /// Whether its entity has the [`EXPLODE`] state.
    pub explodes: bool,
    /// How it meets the ground.
    pub ground: GroundContact,
}

impl Airframe {
    /// An aircraft without a model: it meets the drawn surface itself and goes at once.
    pub const NONE: Self = Self {
        size: DEFAULT_SIZE,
        explodes: false,
        ground: GroundContact {
            offset: 0.0,
            explosion: 0.0,
        },
    };
}

/// Whether the aircraft flies or lies wrecked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Flying,
    /// It struck the ground or water `seconds` ago; the camera holds at `eye`. `exploded` once
    /// its explosion has played and gone, or nothing is left to play it; from the crash without
    /// one.
    Crashed {
        seconds: f32,
        eye: [f32; 3],
        exploded: bool,
    },
    /// Its heading line has no point where it could fly again under the content's
    /// `CAMERA_MAX_HEIGHT`; the camera holds at `eye` over the wreck.
    Wrecked { eye: [f32; 3] },
}

/// One flight.
#[derive(Clone, Debug, PartialEq)]
pub struct Flight {
    /// The aircraft's origin in world units, `x` on the map's first copy.
    pub position: [f32; 3],
    /// Heading in radians, clockwise from north.
    pub yaw: f32,
    /// Radians above the horizontal.
    pub pitch: f32,
    /// Bank in radians: positive lowers the right wing.
    pub roll: f32,
    /// Where the pilot steers: a heading and a pitch.
    pub aim: [f32; 2],
    /// Airspeed in world units per second.
    pub speed: f32,
    /// The throttle, from [`SLOWEST`] at 0 to [`FASTEST`] at 1.
    pub throttle: f32,
    pub view: View,
    pub phase: Phase,
    /// Crashes so far: each life of the aircraft has its own animation history.
    pub crashes: u32,
    /// The chase camera's turn away from the steering point, across and up or down.
    look: [f32; 2],
    /// The chase camera's eye relative to the aircraft, once it has followed a frame.
    chase: Option<[f32; 3]>,
    /// Seconds since the pilot last climbed or dived.
    unsteered: f32,
}

/// What a flight shows: the camera, and the aircraft's model instance when it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    pub camera: Camera,
    pub aircraft: Option<ModelInstance>,
}

impl Flight {
    /// A level flight at `position` heading `yaw` at the starting throttle.
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
            unsteered: 0.0,
        }
    }

    /// A flight starting over the ground `map` looks at, heading the way it faces, a quarter of
    /// its height above the ground.
    pub fn from_map(map: &Camera, floor: Floor<'_>, size: f32) -> Self {
        let forward = map.look_direction();
        let target = match map.free() {
            // A free eye's ground point is the ground under it.
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

    /// Advances one frame of input. A crashed aircraft takes none, and flies again once its
    /// explosion has played and [`RESPAWN_AFTER`] seconds have passed: `played` is what its
    /// entity's own player has played, as the last frame left it ([`Flight::animation_key`]).
    /// The explosion is over at once when `airframe`, the aircraft now shown, has no explode
    /// state: the one that crashed gave way to another or to none, and its player went with it.
    /// `footprints` keeps what a respawn under a low ceiling builds, for the map's next one.
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
                *exploded =
                    *exploded || !airframe.explodes || airframe.ground.exploded(played);
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
        // Steering: the pointer moves the steering point by its motion, keys and stick at a rate.
        // The lead over the heading is limited before it is wrapped, so a long motion one way
        // cannot wrap round into a turn the other way.
        let free_look = frame.held(Key::C);
        let mouse = if free_look { [0.0; 2] } else { frame.look };
        let turn = (frame.axis(Key::A, Key::D)
            + frame.axis(Key::ArrowLeft, Key::ArrowRight)
            + pad.left[0])
            .clamp(-1.0, 1.0);
        let climb = (frame.axis(Key::ArrowDown, Key::ArrowUp) + pad.left[1]).clamp(-1.0, 1.0);
        let lead = wrap(self.aim[0] - self.yaw) + turn * STEER_RATE[0] * dt + mouse[0] * MOUSE_AIM;
        self.aim[0] = wrap(self.yaw + lead.clamp(-MAX_LEAD, MAX_LEAD));
        self.aim[1] += climb * STEER_RATE[1] * dt - mouse[1] * MOUSE_AIM;
        if climb != 0.0 || mouse[1] != 0.0 {
            self.unsteered = 0.0;
        } else {
            self.unsteered += dt;
            if self.unsteered > LEVEL_DELAY {
                self.aim[1] = approach(self.aim[1], 0.0, LEVEL_EASE, dt);
            }
        }
        self.aim[1] = self.aim[1].clamp(-MAX_PITCH, MAX_PITCH);
        // Throttle.
        let throttle =
            (frame.axis(Key::S, Key::W) + pad.triggers[1] - pad.triggers[0]).clamp(-1.0, 1.0);
        self.throttle = (self.throttle + throttle * THROTTLE_RATE * dt).clamp(0.0, 1.0);
        let boost = frame.modifiers.ctrl
            || pad.held(GamepadButton::South)
            || pad.held(GamepadButton::LeftStick);
        let target_speed = cruise(self.throttle) * if boost { BOOST } else { 1.0 };
        // Looking around turns the chase camera away and springs back when let go.
        if free_look {
            self.look[0] = (self.look[0] + frame.look[0] * MOUSE_AIM).clamp(-PI, PI);
            self.look[1] = (self.look[1] - frame.look[1] * MOUSE_AIM)
                .clamp(-LOOK_RANGE[1], LOOK_RANGE[1]);
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
        // The chase camera follows; it starts where it settles.
        let target = self.chase_offset(size);
        self.chase = Some(match self.chase {
            Some(offset) if dt > 0.0 => {
                [0, 1, 2].map(|axis| approach(offset[axis], target[axis], CHASE_EASE, dt))
            }
            Some(offset) => offset,
            None => target,
        });
        // Flying, in steps short enough in time and in distance for the ground tests.
        let rows = map.map.y();
        let clearance = clearance(size);
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
            let mut pitch = self.aim[1].max(self.avoidance(floor, width, clearance));
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
            // Each probe is swept along its path before the position wraps onto the first copy.
            let after = self.probes(airframe.size);
            let struck = before.iter().zip(&after).any(|(from, to)| {
                sweeps_into(floor, width, *from, *to, airframe.ground.offset)
            });
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

    /// The aircraft's origin, nose, tail and wingtips: its model's bounding radius `size` from its
    /// origin along its attitude.
    fn probes(&self, size: f32) -> [[f32; 3]; 5] {
        let [right, _, forward] = self.attitude().axes();
        let reach = size.max(0.0);
        [[0.0; 3], forward, forward.map(|v| -v), right, right.map(|v| -v)]
            .map(|axis| std::array::from_fn(|i| self.position[i] + axis[i] * reach))
    }

    /// [`Flight::update`] with footprints of its own, for tests, with an explosion clip played
    /// at its natural pace from its start: as the last frame left it, its play position is the
    /// seconds since the crash.
    #[cfg(test)]
    pub fn step(&mut self, frame: &Frame, map: &Camera, floor: Floor<'_>, airframe: &Airframe) {
        let played = match self.phase {
            Phase::Crashed { seconds, .. } => Some(Some((seconds, f32::INFINITY))),
            _ => None,
        };
        self.update(frame, map, floor, airframe, played, &mut Footprints::default());
    }

    /// Whether the aircraft's origin, nose, tail or a wingtip is at or below the drawn ground or
    /// water raised by the content's contact offset, where it stands.
    #[cfg(test)]
    pub fn strikes(&self, floor: Floor<'_>, width: f32, airframe: &Airframe) -> bool {
        self.probes(airframe.size).iter().any(|point| {
            point[1] <= ground(floor, width, point[0], point[2]) + airframe.ground.offset
        })
    }

    /// Flies again level on the same heading at the same throttle: over the wreck, clear of the
    /// ground within its bounding radius of the next [`RESPAWN_AHEAD`] seconds of flight, when that
    /// height fits under the content's `CAMERA_MAX_HEIGHT`; otherwise at the vertex of the map
    /// nearest the wreck whose own footprint's height fits (the heading's side first on a tie). A
    /// map with no such vertex leaves the wreck where it lies.
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
        let here = highest(floor, width, [x, z], [x + sin * ahead, z + cos * ahead], reach) + lift;
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

    /// The pitch that clears the ground ahead by [`AVOID_MARGIN`] clearances.
    fn avoidance(&self, floor: Floor<'_>, width: f32, clearance: f32) -> f32 {
        let (sin, cos) = self.yaw.sin_cos();
        LOOKAHEAD
            .iter()
            .map(|seconds| {
                let reach = self.speed.max(SLOWEST) * seconds;
                let ahead = ground(
                    floor,
                    width,
                    self.position[0] + sin * reach,
                    self.position[2] + cos * reach,
                );
                (ahead + clearance * AVOID_MARGIN - self.position[1]).atan2(reach)
            })
            .fold(-FRAC_PI_2, f32::max)
    }

    /// The chase camera's eye relative to the aircraft once it has caught up.
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

    /// The aircraft's attitude as a free pose.
    fn attitude(&self) -> Free {
        Free {
            yaw: self.yaw,
            pitch: self.pitch,
            roll: self.roll,
            lens: Lens::MAP,
        }
    }

    /// The camera the flight shows the map through, on `base`'s map, and the aircraft's model
    /// instance when the view draws it. The camera's eye stands on the map's first copy and the
    /// aircraft beside it, across the map's seam if need be.
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
            // A crash is watched from where the chase view stood.
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
        // The eye moved onto the first copy; the aircraft moves with it.
        let shift = ((camera.center[0] - eye[0]) / width).round() * width;
        let position = [
            self.position[0] + shift,
            self.position[1],
            self.position[2],
        ];
        let aircraft = match self.phase {
            Phase::Flying => (self.view == View::Chase).then(|| self.instance(position)),
            // It explodes where it struck until the explosion has played, then goes.
            Phase::Crashed { exploded, .. } => (!exploded).then(|| ModelInstance {
                animation: viewer_world::map::entity_animation::named_request(EXPLODE),
                ..self.instance(position)
            }),
            Phase::Wrecked { .. } => None,
        };
        Sight { camera, aircraft }
    }

    /// The animation key the aircraft's instance plays under as library slot `mesh`: each life
    /// after a crash has its own animation identity.
    pub fn animation_key(&self, mesh: usize) -> (usize, u32, u32) {
        (mesh, ANIMATION_SEED, self.crashes + 1)
    }

    /// The aircraft's model instance at `position`, posed by its attitude. Plane meshes point
    /// their nose along local `-z` with local `x` to their left, as the game's flight routes pose
    /// them, so the basis is the attitude's left, up and backward axes. Each life after a crash
    /// has its own animation identity.
    fn instance(&self, position: [f32; 3]) -> ModelInstance {
        let [right, up, forward] = self.attitude().axes();
        let basis = [right.map(|v| -v), up, forward.map(|v| -v)];
        ModelInstance {
            position,
            // The Euler angles declare the instance; the basis poses it.
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

/// Whether a probe moving straight from `from` to `to` meets the drawn ground or water raised by
/// `offset` anywhere on the way. The drawn surface is planar between the lines `x = n`, `z = n`,
/// `x - z = n` and `x + z = n` for whole `n` (its cells and either of their diagonals), and so is
/// the water under it, so the probe's height over each, and over the higher of the two, is linear
/// between consecutive crossings of those lines: testing both ends and every crossing is exact.
pub fn sweeps_into(floor: Floor<'_>, width: f32, from: [f32; 3], to: [f32; 3], offset: f32) -> bool {
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

/// The highest drawn ground or water within `radius` of the path from `from` to `to` (world
/// `x, z`). The surface's highest points are the vertices of its cells, and every point of the
/// footprint lies in a cell whose vertices are within a cell's diagonal of it, so the vertices
/// around each unit step along the path bound it from above.
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

/// The highest drawn ground or water within an aircraft's reach of every vertex of the map, for
/// the respawn a ceiling below some of the map's ground needs. Each vertex holds the highest vertex
/// within [`Footprints::radius`] cells of it (wrapping east and west, held at the top and bottom
/// rows): the same bound [`highest`] takes around a point, so a vertex whose value fits under the
/// ceiling stands clear of the ground within the aircraft's reach. Built once for a map and an
/// aircraft size, when a respawn first needs it, and kept until either changes.
#[derive(Debug, Default)]
pub struct Footprints {
    /// The map's width and rows and the radius the values were built for.
    key: Option<(usize, usize, usize)>,
    width: usize,
    heights: Vec<f32>,
}

impl Footprints {
    /// The cells around a vertex its value covers for an aircraft reaching `reach`: the reach and a
    /// cell's diagonal, since a surface point's highest is its cell's highest vertex.
    pub fn radius(reach: f32) -> usize {
        (reach.max(0.0) + std::f32::consts::SQRT_2).ceil() as usize
    }

    /// Builds the values for `map` and an aircraft reaching `reach`, unless they are built: one
    /// height and two passes of the radius per vertex.
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
        // The highest along each row, wrapping east and west, then along each column.
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

    /// The vertex nearest `from`, the wreck's world `x, z`, whose value is at or below `limit`,
    /// with that value: the true distance across the east-west seam too, the heading `toward`'s
    /// side first on a tie. One pass over every vertex.
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

/// The clearance the ground checks keep for an aircraft of `size`.
pub fn clearance(size: f32) -> f32 {
    (size * CLEARANCE_PER_SIZE).max(LEAST_CLEARANCE)
}

/// The airspeed of a throttle setting.
pub fn cruise(throttle: f32) -> f32 {
    SLOWEST + (FASTEST - SLOWEST) * throttle.clamp(0.0, 1.0)
}

/// The drawn ground or water under `(x, z)`, `x` on any copy of the map.
fn ground(floor: Floor<'_>, width: f32, x: f32, z: f32) -> f32 {
    floor(x.rem_euclid(width), z).max(WATER_HEIGHT)
}

/// The unit direction of a heading and pitch.
fn direction(yaw: f32, pitch: f32) -> [f32; 3] {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch]
}

/// An angle within `-PI..PI`.
fn wrap(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests;
