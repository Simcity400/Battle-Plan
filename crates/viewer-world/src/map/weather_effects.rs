//! Weather particles from the recorded content stack and regional weather.
//!
//! The first nonzero rain, heavy-rain, snow, blizzard or sandstorm flag selects
//! the weather kind. Each authored anchor creates a primary/fading entity pair;
//! its small flag selects the small descriptor bank, falling back to the
//! regular bank if absent. Choice precedes culling and persists until the
//! weather kind changes. A region's producer runs on frames whose province
//! update list holds the region: only then does it compare the field's kind
//! with the one its pairs hold and swap them, and set its members' camera fade
//! ([`fade`]): the companion at every height, the primary only in the far band.
//! Camera thresholds and playback rates come from content.
//!
//! Emission accumulates fractional debt, spawns within free capacity and
//! retains blocked debt. Age expires strictly after lifetime. New births share
//! one sampled emitter orientation; per-particle ranges use counter-hash draws.
//! Forces divide time by authored mass before velocity changes, and position
//! updates afterward. Curves bake 64 Catmull-Rom samples, retaining endpoint
//! extrapolation, and interpolate by particle age. Each birth retains its atlas
//! tile for the installed compiled particle shader.
//!
//! Presentation history uses region/hour/anchor seeds and per-emitter streams.
//! New emitters and dumps begin at clock zero. Re-entry retains a frozen
//! emitter; long suspensions preserve its population. Delta subdivision follows
//! the game. Forward playback skips retain the populations of the kinds the
//! producers hold; seeks, step backs, branches, gaps and viewing-country
//! changes reset them at once.
//! Renderer seeds, deltas and earlier particle buffers are not recorded.
use std::collections::BTreeSet;
use std::sync::OnceLock;
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::map::province_admission::side_planes;
use viewer_foundation::map::province_admission::views as province_admission_views;
use viewer_foundation::map::province_schedule::Schedule;
use viewer_foundation::map::textures::load_file;
use viewer_foundation::map::{
    camera::Camera,
    gpu_contract::Targets,
    textures::{self, ColorSpace},
};
mod fade;
mod motion;
mod native;
mod sort;
use eframe::wgpu;
use installed_content_core::{ResolvedLayers, parser, parser::Item, parser::Value, particles::Catalog};
use installed_content_image::{dds, native::Decode};
pub use fade::RegionProvinces;
pub(crate) use native::EntitySimulations;
pub use native::Inputs as NativeInputs;
use std::{collections::BTreeMap, io, sync::Arc};

fn invalid(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.into())
}
pub(super) fn get<'a>(v: &'a [Item], key: &str) -> Option<&'a Value> {
    v.iter()
        .rev()
        .find(|i| i.key.as_deref() == Some(key.as_bytes()))
        .map(|i| &i.value)
}
pub(super) fn block<'a>(v: &'a [Item], key: &str) -> &'a [Item] {
    get(v, key).and_then(Value::items).unwrap_or(&[])
}
pub(super) fn text<'a>(v: &'a [Item], key: &str) -> &'a str {
    get(v, key).and_then(Value::scalar_str).unwrap_or("")
}
pub(super) fn tokens(v: &Value) -> Vec<&str> {
    match v {
        Value::Scalar(_) => v.scalar_str().into_iter().collect(),
        _ => v
            .items()
            .unwrap_or(&[])
            .iter()
            .filter_map(|i| i.value.scalar_str())
            .collect(),
    }
}
pub(super) fn number(v: &[Item], key: &str, default: f32) -> f32 {
    get(v, key)
        .and_then(|v| {
            tokens(v)
                .first()
                .and_then(|s| s.split(',').next()?.parse().ok())
        })
        .unwrap_or(default)
}

/// A parameter as its two comma tracks give it: `{ base spread }`. Each track keeps its last
/// number; the base track's first known animation name is its curve.
#[derive(Debug, Clone, Default)]
struct Parameter {
    base: f32,
    spread: f32,
    curve: Option<Curve>,
}
impl Parameter {
    fn read(v: &[Item], key: &str, default: f32, curves: &BTreeMap<String, Curve>) -> Self {
        let t = get(v, key).map(tokens).unwrap_or_default();
        let last_number = |track: Option<&&str>| {
            track?
                .split(',')
                .filter_map(|item| item.parse::<f32>().ok())
                .last()
        };
        let base = last_number(t.first()).unwrap_or(default);
        let curve = t
            .first()
            .into_iter()
            .flat_map(|track| track.split(','))
            .find_map(|item| curves.get(item))
            .cloned();
        let spread = last_number(t.get(1)).unwrap_or(0.0);
        Self {
            base,
            spread,
            curve,
        }
    }
    fn sample(&self, random: f32, age: f32) -> f32 {
        self.animate(self.base + (random * 2.0 - 1.0) * self.spread, age)
    }
    fn animate(&self, value: f32, age: f32) -> f32 {
        value * self.curve.as_ref().map_or(1.0, |c| c.sample(age))
    }
    /// The value the game's evaluator gives outside a particle's life, at subsystem age
    /// `time`: a spawn-time animation applies to the base at that age, then a nonzero spread
    /// draws once around it. A constant draws nothing.
    fn at(&self, seed: &mut u64, time: f32) -> f32 {
        let base = match &self.curve {
            Some(curve) if curve.spawn => curve.apply(self.base, time),
            _ => self.base,
        };
        if self.spread != 0.0 {
            random(seed) * (self.spread + self.spread) + (base - self.spread)
        } else {
            base
        }
    }
}

/// How an animation combines its sample with the value it animates.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Add,
    Mul,
    Abs,
}

// Entity-light events retain their separately Unverified linear evaluator.
// Weather and other particle parameters use the baked Curve below.
pub(super) fn curve(knots: &[[f32; 2]], x: f32) -> f32 {
    for p in knots.windows(2) {
        if x <= p[1][0] {
            let t = ((x - p[0][0]) / (p[1][0] - p[0][0]).max(1e-6)).clamp(0.0, 1.0);
            return p[0][1] + t * (p[1][1] - p[0][1]);
        }
    }
    knots.last().map_or(1.0, |p| p[1])
}

/// Bake 64 Catmull-Rom samples retaining extrapolated endpoint controls. Read them linearly by
/// normalized age.
#[derive(Debug, Clone)]
struct Curve {
    samples: [f32; 64],
    start: f32,
    duration: f32,
    op: Op,
    /// `time = spawn`, the default: the curve runs on subsystem age where a value is
    /// evaluated outside a particle's life.
    spawn: bool,
    repeat: bool,
}
impl Curve {
    /// The value `op` gives at subsystem age `time`; a repeating curve wraps at its end.
    fn apply(&self, value: f32, time: f32) -> f32 {
        let time = if self.repeat {
            time % (self.start + self.duration)
        } else {
            time
        };
        let sample = self.sample(time);
        match self.op {
            Op::Add => value + sample,
            Op::Mul => value * sample,
            Op::Abs => sample,
        }
    }

    fn new(knots: &[[f32; 2]], start: f32, duration: f32, min: f32, max: f32) -> Self {
        let mut controls = Vec::new();
        if knots.len() >= 2 {
            controls.push(std::array::from_fn(|i| 2.0 * knots[0][i] - knots[1][i]));
            controls.extend_from_slice(knots);
            let n = knots.len();
            controls.push(std::array::from_fn(|i| {
                2.0 * knots[n - 1][i] - knots[n - 2][i]
            }));
        }
        let samples = std::array::from_fn(|i| {
            if controls.len() < 4 {
                return 0.0;
            }
            let x = i as f32 / 63.0;
            let k = (2..controls.len() - 1)
                .find(|&k| x < controls[k][0])
                .unwrap_or(controls.len() - 2);
            let t = (x - controls[k - 1][0]) / (controls[k][0] - controls[k - 1][0]);
            let [a, b, c, d] = std::array::from_fn(|j| controls[k - 2 + j][1]);
            let t2 = t * t;
            let t3 = t2 * t;
            let y = 0.5
                * (a * (2.0 * t2 - t3 - t)
                    + b * (3.0 * t3 - 5.0 * t2 + 2.0)
                    + c * (4.0 * t2 - 3.0 * t3 + t)
                    + d * (t3 - t2));
            min + y * (max - min)
        });
        Self {
            samples,
            start,
            duration,
            op: Op::Mul,
            spawn: true,
            repeat: false,
        }
    }
    fn sample(&self, age: f32) -> f32 {
        // Native uses 64 here, despite baking at i/63, then clamps at 63.
        let x = ((age - self.start) / self.duration * 64.0).clamp(0.0, 63.0);
        let i = (x as usize).min(62);
        self.samples[i] + (self.samples[i + 1] - self.samples[i]) * (x - i as f32)
    }
}
#[derive(Debug, Clone)]
pub struct Emitter {
    /// Authored weather entity scale; generic model events use their event transform.
    scale: f32,
    page: usize,
    registration: usize,
    /// Weather particle-reference ordinal; its subsystems share one actor.
    actor: usize,
    radius: Parameter,
    sphere_pitch: Parameter,
    sphere_yaw: Parameter,
    emitter_yaw: Parameter,
    emitter_pitch: Parameter,
    position: [Parameter; 3],
    velocity: Parameter,
    pitch: Parameter,
    yaw: Parameter,
    life: Parameter,
    mass: Parameter,
    start: Parameter,
    duration: Parameter,
    size: Parameter,
    rotation: [Parameter; 3],
    spin: [Parameter; 3],
    color: [Parameter; 4],
    emission: Parameter,
    max: usize,
    /// The subsystem's forces in its own list order; a name listed twice applies twice.
    forces: Vec<Force>,
    /// The division of each of the particle system's turbulence fields, in declaration order.
    noise_divisions: Vec<usize>,
    /// The system's first drawn subsystem, which holds the fields its subsystems share.
    leads: bool,
    /// Seconds after its weather entity's state begins that the particle starts.
    delay: f32,
    local_space: bool,
    render: installed_content_core::particles::RenderDescriptor,
    system_local_space: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ForceKind {
    Planar,
    Point,
    Vortex,
    Friction,
    Turbulence,
    Spin,
}

/// One particle system `force` a subsystem names.
#[derive(Debug, Clone)]
struct Force {
    kind: ForceKind,
    amount: Parameter,
    position: [f32; 3],
    direction: [f32; 3],
    /// The sine and cosine of the authored yaw; both zero when it is zero.
    yaw: [f32; 2],
    /// `local_force`: the centre moves with the subsystem's transform.
    local: bool,
}

/// One force's operands for a tick, its amount drawn.
#[derive(Debug, Clone)]
enum Applied {
    Planar {
        direction: [f32; 3],
        amount: f32,
    },
    Point {
        center: [f32; 3],
        amount: f32,
    },
    Vortex {
        center: [f32; 3],
        axis: [f32; 3],
        yaw: [f32; 2],
        amount: f32,
    },
    Friction(f32),
    Turbulence {
        noise: usize,
        amount: f32,
    },
    Spin {
        center: [f32; 3],
        forward: [f32; 9],
        back: [f32; 9],
        /// Displacement scale times the step squared.
        step: f32,
    },
}

/// The rotation by `angle` radians about the unit `axis`, rows of a matrix row vectors
/// multiply.
fn axis_angle(axis: [f32; 3], angle: f32) -> [f32; 9] {
    let (s, c) = (angle.sin(), angle.cos());
    let [x, y, z] = axis;
    let t = 1.0 - c;
    let zx = z * x * t;
    let zy = z * y * t;
    let xy = x * y * t;
    [
        x * x * t + c,
        z * s + xy,
        zx - y * s,
        xy - z * s,
        y * y * t + c,
        x * s + zy,
        y * s + zx,
        zy - x * s,
        z * z * t + c,
    ]
}

impl Applied {
    /// `force` of the tick, with its amount drawn, the subsystem's displacement `scale`,
    /// the translation a local force's centre follows, and the turbulence fields used so far.
    fn new(
        force: &Force,
        amount: f32,
        scale: f32,
        translation: [f32; 3],
        dt: f32,
        noise: &mut usize,
    ) -> Self {
        let offset = if force.local { translation } else { [0.0; 3] };
        let center = std::array::from_fn(|i| force.position[i] * scale + offset[i]);
        match force.kind {
            ForceKind::Planar => Self::Planar {
                direction: force.direction,
                amount,
            },
            ForceKind::Point => Self::Point {
                center,
                amount: amount * scale,
            },
            ForceKind::Vortex => Self::Vortex {
                center,
                axis: force.direction,
                yaw: force.yaw,
                amount: amount * scale,
            },
            ForceKind::Friction => Self::Friction(amount),
            ForceKind::Turbulence => {
                *noise += 1;
                Self::Turbulence {
                    noise: *noise - 1,
                    amount,
                }
            }
            ForceKind::Spin => {
                let axis = unit(force.direction);
                Self::Spin {
                    center,
                    forward: axis_angle(axis, -(amount * dt)),
                    back: axis_angle(axis, -(-amount * dt)),
                    step: scale * dt * dt,
                }
            }
        }
    }

