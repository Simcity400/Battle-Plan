//! Headless frame dump: render one timeline hour into an offscreen D3D11
//! target and write it as a BMP. No window, no screen interaction.
//!
//! This is the screen-free verification path for the map renderer and the
//! seed of the visual-QA harness (MAP_REPLICA_PLAN Phase 6): fixed-camera
//! renders that can be diffed across hours, builds, and eventually against
//! real the game screenshots.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use map_viewer::GameFileResolver;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::overlay::Overlay;
use crate::render::{validate_day_night_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE};
use crate::timeline::{build_provider, resolve_dump_hour, DumpHourRequest, OwnerProvider};
use crate::{Result, TimelineMode};

/// Exact quarter of the 5632x2048 map: full-map fit with zero letterbox,
/// so every pixel is map content and dumps diff cleanly.
const DUMP_WIDTH: u32 = 1408;
const DUMP_HEIGHT: u32 = 512;
static IMMUTABLE_PUBLISH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Machine-readable product-render observation consumed by the close-zoom
/// golden/pixel gate.  The execution receipt and BGRA pixels are bound in the
/// same record so a pixel hash can never be promoted from a different ordered
/// D3D execution.  This receipt is an observation, not engine parity authority.
#[derive(Debug, Serialize)]
struct ProductionG2CaptureBinding {
    selected_game_id: String,
    selected_index_path: String,
    selected_index_sha256: String,
    schema: &'static str,
    witness_path: String,
    witness_sha256: String,
    witness_bytes: u64,
    bmp_path: String,
    bmp_sha256: String,
    bmp_bytes: u64,
    capture_content_manifest_digest_sha256: String,
    replay_content_manifest_digest_sha256: String,
}

impl From<&replay_core::PackagedG2EngineCapture> for ProductionG2CaptureBinding {
    fn from(capture: &replay_core::PackagedG2EngineCapture) -> Self {
        Self {
            selected_game_id: capture.selected_game_id.clone(),
            selected_index_path: capture.selected_index_path.clone(),
            selected_index_sha256: capture.selected_index_sha256.clone(),
            schema: "hoi4-g2-packaged-engine-capture-v1",
            witness_path: capture.witness_path.clone(),
            witness_sha256: capture.witness_sha256.clone(),
            witness_bytes: capture.witness_bytes,
            bmp_path: capture.bmp_path.clone(),
            bmp_sha256: capture.bmp_sha256.clone(),
            bmp_bytes: capture.bmp_bytes,
            capture_content_manifest_digest_sha256: capture
                .capture_content_manifest_digest_sha256
                .clone(),
            replay_content_manifest_digest_sha256: capture
                .replay_content_manifest_digest_sha256
                .clone(),
        }
    }
}

/// Typed companion consumption bound to the same headless pixel-gate receipt
/// as the authenticated selected-Present operation frontier. These values do
/// not authorize extra drawing: the UI pass remains the sole pixel owner.
#[derive(Debug, Serialize)]
struct ProductionSelectedPresentCompanionReceipt {
    decision_panel_item_count: Option<usize>,
    power_balance_item_count: Option<usize>,
    intelligence_panel_count: Option<usize>,
    intelligence_unavailable_reason: Option<String>,
    agency_upgrade_count: Option<usize>,
    cipher_target_count: Option<usize>,
    trade_license_count: Option<usize>,
    xui15_surface_drawn: bool,
    xui15_unavailable_reason: Option<String>,
    occupation_state_count: Option<usize>,
    occupation_unavailable_reason: Option<String>,
    battleplan_order_count: Option<usize>,
    battleplan_selected_order_present: Option<bool>,
    remaining: crate::ui_runtime::SelectedPresentTypedConsumption,
}

#[derive(Debug, Serialize)]
struct ProductionPixelGateReceipt {
    schema: &'static str,
    render_boundary: &'static str,
    authority_exe_sha256: String,
    recording_sha256: String,
    production_render_sha256: String,
    selected_present_sha256: String,
    launch_epoch_sha256: String,
    capture_receipt_sha256: String,
    product_modset_sha256: String,
    effective_vfs_manifest_sha256: String,
    content_manifest_digest_sha256: String,
    settings_file_sha256: String,
    localisation_language: String,
    camera_payload_sha256: Option<String>,
    pov_country_or_global: &'static str,
    selected_hour: i64,
    viewport_width: u32,
    viewport_height: u32,
    map_operations: usize,
    selected_present_operations: usize,
    selected_present_m02_draws: usize,
    /// The G2 parity route suppresses selected-Present writers and therefore
    /// omits this key, preserving the exact v4 G2 receipt schema. Ordinary
    /// selected-Present dumps carry it as an additive typed-consumption
    /// extension of the same product observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_present_typed_companions: Option<ProductionSelectedPresentCompanionReceipt>,
    map_presented: bool,
    selected_present_executed: bool,
    receipt_sha256: Option<String>,
    bgra_sha256: String,
    pixel_byte_len: usize,
    g2_engine_capture: Option<ProductionG2CaptureBinding>,
}

fn production_pixel_gate_receipt_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".pixel-gate-receipt.json");
    PathBuf::from(path)
}

fn write_production_pixel_gate_receipt(
    out: &Path,
    receipt: &ProductionPixelGateReceipt,
) -> Result<PathBuf> {
    let path = production_pixel_gate_receipt_path(out);
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    // The BMP is published first.  The receipt is the authority/completion
    // marker, so expose it only after its complete durable bytes exist and
    // never replace a prior receipt with different content.
    write_create_new_or_equal(&path, &bytes, "production pixel-gate receipt")?;
    Ok(path)
}

