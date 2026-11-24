
use super::NativeHost;
use crate::popout::PopOuts;
use dashboard_core::protocol::{GamepadButton, GamepadMessage, GamepadOwner, GamepadState};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, GilrsBuilder};
use std::{collections::HashMap, thread, time::Duration};
use tauri::{AppHandle, Manager};
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

const POLL: Duration = Duration::from_millis(8);
const RESTING: f32 = 0.02;
const STEP: f32 = 1.0 / 512.0;

const BUTTONS: [(Button, GamepadButton); 14] = [
    (Button::South, GamepadButton::South),
    (Button::East, GamepadButton::East),
    (Button::West, GamepadButton::West),
    (Button::North, GamepadButton::North),
    (Button::LeftTrigger, GamepadButton::LeftBumper),
    (Button::RightTrigger, GamepadButton::RightBumper),
    (Button::Select, GamepadButton::Select),
    (Button::Start, GamepadButton::Start),
    (Button::LeftThumb, GamepadButton::LeftStick),
    (Button::RightThumb, GamepadButton::RightStick),
    (Button::DPadUp, GamepadButton::DpadUp),
    (Button::DPadDown, GamepadButton::DpadDown),
    (Button::DPadLeft, GamepadButton::DpadLeft),
    (Button::DPadRight, GamepadButton::DpadRight),
];

pub fn start_gamepad(app: AppHandle) {
    let started = thread::Builder::new()
        .name("gamepad".into())
        .spawn(move || run(&app));
    if let Err(error) = started {
        eprintln!("Controllers are unavailable: {error}");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Campaign,
    PopOut(u64),
}

fn front(foreground: usize, main: Option<usize>, popout: Option<u64>) -> Option<Target> {
    match popout {
        Some(generation) => Some(Target::PopOut(generation)),
        None => (foreground != 0 && main == Some(foreground)).then_some(Target::Campaign),
    }
}

trait Maps {
    fn ready(&self, target: Target) -> Option<u64>;
    fn send(&self, target: Target, viewer: u64, message: GamepadMessage) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Holder {
    target: Target,
    owner: GamepadOwner,
    sent: GamepadState,
}

#[derive(Debug, Default)]
struct Router {
    holder: Option<Holder>,
    handoffs: u64,
}

impl Router {
    fn route(
        &mut self,
        maps: &impl Maps,
        target: Option<Target>,
        state: GamepadState,
        pressed: u32,
    ) {
        let wanted = target.and_then(|target| Some((target, maps.ready(target)?)));
        if let Some(holder) = self.holder {
            if Some((holder.target, holder.owner.viewer)) == wanted {
                if holder.sent != state || pressed != 0 {
                    let message = GamepadMessage::State {
                        owner: holder.owner,
                        state,
                        pressed,
                    };
                    self.holder = maps
                        .send(holder.target, holder.owner.viewer, message)
                        .then_some(Holder {
                            sent: state,
                            ..holder
                        });
                }
                return;
            }
            maps.send(holder.target, holder.owner.viewer, GamepadMessage::Release);
            self.holder = None;
        }
        if let Some((target, viewer)) = wanted {
            self.handoffs += 1;
            let owner = GamepadOwner {
                viewer,
                handoff: self.handoffs,
            };
            self.holder = maps
                .send(target, viewer, GamepadMessage::Take { owner, state })
                .then_some(Holder {
                    target,
                    owner,
                    sent: state,
                });
        }
    }
}

impl NativeHost {
    fn pad_viewer(&self) -> Option<u64> {
        let inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        (inner.map.is_some()
            && !inner.shutting_down
            && !inner.active_dashboard
            && inner.status.ready
            && inner.status.visible)
            .then_some(inner.generation)
    }

    fn send_gamepad(&self, viewer: u64, message: GamepadMessage) -> bool {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if inner.map.is_none() || inner.shutting_down || inner.generation != viewer {
            return false;
        }
        inner.pending.gamepad = Some(GamepadMessage::merge(inner.pending.gamepad.take(), message));
        drop(inner);
        self.flush();
        true
    }

    fn stopping(&self) -> bool {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shutting_down
    }
}

struct LiveMaps<'a> {
    campaign: &'a NativeHost,
    popouts: &'a PopOuts,
}

impl LiveMaps<'_> {
    fn host(&self, target: Target) -> Option<NativeHost> {
        match target {
            Target::Campaign => Some(self.campaign.clone()),
            Target::PopOut(generation) => self.popouts.host_of(generation),
        }
    }
}

