//! The acceptance suite's input channel (tools/e2e). Off unless `BATTLE_PLAN_E2E_INPUT` names a loopback
//! address, which only the suite's own profile sets; it has no UI and is never on for users.
//! The suite also fixes exported maps' animation clock (`BATTLE_PLAN_E2E_CAPTURE_CLOCK`, seconds), so
//! its pictures repeat exactly; without the variable an export draws the live animation.
//!
//! The suite runs the app on a desktop of its own, where the windowing library sees no cursor
//! over the window, so real pointer input cannot reach the map there. Instead the suite sends one
//! JSON line per event and they join egui's raw input, where the windowing library's translated
//! events arrive: everything after (hit-tests, hover, camera, Fly and Walk) runs as for a user.
//! Only the windowing library's own translation is untested.
//!
//! Lines, with points in the window's client pixels:
//! `{"move":[x,y]}`, `{"wheel":lines}` (where the pointer last moved, as a wheel turns where the
//! cursor rests), `{"button":"primary|middle|secondary","at":[x,y],"pressed":b}`, and
//! `{"key":"ArrowRight","pressed":b}` (an egui key name).

use eframe::egui;
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpListener},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum Line {
    Move { r#move: [f32; 2] },
    Wheel { wheel: f32 },
    Button { button: String, at: [f32; 2], pressed: bool },
    Key { key: String, pressed: bool },
}

/// Lines received and not yet given to egui.
static PENDING: Mutex<Vec<Line>> = Mutex::new(Vec::new());
/// Whether this process serves the suite: decided once, from the variable.
static ENABLED: OnceLock<bool> = OnceLock::new();
/// Whether the suite has sent input: from then on the window has the focus a user's would.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The animation clock a map export draws at, when the suite fixes one.
pub(crate) fn capture_clock() -> Option<f32> {
    static CLOCK: OnceLock<Option<f32>> = OnceLock::new();
    *CLOCK.get_or_init(|| std::env::var("BATTLE_PLAN_E2E_CAPTURE_CLOCK").or_else(|_| std::env::var("HOI4_E2E_CAPTURE_CLOCK")).ok()?.parse().ok())
}

fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        let Some(address) = std::env::var("BATTLE_PLAN_E2E_INPUT").or_else(|_| std::env::var("HOI4_E2E_INPUT")).ok() else {
            return false;
        };
        let Ok(address) = address.parse::<SocketAddr>() else {
            eprintln!("BATTLE_PLAN_E2E_INPUT {address}: not an address, ignored");
            return false;
        };
        // Loopback only: the channel never listens beyond this machine.
        if !address.ip().is_loopback() {
            eprintln!("BATTLE_PLAN_E2E_INPUT {address}: not a loopback address, ignored");
            return false;
        }
        // Another viewer of the same app may hold the address; only one serves the suite.
        let Ok(listener) = TcpListener::bind(address) else {
            return false;
        };
        std::thread::Builder::new()
            .name("suite input".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    for line in BufReader::new(stream).lines().map_while(Result::ok) {
                        match serde_json::from_str::<Line>(&line) {
                            Ok(line) => PENDING.lock().unwrap_or_else(|error| error.into_inner()).push(line),
                            Err(error) => eprintln!("BATTLE_PLAN_E2E_INPUT line refused: {error}"),
                        }
                    }
                }
            })
            .is_ok()
    })
}

/// Appends the suite's events to this frame's raw input, as egui's own event types.
pub(crate) fn apply(ctx: &egui::Context, input: &mut egui::RawInput) {
    if !enabled() {
        return;
    }
    let lines = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|error| error.into_inner()));
    if !lines.is_empty() {
        ACTIVE.store(true, Ordering::Relaxed);
    }
    let scale = ctx.pixels_per_point();
    for line in lines {
        if let Some(event) = event(line, scale) {
            input.events.push(event);
        }
    }
    // A window a user types into and points at keeps its focus between their inputs, so held keys
    // and captured input last until released.
    if ACTIVE.load(Ordering::Relaxed) {
        input.focused = true;
    }
}

fn event(line: Line, scale: f32) -> Option<egui::Event> {
    let point = |[x, y]: [f32; 2]| egui::pos2(x / scale, y / scale);
    let modifiers = egui::Modifiers::default();
    Some(match line {
        Line::Move { r#move } => egui::Event::PointerMoved(point(r#move)),
        Line::Wheel { wheel } => egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, wheel),
            phase: egui::TouchPhase::Move,
            modifiers,
        },
        Line::Button { button, at, pressed } => egui::Event::PointerButton {
            pos: point(at),
            button: match button.as_str() {
                "primary" => egui::PointerButton::Primary,
                "middle" => egui::PointerButton::Middle,
                "secondary" => egui::PointerButton::Secondary,
                _ => return None,
            },
            pressed,
            modifiers,
        },
        Line::Key { key, pressed } => {
            let key = egui::Key::from_name(&key)?;
            egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_egui_events_in_points() {
        let parse = |text: &str| event(serde_json::from_str(text).unwrap(), 2.0);
        assert_eq!(parse(r#"{"move":[100,40]}"#), Some(egui::Event::PointerMoved(egui::pos2(50.0, 20.0))));
        assert!(matches!(parse(r#"{"wheel":-3}"#),
            Some(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. }) if delta == egui::vec2(0.0, -3.0)));
        assert!(matches!(parse(r#"{"button":"middle","at":[10,10],"pressed":true}"#),
            Some(egui::Event::PointerButton { button: egui::PointerButton::Middle, pressed: true, .. })));
        assert!(matches!(parse(r#"{"key":"ArrowRight","pressed":false}"#),
            Some(egui::Event::Key { key: egui::Key::ArrowRight, pressed: false, .. })));
        assert_eq!(parse(r#"{"key":"NoSuchKey","pressed":true}"#), None);
        assert!(serde_json::from_str::<Line>(r#"{"jump":1}"#).is_err());
    }
}
