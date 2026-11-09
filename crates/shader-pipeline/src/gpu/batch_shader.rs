use crate::{Compiled, Shader, TextureKind};
use naga::{Binding, BuiltIn, Handle, Module, TypeInner};
use std::collections::BTreeMap;

pub(super) const META_BINDING: u32 = super::CONSTANT_BINDINGS as u32;
pub(super) const META_WORDS: usize = 4 + super::TEXTURE_BINDINGS + super::CONSTANT_BINDINGS;

pub(super) struct Program {
    pub vertex: String,
    pub pixel: String,
    pub buffers: Vec<(u32, u32)>,
    pub input: u32,
    pub varying: u32,
    pub position: Option<super::batch_position::Plan>,
}

fn module(source: &str) -> Result<Module, String> {
    naga::front::wgsl::parse_str(source).map_err(|error| error.emit_to_string(source))
}

fn validate(module: &Module) -> Result<naga::valid::ModuleInfo, String> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    .map_err(|error| error.to_string())
}

fn ty(module: &Module, handle: Handle<naga::Type>) -> Result<String, String> {
    let value = &module.types[handle];
    if matches!(value.inner, TypeInner::Struct { .. })
        && let Some(name) = &value.name
    {
        return Ok(name.clone());
    }
    let scalar = |value: naga::Scalar| match value.kind {
        naga::ScalarKind::Float => Ok(format!("f{}", value.width * 8)),
        naga::ScalarKind::Sint => Ok(format!("i{}", value.width * 8)),
        naga::ScalarKind::Uint => Ok(format!("u{}", value.width * 8)),
        naga::ScalarKind::Bool => Ok("bool".into()),
        _ => Err("abstract scalar in a compiled interface".into()),
    };
    match &value.inner {
        TypeInner::Scalar(value) => scalar(*value),
        TypeInner::Vector {
            size,
            scalar: value,
        } => Ok(format!("vec{}<{}>", *size as u32, scalar(*value)?)),
        TypeInner::Matrix {
            columns,
            rows,
            scalar: value,
        } => Ok(format!(
            "mat{}x{}<{}>",
            *columns as u32,
            *rows as u32,
            scalar(*value)?
        )),
        _ => Err("unnamed composite in a compiled interface".into()),
    }
}

fn attribute(binding: Option<&Binding>) -> Result<String, String> {
    let Some(binding) = binding else {
        return Ok(String::new());
    };
    match binding {
        Binding::BuiltIn(builtin) => {
            let name = match builtin {
                BuiltIn::Position { .. } => "position",
                BuiltIn::InstanceIndex => "instance_index",
                BuiltIn::FrontFacing => "front_facing",
                BuiltIn::SampleIndex => "sample_index",
                BuiltIn::SampleMask => "sample_mask",
                BuiltIn::FragDepth => "frag_depth",
                _ => return Err(format!("batch interface retains {builtin:?}")),
            };
            let invariant = if matches!(builtin, BuiltIn::Position { invariant: true }) {
                "@invariant "
            } else {
                ""
            };
            Ok(format!("{invariant}@builtin({name}) "))
        }
        Binding::Location {
            location,
            interpolation,
            sampling,
            blend_src,
            ..
        } => {
            let mut out = format!("@location({location}) ");
            if let Some(source) = blend_src {
                out.push_str(&format!("@blend_src({source}) "));
            }
            if let Some(interpolation) = interpolation {
                let mode = match interpolation {
                    naga::Interpolation::Perspective => "perspective",
                    naga::Interpolation::Linear => "linear",
                    naga::Interpolation::Flat => "flat",
                    naga::Interpolation::PerVertex => {
                        return Err("per-vertex interpolation requires a native draw".into());
                    }
                };
                let sample = match sampling {
                    Some(naga::Sampling::Center) => ", center",
                    Some(naga::Sampling::Centroid) => ", centroid",
                    Some(naga::Sampling::Sample) => ", sample",
                    Some(naga::Sampling::First) => ", first",
                    Some(naga::Sampling::Either) => ", either",
                    None => "",
                };
                out.push_str(&format!("@interpolate({mode}{sample}) "));
            }
            Ok(out)
        }
    }
}

