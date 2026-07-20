//! Recorder-owned G2 pre-first-UI backbuffer witness.
//!
//! The production render lane is already the sole D3D hook owner. G2 extends
//! that exact hook graph with four Draw* observers instead of injecting the
//! competing `terrain_capture` DLL. The authenticated H4CT arm is accepted
//! only after COMMIT and produces the same immutable witness consumed by the
//! shared G2 materializer.

use super::*;
use compact_contract::early_render_control::EarlyG2CaptureArmV1;
use std::fmt::Write as FmtWrite;
use std::fs::{self, OpenOptions};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

const WITNESS_SCHEMA: &str = "hoi4-g2-engine-pixel-witness-v1";
const BOUNDARY: &str = "post_authenticated_map_bgra8_no_selected_present_writers";
const BMP_FILE: &str = "g2_engine_map_boundary.bmp";
const WITNESS_FILE: &str = "g2_engine_pixel_witness.json";
const FORMAT_RGBA8_UNORM: u32 = 28;
const FORMAT_RGBA8_UNORM_SRGB: u32 = 29;
const FORMAT_BGRA8_UNORM: u32 = 87;
const FORMAT_BGRA8_UNORM_SRGB: u32 = 91;

#[derive(Default)]
struct G2CaptureState {
    arm: Option<EarlyG2CaptureArmV1>,
    /// Hours for which the authenticated map/UI boundary was already allowed
    /// to execute before the recorder arm became visible. Retaining the exact
    /// hour closes the COMMIT-to-arm race without treating unrelated loading
    /// frames as evidence for the requested selected hour.
    missed_pre_arm_hours: BTreeSet<i64>,
}

static CAPTURE_STATE: OnceLock<Mutex<G2CaptureState>> = OnceLock::new();
static CAPTURED: AtomicBool = AtomicBool::new(false);
static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LAST_ATTEMPTED_PRESENT: AtomicU32 = AtomicU32::new(u32::MAX);

fn hex(hash: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in hash {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("G2 refuses to overwrite {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write G2 artifact {} failed: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync G2 artifact {} failed: {error}", path.display()))
}

fn top_down_bgra_bmp(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(height as usize))
        .and_then(|count| count.checked_mul(4))
        .ok_or("G2 BMP dimensions overflowed")?;
    if pixels.len() != expected || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err("G2 BMP dimensions do not match tight BGRA".into());
    }
    let file_size = u32::try_from(54usize.checked_add(pixels.len()).ok_or("G2 BMP size overflow")?)
        .map_err(|_| "G2 BMP size exceeds u32")?;
    let mut bmp = Vec::with_capacity(file_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(width as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(height as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 16]);
    bmp.extend_from_slice(pixels);
    Ok(bmp)
}