fn snow_mud_h2_receipt_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".snow-mud-h2-receipt.json");
    PathBuf::from(path)
}

fn write_snow_mud_h2_receipt(
    out: &Path,
    receipt: &replay_core::SnowMudReplayReadbackReceipt,
) -> Result<PathBuf> {
    let path = snow_mud_h2_receipt_path(out);
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&path)? != bytes {
                return Err(format!(
                    "SnowMudData H2 receipt path already exists with different bytes: {}",
                    path.display()
                )
                .into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

fn occupation_secondary_h2_receipt_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".occupation-secondary-h2-receipt.json");
    PathBuf::from(path)
}

fn occupation_secondary_h2_payload_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".occupation-secondary-h2.tight-bgra8");
    PathBuf::from(path)
}

fn write_occupation_secondary_h2_evidence(
    out: &Path,
    receipt: &replay_core::OccupationSecondaryReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<(PathBuf, PathBuf)> {
    if tight_readback_bytes.len() as u64 != receipt.verified_tight_byte_count
        || format!("{:x}", Sha256::digest(tight_readback_bytes)) != receipt.readback_sha256
    {
        return Err("occupation secondary H2 sidecar bytes differ from the receipt".into());
    }
    let receipt_path = occupation_secondary_h2_receipt_path(out);
    let payload_path = occupation_secondary_h2_payload_path(out);
    let mut receipt_bytes = serde_json::to_vec_pretty(receipt)?;
    receipt_bytes.push(b'\n');
    write_create_new_or_equal(&payload_path, tight_readback_bytes, "occupation H2 payload")?;
    write_create_new_or_equal(&receipt_path, &receipt_bytes, "occupation H2 receipt")?;
    Ok((receipt_path, payload_path))
}

fn write_create_new_or_equal(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    if path.exists() {
        if std::fs::read(path)? == bytes {
            return Ok(());
        }
        return Err(format!(
            "{label} path already exists with different bytes: {}",
            path.display()
        )
        .into());
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{label} path has no file name: {}", path.display()))?
        .to_string_lossy();
    let (temporary, mut file) = (0..64)
        .find_map(|_| {
            let counter = IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{file_name}.{}.{}.tmp",
                std::process::id(),
                counter
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => Some(Ok((candidate, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()?
        .ok_or_else(|| format!("could not allocate atomic {label} staging file"))?;

    let staged = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    let result = staged.and_then(|()| {
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(path)? == bytes {
                    Ok(())
                } else {
                    Err(format!(
                        "{label} path was concurrently published with different bytes: {}",
                        path.display()
                    )
                    .into())
                }
            }
            Err(error) => Err(format!(
                "atomically publish {label} {}: {error}",
                path.display()
            )
            .into()),
        }
    });
    let _ = std::fs::remove_file(&temporary);
    result
}

/// Render `hour` (default: the timeline's first hour) and write a 32-bit BMP.
/// A supplied bundle selects the debug/capture renderer and overlay. A
/// bundle-less recording selects only the strict product D3D carrier and reads
/// the app-owned offscreen RTV; it never substitutes the debug renderer.

struct FrameKeying {
    owners: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    controllers: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    world_frame: Option<std::sync::Arc<replay_core::WorldFrame>>,
}

/// The data-only half of the real headless keying seam. It deliberately runs
/// before D3D initialization so selection and recording-load failures cannot
/// be replaced by a rendered all-underlay frame.
fn load_frame_keying(provider: &dyn OwnerProvider, hour: i64) -> Result<FrameKeying> {
    Ok(FrameKeying {
        owners: provider.owners_at(hour)?,
        controllers: provider.controllers_at(hour)?,
        world_frame: provider.world_frame_at(hour)?,
    })
}

/// DERIVED diagnostic formatting for geometry counts only. This is not a
/// rendered-pixel or parity assertion.
fn format_counter_geometry_counts(counts: [usize; 3]) -> String {
    format!(
        "counters {}/{}/{} inputs/instances/quads",
        counts[0], counts[1], counts[2]
    )
}

pub fn dump_frame(
    bundle: Option<&MapBundle>,
    mode: &TimelineMode,
    hour: Option<i64>,
    nth_hour: Option<usize>,
    out: &Path,
    _game_resolver: Option<&GameFileResolver>,
    product_paths: &crate::install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    day_night_hour_override: Option<f32>,
    water_full: bool,
) -> Result<()> {
    reject_recording_dump_debug_bundle(mode, bundle.is_some())?;
    match bundle {
        Some(bundle) => dump_debug_frame(
            bundle,
            mode,
            hour,
            out,
            product_paths,
            dump_zoom,
            dump_center,
            dump_hdr,
            capture_dir,
            postfx_enabled,
            postfx_restore_full,
            day_night_hour_override,
            water_full,
        ),
        None => dump_production_recording_frame(
            mode,
            hour,
            nth_hour,
            out,
            product_paths,
            dump_zoom,
            dump_center,
            dump_hdr,
            capture_dir,
            day_night_hour_override,
        ),
    }
}

fn reject_recording_dump_debug_bundle(mode: &TimelineMode, has_bundle: bool) -> Result<()> {
    if matches!(mode, TimelineMode::Recording { .. }) && has_bundle {
        return Err(
            "production recording --dump-frame forbids --bundle; no debug renderer substitute is permitted"
                .into(),
        );
    }
    Ok(())
}

fn dump_debug_frame(
    bundle: &MapBundle,
    mode: &TimelineMode,
    hour: Option<i64>,
    out: &Path,
    product_paths: &crate::install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    day_night_hour_override: Option<f32>,
    water_full: bool,
) -> Result<()> {
    let (provider, label) = build_provider(Some(bundle), mode)?;
    let (game_resolver, _content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let hours = provider.hours();
    if hours.is_empty() {
        return Err("timeline provider has no hours".into());
    }
    let hour = match hour {
        Some(hour) if matches!(mode, TimelineMode::Recording { .. }) => {
            resolve_dump_hour(hours, DumpHourRequest::Canonical(hour))?
        }
        Some(hour) => hour,
        None => hours[0],
    };
    let keying = load_frame_keying(provider.as_ref(), hour)?;

    let gfx = Gfx::create_offscreen(DUMP_WIDTH, DUMP_HEIGHT)?;
    if let Some(day_night_hour) = day_night_hour_override {
        validate_day_night_hour(day_night_hour)?;
    }
    let mut renderer = MapRenderer::new(
        &gfx,
        bundle,
        bundle.max_state_id(),
        capture_dir,
        Some(&game_resolver),
        postfx_enabled,
        postfx_restore_full,
        water_full,
    )?;
    let mut overlay = Overlay::create(&gfx, Some(&game_resolver));
    let owners = keying.owners;
    let controllers = keying.controllers;
    let frame = keying.world_frame;
    renderer.update_state_politics(
        &gfx,
        bundle,
        frame.as_deref(),
        &owners,
        &controllers,
        &bundle.country_colors,
    )?;
    let mut camera = Camera::fit(
        bundle.width,
        bundle.height,
        f64::from(DUMP_WIDTH),
        f64::from(DUMP_HEIGHT),
    );
    if let Some(zoom) = dump_zoom {
        camera.zoom = zoom;
    }
    if let Some((x, y)) = dump_center {
        camera.center_x = x;
        camera.center_y = y;
    }
    camera.normalize(f64::from(DUMP_HEIGHT));
    // Day/night terminator shading is opt-in via --hour; the default keeps
    // the captured CB0 state (off), matching the game's default map view.
    let day_night_hour = day_night_hour_override;
    renderer.render(&gfx, &camera, day_night_hour, None)?;
    // COLOR-lane debug readback (--dump-hdr): the rendered HDR MainScene
    // (captured-pass RGBA16F output, capture-native dimensions, BEFORE the
    // final composite/postfx). Never a render input.
    if let Some(hdr_path) = dump_hdr {
        renderer.dump_hdr_mainscene(&gfx, hdr_path)?;
    }
    // Same overlay as app.rs so the dump verifies the real product path:
    // line 1 the raw Clausewitz date + hour (GAP: localisation-formatted
    // dates are a later slice), line 2 mode + play state + probe. Headless
    // has no cursor, so the probe shows the empty readout.
    let hour_label = provider.hour_label(hour)?;
    let hour_index = match hours.binary_search(&hour) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    };
    let line2 =
        format!("{label} | {POLITICAL_COLOR_BLOCKER_BADGE} | prov - state - owner - controller -");
    overlay.draw(&gfx, &hour_label, &line2, hour_index, hours.len())?;
    let pixels = gfx.read_target()?;
    write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;
    let snow_mud_h2_receipt_path = renderer
        .snow_mud_h2_receipt()
        .map(|receipt| write_snow_mud_h2_receipt(out, receipt))
        .transpose()?;
    let day_night_label = match day_night_hour {
        Some(hour) => format!("{hour:.6}"),
        None => "off".to_string(),
    };
    println!(
        "wrote frame dump: {} ({label}, {}, {DUMP_WIDTH}x{DUMP_HEIGHT}, zoom {:.3}, center {:.1},{:.1}, daynight {day_night_label}, postfx {}, restore_full {}, {} controlled states, \
         overlay text {})",
        out.display(),
        hour_label,
        camera.zoom,
        camera.center_x,
        camera.center_y,
        if postfx_enabled { "on" } else { "off" },
        if postfx_restore_full { "on" } else { "off" },
        controllers.len(),
        if overlay.has_text() { "on" } else { "off" }
    );
    if let Some(path) = snow_mud_h2_receipt_path {
        println!(
            "wrote SnowMudData H2 immutable-upload/readback receipt: {}",
            path.display()
        );
    }
    if let Some(stats) = renderer.captured_pass_stats() {
        let stats_day_night = match stats.last_day_night_hour {
            Some(hour) => format!("{hour:.6}"),
            None => "off".to_string(),
        };
        println!(
            "captured pass: session {}, terrain {}x{} format {}, water {}, rivers {}, trees {}, postfx {}, restore_full {}, restore_evidence {}, daynight {stats_day_night}, last render {:.2} ms",
            stats.session_dir.display(),
            stats.target_width,
            stats.target_height,
            stats.target_format,
            if stats.has_water { "on" } else { "off" },
            if stats.has_rivers { "on" } else { "off" },
            if stats.has_trees { "on" } else { "off" },
            if stats.postfx_enabled { "on" } else { "off" },
            if stats.postfx_restore_full { "on" } else { "off" },
            if stats.restore_evidence_found { "found" } else { "none" },
            stats.last_render_ms.unwrap_or(0.0)
        );
        println!(
            "captured pass submitted-stream omissions: particles {}, entities {}, direct_visuals {}, weather {}",
            if stats.particle_stream_omitted {
                "omitted"
            } else {
                "none"
            },
            if stats.entity_stream_omitted {
                "omitted"
            } else {
                "none"
            },
            if stats.direct_visual_stream_omitted {
                "omitted"
            } else {
                "none"
            },
            if stats.weather_stream_omitted {
                "omitted"
            } else {
                "none"
            },
        );
        if let Some(water_gap) = &stats.water_gap {
            println!("captured pass water gap: {water_gap}");
        }
        for gap in &stats.layer_gaps {
            println!("captured pass layer gap: {gap}");
        }
    }
    Ok(())
}

/// Render one portable recording hour through the same app-owned D3D11
/// device/context/RTV route as the product window, then read that exact RTV.
/// No debug bundle, captured pass, proxy map, camera, HUD, or overlay is
/// admitted into this path.
fn dump_production_recording_frame(
    mode: &TimelineMode,
    hour: Option<i64>,
    nth_hour: Option<usize>,
    out: &Path,
    product_paths: &crate::install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    day_night_hour_override: Option<f32>,
) -> Result<()> {
    let (provider, label, recording_sha256) = match mode {
        TimelineMode::Recording {
            path,
            logical_stream_id,
            timeline,
        } => {
            let opened = crate::recording_open::open_product_recording_segment_with_authority(
                path,
                *logical_stream_id,
            )
            .map_err(|error| format!("open recording {}: {error}", path.display()))?;
            let recording_sha256 = opened.authority_sha256.ok_or(
                "production frame receipt requires a single retained replay-container authority",
            )?;
            let (provider, label) = crate::timeline::build_recording_provider(
                None,
                opened.source,
                *timeline,
                &path.display().to_string(),
            )?;
            (provider, label, recording_sha256)
        }
        TimelineMode::Static1936 => {
            return Err(
                "bundle-less --dump-frame is a production recording command and requires --recording"
                    .into(),
            )
        }
    };
    reject_production_dump_debug_inputs(
        dump_zoom,
        dump_center,
        dump_hdr,
        capture_dir,
        day_night_hour_override,
    )?;

    let (game_resolver, content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let content_manifest = provider
        .content_manifest()
        .ok_or("portable playback has no admitted content manifest after binding")?;
    let admitted_g2_capture = provider.g2_engine_capture().map(ProductionG2CaptureBinding::from);
    let hours = provider.hours();
    let selected_hour = match (hour, nth_hour) {
        (Some(hour), None) => resolve_dump_hour(hours, DumpHourRequest::Canonical(hour))?,
        (None, Some(ordinal)) => resolve_dump_hour(hours, DumpHourRequest::Ordinal(ordinal))?,
        (None, None) => *hours.first().ok_or("timeline provider has no hours")?,
        (Some(_), Some(_)) => {
            return Err("competing canonical and ordinal dump-hour selectors".into())
        }
    };
    let initial_frame = provider
        .world_frame_at(hours[0])?
        .ok_or("initial replay frame is unavailable")?;
    let initial_display = initial_frame
        .session_metadata()
        .display_variant
        .measured()
        .ok_or("replay lacks authenticated initial display_variant")?;
    let output_width = initial_display.physical_width;
    let output_height = initial_display.physical_height;
    if output_width == 0 || output_height == 0 {
        return Err("recorded physical client extent is empty".into());
    }

    let frame = provider
        .world_frame_at(selected_hour)?
        .ok_or("selected exact replay frame is unavailable")?;
    let production_render = frame
        .production_render()
        .ok_or("selected replay hour has no authenticated production_render carrier")?;
    let production_render_sha256 =
        format!("{:x}", Sha256::digest(production_render.encode_json()?));
    let selected_present_sha256 = production_render.selected_present.computed_sha256()?;
    let selected_submit = production_render
        .submits
        .first()
        .ok_or("authenticated production_render has no selected submit")?;
    let camera_payload_sha256 = selected_submit
        .camera_carrier
        .as_ref()
        .map(|camera| camera.payload_sha256.clone());
    let content_manifest_digest_sha256 = content_manifest
        .manifest_digest_sha256
        .clone()
        .ok_or("portable playback content manifest has no finalized digest")?;
    let mut production_map = map_viewer::ProductionMapRenderer::new(
        &game_resolver,
        &content_admission,
        content_manifest,
    )?;
    let production_plan = production_map
        .prepare_frame_with_rail(
            Some(production_render),
            Some(frame.rail_replay()),
            Some(frame.production_by_country()),
        )?;
    let prepared =
        map_viewer::prepare_production_d3d_frame(&production_plan, production_map.resources())?;
    for message in production_optional_omission_messages(&production_plan, &prepared) {
        eprintln!("replay_app: {message}");
    }
    let gfx = Gfx::create_offscreen(output_width, output_height)?;
    gfx.clear_target([0.0, 0.0, 0.0, 1.0])?;
    let has_map_route = prepared.target.is_some() && !prepared.ordered_render_operations.is_empty();
    let (
        map_operations,
        map_receipt,
        map_presented,
        snow_mud_proof,
        occupation_secondary_proof,
    ) = if has_map_route {
        let mut runtime = crate::app::ProductProductionD3dRuntime::default();
        let outcome =
            runtime.execute_with_snow_mud_h2(
                &gfx,
                &game_resolver,
                production_map.resources(),
                &prepared,
            )?;
        (
            outcome.executed_operations,
            outcome.receipt_sha256,
            outcome.presented_to_app_target,
            outcome.snow_mud_proof,
            outcome.occupation_secondary_proof,
        )
    } else {
        (0, "none".into(), false, None, None)
    };
    let snow_mud_h2_receipt = match (
        production_render.snow_mud_texture.as_ref(),
        snow_mud_proof.as_ref(),
    ) {
        (None, None) => None,
        (Some(packet), Some(proof)) => {
            let receipt = crate::render::seal_product_snow_mud_h2_receipt(
                packet,
                &production_render.selected_present,
                proof,
            )?;
            crate::render::persist_snow_mud_h2_receipt(
                &receipt,
                &proof.tight_readback_bytes,
            )?;
            Some(receipt)
        }
        (Some(_), None) => {
            return Err("production SnowMudData H packet produced no product H2 proof".into())
        }
        (None, Some(_)) => {
            return Err("production SnowMudData product H2 proof has no H packet".into())
        }
    };
    let mut occupation_packets = production_render
        .submits
        .iter()
        .flat_map(|submit| submit.political_textures.iter())
        .filter(|packet| {
            packet.kind == replay_core::PoliticalTextureKind::Secondary
                && packet.target_binding_identity
                    == replay_core::OCCUPATION_SECONDARY_TARGET_BINDING
        });
    let occupation_packet = occupation_packets.next();
    if occupation_packets.next().is_some() {
        return Err("production recording has repeated occupation secondary packets".into());
    }
    let occupation_secondary_h2_receipt =
        match (occupation_packet, occupation_secondary_proof.as_ref()) {
            (None, None) => None,
            (Some(packet), Some(proof)) => {
                let receipt = crate::render::seal_product_occupation_secondary_h2_receipt(
                    packet,
                    &production_render.selected_present,
                    proof,
                )?;
                crate::render::persist_occupation_secondary_h2_receipt(
                    &receipt,
                    &proof.tight_readback_bytes,
                )?;
                Some(receipt)
            }
            (Some(_), None) => {
                return Err(
                    "production occupation secondary packet produced no product H2 proof".into(),
                )
            }
            (None, Some(_)) => {
                return Err(
                    "production occupation secondary H2 proof has no portable packet".into(),
                )
            }
        };

    let mut selected_present_operations = 0usize;
    let mut selected_present_m02_draws = 0usize;
    let mut selected_present_typed_companions = None;
    let mut selected_present_executed = false;
    if admitted_g2_capture.is_none() {
        if let Some(pass) = prepared.authenticated_ui_pass.as_ref() {
            let mut runtime =
                crate::ui_runtime::ProductSelectedPresentRuntime::new_with_decision_catalog_bytes(
                    &gfx,
                    &game_resolver,
                    &content_admission,
                    pass,
                    provider.decision_catalog().map(|catalog| catalog.bytes()),
                )?;
            let runtime_text = match frame.runtime_text()? {
                replay_core::Leaf::Measured(catalog) => Some(catalog),
                replay_core::Leaf::Absent | replay_core::Leaf::Unobtained => None,
            };
            match runtime.execute(
                &gfx,
                &game_resolver,
                &prepared,
                frame.as_ref(),
                None,
                runtime_text.as_ref(),
                crate::ui_runtime::ArmyBattleplanUiSupplementalInputs::default(),
                crate::ui_runtime::IntelligenceUiSupplementalInputs::default(),
                crate::ui_runtime::OccupationUiSupplementalInputs::default(),
            )? {
                crate::ui_runtime::SelectedPresentUiOutcome::Executed {
                    operation_count,
                    m02_draw_count,
                    decision_panel_item_count,
                    power_balance_item_count,
                    intelligence_panel_count,
                    intelligence_unavailable_reason,
                    agency_upgrade_count,
                    cipher_target_count,
                    trade_license_count,
                    xui15_surface_drawn,
                    xui15_unavailable_reason,
                    occupation_state_count,
                    occupation_unavailable_reason,
                    battleplan_order_count,
                    battleplan_selected_order_present,
                    typed_consumption,
                } => {
                    selected_present_operations = operation_count;
                    selected_present_m02_draws = m02_draw_count;
                    selected_present_typed_companions = Some(
                        ProductionSelectedPresentCompanionReceipt {
                            decision_panel_item_count,
                            power_balance_item_count,
                            intelligence_panel_count,
                            intelligence_unavailable_reason,
                            agency_upgrade_count,
                            cipher_target_count,
                            trade_license_count,
                            xui15_surface_drawn,
                            xui15_unavailable_reason,
                            occupation_state_count,
                            occupation_unavailable_reason,
                            battleplan_order_count,
                            battleplan_selected_order_present,
                            remaining: typed_consumption,
                        },
                    );
                    selected_present_executed = true;
                }
                crate::ui_runtime::SelectedPresentUiOutcome::Omitted(reason) => {
                    eprintln!(
                        "replay_app: production dump feature omitted [SELECTED_PRESENT_UI] {reason}"
                    );
                }
            }
        }
    }
    if !map_presented && !selected_present_executed {
        return Err(
            "production recording dump has neither authenticated map output nor executable selected-Present UI output"
                .into(),
        );
    }
    if map_operations == 0 && selected_present_operations == 0 {
        return Err("production recording dump executed zero authenticated draws".into());
    }
    let pixels = gfx.read_target()?;
    let pixel_sha256 = format!("{:x}", Sha256::digest(&pixels));
    write_bmp(out, output_width, output_height, &pixels)?;
    let snow_mud_h2_receipt_path = snow_mud_h2_receipt
        .as_ref()
        .map(|receipt| write_snow_mud_h2_receipt(out, receipt))
        .transpose()?;
    let occupation_secondary_h2_paths = occupation_secondary_h2_receipt
        .as_ref()
        .zip(occupation_secondary_proof.as_ref())
        .map(|(receipt, proof)| {
            write_occupation_secondary_h2_evidence(
                out,
                receipt,
                &proof.tight_readback_bytes,
            )
        })
        .transpose()?;
    let pixel_gate_receipt = ProductionPixelGateReceipt {
        schema: "battle-plan-production-pixel-gate-receipt-v4",
        render_boundary: "post_authenticated_map_bgra8_no_selected_present_writers",
        authority_exe_sha256: content_manifest.source_game.executable_sha256.clone(),
        recording_sha256,
        production_render_sha256,
        selected_present_sha256,
        launch_epoch_sha256: production_render.authority.launch_epoch_sha256.clone(),
        capture_receipt_sha256: production_render.authority.capture_receipt_sha256.clone(),
        product_modset_sha256: content_manifest.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: content_manifest
            .effective_vfs_manifest_sha256
            .clone(),
        content_manifest_digest_sha256,
        settings_file_sha256: initial_display.settings_file_sha256.clone(),
        localisation_language: content_manifest.localisation_language.clone(),
        camera_payload_sha256,
        // The headless production command has no POV selector and invokes the
        // selected-Present runtime with `None`; its only admissible POV is the
        // explicit omniscient/global route.
        pov_country_or_global: "global",
        selected_hour,
        viewport_width: output_width,
        viewport_height: output_height,
        map_operations,
        selected_present_operations,
        selected_present_m02_draws,
        selected_present_typed_companions,
        map_presented,
        selected_present_executed,
        receipt_sha256: (map_receipt != "none").then(|| map_receipt.clone()),
        bgra_sha256: pixel_sha256.clone(),
        pixel_byte_len: pixels.len(),
        g2_engine_capture: admitted_g2_capture,
    };
    let pixel_gate_receipt_path =
        write_production_pixel_gate_receipt(out, &pixel_gate_receipt)?;
    println!(
        "wrote production recording frame dump: {} ({label}, hour {selected_hour}, {}x{}, {} map operations, {} selected-Present operations including {} M02 draws, map receipt {}, bgra_sha256 {}, pixel gate receipt {})",
        out.display(),
        output_width,
        output_height,
        map_operations,
        selected_present_operations,
        selected_present_m02_draws,
        map_receipt,
        pixel_sha256,
        pixel_gate_receipt_path.display(),
    );
    if let Some(path) = snow_mud_h2_receipt_path {
        println!(
            "wrote product SnowMudData H2 immutable-upload/readback receipt: {}",
            path.display()
        );
    }
    if let Some((receipt_path, payload_path)) = occupation_secondary_h2_paths {
        println!(
            "wrote product occupation secondary H2 receipt {} and raw readback {}",
            receipt_path.display(),
            payload_path.display()
        );
    }
    Ok(())
}

fn production_optional_omission_messages(
    plan: &map_viewer::PreparedProductionMapFrame,
    prepared: &map_viewer::PreparedProductionD3dFrame,
) -> Vec<String> {
    let mut messages = plan
        .omitted_data_needs
        .iter()
        .map(|need| {
            format!(
                "production dump feature omitted [{}] {}: {}",
                need.packet, need.feature, need.reason
            )
        })
        .chain(prepared.omissions.iter().map(|omission| {
            format!(
                "production dump feature omitted [PRODUCTION_MAP_D3D] {}: {}",
                omission.pass, omission.reason
            )
        }))
        .collect::<Vec<_>>();
    if prepared.authenticated_ui_pass.is_none() {
        messages.push(
            "production dump feature omitted [SELECTED_PRESENT_UI] authenticated selected-Present UI/M02 carrier is absent"
                .into(),
        );
    }
    messages
}

fn reject_production_dump_debug_inputs(
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    day_night_hour_override: Option<f32>,
) -> Result<()> {
    if dump_zoom.is_some()
        || dump_center.is_some()
        || dump_hdr.is_some()
        || capture_dir.is_some()
        || day_night_hour_override.is_some()
    {
        return Err(
            "production recording --dump-frame forbids debug camera, HDR, capture, and day/night inputs"
                .into(),
        );
    }
    Ok(())
}

/// Minimal 32-bit BI_RGB BMP writer. `pixels` are top-down BGRA rows (the
/// readback layout); BMP stores rows bottom-up.
fn write_bmp(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let row_bytes = width as usize * 4;
    if pixels.len() != row_bytes * height as usize {
        return Err(format!(
            "pixel buffer is {} bytes, expected {}",
            pixels.len(),
            row_bytes * height as usize
        )
        .into());
    }
    let image_bytes = pixels.len() as u32;
    let file_bytes = 54 + image_bytes;
    let mut out = Vec::with_capacity(file_bytes as usize);
    // BITMAPFILEHEADER
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_bytes.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
                                                 // BITMAPINFOHEADER
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&image_bytes.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // palette
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..height as usize).rev() {
        out.extend_from_slice(&pixels[y * row_bytes..(y + 1) * row_bytes]);
    }
    write_create_new_or_equal(path, &out, "headless BGRA BMP")
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapshot::{pack, PackEntry, PackIdentity};

    #[test]
    fn immutable_authority_publication_is_atomic_idempotent_and_no_replace() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-g2-publish-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("receipt.json");
        write_create_new_or_equal(&path, b"complete receipt bytes\n", "test receipt").unwrap();
        write_create_new_or_equal(&path, b"complete receipt bytes\n", "test receipt").unwrap();
        assert!(write_create_new_or_equal(&path, b"different\n", "test receipt").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"complete receipt bytes\n");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn packed_political_keying_recording() -> std::path::PathBuf {
        let mut states = serde_json::Map::new();
        for state_id in 1..=1_081u32 {
            let owner = if state_id <= 216 { "FRA" } else { "GER" };
            let controller = if state_id <= 8 { "GER" } else { owner };
            states.insert(
                state_id.to_string(),
                serde_json::json!({ "owner": owner, "controller": controller }),
            );
        }
        let mod_identity = serde_json::json!({
            "modset_id": "vanilla",
            "source": "headless_keying_test",
            "load_order": [],
            "mods": [],
        });
        let snapshot = serde_json::json!({
            "schema_version": "1",
            "game_id": "headless_1081_keying",
            "date": "1936.1.1.13",
            "hour_index": 16_959_373,
            "save_version": null,
            "provenance": {
                "collector": null,
                "memory": null,
                "mod": mod_identity,
                "save": null,
                "notes": [],
            },
            "countries": {},
            "states": states,
            "provinces": {},
            "divisions": [],
            "combats": [],
            "diplomacy_world": {},
            "map": {},
            "designs": { "division_templates": [], "equipment_variants": {} },
            "order_of_battle": {
                "army_groups": [], "armies": [], "fleets": [], "navy_theaters": [],
                "air_wings": [], "theatres": [], "ships": [], "by_country": {},
            },
            "mio_live": {}, "market_live": {}, "special_projects": {},
            "diplomacy_live": {}, "naval_air": {}, "supply": {},
            "strategic_air": {}, "strategic_operatives": {}, "raids": {},
            "world_events": {}, "world": {}, "weather": {}, "visibility": {},
            "naval_combats": [], "models": {}, "intel_ledger": {}, "events": [],
        });
        let index = serde_json::json!({
            "game_id": "headless_1081_keying",
            "schema_version": "1",
            "snapshots": [{
                "hour_index": 16_959_373,
                "date": "1936.1.1.13",
                "file": "000016959373.snapshot.json",
            }],
        });
        let entries = vec![
            PackEntry {
                path: "payload/headless_1081_keying/index.json".to_string(),
                bytes: serde_json::to_vec(&index).expect("serialize index"),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: "payload/headless_1081_keying/000016959373.snapshot.json".to_string(),
                bytes: serde_json::to_vec(&snapshot).expect("serialize snapshot"),
                role: "snapshot",
                hour_index: Some(16_959_373),
            },
        ];
        let bytes = pack(
            "headless_1081_keying",
            &entries,
            &PackIdentity {
                schema_version: Some("1".to_string()),
                mod_identity: Some(mod_identity),
                ..Default::default()
            },
            true,
        )
        .expect("pack keying recording");
        let path = crate::bundle::tests::unique_temp_dir("recording_keying")
            .join("headless_1081_keying.battleplan");
        std::fs::create_dir_all(path.parent().expect("recording parent"))
            .expect("create recording parent");
        std::fs::write(&path, bytes).expect("write keying recording");
        path
    }

    #[test]
    fn recording_keying_seam_receives_all_owners_and_controller_overrides_without_d3d() {
        let path = packed_political_keying_recording();
        let source = replay_core::open_recording(&path).expect("open packed keying recording");
        let timeline = replay_core::WorldTimeline::open(source).expect("keying timeline");
        let provider = crate::timeline::RecordingTimeline::new_with_policy(timeline, None);
        let keying = load_frame_keying(&provider, 16_959_373).expect("headless keying frame");

        assert_eq!(keying.owners.len(), 1_081);
        assert_eq!(keying.controllers.len(), 1_081);
        assert_eq!(
            keying
                .controllers
                .iter()
                .filter(|(state_id, controller)| keying.owners.get(state_id) != Some(controller))
                .count(),
            8,
            "controller overrides must survive the real pre-render seam"
        );
        assert_eq!(
            keying
                .world_frame
                .as_ref()
                .expect("typed world frame")
                .hour_index(),
            16_959_373
        );
        std::fs::remove_dir_all(path.parent().expect("recording parent"))
            .expect("remove keying recording");
    }

    #[test]
    fn production_recording_dump_rejects_every_debug_substitute() {
        assert!(reject_production_dump_debug_inputs(None, None, None, None, None).is_ok());
        assert!(reject_production_dump_debug_inputs(Some(1.0), None, None, None, None).is_err());
        assert!(
            reject_production_dump_debug_inputs(None, Some((1.0, 2.0)), None, None, None).is_err()
        );
        assert!(reject_production_dump_debug_inputs(
            None,
            None,
            Some(Path::new("scene.hdr")),
            None,
            None
        )
        .is_err());
        assert!(reject_production_dump_debug_inputs(
            None,
            None,
            None,
            Some(Path::new("capture")),
            None
        )
        .is_err());
        assert!(reject_production_dump_debug_inputs(None, None, None, None, Some(0.5)).is_err());
    }

    #[test]
    fn production_recording_dump_rejects_debug_bundle_route() {
        let mode = TimelineMode::Recording {
            path: "portable.battleplan".into(),
            logical_stream_id: None,
            timeline: replay_core::TimelineConfig::default(),
        };
        assert!(reject_recording_dump_debug_bundle(&mode, false).is_ok());
        assert!(reject_recording_dump_debug_bundle(&mode, true).is_err());
        assert!(reject_recording_dump_debug_bundle(&TimelineMode::Static1936, true).is_ok());
    }

    #[test]
    fn optional_packet_and_d3d_omissions_are_reported_without_rejecting_the_frame() {
        let plan = map_viewer::PreparedProductionMapFrame {
            content_manifest_sha256: "11".repeat(32),
            product_modset_sha256: "99".repeat(32),
            effective_vfs_manifest_sha256: "22".repeat(32),
            selected_submit_index: None,
            selected_submit_global_order: None,
            c03_camera_replay: None,
            ordered_scene_constant_buffers: Vec::new(),
            ordered_static_map_programs: Vec::new(),
            rail_ribbon: None,
            snow_mud_texture: None,
            tree_runtime: None,
            s1_shadow: None,
            ordered_render_operations: Vec::new(),
            political_textures: Vec::new(),
            tiled_lights: Vec::new(),
            ordered_runtime_draws: Vec::new(),
            m02_combat: None,
            authenticated_ui_pass: None,
            m03_arrows: None,
            rail_supply_arrows: None,
            o04_cities: None,
            o05_buildings: None,
            omitted_data_needs: vec![map_viewer::ProductionMapDataNeed {
                packet: "M02".into(),
                feature: "combat indicators".into(),
                reason: "carrier absent".into(),
            }],
        };
        let mut prepared = map_viewer::PreparedProductionD3dFrame {
            content_manifest_sha256: "11".repeat(32),
            product_modset_sha256: "99".repeat(32),
            effective_vfs_manifest_sha256: "22".repeat(32),
            c03_camera_replay: None,
            target: None,
            ordered_scene_constant_buffers: Vec::new(),
            ordered_render_operations: Vec::new(),
            snow_mud_texture: None,
            political_textures: Vec::new(),
            tiled_lights: Vec::new(),
            ordered_static_used_content: Vec::new(),
            ordered_static_programs: Vec::new(),
            rail_ribbon: None,
            mapname_d80_state: replay_core::MapnameD80State::Missing,
            tree_runtime: None,
            tree_materialized_execution_sha256: None,
            s1_shadow: None,
            ordered_runtime_draws: Vec::new(),
            m02_combat: None,
            authenticated_ui_pass: None,
            m03_arrows: None,
            rail_supply_arrows: None,
            omissions: vec![map_viewer::ProductionD3dOmission {
                pass: "optional-pass".into(),
                reason: "unsupported optional interpreter".into(),
            }],
        };
        let messages = production_optional_omission_messages(&plan, &prepared);
        assert_eq!(messages.len(), 3);
        assert!(messages[0].contains("[M02]"));
        assert!(messages[1].contains("[PRODUCTION_MAP_D3D]"));
        assert!(messages[2].contains("[SELECTED_PRESENT_UI]"));
        assert!(messages[2].contains("carrier is absent"));

        prepared.authenticated_ui_pass = Some(replay_core::AuthenticatedUiPass {
            schema: replay_core::UI_PASS_SCHEMA.into(),
            update_index: 1,
            first_sequence: 1,
            last_sequence: 1,
            ui_pass_draw_count: 0,
            target: replay_core::RenderTargetDesc {
                prepared_target: replay_core::PreparedRenderTarget::SwapchainBackBuffer,
                width: 1920,
                height: 1080,
                dxgi_format_u32: 28,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            },
            selected_present_receipt_sha256: "00".repeat(32),
            producer_selection_receipt_sha256: "00".repeat(32),
            source_frontier_sha256: "00".repeat(32),
            recorded_input_receipt_sha256: "00".repeat(32),
            active_mapmode_state: None,
            ui_pass_operations: Vec::new(),
            pass_identity_sha256: "00".repeat(32),
        });
        let messages = production_optional_omission_messages(&plan, &prepared);
        assert_eq!(messages.len(), 2);
        assert!(messages
            .iter()
            .all(|message| !message.contains("[SELECTED_PRESENT_UI]")));
    }

    #[test]
    fn headless_source_suppresses_selected_present_for_admitted_g2_and_orders_non_g2_ui() {
        let source = include_str!("headless.rs");
        let production = &source[source
            .find("fn dump_production_recording_frame(")
            .expect("production recording dump function")..];
        let g2_guard = production
            .find("if admitted_g2_capture.is_none()")
            .expect("admitted G2 selected-Present suppression guard");
        let selected_start = production
            .find("ProductSelectedPresentRuntime::new")
            .expect("headless selected-Present runtime");
        let execute = selected_start
            + production[selected_start..]
                .find("runtime.execute(")
                .expect("headless selected-Present execution route");
        let readback = production
            .find("let pixels = gfx.read_target()?")
            .expect("headless exact target readback");
        assert!(g2_guard < selected_start && execute < readback);
        assert!(production.contains("g2_engine_capture: admitted_g2_capture"));
        let forbidden_legacy_omission = ["interactive Present", "-stage surface"].concat();
        assert!(!production.contains(&forbidden_legacy_omission));
        assert!(production.contains("selected_present_m02_draws"));
    }
}
