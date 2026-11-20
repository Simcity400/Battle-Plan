//! Coarse terrain along coasts.
//!
//! Zoomed out, a coarse terrain block samples the heightmap only at every `step`-th pixel, so a
//! coastal or island vertex lifts whole cells of sea above the water plane: the water fails its
//! depth test there and the terrain's grey sea floor shows as specks and squares along coasts and
//! around islands. The game draws them too; by the owner's sign-off the viewer does not, for every
//! camera. On blocks coarser than the heightmap, a vertex next to a cell holding any pixel below
//! the water stands no higher than the heightmap's shallowest water, so the water covers every such
//! cell and its own per-pixel shore decides what shows. Vertices touching no such cell keep their
//! heights exactly.
//!
//! Blocks of different steps meet at their rims, where each must give a shared vertex the same
//! height or the seam opens. A vertex on a rim is therefore capped by the cells of the coarsest
//! block it touches ([`Steps::rule`]), which every block around it reads alike; a vertex touching a
//! block of the finest step keeps its sampled height, so the finest blocks are never changed.

use super::terrain_shader::{Patch, SIDE, heights_into};

/// The engine stream stores a height byte times this.
const STORED_PER_BYTE: i16 = 10;

/// Which `step`-sized cells hold a pixel below the water, for each step from 2 up.
pub struct Coast {
    /// The stored height a capped vertex takes: the highest height byte below the water.
    cap: i16,
    /// Level `k` holds the cells of step `2^(k+1)`, rows counted up from the map's bottom.
    levels: Vec<Level>,
}

struct Level {
    columns: u32,
    rows: u32,
    wet: Vec<bool>,
}

impl Level {
    fn wet(&self, column: u32, row: u32) -> bool {
        self.wet[(row * self.columns + column) as usize]
    }

    /// Whether the vertex at map pixel `(x, z)` touches a wet cell of this level's `step`.
    fn near(&self, step: u32, x: u32, z: u32) -> bool {
        let [column, row] = [x / step, z / step];
        [column + self.columns - 1, column].into_iter().any(|column| {
            let column = column % self.columns;
            [row.checked_sub(1), Some(row)]
                .into_iter()
                .flatten()
                .filter(|&row| row < self.rows)
                .any(|row| self.wet(column, row))
        })
    }
}

impl Coast {
    /// `bytes` is the heightmap, rows top-down, of a `map`-sized map; `water` the water plane's
    /// height in world units and `scale` world units per height byte. Steps reach `widest`.
    pub fn new(bytes: &[u8], map: [u32; 2], water: f32, scale: f32, widest: u32) -> Self {
        let below = |byte: u8| f32::from(byte) * scale < water;
        let cap = (0..=u8::MAX).rev().find(|&byte| below(byte)).unwrap_or(0);
        let mut levels: Vec<Level> = Vec::new();
        let mut step = 2;
        while step <= widest.max(2) {
            let [columns, rows] = map.map(|side| side.div_ceil(step));
            let mut wet = vec![false; columns as usize * rows as usize];
            for row in 0..rows {
                for column in 0..columns {
                    wet[(row * columns + column) as usize] = match levels.last() {
                        // The finest cells read the heightmap's pixels.
                        None => (0..2).any(|dz| {
                            (0..2).any(|dx| {
                                let [x, z] = [column * 2 + dx, row * 2 + dz];
                                x < map[0]
                                    && z < map[1]
                                    && below(bytes[((map[1] - 1 - z) * map[0] + x) as usize])
                            })
                        }),
                        // Coarser cells are wet where any of their four finer cells is.
                        Some(finer) => (0..2).any(|dz| {
                            (0..2).any(|dx| {
                                let [x, z] = [column * 2 + dx, row * 2 + dz];
                                x < finer.columns && z < finer.rows && finer.wet(x, z)
                            })
                        }),
                    };
                }
            }
            levels.push(Level { columns, rows, wet });
            step *= 2;
        }
        Self {
            cap: i16::from(cap) * STORED_PER_BYTE,
            levels,
        }
    }

