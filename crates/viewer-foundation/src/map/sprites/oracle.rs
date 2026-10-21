//! Offscreen production submission and direct installed-program reference draws.
use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Exercise the production upload, sorting, pipeline cache and draw loop offscreen.
pub fn render_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
) -> (Vec<u8>, BTreeMap<String, u64>, usize) {
    let page = dds::Texture::from_rgba8(installed_content_image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![255; 4],
    });
    render_images(device, queue, batch, &vec![page; 16])
}

pub fn render_images(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
    pages: &[dds::Texture],
) -> (Vec<u8>, BTreeMap<String, u64>, usize) {
    let images = IconTextures::new(device, queue, pages).unwrap();
    let mut pass = SpritePass::new(
        device,
        queue,
        1,
        &images,
        &LazyTextures::new(),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    // The frame's one arena, as the scene owns it: reset per frame, flushed before its draws.
    let arena = shader_pipeline::gpu::Arena::new(device);
    arena.reset();
    pass.upload(device, queue, &arena, batch, [16, 16], 1.);
    let first_count = pass.native.cache.created;
    let bake_count = pass.bake_stream.cache.created;
    let bake_views: Vec<_> = pass.bakes.iter().map(|b| b.view.clone()).collect();
    let retained_frame = pass.uploads.as_ref().unwrap().frame();
    arena.reset();
    // The scene's next frame may overwrite all its own uploads. Retained GUI submissions
    // must still bind their pass-owned bytes, not this deliberately different vertex stream.
    arena.vertices(&[0xff; 4096]).unwrap();
    arena.flush(queue);
    pass.upload(device, queue, &arena, batch, [16, 16], 1.);
    arena.flush(queue);
    assert_eq!(pass.uploads.as_ref().unwrap().frame(), retained_frame);
    assert_eq!(
        first_count, pass.native.cache.created,
        "second frame reuses pipelines"
    );
    assert_eq!(
        bake_count, pass.bake_stream.cache.created,
        "bake pipelines reused"
    );
    assert_eq!(
        bake_views,
        pass.bakes
            .iter()
            .map(|b| b.view.clone())
            .collect::<Vec<_>>(),
        "unchanged bake targets reused"
    );
    let pixels = crate::map::test_support::gpu::render_with(device, queue, |render| {
        pass.draw_runs(render, [16, 16])
    });
    let expected_refusals = pass.native_refusals.clone();
    let mut reference = SpritePass::new(
        device,
        queue,
        1,
        &images,
        &LazyTextures::new(),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let reference_arena = shader_pipeline::gpu::Arena::new(device);
    let mut changed = batch.clone();
    for quad in &mut changed.quads {
        quad.color[0] = 0.25;
    }
    // Returning to the old snapshot after motion must still rebuild: the last submissions
    // belong to the resettable caller arena, even when their inputs happen to match again.
    for (index, (moving, frame_batch)) in [
        (true, batch),
        (true, &changed),
        (false, batch),
        (false, batch),
    ]
    .into_iter()
    .enumerate()
    {
        let private_frame = pass.uploads.as_ref().unwrap().frame();
        arena.reset();
        arena.vertices(&[0xff; 4096]).unwrap();
        pass.set_camera_moving(moving);
        pass.upload(device, queue, &arena, frame_batch, [16, 16], 1.0);
        arena.flush(queue);
        assert_eq!(pass.retained_valid, !moving);
        assert_eq!(
            pass.uploads.as_ref().unwrap().frame(),
            private_frame + u64::from(index == 2),
            "only the first stationary frame touches the private arena"
        );
        assert!(
            pass.retained
                .as_ref()
                .unwrap()
                .matches(batch, [16, 16], 1.0, pass.synced)
        );
        reference_arena.reset();
        reference.upload_changed(device, queue, &reference_arena, frame_batch, [16, 16], 1.0);
        reference_arena.flush(queue);
        let actual = crate::map::test_support::gpu::render_with(device, queue, |render| {
            pass.draw_runs(render, [16, 16])
        });
        let expected = crate::map::test_support::gpu::render_with(device, queue, |render| {
            reference.draw_runs(render, [16, 16])
        });
        assert_eq!(actual, expected, "motion frame {index}");
        assert_eq!(pass.native_refusals, reference.native_refusals);
        assert_eq!(arena.refusals(), reference_arena.refusals());
        if index >= 2 {
            assert_eq!(actual, pixels, "stationary frame {index}");
            assert_eq!(pass.native_refusals, expected_refusals);
        }
    }
    (pixels, pass.native_refusals, first_count)
}

/// Draws `batch` on a 16 by 16 frame through the production upload and draw, fade pages included,
/// over the given image pages.
pub fn render_presented(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
    pages: &[dds::Texture],
) -> (Vec<u8>, usize) {
    render_presented_at(device, queue, batch, pages, [16, 16])
}

/// [`render_presented`] on a frame of `size` pixels.
pub fn render_presented_at(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
    pages: &[dds::Texture],
    size: [u32; 2],
) -> (Vec<u8>, usize) {
    let images = IconTextures::new(device, queue, pages).unwrap();
    let mut pass = SpritePass::new(
        device,
        queue,
        1,
        &images,
        &LazyTextures::new(),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let arena = shader_pipeline::gpu::Arena::new(device);
    arena.reset();
    pass.upload(device, queue, &arena, batch, size, 1.);
    arena.flush(queue);
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.draw_pages(&mut encoder);
    queue.submit([encoder.finish()]);
    let pixels = crate::map::test_support::gpu::render_sized(device, queue, size, |render| {
        pass.draw_runs(render, size)
    });
    (pixels, pass.fade_pages())
}

/// Caller-populated ABI inputs. Reference tests construct these from their cases, independently
/// of GUI emission and SpritePass upload.
pub struct Inputs {
    pub declaration: shader_pipeline::vertices::Declaration,
    pub vertices: Vec<u8>,
    pub stride: usize,
    pub constants: BTreeMap<u32, Vec<u8>>,
    pub known_constants: BTreeMap<u32, Vec<std::ops::Range<u32>>>,
    pub textures: BTreeMap<u32, wgpu::TextureView>,
    pub topology: wgpu::PrimitiveTopology,
    pub state: shader_pipeline::gpu::State,
    pub format: wgpu::TextureFormat,
}

pub struct Oracle {
    pipeline: shader_pipeline::gpu::Pipeline,
    draw: shader_pipeline::gpu::Draw,
}

impl Oracle {
    /// Compile the content-resolved technique, then prepare a direct draw with the reference's
    /// own vertices, constants, resource namespaces and render state.
    pub fn prepare(
        device: &wgpu::Device,
        arena: &shader_pipeline::gpu::Arena,
        effect: &Arc<shader_pipeline::library::Effect>,
        technique: &str,
        inputs: Inputs,
    ) -> Result<Self, String> {
        let selection = effect
            .select(technique)?
            .ok_or_else(|| format!("reference technique {technique:?} is not declared"))?;
        let compiled = effect.compiled(selection.index)?;
        let technique = &effect.source.techniques[selection.index];
        let mut pass =
            shader_pipeline::gpu::Pass::packed(&compiled, &inputs.state, inputs.format, 1);
        pass.primitive.topology = inputs.topology;
        let pipeline = shader_pipeline::gpu::Pipeline::for_pass(
            device,
            &compiled,
            &effect.source,
            technique,
            &inputs.state,
            &pass,
        )?;
        for (slot, ranges) in &inputs.known_constants {
            let bytes = inputs
                .constants
                .get(slot)
                .ok_or_else(|| format!("reference constant slot {slot} has no allocation"))?;
            pipeline.check_initialized(*slot, bytes.len() as u32, ranges)?;
        }
        let draw = pipeline.prepare(
            arena,
            &inputs.constants,
            &inputs.textures,
            &shader_pipeline::vertices::Vertices {
                declaration: inputs.declaration,
                bytes: &inputs.vertices,
                stride: inputs.stride,
                count: inputs.vertices.len() / inputs.stride,
            },
        )?;
        Ok(Self { pipeline, draw })
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_blend_constant(wgpu::Color::WHITE);
        self.pipeline.draw(pass, &self.draw);
    }
}
