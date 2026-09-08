//! One actual hardware route with explicit fixture-native operands. This does
//! not establish current-game source controls, ModColor, library history or PCS3.
use super::PacketFixture;
use crate::compose::{
    ComposeOptions, ComposeReport, Compositor, ContainerRuntime, ContainerTransitionProfile,
};
use crate::container_sprite_clip::{DisplayZeroDesktopMode, EffectiveGuiScale, NativeClipGraphics};
use crate::container_sprite_geometry::SpriteTextureNormalization;
use crate::container_sprite_lifetime::{ContainerSpriteLifetimes, NativeGuiClock};
use crate::container_sprite_paint::{
    prepare_materialized_native_draws, NativeSpriteSourceOperands,
};
use crate::container_sprite_pass::{ContainerSpritePass, ContainerSpritePolicy};
use crate::container_sprite_resources::{
    OrdinarySpriteLoadSchedule, OrdinarySpriteTypeInitializationOperation, ViewerSpriteResources,
};
use crate::container_sprite_session::collect_container_sprite_materialization;
use crate::dds::RgbaImage;
use crate::gui::GuiElement;
use crate::runtime_graphics::ClipRect;
use crate::variant_admission::{DisplayMode, DisplayVariantKey, InputVariant, PixelExtent};
use std::collections::BTreeMap;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const GUI: &str = r#"guiTypes = {
    containerWindowType = {
        name = peaceconference_full_window
        position = { x = 0 y = 0 }
        size = { width = 1280 height = 720 }
        iconType = {
            name = actual_icon
            spriteType = test
            position = { x = 10 y = 20 }
        }
    }
}"#;

fn roots(fixture: &PacketFixture) -> (GuiElement, GuiElement) {
    let key = fixture
        .files
        .admitted_used_content_key(crate::peace_conference_ui::PEACE_CONFERENCE_GUI)
        .unwrap()
        .unwrap();
    let bytes = fixture.files.read_admitted_used_content(&key).unwrap();
    let document = crate::paradox_text::parse_bytes(&bytes);
    assert!(document.recoveries.is_empty());
    let mut roots = crate::gui::parse_gui_document(&document);
    assert_eq!(roots.len(), 1);
    let root = roots.pop().unwrap();
    assert_eq!(root.name, crate::peace_conference_ui::PEACE_CONFERENCE_ROOT);
    let wrapper_doc = crate::paradox_text::parse_str(
        "guiTypes = { containerWindowType = { name = fixture_wrapper size = { x = 1280 y = 720 } } }",
    );
    let mut wrapper = crate::gui::parse_gui_document(&wrapper_doc).pop().unwrap();
    wrapper.children.push(root.clone());
    (root, wrapper)
}

fn graphics() -> NativeClipGraphics {
    let profile = DisplayVariantKey {
        physical_viewport: PixelExtent {
            width: WIDTH,
            height: HEIGHT,
        },
        min_gui: PixelExtent {
            width: WIDTH,
            height: HEIGHT,
        },
        gui_scale_f32_bits: 1_f32.to_bits(),
        display_mode: DisplayMode::Windowed,
        localisation_language: "l_english".into(),
        input_variant: InputVariant::MouseKeyboard,
    };
    let effective = EffectiveGuiScale::after_graphics_settings_read(
        &profile,
        [WIDTH, HEIGHT],
        false,
        DisplayZeroDesktopMode::Unavailable,
    )
    .unwrap();
    let graphics =
        NativeClipGraphics::from_graphics_construction(effective, [WIDTH, HEIGHT]).unwrap();
    assert_eq!(graphics.logical_size, [WIDTH as i32, HEIGHT as i32]);
    graphics
}

