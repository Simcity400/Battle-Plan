
use std::collections::BTreeSet;

use super::{Address, effect_path};
use content_reader_layers::ResolvedLayers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Common,
    Vertex,
    Pixel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub path: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Structure {
    pub name: String,
    pub body: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub ty: String,
    pub name: String,
    pub semantic: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstantBuffer {
    pub index: u32,
    pub legacy_register: u32,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Point,
    Linear,
    Anisotropic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sampler {
    pub name: String,
    pub index: u32,
    pub cube: bool,
    pub mag: Filter,
    pub min: Filter,
    pub mip: Filter,
    pub u: Address,
    pub v: Address,
    pub w: Address,
    pub lod_bias: f32,
    pub min_lod: i32,
    pub max_lod: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Declarations {
    pub samplers: Vec<Sampler>,
    pub structures: Vec<Structure>,
    pub buffers: Vec<ConstantBuffer>,
    pub code: Vec<Source>,
    pub entries: Vec<Entry>,
}

impl Declarations {
    fn append(&mut self, mut other: Self) {
        self.samplers.append(&mut other.samplers);
        self.structures.append(&mut other.structures);
        self.buffers.append(&mut other.buffers);
        self.code.append(&mut other.code);
        self.entries.append(&mut other.entries);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Technique {
    pub name: String,
    pub vertex: String,
    pub pixel: String,
    pub defines: Vec<String>,
    pub blend: String,
    pub depth_stencil: String,
    pub rasterizer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State<T> {
    pub name: String,
    pub value: T,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub effect: String,
    pub identity: [u8; 32],
    pub dependencies: Vec<Vec<u8>>,
    pub warnings: Vec<String>,
    pub common: Declarations,
    pub vertex: Declarations,
    pub pixel: Declarations,
    pub techniques: Vec<Technique>,
    pub blends: Vec<State<super::BlendState>>,
    pub depth_stencils: Vec<State<super::DepthStencilState>>,
    pub rasterizers: Vec<State<super::RasterizerState>>,
    pub header: Source,
}

#[derive(Default)]
struct File {
    includes: Vec<Vec<u8>>,
    program: Program,
}

pub fn read(layers: &ResolvedLayers, effect: &[u8]) -> Result<Program, String> {
    read_layers(layers, effect, false)
}

pub fn read_reported(layers: &ResolvedLayers, effect: &[u8]) -> Result<Program, String> {
    let program = read_layers(layers, effect, true)?;
    for warning in &program.warnings {
        layers.note_shader_warning(warning.clone());
    }
    Ok(program)
}

fn read_layers(layers: &ResolvedLayers, effect: &[u8], reported: bool) -> Result<Program, String> {
    let mut parameters = vec![u8::from(reported)];
    parameters.extend_from_slice(effect);
    layers.product("shader program assembly", 1, &parameters, |layers| {
        read_layer_program(layers, effect, reported).map_err(std::io::Error::other)
    }).map(|program| program.as_ref().clone()).map_err(|error| error.to_string())
}

fn read_layer_program(layers: &ResolvedLayers, effect: &[u8], reported: bool) -> Result<Program, String> {
    let errors = std::cell::RefCell::new(Vec::new());
    let read = |path: &[u8]| {
        if reported {
            layers.read_path(path)
        } else {
            layers.read_uncovered_path(path)
        }
    };
    let result = read_with(effect, &|path| match read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            errors
                .borrow_mut()
                .push(format!("{}: {error}", lossy(path)));
            None
        }
    });
    let errors = errors.into_inner();
    if !errors.is_empty() {
        return Err(format!("effect {:?}: {}", lossy(effect), errors.join("; ")));
    }
    let mut program = result?;
    let uncovered: Vec<_> = program
        .dependencies
        .iter()
        .filter(|path| !layers.covered_path(path))
        .map(|path| lossy(path))
        .collect();
    if !reported && !uncovered.is_empty() {
        program.warnings.push(format!("Effect {:?} reads sources outside the content coverage policy; these bytes are not hash-verified: {}", lossy(effect), uncovered.join(", ")));
    }
    Ok(program)
}

pub fn read_with<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_layers::content::Bytes>>(
    effect: &[u8],
    read: &dyn Fn(&[u8]) -> Option<B>,
) -> Result<Program, String> {
    use sha2::{Digest, Sha256};
    let digest = std::cell::RefCell::new(Sha256::new());
    let mut program = read_with_inner(effect, &|path| {
        let bytes = read(path);
        let mut hash = digest.borrow_mut();
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path);
        hash.update([u8::from(bytes.is_some())]);
        if let Some(bytes) = &bytes {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        bytes
    })?;
    program.identity = digest.into_inner().finalize().into();
    Ok(program)
}

fn include(name: &[u8]) -> Vec<u8> {
    [b"gfx/FX/".as_slice(), name].concat()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn read_with_inner<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_layers::content::Bytes>>(
    effect: &[u8],
    read: &dyn Fn(&[u8]) -> Option<B>,
) -> Result<Program, String> {
    fn walk<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_layers::content::Bytes>>(
        path: &[u8],
        read: &dyn Fn(&[u8]) -> Option<B>,
        seen: &mut BTreeSet<Vec<u8>>,
        out: &mut Vec<(Vec<u8>, File)>,
    ) -> Result<(), String> {
        enum Visit {
            Open(Vec<u8>),
            Close(Vec<u8>, Box<File>),
        }
        let mut stack = vec![Visit::Open(path.to_owned())];
        while let Some(visit) = stack.pop() {
            match visit {
                Visit::Close(path, file) => out.push((path, *file)),
                Visit::Open(path) => {
                    if !seen.insert(path.clone()) {
                        continue;
                    }
                    let Some(bytes) = read(&path) else {
                        continue;
                    };
                    let file = parse(&lossy(&path), bytes)?;
                    let children: Vec<_> = file
                        .includes
                        .iter()
                        .rev()
                        .map(|name| Visit::Open(include(name)))
                        .collect();
                    stack.push(Visit::Close(path, Box::new(file)));
                    stack.extend(children);
                }
            }
        }
        Ok(())
    }
    let name = lossy(effect);
    let result = || {
        let path = effect_path(effect);
        let mut files = Vec::new();
        let Some(bytes) = read(&path) else {
            return Ok(Program {
                effect: name.clone(),
                warnings: vec![format!(
                    "effect {name:?}: missing {}; empty technique record",
                    lossy(&path)
                )],
                dependencies: vec![path],
                ..Program::default()
            });
        };
        let root = parse(&lossy(&path), bytes)?;
        let mut seen = BTreeSet::new();
        for name in &root.includes {
            walk(&include(name), read, &mut seen, &mut files)?;
        }
        let mut program = Program {
            effect: name.clone(),
            techniques: root.program.techniques,
            blends: root.program.blends,
            depth_stencils: root.program.depth_stencils,
            rasterizers: root.program.rasterizers,
            ..Program::default()
        };
        for missing in seen
            .iter()
            .filter(|path| !files.iter().any(|(name, _)| name == *path))
        {
            program
                .warnings
                .push(format!("shader include {:?} not loaded", lossy(missing)));
        }
        for (name, file) in files {
            program.dependencies.push(name);
            program.common.append(file.program.common);
            program.vertex.append(file.program.vertex);
            program.pixel.append(file.program.pixel);
        }
        program.dependencies.push(path);
        program.common.append(root.program.common);
        program.vertex.append(root.program.vertex);
        program.pixel.append(root.program.pixel);
        let header = "gfx/FX/defines_hlsl_dx11.fxh";
        program.header = Source {
            path: header.into(),
            text: std::str::from_utf8(
                &read(header.as_bytes()).ok_or_else(|| format!("missing {header}"))?,
            ).map_err(|e| format!("{header}: {e}"))?.to_owned(),
        };
        program.dependencies.push(header.into());
        Ok(program)
    };
    result().map_err(|reason: String| format!("effect {name:?}: {reason}"))
}

impl Program {
    pub fn technique(&self, name: &str) -> Option<&Technique> {
        self.techniques
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn blend(&self, technique: &Technique) -> super::BlendState {
        named(&self.blends, &technique.blend)
    }

    pub fn depth_stencil(&self, technique: &Technique) -> super::DepthStencilState {
        named(&self.depth_stencils, &technique.depth_stencil)
    }

    pub fn rasterizer(&self, technique: &Technique) -> super::RasterizerState {
        named(&self.rasterizers, &technique.rasterizer)
    }

    pub fn declarations(&self, stage: Stage) -> &Declarations {
        match stage {
            Stage::Common => &self.common,
            Stage::Vertex => &self.vertex,
            Stage::Pixel => &self.pixel,
        }
    }

    pub fn static_float(&self, name: &str, defines: &[String]) -> Result<f32, String> {
        let effect = &self.effect;
        let defined = |define: &String| {
            let mut words = define
                .trim_start()
                .split(|c: char| matches!(c, '=' | '(') || c.is_whitespace());
            words.next() == Some(name)
        };
        if defines
            .iter()
            .chain(self.techniques.iter().flat_map(|technique| &technique.defines))
            .any(defined)
        {
            return Err(format!("{effect}: {name} is defined for the compiler"));
        }
        let mut found = Vec::new();
        let mut depth = 0;
        for source in std::iter::once(&self.header).chain(&self.common.code) {
            scan_static_floats(&source.text, name, &mut depth, &mut found);
        }
        for stage in [&self.vertex, &self.pixel] {
            let mut depth = depth;
            for source in &stage.code {
                scan_static_floats(&source.text, name, &mut depth, &mut found);
            }
        }
        match found.as_slice() {
            [] => Err(format!("{effect} declares no static const float {name}")),
            [Found::Declared(value)] => value
                .clone()
                .map_err(|text| format!("{effect}: {name} is `{text}`, not a literal")),
            found if found.contains(&Found::Macro) => {
                Err(format!("{effect}: {name} is a preprocessor macro"))
            }
            found if found.contains(&Found::Conditional) => Err(format!(
                "{effect}: {name} is declared inside a preprocessor conditional"
            )),
            _ => Err(format!("{effect} declares {name} more than once")),
        }
    }

    pub fn sampler(&self, index: u32) -> Option<&Sampler> {
        self.common
            .samplers
            .iter()
            .chain(&self.pixel.samplers)
            .rev()
            .find(|s| s.index == index)
    }

    pub fn hlsl(
        &self,
        technique: &Technique,
        stage: Stage,
        defines: &[String],
    ) -> Result<String, String> {
        let (define, name) = match stage {
            Stage::Vertex => ("VERTEX_SHADER", &technique.vertex),
            Stage::Pixel => ("PIXEL_SHADER", &technique.pixel),
            Stage::Common => return Err("common declarations are not an executable stage".into()),
        };
        let decl = self.declarations(stage);
        let main = self
            .common
            .entries
            .iter()
            .chain(&decl.entries)
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                format!(
                    "effect {:?}, technique {:?}: missing {stage:?} MainCode {name:?}",
                    self.effect, technique.name
                )
            })?;
        let mut out = format!("#define {define}\n");
        for define in defines.iter().chain(&technique.defines) {
            out.push_str(&format!("#define {define}\n"));
        }
        source(&mut out, &self.header);
        for Sampler {
            name, index, cube, ..
        } in &decl.samplers
        {
            let kind = if *cube { "Cube" } else { "2D" };
            out.push_str(&format!("Texture{kind} {name}_Texture : register(t{index});\nSamplerState {name}_Sampler : register(s{index});\n#define {name} CreateSampler{kind}({name}_Texture, {name}_Sampler)\n"));
        }
        for group in [&self.common, decl] {
            for buffer in &group.buffers {
                out.push_str(&format!(
                    "cbuffer dx11_cb{} : register(b{}) {{\n{}\n}}\n",
                    buffer.index, buffer.index, buffer.body
                ));
            }
        }
        for group in [&self.common, decl] {
            for structure in &group.structures {
                out.push_str(&format!(
                    "struct {} {{\n{}\n}};\n",
                    structure.name, structure.body
                ));
            }
        }
        for group in [&self.common, decl] {
            for code in &group.code {
                source(&mut out, code);
            }
        }
        source(&mut out, &main.source);
        Ok(out)
    }
}

fn named<T: Clone + Default>(states: &[State<T>], name: &str) -> T {
    states
        .iter()
        .find(|state| state.name.eq_ignore_ascii_case(name))
        .map(|state| state.value.clone())
        .unwrap_or_default()
}

fn source(out: &mut String, source: &Source) {
    out.push_str(&format!(
        "\n#line 1 \"{}\"\n{}\n",
        source.path.replace('\\', "/").replace('"', ""),
        source.text
    ));
}

fn parse(path: &str, bytes: impl Into<content_reader_layers::content::Bytes>) -> Result<File, String> {
    fn text(path: &str, bytes: &[u8]) -> Result<String, String> {
        String::from_utf8(bytes.to_vec())
            .map_err(|e| format!("{path}: non-UTF8 shader token/code: {e}"))
    }
    fn named<T>(path: &str, (name, value): (Vec<u8>, T)) -> Result<State<T>, String> {
        Ok(State {
            name: text(path, &name)?,
            value,
        })
    }
    fn declarations(path: &str, stage: super::Stage) -> Result<Declarations, String> {
        let filter = |filter| match filter {
            super::Filter::None | super::Filter::Point => Filter::Point,
            super::Filter::Linear => Filter::Linear,
            super::Filter::Anisotropic => Filter::Anisotropic,
        };
        let samplers = stage
            .samplers
            .into_iter()
            .map(|s| {
                Ok(Sampler {
                    name: text(path, &s.name)?,
                    index: s.index,
                    cube: s.kind == 1,
                    mag: filter(s.mag),
                    min: filter(s.min),
                    mip: filter(s.mip),
                    u: s.address_u,
                    v: s.address_v,
                    w: Address::Wrap,
                    lod_bias: s.lod_bias,
                    min_lod: s.min_lod,
                    max_lod: s.max_lod,
                })
            })
            .collect::<Result<_, String>>()?;
        let structures = stage
            .vertex_structs
            .into_iter()
            .map(|structure| {
                let mut body = String::new();
                let mut members = Vec::new();
                for member in structure.members {
                    if !member.directive.is_empty() {
                        body.push_str(&text(path, &member.directive)?);
                        body.push('\n');
                    } else if !member.kind.is_empty() {
                        let ty = text(path, &member.kind)?;
                        let name = text(path, &member.name)?;
                        let semantic = text(path, &member.semantic)?;
                        body.push_str(&format!("{ty} {name} : {semantic};\n"));
                        members.push(Member { ty, name, semantic });
                    }
                }
                Ok(Structure {
                    name: text(path, &structure.name)?,
                    body,
                    members,
                })
            })
            .collect::<Result<_, String>>()?;
        let buffers = stage
            .constant_buffers
            .into_iter()
            .map(|buffer| {
                let mut body = String::new();
                for (ty, name, count) in buffer.members {
                    let (ty, name) = (text(path, &ty)?, text(path, &name)?);
                    if count == 0 {
                        body.push_str(&format!("{ty} {name};\n"));
                    } else {
                        body.push_str(&format!("{ty} {name}[{count}];\n"));
                    }
                }
                Ok(ConstantBuffer {
                    index: buffer.arguments[0],
                    legacy_register: buffer.arguments[1],
                    body,
                })
            })
            .collect::<Result<_, String>>()?;
        let code = vec![Source {
            path: path.into(),
            text: text(path, &stage.code)?,
        }];
        let entries = stage
            .main_code
            .into_iter()
            .map(|(name, code)| {
                Ok(Entry {
                    name: text(path, &name)?,
                    source: Source {
                        path: path.into(),
                        text: text(path, &code)?,
                    },
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Declarations {
            samplers,
            structures,
            buffers,
            code,
            entries,
        })
    }
    let file = super::read_file(bytes).map_err(|error| format!("{path}: {error}"))?;
    let [common, vertex, pixel] = file.stages;
    Ok(File {
        includes: file.includes,
        program: Program {
            common: declarations(path, common)?,
            vertex: declarations(path, vertex)?,
            pixel: declarations(path, pixel)?,
            techniques: file
                .techniques
                .into_iter()
                .map(|t| {
                    Ok(Technique {
                        name: text(path, &t.name)?,
                        vertex: text(path, &t.vertex_shader)?,
                        pixel: text(path, &t.pixel_shader)?,
                        defines: t
                            .defines
                            .iter()
                            .map(|s| text(path, s))
                            .collect::<Result<_, _>>()?,
                        blend: text(path, &t.blend_state)?,
                        depth_stencil: text(path, &t.depth_stencil_state)?,
                        rasterizer: text(path, &t.rasterizer_state)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            blends: file
                .blend_states
                .into_iter()
                .map(|s| named(path, s))
                .collect::<Result<_, _>>()?,
            depth_stencils: file
                .depth_stencil_states
                .into_iter()
                .map(|s| named(path, s))
                .collect::<Result<_, _>>()?,
            rasterizers: file
                .rasterizer_states
                .into_iter()
                .map(|s| named(path, s))
                .collect::<Result<_, _>>()?,
            ..Default::default()
        },
    })
}

#[derive(Clone, Debug, PartialEq)]
enum Found {
    Declared(Result<f32, String>),
    Conditional,
    Macro,
}

fn scan_static_floats(code: &str, name: &str, depth: &mut u32, found: &mut Vec<Found>) {
    let plain = uncommented(code);
    let (mut statement, mut conditional) = (String::new(), false);
    let mut lines = plain.lines();
    while let Some(line) = lines.next() {
        if let Some(directive) = line.trim_start().strip_prefix('#') {
            let mut directive = directive.to_owned();
            while let Some(joined) = directive.trim_end().strip_suffix('\\') {
                directive = format!("{joined} {}", lines.next().unwrap_or_default());
            }
            let mut words = directive
                .split(|c: char| c.is_whitespace() || c == '(')
                .filter(|word| !word.is_empty());
            let word = words.next();
            match word {
                Some("if" | "ifdef" | "ifndef") => *depth += 1,
                Some("endif") => *depth = depth.saturating_sub(1),
                Some("define" | "undef") if words.next() == Some(name) => found.push(Found::Macro),
                _ => {}
            }
            let branch = matches!(
                word,
                Some("if" | "ifdef" | "ifndef" | "else" | "elif" | "endif")
            );
            conditional |= branch && !statement.trim().is_empty();
            continue;
        }
        let mut pieces = line.split(';').peekable();
        while let Some(piece) = pieces.next() {
            statement.push_str(piece);
            statement.push(' ');
            conditional |= *depth > 0 && !piece.trim().is_empty();
            if pieces.peek().is_some() {
                found.extend(static_float_in(&statement, name, conditional));
                statement.clear();
                conditional = false;
            }
        }
    }
}

fn static_float_in(statement: &str, name: &str, conditional: bool) -> Option<Found> {
    let spaced = statement.replace('=', " = ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let at = tokens
        .windows(5)
        .position(|window| window == ["static", "const", "float", name, "="])?;
    if conditional {
        return Some(Found::Conditional);
    }
    let text = tokens[at + 5..].join(" ");
    let literal = text.strip_suffix(['f', 'F']).unwrap_or(&text);
    Some(Found::Declared(literal.parse::<f32>().map_err(|_| text.clone())))
}

fn uncommented(code: &str) -> String {
    let mut plain = String::with_capacity(code.len());
    let mut rest = code;
    while !rest.is_empty() {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (Some(at), block) if block.is_none_or(|block| at < block) => {
                plain.push_str(&rest[..at]);
                rest = rest[at..].find('\n').map_or("", |end| &rest[at + end..]);
            }
            (_, Some(at)) => {
                plain.push_str(&rest[..at]);
                let end = rest[at + 2..].find("*/").map_or(rest.len(), |end| at + 2 + end + 2);
                plain.push(' ');
                plain.extend(rest[at..end].chars().filter(|&c| c == '\n'));
                rest = &rest[end..];
            }
            _ => {
                plain.push_str(rest);
                rest = "";
            }
        }
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_static_float_reads_its_one_literal_declaration_and_refuses_anything_else() {
        let program = |texts: &[&str]| Program {
            effect: "gfx/FX/test.shader".into(),
            identity: [0; 32],
            dependencies: Vec::new(),
            warnings: Vec::new(),
            common: Declarations {
                code: texts
                    .iter()
                    .map(|text| Source {
                        path: "gfx/FX/constants.fxh".into(),
                        text: (*text).into(),
                    })
                    .collect(),
                ..Default::default()
            },
            vertex: Declarations::default(),
            pixel: Declarations::default(),
            techniques: Vec::new(),
            blends: Vec::new(),
            depth_stencils: Vec::new(),
            rasterizers: Vec::new(),
            header: Source::default(),
        };
        let constants = "// static const float  WATER_HEIGHT = 1.0f;\n\
            static const float  WATER_HEIGHT = 9.5f;\n\
            static const float  WATER_HEIGHT_RECP = 1.0f / WATER_HEIGHT;\n\
            /* static const float WATER_HEIGHT = 2; */";
        let read = |texts: &[&str]| program(texts).static_float("WATER_HEIGHT", &[]);
        let refusal = |texts: &[&str], reason: &str| {
            let error = read(texts).unwrap_err();
            assert!(error.contains(reason), "{texts:?}: {error}");
        };
        assert_eq!(read(&[constants]), Ok(9.5));
        assert_eq!(program(&["static const float X=-3;"]).static_float("X", &[]), Ok(-3.0));
        assert!(program(&[constants]).static_float("WATER", &[]).is_err());
        refusal(&[constants, constants], "more than once");
        refusal(&["static const float WATER_HEIGHT = 19.0f / 2.0f;"], "not a literal");
        let modded = "#ifdef MOD_WATER\nstatic const float WATER_HEIGHT = 12;\n#else\n\
            #define WATER_HEIGHT 9.5\n#endif\n";
        refusal(&[modded], "macro");
        refusal(&["#if 0\nstatic const float WATER_HEIGHT = 2.0f;\n#endif\n"], "conditional");
        let split = ["#ifdef A\n", "static const float WATER_HEIGHT = 2.0f;\n", "#endif\n"];
        refusal(&split, "conditional");
        let cut = "static const float WATER_HEIGHT =\n#if A\n2;\n#else\n9.5;\n#endif\n";
        refusal(&[cut], "conditional");
        refusal(&[constants, "#undef WATER_HEIGHT\n#define WATER_HEIGHT 2.0f\n"], "macro");
        let commented = "/* #ifdef A\n*/ static const float WATER_HEIGHT = 9.5f;\n// #endif\n";
        assert_eq!(read(&[commented]), Ok(9.5));
        let closed = "#ifdef A\nstatic const float OTHER = 1;\n#endif\n";
        assert_eq!(read(&[closed, constants]), Ok(9.5));
        for define in ["WATER_HEIGHT=2", " WATER_HEIGHT 2.0", "WATER_HEIGHT(x) 2.0"] {
            let defined = program(&[constants]).static_float("WATER_HEIGHT", &[define.into()]);
            assert!(defined.unwrap_err().contains("defined for the compiler"), "{define}");
        }
        let other = ["WATER_HEIGHT_RECP 2".into()];
        assert_eq!(program(&[constants]).static_float("WATER_HEIGHT", &other), Ok(9.5));
    }

    #[test]
    fn source_order_and_first_entry_follow_declaration_order() {
        content_reader_script::test_game::enter();
        let read = |path: &[u8]| {
            Some(match path {
            b"gfx/FX/test.shader" => "Includes = { a.fxh b.fxh } MainCode PS [[ float4 main() : PDX_COLOR { return 1; } ]] PixelShader = { MainCode PS [[ WRONG ]] } Effect Up { PixelShader = PS Defines = { EXTRA } }",
            b"gfx/FX/a.fxh" => "Includes = { b.fxh } Code [[\nA\n]]",
            b"gfx/FX/b.fxh" => "Includes = { a.fxh } Code [[\nB\n]]",
            b"gfx/FX/defines_hlsl_dx11.fxh" => "HEADER", _ => return None,
        }.as_bytes().to_vec())
        };
        let program = read_with(b"gfx/FX/test.lua", &read).unwrap();
        assert_eq!(
            &program.dependencies[..3],
            &[
                b"gfx/FX/b.fxh".to_vec(),
                b"gfx/FX/a.fxh".to_vec(),
                b"gfx/FX/test.shader".to_vec()
            ]
        );
        let code = program
            .hlsl(
                program.technique("UP").unwrap(),
                Stage::Pixel,
                &["CALLER".into()],
            )
            .unwrap();
        assert!(code.starts_with("#define PIXEL_SHADER\n#define CALLER\n#define EXTRA\n"));
        assert!(!code.contains("WRONG"));
        assert!(code.find("\nB\n").unwrap() < code.find("\nA\n").unwrap());
    }

    #[test]
    fn only_cube_samplers_change_the_dx11_texture_type() {
        content_reader_script::test_game::enter();
        let program = read_with(b"gfx/FX/t.lua", &|path| {
            Some(match path {
                b"gfx/FX/t.shader" => "PixelShader = { Samplers = { A = { Index = 0 Type = cube } B = { Index = 1 Type = 3d } C = { Index = 2 Type = shadow } } MainCode P [[ ]] } DepthStencilState Depth { DepthEnable = no } Effect Up { PixelShader = P DepthStencilState = DEPTH }",
                b"gfx/FX/defines_hlsl_dx11.fxh" => "",
                _ => return None,
            }.as_bytes().to_vec())
        })
        .unwrap();
        let up = &program.techniques[0];
        let code = program.hlsl(up, Stage::Pixel, &[]).unwrap();
        assert!(code.contains("TextureCube A_Texture : register(t0);"));
        assert!(code.contains("Texture2D B_Texture : register(t1);"));
        assert!(code.contains("#define C CreateSampler2D(C_Texture, C_Sampler)"));
        assert!(!program.depth_stencil(up).depth_enable);
        assert_eq!(
            program.rasterizer(up),
            super::super::RasterizerState::default()
        );
    }

    #[test]
    fn declarations_preserve_code_and_buffer_layout() {
        content_reader_script::test_game::enter();
        let file = parse("test", b"VertexStruct V { float3 p : POSITION;\n@ifdef TEST\nfloat2 uv : TEXCOORD0;\n@endif\n};\nConstantBuffer(2, 9) { float4x4 matrix;\nfloat4 array[3];\n};\nCode [[ float x[2] = {1,2}; // kept\n]]\n").unwrap();
        assert_eq!(file.program.common.buffers[0].legacy_register, 9);
        assert!(file.program.common.buffers[0].body.contains("array[3];"));
        assert!(
            file.program.common.structures[0]
                .body
                .contains("#ifdef TEST\nfloat2 uv : TEXCOORD0;\n#endif")
        );
        assert!(file.program.common.code[0].text.contains("// kept"));
    }

    #[test]
    fn includes_contribute_declarations_but_never_replace_root_states() {
        content_reader_script::test_game::enter();
        let program = read_with(b"gfx/FX/mod.lua", &|path| {
            Some(match path {
                b"gfx/FX/mod.shader" => "Includes = { child.fxh missing.fxh child.fxh } BlendState Alpha { BlendEnable = yes } Effect Up { PixelShader = P BlendState = Alpha } Effect up { PixelShader = Wrong }",
                b"gfx/FX/child.fxh" => "BlendState Alpha { BlendEnable = no } Effect Up { PixelShader = Wrong } MainCode P [[ float4 main() : PDX_COLOR { return 1; } ]]",
                b"gfx/FX/defines_hlsl_dx11.fxh" => "#define PDX_COLOR SV_TARGET\n",
                _ => return None,
            }.as_bytes().to_vec())
        }).unwrap();
        assert_eq!(program.common.entries.len(), 1);
        assert_eq!(program.blends.len(), 1);
        assert!(program.blends[0].value.enable);
        assert_eq!(program.technique("UP").unwrap().pixel, "P");
        assert_eq!(program.warnings.len(), 1);
        assert!(program.warnings[0].contains("missing.fxh"));
    }

    #[test]
    fn parse_failures_name_the_effect_and_missing_main_names_the_technique() {
        content_reader_script::test_game::enter();
        let error =
            read_with(b"mod/broken.lua", &|_| Some(b"Code [[ \x80 ]]".to_vec())).unwrap_err();
        assert!(error.contains("mod/broken.lua"));
        assert!(error.contains("non-UTF8 shader token/code"));
        let program = read_with(b"mod/empty.lua", &|path| {
            Some(if path.ends_with(b".shader") {
                b"Effect Down { VertexShader = V PixelShader = P }\n".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let error = program
            .hlsl(program.technique("Down").unwrap(), Stage::Pixel, &[])
            .unwrap_err();
        assert!(error.contains("mod/empty.lua"));
        assert!(error.contains("Down"));
        assert!(error.contains("missing Pixel MainCode"));
    }

    #[test]
    fn effect_and_include_names_open_by_their_bytes() {
        content_reader_script::test_game::enter();
        let read = |path: &[u8]| {
            Some(match path {
                b"gfx/FX/\x80.shader" => b"Includes = { \"\x81.fxh\" }\nMainCode P [[ float4 main() : PDX_COLOR { return 1; } ]]\nEffect Up { PixelShader = P }".to_vec(),
                b"gfx/FX/\x81.fxh" => b"Code [[\nINCLUDED\n]]".to_vec(),
                b"gfx/FX/defines_hlsl_dx11.fxh" => Vec::new(),
                _ => return None,
            })
        };
        let program = read_with(b"gfx/FX/\x80.lua", &read).unwrap();
        assert_eq!(
            program.dependencies,
            [
                b"gfx/FX/\x81.fxh".to_vec(),
                b"gfx/FX/\x80.shader".to_vec(),
                b"gfx/FX/defines_hlsl_dx11.fxh".to_vec()
            ]
        );
        assert!(program.warnings.is_empty(), "{:?}", program.warnings);
        assert_eq!(program.effect, "gfx/FX/\u{FFFD}.lua");
        let code = program
            .hlsl(&program.techniques[0], Stage::Pixel, &[])
            .unwrap();
        assert!(code.contains("\nINCLUDED\n"));
        let missing = read_with("gfx/FX/\u{FFFD}.lua".as_bytes(), &read).unwrap();
        assert!(missing.techniques.is_empty());
        assert!(missing.warnings[0].contains("empty technique record"));
    }

    #[test]
    fn constant_array_counts_use_the_integer_scanner() {
        content_reader_script::test_game::enter();
        let file = parse("arrays", b"ConstantBuffer(0,0) { float4 octal[010];\nfloat4 hex[0x10];\nfloat4 no_count[MACRO];\nfloat4 zero[0];\n}\n").unwrap();
        assert_eq!(
            file.program.common.buffers[0].body,
            "float4 octal[8];\nfloat4 hex[16];\nfloat4 no_count;\nfloat4 zero;\n"
        );
    }

    #[test]
    fn only_sources_outside_the_covered_effect_root_are_named_unverified() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_effect_coverage_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let effect = b"Includes = { common.fxh }\nMainCode P [[ float4 main() : PDX_COLOR { return 1; } ]]\nEffect Up { PixelShader = P }";
        for (path, bytes) in [
            ("gfx/FX/defines_hlsl_dx11.fxh", &b""[..]),
            ("gfx/FX/common.fxh", b"Code [[\nC\n]]"),
            ("gfx/FX/map.shader", effect),
            ("gfx/models/rail.shader", effect),
        ] {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let covered = read(&layers, b"gfx/FX/map.lua").unwrap();
        assert_eq!(covered.dependencies.len(), 3);
        assert!(covered.warnings.is_empty(), "{:?}", covered.warnings);
        let uncovered = read(&layers, b"gfx/models/rail.lua").unwrap();
        assert_eq!(uncovered.warnings.len(), 1, "{:?}", uncovered.warnings);
        assert!(uncovered.warnings[0].ends_with("not hash-verified: gfx/models/rail.shader"));
        for policy in [2, 3] {
            let layers =
                ResolvedLayers::from_folders_with_coverage_policy(&[(&root, &[])], policy).unwrap();
            let old = read(&layers, b"gfx/FX/map.lua").unwrap();
            assert_eq!(
                old.identity, covered.identity,
                "same reader, same source bytes"
            );
            assert_eq!(old.warnings.len(), 1, "{:?}", old.warnings);
            assert!(old.warnings[0].contains(
                "outside the content coverage policy; these bytes are not hash-verified"
            ));
            for path in [
                "gfx/FX/map.shader",
                "gfx/FX/common.fxh",
                "gfx/FX/defines_hlsl_dx11.fxh",
            ] {
                assert!(old.warnings[0].contains(path), "{}", old.warnings[0]);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
