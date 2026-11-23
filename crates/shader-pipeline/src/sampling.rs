
use content_reader_gui::pdxshader::{Address, program::{Filter, Sampler}};
use std::collections::BTreeMap;

fn resource_names(wgsl: &str, group: u32) -> Result<BTreeMap<String, u32>, String> {
    let mut names = BTreeMap::new();
    let mut remaining = wgsl;
    let prefix = format!("@group({group}) @binding(");
    while let Some(start) = remaining.find(&prefix) {
        remaining = &remaining[start + prefix.len()..];
        let Some(end) = remaining.find(')') else {
            break;
        };
        let binding: u32 = remaining[..end]
            .parse()
            .map_err(|e| format!("resource binding: {e}"))?;
        let Some(var) = remaining[end..].find("var ") else {
            break;
        };
        let tail = &remaining[end + var + 4..];
        let Some(colon) = tail.find(':') else {
            break;
        };
        names.insert(tail[..colon].trim().to_owned(), binding);
    }
    Ok(names)
}

fn address(mode: Address, i: &str, n: &str) -> String {
    match mode {
        Address::Wrap => format!("((({i}) % {n}) + {n}) % {n}"),
        Address::Mirror => format!("pdx_mirror({i}, {n})"),
        Address::Clamp => format!("clamp({i}, 0, {n} - 1)"),
        Address::MirrorOnce => format!("clamp(select({i}, -({i}) - 1, {i} < 0), 0, {n} - 1)"),
        Address::Border => i.to_owned(),
    }
}

const MIRROR: &str = "
fn pdx_mirror(i: i32, n: i32) -> i32 {
    let s = select(i, -i - 1, i < 0);
    return select(s % n, n - s % n - 1, ((s / n) & 1) == 1);
}
";

fn nonzero(offset: &str) -> bool {
    let body = ["vec2<i32>", "vec3<i32>", "vec2", "vec3"]
        .iter()
        .fold(offset.to_owned(), |text, ty| text.replace(ty, ""));
    body.chars().any(|c| c.is_ascii_alphabetic() && c != 'i')
        || body
            .split(|c: char| !c.is_ascii_digit())
            .any(|digits| !digits.trim_start_matches('0').is_empty())
}

fn gather(n: usize, fact: &Sampler) -> String {
    let (u, v) = (
        address(fact.u, "x[i]", "size.x"),
        address(fact.v, "y[i]", "size.y"),
    );
    let mut inside = Vec::new();
    for (mode, axis) in [(fact.u, 'x'), (fact.v, 'y')] {
        if mode == Address::Border {
            inside.push(format!("{axis}[i] >= 0 && {axis}[i] < size.{axis}"));
        }
    }
    let inside = if inside.is_empty() {
        "true".to_owned()
    } else {
        inside.join(" && ")
    };
    format!(
        "
fn pdx_gather{n}(t: texture_2d<f32>, uv: vec2<f32>, offset: vec2<i32>, c: u32) -> vec4<f32> {{
    let size = vec2<i32>(textureDimensions(t, 0));
    let base = vec2<i32>(floor(uv * vec2<f32>(size) - 0.5)) + offset;
    var x = array<i32, 4>(base.x, base.x + 1, base.x + 1, base.x);
    var y = array<i32, 4>(base.y + 1, base.y + 1, base.y, base.y);
    var out = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {{
        if ({inside}) {{
            out[i] = textureLoad(t, vec2<i32>({u}, {v}), 0)[c];
        }}
    }}
    return out;
}}
"
    )
}