    /// Adds this force on a particle at `p` moving at `v`, of `mass`, to `force`.
    fn add(
        &self,
        force: &mut [f32; 3],
        p: [f32; 3],
        v: [f32; 3],
        mass: f32,
        dt: f32,
        noise: &[Noise],
    ) {
        match *self {
            Self::Planar { direction, amount } => {
                for i in 0..3 {
                    force[i] += direction[i] * amount;
                }
            }
            Self::Point { center, amount } => {
                let d: [f32; 3] = std::array::from_fn(|i| p[i] - center[i]);
                let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if length > 0.01 {
                    let k = amount / length;
                    for i in 0..3 {
                        force[i] += k * d[i];
                    }
                }
            }
            Self::Vortex {
                center,
                axis: a,
                yaw: [s, c],
                amount,
            } => {
                let mut d: [f32; 3] = std::array::from_fn(|i| p[i] - center[i]);
                let along = a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
                for i in 0..3 {
                    d[i] -= a[i] * along;
                }
                let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if length > 0.01 {
                    let k = amount / length;
                    if s == 0.0 && c == 0.0 {
                        for i in 0..3 {
                            force[i] += k * d[i];
                        }
                    } else {
                        force[0] += ((a[2] * d[1] - a[1] * d[2]) * s + d[0] * c) * k;
                        force[1] += ((a[0] * d[2] - a[2] * d[0]) * s + c * d[1]) * k;
                        force[2] += ((a[1] * d[0] - a[0] * d[1]) * s + c * d[2]) * k;
                    }
                }
            }
            Self::Friction(amount) => {
                for i in 0..3 {
                    force[i] -= v[i] * amount;
                }
            }
            Self::Turbulence { noise: at, amount } => {
                // A subsystem advanced apart from its system's leader has no fields to read.
                if let Some(field) = noise.get(at) {
                    let n = field.force(p);
                    for i in 0..3 {
                        force[i] += amount * n[i];
                    }
                }
            }
            Self::Spin {
                center: c,
                forward: m,
                back: n,
                step,
            } => {
                let d: [f32; 3] = std::array::from_fn(|i| p[i] - c[i]);
                let rotated = [
                    d[0] * m[0] + d[1] * m[3] + d[2] * m[6] + c[0],
                    d[0] * m[1] + d[1] * m[4] + d[2] * m[7] + c[1],
                    d[0] * m[2] + d[1] * m[5] + d[2] * m[8] + c[2],
                ];
                let mut u = [
                    p[0] - (n[3] * d[1] + n[0] * d[0] + n[6] * d[2] + c[0]),
                    p[1] - (n[4] * d[1] + n[1] * d[0] + n[7] * d[2] + c[1]),
                    p[2] - (n[5] * d[1] + n[2] * d[0] + n[8] * d[2] + c[2]),
                ];
                let length = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
                if length != 0.0 {
                    u = u.map(|x| x / length);
                }
                let along = v[1] * u[1] + v[0] * u[0] + v[2] * u[2];
                let k = step / mass;
                for i in 0..3 {
                    force[i] += ((rotated[i] - p[i]) - u[i] * along * dt) / k;
                }
            }
        }
    }
}
/// Which of several particle blocks of one name a loader takes.
#[derive(Clone, Copy)]
pub enum Pick {
    /// The weather systems take the last.
    Last,
    /// The entity particles take the first.
    First,
}

/// The `particle` block named `name` in the particle file `path`.
pub fn particle_root<'a>(
    document: &'a parser::Document,
    path: &str,
    name: &str,
    which: Pick,
) -> io::Result<&'a [Item]> {
    let mut named = document
        .get_all("particle")
        .filter_map(Value::items)
        .filter(|items| text(items, "name") == name);
    match which {
        Pick::Last => named.last(),
        Pick::First => named.next(),
    }
    .ok_or_else(|| invalid(format!("{path} has no particle {name}")))
}

pub fn read_emitters(
    layers: &ResolvedLayers,
    decode: Decode,
    root: &[Item],
    textures: &mut Vec<dds::Texture>,
    pages: &mut BTreeMap<String, usize>,
) -> io::Result<Vec<Emitter>> {
    let mut emitters = parse_emitters(root)?;
    for emitter in &mut emitters {
        let file = emitter.render.texture.as_str();
        emitter.page = if let Some(&p) = pages.get(file) {
            p
        } else {
            let p = textures.len();
            textures.push(load_file(
                file,
                &|path| layers.read_uncovered_path(path),
                decode,
            )?);
            pages.insert(file.to_owned(), p);
            p
        };
    }
    Ok(emitters)
}

/// A system's `force` block, as each subsystem that names it reads it.
struct SystemForce<'a> {
    name: &'a str,
    /// `None` for a type the game does not know.
    kind: Option<ForceKind>,
    kind_text: String,
    force: Force,
    division: usize,
}

fn system_forces<'a>(root: &'a [Item], curves: &BTreeMap<String, Curve>) -> Vec<SystemForce<'a>> {
    let vector = |f: &[Item], key: &str| {
        let values = get(f, key).map(tokens).unwrap_or_default();
        std::array::from_fn(|i| values.get(i).and_then(|v| v.parse().ok()).unwrap_or(0.0))
    };
    root.iter()
        .filter(|i| i.key.as_deref() == Some(b"force"))
        .map(|f| {
            let f = f.value.items().unwrap_or(&[]);
            let kind_text = text(f, "type").to_ascii_lowercase();
            let kind = match kind_text.as_str() {
                "planar" => Some(ForceKind::Planar),
                "point" => Some(ForceKind::Point),
                "vortex" => Some(ForceKind::Vortex),
                "friction" => Some(ForceKind::Friction),
                "turbulence" => Some(ForceKind::Turbulence),
                "spin" => Some(ForceKind::Spin),
                _ => None,
            };
            let yaw = number(f, "yaw", 0.0);
            let yaw = yaw * std::f32::consts::PI / 180.0;
            SystemForce {
                name: text(f, "name"),
                kind,
                kind_text,
                force: Force {
                    kind: kind.unwrap_or(ForceKind::Planar),
                    amount: Parameter::read(f, "amount", 0.0, curves),
                    position: vector(f, "position"),
                    direction: vector(f, "direction"),
                    yaw: if yaw == 0.0 {
                        [0.0; 2]
                    } else {
                        [yaw.sin(), yaw.cos()]
                    },
                    local: parser::read_bool_field(f, "local_force", false),
                },
                division: (number(f, "division", 1.0) as usize)
                    .clamp(1, 256)
                    .next_power_of_two(),
            }
        })
        .collect()
}

/// The emitters of a `particle` block, their texture pages still unassigned. A subsystem
/// that nests subsystems, or names a force of a type the game does not know, refuses the
/// block by name.
fn parse_emitters(root: &[Item]) -> io::Result<Vec<Emitter>> {
    let particle = text(root, "name");
    let mut curves = BTreeMap::new();
    for a in root
        .iter()
        .filter(|i| i.key.as_deref() == Some(b"animation"))
    {
        let a = a.value.items().unwrap_or(&[]);
        let values = get(a, "curve")
            .map(tokens)
            .unwrap_or_default()
            .iter()
            .filter_map(|s| s.parse::<f32>().ok())
            .collect::<Vec<_>>();
        let mut curve = Curve::new(
            &values
                .chunks_exact(2)
                .map(|p| [p[0], p[1]])
                .collect::<Vec<_>>(),
            number(a, "start", 0.0),
            number(a, "duration", 1.0),
            number(a, "minValue", 0.0),
            number(a, "maxValue", 1.0),
        );
        match text(a, "op").to_ascii_lowercase().as_str() {
            "add" => curve.op = Op::Add,
            "mul" => curve.op = Op::Mul,
            "abs" => curve.op = Op::Abs,
            _ => {}
        }
        curve.spawn = !matches!(text(a, "time"), "life" | "life_abs" | "system");
        curve.repeat = parser::read_bool_field(a, "repeat", false);
        curves.insert(text(a, "name").to_owned(), curve);
    }
    let system = system_forces(root, &curves);
    let turbulence: Vec<usize> = system
        .iter()
        .filter(|f| f.kind == Some(ForceKind::Turbulence))
        .map(|f| f.division)
        .collect();
    let mut emitters = Vec::new();
    for s in root
        .iter()
        .filter(|i| i.key.as_deref() == Some(b"subsystem"))
    {
        let s = s.value.items().unwrap_or(&[]);
        // Hidden subsystems contribute no emitter.
        if parser::read_bool_field(s, "hide", false) {
            continue;
        }
        let subsystem = text(s, "name");
        let refuse =
            |why: String| invalid(format!("particle {particle} subsystem {subsystem}: {why}"));
        if s.iter().any(|i| i.key.as_deref() == Some(b"subsystem")) {
            return Err(refuse(
                "nests particle subsystems, which the map cannot present".into(),
            ));
        }
        let render = installed_content_core::particles::RenderDescriptor::read(s)?;
        let param = |k, d| Parameter::read(s, k, d, &curves);
        // The game looks each listed name up among the system's forces, first match, and
        // skips a number or a name it does not find.
        let mut forces = Vec::new();
        let mut turbulent = 0;
        for name in text(s, "force").split(',') {
            if name.is_empty() || name.parse::<f32>().is_ok() {
                continue;
            }
            let Some(found) = system.iter().find(|f| f.name == name) else {
                continue;
            };
            let kind = found.kind.ok_or_else(|| {
                refuse(format!(
                    "force {name} has unknown type {:?}",
                    found.kind_text
                ))
            })?;
            // The k-th turbulence force applied reads the system's k-th field.
            if kind == ForceKind::Turbulence {
                turbulent += 1;
                if turbulent > turbulence.len() {
                    return Err(refuse(format!(
                        "names more turbulence forces than the system declares, at {name}"
                    )));
                }
            }
            forces.push(found.force.clone());
        }
        let position = block(s, "position");
        let col = block(s, "color");
        emitters.push(Emitter {
            scale: 1.0,
            page: 0,
            registration: 0,
            actor: 0,
            radius: param("sphere_emitter_radius", 0.0),
            sphere_pitch: param("sphere_emitter_pitch", 0.0),
            sphere_yaw: param("sphere_emitter_yaw", 0.0),
            emitter_yaw: param("emitter_yaw", 0.0),
            emitter_pitch: param("emitter_pitch", 0.0),
            position: ["x", "y", "z"].map(|k| Parameter::read(position, k, 0.0, &curves)),
            velocity: param("velocity", 0.0),
            pitch: param("velocity_pitch", 0.0),
            yaw: param("velocity_yaw", 0.0),
            life: param("life", 1.0),
            mass: param("mass", 1.0),
            start: param("start", 0.0),
            duration: param("duration", -1.0),
            size: param("size", 1.0),
            rotation: ["particle_yaw", "particle_pitch", "rotation"].map(|k| param(k, 0.0)),
            spin: [
                "rotation_speed_yaw",
                "rotation_speed_pitch",
                "rotation_speed",
            ]
            .map(|k| param(k, 0.0)),
            color: ["x", "y", "z", "alpha"].map(|k| Parameter::read(col, k, 255.0, &curves)),
            emission: param("emission", 0.0),
            max: installed_content_core::particles::subsystem_capacity(s)?,
            forces,
            noise_divisions: turbulence.clone(),
            leads: emitters.is_empty(),
            delay: 0.0,
            local_space: render.local_space,
            render,
            system_local_space: root
                .iter()
                .filter(|i| i.key.as_deref() == Some(b"subsystem"))
                // Any local subsystem makes the system use local space.
                .any(|item| {
                    parser::read_bool_field(item.value.items().unwrap_or(&[]), "local_space", false)
                }),
        });
    }
    let slots = system_capacity(&emitters);
    if slots > native::MAX_PARTICLES {
        return Err(invalid(format!(
            "particle {particle} holds {slots} particles, more than the {} the renderer's buffers hold",
            native::MAX_PARTICLES
        )));
    }
    Ok(emitters)
}

