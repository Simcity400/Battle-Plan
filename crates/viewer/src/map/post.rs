//! The `post` pass: the engine's colour grading over the resolved map.
//!
//! `restorescene.shader`'s `Restore` effect reads the scene, divides by the
//! adapted average luminance, tone maps, gamma encodes, then runs the colour
//! cube, the season HSV and colour balance, and `Levels`. The average luminance
//! comes from `downsample.shader` and `downsample_luminance.shader`. Which cube
//! and which `MiddleGrey` apply is decided per frame by `gfx/posteffect_volumes.txt`
//! against the camera, and the HSV and colour balance by `map/seasons.txt`
//! (R1 sections 4.2 and 4.3).
//!
//! # Whether the reference session ran this chain
//!
//! `settings.txt` carries `draw_postfx=no` and R1 section 4.1 read that as the
//! chain being off. The suspicion that it was on anyway is worth testing,
//! because the ungraded map is a bright saturated green where the game's land
//! is a desaturated blue-grey, and the chain's `Levels` and season HSV are the
//! only stages anywhere in the engine that desaturate the whole frame.
//!
//! Measured on the two reference pairs at 2048x1152 (`--dump`, scored with
//! `--compare --upscale 1.25`, the daylight pair also `--dim 0.5`):
//!
//! | chain | season band | daylight mae / dE / SSIM | night mae / dE / SSIM |
//! |---|---|---|---|
//! | off | — | **16.74 / 14.72 / 0.788** | **36.05 / 29.37 / 0.632** |
//! | on | centre | 47.93 / 25.34 / 0.608 | 71.59 / 35.30 / 0.512 |
//! | on | latitude fade | 47.20 / 22.93 / 0.620 | 78.38 / 37.76 / 0.493 |
//! | on | north | 46.69 / 22.19 / 0.627 | 82.44 / 40.05 / 0.484 |
//!
//! So the chain driven by the game's own numbers is far worse than no chain,
//! and the pass is off by default. What it gets right is the *shape*: with the
//! north or fade band the daylight land ratios stop being warm and turn
//! blue-dominant like the game's, which nothing else in the pipeline does.
//! What it gets wrong is the level, uniformly: every region comes out about
//! 2.5x too bright.
//!
//! The level is one number. `AverageLuminance` is the geometric mean of the
//! frame's luminance, 0.315 for the daylight frame and 0.154 for the night one,
//! both below `MIN_HDR_ADJUSTMENT`, so both clamp to 0.5 and the exposure is
//! `tonemap_middlegrey / 0.5` = 0.6 by day. Scaling that exposure by 0.25 — the
//! same thing as an `AverageLuminance` near 2.0, or as a scene four times
//! darker before the chain — gives, with the same code and no other change:
//!
//! | chain | season band | daylight mae / dE / SSIM | night mae / dE / SSIM |
//! |---|---|---|---|
//! | on, exposure x0.25 | latitude fade | 15.11 / 10.65 / 0.805 | 31.27 / 22.08 / 0.686 |
//! | on, exposure x0.25 | north | 14.95 / 10.68 / 0.807 | 33.41 / 25.08 / 0.677 |
//!
//! which beats the chain being off on every metric of both pairs, and brings
//! the daylight North-Germany ratios to (1.16, 0.94, 1.04) from (1.34, 1.25,
//! 0.71). That 0.25 has no support in any game file, so it is not in the code;
//! it is recorded because it localises what is left to the scene's absolute
//! level rather than to its colour. A known contributor sits upstream: the
//! terrain's `BORDER_LIGHT_REMOVAL_FACTOR` reads a bloom alpha of 0.600 over
//! the whole German interior instead of the 1.0 an area with no gradient field
//! should give, which leaks a third of the unlit albedo into the lit result and
//! brightens exactly the regions that are worst.

use std::{collections::BTreeMap, io};

use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    image::{Rgba8Image, tga},
    parser::{self, Value},
};

use super::{
    camera::{Camera, MAP_SIZE_X, MAP_SIZE_Y},
    frame::DayNight,
    scene::{SAMPLE_COUNT, Targets, invalid, sampler_entry, texture_entry},
};

