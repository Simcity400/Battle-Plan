//! PdxShader declarations and DX11 source assembly. Engine a729:
//! `142348F80` (file), `1423498C0` (stage), `1423497F0` (effect), `1423457F0`
//! (include postorder), `142411430` (source), `1423F71B0` (entry lookup).
//! Raw code is content, never a table of recognised effect filenames.

use std::collections::BTreeSet;

use super::{Address, effect_path};
use crate::ResolvedLayers;

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
    /// Declarations and native `@ifdef`/`@else`/`@endif` translated to preprocessor lines.
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
    /// The DX9 register; DX11 deliberately ignores it (`142411840`).
    pub legacy_register: u32,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub source: Source,
}

/// A lowered filter: `lower_filter@1424110F0` treats an authored `None` as point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Point,
    Linear,
    Anisotropic,
}

/// Defaults from `SGfxSamplerReader::read_declaration@142348D70`. The DX11 declaration only
/// distinguishes `Type = cube` (`142411AB0`); `3d` and `shadow` declare 2D textures.
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
    /// `lower_d3d11@1423FD550` always wraps W; D3D11's null sampler clamps it.
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

/// State after the native reader applies repeated keys and its defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State<T> {
    pub name: String,
    pub value: T,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub effect: String,
    /// SHA256 of the resolved root, ordered includes (including absent files) and backend header.
    pub identity: [u8; 32],
    pub dependencies: Vec<String>,
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
    includes: Vec<String>,
    program: Program,
}

/// Effect sources are artwork outside the current hash coverage. Diagnostics must retain
/// `Program::warnings`, just as consumers do for other uncovered visual assets.
pub fn read(layers: &ResolvedLayers, effect: &str) -> Result<Program, String> {
    let errors = std::cell::RefCell::new(Vec::new());
    let result = read_with(effect, &|path| match layers.read_uncovered(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            errors.borrow_mut().push(format!("{path}: {error}"));
            None
        }
    });
    let errors = errors.into_inner();
    if !errors.is_empty() {
        return Err(format!("effect {effect:?}: {}", errors.join("; ")));
    }
    let mut program = result?;
    let uncovered: Vec<_> = program
        .dependencies
        .iter()
        .filter(|path| !crate::content::covered(path))
        .cloned()
        .collect();
    if !uncovered.is_empty() {
        program.warnings.push(format!("Effect {effect:?} reads sources outside the content coverage policy; these bytes are not hash-verified: {}", uncovered.join(", ")));
    }
    Ok(program)
}

