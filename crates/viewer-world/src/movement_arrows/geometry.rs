
use viewer_gpu::{province_map::HeightField, ribbon_geometry::smooth_sharp_turns};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct Strip {
    pub vertices: Vec<Vertex>,
    pub bounds: [[f32; 3]; 2],
    pub length: f32,
}

const SUBDIVISIONS: usize = 20;

pub fn build(controls: &[[f32; 3]], size: f32, heights: HeightField<'_>) -> Result<Strip, String> {
    if controls.len() < 4 {
        return Ok(Strip::default());
    }
    let width = size * 2.0;
    if !size.is_finite() || size <= 0.0 || !width.is_finite() {
        return Err("movement arrow size must be finite and positive".into());
    }
    if controls.iter().flatten().any(|value| !value.is_finite()) {
        return Err("movement arrow controls contain a non-finite coordinate".into());
    }
    if heights.width == 0
        || heights.height == 0
        || u64::from(heights.width) * u64::from(heights.height) != heights.values.len() as u64
    {
        return Err("movement arrow heightmap has invalid dimensions".into());
    }
    let mut controls = controls.to_vec();
    smooth_sharp_turns(&mut controls, size);
    if controls.iter().flatten().any(|value| !value.is_finite()) {
        return Err("movement arrow smoothing produced a non-finite coordinate".into());
    }
    let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
    for point in &controls {
        for axis in 0..3 {
            bounds[0][axis] = bounds[0][axis].min(point[axis]);
            bounds[1][axis] = bounds[1][axis].max(point[axis]);
        }
    }
    let count = (controls.len() - 3)
        .checked_mul(SUBDIVISIONS * 2)
        .and_then(|count| count.checked_add(2))
        .ok_or("movement arrow vertex count overflow")?;
    let mut vertices = Vec::with_capacity(count);
    let mut arc = 0.0;
    let mut previous_side = [0.0; 3];
    let increment = 1.0 / SUBDIVISIONS as f32;
    for (segment, window) in controls.windows(4).enumerate() {
        let terminal = segment + 4 == controls.len();
        for step in 0..SUBDIVISIONS + usize::from(terminal) {
            let t = step as f32 / SUBDIVISIONS as f32;
            let last = step == SUBDIVISIONS;
            let (a, b) = if last {
                (t - increment, t)
            } else {
                (t, t + increment)
            };
            let point = |t| {
                viewer_unit_reads::movement_routes::catmull_rom(
                    window[0], window[1], window[2], window[3], t,
                )
            };
            let (a, b) = (point(a), point(b));
            let delta = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let chord = (delta[1] * delta[1] + delta[0] * delta[0] + delta[2] * delta[2]).sqrt();
            let raw_side = normalize([-delta[2], 0.0, delta[0]]);
            let side = if segment == 0 {
                raw_side
            } else {
                normalize([
                    (previous_side[0] + raw_side[0]) * 0.5,
                    (previous_side[1] + raw_side[1]) * 0.5,
                    (previous_side[2] + raw_side[2]) * 0.5,
                ])
            };
            previous_side = raw_side;
            let tangent = normalize(delta);
            let advance = 2.0 * chord / width;
            let center = if last {
                arc += advance;
                b
            } else {
                a
            };
            let mut minus = [
                center[0] - side[0] * width,
                center[1] - side[1] * width,
                center[2] - side[2] * width,
            ];
            let mut plus = [
                center[0] + side[0] * width,
                center[1] + side[1] * width,
                center[2] + side[2] * width,
            ];
            let height = heights
                .native_height(plus[0], plus[2])
                .max(heights.native_height(minus[0], minus[2]));
            minus[1] = height;
            plus[1] = height;
            for (position, lateral) in [(minus, 1.0), (plus, 0.0)] {
                vertices.push(Vertex {
                    position,
                    uv: [lateral, arc],
                    tangent,
                });
            }
            if !last {
                arc += advance;
            }
        }
    }
    if !arc.is_finite()
        || vertices.iter().any(|vertex| {
            vertex
                .position
                .iter()
                .chain(vertex.uv.iter())
                .chain(vertex.tangent.iter())
                .any(|value| !value.is_finite())
        })
    {
        return Err("movement arrow tessellation produced a non-finite vertex".into());
    }
    Ok(Strip {
        bounds,
        length: vertices.last().map_or(0.0, |vertex| vertex.uv[1]),
        vertices,
    })
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length != 0.0 {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        vector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> HeightField<'static> {
        HeightField {
            width: 2,
            height: 2,
            values: &[0; 4],
        }
    }

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn straight_strip_uses_native_width_uv_order_and_terminal_chord() {
        let strip = build(
            &[
                [-10.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
            ],
            1.0,
            flat(),
        )
        .unwrap();
        assert_eq!(std::mem::size_of::<Vertex>(), 32);
        assert_eq!(strip.vertices.len(), 42);
        assert_eq!(strip.vertices[0].position, [0.0, 9.5, -2.0]);
        assert_eq!(strip.vertices[1].position, [0.0, 9.5, 2.0]);
        assert_eq!(strip.vertices[0].uv, [1.0, 0.0]);
        assert_eq!(strip.vertices[1].uv, [0.0, 0.0]);
        assert_eq!(strip.vertices[40].position, [10.0, 9.5, -2.0]);
        assert_eq!(strip.vertices[41].position, [10.0, 9.5, 2.0]);
        close(strip.length, 10.5);
        for vertex in &strip.vertices {
            assert_eq!(vertex.tangent, [1.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn sharp_turn_preserves_inserted_corner_controls_and_uv_continuity() {
        let strip = build(
            &[
                [-10.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [-10.0, 0.0, 0.0],
            ],
            1.0,
            flat(),
        )
        .unwrap();
        assert_eq!(strip.vertices.len(), 162);
        let center = |sample: usize| {
            let (a, b) = (
                strip.vertices[sample * 2].position,
                strip.vertices[sample * 2 + 1].position,
            );
            [(a[0] + b[0]) * 0.5, (a[2] + b[2]) * 0.5]
        };
        assert_eq!(center(20), [7.5, 3.0]);
        assert_eq!(center(40), [10.0, 0.0]);
        assert_eq!(center(60), [7.5, -3.0]);
        assert_eq!(center(80), [0.0, 0.0]);
        #[allow(clippy::excessive_precision)]
        let samples = [
            (20, [7.5101194, 1.0000256], [7.4898806, 4.9999743]),
            (21, [7.5031724, 0.9998970], [7.9843268, 4.9708529]),
        ];
        for (sample, minus, plus) in samples {
            let a = strip.vertices[sample * 2].position;
            let b = strip.vertices[sample * 2 + 1].position;
            for axis in 0..2 {
                close(a[axis * 2], minus[axis]);
                close(b[axis * 2], plus[axis]);
            }
        }
        for pair in strip.vertices.chunks_exact(2) {
            assert_eq!(pair[0].uv[1], pair[1].uv[1]);
            close(
                ((pair[0].position[0] - pair[1].position[0]).powi(2)
                    + (pair[0].position[2] - pair[1].position[2]).powi(2))
                .sqrt(),
                4.0,
            );
        }
        assert!(
            strip
                .vertices
                .windows(2)
                .all(|pair| pair[0].uv[1] <= pair[1].uv[1])
        );
    }

    #[test]
    fn both_edges_use_the_higher_sampled_terrain_height() {
        let heights = HeightField {
            width: 4,
            height: 4,
            values: &[
                200, 200, 200, 200, 180, 180, 180, 180, 160, 160, 160, 160, 140, 140, 140, 140,
            ],
        };
        let strip = build(
            &[
                [0.0, 0.0, 1.5],
                [1.0, 0.0, 1.5],
                [2.0, 0.0, 1.5],
                [3.0, 0.0, 1.5],
            ],
            0.25,
            heights,
        )
        .unwrap();
        assert_eq!(strip.vertices[0].position, [1.0, 17.0, 1.0]);
        assert_eq!(strip.vertices[1].position, [1.0, 17.0, 2.0]);
    }
}