    fn level(&self, step: u32) -> Option<&Level> {
        (step >= 2 && step.is_power_of_two())
            .then(|| self.levels.get(step.trailing_zeros() as usize - 1))
            .flatten()
    }

    /// `height` as a vertex at map pixel `(x, z)` capped by the cells of `step` draws it; no step
    /// keeps it.
    fn capped(&self, height: i16, step: Option<u32>, x: u32, z: u32) -> i16 {
        match step.and_then(|step| Some((step, self.level(step)?))) {
            Some((step, level)) if height > self.cap && level.near(step, x, z) => self.cap,
            _ => height,
        }
    }

    /// Caps `heights`, a `step`-spaced patch's pairs from `origin` for `positions`, where a vertex
    /// touches a wet cell of its own step. The finest step and steps past the widest are left as
    /// sampled. Returns whether any height changed.
    pub fn cap(
        &self,
        positions: &[[i16; 4]],
        origin: [u32; 2],
        step: u32,
        heights: &mut [[i16; 2]],
    ) -> bool {
        if self.level(step).is_none() {
            return false;
        }
        let patch = Patch { origin, step };
        let mut changed = false;
        for (&position, pair) in positions.iter().zip(heights) {
            for (height, at) in pair.iter_mut().zip(samples(position)) {
                let [x, z] = world(patch, at);
                let capped = self.capped(*height, Some(step), x, z);
                changed |= capped != *height;
                *height = capped;
            }
        }
        changed
    }

    /// The heights `patch` draws on world map copy `copy` among the blocks `steps` holds, when
    /// they differ from its own capped heights: each sample on its drawn rim (`cells` across and
    /// up) capped as [`Steps::rule`] says, the rest by its own step.
    #[allow(
        clippy::too_many_arguments,
        reason = "the block, where it stands, and the map it samples"
    )]
    pub(super) fn rim(
        &self,
        steps: &Steps,
        positions: &[[i16; 4]],
        bytes: &[u8],
        map: [u32; 2],
        patch: Patch,
        cells: [u32; 2],
        copy: i64,
    ) -> Option<Vec<[i16; 2]>> {
        let mut heights = Vec::with_capacity(positions.len());
        heights_into(positions, bytes, map, patch, &mut heights);
        let mut differs = false;
        for (&position, pair) in positions.iter().zip(&mut heights) {
            for (height, at) in pair.iter_mut().zip(samples(position)) {
                let [x, z] = world(patch, at);
                let own = self.capped(*height, Some(patch.step), x, z);
                *height = self.capped(*height, steps.at_vertex(patch, cells, copy, at), x, z);
                differs |= *height != own;
            }
        }
        differs.then_some(heights)
    }
}

/// Where a vertex's two samples stand, in vertex units of its patch: its own place, and for an
/// odd vertex on the patch's edge the next vertex along it, where a snapped edge moves it.
fn samples([x, z, edge, direction]: [i16; 4]) -> [[u32; 2]; 2] {
    let at = [x as u32, z as u32];
    let alternate = if edge == 0 {
        at
    } else {
        [
            at[0] + u32::from(direction == 0),
            at[1] + u32::from(direction == 1),
        ]
    };
    [at, alternate]
}

/// Map pixel `(x, z)` of a patch's vertex `at`.
fn world(patch: Patch, [x, z]: [u32; 2]) -> [u32; 2] {
    [
        patch.origin[0] + x * patch.step,
        patch.origin[1] + z * patch.step,
    ]
}

/// Whether vertex `at` stands on the rim of a patch drawing `cells` across and up.
fn on_rim(cells: [u32; 2], [x, z]: [u32; 2]) -> bool {
    x <= cells[0] && z <= cells[1] && (x == 0 || z == 0 || x == cells[0] || z == cells[1])
}

/// The vertices [`on_rim`] holds for a patch drawing `cells` across and up, in the order the
/// patch's positions list them (row by row, west to east), without walking its inside.
fn rim_vertices([wide, tall]: [u32; 2]) -> impl Iterator<Item = [u32; 2]> {
    (0..=tall).flat_map(move |z| {
        let whole = z == 0 || z == tall || wide == 0;
        let row = whole.then(|| (0..=wide).map(move |x| [x, z]));
        let ends = (!whole).then(|| [[0, z], [wide, z]]);
        row.into_iter().flatten().chain(ends.into_iter().flatten())
    })
}

