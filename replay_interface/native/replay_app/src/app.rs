//! Product window loop: input, timeline scrub, picking, rendering.

use std::path::PathBuf;
use std::time::Instant;

use map_viewer::GameFileResolver;
use winapi::um::winuser::{VK_ESCAPE, VK_LEFT, VK_OEM_4, VK_OEM_6, VK_RIGHT, VK_SPACE};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::overlay::{self, Overlay};
use crate::render::{
    playback_day_night_hour_from_timeline_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE,
};
use crate::timeline::{build_provider, OwnerProvider};
use crate::win32::{MouseButton, Window, WindowEvent};
use crate::{Result, TimelineMode};

// Interaction tuning constants (not visual claims).
const KEY_PAN_SCREEN_PX_PER_SEC: f64 = 900.0;
const WHEEL_ZOOM_STEP: f64 = 1.2;
/// Autoplay speed: ~4 timeline hours per wall-clock second.
const AUTOPLAY_SECONDS_PER_HOUR: f64 = 0.25;
const MAX_FRAME_DT: f64 = 0.25;

const VK_A: i32 = 0x41;
const VK_D: i32 = 0x44;
const VK_N: i32 = 0x4E;
const VK_R: i32 = 0x52;
const VK_S: i32 = 0x53;
const VK_W: i32 = 0x57;

#[derive(Default)]
struct PanKeys {
    west: bool,
    east: bool,
    north: bool,
    south: bool,
}

struct DragState {
    start_cursor: (i32, i32),
    start_center: (f64, f64),
}

