use super::*;
use std::sync::{Arc, MutexGuard};
use viewer_gpu::gpu_contract::{device_descriptor, select_adapter};
use wgpu::util::DeviceExt as _;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    ctx
}

#[test]
fn image_jobs_keep_physical_extent_and_full_texture_coordinates() {
    let ctx = context();
    let id = egui::TextureId::User(27);
    let jobs = jobs(&ctx, id, [3440, 1440], 1.25);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].clip_rect.min, egui::Pos2::ZERO);
    assert_eq!(jobs[0].clip_rect.max, egui::pos2(2752.0, 1152.0));
    let egui::epaint::Primitive::Mesh(mesh) = &jobs[0].primitive else {
        panic!("expected an image mesh");
    };
    assert_eq!(mesh.texture_id, id);
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.indices.len(), 6);
    for (physical, uv) in [
        ([0.0, 0.0], [0.0, 0.0]),
        ([3440.0, 0.0], [1.0, 0.0]),
        ([0.0, 1440.0], [0.0, 1.0]),
        ([3440.0, 1440.0], [1.0, 1.0]),
    ] {
        assert!(mesh.vertices.iter().any(|vertex| {
            [vertex.pos.x * 1.25, vertex.pos.y * 1.25] == physical
                && [vertex.uv.x, vertex.uv.y] == uv
                && vertex.color == egui::Color32::WHITE
        }));
    }
}

#[test]
fn bgra_readback_swaps_each_pixels_red_and_blue_without_changing_alpha() {
    let mut pixels = [1, 2, 3, 4, 11, 12, 13, 14];
    swap_bgra(&mut pixels);
    assert_eq!(pixels, [3, 2, 1, 4, 13, 12, 11, 14]);
}

fn state() -> (MutexGuard<'static, ()>, egui_wgpu::RenderState) {
    let serial = viewer_gpu::test_support::serial();
    let state = pollster::block_on(async {
        let instance = shader_pipeline::gpu::instance();
        let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&available_adapters, None).unwrap();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter))
            .await
            .unwrap();
        let target_format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = egui_wgpu::Renderer::new(&device, target_format, Default::default());
        egui_wgpu::RenderState {
            adapter,
            available_adapters,
            instance,
            device,
            queue,
            target_format,
            renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
            surface_config: egui_wgpu::SurfaceConfig::LOW_LATENCY,
        }
    });
    (serial, state)
}

fn texture(state: &egui_wgpu::RenderState, pixels: [[u8; 4]; 4]) -> wgpu::Texture {
    state.device.create_texture_with_data(
        &state.queue,
        &wgpu::TextureDescriptor {
            label: Some("readback pixel fixture"),
            size: wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(&pixels),
    )
}

fn refusal(result: io::Result<Vec<u8>>, expected: &str) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), expected);
}

#[test]
#[ignore = "needs a GPU device"]
fn a_native_readback_keeps_fixed_pixels_and_refusal_order() {
    let (_serial, base) = state();
    let ctx = context();
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ] {
        let state = egui_wgpu::RenderState {
            target_format: format,
            renderer: Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
                &base.device,
                format,
                Default::default(),
            ))),
            ..base.clone()
        };
        let first = texture(
            &state,
            [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
            ],
        );
        let id = state.renderer.write().register_native_texture(
            &state.device,
            &first.create_view(&Default::default()),
            wgpu::FilterMode::Nearest,
        );
        assert_eq!(id, egui::TextureId::User(0));
        let fresh = egui::Context::default();
        let wide = egui_wgpu::RenderState {
            target_format: wgpu::TextureFormat::Rgba16Float,
            ..state.clone()
        };
        let beyond = [state.device.limits().max_texture_dimension_2d + 1, 1];
        refusal(
            present(&wide, &fresh, egui::TextureId::Managed(0), beyond, 1.25),
            "scene presentation requires the registered native scene texture",
        );
        refusal(
            present(&wide, &fresh, id, beyond, 1.25),
            "scene readback requires an 8-bit window target, found Rgba16Float",
        );
        refusal(
            present(
                &state,
                &fresh,
                egui::TextureId::User(u64::MAX),
                beyond,
                1.25,
            ),
            "scene presentation exceeds device texture extent",
        );
        assert_eq!(
            present(&state, &ctx, id, [4, 4], 1.25).unwrap(),
            [
                [255u8, 0, 0, 255],
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 255, 0, 255],
                [255, 0, 0, 255],
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [0, 0, 255, 255],
                [0, 0, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
            ]
            .concat(),
            "{format:?}"
        );
        let second = texture(
            &state,
            [
                [0, 0, 0, 255],
                [255, 255, 0, 255],
                [0, 255, 255, 255],
                [255, 0, 255, 255],
            ],
        );
        state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &state.device,
                &second.create_view(&Default::default()),
                wgpu::FilterMode::Nearest,
                id,
            );
        drop(first);
        assert_eq!(
            present(&state, &ctx, id, [4, 4], 1.25).unwrap(),
            [
                [0u8, 0, 0, 255],
                [0, 0, 0, 255],
                [255, 255, 0, 255],
                [255, 255, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [255, 255, 0, 255],
                [255, 255, 0, 255],
                [0, 255, 255, 255],
                [0, 255, 255, 255],
                [255, 0, 255, 255],
                [255, 0, 255, 255],
                [0, 255, 255, 255],
                [0, 255, 255, 255],
                [255, 0, 255, 255],
                [255, 0, 255, 255],
            ]
            .concat(),
            "{format:?}"
        );
        refusal(
            present(&state, &ctx, egui::TextureId::User(u64::MAX), [4, 4], 1.25),
            "native scene texture is not registered in the renderer",
        );
        state.renderer.write().free_texture(&id);
        refusal(
            present(&state, &ctx, id, [4, 4], 1.25),
            "native scene texture is not registered in the renderer",
        );
    }
}
