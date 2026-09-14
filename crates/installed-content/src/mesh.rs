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
//! Skins are **not** decoded: no pass here deforms a mesh, and a skinned mesh
//! drawn from its own vertex stream is its bind pose. Their presence is
//! recorded on [`SubMesh::has_skin`].
//!
//! Skeletons and the `[info]` half of `.anim` **are**, because several shipped
//! unit entities are attachment rigs: `GER_motorized_entity`'s own mesh is a
//! three-vertex plane and the truck and rifleman the game draws for a motorized
//! division are its `attach` blocks at the rig's `vehicle` and `infantry`
//! joints. [`Mesh::skeleton`] carries the joints and [`read_animation`] the
//! rest pose those joints take, which is the transform an attachment hangs
//! from; see [`Descriptors::resolve_entity`]. The `[samples]` half of `.anim`
//! is not decoded: this pass draws one still hour and never advances a clock,
//! so a sampled frame would be an invention, and every joint that carries an
//! attachment on the four shipped rigs declares no animated channel at all.

use std::{collections::BTreeMap, io};

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

    fn is_finite(&self) -> bool {
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
    pub shader: String,
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

/// One `[mesh]` node: a vertex stream set, its triangles and its material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubMesh {
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
    /// Whether a `[skin]` node was present. Its streams are not decoded.
    pub has_skin: bool,
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

    /// The joint's pose in the mesh's own space under one animation's rest
    /// values, composed down the skeleton's `pa` chain.
    ///
    /// The `.anim` states each joint **relative to its parent** and carries no
    /// parent of its own, so the two files are read together: the `[skeleton]`
    /// gives the tree and the `[info]` block the transform on each link. For a
    /// rig this is one link — every attachment joint of `motorized_frame`,
    /// `cavalry_frame`, `bicycle_frame` and `artillery_frame` hangs off an
    /// identity root — and for a rifleman it is eight, from the hip out to
    /// `Right_Hand_node`.
    ///
    /// A joint the animation does not name contributes its own bind transform
    /// relative to its parent's, so a partial `.anim` still places its joint.
    /// `None` when the skeleton has no joint of that name.
    pub fn rest_pose(&self, animation: &Animation, name: &str) -> Option<Transform> {
        let mut chain = Vec::new();
        let mut joint = self.joint(name)?;
        for _ in 0..=self.skeleton.len() {
            chain.push(joint);
            let Some(parent) = joint.parent else { break };
            let Some(parent) = self.skeleton.iter().find(|other| other.index == parent) else {
                break;
            };
            if parent.index == joint.index {
                break;
            }
            joint = parent;
        }
        let mut out = Transform::IDENTITY;
        for (position, joint) in chain.iter().enumerate().rev() {
            let local = animation.rest(&joint.name).or_else(|| {
                let world = joint.inverse_bind?.inverse()?;
                match chain.get(position + 1) {
                    Some(parent) => Some(parent.inverse_bind?.compose(&world)),
                    None => Some(world),
                }
            })?;
            out = out.compose(&local);
        }
        Some(out)
    }
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
        for node in child.children_named("mesh") {
            meshes.push(read_sub_mesh(&child.name, node)?);
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

/// A decoded `.anim` file's `[info]` block: the rest pose of every joint.
///
/// The `[samples]` block is deliberately not decoded; see the module comment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    /// `fps`, the sample rate.
    pub fps: f32,
    /// `sa`, the number of frames.
    pub samples: u32,
    /// One entry per joint, in skeleton index order.
    pub joints: Vec<AnimationJoint>,
}

impl Animation {
    /// The rest pose of one joint, as a transform in its parent's space.
    pub fn rest(&self, name: &str) -> Option<Transform> {
        let joint = self.joints.iter().find(|joint| joint.name == name)?;
        Some(Transform::from_trs(
            joint.translation,
            joint.rotation,
            joint.scale,
        ))
    }
}

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

/// Decodes the `[info]` block of a `.anim` file (section 1.3).
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
    Ok(Animation {
        fps: single("fps").unwrap_or(0.0),
        samples: info
            .property("sa")
            .and_then(Value::ints)
            .and_then(|values| values.first().copied())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        joints,
    })
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
        shader: material.string("shader").unwrap_or_default().to_string(),
        diffuse: material.string("diff").unwrap_or_default().to_string(),
        normal: material.string("n").unwrap_or_default().to_string(),
        specular: material.string("spec").unwrap_or_default().to_string(),
    });
    Ok(SubMesh {
        positions,
        normals,
        tangents,
        uv0,
        uv1,
        indices,
        bounds,
        material,
        has_skin: node.child("skin").is_some(),
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshSetting {
    /// The shape node name inside the `.mesh`.
    pub name: String,
    /// The material index within that shape.
    pub index: u32,
    pub diffuse: Option<String>,
    pub normal: Option<String>,
    pub specular: Option<String>,
    pub shader: Option<String>,
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
    /// What a `clone` overrides this attachment by: its own handle, or its
    /// joint when the block names no handle.
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
    pub states: Vec<(String, String)>,
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
    /// Every attachment reachable through the chain, the nearest declaration of
    /// each `name` winning, in the order the nearest link declares them.
    pub attachments: Vec<&'a Attachment>,
    /// State name to animation id, the nearest declaration winning.
    pub states: BTreeMap<&'a str, &'a str>,
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
        for _ in 0..=self.entities.len() {
            out.mesh = out.mesh.or(current.mesh.as_deref());
            out.scale = out.scale.or(current.scale);
            out.default_state = out.default_state.or(current.default_state.as_deref());
            for attachment in &current.attachments {
                if !out
                    .attachments
                    .iter()
                    .any(|held| held.handle() == attachment.handle())
                {
                    out.attachments.push(attachment);
                }
            }
            for (state, animation) in &current.states {
                out.states
                    .entry(state.as_str())
                    .or_insert(animation.as_str());
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
            .and_then(|id| {
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

/// Reads the `pdxmesh` and `entity` blocks of the named descriptor files.
///
/// `gfx/entities` is outside the content coverage policy (`RENDERING.md`
/// section 4), so the files go through [`ResolvedLayers::read_uncovered`] and
/// are **not** hash-verified; the viewer states that limitation once.
///
/// A file that is not present is skipped rather than refused, so a descriptor
/// a DLC layer adds can be listed unconditionally. Errors are named for a file
/// that is present but does not parse.
pub fn read_descriptors(layers: &ResolvedLayers, paths: &[&str]) -> io::Result<Descriptors> {
    let mut out = Descriptors::default();
    for path in paths {
        let bytes = match layers.read_uncovered(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let document =
            parser::parse(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
        collect(&document, path, &mut out);
    }
    Ok(out)
}

/// Reads the blocks of one already-read descriptor file. `path` is the file's
/// own path, which an `animation` block's `file` resolves against.
pub fn parse_descriptors(bytes: &[u8], path: &str) -> io::Result<Descriptors> {
    let document = parser::parse(bytes)?;
    let mut out = Descriptors::default();
    collect(&document, path, &mut out);
    Ok(out)
}

fn collect(document: &parser::Document, path: &str, out: &mut Descriptors) {
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    for item in &document.items {
        match item.key.as_deref() {
            Some(b"objectTypes") => {
                for block in item.value.items().unwrap_or(&[]) {
                    if block.key.as_deref() == Some(b"pdxmesh")
                        && let Some(definition) = mesh_definition(&block.value)
                    {
                        out.meshes.insert(definition.name.clone(), definition);
                    }
                }
            }
            Some(b"entity") => {
                if let Some(definition) = entity_definition(&item.value) {
                    out.entities.insert(definition.name.clone(), definition);
                }
            }
            Some(b"animation") => {
                if let Some(definition) = animation_definition(&item.value, directory) {
                    out.animations.insert(definition.name.clone(), definition);
                }
            }
            _ => {}
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
        // The `.anim` sits beside the descriptor that names it; every shipped
        // block gives a bare file name.
        file: if file.contains('/') || directory.is_empty() {
            file
        } else {
            format!("{directory}/{file}")
        },
    })
}

fn mesh_definition(value: &parser::Value) -> Option<MeshDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    let file = unquote(text(items, b"file")?);
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
    Some(MeshSetting {
        name: unquote(text(items, b"name")?),
        index: text(items, b"index")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0),
        diffuse: text(items, b"texture_diffuse").map(unquote),
        normal: text(items, b"texture_normal").map(unquote),
        specular: text(items, b"texture_specular").map(unquote),
        shader: text(items, b"shader").map(unquote),
    })
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
            Some((
                unquote(text(block, b"name")?),
                unquote(text(block, b"animation")?),
            ))
        })
        .collect();
    Some(EntityDefinition {
        name,
        mesh: text(items, b"pdxmesh").map(unquote),
        clone: text(items, b"clone").map(unquote),
        scale: text(items, b"scale").and_then(|value| value.trim().parse().ok()),
        attachments,
        default_state: text(items, b"default_state").map(unquote),
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
                shader: "PdxMeshAdvancedSnow".into(),
                diffuse: "leaf_diffuse.dds".into(),
                normal: "leaf_normal.dds".into(),
                specular: "nospec.dds".into(),
            })
        );
        assert!(!sub.has_skin);
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
            .ints("bones", &[4])
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
        assert!(mesh.shapes[0].meshes[0].has_skin);
        assert!(mesh.shapes[0].meshes[0].uv1.is_none());
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
            // The child's own handle wins and the parent's other one carries.
            vec![
                ("infantry", "GER_crew_entity"),
                ("artillery", "generic_gun_entity"),
            ]
        );
        assert_eq!(german.states.get("idle"), Some(&"idle"));
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

    #[test]
    fn an_animation_info_block_decodes_its_rest_pose() {
        let bytes = Builder::new()
            .node(1, "info")
            .floats("fps", &[30.0])
            .ints("sa", &[2])
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
            .bytes;
        let animation = read_animation(&bytes).unwrap();
        assert_eq!(animation.fps, 30.0);
        assert_eq!(animation.samples, 2);
        assert_eq!(animation.joints.len(), 2);
        assert_eq!(animation.joints[1].channels, "tq");
        let rest = animation.rest("vehicle").unwrap();
        assert_eq!(rest.translation, [0.0742, 0.0, 1.25571]);
        assert!((rest.uniform_scale() - 0.7).abs() < 1e-5);
        assert!(animation.rest("absent").is_none());
    }
}
