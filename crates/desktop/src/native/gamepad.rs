//! Controllers, read in the host and handed to the map in front.
//!
//! Windows.Gaming.Input reports controllers (Xbox, PlayStation and other gamepads) only to the
//! process that owns the foreground window. The maps' windows are children of the host's, so the
//! host reads them and hands the controller the user last touched to the map whose window is in
//! front: the pop-out's while its window is, the campaign map while the main window is and the
//! map shows, and none while any other window is. The maps apply their own dead zones; resting
//! jitter stays here.
//!
//! One owner decides which viewer holds the controller: this thread's [`Router`]. Each handoff is
//! numbered, and every message names the viewer and handoff it is for ([`GamepadMessage`]). A
//! viewer taking the controller gets what is held with no presses; the one it leaves is released,
//! letting go of everything with no presses, and drops any state still on its way. What was last
//! sent belongs to one viewer under one handoff: a change of window, a map that cannot take the
//! controller (hidden, or the dashboard in front), a failed send and a restarted viewer all end
//! it, and the next viewer able to take the controller takes it with what is held then. A map's
//! own focus never ends a handoff; only the host's release does.

use super::NativeHost;
use crate::popout::PopOuts;
use dashboard::protocol::{GamepadButton, GamepadMessage, GamepadOwner, GamepadState};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, GilrsBuilder};
use std::{thread, time::Duration};
use tauri::{AppHandle, Manager};
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// How often the host reads controllers.
const POLL: Duration = Duration::from_millis(8);
/// Stick deflections and trigger pulls below this stay at rest, so a resting controller's jitter
/// is never sent and never makes it the last touched.
const RESTING: f32 = 0.02;
/// Values are sent in steps of this, so noise below it never produces a message.
const STEP: f32 = 1.0 / 512.0;

/// Each standard button and the bit the map reads it as.
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

/// Starts reading controllers for the maps. Without a controller interface the thread ends
/// quietly; the maps keep their keyboard and mouse.
pub fn start_gamepad(app: AppHandle) {
    let started = thread::Builder::new()
        .name("gamepad".into())
        .spawn(move || run(&app));
    if let Err(error) = started {
        eprintln!("Controllers are unavailable: {error}");
    }
}

/// The map a controller drives: the campaign map, or pop-out window `generation`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Campaign,
    PopOut(u64),
}

/// The map whose window is `foreground`: pop-out window `popout`'s when it is that window, the
/// campaign map's when it is the main window, `main`, and none for any other window.
fn front(foreground: usize, main: Option<usize>, popout: Option<u64>) -> Option<Target> {
    match popout {
        Some(generation) => Some(Target::PopOut(generation)),
        None => (foreground != 0 && main == Some(foreground)).then_some(Target::Campaign),
    }
}

/// The maps the router hands the controller between.
trait Maps {
    /// The number of `target`'s viewer while it can take the controller: running, ready and
    /// showing its map.
    fn ready(&self, target: Target) -> Option<u64>;
    /// Queues `message` for `target`'s viewer `viewer`; `false` when another viewer runs there
    /// now, or none.
    fn send(&self, target: Target, viewer: u64, message: GamepadMessage) -> bool;
}

/// The viewer holding the controller: its map, its viewer and handoff, and what it was last sent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Holder {
    target: Target,
    owner: GamepadOwner,
    sent: GamepadState,
}

/// The one owner of the controller's handoffs.
#[derive(Debug, Default)]
struct Router {
    holder: Option<Holder>,
    handoffs: u64,
}