/// The particles one system's drawn subsystems hold at most.
fn system_capacity(emitters: &[Emitter]) -> usize {
    emitters.iter().map(|e| e.max).sum()
}
/// One emitter batch with its texture files still local, in local page
/// order, for the ordered merge in [`Assets::load`].
type LocalEmitters = (Vec<Emitter>, Vec<(String, dds::Texture)>);
/// One descriptor pair's emitters: authored `entity`, then `fading_entity`.
struct WeatherPair {
    members: [Vec<Emitter>; 2],
    radii: [Option<f32>; 2],
    /// Each member entity's sound events.
    sounds: [Vec<installed_content_weather::weather::WeatherSound>; 2],
}
/// One pair job's result: its bank, emitters, local pages and why it is withheld.
type LocalSystem = (
    (usize, bool),
    WeatherPair,
    Vec<(String, dds::Texture)>,
    Option<String>,
);

/// Move one job's local texture pages into the shared pages, remapping the
/// emitters' page ids to the global first-seen order of the serial load.
fn merge_pages(
    pages: &mut Vec<dds::Texture>,
    global: &mut BTreeMap<String, usize>,
    emitters: &mut [Emitter],
    local: Vec<(String, dds::Texture)>,
) {
    let mut remap = Vec::with_capacity(local.len());
    for (file, texture) in local {
        let id = match global.get(&file) {
            Some(&id) => id,
            None => {
                let id = pages.len();
                pages.push(texture);
                global.insert(file, id);
                id
            }
        };
        remap.push(id);
    }
    for emitter in emitters.iter_mut() {
        emitter.page = remap[emitter.page];
    }
}
/// `NDefines.NGraphics.WEATHER_*`, the camera fade and playback operands
/// reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraDefines {
    pub distance_cutoff: f32,
    pub distance_fade_length: f32,
    pub zoom_in_cutoff: f32,
    pub zoom_in_fade_length: f32,
    pub zoom_in_fade_factor: f32,
    pub playback_rate: f32,
    pub playback_rate_cutoff: f32,
    pub playback_rate_length: f32,
}

impl CameraDefines {
    pub fn load(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let number = |key: &str| -> io::Result<f32> {
            let value = defines.number("NGraphics", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.NGraphics.{key} is outside f32")))
            }
        };
        Ok(Self {
            distance_cutoff: number("WEATHER_DISTANCE_CUTOFF")?,
            distance_fade_length: number("WEATHER_DISTANCE_FADE_LENGTH")?,
            zoom_in_cutoff: number("WEATHER_ZOOM_IN_CUTOFF")?,
            zoom_in_fade_length: number("WEATHER_ZOOM_IN_FADE_LENGTH")?,
            zoom_in_fade_factor: number("WEATHER_ZOOM_IN_FADE_FACTOR")?,
            playback_rate: number("WEATHER_PLAYBACK_RATE")?,
            playback_rate_cutoff: number("WEATHER_PLAYBACK_RATE_CUTOFF")?,
            playback_rate_length: number("WEATHER_PLAYBACK_RATE_LENGTH")?,
        })
    }
}

struct Anchor {
    region: u32,
    position: [f32; 3],
    small: bool,
}
/// Authored resources, loaded once per document and shared with its GPU scene.
pub struct Assets {
    effect: Arc<shader_pipeline::library::Effect>,
    max_particles: usize,
    graphics: CameraDefines,
    anchors: Vec<Anchor>,
    anchors_by_region: BTreeMap<u32, Vec<usize>>,
    region_provinces: RegionProvinces,
    systems: BTreeMap<(usize, bool), Vec<WeatherPair>>,
    pages: Vec<dds::Texture>,
    entities: BTreeMap<String, (f32, Vec<Emitter>)>,
    /// Weather effect pairs, `pdxparticle` objects, particle files and anchor lines the map
    /// cannot present as the game does, each with why; each draws nothing, model events naming
    /// a withheld object included, and the rest load.
    pub withheld: Vec<String>,
    lights: super::entity_effects::Lights,
}

/// A `pdxparticle` object's scale and emitters from its parsed particle file, with their local
/// texture pages.
fn read_object(
    layers: &ResolvedLayers,
    decode: Decode,
    document: &parser::Document,
    path: &str,
    particle: &installed_content_core::particles::Particle,
) -> io::Result<(f32, LocalEmitters)> {
    let root = particle_root(document, path, &particle.name, Pick::First)?;
    let mut textures = Vec::new();
    let mut local = BTreeMap::new();
    let mut emitters = read_emitters(layers, decode, root, &mut textures, &mut local)?;
    for emitter in &mut emitters {
        emitter.registration = particle.registration;
    }
    let mut files = vec![String::new(); textures.len()];
    for (file, id) in local {
        files[id] = file;
    }
    Ok((
        particle.scale,
        (emitters, files.into_iter().zip(textures).collect()),
    ))
}

