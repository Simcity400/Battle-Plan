use super::*;
use crate::Buffer;

fn shader(wgsl: &str, bindings: &[(u32, u32)]) -> Shader {
    Shader {
        wgsl: wgsl.into(),
        independent_draws: true,
        buffers: bindings
            .iter()
            .map(|(binding, size)| Buffer {
                binding: *binding,
                size: *size,
                reads: std::iter::once(0..16).collect(),
            })
            .collect(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    }
}

fn skinned() -> Compiled {
    Compiled {
        vertex: shader(
            "
                struct Scene { view: mat4x4<f32> }
                struct Node { world: mat4x4<f32> }
                struct Palette { bones: array<vec4<f32>, 64> }
                @group(0) @binding(0) var<uniform> scene: Scene;
                @group(0) @binding(2) var<uniform> node: Node;
                @group(0) @binding(4) var<uniform> palette: Palette;
                @vertex fn main(@location(0) place: vec4<i32>, @location(1) height: vec2<i32>)
                    -> @builtin(position) vec4<f32> {
                    let local = vec4<f32>(f32(place.x), f32(height.x), f32(place.y), 1.0);
                    return scene.view * (node.world * local) + palette.bones[place.z];
                }
            ",
            &[(0, 64), (2, 64), (4, 1024)],
        ),
        pixel: shader(
            "
                struct Tint { colour: vec4<f32> }
                @group(0) @binding(3) var<uniform> tint: Tint;
                @fragment fn main() -> @location(0) vec4<f32> { return tint.colour; }
            ",
            &[(3, 16)],
        ),
    }
}

fn pass(stream: Option<usize>) -> super::super::Pass {
    let layout = |stride, format, shader_location| super::super::VertexLayout {
        stride,
        step: wgpu::VertexStepMode::Vertex,
        attributes: vec![wgpu::VertexAttribute {
            format,
            offset: 0,
            shader_location,
        }],
    };
    super::super::Pass {
        vertices: vec![
            layout(8, wgpu::VertexFormat::Sint16x4, 0),
            layout(4, wgpu::VertexFormat::Sint16x2, 1),
        ],
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        targets: Vec::new(),
        instanced: 0,
        stream,
        vertex_color_tint: None,
    }
}

const PER_INSTANCE: u32 = (1 << 2) | (1 << 3) | (1 << 4);

#[test]
fn a_skinned_program_of_two_streams_reads_its_rows_and_its_slab_per_instance() {
    let made = Instanced::new(&skinned(), PER_INSTANCE, &pass(Some(1))).unwrap();
    assert_eq!(
        made.rows,
        [(2, 64), (3, 16), (4, 1024)],
        "every bound block of the mask becomes a row of its own stride; the scene block does not"
    );
    assert!(
        made.pixel.is_some(),
        "the fragment stage reads a row, so the vertex stage carries the instance index"
    );
    assert!(made.stream.is_some());
    assert!(
        made.vertex
            .contains("replay_batch_rows.rows[0][2] + replay_batch_index"),
        "the node's row stands at the draw's base plus its instance: {}",
        made.vertex
    );
    assert!(
        made.vertex.contains(&format!(
            "replay_batch_stream_base[replay_batch_rows.rows[{}][{}] + replay_batch_instance]",
            STREAM_ROW / 4,
            STREAM_ROW % 4
        )),
        "the slab's first vertex comes from the instance's own base: {}",
        made.vertex
    );
    assert!(
        made.vertex
            .contains("replay_batch_stream[replay_batch_vertex + 0u]"),
        "the heights are read from storage at the instance's vertex: {}",
        made.vertex
    );
    assert!(
        !made.vertex.contains("@location(1)"),
        "the instanced stream is no longer fetched: {}",
        made.vertex
    );
    assert!(
        made.vertex.contains("@location(0)"),
        "the shared stream is still fetched: {}",
        made.vertex
    );
}

#[test]
fn rows_alone_leave_the_streams_and_the_fragment_stage_as_the_program_has_them() {
    let made = Instanced::new(&skinned(), (1 << 2) | (1 << 4), &pass(None)).unwrap();
    assert_eq!(made.rows, [(2, 64), (4, 1024)]);
    assert!(
        made.pixel.is_none(),
        "no fragment block is read per instance, so that stage is not rewritten"
    );
    assert!(made.stream.is_none());
    assert!(
        made.vertex.contains("@location(1)"),
        "both streams are still fetched: {}",
        made.vertex
    );
}

#[test]
fn each_program_the_rewrites_cannot_take_is_refused_by_name() {
    let refusal = |compiled: &Compiled, mask, stream| {
        Instanced::new(compiled, mask, &pass(stream)).err().expect("refused")
    };
    assert_eq!(
        refusal(&skinned(), 0, None),
        "the program reads nothing per instance"
    );
    assert_eq!(
        refusal(&skinned(), PER_INSTANCE, Some(0)),
        "only the last vertex stream may be read per instance"
    );
    let mut own = skinned();
    own.vertex.wgsl = own
        .vertex
        .wgsl
        .replace(
            "@vertex fn main(@location(0) place: vec4<i32>",
            "@vertex fn main(@builtin(instance_index) which: u32, @location(0) place: vec4<i32>",
        )
        .replace("palette.bones[place.z]", "palette.bones[which]");
    assert_eq!(
        refusal(&own, PER_INSTANCE, None),
        "the program reads the instance index of its own draw"
    );
    let mut packed = pass(Some(1));
    packed.vertices[1].attributes[0].format = wgpu::VertexFormat::Unorm8x4;
    assert_eq!(
        Instanced::new(&skinned(), PER_INSTANCE, &packed).err().expect("refused"),
        "a per-instance vertex stream supplies Unorm8x4"
    );
    let mut grouped = skinned();
    grouped.vertex.wgsl = "
            struct Scene { view: mat4x4<f32> }
            struct Node { world: mat4x4<f32> }
            struct In { @location(0) place: vec4<i32>, @location(1) height: vec2<i32> }
            @group(0) @binding(0) var<uniform> scene: Scene;
            @group(0) @binding(2) var<uniform> node: Node;
            @vertex fn main(input: In) -> @builtin(position) vec4<f32> {
                let local = vec4<f32>(f32(input.place.x), f32(input.height.x), 0.0, 1.0);
                return scene.view * (node.world * local);
            }
        "
    .into();
    grouped.vertex.buffers.retain(|buffer| buffer.binding != 4);
    assert_eq!(
        refusal(&grouped, PER_INSTANCE, Some(1)),
        "a vertex input struct beside a per-instance stream requires a native draw"
    );
}
