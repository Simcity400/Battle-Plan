use viewer_models::{ModelInstance, ModelMesh};
use viewer_unit_history::unit_history::LandFigure;
use content_reader_layers::ResolvedLayers;
use content_reader_mesh::mesh;
use std::sync::Arc;

const PARALLEL: f32 = 1e-4;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Triangles { positions: Vec<[f32; 3]>, indices: Vec<u16> },
    Bounds { min: [f32; 3], max: [f32; 3] },
}

impl Shape {
    pub fn of(layers: &ResolvedLayers, model: &ModelMesh) -> Result<Arc<Self>, String> {
        if model.path.is_empty() {
            return Err(format!("unit model {} has no mesh to test", model.name));
        }
        let tree = mesh::parse_in(layers, &model.path).map_err(|error| format!("{}: {error}", model.path))?;
        if let Some(collision) = mesh::collision(&tree).map_err(|error| format!("{}: {error}", model.path))? {
            return Ok(Arc::new(Self::scaled(collision, model.scale)));
        }
        let decoded = mesh::read_tree(&tree).map_err(|error| format!("{}: {error}", model.path))?;
        Self::bounds(&decoded, model.scale)
            .map(Arc::new)
            .ok_or_else(|| format!("{} has no collision mesh and no sub-mesh bounds", model.path))
    }

    fn scaled(collision: mesh::Collision, scale: f32) -> Self {
        Self::Triangles {
            positions: collision.positions.into_iter().map(|p| p.map(|v| v * scale)).collect(),
            indices: collision.indices,
        }
    }

    fn bounds(decoded: &mesh::Mesh, scale: f32) -> Option<Self> {
        let mut boxes = decoded
            .shapes
            .iter()
            .flat_map(|shape| &shape.meshes)
            .filter(|sub| sub.material.as_ref().and_then(|material| material.shader.as_deref()) != Some("Collision"))
            .filter_map(|sub| sub.bounds);
        let first = boxes.next()?;
        let (min, max) = boxes.fold((first.min, first.max), |(min, max), next| {
            (
                std::array::from_fn(|axis| min[axis].min(next.min[axis])),
                std::array::from_fn(|axis| max[axis].max(next.max[axis])),
            )
        });
        let (a, b) = (min.map(|v| v * scale), max.map(|v| v * scale));
        Some(Self::Bounds {
            min: std::array::from_fn(|axis| a[axis].min(b[axis])),
            max: std::array::from_fn(|axis| a[axis].max(b[axis])),
        })
    }

    pub fn reach(&self) -> Option<f32> {
        match self {
            Self::Triangles { positions, .. } => Some(positions.iter().map(|&p| dot(p, p)).fold(0.0, f32::max).sqrt()),
            Self::Bounds { .. } => None,
        }
    }

    pub fn hit(&self, instance: ModelInstance, joints: &[[f32; 3]], origin: [f32; 3], direction: [f32; 3]) -> bool {
        match self {
            Self::Triangles { positions, indices } => triangles(positions, indices, instance, origin, direction),
            Self::Bounds { min, max } => {
                let (min, max) = match joints.split_first() {
                    Some((first, rest)) => rest.iter().fold((*first, *first), |(min, max), joint| {
                        (
                            std::array::from_fn(|axis| min[axis].min(joint[axis])),
                            std::array::from_fn(|axis| max[axis].max(joint[axis])),
                        )
                    }),
                    None => (*min, *max),
                };
                let place = |corner: [f32; 3]| -> [f32; 3] {
                    std::array::from_fn(|axis| corner[axis] * instance.scale + instance.position[axis])
                };
                let (a, b) = (place(min), place(max));
                slab(
                    std::array::from_fn(|axis| a[axis].min(b[axis])),
                    std::array::from_fn(|axis| a[axis].max(b[axis])),
                    origin,
                    direction,
                )
            }
        }
    }
}

const CELL: f32 = 8.0;
const MOST_CELLS: i64 = 4096;

pub struct Figures {
    stamp: u64,
    library: usize,
    width: f32,
    figures: Vec<LandFigure>,
    columns: i64,
    cell: f32,
    filed: Vec<((i64, i64), u32)>,
    loose: Vec<u32>,
    heights: [f32; 2],
}