/// `LEVELS` in `restorescene.shader`'s `LevelValue`.
const LEVELS: [f32; 2] = [0.04, 0.80];
/// `MIN_HDR_ADJUSTMENT` and `MAX_HDR_ADJUSTMENT` (`00_graphics.lua:1038-1039`).
const HDR_ADJUSTMENT: [f32; 2] = [0.5, 1.0];
/// `HDR_ADJUSTMENT_SPEED` (`00_graphics.lua:1040`), the tau of the luminance
/// adaptation the window runs between frames.
pub const HDR_ADJUSTMENT_SPEED: f32 = 15.0;
/// `tonemap_middlegrey` of `posteffect_values = { name = standard }`, the value
/// every other block inherits unless it sets its own.
const DEFAULT_MIDDLE_GREY: f32 = 0.55;
/// The luminance chain's format. One channel of log luminance, and the only
/// renderable float format every backend guarantees.
const LUMINANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// `SOUTH_POLE_OFFSET` and `NORTH_POLE_OFFSET` (`00_graphics.lua:936-937`) and
/// the `GMT_OFFSET` of `CalcGlobeNormal`, for the CPU copy of the day factor.
const SOUTH_POLE_OFFSET: f32 = 0.17;
const NORTH_POLE_OFFSET: f32 = 0.93;
const GMT_OFFSET: f32 = 2793.0;
const FEATHER: [f32; 2] = [-0.01, 0.01];

/// `TREE_SEASON_MIN` and `TREE_SEASON_FADE_TWEAK` (`constants.fxh:113-114`).
const TREE_SEASON_MIN: f32 = 0.5;
const TREE_SEASON_FADE_TWEAK: f32 = 2.5;

/// One resolved `posteffect_values` block.
#[derive(Clone, Debug, PartialEq)]
pub struct PostValues {
    /// Index into [`PostAssets::luts`].
    pub lut: usize,
    pub middle_grey: f32,
    pub hdr_min: f32,
    pub hdr_max: f32,
}

/// A `posteffect_height_volume`: the fallback chosen by camera height alone.
#[derive(Clone, Debug)]
struct HeightVolume {
    day: String,
    night: String,
    height: f32,
}

/// A `posteffect_volume`: an axis-aligned box in world units that overrides the
/// height volume while the camera is inside it.
#[derive(Clone, Debug)]
struct BoxVolume {
    day: String,
    night: String,
    position: [f32; 3],
    size: [f32; 3],
    fade_distance: f32,
}

/// One season's north, centre and south grading from `map/seasons.txt`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeasonGrade {
    /// `hsv_*`: hue shift in sixths, saturation multiplier, value multiplier.
    pub hsv: [f32; 3],
    /// `colorbalance_*`.
    pub color_balance: [f32; 3],
}

impl Default for SeasonGrade {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

impl SeasonGrade {
    const NEUTRAL: Self = Self {
        hsv: [0.0, 1.0, 1.0],
        color_balance: [1.0, 1.0, 1.0],
    };

    fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: [f32; 3], b: [f32; 3]| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        Self {
            hsv: mix(self.hsv, other.hsv),
            color_balance: mix(self.color_balance, other.color_balance),
        }
    }
}

/// How much of the season's `*_north` band a frame takes, from its camera.
///
/// `HSV` and `ColorBalance` are single values in `ConstantBuffer(1, 32)`, so the
/// engine picks one blend of `*_north`, `*_center` and `*_south` per frame
/// rather than per pixel, and the camera's latitude is the only input it can
/// have. Where the bands meet is **UNVERIFIED**: no decompiled function
/// references the season keys and no define names a boundary. This reuses the
/// one latitude fade the shaders do spell out, `saturate((z / MAP_SIZE_Y -
/// TREE_SEASON_MIN) * TREE_SEASON_FADE_TWEAK)` from `tree.shader:347`, which
/// asks the same question about the same map and puts the reference camera at
/// 0.664 of the way north.
///
/// The two degenerate readings were measured against both pairs as well. With
/// the chain's own exposure all three are far worse than no chain at all
/// (daylight mae 47.93 for `*_center` throughout, 47.20 for this fade, 46.69
/// for `*_north` throughout, against 16.74 with the chain off). With the
/// exposure corrected as the module comment describes, this fade scores 15.11
/// and 31.27 against `*_north`'s 14.95 and 33.41, so it is also the best of the
/// three overall and not only the best supported.
fn season_band(center: [f32; 2]) -> f32 {
    ((center[1] / MAP_SIZE_Y - TREE_SEASON_MIN) * TREE_SEASON_FADE_TWEAK).clamp(0.0, 1.0)
}

