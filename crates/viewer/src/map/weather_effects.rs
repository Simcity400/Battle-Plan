//! Weather particles from `common/weather.txt`, `map/weatherpositions.txt` and
//! `gfx/particles/environment/*.asset`. No simulation weather is invented.
//!
//! +120/+128 (light rain/heavy rain/snow/blizzard/sandstorm; codes 2/3/4/5/7).
//! It creates one primary/fading entity pair at each authored region anchor;
//! the anchor's `small` byte chooses the small descriptor bank. There is no
//! continuous intensity read. Arctic water and temperature do not select FX.
//! Primary particles remain visible at close zoom. The fading cloud companion
//! has alpha clamp((height-358)/220,0,1); both fade from 1100 to 1500. Playback
//! slows from 1 to .15 between heights 300 and 500 (00_graphics.lua).
//!
//! `particle.shader` supplies square quads, yaw/pitch/roll or billboarding,
//! ToLinear(vertex colour), texture modulation, optional distance fog, and
//! SRC_ALPHA/ONE or SRC_ALPHA/INV_SRC_ALPHA, RGB writes only. Captured events
//! 15820/15994/16006/16028 in hoi4_frame33840.rdc confirm BC3_SRGB, depth
//! LESS_EQUAL with no writes, and the float scene before Restore at 16280.
//! Shadercache ps_5_0/0008000008BF0000F243D9EA.scache is the NO_BILLBOARD
//! additive variant; 0008000025BF00000790994F.scache is billboard additive.
//!
//! UNVERIFIED: stochastic initialization uses a region/hour/anchor seed and
//! stationary birth phases, not the unrecorded engine seed, particles or time.
//! Variation is uniform +/- the authored range (rain sizes verified against
//! captured event 15770: 152 particles, size 5.0066..6.9903, mean 6.0039).
//! Its positions also establish the velocity convention: negative local Z is
//! forward, so yaw -30 moves +X/-Z; rain starts at anchor Y+25 and falls
//! at 25*sin(-45) units/s. Curves are piecewise linear,
//! and turbulence is a deterministic sinusoidal acceleration. A particle
//! update decompile or consecutive captured instance buffers would settle
//! these dynamics. Position, emission, lifetime, size, colour, rotations,
//! planar forces and curve knots are read from content, never fitted to a PNG.
//! Work per frame is bounded by visible authored emitters, not replay length.

