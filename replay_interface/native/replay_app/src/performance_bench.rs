//! F1 production headless performance acceptance.
//!
//! The recording profile opens one admitted portable recording, resolves its
//! exact selected frame, and repeatedly executes XMAP22's shared packet-native
//! product compositor with GPU completion in the timed interval. The default F1 profile
//! instead runs a contract-pinned synthetic stress packet through isolated GPU
//! stress adapters so performance acceptance has no future-capture dependency.
//! Stress receipts explicitly prohibit semantic, pixel, RE, and rendering-law
//! claims. Both profiles measure GPU-completed 1920x1080 frames and reject
//! queued CPU-submit time, debug bundles, or omissions; only the recording
//! profile is evidence about the actual production map composition.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use winapi::um::d3dcommon::ID3DBlob;

use crate::app::ProductProductionD3dRuntime;
use crate::d3d::{Gfx, GpuAdapterIdentity};
use crate::timeline::{resolve_dump_hour, DumpHourRequest, OwnerProvider};
use crate::ui_runtime::ProductSelectedPresentPerformanceRuntime;
use crate::{Result, TimelineMode};

const REQUIRED_WIDTH: u32 = 1920;
const REQUIRED_HEIGHT: u32 = 1080;
const MIN_WARMUP_FRAMES: usize = 60;
const MIN_SAMPLE_FRAMES: usize = 300;
const DEFAULT_WARMUP_FRAMES: usize = 120;
const DEFAULT_SAMPLE_FRAMES: usize = 600;
const MAX_AVERAGE_FRAME_MS: f64 = 1000.0 / 60.0;
const MAX_P95_FRAME_MS: f64 = 33.3;

#[derive(Debug)]
pub(crate) struct BenchOptions {
    pub(crate) recording: PathBuf,
    pub(crate) frame_cache_entries: usize,
    pub(crate) warmup_frames: usize,
    pub(crate) sample_frames: usize,
    pub(crate) workload_contract: PathBuf,
    pub(crate) output: PathBuf,
}

