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
const NAME: &str = "BattlePlan Recorder";
const RECORD_AT: usize = 1;
const TOOLTIP_UNITS: usize = 127;
const ICON_SIZE: u32 = 32;

static DRAWN_THEME: AtomicU8 = AtomicU8::new(0);

fn theme(light_taskbar: bool) -> u8 {
    1 + u8::from(light_taskbar)
}

pub struct TrayMenu {
    menu: Menu<Wry>,
    start: MenuItem<Wry>,
    stop: Submenu<Wry>,
    wanted: Mutex<(bool, bool)>,
    stop_shown: AtomicBool,
}

impl TrayMenu {
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
                without_check_column(unsafe {
                    GetSubMenu(self.menu.hpopupmenu()? as HMENU, RECORD_AT as i32)
                });
            }
            self.stop_shown.store(can_stop, Ordering::Relaxed);
        }
        self.start.set_enabled(can_start)
    }
}

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
    if let Some(name) = snapshot
        .name
        .as_ref()
        .filter(|_| snapshot.phase != Phase::Waiting)
    {
        text = format!("{text} · {name}");
    }
    if let Some(hours) = snapshot.behind.filter(|_| snapshot.phase == Phase::Recording) {
        text = format!("{text} · falling behind by {hours} hours");
    }
    if let Some(prompt) = snapshot.prompt {
        let question = match (prompt.quitting, prompt.discarding) {
            (true, true) => "Discard and quit?",
            (false, true) => "Discard this session?",
            (true, false) => "Quit BattlePlan Recorder?",
            (false, false) => "Stop recording?",
        };
        text = format!("{text}\n{question} Click to answer.");
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

fn dot_colour(light: Light) -> [u8; 3] {
    match light {
        Light::Ready => [0x22, 0xb0, 0x5a],
        Light::Recording => [0xef, 0x44, 0x44],
        Light::Saving => [0x2f, 0x6b, 0xff],
        Light::Problem => [0xe8, 0xa3, 0x17],
    }
}

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
    let start = MenuItem::with_id(app, START, "Start recording", false, None::<&str>)?;
    let save = MenuItem::with_id(app, SAVE, "Save", true, None::<&str>)?;
    let discard = MenuItem::with_id(app, DISCARD, "Discard", true, None::<&str>)?;
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
            OPEN => panel::open(app, None, false),
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
            "BattlePlan Recorder\nWaiting for the game"
        );
        let mut snapshot = Snapshot {
            phase: Phase::Recording,
            name: Some("Germany 1936–1943".into()),
            ..Snapshot::default()
        };
        assert_eq!(
            tooltip(&snapshot),
            "BattlePlan Recorder\nRecording · Germany 1936–1943"
        );
        snapshot.problem = Some(crate::snapshot::Problem::caveat(
            "2 hours couldn’t be recorded.",
            "",
        ));
        assert_eq!(
            tooltip(&snapshot),
            "BattlePlan Recorder\nRecording · Germany 1936–1943\n2 hours couldn’t be recorded."
        );
        snapshot.prompt = Some(crate::panel::Prompt { id: 3, quitting: false, discarding: true, session_key: 1 });
        assert_eq!(
            tooltip(&snapshot),
            "BattlePlan Recorder\nRecording · Germany 1936–1943\nDiscard this session? Click to answer.\n2 hours couldn’t be recorded.",
            "a prompt dismissed with its panel waits in the tray"
        );
    }

    #[test]
    fn the_status_line_says_how_far_a_slow_spool_falls_behind_and_clears_when_it_catches_up() {
        let mut snapshot = Snapshot {
            phase: Phase::Recording,
            name: Some("Germany 1936–1943".into()),
            ..Snapshot::default()
        };
        let lines: Vec<String> = [2, 4, 6, 9, 13, 7, 4, 1]
            .into_iter()
            .map(|in_flight: u64| {
                let status = serde_json::json!({
                    "campaign": "00000000-0000-4000-8000-000000000000", "hours": 40,
                    "last_hour": 9,
                    "acquisition_work": {"frames": {
                        "steady": 4, "in_flight": in_flight, "in_flight_bytes": in_flight * 4_096,
                    }},
                });
                snapshot.behind = crate::snapshot::read_live(&status).behind;
                tooltip(&snapshot)
            })
            .collect();
        let quiet = "BattlePlan Recorder\nRecording · Germany 1936–1943";
        let behind = |hours: u64| format!("{quiet} · falling behind by {hours} hours");
        assert_eq!(
            lines,
            [
                quiet.to_owned(),
                quiet.to_owned(),
                behind(6),
                behind(9),
                behind(13),
                behind(7),
                quiet.to_owned(),
                quiet.to_owned(),
            ]
        );
        snapshot.behind = Some(9);
        snapshot.phase = Phase::Saving;
        assert_eq!(tooltip(&snapshot), "BattlePlan Recorder\nSaving recording · Germany 1936–1943");
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
        assert!(text.starts_with("BattlePlan Recorder\nSaved · Nederland 🇳🇱"));
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
            let centre = pixel(&pixels, size, 16, 16);
            assert_eq!(centre, [colour[0], colour[1], colour[2], 255]);
            assert_eq!(pixel(&pixels, size, 16, 16 + 8)[3], 0);
            assert_eq!(pixel(&pixels, size, 16, 16 + 12), [0xd8, 0xd8, 0xd8, 255]);
            assert_eq!(pixel(&pixels, size, 0, 0)[3], 0);
        }
        let light = icon_pixels(Light::Ready, size, true);
        assert_eq!(pixel(&light, size, 16, 16 + 12), [0x3a, 0x3a, 0x3a, 255]);
    }

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
