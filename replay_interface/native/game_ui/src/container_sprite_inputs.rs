//! Retained inputs for the native CSprite animation admission pass.
//!
//! The caller owns the authenticated widget identity, reached draw, native effect,
//! and admitted animation definitions. This module does not infer an interaction
//! from missing input and does not shade or parse animation definitions.

use crate::runtime_graphics::ButtonVisualState;

/// CButtonStandard's native effect values, distinct from the wire enum ordinals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeButtonEffect {
    Up = 1,
    Down = 2,
    Over = 3,
    Disable = 4,
}

impl From<ButtonVisualState> for NativeButtonEffect {
    fn from(value: ButtonVisualState) -> Self {
        match value {
            ButtonVisualState::Up => Self::Up,
            ButtonVisualState::Down => Self::Down,
            ButtonVisualState::Over => Self::Over,
            ButtonVisualState::Disable => Self::Disable,
        }
    }
}

/// The three native CSprite effect setters/getters and the reached draw's
/// dirty-clock branch. This state belongs to the actual constructed sprite,
/// separately from widget hover ownership and from animation trigger epochs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SpriteEffectInputs {
    effect: i32,
    dirty: bool,
    clock: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StagedSpriteEffectInputs {
    pub next: SpriteEffectInputs,
    pub draw_clock: f32,
}

impl SpriteEffectInputs {
    /// Requires a witnessed CSprite constructor, not an unavailable prior row.
    /// Zero is the constructor's unassigned native effect, not native Up.
    pub(crate) fn fresh() -> Self {
        Self {
            effect: 0,
            dirty: false,
            clock: 0.0,
        }
    }

    /// The native setter accepts an i32. A repeated assignment leaves an
    /// already-dirty flag set; it never writes a time or clears the flag.
    pub(crate) fn set_effect(&mut self, effect: i32) {
        if effect != self.effect {
            self.dirty = true;
        }
        self.effect = effect;
    }

    pub(crate) fn write_clock(&mut self, clock: f32) {
        self.clock = clock;
    }

    pub(crate) fn clock(&self) -> f32 {
        self.clock
    }

    /// The clean native draw forwards its caller-supplied clock operand. It
    /// does NOT unconditionally reload the retained clock. The caller must bind
    /// both f32 operands from its actual native-equivalent route.
    pub(crate) fn stage_draw(&self, now: f32, incoming_clock: f32) -> StagedSpriteEffectInputs {
        let mut next = *self;
        let draw_clock = if self.dirty {
            next.write_clock(now);
            next.dirty = false;
            next.clock()
        } else {
            incoming_clock
        };
        StagedSpriteEffectInputs { next, draw_clock }
    }
}

