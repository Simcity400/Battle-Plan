//! Native post-processing over the resolved HDR scene.
//!
//! Downsample: a half-size seed holding the bright pass (plus scene alpha, the
//! emissive mask, times `EMISSIVE_BLOOM_STRENGTH`) in RGB and log luminance in
//! alpha. `draw_map_full_res=no` first halves the HDR scene with truncating
//! integer division; this seed is half that scene again.
//!
//! Luminance: ping-pong reductions of the seed alpha in two fixed 6F targets
//! with shrinking ceil-half viewports, then one
//! gather over the last active rectangle and the adaptation. Odd sizes sample
//! pixels earlier passes left outside their viewport, so a mip chain is not
//! equivalent.
//!
//! Bloom blurs the seed horizontally into a second target and vertically back, where the grading
//! in force gives its taps a weight; with every weight zero the blurs would write zeros, so they
//! are not drawn and the restore adds a zeroed texture. It remains enabled independently of the
//! `draw_bloom` display option.
//!
//! Restore adds the blurred seed, exposure, tone mapping, gamma, colour cube and level adjustment.
//! The compiled pass samples the scene linearly into the full-size output before nonlinear
//! grading; GUI overlays retain that size. HSV and colour balance use neutral renderer constants
//! independently of date-driven seasonal grading.
//!
//! Grading selects the highest faded-in height volume and the first containing box. Day/night,
//! neighbouring heights, winter and edges blend in LUT space, storing UNORM8 after each blend. Winter
//! uses one province per strategic region and raw hourly snow, separately from display snow/mud.
//!
//! Every tunable comes from the recorded content: `gfx/posteffect_volumes.txt`,
//! its LUTs and `NDefines.NGraphics`. Missing or malformed content refuses the
//! pass.

use shader_pipeline::gpu::stats::CountedPasses as _;
use std::sync::Arc;
use std::{collections::BTreeMap, io};
use viewer_foundation::map::textures::load_image;

use eframe::wgpu;
use installed_content_core::{ResolvedLayers, defines::Defines, map::MapData, parser, parser::Value};
use installed_content_image::{SharedRgba8Image as Rgba8Image, native::Decode};
use installed_content_regions::regions::StrategicRegion;

use viewer_foundation::map::{
    camera::{Camera, MapSize},
    frame::DayNight,
    gpu_contract::{Targets, invalid, sampler_entry, texture_entry},
};

#[path = "post_luts.rs"]
mod luts;

#[path = "post_lut_shader.rs"]
mod compiled_luts;

#[path = "post_bloom.rs"]
mod compiled_bloom;

#[path = "post_fullscreen.rs"]
mod fullscreen;

#[path = "post_reduce.rs"]
mod compiled_reduce;

#[path = "post_gather.rs"]
mod compiled_gather;

#[path = "post_restore.rs"]
mod compiled_restore;

#[path = "post_seed.rs"]
mod compiled_seed;

/// Reduction and adaptation targets store single-channel half-float luminance.
const LUMINANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// The downsample and bloom targets: bright-pass RGB with log luminance in alpha.
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub use map_snow::winter::SnowFactors;
use map_snow::winter::{
    BAND_EPSILON, BoxVolume, HeightVolume, SnowDefines, Winter, province_extents, saturate,
    snow_records,
};
#[cfg(test)]
use map_snow::winter::{ProvinceExtent, SnowRecord, band};

/// The `NDefines.NGraphics` operands of this pass.
#[derive(Clone, Copy, Debug)]
struct PostDefines {
    hdr_adjustment: [f32; 2],
    adaptation_speed: f32,
    middle_grey: f32,
    luminance_white_squared: f32,
    /// `BLOOM_WIDTH`, `BLOOM_SCALE`, `BRIGHT_THRESHOLD`: the defaults of the
    /// per-volume bloom values.
    bloom: [f32; 3],
    /// `EMISSIVE_BLOOM_STRENGTH`: how much scene alpha (emissive) seeds bloom.
    emissive_strength: f32,
    snow_per_province: [f32; 2],
    snow_total: [f32; 2],
    poles: [f32; 2],
    gmt_offset: f32,
    feather_distance: [f32; 2],
    feather_amount: [f32; 2],
}

impl PostDefines {
    fn load(defines: &Defines) -> io::Result<Self> {
        let number = |key| -> io::Result<f32> {
            let value = defines.number("NGraphics", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.NGraphics.{key} is outside f32")))
            }
        };
        // Read the poles through the fixed-point getter.
        let fixed =
            |key| -> io::Result<f32> { Ok(defines.fixed5("NGraphics", key)? as f32 / 100_000.0) };
        // Pass the product of the f32 define with itself.
        let luminance_white = number("TONE_MAP_LUMINANCE_WHITE")?;
        Ok(Self {
            hdr_adjustment: [number("MIN_HDR_ADJUSTMENT")?, number("MAX_HDR_ADJUSTMENT")?],
            adaptation_speed: number("HDR_ADJUSTMENT_SPEED")?,
            middle_grey: number("TONE_MAP_MIDDLE_GREY")?,
            luminance_white_squared: luminance_white * luminance_white,
            bloom: [
                number("BLOOM_WIDTH")?,
                number("BLOOM_SCALE")?,
                number("BRIGHT_THRESHOLD")?,
            ],
            emissive_strength: number("EMISSIVE_BLOOM_STRENGTH")?,
            snow_per_province: [
                number("POSTEFFECT_PER_PROVINCE_MIN_SNOW")?,
                number("POSTEFFECT_PER_PROVINCE_MAX_SNOW")?,
            ],
            snow_total: [
                number("POSTEFFECT_TOTAL_MIN_SNOW")?,
                number("POSTEFFECT_TOTAL_MAX_SNOW")?,
            ],
            poles: [fixed("SOUTH_POLE_OFFSET")?, fixed("NORTH_POLE_OFFSET")?],
            // An integer define.
            gmt_offset: number("GMT_OFFSET")?.trunc(),
            feather_distance: [
                number("POSTEFFECT_FEATHER_MIN_DISTANCE")?,
                number("POSTEFFECT_FEATHER_MAX_DISTANCE")?,
            ],
            feather_amount: [
                number("POSTEFFECT_FEATHER_AT_MIN")?,
                number("POSTEFFECT_FEATHER_AT_MAX")?,
            ],
        })
    }
}