pub fn run(
    bundle: MapBundle,
    mode: TimelineMode,
    game_resolver: Option<GameFileResolver>,
    capture_dir: Option<PathBuf>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    water_full: bool,
) -> Result<()> {
    let (provider, mode_label) = build_provider(&bundle, &mode)?;
    let hours: Vec<i64> = provider.hours().to_vec();
    if hours.is_empty() {
        return Err("timeline provider has no hours".into());
    }
    // The state->color table is sized by the bundle: a state id outside the
    // bundle's province->state mapping can never be displayed, and the
    // palette builder skips owner rows beyond the table.
    let max_state_id = bundle.max_state_id();

    let mut window = Window::create("replay_app", 1280, 720)?;
    let (client_w, client_h) = window.client_size()?;
    let mut gfx = Gfx::create(window.hwnd(), client_w, client_h)?;
    let mut renderer = MapRenderer::new(
        &gfx,
        &bundle,
        max_state_id,
        capture_dir.as_deref(),
        game_resolver.as_ref(),
        postfx_enabled,
        postfx_restore_full,
        water_full,
    )?;
    // Fail-open UI, fail-closed content: a missing game install disables
    // overlay text (one stderr warning), never substitutes a font.
    let mut overlay = Overlay::create(&gfx, game_resolver.as_ref());
    let mut camera = Camera::fit(
        bundle.width,
        bundle.height,
        f64::from(gfx.width),
        f64::from(gfx.height),
    );

    let mut hour_index = 0usize;
    let mut uploaded_hour_index: Option<usize> = None;
    // Day/night terminator shading is OFF by default, matching the game's
    // default map view (the captured CB0 gate is off); 'N' toggles it.
    let mut day_night_enabled = false;
    let mut autoplay = false;
    let mut autoplay_accum = 0.0f64;
    let mut pan_keys = PanKeys::default();
    let mut drag: Option<DragState> = None;
    let mut scrub_drag = false;
    let mut cursor: Option<(i32, i32)> = None;
    let mut closing = false;
    let mut last_frame = Instant::now();

    loop {
        let (events, alive) = window.pump();
        if !alive {
            break;
        }
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f64().min(MAX_FRAME_DT);
        last_frame = now;

        let screen_w = f64::from(gfx.width);
        let screen_h = f64::from(gfx.height);
        for event in events {
            match event {
                WindowEvent::Resized { width, height } => {
                    gfx.resize(width, height)?;
                    camera.normalize(f64::from(gfx.height));
                }
                WindowEvent::KeyDown { vk } => match vk {
                    VK_ESCAPE => {
                        closing = true;
                        window.request_close();
                    }
                    VK_SPACE => {
                        autoplay = !autoplay;
                        autoplay_accum = 0.0;
                    }
                    VK_LEFT | VK_OEM_4 => {
                        hour_index = hour_index.saturating_sub(1);
                    }
                    VK_RIGHT | VK_OEM_6 => {
                        hour_index = (hour_index + 1).min(hours.len() - 1);
                    }
                    VK_R => {
                        camera.reset_fit(screen_w, screen_h);
                    }
                    VK_N => {
                        day_night_enabled = !day_night_enabled;
                    }
                    VK_W => pan_keys.north = true,
                    VK_S => pan_keys.south = true,
                    VK_A => pan_keys.west = true,
                    VK_D => pan_keys.east = true,
                    _ => {}
                },
                WindowEvent::KeyUp { vk } => match vk {
                    VK_W => pan_keys.north = false,
                    VK_S => pan_keys.south = false,
                    VK_A => pan_keys.west = false,
                    VK_D => pan_keys.east = false,
                    _ => {}
                },
                WindowEvent::MouseMove { x, y } => {
                    cursor = Some((x, y));
                    if scrub_drag {
                        if let Some(fraction) =
                            overlay::scrub_drag_fraction(x as f32, screen_w as f32, screen_h as f32)
                        {
                            hour_index = overlay::index_for_fraction(fraction, hours.len());
                        }
                    }
                    if let Some(state) = &drag {
                        let dx = f64::from(x - state.start_cursor.0);
                        let dy = f64::from(y - state.start_cursor.1);
                        camera.center_x = state.start_center.0 - dx / camera.zoom;
                        camera.center_y = state.start_center.1 - dy / camera.zoom;
                        camera.normalize(screen_h);
                    }
                }
                WindowEvent::MouseDown { button, x, y } => match button {
                    // Left click/drag on the scrub bar seeks the timeline
                    // (which rewrites only the state->color table below).
                    MouseButton::Left => {
                        if let Some(fraction) = overlay::scrub_fraction_at(
                            x as f32,
                            y as f32,
                            screen_w as f32,
                            screen_h as f32,
                        ) {
                            hour_index = overlay::index_for_fraction(fraction, hours.len());
                            scrub_drag = true;
                        }
                    }
                    MouseButton::Middle | MouseButton::Right => {
                        drag = Some(DragState {
                            start_cursor: (x, y),
                            start_center: (camera.center_x, camera.center_y),
                        });
                    }
                },
                WindowEvent::MouseUp { button } => {
                    if button == MouseButton::Left {
                        scrub_drag = false;
                    } else {
                        drag = None;
                    }
                }
                WindowEvent::Wheel { delta, x, y } => {
                    let notches = f64::from(delta) / 120.0;
                    let factor = WHEEL_ZOOM_STEP.powf(notches);
                    camera.zoom_at(
                        f64::from(x) + 0.5,
                        f64::from(y) + 0.5,
                        screen_w,
                        screen_h,
                        factor,
                    );
                    // A wheel drag would fight the zoom re-anchoring.
                    drag = None;
                }
                WindowEvent::CaptureLost => {
                    drag = None;
                    scrub_drag = false;
                }
            }
        }
        if closing {
            continue;
        }

        // Keyboard pan (WASD; arrows are reserved for timeline stepping).
        let pan_x = (f64::from(pan_keys.east as u8) - f64::from(pan_keys.west as u8))
            * KEY_PAN_SCREEN_PX_PER_SEC
            * dt;
        let pan_y = (f64::from(pan_keys.south as u8) - f64::from(pan_keys.north as u8))
            * KEY_PAN_SCREEN_PX_PER_SEC
            * dt;
        if pan_x != 0.0 || pan_y != 0.0 {
            camera.pan_screen(pan_x, pan_y, f64::from(gfx.height));
        }

        // Autoplay: wraps around recorded hours.
        if autoplay && hours.len() > 1 {
            autoplay_accum += dt;
            while autoplay_accum >= AUTOPLAY_SECONDS_PER_HOUR {
                autoplay_accum -= AUTOPLAY_SECONDS_PER_HOUR;
                hour_index = (hour_index + 1) % hours.len();
            }
        }

        // Scrub: validate replay/controller color inputs only when their
        // signatures change. Visible political recoloring stays blocked until
        // the engine's primary fill and GradientBorder generation law is ported.
        let hour = hours[hour_index];
        if uploaded_hour_index != Some(hour_index) {
            let owners = provider.owners_at(hour);
            let controllers = provider.controllers_at(hour);
            renderer.update_state_politics(&gfx, &owners, &controllers, &bundle.country_colors)?;
            uploaded_hour_index = Some(hour_index);
        }

        // CPU picking from the province-id mirror (never from pixels).
        let probe = match cursor {
            Some((x, y)) => {
                let map_pos = renderer.screen_to_map(
                    &camera,
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                    f64::from(gfx.width),
                    f64::from(gfx.height),
                )?;
                probe_at(&bundle, provider.as_ref(), hour, &camera, map_pos)
            }
            None => String::default(),
        };
        let playing = if autoplay { " [playing]" } else { "" };
        let hour_label = provider.hour_label(hour);
        // The readout lives in the in-window overlay now; the title only
        // repeats it while overlay text is unavailable (fonts not loaded),
        // so no information is silently lost.
        if overlay.has_text() {
            window.set_title(&format!("replay_app - {mode_label}"));
        } else {
            window.set_title(&format!(
                "replay_app - {mode_label} | {hour_label}{playing} | {probe} | zoom {:.2}",
                camera.zoom
            ));
        }

        if gfx.width > 0 && gfx.height > 0 {
            let day_night_hour =
                day_night_enabled.then(|| playback_day_night_hour_from_timeline_hour(hour));
            renderer.render(&gfx, &camera, day_night_hour, None)?;
            // Overlay after the map: date/status text in the real game
            // font plus the scrub bar. Line 1 is the raw Clausewitz date
            // string from the recording (GAP: localisation-formatted dates
            // like the in-game clock are a later slice).
            let line2 =
                format!("{mode_label}{playing} | {POLITICAL_COLOR_BLOCKER_BADGE} | {probe}");
            overlay.draw(&gfx, &hour_label, &line2, hour_index, hours.len())?;
            // Present never blocks (PresentOutcome): a full queue or a
            // fully-occluded window turns into a short sleep with the
            // message pump still alive, instead of freezing all input.
            match gfx.present()? {
                crate::d3d::PresentOutcome::Presented => {}
                crate::d3d::PresentOutcome::Skipped => {
                    std::thread::sleep(std::time::Duration::from_millis(4));
                }
                crate::d3d::PresentOutcome::Occluded => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
    Ok(())
}

fn probe_at(
    bundle: &MapBundle,
    provider: &dyn OwnerProvider,
    hour: i64,
    camera: &Camera,
    map_pos: Option<(f64, f64)>,
) -> String {
    // Captured-pass picking intersects the same camera ray with y=0. the game
    // terrain-height refinement is still a named gap; province lookup remains
    // from the CPU province-id mirror rather than pixels.
    let Some((map_x, map_y)) = map_pos else {
        return String::from("prov - state - owner - controller -");
    };
    if map_y < 0.0 || map_y >= bundle.height as f64 {
        return String::from("prov - state - owner - controller -");
    }
    let x = camera.wrap_x(map_x) as usize;
    let y = map_y as usize;
    if x >= bundle.width || y >= bundle.height {
        return String::from("prov - state - owner - controller -");
    }
    let province = bundle.province_ids[y * bundle.width + x];
    let state = bundle.province_to_state.get(&province);
    let owners = provider.owners_at(hour);
    let controllers = provider.controllers_at(hour);
    let owner = state.and_then(|state_id| owners.get(state_id));
    let controller = state.and_then(|state_id| controllers.get(state_id));
    format!(
        "prov {} state {} owner {} controller {}",
        province,
        state.map_or_else(|| String::from("-"), u32::to_string),
        owner.map_or("-", String::as_str),
        controller.map_or("-", String::as_str)
    )
}
