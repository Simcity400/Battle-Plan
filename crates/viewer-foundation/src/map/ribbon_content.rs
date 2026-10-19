//! Immutable ribbon content and concrete draw records shared by counter/raid producers and world.
use crate::map::unit_pose::PLANE_UNRECORDED;
use crate::{
    errors::invalid,
    map::{name_draw::Placements, ribbon_fixed::Smoothing, textures::load_file},
};
use bytemuck::{Pod, Zeroable};
use installed_content_core::{
    ResolvedLayers, defines::Defines, defines::Defines as ContentDefines, parser, parser::Value,
};
use installed_content_image::{dds, native::Decode};
use std::{collections::BTreeMap, io};
pub const MAPARROWS: &str = "gfx/maparrows/maparrows.txt";
pub const GROUP_SECTION_TYPE_NAME: &str = "ag_child_defence_line";
/// The arrow type every trade-route line draws with.
pub const TRADE_ROUTE_TYPE_NAME: &str = "traderoute";

/// The stride is 32 bytes. XYZ and UV4 are written; bytes 12 through 15 are skipped by the
/// declaration and do not supply POSITION.w.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub padding: u32,
    pub uv: [f32; 4],
}

/// Scene calls : mode0 before entities, then mode3 and mode1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Shadow,
    Main,
    Seethrough,
    Late,
}

impl Mode {
    pub fn admits(self, draw_last: bool) -> bool {
        match self {
            Self::Main => !draw_last,
            Self::Late | Self::Shadow => draw_last,
            Self::Seethrough => true,
        }
    }

    pub fn lighting(self) -> f32 {
        // Modes 1/2 zero both shadow and normal factors.
        if matches!(self, Self::Late | Self::Shadow) {
            0.0
        } else {
            1.0
        }
    }
}

/// Keep each content refusal separate: an inactive/earlier-false branch
/// reaches none of the later defines. Operands belong to the recorded content.
#[derive(Debug)]
pub struct ExecutionDefines {
    pub naval_days: Result<i32, String>,
    pub air_days: Result<i32, String>,
    pub unit_weight: Result<i64, String>,
    pub minimum_planes: Result<i32, String>,
}

impl ExecutionDefines {
    pub fn read(source: &ContentDefines) -> Self {
        Self {
            naval_days: source
                .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
                .map_err(|error| error.to_string()),
            air_days: source
                .int32("NAir", "AIR_INVASION_PREPARE_DAYS")
                .map_err(|error| error.to_string()),
            unit_weight: source
                .fixed5("NAir", "BASE_UNIT_WEIGHT_IN_TRANSPORT_PLANES")
                .map_err(|error| error.to_string()),
            minimum_planes: source
                .int32("NAir", "MIN_PLANE_COUNT_PARADROP")
                .map_err(|error| error.to_string()),
        }
    }
}

/// Both ribbons of an army-group child use the parent's callback result.
/// Inactive construction is also the generic arrow/raid constructor state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionState {
    pub active: bool,
    pub prerequisites: bool,
}

impl ExecutionState {
    /// The exact f32 epsilon. Inner execution FX remains active even when unmet
    /// prerequisites stop texture movement.
    pub fn shader_terms(self, authored_speed: f32, outer: bool) -> [f32; 2] {
        let speed = if authored_speed.abs() <= f32::from_bits(0x38d1_b717)
            || self.active && !self.prerequisites
        {
            0.0
        } else {
            authored_speed
        };
        [speed, if self.active && !outer { 1.0 } else { 0.0 }]
    }
}

// ------------------------------------------------------------------ defines

/// The `NDefines.NGraphics` values the plan geometry and text read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrderDefines {
    pub order_move: Smoothing,
    pub order_front: Smoothing,
    pub root_front: Smoothing,
    pub snapped_off_front: Smoothing,
    /// `ROOT_FRONT_OFFSET`, fixed5.
    pub root_front_offset: i64,
    /// Content budgets for the local control-list pruning of an offset
    /// bound order and an unbound front, respectively.
    pub root_front_max_intersection_tests_order: i32,
    pub root_front_max_intersection_tests_front: i32,
    /// `ORDER_FRONT_MAX_OFFSETS`.
    pub max_offsets: u8,
    /// `ARROW_EXT_TEXT_POINTS_LIMIT`.
    pub ext_text_points_limit: i32,
    /// `VIRTUAL_BATTLEPLANS_COLOR`.
    pub virtual_color: [f32; 4],
    /// `ALLIED_BATTLEPLANS_COLOR`, the ordinary foreign-owner callback tint.
    pub allied_color: [f32; 4],
    /// `ARROW_PARADROP_HEIGHT_TO_LEN` and `ARROW_PARADROP_HEIGHT_MAX`, the
    /// paradrop arc's lift per unit of length and its ceiling.
    pub paradrop_height_to_len: f32,
    pub paradrop_height_max: f32,
}

