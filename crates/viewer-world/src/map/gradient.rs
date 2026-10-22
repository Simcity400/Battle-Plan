//! Gradient border fields for political fill.
//!
//! Two RGBA8 channel textures contain stacked strategic-region and country
//! layers, each followed by a zero guard row. Shader constants use the layer
//! height. Channel 1 holds colour and the distance field; channel 2 holds the
//! layer's visibility and naval attributes. Strategic regions cover sea
//! provinces, while country colours follow recorded controllers. There is no
//! state layer. Naval artwork and all colour/thickness inputs resolve through
//! the recorded content stack.
//!
//! A pixel outside a layer's regions holds channel-1 alpha 255 and zero
//! channel 2. Country thickness interpolates from controlled pixel area; its
//! configured bounds are areas and are not squared. Boundary searches inspect
//! a fixed number of grid cells, retaining scan order and early exits.
//!
//! This module holds shared field arithmetic and a test-only CPU oracle that
//! returns byte buffers. Production uses [`super::political_gpu`]; frame
//! constants live in [`viewer_foundation::map::frame`].

#[cfg(test)]
use std::io;
#[cfg(test)]
use std::{collections::HashMap, thread};

#[cfg(test)]
use viewer_foundation::errors::invalid;

/// The field's slope numerator.
pub const SLOPE_NUMERATOR: f32 = 255.0;
#[cfg(test)]
use viewer_foundation::map::gradient_math::DIVISOR;
use viewer_foundation::map::gradient_math::FieldDefines;
#[cfg(test)]
use viewer_foundation::map::province_map::ProvinceMap;

/// The constant the field formula subtracts before clamping, so the field peaks at 247 rather than
/// 255, keeping the land pass below its outline cutoff of 0.973.
pub const ALPHA_BIAS: i32 = 8;
/// Side of the uniform acceleration grid in map pixels.
pub const GRID_CELL: u32 = 16;
/// Search-radius multiplier.
pub const RADIUS_MULTIPLIER: f32 = 3.0;
/// Region opacity when no map mode overrides it; it lands in `GradientBorderChannel2.g`.
pub const DEFAULT_REGION_OPACITY: u8 = 255;
/// The region key that means "this province is not in this layer" (sea in the country layer, for
/// instance). Such pixels get `NO_REGION_ALPHA` in channel 1 and zero in channel 2.
pub const NO_REGION: u32 = 0;
/// Channel 1 alpha for pixels outside a layer's regions, including sea in layer 2 and land in
/// layer 1.
///
/// Maximum alpha disables both region fill and outline for these pixels. Alpha zero denotes a
/// region interior and retains the camera-dependent fill.
#[cfg(test)]
pub const NO_REGION_ALPHA: u8 = 255;

/// Which thickness rule a layer follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerKind {
    /// Layer 2 in political mode: one region per controller, thickness from the region's area.
    Country,
    /// One thickness for every region; layer 1 uses `THICKNESS_STRATEGIC_REGIONS`.
    Fixed(f32),
}

impl LayerKind {
    /// The field thickness `T` in map pixels for a region of `area_pixels`.
    pub fn thickness(self, area_pixels: u64, field: &FieldDefines) -> f32 {
        match self {
            LayerKind::Country => country_thickness(area_pixels, field),
            LayerKind::Fixed(thickness) => thickness,
        }
    }
}

/// A map mode's presentation of one region beyond its colour: the colour's alpha byte (channel
/// two's green), whether it pulses (channel two's blue), the fill factor that widens or narrows the
/// field's colour band, and a fixed thickness replacing the layer's rule.
///
/// The field byte of a region pixel, or its constant gradient, below 239 becomes
/// `clamp(a - trunc((255 - a) * (1 - fill)), 0, 255)`; at 1.0 it is unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionStyle {
    pub alpha: u8,
    pub pulse: bool,
    pub fill: f32,
    pub thickness: Option<f32>,
}

impl RegionStyle {
    /// The political default: full alpha, no pulse, no fill change, the layer's thickness.
    pub const fn plain(alpha: u8) -> Self {
        Self {
            alpha,
            pulse: false,
            fill: 1.0,
            thickness: None,
        }
    }
}

