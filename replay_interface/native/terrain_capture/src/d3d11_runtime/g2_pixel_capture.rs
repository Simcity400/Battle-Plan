//! Authenticated G2 engine pixel witness at the map/UI boundary.
//!
//! This module deliberately captures inside the hooked render-thread draw path,
//! after both shaders have been authenticated against the current swapchain's
//! creation cache and before the original first UI draw executes. A later
//! packager may join the immutable witness to a finalized portable recording;
//! it may not manufacture or replace this raw observation.

use super::*;
use snapshot::sha256::{to_hex, Sha256};

static PRESENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LAST_ATTEMPTED_PRESENT_SEQUENCE: AtomicU64 = AtomicU64::new(u64::MAX);
static PIXEL_CAPTURED: AtomicBool = AtomicBool::new(false);
static PIXEL_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub(super) fn present_completed() {
    PRESENT_SEQUENCE.fetch_add(1, Ordering::SeqCst);
}

fn canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

fn validate_control(control: &G2PixelCaptureControl) -> Result<(), String> {
    if control.schema != "hoi4-g2-engine-pixel-capture-control-v1" {
        return Err("G2 pixel capture control schema drift".to_string());
    }
    if control.observation_id.trim().is_empty()
        || control.localisation_language.trim().is_empty()
        || control.pov_country_or_global.trim().is_empty()
        || control.expected_selected_hour < 0
    {
        return Err("G2 pixel capture control has an empty identity field".to_string());
    }
    for (name, value) in [
        ("authority_exe_sha256", control.authority_exe_sha256.as_str()),
        ("launch_epoch_sha256", control.launch_epoch_sha256.as_str()),
        ("capture_receipt_sha256", control.capture_receipt_sha256.as_str()),
        (
            "effective_vfs_manifest_sha256",
            control.effective_vfs_manifest_sha256.as_str(),
        ),
        (
            "content_manifest_digest_sha256",
            control.content_manifest_digest_sha256.as_str(),
        ),
        ("settings_file_sha256", control.settings_file_sha256.as_str()),
        ("camera_payload_sha256", control.camera_payload_sha256.as_str()),
    ] {
        if !canonical_sha256(value) {
            return Err(format!("G2 pixel capture control {name} is not canonical SHA-256"));
        }
    }
    if control.authority_exe_sha256
        != "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
    {
        return Err("G2 pixel capture authority executable is not exact a729".to_string());
    }
    let context_bytes = fs::read(&control.capture_context_path).map_err(|error| {
        format!(
            "G2 capture context {} is unreadable: {error}",
            control.capture_context_path.display()
        )
    })?;
    let context: serde_json::Value = serde_json::from_slice(&context_bytes)
        .map_err(|error| format!("G2 capture context JSON is invalid: {error}"))?;
    if context.get("schema").and_then(serde_json::Value::as_str)
        != Some("hoi4-rendering-law-capture-context-v1")
        || context.get("packets") != Some(&serde_json::json!(["G2"]))
    {
        return Err("G2 capture context is not an isolated G2 snapshot".to_string());
    }
    if context.pointer("/inventory/active_hoi4_pids") != Some(&serde_json::json!([])) {
        return Err("G2 capture context was not frozen before the authority launch".to_string());
    }
    Ok(())
}

fn tight_bgra_from_mapped(
    desc: &D3D11_TEXTURE2D_DESC,
    mapped: &MappedTextureSubresourceBytes,
) -> Result<Vec<u8>, String> {
    let tight_row = usize::try_from(desc.Width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| "G2 backbuffer row byte count overflowed".to_string())?;
    let row_pitch = mapped.row_pitch as usize;
    let height = desc.Height as usize;
    if row_pitch < tight_row || mapped.bytes.len() < row_pitch.saturating_mul(height) {
        return Err("G2 backbuffer mapped pitch/length is inconsistent".to_string());
    }
    let rgba_source = matches!(
        desc.Format,
        DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
    );
    if !rgba_source
        && !matches!(
            desc.Format,
            DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
        )
    {
        return Err(format!(
            "G2 backbuffer format {} is not an exact four-channel 8-bit format",
            desc.Format
        ));
    }
    let mut tight = Vec::with_capacity(tight_row.saturating_mul(height));
    for row in 0..height {
        let start = row * row_pitch;
        let source = &mapped.bytes[start..start + tight_row];
        if rgba_source {
            for pixel in source.chunks_exact(4) {
                tight.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        } else {
            tight.extend_from_slice(source);
        }
    }
    Ok(tight)
}

fn top_down_bgra_bmp(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(height as usize))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "G2 BMP dimensions overflowed".to_string())?;
    if pixels.len() != expected || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err("G2 BMP dimensions do not match the tight BGRA plane".to_string());
    }
    let file_size = 54usize
        .checked_add(pixels.len())
        .and_then(|size| u32::try_from(size).ok())
        .ok_or_else(|| "G2 BMP file size overflowed".to_string())?;
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

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("G2 refuses to overwrite {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("G2 write {} failed: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("G2 sync {} failed: {error}", path.display()))
}

