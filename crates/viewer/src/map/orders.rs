//! The viewing country's and admitted allied battle plans: fronts, offensive and fallback lines,
//! move, invasion and paradrop arrows, and their name labels.
//!
//! Walk the country's theatre roster, each theatre's army-group then army
//! roster (recorded roster positions), each group's primary then fallback
//! root orders, then the recorded ordinary successors in ordered depth-first
//! order. Suppress a sole child that has another first predecessor.
//! Virtual editing children are not walked in the deselected view.
//!
//! Geometry per order:
//!
//! * a bound front runs along its
//!   section's province pairs over the signed inclusive span, as integer
//!   gradient-border chains (`chain`), offset by the directed overlap counter
//!   (army groups reserve twice) to the controller-majority side;
//! * an offensive line preceded by a move snaps to its targets' far borders
//!   when the pair chain covers the candidates ([`offensive`]), else it is the
//!   centroid-and-crossing polyline; a one-province line without a snap is the
//!   ray through its province ([`single`]);
//! * a move is the centroid-and-crossing polyline of its thinned path,
//!   starting at the first crossing and ending on the nearest stored control
//!   of its first successor line.
//!
//! Smoothing, offsets and the extended-text limit are `NDefines.NGraphics`;
//! arrow types, fonts, text size and offset are `gfx/maparrows/maparrows.txt`.
//! Remaining limits are reported by `map::tables`.

pub(crate) mod allied;
mod chain;
mod execution;
mod fixed;
mod group_sections;
pub(super) mod invasion;
mod labels;
mod native;
mod offensive;
mod paradrop_counts;
mod single;
mod submit;
mod transport;
mod transport_counts;

pub use labels::LabelInputs;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    defines::Defines,
    image::{dds, native::Decode},
    map::MapData,
    movement::MovementMap,
    parser::{self, Value},
};
use replay::{
    frame::DecodedFrame,
    rows::{
        ArmyOrderPlanRow, ArmyOrderRow, FrontRow, LandTheatreRow, OrdersGroupPlanRow,
        OrdersGroupRow, TableId, tables,
    },
};

use self::fixed::{Point, SCALE, Smoothing};
use super::arrow_shader::Mode;
use super::{
    borders::HeightField,
    name_pass::{self, Placements, WorldName},
    scene::{DEPTH_FORMAT, Targets, buffer},
    tables::{self as table_gate},
    textures::{ColorSpace, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// ------------------------------------------------------------------ constants

/// `gfx/maparrows/maparrows.txt`, the arrow-type and palette file.
pub const MAPARROWS: &str = "gfx/maparrows/maparrows.txt";

/// Move orders.
pub const TYPE_MOVE: i32 = 1;
/// Line orders: the front, offensive and fallback tools.
pub const TYPE_LINE: i32 = 2;
/// Invasion orders.
pub const TYPE_INVASION: i32 = 3;
/// Paradrop orders.
pub const TYPE_PARADROP: i32 = 4;
/// Area-defense orders, the garrison area. They have no drawing branch
/// and produce no arrow.
pub const TYPE_AREA_DEFENSE: i32 = 5;

/// Neighbour kind 4 is impassable and is skipped by the adjacency test.
const IMPASSABLE: u8 = 4;

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
    declaration_index: usize,
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
    types: BTreeMap<String, ArrowType>,
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
                            .and_then(installed_content::parser::read_bool)
                            .unwrap_or(arrow.draw_last);
                    }
                    b"texture_masks" => arrow.masks = strings(&entry.value),
                    // Read shadow admission with the same boolean reader; default true.
                    b"draw_shadows" => {
                        arrow.draw_shadows = entry
                            .value
                            .scalar()
                            .and_then(installed_content::parser::read_bool)
                            .unwrap_or(arrow.draw_shadows);
                    }
                    b"texture_patterns" => arrow.patterns = strings(&entry.value),
                    b"body_uv_scale" => arrow.body_uv_scale = number.unwrap_or(1.0),
                    b"body_uv_scale_2" => arrow.secondary_body_uv_scale = number.unwrap_or(1.0),
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
            if arrow.secondary_body_uv_scale == 1.0 && arrow.body_uv_scale != 1.0 {
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

// ------------------------------------------------------------- the province map

/// Static province quantities read from the content province bitmap: the
/// integer pixel centroid (truncated mean), directed integer border pixels
/// towards each neighbour, and the owned pixel a one-province line starts from.
/// Neighbour order, kinds and crossings come from the content movement map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProvinceGeometry {
    width: u32,
    centroids: Vec<Option<[i32; 2]>>,
    borders: BTreeMap<(u32, u32), Vec<[i32; 2]>>,
    centers: single::Centers,
}

impl ProvinceGeometry {
    /// Centroids and border tracing, cached once at map load.
    pub fn build(map: &MapData) -> Self {
        let width = map.width as usize;
        let height = map.height as usize;
        let largest = map.province_ids.iter().copied().max().unwrap_or(0) as usize;
        let mut sums = vec![(0u64, 0u64, 0u64); largest + 1];
        let mut edge_pixels = Vec::new();
        for row in 0..height {
            for column in 0..width {
                let id = map.province_ids[row * width + column];
                let entry = &mut sums[id as usize];
                entry.0 += column as u64;
                entry.1 += (height - 1 - row) as u64;
                entry.2 += 1;
                // The map wraps in x, like the engine's border collection.
                let right = map.province_ids[row * width + (column + 1) % width];
                if right != id {
                    edge_pixels.push((id, right, (height - 1 - row) as u32, column as u32));
                    edge_pixels.push((
                        right,
                        id,
                        (height - 1 - row) as u32,
                        ((column + 1) % width) as u32,
                    ));
                }
                if row + 1 < height {
                    let below = map.province_ids[(row + 1) * width + column];
                    if below != id {
                        edge_pixels.push((id, below, (height - 1 - row) as u32, column as u32));
                        edge_pixels.push((below, id, (height - 2 - row) as u32, column as u32));
                    }
                }
            }
        }
        // Collect each province's own integer pixels, once per distinct neighbour
        // at a pixel. World rows run south to north.
        edge_pixels.sort_unstable();
        edge_pixels.dedup();
        let mut borders = BTreeMap::new();
        let mut edges = edge_pixels.as_slice();
        while let Some(&(a, b, _, _)) = edges.first() {
            let count = edges.partition_point(|p| (p.0, p.1) == (a, b));
            let points: Vec<_> = edges[..count]
                .iter()
                .map(|p| [p.3 as f32, p.2 as f32])
                .collect();
            let sorted = sort_edge_pixels(&points)
                .into_iter()
                .map(|p| [p[0] as i32, p[1] as i32])
                .collect();
            borders.insert((a, b), sorted);
            edges = &edges[count..];
        }
        // Centroid sums use i32 fields, so a sum past i32 wraps; division is signed.
        let centroids: Vec<Option<[i32; 2]>> = sums
            .into_iter()
            .map(|(sx, sy, count)| {
                (count > 0).then(|| {
                    let count = count as u32 as i32;
                    [
                        (sx as u32 as i32).wrapping_div(count),
                        (sy as u32 as i32).wrapping_div(count),
                    ]
                })
            })
            .collect();
        Self {
            width: map.width,
            centers: single::Centers::build(map, &centroids),
            centroids,
            borders,
        }
    }

    /// World `(x, z)` of a province's integer pixel centroid.
    pub fn centroid(&self, province: u32) -> Option<[f32; 2]> {
        self.pixel_centroid(province)
            .map(|[x, z]| [x as f32, z as f32])
    }

    /// The province's integer map position uses the pixel-centroid mean.
    /// A province with no pixels has zero fields and returns `None`.
    pub fn map_position(&self, province: u32) -> Option<[i32; 2]> {
        self.pixel_centroid(province)
    }

    fn pixel_centroid(&self, province: u32) -> Option<[i32; 2]> {
        self.centroids.get(province as usize).copied().flatten()
    }

    /// Provinces with pixels, for reporting.
    pub fn provinces(&self) -> usize {
        self.centroids.iter().flatten().count()
    }
}

/// Start at the least-connected pixel (Manhattan distance <5), then greedily
/// extend the nearer end. Swap-removal and first-tie order matter.
/// The local pixel census avoids a quadratic seed-neighbour scan.
pub(super) fn sort_edge_pixels(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let pixels: BTreeSet<_> = points.iter().map(|p| (p[0] as i32, p[1] as i32)).collect();
    let seed = points
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| {
            let mut count = 0;
            for dx in -4i32..=4 {
                for dz in -4i32..=4 {
                    if dx.abs() + dz.abs() < 5
                        && pixels.contains(&(p[0] as i32 + dx, p[1] as i32 + dz))
                    {
                        count += 1;
                    }
                }
            }
            count
        })
        .unwrap()
        .0;
    let mut remaining = points.to_vec();
    let mut out = vec![remaining.swap_remove(seed)];
    let squared = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    while !remaining.is_empty() {
        let mut tail = (0, f32::MAX);
        let mut head = (0, f32::MAX);
        for (i, &p) in remaining.iter().enumerate() {
            let dt = squared(p, *out.last().unwrap());
            let dh = squared(p, out[0]);
            if dt < tail.1 {
                tail = (i, dt);
            }
            if dh < dt - 0.001 && dh < head.1 {
                head = (i, dh);
            }
        }
        let index = if head.1 < tail.1 {
            out.reverse();
            head.0
        } else {
            tail.0
        };
        out.push(remaining.swap_remove(index));
    }
    out
}

// --------------------------------------------------------------- classification

/// Classification uses the recorded binding, never path multiplicity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classified {
    FrontLine,
    OffensiveLine,
    Move,
    Invasion,
    Paradrop,
    GarrisonArea,
    Unknown,
}

