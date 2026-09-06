use serde::{Deserialize, Serialize};
use portable_render_stream::Xmap15ViewerMissingInput as Missing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Xmap15ArrowUiCapture {
    #[default]
    Unavailable,
    NoContext,
    Context { keyboard: bool, text_input: bool },
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
            Xmap15ArrowUiCapture::Unavailable => return Err(Missing::ViewerKeyboardCapture),
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
}