#[derive(Debug)]
pub(crate) struct StressBenchOptions {
    pub(crate) warmup_frames: usize,
    pub(crate) sample_frames: usize,
    pub(crate) workload_contract: PathBuf,
    pub(crate) output: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StressWorkloadContract {
    schema: String,
    certification_enabled: bool,
    workload_id: String,
    proof_scope: String,
    prohibited_claims: Vec<String>,
    viewport_width: u32,
    viewport_height: u32,
    map_operation_count: usize,
    selected_present_ui_operation_count: usize,
    map_count_authority: StressCountAuthority,
    ui_count_authority: StressCountAuthority,
    shader_asset_path: String,
    shader_asset_sha256: String,
    synthetic_product_modset_sha256: String,
    synthetic_effective_vfs_manifest_sha256: String,
    ui_surface_algorithm: String,
    ui_surface_sha256: String,
    approved_omissions: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StressCountAuthority {
    repository_relative_path: String,
    sha256: String,
    json_pointer: String,
    expected_count: usize,
}

#[derive(Debug, Serialize)]
struct StressWorkloadReceipt {
    workload_contract_path: String,
    workload_contract_sha256: String,
    workload_id: String,
    proof_scope: String,
    prohibited_claims: Vec<String>,
    map_count_authority: StressCountAuthority,
    ui_count_authority: StressCountAuthority,
    shader_asset_path: String,
    shader_asset_sha256: String,
    compiled_vertex_shader_sha256: String,
    compiled_pixel_shader_sha256: String,
    content_manifest_sha256: String,
    product_modset_sha256: String,
    effective_vfs_manifest_sha256: String,
    prepared_frame_sha256: String,
    production_d3d_receipt_sha256: String,
    ui_surface_algorithm: String,
    ui_surface_sha256: String,
    viewport_width: u32,
    viewport_height: u32,
    gpu_adapter_name: String,
    gpu_dedicated_video_memory_bytes: u64,
    gpu_vendor_id: u32,
    gpu_device_id: u32,
    warmup_frames: usize,
    sample_frames: usize,
    map_operations_per_frame: usize,
    selected_present_operations_per_frame: usize,
    feature_local_omissions: usize,
    gpu_completion_method: &'static str,
    timer: &'static str,
}

#[derive(Debug, Serialize)]
struct StressBenchReceipt {
    schema: &'static str,
    result: &'static str,
    workload: StressWorkloadReceipt,
    reference_machine_contract: &'static str,
    thresholds: Thresholds,
    metrics: Metrics,
    failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RecordingSelectionEnvelope {
    logical_stream_id: Option<u64>,
    selected_hour: i64,
}

#[derive(Debug, Serialize)]
struct RecordingBenchReceipt {
    schema: &'static str,
    result: &'static str,
    recording_path: String,
    recording_identity_before_sha256: String,
    recording_identity_after_sha256: String,
    opened_recording_segment_trust_sha256: String,
    logical_stream_id: Option<u64>,
    selected_hour: i64,
    viewport_width: u32,
    viewport_height: u32,
    gpu_adapter_name: String,
    gpu_dedicated_video_memory_bytes: u64,
    gpu_vendor_id: u32,
    gpu_device_id: u32,
    warmup_frames: usize,
    sample_frames: usize,
    gpu_completion_method: &'static str,
    timer: &'static str,
    last_composition: crate::app::ProductMapCompositionReceipt,
    thresholds: Thresholds,
    metrics: Metrics,
    failures: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Thresholds {
    average_fps_min: f64,
    average_frame_time_ms_max: f64,
    p95_frame_time_ms_max: f64,
}

#[derive(Debug, Serialize)]
struct Metrics {
    sample_count: usize,
    percentile_method: &'static str,
    average_frame_time_ms: f64,
    average_fps: f64,
    p50_frame_time_ms: f64,
    p95_frame_time_ms: f64,
    p99_frame_time_ms: f64,
    max_frame_time_ms: f64,
    frame_times_ms: Vec<f64>,
}

#[derive(Clone, Copy)]
struct FrameCounts {
    map_operations: usize,
    selected_present_operations: usize,
    selected_present_m02_draws: usize,
    observer_ui_draws: usize,
}

fn advance_recording_benchmark_viewer_clock(
    viewer: &mut map_viewer::ViewerOwnedMapState,
    delta_seconds: f32,
) -> Result<()> {
    let elapsed = f32::from_bits(viewer.animation_time_f32_bits);
    if !elapsed.is_finite() || !delta_seconds.is_finite() || delta_seconds < 0.0 {
        return Err("recording benchmark viewer clock requires finite nonnegative time".into());
    }
    let next = elapsed + delta_seconds;
    if !next.is_finite() {
        return Err("recording benchmark viewer clock overflowed".into());
    }
    viewer.animation_time_f32_bits = next.to_bits();
    viewer.frame_delta_seconds_f32_bits = delta_seconds.to_bits();
    Ok(())
}

pub(crate) fn run(
    options: BenchOptions,
    product_paths: &crate::install_paths::ProductPaths,
) -> Result<()> {
    if options.warmup_frames < MIN_WARMUP_FRAMES {
        return Err(format!("--benchmark-warmup-frames must be >= {MIN_WARMUP_FRAMES}").into());
    }
    if options.sample_frames < MIN_SAMPLE_FRAMES {
        return Err(format!("--benchmark-sample-frames must be >= {MIN_SAMPLE_FRAMES}").into());
    }
    let recording_identity_before_sha256 = recording_identity_sha256(&options.recording)?;
    let selection_bytes = std::fs::read(&options.workload_contract)?;
    let selection: RecordingSelectionEnvelope = serde_json::from_slice(&selection_bytes)?;
    let mode = TimelineMode::Recording {
        path: options.recording.clone(),
        logical_stream_id: selection.logical_stream_id,
        timeline: replay_core::TimelineConfig {
            frame_cache_entries: options.frame_cache_entries,
        },
    };
    let (provider, opened_recording_segment_trust_sha256) = open_provider(&mode)?;
    let (resolver, content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let manifest = provider
        .content_manifest()
        .ok_or("performance benchmark recording has no admitted content manifest")?;
    let selected_hour = resolve_dump_hour(
        provider.hours(),
        DumpHourRequest::Canonical(selection.selected_hour),
    )?;
    let frame = provider
        .world_frame_at(selected_hour)?
        .ok_or_else(|| map_viewer::refuse_missing_xmap00_authority("world_frame"))?;
    let (map_width, map_height) =
        map_viewer::foundation_map_dimensions(&resolver, &content_admission)?;
    let camera = map_viewer::ViewerMapCameraState::from_admitted_content(
        &resolver,
        &content_admission,
        map_width,
        map_height,
        REQUIRED_WIDTH,
        REQUIRED_HEIGHT,
        map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
    )?;
    let mut viewer = map_viewer::ViewerOwnedMapState {
        viewport_width: REQUIRED_WIDTH,
        viewport_height: REQUIRED_HEIGHT,
        input_revision: camera.input_revision,
        camera: Some(camera),
        observer_country_id: None,
        map_mode_id: map_viewer::ViewerMapModeSelection::default().viewer_token(),
        map_mode_selection_epoch: 0,
        selected_object_ids: Vec::new(),
        animation_time_f32_bits: 0.0f32.to_bits(),
        frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
    };
    let terrain_options =
        crate::install_paths::playback_terrain_lod_options_from_paths(product_paths)?;
    let mut composition = crate::app::ProductMapCompositionSession::start_for_benchmark(
        &resolver,
        &content_admission,
        manifest,
        terrain_options,
        REQUIRED_WIDTH,
        REQUIRED_HEIGHT,
    )?;
    let ui_profile = crate::install_paths::playback_ui_profile_from_paths(
        product_paths,
        &resolver,
        crate::win32::actual_client_input_variant(),
    )?;
    let xmap15_selection = map_viewer::xmap15_viewer_selection(&frame, None);
    let gpu = composition.adapter_identity()?;
    validate_reference_gpu(&gpu)?;
    let mut render_one = |composition: &mut crate::app::ProductMapCompositionSession| {
        advance_recording_benchmark_viewer_clock(&mut viewer, 1.0 / 60.0)?;
        composition.execute(
            &resolver,
            &content_admission,
            manifest,
            &frame,
            &viewer,
            &xmap15_selection,
            ui_profile.localisation_language(),
            1.0 / 60.0,
            false,
            false,
            ui_profile.key().gui_scale(),
        )
    };
    let mut last_composition = None;
    for _ in 0..options.warmup_frames {
        last_composition = Some(render_one(&mut composition)?);
    }
    let mut samples_ms = Vec::with_capacity(options.sample_frames);
    for _ in 0..options.sample_frames {
        let started = Instant::now();
        last_composition = Some(render_one(&mut composition)?);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return Err("recording benchmark observed an invalid frame duration".into());
        }
        samples_ms.push(elapsed_ms);
    }
    let metrics = summarize(&samples_ms)?;
    let mut failures = Vec::new();
    if metrics.average_fps < 60.0 {
        failures.push(format!(
            "average_fps {:.6} is below 60.0",
            metrics.average_fps
        ));
    }
    if metrics.average_frame_time_ms > MAX_AVERAGE_FRAME_MS {
        failures.push(format!(
            "average_frame_time_ms {:.6} exceeds {:.6}",
            metrics.average_frame_time_ms, MAX_AVERAGE_FRAME_MS
        ));
    }
    if metrics.p95_frame_time_ms > MAX_P95_FRAME_MS {
        failures.push(format!(
            "p95_frame_time_ms {:.6} exceeds {:.6}",
            metrics.p95_frame_time_ms, MAX_P95_FRAME_MS
        ));
    }
    let recording_identity_after_sha256 = recording_identity_sha256(&options.recording)?;
    if recording_identity_after_sha256 != recording_identity_before_sha256 {
        failures.push(
            "recording artifact identity changed between pre-open admission and post-sample verification"
                .into(),
        );
    }
    let result = if failures.is_empty() { "pass" } else { "fail" };
    let receipt = RecordingBenchReceipt {
        schema: "hoi4.replay-map-recording-benchmark.xmap22.v1",
        result,
        recording_path: options.recording.display().to_string(),
        recording_identity_before_sha256,
        recording_identity_after_sha256,
        opened_recording_segment_trust_sha256,
        logical_stream_id: selection.logical_stream_id,
        selected_hour,
        viewport_width: REQUIRED_WIDTH,
        viewport_height: REQUIRED_HEIGHT,
        gpu_adapter_name: gpu.name,
        gpu_dedicated_video_memory_bytes: gpu.dedicated_video_memory_bytes,
        gpu_vendor_id: gpu.vendor_id,
        gpu_device_id: gpu.device_id,
        warmup_frames: options.warmup_frames,
        sample_frames: options.sample_frames,
        gpu_completion_method: "D3D11_QUERY_EVENT_inside_shared_compositor",
        timer: "std::time::Instant_wall_clock",
        last_composition: last_composition.ok_or("recording benchmark executed no frames")?,
        thresholds: Thresholds {
            average_fps_min: 60.0,
            average_frame_time_ms_max: MAX_AVERAGE_FRAME_MS,
            p95_frame_time_ms_max: MAX_P95_FRAME_MS,
        },
        metrics,
        failures,
    };
    let mut bytes = serde_json::to_vec_pretty(&receipt)?;
    bytes.push(b'\n');
    crate::headless::write_create_new_or_equal(
        &options.output,
        &bytes,
        "XMAP22 recording benchmark receipt",
    )?;
    if result != "pass" {
        return Err("XMAP22 recording benchmark thresholds failed; see receipt".into());
    }
    Ok(())
}

/// Run the default F1 gate without a replay recording. The workload is an
/// explicitly synthetic, performance-only stress packet: count authority comes
/// from retained engine observations, but neither its pixels nor its operation
/// identities are admissible as replay, rendering-law, or promotion evidence.
pub(crate) fn run_stress(options: StressBenchOptions) -> Result<()> {
    if options.warmup_frames < MIN_WARMUP_FRAMES {
        return Err(format!("--benchmark-warmup-frames must be >= {MIN_WARMUP_FRAMES}").into());
    }
    if options.sample_frames < MIN_SAMPLE_FRAMES {
        return Err(format!("--benchmark-sample-frames must be >= {MIN_SAMPLE_FRAMES}").into());
    }
    if options.output.exists() {
        return Err(format!(
            "performance receipt already exists; refusing to replace {}",
            options.output.display()
        )
        .into());
    }

    let contract_bytes = std::fs::read(&options.workload_contract)?;
    let contract_sha256 = sha256_hex(&contract_bytes);
    let contract: StressWorkloadContract = serde_json::from_slice(&contract_bytes)?;
    validate_stress_contract(&contract)?;
    let contract_root = options
        .workload_contract
        .parent()
        .ok_or("stress workload contract has no parent directory")?;
    let repository_root = contract_root.join("../../..").canonicalize()?;
    validate_count_authority(&repository_root, &contract.map_count_authority)?;
    validate_count_authority(&repository_root, &contract.ui_count_authority)?;

    let shader_path = checked_descendant(contract_root, &contract.shader_asset_path)?;
    let shader_bytes = std::fs::read(&shader_path)?;
    require_eq(
        "stress shader asset SHA-256",
        &sha256_hex(&shader_bytes),
        &contract.shader_asset_sha256,
    )?;
    let shader_source =
        std::str::from_utf8(&shader_bytes).map_err(|_| "stress shader asset is not UTF-8")?;
    let (vertex_shader, pixel_shader) = compile_stress_shaders(shader_source)?;
    let vertex_shader_sha256 = sha256_hex(&vertex_shader);
    let pixel_shader_sha256 = sha256_hex(&pixel_shader);

    let source = stress_used_content(&contract.shader_asset_path, &shader_bytes, &contract)?;
    let mut prepared = build_stress_map_frame(
        &contract,
        source.clone(),
        vertex_shader,
        pixel_shader,
        &"0".repeat(64),
    )?;
    let program_payload_sha256 = prepared
        .ordered_static_programs
        .first()
        .ok_or("performance stress frame has no static program")?
        .payload_sha256
        .clone();
    let expected_consumer = format!("production_map.static_program/{program_payload_sha256}");
    let (resolver, manifest) =
        build_stress_content(contract_root, &contract, &source, &expected_consumer)?;
    prepared.content_manifest_sha256 = manifest
        .manifest_digest_sha256
        .clone()
        .ok_or("stress content manifest has no digest")?;
    if !prepared.omissions.is_empty() {
        return Err("performance stress frame contains omissions".into());
    }
    let prepared_frame_sha256 = sha256_hex(&serde_json::to_vec(&prepared)?);

    let gfx = Gfx::create_offscreen(REQUIRED_WIDTH, REQUIRED_HEIGHT)?;
    let gpu_adapter = gfx.adapter_identity()?;
    validate_reference_gpu(&gpu_adapter)?;
    let gpu_completion = gfx.create_gpu_completion_query()?;
    let mut map_runtime = ProductProductionD3dRuntime::default();
    let surface = stress_ui_surface()?;
    require_eq(
        "stress UI surface SHA-256",
        &sha256_hex(&surface.pixels),
        &contract.ui_surface_sha256,
    )?;
    let ui_runtime = ProductSelectedPresentPerformanceRuntime::new(&gfx, &surface)?;
    let resources = BTreeMap::from([(
        "f1-performance-stress-shader".into(),
        map_viewer::ProductionMapResource {
            content_key: "f1-performance-stress-shader".into(),
            kind: replay_core::RequiredContentKind::ShaderSource,
            content: source,
            bytes: shader_bytes,
            consumers: vec![expected_consumer],
        },
    )]);

    let render_one =
        |map_runtime: &mut ProductProductionD3dRuntime| -> Result<(FrameCounts, String)> {
            gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;
            let map = map_runtime.execute(&gfx, &resolver, &resources, &prepared)?;
            if !map.presented_to_app_target || map.executed_operations == 0 {
                return Err(
                    "performance stress map route produced no app-target operations".into(),
                );
            }
            let ui_operations =
                ui_runtime.execute(&gfx, contract.selected_present_ui_operation_count)?;
            gpu_completion.wait(&gfx)?;
            Ok((
                FrameCounts {
                    map_operations: map.executed_operations,
                    selected_present_operations: ui_operations,
                    selected_present_m02_draws: 0,
                    observer_ui_draws: 0,
                },
                map.receipt_sha256,
            ))
        };

    let mut stable_counts = None;
    let mut production_d3d_receipt_sha256 = None;
    for _ in 0..options.warmup_frames {
        let (counts, receipt_sha256) = render_one(&mut map_runtime)?;
        require_stable_counts(&mut stable_counts, counts)?;
        require_stable_identity(
            &mut production_d3d_receipt_sha256,
            receipt_sha256,
            "production D3D execution receipt",
        )?;
    }
    let mut samples_ms = Vec::with_capacity(options.sample_frames);
    for _ in 0..options.sample_frames {
        let started = Instant::now();
        let (counts, receipt_sha256) = render_one(&mut map_runtime)?;
        require_stable_counts(&mut stable_counts, counts)?;
        require_stable_identity(
            &mut production_d3d_receipt_sha256,
            receipt_sha256,
            "production D3D execution receipt",
        )?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return Err("performance stress observed an invalid frame duration".into());
        }
        samples_ms.push(elapsed_ms);
    }

    let counts = stable_counts.ok_or("performance stress collected no frame counts")?;
    if counts.map_operations != contract.map_operation_count
        || counts.selected_present_operations != contract.selected_present_ui_operation_count
        || counts.selected_present_m02_draws != 0
    {
        return Err("performance stress operation counts differ from contract".into());
    }
    let metrics = summarize(&samples_ms)?;
    let mut failures = Vec::new();
    if metrics.average_fps < 60.0 {
        failures.push(format!(
            "average_fps {:.6} is below 60.0",
            metrics.average_fps
        ));
    }
    if metrics.average_frame_time_ms > MAX_AVERAGE_FRAME_MS {
        failures.push(format!(
            "average_frame_time_ms {:.6} exceeds {:.6}",
            metrics.average_frame_time_ms, MAX_AVERAGE_FRAME_MS
        ));
    }
    if metrics.p95_frame_time_ms > MAX_P95_FRAME_MS {
        failures.push(format!(
            "p95_frame_time_ms {:.6} exceeds {:.6}",
            metrics.p95_frame_time_ms, MAX_P95_FRAME_MS
        ));
    }
    let result = if failures.is_empty() { "pass" } else { "fail" };
    let receipt = StressBenchReceipt {
        schema: "battle-plan-headless-performance-stress-bench-v1",
        result,
        workload: StressWorkloadReceipt {
            workload_contract_path: options.workload_contract.display().to_string(),
            workload_contract_sha256: contract_sha256,
            workload_id: contract.workload_id,
            proof_scope: contract.proof_scope,
            prohibited_claims: contract.prohibited_claims,
            map_count_authority: contract.map_count_authority,
            ui_count_authority: contract.ui_count_authority,
            shader_asset_path: shader_path.display().to_string(),
            shader_asset_sha256: contract.shader_asset_sha256,
            compiled_vertex_shader_sha256: vertex_shader_sha256,
            compiled_pixel_shader_sha256: pixel_shader_sha256,
            content_manifest_sha256: manifest
                .manifest_digest_sha256
                .ok_or("stress content manifest digest disappeared")?,
            product_modset_sha256: contract.synthetic_product_modset_sha256,
            effective_vfs_manifest_sha256: contract.synthetic_effective_vfs_manifest_sha256,
            prepared_frame_sha256,
            production_d3d_receipt_sha256: production_d3d_receipt_sha256
                .ok_or("performance stress produced no D3D receipt")?,
            ui_surface_algorithm: contract.ui_surface_algorithm,
            ui_surface_sha256: contract.ui_surface_sha256,
            viewport_width: REQUIRED_WIDTH,
            viewport_height: REQUIRED_HEIGHT,
            gpu_adapter_name: gpu_adapter.name,
            gpu_dedicated_video_memory_bytes: gpu_adapter.dedicated_video_memory_bytes,
            gpu_vendor_id: gpu_adapter.vendor_id,
            gpu_device_id: gpu_adapter.device_id,
            warmup_frames: options.warmup_frames,
            sample_frames: options.sample_frames,
            map_operations_per_frame: counts.map_operations,
            selected_present_operations_per_frame: counts.selected_present_operations,
            feature_local_omissions: 0,
            gpu_completion_method: "D3D11_QUERY_EVENT_after_each_frame",
            timer: "std::time::Instant_wall_clock",
        },
        reference_machine_contract:
            "i5-8400_or_Ryzen_5_2600;16_GiB;GTX_1060_6_GiB_or_RX_580_8_GiB;1920x1080",
        thresholds: Thresholds {
            average_fps_min: 60.0,
            average_frame_time_ms_max: MAX_AVERAGE_FRAME_MS,
            p95_frame_time_ms_max: MAX_P95_FRAME_MS,
        },
        metrics,
        failures,
    };
    write_stress_receipt(&options.output, &receipt)?;
    if result != "pass" {
        return Err("headless performance stress thresholds failed; see receipt".into());
    }
    Ok(())
}

fn open_provider(mode: &TimelineMode) -> Result<(Arc<dyn OwnerProvider>, String)> {
    let TimelineMode::Recording {
        path,
        logical_stream_id,
        timeline,
    } = mode
    else {
        return Err("performance benchmark requires a portable recording".into());
    };
    let opened = crate::recording_open::open_product_recording_segment_with_authority(
        path,
        *logical_stream_id,
    )
    .map_err(|error| format!("open benchmark recording {}: {error}", path.display()))?;
    let segment_trust_sha256 = opened.authority_sha256.ok_or(
        "performance benchmark recording has no exact retained-container authority receipt",
    )?;
    let (provider, _) = crate::timeline::build_recording_provider(
        None,
        opened.source,
        *timeline,
        &path.display().to_string(),
    )?;
    Ok((provider, segment_trust_sha256))
}

fn require_eq(label: &str, actual: &str, expected: &str) -> Result<()> {
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!("performance {label} differs from the checked-in contract").into())
    }
}

fn validate_reference_gpu(adapter: &GpuAdapterIdentity) -> Result<()> {
    const NVIDIA_VENDOR: u32 = 0x10DE;
    const GTX_1060_DEVICE: u32 = 0x1C03;
    const AMD_VENDOR: u32 = 0x1002;
    const RX_580_DEVICE: u32 = 0x67DF;
    const GTX_1060_MIN_BYTES: u64 = 6 * 1024 * 1024 * 1024;
    const RX_580_MIN_BYTES: u64 = 8 * 1024 * 1024 * 1024;
    let required = match (adapter.vendor_id, adapter.device_id) {
        (NVIDIA_VENDOR, GTX_1060_DEVICE) => GTX_1060_MIN_BYTES,
        (AMD_VENDOR, RX_580_DEVICE) => RX_580_MIN_BYTES,
        _ => {
            return Err(format!(
                "selected DXGI adapter is not the locked desktop GTX 1060/RX 580 class: vendor 0x{:04X}, device 0x{:04X}",
                adapter.vendor_id, adapter.device_id
            )
            .into())
        }
    };
    if adapter.dedicated_video_memory_bytes != required {
        return Err(format!(
            "selected DXGI adapter has {} dedicated bytes; locked model requires exactly {required}",
            adapter.dedicated_video_memory_bytes
        )
        .into());
    }
    Ok(())
}

fn require_stable_counts(stable: &mut Option<FrameCounts>, current: FrameCounts) -> Result<()> {
    match stable {
        Some(expected)
            if expected.map_operations != current.map_operations
                || expected.selected_present_operations != current.selected_present_operations
                || expected.selected_present_m02_draws != current.selected_present_m02_draws
                || expected.observer_ui_draws != current.observer_ui_draws =>
        {
            Err("performance benchmark operation counts changed between frames".into())
        }
        Some(_) => Ok(()),
        None => {
            *stable = Some(current);
            Ok(())
        }
    }
}

fn summarize(samples: &[f64]) -> Result<Metrics> {
    if samples.is_empty()
        || samples
            .iter()
            .any(|sample| !sample.is_finite() || *sample <= 0.0)
    {
        return Err("performance benchmark metrics are absent or invalid".into());
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let average = sorted.iter().sum::<f64>() / sorted.len() as f64;
    Ok(Metrics {
        sample_count: sorted.len(),
        percentile_method: "nearest_rank_sorted_ceil_q_times_n",
        average_frame_time_ms: average,
        average_fps: 1000.0 / average,
        p50_frame_time_ms: percentile_nearest_rank(&sorted, 0.50),
        p95_frame_time_ms: percentile_nearest_rank(&sorted, 0.95),
        p99_frame_time_ms: percentile_nearest_rank(&sorted, 0.99),
        max_frame_time_ms: *sorted.last().ok_or("performance samples disappeared")?,
        frame_times_ms: samples.to_vec(),
    })
}

fn percentile_nearest_rank(sorted: &[f64], quantile: f64) -> f64 {
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn validate_stress_contract(contract: &StressWorkloadContract) -> Result<()> {
    let required_prohibited_claims = [
        "replay_semantic_parity",
        "rendering_law_promotion",
        "reverse_engineering_evidence",
        "pixel_parity",
    ];
    if contract.schema != "battle-plan-headless-performance-stress-workload-v1"
        || !contract.certification_enabled
        || contract.workload_id != "F1_A729_1920X1080_PERFORMANCE_STRESS_V1"
        || contract.proof_scope != "performance_only_non_semantic_non_promotion"
        || contract.prohibited_claims
            != required_prohibited_claims.map(str::to_string)
        || contract.viewport_width != REQUIRED_WIDTH
        || contract.viewport_height != REQUIRED_HEIGHT
        || contract.map_operation_count != 1266
        || contract.selected_present_ui_operation_count != 1167
        || contract.map_count_authority.expected_count != contract.map_operation_count
        || contract.ui_count_authority.expected_count
            != contract.selected_present_ui_operation_count
        || contract.map_count_authority.repository_relative_path
            != "replay_interface/re_evidence/rendering_laws/XN15L/stage_so_census.json"
        || contract.map_count_authority.json_pointer != "/summary/all/draw_count"
        || contract.ui_count_authority.repository_relative_path
            != "replay_interface/re_evidence/rendering_laws/M02/production_interleave_closure/contract.json"
        || contract.ui_count_authority.json_pointer
            != "/authenticated_m02_draw/visible_ui_pass_draw_count"
        || contract.shader_asset_path != "performance_assets/f1_stress.hlsl"
        || contract.synthetic_product_modset_sha256
            != "b1836cbf1d952178865b910977dc880296ade11e0df3ebb520c40abb94a3f900"
        || contract.synthetic_effective_vfs_manifest_sha256
            != "217adb01b676c3677fbf605be059298f82d5144f5e2b1cde9d51b5bc186593f0"
        || contract.ui_surface_algorithm != "rgba32_64_96_16_repeated_1920x1080"
        || contract.ui_surface_sha256
            != "fe596b4cf34242b20a49f3d5ba95686cd2103097fb314ebc637b0f74f6988a41"
        || !contract.approved_omissions.is_empty()
    {
        return Err("headless performance stress contract is incomplete or unauthorized".into());
    }
    for (label, value) in [
        ("shader asset", &contract.shader_asset_sha256),
        (
            "synthetic product modset",
            &contract.synthetic_product_modset_sha256,
        ),
        (
            "synthetic effective VFS",
            &contract.synthetic_effective_vfs_manifest_sha256,
        ),
        ("UI surface", &contract.ui_surface_sha256),
        ("map count authority", &contract.map_count_authority.sha256),
        ("UI count authority", &contract.ui_count_authority.sha256),
    ] {
        validate_lower_sha256(value, label)?;
    }
    Ok(())
}

fn validate_lower_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!("{label} is not a lowercase SHA-256").into());
    }
    Ok(())
}

