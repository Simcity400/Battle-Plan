//! Shipped-shader compile & reflection resolver (OFF-3).
//!
//! Parses the Clausewitz effect files (`gfx/FX/*.shader` wrapping HLSL),
//! reconstructs the per-stage HLSL translation unit the engine feeds to
//! D3DCompile, compiles it, and compares the result against CAPTURED
//! bytecode at the reflection level (input signatures, cbuffer layouts,
//! bound resources) plus a straight hash/byte comparison.
//!
//! # Effect grammar (surveyed over all 45 vanilla .shader + 13 .fxh files)
//!
//! ```text
//! Includes = { "file.fxh" ... }              ; no nested Includes in vanilla
//! Code [[ raw hlsl ]]
//! ConstantBuffer( slot, dx9_base ) { ... };  ; -> cbuffer dx11_cb<slot> : register(b<slot>)
//! VertexStruct NAME { ... };                 ; -> struct NAME { ... };
//! VertexShader = { Samplers={...} MainCode N [[ hlsl ]] ... }
//! PixelShader  = { ... }
//! BlendState NAME { k = v ... }              ; also DepthStencilState / RasterizerState
//! Effect name { VertexShader = "N" PixelShader = "N" Defines = { "D" } ... }
//! ```
//!
//! # Engine emission laws (recovered from captured DXBC RDEF text)
//!
//! Captured reflection (live_PAN2 river PS, creator "Microsoft (R) HLSL
//! Shader Compiler 10.1", target ps_5_0) names resources
//! `<Sampler>_Texture` / `<Sampler>_Sampler` at the declared Index and
//! cbuffers `dx11_cb<slot>`; unused sampler declarations are dropped from
//! the bound list (river declares indices 0..15, captured binds all but
//! 6/7). The platform header `defines_hlsl_dx11.fxh` supplies the
//! `STextureSampler2D` wrapper (`sampler2D`) and `tex2D*` macros, so the
//! reconstructed declaration per sampler is:
//!
//! ```text
//! Texture2D    <N>_Texture : register(t<i>);
//! SamplerState <N>_Sampler : register(s<i>);
//! static STextureSampler2D <N> = CreateSampler2D( <N>_Texture, <N>_Sampler );
//! ```
//!
//! Byte-exact DXBC additionally depends on compile flags; the comparator
//! tries a small flag matrix and reports any exact hash match. Measured
//! result (2026-07-04): all 8 live map-shader blobs in live_PAN2 match
//! byte-for-byte at D3DCOMPILE_OPTIMIZATION_LEVEL3 with the SYSTEM
//! d3dcompiler_47.dll (10.0.26100.8655, sha256 2A8BC296C24647E0F5B6BB269E
//! 1CD2A0D77256A219BA9FAFC6688E968532FC3B; the game ships no compiler DLL).
//! Caveats from adversarial review (Codex, 2026-07-04): byte identity is
//! empirical per compiler build — other d3dcompiler_47 versions need not
//! reproduce it (reflection equality is the portable check); identity
//! proves the compiler saw the same EFFECTIVE program, not literal TU
//! text; and duplicate cbuffer slots such as pdxmesh.shader's train alias
//! are emitted once per reconstructed stage.
//!
//! Mod compatibility: every game-file read goes through [`GameFileResolver`]
//! (MOD_COMPATIBILITY.md) — mods that override `gfx/FX` are picked up by
//! construction.

use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

pub const SHADER_FX_DIR: &str = "gfx/FX";
pub const DX11_PLATFORM_HEADER: &str = "gfx/FX/defines_hlsl_dx11.fxh";

// ---------------------------------------------------------------------------
// Effect-file model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum EffectElement {
    /// Raw HLSL emitted verbatim (`Code [[ ... ]]`).
    Code(String),
    /// `ConstantBuffer( slot, dx9_base ) { body }`.
    ConstantBuffer {
        slot: u32,
        dx9_base: u32,
        body: String,
    },
    /// `VertexStruct NAME { body }`.
    VertexStruct { name: String, body: String },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectSampler {
    pub name: String,
    pub index: u32,
    pub mag_filter: Option<String>,
    pub min_filter: Option<String>,
    pub mip_filter: Option<String>,
    pub address_u: Option<String>,
    pub address_v: Option<String>,
    /// "Cube" / "Shadow" when present.
    pub sampler_type: Option<String>,
    pub mip_map_lod_bias: Option<String>,
    pub mip_map_min_lod: Option<String>,
    pub mip_map_max_lod: Option<String>,
    pub max_anisotropy: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectStage {
    pub samplers: Vec<EffectSampler>,
    /// MainCode name -> HLSL body.
    pub main_codes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectDef {
    pub name: String,
    pub vertex_shader: Option<String>,
    pub pixel_shader: Option<String>,
    pub defines: Vec<String>,
    pub blend_state: Option<String>,
    pub depth_stencil_state: Option<String>,
    pub rasterizer_state: Option<String>,
}

/// Which stage a top-level or stage-section element belongs to. Stage
/// sections may carry their own `Code` / `ConstantBuffer` elements
/// (fow.fxh, shadow.fxh, tiled_pointlights.fxh, tree.shader); those are
/// emitted only when compiling that stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementScope {
    Global,
    Vertex,
    Pixel,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectFile {
    pub includes: Vec<String>,
    pub elements: Vec<(ElementScope, EffectElement)>,
    pub vertex_stage: EffectStage,
    pub pixel_stage: EffectStage,
    pub blend_states: BTreeMap<String, Vec<(String, String)>>,
    pub depth_stencil_states: BTreeMap<String, Vec<(String, String)>>,
    pub rasterizer_states: BTreeMap<String, Vec<(String, String)>>,
    pub effects: Vec<EffectDef>,
}

impl EffectFile {
    pub fn effect(&self, name: &str) -> Option<&EffectDef> {
        self.effects.iter().find(|effect| effect.name == name)
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct EffectCursor<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> EffectCursor<'a> {
    fn skip_ws(&mut self) {
        loop {
            let rest = &self.text[self.pos..];
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                match trimmed.find('\n') {
                    Some(nl) => self.pos += nl,
                    None => self.pos = self.text.len(),
                }
                continue;
            }
            break;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.text[self.pos..].chars().next()
    }

    fn eat(&mut self, expected: char) -> Result<()> {
        match self.peek() {
            Some(c) if c == expected => {
                self.pos += expected.len_utf8();
                Ok(())
            }
            other => Err(format!(
                "effect parse: expected {expected:?} at byte {}, found {other:?}",
                self.pos
            )
            .into()),
        }
    }

    fn ident(&mut self) -> Result<&'a str> {
        self.skip_ws();
        let rest = &self.text[self.pos..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end == 0 {
            return Err(format!(
                "effect parse: expected identifier at byte {}: {:?}",
                self.pos,
                &rest[..rest.len().min(24)]
            )
            .into());
        }
        self.pos += end;
        Ok(&rest[..end])
    }

    /// A key's right-hand side: quoted string, identifier, or a bare
    /// number/flag token (e.g. `-0.5`, `4`, `yes`).
    fn value_token(&mut self) -> Result<String> {
        if self.peek() == Some('"') {
            return Ok(self.quoted()?.to_string());
        }
        self.skip_ws();
        let rest = &self.text[self.pos..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || "_-.|".contains(c)))
            .unwrap_or(rest.len());
        if end == 0 {
            return Err(format!(
                "effect parse: expected value token at byte {}: {:?}",
                self.pos,
                &rest[..rest.len().min(24)]
            )
            .into());
        }
        self.pos += end;
        Ok(rest[..end].to_string())
    }

    fn quoted(&mut self) -> Result<&'a str> {
        self.eat('"')?;
        let rest = &self.text[self.pos..];
        let end = rest
            .find('"')
            .ok_or_else(|| format!("effect parse: unterminated string at byte {}", self.pos))?;
        self.pos += end + 1;
        Ok(&rest[..end])
    }

    /// Raw block delimited by `[[` ... `]]`.
    fn raw_block(&mut self) -> Result<&'a str> {
        self.skip_ws();
        let rest = &self.text[self.pos..];
        let Some(stripped) = rest.strip_prefix("[[") else {
            return Err(format!("effect parse: expected [[ at byte {}", self.pos).into());
        };
        let end = stripped
            .find("]]")
            .ok_or_else(|| format!("effect parse: unterminated [[ block at byte {}", self.pos))?;
        self.pos += 2 + end + 2;
        Ok(&stripped[..end])
    }

    /// Brace-balanced body text (after the opening `{`), excluding the
    /// closing brace.
    fn braced_body(&mut self) -> Result<&'a str> {
        self.eat('{')?;
        let start = self.pos;
        let mut depth = 1usize;
        let bytes = self.text.as_bytes();
        while self.pos < bytes.len() {
            match bytes[self.pos] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        let body = &self.text[start..self.pos];
                        self.pos += 1;
                        return Ok(body);
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err(format!("effect parse: unbalanced braces from byte {start}").into())
    }

    fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.text.len()
    }
}

