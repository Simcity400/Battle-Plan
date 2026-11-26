use super::{GameCull, ModelMesh};
use viewer_gpu::camera::Camera;

pub struct MotionView {
    cull: GameCull,
    width: f32,
}

impl MotionView {
    pub fn new(camera: &Camera, size: [u32; 2]) -> Self {
        Self {
            cull: GameCull::of(camera, size),
            width: camera.map.x(),
        }
    }

    pub fn times(
        &self,
        path: impl IntoIterator<Item = (f32, [f32; 3])>,
        mesh: &ModelMesh,
        scale: f32,
        mut visit: impl FnMut(f32, f32),
    ) {
        let mut points = path.into_iter();
        let Some(mut previous) = points.next() else {
            return;
        };
        for point in points {
            if let Some([enter, leave]) = self.passage(previous.1, point.1, mesh, scale) {
                let span = point.0 - previous.0;
                visit(previous.0 + span * enter, previous.0 + span * leave);
            }
            previous = point;
        }
    }

    pub fn passage(
        &self,
        a: [f32; 3],
        b: [f32; 3],
        mesh: &ModelMesh,
        scale: f32,
    ) -> Option<[f32; 2]> {
        let height = mesh.height * scale * 0.5;
        let raised = |[x, y, z]: [f32; 3]| [x, y + height, z];
        self.interval(
            raised(a),
            raised(b),
            mesh.bound * scale + height,
            mesh.cull_distance_squared,
        )
    }

    fn interval(
        &self,
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
        distance_squared: f32,
    ) -> Option<[f32; 2]> {
        let mut first = f32::INFINITY;
        let mut last = f32::NEG_INFINITY;
        let near = ((self.cull.x - a[0]) / self.width).round() * self.width;
        let delta: [f32; 3] = std::array::from_fn(|axis| b[axis] - a[axis]);
        for shift in [near - self.width, near, near + self.width] {
            let from = [a[0] + shift, a[1], a[2]];
            let eye: [f32; 3] = std::array::from_fn(|axis| from[axis] - self.cull.eye[axis]);
            let dot = |left: [f32; 3], right: [f32; 3]| {
                left.iter().zip(right).map(|(x, y)| x * y).sum::<f32>()
            };
            let aa = dot(delta, delta);
            let bb = dot(eye, delta);
            let cc = dot(eye, eye) - distance_squared;
            let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
            if aa > 0.0 {
                let discriminant = bb * bb - aa * cc;
                if discriminant < 0.0 {
                    continue;
                }
                let root = discriminant.sqrt();
                enter = enter.max((-bb - root) / aa);
                leave = leave.min((-bb + root) / aa);
            } else if cc > 0.0 {
                continue;
            }
            for [x, y, z, offset] in self.cull.sides {
                let normal = [x, y, z];
                let distance = dot(normal, from) + offset + radius;
                let slope = dot(normal, delta);
                if slope > 0.0 {
                    enter = enter.max(-distance / slope);
                } else if slope < 0.0 {
                    leave = leave.min(-distance / slope);
                } else if distance < 0.0 {
                    enter = f32::INFINITY;
                    break;
                }
                if enter > leave {
                    break;
                }
            }
            if enter <= leave {
                first = first.min(enter);
                last = last.max(leave);
            }
        }
        (first <= last).then_some([first, last])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offscreen_actor_wakes_when_its_route_enters_the_view() {
        let view = MotionView {
            width: 100.0,
            cull: GameCull {
                eye: [0.0; 3],
                x: 0.0,
                pad: 0.0,
                sides: [
                    [1., 0., 0., 2.],
                    [-1., 0., 0., 2.],
                    [0., 0., 1., 1.],
                    [0., 0., -1., 1.],
                ],
            },
        };
        let crossing =
            |start, end, z, distance| view.interval([start, 0., z], [end, 0., z], 0., distance);
        assert_eq!(crossing(-4., 4., 0., 16.), Some([0.25, 0.75]));
        assert_eq!(crossing(96., 104., 0., 16.), Some([0.25, 0.75]));
        assert_eq!(crossing(-4., 4., 0., 1.), Some([0.375, 0.625]));
        assert_eq!(crossing(-4., 4., 2., 16.), None);
        assert_eq!(crossing(3., 4., 0., 16.), None);
    }
}
