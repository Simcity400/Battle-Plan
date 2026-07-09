//! Files-first color-grading law for the map RestoreScene post-process.
//!
//! The installed content are the source of truth so mods work: this module derives the
//! RestoreScene ColorCube and PostEffectBase constants from the shipped
//! `gfx/posteffect_volumes.txt` selection table + `gfx/world/colorcorrection_*.tga`
//! cubes + `common/defines/00_graphics.lua`, all loaded through the mod-aware
//! [`GameFileResolver`]. Captured GPU evidence is used only to VALIDATE this law
//! (see `render.rs` `--postfx-validate-capture`); it is never a render input.
//!
//! ## Selection law (cited)
//!
//! `gfx/posteffect_volumes.txt` declares:
//!   * `posteffect_values` blocks: named grade presets with `lut = "<tga>"`,
//!     `tonemap_middlegrey`, and an optional `inherit = <name>` (unset scalars
//!     fall back to the inherited value; commented lines are inert).
//!   * a `posteffect_volumes { ... }` container holding
//!     - `posteffect_height_volume` entries selected by camera height
//!       (`posteffect_values_day` / `_night`, `height` = the floor at/above
//!       which the volume wins), and
//!     - `posteffect_volume` region boxes selected by 3D `position`/`size`
//!       containment (`posteffect_values_day/_night/_day_winter/_night_winter`).
//!
//! Selection for a frame: if the camera point is inside a region
//! `posteffect_volume` box, that volume's value wins; otherwise the
//! `posteffect_height_volume` with the greatest `height <= camera.y` wins.
//! Day/night (and winter) pick the matching `posteffect_values_*` field.
//!
//! ## Validated against live capture (restore_gate_1, 2026-07-06, vanilla)
//!
//! Camera vCamPos = (2816, 2958, 1400), daytime. Height 2958 exceeds every
//! region box's vertical extent (e.g. EuropeandRussia spans y in -125..675),
//! so no region volume applies; the tallest height volume `Base Highest`
//! (height 1600) wins -> `max_distance` (day) -> `lut =
//! gfx/world/colorcorrection_farrange.tga`, and its `tonemap_middlegrey = 0.65`
//! line is commented out so it inherits `standard`'s 0.55. The composed LUT
//! decodes byte-exactly to the captured ColorCube (max per-channel delta 0 over
//! all 32768 texels), and MiddleGrey 0.55 matches the captured PostEffectBase CB.
//!
//! Caveat (adversarial review): `colorcorrection_farrange.tga`, `_midrange.tga`
//! and `_standard.tga` are byte-identical (MD5 655e09bb...), so the captured
//! cube alone cannot prove which value name was selected -- camera height only
//! makes `max_distance` the plausible route. And this reproduces the captured
//! cube with no CPU blend; `gfx/FX/lut_blender.shader` still exists, and a blend
//! between identical endpoints (or factor 0/1) is observationally identical --
//! this law asserts only that one shipped cube reproduces this frame, not that
//! the engine skipped a blender pass in general.

use map_viewer::{color_correction_lut_from_resolver, ColorCorrectionLut, GameFileResolver};
use std::collections::BTreeMap;

pub const POSTEFFECT_VOLUMES_REL: &str = "gfx/posteffect_volumes.txt";
pub const GRAPHICS_DEFINES_REL: &str = "common/defines/00_graphics.lua";

/// A single `posteffect_values` preset after `inherit` is resolved. Only the
/// fields the grading pass consumes are retained; every scalar is optional so an
/// `inherit` chain can fill the gaps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostEffectValues {
    pub name: String,
    pub inherit: Option<String>,
    pub lut: Option<String>,
    pub tonemap_middlegrey: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct HeightVolume {
    name: String,
    day: Option<String>,
    night: Option<String>,
    height: f32,
    /// `fade_distance` (shipped: 200/200/0 at posteffect_volumes.txt:352/362/372)
    /// — the camera-height span above this volume's floor over which the grade
    /// cross-fades from the next-lower volume into this one.
    fade_distance: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct RegionVolume {
    name: String,
    day: Option<String>,
    night: Option<String>,
    day_winter: Option<String>,
    night_winter: Option<String>,
    position: Vec3,
    size: Vec3,
    /// Region-box `fade_distance` (spatial edge fade). Parsed for completeness;
    /// the engine's spatial fade law for region boxes is UNTRACED and NOT
    /// implemented (region selection stays hard containment).
    #[allow(dead_code)]
    fade_distance: f32,
}

/// The full parsed `posteffect_volumes.txt` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostEffectVolumeTable {
    values: BTreeMap<String, PostEffectValues>,
    value_order: Vec<String>,
    height_volumes: Vec<HeightVolume>,
    region_volumes: Vec<RegionVolume>,
}

/// The camera / world state that drives selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradingSelectionState {
    pub camera: [f32; 3],
    pub night: bool,
    pub winter: bool,
}

/// The resolved grade for one frame: which value block won, why, the LUT it
/// pins, and the MiddleGrey it pins.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGrade {
    pub value_name: String,
    pub selection_reason: String,
    pub lut_rel_path: String,
    pub middle_grey: f32,
}

/// The fully files-derived grading inputs the renderer uploads.
#[derive(Clone, Debug, PartialEq)]
pub struct FileDerivedGrade {
    pub resolved: ResolvedGrade,
    pub lut: ColorCorrectionLut,
    /// 1024x32 RGBA8, ready for an immutable D3D11 texture (opaque alpha).
    pub color_cube_rgba8: Vec<u8>,
    pub average_luminance: f32,
    pub source_note: String,
}

/// `TONE_MAP_MIDDLE_GREY` etc. read from the shipped graphics defines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphicsGradingDefines {
    pub tone_map_middle_grey: f32,
    pub tone_map_luminance_white: f32,
    pub min_hdr_adjustment: f32,
    pub max_hdr_adjustment: f32,
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(idx) => &line[..idx],
        None => line,
    }
}

fn parse_scalar_string(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    value
        .split_whitespace()
        .next()
        .map(|token| token.trim_matches('"').to_string())
        .filter(|token| !token.is_empty())
}

fn parse_vec3(value: &str) -> Option<Vec3> {
    let inner = value.trim().trim_start_matches('{').trim_end_matches('}');
    let mut it = inner.split_whitespace();
    let x: f32 = it.next()?.parse().ok()?;
    let y: f32 = it.next()?.parse().ok()?;
    let z: f32 = it.next()?.parse().ok()?;
    Some(Vec3 { x, y, z })
}