fn sample(n: usize, fact: &Sampler, dimension: super::TextureKind, kind: &str) -> String {
    let volume = dimension == super::TextureKind::D3;
    let (texture, vector, ivector) = if volume {
        ("texture_3d<f32>", "vec3<f32>", "vec3<i32>")
    } else {
        ("texture_2d<f32>", "vec2<f32>", "vec2<i32>")
    };
    let axes: &[(Address, char)] = if volume {
        &[(fact.u, 'x'), (fact.v, 'y'), (fact.w, 'z')]
    } else {
        &[(fact.u, 'x'), (fact.v, 'y')]
    };
    let addressed: Vec<String> = axes
        .iter()
        .map(|&(mode, axis)| address(mode, &format!("i.{axis}"), &format!("size.{axis}")))
        .collect();
    let border: Vec<String> = axes
        .iter()
        .filter(|(mode, _)| *mode == Address::Border)
        .map(|(_, axis)| format!("i.{axis} < 0 || i.{axis} >= size.{axis}"))
        .collect();
    let border = if border.is_empty() {
        String::new()
    } else {
        format!(
            "\n    if ({}) {{\n        return vec4<f32>(0.0);\n    }}",
            border.join(" || ")
        )
    };
    let corners: &[[i32; 3]] = if volume {
        &[
            [0, 0, 0],
            [1, 0, 0],
            [0, 1, 0],
            [1, 1, 0],
            [0, 0, 1],
            [1, 0, 1],
            [0, 1, 1],
            [1, 1, 1],
        ]
    } else {
        &[[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0]]
    };
    let taps: Vec<String> = corners
        .iter()
        .map(|c| {
            let offset = if volume {
                format!("vec3<i32>({}, {}, {})", c[0], c[1], c[2])
            } else {
                format!("vec2<i32>({}, {})", c[0], c[1])
            };
            let weight = ["x", "y", "z"][..axes.len()]
                .iter()
                .enumerate()
                .map(|(k, axis)| {
                    if c[k] == 1 {
                        format!("f.{axis}")
                    } else {
                        format!("(1.0 - f.{axis})")
                    }
                })
                .collect::<Vec<_>>()
                .join(" * ");
            format!("pdx_texel{n}(t, level, size, base + {offset}) * {weight}")
        })
        .collect();
    let computed = if kind == "textureSampleLevel" {
        "lod".to_owned()
    } else {
        "log2(max(length(dx * size), length(dy * size))) + lod".to_owned()
    };
    let linear = |filter| matches!(filter, Filter::Linear);
    let body = if fact.mip == Filter::Linear {
        format!(
            "
    let base = clamp(i32(floor(clamped)), 0, levels - 1);
    let next = min(base + 1, levels - 1);
    let weight = select(0.0, fract(clamped), clamped > 0.0 && next > base);
    let low = pdx_filter{n}(t, uv, offset, base, linear);
    let high = pdx_filter{n}(t, uv, offset, next, linear);
    return low * (1.0 - weight) + high * weight;"
        )
    } else {
        format!(
            "
    let level = clamp(i32(floor(clamped + 0.5)), 0, levels - 1);
    return pdx_filter{n}(t, uv, offset, level, linear);"
        )
    };
    format!(
        "
fn pdx_texel{n}(t: {texture}, level: i32, size: {ivector}, i: {ivector}) -> vec4<f32> {{{border}
    return textureLoad(t, {ivector}({addressed}), level);
}}

fn pdx_filter{n}(t: {texture}, uv: {vector}, offset: {vector}, level: i32, linear: bool) -> vec4<f32> {{
    let size = {ivector}(textureDimensions(t, level));
    let p = uv * {vector}(size) + offset;
    if (!linear) {{
        return pdx_texel{n}(t, level, size, {ivector}(floor(p)));
    }}
    let q = p - 0.5;
    let base = {ivector}(floor(q));
    let f = q - floor(q);
    return {taps};
}}

fn pdx_sample{n}(t: {texture}, uv: {vector}, offset: {vector}, dx: {vector}, dy: {vector}, lod: f32) -> vec4<f32> {{
    let levels = i32(textureNumLevels(t));
    let size = {vector}(textureDimensions(t, 0));
    let clamped = max({min:?}, min({max:?}, {computed} + {bias:?}));
    // 7.18.11: Magnifying = (clampedLOD <= 0).
    let linear = select({min_linear}, {mag_linear}, clamped <= 0.0);{body}
}}
",
        addressed = addressed.join(", "),
        taps = taps.join("\n        + "),
        min = fact.min_lod as f32,
        max = fact.max_lod as f32,
        bias = fact.lod_bias,
        min_linear = linear(fact.min),
        mag_linear = linear(fact.mag),
    )
}

