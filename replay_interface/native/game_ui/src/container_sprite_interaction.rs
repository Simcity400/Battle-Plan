//! Effect operations for a witnessed ordinary CIcon owning a CSprite.
//!
//! CIconType +68 -> 142358760 -> 1422e5d40 and CGui +a0 registration
//! leave the fresh drawable effect unassigned. CIcon is a separate owner from
//! CButtonStandard: input state writes and observer callbacks are not implicit
//! sprite effect assignments. The caller stages this state with the borrowed
//! SpriteEffectInputs and commits both only with successful presentation.

use crate::container_sprite_inputs::{NativeButtonEffect, SpriteEffectInputs};
use crate::runtime_graphics::ButtonVisualState;

/// Event kind carried by the CIcon +378 observer dispatcher. The caller owns
/// the admitted observer chain and its operations; unavailable observers must
/// return an error rather than being replaced by a no-op callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconObserverEvent {
    Over = 3,
    Leave = 4,
}

/// An actual operation of the owning CIcon, not a desired final draw effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconOperation {
    Disable,
    Enable { disable_locked: bool },
    SetState(ButtonVisualState),
    WriteInputState(ButtonVisualState),
    ForceDisabledCallback(bool),
    FinishInputUpdate,
}

/// Only the CIcon fields needed at the reached effect-write/callback boundary.
/// The drawable's dirty flag and clock remain solely in SpriteEffectInputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IconInteractionInputs {
    widget_state: ButtonVisualState,
    effect: NativeButtonEffect,
    force_disabled_callback: bool,
    notified_over: bool,
}

impl IconInteractionInputs {
    pub(crate) fn apply<E>(
        &mut self,
        drawable: &mut SpriteEffectInputs,
        operation: IconOperation,
        dispatch: impl FnMut(IconObserverEvent, &mut Self, &mut SpriteEffectInputs) -> Result<(), E>,
    ) -> Result<(), E> {
        match operation {
            IconOperation::Disable => self.disable(drawable),
            IconOperation::Enable { disable_locked } => self.enable(drawable, disable_locked),
            IconOperation::SetState(state) => self.set_state(drawable, state),
            IconOperation::WriteInputState(state) => self.write_input_state(state),
            IconOperation::ForceDisabledCallback(value) => self.set_force_disabled_callback(value),
            IconOperation::FinishInputUpdate => {
                return self.finish_input_update(drawable, dispatch)
            }
        }
        Ok(())
    }

    /// Requires the actual ordinary CIcon factory/registration path and its
    /// fresh CSprite. It does not reconstruct a missing retained instance.
    /// CButton 142299f30 writes widget+b0=0 and +e4=0xb8; CIcon 1422e5d40
    /// clears +155 bit0. Neither constructor nor CGui 142245830 assigns Up.
    pub(crate) fn from_icon_constructor() -> Self {
        Self {
            widget_state: ButtonVisualState::Up,
            effect: NativeButtonEffect::Unassigned,
            force_disabled_callback: false,
            notified_over: false,
        }
    }

    pub(crate) fn effect(&self) -> NativeButtonEffect {
        self.effect
    }

    /// CIcon +278 -> 14229abe0 reads widget+b0, independently of the drawable
    /// effect. In particular a new icon has widget Up and effect Unassigned.
    pub(crate) fn widget_state(&self) -> ButtonVisualState {
        self.widget_state
    }

    /// Reached direct widget+b0 bookkeeping by the input owner. This is not
    /// CIcon +280 SetState. The caller establishes hit/press ownership and the
    /// native input branch; this module does not infer either from geometry.
    pub(crate) fn write_input_state(&mut self, state: ButtonVisualState) {
        self.widget_state = state;
    }

    /// 14229bcf0 controls widget+e4 bit0x40 without assigning a drawable effect.
    pub(crate) fn set_force_disabled_callback(&mut self, value: bool) {
        self.force_disabled_callback = value;
    }

    fn assign_effect(&mut self, drawable: &mut SpriteEffectInputs, effect: NativeButtonEffect) {
        drawable.set_effect(effect as i32);
        self.effect = effect;
    }

    /// CIcon +280 -> 1422e6e30: base state store, then native state+1 effect.
    /// These are the admitted four CButton states; arbitrary native i32 values
    /// outside that state domain are not represented by this API.
    pub(crate) fn set_state(
        &mut self,
        drawable: &mut SpriteEffectInputs,
        state: ButtonVisualState,
    ) {
        self.widget_state = state;
        self.assign_effect(drawable, state.into());
    }

