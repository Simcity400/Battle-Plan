use super::{Instance, fail};
use bytemuck::{Pod, Zeroable};
use std::io;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub tile: [u8; 4],
    pub color: [u8; 4],
}

pub fn vertices(
    records: &[Instance],
    eye: [f32; 3],
    scale: f32,
    billboard: bool,
) -> io::Result<Vec<Vertex>> {
    let mut out = Vec::new();
    if records.len() < 2 {
        return Ok(out);
    }
    out.try_reserve(
        records
            .len()
            .checked_mul(2)
            .ok_or_else(|| fail("trail vertex count overflow"))?,
    )
    .map_err(fail)?;
    let normalize = |v: [f32; 3]| {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if n != 0. { v.map(|x| x / n) } else { v }
    };
    let mut previous = [0.; 3];
    for (index, pair) in records.windows(2).enumerate() {
        let start = &pair[0];
        let end = &pair[1];
        let delta: [f32; 3] =
            std::array::from_fn(|i| end.position_size[i] - start.position_size[i]);
        let direction = if billboard {
            let eye: [f32; 3] = std::array::from_fn(|i| eye[i] - start.position_size[i]);
            normalize([
                eye[1] * delta[2] - eye[2] * delta[1],
                eye[2] * delta[0] - eye[0] * delta[2],
                eye[0] * delta[1] - eye[1] * delta[0],
            ])
        } else {
            let current = normalize([
                delta[1] * 0. - delta[2],
                delta[2] * 0. - delta[0] * 0.,
                delta[0] - delta[1] * 0.,
            ]);
            let joined = if index == 0 {
                current
            } else {
                normalize(std::array::from_fn(|i| (previous[i] + current[i]) * 0.5))
            };
            previous = current;
            joined
        };
        let mut emit = |p: &Instance, ordinal: usize| -> io::Result<()> {
            for positive in [true, false] {
                let position = std::array::from_fn(|i| {
                    let offset = p.position_size[3] * direction[i];
                    (if positive {
                        offset + p.position_size[i]
                    } else {
                        p.position_size[i] - offset
                    }) * scale
                });
                if position.iter().any(|v| !v.is_finite()) {
                    return Err(fail("trail expansion produced non-finite position"));
                }
                out.push(Vertex {
                    position,
                    uv: [ordinal as f32, f32::from(positive)],
                    tile: p.tile,
                    color: p.color,
                });
            }
            Ok(())
        };
        emit(start, index)?;
        if index + 2 == records.len() {
            emit(end, index + 1)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn particle(position: [f32; 4]) -> Instance {
        Instance {
            position_size: position,
            tile: [2, 3, 0, 0],
            color: [12, 34, 56, 78],
            ..Instance::zeroed()
        }
    }

    #[test]
    fn strip_keeps_pair_order_bytes_scale_and_fixed_plane_join() {
        let records = [
            particle([0., 0., 0., 1.]),
            particle([2., 0., 0., 2.]),
            particle([2., 0., 2., 3.]),
        ];
        let actual = vertices(&records, [0.; 3], 2., false).unwrap();
        assert_eq!(std::mem::size_of::<Vertex>(), 28);
        assert_eq!(actual.len(), 6);
        assert_eq!(actual[0].position, [0., 0., 2.]);
        assert_eq!(actual[1].position, [0., 0., -2.]);
        let join = 0.5_f32 / 0.5_f32.sqrt();
        assert_eq!(actual[2].position, [(2. - 2. * join) * 2., 0., 4. * join]);
        assert_eq!(
            actual[4].position,
            [(2. - 3. * join) * 2., 0., (2. + 3. * join) * 2.]
        );
        for (index, v) in actual.iter().enumerate() {
            assert_eq!(
                v.uv,
                [(index / 2) as f32, if index % 2 == 0 { 1. } else { 0. }]
            );
            assert_eq!(v.tile, [2, 3, 0, 0]);
            assert_eq!(v.color, [12, 34, 56, 78]);
        }
    }

    #[test]
    fn billboard_and_degenerate_segments_preserve_native_draw_counts() {
        let a = particle([0., 0., 0., 1.]);
        assert!(vertices(&[a], [0.; 3], 1., true).unwrap().is_empty());
        let b = particle([2., 0., 0., 1.]);
        let actual = vertices(&[a, b], [0., 0., 4.], 1., true).unwrap();
        assert_eq!(actual[0].position, [0., 1., 0.]);
        assert_eq!(actual[3].position, [2., -1., 0.]);
        let same = vertices(&[b, b], [0., 0., 4.], 1., true).unwrap();
        assert!(same.iter().all(|p| p.position == [2., 0., 0.]));
    }
}