/// A weather pair's emitters read from its particle files, with their local texture pages.
fn read_pair(
    layers: &ResolvedLayers,
    decode: Decode,
    pair: &installed_content_weather::weather::EffectPair,
) -> io::Result<([Vec<Emitter>; 2], Vec<(String, dds::Texture)>)> {
    let mut textures = Vec::new();
    let mut pages = BTreeMap::new();
    let mut emitters = [Vec::new(), Vec::new()];
    for (slot, references) in [&pair.entity, &pair.fading_entity].into_iter().enumerate() {
        for (actor, reference) in references.iter().enumerate() {
            let particle = &reference.particle;
            let document = parser::parse(&particle.path.read_uncovered(layers)?)?;
            let root = particle_root(&document, &particle.path.text(), &particle.name, Pick::Last)?;
            let mut parsed = read_emitters(layers, decode, root, &mut textures, &mut pages)
                .map_err(|error| invalid(format!("{}: {error}", particle.path)))?;
            for emitter in &mut parsed {
                emitter.scale = particle.scale;
                emitter.registration = particle.registration;
                emitter.actor = actor;
                emitter.delay = reference.delay;
            }
            emitters[slot].extend(parsed);
        }
    }
    let mut files = vec![String::new(); textures.len()];
    for (file, id) in pages {
        files[id] = file;
    }
    Ok((emitters, files.into_iter().zip(textures).collect()))
}
impl Assets {
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content_core::defines::Defines,
        map: [u32; 2],
        region_provinces: RegionProvinces,
        decode: Decode,
    ) -> io::Result<Arc<Self>> {
        let catalog = Catalog::read(layers)?;
        let mut out = Self {
            effect: super::particle_shader::load(layers, map, decode)?,
            // Only systems that load count toward the shared buffers.
            max_particles: 0,
            graphics: CameraDefines::load(defines)?,
            anchors: Vec::new(),
            anchors_by_region: BTreeMap::new(),
            region_provinces,
            systems: BTreeMap::new(),
            pages: Vec::new(),
            entities: BTreeMap::new(),
            withheld: catalog.refused().to_vec(),
            lights: super::entity_effects::Lights::load(layers)?,
        };
        const POSITIONS: &str = "map/weatherpositions.txt";
        let (positions, skipped) = match layers.read_uncovered(POSITIONS) {
            Ok(bytes) => installed_content_core::positions::parse_weather_positions(&bytes),
            // The game logs a missing file and places no anchors.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                out.withheld
                    .push(format!("{POSITIONS} is missing, so no weather is placed"));
                (Vec::new(), Vec::new())
            }
            Err(error) => return Err(error),
        };
        out.withheld.extend(skipped.into_iter().map(|(line, fields)| {
            format!("{POSITIONS} line {line} has {fields} fields, not region;x;y;z;size")
        }));
        for position in positions {
            // The installed content an id outside its region table under the null region at 0, and
            // reports and skips a line whose region is not a defined strategic region.
            let region = u32::try_from(position.region)
                .ok()
                .filter(|&region| out.region_provinces.contains(region));
            let Some(region) = region else {
                out.withheld.push(format!(
                    "{POSITIONS} line {} names region {}, which is not a strategic region",
                    position.line, position.region
                ));
                continue;
            };
            out.anchors_by_region
                .entry(region)
                .or_default()
                .push(out.anchors.len());
            out.anchors.push(Anchor {
                region,
                position: [position.x, position.y, position.z],
                small: position.small,
            });
        }
        let descriptors = installed_content_weather::weather::effects(layers, &catalog)?;
        let keys: Vec<_> = descriptors
            .iter()
            .flat_map(|(&key, pairs)| pairs.iter().map(move |pair| (key, pair)))
            .collect();
        // Parse the actual named particle blocks. Neither the entity's name nor
        // pdxparticle.type establishes a filename; content resolves both. A pair
        // that cannot be read keeps its place in its bank and draws nothing.
        let systems: Vec<LocalSystem> = std::thread::scope(|scope| {
            keys.iter()
                .map(|(key, pair)| {
                    job_priority::spawn_in(scope, move || {
                        let read = match &pair.withheld {
                            Some(why) => Err(why.clone()),
                            None => read_pair(layers, decode, pair).map_err(|e| e.to_string()),
                        };
                        let (members, local, withheld) = match read {
                            Ok((members, local)) => (members, local, None),
                            Err(why) => (
                                [Vec::new(), Vec::new()],
                                Vec::new(),
                                Some(format!("{}: {why}", pair.name)),
                            ),
                        };
                        let shown = withheld.is_none();
                        (
                            *key,
                            WeatherPair {
                                members,
                                radii: pair.parents.each_ref().map(|parent| {
                                    parent.as_ref().filter(|_| shown).map(|p| p.radius)
                                }),
                                sounds: pair.parents.each_ref().map(|parent| {
                                    parent
                                        .as_ref()
                                        .filter(|_| shown)
                                        .map(|p| p.sounds.clone())
                                        .unwrap_or_default()
                                }),
                            },
                            local,
                            withheld,
                        )
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("weather system loader panicked"))
                .collect()
        });
        let mut global = BTreeMap::new();
        for system in systems {
            let (
                key,
                WeatherPair {
                    members: [mut entity, fading],
                    radii,
                    sounds,
                },
                local,
                withheld,
            ) = system;
            out.withheld.extend(withheld);
            // Each particle reference of a member is one system.
            for member in [&entity, &fading] {
                for system in member.chunk_by(|a, b| a.actor == b.actor) {
                    out.max_particles = out.max_particles.max(system_capacity(system));
                }
            }
            let entity_count = entity.len();
            entity.extend(fading);
            merge_pages(&mut out.pages, &mut global, &mut entity, local);
            let fading = entity.split_off(entity_count);
            out.systems.entry(key).or_default().push(WeatherPair {
                members: [entity, fading],
                radii,
                sounds,
            });
        }
        // Every declared pdxparticle object, grouped by its particle file so
        // each file parses once; files decode on scoped threads and merge in
        // path order.
        let mut by_file: BTreeMap<
            installed_content_core::script_files::ScriptPath,
            Vec<(String, installed_content_core::particles::Particle)>,
        > = BTreeMap::new();
        for object in catalog.objects() {
            match catalog.resolve(object) {
                Ok(particle) => by_file
                    .entry(particle.path.clone())
                    .or_default()
                    .push((object.to_owned(), particle)),
                Err(why) => out.withheld.push(why.to_string()),
            }
        }
        // An object that cannot be read is withheld by name; model events naming it draw nothing.
        type Parsed = Vec<(String, Result<(f32, LocalEmitters), String>)>;
        let files: Vec<Parsed> = std::thread::scope(|scope| {
            by_file
                .iter()
                .map(|(path, objects)| {
                    job_priority::spawn_in(scope, move || {
                        let document = path
                            .read_uncovered(layers)
                            .and_then(|bytes| parser::parse(&bytes))
                            .map_err(|e| e.to_string());
                        let text = path.text();
                        let path: &str = &text;
                        objects
                            .iter()
                            .map(|(object, particle)| {
                                let read = document.as_ref().map_err(String::clone).and_then(
                                    |document| {
                                        read_object(layers, decode, document, path, particle)
                                            .map_err(|e| e.to_string())
                                    },
                                );
                                let read = read.map_err(|why| {
                                    format!(
                                        "pdxparticle {object} ({path} particle {}): {why}",
                                        particle.name
                                    )
                                });
                                (object.clone(), read)
                            })
                            .collect()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("particle loader panicked"))
                .collect()
        });
        for (name, read) in files.into_iter().flatten() {
            match read {
                Ok((scale, (mut emitters, local))) => {
                    merge_pages(&mut out.pages, &mut global, &mut emitters, local);
                    out.max_particles = out.max_particles.max(system_capacity(&emitters));
                    out.entities.insert(name, (scale, emitters));
                }
                Err(why) => out.withheld.push(why),
            }
        }
        Ok(Arc::new(out))
    }
}

/// Hourly flags. Weather is global simulation state; the engine's dispatch
/// has no country/intel predicate, so every recorded perspective uses it.
pub use map_display_data::WeatherField as Field;

/// The camera fade, whether the camera stands in the far band (above its start, the cutoff
/// included) and the playback rate ([`fade`] applies the first two).
fn gates(h: f32, defines: &CameraDefines) -> (f32, bool, f32) {
    let (alpha, far) = if h < defines.zoom_in_cutoff {
        (defines.zoom_in_fade_factor, false)
    } else if h > defines.distance_cutoff {
        (0.0, true)
    } else if h > defines.distance_cutoff - defines.distance_fade_length {
        ((defines.distance_cutoff - h) / defines.distance_fade_length, true)
    } else if h < defines.zoom_in_cutoff + defines.zoom_in_fade_length {
        let near = (h - defines.zoom_in_cutoff) / defines.zoom_in_fade_length;
        (near * (1.0 - defines.zoom_in_fade_factor) + defines.zoom_in_fade_factor, false)
    } else {
        (1.0, false)
    };
    let rate = ((defines.playback_rate_cutoff - h) / defines.playback_rate_length)
        * (1.0 - defines.playback_rate)
        + defines.playback_rate;
    (alpha, far, rate.clamp(defines.playback_rate, 1.0))
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct RegionPresentation {
    kind: usize,
    seed_hour: i32,
}

/// Each weather region's kind and the hour it began. The pass keeps two: the kinds the field
/// asks for, and the kinds whose pairs the regions' producers hold ([`Pass::produce`]).
#[derive(Clone, Default)]
struct Presentation(BTreeMap<u32, RegionPresentation>);

impl Presentation {
    fn transition(
        &mut self,
        previous: &Field,
        next: &Field,
        continuity: Continuity,
    ) -> BTreeSet<u32> {
        let continuous =
            continuity.keeps((previous.branch, previous.hour), (next.branch, next.hour));
        let retained = self
            .0
            .iter()
            .filter_map(|(&region, state)| {
                (continuous && next.kinds.get(&region) == Some(&state.kind)).then_some(region)
            })
            .collect::<BTreeSet<_>>();
        self.0.retain(|region, _| retained.contains(region));
        for (&region, &kind) in next.kinds.iter() {
            self.0.entry(region).or_insert(RegionPresentation {
                kind,
                seed_hour: next.hour,
            });
        }
        retained
    }
}

/// Counter hash with draw counter in the low word and seed in the high word. Use the
/// hour/region/anchor presentation seed convention.
fn random_bits(seed: &mut u64) -> u32 {
    let count = *seed as u32;
    let mut v = count
        .wrapping_mul(0xb5297a4d)
        .wrapping_add((*seed >> 32) as u32);
    *seed = (*seed & 0xffff_ffff_0000_0000) | count.wrapping_add(1) as u64;
    v = (v ^ (v >> 8)).wrapping_add(0x68e31da4);
    v = (v ^ v.wrapping_shl(8)).wrapping_mul(0x1b56c4e9);
    v = (v ^ (v >> 8)).wrapping_mul(0x92d68ca2);
    (v ^ (v >> 8)) & 0x7fff_ffff
}
fn random(seed: &mut u64) -> f32 {
    random_bits(seed) as f32 * (1.0 / 2147483648.0)
}

/// Choose each atlas coordinate with a separate counter-hash modulo
/// at birth, stores its low byte, and clears zw. Animation retains that tile.
fn sample_tile(seed: &mut u64, dimensions: [i32; 2]) -> [u8; 4] {
    [
        (random_bits(seed) as i32 % dimensions[0]) as u8,
        (random_bits(seed) as i32 % dimensions[1]) as u8,
        0,
        0,
    ]
}

/// The bank an anchor's pair is drawn from and its place in it. Use the small bank only when
/// nonempty. Every admitted anchor consumes a counter-hash draw, including off-camera anchors;
/// presentation supplies the seed. The draw advances a stream the region's anchors share, so
/// which anchors are walked decides what every later one draws ([`Drawn`]).
fn select_pair_at(
    systems: &BTreeMap<(usize, bool), Vec<WeatherPair>>,
    kind: usize,
    small: bool,
    seed: &mut u64,
) -> Option<((usize, bool), usize)> {
    let key = [(kind, small), (kind, false)]
        .into_iter()
        .find(|key| systems.get(key).is_some_and(|bank| !bank.is_empty()))?;
    let bank = systems.get(&key)?;
    Some((key, random_bits(seed) as usize % bank.len()))
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n == 0.0 { [0.0; 3] } else { v.map(|x| x / n) }
}

/// Native changing gradient field, not independent
/// particle jitter. Three linearly interpolated gradient-noise samples are
/// normalized together, so `amount` is the magnitude before mass division.
#[derive(Clone)]
struct Noise {
    permutation: [u8; 256],
    original: [[f32; 3]; 256],
    axes: [[f32; 3]; 256],
    speed: [f32; 256],
    gradients: [[f32; 3]; 256],
    division: usize,
    slice: usize,
}
impl Noise {
    fn new(seed: &mut u64, division: usize) -> Self {
        fn direction(seed: &mut u64) -> [f32; 3] {
            loop {
                let v = std::array::from_fn(|_| 2.0 * random(seed) - 1.0);
                let n = v.iter().map(|x| x * x).sum::<f32>();
                if (0.001..=1.0).contains(&n) {
                    return unit(v);
                }
            }
        }
        let mut original = [[0.0; 3]; 256];
        let mut axes = original;
        let mut speed = [0.0; 256];
        let mut permutation = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            original[i] = direction(seed);
            axes[i] = direction(seed);
            speed[i] = (1.0 + random(seed)) * std::f32::consts::PI;
        }
        for i in 1..256 {
            permutation.swap(i, random_bits(seed) as usize % (i + 1));
        }
        Self {
            permutation,
            original,
            axes,
            speed,
            gradients: original,
            division,
            slice: 0,
        }
    }
    fn advance(&mut self, time: f32) {
        let count = 256 / self.division;
        for i in self.slice * count..(self.slice + 1) * count {
            let v = self.original[i];
            let a = self.axes[i];
            let (s, c) = (time * self.speed[i]).sin_cos();
            let dot = (0..3).map(|j| a[j] * v[j]).sum::<f32>();
            let cross = [
                a[1] * v[2] - a[2] * v[1],
                a[2] * v[0] - a[0] * v[2],
                a[0] * v[1] - a[1] * v[0],
            ];
            self.gradients[i] =
                std::array::from_fn(|j| v[j] * c + cross[j] * s + a[j] * dot * (1.0 - c));
        }
        self.slice = (self.slice + 1) % self.division;
    }
    fn hash(&self, index: usize) -> usize {
        // Do not mask the second or third gradient index sum. Indices 256..510 read the following
        // current-gradient bytes.
        if index < 256 {
            self.permutation[index] as usize
        } else {
            let offset = index - 256;
            self.gradients[offset / 12][offset % 12 / 4].to_le_bytes()[offset % 4] as usize
        }
    }
    fn scalar(&self, p: [f32; 3]) -> f32 {
        let cell = p.map(|v| v as i32);
        let f = std::array::from_fn::<_, 3, _>(|j| (p[j] - cell[j] as f32).abs());
        let mut value = 0.0;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let corner = [x, y, z];
                    let ix = (cell[0] + x) as u8 as usize;
                    let iy = (cell[1] + y) as u8 as usize;
                    let iz = (cell[2] + z) as u8 as usize;
                    let g = self.gradients[self.hash(self.hash(self.hash(ix) + iy) + iz)];
                    let dot = (0..3)
                        .map(|j| g[j] * (f[j] - corner[j] as f32))
                        .sum::<f32>();
                    let weight = (0..3)
                        .map(|j| if corner[j] == 0 { 1.0 - f[j] } else { f[j] })
                        .product::<f32>();
                    value += dot * weight;
                }
            }
        }
        value
    }
    fn force(&self, p: [f32; 3]) -> [f32; 3] {
        unit([
            self.scalar(p),
            self.scalar([p[1] + 31.416, p[2] - 47.853, p[0] + 12.793]),
            self.scalar([p[2] - 233.145, p[0] - 113.408, p[1] - 185.31]),
        ])
    }
}

#[derive(Clone)]
struct LiveParticle {
    age: f32,
    life: f32,
    mass: f32,
    position: [f32; 3],
    velocity: [f32; 3],
    size: f32,
    rotation: [f32; 3],
    spin: [f32; 3],
    color: [f32; 4],
    tile: [u8; 4],
}
#[derive(Clone)]
struct Simulation {
    /// Prepared off-thread before the first render supplied its clock.
    prepared: bool,
    emitting: bool,
    anchor: [f32; 3],
    initial_seed: u64,
    seed: u64,
    /// The particle system's turbulence fields, held by its leading subsystem and
    /// shared by all of them. On the heap: at 10.5 KB each inline, every simulation
    /// moved in or out of the map on an hour change copied them.
    noise: Vec<Noise>,
    particles: Vec<LiveParticle>,
    accumulator: f32,
    age: f32,
    time: f32,
    previous_translation: Option<[f32; 3]>,
    order: sort::Order,
    /// The subsystem's sampled position and emission duration.
    origin: [f32; 3],
    duration: f32,
    /// This tick's forces, kept for their storage.
    applied: Vec<Applied>,
    /// When, on the presentation clock, the entity event creates a delayed particle system;
    /// until then it holds nothing, samples nothing and submits nothing.
    event_at: Option<f32>,
    /// Not yet created: its system's creation draws are still to be made ([`start_members`]).
    fresh: bool,
}

/// Creates the members of one system that are due at `target`: an undelayed one at once, a
/// delayed one once its event time is behind the clock. As in the game, every created
/// subsystem draws its creation values in order from the one system stream, the leading
/// subsystem's, and only then are the shared turbulence fields seeded from it.
fn start_members(members: &mut [Member<'_>], target: f32, noise: &mut Vec<Noise>) {
    let due = |m: &Member<'_>| m.sim.fresh && m.sim.event_at.is_none_or(|at| at < target);
    if !members.iter().any(due) {
        return;
    }
    let leader = members
        .iter()
        .position(|m| m.emitter.leads)
        .filter(|&at| due(&members[at]));
    let mut stream = leader.map(|at| members[at].sim.seed);
    for member in members.iter_mut() {
        if !due(member) {
            continue;
        }
        member.sim.fresh = false;
        member.sim.event_at = None;
        match stream.as_mut() {
            Some(stream) => member.sim.sample(member.emitter, stream),
            None => {
                let mut own = member.sim.seed;
                member.sim.sample(member.emitter, &mut own);
                member.sim.seed = own;
            }
        }
    }
    if let (Some(at), Some(mut stream)) = (leader, stream) {
        noise.clear();
        for &division in &members[at].emitter.noise_divisions {
            noise.push(Noise::new(&mut stream, division));
        }
        members[at].sim.seed = stream;
    }
}

/// One subsystem of a particle system, advanced with its siblings.
struct Member<'a> {
    sim: &'a mut Simulation,
    emitter: &'a Emitter,
    space: Option<motion::Space>,
}

impl Simulation {
    /// Native particle-runtime construction starts with empty storage. The
    /// producer creates a fresh pair on a weather-kind change, and its active
    /// bit gates later manager updates. Its creation draws wait for its system's first
    /// update ([`start_members`]).
    fn cold(e: &Emitter, seed: u64, anchor: [f32; 3], time: f32) -> Self {
        Self {
            prepared: false,
            emitting: true,
            anchor,
            initial_seed: seed,
            seed,
            noise: Vec::new(),
            particles: Vec::with_capacity(e.max),
            accumulator: 0.0,
            age: 0.0,
            time: time.max(0.0),
            previous_translation: None,
            order: sort::Order::new(&e.render),
            origin: [0.0; 3],
            duration: 0.0,
            applied: Vec::new(),
            event_at: (e.delay > 0.0).then(|| time.max(0.0) + e.delay),
            fresh: true,
        }
    }

    /// The subsystem's creation draws: its position, duration and start, from `stream`.
    fn sample(&mut self, e: &Emitter, stream: &mut u64) {
        self.origin = e.position.each_ref().map(|p| p.at(stream, 0.0));
        self.duration = e.duration.at(stream, 0.0);
        self.age = -e.start.at(stream, 0.0);
    }

    /// Whether the entity event that creates this system has yet to fire.
    fn waiting(&self) -> bool {
        self.event_at.is_some()
    }

    #[cfg(test)]
    fn tick(&mut self, e: &Emitter, dt: f32) {
        let mut noise = std::mem::take(&mut self.noise);
        self.tick_space(e, dt, None, &mut noise);
        self.noise = noise;
    }

    fn tick_space(
        &mut self,
        e: &Emitter,
        dt: f32,
        space: Option<motion::Space>,
        noise: &mut Vec<Noise>,
    ) {
        if self.waiting() {
            return;
        }
        if self.fresh {
            start_members(
                &mut [Member {
                    sim: &mut *self,
                    emitter: e,
                    space,
                }],
                0.0,
                noise,
            );
        }
        if let Some(space) = space
            && self.particles.is_empty()
        {
            // Initialize the birth interpolation endpoint when empty.
            self.previous_translation = Some(space.world.translation);
        }
        // Ordinary particles replace an expired slot
        //with the last live record; trails shift the remaining records. The
        //replacement has not aged yet, so revisit that slot before moving on.
        let mut index = 0;
        while index < self.particles.len() {
            let p = &mut self.particles[index];
            p.age += dt;
            if p.age < 0. || p.age > p.life {
                if e.render.trail {
                    self.particles.remove(index);
                } else {
                    self.particles.swap_remove(index);
                }
            } else {
                index += 1;
            }
        }
        let emitting =
            self.emitting && self.age >= 0.0 && (self.duration < 0.0 || self.age <= self.duration);
        if emitting {
            // A ranged emission draws a new rate each update.
            self.accumulator += e.emission.at(&mut self.seed, self.age).max(0.0) * dt;
        }
        let births = if emitting {
            (self.accumulator as usize).min(e.max - self.particles.len())
        } else {
            0
        };
        self.accumulator -= births as f32;
        // Sample emitter orientation once for this birth batch.
        let basis = if births > 0 {
            emitter_basis(
                e.emitter_yaw.sample(random(&mut self.seed), 0.0),
                e.emitter_pitch.sample(random(&mut self.seed), 0.0),
            )
        } else {
            emitter_basis(0.0, 0.0)
        };
        for ordinal in 0..births {
            let mut sample = |p: &Parameter| p.sample(random(&mut self.seed), 0.0);
            let life = sample(&e.life).max(0.01);
            let mass = sample(&e.mass);
            let r = sample(&e.radius);
            let sp = -sample(&e.sphere_pitch).to_radians();
            let sy = -sample(&e.sphere_yaw).to_radians();
            let pitch = -sample(&e.pitch).to_radians();
            let yaw = -sample(&e.yaw).to_radians();
            let speed = sample(&e.velocity) * space.map_or(1., |space| space.scale);
            let offset = transform(
                basis,
                [
                    r * sp.cos() * sy.cos(),
                    -r * sp.sin(),
                    r * sp.cos() * sy.sin(),
                ],
            );
            let position = space.map_or_else(
                || std::array::from_fn(|j| self.origin[j] + offset[j]),
                |space| {
                    space.birth(
                        self.previous_translation.unwrap_or(space.world.translation),
                        self.origin,
                        offset,
                        ordinal,
                        births,
                    )
                },
            );
            let velocity = transform(
                basis,
                [
                    yaw.cos() * pitch.cos() * speed,
                    -pitch.sin() * speed,
                    yaw.sin() * pitch.cos() * speed,
                ],
            );
            let velocity = space.map_or(velocity, |space| space.vector(velocity));
            self.particles.push(LiveParticle {
                age: 0.0,
                life,
                mass,
                position,
                velocity,
                size: e.size.base + (random(&mut self.seed) * 2.0 - 1.0) * e.size.spread,
                rotation: e
                    .rotation
                    .each_ref()
                    .map(|p| p.sample(random(&mut self.seed), 0.0)),
                spin: e
                    .spin
                    .each_ref()
                    .map(|p| p.sample(random(&mut self.seed), 0.0)),
                color: e.color.each_ref().map(|p| {
                    (p.base + (random(&mut self.seed) * 2.0 - 1.0) * p.spread).clamp(0.0, 255.0)
                }),
                tile: sample_tile(&mut self.seed, e.render.tiles),
            });
        }
        // Each populated subsystem's update advances every field of its system.
        if !self.particles.is_empty() {
            for n in noise.iter_mut() {
                n.advance(self.age);
            }
        }
        // The comparison simulator keeps positions beside its anchor; forces see them placed.
        let (scale, translation, offset) = match space {
            Some(space) => (space.scale, space.world.translation, [0.0; 3]),
            None => (1.0, self.anchor, self.anchor),
        };
        // Each force draws its amount once an update, in list order; a zero amount applies
        // nothing and uses no turbulence field.
        self.applied.clear();
        if !self.particles.is_empty() {
            let mut turbulence = 0;
            for force in &e.forces {
                let amount = force.amount.at(&mut self.seed, self.age);
                if amount != 0.0 {
                    self.applied.push(Applied::new(
                        force,
                        amount,
                        scale,
                        translation,
                        dt,
                        &mut turbulence,
                    ));
                }
            }
        }
        for p in &mut self.particles {
            // Newborns move in this tick too. The comparison simulator uses unit displacement scale;
            // world-space motion uses the runtime's retained displacement scale.
            if e.forces.is_empty() {
                // Without forces the game keeps velocity and only moves.
                let step = scale * dt;
                for j in 0..3 {
                    p.position[j] += p.velocity[j] * step;
                }
            } else {
                let mass = e.mass.animate(p.mass, p.age / p.life);
                let placed = std::array::from_fn(|j| p.position[j] + offset[j]);
                let mut force = [0.0; 3];
                for applied in &self.applied {
                    applied.add(&mut force, placed, p.velocity, mass, dt, noise);
                }
                if let Some(space) = space {
                    space.integrate(&mut p.position, &mut p.velocity, force, mass, dt);
                } else {
                    // Preserve the comparison simulator's original evaluation order.
                    for (j, force) in force.into_iter().enumerate() {
                        p.velocity[j] += force * dt / mass;
                        p.position[j] += p.velocity[j] * dt;
                    }
                }
            }
            for j in 0..3 {
                p.rotation[j] += p.spin[j] * dt;
            }
        }
        if let Some(space) = space {
            self.previous_translation = Some(space.world.translation);
        }
        self.age += dt;
    }
    #[cfg(test)]
    fn advance(&mut self, e: &Emitter, time: f32, rate: f32) {
        self.advance_space(e, time, rate, None);
    }
    /// Finish emission while live particles continue.
    #[cfg(test)]
    fn advance_event(
        &mut self,
        e: &Emitter,
        age: f32,
        end: Option<f32>,
        space: Option<motion::Space>,
    ) {
        advance_event_system(
            &mut [Member {
                sim: self,
                emitter: e,
                space,
            }],
            age,
            end,
        );
    }

    #[cfg(test)]
    fn advance_space(&mut self, e: &Emitter, time: f32, rate: f32, space: Option<motion::Space>) {
        advance_system(
            &mut [Member {
                sim: self,
                emitter: e,
                space,
            }],
            time,
            rate,
        );
    }
}

/// [`advance_system`] for an entity event's system: emission finishes at `end` while live
/// particles continue.
fn advance_event_system(members: &mut [Member<'_>], age: f32, end: Option<f32>) {
    let Some(first) = members.first() else {
        return;
    };
    if let Some(end) = end {
        if first.sim.time < end {
            advance_system(members, end.min(age), 1.);
        }
        for member in members.iter_mut() {
            member.sim.emitting = age < end;
        }
    }
    advance_system(members, age, 1.);
}

/// Advances one particle system's subsystems, created together and on one clock, to `time`:
/// each frame step updates them in subsystem order, sharing the system's turbulence fields.
fn advance_system(members: &mut [Member<'_>], time: f32, rate: f32) {
    let Some(first) = members.first() else {
        return;
    };
    let target = time.max(0.0);
    let (prepared, delta) = (first.sim.prepared, target - first.sim.time);
    debug_assert!(members.iter().all(|m| m.sim.time == first.sim.time));
    let settle = |members: &mut [Member<'_>]| {
        for member in members.iter_mut() {
            member.sim.time = target;
            member.sim.prepared = false;
        }
    };
    let reset = !prepared && rate != 0.0 && delta < 0.0;
    if reset {
        for member in members.iter_mut() {
            *member.sim = Simulation::cold(
                member.emitter,
                member.sim.initial_seed,
                member.sim.anchor,
                time,
            );
        }
    }
    let leader = members.iter().position(|m| m.emitter.leads);
    let mut noise = leader
        .map(|at| std::mem::take(&mut members[at].sim.noise))
        .unwrap_or_default();
    // The system is created before the frame's particle updates. The entity fires a delayed
    // event on its own clock once `previous <= event < current`: a system whose event time the
    // clock has passed, culled or not, exists from this frame and takes the whole frame's update.
    start_members(members, target, &mut noise);
    if reset || prepared || rate == 0.0 {
        if let Some(at) = leader {
            members[at].sim.noise = noise;
        }
        return settle(members);
    }
    // Long suspensions have unknown native frame subdivisions. Preserve
    // the existing population, as a hidden native entity does, instead
    // of fabricating a full steady-state cloud or replaying many seconds.
    let mut remaining = if delta > 10.0 {
        0.0
    } else {
        // Subdivide unscaled frame delta in double precision, then multiply each float step by
        // entity playback.
        delta as f64 / rate as f64
    };
    let max_step = 0.1_f32.max((remaining.abs() * 0.25) as f32) as f64;
    while remaining > 0.0 {
        let dt = remaining.min(max_step);
        for member in members.iter_mut() {
            member
                .sim
                .tick_space(member.emitter, dt as f32 * rate, member.space, &mut noise);
        }
        remaining -= dt;
    }
    if let Some(at) = leader {
        members[at].sim.noise = noise;
    }
    settle(members);
}
fn emitter_basis(yaw: f32, pitch: f32) -> [[f32; 3]; 3] {
    let (a, ca) = (-pitch.to_radians()).sin_cos();
    let (b, cb) = (-(yaw + 90.0).to_radians()).sin_cos();
    [[cb * ca, -a, b * ca], [cb * a, ca, b * a], [-b, 0.0, cb]]
}
fn transform(basis: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|j| (0..3).map(|i| basis[i][j] * v[i]).sum())
}
type Simulations = BTreeMap<(usize, bool, usize), Simulation>;

/// One emitter the camera sees: its simulation key, seed and canonical anchor.
struct Visible<'a> {
    key: (usize, bool, usize),
    emitter: &'a Emitter,
    seed: u64,
    /// Authored map coordinates; the seam's camera supplies the world wrap.
    anchor: [f32; 3],
    alpha: f32,
    views: native::ViewMask,
}

type ParentView = ([f32; 3], viewer_foundation::map::province_admission::Planes);

fn parent_views(cameras: &[Camera], size: [u32; 2]) -> Vec<ParentView> {
    cameras
        .iter()
        .map(|&camera| (camera.eye(), side_planes(camera, size)))
        .collect()
}

///Dummy sphere distance/side planes, then the frame's accepted
///stamp. A later failed view keeps prior acceptance, without admitting earlier
///views. The supported depth-one tree's root bypasses ancestor box admission.
fn parent_admission(center: [f32; 3], radius: f32, views: &[ParentView]) -> Option<native::ViewMask> {
    // The mask is false until the first view that accepts and true from there, so the first
    // accepting view is the whole answer and an anchor no view accepts allocates nothing.
    let first = views.iter().position(|(eye, planes)| {
        let [dx, dy, dz] = std::array::from_fn(|axis| center[axis] - eye[axis]);
        (dy * dy + dx * dx) + dz * dz < f32::MAX
            && !planes
                .iter()
                .any(|[a, b, c, d]| center[0] * a + center[1] * b + center[2] * c + d > radius)
    })?;
    Some(native::ViewMask::Some(
        (0..views.len()).map(|view| view >= first).collect(),
    ))
}

/// Which weather pair each anchor's region drew, and the hour its weather began: the draw
/// advances a stream the region's anchors share, so it is made once for a field and a
/// presentation, and a camera that walks the anchors again reads the same answer instead of
/// drawing again. No camera is an input ([`Drawn::for_field`]).
#[derive(Default)]
struct Drawn {
    /// The field branch and hour, and the presentation's own change count, the draws were made
    /// for; `None` before the first walk.
    at: Option<(u64, i32, u64)>,
    /// Per anchor, in anchor order: the bank its pair came from, its place in it, and the hour
    /// its region's weather began.
    chosen: Vec<Option<((usize, bool), usize, i32)>>,
    /// The judge's own call count ([`crate::map::actor_check`]).
    calls: u64,
}

impl Drawn {
    /// The draws for this field and presentation, made again only where one of them moved.
    fn for_field(
        &mut self,
        anchors: &[Anchor],
        systems: &BTreeMap<(usize, bool), Vec<WeatherPair>>,
        field: &Field,
        presentation: &Presentation,
        changes: u64,
    ) -> &[Option<((usize, bool), usize, i32)>] {
        let at = (field.branch, field.hour, changes);
        let walk = |into: &mut Vec<Option<((usize, bool), usize, i32)>>| {
            let _span = viewer_foundation::profiling::span("weather_pairs");
            into.clear();
            let mut selectors = BTreeMap::new();
            for a in anchors {
                into.push(draw_pair(systems, presentation, a, &mut selectors));
            }
        };
        if self.at != Some(at) || self.chosen.len() != anchors.len() {
            viewer_foundation::view_record::add("weather packets", "pairs_drawn", 1);
            walk(&mut self.chosen);
            self.at = Some(at);
        } else if super::actor_check::due(&mut self.calls) {
            // The judge makes the draws again and compares: a kept answer that is not the one a
            // walk over every anchor would give names the anchor.
            let mut again = Vec::new();
            walk(&mut again);
            super::actor_check::checked("weather pairs", again.len() as u64);
            for (index, (kept, made)) in self.chosen.iter().zip(&again).enumerate() {
                if kept != made {
                    super::actor_check::differs("weather pairs", &index, &(kept, made));
                }
            }
        }
        &self.chosen
    }
}

/// One anchor's pair draw for its region's kind in `presentation`, exactly as a walk over every
/// anchor in order makes it.
fn draw_pair(
    systems: &BTreeMap<(usize, bool), Vec<WeatherPair>>,
    presentation: &Presentation,
    a: &Anchor,
    selectors: &mut BTreeMap<u32, u64>,
) -> Option<((usize, bool), usize, i32)> {
    let state = presentation.0.get(&a.region)?;
    let selector = selectors.entry(a.region).or_insert_with(|| {
        (state.seed_hour as u64).wrapping_mul(0x100000001b3) ^ ((a.region as u64) << 32)
    });
    let (bank, at) = select_pair_at(systems, state.kind, a.small, selector)?;
    Some((bank, at, state.seed_hour))
}

/// The emitters of `field` the camera sees, in drawing order, over the pairs already drawn, with
/// each region's member states (`members`).
fn visible_emitters<'a>(
    assets: &'a Assets,
    members: impl Fn(u32) -> [fade::Member; 2],
    views: &[ParentView],
    drawn: &[Option<((usize, bool), usize, i32)>],
) -> Vec<Visible<'a>> {
    let mut out = Vec::new();
    for (index, chosen) in drawn.iter().enumerate() {
        let Some(&(bank, at, seed_hour)) = chosen.as_ref() else {
            continue;
        };
        let a = &assets.anchors[index];
        let Some(pair) = assets.systems.get(&bank).and_then(|bank| bank.get(at)) else {
            continue;
        };
        let anchor = a.position;
        for (member, state) in members(a.region).into_iter().enumerate() {
            if !state.active {
                continue;
            }
            let (far_only, alpha) = (member == 1, state.alpha);
            let Some(radius) = pair.radii[member] else {
                continue;
            };
            let Some(mask) = parent_admission(anchor, radius, views) else {
                continue;
            };
            let emitters = &pair.members[member];
            for (ei, e) in emitters.iter().enumerate() {
                out.push(Visible {
                    key: (index, far_only, ei),
                    emitter: e,
                    seed: (seed_hour as u64).wrapping_mul(0x100000001b3)
                        ^ ((a.region as u64) << 32)
                        ^ (index as u64 * 257 + ei as u64),
                    anchor,
                    alpha,
                    views: mask.clone(),
                });
            }
        }
    }
    out
}

