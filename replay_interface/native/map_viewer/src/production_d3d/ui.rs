//! Runtime UI draw execution. These inputs are not recorded map-target identities.
use crate::{GameFileResolver, Result};
use replay_core::{
    ConstantBufferBinding, GeometryBinding, PipelineDesc, ScissorDesc, ShaderProgram,
    TextureBinding, ViewportDesc,
};
use snapshot::sha256::Sha256;

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::NativeUiD3dExecutor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeUiTargetDesc {
    pub width: u32,
    pub height: u32,
    pub dxgi_format_u32: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
}

impl NativeUiTargetDesc {
    fn byte_count(&self) -> Result<usize> {
        if self.width == 0
            || self.height == 0
            || self.width > 16384
            || self.height > 16384
            || !matches!(self.dxgi_format_u32, 28 | 87)
            || self.sample_count != 1
            || self.sample_quality != 0
        {
            return Err(
                "native UI target requires a bounded single-sample RGBA8/BGRA8 UNORM surface"
                    .into(),
            );
        }
        usize::try_from(self.width)?
            .checked_mul(usize::try_from(self.height)?)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| "native UI target byte count overflows".into())
    }
}

pub struct UiRgbaSurfaceMut<'a> {
    pub width: u32,
    pub height: u32,
    /// Tight RGBA bytes, regardless of the GPU target's channel storage order.
    pub pixels: &'a mut [u8],
}

impl UiRgbaSurfaceMut<'_> {
    fn validate(&self, target: &NativeUiTargetDesc) -> Result<()> {
        if self.width != target.width
            || self.height != target.height
            || self.pixels.len() != target.byte_count()?
        {
            return Err("native UI destination differs from the admitted target".into());
        }
        Ok(())
    }
}

pub struct NativeUiDrawInputs<'a> {
    pub shader: &'a ShaderProgram,
    pub textures: &'a [TextureBinding],
    pub pipeline: &'a PipelineDesc,
    pub geometry: &'a GeometryBinding,
    pub scene_constants: &'a [ConstantBufferBinding],
    pub draw_constants: &'a [ConstantBufferBinding],
    pub viewport: &'a ViewportDesc,
    pub scissor: Option<&'a ScissorDesc>,
    pub target: &'a NativeUiTargetDesc,
}

/// Immutable borrows prevent mutation after admission. The UI owner separately
/// establishes native setter/selection/lifetime provenance for these operands.
pub struct PreparedNativeUiDraw<'a> {
    inputs: NativeUiDrawInputs<'a>,
    manifest_sha256: String,
}

impl<'a> PreparedNativeUiDraw<'a> {
    pub fn admit(resolver: &GameFileResolver, inputs: NativeUiDrawInputs<'a>) -> Result<Self> {
        let manifest_sha256 = manifest_identity(resolver)?;
        validate_bindings(&inputs)?;
        #[cfg(windows)]
        native::authenticate(resolver, &inputs)?;
        #[cfg(not(windows))]
        return Err("native UI shader authentication requires the Windows compiler".into());
        #[cfg(windows)]
        Ok(Self {
            inputs,
            manifest_sha256,
        })
    }
}

#[derive(Debug)]
pub struct NativeUiDrawExecution {
    output_sha256: String,
}

impl NativeUiDrawExecution {
    pub fn submitted_draw_count(&self) -> usize {
        1
    }
    pub fn output_sha256(&self) -> &str {
        &self.output_sha256
    }
}

pub trait NativeUiDrawExecutor {
    /// Synchronous staged replacement. The Windows implementation changes
    /// immediate-context draw state; subsequent GPU owners must bind their own
    /// state. It never presents or installs viewer history/hit authority.
    fn draw_over(
        &mut self,
        draw: &PreparedNativeUiDraw<'_>,
        destination: UiRgbaSurfaceMut<'_>,
    ) -> Result<NativeUiDrawExecution>;
}

fn manifest_identity(resolver: &GameFileResolver) -> Result<String> {
    resolver
        .content_admission_lease()
        .and_then(|lease| lease.manifest_digest_sha256().map(str::to_owned))
        .filter(|digest| digest.len() == 64)
        .ok_or_else(|| "native UI draw requires the current content admission".into())
}