impl Router {
    /// One reading: `state`, for the map in front, `target`, or none.
    fn route(&mut self, maps: &impl Maps, target: Option<Target>, state: GamepadState) {
        let wanted = target.and_then(|target| Some((target, maps.ready(target)?)));
        if let Some(holder) = self.holder {
            if Some((holder.target, holder.owner.viewer)) == wanted {
                if holder.sent != state {
                    let message = GamepadMessage::State {
                        owner: holder.owner,
                        state,
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
            // Another map, viewer or window is in front, or the holder can no longer take it.
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
    /// The number of the viewer running now while it can take the controller: ready and showing
    /// the full map, with no dashboard in front.
    fn pad_viewer(&self) -> Option<u64> {
        let inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        (inner.process.is_some()
            && !inner.shutting_down
            && !inner.active_dashboard
            && inner.status.ready
            && inner.status.visible)
            .then_some(inner.generation)
    }

    /// Queues `message` for viewer `viewer`, merged with any still waiting; `false` when another
    /// viewer runs now, or none.
    fn send_gamepad(&self, viewer: u64, message: GamepadMessage) -> bool {
        let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.process.is_none() || inner.shutting_down || inner.generation != viewer {
            return false;
        }
        inner.pending.gamepad = Some(GamepadMessage::merge(inner.pending.gamepad.take(), message));
        drop(inner);
        self.shared.1.notify_all();
        true
    }

    fn stopping(&self) -> bool {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shutting_down
    }
}

/// The campaign map's host and the pop-out's.
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
    let mut router = Router::default();
    while !campaign.stopping() {
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Disconnected if active == Some(event.id) => active = None,
                EventType::Connected | EventType::Disconnected | EventType::Dropped => {}
                _ => active = last_touched(active, event.id, &read(&gilrs, event.id)),
            }
        }
        let state = active.map_or_else(GamepadState::default, |id| read(&gilrs, id));
        // SAFETY: GetForegroundWindow takes no arguments and only reads the window in front.
        let foreground = unsafe { GetForegroundWindow() } as usize;
        let main = app
            .get_webview_window("main")
            .and_then(|window| window.hwnd().ok())
            .map(|hwnd| hwnd.0 as usize);
        let target = front(foreground, main, popouts.in_front(foreground));
        router.route(&maps, target, state);
        thread::sleep(POLL);
    }
}

/// The controller to forward after an event from `pad`, whose state is now `state`: a controller
/// takes over only once it is really used, so an idle one's resting jitter never takes the
/// selection from the one in hand.
fn last_touched<T: PartialEq>(active: Option<T>, pad: T, state: &GamepadState) -> Option<T> {
    let used = state.sticks.iter().any(|stick| *stick != [0.0; 2])
        || state.triggers.iter().any(|pull| *pull != 0.0)
        || state.buttons != 0;
    if used { Some(pad) } else { active }
}

/// The controller's state in the map's terms.
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
        .fold(0, |bits, (_, bit)| bits | bit.bit());
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

/// A stick at rest reads as centred; otherwise its value in steps, within the unit square.
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
        // Pad b only jitters at rest: pad a, held steady, stays forwarded.
        assert_eq!(
            last_touched(held, 'b', &state([0.015, -0.01], 0.0, 0)),
            held
        );
        // Really used, by a stick, a trigger or a button, it takes over.
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
        // With none forwarded yet, jitter selects nothing.
        assert_eq!(last_touched(None, 'b', &state([0.01, 0.0], 0.0, 0)), None);
        // The forwarded pad coming to rest stays forwarded, so its rest reaches the map.
        assert_eq!(last_touched(held, 'a', &state([0.0; 2], 0.0, 0)), held);
    }

    /// Maps whose readiness and viewers a test sets, recording every message sent.
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
        // The stick held in the pop-out.
        router.route(&maps, Some(popout), held(0.8));
        assert_eq!(taken(&maps.take()), [(popout, held(0.8))]);
        router.route(&maps, Some(popout), held(0.8));
        assert!(maps.take().is_empty());
        // The main window in front shows the dashboard: the campaign map cannot take the
        // controller, and the pop-out lets go of it.
        maps.set(Target::Campaign, None);
        router.route(&maps, Some(Target::Campaign), held(0.8));
        assert_eq!(maps.take(), [(popout, 1, GamepadMessage::Release)]);
        // Back to the pop-out without moving the stick: it takes what is held again.
        router.route(&maps, Some(popout), held(0.8));
        assert_eq!(taken(&maps.take()), [(popout, held(0.8))]);
    }

    #[test]
    fn every_handoff_releases_the_last_holder_and_numbers_the_next() {
        let maps = Fake::default();
        let popout = Target::PopOut(2);
        maps.set(popout, Some(1));
        maps.set(Target::Campaign, Some(7));
        let mut router = Router::default();
        router.route(&maps, Some(Target::Campaign), held(0.5));
        router.route(&maps, Some(popout), held(0.5));
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
        // A restarted pop-out viewer in the same window is another viewer: the old one is
        // released and the new one takes the controller under a new handoff.
        maps.set(popout, Some(2));
        router.route(&maps, Some(popout), held(0.5));
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
        // The main window and the pop-out are the maps' windows; any other is none.
        assert_eq!(front(10, Some(10), None), Some(Target::Campaign));
        assert_eq!(front(20, Some(10), Some(2)), Some(popout));
        assert_eq!(front(30, Some(10), None), None);
        assert_eq!(front(0, None, None), None);
        let mut router = Router::default();
        router.route(&maps, Some(Target::Campaign), held(0.8));
        maps.take();
        // Another program comes to the front with the stick held: the map lets go, as Windows
        // stops reporting the controller to the host.
        router.route(&maps, None, held(0.8));
        assert_eq!(
            maps.take(),
            [(Target::Campaign, 7, GamepadMessage::Release)]
        );
        router.route(&maps, None, held(0.8));
        assert!(maps.take().is_empty());
        // Back in front, it takes what is held then.
        router.route(&maps, Some(Target::Campaign), held(0.3));
        assert_eq!(taken(&maps.take()), [(Target::Campaign, held(0.3))]);
    }

    #[test]
    fn a_failed_state_ends_the_handoff() {
        /// Ready, but every send fails: the viewer went away between the two reads.
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
        router.route(&Gone, Some(Target::Campaign), held(0.1));
        assert!(router.holder.is_some());
        router.route(&Gone, Some(Target::Campaign), held(0.2));
        assert!(router.holder.is_none());
    }

    #[test]
    fn every_standard_button_has_its_own_bit() {
        let bits: Vec<_> = BUTTONS.iter().map(|(_, bit)| bit.bit()).collect();
        let mut unique = bits.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), GamepadButton::ALL.len());
        assert_eq!(bits.iter().fold(0, |all, bit| all | bit), (1 << 14) - 1);
    }
}
