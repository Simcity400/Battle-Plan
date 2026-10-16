//! The terrain the compiled content shader draws: the approved uniform grid for the game's
//! camera, and nested blocks for a free eye.
//!
//! A free eye can see to the horizon, which no uniform grid reaches at a usable step. Its terrain
//! is the content shader's own block scheme instead: blocks of 64 by 64 cells, each at the coarsest
//! step its distance from the eye allows, with the edges that meet a coarser neighbour snapped to
//! it through the shader's snap switches. Blocks are chosen in world space across as many map
//! copies as the view reaches and culled to the view volume.

use std::collections::BTreeSet;

use viewer_foundation::map::camera::Camera;
#[cfg(test)]
use viewer_foundation::map::camera::MapDefines;
#[cfg(test)]
use viewer_foundation::map::camera::MapSize;
use viewer_foundation::map::camera::frustum::TERRAIN_TOP;

use super::terrain_shader::SIDE;
pub use super::terrain_shader::{Block, Cover};

/// Quads the visible grid may hold before the step doubles.
///
/// At the reference camera one map pixel is 5.73 screen pixels and the visible
/// ground is about 464 x 252 units, so a step of 1 costs about 140k quads with
/// the margin. The ceiling only bites when the camera climbs:
/// at `CAMERA_MAX_HEIGHT` the visible ground is roughly 4400 x 2475 units,
/// which a step of 1 would turn into 10.9M quads.
pub const MAX_QUADS: u32 = 600_000;

/// World units the visible rectangle grows by on every side.
///
/// Terrain reaches 25.5 units above the water plane, and at the shallowest
/// pitch the engine allows (53.5 degrees below the horizon at height 50) a
/// 16-unit column leans about 12 units past its own ground position, so 16
/// units of margin keeps everything that can appear inside the frame.
pub const MARGIN: f32 = 16.0;

pub use super::terrain_shader::Grid;

/// Covers `rect` with the finest power-of-two grid that stays within `max_quads`.
///
/// `rect` is `(x0, z0, x1, z1)` in world units. The `z` range is clipped to the
/// map, which has no terrain outside it; `x` is left alone because the map wraps
/// and the shaders wrap their own lookups.
pub fn grid(rect: [f32; 4], map_height: f32, max_quads: u32) -> Grid {
    let z0 = rect[1].max(-MARGIN);
    let z1 = rect[3].min(map_height - 1.0 + MARGIN).max(z0);
    let mut step = 1.0f32;
    loop {
        let origin = [(rect[0] / step).floor() * step, (z0 / step).floor() * step];
        let counts = [
            ((rect[2] - origin[0]) / step).ceil().max(1.0) as u32 + 1,
            ((z1 - origin[1]) / step).ceil().max(1.0) as u32 + 1,
        ];
        let candidate = Grid {
            origin,
            step,
            counts,
        };
        if candidate.quads() <= max_quads || step >= 256.0 {
            return candidate;
        }
        step *= 2.0;
    }
}

/// The grid for one camera and viewport.
pub fn grid_for(camera: Camera, viewport: [u32; 2]) -> Grid {
    match camera.visible_ground(viewport, MARGIN) {
        Some(rect) => grid(rect, camera.map.y(), MAX_QUADS),
        // A camera that cannot see the water plane cannot see the map either.
        None => Grid {
            origin: [0.0, 0.0],
            step: 1.0,
            counts: [0, 0],
        },
    }
}

/// The terrain one camera draws: the uniform grid for the game's camera, blocks for a free eye.
/// A free eye over a map whose sides are not whole blocks is refused: the shader's blocks cannot
/// tile it.
pub fn cover_for(camera: Camera, viewport: [u32; 2]) -> Cover {
    if camera.free().is_none() {
        return Cover::Uniform(grid_for(camera, viewport));
    }
    let map = [camera.map.width, camera.map.height];
    if map.contains(&0) || !map.iter().all(|side| side.is_multiple_of(SIDE)) {
        return Cover::Refused(UNTILED_MAP);
    }
    Cover::Blocks(blocks(camera, viewport))
}