impl OrderDefines {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        const SECTION: &str = "NGraphics";
        let passes = |name: &str| -> io::Result<u32> {
            u32::try_from(defines.int32(SECTION, name)?)
                .map_err(|_| invalid(format!("NDefines.{SECTION}.{name} is negative")))
        };
        let pair = |smoothness: &str, count: &str| -> io::Result<Smoothing> {
            Ok(Smoothing {
                smoothness: defines.fixed5(SECTION, smoothness)?,
                passes: passes(count)?,
            })
        };
        let color = |name: &str| -> io::Result<[f32; 4]> {
            let values = defines.numbers(SECTION, name)?;
            if values.len() < 3 {
                return Err(invalid(format!(
                    "NDefines.{SECTION}.{name} has {} components",
                    values.len()
                )));
            }
            Ok([
                values[0] as f32,
                values[1] as f32,
                values[2] as f32,
                values.get(3).copied().unwrap_or(1.0) as f32,
            ])
        };
        Ok(Self {
            order_move: pair("ORDER_MOVE_SMOOTHNESS", "ORDER_MOVE_SMOOTHEN_PASSES")?,
            order_front: pair("ORDER_FRONT_SMOOTHNESS", "ORDER_FRONT_SMOOTHEN_PASSES")?,
            root_front: pair("ROOT_FRONT_SMOOTHNESS", "ROOT_FRONT_SMOOTHEN_PASSES")?,
            snapped_off_front: pair(
                "SNAPPED_OFF_FRONT_SMOOTHNESS",
                "SNAPPED_OFF_FRONT_SMOOTHEN_PASSES",
            )?,
            root_front_offset: defines.fixed5(SECTION, "ROOT_FRONT_OFFSET")?,
            root_front_max_intersection_tests_order: defines
                .int32(SECTION, "ROOT_FRONT_MAX_INTERSECTION_TESTS_ORDER")?,
            root_front_max_intersection_tests_front: defines
                .int32(SECTION, "ROOT_FRONT_MAX_INTERSECTION_TESTS_FRONT")?,
            max_offsets: u8::try_from(defines.int32(SECTION, "ORDER_FRONT_MAX_OFFSETS")?).map_err(
                |_| invalid("NDefines.NGraphics.ORDER_FRONT_MAX_OFFSETS is out of range"),
            )?,
            ext_text_points_limit: defines.int32(SECTION, "ARROW_EXT_TEXT_POINTS_LIMIT")?,
            virtual_color: color("VIRTUAL_BATTLEPLANS_COLOR")?,
            allied_color: color("ALLIED_BATTLEPLANS_COLOR")?,
            paradrop_height_to_len: defines.number(SECTION, "ARROW_PARADROP_HEIGHT_TO_LEN")? as f32,
            paradrop_height_max: defines.number(SECTION, "ARROW_PARADROP_HEIGHT_MAX")? as f32,
        })
    }
}

// ------------------------------------------------------------- the arrow types

/// One content-resolved `arrowType` block. Text defaults are the constructor
/// values.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowType {
    pub name: String,
    /// Definition-instance bucket index, assigned in file order and consumed
    /// in ascending order.
    pub declaration_index: usize,
    /// Exact authored technique, retained for the compiled effect selection.
    pub shader_technique: String,
    /// Ordinary/late and shadow admission flags.
    pub draw_last: bool,
    pub draw_shadows: bool,
    pub masks: Vec<String>,
    pub patterns: Vec<String>,
    pub body_uv_scale: f32,
    pub secondary_body_uv_scale: f32,
    pub mask_inner: [f32; 3],
    pub mask_outer: [f32; 3],
    /// `body_width = { start end }`, world units.
    pub body_width: [f32; 2],
    pub head_length: f32,
    pub texture_loop: f32,
    pub texture_variant_count: u32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub global_scale_min_size: f32,
    pub global_scale_max_points: f32,
    pub texture_anim_speed: f32,
    /// `culling_direction`: integer teeth side for which the texture is authored.
    /// When the arrow's stored side differs, flip texture v with culling offset 1.
    pub culling_direction: i32,
    /// `text_size`: label font size.
    pub text_size: f32,
    /// `text_offset`: added to the label's start along the arrow.
    pub text_offset: f32,
}

impl Default for ArrowType {
    fn default() -> Self {
        Self {
            name: String::new(),
            declaration_index: 0,
            shader_technique: String::new(),
            draw_last: false,
            draw_shadows: true,
            masks: Vec::new(),
            patterns: Vec::new(),
            body_uv_scale: 1.0,
            secondary_body_uv_scale: 1.0,
            mask_inner: [0.0, 1.0, 0.0],
            mask_outer: [1.0, 0.0, 0.0],
            body_width: [1.0, 1.0],
            head_length: 0.0,
            texture_loop: 0.0,
            texture_variant_count: 1,
            fade_in: 0.0,
            fade_out: 0.0,
            global_scale_min_size: 1.0,
            global_scale_max_points: 1.0,
            texture_anim_speed: 0.0,
            culling_direction: 0,
            text_size: 1.0,
            text_offset: 0.0,
        }
    }
}