/// The field byte after the region's fill factor.
#[cfg(test)]
pub fn filled_alpha(alpha: u8, fill: f32) -> u8 {
    if alpha >= 0xef {
        return alpha;
    }
    let a = i32::from(alpha);
    let band = ((255 - a) as f32 * (1.0 - fill)) as i32;
    (a - band).clamp(0, 255) as u8
}

/// Native per-region FX bytes and optional constant gradient.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    pub control: u8,
    pub contested: u8,
    pub gradient: Option<u8>,
}

impl Attributes {
    pub(crate) fn packed(self) -> u32 {
        u32::from(self.control)
            | (u32::from(self.contested) << 8)
            | self
                .gradient
                .map_or(0, |value| (1 << 24) | (u32::from(value) << 16))
    }
}

/// One gradient layer's geometry, colour and dynamic attributes.
pub struct Layer<'a> {
    /// The thickness rule, which is also what the layer means.
    pub kind: LayerKind,
    /// The content's field operands.
    pub field: &'a FieldDefines,
    /// Region key per province id, indexed by the id itself. [`NO_REGION`] leaves the province out
    /// of this layer. Must be long enough for every id in the province map.
    pub region_of: &'a [u32],
    /// The region's map colour.
    /// Called once per distinct region key, on the calling thread.
    pub region_color: &'a dyn Fn(u32) -> [u8; 3],
    /// Optional explicit gradient and naval control/contested attributes.
    pub region_attributes: Option<&'a dyn Fn(u32) -> Attributes>,
    /// Channel-two green opacity per layer, defaulting to DEFAULT_REGION_OPACITY unless the content-
    /// selected map mode overrides it.
    pub region_opacity: u8,
    /// A map mode's per-region style; `None` uses `region_opacity` and the layer's thickness.
    pub region_style: Option<&'a dyn Fn(u32) -> RegionStyle>,
    /// Channel 1 colour outside a layer's regions: black for layer 2's sea and the configured navy
    /// colour for layer 1's land. Naval control/contested attributes remain independent of this colour.
    ///
    pub no_region_color: [u8; 3],
}

impl<'a> Layer<'a> {
    /// A layer at the default opacity, with no colour where it has no region.
    pub fn new(
        kind: LayerKind,
        region_of: &'a [u32],
        region_color: &'a dyn Fn(u32) -> [u8; 3],
        field: &'a FieldDefines,
    ) -> Self {
        Self {
            kind,
            field,
            region_of,
            region_color,
            region_attributes: None,
            region_opacity: DEFAULT_REGION_OPACITY,
            region_style: None,
            no_region_color: [0, 0, 0],
        }
    }

    /// The style of one region key.
    pub fn style(&self, key: u32) -> RegionStyle {
        self.region_style
            .map_or(RegionStyle::plain(self.region_opacity), |style| style(key))
    }

    /// The field thickness of a region of `area_pixels`.
    pub fn thickness(&self, key: u32, area_pixels: u64) -> f32 {
        self.region_style
            .and_then(|style| style(key).thickness)
            .unwrap_or_else(|| self.kind.thickness(area_pixels, self.field))
    }
}

/// The two gradient border textures, ready for a straight RGBA8 upload.
///
/// `width` is the map width over [`DIVISOR`] and `height` is `GB_TextureHeight`, two guard rows
/// included. Both buffers are row-major, `width * height` texels, in `provinces.bmp`'s own top-down
/// order: row 0 is the north edge.
///
/// Gradient textures share the flipped map coordinate used by terrain and water: north maps to the
/// first row and south to the last. Preserve this order when packing each layer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
pub struct GradientTextures {
    pub width: u32,
    pub height: u32,
    /// The field texture: `rgb` = the region's map colour, `a` = the distance field.
    pub channel1: Vec<[u8; 4]>,
    /// The FX texture: `g` = region opacity, `b` = pulse flag, `r`/`a` = naval control/contested.
    pub channel2: Vec<[u8; 4]>,
}

#[cfg(test)]
impl GradientTextures {
    /// Rows per layer, `H` in the module header: the map height over [`DIVISOR`].
    pub fn layer_rows(&self) -> u32 {
        self.height / 2 - 1
    }

    /// The first row of a layer, `layer` being 0 or 1.
    pub fn layer_origin(&self, layer: u32) -> u32 {
        layer * (self.layer_rows() + 1)
    }