/// The refusal for a free eye over a map the shader's 64-cell blocks cannot tile.
pub const UNTILED_MAP: &str =
    "a free camera needs the map's width and height in whole 64-pixel terrain blocks";

/// A block splits while the eye is nearer to it than this many block widths. Neighbouring blocks
/// then differ by at most one step whenever this is at least the square root of two, which the
/// snap switches need; the margin keeps the finest blocks out to about two widths.
pub const SPLIT_DISTANCE: f32 = 2.0;

/// The coarsest step [`blocks`] uses.
const ROOT_LIMIT: u32 = 256;

/// The coarsest step a free eye's blocks take: the largest power of two, at most `limit`, whose
/// blocks tile the map's width exactly (so blocks meet across the seam) and that divides its
/// height (so the last row of blocks ends on the map's edge).
pub fn root_step(map: [u32; 2], limit: u32) -> u32 {
    let mut step = 1;
    while step * 2 <= limit
        && map[0].is_multiple_of(SIDE * step * 2)
        && map[1].is_multiple_of(step * 2)
    {
        step *= 2;
    }
    step
}

/// The blocks a free eye draws, its own map copy first.
///
/// Every map copy the view volume reaches out to the far plane is tiled by blocks at
/// [`root_step`]; a block splits into four at half its step while the eye stands within
/// [`SPLIT_DISTANCE`] block widths of it, down to step one. Blocks wholly outside the view volume
/// are dropped, and each edge that meets a kept block of twice its step is snapped.
/// A map whose sides are not whole blocks has none ([`cover_for`] refuses it).
pub fn blocks(camera: Camera, viewport: [u32; 2]) -> Vec<Block> {
    let map = [camera.map.width, camera.map.height];
    if map.contains(&0) || !map.iter().all(|side| side.is_multiple_of(SIDE)) {
        return Vec::new();
    }
    let width = camera.map.x();
    let Some([x0, _, x1, _]) = camera.reach(viewport, camera.lens().far, [0.0, TERRAIN_TOP]) else {
        return Vec::new();
    };
    let planes = camera.frustum_planes(viewport);
    let eye = camera.eye();
    let root = root_step(map, ROOT_LIMIT);
    let first = (x0 / width).floor() as i32;
    let last = ((x1 / width).ceil() as i32 - 1).max(first);
    let copies = std::iter::once(0)
        .chain(viewer_foundation::map::province_admission::seam_copies(
            x0, x1, width,
        ))
        .filter(|copy| (first..=last).contains(copy));
    let mut leaves = Vec::new();
    let extent = (SIDE * root) as usize;
    for copy in copies {
        let offset = copy as f32 * width;
        for z in (0..map[1]).step_by(extent) {
            for x in (0..map[0]).step_by(extent) {
                let leaf = Leaf {
                    origin: [x, z],
                    step: root,
                    offset,
                };
                split(leaf, map, eye, &planes, &mut leaves);
            }
        }
    }
    // A key per kept block: its world x origin, z origin and step.
    let kept: BTreeSet<(i64, u32, u32)> = leaves.iter().map(Leaf::key).collect();
    leaves
        .iter()
        .map(|leaf| {
            let extent = i64::from(SIDE * leaf.step);
            let [x, z] = [leaf.world_x(), i64::from(leaf.origin[1])];
            let coarse = 2 * extent;
            let coarser = |px: i64, pz: i64| {
                pz >= 0
                    && pz < i64::from(map[1])
                    && kept.contains(&(
                        px.div_euclid(coarse) * coarse,
                        (pz.div_euclid(coarse) * coarse) as u32,
                        leaf.step * 2,
                    ))
            };
            let middle = extent / 2;
            Block {
                origin: leaf.origin,
                step: leaf.step,
                offset: leaf.offset,
                snap: [
                    coarser(x + middle, z - 1),
                    coarser(x + middle, z + extent),
                    coarser(x - 1, z + middle),
                    coarser(x + extent, z + middle),
                ],
            }
        })
        .collect()
}

/// A candidate block in world space.
#[derive(Clone, Copy)]
struct Leaf {
    origin: [u32; 2],
    step: u32,
    offset: f32,
}