fn checked_descendant(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("stress workload contains a non-descendant path".into());
    }
    let canonical_root = root.canonicalize()?;
    let canonical = canonical_root.join(path).canonicalize()?;
    if !canonical.starts_with(&canonical_root) {
        return Err("stress workload path escapes its authority root".into());
    }
    Ok(canonical)
}

fn validate_count_authority(root: &Path, authority: &StressCountAuthority) -> Result<()> {
    let path = checked_descendant(root, &authority.repository_relative_path)?;
    let bytes = std::fs::read(&path)?;
    require_eq(
        "stress count-authority SHA-256",
        &sha256_hex(&bytes),
        &authority.sha256,
    )?;
    let document: serde_json::Value = serde_json::from_slice(&bytes)?;
    let count = document
        .pointer(&authority.json_pointer)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("stress count authority JSON pointer is absent or not an integer")?;
    if count != authority.expected_count {
        return Err("stress count authority differs from the contract".into());
    }
    Ok(())
}

fn compile_stress_shaders(source: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    unsafe {
        let vertex = crate::render::compile_shader(source, "vs_main", "vs_5_0")?;
        let pixel = crate::render::compile_shader(source, "ps_main", "ps_5_0")?;
        Ok((blob_bytes(&vertex)?, blob_bytes(&pixel)?))
    }
}

