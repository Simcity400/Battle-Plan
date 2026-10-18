//! The walking camera mode: the viewing country's infantry figure
//! ([`viewer_world::map::unit_models::UnitModelAssets::infantry_figure`]) seen from behind, walking
//! on the drawn ground and water and flying as in a creative sandbox.
//!
//! | Action | Keyboard and mouse | Controller |
//! |---|---|---|
//! | Look around the figure | Mouse, while the map holds the pointer | Right stick |
//! | Walk, relative to the view | W A S D | Left stick |
//! | Faster | Ctrl | Left stick pressed in |
//! | Start or stop flying | Space twice | South button (A, cross) twice |
//! | Rise / descend while flying | Space / Shift | South / East button (B, circle), or right / left trigger |
//! | Grow / shrink | + / - held | D-pad up / down held |
//! | View nearer / farther | Mouse wheel | Left / right bumper held |
//! | View from behind the figure again | | Right stick pressed in |
//!
//! Every length is in figure heights: the figure's height at the size the game draws its units,
//! times the user's scale. Walking, flying, falling, the view's distance and the near plane all
//! follow the scale, so a grown or shrunk figure moves and is seen as one of normal size would be.
//!
//! On the ground the figure's feet stand on [`HeightField::surface_height`], the drawn triangle
//! surface or the water above it, so it walks on seas and lakes too. Flying holds its height
//! until Space or Shift moves it, and descending onto the ground lands. Stopping flight falls
//! under gravity back to the ground. The figure plays its entity's `move` state while it walks
//! and `idle` otherwise, and turns to face where it walks.
//!
//! [`HeightField::surface_height`]: viewer_foundation::map::province_map::HeightField::surface_height

use super::{Frame, approach};
use dashboard::protocol::GamepadButton;
use eframe::egui::Key;
use std::f32::consts::{PI, TAU};
use viewer_foundation::map::camera::{Camera, FOV_Y, Free, Lens, Pose, WATER_HEIGHT};
use viewer_foundation::map::province_map::HEIGHT_SCALE;
use viewer_world::map::entity_animation::{IDLE, MOVE};
use viewer_world::map::models::ModelInstance;
use viewer_world::map::unit_models::MESH_FORWARD_OFFSET;

/// Walking pace in figure heights per second.
pub const WALK_PACE: f32 = 1.0;
/// Flying pace, across and up or down, in figure heights per second.
pub const FLY_PACE: f32 = 2.5;
/// The faster control multiplies the pace by this.
pub const BOOST: f32 = 3.0;
/// Falling acceleration and its terminal speed, in figure heights per second (squared).
pub const GRAVITY: f32 = 5.4;
pub const TERMINAL_SPEED: f32 = 30.0;
/// Seconds moving and stick looking take to ease in and out.
pub const EASE: f32 = 0.15;
/// Seconds the figure takes to turn most of the way to where it walks.
pub const TURN: f32 = 0.1;
/// Below this share of its walking pace a figure on the ground stands rather than walks.
pub const STANDING: f32 = 0.2;
/// Holding grow or shrink doubles or halves the scale every this many seconds.
pub const DOUBLING: f32 = 1.0;
/// Radians the view turns per pixel of pointer motion.
pub const MOUSE_LOOK: f32 = 0.0022;
/// Radians per second a full right-stick deflection turns the view across and up or down.
pub const STICK_LOOK: [f32; 2] = [2.2, 1.5];
/// The view's pitch limits around the figure: from nearly straight down to looking up at it.
pub const VIEW_PITCH: [f32; 2] = [-1.45, 0.6];
/// The view's pitch when it starts or is reset: a little above the figure.
pub const VIEW_PITCH_START: f32 = -0.25;
/// The view's distance from the point it looks at, in figure heights, its limits and its
/// start.
pub const VIEW_DISTANCE: [f32; 2] = [1.5, 12.0];
pub const VIEW_DISTANCE_START: f32 = 4.0;
/// The view distance's logarithm changes by this per point of the window's smoothed wheel
/// scroll, and per second a bumper is held.
pub const WHEEL_ZOOM: f32 = 0.002;
pub const BUMPER_ZOOM: f32 = 0.7;
/// The view looks at the figure this share of its height above the feet.
pub const LOOK_AT: f32 = 0.75;
/// The near plane's share of the nearest thing the view must show (the figure, or the ground
/// under the eye, taken as no nearer than `NEAREST_SHARE` of the figure's distance), and the
/// eye's least height above the ground in near planes.
pub const NEAR_SHARE: f32 = 0.25;
pub const NEAREST_SHARE: f32 = 0.1;
pub const CLEARANCE: f32 = 2.0;
/// The far plane: far enough that the ground runs to the horizon.
pub const FAR: f32 = 20_000.0;
/// The figure's height when it has none (no entity resolved): a world unit.
pub const FALLBACK_FIGURE: f32 = 1.0;
/// The figure's entity seed and actor identity, apart from every unit's.
pub const ANIMATION_SEED: u32 = 0x5741_4c4b;