/// Synthetic operands for the law tests.
#[cfg(test)]
impl Default for PostDefines {
    fn default() -> Self {
        Self {
            hdr_adjustment: [0.5, 1.0],
            adaptation_speed: 15.0,
            middle_grey: 0.5,
            luminance_white_squared: 1.0,
            bloom: [1.5, 0.9, 0.4],
            emissive_strength: 1.0,
            snow_per_province: [0.1, 0.2],
            snow_total: [0.0, 0.05],
            poles: [0.17, 0.93],
            gmt_offset: 2793.0,
            feather_distance: [300.0, 2000.0],
            feather_amount: [0.03, 0.80],
        }
    }
}

/// One resolved `posteffect_values` block.
#[derive(Clone, Debug, PartialEq)]
struct PostValues {
    /// Index into [`PostAssets::luts`].
    lut: usize,
    middle_grey: f32,
    hdr_min: f32,
    hdr_max: f32,
    /// `BLOOM_WIDTH`, `BLOOM_SCALE`, `BRIGHT_THRESHOLD`.
    bloom: [f32; 3],
}

/// The volume tables, without the LUT images, which are dropped once uploaded.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct Volumes {
    defines: PostDefines,
    /// The built-in identity cube in [`PostAssets::luts`].
    identity: usize,
    values: BTreeMap<String, PostValues>,
    /// Volumes stored in ascending height order.
    heights: Vec<HeightVolume>,
    boxes: Vec<BoxVolume>,
    /// One winter factor per entry of `boxes`; `None` when a representative's
    /// snow is not recorded.
    snow: SnowFactors,
}

/// Everything the post pass reads from installed content.
pub struct PostAssets {
    lut_shader: Arc<shader_pipeline::library::Effect>,
    bloom: Arc<shader_pipeline::library::Effect>,
    reductions: compiled_reduce::Assets,
    seed: Arc<shader_pipeline::library::Effect>,
    gather: compiled_gather::Assets,
    restore: Arc<shader_pipeline::library::Effect>,
    /// Every `lut` the volume file names in first-seen order, and the identity.
    pub luts: Vec<Rgba8Image>,
    /// The tables that decide which of those LUTs a frame uses.
    pub volumes: Volumes,
}

/// A source LUT or an earlier LUT-blend target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LutSource {
    Asset(usize),
    Intermediate(usize),
}

/// One `lut_blender` draw: `mix(mix(s0, s1, f0), mix(s2, s3, f0), f1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LutOperation {
    sources: [LutSource; 4],
    factors: [f32; 2],
    double: bool,
}

/// The blends of one frame: at most height, winter and box edge.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LutProgram {
    operations: [LutOperation; 3],
    len: usize,
}

impl Default for LutProgram {
    fn default() -> Self {
        let empty = LutOperation {
            sources: [LutSource::Asset(0); 4],
            factors: [0.0; 2],
            double: false,
        };
        Self {
            operations: [empty; 3],
            len: 0,
        }
    }
}

/// The grade of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grading {
    values: PostValuesResolved,
    luts: LutProgram,
    selection: Selection,
}

impl Grading {
    /// The selection and grade, for the dump report.
    pub fn describe(&self) -> String {
        let s = &self.selection;
        format!(
            "height volume {} fade {:.3}, box volume {} fade {:.3}, winter snow {}, night {:.3}; MiddleGrey {:.3}, HDR {:.3}..{:.3}, {} LUT blends",
            s.height,
            s.height_factor,
            s.region.map_or("none".into(), |index| index.to_string()),
            s.region_factor,
            s.snow.map_or("unknown".into(), |snow| format!("{snow:.3}")),
            s.night,
            self.values.middle_grey,
            self.values.hdr_min,
            self.values.hdr_max,
            self.luts.len,
        )
    }
}

/// The operands compares against the last applied grade.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Selection {
    height: usize,
    height_factor: f32,
    region: Option<usize>,
    region_factor: f32,
    snow: Option<f32>,
    night: f32,
}

impl Selection {
    /// Keep the last grade while the night factor moves by less
    /// than 0.001 and every other operand is equal.
    fn keeps(self, previous: Self) -> bool {
        Self {
            night: previous.night,
            ..self
        } == previous
            && (self.night - previous.night).abs() < BAND_EPSILON
    }
}

/// Blended values; the LUT may be an intermediate target.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PostValuesResolved {
    lut: LutSource,
    middle_grey: f32,
    hdr_min: f32,
    hdr_max: f32,
    bloom: [f32; 3],
}

impl PostAssets {
    /// Reads `NDefines.NGraphics`, `gfx/posteffect_volumes.txt` and every LUT.
    ///
    /// The volume file is verified when its recorded policy covers it; older
    /// policies name its uncovered read in the document's warnings. Each LUT
    /// goes through [`ResolvedLayers::read_uncovered_path`].
    pub fn load(
        layers: &ResolvedLayers,
        defines: &Defines,
        map: MapSize,
        decode: Decode,
    ) -> io::Result<Self> {
        let defines = PostDefines::load(defines)?;
        let file = "gfx/posteffect_volumes.txt";
        let refuse = |error: String| invalid(format!("{file}: {error}"));
        let block = |value: &Value, what: &str| {
            as_document(value).ok_or_else(|| refuse(format!("{what} is not a block")))
        };
        let document = map_snow::winter::read_post_effects(layers)?;

        let mut raw: BTreeMap<String, RawValues> = BTreeMap::new();
        for value in document.get_all("posteffect_values") {
            let values = block(value, "posteffect_values")?;
            let name = scalar(&values, "name")
                .ok_or_else(|| refuse("posteffect_values without a name".into()))?;
            raw.insert(
                name,
                RawValues {
                    inherit: scalar(&values, "inherit"),
                    lut: scalar(&values, "lut"),
                    middle_grey: number(&values, "tonemap_middlegrey").map_err(refuse)?,
                    hdr_min: number(&values, "hdr_min_adjustment").map_err(refuse)?,
                    hdr_max: number(&values, "hdr_max_adjustment").map_err(refuse)?,
                    bloom: [
                        number(&values, "BLOOM_WIDTH").map_err(refuse)?,
                        number(&values, "BLOOM_SCALE").map_err(refuse)?,
                        number(&values, "BRIGHT_THRESHOLD").map_err(refuse)?,
                    ],
                },
            );
        }
        // The empty path is the built-in identity cube.
        let mut lut_paths = vec![String::new()];
        let mut values = BTreeMap::new();
        for name in raw.keys() {
            let resolved = resolve(&raw, name, &mut lut_paths, defines).map_err(refuse)?;
            values.insert(name.clone(), resolved);
        }

        let (mut heights, boxes) =
            map_snow::winter::volumes(&document, |name| values.contains_key(name))?;
        heights.sort_by(|a, b| a.height.total_cmp(&b.height));

        let mut luts = Vec::with_capacity(lut_paths.len());
        for path in &lut_paths {
            luts.push(if path.is_empty() {
                identity_lut()
            } else {
                // Load the resolved LUT as a 1024-by-32 cube, which power-of-two decode settings
                // leave unpadded.
                load_image(path, &|path| layers.read_uncovered_path(path), decode)?
            });
        }

        Ok(Self {
            lut_shader: compiled_luts::load(layers, [map.width, map.height], decode)?,
            bloom: compiled_bloom::load(layers, map, decode)?,
            reductions: compiled_reduce::load(layers, [map.width, map.height], decode)?,
            seed: compiled_seed::load(layers, [map.width, map.height], decode)?,
            gather: compiled_gather::load(layers, [map.width, map.height], decode)?,
            restore: compiled_restore::load(layers, [map.width, map.height], decode)?,
            luts,
            volumes: Volumes {
                defines,
                identity: 0,
                // Set by `Volumes::set_snow` once the hour's weather is in.
                snow: SnowFactors::try_from_iter(
                    boxes.len(),
                    std::iter::repeat_n(None, boxes.len()),
                )?,
                values,
                heights,
                boxes,
            },
        })
    }
}

