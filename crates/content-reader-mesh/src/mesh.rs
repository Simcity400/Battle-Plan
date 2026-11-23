use content_reader_layers::ResolvedLayers;

use std::{collections::BTreeMap, io};

mod animation;
mod container;
mod descriptors;

pub use animation::{Animation, AnimationJoint, FIRST_FRAME, JointPose, read_animation};
pub use container::{Node, Value, parse};
pub use descriptors::{
    AnimationDefinition, Attachment, DEFAULT_CULL_DISTANCE_SQUARED, DEFAULT_STATE, Descriptors, EntityDefinition, EntityEvent,
    EntityGameData, EntityState, InheritedEntity, Locator, MeshDefinition, MeshSetting,
    NavalHitMissEffect, ResolvedEntity, ResolvedMesh, Roster, TextureIndex, balanced_blocks,
    lexical_names, parse_descriptors,
};

pub const MAGIC: [u8; 4] = *b"@@b@";

pub fn parse_in(layers: &content_reader_layers::ResolvedLayers, path: &str) -> io::Result<std::sync::Arc<Node>> {
    layers.file_product(path.as_bytes(), "binary asset grammar", 1, &[], |bytes| parse(bytes))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub basis: [[f32; 3]; 3],
    pub translation: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: [0.0; 3],
    };

    pub fn from_columns(values: &[f32]) -> Option<Self> {
        let values: &[f32; 12] = values.get(..12)?.try_into().ok()?;
        Some(Self {
            basis: [
                [values[0], values[1], values[2]],
                [values[3], values[4], values[5]],
                [values[6], values[7], values[8]],
            ],
            translation: [values[9], values[10], values[11]],
        })
    }

    pub fn from_trs(translation: [f32; 3], rotation: [f32; 4], scale: f32) -> Self {
        let length = rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
        let [x, y, z, w] = if length > 0.0 {
            [
                rotation[0] / length,
                rotation[1] / length,
                rotation[2] / length,
                rotation[3] / length,
            ]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        let basis = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + z * w),
                2.0 * (x * z - y * w),
            ],
            [
                2.0 * (x * y - z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z + x * w),
            ],
            [
                2.0 * (x * z + y * w),
                2.0 * (y * z - x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ];
        Self {
            basis: basis.map(|axis| axis.map(|value| value * scale)),
            translation,
        }
    }

    pub fn compose(&self, inner: &Self) -> Self {
        Self {
            basis: inner.basis.map(|axis| self.transform_vector(axis)),
            translation: self.transform_point(inner.translation),
        }
    }

    pub fn transform_point(&self, point: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = self.transform_vector(point);
        [
            x + self.translation[0],
            y + self.translation[1],
            z + self.translation[2],
        ]
    }

    pub fn transform_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        let [a, b, c] = self.basis;
        [
            a[0] * vector[0] + b[0] * vector[1] + c[0] * vector[2],
            a[1] * vector[0] + b[1] * vector[1] + c[1] * vector[2],
            a[2] * vector[0] + b[2] * vector[1] + c[2] * vector[2],
        ]
    }

    pub fn determinant(&self) -> f32 {
        let [a, b, c] = self.basis;
        a[0] * (b[1] * c[2] - b[2] * c[1]) - b[0] * (a[1] * c[2] - a[2] * c[1])
            + c[0] * (a[1] * b[2] - a[2] * b[1])
    }

    pub fn inverse(&self) -> Option<Self> {
        let determinant = self.determinant();
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return None;
        }
        let [a, b, c] = self.basis;
        let inverse = 1.0 / determinant;
        let basis = [
            [
                (b[1] * c[2] - b[2] * c[1]) * inverse,
                (a[2] * c[1] - a[1] * c[2]) * inverse,
                (a[1] * b[2] - a[2] * b[1]) * inverse,
            ],
            [
                (b[2] * c[0] - b[0] * c[2]) * inverse,
                (a[0] * c[2] - a[2] * c[0]) * inverse,
                (a[2] * b[0] - a[0] * b[2]) * inverse,
            ],
            [
                (b[0] * c[1] - b[1] * c[0]) * inverse,
                (a[1] * c[0] - a[0] * c[1]) * inverse,
                (a[0] * b[1] - a[1] * b[0]) * inverse,
            ],
        ];
        let linear = Self {
            basis,
            translation: [0.0; 3],
        };
        let translation = linear.transform_vector(self.translation);
        let out = Self {
            basis,
            translation: [-translation[0], -translation[1], -translation[2]],
        };
        out.is_finite().then_some(out)
    }

    pub fn is_finite(&self) -> bool {
        self.basis
            .iter()
            .flatten()
            .chain(self.translation.iter())
            .all(|value| value.is_finite())
    }

    pub fn uniform_scale(&self) -> f32 {
        self.basis
            .iter()
            .map(|axis| axis.iter().map(|v| v * v).sum::<f32>().sqrt())
            .sum::<f32>()
            / 3.0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Joint {
    pub name: String,
    pub index: u32,
    pub parent: Option<u32>,
    pub inverse_bind: Option<Transform>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Material {
    pub shader: Option<String>,
    pub shader_file: Option<String>,
    pub diffuse: String,
    pub normal: String,
    pub specular: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

pub const INFLUENCES: usize = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skin {
    pub bones: u32,
    pub joints: Vec<[i32; INFLUENCES]>,
    pub weights: Vec<[f32; INFLUENCES]>,
}

impl Skin {
    pub fn influences(&self, vertex: usize) -> Vec<(u32, f32)> {
        let (Some(joints), Some(weights)) = (self.joints.get(vertex), self.weights.get(vertex))
        else {
            return Vec::new();
        };
        joints
            .iter()
            .zip(weights)
            .filter(|(joint, weight)| **joint >= 0 && **weight != 0.0)
            .map(|(joint, weight)| (*joint as u32, *weight))
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubMesh {
    pub child_index: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    pub uv1: Option<Vec<[f32; 2]>>,
    pub uv2: Option<Vec<[f32; 2]>>,
    pub uv3: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    pub bounds: Option<Bounds>,
    pub material: Option<Material>,
    pub skin: Option<Skin>,
}

impl SubMesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    #[cfg(test)]
    fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub name: String,
    pub lod: Option<i32>,
    pub meshes: Vec<SubMesh>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub lod_distances: Vec<f32>,
    pub shapes: Vec<Shape>,
    pub skeleton: Vec<Joint>,
    pub locators: Vec<String>,
}

impl Mesh {
    pub fn lods(&self) -> Vec<Vec<&Shape>> {
        if self.shapes.iter().all(|shape| shape.lod.is_none()) {
            return if self.shapes.is_empty() {
                Vec::new()
            } else {
                vec![self.shapes.iter().collect()]
            };
        }
        let levels = self
            .shapes
            .iter()
            .filter_map(|shape| shape.lod)
            .max()
            .unwrap_or(0)
            .max(0) as usize
            + 1;
        (0..levels)
            .map(|lod| {
                self.shapes
                    .iter()
                    .filter(|shape| shape.lod == Some(lod as i32))
                    .collect()
            })
            .collect()
    }

    pub fn joint(&self, name: &str) -> Option<&Joint> {
        self.skeleton.iter().find(|joint| joint.name == name)
    }

    pub fn bind_pose(&self, name: &str) -> Option<Transform> {
        self.joint(name)?.inverse_bind?.inverse()
    }

    pub fn pose(&self, animation: &Animation, frame: u32, name: &str) -> Option<Transform> {
        let index = self.joint(name)?.index;
        let locals = locals_by_name(animation, frame);
        self.world_pose(&locals, index, &mut self.memo(), 0)
    }

    pub fn sampled_pose(&self, animation: &Animation, locals: &[JointPose]) -> Vec<Transform> {
        let locals: BTreeMap<&str, Transform> = animation
            .joints
            .iter()
            .zip(locals)
            .map(|(joint, local)| (joint.name.as_str(), local.transform()))
            .collect();
        let mut memo = self.memo();
        (0..self
            .skeleton
            .iter()
            .map(|j| j.index as usize + 1)
            .max()
            .unwrap_or(0))
            .map(|slot| {
                self.world_pose(&locals, slot as u32, &mut memo, 0)
                    .unwrap_or(Transform::IDENTITY)
            })
            .collect()
    }

    pub fn palette(&self, animation: &Animation, frame: u32) -> Vec<Transform> {
        let locals = locals_by_name(animation, frame);
        let mut memo = self.memo();
        let slots = self
            .skeleton
            .iter()
            .map(|joint| joint.index as usize + 1)
            .max()
            .unwrap_or(0);
        (0..slots)
            .map(|slot| {
                let skinning = self
                    .skeleton
                    .iter()
                    .find(|joint| joint.index as usize == slot)
                    .and_then(|joint| {
                        let pose = self.world_pose(&locals, joint.index, &mut memo, 0)?;
                        Some(pose.compose(&joint.inverse_bind?))
                    });
                match skinning {
                    Some(skinning) if skinning.is_finite() => skinning,
                    _ => Transform::IDENTITY,
                }
            })
            .collect()
    }

    fn memo(&self) -> Vec<Option<Option<Transform>>> {
        vec![None; self.skeleton.len()]
    }

    fn world_pose(
        &self,
        locals: &BTreeMap<&str, Transform>,
        index: u32,
        memo: &mut [Option<Option<Transform>>],
        depth: usize,
    ) -> Option<Transform> {
        let at = self
            .skeleton
            .iter()
            .position(|joint| joint.index == index)?;
        if let Some(held) = memo.get(at).copied().flatten() {
            return held;
        }
        if depth > self.skeleton.len() {
            return None;
        }
        let joint = &self.skeleton[at];
        let parent = joint
            .parent
            .filter(|parent| *parent != joint.index)
            .and_then(|parent| self.skeleton.iter().position(|other| other.index == parent));
        let local = locals.get(joint.name.as_str()).copied().or_else(|| {
            let world = joint.inverse_bind?.inverse()?;
            match parent {
                Some(position) => Some(self.skeleton[position].inverse_bind?.compose(&world)),
                None => Some(world),
            }
        });
        let out = local.and_then(|local| match parent {
            Some(position) => Some(
                self.world_pose(locals, self.skeleton[position].index, memo, depth + 1)?
                    .compose(&local),
            ),
            None => Some(local),
        });
        if let Some(slot) = memo.get_mut(at) {
            *slot = Some(out);
        }
        out
    }
}

#[derive(Debug)]
pub struct PoseBinding {
    joints: Vec<BoundJoint>,
    slots: Vec<Option<usize>>,
}

#[derive(Debug)]
struct BoundJoint {
    parent: Option<usize>,
    locals: Vec<usize>,
    bind: Option<Transform>,
}

impl PoseBinding {
    pub fn new(mesh: &Mesh, animation: &Animation) -> Self {
        let count = mesh
            .skeleton
            .iter()
            .map(|j| j.index as usize + 1)
            .max()
            .unwrap_or(0);
        let mut slots = vec![None; count];
        for (at, joint) in mesh.skeleton.iter().enumerate() {
            slots[joint.index as usize].get_or_insert(at);
        }
        let joints = mesh
            .skeleton
            .iter()
            .map(|joint| {
                let parent = joint
                    .parent
                    .filter(|parent| *parent != joint.index)
                    .and_then(|parent| slots.get(parent as usize).copied().flatten());
                let bind = joint
                    .inverse_bind
                    .and_then(|inverse| inverse.inverse())
                    .and_then(|world| match parent {
                        Some(at) => Some(mesh.skeleton[at].inverse_bind?.compose(&world)),
                        None => Some(world),
                    });
                BoundJoint {
                    parent,
                    locals: animation
                        .joints
                        .iter()
                        .enumerate()
                        .filter_map(|(at, local)| (local.name == joint.name).then_some(at))
                        .collect(),
                    bind,
                }
            })
            .collect();
        Self { joints, slots }
    }

    pub fn sample_into(
        &self,
        locals: &[JointPose],
        out: &mut Vec<Transform>,
        memo: &mut Vec<Option<Option<Transform>>>,
    ) {
        memo.clear();
        memo.resize(self.joints.len(), None);
        out.clear();
        out.extend(self.slots.iter().map(|at| {
            at.and_then(|at| self.world(locals, at, memo, 0))
                .unwrap_or(Transform::IDENTITY)
        }));
    }

    fn world(
        &self,
        locals: &[JointPose],
        at: usize,
        memo: &mut [Option<Option<Transform>>],
        depth: usize,
    ) -> Option<Transform> {
        if let Some(held) = memo[at] {
            return held;
        }
        if depth > self.joints.len() {
            return None;
        }
        let joint = &self.joints[at];
        let local = joint
            .locals
            .iter()
            .rev()
            .find_map(|&at| locals.get(at))
            .map(JointPose::transform)
            .or(joint.bind);
        let out = local.and_then(|local| match joint.parent {
            Some(parent) => Some(self.world(locals, parent, memo, depth + 1)?.compose(&local)),
            None => Some(local),
        });
        memo[at] = Some(out);
        out
    }
}

fn locals_by_name(animation: &Animation, frame: u32) -> BTreeMap<&str, Transform> {
    animation
        .joints
        .iter()
        .map(|joint| joint.name.as_str())
        .zip(animation.frame(frame))
        .collect()
}

pub fn read(bytes: &[u8]) -> io::Result<Mesh> {
    read_tree(&parse(bytes)?)
}

pub fn read_animation_in(layers: &content_reader_layers::ResolvedLayers, path: &str) -> io::Result<std::sync::Arc<Animation>> {
    layers.file_product(path.as_bytes(), "skeletal animation", 1, &[], |bytes| {
        read_animation(bytes)
    })
}

pub fn read_tree(root: &Node) -> io::Result<Mesh> {
    let object = root
        .child("object")
        .ok_or_else(|| invalid("a PDX mesh carries no [object] node"))?;
    let lod_distances = object
        .property("loddist")
        .and_then(Value::floats)
        .unwrap_or(&[])
        .to_vec();
    let mut shapes = Vec::new();
    for child in &object.children {
        if child.name == "skeleton" {
            continue;
        }
        let mut meshes = Vec::new();
        for (index, node) in child.children.iter().enumerate() {
            if node.name == "mesh" {
                let mut mesh = read_sub_mesh(&child.name, node)?;
                mesh.child_index = index;
                meshes.push(mesh);
            }
        }
        if meshes.is_empty() && child.children.is_empty() {
            continue;
        }
        shapes.push(Shape {
            name: child.name.clone(),
            lod: child
                .property("lod")
                .and_then(Value::ints)
                .and_then(|values| values.first().copied()),
            meshes,
        });
    }
    let locators = root
        .child("locator")
        .map(|node| {
            node.children
                .iter()
                .map(|child| child.name.clone())
                .collect()
        })
        .unwrap_or_default();
    Ok(Mesh {
        lod_distances,
        shapes,
        skeleton: skeleton_node(root).map(read_joints).unwrap_or_default(),
        locators,
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collision {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u16>,
}

pub fn collision(root: &Node) -> io::Result<Option<Collision>> {
    let mut kept = None;
    for mesh in root
        .children_named("object")
        .flat_map(|object| &object.children)
        .flat_map(|shape| shape.children_named("mesh"))
    {
        if mesh.child("material").and_then(|material| material.string("shader")) != Some("Collision") {
            continue;
        }
        let (Some(positions), Some(indices)) = (
            mesh.property("p").and_then(Value::floats),
            mesh.property("tri").and_then(Value::ints),
        ) else {
            continue;
        };
        let positions: Vec<[f32; 3]> = positions.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
        if positions.is_empty() || indices.is_empty() {
            continue;
        }
        let indices: Vec<u16> = indices[..indices.len() / 3 * 3].iter().map(|&index| index as u16).collect();
        if let Some(index) = indices.iter().find(|&&index| usize::from(index) >= positions.len()) {
            return Err(invalid(format!(
                "its collision mesh indexes vertex {index} of {}",
                positions.len()
            )));
        }
        kept = Some(Collision { positions, indices });
    }
    Ok(kept)
}

fn skeleton_node(root: &Node) -> Option<&Node> {
    root.children_named("object")
        .flat_map(|object| &object.children)
        .filter_map(|shape| shape.child("skeleton"))
        .find(|skeleton| !skeleton.children.is_empty())
}

fn read_joints(node: &Node) -> Vec<Joint> {
    node.children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let tx = child.property("tx").and_then(Value::floats);
            let (parent, inverse_bind) = match tx {
                Some(tx) if child.property("ix").and_then(Value::ints).is_some() => (
                    child
                        .property("pa")
                        .and_then(Value::ints)
                        .and_then(|values| values.first())
                        .map(|&value| value as u8)
                        .filter(|&value| value != 255)
                        .map(u32::from),
                    Transform::from_columns(tx),
                ),
                _ => (Some(0), None),
            };
            Joint {
                name: child.name.clone(),
                index: index as u32,
                parent,
                inverse_bind,
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeJoint {
    pub name: String,
    pub parent: u8,
    pub inverse_bind: [f32; 12],
}

pub fn native_skeleton(root: &Node, scale: f32) -> io::Result<Vec<NativeJoint>> {
    let Some(skeleton) = skeleton_node(root) else {
        return Ok(Vec::new());
    };
    let mut joints = Vec::with_capacity(skeleton.children.len());
    for child in &skeleton.children {
        let mut joint = NativeJoint {
            name: child.name.chars().take(31).collect(),
            parent: 0,
            inverse_bind: [0.; 12],
        };
        if let (Some(tx), Some(_index)) = (
            child.property("tx").and_then(Value::floats),
            child.property("ix").and_then(Value::ints),
        ) {
            let tx = tx.get(..12).ok_or_else(|| {
                invalid(format!(
                    "native skeleton joint {:?} has fewer than12 tx floats",
                    child.name
                ))
            })?;
            joint.inverse_bind.copy_from_slice(tx);
            for value in &mut joint.inverse_bind[9..] {
                *value *= scale;
            }
            joint.parent = match child.property("pa") {
                None => 255,
                Some(value) => *value.ints().and_then(|v| v.first()).ok_or_else(|| {
                    invalid(format!(
                        "native skeleton joint {:?} has no parent integer",
                        child.name
                    ))
                })? as u8,
            };
        }
        joints.push(joint);
    }
    Ok(joints)
}

fn read_sub_mesh(shape: &str, node: &Node) -> io::Result<SubMesh> {
    let positions = vectors::<3>(shape, node, "p")?.unwrap_or_default();
    let normals = optional_vectors::<3>(shape, node, "n")?.unwrap_or_default();
    let tangents = optional_vectors::<4>(shape, node, "ta")?.unwrap_or_default();
    let uv0 = optional_vectors::<2>(shape, node, "u0")?.unwrap_or_default();
    let uv1 = optional_vectors::<2>(shape, node, "u1")?;
    let uv2 = optional_vectors::<2>(shape, node, "u2")?;
    let uv3 = optional_vectors::<2>(shape, node, "u3")?;
    let vertices = positions.len();
    let raw = node.property("tri").and_then(Value::ints).unwrap_or(&[]);
    if !raw.len().is_multiple_of(3) {
        return Err(invalid(format!(
            "the PDX mesh shape \"{shape}\" has {} triangle indices, not a multiple of three",
            raw.len()
        )));
    }
    let mut indices = Vec::with_capacity(raw.len());
    for &index in raw {
        let index = u32::try_from(index)
            .ok()
            .filter(|value| (*value as usize) < vertices)
            .ok_or_else(|| {
                invalid(format!(
                    "the PDX mesh shape \"{shape}\" indexes vertex {index} of {vertices}"
                ))
            })?;
        indices.push(index);
    }
    let bounds = node.child("aabb").and_then(|aabb| {
        let min = triple(aabb.property("min")?)?;
        let max = triple(aabb.property("max")?)?;
        Some(Bounds { min, max })
    });
    let material = node.child("material").map(|material| Material {
        shader: material.string("shader").map(str::to_owned),
        shader_file: None,
        diffuse: material.string("diff").unwrap_or_default().to_string(),
        normal: material.string("n").unwrap_or_default().to_string(),
        specular: material.string("spec").unwrap_or_default().to_string(),
    });
    let skin = node
        .child("skin")
        .map(|skin| read_skin(shape, skin, vertices))
        .transpose()?;
    Ok(SubMesh {
        child_index: 0,
        positions,
        normals,
        tangents,
        uv0,
        uv1,
        uv2,
        uv3,
        indices,
        bounds,
        material,
        skin,
    })
}

fn read_skin(shape: &str, node: &Node, vertices: usize) -> io::Result<Skin> {
    let raw_joints = node.property("ix").and_then(Value::ints).unwrap_or(&[]);
    let raw_weights = node.property("w").and_then(Value::floats).unwrap_or(&[]);
    let wanted = vertices * INFLUENCES;
    if raw_joints.len() != wanted || raw_weights.len() != wanted {
        return Err(invalid(format!(
            "the PDX mesh shape \"{shape}\" has {} skin indices and {} weights for {vertices} \
             vertices, not {wanted} of each",
            raw_joints.len(),
            raw_weights.len()
        )));
    }
    let mut joints = Vec::with_capacity(vertices);
    let mut weights = Vec::with_capacity(vertices);
    for vertex in 0..vertices {
        let mut slot_joints = [-1; INFLUENCES];
        let mut slot_weights = [0.0; INFLUENCES];
        for slot in 0..INFLUENCES {
            slot_joints[slot] = raw_joints[vertex * INFLUENCES + slot];
            slot_weights[slot] = raw_weights[vertex * INFLUENCES + slot];
        }
        joints.push(slot_joints);
        weights.push(slot_weights);
    }
    Ok(Skin {
        bones: node
            .property("bones")
            .and_then(Value::ints)
            .and_then(|values| values.first().copied())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(INFLUENCES as u32),
        joints,
        weights,
    })
}

fn optional_vectors<const N: usize>(
    shape: &str,
    node: &Node,
    name: &str,
) -> io::Result<Option<Vec<[f32; N]>>> {
    if node
        .property(name)
        .and_then(Value::floats)
        .is_some_and(|values| !values.len().is_multiple_of(N))
    {
        return Ok(Some(Vec::new()));
    }
    vectors(shape, node, name)
}

fn vectors<const N: usize>(
    shape: &str,
    node: &Node,
    name: &str,
) -> io::Result<Option<Vec<[f32; N]>>> {
    let Some(values) = node.property(name).and_then(Value::floats) else {
        return Ok(None);
    };
    if !values.len().is_multiple_of(N) {
        return Err(invalid(format!(
            "the PDX mesh shape \"{shape}\" has {} floats in \"{name}\", not a multiple of {N}",
            values.len()
        )));
    }
    let mut out = Vec::with_capacity(values.len() / N);
    for chunk in values.chunks_exact(N) {
        let mut vector = [0.0; N];
        vector.copy_from_slice(chunk);
        out.push(vector);
    }
    Ok(Some(out))
}

fn triple(value: &Value) -> Option<[f32; 3]> {
    let values = value.floats()?;
    Some([*values.first()?, *values.get(1)?, *values.get(2)?])
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod callback_id_tests;

pub trait MeshProducts {
    fn texture_index(&self) -> io::Result<std::sync::Arc<crate::mesh::TextureIndex>>;
    fn descriptor_roster(&self) -> io::Result<std::sync::Arc<crate::mesh::Roster>>;
    fn recorded_descriptor_roster(
        &self,
    ) -> io::Result<Option<std::sync::Arc<crate::mesh::Roster>>>;
}

impl MeshProducts for ResolvedLayers {
    fn texture_index(&self) -> io::Result<std::sync::Arc<crate::mesh::TextureIndex>> {
        self.retained_product(
            "model texture index",
            1,
            &[],
            crate::mesh::TextureIndex::read,
        )
    }

    fn descriptor_roster(&self) -> io::Result<std::sync::Arc<crate::mesh::Roster>> {
        self.retained_product("model descriptor roster", 1, &[], crate::mesh::Roster::read)
    }

    fn recorded_descriptor_roster(
        &self,
    ) -> io::Result<Option<std::sync::Arc<crate::mesh::Roster>>> {
        let Some(recorded) = self.without_overlays() else {
            return Ok(None);
        };
        recorded.descriptor_roster().map(Some)
    }
}
