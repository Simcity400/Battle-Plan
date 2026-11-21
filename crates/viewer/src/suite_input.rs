
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
    Modifiers { ctrl: bool, shift: bool, alt: bool },
}

static PENDING: Mutex<Vec<Line>> = Mutex::new(Vec::new());
static ENABLED: OnceLock<bool> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELD: Mutex<egui::Modifiers> = Mutex::new(egui::Modifiers::NONE);

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
        if !address.ip().is_loopback() {
            eprintln!("BATTLE_PLAN_E2E_INPUT {address}: not a loopback address, ignored");
            return false;
        }
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

pub(crate) fn apply(ctx: &egui::Context, input: &mut egui::RawInput) {
    if !enabled() {
        return;
    }
    let lines = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|error| error.into_inner()));
    if !lines.is_empty() {
        ACTIVE.store(true, Ordering::Relaxed);
    }
    let scale = ctx.pixels_per_point();
    let mut held = HELD.lock().unwrap_or_else(|error| error.into_inner());
    for line in lines {
        if let Line::Modifiers { ctrl, shift, alt } = line {
            *held = egui::Modifiers { alt, ctrl, shift, mac_cmd: false, command: ctrl };
            input.events.push(egui::Event::ModifiersChanged(*held));
            continue;
        }
        if let Some(event) = event(line, scale, *held) {
            input.events.push(event);
        }
    }
    drop(held);
    if ACTIVE.load(Ordering::Relaxed) {
        input.focused = true;
    }
}

fn event(line: Line, scale: f32, modifiers: egui::Modifiers) -> Option<egui::Event> {
    let point = |[x, y]: [f32; 2]| egui::pos2(x / scale, y / scale);
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
        Line::Modifiers { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_egui_events_in_points() {
        let parse = |text: &str| event(serde_json::from_str(text).unwrap(), 2.0, egui::Modifiers::NONE);
        assert_eq!(parse(r#"{"move":[100,40]}"#), Some(egui::Event::PointerMoved(egui::pos2(50.0, 20.0))));
        assert!(matches!(parse(r#"{"wheel":-3}"#),
            Some(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. }) if delta == egui::vec2(0.0, -3.0)));
        assert!(matches!(parse(r#"{"button":"middle","at":[10,10],"pressed":true}"#),
            Some(egui::Event::PointerButton { button: egui::PointerButton::Middle, pressed: true, .. })));
        assert!(matches!(parse(r#"{"key":"ArrowRight","pressed":false}"#),
            Some(egui::Event::Key { key: egui::Key::ArrowRight, pressed: false, .. })));
        assert_eq!(parse(r#"{"key":"NoSuchKey","pressed":true}"#), None);
        assert!(serde_json::from_str::<Line>(r#"{"jump":1}"#).is_err());
        assert_eq!(serde_json::from_str::<Line>(r#"{"ctrl":true,"shift":false,"alt":false}"#).unwrap(), Line::Modifiers { ctrl: true, shift: false, alt: false });
        let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        let held = event(serde_json::from_str(r#"{"button":"primary","at":[10,10],"pressed":true}"#).unwrap(), 2.0, ctrl);
        assert!(matches!(held, Some(egui::Event::PointerButton { modifiers, .. }) if modifiers == ctrl));
    }
}
