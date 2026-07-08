use crate::*;
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::fs;
use std::path::Path;

pub(crate) fn view_bundle(bundle: &Path) -> Result<()> {
    let data = load_bundle(bundle)?;
    let mut options = WindowOptions::default();
    options.resize = true;
    options.scale_mode = ScaleMode::AspectRatioStretch;
    let mut window = Window::new("the game Rust Map Viewer", DEFAULT_W, DEFAULT_H, options)?;
    window.set_target_fps(60);

    let mut state = ViewState {
        zoom: 0.18,
        offset_x: 0.0,
        offset_y: 0.0,
        dragging: false,
        drag_x: 0.0,
        drag_y: 0.0,
        drag_offset_x: 0.0,
        drag_offset_y: 0.0,
        selected: None,
    };
    let mut screen = window.get_size();
    let mut buffer = vec![0u32; screen.0 * screen.1];
    fit_world(&mut state, &data, screen);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let new_size = window.get_size();
        if new_size != screen {
            screen = new_size;
            buffer.resize(screen.0 * screen.1, 0);
        }

        if window.is_key_pressed(Key::R, KeyRepeat::No) {
            fit_world(&mut state, &data, screen);
        }

        if let Some((_, wheel_y)) = window.get_scroll_wheel() {
            if wheel_y.abs() > f32::EPSILON {
                let mouse = window
                    .get_mouse_pos(MouseMode::Clamp)
                    .unwrap_or((screen.0 as f32 * 0.5, screen.1 as f32 * 0.5));
                zoom_at(&mut state, mouse, wheel_y);
            }
        }

        let mouse = window.get_mouse_pos(MouseMode::Clamp);
        let left_down = window.get_mouse_down(MouseButton::Left);
        if let Some((mx, my)) = mouse {
            if left_down && !state.dragging {
                state.dragging = true;
                state.drag_x = mx;
                state.drag_y = my;
                state.drag_offset_x = state.offset_x;
                state.drag_offset_y = state.offset_y;
            } else if left_down && state.dragging {
                state.offset_x = state.drag_offset_x + mx - state.drag_x;
                state.offset_y = state.drag_offset_y + my - state.drag_y;
            } else if !left_down && state.dragging {
                state.dragging = false;
                if let Some(id) = province_at_screen(&data, &state, mx, my) {
                    state.selected = Some(id);
                }
            }
            let hover = province_at_screen(&data, &state, mx, my);
            window.set_title(&probe_title(&data, hover, state.selected));
        }

        render(&data, &state, screen, &mut buffer);
        window.update_with_buffer(&buffer, screen.0, screen.1)?;
    }
    Ok(())
}

pub(crate) fn view_production_map_d3d11(
    terrain_map_frame_bundle: &Path,
    water_map_frame_bundle: Option<&Path>,
    rivers_map_frame_bundle: Option<&Path>,
    borders_map_frame_bundle: Option<&Path>,
    trees_map_frame_bundle: Option<&Path>,
    camera: Option<ProductionMapCamera>,
) -> Result<()> {
    let terrain_bundle = NativeD3d11TerrainBundle::load(terrain_map_frame_bundle)?;
    let water_bundle = water_map_frame_bundle
        .map(NativeD3d11TerrainBundle::load_water)
        .transpose()?;
    let rivers_bundle = rivers_map_frame_bundle
        .map(NativeD3d11TerrainBundle::load_rivers)
        .transpose()?;
    let borders_bundle = borders_map_frame_bundle
        .map(NativeD3d11TerrainBundle::load_borders)
        .transpose()?;
    let trees_bundle = trees_map_frame_bundle
        .map(NativeD3d11TerrainBundle::load_trees)
        .transpose()?;
    let renderer = NativeD3d11TerrainRenderer::new()?;
    let camera = match camera {
        Some(camera) => camera,
        None => terrain_bundle.production_camera_from_capture()?,
    };
    let camera_defines = hoi4_map_camera_defines_from_bundle(&terrain_bundle)?;
    let mut camera_state = ProductionMapCameraViewState::new(camera, camera_defines);
    let mut prepared = renderer.prepare_production_map_with_layers(
        &terrain_bundle,
        water_bundle.as_ref(),
        rivers_bundle.as_ref(),
        borders_bundle.as_ref(),
        trees_bundle.as_ref(),
        camera,
    )?;
    let mut frame = render_prepared_production_map_view_frame(&renderer, &prepared)?;

    let mut options = WindowOptions::default();
    options.resize = true;
    options.scale_mode = ScaleMode::AspectRatioStretch;
    let mut window = Window::new("the game Production Map", DEFAULT_W, DEFAULT_H, options)?;
    window.set_target_fps(60);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        if update_production_map_camera_controls(&window, &mut camera_state) {
            renderer
                .update_prepared_production_map_camera(&mut prepared, camera_state.camera()?)?;
            frame = render_prepared_production_map_view_frame(&renderer, &prepared)?;
        }
        window.update_with_buffer(&frame.pixels, frame.width, frame.height)?;
    }
    Ok(())
}