fn locations(module: &Module, handle: Handle<naga::Type>, binding: Option<&Binding>) -> u32 {
    match binding {
        Some(Binding::Location { location, .. }) => location + 1,
        Some(_) => 0,
        None => match &module.types[handle].inner {
            TypeInner::Struct { members, .. } => members
                .iter()
                .map(|member| locations(module, member.ty, member.binding.as_ref()))
                .max()
                .unwrap_or(0),
            _ => 0,
        },
    }
}

fn tokens(source: &str, replacements: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    while at < source.len() {
        let start = at;
        let first = source[at..].chars().next().unwrap();
        at += first.len_utf8();
        if first.is_alphanumeric() || first == '_' {
            while let Some(next) = source[at..].chars().next() {
                if !next.is_alphanumeric() && next != '_' {
                    break;
                }
                at += next.len_utf8();
            }
            let token = &source[start..at];
            out.push_str(replacements.get(token).map_or(token, String::as_str));
        } else {
            out.push(first);
        }
    }
    out
}

fn array_samples(source: &str, images: &BTreeMap<String, u32>) -> Result<String, String> {
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    while let Some(found) = source[at..].find("texture") {
        let start = at + found;
        out.push_str(&source[at..start]);
        let end = source[start..]
            .find(|value: char| !value.is_ascii_alphanumeric() && value != '_')
            .map_or(source.len(), |length| start + length);
        let name = &source[start..end];
        let insertion = match name {
            "textureSample" | "textureSampleBias" | "textureSampleLevel" | "textureSampleGrad" => {
                Some((0, 3))
            }
            "textureLoad" => Some((0, 2)),
            "textureGather" => Some((1, 4)),
            _ => None,
        };
        let Some((image, insertion)) = insertion else {
            out.push_str(name);
            at = end;
            continue;
        };
        let open = source[end..]
            .find(|value: char| !value.is_whitespace())
            .map(|value| value + end)
            .filter(|index| source.as_bytes()[*index] == b'(')
            .ok_or("malformed texture operation")?;
        let mut depth = 1;
        let mut close = open + 1;
        let mut args = Vec::new();
        let mut argument = close;
        while close < source.len() && depth != 0 {
            match source.as_bytes()[close] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b',' if depth == 1 => {
                    args.push(source[argument..close].trim().to_owned());
                    argument = close + 1;
                }
                _ => {}
            }
            if depth == 0 {
                args.push(source[argument..close].trim().to_owned());
            }
            close += 1;
        }
        let binding = args
            .get(image)
            .and_then(|image| images.get(image))
            .ok_or("texture passed through a helper retains its native draw")?;
        if insertion > args.len() {
            return Err("malformed texture arguments".into());
        }
        args.insert(
            insertion,
            format!("i32(replay_batch_meta[replay_batch_index].layers[{binding}])"),
        );
        out.push_str(name);
        out.push('(');
        out.push_str(&args.join(", "));
        out.push(')');
        at = close;
    }
    out.push_str(&source[at..]);
    Ok(out)
}