unsafe fn blob_bytes(blob: &crate::d3d::ComPtr<ID3DBlob>) -> Result<Vec<u8>> {
    let length = (*blob.as_raw()).GetBufferSize();
    let data = (*blob.as_raw()).GetBufferPointer() as *const u8;
    if data.is_null() || length == 0 {
        return Err("D3D compiler returned empty stress shader bytecode".into());
    }
    Ok(std::slice::from_raw_parts(data, length).to_vec())
}

fn build_stress_content(
    root: &Path,
    contract: &StressWorkloadContract,
    used: &replay_core::UsedContentKey,
    expected_consumer: &str,
) -> Result<(
    map_viewer::GameFileResolver,
    portable_content_manifest::ContentManifest,
)> {
    use portable_content_manifest::{
        ContentLayerKind, ContentManifest, LayerStorageKind, PlaybackContentInventory,
        PlaybackContentLayer, RecordedContentLayer, RequiredContent, RequiredContentKind,
        ResolvedPlaybackContent, SourceGameIdentity, SourceProfileIdentity, SCHEMA,
    };

    let mut manifest = ContentManifest {
        schema: SCHEMA.into(),
        product_modset_sha256: contract.synthetic_product_modset_sha256.clone(),
        source_game: SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                .into(),
            build_profile: "performance_only_non_semantic".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: "f1-performance-stress".into(),
            playset_id: "f1-performance-stress".into(),
            selection_sha256: String::new(),
        },
        effective_vfs_manifest_sha256: contract.synthetic_effective_vfs_manifest_sha256.clone(),
        ordered_layers: vec![RecordedContentLayer {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            display_name: "F1 non-semantic performance assets".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
            required_for_playback: true,
        }],
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        required_content: vec![RequiredContent {
            content_key: "f1-performance-stress-shader".into(),
            kind: RequiredContentKind::ShaderSource,
            content: used.clone(),
            observation_count: u64::try_from(contract.map_operation_count)?,
            consumers: vec![expected_consumer.into()],
        }],
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256()?;
    let manifest = manifest.with_computed_digest()?;
    let inventory = PlaybackContentInventory {
        source_game: manifest.source_game.clone(),
        source_profile: manifest.source_profile.clone(),
        layers: vec![PlaybackContentLayer {
            recorded_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
        }],
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        settings: BTreeMap::new(),
        resolved_content: vec![ResolvedPlaybackContent {
            normalized_game_relative_path: used.normalized_game_relative_path.clone(),
            winning_layer_ordinal: used.winning_layer_ordinal,
            layer_kind: used.layer_kind,
            layer_id: used.layer_id.clone(),
            source_sha256: used.source_sha256.clone(),
            payload_sha256: used.payload_sha256.clone(),
            payload_bytes: used.payload_bytes,
        }],
    };
    let admission = manifest.admit_playback(&inventory).map_err(|report| {
        format!(
            "performance stress content admission failed: {}",
            serde_json::to_string(&report).unwrap_or_else(|_| format!("{report:?}"))
        )
    })?;
    let resolver =
        map_viewer::GameFileResolver::vanilla(root)?.with_content_admission(&admission)?;
    Ok((resolver, manifest))
}