impl map_snow::winter::Factors for Volumes {
    fn snow_factors(&self, snow: &[Option<i64>]) -> io::Result<SnowFactors> {
        Volumes::snow_factors(self, snow)
    }
}

impl Volumes {
    pub fn winter(&self) -> Winter {
        Winter {
            defines: SnowDefines {
                per_province: self.defines.snow_per_province,
                total: self.defines.snow_total,
            },
            records: self.boxes.iter().map(|v| v.records.clone()).collect(),
        }
    }

    /// Select the highest faded-in height volume, then the first containing box, preserving LUT blend
    /// order.
    pub fn select(&self, camera: Camera, day_night: DayNight) -> Grading {
        let eye = camera.eye();
        let night = night_factor(eye, camera.map, day_night, self.defines);
        let mut luts = LutProgram::default();
        let mut selection = Selection {
            night,
            ..Selection::default()
        };
        let grading = |values, luts, selection| Grading {
            values,
            luts,
            selection,
        };
        // Without height volumes the native path skips the boxes too.
        if self.heights.is_empty() {
            return grading(self.fallback(), luts, selection);
        }
        let (index, fraction) = self
            .heights
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, volume)| {
                let fade = saturate((eye[1] - volume.height) / volume.fade_distance);
                (fade > 0.0).then_some((index, fade))
            })
            .unwrap_or((0, 0.0));
        selection.height = index;
        selection.height_factor = fraction;
        let current = &self.heights[index];
        let mut values = match index.checked_sub(1).map(|below| &self.heights[below]) {
            None => luts.blend(self.value(&current.day), self.value(&current.night), night),
            Some(previous) => luts.compose(
                [
                    self.value(&previous.day),
                    self.value(&previous.night),
                    self.value(&current.day),
                    self.value(&current.night),
                ],
                night,
                fraction,
            ),
        };

        let Some((index, volume, fraction)) =
            self.boxes.iter().enumerate().find_map(|(index, volume)| {
                let fraction = box_fade(eye, volume);
                (fraction > 0.0).then_some((index, volume, fraction))
            })
        else {
            return grading(values, luts, selection);
        };
        selection.region = Some(index);
        selection.region_factor = fraction;
        selection.snow = self.snow.get(index).copied().flatten();
        let day = self.value(&volume.day);
        let dark = self.value(&volume.night);
        // Take the winter path only with both winter values and
        // a nonzero snow factor; otherwise it composes the summer pair straight
        // into the height grade.
        values = match (&volume.day_winter, &volume.night_winter, selection.snow) {
            // Unrecorded representative snow cannot choose between the paths;
            // the height grade stays and the limitation is noted.
            (Some(_), Some(_), None) => return grading(values, luts, selection),
            (Some(winter_day), Some(winter_night), Some(winter)) if winter != 0.0 => {
                let region = luts.compose(
                    [day, dark, self.value(winter_day), self.value(winter_night)],
                    night,
                    winter,
                );
                luts.blend(values, region, fraction)
            }
            _ => luts.compose([values, values, day, dark], night, fraction),
        };
        grading(values, luts, selection)
    }

    /// Build each box volume's snow records from map geometry.
    ///
    /// Snow records are fixed for the session. Walk strategic regions in database order starting at
    /// index one. Province lists preserve template order and skip duplicates; snow_records supplies
    /// the per-region rule.
    ///
    /// Province selection assumes authored region order and database index order. Those runtime
    /// ordering inputs are unrecorded.
    pub fn set_regions(&mut self, map: &MapData, regions: &BTreeMap<u32, StrategicRegion>) {
        let extents = province_extents(map);
        for volume in &mut self.boxes {
            volume.records = snow_records(
                volume.ground_bounds(),
                regions.values().map(|region| region.provinces.as_slice()),
                &extents,
            );
        }
    }

    /// Winter factor for each box volume this hour.
    ///
    /// Snow uses recorded WeatherProvinces values by province id. A missing representative makes the
    /// factor unknown. Refresh after an hour change while the eye is inside the volume.
    pub fn set_snow(&mut self, snow: &[Option<i64>]) -> io::Result<()> {
        self.snow = self.snow_factors(snow)?;
        Ok(())
    }

    /// Computes the same factors before an hour is published, without changing
    /// the view's currently selected grade or its retained graphics history.
    pub fn snow_factors(&self, snow: &[Option<i64>]) -> io::Result<SnowFactors> {
        SnowFactors::try_from_iter(
            self.boxes.len(),
            self.boxes.iter().map(|volume| {
                map_snow::winter::snow_factor(
                    &volume.records,
                    snow,
                    SnowDefines {
                        per_province: self.defines.snow_per_province,
                        total: self.defines.snow_total,
                    },
                )
            }),
        )
    }

    /// A values block the loader has checked exists.
    fn value(&self, name: &str) -> PostValuesResolved {
        let values = &self.values[name];
        PostValuesResolved {
            lut: LutSource::Asset(values.lut),
            middle_grey: values.middle_grey,
            hdr_min: values.hdr_min,
            hdr_max: values.hdr_max,
            bloom: values.bloom,
        }
    }

    fn fallback(&self) -> PostValuesResolved {
        PostValuesResolved {
            lut: LutSource::Asset(self.identity),
            middle_grey: self.defines.middle_grey,
            hdr_min: self.defines.hdr_adjustment[0],
            hdr_max: self.defines.hdr_adjustment[1],
            bloom: self.defines.bloom,
        }
    }
}