    /// A texel of one layer, or `None` outside it. `y` counts from the layer's first row, which is
    /// the north edge of `provinces.bmp`.
    pub fn channel1_texel(&self, layer: u32, x: u32, y: u32) -> Option<[u8; 4]> {
        self.texel(&self.channel1, layer, x, y)
    }

    /// A texel of one layer of [`GradientTextures::channel2`], indexed like
    /// [`GradientTextures::channel1_texel`].
    pub fn channel2_texel(&self, layer: u32, x: u32, y: u32) -> Option<[u8; 4]> {
        self.texel(&self.channel2, layer, x, y)
    }

    fn texel(&self, plane: &[[u8; 4]], layer: u32, x: u32, y: u32) -> Option<[u8; 4]> {
        if layer > 1 || x >= self.width || y >= self.layer_rows() {
            return None;
        }
        let row = self.layer_origin(layer) + y;
        plane
            .get((row as usize) * (self.width as usize) + (x as usize))
            .copied()
    }
}

/// The country layer's field thickness `T` in map pixels for a country covering `area_pixels`.
///
/// Thickness interpolates from region area using the configured bounds directly; area bounds are not
/// squared.
pub fn country_thickness(area_pixels: u64, field: &FieldDefines) -> f32 {
    let [low, high] = field.country_area.map(f64::from);
    let [thin, thick] = field.country_thickness;
    let fraction = (((area_pixels as f64) - low) / (high - low)).clamp(0.0, 1.0) as f32;
    thin + fraction * (thick - thin)
}

/// How far from a boundary the field is carried, in map pixels.
///
/// The optimization range is a minimum reach, so thick fields can extend beyond it. The boundary
/// search's cell radius may end the field sooner; see `BoundaryGrid::nearest`.
pub fn field_reach(thickness: f32, field: &FieldDefines) -> f32 {
    field.optimization_range.max(field.center + thickness)
}

/// The field byte for a pixel `distance` map pixels from the nearest region boundary, in a region
/// of thickness `thickness`:
///
/// Convert distance to the capped byte field with truncation before packing.
///
/// It peaks at 247 *on* the boundary and falls to 0 about `C + 0.96 * T` pixels inside the region,
/// keeping it below the outline cutoff. A separate thick-border band supplies the dark line
/// between countries.
///
pub fn field_alpha(distance: f32, thickness: f32, centre: f32) -> u8 {
    if !distance.is_finite() || !thickness.is_finite() || thickness <= 0.0 {
        return 0;
    }
    let value = if centre < distance {
        (255 - ((distance - centre) * (1.0 / thickness) * SLOPE_NUMERATOR) as i32) - centre as i32
    } else {
        // The byte mask preserves packing after the field value is capped.
        255 - ((distance as i32) & 0xff)
    };
    (value - ALPHA_BIAS).clamp(0, 255) as u8
}

/// One layer's rows of both channel textures, ready to be stacked by [`stack`].
///
/// Each layer owns its geometry and attributes; strategic keys stay fixed while naval FX changes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
pub struct LayerPlane {
    /// The map width over [`DIVISOR`].
    pub width: u32,
    /// The map height over [`DIVISOR`], without the guard row that follows it in the texture.
    pub rows: u32,
    /// `rgb` = the region's map colour, `a` = the distance field.
    pub channel1: Vec<[u8; 4]>,
    /// `g` = region opacity, `r`/`a` = naval control/contested.
    pub channel2: Vec<[u8; 4]>,
}

