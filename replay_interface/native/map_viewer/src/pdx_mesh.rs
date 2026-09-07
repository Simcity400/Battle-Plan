
use crate::{GameFileResolver, Result};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const PDX_ASSET_BINARY_MAGIC: &[u8; 4] = b"@@b@";
pub const PDX_ASSET_TEXT_MAGIC: &[u8; 4] = b"@@t@";
pub const PDX_ASSET_SUPPORTED_VERSION: [i32; 2] = [1, 0];
pub const PDX_MODELS_ROOT: &str = "gfx/models";


#[derive(Clone, Debug, PartialEq)]
pub enum PdxValues {
    I32(Vec<i32>),
    F32(Vec<f32>),
    Str(Vec<String>),
}

impl PdxValues {
    pub fn type_char(&self) -> char {
        match self {
            PdxValues::I32(_) => 'i',
            PdxValues::F32(_) => 'f',
            PdxValues::Str(_) => 's',
        }
    }

    pub fn len(&self) -> usize {
        match self {
            PdxValues::I32(values) => values.len(),
            PdxValues::F32(values) => values.len(),
            PdxValues::Str(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxProperty {
    pub name: String,
    pub values: PdxValues,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxNode {
    pub name: String,
    pub properties: Vec<PdxProperty>,
    pub children: Vec<PdxNode>,
}

impl PdxNode {
    fn new(name: String) -> Self {
        Self {
            name,
            properties: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn property(&self, name: &str) -> Option<&PdxProperty> {
        self.properties.iter().find(|prop| prop.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a PdxNode> {
        self.children.iter().filter(move |child| child.name == name)
    }

    pub(crate) fn native_aabb(&self) -> std::result::Result<NativePdxAabb<'_>, NativePdxAabbRefusal> {
        let Some(aabb) = self.children_named("aabb").next() else {
            return Ok(NativePdxAabb::Absent);
        };
        let coordinates = |name| {
            let property = aabb.property(name)
                .ok_or(NativePdxAabbRefusal::MissingCoordinate { property: name })?;
            let values = match &property.values {
                PdxValues::F32(values) => values,
                PdxValues::I32(_) | PdxValues::Str(_) => return Err(NativePdxAabbRefusal::CoordinateType {
                    property: name,
                }),
            };
            values.get(..3).and_then(|first| first.try_into().ok())
                .ok_or(NativePdxAabbRefusal::CoordinateUnderflow {
                    property: name,
                    count: values.len(),
                })
        };
        Ok(NativePdxAabb::Present { min: coordinates("min")?, max: coordinates("max")? })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NativePdxAabb<'a> {
    Absent,
    Present { min: &'a [f32; 3], max: &'a [f32; 3] },
}

impl NativePdxAabb<'_> {
    pub(crate) fn scaled(self, scale: f32) -> ([f32; 3], [f32; 3]) {
        match self {
            // The absent branch bypasses multiplication, even for negative or NaN scale.
            Self::Absent => ([0.0; 3], [0.0; 3]),
            Self::Present { min, max } => (
                [scale * min[0], min[1] * scale, min[2] * scale],
                [scale * max[0], max[1] * scale, max[2] * scale],
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativePdxAabbRefusal {
    MissingCoordinate { property: &'static str },
    CoordinateType { property: &'static str },
    CoordinateUnderflow { property: &'static str, count: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxAssetTree {
    pub version: Vec<i32>,
    pub root: PdxNode,
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativePdxCollisionSource {
    pub root_child: usize,
    pub shape_child: usize,
    pub mesh_child: usize,
}

#[derive(Debug, PartialEq)]
pub struct NativePdxCollision {
    pub source: NativePdxCollisionSource,
    pub scaled_positions: Vec<[f32; 3]>,
    pub indices: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativePdxCollisionRefusal {
    PropertyType {
        source: NativePdxCollisionSource,
        property: &'static str,
    },
    PositionTriple {
        source: NativePdxCollisionSource,
        float_count: usize,
    },
    IndexTriple {
        source: NativePdxCollisionSource,
        index_count: usize,
    },
    IndexOutOfBounds {
        source: NativePdxCollisionSource,
        ordinal: usize,
        index: u16,
        vertex_count: usize,
    },
    AllocationUnavailable {
        source: NativePdxCollisionSource,
        stream: &'static str,
    },
}

// Native lookups select the first material/property/string, before drawable stream loading.
fn native_collision_shader(mesh: &PdxNode) -> bool {
    let Some(material) = mesh.children_named("material").next() else { return false; };
    let Some(PdxProperty { values: PdxValues::Str(shader), .. }) = material.property("shader")
        else { return false; };
    shader.first().map(String::as_str) == Some("Collision")
}

impl PdxAssetTree {
    pub fn native_collision(
        &self,
        authored_scale: f32,
    ) -> std::result::Result<Option<NativePdxCollision>, NativePdxCollisionRefusal> {
        let mut selected = None;
        for (root_child, object) in self.root.children.iter().enumerate() {
            if object.name != "object" {
                continue;
            }
            for (shape_child, shape) in object.children.iter().enumerate() {
                for (mesh_child, mesh) in shape.children.iter().enumerate() {
                    if mesh.name != "mesh" {
                        continue;
                    }
                    let source = NativePdxCollisionSource {
                        root_child,
                        shape_child,
                        mesh_child,
                    };
                    if !native_collision_shader(mesh) { continue; }
                    let (Some(p), Some(tri)) = (mesh.property("p"), mesh.property("tri")) else {
                        continue;
                    };
                    if p.values.len() / 3 == 0 || tri.values.is_empty() {
                        continue;
                    }
                    let PdxValues::F32(positions) = &p.values else {
                        return Err(NativePdxCollisionRefusal::PropertyType {
                            source,
                            property: "p",
                        });
                    };
                    let PdxValues::I32(indices) = &tri.values else {
                        return Err(NativePdxCollisionRefusal::PropertyType {
                            source,
                            property: "tri",
                        });
                    };
                    // Native copies all floats into storage sized by count/3. A remainder
                    // is not safe geometry even if a later source mesh would replace it.
                    if positions.len() % 3 != 0 {
                        return Err(NativePdxCollisionRefusal::PositionTriple {
                            source,
                            float_count: positions.len(),
                        });
                    }
                    selected = Some((source, positions, indices));
                }
            }
        }
        let Some((source, positions, raw_indices)) = selected else {
            return Ok(None);
        };
        if raw_indices.len() % 3 != 0 {
            return Err(NativePdxCollisionRefusal::IndexTriple {
                source,
                index_count: raw_indices.len(),
            });
        }
        let vertex_count = positions.len() / 3;
        let mut indices = Vec::new();
        indices.try_reserve_exact(raw_indices.len()).map_err(|_| {
            NativePdxCollisionRefusal::AllocationUnavailable {
                source,
                stream: "indices",
            }
        })?;
        for (ordinal, raw) in raw_indices.iter().enumerate() {
            let index = *raw as u16;
            if usize::from(index) >= vertex_count {
                return Err(NativePdxCollisionRefusal::IndexOutOfBounds {
                    source,
                    ordinal,
                    index,
                    vertex_count,
                });
            }
            indices.push(index);
        }
        let mut scaled_positions = Vec::new();
        scaled_positions
            .try_reserve_exact(vertex_count)
            .map_err(|_| NativePdxCollisionRefusal::AllocationUnavailable {
                source,
                stream: "positions",
            })?;
        for p in positions.chunks_exact(3) {
            scaled_positions.push([
                authored_scale * p[0],
                authored_scale * p[1],
                authored_scale * p[2],
            ]);
        }
        Ok(Some(NativePdxCollision {
            source,
            scaled_positions,
            indices,
        }))
    }
}

pub fn parse_pdx_asset(bytes: &[u8]) -> Result<PdxAssetTree> {
    match bytes.get(..4) {
        Some(magic) if magic == PDX_ASSET_BINARY_MAGIC => parse_pdx_asset_binary(bytes),
        Some(magic) if magic == PDX_ASSET_TEXT_MAGIC => parse_pdx_asset_text(bytes),
        Some(magic) => Err(format!(
            "not a pdxasset stream: magic {:?} (wanted {:?} or {:?})",
            String::from_utf8_lossy(magic),
            String::from_utf8_lossy(PDX_ASSET_BINARY_MAGIC),
            String::from_utf8_lossy(PDX_ASSET_TEXT_MAGIC),
        )
        .into()),
        None => Err("not a pdxasset stream: fewer than 4 bytes".into()),
    }
}

fn finish_tree(mut stack: Vec<PdxNode>) -> Result<PdxAssetTree> {
    while stack.len() > 1 {
        let node = stack.pop().expect("non-root node");
        stack.last_mut().expect("parent node").children.push(node);
    }
    let root = stack.pop().expect("root node");
    let version = match root.property("pdxasset") {
        Some(PdxProperty {
            values: PdxValues::I32(values),
            ..
        }) => values.clone(),
        Some(other) => {
            return Err(format!(
                "pdxasset version property has type '{}', wanted 'i'",
                other.values.type_char()
            )
            .into())
        }
        None => return Err("missing pdxasset version property at root".into()),
    };
    if version != PDX_ASSET_SUPPORTED_VERSION {
        return Err(format!(
            "unsupported pdxasset version {version:?} (only {PDX_ASSET_SUPPORTED_VERSION:?} is \
             format-law-verified); refusing to guess"
        )
        .into());
    }
    Ok(PdxAssetTree { version, root })
}

fn push_object(stack: &mut Vec<PdxNode>, depth: usize, name: String, at: String) -> Result<()> {
    if depth == 0 || depth > stack.len() {
        return Err(format!(
            "object '{name}' {at} jumps to depth {depth} from depth {}",
            stack.len() - 1
        )
        .into());
    }
    while stack.len() > depth {
        let node = stack.pop().expect("non-root node");
        stack.last_mut().expect("parent node").children.push(node);
    }
    stack.push(PdxNode::new(name));
    Ok(())
}

struct BinCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> BinCursor<'a> {
    fn take(&mut self, len: usize, what: &str) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                format!(
                    "pdxasset binary truncated at byte {}: wanted {len} bytes for {what}, {} left",
                    self.pos,
                    self.bytes.len() - self.pos
                )
            })?;
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u8(&mut self, what: &str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    fn u32(&mut self, what: &str) -> Result<u32> {
        let raw = self.take(4, what)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }
}

fn parse_pdx_asset_binary(bytes: &[u8]) -> Result<PdxAssetTree> {
    let mut cursor = BinCursor { bytes, pos: 4 };
    let mut stack = vec![PdxNode::new("<root>".to_string())];
    while cursor.pos < bytes.len() {
        let at = cursor.pos;
        match cursor.u8("token tag")? {
            b'!' => {
                let name_len = cursor.u8("property name length")? as usize;
                if name_len == 0 {
                    return Err(format!("empty property name at byte {at}").into());
                }
                let name = utf8_owned(cursor.take(name_len, "property name")?, "property name")?;
                let type_char = cursor.u8("property type char")?;
                let values = match type_char {
                    b'i' => {
                        let count = cursor.u32("i32 count")? as usize;
                        let raw = cursor.take(
                            count.checked_mul(4).ok_or("i32 payload length overflows")?,
                            "i32 payload",
                        )?;
                        PdxValues::I32(
                            raw.chunks_exact(4)
                                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect(),
                        )
                    }
                    b'f' => {
                        let count = cursor.u32("f32 count")? as usize;
                        let raw = cursor.take(
                            count.checked_mul(4).ok_or("f32 payload length overflows")?,
                            "f32 payload",
                        )?;
                        PdxValues::F32(
                            raw.chunks_exact(4)
                                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect(),
                        )
                    }
                    b's' => {
                        let count = cursor.u32("string count")? as usize;
                        let remaining = cursor.bytes.len() - cursor.pos;
                        let mut values = Vec::with_capacity(count.min(remaining / 5));
                        for index in 0..count {
                            let len = cursor.u32("string byte length")? as usize;
                            let raw = cursor.take(len, "string payload")?;
                            let Some((&0, body)) = raw.split_last() else {
                                return Err(format!(
                                    "string {index} of property '{name}' at byte {at} is not \
                                     NUL-terminated"
                                )
                                .into());
                            };
                            if body.contains(&0) {
                                return Err(format!(
                                    "string {index} of property '{name}' at byte {at} has an \
                                     interior NUL"
                                )
                                .into());
                            }
                            values.push(utf8_owned(body, "string payload")?);
                        }
                        PdxValues::Str(values)
                    }
                    other => {
                        return Err(format!(
                            "unknown pdxasset value type char {:?} for property '{name}' at byte \
                             {at}; refusing to skip",
                            other as char
                        )
                        .into())
                    }
                };
                stack
                    .last_mut()
                    .expect("current node")
                    .properties
                    .push(PdxProperty { name, values });
            }
            b'[' => {
                let mut depth = 1usize;
                while cursor.pos < bytes.len() && bytes[cursor.pos] == b'[' {
                    depth += 1;
                    cursor.pos += 1;
                }
                let rest = &bytes[cursor.pos..];
                let nul = rest
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| format!("unterminated object name at byte {at}"))?;
                let name = utf8_owned(&rest[..nul], "object name")?;
                cursor.pos += nul + 1;
                push_object(&mut stack, depth, name, format!("at byte {at}"))?;
            }
            other => {
                return Err(format!(
                    "unexpected byte 0x{other:02x} at offset {at}: not a property ('!') or \
                     object ('[') tag"
                )
                .into())
            }
        }
    }
    finish_tree(stack)
}

fn utf8_owned(bytes: &[u8], what: &str) -> Result<String> {
    std::str::from_utf8(bytes)
        .map(|value| value.to_string())
        .map_err(|err| format!("{what} is not UTF-8: {err}").into())
}


fn parse_pdx_asset_text(bytes: &[u8]) -> Result<PdxAssetTree> {
    let text = std::str::from_utf8(&bytes[4..])
        .map_err(|err| format!("pdxasset text stream is not UTF-8: {err}"))?;
    let mut tokens = TextTokens { rest: text };
    let mut stack = vec![PdxNode::new("<root>".to_string())];
    while let Some(token) = tokens.next_token()? {
        if let Some(stripped) = token.strip_prefix('[') {
            let leading = token.len() - token.trim_start_matches('[').len();
            let name = stripped.trim_start_matches('[').trim_end_matches(']');
            let trailing = token.len() - token.trim_end_matches(']').len();
            if leading != trailing || name.is_empty() {
                return Err(format!("malformed text object token {token:?}").into());
            }
            push_object(
                &mut stack,
                leading,
                name.to_string(),
                format!("(text token {token:?})"),
            )?;
            continue;
        }
        let name = token.to_string();
        let type_token = tokens
            .next_token()?
            .ok_or_else(|| format!("property '{name}': missing type token"))?;
        let count_token = tokens
            .next_token()?
            .ok_or_else(|| format!("property '{name}': missing count token"))?;
        let count: usize = count_token
            .parse()
            .map_err(|err| format!("property '{name}': bad count {count_token:?}: {err}"))?;
        let values = match type_token {
            "i" => {
                let mut values = Vec::with_capacity(count.min(tokens.rest.len()));
                for _ in 0..count {
                    let raw = tokens
                        .next_token()?
                        .ok_or_else(|| format!("property '{name}': missing i32 value"))?;
                    values.push(
                        raw.parse::<i32>()
                            .map_err(|err| format!("property '{name}': bad i32 {raw:?}: {err}"))?,
                    );
                }
                PdxValues::I32(values)
            }
            "f" => {
                let mut values = Vec::with_capacity(count.min(tokens.rest.len()));
                for _ in 0..count {
                    let raw = tokens
                        .next_token()?
                        .ok_or_else(|| format!("property '{name}': missing f32 value"))?;
                    values.push(
                        raw.parse::<f32>()
                            .map_err(|err| format!("property '{name}': bad f32 {raw:?}: {err}"))?,
                    );
                }
                PdxValues::F32(values)
            }
            "s" => {
                let mut values = Vec::with_capacity(count.min(tokens.rest.len()));
                for _ in 0..count {
                    let raw = tokens
                        .next_token()?
                        .ok_or_else(|| format!("property '{name}': missing string value"))?;
                    let inner = raw
                        .strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                        .ok_or_else(|| {
                            format!("property '{name}': string {raw:?} is not quoted")
                        })?;
                    values.push(inner.to_string());
                }
                PdxValues::Str(values)
            }
            other => {
                return Err(format!(
                    "unknown pdxasset value type token {other:?} for property '{name}'; \
                     refusing to skip"
                )
                .into())
            }
        };
        stack
            .last_mut()
            .expect("current node")
            .properties
            .push(PdxProperty { name, values });
    }
    finish_tree(stack)
}

struct TextTokens<'a> {
    rest: &'a str,
}

impl<'a> TextTokens<'a> {
    fn next_token(&mut self) -> Result<Option<&'a str>> {
        self.rest = self.rest.trim_start();
        if self.rest.is_empty() {
            return Ok(None);
        }
        if self.rest.starts_with('"') {
            let Some(end) = self.rest[1..].find('"') else {
                return Err("unterminated quoted string in pdxasset text stream".into());
            };
            let token = &self.rest[..end + 2];
            self.rest = &self.rest[end + 2..];
            return Ok(Some(token));
        }
        let end = self
            .rest
            .find(char::is_whitespace)
            .unwrap_or(self.rest.len());
        let token = &self.rest[..end];
        self.rest = &self.rest[end..];
        Ok(Some(token))
    }
}


#[derive(Clone, Debug, PartialEq)]
pub struct PdxMaterial {
    pub shader: String,
    pub diffuse: Option<String>,
    pub normal_map: Option<String>,
    pub specular: Option<String>,
}

impl PdxMaterial {
    pub fn is_collision(&self) -> bool {
        self.shader == "Collision"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxSkin {
    pub influences_per_vertex: i32,
    pub joint_indices: Vec<i32>,
    pub weights: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxJoint {
    pub name: String,
    pub index: i32,
    pub parent: Option<i32>,
    pub transform_3x4: [f32; 12],
    pub rcf: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxLocator {
    pub name: String,
    pub position: [f32; 3],
    pub rotation_quat: [f32; 4],
    pub parent_bone: Option<String>,
    pub transform_4x4: Option<[f32; 16]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxMeshData {
    /// Original shape-child slot, including skipped Collision and non-mesh children.
    pub source_mesh_ordinal: usize,
    pub verts: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub uv0: Option<Vec<[f32; 2]>>,
    pub uv1: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub bounding_sphere: Option<[f32; 4]>,
    pub material: PdxMaterial,
    pub skin: Option<PdxSkin>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxShape {
    pub name: String,
    pub lod: Option<i32>,
    pub meshes: Vec<PdxMeshData>,
    pub skeleton: Vec<PdxJoint>,
}

impl PdxShape {
    pub fn mesh_by_source_ordinal(&self, ordinal: usize) -> Option<&PdxMeshData> {
        self.meshes.iter().find(|mesh| mesh.source_mesh_ordinal == ordinal)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxModel {
    pub lod_distances: Vec<f32>,
    pub shapes: Vec<PdxShape>,
    pub locators: Vec<PdxLocator>,
}

struct UnknownCollector {
    unknowns: Vec<String>,
}

impl UnknownCollector {
    fn new() -> Self {
        Self {
            unknowns: Vec::new(),
        }
    }

    fn property(&mut self, path: &str, prop: &PdxProperty) {
        self.unknowns.push(format!(
            "property {path} :: {} (type '{}', {} values)",
            prop.name,
            prop.values.type_char(),
            prop.values.len()
        ));
    }

    fn object(&mut self, path: &str, node: &PdxNode) {
        self.unknowns.push(format!("object {path}/{}", node.name));
    }

    fn finish(self, what: &str) -> Result<()> {
        if self.unknowns.is_empty() {
            return Ok(());
        }
        let mut message = format!(
            "{what}: {} unrecognized structure(s); failing closed rather than skipping:",
            self.unknowns.len()
        );
        for unknown in &self.unknowns {
            let _ = write!(message, "\n  {unknown}");
        }
        Err(message.into())
    }
}

fn floats_exact(node_path: &str, prop: &PdxProperty, want: usize) -> Result<Vec<f32>> {
    match &prop.values {
        PdxValues::F32(values) if values.len() == want => Ok(values.clone()),
        PdxValues::F32(values) => Err(format!(
            "{node_path} :: {}: wanted {want} floats, got {}",
            prop.name,
            values.len()
        )
        .into()),
        other => Err(format!(
            "{node_path} :: {}: wanted floats, got type '{}'",
            prop.name,
            other.type_char()
        )
        .into()),
    }
}

fn ints_exact(node_path: &str, prop: &PdxProperty, want: usize) -> Result<Vec<i32>> {
    match &prop.values {
        PdxValues::I32(values) if values.len() == want => Ok(values.clone()),
        PdxValues::I32(values) => Err(format!(
            "{node_path} :: {}: wanted {want} ints, got {}",
            prop.name,
            values.len()
        )
        .into()),
        other => Err(format!(
            "{node_path} :: {}: wanted ints, got type '{}'",
            prop.name,
            other.type_char()
        )
        .into()),
    }
}

fn single_string(node_path: &str, prop: &PdxProperty) -> Result<String> {
    match &prop.values {
        PdxValues::Str(values) if values.len() == 1 => Ok(values[0].clone()),
        other => Err(format!(
            "{node_path} :: {}: wanted exactly one string, got type '{}' with {} values",
            prop.name,
            other.type_char(),
            other.len()
        )
        .into()),
    }
}

fn fixed<const N: usize>(node_path: &str, name: &str, values: &[f32]) -> Result<[f32; N]> {
    values.try_into().map_err(|_| {
        format!(
            "{node_path} :: {name}: wanted {N} floats, got {}",
            values.len()
        )
        .into()
    })
}

pub fn extract_pdx_model(tree: &PdxAssetTree) -> Result<PdxModel> {
    let mut unknown = UnknownCollector::new();
    let mut model = PdxModel {
        lod_distances: Vec::new(),
        shapes: Vec::new(),
        locators: Vec::new(),
    };

    for prop in &tree.root.properties {
        if prop.name != "pdxasset" {
            unknown.property("<root>", prop);
        }
    }
    for top in &tree.root.children {
        match top.name.as_str() {
            "object" => {
                for prop in &top.properties {
                    match prop.name.as_str() {
                        "loddist" => match &prop.values {
                            PdxValues::F32(values) => model.lod_distances = values.clone(),
                            _ => unknown.property("object", prop),
                        },
                        _ => unknown.property("object", prop),
                    }
                }
                for shape_node in &top.children {
                    model.shapes.push(extract_shape(shape_node, &mut unknown)?);
                }
            }
            "locator" => {
                if !top.properties.is_empty() {
                    for prop in &top.properties {
                        unknown.property("locator", prop);
                    }
                }
                for locator_node in &top.children {
                    if let Some(locator) = extract_locator(locator_node, &mut unknown)? {
                        model.locators.push(locator);
                    }
                }
            }
            _ => unknown.object("<root>", top),
        }
    }
    unknown.finish("pdxasset mesh extraction")?;
    Ok(model)
}

fn extract_shape(shape_node: &PdxNode, unknown: &mut UnknownCollector) -> Result<PdxShape> {
    let path = format!("object/{}", shape_node.name);
    let mut shape = PdxShape {
        name: shape_node.name.clone(),
        lod: None,
        meshes: Vec::new(),
        skeleton: Vec::new(),
    };
    for prop in &shape_node.properties {
        match prop.name.as_str() {
            "lod" => shape.lod = Some(ints_exact(&path, prop, 1)?[0]),
            _ => unknown.property(&path, prop),
        }
    }
    for (source_mesh_ordinal, child) in shape_node.children.iter().enumerate() {
        match child.name.as_str() {
            "mesh" => {
                if native_collision_shader(child) { continue; }
                shape.meshes.push(extract_mesh(&path, child, source_mesh_ordinal, unknown)?);
            }
            "skeleton" => {
                if !child.properties.is_empty() {
                    for prop in &child.properties {
                        unknown.property(&format!("{path}/skeleton"), prop);
                    }
                }
                for joint_node in &child.children {
                    shape
                        .skeleton
                        .push(extract_joint(&path, joint_node, unknown)?);
                }
            }
            _ => unknown.object(&path, child),
        }
    }
    Ok(shape)
}

fn extract_mesh(
    shape_path: &str,
    mesh_node: &PdxNode,
    source_mesh_ordinal: usize,
    unknown: &mut UnknownCollector,
) -> Result<PdxMeshData> {
    let path = format!("{shape_path}/mesh");
    let mut positions: Option<Vec<f32>> = None;
    let mut normals: Option<Vec<f32>> = None;
    let mut tangents: Option<Vec<f32>> = None;
    let mut uv0: Option<Vec<f32>> = None;
    let mut uv1: Option<Vec<f32>> = None;
    let mut indices: Option<Vec<i32>> = None;
    let mut bounding_sphere = None;
    for prop in &mesh_node.properties {
        match (prop.name.as_str(), &prop.values) {
            ("p", PdxValues::F32(values)) => positions = Some(values.clone()),
            ("n", PdxValues::F32(values)) => normals = Some(values.clone()),
            ("ta", PdxValues::F32(values)) => tangents = Some(values.clone()),
            ("u0", PdxValues::F32(values)) => uv0 = Some(values.clone()),
            ("u1", PdxValues::F32(values)) => uv1 = Some(values.clone()),
            ("tri", PdxValues::I32(values)) => indices = Some(values.clone()),
            ("boundingsphere", PdxValues::F32(values)) => {
                bounding_sphere = Some(fixed::<4>(&path, "boundingsphere", values)?)
            }
            _ => unknown.property(&path, prop),
        }
    }

    let positions = positions.ok_or_else(|| format!("{path}: missing p (positions)"))?;
    if positions.len() % 3 != 0 {
        return Err(format!(
            "{path}: p has {} floats, not a multiple of 3",
            positions.len()
        )
        .into());
    }
    let vert_count = positions.len() / 3;
    let verts = positions
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect::<Vec<_>>();

    let normals = per_vertex(&path, "n", normals, vert_count, 3)?
        .map(|values| values.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect());
    let tangents = per_vertex(&path, "ta", tangents, vert_count, 4)?.map(|values| {
        values
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect()
    });
    let uv0 = per_vertex(&path, "u0", uv0, vert_count, 2)?
        .map(|values| values.chunks_exact(2).map(|c| [c[0], c[1]]).collect());
    let uv1 = per_vertex(&path, "u1", uv1, vert_count, 2)?
        .map(|values| values.chunks_exact(2).map(|c| [c[0], c[1]]).collect());

    let raw_indices = indices.ok_or_else(|| format!("{path}: missing tri (indices)"))?;
    if raw_indices.len() % 3 != 0 {
        return Err(format!(
            "{path}: tri has {} indices, not a multiple of 3",
            raw_indices.len()
        )
        .into());
    }
    let mut tri = Vec::with_capacity(raw_indices.len());
    for &index in &raw_indices {
        if index < 0 || index as usize >= vert_count {
            return Err(format!(
                "{path}: triangle index {index} out of bounds for {vert_count} vertices"
            )
            .into());
        }
        tri.push(index as u32);
    }

    let mut aabb = None;
    let mut material = None;
    let mut skin = None;
    for child in &mesh_node.children {
        match child.name.as_str() {
            "aabb" => aabb = Some(extract_aabb(&path, child, unknown)?),
            "material" => material = Some(extract_material(&path, child, unknown)?),
            "skin" => skin = Some(extract_skin(&path, child, vert_count, unknown)?),
            _ => unknown.object(&path, child),
        }
    }
    let (aabb_min, aabb_max) = aabb.unwrap_or(([0.0; 3], [0.0; 3]));
    let material = material
        .ok_or_else(|| format!("{path}: missing material node (present in every shipped mesh)"))?;

    Ok(PdxMeshData {
        source_mesh_ordinal,
        verts,
        normals,
        tangents,
        uv0,
        uv1,
        indices: tri,
        aabb_min,
        aabb_max,
        bounding_sphere,
        material,
        skin,
    })
}

fn per_vertex(
    path: &str,
    name: &str,
    values: Option<Vec<f32>>,
    vert_count: usize,
    components: usize,
) -> Result<Option<Vec<f32>>> {
    let Some(values) = values else {
        return Ok(None);
    };
    if values.len() != vert_count * components {
        return Err(format!(
            "{path}: {name} has {} floats, wanted {components} per vertex x {vert_count}",
            values.len()
        )
        .into());
    }
    Ok(Some(values))
}

fn extract_aabb(
    mesh_path: &str,
    node: &PdxNode,
    unknown: &mut UnknownCollector,
) -> Result<([f32; 3], [f32; 3])> {
    let path = format!("{mesh_path}/aabb");
    let mut min = None;
    let mut max = None;
    for prop in &node.properties {
        match prop.name.as_str() {
            "min" => min = Some(fixed::<3>(&path, "min", &floats_exact(&path, prop, 3)?)?),
            "max" => max = Some(fixed::<3>(&path, "max", &floats_exact(&path, prop, 3)?)?),
            _ => unknown.property(&path, prop),
        }
    }
    for child in &node.children {
        unknown.object(&path, child);
    }
    match (min, max) {
        (Some(min), Some(max)) => Ok((min, max)),
        _ => Err(format!("{path}: needs both min and max").into()),
    }
}

fn extract_material(
    mesh_path: &str,
    node: &PdxNode,
    unknown: &mut UnknownCollector,
) -> Result<PdxMaterial> {
    let path = format!("{mesh_path}/material");
    let mut shader = None;
    let mut diffuse = None;
    let mut normal_map = None;
    let mut specular = None;
    for prop in &node.properties {
        match prop.name.as_str() {
            "shader" => shader = Some(single_string(&path, prop)?),
            "diff" => diffuse = Some(single_string(&path, prop)?),
            "n" => normal_map = Some(single_string(&path, prop)?),
            "spec" => specular = Some(single_string(&path, prop)?),
            _ => unknown.property(&path, prop),
        }
    }
    for child in &node.children {
        unknown.object(&path, child);
    }
    Ok(PdxMaterial {
        shader: shader.ok_or_else(|| format!("{path}: missing shader"))?,
        diffuse,
        normal_map,
        specular,
    })
}

fn extract_skin(
    mesh_path: &str,
    node: &PdxNode,
    vert_count: usize,
    unknown: &mut UnknownCollector,
) -> Result<PdxSkin> {
    let path = format!("{mesh_path}/skin");
    let mut bones = None;
    let mut joint_indices = None;
    let mut weights = None;
    for prop in &node.properties {
        match (prop.name.as_str(), &prop.values) {
            ("bones", _) => bones = Some(ints_exact(&path, prop, 1)?[0]),
            ("ix", PdxValues::I32(values)) => joint_indices = Some(values.clone()),
            ("w", PdxValues::F32(values)) => weights = Some(values.clone()),
            _ => unknown.property(&path, prop),
        }
    }
    for child in &node.children {
        unknown.object(&path, child);
    }
    let influences_per_vertex = bones.ok_or_else(|| format!("{path}: missing bones"))?;
    if !(1..=4).contains(&influences_per_vertex) {
        return Err(format!(
            "{path}: bones={influences_per_vertex} outside the verified 1..=4 range"
        )
        .into());
    }
    let joint_indices = joint_indices.ok_or_else(|| format!("{path}: missing ix"))?;
    let weights = weights.ok_or_else(|| format!("{path}: missing w"))?;
    if joint_indices.len() != vert_count * 4 || weights.len() != vert_count * 4 {
        return Err(format!(
            "{path}: skin arrays must be 4 per vertex (ix {} w {} for {vert_count} verts)",
            joint_indices.len(),
            weights.len()
        )
        .into());
    }
    Ok(PdxSkin {
        influences_per_vertex,
        joint_indices,
        weights,
    })
}

fn extract_joint(
    shape_path: &str,
    node: &PdxNode,
    unknown: &mut UnknownCollector,
) -> Result<PdxJoint> {
    let path = format!("{shape_path}/skeleton/{}", node.name);
    let mut index = None;
    let mut parent = None;
    let mut transform = None;
    let mut rcf = None;
    for prop in &node.properties {
        match prop.name.as_str() {
            "ix" => index = Some(ints_exact(&path, prop, 1)?[0]),
            "pa" => parent = Some(ints_exact(&path, prop, 1)?[0]),
            "tx" => {
                let values = floats_exact(&path, prop, 12)?;
                transform = Some(fixed::<12>(&path, "tx", &values)?);
            }
            "rcf" => rcf = Some(ints_exact(&path, prop, 1)?[0]),
            _ => unknown.property(&path, prop),
        }
    }
    for child in &node.children {
        unknown.object(&path, child);
    }
    Ok(PdxJoint {
        name: node.name.clone(),
        index: index.ok_or_else(|| format!("{path}: missing ix"))?,
        parent,
        transform_3x4: transform.ok_or_else(|| format!("{path}: missing tx"))?,
        rcf,
    })
}

fn extract_locator(node: &PdxNode, unknown: &mut UnknownCollector) -> Result<Option<PdxLocator>> {
    let path = format!("locator/{}", node.name);
    let mut position = None;
    let mut rotation = None;
    let mut parent_bone = None;
    let mut transform = None;
    for prop in &node.properties {
        match prop.name.as_str() {
            "p" => position = Some(fixed::<3>(&path, "p", &floats_exact(&path, prop, 3)?)?),
            "q" => rotation = Some(fixed::<4>(&path, "q", &floats_exact(&path, prop, 4)?)?),
            "pa" => parent_bone = Some(single_string(&path, prop)?),
            "tx" => {
                let values = floats_exact(&path, prop, 16)?;
                transform = Some(fixed::<16>(&path, "tx", &values)?);
            }
            _ => unknown.property(&path, prop),
        }
    }
    for child in &node.children {
        unknown.object(&path, child);
    }
    let (Some(position), Some(rotation)) = (position, rotation) else {
        return Err(format!("{path}: needs both p and q").into());
    };
    Ok(Some(PdxLocator {
        name: node.name.clone(),
        position,
        rotation_quat: rotation,
        parent_bone,
        transform_4x4: transform,
    }))
}


#[derive(Clone, Debug, PartialEq)]
pub struct PdxAnimJoint {
    pub name: String,
    pub channels: String,
    pub base_translation: [f32; 3],
    pub base_rotation_quat: [f32; 4],
    pub base_scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdxAnimData {
    pub fps: f32,
    pub sample_count: i32,
    pub joints: Vec<PdxAnimJoint>,
    pub samples_t: Vec<f32>,
    pub samples_q: Vec<f32>,
    pub samples_s: Vec<f32>,
}

pub fn extract_pdx_anim(tree: &PdxAssetTree) -> Result<PdxAnimData> {
    let mut unknown = UnknownCollector::new();
    let mut info = None;
    let mut samples_t = Vec::new();
    let mut samples_q = Vec::new();
    let mut samples_s = Vec::new();
    for prop in &tree.root.properties {
        if prop.name != "pdxasset" {
            unknown.property("<root>", prop);
        }
    }
    for top in &tree.root.children {
        match top.name.as_str() {
            "info" => info = Some(extract_anim_info(top, &mut unknown)?),
            "samples" => {
                for prop in &top.properties {
                    match (prop.name.as_str(), &prop.values) {
                        ("t", PdxValues::F32(values)) => samples_t = values.clone(),
                        ("q", PdxValues::F32(values)) => samples_q = values.clone(),
                        ("s", PdxValues::F32(values)) => samples_s = values.clone(),
                        _ => unknown.property("samples", prop),
                    }
                }
                for child in &top.children {
                    unknown.object("samples", child);
                }
            }
            _ => unknown.object("<root>", top),
        }
    }
    unknown.finish("pdxasset anim extraction")?;
    let (fps, sample_count, joint_count, joints) =
        info.ok_or("pdxasset anim: missing info node")?;
    if joints.len() != joint_count as usize {
        return Err(format!(
            "pdxasset anim: info.j={joint_count} but {} joint nodes",
            joints.len()
        )
        .into());
    }
    let count = |flag: char| joints.iter().filter(|j| j.channels.contains(flag)).count();
    let expect = [
        ("t", samples_t.len(), 3 * sample_count as usize * count('t')),
        ("q", samples_q.len(), 4 * sample_count as usize * count('q')),
        ("s", samples_s.len(), sample_count as usize * count('s')),
    ];
    for (channel, got, want) in expect {
        if got != want {
            return Err(format!(
                "pdxasset anim: samples.{channel} has {got} floats, law requires {want} \
                 (sample_count {sample_count})"
            )
            .into());
        }
    }
    Ok(PdxAnimData {
        fps,
        sample_count,
        joints,
        samples_t,
        samples_q,
        samples_s,
    })
}

#[allow(clippy::type_complexity)]
fn extract_anim_info(
    node: &PdxNode,
    unknown: &mut UnknownCollector,
) -> Result<(f32, i32, i32, Vec<PdxAnimJoint>)> {
    let path = "info";
    let mut fps = None;
    let mut sample_count = None;
    let mut joint_count = None;
    for prop in &node.properties {
        match prop.name.as_str() {
            "fps" => fps = Some(floats_exact(path, prop, 1)?[0]),
            "sa" => sample_count = Some(ints_exact(path, prop, 1)?[0]),
            "j" => joint_count = Some(ints_exact(path, prop, 1)?[0]),
            _ => unknown.property(path, prop),
        }
    }
    let mut joints = Vec::new();
    for joint_node in &node.children {
        let joint_path = format!("info/{}", joint_node.name);
        let mut channels = None;
        let mut base_t = None;
        let mut base_q = None;
        let mut base_s = None;
        for prop in &joint_node.properties {
            match prop.name.as_str() {
                "sa" => channels = Some(single_string(&joint_path, prop)?),
                "t" => {
                    base_t = Some(fixed::<3>(
                        &joint_path,
                        "t",
                        &floats_exact(&joint_path, prop, 3)?,
                    )?)
                }
                "q" => {
                    base_q = Some(fixed::<4>(
                        &joint_path,
                        "q",
                        &floats_exact(&joint_path, prop, 4)?,
                    )?)
                }
                "s" => base_s = Some(floats_exact(&joint_path, prop, 1)?[0]),
                _ => unknown.property(&joint_path, prop),
            }
        }
        for child in &joint_node.children {
            unknown.object(&joint_path, child);
        }
        let channels = channels.ok_or_else(|| format!("{joint_path}: missing sa"))?;
        if channels.chars().any(|flag| !"tqs".contains(flag)) {
            return Err(format!(
                "{joint_path}: channel flags {channels:?} outside the verified \"tqs\" alphabet"
            )
            .into());
        }
        joints.push(PdxAnimJoint {
            name: joint_node.name.clone(),
            channels,
            base_translation: base_t.ok_or_else(|| format!("{joint_path}: missing t"))?,
            base_rotation_quat: base_q.ok_or_else(|| format!("{joint_path}: missing q"))?,
            base_scale: base_s.ok_or_else(|| format!("{joint_path}: missing s"))?,
        });
    }
    Ok((
        fps.ok_or("info: missing fps")?,
        sample_count.ok_or("info: missing sa")?,
        joint_count.ok_or("info: missing j")?,
        joints,
    ))
}


#[derive(Clone, Debug)]
pub struct PdxMeshValidationRow {
    pub shape: String,
    pub mesh_index: usize,
    pub material_shader: String,
    pub vert_count: usize,
    pub triangle_count: usize,
    pub max_index: u32,
    pub indices_in_bounds: bool,
    pub aabb_contains_positions: bool,
    pub max_aabb_deviation: f32,
    pub uv_min: Option<f32>,
    pub uv_max: Option<f32>,
}

pub fn validate_pdx_model(model: &PdxModel) -> Vec<PdxMeshValidationRow> {
    let mut rows = Vec::new();
    for shape in &model.shapes {
        for mesh in &shape.meshes {
            let mesh_index = mesh.source_mesh_ordinal;
            let max_index = mesh.indices.iter().copied().max().unwrap_or(0);
            let mut deviation = 0f32;
            for vert in &mesh.verts {
                for axis in 0..3 {
                    deviation = deviation
                        .max(mesh.aabb_min[axis] - vert[axis])
                        .max(vert[axis] - mesh.aabb_max[axis]);
                }
            }
            let mut uv_min = None;
            let mut uv_max = None;
            for uvs in [&mesh.uv0, &mesh.uv1].into_iter().flatten() {
                for uv in uvs {
                    for &component in uv {
                        uv_min = Some(uv_min.map_or(component, |v: f32| v.min(component)));
                        uv_max = Some(uv_max.map_or(component, |v: f32| v.max(component)));
                    }
                }
            }
            rows.push(PdxMeshValidationRow {
                shape: shape.name.clone(),
                mesh_index,
                material_shader: mesh.material.shader.clone(),
                vert_count: mesh.verts.len(),
                triangle_count: mesh.indices.len() / 3,
                max_index,
                indices_in_bounds: mesh.verts.is_empty() || (max_index as usize) < mesh.verts.len(),
                aabb_contains_positions: deviation <= 1e-3,
                max_aabb_deviation: deviation,
                uv_min,
                uv_max,
            });
        }
    }
    rows
}


pub fn pdx_model_from_resolver(resolver: &GameFileResolver, rel_path: &str) -> Result<PdxModel> {
    let path = resolver.resolve_required(rel_path, "pdxasset mesh")?;
    let bytes = resolver.read_required_bytes(rel_path, rel_path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let tree = parse_pdx_asset(&bytes)
        .map_err(|err| format!("{rel_path} (resolved {}): {err}", path.display()))?;
    extract_pdx_model(&tree).map_err(|err| format!("{rel_path}: {err}").into())
}

pub fn pdx_anim_from_resolver(resolver: &GameFileResolver, rel_path: &str) -> Result<PdxAnimData> {
    let path = resolver.resolve_required(rel_path, "pdxasset anim")?;
    let bytes = resolver.read_required_bytes(rel_path, rel_path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let tree = parse_pdx_asset(&bytes)
        .map_err(|err| format!("{rel_path} (resolved {}): {err}", path.display()))?;
    extract_pdx_anim(&tree).map_err(|err| format!("{rel_path}: {err}").into())
}

#[derive(Clone, Debug, Default)]
pub struct PdxAssetScanReport {
    pub mesh_files: usize,
    pub anim_files: usize,
    pub binary_files: usize,
    pub text_files: usize,
    pub mesh_failures: Vec<(String, String)>,
    pub anim_failures: Vec<(String, String)>,
    pub property_inventory: BTreeMap<(String, String, char), usize>,
    pub validation_rows: usize,
    pub validation_failures: Vec<(String, String)>,
}

impl PdxAssetScanReport {
    pub fn all_parsed(&self) -> bool {
        self.mesh_failures.is_empty()
            && self.anim_failures.is_empty()
            && self.validation_failures.is_empty()
    }
}

pub fn scan_pdx_assets_from_resolver(
    resolver: &GameFileResolver,
    rel_root: &str,
) -> Result<PdxAssetScanReport> {
    let mut report = PdxAssetScanReport::default();
    for extension in ["mesh", "anim"] {
        let files = resolver.resolve_dir_files_recursive(rel_root, Some(extension))?;
        for (rel, path) in files {
            let bytes =
                resolver.read_required_bytes(&rel, &rel).map_err(|err| format!("read {}: {err}", path.display()))?;
            match bytes.get(..4) {
                Some(magic) if magic == PDX_ASSET_BINARY_MAGIC => report.binary_files += 1,
                Some(magic) if magic == PDX_ASSET_TEXT_MAGIC => report.text_files += 1,
                _ => {}
            }
            let tree = match parse_pdx_asset(&bytes) {
                Ok(tree) => tree,
                Err(err) => {
                    if extension == "mesh" {
                        report.mesh_failures.push((rel, err.to_string()));
                        report.mesh_files += 1;
                    } else {
                        report.anim_failures.push((rel, err.to_string()));
                        report.anim_files += 1;
                    }
                    continue;
                }
            };
            inventory_node(&tree.root, "<root>", &mut report.property_inventory);
            if extension == "mesh" {
                report.mesh_files += 1;
                match extract_pdx_model(&tree) {
                    Ok(model) => {
                        for row in validate_pdx_model(&model) {
                            report.validation_rows += 1;
                            if !row.indices_in_bounds || !row.aabb_contains_positions {
                                report.validation_failures.push((
                                    rel.clone(),
                                    format!(
                                        "shape {} mesh {}: indices_in_bounds={} \
                                         aabb_contains={} (deviation {})",
                                        row.shape,
                                        row.mesh_index,
                                        row.indices_in_bounds,
                                        row.aabb_contains_positions,
                                        row.max_aabb_deviation
                                    ),
                                ));
                            }
                        }
                    }
                    Err(err) => report.mesh_failures.push((rel, err.to_string())),
                }
            } else {
                report.anim_files += 1;
                if let Err(err) = extract_pdx_anim(&tree) {
                    report.anim_failures.push((rel, err.to_string()));
                }
            }
        }
    }
    Ok(report)
}

fn inventory_node(
    node: &PdxNode,
    pattern: &str,
    inventory: &mut BTreeMap<(String, String, char), usize>,
) {
    for prop in &node.properties {
        *inventory
            .entry((
                pattern.to_string(),
                prop.name.clone(),
                prop.values.type_char(),
            ))
            .or_insert(0) += 1;
    }
    for child in &node.children {
        let child_pattern = child_pattern(pattern, &child.name);
        inventory_node(child, &child_pattern, inventory);
    }
}

fn child_pattern(parent_pattern: &str, child_name: &str) -> String {
    let structural = matches!(
        child_name,
        "object"
            | "locator"
            | "info"
            | "samples"
            | "mesh"
            | "aabb"
            | "material"
            | "skin"
            | "skeleton"
    );
    let tail = if structural { child_name } else { "<name>" };
    if parent_pattern == "<root>" {
        tail.to_string()
    } else {
        format!("{parent_pattern}/{tail}")
    }
}


pub fn pdx_model_to_obj(model: &PdxModel, source_label: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# pdxasset OBJ dump — {source_label}");
    let _ = writeln!(
        out,
        "# shapes {} locators {} lod_distances {:?}",
        model.shapes.len(),
        model.locators.len(),
        model.lod_distances
    );
    let mut vertex_base = 1usize;
    for shape in &model.shapes {
        for mesh in &shape.meshes {
            let mesh_index = mesh.source_mesh_ordinal;
            let _ = writeln!(
                out,
                "g {}_mesh{}_{}",
                sanitize_obj_name(&shape.name),
                mesh_index,
                sanitize_obj_name(&mesh.material.shader)
            );
            let _ = writeln!(
                out,
                "# material shader={} diff={:?} n={:?} spec={:?} lod={:?} skinned={}",
                mesh.material.shader,
                mesh.material.diffuse,
                mesh.material.normal_map,
                mesh.material.specular,
                shape.lod,
                mesh.skin.is_some()
            );
            for vert in &mesh.verts {
                let _ = writeln!(out, "v {} {} {}", vert[0], vert[1], vert[2]);
            }
            let has_uv = mesh.uv0.is_some();
            if let Some(uvs) = &mesh.uv0 {
                for uv in uvs {
                    let _ = writeln!(out, "vt {} {}", uv[0], 1.0 - uv[1]);
                }
            }
            let has_normals = mesh.normals.is_some();
            if let Some(normals) = &mesh.normals {
                for normal in normals {
                    let _ = writeln!(out, "vn {} {} {}", normal[0], normal[1], normal[2]);
                }
            }
            for triangle in mesh.indices.chunks_exact(3) {
                let face = triangle
                    .iter()
                    .map(|index| {
                        let global = vertex_base + *index as usize;
                        match (has_uv, has_normals) {
                            (true, true) => format!("{global}/{global}/{global}"),
                            (true, false) => format!("{global}/{global}"),
                            (false, true) => format!("{global}//{global}"),
                            (false, false) => format!("{global}"),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let _ = writeln!(out, "f {face}");
            }
            vertex_base += mesh.verts.len();
        }
    }
    for locator in &model.locators {
        let _ = writeln!(
            out,
            "# locator {} p=({}, {}, {}) parent={:?}",
            locator.name,
            locator.position[0],
            locator.position[1],
            locator.position[2],
            locator.parent_bone
        );
    }
    out
}

fn sanitize_obj_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}


#[cfg(test)]
pub(crate) mod test_fixtures {
    use super::PDX_ASSET_BINARY_MAGIC;

    pub(crate) fn collision_before_drawable_bytes() -> Vec<u8> {
        let mut bytes = PDX_ASSET_BINARY_MAGIC.to_vec();
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        push_object(&mut bytes, 1, "object");
        push_object(&mut bytes, 2, "mixedShape");
        push_object(&mut bytes, 3, "skeleton");
        push_object(&mut bytes, 3, "mesh");
        push_prop_f(&mut bytes, "p", &[100.0, 0.0, 0.0, 101.0, 0.0, 0.0, 100.0, 1.0, 0.0]);
        push_prop_i(&mut bytes, "tri", &[-65536, 65537, 65538]);
        push_prop_i(&mut bytes, "n", &[99]);
        push_prop_f(&mut bytes, "boundingsphere", &[99.0]);
        push_object(&mut bytes, 4, "material");
        push_prop_s(&mut bytes, "shader", "Collision");
        push_prop_i(&mut bytes, "diff", &[99]);
        push_mesh(&mut bytes, 4.0, false, false);
        bytes
    }

    pub(crate) fn append_collision(bytes: &mut Vec<u8>, positions: &[f32], indices: &[i32]) {
        push_object(bytes, 1, "object");
        push_object(bytes, 2, "collisionShape");
        push_object(bytes, 3, "mesh");
        push_prop_f(bytes, "p", positions);
        push_prop_i(bytes, "tri", indices);
        push_object(bytes, 4, "material");
        push_prop_s(bytes, "shader", "Collision");
    }

    pub(crate) fn push_prop_f(bytes: &mut Vec<u8>, name: &str, values: &[f32]) {
        bytes.push(b'!');
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'f');
        bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    pub(crate) fn push_prop_i(bytes: &mut Vec<u8>, name: &str, values: &[i32]) {
        bytes.push(b'!');
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'i');
        bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    pub(crate) fn push_prop_s(bytes: &mut Vec<u8>, name: &str, value: &str) {
        bytes.push(b'!');
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b's');
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&((value.len() + 1) as u32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }

    pub(crate) fn push_object(bytes: &mut Vec<u8>, depth: usize, name: &str) {
        bytes.extend(std::iter::repeat(b'[').take(depth));
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
    }

    /// The fixture mesh with a one-influence skin stream: the shape a729 packs
    /// through the `Skinned` column of its effect-suffix table, and the only
    /// shape on which the bind-pose and animated-entity arms are reachable.
    pub(crate) fn synthetic_skinned_mesh_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        push_object(&mut bytes, 1, "object");
        push_prop_f(&mut bytes, "loddist", &[50.0, 200.0]);
        push_object(&mut bytes, 2, "testShape");
        push_prop_i(&mut bytes, "lod", &[0]);
        push_object(&mut bytes, 3, "mesh");
        push_prop_f(
            &mut bytes,
            "p",
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        );
        push_prop_f(
            &mut bytes,
            "n",
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        );
        push_prop_f(
            &mut bytes,
            "ta",
            &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
        );
        push_prop_f(&mut bytes, "u0", &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        push_prop_i(&mut bytes, "tri", &[0, 2, 1]);
        push_object(&mut bytes, 4, "aabb");
        push_prop_f(&mut bytes, "min", &[0.0, 0.0, 0.0]);
        push_prop_f(&mut bytes, "max", &[1.0, 1.0, 0.0]);
        push_object(&mut bytes, 4, "material");
        push_prop_s(&mut bytes, "shader", "PdxMeshStandard");
        push_prop_s(&mut bytes, "diff", "test_diffuse.dds");
        push_object(&mut bytes, 4, "skin");
        push_prop_i(&mut bytes, "bones", &[1]);
        push_prop_i(&mut bytes, "ix", &[0; 12]);
        let mut weights = Vec::new();
        for _ in 0..3 {
            weights.extend_from_slice(&[1.0f32, 0.0, 0.0, 0.0]);
        }
        push_prop_f(&mut bytes, "w", &weights);
        bytes
    }

    /// The fixture mesh the whole materializer runs over, and the only shape on which several
    /// of its laws are reachable at all.
    pub(crate) fn synthetic_multi_submesh_mesh_bytes(skinned: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        push_object(&mut bytes, 1, "object");
        push_prop_f(&mut bytes, "loddist", &[50.0, 200.0]);
        // A shape at ANOTHER LOD first, so the shapes that ARE selected have
        // source indices 1 and 2 while their dense group ordinals are 0 and 1.
        push_object(&mut bytes, 2, "otherLodShape");
        push_prop_i(&mut bytes, "lod", &[1]);
        push_mesh(&mut bytes, 100.0, false, false);
        for (group, shape) in ["testShape", "secondShape"].into_iter().enumerate() {
            push_object(&mut bytes, 2, shape);
            push_prop_i(&mut bytes, "lod", &[0]);
            // ...and a collision mesh in front of each shape's drawable ones,
            // so their source ordinals are 1 and 2 while their dense submesh
            // ordinals are 0 and 1.
            push_mesh(&mut bytes, (group as f32 * 8.0) + 6.0, true, false);
            for ordinal in 0..2u8 {
                let base = (group as f32 * 8.0) + f32::from(ordinal) * 2.0;
                push_mesh(&mut bytes, base, false, skinned);
            }
        }
        bytes
    }

    /// One `mesh` node at a given position, drawable or collision, with or
    /// without a skin stream.
    fn push_mesh(bytes: &mut Vec<u8>, base: f32, collision: bool, skinned: bool) {
        push_object(bytes, 3, "mesh");
        push_prop_f(
            bytes,
            "p",
            &[base, 0.0, 0.0, base + 1.0, 0.0, 0.0, base, 1.0, 0.0],
        );
        push_prop_f(bytes, "n", &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        push_prop_f(
            bytes,
            "ta",
            &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
        );
        push_prop_f(bytes, "u0", &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        push_prop_i(bytes, "tri", &[0, 2, 1]);
        push_object(bytes, 4, "aabb");
        push_prop_f(bytes, "min", &[base, 0.0, 0.0]);
        push_prop_f(bytes, "max", &[base + 1.0, 1.0, 0.0]);
        push_object(bytes, 4, "material");
        push_prop_s(
            bytes,
            "shader",
            if collision { "Collision" } else { "PdxMeshStandard" },
        );
        push_prop_s(bytes, "diff", "test_diffuse.dds");
        if skinned && !collision {
            push_object(bytes, 4, "skin");
            push_prop_i(bytes, "bones", &[1]);
            push_prop_i(bytes, "ix", &[0; 12]);
            let mut weights = Vec::new();
            for _ in 0..3 {
                weights.extend_from_slice(&[1.0f32, 0.0, 0.0, 0.0]);
            }
            push_prop_f(bytes, "w", &weights);
        }
    }

    pub(crate) fn collision_before_drawable_aabb_bytes(
        bounds: Option<(&[f32], &[f32])>,
    ) -> Vec<u8> {
        let mut bytes = collision_before_drawable_bytes();
        let aabb_marker = b"[[[[aabb\0";
        let material_marker = b"[[[[material\0";
        assert_eq!(bytes.windows(aabb_marker.len()).filter(|w| *w == aabb_marker).count(), 1);
        let start = bytes.windows(aabb_marker.len()).position(|w| w == aabb_marker).unwrap();
        let end = start + bytes[start..].windows(material_marker.len())
            .position(|w| w == material_marker).unwrap();
        let mut replacement = Vec::new();
        if let Some((min, max)) = bounds {
            push_object(&mut replacement, 4, "aabb");
            push_prop_f(&mut replacement, "min", min);
            push_prop_f(&mut replacement, "max", max);
        }
        bytes.splice(start..end, replacement);
        bytes
    }

    pub(crate) fn synthetic_mesh_bytes(x_offset: f32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        push_object(&mut bytes, 1, "object");
        push_prop_f(&mut bytes, "loddist", &[50.0, 200.0]);
        push_object(&mut bytes, 2, "testShape");
        push_prop_i(&mut bytes, "lod", &[0]);
        push_object(&mut bytes, 3, "mesh");
        push_prop_f(
            &mut bytes,
            "p",
            &[
                x_offset,
                0.0,
                0.0,
                x_offset + 1.0,
                0.0,
                0.0,
                x_offset,
                1.0,
                0.0,
            ],
        );
        push_prop_f(
            &mut bytes,
            "n",
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        );
        push_prop_f(
            &mut bytes,
            "ta",
            &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
        );
        push_prop_f(&mut bytes, "u0", &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        push_prop_i(&mut bytes, "tri", &[0, 2, 1]);
        push_object(&mut bytes, 4, "aabb");
        push_prop_f(&mut bytes, "min", &[x_offset, 0.0, 0.0]);
        push_prop_f(&mut bytes, "max", &[x_offset + 1.0, 1.0, 0.0]);
        push_object(&mut bytes, 4, "material");
        push_prop_s(&mut bytes, "shader", "PdxMeshStandard");
        push_prop_s(&mut bytes, "diff", "test_diffuse.dds");
        push_object(&mut bytes, 1, "locator");
        push_object(&mut bytes, 2, "smoke_e");
        push_prop_f(&mut bytes, "p", &[0.5, 0.5, 0.5]);
        push_prop_f(&mut bytes, "q", &[0.0, 0.0, 0.0, 1.0]);
        push_prop_s(&mut bytes, "pa", "testBone");
        bytes
    }

    pub(crate) fn synthetic_anim_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        push_object(&mut bytes, 1, "info");
        push_prop_f(&mut bytes, "fps", &[15.0]);
        push_prop_i(&mut bytes, "sa", &[2]);
        push_prop_i(&mut bytes, "j", &[2]);
        push_object(&mut bytes, 2, "root_bone");
        push_prop_s(&mut bytes, "sa", "tq");
        push_prop_f(&mut bytes, "t", &[0.0, 1.0, 0.0]);
        push_prop_f(&mut bytes, "q", &[0.0, 0.0, 0.0, 1.0]);
        push_prop_f(&mut bytes, "s", &[1.0]);
        push_object(&mut bytes, 2, "still_bone");
        push_prop_s(&mut bytes, "sa", "");
        push_prop_f(&mut bytes, "t", &[0.0, 0.0, 0.0]);
        push_prop_f(&mut bytes, "q", &[0.0, 0.0, 0.0, 1.0]);
        push_prop_f(&mut bytes, "s", &[1.0]);
        push_object(&mut bytes, 1, "samples");
        push_prop_f(&mut bytes, "t", &[0.0, 1.0, 0.0, 0.0, 1.5, 0.0]);
        push_prop_f(&mut bytes, "q", &[0.0, 0.0, 0.0, 1.0, 0.1, 0.0, 0.0, 0.9]);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::{
        push_object, push_prop_f, push_prop_i, push_prop_s, synthetic_anim_bytes,
        synthetic_mesh_bytes,
    };
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::{Path, PathBuf};



    #[test]
    fn absent_whole_aabb_retains_six_positive_zeros_not_vertex_extrema() {
        let bytes = test_fixtures::collision_before_drawable_aabb_bytes(None);
        let tree = parse_pdx_asset(&bytes).unwrap();
        let model = extract_pdx_model(&tree).unwrap();
        let mesh = &model.shapes[0].meshes[0];
        assert_eq!(mesh.source_mesh_ordinal, 2);
        assert_eq!(mesh.aabb_min.map(f32::to_bits), [0; 3]);
        assert_eq!(mesh.aabb_max.map(f32::to_bits), [0; 3]);
        assert_eq!(mesh.verts[0], [4.0, 0.0, 0.0]);
        assert_eq!(mesh.verts[1], [5.0, 0.0, 0.0]);
        assert!(!tree.root.children[0].children[0].children[2].children.iter()
            .any(|node| node.name == "aabb"));
        assert_eq!(tree.native_collision(1.0).unwrap().unwrap().source.mesh_child, 1);
    }

    #[test]
    fn present_aabb_retains_authored_coordinates_and_negative_zero() {
        let min = [-0.0, -2.0, -3.0];
        let max = [8.0, 4.0, 6.0];
        let bytes = test_fixtures::collision_before_drawable_aabb_bytes(Some((&min, &max)));
        let tree = parse_pdx_asset(&bytes).unwrap();
        let model = extract_pdx_model(&tree).unwrap();
        let mesh = &model.shapes[0].meshes[0];
        assert_eq!(mesh.aabb_min.map(f32::to_bits), min.map(f32::to_bits));
        assert_eq!(mesh.aabb_max.map(f32::to_bits), max.map(f32::to_bits));
        assert_eq!(mesh.source_mesh_ordinal, 2);
        assert_eq!(mesh.verts[0], [4.0, 0.0, 0.0]);
    }

    #[test]
    fn present_aabb_missing_or_malformed_members_never_take_absent_branch() {
        for (min, max) in [(&[][..], &[1.0; 3][..]), (&[0.0; 2][..], &[1.0; 3][..]),
            (&[0.0; 4][..], &[1.0; 3][..]), (&[0.0; 3][..], &[1.0; 2][..])] {
            let bytes = test_fixtures::collision_before_drawable_aabb_bytes(Some((min, max)));
            let tree = parse_pdx_asset(&bytes).unwrap();
            assert!(extract_pdx_model(&tree).unwrap_err().to_string().contains("aabb"));
            assert!(tree.native_collision(1.0).unwrap().is_some());
        }
        for missing in ["min", "max"] {
            let bytes = test_fixtures::collision_before_drawable_bytes();
            let mut tree = parse_pdx_asset(&bytes).unwrap();
            let aabb = tree.root.children[0].children[0].children[2].children.iter_mut()
                .find(|node| node.name == "aabb").unwrap();
            aabb.properties.retain(|prop| prop.name != missing);
            assert!(extract_pdx_model(&tree).unwrap_err().to_string().contains("needs both min and max"));
        }
    }

    #[test]
    fn binary_round_trip_parses_tree_and_extracts_model() {
        let tree = parse_pdx_asset(&synthetic_mesh_bytes(0.0)).expect("parse synthetic mesh");
        assert_eq!(tree.version, vec![1, 0]);
        let model = extract_pdx_model(&tree).expect("extract synthetic mesh");
        assert_eq!(model.lod_distances, vec![50.0, 200.0]);
        assert_eq!(model.shapes.len(), 1);
        assert_eq!(model.shapes[0].name, "testShape");
        assert_eq!(model.shapes[0].lod, Some(0));
        let mesh = &model.shapes[0].meshes[0];
        assert_eq!(mesh.verts.len(), 3);
        assert_eq!(mesh.indices, vec![0, 2, 1]);
        assert_eq!(mesh.aabb_min, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.material.shader, "PdxMeshStandard");
        assert_eq!(mesh.material.diffuse.as_deref(), Some("test_diffuse.dds"));
        assert_eq!(model.locators.len(), 1);
        assert_eq!(model.locators[0].parent_bone.as_deref(), Some("testBone"));
        let rows = validate_pdx_model(&model);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].indices_in_bounds);
        assert!(rows[0].aabb_contains_positions);
    }

    #[test]
    fn text_twin_parses_to_the_same_shapes() {
        let text = br#"@@t@pdxasset i 2 1 0
[object]
	[[testShape]]
		[[[mesh]]]
			p f 9 0 0 0 1 0 0 0 1 0
			tri i 3 0 2 1
			[[[[aabb]]]]
				min f 3 0 0 0
				max f 3 1 1 0
			[[[[material]]]]
				shader s 1 "Collision"
[locator]
	[[node]]
		p f 3 0 0 0
		q f 4 0 0 0 1
"#;
        let tree = parse_pdx_asset(text).expect("parse text twin");
        let model = extract_pdx_model(&tree).expect("extract text twin");
        assert_eq!(model.shapes.len(), 1);
        assert!(model.shapes[0].meshes.is_empty());
        assert_eq!(tree.native_collision(1.0).unwrap().unwrap().indices, [0, 2, 1]);
        assert_eq!(model.locators.len(), 1);
    }

    #[test]
    fn anim_round_trip_validates_sample_law() {
        let tree = parse_pdx_asset(&synthetic_anim_bytes()).expect("parse synthetic anim");
        let anim = extract_pdx_anim(&tree).expect("extract synthetic anim");
        assert_eq!(anim.fps, 15.0);
        assert_eq!(anim.sample_count, 2);
        assert_eq!(anim.joints.len(), 2);
        assert_eq!(anim.joints[0].channels, "tq");
        assert_eq!(anim.samples_t.len(), 6);
        assert_eq!(anim.samples_q.len(), 8);
        assert!(anim.samples_s.is_empty());
    }

    #[test]
    fn anim_sample_count_mismatch_fails_closed() {
        let mut bytes = synthetic_anim_bytes();
        let q_prop_len = 1 + 1 + 1 + 1 + 4 + 4 * 8;
        bytes.truncate(bytes.len() - q_prop_len);
        push_prop_f(&mut bytes, "q", &[0.0, 0.0, 0.0, 1.0]);
        let tree = parse_pdx_asset(&bytes).expect("parse tampered anim");
        let err = extract_pdx_anim(&tree).expect_err("sample law violation");
        assert!(err.to_string().contains("samples.q"), "{err}");
    }

    #[test]
    fn unknown_property_fails_closed_with_name() {
        let mut bytes = synthetic_mesh_bytes(0.0);
        push_prop_f(&mut bytes, "mystery", &[1.0]);
        let tree = parse_pdx_asset(&bytes).expect("grammar still parses");
        let err = extract_pdx_model(&tree).expect_err("unknown property");
        let message = err.to_string();
        assert!(message.contains("mystery"), "{message}");
        assert!(message.contains("failing closed"), "{message}");
    }

    #[test]
    fn unknown_type_char_fails_at_grammar_level() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        bytes.push(b'!');
        bytes.push(3);
        bytes.extend_from_slice(b"bad");
        bytes.push(b'd');
        let err = parse_pdx_asset(&bytes).expect_err("unknown type char");
        assert!(
            err.to_string().contains("unknown pdxasset value type"),
            "{err}"
        );
    }

    #[test]
    fn truncated_payload_fails_with_offset() {
        let mut bytes = synthetic_mesh_bytes(0.0);
        bytes.truncate(bytes.len() - 3);
        let err = parse_pdx_asset(&bytes).expect_err("truncated stream");
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    #[test]
    fn binary_oversized_string_count_fails_closed_without_preallocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        bytes.push(b'!');
        bytes.push(4);
        bytes.extend_from_slice(b"huge");
        bytes.push(b's');
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let err = parse_pdx_asset(&bytes).expect_err("oversized string count");
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    #[test]
    fn text_oversized_count_fails_closed_without_preallocation() {
        let text = b"@@t@pdxasset i 2 1 0\nhuge f 4000000000\n";
        let err = parse_pdx_asset(text).expect_err("oversized text count");
        assert!(err.to_string().contains("missing f32 value"), "{err}");
    }

    #[test]
    fn unsupported_version_fails_closed() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[2, 0]);
        let err = parse_pdx_asset(&bytes).expect_err("future version");
        assert!(
            err.to_string().contains("unsupported pdxasset version"),
            "{err}"
        );
    }

    #[test]
    fn out_of_bounds_triangle_index_fails_closed() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PDX_ASSET_BINARY_MAGIC);
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        push_object(&mut bytes, 1, "object");
        push_object(&mut bytes, 2, "shape");
        push_object(&mut bytes, 3, "mesh");
        push_prop_f(&mut bytes, "p", &[0.0; 9]);
        push_prop_i(&mut bytes, "tri", &[0, 1, 7]);
        push_object(&mut bytes, 4, "aabb");
        push_prop_f(&mut bytes, "min", &[0.0; 3]);
        push_prop_f(&mut bytes, "max", &[0.0; 3]);
        push_object(&mut bytes, 4, "material");
        push_prop_s(&mut bytes, "shader", "PdxMeshStandard");
        let tree = parse_pdx_asset(&bytes).expect("grammar parses");
        let err = extract_pdx_model(&tree).expect_err("index out of bounds");
        assert!(err.to_string().contains("out of bounds"), "{err}");
    }


    fn write_bytes(root: &Path, rel: &str, bytes: &[u8]) {
        let path = rel
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, bytes).expect("write fixture file");
    }

    const TEST_MESH_REL: &str = "gfx/models/buildings/test_building.mesh";

    #[test]
    fn pdx_model_from_resolver_uses_mod_override() {
        let root = temp_test_dir("pdx_mesh_resolver_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_bytes(&vanilla, TEST_MESH_REL, &synthetic_mesh_bytes(0.0));
        write_bytes(&mod_root, TEST_MESH_REL, &synthetic_mesh_bytes(100.0));

        let resolver = crate::GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()])
            .expect("resolver");
        let model = pdx_model_from_resolver(&resolver, TEST_MESH_REL).expect("modded mesh");
        assert_eq!(
            model.shapes[0].meshes[0].verts[0],
            [100.0, 0.0, 0.0],
            "mod layer must win"
        );
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn pdx_model_from_resolver_honors_replace_path_exclusion() {
        let root = temp_test_dir("pdx_mesh_resolver_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_bytes(&vanilla, TEST_MESH_REL, &synthetic_mesh_bytes(0.0));
        write_bytes(
            &mod_root,
            "descriptor.mod",
            b"name=\"ReplaceModels\"\nreplace_path=\"gfx/models/buildings\"\n",
        );

        let resolver =
            crate::GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let err = pdx_model_from_resolver(&resolver, TEST_MESH_REL)
            .expect_err("replace_path removes vanilla mesh");
        assert!(err.to_string().contains("missing pdxasset mesh"), "{err}");
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn scan_uses_recursive_resolver_listing_with_mod_added_meshes() {
        let root = temp_test_dir("pdx_mesh_scan_recursive");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_bytes(&vanilla, TEST_MESH_REL, &synthetic_mesh_bytes(0.0));
        write_bytes(
            &mod_root,
            "gfx/models/new_units/added.mesh",
            &synthetic_mesh_bytes(5.0),
        );

        let resolver =
            crate::GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let report = scan_pdx_assets_from_resolver(&resolver, PDX_MODELS_ROOT).expect("scan");
        assert_eq!(
            report.mesh_files, 2,
            "vanilla + mod-added mesh both scanned"
        );
        assert!(report.all_parsed(), "failures: {:?}", report.mesh_failures);
        fs::remove_dir_all(root).ok();
    }


    fn vanilla_dir() -> PathBuf {
        replay_test_gates::require_vanilla_game_root()
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_a_all_vanilla_mesh_and_anim_files_parse() {
        let game_dir = vanilla_dir();
        let resolver = crate::GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let report =
            scan_pdx_assets_from_resolver(&resolver, PDX_MODELS_ROOT).expect("vanilla scan");
        assert!(
            report.mesh_files >= 400,
            "vanilla install should ship hundreds of meshes, saw {}",
            report.mesh_files
        );
        assert!(
            report.anim_files >= 500,
            "vanilla install should ship hundreds of anims, saw {}",
            report.anim_files
        );
        assert!(
            report.mesh_failures.is_empty(),
            "mesh failures: {:?}",
            report.mesh_failures
        );
        assert!(
            report.anim_failures.is_empty(),
            "anim failures: {:?}",
            report.anim_failures
        );
        assert!(
            report.validation_failures.is_empty(),
            "validation failures: {:?}",
            report.validation_failures
        );
        println!(
            "gate_a: {} mesh + {} anim vanilla files parsed ({} binary, {} text); {} \
             validation rows clean; {} distinct property patterns",
            report.mesh_files,
            report.anim_files,
            report.binary_files,
            report.text_files,
            report.validation_rows,
            report.property_inventory.len()
        );
        for ((pattern, name, type_char), count) in &report.property_inventory {
            println!("  inventory {count:8} {type_char} {pattern} :: {name}");
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_b_diverse_mesh_semantic_checks() {
        let game_dir = vanilla_dir();
        let resolver = crate::GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let picks = [
            ("building", "gfx/models/buildings/bunker.mesh"),
            ("unit", "gfx/models/units/ENG_infantry.mesh"),
            ("tree", "gfx/models/mapitems/trees/beech.mesh"),
            ("ship", "gfx/models/units/ships/generic_battleship.mesh"),
            ("plane", "gfx/models/units/planes/ENG_plane_medium.mesh"),
        ];
        println!(
            "gate_b: {:10} {:44} {:>6} {:>6} {:>8} {:>9} {:>18} shader",
            "kind", "rel", "verts", "tris", "maxidx", "aabb_dev", "uv_range"
        );
        for (kind, rel) in picks {
            let model = pdx_model_from_resolver(&resolver, rel).expect(rel);
            let rows = validate_pdx_model(&model);
            assert!(!rows.is_empty(), "{rel}: no meshes");
            for row in &rows {
                assert!(
                    row.indices_in_bounds,
                    "{rel} shape {} mesh {}: max index {} vs {} verts",
                    row.shape, row.mesh_index, row.max_index, row.vert_count
                );
                assert!(
                    row.aabb_contains_positions,
                    "{rel} shape {} mesh {}: aabb deviation {}",
                    row.shape, row.mesh_index, row.max_aabb_deviation
                );
                if let (Some(uv_min), Some(uv_max)) = (row.uv_min, row.uv_max) {
                    assert!(
                        uv_min >= -64.0 && uv_max <= 64.0,
                        "{rel}: implausible uv range [{uv_min}, {uv_max}]"
                    );
                }
                println!(
                    "gate_b: {:10} {:44} {:>6} {:>6} {:>8} {:>9.6} {:>18} {}",
                    kind,
                    rel,
                    row.vert_count,
                    row.triangle_count,
                    row.max_index,
                    row.max_aabb_deviation,
                    match (row.uv_min, row.uv_max) {
                        (Some(lo), Some(hi)) => format!("[{lo:.3}, {hi:.3}]"),
                        _ => "-".to_string(),
                    },
                    row.material_shader
                );
            }
            if matches!(kind, "unit") {
                assert!(
                    model.shapes.iter().any(|shape| !shape.skeleton.is_empty()),
                    "{rel}: expected a skeleton on the infantry mesh"
                );
                assert!(
                    model
                        .shapes
                        .iter()
                        .flat_map(|shape| &shape.meshes)
                        .any(|mesh| mesh.skin.is_some()),
                    "{rel}: expected skinning on the infantry mesh"
                );
            }
        }
    }
}


#[cfg(test)]
mod empty_normal_row_tests {
    use super::*;
    use super::test_fixtures::{push_object, push_prop_f, push_prop_i, push_prop_s};

    fn binary_row(p: Option<&[f32]>, tri: Option<&[i32]>, attributes: &[(&str, &[f32])]) -> Vec<u8> {
        let mut bytes = test_fixtures::collision_before_drawable_bytes();
        let marker = b"[[[mesh\0";
        let start = bytes.windows(marker.len()).rposition(|window| window == marker).unwrap();
        bytes.truncate(start);
        push_object(&mut bytes, 3, "mesh");
        if let Some(p) = p { push_prop_f(&mut bytes, "p", p); }
        if let Some(tri) = tri { push_prop_i(&mut bytes, "tri", tri); }
        for (name, values) in attributes { push_prop_f(&mut bytes, name, values); }
        push_prop_f(&mut bytes, "boundingsphere", &[7.0, 8.0, 9.0, 10.0]);
        push_object(&mut bytes, 4, "aabb");
        push_prop_f(&mut bytes, "min", &[-7.0, -8.0, -9.0]);
        push_prop_f(&mut bytes, "max", &[17.0, 18.0, 19.0]);
        push_object(&mut bytes, 4, "material");
        push_prop_s(&mut bytes, "shader", "PdxMeshStandard");
        push_prop_s(&mut bytes, "diff", "authored_empty_diffuse.dds");
        push_prop_s(&mut bytes, "n", "authored_empty_normal.dds");
        push_prop_s(&mut bytes, "spec", "authored_empty_specular.dds");
        bytes
    }

    #[test]
    fn present_empty_binary_streams_retain_normal_row_material_bounds_and_original_ordinal() {
        let bytes = binary_row(Some(&[]), Some(&[]), &[("n", &[]), ("ta", &[]), ("u0", &[]), ("u1", &[])]);
        let tree = parse_pdx_asset(&bytes).unwrap();
        let collision = tree.native_collision(1.0).unwrap().unwrap();
        let original = parse_pdx_asset(&test_fixtures::collision_before_drawable_bytes()).unwrap();
        assert_eq!(collision, original.native_collision(1.0).unwrap().unwrap());
        let model = extract_pdx_model(&tree).unwrap();
        assert_eq!(model.shapes.len(), 1);
        assert_eq!(model.shapes[0].meshes.len(), 1);
        let mesh = &model.shapes[0].meshes[0];
        assert_eq!(mesh.source_mesh_ordinal, 2);
        assert!(mesh.verts.is_empty() && mesh.indices.is_empty());
        assert_eq!(mesh.normals.as_deref(), Some(&[][..]));
        assert_eq!(mesh.tangents.as_deref(), Some(&[][..]));
        assert_eq!(mesh.uv0.as_deref(), Some(&[][..]));
        assert_eq!(mesh.uv1.as_deref(), Some(&[][..]));
        assert_eq!(mesh.aabb_min, [-7.0, -8.0, -9.0]);
        assert_eq!(mesh.aabb_max, [17.0, 18.0, 19.0]);
        assert_eq!(mesh.bounding_sphere, Some([7.0, 8.0, 9.0, 10.0]));
        assert_eq!(mesh.material.shader, "PdxMeshStandard");
        assert_eq!(mesh.material.diffuse.as_deref(), Some("authored_empty_diffuse.dds"));
        assert_eq!(mesh.material.normal_map.as_deref(), Some("authored_empty_normal.dds"));
        assert_eq!(mesh.material.specular.as_deref(), Some("authored_empty_specular.dds"));
    }

    #[test]
    fn absent_optional_attributes_remain_absent_on_an_empty_normal_row() {
        let tree = parse_pdx_asset(&binary_row(Some(&[]), Some(&[]), &[])).unwrap();
        let model = extract_pdx_model(&tree).unwrap();
        let mesh = &model.shapes[0].meshes[0];
        assert!(mesh.normals.is_none() && mesh.tangents.is_none() && mesh.uv0.is_none() && mesh.uv1.is_none());
        assert_eq!(mesh.source_mesh_ordinal, 2);
        assert!(mesh.verts.is_empty() && mesh.indices.is_empty());
    }

    #[test]
    fn missing_or_misaligned_positions_are_not_empty_streams() {
        for (p, expected) in [(None, "missing p (positions)"), (Some(&[0.0, 1.0][..]), "p has 2 floats, not a multiple of 3")] {
            let tree = parse_pdx_asset(&binary_row(p, Some(&[]), &[])).unwrap();
            assert!(extract_pdx_model(&tree).unwrap_err().to_string().contains(expected));
            assert!(tree.native_collision(1.0).unwrap().is_some());
        }
    }

    #[test]
    fn zero_vertices_preserve_missing_triangle_tuple_and_index_refusals() {
        for (tri, expected) in [
            (None, "missing tri (indices)"),
            (Some(&[0][..]), "tri has 1 indices, not a multiple of 3"),
            (Some(&[0, 0][..]), "tri has 2 indices, not a multiple of 3"),
            (Some(&[0, 0, 0][..]), "triangle index 0 out of bounds for 0 vertices"),
            (Some(&[-1, 0, 0][..]), "triangle index -1 out of bounds for 0 vertices"),
        ] {
            let tree = parse_pdx_asset(&binary_row(Some(&[]), tri, &[])).unwrap();
            assert!(extract_pdx_model(&tree).unwrap_err().to_string().contains(expected));
        }
    }

    #[test]
    fn nonempty_per_vertex_attributes_refuse_on_zero_vertices() {
        for (name, count) in [("n", 3), ("ta", 4), ("u0", 2), ("u1", 2)] {
            let values = vec![1.0; count];
            let tree = parse_pdx_asset(&binary_row(Some(&[]), Some(&[]), &[(name, &values)])).unwrap();
            let error = extract_pdx_model(&tree).unwrap_err().to_string();
            assert!(error.contains(&format!("{name} has {count} floats")), "{error}");
            assert!(error.contains(&format!("wanted {count} per vertex x 0")), "{error}");
        }
    }
}

#[cfg(test)]
mod native_aabb_tests {
    use super::*;

    fn mesh(bounds: Option<(&[f32], &[f32])>) -> PdxNode {
        let bytes = test_fixtures::collision_before_drawable_aabb_bytes(bounds);
        let tree = parse_pdx_asset(&bytes).unwrap();
        tree.root.children[0].children[0].children[2].clone()
    }

    #[test]
    fn absent_whole_node_bypasses_negative_zero_infinite_and_nan_scale() {
        let row = mesh(None);
        assert_eq!(row.native_aabb(), Ok(NativePdxAabb::Absent));
        for scale in [-2.0, -0.0, 0.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let (min, max) = row.native_aabb().unwrap().scaled(scale);
            assert_eq!(min.map(f32::to_bits), [0; 3]);
            assert_eq!(max.map(f32::to_bits), [0; 3]);
        }
    }

    #[test]
    fn first_aabb_and_first_coordinates_borrow_same_raw_storage() {
        let mut row = mesh(Some((&[1.0, 2.0, 3.0, 99.0], &[4.0, 5.0, 6.0, 88.0])));
        let mut later = row.children.iter().find(|n| n.name == "aabb").unwrap().clone();
        later.properties[0].values = PdxValues::F32(vec![70.0; 3]);
        row.children.push(later);
        let first = row.children.iter_mut().find(|n| n.name == "aabb").unwrap();
        first.properties.push(PdxProperty { name: "min".into(), values: PdxValues::F32(vec![90.0; 3]) });
        let NativePdxAabb::Present { min, max } = row.native_aabb().unwrap() else { panic!("present"); };
        assert_eq!(*min, [1.0, 2.0, 3.0]);
        assert_eq!(*max, [4.0, 5.0, 6.0]);
        let first = row.children.iter().find(|n| n.name == "aabb").unwrap();
        let PdxValues::F32(raw) = &first.property("min").unwrap().values else { panic!("floats"); };
        assert_eq!(min.as_ptr(), raw.as_ptr());
        assert_eq!(row.native_aabb().unwrap().scaled(-2.0), ([-2.0, -4.0, -6.0], [-8.0, -10.0, -12.0]));
    }

    #[test]
    fn present_coordinates_multiply_signed_zero_and_nonfinite_values() {
        let row = mesh(Some((&[-0.0, 0.0, -2.0], &[0.0, -0.0, f32::INFINITY])));
        let (min, max) = row.native_aabb().unwrap().scaled(-2.0);
        assert_eq!(min.map(f32::to_bits), [0, 0x8000_0000, 4.0f32.to_bits()]);
        assert_eq!(max.map(f32::to_bits), [0x8000_0000, 0, f32::NEG_INFINITY.to_bits()]);
        let (min, max) = row.native_aabb().unwrap().scaled(f32::NAN);
        assert!(min.into_iter().chain(max).all(f32::is_nan));
    }

    #[test]
    fn malformed_first_node_or_coordinate_is_not_repaired_by_later_duplicates() {
        let valid = mesh(Some((&[1.0; 3], &[2.0; 3])));
        let valid_aabb = valid.children.iter().find(|n| n.name == "aabb").unwrap();
        for property in ["min", "max"] {
            let mut row = valid.clone();
            let first = row.children.iter_mut().find(|n| n.name == "aabb").unwrap();
            first.properties.retain(|p| p.name != property);
            row.children.push(valid_aabb.clone());
            assert_eq!(row.native_aabb(), Err(NativePdxAabbRefusal::MissingCoordinate { property }));
            for count in 0..3 {
                let mut row = valid.clone();
                let first = row.children.iter_mut().find(|n| n.name == "aabb").unwrap();
                let index = first.properties.iter().position(|p| p.name == property).unwrap();
                first.properties.push(first.properties[index].clone());
                first.properties[index].values = PdxValues::F32(vec![0.0; count]);
                assert_eq!(row.native_aabb(), Err(NativePdxAabbRefusal::CoordinateUnderflow { property, count }));
            }
        }
    }

    #[test]
    fn unsupported_payloads_stay_named_independently_of_drawable_streams() {
        for property in ["min", "max"] {
            for integer in [false, true] {
                let mut row = mesh(Some((&[1.0; 3], &[2.0; 3])));
                let aabb = row.children.iter_mut().find(|n| n.name == "aabb").unwrap();
                aabb.properties.iter_mut().find(|p| p.name == property).unwrap().values = if integer {
                    PdxValues::I32(vec![0x3f80_0000; 3])
                } else { PdxValues::Str(vec!["1".into(); 3]) };
                assert_eq!(row.native_aabb(), Err(NativePdxAabbRefusal::CoordinateType { property }));
            }
        }
        let mut row = mesh(Some((&[1.0; 3], &[2.0; 3])));
        row.properties.clear();
        row.children.retain(|n| n.name == "aabb");
        assert_eq!(row.native_aabb().unwrap().scaled(1.0), ([1.0; 3], [2.0; 3]));
    }
}

#[cfg(test)]
mod native_collision_tests {
    use super::test_fixtures::{append_collision, push_object, push_prop_f, push_prop_i, push_prop_s};
    use super::*;

    fn header() -> Vec<u8> {
        let mut bytes = PDX_ASSET_BINARY_MAGIC.to_vec();
        push_prop_i(&mut bytes, "pdxasset", &[1, 0]);
        bytes
    }

    const TRIANGLE: &[f32] = &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    #[test]
    fn last_nonempty_collision_preserves_source_ordinals_and_ignores_lod() {
        let mut bytes = header();
        push_object(&mut bytes, 1, "locator");
        append_collision(&mut bytes, TRIANGLE, &[0, 1, 2]);
        append_collision(&mut bytes, &[2.0; 9], &[2, 1, 0]);
        append_collision(&mut bytes, &[], &[0, 1, 2]);
        append_collision(&mut bytes, TRIANGLE, &[]);
        let tree = parse_pdx_asset(&bytes).unwrap();
        let hit = tree.native_collision(2.0).unwrap().unwrap();
        assert_eq!(
            hit.source,
            NativePdxCollisionSource {
                root_child: 2,
                shape_child: 0,
                mesh_child: 0
            }
        );
        assert_eq!(hit.scaled_positions, [[4.0; 3]; 3]);
        assert_eq!(hit.indices, [2, 1, 0]);
        let mut tree = tree;
        tree.root.children[2].children[0]
            .properties
            .push(PdxProperty {
                name: "lod".into(),
                values: PdxValues::I32(vec![999]),
            });
        assert_eq!(tree.native_collision(2.0).unwrap(), Some(hit));
        let mut bytes = header();
        push_object(&mut bytes, 1, "object");
        push_object(&mut bytes, 2, "ignoredShape");
        push_object(&mut bytes, 2, "actualShape");
        push_object(&mut bytes, 3, "notMesh");
        push_object(&mut bytes, 3, "mesh");
        push_prop_f(&mut bytes, "p", TRIANGLE);
        push_prop_i(&mut bytes, "tri", &[2, 1, 0]);
        push_object(&mut bytes, 4, "material");
        push_prop_s(&mut bytes, "shader", "Collision");
        assert_eq!(
            parse_pdx_asset(&bytes)
                .unwrap()
                .native_collision(1.0)
                .unwrap()
                .unwrap()
                .source,
            NativePdxCollisionSource {
                root_child: 0,
                shape_child: 1,
                mesh_child: 1
            }
        );
    }

    #[test]
    fn drawable_extraction_skips_collision_streams_and_keeps_the_original_child_slot() {
        let bytes = super::test_fixtures::collision_before_drawable_bytes();
        let tree = parse_pdx_asset(&bytes).unwrap();
        let collision = tree.native_collision(1.0).unwrap().unwrap();
        assert_eq!(collision.source.mesh_child, 1);
        assert_eq!(collision.indices, [0, 1, 2]);
        let model = extract_pdx_model(&tree).unwrap();
        let shape = &model.shapes[0];
        assert_eq!(shape.meshes.len(), 1);
        let mesh = shape.mesh_by_source_ordinal(2).unwrap();
        assert_eq!(mesh.source_mesh_ordinal, 2);
        assert_eq!(mesh.aabb_min, [4.0, 0.0, 0.0]);
        assert_eq!(mesh.aabb_max, [5.0, 1.0, 0.0]);
        assert_eq!(mesh.verts.len(), 3);
        assert!(shape.mesh_by_source_ordinal(0).is_none());
        assert!(shape.mesh_by_source_ordinal(1).is_none());
        assert_eq!(validate_pdx_model(&model)[0].mesh_index, 2);
        assert!(pdx_model_to_obj(&model, "fixture").contains("g mixedShape_mesh2_PdxMeshStandard"));
        assert_eq!(tree.native_collision(1.0).unwrap(), Some(collision));
    }

    #[test]
    fn only_the_first_exact_collision_shader_bypasses_drawable_validation() {
        let bytes = super::test_fixtures::collision_before_drawable_bytes();
        let mut tree = parse_pdx_asset(&bytes).unwrap();
        let collision = &mut tree.root.children[0].children[0].children[1];
        collision.children[0].properties[0].values = PdxValues::Str(vec!["Collision".into(), "other".into()]);
        assert!(extract_pdx_model(&tree).is_ok());
        let collision = &mut tree.root.children[0].children[0].children[1];
        let exact = collision.children[0].clone();
        collision.children[0].properties[0].values = PdxValues::Str(vec!["collision".into()]);
        collision.children.push(exact);
        assert!(extract_pdx_model(&tree).is_err());
        assert_eq!(tree.native_collision(1.0), Ok(None));
    }

    #[test]
    fn raw_i32_indices_are_narrowed_before_bounds_validation() {
        let mut bytes = header();
        append_collision(&mut bytes, TRIANGLE, &[-65536, 65537, i32::MIN + 2]);
        let tree = parse_pdx_asset(&bytes).unwrap();
        assert!(extract_pdx_model(&tree).unwrap().shapes[0].meshes.is_empty());
        assert_eq!(
            tree.native_collision(1.0).unwrap().unwrap().indices,
            [0, 1, 2]
        );
        let mut bytes = header();
        append_collision(&mut bytes, TRIANGLE, &[-1, 0, 1]);
        assert!(matches!(
            parse_pdx_asset(&bytes).unwrap().native_collision(1.0),
            Err(NativePdxCollisionRefusal::IndexOutOfBounds {
                ordinal: 0,
                index: 65535,
                vertex_count: 3,
                ..
            })
        ));
    }

    #[test]
    fn witnessed_no_qualifying_collision_is_distinct_from_malformed_geometry() {
        let mut bytes = header();
        append_collision(&mut bytes, &[], &[0, 1, 2]);
        append_collision(&mut bytes, &[1.0, 2.0], &[0, 1, 2]);
        append_collision(&mut bytes, TRIANGLE, &[]);
        assert_eq!(
            parse_pdx_asset(&bytes).unwrap().native_collision(1.0),
            Ok(None)
        );
        append_collision(&mut bytes, &[1.0; 4], &[0, 0, 0]);
        assert!(matches!(
            parse_pdx_asset(&bytes).unwrap().native_collision(1.0),
            Err(NativePdxCollisionRefusal::PositionTriple { float_count: 4, .. })
        ));
    }

    #[test]
    fn lookup_uses_first_material_first_property_and_first_shader_string() {
        let mut bytes = header();
        append_collision(&mut bytes, TRIANGLE, &[0, 1, 2]);
        let mut tree = parse_pdx_asset(&bytes).unwrap();
        let mesh = &mut tree.root.children[0].children[0].children[0];
        mesh.properties.push(PdxProperty {
            name: "tri".into(),
            values: PdxValues::I32(vec![-1]),
        });
        mesh.children[0].properties[0].values =
            PdxValues::Str(vec!["Collision".into(), "notCollision".into()]);
        mesh.children[0].properties.push(PdxProperty {
            name: "shader".into(),
            values: PdxValues::Str(vec!["notCollision".into()]),
        });
        assert_eq!(
            tree.native_collision(1.0).unwrap().unwrap().indices,
            [0, 1, 2]
        );
        let mesh = &mut tree.root.children[0].children[0].children[0];
        let correct = mesh.children[0].clone();
        mesh.children[0].properties[0].values = PdxValues::Str(vec!["collision".into()]);
        mesh.children.push(correct);
        assert_eq!(tree.native_collision(1.0), Ok(None));
    }

    #[test]
    fn only_final_indices_are_consumed_but_unsafe_position_copy_is_never_repaired() {
        let mut bytes = header();
        append_collision(&mut bytes, TRIANGLE, &[-1]);
        append_collision(&mut bytes, TRIANGLE, &[0, 1, 2]);
        assert!(parse_pdx_asset(&bytes)
            .unwrap()
            .native_collision(1.0)
            .unwrap()
            .is_some());
        append_collision(&mut bytes, TRIANGLE, &[0]);
        assert!(matches!(
            parse_pdx_asset(&bytes).unwrap().native_collision(1.0),
            Err(NativePdxCollisionRefusal::IndexTriple { index_count: 1, .. })
        ));
        let mut bytes = header();
        append_collision(&mut bytes, &[1.0; 4], &[0, 0, 0]);
        append_collision(&mut bytes, TRIANGLE, &[0, 1, 2]);
        assert!(matches!(
            parse_pdx_asset(&bytes).unwrap().native_collision(1.0),
            Err(NativePdxCollisionRefusal::PositionTriple { .. })
        ));
    }

    #[test]
    fn wrong_payload_types_refuse_only_after_native_nonempty_qualification() {
        let mut bytes = header();
        append_collision(&mut bytes, TRIANGLE, &[0, 1, 2]);
        let mut tree = parse_pdx_asset(&bytes).unwrap();
        tree.root.children[0].children[0].children[0].properties[0].values =
            PdxValues::I32(vec![0; 9]);
        assert!(matches!(
            tree.native_collision(1.0),
            Err(NativePdxCollisionRefusal::PropertyType { property: "p", .. })
        ));
        tree.root.children[0].children[0].children[0].properties[0].values = PdxValues::I32(vec![]);
        assert_eq!(tree.native_collision(1.0), Ok(None));
    }

    #[test]
    fn authored_scale_preserves_native_float_multiply_special_values() {
        let mut bytes = header();
        append_collision(&mut bytes, &[0.0, -0.0, f32::INFINITY], &[0, 0, 0]);
        let tree = parse_pdx_asset(&bytes).unwrap();
        let hit = tree.native_collision(-2.0).unwrap().unwrap();
        assert_eq!(
            hit.scaled_positions[0].map(f32::to_bits),
            [0x80000000, 0, 0xff800000]
        );
        let hit = tree.native_collision(0.0).unwrap().unwrap();
        assert!(hit.scaled_positions[0][2].is_nan());
        assert_eq!(hit.scaled_positions[0][1].to_bits(), 0x80000000);
    }
}