//!
//! # Cloud companions, distant masses and the Poland sheet
//!
//! The second member of each weather_effect is the fading cloud entity, not
//! a scrolling terrain texture. Rain uses cloud_6.dds (additive, RGB 110/120/
//! 150, alpha 180+/-20, emission 8/s, life 2.6+/-0.7s, size 60.5+/-20);
//! heavy rain uses alpha-blended cloud_6.dds (RGB160, alpha150+/-10,
//! emission15/s, life10+/-2.5s, maximum25, size60.5+/-15.5) plus flashes.
//! Snow uses snow_cloud.dds (RGB200, alpha60, emission7/s, life1.6+/-0.7s,
//! size60.5+/-10). Blizzard and small variants read their own asset blocks.
//! Quads use their full texture once, with no world tiling. Their authored
//! velocity/turbulence, the life curve and the camera gate control motion and
//! opacity. The primary snow systems also contain faint cloud subsystems;
//! those remain visible below the companion's 358 cutoff. Sandstorm has no
//! fading companion. There is no particle shadow-caster effect: the optional
//! PARTICLE_SHADOWS shader define RECEIVES a shadow, and is absent from the
//! cached weather variants. Dark alpha-blended clouds are not cast shadows.
//!
//! pairD_out10 (height1167.101) draws companions at alpha .83225; out20
//! (2122.295) cannot draw any weather entity. Its large water patterns already
//! appear in the pre-existing water pass, which samples the authored
//! colormap_water_0.dds and reflection.dds, not weather. sky.shader only samples
//! the reflection cube along the view ray; it has no scrolling cloud overlay.
//! UNVERIFIED: attribution of each lobby/out20 patch to colormap versus cube
//! reflection requires a saved far/lobby RenderDoc draw or texture-toggle pair.
//!
//! The broad pale Poland sheet at height164 follows the unseen provinces and
//! question-mark counters: fow.fxh's terrain/tree pre-pass, already ported in
//! shadow.rs/common.wgsl, uses ProvinceVisibility, height12..15, fow_noise_0.dds
//! and fow_rgb_waterspec_a.dds. Its three alpha octaves tile .001/.005/.01 and
//! drift at .0015*(.7,.7), .0075*(-.8,-.2), .012*(-.1,-.9) per second;
//! transparency 0.3..0.9, camera fade ((400-h)/350)^2. This is distinct from the
//! particle FOG define (angle/distance fog on a quad's own colour). The format14
//! Poland hour lacks ProvinceVisibility and cannot reconstruct that sheet.
//! UNVERIFIED: its exact separation from faint primary snow clouds needs a
//! same-camera Draw.Fow/Draw.Weather toggle pair; no such capture was supplied.
//!
//! # Reference measurements (2026-09-15)
//!
//! Captures, JSON scores, masks and gate logs are outside the repository at
//! captures/weather_port_20260915; fitting and measurement
//! scripts are at C:/hoi4_scratch/weather. Final dumps use animation time zero.
//! Seven green VP label anchors fit pairG_03_hmin to camera
//! (3009.486699,1568.335738,50.049777), radial RMS 0.828 game-screen pixels;
//! pairG_02_h150 fits (3009.628718,1568.600765,90.996078), RMS 0.233 pixels.
//! Both use format18 visibility18_shared branch2 hour60826675. The h150
//! filename does not describe the fitted height of its screenshot.
//!
//! At common 2048x1152 resolution, the G03 rain measurement uses ROI
//! x80..1900/y170..950, a normalized 25-degree ridge template at four scales,
//! correlation >0.7 and 19px peak suppression. Game/viewer/before yield
//! 86/73/7 candidates (60.58/51.42/4.93 per megapixel). This is a high-confidence
//! candidate estimator, not a particle census: faint/overlapping streaks are
//! missed and the before image measures background false positives. Centerline
//! contrast against flanks 7px away gives median full width at half maximum
//! 27px game versus 24px viewer, IQR 21..32 versus 21..31; median contrast is
//! 38.71 versus 35.42 RGB levels. UNVERIFIED: remaining density/length mismatch
//! needs matched particle state/time or further particle-update evidence; the
//! port does not tune authored sizes or counts to one stochastic screenshot.
//!
//! Reference commands/cameras are those in reference/02..05.md. Scores are
//! MAE / deltaE / SSIM level0, with the prescribed 1.25 upscale and UI masks:
//!
//! Pair | Before c5ff8da68 | Weather before merge | Final after e679e7e56
//! 02 | 11.5649/6.9712/.7483 | 11.5669/6.9727/.7436 | 11.5669/6.9727/.7436
//! 03 | 12.3392/7.0562/.7262 | 12.2325/7.0149/.7219 | 12.2325/7.0149/.7219
//! 04 | 12.7354/7.5475/.7204 | 12.8280/7.5855/.7115 | 12.8280/7.5855/.7115
//! 05 | 13.4482/8.4289/.7317 | 13.3124/8.3676/.7287 | 13.1367/8.2354/.7328
//!
//! The merge includes counter grouping and point lights; its pair05 gain must
//! not be attributed to weather. Rain is visible in the same-camera pairF_04
//! and pairF_04_check frames (pair04_weather_check.png). Stochastic weather
//! lowers SSIM on all four weather-only comparisons; pairs02/04 also worsen
//! MAE. These are measured regressions with weather present, not exact matches.
//! Extra final MAEs: G03 15.3086 (before 15.2701), G02 13.0351, Dprobe 23.2110,
//! Dout10 11.7768, Dout20 10.0975. The latter has no weather at its height;
//! Dprobe remains limited by the missing format14 visibility table. Dedicated
//! snow/blizzard/sandstorm reference pairs and a matching lobby replay are not
//! available, so those particle appearances remain UNVERIFIED beyond assets.
//!
//! All four owner gates passed before and after the rendering merge: workspace
//! build, clippy all-targets -D warnings, fmt check, and workspace tests (2315
//! passing after merge). No live game process was used. Final tracked files:969.