impl PostValuesResolved {
    /// Lerps the numbers; the LUT is whatever the blend produced.
    fn mix(self, other: Self, t: f32, lut: LutSource) -> Self {
        Self {
            lut,
            middle_grey: lerp(self.middle_grey, other.middle_grey, t),
            hdr_min: lerp(self.hdr_min, other.hdr_min, t),
            hdr_max: lerp(self.hdr_max, other.hdr_max, t),
            bloom: std::array::from_fn(|index| lerp(self.bloom[index], other.bloom[index], t)),
        }
    }
}

impl LutProgram {
    fn push(&mut self, sources: [LutSource; 4], inner: f32, outer: f32, double: bool) -> LutSource {
        let index = self.len;
        self.operations[index] = LutOperation {
            sources,
            factors: [inner, outer],
            double,
        };
        self.len += 1;
        LutSource::Intermediate(index)
    }

    /// Endpoints and identical LUTs skip the draw.
    fn blend(
        &mut self,
        a: PostValuesResolved,
        b: PostValuesResolved,
        t: f32,
    ) -> PostValuesResolved {
        if t == 0.0 {
            return a;
        }
        if t == 1.0 {
            return b;
        }
        let lut = if a.lut == b.lut {
            a.lut
        } else {
            self.push([a.lut, a.lut, b.lut, b.lut], 0.0, t, false)
        };
        a.mix(b, t, lut)
    }

    /// a double blend is one draw, not two rounded inner draws.
    fn compose(
        &mut self,
        [a, b, c, d]: [PostValuesResolved; 4],
        inner: f32,
        outer: f32,
    ) -> PostValuesResolved {
        if inner == 0.0 {
            return self.blend(a, c, outer);
        }
        if inner == 1.0 {
            return self.blend(b, d, outer);
        }
        let first = a.mix(b, inner, a.lut);
        let second = c.mix(d, inner, c.lut);
        let lut = if a.lut == b.lut && c.lut == d.lut {
            self.blend(first, second, outer).lut
        } else {
            self.push([a.lut, b.lut, c.lut, d.lut], inner, outer, true)
        };
        first.mix(second, outer, lut)
    }
}

struct RawValues {
    inherit: Option<String>,
    lut: Option<String>,
    middle_grey: Option<f32>,
    hdr_min: Option<f32>,
    hdr_max: Option<f32>,
    bloom: [Option<f32>; 3],
}

