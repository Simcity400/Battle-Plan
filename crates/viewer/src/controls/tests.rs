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

/// The host hands this map the controller with `state` held.
fn take(input: &mut Input, owner: GamepadOwner, state: GamepadState) {
    input.gamepad(GamepadMessage::Take { owner, state });
}

/// The controller this map holds under `owner` changed.
fn update(input: &mut Input, owner: GamepadOwner, state: GamepadState) {
    input.gamepad(GamepadMessage::State { owner, state });
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
    // Full deflection saturates before the rim and keeps its direction.
    let full = stick([0.0, -0.97]);
    assert!((full[1] + 1.0).abs() < 1e-6 && full[0] == 0.0);
    let diagonal = stick([0.7, 0.7]);
    assert!((diagonal[0] - diagonal[1]).abs() < 1e-6);
    assert!((diagonal[0].hypot(diagonal[1]) - 1.0).abs() < 1e-5);
    // Half way past the dead zone gives about a third of full speed.
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
    // A stall moves the camera no more than the longest step.
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
    // Escape gives the pointer back and forgets the keys; the controller stays.
    let frame = input.frame(window(
        &[key(egui::Key::Escape, true), motion.clone()],
        true,
        now,
    ));
    assert!(!frame.captured && frame.held.is_empty());
    assert_eq!(frame.look, [0.0; 2]);
    assert!(frame.pad.active());
    // Losing focus gives the pointer back and forgets the keys too; the controller stays with the
    // map until the host releases it.
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
    // A third tap starts a new pair.
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
    // A controller button pressed twice within the double-tap time is a double tap, once.
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
    // Too slow a second press starts a new pair.
    assert!(!press(&mut input, 0, 1400).double_tapped(GamepadButton::South));
    assert!(!press(&mut input, south, 1800).double_tapped(GamepadButton::South));
}

/// A key event with its place on the keyboard.
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
    // Shift+= arrives as Plus; with Shift released first, the key comes up as Equals.
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
    // On AZERTY the key where W stands reads Z, and is still W: forward.
    let azerty = placed(egui::Key::Z, egui::Key::W, true, egui::Modifiers::NONE);
    let frame = input.frame(window(&[azerty], true, now));
    assert!(frame.held(egui::Key::W) && !frame.held(egui::Key::Z));
    // A double tap counts by place too.
    let tap = [
        placed(egui::Key::Z, egui::Key::W, false, egui::Modifiers::NONE),
        placed(egui::Key::Z, egui::Key::W, true, egui::Modifiers::NONE),
    ];
    assert!(
        input
            .frame(window(&tap, true, now))
            .double_tapped(egui::Key::W)
    );
    // A key the window places nowhere is held by what it means.
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
    // The pop-out holds the stick ahead, then loses focus to the main window; only the host's
    // release lets go.
    take(&mut input, OWNER, ahead);
    assert!(input.frame(window(&[], true, now)).pad.active());
    assert!(input.frame(window(&[], false, now)).pad.active());
    // A state sent before the host handed the controller on arrives after the release.
    input.gamepad(GamepadMessage::Release);
    assert!(!input.frame(window(&[], false, now)).pad.active());
    update(&mut input, OWNER, ahead);
    assert!(!input.frame(window(&[], false, now)).pad.active());
    // States of a handoff the map does not hold never move it either.
    let other = GamepadOwner {
        viewer: 1,
        handoff: 2,
    };
    update(&mut input, other, ahead);
    assert!(!input.frame(window(&[], true, now)).pad.active());
    // Handed back the controller, it moves again.
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
    // View pressed in the campaign map: one press there.
    let mut campaign = Input::default();
    take(&mut campaign, OWNER, GamepadState::default());
    campaign.frame(window(&[], true, now));
    update(&mut campaign, OWNER, holding);
    assert!(campaign.frame(window(&[], true, now)).pad.pressed(view));
    // Still held, the controller goes to the pop-out: held there, not pressed.
    campaign.gamepad(GamepadMessage::Release);
    let mut popout = Input::default();
    let second = GamepadOwner {
        viewer: 4,
        handoff: 2,
    };
    take(&mut popout, second, holding);
    let frame = popout.frame(window(&[], true, now));
    assert!(frame.pad.held(view) && !frame.pad.pressed(view) && frame.pad.double_tapped == 0);
    // And back again, still held: the campaign map does not press it either.
    let third = GamepadOwner {
        viewer: 1,
        handoff: 3,
    };
    take(&mut campaign, third, holding);
    let frame = campaign.frame(window(&[], true, now));
    assert!(frame.pad.held(view) && !frame.pad.pressed(view));
    // Held through a lost and regained focus and a moved stick: no press either.
    campaign.frame(window(&[], false, now));
    let moved = GamepadState {
        sticks: [[0.5, 0.0], [0.0; 2]],
        ..holding
    };
    update(&mut campaign, third, moved);
    assert!(!campaign.frame(window(&[], true, now)).pad.pressed(view));
    // Let go and pressed again: one real press.
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
    // Map A holds the stick ahead.
    take(&mut input, OWNER, ahead);
    assert!(input.frame(window(&[], true, now)).pad.active());
    // The user switches to B and straight back before A's next frame: A's release and its new
    // take arrive together, as the queue merges them, before that frame.
    input.gamepad(GamepadMessage::Release);
    let back = GamepadOwner {
        viewer: 1,
        handoff: 3,
    };
    take(&mut input, back, ahead);
    // That frame carries the focus lost and regained; the host holds the stick steady and sends
    // nothing more, yet A keeps moving.
    let events = [
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
    ];
    let frame = input.frame(window(&events, true, now));
    assert!(frame.pad.active() && frame.pad.left[1] > 0.99);
    assert!(input.frame(window(&[], true, now)).pad.active());
}