/// Resolves each complete file through the caller's recorded content stack. Missing includes
/// contribute nothing, as `1423457F0` does after its diagnostic. A missing root is the
/// empty lookup record (1423F5D60); a present root still requires its backend header.
pub fn read_with(effect: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Program, String> {
    use sha2::{Digest, Sha256};
    let digest = std::cell::RefCell::new(Sha256::new());
    let mut program = read_with_inner(effect, &|path| {
        let bytes = read(path);
        let mut hash = digest.borrow_mut();
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
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

fn read_with_inner(
    effect: &str,
    read: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<Program, String> {
    fn walk(
        path: &str,
        read: &dyn Fn(&str) -> Option<Vec<u8>>,
        seen: &mut BTreeSet<String>,
        out: &mut Vec<(String, File)>,
    ) -> Result<(), String> {
        // Native child-first, once-only traversal; authored nesting never consumes the Rust stack.
        enum Visit {
            Open(String),
            Close(String, Box<File>),
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
                    let file = parse(&path, &bytes)?;
                    let children: Vec<_> = file
                        .includes
                        .iter()
                        .rev()
                        .map(|name| Visit::Open(format!("gfx/FX/{name}")))
                        .collect();
                    stack.push(Visit::Close(path, Box::new(file)));
                    stack.extend(children);
                }
            }
        }
        Ok(())
    }
    let result = || {
        // The effect's path by its bytes, read here through a text path.
        let path = String::from_utf8_lossy(&effect_path(effect.as_bytes())).into_owned();
        let mut files = Vec::new();
        let Some(bytes) = read(&path) else {
            // LookupShaderEffectHandle@1423F5D60 returns 1435C1970 when no effect exists.
            // Keep this distinct from malformed present source or a failed filesystem read.
            return Ok(Program {
                effect: effect.to_owned(),
                dependencies: vec![path.clone()],
                warnings: vec![format!(
                    "effect {effect:?}: missing {path}; native null technique record (1423F5D60)"
                )],
                ..Program::default()
            });
        };
        let root = parse(&path, &bytes)?;
        let mut seen = BTreeSet::new();
        for include in &root.includes {
            walk(&format!("gfx/FX/{include}"), read, &mut seen, &mut files)?;
        }
        // States/effects belong to the root file. Only stage declarations are accumulated
        // from includes (`1423445D0`, `1423F6E10`).
        let mut program = Program {
            effect: effect.to_owned(),
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
                .push(format!("shader include {missing:?} not loaded (1423457F0)"));
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
        // Engine string at 142B5AB60, loaded by 142411430 through the VFS.
        let header = "gfx/FX/defines_hlsl_dx11.fxh";
        program.header = Source {
            path: header.into(),
            text: String::from_utf8(read(header).ok_or_else(|| format!("missing {header}"))?)
                .map_err(|e| format!("{header}: {e}"))?,
        };
        program.dependencies.push(header.into());
        Ok(program)
    };
    result().map_err(|reason: String| format!("effect {effect:?}: {reason}"))
}

impl Program {
    /// First case-insensitive match (`142344DF0`), not last declaration wins.
    pub fn technique(&self, name: &str) -> Option<&Technique> {
        self.techniques
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// The states a technique names: the root file's first `_stricmp` match, else the reader's
    /// defaults (`1423F7110`, `1423F7140`, `1423F7180`).
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

    /// Last common then pixel declaration owns each physical sampler (`142345450`).
    pub fn sampler(&self, index: u32) -> Option<&Sampler> {
        self.common
            .samplers
            .iter()
            .chain(&self.pixel.samplers)
            .rev()
            .find(|s| s.index == index)
    }

    /// The native DX11 source order (`142411430`): stage define, caller then technique defines,
    /// content header, stage samplers, common/stage buffers, common/stage structures,
    /// common/stage code, selected main. `1423F71B0` searches common entries before stage entries.
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
            // Strings 142B5AA08/142B5AA18, replaced only for cube: `3d` stays Texture2D.
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

// The shared native shader reader owns token walking, defaults, malformed keys and scanners.
// This adapter only emits the backend's source spelling and records source-file provenance.
fn parse(path: &str, bytes: &[u8]) -> Result<File, String> {
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
    let file = super::read_file(bytes);
    let [common, vertex, pixel] = file.stages;
    Ok(File {
        includes: file
            .includes
            .iter()
            .map(|s| text(path, s))
            .collect::<Result<_, _>>()?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_order_and_first_entry_follow_native() {
        let read = |path: &str| {
            Some(match path {
            "gfx/FX/test.shader" => "Includes = { a.fxh b.fxh } MainCode PS [[ float4 main() : PDX_COLOR { return 1; } ]] PixelShader = { MainCode PS [[ WRONG ]] } Effect Up { PixelShader = PS Defines = { EXTRA } }",
            "gfx/FX/a.fxh" => "Includes = { b.fxh } Code [[\nA\n]]",
            "gfx/FX/b.fxh" => "Includes = { a.fxh } Code [[\nB\n]]",
            "gfx/FX/defines_hlsl_dx11.fxh" => "HEADER", _ => return None,
        }.as_bytes().to_vec())
        };
        let program = read_with("gfx/FX/test.lua", &read).unwrap();
        assert_eq!(
            &program.dependencies[..3],
            &["gfx/FX/b.fxh", "gfx/FX/a.fxh", "gfx/FX/test.shader"]
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
        let program = read_with("gfx/FX/t.lua", &|path| {
            Some(match path {
                "gfx/FX/t.shader" => "PixelShader = { Samplers = { A = { Index = 0 Type = cube } B = { Index = 1 Type = 3d } C = { Index = 2 Type = shadow } } MainCode P [[ ]] } DepthStencilState Depth { DepthEnable = no } Effect Up { PixelShader = P DepthStencilState = DEPTH }",
                "gfx/FX/defines_hlsl_dx11.fxh" => "",
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
    fn declarations_preserve_code_and_native_buffer_layout() {
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
        let program = read_with("gfx/FX/mod.lua", &|path| {
            Some(match path {
                "gfx/FX/mod.shader" => "Includes = { child.fxh missing.fxh child.fxh } BlendState Alpha { BlendEnable = yes } Effect Up { PixelShader = P BlendState = Alpha } Effect up { PixelShader = Wrong }",
                "gfx/FX/child.fxh" => "BlendState Alpha { BlendEnable = no } Effect Up { PixelShader = Wrong } MainCode P [[ float4 main() : PDX_COLOR { return 1; } ]]",
                "gfx/FX/defines_hlsl_dx11.fxh" => "#define PDX_COLOR SV_TARGET\n",
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
        let error =
            read_with("mod/broken.lua", &|_| Some(b"Code [[ \x80 ]]".to_vec())).unwrap_err();
        assert!(error.contains("mod/broken.lua"));
        assert!(error.contains("non-UTF8 shader token/code"));
        let program = read_with("mod/empty.lua", &|path| {
            Some(if path.ends_with(".shader") {
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
    fn constant_array_counts_use_the_native_integer_scanner() {
        let file = parse("arrays", b"ConstantBuffer(0,0) { float4 octal[010];\nfloat4 hex[0x10];\nfloat4 no_count[MACRO];\nfloat4 zero[0];\n}\n").unwrap();
        assert_eq!(
            file.program.common.buffers[0].body,
            "float4 octal[8];\nfloat4 hex[16];\nfloat4 no_count;\nfloat4 zero;\n"
        );
    }
}