/// Parse `posteffect_volumes.txt` text into the selection table. This is a
/// tolerant Clausewitz-style scanner: it tracks brace depth to distinguish the
/// top-level `posteffect_values` presets from the volumes inside the
/// `posteffect_volumes { ... }` container, and honours `#` comments.
pub fn parse_posteffect_volume_table(text: &str) -> Result<PostEffectVolumeTable, String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Block {
        None,
        Values,
        HeightVolume,
        RegionVolume,
    }

    let mut table = PostEffectVolumeTable::default();
    let mut parse_errors: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut in_volumes_container = false;
    let mut container_depth: i32 = 0;
    let mut block = Block::None;
    let mut block_depth: i32 = 0;
    let mut cur_values = PostEffectValues::default();
    let mut cur_height = HeightVolume {
        name: String::new(),
        day: None,
        night: None,
        height: 0.0,
        fade_distance: 0.0,
    };
    let mut cur_region = RegionVolume {
        name: String::new(),
        day: None,
        night: None,
        day_winter: None,
        night_winter: None,
        position: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        size: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        fade_distance: 0.0,
    };

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        // A block opener nets more `{` than `}` on the line; an inline vector
        // like `position = { 3300 275 1700 }` nets zero and must fall through
        // to key/value parsing.
        let net_braces = line.matches('{').count() as i32 - line.matches('}').count() as i32;
        let opens_block = net_braces > 0;
        let key_part = line.split('=').next().unwrap_or("").trim();

        if opens_block {
            if key_part == "posteffect_values" && block == Block::None && !in_volumes_container {
                block = Block::Values;
                block_depth = depth;
                cur_values = PostEffectValues::default();
            } else if key_part == "posteffect_volumes" && !in_volumes_container {
                in_volumes_container = true;
                container_depth = depth;
            } else if in_volumes_container && key_part == "posteffect_height_volume" {
                block = Block::HeightVolume;
                block_depth = depth;
                cur_height = HeightVolume {
                    name: String::new(),
                    day: None,
                    night: None,
                    height: 0.0,
                    fade_distance: 0.0,
                };
            } else if in_volumes_container && key_part == "posteffect_volume" {
                block = Block::RegionVolume;
                block_depth = depth;
                cur_region = RegionVolume {
                    name: String::new(),
                    day: None,
                    night: None,
                    day_winter: None,
                    night_winter: None,
                    position: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    size: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    fade_distance: 0.0,
                };
            }
        } else if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match block {
                Block::Values => match key {
                    "name" => cur_values.name = parse_scalar_string(value).unwrap_or_default(),
                    "inherit" => cur_values.inherit = parse_scalar_string(value),
                    "lut" => cur_values.lut = parse_scalar_string(value),
                    "tonemap_middlegrey" => {
                        cur_values.tonemap_middlegrey =
                            value.split_whitespace().next().and_then(|t| t.parse().ok())
                    }
                    _ => {}
                },
                Block::HeightVolume => match key {
                    "name" => cur_height.name = parse_scalar_string(value).unwrap_or_default(),
                    "posteffect_values_day" => cur_height.day = parse_scalar_string(value),
                    "posteffect_values_night" => cur_height.night = parse_scalar_string(value),
                    "height" => match value.split_whitespace().next().map(str::parse::<f32>) {
                        Some(Ok(h)) => cur_height.height = h,
                        _ => parse_errors.push(format!(
                            "posteffect_height_volume '{}' has unparseable height {value:?}",
                            cur_height.name
                        )),
                    },
                    "fade_distance" => {
                        match value.split_whitespace().next().map(str::parse::<f32>) {
                            Some(Ok(d)) if d.is_finite() && d >= 0.0 => {
                                cur_height.fade_distance = d
                            }
                            _ => parse_errors.push(format!(
                                "posteffect_height_volume '{}' has unparseable fade_distance {value:?}",
                                cur_height.name
                            )),
                        }
                    }
                    _ => {}
                },
                Block::RegionVolume => match key {
                    "name" => cur_region.name = parse_scalar_string(value).unwrap_or_default(),
                    "posteffect_values_day" => cur_region.day = parse_scalar_string(value),
                    "posteffect_values_night" => cur_region.night = parse_scalar_string(value),
                    "posteffect_values_day_winter" => {
                        cur_region.day_winter = parse_scalar_string(value)
                    }
                    "posteffect_values_night_winter" => {
                        cur_region.night_winter = parse_scalar_string(value)
                    }
                    "position" => match parse_vec3(value) {
                        Some(v) => cur_region.position = v,
                        None => parse_errors.push(format!(
                            "posteffect_volume '{}' has unparseable position {value:?}",
                            cur_region.name
                        )),
                    },
                    "size" => match parse_vec3(value) {
                        Some(v) => cur_region.size = v,
                        None => parse_errors.push(format!(
                            "posteffect_volume '{}' has unparseable size {value:?}",
                            cur_region.name
                        )),
                    },
                    "fade_distance" => {
                        match value.split_whitespace().next().map(str::parse::<f32>) {
                            Some(Ok(d)) if d.is_finite() && d >= 0.0 => {
                                cur_region.fade_distance = d
                            }
                            _ => parse_errors.push(format!(
                                "posteffect_volume '{}' has unparseable fade_distance {value:?}",
                                cur_region.name
                            )),
                        }
                    }
                    _ => {}
                },
                Block::None => {}
            }
        }

        // Now count brace deltas on this line to update depth and close blocks.
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if block != Block::None && depth == block_depth {
                        match block {
                            Block::Values => {
                                if !cur_values.name.is_empty() {
                                    if !table.values.contains_key(&cur_values.name) {
                                        table.value_order.push(cur_values.name.clone());
                                    }
                                    table
                                        .values
                                        .insert(cur_values.name.clone(), cur_values.clone());
                                }
                            }
                            Block::HeightVolume => table.height_volumes.push(cur_height.clone()),
                            Block::RegionVolume => table.region_volumes.push(cur_region.clone()),
                            Block::None => {}
                        }
                        block = Block::None;
                    }
                    if in_volumes_container && depth == container_depth {
                        in_volumes_container = false;
                    }
                }
                _ => {}
            }
        }
    }

    if block != Block::None || in_volumes_container || depth != 0 {
        return Err(format!(
            "{POSTEFFECT_VOLUMES_REL} has an unbalanced/unclosed block (final brace depth {depth})"
        ));
    }
    if !parse_errors.is_empty() {
        return Err(format!(
            "{POSTEFFECT_VOLUMES_REL} has malformed entries: {}",
            parse_errors.join("; ")
        ));
    }
    if table.values.is_empty() {
        return Err(format!(
            "{POSTEFFECT_VOLUMES_REL} declared no posteffect_values presets"
        ));
    }
    Ok(table)
}

