use super::*;
use eframe::egui::{Color32, ColorImage, TextureId, epaint::ImageDelta};
use eframe::wgpu::util::DeviceExt as _;
use viewer_gpu::gpu_contract::{device_descriptor, select_adapter};

#[test]
fn sessions_are_found_by_their_renderer_and_bound_once() {
    let mut registry = Registry::<u8, &str>::new();
    let first = Arc::new(0u8);
    let second = Arc::new(0u8);
    let bound = registry.bind(&first, || "first");
    assert_eq!(
        *registry.bind(&first, || panic!("a session was rebound")),
        "first"
    );
    assert!(registry.find(&second).is_none());
    registry.bind(&second, || "second");
    assert_eq!(*registry.find(&second).unwrap(), "second");
    assert!(Arc::ptr_eq(&registry.find(&first).unwrap(), &bound));
    drop(first);
    let third = Arc::new(0u8);
    assert!(registry.find(&third).is_none());
    assert_eq!(registry.entries.len(), 1);
    assert_eq!(*registry.bind(&third, || "third"), "third");
    assert_eq!(*registry.find(&second).unwrap(), "second");
    assert_eq!(*bound, "first");
}

fn render_states(
    format: wgpu::TextureFormat,
) -> (MutexGuard<'static, ()>, [egui_wgpu::RenderState; 2]) {
    let serial = viewer_gpu::test_support::serial();
    let state = pollster::block_on(async {
        let instance = shader_pipeline::gpu::instance();
        let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&available_adapters, None).unwrap();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter))
            .await
            .unwrap();
        let renderer = egui_wgpu::Renderer::new(&device, format, Default::default());
        egui_wgpu::RenderState {
            adapter,
            available_adapters,
            instance,
            device,
            queue,
            target_format: format,
            renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
            surface_config: egui_wgpu::SurfaceConfig::LOW_LATENCY,
        }
    });
    let other = with_format(&state, format);
    (serial, [state, other])
}

fn with_format(
    state: &egui_wgpu::RenderState,
    format: wgpu::TextureFormat,
) -> egui_wgpu::RenderState {
    egui_wgpu::RenderState {
        target_format: format,
        renderer: Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
            &state.device,
            format,
            Default::default(),
        ))),
        ..state.clone()
    }
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    ctx
}

