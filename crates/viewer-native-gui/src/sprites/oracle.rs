use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn render_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
) -> (Vec<u8>, BTreeMap<String, u64>, usize) {
    let page = dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
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
        &images,
        &LazyTextures::new(),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let arena = shader_pipeline::gpu::Arena::new(device);
    arena.reset();
    pass.upload(device, queue, &arena, batch, [16, 16], 1.);
    queue.submit(pass.bake_buffers());
    let first_count = pass.native.cache.created;
    let bake_count = pass.bake_stream.cache.created;
    let bake_views: Vec<_> = pass.bakes.iter().map(|b| b.view.clone()).collect();
    let retained_frame = pass.uploads.as_ref().unwrap().frame();
    arena.reset();
    arena.vertices(&[0xff; 4096]).unwrap();
    arena.flush(queue);
    pass.upload(device, queue, &arena, batch, [16, 16], 1.);
    arena.flush(queue);
    queue.submit(pass.bake_buffers());
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
    let pixels = viewer_gpu::test_support::gpu::render_with(device, queue, |render| {
        pass.draw_runs(render, [16, 16])
    });
    let expected_refusals = pass.native_refusals.clone();
    let mut reference = SpritePass::new(
        device,
        queue,
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
    for (index, frame_batch) in [batch, &changed, batch, batch].into_iter().enumerate() {
        let private_frame = pass.uploads.as_ref().unwrap().frame();
        arena.reset();
        arena.vertices(&[0xff; 4096]).unwrap();
        pass.upload(device, queue, &arena, frame_batch, [16, 16], 1.0);
        arena.flush(queue);
        queue.submit(pass.bake_buffers());
        assert!(pass.retained_valid);
        assert_eq!(
            pass.uploads.as_ref().unwrap().frame(),
            private_frame + u64::from(matches!(index, 1 | 2)),
            "an upload that retains everything leaves the private arena alone"
        );
        assert!(pass.retained.as_ref().unwrap().matches(
            frame_batch,
            [16, 16],
            1.0,
            pass.synced
        ));
        reference_arena.reset();
        reference.upload_changed(device, queue, &reference_arena, frame_batch, [16, 16], 1.0, None);
        reference_arena.flush(queue);
        queue.submit(reference.bake_buffers());
        let actual = viewer_gpu::test_support::gpu::render_with(device, queue, |render| {
            pass.draw_runs(render, [16, 16])
        });
        let expected = viewer_gpu::test_support::gpu::render_with(device, queue, |render| {
            reference.draw_runs(render, [16, 16])
        });
        assert_eq!(actual, expected, "frame {index}");
        assert_eq!(pass.native_refusals, reference.native_refusals);
        assert_eq!(arena.refusals(), reference_arena.refusals());
        if index >= 2 {
            assert_eq!(actual, pixels, "frame {index}");
            assert_eq!(pass.native_refusals, expected_refusals);
        }
    }
    (pixels, pass.native_refusals, first_count)
}

pub fn render_presented(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
    pages: &[dds::Texture],
) -> (Vec<u8>, usize) {
    render_presented_at(device, queue, batch, pages, [16, 16])
}

pub fn render_presented_at(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
    pages: &[dds::Texture],
    size: [u32; 2],
) -> (Vec<u8>, usize) {
    use viewer_gpu::gpu_contract::{OUTPUT_FORMAT, read_texture};
    let images = IconTextures::new(device, queue, pages).unwrap();
    let mut pass =
        SpritePass::new(device, queue, &images, &LazyTextures::new(), OUTPUT_FORMAT).unwrap();
    let arena = shader_pipeline::gpu::Arena::new(device);
    arena.reset();
    pass.upload(device, queue, &arena, batch, size, 1.);
    arena.flush(queue);
    let targets = Targets::new(device, size, size, 4, viewer_gpu::gpu_contract::SeaBed::Half);
    let output = targets.output_attachment(wgpu::LoadOp::Load);
    assert_eq!(output.view.texture().sample_count(), 1);
    assert!(output.resolve_target.is_none());
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.draw(&mut encoder, &targets);
    queue.submit([encoder.finish()]);
    let pixels = read_texture(device, queue, targets.output(), 4).unwrap();
    (pixels, pass.fade_pages())
}

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
