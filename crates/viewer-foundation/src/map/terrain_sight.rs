//! Sight lines from a free eye over the drawn terrain.
//!
//! A free eye sees a map icon's anchor unless the drawn ground or water stands between them. The
//! test is one sight line per anchor: it walks the line over a pyramid of block maxima of the
//! height bytes, passes every block the line clears by at least the caller's band, and samples the
//! drawn triangle surface ([`HeightField::surface_height`]) only where the line comes closer than
//! that. The map wraps in `x` and the line walks the wrapped cells; rows beyond the map's top and
//! bottom read its edge rows, as the surface does.

use super::province_map::{HEIGHT_SCALE, HeightField, WATER_HEIGHT};

/// Block maxima of a height field: level `k` holds the highest byte under each block of `2^k` by
/// `2^k` terrain cells, a cell being the square between four neighbouring height samples.
#[derive(Clone, Debug, Default)]
pub struct TerrainSight {
    /// Height samples a row, which is also the number of cells a row: the last cell joins the
    /// last column to the first, as the map wraps.
    width: u32,
    /// Height sample rows; cells span one row fewer.
    rows: u32,
    /// Levels 1, 2, ... up to a single block.
    levels: Vec<Level>,
    /// The highest drawn surface anywhere, water included.
    ceiling: f32,
}

#[derive(Clone, Debug)]
struct Level {
    columns: u32,
    rows: u32,
    max: Vec<u8>,
}

impl Level {
    fn get(&self, column: u32, row: u32) -> u8 {
        self.max[(row * self.columns + column) as usize]
    }
}

impl TerrainSight {
    /// The pyramid of `field`; empty (every line clear) when the field holds no full raster.
    pub fn new(field: &HeightField<'_>) -> Self {
        let (width, rows) = (field.width, field.height);
        if width == 0 || rows < 2 || field.values.len() < (width as usize) * (rows as usize) {
            return Self::default();
        }
        // A sample by column and bottom-up row; bitmap rows run down.
        let sample = |column: u32, bottom: u32| {
            let row = (rows - 1 - bottom) as usize;
            field.values[row * width as usize + (column % width) as usize]
        };
        let cell_rows = rows - 1;
        // Level 1 reads the 3 by 3 samples around each 2 by 2 block of cells.
        let first_columns = width.div_ceil(2);
        let first_rows = cell_rows.div_ceil(2);
        let mut max = Vec::with_capacity((first_columns * first_rows) as usize);
        for block_row in 0..first_rows {
            let bottoms = 2 * block_row..=(2 * block_row + 2).min(rows - 1);
            for block_column in 0..first_columns {
                let mut high = 0;
                for bottom in bottoms.clone() {
                    for column in 2 * block_column..=2 * block_column + 2 {
                        high = high.max(sample(column, bottom));
                    }
                }
                max.push(high);
            }
        }
        let mut levels = vec![Level {
            columns: first_columns,
            rows: first_rows,
            max,
        }];
        while let Some(last) = levels.last()
            && (last.columns > 1 || last.rows > 1)
        {
            let columns = last.columns.div_ceil(2);
            let level_rows = last.rows.div_ceil(2);
            let mut max = Vec::with_capacity((columns * level_rows) as usize);
            for row in 0..level_rows {
                for column in 0..columns {
                    let mut high = 0;
                    for r in 2 * row..(2 * row + 2).min(last.rows) {
                        for c in 2 * column..(2 * column + 2).min(last.columns) {
                            high = high.max(last.get(c, r));
                        }
                    }
                    max.push(high);
                }
            }
            levels.push(Level {
                columns,
                rows: level_rows,
                max,
            });
        }
        let ceiling = levels
            .last()
            .map_or(WATER_HEIGHT, |top| surface(top.max[0]));
        Self {
            width,
            rows,
            levels,
            ceiling,
        }
    }

