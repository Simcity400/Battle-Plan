//! Executable PdxShader effects. Source assembly belongs to installed-content; DXC compiles HLSL
//! to SPIR-V, naga validates it and emits WGSL for the same wgpu device as the map.
//! No effect filename selects a substitute program.

use std::{collections::BTreeMap, path::PathBuf, process::Command};

use installed_content::pdxshader::program::{Program, Stage, Technique};

pub mod artwork;
pub mod cache;
pub mod clock;
pub mod gpu;
pub mod library;
mod reads;
mod sampling;
mod source;
pub mod stream;
pub mod vertices;

/// Native registers are separate namespaces. Vertex and pixel bindings are separate too.
/// Saturating: an authored index beyond D3D11's slots is refused at compile, never wrapped.
pub fn binding(index: u32, stage: Stage) -> u32 {
    index
        .saturating_mul(2)
        .saturating_add(u32::from(stage == Stage::Pixel))
}

/// D3D11 constant-buffer slots (b0-b13) and sampler slots (s0-s15); a PdxShader sampler binds
/// its texture and sampler at the same index.
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
    /// Conservative component byte reads, excluding unused fields and padding.
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
    pub buffers: Vec<Buffer>,
    /// Physical slots actually read, after compiler dead-code elimination.
    pub textures: Vec<(u32, TextureKind)>,
    pub samplers: Vec<u32>,
    /// Actual image/sampler pairs, including pairs whose register numbers differ.
    pub sampling_pairs: Vec<(u32, u32)>,
    pub inputs: Vec<VertexInput>,
    /// How the stage's output depends on its constant buffers, read from the program.
    pub clock: clock::Clock,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Compiled {
    pub vertex: Shader,
    pub pixel: Shader,
}

impl Compiled {
    /// Seconds after its restart past which the draw no longer changes on the clock alone, when
    /// constant buffer `register` holds the clock at byte `time` and the restart at `since` in
    /// both stages, as sprite constants do: the longer stage's [`clock::Clock::settles`];
    /// `None` while either keeps changing.
    pub fn settles(&self, register: u32, time: u32, since: u32) -> Option<f32> {
        let mut settles = 0.0_f32;
        for (stage, shader) in [(Stage::Vertex, &self.vertex), (Stage::Pixel, &self.pixel)] {
            let binding = binding(register, stage);
            settles = settles.max(shader.clock.settles((binding, time), (binding, since))?);
        }
        Some(settles)
    }
}

/// `dxc --version` of the official v1.9.2607 release that tools/install-shader-compiler.ps1
/// pins. The persistent cache ABI names it, so only this compiler may produce entries.
pub const DXC_VERSION: &str = "1.9.0.5402 (0d3ee6b55-dirty)";

/// The compiler path is configuration, never taken from shader content. The Windows SDK and
/// RenderDoc builds tested here omit SPIR-V; tools/install-shader-compiler.ps1 installs DXC.
pub struct Compiler {
    executable: PathBuf,
}

impl Compiler {
    /// `BATTLE_PLAN_SHADER_COMPILER`, else `shader-tools/dxc/dxc.exe` beside the running executable.
    pub fn discover() -> Result<Self, String> {
        if let Some(path) = std::env::var_os("BATTLE_PLAN_SHADER_COMPILER").or_else(|| std::env::var_os("HOI4_SHADER_COMPILER")) {
            return Self::at(PathBuf::from(path));
        }
        let path = std::env::current_exe()
            .map_err(|e| format!("DXC location: {e}"))?
            .with_file_name("shader-tools")
            .join("dxc/dxc.exe");
        Self::at(path)
    }