fn stage(
    shader: &Shader,
    vertex: bool,
    input: u32,
    varying: u32,
) -> Result<(String, Vec<(u32, u32)>), String> {
    if shader.wgsl.contains("replay_batch_") {
        return Err("reserved batch identifier is already used".into());
    }
    let mut parsed = module(&shader.wgsl)?;
    if parsed.entry_points.len() != 1 {
        return Err("batch needs one stage entry".into());
    }
    let mut entry = parsed.entry_points.remove(0);
    let argument_bindings: Vec<_> = entry
        .function
        .arguments
        .iter()
        .map(|argument| argument.binding.clone())
        .collect();
    for argument in &mut entry.function.arguments {
        argument.binding = None;
    }
    let result = entry
        .function
        .result
        .as_mut()
        .ok_or("batch stage has no result")?;
    let result_attribute = attribute(result.binding.as_ref())?;
    result.binding = None;
    entry.function.name = Some("replay_batch_native".into());
    parsed
        .functions
        .append(entry.function, naga::Span::UNDEFINED);
    for (_, variable) in parsed.global_variables.iter_mut() {
        if variable.space == naga::AddressSpace::Uniform {
            let binding = variable.binding.as_ref().ok_or("uniform without binding")?;
            if binding.group != 0 {
                return Err("uniform outside native register group".into());
            }
            variable.name = Some(format!("replay_batch_cb{}", binding.binding));
        }
    }
    let info = validate(&parsed)?;
    let mut source = naga::back::wgsl::write_string(
        &parsed,
        &info,
        naga::back::wgsl::WriterFlags::EXPLICIT_TYPES,
    )
    .map_err(|error| error.to_string())?;
    let written = module(&source)?;
    let native = &written
        .functions
        .iter()
        .last()
        .ok_or("generated batch function missing")?
        .1;
    let native_name = native
        .name
        .as_deref()
        .ok_or("generated function name missing")?;
    let mut inputs = Vec::new();
    let mut calls = Vec::new();
    let mut position = None;
    for (index, (argument, binding)) in native.arguments.iter().zip(&argument_bindings).enumerate()
    {
        let name = format!("replay_batch_arg{index}");
        inputs.push(format!(
            "{}{}: {}",
            attribute(binding.as_ref())?,
            name,
            ty(&written, argument.ty)?
        ));
        if matches!(binding, Some(Binding::BuiltIn(BuiltIn::Position { .. }))) {
            position = Some(name.clone());
        }
        if let TypeInner::Struct { members, .. } = &written.types[argument.ty].inner {
            for member in members {
                if matches!(
                    member.binding,
                    Some(Binding::BuiltIn(BuiltIn::Position { .. }))
                ) {
                    position = Some(format!(
                        "{name}.{}",
                        member.name.as_deref().ok_or("unnamed interface member")?
                    ));
                }
            }
        }
        calls.push(name);
    }
    let result = native
        .result
        .as_ref()
        .ok_or("generated batch result missing")?;
    let result_type = ty(&written, result.ty)?;
    let mut fields = Vec::new();
    let mut copies = Vec::new();
    if vertex {
        match &written.types[result.ty].inner {
            TypeInner::Struct { members, .. } => {
                for (index, member) in members.iter().enumerate() {
                    let field = format!("value{index}");
                    fields.push(format!(
                        "{}{}: {},",
                        attribute(member.binding.as_ref())?,
                        field,
                        ty(&written, member.ty)?
                    ));
                    copies.push(format!(
                        "out.{field} = value.{};",
                        member.name.as_deref().ok_or("unnamed interface member")?
                    ));
                }
            }
            _ => {
                fields.push(format!("{result_attribute}value: {result_type},"));
                copies.push("out.value = value;".into());
            }
        }
    }

    let mut layout = naga::proc::Layouter::default();
    layout
        .update(written.to_ctx())
        .map_err(|error| error.to_string())?;
    let mut replacements = BTreeMap::new();
    let mut declarations = String::from("var<private> replay_batch_index: u32;\n");
    let mut buffers = Vec::new();
    let mut images = BTreeMap::new();
    for (_, variable) in written.global_variables.iter() {
        let Some(binding) = &variable.binding else {
            continue;
        };
        if variable.space == naga::AddressSpace::Uniform {
            let name = variable.name.as_deref().ok_or("unnamed uniform")?;
            let prefix = format!("@group(0) @binding({})", binding.binding);
            let start = source
                .find(&prefix)
                .ok_or("generated uniform declaration missing")?;
            let end = start
                + source[start..]
                    .find(';')
                    .ok_or("unterminated uniform declaration")?
                + 1;
            source.replace_range(start..end, "");
            let rows = format!("{name}_rows");
            declarations.push_str(&format!(
                "@group(0) @binding({}) var<storage, read> {rows}: array<{}>;\n",
                binding.binding,
                ty(&written, variable.ty)?
            ));
            replacements.insert(
                name.into(),
                format!(
                    "{rows}[replay_batch_meta[replay_batch_index].rows[{}]]",
                    binding.binding
                ),
            );
            buffers.push((binding.binding, layout[variable.ty].to_stride()));
        } else if binding.group == 1 {
            if !matches!(
                written.types[variable.ty].inner,
                TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed: false,
                    ..
                }
            ) {
                return Err("batch textures must be two-dimensional".into());
            }
            images.insert(
                variable.name.clone().ok_or("unnamed image")?,
                binding.binding,
            );
        }
    }
    source = tokens(&source, &replacements);
    source = array_samples(&source, &images)?.replace("texture_2d<f32>", "texture_2d_array<f32>");
    declarations.push_str(&format!("struct ReplayBatchMeta {{ scissor: vec4<f32>, layers: array<u32, {}>, rows: array<u32, {}>, }}\n@group(0) @binding({META_BINDING}) var<storage, read> replay_batch_meta: array<ReplayBatchMeta>;\n", super::TEXTURE_BINDINGS, super::CONSTANT_BINDINGS));
    let wrapper = if vertex {
        inputs.push(format!("@location({input}) replay_batch_id: u32"));
        fields.push(format!("@location({varying}) @interpolate(flat) id: u32,"));
        format!(
            "struct ReplayBatchOutput {{ {} }}\n@vertex fn main({}) -> ReplayBatchOutput {{ replay_batch_index = replay_batch_id; let value = {native_name}({}); var out: ReplayBatchOutput; {} out.id = replay_batch_id; return out; }}\n",
            fields.join("\n"),
            inputs.join(", "),
            calls.join(", "),
            copies.join("\n")
        )
    } else {
        inputs.push(format!(
            "@location({varying}) @interpolate(flat) replay_batch_id: u32"
        ));
        let position = position.unwrap_or_else(|| {
            inputs.push("@builtin(position) replay_batch_position: vec4<f32>".into());
            "replay_batch_position".into()
        });
        format!(
            "@fragment fn main({}) -> {result_attribute}{result_type} {{ replay_batch_index = replay_batch_id; let value = {native_name}({}); let clip = replay_batch_meta[replay_batch_index].scissor; if ({position}.x < clip.x || {position}.y < clip.y || {position}.x >= clip.z || {position}.y >= clip.w) {{ discard; }} return value; }}\n",
            inputs.join(", "),
            calls.join(", ")
        )
    };
    let source =
        format!("diagnostic(off, derivative_uniformity);\n{source}\n{declarations}\n{wrapper}");
    validate(&module(&source)?)?;
    Ok((source, buffers))
}

