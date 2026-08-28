//! A product recording dump executes the same XMAP22 packet-native composition used by windowed playback.
//! It selects one timeline hour before publishing the final BMP and its
//! content-bound composition receipt.
//!
//! Retained bundle-based and explicit evidence helpers remain non-product
//! screen-free tooling; they may render/read back evidence without authorizing
//! product-map output.
//!
//! GATE INPUT: xmap00's shipping_help foundation-gate test requires the first
//! line of this header verbatim in the file's prelude; do not delete it
//! without re-anchoring that test.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use map_viewer::GameFileResolver;
use sha2::{Digest, Sha256};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::overlay::Overlay;
use crate::render::{validate_day_night_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE};
use crate::timeline::{build_provider, resolve_dump_hour, DumpHourRequest, OwnerProvider};
use crate::{Result, TimelineMode};

const DUMP_WIDTH: u32 = 1408;
const DUMP_HEIGHT: u32 = 512;
static IMMUTABLE_PUBLISH_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn verify_xmap22_dump(path: &Path) -> Result<()> {
    let receipt_path = xmap22_receipt_path(path);
    let receipt_bytes = std::fs::read(&receipt_path).map_err(|error| {
        format!(
            "read XMAP22 composition receipt {}: {error}",
            receipt_path.display()
        )
    })?;
    let receipt = crate::app::ProductMapCompositionReceipt::decode_json(
        &receipt_bytes,
        &format!("XMAP22 composition receipt {}", receipt_path.display()),
    )?;
    receipt.validate()?;
    let (width, height, pixels) = read_xmap22_bmp(path)?;
    if width != receipt.viewport_width || height != receipt.viewport_height {
        return Err(format!(
            "XMAP22 BMP dimensions {width}x{height} disagree with receipt {}x{}",
            receipt.viewport_width, receipt.viewport_height
        )
        .into());
    }
    let observed = snapshot::sha256::Sha256::hex(&pixels);
    if observed != receipt.final_pixels_sha256 {
        return Err(format!(
            "XMAP22 BMP pixels hash to {observed}, expected {}",
            receipt.final_pixels_sha256
        )
        .into());
    }
    println!(
        "verified XMAP22 dump: {} (receipt {})",
        path.display(),
        receipt.deterministic_sha256
    );
    Ok(())
}

fn xmap22_receipt_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".xmap22.json");
    PathBuf::from(value)
}

fn read_xmap22_bmp(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("read XMAP22 BMP {}: {error}", path.display()))?;
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return Err("XMAP22 dump is not a complete BMP".into());
    }
    let u16_at = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let u32_at = |offset: usize| {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let i32_at = |offset: usize| {
        i32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let pixel_offset = usize::try_from(u32_at(10))?;
    let width_i32 = i32_at(18);
    let height_i32 = i32_at(22);
    if u32_at(14) != 40
        || width_i32 <= 0
        || height_i32 <= 0
        || u16_at(26) != 1
        || u16_at(28) != 32
        || u32_at(30) != 0
    {
        return Err("XMAP22 dump is not the canonical 32-bit bottom-up BI_RGB BMP".into());
    }
    let width = u32::try_from(width_i32)?;
    let height = u32::try_from(height_i32)?;
    let row_bytes = usize::try_from(width)?
        .checked_mul(4)
        .ok_or("XMAP22 BMP row size overflow")?;
    let image_bytes = row_bytes
        .checked_mul(usize::try_from(height)?)
        .ok_or("XMAP22 BMP image size overflow")?;
    let canonical_header = canonical_bmp_header(
        width,
        height,
        u32::try_from(image_bytes).map_err(|_| "XMAP22 BMP image exceeds u32")?,
    )?;
    if bytes[..54] != canonical_header {
        return Err("XMAP22 BMP header differs from the canonical producer header".into());
    }
    let end = pixel_offset
        .checked_add(image_bytes)
        .ok_or("XMAP22 BMP extent overflow")?;
    if end != bytes.len() || u32_at(2) as usize != bytes.len() || pixel_offset != 54 {
        return Err("XMAP22 BMP byte extent is noncanonical or truncated".into());
    }
    let mut pixels = Vec::with_capacity(image_bytes);
    for y in (0..usize::try_from(height)?).rev() {
        let start = pixel_offset + y * row_bytes;
        pixels.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    Ok((width, height, pixels))
}

#[derive(Debug)]
pub(crate) struct ProductionEvidenceSinkError {
    sink: &'static str,
    dump: PathBuf,
    source: Box<dyn std::error::Error>,
}

impl ProductionEvidenceSinkError {
    fn wrap(
        sink: &'static str,
        dump: &Path,
    ) -> impl FnOnce(Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
        let dump = dump.to_path_buf();
        move |source| Box::new(Self { sink, dump, source }) as Box<dyn std::error::Error>
    }

    #[cfg(test)]
    pub(crate) fn sink(&self) -> &'static str {
        self.sink
    }
}

impl std::fmt::Display for ProductionEvidenceSinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frame-dump evidence sink '{}' failed after the frame BMP {} was already published; \
             the dump fails closed with its completion evidence withheld: {}",
            self.sink,
            self.dump.display(),
            self.source
        )
    }
}