impl ArrowType {
    /// A linear ramp from `global_scale_min_size` to 1 over
    /// `global_scale_max_points` controls.
    pub fn global_scale(&self, points: usize) -> f32 {
        if self.global_scale_max_points <= 1.0 {
            return 1.0;
        }
        let t = (points as f32 / self.global_scale_max_points).clamp(0.0, 1.0);
        self.global_scale_min_size + (1.0 - self.global_scale_min_size) * t
    }
}

/// The parsed `gfx/maparrows/maparrows.txt`: the army-group palette, every
/// `arrowType` and the `arrowText` font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArrowTypes {
    pub types: BTreeMap<String, ArrowType>,
    /// The `colors = { leader_group = { r g b a } ... }` list at the head of the
    /// file, which is the army and army-group colour palette.
    pub leader_colors: Vec<[f32; 4]>,
    /// `arrowText = { font = ... }`, the label font.
    pub text_font: Option<String>,
    /// `arrowText = { technique = ... }`, selected independently of country names.
    pub text_technique: String,
}

impl ArrowTypes {
    pub fn get(&self, name: &str) -> Option<&ArrowType> {
        self.types.get(name)
    }

    /// Parses the file. `symbolType` and buttons are not drawn by this layer.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let document = parser::parse(bytes)?;
        let mut out = Self {
            text_font: text_font(&document),
            text_technique: document
                .get_all("arrowText")
                .filter_map(Value::items)
                .flat_map(|items| items.iter())
                .filter(|item| item.key.as_deref() == Some(b"technique"))
                .filter_map(|item| item.value.scalar_str())
                .last()
                .unwrap_or_default()
                .into(),
            ..Self::default()
        };
        for item in document.get_all("colors") {
            let Some(entries) = item.items() else {
                continue;
            };
            for entry in entries {
                if entry.key.as_deref() != Some(b"leader_group".as_slice()) {
                    continue;
                }
                if let Some(color) = floats(&entry.value)
                    && color.len() >= 3
                {
                    out.leader_colors.push([
                        color[0],
                        color[1],
                        color[2],
                        color.get(3).copied().unwrap_or(1.0),
                    ]);
                }
            }
        }
        for (declaration_index, item) in document.get_all("arrowType").enumerate() {
            let Some(entries) = item.items() else {
                continue;
            };
            let mut arrow = ArrowType {
                declaration_index,
                ..Default::default()
            };
            let mut secondary_scale_declared = false;
            for entry in entries {
                let Some(key) = entry.key.as_deref() else {
                    continue;
                };
                let scalar = entry.value.scalar_str().map(str::to_owned);
                let number = scalar.as_deref().and_then(|text| text.parse::<f32>().ok());
                match key {
                    b"name" => arrow.name = scalar.unwrap_or_default(),
                    b"technique" => {
                        arrow.shader_technique = scalar.unwrap_or_default();
                    }
                    // Read the admission boolean; failure keeps the byte.
                    b"draw_last" => {
                        arrow.draw_last = entry
                            .value
                            .scalar()
                            .and_then(installed_content_core::parser::read_bool)
                            .unwrap_or(arrow.draw_last);
                    }
                    b"texture_masks" => arrow.masks = strings(&entry.value),
                    // Read shadow admission with the same boolean reader; default true.
                    b"draw_shadows" => {
                        arrow.draw_shadows = entry
                            .value
                            .scalar()
                            .and_then(installed_content_core::parser::read_bool)
                            .unwrap_or(arrow.draw_shadows);
                    }
                    b"texture_patterns" => arrow.patterns = strings(&entry.value),
                    b"body_uv_scale" => arrow.body_uv_scale = number.unwrap_or(1.0),
                    b"body_uv_scale_2" => {
                        secondary_scale_declared = true;
                        arrow.secondary_body_uv_scale = number.unwrap_or(1.0);
                    }
                    b"mask_inner" => {
                        if let Some(v) = floats(&entry.value)
                            && v.len() >= 3
                        {
                            arrow.mask_inner = [v[0], v[1], v[2]];
                        }
                    }
                    b"mask_outer" => {
                        if let Some(v) = floats(&entry.value)
                            && v.len() >= 3
                        {
                            arrow.mask_outer = [v[0], v[1], v[2]];
                        }
                    }
                    b"body_width" => {
                        if let Some(v) = floats(&entry.value)
                            && v.len() >= 2
                        {
                            arrow.body_width = [v[0], v[1]];
                        }
                    }
                    b"head_length" => arrow.head_length = number.unwrap_or(0.0),
                    b"texture_loop" => arrow.texture_loop = number.unwrap_or(0.0),
                    b"texture_variant_count" => {
                        arrow.texture_variant_count = number.unwrap_or(1.0).max(1.0) as u32
                    }
                    b"fade_in" => arrow.fade_in = number.unwrap_or(0.0),
                    b"fade_out" => arrow.fade_out = number.unwrap_or(0.0),
                    b"global_scale_min_size" => arrow.global_scale_min_size = number.unwrap_or(1.0),
                    b"global_scale_max_points" => {
                        arrow.global_scale_max_points = number.unwrap_or(1.0)
                    }
                    b"texture_anim_speed" => arrow.texture_anim_speed = number.unwrap_or(0.0),
                    b"culling_direction" => arrow.culling_direction = number.unwrap_or(0.0) as i32,
                    b"text_size" => arrow.text_size = number.unwrap_or(1.0),
                    b"text_offset" => arrow.text_offset = number.unwrap_or(0.0),
                    _ => {}
                }
            }
            if !secondary_scale_declared {
                arrow.secondary_body_uv_scale = arrow.body_uv_scale;
            }
            if !arrow.name.is_empty() {
                // The first definition of a name wins.
                out.types.entry(arrow.name.clone()).or_insert(arrow);
            }
        }
        if out.types.is_empty() {
            return Err(invalid(format!("{MAPARROWS} declares no arrowType")));
        }
        Ok(out)
    }
}