pub(crate) fn render_prepared_production_map_view_frame(
    renderer: &NativeD3d11TerrainRenderer,
    prepared: &NativeD3d11PreparedProductionMap,
) -> Result<ProductionMapWindowFrame> {
    let options = NativeD3d11RenderOptions::default()
        .with_readback()
        .without_final_reference_comparison();
    let result = renderer.render_prepared_production_map(prepared, &options)?;
    let readback = result
        .readback
        .ok_or("production native D3D11 map viewer blocked: render produced no readback")?;
    native_d3d11_readback_to_window_frame(&readback)
}

impl ProductionMapCameraViewState {
    pub(crate) fn new(base: ProductionMapCamera, defines: Hoi4MapCameraDefines) -> Self {
        let mut state = Self {
            base,
            defines,
            zoom: 1.0,
            pan_clip_x: 0.0,
            pan_clip_y: 0.0,
            dragging: false,
            drag_x: 0.0,
            drag_y: 0.0,
            drag_pan_clip_x: 0.0,
            drag_pan_clip_y: 0.0,
        };
        state.zoom = state.clamp_zoom(1.0);
        state
    }

    pub(crate) fn reset(&mut self) {
        self.zoom = self.clamp_zoom(1.0);
        self.pan_clip_x = 0.0;
        self.pan_clip_y = 0.0;
        self.dragging = false;
    }

    pub(crate) fn camera(&self) -> Result<ProductionMapCamera> {
        let zoom = self.clamp_zoom(self.zoom);
        let matrix = production_map_camera_clip_transform(
            self.base.view_projection_matrix,
            zoom,
            self.pan_clip_x,
            self.pan_clip_y,
        );
        let mut position = self.base.position;
        position[1] = self.camera_height_for_zoom(zoom);
        ProductionMapCamera::new(matrix, position)
    }

    pub(crate) fn zoom_bounds(&self) -> (f32, f32) {
        let base_height = self
            .base
            .position
            .get(1)
            .copied()
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(self.defines.max_height);
        let min_zoom = (base_height / self.defines.max_height).max(f32::MIN_POSITIVE);
        let max_zoom = (base_height / self.defines.min_height).max(min_zoom);
        (min_zoom, max_zoom)
    }