    /// CIcon +288 -> 1422e6490. Base14229abb0 clears state3 only when widget+74
    /// bit7 is clear; the subsequent drawable Up assignment is unconditional.
    /// `disable_locked` is that actual flag, supplied by the widget owner.
    pub(crate) fn enable(&mut self, drawable: &mut SpriteEffectInputs, disable_locked: bool) {
        if !disable_locked && self.widget_state == ButtonVisualState::Disable {
            self.widget_state = ButtonVisualState::Up;
        }
        self.assign_effect(drawable, NativeButtonEffect::Up);
    }

    /// CIcon +290 -> 1422e6460. Base14229ab10 sets state3 and invokes +378;
    /// CIcon's 1422e5fa0 emits no notification for state3, then effect4 is set.
    pub(crate) fn disable(&mut self, drawable: &mut SpriteEffectInputs) {
        self.widget_state = ButtonVisualState::Disable;
        self.assign_effect(drawable, NativeButtonEffect::Disable);
    }

    /// Reached tail of 14229bd10 after the input owner has performed its state
    /// bookkeeping. Preserve both forced writes even if the final effect equals
    /// the previous draw's effect: intermediate changes latch drawable dirty.
    /// CIcon +278 raw142b34418 ->14229abe0 supplies the saved widget state.
    pub(crate) fn finish_input_update<E>(
        &mut self,
        drawable: &mut SpriteEffectInputs,
        mut dispatch: impl FnMut(IconObserverEvent, &mut Self, &mut SpriteEffectInputs) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.force_disabled_callback {
            let saved = self.widget_state;
            self.set_state(drawable, ButtonVisualState::Disable);
            self.dispatch_state_callback(drawable, &mut dispatch)?;
            self.set_state(drawable, saved);
        } else {
            self.dispatch_state_callback(drawable, &mut dispatch)?;
        }
        Ok(())
    }

    /// Exact CIcon +378 ->1422e5fa0 continuation. The second state test occurs
    /// after the Leave callback and its flag clear; observer operations can
    /// therefore cause an Over notification in this same invocation.
    fn dispatch_state_callback<E>(
        &mut self,
        drawable: &mut SpriteEffectInputs,
        dispatch: &mut impl FnMut(
            IconObserverEvent,
            &mut Self,
            &mut SpriteEffectInputs,
        ) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.widget_state == ButtonVisualState::Up && self.notified_over {
            dispatch(IconObserverEvent::Leave, self, drawable)?;
            self.notified_over = false;
        }
        if self.widget_state == ButtonVisualState::Over {
            dispatch(IconObserverEvent::Over, self, drawable)?;
            self.notified_over = true;
        }
        Ok(())
    }

