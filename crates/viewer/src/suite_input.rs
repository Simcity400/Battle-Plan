
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
use viewer_foundation::map::camera::{Camera, Pose, motion::Floor};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CameraTarget {
    Place { center: [f32; 2], height: f32 },
    Reset,
}

#[derive(Clone, Debug, PartialEq)]
struct CameraCommand {
    view: Option<String>,
    target: CameraTarget,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct CameraLine {
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    center: Option<[f32; 2]>,
    #[serde(default)]
    height: Option<f32>,
    #[serde(default)]
    reset: bool,
}

impl CameraLine {
    fn into_command(self) -> Result<CameraCommand, &'static str> {
        let target = if self.reset {
            if self.center.is_some() || self.height.is_some() {
                return Err("camera reset takes neither center nor height");
            }
            CameraTarget::Reset
        } else {
            let (Some(center), Some(height)) = (self.center, self.height) else {
                return Err("camera placement requires finite center and positive height");
            };
            if !center.iter().all(|value| value.is_finite()) || !height.is_finite() || height <= 0.0 {
                return Err("camera placement requires finite center and positive height");
            }
            CameraTarget::Place { center, height }
        };
        Ok(CameraCommand {
            view: self.view,
            target,
        })
    }
}

#[derive(Debug, Default)]
struct CameraQueue {
    pending: Vec<CameraCommand>,
}

impl CameraQueue {
    fn push(&mut self, command: CameraCommand) {
        self.pending.push(command);
    }

