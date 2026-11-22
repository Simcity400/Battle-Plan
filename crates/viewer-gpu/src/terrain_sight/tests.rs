use super::*;

const SEA: u8 = 95;

struct Raster {
    width: u32,
    rows: u32,
    values: Vec<u8>,
}

impl Raster {
    fn flat(width: u32, rows: u32, byte: u8) -> Self {
        Self {
            width,
            rows,
            values: vec![byte; (width * rows) as usize],
        }
    }

    fn set(&mut self, column: u32, bottom: u32, byte: u8) {
        let row = (self.rows - 1 - bottom) as usize;
        self.values[row * self.width as usize + column as usize] = byte;
    }

    fn field(&self) -> HeightField<'_> {
        HeightField {
            width: self.width,
            height: self.rows,
            values: &self.values,
        }
    }
}

fn brute(field: &HeightField<'_>, eye: [f32; 3], target: [f32; 3], skip: f32, band: f32) -> f32 {
    let width = field.width as f32;
    let mut dx = target[0] - eye[0];
    dx -= width * (dx / width).round();
    let dz = target[2] - eye[2];
    let length = dx.hypot(dz);
    let end = length - skip;
    let mut least = band;
    let steps = (end / 0.01).ceil() as usize;
    for step in 0..=steps {
        let u = (step as f32 * 0.01).min(end);
        let y = eye[1] + (target[1] - eye[1]) * u / length;
        let x = eye[0] + dx * u / length;
        let z = eye[2] + dz * u / length;
        least = least.min(y - field.surface_height(x, z));
    }
    least.clamp(-band, band)
}

#[test]
fn an_open_sea_hides_nothing() {
    let raster = Raster::flat(128, 32, SEA);
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    assert_eq!(sight.ceiling(), WATER_HEIGHT);
    let clear = sight.clearance(&field, [2.0, 10.5, 16.0], [60.0, 9.6, 16.0], 1.0, 0.5);
    assert!(clear > 0.0, "{clear}");
    let high = sight.clearance(&field, [2.0, 30.0, 16.0], [60.0, 12.0, 16.0], 1.0, 0.5);
    assert_eq!(high, 0.5);
}

