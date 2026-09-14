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
//! Skeletons, skins and animations are **not** decoded. The tree layer this
//! module was written for draws static canopy tiles, and R3 section 1.3 records
//! that no map object animates at rest. Their presence is recorded on
//! [`SubMesh::has_skin`] and [`Mesh::has_skeleton`] so a later pass knows the
//! streams exist without this module inventing a representation for them.

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
    /// Whether a `[skeleton]` node was present. Its joints are not decoded.
    pub has_skeleton: bool,
    /// The names under the root `[locator]` node, if any.
    pub locators: Vec<String>,
}

impl Mesh {
    /// The shape that carries `lod`, or the `index`-th shape on a mesh that
    /// declares no LOD indices at all.
    pub fn shape_for_lod(&self, lod: usize) -> Option<&Shape> {
        self.shapes
            .iter()
            .find(|shape| shape.lod == Some(lod as i32))
            .or_else(|| {
                self.shapes
                    .iter()
                    .all(|shape| shape.lod.is_none())
                    .then(|| self.shapes.get(lod))
                    .flatten()
            })
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
    let mut has_skeleton = false;
    let mut shapes = Vec::new();
    for child in &object.children {
        if child.name == "skeleton" {
            has_skeleton = true;
            continue;
        }
        // `[skeleton]` sits beside the shape's `[mesh]` nodes at depth 3
        // (section 1.2); the depth-1 form above is accepted for robustness.
        has_skeleton |= child.child("skeleton").is_some();
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
        has_skeleton,
        locators,
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
}

/// One `entity` block of a `gfx/entities/*.asset` file (section 1.5).
///
/// Only the four keys a static map object needs are kept: `state`, `attach`,
/// `event` and the particle machinery matter below camera height 180, which no
/// pass reads yet (R3 section 5.3).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityDefinition {
    pub name: String,
    /// `pdxmesh`, the key of a [`MeshDefinition`].
    pub mesh: Option<String>,
    /// `clone`, the entity this one inherits from before its own overrides.
    pub clone: Option<String>,
    pub scale: Option<f32>,
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
}

impl Descriptors {
    /// Resolves an entity or mesh name to its `.mesh` and world scale.
    ///
    /// A `mapobject_<paletteIndex>` is a `pdxmesh` name directly (R3 section
    /// 3.1), so the mesh table is tried first; an entity name follows its
    /// `pdxmesh`, or its `clone` chain when it declares none. The chain is
    /// bounded by the number of entities, so a cyclic `clone` terminates.
    pub fn resolve(&self, name: &str) -> Option<ResolvedMesh<'_>> {
        if let Some(definition) = self.meshes.get(name) {
            return Some(ResolvedMesh {
                definition,
                scale: definition.scale,
            });
        }
        let mut entity = self.entities.get(name)?;
        let mut scale = entity.scale;
        for _ in 0..self.entities.len() {
            if let Some(mesh) = &entity.mesh {
                let definition = self.meshes.get(mesh)?;
                return Some(ResolvedMesh {
                    definition,
                    scale: scale.unwrap_or(1.0) * definition.scale,
                });
            }
            entity = self.entities.get(entity.clone.as_deref()?)?;
            scale = scale.or(entity.scale);
        }
        None
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
        collect(&document, &mut out);
    }
    Ok(out)
}

/// Reads the `pdxmesh` and `entity` blocks of one already-read descriptor file.
pub fn parse_descriptors(bytes: &[u8]) -> io::Result<Descriptors> {
    let document = parser::parse(bytes)?;
    let mut out = Descriptors::default();
    collect(&document, &mut out);
    Ok(out)
}

fn collect(document: &parser::Document, out: &mut Descriptors) {
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
            _ => {}
        }
    }
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
    Some(MeshDefinition {
        name,
        file,
        scale: text(items, b"scale")
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(1.0),
        settings,
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
    Some(EntityDefinition {
        name,
        mesh: text(items, b"pdxmesh").map(unquote),
        clone: text(items, b"clone").map(unquote),
        scale: text(items, b"scale").and_then(|value| value.trim().parse().ok()),
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
        assert!(!mesh.has_skeleton);
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
        assert_eq!(
            mesh.shape_for_lod(0).map(|shape| shape.name.as_str()),
            Some("planeShape")
        );
        assert_eq!(mesh.shape_for_lod(1), None);
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
    fn a_skin_and_a_skeleton_are_recorded_without_being_decoded() {
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
            .bytes;
        let mesh = read(&bytes).unwrap();
        assert!(mesh.has_skeleton);
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
            "#,
        )
        .unwrap();
        assert_eq!(descriptors.resolve("base").unwrap().scale, 2.0);
        assert_eq!(descriptors.resolve("child").unwrap().scale, 3.0);
        assert_eq!(descriptors.entities["child"].clone, Some("base".into()));
        // A cyclic clone chain terminates instead of looping.
        assert!(descriptors.resolve("loop_a").is_none());
    }
}