pub fn lower(
    wgsl: &str,
    samplers: &BTreeMap<u32, Sampler>,
    textures: &[(u32, super::TextureKind)],
    unfilterable: u32,
) -> Result<String, String> {
    let names = resource_names(wgsl, 2)?;
    let texture_names = resource_names(wgsl, 1)?;
    let resolve = |texture: &str, sampler: &str| {
        let slot = names
            .get(sampler.trim())
            .ok_or_else(|| format!("sampling uses unresolved sampler {sampler}"))?;
        let fact = samplers
            .get(slot)
            .ok_or_else(|| format!("sampler slot {slot} retains an unbound native state"))?;
        let binding = texture_names
            .get(texture.trim())
            .ok_or_else(|| format!("sampling uses unresolved texture {texture}"))?;
        let dimension = textures
            .iter()
            .find(|(own, _)| own == binding)
            .ok_or_else(|| format!("texture binding {binding} has no reflected dimension"))?
            .1;
        Ok::<_, String>((fact, dimension, unfilterable & (1 << binding) != 0))
    };
    let mut helpers = String::new();
    let mut mirror_helper = false;
    let mut out = String::new();
    let mut cursor = 0;
    let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    while let Some(relative) = wgsl[cursor..].find("texture") {
        let start = cursor + relative;
        out.push_str(&wgsl[cursor..start]);
        let name_end = wgsl[start..]
            .bytes()
            .position(|byte| !identifier(byte))
            .map_or(wgsl.len(), |length| start + length);
        let name = &wgsl[start..name_end];
        let paren = name_end + wgsl[name_end..].len() - wgsl[name_end..].trim_start().len();
        let bounded = start == 0 || !identifier(wgsl.as_bytes()[start - 1]);
        let sampling = matches!(
            name,
            "textureSample" | "textureSampleBias" | "textureSampleGrad" | "textureSampleLevel"
        );
        if !bounded || !(sampling || name == "textureGather") || !wgsl[paren..].starts_with('(') {
            out.push_str(name);
            cursor = name_end;
            continue;
        }
        let (end, mut args) = arguments(wgsl, paren)?;
        cursor = end;
        if name == "textureGather" {
            let (fact, dimension, manual) = resolve(&args[1], &args[2])?;
            let mirrored = fact.u == Address::MirrorOnce || fact.v == Address::MirrorOnce;
            if manual && dimension != super::TextureKind::D2 {
                return Err(format!(
                    "unfilterable {dimension:?} gather through sampler {:?} is unsupported",
                    fact.name
                ));
            }
            if dimension != super::TextureKind::D2 || !(mirrored || manual) {
                out.push_str(&wgsl[start..end]);
                continue;
            }
            let n = helpers.len();
            helpers.push_str(&gather(n, fact));
            mirror_helper |= fact.u == Address::Mirror || fact.v == Address::Mirror;
            let offset = args.get(4).cloned().unwrap_or("vec2<i32>(0)".into());
            out.push_str(&format!(
                "pdx_gather{n}({}, {}, {offset}, u32({}))",
                args[1], args[3], args[0]
            ));
            continue;
        }
        let (fact, dimension, manual) = resolve(&args[0], &args[1])?;
        let mirror_u = dimension != super::TextureKind::Cube && fact.u == Address::MirrorOnce;
        let mirror_v = dimension != super::TextureKind::Cube && fact.v == Address::MirrorOnce;
        let mirror_w = dimension == super::TextureKind::D3 && fact.w == Address::MirrorOnce;
        let mirrored = mirror_u || mirror_v || mirror_w;
        let uv = args[2].clone();
        let offset = match name {
            "textureSample" => 3,
            "textureSampleGrad" => 5,
            _ => 4,
        };
        if manual
            && (dimension == super::TextureKind::Cube
                || [fact.mag, fact.min, fact.mip].contains(&Filter::Anisotropic))
        {
            return Err(format!(
                "unfilterable {dimension:?} sample through sampler {:?} with {:?}/{:?}/{:?} filters is unsupported",
                fact.name, fact.mag, fact.min, fact.mip
            ));
        }
        if mirrored && [fact.mag, fact.min, fact.mip].contains(&Filter::Anisotropic) {
            return Err(format!(
                "anisotropic sample through MIRROR_ONCE sampler {:?}: wgpu has no MIRROR_ONCE \
                 address mode to mirror the footprint tap by tap",
                fact.name
            ));
        }
        if manual || mirrored && args.get(offset).is_some_and(|o| nonzero(o)) {
            let n = helpers.len();
            helpers.push_str(&sample(n, fact, dimension, name));
            mirror_helper |= [fact.u, fact.v, fact.w].contains(&Address::Mirror);
            let vector = if dimension == super::TextureKind::D3 {
                "vec3<f32>"
            } else {
                "vec2<f32>"
            };
            let (dx, dy, lod) = match name {
                "textureSampleGrad" => (args[3].clone(), args[4].clone(), "0.0".to_owned()),
                "textureSampleLevel" => (
                    "vec3<f32>(0.0)".into(),
                    "vec3<f32>(0.0)".into(),
                    args[3].clone(),
                ),
                _ => (
                    format!("dpdxCoarse({uv})"),
                    format!("dpdyCoarse({uv})"),
                    if name == "textureSampleBias" {
                        args[3].clone()
                    } else {
                        "0.0".into()
                    },
                ),
            };
            let zero = |d: String| d.replace("vec3<f32>(0.0)", &format!("{vector}(0.0)"));
            out.push_str(&format!(
                "pdx_sample{n}({}, {uv}, {vector}({}), {}, {}, {lod})",
                args[0],
                args.get(offset).map(String::as_str).unwrap_or("0"),
                zero(dx),
                zero(dy)
            ));
            continue;
        }
        if mirrored {
            let fold = |mirror: bool, axis: char| {
                if mirror {
                    format!("abs(({uv}).{axis})")
                } else {
                    format!("({uv}).{axis}")
                }
            };
            let (x, y) = (fold(mirror_u, 'x'), fold(mirror_v, 'y'));
            args[2] = if dimension == super::TextureKind::D3 {
                format!("vec3<f32>({x}, {y}, {})", fold(mirror_w, 'z'))
            } else {
                format!("vec2<f32>({x}, {y})")
            };
        }
        let mut function = name;
        if name == "textureSample" && fact.lod_bias != 0.0 && !mirrored {
            function = "textureSampleBias";
            args.insert(3, format!("{:?}", fact.lod_bias));
        } else if name == "textureSampleBias" && !mirrored {
            args[3] = format!("({}) + {:?}", args[3], fact.lod_bias);
        } else if name == "textureSampleLevel" && fact.lod_bias != 0.0 {
            args[3] = format!("({}) + {:?}", args[3], fact.lod_bias);
        } else if name == "textureSampleGrad" {
            let scale = 2_f32.powf(fact.lod_bias);
            args[3] = format!("({}) * {scale:?}", args[3]);
            args[4] = format!("({}) * {scale:?}", args[4]);
        } else if name != "textureSampleLevel" && mirrored {
            function = "textureSampleGrad";
            let bias = if name == "textureSampleBias" {
                args.remove(3)
            } else {
                "0.0".into()
            };
            args.insert(
                3,
                format!("dpdxCoarse({uv}) * exp2(({bias}) + {:?})", fact.lod_bias),
            );
            args.insert(
                4,
                format!("dpdyCoarse({uv}) * exp2(({bias}) + {:?})", fact.lod_bias),
            );
        }
        out.push_str(function);
        out.push('(');
        out.push_str(&args.join(", "));
        out.push(')');
    }
    out.push_str(&wgsl[cursor..]);
    if mirror_helper {
        out.push_str(MIRROR);
    }
    out.push_str(&helpers);
    Ok(out)
}