    pub(crate) fn clamp_zoom(&self, zoom: f32) -> f32 {
        let (min_zoom, max_zoom) = self.zoom_bounds();
        let zoom = if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        };
        zoom.clamp(min_zoom, max_zoom)
    }

    pub(crate) fn camera_height_for_zoom(&self, zoom: f32) -> f32 {
        let zoom = self.clamp_zoom(zoom);
        (self.base.position[1] / zoom).clamp(self.defines.min_height, self.defines.max_height)
    }

    pub(crate) fn camera_height_fraction(&self) -> f32 {
        let height = self.camera_height_for_zoom(self.zoom);
        ((height - self.defines.min_height) / (self.defines.max_height - self.defines.min_height))
            .clamp(0.0, 1.0)
    }

    pub(crate) fn camera_define_distance_multiplier(&self) -> f32 {
        let extra = (self.defines.zoom_speed_distance_mult - 1.0).max(0.0);
        1.0 + extra * self.camera_height_fraction()
    }

    pub(crate) fn camera_define_input_step(&self) -> f32 {
        let step = self.defines.zoom_speed
            * self.defines.zoom_key_scale
            * self.camera_define_distance_multiplier()
            / PRODUCTION_CAMERA_CONTROL_UPDATE_HZ;
        if step.is_finite() {
            step.max(0.0)
        } else {
            0.0
        }
    }

    pub(crate) fn edge_pan_threshold_px(&self, window_size: (usize, usize)) -> f32 {
        let min_dim = window_size.0.min(window_size.1) as f32;
        let threshold = min_dim * self.defines.zoom_key_scale;
        if threshold.is_finite() && threshold > 0.0 {
            threshold.min(min_dim * 0.5)
        } else {
            0.0
        }
    }

    pub(crate) fn zoom_at(&mut self, window_size: (usize, usize), mouse: (f32, f32), wheel_y: f32) {
        let old_zoom = self.clamp_zoom(self.zoom);
        let step = self.camera_define_input_step();
        if step <= 0.0 {
            return;
        }
        let factor = (1.0 + step).powf(wheel_y.abs());
        let next_zoom = if !factor.is_finite() || factor <= 0.0 {
            let (min_zoom, max_zoom) = self.zoom_bounds();
            if wheel_y > 0.0 {
                max_zoom
            } else {
                min_zoom
            }
        } else if wheel_y > 0.0 {
            old_zoom * factor
        } else {
            old_zoom / factor
        };
        self.zoom = self.clamp_zoom(next_zoom);
        let actual_factor = self.zoom / old_zoom;
        if (actual_factor - 1.0).abs() <= f32::EPSILON {
            return;
        }
        let cursor_clip_x = mouse.0 / window_size.0.max(1) as f32 * 2.0 - 1.0;
        let cursor_clip_y = 1.0 - mouse.1 / window_size.1.max(1) as f32 * 2.0;
        self.pan_clip_x = cursor_clip_x - actual_factor * (cursor_clip_x - self.pan_clip_x);
        self.pan_clip_y = cursor_clip_y - actual_factor * (cursor_clip_y - self.pan_clip_y);
    }
}

pub(crate) fn update_production_map_camera_controls(
    window: &Window,
    state: &mut ProductionMapCameraViewState,
) -> bool {
    let mut changed = false;
    let window_size = window.get_size();

    if window.is_key_pressed(Key::R, KeyRepeat::No) {
        state.reset();
        changed = true;
    }

    if let Some((_, wheel_y)) = window.get_scroll_wheel() {
        if wheel_y.abs() > f32::EPSILON {
            let mouse = window
                .get_mouse_pos(MouseMode::Clamp)
                .unwrap_or((window_size.0 as f32 * 0.5, window_size.1 as f32 * 0.5));
            state.zoom_at(window_size, mouse, wheel_y);
            changed = true;
        }
    }

    let drag_down =
        window.get_mouse_down(MouseButton::Right) || window.get_mouse_down(MouseButton::Middle);
    if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Clamp) {
        if drag_down && !state.dragging {
            state.dragging = true;
            state.drag_x = mx;
            state.drag_y = my;
            state.drag_pan_clip_x = state.pan_clip_x;
            state.drag_pan_clip_y = state.pan_clip_y;
        } else if drag_down && state.dragging {
            state.pan_clip_x =
                state.drag_pan_clip_x + (mx - state.drag_x) * 2.0 / window_size.0.max(1) as f32;
            state.pan_clip_y =
                state.drag_pan_clip_y - (my - state.drag_y) * 2.0 / window_size.1.max(1) as f32;
            changed = true;
        } else if !drag_down && state.dragging {
            state.dragging = false;
        }

        let edge = state.edge_pan_threshold_px(window_size);
        let edge_step = state.camera_define_input_step();
        if edge_step > 0.0 {
            if mx <= edge {
                state.pan_clip_x += edge_step;
                changed = true;
            } else if mx >= window_size.0 as f32 - edge {
                state.pan_clip_x -= edge_step;
                changed = true;
            }
            if my <= edge {
                state.pan_clip_y -= edge_step;
                changed = true;
            } else if my >= window_size.1 as f32 - edge {
                state.pan_clip_y += edge_step;
                changed = true;
            }
        }
    } else {
        state.dragging = false;
    }

    let key_step = state.camera_define_input_step();
    if key_step > 0.0 {
        if window.is_key_down(Key::A) || window.is_key_down(Key::Left) {
            state.pan_clip_x += key_step;
            changed = true;
        }
        if window.is_key_down(Key::D) || window.is_key_down(Key::Right) {
            state.pan_clip_x -= key_step;
            changed = true;
        }
        if window.is_key_down(Key::W) || window.is_key_down(Key::Up) {
            state.pan_clip_y -= key_step;
            changed = true;
        }
        if window.is_key_down(Key::S) || window.is_key_down(Key::Down) {
            state.pan_clip_y += key_step;
            changed = true;
        }
    }

    changed
}