/// The volume and season tables, without the LUT images.
///
/// Small enough that the pass keeps its own copy and the decoded LUTs are
/// dropped once they are on the GPU.
#[derive(Clone, Debug, Default)]
pub struct Volumes {
    values: BTreeMap<String, PostValues>,
    heights: Vec<HeightVolume>,
    boxes: Vec<BoxVolume>,
    north: SeasonGrade,
    center: SeasonGrade,
}

/// Everything the post pass reads from installed content.
pub struct PostAssets {
    /// Every `lut` the volume file names, in first-seen order.
    pub luts: Vec<Rgba8Image>,
    /// The tables that decide which of those LUTs a frame uses.
    pub volumes: Volumes,
}

/// The uniform the grading shader reads, resolved for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grading {
    pub day: PostValuesResolved,
    pub night: PostValuesResolved,
    pub night_blend: f32,
    pub season: SeasonGrade,
}

/// The subset of [`PostValues`] that survives volume blending as numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostValuesResolved {
    pub lut: usize,
    pub middle_grey: f32,
    pub hdr_min: f32,
    pub hdr_max: f32,
}

impl PostAssets {
    /// Reads `gfx/posteffect_volumes.txt`, `map/seasons.txt` and every LUT.
    ///
    /// `gfx/` is outside the coverage policy, so the volume file and the LUTs go
    /// through [`ResolvedLayers::read_uncovered`] like the fonts do; `map/` is
    /// covered, so `map/seasons.txt` is hash-verified.
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let document = parser::parse(&layers.read_uncovered("gfx/posteffect_volumes.txt")?)
            .map_err(|error| invalid(format!("gfx/posteffect_volumes.txt: {error}")))?;

        let mut lut_paths: Vec<String> = Vec::new();
        let mut raw: BTreeMap<String, RawValues> = BTreeMap::new();
        for value in document.get_all("posteffect_values") {
            let Some(items) = value.items() else { continue };
            let block = parser::Document {
                items: items.to_vec(),
            };
            let Some(name) = scalar(&block, "name") else {
                continue;
            };
            raw.insert(
                name,
                RawValues {
                    inherit: scalar(&block, "inherit"),
                    lut: scalar(&block, "lut"),
                    middle_grey: number(&block, "tonemap_middlegrey"),
                    hdr_min: number(&block, "hdr_min_adjustment"),
                    hdr_max: number(&block, "hdr_max_adjustment"),
                },
            );
        }

        let mut values = BTreeMap::new();
        for name in raw.keys() {
            let resolved = resolve(&raw, name, &mut lut_paths);
            values.insert(name.clone(), resolved);
        }

        let mut heights = Vec::new();
        let mut boxes = Vec::new();
        for outer in document.get_all("posteffect_volumes") {
            let Some(items) = outer.items() else { continue };
            let block = parser::Document {
                items: items.to_vec(),
            };
            for value in block.get_all("posteffect_height_volume") {
                let Some(inner) = as_document(value) else {
                    continue;
                };
                let (Some(day), Some(night)) = (
                    scalar(&inner, "posteffect_values_day"),
                    scalar(&inner, "posteffect_values_night"),
                ) else {
                    continue;
                };
                heights.push(HeightVolume {
                    day,
                    night,
                    height: number(&inner, "height").unwrap_or(0.0),
                });
            }
            for value in block.get_all("posteffect_volume") {
                let Some(inner) = as_document(value) else {
                    continue;
                };
                let (Some(day), Some(night), Some(position), Some(size)) = (
                    scalar(&inner, "posteffect_values_day"),
                    scalar(&inner, "posteffect_values_night"),
                    triple(&inner, "position"),
                    triple(&inner, "size"),
                ) else {
                    continue;
                };
                boxes.push(BoxVolume {
                    day,
                    night,
                    position,
                    size,
                    fade_distance: number(&inner, "fade_distance").unwrap_or(0.0),
                });
            }
        }
        // Deepest first, so the innermost override wins the search below.
        heights.sort_by(|a, b| b.height.total_cmp(&a.height));