fn arguments(source: &str, paren: usize) -> Result<(usize, Vec<String>), String> {
    let mut depth = 1;
    let mut from = paren + 1;
    let mut args = Vec::new();
    for (offset, byte) in source.as_bytes()[from..].iter().enumerate() {
        let index = paren + 1 + offset;
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    args.push(source[from..index].trim().to_owned());
                    return Ok((index + 1, args));
                }
            }
            b',' if depth == 1 => {
                args.push(source[from..index].trim().to_owned());
                from = index + 1;
            }
            _ => {}
        }
    }
    Err("unclosed generated sampling operation".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_gui::pdxshader::program::Filter;

    #[test]
    fn mirror_once_gathers_and_offsets_lower_to_native_addressing() {
        let wgsl = "@group(1) @binding(3) var t: texture_2d<f32>;
@group(2) @binding(3) var s: sampler;
fn f(b: B, uv: vec2<f32>) -> vec4<f32> { let w = b.textureUv; return textureGather(0, t, s, uv) + textureSample(t, s, uv, vec2<i32>(1i, 0i)) + textureSample(t, s, uv, vec2<i32>(0i, 0i)); }
";
        let sampler = |u, mip| Sampler {
            name: "Map".into(),
            index: 1,
            cube: false,
            mag: Filter::Linear,
            min: Filter::Linear,
            mip,
            u,
            v: Address::Clamp,
            w: Address::Wrap,
            lod_bias: 0.,
            min_lod: 0,
            max_lod: 1024,
        };
        let plain = [(3, super::super::TextureKind::D2)];
        let mirrored = BTreeMap::from([(3, sampler(Address::MirrorOnce, Filter::Linear))]);
        let lowered = lower(wgsl, &mirrored, &plain, 0).unwrap();
        assert!(lowered.contains("pdx_gather0(t, uv, vec2<i32>(0), u32(0))"));
        assert!(lowered.contains("fn pdx_gather0(") && lowered.contains("textureLoad(t,"));
        assert!(lowered.contains("pdx_sample") && lowered.contains("dpdxCoarse(uv)"));
        assert!(lowered.contains("textureSampleGrad(t, s, vec2<f32>(abs((uv).x), (uv).y)"));
        let clamped = BTreeMap::from([(3, sampler(Address::Clamp, Filter::Linear))]);
        assert_eq!(lower(wgsl, &clamped, &plain, 0).unwrap(), wgsl);
        let cube = [(3, super::super::TextureKind::Cube)];
        assert!(
            !lower(wgsl, &mirrored, &cube, 0)
                .unwrap()
                .contains("pdx_gather")
        );
        let anisotropic = BTreeMap::from([(3, sampler(Address::MirrorOnce, Filter::Anisotropic))]);
        let reason = lower(wgsl, &anisotropic, &plain, 0).unwrap_err();
        assert!(reason.contains("anisotropic") && reason.contains("Map"));
        let no_offset = "@group(1) @binding(3) var t: texture_2d<f32>;
@group(2) @binding(3) var s: sampler;
fn f(uv: vec2<f32>) -> vec4<f32> { return textureSample(t, s, uv); }
";
        assert!(lower(no_offset, &anisotropic, &plain, 0).is_err());
        let manual = lower(no_offset, &clamped, &plain, 1 << 3).unwrap();
        assert!(manual.contains("textureLoad(t,") && !manual.contains("textureSample(t,"));
        assert!(
            lower(no_offset, &clamped, &cube, 1 << 3)
                .unwrap_err()
                .contains("Cube")
        );
        let anisotropic = BTreeMap::from([(3, sampler(Address::Clamp, Filter::Anisotropic))]);
        assert!(
            lower(no_offset, &anisotropic, &plain, 1 << 3)
                .unwrap_err()
                .contains("Anisotropic")
        );
    }

    #[test]
    fn mirror_once_folds_a_volumes_w_axis() {
        let wgsl = "@group(1) @binding(3) var t: texture_3d<f32>;
@group(2) @binding(3) var s: sampler;
fn f(uvw: vec3<f32>) -> vec4<f32> { return textureSample(t, s, uvw); }
";
        let sampler = |w, mip| Sampler {
            name: "Volume".into(),
            index: 1,
            cube: false,
            mag: Filter::Linear,
            min: Filter::Linear,
            mip,
            u: Address::Clamp,
            v: Address::Clamp,
            w,
            lod_bias: 0.,
            min_lod: 0,
            max_lod: 1024,
        };
        let volume = [(3, super::super::TextureKind::D3)];
        let mirrored = BTreeMap::from([(3, sampler(Address::MirrorOnce, Filter::Linear))]);
        let lowered = lower(wgsl, &mirrored, &volume, 0).unwrap();
        assert!(lowered.contains(
            "textureSampleGrad(t, s, vec3<f32>((uvw).x, (uvw).y, abs((uvw).z)), dpdxCoarse(uvw)"
        ));
        let flat = wgsl
            .replace("texture_3d", "texture_2d")
            .replace("vec3<f32>", "vec2<f32>");
        let plain = [(3, super::super::TextureKind::D2)];
        assert_eq!(lower(&flat, &mirrored, &plain, 0).unwrap(), flat);
        let anisotropic = BTreeMap::from([(3, sampler(Address::MirrorOnce, Filter::Anisotropic))]);
        let reason = lower(wgsl, &anisotropic, &volume, 0).unwrap_err();
        assert!(reason.contains("anisotropic") && reason.contains("Volume"));
    }
}