/// Exact already-admitted fields of one authored animation, in source order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AnimationAdmissionRule {
    pub trigger_mask: u32,
    pub show_mask: u32,
    pub looping: bool,
    pub duration: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteInputRefusal {
    AllocationFailed,
    NativeCountRange,
    AnimationCensusChanged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AnimationTrigger {
    started_at: f32,
    previous_effect: u32,
}

impl AnimationTrigger {
    fn fresh() -> Self {
        Self {
            started_at: -1.0,
            previous_effect: 0,
        }
    }

    fn admit(
        &mut self,
        rule: AnimationAdmissionRule,
        effect: NativeButtonEffect,
        sprite_elapsed: f32,
    ) -> Option<f32> {
        let effect = effect as u32;
        let bit = 1_u32 << effect;
        let mut elapsed = sprite_elapsed;
        if rule.trigger_mask != 0 {
            if rule.trigger_mask & bit != 0
                && self.previous_effect != effect
                && !(effect == 3 && self.previous_effect == 2)
            {
                self.started_at = sprite_elapsed;
            }
            self.previous_effect = effect;
            // COMISS zero,start; JA rejects only an ordered negative start.
            if self.started_at < 0.0 {
                return None;
            }
            elapsed -= self.started_at;
        }
        if rule.show_mask != u32::MAX && rule.show_mask & bit == 0 {
            return None;
        }
        // COMISS elapsed,duration; JBE admits equality AND unordered operands.
        // Do not replace this with <=, which rejects the native NaN branch.
        if !rule.looping && elapsed > rule.duration {
            return None;
        }
        Some(elapsed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ReachedAnimation {
    pub source_ordinal: usize,
    pub elapsed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpriteAnimationInputs {
    enabled_at_previous_draw: bool,
    sprite_started_at: f32,
    triggers: Vec<AnimationTrigger>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StagedSpriteAnimationInputs {
    pub next: SpriteAnimationInputs,
    pub reached: [Option<ReachedAnimation>; 2],
}

impl SpriteAnimationInputs {
    /// Call only for an actual newly constructed sprite in the owning staged
    /// widget lifetime. A missing prior state is not a constructor witness.
    pub(crate) fn fresh(animation_count: usize) -> Result<Self, SpriteInputRefusal> {
        if animation_count > i32::MAX as usize {
            return Err(SpriteInputRefusal::NativeCountRange);
        }
        let mut triggers = Vec::new();
        triggers
            .try_reserve_exact(animation_count)
            .map_err(|_| SpriteInputRefusal::AllocationFailed)?;
        triggers.resize(animation_count, AnimationTrigger::fresh());
        Ok(Self {
            enabled_at_previous_draw: false,
            sprite_started_at: 0.0,
            triggers,
        })
    }

    /// Pure candidate for one reached native draw. Hidden or unpresented widgets
    /// must not call this as a substitute for a reached draw. Commit `next` only
    /// with the owning compositor/presentation transaction.
    ///
    /// `now` is the already-derived native f32 GUI clock. It is intentionally
    /// neither clamped nor converted from integer micros here. `enabled` is the
    /// actual sprite animation-enable state, not merely whether rules exist.
    pub(crate) fn stage_draw(
        &self,
        enabled: bool,
        now: f32,
        effect: NativeButtonEffect,
        rules: &[AnimationAdmissionRule],
    ) -> Result<StagedSpriteAnimationInputs, SpriteInputRefusal> {
        if rules.len() != self.triggers.len() {
            return Err(SpriteInputRefusal::AnimationCensusChanged);
        }
        let mut next = Self::fresh(self.triggers.len())?;
        next.triggers.copy_from_slice(&self.triggers);
        next.enabled_at_previous_draw = enabled;
        next.sprite_started_at = self.sprite_started_at;
        let mut reached = [None; 2];
        if enabled {
            if !self.enabled_at_previous_draw {
                next.sprite_started_at = now;
            }
            let elapsed = now - next.sprite_started_at;
            let mut accepted = 0;
            for (ordinal, (trigger, rule)) in next.triggers.iter_mut().zip(rules).enumerate() {
                if let Some(elapsed) = trigger.admit(*rule, effect, elapsed) {
                    reached[accepted] = Some(ReachedAnimation {
                        source_ordinal: ordinal,
                        elapsed,
                    });
                    accepted += 1;
                    if accepted == 2 {
                        break;
                    }
                }
            }
        }
        Ok(StagedSpriteAnimationInputs { next, reached })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(trigger_mask: u32) -> AnimationAdmissionRule {
        AnimationAdmissionRule {
            trigger_mask,
            show_mask: u32::MAX,
            looping: false,
            duration: 10.0,
        }
    }

    #[test]
    fn constructor_effect_zero_is_not_up_and_does_not_stamp_a_clock() {
        let state = SpriteEffectInputs::fresh();
        assert_eq!(state.effect, 0);
        assert!(!state.dirty);
        assert_eq!(state.clock().to_bits(), 0);
        let draw = state.stage_draw(17.0, -3.0);
        assert_eq!(draw.draw_clock, -3.0);
        assert_eq!(draw.next, state);
    }

    #[test]
    fn effect_assignments_latch_dirty_without_eager_timestamp_or_cancel() {
        let mut state = SpriteEffectInputs::fresh();
        state.set_effect(NativeButtonEffect::Up as i32);
        state.set_effect(NativeButtonEffect::Up as i32);
        state.set_effect(0);
        assert!(
            state.dirty,
            "returning to the constructor value does not cancel a pending change"
        );
        assert_eq!(state.clock().to_bits(), 0);
        let draw = state.stage_draw(29.0, 4.0);
        assert_eq!(draw.draw_clock, 29.0);
        assert!(!draw.next.dirty);
        assert_eq!(draw.next.effect, 0);
        assert!(
            state.dirty,
            "an uncommitted draw cannot consume the retained dirty flag"
        );
    }

    #[test]
    fn dirty_draw_stamps_reached_time_and_clean_draw_forwards_caller_time() {
        let mut state = SpriteEffectInputs::fresh();
        state.set_effect(NativeButtonEffect::Over as i32);
        let first = state.stage_draw(10.0, 1.0);
        assert_eq!(first.next.clock(), 10.0);
        let mut state = first.next;
        state.set_effect(NativeButtonEffect::Over as i32);
        let second = state.stage_draw(12.0, 8.0);
        assert_eq!(
            second.draw_clock, 8.0,
            "clean path uses the call operand, not retained 10"
        );
        assert_eq!(second.next.clock(), 10.0);
        state.set_effect(NativeButtonEffect::Down as i32);
        assert_eq!(state.stage_draw(15.0, 8.0).draw_clock, 15.0);
    }

    #[test]
    fn direct_clock_assignment_preserves_bits_and_does_not_clear_dirty() {
        let mut state = SpriteEffectInputs::fresh();
        let payload = f32::from_bits(0x7fc1_2345);
        state.write_clock(payload);
        assert_eq!(state.clock().to_bits(), payload.to_bits());
        assert!(!state.dirty);
        state.set_effect(i32::MIN);
        state.write_clock(-0.0);
        assert_eq!(state.clock().to_bits(), (-0.0_f32).to_bits());
        assert!(state.dirty);
        let draw = state.stage_draw(payload, 0.0);
        assert_eq!(draw.draw_clock.to_bits(), payload.to_bits());
        assert!(!draw.next.dirty);
    }

    #[test]
    fn rejected_stage_keeps_pending_change_for_actual_later_draw() {
        let mut published = SpriteEffectInputs::fresh();
        published.set_effect(NativeButtonEffect::Disable as i32);
        let rejected = published.stage_draw(7.0, 1.0);
        assert_eq!(rejected.draw_clock, 7.0);
        assert_eq!(published.clock().to_bits(), 0);
        let accepted = published.stage_draw(13.0, 1.0);
        assert_eq!(accepted.draw_clock, 13.0);
        published = accepted.next;
        assert_eq!(published.clock(), 13.0);
        assert!(!published.dirty);
    }

    #[test]
    fn effect_values_are_native_values_not_wire_ordinals() {
        for (source, expected) in [
            (ButtonVisualState::Up, 1),
            (ButtonVisualState::Down, 2),
            (ButtonVisualState::Over, 3),
            (ButtonVisualState::Disable, 4),
        ] {
            assert_eq!(NativeButtonEffect::from(source) as u32, expected);
        }
    }

    #[test]
    fn fresh_sprite_epoch_is_first_reached_draw_not_construction_time() {
        let original = SpriteAnimationInputs::fresh(1).unwrap();
        let first = original
            .stage_draw(true, 90.0, NativeButtonEffect::Up, &[rule(0)])
            .unwrap();
        assert_eq!(first.reached[0].unwrap().elapsed.to_bits(), 0);
        let second = first
            .next
            .stage_draw(true, 93.0, NativeButtonEffect::Up, &[rule(0)])
            .unwrap();
        assert_eq!(second.reached[0].unwrap().elapsed, 3.0);
        assert_eq!(original.sprite_started_at.to_bits(), 0);
        assert!(!original.enabled_at_previous_draw);
    }

    #[test]
    fn triggers_start_at_minus_one_and_down_to_over_keeps_epoch() {
        let mut state = AnimationTrigger::fresh();
        let both = rule((1 << 2) | (1 << 3));
        assert_eq!(state.admit(both, NativeButtonEffect::Up, 0.0), None);
        assert_eq!(state.started_at, -1.0);
        assert_eq!(state.previous_effect, 1);
        assert_eq!(state.admit(both, NativeButtonEffect::Down, 4.0), Some(0.0));
        assert_eq!(state.admit(both, NativeButtonEffect::Over, 7.0), Some(3.0));
        assert_eq!(state.started_at, 4.0);
        assert_eq!(state.admit(both, NativeButtonEffect::Down, 9.0), Some(0.0));
    }

    #[test]
    fn show_rejection_still_updates_trigger_and_effect() {
        let mut state = AnimationTrigger::fresh();
        let hidden = AnimationAdmissionRule {
            show_mask: 1 << 3,
            ..rule(1 << 2)
        };
        assert_eq!(state.admit(hidden, NativeButtonEffect::Down, 2.0), None);
        assert_eq!(state.started_at, 2.0);
        assert_eq!(
            state.admit(hidden, NativeButtonEffect::Over, 3.0),
            Some(1.0)
        );
    }

    #[test]
    fn first_two_admitted_stop_before_later_trigger_mutation() {
        let rules = [
            AnimationAdmissionRule {
                show_mask: 0,
                ..rule(1 << 1)
            },
            rule(1 << 1),
            rule(1 << 1),
            rule(1 << 1),
        ];
        let stage = SpriteAnimationInputs::fresh(4)
            .unwrap()
            .stage_draw(true, 8.0, NativeButtonEffect::Up, &rules)
            .unwrap();
        assert_eq!(
            stage.reached.map(|entry| entry.unwrap().source_ordinal),
            [1, 2]
        );
        assert_eq!(stage.next.triggers[0].started_at, 0.0);
        assert_eq!(stage.next.triggers[3], AnimationTrigger::fresh());
    }

    #[test]
    fn duration_gate_preserves_equal_unordered_and_signed_zero_branches() {
        let mut state = AnimationTrigger::fresh();
        let bounded = rule(0);
        assert_eq!(
            state.admit(bounded, NativeButtonEffect::Up, 10.0),
            Some(10.0)
        );
        assert_eq!(state.admit(bounded, NativeButtonEffect::Up, 11.0), None);
        assert!(state
            .admit(bounded, NativeButtonEffect::Up, f32::NAN)
            .unwrap()
            .is_nan());
        assert_eq!(
            state.admit(
                AnimationAdmissionRule {
                    duration: f32::NAN,
                    ..bounded
                },
                NativeButtonEffect::Up,
                11.0
            ),
            Some(11.0)
        );
        assert_eq!(
            state
                .admit(bounded, NativeButtonEffect::Up, -0.0)
                .unwrap()
                .to_bits(),
            (-0.0_f32).to_bits()
        );
    }

    #[test]
    fn disabling_draw_records_disabled_without_erasing_trigger_history() {
        let rules = [rule(1 << 1)];
        let first = SpriteAnimationInputs::fresh(1)
            .unwrap()
            .stage_draw(true, 8.0, NativeButtonEffect::Up, &rules)
            .unwrap();
        let disabled = first
            .next
            .stage_draw(false, 9.0, NativeButtonEffect::Down, &rules)
            .unwrap();
        assert_eq!(disabled.reached, [None, None]);
        assert_eq!(disabled.next.triggers, first.next.triggers);
        let restart = disabled
            .next
            .stage_draw(true, 20.0, NativeButtonEffect::Up, &rules)
            .unwrap();
        assert_eq!(restart.next.sprite_started_at, 20.0);
        assert_eq!(restart.reached[0].unwrap().elapsed.to_bits(), 0);
    }

    #[test]
    fn census_mismatch_refuses_without_mutation_and_empty_is_measured() {
        let state = SpriteAnimationInputs::fresh(1).unwrap();
        assert_eq!(
            state.stage_draw(true, 1.0, NativeButtonEffect::Up, &[]),
            Err(SpriteInputRefusal::AnimationCensusChanged)
        );
        assert_eq!(state.triggers, vec![AnimationTrigger::fresh()]);
        let empty = SpriteAnimationInputs::fresh(0)
            .unwrap()
            .stage_draw(false, 1.0, NativeButtonEffect::Up, &[])
            .unwrap();
        assert_eq!(empty.reached, [None, None]);
    }
}