impl Classified {
    /// Bound style uses the owner's style and ignores overrides.
    /// Unbound styles use army-group styles only without an override.
    pub fn arrow_type(self, plan: &ArmyOrderPlanRow, army_group: bool) -> Option<&'static str> {
        let ag = army_group && plan.owner_override == 0;
        Some(match self {
            Self::FrontLine => {
                if army_group {
                    if plan.has_child == 0 {
                        "ag_defence_line_no_child"
                    } else {
                        "ag_defence_line"
                    }
                } else {
                    "defence_line"
                }
            }
            Self::OffensiveLine => {
                if plan.virtual_order != 0 {
                    "virtual_line"
                } else if plan.fallback != 0 {
                    "fallback_line"
                } else if plan.blitz != 0 {
                    "blitz_line"
                } else if plan.withdraw != 0 {
                    "withdraw_line"
                } else if ag {
                    "ag_offence_line"
                } else {
                    "offence_line"
                }
            }
            Self::Move => {
                if plan.blitz != 0 {
                    "blitz"
                } else if plan.withdraw != 0 {
                    "withdraw"
                } else if ag {
                    "ag_move_thick"
                } else {
                    "move_thick"
                }
            }
            Self::Invasion => "invasion",
            Self::Paradrop => "paradrop",
            Self::GarrisonArea | Self::Unknown => return None,
        })
    }
}

fn classify(order: &ArmyOrderRow, plan: Option<&ArmyOrderPlanRow>) -> Classified {
    if plan.is_some_and(|p| p.front_valid != 0) {
        return Classified::FrontLine;
    }
    match order.order_type {
        TYPE_MOVE => Classified::Move,
        TYPE_LINE if plan.is_some() => Classified::OffensiveLine,
        TYPE_LINE => Classified::Unknown,
        TYPE_INVASION => Classified::Invasion,
        TYPE_PARADROP => Classified::Paradrop,
        TYPE_AREA_DEFENSE => Classified::GarrisonArea,
        _ => Classified::Unknown,
    }
}

/// The province chain a move's or an unsnapped line's polyline runs along.
///
/// A move keeps its first two and last two provinces and, for counts of at
/// least five, every second one starting at the fourth between them.
/// Invasions and paradrops use their own builders ([`transport`]).
pub fn chain(order: &ArmyOrderRow, kind: Classified) -> Vec<u32> {
    match kind {
        Classified::Invasion
        | Classified::Paradrop
        | Classified::GarrisonArea
        | Classified::Unknown => Vec::new(),
        Classified::Move if order.path.len() >= 5 => order.path[..2]
            .iter()
            .chain(order.path[3..order.path.len() - 2].iter().step_by(2))
            .chain(&order.path[order.path.len() - 2..])
            .copied()
            .collect(),
        _ => order.path.clone(),
    }
}

// ------------------------------------------------------------------ the graph

type OrderKey = (u64, u32);

/// Walk the country's theatre roster, then each theatre's army-group roster
/// before its regular-group roster. A group without a recorded roster position
/// cannot be reached; a duplicated position is refused.
fn ordered_group_roots<'a>(
    groups: &BTreeMap<u64, &OrdersGroupRow>,
    theatres: &[LandTheatreRow],
    roots: &'a [OrdersGroupPlanRow],
    links: &BTreeMap<u32, u32>,
    countries: &[u32],
    show_allied: bool,
    refused: &mut BTreeMap<&'static str, usize>,
) -> Vec<&'a OrdersGroupPlanRow> {
    let recorded_theatres: BTreeSet<_> = theatres.iter().map(|theatre| theatre.id).collect();
    let unknown_owners: BTreeSet<_> = theatres
        .iter()
        .filter(|theatre| theatre.owner != 0 && !links.contains_key(&theatre.owner))
        .map(|theatre| theatre.id)
        .collect();
    let theatres: BTreeMap<_, _> = theatres
        .iter()
        .filter_map(|theatre| {
            let country = if theatre.owner == 0 {
                0
            } else {
                *links.get(&theatre.owner)?
            };
            countries
                .contains(&country)
                .then_some((theatre.id, (country, theatre.ordinal)))
        })
        .collect();
    let mut ordered: BTreeMap<u32, BTreeMap<_, _>> = BTreeMap::new();
    for root in roots {
        let Some(group) = groups.get(&root.group) else {
            *refused
                .entry("group plan whose orders group is not recorded")
                .or_default() += 1;
            continue;
        };
        // Allied plans skip the country guard. Admission belongs to the owning
        // theatre's country occurrence; the group's country may differ from that
        // theatre's owner.
        let same_country = if group.country == 0 {
            Some(countries.first() == Some(&0))
        } else {
            links
                .get(&group.country)
                .map(|country| Some(country) == countries.first())
        };
        if !show_allied && same_country == Some(false) {
            continue;
        }
        let Some(&(country, theatre)) = theatres.get(&group.theatre) else {
            if unknown_owners.contains(&group.theatre) {
                *refused
                    .entry("group whose theatre owner has no CountryLinks row")
                    .or_default() += 1;
            } else if !recorded_theatres.contains(&group.theatre) {
                *refused
                    .entry("group whose theatre roster is not recorded")
                    .or_default() += 1;
            }
            continue;
        };
        if !show_allied && same_country.is_none() {
            *refused
                .entry("group country has no CountryLinks row")
                .or_default() += 1;
            continue;
        }
        let Some(ordinal) = group.theatre_ordinal else {
            *refused
                .entry("group without a theatre roster position")
                .or_default() += 1;
            continue;
        };
        let key = (theatre, group.is_army_group == 0, ordinal);
        if ordered
            .entry(country)
            .or_default()
            .insert(key, root)
            .is_some()
        {
            *refused
                .entry("duplicate theatre roster position")
                .or_default() += 1;
        }
    }
    // Keep every admitted physical country occurrence, in order and with
    // repetitions, while theatre ordinals stay country-local.
    countries
        .iter()
        .filter_map(|country| ordered.get(country))
        .flat_map(|roots| roots.values().copied())
        .collect()
}

/// The first linked instance of the type, in recorded pointer-vector order.
fn first_linked<'a>(
    owner: u64,
    ids: &[u32],
    order_type: i32,
    orders: &BTreeMap<OrderKey, &'a ArmyOrderRow>,
) -> Option<&'a ArmyOrderRow> {
    ids.iter()
        .filter_map(|id| orders.get(&(owner, *id)).copied())
        .find(|order| order.order_type == order_type)
}

/// Visit primary roots then fallback roots, walking ordinary successors depth
/// first. Keep repeated visits: each rebuilds the graphic and advances overlap
/// counters. Refuse a group whose graph names an absent order or a root whose
/// walk cycles.
fn drawing_order<'a>(
    groups: &[&OrdersGroupPlanRow],
    orders: &BTreeMap<OrderKey, &'a ArmyOrderRow>,
    plans: &BTreeMap<OrderKey, &ArmyOrderPlanRow>,
    refused: &mut BTreeMap<&'static str, usize>,
) -> Vec<&'a ArmyOrderRow> {
    let mut output = Vec::new();
    for group in groups {
        let keys: Vec<OrderKey> = orders
            .range((group.group, 0)..=(group.group, u32::MAX))
            .map(|(key, _)| *key)
            .collect();
        if keys.is_empty() {
            continue;
        }
        let complete = keys.iter().all(|key| {
            plans.get(key).is_some_and(|plan| {
                plan.predecessors
                    .iter()
                    .chain(&plan.successors)
                    .all(|id| orders.contains_key(&(key.0, *id)))
            })
        }) && group
            .primary_orders
            .iter()
            .chain(&group.fallback_orders)
            .all(|id| orders.contains_key(&(group.group, *id)));
        if !complete {
            *refused
                .entry("group whose plan graph names an absent order")
                .or_default() += 1;
            continue;
        }
        for &root in group.primary_orders.iter().chain(&group.fallback_orders) {
            let start = output.len();
            let mut active = BTreeSet::new();
            let mut stack = vec![((group.group, root), false)];
            let mut cycled = false;
            while let Some((key, leaving)) = stack.pop() {
                if leaving {
                    active.remove(&key);
                    continue;
                }
                if !active.insert(key) {
                    cycled = true;
                    break;
                }
                stack.push((key, true));
                output.push(orders[&key]);
                let plan = plans[&key];
                if let [child] = plan.successors.as_slice() {
                    let next = plans[&(key.0, *child)];
                    if next.predecessors.len() > 1 && next.predecessors[0] != key.1 {
                        continue;
                    }
                }
                for &child in plan.successors.iter().rev() {
                    stack.push(((key.0, child), false));
                }
            }
            if cycled {
                output.truncate(start);
                *refused
                    .entry("root whose successor graph cycles")
                    .or_default() += 1;
            }
        }
    }
    output
}

/// A bound order's span of its section.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FrontSpan {
    /// The ordered province pairs, inclusive; empty for a start after the end.
    Pairs(Vec<[u32; 2]>),
    /// Empty the arrow's pair list and destroy the arrow. No arrow is drawn;
    /// this is not a refusal.
    Destroyed,
}

