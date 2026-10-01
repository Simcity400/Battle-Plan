use super::*;
use eframe::wgpu::{self, util::DeviceExt};

#[test]
fn typed_dependency_closure() {
    for (failed, dependents) in [
        (
            Pass::LandBorders,
            &[
                Pass::LandBorders,
                Pass::Rivers,
                Pass::Railways,
                Pass::Trees,
                Pass::Water,
            ][..],
        ),
        (Pass::Underwater, &[Pass::Underwater, Pass::Water][..]),
    ] {
        let mut diagnostics = Diagnostics::new();
        let mut frame = Frame::new(&mut diagnostics);
        frame.run::<()>(failed, || Err(io::Error::other("fault")));
        for pass in [
            Pass::Underwater,
            Pass::Terrain,
            Pass::LandBorders,
            Pass::Rivers,
            Pass::Railways,
            Pass::Trees,
            Pass::Water,
            Pass::SeaBorders,
            Pass::Demilitarized,
            Pass::Straits,
            Pass::CountryNames,
            Pass::Sky,
        ] {
            assert_eq!(frame.ready(pass), !dependents.contains(&pass), "{pass:?}");
        }
    }
}

/// A preflight refusal leaves the single encoder usable for an independent draw.
#[test]
fn preflight_fault_skips_water_and_independent_pass_renders() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let descriptor = wgpu::TextureDescriptor {
        label: Some("independent output"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let output = device.create_texture(&descriptor);
    let view = output.create_view(&Default::default());
    let source = device.create_texture_with_data(
        &queue,
        &wgpu::TextureDescriptor {
            label: Some("fixture source"),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ..descriptor.clone()
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[0, 255, 0, 255],
    );
    let source = source.create_view(&Default::default());
    let blitter = wgpu::util::TextureBlitter::new(&device, output.format());
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut diagnostics = Diagnostics::new();
    let mut frame = Frame::new(&mut diagnostics);
    let failed: Result<(), String> = Err("forced underwater preflight fault".into());
    assert!(frame.run(Pass::Underwater, || compiled(&failed)).is_none());
    assert!(
        frame
            .run::<()>(Pass::Underwater, || panic!("failed pass retried"))
            .is_none()
    );
    assert!(
        frame
            .run::<()>(Pass::Water, || panic!("sampled unavailable refraction"))
            .is_none()
    );
    assert!(
        frame
            .run(Pass::CountryNames, || {
                blitter.copy(&device, &mut encoder, &source, &view);
                Ok(())
            })
            .is_some()
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fixture readback"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        output.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receive
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(
        &buffer.slice(..).get_mapped_range().unwrap()[..4],
        &[0, 255, 0, 255]
    );
    // Persistent diagnostics retain one entry per pass, even on later frames.
    Frame::new(&mut diagnostics).run(Pass::Underwater, || compiled(&failed));
    let warnings: Vec<_> = lines(&diagnostics).collect();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("Map shader underwater: 1 refused pass: forced underwater"))
    );
    assert!(warnings.iter().any(|line| {
        line.contains("Map shader water: 1 refused pass: skipped: output of underwater")
    }));
}
