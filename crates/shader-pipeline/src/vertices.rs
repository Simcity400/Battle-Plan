//! Native input declarations, independent of the vertex buffer currently bound. The null
//! program transition keeps its input layout, even when the next draw changes buffer stride.

use super::{ScalarKind, VertexInput};
mod native;
pub use native::{NativeBuffer, PackedBuffers, pack_native};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declaration {
    /// Sprite and progress draws share float3 POSITION and float2 TEXCOORD0.
    Sprite,
    /// float3 POSITION at 0, float2 TEXCOORD0 at 12, B8G8R8A8_UNORM COLOR0 at 20/COLOR1 at 24.
    Text,
    /// float2 POSITION at 0, TEXCOORD0 at 8, TEXCOORD1 at 16.
    Circular,
    LineChart,
}

impl Declaration {
    pub(super) const ALL: [Self; 4] = [Self::Sprite, Self::Text, Self::Circular, Self::LineChart];
}

/// The declaration belongs to the retained program; bytes/stride/count belong to this draw.
pub struct Vertices<'a> {
    pub declaration: Declaration,
    pub bytes: &'a [u8],
    pub stride: usize,
    pub count: usize,
}

/// A pipeline's native fetch conversion for one retained declaration. Errors are kept at
/// their input position, so a short buffer still refuses at the first read it cannot supply.
pub(super) struct Packing {
    inputs: Vec<Result<Input, String>>,
    components: usize,
}

struct Input {
    short: bool,
    kind: ScalarKind,
    semantic: String,
    offset: usize,
    components: usize,
    output: usize,
    bgra: bool,
}

impl Packing {
    pub(super) fn new(declaration: Declaration, inputs: &[VertexInput]) -> Self {
        Self {
            components: inputs.iter().map(|input| input.components as usize).sum(),
            inputs: inputs.iter().map(|input| {
                if input.kind != ScalarKind::Float && declaration != Declaration::LineChart {
                    return Err(format!(
                        "native {declaration:?} declaration supplies floats for {:?}, shader expects {:?}",
                        input.semantic, input.kind
                    ));
                }
                let semantic = input.semantic.to_ascii_uppercase();
                let (offset, components, bgra) = match (declaration, semantic.as_str()) {
                    (Declaration::Sprite | Declaration::Text, "POSITION" | "POSITION0") => (0, 3, false),
                    (Declaration::Sprite | Declaration::Text, "TEXCOORD" | "TEXCOORD0") => (12, 2, false),
                    (Declaration::Text, "COLOR" | "COLOR0") => (20, 4, true),
                    (Declaration::Text, "COLOR1") => (24, 4, true),
                    (Declaration::Circular, "POSITION" | "POSITION0") => (0, 2, false),
                    (Declaration::Circular, "TEXCOORD" | "TEXCOORD0") => (8, 2, false),
                    (Declaration::Circular, "TEXCOORD1") => (16, 2, false),
                    (Declaration::LineChart, "POSITION" | "POSITION0") => (0, 4, false),
                    _ => return Err(format!("native {declaration:?} declaration has no {:?} input", input.semantic)),
                };
                Ok(Input {
                    short: declaration == Declaration::LineChart,
                    kind: input.kind,
                    semantic: input.semantic.clone(), offset, components,
                    output: input.components as usize, bgra,
                })
            }).collect(),
        }
    }

    pub(super) fn pack_into(
        &self,
        vertices: &Vertices<'_>,
        packed: &mut Vec<f32>,
    ) -> Result<(), String> {
        packed.clear();
        // A short supplied buffer must refuse at its first missing fetch, not reserve for
        // vertices it cannot supply. Zero-stride fetches still repeat their current bytes.
        let reserve = vertices
            .count
            .min(vertices.bytes.len() / vertices.stride.max(1));
        packed.reserve(
            reserve
                .checked_mul(self.components)
                .ok_or("vertex size overflow")?,
        );
        for vertex in 0..vertices.count {
            let base = vertex
                .checked_mul(vertices.stride)
                .ok_or("vertex offset overflow")?;
            for input in &self.inputs {
                let input = input.as_ref().map_err(Clone::clone)?;
                let from = base
                    .checked_add(input.offset)
                    .ok_or("vertex offset overflow")?;
                let size = if input.bgra {
                    4
                } else if input.short {
                    input.components * 2
                } else {
                    input.components * 4
                };
                let end = from.checked_add(size).ok_or("vertex offset overflow")?;
                let bytes = vertices.bytes.get(from..end).ok_or_else(|| format!("native {:?} input {:?} at vertex {vertex} reads beyond supplied buffer ({} bytes)", vertices.declaration, input.semantic, vertices.bytes.len()))?;
                // D3D's input conversion fills missing x/y/z with zero and w with one.
                let mut value = [0.0, 0.0, 0.0, 1.0];
                if input.short {
                    for (dst, bytes) in bytes.chunks_exact(2).enumerate() {
                        let value_raw = i16::from_le_bytes(bytes.try_into().unwrap()) as i32;
                        value[dst] = match input.kind {
                            ScalarKind::Float => value_raw as f32,
                            ScalarKind::Sint | ScalarKind::Uint => f32::from_bits(value_raw as u32),
                        };
                    }
                } else if input.bgra {
                    for (dst, src) in [2, 1, 0, 3].into_iter().enumerate() {
                        value[dst] = f32::from(bytes[src]) / 255.0;
                    }
                } else {
                    for (dst, bytes) in bytes.chunks_exact(4).enumerate() {
                        value[dst] = f32::from_le_bytes(bytes.try_into().unwrap());
                    }
                }
                packed.extend_from_slice(&value[..input.output]);
            }
        }
        Ok(())
    }
}

