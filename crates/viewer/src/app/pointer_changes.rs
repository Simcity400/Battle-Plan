
use eframe::egui;

pub(super) type Change = (egui::Pos2, Option<(egui::PointerButton, bool)>);

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