/// How the figure moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    /// Standing or walking on the ground or water.
    Ground,
    /// Flying: holding its height until told to rise or descend.
    Flying,
    /// Falling under gravity after flight stopped.
    Falling,
}

/// The world the figure moves in: the drawn ground under a world `(x, z)`, the map's width and
/// height, and the content's highest camera.
pub struct World<'a> {
    pub ground: &'a dyn Fn(f32, f32) -> f32,
    pub map: [f32; 2],
    pub ceiling: f32,
}

impl World<'_> {
    /// The world of a camera on the map, standing on `ground`.
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

/// The scale's limits for a figure `figure` world units tall under `ceiling`: no shorter than
/// one step of the height map, below which the ground's own faceting outgrows it, and no taller
/// than the figure and its farthest view fit between the water and the content's highest camera.
pub fn scale_limits(figure: f32, ceiling: f32) -> [f32; 2] {
    let figure = figure.max(f32::EPSILON);
    let smallest = HEIGHT_SCALE / figure;
    let largest = (ceiling - WATER_HEIGHT) / (figure * (1.0 + VIEW_DISTANCE[1]));
    [smallest, largest.max(smallest)]
}

/// The walker: its figure and the view following it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    /// The figure's feet in world units.
    pub feet: [f32; 3],
    /// Radians clockwise from north the figure faces.
    pub facing: f32,
    /// The view's heading around the figure, radians clockwise from north.
    pub view_yaw: f32,
    /// The view's pitch, radians above the horizontal: negative looks down at the figure.
    pub view_pitch: f32,
    /// The view's distance from the point it looks at, in figure heights.
    pub distance: f32,
    /// The user's scale: 1 is the size the game draws its units.
    pub scale: f32,
    pub gait: Gait,
    /// Eased velocity across the ground, world units per second.
    velocity: [f32; 2],
    /// Upward speed in world units per second: eased while flying, gravity's while falling.
    climb: f32,
    /// Eased stick turn rates.
    turn: [f32; 2],
}

impl Walker {
    /// A figure standing on the ground at world `(x, z)`, facing `yaw` with the view behind it.
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

    /// The figure's height in world units, `figure` being its height at scale 1.
    fn height(&self, figure: f32) -> f32 {
        figure * self.scale
    }