        let mut luts = Vec::with_capacity(lut_paths.len());
        for path in &lut_paths {
            let bytes = layers.read_uncovered(path)?;
            let image = tga::decode(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
            luts.push(image);
        }
        if luts.is_empty() {
            return Err(invalid(
                "gfx/posteffect_volumes.txt names no colour-correction LUT",
            ));
        }

        let seasons = parser::parse(&layers.read("map/seasons.txt")?)
            .map_err(|error| invalid(format!("map/seasons.txt: {error}")))?;
        let (north, center) = read_seasons(&seasons);

        Ok(Self {
            luts,
            volumes: Volumes {
                values,
                heights,
                boxes,
                north,
                center,
            },
        })
    }
}

impl Volumes {
    /// The grading for one camera and hour.
    ///
    /// The height volume whose `height` the camera clears is the base; a box
    /// volume containing the camera replaces it, faded in over `fade_distance`
    /// from the box's faces. Overlapping boxes resolve to the last in file
    /// order, which is the only rule the file itself suggests and is
    /// **UNVERIFIED**; the reference camera sits 188 world units inside
    /// `EuropeandRussia`'s nearest face against a `fade_distance` of 125, so it
    /// takes that volume outright and the rule does not bite there.
    pub fn select(&self, camera: Camera, day_night: DayNight) -> Grading {
        let base = self
            .heights
            .iter()
            .find(|volume| camera.height >= volume.height)
            .or_else(|| self.heights.last());
        let (mut day, mut night) = match base {
            Some(volume) => (self.value(&volume.day), self.value(&volume.night)),
            None => (self.fallback(), self.fallback()),
        };

        let eye = [camera.center[0], camera.height, camera.center[1]];
        let inside = self.boxes.iter().rev().find_map(|volume| {
            let fraction = box_fade(eye, volume);
            (fraction > 0.0).then_some((volume, fraction))
        });
        if let Some((volume, fraction)) = inside {
            day = day.lerp(self.value(&volume.day), fraction);
            night = night.lerp(self.value(&volume.night), fraction);
        }

        Grading {
            day,
            night,
            night_blend: night_factor(camera.center, day_night),
            season: self.center.lerp(self.north, season_band(camera.center)),
        }
    }

    fn value(&self, name: &str) -> PostValuesResolved {
        self.values
            .get(name)
            .map(|values| PostValuesResolved {
                lut: values.lut,
                middle_grey: values.middle_grey,
                hdr_min: values.hdr_min,
                hdr_max: values.hdr_max,
            })
            .unwrap_or_else(|| self.fallback())
    }

    fn fallback(&self) -> PostValuesResolved {
        PostValuesResolved {
            lut: 0,
            middle_grey: DEFAULT_MIDDLE_GREY,
            hdr_min: HDR_ADJUSTMENT[0],
            hdr_max: HDR_ADJUSTMENT[1],
        }
    }
}

impl PostValuesResolved {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            // The LUT is an index into an array of cubes, so it switches rather
            // than blends; the shader lerps the two samples instead.
            lut: if t >= 0.5 { other.lut } else { self.lut },
            middle_grey: self.middle_grey + (other.middle_grey - self.middle_grey) * t,
            hdr_min: self.hdr_min + (other.hdr_min - self.hdr_min) * t,
            hdr_max: self.hdr_max + (other.hdr_max - self.hdr_max) * t,
        }
    }
}

struct RawValues {
    inherit: Option<String>,
    lut: Option<String>,
    middle_grey: Option<f32>,
    hdr_min: Option<f32>,
    hdr_max: Option<f32>,
}