impl PostEffectVolumeTable {
    /// Resolve a value's `lut` following the `inherit` chain (guarding cycles).
    fn resolved_lut(&self, name: &str) -> Result<String, String> {
        let mut current = name;
        let mut guard = 0;
        loop {
            let value = self.values.get(current).ok_or_else(|| {
                format!("posteffect_values '{current}' referenced but not defined")
            })?;
            if let Some(lut) = &value.lut {
                return Ok(lut.clone());
            }
            match &value.inherit {
                Some(parent) => {
                    current = parent;
                    guard += 1;
                    if guard > 64 {
                        return Err(format!("posteffect_values inherit cycle at '{name}'"));
                    }
                }
                None => {
                    return Err(format!(
                        "posteffect_values '{name}' resolves to no lut (empty inherit chain)"
                    ))
                }
            }
        }
    }

    /// Resolve a value's `tonemap_middlegrey` following `inherit`.
    fn resolved_middle_grey(&self, name: &str) -> Result<f32, String> {
        let mut current = name;
        let mut guard = 0;
        loop {
            let value = self.values.get(current).ok_or_else(|| {
                format!("posteffect_values '{current}' referenced but not defined")
            })?;
            if let Some(mg) = value.tonemap_middlegrey {
                return Ok(mg);
            }
            match &value.inherit {
                Some(parent) => {
                    current = parent;
                    guard += 1;
                    if guard > 64 {
                        return Err(format!("posteffect_values inherit cycle at '{name}'"));
                    }
                }
                None => {
                    return Err(format!(
                        "posteffect_values '{name}' resolves to no tonemap_middlegrey"
                    ))
                }
            }
        }
    }

    fn region_contains(volume: &RegionVolume, camera: [f32; 3]) -> bool {
        // Boxes are centred on `position` with full-extent `size`.
        let half = [
            volume.size.x * 0.5,
            volume.size.y * 0.5,
            volume.size.z * 0.5,
        ];
        let center = [volume.position.x, volume.position.y, volume.position.z];
        (0..3).all(|axis| (camera[axis] - center[axis]).abs() <= half[axis])
    }

    fn pick_field(
        day: &Option<String>,
        night: &Option<String>,
        night_active: bool,
    ) -> Option<String> {
        if night_active {
            night.clone().or_else(|| day.clone())
        } else {
            day.clone()
        }
    }

    /// Select the winning `posteffect_values` name for the given frame state.
    pub fn select_value(&self, state: GradingSelectionState) -> Result<(String, String), String> {
        // Region volumes take priority when the camera is inside their 3D box.
        for volume in &self.region_volumes {
            if Self::region_contains(volume, state.camera) {
                let field = if state.winter {
                    if state.night {
                        volume
                            .night_winter
                            .clone()
                            .or_else(|| volume.night.clone())
                            .or_else(|| volume.day.clone())
                    } else {
                        volume.day_winter.clone().or_else(|| volume.day.clone())
                    }
                } else {
                    Self::pick_field(&volume.day, &volume.night, state.night)
                };
                if let Some(name) = field {
                    let reason = format!(
                        "camera {:?} inside region posteffect_volume '{}' (pos {:?} size {:?}); {} field",
                        state.camera,
                        volume.name,
                        [volume.position.x, volume.position.y, volume.position.z],
                        [volume.size.x, volume.size.y, volume.size.z],
                        if state.night { "night" } else { "day" }
                    );
                    return Ok((name, reason));
                }
            }
        }

        // Otherwise the height-volume stack: greatest height whose floor is at
        // or below the camera height.
        let mut best: Option<&HeightVolume> = None;
        for volume in &self.height_volumes {
            if state.camera[1] >= volume.height
                && best.map(|b| volume.height > b.height).unwrap_or(true)
            {
                best = Some(volume);
            }
        }
        let volume = best.ok_or_else(|| {
            format!(
                "no posteffect_height_volume floor at or below camera height {}",
                state.camera[1]
            )
        })?;
        let name = Self::pick_field(&volume.day, &volume.night, state.night).ok_or_else(|| {
            format!(
                "posteffect_height_volume '{}' has no {} value",
                volume.name,
                if state.night { "night" } else { "day" }
            )
        })?;
        let reason = format!(
            "camera height {} selects posteffect_height_volume '{}' (height {}); {} field",
            state.camera[1],
            volume.name,
            volume.height,
            if state.night { "night" } else { "day" }
        );
        Ok((name, reason))
    }

    /// Resolve the full grade (winning value name, LUT rel path, MiddleGrey).
    pub fn resolve_grade(&self, state: GradingSelectionState) -> Result<ResolvedGrade, String> {
        let (value_name, selection_reason) = self.select_value(state)?;
        let lut_rel_path = self.resolved_lut(&value_name)?;
        let middle_grey = self.resolved_middle_grey(&value_name)?;
        Ok(ResolvedGrade {
            value_name,
            selection_reason,
            lut_rel_path,
            middle_grey,
        })
    }

