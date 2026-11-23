use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, VertexInput, gpu, library::Effect};
use std::{io, sync::Arc};
#[path = "particle_shader/trail.rs"]
pub mod trail;

pub const QUAD: [[f32; 2]; 4] = [[0., 0.], [0., 1.], [1., 0.], [1., 1.]];
pub const INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    pub position_size: [f32; 4],
    pub rotation: [f32; 3],
    pub tile: [u8; 4],
    pub color: [u8; 4],
    pub tail: [u8; 12],
}

pub struct Packet {
    pub world: [[f32; 4]; 4],
    pub scale: [f32; 2],
    pub tiles: [i32; 2],
    pub time: f32,
}

pub fn world_scale(basis: [[f32; 3]; 3]) -> f32 {
    let [a, b, c] = basis.map(|v| v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    let maxss = |a, b| if a > b { a } else { b };
    maxss(a, maxss(b, c)).sqrt()
}

fn fail(reason: impl ToString) -> io::Error {
    io::Error::other(format!(
        "Map shader particles: 1 refused pass: {}",
        reason.to_string()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library.load(b"gfx/FX/particle.lua", &defines).map_err(fail)
}

pub fn layouts(inputs: &[VertexInput], trail: bool) -> io::Result<Vec<gpu::VertexLayout>> {
    if trail {
        trail_layout(inputs)
    } else {
        vertex_layouts(inputs)
    }
}

fn trail_layout(inputs: &[VertexInput]) -> io::Result<Vec<gpu::VertexLayout>> {
    let mut attributes = Vec::new();
    for input in inputs {
        let (offset, format, kind, width) = match input.semantic.as_str() {
            "POSITION" | "POSITION0" => (0, wgpu::VertexFormat::Float32x3, ScalarKind::Float, 3),
            "TEXCOORD" | "TEXCOORD0" => (12, wgpu::VertexFormat::Float32x2, ScalarKind::Float, 2),
            "TEXCOORD1" => (20, wgpu::VertexFormat::Uint8x4, ScalarKind::Uint, 4),
            "COLOR" | "COLOR0" => (24, wgpu::VertexFormat::Unorm8x4, ScalarKind::Float, 4),
            _ => return Err(fail(format!("trail declaration cannot supply {input:?}"))),
        };
        if input.kind != kind || input.components > width {
            return Err(fail(format!("trail declaration cannot supply {input:?}")));
        }
        attributes.push(wgpu::VertexAttribute {
            offset,
            format,
            shader_location: input.location,
        });
    }
    Ok(vec![gpu::VertexLayout {
        stride: 28,
        step: wgpu::VertexStepMode::Vertex,
        attributes,
    }])
}

fn vertex_layouts(inputs: &[VertexInput]) -> io::Result<Vec<gpu::VertexLayout>> {
    let mut layouts = vec![
        gpu::VertexLayout {
            stride: 8,
            step: wgpu::VertexStepMode::Vertex,
            attributes: Vec::new(),
        },
        gpu::VertexLayout {
            stride: 48,
            step: wgpu::VertexStepMode::Instance,
            attributes: Vec::new(),
        },
    ];
    for input in inputs {
        let (stream, offset, format, kind, width) = match input.semantic.as_str() {
            "TEXCOORD" | "TEXCOORD0" => (0, 0, wgpu::VertexFormat::Float32x2, ScalarKind::Float, 2),
            "TEXCOORD1" => (1, 0, wgpu::VertexFormat::Float32x4, ScalarKind::Float, 4),
            "TEXCOORD2" => (1, 16, wgpu::VertexFormat::Float32x3, ScalarKind::Float, 3),
            "TEXCOORD3" => (1, 28, wgpu::VertexFormat::Uint8x4, ScalarKind::Uint, 4),
            "COLOR" | "COLOR0" => (1, 32, wgpu::VertexFormat::Unorm8x4, ScalarKind::Float, 4),
            _ => {
                return Err(fail(format!(
                    "simple particle declaration cannot supply {input:?}"
                )));
            }
        };
        if input.kind != kind || input.components > width {
            return Err(fail(format!(
                "simple particle declaration cannot supply {input:?}"
            )));
        }
        layouts[stream].attributes.push(wgpu::VertexAttribute {
            offset,
            format,
            shader_location: input.location,
        });
    }
    Ok(layouts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actor_scale_uses_largest_basis_and_native_nan_order() {
        assert_eq!(world_scale([[2., 0., 0.], [0., 4., 0.], [0., 0., 3.]]), 4.);
        assert_eq!(
            world_scale([[f32::NAN, 0., 0.], [0., 4., 0.], [0., 0., 3.]]),
            4.
        );
        assert!(world_scale([[2., 0., 0.], [0., 4., 0.], [0., 0., f32::NAN]]).is_nan());
    }
}
