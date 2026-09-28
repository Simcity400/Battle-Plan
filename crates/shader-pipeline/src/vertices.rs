//! Native input declarations, independent of the vertex buffer currently bound. The null
//! program transition keeps its input layout, even when the next draw changes buffer stride.

use super::{ScalarKind, VertexInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declaration {
    /// 1422BDD80 passes 142B30CF0; progress builds the same declaration at 14234A5E0.
    Sprite,
    /// 1422BD9F0: float3@0, float2@12, B8G8R8A8_UNORM COLOR0@20/COLOR1@24.
    Text,
    /// 1423421E0: float2 POSITION@0, TEXCOORD0@8, TEXCOORD1@16.
    Circular,
}

/// The declaration belongs to the retained program; bytes/stride/count belong to this draw.
pub struct Vertices<'a> {
    pub declaration: Declaration,
    pub bytes: &'a [u8],
    pub stride: usize,
    pub count: usize,
}

impl Vertices<'_> {
    pub(super) fn pack(&self, inputs: &[VertexInput]) -> Result<Vec<f32>, String> {
        let mut packed = Vec::new();
        for vertex in 0..self.count {
            let base = vertex
                .checked_mul(self.stride)
                .ok_or("vertex offset overflow")?;
            for input in inputs {
                if input.kind != ScalarKind::Float {
                    return Err(format!(
                        "native {:?} declaration supplies floats for {:?}, shader expects {:?}",
                        self.declaration, input.semantic, input.kind
                    ));
                }
                let semantic = input.semantic.to_ascii_uppercase();
                let (offset, components, bgra) = match (self.declaration, semantic.as_str()) {
                    (Declaration::Sprite | Declaration::Text, "POSITION" | "POSITION0") => {
                        (0, 3, false)
                    }
                    (Declaration::Sprite | Declaration::Text, "TEXCOORD" | "TEXCOORD0") => {
                        (12, 2, false)
                    }
                    (Declaration::Text, "COLOR" | "COLOR0") => (20, 4, true),
                    (Declaration::Text, "COLOR1") => (24, 4, true),
                    (Declaration::Circular, "POSITION" | "POSITION0") => (0, 2, false),
                    (Declaration::Circular, "TEXCOORD" | "TEXCOORD0") => (8, 2, false),
                    (Declaration::Circular, "TEXCOORD1") => (16, 2, false),
                    _ => {
                        return Err(format!(
                            "native {:?} declaration has no {:?} input",
                            self.declaration, input.semantic
                        ));
                    }
                };
                let from = base.checked_add(offset).ok_or("vertex offset overflow")?;
                let size = if bgra { 4 } else { components * 4 };
                let end = from.checked_add(size).ok_or("vertex offset overflow")?;
                let bytes = self.bytes.get(from..end).ok_or_else(|| format!("native {:?} input {:?} at vertex {vertex} reads beyond supplied buffer ({} bytes)", self.declaration, input.semantic, self.bytes.len()))?;
                // D3D's input conversion fills missing x/y/z with zero and w with one.
                let mut value = [0.0, 0.0, 0.0, 1.0];
                if bgra {
                    for (dst, src) in [2, 1, 0, 3].into_iter().enumerate() {
                        value[dst] = f32::from(bytes[src]) / 255.0;
                    }
                } else {
                    for (dst, bytes) in bytes.chunks_exact(4).enumerate() {
                        value[dst] = f32::from_le_bytes(bytes.try_into().unwrap());
                    }
                }
                packed.extend_from_slice(&value[..input.components as usize]);
            }
        }
        Ok(packed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