impl Leaf {
    fn world_x(&self) -> i64 {
        i64::from(self.origin[0]) + self.offset as i64
    }

    fn key(&self) -> (i64, u32, u32) {
        (self.world_x(), self.origin[1], self.step)
    }

    /// The block's world box, cut at the map's edges, standing from the ground to the highest
    /// terrain.
    fn bounds(&self, map: [u32; 2]) -> [[f32; 3]; 2] {
        let extent = SIDE * self.step;
        let end = [
            (self.origin[0] + extent).min(map[0]),
            (self.origin[1] + extent).min(map[1]),
        ];
        [
            [
                self.origin[0] as f32 + self.offset,
                0.0,
                self.origin[1] as f32,
            ],
            [end[0] as f32 + self.offset, TERRAIN_TOP, end[1] as f32],
        ]
    }
}

fn split(leaf: Leaf, map: [u32; 2], eye: [f32; 3], planes: &[[f32; 4]; 6], out: &mut Vec<Leaf>) {
    let bounds = leaf.bounds(map);
    if planes.iter().any(|plane| outside(*plane, bounds)) {
        return;
    }
    let extent = (SIDE * leaf.step) as f32;
    if leaf.step > 1 && distance(eye, bounds) < SPLIT_DISTANCE * extent {
        let step = leaf.step / 2;
        let half = SIDE * step;
        for [dx, dz] in [[0, 0], [half, 0], [0, half], [half, half]] {
            let origin = [leaf.origin[0] + dx, leaf.origin[1] + dz];
            if origin[0] < map[0] && origin[1] < map[1] {
                let child = Leaf {
                    origin,
                    step,
                    offset: leaf.offset,
                };
                split(child, map, eye, planes, out);
            }
        }
    } else {
        out.push(leaf);
    }
}

/// Whether a box lies wholly outside a view plane (inside non-negative).
fn outside([a, b, c, d]: [f32; 4], [min, max]: [[f32; 3]; 2]) -> bool {
    let x = if a > 0.0 { max[0] } else { min[0] };
    let y = if b > 0.0 { max[1] } else { min[1] };
    let z = if c > 0.0 { max[2] } else { min[2] };
    a * x + b * y + c * z + d < 0.0
}