/// Builds one layer's plane.
///
/// The map wraps horizontally - the pixels at `x = 0` and `x = width - 1` are neighbours - while the
/// top and bottom edges of the map are boundaries, so a coastal or polar region gets a field along
/// them.
///
/// Errors are named `InvalidData`: a province map whose id count does not match its dimensions or
/// whose dimensions are not divisible by [`DIVISOR`], and a `region_of` table too short for an id
/// the image uses.
#[cfg(test)]
pub fn build_layer(map: &ProvinceMap, layer: &Layer) -> io::Result<LayerPlane> {
    map.validate()?;
    if !map.width.is_multiple_of(DIVISOR) || !map.height.is_multiple_of(DIVISOR) {
        return Err(invalid(format!(
            "province map {} x {} is not divisible by the gradient border divisor {DIVISOR}",
            map.width, map.height
        )));
    }
    let width = map.width / DIVISOR;
    let rows = map.height / DIVISOR;
    let texels = (width as usize)
        .checked_mul(rows as usize)
        .ok_or_else(|| invalid("gradient border layer size overflows"))?;
    let mut channel1 = Vec::new();
    let mut channel2 = Vec::new();
    channel1
        .try_reserve_exact(texels)
        .map_err(|_| invalid("gradient border texture exceeds available memory"))?;
    channel2
        .try_reserve_exact(texels)
        .map_err(|_| invalid("gradient border texture exceeds available memory"))?;
    channel1.resize(texels, [0; 4]);
    channel2.resize(texels, [0; 4]);
    let regions = Regions::build(map, layer)?;
    let grid = BoundaryGrid::build(map, &regions);
    fill_plane(map, &regions, &grid, width, &mut channel1, &mut channel2);
    Ok(LayerPlane {
        width,
        rows,
        channel1,
        channel2,
    })
}

/// Stacks two layer planes into the two textures the shader samples, guard rows included.
///
/// Errors are named `InvalidData` when the two planes are not the same shape.
#[cfg(test)]
pub fn stack(layer1: &LayerPlane, layer2: &LayerPlane) -> io::Result<GradientTextures> {
    if layer1.width != layer2.width || layer1.rows != layer2.rows {
        return Err(invalid(format!(
            "gradient border layers are {} x {} and {} x {}",
            layer1.width, layer1.rows, layer2.width, layer2.rows
        )));
    }
    let width = layer1.width;
    let height = 2 * (layer1.rows + 1);
    let texels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| invalid("gradient border texture size overflows"))?;
    let mut channel1 = Vec::new();
    let mut channel2 = Vec::new();
    channel1
        .try_reserve_exact(texels)
        .map_err(|_| invalid("gradient border texture exceeds available memory"))?;
    channel2
        .try_reserve_exact(texels)
        .map_err(|_| invalid("gradient border texture exceeds available memory"))?;
    channel1.resize(texels, [0; 4]);
    channel2.resize(texels, [0; 4]);
    let stride = (width as usize) * (layer1.rows as usize);
    let origin = ((layer1.rows + 1) * width) as usize;
    for (index, layer) in [layer1, layer2].into_iter().enumerate() {
        let start = index * origin;
        channel1[start..start + stride].copy_from_slice(&layer.channel1);
        channel2[start..start + stride].copy_from_slice(&layer.channel2);
    }
    Ok(GradientTextures {
        width,
        height,
        channel1,
        channel2,
    })
}

/// Builds both gradient border textures for one political map frame.
///
/// `layer1` is the strategic-region layer and `layer2` the country layer; each says how provinces
/// group into regions, what colour a region is and how opaque it is.
#[cfg(test)]
pub fn build(map: &ProvinceMap, layer1: &Layer, layer2: &Layer) -> io::Result<GradientTextures> {
    stack(&build_layer(map, layer1)?, &build_layer(map, layer2)?)
}

/// The layer's regions resolved onto the pixel grid, with one dense index per region so the
/// per-pixel work never touches a hash map or a caller closure.
#[cfg(test)]
struct Regions {
    /// Dense region index per map pixel, in the image's own top-down order. 0 means no region.
    index: Vec<u32>,
    /// Map colour per dense index; entry 0 is the unused "no region" slot.
    color: Vec<[u8; 3]>,
    attributes: Vec<Attributes>,
    /// Map-mode style per dense index.
    style: Vec<RegionStyle>,
    /// Field thickness `T` per dense index.
    thickness: Vec<f32>,
    field: FieldDefines,
}