impl std::error::Error for ProductionEvidenceSinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
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
    write_snow_mud_h2_receipt_inner(out, receipt).map_err(ProductionEvidenceSinkError::wrap(
        "SnowMudData H2 receipt",
        out,
    ))
}

fn write_snow_mud_h2_receipt_inner(
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

fn snow_mud_h2_payload_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".snow-mud-h2.tight-bgra8");
    PathBuf::from(path)
}

fn write_snow_mud_h2_evidence(
    out: &Path,
    receipt: &replay_core::SnowMudReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<(PathBuf, PathBuf)> {
    if tight_readback_bytes.len() as u64 != receipt.verified_tight_byte_count
        || format!("{:x}", Sha256::digest(tight_readback_bytes)) != receipt.readback_sha256
    {
        return Err("SnowMudData H2 sidecar bytes differ from the receipt".into());
    }
    let receipt_path = snow_mud_h2_receipt_path(out);
    let payload_path = snow_mud_h2_payload_path(out);
    let mut receipt_bytes = serde_json::to_vec_pretty(receipt)?;
    receipt_bytes.push(b'\n');
    write_create_new_or_equal(
        &payload_path,
        tight_readback_bytes,
        "SnowMudData H2 payload",
    )?;
    write_create_new_or_equal(&receipt_path, &receipt_bytes, "SnowMudData H2 receipt")?;
    Ok((receipt_path, payload_path))
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
    write_occupation_secondary_h2_evidence_inner(out, receipt, tight_readback_bytes).map_err(
        ProductionEvidenceSinkError::wrap("occupation secondary H2 evidence", out),
    )
}

fn write_occupation_secondary_h2_evidence_inner(
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

pub(crate) fn write_create_new_or_equal(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
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
    let result = staged.and_then(|()| match std::fs::hard_link(&temporary, path) {
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
        Err(error) => Err(format!("atomically publish {label} {}: {error}", path.display()).into()),
    });
    let _ = std::fs::remove_file(&temporary);
    result
}


struct FrameKeying {
    owners: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    controllers: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    world_frame: Option<std::sync::Arc<replay_core::WorldFrame>>,
}

fn load_frame_keying(provider: &dyn OwnerProvider, hour: i64) -> Result<FrameKeying> {
    Ok(FrameKeying {
        owners: provider.owners_at(hour)?,
        controllers: provider.controllers_at(hour)?,
        world_frame: provider.world_frame_at(hour)?,
    })
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
        return Err(crate::blockers::reason(crate::blockers::BlockerId::RecordingBundle).into());
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
        provider
            .content_manifest()
            .ok_or("portable playback has no admitted content manifest")?
            .effective_vfs_manifest_sha256
            .as_str(),
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
    let day_night_hour = day_night_hour_override;
    renderer.render(&gfx, &camera, day_night_hour, None)?;
    if let Some(hdr_path) = dump_hdr {
        renderer.dump_hdr_mainscene(&gfx, hdr_path)?;
    }
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
            "captured pass submitted-stream omissions: particles {}, entities {}, weather {}",
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
    let provider = match mode {
        TimelineMode::Recording {
            path,
            logical_stream_id,
            timeline,
        } => {
            let source =
                crate::recording_open::open_product_recording_segment(path, *logical_stream_id)
                    .map_err(|error| format!("open recording {}: {error}", path.display()))?;
            let (provider, _label) = crate::timeline::build_recording_provider(
                None,
                source,
                *timeline,
                &path.display().to_string(),
            )?;
            provider
        }
        TimelineMode::Static1936 => return Err(
            "bundle-less --dump-frame is a production recording command and requires --recording"
                .into(),
        ),
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
    let hours = provider.hours();
    let selected_hour = match (hour, nth_hour) {
        (Some(hour), None) => resolve_dump_hour(hours, DumpHourRequest::Canonical(hour))?,
        (None, Some(ordinal)) => resolve_dump_hour(hours, DumpHourRequest::Ordinal(ordinal))?,
        (None, None) => *hours.first().ok_or("timeline provider has no hours")?,
        (Some(_), Some(_)) => {
            return Err("competing canonical and ordinal dump-hour selectors".into())
        }
    };
    let frame = provider
        .world_frame_at(selected_hour)?
        .ok_or_else(|| map_viewer::refuse_missing_xmap00_authority("world_frame"))?;
    let (map_width, map_height) =
        map_viewer::foundation_map_dimensions(&game_resolver, &content_admission)?;
    let camera = map_viewer::ViewerMapCameraState::from_admitted_content(
        &game_resolver,
        &content_admission,
        map_width,
        map_height,
        DUMP_WIDTH,
        DUMP_HEIGHT,
        map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
    )?;
    let viewer = map_viewer::ViewerOwnedMapState {
        viewport_width: DUMP_WIDTH,
        viewport_height: DUMP_HEIGHT,
        input_revision: camera.input_revision,
        camera: Some(camera),
        observer_country_id: None,
        map_mode_id: map_viewer::ViewerMapModeSelection::default().viewer_token(),
        map_mode_selection_epoch: 0,
        selected_object_ids: Vec::new(),
        animation_time_f32_bits: 0.0f32.to_bits(),
        frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
    };
    let (terrain_options, render_settings_pin) =
        crate::install_paths::playback_render_settings(
            product_paths,
            &content_manifest.required_settings,
        )?;
    crate::app::announce_game_file_ledger(
        &content_admission,
        provider.product_admission(),
        &render_settings_pin,
        &content_manifest.localisation_language,
    );
    let mut composition = crate::app::ProductMapCompositionSession::start_for_headless(
        &game_resolver,
        &content_admission,
        content_manifest,
        terrain_options,
        DUMP_WIDTH,
        DUMP_HEIGHT,
        provider.foundation_content(),
    )?;
    let ui_profile = crate::install_paths::playback_ui_profile_from_paths(
        product_paths,
        &game_resolver,
        crate::win32::actual_client_input_variant(),
    )?;
    let xmap15_selection = map_viewer::xmap15_viewer_selection(&frame, None);
    let receipt = composition.execute(
        &game_resolver,
        &content_admission,
        content_manifest,
        &frame,
        &viewer,
        &xmap15_selection,
        ui_profile.localisation_language(),
        0.0,
        false,
        false,
        ui_profile.key().gui_scale(),
    )?;
    let pixels = composition.read_target()?;
    if snapshot::sha256::Sha256::hex(&pixels) != receipt.final_pixels_sha256 {
        return Err("XMAP22 headless readback changed after the sealed composition".into());
    }
    write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;
    let receipt_path = xmap22_receipt_path(out);
    let mut receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
    receipt_bytes.push(b'\n');
    write_create_new_or_equal(&receipt_path, &receipt_bytes, "XMAP22 composition receipt")
}

fn reject_production_dump_debug_inputs(
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    day_night_hour_override: Option<f32>,
) -> Result<()> {
    use crate::blockers::{reason, BlockerId};
    if dump_zoom.is_some() {
        return Err(reason(BlockerId::RecordingDumpZoom).into());
    }
    if dump_center.is_some() {
        return Err(reason(BlockerId::RecordingDumpCenter).into());
    }
    if dump_hdr.is_some() {
        return Err(reason(BlockerId::RecordingDumpHdr).into());
    }
    if capture_dir.is_some() {
        return Err(reason(BlockerId::RecordingCapture).into());
    }
    if day_night_hour_override.is_some() {
        return Err(reason(BlockerId::RecordingDayNightHour).into());
    }
    Ok(())
}

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
    let header = canonical_bmp_header(width, height, image_bytes)?;
    let file_bytes = 54 + image_bytes;
    let mut out = Vec::with_capacity(file_bytes as usize);
    out.extend_from_slice(&header);
    for y in (0..height as usize).rev() {
        out.extend_from_slice(&pixels[y * row_bytes..(y + 1) * row_bytes]);
    }
    write_create_new_or_equal(path, &out, "headless BGRA BMP")
}

fn canonical_bmp_header(width: u32, height: u32, image_bytes: u32) -> Result<[u8; 54]> {
    let file_bytes = 54u32
        .checked_add(image_bytes)
        .ok_or("XMAP22 BMP file size overflow")?;
    let width = i32::try_from(width).map_err(|_| "XMAP22 BMP width exceeds i32")?;
    let height = i32::try_from(height).map_err(|_| "XMAP22 BMP height exceeds i32")?;
    let mut out = Vec::with_capacity(54);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_bytes.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&image_bytes.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.try_into()
        .map_err(|_| "XMAP22 canonical BMP header length is not 54".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapshot::{pack, PackEntry, PackIdentity};

    #[test]
    fn xmap22_bmp_decoder_round_trips_top_down_bgra_and_rejects_trailing_bytes() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-xmap22-bmp-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("frame.bmp");
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        write_bmp(&path, 2, 2, &pixels).unwrap();
        let (width, height, decoded) =
            portable_test_fs_retry::with_read_guard(&path, || read_xmap22_bmp(&path))
                .expect("hold the freshly written BMP open across the scanner's window")
                .unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(decoded, pixels);

        let mut corrupt = portable_test_fs_retry::read(&path).unwrap();
        corrupt.push(0);
        let corrupt_path = root.join("corrupt.bmp");
        std::fs::write(&corrupt_path, corrupt).unwrap();
        assert!(
            portable_test_fs_retry::with_read_guard(&corrupt_path, || read_xmap22_bmp(
                &corrupt_path
            ))
            .expect("hold the corrupt fixture open across the scanner's window")
            .is_err()
        );

        for offset in [6usize, 34, 38, 42, 46, 50] {
            let mut corrupt = portable_test_fs_retry::read(&path).unwrap();
            corrupt[offset] ^= 1;
            let corrupt_path = root.join(format!("corrupt-header-{offset}.bmp"));
            std::fs::write(&corrupt_path, corrupt).unwrap();
            assert!(
                portable_test_fs_retry::with_read_guard(&corrupt_path, || read_xmap22_bmp(
                    &corrupt_path
                ))
                .expect("hold the corrupt fixture open across the scanner's window")
                .is_err(),
                "offset {offset}"
            );
        }
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn immutable_authority_publication_is_atomic_idempotent_and_no_replace() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-receipt-publish-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("receipt.json");
        write_create_new_or_equal(&path, b"complete receipt bytes\n", "test receipt").unwrap();
        write_create_new_or_equal(&path, b"complete receipt bytes\n", "test receipt").unwrap();
        assert!(write_create_new_or_equal(&path, b"different\n", "test receipt").is_err());
        assert_eq!(portable_test_fs_retry::read(&path).unwrap(), b"complete receipt bytes\n");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn post_publication_h2_evidence_sink_failure_is_typed() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-evidence-sink-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let out = root.join("frame.bmp");
        std::fs::write(&out, b"published bmp bytes").unwrap();
        std::fs::write(snow_mud_h2_receipt_path(&out), b"different\n").unwrap();
        let snow_receipt = replay_core::SnowMudReplayReadbackReceipt {
            schema: "test".to_string(),
            process_start_nonce: 1,
            hook_epoch_nonce: 1,
            producer_ordinal: 1,
            producer_game_hour_raw: 0,
            display_game_hour_raw: 0,
            selected_candidate_id: 1,
            selected_qpc_ticks: 1,
            selected_present_sha256: String::new(),
            snow_mud_fade_f32_bits: 0,
            snow_mud_fade_consumer_count: 0,
            snow_mud_fade_consumer_set_sha256: String::new(),
            observation_sha256: String::new(),
            payload_sha256: String::new(),
            readback_sha256: String::new(),
            width: 1,
            height: 1,
            source_row_pitch: 4,
            mapped_row_pitch: 4,
            verified_row_count: 1,
            verified_tight_byte_count: 4,
            replaced_binding_count: 1,
            receipt_sha256: String::new(),
        };
        let error = write_snow_mud_h2_receipt(&out, &snow_receipt)
            .expect_err("a colliding snow/mud receipt with different bytes must refuse");
        let typed = error
            .downcast_ref::<ProductionEvidenceSinkError>()
            .expect("the snow/mud sink failure must be the typed classification");
        assert_eq!(typed.sink(), "SnowMudData H2 receipt");
        assert!(typed.to_string().contains("already published"), "{typed}");
        assert!(
            std::error::Error::source(typed).is_some(),
            "the underlying collision error must be preserved as the source"
        );
        assert_eq!(portable_test_fs_retry::read(&out).unwrap(), b"published bmp bytes");

        let payload = b"bgra";
        std::fs::write(occupation_secondary_h2_payload_path(&out), b"other\n").unwrap();
        let occupation_receipt = replay_core::OccupationSecondaryReplayReadbackReceipt {
            schema: "test".to_string(),
            target_binding_identity: String::new(),
            upload_order: 1,
            selected_present_sha256: String::new(),
            payload_sha256: String::new(),
            readback_sha256: format!("{:x}", Sha256::digest(payload)),
            width: 1,
            height: 1,
            source_row_pitch: 4,
            mapped_row_pitch: 4,
            verified_row_count: 1,
            verified_tight_byte_count: payload.len() as u64,
            terrain_t6_binding_count: 1,
            water_t3_binding_count: 0,
            receipt_sha256: String::new(),
        };
        let error = write_occupation_secondary_h2_evidence(&out, &occupation_receipt, payload)
            .expect_err("a colliding occupation payload with different bytes must refuse");
        let typed = error
            .downcast_ref::<ProductionEvidenceSinkError>()
            .expect("the occupation sink failure must be the typed classification");
        assert_eq!(typed.sink(), "occupation secondary H2 evidence");
        assert!(typed.to_string().contains("already published"), "{typed}");

        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snow_mud_h2_evidence_publishes_raw_plane_with_receipt_and_refuses_mismatched_bytes() {
        let bytes: Vec<u8> = (0..64u8).collect();
        let receipt = replay_core::SnowMudReplayReadbackReceipt {
            schema: replay_core::SNOW_MUD_H2_READBACK_SCHEMA.into(),
            process_start_nonce: 1,
            hook_epoch_nonce: 2,
            producer_ordinal: 3,
            producer_game_hour_raw: 4,
            display_game_hour_raw: 4,
            selected_candidate_id: 5,
            selected_qpc_ticks: 6,
            selected_present_sha256: "11".repeat(32),
            snow_mud_fade_f32_bits: 0,
            snow_mud_fade_consumer_count: 1,
            snow_mud_fade_consumer_set_sha256: "22".repeat(32),
            observation_sha256: "33".repeat(32),
            payload_sha256: "44".repeat(32),
            readback_sha256: format!("{:x}", Sha256::digest(&bytes)),
            width: 4,
            height: 4,
            source_row_pitch: 16,
            mapped_row_pitch: 16,
            verified_row_count: 4,
            verified_tight_byte_count: bytes.len() as u64,
            replaced_binding_count: 1,
            receipt_sha256: String::new(),
        };
        let root = std::env::temp_dir().join(format!(
            "battle-plan-snow-mud-h2-evidence-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let out = root.join("hour-00000000.bmp");

        let mut wrong = bytes.clone();
        wrong[0] ^= 0xff;
        assert!(write_snow_mud_h2_evidence(&out, &receipt, &wrong).is_err());
        assert!(!snow_mud_h2_receipt_path(&out).exists());
        assert!(!snow_mud_h2_payload_path(&out).exists());

        let (receipt_path, payload_path) =
            write_snow_mud_h2_evidence(&out, &receipt, &bytes).unwrap();
        assert_eq!(portable_test_fs_retry::read(&payload_path).unwrap(), bytes);
        let written: serde_json::Value =
            serde_json::from_slice(&portable_test_fs_retry::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(
            written["readback_sha256"],
            serde_json::Value::from(receipt.readback_sha256.as_str())
        );
        write_snow_mud_h2_evidence(&out, &receipt, &bytes).unwrap();

        let mut altered = receipt.clone();
        altered.replaced_binding_count += 1;
        assert!(write_snow_mud_h2_evidence(&out, &altered, &bytes).is_err());
        portable_test_fs_retry::remove_dir_all(root).unwrap();
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
                // A null channel is NOT the "no collector" shape. The fold
                // always writes this channel as an object carrying a
                // `domain_problems` array (`world_fold.rs:9290-9294, :9744`),
                // so `rederive_collector_domain_problems` reads a present
                // non-object as a MALFORMED envelope and fails closed --
                // every problem-backed typed view is downgraded, owners
                // included. A fixture that then asserts 1081 measured owners
                // is asserting against a recording shape no capture can
                // produce. Say what this fixture means instead: the collector
                // captured nothing here, and it declares no problem.
                "collector": {
                    "captured": false,
                    "domains": [],
                    "domain_problems": [],
                },
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
            "naval_combats": [], "models": {}, "intel_ledger": {},
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
        let source = portable_test_fs_retry::with_read_guard(&path, || {
            replay_core::open_recording(&path)
        })
        .expect("hold the recording open across the scanner's window")
        .expect("open packed keying recording");
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
        drop(provider);
        portable_test_fs_retry::remove_dir_all(path.parent().expect("recording parent"))
            .expect("remove keying recording");
    }

    #[test]
    fn production_recording_dump_rejects_every_debug_substitute() {
        use crate::blockers::{reason, BlockerId};
        assert!(reject_production_dump_debug_inputs(None, None, None, None, None).is_ok());
        let err = reject_production_dump_debug_inputs(Some(1.0), None, None, None, None)
            .expect_err("zoom");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDumpZoom));
        let err = reject_production_dump_debug_inputs(None, Some((1.0, 2.0)), None, None, None)
            .expect_err("center");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDumpCenter));
        let err = reject_production_dump_debug_inputs(
            None,
            None,
            Some(Path::new("scene.hdr")),
            None,
            None,
        )
        .expect_err("hdr");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDumpHdr));
        let err =
            reject_production_dump_debug_inputs(None, None, None, Some(Path::new("capture")), None)
                .expect_err("capture");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingCapture));
        let err = reject_production_dump_debug_inputs(None, None, None, None, Some(0.5))
            .expect_err("day/night");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDayNightHour));
    }

    #[test]
    fn production_recording_dump_rejects_debug_bundle_route() {
        let mode = TimelineMode::Recording {
            path: "portable.battleplan".into(),
            logical_stream_id: None,
            timeline: replay_core::TimelineConfig::default(),
        };
        assert!(reject_recording_dump_debug_bundle(&mode, false).is_ok());
        let err = reject_recording_dump_debug_bundle(&mode, true).expect_err("bundle");
        assert_eq!(
            err.to_string(),
            crate::blockers::reason(crate::blockers::BlockerId::RecordingBundle),
            "bundle rejection is the registry row's reason verbatim"
        );
        assert!(reject_recording_dump_debug_bundle(&TimelineMode::Static1936, true).is_ok());
    }

    #[test]
    fn headless_product_dump_executes_the_shared_xmap22_composition() {
        let (topology, schedule) =
            crate::validate::product_foundation_assigned_stage_plan().unwrap();
        assert_eq!(topology.ordered_stages().len(), 22);
        assert_eq!(topology.ordered_edges().len(), 18);
        schedule
            .require(
                map_viewer::XMAP06_EARLY_STAGE_NAME,
                map_viewer::XMAP06_EARLY_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP06");
        schedule
            .require(
                map_viewer::XMAP08_RIVER_STAGE_NAME,
                map_viewer::XMAP08_RIVER_OPERATION_ID,
            )
            .expect("headless product foundation includes XMAP08 rivers");
        schedule
            .require(
                map_viewer::XMAP08_STRAIT_STAGE_NAME,
                map_viewer::XMAP08_STRAIT_OPERATION_ID,
            )
            .expect("headless product foundation includes XMAP08 straits");
        schedule
            .require(
                map_viewer::XMAP11_TREE_STAGE_NAME,
                map_viewer::XMAP11_TREE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP11 trees");
        schedule
            .require(
                map_viewer::XMAP17_EXECUTE_STAGE,
                map_viewer::XMAP17_EXECUTE_OPERATION,
            )
            .expect("headless product foundation includes XMAP17 labels");
        schedule
            .require(
                map_viewer::XMAP09_PREPARE_STAGE_NAME,
                map_viewer::XMAP09_PREPARE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP09");
        schedule
            .require(
                map_viewer::XMAP19_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP19_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP19");
        schedule
            .require(
                map_viewer::XMAP07_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP07_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP07");
        schedule
            .require(
                map_viewer::XMAP13_UNIT_ENTITY_STAGE_NAME,
                map_viewer::XMAP13_UNIT_ENTITY_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP13 entities");
        schedule
            .require(
                map_viewer::XMAP13_COUNTER_QUAD_STAGE_NAME,
                map_viewer::XMAP13_COUNTER_QUAD_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP13 counters");
        schedule
            .require(
                map_viewer::XMAP14_COMBAT_STAGE_NAME,
                map_viewer::XMAP14_COMBAT_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP14 combat indicators");
        schedule
            .require(
                map_viewer::XMAP15_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP15_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP15 orders");
        schedule
            .require(
                map_viewer::XMAP20_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP20_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP20");
        schedule
            .require(
                map_viewer::XMAP16_RAIL_STAGE_NAME,
                map_viewer::XMAP16_RAIL_OPERATION_ID,
            )
            .expect("headless product foundation includes XMAP16");
        schedule
            .require(
                map_viewer::XMAP21_PREPARE_STAGE_NAME,
                map_viewer::XMAP21_PREPARE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP21 prepare");
        schedule
            .require(
                map_viewer::XMAP21_EXECUTE_STAGE_NAME,
                map_viewer::XMAP21_EXECUTE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP21 execute");

        let source = include_str!("headless.rs").replace("\r\n", "\n");
        let start = source
            .find("fn dump_production_recording_frame(")
            .expect("production recording dump function");
        let end = source[start..]
            .find("fn reject_production_dump_debug_inputs(")
            .expect("next headless function boundary");
        let production = &source[start..start + end];
        let composition = production
            .find("ProductMapCompositionSession::start_for_headless(")
            .expect("shared XMAP22 product compositor");
        let ui_profile = production
            .find("playback_ui_profile_from_paths(")
            .expect("presentation profile load");
        let execute = production
            .find("composition.execute(")
            .expect("shared XMAP22 execution");
        assert!(production[..composition].contains("foundation_map_dimensions("));
        assert!(
            composition < ui_profile,
            "foundation admission must precede UI profile loading"
        );
        assert!(
            ui_profile < execute,
            "UI profile must be available before composition execution"
        );
        assert!(production[composition..].contains("write_bmp("));
        assert!(production[composition..].contains("xmap22_receipt_path(out)"));
        assert_eq!(
            xmap22_receipt_path(Path::new("frame.bmp")),
            PathBuf::from("frame.bmp.xmap22.json")
        );
        for forbidden in [
            "production_render",
            "selected_present",
            "ProductObserverUiRuntime",
        ] {
            assert!(
                !production[composition..].contains(forbidden),
                "headless product composition consumed {forbidden}"
            );
        }
    }
}
