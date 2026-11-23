
use viewer_gpu::camera::Camera;
#[cfg(test)]
use viewer_gpu::camera::MapDefines;
#[cfg(test)]
use viewer_gpu::camera::MapSize;
use viewer_gpu::camera::frustum::{OBJECT_TOP, TERRAIN_TOP};

use viewer_scene_base::line_lod::MARGIN;
use super::terrain_shader::SIDE;
use super::terrain_shader::{Block, Cover};

const LOD_DISTANCES: [f32; 4] = [150.0, 350.0, 750.0, 1600.0];

pub const UNTILED_MAP: &str =
    "the terrain needs the map's width and height in whole 64-pixel terrain blocks";

pub fn cover_for(camera: Camera, viewport: [u32; 2]) -> Cover {
    let map = [camera.map.width, camera.map.height];
    if map.contains(&0) || !map.iter().all(|side| side.is_multiple_of(SIDE)) {
        return Cover::Refused(UNTILED_MAP);
    }
    Cover::Blocks(blocks(camera, viewport))
}

pub fn wrap_copies(camera: Camera, viewport: [u32; 2]) -> Vec<f32> {
    let Some(free) = camera.free() else {
        return viewer_scene_base::line_lod::wrap_offsets(
            camera.visible_ground(viewport, MARGIN),
            camera.map.x(),
        );
    };
    let map = [camera.map.x(), camera.map.y()];
    let planes = camera.frustum_planes(viewport);
    let copies: Vec<f32> = viewer_scene_base::line_lod::free_wrap_offsets(
        camera.reach(viewport, free.lens.far, [0.0, OBJECT_TOP]),
        camera.map.x(),
    )
    .into_iter()
    .filter(|offset| in_view(*offset, map, &planes))
    .collect();
    if copies.is_empty() { vec![0.0] } else { copies }
}

fn in_view(offset: f32, map: [f32; 2], planes: &[[f32; 4]; 6]) -> bool {
    let bounds = [
        [offset, 0.0, 0.0],
        [map[0] + offset, OBJECT_TOP, map[1]],
    ];
    !planes.iter().any(|plane| outside(*plane, bounds))
}

pub fn blocks(camera: Camera, viewport: [u32; 2]) -> Vec<Block> {
    let map = [camera.map.width, camera.map.height];
    let (columns, rows) = ((map[0] / SIDE) as usize, (map[1] / SIDE) as usize);
    if columns == 0 || rows == 0 {
        return Vec::new();
    }
    let copies = wrap_copies(camera, viewport);
    let planes = camera.frustum_planes(viewport);
    let steps = steps([columns, rows], camera.eye(), &copies);
    let level = |row: usize, column: usize| steps[row * columns + column];
    let mut out = Vec::new();
    for offset in copies {
        for row in 0..rows {
            for column in 0..columns {
                let at = level(row, column);
                let span = 1usize << at;
                if !row.is_multiple_of(span) || !column.is_multiple_of(span) {
                    continue;
                }
                let origin = [(column as u32) * SIDE, (row as u32) * SIDE];
                let step = span as u32;
                if outside_view(origin, step, offset, map, &planes) {
                    continue;
                }
                out.push(Block {
                    origin,
                    step,
                    offset,
                    snap: [
                        row >= span && level(row - span, column) > at,
                        row + span + 1 < rows && level(row + span, column) > at,
                        column >= span && level(row, column - span) > at,
                        column + span + 1 < columns && level(row, column + span) > at,
                    ],
                });
            }
        }
    }
    out
}