fn stress_used_content(
    relative_shader_path: &str,
    shader_bytes: &[u8],
    contract: &StressWorkloadContract,
) -> Result<replay_core::UsedContentKey> {
    let source_sha256 = sha256_hex(shader_bytes);
    let used = replay_core::UsedContentKey {
        effective_vfs_manifest_sha256: contract.synthetic_effective_vfs_manifest_sha256.clone(),
        normalized_game_relative_path: replay_core::normalize_game_relative_path(
            relative_shader_path,
        )?,
        winning_layer_ordinal: 0,
        layer_kind: replay_core::ContentLayerKind::Base,
        layer_id: "base".into(),
        source_sha256: source_sha256.clone(),
        payload_sha256: source_sha256,
        payload_bytes: u64::try_from(shader_bytes.len())?,
    };
    used.validate()?;
    Ok(used)
}

fn build_stress_map_frame(
    contract: &StressWorkloadContract,
    source: replay_core::UsedContentKey,
    vertex_shader: Vec<u8>,
    pixel_shader: Vec<u8>,
    manifest_sha256: &str,
) -> Result<map_viewer::PreparedProductionD3dFrame> {
    use replay_core::{
        BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison, CompiledShaderPayload,
        CullMode, DepthDesc, DrawCommand, FillMode, GeometryBinding, IndexFormat, InputElementDesc,
        MapnameD80State, PipelineDesc, PreparedRenderTarget, PrimitiveTopology, RasterizerDesc,
        RenderExecutionRef, RenderOutputState, RenderTargetDesc, ShaderProgram,
        StaticGeometrySource, StaticMapCommand, StaticMapDraw, StaticMapOperation,
        StaticMapPassProgram, StencilFaceDesc, StencilOperation, VertexBufferBinding, ViewportDesc,
    };

    let mut vertices = Vec::new();
    for value in [-1.0f32, -1.0, -1.0, 3.0, 3.0, -1.0] {
        vertices.extend_from_slice(&value.to_le_bytes());
    }
    let stencil = StencilFaceDesc {
        fail_operation: StencilOperation::Keep,
        depth_fail_operation: StencilOperation::Keep,
        pass_operation: StencilOperation::Keep,
        comparison: Comparison::Always,
    };
    let draw_template = StaticMapDraw {
        draw_token: String::new(),
        shader_program: ShaderProgram {
            source: source.clone(),
            producer_registration_root_rva: None,
            effect_token: "f1.performance_only.non_semantic".into(),
            vertex_entry_token: "vs_main".into(),
            pixel_entry_token: "ps_main".into(),
            ordered_defines: Vec::new(),
            compiled_vertex_shader: CompiledShaderPayload {
                compiler_identity: "windows_d3dcompiler_runtime".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                sha256: sha256_hex(&vertex_shader),
                bytecode: vertex_shader,
            },
            compiled_pixel_shader: CompiledShaderPayload {
                compiler_identity: "windows_d3dcompiler_runtime".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                sha256: sha256_hex(&pixel_shader),
                bytecode: pixel_shader,
            },
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        },
        ordered_shader_resources: Vec::new(),
        pipeline: PipelineDesc {
            topology: PrimitiveTopology::TriangleList,
            blend: BlendDesc {
                alpha_to_coverage_enabled: false,
                independent_blend_enabled: false,
                enabled: false,
                source_color: BlendFactor::One,
                destination_color: BlendFactor::Zero,
                color_operation: BlendOperation::Add,
                source_alpha: BlendFactor::One,
                destination_alpha: BlendFactor::Zero,
                alpha_operation: BlendOperation::Add,
                write_mask_rgba_u8: 0xf,
                additional_targets: Vec::new(),
                blend_factor_rgba_f32_bits: [0; 4],
                sample_mask_u32: u32::MAX,
            },
            depth: DepthDesc {
                enabled: false,
                write_enabled: false,
                comparison: Comparison::Always,
                stencil_enabled: false,
                stencil_read_mask_u8: 0xff,
                stencil_write_mask_u8: 0xff,
                front_face: stencil.clone(),
                back_face: stencil,
                stencil_reference_u32: 0,
            },
            rasterizer: RasterizerDesc {
                fill: FillMode::Solid,
                cull: CullMode::None,
                front_counter_clockwise: false,
                depth_clip_enabled: true,
                scissor_enabled: false,
                depth_bias_i32: 0,
                depth_bias_clamp_f32_bits: 0,
                slope_scaled_depth_bias_f32_bits: 0,
                multisample_enabled: false,
                antialiased_line_enabled: false,
            },
            samplers: Vec::new(),
            input_layout: vec![InputElementDesc {
                semantic_token: "POSITION".into(),
                semantic_index: 0,
                dxgi_format_u32: winapi::shared::dxgiformat::DXGI_FORMAT_R32G32_FLOAT,
                input_slot: 0,
                aligned_byte_offset: 0,
                per_instance: false,
                instance_step_rate: 0,
            }],
            index_format: IndexFormat::None,
            static_indices_u16: Vec::new(),
        },
        geometry: StaticGeometrySource::Exact {
            geometry: GeometryBinding {
                ordered_vertex_buffers: vec![VertexBufferBinding {
                    input_slot: 0,
                    stride_bytes: 8,
                    byte_offset: 0,
                    source: BufferSource::Inline {
                        sha256: sha256_hex(&vertices),
                        bytes: vertices,
                    },
                }],
                index_buffer: None,
                stream_output_targets: Vec::new(),
                draw: DrawCommand::Draw {
                    vertex_count: 3,
                    start_vertex: 0,
                },
            },
        },
        ordered_constant_buffers: Vec::new(),
        output_state: RenderOutputState {
            ordered_color_targets: vec![RenderTargetDesc {
                prepared_target: PreparedRenderTarget::SwapchainBackBuffer,
                width: REQUIRED_WIDTH,
                height: REQUIRED_HEIGHT,
                dxgi_format_u32: winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }],
            depth_target: None,
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: (REQUIRED_WIDTH as f32).to_bits(),
                height_f32_bits: (REQUIRED_HEIGHT as f32).to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor: None,
        },
        payload_sha256: sha256_hex(b"f1-performance-only-map-draw-v1"),
    };
    let mut operations = Vec::with_capacity(contract.map_operation_count);
    let mut order = Vec::with_capacity(contract.map_operation_count);
    for ordinal in 0..contract.map_operation_count {
        let ordinal = u32::try_from(ordinal)?;
        let mut draw = draw_template.clone();
        draw.draw_token = format!("f1.performance_only.map.{ordinal:04}");
        draw.payload_sha256 = sha256_hex(draw.draw_token.as_bytes());
        operations.push(StaticMapOperation {
            operation_ordinal: ordinal,
            payload_sha256: sha256_hex(format!("f1-operation-{ordinal}").as_bytes()),
            command: StaticMapCommand::Draw { draw },
        });
        order.push(RenderExecutionRef::Static {
            operation_ordinal: ordinal,
        });
    }
    let program_sha256 = sha256_hex(&serde_json::to_vec(&operations)?);
    Ok(map_viewer::PreparedProductionD3dFrame {
        content_manifest_sha256: manifest_sha256.into(),
        product_modset_sha256: contract.synthetic_product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: contract.synthetic_effective_vfs_manifest_sha256.clone(),
        c03_camera_replay: None,
        target: Some(map_viewer::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 1,
            color_width: REQUIRED_WIDTH,
            color_height: REQUIRED_HEIGHT,
            color_format: winapi::shared::dxgiformat::DXGI_FORMAT_R8G8B8A8_UNORM,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 2,
            depth_width: REQUIRED_WIDTH,
            depth_height: REQUIRED_HEIGHT,
            depth_format: winapi::shared::dxgiformat::DXGI_FORMAT_D24_UNORM_S8_UINT,
            depth_samples: 1,
            depth_quality: 0,
        }),
        ordered_scene_constant_buffers: Vec::new(),
        ordered_render_operations: order,
        snow_mud_texture: None,
        political_textures: Vec::new(),
        tiled_lights: Vec::new(),
        authenticated_texture_snapshots: Vec::new(),
        ordered_static_used_content: vec![source],
        ordered_static_programs: vec![StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: "f1.performance_only.non_semantic_map_stress".into(),
            ordered_content_keys: vec!["f1-performance-stress-shader".into()],
            ordered_operations: operations,
            payload_sha256: program_sha256,
        }],
        rail_ribbon: None,
        mapname_d80_state: MapnameD80State::Missing,
        tree_runtime: None,
        tree_materialized_execution_sha256: None,
        s1_shadow: None,
        ordered_runtime_draws: Vec::new(),
        m02_combat: None,
        authenticated_ui_pass: None,
        m03_arrows: None,
        rail_supply_arrows: None,
        omissions: Vec::new(),
    })
}