    /// The highest drawn surface anywhere on the map.
    pub fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// How far the sight line from `eye` to `target` passes above the drawn surface at its
    /// lowest, the part within `skip` of the target left out (the target's own ground), and held
    /// within `[-band, band]`: anything clearer than `band` reads `band`, and the walk stops as
    /// soon as the line is `band` below the surface. Both points are world positions; `target`
    /// is taken on the map copy nearest the eye.
    pub fn clearance(
        &self,
        field: &HeightField<'_>,
        eye: [f32; 3],
        target: [f32; 3],
        skip: f32,
        band: f32,
    ) -> f32 {
        let band = band.max(0.0);
        if self.levels.is_empty() {
            return band;
        }
        let width = self.width as f32;
        let mut dx = target[0] - eye[0];
        dx -= width * (dx / width).round();
        let dz = target[2] - eye[2];
        let length = dx.hypot(dz);
        let end = length - skip.max(0.0);
        if !(length.is_finite() && end > 0.0) {
            return band;
        }
        let slope = (target[1] - eye[1]) / length;
        let direction = [dx / length, dz / length];
        let height = |u: f32| eye[1] + slope * u;
        let point = |u: f32| [eye[0] + direction[0] * u, eye[2] + direction[1] * u];
        // Nothing stands above the ceiling: start where the line comes down to it.
        let roof = self.ceiling + band;
        let mut u = if eye[1] <= roof {
            0.0
        } else if slope < 0.0 {
            (eye[1] - roof) / -slope
        } else {
            return band;
        };
        let mut least = band;
        'walk: while u < end {
            for (index, level) in self.levels.iter().enumerate().rev() {
                let size = (2u32 << index) as f32;
                let next = (u + size).min(end);
                let low = height(u).min(height(next));
                if low - self.block_high(index, level, point(u), point(next)) >= band {
                    u = next;
                    continue 'walk;
                }
            }
            // Within the band of the blocks, for about a cell: walk the drawn surface plane by
            // plane. Between the points where the line enters and leaves a triangle the clearance
            // is linear (or the least of two linear terms over water), so those points hold its
            // lowest values and no crest falls between them.
            let stop = (u + 1.0).min(end);
            let mut at = u;
            loop {
                let [x, z] = point(at);
                least = least.min(height(at) - field.surface_height(x, z));
                if least <= -band {
                    return -band;
                }
                if at >= stop {
                    break;
                }
                at = (at + next_break(point(at), direction)).min(stop);
            }
            u = stop;
        }
        least
    }

    /// The highest surface under every block of the level that the cells between `from` and `to`
    /// touch: each block column the cells' `x` range passes, taken across the map's seam (where
    /// the last block, which may be narrower, meets the first), by each block row their `z` range
    /// spans.
    fn block_high(&self, index: usize, level: &Level, from: [f32; 2], to: [f32; 2]) -> f32 {
        let shift = index + 1;
        let width = i64::from(self.width);
        let block_row = |z: f32| {
            let row = (z.floor() as i64).clamp(0, i64::from(self.rows) - 2) as u32;
            (row >> shift).min(level.rows - 1)
        };
        let (low_row, high_row) = (block_row(from[1].min(to[1])), block_row(from[1].max(to[1])));
        let last_cell = from[0].max(to[0]).floor() as i64;
        let mut cell = from[0].min(to[0]).floor() as i64;
        let mut high = 0;
        loop {
            let column = cell.rem_euclid(width);
            let block = ((column as u32) >> shift).min(level.columns - 1);
            for row in low_row..=high_row {
                high = high.max(level.get(block, row));
            }
            // On to the next block's first cell, or the first cell past the seam.
            let block_end = (i64::from(block + 1) << shift).min(width);
            cell += block_end - column;
            if cell > last_cell {
                break;
            }
        }
        surface(high)
    }
}

/// How far along `direction` (a horizontal unit vector) from `from` the drawn surface next changes
/// plane: the next edge of the cell the line runs through, or the diagonal the terrain's index
/// pattern splits that cell along ([`HeightField::surface_height`]).
fn next_break(from: [f32; 2], direction: [f32; 2]) -> f32 {
    // The cell the line runs through just past `from`.
    let ahead = [0, 1].map(|axis| from[axis] + direction[axis] * BREAK_EPSILON);
    let (column, bottom) = (ahead[0].floor(), ahead[1].floor());
    let mut step = f32::INFINITY;
    for (axis, start) in [(0, column), (1, bottom)] {
        let rate = direction[axis];
        if rate > 0.0 {
            step = step.min((start + 1.0 - from[axis]) / rate);
        } else if rate < 0.0 {
            step = step.min((start - from[axis]) / rate);
        }
    }
    let (fx, fz) = (from[0] - column, from[1] - bottom);
    let (value, rate) = if (column as i64 + bottom as i64).rem_euclid(2) == 0 {
        (fx - fz, direction[0] - direction[1])
    } else {
        (fx + fz - 1.0, direction[0] + direction[1])
    };
    if rate != 0.0 {
        let diagonal = -value / rate;
        if diagonal > BREAK_EPSILON && diagonal < step {
            step = diagonal;
        }
    }
    step.max(BREAK_EPSILON)
}

/// The least step the surface walk takes, past a break it stands on.
const BREAK_EPSILON: f32 = 1.0e-4;

/// The drawn surface over a height byte: the byte's height or the water above it.
fn surface(byte: u8) -> f32 {
    (f32::from(byte) * HEIGHT_SCALE).max(WATER_HEIGHT)
}

#[cfg(test)]
mod tests;