#[cfg(test)]
impl Regions {
    fn build(map: &ProvinceMap, layer: &Layer) -> io::Result<Self> {
        let mut dense_of_key: HashMap<u32, u32> = HashMap::new();
        let mut dense_of_province = vec![0_u32; layer.region_of.len()];
        let mut keys: Vec<u32> = vec![NO_REGION];
        for (province, &key) in layer.region_of.iter().enumerate() {
            if key == NO_REGION {
                continue;
            }
            let next = keys.len() as u32;
            let dense = *dense_of_key.entry(key).or_insert_with(|| {
                keys.push(key);
                next
            });
            dense_of_province[province] = dense;
        }

        let mut index = Vec::new();
        index
            .try_reserve_exact(map.ids.len())
            .map_err(|_| invalid("region index exceeds available memory"))?;
        let mut area = vec![0_u64; keys.len()];
        for &id in map.ids {
            let dense = *dense_of_province.get(id as usize).ok_or_else(|| {
                invalid(format!(
                    "province {id} is outside the layer's region table of {} entries",
                    layer.region_of.len()
                ))
            })?;
            area[dense as usize] += 1;
            index.push(dense);
        }

        let color = keys
            .iter()
            .map(|&key| {
                if key == NO_REGION {
                    layer.no_region_color
                } else {
                    (layer.region_color)(key)
                }
            })
            .collect();
        let thickness = keys
            .iter()
            .zip(&area)
            .map(|(&key, &pixels)| layer.thickness(key, pixels))
            .collect();
        let style = keys.iter().map(|&key| layer.style(key)).collect();
        Ok(Self {
            index,
            color,
            attributes: keys
                .iter()
                .map(|&key| {
                    if key == NO_REGION {
                        Attributes::default()
                    } else {
                        layer
                            .region_attributes
                            .map_or(Attributes::default(), |read| read(key))
                    }
                })
                .collect(),
            style,
            thickness,
            field: *layer.field,
        })
    }
}

/// The boundary points of every region of one layer, bucketed into a uniform grid.
///
/// Boundary pixels have a four-neighbour in another region. Horizontal neighbours wrap. All regions
/// share a grid; search radius selects cells while preserving scan order.
#[cfg(test)]
struct BoundaryGrid {
    cell: u32,
    grid_width: u32,
    grid_height: u32,
    /// Start of each bucket in `points`, `grid_width * grid_height + 1` entries.
    offsets: Vec<u32>,
    points: Vec<[u16; 2]>,
    /// Chebyshev distance in cells from this cell to the nearest cell that holds a point, the map
    /// wrapping in x, saturated at `u32::MAX` when the layer has no boundary at all.
    ///
    /// Skip empty interior cells across the complete search radius. Thick fields span multiple cells
    /// and would otherwise repeatedly scan empty buckets.
    cell_distance: Vec<u32>,
}

#[cfg(test)]
impl BoundaryGrid {
    fn build(map: &ProvinceMap, regions: &Regions) -> Self {
        let cell = GRID_CELL;
        let grid_width = map.width.div_ceil(cell);
        let grid_height = map.height.div_ceil(cell);
        let cells = (grid_width as usize) * (grid_height as usize);
        let mut counts = vec![0_u32; cells + 1];
        let width = map.width as usize;
        let bucket_of =
            |x: u32, y: u32| (y / cell) as usize * (grid_width as usize) + (x / cell) as usize;
        for y in 0..map.height {
            for x in 0..map.width {
                if is_boundary(map, regions, width, x, y) {
                    counts[bucket_of(x, y)] += 1;
                }
            }
        }
        let mut offsets = vec![0_u32; cells + 1];
        let mut total = 0_u32;
        for (offset, count) in offsets.iter_mut().zip(counts.iter()) {
            *offset = total;
            total += count;
        }
        offsets[cells] = total;
        let mut cursor = offsets.clone();
        let mut points = vec![[0_u16; 2]; total as usize];
        for y in 0..map.height {
            for x in 0..map.width {
                if is_boundary(map, regions, width, x, y) {
                    let bucket = bucket_of(x, y);
                    points[cursor[bucket] as usize] = [x as u16, y as u16];
                    cursor[bucket] += 1;
                }
            }
        }
        let cell_distance = chebyshev_cell_distance(&offsets, grid_width, grid_height);
        Self {
            cell,
            grid_width,
            grid_height,
            offsets,
            points,
            cell_distance,
        }
    }