/// Parse one effect file. Fails closed on unknown top-level constructs
/// (naming them) — the grammar above covers all 45 vanilla .shader files.
pub fn parse_effect_file(text: &str) -> Result<EffectFile> {
    let mut cursor = EffectCursor { text, pos: 0 };
    let mut file = EffectFile::default();
    while !cursor.at_end() {
        let keyword = cursor.ident()?;
        match keyword {
            "Includes" => {
                cursor.eat('=')?;
                cursor.eat('{')?;
                while cursor.peek() != Some('}') {
                    file.includes.push(cursor.quoted()?.to_string());
                }
                cursor.eat('}')?;
            }
            "Code" => {
                file.elements.push((
                    ElementScope::Global,
                    EffectElement::Code(cursor.raw_block()?.to_string()),
                ));
            }
            "ConstantBuffer" => {
                let element = parse_constant_buffer(&mut cursor)?;
                file.elements.push((ElementScope::Global, element));
            }
            "VertexStruct" => {
                let name = cursor.ident()?.to_string();
                let body = cursor.braced_body()?.to_string();
                if cursor.peek() == Some(';') {
                    cursor.eat(';')?;
                }
                file.elements.push((
                    ElementScope::Global,
                    EffectElement::VertexStruct { name, body },
                ));
            }
            "VertexShader" | "PixelShader" => {
                cursor.eat('=')?;
                let scope = if keyword == "VertexShader" {
                    ElementScope::Vertex
                } else {
                    ElementScope::Pixel
                };
                let stage = parse_stage(&mut cursor, scope, &mut file.elements)?;
                if keyword == "VertexShader" {
                    merge_stage(&mut file.vertex_stage, stage);
                } else {
                    merge_stage(&mut file.pixel_stage, stage);
                }
            }
            "BlendState" | "DepthStencilState" | "RasterizerState" => {
                let name = cursor.ident()?.to_string();
                let pairs = parse_state_body(&mut cursor)?;
                let map = match keyword {
                    "BlendState" => &mut file.blend_states,
                    "DepthStencilState" => &mut file.depth_stencil_states,
                    _ => &mut file.rasterizer_states,
                };
                map.insert(name, pairs);
            }
            "Effect" => {
                let name = cursor.ident()?.to_string();
                file.effects.push(parse_effect_def(&mut cursor, name)?);
            }
            other => {
                return Err(format!(
                    "effect parse: unknown top-level construct {other:?} at byte {}; failing \
                     closed",
                    cursor.pos
                )
                .into())
            }
        }
    }
    Ok(file)
}

fn merge_stage(target: &mut EffectStage, source: EffectStage) {
    target.samplers.extend(source.samplers);
    target.main_codes.extend(source.main_codes);
}

fn parse_constant_buffer(cursor: &mut EffectCursor) -> Result<EffectElement> {
    cursor.eat('(')?;
    let slot: u32 = cursor
        .ident()?
        .parse()
        .map_err(|err| format!("effect parse: ConstantBuffer slot not a number: {err}"))?;
    cursor.eat(',')?;
    let dx9_base: u32 = cursor
        .ident()?
        .parse()
        .map_err(|err| format!("effect parse: ConstantBuffer base not a number: {err}"))?;
    cursor.eat(')')?;
    let body = cursor.braced_body()?.to_string();
    if cursor.peek() == Some(';') {
        cursor.eat(';')?;
    }
    Ok(EffectElement::ConstantBuffer {
        slot,
        dx9_base,
        body,
    })
}

fn parse_stage(
    cursor: &mut EffectCursor,
    scope: ElementScope,
    elements: &mut Vec<(ElementScope, EffectElement)>,
) -> Result<EffectStage> {
    let mut stage = EffectStage::default();
    cursor.eat('{')?;
    while cursor.peek() != Some('}') {
        let keyword = cursor.ident()?;
        match keyword {
            "Samplers" => {
                cursor.eat('=')?;
                cursor.eat('{')?;
                while cursor.peek() != Some('}') {
                    let name = cursor.ident()?.to_string();
                    cursor.eat('=')?;
                    stage.samplers.push(parse_sampler(cursor, name)?);
                }
                cursor.eat('}')?;
            }
            "MainCode" => {
                let name = cursor.ident()?.to_string();
                let body = cursor.raw_block()?.to_string();
                stage.main_codes.insert(name, body);
            }
            "Code" => {
                elements.push((scope, EffectElement::Code(cursor.raw_block()?.to_string())));
            }
            "ConstantBuffer" => {
                let element = parse_constant_buffer(cursor)?;
                elements.push((scope, element));
            }
            other => {
                return Err(format!(
                    "effect parse: unknown stage construct {other:?} at byte {}",
                    cursor.pos
                )
                .into())
            }
        }
    }
    cursor.eat('}')?;
    Ok(stage)
}

fn parse_sampler(cursor: &mut EffectCursor, name: String) -> Result<EffectSampler> {
    let mut sampler = EffectSampler {
        name,
        ..Default::default()
    };
    let mut have_index = false;
    cursor.eat('{')?;
    while cursor.peek() != Some('}') {
        let key = cursor.ident()?.to_string();
        cursor.eat('=')?;
        let value = cursor.value_token()?;
        match key.as_str() {
            "Index" => {
                sampler.index = value
                    .parse()
                    .map_err(|err| format!("sampler {}: bad Index: {err}", sampler.name))?;
                have_index = true;
            }
            "MagFilter" => sampler.mag_filter = Some(value),
            "MinFilter" => sampler.min_filter = Some(value),
            "MipFilter" => sampler.mip_filter = Some(value),
            "AddressU" => sampler.address_u = Some(value),
            "AddressV" => sampler.address_v = Some(value),
            "Type" => sampler.sampler_type = Some(value),
            "MipMapLodBias" => sampler.mip_map_lod_bias = Some(value),
            "MipMapMinLod" => sampler.mip_map_min_lod = Some(value),
            "MipMapMaxLod" => sampler.mip_map_max_lod = Some(value),
            "MaxAnisotropy" => sampler.max_anisotropy = Some(value),
            other => {
                return Err(format!(
                    "effect parse: unknown sampler key {other:?} in {}",
                    sampler.name
                )
                .into())
            }
        }
    }
    cursor.eat('}')?;
    if !have_index {
        return Err(format!("effect parse: sampler {} has no Index", sampler.name).into());
    }
    Ok(sampler)
}