fn stage(
    fixture: &PacketFixture,
    root: &GuiElement,
    wrapper: &GuiElement,
    runtime: &ContainerRuntime,
    graphics: &NativeClipGraphics,
    presented: &ContainerSpriteLifetimes,
    retained_resources: &ViewerSpriteResources,
    elapsed_ns: i64,
    mod_color: [f32; 4],
) -> (ContainerSpritePass, ViewerSpriteResources) {
    let materialized = collect_container_sprite_materialization(
        wrapper,
        &["fixture-actual-main-root".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &fixture.registry,
        &fixture.assets,
        None,
    );
    assert!(materialized.refusals.is_empty());
    assert_eq!(materialized.elements.len(), 1);
    assert_eq!(materialized.elements[0].display_child_path, [0, 0]);
    let mut pass = ContainerSpritePass::begin(
        materialized,
        presented,
        Some(NativeGuiClock::from_application_elapsed_nanoseconds(
            elapsed_ns,
        )),
        ContainerSpritePolicy {
            texture_normalization: Some(SpriteTextureNormalization::ActualDimensions),
            native_mxcsr: Some(0x1f80),
        },
    )
    .unwrap();
    let mut resources = retained_resources.clone();
    let draws = prepare_materialized_native_draws(
        &fixture.assets, &fixture.registry, root, runtime,
        pass.materialization(), &pass.lifetimes, &mut resources, Ok(graphics), Some(false),
        NativeSpriteSourceOperands {
            // Explicit test input, never filled from the viewer's MXCSR or
            // promoted into the production source-policy unknown branch.
            native_mxcsr: Some(0x1f80),
            mod_color: Some(mod_color.map(f32::to_bits)),
            engine_defines: Some(Vec::new()),
            renderer_creation_aa_request: Some(0),
            text_policy: Some(crate::container_sprite_fixed_states::NativeShaderStateTextPolicy::AsciiCaseFoldAndUppercase),
        },
    );
    assert_eq!(draws.len(), 1);
    assert!(draws[&vec![0, 0]].is_ok(), "{:?}", draws[&vec![0, 0]]);
    let original = draws[&vec![0, 0]].as_ref().unwrap();
    assert_eq!(original.geometry.authored_position.fixed_position, [10, 20]);
    pass.bind_native_draws(draws).unwrap();
    // This fixture supplies the reached native queue result explicitly. The
    // production root-clip authority remains a distinct owning contract.
    pass.bind_native_clips(BTreeMap::from([(
        vec![0, 0],
        Ok(Some(ClipRect {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: HEIGHT as i32,
        })),
    )]))
    .unwrap();
    (pass, resources)
}

#[test]
#[ignore = "requires actual D3D11 hardware; explicit fixture native operands, not current recording or source-policy admission"]
fn reached_compositor_uses_actual_gpu_and_preserves_published_pixels_on_refusal() {
    use map_viewer::native_ui::{NativeUiD3dExecutor, NativeUiTargetDesc};
    let fixture = PacketFixture::with_options_and_gui(1, true, Some(GUI));
    let (root, wrapper) = roots(&fixture);
    let graphics = graphics();
    let profile = ContainerTransitionProfile::from_root(&root).unwrap();
    let size = profile.size.size_pixels(
        graphics.logical_size,
        profile.base.pixels(graphics.logical_size),
    );
    let mut runtime = ContainerRuntime::construct(profile, graphics.logical_size, size);
    runtime.set_local_visibility(true);
    assert!(runtime.visible);
    assert_eq!(runtime.opacity(), 1.0);

    // Reuse the public hardware owner purely to obtain a real device/context.
    // These are actual test target allocations, not invented map draw records.
    let device_owner = map_viewer::NativeProductionD3dBackend::create_offscreen(
        map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 0,
            color_width: WIDTH,
            color_height: HEIGHT,
            color_format: 87,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 1,
            depth_width: WIDTH,
            depth_height: HEIGHT,
            depth_format: 40,
            depth_samples: 1,
            depth_quality: 0,
        },
    )
    .expect("actual D3D11 hardware is required");
    let actual = device_owner.desc();
    let target = NativeUiTargetDesc {
        width: actual.color_width,
        height: actual.color_height,
        dxgi_format_u32: actual.color_format,
        sample_count: actual.color_samples,
        sample_quality: actual.color_quality,
    };
    let mut executor = unsafe {
        NativeUiD3dExecutor::create_on_device(
            device_owner.device(),
            device_owner.context(),
            &fixture.files,
        )
    }
    .unwrap();
    let empty_resources = ViewerSpriteResources::new_viewer_session();
    let operation = OrdinarySpriteTypeInitializationOperation::fixture(
        &fixture.assets,
        &fixture.resources.requests,
        &fixture.records,
        1,
    );
    let retained_resources = empty_resources
        .stage_reached_type_initialization(
            &fixture.assets,
            &operation,
            &fixture.resources.requests,
            &fixture.records,
            Some(OrdinarySpriteLoadSchedule::Eager),
            Some(false),
            None,
        )
        .unwrap();
    assert_ne!(retained_resources, empty_resources);
    let mut options = ComposeOptions::new(WIDTH as usize, HEIGHT as usize);
    options
        .container_sprite_opacity_by_element
        .insert("actual_icon".into(), runtime.opacity());
    let background = RgbaImage {
        width: WIDTH as usize,
        height: HEIGHT as usize,
        pixels: [80, 120, 160, 200].repeat((WIDTH * HEIGHT) as usize),
    };
    let empty_lifetimes = ContainerSpriteLifetimes::default();
    let (mut pass, candidate_resources) = stage(
        &fixture,
        &root,
        &wrapper,
        &runtime,
        &graphics,
        &empty_lifetimes,
        &retained_resources,
        7_000_000_000,
        [0.25, 0.5, 0.75, 0.25],
    );
    assert_eq!(pass.created.len(), 1);
    let output = Compositor::with_container_sprite_pass(
        &fixture.assets,
        &fixture.registry,
        &mut pass,
        Some(crate::NativeUiPaintContext {
            resolver: &fixture.files,
            executor: &mut executor,
            target: target.clone(),
        }),
    )
    .compose_over(&wrapper, &options, &background)
    .unwrap();
    assert_eq!(
        output.report,
        ComposeReport {
            drawn_elements: 1,
            ..ComposeReport::default()
        }
    );
    assert_eq!(
        background.pixels,
        [80, 120, 160, 200].repeat((WIDTH * HEIGHT) as usize)
    );
    // Exact StaticButton sample [29,13,7,127]/255 times the supplied ModColor,
    // followed by one SRC_ALPHA/INV_SRC_ALPHA blend into [80,120,160,200].
    // The 2x2 original strip reaches only native pixels x10..11/y20..21.
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let at = (y * WIDTH as usize + x) * 4;
            let expected = if (10..12).contains(&x) && (20..22).contains(&y) {
                [71, 106, 141, 179]
            } else {
                [80, 120, 160, 200]
            };
            assert_eq!(
                &output.image.pixels[at..at + 4],
                &expected,
                "pixel({x},{y})"
            );
        }
    }
    let after_first = pass.lifetimes.clone();
    pass.begin_raster_replay();
    let replay = Compositor::with_container_sprite_pass(
        &fixture.assets,
        &fixture.registry,
        &mut pass,
        Some(crate::NativeUiPaintContext {
            resolver: &fixture.files,
            executor: &mut executor,
            target: target.clone(),
        }),
    )
    .compose_over(&wrapper, &options, &background)
    .unwrap();
    pass.finish_raster_replay().unwrap();
    assert_eq!(replay.report, output.report);
    assert_eq!(replay.image, output.image);
    assert_eq!(pass.lifetimes, after_first);
    let published_image = output.image;
    let published_lifetimes = pass.lifetimes;
    let published_resources = candidate_resources;

    // A real executor with another valid admission must reject this draw.
    // No mocked successful executor or synthetic readback is involved.
    let other = PacketFixture::with_static_shader(true);
    let mut wrong_executor = unsafe {
        NativeUiD3dExecutor::create_on_device(
            device_owner.device(),
            device_owner.context(),
            &other.files,
        )
    }
    .unwrap();
    let (mut failed_pass, failed_resources) = stage(
        &fixture,
        &root,
        &wrapper,
        &runtime,
        &graphics,
        &published_lifetimes,
        &published_resources,
        11_000_000_000,
        [0.25, 0.5, 0.75, 0.25],
    );
    assert!(failed_pass.created.is_empty());
    failed_pass
        .icon_operation(
            &[0, 0],
            "test",
            crate::container_sprite_interaction::IconOperation::Enable {
                disable_locked: false,
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
    let before_image = published_image.clone();
    let before_lifetimes = published_lifetimes.clone();
    let before_resources = published_resources.clone();
    let refused = Compositor::with_container_sprite_pass(
        &fixture.assets,
        &fixture.registry,
        &mut failed_pass,
        Some(crate::NativeUiPaintContext {
            resolver: &fixture.files,
            executor: &mut wrong_executor,
            target,
        }),
    )
    .compose_over(&wrapper, &options, &published_image)
    .unwrap();
    assert_eq!(refused.report.drawn_elements, 0);
    assert_eq!(refused.report.undecodable_sprites.len(), 1);
    assert!(
        refused.report.undecodable_sprites[0]
            .contains("native UI draw and executor admissions differ"),
        "{:?}",
        refused.report
    );
    assert_eq!(refused.image, before_image);
    assert_ne!(
        failed_pass.lifetimes, before_lifetimes,
        "failed frame has a distinct staged owning input operation"
    );
    assert_eq!(published_image, before_image);
    assert_eq!(published_lifetimes, before_lifetimes);
    assert_eq!(published_resources, before_resources);
    assert_eq!(failed_resources, before_resources);
}

#[test]
#[ignore = "requires actual D3D11 hardware; explicit fixture native operands, not current recording or source-policy admission"]
fn actual_gpu_uses_native_vertical_frame_offset_and_top_to_bottom_raster() {
    use map_viewer::native_ui::{NativeUiD3dExecutor, NativeUiTargetDesc};
    let gui = GUI.replace("spriteType = test", "spriteType = test frame = 2");
    let fixture = PacketFixture::with_vertical_frame_gui(&gui);
    let (root, wrapper) = roots(&fixture);
    let graphics = graphics();
    let profile = ContainerTransitionProfile::from_root(&root).unwrap();
    let size = profile.size.size_pixels(
        graphics.logical_size,
        profile.base.pixels(graphics.logical_size),
    );
    let mut runtime = ContainerRuntime::construct(profile, graphics.logical_size, size);
    runtime.set_local_visibility(true);
    assert!(runtime.visible);
    assert_eq!(runtime.opacity(), 1.0);

    // Reuse the public hardware owner purely to obtain a real device/context.
    // These are actual test target allocations, not invented map draw records.
    let device_owner = map_viewer::NativeProductionD3dBackend::create_offscreen(
        map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 0,
            color_width: WIDTH,
            color_height: HEIGHT,
            color_format: 87,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 1,
            depth_width: WIDTH,
            depth_height: HEIGHT,
            depth_format: 40,
            depth_samples: 1,
            depth_quality: 0,
        },
    )
    .expect("actual D3D11 hardware is required");
    let actual = device_owner.desc();
    let target = NativeUiTargetDesc {
        width: actual.color_width,
        height: actual.color_height,
        dxgi_format_u32: actual.color_format,
        sample_count: actual.color_samples,
        sample_quality: actual.color_quality,
    };
    let mut executor = unsafe {
        NativeUiD3dExecutor::create_on_device(
            device_owner.device(),
            device_owner.context(),
            &fixture.files,
        )
    }
    .unwrap();
    let empty_resources = ViewerSpriteResources::new_viewer_session();
    let operation = OrdinarySpriteTypeInitializationOperation::fixture(
        &fixture.assets,
        &fixture.resources.requests,
        &fixture.records,
        1,
    );
    let retained_resources = empty_resources
        .stage_reached_type_initialization(
            &fixture.assets,
            &operation,
            &fixture.resources.requests,
            &fixture.records,
            Some(OrdinarySpriteLoadSchedule::Eager),
            Some(false),
            None,
        )
        .unwrap();
    assert_ne!(retained_resources, empty_resources);
    let mut options = ComposeOptions::new(WIDTH as usize, HEIGHT as usize);
    options
        .container_sprite_opacity_by_element
        .insert("actual_icon".into(), runtime.opacity());
    let background = RgbaImage {
        width: WIDTH as usize,
        height: HEIGHT as usize,
        pixels: [80, 120, 160, 200].repeat((WIDTH * HEIGHT) as usize),
    };
    let (mut pass, _) = stage(
        &fixture,
        &root,
        &wrapper,
        &runtime,
        &graphics,
        &ContainerSpriteLifetimes::default(),
        &retained_resources,
        7_000_000_000,
        [1.0; 4],
    );
    let output = Compositor::with_container_sprite_pass(
        &fixture.assets,
        &fixture.registry,
        &mut pass,
        Some(crate::NativeUiPaintContext {
            resolver: &fixture.files,
            executor: &mut executor,
            target: target.clone(),
        }),
    )
    .compose_over(&wrapper, &options, &background)
    .unwrap();
    // ButtonState's authored numeric sampler operands still lack a production
    // source-conversion witness. Keep that refusal visible at the Compositor.
    assert_eq!(
        output.report,
        ComposeReport {
            undecodable_sprites: vec![
                "test: native sprite test: native sampler MapTexture MipMapLodBias source conversion unavailable".into(),
            ],
            ..ComposeReport::default()
        }
    );
    assert_eq!(output.image.pixels, background.pixels);

    // Exercise the same reached geometry/resources through the original packet
    // and actual GPU with explicitly supplied fixture sampler/fixed-state inputs.
    // This proves vertical packet mechanics, not ButtonState Compositor admission.
    use map_viewer::native_ui::NativeUiDrawExecutor;
    let prepared = pass.native_draw(&[0, 0]).unwrap();
    let request = crate::runtime_graphics::ContainerSpriteRequest {
        source_identity: fixture.request.source_identity.clone(),
        frame_count: fixture.request.frame_count,
        frame: crate::runtime_graphics::ContainerSpriteFrame::NativeIcon(
            crate::container_static_icon_geometry::native_static_icon_frame(&root.children[0])
                .unwrap(),
        ),
        interaction: None,
        color: crate::runtime_graphics::RgbaF32([1.0; 4]),
        opacity: runtime.opacity(),
        clip: ClipRect {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: HEIGHT as i32,
        },
        reached: Some(crate::runtime_graphics::ContainerSpriteReachedState {
            effect: crate::container_sprite_inputs::NativeButtonEffect::Unassigned,
            ..fixture.request.reached.unwrap()
        }),
    };
    let matrix: [u32; 16] = prepared
        .geometry
        .upload
        .rows()
        .map(|r| r.map(f32::to_bits))
        .concat()
        .try_into()
        .unwrap();
    let viewport = portable_render_stream::ViewportDesc {
        width_f32_bits: (WIDTH as f32).to_bits(),
        height_f32_bits: (HEIGHT as f32).to_bits(),
        ..fixture.viewport.clone()
    };
    let packet = super::ContainerSpriteDrawPacket::build(
        &fixture.assets,
        &fixture.files,
        &fixture.sprite,
        &request,
        super::ContainerSpriteGpuInputs {
            local_positions: &prepared.geometry.local_positions,
            uv: &prepared.geometry.uv_geometry,
            world_view_projection_bits: &matrix,
            engine_defines: Some(&[]),
            native_mxcsr: Some(0x1f80),
            resources: &prepared.resources,
            pipeline: &fixture.pipeline,
            viewport: &viewport,
            scissor: None,
            target: &target,
        },
    )
    .unwrap();
    let admitted = packet.prepare(&fixture.files).unwrap();
    let mut image = background.clone();
    executor
        .draw_over(
            &admitted,
            map_viewer::native_ui::UiRgbaSurfaceMut {
                width: WIDTH,
                height: HEIGHT,
                pixels: &mut image.pixels,
            },
        )
        .unwrap();
    // The admitted 1x4 texture has red/green/blue/white rows. Native frame 2
    // selects rows 2/3: blue at screen top, white below, exactly one pixel wide.
    // This preserves the removed CPU route's vertical-frame expectation through
    // the original vertex shader, hardware sampling and actual GPU readback.
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let at = (y * WIDTH as usize + x) * 4;
            let expected = match (x, y) {
                (10, 20) => [0, 0, 255, 255],
                (10, 21) => [255, 255, 255, 255],
                _ => [80, 120, 160, 200],
            };
            assert_eq!(&image.pixels[at..at + 4], &expected, "pixel({x},{y})");
        }
    }
    assert_eq!(
        background.pixels,
        [80, 120, 160, 200].repeat((WIDTH * HEIGHT) as usize)
    );
}
