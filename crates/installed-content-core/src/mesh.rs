//! The Clausewitz binary asset container (`.mesh`, `.anim`) and the `.gfx` and
//! `.asset` descriptors that name a mesh file for an entity.
//!
//! The container holds a depth-delimited node tree with named properties.
//! Each property's tag selects an integer, float or string stream, and its
//! count determines how many values follow. The parser retains node and
//! property order while checking every declared length against the remaining
//! bytes. Descriptors join the resulting assets to entity names, materials
//! and animation declarations; consumers resolve their files through the
//! recorded content stack.
//!
//! ```text
//! magic "@@b@", then tokens until EOF:
//!   0x5B ('[') x N, name bytes, 0x00     open a node at depth N (root = 0)
//!   0x21 ('!'), u8 len, name bytes,      a property of the open node
//!       'i' u32 count, count x i32
//!       'f' u32 count, count x f32
//!       's' u32 count, count x { u32 len, len bytes }
//! ```
//!
//! There is no close token: depth alone delimits, so the file is a pre-order
//! walk. [`parse`] returns that tree verbatim and [`read`] projects the
//! `.mesh` shape of it into typed vertex streams.
//!
//! Skins, skeletons and both halves of `.anim` are decoded, because the figure
//! the game draws for a division is a posed one. A `.mesh`'s own vertex stream
//! is its bind pose, which can differ from the displayed default pose; the
//! engine plays the entity's default state animation and skins the mesh by it.
//! [`SubMesh::skin`] carries the per-vertex influences, [`Mesh::skeleton`] the
//! joints, [`read_animation`] both the `[info]` rest pose and the `[samples]`
//! frames, and [`Mesh::palette`] composes the two into the matrix a linear
//! blend skinning pass multiplies a vertex by.
//!
//! The same joints place attachments. An attachment rig can carry a small
//! base mesh while its `attach` blocks supply the visible objects at named
//! joints. Each attached entity follows its parent's animated joint rather
//! than the base mesh's bind pose. An attachment hangs from [`Mesh::pose`],
//! which is the same joint transform the palette is built from, so an object
//! attached to a hand follows the skinned hand; see
//! [`Descriptors::resolve_entity`].
//!
//! **A still hour is [`FIRST_FRAME`].** A recording samples the game once an
//! hour and carries no animation clock, so there is no time to advance to: the
//! frame the engine shows an entity that has just entered its default state is
//! sample 0, and that is the one evaluated here.

use std::{collections::BTreeMap, io};

mod animation;
mod container;
mod descriptors;

pub use animation::{Animation, AnimationJoint, FIRST_FRAME, JointPose, read_animation};
pub use container::{Node, Value, parse};
pub use descriptors::{
    AnimationDefinition, Attachment, DEFAULT_STATE, Descriptors, EntityDefinition, EntityEvent,
    EntityGameData, EntityState, InheritedEntity, Locator, MeshDefinition, MeshSetting,
    NavalHitMissEffect, ResolvedEntity, ResolvedMesh, Roster, TextureIndex, balanced_blocks,
    lexical_names, parse_descriptors,
};

/// The four magic bytes every PDX binary asset starts with.
pub const MAGIC: [u8; 4] = *b"@@b@";