pub(crate) fn production_map_camera_clip_transform(
    base: [[f32; 4]; 4],
    zoom: f32,
    pan_clip_x: f32,
    pan_clip_y: f32,
) -> [[f32; 4]; 4] {
    let mut matrix = base;
    for column in 0..4 {
        matrix[0][column] = base[0][column] * zoom + base[3][column] * pan_clip_x;
        matrix[1][column] = base[1][column] * zoom + base[3][column] * pan_clip_y;
    }
    matrix
}

pub(crate) fn native_d3d11_readback_to_window_frame(
    readback: &NativeD3d11ReplayReadback,
) -> Result<ProductionMapWindowFrame> {
    let width = usize::try_from(readback.width)?;
    let height = usize::try_from(readback.height)?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or("production native D3D11 map viewer blocked: readback pixel count overflow")?;
    let row_pitch = usize::try_from(readback.row_pitch)?;
    let expected_len = row_pitch
        .checked_mul(height)
        .ok_or("production native D3D11 map viewer blocked: readback byte count overflow")?;
    if readback.bytes.len() != expected_len {
        return Err(format!(
            "production native D3D11 map viewer blocked: readback has {} bytes, expected {}",
            readback.bytes.len(),
            expected_len
        )
        .into());
    }

    let mut pixels = Vec::with_capacity(pixel_count);
    match readback.dxgi_format {
        DXGI_FORMAT_R8G8B8A8_UNORM => {
            for chunk in readback.bytes.chunks_exact(4) {
                pixels.push(window_rgb(chunk[0], chunk[1], chunk[2]));
            }
        }
        DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
        | DXGI_FORMAT_B8G8R8X8_UNORM => {
            for chunk in readback.bytes.chunks_exact(4) {
                pixels.push(window_rgb(chunk[2], chunk[1], chunk[0]));
            }
        }
        DXGI_FORMAT_R16G16B16A16_FLOAT => {
            for chunk in readback.bytes.chunks_exact(8) {
                let r = half_float_to_unorm8(u16::from_le_bytes([chunk[0], chunk[1]]));
                let g = half_float_to_unorm8(u16::from_le_bytes([chunk[2], chunk[3]]));
                let b = half_float_to_unorm8(u16::from_le_bytes([chunk[4], chunk[5]]));
                pixels.push(window_rgb(r, g, b));
            }
        }
        other => {
            return Err(format!(
            "production native D3D11 map viewer blocked: unsupported readback DXGI format {other}"
        )
            .into())
        }
    }
    if pixels.len() != pixel_count {
        return Err(format!(
            "production native D3D11 map viewer blocked: converted {} pixels, expected {}",
            pixels.len(),
            pixel_count
        )
        .into());
    }
    Ok(ProductionMapWindowFrame {
        width,
        height,
        pixels,
    })
}

