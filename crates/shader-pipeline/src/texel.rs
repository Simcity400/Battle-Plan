//! Reads of a texture whose result depends on its size in texels, from the compiled program.
use naga::Expression as E;
use std::collections::BTreeSet;

/// A read that answers differently once the texture is smaller: plain sampling at normalised
/// coordinates is the one read that does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TexelRead {
    Offset,
    Gather,
    Load,
    Size,
}

impl TexelRead {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Offset => "samples it at texel offsets",
            Self::Gather => "gathers its texels",
            Self::Load => "loads its texels",
            Self::Size => "reads its own size",
        }
    }
}

/// Every [`TexelRead`] of the pixel or vertex texture at `binding` in a stage's WGSL. An image
/// the walk cannot follow to its declaration counts against every texture.
pub fn texel_reads(wgsl: &str, binding: u32) -> Result<BTreeSet<TexelRead>, String> {
    let module =
        naga::front::wgsl::parse_str(wgsl).map_err(|why| format!("compiled stage: {why}"))?;
    let target = module
        .global_variables
        .iter()
        .find_map(|(handle, variable)| {
            variable
                .binding
                .as_ref()
                .is_some_and(|resource| resource.group == 1 && resource.binding == binding)
                .then_some(handle)
        });
    let mut reads = BTreeSet::new();
    let Some(target) = target else {
        return Ok(reads);
    };
    let functions = module
        .functions
        .iter()
        .map(|(_, function)| function)
        .chain(module.entry_points.iter().map(|entry| &entry.function));
    for function in functions {
        let ours = |image: naga::Handle<naga::Expression>| match function.expressions[image] {
            E::GlobalVariable(handle) => handle == target,
            _ => true,
        };
        for (_, expression) in function.expressions.iter() {
            match *expression {
                E::ImageSample {
                    image,
                    gather,
                    offset,
                    ..
                } if ours(image) => {
                    if offset.is_some() {
                        reads.insert(TexelRead::Offset);
                    }
                    if gather.is_some() {
                        reads.insert(TexelRead::Gather);
                    }
                }
                E::ImageLoad { image, .. } if ours(image) => {
                    reads.insert(TexelRead::Load);
                }
                E::ImageQuery {
                    image,
                    query: naga::ImageQuery::Size { .. },
                } if ours(image) => {
                    reads.insert(TexelRead::Size);
                }
                _ => {}
            }
        }
    }
    Ok(reads)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(body: &str) -> String {
        format!(
            "@group(1) @binding(11) var refraction: texture_2d<f32>;
             @group(1) @binding(1) var other: texture_2d<f32>;
             @group(2) @binding(11) var linear: sampler;
             @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
                 {body}
             }}"
        )
    }

    #[test]
    fn only_sampling_at_normalised_coordinates_is_not_a_texel_read() {
        let reads = |body: &str| texel_reads(&stage(body), 11).unwrap();
        assert!(reads("return textureSample(refraction, linear, uv);").is_empty());
        assert!(reads("return textureSampleLevel(refraction, linear, uv, 0.0);").is_empty());
        assert!(
            reads("return textureSample(other, linear, uv, vec2<i32>(1, 0));").is_empty(),
            "another texture's offsets"
        );
        assert_eq!(
            reads("return textureSample(refraction, linear, uv, vec2<i32>(1, 0));"),
            BTreeSet::from([TexelRead::Offset])
        );
        assert_eq!(
            reads("return textureGather(0, refraction, linear, uv);"),
            BTreeSet::from([TexelRead::Gather])
        );
        assert_eq!(
            reads("return textureLoad(refraction, vec2<i32>(uv), 0);"),
            BTreeSet::from([TexelRead::Load])
        );
        assert_eq!(
            reads(
                "let size = vec2<f32>(textureDimensions(refraction));
                 return textureSample(refraction, linear, uv + 1.0 / size);"
            ),
            BTreeSet::from([TexelRead::Size])
        );
        assert!(
            texel_reads(&stage("return vec4<f32>(uv, 0.0, 1.0);"), 7)
                .unwrap()
                .is_empty()
        );
    }
}
