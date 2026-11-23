use content_reader_mesh::mesh::Transform;

#[derive(Clone, Copy)]
pub(super) struct Space {
    pub world: Transform,
    pub scale: f32,
}

impl Space {
    pub fn new(local: bool, any_local: bool, authored: f32, world: Transform) -> Self {
        if local {
            Self {
                world: Transform::IDENTITY,
                scale: 1.,
            }
        } else {
            let scale = authored
                * if any_local {
                    viewer_scene_base::particle_shader::world_scale(world.basis)
                } else {
                    1.
                };
            Self { world, scale }
        }
    }

    pub fn vector(self, value: [f32; 3]) -> [f32; 3] {
        super::transform(self.world.basis, value)
    }

    pub fn birth(
        self,
        previous: [f32; 3],
        origin: [f32; 3],
        shape: [f32; 3],
        ordinal: usize,
        count: usize,
    ) -> [f32; 3] {
        let shape = self.vector(shape);
        let fraction = ordinal as f32 / count as f32;
        std::array::from_fn(|i| {
            (self.world.translation[i] - previous[i]) * fraction
                + previous[i]
                + origin[i]
                + shape[i]
        })
    }

    pub fn integrate(
        self,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        force: [f32; 3],
        mass: f32,
        dt: f32,
    ) {
        let motion_dt = self.scale * dt;
        let force_dt = dt / mass;
        for i in 0..3 {
            velocity[i] += force[i] * force_dt;
            position[i] += velocity[i] * motion_dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_space_keeps_local_births_local_and_world_births_between_frames() {
        let world = Transform {
            basis: [[0., 0., 2.], [0., 3., 0.], [-4., 0., 0.]],
            translation: [20., 10., 8.],
        };
        let local = Space::new(true, true, 2., world);
        assert_eq!(local.scale, 1.);
        assert_eq!(
            local.birth([0.; 3], [1., 2., 3.], [2., 0., 0.], 1, 2),
            [3., 2., 3.]
        );
        let nonlocal = Space::new(false, true, 2., world);
        assert_eq!(nonlocal.scale, 8.);
        assert_eq!(
            nonlocal.birth([10., 4., 2.], [1., 2., 3.], [2., 0., 0.], 0, 2),
            [11., 6., 9.]
        );
        assert_eq!(
            nonlocal.birth([10., 4., 2.], [1., 2., 3.], [2., 0., 0.], 1, 2),
            [16., 9., 12.]
        );
        assert_eq!(Space::new(false, false, 2., world).scale, 2.);
    }

    #[test]
    fn scale_changes_motion_but_never_force_time_or_existing_world_positions() {
        let space = Space::new(
            false,
            true,
            2.,
            Transform::from_trs([100., 0., 0.], [0., 0., 0., 1.], 3.),
        );
        let mut position = [10., 20., 30.];
        let mut velocity = [1., 2., 3.];
        space.integrate(&mut position, &mut velocity, [8., -4., 0.], 2., 0.5);
        assert_eq!(velocity, [3., 1., 3.]);
        assert_eq!(position, [19., 23., 39.]);
        let changed = Space::new(false, true, 1., Transform::IDENTITY);
        changed.integrate(&mut position, &mut velocity, [0.; 3], 2., 0.5);
        assert_eq!(position, [20.5, 23.5, 40.5]);
    }

    #[test]
    fn native_force_divides_time_by_mass_before_multiplying() {
        let space = Space::new(true, true, 1., Transform::IDENTITY);
        let mut position = [0.; 3];
        let mut velocity = [0.; 3];
        space.integrate(&mut position, &mut velocity, [3.1; 3], 0.3, 0.1);
        assert_eq!(velocity[0].to_bits(), 1_065_632_835);
        assert_eq!(velocity, [velocity[0]; 3]);
    }
}