impl Vertices<'_> {
    pub(super) fn pack(&self, inputs: &[VertexInput]) -> Result<Vec<f32>, String> {
        let mut packed = Vec::new();
        Packing::new(self.declaration, inputs).pack_into(self, &mut packed)?;
        Ok(packed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Packing {
        fn pack(&self, vertices: &Vertices<'_>) -> Result<Vec<f32>, String> {
            let mut packed = Vec::new();
            self.pack_into(vertices, &mut packed)?;
            Ok(packed)
        }
    }

    #[test]
    fn prepared_fetches_keep_read_failure_order_and_current_bytes() {
        let inputs = [
            VertexInput {
                kind: ScalarKind::Float,
                location: 0,
                semantic: "position0".into(),
                components: 4,
            },
            VertexInput {
                kind: ScalarKind::Uint,
                location: 1,
                semantic: "COLOR0".into(),
                components: 4,
            },
        ];
        let plan = Packing::new(Declaration::Sprite, &inputs);
        let vertices = Vertices {
            declaration: Declaration::Sprite,
            bytes: &[],
            stride: 12,
            count: 1,
        };
        assert!(
            plan.pack(&vertices)
                .unwrap_err()
                .contains("reads beyond supplied buffer")
        );
        assert!(
            plan.pack(&Vertices {
                count: usize::MAX,
                ..vertices
            })
            .unwrap_err()
            .contains("reads beyond supplied buffer")
        );
        let bytes = [2_f32, 3., 4.];
        let vertices = Vertices {
            bytes: bytemuck::cast_slice(&bytes),
            ..vertices
        };
        assert!(
            plan.pack(&vertices)
                .unwrap_err()
                .contains("shader expects Uint")
        );
        let plan = Packing::new(Declaration::Sprite, &inputs[..1]);
        assert_eq!(plan.pack(&vertices).unwrap(), [2., 3., 4., 1.]);
        let mut scratch = Vec::new();
        plan.pack_into(&vertices, &mut scratch).unwrap();
        let allocation = scratch.as_ptr();
        plan.pack_into(&vertices, &mut scratch).unwrap();
        assert_eq!(
            scratch.as_ptr(),
            allocation,
            "equal-sized draws reuse packed storage"
        );
        let changed = [-2_f32, -3., -4.];
        assert_eq!(
            plan.pack(&Vertices {
                bytes: bytemuck::cast_slice(&changed),
                ..vertices
            })
            .unwrap(),
            [-2., -3., -4., 1.]
        );
        assert!(
            Packing::new(Declaration::Sprite, &inputs)
                .pack(&Vertices {
                    count: 0,
                    ..vertices
                })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn retained_declaration_reads_current_buffer_stride_and_bgra_channels() {
        let mut bytes = vec![0; 56];
        for (offset, value) in [(0, 2_f32), (8, 1.), (28, 3.), (36, 1.)] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[20..28].copy_from_slice(&[51, 102, 153, 204, 255, 0, 0, 255]);
        bytes[48..56].copy_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255]);
        let inputs: Vec<_> = ["POSITION0", "COLOR0", "COLOR1"]
            .into_iter()
            .enumerate()
            .map(|(location, name)| VertexInput {
                kind: ScalarKind::Float,
                location: location as u32,
                semantic: name.into(),
                components: 4,
            })
            .collect();
        let vertices = Vertices {
            declaration: Declaration::Text,
            bytes: &bytes,
            stride: 28,
            count: 2,
        };
        assert_eq!(
            vertices.pack(&inputs).unwrap(),
            [
                2., 0., 1., 1., 0.6, 0.4, 0.2, 0.8, 0., 0., 1., 1., 3., 0., 1., 1., 1., 0., 0., 1.,
                0., 1., 0., 1.
            ]
        );
        // A previous sprite program still fetches the second vertex at byte 28.
        let vertices = Vertices {
            declaration: Declaration::Sprite,
            ..vertices
        };
        assert_eq!(
            vertices.pack(&inputs[..1]).unwrap(),
            [2., 0., 1., 1., 3., 0., 1., 1.]
        );
        assert!(vertices.pack(&inputs).unwrap_err().contains("COLOR0"));
        let circular = [2_f32, 3., 4., 5., 6., 7.];
        let vertices = Vertices {
            declaration: Declaration::Circular,
            bytes: bytemuck::cast_slice(&circular),
            stride: 24,
            count: 1,
        };
        let inputs = [
            VertexInput {
                kind: ScalarKind::Float,
                location: 0,
                semantic: "POSITION".into(),
                components: 4,
            },
            VertexInput {
                kind: ScalarKind::Float,
                location: 1,
                semantic: "TEXCOORD1".into(),
                components: 2,
            },
        ];
        assert_eq!(vertices.pack(&inputs).unwrap(), [2., 3., 0., 1., 6., 7.]);
    }
}
