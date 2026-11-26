
use super::{
    Lost,
    window::{Input, Inputs, Window},
};
use crate::{app, render, renderer_host};
use eframe::{App as _, egui, egui_wgpu, wgpu};
use std::{
    sync::{Arc, mpsc::RecvTimeoutError},
    time::{Duration, Instant},
};

pub(crate) fn map_device(window: Option<&Window>) -> Result<(egui_wgpu::RenderState, Option<wgpu::Surface<'static>>), String> {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
    let configuration = render::configuration();
    let surface_config = configuration.surface;
    let egui_wgpu::WgpuSetup::CreateNew(setup) = configuration.wgpu_setup else {
        return Err("The map’s device setup isn’t its own".into());
    };
    let lifecycle = shader_pipeline::gpu::device_lifecycle();
    let instance = shader_pipeline::gpu::instance();
    let surface = match window {
        Some(window) => {
            let handle = std::num::NonZeroIsize::new(window.hwnd() as isize).ok_or("The map’s window has no handle")?;
            // SAFETY: the window outlives the surface: a close asks the map's thread to stop, and the
            // window is destroyed only once that thread has dropped the surface (`Window::destroy`).
            Some(
                unsafe {
                    instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                        raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(handle)),
                    })
                }
                .map_err(|error| format!("Couldn’t draw in the map’s window: {error}"))?,
            )
        }
        None => None,
    };
    pollster::block_on(async {
        let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = match &setup.native_adapter_selector {
            Some(select) => select(&available_adapters, surface.as_ref())?,
            None => return Err("The map names no adapter choice".to_owned()),
        };
        let (device, queue) = adapter
            .request_device(&(setup.device_descriptor)(&adapter))
            .await
            .map_err(|error| format!("Couldn’t use the graphics device: {error}"))?;
        drop(lifecycle);
        let target_format = match &surface {
            Some(surface) => egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
                .map_err(|error| format!("Couldn’t draw in the map’s window: {error}"))?,
            None => viewer_gpu::gpu_contract::OUTPUT_FORMAT,
        };
        let renderer = egui_wgpu::Renderer::new(&device, target_format, Default::default());
        Ok((
            egui_wgpu::RenderState {
                instance,
                adapter,
                available_adapters,
                device,
                queue,
                target_format,
                renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
                surface_config,
            },
            surface,
        ))
    })
}

struct Released<T>(Option<T>);

impl<T> std::ops::Deref for Released<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("held until dropped")
    }
}

impl<T> std::ops::DerefMut for Released<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("held until dropped")
    }
}

impl<T> Drop for Released<T> {
    fn drop(&mut self) {
        let _held = shader_pipeline::gpu::device_lifecycle();
        self.0.take();
    }
}

