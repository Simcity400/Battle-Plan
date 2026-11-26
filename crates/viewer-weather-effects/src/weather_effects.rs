use std::collections::BTreeSet;
use std::sync::OnceLock;
use viewer_runtime::presentation::Continuity;
use viewer_gpu::province_admission::side_planes;
use viewer_gpu::province_admission::views as province_admission_views;
use viewer_gpu::province_schedule::Schedule;
use viewer_gpu::textures::load_file;
use viewer_gpu::{camera::Camera, gpu_contract::Targets, textures::{self, ColorSpace}};
mod fade;
mod motion;
mod native;
mod sort;
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_parser::parser::{self, Item, Value};
use content_reader_mesh::particles::Catalog;
use content_reader_image::{dds, native::Decode};
pub use fade::RegionProvinces;
pub use native::EntitySimulations;
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
                .next_back()
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
    #[cfg(test)]
    fn sample(&self, random: f32, age: f32) -> f32 {
        self.animate(self.base + (random * 2.0 - 1.0) * self.spread, age)
    }
    fn sample_spawn(&self, random: f32, system_age: f32) -> f32 {
        let base = match &self.curve {
            Some(curve) if curve.time == TimeMode::Spawn => curve.apply(self.base, system_age),
            _ => self.base,
        };
        base + (random * 2.0 - 1.0) * self.spread
    }
    fn has_live_curve(&self) -> bool {
        self.curve
            .as_ref()
            .is_some_and(|curve| curve.time != TimeMode::Spawn)
    }
    #[cfg(test)]
    fn animate(&self, value: f32, age: f32) -> f32 {
        self.curve.as_ref().map_or(value, |c| c.apply(value, age))
    }
    fn animate_particle(&self, value: f32, age: f32, life: f32, system_age: f32) -> f32 {
        let Some(curve) = &self.curve else {
            return value;
        };
        let time = match curve.time {
            TimeMode::Spawn => return value,
            TimeMode::Life => age / life,
            TimeMode::LifeAbs => age,
            TimeMode::System => system_age,
        };
        curve.apply(value, time)
    }
    fn at(&self, seed: &mut u64, time: f32) -> f32 {
        let base = match &self.curve {
            Some(curve) if matches!(curve.time, TimeMode::Spawn | TimeMode::System) => {
                curve.apply(self.base, time)
            }
            _ => self.base,
        };
        if self.spread != 0.0 {
            random(seed) * (self.spread + self.spread) + (base - self.spread)
        } else {
            base
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimeMode {
    Spawn,
    System,
    Life,
    LifeAbs,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Op {
    Add,
    Mul,
    Abs,
}

#[derive(Debug, Clone)]
pub(super) struct Curve {
    samples: [f32; 64],
    start: f32,
    duration: f32,
    pub(super) op: Op,
    pub(super) time: TimeMode,
    pub(super) repeat: bool,
}
impl Curve {
    pub(super) fn apply(&self, value: f32, time: f32) -> f32 {
        let end = self.start + self.duration;
        let time = if self.repeat && end != 0.0 {
            time % end
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

    fn curve_number(items: &[Item], name: &str, key: &str, default: f32) -> io::Result<f32> {
        let Some(value) = get(items, key) else {
            return Ok(default);
        };
        let token = match value {
            Value::Scalar(_) => value.scalar_str(),
            Value::Block(entries) if entries.len() == 1 && entries[0].key.is_none() => {
                entries[0].value.scalar_str()
            }
            _ => None,
        }
        .ok_or_else(|| invalid(format!("animation curve {name:?} has malformed {key} value")))?;
        let parsed = token.parse::<f32>().map_err(|_| {
            invalid(format!(
                "animation curve {name:?} has malformed {key} value {token:?}"
            ))
        })?;
        if !parsed.is_finite() {
            return Err(invalid(format!(
                "animation curve {name:?} has non-finite {key} value {token:?}"
            )));
        }
        Ok(parsed)
    }

    pub(super) fn parse(items: &[Item]) -> io::Result<Self> {
        let name = text(items, "name");
        let mut values = Vec::new();
        if let Some(raw_curve) = get(items, "curve") {
            let entries = match raw_curve {
                Value::Block(entries) => entries.as_slice(),
                _ => {
                    return Err(invalid(format!(
                        "animation curve {name:?} has malformed curve value"
                    )));
                }
            };
            values.reserve(entries.len());
            for entry in entries {
                if entry.key.is_some() {
                    return Err(invalid(format!(
                        "animation curve {name:?} has malformed curve entry"
                    )));
                }
                let token = entry.value.scalar_str().ok_or_else(|| {
                    invalid(format!(
                        "animation curve {name:?} has nested non-scalar item in curve"
                    ))
                })?;
                let parsed = token.parse::<f32>().map_err(|_| {
                    invalid(format!(
                        "animation curve {name:?} has malformed curve value {token:?}"
                    ))
                })?;
                if !parsed.is_finite() {
                    return Err(invalid(format!(
                        "animation curve {name:?} has non-finite curve value {token:?}"
                    )));
                }
                values.push(parsed);
            }
        }
        if values.len() % 2 != 0 {
            return Err(invalid(format!(
                "animation curve {name:?} has an odd number of curve values ({})",
                values.len()
            )));
        }
        let knots: Vec<[f32; 2]> = values.chunks_exact(2).map(|p| [p[0], p[1]]).collect();
        for pair in knots.windows(2) {
            if !(pair[1][0] > pair[0][0]) {
                return Err(invalid(format!(
                    "animation curve {name:?} has a non-increasing knot domain"
                )));
            }
        }
        let op = match get(items, "op") {
            None => Op::Mul,
            Some(value) => match value.scalar_str() {
                Some(raw) => match raw.to_ascii_lowercase().as_str() {
                    "" | "mul" => Op::Mul,
                    "add" => Op::Add,
                    "abs" => Op::Abs,
                    other => {
                        return Err(invalid(format!(
                            "animation curve {name:?} has unsupported op {other:?}"
                        )));
                    }
                },
                None => {
                    return Err(invalid(format!(
                        "animation curve {name:?} has malformed op value"
                    )));
                }
            },
        };
        let time = match get(items, "time") {
            None => TimeMode::Spawn,
            Some(value) => match value.scalar_str() {
                Some("life_abs") => TimeMode::LifeAbs,
                Some("life") => TimeMode::Life,
                Some("system") => TimeMode::System,
                Some(_) => TimeMode::Spawn,
                None => {
                    return Err(invalid(format!(
                        "animation curve {name:?} has malformed time value"
                    )));
                }
            },
        };
        let start = Self::curve_number(items, name, "start", 0.0)?;
        let duration = Self::curve_number(items, name, "duration", 1.0)?;
        let min = Self::curve_number(items, name, "minValue", 0.0)?;
        let max = Self::curve_number(items, name, "maxValue", 1.0)?;
        let mut curve = Self::new(&knots, start, duration, min, max);
        curve.op = op;
        curve.time = time;
        curve.repeat = parser::read_bool_field(items, "repeat", false);
        Ok(curve)
    }

    pub(super) fn new(knots: &[[f32; 2]], start: f32, duration: f32, min: f32, max: f32) -> Self {
        let duration = if duration.is_finite() && duration > 0.0 {
            duration
        } else {
            1.0
        };
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
            time: TimeMode::Spawn,
            repeat: false,
        }
    }
    pub(super) fn sample(&self, age: f32) -> f32 {
        let x = (age - self.start) * 64.0 / self.duration;
        if x <= 0.0 {
            self.samples[0]
        } else if x >= 63.0 {
            self.samples[63]
        } else {
            let i = x as usize;
            self.samples[i] + (self.samples[i + 1] - self.samples[i]) * (x - i as f32)
        }
    }
}
#[derive(Debug, Clone)]
pub struct Emitter {
    scale: f32,
    page: usize,
    registration: usize,
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
    forces: Vec<Force>,
    noise_divisions: Vec<usize>,
    leads: bool,
    delay: f32,
    local_space: bool,
    render: content_reader_mesh::particles::RenderDescriptor,
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

#[derive(Debug, Clone)]
struct Force {
    kind: ForceKind,
    amount: Parameter,
    position: [f32; 3],
    direction: [f32; 3],
    yaw: [f32; 2],
    local: bool,
}

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
        step: f32,
    },
}

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
#[derive(Clone, Copy)]
pub enum Pick {
    Last,
    First,
}

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

struct SystemForce<'a> {
    name: &'a str,
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

fn parse_emitters(root: &[Item]) -> io::Result<Vec<Emitter>> {
    let particle = text(root, "name");
    let mut curves = BTreeMap::new();
    for a in root
        .iter()
        .filter(|i| i.key.as_deref() == Some(b"animation"))
    {
        let a = a.value.items().unwrap_or(&[]);
        curves.insert(text(a, "name").to_owned(), Curve::parse(a)?);
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
        let render = content_reader_mesh::particles::RenderDescriptor::read(s)?;
        let param = |k, d| Parameter::read(s, k, d, &curves);
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
            max: content_reader_mesh::particles::subsystem_capacity(s)?,
            forces,
            noise_divisions: turbulence.clone(),
            leads: emitters.is_empty(),
            delay: 0.0,
            local_space: render.local_space,
            render,
            system_local_space: root
                .iter()
                .filter(|i| i.key.as_deref() == Some(b"subsystem"))
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

fn system_capacity(emitters: &[Emitter]) -> usize {
    emitters.iter().map(|e| e.max).sum()
}
type LocalEmitters = (Vec<Emitter>, Vec<(String, dds::Texture)>);
struct WeatherPair {
    members: [Vec<Emitter>; 2],
    radii: [Option<f32>; 2],
    sounds: [Vec<content_reader_weather::weather::WeatherSound>; 2],
}
type LocalSystem = (
    (usize, bool),
    WeatherPair,
    Vec<(String, dds::Texture)>,
    Option<String>,
);

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
    pub fn load(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
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
    pub withheld: Vec<String>,
    lights: super::entity_effects::Lights,
}

fn read_object(
    layers: &ResolvedLayers,
    decode: Decode,
    document: &parser::Document,
    path: &str,
    particle: &content_reader_mesh::particles::Particle,
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

fn read_pair(
    layers: &ResolvedLayers,
    decode: Decode,
    pair: &content_reader_weather::weather::EffectPair,
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
        defines: &content_reader_defines::defines::Defines,
        map: [u32; 2],
        region_provinces: RegionProvinces,
        decode: Decode,
    ) -> io::Result<Arc<Self>> {
        let catalog = Catalog::read(layers)?;
        let mut withheld = catalog.refused().to_vec();
        let lights = super::entity_effects::Lights::load(layers, &mut withheld)?;
        let mut out = Self {
            effect: viewer_scene_base::particle_shader::load(layers, map, decode)?,
            max_particles: 0,
            graphics: CameraDefines::load(defines)?,
            anchors: Vec::new(),
            anchors_by_region: BTreeMap::new(),
            region_provinces,
            systems: BTreeMap::new(),
            pages: Vec::new(),
            entities: BTreeMap::new(),
            withheld,
            lights,
        };
        const POSITIONS: &str = "map/weatherpositions.txt";
        let (positions, skipped) = match layers.read_uncovered(POSITIONS) {
            Ok(bytes) => content_reader_map::positions::parse_weather_positions(&bytes),
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
        let descriptors = content_reader_weather::weather::effects(layers, &catalog)?;
        let keys: Vec<_> = descriptors
            .iter()
            .flat_map(|(&key, pairs)| pairs.iter().map(move |pair| (key, pair)))
            .collect();
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
        let mut by_file: BTreeMap<
            content_reader_layers::script_files::ScriptPath,
            Vec<(String, content_reader_mesh::particles::Particle)>,
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

use map_display_data::WeatherField as Field;

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

fn sample_tile(seed: &mut u64, dimensions: [i32; 2]) -> [u8; 4] {
    [
        (random_bits(seed) as i32 % dimensions[0]) as u8,
        (random_bits(seed) as i32 % dimensions[1]) as u8,
        0,
        0,
    ]
}

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
    rotation_base: [f32; 3],
    spin: [f32; 3],
    color: [f32; 4],
    tile: [u8; 4],
}
#[derive(Clone)]
struct Simulation {
    prepared: bool,
    emitting: bool,
    anchor: [f32; 3],
    initial_seed: u64,
    seed: u64,
    noise: Vec<Noise>,
    particles: Vec<LiveParticle>,
    accumulator: f32,
    age: f32,
    update_age: f32,
    time: f32,
    previous_translation: Option<[f32; 3]>,
    order: sort::Order,
    origin: [f32; 3],
    duration: f32,
    applied: Vec<Applied>,
    event_at: Option<f32>,
    fresh: bool,
}

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

struct Member<'a> {
    sim: &'a mut Simulation,
    emitter: &'a Emitter,
    space: Option<motion::Space>,
}

impl Simulation {
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
            update_age: 0.0,
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

    fn sample(&mut self, e: &Emitter, stream: &mut u64) {
        self.origin = e.position.each_ref().map(|p| p.at(stream, 0.0));
        self.duration = e.duration.at(stream, 0.0);
        self.age = -e.start.at(stream, 0.0);
        self.update_age = self.age;
    }

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
            self.previous_translation = Some(space.world.translation);
        }
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
            self.accumulator += e.emission.at(&mut self.seed, self.age).max(0.0) * dt;
        }
        let births = if emitting {
            (self.accumulator as usize).min(e.max - self.particles.len())
        } else {
            0
        };
        self.accumulator -= births as f32;
        let basis = if births > 0 {
            emitter_basis(
                e.emitter_yaw.sample_spawn(random(&mut self.seed), self.age),
                e.emitter_pitch.sample_spawn(random(&mut self.seed), self.age),
            )
        } else {
            emitter_basis(0.0, 0.0)
        };
        for ordinal in 0..births {
            let life = e
                .life
                .sample_spawn(random(&mut self.seed), self.age)
                .max(0.01);
            let mass = e.mass.sample_spawn(random(&mut self.seed), self.age);
            let mut sample = |p: &Parameter| p.sample_spawn(random(&mut self.seed), self.age);
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
            let size = e.size.sample_spawn(random(&mut self.seed), self.age);
            let rotation = e
                .rotation
                .each_ref()
                .map(|p| p.sample_spawn(random(&mut self.seed), self.age));
            let spin = e
                .spin
                .each_ref()
                .map(|p| p.sample_spawn(random(&mut self.seed), self.age));
            self.particles.push(LiveParticle {
                age: 0.0,
                life,
                mass,
                position,
                velocity,
                size,
                rotation,
                rotation_base: rotation,
                spin,
                color: e.color.each_ref().map(|p| {
                    p.sample_spawn(random(&mut self.seed), self.age)
                        .clamp(0.0, 255.0)
                }),
                tile: sample_tile(&mut self.seed, e.render.tiles),
            });
        }
        if !self.particles.is_empty() {
            for n in noise.iter_mut() {
                n.advance(self.age);
            }
        }
        let (scale, translation, offset) = match space {
            Some(space) => (space.scale, space.world.translation, [0.0; 3]),
            None => (1.0, self.anchor, self.anchor),
        };
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
        let shared_spin = self.particles.first().map_or([0.0; 3], |first| {
            std::array::from_fn(|j| {
                e.spin[j].animate_particle(first.spin[j], first.age, first.life, self.age)
            })
        });
        for p in &mut self.particles {
            if e.forces.is_empty() {
                let step = scale * dt;
                for j in 0..3 {
                    p.position[j] += p.velocity[j] * step;
                }
            } else {
                let mass = e.mass.animate_particle(p.mass, p.age, p.life, self.age);
                let placed = std::array::from_fn(|j| p.position[j] + offset[j]);
                let mut force = [0.0; 3];
                for applied in &self.applied {
                    applied.add(&mut force, placed, p.velocity, mass, dt, noise);
                }
                if let Some(space) = space {
                    space.integrate(&mut p.position, &mut p.velocity, force, mass, dt);
                } else {
                    for (j, force) in force.into_iter().enumerate() {
                        p.velocity[j] += force * dt / mass;
                        p.position[j] += p.velocity[j] * dt;
                    }
                }
            }
            for j in 0..3 {
                if e.rotation[j].has_live_curve() {
                    p.rotation[j] = e.rotation[j].animate_particle(
                        p.rotation_base[j],
                        p.age,
                        p.life,
                        self.age,
                    );
                }
                p.rotation[j] += shared_spin[j] * dt;
            }
        }
        if let Some(space) = space {
            self.previous_translation = Some(space.world.translation);
        }
        self.update_age = self.age;
        self.age += dt;
    }
    #[cfg(test)]
    fn advance(&mut self, e: &Emitter, time: f32, rate: f32) {
        self.advance_space(e, time, rate, None);
    }
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
    start_members(members, target, &mut noise);
    if reset || prepared || rate == 0.0 {
        if let Some(at) = leader {
            members[at].sim.noise = noise;
        }
        return settle(members);
    }
    let mut remaining = if delta > 10.0 {
        0.0
    } else {
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

struct Visible<'a> {
    key: (usize, bool, usize),
    emitter: &'a Emitter,
    seed: u64,
    anchor: [f32; 3],
    alpha: f32,
    views: native::ViewMask,
}

type ParentView = ([f32; 3], viewer_gpu::province_admission::Planes);

fn parent_views(cameras: &[Camera], size: [u32; 2]) -> Vec<ParentView> {
    cameras
        .iter()
        .map(|&camera| (camera.eye(), side_planes(camera, size)))
        .collect()
}

fn parent_admission(center: [f32; 3], radius: f32, views: &[ParentView]) -> Option<native::ViewMask> {
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

#[derive(Default)]
struct Drawn {
    at: Option<(u64, i32, u64)>,
    chosen: Vec<Option<((usize, bool), usize, i32)>>,
    calls: u64,
}

impl Drawn {
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
            let _span = viewer_runtime::profiling::span("weather_pairs");
            into.clear();
            let mut selectors = BTreeMap::new();
            for a in anchors {
                into.push(draw_pair(systems, presentation, a, &mut selectors));
            }
        };
        if self.at != Some(at) || self.chosen.len() != anchors.len() {
            viewer_runtime::view_record::add("weather packets", "pairs_drawn", 1);
            walk(&mut self.chosen);
            self.at = Some(at);
        } else if viewer_scene_base::actor_check::due(&mut self.calls) {
            let mut again = Vec::new();
            walk(&mut again);
            viewer_scene_base::actor_check::checked("weather pairs", again.len() as u64);
            for (index, (kept, made)) in self.chosen.iter().zip(&again).enumerate() {
                if kept != made {
                    viewer_scene_base::actor_check::differs("weather pairs", &index, &(kept, made));
                }
            }
        }
        &self.chosen
    }
}

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

struct SoundInputs<'a> {
    anchors: &'a [Anchor],
    systems: &'a BTreeMap<(usize, bool), Vec<WeatherPair>>,
    fades: &'a fade::Fades,
    presentation: &'a Presentation,
    drawn: &'a [Option<((usize, bool), usize, i32)>],
    starts: &'a mut SoundStarts,
    time: f32,
}

type SoundStarts = BTreeMap<(usize, usize, u32, i32), f32>;

fn sound_events(
    inputs: SoundInputs<'_>,
    views: &[ParentView],
    names: &mut viewer_models::sound_events::Names,
    out: &mut Vec<viewer_models::sound_events::SoundEvent>,
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
            let start = starts
                .entry((index, member, a.region, seed_hour))
                .or_insert(time);
            *start = start.min(time);
            for (event, sound) in pair.sounds[member].iter().enumerate() {
                let age = time - *start - sound.time;
                if age < 0.0 {
                    continue;
                }
                out.push(viewer_models::sound_events::weather(
                    (index, member, event, a.region, seed_hour),
                    names,
                    &sound.effect,
                    a.position,
                    age,
                ));
            }
        }
    }
    starts.retain(|&(_, _, region, seed_hour), _| {
        presentation
            .0
            .get(&region)
            .is_some_and(|state| state.seed_hour == seed_hour)
    });
}

