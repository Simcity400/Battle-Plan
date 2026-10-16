//! Border-mesh and strait classification, uploads and compiled-content draws.
//!
//! [`super::borders`] traces and widens strips once from static province and
//! height rasters. Sea, sea-region and impassable classes come from content;
//! land edges initially use [`BorderClass::Province`]. Each hour,
//! [`BorderPass::set_classes`] rebuilds index buffers over the shared vertices
//! as canonical ownership and state boundaries change.
use std::{
    collections::{BTreeMap, HashSet},
    io,
    sync::Arc,
};
use viewer_foundation::map::textures::load_uploadable;

use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    adjacencies::{Adjacency, IMPASSABLE},
    image::{dds, native::Decode},
    map::{MapData, ProvinceKind},
    regions::StrategicRegion,
    states::StateDefinition,
};

use {
    super::{
        borders::{self, BorderClass, BorderDraw, BorderMeshes},
        demilitarized::Zone,
        political::Regions,
    },
    viewer_foundation::map::{
        gpu_contract::{DEPTH_FORMAT, buffer, invalid},
        province_map::WATER_HEIGHT,
        textures::{ColorSpace, upload},
    },
};

/// Width of a strait ribbon in world units, and the pitch it tiles `strait.dds` at along its run.
///
/// The ribbon is four world units wide and repeats one texture tile every four units along its run,
/// preserving square tiles.
pub const STRAIT_WIDTH: f32 = 4.0;

/// Bytes one border vertex occupies: `float3 position` then `6G16_SINT uv`.
const BORDER_VERTEX_STRIDE: u64 = 16;
pub use super::strait_shader::StraitVertex;

/// The border and strait textures, decoded but not uploaded.
pub struct BorderAssets {
    pub border_shader: Arc<shader_pipeline::library::Effect>,
    pub strait_shader: Arc<shader_pipeline::library::Effect>,
    /// `map/terrain/border_<class>_<quality>.dds`, in [`BorderClass::ALL`] order.
    pub classes: Vec<dds::Texture>,
    /// `map/terrain/strait.dds`, which has no quality suffix.
    pub strait: dds::Texture,
    /// `NDefines.NGraphics.BORDER_COLOR_DEMILITARIZED_R/G/B/A`: selection
    /// palette entry 7, the demilitarised line's `vSelectionColor`.
    pub demilitarized_color: [f32; 4],
    /// The content-selected custom palette entry used by faction-theatre hover.
    /// Unsupported definitions withhold this highlight alone.
    pub faction_highlight_color: Result<[f32; 4], String>,
}

impl BorderAssets {
    /// Select the border class suffix from map quality and read all seven textures through the
    /// presentation layers, so a selected visual mod's borders replace them. Recorded map
    /// content is hash-verified.
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content::defines::Defines,
        map: [u32; 2],
        texture_quality: u8,
        decode: Decode,
    ) -> io::Result<Self> {
        let channel = |name: &str| -> io::Result<f32> {
            Ok(defines.number("NGraphics", &format!("BORDER_COLOR_DEMILITARIZED_{name}"))? as f32)
        };
        // Checked for the colour space each is uploaded in (classes sRGB, strait linear).
        let texture = |path: String, space: ColorSpace| -> io::Result<dds::Texture> {
            layers.artwork(&path, |stack| {
                load_uploadable(&path, &|request| stack.read_path(request), decode, space)
            })
        };
        Ok(Self {
            border_shader: super::border_shader::load(layers, map, decode)?,
            strait_shader: super::strait_shader::load(layers, map, decode)?,
            classes: BorderClass::ALL
                .iter()
                .map(|class| {
                    texture(
                        format!(
                            "map/terrain/border_{}_{texture_quality}.dds",
                            class.texture_name()
                        ),
                        ColorSpace::Srgb,
                    )
                })
                .collect::<io::Result<Vec<_>>>()?,
            strait: texture("map/terrain/strait.dds".into(), ColorSpace::Linear)?,
            demilitarized_color: [channel("R")?, channel("G")?, channel("B")?, channel("A")?],
            faction_highlight_color: faction_highlight_color(defines)
                .map_err(|why| why.to_string()),
        })
    }
}

/// Use the configured faction-highlight palette index. Custom RGBA entries begin at palette
/// index 10.
fn faction_highlight_color(defines: &installed_content::defines::Defines) -> io::Result<[f32; 4]> {
    let index = defines.int32("NMapMode", "FACTION_THEATER_HIGHLIGHT_COLOR_INDEX")?;
    if !(0..=i32::from(u8::MAX) - 10).contains(&index) {
        return Err(invalid(
            "FACTION_THEATER_HIGHLIGHT_COLOR_INDEX exceeds the native reason byte",
        ));
    }
    let start = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_mul(4))
        .ok_or_else(|| {
            invalid("FACTION_THEATER_HIGHLIGHT_COLOR_INDEX is outside the custom palette")
        })?;
    let colors = defines.numbers("NGraphics", "BORDER_COLOR_CUSTOM_HIGHLIGHTS")?;
    let rgba = colors.get(start..start.saturating_add(4)).ok_or_else(|| {
        invalid("FACTION_THEATER_HIGHLIGHT_COLOR_INDEX names no complete custom RGBA quad")
    })?;
    Ok(std::array::from_fn(|channel| rgba[channel] as f32))
}