    /// PER-FRAME recompose (GRADE2, 2026-07-09): resolve the grade for the
    /// frame's camera, folding the height-volume `fade_distance` cross-fade.
    ///
    /// Law (all inputs shipped in posteffect_volumes.txt): the winning height
    /// volume `V` is the greatest `height <= camera.y` (unchanged); when the
    /// camera sits within `V.fade_distance` above `V.height` and a lower
    /// volume `L` exists, the frame's grade is the cross-fade
    /// `lerp(L, V, t)` with `t = (camera.y − V.height) / V.fade_distance`
    /// (shipped fade_distance: 200/200/0 at :352/:362/:372). This makes the
    /// grade CONTINUOUS across the volume floor: at `camera.y == V.height`
    /// the blend is exactly `L`, matching the selection just below the floor.
    /// The blended-LUT mechanism is the engine's own `lut_blender.shader`
    /// (`lerp(base, blend, vBlendFactor2)`, lut_blender.shader:94-104); the
    /// t-anchor (fade upward from the floor) is the documented law choice —
    /// the engine's C++ blend-factor source is UNTRACED. Region volumes keep
    /// hard containment (their spatial fade is UNTRACED and not implemented).
    ///
    /// Returns the winning grade plus, when a fade is active (`0 <= t < 1`),
    /// the lower-volume grade and the winning volume's blend weight `t`.
    pub fn resolve_grade_per_frame(
        &self,
        state: GradingSelectionState,
    ) -> Result<PerFrameGrade, String> {
        let primary = self.resolve_grade(state)?;
        // Region-volume wins bypass the height stack (hard containment).
        for volume in &self.region_volumes {
            if Self::region_contains(volume, state.camera) {
                return Ok(PerFrameGrade {
                    primary,
                    cross_fade: None,
                });
            }
        }
        // Winning height volume + the next-lower volume in the stack.
        let mut winner: Option<&HeightVolume> = None;
        for volume in &self.height_volumes {
            if state.camera[1] >= volume.height
                && winner.map(|b| volume.height > b.height).unwrap_or(true)
            {
                winner = Some(volume);
            }
        }
        let Some(winner) = winner else {
            return Ok(PerFrameGrade {
                primary,
                cross_fade: None,
            });
        };
        let mut lower: Option<&HeightVolume> = None;
        for volume in &self.height_volumes {
            if volume.height < winner.height
                && lower.map(|b| volume.height > b.height).unwrap_or(true)
            {
                lower = Some(volume);
            }
        }
        let (Some(lower), true) = (lower, winner.fade_distance > 0.0) else {
            return Ok(PerFrameGrade {
                primary,
                cross_fade: None,
            });
        };
        let t = (state.camera[1] - winner.height) / winner.fade_distance;
        if !(0.0..1.0).contains(&t) {
            return Ok(PerFrameGrade {
                primary,
                cross_fade: None,
            });
        }
        let lower_name =
            Self::pick_field(&lower.day, &lower.night, state.night).ok_or_else(|| {
                format!(
                    "posteffect_height_volume '{}' has no {} value for the cross-fade",
                    lower.name,
                    if state.night { "night" } else { "day" }
                )
            })?;
        let lower_grade = ResolvedGrade {
            value_name: lower_name.clone(),
            selection_reason: format!(
                "cross-fade lower volume '{}' (height {}) under '{}' (height {}, fade_distance {}), t = {t:.4}",
                lower.name, lower.height, winner.name, winner.height, winner.fade_distance
            ),
            lut_rel_path: self.resolved_lut(&lower_name)?,
            middle_grey: self.resolved_middle_grey(&lower_name)?,
        };
        Ok(PerFrameGrade {
            primary,
            cross_fade: Some((lower_grade, t)),
        })
    }
}

/// A per-frame grade: the winning volume's grade plus an optional active
/// height cross-fade (lower-volume grade + the WINNER's blend weight `t`).
#[derive(Clone, Debug, PartialEq)]
pub struct PerFrameGrade {
    pub primary: ResolvedGrade,
    pub cross_fade: Option<(ResolvedGrade, f32)>,
}

/// Load and parse `posteffect_volumes.txt` through the resolver (mod-aware).
pub fn load_posteffect_volume_table(
    resolver: &GameFileResolver,
) -> Result<PostEffectVolumeTable, String> {
    let path = resolver
        .resolve_required(POSTEFFECT_VOLUMES_REL, "posteffect volume table")
        .map_err(|e| e.to_string())?;
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_posteffect_volume_table(&text)
}

/// Read the grading-relevant graphics defines through the resolver.
pub fn load_graphics_grading_defines(
    resolver: &GameFileResolver,
) -> Result<GraphicsGradingDefines, String> {
    let path = resolver
        .resolve_required(GRAPHICS_DEFINES_REL, "graphics defines")
        .map_err(|e| e.to_string())?;
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_graphics_grading_defines(&text)
}

/// Parse `TONE_MAP_MIDDLE_GREY`, `TONE_MAP_LUMINANCE_WHITE`,
/// `MIN_HDR_ADJUSTMENT`, `MAX_HDR_ADJUSTMENT` from a Lua defines file. These are
/// the engine-wide adaptation constants; the per-volume `tonemap_middlegrey`
/// overrides `TONE_MAP_MIDDLE_GREY` at runtime, so the volume value wins for the
/// composed CB while these defines drive the (engine-side) adaptation clamps.
pub fn parse_graphics_grading_defines(text: &str) -> Result<GraphicsGradingDefines, String> {
    let find = |key: &str| -> Option<f32> {
        for raw in text.lines() {
            let line = raw.trim();
            if line.starts_with("--") {
                continue;
            }
            let Some((lhs, rhs)) = line.split_once('=') else {
                continue;
            };
            if lhs.trim() != key {
                continue;
            }
            // rhs like " 0.5,     --comment"
            let rhs = rhs.split("--").next().unwrap_or(rhs);
            let rhs = rhs.trim().trim_end_matches(',').trim();
            if let Ok(v) = rhs.parse::<f32>() {
                return Some(v);
            }
        }
        None
    };
    let tone_map_middle_grey = find("TONE_MAP_MIDDLE_GREY")
        .ok_or_else(|| format!("{GRAPHICS_DEFINES_REL} missing TONE_MAP_MIDDLE_GREY"))?;
    let tone_map_luminance_white = find("TONE_MAP_LUMINANCE_WHITE")
        .ok_or_else(|| format!("{GRAPHICS_DEFINES_REL} missing TONE_MAP_LUMINANCE_WHITE"))?;
    let min_hdr_adjustment = find("MIN_HDR_ADJUSTMENT")
        .ok_or_else(|| format!("{GRAPHICS_DEFINES_REL} missing MIN_HDR_ADJUSTMENT"))?;
    let max_hdr_adjustment = find("MAX_HDR_ADJUSTMENT")
        .ok_or_else(|| format!("{GRAPHICS_DEFINES_REL} missing MAX_HDR_ADJUSTMENT"))?;
    Ok(GraphicsGradingDefines {
        tone_map_middle_grey,
        tone_map_luminance_white,
        min_hdr_adjustment,
        max_hdr_adjustment,
    })
}

