use super::native_draw::{
    execute_static_exact_used_draw, Gfx, PreparedTargetTexture, StreamOutputBuffers,
    SubmittedMainSceneTarget,
};
use crate::{GameFileResolver, Result};
use replay_core::{
    BufferSource, M02CombatDraw, M02CombatDrawRef, M02CombatFrame, PreparedRenderTarget,
    StaticGeometrySource, StaticShaderResourceSource, UsedContentKey,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::{collections::BTreeMap, ptr};
use winapi::um::d3d11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Resource, ID3D11Texture2D,
};
use winapi::um::unknwnbase::IUnknown;
use winapi::Interface;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct M02OverlayExecutedDraw {
    pub ui_pass_draw_ordinal: u32,
    pub stable_combat_handle_le_hex: String,
    pub semantic_role: String,
    pub draw_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct M02OverlayExecutionReceipt {
    pub schema: String,
    pub update_index: u64,
    pub ordered_draws: Vec<M02OverlayExecutedDraw>,
    pub deterministic_sha256: String,
}

#[derive(Clone)]
struct PreparedM02OverlayDraw {
    reference: M02CombatDrawRef,
    stable_combat_handle_le_hex: String,
    draw: M02CombatDraw,
}

/// Fully preflighted M02 packet set. Product UI-pass playback advances this
/// plan only when the authenticated operation stream reaches an M02 ordinal;
/// non-M02 product-surface operations remain owned by the product compositor.
pub struct PreparedM02OverlayPlan {
    update_index: u64,
    target_rtv: *mut ID3D11RenderTargetView,
    ordered_draws: Vec<PreparedM02OverlayDraw>,
    next_draw_index: usize,
    executed_draws: Vec<M02OverlayExecutedDraw>,
}

impl PreparedM02OverlayPlan {
    pub fn next_reference(&self) -> Option<&M02CombatDrawRef> {
        self.ordered_draws
            .get(self.next_draw_index)
            .map(|draw| &draw.reference)
    }

    pub fn is_complete(&self) -> bool {
        self.next_draw_index == self.ordered_draws.len()
    }
}

/// Persistent exact executor for the separate M02 swapchain UI-overlay stage.
/// It is intentionally not part of static-map or particle/entity ordering.
pub struct NativeM02OverlayExecutor {
    gfx: Gfx,
    stream_output: StreamOutputBuffers,
}

impl NativeM02OverlayExecutor {
    pub unsafe fn new(
        device: *mut ID3D11Device,
        context: *mut ID3D11DeviceContext,
    ) -> Result<Self> {
        Ok(Self {
            gfx: Gfx::borrowed(device, context)?,
            stream_output: StreamOutputBuffers::default(),
        })
    }

    /// Execute only the exact M02 draws, in their recorded complete UI-pass
    /// ordinal order, against the app-owned swapchain target immediately
    /// before Present. The caller owns placement relative to other UI draws.
    pub unsafe fn execute(
        &mut self,
        resolver: &GameFileResolver,
        rtv: *mut ID3D11RenderTargetView,
        frame: &M02CombatFrame,
    ) -> Result<M02OverlayExecutionReceipt> {
        let mut plan = self.prepare(resolver, rtv, frame)?;
        while let Some(reference) = plan.next_reference().cloned() {
            self.execute_next(resolver, rtv, &mut plan, &reference)?;
        }
        Self::finish(plan)
    }

    /// Validate the complete carrier, content admission, exact ordering, and
    /// swapchain output descriptors before the first M02 pixel can be written.
    pub unsafe fn prepare(
        &self,
        resolver: &GameFileResolver,
        rtv: *mut ID3D11RenderTargetView,
        frame: &M02CombatFrame,
    ) -> Result<PreparedM02OverlayPlan> {
        if !resolver.has_content_admission() {
            return Err("M02 overlay execution requires validated effective-VFS admission".into());
        }
        frame
            .validate()
            .map_err(|error| format!("invalid M02 overlay carrier: {error}"))?;
        let target = swapchain_target(rtv)?;
        let mut by_ref = BTreeMap::<M02CombatDrawRef, (&str, &M02CombatDraw)>::new();
        for indicator in &frame.indicators {
            for content in &indicator.used_content {
                resolver.read_admitted_used_content(content)?;
            }
            for draw in &indicator.draws {
                validate_draw_content_membership(draw, &indicator.used_content)?;
                by_ref.insert(
                    M02CombatDrawRef {
                        stable_combat_handle_le_hex: indicator.stable_combat_handle_le_hex.clone(),
                        draw_ordinal: draw.draw_ordinal,
                    },
                    (&indicator.stable_combat_handle_le_hex, draw),
                );
            }
        }

        let mut previous = None;
        let mut ordered_draws = Vec::with_capacity(frame.ordered_draws.len());
        for reference in &frame.ordered_draws {
            if previous.is_some_and(|value| reference.draw_ordinal <= value) {
                return Err(
                    "M02 complete UI-pass draw ordinals are not strictly increasing".into(),
                );
            }
            previous = Some(reference.draw_ordinal);
            let (handle, draw) = by_ref
                .remove(reference)
                .ok_or("M02 ordered draw references an absent exact draw")?;
            validate_overlay_draw_target(draw, &target)?;
            ordered_draws.push(PreparedM02OverlayDraw {
                reference: reference.clone(),
                stable_combat_handle_le_hex: handle.to_string(),
                draw: draw.clone(),
            });
        }
        if !by_ref.is_empty() {
            return Err("M02 ordered draw list omits an exact draw".into());
        }

        Ok(PreparedM02OverlayPlan {
            update_index: frame.update_index,
            target_rtv: rtv,
            ordered_draws,
            next_draw_index: 0,
            executed_draws: Vec::with_capacity(frame.ordered_draws.len()),
        })
    }

    /// Execute exactly the next M02 packet in the preflighted M02-only order.
    /// The containing selected-Present UI-pass stream owns when this method is
    /// called relative to authenticated product-surface segments.
    pub unsafe fn execute_next(
        &mut self,
        resolver: &GameFileResolver,
        rtv: *mut ID3D11RenderTargetView,
        plan: &mut PreparedM02OverlayPlan,
        reference: &M02CombatDrawRef,
    ) -> Result<M02OverlayExecutedDraw> {
        if rtv != plan.target_rtv {
            return Err("M02 prepared overlay target changed before execution".into());
        }
        let prepared = plan
            .ordered_draws
            .get(plan.next_draw_index)
            .ok_or("M02 prepared overlay plan is already complete")?;
        if &prepared.reference != reference {
            return Err("M02 UI-pass operation changes the prepared exact draw order".into());
        }
        let target = swapchain_target(rtv)?;
        validate_overlay_draw_target(&prepared.draw, &target)?;
        execute_static_exact_used_draw(
            &self.gfx,
            resolver,
            &[],
            &prepared.draw.draw,
            &[],
            &[],
            &[] as &[PreparedTargetTexture],
            &target,
            &mut self.stream_output,
        )?;
        let executed = M02OverlayExecutedDraw {
            ui_pass_draw_ordinal: prepared.draw.draw_ordinal,
            stable_combat_handle_le_hex: prepared.stable_combat_handle_le_hex.clone(),
            semantic_role: prepared.draw.semantic_role.clone(),
            draw_token: prepared.draw.draw.draw_token.clone(),
        };
        plan.next_draw_index += 1;
        plan.executed_draws.push(executed.clone());
        Ok(executed)
    }

    pub fn finish(plan: PreparedM02OverlayPlan) -> Result<M02OverlayExecutionReceipt> {
        if !plan.is_complete() {
            return Err("M02 prepared overlay plan was not executed completely".into());
        }
        let mut receipt = M02OverlayExecutionReceipt {
            schema: "hoi4.production-m02-overlay-execution.v1".into(),
            update_index: plan.update_index,
            ordered_draws: plan.executed_draws,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
        Ok(receipt)
    }

    /// Preflight one authenticated non-M02 exact UI pipeline draw without
    /// writing pixels. The caller runs this for the complete UI-pass stream
    /// before executing its first operation.
    pub unsafe fn preflight_exact_ui_draw(
        &self,
        resolver: &GameFileResolver,
        rtv: *mut ID3D11RenderTargetView,
        draw: &replay_core::StaticMapDraw,
        ordered_used_content: &[UsedContentKey],
    ) -> Result<()> {
        if !resolver.has_content_admission() {
            return Err("exact UI draw requires validated effective-VFS admission".into());
        }
        for content in ordered_used_content {
            resolver.read_admitted_used_content(content)?;
        }
        validate_static_draw_content_membership(draw, ordered_used_content, "exact UI draw")?;
        let target = swapchain_target(rtv)?;
        validate_overlay_output(
            &draw.output_state,
            draw.pipeline.rasterizer.scissor_enabled,
            &target,
        )
    }

    /// Execute one already-preflighted exact UI pipeline draw at its global
    /// selected-Present ordinal. Interpreted segments and M02 own their own
    /// operations; this method never reorders or batches D3D submissions.
    pub unsafe fn execute_exact_ui_draw(
        &mut self,
        resolver: &GameFileResolver,
        rtv: *mut ID3D11RenderTargetView,
        draw: &replay_core::StaticMapDraw,
    ) -> Result<()> {
        let target = swapchain_target(rtv)?;
        validate_overlay_output(
            &draw.output_state,
            draw.pipeline.rasterizer.scissor_enabled,
            &target,
        )?;
        execute_static_exact_used_draw(
            &self.gfx,
            resolver,
            &[],
            draw,
            &[],
            &[],
            &[] as &[PreparedTargetTexture],
            &target,
            &mut self.stream_output,
        )
    }
}

fn validate_overlay_draw_target(
    draw: &M02CombatDraw,
    target: &SubmittedMainSceneTarget,
) -> Result<()> {
    let output = &draw.draw.output_state;
    validate_overlay_output(
        output,
        draw.draw.pipeline.rasterizer.scissor_enabled,
        target,
    )
}

fn validate_overlay_output(
    output: &replay_core::RenderOutputState,
    scissor_enabled: bool,
    target: &SubmittedMainSceneTarget,
) -> Result<()> {
    if output.ordered_color_targets.len() != 1 {
        return Err("M02 exact overlay draw does not bind exactly one color target".into());
    }
    let color = &output.ordered_color_targets[0];
    if color.prepared_target != PreparedRenderTarget::SwapchainBackBuffer
        || color.prepared_target != target.color_identity
        || color.persistent_content.is_some()
        || color.mip_level != 0
        || color.width != target.color_width
        || color.height != target.color_height
        || color.dxgi_format_u32 != target.color_format
        || color.sample_count != target.color_samples
        || color.sample_quality != target.color_quality
    {
        return Err(
            "M02 exact overlay color binding disagrees with the authenticated swapchain target"
                .into(),
        );
    }
    if output.depth_target.is_some() {
        return Err("M02 exact swapchain overlay draw unexpectedly binds a depth target".into());
    }
    if scissor_enabled != output.scissor.is_some() {
        return Err("M02 exact overlay scissor enable/state is incomplete".into());
    }
    Ok(())
}

unsafe fn swapchain_target(rtv: *mut ID3D11RenderTargetView) -> Result<SubmittedMainSceneTarget> {
    if rtv.is_null() {
        return Err("M02 overlay target RTV is null".into());
    }
    let mut resource = ptr::null_mut::<ID3D11Resource>();
    (*rtv).GetResource(&mut resource);
    let resource = crate::d3d11_device::D3dPtr::from_raw(resource, "M02 overlay target resource")?;
    let mut texture = ptr::null_mut::<ID3D11Texture2D>();
    let hr = (*(resource.as_raw() as *mut IUnknown)).QueryInterface(
        &ID3D11Texture2D::uuidof(),
        &mut texture as *mut *mut ID3D11Texture2D as *mut *mut _,
    );
    if hr < 0 {
        return Err(format!(
            "M02 overlay target QueryInterface failed: 0x{:08x}",
            hr as u32
        )
        .into());
    }
    let texture = crate::d3d11_device::D3dPtr::from_raw(texture, "M02 overlay target texture")?;
    let mut desc = std::mem::zeroed();
    (*texture.as_raw()).GetDesc(&mut desc);
    Ok(SubmittedMainSceneTarget {
        color_identity: PreparedRenderTarget::SwapchainBackBuffer,
        depth_identity: None,
        color_rtv: rtv,
        depth_dsv: ptr::null_mut(),
        color_width: desc.Width,
        color_height: desc.Height,
        color_format: desc.Format,
        color_samples: desc.SampleDesc.Count,
        color_quality: desc.SampleDesc.Quality,
        depth_width: 0,
        depth_height: 0,
        depth_format: 0,
        depth_samples: 0,
        depth_quality: 0,
    })
}

fn validate_draw_content_membership(
    draw: &M02CombatDraw,
    admitted: &[UsedContentKey],
) -> Result<()> {
    validate_static_draw_content_membership(&draw.draw, admitted, "M02 exact draw")
}

fn validate_static_draw_content_membership(
    draw: &replay_core::StaticMapDraw,
    admitted: &[UsedContentKey],
    role: &str,
) -> Result<()> {
    let mut used = vec![&draw.shader_program.source];
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::UsedContent { content, resource, .. } => {
                used.push(content);
                for subresource in &resource.ordered_subresources {
                    if let BufferSource::UsedContentRange { content, .. } = &subresource.source {
                        used.push(content);
                    }
                }
            }
            _ => {
                return Err(format!(
                    "{role} references a dynamic texture source unavailable to the swapchain UI executor"
                )
                .into())
            }
        }
    }
    let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
        return Err(format!("{role} contains an interpreter geometry source").into());
    };
    for vertex in &geometry.ordered_vertex_buffers {
        if let BufferSource::UsedContentRange { content, .. } = &vertex.source {
            used.push(content);
        }
    }
    if let Some(index) = &geometry.index_buffer {
        if let BufferSource::UsedContentRange { content, .. } = &index.source {
            used.push(content);
        }
    }
    for content in used {
        if !admitted.iter().any(|candidate| candidate == content) {
            return Err(
                format!("{role} uses content absent from its command admission list").into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{RenderOutputState, RenderTargetDesc, ScissorDesc, ViewportDesc};

    fn target() -> SubmittedMainSceneTarget {
        SubmittedMainSceneTarget {
            color_identity: PreparedRenderTarget::SwapchainBackBuffer,
            depth_identity: None,
            color_rtv: ptr::null_mut(),
            depth_dsv: ptr::null_mut(),
            color_width: 1920,
            color_height: 1080,
            color_format: 28,
            color_samples: 1,
            color_quality: 0,
            depth_width: 0,
            depth_height: 0,
            depth_format: 0,
            depth_samples: 0,
            depth_quality: 0,
        }
    }

    fn output() -> RenderOutputState {
        RenderOutputState {
            ordered_color_targets: vec![RenderTargetDesc {
                prepared_target: PreparedRenderTarget::SwapchainBackBuffer,
                width: 1920,
                height: 1080,
                dxgi_format_u32: 28,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }],
            depth_target: None,
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0.0f32.to_bits(),
                top_left_y_f32_bits: 0.0f32.to_bits(),
                width_f32_bits: 1920.0f32.to_bits(),
                height_f32_bits: 1080.0f32.to_bits(),
                min_depth_f32_bits: 0.0f32.to_bits(),
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        }
    }

    #[test]
    fn exact_swapchain_output_preflights() {
        validate_overlay_output(&output(), false, &target()).unwrap();
    }

    #[test]
    fn descriptor_or_depth_mismatch_is_rejected_before_execution() {
        let mut wrong_extent = output();
        wrong_extent.ordered_color_targets[0].width = 1919;
        assert!(validate_overlay_output(&wrong_extent, false, &target()).is_err());

        let mut depth = output();
        let mut depth_target = depth.ordered_color_targets[0].clone();
        depth_target.prepared_target = PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: 7,
        };
        depth.depth_target = Some(depth_target);
        assert!(validate_overlay_output(&depth, false, &target()).is_err());
    }

    #[test]
    fn scissor_enable_and_rectangle_presence_must_match() {
        assert!(validate_overlay_output(&output(), true, &target()).is_err());
        let mut clipped = output();
        clipped.scissor = Some(ScissorDesc {
            left_i32: 10,
            top_i32: 20,
            right_i32: 30,
            bottom_i32: 40,
        });
        assert!(validate_overlay_output(&clipped, false, &target()).is_err());
        validate_overlay_output(&clipped, true, &target()).unwrap();
    }
}