    /// The distance to the nearest boundary point, or `None` when there is none in range.
    ///
    /// Faithful to the engine's search: the cell radius comes from the thickness, the
    /// scan stops dead once a point is closer than `sqrt(0.5)` - which on an integer pixel grid
    /// means the pixel is itself a boundary pixel - and a bucket is abandoned once a point is
    /// further than `best + cellW^2 + cellH^2`. That last one is a heuristic and can in principle
    /// skip a nearer point in the same bucket; it is the engine's, kept for identical output. The
    /// prune on the bucket's own rectangle is ours and is exact.
    ///
    /// Search radius can end the field before field_reach. Its effective distance depends on pixel
    /// position within the grid cell.
    fn nearest(&self, width: u32, x: u32, y: u32, thickness: f32, centre: f32) -> Option<f32> {
        let radius =
            ((thickness + centre).sqrt() * RADIUS_MULTIPLIER / self.cell as f32) as i32 + 1;
        let cell_x = (x / self.cell) as i32;
        let cell_y = (y / self.cell) as i32;
        if self.cell_distance[(cell_y as usize) * (self.grid_width as usize) + cell_x as usize]
            > radius as u32
        {
            return None;
        }
        let span = 2 * radius + 1;
        let (first, last) = if span >= self.grid_width as i32 {
            (0, self.grid_width as i32 - 1)
        } else {
            (cell_x - radius, cell_x + radius)
        };
        let diagonal = 2 * self.cell * self.cell;
        let mut best = u32::MAX;
        for row in (cell_y - radius).max(0)..=(cell_y + radius).min(self.grid_height as i32 - 1) {
            for column in first..=last {
                let column = column.rem_euclid(self.grid_width as i32) as u32;
                let bucket = (row as usize) * (self.grid_width as usize) + column as usize;
                let start = self.offsets[bucket] as usize;
                let end = self.offsets[bucket + 1] as usize;
                if start == end {
                    continue;
                }
                if best != u32::MAX
                    && self.bucket_distance_squared(width, x, y, column, row as u32) >= best
                {
                    continue;
                }
                for &[point_x, point_y] in &self.points[start..end] {
                    let mut dx = (i32::from(point_x) - x as i32).unsigned_abs();
                    if dx > width / 2 {
                        dx = width - dx;
                    }
                    let dy = (i32::from(point_y) - y as i32).unsigned_abs();
                    let distance = dx * dx + dy * dy;
                    if distance < best {
                        best = distance;
                        if best == 0 {
                            return Some(0.0);
                        }
                    } else if best + diagonal < distance {
                        break;
                    }
                }
            }
        }
        (best != u32::MAX).then(|| (best as f32).sqrt())
    }

    /// The exact minimum squared distance from a pixel to any pixel a bucket can hold.
    fn bucket_distance_squared(&self, width: u32, x: u32, y: u32, column: u32, row: u32) -> u32 {
        let x0 = column * self.cell;
        let x1 = ((column + 1) * self.cell).min(width) - 1;
        let dx = if x >= x0 && x <= x1 {
            0
        } else {
            wrapped_distance(width, x, x0).min(wrapped_distance(width, x, x1))
        };
        let y0 = row * self.cell;
        let y1 = (row + 1) * self.cell - 1;
        let dy = if y >= y0 && y <= y1 {
            0
        } else {
            y.abs_diff(y0).min(y.abs_diff(y1))
        };
        dx * dx + dy * dy
    }
}

/// Chebyshev distance in cells to the nearest non-empty bucket, wrapping in x.
///
/// An eight-neighbour breadth-first search from every non-empty bucket at once, which is Chebyshev
/// distance by construction; the grid is the map over [`GRID_CELL`] squared, a few tens of
/// thousands of cells, so nothing cleverer is worth the risk of getting a chamfer's sweep order
/// wrong on a cylinder.
#[cfg(test)]
fn chebyshev_cell_distance(offsets: &[u32], grid_width: u32, grid_height: u32) -> Vec<u32> {
    let width = grid_width as usize;
    let height = grid_height as usize;
    let mut distance = vec![u32::MAX; width * height];
    let mut queue = std::collections::VecDeque::new();
    for cell in 0..width * height {
        if offsets[cell + 1] > offsets[cell] {
            distance[cell] = 0;
            queue.push_back(cell);
        }
    }
    while let Some(cell) = queue.pop_front() {
        let row = (cell / width) as i32;
        let column = (cell % width) as i32;
        let next = distance[cell] + 1;
        for dy in -1..=1_i32 {
            let neighbour_row = row + dy;
            if neighbour_row < 0 || neighbour_row >= height as i32 {
                continue;
            }
            for dx in -1..=1_i32 {
                let neighbour_column = (column + dx).rem_euclid(grid_width as i32);
                let neighbour = (neighbour_row as usize) * width + neighbour_column as usize;
                if distance[neighbour] > next {
                    distance[neighbour] = next;
                    queue.push_back(neighbour);
                }
            }
        }
    }
    distance
}

