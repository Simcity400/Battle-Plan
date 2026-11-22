
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use viewer_gpu::camera::{Camera, Pose, motion::Floor};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CameraTarget {
    Place { center: [f32; 2], height: f32 },
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct ResolvedCamera {
    pub(crate) center: [f32; 2],
    pub(crate) height: f32,
}

#[derive(Debug)]
pub(crate) struct PendingCamera {
    pub(crate) target: CameraTarget,
    cancelled: Arc<AtomicBool>,
    reply: Option<mpsc::SyncSender<Result<ResolvedCamera, String>>>,
}

impl PendingCamera {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn finish(mut self, result: Result<ResolvedCamera, String>) {
        if let Some(tx) = self.reply.take() {
            let _ = tx.send(result);
        }
    }
}

#[derive(Debug)]
struct QueuedCamera {
    id: u64,
    view: Option<String>,
    pending: PendingCamera,
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
    fn into_command(self) -> Result<(Option<String>, CameraTarget), &'static str> {
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
        Ok((self.view, target))
    }
}

#[derive(Debug, Default)]
struct CameraQueue {
    next_id: u64,
    pending: Vec<QueuedCamera>,
}

impl CameraQueue {
    fn push(
        &mut self,
        view: Option<String>,
        target: CameraTarget,
        reply: Option<mpsc::SyncSender<Result<ResolvedCamera, String>>>,
    ) -> (u64, Arc<AtomicBool>) {
        self.next_id += 1;
        let id = self.next_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.pending.push(QueuedCamera {
            id,
            view,
            pending: PendingCamera {
                target,
                cancelled: Arc::clone(&cancelled),
                reply,
            },
        });
        (id, cancelled)
    }

    fn cancel(&mut self, id: u64) -> bool {
        let mut removed = false;
        let existing = std::mem::take(&mut self.pending);
        for command in existing {
            if command.id == id {
                command.pending.cancelled.store(true, Ordering::Release);
                removed = true;
            } else {
                self.pending.push(command);
            }
        }
        removed
    }