/// The same emitters for `indices` alone, off the frame's path, where no draws are kept, as the
/// producer run that creates their pairs at `camera` would leave them.
fn visible_emitters_at<'a>(
    assets: &'a Assets,
    presentation: &Presentation,
    camera: Camera,
    views: &[ParentView],
    indices: impl IntoIterator<Item = usize>,
) -> Vec<Visible<'a>> {
    let mut selectors = BTreeMap::new();
    let mut drawn = vec![None; assets.anchors.len()];
    for index in indices {
        let Some(a) = assets.anchors.get(index) else {
            continue;
        };
        drawn[index] = draw_pair(&assets.systems, presentation, a, &mut selectors);
    }
    let created = fade::created(camera.height, &assets.graphics);
    visible_emitters(assets, |_| created, views, &drawn)
}

/// The sound events of the weather entities the map shows: each admitted
/// anchor's active members, as [`visible_emitters`] admits them. A region
/// keeps its keys while it keeps its weather.
struct SoundInputs<'a> {
    anchors: &'a [Anchor],
    systems: &'a BTreeMap<(usize, bool), Vec<WeatherPair>>,
    /// Each region's member states ([`fade`]).
    fades: &'a fade::Fades,
    presentation: &'a Presentation,
    /// The pairs the anchors drew for this field and presentation ([`Drawn`]).
    drawn: &'a [Option<((usize, bool), usize, i32)>],
    /// When each shown member entity first appeared on the presentation
    /// clock, by anchor, member, region and the hour its weather began.
    starts: &'a mut SoundStarts,
    /// The presentation clock.
    time: f32,
}