    /// 1422e6050/1422e6180 copy the OLD drawable effect through NEW +98.
    /// The caller constructs the replacement and handles its separate animation
    /// and geometry lifetimes. Owner state/notification flags remain retained.
    pub(crate) fn attach_replacement(&self, replacement: &mut SpriteEffectInputs) {
        replacement.set_effect(self.effect as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_observers(
        _: IconObserverEvent,
        _: &mut IconInteractionInputs,
        _: &mut SpriteEffectInputs,
    ) -> Result<(), ()> {
        Ok(())
    }

    #[test]
    fn ordinary_factory_and_hover_bookkeeping_preserve_unassigned_effect() {
        let mut icon = IconInteractionInputs::from_icon_constructor();
        let mut drawable = SpriteEffectInputs::fresh();
        let fresh = drawable;
        let mut events = Vec::new();
        for state in [
            ButtonVisualState::Up,
            ButtonVisualState::Over,
            ButtonVisualState::Over,
            ButtonVisualState::Down,
            ButtonVisualState::Up,
        ] {
            icon.write_input_state(state);
            icon.finish_input_update(&mut drawable, |event, _, _| {
                events.push(event);
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        assert_eq!(
            events,
            [
                IconObserverEvent::Over,
                IconObserverEvent::Over,
                IconObserverEvent::Leave
            ]
        );
        assert_eq!(icon.widget_state(), ButtonVisualState::Up);
        assert_eq!(icon.effect(), NativeButtonEffect::Unassigned);
        assert_eq!(drawable, fresh);
        assert_eq!(drawable.stage_generic_draw(10.0).draw_clock.to_bits(), 0);
    }

    #[test]
    fn enable_assigns_up_even_when_base_disabled_state_is_locked() {
        let mut icon = IconInteractionInputs::from_icon_constructor();
        let mut drawable = SpriteEffectInputs::fresh();
        icon.disable(&mut drawable);
        drawable = drawable.stage_generic_draw(2.0).next;
        icon.enable(&mut drawable, true);
        assert_eq!(icon.widget_state(), ButtonVisualState::Disable);
        assert_eq!(icon.effect(), NativeButtonEffect::Up);
        let draw = drawable.stage_generic_draw(3.0);
        assert_eq!(draw.draw_clock, 3.0);
        drawable = draw.next;
        icon.enable(&mut drawable, false);
        assert_eq!(icon.widget_state(), ButtonVisualState::Up);
        assert_eq!(drawable.stage_generic_draw(4.0).draw_clock, 3.0);
    }

    #[test]
    fn forced_update_preserves_transient_disable_dirty_latch() {
        let mut icon = IconInteractionInputs::from_icon_constructor();
        let mut drawable = SpriteEffectInputs::fresh();
        icon.enable(&mut drawable, false);
        drawable = drawable.stage_generic_draw(1.0).next;
        icon.set_force_disabled_callback(true);
        icon.finish_input_update(&mut drawable, |_, _, _| -> Result<(), ()> {
            panic!("disabled CIcon callback cannot notify observers")
        })
        .unwrap();
        assert_eq!(icon.widget_state(), ButtonVisualState::Up);
        assert_eq!(icon.effect(), NativeButtonEffect::Up);
        assert_eq!(drawable.stage_generic_draw(2.0).draw_clock, 2.0);
    }

    #[test]
    fn forced_restore_uses_widget_state_and_suppresses_hover_notifications() {
        let mut icon = IconInteractionInputs::from_icon_constructor();
        let mut drawable = SpriteEffectInputs::fresh();
        icon.write_input_state(ButtonVisualState::Over);
        icon.set_force_disabled_callback(true);
        icon.finish_input_update(&mut drawable, |_, _, _| -> Result<(), ()> {
            panic!("forced callback sees disabled state")
        })
        .unwrap();
        assert_eq!(icon.widget_state(), ButtonVisualState::Over);
        assert_eq!(icon.effect(), NativeButtonEffect::Over);
        icon.set_force_disabled_callback(false);
        let mut events = Vec::new();
        icon.finish_input_update(&mut drawable, |event, _, _| {
            events.push(event);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(events, [IconObserverEvent::Over]);
    }

    #[test]
    fn leave_callback_state_write_is_observed_by_second_state_test() {
        let mut icon = IconInteractionInputs::from_icon_constructor();
        let mut drawable = SpriteEffectInputs::fresh();
        icon.write_input_state(ButtonVisualState::Over);
        icon.finish_input_update(&mut drawable, no_observers)
            .unwrap();
        icon.write_input_state(ButtonVisualState::Up);
        let mut events = Vec::new();
        icon.finish_input_update(&mut drawable, |event, icon, drawable| {
            events.push(event);
            if event == IconObserverEvent::Leave {
                icon.set_state(drawable, ButtonVisualState::Over);
            }
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(events, [IconObserverEvent::Leave, IconObserverEvent::Over]);
        assert_eq!(icon.effect(), NativeButtonEffect::Over);
        assert_eq!(drawable.stage_generic_draw(6.0).draw_clock, 6.0);
    }

    #[test]
    fn replacement_copies_effect_through_new_setter_without_resetting_owner() {
        let mut icon = IconInteractionInputs::from_icon_constructor();
        let mut original = SpriteEffectInputs::fresh();
        icon.set_state(&mut original, ButtonVisualState::Down);
        let retained_owner = icon;
        let mut replacement = SpriteEffectInputs::fresh();
        icon.attach_replacement(&mut replacement);
        assert_eq!(icon, retained_owner);
        assert_eq!(replacement.stage_generic_draw(7.0).draw_clock, 7.0);
        let fresh_icon = IconInteractionInputs::from_icon_constructor();
        let mut fresh_replacement = SpriteEffectInputs::fresh();
        fresh_icon.attach_replacement(&mut fresh_replacement);
        assert_eq!(fresh_replacement, SpriteEffectInputs::fresh());
    }

    #[test]
    fn unavailable_observer_discards_candidate_without_changing_presented_state() {
        let presented_icon = IconInteractionInputs::from_icon_constructor();
        let presented_drawable = SpriteEffectInputs::fresh();
        let mut candidate_icon = presented_icon;
        let mut candidate_drawable = presented_drawable;
        candidate_icon.write_input_state(ButtonVisualState::Over);
        let result =
            candidate_icon.finish_input_update(&mut candidate_drawable, |_, icon, drawable| {
                icon.disable(drawable);
                Err("observer continuation unavailable")
            });
        assert_eq!(result, Err("observer continuation unavailable"));
        assert_eq!(presented_icon.effect(), NativeButtonEffect::Unassigned);
        assert_eq!(presented_drawable, SpriteEffectInputs::fresh());
    }
}
