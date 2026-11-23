
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

use std::{collections::BTreeMap, path::Path};

use content_reader_gui::pdxshader::program::{Program, Stage, Technique};

pub mod artwork;
pub mod atlas;
mod batching;
pub mod cache;
pub mod clock;
mod dxc;
pub mod gpu;
pub mod library;
mod numeric;
mod reads;
mod sampling;
mod source;
pub mod stream;
pub mod texel;
pub mod vertices;

pub fn binding(index: u32, stage: Stage) -> u32 {
    index
        .saturating_mul(2)
        .saturating_add(u32::from(stage == Stage::Pixel))
}

const BUFFER_SLOTS: u32 = 14;
const SAMPLER_SLOTS: u32 = 16;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum TextureKind {
    D2,
    Cube,
    D3,
}
impl TextureKind {
    pub fn view(self) -> wgpu::TextureViewDimension {
        match self {
            Self::D2 => wgpu::TextureViewDimension::D2,
            Self::Cube => wgpu::TextureViewDimension::Cube,
            Self::D3 => wgpu::TextureViewDimension::D3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Buffer {
    pub binding: u32,
    pub size: u32,
    pub reads: Vec<std::ops::Range<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScalarKind {
    Float,
    Sint,
    Uint,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VertexInput {
    pub kind: ScalarKind,
    pub location: u32,
    pub semantic: String,
    pub components: u32,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Shader {
    pub wgsl: String,
    pub independent_draws: bool,
    pub buffers: Vec<Buffer>,
    pub textures: Vec<(u32, TextureKind)>,
    pub samplers: Vec<u32>,
    pub sampling_pairs: Vec<(u32, u32)>,
    pub inputs: Vec<VertexInput>,
    pub clock: clock::Clock,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Compiled {
    pub vertex: Shader,
    pub pixel: Shader,
}

impl Compiled {
    pub fn settles(&self, register: u32, time: u32, since: u32) -> Option<clock::Settling> {
        let mut settles = clock::Settling::NOW;
        for (stage, shader) in [(Stage::Vertex, &self.vertex), (Stage::Pixel, &self.pixel)] {
            let binding = binding(register, stage);
            settles = settles.and(shader.clock.settles((binding, time), (binding, since))?);
        }
        Some(settles)
    }
}

pub const DXC_VERSION: &str = "1.9.0.5402 (0d3ee6b55-dirty)";

pub struct Compiler {
    library: dxc::Library,
}

impl Compiler {
    pub fn discover() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|_| MISSING_COMPILER.to_owned())?;
        Self::at(&exe.with_file_name("dxcompiler.dll"))
    }

    pub fn at(library: &Path) -> Result<Self, String> {
        if !library.is_file() {
            return Err(MISSING_COMPILER.into());
        }
        let version = dxc::product_version(library)?;
        if version != DXC_VERSION {
            return Err(format!("dxcompiler.dll is {version:?}, not the pinned {DXC_VERSION}"));
        }
        Ok(Self { library: dxc::Library::load(library)? })
    }

    pub fn compile(
        &self,
        program: &Program,
        technique: &Technique,
        defines: &[String],
    ) -> Result<Compiled, String> {
        let compile = || {
            let (vertex, outputs) = self.stage(program, technique, Stage::Vertex, defines)?;
            let (pixel, inputs) = self.stage(program, technique, Stage::Pixel, defines)?;
            link(&outputs, &inputs)?;
            Ok(Compiled { vertex, pixel })
        };
        compile().map_err(|reason: String| {
            format!(
                "effect {:?}, technique {:?}: {reason}",
                program.effect, technique.name
            )
        })
    }

    fn stage(
        &self,
        program: &Program,
        technique: &Technique,
        stage: Stage,
        defines: &[String],
    ) -> Result<(Shader, BTreeMap<u32, String>), String> {
        let source = source::without_host_files(&program.hlsl(technique, stage, defines)?)?;
        let mut arguments: Vec<String> = [
            "-spirv",
            "-fspv-target-env=vulkan1.0",
            "-fvk-use-dx-layout",
            "-HV",
            "2016",
            "-E",
            "main",
            "-T",
            if stage == Stage::Vertex {
                "vs_6_0"
            } else {
                "ps_6_0"
            },
            "effect.hlsl",
        ]
        .map(str::to_owned)
        .into();
        let buffers: Vec<u32> = program
            .common
            .buffers
            .iter()
            .chain(&program.declarations(stage).buffers)
            .map(|buffer| buffer.index)
            .collect();
        let samplers: Vec<u32> = program
            .declarations(stage)
            .samplers
            .iter()
            .map(|sampler| sampler.index)
            .collect();
        if let Some(index) = buffers.iter().find(|&&index| index >= BUFFER_SLOTS) {
            return Err(format!(
                "{stage:?}: ConstantBuffer register b{index} exceeds D3D11's {BUFFER_SLOTS} slots"
            ));
        }
        if let Some(index) = samplers.iter().find(|&&index| index >= SAMPLER_SLOTS) {
            return Err(format!(
                "{stage:?}: sampler register s{index} exceeds D3D11's {SAMPLER_SLOTS} slots"
            ));
        }
        for &index in &buffers {
            arguments.extend([
                "-fvk-bind-register".to_owned(),
                format!("b{index}"),
                "0".to_owned(),
                binding(index, stage).to_string(),
                "0".to_owned(),
            ]);
        }
        for &index in &samplers {
            for (register, group) in [("t", "1"), ("s", "2")] {
                arguments.extend([
                    "-fvk-bind-register".to_owned(),
                    format!("{register}{index}"),
                    "0".to_owned(),
                    binding(index, stage).to_string(),
                    group.to_owned(),
                ]);
            }
        }
        let bytes = self
            .library
            .compile(&source, &arguments)
            .map_err(|messages| format!("{stage:?} DXC: {messages}"))?;
        let declared = |indices: &[u32]| -> Vec<u32> {
            indices.iter().map(|&index| binding(index, stage)).collect()
        };
        let shader = translate(&bytes, stage, &declared(&buffers), &declared(&samplers))?;
        let linkage = interface(&bytes, if stage == Stage::Vertex { 3 } else { 1 })?;
        Ok((shader, linkage))
    }
}

const MISSING_COMPILER: &str = "DXC not found: dxcompiler.dll is not beside the application";

fn link(outputs: &BTreeMap<u32, String>, inputs: &BTreeMap<u32, String>) -> Result<(), String> {
    for (location, semantic) in inputs {
        if outputs.get(location) != Some(semantic) {
            return Err(format!(
                "pixel input {semantic} at location {location} would read vertex output {}; \
                 only signatures in the vertex stage's declaration order link",
                outputs.get(location).map_or("nothing", String::as_str)
            ));
        }
    }
    Ok(())
}

fn interface(bytes: &[u8], storage: u32) -> Result<BTreeMap<u32, String>, String> {
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    let (mut names, mut locations, mut variables) = (BTreeMap::new(), BTreeMap::new(), Vec::new());
    let mut at = 5;
    while at < words.len() {
        let count = ((words[at] >> 16) as usize).max(1);
        let operands = words
            .get(at + 1..at + count)
            .ok_or("truncated SPIR-V instruction")?;
        match (words[at] & 0xFFFF, operands) {
            (5, [id, text @ ..]) => {
                let text: Vec<u8> = text.iter().flat_map(|word| word.to_le_bytes()).collect();
                let end = text
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(text.len());
                names.insert(*id, String::from_utf8_lossy(&text[..end]).into_owned());
            }
            (71, [id, 30, location, ..]) => {
                locations.insert(*id, *location);
            }
            (59, [_, id, class, ..]) if *class == storage => variables.push(*id),
            _ => {}
        }
        at += count;
    }
    let mut out = BTreeMap::new();
    for id in variables {
        let (Some(location), Some(name)) = (locations.get(&id), names.get(&id)) else {
            continue;
        };
        let semantic = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase();
        let indexed = semantic.ends_with(|c: char| c.is_ascii_digit());
        out.insert(*location, if indexed { semantic } else { semantic + "0" });
    }
    Ok(out)
}

#[cfg(test)]
struct Scratch(std::path::PathBuf);
#[cfg(test)]
impl Scratch {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("hoi4-shader-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).map_err(|e| format!("shader scratch: {e}"))?;
        Ok(Self(path))
    }
}
#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn translate(
    bytes: &[u8],
    stage: Stage,
    buffers: &[u32],
    samplers: &[u32],
) -> Result<Shader, String> {
    let options = naga::front::spv::Options {
        adjust_coordinate_space: false,
        ..Default::default()
    };
    let parse = |bytes: &[u8]| -> Result<_, String> {
        let module = naga::front::spv::parse_u8_slice(bytes, &options)
            .map_err(|e| format!("{stage:?} SPIR-V: {e}"))?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| format!("{stage:?} validation: {e:?}"))?;
        Ok((module, info))
    };
    let (module, info) = parse(bytes)?;
    let clock = clock::analyse(&module, &info);
    let numeric = numeric::d3d_intrinsics(bytes)?;
    let (module, info) = if numeric == bytes {
        (module, info)
    } else {
        parse(&numeric)?
    };
    let wgsl = naga::back::wgsl::write_string(
        &module,
        &info,
        naga::back::wgsl::WriterFlags::EXPLICIT_TYPES,
    )
    .map_err(|e| format!("{stage:?} WGSL: {e}"))?;
    let mut shader = Shader {
        wgsl,
        independent_draws: batching::independent(&module),
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock,
    };
    let mut read_ranges = reads::reflect(&module, &info);
    for (_, variable) in module.global_variables.iter() {
        let Some(resource) = &variable.binding else {
            continue;
        };
        let declared = if resource.group == 0 {
            buffers
        } else {
            samplers
        };
        if !declared.contains(&resource.binding) {
            return Err(format!(
                "{stage:?}: {:?} uses a register no PdxShader declaration binds ({resource:?})",
                variable.name
            ));
        }
        match (&module.types[variable.ty].inner, resource.group) {
            (naga::TypeInner::Struct { span, .. }, 0) => shader.buffers.push(Buffer {
                binding: resource.binding,
                size: *span,
                reads: read_ranges.remove(&resource.binding).unwrap_or_default(),
            }),
            (
                naga::TypeInner::Image {
                    dim,
                    arrayed: false,
                    class:
                        naga::ImageClass::Sampled {
                            kind: naga::ScalarKind::Float,
                            multi: false,
                        },
                },
                1,
            ) => {
                let kind = match dim {
                    naga::ImageDimension::D2 => TextureKind::D2,
                    naga::ImageDimension::Cube => TextureKind::Cube,
                    naga::ImageDimension::D3 => TextureKind::D3,
                    _ => return Err(format!("{stage:?}: unsupported texture dimension {dim:?}")),
                };
                shader.textures.push((resource.binding, kind));
            }
            (naga::TypeInner::Sampler { .. }, 2) => shader.samplers.push(resource.binding),
            _ => {
                return Err(format!(
                    "{stage:?}: unsupported resource {:?} at {resource:?}",
                    variable.name
                ));
            }
        }
    }
    for pair in &info.get_entry_point(0).sampling_set {
        let image = module.global_variables[pair.image]
            .binding
            .as_ref()
            .ok_or("sampled image has no physical binding")?;
        let sampler = module.global_variables[pair.sampler]
            .binding
            .as_ref()
            .ok_or("sampled sampler has no physical binding")?;
        shader.sampling_pairs.push((image.binding, sampler.binding));
    }
    shader.sampling_pairs.sort_unstable();
    shader.sampling_pairs.dedup();
    if stage == Stage::Vertex {
        for argument in &module.entry_points[0].function.arguments {
            reflect_input(
                &module,
                argument.ty,
                argument.binding.as_ref(),
                argument.name.as_deref().unwrap_or_default(),
                &mut shader.inputs,
            )?;
        }
        shader.inputs.sort_by_key(|input| input.location);
    }
    Ok(shader)
}

fn reflect_input(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
    binding: Option<&naga::Binding>,
    name: &str,
    out: &mut Vec<VertexInput>,
) -> Result<(), String> {
    if let Some(naga::Binding::Location { location, .. }) = binding {
        let (components, scalar) = match module.types[ty].inner {
            naga::TypeInner::Scalar(scalar) => (1, scalar),
            naga::TypeInner::Vector { size, scalar } => (size as u32, scalar),
            _ => {
                return Err(format!(
                    "vertex input {name:?} has an unsupported vertex type"
                ));
            }
        };
        let kind = match (scalar.kind, scalar.width) {
            (naga::ScalarKind::Float, 4) => ScalarKind::Float,
            (naga::ScalarKind::Sint, 4) => ScalarKind::Sint,
            (naga::ScalarKind::Uint, 4) => ScalarKind::Uint,
            _ => {
                return Err(format!(
                    "vertex input {name:?} has an unsupported vertex scalar type"
                ));
            }
        };
        out.push(VertexInput {
            kind,
            location: *location,
            semantic: name.rsplit('.').next().unwrap_or(name).to_owned(),
            components,
        });
    } else if let naga::TypeInner::Struct { members, .. } = &module.types[ty].inner {
        for member in members {
            reflect_input(
                module,
                member.ty,
                member.binding.as_ref(),
                member.name.as_deref().unwrap_or_default(),
                out,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