pub fn parse_in(layers: &crate::ResolvedLayers, path: &str) -> io::Result<std::sync::Arc<Node>> {
    layers.file_product(path.as_bytes(), "binary asset grammar", 1, &[], |bytes| parse(bytes))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// ----------------------------------------------------------------- transforms

/// An affine transform: three basis columns and a translation.
///
/// This is the layout a `tx` property carries. The twelve floats are four
/// triples; the first three are **columns** of the linear part, and the
/// fourth is the translation. A basis column gives the transformed unit axis,
/// so a point is the weighted sum of those columns plus the translation.
/// The same layout is used for inverse binds, joint poses and attachment
/// transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// `basis[c]` is where the `c`-th unit axis lands.
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

    /// The twelve floats of a `tx` property, four columns of three.
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

    /// A translation, a rotation quaternion `(x, y, z, w)` and a uniform scale,
    /// which is how `.anim` states a joint's rest pose.
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
        // Columns of the rotation matrix, then the uniform scale on each.
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

    /// `self` applied after `inner`: the transform that maps `inner`'s space
    /// through `self`'s.
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

    /// The determinant of the linear part.
    pub fn determinant(&self) -> f32 {
        let [a, b, c] = self.basis;
        a[0] * (b[1] * c[2] - b[2] * c[1]) - b[0] * (a[1] * c[2] - a[2] * c[1])
            + c[0] * (a[1] * b[2] - a[2] * b[1])
    }

    /// The inverse, or `None` when the linear part is singular or not finite.
    ///
    /// An all-zero `tx` has no inverse. Large basis entries can also produce
    /// a non-finite determinant or inverse, so those transforms follow the
    /// same absent-inverse path.
    pub fn inverse(&self) -> Option<Self> {
        let determinant = self.determinant();
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return None;
        }
        let [a, b, c] = self.basis;
        let inverse = 1.0 / determinant;
        // The inverse of the 3x3, as columns.
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

    /// The mean length of the three basis columns: the uniform scale a joint
    /// carries, which is 0 on a joint whose attachment the animation hides.
    pub fn uniform_scale(&self) -> f32 {
        self.basis
            .iter()
            .map(|axis| axis.iter().map(|v| v * v).sum::<f32>().sqrt())
            .sum::<f32>()
            / 3.0
    }
}

/// One `[skeleton]` joint, as the importer keeps it ([`read_tree`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Joint {
    pub name: String,
    /// The joint's place in its skeleton, which skin influences and `pa` name;
    /// the authored `ix` does not choose it.
    pub index: u32,
    /// `pa`, the parent's place; absent on a root.
    pub parent: Option<u32>,
    /// `tx`, the inverse bind matrix: world space to this joint's own space.
    /// Absent when the file declares none.
    pub inverse_bind: Option<Transform>,
}

/// A `[material]` node: the effect name and the three texture references.
/// Every name resolves relative to the `.mesh`'s own directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Material {
    /// The `Effect` in `gfx/FX/pdxmesh.shader`, e.g. `PdxMeshAdvancedSnow`.
    /// None is an absent key, distinct from an authored empty shader.
    pub shader: Option<String>,
    /// Nonempty meshsettings override of every variant's source.
    pub shader_file: Option<String>,
    pub diffuse: String,
    pub normal: String,
    pub specular: String,
}

/// An `[aabb]` node: the sub-mesh's object-space bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// How many joints one vertex of a `[skin]` is influenced by.
///
/// The streams are **always** four wide. `bones` is not that width: it gives
/// the authoring tool's influence cap, while `ix` and `w` each carry four
/// entries per vertex. A vertex can use fewer entries than the stream holds,
/// with its unused entries carrying zero weight. Reading `bones` as the
/// stride would move the start of the following vertex whenever the cap is
/// below four.
pub const INFLUENCES: usize = 4;

/// One `[skin]` node: which joints move a vertex and by how much.
///
/// A slot a vertex does not use is joint `-1` at weight 0. The four weights
/// normally sum to one, but a vertex can be bound to nothing at all: all
/// four slots are `-1` at weight 0. Such a vertex keeps its bind position
/// rather than collapsing onto the origin. Influence extraction ignores
/// unused slots and leaves the unbound case to the skinning consumer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skin {
    /// `bones`: the largest number of non-zero influences a vertex carries.
    pub bones: u32,
    /// `ix`, four per vertex. `-1` is an unused slot.
    pub joints: Vec<[i32; INFLUENCES]>,
    /// `w`, four per vertex, in the same slot order.
    pub weights: Vec<[f32; INFLUENCES]>,
}