fn parse_state_body(cursor: &mut EffectCursor) -> Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    cursor.eat('{')?;
    while cursor.peek() != Some('}') {
        let key = cursor.ident()?.to_string();
        cursor.eat('=')?;
        let value = cursor.value_token()?;
        pairs.push((key, value));
    }
    cursor.eat('}')?;
    Ok(pairs)
}

fn parse_effect_def(cursor: &mut EffectCursor, name: String) -> Result<EffectDef> {
    let mut effect = EffectDef {
        name,
        ..Default::default()
    };
    cursor.eat('{')?;
    while cursor.peek() != Some('}') {
        let key = cursor.ident()?.to_string();
        cursor.eat('=')?;
        match key.as_str() {
            "Defines" => {
                cursor.eat('{')?;
                while cursor.peek() != Some('}') {
                    effect.defines.push(cursor.quoted()?.to_string());
                }
                cursor.eat('}')?;
            }
            _ => {
                let value = cursor.quoted()?.to_string();
                match key.as_str() {
                    "VertexShader" => effect.vertex_shader = Some(value),
                    "PixelShader" => effect.pixel_shader = Some(value),
                    "BlendState" => effect.blend_state = Some(value),
                    "DepthStencilState" => effect.depth_stencil_state = Some(value),
                    "RasterizerState" => effect.rasterizer_state = Some(value),
                    other => {
                        return Err(format!(
                            "effect parse: unknown Effect key {other:?} in {}",
                            effect.name
                        )
                        .into())
                    }
                }
            }
        }
    }
    cursor.eat('}')?;
    Ok(effect)
}

// ---------------------------------------------------------------------------
// Resolver-routed loading + HLSL reconstruction
// ---------------------------------------------------------------------------

