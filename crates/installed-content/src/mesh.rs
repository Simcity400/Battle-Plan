//! The Clausewitz binary asset container (`.mesh`, `.anim`) and the `.gfx` and
//! `.asset` descriptors that name a mesh file for an entity.
//!
//! The specification is research report R3
//! `C:\hoi4_scratch\replay_map\research\R3-meshes-trees-cities.md`, section 1
//! for the container and the descriptors and "Algorithms for implementers"
//! section A for the byte table; section numbers below refer to it. The
//! container reading is not inferred: R3's `r3/pdxmesh.py` parses every shipped
//! `.mesh` and `.anim` without an unknown byte, and this module is a port of it
//! with the bounds checks a reader of untrusted bytes needs.
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
//! `.mesh` shape of it (section 1.2) into typed vertex streams.
//!
//! Skins, skeletons and both halves of `.anim` are decoded, because the figure
//! the game draws for a division is a posed one. A `.mesh`'s own vertex stream
//! is its bind pose, and `GER_infantry.mesh`'s bind pose is a T-pose; the
//! engine plays the entity's default state animation and skins the mesh by it.
//! [`SubMesh::skin`] carries the per-vertex influences, [`Mesh::skeleton`] the
//! joints, [`read_animation`] both the `[info]` rest pose and the `[samples]`
//! frames, and [`Mesh::palette`] composes the two into the matrix a linear
//! blend skinning pass multiplies a vertex by.
//!
//! The same joints place attachments, because several shipped unit entities are
//! attachment rigs: `GER_motorized_entity`'s own mesh is a three-vertex plane
//! and the truck and rifleman the game draws for a motorized division are its
//! `attach` blocks at the rig's `vehicle` and `infantry` joints. An attachment
//! hangs from [`Mesh::pose`], which is the same joint transform the palette is
//! built from, so a rifle in a hand follows the skinned hand; see
//! [`Descriptors::resolve_entity`].
//!
//! **A still hour is [`FIRST_FRAME`].** A recording samples the game once an
//! hour and carries no animation clock, so there is no time to advance to: the
//! frame the engine shows an entity that has just entered its default state is
//! sample 0, and that is the one evaluated here.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{ResolvedLayers, parser};

/// The four magic bytes every PDX binary asset starts with (section 1.1).
pub const MAGIC: [u8; 4] = *b"@@b@";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// One property payload. The container carries only these three types.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Ints(Vec<i32>),
    Floats(Vec<f32>),
    /// Strings are NUL-terminated inside their own length, and the terminator
    /// is stripped here.
    Strings(Vec<String>),
}

impl Value {
    pub fn ints(&self) -> Option<&[i32]> {
        match self {
            Self::Ints(values) => Some(values),
            _ => None,
        }
    }

    pub fn floats(&self) -> Option<&[f32]> {
        match self {
            Self::Floats(values) => Some(values),
            _ => None,
        }
    }

    pub fn strings(&self) -> Option<&[String]> {
        match self {
            Self::Strings(values) => Some(values),
            _ => None,
        }
    }
}

/// One node of the container tree, with its properties in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// The bracketed name, or the empty string for the synthetic root.
    pub name: String,
    pub properties: Vec<(String, Value)>,
    pub children: Vec<Node>,
}

impl Node {
    /// The first property with this name.
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The first child node with this name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child node with this name, in file order.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    fn string(&self, name: &str) -> Option<&str> {
        self.property(name)?.strings()?.first().map(String::as_str)
    }
}

/// A cursor over the token stream that never reads past the end.
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| invalid("a PDX asset length overflows"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.truncated(count))?;
        self.offset = end;
        Ok(slice)
    }

    fn truncated(&self, wanted: usize) -> io::Error {
        invalid(format!(
            "a PDX asset ends after {} bytes while reading {wanted} more at offset {}",
            self.bytes.len(),
            self.offset
        ))
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A declared element count, refused when the payload cannot fit in what is
    /// left. Without this a corrupt count would reserve gigabytes.
    fn count(&mut self, element_bytes: usize) -> io::Result<usize> {
        let count = self.u32()? as usize;
        let needed = count
            .checked_mul(element_bytes)
            .ok_or_else(|| invalid("a PDX asset element count overflows"))?;
        if needed > self.remaining() {
            return Err(invalid(format!(
                "a PDX asset declares {count} elements of {element_bytes} bytes at offset {}, \
                 past its {} byte end",
                self.offset,
                self.bytes.len()
            )));
        }
        Ok(count)
    }

    /// An ASCII name terminated by NUL, as a bracketed node name carries.
    fn nul_terminated(&mut self) -> io::Result<String> {
        let start = self.offset;
        let end = self.bytes[start..]
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| {
                invalid(format!(
                    "a PDX asset node name at offset {start} has no end"
                ))
            })?;
        let name = latin1(&self.bytes[start..start + end]);
        self.offset = start + end + 1;
        Ok(name)
    }
}

/// The container's names are plain bytes; every shipped one is ASCII, and a
/// byte above 127 becomes the Latin-1 character rather than a parse error.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// Parses a `.mesh` or `.anim` container into its node tree (section 1.1).
///
/// Errors are named `InvalidData` for a wrong magic, an unknown property type,
/// a byte that starts no token, a depth that skips a level, or any length that
/// runs past the end of the input.
pub fn parse(bytes: &[u8]) -> io::Result<Node> {
    if bytes.get(..4) != Some(&MAGIC) {
        return Err(invalid(
            "not a PDX binary asset: the file does not start with the magic \"@@b@\"",
        ));
    }
    let mut reader = Reader {
        bytes,
        offset: MAGIC.len(),
    };
    // The open path from the root downwards; a node is attached to its parent
    // when a later token closes it, which is what "depth alone delimits" means.
    let mut stack = vec![Node::default()];
    while let Some(byte) = reader.peek() {
        match byte {
            b'[' => {
                let mut depth = 0usize;
                while reader.peek() == Some(b'[') {
                    reader.offset += 1;
                    depth += 1;
                }
                let name = reader.nul_terminated()?;
                if depth == 0 || depth > stack.len() {
                    return Err(invalid(format!(
                        "the PDX asset node \"{name}\" opens at depth {depth} under an open path \
                         of depth {}",
                        stack.len() - 1
                    )));
                }
                close_to(&mut stack, depth);
                stack.push(Node {
                    name,
                    ..Node::default()
                });
            }
            b'!' => {
                reader.offset += 1;
                let length = reader.byte()? as usize;
                let key = latin1(reader.take(length)?);
                let kind = reader.byte()?;
                let value = match kind {
                    b'i' => {
                        let count = reader.count(4)?;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(reader.u32()? as i32);
                        }
                        Value::Ints(values)
                    }
                    b'f' => {
                        let count = reader.count(4)?;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(f32::from_bits(reader.u32()?));
                        }
                        Value::Floats(values)
                    }
                    b's' => {
                        // Each string carries its own length prefix, so only the
                        // prefixes themselves can be pre-checked here.
                        let count = reader.count(4)?;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            let length = reader.count(1)?;
                            let bytes = reader.take(length)?;
                            let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(length);
                            values.push(latin1(&bytes[..end]));
                        }
                        Value::Strings(values)
                    }
                    other => {
                        return Err(invalid(format!(
                            "the PDX asset property \"{key}\" has the unknown type byte {other:#04x}"
                        )));
                    }
                };
                if let Some(node) = stack.last_mut() {
                    node.properties.push((key, value));
                }
            }
            other => {
                return Err(invalid(format!(
                    "the byte {other:#04x} at offset {} starts no PDX asset token",
                    reader.offset
                )));
            }
        }
    }
    close_to(&mut stack, 1);
    stack
        .pop()
        .ok_or_else(|| invalid("the PDX asset reader lost its root node"))
}

/// Attaches open nodes to their parents until the stack is `depth` deep.
fn close_to(stack: &mut Vec<Node>, depth: usize) {
    while stack.len() > depth {
        let Some(node) = stack.pop() else { return };
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            // Unreachable while `depth >= 1`; pushing the node back keeps the
            // reader from losing it if that ever changes.
            None => {
                stack.push(node);
                return;
            }
        }
    }
}

// ----------------------------------------------------------------- transforms

/// An affine transform: three basis columns and a translation.
///
/// This is the layout a `tx` property carries. The twelve floats are four
/// triples; the **column** reading of the first three is measured rather than
/// assumed. Applying `GER_infantry.mesh`'s `RightHand` inverse bind to the 80
/// vertices that joint alone skins puts them a mean 0.496 from the joint's own
/// origin read as columns and 1.338 read as rows, and `RightUpLeg` 0.863
/// against 1.029.
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
    /// which is how `.anim` states a joint's rest pose (section 1.3).
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
    /// Both cases occur in shipped rigs: `artillery_frame.mesh` writes an
    /// all-zero `tx` for 33 of its 41 joints and `cavalry_frame.mesh` writes
    /// `1e12` on the diagonal of two of its five.
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

/// One `[skeleton]` joint (section 1.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Joint {
    pub name: String,
    /// `ix`, the joint's own index.
    pub index: u32,
    /// `pa`, the parent's index; absent on a root.
    pub parent: Option<u32>,
    /// `tx`, the inverse bind matrix: world space to this joint's own space.
    /// Absent when the file declares none.
    pub inverse_bind: Option<Transform>,
}

/// A `[material]` node: the effect name and the three texture references
/// (section 1.2). Every name resolves relative to the `.mesh`'s own directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Material {
    /// The `Effect` in `gfx/FX/pdxmesh.shader`, e.g. `PdxMeshAdvancedSnow`.
    /// None is an absent key, distinct from an authored empty shader.
    pub shader: Option<String>,
    /// Nonempty meshsettings override of every variant's source (14224BB00).
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
/// The streams are **always** four wide. `bones` is not that width: across the
/// 341 skinned sub-meshes of the shipped `gfx` tree it takes the values 1, 2, 3
/// and 4 while `ix` and `w` are `4 * V` long in every one of them, and no
/// vertex of a sub-mesh ever carries more non-zero weights than its own `bones`
/// says. `bones` is the authoring tool's influence cap, the stream width is
/// fixed; reading `bones` as the stride mis-strides 269 of the 341.
pub const INFLUENCES: usize = 4;

/// One `[skin]` node: which joints move a vertex and by how much (section 1.2).
///
/// A slot a vertex does not use is joint `-1` at weight 0 — measured: across
/// every shipped skin the largest weight on a `-1` slot is exactly 0. The four
/// weights of a vertex sum to 1 in 338 of the 341 sub-meshes; in the other
/// three (`generic_railway_gun.mesh`, `algerie_heavy_cruiser.mesh`,
/// `train_locomotive_1_equipment_2.mesh`) a handful of vertices are bound to
/// nothing at all — all four slots `-1` at weight 0 — and those keep their bind
/// position rather than collapsing onto the origin.
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
    /// Ordinal among all shape children, before filtering to `[mesh]` (14224F250).
    pub child_index: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// `ta`: tangent in `xyz` and the bitangent handedness in `w`.
    pub tangents: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    /// `u1`, which the tree shader reads as the tint row and the height jitter.
    /// Absent on meshes that declare only one texture coordinate set.
    pub uv1: Option<Vec<[f32; 2]>>,
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

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether the sub-mesh carries the streams a lit pass needs.
    ///
    /// A shipped `.mesh` can hold collision geometry beside its drawn shapes  -
    /// `GER_infantry.mesh` carries a `collision_boxShape` of eight positions
    /// and no normals, tangents or texture coordinates  -  and that geometry is
    /// not something a shader with a normal map can draw.
    pub fn is_drawable(&self) -> bool {
        let vertices = self.vertex_count();
        vertices > 0
            && self.normals.len() == vertices
            && self.tangents.len() == vertices
            && self.uv0.len() == vertices
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
    /// and they all draw together at the single level, which is what
    /// `artillery_frame.mesh` needs — its first shape is an undrawable
    /// `Collision` hull and its drawn plane is the second, so reading the
    /// shapes as LOD 0 and LOD 1 draws nothing at any distance. Two shipped
    /// meshes have that shape (`artillery_frame.mesh` and
    /// `landmark_eiffel_tower.mesh`) and exactly one has two drawable sub-
    /// objects (`ger_battleship_bismark.mesh`, which no layer here draws);
    /// every LOD mesh the tree, sprawl and building layers ask for declares
    /// `lod` on every shape, so this reading moves only those.
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
    /// inverse bind, indexed by the joint's `ix`.
    ///
    /// This is what a linear blend skinning pass multiplies a bind-pose vertex
    /// by. A joint with no usable `tx` takes the identity rather than a
    /// singular or 1e12 matrix — `artillery_frame.mesh` writes an all-zero `tx`
    /// for 33 of its 41 joints and `GER_infantry.mesh` writes 1e12 on
    /// `Left_Hand_node`'s diagonal — so a vertex weighted to one of them stays
    /// where the file put it instead of leaving the screen.
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

/// One frame of an animation, keyed by joint name.
///
/// The skeleton and the animation are two files and are joined by name: R3 has
/// the `[info]` joints "in skeleton index order", which every shipped pair
/// honours, but a partial or reordered `.anim` would place a joint's transform
/// on its neighbour if position were trusted.
fn locals_by_name(animation: &Animation, frame: u32) -> BTreeMap<&str, Transform> {
    animation
        .joints
        .iter()
        .map(|joint| joint.name.as_str())
        .zip(animation.frame(frame))
        .collect()
}

/// Decodes a `.mesh` file into typed vertex streams (section 1.2).
///
/// Errors are named `InvalidData` for anything [`parse`] refuses, for a file
/// with no `[object]` node, for vertex streams of disagreeing lengths, for a
/// stream whose float count is not a multiple of its component count, and for a
/// triangle index outside the vertex streams.
pub fn read(bytes: &[u8]) -> io::Result<Mesh> {
    let root = parse(bytes)?;
    let object = root
        .child("object")
        .ok_or_else(|| invalid("a PDX mesh carries no [object] node"))?;
    let lod_distances = object
        .property("loddist")
        .and_then(Value::floats)
        .unwrap_or(&[])
        .to_vec();
    let mut skeleton = Vec::new();
    let mut shapes = Vec::new();
    for child in &object.children {
        if child.name == "skeleton" {
            skeleton.extend(read_joints(child));
            continue;
        }
        // `[skeleton]` sits beside the shape's `[mesh]` nodes at depth 3
        // (section 1.2), which is where every shipped unit mesh puts it; the
        // depth-2 form above is accepted for robustness.
        if let Some(node) = child.child("skeleton") {
            skeleton.extend(read_joints(node));
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
        skeleton,
        locators,
    })
}

/// The joints under one `[skeleton]` node, skipping any child that declares no
/// `ix`.
fn read_joints(node: &Node) -> Vec<Joint> {
    node.children
        .iter()
        .filter_map(|child| {
            let index = child.property("ix")?.ints()?.first().copied()?;
            Some(Joint {
                name: child.name.clone(),
                index: u32::try_from(index).ok()?,
                parent: child
                    .property("pa")
                    .and_then(Value::ints)
                    .and_then(|values| values.first().copied())
                    .and_then(|value| u32::try_from(value).ok()),
                inverse_bind: child
                    .property("tx")
                    .and_then(Value::floats)
                    .and_then(Transform::from_columns),
            })
        })
        .collect()
}

///142250010/1423997A0 keep the first nonempty skeleton in source order.
///`ix` must exist but its value does not select the destination record.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeJoint {
    pub name: String,
    pub parent: u8,
    pub inverse_bind: [f32; 12],
}