/// Folds an `inherit` chain, registering each distinct LUT path once. Unset
/// numbers take the `NDefines.NGraphics` defaults; an unset LUT is the
/// identity (the empty path).
fn resolve(
    raw: &BTreeMap<String, RawValues>,
    name: &str,
    paths: &mut Vec<String>,
    defines: PostDefines,
) -> Result<PostValues, String> {
    let mut chain = Vec::new();
    let mut cursor = Some(name);
    while let Some(current) = cursor {
        let entry = raw
            .get(current)
            .ok_or_else(|| format!("{name:?} inherits unknown values {current:?}"))?;
        if chain.len() == raw.len() {
            return Err(format!("{name:?} has an inheritance cycle"));
        }
        chain.push(entry);
        cursor = entry.inherit.as_deref();
    }
    let pick = |get: fn(&RawValues) -> Option<f32>, default: f32| {
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
    Ok(PostValues {
        lut,
        middle_grey: pick(|entry| entry.middle_grey, defines.middle_grey),
        hdr_min: pick(|entry| entry.hdr_min, defines.hdr_adjustment[0]),
        hdr_max: pick(|entry| entry.hdr_max, defines.hdr_adjustment[1]),
        bloom: [
            pick(|entry| entry.bloom[0], defines.bloom[0]),
            pick(|entry| entry.bloom[1], defines.bloom[1]),
            pick(|entry| entry.bloom[2], defines.bloom[2]),
        ],
    })
}

/// the identity cube uses nearest-byte rounding on a 32-point
/// RGB lattice. Native storage is BGRA; this asset boundary owns RGBA bytes.
fn identity_lut() -> Rgba8Image {
    let mut pixels = vec![0; 1024 * 32 * 4];
    let channel = |value: usize| ((value as f32 * (255.0 / 31.0)) + 0.5) as u8;
    for blue in 0..32 {
        for green in 0..32 {
            for red in 0..32 {
                let at = (green * 1024 + blue * 32 + red) * 4;
                pixels[at..at + 4].copy_from_slice(&[
                    channel(red),
                    channel(green),
                    channel(blue),
                    255,
                ]);
            }
        }
    }
    Rgba8Image {
        width: 1024,
        height: 32,
        pixels: pixels.into(),
    }
}

/// How far inside a box volume the camera is, as 0 outside to 1 beyond the fade.
fn box_fade(eye: [f32; 3], volume: &BoxVolume) -> f32 {
    let mut inside = f32::INFINITY;
    for ((at, position), size) in eye.iter().zip(&volume.position).zip(&volume.size) {
        // Stored native faces are rounded before the per-frame subtractions.
        let lower = position - size * 0.5;
        let upper = position + size * 0.5;
        inside = inside.min((at - lower).min(upper - at));
    }
    saturate(inside / volume.fade_distance)
}

/// Post-processing height feather for grading volumes.
fn night_factor(eye: [f32; 3], map: MapSize, day_night: DayNight, defines: PostDefines) -> f32 {
    let direction = day_night.hour_sun_dir();
    let x = ((eye[0] - defines.gmt_offset) / map.x() + direction[0]) % 1.0;
    let mut y = eye[2] / map.y();
    y = defines.poles[0] + (defines.poles[1] - defines.poles[0]) * y;
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
    let normal = normal.map(|component| component / length);
    let value = normal[1] * direction[2] + normal[0] * direction[1] + normal[2] * direction[3];
    let height = saturate(
        (eye[1] - defines.feather_distance[0])
            / (defines.feather_distance[1] - defines.feather_distance[0]),
    );
    let feather = lerp(defines.feather_amount[0], defines.feather_amount[1], height);
    saturate(((value + feather) / (2.0 * feather)) * day_night.strength)
}

fn as_document(value: &Value) -> Option<parser::Document> {
    Some(parser::Document {
        items: value.items()?.to_vec(),
    })
}

fn scalar(document: &parser::Document, key: &str) -> Option<String> {
    document.get(key).and_then(Value::scalar_str).map(unquote)
}

/// An optional number; a value that is present but not a number is refused.
fn number(document: &parser::Document, key: &str) -> Result<Option<f32>, String> {
    let Some(value) = document.get(key) else {
        return Ok(None);
    };
    value
        .scalar_str()
        .and_then(|text| text.trim().parse().ok())
        .map(Some)
        .ok_or_else(|| format!("{key} is not a number"))
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

/// One luminance reduction: its viewport and native `UVScale`.
#[derive(Clone, Debug, PartialEq)]
struct ReductionStep {
    viewport: [u32; 2],
    uv_scale: [f32; 2],
}

/// The seed size, fixed backing size of both reduction targets, shrinking viewports and rectangle
/// read by the gather pass.
#[derive(Clone, Debug, PartialEq)]
struct ReductionSchedule {
    seed: [u32; 2],
    backing: [u32; 2],
    steps: Vec<ReductionStep>,
    gather: [u32; 2],
}

impl ReductionSchedule {
    fn new(scene: [u32; 2]) -> Self {
        let seed = scene.map(|value| value / 2);
        let backing = seed.map(|value| value.div_ceil(2));
        let mut active = seed;
        let mut source_size = seed;
        let mut steps = Vec::new();
        loop {
            let viewport = active.map(|value| value.div_ceil(2));
            if viewport[0] <= 3 && viewport[1] <= 3 {
                break;
            }
            // Preserve native float operation order, including odd dimensions.
            let uv_scale = std::array::from_fn(|axis| {
                ((viewport[axis] * 2) as f32 / active[axis] as f32)
                    * (active[axis] as f32 / source_size[axis] as f32)
            });
            steps.push(ReductionStep { viewport, uv_scale });
            active = viewport;
            source_size = backing;
        }
        Self {
            seed,
            backing,
            steps,
            gather: active,
        }
    }
}

/// The bloom seed, the bloom blur target and the two fixed reduction targets for one render
/// size, and the scene copy's binding of the targets they were made for.
struct Chain {
    size: [u32; 2],
    /// The native downsample target: bright-pass RGB and log-luminance alpha,
    /// later overwritten by the vertical blur and read by `RestoreBloom`.
    seed_view: wgpu::TextureView,
    bloom_view: wgpu::TextureView,
    reductions: [wgpu::TextureView; 2],
    schedule: ReductionSchedule,
    copy: wgpu::BindGroup,
}

/// The two one-texel adaptation targets: the exposure history of targets of one size and world
/// size. Hiding the view keeps it, so a view shown again adapts on from where it was.
struct History {
    targets: [[u32; 2]; 2],
    adapted: [wgpu::TextureView; 2],
    /// The target the next gather reads.
    parity: usize,
    /// Made zeroed and not drawn since: the next grade takes the instantaneous value.
    fresh: bool,
}

/// A block of the chain's kept constants: large enough for any of its constant registers (the
/// seed's 80-byte b1 is the largest) and laid at the device's uniform offset alignment, so the
/// bytes at an offset can be written again without the bindings over them changing.
const BLOCK: usize = 256;

/// The chain's constant blocks, laid out once for the bindings that read them.
struct Blocks {
    stride: usize,
    bytes: Vec<u8>,
}

impl Blocks {
    /// `alignment` is the device's `min_uniform_buffer_offset_alignment`.
    fn new(alignment: u32) -> Self {
        Self {
            stride: BLOCK.next_multiple_of((alignment as usize).max(1)),
            bytes: Vec::new(),
        }
    }

    /// Lays `bytes` down zero-filled to a whole block and gives the offset they stand at.
    fn push(&mut self, bytes: &[u8]) -> u32 {
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        self.bytes.resize(offset + self.stride, 0);
        offset as u32
    }
}

/// The views the chain's kept bindings bind: the targets and the exposure history they were made
/// with, the colour cube the restore samples, and whether the grading's blur has a weight, which
/// decides whether the restore samples the blurred seed or the zeroed texture. A grade that keeps
/// these is a write of the blocks the bindings read, never bindings of its own.
#[derive(PartialEq)]
struct KeptFor {
    made: u64,
    lut: wgpu::TextureView,
    blurred: bool,
}

/// Every post pass's constants and bindings, made when [`KeptFor`] changes and recorded on every
/// frame between: one buffer holds each pass's block, and the passes whose views the exposure
/// history's parity picks keep one set of bindings a parity.
struct Kept {
    made_for: KeptFor,
    constants: wgpu::Buffer,
    seed: shader_pipeline::gpu::Bindings,
    reductions: Vec<shader_pipeline::gpu::Bindings>,
    /// By the parity the adaptation reads.
    adaptation: [shader_pipeline::gpu::Bindings; 2],
    /// By the parity the adaptation reads; the restore samples the one it wrote.
    restore: [shader_pipeline::gpu::Bindings; 2],
    /// Horizontal then vertical; `None` where the grading gives the blur no weight.
    blurs: Option<[shader_pipeline::gpu::Bindings; 2]>,
    /// Where the blocks that are not the chain's own stand: the seed's and restore's b1, the
    /// adaptation's b2, and the two blurs' b2.
    native_block: u32,
    adaptation_block: u32,
    blur_blocks: Option<[u32; 2]>,
    /// The operands the blocks above hold: the output size and grade of the last frame that
    /// wrote them, and the adaptation's τ, which is the frame's own.
    output: [u32; 2],
    values: PostValuesResolved,
    tau: f32,
}

impl Kept {
    /// Writes the blocks whose operands this frame changed, leaving the bindings over them as
    /// they are: the grade's own values (and with them the bloom's weights), the output size the
    /// restore grades at, and the adaptation's τ.
    fn restate(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        chain: &Chain,
        defines: PostDefines,
        output: [u32; 2],
        values: PostValuesResolved,
        tau: f32,
    ) {
        let graded = (self.output, self.values) != (output, values);
        let write = |offset: u32, bytes: &[u8]| arena.write(&self.constants, offset.into(), bytes);
        if graded {
            let native = native_block(chain, defines, output, values);
            write(self.native_block, bytemuck::cast_slice(&native));
            if let Some(blocks) = self.blur_blocks {
                // A grade that keeps a weight keeps the blurs; one that loses it remakes them.
                for (offset, blur) in blocks.iter().zip(blur_blocks(chain, values).iter().flatten())
                {
                    write(*offset, bytemuck::bytes_of(blur));
                }
            }
            self.output = output;
            self.values = values;
        }
        if graded || self.tau != tau {
            let adaptation = compiled_gather::Gather::constants(
                chain.schedule.gather,
                chain.schedule.backing,
                tau,
                [values.hdr_min, values.hdr_max],
            );
            write(self.adaptation_block, bytemuck::cast_slice(&adaptation));
            self.tau = tau;
        }
    }
}

/// The seed's and restore's shared b1: HSV bytes 0..15, colour balance 16..27, emissive 28,
/// inverse seed 32/36, inverse viewport 40/44, bloom scale 48/52, threshold 56, middle grey 60
/// and squared white luminance 64. HSV and colour balance use neutral renderer constants.
/// Preserve the negative zero.
fn native_block(
    chain: &Chain,
    defines: PostDefines,
    output: [u32; 2],
    values: PostValuesResolved,
) -> [f32; 17] {
    [
        -0.0_f32,
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        defines.emissive_strength,
        1.0 / chain.schedule.seed[0] as f32,
        1.0 / chain.schedule.seed[1] as f32,
        // Use the full restore viewport for InvWindowSize,
        // including with a half-size scene; seed/seed BloomToScreenScale is 1.
        1.0 / output[0] as f32,
        1.0 / output[1] as f32,
        1.0,
        1.0,
        values.bloom[2],
        values.middle_grey,
        defines.luminance_white_squared,
    ]
}

/// The two blurs' b2, horizontal into the blur target then vertical back into the seed; `None`
/// where the grading gives the blur no weight, when the blurs are not drawn at all.
fn blur_blocks(chain: &Chain, values: PostValuesResolved) -> Option<[Blur; 2]> {
    let [width, scale, _] = values.bloom;
    blur_weights(width, scale).map(|(weight0, weights, offsets)| {
        [1.0, 0.0].map(|axis| Blur {
            inv_size: chain.schedule.seed.map(|side| 1.0 / side as f32),
            axis,
            weight0,
            weights,
            offsets,
        })
    })
}

/// The pipelines of the post stage and the resources they bind.
pub struct PostPass {
    compiled_bloom: compiled_bloom::Bloom,
    compiled_reduce: compiled_reduce::Reductions,
    compiled_seed: compiled_seed::Seed,
    compiled_gather: compiled_gather::Gather,
    compiled_restore: compiled_restore::Restore,
    /// `--postfx off`: the float scene into the 8-bit output without a grade.
    /// See [`PostPass::copy_scene`].
    copy: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Keeps the device's copy program for the next view.
    _copy: Arc<CopyProgram>,
    luts: luts::LutRenderer,
    /// The blurred seed a zero-weight bloom adds: one zeroed texel, whatever the restore samples.
    zero_bloom: wgpu::TextureView,
    volumes: Volumes,
    chain: Option<Chain>,
    history: Option<History>,
    kept: Option<Kept>,
    /// Counts the targets and histories made, so bindings over the ones before are not kept.
    made: u64,
    last_grading: Option<Grading>,
}

/// `bloom.shader`'s `ConstantBuffer(2, 39)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Blur {
    inv_size: [f32; 2],
    axis: f32,
    weight0: f32,
    weights: [f32; 4],
    offsets: [f32; 4],
}

/// Five Gaussian taps use variance `2 * BLOOM_WIDTH^2`, floored at 0.01, over distances zero through
/// eight texels. Normalize over both sides, scale by `BLOOM_SCALE`, then fold into linearly filtered
/// pairs. `None` when every weight is zero, so the blur would write zeros whatever it read; a pair
/// of zero weight keeps the centre texel, which that weight then multiplies by zero.
fn blur_weights(width: f32, scale: f32) -> Option<(f32, [f32; 4], [f32; 4])> {
    let variance = (2.0 * width * width).max(0.01);
    let norm = (variance * std::f32::consts::PI).sqrt();
    let taps: [f32; 9] = std::array::from_fn(|i| (-((i * i) as f32) / variance).exp() / norm);
    let total = taps[0] + 2.0 * taps[1..].iter().sum::<f32>();
    let g = taps.map(|tap| tap / total * scale);
    let weights: [f32; 4] = std::array::from_fn(|k| g[2 * k + 1] + g[2 * k + 2]);
    if g[0] == 0.0 && weights.iter().all(|weight| *weight == 0.0) {
        return None;
    }
    let offsets = std::array::from_fn(|k| {
        if weights[k] == 0.0 {
            0.0
        } else {
            ((2 * k + 1) as f32 * g[2 * k + 1] + (2 * k + 2) as f32 * g[2 * k + 2]) / weights[k]
        }
    });
    Some((g[0], weights, offsets))
}

/// The post copy program, its layout and sampler: one per device and format.
struct CopyProgram {
    copy: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl CopyProgram {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer scene copy inputs"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(1),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_copy.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let stage = |label: &str, entry: &'static str, target| {
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
                multisample: wgpu::MultisampleState::default(),
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
        Self {
            copy: stage("post copy", "copy_main", format),
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
        }
    }
}

impl PostPass {
    pub fn invalidate(&mut self) {
        self.chain = None;
        self.history = None;
        self.kept = None;
        self.luts.invalidate();
    }

    /// Lets go of the chain sized to the targets; [`Self::prepare`] makes it again. The exposure
    /// history stays unless it was never drawn, when it holds nothing a new one would not.
    pub fn release_chain(&mut self) {
        self.chain = None;
        self.kept = None;
        if self.history.as_ref().is_some_and(|history| history.fresh) {
            self.history = None;
        }
    }

    /// Whether [`Self::prepare`] has made everything targets of `size` and `world_size` need.
    pub fn prepared(&self, size: [u32; 2], world_size: [u32; 2]) -> bool {
        self.chain
            .as_ref()
            .is_some_and(|chain| chain.size == world_size)
            && self
                .history
                .as_ref()
                .is_some_and(|history| history.targets == [size, world_size])
    }

    /// Makes what [`Self::draw`] and [`Self::copy_scene`] draw into for `targets`, which are new
    /// when `rebuilt`. Targets of another size or world size start a zeroed exposure history,
    /// as a resize does in the game; the same targets made again keep it.
    pub fn prepare(&mut self, device: &wgpu::Device, targets: &Targets, rebuilt: bool) {
        let size = targets.world_size();
        if rebuilt || self.chain.as_ref().is_none_or(|chain| chain.size != size) {
            self.chain = Some(self.make_chain(device, targets));
            self.made += 1;
        }
        let key = [targets.size(), size];
        if self
            .history
            .as_ref()
            .is_none_or(|history| history.targets != key)
        {
            self.made += 1;
            self.history = Some(History {
                targets: key,
                adapted: [
                    target(device, "post adapted 0", [1, 1], LUMINANCE_FORMAT),
                    target(device, "post adapted 1", [1, 1], LUMINANCE_FORMAT),
                ],
                parity: 0,
                fresh: true,
            });
        }
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &PostAssets,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let volumes = assets.volumes.clone();
        let program =
            viewer_foundation::map::device_shared::shared(device, ("post copy", format), || {
                CopyProgram::new(device, format)
            });
        let luts = luts::LutRenderer::new(device, queue, assets)?;

        Ok(Self {
            compiled_bloom: compiled_bloom::Bloom::new(device, &assets.bloom)?,
            compiled_reduce: compiled_reduce::Reductions::new(device, &assets.reductions)?,
            compiled_seed: compiled_seed::Seed::new(device, &assets.seed)?,
            compiled_gather: compiled_gather::Gather::new(device, &assets.gather)?,
            compiled_restore: compiled_restore::Restore::new(device, &assets.restore, format)?,
            copy: program.copy.clone(),
            layout: program.layout.clone(),
            sampler: program.sampler.clone(),
            _copy: program,
            luts,
            zero_bloom: target(device, "post bloom zero", [1, 1], BLOOM_FORMAT),
            volumes,
            chain: None,
            history: None,
            kept: None,
            made: 0,
            last_grading: None,
        })
    }

    /// The grade the last graded frame used.
    pub fn last_grading(&self) -> Option<Grading> {
        self.last_grading
    }

    pub fn adaptation_tau(&self, delta: Option<f32>) -> f32 {
        // Raw speed times delta; reset value is 100. A stationary
        // replay frame deliberately resets exposure so offline dumps stand alone.
        delta.map_or(100.0, |dt| self.volumes.defines.adaptation_speed * dt)
    }

    /// The grading this camera and hour select, or the last applied one while
    /// [`Selection::keeps`] it.
    pub fn grading(&mut self, camera: Camera, day_night: DayNight) -> Grading {
        let current = self.volumes.select(camera, day_night);
        if let Some(previous) = self.last_grading
            && current.selection.keeps(previous.selection)
        {
            return previous;
        }
        self.last_grading = Some(current);
        current
    }

    /// Recomputes every volume's winter factor from this hour's raw province snow.
    pub fn set_snow(&mut self, snow: &[Option<i64>]) -> io::Result<()> {
        self.volumes.set_snow(snow)
    }

    /// Adopts factors built from the same content's volume tables.
    pub fn set_snow_factors(&mut self, factors: &SnowFactors) {
        self.volumes.snow = factors.clone();
    }

    /// The post targets and bind groups for a render size. New targets are zeroed like the
    /// native allocation.
    fn make_chain(&self, device: &wgpu::Device, targets: &Targets) -> Chain {
        let size = targets.world_size();
        let schedule = ReductionSchedule::new(size);
        let seed_view = target(device, "post bright/log seed", schedule.seed, BLOOM_FORMAT);
        let bloom_view = target(device, "post bloom blur", schedule.seed, BLOOM_FORMAT);
        let reductions = [
            target(device, "post reduction 0", schedule.backing, LUMINANCE_FORMAT),
            target(device, "post reduction 1", schedule.backing, LUMINANCE_FORMAT),
        ];
        let copy = shader_pipeline::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
            label: Some("viewer scene copy"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(targets.scene_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Chain {
            size,
            seed_view,
            bloom_view,
            reductions,
            schedule,
            copy,
        }
    }

    /// The chain's constants and bindings for these targets: made here, kept while [`KeptFor`]
    /// stands, and recorded by [`Self::draw`] on every frame between.
    fn make_kept(
        &self,
        device: &wgpu::Device,
        targets: &Targets,
        made_for: KeptFor,
        values: PostValuesResolved,
        tau: f32,
    ) -> io::Result<Kept> {
        let (Some(chain), Some(history)) = (&self.chain, &self.history) else {
            unreachable!("prepared by the caller");
        };
        let output = targets.size();
        let native = native_block(chain, self.volumes.defines, output, values);
        let native: &[u8] = bytemuck::cast_slice(&native);
        let reductions: Vec<[f32; 10]> = chain
            .schedule
            .steps
            .iter()
            .map(|step| compiled_reduce::Reductions::constants(step.viewport, step.uv_scale))
            .collect();
        let adaptation = compiled_gather::Gather::constants(
            chain.schedule.gather,
            chain.schedule.backing,
            tau,
            [values.hdr_min, values.hdr_max],
        );
        let adaptation: &[u8] = bytemuck::cast_slice(&adaptation);
        let blurs = blur_blocks(chain, values);
        let mut blocks = Blocks::new(device.limits().min_uniform_buffer_offset_alignment);
        let native_at = blocks.push(native);
        let reductions_at: Vec<u32> = reductions
            .iter()
            .map(|constants| blocks.push(bytemuck::cast_slice(constants)))
            .collect();
        let adaptation_at = blocks.push(adaptation);
        let blurs_at = blurs.map(|pair| pair.map(|blur| blocks.push(bytemuck::bytes_of(&blur))));
        let constants = shader_pipeline::gpu::stats::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("post kept constants"),
                contents: &blocks.bytes,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        let seed = self.compiled_seed.kept(
            device,
            &constants,
            native_at,
            native,
            targets.scene_view(),
        )?;
        let reduction_bindings = reductions
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let source = if index == 0 {
                    &chain.seed_view
                } else {
                    &chain.reductions[(index - 1) % 2]
                };
                self.compiled_reduce.kept(
                    device,
                    &constants,
                    index,
                    reductions_at[index],
                    bytemuck::cast_slice(step),
                    source,
                )
            })
            .collect::<io::Result<Vec<_>>>()?;
        let gathered = if reductions.is_empty() {
            &chain.seed_view
        } else {
            &chain.reductions[(reductions.len() - 1) % 2]
        };
        let adapted = |parity: usize| {
            self.compiled_gather.kept(
                device,
                &constants,
                adaptation_at,
                adaptation,
                gathered,
                &history.adapted[parity],
            )
        };
        // The vertical blur writes the blurred seed back into the seed target, so the restore reads
        // it there; with no weight it reads the zeroed texture the blurs would have written zeros into.
        let blurred = if blurs.is_some() {
            &chain.seed_view
        } else {
            &self.zero_bloom
        };
        let graded = |parity: usize| {
            self.compiled_restore.kept(
                device,
                &constants,
                native_at,
                native,
                [
                    targets.scene_view(),
                    blurred,
                    self.luts.output(),
                    // The restore samples what the adaptation of this parity wrote.
                    &history.adapted[1 - parity],
                ],
            )
        };
        let blurs = match (blurs, blurs_at) {
            (Some([horizontal, vertical]), Some([at_x, at_y])) => Some([
                self.compiled_bloom
                    .kept(device, &constants, at_x, &horizontal, &chain.seed_view)?,
                self.compiled_bloom
                    .kept(device, &constants, at_y, &vertical, &chain.bloom_view)?,
            ]),
            _ => None,
        };
        Ok(Kept {
            made_for,
            seed,
            reductions: reduction_bindings,
            adaptation: [adapted(0)?, adapted(1)?],
            restore: [graded(0)?, graded(1)?],
            blurs,
            constants,
            native_block: native_at,
            adaptation_block: adaptation_at,
            blur_blocks: blurs_at,
            output,
            values,
            tau,
        })
    }

    /// Runs the whole stage in 's order: the LUT blends, the
    /// bright/log seed, the luminance chain, the bloom blur where the grading in force gives it a
    /// weight, then `RestoreBloom` into the output attachment. Every pass draws with the bindings
    /// [`Self::make_kept`] made for this grade and these targets, so an ordinary frame writes the
    /// adaptation's τ and records the draws.
    ///
    /// `tau_delta` is raw `HDR_ADJUSTMENT_SPEED * dt`; reset uses native100. It draws into
    /// what [`Self::prepare`] made for `targets`.
    pub fn draw(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        grading: Grading,
        tau_delta: f32,
        mark: &mut impl FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) -> io::Result<()> {
        if !self.prepared(targets.size(), targets.world_size()) {
            return Err(invalid(
                "the post chain was not prepared for this map's targets",
            ));
        }
        self.luts
            .draw(arena, encoder, grading.luts, grading.values.lut, mark)?;
        mark(encoder, "post LUT");
        let values = grading.values;
        // A fresh history takes the instantaneous value whatever the caller asked for.
        let tau = match &self.history {
            Some(history) if history.fresh => 100.0,
            _ => tau_delta,
        };
        let made_for = KeptFor {
            made: self.made,
            lut: self.luts.output().clone(),
            blurred: blur_weights(values.bloom[0], values.bloom[1]).is_some(),
        };
        if self
            .kept
            .as_ref()
            .is_none_or(|kept| kept.made_for != made_for)
        {
            self.kept = Some(self.make_kept(arena.device(), targets, made_for, values, tau)?);
        }
        let defines = self.volumes.defines;
        let (Some(chain), Some(history), Some(kept)) =
            (&self.chain, &mut self.history, &mut self.kept)
        else {
            unreachable!("prepared and made above");
        };
        kept.restate(arena, chain, defines, targets.size(), values, tau);
        self.compiled_seed
            .record(encoder, &chain.seed_view, &kept.seed);
        mark(encoder, "post seed");
        for (index, step) in chain.schedule.steps.iter().enumerate() {
            self.compiled_reduce.record(
                encoder,
                index,
                step.viewport,
                &chain.reductions[index % 2],
                &kept.reductions[index],
            );
        }
        mark(encoder, "post luminance reductions");
        let next = 1 - history.parity;
        self.compiled_gather.record(
            encoder,
            &history.adapted[next],
            &kept.adaptation[history.parity],
        );
        mark(encoder, "post luminance adaptation");
        if let Some([horizontal, vertical]) = &kept.blurs {
            self.compiled_bloom
                .draw(encoder, &chain.bloom_view, horizontal);
            mark(encoder, "post bloom x");
            self.compiled_bloom.draw(encoder, &chain.seed_view, vertical);
            mark(encoder, "post bloom y");
        }
        let restore = &kept.restore[history.parity];
        // The grade writes the single-sample 8-bit output; the overlays load it and draw over it.
        let mut render = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post grade"),
            color_attachments: &[Some(
                targets.output_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
            )],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.compiled_restore.draw(&mut render, restore);
        drop(render);
        history.parity = next;
        history.fresh = false;
        Ok(())
    }

    /// `--postfx off`: moves the float scene into the 8-bit output without
    /// grading it, so the verification dump is the raw scene the world passes
    /// wrote, clamped only by the output format.
    ///
    /// The scene and the output are separate targets now that the scene is
    /// float, so skipping the grade would otherwise leave the output holding
    /// the previous frame. This separate viewer shader samples only the scene.
    ///
    /// A fresh exposure history stays zeroed, so a later grade adapts from zero.
    pub fn copy_scene(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
    ) -> io::Result<()> {
        if !self.prepared(targets.size(), targets.world_size()) {
            return Err(invalid(
                "the post chain was not prepared for this map's targets",
            ));
        }
        let (Some(chain), Some(history)) = (&self.chain, &mut self.history) else {
            unreachable!("prepared above");
        };
        history.fresh = false;
        let mut render = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post copy"),
            color_attachments: &[Some(
                targets.output_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
            )],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        shader_pipeline::gpu::stats::pipeline();
        render.set_pipeline(&self.copy);
        shader_pipeline::gpu::stats::bind();
        render.set_bind_group(0, &chain.copy, &[]);
        shader_pipeline::gpu::stats::draw();
        render.draw(0..3, 0..1);
        Ok(())
    }
}

/// A zeroed single-sample target the post stage draws into and samples.
fn target(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests;
