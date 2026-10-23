//! The tray icon, a ring whose dot is the only status, and its right-click menu.
use crate::{panel, snapshot::Light, worker::Recorder};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub const TRAY_ID: &str = "recorder";
const OPEN: &str = "open";
const START: &str = "start";
const STOP: &str = "stop";
const QUIT: &str = "quit";
/// Windows scales this to the notification area's size.
const ICON_SIZE: u32 = 32;

/// Last drawn taskbar theme: 0 before drawing, 1 dark, 2 light.
static DRAWN_THEME: AtomicU8 = AtomicU8::new(0);

fn theme(light_taskbar: bool) -> u8 {
    1 + u8::from(light_taskbar)
}

/// The menu's Start and Stop items, each enabled only when it applies.
pub struct TrayMenu {
    start: MenuItem<Wry>,
    stop: MenuItem<Wry>,
}

/// Mid-tone shades that stand out from both a light and a dark taskbar.
fn dot_colour(light: Light) -> [u8; 3] {
    match light {
        Light::Ready => [0x22, 0xb0, 0x5a],
        Light::Recording => [0xef, 0x44, 0x44],
        Light::Saving => [0x2f, 0x6b, 0xff],
        Light::Problem => [0xe8, 0xa3, 0x17],
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
    let open = MenuItem::with_id(app, OPEN, "Open", true, None::<&str>)?;
    let start = MenuItem::with_id(app, START, "Start recording", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, STOP, "Stop recording", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &start, &stop, &quit])?;
    app.manage(TrayMenu { start, stop });
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Light::Ready))
        .tooltip("Battle Plan Recorder")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            OPEN => panel::open(app, None, panel::Opening::Panel),
            START => recorder.record(),
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

/// Show the snapshot's light and which of Start and Stop apply.
pub fn show(app: &AppHandle, light: Light, can_start: bool, can_stop: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID)
        && let Err(error) = tray.set_icon(Some(icon(light)))
    {
        eprintln!("Tray icon could not change: {error}");
    }
    if let Some(menu) = app.try_state::<TrayMenu>()
        && let Err(error) = menu
            .start
            .set_enabled(can_start)
            .and_then(|()| menu.stop.set_enabled(can_stop))
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
            (Light::Ready, [0x22, 0xb0, 0x5a]),
            (Light::Recording, [0xef, 0x44, 0x44]),
            (Light::Saving, [0x2f, 0x6b, 0xff]),
            (Light::Problem, [0xe8, 0xa3, 0x17]),
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
        let light = icon_pixels(Light::Ready, size, true);
        assert_eq!(pixel(&light, size, 16, 16 + 12), [0x3a, 0x3a, 0x3a, 255]);
    }

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(colour: [u8; 3]) -> f64 {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(colour[0]) + 0.7152 * linear(colour[1]) + 0.0722 * linear(colour[2])
    }

    #[test]
    fn each_status_colour_stands_out_on_a_light_and_a_dark_taskbar() {
        let contrast = |a: f64, b: f64| (a.max(b) + 0.05) / (a.min(b) + 0.05);
        // Windows 11's light and dark taskbars.
        let (light, dark) = (luminance([0xf3, 0xf3, 0xf3]), luminance([0x20, 0x20, 0x20]));
        let lights = [
            Light::Ready,
            Light::Recording,
            Light::Saving,
            Light::Problem,
        ];
        for status in lights {
            let dot = luminance(dot_colour(status));
            assert!(contrast(dot, light) >= 1.9, "{status:?} on a light taskbar");
            assert!(contrast(dot, dark) >= 3.0, "{status:?} on a dark taskbar");
        }
        // Four colours, none mistaken for another.
        for (at, first) in lights.iter().enumerate() {
            for second in &lights[at + 1..] {
                let [a, b] = [dot_colour(*first), dot_colour(*second)];
                let distance: i32 = (0..3)
                    .map(|channel| (i32::from(a[channel]) - i32::from(b[channel])).abs())
                    .sum();
                assert!(distance >= 120, "{first:?} and {second:?}");
            }
        }
    }
}