use super::{
    camera::Camera,
    scene::{self, Targets},
    textures::{self, ColorSpace},
};
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    image::dds,
    parser::{self, Item, Value},
};
use replay::{frame::DecodedFrame, rows::tables};
use std::{collections::BTreeMap, io, sync::Arc};

fn invalid(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.into())
}
fn get<'a>(v: &'a [Item], key: &str) -> Option<&'a Value> {
    v.iter()
        .rev()
        .find(|i| i.key.as_deref() == Some(key.as_bytes()))
        .map(|i| &i.value)
}
fn block<'a>(v: &'a [Item], key: &str) -> &'a [Item] {
    get(v, key).and_then(Value::items).unwrap_or(&[])
}
fn text<'a>(v: &'a [Item], key: &str) -> &'a str {
    get(v, key).and_then(Value::scalar_str).unwrap_or("")
}
fn tokens(v: &Value) -> Vec<&str> {
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
fn number(v: &[Item], key: &str, default: f32) -> f32 {
    get(v, key)
        .and_then(|v| {
            tokens(v)
                .first()
                .and_then(|s| s.split(',').next()?.parse().ok())
        })
        .unwrap_or(default)
}

#[derive(Clone, Default)]
struct Parameter {
    base: f32,
    spread: f32,
    curve: Vec<[f32; 2]>,
}
impl Parameter {
    fn read(v: &[Item], key: &str, default: f32, curves: &BTreeMap<String, Vec<[f32; 2]>>) -> Self {
        let t = get(v, key).map(tokens).unwrap_or_default();
        let mut first = t.first().copied().unwrap_or("").split(',');
        let base = first.next().and_then(|s| s.parse().ok()).unwrap_or(default);
        let curve = first
            .next()
            .and_then(|s| curves.get(s))
            .cloned()
            .unwrap_or_default();
        let spread = t.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        Self {
            base,
            spread,
            curve,
        }
    }
    fn sample(&self, random: f32, age: f32) -> f32 {
        let value = self.base + (random * 2.0 - 1.0) * self.spread;
        value
            * if self.curve.is_empty() {
                1.0
            } else {
                curve(&self.curve, age)
            }
    }
}
fn curve(knots: &[[f32; 2]], x: f32) -> f32 {
    for p in knots.windows(2) {
        if x <= p[1][0] {
            let t = ((x - p[0][0]) / (p[1][0] - p[0][0]).max(1e-6)).clamp(0.0, 1.0);
            return p[0][1] + t * (p[1][1] - p[0][1]);
        }
    }
    knots.last().map_or(1.0, |p| p[1])
}
struct Emitter {
    page: usize,
    additive: bool,
    billboard: bool,
    radius: Parameter,
    position: [f32; 3],
    velocity: Parameter,
    pitch: Parameter,
    yaw: Parameter,
    life: Parameter,
    size: Parameter,
    rotation: [Parameter; 3],
    spin: [Parameter; 3],
    color: [Parameter; 4],
    emission: f32,
    max: usize,
    acceleration: [f32; 3],
    turbulence: f32,
}
struct Anchor {
    region: u32,
    position: [f32; 3],
    small: bool,
}
/// Authored resources, loaded once per document and shared with its GPU scene.
pub struct Assets {
    anchors: Vec<Anchor>,
    systems: BTreeMap<(usize, bool, bool), Vec<Emitter>>,
    pages: Vec<dds::Texture>,
}
impl Assets {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Arc<Self>> {
        let mut out = Self {
            anchors: Vec::new(),
            systems: BTreeMap::new(),
            pages: Vec::new(),
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
        let mut pages = BTreeMap::new();
        for (kind, name) in ["rain", "lightning_storm", "snow", "snow_storm", "sandstorm"]
            .iter()
            .enumerate()
        {
            for small in [false, true] {
                for clouds in [false, true] {
                    if kind == 4 && (small || clouds) {
                        continue;
                    }
                    let path = format!(
                        "gfx/particles/environment/{name}{}{}.asset",
                        if small { "_small" } else { "" },
                        if clouds { "_clouds" } else { "" }
                    );
                    let doc = parser::parse(&layers.read_uncovered(&path)?)?;
                    let root = block(&doc.items, "particle");
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
                            values.chunks_exact(2).map(|p| [p[0], p[1]]).collect(),
                        );
                    }
                    let mut emitters = Vec::new();
                    for s in root
                        .iter()
                        .filter(|i| i.key.as_deref() == Some(b"subsystem"))
                    {
                        let s = s.value.items().unwrap_or(&[]);
                        if text(s, "hide") == "yes" {
                            continue;
                        }
                        let tex = block(s, "texture");
                        let file = text(tex, "file");
                        let page = if let Some(&p) = pages.get(file) {
                            p
                        } else {
                            let p = out.pages.len();
                            out.pages.push(dds::decode(&layers.read_uncovered(file)?)?);
                            pages.insert(file.to_owned(), p);
                            p
                        };
                        let param = |k, d| Parameter::read(s, k, d, &curves);
                        let mut acceleration = [0.0; 3];
                        let mut turbulence = 0.0;
                        let forces = text(s, "force").split(',').collect::<Vec<_>>();
                        for f in root.iter().filter(|i| i.key.as_deref() == Some(b"force")) {
                            let f = f.value.items().unwrap_or(&[]);
                            if !forces.contains(&text(f, "name")) {
                                continue;
                            }
                            let amount = number(f, "amount", 0.0);
                            if text(f, "type") == "turbulence" {
                                turbulence += amount;
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
                            page,
                            additive: text(tex, "shader") == "ParticleAdditive",
                            billboard: text(s, "billboard") != "no",
                            radius: param("sphere_emitter_radius", 0.0),
                            position: ["x", "y", "z"].map(|k| number(block(s, "position"), k, 0.0)),
                            velocity: param("velocity", 0.0),
                            pitch: param("velocity_pitch", 0.0),
                            yaw: param("velocity_yaw", 0.0),
                            life: param("life", 1.0),
                            size: param("size", 1.0),
                            rotation: ["particle_yaw", "particle_pitch", "rotation"]
                                .map(|k| param(k, 0.0)),
                            spin: [
                                "rotation_speed_yaw",
                                "rotation_speed_pitch",
                                "rotation_speed",
                            ]
                            .map(|k| param(k, 0.0)),
                            color: ["x", "y", "z", "alpha"]
                                .map(|k| Parameter::read(col, k, 255.0, &curves)),
                            emission: number(s, "emission", 0.0),
                            max: number(s, "max_amount", 0.0) as usize,
                            acceleration,
                            turbulence,
                        });
                    }
                    out.systems.insert((kind, small, clouds), emitters);
                }
            }
        }
        Ok(Arc::new(out))
    }
}