pub fn native_skeleton(root: &Node, scale: f32) -> io::Result<Vec<NativeJoint>> {
    for object in root.children_named("object") {
        for shape in &object.children {
            let Some(skeleton) = shape.child("skeleton") else {
                continue;
            };
            let mut joints = Vec::with_capacity(skeleton.children.len());
            for child in &skeleton.children {
                //142250010 copies31 Latin1 bytes and terminates;1423997A0 copies32.
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
            if !joints.is_empty() {
                return Ok(joints);
            }
        }
    }
    Ok(Vec::new())
}

/// A decoded `.anim` file: the `[info]` rest pose and the `[samples]` frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    /// `fps`, the sample rate.
    pub fps: f32,
    /// `sa`, the number of frames.
    pub samples: u32,
    /// One entry per joint, in skeleton index order.
    pub joints: Vec<AnimationJoint>,
    /// `[samples]` `t`: three floats per frame per joint that declares `t`,
    /// **frame-major** and in joint order.
    translations: Vec<f32>,
    /// `[samples]` `q`: four floats, the same way.
    rotations: Vec<f32>,
    /// `[samples]` `s`: one float, the same way.
    scales: Vec<f32>,
}

impl Animation {
    /// The rest pose of one joint, as a transform in its parent's space.
    ///
    /// This is the `[info]` block's own `t`, `q` and `s`, read verbatim. It is
    /// also [`FIRST_FRAME`]: across all 474 shipped `.anim` files that carry a
    /// `[samples]` block, every animated joint's frame 0 equals its `[info]`
    /// values to the last bit, in all three channels, with no exception. The
    /// two readings are kept apart anyway, because a modded file need not
    /// honour it and because [`Animation::local`] is what an animating pass
    /// would call.
    pub fn rest(&self, name: &str) -> Option<Transform> {
        let joint = self.joints.iter().find(|joint| joint.name == name)?;
        Some(Transform::from_trs(
            joint.translation,
            joint.rotation,
            joint.scale,
        ))
    }

    /// How many joints declare each of the three channels, in `tqs` order.
    ///
    /// The `[samples]` streams carry only the joints that declare the channel,
    /// so these are the strides one frame advances by.
    pub fn channel_joints(&self) -> [usize; 3] {
        let count = |channel: char| {
            self.joints
                .iter()
                .filter(|joint| joint.channels.contains(channel))
                .count()
        };
        [count('t'), count('q'), count('s')]
    }

    /// Every joint's transform in its parent's space at one frame, in file
    /// order.
    ///
    /// A channel the joint does not animate contributes its `[info]` value, and
    /// so does any channel whose stream is short of this frame, which is what
    /// makes a file with no `[samples]` block at all evaluate to its rest pose.
    pub fn frame(&self, frame: u32) -> Vec<Transform> {
        self.frame_trs(frame)
            .iter()
            .map(JointPose::transform)
            .collect()
    }

    /// Authored channel values before quaternion-to-matrix conversion.
    pub fn frame_trs(&self, frame: u32) -> Vec<JointPose> {
        let [strides_t, strides_q, strides_s] = self.channel_joints();
        let frame = frame as usize;
        let (mut slot_t, mut slot_q, mut slot_s) = (0usize, 0usize, 0usize);
        let mut out = Vec::with_capacity(self.joints.len());
        for joint in &self.joints {
            let mut translation = joint.translation;
            let mut rotation = joint.rotation;
            let mut scale = joint.scale;
            if joint.channels.contains('t') {
                let at = (frame * strides_t + slot_t) * 3;
                if let Some(values) = self.translations.get(at..at + 3) {
                    translation = [values[0], values[1], values[2]];
                }
                slot_t += 1;
            }
            if joint.channels.contains('q') {
                let at = (frame * strides_q + slot_q) * 4;
                if let Some(values) = self.rotations.get(at..at + 4) {
                    rotation = [values[0], values[1], values[2], values[3]];
                }
                slot_q += 1;
            }
            if joint.channels.contains('s') {
                let at = frame * strides_s + slot_s;
                if let Some(value) = self.scales.get(at) {
                    scale = *value;
                }
                slot_s += 1;
            }
            out.push(JointPose {
                translation,
                rotation,
                scale,
            });
        }
        out
    }

    /// 142399290: loop period is the last sample's time, (samples - 1) / fps;
    /// a single-sample clip lasts one frame interval.
    pub fn duration(&self) -> f32 {
        if self.fps > 0.0 {
            self.samples.saturating_sub(1).max(1) as f32 / self.fps
        } else {
            0.0
        }
    }

    pub fn sample(&self, seconds: f32, looping: bool) -> Vec<JointPose> {
        let duration = self.duration();
        let time = if looping && duration > 0.0 {
            seconds.rem_euclid(duration)
        } else {
            seconds.clamp(0.0, duration)
        };
        let frame = time * self.fps;
        let a = (frame.floor() as u32).min(self.samples.saturating_sub(1));
        let b = (a + 1).min(self.samples.saturating_sub(1));
        let weight = frame.fract();
        self.frame_trs(a)
            .into_iter()
            .zip(self.frame_trs(b))
            .map(|(a, b)| a.blend(b, weight))
            .collect()
    }

    /// One joint's transform in its parent's space at one frame.
    pub fn local(&self, name: &str, frame: u32) -> Option<Transform> {
        let index = self.joints.iter().position(|joint| joint.name == name)?;
        self.frame(frame).into_iter().nth(index)
    }
}

/// Local translation, quaternion and uniform scale. Blend before composing parents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointPose {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: f32,
}
impl JointPose {
    pub fn transform(&self) -> Transform {
        Transform::from_trs(self.translation, self.rotation, self.scale)
    }
    pub fn blend(self, mut other: Self, t: f32) -> Self {
        if t <= 0.0 {
            return self;
        }
        if t >= 1.0 {
            return other;
        }
        let mut dot: f32 = self
            .rotation
            .iter()
            .zip(other.rotation)
            .map(|(a, b)| a * b)
            .sum();
        if dot < 0.0 {
            other.rotation.iter_mut().for_each(|q| *q = -*q);
            dot = -dot;
        }
        let (a, b) = if dot > 0.9995 {
            (1.0 - t, t)
        } else {
            let theta = dot.clamp(-1.0, 1.0).acos();
            (
                ((1.0 - t) * theta).sin() / theta.sin(),
                (t * theta).sin() / theta.sin(),
            )
        };
        let mut rotation = std::array::from_fn(|i| a * self.rotation[i] + b * other.rotation[i]);
        let norm = rotation.iter().map(|q| q * q).sum::<f32>().sqrt();
        if norm > 0.0 {
            rotation.iter_mut().for_each(|q| *q /= norm);
        }
        Self {
            translation: std::array::from_fn(|i| {
                self.translation[i] + (other.translation[i] - self.translation[i]) * t
            }),
            rotation,
            scale: self.scale + (other.scale - self.scale) * t,
        }
    }
}

/// The frame of a state animation a still hour shows.
///
/// A recording holds one sample an hour and no animation clock, so an entity
/// has no elapsed time in its default state: what the engine draws at time 0 is
/// sample 0. Nothing here advances it.
pub const FIRST_FRAME: u32 = 0;

/// One joint's rest pose inside an `.anim` (section 1.3).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationJoint {
    pub name: String,
    /// `sa`, the animated channels: a subset of `"tqs"` in that order. Empty
    /// means the joint is static, which every attachment joint of the four
    /// shipped rigs is.
    pub channels: String,
    /// `t`, the rest translation.
    pub translation: [f32; 3],
    /// `q`, the rest rotation quaternion `(x, y, z, w)`.
    pub rotation: [f32; 4],
    /// `s`, the rest uniform scale. **Zero hides the joint's attachment**:
    /// that is how a rifleman's idle animation holds one of his four rifles
    /// and how `cavalry_frame`'s idle drops its separate rider and horse in
    /// favour of the combined model on its `cavalry` joint.
    pub scale: f32,
}

/// Decodes the `[info]` and `[samples]` blocks of a `.anim` file (section 1.3).
///
/// Errors are named `InvalidData` for anything [`parse`] refuses, for a file
/// with no `[info]` node, and for a `[samples]` stream whose length is not the
/// frame count times the number of joints declaring that channel.
pub fn read_animation(bytes: &[u8]) -> io::Result<Animation> {
    let root = parse(bytes)?;
    let info = root
        .child("info")
        .ok_or_else(|| invalid("a PDX animation carries no [info] node"))?;
    let single = |name: &str| {
        info.property(name)
            .and_then(Value::floats)
            .and_then(|values| values.first().copied())
    };
    let joints = info
        .children
        .iter()
        .map(|child| {
            let translation = child
                .property("t")
                .and_then(triple)
                .unwrap_or([0.0, 0.0, 0.0]);
            let rotation = child
                .property("q")
                .and_then(Value::floats)
                .and_then(|values| values.get(..4))
                .map_or([0.0, 0.0, 0.0, 1.0], |values| {
                    [values[0], values[1], values[2], values[3]]
                });
            AnimationJoint {
                name: child.name.clone(),
                channels: child.string("sa").unwrap_or_default().to_string(),
                translation,
                rotation,
                scale: child
                    .property("s")
                    .and_then(Value::floats)
                    .and_then(|values| values.first().copied())
                    .unwrap_or(1.0),
            }
        })
        .collect();
    let mut animation = Animation {
        fps: single("fps").unwrap_or(0.0),
        samples: info
            .property("sa")
            .and_then(Value::ints)
            .and_then(|values| values.first().copied())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        joints,
        ..Animation::default()
    };
    let Some(samples) = root.child("samples") else {
        return Ok(animation);
    };
    let stream = |name: &str| {
        samples
            .property(name)
            .and_then(Value::floats)
            .unwrap_or(&[])
            .to_vec()
    };
    animation.translations = stream("t");
    animation.rotations = stream("q");
    animation.scales = stream("s");
    let [joints_t, joints_q, joints_s] = animation.channel_joints();
    let frames = animation.samples as usize;
    for (name, have, want) in [
        ("t", animation.translations.len(), 3 * frames * joints_t),
        ("q", animation.rotations.len(), 4 * frames * joints_q),
        ("s", animation.scales.len(), frames * joints_s),
    ] {
        if have != want {
            return Err(invalid(format!(
                "a PDX animation of {frames} frames declares {} joints with a \"{name}\" channel, \
                 so its [samples] \"{name}\" needs {want} floats and has {have}",
                match name {
                    "t" => joints_t,
                    "q" => joints_q,
                    _ => joints_s,
                }
            )));
        }
    }
    Ok(animation)
}

fn read_sub_mesh(shape: &str, node: &Node) -> io::Result<SubMesh> {
    let positions = vectors::<3>(shape, node, "p")?.unwrap_or_default();
    let normals = vectors::<3>(shape, node, "n")?.unwrap_or_default();
    let tangents = vectors::<4>(shape, node, "ta")?.unwrap_or_default();
    let uv0 = vectors::<2>(shape, node, "u0")?.unwrap_or_default();
    let uv1 = vectors::<2>(shape, node, "u1")?;
    let vertices = positions.len();
    // A stream the file does not declare at all is legitimate: shipped meshes
    // carry collision hulls (`GER_infantry.mesh`'s `collision_boxShape`) that
    // hold only positions and triangles, and the last LOD of `beech.mesh`
    // declares no `u1`. A stream that *is* declared has to cover every vertex.
    let mut disagreeing = Vec::new();
    for (name, count) in [
        ("n", normals.len()),
        ("ta", tangents.len()),
        ("u0", uv0.len()),
        ("u1", uv1.as_ref().map_or(vertices, Vec::len)),
    ] {
        if count != vertices && count != 0 {
            disagreeing.push(format!("{name} has {count}"));
        }
    }
    if !disagreeing.is_empty() {
        return Err(invalid(format!(
            "the PDX mesh shape \"{shape}\" has {vertices} positions but {}",
            disagreeing.join(", ")
        )));
    }
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
        indices,
        bounds,
        material,
        skin,
    })
}

/// The `[skin]` node's four-wide influence streams (section 1.2).
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

// ---------------------------------------------------------------- descriptors

/// One `meshsettings` block: a per (shape, material index) override of the
/// `.mesh`'s own textures and effect (section 1.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshSetting {
    /// The shape node name inside the `.mesh`.
    pub name: String,
    /// Raw child ordinal within the named shape; exactly -1 matches every ordinal.
    pub index: i32,
    pub diffuse: Option<String>,
    pub normal: Option<String>,
    pub specular: Option<String>,
    pub shader: Option<String>,
    pub shader_file: Option<String>,
}

impl Default for MeshSetting {
    fn default() -> Self {
        // SMeshData construction in 1422F8720; signed reader 142251AE0(+D4).
        Self {
            name: String::new(),
            index: -1,
            diffuse: None,
            normal: None,
            specular: None,
            shader: None,
            shader_file: None,
        }
    }
}

impl MeshSetting {
    /// 14224F250: an empty name matches immediately, regardless of index.
    pub fn applies(&self, shape: &str, child_index: usize) -> bool {
        self.name.is_empty()
            || (self.name == shape
                && (self.index == -1 || usize::try_from(self.index).ok() == Some(child_index)))
    }
}

/// One `pdxmesh` block of a `gfx/entities/*.gfx` file (section 1.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshDefinition {
    pub name: String,
    /// The `.mesh` path, relative to the content root.
    pub file: String,
    /// `scale`, default 1.0.
    pub scale: f32,
    pub settings: Vec<MeshSetting>,
    /// `animation = { id = … type = … }`: the animation **name** this mesh
    /// plays for each state animation id, in file order.
    pub animations: Vec<(String, String)>,
}