fn tight_bgra(snapshot: &RawTextureResourceSnapshot) -> Result<Vec<u8>, String> {
    if snapshot.dimension_u32 != 3
        || snapshot.depth != 1
        || snapshot.mip_levels != 1
        || snapshot.array_size != 1
        || snapshot.sample_count != 1
        || snapshot.ordered_subresources.len() != 1
        || snapshot.width == 0
        || snapshot.height == 0
    {
        return Err("G2 backbuffer is not one sample-1 Texture2D subresource".into());
    }
    let rgba = matches!(snapshot.dxgi_format_u32, FORMAT_RGBA8_UNORM | FORMAT_RGBA8_UNORM_SRGB);
    if !rgba
        && !matches!(
            snapshot.dxgi_format_u32,
            FORMAT_BGRA8_UNORM | FORMAT_BGRA8_UNORM_SRGB
        )
    {
        return Err("G2 backbuffer is not an exact four-channel 8-bit format".into());
    }
    let subresource = &snapshot.ordered_subresources[0];
    let tight_row = usize::try_from(snapshot.width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or("G2 tight row overflowed")?;
    let row_pitch = subresource.row_pitch as usize;
    let height = snapshot.height as usize;
    let bytes = &subresource.payload.captured_bytes;
    if row_pitch < tight_row || bytes.len() < row_pitch.saturating_mul(height) {
        return Err("G2 mapped row pitch/length is inconsistent".into());
    }
    let mut tight = Vec::with_capacity(tight_row.saturating_mul(height));
    for row in 0..height {
        let start = row * row_pitch;
        let source = &bytes[start..start + tight_row];
        if rgba {
            for pixel in source.chunks_exact(4) {
                tight.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        } else {
            tight.extend_from_slice(source);
        }
    }
    Ok(tight)
}

pub(super) fn arm(arm: EarlyG2CaptureArmV1) -> Result<(), String> {
    if hex(&arm.authority_exe_sha256) != A729_EXECUTABLE_SHA256 {
        return Err("G2 arm executable is not exact a729".into());
    }
    if !Path::new(&arm.capture_context_path).is_absolute()
        || !Path::new(&arm.output_directory).is_absolute()
    {
        return Err("G2 arm paths are not absolute".into());
    }
    let epoch = current_capture_launch_epoch().ok_or("G2 arm arrived before exact COMMIT")?;
    if epoch.process_id != arm.process_id
        || epoch.process_start_nonce != arm.process_start_nonce
        || epoch.hook_epoch_nonce != arm.hook_epoch_nonce
        || epoch.capture_receipt_sha256 != hex(&arm.capture_receipt_sha256)
    {
        return Err("G2 arm differs from the committed render epoch".into());
    }
    let launch_epoch = portable_render_stream::compute_render_launch_epoch_sha256(
        epoch.process_id,
        epoch.process_start_nonce,
        epoch.observer_installed_qpc,
        epoch.d3d_device_created_qpc,
        epoch.first_creation_sequence,
        epoch.hook_epoch_nonce,
        &epoch.hook_set_receipt_sha256,
        &epoch.capture_receipt_sha256,
    );
    if launch_epoch != hex(&arm.launch_epoch_sha256) {
        return Err("G2 arm launch epoch digest is not mechanically derived".into());
    }
    if hash(&fs::read(&arm.capture_context_path).map_err(|error| {
        format!("read G2 capture context at arm failed: {error}")
    })?) != hex(&arm.capture_context_sha256)
    {
        return Err("G2 capture context changed before arm".into());
    }
    if Path::new(&arm.output_directory).exists() {
        return Err("G2 output directory already exists".into());
    }
    let mut state = CAPTURE_STATE
        .get_or_init(|| Mutex::new(G2CaptureState::default()))
        .lock()
        .map_err(|_| "G2 arm state poisoned")?;
    if state.arm.is_some() || CAPTURED.load(Ordering::Acquire) {
        return Err("G2 capture was already armed or completed".into());
    }
    if state
        .missed_pre_arm_hours
        .contains(&arm.expected_selected_hour)
    {
        return Err(
            "G2 requested-hour map/UI boundary executed before the authenticated arm".into(),
        );
    }
    state.arm = Some(arm);
    Ok(())
}

fn boundary_candidate(
    device_context: usize,
) -> Option<(usize, usize, usize, u32)> {
    let epoch = EARLY_CAPTURE_EPOCH.get()?.lock().ok()?.clone();
    if epoch.immediate_context_ptr != device_context {
        return None;
    }
    let (vertex_shader, vertex_linkage) = capture_shader(device_context, 80)?;
    let (pixel_shader, pixel_linkage) = capture_shader(device_context, 78)?;
    let mut acquired = ComRefs::default();
    acquired.extend([vertex_shader, pixel_shader]);
    if vertex_shader == 0 || pixel_shader == 0 || vertex_linkage || pixel_linkage {
        return None;
    }
    let (backbuffer_view, backbuffer_resource) = {
        let state = STATE.get()?.lock().ok()?;
        let boundary = state.render_order_events.iter().rev().find_map(|event| match event {
            RawRenderOrderEvent::Boundary { kind, .. } => Some(*kind),
            _ => None,
        });
        if state.pending_render_frame.is_none()
            || boundary != Some(RenderOrderBoundary::PostProcessEnd)
            || !state.shader_creation_observations.iter().any(|row| {
                row.shader_ptr == vertex_shader && row.stage == RawShaderStage::Vertex
            })
            || !state.shader_creation_observations.iter().any(|row| {
                row.shader_ptr == pixel_shader && row.stage == RawShaderStage::Pixel
            })
        {
            return None;
        }
        let view = state.swapchain_backbuffer_view_ptr?;
        let registration = state.registered_render_targets.get(&view)?;
        if registration.kind
            != (RawRegisteredRenderTargetKind::SwapchainBackBuffer { buffer_index: 0 })
        {
            return None;
        }
        (view, registration.resource_ptr)
    };
    let (color_targets, depth_target) = capture_render_targets(device_context)?;
    acquired.extend(color_targets.iter().copied());
    acquired.push(depth_target);
    if color_targets.as_slice() != [backbuffer_view] {
        return None;
    }
    let (active_resource, _) = capture_view_desc(
        backbuffer_view,
        CreatedD3dObjectKind::RenderTargetView,
    )?;
    if active_resource != backbuffer_resource {
        return None;
    }
    Some((
        vertex_shader,
        pixel_shader,
        backbuffer_resource,
        NEXT_PRESENT_GLOBAL_ORDER.load(Ordering::Acquire).saturating_add(1),
    ))
}

pub(super) fn capture_before_draw(device_context: usize, draw_call: &'static str, draw_count: u32) {
    if draw_count == 0 || CAPTURED.load(Ordering::Acquire) || IN_PROGRESS.load(Ordering::Acquire) {
        return;
    }
    let Ok(raw_hour) = capture_present_game_hour_raw() else {
        return;
    };
    let Some(hour) = compact_contract::calendar::hour_index_from_raw(raw_hour) else {
        return;
    };
    let Some((vertex_shader, pixel_shader, resource, present_sequence)) =
        boundary_candidate(device_context)
    else {
        return;
    };
    let arm = {
        let Ok(mut state) = CAPTURE_STATE
            .get_or_init(|| Mutex::new(G2CaptureState::default()))
            .lock()
        else {
            reject();
            return;
        };
        let Some(arm) = state.arm.clone() else {
            state.missed_pre_arm_hours.insert(hour);
            return;
        };
        arm
    };
    if hour != arm.expected_selected_hour {
        return;
    }
    if LAST_ATTEMPTED_PRESENT.swap(present_sequence, Ordering::AcqRel) == present_sequence
        || IN_PROGRESS.swap(true, Ordering::AcqRel)
    {
        return;
    }
    let result = (|| -> Result<(), String> {
        let sequence = next_catalog_observation_sequence()
            .ok_or("G2 catalog sequence is unavailable")?;
        let snapshot = capture_texture_snapshot(device_context, resource, sequence)
            .ok_or("G2 exact backbuffer readback failed")?;
        let pixels = tight_bgra(&snapshot)?;
        let bmp = top_down_bgra_bmp(snapshot.width, snapshot.height, &pixels)?;
        let output = PathBuf::from(&arm.output_directory);
        fs::create_dir(&output)
            .map_err(|error| format!("create G2 output directory failed: {error}"))?;
        write_new(&output.join(BMP_FILE), &bmp)?;
        let witness = serde_json::json!({
            "schema": WITNESS_SCHEMA,
            "status": "AUTHENTICATED_PRE_FIRST_UI_DRAW_MAP_READBACK",
            "observation_id": arm.observation_id,
            "render_boundary": BOUNDARY,
            "pixel_format": "BGRA8_UNORM",
            "authority_exe_sha256": hex(&arm.authority_exe_sha256),
            "launch_epoch_sha256": hex(&arm.launch_epoch_sha256),
            "capture_receipt_sha256": hex(&arm.capture_receipt_sha256),
            "effective_vfs_manifest_sha256": hex(&arm.effective_vfs_manifest_sha256),
            "content_manifest_digest_sha256": hex(&arm.content_manifest_digest_sha256),
            "settings_file_sha256": hex(&arm.settings_file_sha256),
            "localisation_language": arm.localisation_language,
            "camera_payload_sha256": hex(&arm.camera_payload_sha256),
            "pov_country_or_global": arm.pov_country_or_global,
            "selected_hour": hour,
            "selected_present_sequence": present_sequence,
            "viewport_width": snapshot.width,
            "viewport_height": snapshot.height,
            "source_dxgi_format": snapshot.dxgi_format_u32,
            "source_sample_count": snapshot.sample_count,
            "engine_bmp_file": BMP_FILE,
            "engine_bmp_sha256": hash(&bmp),
            "engine_bgra_sha256": hash(&pixels),
            "pixel_byte_len": pixels.len(),
            "capture_context_sha256": hex(&arm.capture_context_sha256),
            "ui_boundary_authentication": {
                "active_backbuffer_resource": format!("0x{resource:X}"),
                "pixel_shader_pointer": format!("0x{pixel_shader:X}"),
                "vertex_shader_pointer": format!("0x{vertex_shader:X}"),
                "both_shaders_from_current_swapchain_creation_cache": true,
                "first_ui_draw_call": draw_call,
                "first_ui_draw_count": draw_count,
                "first_ui_draw_executed": false
            },
            "unreceipted_pixel_writers": 0
        });
        let mut bytes = serde_json::to_vec_pretty(&witness)
            .map_err(|error| format!("serialize G2 witness failed: {error}"))?;
        bytes.push(b'\n');
        write_new(&output.join(WITNESS_FILE), &bytes)
    })();
    IN_PROGRESS.store(false, Ordering::Release);
    if result.is_ok() {
        CAPTURED.store(true, Ordering::Release);
    } else {
        reject();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_is_top_down_bgra32() {
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8];
        let bmp = top_down_bgra_bmp(2, 1, &pixels).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -1);
        assert_eq!(&bmp[54..], pixels);
    }

    #[test]
    fn tight_bgra_strips_pitch_and_swizzles_rgba() {
        let snapshot = RawTextureResourceSnapshot {
            snapshot_sequence: 1,
            resource_ptr: 2,
            dimension_u32: 3,
            width: 1,
            height: 2,
            depth: 1,
            mip_levels: 1,
            array_size: 1,
            dxgi_format_u32: FORMAT_RGBA8_UNORM,
            sample_count: 1,
            sample_quality: 0,
            usage_u32: 0,
            bind_flags_u32: 0,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            content: None,
            ordered_subresources: vec![RawTextureSubresourceSnapshot {
                subresource_index: 0,
                mip_level: 0,
                array_slice: 0,
                row_pitch: 8,
                slice_pitch: 16,
                payload: RawPayloadSnapshot {
                    captured_sha256: "a".repeat(64),
                    captured_bytes: vec![1, 2, 3, 4, 9, 9, 9, 9, 5, 6, 7, 8, 9, 9, 9, 9],
                    source: RawPayloadSource::RuntimeGenerated { reason: "fixture".into() },
                },
            }],
        };
        assert_eq!(tight_bgra(&snapshot).unwrap(), [3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