/// The content-derived facts the border classifier needs.
///
/// All four come from files the replay's layer roster hash-verifies: `map/definition.csv`,
/// `map/strategicregions`, `history/states` and `map/adjacencies.csv`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    /// Per province id: the province is sea or lake.
    water: Vec<bool>,
    /// Land with no non-sea land adjacency.
    hidden: Vec<bool>,
    /// Per province id: the strategic region that owns it, zero when none does.
    region_of: Vec<u32>,
    /// Per province id: the province belongs to a state `history/states` marks `impassable`.
    impassable: Vec<bool>,
    /// Province pairs an `impassable` row of `map/adjacencies.csv` separates, ascending.
    impassable_pairs: HashSet<(u32, u32)>,
}

impl Topology {
    /// Builds the tables from the loaded content.
    ///
    /// Provinces the map does not define are ignored rather than refused: `history/states` and
    /// `map/strategicregions` are authored by hand, and a stale id in one of them is not a reason to
    /// refuse to draw a map.
    pub fn new(
        map: &MapData,
        states: &BTreeMap<u32, StateDefinition>,
        regions: &BTreeMap<u32, StrategicRegion>,
        adjacencies: &[Adjacency],
    ) -> io::Result<Self> {
        let largest = map
            .definitions
            .last_key_value()
            .map(|(&id, _)| id)
            .unwrap_or(0)
            .max(map.province_ids.iter().copied().max().unwrap_or(0));
        let count = usize::try_from(largest)
            .ok()
            .and_then(|id| id.checked_add(1))
            .ok_or_else(|| invalid("province ids exceed address space"))?;
        let mut topology = Self {
            water: vec![false; count],
            hidden: vec![false; count],
            region_of: vec![0; count],
            impassable: vec![false; count],
            impassable_pairs: HashSet::new(),
        };
        for (&id, definition) in &map.definitions {
            if let Some(slot) = topology.water.get_mut(id as usize) {
                *slot = matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake);
            }
        }
        for region in regions.values() {
            for &province in &region.provinces {
                if let Some(slot) = topology.region_of.get_mut(province as usize) {
                    *slot = region.id;
                }
            }
        }
        for state in states.values().filter(|state| state.impassable) {
            for &province in &state.provinces {
                if let Some(slot) = topology.impassable.get_mut(province as usize) {
                    *slot = true;
                }
            }
        }
        // Read the first adjacency record naming the other province. Later authored rows rewrite the
        // kind on both sides of an existing pair, so the last row decides.
        let mut kinds = BTreeMap::new();
        for adjacency in adjacencies {
            kinds.insert(pair(adjacency.from, adjacency.to), adjacency.kind);
        }
        topology.impassable_pairs = kinds
            .into_iter()
            .filter(|&(_, kind)| kind == IMPASSABLE)
            .map(|(pair, _)| pair)
            .collect();
        topology.hidden = hidden_provinces(map, count);
        Ok(topology)
    }

    /// Whether the province is sea or lake, the both-sides-water test `borders::trace` smooths on.
    pub fn is_water(&self, province: u32) -> bool {
        self.water(province)
    }

    fn water(&self, province: u32) -> bool {
        self.water.get(province as usize).copied().unwrap_or(false)
    }

    fn hidden(&self, province: u32) -> bool {
        self.hidden.get(province as usize).copied().unwrap_or(false)
    }

    fn region(&self, province: u32) -> u32 {
        self.region_of
            .get(province as usize)
            .copied()
            .unwrap_or_default()
    }

    fn impassable(&self, a: u32, b: u32) -> bool {
        let flagged = |province: u32| {
            self.impassable
                .get(province as usize)
                .copied()
                .unwrap_or(false)
        };
        flagged(a) != flagged(b) || self.impassable_pairs.contains(&pair(a, b))
    }

    /// The class an edge takes from content alone, or `None` when the hour decides it.
    ///
    /// Classifies a pair in material-priority order: water pairs use sea-region or
    /// sea material; land pairs then test hidden edges, impassability, canonical
    /// country ownership and state boundaries. Remaining land pairs are province
    /// material. Coast pairs have no ordinary material.
    ///
    /// The rules this settles, and what each is evidence for:
    ///
    /// Coasts have no ordinary material and are suppressed. Water pairs use Sea within a strategic
    /// region and SeaRegion across regions. Impassability on exactly one state, or an impassable
    /// authored adjacency, precedes country classification. Other land pairs use the hourly land
    /// class.
    ///
    /// Bit 2 hides land with no land neighbour whose kind differs from sea. Compute it before
    /// authored adjacency overrides; see hidden_provinces.
    pub fn static_class(&self, a: u32, b: u32) -> Option<BorderClass> {
        if a == b || self.hidden(a) || self.hidden(b) {
            return None;
        }
        if self.water(a) || self.water(b) {
            if !(self.water(a) && self.water(b)) {
                // A coast: material 0xff has no ordinary bucket. A custom
                // highlight may remap it into the Province secondary bank.
                return None;
            }
            if self.region(a) != self.region(b) {
                return Some(BorderClass::SeaRegion);
            }
            return Some(BorderClass::Sea);
        }
        if self.impassable(a, b) {
            return Some(BorderClass::Impassable);
        }
        None
    }

    /// The classifier [`borders::build`] takes: the static class, or the land placeholder.
    ///
    /// A coast stays `None`, so the trace retains it in its hidden mesh rather
    /// than an ordinary land bucket. Only custom highlight admission remaps it.
    pub fn trace_class(&self, a: u32, b: u32) -> Option<BorderClass> {
        if a == b || self.hidden(a) || self.hidden(b) || self.water(a) != self.water(b) {
            return None;
        }
        Some(self.static_class(a, b).unwrap_or(BorderClass::Province))
    }
}