/// Adaptation law. Cited from the shipped shaders (not engine-internal):
///   * `gfx/FX/restorescene.shader` `Exposure()` under `#define
///     ADJUSTED_EXPOSURE`: `return inColor * (MiddleGrey / AverageLuminance)`.
///   * `gfx/FX/downsample_luminance.shader` computes the `AverageLuminance`
///     texel: `vCurrentLuminance = clamp(exp(mean(log-luminance)),
///     MinHdr_MaxHdr.x, MinHdr_MaxHdr.y)` then eases `LastLuminance` toward it
///     (tau/delta-time from `HDR_ADJUSTMENT_SPEED`).
///   * `common/defines/00_graphics.lua`: `MIN_HDR_ADJUSTMENT` / `MAX_HDR_ADJUSTMENT`
///     are the clamp bounds `MinHdr_MaxHdr`.
///
/// The replay has no per-frame luminance readback, so we use the clamp floor
/// `MIN_HDR_ADJUSTMENT` as the steady-state average luminance. The captured
/// frame's AverageLuminance texel was exactly 0.5 == `MIN_HDR_ADJUSTMENT`, i.e.
/// the geometric-mean scene luminance was at or below the clamp floor that
/// frame (NOT an assertion about scene brightness — only that `exp(vSum)` landed
/// at/under 0.5). Exposure scale becomes `MiddleGrey / MIN_HDR_ADJUSTMENT`.
pub fn steady_state_average_luminance(defines: &GraphicsGradingDefines) -> f32 {
    defines.min_hdr_adjustment
}

/// Convert a decoded 1024x32 RGB LUT into RGBA8 with opaque alpha, matching the
/// immutable texture the renderer uploads (and the swizzled captured cube).
pub fn lut_to_rgba8(lut: &ColorCorrectionLut) -> Vec<u8> {
    let mut out = Vec::with_capacity(lut.texels.len() * 4);
    for texel in &lut.texels {
        out.push((texel[0] * 255.0).round().clamp(0.0, 255.0) as u8);
        out.push((texel[1] * 255.0).round().clamp(0.0, 255.0) as u8);
        out.push((texel[2] * 255.0).round().clamp(0.0, 255.0) as u8);
        out.push(0xFF);
    }
    out
}