/// The last `arrowText` block's `font`.
fn text_font(document: &parser::Document) -> Option<String> {
    document
        .get_all("arrowText")
        .filter_map(|item| item.items())
        .filter_map(|entries| {
            entries
                .iter()
                .find(|entry| entry.key.as_deref() == Some(b"font".as_slice()))
                .and_then(|entry| entry.value.scalar_str().map(str::to_owned))
        })
        .last()
}

/// The `arrowText` font name, for the icon loader, which loads every font the
/// map draws. An unreadable file names none here; [`OrderAssets::load`]
/// reports that failure itself.
pub fn arrow_text_font(layers: &ResolvedLayers) -> io::Result<Option<String>> {
    let Ok(bytes) = layers.read_uncovered(MAPARROWS) else {
        return Ok(None);
    };
    Ok(text_font(&parser::parse(&bytes)?))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .items()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.value.scalar_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn floats(value: &Value) -> Option<Vec<f32>> {
    let items = value.items()?;
    Some(
        items
            .iter()
            .filter_map(|item| item.value.scalar_str())
            .filter_map(|text| text.parse::<f32>().ok())
            .collect(),
    )
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct OrderVertex {
    pub position: [f32; 3],
    /// `uv_isHead_variant`: `u` along, `v` across, `z` head flag, `w` variant.
    pub uv_head_variant: [f32; 4],
}

/// One order's draw: its arrow type, its colour and its slice of the buffers.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderDraw {
    /// Air map controls share this manager, independently of battle-plan switches.
    pub air_mode: bool,
    pub arrow_type: String,
    pub texture_choice: usize,
    pub color: [f32; 4],
    /// The curve's own length in world units, which the no-head shader's fade
    /// arithmetic is measured in.
    pub total_length: f32,
    /// World units one unit of `uv.x` covers.
    pub uv_to_length: f32,
    /// Culling offset: 1 when the type declares a culling direction and the
    /// arrow's stored teeth side differs from it.
    pub culling_offset: f32,
    /// The type's definition bucket, which the draw visits in ascending order.
    pub declaration_index: usize,
    /// Comparator width: larger widths draw first in a bucket.
    pub width: f32,
    pub label: Option<usize>,
    pub native_strips: Vec<std::ops::Range<u32>>,
    pub bounds: [[f32; 3]; 2],
    /// Secondary colour of the inner pass and the two outer-pass colours,
    /// loaded from the arrow and initialized when the arrow is created.
    pub secondary_color: [f32; 4],
    pub outer_color: [f32; 4],
    /// Viewer-derived state for this hour. Child ribbons share the parent
    /// callback; generic raid arrows retain their constructor state.
    pub execution: ExecutionState,
    /// The plan family a viewer can hide; child sections take their parent's.
    pub kind: dashboard_core::protocol::PlanKind,
}

/// Every order of one hour, in one buffer pair, and the plan labels.
#[derive(Clone, Debug, Default)]
pub struct OrderField {
    pub vertices: Vec<OrderVertex>,
    pub native_vertices: Vec<Vertex>,
    pub draws: Vec<OrderDraw>,
    /// Trade-route stored control samples and their world-space picking radius.
    pub trade_route_hovers: Vec<[f32; 4]>,
    /// The name labels, drawn by a `mapname` pass after the ribbons.
    pub labels: Placements,
    /// The raid arrows' flying units ([`super::raid_arrows`]), which the
    /// unit model layer draws.
    pub raid_units: Vec<crate::map::unit_pose::UnitPlacement>,
    /// What the hour held and what was drawn, for the report.
    pub report: OrderReport,
}

/// What one hour's orders came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderReport {
    /// Orders the hour records, over every country.
    pub recorded: usize,
    /// Orders belonging to the countries this perspective draws.
    pub ours: usize,
    /// Orders drawn, one ribbon each.
    pub drawn: usize,
    /// Army-group child ribbons and labels, separate from the order count.
    pub child_sections_drawn: usize,
    pub child_labels_drawn: usize,
    /// Orders the engine draws no arrow for: the garrison areas, invasions
    /// with no path, paradrops without a source or target, and fronts whose
    /// recorded span is empty or one the native destroys ([`front_pairs`]).
    pub no_arrow: usize,
    /// Orders whose geometry has fewer than two controls.
    pub too_short: usize,
    /// Per classification, how many were drawn.
    pub per_kind: BTreeMap<&'static str, usize>,
    /// Per classification, how many the hour records over every country.
    pub per_kind_recorded: BTreeMap<&'static str, usize>,
    /// Offensive lines drawn along their snapped borders.
    pub snapped: usize,
    /// Fronts drawn with an overlap offset.
    pub offset_fronts: usize,
    /// Labels placed, and orders whose label fitted no variant.
    pub labels: usize,
    pub labels_unfitted: usize,
    /// Labels refused because a variant the layout reached cannot be
    /// localised or holds a control map text does not draw, by that reason.
    pub labels_refused: BTreeMap<String, usize>,
    /// Labels whose extended transport-count operands are unavailable.
    pub transports_omitted: usize,
    /// What was refused rather than guessed, and how often.
    pub refused: BTreeMap<&'static str, usize>,
    /// Named missing facts/content of the sea search, counted once per order.
    pub invasion_refused: BTreeMap<String, usize>,
    /// Active orders whose reached execution inputs are unknown. The ribbon
    /// and its children are withheld; unknown never means native false.
    pub execution_refused: BTreeMap<String, usize>,
    /// Unknown allied-country admission, counted per physical occurrence.
    pub allied_refused: BTreeMap<String, usize>,
    /// Why nothing could be drawn, when the content lacks an input.
    pub unavailable: Option<String>,
    /// Unbound centroid curves only: maximum/mean distance from a path
    /// centroid to the nearest control, in thousandths of a world unit.
    pub control_gap_max_milli: u32,
    pub control_gap_mean_milli: u32,
    /// The raid arrows drawn in this field and what they could not draw
    /// ([`super::raid_arrows`]).
    pub raid_arrows: RaidArrowReport,
    /// The trade-route lines' warning-area lines, drawn in this field's arrow manager.
    pub trade_routes: Vec<String>,
    /// Air map arrows sharing the same type buckets.
    pub air_orders: Vec<String>,
}