fn validate_bindings(inputs: &NativeUiDrawInputs<'_>) -> Result<()> {
    inputs.target.byte_count()?;
    if inputs
        .scene_constants
        .len()
        .saturating_add(inputs.draw_constants.len())
        > 28
        || inputs
            .scene_constants
            .iter()
            .chain(inputs.draw_constants)
            .any(|c| c.raw_bytes.len() > 65536)
    {
        return Err("native UI constant-buffer census exceeds VS/PS allocation limits".into());
    }
    // Call the owning render-stream validators, without fabricating a map draw.
    let constants = inputs
        .scene_constants
        .iter()
        .chain(inputs.draw_constants)
        .cloned()
        .collect::<Vec<_>>();
    portable_render_stream::validate_runtime_draw_bindings(
        inputs.shader,
        inputs.pipeline,
        inputs.geometry,
        inputs.textures,
        &constants,
    )
    .map_err(|error| format!("native UI bindings: {error}"))?;
    if inputs.shader.geometry_shader.is_some()
        || inputs.shader.hull_shader.is_some()
        || inputs.shader.domain_shader.is_some()
        || inputs.shader.compute_shader.is_some()
        || !inputs.geometry.stream_output_targets.is_empty()
        || inputs.pipeline.depth.enabled
        || inputs.pipeline.depth.stencil_enabled
        || inputs.pipeline.blend.independent_blend_enabled
        || !inputs.pipeline.blend.additional_targets.is_empty()
    {
        return Err(
            "native UI executor requires a single-color VS/PS draw without depth or stream output"
                .into(),
        );
    }
    // This first facade admits ordinary non-indexed, per-vertex triangles only.
    // Other native branches must arrive with their own reached binding contract.
    if inputs.geometry.index_buffer.is_some()
        || !matches!(
            inputs.pipeline.topology,
            replay_core::PrimitiveTopology::TriangleStrip
                | replay_core::PrimitiveTopology::TriangleList
        )
        || inputs
            .pipeline
            .input_layout
            .iter()
            .any(|input| input.per_instance)
        || !matches!(inputs.geometry.draw, replay_core::DrawCommand::Draw { .. })
    {
        return Err("native UI geometry requires the ordinary non-indexed triangle branch".into());
    }
    if inputs.pipeline.input_layout.len() > 32
        || inputs
            .pipeline
            .input_layout
            .iter()
            .any(|element| element.input_slot >= 32)
        || inputs
            .geometry
            .ordered_vertex_buffers
            .iter()
            .any(|buffer| buffer.stride_bytes > 2048)
    {
        return Err("native UI input layout exceeds D3D11 slot/stride limits".into());
    }
    let v = inputs.viewport;
    let [x, y, w, h, near, far] = [
        v.top_left_x_f32_bits,
        v.top_left_y_f32_bits,
        v.width_f32_bits,
        v.height_f32_bits,
        v.min_depth_f32_bits,
        v.max_depth_f32_bits,
    ]
    .map(f32::from_bits);
    if ![x, y, w, h, near, far].iter().all(|v| v.is_finite())
        || w <= 0.0
        || h <= 0.0
        || x < -32768.0
        || y < -32768.0
        || x + w > 32767.0
        || y + h > 32767.0
        || near < 0.0
        || far > 1.0
        || near > far
    {
        return Err("native UI viewport is outside the D3D11 range".into());
    }
    if inputs.pipeline.rasterizer.scissor_enabled != inputs.scissor.is_some() {
        return Err("native UI scissor state has no matching reached rectangle".into());
    }
    if inputs
        .scissor
        .is_some_and(|r| r.right_i32 <= r.left_i32 || r.bottom_i32 <= r.top_i32)
    {
        return Err("native UI scissor is empty or inverted".into());
    }
    for c in &constants {
        if c.stage_mask_u8 & !3 != 0
            || c.slot >= 14
            || c.byte_count > 65536
            || c.byte_count % 16 != 0
        {
            return Err("native UI constant buffer has an unsupported stage/slot/size".into());
        }
    }
    if inputs.textures.iter().any(|t| {
        !matches!(
            t.stage,
            replay_core::ShaderStage::Vertex | replay_core::ShaderStage::Pixel
        ) || t.slot >= 128
    }) || inputs.pipeline.samplers.iter().any(|s| {
        !matches!(
            s.stage,
            replay_core::ShaderStage::Vertex | replay_core::ShaderStage::Pixel
        ) || s.slot >= 16
    }) {
        return Err("native UI texture/sampler has an unsupported stage or slot".into());
    }
    Ok(())
}