impl Maps for LiveMaps<'_> {
    fn ready(&self, target: Target) -> Option<u64> {
        self.host(target)?.pad_viewer()
    }

    fn send(&self, target: Target, viewer: u64, message: GamepadMessage) -> bool {
        self.host(target)
            .is_some_and(|host| host.send_gamepad(viewer, message))
    }
}

fn run(app: &AppHandle) {
    let mut gilrs = match GilrsBuilder::new().with_default_filters(false).build() {
        Ok(gilrs) => gilrs,
        Err(error) => {
            eprintln!("Controllers are unavailable: {error}");
            return;
        }
    };
    let campaign = app.state::<NativeHost>().inner().clone();
    let popouts = app.state::<PopOuts>();
    let maps = LiveMaps {
        campaign: &campaign,
        popouts: popouts.inner(),
    };
    let mut active: Option<GamepadId> = None;
    let mut buttons: HashMap<GamepadId, u32> = HashMap::new();
    let mut presses: HashMap<GamepadId, u32> = HashMap::new();
    let mut router = Router::default();
    while !campaign.stopping() {
        presses.clear();
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Disconnected => {
                    if active == Some(event.id) {
                        active = None;
                    }
                    buttons.remove(&event.id);
                }
                EventType::Connected | EventType::Dropped => {}
                _ => {
                    let state = read(&gilrs, event.id);
                    active = last_touched(active, event.id, &state);
                    let held = buttons.entry(event.id).or_default();
                    *presses.entry(event.id).or_default() |= state.buttons & !*held;
                    *held = state.buttons;
                }
            }
        }
        let state = active.map_or_else(GamepadState::default, |id| read(&gilrs, id));
        let pressed = active.and_then(|id| presses.get(&id).copied()).unwrap_or(0);
        if let Some(id) = active {
            buttons.insert(id, state.buttons);
        }
        let foreground = unsafe { GetForegroundWindow() } as usize;
        let main = app
            .get_webview_window("main")
            .and_then(|window| window.hwnd().ok())
            .map(|hwnd| hwnd.0 as usize);
        let target = front(foreground, main, popouts.in_front(foreground));
        router.route(&maps, target, state, pressed);
        thread::sleep(POLL);
    }
}

fn last_touched<T: PartialEq>(active: Option<T>, pad: T, state: &GamepadState) -> Option<T> {
    let used = state.sticks.iter().any(|stick| *stick != [0.0; 2])
        || state.triggers.iter().any(|pull| *pull != 0.0)
        || state.buttons != 0;
    if used { Some(pad) } else { active }
}

fn read(gilrs: &Gilrs, id: GamepadId) -> GamepadState {
    let Some(pad) = gilrs.connected_gamepad(id) else {
        return GamepadState::default();
    };
    let stick = |x: Axis, y: Axis| settle([pad.value(x), pad.value(y)]);
    let trigger = |button: Button| {
        let value = pad.button_data(button).map_or(0.0, |data| data.value());
        if value.is_finite() && value >= RESTING {
            step(value.min(1.0))
        } else {
            0.0
        }
    };
    let buttons = BUTTONS
        .iter()
        .filter(|(button, _)| pad.is_pressed(*button))
        .fold(0, |bits, (_, bit)| bits | bit.bit())
        | hat_bits(pad.value(Axis::DPadX), pad.value(Axis::DPadY));
    GamepadState {
        sticks: [
            stick(Axis::LeftStickX, Axis::LeftStickY),
            stick(Axis::RightStickX, Axis::RightStickY),
        ],
        triggers: [
            trigger(Button::LeftTrigger2),
            trigger(Button::RightTrigger2),
        ],
        buttons,
    }
}

fn hat_bits(x: f32, y: f32) -> u32 {
    [
        (x >= 1.0, GamepadButton::DpadRight),
        (x <= -1.0, GamepadButton::DpadLeft),
        (y >= 1.0, GamepadButton::DpadUp),
        (y <= -1.0, GamepadButton::DpadDown),
    ]
    .into_iter()
    .filter(|(pressed, _)| *pressed)
    .fold(0, |bits, (_, bit)| bits | bit.bit())
}

fn settle([x, y]: [f32; 2]) -> [f32; 2] {
    if !x.is_finite() || !y.is_finite() || x.hypot(y) < RESTING {
        return [0.0; 2];
    }
    [x, y].map(|value| step(value.clamp(-1.0, 1.0)))
}