/// Folds `inherit` chains, registering each distinct LUT path once.
fn resolve(raw: &BTreeMap<String, RawValues>, name: &str, paths: &mut Vec<String>) -> PostValues {
    let mut chain = Vec::new();
    let mut cursor = Some(name.to_string());
    // `inherit` is authored one level deep; the bound stops a cycle regardless.
    while let Some(current) = cursor.take() {
        let Some(entry) = raw.get(&current) else {
            break;
        };
        chain.push(entry);
        if chain.len() >= 8 {
            break;
        }
        cursor = entry.inherit.clone();
    }
    let pick = |get: &dyn Fn(&RawValues) -> Option<f32>, default: f32| {
        chain.iter().find_map(|entry| get(entry)).unwrap_or(default)
    };
    let path = chain
        .iter()
        .find_map(|entry| entry.lut.clone())
        .unwrap_or_default();
    let lut = paths
        .iter()
        .position(|known| *known == path)
        .unwrap_or_else(|| {
            paths.push(path);
            paths.len() - 1
        });
    PostValues {
        lut,
        middle_grey: pick(&|entry| entry.middle_grey, DEFAULT_MIDDLE_GREY),
        hdr_min: pick(&|entry| entry.hdr_min, HDR_ADJUSTMENT[0]),
        hdr_max: pick(&|entry| entry.hdr_max, HDR_ADJUSTMENT[1]),
    }
}

/// How far inside a box volume the camera is, as 0 outside to 1 beyond the fade.
fn box_fade(eye: [f32; 3], volume: &BoxVolume) -> f32 {
    let mut inside = f32::INFINITY;
    for ((at, position), size) in eye.iter().zip(&volume.position).zip(&volume.size) {
        inside = inside.min(size * 0.5 - (at - position).abs());
    }
    if inside <= 0.0 {
        return 0.0;
    }
    if volume.fade_distance <= 0.0 {
        return 1.0;
    }
    (inside / volume.fade_distance).clamp(0.0, 1.0)
}

/// `DayNightFactor(CalcGlobeNormal(xz))` on the CPU, for the camera's target.
fn night_factor(center: [f32; 2], day_night: DayNight) -> f32 {
    let direction = day_night.hour_sun_dir();
    let mut x = (center[0] - GMT_OFFSET) / MAP_SIZE_X + direction[0];
    x -= x.floor();
    let mut y = center[1] / MAP_SIZE_Y;
    y = SOUTH_POLE_OFFSET + (NORTH_POLE_OFFSET - SOUTH_POLE_OFFSET) * y;
    y = -(y * std::f32::consts::PI).cos();
    let planar = 1.0 - y.abs();
    let normal = [
        (x * std::f32::consts::TAU).sin() * planar,
        y,
        (x * std::f32::consts::TAU).cos() * planar,
    ];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length == 0.0 {
        return 0.0;
    }
    let value = (normal[0] * 0.0 + normal[1] * direction[2] + normal[2] * direction[3]) / length;
    ((value - FEATHER[0]) / (FEATHER[1] - FEATHER[0])).clamp(0.0, 1.0) * day_night.strength
}

fn as_document(value: &Value) -> Option<parser::Document> {
    Some(parser::Document {
        items: value.items()?.to_vec(),
    })
}

fn scalar(document: &parser::Document, key: &str) -> Option<String> {
    document.get(key).and_then(Value::scalar_str).map(unquote)
}

fn number(document: &parser::Document, key: &str) -> Option<f32> {
    document.get(key)?.scalar_str()?.trim().parse().ok()
}

fn triple(document: &parser::Document, key: &str) -> Option<[f32; 3]> {
    let items = document.get(key)?.items()?;
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.value.scalar_str()?.trim().parse().ok()?;
    }
    (items.len() >= 3).then_some(out)
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

/// The north and centre grading of `map/seasons.txt`.
///
/// Every shipped season block carries the same three bands, so which season the
/// date selects cannot change the result on this content; the reader takes the
/// first block that defines the keys and the caller states that limitation.
fn read_seasons(document: &parser::Document) -> (SeasonGrade, SeasonGrade) {
    let mut north = SeasonGrade::NEUTRAL;
    let mut center = SeasonGrade::NEUTRAL;
    for name in ["summer", "spring", "autumn", "winter"] {
        let Some(block) = document.get(name).and_then(as_document) else {
            continue;
        };
        let (Some(hsv_north), Some(balance_north)) = (
            triple(&block, "hsv_north"),
            triple(&block, "colorbalance_north"),
        ) else {
            continue;
        };
        north = SeasonGrade {
            hsv: hsv_north,
            color_balance: balance_north,
        };
        center = SeasonGrade {
            hsv: triple(&block, "hsv_center").unwrap_or(SeasonGrade::NEUTRAL.hsv),
            color_balance: triple(&block, "colorbalance_center")
                .unwrap_or(SeasonGrade::NEUTRAL.color_balance),
        };
        break;
    }
    (north, center)
}