#[derive(Clone)]
pub struct Prepared {
    field: Field,
    camera: Camera,
    size: [u32; 2],
}

pub struct Retired(
    #[expect(
        dead_code,
        reason = "held only so the window's dropper frees the runtimes off the frame path"
    )]
    Vec<Simulation>,
);

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

pub fn prepare(_assets: &Assets, field: Field, camera: Camera, size: [u32; 2]) -> Prepared {
    Prepared {
        field,
        camera,
        size,
    }
}

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

fn weather_packets(
    assets: &Assets,
    drawn: &[Option<((usize, bool), usize, i32)>],
    frame: (Camera, &[ParentView], f32),
    fades: &fade::Fades,
    simulations: &mut Simulations,
    packets: &mut Vec<native::DrawPacket>,
) -> bool {
    let (camera, views, time) = frame;
    let rate = gates(camera.height, &assets.graphics).2;
    let visible_span = viewer_runtime::profiling::span("weather_visible");
    let seen = visible_emitters(assets, |region| fades.members(region), views, drawn);
    drop(visible_span);
    viewer_runtime::view_record::add("weather packets", "anchors", assets.anchors.len() as u64);
    viewer_runtime::view_record::add("weather packets", "emitters_admitted", seen.len() as u64);
    for visible in &seen {
        simulations.entry(visible.key).or_insert_with(|| {
            Simulation::cold(visible.emitter, visible.seed, visible.anchor, time)
        });
    }
    debug_assert!(seen.windows(2).all(|pair| pair[0].key < pair[1].key));
    let advance_span = viewer_runtime::profiling::span("weather_advance");
    let eye = sort::Eye::of(camera);
    let mut seen = seen.into_iter().peekable();
    let tasks: Vec<_> = simulations
        .iter_mut()
        .filter_map(|(key, simulation)| {
            if let Some(visible) = seen.next_if(|visible| visible.key == *key) {
                Some((simulation, visible))
            } else {
                simulation.time = time.max(0.0);
                if fades.members(assets.anchors[key.0].region)[usize::from(key.1)].active {
                    simulation.sort(eye);
                }
                None
            }
        })
        .collect();
    let mut tasks = tasks;
    for system in tasks.chunk_by_mut(|(_, a), (_, b)| {
        (a.key.0, a.key.1, a.emitter.actor) == (b.key.0, b.key.1, b.emitter.actor)
    }) {
        let mut members: Vec<Member<'_>> = system
            .iter_mut()
            .map(|(sim, visible)| {
                let e = visible.emitter;
                if sim.waiting()
                    && let Some(born) = fades.born(assets.anchors[visible.key.0].region)
                {
                    sim.event_at = Some(born + e.delay);
                }
                let space = motion::Space::new(
                    e.local_space,
                    e.system_local_space,
                    e.scale,
                    content_reader_mesh::mesh::Transform {
                        translation: sim.anchor,
                        ..content_reader_mesh::mesh::Transform::IDENTITY
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
    let mut moving = false;
    for (sim, mut visible) in tasks {
        moving |= rate > 0.0 && visible.alpha > 0.0 && visible.emitter.max > 0
            && (sim.waiting() || sim.emitting || !sim.particles.is_empty());
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
    if viewer_runtime::view_record::on() {
        use viewer_runtime::view_record::add;
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
    moving
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

struct Advance {
    assets: Arc<Assets>,
    simulations: Simulations,
    fades: fade::Fades,
    drawn: Drawn,
    frame: (Camera, Vec<ParentView>, f32),
    packets: Vec<native::DrawPacket>,
    moving: bool,
}

impl Advance {
    fn run(mut self) -> Self {
        let (camera, views, time) = &self.frame;
        self.moving = weather_packets(
            &self.assets,
            &self.drawn.chosen,
            (*camera, views, *time),
            &self.fades,
            &mut self.simulations,
            &mut self.packets,
        );
        self
    }
}

#[derive(Default)]
pub struct Packets {
    draws: Vec<native::DrawPacket>,
    pub moving: bool,
}

impl Packets {
    pub fn count(&self) -> usize {
        self.draws.len()
    }
}

pub struct Pass {
    #[cfg(any(test, feature = "test-support"))]
    pub offline_packets: BTreeMap<String, usize>,
    native: native::Renderer,
    assets: Arc<Assets>,
    field: Field,
    wanted: Presentation,
    presentation: Presentation,
    fades: fade::Fades,
    clock: Clock,
    simulations: Simulations,
    pending: Simulations,
    retired: Vec<Simulation>,
    entity_simulations: native::EntitySimulations,
    sound_starts: SoundStarts,
    events: Vec<viewer_models::entity_animation::Event>,
    drawn: Drawn,
    changes: u64,
    lane: viewer_scene_base::lane::Lane<Advance>,
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
            lane: viewer_scene_base::lane::Lane::new("weather particles"),
            #[cfg(any(test, feature = "test-support"))]
            offline_packets: Default::default(),
        })
    }
    pub fn set_events(
        &mut self,
        events: Vec<viewer_models::entity_animation::Event>,
    ) -> io::Result<Vec<viewer_scene_base::pointlights::Light>> {
        let (events, lights) = advance_entity_effects(
            &mut self.entity_simulations,
            &self.assets.entities,
            &self.assets.lights,
            events,
        )?;
        static PROFILE: OnceLock<bool> = OnceLock::new();
        if *PROFILE.get_or_init(|| std::env::var_os("BATTLEPLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some()) {
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
    fn adopt_field(
        &mut self,
        field: Field,
        preparation: Option<(Camera, [u32; 2])>,
        continuity: Continuity,
    ) -> Vec<Simulation> {
        let _cpu = replay_frame::cpu::scope("viewer", Some("weather"));
        self.settle();
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

    pub fn set_prepared(&mut self, prepared: Prepared, continuity: Continuity) -> Retired {
        Retired(self.adopt_field(
            prepared.field,
            Some((prepared.camera, prepared.size)),
            continuity,
        ))
    }
    pub fn produce(&mut self, height: f32, schedule: &Schedule, weather: bool) {
        self.settle();
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

    pub fn sound_events(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        time: f32,
        names: &mut viewer_models::sound_events::Names,
        out: &mut Vec<viewer_models::sound_events::SoundEvent>,
    ) {
        self.settle();
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

    pub fn begin_frame(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        cameras: &[Camera],
        animation_time: f32,
    ) {
        self.settle();
        let time = self
            .clock
            .advance(animation_time, camera.height, &self.assets.graphics);
        let views = parent_views(cameras, size);
        self.fades.stamp(time);
        let Self {
            assets,
            field,
            presentation,
            drawn,
            changes,
            ..
        } = self;
        drawn.for_field(&assets.anchors, &assets.systems, field, presentation, *changes);
        let advance = Advance {
            assets: Arc::clone(&self.assets),
            simulations: std::mem::take(&mut self.simulations),
            fades: std::mem::take(&mut self.fades),
            drawn: std::mem::take(&mut self.drawn),
            frame: (camera, views, time),
            packets: Vec::new(),
            moving: false,
        };
        self.lane.start(move || advance.run());
    }

    fn settle(&mut self) -> Packets {
        let Some(advance) = self.lane.take() else {
            return Packets::default();
        };
        self.simulations = advance.simulations;
        self.fades = advance.fades;
        self.drawn = advance.drawn;
        Packets {
            draws: advance.packets,
            moving: advance.moving,
        }
    }

    pub fn join_frame(&mut self) {
        self.settle();
    }

    pub fn finish_frame(&mut self, camera: Camera) -> Packets {
        let _profile = viewer_runtime::profiling::thread_span("weather_packets");
        let Packets {
            draws: mut packets,
            mut moving,
        } = self.settle();
        let entity_span = viewer_runtime::profiling::span("weather_entity_packets");
        let before = packets.len();
        self.entity_simulations.sort(sort::Eye::of(camera));
        self.entity_simulations.packets(&self.assets.entities, &self.events, &mut packets);
        moving |= packets[before..]
            .iter()
            .any(|packet| !packet.instances.is_empty());
        drop(entity_span);
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
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
        let _order_span = viewer_runtime::profiling::span("weather_order");
        native::order(&mut packets);
        Packets {
            draws: packets,
            moving,
        }
    }

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

    #[expect(clippy::too_many_arguments, reason = "the arena, encoder and targets, plus the camera, view, native inputs and packets of one draw")]
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
                &packets.draws,
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

pub fn advance_entity_effects(
    simulations: &mut EntitySimulations,
    entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
    lights: &super::entity_effects::Lights,
    mut events: Vec<viewer_models::entity_animation::Event>,
) -> io::Result<(
    Vec<viewer_models::entity_animation::Event>,
    Vec<viewer_scene_base::pointlights::Light>,
)> {
    let prepare_span = viewer_runtime::profiling::span("entity_prepare");
    simulations.prepare(entities, &events)?;
    drop(prepare_span);
    let _lights_span = viewer_runtime::profiling::span("entity_lights");
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
