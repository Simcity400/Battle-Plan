use super::{LiveParticle, Simulation};
use content_reader_mesh::particles::{RenderDescriptor, Sort};
use viewer_gpu::camera::Camera;

const GAPS: [usize; 6] = [1, 4, 1, 16, 1, 64];

#[derive(Clone, Copy)]
pub(super) struct Eye {
    position: [f32; 3],
    forward: [f32; 3],
}

impl Eye {
    pub fn of(camera: Camera) -> Self {
        Self {
            position: camera.eye(),
            forward: camera.look_direction(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Order {
    sort: Sort,
    invert: bool,
    phase: u8,
}

impl Order {
    pub fn new(render: &RenderDescriptor) -> Self {
        Self {
            sort: render.sort,
            invert: render.invert,
            phase: 1,
        }
    }

    fn pass(&mut self, particles: &mut [LiveParticle], eye: Eye) {
        let count = particles.len();
        if self.sort == Sort::None || count == 0 {
            return;
        }
        let gap = GAPS[usize::from(self.phase)];
        self.phase = (self.phase + 1) % GAPS.len() as u8;
        let (sort, invert) = (self.sort, self.invert);
        let key = |p: &LiveParticle| {
            let [dx, dy, dz] = std::array::from_fn(|axis| p.position[axis] - eye.position[axis]);
            match sort {
                Sort::Depth => dx * eye.forward[0] + dy * eye.forward[1] + dz * eye.forward[2],
                Sort::Distance => dy * dy + dx * dx + dz * dz,
                Sort::Age | Sort::None => p.age,
            }
        };
        for mut start in [0, gap] {
            while start + 2 * gap < count {
                for j in start..start + gap {
                    let (near, far) = (key(&particles[j]), key(&particles[j + gap]));
                    if if invert { far < near } else { near < far } {
                        particles.swap(j, j + gap);
                    }
                }
                start += 2 * gap;
            }
        }
    }
}

impl Simulation {
    pub(super) fn sort(&mut self, eye: Eye) {
        self.order.pass(&mut self.particles, eye);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn particles(depths: &[f32]) -> Vec<LiveParticle> {
        depths
            .iter()
            .map(|&z| LiveParticle {
                age: -z,
                life: 1.,
                mass: 1.,
                position: [0., 0., z],
                velocity: [0.; 3],
                size: 1.,
                rotation: [0.; 3],
                rotation_base: [0.; 3],
                spin: [0.; 3],
                color: [0.; 4],
                tile: [0; 4],
            })
            .collect()
    }

    fn depths(particles: &[LiveParticle]) -> Vec<f32> {
        particles.iter().map(|p| p.position[2]).collect()
    }

    const EYE: Eye = Eye {
        position: [0., 0., 0.],
        forward: [0., 0., 1.],
    };

    fn order(sort: Sort, invert: bool) -> Order {
        Order::new(&RenderDescriptor {
            sort,
            invert,
            ..RenderDescriptor::read(&[]).unwrap()
        })
    }

    fn at_gap_one(sort: Sort, invert: bool) -> Order {
        Order {
            phase: 0,
            ..order(sort, invert)
        }
    }

    #[test]
    fn one_frame_is_one_gap_pass_and_the_last_particle_is_never_compared() {
        let mut storage = particles(&[1., 2., 3., 4., 5., 6.]);
        let mut depth = order(Sort::Depth, false);
        depth.pass(&mut storage, EYE);
        assert_eq!(depths(&storage), [1., 2., 3., 4., 5., 6.]);
        depth.pass(&mut storage, EYE);
        assert_eq!(depths(&storage), [2., 4., 1., 5., 3., 6.]);
        for _ in 0..12 {
            depth.pass(&mut storage, EYE);
        }
        assert_eq!(depths(&storage), [5., 4., 3., 2., 1., 6.]);
    }

    #[test]
    fn invert_distance_and_age_choose_the_key_and_ties_stay() {
        let mut storage = particles(&[3., -1., 2., 2., -4.]);
        at_gap_one(Sort::Depth, true).pass(&mut storage, EYE);
        assert_eq!(depths(&storage), [-1., 2., 3., 2., -4.]);
        let mut storage = particles(&[3., -1., 2., 2., -4.]);
        at_gap_one(Sort::Distance, false).pass(&mut storage, EYE);
        assert_eq!(depths(&storage), [3., 2., -1., 2., -4.]);
        let mut storage = particles(&[3., -1., 2., 2., -4.]);
        at_gap_one(Sort::Age, false).pass(&mut storage, EYE);
        assert_eq!(depths(&storage), [-1., 2., 3., 2., -4.]);
        let mut storage = particles(&[3., -1., 2., 2., -4.]);
        let mut none = at_gap_one(Sort::None, false);
        none.pass(&mut storage, EYE);
        assert_eq!(
            (depths(&storage), none.phase),
            (vec![3., -1., 2., 2., -4.], 0)
        );
    }
}