/// The signed inclusive span of a bound order's section. An invalid binding is
/// never clamped onto another part of the front.
///
/// A negative start or end or an end past the section is
/// [`FrontSpan::Destroyed`]. A start after the end passes that test: the arrow
/// keeps its pairs and reserves no overlap. Its inclusive span is empty, so
/// it emits no controls and builds no strip: the order draws nothing. `None`
/// is a section the recorded front does not hold, which the viewer cannot
/// place and refuses.
fn front_pairs(plan: &ArmyOrderPlanRow, front: &FrontRow) -> Option<FrontSpan> {
    let section = front
        .section_ids
        .iter()
        .position(|id| *id == plan.section)?;
    let first = if section == 0 {
        0
    } else {
        front.section_ends[section - 1] as usize
    };
    let words = front
        .province_pairs
        .get(first..front.section_ends[section] as usize)?;
    let (Ok(start), Ok(end)) = (
        usize::try_from(plan.segment_start),
        usize::try_from(plan.segment_end),
    ) else {
        return Some(FrontSpan::Destroyed);
    };
    if end >= words.len() / 2 {
        return Some(FrontSpan::Destroyed);
    }
    if start > end {
        return Some(FrontSpan::Pairs(Vec::new()));
    }
    Some(FrontSpan::Pairs(
        words[start * 2..(end + 1) * 2]
            .chunks_exact(2)
            .map(|pair| [pair[0], pair[1]])
            .collect(),
    ))
}

/// The stored control nearest the target, x normalized once by the world
/// width and taken at its nearer wrap; fixed5 half-deltas, strict less-than,
/// first tie kept.
fn nearest_control(points: &[Point], target: Point, width: i64) -> Option<Point> {
    let mut best = None;
    let mut distance = 0x2540_a5d60_i128;
    for &[mut x, z] in points {
        if width > 0 {
            if x > width {
                x -= width;
            } else if x < 0 {
                x += width;
            }
            if (x - target[0]).abs() > width / 2 {
                x += if target[0] < x { -width } else { width };
            }
        }
        let dx = i128::from((x - target[0]) / 2);
        let dz = i128::from((z - target[1]) / 2);
        let candidate = dx * dx / 100_000 + dz * dz / 100_000;
        if candidate < distance {
            distance = candidate;
            best = Some([x, z]);
        }
    }
    best
}

// ---------------------------------------------------------------- the geometry

/// Ribbon construction vertex; native upload uses `arrow_shader::Vertex`.
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
    declaration_index: usize,
    /// Comparator width: larger widths draw first in a bucket.
    width: f32,
    label: Option<usize>,
    native_strips: Vec<std::ops::Range<u32>>,
    bounds: [[f32; 3]; 2],
    /// Secondary colour of the inner pass and the two outer-pass colours,
    /// loaded from the arrow and initialized when the arrow is created.
    pub secondary_color: [f32; 4],
    pub outer_color: [f32; 4],
    /// Viewer-derived state for this hour. Child ribbons share the parent
    /// callback; generic raid arrows retain their constructor state.
    execution: execution::State,
}

/// Every order of one hour, in one buffer pair, and the plan labels.
#[derive(Clone, Debug, Default)]
pub struct OrderField {
    pub vertices: Vec<OrderVertex>,
    native_vertices: Vec<super::arrow_shader::Vertex>,
    pub draws: Vec<OrderDraw>,
    /// The name labels, drawn by a `mapname` pass after the ribbons.
    pub labels: Placements,
    /// The raid arrows' flying units ([`super::raids::arrows`]), which the
    /// unit model layer draws.
    pub raid_units: Vec<super::raids::arrows::UnitPlacement>,
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
    /// ([`super::raids::arrows`]).
    pub raid_arrows: super::raids::arrows::RaidArrowReport,
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
        out
    }
}

/// Every table [`build`] and its [`OrderInputs::links`] read. The document
/// keeps one hour's plan while these tables are shared (`crate::map::memo`)
/// and builds it from a frame holding only these, so a table missing here is
/// seen as unavailable rather than read stale, and an hour whose plan inputs
/// are unchanged reuses the previous field, its uploaded buffers and its
/// label generation.
// Invasions use the full frame outside this memo: adjacency-rule triggers
// can read any supported country-scope input (document::plan_field).
pub const TABLES: &[TableId] = &[
    TableId::ArmyOrders,
    TableId::OrdersGroups,
    TableId::Divisions,
    TableId::ArmyOrderPlans,
    TableId::Fronts,
    TableId::OrdersGroupPlans,
    TableId::LandTheatres,
    TableId::Provinces,
    TableId::CountryLinks,
];

/// Local allied admission adds dependencies only while that option is enabled.
pub(crate) fn memo_tables(show_allied: bool) -> Vec<TableId> {
    TABLES
        .iter()
        .chain(if show_allied { allied::TABLES } else { &[] })
        .copied()
        .collect()
}

/// Known own-order prefixes use the same admitted group roots as the draw.
/// Zero with unavailable inputs does not prove absence: the caller keeps table
/// limitations and the draw's named group refusals. This needs no geometry or
/// prepared order field, which the separate limitations job does not hold.
pub(crate) fn own_order_count(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
) -> io::Result<usize> {
    let groups = table_gate::rows::<tables::OrdersGroups>(frame)?;
    let groups = groups
        .rows()
        .iter()
        .map(|group| (group.handle, group))
        .collect();
    let theatres = table_gate::rows::<tables::LandTheatres>(frame)?;
    let roots = table_gate::rows::<tables::OrdersGroupPlans>(frame)?;
    let roots = ordered_group_roots(
        &groups,
        theatres.rows(),
        roots.rows(),
        links,
        &[ours],
        false,
        &mut BTreeMap::new(),
    );
    let reached: BTreeSet<_> = roots.iter().map(|root| root.group).collect();
    Ok(table_gate::rows::<tables::ArmyOrders>(frame)?
        .rows()
        .iter()
        .filter(|order| reached.contains(&order.owner_group))
        .count())
}

/// Invasion triggers, active callbacks and reached paradrop labels need the full hour.
/// This follows admitted theatre/group roots, including cross-country groups.
pub(crate) fn requires_full_frame(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
    show_allied: bool,
    labels: bool,
) -> io::Result<bool> {
    let groups = table_gate::rows::<tables::OrdersGroups>(frame)?;
    let groups = groups
        .rows()
        .iter()
        .map(|group| (group.handle, group))
        .collect();
    let theatres = table_gate::rows::<tables::LandTheatres>(frame)?;
    let roots = table_gate::rows::<tables::OrdersGroupPlans>(frame)?;
    let selection = allied::Selection::read(frame, links, ours, show_allied);
    let roots = ordered_group_roots(
        &groups,
        theatres.rows(),
        roots.rows(),
        links,
        &selection.countries,
        show_allied,
        &mut BTreeMap::new(),
    );
    let reached: BTreeSet<_> = roots.iter().map(|root| root.group).collect();
    Ok(table_gate::rows::<tables::ArmyOrders>(frame)?
        .rows()
        .iter()
        .any(|order| {
            reached.contains(&order.owner_group)
                && (order.active != 0
                    || order.order_type == TYPE_INVASION
                    || labels && order.order_type == TYPE_PARADROP)
        }))
}

/// What [`build`] needs beyond the frame.
pub struct OrderInputs<'a> {
    pub assets: &'a OrderAssets,
    pub raids: Option<&'a super::raids::RaidAssets>,
    pub geometry: &'a ProvinceGeometry,
    /// The content movement map: native neighbour order, kinds and crossings.
    pub movement: Option<&'a MovementMap>,
    pub map: &'a MapData,
    pub heights: &'a HeightField<'a>,
    /// The viewing country's canonical slot (`perspective::Vision::ours`),
    /// whose plan is drawn.
    pub ours: u32,
    /// Viewer-local allied-plan switch; never replayed from the client's UI.
    pub show_allied: bool,
    /// Raw country reference to canonical slot, the `CountryLinks` join.
    pub links: &'a BTreeMap<u32, u32>,
    /// The label font and localisation; `None` draws no labels.
    pub labels: Option<LabelInputs<'a>>,
}

/// One order's prepared geometry.
#[derive(Clone, Debug, Default)]
struct Prepared {
    /// The stored fixed5 controls.
    controls: Vec<Point>,
    /// A move's unsmoothed controls, re-curved once its end snaps.
    raw: Vec<Point>,
    /// Teeth side, initialized to zero.
    side: i32,
    snapped: bool,
    offset: bool,
    /// A paradrop's height callback, replacing the terrain height of every
    /// strip sample.
    lift: Option<transport::Lift>,
    /// A front whose span starts after it ends: the arrow exists natively but
    /// has no controls, so it draws nothing ([`front_pairs`]).
    empty_span: bool,
    /// The parent's final visit supplies its child graphics as well.
    children: Vec<group_sections::Prepared>,
}

fn group_color(row: &OrdersGroupPlanRow) -> [f32; 4] {
    [
        f32::from_bits(row.red),
        f32::from_bits(row.green),
        f32::from_bits(row.blue),
        f32::from_bits(row.alpha),
    ]
}

/// Ordinary read-only colour selection: virtual orders first, then the owner
/// group's country chooses its display group's stored colour or the allied define.
fn callback_color(
    defines: &OrderDefines,
    virtual_order: bool,
    owner_is_ours: Option<bool>,
    display: Option<&OrdersGroupPlanRow>,
) -> Option<[f32; 4]> {
    let color = if virtual_order {
        defines.virtual_color
    } else if owner_is_ours? {
        group_color(display?)
    } else {
        defines.allied_color
    };
    color.iter().all(|value| value.is_finite()).then_some(color)
}

fn owner_is_ours(group: &OrdersGroupRow, inputs: &OrderInputs<'_>) -> Option<bool> {
    if group.country == 0 {
        Some(inputs.ours == 0)
    } else {
        inputs
            .links
            .get(&group.country)
            .map(|&country| country == inputs.ours)
    }
}