/// Cross-fade two decoded LUTs texel-wise and quantize like [`lut_to_rgba8`].
/// The blend law is the engine's own `lut_blender.shader` pixel shader
/// (`lerp(base, blend, vBlendFactor2)`, lut_blender.shader:94-104) evaluated
/// on the CPU; `upper_blend` is the weight of `upper` (the engine's
/// `vBlendFactor2`). At `upper_blend == 0.0`/`1.0` this reduces bit-exactly to
/// `lut_to_rgba8(lower)`/`lut_to_rgba8(upper)` (`a + (b-a)*0 == a`).
pub fn blend_luts_to_rgba8(
    lower: &ColorCorrectionLut,
    upper: &ColorCorrectionLut,
    upper_blend: f32,
) -> Result<Vec<u8>, String> {
    if lower.texels.len() != upper.texels.len()
        || lower.width != upper.width
        || lower.height != upper.height
    {
        return Err(format!(
            "cross-fade LUT shape mismatch: {}x{} ({} texels) vs {}x{} ({} texels)",
            lower.width,
            lower.height,
            lower.texels.len(),
            upper.width,
            upper.height,
            upper.texels.len()
        ));
    }
    if !(0.0..=1.0).contains(&upper_blend) {
        return Err(format!(
            "cross-fade blend factor {upper_blend} out of [0,1]"
        ));
    }
    let mut out = Vec::with_capacity(lower.texels.len() * 4);
    for (lo, hi) in lower.texels.iter().zip(upper.texels.iter()) {
        for channel in 0..3 {
            let value = lo[channel] + (hi[channel] - lo[channel]) * upper_blend;
            out.push((value * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        out.push(0xFF);
    }
    Ok(out)
}

/// PER-FRAME files-derived grade (GRADE2): like [`file_derived_grade`] but
/// recomposed for the frame's own camera, folding the height-volume
/// `fade_distance` cross-fade (blended LUT bytes + lerped MiddleGrey). With no
/// active fade this is exactly the [`file_derived_grade`] composition, so the
/// once-at-construction grade at the captured camera is reproduced
/// byte-identically (gated).
pub fn file_derived_grade_per_frame(
    resolver: &GameFileResolver,
    state: GradingSelectionState,
    measured_average_luminance: Option<f32>,
) -> Result<FileDerivedGrade, String> {
    let table = load_posteffect_volume_table(resolver)?;
    let defines = load_graphics_grading_defines(resolver)?;
    let per_frame = table.resolve_grade_per_frame(state)?;
    let resolved = per_frame.primary.clone();
    let lut = color_correction_lut_from_resolver(resolver, &resolved.lut_rel_path)
        .map_err(|e| e.to_string())?;
    if lut.width != 1024 || lut.height != 32 {
        return Err(format!(
            "composed LUT {} is {}x{}, expected 1024x32",
            resolved.lut_rel_path, lut.width, lut.height
        ));
    }
    let (color_cube_rgba8, resolved, fade_note) = match &per_frame.cross_fade {
        None => (lut_to_rgba8(&lut), resolved, String::new()),
        Some((lower_grade, t)) => {
            let lower_lut = color_correction_lut_from_resolver(resolver, &lower_grade.lut_rel_path)
                .map_err(|e| e.to_string())?;
            if lower_lut.width != 1024 || lower_lut.height != 32 {
                return Err(format!(
                    "cross-fade LUT {} is {}x{}, expected 1024x32",
                    lower_grade.lut_rel_path, lower_lut.width, lower_lut.height
                ));
            }
            let bytes = blend_luts_to_rgba8(&lower_lut, &lut, *t)?;
            // MiddleGrey rides the same lerp as the LUT (one PostEffectBase
            // state per frame).
            let blended = ResolvedGrade {
                value_name: resolved.value_name.clone(),
                selection_reason: format!(
                    "{}; ACTIVE height cross-fade: {}",
                    resolved.selection_reason, lower_grade.selection_reason
                ),
                lut_rel_path: resolved.lut_rel_path.clone(),
                middle_grey: lower_grade.middle_grey
                    + (resolved.middle_grey - lower_grade.middle_grey) * t,
            };
            let note = format!(
                "; fade t={t:.4} from '{}' (mg {:.3})",
                lower_grade.value_name, lower_grade.middle_grey
            );
            (bytes, blended, note)
        }
    };
    let average_luminance =
        measured_average_luminance.unwrap_or_else(|| steady_state_average_luminance(&defines));
    if !(average_luminance.is_finite() && average_luminance > 0.0) {
        return Err(format!(
            "adaptation average luminance {average_luminance} is not a positive finite value"
        ));
    }
    let source_note = format!(
        "files-first per-frame grade: {} -> lut {} (mod-resolver), MiddleGrey {:.3}, AvgLum {:.3}{fade_note}",
        resolved.selection_reason,
        resolved.lut_rel_path,
        resolved.middle_grey,
        average_luminance,
    );
    Ok(FileDerivedGrade {
        resolved,
        lut,
        color_cube_rgba8,
        average_luminance,
        source_note,
    })
}

/// Compose the full files-derived grade for a frame: select the volume, load the
/// LUT tga and defines through the resolver, and compute the adaptation. Fails
/// closed (naming the gap) if any file or key is missing for the modset.
pub fn file_derived_grade(
    resolver: &GameFileResolver,
    state: GradingSelectionState,
    measured_average_luminance: Option<f32>,
) -> Result<FileDerivedGrade, String> {
    let table = load_posteffect_volume_table(resolver)?;
    let defines = load_graphics_grading_defines(resolver)?;
    let resolved = table.resolve_grade(state)?;
    let lut = color_correction_lut_from_resolver(resolver, &resolved.lut_rel_path)
        .map_err(|e| e.to_string())?;
    if lut.width != 1024 || lut.height != 32 {
        return Err(format!(
            "composed LUT {} is {}x{}, expected 1024x32",
            resolved.lut_rel_path, lut.width, lut.height
        ));
    }
    let color_cube_rgba8 = lut_to_rgba8(&lut);
    let average_luminance =
        measured_average_luminance.unwrap_or_else(|| steady_state_average_luminance(&defines));
    if !(average_luminance.is_finite() && average_luminance > 0.0) {
        return Err(format!(
            "adaptation average luminance {average_luminance} is not a positive finite value"
        ));
    }
    let source_note = format!(
        "files-first grade: {} -> lut {} (mod-resolver), MiddleGrey {:.3} (posteffect_volumes.txt), AvgLum {:.3} ({}); define TONE_MAP_MIDDLE_GREY {:.3}",
        resolved.selection_reason,
        resolved.lut_rel_path,
        resolved.middle_grey,
        average_luminance,
        if measured_average_luminance.is_some() {
            "measured"
        } else {
            "MIN_HDR_ADJUSTMENT steady-state floor"
        },
        defines.tone_map_middle_grey,
    );
    Ok(FileDerivedGrade {
        resolved,
        lut,
        color_cube_rgba8,
        average_luminance,
        source_note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    fn sample_table() -> PostEffectVolumeTable {
        let text = r#"
posteffect_values = {
    name = standard
    lut = "gfx/world/colorcorrection.tga"
    tonemap_middlegrey = 0.55
}
posteffect_values = {
    name = default
    inherit = standard
    lut = "gfx/world/colorcorrection_standard.tga"
}
posteffect_values = {
    name = max_distance
    inherit = standard
    lut = "gfx/world/colorcorrection_farrange.tga"
    #tonemap_middlegrey = 0.65
}
posteffect_values = {
    name = green
    inherit = standard
    lut = "gfx/world/colorcorrection_green.tga"
    tonemap_middlegrey = 0.3
}
posteffect_values = {
    name = mid_distance
    inherit = standard
    lut = "gfx/world/colorcorrection_midrange.tga"
    tonemap_middlegrey = 0.45
}
posteffect_volumes = {
    posteffect_height_volume = {
        name = "Base Highest"
        posteffect_values_day = max_distance
        posteffect_values_night = max_distance_night
        height = 1600
        fade_distance = 200
    }
    posteffect_height_volume = {
        name = "Base Medium"
        posteffect_values_day = mid_distance
        posteffect_values_night = mid_distance_night
        height = 900
        fade_distance = 200
    }
    posteffect_height_volume = {
        name = "Base"
        posteffect_values_day = default
        posteffect_values_night = default_night
        height = 0
        fade_distance = 0
    }
    posteffect_volume = {
        name = EuropeandRussia
        posteffect_values_day = green
        posteffect_values_night = green_night
        position = { 3300.0 275.0 1700.0 }
        size = { 1300.0 800.0 700.0 }
        fade_distance = 125.0
    }
}
"#;
        parse_posteffect_volume_table(text).expect("parse sample table")
    }

    #[test]
    fn parses_values_inherit_and_commented_middlegrey() {
        let table = sample_table();
        assert_eq!(
            table.resolved_lut("max_distance").unwrap(),
            "gfx/world/colorcorrection_farrange.tga"
        );
        // max_distance's tonemap_middlegrey is commented out -> inherits 0.55.
        assert!((table.resolved_middle_grey("max_distance").unwrap() - 0.55).abs() < 1e-6);
        // default inherits standard's 0.55 too but overrides the lut.
        assert_eq!(
            table.resolved_lut("default").unwrap(),
            "gfx/world/colorcorrection_standard.tga"
        );
        assert!((table.resolved_middle_grey("default").unwrap() - 0.55).abs() < 1e-6);
        // green overrides middlegrey.
        assert!((table.resolved_middle_grey("green").unwrap() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn high_altitude_daytime_selects_base_highest_max_distance() {
        // The restore_gate_1 capture state: camera (2816, 2958, 1400), day.
        // 2958 > every region box top; EuropeandRussia spans y -125..675.
        let table = sample_table();
        let grade = table
            .resolve_grade(GradingSelectionState {
                camera: [2816.0, 2958.0, 1400.0],
                night: false,
                winter: false,
            })
            .expect("resolve grade");
        assert_eq!(grade.value_name, "max_distance");
        assert_eq!(grade.lut_rel_path, "gfx/world/colorcorrection_farrange.tga");
        assert!((grade.middle_grey - 0.55).abs() < 1e-6);
        assert!(grade.selection_reason.contains("Base Highest"));
    }

    #[test]
    fn low_altitude_inside_region_box_selects_region_value() {
        // Same XZ but at a low camera height inside the EuropeandRussia box.
        let table = sample_table();
        let grade = table
            .resolve_grade(GradingSelectionState {
                camera: [3300.0, 300.0, 1700.0],
                night: false,
                winter: false,
            })
            .expect("resolve grade");
        assert_eq!(grade.value_name, "green");
        assert!(grade.selection_reason.contains("EuropeandRussia"));
    }

    #[test]
    fn low_altitude_outside_region_selects_base_height_volume() {
        let table = sample_table();
        let grade = table
            .resolve_grade(GradingSelectionState {
                camera: [200.0, 100.0, 200.0],
                night: false,
                winter: false,
            })
            .expect("resolve grade");
        assert_eq!(grade.value_name, "default");
        assert!(grade.selection_reason.contains("Base"));
    }

    #[test]
    fn parses_graphics_defines() {
        let text = r#"
        BRIGHT_THRESHOLD = 0.4,
        MIN_HDR_ADJUSTMENT = 0.5,     --comment
        MAX_HDR_ADJUSTMENT = 1.0,      --comment
        HDR_ADJUSTMENT_SPEED = 15.0, -- 6
        TONE_MAP_MIDDLE_GREY = 0.5, --0.7
        TONE_MAP_LUMINANCE_WHITE = 1.0, --
        "#;
        let defines = parse_graphics_grading_defines(text).expect("parse defines");
        assert!((defines.tone_map_middle_grey - 0.5).abs() < 1e-6);
        assert!((defines.tone_map_luminance_white - 1.0).abs() < 1e-6);
        assert!((defines.min_hdr_adjustment - 0.5).abs() < 1e-6);
        assert!((defines.max_hdr_adjustment - 1.0).abs() < 1e-6);
        assert!((steady_state_average_luminance(&defines) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gate_height_sweep_cross_fades_between_adjacent_volumes_with_file_values() {
        // GRADE2 item gate: sweep the camera height across the Base Medium
        // (900, fade 200) and Base Highest (1600, fade 200) floors — the same
        // fade_distance values the shipped file carries at
        // posteffect_volumes.txt:352/362/372 — and check the cross-fade uses
        // ONLY those parsed file values.
        let table = sample_table();
        let at = |y: f32| {
            table
                .resolve_grade_per_frame(GradingSelectionState {
                    camera: [200.0, y, 200.0],
                    night: false,
                    winter: false,
                })
                .expect("per-frame grade")
        };
        // Below every fade: pure lowest volume.
        let low = at(500.0);
        assert_eq!(low.primary.value_name, "default");
        assert!(low.cross_fade.is_none());
        // At the Base Medium floor: winner flips to mid_distance with t = 0 —
        // the blend is exactly the lower volume, so the grade is CONTINUOUS.
        let floor = at(900.0);
        assert_eq!(floor.primary.value_name, "mid_distance");
        let (lower, t) = floor.cross_fade.expect("fade active at the floor");
        assert_eq!(lower.value_name, "default");
        assert!(t.abs() < 1e-6, "t at floor = {t}");
        // Mid-fade: t = (1000 - 900) / 200 = 0.5 (file height + file fade).
        let mid = at(1000.0);
        let (lower, t) = mid.cross_fade.expect("fade active inside the window");
        assert_eq!(mid.primary.value_name, "mid_distance");
        assert_eq!(lower.value_name, "default");
        assert!((t - 0.5).abs() < 1e-6, "t mid-window = {t}");
        // Past the fade window: pure mid_distance.
        let past = at(1101.0);
        assert_eq!(past.primary.value_name, "mid_distance");
        assert!(past.cross_fade.is_none());
        // Second boundary: 1650 -> t = (1650 - 1600)/200 = 0.25 into
        // max_distance from mid_distance.
        let upper = at(1650.0);
        assert_eq!(upper.primary.value_name, "max_distance");
        let (lower, t) = upper.cross_fade.expect("fade active above 1600");
        assert_eq!(lower.value_name, "mid_distance");
        assert!((t - 0.25).abs() < 1e-6, "t = {t}");
        // The captured camera height (2958) is far above 1600 + 200: NO fade.
        let captured = at(2958.0);
        assert_eq!(captured.primary.value_name, "max_distance");
        assert!(
            captured.cross_fade.is_none(),
            "no height fade may be active at the captured camera"
        );
        // MiddleGrey lerp on the file values: mid-window blend at t = 0.5
        // between default (0.55) and mid_distance (0.45) = 0.5.
        let (lower, t) = at(1000.0).cross_fade.unwrap();
        let blended = lower.middle_grey
            + (table.resolved_middle_grey("mid_distance").unwrap() - lower.middle_grey) * t;
        assert!((blended - 0.5).abs() < 1e-6, "blended MiddleGrey {blended}");
    }

    #[test]
    fn blend_luts_reduces_to_endpoints_bit_exactly() {
        let make = |v: f32| ColorCorrectionLut {
            width: 2,
            height: 1,
            slice_size: 1,
            texels: vec![[v, v * 0.5, 1.0 - v], [0.25, v, 0.75]],
        };
        let lower = make(0.2);
        let upper = make(0.9);
        assert_eq!(
            blend_luts_to_rgba8(&lower, &upper, 0.0).unwrap(),
            lut_to_rgba8(&lower),
            "t = 0 must be the lower LUT bit-exactly"
        );
        assert_eq!(
            blend_luts_to_rgba8(&lower, &upper, 1.0).unwrap(),
            lut_to_rgba8(&upper),
            "t = 1 must be the upper LUT bit-exactly"
        );
        // Interior blend follows lut_blender.shader's lerp then quantizes.
        let half = blend_luts_to_rgba8(&lower, &upper, 0.5).unwrap();
        assert_eq!(
            half[0],
            ((0.2f32 + (0.9 - 0.2) * 0.5) * 255.0).round() as u8
        );
        // Shape mismatch fails closed.
        let tiny = ColorCorrectionLut {
            width: 1,
            height: 1,
            slice_size: 1,
            texels: vec![[0.0, 0.0, 0.0]],
        };
        assert!(blend_luts_to_rgba8(&lower, &tiny, 0.5).is_err());
    }

    #[test]
    fn gate_per_frame_recompose_is_byte_identical_at_captured_camera() {
        // GRADE2 byte-noop proof: at the captured restore_gate_1 camera
        // (2816, 2958, 1400) the per-frame recompose must reproduce the
        // once-at-construction grade BYTE-IDENTICALLY (no fade is active at
        // 2958, so the paths must agree exactly).
        let game_dir = std::path::PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping per-frame grade byte-noop gate: vanilla install absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let state = GradingSelectionState {
            camera: [2816.0, 2958.0, 1400.0],
            night: false,
            winter: false,
        };
        let construction = file_derived_grade(&resolver, state, None).expect("construction grade");
        let per_frame =
            file_derived_grade_per_frame(&resolver, state, None).expect("per-frame grade");
        assert_eq!(
            per_frame.color_cube_rgba8, construction.color_cube_rgba8,
            "per-frame LUT bytes must equal the construction grade at the captured camera"
        );
        assert_eq!(
            per_frame.resolved.value_name,
            construction.resolved.value_name
        );
        assert_eq!(
            per_frame.resolved.middle_grey.to_bits(),
            construction.resolved.middle_grey.to_bits(),
            "MiddleGrey must be bit-identical"
        );
        assert_eq!(
            per_frame.average_luminance.to_bits(),
            construction.average_luminance.to_bits()
        );
        // And the shipped file's own height volumes drive a REAL cross-fade
        // just above the Base Highest floor (1600 + 100 -> t = 0.5 with the
        // shipped fade_distance 200 at posteffect_volumes.txt:352).
        let fade_state = GradingSelectionState {
            camera: [2816.0, 1700.0, 1400.0],
            night: false,
            winter: false,
        };
        let table = load_posteffect_volume_table(&resolver).expect("table");
        let faded = table
            .resolve_grade_per_frame(fade_state)
            .expect("faded grade");
        let (lower, t) = faded
            .cross_fade
            .expect("shipped fade_distance 200 must be active at height 1700");
        assert_eq!(faded.primary.value_name, "max_distance");
        assert_eq!(lower.value_name, "mid_distance");
        assert!((t - 0.5).abs() < 1e-6, "shipped-file fade t {t}");
        println!(
            "per-frame grade byte-noop gate: captured camera identical; shipped fade t=0.5 at 1700"
        );
    }

    #[test]
    fn missing_value_fails_closed() {
        let table = sample_table();
        let err = table
            .resolved_lut("does_not_exist")
            .expect_err("must fail closed");
        assert!(err.contains("not defined"), "{err}");
    }

    #[test]
    fn parser_rejects_unclosed_block() {
        let text = r#"
posteffect_values = {
    name = standard
    lut = "gfx/world/colorcorrection.tga"
    tonemap_middlegrey = 0.55
"#; // missing closing brace
        let err = parse_posteffect_volume_table(text).expect_err("unclosed block must fail");
        assert!(
            err.contains("unbalanced") || err.contains("unclosed"),
            "{err}"
        );
    }

    #[test]
    fn parser_rejects_malformed_height() {
        let text = r#"
posteffect_values = {
    name = standard
    lut = "gfx/world/colorcorrection.tga"
    tonemap_middlegrey = 0.55
}
posteffect_volumes = {
    posteffect_height_volume = {
        name = "Bad"
        posteffect_values_day = standard
        height = not_a_number
    }
}
"#;
        let err = parse_posteffect_volume_table(text).expect_err("malformed height must fail");
        assert!(err.contains("unparseable height"), "{err}");
    }

    #[test]
    fn parser_ignores_posteffect_values_inside_volumes_container() {
        // A mod that nests `posteffect_values` inside `posteffect_volumes {}`
        // must NOT register it as a global preset.
        let text = r#"
posteffect_values = {
    name = standard
    lut = "gfx/world/colorcorrection.tga"
    tonemap_middlegrey = 0.55
}
posteffect_volumes = {
    posteffect_values = {
        name = sneaky
        lut = "gfx/world/colorcorrection_green.tga"
    }
    posteffect_height_volume = {
        name = "Base"
        posteffect_values_day = standard
        height = 0
    }
}
"#;
        let table = parse_posteffect_volume_table(text).expect("parse");
        assert!(
            table.resolved_lut("sneaky").is_err(),
            "nested posteffect_values must not be a global preset"
        );
    }

    #[test]
    fn gate_vanilla_law_reproduces_captured_selection() {
        let game_dir = std::path::PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla grading-law gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let grade = file_derived_grade(
            &resolver,
            GradingSelectionState {
                camera: [2816.0, 2958.0, 1400.0],
                night: false,
                winter: false,
            },
            None,
        )
        .expect("compose files-first grade");
        // Law selects the far-range cube for this high-altitude day frame.
        assert_eq!(
            grade.resolved.lut_rel_path,
            "gfx/world/colorcorrection_farrange.tga"
        );
        assert!((grade.resolved.middle_grey - 0.55).abs() < 1e-6);
        // Steady-state adaptation floor is MIN_HDR_ADJUSTMENT = 0.5.
        assert!((grade.average_luminance - 0.5).abs() < 1e-6);
        assert_eq!(grade.color_cube_rgba8.len(), 1024 * 32 * 4);
        assert!(grade.color_cube_rgba8.chunks_exact(4).all(|t| t[3] == 0xFF));
        println!("vanilla grading-law gate: {}", grade.source_note);
    }

    // The live restore capture (has the shipped composed ColorCube SRV payload).
    const RESTORE_CAPTURE_COLOR_CUBE: &str = concat!(
        r"C:\Users\Example\AppData\Local\hoi4_captures\restore_gate_1\",
        r"restore_frame_draw_map_frame_shared_payload_ps_srv_ColorCube_fb9c4f759c5171c9.d3d11"
    );

    /// END-TO-END LUT FIT GATE: the files-composed LUT must equal the captured
    /// engine ColorCube byte-exactly after BGRX -> RGBA swizzle. This is the
    /// files-first law's regression anchor against real GPU evidence.
    #[test]
    fn gate_files_lut_matches_captured_color_cube_byte_exact() {
        let game_dir = std::path::PathBuf::from(VANILLA_GAME_DIR);
        let cube_path = std::path::PathBuf::from(RESTORE_CAPTURE_COLOR_CUBE);
        if !game_dir.is_dir() || !cube_path.is_file() {
            eprintln!(
                "skipping LUT fit gate: game dir or captured ColorCube absent ({})",
                RESTORE_CAPTURE_COLOR_CUBE
            );
            return;
        }
        let captured_bgrx = std::fs::read(&cube_path).expect("read captured ColorCube payload");
        assert_eq!(
            captured_bgrx.len(),
            1024 * 32 * 4,
            "captured cube byte count"
        );
        // Captured DXGI 88 (B8G8R8X8) -> RGBA8 opaque, matching lut_to_rgba8.
        let mut captured_rgba = captured_bgrx.clone();
        for texel in captured_rgba.chunks_exact_mut(4) {
            texel.swap(0, 2);
            texel[3] = 0xFF;
        }

        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let grade = file_derived_grade(
            &resolver,
            GradingSelectionState {
                camera: [2816.0, 2958.0, 1400.0],
                night: false,
                winter: false,
            },
            None,
        )
        .expect("compose files-first grade");

        let mut max_delta = 0u8;
        for (cap, file) in captured_rgba
            .chunks_exact(4)
            .zip(grade.color_cube_rgba8.chunks_exact(4))
        {
            for channel in 0..3 {
                max_delta = max_delta.max(cap[channel].abs_diff(file[channel]));
            }
        }
        assert_eq!(
            max_delta, 0,
            "files-composed LUT ({}) must match captured ColorCube byte-exactly",
            grade.resolved.lut_rel_path
        );
        println!(
            "LUT fit gate: composed {} matches captured ColorCube byte-exactly (max delta 0)",
            grade.resolved.lut_rel_path
        );
    }
}