/// Reconstruct province-definition bit 2 independently of fog. Start with raster type-0 neighbours
/// and set it only for land lacking a non-sea land neighbour. Authored adjacency overrides arrive
/// afterward without recomputing it.
fn hidden_provinces(map: &MapData, count: usize) -> Vec<bool> {
    let mut hidden = vec![false; count];
    for (&id, definition) in &map.definitions {
        hidden[id as usize] = id != 0 && definition.kind == ProvinceKind::Land;
    }
    let land = |id| {
        id != 0
            && map
                .definitions
                .get(&id)
                .is_some_and(|definition| definition.kind == ProvinceKind::Land)
    };
    let mut connect = |a: u32, b: u32| {
        if a != b && land(a) && land(b) {
            hidden[a as usize] = false;
            hidden[b as usize] = false;
        }
    };
    let width = map.width as usize;
    if width != 0 {
        for (y, row) in map.province_ids.chunks_exact(width).enumerate() {
            for (x, &a) in row.iter().enumerate() {
                let right = row[(x + 1) % width];
                connect(a, right);
                if y + 1 < map.height as usize {
                    let below = map.province_ids[(y + 1) * width + x];
                    let diagonal = map.province_ids[(y + 1) * width + (x + 1) % width];
                    connect(a, below);
                    // The engine diagnoses an invalid four-province X crossing, but still
                    // inserts this diagonal adjacency before setting the bit.
                    if [a, right, below, diagonal].iter().all(|&id| id != 0)
                        && a != right
                        && a != below
                        && a != diagonal
                        && right != below
                        && right != diagonal
                        && below != diagonal
                    {
                        // Native y increases northward; our bitmap rows increase southward.
                        connect(right, below);
                    }
                }
            }
        }
    }
    hidden
}

/// The class of one land edge, independent of the political fill's mode.
///
/// Classify canonical state owners through CountryLinks, reserving zero for a null owner. Different
/// owners use country material; equal owners with different state ids use state material; other land
/// pairs use province. Faction, subject and war membership do not change country identity.
///
/// Captured at hour 60828490: all 6,539 draws / 6,524 province pairs agree with the complete classifier;
/// controller keying disagrees on 155 pairs. See
pub fn land_class(regions: &Regions, a: u32, b: u32) -> BorderClass {
    let key = |province: u32| {
        regions
            .owner_of
            .get(province as usize)
            .copied()
            .unwrap_or_default()
    };
    let state = |province: u32| {
        regions
            .state_of
            .get(province as usize)
            .copied()
            .unwrap_or_default()
    };
    if key(a) != key(b) {
        BorderClass::Country
    } else if state(a) != state(b) {
        BorderClass::State
    } else {
        BorderClass::Province
    }
}

