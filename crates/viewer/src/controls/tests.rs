use super::*;
use std::time::Duration;

fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

const OWNER: GamepadOwner = GamepadOwner {
    viewer: 1,
    handoff: 1,
};

fn take(input: &mut Input, owner: GamepadOwner, state: GamepadState) {
    input.gamepad(GamepadMessage::Take { owner, state });
}

fn update(input: &mut Input, owner: GamepadOwner, state: GamepadState) {
    input.gamepad(GamepadMessage::State {
        owner,
        state,
        pressed: 0,
    });
}

fn window(events: &[egui::Event], focused: bool, now: Instant) -> Window<'_> {
    Window {
        events,
        focused,
        modifiers: egui::Modifiers::NONE,
        now,
    }
}

#[test]
fn sticks_have_a_radial_dead_zone_and_a_curve() {
    assert_eq!(stick([0.1, 0.1]), [0.0; 2]);
    assert_eq!(stick([STICK_DEAD_ZONE, 0.0]), [0.0; 2]);
    let full = stick([0.0, -0.97]);
    assert!((full[1] + 1.0).abs() < 1e-6 && full[0] == 0.0);
    let diagonal = stick([0.7, 0.7]);
    assert!((diagonal[0] - diagonal[1]).abs() < 1e-6);
    assert!((diagonal[0].hypot(diagonal[1]) - 1.0).abs() < 1e-5);
    let half = stick([(STICK_DEAD_ZONE + STICK_SATURATION) / 2.0, 0.0])[0];
    assert!((half - 0.5f32.powf(STICK_CURVE)).abs() < 1e-5, "{half}");
    assert_eq!(stick([f32::NAN, 0.0]), [0.0; 2]);
    assert_eq!(trigger(0.05), 0.0);
    assert_eq!(trigger(1.0), 1.0);
    assert!((trigger(0.53) - 0.5).abs() < 1e-6);
}

#[test]
fn approach_is_independent_of_the_frame_rate() {
    let mut fast = 0.0;
    for _ in 0..100 {
        fast = approach(fast, 1.0, 0.2, 0.01);
    }
    let mut slow = 0.0;
    for _ in 0..10 {
        slow = approach(slow, 1.0, 0.2, 0.1);
    }
    assert!((fast - slow).abs() < 1e-5, "{fast} {slow}");
    assert!((fast - (1.0 - (-5.0f32).exp())).abs() < 1e-5);
}

#[test]
fn frames_measure_wall_time_and_hold_keys_until_released() {
    let mut input = Input::default();
    let start = Instant::now();
    let first = input.frame(window(&[key(egui::Key::W, true)], true, start));
    assert_eq!(first.dt, 0.0);
    assert!(first.held(egui::Key::W) && first.pressed.contains(&egui::Key::W));
    let second = input.frame(window(&[], true, start + Duration::from_millis(16)));
    assert!((second.dt - 0.016).abs() < 1e-6);
    assert!(second.held(egui::Key::W) && second.pressed.is_empty());
    let stalled = input.frame(window(&[], true, start + Duration::from_secs(3)));
    assert_eq!(stalled.dt, LONGEST_STEP);
    let released = input.frame(window(
        &[key(egui::Key::W, false)],
        true,
        start + Duration::from_secs(4),
    ));
    assert!(!released.held(egui::Key::W));
}

#[test]
fn escape_and_focus_loss_release_the_pointer_and_held_input() {
    let mut input = Input::default();
    let now = Instant::now();
    input.capture();
    take(
        &mut input,
        OWNER,
        GamepadState {
            sticks: [[0.0, 1.0], [0.0; 2]],
            ..GamepadState::default()
        },
    );
    let motion = egui::Event::MouseMoved(egui::vec2(4.0, -2.0));
    let frame = input.frame(window(
        &[key(egui::Key::D, true), motion.clone()],
        true,
        now,
    ));
    assert!(frame.captured && frame.held(egui::Key::D));
    assert_eq!(frame.look, [4.0, -2.0]);
    assert!(frame.pad.left[1] > 0.99);
    let frame = input.frame(window(
        &[key(egui::Key::Escape, true), motion],
        true,
        now,
    ));
    assert!(!frame.captured && frame.held.is_empty());
    assert_eq!(frame.look, [0.0; 2]);
    assert!(frame.pad.active());
    input.capture();
    let frame = input.frame(window(&[key(egui::Key::D, true)], false, now));
    assert!(!frame.captured && frame.held.is_empty() && frame.pad.active());
    update(
        &mut input,
        OWNER,
        GamepadState {
            triggers: [0.0, 1.0],
            ..GamepadState::default()
        },
    );
    assert!(input.frame(window(&[], false, now)).pad.active());
}