/// The luminance mip chain and the two one-texel adaptation targets, for one
/// render size.
struct Chain {
    size: [u32; 2],
    levels: Vec<wgpu::TextureView>,
    adapted: [wgpu::TextureView; 2],
    seed: wgpu::BindGroup,
    reduce: Vec<wgpu::BindGroup>,
    adapt: [wgpu::BindGroup; 2],
    grade: [wgpu::BindGroup; 2],
}

/// The three pipelines of the post stage and the resources they bind.
pub struct PostPass {
    seed: wgpu::RenderPipeline,
    reduce: wgpu::RenderPipeline,
    adapt: wgpu::RenderPipeline,
    grade: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    cube: wgpu::TextureView,
    uniform: wgpu::Buffer,
    volumes: Volumes,
    chain: Option<Chain>,
    parity: usize,
}

/// The uniform block `post.wgsl` reads.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    hsv: [f32; 4],
    color_balance: [f32; 4],
    grade: [f32; 4],
    luts: [f32; 4],
    misc: [f32; 4],
}

impl PostPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &PostAssets,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let volumes = assets.volumes.clone();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post inputs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(
                    1,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                texture_entry(
                    2,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                texture_entry(
                    3,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                texture_entry(
                    4,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2Array,
                ),
                sampler_entry(5),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let stage = |label: &str, entry: &'static str, target, samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let width = assets.luts[0].width;
        let height = assets.luts[0].height;
        if assets
            .luts
            .iter()
            .any(|lut| lut.width != width || lut.height != height)
        {
            return Err(invalid(
                "the colour-correction LUTs are not all the same size",
            ));
        }
        let cube = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ColorCube"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: assets.luts.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, lut) in assets.luts.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &cube,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &lut.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        Ok(Self {
            seed: stage("post luminance seed", "seed_main", LUMINANCE_FORMAT, 1),
            reduce: stage("post luminance reduce", "reduce_main", LUMINANCE_FORMAT, 1),
            adapt: stage("post luminance adapt", "adapt_main", LUMINANCE_FORMAT, 1),
            grade: stage("post grade", "grade_main", format, SAMPLE_COUNT),
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post linear clamp"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            cube: cube.create_view(&wgpu::TextureViewDescriptor {
                label: Some("ColorCube"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post uniform"),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            volumes,
            chain: None,
            parity: 0,
        })
    }

    /// The grading this camera and hour select.
    pub fn grading(&self, camera: Camera, day_night: DayNight) -> Grading {
        self.volumes.select(camera, day_night)
    }

    /// Rebuilds the luminance chain and every bind group for a target size.
    fn rebuild(&mut self, device: &wgpu::Device, targets: &Targets) {
        let size = targets.size();
        let half = [(size[0] / 2).max(1), (size[1] / 2).max(1)];
        let levels = 32 - half[0].max(half[1]).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post luminance"),
            size: wgpu::Extent3d {
                width: half[0],
                height: half[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LUMINANCE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let level_view = |level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("post luminance level"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let views: Vec<_> = (0..levels).map(level_view).collect();
        let one_texel = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: LUMINANCE_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let adapted = [one_texel("post adapted 0"), one_texel("post adapted 1")];

        let bind = |label: &str,
                    scene: &wgpu::TextureView,
                    lum: &wgpu::TextureView,
                    prev: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(scene),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(lum),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(prev),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&self.cube),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        let scene = targets.output_view();
        // The seed writes level 0 and reads only the scene, so its luminance
        // slot has to point somewhere else: a render pass may not bind the
        // subresource it is drawing into.
        let seed = bind("post seed", scene, &adapted[0], &adapted[0]);
        let reduce = (1..levels as usize)
            .map(|level| bind("post reduce", scene, &views[level - 1], &adapted[0]))
            .collect();
        let last = &views[levels as usize - 1];
        let adapt = [
            bind("post adapt 0", scene, last, &adapted[1]),
            bind("post adapt 1", scene, last, &adapted[0]),
        ];
        let grade = [
            bind("post grade 0", scene, last, &adapted[0]),
            bind("post grade 1", scene, last, &adapted[1]),
        ];
        self.chain = Some(Chain {
            size,
            levels: views,
            adapted,
            seed,
            reduce,
            adapt,
            grade,
        });
    }

    /// Runs the whole stage: the luminance reduction, then the grading over the
    /// resolved map colour and back into the multisampled attachment.
    ///
    /// `blend` is `1 - exp(-HDR_ADJUSTMENT_SPEED * dt)`; a headless dump passes
    /// 1 so the frame stands alone and two identical renders agree.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        grading: Grading,
        blend: f32,
        generation_changed: bool,
    ) {
        if generation_changed || self.chain.as_ref().is_none_or(|c| c.size != targets.size()) {
            self.rebuild(device, targets);
            // A fresh chain has no history, so the first frame takes the
            // instantaneous value whatever the caller asked for.
            self.parity = 0;
        }
        let Some(chain) = &self.chain else { return };
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Uniform {
                hsv: [
                    grading.season.hsv[0],
                    grading.season.hsv[1],
                    grading.season.hsv[2],
                    0.0,
                ],
                color_balance: [
                    grading.season.color_balance[0],
                    grading.season.color_balance[1],
                    grading.season.color_balance[2],
                    0.0,
                ],
                grade: [
                    lerp(
                        grading.day.middle_grey,
                        grading.night.middle_grey,
                        grading.night_blend,
                    ),
                    lerp(
                        grading.day.hdr_min,
                        grading.night.hdr_min,
                        grading.night_blend,
                    ),
                    lerp(
                        grading.day.hdr_max,
                        grading.night.hdr_max,
                        grading.night_blend,
                    ),
                    grading.night_blend,
                ],
                luts: [
                    grading.day.lut as f32,
                    grading.night.lut as f32,
                    LEVELS[0],
                    LEVELS[1],
                ],
                misc: [blend.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
            }),
        );

        let pass = |label: &str,
                    encoder: &mut wgpu::CommandEncoder,
                    pipeline: &wgpu::RenderPipeline,
                    view: &wgpu::TextureView,
                    bind_group: &wgpu::BindGroup| {
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            render.set_pipeline(pipeline);
            render.set_bind_group(0, bind_group, &[]);
            render.draw(0..3, 0..1);
        };

        pass(
            "post luminance seed",
            encoder,
            &self.seed,
            &chain.levels[0],
            &chain.seed,
        );
        for (index, bind_group) in chain.reduce.iter().enumerate() {
            pass(
                "post luminance reduce",
                encoder,
                &self.reduce,
                &chain.levels[index + 1],
                bind_group,
            );
        }
        let next = 1 - self.parity;
        pass(
            "post luminance adapt",
            encoder,
            &self.adapt,
            &chain.adapted[next],
            &chain.adapt[next],
        );

        // The grading writes every sample of the multisampled attachment and
        // does not resolve: the overlays still have to draw over it, and the
        // resolve at the end of the frame is theirs.
        let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post grade"),
            color_attachments: &[Some(
                targets.color_attachment_without_resolve(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
            )],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render.set_pipeline(&self.grade);
        render.set_bind_group(0, &chain.grade[next], &[]);
        render.draw(0..3, 0..1);
        drop(render);
        self.parity = next;
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume() -> BoxVolume {
        BoxVolume {
            day: "green".into(),
            night: "green_night".into(),
            position: [3300.0, 275.0, 1700.0],
            size: [1300.0, 800.0, 700.0],
            fade_distance: 125.0,
        }
    }

    #[test]
    fn the_reference_camera_is_fully_inside_europe_and_russia() {
        // 1538 is 188 inside the z face at 1350, which clears the 125 fade.
        assert_eq!(box_fade([3010.0, 310.178, 1538.0], &volume()), 1.0);
    }

    #[test]
    fn a_camera_outside_a_volume_and_one_within_its_fade_are_distinguished() {
        assert_eq!(box_fade([4000.0, 310.0, 1538.0], &volume()), 0.0);
        let edge = box_fade([3010.0, 310.0, 1400.0], &volume());
        assert!((edge - 0.4).abs() < 1e-5, "{edge}");
    }

    #[test]
    fn inherit_folds_the_standard_block_into_the_green_ones() {
        let text = br#"
            posteffect_values = { name = standard lut = "a.tga" tonemap_middlegrey = 0.55 }
            posteffect_values = { name = green inherit = standard lut = "b.tga" tonemap_middlegrey = 0.3 }
            posteffect_values = { name = green_night inherit = standard lut = "c.tga"
                tonemap_middlegrey = 0.7 hdr_min_adjustment = 0.8 hdr_max_adjustment = 0.6 }
            posteffect_values = { name = plain inherit = standard }
        "#;
        let document = parser::parse(text).unwrap();
        let mut raw = BTreeMap::new();
        for value in document.get_all("posteffect_values") {
            let block = parser::Document {
                items: value.items().unwrap().to_vec(),
            };
            raw.insert(
                scalar(&block, "name").unwrap(),
                RawValues {
                    inherit: scalar(&block, "inherit"),
                    lut: scalar(&block, "lut"),
                    middle_grey: number(&block, "tonemap_middlegrey"),
                    hdr_min: number(&block, "hdr_min_adjustment"),
                    hdr_max: number(&block, "hdr_max_adjustment"),
                },
            );
        }
        let mut paths = Vec::new();
        let green = resolve(&raw, "green", &mut paths);
        assert_eq!(green.middle_grey, 0.3);
        assert_eq!(green.hdr_min, 0.5);
        assert_eq!(green.hdr_max, 1.0);
        let night = resolve(&raw, "green_night", &mut paths);
        assert_eq!(night.middle_grey, 0.7);
        assert_eq!((night.hdr_min, night.hdr_max), (0.8, 0.6));
        let plain = resolve(&raw, "plain", &mut paths);
        assert_eq!(plain.middle_grey, 0.55);
        assert_eq!(paths, vec!["b.tga", "c.tga", "a.tga"]);
        assert_eq!(plain.lut, 2);
    }

    #[test]
    fn the_shipped_season_bands_desaturate_the_north_and_leave_the_centre_alone() {
        let text = br#"
            summer = {
                start_date=00.05.20
                hsv_north = { 0 0.1 1 }
                colorbalance_north = { 0.9 0.9 1 }
                hsv_center = { 0.0 1.0 1.0 }
                colorbalance_center = { 1.0 1.0 1.0 }
            }
        "#;
        let (north, center) = read_seasons(&parser::parse(text).unwrap());
        assert_eq!(north.hsv, [0.0, 0.1, 1.0]);
        assert_eq!(north.color_balance, [0.9, 0.9, 1.0]);
        assert_eq!(center, SeasonGrade::NEUTRAL);
        let half = center.lerp(north, 0.5);
        assert_eq!(half.hsv, [0.0, 0.55, 1.0]);
        assert_eq!(half.color_balance, [0.95, 0.95, 1.0]);
    }

    #[test]
    fn the_reference_hours_land_on_day_and_night_at_the_camera_target() {
        // 14:00 over Berlin is full day; the 03:00 hour twelve later is night.
        let center = [3010.0, 1568.0];
        assert_eq!(night_factor(center, DayNight::at_hour(60832933)), 0.0);
        assert_eq!(night_factor(center, DayNight::at_hour(60832933 + 12)), 1.0);
    }

    #[test]
    fn a_missing_lut_path_still_yields_one_slot() {
        let document = parser::parse(b"posteffect_values = { name = bare }").unwrap();
        let block = parser::Document {
            items: document
                .get_all("posteffect_values")
                .next()
                .unwrap()
                .items()
                .unwrap()
                .to_vec(),
        };
        let mut raw = BTreeMap::new();
        raw.insert(
            scalar(&block, "name").unwrap(),
            RawValues {
                inherit: None,
                lut: None,
                middle_grey: None,
                hdr_min: None,
                hdr_max: None,
            },
        );
        let mut paths = Vec::new();
        assert_eq!(resolve(&raw, "bare", &mut paths).lut, 0);
        assert_eq!(paths, vec![String::new()]);
    }
}