impl Program {
    pub fn new(compiled: &Compiled) -> Result<Self, String> {
        if !compiled.vertex.independent_draws || !compiled.pixel.independent_draws {
            return Err("program retains native draw-local inputs or side effects".into());
        }
        if compiled
            .vertex
            .textures
            .iter()
            .chain(&compiled.pixel.textures)
            .any(|(_, kind)| *kind != TextureKind::D2)
        {
            return Err("program uses a non-2D texture".into());
        }
        let vertex = module(&compiled.vertex.wgsl)?;
        let result = vertex.entry_points[0]
            .function
            .result
            .as_ref()
            .ok_or("vertex result missing")?;
        let position = compiled
            .vertex
            .inputs
            .iter()
            .all(|input| input.kind == crate::ScalarKind::Float)
            .then(|| super::batch_position::Plan::new(&vertex))
            .flatten();
        let varying = locations(&vertex, result.ty, result.binding.as_ref());
        let input = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| input.location + 1)
            .max()
            .unwrap_or(0);
        let (vertex, mut buffers) = stage(&compiled.vertex, true, input, varying)?;
        let (pixel, pixel_buffers) = stage(&compiled.pixel, false, input, varying)?;
        buffers.extend(pixel_buffers);
        buffers.sort_unstable();
        Ok(Self {
            vertex,
            pixel,
            buffers,
            input,
            varying,
            position,
        })
    }
}