#[test]
fn double_taps_and_button_presses_are_reported_once() {
    let mut input = Input::default();
    let start = Instant::now();
    let tap = |input: &mut Input, at: u64| {
        let events = [key(egui::Key::Space, true), key(egui::Key::Space, false)];
        input.frame(window(&events, true, start + Duration::from_millis(at)))
    };
    assert!(!tap(&mut input, 0).double_tapped(egui::Key::Space));
    assert!(tap(&mut input, 200).double_tapped(egui::Key::Space));
    assert!(!tap(&mut input, 350).double_tapped(egui::Key::Space));
    assert!(!tap(&mut input, 1000).double_tapped(egui::Key::Space));
    take(&mut input, OWNER, GamepadState::default());
    update(
        &mut input,
        OWNER,
        GamepadState {
            buttons: GamepadButton::RightStick.bit(),
            ..GamepadState::default()
        },
    );
    let pressed = input.frame(window(&[], true, start));
    assert!(pressed.pad.pressed(GamepadButton::RightStick));
    let held = input.frame(window(&[], true, start));
    assert!(
        held.pad.held(GamepadButton::RightStick) && !held.pad.pressed(GamepadButton::RightStick)
    );
    let press = |input: &mut Input, buttons: u32, at: u64| {
        update(
            input,
            OWNER,
            GamepadState {
                buttons,
                ..GamepadState::default()
            },
        );
        input
            .frame(window(&[], true, start + Duration::from_millis(at)))
            .pad
    };
    let south = GamepadButton::South.bit();
    assert!(!press(&mut input, south, 1000).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, 0, 1100).double_tapped(GamepadButton::South));
    assert!(press(&mut input, south, 1200).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, 0, 1250).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, south, 1300).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, 0, 1400).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, south, 1800).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, 0, 2500).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, south, 2600).double_tapped(GamepadButton::South));
    input.forget_pad_taps();
    assert!(!press(&mut input, 0, 2650).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, south, 2780).double_tapped(GamepadButton::South));
}

#[test]
fn a_press_released_again_before_the_frame_is_pressed_once() {
    let mut input = Input::default();
    let now = Instant::now();
    take(&mut input, OWNER, GamepadState::default());
    let south = GamepadButton::South.bit();
    for pressed in [south, 0] {
        input.gamepad(GamepadMessage::State {
            owner: OWNER,
            state: GamepadState::default(),
            pressed,
        });
    }
    let frame = input.frame(window(&[], true, now));
    assert!(frame.pad.pressed(GamepadButton::South) && !frame.pad.held(GamepadButton::South));
    let next = input.frame(window(&[], true, now));
    assert!(!next.pad.pressed(GamepadButton::South));
}

fn placed(
    key: egui::Key,
    physical: egui::Key,
    pressed: bool,
    modifiers: egui::Modifiers,
) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(physical),
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn keys_are_held_by_their_place_on_the_keyboard() {
    let mut input = Input::default();
    let now = Instant::now();
    let down = placed(
        egui::Key::Plus,
        egui::Key::Equals,
        true,
        egui::Modifiers::SHIFT,
    );
    let up = placed(
        egui::Key::Equals,
        egui::Key::Equals,
        false,
        egui::Modifiers::NONE,
    );
    let frame = input.frame(window(&[down], true, now));
    assert!(frame.held(egui::Key::Equals) && frame.pressed == [egui::Key::Equals]);
    assert!(input.frame(window(&[up], true, now)).held.is_empty());
    let azerty = placed(egui::Key::Z, egui::Key::W, true, egui::Modifiers::NONE);
    let frame = input.frame(window(&[azerty], true, now));
    assert!(frame.held(egui::Key::W) && !frame.held(egui::Key::Z));
    let tap = [
        placed(egui::Key::Z, egui::Key::W, false, egui::Modifiers::NONE),
        placed(egui::Key::Z, egui::Key::W, true, egui::Modifiers::NONE),
    ];
    assert!(
        input
            .frame(window(&tap, true, now))
            .double_tapped(egui::Key::W)
    );
    let frame = input.frame(window(&[key(egui::Key::Space, true)], true, now));
    assert!(frame.held(egui::Key::Space));
}