impl MeshDefinition {
    /// 14224F250 selects the first matching row. 14224CC00/14224BB00 apply only
    /// nonempty texture/shader overrides; later matching rows are never merged.
    pub fn material(&self, shape: &str, sub: &SubMesh) -> Material {
        let mut material = sub.material.clone().unwrap_or_default();
        if let Some(setting) = self
            .settings
            .iter()
            .find(|s| s.applies(shape, sub.child_index))
        {
            for (target, source) in [
                (&mut material.diffuse, &setting.diffuse),
                (&mut material.normal, &setting.normal),
                (&mut material.specular, &setting.specular),
            ] {
                if let Some(value) = source.as_ref().filter(|value| !value.is_empty()) {
                    target.clone_from(value);
                }
            }
            for (target, source) in [
                (&mut material.shader, &setting.shader),
                (&mut material.shader_file, &setting.shader_file),
            ] {
                if let Some(value) = source.as_ref().filter(|value| !value.is_empty()) {
                    *target = Some(value.clone());
                }
            }
        }
        material
    }
}

/// One `attach` block of an `entity` (section 1.5).
///
/// The block is `attach = { name = "<own name>" <joint> = "<entity>" }`: the
/// `name` is the attachment's own handle, which `propagate_state` addresses and
/// which a `clone` overrides by, and the one other key is a joint of the
/// parent's skeleton verbatim — `Right_Hand_node`, `Root_node_2`, `vehicle`,
/// `infantry`, `horse`, `artillery`, `bike` are all joint names in the
/// matching `.mesh`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    /// The attachment's own handle.
    pub name: String,
    /// The parent skeleton's joint it hangs from.
    pub joint: String,
    /// The entity attached there.
    pub entity: String,
}

impl Attachment {
    /// The name `propagate_state` addresses the attachment by: its own
    /// handle, or its joint when the block names no handle.
    pub fn handle(&self) -> &str {
        if self.name.is_empty() {
            &self.joint
        } else {
            &self.name
        }
    }
}

/// One `entity` block of a `gfx/entities/*.asset` file (section 1.5).
///
/// The `event` and particle machinery is still dropped; nothing here fires an
/// event or advances a clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityDefinition {
    pub name: String,
    /// `pdxmesh`, the key of a [`MeshDefinition`].
    pub mesh: Option<String>,
    /// `clone`, the entity this one inherits from before its own overrides.
    pub clone: Option<String>,
    pub scale: Option<f32>,
    /// `attach`, in file order.
    pub attachments: Vec<Attachment>,
    /// `default_state`; the engine's own default is `idle`.
    pub default_state: Option<String>,
    /// `state = { name = … animation = … }` pairs, in file order. A state name
    /// occurs several times when the engine picks between weighted variants;
    /// the first is taken.
    pub states: Vec<EntityState>,
    pub get_state_from_parent: Option<bool>,
    pub locators: Vec<(String, Locator)>,
    pub game_data: Option<Result<EntityGameData, String>>,
}

///140B65040: allocated on the first entity-level game_data field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityGameData {
    pub texture_anim_speed: f32,
    pub atlas_size: [i32; 2],
}

impl Default for EntityGameData {
    fn default() -> Self {
        Self {
            texture_anim_speed: 0.,
            atlas_size: [1, 1],
        }
    }
}

fn entity_game_data(items: &[parser::Item]) -> Option<Result<EntityGameData, String>> {
    let fields: Vec<_> = items
        .iter()
        .filter(|i| i.key.as_deref() == Some(b"game_data"))
        .filter_map(|i| i.value.items())
        .flatten()
        .collect();
    if fields.is_empty() {
        return None;
    }
    let mut data = EntityGameData::default();
    for field in fields {
        match field.key.as_deref() {
            Some(b"texture_anim_speed") => {
                let Some(speed) = field.value.scalar_str().and_then(|v| v.parse().ok()) else {
                    return Some(Err(
                        "entity texture_anim_speed is not a literal float".into()
                    ));
                };
                data.texture_anim_speed = speed;
            }
            Some(b"atlas_size") => {
                let Some(values) = field.value.items() else {
                    return Some(Err("entity atlas_size is not an integer vector".into()));
                };
                if values.len() > 2 {
                    return Some(Err(
                        "entity atlas_size leaves its two native integer fields".into(),
                    ));
                }
                for (index, value) in values.iter().enumerate() {
                    //1402C7A50 accepts ordered x/y labels as well as bare values.
                    if value
                        .key
                        .as_deref()
                        .is_some_and(|key| key != [b"x".as_slice(), b"y".as_slice()][index])
                    {
                        return Some(Err(
                            "entity atlas_size component labels are out of native order".into(),
                        ));
                    }
                    let Some(value) = value.value.scalar_str().and_then(|v| v.parse().ok()) else {
                        return Some(Err(
                            "entity atlas_size component is not a literal integer".into()
                        ));
                    };
                    data.atlas_size[index] = value;
                }
            }
            _ => {} //Native logs unknown keys after allocating the defaults.
        }
    }
    Some(Ok(data))
}

/// One entity `locator`: a transform in entity space, or in the space of its
/// `parent_joint`. `SEntityLocatorReader::ReadMember` (`14233BE00`) reads
/// `name`, `position`, `rotation` (degrees), `scale` (default 1) and
/// `parent_joint`; the lexer registers `locator` as token 0x1FF and
/// `parent_joint` as 0x200 (`14008AA8B`..`14008AAE7`). `14227D8C0` resolves
/// the joint name; one the skeleton lacks is logged and the locator stays in
/// entity space. No vanilla entity names a joint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Locator {
    pub transform: Transform,
    pub parent_joint: Option<String>,
}

/// Authored state variants retain their order and events, including states with no clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityState {
    pub name: String,
    pub animation: String,
    pub looping: bool,
    pub next_state: Option<String>,
    pub chance: f32,
    pub blend: f32,
    pub speed: f32,
    pub start: f32,
    pub random_start: f32,
    pub propagate: Vec<(String, String)>,
    pub events: Vec<EntityEvent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityEvent {
    pub time: f32,
    pub node: String,
    pub light: Option<String>,
    pub particle: Option<String>,
    pub keep_particle: bool,
    pub trigger_once: bool,
}

fn number(items: &[parser::Item], key: &[u8], fallback: f32) -> f32 {
    text(items, key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

/// One `animation = { name = … file = … }` block of a `gfx/models/**/*.asset`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnimationDefinition {
    pub name: String,
    /// The `.anim` path, already resolved against the descriptor's own
    /// directory and relative to the content root.
    pub file: String,
}

/// What a name resolved to: the `.mesh` to load, its world scale and its
/// material overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMesh<'a> {
    pub definition: &'a MeshDefinition,
    /// `entity.scale` times `pdxmesh.scale`.
    ///
    /// UNVERIFIED (section 1.5): whether the two compose or the entity's wins.
    /// In shipped data only one of the two is ever set for a map object, so on
    /// this content the two readings agree; every `mapobject_*` sets only the
    /// `pdxmesh` scale.
    pub scale: f32,
}

/// The `pdxmesh` and `entity` blocks of a set of descriptor files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descriptors {
    pub meshes: BTreeMap<String, MeshDefinition>,
    pub entities: BTreeMap<String, EntityDefinition>,
    /// `animation` blocks, keyed by the name a `pdxmesh` refers to them by.
    pub animations: BTreeMap<String, AnimationDefinition>,
}

/// An entity with its `clone` chain already applied (section 1.5).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InheritedEntity<'a> {
    /// `pdxmesh`, from the nearest link that declares one.
    pub mesh: Option<&'a str>,
    /// `scale`, from the nearest link that declares one.
    pub scale: Option<f32>,
    /// `default_state`, from the nearest link that declares one.
    pub default_state: Option<&'a str>,
    /// Every attachment reachable through the chain, merged down it as each
    /// clone merges its own (`14227CEF0`): a declaration replaces the held
    /// one of its slot where it stands, a new slot is appended.
    pub attachments: Vec<&'a Attachment>,
    /// State name to animation id, the nearest declaration winning.
    pub states: BTreeMap<&'a str, Vec<&'a EntityState>>,
    pub get_state_from_parent: Option<bool>,
    pub locators: BTreeMap<&'a str, &'a Locator>,
    pub game_data: Option<&'a Result<EntityGameData, String>>,
}

impl Descriptors {
    /// Resolves an entity's `clone` chain into one flattened definition.
    ///
    /// **The engine's `clone` copies the whole entity and then applies the
    /// overrides**, so a field the child does not declare is the parent's even
    /// when the child declares some other field. The shipped data needs that
    /// reading and the frames confirm it: `GER_infantry_entity` is
    /// `clone = "infantry_rifle_entity"` plus its own `pdxmesh` and its own
    /// five `attach` handles, and it declares no `scale`, no `default_state`
    /// and no `idle` state; the game draws it at `infantry_rifle_entity`'s
    /// `scale = 0.8` (`pairD_in10.png`: the game's helmet is 31.7 device pixels
    /// across and the unscaled figure's 40) and plays that entity's idle
    /// animation. A field-by-field override also explains every `attach`
    /// handle in the file: `GER_artillery_entity` clones `artillery_entity` and
    /// redeclares exactly the two handles `artillery` and `infantry` that it
    /// wants to point elsewhere.
    ///
    /// The walk is bounded by the number of entities, so a cyclic `clone`
    /// terminates.
    pub fn inherit(&self, name: &str) -> Option<InheritedEntity<'_>> {
        let mut out = InheritedEntity::default();
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        let mut current = self.entities.get(name)?;
        let mut chain = Vec::new();
        for _ in 0..=self.entities.len() {
            chain.push(current);
            out.mesh = out.mesh.or(current.mesh.as_deref());
            out.scale = out.scale.or(current.scale);
            out.game_data = out.game_data.or(current.game_data.as_ref());
            out.default_state = out.default_state.or(current.default_state.as_deref());
            out.get_state_from_parent = out.get_state_from_parent.or(current.get_state_from_parent);
            for (name, locator) in &current.locators {
                out.locators.entry(name).or_insert(locator);
            }
            let mut local = BTreeMap::<&str, Vec<&EntityState>>::new();
            for state in &current.states {
                local.entry(&state.name).or_default().push(state);
            }
            for (name, variants) in local {
                out.states.entry(name).or_insert(variants);
            }
            let Some(parent) = current.clone.as_deref() else {
                break;
            };
            if seen.insert(parent, ()).is_some() {
                break;
            }
            let Some(parent) = self.entities.get(parent) else {
                break;
            };
            current = parent;
        }
        // From the farthest ancestor down, as each clone's merge left them.
        for link in chain.into_iter().rev() {
            for attachment in &link.attachments {
                match out
                    .attachments
                    .iter()
                    .position(|held| same_slot(held, attachment))
                {
                    Some(at) => out.attachments[at] = attachment,
                    None => out.attachments.push(attachment),
                }
            }
        }
        Some(out)
    }

    /// Resolves an entity name to its mesh, world scale, attachments and the
    /// animation its default state plays.
    ///
    /// A name that is a `pdxmesh` and not an entity resolves to that mesh with
    /// no attachments, which is what every `mapobject_<paletteIndex>` the tree
    /// layer asks for is.
    pub fn resolve_entity(&self, name: &str) -> Option<ResolvedEntity<'_>> {
        let Some(inherited) = self.inherit(name) else {
            let definition = self.meshes.get(name)?;
            return Some(ResolvedEntity {
                definition,
                scale: definition.scale,
                attachments: Vec::new(),
                animation: None,
            });
        };
        let definition = self.meshes.get(inherited.mesh?)?;
        let state = inherited.default_state.unwrap_or(DEFAULT_STATE);
        let animation = inherited
            .states
            .get(state)
            .and_then(|variants| variants.first())
            .and_then(|state| {
                let id = &state.animation;
                definition
                    .animations
                    .iter()
                    .find(|(declared, _)| declared == id)
            })
            .and_then(|(_, name)| self.animations.get(name.as_str()));
        Some(ResolvedEntity {
            definition,
            scale: inherited.scale.unwrap_or(1.0) * definition.scale,
            attachments: inherited.attachments,
            animation,
        })
    }
}

/// The state an entity that declares no `default_state` is in.
pub const DEFAULT_STATE: &str = "idle";

/// What an entity name resolved to, with its `clone` chain applied.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedEntity<'a> {
    pub definition: &'a MeshDefinition,
    /// `entity.scale * pdxmesh.scale`.
    pub scale: f32,
    pub attachments: Vec<&'a Attachment>,
    /// The `.anim` the entity's default state plays, when one resolves.
    pub animation: Option<&'a AnimationDefinition>,
}

impl Descriptors {
    /// Resolves an entity or mesh name to its `.mesh` and world scale.
    ///
    /// **Entities are tried first.** The two tables share a namespace and the
    /// shipped content collides in it: `gfx/entities/buildings.gfx:4-8`
    /// declares a *pdxmesh* named `westerngfx_house_1_1` and
    /// `buildings.asset:6-10` declares an *entity* of the same name whose only
    /// content is `pdxmesh = "westerngfx_house_1_1"` and `scale = 1.4`. Taking
    /// the mesh first loses that scale, and the sprawl houses come out at 1.0
    /// against the game's 1.4 (or 1.8 and 2.0 for the taller rings). Every
    /// name that exists in only one table resolves the same either way, which
    /// includes every `mapobject_<paletteIndex>` the tree layer asks for
    /// (R3 section 3.1): those are pdxmesh names with no entity.
    ///
    /// An entity name follows its whole `clone` chain, field by field; see
    /// [`Descriptors::inherit`].
    pub fn resolve(&self, name: &str) -> Option<ResolvedMesh<'_>> {
        self.resolve_entity(name).map(|entity| ResolvedMesh {
            definition: entity.definition,
            scale: entity.scale,
        })
    }
}

/// Reads the blocks of one already-read descriptor file. `path` is the file's
/// own path, which an `animation` block's `file` resolves against.
pub fn parse_descriptors(bytes: &[u8], path: &str) -> io::Result<Descriptors> {
    let document = parser::parse(bytes)?;
    let mut out = Descriptors::default();
    let _ = collect(&document, path, &mut out);
    Ok(out)
}

/// Adds one file's blocks; returns the names that replaced an earlier block.
fn collect(document: &parser::Document, path: &str, out: &mut Descriptors) -> Vec<String> {
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    let mut replaced = Vec::new();
    for item in &document.items {
        match item.key.as_deref() {
            Some(b"objectTypes") => {
                for block in item.value.items().unwrap_or(&[]) {
                    if block.key.as_deref() == Some(b"pdxmesh")
                        && let Some(definition) = mesh_definition(&block.value)
                        && let Some(old) = out.meshes.insert(definition.name.clone(), definition)
                    {
                        replaced.push(old.name);
                    }
                }
            }
            Some(b"entity") => {
                if let Some(definition) = entity_definition(&item.value)
                    && let Some(old) = out.entities.insert(definition.name.clone(), definition)
                {
                    replaced.push(old.name);
                }
            }
            Some(b"animation") => {
                if let Some(definition) = animation_definition(&item.value, directory)
                    && let Some(old) = out.animations.insert(definition.name.clone(), definition)
                {
                    replaced.push(old.name);
                }
            }
            _ => {}
        }
    }
    replaced
}