/// The distance from a point to a box.
fn distance(point: [f32; 3], [min, max]: [[f32; 3]; 2]) -> f32 {
    let gap: [f32; 3] = std::array::from_fn(|axis| {
        (min[axis] - point[axis])
            .max(point[axis] - max[axis])
            .max(0.0)
    });
    (gap[0] * gap[0] + gap[1] * gap[1] + gap[2] * gap[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f32 = 2048.0;

    /// World `(x, z)` of the last vertex the grid places.
    fn far_corner(grid: Grid) -> [f32; 2] {
        [
            grid.origin[0] + (grid.counts[0] - 1) as f32 * grid.step,
            grid.origin[1] + (grid.counts[1] - 1) as f32 * grid.step,
        ]
    }

    #[test]
    fn the_grid_covers_the_rectangle_at_step_one_when_it_is_small() {
        let grid = grid([100.4, 200.9, 140.0, 220.0], H, MAX_QUADS);
        assert_eq!(grid.step, 1.0);
        assert_eq!(grid.origin, [100.0, 200.0]);
        // 100..140 needs 41 columns, 200..220 needs 21 rows.
        assert_eq!(grid.counts, [41, 21]);
        let far = far_corner(grid);
        assert!(far[0] >= 140.0 && far[1] >= 220.0);
        assert_eq!(grid.quads(), 40 * 20);
    }

    #[test]
    fn the_step_doubles_until_the_quad_budget_holds() {
        // The full-map view at CAMERA_MAX_HEIGHT: about 4400 x 2475 units,
        // of which only the map's own 2048 rows plus the margin survive.
        let rect = [0.0, 0.0, 4400.0, 2475.0];
        let coarse = grid(rect, H, MAX_QUADS);
        assert!(coarse.quads() <= MAX_QUADS, "{coarse:?}");
        // Step 2 would be 2.27M quads and step 4 is 567_600, just inside.
        assert_eq!(coarse.step, 4.0);
        assert_eq!(coarse.quads(), 1100 * 516);
        // A four-times budget admits step 2, which is what makes the rule a
        // doubling against the budget and not a fixed choice.
        assert_eq!(grid(rect, H, MAX_QUADS * 4).step, 2.0);
        // The step never runs away, even on an impossible budget.
        assert_eq!(grid(rect, H, 1).step, 256.0);
    }

    #[test]
    fn the_origin_stays_on_a_multiple_of_the_step() {
        let grid = grid([-13.0, 7.0, 4400.0, 2475.0], H, MAX_QUADS);
        assert_eq!(grid.origin[0] % grid.step, 0.0);
        assert_eq!(grid.origin[1] % grid.step, 0.0);
        assert!(grid.origin[0] <= -13.0 && grid.origin[1] <= 7.0);
    }

    #[test]
    fn the_grid_is_clipped_to_the_map_in_z_but_not_in_x() {
        let grid = grid([-400.0, -900.0, 200.0, 9000.0], H, MAX_QUADS);
        assert!(grid.origin[0] < 0.0, "x is never clipped: the map wraps");
        assert!(grid.origin[1] >= -MARGIN - grid.step);
        let top = far_corner(grid)[1];
        assert!(top <= H - 1.0 + MARGIN + grid.step, "{top}");
    }

    #[test]
    fn the_reference_camera_grid_is_about_a_hundred_thousand_quads() {
        let camera = Camera {
            center: [3010.0, 1568.0],
            height: 310.178,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        };
        let grid = grid_for(camera, [2048, 1152]);
        assert_eq!(grid.step, 1.0);
        assert!(
            (100_000..250_000).contains(&grid.quads()),
            "{} quads",
            grid.quads()
        );
    }

    use viewer_foundation::map::camera::{Free, Lens, Pose};

    const VIEWPORT: [u32; 2] = [1600, 900];

    /// A free eye over the test map with a lens reaching `far`.
    fn free(eye: [f32; 3], yaw: f32, pitch: f32, far: f32) -> Camera {
        Camera {
            center: [eye[0], eye[2]],
            height: eye[1],
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Free(Free::new(
                yaw,
                pitch,
                Lens {
                    near: 1.0,
                    far,
                    ..Lens::MAP
                },
            )),
        }
    }

    /// A block's world box in `x` and `z`, cut at the map's edges.
    fn world(block: &Block) -> [f32; 4] {
        let extent = (SIDE * block.step) as f32;
        let map = MapSize::TEST;
        [
            block.origin[0] as f32 + block.offset,
            block.origin[1] as f32,
            (block.origin[0] as f32 + extent).min(map.x()) + block.offset,
            (block.origin[1] as f32 + extent).min(map.y()),
        ]
    }

    fn containing(blocks: &[Block], [x, z]: [f32; 2]) -> Vec<&Block> {
        blocks
            .iter()
            .filter(|block| {
                let [x0, z0, x1, z1] = world(block);
                x0 <= x && x < x1 && z0 <= z && z < z1
            })
            .collect()
    }

    fn cameras() -> Vec<Camera> {
        let width = MapSize::TEST.x();
        vec![
            free([3000.0, 30.0, 1000.0], 0.0, 0.0, 20_000.0),
            free([3000.0, 300.0, 1000.0], 0.8, -0.35, 20_000.0),
            free([width - 40.0, 25.0, 900.0], 1.6, -0.05, 20_000.0),
            free([20.0, 12.0, 30.0], 3.4, -0.2, 9_000.0),
            free([1200.0, 800.0, 1800.0], -2.0, -1.2, 6_000.0),
        ]
    }

    /// Neighbouring blocks differ by at most one step, every edge meeting a coarser block is
    /// snapped and no other edge is.
    #[test]
    fn free_blocks_are_balanced_and_snap_to_coarser_neighbours() {
        for camera in cameras() {
            let blocks = blocks(camera, VIEWPORT);
            assert!(!blocks.is_empty(), "{camera:?}");
            for block in &blocks {
                let [x0, z0, x1, z1] = world(block);
                let middle = [(x0 + x1) * 0.5, (z0 + z1) * 0.5];
                for (edge, point) in [
                    [middle[0], z0 - 0.5],
                    [middle[0], z1 + 0.5],
                    [x0 - 0.5, middle[1]],
                    [x1 + 0.5, middle[1]],
                ]
                .into_iter()
                .enumerate()
                {
                    let neighbours = containing(&blocks, point);
                    assert!(neighbours.len() <= 1, "{neighbours:?}");
                    let coarser = neighbours.first().is_some_and(|neighbour| {
                        assert!(
                            neighbour.step <= 2 * block.step && block.step <= 2 * neighbour.step,
                            "{block:?} meets {neighbour:?}"
                        );
                        neighbour.step == 2 * block.step
                    });
                    // A cut edge at the map's top or bottom has no neighbour to meet.
                    if (edge == 1 && (z1 - z0) < (SIDE * block.step) as f32)
                        || (edge == 3 && (x1 - x0) < (SIDE * block.step) as f32)
                    {
                        continue;
                    }
                    assert_eq!(block.snap[edge], coarser, "{block:?} edge {edge}");
                }
            }
        }
    }

    /// Ground the view sees near the eye is covered by exactly one block, the finest step, and
    /// ground far ahead by a coarse one; nothing behind the eye is drawn.
    #[test]
    fn free_blocks_cover_the_view_once_and_coarsen_with_distance() {
        let camera = free([3000.0, 30.0, 1000.0], 0.0, 0.0, 20_000.0);
        let blocks = blocks(camera, VIEWPORT);
        for z in [1010.0, 1100.0, 1500.0, 1990.0] {
            for x in [2990.0, 3000.0, 3010.0] {
                assert_eq!(containing(&blocks, [x, z]).len(), 1, "{x} {z}");
            }
        }
        assert_eq!(containing(&blocks, [3000.0, 1010.0])[0].step, 1);
        assert!(containing(&blocks, [3000.0, 1990.0])[0].step >= 4);
        assert!(containing(&blocks, [3000.0, 500.0]).is_empty());
        // Ahead of the eye the map ends at its top edge; no block stands past it.
        assert!(
            blocks
                .iter()
                .all(|block| world(block)[3] <= MapSize::TEST.y())
        );
        // A level look along the map's width reaches several copies, coarsely.
        let east = free(
            [3000.0, 30.0, 1000.0],
            std::f32::consts::FRAC_PI_2,
            0.0,
            20_000.0,
        );
        let blocks = super::blocks(east, VIEWPORT);
        let mut offsets: Vec<_> = blocks.iter().map(|block| block.offset as i64).collect();
        offsets.dedup();
        assert!(offsets.len() >= 3, "{offsets:?}");
        let quads: u64 = Cover::Blocks(blocks).quads();
        assert!(quads < 4_000_000, "{quads} quads");
    }

    #[test]
    fn the_root_step_tiles_the_width_and_divides_the_height() {
        assert_eq!(root_step([5632, 2048], 256), 8);
        assert_eq!(root_step([5632, 2048], 4), 4);
        assert_eq!(root_step([64 * 7, 2048], 256), 1);
        assert_eq!(root_step([8192, 4096], 256), 128);
        assert_eq!(root_step([8192, 96], 256), 32);
    }

    #[test]
    fn the_game_camera_keeps_its_uniform_grid() {
        let camera = Camera {
            center: [3010.0, 1568.0],
            height: 310.178,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        };
        assert_eq!(
            cover_for(camera, [2048, 1152]),
            Cover::Uniform(grid_for(camera, [2048, 1152]))
        );
    }

    #[test]
    fn a_free_eye_over_a_map_of_partial_blocks_is_refused_by_name() {
        let mut camera = free([300.0, 40.0, 300.0], 0.0, -0.2, 2_000.0);
        camera.map = MapSize {
            width: 64 * 10 + 8,
            height: 640,
        };
        assert_eq!(cover_for(camera, VIEWPORT), Cover::Refused(UNTILED_MAP));
        assert_eq!(Cover::Refused(UNTILED_MAP).quads(), 0);
    }
}
