//! Parses and validates every map shader with naga, without a device or game
//! content, so a WGSL error fails `cargo test` instead of the first render.
use eframe::wgpu::naga;

/// Several WARP devices used at once in one process fault inside `d3d10warp.dll` (probed
/// 2026-09-26: 5 of 6 runs with three concurrent devices, 0 of 6 with one device shared by three
/// threads). The viewer creates one device per process, so only tests need this. Every GPU test
/// takes it before requesting its adapter, so no second device exists while another test runs.
pub(super) static GPU_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
}

#[test]
fn standalone_map_shaders_validate() {
    for (name, source) in [
        ("political_fill", include_str!("political_fill.wgsl")),
        ("post", include_str!("post.wgsl")),
        ("post_luts", include_str!("post_luts.wgsl")),
        ("shadow", include_str!("shadow.wgsl")),
        ("sprites", super::sprites::SHADER),
    ] {
        validate(name, source);
    }
}

#[test]
fn world_shaders_validate_with_shared_frame_inputs() {
    for (name, source) in [
        ("border", include_str!("border.wgsl")),
        ("maparrow", include_str!("maparrow.wgsl")),
        ("mapframe", include_str!("mapframe.wgsl")),
        ("mapname", include_str!("mapname.wgsl")),
        ("pdxmesh", include_str!("pdxmesh.wgsl")),
        ("railroad", include_str!("railroad.wgsl")),
        ("river", include_str!("river.wgsl")),
        ("terrain", include_str!("terrain.wgsl")),
        ("tree", include_str!("tree.wgsl")),
        ("water", include_str!("water.wgsl")),
        ("weather_effects", include_str!("weather_effects.wgsl")),
    ] {
        // This is the same composition used by scene::module.
        validate(name, &format!("{}\n{source}", include_str!("common.wgsl")));
    }
}

#[test]
fn arrow_normals_and_second_shadow_run_the_native_shader_laws() {
    use eframe::wgpu;
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("SKIP arrow shader oracle: no adapter");
        return;
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&crate::render::device_descriptor(&adapter)))
            .unwrap();
    let source = format!(
        "{}\n{}\n{}",
        include_str!("common.wgsl"),
        include_str!("maparrow.wgsl"),
        r#"
@group(3) @binding(0) var<storage, read_write> result: array<vec4<f32>, 5>;
@compute @workgroup_size(1)
fn oracle() {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let slope = vec3<f32>(0.6, 0.8, 0.0);
    result[0] = vec4<f32>(arrow_blend_normals(up, slope, up, 0.0), 1.0);
    result[1] = vec4<f32>(arrow_blend_normals(slope, slope, up, 1.0), 1.0);
    let color = vec3<f32>(0.8, 0.4, 0.2);
    let lit = vec3<f32>(0.4, 0.2, 0.0);
    result[2] = vec4<f32>(arrow_lit_color(color, lit, false, 0.3), 1.0);
    result[3] = vec4<f32>(arrow_lit_color(color, lit, true, 0.3), 1.0);
    result[4] = vec4<f32>(arrow_lit_color(color, lit, false, 1.0), 1.0);
}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("arrow shader oracle"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[None, None, None, Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("oracle"),
        compilation_options: Default::default(),
        cache: None,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 80,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 80,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(3, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, 80);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = read.slice(..).get_mapped_range().unwrap();
    let values: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
    for (actual, expected) in values.iter().zip([
        [0.6, 0.8, 0.0, 1.0],    // Tile normal perturbs a flat land surface.
        [0.0, 1.0, 0.0, 1.0],    // Water discards both land topology and tile normal.
        [0.18, 0.09, 0.03, 1.0], // No-head: shadow multiplies the entire lighting blend.
        [0.6, 0.3, 0.1, 1.0],    // Head: no second shadow.
        [0.6, 0.3, 0.1, 1.0],    // No shadow leaves no-head unchanged.
    ]) {
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.00001,
                "{actual} != {expected}"
            );
        }
    }
}