pub(crate) fn run(
    window: &Window,
    inputs: Inputs,
    lost: &Lost,
    make: impl FnOnce(&egui::Context, &egui_wgpu::RenderState) -> Result<app::Viewer, String>,
) -> Result<(), String> {
    let _followed = viewer_runtime::display_refresh::follow_window(window.hwnd());
    let (state, surface) = map_device(Some(window))?;
    let state = Released(Some(state));
    let mut surface = Released(Some(renderer_host::WindowSurface::new(surface.ok_or("The map’s window gave no surface")?)));
    renderer_host::bind(&state);
    let ctx = egui::Context::default();
    let link = Arc::clone(window.link());
    ctx.set_request_repaint_callback(move |info| link.send(Input::Repaint(info.delay)));
    let repaint = ctx.clone();
    lost.watch(&state.device, move || repaint.request_repaint());
    let mut viewer = make(&ctx, &state)?;
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(state.clone());
    let started = Instant::now();
    let mut due = Some(Instant::now());
    let mut limited = viewer_runtime::frame_limit::Grid::default();
    let mut focused = false;
    let mut events = Vec::new();
    let max_texture_side = state.device.limits().max_texture_dimension_2d as usize;
    loop {
        let input = match due {
            Some(at) => match inputs.received.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(input) => Some(input),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            },
            None => match inputs.received.recv() {
                Ok(input) => Some(input),
                Err(_) => return Ok(()),
            },
        };
        let mut closed = false;
        for input in input.into_iter().chain(inputs.received.try_iter()) {
            match input {
                Input::Event(event) => {
                    events.push(event);
                    due = Some(Instant::now());
                }
                Input::Focus(now) => {
                    focused = now;
                    events.push(egui::Event::WindowFocused(now));
                    due = Some(Instant::now());
                }
                Input::Resized => due = Some(Instant::now()),
                Input::Repaint(delay) => {
                    let at = Instant::now() + delay;
                    due = Some(due.map_or(at, |due| due.min(at)));
                }
                Input::Close => closed = true,
            }
        }
        if closed {
            return Ok(());
        }
        if due.is_none_or(|at| at > Instant::now()) {
            continue;
        }
        if lost.reason().is_some() {
            return Ok(());
        }
        let waiting = limited.until(Instant::now());
        if !waiting.is_zero() {
            due = Some(Instant::now() + waiting);
            continue;
        }
        limited.take(Instant::now());
        due = None;
        let (size, pixels_per_point) = window.metrics();
        let rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0] as f32, size[1] as f32) / pixels_per_point,
        );
        let mut raw = egui::RawInput {
            screen_rect: Some(rect),
            max_texture_side: Some(max_texture_side),
            time: Some(started.elapsed().as_secs_f64()),
            focused,
            events: std::mem::take(&mut events),
            ..Default::default()
        };
        let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.native_pixels_per_point = Some(pixels_per_point);
        viewport.inner_rect = Some(rect);
        viewport.focused = Some(focused);
        viewer.raw_input_hook(&ctx, &mut raw);
        let output = ctx.run_ui(raw, |ui| {
            viewer.logic(ui.ctx(), &mut frame);
            viewer.ui(ui, &mut frame);
        });
        window.cursor(output.platform_output.cursor_icon);
        let mut close = false;
        if let Some(root) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            for command in &root.commands {
                match command {
                    egui::ViewportCommand::Close => close = true,
                    egui::ViewportCommand::CursorGrab(grab) => window.capture(*grab != egui::CursorGrab::None),
                    _ => {}
                }
            }
            if root.repaint_delay < Duration::MAX {
                let at = Instant::now() + root.repaint_delay;
                due = Some(due.map_or(at, |due| due.min(at)));
            }
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let mut textures = output.textures_delta;
        let config = frame.wgpu_render_state.as_ref().map_or(state.surface_config, |state| state.surface_config);
        let clear = viewer.clear_color(&ctx.style_of(egui::Theme::Dark).visuals);
        let painting = std::time::Instant::now();
        let shown = window.shown() && size[0] > 0 && size[1] > 0;
        let acquire = surface.paint(&state, config, shown.then_some(size), output.pixels_per_point, clear, &primitives, &mut textures)?.acquire;
        if shown {
            crate::perf_log::frame_painted(painting.elapsed(), acquire);
        }
        if close {
            return Ok(());
        }
    }
}

pub(crate) fn run_headless(
    size: [u32; 2],
    lost: &Lost,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    make: impl FnOnce(&egui::Context, &egui_wgpu::RenderState) -> Result<app::Viewer, String>,
) -> Result<(), String> {
    let (state, _) = map_device(None)?;
    let state = Released(Some(state));
    renderer_host::bind(&state);
    let ctx = egui::Context::default();
    lost.watch(&state.device, || {});
    let mut viewer = make(&ctx, &state)?;
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(state.clone());
    let started = Instant::now();
    let max_texture_side = state.device.limits().max_texture_dimension_2d as usize;
    let rect = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(size[0] as f32, size[1] as f32),
    );
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            viewer.cancel_video_export();
            return Ok(());
        }
        if let Some(reason) = lost.reason() {
            viewer.cancel_video_export();
            return Err(reason);
        }
        let mut raw = egui::RawInput {
            screen_rect: Some(rect),
            max_texture_side: Some(max_texture_side),
            time: Some(started.elapsed().as_secs_f64()),
            focused: false,
            events: vec![egui::Event::PointerGone],
            ..Default::default()
        };
        let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.native_pixels_per_point = Some(1.0);
        viewport.inner_rect = Some(rect);
        viewport.focused = Some(false);
        viewer.raw_input_hook(&ctx, &mut raw);
        let output = ctx.run_ui(raw, |ui| {
            viewer.logic(ui.ctx(), &mut frame);
            viewer.ui(ui, &mut frame);
        });
        let close = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|root| {
                root.commands
                    .iter()
                    .any(|command| matches!(command, egui::ViewportCommand::Close))
            });
        if close {
            return Ok(());
        }
        if viewer.video_export_idle() {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
