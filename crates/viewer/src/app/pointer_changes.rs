//! A frame's pointer moves and primary and secondary button changes, in order, each with the
//! modifiers its own event carried. The game reads Ctrl, Shift and Alt with the click; the frame's
//! final modifiers can already have let go of a key that was down when the button changed.

use eframe::egui;

/// One pointer change: where it happened, and the button that changed and whether it went down
/// (`None` for a move).
pub(super) type Change = (egui::Pos2, Option<(egui::PointerButton, bool)>);

/// The frame's changes, and beside each the modifiers it came with; a move takes the frame's.
pub(super) fn changes(input: &egui::InputState) -> (Vec<Change>, Vec<egui::Modifiers>) {
    input
        .events
        .iter()
        .filter_map(|event| match event {
            egui::Event::PointerMoved(pos) => Some(((*pos, None), input.modifiers)),
            egui::Event::PointerButton {
                pos,
                button: button @ (egui::PointerButton::Primary | egui::PointerButton::Secondary),
                pressed,
                modifiers,
            } => Some(((*pos, Some((*button, *pressed))), *modifiers)),
            _ => None,
        })
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Ctrl+click whose Ctrl lifts right after the button, in the same frame: the frame ends with
    /// no Ctrl, and the click still has it.
    #[test]
    fn a_click_keeps_the_modifiers_its_button_changed_with() {
        let ctx = egui::Context::default();
        let at = egui::pos2(40., 40.);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::CTRL };
        let mut seen = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                    egui::Event::PointerMoved(at),
                    button(true),
                    button(false),
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
                ..Default::default()
            },
            |ui| seen = Some(ui.input(|input| (input.modifiers, changes(input)))),
        );
        let (frame, (changes, modifiers)) = seen.unwrap();
        assert!(!frame.ctrl, "the frame ends with Ctrl up");
        assert_eq!(changes, vec![(at, None), (at, Some((egui::PointerButton::Primary, true))), (at, Some((egui::PointerButton::Primary, false)))]);
        assert_eq!(modifiers[1..], [egui::Modifiers::CTRL; 2], "the press and the release keep Ctrl");
    }
}