impl Skin {
    /// The joints and weights of one vertex, unused slots dropped.
    ///
    /// Empty for a vertex bound to nothing, which is the caller's signal to
    /// leave it in its bind position.
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

/// One `[mesh]` node: a vertex stream set, its triangles and its material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubMesh {
    /// Ordinal among all shape children, before filtering to `[mesh]`.
    pub child_index: usize,
    pub positions: Vec<[f32; 3]>,
    /// Optional stream lengths may differ from vertex_count; mismatches are omitted.
    pub normals: Vec<[f32; 3]>,
    /// `ta`: tangent XYZ and handedness W; optional stream lengths may differ from vertex_count.
    pub tangents: Vec<[f32; 4]>,
    /// Optional stream lengths may differ from vertex_count.
    pub uv0: Vec<[f32; 2]>,
    /// `u1`: tree tint row/height jitter; presence does not guarantee vertex_count entries.
    pub uv1: Option<Vec<[f32; 2]>>,
    /// Extra packed coordinates; present streams may differ from vertex_count.
    pub uv2: Option<Vec<[f32; 2]>>,
    /// Extra packed coordinates; present streams may differ from vertex_count.
    pub uv3: Option<Vec<[f32; 2]>>,
    /// `tri`, three indices per triangle, each inside the vertex streams.
    pub indices: Vec<u32>,
    pub bounds: Option<Bounds>,
    pub material: Option<Material>,
    /// The `[skin]` node, when the sub-mesh declares one.
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

/// One shape node under `[object]`: a LOD level, or a whole sub-object on a
/// single-LOD mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub name: String,
    /// `lod`, absent on single-LOD meshes.
    pub lod: Option<i32>,
    pub meshes: Vec<SubMesh>,
}

/// A decoded `.mesh` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// `loddist`, ascending and starting at 0; empty on a single-LOD mesh.
    pub lod_distances: Vec<f32>,
    pub shapes: Vec<Shape>,
    /// The `[skeleton]` joints, in file order.
    pub skeleton: Vec<Joint>,
    /// The names under the root `[locator]` node, if any.
    pub locators: Vec<String>,
}

impl Mesh {
    /// The shapes to draw at each LOD level, outermost first.
    ///
    /// A shape that declares `lod` is one rung of a ladder. A mesh where **no**
    /// shape declares one is not a ladder at all: its shapes are sub-objects
    /// and they all draw together at the single level. An undrawable
    /// collision hull can precede a drawable shape, so child order alone
    /// cannot select a detail level. Treating those children as successive
    /// levels could hide the drawable shape at the nearest distance.
    /// Explicit `lod` properties supply the ladder's level numbers; shapes
    /// without one remain together when no shape declares a ladder.
    /// This also keeps multiple drawable sub-objects visible together.
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

    /// The joint of this name.
    pub fn joint(&self, name: &str) -> Option<&Joint> {
        self.skeleton.iter().find(|joint| joint.name == name)
    }

    /// The joint's bind-pose transform in the mesh's own space, composed from
    /// the inverse bind matrix the file carries.
    ///
    /// `tx` is the **world** inverse bind, not a local one: on
    /// `motorized_frame.mesh` the `infantry` joint's `tx` translates by
    /// `(-1.88863, 0, 0)` and the idle animation rests that joint at
    /// `(1.88863, 0, 0)`, so inverting `tx` alone reproduces the pose with no
    /// parent walk. `None` when the file declares no `tx` for the joint or
    /// declares a singular one.
    pub fn bind_pose(&self, name: &str) -> Option<Transform> {
        self.joint(name)?.inverse_bind?.inverse()
    }

    /// The joint's pose in the mesh's own space at one frame of an animation,
    /// composed down the skeleton's `pa` chain.
    ///
    /// The `.anim` states each joint **relative to its parent** and carries no
    /// parent of its own, so the two files are read together: the `[skeleton]`
    /// gives the tree and the animation the transform on each link. For a rig
    /// this is one link — every attachment joint of `motorized_frame`,
    /// `cavalry_frame`, `bicycle_frame` and `artillery_frame` hangs off an
    /// identity root — and for a rifleman it is eight, from the hip out to
    /// `Right_Hand_node`.
    ///
    /// A joint the animation does not name contributes its own bind transform
    /// relative to its parent's, so a partial `.anim` still places its joint.
    /// `None` when the skeleton has no joint of that name.
    ///
    /// This is where an `attach` block hangs, and it is the left half of the
    /// skinning matrix [`Mesh::palette`] builds, so an attachment and the joint
    /// that carries it move together by construction.
    pub fn pose(&self, animation: &Animation, frame: u32, name: &str) -> Option<Transform> {
        let index = self.joint(name)?.index;
        let locals = locals_by_name(animation, frame);
        self.world_pose(&locals, index, &mut self.memo(), 0)
    }

    /// World transforms of a sampled local pose, indexed by skeleton joint index.
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