impl Figures {
    pub fn current<'a>(
        slot: &'a mut Option<Self>,
        stamp: u64,
        library: usize,
        width: f32,
        build: impl FnOnce() -> Vec<LandFigure>,
        reach: impl FnMut(usize) -> Option<f32>,
    ) -> &'a Self {
        if slot.as_ref().is_some_and(|figures| (figures.stamp, figures.library, figures.width) != (stamp, library, width)) {
            *slot = None;
        }
        slot.get_or_insert_with(|| Self::new(stamp, library, width, build(), reach))
    }

    fn new(stamp: u64, library: usize, width: f32, figures: Vec<LandFigure>, mut reach: impl FnMut(usize) -> Option<f32>) -> Self {
        let columns = if width > 0.0 && width.is_finite() { ((width / CELL).round() as i64).max(1) } else { 0 };
        let cell = width / columns as f32;
        let (mut filed, mut loose, mut heights) = (Vec::new(), Vec::new(), [f32::INFINITY, f32::NEG_INFINITY]);
        for (index, figure) in (0..).zip(&figures) {
            let instance = figure.instance;
            let [x, y, z] = instance.position;
            let span = (columns > 0 && instance.native_flags & 4 == 0)
                .then(|| reach(figure.mesh))
                .flatten()
                .map(|reach| reach * instance.scale.abs() * (1.0 + 1e-4) + 1e-3)
                .filter(|reach| reach.is_finite() && x.is_finite() && y.is_finite() && z.is_finite())
                .and_then(|reach| {
                    let cells = |at: f32| ((at - reach) / cell).floor() as i64..=((at + reach) / cell).floor() as i64;
                    let (across, down) = (cells(x), cells(z));
                    let count = (across.end() - across.start() + 1) * (down.end() - down.start() + 1);
                    (across.end() - across.start() < columns && count <= MOST_CELLS).then_some((reach, across, down))
                });
            let Some((reach, across, down)) = span else {
                loose.push(index);
                continue;
            };
            heights = [heights[0].min(y - reach), heights[1].max(y + reach)];
            for column in across {
                filed.extend(down.clone().map(|row| ((column.rem_euclid(columns), row), index)));
            }
        }
        filed.sort_unstable();
        Self { stamp, library, width, figures, columns, cell, filed, loose, heights }
    }

    pub fn is_empty(&self) -> bool {
        self.figures.is_empty()
    }

    pub fn along(&self, eye: [f32; 3], ray: [f32; 3]) -> Vec<&LandFigure> {
        match self.indices(eye, ray) {
            Some(indices) => indices.into_iter().map(|index| &self.figures[index as usize]).collect(),
            None => self.figures.iter().collect(),
        }
    }

    fn indices(&self, eye: [f32; 3], ray: [f32; 3]) -> Option<Vec<u32>> {
        let [low, high] = self.heights;
        let mut found = self.loose.clone();
        if low > high {
            return Some(found);
        }
        if !(ray[1].abs() > f32::EPSILON) {
            return None;
        }
        let at = |y: f32| {
            let t = (y - eye[1]) / ray[1];
            [eye[0] + t * ray[0], eye[2] + t * ray[2]]
        };
        let (a, b) = (at(low), at(high));
        if !a.iter().chain(&b).all(|v| v.is_finite()) {
            return None;
        }
        let ([x0, z0], [x1, z1]) = if a[0] <= b[0] { (a, b) } else { (b, a) };
        let (first, last) = ((x0 / self.cell).floor() as i64, (x1 / self.cell).floor() as i64);
        if last - first >= self.columns {
            return None;
        }
        let mut visited = 0;
        for column in first..=last {
            let (zlo, zhi) = if x1 > x0 {
                let z = |x: f32| z0 + (z1 - z0) * ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
                let (from, to) = (z(column as f32 * self.cell), z((column + 1) as f32 * self.cell));
                (from.min(to), from.max(to))
            } else {
                (z0.min(z1), z0.max(z1))
            };
            let rows = (zlo / self.cell).floor() as i64..=(zhi / self.cell).floor() as i64;
            visited += rows.end() - rows.start() + 1;
            if visited > MOST_CELLS {
                return None;
            }
            for row in rows {
                let key = (column.rem_euclid(self.columns), row);
                let start = self.filed.partition_point(|entry| entry.0 < key);
                found.extend(self.filed[start..].iter().take_while(|entry| entry.0 == key).map(|entry| entry.1));
            }
        }
        found.sort_unstable();
        found.dedup();
        Some(found)
    }
}

