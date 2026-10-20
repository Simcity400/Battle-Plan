//! Weather particles from the recorded content stack and regional weather.
//!
//! The first nonzero rain, heavy-rain, snow, blizzard or sandstorm flag selects
//! the weather kind. Each authored anchor creates a primary/fading entity pair;
//! its small flag selects the small descriptor bank, falling back to the
//! regular bank if absent. Choice precedes culling and persists until the
//! weather kind changes. Camera thresholds and playback rates come from
//! content. The primary member uses the ordinary near/far camera fade; the
//! companion draws only in the far band.
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
//! the game. Forward playback skips retain unchanged weather populations;
//! seeks, step backs, branches, gaps and viewing-country changes reset them.
//! Renderer seeds, deltas and earlier particle buffers are not recorded.
use std::collections::BTreeSet;
use std::sync::OnceLock;
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::map::province_admission::side_planes;
use viewer_foundation::map::province_admission::views as province_admission_views;
use viewer_foundation::map::tables::rows;
use viewer_foundation::map::textures::load_file;
use viewer_foundation::map::{
    camera::Camera,
    gpu_contract::Targets,
    textures::{self, ColorSpace},
};
mod motion;
mod native;
use eframe::wgpu;
use installed_content_core::{ResolvedLayers, parser, parser::Item, parser::Value, particles::Catalog};
use installed_content_image::{dds, native::Decode};
pub(crate) use native::EntitySimulations;
pub use native::Inputs as NativeInputs;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
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
        let mut first = t.first().copied().unwrap_or("").split(',');
        let base = first.next().and_then(|s| s.parse().ok()).unwrap_or(default);
        let curve = first.next().and_then(|s| curves.get(s)).cloned();
        let spread = t.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
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
}
impl Curve {
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
    position: [f32; 3],
    velocity: Parameter,
    pitch: Parameter,
    yaw: Parameter,
    life: Parameter,
    mass: Parameter,
    start: Parameter,
    duration: f32,
    size: Parameter,
    rotation: [Parameter; 3],
    spin: [Parameter; 3],
    color: [Parameter; 4],
    emission: f32,
    max: usize,
    acceleration: [f32; 3],
    turbulence: f32,
    noise_division: usize,
    local_space: bool,
    render: installed_content_core::particles::RenderDescriptor,
    system_local_space: bool,
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
        curves.insert(
            text(a, "name").to_owned(),
            Curve::new(
                &values
                    .chunks_exact(2)
                    .map(|p| [p[0], p[1]])
                    .collect::<Vec<_>>(),
                number(a, "start", 0.0),
                number(a, "duration", 1.0),
                number(a, "minValue", 0.0),
                number(a, "maxValue", 1.0),
            ),
        );
    }
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
        let render = installed_content_core::particles::RenderDescriptor::read(s)?;
        let file = render.texture.as_str();
        let page = if let Some(&p) = pages.get(file) {
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
        let param = |k, d| Parameter::read(s, k, d, &curves);
        let mut acceleration = [0.0; 3];
        let mut turbulence = 0.0;
        let mut noise_division = 1;
        let forces = text(s, "force").split(',').collect::<Vec<_>>();
        for f in root.iter().filter(|i| i.key.as_deref() == Some(b"force")) {
            let f = f.value.items().unwrap_or(&[]);
            if !forces.contains(&text(f, "name")) {
                continue;
            }
            let amount = number(f, "amount", 0.0);
            if text(f, "type") == "turbulence" {
                turbulence += amount;
                noise_division = (number(f, "division", 1.0) as usize)
                    .clamp(1, 256)
                    .next_power_of_two();
            }
            if text(f, "type") == "planar" {
                for (i, v) in get(f, "direction")
                    .map(tokens)
                    .unwrap_or_default()
                    .iter()
                    .take(3)
                    .enumerate()
                {
                    acceleration[i] += v.parse::<f32>().unwrap_or(0.0) * amount;
                }
            }
        }
        let col = block(s, "color");
        emitters.push(Emitter {
            scale: 1.0,
            page,
            registration: 0,
            actor: 0,
            radius: param("sphere_emitter_radius", 0.0),
            sphere_pitch: param("sphere_emitter_pitch", 0.0),
            sphere_yaw: param("sphere_emitter_yaw", 0.0),
            emitter_yaw: param("emitter_yaw", 0.0),
            emitter_pitch: param("emitter_pitch", 0.0),
            position: ["x", "y", "z"].map(|k| number(block(s, "position"), k, 0.0)),
            velocity: param("velocity", 0.0),
            pitch: param("velocity_pitch", 0.0),
            yaw: param("velocity_yaw", 0.0),
            life: param("life", 1.0),
            mass: param("mass", 1.0),
            start: param("start", 0.0),
            duration: number(s, "duration", -1.0),
            size: param("size", 1.0),
            rotation: ["particle_yaw", "particle_pitch", "rotation"].map(|k| param(k, 0.0)),
            spin: [
                "rotation_speed_yaw",
                "rotation_speed_pitch",
                "rotation_speed",
            ]
            .map(|k| param(k, 0.0)),
            color: ["x", "y", "z", "alpha"].map(|k| Parameter::read(col, k, 255.0, &curves)),
            emission: number(s, "emission", 0.0),
            max: number(s, "max_amount", 0.0) as usize,
            acceleration,
            turbulence,
            noise_division,
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
    Ok(emitters)
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
type LocalSystem = ((usize, bool), WeatherPair, Vec<(String, dds::Texture)>);

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
    systems: BTreeMap<(usize, bool), Vec<WeatherPair>>,
    pages: Vec<dds::Texture>,
    entities: BTreeMap<String, (f32, Vec<Emitter>)>,
    /// `pdxparticle` objects whose particle block is missing; model events
    /// naming them draw nothing.
    pub unresolved: Vec<String>,
    lights: super::entity_effects::Lights,
}
impl Assets {
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content_core::defines::Defines,
        map: [u32; 2],
        decode: Decode,
    ) -> io::Result<Arc<Self>> {
        let catalog = Catalog::read(layers)?;
        let mut out = Self {
            effect: super::particle_shader::load(layers, map, decode)?,
            max_particles: catalog.max_particles(),
            graphics: CameraDefines::load(defines)?,
            anchors: Vec::new(),
            systems: BTreeMap::new(),
            pages: Vec::new(),
            entities: BTreeMap::new(),
            unresolved: Vec::new(),
            lights: super::entity_effects::Lights::load(layers)?,
        };
        let bytes = layers.read_uncovered("map/weatherpositions.txt")?;
        for line in String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let f = line.split(';').collect::<Vec<_>>();
            if f.len() != 5 {
                return Err(invalid("weatherpositions: expected region;x;y;z;size"));
            }
            out.anchors.push(Anchor {
                region: f[0].parse().map_err(|_| invalid("weather region id"))?,
                position: [f[1], f[2], f[3]].map(|s| s.parse::<f32>().unwrap_or(f32::NAN)),
                small: f[4].trim() == "small",
            });
        }
        if out
            .anchors
            .iter()
            .any(|a| a.position.iter().any(|x| !x.is_finite()))
        {
            return Err(invalid("non-finite weather anchor"));
        }
        let descriptors = installed_content_weather::weather::effects(layers, &catalog)?;
        let keys: Vec<_> = descriptors
            .iter()
            .flat_map(|(&key, pairs)| pairs.iter().map(move |pair| (key, pair)))
            .collect();
        // Parse the actual named particle blocks. Neither the entity's name nor
        // pdxparticle.type establishes a filename; content resolves both.
        let systems: Vec<io::Result<LocalSystem>> = std::thread::scope(|scope| {
            keys.iter()
                .map(|(key, pair)| {
                    scope.spawn(move || {
                        let mut textures = Vec::new();
                        let mut pages = BTreeMap::new();
                        let mut emitters = [Vec::new(), Vec::new()];
                        for (slot, references) in
                            [&pair.entity, &pair.fading_entity].into_iter().enumerate()
                        {
                            for (actor, reference) in references.iter().enumerate() {
                                let document =
                                    parser::parse(&layers.read_uncovered(&reference.path)?)?;
                                let root = particle_root(
                                    &document,
                                    &reference.path,
                                    &reference.name,
                                    Pick::Last,
                                )?;
                                let mut parsed =
                                    read_emitters(layers, decode, root, &mut textures, &mut pages)?;
                                for emitter in &mut parsed {
                                    emitter.scale = reference.scale;
                                    emitter.registration = reference.registration;
                                    emitter.actor = actor;
                                }
                                emitters[slot].extend(parsed);
                            }
                        }
                        let mut files = vec![String::new(); textures.len()];
                        for (file, id) in pages {
                            files[id] = file;
                        }
                        Ok((
                            *key,
                            WeatherPair {
                                members: emitters,
                                radii: pair
                                    .parents
                                    .each_ref()
                                    .map(|parent| parent.as_ref().map(|p| p.radius)),
                                sounds: pair.parents.each_ref().map(|parent| {
                                    parent
                                        .as_ref()
                                        .map(|p| p.sounds.clone())
                                        .unwrap_or_default()
                                }),
                            },
                            files.into_iter().zip(textures).collect(),
                        ))
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
            ) = system?;
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
        let mut by_file: BTreeMap<String, Vec<(String, installed_content_core::particles::Particle)>> =
            BTreeMap::new();
        for object in catalog.objects() {
            match catalog.resolve(object) {
                Ok(particle) => by_file
                    .entry(particle.path.clone())
                    .or_default()
                    .push((object.to_owned(), particle)),
                Err(_) => out.unresolved.push(object.to_owned()),
            }
        }
        type Parsed = Vec<(String, f32, LocalEmitters)>;
        let files: Vec<io::Result<Parsed>> = std::thread::scope(|scope| {
            by_file
                .iter()
                .map(|(path, objects)| {
                    scope.spawn(move || {
                        let document = parser::parse(&layers.read_uncovered(path)?)?;
                        objects
                            .iter()
                            .map(|(object, particle)| {
                                let root =
                                    particle_root(&document, path, &particle.name, Pick::First)?;
                                let mut textures = Vec::new();
                                let mut local = BTreeMap::new();
                                let mut emitters =
                                    read_emitters(layers, decode, root, &mut textures, &mut local)?;
                                for emitter in &mut emitters {
                                    emitter.registration = particle.registration;
                                }
                                let mut files = vec![String::new(); textures.len()];
                                for (file, id) in local {
                                    files[id] = file;
                                }
                                Ok((
                                    object.clone(),
                                    particle.scale,
                                    (emitters, files.into_iter().zip(textures).collect()),
                                ))
                            })
                            .collect()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("particle loader panicked"))
                .collect()
        });
        for file in files {
            for (name, scale, (mut emitters, local)) in file? {
                merge_pages(&mut out.pages, &mut global, &mut emitters, local);
                out.entities.insert(name, (scale, emitters));
            }
        }
        Ok(Arc::new(out))
    }
}

/// Hourly flags. Weather is global simulation state; the engine's dispatch
/// has no country/intel predicate, so every recorded perspective uses it.
#[derive(Clone, Default, PartialEq)]
pub struct Field {
    branch: replay_data::BranchId,
    hour: i32,
    kinds: BTreeMap<u32, usize>,
}
pub fn field(frame: &DecodedFrame) -> io::Result<Field> {
    let mut out = Field {
        branch: frame.branch,
        hour: frame.hour,
        ..Default::default()
    };
    for r in rows::<tables::WeatherRegions>(frame)?.rows() {
        if let Some(kind) = [r.light_rain, r.heavy_rain, r.snow, r.blizzard, r.sandstorm]
            .iter()
            .position(|v| *v != 0)
        {
            out.kinds.insert(r.id, kind);
        }
    }
    Ok(out)
}

/// The alpha of the authored `entity` (the zoom fade), of the
/// `fading_entity` (drawn only in the far band) and the playback rate.
fn gates(h: f32, defines: &CameraDefines) -> (f32, f32, f32) {
    let far = defines.distance_cutoff - defines.distance_fade_length < h;
    let alpha = if h < defines.zoom_in_cutoff {
        defines.zoom_in_fade_factor
    } else if h > defines.distance_cutoff {
        0.0
    } else if far {
        (defines.distance_cutoff - h) / defines.distance_fade_length
    } else if h < defines.zoom_in_cutoff + defines.zoom_in_fade_length {
        ((h - defines.zoom_in_cutoff) / defines.zoom_in_fade_length)
            * (1.0 - defines.zoom_in_fade_factor)
            + defines.zoom_in_fade_factor
    } else {
        1.0
    };
    let rate = ((defines.playback_rate_cutoff - h) / defines.playback_rate_length)
        * (1.0 - defines.playback_rate)
        + defines.playback_rate;
    (
        alpha,
        if far { alpha } else { 0.0 },
        rate.clamp(defines.playback_rate, 1.0),
    )
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct RegionPresentation {
    kind: usize,
    seed_hour: i32,
}

#[derive(Default)]
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
        for (&region, &kind) in &next.kinds {
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

/// Use the small bank only when nonempty. Every admitted anchor consumes a counter-hash draw,
/// including off-camera anchors. Presentation supplies the seed.
fn select_pair<'a>(
    systems: &'a BTreeMap<(usize, bool), Vec<WeatherPair>>,
    kind: usize,
    small: bool,
    seed: &mut u64,
) -> Option<&'a WeatherPair> {
    let bank = systems
        .get(&(kind, small))
        .filter(|bank| !bank.is_empty())
        .or_else(|| systems.get(&(kind, false)).filter(|bank| !bank.is_empty()))?;
    Some(&bank[random_bits(seed) as usize % bank.len()])
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
    /// Boxed: at 10.5 KB inline, every simulation moved in or out of the map
    /// on an hour change copied it, tens of megabytes an hour in winter.
    noise: Option<Box<Noise>>,
    particles: Vec<LiveParticle>,
    accumulator: f32,
    age: f32,
    time: f32,
    previous_translation: Option<[f32; 3]>,
}
impl Simulation {
    /// Native particle-runtime construction starts with empty storage. The
    /// producer creates a fresh pair on a weather-kind change, and its active
    /// bit gates later manager updates.
    fn cold(e: &Emitter, seed: u64, anchor: [f32; 3], time: f32) -> Self {
        let mut out = Self {
            prepared: false,
            emitting: true,
            anchor,
            initial_seed: seed,
            seed,
            noise: None,
            particles: Vec::with_capacity(e.max),
            accumulator: 0.0,
            age: 0.0,
            time: time.max(0.0),
            previous_translation: None,
        };
        if e.turbulence != 0.0 {
            out.noise = Some(Box::new(Noise::new(&mut out.seed, e.noise_division)));
        }
        // Initialize subsystem age to negative sampled start.
        out.age = -e.start.sample(random(&mut out.seed), 0.0);
        out
    }

    fn tick(&mut self, e: &Emitter, dt: f32) {
        self.tick_space(e, dt, None);
    }

    fn tick_space(&mut self, e: &Emitter, dt: f32, space: Option<motion::Space>) {
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
            self.emitting && self.age >= 0.0 && (e.duration < 0.0 || self.age <= e.duration);
        if emitting {
            self.accumulator += e.emission.max(0.0) * dt;
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
                || std::array::from_fn(|j| e.position[j] + offset[j]),
                |space| {
                    space.birth(
                        self.previous_translation.unwrap_or(space.world.translation),
                        e.position,
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
        if let Some(n) = &mut self.noise
            && !self.particles.is_empty()
        {
            n.advance(self.age);
        }
        for p in &mut self.particles {
            let force = self.noise.as_ref().map_or([0.0; 3], |n| {
                n.force(if space.is_some() {
                    p.position
                } else {
                    std::array::from_fn(|j| p.position[j] + self.anchor[j])
                })
            });
            let force = std::array::from_fn(|j| e.acceleration[j] + e.turbulence * force[j]);
            // Newborns move in this tick too. The comparison simulator uses unit displacement scale;
            // world-space motion uses the runtime's retained displacement scale.
            let mass = e.mass.animate(p.mass, p.age / p.life);
            if let Some(space) = space {
                space.integrate(&mut p.position, &mut p.velocity, force, mass, dt);
            } else {
                // Preserve the comparison simulator's original evaluation order.
                for (j, force) in force.into_iter().enumerate() {
                    p.velocity[j] += force * dt / mass;
                    p.position[j] += p.velocity[j] * dt;
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
    fn advance_event(
        &mut self,
        e: &Emitter,
        age: f32,
        end: Option<f32>,
        space: Option<motion::Space>,
    ) {
        if let Some(end) = end {
            if self.time < end {
                self.advance_space(e, end.min(age), 1., space);
            }
            self.emitting = age < end;
        }
        self.advance_space(e, age, 1., space);
    }

    fn advance_space(&mut self, e: &Emitter, time: f32, rate: f32, space: Option<motion::Space>) {
        let target = time.max(0.0);
        if self.prepared {
            self.time = target;
            self.prepared = false;
            return;
        }
        if rate == 0.0 {
            self.time = target;
            return;
        }
        let delta = target - self.time;
        if delta < 0.0 {
            *self = Self::cold(e, self.initial_seed, self.anchor, time);
            return;
        }
        // Long suspensions have unknown native frame subdivisions. Preserve
        // the existing population, as a hidden native entity does, instead
        // of fabricating a full steady-state cloud or replaying many seconds.
        if delta > 10.0 {
            self.time = target;
            return;
        }
        // Subdivide unscaled frame delta in double precision, then multiply each float step by
        // entity playback.
        let mut remaining = delta as f64 / rate as f64;
        let max_step = 0.1_f32.max((remaining.abs() * 0.25) as f32) as f64;
        while remaining > 0.0 {
            let dt = remaining.min(max_step);
            if let Some(space) = space {
                self.tick_space(e, dt as f32 * rate, Some(space));
            } else {
                self.tick(e, dt as f32 * rate);
            }
            remaining -= dt;
        }
        self.time = target;
    }
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
fn parent_admission(center: [f32; 3], radius: f32, views: &[ParentView]) -> native::ViewMask {
    let mut accepted = false;
    let mut mask = vec![false; views.len()];
    for (view, (eye, planes)) in views.iter().enumerate() {
        let [dx, dy, dz] = std::array::from_fn(|axis| center[axis] - eye[axis]);
        if (dy * dy + dx * dx) + dz * dz < f32::MAX
            && !planes
                .iter()
                .any(|[a, b, c, d]| center[0] * a + center[1] * b + center[2] * c + d > radius)
        {
            accepted = true;
        }
        mask[view] = accepted;
    }
    native::ViewMask::Some(mask)
}

/// The emitters of `field` the camera sees, in drawing order.
fn visible_emitters<'a>(
    assets: &'a Assets,
    field: &Field,
    presentation: &Presentation,
    camera: Camera,
    views: &[ParentView],
) -> Vec<Visible<'a>> {
    let (entity_alpha, fading_alpha, _) = gates(camera.height, &assets.graphics);
    let mut out = Vec::new();
    let mut selectors = BTreeMap::new();
    for (index, a) in assets.anchors.iter().enumerate() {
        let Some(&kind) = field.kinds.get(&a.region) else {
            continue;
        };
        let Some(state) = presentation.0.get(&a.region) else {
            continue;
        };
        let selector = selectors.entry(a.region).or_insert_with(|| {
            (state.seed_hour as u64).wrapping_mul(0x100000001b3) ^ ((a.region as u64) << 32)
        });
        let Some(pair) = select_pair(&assets.systems, kind, a.small, selector) else {
            continue;
        };
        let anchor = a.position;
        for (far_only, alpha) in [(false, entity_alpha), (true, fading_alpha)] {
            if alpha == 0.0 {
                continue;
            }
            let member = usize::from(far_only);
            let Some(radius) = pair.radii[member] else {
                continue;
            };
            let mask = parent_admission(anchor, radius, views);
            if mask.is_empty() {
                continue;
            }
            let emitters = &pair.members[member];
            for (ei, e) in emitters.iter().enumerate() {
                out.push(Visible {
                    key: (index, far_only, ei),
                    emitter: e,
                    seed: (state.seed_hour as u64).wrapping_mul(0x100000001b3)
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

/// The sound events of the weather entities the map shows: each admitted
/// anchor's selected entity and, in the far band, its fading entity, as
/// [`visible_emitters`] admits them. A region keeps its keys while it keeps
/// its weather.
struct SoundInputs<'a> {
    anchors: &'a [Anchor],
    systems: &'a BTreeMap<(usize, bool), Vec<WeatherPair>>,
    graphics: &'a CameraDefines,
    field: &'a Field,
    presentation: &'a Presentation,
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
    height: f32,
    views: &[ParentView],
    names: &mut super::sound_events::Names,
    out: &mut Vec<super::sound_events::SoundEvent>,
) {
    let SoundInputs {
        anchors,
        systems,
        graphics,
        field,
        presentation,
        starts,
        time,
    } = inputs;
    let (entity_alpha, fading_alpha, _) = gates(height, graphics);
    let mut selectors = BTreeMap::new();
    for (index, a) in anchors.iter().enumerate() {
        let Some(&kind) = field.kinds.get(&a.region) else {
            continue;
        };
        let Some(state) = presentation.0.get(&a.region) else {
            continue;
        };
        let selector = selectors.entry(a.region).or_insert_with(|| {
            (state.seed_hour as u64).wrapping_mul(0x100000001b3) ^ ((a.region as u64) << 32)
        });
        let Some(pair) = select_pair(systems, kind, a.small, selector) else {
            continue;
        };
        for (member, alpha) in [entity_alpha, fading_alpha].into_iter().enumerate() {
            let Some(radius) = pair.radii[member] else {
                continue;
            };
            if alpha == 0.0
                || pair.sounds[member].is_empty()
                || matches!(
                    parent_admission(a.position, radius, views),
                    native::ViewMask::Some(mask) if !mask.contains(&true)
                )
            {
                continue;
            }
            // The entity enters its state when it first appears; each sound
            // event fires at its `time` after that.
            let start = starts
                .entry((index, member, a.region, state.seed_hour))
                .or_insert(time);
            *start = start.min(time);
            for (event, sound) in pair.sounds[member].iter().enumerate() {
                let age = time - *start - sound.time;
                if age < 0.0 {
                    continue;
                }
                out.push(super::sound_events::weather(
                    (index, member, event, a.region, state.seed_hour),
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

/// New, empty weather emitters prepared off the frame path for this hour.
#[derive(Clone)]
pub struct Prepared {
    field: Field,
    simulations: Simulations,
}

/// Particle runtimes an hour change let go of: the regions whose weather
/// changed, and the prepared runtimes of regions that kept theirs. Handed back
/// so the window frees them off the frame path.
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

/// Construct visible native-style particle runtimes without inventing births.
pub fn prepare(assets: &Assets, field: Field, camera: Camera, size: [u32; 2]) -> Prepared {
    let mut presentation = Presentation::default();
    presentation.transition(&Field::default(), &field, Continuity::Reset);
    let views = parent_views(&province_admission_views(camera, size), size);
    let simulations = visible_emitters(assets, &field, &presentation, camera, &views)
        .into_iter()
        .map(|visible| {
            let mut simulation =
                Simulation::cold(visible.emitter, visible.seed, visible.anchor, 0.0);
            simulation.prepared = true;
            (visible.key, simulation)
        })
        .collect();
    Prepared { field, simulations }
}

#[allow(clippy::too_many_arguments)]
fn weather_packets(
    assets: &Assets,
    field: &Field,
    presentation: &Presentation,
    frame: (Camera, &[ParentView], f32),
    simulations: &mut Simulations,
    packets: &mut Vec<native::DrawPacket>,
    workers: &viewer_foundation::parallel::Workers,
) {
    let (camera, views, time) = frame;
    let rate = gates(camera.height, &assets.graphics).2;
    let seen = visible_emitters(assets, field, presentation, camera, views);
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
    let mut seen = seen.into_iter().peekable();
    let tasks: Vec<_> = simulations
        .iter_mut()
        .filter_map(|(key, simulation)| {
            if let Some(visible) = seen.next_if(|visible| visible.key == *key) {
                Some(std::sync::Mutex::new((simulation, visible, None)))
            } else {
                // Unadmitted populations freeze; rebase so reentry invents no hidden births.
                simulation.time = time.max(0.0);
                None
            }
        })
        .collect();
    workers.run(tasks.len(), &|at| {
        let mut task = tasks[at].lock().unwrap_or_else(|error| error.into_inner());
        let (sim, visible, output) = &mut *task;
        let e = visible.emitter;
        sim.advance_space(
            e,
            time,
            rate,
            Some(motion::Space::new(
                e.local_space,
                e.system_local_space,
                e.scale,
                installed_content_core::mesh::Transform {
                    translation: sim.anchor,
                    ..installed_content_core::mesh::Transform::IDENTITY
                },
            )),
        );
        let mut packet = native::DrawPacket::weather(e, sim, visible.anchor, visible.alpha);
        packet.actor = [0, visible.key.0, usize::from(visible.key.1), e.actor];
        packet.views = std::mem::replace(&mut visible.views, native::ViewMask::All);
        *output = Some(packet);
    });
    // Preserve the producer's anchor/member/emitter order, including empty packets:
    // an enabled actor binds scale/quad IA before inspecting its particle count.
    for task in tasks {
        packets.push(
            task.into_inner()
                .unwrap_or_else(|error| error.into_inner())
                .2
                .expect("prepared weather packet"),
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

/// One shared pass for window and dump. Static textures stay resident; only
/// particles at the selected hour and visible camera are generated each frame.
pub struct Pass {
    #[cfg(any(test, feature = "test-support"))]
    pub offline_packets: BTreeMap<String, usize>,
    native: native::Renderer,
    assets: Arc<Assets>,
    field: Field,
    presentation: Presentation,
    clock: Clock,
    simulations: Simulations,
    entity_simulations: native::EntitySimulations,
    /// See [`SoundInputs::starts`].
    sound_starts: SoundStarts,
    events: Vec<super::entity_animation::Event>,
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
            presentation: Presentation::default(),
            clock: Clock::default(),
            simulations: Simulations::new(),
            entity_simulations: native::EntitySimulations::default(),
            sound_starts: SoundStarts::new(),
            events: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            offline_packets: Default::default(),
        })
    }
    pub fn set_events(
        &mut self,
        events: Vec<super::entity_animation::Event>,
    ) -> io::Result<Vec<super::pointlights::Light>> {
        self.set_events_with_workers(events, &Default::default())
    }

    pub fn set_events_with_workers(
        &mut self,
        events: Vec<super::entity_animation::Event>,
        workers: &viewer_foundation::parallel::Workers,
    ) -> io::Result<Vec<super::pointlights::Light>> {
        let (events, lights) = advance_entity_effects_with_workers(
            &mut self.entity_simulations,
            &self.assets.entities,
            &self.assets.lights,
            events,
            workers,
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
    /// Moves to `field` with `prepared` runtimes for its changed regions (see
    /// [`merge_runtimes`]), returning the runtimes it let go of.
    fn adopt_field(
        &mut self,
        field: Field,
        prepared: Simulations,
        continuity: Continuity,
    ) -> Vec<Simulation> {
        if continuity == Continuity::Reset {
            self.entity_simulations.clear();
        }
        let retained = self
            .presentation
            .transition(&self.field, &field, continuity);
        let anchors = &self.assets.anchors;
        let retired = merge_runtimes(&mut self.simulations, prepared, |(anchor, _, _)| {
            retained.contains(&anchors[*anchor].region)
        });
        self.field = field;
        retired
    }

    pub fn set(&mut self, field: Field) {
        self.adopt_field(field, Simulations::new(), Continuity::Reset);
    }

    /// Adopt new weather regions' empty runtimes while preserving an
    /// unchanged region's own particles, descriptor choice and retained alpha.
    /// Returns what it let go of, for the caller to free off the frame path.
    pub fn set_prepared(&mut self, prepared: Prepared, continuity: Continuity) -> Retired {
        Retired(self.adopt_field(prepared.field, prepared.simulations, continuity))
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
        sound_events(
            SoundInputs {
                anchors: &self.assets.anchors,
                systems: &self.assets.systems,
                graphics: &self.assets.graphics,
                field: &self.field,
                presentation: &self.presentation,
                starts: &mut self.sound_starts,
                time,
            },
            camera.height,
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
        weather: bool,
    ) -> Packets {
        self.prepare_frame_with_workers(
            camera,
            size,
            cameras,
            animation_time,
            weather,
            &Default::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_frame_with_workers(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        cameras: &[Camera],
        animation_time: f32,
        weather: bool,
        workers: &viewer_foundation::parallel::Workers,
    ) -> Packets {
        let mut packets = Vec::new();
        if weather {
            let time = self
                .clock
                .advance(animation_time, camera.height, &self.assets.graphics);
            let views = parent_views(cameras, size);
            weather_packets(
                &self.assets,
                &self.field,
                &self.presentation,
                (camera, &views, time),
                &mut self.simulations,
                &mut packets,
                workers,
            );
        } else {
            // As the game does: its weather update rereads the map-rendering weather option
            // (`draw_weather_effects`) each time. Off, the weather kind it wants is none, so it
            // releases every particle pair it holds; back on, it builds new pairs from a freshly
            // seeded random stream. This is the one display switch that changes presentation
            // state (checked against the game's 1.19.2 executable). Entity events and the shared
            // submit callback still run.
            self.simulations.clear();
            self.clock = Clock::default();
        }
        self.entity_simulations.packets_with_workers(
            &self.assets.entities,
            &self.events,
            &mut packets,
            workers,
        );
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
        native::order(&mut packets);
        Packets(packets)
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
/// no living particle.
#[cfg(test)]
pub(crate) fn advance_entity_effects(
    simulations: &mut EntitySimulations,
    entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
    lights: &super::entity_effects::Lights,
    events: Vec<super::entity_animation::Event>,
) -> io::Result<(
    Vec<super::entity_animation::Event>,
    Vec<super::pointlights::Light>,
)> {
    advance_entity_effects_with_workers(simulations, entities, lights, events, &Default::default())
}

pub(crate) fn advance_entity_effects_with_workers(
    simulations: &mut EntitySimulations,
    entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
    lights: &super::entity_effects::Lights,
    mut events: Vec<super::entity_animation::Event>,
    workers: &viewer_foundation::parallel::Workers,
) -> io::Result<(
    Vec<super::entity_animation::Event>,
    Vec<super::pointlights::Light>,
)> {
    simulations.prepare_with_workers(entities, &events, workers)?;
    for event in &mut events {
        if event.emission_end.is_some() && !simulations.living(event) {
            event.definition.light = None;
        }
    }
    let lights = lights.evaluate(&events);
    Ok((events, lights))
}

#[cfg(test)]
#[test]
fn a_reused_weather_preparation_keeps_an_independent_initial_runtime() {
    let emitter = tests::activation_test_emitter();
    let key = (0, false, 0);
    let mut simulation = Simulation::cold(&emitter, 12345, [10.0, 0.0, 20.0], 0.0);
    simulation.prepared = true;
    let prepared_seed = simulation.seed;
    let prepared = Prepared {
        field: Field {
            branch: 1,
            hour: 10,
            kinds: BTreeMap::from([(7, 0)]),
        },
        simulations: BTreeMap::from([(key, simulation)]),
    };
    let mut reused = prepared.clone();
    assert!(prepared.field == reused.field);
    let next = reused.simulations.get_mut(&key).unwrap();
    next.advance(&emitter, 8.0, 1.0);
    next.advance(&emitter, 8.25, 1.0);
    assert!(!next.particles.is_empty());
    let initial = &prepared.simulations[&key];
    assert!(initial.prepared);
    assert!(initial.particles.is_empty());
    assert_eq!(initial.seed, prepared_seed);
    assert_eq!(initial.time, 0.0);
}

#[cfg(test)]
pub(crate) mod tests;
