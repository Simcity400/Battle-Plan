//! The tray icon, drawn at run time as a ring with a coloured dot, its tooltip
//! and its right-click menu.
use crate::{panel, snapshot::Light, worker::Recorder};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub const TRAY_ID: &str = "recorder";
const OPEN: &str = "open";
const STOP: &str = "stop";
const QUIT: &str = "quit";
/// Windows scales this to the notification area's size.
const ICON_SIZE: u32 = 32;

/// Last drawn taskbar theme: 0 before drawing, 1 dark, 2 light.
static DRAWN_THEME: AtomicU8 = AtomicU8::new(0);

fn theme(light_taskbar: bool) -> u8 {
    1 + u8::from(light_taskbar)
}

/// The menu's Stop item, enabled only while recording.
pub struct TrayMenu(MenuItem<Wry>);

fn dot_colour(light: Light) -> [u8; 3] {
    match light {
        Light::Idle => [0x8a, 0x8a, 0x8a],
        Light::Recording => [0xef, 0x44, 0x44],
        Light::Problem => [0xf0, 0xb4, 0x4c],
        Light::Busy => [0x2f, 0x6b, 0xff],
    }
}

/// RGBA pixels of a `size`-pixel ring around a dot, in the design's 16-pixel
/// proportions: an 8-pixel ring radius with a 2-pixel stroke and a 3-pixel dot.
/// Each pixel samples a 4x4 grid so the edges stay smooth.
pub fn icon_pixels(light: Light, size: u32, light_taskbar: bool) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let ring: [u8; 3] = if light_taskbar {
        [0x3a, 0x3a, 0x3a]
    } else {
        [0xd8, 0xd8, 0xd8]
    };
    let dot = dot_colour(light);
    let scale = f64::from(size) / 16.0;
    let centre = f64::from(size) / 2.0;
    // Half a design pixel inside the edge, so the ring is not clipped.
    let outer = 7.5 * scale;
    let inner = outer - 2.0 * scale;
    let dot_radius = 3.0 * scale;
    let mut pixels = vec![0; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (mut ring_cover, mut dot_cover) = (0_u32, 0_u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let dx = f64::from(x) + (f64::from(sx) + 0.5) / f64::from(SAMPLES) - centre;
                    let dy = f64::from(y) + (f64::from(sy) + 0.5) / f64::from(SAMPLES) - centre;
                    let distance = (dx * dx + dy * dy).sqrt();
                    if distance <= outer && distance >= inner {
                        ring_cover += 1;
                    } else if distance <= dot_radius {
                        dot_cover += 1;
                    }
                }
            }
            let cover = ring_cover + dot_cover;
            if cover == 0 {
                continue;
            }
            let at = ((y * size + x) * 4) as usize;
            for channel in 0..3 {
                pixels[at + channel] = ((u32::from(ring[channel]) * ring_cover
                    + u32::from(dot[channel]) * dot_cover)
                    / cover) as u8;
            }
            pixels[at + 3] = (cover * 255 / (SAMPLES * SAMPLES)) as u8;
        }
    }
    pixels
}

fn icon(light: Light) -> Image<'static> {
    let light_taskbar = crate::startup::light_taskbar();
    DRAWN_THEME.store(theme(light_taskbar), Ordering::Relaxed);
    Image::new_owned(
        icon_pixels(light, ICON_SIZE, light_taskbar),
        ICON_SIZE,
        ICON_SIZE,
    )
}

/// Called on unchanged status polls too, since the taskbar theme can change independently.
pub fn follow_theme(app: &AppHandle, light: Light) {
    let drawn = DRAWN_THEME.load(Ordering::Relaxed);
    if drawn == 0 || drawn == theme(crate::startup::light_taskbar()) {
        return;
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID)
        && let Err(error) = tray.set_icon(Some(icon(light)))
    {
        eprintln!("Tray icon could not change: {error}");
    }
}

pub fn create(app: &AppHandle, recorder: Arc<Recorder>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, OPEN, "Open recorder", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, STOP, "Stop recording", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &stop, &separator, &quit])?;
    app.manage(TrayMenu(stop));
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Light::Idle))
        .tooltip("Battle Plan Recorder")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            OPEN => panel::open(app, None, false),
            STOP => recorder.ask_stop(app),
            QUIT => recorder.quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                panel::toggle(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    Ok(())
}

/// Show the snapshot's light, tooltip and whether Stop is available.
pub fn show(app: &AppHandle, light: Light, tooltip: &str, can_stop: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Err(error) = tray.set_icon(Some(icon(light))) {
            eprintln!("Tray icon could not change: {error}");
        }
        if let Err(error) = tray.set_tooltip(Some(tooltip)) {
            eprintln!("Tray tooltip could not change: {error}");
        }
    }
    if let Some(menu) = app.try_state::<TrayMenu>()
        && let Err(error) = menu.0.set_enabled(can_stop)
    {
        eprintln!("Tray menu could not change: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        pixels[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn icon_draws_a_ring_and_a_coloured_dot_on_a_clear_background() {
        let size = 32;
        for (light, colour) in [
            (Light::Idle, [0x8a, 0x8a, 0x8a]),
            (Light::Recording, [0xef, 0x44, 0x44]),
            (Light::Problem, [0xf0, 0xb4, 0x4c]),
            (Light::Busy, [0x2f, 0x6b, 0xff]),
        ] {
            let pixels = icon_pixels(light, size, false);
            assert_eq!(pixels.len(), (size * size * 4) as usize);
            // The centre is the dot's colour, fully opaque.
            let centre = pixel(&pixels, size, 16, 16);
            assert_eq!(centre, [colour[0], colour[1], colour[2], 255]);
            // Between the dot and the ring is clear.
            assert_eq!(pixel(&pixels, size, 16, 16 + 8)[3], 0);
            // The ring is light on a dark taskbar.
            assert_eq!(pixel(&pixels, size, 16, 16 + 12), [0xd8, 0xd8, 0xd8, 255]);
            // The corners stay clear.
            assert_eq!(pixel(&pixels, size, 0, 0)[3], 0);
        }
        let light = icon_pixels(Light::Idle, size, true);
        assert_eq!(pixel(&light, size, 16, 16 + 12), [0x3a, 0x3a, 0x3a, 255]);
    }
}