/// Drops unmatched closing braces and closes blocks left open at the end,
/// leaving strings and comments alone. Several shipped descriptors need it,
/// and the game loads them; callers use it only after a parse fails.
pub fn balanced_blocks(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let (mut depth, mut quoted, mut comment, mut escaped) = (0usize, false, false, false);
    for &b in bytes {
        if comment {
            out.push(b);
            if b == b'\n' {
                comment = false;
            }
            continue;
        }
        if quoted {
            out.push(b);
            if b == b'"' && !escaped {
                quoted = false;
            }
            escaped = b == b'\\' && !escaped;
            continue;
        }
        match b {
            b'#' => comment = true,
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(b);
    }
    out.push(b'\n');
    out.extend(std::iter::repeat_n(b'}', depth));
    out
}

/// Every descriptor the engine loads, registered as the engine registers it.
///
/// Three registries, each filled in its own pass; no file name is derived from
/// an identifier, and stray braces are balanced as the engine's reader
/// tolerates them:
/// - Meshes. `SGfxFileReader::ReadToken` (`1422264C0`) reads each object of
///   the `.gfx` files in the particle catalog's order (startup, `interface`,
///   then `gfx`) and prepends it to its name's bucket (`1406B1110`); the
///   lookup (`142223D70`) takes the first exact match, so the last object of a
///   name wins. `142272140` binds an entity's `pdxmesh` only to an object of
///   type 0x180, so a name whose last object is an `arrowType`,
///   `animatedmaptext` or `pdxparticle` is no mesh.
/// - Animations and lights. The asset factory (`142218E80` from
///   `CApplication::Initialize` `142218780`, entity flag clear) reads every
///   `gfx/**/*.asset` in `Vfs_EnumerateRecursive` order, and `142338C30`
///   registers each `animation` whose file loads (`14224D620`: it opens and
///   holds `[info]` and `[samples]`, `14224D6A0`), a later one replacing the
///   clip (`142399290`), and each `light` (token 0x54, `SAnimatedLightReader`),
///   a later one of the name rewriting the record (`1423C96F0`).
/// - Entities. `LoadAssets` (`142218E80` from `CGameApplication::Initialize`
///   `1401806B0`, entity flag set) reads the same files again for `entity`
///   blocks only: see [`Entities`].
#[derive(Clone, Debug, Default)]
pub struct Roster {
    pub descriptors: Descriptors,
    /// Each `light` block's fields by name, the last declaration's.
    pub lights: BTreeMap<String, Vec<parser::Item>>,
    /// What entity registration logs: dropped clones and failed mesh bindings, each with why.
    pub logged: Vec<String>,
    /// Files that do not parse even when closed, with the error.
    pub unreadable: Vec<String>,
    /// Every word such a file holds ([`lexical_names`]), with the first
    /// such file. Which of its blocks the engine still registers is
    /// unverified, so each could be a declaration a consumer cannot see. A
    /// clone whose parent is such a word is one too, with the parent's file:
    /// the parent it copied may be that file's.
    pub unreadable_names: BTreeMap<String, String>,
}

/// The objects of the mesh registry (`1422264C0` tokens 0x180, 0x57, 0x10E
/// and 0x181); only the first is a mesh.
const OBJECTS: [&str; 4] = ["pdxmesh", "arrowType", "animatedmaptext", "pdxparticle"];

/// `14224D6A0` copies an `animation` name, and `1423C96F0` a light's, into 64
/// bytes, and `14233BF40` an
/// entity's name, `clone` and `pdxmesh` into 256, each ending with a NUL. A
/// longer name would be cut, which is not modelled.
const ANIMATION_NAME: usize = 63;
const ENTITY_NAME: usize = 255;

impl Roster {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut roster = Self::default();
        let interface = layers.descriptor_paths("interface", &[".gfx"])?;
        let gfx = layers.descriptor_paths("gfx", &[".gfx", ".asset"])?;
        let mut objects = BTreeMap::new();
        for path in crate::particles::object_file_order(&interface, &gfx) {
            if let Some(document) = roster.parse(layers, &path, &OBJECTS)? {
                add_objects(&document, &mut objects);
            }
        }
        roster.descriptors.meshes = objects
            .into_iter()
            .filter_map(|(name, mesh)| Some((name, mesh?)))
            .collect();
        let mut animations = BTreeMap::<String, Vec<AnimationDefinition>>::new();
        let mut entities = Entities::default();
        for path in crate::defines::load_order("gfx/", ".asset", gfx.iter().map(String::as_str)) {
            let Some(document) = roster.parse(layers, &path, &["entity", "animation", "light"])?
            else {
                continue;
            };
            let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
            for item in &document.items {
                if item.keyword_is("animation") {
                    let Some(animation) = animation_definition(&item.value, directory) else {
                        continue;
                    };
                    if animation.name.len() > ANIMATION_NAME {
                        return Err(invalid(format!(
                            "{path}: animation {} is longer than the {ANIMATION_NAME} bytes the engine keeps",
                            animation.name
                        )));
                    }
                    animations
                        .entry(animation.name.clone())
                        .or_default()
                        .push(animation);
                } else if item.keyword_is("entity") {
                    entities.declare(&item.value, &path, &mut roster)?;
                } else if item.keyword_is("light")
                    && let Some(fields) = item.value.items()
                {
                    let name = last_text(fields, "name").unwrap_or_default();
                    if name.len() > ANIMATION_NAME {
                        return Err(invalid(format!(
                            "{path}: light {name} is longer than the {ANIMATION_NAME} bytes the engine keeps"
                        )));
                    }
                    roster.lights.insert(name, fields.to_vec());
                }
            }
        }
        for (name, declarations) in animations {
            let animation = loaded_animation(layers, declarations)?;
            roster.descriptors.animations.insert(name, animation);
        }
        roster.descriptors.entities = entities
            .0
            .into_iter()
            .map(|(name, (definition, _))| (name, definition))
            .collect();
        Ok(roster)
    }

    /// The file's document when it holds any of `words` (without case, as the
    /// lexer folds keywords), else `None`; a file that does not parse is
    /// recorded as unreadable.
    fn parse(
        &mut self,
        layers: &ResolvedLayers,
        path: &str,
        words: &[&str],
    ) -> io::Result<Option<parser::Document>> {
        let bytes = layers.read_uncovered(path)?;
        if !words.iter().any(|word| {
            bytes
                .windows(word.len())
                .any(|window| window.eq_ignore_ascii_case(word.as_bytes()))
        }) {
            return Ok(None);
        }
        match parser::parse(&bytes).or_else(|_| parser::parse(&balanced_blocks(&bytes))) {
            Ok(document) => Ok(Some(document)),
            Err(error) => {
                for name in lexical_names(&bytes) {
                    self.unreadable_names
                        .entry(name)
                        .or_insert_with(|| path.to_owned());
                }
                self.unreadable.push(format!("{path} ({error})"));
                Ok(None)
            }
        }
    }
}

/// Registers one `.gfx` file's objects: the latest object of a name is the one
/// found, a mesh or, for another kind, `None`.
fn add_objects(
    document: &parser::Document,
    objects: &mut BTreeMap<String, Option<MeshDefinition>>,
) {
    for item in document
        .items
        .iter()
        .filter(|item| item.keyword_is("objectTypes"))
    {
        for object in item.value.items().unwrap_or(&[]) {
            if object.keyword_is(OBJECTS[0]) {
                if let Some(mesh) = mesh_definition(&object.value) {
                    objects.insert(mesh.name.clone(), Some(mesh));
                }
            } else if OBJECTS[1..].iter().any(|kind| object.keyword_is(kind))
                && let Some(name) = object
                    .value
                    .items()
                    .and_then(|fields| last_text(fields, "name"))
            {
                objects.insert(name, None);
            }
        }
    }
}

/// The declaration of an animation name the engine keeps: the last whose file
/// loads (`14224D620`). Declarations of one file need no reading, and when
/// none loads no clip does either, so the last stands.
fn loaded_animation(
    layers: &ResolvedLayers,
    mut declarations: Vec<AnimationDefinition>,
) -> io::Result<AnimationDefinition> {
    let last = declarations.pop().expect("a name has a declaration");
    if declarations.iter().all(|earlier| earlier.file == last.file) {
        return Ok(last);
    }
    for declaration in std::iter::once(&last).chain(declarations.iter().rev()) {
        let bytes = match layers.read_uncovered(&declaration.file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if parse(&bytes)
            .is_ok_and(|root| root.child("info").is_some() && root.child("samples").is_some())
        {
            return Ok(declaration.clone());
        }
    }
    Ok(last)
}

/// The entity registry as `LoadAssets` fills it, block by block, each name with
/// its definition and stored version.
///
/// A plain block is `create_or_reuse` (`14227D8C0`); a block with a `clone` is
/// `update` (`14227CEF0`) of a copy of the parent as registered at that moment
/// (`entity_definition_lookup` `14227E5F0`, exact name), and is dropped when no
/// parent is registered yet ("Couldn't find parent clone entity",
/// `142338C30`). Either way a name already registered keeps its definition when
/// the block's `version` (`ParseTokenInt32`) is lower than the stored one, and
/// is replaced otherwise. The stored version is the one the name was created
/// with, and a clone stores its parent's (`14226C110` and `14226D5E0` copy it;
/// neither path writes the block's).
#[derive(Default)]
struct Entities(BTreeMap<String, (EntityDefinition, i32)>);

impl Entities {
    fn declare(
        &mut self,
        value: &parser::Value,
        path: &str,
        roster: &mut Roster,
    ) -> io::Result<()> {
        let Some(fields) = value.items() else {
            return Ok(());
        };
        let Some(name) = last_text(fields, "name").filter(|name| !name.is_empty()) else {
            return Ok(());
        };
        let clone = last_text(fields, "clone").filter(|parent| !parent.is_empty());
        let mesh = last_text(fields, "pdxmesh");
        for text in std::iter::once(&name).chain(&clone).chain(&mesh) {
            if text.len() > ENTITY_NAME {
                return Err(invalid(format!(
                    "{path}: entity {name} names {text}, longer than the {ENTITY_NAME} bytes the engine keeps"
                )));
            }
        }
        let version = fields
            .iter()
            .rev()
            .find(|item| item.keyword_is("version"))
            .and_then(|item| item.value.scalar())
            .map_or(0, crate::fixed5::decimal_i32);
        if let Some(file) = clone
            .as_ref()
            .and_then(|parent| roster.unreadable_names.get(parent))
        {
            let file = file.clone();
            roster.unreadable_names.entry(name.clone()).or_insert(file);
        }
        let parent = match &clone {
            None => None,
            Some(parent) => match self.0.get(parent) {
                Some(registered) => Some(registered.clone()),
                None => {
                    roster.logged.push(format!(
                        "{name} in {path}: the engine drops it, finding no entity {parent} to clone yet"
                    ));
                    return Ok(());
                }
            },
        };
        let registered = self.0.get(&name);
        let stored = registered.map(|(_, stored)| *stored);
        if stored.is_some_and(|stored| version < stored) {
            return Ok(());
        }
        let Some(mut definition) = entity_definition(value) else {
            return Ok(());
        };
        definition.name = name.clone();
        definition.clone = None;
        // 142272140 binds the named object only when it is a `pdxmesh`, and
        // otherwise logs and leaves the bindings as they are: the parent's
        // copy (14226C110, 14226D5E0) or the definition being redefined,
        // whose bindings create_or_reuse (14227D8C0) does not clear.
        let kept = match &parent {
            Some((parent, _)) => parent.mesh.clone(),
            None => registered.and_then(|(held, _)| held.mesh.clone()),
        };
        definition.mesh = match mesh.filter(|mesh| !mesh.is_empty()) {
            Some(mesh) if roster.descriptors.meshes.contains_key(&mesh) => Some(mesh),
            Some(mesh) => {
                roster.logged.push(format!(
                    "{name} in {path}: no pdxmesh {mesh} is registered, so the engine keeps its mesh binding"
                ));
                kept
            }
            None => kept,
        };
        let entry = match parent {
            Some((parent, version)) => (overlay(parent, definition), version),
            None => (definition, stored.unwrap_or(version)),
        };
        self.0.insert(name, entry);
        Ok(())
    }
}

/// A clone as `14227CEF0` builds it: the parent's definition, each field the
/// block declares replacing the parent's as [`Descriptors::inherit`] reads a
/// chain, and its attachments merged ([`merge_attachments`]).
fn overlay(parent: EntityDefinition, child: EntityDefinition) -> EntityDefinition {
    let mut attachments = parent.attachments;
    merge_attachments(&mut attachments, child.attachments);
    let mut locators: Vec<(String, Locator)> = Vec::new();
    for (name, locator) in child.locators.into_iter().chain(parent.locators) {
        if !locators.iter().any(|(held, _)| *held == name) {
            locators.push((name, locator));
        }
    }
    let own: BTreeSet<String> = child
        .states
        .iter()
        .map(|state| state.name.clone())
        .collect();
    let mut states = child.states;
    states.extend(
        parent
            .states
            .into_iter()
            .filter(|state| !own.contains(&state.name)),
    );
    EntityDefinition {
        name: child.name,
        mesh: child.mesh.or(parent.mesh),
        clone: None,
        scale: child.scale.or(parent.scale),
        attachments,
        default_state: child.default_state.or(parent.default_state),
        states,
        get_state_from_parent: child.get_state_from_parent.or(parent.get_state_from_parent),
        locators,
        game_data: child.game_data.or(parent.game_data),
    }
}

/// `14227CEF0`'s attachment merge: each attachment the clone declares
/// replaces, where it stands, the held one of the same name (of the same
/// joint when both names are empty), and is appended otherwise.
fn merge_attachments(held: &mut Vec<Attachment>, declared: Vec<Attachment>) {
    for attachment in declared {
        match held.iter().position(|slot| same_slot(slot, &attachment)) {
            Some(at) => held[at] = attachment,
            None => held.push(attachment),
        }
    }
}

/// Whether a clone's attachment replaces a held one (`14227CEF0`): the same
/// name, and the same joint when both names are empty.
fn same_slot(held: &Attachment, declared: &Attachment) -> bool {
    held.name == declared.name && (!held.name.is_empty() || held.joint == declared.joint)
}

/// The text of the last `key` a block holds, as the reader's last write of
/// that field leaves it.
fn last_text(fields: &[parser::Item], key: &str) -> Option<String> {
    fields
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .and_then(|item| item.value.scalar_str())
        .map(unquote)
}

/// Every name a descriptor file could declare, read without parsing it:
/// each scalar the shared lexer reads ([`parser::scalars`], which keeps
/// scanning past the error that made the parse fail), decoded and trimmed
/// as a parsed name is (`scalar_str`, then [`unquote`]).
pub fn lexical_names(bytes: &[u8]) -> BTreeSet<String> {
    parser::scalars(bytes)
        .iter()
        .filter_map(|word| std::str::from_utf8(word).ok())
        .map(unquote)
        .collect()
}

/// `InitTextureLookup(gfx/models)`: `142251F00` indexes every `.dds` and
/// `.tga` below `gfx/models` by its stored basename.
#[derive(Clone, Debug, Default)]
pub struct TextureIndex {
    paths: BTreeMap<String, String>,
}

impl TextureIndex {
    /// `142251F00` walks `gfx/models` with `Vfs_EnumerateRecursive`
    /// (`1424C7630`, no filter, any depth), in [`crate::defines::load_order`].
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let paths = layers.descriptor_paths("gfx/models", &[".dds", ".tga"])?;
        Ok(Self::new(crate::defines::load_order(
            "gfx/models/",
            "",
            paths.iter().map(String::as_str),
        )))
    }

    /// Indexes `paths` in enumeration order: each basename keeps the first
    /// file (the FNV-1a and byte-compare map of `142251F00`; a later file of
    /// that basename is only reported, `1422527C0`).
    pub fn new(paths: impl IntoIterator<Item = String>) -> Self {
        let mut index = Self::default();
        for path in paths {
            // The suffix test is byte-exact (ends_with 1424B6AB0).
            if !(path.ends_with(".dds") || path.ends_with(".tga")) {
                continue;
            }
            let basename = path.rsplit('/').next().unwrap_or(&path).to_owned();
            index.paths.entry(basename).or_insert(path);
        }
        index
    }

    /// The one path `14224CC00` requests for a material texture.
    /// `142251230` looks the authored string up byte-exactly after spelling
    /// its first `.tga` as `.dds`; a miss keeps the mesh directory plus the
    /// authored name.
    pub fn resolve(&self, directory: &str, authored: &str) -> String {
        let key = authored.replacen(".tga", ".dds", 1);
        match self.paths.get(&key) {
            Some(path) => path.clone(),
            None if directory.is_empty() => authored.to_owned(),
            None => format!("{directory}/{authored}"),
        }
    }
}