    /// Advances one frame. `wheel` is the window's smoothed wheel scroll in points, positive
    /// away from the user; `figure` is the figure's height at scale 1.
    pub fn update(&mut self, frame: &Frame, wheel: f32, figure: f32, world: &World<'_>) {
        let dt = frame.dt;
        let pad = &frame.pad;
        // Flight: a double tap starts it on the ground or in a fall, and stops it in the air.
        if frame.double_tapped(Key::Space) || pad.double_tapped(GamepadButton::South) {
            self.gait = match self.gait {
                Gait::Ground | Gait::Falling => Gait::Flying,
                Gait::Flying => Gait::Falling,
            };
            self.climb = 0.0;
        }
        // Scale: held grow and shrink double or halve it every `DOUBLING` seconds.
        let grow = f32::from(
            i8::from(frame.held(Key::Plus) || frame.held(Key::Equals))
                - i8::from(frame.held(Key::Minus)),
        ) + f32::from(
            i8::from(pad.held(GamepadButton::DpadUp)) - i8::from(pad.held(GamepadButton::DpadDown)),
        );
        let [smallest, largest] = scale_limits(figure, world.ceiling);
        self.scale =
            (self.scale * (grow.clamp(-1.0, 1.0) * dt / DOUBLING).exp2()).clamp(smallest, largest);
        // The view: pointer motion is already an angle; the stick is a rate.
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
        // Moving: relative to the view's heading, at a pace in figure heights.
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
        // Up and down.
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
                    // Descending onto the ground lands.
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
        // The figure and the view behind it stay under the content's highest camera.
        let top = (world.ceiling - height * (1.0 + self.distance)).max(floor);
        if self.feet[1] > top {
            self.feet[1] = top;
            self.climb = self.climb.min(0.0);
        }
    }

    /// Whether the figure walks: on the ground and moving at more than [`STANDING`] of its pace.
    pub fn walking(&self, figure: f32) -> bool {
        self.gait == Gait::Ground
            && self.velocity[0].hypot(self.velocity[1]) > STANDING * WALK_PACE * self.height(figure)
    }

    /// The figure as the scene places it: its feet, facing and scale, playing `move` while it
    /// walks and `idle` otherwise.
    pub fn instance(&self, figure: f32) -> ModelInstance {
        let walking = self.walking(figure);
        // The engine's heading is measured from east towards north; a unit mesh faces its
        // own -Z, which `MESH_FORWARD_OFFSET` turns onto the heading.
        let (east, north) = self.facing.sin_cos();
        ModelInstance {
            position: self.feet,
            rotation: north.atan2(east) + MESH_FORWARD_OFFSET,
            scale: self.scale,
            // A land figure; its textures scroll while it walks, as a moving division's do.
            native_flags: 1 | (u32::from(walking) << 1),
            animation: if walking { MOVE } else { IDLE },
            animation_seed: ANIMATION_SEED,
            animation_identity: ANIMATION_SEED,
            ..ModelInstance::default()
        }
    }

    /// The camera the walker is seen through: behind and around the figure at the view's
    /// distance, looking at it, above the drawn ground and on `base`'s map.
    ///
    /// The eye is held on the map's rows, above the ground and under the highest camera before
    /// it is turned to the figure, so at the map's top or bottom it rises to look down at the
    /// figure from the view's distance rather than looking past it.
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
        // The near plane keeps the figure and the ground under the eye in front of it.
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
        // Only the eye's x is left to move, onto the map's first copy, which turns nothing.
        camera.hold_free(world.ground, near * CLEARANCE);
        camera
    }
}

/// A walk the command line places (`--walk`) for a window dump: where the figure starts, and
/// the keys held through every frame it draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Start {
    /// World `(x, z)`.
    pub at: [f32; 2],
    /// Radians clockwise from north the figure and its view face.
    pub facing: f32,
    pub scale: f32,
    /// Starts in flight rather than on the ground.
    pub flying: bool,
    pub held: Vec<Key>,
    pub modifiers: eframe::egui::Modifiers,
}

impl Start {
    /// `--walk x,z,heading[,scale]` with the heading in degrees, and `--walk-keys`' names:
    /// `w`, `a`, `s`, `d`, `space`, `shift`, `ctrl`, `plus` and `minus`.
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

    /// The input of a frame `dt` seconds after the last, holding the start's keys.
    pub fn frame(&self, dt: f32) -> Frame {
        Frame {
            dt,
            held: self.held.iter().copied().collect(),
            modifiers: self.modifiers,
            ..Frame::default()
        }
    }

    /// The walker this start places on `world`.
    pub fn walker(&self, world: &World<'_>) -> Walker {
        let mut walker = Walker::standing(self.at, self.facing, self.scale, world);
        if self.flying {
            walker.gait = Gait::Flying;
        }
        walker
    }
}

/// An angle in `[-PI, PI)`.
fn wrap_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests;
