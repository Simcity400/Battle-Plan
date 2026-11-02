//! A composed window drawn as the game draws it, offscreen. Every part is emitted as the map
//! viewer emits it ([`native_shader::draw::emit`]) and drawn by the viewer's sprite pass into
//! one layer that keeps premultiplied colour, returned as straight alpha: the content's own
//! effects, techniques, blend and sampler states, text effects and colour runs. A part the
//! native draw refuses is left out and named by path and reason.

use std::sync::OnceLock;

use installed_content_image::{Rgba8Image, dds};
use viewer_foundation::map::{
    gpu_contract,
    lazy_icons::LazyTextures,
    native_gui::{Part, Sheets},
    native_shader,
    sprites::{Batch, IconTextures, SpritePass},
};

use crate::scene::Counted;

/// The device every window is drawn on, opened once per process.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();

fn gpu() -> Result<&'static Gpu, String> {
    GPU.get_or_init(|| {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .map_err(|why| format!("no GPU adapter draws game windows: {why}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&gpu_contract::device_descriptor(&adapter)))
                .map_err(|why| format!("the GPU refused a device for game windows: {why}"))?;
        Ok(Gpu { device, queue })
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A session's texture pages on the device and the sprite pass drawing over them. Pages a
/// later window loads rebuild it.
pub struct Painter {
    pages: usize,
    pass: SpritePass,
}

/// One painted window and what its draw refused, counted.
pub struct Painted {
    pub image: Rgba8Image,
    pub refused: Counted,
}

impl Painter {
    fn new(gpu: &Gpu, pages: &[dds::Texture]) -> Result<Self, String> {
        let textures = IconTextures::new(&gpu.device, &gpu.queue, pages)
            .map_err(|why| format!("a window's texture pages do not upload: {why}"))?;
        let mut pass = SpritePass::new(
            &gpu.device,
            &gpu.queue,
            1,
            &textures,
            &LazyTextures::new(),
            shader_pipeline::artwork::LAYER_FORMAT,
        )
        .map_err(|why| format!("the window's sprite pass is unavailable: {why}"))?;
        pass.set_layer(true);
        Ok(Self {
            pages: pages.len(),
            pass,
        })
    }
}

/// Draws `parts` in order over a window of `size` interface pixels, `pages` being the
/// session's texture pages by id. `painter` is the session's, built or rebuilt here.
pub fn paint<'a>(
    painter: &mut Option<Painter>,
    pages: &[dds::Texture],
    sheets: &dyn Sheets,
    parts: impl IntoIterator<Item = &'a Part>,
    size: [i32; 2],
) -> Result<Painted, String> {
    let gpu = gpu()?;
    let size = size.map(|edge| u32::try_from(edge).unwrap_or(0));
    if size.contains(&0) {
        return Err("the window has no extent to draw".into());
    }
    let mut refused = Counted::default();
    let mut batch = Batch::default();
    for part in parts {
        batch.at_part(&part.path);
        let first = batch.quads.len();
        if let Err(refusal) = native_shader::draw::emit(part, [0, 0], sheets, &mut batch) {
            batch.quads.truncate(first);
            refused.note(format!("{} is not drawn: {}", part.path, refusal.detail));
        }
    }
    if painter
        .as_ref()
        .is_none_or(|painter| painter.pages != pages.len())
    {
        *painter = Some(Painter::new(gpu, pages)?);
    }
    let painter = painter.as_mut().expect("built above");
    let arena = shader_pipeline::gpu::Arena::new(&gpu.device);
    arena.reset();
    painter
        .pass
        .upload(&gpu.device, &gpu.queue, &arena, &batch, size, 1.0);
    let pass = &mut painter.pass;
    for (reason, count) in &pass.native_refusals {
        for _ in 0..*count {
            refused.note(reason.clone());
        }
    }
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("game window"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: shader_pipeline::artwork::LAYER_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let row = size[0] as usize * 8;
    let pitch = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("game window pixels"),
        size: (pitch * size[1] as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pass.draw_layer(&mut encoder, &target.create_view(&Default::default()), size)?;
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch as u32),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    arena.flush(&gpu.queue);
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|why| why.to_string())?;
    receiver
        .recv()
        .map_err(|why| why.to_string())?
        .map_err(|why| why.to_string())?;
    let mapped = slice.get_mapped_range().map_err(|why| why.to_string())?;
    let pixels = mapped
        .chunks_exact(pitch)
        .flat_map(|texels| shader_pipeline::artwork::layer_pixels(&texels[..row]))
        .collect();
    Ok(Painted {
        image: Rgba8Image {
            width: size[0],
            height: size[1],
            pixels,
        },
        refused,
    })
}
