//! Serialized GPU device and pixel-readback fixtures.
use crate::map::{gpu_contract::device_descriptor, test_support::serial as test_support_serial};
use std::sync::MutexGuard;

/// A device on the viewer's own descriptor, with the crate's one-device-at-a-time guard
/// ([`crate::map::test_support::serial`]) taken before the adapter: hold the guard for as
/// long as the device is used.
pub fn device() -> (
    MutexGuard<'static, ()>,
    eframe::wgpu::Device,
    eframe::wgpu::Queue,
) {
    let serial = test_support_serial();
    let instance = eframe::wgpu::Instance::new(
        eframe::wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
    );
    let adapter = pollster::block_on(
        instance.request_adapter(&eframe::wgpu::RequestAdapterOptions::default()),
    )
    .unwrap();
    println!("native shader GPU: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&device_descriptor(&adapter))).unwrap();
    (serial, device, queue)
}

pub fn render_with(
    device: &eframe::wgpu::Device,
    queue: &eframe::wgpu::Queue,
    draw: impl FnOnce(&mut eframe::wgpu::RenderPass<'_>),
) -> Vec<u8> {
    use eframe::wgpu;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native effect pixel comparison"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shader pixels"),
        size: 256 * 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("native shader"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = slice.get_mapped_range().unwrap();
    bytes
        .chunks_exact(256)
        .flat_map(|row| row[..64].iter().copied())
        .collect()
}