impl OrderReport {
    /// The warning-area lines: why the plan is not drawn, and each refusal
    /// with its count. Empty when every order of the view was drawn.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(why) = &self.unavailable {
            out.push(format!("Battle plans are not drawn: {why}."));
        }
        for (what, count) in &self.refused {
            out.push(format!("Battle plan: {count} refused, {what}."));
        }
        for (what, count) in &self.invasion_refused {
            out.push(format!(
                "Battle plan: {count} invasion routes refused, {what}."
            ));
        }
        for (what, count) in &self.execution_refused {
            out.push(format!(
                "Battle plan: {count} active orders and their child ribbons are not drawn, {what}."
            ));
        }
        for (what, count) in &self.allied_refused {
            out.push(format!(
                "Battle plan: {count} allied plans refused, {what}."
            ));
        }
        if self.no_arrow > 0 {
            out.push(format!(
                "Battle plan: {} orders the game draws no arrow for (garrison areas, invasions without a path, paradrops without a source or target, fronts with an empty or destroyed span).",
                self.no_arrow
            ));
        }
        if self.too_short > 0 {
            out.push(format!(
                "Battle plan: {} orders with fewer than two placeable controls are not drawn.",
                self.too_short
            ));
        }
        if self.labels_unfitted > 0 {
            out.push(format!(
                "Battle plan: {} labels fit no name variant along their arrow and are not drawn.",
                self.labels_unfitted
            ));
        }
        for (reason, count) in &self.labels_refused {
            out.push(format!(
                "Battle plan: {count} labels are not drawn: {reason}."
            ));
        }
        out.extend(self.raid_arrows.warnings());
        out.extend(self.trade_routes.iter().cloned());
        out.extend(self.air_orders.iter().cloned());
        out
    }
}
/// What one hour's raid arrows came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidArrowReport {
    /// The viewing country's non-ended raids, one arrow each natively.
    pub raids: usize,
    pub drawn: usize,
    /// Arrows the default raid filter hides.
    pub filtered: usize,
    /// Arrows the game draws nothing for: no path, or no controls.
    pub empty: usize,
    /// Launched arrows drawn without their unit, and why.
    pub units_missing: usize,
    pub units_reason: Option<String>,
    /// What was refused rather than guessed, and how often.
    pub refused: BTreeMap<String, usize>,
    /// Why no arrow could be decided at all.
    pub unavailable: Option<String>,
}