pub(super) fn strip_safe(source: &str) -> bool {
    let Ok(parsed) = module(source) else {
        return false;
    };
    let Some(result) = parsed
        .entry_points
        .first()
        .and_then(|entry| entry.function.result.as_ref())
    else {
        return false;
    };
    let flat = |binding: &Option<Binding>| {
        matches!(
            binding,
            Some(Binding::Location {
                interpolation: Some(naga::Interpolation::Flat),
                ..
            })
        )
    };
    !flat(&result.binding)
        && !matches!(&parsed.types[result.ty].inner,
        TypeInner::Struct { members, .. } if members.iter().any(|member| flat(&member.binding)))
}

pub(super) fn unsupported(
    compiled: &Compiled,
    pass: &super::Pass,
) -> Result<Option<String>, String> {
    let reason = if !compiled.vertex.independent_draws || !compiled.pixel.independent_draws {
        Some("draw-local inputs or shader side effects")
    } else if compiled
        .vertex
        .textures
        .iter()
        .chain(&compiled.pixel.textures)
        .any(|(_, kind)| *kind != TextureKind::D2)
    {
        Some("non-2D texture")
    } else if !matches!(
        pass.primitive.topology,
        wgpu::PrimitiveTopology::TriangleList | wgpu::PrimitiveTopology::TriangleStrip
    ) || pass.vertices.len() != 1
        || pass.vertices[0].step != wgpu::VertexStepMode::Vertex
    {
        Some("topology or vertex rate requires a native draw")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Ok(Some(reason.into()));
    }
    for (stage, shader) in [(true, &compiled.vertex), (false, &compiled.pixel)] {
        let parsed = module(&shader.wgsl)?;
        if shader.wgsl.contains("replay_batch_") {
            return Ok(Some("reserved batch identifier is already used".into()));
        }
        let entry = &parsed.entry_points[0].function;
        let Some(result) = &entry.result else {
            return Ok(Some(
                "stage without a result retains its native interface".into(),
            ));
        };
        for (ty, binding) in entry
            .arguments
            .iter()
            .map(|argument| (argument.ty, argument.binding.as_ref()))
            .chain(std::iter::once((result.ty, result.binding.as_ref())))
        {
            if let Err(reason) = attribute(binding) {
                return Ok(Some(reason));
            }
            if let TypeInner::Struct { members, .. } = &parsed.types[ty].inner {
                for member in members {
                    if let Err(reason) = attribute(member.binding.as_ref()) {
                        return Ok(Some(reason));
                    }
                }
            }
        }
        for (_, function) in parsed.functions.iter() {
            if function
                .arguments
                .iter()
                .any(|arg| matches!(parsed.types[arg.ty].inner, TypeInner::Image { .. }))
            {
                return Ok(Some(
                    "texture helper argument retains its native binding".into(),
                ));
            }
        }
        if stage && pass.primitive.topology == wgpu::PrimitiveTopology::TriangleStrip {
            let result = parsed.entry_points[0]
                .function
                .result
                .as_ref()
                .ok_or("vertex result missing")?;
            let flat = |binding: &Option<Binding>| {
                matches!(
                    binding,
                    Some(Binding::Location {
                        interpolation: Some(naga::Interpolation::Flat),
                        ..
                    })
                )
            };
            if flat(&result.binding)
                || matches!(&parsed.types[result.ty].inner, TypeInner::Struct { members, .. } if members.iter().any(|member| flat(&member.binding)))
            {
                return Ok(Some(
                    "strip with flat outputs retains native provoking vertices".into(),
                ));
            }
        }
    }
    Ok(None)
}