fn step(value: f32) -> f32 {
    (value / STEP).round() * STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resting_sticks_are_centred_and_values_stay_in_range_and_steps() {
        assert_eq!(settle([0.01, -0.01]), [0.0; 2]);
        assert_eq!(settle([f32::NAN, 0.5]), [0.0; 2]);
        assert_eq!(settle([1.3, -2.0]), [1.0, -1.0]);
        let [x, y] = settle([0.5004, -0.25]);
        assert_eq!([x, y], [0.5, -0.25]);
        let state = GamepadState {
            sticks: [settle([0.7, 0.7]), [0.0; 2]],
            triggers: [step(0.3), step(1.0)],
            buttons: GamepadButton::South.bit() | GamepadButton::DpadRight.bit(),
        };
        assert!(state.validate().is_ok());
        assert!(state.held(GamepadButton::DpadRight) && !state.held(GamepadButton::North));
    }

    #[test]
    fn an_idle_controllers_jitter_never_takes_the_selection() {
        let state = |stick: [f32; 2], trigger: f32, buttons: u32| GamepadState {
            sticks: [settle(stick), [0.0; 2]],
            triggers: [trigger, 0.0],
            buttons,
        };
        let held = Some('a');
        assert_eq!(
            last_touched(held, 'b', &state([0.015, -0.01], 0.0, 0)),
            held
        );
        assert_eq!(
            last_touched(held, 'b', &state([0.3, 0.0], 0.0, 0)),
            Some('b')
        );
        assert_eq!(last_touched(held, 'b', &state([0.0; 2], 0.4, 0)), Some('b'));
        let south = GamepadButton::South.bit();
        assert_eq!(
            last_touched(held, 'b', &state([0.0; 2], 0.0, south)),
            Some('b')
        );
        assert_eq!(last_touched(None, 'b', &state([0.01, 0.0], 0.0, 0)), None);
        assert_eq!(last_touched(held, 'a', &state([0.0; 2], 0.0, 0)), held);
    }

    #[derive(Default)]
    struct Fake {
        ready: std::cell::RefCell<Vec<(Target, u64)>>,
        sent: std::cell::RefCell<Vec<(Target, u64, GamepadMessage)>>,
    }

    impl Fake {
        fn set(&self, target: Target, viewer: Option<u64>) {
            let mut ready = self.ready.borrow_mut();
            ready.retain(|(at, _)| *at != target);
            if let Some(viewer) = viewer {
                ready.push((target, viewer));
            }
        }

        fn take(&self) -> Vec<(Target, u64, GamepadMessage)> {
            std::mem::take(&mut self.sent.borrow_mut())
        }
    }

    impl Maps for Fake {
        fn ready(&self, target: Target) -> Option<u64> {
            self.ready
                .borrow()
                .iter()
                .find(|(at, _)| *at == target)
                .map(|(_, viewer)| *viewer)
        }

        fn send(&self, target: Target, viewer: u64, message: GamepadMessage) -> bool {
            self.sent.borrow_mut().push((target, viewer, message));
            self.ready(target) == Some(viewer)
        }
    }

    fn held(x: f32) -> GamepadState {
        GamepadState {
            sticks: [[x, 0.0], [0.0; 2]],
            ..GamepadState::default()
        }
    }

    fn taken(sent: &[(Target, u64, GamepadMessage)]) -> Vec<(Target, GamepadState)> {
        sent.iter()
            .filter_map(|(target, _, message)| match message {
                GamepadMessage::Take { state, .. } => Some((*target, *state)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_map_that_cannot_take_the_controller_ends_the_holders_handoff() {
        let maps = Fake::default();
        let popout = Target::PopOut(2);
        maps.set(popout, Some(1));
        maps.set(Target::Campaign, Some(7));
        let mut router = Router::default();
        router.route(&maps, Some(popout), held(0.8), 0);
        assert_eq!(taken(&maps.take()), [(popout, held(0.8))]);
        router.route(&maps, Some(popout), held(0.8), 0);
        assert!(maps.take().is_empty());
        maps.set(Target::Campaign, None);
        router.route(&maps, Some(Target::Campaign), held(0.8), 0);
        assert_eq!(maps.take(), [(popout, 1, GamepadMessage::Release)]);
        router.route(&maps, Some(popout), held(0.8), 0);
        assert_eq!(taken(&maps.take()), [(popout, held(0.8))]);
    }

    #[test]
    fn every_handoff_releases_the_last_holder_and_numbers_the_next() {
        let maps = Fake::default();
        let popout = Target::PopOut(2);
        maps.set(popout, Some(1));
        maps.set(Target::Campaign, Some(7));
        let mut router = Router::default();
        router.route(&maps, Some(Target::Campaign), held(0.5), 0);
        router.route(&maps, Some(popout), held(0.5), 0);
        let sent = maps.take();
        let owners: Vec<_> = sent
            .iter()
            .filter_map(|(_, _, message)| match message {
                GamepadMessage::Take { owner, .. } => Some(*owner),
                _ => None,
            })
            .collect();
        assert_eq!(
            owners,
            [
                GamepadOwner {
                    viewer: 7,
                    handoff: 1
                },
                GamepadOwner {
                    viewer: 1,
                    handoff: 2
                }
            ]
        );
        assert_eq!(sent[1], (Target::Campaign, 7, GamepadMessage::Release));
        maps.set(popout, Some(2));
        router.route(&maps, Some(popout), held(0.5), 0);
        let sent = maps.take();
        assert_eq!(sent[0], (popout, 1, GamepadMessage::Release));
        assert!(matches!(
            sent[1],
            (
                _,
                2,
                GamepadMessage::Take {
                    owner: GamepadOwner {
                        viewer: 2,
                        handoff: 3
                    },
                    ..
                }
            )
        ));
    }

    #[test]
    fn no_map_in_front_releases_the_holder() {
        let maps = Fake::default();
        let popout = Target::PopOut(2);
        maps.set(popout, Some(1));
        maps.set(Target::Campaign, Some(7));
        assert_eq!(front(10, Some(10), None), Some(Target::Campaign));
        assert_eq!(front(20, Some(10), Some(2)), Some(popout));
        assert_eq!(front(30, Some(10), None), None);
        assert_eq!(front(0, None, None), None);
        let mut router = Router::default();
        router.route(&maps, Some(Target::Campaign), held(0.8), 0);
        maps.take();
        router.route(&maps, None, held(0.8), 0);
        assert_eq!(
            maps.take(),
            [(Target::Campaign, 7, GamepadMessage::Release)]
        );
        router.route(&maps, None, held(0.8), 0);
        assert!(maps.take().is_empty());
        router.route(&maps, Some(Target::Campaign), held(0.3), 0);
        assert_eq!(taken(&maps.take()), [(Target::Campaign, held(0.3))]);
    }

    #[test]
    fn a_failed_state_ends_the_handoff() {
        struct Gone;
        impl Maps for Gone {
            fn ready(&self, _: Target) -> Option<u64> {
                Some(1)
            }
            fn send(&self, _: Target, _: u64, message: GamepadMessage) -> bool {
                !matches!(message, GamepadMessage::State { .. })
            }
        }
        let mut router = Router::default();
        router.route(&Gone, Some(Target::Campaign), held(0.1), 0);
        assert!(router.holder.is_some());
        router.route(&Gone, Some(Target::Campaign), held(0.2), 0);
        assert!(router.holder.is_none());
    }

    #[test]
    fn a_tap_between_readings_reaches_the_holder_as_a_press() {
        let maps = Fake::default();
        maps.set(Target::Campaign, Some(7));
        let mut router = Router::default();
        router.route(&maps, Some(Target::Campaign), held(0.0), 0);
        maps.take();
        let south = GamepadButton::South.bit();
        router.route(&maps, Some(Target::Campaign), held(0.0), south);
        let sent = maps.take();
        assert!(
            matches!(
                sent[..],
                [(Target::Campaign, 7, GamepadMessage::State { pressed, .. })] if pressed == south
            ),
            "{sent:?}"
        );
        router.route(&maps, Some(Target::Campaign), held(0.0), 0);
        assert!(maps.take().is_empty());
        maps.set(Target::Campaign, Some(8));
        router.route(&maps, Some(Target::Campaign), held(0.0), south);
        assert!(
            maps.take()
                .iter()
                .all(|(_, _, message)| !matches!(message, GamepadMessage::State { .. }))
        );
    }

    #[test]
    fn a_hat_switch_presses_the_directional_pad() {
        assert_eq!(hat_bits(0.0, 0.0), 0);
        assert_eq!(hat_bits(1.0, 0.0), GamepadButton::DpadRight.bit());
        assert_eq!(hat_bits(0.0, -1.0), GamepadButton::DpadDown.bit());
        assert_eq!(
            hat_bits(-1.0, 1.0),
            GamepadButton::DpadLeft.bit() | GamepadButton::DpadUp.bit()
        );
        assert_eq!(hat_bits(f32::NAN, 0.5), 0);
        let state = GamepadState {
            buttons: hat_bits(0.0, 1.0),
            ..GamepadState::default()
        };
        assert_eq!(last_touched(None, 'h', &state), Some('h'));
    }

}