impl RaidArrowReport {
    pub fn refuse(&mut self, reason: impl Into<String>) {
        *self.refused.entry(reason.into()).or_default() += 1;
    }

    /// The warning-area lines; empty when every arrow and unit was drawn.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(why) = &self.unavailable {
            out.push(format!("Raid arrows are not drawn: {why}."));
        }
        for (what, count) in &self.refused {
            out.push(format!("Raid arrows: {count} refused, {what}."));
        }
        if self.units_missing > 0 {
            out.push(format!(
                "Raid arrows: {} launched raids are drawn without their unit: {}.",
                self.units_missing,
                self.units_reason.as_deref().unwrap_or(PLANE_UNRECORDED)
            ));
        }
        out
    }
}
// --------------------------------------------------------------------- assets

/// The arrow types, the mask/pattern pages this layer uploads and the
/// `NDefines.NGraphics` values the geometry reads.
#[derive(Debug)]
pub struct OrderAssets {
    pub types: ArrowTypes,
    /// Separate assigned/unassigned texture choices for each arrow type.
    pub pages: BTreeMap<(String, usize), ArrowPage>,
    pub defines: OrderDefines,
    /// Simulation defines used only by reached active execution branches.
    pub execution: ExecutionDefines,
    /// The raid arrows' defines ([`super::raid_arrows`]), or why they
    /// cannot be read; the battle plan does not need them.
    pub raid_defines: Result<installed_content_raids::raids::RaidGraphicsDefines, String>,
    pub radar: Result<SymbolPage, String>,
    pub air_range: Result<SymbolPage, String>,
    pub air_transfer: Result<SymbolPage, String>,
}

/// One map symbol definition and its runtime texture, through the recorded content stack.
#[derive(Debug)]
pub struct SymbolPage {
    pub technique: String,
    pub size: f32,
    pub texture: dds::Texture,
    /// Own, allied and selected colours.
    pub colors: [[f32; 4]; 3],
    /// Fixed5 angular step per presentation frame.
    pub rotation: i64,
}

impl SymbolPage {
    fn radar(
        layers: &ResolvedLayers,
        decode: Decode,
        recorded: &Defines,
        graphics: &Defines,
    ) -> io::Result<Self> {
        Self::load(
            layers,
            decode,
            recorded,
            graphics,
            "radar",
            [
                "RADAR_RANGE_COLOR",
                "RADAR_RANGE_ALLIED_COLOR",
                "RADAR_RANGE_SELECTED_COLOR",
            ],
            Some("RADAR_ROTATION_SPEED"),
        )
    }

    fn air_range(
        layers: &ResolvedLayers,
        decode: Decode,
        recorded: &Defines,
        graphics: &Defines,
    ) -> io::Result<Self> {
        Self::load(
            layers,
            decode,
            recorded,
            graphics,
            "air_range",
            [
                "AIR_RANGE_INDICATOR_DEFAULT_COLOR",
                "AIR_RANGE_INDICATOR_NO_WINGS_COLOR",
                "AIR_RANGE_INDICATOR_DEFAULT_COLOR",
            ],
            Some("AIR_RANGE_INDICATOR_ROTATION_SPEED"),
        )
    }

    fn air_transfer(
        layers: &ResolvedLayers,
        decode: Decode,
        recorded: &Defines,
        graphics: &Defines,
    ) -> io::Result<Self> {
        Self::load(
            layers,
            decode,
            recorded,
            graphics,
            "air_transfer_progress",
            ["AIR_TRANSFER_ARROW_COLOR"; 3],
            None,
        )
    }

    fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        recorded: &Defines,
        graphics: &Defines,
        symbol: &str,
        colors: [&str; 3],
        rotation: Option<&str>,
    ) -> io::Result<Self> {
        let document = parser::parse(&layers.read_uncovered(MAPARROWS)?)?;
        let mut selected = None;
        for declaration in document.get_all("symbolType").filter_map(Value::items) {
            let mut name = String::new();
            let mut technique = String::new();
            let mut texture = String::new();
            let mut size = 0.;
            let mut text = false;
            for entry in declaration {
                match entry.key.as_deref() {
                    Some(b"name") => name = entry.value.scalar_str().unwrap_or_default().into(),
                    Some(b"technique") => {
                        technique = entry.value.scalar_str().unwrap_or_default().into()
                    }
                    Some(b"texture") => {
                        texture = entry.value.scalar_str().unwrap_or_default().into()
                    }
                    Some(b"size") => {
                        size = entry
                            .value
                            .scalar_str()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.)
                    }
                    Some(b"text") => text = true,
                    _ => {}
                }
            }
            if name == symbol {
                selected = Some((technique, texture, size, text));
            }
        }
        let (technique, texture, size, text) = selected
            .ok_or_else(|| invalid(format!("maparrows declares no {symbol} symbolType")))?;
        if text {
            return Err(invalid(format!(
                "{symbol} symbolType text attachments are not drawn"
            )));
        }
        let texture = load_file(&texture, &|path| layers.read_uncovered_path(path), decode)?;
        if texture.format != dds::TextureFormat::Rgba8 {
            return Err(invalid(format!("{symbol} symbol texture: expected RGBA8")));
        }
        super::textures::check_upload("radar symbol", &texture, super::textures::ColorSpace::Srgb)?;
        let (colors, rotation) =
            layers.graphics_define("radar symbols", graphics, recorded, |defines| {
                let color = |name| -> io::Result<[f32; 4]> {
                    match defines.numbers("NMapMode", name)?.as_slice() {
                        [r, g, b, a, ..] => Ok([*r as f32, *g as f32, *b as f32, *a as f32]),
                        _ => Err(invalid(format!(
                            "NMapMode.{name} is not a four-component colour"
                        ))),
                    }
                };
                let speed = rotation
                    .map(|name| defines.number("NMapMode", name))
                    .transpose()?
                    .unwrap_or(0.) as f32;
                let rotation =
                    ((if speed >= 0. { 1. } else { 0. }) + speed * 100_000. - 0.5) as i64;
                Ok::<_, io::Error>((
                    [color(colors[0])?, color(colors[1])?, color(colors[2])?],
                    rotation,
                ))
            })?;
        Ok(Self {
            technique,
            size,
            texture,
            rotation,
            colors,
        })
    }
}

/// One texture choice. Authored internal variants use vTextureVariantScale;
/// assigned/unassigned masks are separate resources, not stacked variants.
#[derive(Debug)]
pub struct ArrowPage {
    pub mask: dds::Texture,
    pub pattern: dds::Texture,
    pub variants: u32,
}

/// Every arrow type the battle plan can select; the raid arrows' types
/// ([`installed_content_raids::raids::RaidArrowKind::arrow_type`]) and the trade-route lines' type
/// ([`TRADE_ROUTE_TYPE_NAME`]) are uploaded with them.
pub const DRAWN_ARROW_TYPES: &[&str] = &[
    "defence_line",
    "ag_defence_line_no_child",
    GROUP_SECTION_TYPE_NAME,
    "blitz",
    "withdraw",
    "blitz_line",
    "withdraw_line",
    "fallback_line",
    "virtual_line",
    "ag_defence_line",
    "offence_line",
    "ag_offence_line",
    "move_thick",
    "ag_move_thick",
    "invasion",
    "paradrop",
    "air_mission_direction",
];