fn animation_definition(value: &parser::Value, directory: &str) -> Option<AnimationDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    let file = unquote(text(items, b"file")?);
    if name.is_empty() || file.is_empty() {
        return None;
    }
    Some(AnimationDefinition {
        name,
        // 142338C30 joins the descriptor's directory, `/` and the file,
        // whatever the file holds.
        file: if directory.is_empty() {
            file
        } else {
            format!("{directory}/{file}")
        },
    })
}

fn mesh_definition(value: &parser::Value) -> Option<MeshDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    // PhysFS drops a path's leading slashes (`sanitizePlatformIndependentPath`),
    // so `/gfx/models/...`, as the shipped supply-train meshes spell it, is
    // the content-root path.
    let file = unquote(text(items, b"file")?)
        .trim_start_matches('/')
        .to_owned();
    if name.is_empty() || file.is_empty() {
        return None;
    }
    let settings = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"meshsettings"))
        .filter_map(|item| mesh_setting(&item.value))
        .collect();
    let animations = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"animation"))
        .filter_map(|item| {
            let block = item.value.items()?;
            Some((unquote(text(block, b"id")?), unquote(text(block, b"type")?)))
        })
        .collect();
    Some(MeshDefinition {
        name,
        file,
        scale: text(items, b"scale")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(1.0),
        settings,
        animations,
    })
}

fn mesh_setting(value: &parser::Value) -> Option<MeshSetting> {
    let items = value.items()?;
    let mut setting = MeshSetting::default();
    // 142251AE0 writes fields in reader order; failed numeric conversions retain the field.
    for item in items {
        let Some(value) = item.value.scalar_str() else {
            continue;
        };
        match item.key.as_deref() {
            Some(b"name") => setting.name = unquote(value),
            Some(b"index") => {
                if let crate::crt::Scanned::Stored(index) = crate::crt::scan_i32(value.as_bytes()) {
                    setting.index = index;
                }
            }
            Some(b"texture_diffuse") => setting.diffuse = Some(unquote(value)),
            Some(b"texture_normal") => setting.normal = Some(unquote(value)),
            Some(b"texture_specular") => setting.specular = Some(unquote(value)),
            Some(b"shader") => setting.shader = Some(unquote(value)),
            // Token 0x2A6 at 14008C426; reader142251AE0 writes SMeshData+A8.
            Some(b"shader_file") => setting.shader_file = Some(unquote(value)),
            _ => {}
        }
    }
    Some(setting)
}

fn entity_definition(value: &parser::Value) -> Option<EntityDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    if name.is_empty() {
        return None;
    }
    let attachments = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"attach"))
        .filter_map(|item| {
            let block = item.value.items()?;
            // The joint is whichever key is not `name`; the shipped blocks
            // carry exactly those two.
            let (joint, entity) = block.iter().find_map(|field| {
                let key = field.key.as_deref()?;
                if key == b"name" {
                    return None;
                }
                Some((
                    unquote(std::str::from_utf8(key).ok()?),
                    unquote(field.value.scalar_str()?),
                ))
            })?;
            Some(Attachment {
                name: text(block, b"name").map(unquote).unwrap_or_default(),
                joint,
                entity,
            })
        })
        .filter(|attachment| !attachment.joint.is_empty() && !attachment.entity.is_empty())
        .collect();
    let states = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"state"))
        .filter_map(|item| {
            let block = item.value.items()?;
            Some(EntityState {
                name: unquote(text(block, b"name")?),
                animation: text(block, b"animation").map(unquote).unwrap_or_default(),
                looping: text(block, b"looping") != Some("no"),
                next_state: text(block, b"next_state").map(unquote),
                chance: number(block, b"chance", 1.0),
                blend: number(block, b"animation_blend_time", 0.0),
                speed: number(block, b"animation_speed", 1.0),
                start: number(block, b"animation_start_time", 0.0),
                random_start: number(block, b"animation_random_start_time", 0.0),
                propagate: block
                    .iter()
                    .filter(|i| i.key.as_deref() == Some(b"propagate_state"))
                    .filter_map(|i| i.value.items())
                    .flatten()
                    .filter_map(|i| {
                        Some((
                            String::from_utf8_lossy(i.key.as_deref()?).into_owned(),
                            unquote(i.value.scalar_str()?),
                        ))
                    })
                    .collect(),
                events: block
                    .iter()
                    .filter(|i| i.key.as_deref() == Some(b"event"))
                    .filter_map(|i| i.value.items())
                    .map(|event| EntityEvent {
                        time: number(event, b"time", 0.0),
                        node: text(event, b"node").map(unquote).unwrap_or_default(),
                        light: text(event, b"light").map(unquote),
                        particle: text(event, b"particle").map(unquote),
                        keep_particle: text(event, b"keep_particle") == Some("yes"),
                        trigger_once: text(event, b"trigger_once") == Some("yes"),
                    })
                    .collect(),
            })
        })
        .collect();
    Some(EntityDefinition {
        name,
        game_data: entity_game_data(items),
        mesh: text(items, b"pdxmesh").map(unquote),
        clone: text(items, b"clone").map(unquote),
        scale: text(items, b"scale").and_then(|value| value.trim().parse().ok()),
        attachments,
        default_state: text(items, b"default_state").map(unquote),
        get_state_from_parent: text(items, b"get_state_from_parent").map(|v| v == "yes"),
        locators: items
            .iter()
            .filter(|i| i.key.as_deref() == Some(b"locator"))
            .filter_map(|i| {
                let fields = i.value.items()?;
                let vector = |key: &[u8]| {
                    let values = fields
                        .iter()
                        .find(|i| i.key.as_deref() == Some(key))
                        .and_then(|i| i.value.items())
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|i| i.value.scalar_str()?.parse::<f32>().ok())
                        .collect::<Vec<_>>();
                    std::array::from_fn::<_, 3, _>(|i| values.get(i).copied().unwrap_or(0.0))
                };
                let r = vector(b"rotation").map(|v| v.to_radians() * 0.5);
                let x = Transform::from_trs([0.0; 3], [r[0].sin(), 0.0, 0.0, r[0].cos()], 1.0);
                let y = Transform::from_trs([0.0; 3], [0.0, r[1].sin(), 0.0, r[1].cos()], 1.0);
                let z = Transform::from_trs([0.0; 3], [0.0, 0.0, r[2].sin(), r[2].cos()], 1.0);
                let scale = text(fields, b"scale")
                    .and_then(|value| value.trim().parse::<f32>().ok())
                    .unwrap_or(1.0);
                let mut transform = y.compose(&x).compose(&z).compose(&Transform::from_trs(
                    [0.0; 3],
                    [0.0, 0.0, 0.0, 1.0],
                    scale,
                ));
                transform.translation = vector(b"position");
                Some((
                    unquote(text(fields, b"name")?),
                    Locator {
                        transform,
                        parent_joint: text(fields, b"parent_joint").map(unquote),
                    },
                ))
            })
            .collect(),
        states,
    })
}