fn pair(a: u32, b: u32) -> (u32, u32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Use the neighbour record's stored end when its x is nonnegative, otherwise `(0, 0)`. A zero end
/// is derived from province pixels.
fn authored_end(end: (i32, i32)) -> Option<(i32, i32)> {
    (end.0 >= 0 && end != (0, 0)).then_some(end)
}

/// One neighbour record: neighbour id, through value and stored end.
#[derive(Clone, Copy)]
struct Record {
    neighbour: u32,
    through: i32,
    end: (i32, i32),
}

/// Ordered province adjacency lists, seeded with raster neighbours before applying authored rows.
///
/// Rewrite existing pairs in place on both sides, using stop for From and start for To. A new pair
/// appends start for From and stop for To, only on sides that do not already list the neighbour.
fn neighbour_records(adjacencies: &[Adjacency], raster: &[Vec<u32>]) -> BTreeMap<u32, Vec<Record>> {
    let mut lists: BTreeMap<u32, Vec<Record>> = BTreeMap::new();
    for row in adjacencies {
        for province in [row.from, row.to] {
            lists.entry(province).or_insert_with(|| {
                raster
                    .get(province as usize)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .map(|&neighbour| Record {
                        neighbour,
                        through: 0,
                        end: (0, 0),
                    })
                    .collect()
            });
        }
        let (from, to) = (row.from, row.to);
        let listed = lists[&from].iter().any(|record| record.neighbour == to);
        let mut write = |owner: u32, neighbour: u32, end: (i32, i32)| {
            let list = lists.get_mut(&owner).expect("seeded above");
            let record = Record {
                neighbour,
                through: row.through,
                end,
            };
            if listed {
                for existing in list.iter_mut().filter(|r| r.neighbour == neighbour) {
                    *existing = record;
                }
            } else if !list.iter().any(|r| r.neighbour == neighbour) {
                list.push(record);
            }
        };
        if listed {
            write(from, to, row.stop);
            write(to, from, row.start);
        } else {
            write(from, to, row.start);
            write(to, from, row.stop);
        }
    }
    lists
}

/// Build strait ribbons by province id and neighbour-list order. Keep a record when its neighbour
/// id is at or below the province's own, its through value is nonzero, and the province itself is
/// land. Crossing kind is not tested. [`neighbour_records`] retains the CSV effects: ordinary
/// bitmap neighbours have through zero, `impassable` rows write -1, and empty columns read zero.
///
/// `A` is the kept record's end, `B` the end of the neighbour's first record back; an end the
/// builder does not take ([`authored_end`]) is the nearest pixel of that province to the other
/// ([`nearest_pixels`]).
///
/// Each quad extends two world units on either side of the line at height 9.5. Its cross-ribbon
/// texture coordinate runs from zero to one, and its along-ribbon span rounds the length divided
/// by four upward. The index order preserves the two triangles' winding.
///
/// The endpoints are world `(x, z)`, `z` counting up from the bottom of the map.
pub fn strait_quads(
    adjacencies: &[Adjacency],
    map: &MapData,
    land: &dyn Fn(u32) -> bool,
) -> io::Result<Vec<StraitVertex>> {
    let raster = installed_content::movement::raster_neighbours(
        map.width as usize,
        map.height as usize,
        &map.province_ids,
    )?;
    let lists = neighbour_records(adjacencies, &raster);
    let kept: Vec<(u32, Record)> = lists
        .iter()
        .filter(|&(&owner, _)| land(owner))
        .flat_map(|(&owner, list)| list.iter().map(move |&record| (owner, record)))
        .filter(|&(owner, record)| record.neighbour <= owner && record.through != 0)
        .collect();
    let ends = |owner: u32, record: Record| {
        let back = lists
            .get(&record.neighbour)
            .and_then(|list| list.iter().find(|r| r.neighbour == owner))
            .and_then(|r| authored_end(r.end));
        (authored_end(record.end), back)
    };
    let needed = kept
        .iter()
        .filter(|&&(owner, record)| {
            let (a, b) = ends(owner, record);
            a.is_none() || b.is_none()
        })
        .flat_map(|&(owner, record)| [owner, record.neighbour])
        .collect::<HashSet<_>>();
    let spans = boundary_spans(map, &needed)?;
    let mut vertices = Vec::new();
    for (high, record) in kept {
        let low = record.neighbour;
        let (a, b) = match ends(high, record) {
            (Some(a), Some(b)) => (a, b),
            (a, b) => {
                let Some((near_a, near_b)) = nearest_pixels(&spans, high, low) else {
                    continue;
                };
                (a.unwrap_or(near_a), b.unwrap_or(near_b))
            }
        };
        let (ax, az) = (a.0 as f32, a.1 as f32);
        let (bx, bz) = (b.0 as f32, b.1 as f32);
        let (dx, dz) = (bx - ax, bz - az);
        let length = (dx * dx + dz * dz).sqrt();
        if length <= 0.0 {
            continue;
        }
        let (nx, nz) = (dz / length, -dx / length);
        let half = STRAIT_WIDTH * 0.5;
        let tiles = (length / STRAIT_WIDTH).ceil();
        let corner = |x: f32, z: f32, side: f32, u: f32, v: f32| {
            let (cx, cz) = (x + nx * half * side, z + nz * half * side);
            StraitVertex {
                position: [cx, WATER_HEIGHT, cz],
                uv: [u, v],
            }
        };
        let v0 = corner(ax, az, 1.0, 0.0, 0.0);
        let v1 = corner(ax, az, -1.0, 1.0, 0.0);
        let v2 = corner(bx, bz, 1.0, 0.0, tiles);
        let v3 = corner(bx, bz, -1.0, 1.0, tiles);
        vertices.extend_from_slice(&[v0, v1, v3, v0, v3, v2]);
    }
    Ok(vertices)
}

/// One run of a province's pixels on one map row, the `PIXEL_SPANS` record, holding only the
/// pixels [`nearest_pixels`] can pick.
struct Span {
    z: i32,
    xs: Vec<i32>,
}

/// Each needed province's pixel runs in the order the definition stores them: rows from the
/// bottom of the map up, runs left to right.
///
/// A pixel whose four neighbours are all its own province is dropped. It can never be a nearest
/// pixel — the neighbour one step toward any pixel of the other province is strictly closer — so
/// dropping it leaves the native scan's first strict minimum where it was.
fn boundary_spans(map: &MapData, needed: &HashSet<u32>) -> io::Result<BTreeMap<u32, Vec<Span>>> {
    let (width, height) = (map.width as usize, map.height as usize);
    if map.province_ids.len() != width * height {
        return Err(invalid("the province image does not match its dimensions"));
    }
    let mut spans: BTreeMap<u32, Vec<Span>> = BTreeMap::new();
    if needed.is_empty() {
        return Ok(spans);
    }
    // `province_ids` is stored north first; world z counts up from the bottom row.
    let at = |x: i64, z: i64| -> Option<u32> {
        (x >= 0 && z >= 0 && (x as usize) < width && (z as usize) < height)
            .then(|| map.province_ids[(height - 1 - z as usize) * width + x as usize])
    };
    for z in 0..height as i64 {
        let mut x = 0i64;
        while (x as usize) < width {
            let id = at(x, z).unwrap_or(0);
            let mut end = x + 1;
            while (end as usize) < width && at(end, z) == Some(id) {
                end += 1;
            }
            if needed.contains(&id) {
                let xs = (x..end)
                    .filter(|&px| {
                        ![(px - 1, z), (px + 1, z), (px, z - 1), (px, z + 1)]
                            .iter()
                            .all(|&(nx, nz)| at(nx, nz) == Some(id))
                    })
                    .map(|px| px as i32)
                    .collect();
                spans.entry(id).or_default().push(Span { z: z as i32, xs });
            }
            x = end;
        }
    }
    Ok(spans)
}

/// The nearest pixel pair of two provinces, as `build_strait_ribbons` finds it: the first
/// province's spans outermost, then the second's, then the first span's pixels, then the second's,
/// keeping the first strict minimum of the squared distance.
fn nearest_pixels(
    spans: &BTreeMap<u32, Vec<Span>>,
    first: u32,
    second: u32,
) -> Option<((i32, i32), (i32, i32))> {
    let (a, b) = (spans.get(&first)?, spans.get(&second)?);
    let mut best = (i64::MAX, None);
    for sa in a {
        for sb in b {
            let dz = i64::from(sa.z - sb.z);
            for &xa in &sa.xs {
                for &xb in &sb.xs {
                    let dx = i64::from(xa - xb);
                    let d = dx * dx + dz * dz;
                    if d < best.0 {
                        best = (d, Some(((xa, sa.z), (xb, sb.z))));
                    }
                }
            }
        }
    }
    best.1
}

/// The vertex bytes of one chunk, in `borders::Vertex`'s 16-byte layout.
///
/// `borders::Vertex` is `#[repr(C)]` but not `Pod`, so the bytes are written out field by field
/// rather than reinterpreted.
fn chunk_bytes(chunk: &[borders::Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(chunk.len() * BORDER_VERTEX_STRIDE as usize);
    for vertex in chunk {
        for value in vertex.position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.uv {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// The quad pattern for one run of `points` points.
///
/// The engine shares one index buffer across every draw and offsets it by the draw's base vertex;
/// wgpu can do that too, but folding the base vertex in here lets every strip of a class that lands
/// in one vertex chunk be drawn with a single call instead of one call per province pair.
fn run_indices(base_vertex: u32, points: u32, out: &mut Vec<u32>) {
    for segment in 0..points.saturating_sub(1) {
        let base = base_vertex + segment * 2;
        out.extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);
    }
}

/// One hour's border index runs, bucketed without a device so a builder thread
/// can do it ahead of the frame that shows the hour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassPlan {
    /// Runs per drawn class, in [`BorderClass::ALL`] order.
    runs: Vec<Vec<PlannedRun>>,
}

#[derive(Clone, Debug, PartialEq)]
struct PlannedRun {
    source: usize,
    chunk: usize,
    /// Shared with a neighbouring hour's plan when equal ([`ClassPlan::share`]),
    /// which is how [`BorderPass::set_plan`] knows to keep the buffer it has.
    indices: Arc<Vec<u32>>,
}

impl ClassPlan {
    /// Buckets every edge for one hour's political regions and demilitarised zone.
    ///
    /// Every land edge lives in the [`BorderClass::Province`] vertex chunks, so the three land
    /// classes index those same vertices; the other three keep the class the content settled at
    /// trace time and only drop the edges the zone splits.
    ///
    /// An edge the zone splits is left out of every bucket here. The engine puts each chain in
    /// exactly one bucket, `base_material + 6 * secondary_bank`, so a demilitarised edge is drawn
    /// by [`super::demilitarized::DemilitarizedPass`] *instead of* here, not on top of it; leaving
    /// it in both would blend the strip twice and, with depth writes on, let the second draw fail
    /// its own depth test.
    pub fn new(meshes: &BorderMeshes, regions: &Regions, zone: &Zone) -> Self {
        Self::matching(meshes, regions, |a, b| !zone.splits(a, b))
    }

    /// Native ordinary buckets contain only edges not claimed by a secondary highlight.
    pub fn matching(
        meshes: &BorderMeshes,
        regions: &Regions,
        keep: impl Fn(u32, u32) -> bool,
    ) -> Self {
        let kept = |draw: &BorderDraw| keep(draw.province_a, draw.province_b);
        let draws = |bucket: usize| {
            meshes
                .meshes
                .get(bucket)
                .map(|mesh| mesh.draws.as_slice())
                .unwrap_or_default()
                .iter()
                .copied()
        };
        let runs = BorderClass::ALL
            .iter()
            .map(|&class| match class {
                BorderClass::Province | BorderClass::State | BorderClass::Country => Self::runs(
                    meshes,
                    BorderClass::Province,
                    draws(BorderClass::Province.bucket()).filter(|draw| {
                        kept(draw) && land_class(regions, draw.province_a, draw.province_b) == class
                    }),
                ),
                // With no zone this is every edge, which is also what `BorderPass::upload`
                // gives these classes before any hour is set.
                _ => Self::runs(meshes, class, draws(class.bucket()).filter(kept)),
            })
            .collect();
        Self { runs }
    }

    /// Groups a class's draws by their vertex chunk, one index run per chunk.
    fn runs(
        meshes: &BorderMeshes,
        source: BorderClass,
        draws: impl Iterator<Item = BorderDraw>,
    ) -> Vec<PlannedRun> {
        let chunks = meshes
            .meshes
            .get(source.bucket())
            .map_or(0, |mesh| mesh.chunks.len());
        let mut per_chunk: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
        for draw in draws {
            if (draw.chunk as usize) >= chunks {
                continue;
            }
            let indices = per_chunk.entry(draw.chunk as usize).or_default();
            run_indices(draw.base_vertex, draw.vertex_count / 2, indices);
        }
        per_chunk
            .into_iter()
            .filter(|(_, indices)| !indices.is_empty())
            .map(|(chunk, indices)| PlannedRun {
                source: source.bucket(),
                chunk,
                indices: Arc::new(indices),
            })
            .collect()
    }

    /// Takes `other`'s copy of every run the two plans hold alike, so a pass
    /// that drew `other` keeps those runs' buffers when it is given this plan.
    pub fn share(&mut self, other: &ClassPlan) {
        for (runs, others) in self.runs.iter_mut().zip(&other.runs) {
            for run in runs {
                if let Some(same) = others.iter().find(|other| {
                    (other.source, other.chunk) == (run.source, run.chunk)
                        && other.indices == run.indices
                }) {
                    run.indices = Arc::clone(&same.indices);
                }
            }
        }
    }
}

/// One index buffer over one vertex chunk of one source bucket.
struct Run {
    source: usize,
    chunk: usize,
    indices: wgpu::Buffer,
    count: u32,
    /// The planned indices the buffer holds, when it came from a [`ClassPlan`].
    planned: Option<Arc<Vec<u32>>>,
}

/// What one frame of the pass needs beyond its own geometry.
#[derive(Clone, Copy)]
pub struct BorderFrame {
    /// `vCamPos.y`, which decides the province and state fades.
    pub camera_height: f32,
    /// The content's province and state fade heights; see [`borders::class_alpha`].
    pub fades: [[f32; 2]; 2],
    /// Whether the map mode draws province borders; the resources, infrastructure and manpower
    /// modes give them no opacity.
    pub province_borders: bool,
    /// Native land batch precedes rivers; sea batch follows trees and water.
    pub stage: BorderStage,
    /// A hidden stage submits nothing but leaves the passes after it the same inputs.
    pub visible: bool,
}

/// Each world-wrap view draws terrain, buckets 0..4, rivers, map objects, trees, water, buckets
/// 4..12, then straits with their own program and state.
#[derive(Clone, Copy)]
pub enum BorderStage {
    Land,
    Sea,
}

const LAND_ORDER: &[BorderClass] = &[
    BorderClass::Province,
    BorderClass::State,
    BorderClass::Country,
    BorderClass::Impassable,
];
const SEA_ORDER: &[BorderClass] = &[BorderClass::Sea, BorderClass::SeaRegion];

/// The GPU border and strait pass.
pub struct BorderPass {
    compiled: Result<super::border_shader::Border, String>,
    textures: Vec<wgpu::TextureView>,
    pub compiled_straits: super::strait_shader::Straits,
    /// Vertex chunks per source bucket, in [`BorderClass::ALL`] order.
    vertices: Vec<Vec<wgpu::Buffer>>,
    /// Index runs per drawn class, in [`BorderClass::ALL`] order.
    runs: Vec<Vec<Run>>,
    /// Every traced edge, per source bucket in [`BorderClass::ALL`] order, kept for re-bucketing
    /// and for leaving the demilitarised-zone edges out.
    draws: Vec<Vec<BorderDraw>>,
}

impl BorderPass {
    /// Builds the content pipelines and uploads their textures.
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: &BorderAssets,
    ) -> io::Result<Self> {
        let mut textures = Vec::new();
        for (class, texture) in BorderClass::ALL.iter().zip(&assets.classes) {
            textures.push(upload(
                device,
                queue,
                &format!("border_{}", class.texture_name()),
                texture,
                ColorSpace::Srgb,
            )?);
        }
        let strait_texture = upload(
            device,
            queue,
            "strait",
            &assets.strait,
            // Strait textures use BC3_UNORM and retain linear authored values. Border strips use an
            // sRGB view.
            ColorSpace::Linear,
        )?;
        let compiled_straits = super::strait_shader::Straits::new(
            device,
            &assets.strait_shader,
            samples,
            format,
            DEPTH_FORMAT,
            strait_texture.view,
        );
        Ok(Self {
            compiled: super::border_shader::Border::new(
                device,
                &assets.border_shader,
                samples,
                format,
                DEPTH_FORMAT,
            )
            .map_err(|error| error.to_string()),
            textures: textures
                .iter()
                .map(|texture| texture.view.clone())
                .collect(),
            compiled_straits,
            vertices: Vec::new(),
            runs: BorderClass::ALL.iter().map(|_| Vec::new()).collect(),
            draws: Vec::new(),
        })
    }

    /// Uploads the traced meshes and the strait ribbons. Called once per document.
    ///
    /// The three land classes have no indices until [`BorderPass::set_classes`] runs; the
    /// classes the content alone decides get theirs here.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        straits: &[StraitVertex],
    ) -> io::Result<()> {
        self.compiled_straits.upload(device, straits)?;
        self.vertices = meshes
            .meshes
            .iter()
            .map(|mesh| {
                mesh.chunks
                    .iter()
                    .enumerate()
                    .map(|(index, chunk)| {
                        buffer(
                            device,
                            &format!("border {} chunk {index}", mesh.class.texture_name()),
                            wgpu::BufferUsages::VERTEX,
                            &chunk_bytes(chunk),
                        )
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .collect::<io::Result<Vec<_>>>()?;
        self.draws = meshes
            .meshes
            .iter()
            .map(|mesh| mesh.draws.clone())
            .collect();
        // The three classes the content alone settles get their indices here, so a scene that has
        // not been given an hour yet still draws them; `set_classes` rebuilds them per hour once
        // the demilitarised zone can take edges out.
        for class in [
            BorderClass::Impassable,
            BorderClass::Sea,
            BorderClass::SeaRegion,
        ] {
            self.runs[class.bucket()] =
                self.build_runs(device, class, meshes.mesh(class).draws.iter().copied())?;
        }
        Ok(())
    }

    /// Re-buckets every edge for one hour's political regions and demilitarised zone.
    ///
    /// Every land edge lives in the [`BorderClass::Province`] vertex chunks, so the three land
    /// classes rebuild one index buffer each over those same vertices; the other three keep the
    /// class the content settled at trace time and only have to drop the highlighted edges.
    ///
    /// An edge the zone splits is left out of every bucket here. The engine puts each chain in
    /// exactly one bucket, `base_material + 6 * secondary_bank`, so a demilitarised edge is drawn
    /// by [`super::demilitarized::DemilitarizedPass`] *instead of* here, not on top of it; leaving
    /// it in both would blend the strip twice and, with depth writes on, let the second draw fail
    /// its own depth test.
    pub fn set_classes(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
    ) -> io::Result<()> {
        self.set_plan(device, &ClassPlan::new(meshes, regions, zone))
    }

    /// Uploads index runs [`ClassPlan::new`] bucketed, one buffer per run.
    ///
    /// A run whose indices are the very ones a buffer already holds (the plan
    /// shared them with the plan drawn before, [`ClassPlan::share`]) keeps
    /// that buffer, so an hour that moved a few edges uploads only the runs
    /// they were in.
    pub fn set_plan(&mut self, device: &wgpu::Device, plan: &ClassPlan) -> io::Result<()> {
        for (bucket, runs) in plan.runs.iter().enumerate() {
            let chunks = |source: usize| self.vertices.get(source).map_or(0, Vec::len);
            let mut held = self
                .runs
                .get_mut(bucket)
                .map(std::mem::take)
                .unwrap_or_default();
            let built = runs
                .iter()
                .filter(|run| run.chunk < chunks(run.source))
                .map(|run| {
                    let kept = held.iter().position(|old| {
                        (old.source, old.chunk) == (run.source, run.chunk)
                            && old
                                .planned
                                .as_ref()
                                .is_some_and(|planned| Arc::ptr_eq(planned, &run.indices))
                    });
                    if let Some(kept) = kept {
                        return Ok(held.swap_remove(kept));
                    }
                    Ok(Run {
                        source: run.source,
                        chunk: run.chunk,
                        count: run.indices.len() as u32,
                        indices: buffer(
                            device,
                            "border indices",
                            wgpu::BufferUsages::INDEX,
                            bytemuck::cast_slice(&run.indices),
                        )?,
                        planned: Some(Arc::clone(&run.indices)),
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            if let Some(slot) = self.runs.get_mut(bucket) {
                *slot = built;
            }
        }
        Ok(())
    }

    /// Groups a class's draws by their vertex chunk and builds one index buffer per chunk.
    fn build_runs(
        &self,
        device: &wgpu::Device,
        source: BorderClass,
        draws: impl Iterator<Item = BorderDraw>,
    ) -> io::Result<Vec<Run>> {
        let chunks = self.vertices.get(source.bucket()).map_or(0, Vec::len);
        let mut per_chunk: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
        for draw in draws {
            if (draw.chunk as usize) >= chunks {
                continue;
            }
            let indices = per_chunk.entry(draw.chunk as usize).or_default();
            run_indices(draw.base_vertex, draw.vertex_count / 2, indices);
        }
        per_chunk
            .into_iter()
            .filter(|(_, indices)| !indices.is_empty())
            .map(|(chunk, indices)| {
                Ok(Run {
                    source: source.bucket(),
                    chunk,
                    count: indices.len() as u32,
                    indices: buffer(
                        device,
                        "border indices",
                        wgpu::BufferUsages::INDEX,
                        bytemuck::cast_slice(&indices),
                    )?,
                    planned: None,
                })
            })
            .collect()
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: super::border_shader::Target<'_>,
        inputs: super::border_shader::Inputs<'_>,
        frame: BorderFrame,
    ) -> io::Result<super::shader_constants::WorldPixelConstants> {
        let classes = match frame.stage {
            BorderStage::Land => LAND_ORDER,
            BorderStage::Sea => SEA_ORDER,
        };
        let batches = classes
            .iter()
            .map(|&class| {
                let alpha = borders::class_alpha(
                    class,
                    frame.camera_height,
                    frame.fades,
                    frame.province_borders,
                );
                super::border_shader::Batch {
                    texture: &self.textures[class.bucket()],
                    selection: [0.; 4],
                    alpha,
                    runs: self.runs[class.bucket()]
                        .iter()
                        .filter(|_| alpha > 0.)
                        .filter_map(|run| {
                            self.vertices
                                .get(run.source)
                                .and_then(|chunks| chunks.get(run.chunk))
                                .map(|vertices| super::border_shader::Run {
                                    vertices,
                                    indices: &run.indices,
                                    count: run.count,
                                })
                        })
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        viewer_foundation::map::gpu_contract::compiled(&self.compiled)?.draw(
            arena,
            encoder,
            target,
            inputs,
            &batches,
            frame.visible,
        )
    }
}

/// The world x translations of the wrap copies the visible rectangle reaches.
///
/// The border meshes are cut at the x seam and the terrain grid is never clipped in x — it wraps its
/// own texture lookups instead (`terrain::grid`) — so a view that runs
/// past either edge of the map needs the geometry drawn again a map width away. The engine always
/// draws three copies; this submits only the ones that can be seen.
pub fn wrap_offsets(visible: Option<[f32; 4]>, map_width: f32) -> Vec<f32> {
    let mut offsets = vec![0.0];
    let Some(rect) = visible else {
        return offsets;
    };
    if rect[0] < 0.0 {
        offsets.push(-map_width);
    }
    if rect[2] > map_width {
        offsets.push(map_width);
    }
    offsets
}

/// The world-wrap copies a free eye's reach overlaps, as [`wrap_offsets`] orders them: its own copy
/// first, then nearer copies before farther ones, west before east. The eye can stand on any copy,
/// so the offsets are whole map widths from the first copy rather than from the eye's.
pub fn free_wrap_offsets(reach: Option<[f32; 4]>, map_width: f32) -> Vec<f32> {
    let Some([x0, _, x1, _]) = reach else {
        return vec![0.0];
    };
    if !(map_width > 0.0 && x0.is_finite() && x1.is_finite()) {
        return vec![0.0];
    }
    let first = (x0 / map_width).floor() as i32;
    let last = ((x1 / map_width).ceil() as i32 - 1).max(first);
    let reach = first.unsigned_abs().max(last.unsigned_abs()) as i32;
    std::iter::once(0)
        .chain((1..=reach).flat_map(|distance| [-distance, distance]))
        .filter(|copy| *copy == 0 || (first..=last).contains(copy))
        .map(|copy| copy as f32 * map_width)
        .collect()
}

#[cfg(test)]
mod tests;
