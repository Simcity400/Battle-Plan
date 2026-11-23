use shader_pipeline::gpu::stats::CountedPasses as _;
use eframe::{egui, egui_wgpu, wgpu};
use std::io;

#[cfg(test)]
mod tests;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn jobs(
    ctx: &egui::Context,
    texture: egui::TextureId,
    pixels: [u32; 2],
    ppp: f32,
) -> Vec<egui::ClippedPrimitive> {
    let rect = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(pixels[0] as f32 / ppp, pixels[1] as f32 / ppp),
    );
    ctx.tessellate(
        vec![egui::epaint::ClippedShape {
            clip_rect: rect,
            shape: egui::Shape::image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            ),
        }],
        ppp,
    )
}

pub(crate) fn present(
    state: &egui_wgpu::RenderState,
    ctx: &egui::Context,
    texture: egui::TextureId,
    size: [u32; 2],
    pixels_per_point: f32,
) -> io::Result<Vec<u8>> {
    if !matches!(texture, egui::TextureId::User(_)) {
        return Err(invalid(
            "scene presentation requires the registered native scene texture",
        ));
    }
    let bgra = match state.target_format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        format => {
            return Err(invalid(format!(
                "scene readback requires an 8-bit window target, found {format:?}"
            )));
        }
    };
    if size
        .into_iter()
        .any(|v| v > state.device.limits().max_texture_dimension_2d)
    {
        return Err(invalid("scene presentation exceeds device texture extent"));
    }
    let mut pixels = viewer_gpu::gpu_contract::scoped(
        &state.device,
        || "The graphics device could not read this map back".to_owned(),
        || draw_and_read(state, ctx, texture, size, pixels_per_point),
    )?;
    if bgra {
        swap_bgra(&mut pixels);
    }
    Ok(pixels)
}

fn draw_and_read(
    state: &egui_wgpu::RenderState,
    ctx: &egui::Context,
    texture: egui::TextureId,
    size: [u32; 2],
    pixels_per_point: f32,
) -> io::Result<Vec<u8>> {
    let target = state.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene image readback target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: state.target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let jobs = jobs(ctx, texture, size, pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point,
    };
    let mut encoder = state
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene image readback encoder"),
        });
    let mut renderer = state.renderer.write();
    if renderer.texture(&texture).is_none() {
        return Err(invalid(
            "native scene texture is not registered in the renderer",
        ));
    }
    let commands =
        renderer.update_buffers(&state.device, &state.queue, &mut encoder, &jobs, &screen);
    {
        let mut pass = encoder
            .counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene image readback pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &jobs, &screen);
    }
    state
        .queue
        .submit(commands.into_iter().chain([encoder.finish()]));
    drop(renderer);
    viewer_gpu::gpu_contract::read_texture(&state.device, &state.queue, &target, 4)
}

fn swap_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}