fn text<'a>(items: &'a [parser::Item], key: &[u8]) -> Option<&'a str> {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key))
        .and_then(|item| item.value.scalar_str())
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_shader_data_retains_allocation_defaults_and_authored_fields() {
        let source = parse_descriptors(br#"
            entity = { name = "base" game_data = { atlas_size = { 6 2 } texture_anim_speed = 0.5 } }
            entity = { name = "clone" clone = "base" }
            entity = { name = "override" clone = "base" game_data = { texture_anim_speed = 0.75 } }
            entity = { name = "empty" game_data = {} }
            entity = { name = "partial" game_data = { atlas_size = { x = 4 } } game_data = { texture_anim_speed = 2 } }
            entity = { name = "bad" game_data = { atlas_size = { y = 2 x = 3 } } }
        "#, "gfx/test.asset").unwrap();
        assert_eq!(
            source
                .inherit("clone")
                .unwrap()
                .game_data
                .unwrap()
                .as_ref()
                .unwrap(),
            &EntityGameData {
                texture_anim_speed: 0.5,
                atlas_size: [6, 2]
            }
        );
        assert_eq!(
            source
                .inherit("override")
                .unwrap()
                .game_data
                .unwrap()
                .as_ref()
                .unwrap(),
            &EntityGameData {
                texture_anim_speed: 0.75,
                atlas_size: [1, 1]
            }
        );
        assert_eq!(source.entities["empty"].game_data, None);
        assert_eq!(
            source.entities["partial"].game_data,
            Some(Ok(EntityGameData {
                texture_anim_speed: 2.,
                atlas_size: [4, 1]
            }))
        );
        assert!(source.entities["bad"].game_data.as_ref().unwrap().is_err());
    }

    fn native_skeleton_root(skeletons: Vec<Vec<Node>>) -> Node {
        Node {
            children: vec![Node {
                name: "object".into(),
                children: skeletons
                    .into_iter()
                    .map(|children| Node {
                        children: vec![Node {
                            name: "skeleton".into(),
                            children,
                            ..Node::default()
                        }],
                        ..Node::default()
                    })
                    .collect(),
                ..Node::default()
            }],
            ..Node::default()
        }
    }

    #[test]
    fn native_skeleton_keeps_source_order_and_first_nonempty_allocation() {
        let joint = |name: &str, index, parent| Node {
            name: name.into(),
            properties: vec![
                ("ix".into(), Value::Ints(vec![index])),
                ("pa".into(), Value::Ints(vec![parent])),
                (
                    "tx".into(),
                    Value::Floats((1..=12).map(|v| v as f32).collect()),
                ),
            ],
            ..Node::default()
        };
        let root = native_skeleton_root(vec![
            vec![],
            vec![joint("z", 9, 258), joint("a", 0, -1)],
            vec![joint("ignored", 1, 0)],
        ]);
        let result = native_skeleton(&root, 0.5).unwrap();
        assert_eq!(
            result
                .iter()
                .map(|j| (j.name.as_str(), j.parent))
                .collect::<Vec<_>>(),
            [("z", 2), ("a", 255)]
        );
        assert_eq!(
            result[0].inverse_bind,
            [1., 2., 3., 4., 5., 6., 7., 8., 9., 5., 5.5, 6.]
        );
        let root = native_skeleton_root(vec![
            vec![Node {
                name: "unfilled".into(),
                ..Node::default()
            }],
            vec![joint("unused", 0, -1)],
        ]);
        assert_eq!(
            native_skeleton(&root, 2.).unwrap(),
            [NativeJoint {
                name: "unfilled".into(),
                parent: 0,
                inverse_bind: [0.; 12]
            }]
        );
    }

    #[test]
    fn native_skeleton_names_and_missing_parent_follow_byte_fields() {
        let mut node = Node {
            name: "é".repeat(35),
            properties: vec![
                ("ix".into(), Value::Ints(vec![71])),
                ("tx".into(), Value::Floats(vec![1.; 12])),
            ],
            ..Node::default()
        };
        let result = native_skeleton(&native_skeleton_root(vec![vec![node.clone()]]), 2.).unwrap();
        assert_eq!(result[0].name, "é".repeat(31));
        assert_eq!(result[0].parent, 255);
        node.properties[1].1 = Value::Floats(vec![1.; 11]);
        assert!(
            native_skeleton(&native_skeleton_root(vec![vec![node]]), 1.)
                .unwrap_err()
                .to_string()
                .contains("fewer than12 tx floats")
        );
    }

    #[test]
    fn entity_locators_keep_their_position_and_parent_joint() {
        let parsed = parse_descriptors(
            br#"entity = { name = "e" pdxmesh = "m"
                locator = { name = "a" position = { 1 2 3 } }
                locator = { name = "b" position = { 0 1 0 } scale = 2 parent_joint = "hand" } }"#,
            "gfx/entities/x.asset",
        )
        .unwrap();
        let locators = &parsed.entities["e"].locators;
        assert_eq!(locators[0].0, "a");
        assert_eq!(locators[0].1.transform.translation, [1.0, 2.0, 3.0]);
        assert_eq!(locators[0].1.parent_joint, None);
        assert_eq!(locators[1].1.parent_joint.as_deref(), Some("hand"));
        assert!((locators[0].1.transform.uniform_scale() - 1.0).abs() < 1e-6);
        assert!((locators[1].1.transform.uniform_scale() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn texture_lookup_is_byte_exact_by_basename_and_keeps_the_first_file() {
        let index = TextureIndex::new(
            [
                "gfx/models/buildings/a_diffuse.dds",
                "gfx/models/nospec.dds",
                "gfx/models/x/shared.dds",
                "gfx/models/y/shared.dds",
                "gfx/models/upper.DDS",
            ]
            .map(String::from),
        );
        let resolve = |authored| index.resolve("gfx/models/units", authored);
        assert_eq!(resolve("nospec.dds"), "gfx/models/nospec.dds");
        assert_eq!(
            resolve("a_diffuse.tga"),
            "gfx/models/buildings/a_diffuse.dds"
        );
        assert_eq!(resolve("Nospec.dds"), "gfx/models/units/Nospec.dds");
        assert_eq!(resolve("upper.DDS"), "gfx/models/units/upper.DDS");
        assert_eq!(resolve("shared.dds"), "gfx/models/x/shared.dds");
        // Enumeration order: a directory's files before its subdirectories.
        let order = crate::defines::load_order(
            "gfx/models/",
            "",
            [
                "gfx/models/a/deep/t.dds",
                "gfx/models/a/t.dds",
                "gfx/models/b.dds",
            ]
            .into_iter(),
        );
        assert_eq!(
            TextureIndex::new(order).resolve("", "t.dds"),
            "gfx/models/a/t.dds"
        );
    }

    /// A stack's descriptor files, the overlay's second.
    #[cfg(feature = "test-support")]
    fn roster_of(name: &str, base: &[(&str, &[u8])], overlay: &[(&str, &[u8])]) -> Roster {
        let root =
            std::env::temp_dir().join(format!("installed-content-roster-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (under, over) = (root.join("base"), root.join("over"));
        for (dir, files) in [(&under, base), (&over, overlay)] {
            std::fs::create_dir_all(dir).unwrap();
            for (path, bytes) in files {
                let path = dir.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        let layers =
            crate::ResolvedLayers::from_folders(&[(&under, &[][..]), (&over, &[][..])]).unwrap();
        let roster = Roster::read(&layers).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        roster
    }

    /// `LoadAssets` reads every `gfx/**/*.asset` in `Vfs_EnumerateRecursive`
    /// order (each directory's files, then its subdirectories, across
    /// layers); a clone copies its parent as then registered or is dropped,
    /// and a repeated name is replaced unless its `version` is lower than the
    /// stored one, a clone storing its parent's.
    #[cfg(feature = "test-support")]
    #[test]
    fn the_roster_registers_entities_as_load_assets_does() {
        let roster = roster_of(
            "entities",
            &[
                (
                    "gfx/models/meshes.gfx",
                    b"objectTypes = { pdxmesh = { name = f file = f.mesh } pdxmesh = { name = m file = m.mesh }
                        pdxmesh = { name = n file = n.mesh } pdxmesh = { name = o file = o.mesh } }",
                ),
                (
                    "gfx/entities/a.asset",
                    br#"entity = { name = "base" pdxmesh = "m" scale = 2 }
                        entity = { name = "early" clone = "late" }
                        entity = { name = "after_first" clone = "first" }
                        Entity = { name = "v" pdxmesh = "m" version = 2 }
                        entity = { name = "child" clone = "base" attach = { gun = "rifle" } }"#,
                ),
                (
                    "gfx/entities/b.asset",
                    br#"entity = { name = "late" pdxmesh = "m" }
                        entity = { name = "v" pdxmesh = "n" version = 1 }
                        entity = { name = "base" pdxmesh = "n" }
                        entity = { name = "cross" clone = "child" pdxmesh = "o" }"#,
                ),
                (
                    "gfx/entities/sub/c.asset",
                    br#"entity = { name = "v" pdxmesh = "o" version = 2 }
                        entity = { name = "vc" clone = "v" version = 1 }
                        entity = { name = "vc" pdxmesh = "m" version = 1 }
                        "entity" = { name = "quoted" pdxmesh = "m" }"#,
                ),
            ],
            &[(
                "gfx/entities/0.asset",
                br#"entity = { name = "first" pdxmesh = "f" }"#,
            )],
        );
        let entities = &roster.descriptors.entities;
        let mesh = |name: &str| entities[name].mesh.as_deref();
        assert!(!entities.contains_key("early"));
        assert_eq!(roster.logged.len(), 1, "{:?}", roster.logged);
        assert!(roster.logged[0].starts_with("early in gfx/entities/a.asset"));
        assert_eq!(mesh("after_first"), Some("f"));
        assert_eq!(mesh("base"), Some("n"));
        assert_eq!(entities["base"].scale, None);
        // The clone kept the base it copied, not the later one.
        assert_eq!(mesh("child"), Some("m"));
        assert_eq!(entities["child"].scale, Some(2.0));
        assert_eq!(entities["child"].clone, None);
        assert_eq!(mesh("cross"), Some("o"));
        assert_eq!(entities["cross"].scale, Some(2.0));
        assert_eq!(entities["cross"].attachments.len(), 1);
        assert_eq!(mesh("v"), Some("o"));
        // "vc" stored v's version 2, so its version 1 redeclaration is kept out.
        assert_eq!(mesh("vc"), Some("o"));
        assert!(!entities.contains_key("quoted"));
        assert!(roster.unreadable.is_empty());
    }

    /// A clone of a name an unreadable file holds may copy that file's
    /// declaration, so it may be declared there too, and so may its clones.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_clone_of_an_unreadable_name_may_be_declared_there_too() {
        let roster = roster_of(
            "unreadable",
            &[
                (
                    "gfx/models/meshes.gfx",
                    b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
                ),
                (
                    "gfx/entities/a.asset",
                    br#"entity = { name = "maybe" } }{ = ="#,
                ),
                (
                    "gfx/entities/b.asset",
                    br#"entity = { name = "child" clone = "maybe" }
                        entity = { name = "grandchild" clone = "child" }
                        entity = { name = "plain" pdxmesh = "m" }"#,
                ),
            ],
            &[],
        );
        let file = |name: &str| roster.unreadable_names.get(name).map(String::as_str);
        assert_eq!(roster.unreadable.len(), 1);
        assert_eq!(file("maybe"), Some("gfx/entities/a.asset"));
        assert_eq!(file("child"), Some("gfx/entities/a.asset"));
        assert_eq!(file("grandchild"), Some("gfx/entities/a.asset"));
        assert_eq!(file("plain"), None);
        assert_eq!(roster.logged.len(), 2, "{:?}", roster.logged);
        assert_eq!(
            roster.descriptors.entities.keys().collect::<Vec<_>>(),
            ["plain"]
        );
    }

    /// A clone's attachments replace the parent's of the same name where
    /// they stand and append new ones (`14227CEF0`), as vanilla
    /// `units_infantry.asset:144` needs: its parent's rifles 1, 2, 4, 3 keep
    /// that order when the clone declares them 1, 4, 2, 3. An unnamed one
    /// matches by joint. A mesh override no `pdxmesh` registers keeps the
    /// parent's mesh (142272140) and is logged; a redefinition's keeps its
    /// own.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_clone_merges_attachments_in_place_and_keeps_an_unregistered_mesh() {
        let roster = roster_of(
            "merge",
            &[
                (
                    "gfx/models/a.gfx",
                    b"objectTypes = { pdxmesh = { name = m file = m.mesh } pdxmesh = { name = n file = n.mesh } }",
                ),
                (
                    "gfx/entities/a.asset",
                    br#"entity = { name = "base" pdxmesh = "m"
                            attach = { name = rifle1 a = x1 } attach = { name = rifle2 a = x2 }
                            attach = { name = rifle4 a = x4 } attach = { name = rifle3 a = x3 }
                            attach = { b = joint_only } }
                        entity = { name = "child" clone = "base" pdxmesh = "missing"
                            attach = { name = rifle1 a = y1 } attach = { name = rifle4 a = y4 }
                            attach = { name = rifle2 a = y2 } attach = { name = rifle3 a = y3 }
                            attach = { b = joint_new } attach = { name = extra a = z } }
                        entity = { name = "plain" pdxmesh = "n" }
                        entity = { name = "plain" pdxmesh = "gone" }"#,
                ),
            ],
            &[],
        );
        let entities = &roster.descriptors.entities;
        let slots: Vec<_> = entities["child"]
            .attachments
            .iter()
            .map(|attachment| (attachment.name.as_str(), attachment.entity.as_str()))
            .collect();
        assert_eq!(
            slots,
            [
                ("rifle1", "y1"),
                ("rifle2", "y2"),
                ("rifle4", "y4"),
                ("rifle3", "y3"),
                ("", "joint_new"),
                ("extra", "z"),
            ]
        );
        assert_eq!(entities["child"].mesh.as_deref(), Some("m"));
        assert_eq!(entities["plain"].mesh.as_deref(), Some("n"));
        assert_eq!(roster.logged.len(), 2, "{:?}", roster.logged);
        assert!(roster.logged[0].starts_with("child in gfx/entities/a.asset: no pdxmesh missing"));
    }

    /// The last `.gfx` object of a name is the one the engine finds, and only
    /// a `pdxmesh` is a mesh; `.gfx` files declare no entities and `.asset`
    /// files no meshes.
    #[cfg(feature = "test-support")]
    #[test]
    fn the_roster_keeps_the_last_object_of_a_name() {
        let roster = roster_of(
            "objects",
            &[
                (
                    "gfx/models/a.gfx",
                    br#"objectTypes = {
                            pdxmesh = { name = "m" file = "a.mesh" }
                            pdxmesh = { name = "shadowed" file = "s.mesh" }
                            pdxmesh = { name = "kept" file = "k.mesh" }
                        }
                        entity = { name = "in_gfx" pdxmesh = "m" }"#,
                ),
                (
                    "gfx/models/b.gfx",
                    br#"objectTypes = {
                            pdxmesh = { name = "m" file = "b.mesh" }
                            pdxparticle = { name = "shadowed" type = "x" }
                        }"#,
                ),
                (
                    "gfx/models/c.asset",
                    br#"objectTypes = { pdxmesh = { name = "kept" file = "c.mesh" } }"#,
                ),
            ],
            &[],
        );
        let meshes = &roster.descriptors.meshes;
        assert_eq!(meshes["m"].file, "b.mesh");
        assert_eq!(meshes["kept"].file, "k.mesh");
        assert!(!meshes.contains_key("shadowed"));
        assert!(roster.descriptors.entities.is_empty());
    }

    /// An animation name keeps the last declaration whose file loads, and its
    /// file is always beside the descriptor; a light keeps its last.
    #[cfg(feature = "test-support")]
    #[test]
    fn the_roster_keeps_the_last_animation_that_loads() {
        let clip: &[u8] = b"@@b@[info\0[samples\0";
        let roster = roster_of(
            "animations",
            &[
                (
                    "gfx/models/x.asset",
                    br#"animation = { name = "walk" file = "walk.anim" }
                        animation = { name = "run" file = "sub/run.anim" }
                        animation = { name = "same" file = "same.anim" }
                        light = { name = "lamp" intensity = 1 }"#,
                ),
                (
                    "gfx/models/y.asset",
                    br#"animation = { name = "walk" file = "missing.anim" }
                        animation = { name = "run" file = "broken.anim" }
                        animation = { name = "same" file = "same.anim" }
                        light = { name = "lamp" intensity = 2 }"#,
                ),
                ("gfx/models/walk.anim", clip),
                ("gfx/models/sub/run.anim", clip),
                ("gfx/models/broken.anim", b"@@b@[info\0"),
            ],
            &[],
        );
        let file = |name: &str| roster.descriptors.animations[name].file.as_str();
        assert_eq!(file("walk"), "gfx/models/walk.anim");
        assert_eq!(file("run"), "gfx/models/sub/run.anim");
        assert_eq!(file("same"), "gfx/models/same.anim");
        // The last light of a name rewrites the record (1423C96F0).
        let lamp = &roster.lights["lamp"];
        assert_eq!(last_text(lamp, "intensity").as_deref(), Some("2"));
    }

    #[test]
    fn lexical_names_keep_quoted_and_bare_words_outside_comments() {
        // A bare CR ends a comment as the parser's lexer ends it, and a
        // quoted name's escapes are decoded as a parsed name's are.
        let names = lexical_names(
            b"# header\rentity = { name = \"GER_plane_entity\" pdxmesh=plane_mesh # hidden_entity
              clone = \"a b\\\"c\" } } {",
        );
        for name in [
            "entity",
            "name",
            "GER_plane_entity",
            "pdxmesh",
            "plane_mesh",
            "clone",
        ] {
            assert!(names.contains(name), "{name}");
        }
        assert!(names.contains("a b\"c"));
        assert!(!names.contains("a b\\\"c"));
        assert!(!names.contains("hidden_entity"));
    }

    #[test]
    fn sampled_pose_uses_first_joint_definition_across_lods() {
        let joint = Joint {
            name: "root".into(),
            index: 0,
            parent: None,
            inverse_bind: Some(Transform::IDENTITY),
        };
        let mesh = Mesh {
            skeleton: vec![
                joint.clone(),
                Joint {
                    inverse_bind: Some(Transform {
                        basis: [[0.0; 3]; 3],
                        translation: [0.0; 3],
                    }),
                    ..joint
                },
            ],
            ..Mesh::default()
        };
        let animation = Animation {
            fps: 2.0,
            samples: 3,
            joints: vec![AnimationJoint {
                name: "root".into(),
                channels: "tq".into(),
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: 1.0,
                ..AnimationJoint::default()
            }],
            translations: vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            rotations: vec![0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0],
            ..Animation::default()
        };
        let sampled = mesh.sampled_pose(&animation, &animation.sample(0.0, true));
        assert_eq!(sampled.len(), 1);
        assert_eq!(sampled, mesh.palette(&animation, 0));
        let middle = mesh.sampled_pose(&animation, &animation.sample(0.25, true))[0];
        assert!((middle.translation[0] - 1.0).abs() < 1e-6);
        assert!((middle.determinant() - 1.0).abs() < 1e-5);
        assert_eq!(
            animation.sample(animation.duration(), true),
            animation.sample(0.0, true)
        );
        assert_eq!(
            animation.sample(100.0, false),
            animation.sample(animation.duration(), false)
        );
    }

    #[test]
    fn entity_states_keep_variants_empty_states_and_effect_events() {
        let descriptors=parse_descriptors(br#"
          entity = { name = parent
            state = { name = move animation = walk chance = 2 animation_blend_time = 0.4 looping = no next_state = idle
              event = { time = 0.2 node = muzzle light = flash particle = fire keep_particle = yes trigger_once = yes }
              propagate_state = { weapon = fire }
            }
            state = { name = move animation = march chance = 1 }
            state = { name = idle }
          }
          entity = { name = child clone = parent state = { name = idle animation = rest } }
        "#,"gfx/entities/test.asset").unwrap();
        let entity = descriptors.inherit("child").unwrap();
        assert_eq!(entity.states["move"].len(), 2);
        assert_eq!(entity.states["idle"][0].animation, "rest");
        let state = entity.states["move"][0];
        assert_eq!(state.chance, 2.0);
        assert_eq!(state.blend, 0.4);
        assert!(!state.looping);
        assert_eq!(state.next_state.as_deref(), Some("idle"));
        assert_eq!(state.propagate, vec![("weapon".into(), "fire".into())]);
        let event = &state.events[0];
        assert_eq!(event.time, 0.2);
        assert_eq!(event.light.as_deref(), Some("flash"));
        assert!(event.keep_particle && event.trigger_once);
    }

    /// Builds a container by hand, so the tests do not depend on installed content.
    #[derive(Default)]
    struct Builder {
        bytes: Vec<u8>,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                bytes: MAGIC.to_vec(),
            }
        }

        fn node(mut self, depth: usize, name: &str) -> Self {
            self.bytes.extend(std::iter::repeat_n(b'[', depth));
            self.bytes.extend_from_slice(name.as_bytes());
            self.bytes.push(0);
            self
        }

        fn key(&mut self, name: &str, kind: u8, count: usize) {
            self.bytes.push(b'!');
            self.bytes.push(name.len() as u8);
            self.bytes.extend_from_slice(name.as_bytes());
            self.bytes.push(kind);
            self.bytes.extend_from_slice(&(count as u32).to_le_bytes());
        }

        fn ints(mut self, name: &str, values: &[i32]) -> Self {
            self.key(name, b'i', values.len());
            for value in values {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            self
        }

        fn floats(mut self, name: &str, values: &[f32]) -> Self {
            self.key(name, b'f', values.len());
            for value in values {
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            self
        }

        fn strings(mut self, name: &str, values: &[&str]) -> Self {
            self.key(name, b's', values.len());
            for value in values {
                self.bytes
                    .extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
                self.bytes.extend_from_slice(value.as_bytes());
                self.bytes.push(0);
            }
            self
        }
    }

    /// One shape, one LOD, one triangle, with every stream the tree pass reads.
    fn triangle_mesh() -> Vec<u8> {
        Builder::new()
            .ints("pdxasset", &[1, 0])
            .node(1, "object")
            .floats("loddist", &[0.0, 100.0])
            .node(2, "planeShape")
            .ints("lod", &[0])
            .node(3, "mesh")
            .floats("p", &[-1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 0.0, 0.5, 1.0])
            .floats("n", &[0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0])
            .floats(
                "ta",
                &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0],
            )
            .floats("u0", &[0.0, 0.0, 1.0, 0.0, 0.5, 1.0])
            .floats("u1", &[0.25, 0.75, 0.25, 0.75, 0.25, 0.75])
            .ints("tri", &[0, 1, 2])
            .node(4, "aabb")
            .floats("min", &[-1.0, 0.0, -1.0])
            .floats("max", &[1.0, 0.5, 1.0])
            .node(4, "material")
            .strings("shader", &["PdxMeshAdvancedSnow"])
            .strings("diff", &["leaf_diffuse.dds"])
            .strings("n", &["leaf_normal.dds"])
            .strings("spec", &["nospec.dds"])
            .node(1, "locator")
            .node(2, "attach_01")
            .bytes
    }

    #[test]
    fn a_synthetic_container_round_trips_into_typed_streams() {
        let mesh = read(&triangle_mesh()).unwrap();
        assert_eq!(mesh.lod_distances, vec![0.0, 100.0]);
        assert_eq!(mesh.locators, vec!["attach_01".to_string()]);
        assert!(mesh.skeleton.is_empty());
        assert_eq!(mesh.shapes.len(), 1);
        let shape = &mesh.shapes[0];
        assert_eq!(shape.name, "planeShape");
        assert_eq!(shape.lod, Some(0));
        let sub = &shape.meshes[0];
        assert_eq!(sub.vertex_count(), 3);
        assert_eq!(sub.triangle_count(), 1);
        assert_eq!(sub.positions[2], [0.0, 0.5, 1.0]);
        assert_eq!(sub.tangents[2], [1.0, 0.0, 0.0, -1.0]);
        assert_eq!(sub.uv0[1], [1.0, 0.0]);
        assert_eq!(sub.uv1.as_ref().unwrap()[0], [0.25, 0.75]);
        assert_eq!(sub.indices, vec![0, 1, 2]);
        assert_eq!(
            sub.bounds,
            Some(Bounds {
                min: [-1.0, 0.0, -1.0],
                max: [1.0, 0.5, 1.0],
            })
        );
        assert_eq!(
            sub.material,
            Some(Material {
                shader: Some("PdxMeshAdvancedSnow".into()),
                shader_file: None,
                diffuse: "leaf_diffuse.dds".into(),
                normal: "leaf_normal.dds".into(),
                specular: "nospec.dds".into(),
            })
        );
        assert!(sub.skin.is_none());
        assert!(sub.is_drawable());
        let lods = mesh.lods();
        assert_eq!(lods.len(), 1);
        assert_eq!(lods[0][0].name, "planeShape");
    }

    #[test]
    fn the_node_tree_nests_by_depth_alone() {
        let root = parse(&triangle_mesh()).unwrap();
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].name, "object");
        assert_eq!(root.children[1].name, "locator");
        let shape = &root.children[0].children[0];
        assert_eq!(shape.children[0].name, "mesh");
        // `aabb` and `material` are depth 4, so they are children of `mesh`.
        assert_eq!(shape.children[0].children.len(), 2);
        assert_eq!(
            root.property("pdxasset").and_then(Value::ints),
            Some([1, 0].as_slice())
        );
    }

    #[test]
    fn a_skin_is_recorded_and_a_skeleton_decoded() {
        let bytes = Builder::new()
            .node(1, "object")
            .node(2, "bodyShape")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0])
            .ints("tri", &[])
            .node(4, "skin")
            .ints("bones", &[2])
            // Four slots for the one vertex, whatever `bones` says.
            .ints("ix", &[1, 0, -1, -1])
            .floats("w", &[0.75, 0.25, 0.0, 0.0])
            .node(3, "skeleton")
            .node(4, "root")
            .ints("ix", &[0])
            .node(4, "hand")
            .ints("ix", &[1])
            .ints("pa", &[0])
            .floats(
                "tx",
                &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -2.0, -3.0, 0.0],
            )
            .bytes;
        let mesh = read(&bytes).unwrap();
        assert_eq!(mesh.skeleton.len(), 2);
        assert_eq!(mesh.skeleton[1].name, "hand");
        assert_eq!(mesh.skeleton[1].parent, Some(0));
        // `tx` is the world inverse bind, so inverting it is the bind pose.
        assert_eq!(mesh.bind_pose("hand").unwrap().translation, [2.0, 3.0, 0.0]);
        // A joint the file gives no `tx` has no bind pose at all.
        assert_eq!(mesh.skeleton[0].inverse_bind, None);
        assert_eq!(mesh.bind_pose("root"), None);
        let skin = mesh.shapes[0].meshes[0].skin.as_ref().unwrap();
        assert_eq!(skin.bones, 2);
        assert_eq!(skin.joints, vec![[1, 0, -1, -1]]);
        // The unused slots are dropped and the `-1` never reaches a palette.
        assert_eq!(skin.influences(0), vec![(1, 0.75), (0, 0.25)]);
        assert!(skin.influences(1).is_empty());
        assert!(mesh.shapes[0].meshes[0].uv1.is_none());
    }

    #[test]
    fn a_skin_stream_that_does_not_cover_every_vertex_four_times_is_refused() {
        let bytes = Builder::new()
            .node(1, "object")
            .node(2, "bodyShape")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0, 0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0, 0.0, 0.0])
            // `bones = 1` does not make the stream one wide.
            .ints("bones", &[1])
            .node(4, "skin")
            .ints("bones", &[1])
            .ints("ix", &[0, 0])
            .floats("w", &[1.0, 1.0])
            .bytes;
        let error = read(&bytes).unwrap_err().to_string();
        assert!(error.contains("not 8 of each"), "{error}");
    }

    #[test]
    fn bad_input_is_refused_without_panicking() {
        assert!(parse(b"").is_err());
        assert!(parse(b"nope").is_err());
        // A property type byte the container does not define.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(b"!\x01xq\x01\x00\x00\x00\x00");
        assert!(parse(&bytes).is_err());
        // A count that runs past the end.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(b"!\x01xi\xff\xff\xff\x0f");
        assert!(parse(&bytes).is_err());
        // A node name with no terminator.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(b"[object");
        assert!(parse(&bytes).is_err());
        // A depth that skips a level.
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(b"[[deep\x00");
        assert!(parse(&bytes).is_err());
        // A byte that starts no token.
        let mut bytes = MAGIC.to_vec();
        bytes.push(b'?');
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn disagreeing_streams_and_out_of_range_triangles_are_named_errors() {
        let short = Builder::new()
            .node(1, "object")
            .node(2, "s")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0, 0.0, 0.0])
            .bytes;
        let error = read(&short).unwrap_err().to_string();
        assert!(error.contains("n has 1"), "{error}");

        // A stream the file omits entirely is a collision hull, not an error.
        let hull = Builder::new()
            .node(1, "object")
            .node(2, "collision_boxShape")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
            .ints("tri", &[0, 1, 2])
            .bytes;
        let mesh = read(&hull).unwrap();
        let sub = &mesh.shapes[0].meshes[0];
        assert_eq!(sub.vertex_count(), 3);
        assert!(sub.normals.is_empty());
        assert!(!sub.is_drawable());

        let ragged = Builder::new()
            .node(1, "object")
            .node(2, "s")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0])
            .bytes;
        assert!(
            read(&ragged)
                .unwrap_err()
                .to_string()
                .contains("not a multiple of 3")
        );

        let out_of_range = Builder::new()
            .node(1, "object")
            .node(2, "s")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0])
            .ints("tri", &[0, 1, 2])
            .bytes;
        assert!(
            read(&out_of_range)
                .unwrap_err()
                .to_string()
                .contains("indexes vertex 1 of 1")
        );

        let odd = Builder::new()
            .node(1, "object")
            .node(2, "s")
            .node(3, "mesh")
            .floats("p", &[0.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0])
            .ints("tri", &[0, 0])
            .bytes;
        assert!(
            read(&odd)
                .unwrap_err()
                .to_string()
                .contains("not a multiple of three")
        );
        assert!(read(&MAGIC).unwrap_err().to_string().contains("[object]"));
    }

    #[test]
    fn material_keeps_missing_shader_and_source_override_distinct() {
        let bytes = Builder::new()
            .node(1, "object")
            .node(2, "shape")
            .node(3, "mesh")
            .node(4, "material")
            .node(3, "mesh")
            .node(4, "material")
            .strings("shader", &[""])
            .bytes;
        let decoded = read(&bytes).unwrap();
        let shape = &decoded.shapes[0];
        assert_eq!(shape.meshes[0].material.as_ref().unwrap().shader, None);
        assert_eq!(
            shape.meshes[1].material.as_ref().unwrap().shader.as_deref(),
            Some("")
        );
        let descriptors = parse_descriptors(
            br#"objectTypes = { pdxmesh = { name = "m" file = "m.mesh"
                meshsettings = { name = "shape" index = 0 shader_file = "" }
                meshsettings = { name = "shape" index = 0 shader_file = "later.shader" }
                meshsettings = { name = "shape" index = 1
                    shader_file = "old.shader" shader_file = "gfx/FX/custom.shader" }
            } }"#,
            "fixture.gfx",
        )
        .unwrap();
        let definition = &descriptors.meshes["m"];
        assert_eq!(
            definition.material("shape", &shape.meshes[0]).shader_file,
            None
        );
        assert_eq!(
            definition
                .material("shape", &shape.meshes[1])
                .shader_file
                .as_deref(),
            Some("gfx/FX/custom.shader")
        );
    }

    #[test]
    fn material_overrides_follow_native_first_match_and_raw_child_order() {
        let bytes = Builder::new()
            .node(1, "object")
            .node(2, "shape")
            .node(3, "auxiliary")
            .node(3, "mesh")
            .node(4, "material")
            .strings("shader", &["base_effect"])
            .strings("diff", &["base_diff"])
            .strings("n", &["base_normal"])
            .strings("spec", &["base_spec"])
            .node(3, "mesh")
            .node(4, "material")
            .strings("shader", &["second_effect"])
            .bytes;
        let decoded = read(&bytes).unwrap();
        let shape = &decoded.shapes[0];
        assert_eq!(
            shape
                .meshes
                .iter()
                .map(|s| s.child_index)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let descriptors = parse_descriptors(
            br#"
            objectTypes = { pdxmesh = { name = "m" file = "m.mesh"
                meshsettings = { name = "shape" index = -2 texture_diffuse = "wrong-negative" }
                meshsettings = { name = "shape" index = 0 texture_diffuse = "wrong-filtered-index" }
                meshsettings = { name = "shape" index = 1 texture_diffuse = "first"
                    texture_normal = "" texture_specular = "" shader = "" }
                meshsettings = { name = "shape" index = 1 texture_diffuse = "later-exact" }
                meshsettings = { name = "shape" shader = "wildcard" }
            } }"#,
            "fixture.gfx",
        )
        .unwrap();
        let definition = &descriptors.meshes["m"];
        let material = definition.material("shape", &shape.meshes[0]);
        assert_eq!(
            material,
            Material {
                shader: Some("base_effect".into()),
                shader_file: None,
                diffuse: "first".into(),
                normal: "base_normal".into(),
                specular: "base_spec".into(),
            }
        );
        assert_eq!(
            definition
                .material("shape", &shape.meshes[1])
                .shader
                .as_deref(),
            Some("wildcard")
        );
        assert_eq!(
            definition.material("other", &shape.meshes[0]),
            shape.meshes[0].material.clone().unwrap()
        );
    }

    #[test]
    fn meshsettings_keep_unnamed_rows_signed_wildcards_and_last_field_writes() {
        let descriptors = parse_descriptors(
            br#"
            objectTypes = { pdxmesh = { name = "m" file = "m.mesh"
                meshsettings = { name = "old" name = "" index = -2 shader = "first" }
                meshsettings = { name = "shape" index = 3 shader = "later" }
                meshsettings = { name = "shape" index = -1 }
                meshsettings = { name = "shape" index = 2 index = 0x10tail index = invalid }
                meshsettings = { name = "shape" index = 010 }
                meshsettings = { name = "shape" index = 4294967295 }
                meshsettings = { texture_diffuse = "old" texture_diffuse = "" }
            } }"#,
            "fixture.gfx",
        )
        .unwrap();
        let definition = &descriptors.meshes["m"];
        assert_eq!(definition.settings.len(), 7);
        assert!(definition.settings[0].name.is_empty());
        assert!(definition.settings[0].applies("unrelated", 999));
        assert!(definition.settings[2].applies("shape", 999));
        assert!(!definition.settings[2].applies("unrelated", 999));
        assert_eq!(definition.settings[3].index, 16);
        assert_eq!(definition.settings[4].index, 8);
        assert_eq!(definition.settings[5].index, -1);
        assert_eq!(definition.settings[6].diffuse.as_deref(), Some(""));
        let sub = SubMesh {
            child_index: 3,
            ..Default::default()
        };
        assert_eq!(
            definition.material("shape", &sub).shader.as_deref(),
            Some("first")
        );
    }

    #[test]
    fn the_gfx_grammar_maps_a_mapobject_name_to_its_mesh_and_scale() {
        let descriptors = parse_descriptors(
            br#"
            objectTypes = {
                pdxmesh = {
                    name = "mapobject_29"
                    file = "gfx/models/mapitems/trees/beech.mesh"
                    scale = 1.2
                }
                pdxmesh = {
                    name = "house"
                    file = "gfx/models/buildings/city_4_01.mesh"
                    meshsettings = {
                        name = "city_4_01_LODShape0"
                        index = 1
                        texture_diffuse = "city_d.dds"
                        shader = "PdxMeshSnow"
                    }
                }
            }
            "#,
            "gfx/entities/trees.gfx",
        )
        .unwrap();
        let resolved = descriptors.resolve("mapobject_29").unwrap();
        assert_eq!(
            resolved.definition.file,
            "gfx/models/mapitems/trees/beech.mesh"
        );
        assert!((resolved.scale - 1.2).abs() < 1e-6);
        let house = &descriptors.meshes["house"];
        assert_eq!(house.scale, 1.0);
        assert_eq!(
            house.settings,
            vec![MeshSetting {
                name: "city_4_01_LODShape0".into(),
                index: 1,
                diffuse: Some("city_d.dds".into()),
                normal: None,
                specular: None,
                shader: Some("PdxMeshSnow".into()),
                shader_file: None,
            }]
        );
        assert!(descriptors.resolve("absent").is_none());
    }

    #[test]
    fn an_entity_follows_its_mesh_and_its_clone_chain() {
        let descriptors = parse_descriptors(
            br#"
            objectTypes = { pdxmesh = { name = "m" file = "a.mesh" scale = 2.0 } }
            entity = { name = "base" pdxmesh = "m" }
            entity = { name = "child" clone = "base" scale = 1.5 }
            entity = { name = "loop_a" clone = "loop_b" }
            entity = { name = "loop_b" clone = "loop_a" }
            entity = { name = "own_mesh" clone = "scaled" pdxmesh = "m" }
            entity = { name = "scaled" pdxmesh = "other" scale = 0.8 }
            "#,
            "gfx/entities/units_infantry.asset",
        )
        .unwrap();
        assert_eq!(descriptors.resolve("base").unwrap().scale, 2.0);
        assert_eq!(descriptors.resolve("child").unwrap().scale, 3.0);
        assert_eq!(descriptors.entities["child"].clone, Some("base".into()));
        // A cyclic clone chain terminates instead of looping.
        assert!(descriptors.resolve("loop_a").is_none());
        // An entity that declares its own mesh still inherits the clone's
        // scale: this is `GER_infantry_entity` over `infantry_rifle_entity`.
        let own = descriptors.resolve("own_mesh").unwrap();
        assert_eq!(own.definition.file, "a.mesh");
        assert!((own.scale - 1.6).abs() < 1e-6);
    }

    #[test]
    fn an_entity_inherits_its_clone_s_attachments_by_handle_and_its_states() {
        let descriptors = parse_descriptors(
            br#"
            objectTypes = {
                pdxmesh = {
                    name = "rig"
                    file = "rig.mesh"
                    animation = { id = "idle" type = "rig_idle_animation" }
                    animation = { id = "move" type = "rig_move_animation" }
                }
            }
            entity = {
                name = "base"
                pdxmesh = "rig"
                default_state = "idle"
                state = { name = "idle" animation = "idle" }
                state = { name = "move" animation = "move" }
                attach = { name = "gun" artillery = "generic_gun_entity" }
                attach = { name = "crew" infantry = "generic_crew_entity" }
            }
            entity = {
                clone = "base"
                name = "german"
                pdxmesh = "rig"
                attach = { name = "crew" infantry = "GER_crew_entity" }
            }
            animation = { name = "rig_idle_animation" file = "rig_idle.anim" }
            "#,
            "gfx/entities/units_artillery.asset",
        )
        .unwrap();
        let german = descriptors.inherit("german").unwrap();
        assert_eq!(german.default_state, Some("idle"));
        assert_eq!(
            german
                .attachments
                .iter()
                .map(|attachment| (attachment.joint.as_str(), attachment.entity.as_str()))
                .collect::<Vec<_>>(),
            // The child's handle replaces the parent's where it stands.
            vec![
                ("artillery", "generic_gun_entity"),
                ("infantry", "GER_crew_entity"),
            ]
        );
        assert_eq!(german.states["idle"][0].animation, "idle");
        // The default state picks the animation id, the mesh the animation
        // name and the descriptor the file.
        let resolved = descriptors.resolve_entity("german").unwrap();
        assert_eq!(
            resolved.animation.map(|animation| animation.file.as_str()),
            Some("gfx/entities/rig_idle.anim")
        );
    }

    #[test]
    fn an_animation_block_resolves_against_its_own_directory() {
        let descriptors = parse_descriptors(
            br#"
            animation = { name = "rig_idle_animation" file = "rig_idle.anim" }
            "#,
            "gfx/models/units/vehicles/animation_vehicles.asset",
        )
        .unwrap();
        assert_eq!(
            descriptors.animations["rig_idle_animation"].file,
            "gfx/models/units/vehicles/rig_idle.anim"
        );
    }

    #[test]
    fn a_rest_pose_composes_its_translation_rotation_and_scale() {
        // A quarter turn about +Y at scale 2: +X lands on -Z.
        let half = std::f32::consts::FRAC_PI_4;
        let transform =
            Transform::from_trs([1.0, 0.0, 0.0], [0.0, half.sin(), 0.0, half.cos()], 2.0);
        let point = transform.transform_point([1.0, 0.0, 0.0]);
        assert!((point[0] - 1.0).abs() < 1e-5, "{point:?}");
        assert!(point[1].abs() < 1e-5, "{point:?}");
        assert!((point[2] + 2.0).abs() < 1e-5, "{point:?}");
        assert!((transform.uniform_scale() - 2.0).abs() < 1e-5);
        // A zero scale is the switch that hides an attachment, and it has no
        // inverse.
        let hidden = Transform::from_trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], 0.0);
        assert_eq!(hidden.uniform_scale(), 0.0);
        assert!(hidden.inverse().is_none());
        // So does a matrix whose entries are not finite.
        let wild = Transform::from_columns(&[1e30; 12]).unwrap();
        assert!(wild.inverse().is_none());
    }

    /// Two joints, three frames. `root` animates nothing, `vehicle` animates
    /// `t` and `q`, so only `vehicle` appears in the `[samples]` streams.
    fn animation_bytes() -> Vec<u8> {
        Builder::new()
            .node(1, "info")
            .floats("fps", &[30.0])
            .ints("sa", &[3])
            .ints("j", &[2])
            .node(2, "root")
            .strings("sa", &[""])
            .floats("t", &[0.0, 0.0, 0.0])
            .floats("q", &[0.0, 0.0, 0.0, -1.0])
            .floats("s", &[1.0])
            .node(2, "vehicle")
            .strings("sa", &["tq"])
            .floats("t", &[0.0742, 0.0, 1.25571])
            .floats("q", &[0.0, 0.0, 0.0, 1.0])
            .floats("s", &[0.7])
            .node(1, "samples")
            // Frame-major: frame 0 repeats the `[info]` values, as every
            // shipped file does, then the joint walks along +x.
            .floats(
                "t",
                &[
                    0.0742, 0.0, 1.25571, // frame 0
                    1.0742, 0.0, 1.25571, // frame 1
                    2.0742, 0.0, 1.25571, // frame 2
                ],
            )
            .floats(
                "q",
                &[
                    0.0, 0.0, 0.0, 1.0, // frame 0
                    0.0, 0.0, 0.0, 1.0, // frame 1
                    0.0, 0.0, 0.0, 1.0, // frame 2
                ],
            )
            .bytes
    }

    #[test]
    fn an_animation_info_block_decodes_its_rest_pose() {
        let animation = read_animation(&animation_bytes()).unwrap();
        assert_eq!(animation.fps, 30.0);
        assert_eq!(animation.samples, 3);
        assert_eq!(animation.joints.len(), 2);
        assert_eq!(animation.joints[1].channels, "tq");
        let rest = animation.rest("vehicle").unwrap();
        assert_eq!(rest.translation, [0.0742, 0.0, 1.25571]);
        assert!((rest.uniform_scale() - 0.7).abs() < 1e-5);
        assert!(animation.rest("absent").is_none());
    }

    #[test]
    fn the_samples_block_is_frame_major_over_the_joints_that_declare_the_channel() {
        let animation = read_animation(&animation_bytes()).unwrap();
        // One joint declares `t` and `q`, none declares `s`.
        assert_eq!(animation.channel_joints(), [1, 1, 0]);
        // Frame 0 is the `[info]` block, which is what a still hour draws.
        assert_eq!(
            animation.local("vehicle", FIRST_FRAME),
            animation.rest("vehicle")
        );
        assert_eq!(
            animation.local("vehicle", 2).unwrap().translation,
            [2.0742, 0.0, 1.25571]
        );
        // A channel the joint does not animate keeps its rest value at every
        // frame, and so does every channel of a joint with no channels at all.
        assert!((animation.local("vehicle", 2).unwrap().uniform_scale() - 0.7).abs() < 1e-5);
        assert_eq!(animation.local("root", 2), animation.rest("root"));
        // A frame past the end falls back to the rest pose rather than
        // reading another joint's samples.
        assert_eq!(animation.local("vehicle", 99), animation.rest("vehicle"));
        assert_eq!(animation.local("absent", 0), None);
    }

    #[test]
    fn a_samples_stream_of_the_wrong_length_is_a_named_error() {
        let bytes = Builder::new()
            .node(1, "info")
            .floats("fps", &[30.0])
            .ints("sa", &[3])
            .node(2, "vehicle")
            .strings("sa", &["t"])
            .floats("t", &[0.0, 0.0, 0.0])
            .node(1, "samples")
            .floats("t", &[0.0, 0.0, 0.0])
            .bytes;
        let error = read_animation(&bytes).unwrap_err().to_string();
        assert!(error.contains("needs 9 floats and has 3"), "{error}");
    }

    /// A two-joint arm: `root` at the origin and `hand` bound two units out
    /// along +x, with one vertex at the hand.
    fn arm_bytes() -> Vec<u8> {
        Builder::new()
            .node(1, "object")
            .node(2, "armShape")
            .node(3, "mesh")
            .floats("p", &[2.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0])
            .ints("tri", &[])
            .node(4, "skin")
            .ints("bones", &[1])
            .ints("ix", &[1, -1, -1, -1])
            .floats("w", &[1.0, 0.0, 0.0, 0.0])
            .node(3, "skeleton")
            .node(4, "root")
            .ints("ix", &[0])
            .floats(
                "tx",
                &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            )
            .node(4, "hand")
            .ints("ix", &[1])
            .ints("pa", &[0])
            .floats(
                "tx",
                &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -2.0, 0.0, 0.0],
            )
            .bytes
    }

    #[test]
    fn a_palette_poses_a_joint_and_undoes_its_bind() {
        let mesh = read(&arm_bytes()).unwrap();
        // An animation that lifts the hand a unit and leaves the root alone.
        let animation = read_animation(
            &Builder::new()
                .node(1, "info")
                .ints("sa", &[1])
                .node(2, "root")
                .strings("sa", &[""])
                .floats("t", &[0.0, 0.0, 0.0])
                .floats("q", &[0.0, 0.0, 0.0, 1.0])
                .floats("s", &[1.0])
                .node(2, "hand")
                .strings("sa", &[""])
                .floats("t", &[2.0, 1.0, 0.0])
                .floats("q", &[0.0, 0.0, 0.0, 1.0])
                .floats("s", &[1.0])
                .bytes,
        )
        .unwrap();
        // The joint is where the animation puts it, which is where an
        // attachment on it hangs.
        assert_eq!(
            mesh.pose(&animation, FIRST_FRAME, "hand")
                .unwrap()
                .translation,
            [2.0, 1.0, 0.0]
        );
        // And the vertex bound to it moves with it: bind (2, 0, 0) to
        // (2, 1, 0), the same one unit up.
        let palette = mesh.palette(&animation, FIRST_FRAME);
        assert_eq!(palette.len(), 2);
        let skin = mesh.shapes[0].meshes[0].skin.as_ref().unwrap();
        assert_eq!(skin.influences(0), vec![(1, 1.0)]);
        assert_eq!(palette[1].transform_point([2.0, 0.0, 0.0]), [2.0, 1.0, 0.0]);
        // The root is unmoved, so its own matrix is the identity.
        assert_eq!(palette[0], Transform::IDENTITY);
    }

    #[test]
    fn a_palette_under_no_animation_leaves_every_vertex_in_its_bind_pose() {
        let mesh = read(&arm_bytes()).unwrap();
        let palette = mesh.palette(&Animation::default(), FIRST_FRAME);
        for (joint, matrix) in palette.iter().enumerate() {
            let at = matrix.transform_point([2.0, 0.0, 0.0]);
            assert!(
                at.iter()
                    .zip([2.0, 0.0, 0.0])
                    .all(|(value, want)| (value - want).abs() < 1e-5),
                "joint {joint} moved the vertex to {at:?}"
            );
        }
    }

    #[test]
    fn a_joint_with_an_unusable_inverse_bind_takes_the_identity() {
        // `artillery_frame.mesh` writes an all-zero `tx` and
        // `GER_infantry.mesh` writes 1e12 on one diagonal; neither inverts.
        let bytes = Builder::new()
            .node(1, "object")
            .node(2, "s")
            .node(3, "mesh")
            .floats("p", &[1.0, 0.0, 0.0])
            .floats("n", &[0.0, 1.0, 0.0])
            .floats("ta", &[1.0, 0.0, 0.0, 1.0])
            .floats("u0", &[0.0, 0.0])
            .node(3, "skeleton")
            .node(4, "zero")
            .ints("ix", &[0])
            .floats("tx", &[0.0; 12])
            .node(4, "huge")
            .ints("ix", &[1])
            .floats(
                "tx",
                &[
                    1e12, 0.0, 0.0, 0.0, 1e12, 0.0, 0.0, 0.0, 1e12, -1e30, 0.0, 0.0,
                ],
            )
            .bytes;
        let mesh = read(&bytes).unwrap();
        let palette = mesh.palette(&Animation::default(), FIRST_FRAME);
        assert_eq!(palette, vec![Transform::IDENTITY; 2]);
    }
}