fn channel_order(bytes: &mut [u8], format: u32) {
    if format == 87 {
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
}

/// Commit after every fallible operation. This is channel storage conversion,
/// never another blend or an unpremultiply of the shader output.
fn install_readback(
    target: &NativeUiTargetDesc,
    destination: UiRgbaSurfaceMut<'_>,
    width: u32,
    height: u32,
    format: u32,
    row_pitch: u32,
    mut bytes: Vec<u8>,
) -> Result<NativeUiDrawExecution> {
    destination.validate(target)?;
    if width != target.width
        || height != target.height
        || format != target.dxgi_format_u32
        || row_pitch
            != target
                .width
                .checked_mul(4)
                .ok_or("native UI row pitch overflow")?
        || bytes.len() != target.byte_count()?
    {
        return Err("native UI readback differs from the admitted target".into());
    }
    channel_order(&mut bytes, format);
    let output_sha256 = Sha256::hex(&bytes);
    destination.pixels.copy_from_slice(&bytes);
    Ok(NativeUiDrawExecution { output_sha256 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_refuses_unknown_formats_multisample_and_overflow_extent() {
        let mut t = NativeUiTargetDesc {
            width: 2,
            height: 1,
            dxgi_format_u32: 87,
            sample_count: 1,
            sample_quality: 0,
        };
        assert_eq!(t.byte_count().unwrap(), 8);
        for bad in [0, 29, 91] {
            t.dxgi_format_u32 = bad;
            assert!(t.byte_count().is_err());
        }
        t.dxgi_format_u32 = 28;
        t.sample_count = 4;
        assert!(t.byte_count().is_err());
        t.sample_count = 1;
        t.width = u32::MAX;
        assert!(t.byte_count().is_err());
    }

    #[test]
    fn readback_replaces_once_with_exact_channel_order() {
        let t = NativeUiTargetDesc {
            width: 1,
            height: 1,
            dxgi_format_u32: 87,
            sample_count: 1,
            sample_quality: 0,
        };
        let mut pixels = [19, 31, 47, 211];
        let receipt = install_readback(
            &t,
            UiRgbaSurfaceMut {
                width: 1,
                height: 1,
                pixels: &mut pixels,
            },
            1,
            1,
            87,
            4,
            vec![71, 53, 37, 109],
        )
        .unwrap();
        assert_eq!(pixels, [37, 53, 71, 109]);
        assert_eq!(receipt.submitted_draw_count(), 1);
        assert_eq!(receipt.output_sha256(), Sha256::hex(&pixels));
    }

    #[test]
    fn malformed_readback_never_changes_destination() {
        let t = NativeUiTargetDesc {
            width: 1,
            height: 1,
            dxgi_format_u32: 28,
            sample_count: 1,
            sample_quality: 0,
        };
        for (width, format, pitch, bytes) in [
            (2, 28, 4, vec![0; 4]),
            (1, 87, 4, vec![0; 4]),
            (1, 28, 8, vec![0; 4]),
            (1, 28, 4, vec![0; 3]),
        ] {
            let mut pixels = [7, 13, 17, 23];
            assert!(install_readback(
                &t,
                UiRgbaSurfaceMut {
                    width: 1,
                    height: 1,
                    pixels: &mut pixels
                },
                width,
                1,
                format,
                pitch,
                bytes
            )
            .is_err());
            assert_eq!(pixels, [7, 13, 17, 23]);
        }
    }
}