fn texture(state: &egui_wgpu::RenderState, size: [u32; 2], pixels: &[[u8; 4]]) -> wgpu::Texture {
    assert_eq!(pixels.len(), size[0] as usize * size[1] as usize);
    state.device.create_texture_with_data(
        &state.queue,
        &wgpu::TextureDescriptor {
            label: Some("session pixel fixture"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
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
        bytemuck::cast_slice(pixels),
    )
}

fn target(texture: &wgpu::Texture, filter: wgpu::FilterMode) -> Target {
    Target {
        view: texture.create_view(&Default::default()),
        filter,
    }
}

fn held(state: &egui_wgpu::RenderState, ids: &[TextureId]) -> Vec<bool> {
    let renderer = state.renderer.read();
    ids.iter()
        .map(|id| renderer.texture(id).is_some())
        .collect()
}

const SCREEN: [u32; 2] = [8, 4];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];

fn composite(
    state: &egui_wgpu::RenderState,
    ctx: &egui::Context,
    pending: &mut egui::TexturesDelta,
    images: &[TextureId],
) -> Vec<u8> {
    let jobs = ctx.tessellate(
        images
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(4.0 * index as f32, 0.0),
                    egui::vec2(4.0, 4.0),
                );
                egui::epaint::ClippedShape {
                    clip_rect: rect,
                    shape: egui::Shape::image(
                        id,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    ),
                }
            })
            .collect(),
        1.0,
    );
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: SCREEN,
        pixels_per_point: 1.0,
    };
    let output = state.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("session composite fixture"),
        size: wgpu::Extent3d {
            width: SCREEN[0],
            height: SCREEN[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: state.target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = output.create_view(&Default::default());
    let mut renderer = state.renderer.write();
    for (id, deltas) in pending.set.drain() {
        for delta in deltas {
            renderer.update_texture(&state.device, &state.queue, id, &delta);
        }
    }
    let mut encoder = state.device.create_command_encoder(&Default::default());
    let commands =
        renderer.update_buffers(&state.device, &state.queue, &mut encoder, &jobs, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("session composite fixture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
    for id in pending.free.drain() {
        renderer.free_texture(&id);
    }
    drop(renderer);
    viewer_gpu::gpu_contract::read_texture(&state.device, &state.queue, &output, 4)
        .unwrap()
}

#[test]
#[ignore = "needs a GPU device"]
fn local_commands_preserve_script_pixels_and_texture_lifetimes() {
    let (_serial, [state, unbound]) = render_states(wgpu::TextureFormat::Rgba8Unorm);
    assert!(local(&unbound).is_none());
    let session = bind(&state);
    assert!(Arc::ptr_eq(&session, &bind(&state.clone())));
    let host = local(&state).unwrap();
    assert!(Arc::ptr_eq(&session, &host.session));
    let ctx = context();
    let first = texture(&state, [2, 2], &[RED; 4]);
    let second = texture(&state, [2, 2], &[GREEN; 4]);
    let campaign = host.register(target(&first, wgpu::FilterMode::Linear));
    let dashboard = host.register(target(&second, wgpu::FilterMode::Linear));
    assert_eq!(
        [campaign, dashboard],
        [TextureId::User(0), TextureId::User(1)]
    );
    drop((first, second));
    assert_eq!(
        crate::app::readback::present(&state, &ctx, campaign, [4, 4], 1.0).unwrap(),
        [RED; 16].concat()
    );
    assert_eq!(
        crate::app::readback::present(&state, &ctx, dashboard, [4, 4], 1.0).unwrap(),
        [GREEN; 16].concat()
    );

    let larger = texture(&state, [4, 4], &[BLUE; 16]);
    host.rebind(campaign, target(&larger, wgpu::FilterMode::Linear));
    drop(larger);
    assert_eq!(held(&state, &[campaign, dashboard]), [true, true]);
    assert_eq!(
        crate::app::readback::present(&state, &ctx, campaign, [4, 4], 1.0).unwrap(),
        [BLUE; 16].concat()
    );
    assert_eq!(
        crate::app::readback::present(&state, &ctx, dashboard, [4, 4], 1.0).unwrap(),
        [GREEN; 16].concat()
    );

    host.free(dashboard);
    assert_eq!(held(&state, &[campaign, dashboard]), [true, false]);
    let again = texture(&state, [2, 2], &[WHITE; 4]);
    let dashboard_again = host.register(target(&again, wgpu::FilterMode::Linear));
    drop(again);
    assert_eq!(dashboard_again, TextureId::User(2));
    assert_eq!(
        crate::app::readback::present(&state, &ctx, campaign, [4, 4], 1.0).unwrap(),
        [BLUE; 16].concat()
    );
    assert_eq!(
        crate::app::readback::present(&state, &ctx, dashboard_again, [4, 4], 1.0).unwrap(),
        [WHITE; 16].concat()
    );

    host.free(campaign);
    assert_eq!(
        held(
            &state,
            &[campaign, dashboard, dashboard_again, TextureId::User(3)]
        ),
        [false, false, true, false]
    );
    assert_eq!(
        crate::app::readback::present(&state, &ctx, dashboard_again, [4, 4], 1.0).unwrap(),
        [WHITE; 16].concat()
    );

    let mut pending = egui::TexturesDelta::default();
    let managed = TextureId::Managed(41);
    let options = egui::TextureOptions::NEAREST;
    pending.push(
        managed,
        ImageDelta::full(
            ColorImage::new(
                [2, 2],
                vec![Color32::RED, Color32::GREEN, Color32::BLUE, Color32::WHITE],
            ),
            options,
        ),
    );
    pending.push(
        managed,
        ImageDelta::partial(
            [1, 0],
            ColorImage::new([1, 1], vec![Color32::YELLOW]),
            options,
        ),
    );
    pending.push(
        managed,
        ImageDelta::partial(
            [1, 0],
            ColorImage::new([1, 1], vec![Color32::MAGENTA]),
            options,
        ),
    );
    let expected: Vec<u8> = (0..4)
        .flat_map(|y| {
            (0..8).flat_map(move |x| match (x, y) {
                (0..=1, 0..=1) => RED,
                (2..=3, 0..=1) => MAGENTA,
                (0..=1, _) => BLUE,
                (2..=3, _) => WHITE,
                _ => BLACK,
            })
        })
        .collect();
    pending.free(managed);
    assert_eq!(composite(&state, &ctx, &mut pending, &[managed]), expected);
    assert!(pending.is_empty());
    assert_eq!(held(&state, &[managed, dashboard_again]), [false, true]);
    host.free(dashboard_again);
    assert_eq!(
        held(&state, &[campaign, dashboard, dashboard_again, managed]),
        [false; 4]
    );
}