fn steps(blocks: [usize; 2], eye: [f32; 3], copies: &[f32]) -> Vec<u8> {
    let [columns, rows] = blocks;
    let side = SIDE as f32;
    let high = TERRAIN_TOP - eye[1];
    let nearest_square = |edges: [f32; 2], eye: f32| {
        edges
            .into_iter()
            .map(|edge| (edge - eye) * (edge - eye))
            .fold(f32::MAX, f32::min)
    };
    let across: Vec<f32> = (0..columns)
        .map(|column| {
            let x = column as f32 * side;
            copies
                .iter()
                .map(|offset| nearest_square([x + offset, x + side + offset], eye[0]))
                .fold(f32::MAX, f32::min)
        })
        .collect();
    let along: Vec<f32> = (0..rows)
        .map(|row| {
            let z = row as f32 * side;
            nearest_square([z, z + side], eye[2])
        })
        .collect();
    let mut nearest = Vec::with_capacity(columns * rows);
    for row in &along {
        nearest.extend(across.iter().map(|across| across + row + high * high));
    }
    let mut pyramid: Vec<(usize, Vec<f32>)> = vec![(columns, nearest)];
    for level in 1..=LOD_DISTANCES.len() {
        let (below_columns, below) = &pyramid[level - 1];
        let below_rows = below.len() / below_columns;
        let (wide, tall) = (below_columns.div_ceil(2), below_rows.div_ceil(2));
        let mut least = vec![f32::MAX; wide * tall];
        for row in 0..below_rows {
            for column in 0..*below_columns {
                let at = (row / 2) * wide + column / 2;
                least[at] = least[at].min(below[row * below_columns + column]);
            }
        }
        pyramid.push((wide, least));
    }
    (0..rows * columns)
        .map(|at| {
            let (row, column) = (at / columns, at % columns);
            for (level, distance) in LOD_DISTANCES.iter().enumerate().rev() {
                let (wide, least) = &pyramid[level + 1];
                if least[(row >> (level + 1)) * wide + (column >> (level + 1))]
                    > distance * distance
                {
                    return level as u8 + 1;
                }
            }
            0
        })
        .collect()
}

fn outside_view(origin: [u32; 2], step: u32, offset: f32, map: [u32; 2], planes: &[[f32; 4]; 6]) -> bool {
    let extent = SIDE * step;
    let end = [
        (origin[0] + extent).min(map[0]),
        (origin[1] + extent).min(map[1]),
    ];
    let bounds = [
        [origin[0] as f32 + offset, 0.0, origin[1] as f32],
        [end[0] as f32 + offset, TERRAIN_TOP, end[1] as f32],
    ];
    planes.iter().any(|plane| outside(*plane, bounds))
}