/// The step of the block a cover draws over each 64-pixel cell, across the map copies it stands
/// on.
pub(super) struct Steps {
    /// The world column, counted across the copies from the first copy's first, of the first
    /// cell held, and the columns held.
    first: i64,
    columns: i64,
    /// The map's columns and rows of cells.
    map: [u32; 2],
    /// Each cell's step, 0 where no block is drawn.
    steps: Vec<u32>,
}

impl Steps {
    /// `blocks` gives each drawn block's first cell, as a world column and a row, and its step;
    /// `map` is the map's columns and rows of cells.
    pub(super) fn new(blocks: &[(i64, u32, u32)], map: [u32; 2]) -> Self {
        let first = blocks.iter().map(|block| block.0).min().unwrap_or(0);
        let end = blocks
            .iter()
            .map(|block| block.0 + i64::from(block.2))
            .max()
            .unwrap_or(0);
        let columns = (end - first).max(0);
        let mut held = Self {
            first,
            columns,
            map,
            steps: vec![0; (columns * i64::from(map[1])) as usize],
        };
        for &(column, row, step) in blocks {
            let [wide, tall] = held.span(column, row, step).map(i64::from);
            for dz in 0..tall {
                for dx in 0..wide {
                    let at = (i64::from(row) + dz) * columns + column - first + dx;
                    held.steps[at as usize] = step;
                }
            }
        }
        held
    }

    /// The cells a block starting at world column `column` and row `row` covers across and up:
    /// a block the map's edge cuts covers only those inside the map.
    fn span(&self, column: i64, row: u32, step: u32) -> [u32; 2] {
        let local = column.rem_euclid(i64::from(self.map[0])) as u32;
        [
            step.min(self.map[0].saturating_sub(local)),
            step.min(self.map[1].saturating_sub(row)),
        ]
    }

    fn at(&self, column: i64, row: i64) -> u32 {
        let x = column - self.first;
        if (0..self.columns).contains(&x) && (0..i64::from(self.map[1])).contains(&row) {
            self.steps[(row * self.columns + x) as usize]
        } else {
            0
        }
    }

    /// Whether every drawn cell around the block starting at world column `column` and row
    /// `row` draws its `step`, so its whole rim takes its own step's cells.
    pub(super) fn alone(&self, column: i64, row: u32, step: u32) -> bool {
        let [wide, tall] = self.span(column, row, step).map(i64::from);
        let row = i64::from(row);
        let other = |column: i64, row: i64| ![0, step].contains(&self.at(column, row));
        !(column - 1..=column + wide).any(|x| other(x, row - 1) || other(x, row + tall))
            && !(row..row + tall).any(|z| other(column - 1, z) || other(column + wide, z))
    }

    /// Whether `other` holds the same steps as this in every cell around the block starting at
    /// world column `column` and row `row`, the cells [`Self::alone`] and [`Self::rim`] read
    /// besides its own: the block drawn in both has the same rim in both.
    pub(super) fn same_around(&self, other: &Self, column: i64, row: u32, step: u32) -> bool {
        let [wide, tall] = self.span(column, row, step).map(i64::from);
        let row = i64::from(row);
        let same = |column: i64, row: i64| self.at(column, row) == other.at(column, row);
        (column - 1..=column + wide).all(|x| same(x, row - 1) && same(x, row + tall))
            && (row..row + tall).all(|z| same(column - 1, z) && same(column + wide, z))
    }

    /// The cells vertex `at` of `patch` (`cells` of them across and up) on world map copy `copy`
    /// is capped by: [`Self::rule`] on its drawn rim, its own step within.
    fn at_vertex(&self, patch: Patch, cells: [u32; 2], copy: i64, at: [u32; 2]) -> Option<u32> {
        if !on_rim(cells, at) {
            return Some(patch.step);
        }
        let [x, z] = world(patch, at);
        self.rule(
            copy * i64::from(self.map[0] * SIDE) + i64::from(x),
            i64::from(z),
        )
    }