/// See [`SoundInputs::starts`].
type SoundStarts = BTreeMap<(usize, usize, u32, i32), f32>;

fn sound_events(
    inputs: SoundInputs<'_>,
    views: &[ParentView],
    names: &mut super::sound_events::Names,
    out: &mut Vec<super::sound_events::SoundEvent>,
) {
    let SoundInputs {
        anchors,
        systems,
        fades,
        presentation,
        drawn,
        starts,
        time,
    } = inputs;
    for (index, chosen) in drawn.iter().enumerate() {
        let (Some(a), Some(&(bank, at, seed_hour))) = (anchors.get(index), chosen.as_ref()) else {
            continue;
        };
        let Some(pair) = systems.get(&bank).and_then(|bank| bank.get(at)) else {
            continue;
        };
        for (member, state) in fades.members(a.region).into_iter().enumerate() {
            let Some(radius) = pair.radii[member] else {
                continue;
            };
            if !state.active
                || pair.sounds[member].is_empty()
                || parent_admission(a.position, radius, views).is_none()
            {
                continue;
            }
            // The entity enters its state when it first appears; each sound
            // event fires at its `time` after that.
            let start = starts
                .entry((index, member, a.region, seed_hour))
                .or_insert(time);
            *start = start.min(time);
            for (event, sound) in pair.sounds[member].iter().enumerate() {
                let age = time - *start - sound.time;
                if age < 0.0 {
                    continue;
                }
                out.push(super::sound_events::weather(
                    (index, member, event, a.region, seed_hour),
                    names,
                    &sound.effect,
                    a.position,
                    age,
                ));
            }
        }
    }
    // An entity lasts while its region keeps its weather.
    starts.retain(|&(_, _, region, seed_hour), _| {
        presentation
            .0
            .get(&region)
            .is_some_and(|state| state.seed_hour == seed_hour)
    });
}