fn outside([a, b, c, d]: [f32; 4], [min, max]: [[f32; 3]; 2]) -> bool {
    let x = if a > 0.0 { max[0] } else { min[0] };
    let y = if b > 0.0 { max[1] } else { min[1] };
    let z = if c > 0.0 { max[2] } else { min[2] };
    a * x + b * y + c * z + d < 0.0
}
#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::camera::{Free, Lens, Pose};

    const VIEWPORT: [u32; 2] = [1600, 900];

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

    fn map_camera(center: [f32; 2], height: f32) -> Camera {
        Camera {
            center,
            height,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        }
    }

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
            map_camera([3010.0, 1568.0], 310.178),
            map_camera([2816.0, 1024.0], 3000.0),
        ]
    }

    #[test]
    fn a_free_eye_draws_only_the_copies_its_view_volume_meets() {
        let width = MapSize::TEST.x();
        let north = free([width * 0.5, 12.0, 400.0], 0.0, -0.25, 20_000.0);
        assert_eq!(wrap_copies(north, VIEWPORT), [0.0]);
        assert_eq!(wrap_copies(free([width * 0.5, 12.0, 1600.0], std::f32::consts::PI, -0.25, 20_000.0), VIEWPORT), [0.0]);
        let seam = free([width - 5.0, 12.0, 1000.0], std::f32::consts::FRAC_PI_2, -0.25, 20_000.0);
        let copies = wrap_copies(seam, VIEWPORT);
        assert!(copies.contains(&0.0) && copies.contains(&width), "{copies:?}");
        let sky = free([width * 0.5, 12.0, 1000.0], 0.0, 1.5, 20_000.0);
        assert_eq!(wrap_copies(sky, VIEWPORT), [0.0]);
        for camera in cameras() {
            let copies = wrap_copies(camera, VIEWPORT);
            for block in blocks(camera, VIEWPORT) {
                assert!(copies.contains(&block.offset), "{block:?} of {copies:?}");
            }
        }
    }

    #[test]
    fn blocks_tile_the_view_once_on_their_own_step_grid() {
        for camera in cameras() {
            let blocks = blocks(camera, VIEWPORT);
            assert!(!blocks.is_empty(), "{camera:?}");
            for block in &blocks {
                assert!(
                    block.step.is_power_of_two() && block.step <= 1 << LOD_DISTANCES.len(),
                    "{block:?}"
                );
                assert!(
                    block
                        .origin
                        .iter()
                        .all(|origin| origin.is_multiple_of(SIDE * block.step)),
                    "{block:?}"
                );
            }
            for block in &blocks {
                let [x0, z0, x1, z1] = world(block);
                let middle = [(x0 + x1) * 0.5, (z0 + z1) * 0.5];
                assert_eq!(containing(&blocks, middle).len(), 1, "{block:?}");
            }
        }
    }

    #[test]
    fn the_step_follows_the_games_distances() {
        let blocks = [MapSize::TEST.width as usize / 64, MapSize::TEST.height as usize / 64];
        let steps = steps(blocks, [0.0, TERRAIN_TOP, 0.0], &[0.0]);
        assert_eq!(steps[0], 0, "the block under the eye draws every cell");
        let at = |column: usize, row: usize| steps[row * blocks[0] + column];
        assert_eq!(at(blocks[0] / 2, blocks[1] / 2), LOD_DISTANCES.len() as u8);
        for row in 0..16.min(blocks[1]) {
            for column in 0..16 {
                assert!(at(column, row) < LOD_DISTANCES.len() as u8, "{column} {row}");
            }
        }
        let mut coarsest = 0;
        for group in 0..blocks[0] / 16 {
            let step = at(group * 16, 0);
            assert!(step >= coarsest, "{group}: {step} after {coarsest}");
            coarsest = step;
        }
    }

    #[test]
    fn the_whole_map_view_draws_the_games_coarsest_step() {
        let camera = map_camera([2816.0, 1024.0], 3000.0);
        let drawn = blocks(camera, [2560, 1440]);
        assert!(
            drawn.iter().all(|block| block.step == 1 << LOD_DISTANCES.len()),
            "{drawn:?}"
        );
        let quads = Cover::Blocks(drawn).quads();
        assert!((20_000..80_000).contains(&quads), "{quads} quads");
    }

    #[test]
    fn the_reference_camera_draws_its_nearest_ground_at_step_two() {
        let camera = map_camera([3010.0, 1568.0], 310.178);
        let drawn = blocks(camera, [2048, 1152]);
        assert_eq!(drawn.iter().map(|block| block.step).min(), Some(2));
        let quads = Cover::Blocks(drawn).quads();
        assert!((20_000..200_000).contains(&quads), "{quads} quads");
        let close = map_camera([3010.0, 1568.0], 60.0);
        assert_eq!(
            blocks(close, [2048, 1152])
                .iter()
                .map(|block| block.step)
                .min(),
            Some(1)
        );
    }

    #[test]
    fn an_edge_that_meets_a_coarser_block_is_snapped() {
        for camera in cameras() {
            let blocks = blocks(camera, VIEWPORT);
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
                    let Some(neighbour) = containing(&blocks, point).first().copied() else {
                        continue;
                    };
                    if neighbour.offset != block.offset {
                        continue;
                    }
                    assert_eq!(
                        block.snap[edge],
                        neighbour.step > block.step,
                        "{block:?} edge {edge} meets {neighbour:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_level_look_reaches_several_map_copies_coarsely() {
        let east = free(
            [3000.0, 30.0, 1000.0],
            std::f32::consts::FRAC_PI_2,
            0.0,
            20_000.0,
        );
        let blocks = blocks(east, VIEWPORT);
        let mut offsets: Vec<_> = blocks.iter().map(|block| block.offset as i64).collect();
        offsets.dedup();
        assert!(offsets.len() >= 3, "{offsets:?}");
        let quads: u64 = Cover::Blocks(blocks).quads();
        assert!(quads < 4_000_000, "{quads} quads");
    }

    #[test]
    fn the_view_volume_bounds_the_blocks() {
        let camera = free([3000.0, 30.0, 1000.0], 0.0, 0.0, 20_000.0);
        let blocks = blocks(camera, VIEWPORT);
        assert!(containing(&blocks, [3000.0, 500.0]).is_empty());
        assert!(
            blocks
                .iter()
                .all(|block| world(block)[3] <= MapSize::TEST.y())
        );
    }

    #[test]
    fn a_map_of_partial_blocks_is_refused_by_name() {
        let mut camera = free([300.0, 40.0, 300.0], 0.0, -0.2, 2_000.0);
        camera.map = MapSize {
            width: 64 * 10 + 8,
            height: 640,
        };
        assert_eq!(cover_for(camera, VIEWPORT), Cover::Refused(UNTILED_MAP));
        assert_eq!(Cover::Refused(UNTILED_MAP).quads(), 0);
    }
}