#[cfg(test)]
fn wrapped_distance(width: u32, a: u32, b: u32) -> u32 {
    let direct = a.abs_diff(b);
    direct.min(width - direct)
}

/// Whether the pixel belongs to a region and touches a different one, the map wrapping in x.
///
/// Only neighbours within the bitmap contribute vertical boundaries. The top and bottom map edges
/// do not create an artificial country band.
#[cfg(test)]
fn is_boundary(map: &ProvinceMap, regions: &Regions, width: usize, x: u32, y: u32) -> bool {
    let row = (y as usize) * width;
    let here = regions.index[row + x as usize];
    if here == NO_REGION {
        return false;
    }
    let left = if x == 0 { map.width - 1 } else { x - 1 };
    let right = if x + 1 == map.width { 0 } else { x + 1 };
    if regions.index[row + left as usize] != here || regions.index[row + right as usize] != here {
        return true;
    }
    (y > 0 && regions.index[row - width + x as usize] != here)
        || (y + 1 < map.height && regions.index[row + width + x as usize] != here)
}

/// Writes one layer's rows of both textures.
///
/// The full-resolution field is never materialised: each output texel averages the
/// [`DIVISOR`] x [`DIVISOR`] block of map pixels it covers. Rows run top-down in province-bitmap
/// order, matching the shader's flipped coordinate; see [`GradientTextures`]. Each scoped thread
/// writes a separate row band and shares only immutable state.
#[cfg(test)]
fn fill_plane(
    map: &ProvinceMap,
    regions: &Regions,
    grid: &BoundaryGrid,
    plane_width: u32,
    channel1: &mut [[u8; 4]],
    channel2: &mut [[u8; 4]],
) {
    let rows = map.height / DIVISOR;
    let threads = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(rows.max(1) as usize);
    let band = (rows as usize).div_ceil(threads.max(1));
    let stride = plane_width as usize;
    thread::scope(|scope| {
        for (index, (first, second)) in channel1
            .chunks_mut(band * stride)
            .zip(channel2.chunks_mut(band * stride))
            .enumerate()
        {
            let origin = (index * band) as u32;
            scope.spawn(move || {
                fill_band(map, regions, grid, plane_width, origin, first, second);
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn fill_band(
    map: &ProvinceMap,
    regions: &Regions,
    grid: &BoundaryGrid,
    plane_width: u32,
    origin: u32,
    channel1: &mut [[u8; 4]],
    channel2: &mut [[u8; 4]],
) {
    let stride = plane_width as usize;
    for (offset, (first, second)) in channel1
        .chunks_mut(stride)
        .zip(channel2.chunks_mut(stride))
        .enumerate()
    {
        let plane_row = origin + offset as u32;
        for column in 0..plane_width {
            let mut sum = [0_u32; 4];
            let mut effects = [0_u32; 4];
            // Average vertically and truncate to bytes, then average horizontally. A single rounded
            // box mean loses both truncations.
            for dx in 0..DIVISOR {
                let mut vertical = [0_u32; 4];
                let mut vertical_effects = [0_u32; 4];
                for dy in 0..DIVISOR {
                    let y = plane_row * DIVISOR + dy;
                    let x = column * DIVISOR + dx;
                    let (texel, covered) = sample(map, regions, grid, x, y);
                    for (total, channel) in vertical.iter_mut().zip(texel) {
                        *total += u32::from(channel);
                    }
                    for (total, channel) in vertical_effects.iter_mut().zip(covered) {
                        *total += u32::from(channel);
                    }
                }
                for (total, channel) in sum.iter_mut().zip(vertical) {
                    *total += channel / DIVISOR;
                }
                for (total, channel) in effects.iter_mut().zip(vertical_effects) {
                    *total += channel / DIVISOR;
                }
            }
            let average = |total: u32| (total / DIVISOR) as u8;
            first[column as usize] = sum.map(average);
            second[column as usize] = effects.map(average);
        }
    }
}

/// One map pixel of the field and all four channel2 bytes.
///
/// Pixels beyond boundary-search reach keep their region colour and opacity with field alpha
/// zero, preserving the camera-dependent interior fill. Pixels outside every region use
/// [`NO_REGION_ALPHA`] and zero ordinary opacity, keeping the sea clear.
#[cfg(test)]
fn sample(
    map: &ProvinceMap,
    regions: &Regions,
    grid: &BoundaryGrid,
    x: u32,
    y: u32,
) -> ([u8; 4], [u8; 4]) {
    let region = regions.index[(y as usize) * (map.width as usize) + (x as usize)];
    if region == NO_REGION {
        let cell = (y / grid.cell * grid.grid_width + x / grid.cell) as usize;
        if grid.cell_distance[cell] > 1 {
            let [red, green, blue] = regions.color[NO_REGION as usize];
            return ([red, green, blue, NO_REGION_ALPHA], [0; 4]);
        }
        // Extend colour and opacity one map pixel across coasts, averaging the eight neighbours
        // belonging to a region. Out-of-bitmap neighbours are absent. Field alpha stays 255,
        // preventing dark fringes without creating country interior on water.
        let mut sum = [0_u32; 3];
        let mut fx = [0_u8; 2];
        let mut opacity = 0_u32;
        let mut pulse = 0_u32;
        let mut gradient = 0_u32;
        let mut has_gradient = false;
        let mut count = 0;
        for dy in -1..=1_i32 {
            for dx in -1..=1_i32 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= map.width as i32 || ny >= map.height as i32 {
                    continue;
                }
                let neighbour = regions.index[ny as usize * map.width as usize + nx as usize];
                if neighbour != NO_REGION {
                    for (total, channel) in sum.iter_mut().zip(regions.color[neighbour as usize]) {
                        *total += u32::from(channel);
                    }
                    let attr = regions.attributes[neighbour as usize];
                    let style = regions.style[neighbour as usize];
                    opacity += u32::from(style.alpha);
                    pulse += if style.pulse { 255 } else { 0 };
                    // Add these as bytes before unsigned division.
                    fx[0] = fx[0].wrapping_add(attr.control);
                    fx[1] = fx[1].wrapping_add(attr.contested);
                    if let Some(value) = attr.gradient {
                        gradient += u32::from(value);
                        has_gradient = true;
                    }
                    count += 1;
                }
            }
        }
        if let Some(count) = std::num::NonZeroU32::new(count) {
            let count = count.get();
            let [red, green, blue] = sum.map(|channel| (channel / count) as u8);
            return (
                [
                    red,
                    green,
                    blue,
                    if has_gradient {
                        (gradient / count) as u8
                    } else {
                        NO_REGION_ALPHA
                    },
                ],
                [
                    (u32::from(fx[0]) / count) as u8,
                    (opacity / count) as u8,
                    (pulse / count) as u8,
                    (u32::from(fx[1]) / count) as u8,
                ],
            );
        }
        let [red, green, blue] = regions.color[NO_REGION as usize];
        return ([red, green, blue, NO_REGION_ALPHA], [0; 4]);
    }
    let thickness = regions.thickness[region as usize];
    let field = &regions.field;
    let alpha = match grid.nearest(map.width, x, y, thickness, field.center) {
        Some(distance) if distance <= field_reach(thickness, field) => {
            field_alpha(distance, thickness, field.center)
        }
        _ => 0,
    };
    let [red, green, blue] = regions.color[region as usize];
    let attr = regions.attributes[region as usize];
    let style = regions.style[region as usize];
    (
        [
            red,
            green,
            blue,
            filled_alpha(attr.gradient.unwrap_or(alpha), style.fill),
        ],
        [
            attr.control,
            style.alpha,
            if style.pulse { 255 } else { 0 },
            attr.contested,
        ],
    )
}

#[cfg(test)]
use viewer_foundation::map::gradient_math::hsv_to_rgb;

#[cfg(test)]
mod tests;
