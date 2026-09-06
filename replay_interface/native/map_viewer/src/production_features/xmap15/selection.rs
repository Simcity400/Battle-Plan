use serde::{Deserialize, Serialize};
use portable_render_stream::Xmap15ViewerMissingInput as Missing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Xmap15ArrowUiCapture {
    #[default]
    Unavailable,
    NoContext,
    Context { keyboard: bool, text_input: bool },
    ViewerEventRouting {
        alt_map_owned: [Option<bool>; 2],
        control_map_owned: [Option<bool>; 2],
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ArrowKeyboard {
    pub alt: Option<[bool; 2]>,
    pub control: Option<[bool; 2]>,
    pub ui: Xmap15ArrowUiCapture,
}

impl Xmap15ArrowKeyboard {
    fn modifiers(self) -> Result<(bool, bool), Missing> {
        match self.ui {
            Xmap15ArrowUiCapture::Unavailable => {
                // Suppression can only turn a pressed key off.
                if self.alt == Some([false; 2]) && self.control == Some([false; 2]) {
                    return Ok((false, false));
                }
                return Err(Missing::ViewerKeyboardCapture);
            },
            Xmap15ArrowUiCapture::ViewerEventRouting { alt_map_owned, control_map_owned } => {
                let effective = |pressed: Option<[bool; 2]>, owned: [Option<bool>; 2]| -> Result<bool, Missing> {
                    let pressed = pressed.ok_or(Missing::ViewerModifiers)?;
                    let mut result = false;
                    for side in 0..2 {
                        if pressed[side] {
                            result |= owned[side].ok_or(Missing::ViewerKeyboardCapture)?;
                        }
                    }
                    Ok(result)
                };
                return Ok((effective(self.alt, alt_map_owned)?,
                    effective(self.control, control_map_owned)?));
            },
            Xmap15ArrowUiCapture::Context { keyboard, text_input } if keyboard || text_input =>
                return Ok((false, false)),
            _ => {}
        }
        let alt = self.alt.ok_or(Missing::ViewerModifiers)?;
        let control = self.control.ok_or(Missing::ViewerModifiers)?;
        Ok((alt[0] || alt[1], control[0] || control[1]))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Xmap15ArrowColourCallback {
    #[default]
    Unavailable,
    Absent,
    Installed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Xmap15ArrowInteraction {
    #[default]
    Unavailable,
    // Witnessed tool 0, flags +21c/+220/+221/+260/+270 clear, no active edit point or pending +2a8.
    Ordinary,
    Editing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ViewerPresentation {
    pub keyboard: Xmap15ArrowKeyboard,
    pub interaction: Xmap15ArrowInteraction,
    pub colour_callback: Xmap15ArrowColourCallback,
}

impl Xmap15ViewerPresentation {
    pub(super) fn held_alt_can_preserve_selected_green(self) -> bool {
        self.interaction == Xmap15ArrowInteraction::Ordinary
            && self.colour_callback == Xmap15ArrowColourCallback::Absent
            && self.keyboard.control == Some([false; 2])
            && self.keyboard.alt.is_some_and(|keys| keys[0] || keys[1])
    }

    pub(super) fn selected_paradrop(self) -> Result<(), Missing> {
        match self.interaction {
            Xmap15ArrowInteraction::Unavailable => return Err(Missing::ViewerInteraction),
            Xmap15ArrowInteraction::Editing => return Err(Missing::SelectedPresentation),
            Xmap15ArrowInteraction::Ordinary => {}
        }
        let (alt, control) = self.keyboard.modifiers()?;
        if alt || control { return Err(Missing::SelectedPresentation); }
        match self.colour_callback {
            Xmap15ArrowColourCallback::Unavailable => Err(Missing::ViewerColourCallback),
            Xmap15ArrowColourCallback::Installed => Err(Missing::SelectedPresentation),
            Xmap15ArrowColourCallback::Absent => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_capture_suppresses_both_sides_and_unknown_is_not_released() {
        for mask in 0..16 {
            let keyboard = Xmap15ArrowKeyboard { alt: Some([mask & 1 != 0, mask & 2 != 0]),
                control: Some([mask & 4 != 0, mask & 8 != 0]), ui: Xmap15ArrowUiCapture::NoContext };
            assert_eq!(keyboard.modifiers(), Ok((mask & 3 != 0, mask & 12 != 0)));
            let unknown_capture = Xmap15ArrowKeyboard { ui: Xmap15ArrowUiCapture::Unavailable, ..keyboard };
            assert_eq!(unknown_capture.modifiers(), if mask == 0 { Ok((false, false)) }
                else { Err(Missing::ViewerKeyboardCapture) });
            for (capture, text) in [(false, false), (true, false), (false, true), (true, true)] {
                let observed = Xmap15ArrowKeyboard { ui: Xmap15ArrowUiCapture::Context {
                    keyboard: capture, text_input: text }, ..keyboard };
                assert_eq!(observed.modifiers(), Ok(if capture || text { (false, false) }
                    else { (mask & 3 != 0, mask & 12 != 0) }));
            }
        }
        assert_eq!(Xmap15ArrowKeyboard::default().modifiers(), Err(Missing::ViewerKeyboardCapture));
        for (alt, control) in [(None, Some([false; 2])), (Some([false; 2]), None), (None, None)] {
            assert_eq!(Xmap15ArrowKeyboard { alt, control, ui: Xmap15ArrowUiCapture::NoContext }.modifiers(),
                Err(Missing::ViewerModifiers));
        }
    }

    #[test]
    fn held_alt_convergence_requires_released_control_and_proven_callback_and_tool() {
        for mask in 0u8..16 {
            for ui in [Xmap15ArrowUiCapture::Unavailable, Xmap15ArrowUiCapture::NoContext,
                Xmap15ArrowUiCapture::Context { keyboard: true, text_input: false },
                Xmap15ArrowUiCapture::Context { keyboard: false, text_input: true },
                Xmap15ArrowUiCapture::Context { keyboard: false, text_input: false }] {
                let input = Xmap15ViewerPresentation {
                    keyboard: Xmap15ArrowKeyboard { alt: Some([mask & 1 != 0, mask & 2 != 0]),
                        control: Some([mask & 4 != 0, mask & 8 != 0]), ui },
                    interaction: Xmap15ArrowInteraction::Ordinary,
                    colour_callback: Xmap15ArrowColourCallback::Absent,
                };
                assert_eq!(input.held_alt_can_preserve_selected_green(), mask & 3 != 0 && mask & 12 == 0);
                for interaction in [Xmap15ArrowInteraction::Unavailable, Xmap15ArrowInteraction::Editing] {
                    assert!(!Xmap15ViewerPresentation { interaction, ..input }.held_alt_can_preserve_selected_green());
                }
                for colour_callback in [Xmap15ArrowColourCallback::Unavailable, Xmap15ArrowColourCallback::Installed] {
                    assert!(!Xmap15ViewerPresentation { colour_callback, ..input }.held_alt_can_preserve_selected_green());
                }
                for keyboard in [Xmap15ArrowKeyboard { alt: None, ..input.keyboard },
                    Xmap15ArrowKeyboard { control: None, ..input.keyboard }] {
                    assert!(!Xmap15ViewerPresentation { keyboard, ..input }.held_alt_can_preserve_selected_green());
                }
            }
        }
    }

    #[test]
    fn ordinary_selected_branch_requires_each_independent_presentation_witness() {
        let exact = Xmap15ViewerPresentation { interaction: Xmap15ArrowInteraction::Ordinary,
            colour_callback: Xmap15ArrowColourCallback::Absent,
            keyboard: Xmap15ArrowKeyboard { alt: Some([false; 2]), control: Some([false; 2]),
                ui: Xmap15ArrowUiCapture::NoContext } };
        assert_eq!(exact.selected_paradrop(), Ok(()));
        assert_eq!(Xmap15ViewerPresentation { interaction: Xmap15ArrowInteraction::Unavailable, ..exact }
            .selected_paradrop(), Err(Missing::ViewerInteraction));
        assert_eq!(Xmap15ViewerPresentation { colour_callback: Xmap15ArrowColourCallback::Unavailable, ..exact }
            .selected_paradrop(), Err(Missing::ViewerColourCallback));
        for input in [Xmap15ViewerPresentation { interaction: Xmap15ArrowInteraction::Editing, ..exact },
            Xmap15ViewerPresentation { colour_callback: Xmap15ArrowColourCallback::Installed, ..exact }] {
            assert_eq!(input.selected_paradrop(), Err(Missing::SelectedPresentation));
        }
    }
    #[test]
    fn viewer_event_routes_suppress_only_the_consumed_modifier_side() {
        let keyboard = Xmap15ArrowKeyboard {
            alt: Some([true, true]), control: Some([true, true]),
            ui: Xmap15ArrowUiCapture::ViewerEventRouting {
                alt_map_owned: [Some(false), Some(true)],
                control_map_owned: [Some(true), Some(false)],
            },
        };
        assert_eq!(keyboard.modifiers(), Ok((true, true)));
        let released = Xmap15ArrowKeyboard { alt: Some([true, false]), control: Some([false, true]), ..keyboard };
        assert_eq!(released.modifiers(), Ok((false, false)));
        assert_eq!(Xmap15ArrowKeyboard { alt: None, ..keyboard }.modifiers(), Err(Missing::ViewerModifiers));
        let unknown = Xmap15ArrowKeyboard {
            ui: Xmap15ArrowUiCapture::ViewerEventRouting {
                alt_map_owned: [None; 2], control_map_owned: [None; 2],
            }, ..keyboard
        };
        assert_eq!(unknown.modifiers(), Err(Missing::ViewerKeyboardCapture));
        assert_eq!(Xmap15ArrowKeyboard { alt: Some([false; 2]), control: Some([false; 2]), ..unknown }
            .modifiers(), Ok((false, false)));
        assert_ne!(keyboard.ui, Xmap15ArrowUiCapture::Context { keyboard: false, text_input: false });
    }

}