/// Hourly flags. Weather is global simulation state; the engine's dispatch
/// has no country/intel predicate, so every recorded perspective uses it.
#[derive(Default)]
pub struct Field {
    hour: i32,
    kinds: BTreeMap<u32, usize>,
}
pub fn field(frame: &DecodedFrame) -> io::Result<Field> {
    let mut out = Field {
        hour: frame.hour,
        ..Default::default()
    };
    for r in super::tables::rows::<tables::WeatherRegions>(frame)?.rows() {
        if let Some(kind) = [r.light_rain, r.heavy_rain, r.snow, r.blizzard, r.sandstorm]
            .iter()
            .position(|v| *v != 0)
        {
            out.kinds.insert(r.id, kind);
        }
    }
    Ok(out)
}

/// Engine near/far fades and animation rate, independent of recorded intensity.
fn gates(h: f32) -> (f32, f32, f32) {
    let far = ((1500.0 - h) / 400.0).clamp(0.0, 1.0);
    (
        far,
        ((h - 358.0) / 220.0).clamp(0.0, 1.0) * far,
        0.15 + 0.85 * ((500.0 - h) / 200.0).clamp(0.0, 1.0),
    )
}
fn random(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
    let mut v = *seed;
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((v ^ (v >> 31)) >> 40) as f32 / 16777216.0
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Particle {
    position_size: [f32; 4],
    rotation_billboard: [f32; 4],
    color: [f32; 4],
    fog: [f32; 4],
}
struct Batch {
    page: usize,
    additive: bool,
    particles: Vec<Particle>,
}
fn batches(
    assets: &Assets,
    field: &Field,
    camera: Camera,
    size: [u32; 2],
    time: f32,
) -> Vec<Batch> {
    let (primary, cloud, _) = gates(camera.height);
    let mut groups: BTreeMap<(usize, bool), Vec<Particle>> = BTreeMap::new();
    if primary == 0.0 {
        return Vec::new();
    }
    for (index, a) in assets.anchors.iter().enumerate() {
        let Some(&kind) = field.kinds.get(&a.region) else {
            continue;
        };
        let mut anchor = a.position;
        anchor[0] += camera.wrap_offset(anchor, size);
        // Conservative world-space cull includes the emitter radius, flight and quad.
        let radius = 200.0 + camera.height * 1.2;
        if (anchor[0] - camera.center[0]).abs() > radius
            || (anchor[2] - camera.center[1]).abs() > radius
        {
            continue;
        }
        for (clouds, alpha) in [(false, primary), (true, cloud)] {
            if alpha == 0.0 {
                continue;
            }
            let Some(emitters) = assets.systems.get(&(kind, a.small && kind != 4, clouds)) else {
                continue;
            };
            for (ei, e) in emitters.iter().enumerate() {
                let mut seed = (field.hour as u64).wrapping_mul(0x100000001b3)
                    ^ ((a.region as u64) << 32)
                    ^ (index as u64 * 257 + ei as u64);
                let life_max = (e.life.base + e.life.spread).max(0.01);
                // Stationary reconstruction of an emitter at capacity. The
                // engine birth/death history is unrecorded; spread births over
                // the mean lifetime instead of continually replacing old ones.
                let emission = e.emission.min(e.max as f32 / e.life.base.max(0.01));
                let slots = (emission * life_max).ceil().max(0.0) as usize;
                let birth_phase = time * emission + random(&mut seed);
                let newest = birth_phase.floor() as i64;
                let base_seed = seed;
                let mut living = 0;
                for i in 0..slots {
                    let birth = newest - i as i64;
                    let age = (birth_phase - birth as f32) / emission.max(0.01);
                    let mut seed = base_seed ^ (birth as u64).wrapping_mul(0xd6e8feb86659fd93);
                    let life = e.life.sample(random(&mut seed), 0.0).max(0.01);
                    if age > life {
                        continue;
                    }
                    if living == e.max {
                        break;
                    }
                    living += 1;
                    let t = age / life;
                    let angle = random(&mut seed) * std::f32::consts::TAU;
                    let radius = e.radius.sample(random(&mut seed), 0.0);
                    let pitch = e.pitch.sample(random(&mut seed), 0.0).to_radians();
                    let yaw = e.yaw.sample(random(&mut seed), 0.0).to_radians();
                    let speed = e.velocity.sample(random(&mut seed), 0.0);
                    let velocity = [
                        -yaw.sin() * pitch.cos() * speed,
                        pitch.sin() * speed,
                        -yaw.cos() * pitch.cos() * speed,
                    ];
                    let mut p = [
                        anchor[0] + e.position[0] + angle.sin() * radius,
                        anchor[1] + e.position[1],
                        anchor[2] + e.position[2] + angle.cos() * radius,
                    ];
                    for j in 0..3 {
                        let phase = random(&mut seed) * std::f32::consts::TAU;
                        p[j] += velocity[j] * age
                            + 0.5 * e.acceleration[j] * age * age
                            + e.turbulence
                                * (phase.sin() - (phase + age).sin() + age * phase.cos());
                    }
                    let rotation = std::array::from_fn::<_, 3, _>(|j| {
                        (e.rotation[j].sample(random(&mut seed), t)
                            + e.spin[j].sample(random(&mut seed), t) * age)
                            .to_radians()
                    });
                    let mut color = e
                        .color
                        .each_ref()
                        .map(|p| (p.sample(random(&mut seed), t) / 255.0).clamp(0.0, 1.0));
                    for c in &mut color[..3] {
                        *c = c.powf(2.2);
                    }
                    color[3] *= alpha;
                    let sz = e.size.sample(random(&mut seed), t).max(0.0);
                    groups
                        .entry((e.page, e.additive))
                        .or_default()
                        .push(Particle {
                            position_size: [p[0], p[1], p[2], sz],
                            rotation_billboard: [
                                rotation[0],
                                rotation[1],
                                rotation[2],
                                f32::from(e.billboard),
                            ],
                            color,
                            fog: [f32::from(!e.additive), 0.0, 0.0, 0.0],
                        });
                }
            }
        }
    }
    groups
        .into_iter()
        .map(|((page, additive), mut particles)| {
            let forward = camera.look_direction();
            let depth = |p: &Particle| (0..3).map(|i| p.position_size[i] * forward[i]).sum::<f32>();
            particles.sort_by(|a, b| depth(b).total_cmp(&depth(a)));
            Batch {
                page,
                additive,
                particles,
            }
        })
        .collect()
}

/// Integrate the camera-dependent playback rate. Multiplying absolute time by
/// a new rate would jump every particle whenever the camera crossed a band.
#[derive(Default)]
struct Clock {
    previous: Option<f32>,
    elapsed: f32,
}
impl Clock {
    fn advance(&mut self, time: f32, height: f32) -> f32 {
        if time <= 0.0 {
            self.elapsed = 0.0;
            self.previous = None;
            return 0.0;
        }
        if let Some(previous) = self.previous {
            self.elapsed += (time - previous).max(0.0) * gates(height).2;
        }
        self.previous = Some(time);
        self.elapsed
    }
}

/// One shared pass for window and dump. Static textures stay resident; only
/// particles at the selected hour and visible camera are generated each frame.
pub struct Pass {
    assets: Arc<Assets>,
    field: Field,
    groups: Vec<wgpu::BindGroup>,
    pipelines: [wgpu::RenderPipeline; 2],
    buffers: BTreeMap<(usize, bool), (wgpu::Buffer, u64)>,
    clock: Clock,
}
impl Pass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        globals: &wgpu::BindGroupLayout,
        assets: Arc<Assets>,
    ) -> io::Result<Self> {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("weather textures"),
            entries: &[
                scene::texture_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                scene::sampler_entry(1),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });
        let mut groups = Vec::new();
        for page in &assets.pages {
            let bound = textures::upload(device, queue, "weather", page, ColorSpace::Srgb)?;
            groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("weather texture"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&bound.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            }));
        }
        let module = scene::module(
            device,
            "particle.shader",
            include_str!("weather_effects.wgsl"),
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("weather"),
            bind_group_layouts: &[Some(globals), Some(&layout)],
            immediate_size: 0,
        });
        let pipelines=[false,true].map(|additive|device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:Some("weather particles"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&module,entry_point:Some("particle_vertex"),compilation_options:Default::default(),buffers:&[Some(wgpu::VertexBufferLayout {array_stride:64,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4]})]},
            primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState {format:scene::DEPTH_FORMAT,depth_write_enabled:Some(false),depth_compare:Some(wgpu::CompareFunction::LessEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:wgpu::MultisampleState {count:scene::SAMPLE_COUNT,..Default::default()},
            fragment:Some(wgpu::FragmentState {module:&module,entry_point:Some("particle_fragment"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format:scene::SCENE_FORMAT,blend:Some(wgpu::BlendState {color:wgpu::BlendComponent {src_factor:wgpu::BlendFactor::SrcAlpha,dst_factor:if additive {wgpu::BlendFactor::One} else {wgpu::BlendFactor::OneMinusSrcAlpha},operation:wgpu::BlendOperation::Add},alpha:wgpu::BlendComponent::REPLACE}),write_mask:wgpu::ColorWrites::COLOR})]}),multiview_mask:None,cache:None,
        }));
        Ok(Self {
            assets,
            field: Field::default(),
            groups,
            pipelines,
            buffers: BTreeMap::new(),
            clock: Clock::default(),
        })
    }
    pub fn set(&mut self, field: Field) {
        self.field = field;
    }
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        globals: &wgpu::BindGroup,
        frame: (Camera, f32),
    ) -> io::Result<()> {
        let (camera, animation_time) = frame;
        let time = self.clock.advance(animation_time, camera.height);
        let batches = batches(&self.assets, &self.field, camera, targets.size(), time);
        if batches.is_empty() {
            return Ok(());
        }
        for b in &batches {
            let key = (b.page, b.additive);
            let data = bytemuck::cast_slice(&b.particles);
            let bytes = data.len() as u64;
            if self
                .buffers
                .get(&key)
                .is_none_or(|(_, capacity)| *capacity < bytes)
            {
                let capacity = bytes.max(64).next_power_of_two();
                if capacity > device.limits().max_buffer_size {
                    return Err(invalid("weather instance buffer exceeds device limit"));
                }
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("weather instances"),
                    size: capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.buffers.insert(key, (buffer, capacity));
            }
            queue.write_buffer(&self.buffers[&key].0, 0, data);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("weather particles"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, globals, &[]);
        for b in &batches {
            let buffer = &self.buffers[&(b.page, b.additive)].0;
            pass.set_pipeline(&self.pipelines[usize::from(b.additive)]);
            pass.set_bind_group(1, &self.groups[b.page], &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..b.particles.len() as u32);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_rain_remains_at_minimum_zoom_while_companion_is_hidden() {
        assert_eq!(gates(50.0), (1.0, 0.0, 1.0));
        assert_eq!(gates(358.0).1, 0.0);
        assert_eq!(gates(578.0), (1.0, 1.0, 0.15));
        assert_eq!(gates(1300.0), (0.5, 0.5, 0.15));
        assert_eq!(gates(1500.0), (0.0, 0.0, 0.15));
    }

    #[test]
    fn zoom_changes_playback_rate_without_teleporting_particle_ages() {
        let mut clock = Clock::default();
        assert_eq!(clock.advance(20.0, 50.0), 0.0);
        assert_eq!(clock.advance(21.0, 50.0), 1.0);
        assert_eq!(clock.advance(21.0, 600.0), 1.0);
        assert!((clock.advance(22.0, 600.0) - 1.15).abs() < 1e-6);
        assert_eq!(clock.advance(0.0, 50.0), 0.0);
    }

    #[test]
    fn scalar_and_block_curve_parameters_both_preserve_the_reference() {
        let doc = parser::parse(br#"alpha=60,fade size={ 6,fade 1 }"#).unwrap();
        let curves =
            BTreeMap::from([("fade".to_owned(), vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]])]);
        let alpha = Parameter::read(&doc.items, "alpha", 255.0, &curves);
        let size = Parameter::read(&doc.items, "size", 0.0, &curves);
        assert_eq!(alpha.sample(0.5, 0.5), 60.0);
        assert_eq!(alpha.sample(0.5, 1.0), 0.0);
        assert_eq!(size.sample(1.0, 0.5), 7.0);
        assert_eq!(size.sample(0.0, 0.5), 5.0);
    }
}