pub fn effect_file_from_resolver(
    resolver: &GameFileResolver,
    rel_path: &str,
) -> Result<EffectFile> {
    let path = resolver.resolve_required(rel_path, "effect file")?;
    let text =
        std::fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    parse_effect_file(&text).map_err(|err| format!("{rel_path}: {err}").into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderStage {
    Vertex,
    Pixel,
}

impl ShaderStage {
    pub fn profile(&self) -> &'static str {
        match self {
            ShaderStage::Vertex => "vs_5_0",
            ShaderStage::Pixel => "ps_5_0",
        }
    }
}

/// The six engine-injected macros (exactly the shader-cache constants the
/// capture layer already recovers from .scache scans): MAP_SIZE_X/Y,
/// MAP_POW2_X/Y, FOW_POW2_X/Y. Vanilla values measured from
/// live_PAN2 map_frame_bundle.json shader_cache_constants.
pub fn vanilla_engine_shader_defines() -> Vec<(String, String)> {
    vec![
        ("MAP_SIZE_X".to_string(), "5632.0f".to_string()),
        ("MAP_SIZE_Y".to_string(), "2048.0f".to_string()),
        ("MAP_POW2_X".to_string(), "1.0f".to_string()),
        ("MAP_POW2_Y".to_string(), "1.0f".to_string()),
        ("FOW_POW2_X".to_string(), "1.0f".to_string()),
        ("FOW_POW2_Y".to_string(), "1.0f".to_string()),
        // Graphics-settings feature flag: gradient borders were ON for the
        // captured sessions (gradient_border_apply early-outs behind
        // `#ifndef GRADIENT_BORDERS`; the captured map PS bytecode binds the
        // GB textures, so the engine injected it). pdxmesh.shader uses the
        // analogous PDX_GRADIENT_BORDERS.
        ("GRADIENT_BORDERS".to_string(), String::new()),
    ]
}

/// Reconstruct the full HLSL translation unit for one effect + stage.
pub fn build_stage_hlsl(
    resolver: &GameFileResolver,
    shader_rel_path: &str,
    effect_name: &str,
    stage: ShaderStage,
) -> Result<String> {
    build_stage_hlsl_with_defines(
        resolver,
        shader_rel_path,
        effect_name,
        stage,
        &vanilla_engine_shader_defines(),
    )
}

pub fn build_stage_hlsl_with_defines(
    resolver: &GameFileResolver,
    shader_rel_path: &str,
    effect_name: &str,
    stage: ShaderStage,
    engine_defines: &[(String, String)],
) -> Result<String> {
    let file = effect_file_from_resolver(resolver, shader_rel_path)?;
    let effect = file
        .effect(effect_name)
        .ok_or_else(|| format!("{shader_rel_path}: no Effect named {effect_name}"))?
        .clone();
    let (stage_section, entry) = match stage {
        ShaderStage::Vertex => (
            &file.vertex_stage,
            effect
                .vertex_shader
                .clone()
                .ok_or_else(|| format!("effect {effect_name} has no VertexShader"))?,
        ),
        ShaderStage::Pixel => (
            &file.pixel_stage,
            effect
                .pixel_shader
                .clone()
                .ok_or_else(|| format!("effect {effect_name} has no PixelShader"))?,
        ),
    };
    let main_code = stage_section.main_codes.get(&entry).ok_or_else(|| {
        format!("{shader_rel_path}: stage has no MainCode {entry:?} for effect {effect_name}")
    })?;

    let mut hlsl = String::new();
    for (name, value) in engine_defines {
        if value.is_empty() {
            let _ = writeln!(hlsl, "#define {name}");
        } else {
            let _ = writeln!(hlsl, "#define {name} {value}");
        }
    }
    for define in &effect.defines {
        let _ = writeln!(hlsl, "#define {define}");
    }
    let platform = resolver.resolve_required(DX11_PLATFORM_HEADER, "dx11 platform header")?;
    hlsl.push_str(
        &std::fs::read_to_string(&platform)
            .map_err(|err| format!("read {}: {err}", platform.display()))?,
    );
    hlsl.push('\n');

    // Two passes to match the engine's cbuffer declaration order (captured
    // RDEF lists dx11_cb0, dx11_cb1, dx11_cb2 in slot order even though the
    // stage-scoped tiled_pointlights cb2 comes from an earlier include):
    // GLOBAL-scope elements of every file first, then stage-scoped ones.
    let mut ordered_files = Vec::new();
    for include in &file.includes {
        let include_rel = format!("{SHADER_FX_DIR}/{include}");
        let include_file = effect_file_from_resolver(resolver, &include_rel)?;
        if !include_file.includes.is_empty() {
            return Err(format!(
                "{include_rel}: nested Includes are unsupported (none exist in vanilla)"
            )
            .into());
        }
        ordered_files.push(include_file);
    }
    ordered_files.push(file.clone());
    let stage_cbuffer_slots = stage_scoped_cbuffer_slots(&ordered_files, stage);
    let mut emitted_cbuffer_slots = BTreeSet::new();
    for pass_scope in [EmitPass::Global, EmitPass::Stage] {
        for emit_file in &ordered_files {
            emit_elements(
                &mut hlsl,
                &emit_file.elements,
                stage,
                pass_scope,
                &stage_cbuffer_slots,
                &mut emitted_cbuffer_slots,
            );
        }
    }
    for sampler in &stage_section.samplers {
        emit_sampler(&mut hlsl, sampler);
    }
    hlsl.push_str(&normalize_effect_hlsl_fragment(main_code));
    hlsl.push('\n');
    Ok(hlsl)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EmitPass {
    Global,
    Stage,
}

fn stage_scoped_cbuffer_slots(files: &[EffectFile], stage: ShaderStage) -> BTreeSet<u32> {
    let wanted = match stage {
        ShaderStage::Vertex => ElementScope::Vertex,
        ShaderStage::Pixel => ElementScope::Pixel,
    };
    let mut slots = BTreeSet::new();
    for file in files {
        for (scope, element) in &file.elements {
            if *scope == wanted {
                if let EffectElement::ConstantBuffer { slot, .. } = element {
                    slots.insert(*slot);
                }
            }
        }
    }
    slots
}

fn emit_elements(
    hlsl: &mut String,
    elements: &[(ElementScope, EffectElement)],
    stage: ShaderStage,
    pass: EmitPass,
    stage_cbuffer_slots: &BTreeSet<u32>,
    emitted_cbuffer_slots: &mut BTreeSet<u32>,
) {
    let wanted = match stage {
        ShaderStage::Vertex => ElementScope::Vertex,
        ShaderStage::Pixel => ElementScope::Pixel,
    };
    for (scope, element) in elements {
        let emit_now = match pass {
            EmitPass::Global => *scope == ElementScope::Global,
            EmitPass::Stage => *scope == wanted,
        };
        if !emit_now {
            continue;
        }
        match element {
            EffectElement::Code(code) => {
                hlsl.push_str(&normalize_effect_hlsl_fragment(code));
                hlsl.push('\n');
            }
            EffectElement::ConstantBuffer { slot, body, .. } => {
                if pass == EmitPass::Global && stage_cbuffer_slots.contains(slot) {
                    continue;
                }
                if !emitted_cbuffer_slots.insert(*slot) {
                    continue;
                }
                let body = normalize_effect_hlsl_fragment(body);
                let _ = writeln!(
                    hlsl,
                    "cbuffer dx11_cb{slot} : register(b{slot})\n{{{body}}};"
                );
            }
            EffectElement::VertexStruct { name, body } => {
                let body = normalize_effect_hlsl_fragment(body);
                let _ = writeln!(hlsl, "struct {name}\n{{{body}}};");
            }
        }
    }
}

fn normalize_effect_hlsl_fragment(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    for line in fragment.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("#//") {
            let leading = &line[..line.len() - trimmed.len()];
            out.push_str(leading);
            out.push_str("//");
            out.push_str(rest);
        } else if let Some(rest) = trimmed.strip_prefix('@') {
            let leading = &line[..line.len() - trimmed.len()];
            if rest.starts_with("//") {
                out.push_str(leading);
                out.push_str(rest);
            } else if is_clausewitz_preprocessor_directive(rest) {
                out.push('#');
                out.push_str(rest);
            } else {
                out.push_str(line);
            }
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn is_clausewitz_preprocessor_directive(rest: &str) -> bool {
    let directive = rest
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or_default();
    matches!(
        directive,
        "if" | "ifdef" | "ifndef" | "elif" | "else" | "endif" | "define" | "undef"
    )
}

fn emit_sampler(hlsl: &mut String, sampler: &EffectSampler) {
    let name = &sampler.name;
    let index = sampler.index;
    let is_cube = sampler.sampler_type.as_deref() == Some("Cube");
    let (texture_type, wrapper, ctor) = if is_cube {
        ("TextureCube", "STextureSamplerCube", "CreateSamplerCube")
    } else {
        ("Texture2D", "STextureSampler2D", "CreateSampler2D")
    };
    let _ = writeln!(hlsl, "{texture_type} {name}_Texture : register(t{index});");
    let _ = writeln!(hlsl, "SamplerState {name}_Sampler : register(s{index});");
    let _ = writeln!(
        hlsl,
        "static {wrapper} {name} = {ctor}( {name}_Texture, {name}_Sampler );"
    );
}

// ---------------------------------------------------------------------------
// D3DCompile
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub fn d3d_compile_stage(hlsl: &str, stage: ShaderStage, flags: u32) -> Result<Vec<u8>> {
    use winapi::um::d3dcommon::ID3DBlob;
    use winapi::um::d3dcompiler::D3DCompile;
    let entry = b"main\0";
    let target = format!("{}\0", stage.profile());
    let mut code: *mut ID3DBlob = std::ptr::null_mut();
    let mut errors: *mut ID3DBlob = std::ptr::null_mut();
    let hr = unsafe {
        D3DCompile(
            hlsl.as_ptr() as *const _,
            hlsl.len(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            entry.as_ptr() as *const _,
            target.as_ptr() as *const _,
            flags,
            0,
            &mut code,
            &mut errors,
        )
    };
    let error_text = if errors.is_null() {
        String::new()
    } else {
        unsafe {
            let text = std::slice::from_raw_parts(
                (*errors).GetBufferPointer() as *const u8,
                (*errors).GetBufferSize(),
            )
            .to_vec();
            (*errors).Release();
            String::from_utf8_lossy(&text).into_owned()
        }
    };
    if hr < 0 {
        return Err(format!(
            "D3DCompile({}) failed (0x{hr:08x}): {error_text}",
            stage.profile()
        )
        .into());
    }
    let bytes = unsafe {
        let out = std::slice::from_raw_parts(
            (*code).GetBufferPointer() as *const u8,
            (*code).GetBufferSize(),
        )
        .to_vec();
        (*code).Release();
        out
    };
    Ok(bytes)
}

/// The exact FXC optimization level the game engine compiled its shipped map
/// shaders with. OFF-3 established (and the shader-bytecode lane re-proved live,
/// 2026-07-08) that `OPTIMIZATION_LEVEL3` reproduces the captured engine DXBC
/// byte-for-byte while FXC's default level-1 (flags 0) does NOT. File-derivation
/// render paths (border, and — pending per-shader proof — underwater/buildings)
/// MUST use this to match the engine's own bytecode.
#[cfg(windows)]
pub const ENGINE_COMPILE_FLAGS: u32 = winapi::um::d3dcompiler::D3DCOMPILE_OPTIMIZATION_LEVEL3;

/// Compile-flag matrix tried when hunting a byte-exact match. FXC default
/// optimization is level 1 (flags 0).
#[cfg(windows)]
pub const COMPILE_FLAG_MATRIX: &[(&str, u32)] = &[
    ("default", 0),
    (
        "O3",
        winapi::um::d3dcompiler::D3DCOMPILE_OPTIMIZATION_LEVEL3,
    ),
    (
        "row_major",
        winapi::um::d3dcompiler::D3DCOMPILE_PACK_MATRIX_ROW_MAJOR,
    ),
    (
        "O3+row_major",
        winapi::um::d3dcompiler::D3DCOMPILE_OPTIMIZATION_LEVEL3
            | winapi::um::d3dcompiler::D3DCOMPILE_PACK_MATRIX_ROW_MAJOR,
    ),
];

// ---------------------------------------------------------------------------
// DXBC reflection parsing (RDEF / ISGN / OSGN)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DxbcResource {
    pub name: String,
    /// 0 cbuffer, 1 tbuffer, 2 texture, 3 sampler (D3D_SIT_*).
    pub resource_type: u32,
    pub bind_point: u32,
    pub bind_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DxbcCbufferVar {
    pub name: String,
    pub offset: u32,
    pub size: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DxbcCbuffer {
    pub name: String,
    pub size: u32,
    pub vars: Vec<DxbcCbufferVar>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DxbcSignatureParam {
    pub semantic_name: String,
    pub semantic_index: u32,
    pub register: u32,
    pub component_type: u32,
    pub mask: u8,
    pub read_write_mask: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DxbcReflection {
    /// Program type (0xffff pixel, 0xfffe vertex) and version nibble pair.
    pub target: u32,
    pub creator: String,
    pub resources: Vec<DxbcResource>,
    pub cbuffers: Vec<DxbcCbuffer>,
    pub inputs: Vec<DxbcSignatureParam>,
    pub outputs: Vec<DxbcSignatureParam>,
}

pub fn parse_dxbc_reflection(blob: &[u8]) -> Result<DxbcReflection> {
    if blob.get(..4) != Some(b"DXBC") {
        return Err("not a DXBC container".into());
    }
    let chunk_count = read_u32(blob, 28)? as usize;
    let mut reflection = DxbcReflection {
        target: 0,
        creator: String::new(),
        resources: Vec::new(),
        cbuffers: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    for chunk_index in 0..chunk_count {
        let offset = read_u32(blob, 32 + chunk_index * 4)? as usize;
        let tag = blob
            .get(offset..offset + 4)
            .ok_or("DXBC chunk offset out of range")?;
        let size = read_u32(blob, offset + 4)? as usize;
        let body = blob
            .get(offset + 8..offset + 8 + size)
            .ok_or("DXBC chunk size out of range")?;
        match tag {
            b"RDEF" => parse_rdef(body, &mut reflection)?,
            b"ISGN" => reflection.inputs = parse_signature(body)?,
            b"OSGN" => reflection.outputs = parse_signature(body)?,
            _ => {}
        }
    }
    Ok(reflection)
}

fn parse_rdef(body: &[u8], reflection: &mut DxbcReflection) -> Result<()> {
    let cb_count = read_u32(body, 0)? as usize;
    let cb_offset = read_u32(body, 4)? as usize;
    let rb_count = read_u32(body, 8)? as usize;
    let rb_offset = read_u32(body, 12)? as usize;
    let target = read_u32(body, 16)?;
    let creator_offset = read_u32(body, 24)? as usize;
    reflection.target = target;
    reflection.creator = read_cstr(body, creator_offset)?;
    // target = u8 minor | u8 major << 8 | u16 program type << 16
    // (e.g. 0xffff0500 = ps_5_0). SM5 variable records are 40 bytes (SM4: 24).
    let major = (target >> 8) & 0xff;
    let var_stride = if major >= 5 { 40 } else { 24 };
    for index in 0..rb_count {
        let base = rb_offset + index * 32;
        reflection.resources.push(DxbcResource {
            name: read_cstr(body, read_u32(body, base)? as usize)?,
            resource_type: read_u32(body, base + 4)?,
            bind_point: read_u32(body, base + 20)?,
            bind_count: read_u32(body, base + 24)?,
        });
    }
    for index in 0..cb_count {
        let base = cb_offset + index * 24;
        let name = read_cstr(body, read_u32(body, base)? as usize)?;
        let var_count = read_u32(body, base + 4)? as usize;
        let var_offset = read_u32(body, base + 8)? as usize;
        let size = read_u32(body, base + 12)?;
        let mut vars = Vec::with_capacity(var_count);
        for var_index in 0..var_count {
            let var_base = var_offset + var_index * var_stride;
            vars.push(DxbcCbufferVar {
                name: read_cstr(body, read_u32(body, var_base)? as usize)?,
                offset: read_u32(body, var_base + 4)?,
                size: read_u32(body, var_base + 8)?,
            });
        }
        reflection.cbuffers.push(DxbcCbuffer { name, size, vars });
    }
    Ok(())
}

fn parse_signature(body: &[u8]) -> Result<Vec<DxbcSignatureParam>> {
    let count = read_u32(body, 0)? as usize;
    let mut params = Vec::with_capacity(count);
    for index in 0..count {
        let base = 8 + index * 24;
        params.push(DxbcSignatureParam {
            semantic_name: read_cstr(body, read_u32(body, base)? as usize)?,
            semantic_index: read_u32(body, base + 4)?,
            component_type: read_u32(body, base + 12)?,
            register: read_u32(body, base + 16)?,
            mask: *body.get(base + 20).ok_or("signature mask out of range")?,
            read_write_mask: *body
                .get(base + 21)
                .ok_or("signature rw mask out of range")?,
        });
    }
    Ok(params)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("DXBC read out of range at {offset}"))?;
    Ok(u32::from_le_bytes(raw.try_into()?))
}

fn read_cstr(bytes: &[u8], offset: usize) -> Result<String> {
    let rest = bytes
        .get(offset..)
        .ok_or_else(|| format!("DXBC string offset {offset} out of range"))?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("DXBC string not NUL-terminated")?;
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ShaderCompileComparison {
    pub effect_label: String,
    pub stage: &'static str,
    pub captured_file: String,
    pub compiled_byte_len: usize,
    pub captured_byte_len: usize,
    pub inputs_match: bool,
    pub outputs_match: bool,
    pub cbuffers_match: bool,
    pub resources_match: bool,
    pub deltas: Vec<String>,
    /// Flag-matrix label with a byte-exact DXBC match, if any.
    pub exact_hash_flags: Option<String>,
    pub compiled_fnv1a64: String,
    pub captured_fnv1a64: String,
}

impl ShaderCompileComparison {
    pub fn reflection_matches(&self) -> bool {
        self.inputs_match && self.outputs_match && self.cbuffers_match && self.resources_match
    }

    pub fn print(&self) {
        println!(
            "{} [{}] vs {}: inputs {} outputs {} cbuffers {} resources {} | bytes {} vs {} | \
             exact_hash {}",
            self.effect_label,
            self.stage,
            self.captured_file,
            tick(self.inputs_match),
            tick(self.outputs_match),
            tick(self.cbuffers_match),
            tick(self.resources_match),
            self.compiled_byte_len,
            self.captured_byte_len,
            self.exact_hash_flags.as_deref().unwrap_or("no"),
        );
        for delta in &self.deltas {
            println!("    delta: {delta}");
        }
    }
}

fn tick(value: bool) -> &'static str {
    if value {
        "MATCH"
    } else {
        "DIFF"
    }
}

/// DXBC contains internal hashes over content; byte equality is the honest
/// comparator, reported alongside fnv1a64 (the capture manifest hash).
#[cfg(windows)]
pub fn compare_effect_stage_to_captured(
    resolver: &GameFileResolver,
    shader_rel_path: &str,
    effect_name: &str,
    stage: ShaderStage,
    captured_blob: &[u8],
    captured_file: &str,
) -> Result<ShaderCompileComparison> {
    let hlsl = build_stage_hlsl(resolver, shader_rel_path, effect_name, stage)?;
    let captured = parse_dxbc_reflection(captured_blob)
        .map_err(|err| format!("captured {captured_file}: {err}"))?;
    let mut exact = None;
    let mut compiled_default = None;
    for (label, flags) in COMPILE_FLAG_MATRIX {
        let compiled = d3d_compile_stage(&hlsl, stage, *flags)
            .map_err(|err| format!("{shader_rel_path}:{effect_name} [{label}]: {err}"))?;
        if compiled == captured_blob && exact.is_none() {
            exact = Some((*label).to_string());
        }
        if *label == "default" {
            compiled_default = Some(compiled);
        }
    }
    let compiled = compiled_default.expect("default flags present in matrix");
    let reflection = parse_dxbc_reflection(&compiled)
        .map_err(|err| format!("compiled {shader_rel_path}:{effect_name}: {err}"))?;

    let mut deltas = Vec::new();
    let inputs_match = compare_params("input", &reflection.inputs, &captured.inputs, &mut deltas);
    let outputs_match = compare_params(
        "output",
        &reflection.outputs,
        &captured.outputs,
        &mut deltas,
    );
    let resources_match = {
        let mut mine = reflection.resources.clone();
        let mut theirs = captured.resources.clone();
        mine.sort();
        theirs.sort();
        if mine != theirs {
            deltas.push(format!(
                "resources: compiled {:?} vs captured {:?}",
                mine.iter()
                    .map(|r| format!("{}@{}:{}", r.name, r.bind_point, r.resource_type))
                    .collect::<Vec<_>>(),
                theirs
                    .iter()
                    .map(|r| format!("{}@{}:{}", r.name, r.bind_point, r.resource_type))
                    .collect::<Vec<_>>()
            ));
            false
        } else {
            true
        }
    };
    let cbuffers_match = if reflection.cbuffers == captured.cbuffers {
        true
    } else {
        for (mine, theirs) in reflection.cbuffers.iter().zip(captured.cbuffers.iter()) {
            if mine != theirs {
                deltas.push(format!(
                    "cbuffer {}: compiled size {} vars {} vs captured {} size {} vars {}",
                    mine.name,
                    mine.size,
                    mine.vars.len(),
                    theirs.name,
                    theirs.size,
                    theirs.vars.len()
                ));
                for (a, b) in mine.vars.iter().zip(theirs.vars.iter()) {
                    if a != b {
                        deltas.push(format!(
                            "  var {} off {} size {} vs {} off {} size {}",
                            a.name, a.offset, a.size, b.name, b.offset, b.size
                        ));
                    }
                }
            }
        }
        if reflection.cbuffers.len() != captured.cbuffers.len() {
            deltas.push(format!(
                "cbuffer count: compiled {} vs captured {}",
                reflection.cbuffers.len(),
                captured.cbuffers.len()
            ));
        }
        false
    };

    Ok(ShaderCompileComparison {
        effect_label: format!("{shader_rel_path}:{effect_name}"),
        stage: match stage {
            ShaderStage::Vertex => "VS",
            ShaderStage::Pixel => "PS",
        },
        captured_file: captured_file.to_string(),
        compiled_byte_len: compiled.len(),
        captured_byte_len: captured_blob.len(),
        inputs_match,
        outputs_match,
        cbuffers_match,
        resources_match,
        deltas,
        exact_hash_flags: exact,
        compiled_fnv1a64: fnv1a64_bytes(&compiled),
        captured_fnv1a64: fnv1a64_bytes(captured_blob),
    })
}

fn compare_params(
    label: &str,
    mine: &[DxbcSignatureParam],
    theirs: &[DxbcSignatureParam],
    deltas: &mut Vec<String>,
) -> bool {
    if mine == theirs {
        return true;
    }
    deltas.push(format!(
        "{label} signature: compiled {:?} vs captured {:?}",
        mine.iter()
            .map(|p| format!(
                "{}{}@r{} mask{:x}",
                p.semantic_name, p.semantic_index, p.register, p.mask
            ))
            .collect::<Vec<_>>(),
        theirs
            .iter()
            .map(|p| format!(
                "{}{}@r{} mask{:x}",
                p.semantic_name, p.semantic_index, p.register, p.mask
            ))
            .collect::<Vec<_>>()
    ));
    false
}

// ---------------------------------------------------------------------------
// Parse-only gate over every vanilla effect file
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct EffectParseScan {
    pub shader_files: usize,
    pub fxh_files: usize,
    pub effects: usize,
    pub main_codes: usize,
    pub samplers: usize,
    pub failures: Vec<(String, String)>,
}

pub fn scan_effect_files_from_resolver(resolver: &GameFileResolver) -> Result<EffectParseScan> {
    let mut scan = EffectParseScan::default();
    for extension in ["shader", "fxh"] {
        for (rel, path) in resolver.resolve_dir_files(SHADER_FX_DIR, Some(extension))? {
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("read {}: {err}", path.display()))?;
            // The GLSL/HLSL platform headers are raw shader text, not effect
            // syntax; the engine inlines them, so parse-only skips them.
            if rel.contains("defines_glsl") || rel.contains("defines_hlsl") {
                continue;
            }
            match parse_effect_file(&text) {
                Ok(file) => {
                    if extension == "shader" {
                        scan.shader_files += 1;
                    } else {
                        scan.fxh_files += 1;
                    }
                    scan.effects += file.effects.len();
                    scan.main_codes +=
                        file.vertex_stage.main_codes.len() + file.pixel_stage.main_codes.len();
                    scan.samplers +=
                        file.vertex_stage.samplers.len() + file.pixel_stage.samplers.len();
                }
                Err(err) => scan.failures.push((rel, err.to_string())),
            }
        }
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::PathBuf;

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    const MINI_SHADER: &str = r#"
Includes = {
	"helper.fxh"
}

VertexStruct VS_INPUT
{
    float4 vPosition : POSITION;
};

VertexStruct VS_OUTPUT
{
    float4 vPosition : PDX_POSITION;
    float2 vUV : TEXCOORD0;
};

ConstantBuffer( 1, 32 )
{
	float4 vParams;
};

VertexShader =
{
	MainCode VertexShader
	[[
		VS_OUTPUT main( const VS_INPUT v )
		{
			VS_OUTPUT Out;
			Out.vPosition = mul( ViewProjectionMatrix, v.vPosition );
			Out.vUV = v.vPosition.xy;
			return Out;
		}
	]]
}

PixelShader =
{
	Samplers =
	{
		BaseTexture =
		{
			Index = 0
			MagFilter = "Linear"
			MinFilter = "Linear"
			MipFilter = "Linear"
			AddressU = "Wrap"
			AddressV = "Wrap"
		}
	}
	MainCode PixelShader
	[[
		float4 main( VS_OUTPUT Input ) : PDX_COLOR
		{
		#ifdef TINT_RED
			return float4( 1.0, 0.0, 0.0, 1.0 ) * vParams;
		#else
			return tex2D( BaseTexture, Input.vUV ) * vParams;
		#endif
		}
	]]
}

BlendState BlendState
{
	BlendEnable = yes
	SourceBlend = "src_alpha"
	DestBlend = "inv_src_alpha"
}

Effect mini
{
	VertexShader = "VertexShader"
	PixelShader = "PixelShader"
}

Effect mini_red
{
	VertexShader = "VertexShader"
	PixelShader = "PixelShader"
	Defines = { "TINT_RED" }
}
"#;

    const HELPER_FXH: &str = r#"
ConstantBuffer( 0, 0 )
{
	float4x4 ViewProjectionMatrix;
};

Code
[[
	float helper_one() { return 1.0; }
]]
"#;

    #[test]
    fn parses_mini_effect_file() {
        let file = parse_effect_file(MINI_SHADER).expect("parse mini shader");
        assert_eq!(file.includes, vec!["helper.fxh"]);
        assert_eq!(file.elements.len(), 3); // two VertexStructs + one ConstantBuffer
        assert_eq!(file.pixel_stage.samplers.len(), 1);
        assert_eq!(file.pixel_stage.samplers[0].index, 0);
        assert_eq!(file.effects.len(), 2);
        assert_eq!(file.effects[1].defines, vec!["TINT_RED"]);
        assert!(file.blend_states.contains_key("BlendState"));
        assert!(file.vertex_stage.main_codes.contains_key("VertexShader"));
    }

    #[test]
    fn unknown_top_level_construct_fails_closed() {
        let err = parse_effect_file("Mystery = { }").expect_err("unknown construct");
        assert!(err.to_string().contains("Mystery"), "{err}");
    }

    fn write_fx_fixture(root: &Path) {
        let fx = root.join("gfx").join("FX");
        fs::create_dir_all(&fx).expect("fx dir");
        fs::write(fx.join("mini.shader"), MINI_SHADER).expect("mini.shader");
        fs::write(fx.join("helper.fxh"), HELPER_FXH).expect("helper.fxh");
        fs::write(
            fx.join("defines_hlsl_dx11.fxh"),
            r#"#define PDX_POSITION SV_POSITION
#define PDX_COLOR SV_TARGET
struct STextureSampler2D { Texture2D _Texture; SamplerState _Sampler; };
STextureSampler2D CreateSampler2D( Texture2D Texture, SamplerState Sampler )
{ STextureSampler2D ret = { Texture, Sampler }; return ret; }
#define sampler2D STextureSampler2D
#define tex2D(samp,uv) samp._Texture.Sample(samp._Sampler, uv)
"#,
        )
        .expect("platform header");
    }

    #[test]
    fn build_stage_hlsl_reconstructs_translation_unit() {
        let root = temp_test_dir("shader_compile_build");
        let vanilla = root.join("vanilla");
        write_fx_fixture(&vanilla);
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let hlsl = build_stage_hlsl(&resolver, "gfx/FX/mini.shader", "mini", ShaderStage::Pixel)
            .expect("build PS hlsl");
        assert!(hlsl.contains("cbuffer dx11_cb0 : register(b0)"), "{hlsl}");
        assert!(hlsl.contains("cbuffer dx11_cb1 : register(b1)"), "{hlsl}");
        assert!(
            hlsl.contains("Texture2D BaseTexture_Texture : register(t0);"),
            "{hlsl}"
        );
        assert!(
            hlsl.contains("SamplerState BaseTexture_Sampler : register(s0);"),
            "{hlsl}"
        );
        assert!(hlsl.contains("struct VS_OUTPUT"), "{hlsl}");
        assert!(hlsl.contains("helper_one"), "{hlsl}");
        // Defines prepended for the effect that has them.
        let red = build_stage_hlsl(
            &resolver,
            "gfx/FX/mini.shader",
            "mini_red",
            ShaderStage::Pixel,
        )
        .expect("build red PS hlsl");
        // Engine-injected defines come first, then the effect's own defines.
        assert!(red.contains("#define TINT_RED\n"), "{red}");
        assert!(red.contains("#define MAP_SIZE_X 5632.0f\n"), "{red}");
        assert!(red.contains("#define GRADIENT_BORDERS\n"), "{red}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn normalizes_clausewitz_at_preprocessor_directives() {
        let hlsl = normalize_effect_hlsl_fragment(
            "float2 a;\n@ifdef PDX_MESH_UV1\nfloat2 b;\n@endif\n\t@// comment\n#// shipped comment\n",
        );
        assert!(hlsl.contains("#ifdef PDX_MESH_UV1"), "{hlsl}");
        assert!(hlsl.contains("#endif"), "{hlsl}");
        assert!(hlsl.contains("\t// comment"), "{hlsl}");
        assert!(hlsl.contains("// shipped comment"), "{hlsl}");
        assert!(!hlsl.contains("@ifdef"), "{hlsl}");
        assert!(!hlsl.contains("#//"), "{hlsl}");
    }

    #[test]
    fn stage_scoped_cbuffer_overrides_same_slot_global_for_stage() {
        let root = temp_test_dir("shader_compile_stage_cbuffer_override");
        let vanilla = root.join("vanilla");
        write_fx_fixture(&vanilla);
        let fx = vanilla.join("gfx").join("FX");
        fs::write(
            fx.join("stage_override.fxh"),
            r#"
PixelShader =
{
	ConstantBuffer( 1, 10 )
	{
		float4 StageValue;
	}
}
"#,
        )
        .expect("stage override include");
        fs::write(
            fx.join("stage_override.shader"),
            r#"
Includes = {
	"stage_override.fxh"
}

ConstantBuffer( 1, 0 )
{
	float4 GlobalValue;
};

VertexStruct VS_OUTPUT
{
	float4 vPosition : PDX_POSITION;
};

PixelShader =
{
	MainCode PixelShader
	[[
		float4 main( VS_OUTPUT Input ) : PDX_COLOR
		{
			return StageValue;
		}
	]]
}

Effect stage_override
{
	PixelShader = "PixelShader"
}
"#,
        )
        .expect("stage override shader");

        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let hlsl = build_stage_hlsl(
            &resolver,
            "gfx/FX/stage_override.shader",
            "stage_override",
            ShaderStage::Pixel,
        )
        .expect("build stage override hlsl");
        assert!(hlsl.contains("StageValue"), "{hlsl}");
        assert!(!hlsl.contains("GlobalValue"), "{hlsl}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn effect_file_from_resolver_uses_mod_override() {
        let root = temp_test_dir("shader_compile_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_fx_fixture(&vanilla);
        write_fx_fixture(&mod_root);
        let modded = MINI_SHADER.replace("Index = 0", "Index = 4");
        fs::write(mod_root.join("gfx/FX/mini.shader"), modded).expect("modded shader");

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let file =
            effect_file_from_resolver(&resolver, "gfx/FX/mini.shader").expect("modded effect");
        assert_eq!(file.pixel_stage.samplers[0].index, 4, "mod layer must win");
        let hlsl = build_stage_hlsl(&resolver, "gfx/FX/mini.shader", "mini", ShaderStage::Pixel)
            .expect("modded hlsl");
        assert!(hlsl.contains("register(t4)"), "{hlsl}");
        fs::remove_dir_all(root).ok();
    }

    #[cfg(windows)]
    #[test]
    fn mini_effect_compiles_and_reflects() {
        let root = temp_test_dir("shader_compile_compile");
        let vanilla = root.join("vanilla");
        write_fx_fixture(&vanilla);
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        for stage in [ShaderStage::Vertex, ShaderStage::Pixel] {
            let hlsl =
                build_stage_hlsl(&resolver, "gfx/FX/mini.shader", "mini", stage).expect("hlsl");
            let blob = d3d_compile_stage(&hlsl, stage, 0).expect("compile");
            let reflection = parse_dxbc_reflection(&blob).expect("reflect");
            assert!(reflection.creator.contains("HLSL Shader Compiler"));
            match stage {
                ShaderStage::Vertex => {
                    assert_eq!(reflection.target >> 16, 0xfffe);
                    assert_eq!(reflection.inputs.len(), 1);
                    assert_eq!(reflection.inputs[0].semantic_name, "POSITION");
                    assert!(reflection
                        .cbuffers
                        .iter()
                        .any(|cb| cb.name == "dx11_cb0" && cb.size == 64));
                }
                ShaderStage::Pixel => {
                    assert_eq!(reflection.target >> 16, 0xffff);
                    assert!(reflection
                        .resources
                        .iter()
                        .any(|r| r.name == "BaseTexture_Texture"
                            && r.resource_type == 2
                            && r.bind_point == 0));
                    assert!(reflection
                        .resources
                        .iter()
                        .any(|r| r.name == "BaseTexture_Sampler" && r.resource_type == 3));
                    assert!(reflection
                        .cbuffers
                        .iter()
                        .any(|cb| cb.name == "dx11_cb1" && cb.size == 16));
                }
            }
        }
        fs::remove_dir_all(root).ok();
    }

    fn vanilla_dir() -> Option<PathBuf> {
        let dir = PathBuf::from(VANILLA_GAME_DIR);
        if dir.is_dir() {
            Some(dir)
        } else {
            eprintln!("skipping vanilla shader gate: {VANILLA_GAME_DIR} is absent");
            None
        }
    }

    #[test]
    fn gate_all_vanilla_effect_files_parse() {
        let Some(game_dir) = vanilla_dir() else {
            return;
        };
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let scan = scan_effect_files_from_resolver(&resolver).expect("scan");
        assert_eq!(
            scan.failures,
            Vec::<(String, String)>::new(),
            "effect parse failures"
        );
        assert_eq!(scan.shader_files, 45, "vanilla ships 45 .shader files");
        println!(
            "gate: parsed {} .shader + {} effect-syntax .fxh files; {} effects, {} MainCode \
             blocks, {} samplers",
            scan.shader_files, scan.fxh_files, scan.effects, scan.main_codes, scan.samplers
        );
    }

    /// Compile + reflection comparison against a captured session. Skipped
    /// unless SHADER_CAPTURE_DIR points at a capture with the map bundles.
    #[cfg(windows)]
    #[test]
    fn gate_map_effects_reflection_matches_captured() {
        let Ok(capture) = std::env::var("SHADER_CAPTURE_DIR") else {
            eprintln!("skipping shader capture gate: SHADER_CAPTURE_DIR unset");
            return;
        };
        let Some(game_dir) = vanilla_dir() else {
            return;
        };
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let rows = compare_map_effects_to_capture(&resolver, Path::new(&capture))
            .expect("map effect comparison");
        assert!(!rows.is_empty(), "no captured map bundles found");
        let mut matched = 0usize;
        let mut exact = 0usize;
        for row in &rows {
            row.print();
            if row.reflection_matches() {
                matched += 1;
            }
            if row.exact_hash_flags.is_some() {
                exact += 1;
            }
        }
        // Every captured blob must be explained by at least one candidate
        // effect whose reflection fully matches.
        let mut captured_files: Vec<&str> =
            rows.iter().map(|row| row.captured_file.as_str()).collect();
        captured_files.sort();
        captured_files.dedup();
        for captured_file in captured_files {
            assert!(
                rows.iter()
                    .any(|row| row.captured_file == captured_file && row.reflection_matches()),
                "{captured_file}: no candidate effect matches its reflection"
            );
        }
        println!(
            "gate: {}/{} stage comparisons reflection-MATCH, {} byte-exact DXBC",
            matched,
            rows.len(),
            exact
        );
    }
}

// ---------------------------------------------------------------------------
// Captured-bundle driver (map-relevant effects)
// ---------------------------------------------------------------------------

/// Known mapping from captured bundle files to their effect files and the
/// candidate Effects to try (the runtime picks by gfx settings; comparing
/// all candidates identifies the live one).
pub const MAP_EFFECT_CANDIDATES: &[(&str, &str, &[&str])] = &[
    (
        "map_frame_bundle.json",
        "gfx/FX/pdxmap.shader",
        &["terrain", "terrain_low_gfx"],
    ),
    (
        "map_frame_water_pass_bundle.json",
        "gfx/FX/pdxwater.shader",
        &["water", "water_no_refractions", "water_low_gfx"],
    ),
    (
        "map_frame_rivers_pass_bundle.json",
        "gfx/FX/river.shader",
        &["river", "river_low_gfx"],
    ),
    (
        "map_frame_borders_pass_bundle.json",
        "gfx/FX/border.shader",
        &["border"],
    ),
    (
        "map_frame_trees_pass_bundle.json",
        "gfx/FX/tree.shader",
        &["tree", "tree_low_gfx", "treeunlit"],
    ),
];

/// Effects with no captured bundle yet (compile-only coverage): no session
/// has a borders_pass or restorescene bundle on disk.
pub const MAP_EFFECT_COMPILE_ONLY: &[(&str, &[&str])] = &[
    ("gfx/FX/restorescene.shader", &["Restore", "RestoreBloom"]),
    ("gfx/FX/border.shader", &["border"]),
    ("gfx/FX/pdxmesh.shader", &["PdxMeshStandard"]),
];

#[cfg(windows)]
pub fn compare_map_effects_to_capture(
    resolver: &GameFileResolver,
    capture_dir: &Path,
) -> Result<Vec<ShaderCompileComparison>> {
    let mut rows = Vec::new();
    for (bundle_file, shader_rel, candidates) in MAP_EFFECT_CANDIDATES {
        let bundle_path = capture_dir.join(bundle_file);
        if !bundle_path.is_file() {
            continue;
        }
        let bundle: MapFrameRuntimeBundle = serde_json::from_slice(
            &std::fs::read(&bundle_path)
                .map_err(|err| format!("read {}: {err}", bundle_path.display()))?,
        )
        .map_err(|err| format!("parse {}: {err}", bundle_path.display()))?;
        let draw = &bundle.active_draw;
        let ps_blob = std::fs::read(capture_dir.join(&draw.shader.bytecode_file))
            .map_err(|err| format!("read {}: {err}", draw.shader.bytecode_file))?;
        let vs = draw
            .vertex_shader
            .as_ref()
            .ok_or_else(|| format!("{bundle_file}: active draw has no vertex shader"))?;
        let vs_blob = std::fs::read(capture_dir.join(&vs.bytecode_file))
            .map_err(|err| format!("read {}: {err}", vs.bytecode_file))?;
        for effect_name in *candidates {
            rows.push(compare_effect_stage_to_captured(
                resolver,
                shader_rel,
                effect_name,
                ShaderStage::Pixel,
                &ps_blob,
                &draw.shader.bytecode_file,
            )?);
            rows.push(compare_effect_stage_to_captured(
                resolver,
                shader_rel,
                effect_name,
                ShaderStage::Vertex,
                &vs_blob,
                &vs.bytecode_file,
            )?);
        }
    }
    Ok(rows)
}