    fn take(&mut self, view: Option<&str>) -> Option<CameraTarget> {
        let mut chosen = None;
        self.pending.retain(|command| {
            if command.view.as_deref() == view {
                chosen = Some(command.target);
                false
            } else {
                true
            }
        });
        chosen
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum Line {
    Move { r#move: [f32; 2] },
    Wheel { wheel: f32 },
    Button { button: String, at: [f32; 2], pressed: bool },
    Key { key: String, pressed: bool },
    Modifiers { ctrl: bool, shift: bool, alt: bool },
    Camera { camera: CameraLine },
}

static PENDING: Mutex<Vec<Line>> = Mutex::new(Vec::new());
static PENDING_CAMERAS: Mutex<CameraQueue> = Mutex::new(CameraQueue { pending: Vec::new() });
static WAKER: Mutex<Option<egui::Context>> = Mutex::new(None);
static ENABLED: OnceLock<bool> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELD: Mutex<egui::Modifiers> = Mutex::new(egui::Modifiers::NONE);

pub(crate) fn capture_clock() -> Option<f32> {
    static CLOCK: OnceLock<Option<f32>> = OnceLock::new();
    *CLOCK.get_or_init(|| std::env::var("BATTLEPLAN_E2E_CAPTURE_CLOCK").or_else(|_| std::env::var("HOI4_E2E_CAPTURE_CLOCK")).ok()?.parse().ok())
}

fn wake() {
    if let Some(ctx) = WAKER.lock().unwrap_or_else(|error| error.into_inner()).clone() {
        ctx.request_repaint();
    }
}

fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        let Some(address) = std::env::var("BATTLEPLAN_E2E_INPUT").or_else(|_| std::env::var("HOI4_E2E_INPUT")).ok() else {
            return false;
        };
        let Ok(address) = address.parse::<SocketAddr>() else {
            eprintln!("BATTLEPLAN_E2E_INPUT {address}: not an address, ignored");
            return false;
        };
        if !address.ip().is_loopback() {
            eprintln!("BATTLEPLAN_E2E_INPUT {address}: not a loopback address, ignored");
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
                            Ok(Line::Camera { camera }) => match camera.into_command() {
                                Ok(command) => {
                                    PENDING_CAMERAS
                                        .lock()
                                        .unwrap_or_else(|error| error.into_inner())
                                        .push(command);
                                    wake();
                                }
                                Err(error) => eprintln!("BATTLEPLAN_E2E_INPUT line refused: {error}"),
                            },
                            Ok(line) => {
                                PENDING.lock().unwrap_or_else(|error| error.into_inner()).push(line);
                                wake();
                            }
                            Err(error) => eprintln!("BATTLEPLAN_E2E_INPUT line refused: {error}"),
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
    *WAKER.lock().unwrap_or_else(|error| error.into_inner()) = Some(ctx.clone());
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

pub(crate) fn take_camera(view: Option<&str>) -> Option<CameraTarget> {
    if ENABLED.get() != Some(&true) {
        return None;
    }
    PENDING_CAMERAS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take(view)
}

pub(crate) fn apply_camera(
    camera: &mut Camera,
    target: CameraTarget,
    viewport: [u32; 2],
    floor: Floor<'_>,
) -> bool {
    match target {
        CameraTarget::Reset => {
            *camera = Camera::fit(camera.map, camera.defines, viewport);
            false
        }
        CameraTarget::Place { center, height } => {
            let width = camera.map.x();
            let x = if width > 0.0 {
                center[0].rem_euclid(width)
            } else {
                center[0]
            };
            let z = center[1].clamp(0.0, camera.map.y());
            let [low_limit, high_limit] = camera.defines.heights;
            camera.center = [x, z];
            camera.pose = Pose::Map;
            let initial_low = floor(x, z - camera.defines.tilt_lead).max(low_limit);
            camera.height = height.clamp(initial_low, high_limit.max(initial_low));
            camera.clamp_center(viewport);
            let eye = camera.eye();
            let low = floor(eye[0], eye[2]).max(low_limit);
            camera.height = camera.height.clamp(low, high_limit.max(low));
            true
        }
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
        Line::Modifiers { .. } | Line::Camera { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_foundation::map::camera::{MapDefines, MapSize};

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

    #[test]
    fn camera_lines_parse_validate_and_route_by_view() {
        let parse_camera = |text: &str| match serde_json::from_str::<Line>(text).unwrap() {
            Line::Camera { camera } => camera.into_command(),
            other => panic!("unexpected line: {other:?}"),
        };
        let placed = parse_camera(r#"{"camera":{"view":"overview-map","center":[2816,1024],"height":180}}"#).unwrap();
        assert_eq!(placed, CameraCommand {
            view: Some("overview-map".into()),
            target: CameraTarget::Place { center: [2816.0, 1024.0], height: 180.0 },
        });
        let reset = parse_camera(r#"{"camera":{"view":"overview-map","reset":true}}"#).unwrap();
        assert_eq!(reset, CameraCommand {
            view: Some("overview-map".into()),
            target: CameraTarget::Reset,
        });
        let campaign = parse_camera(r#"{"camera":{"center":[3000,1200],"height":200}}"#).unwrap();
        assert_eq!(campaign, CameraCommand {
            view: None,
            target: CameraTarget::Place { center: [3000.0, 1200.0], height: 200.0 },
        });
        assert!(parse_camera(r#"{"camera":{"view":"overview-map"}}"#).is_err());
        assert!(parse_camera(r#"{"camera":{"view":"overview-map","center":[2816,1024],"height":0}}"#).is_err());
        assert!(parse_camera(r#"{"camera":{"view":"overview-map","center":[2816,1024],"height":180,"reset":true}}"#).is_err());
        assert!(serde_json::from_str::<Line>(r#"{"camera":{"extra":1}}"#).is_err());

        let mut queue = CameraQueue::default();
        queue.push(placed);
        queue.push(campaign.clone());
        queue.push(reset.clone());
        assert_eq!(queue.take(Some("overview-map")), Some(CameraTarget::Reset));
        assert_eq!(queue.take(Some("overview-map")), None);
        assert_eq!(queue.take(None), Some(campaign.target));
        assert_eq!(queue.take(None), None);
    }

    #[test]
    fn apply_camera_places_deterministically_clamps_bounds_and_resets_to_fit() {
        let viewport = [1000, 500];
        let mut camera = Camera::fit(MapSize::TEST, MapDefines::test(), viewport);
        let fit = camera;
        let floor = |_: f32, _: f32| 12.0;

        assert!(apply_camera(
            &mut camera,
            CameraTarget::Place { center: [2816.0, 1024.0], height: 180.0 },
            viewport,
            &floor,
        ));
        assert_eq!(camera.center, [2816.0, 1024.0]);
        assert_eq!(camera.height, 180.0);
        assert_eq!(camera.pose, Pose::Map);

        let high_floor = |_: f32, _: f32| 95.0;
        assert!(apply_camera(
            &mut camera,
            CameraTarget::Place { center: [-10.0, 1024.0], height: 10.0 },
            viewport,
            &high_floor,
        ));
        assert_eq!(camera.center[0], MapSize::TEST.x() - 10.0);
        assert_eq!(camera.height, 95.0);

        assert!(!apply_camera(&mut camera, CameraTarget::Reset, viewport, &floor));
        assert_eq!(camera, fit);
    }
}