    fn take(&mut self, view: Option<&str>) -> Option<PendingCamera> {
        let mut chosen = None;
        let existing = std::mem::take(&mut self.pending);
        for command in existing {
            if command.pending.is_cancelled() {
                continue;
            }
            if command.view.as_deref() == view {
                chosen = Some(command.pending);
            } else {
                self.pending.push(command);
            }
        }
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
static PENDING_CAMERAS: Mutex<CameraQueue> = Mutex::new(CameraQueue {
    next_id: 0,
    pending: Vec::new(),
});
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

fn encode_camera_reply(result: Result<ResolvedCamera, String>) -> String {
    match result {
        Ok(resolved) => serde_json::to_string(&resolved)
            .unwrap_or_else(|_| r#"{"error":"failed to encode resolved camera"}"#.into()),
        Err(error) => serde_json::json!({ "error": error }).to_string(),
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
                    let mut writer = stream.try_clone().ok();
                    for line in BufReader::new(stream).lines().map_while(Result::ok) {
                        match serde_json::from_str::<Line>(&line) {
                            Ok(Line::Camera { camera }) => {
                                let reply = match camera.into_command() {
                                    Ok((view, target)) => {
                                        let (tx, rx) = mpsc::sync_channel(1);
                                        let (id, cancelled) = PENDING_CAMERAS
                                            .lock()
                                            .unwrap_or_else(|error| error.into_inner())
                                            .push(view, target, Some(tx));
                                        wake();
                                        match rx.recv_timeout(Duration::from_secs(10)) {
                                            Ok(result) => result,
                                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                                cancelled.store(true, Ordering::Release);
                                                PENDING_CAMERAS
                                                    .lock()
                                                    .unwrap_or_else(|error| error.into_inner())
                                                    .cancel(id);
                                                Err("viewer did not apply camera command within 10 s".into())
                                            }
                                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                                Err("camera command was superseded or dropped before drawing".into())
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        eprintln!("BATTLEPLAN_E2E_INPUT line refused: {error}");
                                        Err(error.into())
                                    }
                                };
                                if let Some(writer) = writer.as_mut() {
                                    let _ = writeln!(writer, "{}", encode_camera_reply(reply));
                                    let _ = writer.flush();
                                }
                            }
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

pub(crate) fn take_camera(view: Option<&str>) -> Option<PendingCamera> {
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
            camera.clamp_center(viewport);
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
    use viewer_gpu::camera::{MapDefines, MapSize};

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
    fn camera_lines_parse_validate_route_and_reply_by_view() {
        let parse_camera = |text: &str| match serde_json::from_str::<Line>(text).unwrap() {
            Line::Camera { camera } => camera.into_command(),
            other => panic!("unexpected line: {other:?}"),
        };
        let (placed_view, placed_target) = parse_camera(r#"{"camera":{"view":"overview-map","center":[2816,1024],"height":180}}"#).unwrap();
        assert_eq!(placed_view, Some("overview-map".into()));
        assert_eq!(placed_target, CameraTarget::Place { center: [2816.0, 1024.0], height: 180.0 });
        let (reset_view, reset_target) = parse_camera(r#"{"camera":{"view":"overview-map","reset":true}}"#).unwrap();
        assert_eq!(reset_view, Some("overview-map".into()));
        assert_eq!(reset_target, CameraTarget::Reset);
        let (campaign_view, campaign_target) = parse_camera(r#"{"camera":{"center":[3000,1200],"height":200}}"#).unwrap();
        assert_eq!(campaign_view, None);
        assert_eq!(campaign_target, CameraTarget::Place { center: [3000.0, 1200.0], height: 200.0 });
        assert!(parse_camera(r#"{"camera":{"view":"overview-map"}}"#).is_err());
        assert!(parse_camera(r#"{"camera":{"view":"overview-map","center":[2816,1024],"height":0}}"#).is_err());
        assert!(parse_camera(r#"{"camera":{"view":"overview-map","center":[2816,1024],"height":180,"reset":true}}"#).is_err());
        assert!(serde_json::from_str::<Line>(r#"{"camera":{"extra":1}}"#).is_err());

        let (tx_old, rx_old) = mpsc::sync_channel(1);
        let (tx_reset, rx_reset) = mpsc::sync_channel(1);
        let (tx_campaign, rx_campaign) = mpsc::sync_channel(1);
        let mut queue = CameraQueue::default();
        queue.push(placed_view, placed_target, Some(tx_old));
        queue.push(campaign_view, campaign_target, Some(tx_campaign));
        queue.push(reset_view, reset_target, Some(tx_reset));

        let taken = queue.take(Some("overview-map")).unwrap();
        assert_eq!(taken.target, CameraTarget::Reset);
        assert!(rx_old.recv().is_err());
        let fit = ResolvedCamera { center: [2816.0, 1024.0], height: 3000.0 };
        taken.finish(Ok(fit));
        assert_eq!(rx_reset.recv().unwrap(), Ok(fit));
        assert!(queue.take(Some("overview-map")).is_none());

        let taken_campaign = queue.take(None).unwrap();
        assert_eq!(taken_campaign.target, campaign_target);
        let placed = ResolvedCamera { center: [3000.0, 1200.0], height: 200.0 };
        taken_campaign.finish(Ok(placed));
        assert_eq!(rx_campaign.recv().unwrap(), Ok(placed));
        assert!(queue.take(None).is_none());
    }

    #[test]
    fn timed_out_camera_command_is_discarded_and_never_applies_later() {
        let mut queue = CameraQueue::default();
        let (tx, _rx) = mpsc::sync_channel(1);
        let (id, cancelled) = queue.push(
            Some("overview-map".into()),
            CameraTarget::Place { center: [2816.0, 1024.0], height: 180.0 },
            Some(tx),
        );
        cancelled.store(true, Ordering::Release);
        assert!(queue.cancel(id));
        assert!(queue.take(Some("overview-map")).is_none());

        let (tx_in_flight, _rx_in_flight) = mpsc::sync_channel(1);
        let (in_flight_id, in_flight_cancelled) = queue.push(
            Some("overview-map".into()),
            CameraTarget::Place { center: [2816.0, 1024.0], height: 180.0 },
            Some(tx_in_flight),
        );
        let taken = queue.take(Some("overview-map")).unwrap();
        assert!(!taken.is_cancelled());
        in_flight_cancelled.store(true, Ordering::Release);
        assert!(!queue.cancel(in_flight_id));
        assert!(taken.is_cancelled());
    }

    #[test]
    fn apply_camera_places_deterministically_clamps_modded_limits_and_resets_to_fit() {
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

        let mut modded_defines = MapDefines::test();
        modded_defines.heights = [250.0, 4000.0];
        let mut modded = Camera::fit(MapSize::TEST, modded_defines, viewport);
        let modded_center = modded.center;
        let modded_fit_height = modded.height;
        assert!(modded_fit_height > 250.0);
        assert!(apply_camera(
            &mut modded,
            CameraTarget::Place { center: modded_center, height: 180.0 },
            viewport,
            &floor,
        ));
        assert_eq!(modded.height, 250.0);
        assert!(modded.height < modded_fit_height);
        let encoded = encode_camera_reply(Ok(ResolvedCamera { center: modded.center, height: modded.height }));
        let decoded: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded["height"].as_f64(), Some(250.0));

        assert!(!apply_camera(&mut camera, CameraTarget::Reset, viewport, &floor));
        assert_eq!(camera, fit);
    }
}