#[test]
fn a_released_map_drops_states_sent_before_its_release() {
    let mut input = Input::default();
    let now = Instant::now();
    let ahead = GamepadState {
        sticks: [[0.0, 1.0], [0.0; 2]],
        ..GamepadState::default()
    };
    take(&mut input, OWNER, ahead);
    assert!(input.frame(window(&[], true, now)).pad.active());
    assert!(input.frame(window(&[], false, now)).pad.active());
    input.gamepad(GamepadMessage::Release);
    assert!(!input.frame(window(&[], false, now)).pad.active());
    update(&mut input, OWNER, ahead);
    assert!(!input.frame(window(&[], false, now)).pad.active());
    let other = GamepadOwner {
        viewer: 1,
        handoff: 2,
    };
    update(&mut input, other, ahead);
    assert!(!input.frame(window(&[], true, now)).pad.active());
    take(&mut input, other, ahead);
    assert!(input.frame(window(&[], true, now)).pad.active());
}

#[test]
fn a_held_button_is_never_pressed_again_by_a_handoff_or_a_lost_focus() {
    let now = Instant::now();
    let view = GamepadButton::Select;
    let holding = GamepadState {
        buttons: view.bit(),
        ..GamepadState::default()
    };
    let mut campaign = Input::default();
    take(&mut campaign, OWNER, GamepadState::default());
    campaign.frame(window(&[], true, now));
    update(&mut campaign, OWNER, holding);
    assert!(campaign.frame(window(&[], true, now)).pad.pressed(view));
    campaign.gamepad(GamepadMessage::Release);
    let mut popout = Input::default();
    let second = GamepadOwner {
        viewer: 4,
        handoff: 2,
    };
    take(&mut popout, second, holding);
    let frame = popout.frame(window(&[], true, now));
    assert!(frame.pad.held(view) && !frame.pad.pressed(view) && frame.pad.double_tapped == 0);
    let third = GamepadOwner {
        viewer: 1,
        handoff: 3,
    };
    take(&mut campaign, third, holding);
    let frame = campaign.frame(window(&[], true, now));
    assert!(frame.pad.held(view) && !frame.pad.pressed(view));
    campaign.frame(window(&[], false, now));
    let moved = GamepadState {
        sticks: [[0.5, 0.0], [0.0; 2]],
        ..holding
    };
    update(&mut campaign, third, moved);
    assert!(!campaign.frame(window(&[], true, now)).pad.pressed(view));
    update(&mut campaign, third, GamepadState::default());
    campaign.frame(window(&[], true, now));
    update(&mut campaign, third, holding);
    assert!(campaign.frame(window(&[], true, now)).pad.pressed(view));
}

#[test]
fn a_focus_lost_and_regained_in_one_frame_keeps_a_newer_handoff() {
    let mut input = Input::default();
    let now = Instant::now();
    let ahead = GamepadState {
        sticks: [[0.0, 1.0], [0.0; 2]],
        ..GamepadState::default()
    };
    take(&mut input, OWNER, ahead);
    assert!(input.frame(window(&[], true, now)).pad.active());
    input.gamepad(GamepadMessage::Release);
    let back = GamepadOwner {
        viewer: 1,
        handoff: 3,
    };
    take(&mut input, back, ahead);
    let events = [
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
    ];
    let frame = input.frame(window(&events, true, now));
    assert!(frame.pad.active() && frame.pad.left[1] > 0.99);
    assert!(input.frame(window(&[], true, now)).pad.active());
}