    /// Refuses a missing executable and any build other than the pinned [`DXC_VERSION`].
    pub fn at(executable: PathBuf) -> Result<Self, String> {
        if !executable.is_file() {
            return Err(format!("DXC not found at {}", executable.display()));
        }
        let mut command = Command::new(&executable);
        command.arg("--version");
        no_window(&mut command);
        let output = command
            .output()
            .map_err(|e| format!("launch DXC at {}: {e}", executable.display()))?;
        let version = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !version.contains(DXC_VERSION) {
            return Err(format!(
                "DXC at {} reports {:?}, not the pinned {DXC_VERSION}",
                executable.display(),
                version.trim()
            ));
        }
        Ok(Self { executable })
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

    /// The stage's program and its linkage semantics by location: vertex outputs, pixel inputs.
    fn stage(
        &self,
        program: &Program,
        technique: &Technique,
        stage: Stage,
        defines: &[String],
    ) -> Result<(Shader, BTreeMap<u32, String>), String> {
        let source = source::native_includes(&program.hlsl(technique, stage, defines)?);
        let scratch = Scratch::new()?;
        let input = scratch.0.join("effect.hlsl");
        let output = scratch.0.join("effect.spv");
        std::fs::write(&input, &source).map_err(|e| e.to_string())?;
        let mut command = Command::new(&self.executable);
        // No shell, include search path or source-controlled executable. HLSL 2016 matches
        // the legacy D3DCompile syntax more closely than DXC's 2021 short-circuit semantics.
        command
            .args([
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
                "-Fo",
            ])
            .arg(&output)
            .arg(&input)
            .current_dir(&scratch.0);
        // The engine binds only declared registers.
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
            command.args([
                "-fvk-bind-register",
                &format!("b{index}"),
                "0",
                &binding(index, stage).to_string(),
                "0",
            ]);
        }
        for &index in &samplers {
            for (register, group) in [("t", "1"), ("s", "2")] {
                command.args([
                    "-fvk-bind-register",
                    &format!("{register}{index}"),
                    "0",
                    &binding(index, stage).to_string(),
                    group,
                ]);
            }
        }
        no_window(&mut command);
        let result = command.output().map_err(|e| format!("launch DXC: {e}"))?;
        if !result.status.success() {
            return Err(format!(
                "{stage:?} DXC: {}{}",
                scrubbed(&result.stdout, &scratch.0),
                scrubbed(&result.stderr, &scratch.0)
            ));
        }
        let bytes = std::fs::read(output).map_err(|e| format!("DXC output: {e}"))?;
        let declared = |indices: &[u32]| -> Vec<u32> {
            indices.iter().map(|&index| binding(index, stage)).collect()
        };
        let shader = translate(&bytes, stage, &declared(&buffers), &declared(&samplers))?;
        let linkage = interface(&bytes, if stage == Stage::Vertex { 3 } else { 1 })?;
        Ok((shader, linkage))
    }
}

/// D3D11 links a vertex output to a pixel input by register, that is by declaration order, not
/// by semantic. The same order compiles to the same SPIR-V locations; any other signature
/// (reordered, or a subset that is not a prefix) is refused rather than guessed.
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

/// Stage-interface semantics by location for one SPIR-V storage class (1 input, 3 output),
/// from DXC's `in.var.<SEMANTIC>` and `out.var.<SEMANTIC>` names; `TEXCOORD` is `TEXCOORD0`.
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
            // OpName
            (5, [id, text @ ..]) => {
                let text: Vec<u8> = text.iter().flat_map(|word| word.to_le_bytes()).collect();
                let end = text
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(text.len());
                names.insert(*id, String::from_utf8_lossy(&text[..end]).into_owned());
            }
            // OpDecorate Location
            (71, [id, 30, location, ..]) => {
                locations.insert(*id, *location);
            }
            // OpVariable
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

fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("hoi4-shader-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).map_err(|e| format!("shader scratch: {e}"))?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `buffers` and `samplers` are the bindings of the registers the program declares. Any other
/// register is raw code's own, which the engine never binds (the draw would read whatever an
/// earlier one left there), so it is refused.
fn translate(
    bytes: &[u8],
    stage: Stage,
    buffers: &[u32],
    samplers: &[u32],
) -> Result<Shader, String> {
    // DXC leaves the authored DX clip coordinates unchanged without -fvk-invert-y; wgpu
    // consumes that convention. Do not apply naga's default Vulkan Y inversion a second time.
    let options = naga::front::spv::Options {
        adjust_coordinate_space: false,
        ..Default::default()
    };
    let module = naga::front::spv::parse_u8_slice(bytes, &options)
        .map_err(|e| format!("{stage:?} SPIR-V: {e}"))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("{stage:?} validation: {e:?}"))?;
    let wgsl = naga::back::wgsl::write_string(
        &module,
        &info,
        naga::back::wgsl::WriterFlags::EXPLICIT_TYPES,
    )
    .map_err(|e| format!("{stage:?} WGSL: {e}"))?;
    let mut shader = Shader {
        wgsl,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: clock::analyse(&module, &info),
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

/// Compiler output with the scratch folder it ran in, which differs every run, named as
/// `.`: the same refusal reads the same on every load, so a map compares it across loads.
fn scrubbed(text: &[u8], scratch: &std::path::Path) -> String {
    let shown = scratch.display().to_string();
    let mut text = String::from_utf8_lossy(text).into_owned();
    for form in [shown.clone(), shown.replace('\\', "/")] {
        text = text.replace(&form, ".");
    }
    text
}

#[cfg(test)]
pub(crate) mod tests;