impl OrderAssets {
    /// `gfx/maparrows` is outside content coverage, so its files and textures
    /// are read unverified, like the tree meshes and rail strip.
    ///
    /// `defines` are the recorded defines, whose operands the plans execute with, and the
    /// graphics defines, which the arrows' shapes and colours read, each value falling back to
    /// the recorded one ([`ResolvedLayers::graphics_define`]). A texture page the visual mods
    /// break is read from the recorded layers below them ([`ResolvedLayers::artwork_item`]).
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        (defines, graphics): (&Defines, &Defines),
        warnings: &mut Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        let types = ArrowTypes::parse(&layers.read_uncovered(MAPARROWS)?)?;
        let defines_source = defines;
        let defines =
            layers.graphics_define("battle-plan arrows", graphics, defines, OrderDefines::read)?;
        let below = layers.without_overlays();
        let mut pages = BTreeMap::new();
        let raid_types = installed_content_raids::raids::RaidArrowKind::ALL.map(|kind| kind.arrow_type());
        for name in DRAWN_ARROW_TYPES
            .iter()
            .chain(raid_types.iter())
            .chain(std::iter::once(&TRADE_ROUTE_TYPE_NAME))
        {
            let Some(arrow) = types.get(name) else {
                // Named for the map's fallback, which compares it with the recording's.
                layers.note_refused(format!("arrowType {name}"));
                warnings.push((installed_content_core::DiagnosticKind::Refused, if *name == TRADE_ROUTE_TYPE_NAME {
                    format!("{MAPARROWS} declares no arrowType {name}; trade-route lines are not drawn.")
                } else if raid_types.contains(name) {
                    format!("{MAPARROWS} declares no arrowType {name}; raid arrows that would use it are not drawn.")
                } else {
                    format!("{MAPARROWS} declares no arrowType {name}; battle-plan orders that would use it are not drawn.")
                }));
                continue;
            };
            for choice in 0..arrow.masks.len().min(arrow.patterns.len()) {
                let item = layers.artwork_item(
                    &format!("arrowType {name} texture choice {choice}"),
                    below.is_some(),
                    |from_recording| match &below {
                        Some(below) if from_recording => page(below, decode, arrow, choice),
                        _ => page(layers, decode, arrow, choice),
                    },
                );
                match item {
                    Ok(page) => {
                        pages.insert(((*name).to_owned(), choice), page);
                    }
                    Err(error) => warnings.push((
                        installed_content_core::DiagnosticKind::Refused,
                        format!("arrowType {name} texture choice {choice}: {error}"),
                    )),
                }
            }
        }
        warnings.push((installed_content_core::DiagnosticKind::Warning,
            "Battle-plan arrows and their textures are read from gfx/maparrows, which is outside the content coverage policy and is not hash-verified."
                .into(),
        ));
        let raid_defines = layers
            .graphics_define(
                "raid arrows",
                graphics,
                defines_source,
                installed_content_raids::raids::RaidGraphicsDefines::read,
            )
            .map_err(|error| error.to_string());
        Ok(Self {
            air_transfer: layers
                .artwork_item(
                    "air transfer progress symbol",
                    below.is_some(),
                    |from_recording| {
                        let stack = if from_recording {
                            below.as_ref().unwrap_or(layers)
                        } else {
                            layers
                        };
                        SymbolPage::air_transfer(stack, decode, defines_source, graphics)
                    },
                )
                .map_err(|error| error.to_string()),
            air_range: layers
                .artwork_item("air range symbol", below.is_some(), |from_recording| {
                    let stack = if from_recording {
                        below.as_ref().unwrap_or(layers)
                    } else {
                        layers
                    };
                    SymbolPage::air_range(stack, decode, defines_source, graphics)
                })
                .map_err(|e| e.to_string()),
            radar: layers
                .artwork_item("radar symbol", below.is_some(), |from_recording| {
                    let stack = if from_recording {
                        below.as_ref().unwrap_or(layers)
                    } else {
                        layers
                    };
                    SymbolPage::radar(stack, decode, defines_source, graphics)
                })
                .map_err(|e| e.to_string()),
            types,
            pages,
            defines,
            raid_defines,
            execution: ExecutionDefines::read(defines_source),
        })
    }
}

fn page(
    layers: &ResolvedLayers,
    decode: Decode,
    arrow: &ArrowType,
    choice: usize,
) -> io::Result<ArrowPage> {
    let read = |path: &str| -> io::Result<dds::Texture> {
        let image = load_file(path, &|path| layers.read_uncovered_path(path), decode)?;
        if image.format != dds::TextureFormat::Rgba8 {
            return Err(invalid(format!("{path}: expected RGBA8")));
        }
        Ok(image)
    };
    Ok(ArrowPage {
        mask: read(&arrow.masks[choice])?,
        pattern: read(&arrow.patterns[choice])?,
        variants: arrow.texture_variant_count,
    })
}

/// Native route picker: compare stored samples to the point on the normalized pointer ray
/// whose eye-distance equals the sample's eye-distance, including equality at the radius.
pub fn trade_route_hit(samples: &[[f32; 4]], eye: [f32; 3], ray: [f32; 3], width: f32) -> bool {
    samples.iter().any(|&[x, y, z, radius]| {
        let p = [super::unit_pose::unwrap(x, eye[0], width), y, z];
        let distance = (0..3).map(|i| (p[i] - eye[i]).powi(2)).sum::<f32>().sqrt();
        (0..3)
            .map(|i| (p[i] - (eye[i] + ray[i] * distance)).powi(2))
            .sum::<f32>()
            <= radius * radius
    })
}

#[cfg(test)]
mod trade_hover_tests {
    use super::*;
    #[test]
    fn explicitly_authored_secondary_scale_one_survives_the_primary_fallback() {
        let content=ArrowTypes::parse(b"arrowType={name=trade body_uv_scale=0.41 body_uv_scale_2=1} arrowType={name=other body_uv_scale=0.2}").unwrap();
        assert_eq!(content.types["trade"].secondary_body_uv_scale, 1.);
        assert_eq!(content.types["other"].secondary_body_uv_scale, 0.2);
    }
    #[test]
    fn route_picking_uses_stored_samples_and_keeps_radius_equality() {
        let ray = [0., 0., 1.];
        let eye = [0.; 3];
        assert!(trade_route_hit(&[[0., 0., 10., 0.]], eye, ray, 100.));
        // A continuous line joining these samples crosses the ray, but no stored sample hits.
        assert!(!trade_route_hit(
            &[[-10., 0., 10., 1.], [10., 0., 10., 1.]],
            eye,
            ray,
            100.
        ));
        assert!(trade_route_hit(&[[0., 0., -10., 20.]], eye, ray, 100.));
        assert!(!trade_route_hit(&[[0., 0., -10., 19.99]], eye, ray, 100.));
    }
}
