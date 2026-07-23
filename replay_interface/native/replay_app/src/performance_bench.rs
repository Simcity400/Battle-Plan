//! F1 production headless performance acceptance.
//!
//! The stronger recording profile opens one admitted portable recording and
//! executes its exact product map and selected-Present programs. The default F1
//! profile instead runs a contract-pinned synthetic stress packet through those
//! same production GPU paths so performance acceptance has no future-capture
//! dependency. Stress receipts explicitly prohibit semantic, pixel, RE, and
//! rendering-law claims. Both profiles measure GPU-completed 1920x1080 frames;
//! neither accepts queued CPU-submit time, debug bundles, or omissions.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use winapi::um::d3dcommon::ID3DBlob;

use crate::app::ProductProductionD3dRuntime;
use crate::d3d::{Gfx, GpuAdapterIdentity};
use crate::timeline::{resolve_dump_hour, DumpHourRequest, OwnerProvider};
use crate::ui_runtime::{
    ArmyBattleplanUiSupplementalInputs, IntelligenceUiSupplementalInputs,
    OccupationUiSupplementalInputs, ProductSelectedPresentPerformanceRuntime,
    ProductSelectedPresentRuntime, SelectedPresentUiOutcome,
};
use crate::{Result, TimelineMode};

const REQUIRED_WIDTH: u32 = 1920;
const REQUIRED_HEIGHT: u32 = 1080;
const MIN_WARMUP_FRAMES: usize = 60;
const MIN_SAMPLE_FRAMES: usize = 300;
const DEFAULT_WARMUP_FRAMES: usize = 120;
const DEFAULT_SAMPLE_FRAMES: usize = 600;
const MAX_AVERAGE_FRAME_MS: f64 = 1000.0 / 60.0;
const MAX_P95_FRAME_MS: f64 = 33.3;
const MIN_CERTIFICATION_RECORDING_HOURS: usize = 24;

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
#[serde(deny_unknown_fields)]
struct WorkloadContract {
    schema: String,
    certification_enabled: bool,
    workload_id: String,
    recording_authority_sha256: String,
    logical_stream_id: Option<u64>,
    recording_hour_count: usize,
    first_hour: i64,
    last_hour: i64,
    selected_hour: i64,
    content_manifest_digest_sha256: String,
    product_modset_sha256: String,
    effective_vfs_manifest_sha256: String,
    production_render_sha256: String,
    selected_present_sha256: String,
    launch_epoch_sha256: String,
    capture_receipt_sha256: String,
    frame_cache_entries: usize,
    expected_map_operations: usize,
    expected_selected_present_operations: usize,
    expected_selected_present_m02_draws: usize,
    approved_plan_omissions: Vec<String>,
    approved_d3d_omissions: Vec<String>,
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

#[derive(Debug, Serialize)]
struct Workload {
    recording_path: String,
    recording_sha256: String,
    workload_contract_path: String,
    workload_contract_sha256: String,
    workload_id: String,
    logical_stream_id: Option<u64>,
    recording_hour_count: usize,
    first_hour: i64,
    last_hour: i64,
    selected_hour: i64,
    content_manifest_digest_sha256: String,
    product_modset_sha256: String,
    effective_vfs_manifest_sha256: String,
    production_render_sha256: String,
    selected_present_sha256: String,
    launch_epoch_sha256: String,
    capture_receipt_sha256: String,
    viewport_width: u32,
    viewport_height: u32,
    gpu_adapter_name: String,
    gpu_dedicated_video_memory_bytes: u64,
    gpu_vendor_id: u32,
    gpu_device_id: u32,
    frame_cache_entries: usize,
    warmup_frames: usize,
    sample_frames: usize,
    map_operations_per_frame: usize,
    selected_present_operations_per_frame: usize,
    selected_present_m02_draws_per_frame: usize,
    feature_local_plan_omissions: usize,
    feature_local_d3d_omissions: usize,
    gpu_completion_method: &'static str,
    timer: &'static str,
}

#[derive(Debug, Serialize)]
struct BenchReceipt {
    schema: &'static str,
    result: &'static str,
    workload: Workload,
    reference_machine_contract: &'static str,
    thresholds: Thresholds,
    metrics: Metrics,
    failures: Vec<String>,
}

#[derive(Clone, Copy)]
struct FrameCounts {
    map_operations: usize,
    selected_present_operations: usize,
    selected_present_m02_draws: usize,
}

pub(crate) fn run(
    options: BenchOptions,
    product_paths: &crate::install_paths::ProductPaths,
) -> Result<()> {
    validate_options(&options)?;
    if options.output.exists() {
        return Err(format!(
            "performance receipt already exists; refusing to replace {}",
            options.output.display()
        )
        .into());
    }

    let contract_bytes = std::fs::read(&options.workload_contract)?;
    let contract_sha256 = format!("{:x}", Sha256::digest(&contract_bytes));
    let contract: WorkloadContract = serde_json::from_slice(&contract_bytes)?;
    validate_contract(&contract)?;
    if options.frame_cache_entries != contract.frame_cache_entries {
        return Err("frame-cache entries differ from the checked-in workload contract".into());
    }
    let mode = TimelineMode::Recording {
        path: options.recording.clone(),
        logical_stream_id: contract.logical_stream_id,
        timeline: replay_core::TimelineConfig {
            frame_cache_entries: options.frame_cache_entries,
        },
    };
    let (provider, recording_sha256) = open_provider(&mode)?;
    require_eq(
        "recording authority SHA-256",
        &recording_sha256,
        &contract.recording_authority_sha256,
    )?;
    let (resolver, content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let manifest = provider
        .content_manifest()
        .ok_or("performance benchmark recording has no admitted content manifest")?;
    let static_research = Arc::new(
        game_ui::static_research::admit_static_research_catalogs(
            provider.static_artifact(replay_core::LOCALISATION_JOIN_ARTIFACT),
            provider.static_artifact(replay_core::IDEA_TECH_LOC_XREF_ARTIFACT),
            provider.static_artifact(replay_core::TECHNOLOGY_SHARING_ARTIFACT),
        )
        .map_err(|error| format!("admit packaged static research catalogs: {error}"))?,
    );
    let hours = provider.hours();
    if hours.len() != contract.recording_hour_count
        || hours.first().copied() != Some(contract.first_hour)
        || hours.last().copied() != Some(contract.last_hour)
    {
        return Err("performance recording hour census differs from the checked-in contract".into());
    }
    let selected_hour =
        resolve_dump_hour(hours, DumpHourRequest::Canonical(contract.selected_hour))?;

    let initial_frame = provider
        .world_frame_at(
            *hours
                .first()
                .ok_or("performance benchmark timeline has no hours")?,
        )?
        .ok_or("performance benchmark initial replay frame is unavailable")?;
    let display = initial_frame
        .session_metadata()
        .display_variant
        .measured()
        .ok_or("performance benchmark lacks authenticated initial display_variant")?;
    if (display.physical_width, display.physical_height) != (REQUIRED_WIDTH, REQUIRED_HEIGHT) {
        return Err(format!(
            "performance benchmark requires an authenticated 1920x1080 recording, got {}x{}",
            display.physical_width, display.physical_height
        )
        .into());
    }

    let frame = provider
        .world_frame_at(selected_hour)?
        .ok_or("performance benchmark selected exact replay frame is unavailable")?;
    static_research
        .bind_frame(&frame)
        .map_err(|error| {
            format!(
                "bind packaged static research catalogs at hour {selected_hour}: {error}"
            )
        })?;
    let production_render = frame
        .production_render()
        .ok_or("performance benchmark selected frame has no authenticated production_render")?;
    if provider.g2_engine_capture().is_some() {
        return Err("G2 engine-capture frames cannot certify the production performance gate".into());
    }
    require_eq(
        "content manifest digest",
        manifest
            .manifest_digest_sha256
            .as_deref()
            .ok_or("performance content manifest has no digest")?,
        &contract.content_manifest_digest_sha256,
    )?;
    require_eq(
        "product modset SHA-256",
        &manifest.product_modset_sha256,
        &contract.product_modset_sha256,
    )?;
    require_eq(
        "effective VFS manifest SHA-256",
        &manifest.effective_vfs_manifest_sha256,
        &contract.effective_vfs_manifest_sha256,
    )?;
    let production_render_sha256 = format!("{:x}", Sha256::digest(production_render.encode_json()?));
    let selected_present_sha256 = production_render.selected_present.computed_sha256()?;
    require_eq(
        "production render SHA-256",
        &production_render_sha256,
        &contract.production_render_sha256,
    )?;
    require_eq(
        "selected-Present SHA-256",
        &selected_present_sha256,
        &contract.selected_present_sha256,
    )?;
    require_eq(
        "launch epoch SHA-256",
        &production_render.authority.launch_epoch_sha256,
        &contract.launch_epoch_sha256,
    )?;
    require_eq(
        "capture receipt SHA-256",
        &production_render.authority.capture_receipt_sha256,
        &contract.capture_receipt_sha256,
    )?;
    let mut production_map =
        map_viewer::ProductionMapRenderer::new(&resolver, &content_admission, manifest)?;
    let plan = production_map.prepare_frame_with_rail(
        Some(production_render),
        Some(frame.rail_replay()),
        Some(frame.production_by_country()),
    )?;
    let prepared = map_viewer::prepare_production_d3d_frame(&plan, production_map.resources())?;
    if prepared.target.is_none() || prepared.ordered_render_operations.is_empty() {
        return Err(
            "performance benchmark has no authenticated product map target and operation stream"
                .into(),
        );
    }
    let plan_omissions = plan
        .omitted_data_needs
        .iter()
        .map(|value| format!("{}|{}|{}", value.packet, value.feature, value.reason))
        .collect::<Vec<_>>();
    let d3d_omissions = prepared
        .omissions
        .iter()
        .map(|value| format!("{}|{}", value.pass, value.reason))
        .collect::<Vec<_>>();
    if plan_omissions != contract.approved_plan_omissions
        || d3d_omissions != contract.approved_d3d_omissions
    {
        return Err("performance frame has unapproved or missing omission identities".into());
    }
    if prepared.authenticated_ui_pass.is_none() {
        return Err(
            "performance benchmark frame has no authenticated selected-Present UI pass".into(),
        );
    }

    let gfx = Gfx::create_offscreen(REQUIRED_WIDTH, REQUIRED_HEIGHT)?;
    let gpu_adapter = gfx.adapter_identity()?;
    validate_reference_gpu(&gpu_adapter)?;
    let gpu_completion = gfx.create_gpu_completion_query()?;
    let mut map_runtime = ProductProductionD3dRuntime::default();
    let static_focus_definitions = Arc::new(
        replay_core::admit_static_focus_definitions(
            provider
                .static_artifact(replay_core::FOCUS_DEFS_STATIC_ARTIFACT)
                .ok_or("performance replay has no authenticated focus_defs_static artifact")?,
        )
        .map_err(|error| format!("admit performance static focus definitions: {error}"))?,
    );
    let static_focus_catalog = Arc::new(
        game_ui::static_focus_defs::focus_catalog_from_static(&static_focus_definitions),
    );
    let recorded_ui = prepared
        .authenticated_ui_pass
        .as_ref()
        .map(|pass| crate::recorded_ui_owner::recorded_ui_owner(frame.as_ref(), pass))
        .transpose()
        .map_err(|error| {
            format!(
                "performance frame recorder-owned selected-Present UI identity is unavailable: {error}"
            )
        })?;
    let mut ui_runtime = match prepared.authenticated_ui_pass.as_ref() {
        Some(pass) => Some(
            ProductSelectedPresentRuntime::new_with_decision_catalog_bytes(
                &gfx,
                &resolver,
                &content_admission,
                pass,
                provider.decision_catalog().map(|catalog| catalog.bytes()),
                provider
                    .static_artifact("gfx_icon_asset_manifest_static")
                    .map(|artifact| artifact.bytes()),
                Arc::clone(&static_research),
                Arc::clone(&static_focus_definitions),
                Arc::clone(&static_focus_catalog),
            )?,
        ),
        _ => None,
    };
    let runtime_text = match frame.runtime_text()? {
        replay_core::Leaf::Measured(catalog) => Some(catalog),
        replay_core::Leaf::Absent | replay_core::Leaf::Unobtained => None,
    };

    let render_one = |map_runtime: &mut ProductProductionD3dRuntime,
                      ui_runtime: &mut Option<ProductSelectedPresentRuntime>|
     -> Result<FrameCounts> {
        gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;
        let map = map_runtime.execute(&gfx, &resolver, production_map.resources(), &prepared)?;
        if !map.presented_to_app_target || map.executed_operations == 0 {
            return Err(
                "performance benchmark map execution produced no authenticated app-target draws"
                    .into(),
            );
        }
        let (selected_present_operations, selected_present_m02_draws) = if let Some(runtime) =
            ui_runtime.as_mut()
        {
            match runtime.execute(
                &gfx,
                &resolver,
                &prepared,
                frame.as_ref(),
                recorded_ui.as_ref().and_then(|identity| identity.country.as_deref()),
                recorded_ui
                    .as_ref()
                    .map(|identity| identity.runtime_text_pov_country_or_global.as_str())
                    .unwrap_or("global"),
                runtime_text.as_ref(),
                ArmyBattleplanUiSupplementalInputs::default(),
                IntelligenceUiSupplementalInputs::default(),
                OccupationUiSupplementalInputs::default(),
            )? {
                SelectedPresentUiOutcome::Executed {
                    operation_count,
                    m02_draw_count,
                    ..
                } if operation_count > 0 => (operation_count, m02_draw_count),
                SelectedPresentUiOutcome::Executed { .. } => {
                    return Err(
                        "performance benchmark selected-Present executed zero operations".into(),
                    )
                }
                SelectedPresentUiOutcome::Omitted(reason) => {
                    return Err(format!(
                        "performance benchmark selected-Present was omitted: {reason}"
                    )
                    .into())
                }
            }
        } else {
            (0, 0)
        };
        gpu_completion.wait(&gfx)?;
        Ok(FrameCounts {
            map_operations: map.executed_operations,
            selected_present_operations,
            selected_present_m02_draws,
        })
    };

    let mut stable_counts = None;
    for _ in 0..options.warmup_frames {
        let counts = render_one(&mut map_runtime, &mut ui_runtime)?;
        require_stable_counts(&mut stable_counts, counts)?;
    }
    let mut samples_ms = Vec::with_capacity(options.sample_frames);
    for _ in 0..options.sample_frames {
        let started = Instant::now();
        let counts = render_one(&mut map_runtime, &mut ui_runtime)?;
        require_stable_counts(&mut stable_counts, counts)?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return Err("performance benchmark observed an invalid frame duration".into());
        }
        samples_ms.push(elapsed_ms);
    }
    if samples_ms.len() != options.sample_frames {
        return Err("performance benchmark did not collect the requested sample count".into());
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
    let counts = stable_counts.ok_or("performance benchmark collected no frame counts")?;
    if counts.map_operations != contract.expected_map_operations
        || counts.selected_present_operations != contract.expected_selected_present_operations
        || counts.selected_present_m02_draws != contract.expected_selected_present_m02_draws
    {
        return Err(
            "performance operation counts differ from the checked-in workload contract".into(),
        );
    }
    let result = if failures.is_empty() { "pass" } else { "fail" };
    let receipt = BenchReceipt {
        schema: "battle-plan-headless-performance-bench-v1",
        result,
        workload: Workload {
            recording_path: options.recording.display().to_string(),
            recording_sha256,
            workload_contract_path: options.workload_contract.display().to_string(),
            workload_contract_sha256: contract_sha256,
            workload_id: contract.workload_id,
            logical_stream_id: contract.logical_stream_id,
            recording_hour_count: hours.len(),
            first_hour: contract.first_hour,
            last_hour: contract.last_hour,
            selected_hour,
            content_manifest_digest_sha256: contract.content_manifest_digest_sha256,
            product_modset_sha256: contract.product_modset_sha256,
            effective_vfs_manifest_sha256: contract.effective_vfs_manifest_sha256,
            production_render_sha256,
            selected_present_sha256,
            launch_epoch_sha256: contract.launch_epoch_sha256,
            capture_receipt_sha256: contract.capture_receipt_sha256,
            viewport_width: REQUIRED_WIDTH,
            viewport_height: REQUIRED_HEIGHT,
            gpu_adapter_name: gpu_adapter.name,
            gpu_dedicated_video_memory_bytes: gpu_adapter.dedicated_video_memory_bytes,
            gpu_vendor_id: gpu_adapter.vendor_id,
            gpu_device_id: gpu_adapter.device_id,
            frame_cache_entries: options.frame_cache_entries,
            warmup_frames: options.warmup_frames,
            sample_frames: options.sample_frames,
            map_operations_per_frame: counts.map_operations,
            selected_present_operations_per_frame: counts.selected_present_operations,
            selected_present_m02_draws_per_frame: counts.selected_present_m02_draws,
            feature_local_plan_omissions: plan.omitted_data_needs.len(),
            feature_local_d3d_omissions: prepared.omissions.len(),
            gpu_completion_method: "D3D11_QUERY_EVENT_after_each_frame",
            timer: "std::time::Instant_wall_clock",
        },
        reference_machine_contract: "i5-8400_or_Ryzen_5_2600;16_GiB;GTX_1060_6_GiB_or_RX_580_8_GiB;1920x1080",
        thresholds: Thresholds {
            average_fps_min: 60.0,
            average_frame_time_ms_max: MAX_AVERAGE_FRAME_MS,
            p95_frame_time_ms_max: MAX_P95_FRAME_MS,
        },
        metrics,
        failures,
    };
    write_receipt(&options.output, &receipt)?;
    println!(
        "wrote headless performance receipt: {} (result {}, avg {:.3} fps, p95 {:.3} ms)",
        options.output.display(),
        receipt.result,
        receipt.metrics.average_fps,
        receipt.metrics.p95_frame_time_ms
    );
    if receipt.result != "pass" {
        return Err("headless performance thresholds failed; see machine-readable receipt".into());
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
    let shader_source = std::str::from_utf8(&shader_bytes)
        .map_err(|_| "stress shader asset is not UTF-8")?;
    let (vertex_shader, pixel_shader) = compile_stress_shaders(shader_source)?;
    let vertex_shader_sha256 = sha256_hex(&vertex_shader);
    let pixel_shader_sha256 = sha256_hex(&pixel_shader);

    let source = stress_used_content(
        &contract.shader_asset_path,
        &shader_bytes,
        &contract,
    )?;
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
    let (resolver, manifest) = build_stress_content(
        contract_root,
        &contract,
        &source,
        &expected_consumer,
    )?;
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

    let render_one = |map_runtime: &mut ProductProductionD3dRuntime| -> Result<(FrameCounts, String)> {
        gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;
        let map = map_runtime.execute(&gfx, &resolver, &resources, &prepared)?;
        if !map.presented_to_app_target || map.executed_operations == 0 {
            return Err("performance stress map route produced no app-target operations".into());
        }
        let ui_operations =
            ui_runtime.execute(&gfx, contract.selected_present_ui_operation_count)?;
        gpu_completion.wait(&gfx)?;
        Ok((
            FrameCounts {
                map_operations: map.executed_operations,
                selected_present_operations: ui_operations,
                selected_present_m02_draws: 0,
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
        failures.push(format!("average_fps {:.6} is below 60.0", metrics.average_fps));
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
        reference_machine_contract: "i5-8400_or_Ryzen_5_2600;16_GiB;GTX_1060_6_GiB_or_RX_580_8_GiB;1920x1080",
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

fn open_provider(mode: &TimelineMode) -> Result<(Box<dyn OwnerProvider>, String)> {
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
    let recording_sha256 = opened
        .authority_sha256
        .ok_or("performance benchmark requires one retained replay-container authority SHA-256")?;
    let (provider, _) = crate::timeline::build_recording_provider(
        None,
        opened.source,
        *timeline,
        &path.display().to_string(),
    )?;
    Ok((provider, recording_sha256))
}

fn validate_options(options: &BenchOptions) -> Result<()> {
    if options.frame_cache_entries == 0 {
        return Err("--frame-cache must be >= 1".into());
    }
    if options.warmup_frames < MIN_WARMUP_FRAMES {
        return Err(format!("--benchmark-warmup-frames must be >= {MIN_WARMUP_FRAMES}").into());
    }
    if options.sample_frames < MIN_SAMPLE_FRAMES {
        return Err(format!("--benchmark-sample-frames must be >= {MIN_SAMPLE_FRAMES}").into());
    }
    Ok(())
}

fn validate_contract(contract: &WorkloadContract) -> Result<()> {
    if contract.schema != "battle-plan-headless-performance-workload-v1" {
        return Err("unknown headless performance workload contract schema".into());
    }
    if !contract.certification_enabled {
        return Err("headless performance workload contract is not certification-enabled".into());
    }
    if contract.workload_id.trim().is_empty()
        || contract.recording_hour_count < MIN_CERTIFICATION_RECORDING_HOURS
        || contract.first_hour >= contract.last_hour
        || contract.selected_hour < contract.first_hour
        || contract.selected_hour > contract.last_hour
        || contract.expected_map_operations == 0
        || contract.expected_selected_present_operations == 0
        || contract.frame_cache_entries == 0
    {
        return Err("headless performance workload contract is sparse or incomplete".into());
    }
    for (label, value) in [
        ("recording authority", &contract.recording_authority_sha256),
        ("content manifest", &contract.content_manifest_digest_sha256),
        ("product modset", &contract.product_modset_sha256),
        ("effective VFS", &contract.effective_vfs_manifest_sha256),
        ("production render", &contract.production_render_sha256),
        ("selected-Present", &contract.selected_present_sha256),
        ("launch epoch", &contract.launch_epoch_sha256),
        ("capture receipt", &contract.capture_receipt_sha256),
    ] {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(format!("workload contract {label} SHA-256 is invalid").into());
        }
    }
    if !contract.approved_plan_omissions.is_empty()
        || !contract.approved_d3d_omissions.is_empty()
    {
        return Err("production F1 workload approves no plan or D3D omissions".into());
    }
    Ok(())
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
                || expected.selected_present_m02_draws != current.selected_present_m02_draws =>
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

fn write_receipt(path: &Path, receipt: &BenchReceipt) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    crate::headless::write_create_new_or_equal(path, &bytes, "headless performance receipt")
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
        ("synthetic product modset", &contract.synthetic_product_modset_sha256),
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
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                    .into(),
            build_profile: "performance_only_non_semantic".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: "f1-performance-stress".into(),
            playset_id: "f1-performance-stress".into(),
            selection_sha256: String::new(),
        },
        effective_vfs_manifest_sha256: contract
            .synthetic_effective_vfs_manifest_sha256
            .clone(),
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
    let resolver = map_viewer::GameFileResolver::vanilla(root)?.with_content_admission(&admission)?;
    Ok((resolver, manifest))
}

fn stress_used_content(
    relative_shader_path: &str,
    shader_bytes: &[u8],
    contract: &StressWorkloadContract,
) -> Result<replay_core::UsedContentKey> {
    let source_sha256 = sha256_hex(shader_bytes);
    let used = replay_core::UsedContentKey {
        effective_vfs_manifest_sha256: contract
            .synthetic_effective_vfs_manifest_sha256
            .clone(),
        normalized_game_relative_path: relative_shader_path.replace('\\', "/"),
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
        BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison,
        CompiledShaderPayload, CullMode, DepthDesc, DrawCommand, FillMode, GeometryBinding,
        IndexFormat, InputElementDesc, MapnameD80State, PipelineDesc, PreparedRenderTarget,
        PrimitiveTopology, RasterizerDesc, RenderExecutionRef, RenderOutputState, RenderTargetDesc,
        ShaderProgram, StaticGeometrySource, StaticMapCommand, StaticMapDraw, StaticMapOperation,
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
        effective_vfs_manifest_sha256: contract
            .synthetic_effective_vfs_manifest_sha256
            .clone(),
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
        Some(expected) if expected != &current => Err(format!("{label} changed between frames").into()),
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

    fn valid_recording_contract() -> WorkloadContract {
        let hash = "a".repeat(64);
        WorkloadContract {
            schema: "battle-plan-headless-performance-workload-v1".into(),
            certification_enabled: true,
            workload_id: "deterministic-fixture".into(),
            recording_authority_sha256: hash.clone(),
            logical_stream_id: Some(7),
            recording_hour_count: MIN_CERTIFICATION_RECORDING_HOURS,
            first_hour: 100,
            last_hour: 124,
            selected_hour: 124,
            content_manifest_digest_sha256: hash.clone(),
            product_modset_sha256: hash.clone(),
            effective_vfs_manifest_sha256: hash.clone(),
            production_render_sha256: hash.clone(),
            selected_present_sha256: hash.clone(),
            launch_epoch_sha256: hash.clone(),
            capture_receipt_sha256: hash,
            frame_cache_entries: 1,
            expected_map_operations: 1,
            expected_selected_present_operations: 1,
            expected_selected_present_m02_draws: 0,
            approved_plan_omissions: Vec::new(),
            approved_d3d_omissions: Vec::new(),
        }
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
        validate_count_authority(&root, &contract.ui_count_authority)
            .expect("UI count authority");

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
    fn recording_contract_schema_range_hashes_and_omissions_fail_closed() {
        validate_contract(&valid_recording_contract()).expect("valid recording contract");

        let mut contract = valid_recording_contract();
        contract.schema = "unknown".into();
        assert!(validate_contract(&contract).is_err());

        let mut contract = valid_recording_contract();
        contract.selected_hour = contract.last_hour + 1;
        assert!(validate_contract(&contract).is_err());

        let mut contract = valid_recording_contract();
        contract.capture_receipt_sha256 = "A".repeat(64);
        assert!(validate_contract(&contract).is_err());

        let mut contract = valid_recording_contract();
        contract.approved_d3d_omissions.push("unmeasured".into());
        assert!(validate_contract(&contract).is_err());
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
            repository_relative_path: contract
                .map_count_authority
                .repository_relative_path,
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
        assert!(require_stable_identity(
            &mut stable_identity,
            "b".repeat(64),
            "fixture receipt"
        )
        .is_err());
    }
}