    /// The rules of the drawn rim's vertices of `patch` on world map copy `copy`, in the order
    /// of the patch's positions: 0 keeps the sampled height, else the power of two of the capping
    /// cells. With the patch they decide what [`Coast::rim`] makes. Returns whether any differs
    /// from the patch's own step.
    pub(super) fn rim(&self, patch: Patch, cells: [u32; 2], copy: i64, out: &mut Vec<u8>) -> bool {
        let code = |rule: Option<u32>| rule.map_or(0, |step| step.trailing_zeros() as u8);
        out.clear();
        out.extend(rim_vertices(cells).map(|at| code(self.at_vertex(patch, cells, copy, at))));
        out.iter().any(|&rule| rule != code(Some(patch.step)))
    }

    /// The cells a vertex at world pixel `(x, z)` is capped by: those of the coarsest drawn block
    /// it touches, or none where it touches a block of the finest step, which keeps its height.
    /// A vertex on the east-west wrap keeps its height too: the game never snaps across the wrap,
    /// so blocks of different steps leave the gap there that they always did.
    pub(super) fn rule(&self, x: i64, z: i64) -> Option<u32> {
        let side = i64::from(SIDE);
        if x.rem_euclid(i64::from(self.map[0]) * side) == 0 {
            return None;
        }
        let around = |at: i64| {
            let cell = at.div_euclid(side);
            if at.rem_euclid(side) == 0 {
                [cell - 1, cell]
            } else {
                [cell, cell]
            }
        };
        let mut coarsest = 0;
        for column in around(x) {
            for row in around(z) {
                match self.at(column, row) {
                    0 => {}
                    1 => return None,
                    step => coarsest = coarsest.max(step),
                }
            }
        }
        (coarsest >= 2).then_some(coarsest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: [u32; 2] = [128, 64];
    const WATER: f32 = 9.5;
    const SCALE: f32 = 0.1;

    /// A sea of byte 40 with a land rectangle of byte 200 and a one-pixel island of byte 150, rows
    /// top-down as the heightmap stores them.
    fn heightmap() -> Vec<u8> {
        let mut bytes = vec![40u8; (MAP[0] * MAP[1]) as usize];
        let mut put = |x: u32, z: u32, byte: u8| {
            bytes[((MAP[1] - 1 - z) * MAP[0] + x) as usize] = byte;
        };
        for z in 8..40 {
            for x in 16..80 {
                put(x, z, 200);
            }
        }
        put(100, 50, 150);
        bytes
    }

    fn sampled(bytes: &[u8], origin: [u32; 2], step: u32) -> Vec<[i16; 2]> {
        let mut out = Vec::new();
        super::super::terrain_shader::heights_into(
            &super::super::terrain_shader::positions(),
            bytes,
            MAP,
            super::super::terrain_shader::Patch { origin, step },
            &mut out,
        );
        out
    }

    /// The stored heights of the four corners of the cell at world `(x, z)`, which bound the
    /// surface over it.
    fn cell_corners(heights: &[[i16; 2]], origin: [u32; 2], step: u32, x: u32, z: u32) -> [i16; 4] {
        let side = (super::super::terrain_shader::SIDE + 1) as usize;
        let [cx, cz] = [((x - origin[0]) / step) as usize, ((z - origin[1]) / step) as usize];
        [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dz)| heights[(cz + dz) * side + cx + dx][0])
    }

    #[test]
    fn coarse_cells_holding_water_lie_below_it_and_cells_away_from_water_keep_their_heights() {
        let bytes = heightmap();
        let coast = Coast::new(&bytes, MAP, WATER, SCALE, 256);
        assert_eq!(coast.cap, 940, "the highest byte below 9.5 is 94");
        let positions = super::super::terrain_shader::positions();
        let wet_pixel = |x: u32, z: u32| {
            f32::from(bytes[((MAP[1] - 1 - z.min(MAP[1] - 1)) * MAP[0] + x % MAP[0]) as usize])
                * SCALE
                < WATER
        };
        for step in [2, 4, 8] {
            let raw = sampled(&bytes, [0, 0], step);
            let mut capped = raw.clone();
            assert!(coast.cap(&positions, [0, 0], step, &mut capped), "step {step}");
            let wet = |x: u32, z: u32| {
                (z..(z + step).min(MAP[1])).any(|pz| (x..x + step).any(|px| wet_pixel(px, pz)))
            };
            for z in (0..MAP[1] - step).step_by(step as usize) {
                for x in (0..MAP[0] - step).step_by(step as usize) {
                    let corners = cell_corners(&capped, [0, 0], step, x, z);
                    if wet(x, z) {
                        assert!(corners.iter().all(|&h| h <= 940), "step {step} cell {x},{z}");
                    }
                    // A cell whose neighbourhood holds no water keeps its sampled corners.
                    let around = [x.wrapping_sub(step), x, x + step]
                        .into_iter()
                        .map(|x| x.min(MAP[0] - step))
                        .any(|cx| {
                            [z.saturating_sub(step), z, (z + step).min(MAP[1] - step)]
                                .into_iter()
                                .any(|cz| wet(cx, cz))
                        });
                    if !around && x >= step {
                        assert_eq!(
                            corners,
                            cell_corners(&raw, [0, 0], step, x, z),
                            "step {step}: cell {x},{z} away from water keeps its heights"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_finest_grid_and_the_sea_floor_are_never_changed() {
        let bytes = heightmap();
        let coast = Coast::new(&bytes, MAP, WATER, SCALE, 256);
        let positions = super::super::terrain_shader::positions();
        let raw = sampled(&bytes, [0, 0], 1);
        let mut capped = raw.clone();
        assert!(!coast.cap(&positions, [0, 0], 1, &mut capped));
        assert_eq!(capped, raw);
        // Capping only lowers land: the sea floor's byte 40 stays wherever it was sampled.
        let raw = sampled(&bytes, [0, 0], 4);
        let mut capped = raw.clone();
        coast.cap(&positions, [0, 0], 4, &mut capped);
        for (raw, capped) in raw.iter().zip(&capped) {
            for (&raw, &capped) in raw.iter().zip(capped) {
                assert!(capped == raw || (raw > 940 && capped == 940));
            }
        }
    }

    #[test]
    fn an_island_between_coarse_vertices_no_longer_lifts_the_sea_around_it() {
        let bytes = heightmap();
        let coast = Coast::new(&bytes, MAP, WATER, SCALE, 256);
        let positions = super::super::terrain_shader::positions();
        // At step 2 the island's pixel (100, 50) is a vertex, raising the four cells around it.
        let raw = sampled(&bytes, [64, 0], 2);
        let side = (super::super::terrain_shader::SIDE + 1) as usize;
        let island = 25 * side + 18;
        assert_eq!(raw[island][0], 1500);
        let mut capped = raw.clone();
        coast.cap(&positions, [64, 0], 2, &mut capped);
        assert_eq!(capped[island][0], 940);
    }

    #[test]
    fn the_rim_walk_lists_the_positions_on_the_rim_in_their_order() {
        let positions = super::super::terrain_shader::positions();
        let sides = [0, 1, 2, 7, 31, SIDE - 1, SIDE];
        for wide in sides {
            for tall in sides {
                let cells = [wide, tall];
                let filtered: Vec<[u32; 2]> = positions
                    .iter()
                    .map(|&[x, z, ..]| [x as u32, z as u32])
                    .filter(|&at| on_rim(cells, at))
                    .collect();
                assert_eq!(rim_vertices(cells).collect::<Vec<_>>(), filtered, "cells {cells:?}");
            }
        }
    }

    /// Two covers that differ only away from a block give it the same rim; one that changes a
    /// cell beside it, its corner's included, is seen.
    #[test]
    fn a_block_with_the_same_steps_around_it_has_the_same_rim_in_another_cover() {
        let map = [16, 8];
        let first: [(i64, u32, u32); 7] =
            [(0, 0, 4), (4, 0, 2), (6, 0, 2), (4, 2, 2), (6, 2, 1), (7, 2, 1), (8, 0, 8)];
        // The step-8 block splits in two step-4 blocks, far from the blocks at the west.
        let mut moved: Vec<_> = first[..6].to_vec();
        moved.extend([(8, 0, 4), (12, 0, 4), (8, 4, 4), (12, 4, 4)]);
        let (was, now) = (Steps::new(&first, map), Steps::new(&moved, map));
        let rim = |steps: &Steps, &(column, row, step): &(i64, u32, u32)| {
            let origin = [column as u32 * SIDE, row * SIDE];
            let patch = Patch { origin, step };
            let cells: [u32; 2] =
                std::array::from_fn(|axis| ((map[axis] * SIDE - origin[axis]) / step).min(SIDE));
            let mut rules = Vec::new();
            let differs = steps.rim(patch, cells, 0, &mut rules);
            (steps.alone(column, row, step), differs, rules)
        };
        let mut kept = 0;
        for block in &first[..6] {
            if now.same_around(&was, block.0, block.1, block.2) {
                kept += 1;
                assert_eq!(rim(&now, block), rim(&was, block), "{block:?}");
            }
        }
        assert!(kept >= 3, "the blocks away from the split keep their rims");
        assert!(!now.same_around(&was, 6, 0, 2), "a block beside the split sees it");
        assert!(!now.same_around(&was, 7, 2, 1), "a corner cell beside the split is read too");
    }

    /// Blocks of different steps give every rim sample they share one height, whatever step
    /// each draws, and blocks of the finest step keep their sampled heights.
    #[test]
    fn blocks_of_different_steps_meet_at_their_rims_and_the_finest_keep_their_heights() {
        const WIDE: [u32; 2] = [512, 256];
        let mut bytes = vec![40u8; (WIDE[0] * WIDE[1]) as usize];
        let mut put = |x: u32, z: u32, byte: u8| {
            bytes[((WIDE[1] - 1 - z) * WIDE[0] + x) as usize] = byte;
        };
        // Land across the step-4 block's rim with the step-2 blocks, its shore a pixel short of a
        // step-4 vertex row, and land reaching into the finest blocks.
        for z in 30..99 {
            for x in 200..420 {
                put(x, z, 200);
            }
        }
        for z in 150..230 {
            for x in 230..400 {
                put(x, z, 200);
            }
        }
        let coast = Coast::new(&bytes, WIDE, WATER, SCALE, 256);
        let blocks: [([u32; 2], u32); 8] = [
            ([0, 0], 4),
            ([256, 0], 2),
            ([384, 0], 2),
            ([384, 128], 2),
            ([256, 128], 1),
            ([320, 128], 1),
            ([256, 192], 1),
            ([320, 192], 1),
        ];
        let held: Vec<_> = blocks
            .iter()
            .map(|&(origin, step)| (i64::from(origin[0] / SIDE), origin[1] / SIDE, step))
            .collect();
        let steps = Steps::new(&held, WIDE.map(|side| side / SIDE));
        assert!(!steps.alone(4, 0, 2), "a block beside a coarser one");
        assert!(!steps.alone(6, 0, 2), "a block whose corner meets a finer one");
        let even = Steps::new(&[(0, 0, 2), (2, 0, 2), (0, 2, 2)], [4, 4]);
        assert!(even.alone(0, 0, 2), "a block among its own step");
        let positions = super::super::terrain_shader::positions();
        let mut shared = std::collections::BTreeMap::new();
        let (mut rims, mut rule) = (0, Vec::new());
        for &(origin, step) in &blocks {
            let patch = Patch { origin, step };
            let cells: [u32; 2] =
                std::array::from_fn(|axis| ((WIDE[axis] - origin[axis]) / step).min(SIDE));
            let mut heights = Vec::new();
            heights_into(&positions, &bytes, WIDE, patch, &mut heights);
            let sampled = heights.clone();
            coast.cap(&positions, origin, step, &mut heights);
            if !steps.alone(i64::from(origin[0] / SIDE), origin[1] / SIDE, step)
                && steps.rim(patch, cells, 0, &mut rule)
                && let Some(rimmed) = coast.rim(&steps, &positions, &bytes, WIDE, patch, cells, 0)
            {
                heights = rimmed;
                rims += 1;
            }
            if step == 1 {
                assert_eq!(heights, sampled, "the finest block at {origin:?} keeps its heights");
            }
            for (&position, pair) in positions.iter().zip(&heights) {
                for (&height, at) in pair.iter().zip(samples(position)) {
                    if !on_rim(cells, at) {
                        continue;
                    }
                    let point = world(patch, at);
                    match shared.get(&point) {
                        Some(&(first, by)) if by != origin => assert_eq!(
                            height, first,
                            "{point:?}: the block at {by:?} and the step-{step} block at {origin:?}"
                        ),
                        Some(_) => {}
                        None => {
                            shared.insert(point, (height, origin));
                        }
                    }
                }
            }
        }
        assert!(rims > 0, "a rim was matched to the coarser block beside it");
    }

    /// The game never snaps across the east-west wrap, so blocks of different steps meeting
    /// there leave the gap they always did: their vertices on the wrap keep their sampled
    /// heights, while those inside the blocks are still capped.
    #[test]
    fn blocks_meeting_across_the_wrap_keep_their_sampled_heights_on_it() {
        const WIDE: [u32; 2] = [512, 256];
        let mut bytes = vec![40u8; (WIDE[0] * WIDE[1]) as usize];
        // Land across the wrap with sea beside it, which the cells of either step hold.
        for z in 0..WIDE[1] {
            for x in (0..6).chain(506..512) {
                if z % 32 >= 8 {
                    bytes[((WIDE[1] - 1 - z) * WIDE[0] + x) as usize] = 200;
                }
            }
        }
        let coast = Coast::new(&bytes, WIDE, WATER, SCALE, 256);
        // The last two columns at step 2 on the eye's copy, the first four at step 4 on the next.
        let blocks = [(([384, 0], 2), 0), (([0, 0], 4), 1)];
        let held: Vec<_> = blocks
            .iter()
            .map(|&((origin, step), copy)| (copy * 8 + i64::from(origin[0] / SIDE), 0, step))
            .collect();
        let steps = Steps::new(&held, WIDE.map(|side| side / SIDE));
        let positions = super::super::terrain_shader::positions();
        let (mut kept, mut capped) = (0, 0);
        for &((origin, step), copy) in &blocks {
            let patch = Patch { origin, step };
            let cells: [u32; 2] =
                std::array::from_fn(|axis| ((WIDE[axis] - origin[axis]) / step).min(SIDE));
            assert!(!steps.alone(copy * 8 + i64::from(origin[0] / SIDE), 0, step));
            let mut sampled = Vec::new();
            heights_into(&positions, &bytes, WIDE, patch, &mut sampled);
            let mut drawn = sampled.clone();
            coast.cap(&positions, origin, step, &mut drawn);
            let mut rule = Vec::new();
            if steps.rim(patch, cells, copy, &mut rule)
                && let Some(rimmed) = coast.rim(&steps, &positions, &bytes, WIDE, patch, cells, copy)
            {
                drawn = rimmed;
            }
            for ((&position, pair), raw) in positions.iter().zip(&drawn).zip(&sampled) {
                for ((&height, &raw), at) in pair.iter().zip(raw).zip(samples(position)) {
                    if at[0] > cells[0] || at[1] > cells[1] {
                        continue;
                    }
                    if world(patch, at)[0] % WIDE[0] == 0 {
                        assert_eq!(height, raw, "step {step}: {at:?} on the wrap");
                        kept += usize::from(raw > 940);
                    } else {
                        capped += usize::from(height != raw);
                    }
                }
            }
        }
        assert!(kept > 0, "land on the wrap that a cap would have lowered");
        assert!(capped > 0, "the blocks are still capped off the wrap");
    }
}