#[test]
fn a_ridge_between_the_eye_and_the_anchor_hides_it() {
    let mut raster = Raster::flat(128, 32, SEA);
    for bottom in 0..32 {
        raster.set(30, bottom, 200);
        raster.set(31, bottom, 200);
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    assert_eq!(sight.ceiling(), 20.0);
    let eye = [10.0, 12.0, 16.0];
    let anchor = [50.0, 9.5, 16.0];
    assert_eq!(sight.clearance(&field, eye, anchor, 1.0, 0.5), -0.5);
    let high = [10.0, 200.0, 16.0];
    assert!(sight.clearance(&field, high, anchor, 1.0, 0.5) > 0.0);
    let grazing = sight.clearance(&field, [10.0, 30.0, 16.0], anchor, 1.0, 5.0);
    let dense = brute(&field, [10.0, 30.0, 16.0], anchor, 1.0, 5.0);
    assert!(grazing < 0.0 && grazing > -5.0, "{grazing}");
    assert!((grazing - dense).abs() < 0.05, "{grazing} against {dense}");
}

#[test]
fn a_ridge_beside_the_line_hides_nothing() {
    let mut raster = Raster::flat(128, 32, SEA);
    for column in 20..40 {
        raster.set(column, 2, 250);
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    let clear = sight.clearance(&field, [10.0, 11.0, 20.0], [50.0, 9.5, 20.0], 1.0, 0.5);
    assert!(clear > 0.0, "{clear}");
}

#[test]
fn the_anchor_s_own_ground_is_skipped() {
    let mut raster = Raster::flat(128, 32, SEA);
    for column in 44..56 {
        for bottom in 10..22 {
            raster.set(column, bottom, 150);
        }
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    let anchor = [50.0, 15.0, 16.0];
    let eye = [10.0, 14.0, 16.0];
    assert!(sight.clearance(&field, eye, anchor, 0.0, 0.5) < 0.0);
    assert!(sight.clearance(&field, eye, anchor, 7.0, 0.5) > 0.0);
}

#[test]
fn the_line_wraps_across_the_map_seam() {
    let mut raster = Raster::flat(64, 32, SEA);
    for bottom in 0..32 {
        raster.set(0, bottom, 220);
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    let eye = [58.0, 11.0, 16.0];
    for x in [4.0, 68.0, -60.0] {
        assert_eq!(
            sight.clearance(&field, eye, [x, 9.5, 16.0], 1.0, 0.5),
            -0.5,
            "anchor at x {x}"
        );
    }
    let open = Raster::flat(64, 32, SEA);
    let open_field = open.field();
    let open_sight = TerrainSight::new(&open_field);
    assert!(open_sight.clearance(&open_field, eye, [4.0, 9.5, 16.0], 1.0, 0.5) > 0.0);
}

#[test]
fn a_shortcut_across_the_seam_still_meets_a_ridge_before_it() {
    let mut raster = Raster::flat(192, 64, SEA);
    for bottom in 0..64 {
        raster.set(160, bottom, 200);
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    assert_eq!(
        sight.clearance(&field, [120.0, 12.0, 32.0], [18.0, 12.0, 32.0], 1.0, 0.5),
        -0.5
    );
    for width in [160, 200, 250, 300] {
        let mut raster = Raster::flat(width, 32, SEA);
        for bottom in 0..32 {
            raster.set(width - 10, bottom, 200);
        }
        let field = raster.field();
        let sight = TerrainSight::new(&field);
        let eye = [(width - 60) as f32, 12.0, 16.0];
        assert_eq!(
            sight.clearance(&field, eye, [15.0, 12.0, 16.0], 1.0, 0.5),
            -0.5,
            "width {width}"
        );
        assert_eq!(
            sight.clearance(&field, eye, [15.0, 12.0, 16.0], 1.0, 0.5),
            brute(&field, eye, [15.0, 12.0, 16.0], 1.0, 0.5),
            "width {width}"
        );
    }
}

#[test]
fn a_line_rising_away_from_the_ground_is_clear() {
    let mut raster = Raster::flat(128, 32, SEA);
    raster.set(30, 16, 255);
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    assert_eq!(
        sight.clearance(&field, [10.0, 30.0, 16.0], [50.0, 40.0, 16.0], 0.0, 0.5),
        0.5
    );
}

#[test]
fn an_empty_or_short_field_hides_nothing() {
    let field = HeightField {
        width: 4,
        height: 4,
        values: &[],
    };
    let sight = TerrainSight::new(&field);
    assert_eq!(
        sight.clearance(&field, [0.0, 1.0, 0.0], [3.0, 0.0, 3.0], 0.0, 2.0),
        2.0
    );
}

#[test]
fn the_block_walk_matches_a_dense_walk_over_smooth_hills() {
    let mut state = 0x2545_f491_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let (width, rows) = (96u32, 64u32);
    for case in 0..24 {
        let bumps: Vec<[f32; 4]> = (0..6)
            .map(|_| {
                [
                    (next() % width) as f32,
                    (next() % rows) as f32,
                    8.0 + (next() % 12) as f32,
                    20.0 + (next() % 60) as f32,
                ]
            })
            .collect();
        let mut raster = Raster::flat(width, rows, SEA);
        for bottom in 0..rows {
            for column in 0..width {
                let mut lift = 0.0f32;
                for [x, z, radius, peak] in &bumps {
                    let mut dx = column as f32 - x;
                    dx -= width as f32 * (dx / width as f32).round();
                    let d = dx.hypot(bottom as f32 - z) / radius;
                    lift = lift.max(peak * (1.0 - d * d).max(0.0));
                }
                raster.set(column, bottom, (80.0 + lift).min(255.0) as u8);
            }
        }
        let field = raster.field();
        let sight = TerrainSight::new(&field);
        let eye = [
            (next() % width) as f32 + 0.25,
            10.0 + (next() % 30) as f32,
            (next() % rows) as f32 + 0.5,
        ];
        let target = [
            (next() % width) as f32 + 0.75,
            9.5 + (next() % 8) as f32,
            (next() % rows) as f32 + 0.25,
        ];
        let band = 1.5;
        let walked = sight.clearance(&field, eye, target, 1.0, band);
        let dense = brute(&field, eye, target, 1.0, band);
        assert!(
            (walked - dense).abs() <= 0.35,
            "case {case}: block walk {walked}, dense walk {dense} from {eye:?} to {target:?}"
        );
    }
}

#[test]
fn a_westward_line_far_along_its_parameter_still_moves_on() {
    assert_eq!(next_break([3000.0, 0.0], [-1.0, 0.0]), 1.0);
    assert!(past(2048.0, 2048.0 + BREAK_EPSILON) > 2048.0);
    assert_eq!(past(1.0, 2.0), 2.0);
}

#[test]
fn a_crest_between_samples_still_hides_the_anchor() {
    let mut raster = Raster::flat(64, 32, SEA);
    for bottom in 0..32 {
        raster.set(5, bottom, 200);
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    let clearance = sight.clearance(&field, [0.25, 19.5, 16.2], [10.25, 19.5, 16.2], 0.5, 1.0);
    assert!(clearance < 0.0, "{clearance}");
    assert!((clearance - -0.5).abs() < 1e-3, "{clearance}");
    let diagonal = sight.clearance(&field, [0.3, 19.5, 12.1], [9.3, 19.5, 21.1], 0.5, 1.0);
    assert!((diagonal - -0.5).abs() < 1e-3, "{diagonal}");
}

#[test]
fn the_walk_finds_every_plane_break_of_the_drawn_surface() {
    let mut state = 0x9e37_79b9_u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let (width, rows) = (48u32, 32u32);
    let mut raster = Raster::flat(width, rows, SEA);
    for bottom in 0..rows {
        for column in 0..width {
            raster.set(column, bottom, (80 + next() % 140) as u8);
        }
    }
    let field = raster.field();
    let sight = TerrainSight::new(&field);
    for case in 0..40 {
        let mut at = || {
            [
                (next() % (width * 100)) as f32 / 100.0,
                (next() % ((rows - 1) * 100)) as f32 / 100.0,
            ]
        };
        let [ex, ez] = at();
        let [tx, tz] = at();
        let eye = [ex, 10.0 + (next() % 200) as f32 / 10.0, ez];
        let target = [tx, 9.5 + (next() % 120) as f32 / 10.0, tz];
        let band = 30.0;
        let walked = sight.clearance(&field, eye, target, 0.0, band);
        let dense = brute(&field, eye, target, 0.0, band);
        assert!(
            walked <= dense + 1e-3,
            "case {case}: walk {walked} above the dense walk's {dense} from {eye:?} to {target:?}"
        );
    }
}