/// Camera admission operands; presentation alone knows which region runtimes survive.
#[derive(Clone)]
pub struct Prepared {
    field: Field,
    camera: Camera,
    size: [u32; 2],
}

/// Replaced region runtimes, freed off the frame path.
pub struct Retired(
    #[expect(
        dead_code,
        reason = "held only so the window's dropper frees the runtimes off the frame path"
    )]
    Vec<Simulation>,
);

/// Merges an hour's `prepared` runtimes into `current`, keyed by emitter
/// anchor: a runtime whose anchor's region `kept` stays as it is and the
/// prepared one for it is let go; every other current runtime is let go and the
/// prepared one takes its place. Returns everything let go of.
fn merge_runtimes<K: Ord + Copy, T>(
    current: &mut BTreeMap<K, T>,
    prepared: BTreeMap<K, T>,
    kept: impl Fn(&K) -> bool,
) -> Vec<T> {
    let mut retired = Vec::with_capacity(prepared.len());
    retired.extend(
        current
            .extract_if(.., |key, _| !kept(key))
            .map(|(_, runtime)| runtime),
    );
    for (key, runtime) in prepared {
        if kept(&key) {
            retired.push(runtime);
        } else {
            retired.extend(current.insert(key, runtime));
        }
    }
    retired
}

/// The producer runs' kind checks over the `listed` regions, as the game makes them only when a
/// region's producer runs: a region whose `wanted` kind is not the `held` one lets its pairs go
/// (member states and `runtimes`) and takes the wanted kind, with the `pending` runtimes prepared
/// for it; wanting none, it holds nothing and lets those go too. Returns whether any region
/// swapped and what it let go.
fn swap_listed<T>(
    listed: &[u32],
    wanted: &Presentation,
    held: &mut Presentation,
    fades: &mut fade::Fades,
    (runtimes, pending): (&mut BTreeMap<(usize, bool, usize), T>, &mut BTreeMap<(usize, bool, usize), T>),
    region_of: impl Fn(usize) -> u32,
) -> (bool, Vec<T>) {
    let kind = |presentation: &Presentation, region: &u32| presentation.0.get(region).map(|state| state.kind);
    let swapped: BTreeSet<u32> = listed
        .iter()
        .copied()
        .filter(|region| kind(wanted, region) != kind(held, region))
        .collect();
    if swapped.is_empty() {
        return (false, Vec::new());
    }
    for &region in &swapped {
        fades.remove(region);
        match wanted.0.get(&region) {
            Some(&state) => held.0.insert(region, state),
            None => held.0.remove(&region),
        };
    }
    let swaps = |&(anchor, _, _): &(usize, bool, usize)| swapped.contains(&region_of(anchor));
    let (adopted, unwanted): (BTreeMap<_, _>, BTreeMap<_, _>) = pending
        .extract_if(.., |key, _| swaps(key))
        .partition(|&(key, _)| held.0.contains_key(&region_of(key.0)));
    let mut let_go = merge_runtimes(runtimes, adopted, |key| !swaps(key));
    let_go.extend(unwanted.into_values());
    (true, let_go)
}

/// A kept region also keeps absent emitters; later camera admission creates them.
pub fn prepare(_assets: &Assets, field: Field, camera: Camera, size: [u32; 2]) -> Prepared {
    Prepared {
        field,
        camera,
        size,
    }
}