    /// The skinning matrix of every joint: its pose at this frame times its own
    /// inverse bind, indexed by the joint's place in its skeleton.
    ///
    /// This is what a linear blend skinning pass multiplies a bind-pose vertex
    /// by. A joint with no usable `tx` takes the identity rather than a
    /// singular or non-finite matrix. A vertex weighted to such a joint
    /// stays where the file put it instead of inheriting an unusable
    /// transform. Valid joints still combine their animated pose with
    /// their inverse bind.
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

    /// An empty memo for [`Mesh::world_pose`], one slot per joint.
    fn memo(&self) -> Vec<Option<Option<Transform>>> {
        vec![None; self.skeleton.len()]
    }

    /// One joint's pose in the mesh's own space, memoised by its position in
    /// [`Mesh::skeleton`]. `depth` bounds a `pa` chain that loops.
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
        // A joint the animation names takes its transform; one it does not
        // contributes its own bind transform relative to its parent's.
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

/// A mesh/animation name join, retained while both immutable inputs are in use.
/// Duplicate animation names retain their last supplied local; duplicate skeleton
/// indices resolve to the first skeleton record, as the ordinary pose query does.
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

    /// The same parent walk and composition order as [`Mesh::sampled_pose`],
    /// with its name and index joins already resolved and its buffers reused.
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

/// One frame of an animation, keyed by joint name.
///
/// The skeleton and the animation are two files and are joined by name.
/// A partial or reordered `.anim` can have different joint positions, so
/// joining by position would place a joint's transform on its neighbour.
/// The name join preserves each animation joint's own transform.
fn locals_by_name(animation: &Animation, frame: u32) -> BTreeMap<&str, Transform> {
    animation
        .joints
        .iter()
        .map(|joint| joint.name.as_str())
        .zip(animation.frame(frame))
        .collect()
}

/// Decodes a `.mesh` file into typed vertex streams.
///
/// Errors are named `InvalidData` for anything [`parse`] refuses, for a file
/// with no `[object]` node, for vertex streams of disagreeing lengths, for a
/// stream whose float count is not a multiple of its component count, and for a
/// triangle index outside the vertex streams.
pub fn read(bytes: &[u8]) -> io::Result<Mesh> {
    read_tree(&parse(bytes)?)
}

pub fn read_animation_in(layers: &crate::ResolvedLayers, path: &str) -> io::Result<std::sync::Arc<Animation>> {
    layers.file_product(path.as_bytes(), "skeletal animation", 1, &[], |bytes| {
        read_animation(bytes)
    })
}

/// Projects a parsed container, preserving `read`'s post-parse validation order.
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
        // A `[skeleton]` beside the shapes is no shape, and the importer reads no joints from it.
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
            // A property-only child of `[object]` is not a shape.
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

/// The skeleton the importer keeps: the first nonempty `[skeleton]` of a shape
/// (depth 3), in source order across every `[object]`.
fn skeleton_node(root: &Node) -> Option<&Node> {
    root.children_named("object")
        .flat_map(|object| &object.children)
        .filter_map(|shape| shape.child("skeleton"))
        .find(|skeleton| !skeleton.children.is_empty())
}

/// Every joint of the kept skeleton in source order, each at its place ([`native_skeleton`]):
/// with `ix` and `tx` its parent is `pa` as the importer's byte holds it (255 is none), without
/// them parent 0 and no bind.
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

/// Keeps the first nonempty skeleton in source order.
/// `ix` must exist but its value does not select the destination record.
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
        // Mesh joint names retain the first 31 Latin-1 bytes.
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
    // Wrong-sized optional streams are omitted; retain their presence here
    // because declaration bits clear only for an absent property.
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

/// The `[skin]` node's four-wide influence streams.
///
/// Errors are named `InvalidData` for a stream that does not cover every vertex
/// four times over; see [`INFLUENCES`] for why the width is not `bones`.
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

/// Keeps optional property presence while omitting ragged vectors.
fn optional_vectors<const N: usize>(
    shape: &str,
    node: &Node,
    name: &str,
) -> io::Result<Option<Vec<[f32; N]>>> {
    // Ragged streams are omitted while their presence bit stays set.
    if node
        .property(name)
        .and_then(Value::floats)
        .is_some_and(|values| !values.len().is_multiple_of(N))
    {
        return Ok(Some(Vec::new()));
    }
    vectors(shape, node, name)
}

/// Splits one float property into fixed-width vectors.
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