fn triangles(positions: &[[f32; 3]], indices: &[u16], instance: ModelInstance, origin: [f32; 3], direction: [f32; 3]) -> bool {
    let Some(inverse) = viewer_models::entity_animation::world_transform(instance).inverse() else {
        return false;
    };
    let origin = inverse.transform_point(origin);
    let direction = inverse.transform_vector(direction);
    let length = dot(direction, direction).sqrt();
    if length == 0.0 || !length.is_finite() {
        return false;
    }
    let direction = direction.map(|v| v / length);
    indices.chunks_exact(3).any(|triangle| {
        let [a, b, c] = [0, 1, 2].map(|at| positions[usize::from(triangle[at])]);
        let (first, second) = (sub(b, a), sub(c, a));
        let p = cross(direction, second);
        let determinant = dot(first, p);
        if determinant > -PARALLEL && determinant < PARALLEL {
            return false;
        }
        let t = sub(origin, a);
        let u = dot(t, p) / determinant;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let v = dot(direction, cross(t, first)) / determinant;
        v >= 0.0 && u + v <= 1.0
    })
}

fn slab(min: [f32; 3], max: [f32; 3], origin: [f32; 3], direction: [f32; 3]) -> bool {
    let inverse = direction.map(|v| 1.0 / v);
    let span = |axis: usize| {
        let (near, far) = if inverse[axis] < 0.0 { (max[axis], min[axis]) } else { (min[axis], max[axis]) };
        ((near - origin[axis]) * inverse[axis], (far - origin[axis]) * inverse[axis])
    };
    let (mut enter, mut exit) = span(0);
    let (y_enter, y_exit) = span(1);
    if !(enter <= y_exit && y_enter <= exit) {
        return false;
    }
    if y_exit <= exit {
        exit = y_exit;
    }
    if enter <= y_enter {
        enter = y_enter;
    }
    let (z_enter, z_exit) = span(2);
    if !(enter <= z_exit && z_enter <= exit) {
        return false;
    }
    if enter <= z_enter {
        enter = z_enter;
    }
    if z_exit <= exit {
        exit = z_exit;
    }
    enter < f32::MAX && 0.0 < exit
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Shape {
        Shape::scaled(
            mesh::Collision {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                indices: vec![0, 1, 2],
            },
            2.0,
        )
    }

    fn at(position: [f32; 3]) -> ModelInstance {
        ModelInstance { position, scale: 1.0, ..Default::default() }
    }

    const DOWN: [f32; 3] = [0.0, -1.0, 0.0];

    #[test]
    fn the_ray_meets_the_scaled_triangle_where_it_is_drawn() {
        let shape = triangle();
        assert!(shape.hit(at([10.0, 0.0, 10.0]), &[], [10.5, 5.0, 10.5], DOWN));
        assert!(shape.hit(at([10.0, 0.0, 10.0]), &[], [11.9, 5.0, 10.05], DOWN), "inside only once scaled");
        assert!(!shape.hit(at([10.0, 0.0, 10.0]), &[], [12.1, 5.0, 10.05], DOWN));
        assert!(!shape.hit(at([0.0, 0.0, 0.0]), &[], [10.5, 5.0, 10.5], DOWN));
    }

    #[test]
    fn the_edge_counts_a_triangle_behind_the_eye_counts_and_a_grazing_ray_does_not() {
        let shape = triangle();
        assert!(shape.hit(at([0.0; 3]), &[], [1.0, 5.0, 1.0], DOWN), "u + v = 1");
        assert!(shape.hit(at([0.0; 3]), &[], [0.5, -5.0, 0.5], DOWN), "no distance test");
        assert!(!shape.hit(at([0.0; 3]), &[], [0.5, 1.0, -5.0], [0.0, 0.0, 1.0]));
    }

    #[test]
    fn the_instance_scale_and_turn_carry_the_triangle() {
        let shape = triangle();
        let turned = ModelInstance { rotation: std::f32::consts::PI, scale: 0.5, ..Default::default() };
        assert!(!shape.hit(turned, &[], [0.5, 5.0, 0.5], DOWN));
        assert!(shape.hit(turned, &[], [-0.4, 5.0, -0.4], DOWN));
        assert!(!shape.hit(turned, &[], [-1.1, 5.0, -0.05], DOWN));
    }

    #[test]
    fn bounds_are_scaled_and_placed_but_never_turned_and_posed_joints_replace_them() {
        let shape = Shape::Bounds { min: [-1.0, 0.0, -1.0], max: [1.0, 2.0, 1.0] };
        let turned = ModelInstance { position: [10.0, 0.0, 0.0], rotation: 0.7, scale: 2.0, ..Default::default() };
        assert!(shape.hit(turned, &[], [11.9, 9.0, 1.9], DOWN), "the box keeps the world axes");
        assert!(!shape.hit(turned, &[], [12.1, 9.0, 0.0], DOWN));
        let joints = [[0.0, 0.0, 0.0], [3.0, 1.0, 0.5]];
        assert!(shape.hit(turned, &joints, [15.9, 9.0, 0.5], DOWN), "the posed joints' box");
        assert!(!shape.hit(turned, &joints, [9.0, 9.0, 0.5], DOWN), "the static box no longer counts");
    }

    fn figure(province: u32, mesh: usize, instance: ModelInstance) -> LandFigure {
        LandFigure { province, order: 0, mesh, instance }
    }

    #[test]
    fn every_pick_between_two_changes_reads_one_build_of_the_figures() {
        let builds = std::cell::Cell::new(0);
        let mut slot = None;
        let pick = |slot: &mut Option<Figures>, stamp: u64, library: usize| {
            let build = || {
                builds.set(builds.get() + 1);
                vec![figure(1, 0, at([1.0, 0.0, 1.0]))]
            };
            Figures::current(slot, stamp, library, 100.0, build, |_| Some(1.0)).along([1.0, 5.0, 1.0], DOWN).len()
        };
        for _ in ["hover", "cursor", "click"] {
            assert_eq!(pick(&mut slot, 7, 1), 1);
        }
        assert_eq!(builds.get(), 1, "one frame's picks share one build");
        pick(&mut slot, 8, 1);
        assert_eq!(builds.get(), 2, "a new land stamp builds again");
        pick(&mut slot, 8, 2);
        pick(&mut slot, 8, 2);
        assert_eq!(builds.get(), 3, "so does another library, once");
    }

    #[test]
    fn a_ray_meets_through_the_filed_cells_exactly_what_it_meets_testing_every_figure() {
        let shape = triangle();
        let width = 100.0;
        let figures: Vec<LandFigure> = (0..60u32)
            .map(|i| {
                let instance = ModelInstance {
                    position: [(i * 37 % 103) as f32 - 1.5, (i % 5) as f32, (i * 53 % 61) as f32],
                    rotation: i as f32 * 0.7,
                    pitch: if i % 4 == 0 { 0.3 } else { 0.0 },
                    scale: 0.5 + (i % 3) as f32 * 0.5,
                    ..Default::default()
                };
                figure(i, usize::from(i % 7 == 0), instance)
            })
            .collect();
        let mut slot = None;
        let index = Figures::current(&mut slot, 1, 1, width, || figures.clone(), |mesh| (mesh == 0).then(|| shape.reach()).flatten());
        let met = |figure: &LandFigure, eye: [f32; 3], ray: [f32; 3]| {
            let mut instance = figure.instance;
            instance.position[0] = viewer_gpu::unit_pose::unwrap(instance.position[0], eye[0], width);
            shape.hit(instance, &[], eye, ray)
        };
        let rays = [DOWN, [0.3, -1.0, 0.2], [-0.6, -0.5, 0.1], [0.05, -1.0, -0.7], [1.0, 0.0, 0.2]];
        let (mut hits, mut narrowed) = (0, 0);
        for column in 0..40 {
            for row in 0..20 {
                let eye = [column as f32 * 3.7 - 10.0, 30.0, row as f32 * 3.3];
                for ray in rays {
                    let every: Vec<u32> = figures.iter().filter(|f| met(f, eye, ray)).map(|f| f.province).collect();
                    let along = index.along(eye, ray);
                    narrowed += usize::from(along.len() < figures.len());
                    let filed: Vec<u32> = along.into_iter().filter(|f| met(f, eye, ray)).map(|f| f.province).collect();
                    assert_eq!(filed, every, "eye {eye:?} ray {ray:?}");
                    hits += every.len();
                }
            }
        }
        assert!(hits > 10 && narrowed > 1000, "{hits} hits, {narrowed} rays narrowed");
    }

    #[test]
    fn the_slab_needs_the_exit_ahead_of_the_origin() {
        let (min, max) = ([0.0; 3], [1.0; 3]);
        assert!(slab(min, max, [0.5, 5.0, 0.5], DOWN));
        assert!(!slab(min, max, [0.5, -5.0, 0.5], DOWN), "a box behind the eye is missed");
        assert!(slab(min, max, [0.5, 0.5, 0.5], DOWN), "from inside");
        assert!(!slab(min, max, [2.0, 5.0, 0.5], DOWN));
    }

    #[test]
    fn static_bounds_gather_drawable_sub_meshes_and_leave_the_collision_hull_out() {
        let sub = |shader: &str, min: [f32; 3], max: [f32; 3]| mesh::SubMesh {
            bounds: Some(mesh::Bounds { min, max }),
            material: Some(mesh::Material { shader: Some(shader.into()), ..Default::default() }),
            ..Default::default()
        };
        let decoded = mesh::Mesh {
            shapes: vec![mesh::Shape {
                meshes: vec![
                    sub("PdxMeshStandard", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
                    sub("Collision", [-9.0; 3], [9.0; 3]),
                    sub("PdxMeshStandard", [-1.0, 0.5, 0.0], [0.5, 2.0, 0.5]),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(Shape::bounds(&decoded, 2.0), Some(Shape::Bounds { min: [-2.0, 0.0, 0.0], max: [2.0, 4.0, 2.0] }));
        let mirrored = Shape::bounds(&decoded, -2.0).unwrap();
        assert_eq!(mirrored, Shape::Bounds { min: [-2.0, -4.0, -2.0], max: [2.0, 0.0, 0.0] });
        assert!(mirrored.hit(at([10.0, 0.0, 10.0]), &[], [9.0, 5.0, 9.0], DOWN));
        assert!(mirrored.hit(ModelInstance { position: [10.0, 0.0, 10.0], scale: -1.0, ..Default::default() }, &[], [11.0, 5.0, 11.0], DOWN));
        assert_eq!(Shape::bounds(&mesh::Mesh::default(), 1.0), None);
    }

    #[test]
    fn root_joints_supports_mirrored_models_with_negative_pdxmesh_scale() {
        use viewer_models::{entity_animation, test_support::animated_root_fixture};

        let instance = ModelInstance {
            position: [10.0, 0.0, 20.0],
            scale: 1.0,
            animation: entity_animation::IDLE,
            animation_seed: 1,
            animation_identity: 1,
            ..Default::default()
        };
        let (positive, positive_instance) = animated_root_fixture(2.0, "positive", instance);
        let (mirrored, mirrored_instance) = animated_root_fixture(-2.0, "mirrored", instance);
        let positive_joints = positive
            .root_joints(0, &positive_instance)
            .unwrap()
            .unwrap();
        let mirrored_joints = mirrored
            .root_joints(0, &mirrored_instance)
            .unwrap()
            .unwrap();
        assert!(!mirrored_joints.is_empty());
        let expected: Vec<[f32; 3]> = positive_joints
            .iter()
            .map(|joint| joint.map(|v| -v))
            .collect();
        for (actual, want) in mirrored_joints.iter().zip(&expected) {
            for axis in 0..3 {
                assert!(
                    (actual[axis] - want[axis]).abs() < 1e-5,
                    "axis {axis}: actual {:?} vs want {:?}",
                    actual,
                    want
                );
            }
        }
        let shape = Shape::Bounds {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        let center = [0, 1, 2].map(|axis| {
            let lo = mirrored_joints
                .iter()
                .map(|j| j[axis])
                .fold(f32::INFINITY, f32::min);
            let hi = mirrored_joints
                .iter()
                .map(|j| j[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            0.5 * (lo + hi) + instance.position[axis]
        });
        assert!(shape.hit(
            mirrored_instance,
            &mirrored_joints,
            [center[0], center[1] + 10.0, center[2]],
            [0.0, -1.0, 0.0],
        ));
    }
}
