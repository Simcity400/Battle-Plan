#[cfg(not(windows))]
fn main() {
    eprintln!("capture_harness is Windows-only because it creates a real D3D11 device");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_app::run() {
        eprintln!("capture_harness failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_app {
    use serde::Deserialize;
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::env;
    use std::ffi::CString;
    use std::fs;
    use std::mem;
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::slice;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};
    use winapi::ctypes::c_void;
    use winapi::shared::dxgi::{IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD};
    use winapi::shared::dxgiformat::{
        DXGI_FORMAT, DXGI_FORMAT_D24_UNORM_S8_UINT, DXGI_FORMAT_R16_UINT,
        DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32_FLOAT,
        DXGI_FORMAT_R8G8B8A8_UNORM,
    };
    use winapi::shared::dxgitype::{
        DXGI_MODE_DESC, DXGI_RATIONAL, DXGI_SAMPLE_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    };
    use winapi::shared::minwindef::{FALSE, TRUE, UINT};
    use winapi::shared::winerror::{FAILED, SUCCEEDED};
    use winapi::um::d3d11::{
        D3D11CreateDeviceAndSwapChain, ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilState,
        ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
        ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11Resource,
        ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
        IID_ID3D11Texture2D, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_DEPTH_STENCIL,
        D3D11_BIND_INDEX_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
        D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_BUFFER_DESC,
        D3D11_CLEAR_DEPTH, D3D11_CLEAR_STENCIL, D3D11_COLOR_WRITE_ENABLE_ALL,
        D3D11_COMPARISON_ALWAYS, D3D11_COMPARISON_NEVER, D3D11_CREATE_DEVICE_DEBUG,
        D3D11_CULL_NONE, D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ZERO, D3D11_FILL_SOLID,
        D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA,
        D3D11_RASTERIZER_DESC, D3D11_RECT, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SAMPLER_DESC,
        D3D11_SDK_VERSION, D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
    };
    use winapi::um::d3dcommon::{
        ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
        D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
    };
    use winapi::um::d3dcompiler::{
        D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL0,
    };
    use winapi::um::unknwnbase::IUnknown;
    use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, PeekMessageW, RegisterClassW,
        ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, MSG, PM_REMOVE,
        SW_SHOW, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    const WINDOW_WIDTH: u32 = 320;
    const WINDOW_HEIGHT: u32 = 180;
    const DEFAULT_LOAD_BURST_MS: u64 = 3_000;
    const DEFAULT_LOAD_BURST_SHADERS_PER_FRAME: u32 = 12;
    const STRESS_LOAD_BURST_SHADERS_PER_FRAME: u32 = 128;
    const DEFAULT_TERRAIN_FRAMES: u32 = 3;
    const DEFAULT_WATER_FRAMES: u32 = 1;
    const DEFAULT_MULTI_THREAD_MS: u64 = 3_000;
    const DEFAULT_RIVER_FRAMES: u32 = 120;
    const DEFAULT_BORDER_FRAMES: u32 = 16;
    const DEFAULT_TREE_FRAMES: u32 = 16;
    const DEFAULT_UI_FRAMES: u32 = 16;
    const DEFAULT_UI_DRAWS_PER_FRAME: u32 = 36;
    const UI_ATLAS_MIN_FRAMES: u32 = 16;
    const UI_STATE_B_CAPTURE_FRAME: u32 = 4;
    const UI_STATE_C_CAPTURE_FRAME: u32 = 8;
    const DEFAULT_CHUNK_SWEEP_DRAWS: u32 = 44;
    const DEFAULT_HOOK_OVERHEAD_MS: u64 = 250;
    const MOMENT_ADVANCE_TIMEOUT_MS: u64 = 30_000;
    const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
    const WATER_CAPTURE_MOMENT: &str = "water_pass";
    const RIVERS_CAPTURE_MOMENT: &str = "rivers_pass";
    const BORDERS_CAPTURE_MOMENT: &str = "borders_pass";
    const TREES_CAPTURE_MOMENT: &str = "trees_pass";
    const RESTORE_CAPTURE_MOMENT: &str = "restore_pass";
    const MAPNAME_CAPTURE_MOMENT: &str = "mapname_pass";
    const UI_PASS_CAPTURE_MOMENT: &str = "ui_pass";
    const UI_INTEROP_HAZARD_UNOBTAINED_REASON: &str = "d3dkmt_openresource_interop_hazard";
    const UI_HAZARD_DRAW_INDEX: u32 = 2;
    const UI_LABEL_COMMAND_CAPTURE_FRAMES: u32 = 5;

    pub fn run() -> Result<()> {
        let config = Config::from_args()?;
        let capture_output_dir = env::var_os("TERRAIN_CAPTURE_OUTPUT_DIR").map(PathBuf::from);
        println!(
            "capture_harness starting bundle={} stress={} d3d_debug={} live_shape_layers={} skip_capture_waits={} ui_only={} restore_pass={} mapname_pass={} ui_cb_skip={} startup_delay_ms={} load_burst_ms={} load_burst_shaders_per_frame={} hook_overhead_ms={} terrain_frames={} water_frames={} multi_thread_ms={} river_frames={} border_frames={} tree_frames={} ui_frames={} ui_draws_per_frame={} chunk_sweep_draws={}",
            config.bundle.display(),
            config.stress,
            config.d3d_debug,
            config.live_shape_layers,
            config.skip_capture_waits,
            config.ui_only,
            config.restore_pass,
            config.mapname_pass,
            config.ui_cb_skip.as_str(),
            config.startup_delay_ms,
            config.load_burst_ms,
            config.load_burst_shaders_per_frame,
            config.hook_overhead_ms,
            config.terrain_frames,
            config.water_frames,
            config.multi_thread_ms,
            config.river_frames,
            config.border_frames,
            config.tree_frames,
            config.ui_frames,
            config.ui_draws_per_frame,
            config.chunk_sweep_draws
        );
        if config.startup_delay_ms > 0 {
            thread::sleep(Duration::from_millis(config.startup_delay_ms));
        }

        if config.ui_only {
            let d3d = D3dApp::new(config.d3d_debug)?;
            let foreign_ui = unsafe { ForeignUiResources::new(d3d.device.as_ptr())? };
            let ui_resources = unsafe { UiResources::new(d3d.device.as_ptr())? };
            unsafe {
                ui_only_phase(
                    &d3d,
                    &ui_resources,
                    &foreign_ui,
                    config.ui_frames,
                    config.ui_draws_per_frame,
                    capture_output_dir.as_deref(),
                    config.ui_cb_skip,
                    config.skip_capture_waits,
                )?;
            }
            println!("capture_harness phase complete: ui-pass");
            d3d.clear_state_and_flush();
            println!("capture_harness completed cleanly");
            return Ok(());
        }

        let terrain_bundle_path = config.bundle.join("map_frame_bundle.json");
        let terrain_bundle = CaptureBundle::read(&terrain_bundle_path)?;
        let water_bundle_path = config.bundle.join("map_frame_water_pass_bundle.json");
        let water_bundle = if water_bundle_path.is_file() {
            CaptureBundle::read(&water_bundle_path)?
        } else {
            CaptureBundle::read(&terrain_bundle_path)?
        };
        let d3d = D3dApp::new(config.d3d_debug)?;

        let filler_vs = compile_shader(FILLER_VS, "main", "vs_5_0")?;
        let filler_ps = compile_shader(FILLER_PS, "main", "ps_5_0")?;
        let river_ps = compile_shader(RIVER_PS, "main", "ps_5_0")?;
        let border_ps = compile_shader(BORDER_PS, "main", "ps_5_0")?;
        let tree_ps = compile_shader(TREE_PS, "main", "ps_5_0")?;
        let non_terrain_vs = unsafe { create_vertex_shader(d3d.device.as_ptr(), &filler_vs)? };
        let non_terrain_ps = unsafe { create_pixel_shader(d3d.device.as_ptr(), &filler_ps)? };
        let overhead_index_buffer = unsafe {
            create_buffer(
                d3d.device.as_ptr(),
                D3D11_BIND_INDEX_BUFFER,
                &[0u8, 0, 1, 0, 2, 0],
            )?
        };
        let foreign_ui = unsafe { ForeignUiResources::new(d3d.device.as_ptr())? };
        let ui_resources = unsafe { UiResources::new(d3d.device.as_ptr())? };
        let restore_resources = if config.restore_pass {
            Some(unsafe { RestoreResources::new(d3d.device.as_ptr())? })
        } else {
            None
        };
        let mapname_resources = if config.mapname_pass {
            Some(unsafe { MapnameResources::new(d3d.device.as_ptr())? })
        } else {
            None
        };

        let mut resources = HarnessResources::new(config.bundle.clone());
        let terrain_pass =
            unsafe { resources.prepare_pass(d3d.device.as_ptr(), &terrain_bundle, "terrain")? };
        let water_pass =
            unsafe { resources.prepare_pass(d3d.device.as_ptr(), &water_bundle, "water")? };
        let river_shader = unsafe { create_pixel_shader(d3d.device.as_ptr(), &river_ps)? };
        let border_shader = unsafe { create_pixel_shader(d3d.device.as_ptr(), &border_ps)? };
        let tree_shader = unsafe { create_pixel_shader(d3d.device.as_ptr(), &tree_ps)? };
        let river_resources = unsafe {
            resources.prepare_synthetic_shader_resources(d3d.device.as_ptr(), "river", 6)?
        };
        let border_resources = unsafe {
            resources.prepare_synthetic_shader_resources(d3d.device.as_ptr(), "border", 4)?
        };
        let tree_resources = unsafe {
            resources.prepare_synthetic_shader_resources(d3d.device.as_ptr(), "tree", 7)?
        };

        unsafe {
            hook_overhead_stress_phase(
                "unarmed_irrelevant",
                &d3d,
                non_terrain_vs.as_ptr(),
                non_terrain_ps.as_ptr(),
                overhead_index_buffer.as_ptr(),
                config.hook_overhead_ms,
                true,
            )?;
        }
        println!("capture_harness phase complete: hook-overhead-unarmed");

        unsafe {
            load_burst_phase(
                &d3d,
                &config.bundle,
                &filler_vs,
                &filler_ps,
                config.load_burst_ms,
                config.load_burst_shaders_per_frame,
            )?;
        }
        println!("capture_harness phase complete: load-burst");

        unsafe {
            census_load_screen_phase(
                &d3d,
                non_terrain_vs.as_ptr(),
                non_terrain_ps.as_ptr(),
                overhead_index_buffer.as_ptr(),
                config.census_load_screen_frames,
            )?;
        }
        println!("capture_harness phase complete: census-load-screen");

        unsafe {
            hook_overhead_stress_phase(
                "armed_irrelevant",
                &d3d,
                non_terrain_vs.as_ptr(),
                non_terrain_ps.as_ptr(),
                overhead_index_buffer.as_ptr(),
                config.hook_overhead_ms,
                config.stress,
            )?;
        }
        println!("capture_harness phase complete: hook-overhead-armed");

        for _ in 0..config.terrain_frames {
            unsafe {
                draw_prepared_pass(&d3d, &terrain_pass, non_terrain_ps.as_ptr())?;
            }
        }
        if !config.skip_capture_waits {
            unsafe {
                wait_for_capture_moment_advanced(
                    &d3d,
                    non_terrain_ps.as_ptr(),
                    capture_output_dir.as_deref(),
                    TERRAIN_CAPTURE_MOMENT,
                )?;
            }
        }
        println!("capture_harness phase complete: terrain-pass");

        if let Some(restore_resources) = restore_resources.as_ref() {
            unsafe {
                draw_restore_pass(&d3d, restore_resources, non_terrain_ps.as_ptr())?;
            }
            if !config.skip_capture_waits {
                unsafe {
                    wait_for_capture_moment_advanced(
                        &d3d,
                        non_terrain_ps.as_ptr(),
                        capture_output_dir.as_deref(),
                        RESTORE_CAPTURE_MOMENT,
                    )?;
                }
            }
            println!("capture_harness phase complete: restore-pass");
        }

        if let Some(mapname_resources) = mapname_resources.as_ref() {
            unsafe {
                draw_mapname_pass(&d3d, mapname_resources, non_terrain_ps.as_ptr())?;
            }
            if !config.skip_capture_waits {
                unsafe {
                    wait_for_capture_moment_advanced(
                        &d3d,
                        non_terrain_ps.as_ptr(),
                        capture_output_dir.as_deref(),
                        MAPNAME_CAPTURE_MOMENT,
                    )?;
                }
            }
            println!("capture_harness phase complete: mapname-pass");
        }

        for _ in 0..config.water_frames {
            unsafe {
                draw_prepared_pass(&d3d, &water_pass, non_terrain_ps.as_ptr())?;
            }
        }
        if !config.skip_capture_waits {
            unsafe {
                wait_for_capture_moment_advanced(
                    &d3d,
                    non_terrain_ps.as_ptr(),
                    capture_output_dir.as_deref(),
                    WATER_CAPTURE_MOMENT,
                )?;
            }
        }
        println!("capture_harness phase complete: water-pass");

        // Persistent-CB0-sampler verification: the terrain and water moments are
        // now COMPLETE. Keep issuing tracked terrain draws (terrain PS + scene
        // CB0 bound) on the render thread so the DLL's --cb0-sample-ms sampler
        // provably keeps appending past moment completion — the exact survival
        // the two prior day/night sessions lacked. No-op unless --cb0-verify-ms.
        if config.cb0_verify_ms > 0 {
            unsafe {
                cb0_sampler_verify_phase(
                    &d3d,
                    &terrain_pass,
                    non_terrain_ps.as_ptr(),
                    config.cb0_verify_ms,
                )?;
            }
            println!("capture_harness phase complete: cb0-sampler-verify");
        }

        multi_thread_phase(
            &d3d,
            &terrain_pass,
            non_terrain_ps.as_ptr(),
            config.multi_thread_ms,
        )?;
        println!("capture_harness phase complete: multi-thread");

        unsafe {
            river_and_chunk_phase(
                &d3d,
                &terrain_pass,
                river_shader.as_ptr(),
                &river_resources,
                config.river_frames,
                border_shader.as_ptr(),
                &border_resources,
                config.border_frames,
                tree_shader.as_ptr(),
                &tree_resources,
                config.tree_frames,
                non_terrain_ps.as_ptr(),
                config.chunk_sweep_draws,
                capture_output_dir.as_deref(),
                config.live_shape_layers,
                config.skip_capture_waits,
            )?;
        }
        println!("capture_harness phase complete: rivers/chunk");

        unsafe {
            ui_phase(
                &d3d,
                &ui_resources,
                &foreign_ui,
                config.ui_frames,
                config.ui_draws_per_frame,
                capture_output_dir.as_deref(),
                config.ui_cb_skip,
                config.skip_capture_waits,
            )?;
        }
        println!("capture_harness phase complete: ui-pass");

        // MOMFIX1 round driver: with the whole continuous control sequence
        // (including the round-EXCLUDED restore_pass/mapname_pass moments) now
        // captured, drive a manual round. HOLD guarantees the round could not
        // have started earlier and starved those excluded moments; here the round
        // must start promptly and land a completed round dir alongside them.
        if let Some(label) = config.round_after_moments.as_deref() {
            unsafe {
                drive_map_frame_round_after_moments(
                    &d3d,
                    &terrain_pass,
                    &water_pass,
                    non_terrain_ps.as_ptr(),
                    capture_output_dir.as_deref(),
                    label,
                    config.skip_capture_waits,
                )?;
            }
            println!("capture_harness phase complete: map-frame-round");
        }

        for _ in 0..12 {
            d3d.pump_messages();
            unsafe {
                clear_and_present(&d3d, non_terrain_ps.as_ptr())?;
            }
            thread::sleep(Duration::from_millis(16));
        }

        d3d.clear_state_and_flush();
        println!("capture_harness completed cleanly");
        Ok(())
    }

    /// MOMFIX1: write a manual capture_map_frames_round command to the control
    /// file, then re-render the map passes so the DLL can capture the round's
    /// filtered sequence, and wait for map_frame_capture_round_finished.
    #[allow(clippy::too_many_arguments)]
    unsafe fn drive_map_frame_round_after_moments(
        d3d: &D3dApp,
        terrain_pass: &PreparedPass,
        water_pass: &PreparedPass,
        non_terrain_ps: *mut ID3D11PixelShader,
        capture_output_dir: Option<&Path>,
        label: &str,
        skip_capture_waits: bool,
    ) -> Result<()> {
        let control_path = env::var_os("TERRAIN_CAPTURE_CONTROL_PATH").map(PathBuf::from);
        let Some(control_path) = control_path else {
            return Err(
                "TERRAIN_CAPTURE_CONTROL_PATH not set; cannot drive a map-frame round".into(),
            );
        };
        fs::write(
            &control_path,
            format!(r#"{{"type":"capture_map_frames_round","label":"{label}"}}"#),
        )
        .map_err(|error| {
            format!(
                "failed to write capture_map_frames_round command {}: {error}",
                control_path.display()
            )
        })?;
        println!("capture_harness wrote round command label={label}");

        // Re-render the map passes so the round's filtered sequence (terrain,
        // water, ...) has live draws to capture. Bounded frame budget; the wait
        // below stops early once the round reports finished.
        let needle = format!("\"type\":\"map_frame_capture_round_finished\",\"label\":\"{label}\"");
        let events_path = capture_output_dir.map(|dir| dir.join("terrain_capture_events.jsonl"));
        let started = Instant::now();
        let timeout = Duration::from_millis(MOMENT_ADVANCE_TIMEOUT_MS);
        let mut frame = 0u32;
        loop {
            if let Some(events_path) = events_path.as_ref() {
                if !skip_capture_waits
                    && fs::read_to_string(events_path)
                        .map(|events| events.contains(&needle))
                        .unwrap_or(false)
                {
                    println!("capture_harness observed round finished label={label}");
                    return Ok(());
                }
            }
            if started.elapsed() >= timeout {
                if skip_capture_waits || capture_output_dir.is_none() {
                    return Ok(());
                }
                return Err(format!(
                    "timed out waiting for round {label} to finish; checked {}",
                    events_path
                        .map(|path| path.display().to_string())
                        .unwrap_or_default()
                )
                .into());
            }
            d3d.pump_messages();
            unsafe {
                draw_prepared_pass(d3d, terrain_pass, non_terrain_ps)?;
                draw_prepared_pass(d3d, water_pass, non_terrain_ps)?;
            }
            frame = frame.saturating_add(1);
            let _ = frame;
            thread::sleep(Duration::from_millis(16));
        }
    }

    #[derive(Clone, Debug)]
    struct Config {
        bundle: PathBuf,
        stress: bool,
        d3d_debug: bool,
        live_shape_layers: bool,
        skip_capture_waits: bool,
        ui_only: bool,
        restore_pass: bool,
        mapname_pass: bool,
        ui_cb_skip: UiCbSkipMode,
        startup_delay_ms: u64,
        load_burst_ms: u64,
        load_burst_shaders_per_frame: u32,
        hook_overhead_ms: u64,
        terrain_frames: u32,
        water_frames: u32,
        multi_thread_ms: u64,
        river_frames: u32,
        border_frames: u32,
        tree_frames: u32,
        ui_frames: u32,
        ui_draws_per_frame: u32,
        chunk_sweep_draws: u32,
        census_load_screen_frames: u32,
        // Persistent-CB0-sampler verification: after the terrain + water moments
        // complete, keep issuing tracked terrain draws (terrain PS + scene CB0
        // bound) on the render thread for this many ms so the DLL's --cb0-sample-ms
        // sampler provably keeps appending PAST moment completion. 0 => skip.
        cb0_verify_ms: u64,
        // MOMFIX1 round driver: after the CONTINUOUS control sequence (including
        // the round-excluded restore_pass/mapname_pass/ui_pass moments) has
        // completed, write a manual capture_map_frames_round command to the
        // control file, then re-render the map passes so the round captures its
        // filtered sequence, and wait for map_frame_capture_round_finished. This
        // exercises the HOLD gate: the round can only start AFTER the excluded
        // moments land, and BOTH the excluded bundle and a completed round must
        // exist on disk. Empty => no round driven.
        round_after_moments: Option<String>,
    }

    impl Config {
        fn from_args() -> Result<Self> {
            let args = env::args().skip(1).collect::<Vec<_>>();
            if args.iter().any(|arg| arg == "--help" || arg == "-h") {
                print_help();
                std::process::exit(0);
            }
            let quick = args.iter().any(|arg| arg == "--quick");
            let stress = args.iter().any(|arg| arg == "--stress");
            let d3d_debug = args.iter().any(|arg| arg == "--d3d-debug");
            let live_shape_layers = args.iter().any(|arg| arg == "--live-shape-layers");
            let skip_capture_waits = args.iter().any(|arg| arg == "--skip-capture-waits");
            let ui_only = args.iter().any(|arg| arg == "--ui-only");
            let restore_pass = args.iter().any(|arg| arg == "--restore-pass");
            let mapname_pass = args.iter().any(|arg| arg == "--mapname-pass");
            let ui_cb_skip = value_after(&args, "--ui-cb-skip")
                .map(|value| UiCbSkipMode::parse(&value))
                .transpose()?
                .unwrap_or_default();
            let startup_delay_ms = value_after(&args, "--startup-delay-ms")
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(0);
            let bundle = value_after(&args, "--bundle")
                .map(PathBuf::from)
                .unwrap_or_else(default_bundle_dir);
            let load_burst_ms = value_after(&args, "--load-burst-ms")
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(if quick { 700 } else { DEFAULT_LOAD_BURST_MS });
            let load_burst_shaders_per_frame = value_after(&args, "--load-burst-shaders-per-frame")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if stress {
                    STRESS_LOAD_BURST_SHADERS_PER_FRAME
                } else {
                    DEFAULT_LOAD_BURST_SHADERS_PER_FRAME
                });
            let hook_overhead_ms = value_after(&args, "--hook-overhead-ms")
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(if quick { 100 } else { DEFAULT_HOOK_OVERHEAD_MS });
            let terrain_frames = value_after(&args, "--terrain-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick { 2 } else { DEFAULT_TERRAIN_FRAMES });
            let water_frames = value_after(&args, "--water-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick { 1 } else { DEFAULT_WATER_FRAMES });
            let multi_thread_ms = value_after(&args, "--multi-thread-ms")
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(if quick { 700 } else { DEFAULT_MULTI_THREAD_MS });
            let river_frames = value_after(&args, "--river-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick { 16 } else { DEFAULT_RIVER_FRAMES });
            let border_frames = value_after(&args, "--border-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick { 4 } else { DEFAULT_BORDER_FRAMES });
            let tree_frames = value_after(&args, "--tree-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick { 4 } else { DEFAULT_TREE_FRAMES });
            let ui_frames = value_after(&args, "--ui-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick {
                    UI_ATLAS_MIN_FRAMES
                } else {
                    DEFAULT_UI_FRAMES
                });
            let ui_draws_per_frame = value_after(&args, "--ui-draws-per-frame")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick {
                    12
                } else {
                    DEFAULT_UI_DRAWS_PER_FRAME
                });
            let chunk_sweep_draws = value_after(&args, "--chunk-sweep-draws")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(if quick { 44 } else { DEFAULT_CHUNK_SWEEP_DRAWS });
            let census_load_screen_frames = value_after(&args, "--census-load-screen-frames")
                .map(|value| value.parse::<u32>())
                .transpose()?
                .unwrap_or(0);
            let cb0_verify_ms = value_after(&args, "--cb0-verify-ms")
                .map(|value| value.parse::<u64>())
                .transpose()?
                .unwrap_or(0);
            let round_after_moments = value_after(&args, "--round-after-moments")
                .map(|value| value.trim().to_string())
                .filter(|label| !label.is_empty());
            Ok(Self {
                bundle,
                stress,
                d3d_debug,
                live_shape_layers,
                skip_capture_waits,
                ui_only,
                restore_pass,
                mapname_pass,
                ui_cb_skip,
                startup_delay_ms,
                load_burst_ms,
                load_burst_shaders_per_frame,
                hook_overhead_ms,
                terrain_frames,
                water_frames,
                multi_thread_ms,
                river_frames,
                border_frames,
                tree_frames,
                ui_frames,
                ui_draws_per_frame,
                chunk_sweep_draws,
                census_load_screen_frames,
                cb0_verify_ms,
                round_after_moments,
            })
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    enum UiCbSkipMode {
        #[default]
        Hazard,
        All,
        None,
    }

    impl UiCbSkipMode {
        fn parse(value: &str) -> Result<Self> {
            match value {
                "hazard" => Ok(Self::Hazard),
                "all" => Ok(Self::All),
                "none" => Ok(Self::None),
                other => Err(format!(
                    "unsupported --ui-cb-skip {other}; expected hazard, all, or none"
                )
                .into()),
            }
        }

        fn as_str(self) -> &'static str {
            match self {
                Self::Hazard => "hazard",
                Self::All => "all",
                Self::None => "none",
            }
        }

        fn skips_draw(self, hazard_draw: bool) -> bool {
            match self {
                Self::Hazard => hazard_draw,
                Self::All => true,
                Self::None => false,
            }
        }
    }

    fn print_help() {
        println!(
            "capture_harness [--bundle <session_20260702d dir>] [--quick] [--stress] [--d3d-debug] \
             [--load-burst-ms N] [--terrain-frames N] [--water-frames N] \
              [--multi-thread-ms N] [--river-frames N] [--border-frames N] \
              [--tree-frames N] [--ui-frames N] [--ui-draws-per-frame N] [--chunk-sweep-draws N] \
              [--load-burst-shaders-per-frame N] [--hook-overhead-ms N] [--live-shape-layers] \
              [--ui-only] [--restore-pass] [--mapname-pass] [--skip-capture-waits] [--ui-cb-skip hazard|all|none] [--startup-delay-ms N] \
              [--census-load-screen-frames N (default 0: filler-shader draws before any map phase,\
               for the census untrusted-context gate)] \
              [--round-after-moments <label> (MOMFIX1: after the continuous control sequence \
               completes, write a capture_map_frames_round command and re-render map passes so the \
               round captures its filtered sequence; requires --enable-map-frame-rounds on the attach side)]"
        );
    }

    fn value_after(args: &[String], flag: &str) -> Option<String> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
    }

    fn default_bundle_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../map_viewer/out/captures/session_20260702d")
    }

    #[derive(Debug, Deserialize)]
    struct CaptureBundle {
        draws: Vec<DrawRecord>,
    }

    impl CaptureBundle {
        fn read(path: &Path) -> Result<Self> {
            let bytes = fs::read(path)
                .map_err(|error| format!("failed to read bundle {}: {error}", path.display()))?;
            serde_json::from_slice(&bytes).map_err(|error| {
                format!("failed to parse bundle {}: {error}", path.display()).into()
            })
        }
    }

    #[derive(Debug, Deserialize)]
    struct DrawRecord {
        draw_call: String,
        draw_count: u32,
        draw_start: u32,
        #[serde(default)]
        base_vertex_location: i32,
        shader: ShaderRecord,
        vertex_shader: ShaderRecord,
        input_layout: InputLayoutRecord,
        #[serde(default)]
        vertex_buffers: BTreeMap<String, BufferBindingRecord>,
        #[serde(default)]
        index_buffer: Option<BufferBindingRecord>,
        #[serde(default)]
        constant_buffers: Vec<BufferBindingRecord>,
        #[serde(default)]
        shader_resources: Vec<ShaderResourceRecord>,
    }

    #[derive(Debug, Deserialize)]
    struct ShaderRecord {
        bytecode_file: String,
    }

    #[derive(Debug, Deserialize)]
    struct InputLayoutRecord {
        #[serde(default)]
        elements: Vec<InputElementRecord>,
    }

    #[derive(Debug, Deserialize)]
    struct InputElementRecord {
        semantic_name: String,
        #[serde(default)]
        semantic_index: u32,
        format: u32,
        input_slot: u32,
        aligned_byte_offset: u32,
        #[serde(default = "input_per_vertex")]
        input_slot_class: u32,
        #[serde(default)]
        instance_data_step_rate: u32,
    }

    fn input_per_vertex() -> u32 {
        D3D11_INPUT_PER_VERTEX_DATA as u32
    }

    #[derive(Debug, Deserialize)]
    struct BufferBindingRecord {
        #[serde(default)]
        stage: Option<String>,
        #[serde(default)]
        slot: Option<u32>,
        file: Option<String>,
        #[serde(default)]
        dxgi_format: Option<u32>,
        #[serde(default)]
        offset: Option<u32>,
        #[serde(default)]
        bind_flags: Option<u32>,
        #[serde(default)]
        stride: Option<u32>,
    }

    #[derive(Debug, Deserialize)]
    struct ShaderResourceRecord {
        slot: u32,
        #[serde(default)]
        declared_name: Option<String>,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        is_null: bool,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        #[serde(default)]
        dxgi_format: Option<u32>,
        #[serde(default)]
        mip_count: Option<u32>,
        #[serde(default)]
        array_size: Option<u32>,
        #[serde(default)]
        sample_count: Option<u32>,
        #[serde(default)]
        sample_quality: Option<u32>,
        #[serde(default)]
        bind_flags: Option<u32>,
        #[serde(default)]
        misc_flags: Option<u32>,
    }

    struct PreparedPass {
        draws: Vec<PreparedDraw>,
    }

    struct PreparedDraw {
        draw_call: String,
        draw_count: u32,
        draw_start: u32,
        base_vertex_location: i32,
        vertex_shader: *mut ID3D11VertexShader,
        pixel_shader: *mut ID3D11PixelShader,
        input_layout: ComPtr<ID3D11InputLayout>,
        _vertex_buffers: Vec<ComPtr<ID3D11Buffer>>,
        vertex_buffer_ptrs: Vec<*mut ID3D11Buffer>,
        strides: Vec<u32>,
        offsets: Vec<u32>,
        index_buffer: Option<ComPtr<ID3D11Buffer>>,
        index_format: DXGI_FORMAT,
        index_offset: u32,
        vs_constant_buffers: Vec<(u32, ComPtr<ID3D11Buffer>)>,
        ps_constant_buffers: Vec<(u32, ComPtr<ID3D11Buffer>)>,
        shader_resources: Vec<*mut ID3D11ShaderResourceView>,
        cb1_template: Option<Vec<u8>>,
    }

    struct HarnessResources {
        bundle_root: PathBuf,
        vertex_shaders: HashMap<String, ComPtr<ID3D11VertexShader>>,
        pixel_shaders: HashMap<String, ComPtr<ID3D11PixelShader>>,
        srv_cache: HashMap<String, usize>,
        srvs: Vec<ComPtr<ID3D11ShaderResourceView>>,
    }

    impl HarnessResources {
        fn new(bundle_root: PathBuf) -> Self {
            Self {
                bundle_root,
                vertex_shaders: HashMap::new(),
                pixel_shaders: HashMap::new(),
                srv_cache: HashMap::new(),
                srvs: Vec::new(),
            }
        }

        unsafe fn prepare_pass(
            &mut self,
            device: *mut ID3D11Device,
            bundle: &CaptureBundle,
            label: &str,
        ) -> Result<PreparedPass> {
            let mut draws = Vec::new();
            for draw in &bundle.draws {
                draws.push(unsafe { self.prepare_draw(device, draw, label)? });
            }
            Ok(PreparedPass { draws })
        }

        unsafe fn prepare_draw(
            &mut self,
            device: *mut ID3D11Device,
            draw: &DrawRecord,
            label: &str,
        ) -> Result<PreparedDraw> {
            let vs = unsafe { self.vertex_shader(device, &draw.vertex_shader.bytecode_file)? };
            let ps = unsafe { self.pixel_shader(device, &draw.shader.bytecode_file)? };
            let vs_bytes = fs::read(self.bundle_root.join(&draw.vertex_shader.bytecode_file))?;
            let input_layout =
                unsafe { create_input_layout(device, &draw.input_layout.elements, &vs_bytes)? };

            let mut vertex_slots = draw
                .vertex_buffers
                .iter()
                .filter_map(|(slot, binding)| slot.parse::<u32>().ok().map(|slot| (slot, binding)))
                .collect::<Vec<_>>();
            vertex_slots.sort_by_key(|(slot, _)| *slot);
            let mut vertex_buffers = Vec::new();
            let mut vertex_buffer_ptrs = Vec::new();
            let mut strides = Vec::new();
            let mut offsets = Vec::new();
            for (_, binding) in vertex_slots {
                let file = binding
                    .file
                    .as_deref()
                    .ok_or_else(|| format!("{label} draw vertex buffer has no file"))?;
                let bytes = fs::read(self.bundle_root.join(file))?;
                let bind = binding.bind_flags.unwrap_or(D3D11_BIND_VERTEX_BUFFER);
                let buffer = unsafe { create_buffer(device, bind, &bytes)? };
                vertex_buffer_ptrs.push(buffer.as_ptr());
                vertex_buffers.push(buffer);
                strides.push(binding.stride.unwrap_or(0));
                offsets.push(binding.offset.unwrap_or(0));
            }

            let (index_buffer, index_format, index_offset) = if let Some(index) = &draw.index_buffer
            {
                let file = index.file.as_deref().ok_or_else(|| {
                    format!("{label} indexed draw has an index buffer without a file")
                })?;
                let bytes = fs::read(self.bundle_root.join(file))?;
                (
                    Some(unsafe {
                        create_buffer(
                            device,
                            index.bind_flags.unwrap_or(D3D11_BIND_INDEX_BUFFER),
                            &bytes,
                        )?
                    }),
                    index.dxgi_format.unwrap_or(DXGI_FORMAT_R16_UINT as u32) as DXGI_FORMAT,
                    index.offset.unwrap_or(0),
                )
            } else {
                (None, DXGI_FORMAT_R16_UINT, 0)
            };

            let mut vs_constant_buffers = Vec::new();
            let mut ps_constant_buffers = Vec::new();
            let mut cb1_template = None;
            for binding in &draw.constant_buffers {
                let Some(file) = binding.file.as_deref() else {
                    continue;
                };
                let mut bytes = fs::read(self.bundle_root.join(file))?;
                let bind = binding.bind_flags.unwrap_or(D3D11_BIND_CONSTANT_BUFFER);
                pad_constant_buffer(&mut bytes);
                let buffer = unsafe { create_buffer(device, bind, &bytes)? };
                let slot = binding.slot.unwrap_or(0);
                match binding.stage.as_deref() {
                    Some("VS") => {
                        if slot == 1 {
                            cb1_template = Some(bytes.clone());
                        }
                        vs_constant_buffers.push((slot, buffer));
                    }
                    Some("PS") => ps_constant_buffers.push((slot, buffer)),
                    _ => {}
                }
            }

            let mut shader_resources = vec![ptr::null_mut(); 16];
            for resource in &draw.shader_resources {
                if resource.is_null {
                    continue;
                }
                let Some(srv) = (unsafe { self.shader_resource_view(device, resource)? }) else {
                    continue;
                };
                if resource.slot as usize >= shader_resources.len() {
                    shader_resources.resize(resource.slot as usize + 1, ptr::null_mut());
                }
                shader_resources[resource.slot as usize] = srv;
            }

            Ok(PreparedDraw {
                draw_call: draw.draw_call.clone(),
                draw_count: draw.draw_count,
                draw_start: draw.draw_start,
                base_vertex_location: draw.base_vertex_location,
                vertex_shader: vs,
                pixel_shader: ps,
                input_layout,
                _vertex_buffers: vertex_buffers,
                vertex_buffer_ptrs,
                strides,
                offsets,
                index_buffer,
                index_format,
                index_offset,
                vs_constant_buffers,
                ps_constant_buffers,
                shader_resources,
                cb1_template,
            })
        }

        unsafe fn vertex_shader(
            &mut self,
            device: *mut ID3D11Device,
            file: &str,
        ) -> Result<*mut ID3D11VertexShader> {
            if !self.vertex_shaders.contains_key(file) {
                let bytes = fs::read(self.bundle_root.join(file))?;
                let shader = unsafe { create_vertex_shader(device, &bytes)? };
                self.vertex_shaders.insert(file.to_string(), shader);
            }
            Ok(self.vertex_shaders.get(file).unwrap().as_ptr())
        }

        unsafe fn pixel_shader(
            &mut self,
            device: *mut ID3D11Device,
            file: &str,
        ) -> Result<*mut ID3D11PixelShader> {
            if !self.pixel_shaders.contains_key(file) {
                let bytes = fs::read(self.bundle_root.join(file))?;
                let shader = unsafe { create_pixel_shader(device, &bytes)? };
                self.pixel_shaders.insert(file.to_string(), shader);
            }
            Ok(self.pixel_shaders.get(file).unwrap().as_ptr())
        }

        unsafe fn shader_resource_view(
            &mut self,
            device: *mut ID3D11Device,
            resource: &ShaderResourceRecord,
        ) -> Result<Option<*mut ID3D11ShaderResourceView>> {
            let width = resource.width.unwrap_or(0);
            let height = resource.height.unwrap_or(0);
            let format = resource.dxgi_format.unwrap_or(0);
            if width == 0 || height == 0 || format == 0 {
                return Ok(None);
            }
            let key = resource
                .file
                .clone()
                .or_else(|| resource.declared_name.clone())
                .unwrap_or_else(|| format!("slot{}", resource.slot));
            if let Some(index) = self.srv_cache.get(&key).copied() {
                return Ok(Some(self.srvs[index].as_ptr()));
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: resource.mip_count.unwrap_or(1).max(1),
                ArraySize: resource.array_size.unwrap_or(1).max(1),
                Format: format as DXGI_FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: resource.sample_count.unwrap_or(1).max(1),
                    Quality: resource.sample_quality.unwrap_or(0),
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: resource.bind_flags.unwrap_or(D3D11_BIND_SHADER_RESOURCE),
                CPUAccessFlags: 0,
                MiscFlags: resource.misc_flags.unwrap_or(0),
            };
            let srv = unsafe { create_texture_srv(device, &desc)? };
            self.srvs.push(srv);
            let index = self.srvs.len() - 1;
            self.srv_cache.insert(key, index);
            Ok(Some(self.srvs[index].as_ptr()))
        }

        unsafe fn prepare_synthetic_shader_resources(
            &mut self,
            device: *mut ID3D11Device,
            label: &str,
            slot_count: u32,
        ) -> Result<Vec<*mut ID3D11ShaderResourceView>> {
            let mut srvs = Vec::new();
            for slot in 0..slot_count {
                let record = ShaderResourceRecord {
                    slot,
                    declared_name: Some(format!("{label}_slot_{slot}")),
                    file: Some(format!("synthetic_{label}_slot_{slot}")),
                    is_null: false,
                    width: Some(32),
                    height: Some(32),
                    dxgi_format: Some(DXGI_FORMAT_R8G8B8A8_UNORM as u32),
                    mip_count: Some(1),
                    array_size: Some(1),
                    sample_count: Some(1),
                    sample_quality: Some(0),
                    bind_flags: Some(D3D11_BIND_SHADER_RESOURCE),
                    misc_flags: Some(0),
                };
                let srv = unsafe { self.shader_resource_view(device, &record)? }
                    .ok_or("failed to create synthetic shader SRV")?;
                srvs.push(srv);
            }
            Ok(srvs)
        }
    }

    struct UiResources {
        vertex_shader: ComPtr<ID3D11VertexShader>,
        pixel_shader: ComPtr<ID3D11PixelShader>,
        input_layout: ComPtr<ID3D11InputLayout>,
        index_buffer: ComPtr<ID3D11Buffer>,
        textures: Vec<ComPtr<ID3D11ShaderResourceView>>,
        vs_constant_buffer: ComPtr<ID3D11Buffer>,
        ps_constant_buffer: ComPtr<ID3D11Buffer>,
        blend_state: ComPtr<ID3D11BlendState>,
        depth_disabled_state: ComPtr<ID3D11DepthStencilState>,
        rasterizer_state: ComPtr<ID3D11RasterizerState>,
    }

    /// Emulates draws whose pipeline state the capture DLL must refuse to
    /// read: the draw binds a NULL pixel shader (a legal depth-only style
    /// pipeline), so the DLL classifies it as an unknown/foreign-shader draw
    /// and skips all state reads for it.
    ///
    /// Historical note: this emulation previously created the VS/PS on a
    /// second D3D11 device and bound those objects on the main device's
    /// immediate context. Cross-device binding of context state objects is
    /// undefined behavior in D3D11 and was proven (lane bisect, 2026-07-04)
    /// to plant latent driver corruption that intermittently killed the
    /// harness with 0xC0000005 once census-mode capture interleaved
    /// additional context calls. The dedicated vertex shader below is
    /// created on the MAIN device (legal) but is used exclusively by this
    /// always-skipped draw, so it must never appear in captured bundles.
    struct ForeignUiResources {
        vertex_shader: ComPtr<ID3D11VertexShader>,
        vertex_buffer: ComPtr<ID3D11Buffer>,
        texture: ComPtr<ID3D11ShaderResourceView>,
        vs_constant_buffer: ComPtr<ID3D11Buffer>,
        ps_constant_buffer: ComPtr<ID3D11Buffer>,
        vertex_bytes: Vec<u8>,
    }

    impl ForeignUiResources {
        unsafe fn new(main_device: *mut ID3D11Device) -> Result<Self> {
            let vs_bytes = compile_shader(FOREIGN_UI_VS, "main", "vs_5_0")?;
            let vertex_shader = unsafe { create_vertex_shader(main_device, &vs_bytes)? };
            let vertex_bytes = foreign_ui_quad_vertices();
            let vertex_buffer =
                unsafe { create_buffer(main_device, D3D11_BIND_VERTEX_BUFFER, &vertex_bytes)? };
            let texture = unsafe {
                create_texture_srv_with_bytes(
                    main_device,
                    4,
                    4,
                    &ui_texture_bytes(0x13, 0xC7, 0x5A),
                )?
            };
            let mut vs_cb = Vec::new();
            for value in [0.125f32, 0.25, 0.5, 0.75] {
                vs_cb.extend_from_slice(&value.to_le_bytes());
            }
            let mut ps_cb = Vec::new();
            for value in [0.91f32, 0.37, 0.19, 1.0] {
                ps_cb.extend_from_slice(&value.to_le_bytes());
            }
            let vs_constant_buffer =
                unsafe { create_buffer(main_device, D3D11_BIND_CONSTANT_BUFFER, &vs_cb)? };
            let ps_constant_buffer =
                unsafe { create_buffer(main_device, D3D11_BIND_CONSTANT_BUFFER, &ps_cb)? };
            Ok(Self {
                vertex_shader,
                vertex_buffer,
                texture,
                vs_constant_buffer,
                ps_constant_buffer,
                vertex_bytes,
            })
        }
    }

    impl UiResources {
        unsafe fn new(device: *mut ID3D11Device) -> Result<Self> {
            let vs_bytes = compile_shader(UI_VS, "main", "vs_5_0")?;
            let ps_bytes = compile_shader(UI_PS, "main", "ps_5_0")?;
            let vertex_shader = unsafe { create_vertex_shader(device, &vs_bytes)? };
            let pixel_shader = unsafe { create_pixel_shader(device, &ps_bytes)? };
            let layout = vec![
                InputElementRecord {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: 0,
                    format: DXGI_FORMAT_R32G32_FLOAT as u32,
                    input_slot: 0,
                    aligned_byte_offset: 0,
                    input_slot_class: D3D11_INPUT_PER_VERTEX_DATA as u32,
                    instance_data_step_rate: 0,
                },
                InputElementRecord {
                    semantic_name: "TEXCOORD".to_string(),
                    semantic_index: 0,
                    format: DXGI_FORMAT_R32G32_FLOAT as u32,
                    input_slot: 0,
                    aligned_byte_offset: 8,
                    input_slot_class: D3D11_INPUT_PER_VERTEX_DATA as u32,
                    instance_data_step_rate: 0,
                },
            ];
            let input_layout = unsafe { create_input_layout(device, &layout, &vs_bytes)? };
            let mut index_bytes = Vec::new();
            for index in [0u16, 1, 2, 0, 2, 3] {
                index_bytes.extend_from_slice(&index.to_le_bytes());
            }
            let index_buffer =
                unsafe { create_buffer(device, D3D11_BIND_INDEX_BUFFER, &index_bytes)? };
            let textures = vec![
                unsafe {
                    create_texture_srv_with_bytes(
                        device,
                        4,
                        4,
                        &ui_texture_bytes(0x30, 0x90, 0xE0),
                    )?
                },
                unsafe {
                    create_texture_srv_with_bytes(
                        device,
                        4,
                        4,
                        &ui_texture_bytes(0xE0, 0x70, 0x30),
                    )?
                },
            ];
            let mut vs_cb = Vec::new();
            for value in [1.0f32, 1.0, 0.0, 0.0] {
                vs_cb.extend_from_slice(&value.to_le_bytes());
            }
            let mut ps_cb = Vec::new();
            for value in [1.0f32, 1.0, 1.0, 0.92] {
                ps_cb.extend_from_slice(&value.to_le_bytes());
            }
            let vs_constant_buffer =
                unsafe { create_buffer(device, D3D11_BIND_CONSTANT_BUFFER, &vs_cb)? };
            let ps_constant_buffer =
                unsafe { create_buffer(device, D3D11_BIND_CONSTANT_BUFFER, &ps_cb)? };
            let blend_state = unsafe { create_ui_blend_state(device)? };
            let depth_disabled_state = unsafe { create_ui_depth_disabled_state(device)? };
            let rasterizer_state = unsafe { create_ui_rasterizer_state(device)? };
            Ok(Self {
                vertex_shader,
                pixel_shader,
                input_layout,
                index_buffer,
                textures,
                vs_constant_buffer,
                ps_constant_buffer,
                blend_state,
                depth_disabled_state,
                rasterizer_state,
            })
        }
    }

    struct RestoreResources {
        vertex_shader: ComPtr<ID3D11VertexShader>,
        pixel_shader: ComPtr<ID3D11PixelShader>,
        ps_constant_buffer: ComPtr<ID3D11Buffer>,
        textures: Vec<ComPtr<ID3D11ShaderResourceView>>,
    }

    impl RestoreResources {
        unsafe fn new(device: *mut ID3D11Device) -> Result<Self> {
            let vs_bytes = compile_shader(RESTORE_VS, "main", "vs_5_0")?;
            let ps_bytes = compile_shader(RESTORE_PS, "main", "ps_5_0")?;
            let vertex_shader = unsafe { create_vertex_shader(device, &vs_bytes)? };
            let pixel_shader = unsafe { create_pixel_shader(device, &ps_bytes)? };
            let textures = vec![
                unsafe {
                    create_texture_srv_with_bytes(
                        device,
                        4,
                        4,
                        &ui_texture_bytes(0x20, 0x50, 0x90),
                    )?
                },
                unsafe {
                    create_texture_srv_with_bytes(
                        device,
                        4,
                        4,
                        &ui_texture_bytes(0x0A, 0x16, 0x28),
                    )?
                },
                unsafe {
                    create_texture_srv_with_bytes(device, 1024, 32, &restore_color_cube_bytes())?
                },
                unsafe {
                    create_texture_srv_with_format_bytes(
                        device,
                        1,
                        1,
                        DXGI_FORMAT_R32_FLOAT,
                        4,
                        &0.18f32.to_le_bytes(),
                        "restore average luminance",
                    )?
                },
            ];
            let ps_constant_buffer = unsafe {
                create_buffer(
                    device,
                    D3D11_BIND_CONSTANT_BUFFER,
                    &restore_posteffect_cb_bytes(),
                )?
            };
            Ok(Self {
                vertex_shader,
                pixel_shader,
                ps_constant_buffer,
                textures,
            })
        }
    }

    // Synthetic stand-in for the mapname (country-label) pass. The VS input is
    // the discriminating signature: float3 POSITION + float2 TEXCOORD0 with a
    // slot-1 Transp_OffsetX CB. The glyph VB (float3 world positions + float2
    // uv) is the payload the moment must capture. A separate browser-overlay
    // hazard VS (float2 POSITION) must be REJECTED by the discriminator.
    struct MapnameResources {
        vertex_shader: ComPtr<ID3D11VertexShader>,
        pixel_shader: ComPtr<ID3D11PixelShader>,
        input_layout: ComPtr<ID3D11InputLayout>,
        vertex_buffer: ComPtr<ID3D11Buffer>,
        vs_scene_constant_buffer: ComPtr<ID3D11Buffer>,
        transp_offset_constant_buffer: ComPtr<ID3D11Buffer>,
        font_atlas: ComPtr<ID3D11ShaderResourceView>,
        hazard_vertex_shader: ComPtr<ID3D11VertexShader>,
        hazard_pixel_shader: ComPtr<ID3D11PixelShader>,
        hazard_vertex_buffer: ComPtr<ID3D11Buffer>,
        vertex_stride: u32,
    }

    impl MapnameResources {
        unsafe fn new(device: *mut ID3D11Device) -> Result<Self> {
            let vs_bytes = compile_shader(MAPNAME_VS, "main", "vs_5_0")?;
            let ps_bytes = compile_shader(MAPNAME_PS, "main", "ps_5_0")?;
            let vertex_shader = unsafe { create_vertex_shader(device, &vs_bytes)? };
            let pixel_shader = unsafe { create_pixel_shader(device, &ps_bytes)? };
            let elements = vec![
                InputElementRecord {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: 0,
                    // R32G32B32_FLOAT == 6: the float3 world-position input.
                    format: DXGI_FORMAT_R32G32B32_FLOAT as u32,
                    input_slot: 0,
                    aligned_byte_offset: 0,
                    input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
                    instance_data_step_rate: 0,
                },
                InputElementRecord {
                    semantic_name: "TEXCOORD".to_string(),
                    semantic_index: 0,
                    format: DXGI_FORMAT_R32G32_FLOAT as u32,
                    input_slot: 0,
                    aligned_byte_offset: 12,
                    input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
                    instance_data_step_rate: 0,
                },
            ];
            let input_layout = unsafe { create_input_layout(device, &elements, &vs_bytes)? };
            let vertex_buffer = unsafe {
                create_buffer(device, D3D11_BIND_VERTEX_BUFFER, &mapname_glyph_vertices())?
            };
            let vs_scene_constant_buffer = unsafe {
                create_buffer(
                    device,
                    D3D11_BIND_CONSTANT_BUFFER,
                    &mapname_scene_cb_bytes(),
                )?
            };
            let transp_offset_constant_buffer = unsafe {
                create_buffer(
                    device,
                    D3D11_BIND_CONSTANT_BUFFER,
                    &mapname_transp_offset_cb_bytes(),
                )?
            };
            let font_atlas = unsafe {
                create_texture_srv_with_bytes(device, 8, 8, &mapname_font_atlas_bytes())?
            };
            let hazard_vs_bytes = compile_shader(MAPNAME_HAZARD_VS, "main", "vs_5_0")?;
            let hazard_ps_bytes = compile_shader(MAPNAME_HAZARD_PS, "main", "ps_5_0")?;
            let hazard_vertex_shader = unsafe { create_vertex_shader(device, &hazard_vs_bytes)? };
            let hazard_pixel_shader = unsafe { create_pixel_shader(device, &hazard_ps_bytes)? };
            let hazard_vertex_buffer = unsafe {
                create_buffer(device, D3D11_BIND_VERTEX_BUFFER, &mapname_hazard_vertices())?
            };
            Ok(Self {
                vertex_shader,
                pixel_shader,
                input_layout,
                vertex_buffer,
                vs_scene_constant_buffer,
                transp_offset_constant_buffer,
                font_atlas,
                hazard_vertex_shader,
                hazard_pixel_shader,
                hazard_vertex_buffer,
                vertex_stride: 20,
            })
        }
    }

    struct D3dApp {
        hwnd: HWND,
        render_target: ComPtr<ID3D11RenderTargetView>,
        // Offscreen target for the map phases: the real game renders 3D to
        // intermediate targets and ONLY UI to the backbuffer, so the harness
        // must too or map draws pollute the ui_pass open condition.
        map_target: ComPtr<ID3D11RenderTargetView>,
        _map_target_tex: ComPtr<ID3D11Texture2D>,
        depth_stencil: ComPtr<ID3D11DepthStencilView>,
        _samplers: Vec<ComPtr<ID3D11SamplerState>>,
        sampler_ptrs: Vec<*mut ID3D11SamplerState>,
        swap_chain: ComPtr<IDXGISwapChain>,
        context: ComPtr<ID3D11DeviceContext>,
        device: ComPtr<ID3D11Device>,
    }

    impl D3dApp {
        fn new(debug_device: bool) -> Result<Self> {
            let hwnd = unsafe { create_window()? };
            let mut swap_desc = DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Width: WINDOW_WIDTH,
                    Height: WINDOW_HEIGHT,
                    RefreshRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ScanlineOrdering: 0,
                    Scaling: 0,
                },
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                OutputWindow: hwnd as winapi::shared::windef::HWND,
                Windowed: TRUE,
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                Flags: 0,
            };
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut swap_chain = ptr::null_mut();
            let mut device = ptr::null_mut();
            let mut feature_level = D3D_FEATURE_LEVEL_11_0;
            let mut context = ptr::null_mut();
            let create_flags = if debug_device {
                D3D11_CREATE_DEVICE_DEBUG
            } else {
                0
            };
            let mut hr = unsafe {
                D3D11CreateDeviceAndSwapChain(
                    ptr::null_mut(),
                    D3D_DRIVER_TYPE_HARDWARE,
                    ptr::null_mut(),
                    create_flags,
                    feature_levels.as_ptr(),
                    feature_levels.len() as UINT,
                    D3D11_SDK_VERSION,
                    &mut swap_desc,
                    &mut swap_chain,
                    &mut device,
                    &mut feature_level,
                    &mut context,
                )
            };
            if FAILED(hr) && debug_device {
                eprintln!(
                    "capture_harness D3D11 debug device unavailable: HRESULT 0x{hr:08X}; retrying without debug layer"
                );
                swap_chain = ptr::null_mut();
                device = ptr::null_mut();
                context = ptr::null_mut();
                hr = unsafe {
                    D3D11CreateDeviceAndSwapChain(
                        ptr::null_mut(),
                        D3D_DRIVER_TYPE_HARDWARE,
                        ptr::null_mut(),
                        0,
                        feature_levels.as_ptr(),
                        feature_levels.len() as UINT,
                        D3D11_SDK_VERSION,
                        &mut swap_desc,
                        &mut swap_chain,
                        &mut device,
                        &mut feature_level,
                        &mut context,
                    )
                };
            }
            if FAILED(hr) {
                return Err(
                    format!("D3D11CreateDeviceAndSwapChain failed: HRESULT 0x{hr:08X}").into(),
                );
            }
            let device =
                unsafe { ComPtr::new(device).ok_or("D3D11CreateDevice returned null device")? };
            let context =
                unsafe { ComPtr::new(context).ok_or("D3D11CreateDevice returned null context")? };
            let swap_chain = unsafe {
                ComPtr::new(swap_chain).ok_or("D3D11CreateDevice returned null swap chain")?
            };
            let render_target =
                unsafe { create_backbuffer_render_target(device.as_ptr(), swap_chain.as_ptr())? };
            let (map_target, map_target_tex) =
                unsafe { create_offscreen_map_target(device.as_ptr())? };
            let depth_stencil = unsafe { create_depth_stencil(device.as_ptr())? };
            let samplers = unsafe { create_default_samplers(device.as_ptr())? };
            let sampler_ptrs = samplers.iter().map(ComPtr::as_ptr).collect::<Vec<_>>();
            Ok(Self {
                hwnd,
                render_target,
                map_target,
                _map_target_tex: map_target_tex,
                depth_stencil,
                _samplers: samplers,
                sampler_ptrs,
                swap_chain,
                context,
                device,
            })
        }

        fn pump_messages(&self) {
            unsafe {
                let mut msg = mem::zeroed::<MSG>();
                while PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        fn clear_state_and_flush(&self) {
            unsafe {
                (*self.context.as_ptr()).ClearState();
                (*self.context.as_ptr()).Flush();
            }
        }
    }

    impl Drop for D3dApp {
        fn drop(&mut self) {
            self.clear_state_and_flush();
        }
    }

    unsafe fn load_burst_phase(
        d3d: &D3dApp,
        bundle_root: &Path,
        filler_vs: &[u8],
        filler_ps: &[u8],
        duration_ms: u64,
        filler_shaders_per_frame: u32,
    ) -> Result<()> {
        let shader_files = collect_shader_files(bundle_root)?;
        let started = Instant::now();
        let duration = Duration::from_millis(duration_ms);
        let mut frame = 0u32;
        while started.elapsed() < duration {
            d3d.pump_messages();
            let mut created_vs = Vec::new();
            let mut created_ps = Vec::new();
            for path in &shader_files {
                let bytes = fs::read(path)?;
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if name.contains("_vertex_shader_") || name.starts_with("map_vertex_shader_") {
                    if let Ok(shader) = unsafe { create_vertex_shader(d3d.device.as_ptr(), &bytes) }
                    {
                        created_vs.push(shader);
                    }
                } else if name.contains("_pixel_shader_") {
                    if let Ok(shader) = unsafe { create_pixel_shader(d3d.device.as_ptr(), &bytes) }
                    {
                        created_ps.push(shader);
                    }
                }
            }
            // A batch of NON-family filler shaders per frame exercises the
            // create hooks. Before the 2026-07-02 load-freeze fix each created
            // pixel shader took the global capture lock and ran D3D reflection
            // 5x; the lock-free byte-scan pre-filter now rejects them before
            // any lock (a real session's create_pixel_shader_lock_stats event
            // reports the skip/lock split). ComPtr drops release each shader at
            // end of iteration — no retention, no resource exhaustion.
            for _ in 0..filler_shaders_per_frame {
                drop(unsafe { create_vertex_shader(d3d.device.as_ptr(), filler_vs)? });
                drop(unsafe { create_pixel_shader(d3d.device.as_ptr(), filler_ps)? });
            }
            unsafe {
                clear_and_present(d3d, ptr::null_mut())?;
            }
            frame = frame.saturating_add(1);
            if frame % 30 == 0 {
                println!("capture_harness load-burst frames={frame}");
            }
        }
        Ok(())
    }

    fn collect_shader_files(root: &Path) -> Result<Vec<PathBuf>> {
        let mut files = fs::read_dir(root)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| {
                        name.contains("_pixel_shader_")
                            || name.contains("_vertex_shader_")
                            || name.starts_with("map_vertex_shader_")
                    })
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        files.sort();
        Ok(files)
    }

    /// Post-moment CB0-sampler survival driver. Runs AFTER the terrain + water
    /// moments have completed, on the render thread, re-binding the tracked
    /// terrain pass (terrain PS + scene CB0 at slot 0) every iteration for
    /// `duration_ms` with a short sleep so the DLL's wall-clock sampler crosses
    /// several of its sample windows entirely in post-moment time. Present each
    /// iteration so the window stays responsive.
    unsafe fn cb0_sampler_verify_phase(
        d3d: &D3dApp,
        terrain_pass: &PreparedPass,
        non_terrain_ps: *mut ID3D11PixelShader,
        duration_ms: u64,
    ) -> Result<()> {
        let started = Instant::now();
        let duration = Duration::from_millis(duration_ms);
        let mut frames = 0u64;
        while started.elapsed() < duration {
            d3d.pump_messages();
            unsafe { draw_prepared_pass(d3d, terrain_pass, non_terrain_ps)? };
            frames = frames.saturating_add(1);
            thread::sleep(Duration::from_millis(16));
        }
        println!(
            "capture_harness cb0-sampler-verify frames={} elapsed_ms={}",
            frames,
            started.elapsed().as_millis()
        );
        Ok(())
    }

    unsafe fn draw_prepared_pass(
        d3d: &D3dApp,
        pass: &PreparedPass,
        non_terrain_ps: *mut ID3D11PixelShader,
    ) -> Result<()> {
        for draw in &pass.draws {
            unsafe { bind_and_draw(d3d, draw)? };
        }
        unsafe { clear_and_present(d3d, non_terrain_ps) }
    }

    unsafe fn bind_and_draw(d3d: &D3dApp, draw: &PreparedDraw) -> Result<()> {
        unsafe { bind_and_draw_with_ps(d3d, draw, draw.pixel_shader, &draw.shader_resources) }
    }

    unsafe fn bind_and_draw_with_ps(
        d3d: &D3dApp,
        draw: &PreparedDraw,
        pixel_shader: *mut ID3D11PixelShader,
        shader_resources: &[*mut ID3D11ShaderResourceView],
    ) -> Result<()> {
        let context = d3d.context.as_ptr();
        unsafe {
            bind_frame_targets(d3d);
            (*context).IASetInputLayout(draw.input_layout.as_ptr());
            (*context).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            if !draw.vertex_buffer_ptrs.is_empty() {
                (*context).IASetVertexBuffers(
                    0,
                    draw.vertex_buffer_ptrs.len() as UINT,
                    draw.vertex_buffer_ptrs.as_ptr(),
                    draw.strides.as_ptr(),
                    draw.offsets.as_ptr(),
                );
            }
            if let Some(index_buffer) = &draw.index_buffer {
                (*context).IASetIndexBuffer(
                    index_buffer.as_ptr(),
                    draw.index_format,
                    draw.index_offset,
                );
            }
            (*context).VSSetShader(draw.vertex_shader, ptr::null(), 0);
            (*context).PSSetShader(pixel_shader, ptr::null(), 0);
            bind_constant_buffers(context, &draw.vs_constant_buffers, true);
            bind_constant_buffers(context, &draw.ps_constant_buffers, false);
            if !shader_resources.is_empty() {
                (*context).PSSetShaderResources(
                    0,
                    shader_resources.len() as UINT,
                    shader_resources.as_ptr(),
                );
            }
            bind_default_samplers(d3d);
            if draw.draw_call == "Draw" {
                (*context).Draw(draw.draw_count, draw.draw_start);
            } else {
                (*context).DrawIndexed(draw.draw_count, draw.draw_start, draw.base_vertex_location);
            }
        }
        Ok(())
    }

    unsafe fn draw_restore_pass(
        d3d: &D3dApp,
        restore: &RestoreResources,
        non_terrain_ps: *mut ID3D11PixelShader,
    ) -> Result<()> {
        let context = d3d.context.as_ptr();
        unsafe {
            bind_frame_targets(d3d);
            let blend_factor = [0.0f32, 0.0, 0.0, 0.0];
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
            (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
            (*context).RSSetState(ptr::null_mut());
            (*context).IASetInputLayout(ptr::null_mut());
            (*context).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetIndexBuffer(ptr::null_mut(), DXGI_FORMAT_R16_UINT, 0);
            (*context).VSSetShader(restore.vertex_shader.as_ptr(), ptr::null(), 0);
            (*context).PSSetShader(restore.pixel_shader.as_ptr(), ptr::null(), 0);
            let ps_cb = restore.ps_constant_buffer.as_ptr();
            (*context).PSSetConstantBuffers(1, 1, &ps_cb);
            let srvs = restore
                .textures
                .iter()
                .map(|srv| srv.as_ptr())
                .collect::<Vec<_>>();
            (*context).PSSetShaderResources(0, srvs.len() as UINT, srvs.as_ptr());
            bind_default_samplers(d3d);
            (*context).Draw(3, 0);
            clear_and_present(d3d, non_terrain_ps)?;
        }
        Ok(())
    }

    unsafe fn draw_mapname_pass(
        d3d: &D3dApp,
        mapname: &MapnameResources,
        non_terrain_ps: *mut ID3D11PixelShader,
    ) -> Result<()> {
        let context = d3d.context.as_ptr();
        let offset = 0u32;
        unsafe {
            bind_frame_targets(d3d);
            let blend_factor = [0.0f32, 0.0, 0.0, 0.0];
            (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
            (*context).OMSetDepthStencilState(ptr::null_mut(), 0);
            (*context).RSSetState(ptr::null_mut());
            (*context).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetIndexBuffer(ptr::null_mut(), DXGI_FORMAT_R16_UINT, 0);
            bind_default_samplers(d3d);

            // Hazard draw FIRST: a browser-overlay-shaped float2-position VS
            // that the mapname discriminator MUST reject. It shares the frame
            // but must never land in the bundle.
            let hazard_vb = mapname.hazard_vertex_buffer.as_ptr();
            let hazard_stride = 16u32;
            (*context).IASetInputLayout(ptr::null_mut());
            (*context).IASetVertexBuffers(0, 1, &hazard_vb, &hazard_stride, &offset);
            (*context).VSSetShader(mapname.hazard_vertex_shader.as_ptr(), ptr::null(), 0);
            (*context).PSSetShader(mapname.hazard_pixel_shader.as_ptr(), ptr::null(), 0);
            (*context).Draw(3, 0);

            // The mapname glyph draw: float3-position VS + font atlas + CB1.
            let vb = mapname.vertex_buffer.as_ptr();
            let stride = mapname.vertex_stride;
            (*context).IASetInputLayout(mapname.input_layout.as_ptr());
            (*context).IASetVertexBuffers(0, 1, &vb, &stride, &offset);
            (*context).VSSetShader(mapname.vertex_shader.as_ptr(), ptr::null(), 0);
            (*context).PSSetShader(mapname.pixel_shader.as_ptr(), ptr::null(), 0);
            let scene_cb = mapname.vs_scene_constant_buffer.as_ptr();
            let transp_cb = mapname.transp_offset_constant_buffer.as_ptr();
            (*context).VSSetConstantBuffers(0, 1, &scene_cb);
            (*context).VSSetConstantBuffers(1, 1, &transp_cb);
            (*context).PSSetConstantBuffers(1, 1, &transp_cb);
            let atlas = mapname.font_atlas.as_ptr();
            (*context).PSSetShaderResources(0, 1, &atlas);
            // 6 glyph-quad vertices (two triangles) — full float3 VB payload.
            (*context).Draw(6, 0);
            clear_and_present(d3d, non_terrain_ps)?;
        }
        Ok(())
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    struct HookLockStatsSnapshot {
        create_ps_prefilter_skips: u64,
        create_ps_lock_acquisitions: u64,
        context_prefilter_skips: u64,
        context_lock_acquisitions: u64,
        context_ia_set_input_layout_lock_acquisitions: u64,
        context_vs_set_shader_lock_acquisitions: u64,
        context_ps_set_shader_lock_acquisitions: u64,
        context_ps_set_shader_resources_lock_acquisitions: u64,
        context_draw_lock_acquisitions: u64,
        context_clear_lock_acquisitions: u64,
    }

    impl HookLockStatsSnapshot {
        fn context_locks(self) -> u64 {
            self.context_lock_acquisitions
        }
    }

    /// Simulated load screen for the census trust gate: draws issued with
    /// NON-capture-family (filler) shaders and no TerrainSceneCB0, BEFORE any
    /// map-family shader has ever been bound. With --census armed the DLL must
    /// record these signatures with uncaptured_reason=untrusted_context_class
    /// and issue zero immediate-context queries (the 2026-07-05 live crash was
    /// census payload capture running PSGetConstantBuffers against exactly
    /// this phase of the real game). Disabled by default (frames == 0) so
    /// non-census gates are unchanged.
    unsafe fn census_load_screen_phase(
        d3d: &D3dApp,
        vertex_shader: *mut ID3D11VertexShader,
        pixel_shader: *mut ID3D11PixelShader,
        index_buffer: *mut ID3D11Buffer,
        frames: u32,
    ) -> Result<()> {
        if frames == 0 {
            println!("capture_harness census-load-screen: skipped frames=0");
            return Ok(());
        }
        unsafe {
            bind_frame_targets(d3d);
            (*d3d.context.as_ptr()).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*d3d.context.as_ptr()).IASetIndexBuffer(index_buffer, DXGI_FORMAT_R16_UINT, 0);
            (*d3d.context.as_ptr()).IASetInputLayout(ptr::null_mut());
            (*d3d.context.as_ptr()).VSSetShader(vertex_shader, ptr::null(), 0);
            (*d3d.context.as_ptr()).PSSetShader(pixel_shader, ptr::null(), 0);
        }
        for frame in 0..frames {
            d3d.pump_messages();
            unsafe {
                // Two draw shapes per frame so the census sees more than one
                // pre-trust signature candidate.
                (*d3d.context.as_ptr()).DrawIndexed(3, 0, 0);
                (*d3d.context.as_ptr()).Draw(3, 0);
                clear_and_present(d3d, pixel_shader)?;
            }
            if frame % 8 == 0 {
                println!("capture_harness census-load-screen frame={frame}");
            }
        }
        Ok(())
    }

    unsafe fn hook_overhead_stress_phase(
        label: &'static str,
        d3d: &D3dApp,
        vertex_shader: *mut ID3D11VertexShader,
        pixel_shader: *mut ID3D11PixelShader,
        index_buffer: *mut ID3D11Buffer,
        duration_ms: u64,
        assert_lock_split: bool,
    ) -> Result<()> {
        if duration_ms == 0 {
            println!("capture_harness hook-overhead {label}: skipped duration_ms=0");
            return Ok(());
        }
        unsafe {
            bind_frame_targets(d3d);
            (*d3d.context.as_ptr()).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*d3d.context.as_ptr()).IASetIndexBuffer(index_buffer, DXGI_FORMAT_R16_UINT, 0);
            (*d3d.context.as_ptr()).VSSetShader(vertex_shader, ptr::null(), 0);
        }
        let before = unsafe { read_hook_lock_stats() };
        let started = Instant::now();
        let duration = Duration::from_millis(duration_ms);
        let mut calls = 0u64;
        while started.elapsed() < duration {
            unsafe {
                (*d3d.context.as_ptr()).IASetInputLayout(ptr::null_mut());
                (*d3d.context.as_ptr()).PSSetShader(pixel_shader, ptr::null(), 0);
                (*d3d.context.as_ptr()).DrawIndexed(3, 0, 0);
            }
            calls = calls.saturating_add(3);
        }
        let elapsed = started.elapsed();
        unsafe {
            clear_and_present(d3d, pixel_shader)?;
        }
        let after = unsafe { read_hook_lock_stats() };
        let elapsed_ns = elapsed.as_nanos().max(1);
        let mean_ns = elapsed_ns / u128::from(calls.max(1));
        let calls_per_sec = (u128::from(calls) * 1_000_000_000u128) / elapsed_ns;
        let lock_delta = before
            .zip(after)
            .map(|(before, after)| after.context_locks().saturating_sub(before.context_locks()));
        println!(
            "capture_harness hook-overhead {label}: calls={} elapsed_ms={} calls_per_sec={} mean_wrapper_ns={} context_lock_delta={}",
            calls,
            elapsed.as_millis(),
            calls_per_sec,
            mean_ns,
            lock_delta
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unavailable".to_string())
        );
        if assert_lock_split {
            if let Some(lock_delta) = lock_delta {
                let allowed = 16u64;
                if lock_delta > allowed {
                    return Err(format!(
                        "hook-overhead {label} acquired {lock_delta} context locks for {calls} irrelevant calls (allowed {allowed})"
                    )
                    .into());
                }
            }
            if calls_per_sec < 50_000 {
                return Err(format!(
                    "hook-overhead {label} throughput collapsed: {calls_per_sec} calls/sec"
                )
                .into());
            }
        }
        Ok(())
    }

    type GetHookLockStatsFn = unsafe extern "system" fn(*mut HookLockStatsSnapshot) -> i32;

    unsafe fn read_hook_lock_stats() -> Option<HookLockStatsSnapshot> {
        let module_name = env::var("TERRAIN_CAPTURE_DLL_NAME").ok()?;
        let module = unsafe { GetModuleHandleW(wide_null(&module_name).as_ptr()) };
        if module.is_null() {
            return None;
        }
        let proc = unsafe {
            GetProcAddress(
                module,
                b"Hoi4TerrainCaptureGetHookLockStats\0".as_ptr().cast(),
            )
        }?;
        let get_stats: GetHookLockStatsFn = unsafe { mem::transmute(proc) };
        let mut stats = HookLockStatsSnapshot::default();
        let ok = unsafe { get_stats(&mut stats) };
        (ok != 0).then_some(stats)
    }

    unsafe fn bind_constant_buffers(
        context: *mut ID3D11DeviceContext,
        buffers: &[(u32, ComPtr<ID3D11Buffer>)],
        vertex_stage: bool,
    ) {
        for (slot, buffer) in buffers {
            let ptr = buffer.as_ptr();
            unsafe {
                if vertex_stage {
                    (*context).VSSetConstantBuffers(*slot, 1, &ptr);
                } else {
                    (*context).PSSetConstantBuffers(*slot, 1, &ptr);
                }
            }
        }
    }

    fn multi_thread_phase(
        d3d: &D3dApp,
        terrain_pass: &PreparedPass,
        non_terrain_ps: *mut ID3D11PixelShader,
        duration_ms: u64,
    ) -> Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let device = d3d.device.as_ptr();
        unsafe { add_ref_unknown(device as *mut c_void) };
        let device_addr = device as usize;
        let worker = thread::spawn(move || {
            let device = device_addr as *mut ID3D11Device;
            let mut churned = 0u32;
            while !worker_stop.load(Ordering::SeqCst) {
                unsafe {
                    let _ = churn_device_resources(device, churned);
                }
                churned = churned.wrapping_add(1);
            }
            unsafe { release_unknown(device as *mut c_void) };
            churned
        });
        let started = Instant::now();
        let duration = Duration::from_millis(duration_ms);
        while started.elapsed() < duration {
            unsafe { draw_prepared_pass(d3d, terrain_pass, non_terrain_ps)? };
        }
        stop.store(true, Ordering::SeqCst);
        let churned = worker.join().map_err(|_| "device churn worker panicked")?;
        println!("capture_harness device churn iterations={churned}");
        Ok(())
    }

    unsafe fn river_and_chunk_phase(
        d3d: &D3dApp,
        terrain_pass: &PreparedPass,
        river_ps: *mut ID3D11PixelShader,
        river_resources: &[*mut ID3D11ShaderResourceView],
        river_frames: u32,
        border_ps: *mut ID3D11PixelShader,
        border_resources: &[*mut ID3D11ShaderResourceView],
        border_frames: u32,
        tree_ps: *mut ID3D11PixelShader,
        tree_resources: &[*mut ID3D11ShaderResourceView],
        tree_frames: u32,
        non_terrain_ps: *mut ID3D11PixelShader,
        chunk_sweep_draws: u32,
        capture_output_dir: Option<&Path>,
        live_shape_layers: bool,
        skip_capture_waits: bool,
    ) -> Result<()> {
        let Some(seed_draw) = terrain_pass.draws.first() else {
            return Ok(());
        };
        if live_shape_layers {
            unsafe {
                live_shape_blend_layer_phase(
                    d3d,
                    seed_draw,
                    river_ps,
                    river_resources,
                    river_frames,
                    capture_output_dir,
                    RIVERS_CAPTURE_MOMENT,
                    "map_frame_rivers_pass_bundle.json",
                )?;
                live_shape_blend_layer_phase(
                    d3d,
                    seed_draw,
                    border_ps,
                    border_resources,
                    border_frames,
                    capture_output_dir,
                    BORDERS_CAPTURE_MOMENT,
                    "map_frame_borders_pass_bundle.json",
                )?;
                live_shape_blend_layer_phase(
                    d3d,
                    seed_draw,
                    tree_ps,
                    tree_resources,
                    tree_frames,
                    capture_output_dir,
                    TREES_CAPTURE_MOMENT,
                    "map_frame_trees_pass_bundle.json",
                )?;
            }
        } else {
            for _ in 0..river_frames {
                unsafe {
                    bind_and_draw_with_ps(d3d, seed_draw, river_ps, river_resources)?;
                    clear_and_present(d3d, non_terrain_ps)?;
                }
            }
            if !skip_capture_waits {
                unsafe {
                    wait_for_capture_moment_advanced(
                        d3d,
                        non_terrain_ps,
                        capture_output_dir,
                        RIVERS_CAPTURE_MOMENT,
                    )?;
                }
            }
            for _ in 0..border_frames {
                unsafe {
                    bind_and_draw_with_ps(d3d, seed_draw, border_ps, border_resources)?;
                    clear_and_present(d3d, non_terrain_ps)?;
                }
            }
            if !skip_capture_waits {
                unsafe {
                    wait_for_capture_moment_advanced(
                        d3d,
                        non_terrain_ps,
                        capture_output_dir,
                        BORDERS_CAPTURE_MOMENT,
                    )?;
                }
            }
            for _ in 0..tree_frames {
                unsafe {
                    bind_and_draw_with_ps(d3d, seed_draw, tree_ps, tree_resources)?;
                    clear_and_present(d3d, non_terrain_ps)?;
                }
            }
            if !skip_capture_waits {
                unsafe {
                    wait_for_map_frame_bundle_sequence_complete(
                        d3d,
                        non_terrain_ps,
                        capture_output_dir,
                        "map_frame_trees_pass_bundle.json",
                    )?;
                }
            }
        }
        for index in 0..chunk_sweep_draws {
            unsafe {
                bind_chunk_variant_and_draw(d3d, seed_draw, index)?;
                clear_and_present(d3d, non_terrain_ps)?;
            }
        }
        unsafe { clear_and_present(d3d, non_terrain_ps)? };
        Ok(())
    }

    unsafe fn live_shape_blend_layer_phase(
        d3d: &D3dApp,
        draw: &PreparedDraw,
        pixel_shader: *mut ID3D11PixelShader,
        shader_resources: &[*mut ID3D11ShaderResourceView],
        frames: u32,
        capture_output_dir: Option<&Path>,
        moment: &str,
        bundle_file: &str,
    ) -> Result<()> {
        let Some(capture_output_dir) = capture_output_dir else {
            for _ in 0..frames {
                unsafe { bind_and_draw_with_ps(d3d, draw, pixel_shader, shader_resources)? };
            }
            return Ok(());
        };
        let events_path = capture_output_dir.join("terrain_capture_events.jsonl");
        for frame in 0..frames.max(1) {
            unsafe { bind_and_draw_with_ps(d3d, draw, pixel_shader, shader_resources)? };
            d3d.pump_messages();
            if live_shape_bundle_sequence_complete(&events_path, bundle_file) {
                assert_live_shape_bundle_bounded(&events_path, bundle_file, frame + 1)?;
                return Ok(());
            }
            if let Some(message) = live_shape_budget_exceeded(&events_path, moment) {
                return Err(format!(
                    "live-shape {moment} reproduced non-closing bundle before {bundle_file} sequence completion: {message}"
                )
                .into());
            }
            thread::sleep(Duration::from_millis(4));
        }
        Err(format!(
            "live-shape {moment} did not complete {bundle_file} within {frames} frames; checked {}",
            events_path.display()
        )
        .into())
    }

    fn live_shape_bundle_sequence_complete(events_path: &Path, bundle_file: &str) -> bool {
        let needle = format!("\"file\":\"{}\",\"sequence_complete\":true", bundle_file);
        fs::read_to_string(events_path)
            .map(|events| events.contains(&needle))
            .unwrap_or(false)
    }

    fn live_shape_budget_exceeded(events_path: &Path, moment: &str) -> Option<String> {
        fs::read_to_string(events_path).ok().and_then(|events| {
            events
                .lines()
                .rev()
                .find(|line| {
                    line.contains("\"type\":\"capture_budget_exceeded\"")
                        && line.contains(&format!("\"moment\":\"{moment}\""))
                })
                .map(ToOwned::to_owned)
        })
    }

    fn assert_live_shape_bundle_bounded(
        events_path: &Path,
        bundle_file: &str,
        completed_within_frames: u32,
    ) -> Result<()> {
        let events = fs::read_to_string(events_path)?;
        let bundle_events = events
            .lines()
            .filter(|line| line.contains(&format!("\"file\":\"{bundle_file}\"")))
            .collect::<Vec<_>>();
        let complete_count = bundle_events
            .iter()
            .filter(|line| line.contains("\"sequence_complete\":true"))
            .count();
        let incomplete_count = bundle_events
            .iter()
            .filter(|line| line.contains("\"sequence_complete\":false"))
            .count();
        if complete_count != 1 || incomplete_count > completed_within_frames as usize {
            return Err(format!(
                "live-shape {bundle_file} was not bounded: complete_count={complete_count} incomplete_count={incomplete_count} completed_within_frames={completed_within_frames}"
            )
            .into());
        }
        Ok(())
    }

    unsafe fn wait_for_capture_moment_advanced(
        d3d: &D3dApp,
        non_terrain_ps: *mut ID3D11PixelShader,
        capture_output_dir: Option<&Path>,
        completed_moment: &str,
    ) -> Result<()> {
        let needle = format!(
            "\"type\":\"map_capture_moment_advanced\",\"completed\":\"{}\"",
            completed_moment
        );
        unsafe {
            wait_for_capture_event(
                d3d,
                non_terrain_ps,
                capture_output_dir,
                &needle,
                &format!("capture moment {completed_moment} to advance"),
            )
        }
    }

    unsafe fn wait_for_map_frame_bundle_sequence_complete(
        d3d: &D3dApp,
        non_terrain_ps: *mut ID3D11PixelShader,
        capture_output_dir: Option<&Path>,
        bundle_file: &str,
    ) -> Result<()> {
        let needle = format!("\"file\":\"{}\",\"sequence_complete\":true", bundle_file);
        unsafe {
            wait_for_capture_event(
                d3d,
                non_terrain_ps,
                capture_output_dir,
                &needle,
                &format!("{bundle_file} sequence completion"),
            )
        }
    }

    unsafe fn wait_for_capture_event(
        d3d: &D3dApp,
        non_terrain_ps: *mut ID3D11PixelShader,
        capture_output_dir: Option<&Path>,
        needle: &str,
        description: &str,
    ) -> Result<()> {
        let Some(capture_output_dir) = capture_output_dir else {
            return Ok(());
        };
        let events_path = capture_output_dir.join("terrain_capture_events.jsonl");
        let started = Instant::now();
        let timeout = Duration::from_millis(MOMENT_ADVANCE_TIMEOUT_MS);
        while started.elapsed() < timeout {
            if fs::read_to_string(&events_path)
                .map(|events| events.contains(&needle))
                .unwrap_or(false)
            {
                return Ok(());
            }
            d3d.pump_messages();
            unsafe { clear_and_present(d3d, non_terrain_ps)? };
            thread::sleep(Duration::from_millis(16));
        }
        Err(format!(
            "timed out waiting for {description}; checked {}",
            events_path.display()
        )
        .into())
    }

    unsafe fn bind_chunk_variant_and_draw(
        d3d: &D3dApp,
        draw: &PreparedDraw,
        index: u32,
    ) -> Result<()> {
        let Some(mut cb1) = draw.cb1_template.clone() else {
            unsafe { bind_and_draw(d3d, draw)? };
            return Ok(());
        };
        if cb1.len() >= 80 {
            let columns = 11;
            write_f32_le(&mut cb1, 64, (index % columns) as f32 * 512.0);
            write_f32_le(&mut cb1, 68, (index / columns) as f32 * 512.0);
            write_f32_le(&mut cb1, 72, 8.0);
            write_f32_le(&mut cb1, 76, 0.0);
        }
        pad_constant_buffer(&mut cb1);
        let variant =
            unsafe { create_buffer(d3d.device.as_ptr(), D3D11_BIND_CONSTANT_BUFFER, &cb1)? };
        unsafe {
            bind_frame_targets(d3d);
            (*d3d.context.as_ptr()).IASetInputLayout(draw.input_layout.as_ptr());
            (*d3d.context.as_ptr()).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*d3d.context.as_ptr()).IASetVertexBuffers(
                0,
                draw.vertex_buffer_ptrs.len() as UINT,
                draw.vertex_buffer_ptrs.as_ptr(),
                draw.strides.as_ptr(),
                draw.offsets.as_ptr(),
            );
            if let Some(index_buffer) = &draw.index_buffer {
                (*d3d.context.as_ptr()).IASetIndexBuffer(
                    index_buffer.as_ptr(),
                    draw.index_format,
                    draw.index_offset,
                );
            }
            (*d3d.context.as_ptr()).VSSetShader(draw.vertex_shader, ptr::null(), 0);
            (*d3d.context.as_ptr()).PSSetShader(draw.pixel_shader, ptr::null(), 0);
            bind_constant_buffers(d3d.context.as_ptr(), &draw.vs_constant_buffers, true);
            let variant_ptr = variant.as_ptr();
            (*d3d.context.as_ptr()).VSSetConstantBuffers(1, 1, &variant_ptr);
            bind_constant_buffers(d3d.context.as_ptr(), &draw.ps_constant_buffers, false);
            if !draw.shader_resources.is_empty() {
                (*d3d.context.as_ptr()).PSSetShaderResources(
                    0,
                    draw.shader_resources.len() as UINT,
                    draw.shader_resources.as_ptr(),
                );
            }
            bind_default_samplers(d3d);
            (*d3d.context.as_ptr()).DrawIndexed(
                draw.draw_count,
                draw.draw_start,
                draw.base_vertex_location,
            );
        }
        Ok(())
    }

    unsafe fn clear_and_present(d3d: &D3dApp, pixel_shader: *mut ID3D11PixelShader) -> Result<()> {
        unsafe {
            bind_frame_targets(d3d);
            if !pixel_shader.is_null() {
                (*d3d.context.as_ptr()).PSSetShader(pixel_shader, ptr::null(), 0);
            }
            let color = [0.02, 0.04, 0.06, 1.0];
            (*d3d.context.as_ptr()).ClearRenderTargetView(d3d.map_target.as_ptr(), &color);
            (*d3d.context.as_ptr()).ClearRenderTargetView(d3d.render_target.as_ptr(), &color);
            (*d3d.context.as_ptr()).ClearDepthStencilView(
                d3d.depth_stencil.as_ptr(),
                D3D11_CLEAR_DEPTH | D3D11_CLEAR_STENCIL,
                1.0,
                0,
            );
            let hr = (*d3d.swap_chain.as_ptr()).Present(0, 0);
            if FAILED(hr) {
                return Err(format!("IDXGISwapChain::Present failed: HRESULT 0x{hr:08X}").into());
            }
        }
        Ok(())
    }

    unsafe fn bind_frame_targets(d3d: &D3dApp) {
        let context = d3d.context.as_ptr();
        let rtv = d3d.map_target.as_ptr();
        unsafe {
            (*context).OMSetRenderTargets(1, &rtv, d3d.depth_stencil.as_ptr());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: WINDOW_WIDTH as f32,
                Height: WINDOW_HEIGHT as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
        }
    }

    unsafe fn bind_default_samplers(d3d: &D3dApp) {
        if !d3d.sampler_ptrs.is_empty() {
            unsafe {
                (*d3d.context.as_ptr()).PSSetSamplers(
                    0,
                    d3d.sampler_ptrs.len() as UINT,
                    d3d.sampler_ptrs.as_ptr(),
                );
            }
        }
    }

    unsafe fn ui_phase(
        d3d: &D3dApp,
        ui: &UiResources,
        foreign_ui: &ForeignUiResources,
        frames: u32,
        draws_per_frame: u32,
        capture_output_dir: Option<&Path>,
        ui_cb_skip: UiCbSkipMode,
        skip_capture_waits: bool,
    ) -> Result<()> {
        let frames = frames.max(UI_ATLAS_MIN_FRAMES);
        let draws_per_frame = draws_per_frame.max(1);
        for frame in 0..frames {
            unsafe {
                draw_ui_frame(
                    d3d,
                    ui,
                    foreign_ui,
                    frame,
                    draws_per_frame,
                    ui_state_for_frame(frame),
                )?
            };
            thread::sleep(Duration::from_millis(8));
        }
        if let Some(output_dir) = capture_output_dir.filter(|_| !skip_capture_waits) {
            assert_ui_pass_atlas(output_dir, frames, draws_per_frame, foreign_ui, ui_cb_skip)?;
        }
        Ok(())
    }

    unsafe fn ui_only_phase(
        d3d: &D3dApp,
        ui: &UiResources,
        foreign_ui: &ForeignUiResources,
        frames: u32,
        draws_per_frame: u32,
        capture_output_dir: Option<&Path>,
        ui_cb_skip: UiCbSkipMode,
        skip_capture_waits: bool,
    ) -> Result<()> {
        let draws_per_frame = draws_per_frame.max(UI_HAZARD_DRAW_INDEX + 1);
        let control_path = env::var_os("TERRAIN_CAPTURE_CONTROL_PATH").map(PathBuf::from);
        if let (Some(output_dir), Some(control_path)) = (capture_output_dir, control_path) {
            let labels = ["ledger", "ledger_repeat"];
            for (label_index, label) in labels.iter().enumerate() {
                fs::write(
                    &control_path,
                    format!(r#"{{"type":"ui_capture_state_now","label":"{label}"}}"#),
                )
                .map_err(|error| {
                    format!(
                        "failed to write ui_capture_state_now command {}: {error}",
                        control_path.display()
                    )
                })?;
                thread::sleep(Duration::from_millis(50));
                for frame in 0..UI_LABEL_COMMAND_CAPTURE_FRAMES {
                    unsafe {
                        draw_ui_frame(
                            d3d,
                            ui,
                            foreign_ui,
                            frame + (label_index as u32 * UI_LABEL_COMMAND_CAPTURE_FRAMES),
                            draws_per_frame,
                            UiHarnessState::A,
                        )?
                    };
                    thread::sleep(Duration::from_millis(8));
                }
            }
            if !skip_capture_waits {
                assert_ui_pass_labelled_commands(output_dir, &labels, draws_per_frame, ui_cb_skip)?;
            }
            return Ok(());
        }

        let frames = frames.max(UI_ATLAS_MIN_FRAMES);
        for frame in 0..frames {
            unsafe {
                draw_ui_frame(
                    d3d,
                    ui,
                    foreign_ui,
                    frame,
                    draws_per_frame,
                    ui_state_for_frame(frame),
                )?
            };
            thread::sleep(Duration::from_millis(8));
        }
        Ok(())
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum UiHarnessState {
        A,
        B,
        C,
    }

    fn ui_state_for_frame(frame: u32) -> UiHarnessState {
        match frame {
            0 | 1 => UiHarnessState::A,
            2..=6 => UiHarnessState::B,
            7..=11 => UiHarnessState::C,
            _ => UiHarnessState::A,
        }
    }

    fn ui_state_extra_draws(base_draws: u32) -> u32 {
        (base_draws / 6).max(2)
    }

    fn ui_state_c_draws(base_draws: u32) -> u32 {
        (base_draws / 2).max(1)
    }

    fn ui_state_draw_count(state: UiHarnessState, base_draws: u32) -> u32 {
        match state {
            UiHarnessState::A => base_draws,
            UiHarnessState::B => base_draws.saturating_add(ui_state_extra_draws(base_draws)),
            UiHarnessState::C => ui_state_c_draws(base_draws),
        }
        .max(1)
    }

    unsafe fn draw_ui_frame(
        d3d: &D3dApp,
        ui: &UiResources,
        foreign_ui: &ForeignUiResources,
        frame: u32,
        base_draws_per_frame: u32,
        state: UiHarnessState,
    ) -> Result<()> {
        let context = d3d.context.as_ptr();
        let draws_per_frame = ui_state_draw_count(state, base_draws_per_frame);
        let rtv = d3d.render_target.as_ptr();
        unsafe {
            (*context).OMSetRenderTargets(1, &rtv, ptr::null_mut());
            (*context).OMSetDepthStencilState(ui.depth_disabled_state.as_ptr(), 0);
            let blend_factor = [0.0f32, 0.0, 0.0, 0.0];
            (*context).OMSetBlendState(ui.blend_state.as_ptr(), &blend_factor, u32::MAX);
            (*context).RSSetState(ui.rasterizer_state.as_ptr());
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: WINDOW_WIDTH as f32,
                Height: WINDOW_HEIGHT as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).RSSetViewports(1, &viewport);
            let color = [0.0, 0.0, 0.0, 1.0];
            (*context).ClearRenderTargetView(rtv, &color);
            (*context).IASetInputLayout(ui.input_layout.as_ptr());
            (*context).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetIndexBuffer(ui.index_buffer.as_ptr(), DXGI_FORMAT_R16_UINT, 0);
            (*context).VSSetShader(ui.vertex_shader.as_ptr(), ptr::null(), 0);
            (*context).PSSetShader(ui.pixel_shader.as_ptr(), ptr::null(), 0);
            let vs_cb = ui.vs_constant_buffer.as_ptr();
            let ps_cb = ui.ps_constant_buffer.as_ptr();
            (*context).VSSetConstantBuffers(0, 1, &vs_cb);
            (*context).PSSetConstantBuffers(0, 1, &ps_cb);
            bind_default_samplers(d3d);
        }
        let mut frame_buffers = Vec::new();
        let null_srvs = vec![ptr::null_mut(); 16];
        unsafe {
            (*context).PSSetShaderResources(0, null_srvs.len() as UINT, null_srvs.as_ptr());
        }
        for draw in 0..draws_per_frame {
            let vertices = ui_quad_vertices(frame, draw, draws_per_frame);
            let vertex_buffer =
                unsafe { create_buffer(d3d.device.as_ptr(), D3D11_BIND_VERTEX_BUFFER, &vertices)? };
            let vertex_ptr = vertex_buffer.as_ptr();
            frame_buffers.push(vertex_buffer);
            let stride = 16u32;
            let offset = 0u32;
            let texture_index = match state {
                UiHarnessState::B if draw >= base_draws_per_frame => 1usize,
                _ => 0usize,
            };
            let texture = ui.textures[texture_index % ui.textures.len()].as_ptr();
            let scissor = ui_scissor_rect(state, draw);
            unsafe {
                (*context).IASetVertexBuffers(0, 1, &vertex_ptr, &stride, &offset);
                (*context).PSSetShaderResources(0, 1, &texture);
                (*context).RSSetScissorRects(1, &scissor);
                if draw == UI_HAZARD_DRAW_INDEX {
                    (*context).IASetIndexBuffer(ptr::null_mut(), DXGI_FORMAT_R16_UINT, 0);
                    (*context).Draw(4, 0);
                    (*context).IASetIndexBuffer(ui.index_buffer.as_ptr(), DXGI_FORMAT_R16_UINT, 0);
                } else {
                    (*context).DrawIndexed(6, 0, 0);
                }
            }
        }
        unsafe { draw_foreign_ui_overlay(d3d, ui, foreign_ui)? };
        unsafe {
            let hr = (*d3d.swap_chain.as_ptr()).Present(0, 0);
            if FAILED(hr) {
                return Err(format!("ui_pass Present failed: HRESULT 0x{hr:08X}").into());
            }
        }
        Ok(())
    }

    unsafe fn draw_foreign_ui_overlay(
        d3d: &D3dApp,
        ui: &UiResources,
        foreign_ui: &ForeignUiResources,
    ) -> Result<()> {
        let context = d3d.context.as_ptr();
        let stride = 16u32;
        let offset = 0u32;
        let vertex_buffer = foreign_ui.vertex_buffer.as_ptr();
        let texture = foreign_ui.texture.as_ptr();
        let vs_cb = foreign_ui.vs_constant_buffer.as_ptr();
        let ps_cb = foreign_ui.ps_constant_buffer.as_ptr();
        let scissor = D3D11_RECT {
            left: 0,
            top: 0,
            right: WINDOW_WIDTH as i32,
            bottom: WINDOW_HEIGHT as i32,
        };
        unsafe {
            (*context).IASetInputLayout(ui.input_layout.as_ptr());
            (*context).IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetIndexBuffer(ui.index_buffer.as_ptr(), DXGI_FORMAT_R16_UINT, 0);
            (*context).IASetVertexBuffers(0, 1, &vertex_buffer, &stride, &offset);
            (*context).VSSetShader(foreign_ui.vertex_shader.as_ptr(), ptr::null(), 0);
            // NULL pixel shader: a legal (depth-only style) pipeline whose
            // shader state the capture DLL cannot resolve, so the draw is
            // classified foreign/unknown and skipped. Never bind objects
            // created on another device here - that is D3D11 UB and it
            // corrupts the driver (see ForeignUiResources doc comment).
            (*context).PSSetShader(ptr::null_mut(), ptr::null(), 0);
            (*context).VSSetConstantBuffers(0, 1, &vs_cb);
            (*context).PSSetConstantBuffers(0, 1, &ps_cb);
            (*context).PSSetShaderResources(0, 1, &texture);
            (*context).RSSetScissorRects(1, &scissor);
            (*context).DrawIndexed(6, 0, 0);
        }
        Ok(())
    }

    fn ui_quad_vertices(frame: u32, draw: u32, draws_per_frame: u32) -> Vec<u8> {
        let columns = 6u32;
        let rows = ((draws_per_frame + columns - 1) / columns).max(1);
        let column = draw % columns;
        let row = draw / columns;
        let cell_w = 1.72f32 / columns as f32;
        let cell_h = 1.36f32 / rows as f32;
        let x0 = -0.86 + column as f32 * cell_w + 0.01 * frame as f32;
        let y0 = -0.72 + row as f32 * cell_h;
        let x1 = x0 + cell_w * 0.72;
        let y1 = y0 + cell_h * 0.64;
        let vertices = [
            (x0, y0, 0.0f32, 0.0f32),
            (x1, y0, 1.0f32, 0.0f32),
            (x1, y1, 1.0f32, 1.0f32),
            (x0, y1, 0.0f32, 1.0f32),
        ];
        let mut bytes = Vec::with_capacity(vertices.len() * 16);
        for (x, y, u, v) in vertices {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
            bytes.extend_from_slice(&u.to_le_bytes());
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    fn foreign_ui_quad_vertices() -> Vec<u8> {
        let vertices = [
            (-0.97f32, -0.95f32, 0.0f32, 0.0f32),
            (-0.71f32, -0.95f32, 1.0f32, 0.0f32),
            (-0.71f32, -0.69f32, 1.0f32, 1.0f32),
            (-0.97f32, -0.69f32, 0.0f32, 1.0f32),
        ];
        let mut bytes = Vec::with_capacity(vertices.len() * 16);
        for (x, y, u, v) in vertices {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
            bytes.extend_from_slice(&u.to_le_bytes());
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    fn ui_scissor_rect(state: UiHarnessState, draw: u32) -> D3D11_RECT {
        let inset = match state {
            UiHarnessState::C => 12 + ((draw % 4) as i32 * 3),
            _ => (draw % 5) as i32,
        };
        D3D11_RECT {
            left: inset,
            top: if state == UiHarnessState::C {
                inset + 2
            } else {
                inset
            },
            right: WINDOW_WIDTH as i32 - inset,
            bottom: WINDOW_HEIGHT as i32 - inset,
        }
    }

    fn assert_ui_pass_atlas(
        output_dir: &Path,
        frames: u32,
        draws_per_frame: u32,
        foreign_ui: &ForeignUiResources,
        ui_cb_skip: UiCbSkipMode,
    ) -> Result<()> {
        let bundle_paths = [
            output_dir.join("ui_pass_bundle_000.json"),
            output_dir.join("ui_pass_bundle_001.json"),
            output_dir.join("ui_pass_bundle_002.json"),
        ];
        let events_path = output_dir.join("terrain_capture_events.jsonl");
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(MOMENT_ADVANCE_TIMEOUT_MS) {
            if bundle_paths.iter().all(|path| path.is_file()) {
                break;
            }
            thread::sleep(Duration::from_millis(16));
        }
        let bundle_count = fs::read_dir(output_dir)?
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("ui_pass_bundle_")
                    && entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
            })
            .count();
        if bundle_count != 3 {
            return Err(
                format!("ui_pass expected exactly 3 atlas bundles, got {bundle_count}").into(),
            );
        }
        let expected_states = [
            (UiHarnessState::A, 1, draws_per_frame),
            (
                UiHarnessState::B,
                UI_STATE_B_CAPTURE_FRAME,
                ui_state_draw_count(UiHarnessState::B, draws_per_frame),
            ),
            (
                UiHarnessState::C,
                UI_STATE_C_CAPTURE_FRAME,
                ui_state_draw_count(UiHarnessState::C, draws_per_frame),
            ),
        ];
        let mut bundles = Vec::new();
        for (index, path) in bundle_paths.iter().enumerate() {
            let bytes = fs::read(path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let draws = value
                .get("draws")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("{} has no draws array", path.display()))?;
            let expected_draws = expected_states[index].2 as usize;
            if draws.len() != expected_draws {
                return Err(format!(
                    "ui_pass atlas bundle {index} draw count mismatch: expected {expected_draws}, got {}",
                    draws.len()
                )
                .into());
            }
            bundles.push(value);
        }
        let mut texture_payloads = BTreeSet::new();
        let mut signatures = BTreeSet::new();
        let mut vertex_matches = Vec::new();
        let mut foreign_payload_absent = true;
        let mut evidence = UiEvidenceStats::default();
        // The foreign draw binds a NULL pixel shader and a dedicated vertex
        // shader; neither may ever appear in a captured bundle draw.
        let foreign_ps = "0x0".to_string();
        let foreign_vs = format!("0x{:X}", foreign_ui.vertex_shader.as_ptr() as usize);
        for (index, bundle) in bundles.iter().enumerate() {
            let signature = bundle
                .get("signature")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("ui_pass bundle {index} has no signature"))?;
            signatures.insert(signature.to_string());
            let draws = bundle
                .get("draws")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("ui_pass bundle {index} has no draws array"))?;
            for draw in draws {
                let ps_pointer = draw
                    .get("shader")
                    .and_then(|value| value.get("pointer"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let vs_pointer = draw
                    .get("vertex_shader")
                    .and_then(|value| value.get("pointer"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if ps_pointer == foreign_ps || vs_pointer == foreign_vs {
                    return Err("ui_pass captured a foreign-shader draw in the bundle".into());
                }
                assert_ui_draw_evidence(draw, ui_cb_skip, &mut evidence, &mut texture_payloads)?;
            }
            let first_draw = draws
                .first()
                .ok_or_else(|| format!("ui_pass bundle {index} has no first draw"))?;
            let vb_file = first_draw
                .get("vertex_buffers")
                .and_then(|value| value.get("0"))
                .and_then(|value| value.get("file"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    format!("ui_pass bundle {index} first draw has no slot 0 vertex buffer file")
                })?;
            let vb_bytes = fs::read(output_dir.join(vb_file))?;
            if vb_bytes == foreign_ui.vertex_bytes {
                foreign_payload_absent = false;
            }
            let (_, expected_frame, expected_draws) = expected_states[index];
            let expected = ui_quad_vertices(expected_frame, 0, expected_draws);
            let first_vertex_ok = vb_bytes.get(0..16) == expected.get(0..16);
            let second_vertex_ok = vb_bytes.get(16..32) == expected.get(16..32);
            vertex_matches.push(first_vertex_ok && second_vertex_ok);
        }
        if signatures.len() != 3 {
            return Err(format!(
                "ui_pass expected 3 distinct signatures, got {}: {:?}",
                signatures.len(),
                signatures
            )
            .into());
        }
        let expected_texture_payloads = if ui_cb_skip == UiCbSkipMode::All {
            0
        } else {
            2
        };
        if texture_payloads.len() != expected_texture_payloads {
            return Err(format!(
                "ui_pass atlas expected {expected_texture_payloads} unique texture payloads, got {}: {:?}",
                texture_payloads.len(),
                texture_payloads
            )
            .into());
        }
        if vertex_matches.iter().any(|matched| !*matched) {
            return Err(format!(
                "ui_pass atlas vertex byte spot checks failed: {:?}",
                vertex_matches
            )
            .into());
        }
        if !foreign_payload_absent {
            return Err("ui_pass captured the foreign draw vertex payload".into());
        }
        evidence.assert_policy(ui_cb_skip)?;
        let events = fs::read_to_string(&events_path).unwrap_or_default();
        let foreign_skip_events = events
            .lines()
            .filter(|line| line.contains("\"type\":\"ui_pass_draw_skipped_foreign_shader\""))
            .count();
        if foreign_skip_events < 3 {
            return Err(format!(
                "ui_pass expected at least 3 skipped foreign shader events, got {foreign_skip_events}"
            )
            .into());
        }
        let captured_draw_events = events
            .lines()
            .filter(|line| line.contains("\"type\":\"ui_pass_draw_captured\""))
            .count();
        let expected_captured_draw_events = expected_states
            .iter()
            .map(|(_, _, draws)| *draws as usize)
            .sum::<usize>();
        if captured_draw_events != expected_captured_draw_events {
            return Err(format!(
                "ui_pass expected {expected_captured_draw_events} captured draw events across 3 states, got {captured_draw_events}"
            )
            .into());
        }
        let state_events = events
            .lines()
            .filter(|line| line.contains("\"type\":\"ui_pass_state_captured\""))
            .count();
        if state_events != 3 {
            return Err(
                format!("ui_pass expected 3 state captured events, got {state_events}").into(),
            );
        }
        let exhausted_events = events
            .lines()
            .filter(|line| line.contains("\"type\":\"ui_pass_states_exhausted\""))
            .count();
        if exhausted_events != 1 {
            return Err(format!(
                "ui_pass expected exactly one states exhausted event, got {exhausted_events}"
            )
            .into());
        }
        let complete_events = events
            .lines()
            .filter(|line| {
                line.contains(&format!(
                    "\"type\":\"{UI_PASS_CAPTURE_MOMENT}_bundle_captured\""
                )) && line.contains("\"sequence_complete\":true")
            })
            .count();
        if complete_events != 1 {
            return Err(format!(
                "ui_pass expected exactly one complete event, got {complete_events}"
            )
            .into());
        }
        let draw_counts = expected_states
            .iter()
            .map(|(_, _, draws)| draws.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let vertex_match_text = vertex_matches
            .iter()
            .map(|matched| matched.to_string())
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "capture_harness ui-pass gate: frames={} state_bundles={} draw_counts=[{}] captured_draw_events={} skipped_foreign_shader_events={} foreign_resources_absent={} ui_cb_skip={} hazard_draws={} skipped_cb_draws={} skipped_srv_draws={} normal_cb_draws={} state_events={} signatures_distinct={} unique_texture_payloads={} vertex_byte_exact=[{}] repeats_ignored={} exhausted_events={} complete_events={}",
            frames,
            bundle_count,
            draw_counts,
            captured_draw_events,
            foreign_skip_events,
            foreign_payload_absent,
            ui_cb_skip.as_str(),
            evidence.hazard_draws,
            evidence.cb_unobtained_draws,
            evidence.srv_unobtained_draws,
            evidence.normal_cb_draws,
            state_events,
            signatures.len() == 3,
            texture_payloads.len(),
            vertex_match_text,
            bundle_count == 3 && state_events == 3,
            exhausted_events,
            complete_events
        );
        Ok(())
    }

    fn assert_ui_pass_labelled_commands(
        output_dir: &Path,
        expected_labels: &[&str],
        draws_per_frame: u32,
        ui_cb_skip: UiCbSkipMode,
    ) -> Result<()> {
        let events_path = output_dir.join("terrain_capture_events.jsonl");
        let started = Instant::now();
        let mut labelled_bundles = Vec::new();
        while started.elapsed() < Duration::from_millis(MOMENT_ADVANCE_TIMEOUT_MS) {
            labelled_bundles = read_labelled_ui_bundles(output_dir)?;
            if expected_labels.iter().all(|label| {
                labelled_bundles
                    .iter()
                    .any(|(_, value)| bundle_label(value) == Some(*label))
            }) {
                break;
            }
            thread::sleep(Duration::from_millis(16));
        }
        if !expected_labels.iter().all(|label| {
            labelled_bundles
                .iter()
                .any(|(_, value)| bundle_label(value) == Some(*label))
        }) {
            let labels = labelled_bundles
                .iter()
                .filter_map(|(_, value)| bundle_label(value).map(ToOwned::to_owned))
                .collect::<Vec<_>>();
            return Err(format!(
                "ui_pass labelled command gate expected labels {:?}, got {:?}",
                expected_labels, labels
            )
            .into());
        }

        let mut signatures = BTreeSet::new();
        let mut texture_payloads = BTreeSet::new();
        let mut evidence = UiEvidenceStats::default();
        for (path, bundle) in &labelled_bundles {
            let Some(label) = bundle_label(bundle) else {
                continue;
            };
            if !expected_labels.contains(&label) {
                continue;
            }
            let signature = bundle
                .get("signature")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("{} has no signature", path.display()))?;
            signatures.insert(signature.to_string());
            let draws = bundle
                .get("draws")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("{} has no draws array", path.display()))?;
            if draws.len() != draws_per_frame as usize {
                return Err(format!(
                    "{} labelled {label} draw count mismatch: expected {}, got {}",
                    path.display(),
                    draws_per_frame,
                    draws.len()
                )
                .into());
            }
            for draw in draws {
                assert_ui_draw_evidence(draw, ui_cb_skip, &mut evidence, &mut texture_payloads)?;
            }
        }
        if signatures.len() != 1 {
            return Err(format!(
                "ui_pass labelled command gate expected one repeated UI signature, got {:?}",
                signatures
            )
            .into());
        }
        evidence.assert_policy(ui_cb_skip)?;

        let events = fs::read_to_string(&events_path).unwrap_or_default();
        for label in expected_labels {
            let captured_line = events.lines().any(|line| {
                line.contains("\"type\":\"ui_pass_state_captured\"")
                    && line.contains(&format!("\"label\":\"{label}\""))
                    && line.contains("\"signature\":")
                    && line.contains("\"draw_count\":")
                    && line.contains("\"bundle_path\":")
            });
            if !captured_line {
                return Err(format!(
                    "ui_pass labelled command gate missing captured event for {label}"
                )
                .into());
            }
        }
        println!(
            "capture_harness ui-pass labelled gate: labels={:?} signatures={} ui_cb_skip={} hazard_draws={} skipped_cb_draws={} skipped_srv_draws={} normal_cb_draws={}",
            expected_labels,
            signatures.len(),
            ui_cb_skip.as_str(),
            evidence.hazard_draws,
            evidence.cb_unobtained_draws,
            evidence.srv_unobtained_draws,
            evidence.normal_cb_draws
        );
        Ok(())
    }

    fn read_labelled_ui_bundles(output_dir: &Path) -> Result<Vec<(PathBuf, serde_json::Value)>> {
        let mut paths = fs::read_dir(output_dir)?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("ui_pass_bundle_"))
                    && path.extension().and_then(|ext| ext.to_str()) == Some("json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut bundles = Vec::new();
        for path in paths {
            let value = serde_json::from_slice(&fs::read(&path)?)?;
            if bundle_label(&value).is_some() {
                bundles.push((path, value));
            }
        }
        Ok(bundles)
    }

    fn bundle_label(value: &serde_json::Value) -> Option<&str> {
        value.get("label").and_then(serde_json::Value::as_str)
    }

    #[derive(Default)]
    struct UiEvidenceStats {
        hazard_draws: usize,
        cb_unobtained_draws: usize,
        srv_unobtained_draws: usize,
        normal_cb_draws: usize,
        normal_srv_draws: usize,
    }

    impl UiEvidenceStats {
        fn assert_policy(&self, ui_cb_skip: UiCbSkipMode) -> Result<()> {
            if self.hazard_draws == 0 {
                return Err(
                    "ui_pass fixture did not include the 4-vertex non-indexed hazard draw".into(),
                );
            }
            match ui_cb_skip {
                UiCbSkipMode::Hazard => {
                    if self.cb_unobtained_draws < self.hazard_draws {
                        return Err(format!(
                            "ui_pass hazard policy expected CB unobtained evidence on {} hazard draws, got {}",
                            self.hazard_draws, self.cb_unobtained_draws
                        )
                        .into());
                    }
                    if self.srv_unobtained_draws < self.hazard_draws {
                        return Err(format!(
                            "ui_pass hazard policy expected SRV unobtained evidence on {} hazard draws, got {}",
                            self.hazard_draws, self.srv_unobtained_draws
                        )
                        .into());
                    }
                    if self.normal_cb_draws == 0 || self.normal_srv_draws == 0 {
                        return Err(
                            "ui_pass hazard policy did not preserve normal evidence on non-hazard draws"
                                .into(),
                        );
                    }
                }
                UiCbSkipMode::All => {
                    if self.normal_cb_draws != 0 || self.normal_srv_draws != 0 {
                        return Err(
                            "ui_pass all policy captured normal CB/SRV payload evidence".into()
                        );
                    }
                    if self.cb_unobtained_draws == 0 || self.srv_unobtained_draws == 0 {
                        return Err(
                            "ui_pass all policy did not record unobtained CB/SRV evidence".into(),
                        );
                    }
                }
                UiCbSkipMode::None => {
                    if self.cb_unobtained_draws != 0 || self.srv_unobtained_draws != 0 {
                        return Err(
                            "ui_pass none policy recorded unobtained hazard evidence".into()
                        );
                    }
                    if self.normal_cb_draws == 0 || self.normal_srv_draws == 0 {
                        return Err(
                            "ui_pass none policy did not capture normal CB/SRV evidence".into()
                        );
                    }
                }
            }
            Ok(())
        }
    }

    fn assert_ui_draw_evidence(
        draw: &serde_json::Value,
        ui_cb_skip: UiCbSkipMode,
        evidence: &mut UiEvidenceStats,
        texture_payloads: &mut BTreeSet<String>,
    ) -> Result<()> {
        let draw_call = draw
            .get("draw_call")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let draw_count = draw
            .get("draw_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let hazard_draw = draw_call == "Draw" && draw_count <= 6;
        if hazard_draw {
            evidence.hazard_draws += 1;
        }
        let should_skip = ui_cb_skip.skips_draw(hazard_draw);
        let constant_buffers = draw
            .get("constant_buffers")
            .and_then(serde_json::Value::as_array)
            .ok_or("ui_pass draw has no constant_buffers array")?;
        let shader_resources = draw
            .get("shader_resources")
            .and_then(serde_json::Value::as_array)
            .ok_or("ui_pass draw has no shader_resources array")?;

        if should_skip {
            if !constant_buffers_have_unobtained_reason(constant_buffers) {
                return Err(format!(
                    "ui_pass {} draw_count={} missing unobtained CB evidence",
                    draw_call, draw_count
                )
                .into());
            }
            evidence.cb_unobtained_draws += 1;
            if !slot0_srv_has_unobtained_reason(shader_resources) {
                return Err(format!(
                    "ui_pass {} draw_count={} missing unobtained SRV evidence",
                    draw_call, draw_count
                )
                .into());
            }
            evidence.srv_unobtained_draws += 1;
        } else {
            if !constant_buffers_have_shadow_cb0_metadata(constant_buffers) {
                return Err(format!(
                    "ui_pass {} draw_count={} missing normal VS/PS CB0 metadata",
                    draw_call, draw_count
                )
                .into());
            }
            evidence.normal_cb_draws += 1;
            if !slot0_srv_has_payload(shader_resources, texture_payloads) {
                return Err(format!(
                    "ui_pass {} draw_count={} missing normal SRV payload",
                    draw_call, draw_count
                )
                .into());
            }
            evidence.normal_srv_draws += 1;
        }
        Ok(())
    }

    fn constant_buffers_have_shadow_cb0_metadata(bindings: &[serde_json::Value]) -> bool {
        has_shadow_cb0_metadata(bindings, "VS") && has_shadow_cb0_metadata(bindings, "PS")
    }

    fn has_shadow_cb0_metadata(bindings: &[serde_json::Value], stage: &str) -> bool {
        bindings.iter().any(|binding| {
            binding.get("stage").and_then(serde_json::Value::as_str) == Some(stage)
                && binding.get("slot").and_then(serde_json::Value::as_u64) == Some(0)
                && binding.get("file").map_or(true, serde_json::Value::is_null)
                && binding
                    .get("byte_len")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    > 0
                && binding.get("unknown_origin").is_none()
                && binding.get("unobtained_reason").is_none()
        })
    }

    fn constant_buffers_have_unobtained_reason(bindings: &[serde_json::Value]) -> bool {
        has_unobtained_cb0(bindings, "VS") && has_unobtained_cb0(bindings, "PS")
    }

    fn has_unobtained_cb0(bindings: &[serde_json::Value], stage: &str) -> bool {
        bindings.iter().any(|binding| {
            binding.get("stage").and_then(serde_json::Value::as_str) == Some(stage)
                && binding.get("slot").and_then(serde_json::Value::as_u64) == Some(0)
                && binding.get("file").map_or(true, serde_json::Value::is_null)
                && binding
                    .get("byte_len")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(u64::MAX)
                    == 0
                && binding
                    .get("unobtained_reason")
                    .and_then(serde_json::Value::as_str)
                    == Some(UI_INTEROP_HAZARD_UNOBTAINED_REASON)
        })
    }

    fn slot0_srv_has_unobtained_reason(resources: &[serde_json::Value]) -> bool {
        resources.iter().any(|resource| {
            resource.get("slot").and_then(serde_json::Value::as_u64) == Some(0)
                && resource
                    .get("file")
                    .map_or(true, serde_json::Value::is_null)
                && resource.get("byte_len").and_then(serde_json::Value::as_u64) == Some(0)
                && resource
                    .get("unobtained_reason")
                    .and_then(serde_json::Value::as_str)
                    == Some(UI_INTEROP_HAZARD_UNOBTAINED_REASON)
        })
    }

    fn slot0_srv_has_payload(
        resources: &[serde_json::Value],
        texture_payloads: &mut BTreeSet<String>,
    ) -> bool {
        let mut found = false;
        for resource in resources {
            if resource.get("slot").and_then(serde_json::Value::as_u64) != Some(0) {
                continue;
            }
            if resource.get("unobtained_reason").is_some() {
                return false;
            }
            if let Some(file) = resource.get("file").and_then(serde_json::Value::as_str) {
                texture_payloads.insert(file.to_string());
                found = true;
            }
        }
        found
    }

    unsafe fn churn_device_resources(device: *mut ID3D11Device, iteration: u32) -> Result<()> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: 16 + (iteration % 4),
            Height: 16 + (iteration % 4),
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = ptr::null_mut();
        let hr = unsafe { (*device).CreateTexture2D(&desc, ptr::null(), &mut texture) };
        if SUCCEEDED(hr) {
            unsafe { release_unknown(texture as *mut c_void) };
        }
        let bytes = [0u8; 256];
        let buffer = unsafe { create_buffer(device, D3D11_BIND_VERTEX_BUFFER, &bytes) };
        drop(buffer);
        let mut deferred = ptr::null_mut::<ID3D11DeviceContext>();
        let hr = unsafe { (*device).CreateDeferredContext(0, &mut deferred) };
        if SUCCEEDED(hr) && !deferred.is_null() {
            let mut null_srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
            unsafe {
                (*deferred).PSSetShaderResources(0, 1, &mut null_srv);
                (*deferred).Draw(3, 0);
                release_unknown(deferred as *mut c_void);
            }
        }
        Ok(())
    }

    unsafe fn create_backbuffer_render_target(
        device: *mut ID3D11Device,
        swap_chain: *mut IDXGISwapChain,
    ) -> Result<ComPtr<ID3D11RenderTargetView>> {
        let mut backbuffer = ptr::null_mut::<ID3D11Texture2D>();
        let hr = unsafe {
            (*swap_chain).GetBuffer(
                0,
                &IID_ID3D11Texture2D,
                &mut backbuffer as *mut _ as *mut *mut c_void,
            )
        };
        if FAILED(hr) {
            return Err(format!("IDXGISwapChain::GetBuffer failed: HRESULT 0x{hr:08X}").into());
        }
        let mut rtv = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateRenderTargetView(
                backbuffer as *mut ID3D11Resource,
                ptr::null(),
                &mut rtv,
            )
        };
        unsafe { release_unknown(backbuffer as *mut c_void) };
        if FAILED(hr) {
            return Err(format!("CreateRenderTargetView failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(rtv).ok_or_else(|| "CreateRenderTargetView returned null".into()) }
    }

    unsafe fn create_offscreen_map_target(
        device: *mut ID3D11Device,
    ) -> Result<(ComPtr<ID3D11RenderTargetView>, ComPtr<ID3D11Texture2D>)> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: WINDOW_WIDTH,
            Height: WINDOW_HEIGHT,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = ptr::null_mut::<ID3D11Texture2D>();
        let hr = unsafe { (*device).CreateTexture2D(&desc, ptr::null(), &mut tex) };
        if FAILED(hr) {
            return Err(
                format!("offscreen map target CreateTexture2D failed: HRESULT 0x{hr:08X}").into(),
            );
        }
        let mut rtv = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateRenderTargetView(tex as *mut ID3D11Resource, ptr::null(), &mut rtv)
        };
        if FAILED(hr) {
            unsafe { release_unknown(tex as *mut c_void) };
            return Err(format!(
                "offscreen map target CreateRenderTargetView failed: HRESULT 0x{hr:08X}"
            )
            .into());
        }
        let rtv = unsafe { ComPtr::new(rtv) }.ok_or("offscreen map RTV null")?;
        let tex = unsafe { ComPtr::new(tex) }.ok_or("offscreen map tex null")?;
        Ok((rtv, tex))
    }

    unsafe fn create_depth_stencil(
        device: *mut ID3D11Device,
    ) -> Result<ComPtr<ID3D11DepthStencilView>> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: WINDOW_WIDTH,
            Height: WINDOW_HEIGHT,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_D24_UNORM_S8_UINT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_DEPTH_STENCIL,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = ptr::null_mut();
        let hr = unsafe { (*device).CreateTexture2D(&desc, ptr::null(), &mut texture) };
        if FAILED(hr) {
            return Err(format!("CreateTexture2D depth failed: HRESULT 0x{hr:08X}").into());
        }
        let mut dsv = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateDepthStencilView(texture as *mut ID3D11Resource, ptr::null(), &mut dsv)
        };
        unsafe { release_unknown(texture as *mut c_void) };
        if FAILED(hr) {
            return Err(format!("CreateDepthStencilView failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(dsv).ok_or_else(|| "CreateDepthStencilView returned null".into()) }
    }

    unsafe fn create_default_samplers(
        device: *mut ID3D11Device,
    ) -> Result<Vec<ComPtr<ID3D11SamplerState>>> {
        let mut samplers = Vec::new();
        for _ in 0..16 {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                BorderColor: [0.0, 0.0, 0.0, 0.0],
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
            };
            let mut sampler = ptr::null_mut();
            let hr = unsafe { (*device).CreateSamplerState(&desc, &mut sampler) };
            if FAILED(hr) {
                return Err(format!("CreateSamplerState failed: HRESULT 0x{hr:08X}").into());
            }
            samplers.push(unsafe {
                ComPtr::new(sampler).ok_or("CreateSamplerState returned null sampler")?
            });
        }
        Ok(samplers)
    }

    unsafe fn create_vertex_shader(
        device: *mut ID3D11Device,
        bytes: &[u8],
    ) -> Result<ComPtr<ID3D11VertexShader>> {
        let mut shader = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateVertexShader(
                bytes.as_ptr().cast(),
                bytes.len(),
                ptr::null_mut(),
                &mut shader,
            )
        };
        if FAILED(hr) {
            return Err(format!("CreateVertexShader failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(shader).ok_or_else(|| "CreateVertexShader returned null".into()) }
    }

    unsafe fn create_pixel_shader(
        device: *mut ID3D11Device,
        bytes: &[u8],
    ) -> Result<ComPtr<ID3D11PixelShader>> {
        let mut shader = ptr::null_mut();
        let hr = unsafe {
            (*device).CreatePixelShader(
                bytes.as_ptr().cast(),
                bytes.len(),
                ptr::null_mut(),
                &mut shader,
            )
        };
        if FAILED(hr) {
            return Err(format!("CreatePixelShader failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(shader).ok_or_else(|| "CreatePixelShader returned null".into()) }
    }

    unsafe fn create_input_layout(
        device: *mut ID3D11Device,
        elements: &[InputElementRecord],
        shader_bytes: &[u8],
    ) -> Result<ComPtr<ID3D11InputLayout>> {
        let names = elements
            .iter()
            .map(|element| CString::new(element.semantic_name.as_str()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let native = elements
            .iter()
            .zip(names.iter())
            .map(|(element, name)| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: name.as_ptr(),
                SemanticIndex: element.semantic_index,
                Format: element.format as DXGI_FORMAT,
                InputSlot: element.input_slot,
                AlignedByteOffset: element.aligned_byte_offset,
                InputSlotClass: element.input_slot_class,
                InstanceDataStepRate: element.instance_data_step_rate,
            })
            .collect::<Vec<_>>();
        let mut layout = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateInputLayout(
                native.as_ptr(),
                native.len() as UINT,
                shader_bytes.as_ptr().cast(),
                shader_bytes.len(),
                &mut layout,
            )
        };
        if FAILED(hr) {
            return Err(format!("CreateInputLayout failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(layout).ok_or_else(|| "CreateInputLayout returned null".into()) }
    }

    unsafe fn create_buffer(
        device: *mut ID3D11Device,
        bind_flags: u32,
        bytes: &[u8],
    ) -> Result<ComPtr<ID3D11Buffer>> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: bytes.len().max(1) as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: bind_flags,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr().cast(),
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = ptr::null_mut();
        let hr = unsafe { (*device).CreateBuffer(&desc, &init, &mut buffer) };
        if FAILED(hr) {
            return Err(format!("CreateBuffer failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(buffer).ok_or_else(|| "CreateBuffer returned null".into()) }
    }

    unsafe fn create_texture_srv(
        device: *mut ID3D11Device,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<ComPtr<ID3D11ShaderResourceView>> {
        let mut texture = ptr::null_mut();
        let hr = unsafe { (*device).CreateTexture2D(desc, ptr::null(), &mut texture) };
        if FAILED(hr) {
            return Err(format!(
                "CreateTexture2D SRV resource {}x{} fmt {} failed: HRESULT 0x{hr:08X}",
                desc.Width, desc.Height, desc.Format
            )
            .into());
        }
        let mut srv = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateShaderResourceView(
                texture as *mut ID3D11Resource,
                ptr::null(),
                &mut srv,
            )
        };
        unsafe { release_unknown(texture as *mut c_void) };
        if FAILED(hr) {
            return Err(format!(
                "CreateShaderResourceView {}x{} fmt {} failed: HRESULT 0x{hr:08X}",
                desc.Width, desc.Height, desc.Format
            )
            .into());
        }
        unsafe { ComPtr::new(srv).ok_or_else(|| "CreateShaderResourceView returned null".into()) }
    }

    fn ui_texture_bytes(red: u8, green: u8, blue: u8) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                bytes.push(red.saturating_add(x * 3));
                bytes.push(green.saturating_add(y * 5));
                bytes.push(blue);
                bytes.push(0xD0);
            }
        }
        bytes
    }

    fn restore_color_cube_bytes() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1024 * 32 * 4);
        for y in 0..32u8 {
            for x in 0..1024u16 {
                bytes.push((x & 31) as u8 * 8);
                bytes.push(y * 8);
                bytes.push(((x >> 5) & 31) as u8 * 8);
                bytes.push(0xFF);
            }
        }
        bytes
    }

    fn restore_posteffect_cb_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8; 80];
        write_f32_le(&mut bytes, 0, 0.0);
        write_f32_le(&mut bytes, 4, 1.0);
        write_f32_le(&mut bytes, 8, 1.0);
        write_f32_le(&mut bytes, 12, 0.0);
        write_f32_le(&mut bytes, 16, 1.0);
        write_f32_le(&mut bytes, 20, 1.0);
        write_f32_le(&mut bytes, 24, 1.0);
        write_f32_le(&mut bytes, 28, 0.25);
        write_f32_le(&mut bytes, 32, 1.0 / 320.0);
        write_f32_le(&mut bytes, 36, 1.0 / 180.0);
        write_f32_le(&mut bytes, 40, 1.0 / 320.0);
        write_f32_le(&mut bytes, 44, 1.0 / 180.0);
        write_f32_le(&mut bytes, 48, 1.0);
        write_f32_le(&mut bytes, 52, 1.0);
        write_f32_le(&mut bytes, 56, 1.0);
        write_f32_le(&mut bytes, 60, 0.18);
        write_f32_le(&mut bytes, 64, 1.0);
        bytes
    }

    // Six vertices (two triangles) of a single label glyph quad. Layout matches
    // the input layout: float3 world position + float2 uv, stride 20. These
    // float3 positions are the ground-truth payload the moment captures.
    fn mapname_glyph_vertices() -> Vec<u8> {
        let quad = [
            ([-2500.0f32, 12.0, 1800.0], [0.0f32, 0.0]),
            ([-2400.0, 12.0, 1800.0], [1.0, 0.0]),
            ([-2400.0, 12.0, 1900.0], [1.0, 1.0]),
            ([-2500.0, 12.0, 1800.0], [0.0, 0.0]),
            ([-2400.0, 12.0, 1900.0], [1.0, 1.0]),
            ([-2500.0, 12.0, 1900.0], [0.0, 1.0]),
        ];
        let mut bytes = Vec::with_capacity(quad.len() * 20);
        for (pos, uv) in quad {
            for component in pos {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
            for component in uv {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        bytes
    }

    // Scene CB (slot 0): a ViewProjection-like matrix + camera look-at dir.
    fn mapname_scene_cb_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8; 80];
        // 4x4 matrix (identity-ish, non-trivial diagonal) + a look-at vec4.
        for i in 0..4 {
            write_f32_le(&mut bytes, i * 20, 1.0);
        }
        write_f32_le(&mut bytes, 64, 0.0);
        write_f32_le(&mut bytes, 68, -1.0);
        write_f32_le(&mut bytes, 72, 0.0);
        write_f32_le(&mut bytes, 76, 0.0);
        bytes
    }

    // Slot-1 Transp_OffsetX CB (32 bytes: float4 + padding). x = transparency,
    // y = wrap offset.
    fn mapname_transp_offset_cb_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8; 32];
        write_f32_le(&mut bytes, 0, 0.85);
        write_f32_le(&mut bytes, 4, 6144.0);
        write_f32_le(&mut bytes, 8, 0.0);
        write_f32_le(&mut bytes, 12, 0.0);
        bytes
    }

    fn mapname_font_atlas_bytes() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 * 8 * 4);
        for y in 0..8u8 {
            for x in 0..8u8 {
                bytes.push(x * 32);
                bytes.push(y * 32);
                bytes.push(0x40);
                bytes.push(0xFF);
            }
        }
        bytes
    }

    // Browser-overlay hazard: float2 position + float2 uv, stride 16. The
    // discriminator must reject this (float2 POSITION, not float3).
    fn mapname_hazard_vertices() -> Vec<u8> {
        let tri = [
            ([-0.5f32, -0.5], [0.0f32, 0.0]),
            ([0.5, -0.5], [1.0, 0.0]),
            ([0.0, 0.5], [0.5, 1.0]),
        ];
        let mut bytes = Vec::with_capacity(tri.len() * 16);
        for (pos, uv) in tri {
            for component in pos {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
            for component in uv {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        bytes
    }

    unsafe fn create_texture_srv_with_bytes(
        device: *mut ID3D11Device,
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> Result<ComPtr<ID3D11ShaderResourceView>> {
        let expected_len = width as usize * height as usize * 4;
        if bytes.len() != expected_len {
            return Err(format!(
                "ui texture byte length mismatch: expected {expected_len}, got {}",
                bytes.len()
            )
            .into());
        }
        unsafe {
            create_texture_srv_with_format_bytes(
                device,
                width,
                height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                width * 4,
                bytes,
                "rgba8 texture",
            )
        }
    }

    unsafe fn create_texture_srv_with_format_bytes(
        device: *mut ID3D11Device,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        row_pitch: u32,
        bytes: &[u8],
        label: &str,
    ) -> Result<ComPtr<ID3D11ShaderResourceView>> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr().cast(),
            SysMemPitch: row_pitch,
            SysMemSlicePitch: bytes.len() as UINT,
        };
        let mut texture = ptr::null_mut();
        let hr = unsafe { (*device).CreateTexture2D(&desc, &init, &mut texture) };
        if FAILED(hr) {
            return Err(format!("CreateTexture2D {label} failed: HRESULT 0x{hr:08X}").into());
        }
        let mut srv = ptr::null_mut();
        let hr = unsafe {
            (*device).CreateShaderResourceView(
                texture as *mut ID3D11Resource,
                ptr::null(),
                &mut srv,
            )
        };
        unsafe { release_unknown(texture as *mut c_void) };
        if FAILED(hr) {
            return Err(
                format!("CreateShaderResourceView {label} failed: HRESULT 0x{hr:08X}").into(),
            );
        }
        unsafe { ComPtr::new(srv).ok_or_else(|| "CreateShaderResourceView returned null".into()) }
    }

    unsafe fn create_ui_blend_state(device: *mut ID3D11Device) -> Result<ComPtr<ID3D11BlendState>> {
        let rt = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: TRUE,
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_ZERO,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL as u8,
        };
        let desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: FALSE,
            IndependentBlendEnable: FALSE,
            RenderTarget: [rt; 8],
        };
        let mut state = ptr::null_mut();
        let hr = unsafe { (*device).CreateBlendState(&desc, &mut state) };
        if FAILED(hr) {
            return Err(format!("CreateBlendState ui failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(state).ok_or_else(|| "CreateBlendState returned null".into()) }
    }

    unsafe fn create_ui_depth_disabled_state(
        device: *mut ID3D11Device,
    ) -> Result<ComPtr<ID3D11DepthStencilState>> {
        let mut desc: D3D11_DEPTH_STENCIL_DESC = unsafe { mem::zeroed() };
        desc.DepthEnable = FALSE;
        desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ZERO;
        desc.DepthFunc = D3D11_COMPARISON_ALWAYS;
        desc.StencilEnable = FALSE;
        desc.FrontFace.StencilFailOp = D3D11_STENCIL_OP_KEEP;
        desc.FrontFace.StencilDepthFailOp = D3D11_STENCIL_OP_KEEP;
        desc.FrontFace.StencilPassOp = D3D11_STENCIL_OP_KEEP;
        desc.FrontFace.StencilFunc = D3D11_COMPARISON_ALWAYS;
        desc.BackFace = desc.FrontFace;
        let mut state = ptr::null_mut();
        let hr = unsafe { (*device).CreateDepthStencilState(&desc, &mut state) };
        if FAILED(hr) {
            return Err(format!("CreateDepthStencilState ui failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(state).ok_or_else(|| "CreateDepthStencilState returned null".into()) }
    }

    unsafe fn create_ui_rasterizer_state(
        device: *mut ID3D11Device,
    ) -> Result<ComPtr<ID3D11RasterizerState>> {
        let desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: FALSE,
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: TRUE,
            ScissorEnable: TRUE,
            MultisampleEnable: FALSE,
            AntialiasedLineEnable: FALSE,
        };
        let mut state = ptr::null_mut();
        let hr = unsafe { (*device).CreateRasterizerState(&desc, &mut state) };
        if FAILED(hr) {
            return Err(format!("CreateRasterizerState ui failed: HRESULT 0x{hr:08X}").into());
        }
        unsafe { ComPtr::new(state).ok_or_else(|| "CreateRasterizerState returned null".into()) }
    }

    fn compile_shader(source: &str, entry: &str, target: &str) -> Result<Vec<u8>> {
        let entry = CString::new(entry)?;
        let target = CString::new(target)?;
        let source_name = CString::new("capture_harness.hlsl")?;
        let mut code = ptr::null_mut::<ID3DBlob>();
        let mut errors = ptr::null_mut::<ID3DBlob>();
        let hr = unsafe {
            D3DCompile(
                source.as_ptr().cast(),
                source.len(),
                source_name.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                entry.as_ptr(),
                target.as_ptr(),
                D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL0,
                0,
                &mut code,
                &mut errors,
            )
        };
        if FAILED(hr) {
            let message = blob_string(errors);
            unsafe { release_unknown(errors as *mut c_void) };
            return Err(
                format!("D3DCompile {target:?} failed: HRESULT 0x{hr:08X}: {message}").into(),
            );
        }
        unsafe { release_unknown(errors as *mut c_void) };
        let Some(code) = (unsafe { ComPtr::new(code) }) else {
            return Err("D3DCompile returned null code blob".into());
        };
        let bytes = unsafe {
            slice::from_raw_parts(
                (*code.as_ptr()).GetBufferPointer().cast::<u8>(),
                (*code.as_ptr()).GetBufferSize(),
            )
        }
        .to_vec();
        Ok(bytes)
    }

    fn blob_string(blob: *mut ID3DBlob) -> String {
        if blob.is_null() {
            return "<no compiler error blob>".to_string();
        }
        let bytes = unsafe {
            slice::from_raw_parts(
                (*blob).GetBufferPointer().cast::<u8>(),
                (*blob).GetBufferSize(),
            )
        };
        String::from_utf8_lossy(bytes).trim().to_string()
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    unsafe fn create_window() -> Result<HWND> {
        let class_name = wide_null(&format!(
            "TerrainCaptureHarnessWindow{}",
            std::process::id()
        ));
        let title = wide_null("Terrain Capture Harness");
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { mem::zeroed() }
        };
        let registered = unsafe { RegisterClassW(&wnd_class) };
        if registered == 0 {
            let error = unsafe { GetLastError() };
            return Err(format!("RegisterClassW failed: {error}").into());
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                WINDOW_WIDTH as i32,
                WINDOW_HEIGHT as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            let error = unsafe { GetLastError() };
            return Err(format!("CreateWindowExW failed: {error}").into());
        }
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
        }
        Ok(hwnd)
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }

    fn pad_constant_buffer(bytes: &mut Vec<u8>) {
        let padded = (bytes.len() + 15) & !15;
        bytes.resize(padded.max(16), 0);
    }

    fn write_f32_le(bytes: &mut [u8], offset: usize, value: f32) {
        let raw = value.to_le_bytes();
        bytes[offset..offset + 4].copy_from_slice(&raw);
    }

    #[derive(Debug)]
    struct ComPtr<T>(*mut T);

    impl<T> ComPtr<T> {
        unsafe fn new(ptr: *mut T) -> Option<Self> {
            (!ptr.is_null()).then_some(Self(ptr))
        }

        fn as_ptr(&self) -> *mut T {
            self.0
        }
    }

    impl<T> Drop for ComPtr<T> {
        fn drop(&mut self) {
            unsafe { release_unknown(self.0 as *mut c_void) };
        }
    }

    unsafe fn add_ref_unknown(object: *mut c_void) {
        if !object.is_null() {
            unsafe {
                (*(object as *mut IUnknown)).AddRef();
            }
        }
    }

    unsafe fn release_unknown(object: *mut c_void) {
        if !object.is_null() {
            unsafe {
                (*(object as *mut IUnknown)).Release();
            }
        }
    }

    const FILLER_VS: &str = r#"
        struct VSOut { float4 position : SV_Position; };
        VSOut main(uint id : SV_VertexID) {
            float2 p[3] = {
                float2(-0.8, -0.8),
                float2( 0.0,  0.8),
                float2( 0.8, -0.8)
            };
            VSOut o;
            o.position = float4(p[id % 3], 0.0, 1.0);
            return o;
        }
    "#;

    const FILLER_PS: &str = r#"
        float4 main() : SV_Target {
            return float4(0.03, 0.04, 0.05, 1.0);
        }
    "#;

    const RESTORE_VS: &str = r#"
        struct VSOut {
            float4 position : SV_Position;
            float2 uv : TEXCOORD0;
        };

        VSOut main(uint id : SV_VertexID) {
            float2 p[3] = {
                float2(-1.0, -1.0),
                float2(-1.0,  3.0),
                float2( 3.0, -1.0)
            };
            VSOut o;
            o.position = float4(p[id], 0.0, 1.0);
            o.uv = p[id] * float2(0.5, -0.5) + 0.5;
            return o;
        }
    "#;

    const RESTORE_PS: &str = r#"
        Texture2D<float4> MainScene : register(t0);
        Texture2D<float4> RestoreBloom : register(t1);
        Texture2D<float4> ColorCube : register(t2);
        Texture2D<float> AverageLuminanceTexture : register(t3);
        SamplerState MainSceneSampler : register(s0);
        SamplerState RestoreBloomSampler : register(s1);
        SamplerState ColorCubeSampler : register(s2);
        SamplerState AverageLuminanceSampler : register(s3);

        cbuffer PostEffectBase : register(b1) {
            float4 HSV;
            float3 ColorBalance;
            float EmissiveBloomStrength;
            float2 InvDownSampleSize;
            float2 InvWindowSize;
            float2 BloomToScreenScale;
            float BrightThreshold;
            float MiddleGrey;
            float LumWhite2;
        };

        float4 main(float4 position : SV_Position, float2 uv : TEXCOORD0) : SV_Target {
            float4 scene = MainScene.Sample(MainSceneSampler, uv);
            float4 bloom = RestoreBloom.Sample(RestoreBloomSampler, uv);
            float lum = AverageLuminanceTexture.Load(int3(0, 0, 0));
            float2 lut_uv = float2(frac(scene.r * 31.0 / 32.0), saturate(scene.g));
            float4 lut = ColorCube.Sample(ColorCubeSampler, lut_uv);
            float exposure = MiddleGrey / max(lum, 0.0001);
            float3 graded = lerp(scene.rgb, lut.rgb, 0.25) * ColorBalance * exposure;
            graded += bloom.rgb * EmissiveBloomStrength;
            graded = lerp(graded, graded * HSV.y, saturate(HSV.z));
            return float4(saturate(graded), scene.a);
        }
    "#;

    // Mirrors gfx/FX/mapname.shader: float3 POSITION + float2 TEXCOORD0 VS with
    // a scene CB at b0 (ViewProjection + camera look-at) and Transp_OffsetX at
    // b1. This is the exact reflected signature the discriminator keys on.
    const MAPNAME_VS: &str = r#"
        cbuffer SceneConstants : register(b0) {
            float4x4 ViewProjectionMatrix;
            float4 vCamLookAtDir;
        };
        cbuffer TranspOffset : register(b1) {
            float4 Transp_OffsetX;
        };

        struct VSIn {
            float3 vPosition : POSITION;
            float2 vTexCoord : TEXCOORD0;
        };

        struct VSOut {
            float4 position : SV_Position;
            float3 prepos : TEXCOORD0;
            float2 uv : TEXCOORD1;
        };

        VSOut main(VSIn input) {
            VSOut output;
            float4 pos = float4(input.vPosition, 1.0);
            pos.x += Transp_OffsetX.y;
            output.position = mul(ViewProjectionMatrix, pos) + vCamLookAtDir * 0.0;
            output.prepos = input.vPosition.xyz;
            output.uv = input.vTexCoord;
            return output;
        }
    "#;

    // Mirrors the mapname PS: a single DiffuseTexture (font atlas) at t0/s0 and
    // Transp_OffsetX.x transparency from b1.
    const MAPNAME_PS: &str = r#"
        Texture2D<float4> DiffuseTexture : register(t0);
        SamplerState DiffuseSampler : register(s0);
        cbuffer TranspOffset : register(b1) {
            float4 Transp_OffsetX;
        };

        float4 main(float4 position : SV_Position, float3 prepos : TEXCOORD0, float2 uv : TEXCOORD1) : SV_Target {
            float4 sample = DiffuseTexture.Sample(DiffuseSampler, uv);
            sample.a *= Transp_OffsetX.x;
            return sample;
        }
    "#;

    // Browser-overlay hazard: float2 POSITION VS + a two-texture PS. The
    // mapname discriminator must reject both the float2 VS signature and the
    // multi-texture PS.
    const MAPNAME_HAZARD_VS: &str = r#"
        struct VSIn {
            float2 position : POSITION;
            float2 uv : TEXCOORD0;
        };
        struct VSOut {
            float4 position : SV_Position;
            float2 uv : TEXCOORD0;
        };
        VSOut main(VSIn input) {
            VSOut output;
            output.position = float4(input.position, 0.0, 1.0);
            output.uv = input.uv;
            return output;
        }
    "#;

    const MAPNAME_HAZARD_PS: &str = r#"
        Texture2D<float4> BrowserTexture : register(t0);
        Texture2D<float4> ColorMask : register(t1);
        SamplerState BrowserSampler : register(s0);
        float4 main(float4 position : SV_Position, float2 uv : TEXCOORD0) : SV_Target {
            return BrowserTexture.Sample(BrowserSampler, uv) * ColorMask.Sample(BrowserSampler, uv);
        }
    "#;

    const UI_VS: &str = r#"
        cbuffer UiVsConstants : register(b0) {
            float2 UiScale;
            float2 UiOffset;
        };

        struct VSIn {
            float2 position : POSITION;
            float2 uv : TEXCOORD0;
        };

        struct VSOut {
            float4 position : SV_Position;
            float2 uv : TEXCOORD0;
        };

        VSOut main(VSIn input) {
            VSOut output;
            output.position = float4(input.position * UiScale + UiOffset, 0.0, 1.0);
            output.uv = input.uv;
            return output;
        }
    "#;

    // Distinct bytecode from UI_VS so the foreign draw's vertex shader is a
    // separate D3D object from the tracked ui vertex shader (the draw itself
    // is classified foreign because it binds a NULL pixel shader).
    const FOREIGN_UI_VS: &str = r#"
        cbuffer UiVsConstants : register(b0) {
            float2 UiScale;
            float2 UiOffset;
        };

        struct VSIn {
            float2 position : POSITION;
            float2 uv : TEXCOORD0;
        };

        struct VSOut {
            float4 position : SV_Position;
            float2 uv : TEXCOORD0;
        };

        VSOut main(VSIn input) {
            VSOut output;
            float2 scaled = input.position * UiScale * 0.5 + UiOffset;
            output.position = float4(scaled + UiOffset * 0.25, 0.0, 1.0);
            output.uv = input.uv * 0.75;
            return output;
        }
    "#;

    const UI_PS: &str = r#"
        Texture2D<float4> UiTexture : register(t0);
        SamplerState UiSampler : register(s0);

        cbuffer UiPsConstants : register(b0) {
            float4 Tint;
        };

        float4 main(float4 position : SV_Position, float2 uv : TEXCOORD0) : SV_Target {
            return UiTexture.Sample(UiSampler, uv) * Tint;
        }
    "#;

    const RIVER_PS: &str = r#"
        Texture2D<float4> RiverData : register(t0);
        Texture2D<float4> LeanTexture1 : register(t1);
        Texture2D<float4> LeanTexture2 : register(t2);
        Texture2D<float4> SnowMudTexture : register(t3);
        Texture2D<float4> GradientBorderChannel1 : register(t4);
        Texture2D<float4> GradientBorderChannel2 : register(t5);

        float4 main(float4 position : SV_Position) : SV_Target {
            int3 uv = int3(0, 0, 0);
            float v =
                RiverData.Load(uv).r +
                LeanTexture1.Load(uv).g +
                LeanTexture2.Load(uv).b +
                SnowMudTexture.Load(uv).a +
                GradientBorderChannel1.Load(uv).r +
                GradientBorderChannel2.Load(uv).g;
            return float4(v, v, v, 1.0);
        }
    "#;

    const BORDER_PS: &str = r#"
        Texture2D<float4> BorderDiffuse : register(t0);
        Texture2D<float4> SnowMudData : register(t1);
        Texture2D<float4> IntelMap : register(t2);
        Texture2D<float4> ShadowMap : register(t3);

        float4 main(float4 position : SV_Position) : SV_Target {
            int3 uv = int3(0, 0, 0);
            float v =
                BorderDiffuse.Load(uv).r +
                SnowMudData.Load(uv).g +
                IntelMap.Load(uv).b +
                ShadowMap.Load(uv).a;
            return float4(v, 0.0, 1.0 - v, 1.0);
        }
    "#;

    const TREE_PS: &str = r#"
        Texture2D<float4> DiffuseMap : register(t0);
        Texture2D<float4> NormalMap : register(t1);
        Texture2D<float4> TintMap : register(t2);
        Texture2D<float4> SeasonMap : register(t3);
        Texture2D<float4> TreeMaskTexture : register(t4);
        Texture2D<float4> GradientBorderChannel1 : register(t5);
        Texture2D<float4> GradientBorderChannel2 : register(t6);

        float4 main(float4 position : SV_Position) : SV_Target {
            int3 uv = int3(0, 0, 0);
            float v =
                DiffuseMap.Load(uv).r +
                NormalMap.Load(uv).g +
                TintMap.Load(uv).b +
                SeasonMap.Load(uv).a +
                TreeMaskTexture.Load(uv).r +
                GradientBorderChannel1.Load(uv).g +
                GradientBorderChannel2.Load(uv).b;
            return float4(0.0, v, 1.0 - v, 1.0);
        }
    "#;
}