pub(crate) fn window_rgb(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

pub(crate) fn half_float_to_unorm8(bits: u16) -> u8 {
    let value = half_float_to_f32(bits);
    if !value.is_finite() {
        return if value.is_sign_positive() { 255 } else { 0 };
    }
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(crate) fn half_float_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    match exponent {
        0 => {
            if fraction == 0 {
                sign * 0.0
            } else {
                sign * 2.0f32.powi(-14) * (f32::from(fraction) / 1024.0)
            }
        }
        0x1f => {
            if fraction == 0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * 2.0f32.powi(i32::from(exponent) - 15) * (1.0 + f32::from(fraction) / 1024.0),
    }
}

pub(crate) fn load_bundle(bundle: &Path) -> Result<BundleData> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(bundle.join("manifest.json"))?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let len = manifest.dimensions.width * manifest.dimensions.height;
    Ok(BundleData {
        province_ids: read_u32_raw(&bundle.join(&manifest.files.province_ids), len)?,
        visual: read_u32_raw(&bundle.join(&manifest.files.map_visual), len)?,
        province_to_state: serde_json::from_slice(&fs::read(
            bundle.join(&manifest.files.province_to_state),
        )?)?,
        province_to_region: serde_json::from_slice(&fs::read(
            bundle.join(&manifest.files.province_to_strategic_region),
        )?)?,
        state_owners: serde_json::from_slice(&fs::read(
            bundle.join(&manifest.files.state_owner_1936),
        )?)?,
        state_meta: serde_json::from_slice(&fs::read(bundle.join(&manifest.files.state_meta))?)?,
        province_defs: serde_json::from_slice(&fs::read(
            bundle.join(&manifest.files.province_definitions),
        )?)?,
        manifest,
    })
}

pub(crate) fn fit_world(state: &mut ViewState, data: &BundleData, screen: (usize, usize)) {
    let w = data.manifest.dimensions.width as f32;
    let h = data.manifest.dimensions.height as f32;
    state.zoom = (screen.0 as f32 / w).min(screen.1 as f32 / h);
    state.offset_x = (screen.0 as f32 - w * state.zoom) * 0.5;
    state.offset_y = (screen.1 as f32 - h * state.zoom) * 0.5;
}

pub(crate) fn zoom_at(state: &mut ViewState, mouse: (f32, f32), wheel_y: f32) {
    let before_x = (mouse.0 - state.offset_x) / state.zoom;
    let before_y = (mouse.1 - state.offset_y) / state.zoom;
    let factor = if wheel_y > 0.0 { 1.18 } else { 1.0 / 1.18 };
    state.zoom = (state.zoom * factor).clamp(0.05, 8.0);
    state.offset_x = mouse.0 - before_x * state.zoom;
    state.offset_y = mouse.1 - before_y * state.zoom;
}

pub(crate) fn province_at_screen(
    data: &BundleData,
    state: &ViewState,
    x: f32,
    y: f32,
) -> Option<u32> {
    let mx = ((x - state.offset_x) / state.zoom).floor() as isize;
    let my = ((y - state.offset_y) / state.zoom).floor() as isize;
    if mx < 0
        || my < 0
        || mx >= data.manifest.dimensions.width as isize
        || my >= data.manifest.dimensions.height as isize
    {
        return None;
    }
    let idx = my as usize * data.manifest.dimensions.width + mx as usize;
    data.province_ids.get(idx).copied().filter(|id| *id != 0)
}

pub(crate) fn probe_title(data: &BundleData, hover: Option<u32>, selected: Option<u32>) -> String {
    let Some(province) = hover else {
        return format!(
            "the game Rust Map Viewer | selected {}",
            selected.map_or("none".to_string(), |id| id.to_string())
        );
    };
    let state = data.province_to_state.get(&province).copied();
    let state_label = state.and_then(|s| data.state_meta.get(&s).map(|m| (s, m.name_key.as_str())));
    let owner = state
        .and_then(|s| data.state_owners.get(&s))
        .map(String::as_str)
        .unwrap_or("none");
    let region = data.province_to_region.get(&province).copied();
    let def = data.province_defs.get(&province);
    let terrain = def.map(|d| d.terrain.as_str()).unwrap_or("unknown");
    let ptype = def.map(|d| d.province_type.as_str()).unwrap_or("unknown");
    format!(
        "the game Rust Map Viewer | province {province} | state {} | owner {owner} | {ptype}/{terrain} | region {} | selected {}",
        state_label.map_or("none".to_string(), |(s, name)| format!("{s} {name}")),
        region.map_or("none".to_string(), |r| r.to_string()),
        selected.map_or("none".to_string(), |id| id.to_string())
    )
}

pub(crate) fn render(
    data: &BundleData,
    state: &ViewState,
    screen: (usize, usize),
    buffer: &mut [u32],
) {
    let map_w = data.manifest.dimensions.width;
    let map_h = data.manifest.dimensions.height;
    for sy in 0..screen.1 {
        let my = ((sy as f32 - state.offset_y) / state.zoom).floor() as isize;
        let row = sy * screen.0;
        if my < 0 || my >= map_h as isize {
            for px in &mut buffer[row..row + screen.0] {
                *px = 0;
            }
            continue;
        }
        let source_row = my as usize * map_w;
        for sx in 0..screen.0 {
            let mx = ((sx as f32 - state.offset_x) / state.zoom).floor() as isize;
            buffer[row + sx] = if mx < 0 || mx >= map_w as isize {
                0
            } else {
                data.visual[source_row + mx as usize]
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn production_camera_clip_transform_applies_zoom_and_pan_to_clip_rows() {
        let transformed = production_map_camera_clip_transform(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            2.0,
            0.25,
            -0.5,
        );

        assert_eq!(
            transformed,
            [
                [2.0, 0.0, 0.0, 0.25],
                [0.0, 2.0, 0.0, -0.5],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
    }

    #[test]
    fn production_map_camera_view_state_clamps_zoom_to_graphics_define_heights() {
        let camera = ProductionMapCamera::new(identity4(), [0.0, 1000.0, 0.0])
            .expect("finite production camera");
        let defines = Hoi4MapCameraDefines::new(50.0, 3000.0, 200.0, 200.0, 50.0, 0.02, 6.0)
            .expect("valid the game camera defines");
        let mut state = ProductionMapCameraViewState::new(camera, defines);
        let (min_zoom, max_zoom) = state.zoom_bounds();

        assert_close(min_zoom, 1000.0 / 3000.0);
        assert_close(max_zoom, 1000.0 / 50.0);

        state.zoom = 0.001;
        let max_height_camera = state.camera().expect("camera at max height");
        assert_close(max_height_camera.position[1], 3000.0);
        assert_close(max_height_camera.view_projection_matrix[0][0], min_zoom);

        state.zoom = 1000.0;
        let min_height_camera = state.camera().expect("camera at min height");
        assert_close(min_height_camera.position[1], 50.0);
        assert_close(min_height_camera.view_projection_matrix[0][0], max_zoom);
    }

    #[test]
    fn production_map_camera_view_state_uses_hoi4_zoom_define_step() {
        let camera = ProductionMapCamera::new(identity4(), [0.0, 1000.0, 0.0])
            .expect("finite production camera");
        let defines = Hoi4MapCameraDefines::new(50.0, 3000.0, 200.0, 200.0, 50.0, 0.02, 6.0)
            .expect("valid the game camera defines");
        let mut state = ProductionMapCameraViewState::new(camera, defines);
        let (min_zoom, max_zoom) = state.zoom_bounds();

        state.zoom = max_zoom;
        assert_close(
            state.camera_define_input_step(),
            50.0 * 0.02 / PRODUCTION_CAMERA_CONTROL_UPDATE_HZ,
        );

        state.zoom = min_zoom;
        assert_close(
            state.camera_define_input_step(),
            50.0 * 0.02 * 6.0 / PRODUCTION_CAMERA_CONTROL_UPDATE_HZ,
        );
        assert_close(state.edge_pan_threshold_px((1280, 720)), 720.0 * 0.02);
    }

    #[test]
    fn production_map_camera_view_state_wheel_zoom_stays_inside_hoi4_height_range() {
        let camera = ProductionMapCamera::new(identity4(), [0.0, 1000.0, 0.0])
            .expect("finite production camera");
        let defines = Hoi4MapCameraDefines::new(50.0, 3000.0, 200.0, 200.0, 50.0, 0.02, 6.0)
            .expect("valid the game camera defines");
        let mut state = ProductionMapCameraViewState::new(camera, defines);
        let (min_zoom, max_zoom) = state.zoom_bounds();

        for _ in 0..200 {
            state.zoom_at((1280, 720), (640.0, 360.0), 1.0);
        }
        assert_close(state.zoom, max_zoom);
        assert_close(state.camera().expect("min height camera").position[1], 50.0);

        for _ in 0..400 {
            state.zoom_at((1280, 720), (640.0, 360.0), -1.0);
        }
        assert_close(state.zoom, min_zoom);
        assert_close(
            state.camera().expect("max height camera").position[1],
            3000.0,
        );
    }

    #[test]
    fn native_d3d11_readback_conversion_handles_bgra_and_half_float() {
        let bgra = NativeD3d11ReplayReadback {
            width: 2,
            height: 1,
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM,
            row_pitch: 8,
            bytes: vec![1, 2, 3, 255, 4, 5, 6, 255],
        };
        let frame = native_d3d11_readback_to_window_frame(&bgra).expect("convert BGRA readback");
        assert_eq!(frame.pixels, vec![window_rgb(3, 2, 1), window_rgb(6, 5, 4)]);

        let half = NativeD3d11ReplayReadback {
            width: 1,
            height: 1,
            dxgi_format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            row_pitch: 8,
            bytes: vec![0x00, 0x3c, 0x00, 0x38, 0x00, 0x00, 0x00, 0x3c],
        };
        let frame = native_d3d11_readback_to_window_frame(&half).expect("convert RGBA16F readback");
        assert_eq!(frame.pixels, vec![window_rgb(255, 128, 0)]);
    }
}