fn stress_ui_surface() -> Result<game_ui::dds::RgbaImage> {
    let pixel_count = usize::try_from(REQUIRED_WIDTH)?
        .checked_mul(usize::try_from(REQUIRED_HEIGHT)?)
        .ok_or("stress UI surface extent overflows")?;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for _ in 0..pixel_count {
        pixels.extend_from_slice(&[32, 64, 96, 16]);
    }
    Ok(game_ui::dds::RgbaImage {
        width: usize::try_from(REQUIRED_WIDTH)?,
        height: usize::try_from(REQUIRED_HEIGHT)?,
        pixels,
    })
}

fn require_stable_identity(
    stable: &mut Option<String>,
    current: String,
    label: &str,
) -> Result<()> {
    match stable {
        Some(expected) if expected != &current => {
            Err(format!("{label} changed between frames").into())
        }
        Some(_) => Ok(()),
        None => {
            *stable = Some(current);
            Ok(())
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn recording_identity_sha256(path: &Path) -> Result<String> {
    if path.file_name().and_then(|value| value.to_str())
        == Some(replay_core::ROLLOVER_PACKAGE_SET_FILE)
    {
        return Ok(recording_identity_sha256_for_bytes(
            b"rollover-package-set",
            &std::fs::read(path)?,
        ));
    }
    if path.is_file() {
        return Ok(recording_identity_sha256_for_bytes(
            b"portable-file",
            &std::fs::read(path)?,
        ));
    }
    if path.is_dir() {
        let package_set = path.join(replay_core::ROLLOVER_PACKAGE_SET_FILE);
        if !package_set.is_file() {
            return Err(format!(
                "recording directory has no {}",
                replay_core::ROLLOVER_PACKAGE_SET_FILE
            )
            .into());
        }
        return Ok(recording_identity_sha256_for_bytes(
            b"rollover-package-set",
            &std::fs::read(package_set)?,
        ));
    }
    Err(format!(
        "recording path is neither a file nor a directory: {}",
        path.display()
    )
    .into())
}

fn recording_identity_sha256_for_bytes(kind: &[u8], bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"hoi4.replay-recording-artifact.xmap22.v1\0");
    digest.update(kind);
    digest.update(b"\0");
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn write_stress_receipt(path: &Path, receipt: &StressBenchReceipt) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    crate::headless::write_create_new_or_equal(path, &bytes, "headless performance stress receipt")
}

pub(crate) fn defaults() -> (usize, usize) {
    (DEFAULT_WARMUP_FRAMES, DEFAULT_SAMPLE_FRAMES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stress_contract() -> StressWorkloadContract {
        serde_json::from_str(include_str!("../headless_performance_workload.json"))
            .expect("checked-in stress contract parses")
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("canonical repository root")
    }

    #[test]
    fn nearest_rank_summary_preserves_tail_samples() {
        let samples = (1..=100).map(|value| value as f64).collect::<Vec<_>>();
        let metrics = summarize(&samples).expect("valid metrics");
        assert_eq!(metrics.sample_count, 100);
        assert_eq!(metrics.p50_frame_time_ms, 50.0);
        assert_eq!(metrics.p95_frame_time_ms, 95.0);
        assert_eq!(metrics.p99_frame_time_ms, 99.0);
        assert_eq!(metrics.max_frame_time_ms, 100.0);
    }

    #[test]
    fn recording_identity_is_kind_bound_and_deterministic() {
        let bytes = br#"{"schema":"fixture"}"#;
        let portable = recording_identity_sha256_for_bytes(b"portable-file", bytes);
        let portable_again = recording_identity_sha256_for_bytes(b"portable-file", bytes);
        let rollover = recording_identity_sha256_for_bytes(b"rollover-package-set", bytes);
        assert_eq!(portable, portable_again);
        assert_ne!(portable, rollover);
        assert_eq!(portable.len(), 64);
    }

    #[test]
    fn absent_or_invalid_metrics_fail_closed() {
        assert!(summarize(&[]).is_err());
        assert!(summarize(&[0.0]).is_err());
        assert!(summarize(&[f64::NAN]).is_err());
        assert!(summarize(&[f64::INFINITY]).is_err());
    }

    #[test]
    fn checked_in_stress_schema_and_authority_hashes_are_self_consistent() {
        let contract = stress_contract();
        validate_stress_contract(&contract).expect("authorize checked-in stress contract");
        let root = repository_root();
        validate_count_authority(&root, &contract.map_count_authority)
            .expect("map count authority");
        validate_count_authority(&root, &contract.ui_count_authority).expect("UI count authority");

        let contract_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let shader = checked_descendant(contract_root, &contract.shader_asset_path)
            .expect("checked-in shader is a descendant");
        let shader_bytes = std::fs::read(shader).expect("read checked-in shader");
        require_eq(
            "stress shader asset SHA-256",
            &sha256_hex(&shader_bytes),
            &contract.shader_asset_sha256,
        )
        .expect("shader hash is pinned");
    }

    #[test]
    fn stress_contract_rejects_semantic_claims_counts_and_noncanonical_hashes() {
        let mut contract = stress_contract();
        contract.prohibited_claims.pop();
        assert!(validate_stress_contract(&contract).is_err());

        let mut contract = stress_contract();
        contract.map_operation_count += 1;
        assert!(validate_stress_contract(&contract).is_err());

        let mut contract = stress_contract();
        contract.shader_asset_sha256 = "A".repeat(64);
        assert!(validate_stress_contract(&contract).is_err());

        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../headless_performance_workload.json"))
                .expect("stress JSON");
        value["unexpected_promotion_claim"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<StressWorkloadContract>(value).is_err());
    }

    #[test]
    fn recording_selection_envelope_requires_only_stream_and_hour() {
        let selection: RecordingSelectionEnvelope = serde_json::from_value(serde_json::json!({
            "logical_stream_id": 7,
            "selected_hour": 124,
            "ignored_legacy_contract": {
                "not_an_admission_prerequisite": true
            }
        }))
        .expect("minimal recording selection parses");
        assert_eq!(selection.logical_stream_id, Some(7));
        assert_eq!(selection.selected_hour, 124);

        assert!(
            serde_json::from_value::<RecordingSelectionEnvelope>(serde_json::json!({
                "logical_stream_id": 7
            }))
            .is_err()
        );
    }

    #[test]
    fn recording_benchmark_warmups_and_samples_advance_route_equivalent_viewer_clock() {
        let mut viewer = map_viewer::ViewerOwnedMapState {
            viewport_width: REQUIRED_WIDTH,
            viewport_height: REQUIRED_HEIGHT,
            input_revision: 0,
            camera: None,
            observer_country_id: None,
            map_mode_id: map_viewer::ViewerMapModeSelection::default().viewer_token(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            animation_time_f32_bits: 0.0f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
        };
        let warmups = 3;
        let samples = 4;
        for _ in 0..warmups {
            advance_recording_benchmark_viewer_clock(&mut viewer, 1.0 / 60.0).unwrap();
        }
        let after_warmups = f32::from_bits(viewer.animation_time_f32_bits);
        let expected_after_warmups =
            (0..warmups).fold(0.0f32, |elapsed, _| elapsed + 1.0f32 / 60.0);
        for _ in 0..samples {
            advance_recording_benchmark_viewer_clock(&mut viewer, 1.0 / 60.0).unwrap();
        }
        let expected_after_samples =
            (0..warmups + samples).fold(0.0f32, |elapsed, _| elapsed + 1.0f32 / 60.0);
        assert_eq!(after_warmups.to_bits(), expected_after_warmups.to_bits());
        assert_eq!(
            viewer.animation_time_f32_bits,
            expected_after_samples.to_bits()
        );
        assert_eq!(
            viewer.frame_delta_seconds_f32_bits,
            (1.0f32 / 60.0).to_bits()
        );
    }

    #[test]
    fn production_recording_benchmark_executes_shared_xmap22_composition() {
        let (topology, schedule) =
            crate::validate::product_foundation_assigned_stage_plan().unwrap();
        assert_eq!(topology.ordered_stages().len(), 22);
        assert_eq!(topology.ordered_edges().len(), 18);
        schedule
            .require(
                map_viewer::XMAP06_EARLY_STAGE_NAME,
                map_viewer::XMAP06_EARLY_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP06");
        schedule
            .require(
                map_viewer::XMAP08_RIVER_STAGE_NAME,
                map_viewer::XMAP08_RIVER_OPERATION_ID,
            )
            .expect("recording benchmark product foundation includes XMAP08 rivers");
        schedule
            .require(
                map_viewer::XMAP08_STRAIT_STAGE_NAME,
                map_viewer::XMAP08_STRAIT_OPERATION_ID,
            )
            .expect("recording benchmark product foundation includes XMAP08 straits");
        schedule
            .require(
                map_viewer::XMAP11_TREE_STAGE_NAME,
                map_viewer::XMAP11_TREE_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP11 trees");
        schedule
            .require(
                map_viewer::XMAP17_EXECUTE_STAGE,
                map_viewer::XMAP17_EXECUTE_OPERATION,
            )
            .expect("recording benchmark product foundation includes XMAP17 labels");
        schedule
            .require(
                map_viewer::XMAP09_PREPARE_STAGE_NAME,
                map_viewer::XMAP09_PREPARE_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP09");
        schedule
            .require(
                map_viewer::XMAP19_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP19_ASSIGNED_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP19");
        schedule
            .require(
                map_viewer::XMAP07_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP07_ASSIGNED_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP07");
        schedule
            .require(
                map_viewer::XMAP13_UNIT_ENTITY_STAGE_NAME,
                map_viewer::XMAP13_UNIT_ENTITY_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP13 entities");
        schedule
            .require(
                map_viewer::XMAP13_COUNTER_QUAD_STAGE_NAME,
                map_viewer::XMAP13_COUNTER_QUAD_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP13 counters");
        schedule
            .require(
                map_viewer::XMAP14_COMBAT_STAGE_NAME,
                map_viewer::XMAP14_COMBAT_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP14 combat indicators");
        schedule
            .require(
                map_viewer::XMAP15_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP15_ASSIGNED_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP15 orders");
        schedule
            .require(
                map_viewer::XMAP20_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP20_ASSIGNED_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP20");
        schedule
            .require(
                map_viewer::XMAP16_RAIL_STAGE_NAME,
                map_viewer::XMAP16_RAIL_OPERATION_ID,
            )
            .expect("recording benchmark product foundation includes XMAP16");
        schedule
            .require(
                map_viewer::XMAP21_PREPARE_STAGE_NAME,
                map_viewer::XMAP21_PREPARE_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP21 prepare");
        schedule
            .require(
                map_viewer::XMAP21_EXECUTE_STAGE_NAME,
                map_viewer::XMAP21_EXECUTE_OPERATION_NAME,
            )
            .expect("recording benchmark product foundation includes XMAP21 execute");

        let source = include_str!("performance_bench.rs");
        let start = source
            .find("pub(crate) fn run(")
            .expect("recording benchmark entry");
        let end = source
            .find("pub(crate) fn run_stress(")
            .expect("stress benchmark entry");
        let recording = &source[start..end];
        let composition = recording
            .find("ProductMapCompositionSession::start_for_benchmark(")
            .expect("shared XMAP22 product compositor");
        let ui_profile = recording
            .find("playback_ui_profile_from_paths(")
            .expect("presentation profile load");
        let execute = recording
            .find("composition.execute(")
            .expect("shared XMAP22 execution");
        let identity_before = recording
            .find("recording_identity_before_sha256 =")
            .expect("pre-open recording identity");
        let provider_open = recording
            .find("open_provider(&mode)")
            .expect("recording provider open");
        let identity_after = recording
            .find("recording_identity_after_sha256 =")
            .expect("post-sample recording identity");
        assert!(
            composition < ui_profile,
            "foundation admission must precede UI profile loading"
        );
        assert!(
            ui_profile < execute,
            "UI profile must be available before composition execution"
        );
        assert!(
            identity_before < provider_open,
            "recording identity must be bound before open"
        );
        assert!(
            provider_open < identity_after,
            "recording identity must be rechecked after use"
        );
        assert!(recording[composition..].contains("summarize(&samples_ms)"));
        assert!(recording[composition..].contains("options.output"));
        for forbidden in [
            "WorkloadContract",
            "validate_contract(",
            "validate_options(",
            "certification_enabled",
            "recording_hour_count",
            "first_hour",
            "last_hour",
            "content_manifest_digest_sha256",
            "product_modset_sha256",
            "effective_vfs_manifest_sha256",
            "production_render",
            "selected_present",
            "launch_epoch",
            "capture_receipt",
            "expected_map_operations",
            "expected_selected_present_operations",
            "map_operation_count",
            "selected_present_ui_operation_count",
            "approved_plan_omissions",
            "approved_d3d_omissions",
        ] {
            assert!(
                !recording.contains(forbidden),
                "recording benchmark consumes legacy authority token {forbidden}"
            );
        }
        for forbidden in [
            "ProductObserverUiRuntime::from_playback_content",
            "ui_runtime.execute(",
            "ProductionMapRenderer::new",
            "prepare_production_d3d_frame(",
            "Gfx::create_offscreen",
            "write_receipt(",
            "recorded_ui_owner(",
            "prepared.authenticated_ui_pass",
            "production_render",
            "selected_present",
            ".submit(",
            "read_target",
            "write_bmp",
            "pixel-gate",
        ] {
            assert!(
                !recording[composition..].contains(forbidden),
                "production benchmark must not consume {forbidden}"
            );
        }
    }

    #[test]
    fn authority_paths_and_counts_cannot_escape_or_drift() {
        let root = repository_root();
        assert!(checked_descendant(&root, "../outside.json").is_err());
        assert!(checked_descendant(&root, r"C:\outside.json").is_err());

        let contract = stress_contract();
        let drifted = StressCountAuthority {
            repository_relative_path: contract
                .map_count_authority
                .repository_relative_path
                .clone(),
            sha256: contract.map_count_authority.sha256.clone(),
            json_pointer: contract.map_count_authority.json_pointer.clone(),
            expected_count: contract.map_count_authority.expected_count + 1,
        };
        assert!(validate_count_authority(&root, &drifted).is_err());

        let malformed_hash = StressCountAuthority {
            repository_relative_path: contract.map_count_authority.repository_relative_path,
            sha256: "0".repeat(64),
            json_pointer: contract.map_count_authority.json_pointer,
            expected_count: contract.map_count_authority.expected_count,
        };
        assert!(validate_count_authority(&root, &malformed_hash).is_err());
    }

    #[test]
    fn per_frame_counts_and_receipt_identity_must_remain_stable() {
        let counts = FrameCounts {
            map_operations: 1266,
            selected_present_operations: 1167,
            selected_present_m02_draws: 0,
            observer_ui_draws: 0,
        };
        let mut stable_counts = None;
        require_stable_counts(&mut stable_counts, counts).expect("first count sample");
        require_stable_counts(&mut stable_counts, counts).expect("same count sample");
        assert!(require_stable_counts(
            &mut stable_counts,
            FrameCounts {
                map_operations: 1265,
                ..counts
            }
        )
        .is_err());

        let mut stable_identity = None;
        require_stable_identity(&mut stable_identity, "a".repeat(64), "fixture receipt")
            .expect("first receipt");
        require_stable_identity(&mut stable_identity, "a".repeat(64), "fixture receipt")
            .expect("same receipt");
        assert!(
            require_stable_identity(&mut stable_identity, "b".repeat(64), "fixture receipt")
                .is_err()
        );
    }
}