/// Empty runtimes for the pairs each region's producer will create when it next runs: those of
/// the regions whose `wanted` kind is not the `held` one.
fn prepare_changed(
    assets: &Assets,
    wanted: &Presentation,
    held: &Presentation,
    camera: Camera,
    size: [u32; 2],
) -> Simulations {
    let indices: BTreeSet<_> = wanted
        .0
        .iter()
        .filter(|(region, state)| held.0.get(region).map(|held| held.kind) != Some(state.kind))
        .flat_map(|(region, _)| {
            assets
                .anchors_by_region
                .get(region)
                .into_iter()
                .flatten()
                .copied()
        })
        .collect();
    let views = parent_views(&province_admission_views(camera, size), size);
    visible_emitters_at(assets, wanted, camera, &views, indices)
        .into_iter()
        .map(|visible| {
            let mut simulation =
                Simulation::cold(visible.emitter, visible.seed, visible.anchor, 0.0);
            simulation.prepared = true;
            (visible.key, simulation)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn weather_packets(
    assets: &Assets,
    drawn: &[Option<((usize, bool), usize, i32)>],
    frame: (Camera, &[ParentView], f32),
    fades: &fade::Fades,
    simulations: &mut Simulations,
    packets: &mut Vec<native::DrawPacket>,
) {
    let (camera, views, time) = frame;
    let rate = gates(camera.height, &assets.graphics).2;
    let visible_span = viewer_foundation::profiling::span("weather_visible");
    let seen = visible_emitters(assets, |region| fades.members(region), views, drawn);
    drop(visible_span);
    viewer_foundation::view_record::add("weather packets", "anchors", assets.anchors.len() as u64);
    viewer_foundation::view_record::add("weather packets", "emitters_admitted", seen.len() as u64);
    // A newly active member starts empty; its first activation accumulates births
    // over later frame updates rather than inventing a steady-state cloud.
    for visible in &seen {
        simulations.entry(visible.key).or_insert_with(|| {
            Simulation::cold(visible.emitter, visible.seed, visible.anchor, time)
        });
    }
    // Keys are emitted in anchor/member/emitter order, the numeric map's order too.
    // Merge the two ordered walks without another keyed allocation or sort.
    debug_assert!(seen.windows(2).all(|pair| pair[0].key < pair[1].key));
    let advance_span = viewer_foundation::profiling::span("weather_advance");
    let eye = sort::Eye::of(camera);
    let mut seen = seen.into_iter().peekable();
    let tasks: Vec<_> = simulations
        .iter_mut()
        .filter_map(|(key, simulation)| {
            if let Some(visible) = seen.next_if(|visible| visible.key == *key) {
                Some((simulation, visible))
            } else {
                // Unadmitted populations freeze; rebase so reentry invents no hidden births.
                simulation.time = time.max(0.0);
                // Only an enabled runtime sorts: a culled member does, a deactivated one not.
                if fades.members(assets.anchors[key.0].region)[usize::from(key.1)].active {
                    simulation.sort(eye);
                }
                None
            }
        })
        .collect();
    // One weather particle reference's subsystems are one particle system: they update
    // together, sharing its turbulence fields.
    let mut tasks = tasks;
    for system in tasks.chunk_by_mut(|(_, a), (_, b)| {
        (a.key.0, a.key.1, a.emitter.actor) == (b.key.0, b.key.1, b.emitter.actor)
    }) {
        let mut members: Vec<Member<'_>> = system
            .iter_mut()
            .map(|(sim, visible)| {
                let e = visible.emitter;
                // A delayed event is timed from the entity's creation by its producer,
                // however long the pair stayed out of view.
                if sim.waiting()
                    && let Some(born) = fades.born(assets.anchors[visible.key.0].region)
                {
                    sim.event_at = Some(born + e.delay);
                }
                let space = motion::Space::new(
                    e.local_space,
                    e.system_local_space,
                    e.scale,
                    installed_content_core::mesh::Transform {
                        translation: sim.anchor,
                        ..installed_content_core::mesh::Transform::IDENTITY
                    },
                );
                Member {
                    sim: &mut **sim,
                    emitter: e,
                    space: Some(space),
                }
            })
            .collect();
        advance_system(&mut members, time, rate);
    }
    // The producer's anchor/member/emitter order, empty packets included: an enabled actor
    // binds scale/quad IA before inspecting its particle count. A system whose entity event
    // has not fired yet is no actor.
    for (sim, mut visible) in tasks {
        if sim.waiting() {
            continue;
        }
        let e = visible.emitter;
        sim.sort(eye);
        let mut packet = native::DrawPacket::weather(e, sim, visible.anchor, visible.alpha);
        packet.actor = [0, visible.key.0, usize::from(visible.key.1), e.actor];
        packet.views = std::mem::replace(&mut visible.views, native::ViewMask::All);
        packets.push(packet);
    }
    drop(advance_span);
    if viewer_foundation::view_record::on() {
        use viewer_foundation::view_record::add;
        add("weather packets", "simulations", simulations.len() as u64);
        add(
            "weather packets",
            "weather_instances",
            packets.iter().map(|packet| packet.instances.len() as u64).sum(),
        );
        add(
            "weather packets",
            "weather_packets_empty",
            packets.iter().filter(|packet| packet.instances.is_empty()).count() as u64,
        );
    }
}

#[derive(Default)]
struct Clock {
    previous: Option<f32>,
    elapsed: f32,
}
impl Clock {
    fn advance(&mut self, time: f32, height: f32, defines: &CameraDefines) -> f32 {
        if time <= 0.0 {
            self.elapsed = 0.0;
            self.previous = None;
            return 0.0;
        }
        if let Some(previous) = self.previous {
            self.elapsed += (time - previous).max(0.0) * gates(height, defines).2;
        }
        self.previous = Some(time);
        self.elapsed
    }
}

/// Immutable canonical packets prepared once for every seam of this frame.
pub struct Packets(Vec<native::DrawPacket>);

impl Packets {
    /// How many draw packets one frame prepared.
    pub fn count(&self) -> usize {
        self.0.len()
    }
}

/// One shared pass for window and dump. Static textures stay resident; only
/// particles at the selected hour and visible camera are generated each frame.
pub struct Pass {
    #[cfg(any(test, feature = "test-support"))]
    pub offline_packets: BTreeMap<String, usize>,
    native: native::Renderer,
    assets: Arc<Assets>,
    field: Field,
    /// The kinds `field` asks for, each with the hour it began.
    wanted: Presentation,
    /// The kinds whose pairs the regions' producers hold: a region takes its wanted kind only
    /// when its producer runs ([`Self::produce`]).
    presentation: Presentation,
    /// Each region's member states, set by its producer runs.
    fades: fade::Fades,
    clock: Clock,
    simulations: Simulations,
    /// Empty runtimes made off the frame path for the pairs producers have yet to create
    /// ([`prepare_changed`]).
    pending: Simulations,
    /// Runtimes producers let go of, for the next field change to hand off the frame path.
    retired: Vec<Simulation>,
    entity_simulations: native::EntitySimulations,
    /// See [`SoundInputs::starts`].
    sound_starts: SoundStarts,
    events: Vec<super::entity_animation::Event>,
    /// The anchors' pair draws, kept for the field and presentation that made them.
    drawn: Drawn,
    /// How many times the field or the presentation has moved, which is what makes the draws
    /// again ([`Drawn::for_field`]).
    changes: u64,
}
impl Pass {
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        assets: Arc<Assets>,
    ) -> io::Result<Self> {
        let mut native_pages = Vec::new();
        for page in &assets.pages {
            let bound = textures::upload(device, queue, "weather", page, ColorSpace::Srgb)?;
            native_pages.push(bound.view);
        }
        Ok(Self {
            native: native::Renderer::new(
                device,
                assets.effect.clone(),
                native_pages,
                samples,
                assets.max_particles,
            ),
            assets,
            field: Field::default(),
            wanted: Presentation::default(),
            presentation: Presentation::default(),
            fades: fade::Fades::default(),
            clock: Clock::default(),
            simulations: Simulations::new(),
            pending: Simulations::new(),
            retired: Vec::new(),
            entity_simulations: native::EntitySimulations::default(),
            sound_starts: SoundStarts::new(),
            events: Vec::new(),
            drawn: Drawn::default(),
            changes: 0,
            #[cfg(any(test, feature = "test-support"))]
            offline_packets: Default::default(),
        })
    }
    pub fn set_events(
        &mut self,
        events: Vec<super::entity_animation::Event>,
    ) -> io::Result<Vec<super::pointlights::Light>> {
        let (events, lights) = advance_entity_effects(
            &mut self.entity_simulations,
            &self.assets.entities,
            &self.assets.lights,
            events,
        )?;
        static PROFILE: OnceLock<bool> = OnceLock::new();
        if *PROFILE.get_or_init(|| std::env::var_os("BATTLE_PLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some()) {
            eprintln!(
                "Entity effects: events={} lights={} particles={}",
                events.len(),
                lights.len(),
                events
                    .iter()
                    .filter(|e| e.definition.particle.is_some())
                    .count()
            );
        }
        self.events = events;
        Ok(lights)
    }
    /// Moves to `field`: the regions' producers take its kinds when they next run, and runtimes
    /// for the pairs they will create are prepared at `preparation`'s camera. A discontinuity lets
    /// every held pair go at once, as a load does. Returns the runtimes it let go of.
    fn adopt_field(
        &mut self,
        field: Field,
        preparation: Option<(Camera, [u32; 2])>,
        continuity: Continuity,
    ) -> Vec<Simulation> {
        let _cpu = replay_frame::cpu::scope("viewer", Some("weather"));
        self.changes += 1;
        let mut retired = std::mem::take(&mut self.retired);
        let continuous = continuity.keeps(
            (self.field.branch, self.field.hour),
            (field.branch, field.hour),
        );
        if continuous && self.field.kinds == field.kinds {
            self.field = field;
            return retired;
        }
        if continuity == Continuity::Reset {
            self.entity_simulations.clear();
        }
        self.wanted.transition(&self.field, &field, continuity);
        if !continuous {
            self.presentation.0.clear();
            self.fades.clear();
            retired.extend(std::mem::take(&mut self.simulations).into_values());
        }
        retired.extend(std::mem::take(&mut self.pending).into_values());
        if let Some((camera, size)) = preparation {
            self.pending =
                prepare_changed(&self.assets, &self.wanted, &self.presentation, camera, size);
        }
        self.field = field;
        retired
    }

    pub fn set(&mut self, field: Field) {
        self.adopt_field(field, None, Continuity::Reset);
    }

    /// Moves to `prepared`'s field ([`Self::adopt_field`]), returning what it let go of for the
    /// caller to free off the frame path.
    pub fn set_prepared(&mut self, prepared: Prepared, continuity: Continuity) -> Retired {
        Retired(self.adopt_field(
            prepared.field,
            Some((prepared.camera, prepared.size)),
            continuity,
        ))
    }
    /// This frame's producer runs at camera height `height`: one for each region with a province
    /// in the frame's update list on `schedule`. Each takes its wanted kind if it holds another
    /// ([`swap_listed`]), then sets its members' fade ([`fade`]). Called once a frame, after the
    /// walk that took the list and before the frame's sounds and packets read the states.
    ///
    /// As in the game, each run rereads the map's weather switch (`weather`): off, the region
    /// wants no weather, so a listed region lets its pairs go and an unlisted one keeps them.
    pub fn produce(&mut self, height: f32, schedule: &Schedule, weather: bool) {
        let none = Presentation::default();
        let Self {
            assets,
            wanted,
            presentation,
            fades,
            simulations,
            pending,
            retired,
            changes,
            ..
        } = self;
        let regions: BTreeSet<u32> = wanted.0.keys().chain(presentation.0.keys()).copied().collect();
        let listed: Vec<u32> = regions
            .into_iter()
            .filter(|&region| assets.region_provinces.listed(region, schedule))
            .collect();
        let anchors = &assets.anchors;
        let (swapped, let_go) = swap_listed(
            &listed,
            if weather { &*wanted } else { &none },
            presentation,
            fades,
            (simulations, pending),
            |anchor| anchors[anchor].region,
        );
        retired.extend(let_go);
        *changes += u64::from(swapped);
        fades.produce(
            listed.iter().copied().filter(|region| presentation.0.contains_key(region)),
            height,
            &assets.graphics,
            |_| true,
        );
    }

    /// Appends the shown weather entities' sound events at presentation
    /// time `time` (see [`sound_events`]).
    pub fn sound_events(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        time: f32,
        names: &mut super::sound_events::Names,
        out: &mut Vec<super::sound_events::SoundEvent>,
    ) {
        let views = parent_views(&[camera], size);
        let Self {
            assets,
            field,
            presentation,
            fades,
            drawn,
            changes,
            sound_starts,
            ..
        } = self;
        sound_events(
            SoundInputs {
                anchors: &assets.anchors,
                systems: &assets.systems,
                fades,
                presentation,
                drawn: drawn.for_field(
                    &assets.anchors,
                    &assets.systems,
                    field,
                    presentation,
                    *changes,
                ),
                starts: sound_starts,
                time,
            },
            &views,
            names,
            out,
        );
    }

    /// Advance presentation and births once, before submitting any seam view.
    pub fn prepare_frame(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        cameras: &[Camera],
        animation_time: f32,
    ) -> Packets {
        let _profile = viewer_foundation::profiling::thread_span("weather_packets");
        let mut packets = Vec::new();
        // Pairs a producer still holds draw whatever the weather switch says ([`Self::produce`]).
        let time = self
            .clock
            .advance(animation_time, camera.height, &self.assets.graphics);
        let views = parent_views(cameras, size);
        let Self {
            assets,
            field,
            presentation,
            drawn,
            changes,
            fades,
            simulations,
            ..
        } = self;
        fades.stamp(time);
        weather_packets(
            assets,
            drawn.for_field(&assets.anchors, &assets.systems, field, presentation, *changes),
            (camera, &views, time),
            fades,
            simulations,
            &mut packets,
        );
        let entity_span = viewer_foundation::profiling::span("weather_entity_packets");
        let before = packets.len();
        self.entity_simulations.sort(sort::Eye::of(camera));
        self.entity_simulations.packets(&self.assets.entities, &self.events, &mut packets);
        drop(entity_span);
        if viewer_foundation::view_record::on() {
            use viewer_foundation::view_record::add;
            add("weather packets", "entity_packets", (packets.len() - before) as u64);
            add(
                "weather packets",
                "entity_instances",
                packets[before..].iter().map(|packet| packet.instances.len() as u64).sum(),
            );
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.offline_packets.clear();
            for packet in packets.iter().filter(|packet| !packet.instances.is_empty()) {
                *self
                    .offline_packets
                    .entry(packet.descriptor.technique())
                    .or_default() += packet.instances.len();
            }
        }
        let _order_span = viewer_foundation::profiling::span("weather_order");
        native::order(&mut packets);
        Packets(packets)
    }

    /// Build each content-selected particle pipeline before drawing a frame.
    pub fn warm(&mut self, device: &wgpu::Device) {
        let assets = Arc::clone(&self.assets);
        let emitters: Vec<&Emitter> = assets
            .systems
            .values()
            .flatten()
            .flat_map(|pair| pair.members.iter().flatten())
            .chain(assets.entities.values().flat_map(|(_, emitters)| emitters))
            .collect();
        self.native.warm(device, &emitters);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_view(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        camera: Camera,
        view: usize,
        native_inputs: native::Inputs<'_>,
        packets: &Packets,
    ) -> io::Result<()> {
        self.native
            .draw(
                arena,
                encoder,
                targets,
                camera.projection(targets.size()),
                camera.eye(),
                view,
                native_inputs,
                &packets.0,
            )
            .map_err(|error| {
                if error.to_string().starts_with("Map shader particles:") {
                    error
                } else {
                    invalid(format!("Map shader particles: 1 refused pass: {error}"))
                }
            })
    }
}

/// Advances every entity-event particle runtime on `events`, hidden ones included, so hiding never
/// changes a runtime's population, emission accumulator or random stream; then returns the events
/// with the lights the shown ones give. A kept event's light lasts until its finished runtime has
/// no living particle; then the event is finished for its player to drop.
pub(crate) fn advance_entity_effects(
    simulations: &mut EntitySimulations,
    entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
    lights: &super::entity_effects::Lights,
    mut events: Vec<super::entity_animation::Event>,
) -> io::Result<(
    Vec<super::entity_animation::Event>,
    Vec<super::pointlights::Light>,
)> {
    let prepare_span = viewer_foundation::profiling::span("entity_prepare");
    simulations.prepare(entities, &events)?;
    drop(prepare_span);
    let _lights_span = viewer_foundation::profiling::span("entity_lights");
    for event in &mut events {
        if event.emission_end.is_some() && !simulations.living(event) {
            event.definition.light = None;
            if let Some(kept) = &event.kept {
                kept.finish();
            }
        }
    }
    let lights = lights.evaluate(&events);
    Ok((events, lights))
}

#[cfg(test)]
pub(crate) mod tests;