/// Builds one hour's battle plan for the viewing country.
pub fn build(inputs: &OrderInputs<'_>, frame: &DecodedFrame) -> io::Result<OrderField> {
    let transport_context = paradrop_counts::Context::new(frame, inputs.raids);
    let orders = table_gate::rows::<tables::ArmyOrders>(frame)?;
    let groups = table_gate::rows::<tables::OrdersGroups>(frame)?;
    let plans = table_gate::rows::<tables::ArmyOrderPlans>(frame)?;
    let fronts = table_gate::rows::<tables::Fronts>(frame)?;
    let group_plans = table_gate::rows::<tables::OrdersGroupPlans>(frame)?;
    let theatres = table_gate::rows::<tables::LandTheatres>(frame)?;
    let child_division_groups = group_sections::DivisionGroups::read(frame);
    let plans: BTreeMap<_, _> = plans
        .rows()
        .iter()
        .map(|p| ((p.owner_group, p.instance), p))
        .collect();
    let fronts: BTreeMap<_, _> = fronts.rows().iter().map(|p| (p.front, p)).collect();
    let group_plan_rows: BTreeMap<_, _> = group_plans.rows().iter().map(|p| (p.group, p)).collect();
    let groups: BTreeMap<_, _> = groups
        .rows()
        .iter()
        .map(|group| (group.handle, group))
        .collect();
    let mut field = OrderField::default();
    field.report.recorded = orders.rows().len();
    for order in orders.rows() {
        let kind = classify(
            order,
            plans.get(&(order.owner_group, order.instance)).copied(),
        );
        *field
            .report
            .per_kind_recorded
            .entry(kind_name(kind))
            .or_default() += 1;
    }
    let selection = allied::Selection::read(frame, inputs.links, inputs.ours, inputs.show_allied);
    field.report.allied_refused = selection.refused;
    let mut refused = BTreeMap::new();
    let roots = ordered_group_roots(
        &groups,
        theatres.rows(),
        group_plans.rows(),
        inputs.links,
        &selection.countries,
        inputs.show_allied,
        &mut refused,
    );
    // Once a group is admitted, do not gate an order's country.
    let reached_groups: BTreeSet<_> = roots.iter().map(|root| root.group).collect();
    let mine: Vec<_> = orders
        .rows()
        .iter()
        .filter(|order| reached_groups.contains(&order.owner_group))
        .collect();
    field.report.ours = mine.len();
    if mine.is_empty() {
        field.report.refused = refused;
        return Ok(field);
    }
    let Some(movement) = inputs.movement else {
        field.report.unavailable = Some(
            "the content movement map (native neighbour order and crossings) could not be derived"
                .into(),
        );
        return Ok(field);
    };
    let controllers: BTreeMap<_, _> = table_gate::core::<tables::Provinces>(frame)?
        .iter()
        .map(|province| (province.id, province.controller))
        .collect();
    let indexed: BTreeMap<OrderKey, &ArmyOrderRow> = mine
        .iter()
        .map(|&order| ((order.owner_group, order.instance), order))
        .collect();
    let execution_context = execution::Context::new(
        frame,
        &inputs.assets.execution,
        &indexed,
        &plans,
        &groups,
        &transport_context,
    );
    let visits = drawing_order(&roots, &indexed, &plans, &mut refused);
    // Draw only what the roots reach; count an unreachable order (its group
    // refused above, or a graph island) without drawing it.
    let reached: BTreeSet<OrderKey> = visits
        .iter()
        .map(|order| (order.owner_group, order.instance))
        .collect();
    let unreached = indexed.len() - reached.len();
    if unreached > 0 {
        *refused
            .entry("order not reached from its group's recorded roots")
            .or_default() += unreached;
    }

    let assets = inputs.assets;
    let defines = &assets.defines;
    let geometry = inputs.geometry;
    let centroid = |province: u32| geometry.pixel_centroid(province);
    let neighbours = |province: u32| movement.neighbours(province);
    // Require a neighbour listed by the first province that is not impassable.
    let adjacent = |a: u32, b: u32| {
        movement.neighbours(a).is_some_and(|list| list.contains(&b))
            && movement
                .crossing_kind(a, b)
                .is_some_and(|kind| kind != IMPASSABLE)
    };
    let border_target = |a, b| movement.front_border_target(a, b);
    let offensive_world = offensive::World {
        centroid: &centroid,
        neighbours: &neighbours,
        adjacent: &adjacent,
    };
    let raster = chain::Raster {
        map: inputs.map,
        controllers: &controllers,
        links: inputs.links,
    };
    let chain_world = chain::ChainWorld {
        borders: &geometry.borders,
        border_target: &border_target,
        raster: Some(&raster),
    };
    // `None` when a path province has no pixel centroid: the order is refused,
    // never drawn through the provinces that remain.
    let centroid_controls = |path: &[u32], order_type: i32| -> Option<Vec<Point>> {
        let mut out = Vec::with_capacity(path.len() * 2);
        for (index, &province) in path.iter().enumerate() {
            let c = centroid(province)?;
            let mut point = fixed::from_pixel(c);
            // A move starts at its first adjacency crossing.
            if order_type == TYPE_MOVE
                && index == 0
                && path.len() > 1
                && let Some(crossing) = movement.neighbour_crossing(province, path[1])
            {
                point = fixed::from_pixel(crossing);
            }
            out.push(point);
            if let Some(&next) = path.get(index + 1)
                && let Some(crossing) = movement.neighbour_crossing(province, next)
            {
                out.push(fixed::from_pixel(crossing));
            }
        }
        Some(out)
    };
    const NO_CENTROID: &str = "order path province without a map centroid";

    // Geometry follows visit order: repeated visits advance overlap counters
    // (reset once per frame), and the last visit's geometry wins.
    let mut overlaps: BTreeMap<(u32, u32), u8> = BTreeMap::new();
    let mut prepared: BTreeMap<OrderKey, Prepared> = BTreeMap::new();
    let mut first_visit: Vec<OrderKey> = Vec::new();
    let mut seen: BTreeSet<OrderKey> = BTreeSet::new();
    // An order visited more than once is refused once per reason.
    let mut refused_orders: BTreeSet<(OrderKey, &'static str)> = BTreeSet::new();
    let mut invasion_routes = invasion::Routes::new(frame, inputs.raids);
    let mut refused_invasions = BTreeSet::new();
    for &order in &visits {
        let key = (order.owner_group, order.instance);
        if seen.insert(key) {
            first_visit.push(key);
        }
        let plan = plans[&key];
        // A missing group or arrow type is counted once, at emission.
        let Some(group) = groups.get(&order.owner_group) else {
            continue;
        };
        let kind = classify(order, Some(plan));
        let Some(arrow) = kind
            .arrow_type(plan, group.is_army_group != 0)
            .and_then(|name| assets.types.get(name))
        else {
            continue;
        };
        let mut out = Prepared::default();
        match kind {
            Classified::FrontLine => {
                let pairs = match fronts
                    .get(&plan.front)
                    .and_then(|front| front_pairs(plan, front))
                {
                    Some(FrontSpan::Pairs(pairs)) => pairs,
                    Some(FrontSpan::Destroyed) => Vec::new(),
                    None => {
                        refuse(
                            &mut refused,
                            &mut refused_orders,
                            key,
                            "front binding outside its recorded section",
                        );
                        continue;
                    }
                };
                if pairs.is_empty() {
                    out.empty_span = true;
                    prepared.insert(key, out);
                    continue;
                }
                // Reserve each pair once, twice for an army group, and store the first
                // returned count.
                let reservation = if group.is_army_group != 0 { 2 } else { 1 };
                let pairs: Vec<chain::Pair> = pairs
                    .iter()
                    .map(|&[source, target]| {
                        let count = overlaps.entry((source, target)).or_default();
                        let units = *count;
                        *count = count.saturating_add(reservation).min(defines.max_offsets);
                        chain::Pair {
                            source,
                            target,
                            units,
                        }
                    })
                    .collect();
                // Reserve pairs before destroying an empty-path root. Keep those overlap
                // side effects.
                if order.path.is_empty() {
                    out.empty_span = true;
                    prepared.insert(key, out);
                    continue;
                }
                let extra_units = match group_sections::extra_units(plan) {
                    Ok(value) => value,
                    Err(reason) => {
                        refuse(&mut refused, &mut refused_orders, key, reason);
                        continue;
                    }
                };
                let params = chain::ChainParams {
                    subdivided: true,
                    smoothing: defines.root_front,
                    extra_units,
                    offset: defines.root_front_offset,
                    max_intersection_tests: defines.root_front_max_intersection_tests_order,
                    probe_teeth: arrow.culling_direction != 0,
                };
                if group.is_army_group != 0 && order.order_type == TYPE_LINE {
                    match group_sections::prepare(
                        plan,
                        &pairs,
                        &chain_world,
                        assets.types.get(group_sections::TYPE_NAME),
                        defines,
                    ) {
                        Ok((children, reasons)) => {
                            out.children = children;
                            for reason in reasons {
                                refuse(&mut refused, &mut refused_orders, key, reason);
                            }
                        }
                        Err(reason) => refuse(&mut refused, &mut refused_orders, key, reason),
                    }
                }
                let Some(built) = chain::build(&pairs, &chain_world, params) else {
                    refuse(
                        &mut refused,
                        &mut refused_orders,
                        key,
                        "front whose border chain could not be built",
                    );
                    prepared.insert(key, out);
                    continue;
                };
                out.controls = built.points;
                out.side = built.side.unwrap_or(0);
                out.offset = built.offset;
            }
            Classified::OffensiveLine => {
                let predecessor =
                    first_linked(order.owner_group, &plan.predecessors, TYPE_MOVE, &indexed);
                let approach = predecessor
                    .map(|order| order.path.as_slice())
                    .unwrap_or_default();
                if let Some(snap) = offensive::snap(&order.path, approach, &offensive_world) {
                    let pairs: Vec<chain::Pair> = snap
                        .pairs
                        .iter()
                        .map(|&[source, target]| chain::Pair {
                            source,
                            target,
                            units: 0,
                        })
                        .collect();
                    let params = chain::ChainParams {
                        subdivided: false,
                        smoothing: defines.snapped_off_front,
                        extra_units: 0,
                        offset: defines.root_front_offset,
                        max_intersection_tests: defines.root_front_max_intersection_tests_order,
                        probe_teeth: false,
                    };
                    let Some(built) = chain::build(&pairs, &chain_world, params) else {
                        refuse(
                            &mut refused,
                            &mut refused_orders,
                            key,
                            "snapped offensive line whose border chain could not be built",
                        );
                        continue;
                    };
                    out.controls = built.points;
                    out.side = snap.side;
                    out.snapped = true;
                } else if order.path.len() == 1 {
                    out.controls =
                        geometry
                            .centers
                            .points(inputs.map, order.path[0], approach, centroid);
                    out.side = arrow.culling_direction;
                } else {
                    let Some(mut controls) = centroid_controls(&order.path, TYPE_LINE) else {
                        refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                        continue;
                    };
                    fixed::curve(&mut controls, defines.order_front, true);
                    if arrow.culling_direction != 0
                        && let Some(side) =
                            offensive::unsnapped_side(&controls, approach, &offensive_world)
                    {
                        out.side = side;
                    }
                    out.controls = controls;
                }
            }
            Classified::Move => {
                let Some(raw) = centroid_controls(&chain(order, kind), TYPE_MOVE) else {
                    refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                    continue;
                };
                out.raw = raw;
                let mut controls = out.raw.clone();
                fixed::curve(&mut controls, defines.order_move, true);
                out.controls = controls;
            }
            Classified::Paradrop => {
                // A source or first path target below 1 destroys the arrow. Both are signed
                // integers, so -1 (no province) is below 1.
                let source = order.paradrop_target;
                let target = order.path.first().copied().unwrap_or(0);
                if (source as i32) < 1 || (target as i32) < 1 {
                    no_arrow(&mut field.report, &mut refused_orders, key);
                    continue;
                }
                let (Some(from), Some(to)) = (centroid(source), centroid(target)) else {
                    refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                    continue;
                };
                out.controls = transport::paradrop_controls(
                    fixed::from_pixel(from),
                    fixed::from_pixel(to),
                    i64::from(geometry.width),
                );
                out.lift = transport::Lift::new(
                    &out.controls,
                    geometry.width as f32,
                    defines.paradrop_height_to_len,
                    defines.paradrop_height_max,
                    &|x, z| inputs.heights.native_height(x, z),
                );
            }
            Classified::Invasion => {
                // An empty path destroys the arrow.
                if order.path.is_empty() {
                    no_arrow(&mut field.report, &mut refused_orders, key);
                    continue;
                }
                let route = match invasion_routes.route(order, plan, group) {
                    Ok(route) if route.is_empty() => {
                        no_arrow(&mut field.report, &mut refused_orders, key);
                        continue;
                    }
                    Ok(route) => route,
                    Err(reason) => {
                        if refused_invasions.insert(key) {
                            *field.report.invasion_refused.entry(reason).or_default() += 1;
                        }
                        continue;
                    }
                };
                let mut path = Vec::with_capacity(route.len() + 2);
                if (order.paradrop_target as i32) > 0 {
                    path.push(order.paradrop_target);
                }
                path.extend(route);
                path.push(order.path[0]);
                let Some(mut controls) = transport::invasion_controls(
                    &path,
                    i64::from(geometry.width),
                    |id| centroid(id).map(fixed::from_pixel),
                    |a, b| movement.neighbour_crossing(a, b).map(fixed::from_pixel),
                ) else {
                    refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                    continue;
                };
                fixed::curve(&mut controls, defines.order_move, true);
                out.controls = controls;
            }
            Classified::GarrisonArea | Classified::Unknown => continue,
        }
        prepared.insert(key, out);
    }
    // A move's last control is the nearest stored control of its first successor
    // line, when that graphic exists.
    let width = i64::from(geometry.width) * SCALE;
    let snapped_ends: Vec<(OrderKey, Point)> = prepared
        .iter()
        .filter(|(_, p)| p.raw.len() > 1)
        .filter_map(|(key, _)| {
            let order = indexed.get(key)?;
            let plan = plans.get(key)?;
            let successor = first_linked(key.0, &plan.successors, TYPE_LINE, &indexed)?;
            let boundary = prepared.get(&(successor.owner_group, successor.instance))?;
            let target = fixed::from_pixel(centroid(*order.path.last()?)?);
            Some((*key, nearest_control(&boundary.controls, target, width)?))
        })
        .collect();
    for (key, end) in snapped_ends {
        let entry = prepared.get_mut(&key).expect("prepared above");
        if let Some(last) = entry.raw.last_mut() {
            *last = end;
        }
        let mut controls = entry.raw.clone();
        fixed::curve(&mut controls, defines.order_move, true);
        entry.controls = controls;
    }

    // Emission: one graphic per order, in first-visit order.
    let mut gap_total = 0u64;
    let mut gap_count = 0u64;
    let mut placed_labels = Vec::new();
    for key in first_visit {
        let order = indexed[&key];
        let plan = plans[&key];
        let kind = classify(order, Some(plan));
        let Some(group) = groups.get(&order.owner_group) else {
            *refused
                .entry("order whose owner group is not recorded")
                .or_default() += 1;
            continue;
        };
        let Some(name) = kind.arrow_type(plan, group.is_army_group != 0) else {
            if kind == Classified::Unknown {
                *refused
                    .entry("order of a type or binding no arrow is known for")
                    .or_default() += 1;
            } else {
                field.report.no_arrow += 1;
            }
            continue;
        };
        let Some(arrow) = assets.types.get(name) else {
            *refused
                .entry("arrow type the content's maparrows.txt does not declare")
                .or_default() += 1;
            continue;
        };
        // Every order without prepared geometry was refused above with its reason.
        let Some(entry) = prepared.get(&key) else {
            continue;
        };
        if entry.empty_span {
            field.report.no_arrow += 1;
            continue;
        }
        let callback = match execution_context.callback(order) {
            Ok(callback) => callback,
            Err(reason) => {
                *field.report.execution_refused.entry(reason).or_default() += 1;
                continue;
            }
        };
        let child_display = group_sections::Display {
            plan,
            group_plans: &group_plan_rows,
            groups: &groups,
            division_groups: &child_division_groups,
            owner_is_ours: owner_is_ours(group, inputs),
        };
        // The display group is the override, else the owner; its colour tints
        // the arrow and its HQ is not counted.
        let display = if plan.owner_override != 0 {
            plan.owner_override
        } else {
            order.owner_group
        };
        let virtual_order = plan.virtual_order != 0;
        let color = callback_color(
            defines,
            virtual_order,
            owner_is_ours(group, inputs),
            group_plan_rows.get(&display).copied(),
        );
        let Some(color) = color else {
            *refused
                .entry("order whose callback colour is unavailable or nonfinite")
                .or_default() += 1;
            group_sections::emit(
                &mut field,
                &mut placed_labels,
                inputs,
                key,
                &entry.children,
                &child_display,
                callback,
                &mut refused,
            );
            continue;
        };
        let display_group = groups.get(&display);
        let count = match assigned_count(plan, display_group.copied()) {
            Ok(count) => count,
            Err(reason) => {
                *refused.entry(reason).or_default() += 1;
                group_sections::emit(
                    &mut field,
                    &mut placed_labels,
                    inputs,
                    key,
                    &entry.children,
                    &child_display,
                    callback,
                    &mut refused,
                );
                continue;
            }
        };
        let controls = entry.controls.len();
        let scale = arrow.global_scale(controls);
        let terrain = |x: f32, z: f32, _: usize| ribbon_height(inputs.heights, x, z);
        // Call the height callback per stored control with its index `i` and `n`,
        // the control count plus two when the type has a head.
        let lifted = |_: f32, _: f32, control: usize| {
            entry.lift.map_or(0.0, |lift| {
                lift.at_control(control, controls, arrow.head_length > 0.0)
            })
        };
        let height: &dyn Fn(f32, f32, usize) -> f32 = if entry.lift.is_some() {
            &lifted
        } else {
            &terrain
        };
        let texture_choice = match texture_choice(arrow, count == 0 && !virtual_order) {
            Ok(choice) => choice,
            Err(reason) => {
                *refused.entry(reason).or_default() += 1;
                group_sections::emit(
                    &mut field,
                    &mut placed_labels,
                    inputs,
                    key,
                    &entry.children,
                    &child_display,
                    callback,
                    &mut refused,
                );
                continue;
            }
        };
        let Some(draw_index) = emit(&mut field, arrow, &entry.controls, scale, color, height)
        else {
            field.report.too_short += 1;
            group_sections::emit(
                &mut field,
                &mut placed_labels,
                inputs,
                key,
                &entry.children,
                &child_display,
                callback,
                &mut refused,
            );
            continue;
        };
        let draw = &mut field.draws[draw_index];
        // Use the unassigned mask when nothing is counted.
        draw.texture_choice = texture_choice;
        draw.culling_offset = culling_offset(arrow.culling_direction, entry.side);
        draw.execution = callback;
        field.report.drawn += 1;
        field.report.snapped += usize::from(entry.snapped);
        field.report.offset_fronts += usize::from(entry.offset);
        *field.report.per_kind.entry(kind_name(kind)).or_default() += 1;
        group_sections::emit(
            &mut field,
            &mut placed_labels,
            inputs,
            key,
            &entry.children,
            &child_display,
            callback,
            &mut refused,
        );
        if matches!(kind, Classified::OffensiveLine | Classified::Move) && !entry.snapped {
            for province in &order.path {
                let Some(c) = geometry.centroid(*province) else {
                    continue;
                };
                let gap = entry
                    .controls
                    .iter()
                    .map(|&control| span(fixed::to_world(control), c))
                    .fold(f32::MAX, f32::min);
                let milli = (gap * 1000.0).round().max(0.0) as u32;
                field.report.control_gap_max_milli = field.report.control_gap_max_milli.max(milli);
                gap_total += u64::from(milli);
                gap_count += 1;
            }
        }
        // The virtual branch never sets label text.
        let Some(label_inputs) = inputs.labels.filter(|_| !virtual_order) else {
            continue;
        };
        let group_name = group_plan_rows
            .get(&display)
            .map_or(&[][..], |row| row.name.as_slice());
        let composer = labels::Composer {
            localisation: label_inputs.localisation,
            order_type: order.order_type,
            name: labels::OrderName {
                prefix: &plan.name_prefix,
                name: &plan.name,
                first: &plan.name_first,
                second: &plan.name_second,
                unique: &plan.name_unique,
                postfix: &plan.name_postfix,
            },
            group_name,
            army_group: display_group.is_some_and(|group| group.is_army_group != 0),
            count,
        };
        let transport_failed = std::cell::Cell::new(false);
        let transports = || {
            let result = if order.order_type == TYPE_INVASION {
                transport_counts::invasion(plan)
            } else {
                transport_context.count(order, groups.get(&order.owner_group).copied())
            };
            if result.is_err() {
                transport_failed.set(true);
            }
            result
        };
        // Use the arrow's height callback where present (the paradrop lift),
        // otherwise terrain height.
        let terrain = |x: f32, z: f32, _: f32| inputs.heights.native_height(x, z);
        let lifted = |_: f32, _: f32, t: f32| entry.lift.map_or(0.0, |lift| lift.at(t));
        let height: &dyn Fn(f32, f32, f32) -> f32 = if entry.lift.is_some() {
            &lifted
        } else {
            &terrain
        };
        // Each variant is localised and checked only when the layout reaches
        // it; one it needs and cannot draw refuses the label by its reason.
        let placement = labels::place(
            &|variant| {
                if variant == labels::Variant::Full {
                    if count == 0 && display_group.is_none() {
                        return Err("order display group kind unavailable".into());
                    }
                    if (count > 0 || composer.army_group) && !group_plan_rows.contains_key(&display)
                    {
                        return Err("order display group name unavailable".into());
                    }
                }
                composer.drawn_with_transports(
                    variant,
                    controls,
                    defines.ext_text_points_limit,
                    &transports,
                )
            },
            &entry.controls,
            label_inputs.font,
            arrow.text_size,
            arrow.text_offset,
            height,
        );
        if transport_failed.get() {
            field.report.transports_omitted += 1;
        }
        match placement {
            Ok(Some(placed)) => {
                let glyphs = placed
                    .glyphs
                    .iter()
                    .map(|glyph| (glyph.corners, glyph.uv, glyph.page));
                match WorldName::from_glyphs(
                    format!("{}:{}", key.0, key.1),
                    placed.text,
                    glyphs,
                    color[3],
                ) {
                    Some(name) => {
                        field.draws[draw_index].label = Some(placed_labels.len());
                        placed_labels.push(name);
                        field.report.labels += 1;
                    }
                    None => field.report.labels_unfitted += 1,
                }
            }
            Ok(None) => field.report.labels_unfitted += 1,
            Err(reason) => *field.report.labels_refused.entry(reason).or_default() += 1,
        }
    }
    if let Some(mean) = gap_total.checked_div(gap_count) {
        field.report.control_gap_mean_milli = mean as u32;
    }
    sort_draws(&mut field.draws);
    field.labels = Placements {
        country: placed_labels,
        warnings: Vec::new(),
        generation: name_pass::next_generation(),
    };
    field.report.refused = refused;
    Ok(field)
}

/// Visit definition buckets in file order and, inside one, larger comparator
/// widths first; equal keys keep insertion order.
pub(crate) fn sort_draws(draws: &mut [OrderDraw]) {
    draws.sort_by(|a, b| {
        a.declaration_index.cmp(&b.declaration_index).then_with(|| {
            b.width
                .partial_cmp(&a.width)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
}

/// Append `more`'s ribbons after `field`'s in the shared arrow manager's
/// draw order. Raid and battle-plan arrows use the same manager, so a
/// definition bucket holds both. Labels and the report stay `field`'s.
pub(crate) fn append(field: &OrderField, more: &OrderField) -> OrderField {
    let mut out = field.clone();
    let base = out.native_vertices.len() as u32;
    out.vertices.extend_from_slice(&more.vertices);
    out.native_vertices.extend_from_slice(&more.native_vertices);
    out.raid_units.extend(more.raid_units.iter().cloned());
    out.draws.extend(more.draws.iter().map(|draw| {
        OrderDraw {
            native_strips: draw
                .native_strips
                .iter()
                .map(|strip| strip.start + base..strip.end + base)
                .collect(),
            label: None,
            ..draw.clone()
        }
    }));
    sort_draws(&mut out.draws);
    out
}

/// Counts `reason` once for the order `key`, however often the walk visits it.
fn refuse(
    refused: &mut BTreeMap<&'static str, usize>,
    counted: &mut BTreeSet<(OrderKey, &'static str)>,
    key: OrderKey,
    reason: &'static str,
) {
    if counted.insert((key, reason)) {
        *refused.entry(reason).or_default() += 1;
    }
}

/// Counts an order the game draws no arrow for once, however often the walk
/// visits it; it shares [`refuse`]'s per-order set under its own reason.
fn no_arrow(
    report: &mut OrderReport,
    counted: &mut BTreeSet<(OrderKey, &'static str)>,
    key: OrderKey,
) {
    if counted.insert((key, NO_ARROW)) {
        report.no_arrow += 1;
    }
}

/// The per-order dedupe key of [`no_arrow`].
const NO_ARROW: &str = "no arrow";

/// Keep the selected assigned/unassigned resource index verbatim.
/// A missing authored resource is not a request to substitute index zero.
fn texture_choice(arrow: &ArrowType, unassigned: bool) -> Result<usize, &'static str> {
    let choice = usize::from(unassigned);
    if choice >= arrow.masks.len() || choice >= arrow.patterns.len() {
        return Err("order selected texture mask or pattern absent from recorded content");
    }
    Ok(choice)
}

/// Virtual orders bypass assignments; other orders exclude the display
/// group's active HQ. Missing metadata cannot prove no HQ.
fn assigned_count(
    plan: &ArmyOrderPlanRow,
    group: Option<&OrdersGroupRow>,
) -> Result<usize, &'static str> {
    if plan.virtual_order != 0 || plan.assigned_divisions.is_empty() {
        return Ok(0);
    }
    let group = group.ok_or("order display group HQ inputs unavailable")?;
    let hq = if group.leader != 0 {
        group.hq_provider
    } else {
        0
    };
    Ok(plan
        .assigned_divisions
        .iter()
        .filter(|&&unit| unit != hq)
        .count())
}

/// Culling offset is 1 when the type declares a culling direction and the
/// arrow's stored side is another.
fn culling_offset(direction: i32, side: i32) -> f32 {
    f32::from(u8::from(direction != 0 && side != direction))
}

fn kind_name(kind: Classified) -> &'static str {
    match kind {
        Classified::FrontLine => "front line",
        Classified::OffensiveLine => "offensive line",
        Classified::Move => "offensive arrow",
        Classified::Invasion => "invasion arrow",
        Classified::Paradrop => "paradrop arrow",
        Classified::GarrisonArea => "garrison area",
        Classified::Unknown => "unknown",
    }
}

/// Wrap once, sample bilinear terrain at the texel centre, clamp to sea
/// level, then add the 0.01 lift.
pub(super) fn ribbon_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
    let width = heights.width as f32;
    let x = if x < 0.0 {
        x + width
    } else if x >= width {
        x - width
    } else {
        x
    };
    heights.border_height_at_world(x, z) + 0.01
}

fn span(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// Smooth the fixed controls once and emit one pair per smoothed control up
/// to the head root, adding no samples between them, then the head.
/// Return the new draw's index.
pub(super) fn emit(
    field: &mut OrderField,
    arrow: &ArrowType,
    controls: &[Point],
    scale: f32,
    color: [f32; 4],
    height: &dyn Fn(f32, f32, usize) -> f32,
) -> Option<usize> {
    let mut smoothed = controls.to_vec();
    fixed::smooth(&mut smoothed, 75_000);
    let points: Vec<_> = smoothed.into_iter().map(fixed::to_world).collect();
    if points.len() < 2 {
        return None;
    }
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0f32;
    lengths.push(0.0);
    for index in 1..points.len() {
        total += span(points[index - 1], points[index]);
        lengths.push(total);
    }
    if total <= 0.0 {
        return None;
    }
    // Geometry follows head_length, independently of shader technique.
    let has_head = arrow.head_length > 0.0;
    let head_length = if has_head {
        arrow.head_length * scale
    } else {
        0.0
    };
    let body_end = total - head_length;
    // A nonzero texture loop replaces the total-length divisor.
    let uv_to_length = if arrow.texture_loop != 0.0 {
        arrow.texture_loop
    } else {
        total
    };
    let base = field.vertices.len() as u32;
    let mut pairs = 0u32;
    let mut head_root = None;
    for (index, &point) in points.iter().enumerate() {
        let along = lengths[index];
        let cut = has_head && index > 0 && along >= body_end;
        let point = if cut {
            point_at(&points, &lengths, body_end)
        } else {
            point
        };
        let (nx, nz) = if cut {
            normal(&[point, points[points.len() - 1]], 0)
        } else {
            normal(&points, index)
        };
        // At the head cut, keep this control's distance for width and U, even
        // though the pair moves back to the head root.
        let half = (arrow.body_width[0]
            + (arrow.body_width[1] - arrow.body_width[0]) * (along / total))
            * scale;
        push_pair(
            field,
            point,
            nx,
            nz,
            half * arrow.body_uv_scale * 2.0,
            along / uv_to_length,
            0.0,
            &|x, z| height(x, z, index),
        );
        pairs += 1;
        if cut {
            head_root = Some((point, (nx, nz), half));
            break;
        }
    }
    let mut bounds = native::bounds(&field.vertices[base as usize..]);
    // A head always cuts the body: the last control lies at `total`.
    if let Some((root, root_normal, half)) = head_root {
        let last = base as usize + (pairs as usize - 1) * 2;
        let edge = [
            field.vertices[last].position,
            field.vertices[last + 1].position,
        ];
        let head_bounds = emit_head(
            field,
            root,
            root_normal,
            edge,
            head_length,
            half,
            points.len(),
            height,
        );
        native::extend_bounds(&mut bounds, head_bounds);
    }
    if pairs < 2 && !has_head {
        return None;
    }
    let (vertices, mut native_strips) =
        native::vertices(&field.vertices[base as usize..], points.len(), has_head);
    let native_base = field.native_vertices.len() as u32;
    for strip in &mut native_strips {
        strip.start += native_base;
        strip.end += native_base;
    }
    field.native_vertices.extend(vertices);
    field.draws.push(OrderDraw {
        arrow_type: arrow.name.clone(),
        texture_choice: 0,
        color,
        total_length: total,
        uv_to_length,
        culling_offset: 0.0,
        declaration_index: arrow.declaration_index,
        // Use the larger body start/end width; the constructor's zero highlight
        // term leaves nothing else in the deselected view.
        width: arrow.body_width[0].max(arrow.body_width[1]),
        label: None,
        native_strips,
        bounds,
        secondary_color: color,
        outer_color: [1.0, 1.0, 1.0, color[3]],
        execution: execution::State::default(),
    });
    Some(field.draws.len() - 1)
}

/// The head's base uses the body's 3D cross-slope; its tip extends from the
/// root by the authored head length towards the end.
#[allow(clippy::too_many_arguments)]
fn emit_head(
    field: &mut OrderField,
    root: [f32; 2],
    (nx, nz): (f32, f32),
    edge: [[f32; 3]; 2],
    length: f32,
    half: f32,
    count: usize,
    height: &dyn Fn(f32, f32, usize) -> f32,
) -> [[f32; 3]; 2] {
    let tip = [root[0] + nz * length, root[1] - nx * length];
    let base = field.vertices.len() as u32;
    for (point, u, control) in [(root, 0.0, count + 1), (tip, 1.0, count + 2)] {
        push_pair(field, point, nx, nz, half * 2.0, u, 1.0, &|x, z| {
            height(x, z, control)
        });
    }
    // Correct the root cross-slope after updating the bounds.
    let bounds = native::bounds(&field.vertices[base as usize..]);
    let delta = std::array::from_fn::<_, 3, _>(|axis| edge[0][axis] - edge[1][axis]);
    let span = delta.iter().map(|value| value * value).sum::<f32>().sqrt();
    if span > 0.0 {
        for (index, side) in [(0, 1.0), (1, -1.0)] {
            field.vertices[base as usize + index].position = std::array::from_fn(|axis| {
                (edge[0][axis] + edge[1][axis]) * 0.5 + delta[axis] / span * half * side
            });
        }
    }
    bounds
}

#[allow(clippy::too_many_arguments)]
fn push_pair(
    field: &mut OrderField,
    point: [f32; 2],
    nx: f32,
    nz: f32,
    width: f32,
    u: f32,
    is_head: f32,
    height: &dyn Fn(f32, f32) -> f32,
) {
    let half = width * 0.5;
    // The positive perpendicular edge carries v=0; the opposite edge v=1.
    for (side, v) in [(1.0f32, 0.0f32), (-1.0, 1.0)] {
        let x = point[0] + nx * half * side;
        let z = point[1] + nz * half * side;
        let y = height(x, z);
        field.vertices.push(OrderVertex {
            position: [x, y, z],
            uv_head_variant: [u, v, is_head, 0.0],
        });
    }
}

fn normal(points: &[[f32; 2]], index: usize) -> (f32, f32) {
    let p = points[index];
    let previous = points[index.saturating_sub(1)];
    let next = points[(index + 1).min(points.len() - 1)];
    let direction = |a: [f32; 2], b: [f32; 2]| {
        let l = span(a, b);
        if l > 1e-6 {
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        } else {
            [0.0, 0.0]
        }
    };
    let a = direction(previous, p);
    let b = direction(p, next);
    let t = [a[0] + b[0], a[1] + b[1]];
    let l = span([0.0, 0.0], t);
    if l > 1e-6 {
        (-t[1] / l, t[0] / l)
    } else {
        (0.0, 1.0)
    }
}

fn point_at(points: &[[f32; 2]], lengths: &[f32], distance: f32) -> [f32; 2] {
    for index in 1..points.len() {
        if lengths[index] >= distance {
            let run = lengths[index] - lengths[index - 1];
            let t = if run < 1e-6 {
                0.0
            } else {
                (distance - lengths[index - 1]) / run
            };
            return [
                points[index - 1][0] + (points[index][0] - points[index - 1][0]) * t,
                points[index - 1][1] + (points[index][1] - points[index - 1][1]) * t,
            ];
        }
    }
    points[points.len() - 1]
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
    pub(crate) execution: execution::Defines,
    /// The raid arrows' defines ([`super::raids::arrows`]), or why they
    /// cannot be read; the battle plan does not need them.
    pub raid_defines: Result<installed_content::raids::RaidGraphicsDefines, String>,
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
/// ([`installed_content::raids::RaidArrowKind::arrow_type`]) are uploaded with them.
pub const DRAWN_ARROW_TYPES: &[&str] = &[
    "defence_line",
    "ag_defence_line_no_child",
    group_sections::TYPE_NAME,
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
];

impl OrderAssets {
    /// `gfx/maparrows` is outside content coverage, so its files and textures
    /// are read unverified, like the tree meshes and rail strip.
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &Defines,
        warnings: &mut Vec<installed_content::Diagnostic>,
    ) -> io::Result<Self> {
        let types = ArrowTypes::parse(&layers.read_uncovered(MAPARROWS)?)?;
        let defines_source = defines;
        let defines = OrderDefines::read(defines)?;
        let mut pages = BTreeMap::new();
        let raid_types = installed_content::raids::RaidArrowKind::ALL.map(|kind| kind.arrow_type());
        for name in DRAWN_ARROW_TYPES.iter().chain(raid_types.iter()) {
            let Some(arrow) = types.get(name) else {
                warnings.push((installed_content::DiagnosticKind::Refused, if raid_types.contains(name) {
                    format!("{MAPARROWS} declares no arrowType {name}; raid arrows that would use it are not drawn.")
                } else {
                    format!("{MAPARROWS} declares no arrowType {name}; battle-plan orders that would use it are not drawn.")
                }));
                continue;
            };
            for choice in 0..arrow.masks.len().min(arrow.patterns.len()) {
                match page(layers, decode, arrow, choice) {
                    Ok(page) => {
                        pages.insert(((*name).to_owned(), choice), page);
                    }
                    Err(error) => warnings.push((
                        installed_content::DiagnosticKind::Refused,
                        format!("arrowType {name} texture choice {choice}: {error}"),
                    )),
                }
            }
        }
        warnings.push((installed_content::DiagnosticKind::Warning,
            "Battle-plan arrows and their textures are read from gfx/maparrows, which is outside the content coverage policy and is not hash-verified."
                .into(),
        ));
        let raid_defines = installed_content::raids::RaidGraphicsDefines::read(defines_source)
            .map_err(|error| error.to_string());
        Ok(Self {
            types,
            pages,
            defines,
            raid_defines,
            execution: execution::Defines::read(defines_source),
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
        let image =
            super::textures::load_file(path, &|path| layers.read_uncovered_path(path), decode)?;
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

// ------------------------------------------------------------------- the pass

/// 128-byte allocation; bytes 116..128 remain unknown to shader reads.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Slot {
    arrow_mask: [f32; 4],
    arrow_color: [f32; 4],
    arrow_secondary_color: [f32; 4],
    /// `vTime_IsSelected_FadeInOut`.
    time_selected_fade: [f32; 4],
    /// `vOfsX_vOfsTex_vOfsFx`, with `w` unused.
    ofs_x_tex_fx: [f32; 4],
    body_uv_scale: f32,
    secondary_body_uv_scale: f32,
    texture_variant_scale: f32,
    culling_offset: f32,
    shadow_factor: f32,
    normal_map_factor: f32,
    seethrough_opacity: f32,
    total_length: f32,
    uv_to_absolute_length: f32,
    _padding: [f32; 3],
}

struct TypeResources {
    compiled: BTreeMap<Mode, std::sync::Arc<super::arrow_shader::Arrow>>,
    mask: wgpu::TextureView,
    pattern: wgpu::TextureView,
    arrow: ArrowType,
    variants: u32,
}

/// Physical PS slots 2..10; b0 is the original camera (b3 carries wrap).
#[derive(Clone, Copy)]
pub struct NativeInputs<'a> {
    pub retained: Option<&'a std::cell::RefCell<super::shader_scene::State>>,
    pub scene: &'a [u8],
    pub grid: &'a [u8],
    pub textures: [&'a wgpu::TextureView; 9],
}

struct NativeDraw<'a> {
    draw: &'a OrderDraw,
    program: &'a super::arrow_shader::Arrow,
    bound: shader_pipeline::gpu::Bindings,
}

/// The GPU battle-plan pass.
pub struct OrderPass {
    types: BTreeMap<(String, usize), TypeResources>,
    native_vertices: Option<wgpu::Buffer>,
    native_bytes: std::sync::Arc<[u8]>,
    draws: Vec<OrderDraw>,
    native_plans: BTreeMap<Mode, Vec<native::View>>,
}

impl OrderPass {
    pub fn prepare(&mut self, camera: super::camera::Camera, size: [u32; 2], wraps: &[f32]) {
        self.native_plans = native::plans(&self.draws, camera, size, wraps, |draw, mode| {
            self.types
                .get(&(draw.arrow_type.clone(), draw.texture_choice))
                .filter(|resources| resources.compiled.contains_key(&mode))
                .map(|resources| &resources.arrow)
        });
    }

    /// Native labels follow arrow admission, independently of glyph bounds.
    pub fn labels(&self, mode: Mode) -> Vec<(usize, usize)> {
        self.native_plans
            .get(&mode)
            .into_iter()
            .flatten()
            .enumerate()
            .flat_map(|(view, plan)| plan.labels.iter().map(move |source| (view, *source)))
            .collect()
    }
    /// Builds the pipeline and uploads every arrow type's pages.
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        effect: &std::sync::Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<Self> {
        let mut pass = Self {
            types: BTreeMap::new(),
            native_vertices: None,
            native_bytes: Default::default(),
            draws: Vec::new(),
            native_plans: BTreeMap::new(),
        };
        pass.upload_types(device, samples, queue, assets, effect)?;
        Ok(pass)
    }

    fn upload_types(
        &mut self,
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        effect: &std::sync::Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<()> {
        let mut programs = BTreeMap::new();
        for (key, page) in &assets.pages {
            let name = &key.0;
            let Some(arrow) = assets.types.get(name) else {
                continue;
            };
            let mask = upload(
                device,
                queue,
                &format!("TexMask ({name})"),
                &page.mask,
                ColorSpace::Linear,
            )?;
            let pattern = upload(
                device,
                queue,
                &format!("TexPattern ({name})"),
                &page.pattern,
                ColorSpace::Srgb,
            )?;
            let mut compiled = BTreeMap::new();
            for mode in [Mode::Shadow, Mode::Main, Mode::Seethrough, Mode::Late] {
                if !mode.admits(arrow.draw_last) || (mode == Mode::Shadow && !arrow.draw_shadows) {
                    continue;
                }
                // Append the effect suffix before lookup.
                let name = if mode == Mode::Seethrough {
                    format!("{}Seethrough", arrow.shader_technique)
                } else {
                    arrow.shader_technique.clone()
                };
                if mode == Mode::Seethrough && effect.select(&name).map_err(invalid)?.is_none() {
                    continue; // Absent optional technique skips mode 3.
                }
                let key = (name.clone(), mode);
                let program = if let Some(program) = programs.get(&key) {
                    std::sync::Arc::clone(program)
                } else {
                    let program = std::sync::Arc::new(super::arrow_shader::Arrow::new(
                        device,
                        effect,
                        &name,
                        if mode == Mode::Shadow { 1 } else { samples },
                        if mode == Mode::Shadow {
                            super::shadow::DEPTH_MAP_FORMAT
                        } else {
                            super::scene::SCENE_FORMAT
                        },
                        DEPTH_FORMAT,
                        mode,
                    )?);
                    programs.insert(key, program.clone());
                    program
                };
                compiled.insert(mode, program);
            }
            self.types.insert(
                key.clone(),
                TypeResources {
                    compiled,
                    mask: mask.view,
                    pattern: pattern.view,
                    arrow: arrow.clone(),
                    variants: page.variants,
                },
            );
        }
        Ok(())
    }

    /// Uploads one hour's ribbons. Called whenever the hour changes.
    pub fn upload(&mut self, device: &wgpu::Device, field: &OrderField) -> io::Result<()> {
        self.draws.clear();
        self.native_bytes = bytemuck::cast_slice(&field.native_vertices).into();
        if field.native_vertices.is_empty() {
            self.native_vertices = None;
            return Ok(());
        }
        self.native_vertices = Some(buffer(
            device,
            "native battle-plan ribbon vertices",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&field.native_vertices),
        )?);
        self.draws = field.draws.clone();
        Ok(())
    }

    fn prepare_bucket(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        native: NativeInputs<'_>,
        mode: Mode,
        time: f32,
        bucket: &[native::Draw],
    ) -> io::Result<Vec<NativeDraw<'_>>> {
        use installed_content::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let mut textures: BTreeMap<_, _> = native
            .textures
            .iter()
            .enumerate()
            .map(|(index, view)| (binding(index as u32 + 2, Stage::Pixel), (*view).clone()))
            .collect();
        let mut draws = Vec::new();
        for outer in [false, true] {
            for admitted in bucket {
                let draw = &self.draws[admitted.index];
                let resources = &self.types[&(draw.arrow_type.clone(), draw.texture_choice)];
                let mask = if outer {
                    resources.arrow.mask_outer
                } else {
                    resources.arrow.mask_inner
                };
                // Skip the entire loop when all RGB mask components are zero.
                if mask == [0.; 3] {
                    continue;
                }
                textures.insert(binding(0, Stage::Pixel), resources.mask.clone());
                textures.insert(binding(1, Stage::Pixel), resources.pattern.clone());
                for &offset in &admitted.offsets {
                    let constants = Self::slot(draw, resources, outer, offset, time, mode);
                    let program = &resources.compiled[&mode];
                    let bound = program.bindings(
                        arena,
                        native.scene,
                        native.grid,
                        bytemuck::bytes_of(&constants),
                        &textures,
                    )?;
                    if let Some(retained) = native.retained {
                        let mut retained = retained.borrow_mut();
                        program.retain(
                            &mut retained,
                            native.scene,
                            native.grid,
                            bytemuck::bytes_of(&constants),
                            &textures,
                        );
                        retained.bind_vertex(0, 32, self.native_bytes.clone());
                    }
                    draws.push(NativeDraw {
                        draw,
                        program,
                        bound,
                    });
                }
            }
        }
        Ok(draws)
    }

    /// Submit mode 2 after mesh/tree casters and before receivers.
    pub fn draw_shadow(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        native: NativeInputs<'_>,
        view: usize,
        time: f32,
    ) -> io::Result<()> {
        let Some(vertices) = &self.native_vertices else {
            return Ok(());
        };
        let Some(plan) = self
            .native_plans
            .get(&Mode::Shadow)
            .and_then(|views| views.get(view))
        else {
            return Ok(());
        };
        pass.set_vertex_buffer(0, vertices.slice(..));
        for bucket in &plan.buckets {
            for draw in self.prepare_bucket(arena, native, Mode::Shadow, time, bucket)? {
                draw.program.begin(pass);
                draw.bound.bind(pass);
                for strip in &draw.draw.native_strips {
                    pass.draw(strip.clone(), 0..1);
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        native: NativeInputs<'_>,
        mode: Mode,
        view: usize,
        time: f32,
    ) -> io::Result<()> {
        let Some(vertices) = &self.native_vertices else {
            return Ok(());
        };
        let Some(plan) = self
            .native_plans
            .get(&mode)
            .and_then(|views| views.get(view))
        else {
            return Ok(());
        };
        for bucket in &plan.buckets {
            let draws = self.prepare_bucket(arena, native, mode, time, bucket)?;
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("compiled map arrows"),
                    color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                    depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_vertex_buffer(0, vertices.slice(..));
                for draw in draws {
                    draw.program.begin(&mut pass);
                    draw.bound.bind(&mut pass);
                    for strip in &draw.draw.native_strips {
                        pass.draw(strip.clone(), 0..1);
                    }
                }
            }
            if mode == Mode::Seethrough {
                continue;
            }
            // Clear the whole stencil after both masks in modes 0/1.
            let mut attachment = targets.depth_attachment(wgpu::LoadOp::Load);
            attachment.stencil_ops = Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0),
                store: wgpu::StoreOp::Store,
            });
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("arrow definition stencil clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(attachment),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        Ok(())
    }

    fn slot(
        draw: &OrderDraw,
        resources: &TypeResources,
        outer: bool,
        offset: f32,
        time: f32,
        mode: Mode,
    ) -> Slot {
        let arrow = &resources.arrow;
        let [texture_speed, execution_fx] =
            draw.execution.shader_terms(arrow.texture_anim_speed, outer);
        Slot {
            arrow_mask: {
                let m = if outer {
                    arrow.mask_outer
                } else {
                    arrow.mask_inner
                };
                [m[0], m[1], m[2], 1.0]
            },
            arrow_color: if outer { draw.outer_color } else { draw.color },
            arrow_secondary_color: if outer {
                draw.outer_color
            } else {
                draw.secondary_color
            },
            time_selected_fade: [time, 0.0, arrow.fade_in, arrow.fade_out],
            ofs_x_tex_fx: [offset, texture_speed, execution_fx, 0.0],
            body_uv_scale: arrow.body_uv_scale,
            secondary_body_uv_scale: arrow.secondary_body_uv_scale,
            texture_variant_scale: 1.0 / resources.variants.max(1) as f32,
            culling_offset: draw.culling_offset,
            shadow_factor: mode.lighting(),
            normal_map_factor: mode.lighting(),
            total_length: draw.total_length,
            uv_to_absolute_length: draw.uv_to_length,
            seethrough_opacity: 0.2,
            _padding: [0.0; 3],
        }
    }
}

#[cfg(test)]
mod tests;