unsafe fn backbuffer_bgra(
    context: *mut ID3D11DeviceContext,
    expected: &UiPassBackbufferFile,
) -> Result<(D3D11_TEXTURE2D_DESC, Vec<u8>), String> {
    let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
    let mut dsv: *mut ID3D11DepthStencilView = ptr::null_mut();
    unsafe { (*context).OMGetRenderTargets(1, &mut rtv, &mut dsv) };
    if !dsv.is_null() {
        unsafe { release_unknown(dsv as *mut WinVoid) };
    }
    if rtv.is_null() {
        return Err("G2 first UI draw has no render target".to_string());
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe { (*rtv).GetResource(&mut resource) };
    unsafe { release_unknown(rtv as *mut WinVoid) };
    if resource.is_null() || format!("0x{:X}", resource as usize) != expected.resource_pointer {
        if !resource.is_null() {
            unsafe { release_unknown(resource as *mut WinVoid) };
        }
        return Err("G2 active render target is not the observed swapchain backbuffer".to_string());
    }
    let texture = match unsafe { texture2d_from_resource(resource) } {
        Ok(texture) => texture,
        Err(error) => {
            unsafe { release_unknown(resource as *mut WinVoid) };
            return Err(error);
        }
    };
    let mut desc: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
    unsafe { (*texture).GetDesc(&mut desc) };
    let result = (|| -> Result<Vec<u8>, String> {
        if desc.Width != expected.width || desc.Height != expected.height {
            return Err("G2 backbuffer identity dimensions drifted before UI draw".to_string());
        }
        if desc.MipLevels != 1 || desc.ArraySize != 1 {
            return Err("G2 backbuffer is not a single mip/slice Texture2D".to_string());
        }
        let mut device: *mut ID3D11Device = ptr::null_mut();
        unsafe { (*texture).GetDevice(&mut device) };
        if device.is_null() {
            return Err("G2 backbuffer has no D3D11 device".to_string());
        }
        let inner = (|| -> Result<Vec<u8>, String> {
            let mut readback_desc = desc;
            let mut resolved: *mut ID3D11Texture2D = ptr::null_mut();
            let readback_texture = if desc.SampleDesc.Count > 1 {
                readback_desc.SampleDesc.Count = 1;
                readback_desc.SampleDesc.Quality = 0;
                readback_desc.BindFlags = 0;
                readback_desc.CPUAccessFlags = 0;
                readback_desc.MiscFlags = 0;
                resolved = unsafe { create_texture2d_resolve_target(device, &readback_desc, 0)? };
                unsafe {
                    (*context).ResolveSubresource(
                        resolved as *mut ID3D11Resource,
                        0,
                        texture as *mut ID3D11Resource,
                        0,
                        desc.Format,
                    )
                };
                resolved
            } else {
                texture
            };
            let staging = match unsafe { create_texture2d_exact_staging(device, &readback_desc, 0) }
            {
                Ok(staging) => staging,
                Err(error) => {
                    if !resolved.is_null() {
                        unsafe { release_unknown(resolved as *mut WinVoid) };
                    }
                    return Err(error);
                }
            };
            unsafe {
                (*context).CopyResource(
                    staging as *mut ID3D11Resource,
                    readback_texture as *mut ID3D11Resource,
                )
            };
            let mapped = unsafe {
                readback_texture2d_subresource_exact(
                    context,
                    staging,
                    0,
                    0,
                    0,
                    readback_desc.Width,
                    readback_desc.Height,
                    0,
                )
            };
            unsafe {
                release_unknown(staging as *mut WinVoid);
                if !resolved.is_null() {
                    release_unknown(resolved as *mut WinVoid);
                }
            }
            tight_bgra_from_mapped(&readback_desc, &mapped?)
        })();
        unsafe { release_unknown(device as *mut WinVoid) };
        inner
    })();
    unsafe {
        release_unknown(texture as *mut WinVoid);
        release_unknown(resource as *mut WinVoid);
    }
    result.map(|pixels| (desc, pixels))
}

pub(super) unsafe fn capture_before_first_ui_draw(
    context: *mut ID3D11DeviceContext,
    backbuffer: &UiPassBackbufferFile,
    draw_call: &'static str,
    draw_count: UINT,
    pixel_shader: usize,
    vertex_shader: usize,
) {
    if PIXEL_CAPTURED.load(Ordering::Acquire) {
        return;
    }
    let Some((output_dir, control)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|runtime| {
            runtime
                .control
                .as_ref()
                .and_then(|control| control.g2_pixel_capture.clone())
                .map(|control| (runtime.output_dir.clone(), control))
        })
    }) else {
        return;
    };
    let selected_present_sequence = PRESENT_SEQUENCE.load(Ordering::Acquire).saturating_add(1);
    if LAST_ATTEMPTED_PRESENT_SEQUENCE.swap(selected_present_sequence, Ordering::AcqRel)
        == selected_present_sequence
        || PIXEL_CAPTURE_IN_PROGRESS.swap(true, Ordering::AcqRel)
    {
        return;
    }
    let observation_id = control.observation_id.clone();
    let result = (|| -> Result<(), String> {
        validate_control(&control)?;
        let time = exact_game_time_from_engine()
            .ok_or_else(|| "G2 exact engine hour is unavailable".to_string())?;
        if time.hour_index != control.expected_selected_hour {
            return Err(format!(
                "G2 expected selected hour {}, observed {}",
                control.expected_selected_hour, time.hour_index
            ));
        }
        let capture_context_sha256 = sha256_bytes(
            &fs::read(&control.capture_context_path).map_err(|error| {
                format!("G2 capture context became unreadable: {error}")
            })?,
        );
        let (desc, pixels) = unsafe { backbuffer_bgra(context, backbuffer)? };
        let bmp = top_down_bgra_bmp(desc.Width, desc.Height, &pixels)?;
        fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
        write_new_file(&output_dir.join(G2_ENGINE_BMP_FILE), &bmp)?;
        let witness = serde_json::json!({
            "schema": G2_ENGINE_WITNESS_SCHEMA,
            "status": "AUTHENTICATED_PRE_FIRST_UI_DRAW_MAP_READBACK",
            "observation_id": observation_id.clone(),
            "render_boundary": G2_RENDER_BOUNDARY,
            "pixel_format": "BGRA8_UNORM",
            "authority_exe_sha256": control.authority_exe_sha256,
            "launch_epoch_sha256": control.launch_epoch_sha256,
            "capture_receipt_sha256": control.capture_receipt_sha256,
            "effective_vfs_manifest_sha256": control.effective_vfs_manifest_sha256,
            "content_manifest_digest_sha256": control.content_manifest_digest_sha256,
            "settings_file_sha256": control.settings_file_sha256,
            "localisation_language": control.localisation_language,
            "camera_payload_sha256": control.camera_payload_sha256,
            "pov_country_or_global": control.pov_country_or_global,
            "selected_hour": time.hour_index,
            "selected_present_sequence": selected_present_sequence,
            "viewport_width": desc.Width,
            "viewport_height": desc.Height,
            "source_dxgi_format": desc.Format,
            "source_sample_count": desc.SampleDesc.Count,
            "engine_bmp_file": G2_ENGINE_BMP_FILE,
            "engine_bmp_sha256": sha256_bytes(&bmp),
            "engine_bgra_sha256": sha256_bytes(&pixels),
            "pixel_byte_len": pixels.len(),
            "capture_context_sha256": capture_context_sha256,
            "ui_boundary_authentication": {
                "active_backbuffer_resource": backbuffer.resource_pointer,
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
            .map_err(|error| format!("G2 witness serialization failed: {error}"))?;
        bytes.push(b'\n');
        write_new_file(&output_dir.join(G2_ENGINE_WITNESS_FILE), &bytes)
    })();
    PIXEL_CAPTURE_IN_PROGRESS.store(false, Ordering::Release);
    match result {
        Ok(()) => {
            PIXEL_CAPTURED.store(true, Ordering::Release);
            write_event(
                &output_dir,
                &format!(
                    "{{\"type\":\"g2_engine_pixel_witness_retained\",\"observation_id\":\"{}\",\"selected_present_sequence\":{},\"bmp\":\"{}\",\"witness\":\"{}\"}}",
                    json_escape(&observation_id),
                    selected_present_sequence,
                    G2_ENGINE_BMP_FILE,
                    G2_ENGINE_WITNESS_FILE,
                ),
            );
        }
        Err(error) => write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"g2_engine_pixel_witness_rejected\",\"selected_present_sequence\":{},\"reason\":\"{}\"}}",
                selected_present_sequence,
                json_escape(&error),
            ),
        ),
    }
}
