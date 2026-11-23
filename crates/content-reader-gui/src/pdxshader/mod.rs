
use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use content_reader_layers::ResolvedLayers;
use crate::gui::native::reader::{Reader, keyword_of, read_string};
use content_reader_script::native::{self, Kind as TokenKind};
use content_reader_parser::parser::read_bool;

pub mod program;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Wrap,
    Mirror,
    Clamp,
    Border,
    MirrorOnce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    None,
    Point,
    Linear,
    Anisotropic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SamplerRow {
    pub name: Vec<u8>,
    pub kind: u32,
    pub index: u32,
    pub lod_bias: f32,
    pub min_lod: i32,
    pub max_lod: i32,
    pub max_anisotropy: i32,
    pub mag: Filter,
    pub min: Filter,
    pub mip: Filter,
    pub address_u: Address,
    pub address_v: Address,
}

impl SamplerRow {
    fn new(name: Vec<u8>) -> Self {
        Self {
            name,
            kind: 0,
            index: 0,
            lod_bias: 0.0,
            min_lod: 0,
            max_lod: 1024,
            max_anisotropy: -1,
            mag: Filter::Linear,
            min: Filter::Linear,
            mip: Filter::Linear,
            address_u: Address::Wrap,
            address_v: Address::Wrap,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConstantBuffer {
    pub arguments: [u32; 2],
    pub members: Vec<(Vec<u8>, Vec<u8>, i32)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub directive: Vec<u8>,
    pub kind: Vec<u8>,
    pub name: Vec<u8>,
    pub semantic: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VertexStruct {
    pub name: Vec<u8>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stage {
    pub samplers: Vec<SamplerRow>,
    pub constant_buffers: Vec<ConstantBuffer>,
    pub vertex_structs: Vec<VertexStruct>,
    pub main_code: Vec<(Vec<u8>, Vec<u8>)>,
    pub code: Vec<u8>,
}

impl Stage {
    fn append(&mut self, other: &Stage) {
        self.samplers.extend(other.samplers.iter().cloned());
        self.constant_buffers
            .extend(other.constant_buffers.iter().cloned());
        self.vertex_structs
            .extend(other.vertex_structs.iter().cloned());
        self.main_code.extend(other.main_code.iter().cloned());
        self.code.extend_from_slice(&other.code);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Technique {
    pub name: Vec<u8>,
    pub vertex_shader: Vec<u8>,
    pub pixel_shader: Vec<u8>,
    pub blend_state: Vec<u8>,
    pub depth_stencil_state: Vec<u8>,
    pub rasterizer_state: Vec<u8>,
    pub defines: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlendState {
    pub enable: bool,
    pub alpha_test: bool,
    pub source: u32,
    pub dest: u32,
    pub op: u32,
    pub source_alpha: u32,
    pub dest_alpha: u32,
    pub op_alpha: u32,
    pub write_mask: u32,
}

impl Default for BlendState {
    fn default() -> Self {
        Self {
            enable: false,
            alpha_test: false,
            source: 1,
            dest: 0,
            op: 0,
            source_alpha: 1,
            dest_alpha: 0,
            op_alpha: 0,
            write_mask: 0xF,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepthStencilState {
    pub depth_enable: bool,
    pub depth_write_mask: u32,
    pub depth_function: u32,
    pub stencil_enable: bool,
    pub stencil_read_mask: u32,
    pub stencil_write_mask: u32,
    pub front_fail: u32,
    pub front_depth_fail: u32,
    pub front_pass: u32,
    pub front_function: u32,
    pub stencil_ref: u32,
}

impl Default for DepthStencilState {
    fn default() -> Self {
        Self {
            depth_enable: true,
            depth_write_mask: 1,
            depth_function: 3,
            stencil_enable: false,
            stencil_read_mask: u32::MAX,
            stencil_write_mask: u32::MAX,
            front_fail: 0,
            front_depth_fail: 0,
            front_pass: 0,
            front_function: 7,
            stencil_ref: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RasterizerState {
    pub fill_mode: u32,
    pub cull_mode: u32,
    pub front_ccw: bool,
    pub depth_bias: i32,
    pub depth_bias_clamp: f32,
    pub slope_scaled_depth_bias: f32,
    pub scissor: bool,
    pub multisample: bool,
}

impl Default for RasterizerState {
    fn default() -> Self {
        Self {
            fill_mode: 2,
            cull_mode: 2,
            front_ccw: false,
            depth_bias: 0,
            depth_bias_clamp: 0.0,
            slope_scaled_depth_bias: 0.0,
            scissor: false,
            multisample: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct File {
    includes: Vec<Vec<u8>>,
    stages: [Stage; 3],
    techniques: Vec<Technique>,
    blend_states: Vec<(Vec<u8>, BlendState)>,
    depth_stencil_states: Vec<(Vec<u8>, DepthStencilState)>,
    rasterizer_states: Vec<(Vec<u8>, RasterizerState)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub path: String,
    pub identity: [u8; 32],
    pub includes: Vec<(String, Option<[u8; 32]>)>,
    pub stages: [Stage; 3],
    pub techniques: Vec<Technique>,
    pub blend_states: Vec<(Vec<u8>, BlendState)>,
    pub depth_stencil_states: Vec<(Vec<u8>, DepthStencilState)>,
    pub rasterizer_states: Vec<(Vec<u8>, RasterizerState)>,
}

impl Effect {
    pub fn technique(&self, name: &[u8]) -> Option<&Technique> {
        self.techniques
            .iter()
            .find(|technique| technique.name.eq_ignore_ascii_case(name))
    }

    pub fn blend(&self, technique: &Technique) -> BlendState {
        self.blend_states
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&technique.blend_state))
            .map(|(_, state)| state.clone())
            .unwrap_or_default()
    }

    pub fn sampler_slots(&self) -> Result<Vec<Option<&SamplerRow>>, String> {
        let mut slots: Vec<Option<&SamplerRow>> = Vec::new();
        for stage in [&self.stages[0], &self.stages[2]] {
            for row in &stage.samplers {
                if row.index >= 0x7FFF_FFFF {
                    return Err(format!(
                        "{} declares sampler {:?} at index {}, which writes \
                         out of bounds",
                        self.path,
                        String::from_utf8_lossy(&row.name),
                        row.index
                    ));
                }
                let index = row.index as usize;
                if slots.len() <= index {
                    slots.resize(index + 1, None);
                }
                slots[index] = Some(row);
            }
        }
        Ok(slots)
    }
}

pub fn effect_path(effect: &[u8]) -> Vec<u8> {
    let effect = content_reader_parser::parser::c_string(effect);
    match effect.windows(4).position(|window| window == b".lua") {
        Some(at) => [&effect[..at], b".shader", &effect[at + 4..]].concat(),
        None => effect.to_vec(),
    }
}

pub fn load(layers: &ResolvedLayers, effect: &[u8]) -> Option<Effect> {
    load_with(effect, &|path| layers.read_uncovered_path(path).ok())
}

pub fn load_with<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_layers::content::Bytes>>(
    effect: &[u8],
    read: &dyn Fn(&[u8]) -> Option<B>,
) -> Option<Effect> {
    let file = effect_path(effect);
    let path = String::from_utf8_lossy(&file).into_owned();
    let bytes: content_reader_layers::content::Bytes = read(&file)?.into();
    let root = read_file(&bytes).ok()?;
    let mut includes = Vec::new();
    let mut files = Vec::new();
    let mut visited = BTreeSet::new();
    type Open = (Vec<u8>, usize, Option<(File, String, Option<[u8; 32]>)>);
    for include in &root.includes {
        let mut stack: Vec<Open> = vec![(include.clone(), 0, None)];
        while let Some((name, next, opened)) = stack.pop() {
            let (file, path, identity) = match opened {
                Some(opened) => opened,
                None => {
                    if !visited.insert(name.clone()) {
                        continue;
                    }
                    let file = [b"gfx/FX/".as_slice(), &name].concat();
                    let path = String::from_utf8_lossy(&file).into_owned();
                    let bytes: Option<content_reader_layers::content::Bytes> = read(&file).map(Into::into);
                    let identity = bytes.as_deref().map(sha256);
                    let file = match bytes {
                        Some(bytes) => read_file(&bytes).ok()?,
                        None => File::default(),
                    };
                    (file, path, identity)
                }
            };
            if let Some(child) = file.includes.get(next).cloned() {
                stack.push((name, next + 1, Some((file, path, identity))));
                stack.push((child, 0, None));
            } else {
                includes.push((path, identity));
                files.push(file);
            }
        }
    }
    let mut stages: [Stage; 3] = Default::default();
    for file in files.iter().chain(std::iter::once(&root)) {
        for (merged, own) in stages.iter_mut().zip(&file.stages) {
            merged.append(own);
        }
    }
    Some(Effect {
        path,
        identity: sha256(&bytes),
        includes,
        stages,
        techniques: root.techniques,
        blend_states: root.blend_states,
        depth_stencil_states: root.depth_stencil_states,
        rasterizer_states: root.rasterizer_states,
    })
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn read_file(bytes: impl Into<content_reader_layers::content::Bytes>) -> std::io::Result<File> {
    let bytes = bytes.into();
    content_reader_layers::content::Generation::derive(&bytes, "shader file grammar", 1, &[], || {
        content_reader_parser::parse_counts::parse("shader file grammar", &bytes);
        Ok(parse_file(&bytes))
    })
}

fn parse_file(bytes: &[u8]) -> File {
    let mut reader = Reader::new(native::lex_effect(bytes, native::Mode::KeepMark), 0);
    let mut file = File::default();
    while reader.next_key() {
        let word = keyword_of(&reader.key.clone()).unwrap_or_default();
        match word {
            "includes" => file.includes.extend(reader.string_list()),
            "samplers" | "vertexstruct" | "constantbuffer" | "maincode" | "code" => {
                reader.stage_key(word, &mut file.stages[0]);
            }
            "vertexshader" | "pixelshader" => {
                let index = if word == "vertexshader" { 1 } else { 2 };
                while reader.next_key() {
                    let inner = keyword_of(&reader.key.clone()).unwrap_or_default();
                    if matches!(
                        inner,
                        "samplers" | "vertexstruct" | "constantbuffer" | "maincode" | "code"
                    ) {
                        reader.stage_key(inner, &mut file.stages[index]);
                    } else {
                        reader.unexpected();
                    }
                }
            }
            "effect" => {
                let name = reader.name();
                file.techniques.push(reader.technique(name));
            }
            "blendstate" => {
                let name = reader.name();
                let state = reader.blend_state();
                file.blend_states.push((name, state));
            }
            "depthstencilstate" => {
                let name = reader.name();
                let state = reader.depth_stencil_state();
                file.depth_stencil_states.push((name, state));
            }
            "rasterizerstate" => {
                let name = reader.name();
                let state = reader.rasterizer_state();
                file.rasterizer_states.push((name, state));
            }
            _ => reader.unexpected(),
        }
    }
    file
}

fn value_index(text: &[u8], names: &[&str]) -> Option<u32> {
    let text = content_reader_parser::parser::c_string(text);
    names
        .iter()
        .position(|name| text.eq_ignore_ascii_case(name.as_bytes()))
        .and_then(|index| u32::try_from(index).ok())
}

fn blend_factor(text: &[u8]) -> Option<u32> {
    match content_reader_parser::parser::c_string(text)
        .to_ascii_lowercase()
        .as_slice()
    {
        b"zero" => Some(0),
        b"one" => Some(1),
        b"src_color" => Some(2),
        b"inv_src_color" => Some(3),
        b"src_alpha" => Some(4),
        b"inv_src_alpha" => Some(5),
        b"dest_alpha" => Some(6),
        b"inv_dest_alpha" => Some(7),
        b"dest_color" => Some(8),
        b"inv_dest_color" => Some(9),
        b"src_alpha_sat" => Some(10),
        b"blend_factor" => Some(11),
        b"inv_blend_factor" => Some(12),
        b"src1_color" => Some(13),
        b"inv_src1_color" => Some(14),
        b"src1_alpha" => Some(15),
        b"inv_src1_alpha" => Some(16),
        _ => None,
    }
}

fn blend_operation(text: &[u8]) -> Option<u32> {
    match content_reader_parser::parser::c_string(text)
        .to_ascii_lowercase()
        .as_slice()
    {
        b"blend_op_add" => Some(0),
        b"blend_op_subtract" => Some(1),
        b"blend_op_rev_subtract" => Some(2),
        b"blend_op_min" => Some(3),
        b"blend_op_max" => Some(4),
        _ => None,
    }
}

fn depth_comparison(text: &[u8]) -> Option<u32> {
    match content_reader_parser::parser::c_string(text)
        .to_ascii_lowercase()
        .as_slice()
    {
        b"comparison_never" => Some(0),
        b"comparison_less" => Some(1),
        b"comparison_equal" => Some(2),
        b"comparison_less_equal" => Some(3),
        b"comparison_greater" => Some(4),
        b"comparison_not_equal" => Some(5),
        b"comparison_greater_equal" => Some(6),
        b"comparison_always" => Some(7),
        _ => None,
    }
}

fn stencil_operation(text: &[u8]) -> Option<u32> {
    match content_reader_parser::parser::c_string(text)
        .to_ascii_lowercase()
        .as_slice()
    {
        b"stencil_op_keep" => Some(0),
        b"stencil_op_zero" => Some(1),
        b"stencil_op_replace" => Some(2),
        b"stencil_op_incr_sat" => Some(3),
        b"stencil_op_decr_sat" => Some(4),
        b"stencil_op_invert" => Some(5),
        b"stencil_op_incr" => Some(6),
        b"stencil_op_decr" => Some(7),
        _ => None,
    }
}

fn depth_write_mask(text: &[u8]) -> Option<u32> {
    match content_reader_parser::parser::c_string(text)
        .to_ascii_lowercase()
        .as_slice()
    {
        b"depth_write_zero" => Some(0),
        b"depth_write_all" => Some(1),
        _ => None,
    }
}

impl Reader<'_> {
    fn name(&mut self) -> Vec<u8> {
        let name = self.current.c_str().to_vec();
        self.read_resolved();
        name
    }

    fn stage_key(&mut self, word: &str, stage: &mut Stage) {
        match word {
            "samplers" => {
                while self.next_key() {
                    let mut row = SamplerRow::new(self.key.c_str().to_vec());
                    while self.next_key() {
                        let key = keyword_of(&self.key.clone()).unwrap_or_default();
                        let text = self.value.text.clone();
                        match key {
                            "index" => {
                                row.index =
                                    content_reader_parser::crt::scan_u32(&text).stored().unwrap_or(row.index)
                            }
                            "type" => {
                                if let Some(kind) = value_index(&text, &["2d", "cube", "3d"]) {
                                    row.kind = kind;
                                } else if value_index(&text, &["shadow"]).is_some() {
                                    row.kind = 0;
                                }
                            }
                            "magfilter" | "minfilter" | "mipfilter" => {
                                let filter = match value_index(
                                    &text,
                                    &["none", "point", "linear", "anisotropic"],
                                ) {
                                    Some(0) => Some(Filter::None),
                                    Some(1) => Some(Filter::Point),
                                    Some(2) => Some(Filter::Linear),
                                    Some(3) => Some(Filter::Anisotropic),
                                    _ => None,
                                };
                                if let Some(filter) = filter {
                                    match key {
                                        "magfilter" => row.mag = filter,
                                        "minfilter" => row.min = filter,
                                        _ => row.mip = filter,
                                    }
                                }
                            }
                            "addressu" | "addressv" => {
                                let address = match value_index(
                                    &text,
                                    &["wrap", "mirror", "clamp", "border", "mirror_once"],
                                ) {
                                    Some(0) => Some(Address::Wrap),
                                    Some(1) => Some(Address::Mirror),
                                    Some(2) => Some(Address::Clamp),
                                    Some(3) => Some(Address::Border),
                                    Some(4) => Some(Address::MirrorOnce),
                                    _ => None,
                                };
                                if let Some(address) = address {
                                    if key == "addressu" {
                                        row.address_u = address;
                                    } else {
                                        row.address_v = address;
                                    }
                                }
                            }
                            "mipmaplodbias" => {
                                row.lod_bias = content_reader_parser::crt::scan_f32(&text).unwrap_or(row.lod_bias);
                            }
                            "mipmapmaxlod" => {
                                row.max_lod =
                                    content_reader_parser::crt::scan_i32(&text).stored().unwrap_or(row.max_lod)
                            }
                            "mipmapminlod" => {
                                row.min_lod =
                                    content_reader_parser::crt::scan_i32(&text).stored().unwrap_or(row.min_lod)
                            }
                            "maxanisotropy" => {
                                row.max_anisotropy = content_reader_parser::crt::scan_i32(&text)
                                    .stored()
                                    .unwrap_or(row.max_anisotropy);
                            }
                            _ => self.unexpected(),
                        }
                    }
                    stage.samplers.push(row);
                }
            }
            "vertexstruct" => stage.vertex_structs.push(self.vertex_struct()),
            "constantbuffer" => stage.constant_buffers.push(self.constant_buffer()),
            "maincode" => {
                let name = self.name();
                let code = self.code();
                stage.main_code.push((name, code));
            }
            _ => {
                let code = self.code();
                stage.code.extend(code);
            }
        }
    }

    fn code(&mut self) -> Vec<u8> {
        if self.current.kind == TokenKind::Code {
            self.current.text.clone()
        } else {
            Vec::new()
        }
    }

    fn vertex_struct(&mut self) -> VertexStruct {
        let mut out = VertexStruct {
            name: self.name(),
            members: Vec::new(),
        };
        loop {
            if self.peek().ends_block() {
                self.read_resolved();
                return out;
            }
            let kind = self.read_resolved();
            let mut member = Member::default();
            let kind = content_reader_parser::parser::c_string(&kind.text).to_vec();
            if kind.starts_with(b"@") {
                if kind.starts_with(b"@ifdef") {
                    let name = self.read_resolved();
                    member.directive =
                        [b"#ifdef ".as_slice(), content_reader_parser::parser::c_string(&name.text)].concat();
                } else if kind.starts_with(b"@endif") {
                    member.directive = b"#endif".to_vec();
                } else if kind.starts_with(b"@else") {
                    member.directive = b"#else".to_vec();
                }
            } else {
                member.kind = kind;
                member.name = content_reader_parser::parser::c_string(&self.read_resolved().text).to_vec();
                self.read_resolved();
                member.semantic = content_reader_parser::parser::c_string(&self.read_resolved().text).to_vec();
            }
            out.members.push(member);
        }
    }

    fn constant_buffer(&mut self) -> ConstantBuffer {
        let mut out = ConstantBuffer::default();
        let first = self.read_resolved();
        out.arguments[0] = content_reader_parser::crt::scan_u32(&first.text).stored().unwrap_or(0);
        self.read_resolved();
        let second = self.read_resolved();
        out.arguments[1] = content_reader_parser::crt::scan_u32(&second.text).stored().unwrap_or(0);
        self.read_resolved();
        self.read_resolved();
        loop {
            if self.peek().ends_block() {
                self.read_resolved();
                return out;
            }
            let kind = self.read_resolved();
            let name = content_reader_parser::parser::c_string(&self.read_resolved().text).to_vec();
            let (name, count) = match name.iter().position(|byte| *byte == b'[') {
                Some(at) => (
                    name[..at].to_vec(),
                    content_reader_parser::crt::scan_i32(&name[at + 1..]).stored().unwrap_or(0),
                ),
                None => (name, 0),
            };
            out.members
                .push((content_reader_parser::parser::c_string(&kind.text).to_vec(), name, count));
        }
    }

    fn technique(&mut self, name: Vec<u8>) -> Technique {
        let mut technique = Technique {
            name,
            vertex_shader: Vec::new(),
            pixel_shader: Vec::new(),
            blend_state: b"BlendState".to_vec(),
            depth_stencil_state: b"DepthStencilState".to_vec(),
            rasterizer_state: b"RasterizerState".to_vec(),
            defines: Vec::new(),
        };
        while self.next_key() {
            let key = keyword_of(&self.key.clone()).unwrap_or_default();
            let text = self.value.text.clone();
            match key {
                "vertexshader" => technique.vertex_shader = read_string(&text),
                "pixelshader" => technique.pixel_shader = read_string(&text),
                "blendstate" => technique.blend_state = read_string(&text),
                "depthstencilstate" => technique.depth_stencil_state = read_string(&text),
                "rasterizerstate" => technique.rasterizer_state = read_string(&text),
                "defines" => technique.defines.extend(self.string_list()),
                _ => self.unexpected(),
            }
        }
        technique
    }

    fn blend_state(&mut self) -> BlendState {
        let mut state = BlendState::default();
        while self.next_key() {
            let key = keyword_of(&self.key.clone()).unwrap_or_default();
            let text = self.value.text.clone();
            let factor = |own: &mut u32| {
                if let Some(value) = blend_factor(&text) {
                    *own = value;
                }
            };
            let operation = |own: &mut u32| {
                if let Some(value) = blend_operation(&text) {
                    *own = value;
                }
            };
            match key {
                "blendenable" => state.enable = read_bool(&text).unwrap_or(state.enable),
                "alphatest" | "alphatocoverage" => {
                    state.alpha_test = read_bool(&text).unwrap_or(state.alpha_test);
                }
                "sourceblend" => factor(&mut state.source),
                "destblend" => factor(&mut state.dest),
                "blendop" => operation(&mut state.op),
                "sourcealpha" => factor(&mut state.source_alpha),
                "destalpha" => factor(&mut state.dest_alpha),
                "blendopalpha" => operation(&mut state.op_alpha),
                "writemask" => {
                    let mask = content_reader_parser::parser::c_string(&text).to_ascii_uppercase();
                    let has = |word: &[u8]| mask.windows(word.len()).any(|window| window == word);
                    state.write_mask = if has(b"ALL") {
                        0xF
                    } else {
                        [(&b"RED"[..], 1), (b"GREEN", 2), (b"BLUE", 4), (b"ALPHA", 8)]
                            .iter()
                            .filter(|(word, _)| has(word))
                            .fold(0, |mask, (_, bit)| mask | bit)
                    };
                }
                _ => self.unexpected(),
            }
        }
        state
    }

    fn depth_stencil_state(&mut self) -> DepthStencilState {
        let mut state = DepthStencilState::default();
        while self.next_key() {
            let key = keyword_of(&self.key.clone()).unwrap_or_default();
            let text = self.value.text.clone();
            let pick = |value: Option<u32>, own: &mut u32| {
                if let Some(value) = value {
                    *own = value;
                }
            };
            match key {
                "depthenable" => {
                    state.depth_enable = read_bool(&text).unwrap_or(state.depth_enable)
                }
                "depthwritemask" => pick(depth_write_mask(&text), &mut state.depth_write_mask),
                "depthfunction" => pick(depth_comparison(&text), &mut state.depth_function),
                "stencilenable" => {
                    state.stencil_enable = read_bool(&text).unwrap_or(state.stencil_enable);
                }
                "stencilreadmask" => {
                    state.stencil_read_mask = content_reader_parser::crt::scan_i32(&text)
                        .stored()
                        .map_or(state.stencil_read_mask, |value| value as u32);
                }
                "stencilwritemask" => {
                    state.stencil_write_mask = content_reader_parser::crt::scan_i32(&text)
                        .stored()
                        .map_or(state.stencil_write_mask, |value| value as u32);
                }
                "frontstencilfailop" => pick(stencil_operation(&text), &mut state.front_fail),
                "frontstencildepthfailop" => {
                    pick(stencil_operation(&text), &mut state.front_depth_fail)
                }
                "frontstencilpassop" => pick(stencil_operation(&text), &mut state.front_pass),
                "frontstencilfunc" => pick(depth_comparison(&text), &mut state.front_function),
                "stencilref" => {
                    state.stencil_ref = content_reader_parser::crt::scan_i32(&text)
                        .stored()
                        .map_or(state.stencil_ref, |value| value as u32);
                }
                _ => self.unexpected(),
            }
        }
        state
    }

    fn rasterizer_state(&mut self) -> RasterizerState {
        let mut state = RasterizerState::default();
        while self.next_key() {
            let key = keyword_of(&self.key.clone()).unwrap_or_default();
            let text = self.value.text.clone();
            match key {
                "fillmode" => {
                    if let Some(value) = value_index(&text, &["fill_wireframe", "fill_solid"]) {
                        state.fill_mode = value + 1;
                    }
                }
                "cullmode" => {
                    if let Some(value) =
                        value_index(&text, &["cull_none", "cull_front", "cull_back"])
                            .or_else(|| value_index(&text, &["none", "front", "back"]))
                    {
                        state.cull_mode = value;
                    }
                }
                "frontccw" => state.front_ccw = read_bool(&text).unwrap_or(state.front_ccw),
                "depthbias" => {
                    state.depth_bias = content_reader_parser::crt::scan_i32(&text)
                        .stored()
                        .unwrap_or(state.depth_bias)
                }
                "depthbiasclamp" => {
                    state.depth_bias_clamp =
                        content_reader_parser::crt::scan_f32(&text).unwrap_or(state.depth_bias_clamp);
                }
                "slopescaledepthbias" => {
                    state.slope_scaled_depth_bias =
                        content_reader_parser::crt::scan_f32(&text).unwrap_or(state.slope_scaled_depth_bias);
                }
                "scissor" => state.scissor = read_bool(&text).unwrap_or(state.scissor),
                "multisample" => state.multisample = read_bool(&text).unwrap_or(state.multisample),
                _ => self.unexpected(),
            }
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(files: &[(&str, &str)]) -> impl Fn(&[u8]) -> Option<Vec<u8>> {
        let files: Vec<(Vec<u8>, Vec<u8>)> = files
            .iter()
            .map(|(path, text)| (path.as_bytes().to_vec(), text.as_bytes().to_vec()))
            .collect();
        move |path| {
            files
                .iter()
                .find(|(own, _)| own.as_slice() == path)
                .map(|(_, bytes)| bytes.clone())
        }
    }

    const DECLARATIONS: &str = r#"Includes = { "fixture_helpers.fxh" }
PixelShader =
{
    Samplers =
    {
        Signal =
        {
            Index = 4
            MagFilter = "Point"
            MinFilter = "Linear"
            MipFilter = "Anisotropic"
            AddressU = "Mirror"
            AddressV = "Border"
            MipMapLodBias = 0.25
        }
        Detail = { Index = 1 AddressU = "Wrap" }
    }
}
VertexStruct Packet
{
    uint marker : TEXCOORD7;
@ifdef EXTRA_VALUE
    float3 payload : TEXCOORD6;
@endif
};
ConstantBuffer( 3, 7 )
{
    uint offset;
    float3 bands;
    int steps[3];
};
VertexShader =
{
    MainCode Emit
    [[
        float2 main() : TEXCOORD2 { return float2(2, 4); }
    ]]
}
PixelShader =
{
    MainCode Shade
    [[
        float main() : SV_TARGET { uint values[2] = {3,5}; return values[values[0] % 2]; }
    ]]
}
BlendState Mixed
{
    BlendEnable = yes
    SourceBlend = "one"
    DestBlend = "DEST_COLOR"
    WriteMask = "GREEN|ALPHA"
}
Effect Above
{
    VertexShader = "Emit"
    PixelShader = "Shade"
    BlendState = "Mixed"
}
Effect = Below
{
    VertexShader = "Emit"
    PixelShader = "Shade"
    BlendState = "Mixed"
    Defines = { "EXTRA_VALUE" }
}
"#;

    #[test]
    fn an_effect_keeps_its_full_facts() {
        content_reader_script::test_game::enter();
        let read = files(&[
            ("gfx/FX/fixture.shader", DECLARATIONS),
            (
                "gfx/FX/fixture_helpers.fxh",
                "Code\n[[\n struct Helper { uint value; };\n]]\nSamplers = { Shared = { Index = 2 } }",
            ),
        ]);
        let effect = load_with(b"gfx/FX/fixture.lua", &read).unwrap();
        assert_eq!(effect.path, "gfx/FX/fixture.shader");
        assert_eq!(effect.includes.len(), 1);
        assert!(effect.includes[0].1.is_some());
        let above = effect.technique(b"ABOVE").unwrap();
        assert_eq!(above.pixel_shader, b"Shade");
        assert_eq!(
            effect.technique(b"below").unwrap().defines,
            [b"EXTRA_VALUE".to_vec()]
        );
        let blend = effect.blend(above);
        assert!(blend.enable);
        assert_eq!((blend.source, blend.dest, blend.write_mask), (1, 8, 10));
        let slots = effect.sampler_slots().unwrap();
        let signal = slots[4].unwrap();
        assert_eq!(
            (
                signal.address_u,
                signal.address_v,
                signal.mag,
                signal.min,
                signal.mip
            ),
            (
                Address::Mirror,
                Address::Border,
                Filter::Point,
                Filter::Linear,
                Filter::Anisotropic
            )
        );
        assert_eq!(signal.lod_bias, 0.25);
        assert_eq!(slots[2].unwrap().name, b"Shared");
        assert!(slots[1].is_some());
        let structs = &effect.stages[0].vertex_structs[0];
        assert_eq!(structs.members[1].directive, b"#ifdef EXTRA_VALUE");
        assert_eq!(structs.members[0].semantic, b"TEXCOORD7");
        let buffer = &effect.stages[0].constant_buffers[0];
        assert_eq!(buffer.members[2], (b"int".to_vec(), b"steps".to_vec(), 3));
        assert!(effect.stages[0].code.starts_with(b" struct Helper"));
        assert_eq!(effect.stages[1].main_code[0].0, b"Emit");
        assert_eq!(load_with(b"gfx/FX/missing.lua", &read), None);
    }

    #[test]
    fn includes_and_stages_order_the_rows() {
        content_reader_script::test_game::enter();
        let read = files(&[
            (
                "gfx/FX/mod.shader",
                "Includes = { \"a.fxh\" \"b.fxh\" }\n\
                 PixelShader = { Samplers = { T = { AddressU = mirror AddressV = \"sideways\" } } }",
            ),
            (
                "gfx/FX/a.fxh",
                "Includes = { \"b.fxh\" }\nSamplers = { T = { Index = 0 AddressU = CLAMP } }",
            ),
            (
                "gfx/FX/b.fxh",
                "PixelShader = { Samplers = { T = { Index = 0 AddressU = border } } }",
            ),
        ]);
        let effect = load_with(b"gfx/FX/mod.lua", &read).unwrap();
        let paths: Vec<_> = effect
            .includes
            .iter()
            .map(|(path, _)| path.as_str())
            .collect();
        assert_eq!(paths, ["gfx/FX/b.fxh", "gfx/FX/a.fxh"]);
        let slot = effect.sampler_slots().unwrap()[0].unwrap();
        assert_eq!(
            (slot.address_u, slot.address_v),
            (Address::Mirror, Address::Wrap)
        );
    }

    #[test]
    fn members_end_at_the_end_of_the_text() {
        content_reader_script::test_game::enter();
        for tail in [&b""[..], b"\xff"] {
            let text = [b"VertexStruct VS { float2 a : TEXCOORD0 ".as_slice(), tail].concat();
            let file = read_file(&text).unwrap();
            let members = &file.stages[0].vertex_structs[0].members;
            assert_eq!(members.len(), 1, "{tail:?}");
            assert_eq!(members[0].kind, b"float2");
            let text = [b"ConstantBuffer( 1, 2 ) { float4 x ".as_slice(), tail].concat();
            let file = read_file(&text).unwrap();
            let buffer = &file.stages[0].constant_buffers[0];
            assert_eq!(buffer.members.len(), 1, "{tail:?}");
            assert_eq!(buffer.members[0].1, b"x");
        }
    }

    #[test]
    fn declaration_names_keep_their_line_feeds() {
        content_reader_script::test_game::enter();
        let text = "PixelShader = { Samplers = { \"Map\nTexture\" = { Index = 1 } } }\n\
                    BlendState \"multi\nline\" { BlendEnable = yes }\n\
                    DepthStencilState \"a\0b\" { }\n\
                    Effect \"Up\nTwo\" { BlendState = \"multi\nline\" }\n";
        let file = read_file(text.as_bytes()).unwrap();
        assert_eq!(file.blend_states[0].0, b"multi\nline");
        assert_eq!(file.depth_stencil_states[0].0, b"a");
        assert_eq!(file.techniques[0].name, b"Up\nTwo");
        assert_eq!(file.techniques[0].blend_state, b"Unreadable String");
        assert_eq!(file.stages[2].samplers[0].name, b"Map\nTexture");
        let listed = [("gfx/FX/x.shader", text)];
        let read = files(&listed);
        let effect = load_with(b"gfx/FX/x.shader", &read).unwrap();
        let up = effect.technique(b"UP\nTWO").unwrap();
        assert!(!effect.blend(up).enable);
        let text = text.replace(
            "BlendState \"multi\nline\"",
            "BlendState \"Unreadable String\"",
        );
        let listed = [("gfx/FX/x.shader", text.as_str())];
        let read = files(&listed);
        let effect = load_with(b"gfx/FX/x.shader", &read).unwrap();
        assert!(effect.blend(effect.technique(b"up\ntwo").unwrap()).enable);
    }
}
