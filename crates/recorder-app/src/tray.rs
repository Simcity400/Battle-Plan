//! The tray icon, a ring whose dot is the status at a glance, its tooltip and its right-click menu.
use crate::{
    panel,
    snapshot::{Light, Phase, Snapshot},
    worker::Recorder,
};
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{ContextMenu, IsMenuItem, Menu, MenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSubMenu, HMENU, MENUINFO, MIM_STYLE, MNS_NOCHECK, SetMenuInfo,
};

pub const TRAY_ID: &str = "recorder";
const OPEN: &str = "open";
const START: &str = "start";
const SAVE: &str = "save";
const DISCARD: &str = "discard";
const QUIT: &str = "quit";
const NAME: &str = "Battle Plan Recorder";
/// Start or Stop stands second, after Open.
const RECORD_AT: usize = 1;
/// UTF-16 units a notification area tooltip holds before its terminator.
const TOOLTIP_UNITS: usize = 127;
/// Windows scales this to the notification area's size.
const ICON_SIZE: u32 = 32;

/// Last drawn taskbar theme: 0 before drawing, 1 dark, 2 light.
static DRAWN_THEME: AtomicU8 = AtomicU8::new(0);

fn theme(light_taskbar: bool) -> u8 {
    1 + u8::from(light_taskbar)
}

/// Start, or Stop with its Save and Discard, never both; separate ids, so a click does what its
/// item said if the menu changes.
pub struct TrayMenu {
    menu: Menu<Wry>,
    start: MenuItem<Wry>,
    stop: Submenu<Wry>,
    /// The latest (can start, can stop) to show.
    wanted: Mutex<(bool, bool)>,
    /// Stop is in the menu; changed only on the main thread.
    stop_shown: AtomicBool,
}

impl TrayMenu {
    /// On the main thread, one change at a time, so the menu ends on the latest state.
    fn follow(&self) -> tauri::Result<()> {
        let (can_start, can_stop) = *self.wanted.lock().unwrap_or_else(PoisonError::into_inner);
        if self.stop_shown.load(Ordering::Relaxed) != can_stop {
            let (shown, hidden): (&dyn IsMenuItem<Wry>, &dyn IsMenuItem<Wry>) = if can_stop {
                (&self.stop, &self.start)
            } else {
                (&self.start, &self.stop)
            };
            self.menu.remove(hidden)?;
            self.menu.insert(shown, RECORD_AT)?;
            if can_stop {
                // The handle the menu shows the submenu by, not the submenu's own popup handle.
                without_check_column(unsafe {
                    GetSubMenu(self.menu.hpopupmenu()? as HMENU, RECORD_AT as i32)
                });
            }
            self.stop_shown.store(can_stop, Ordering::Relaxed);
        }
        self.start.set_enabled(can_start)
    }
}

/// No item is ever checked, so Windows need not keep a column for check marks.
fn without_check_column(menu: HMENU) {
    let style = MENUINFO {
        cbSize: size_of::<MENUINFO>() as u32,
        fMask: MIM_STYLE,
        dwStyle: MNS_NOCHECK,
        ..MENUINFO::default()
    };
    if unsafe { SetMenuInfo(menu, &style) } == 0 {
        eprintln!(
            "Tray menu kept its check mark column: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// The app, then the status the panel's head shows and the recording's name, then any problem.
pub fn tooltip(snapshot: &Snapshot) -> String {
    let status = match snapshot.phase {
        Phase::Waiting => "Waiting for the game",
        Phase::NotRecording => "Not recording",
        Phase::Recording => "Recording",
        Phase::Saving => "Saving recording",
        Phase::Saved => "Saved",
        Phase::Discarded => "Session discarded",
        Phase::Unsaved => "Not saved yet",
    };
    let mut text = format!("{NAME}\n{status}");
    // Waiting names no campaign, as in the panel.
    if let Some(name) = snapshot
        .name
        .as_ref()
        .filter(|_| snapshot.phase != Phase::Waiting)
    {
        text = format!("{text} · {name}");
    }
    if let Some(problem) = &snapshot.problem {
        text = format!("{text}\n{}", problem.text);
    }
    if text.encode_utf16().count() <= TOOLTIP_UNITS {
        return text;
    }
    let mut units = 0;
    let mut cut: String = text
        .chars()
        .take_while(|character| {
            units += character.len_utf16();
            units < TOOLTIP_UNITS
        })
        .collect();
    cut.push('…');
    cut
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
    // Enabled once the recorder has looked for a game.
    let start = MenuItem::with_id(app, START, "Start recording", false, None::<&str>)?;
    let save = MenuItem::with_id(app, SAVE, "Save", true, None::<&str>)?;
    let discard = MenuItem::with_id(app, DISCARD, "Discard…", true, None::<&str>)?;
    let stop = Submenu::with_items(app, "Stop recording", true, &[&save, &discard])?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &start, &quit])?;
    without_check_column(menu.hpopupmenu()? as HMENU);
    app.manage(TrayMenu {
        menu: menu.clone(),
        start,
        stop,
        wanted: Mutex::new((false, false)),
        stop_shown: AtomicBool::new(false),
    });
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Light::Ready))
        .tooltip(tooltip(&Snapshot::default()))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            OPEN => panel::open(app, None, panel::Opening::Panel),
            START => recorder.record(),
            SAVE => recorder.save(app),
            DISCARD => recorder.ask_discard(app),
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

/// Show the snapshot's light and tooltip, and whether Start or Stop applies.
pub fn show(app: &AppHandle, light: Light, tooltip: &str, can_start: bool, can_stop: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID)
        && let Err(error) = tray
            .set_icon(Some(icon(light)))
            .and_then(|()| tray.set_tooltip(Some(tooltip)))
    {
        eprintln!("Tray icon could not change: {error}");
    }
    let Some(menu) = app.try_state::<TrayMenu>() else {
        return;
    };
    *menu.wanted.lock().unwrap_or_else(PoisonError::into_inner) = (can_start, can_stop);
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        if let Err(error) = handle.state::<TrayMenu>().follow() {
            eprintln!("Tray menu could not change: {error}");
        }
    });
    if let Err(error) = queued {
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
    fn the_tooltip_names_the_status_the_recording_and_any_problem() {
        assert_eq!(
            tooltip(&Snapshot::default()),
            "Battle Plan Recorder\nWaiting for the game"
        );
        let mut snapshot = Snapshot {
            phase: Phase::Recording,
            name: Some("Germany 1936–1943".into()),
            ..Snapshot::default()
        };
        assert_eq!(
            tooltip(&snapshot),
            "Battle Plan Recorder\nRecording · Germany 1936–1943"
        );
        snapshot.problem = Some(crate::snapshot::Problem {
            text: "2 hours couldn't be recorded".into(),
            detail: String::new(),
        });
        assert_eq!(
            tooltip(&snapshot),
            "Battle Plan Recorder\nRecording · Germany 1936–1943\n2 hours couldn't be recorded"
        );
    }

    #[test]
    fn a_long_tooltip_ends_in_an_ellipsis_within_the_notification_areas_limit() {
        let snapshot = Snapshot {
            phase: Phase::Saved,
            name: Some("Nederland 🇳🇱 ".repeat(20)),
            ..Snapshot::default()
        };
        let text = tooltip(&snapshot);
        assert!(text.encode_utf16().count() <= TOOLTIP_UNITS);
        assert!(text.starts_with("Battle Plan Recorder\nSaved · Nederland 🇳🇱"));
        assert!(text.ends_with('…'));
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
