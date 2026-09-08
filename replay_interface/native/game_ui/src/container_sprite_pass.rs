//! One native sprite draw pass and its raster-only recompositions.
//! The owning presentation transaction installs `lifetimes` only after the
//! complete renderer report and framebuffer presentation have succeeded.

use crate::container_animation_records::AdmittedContainerAnimations;
use crate::container_sprite_inputs::{NativeButtonEffect, SpriteEffectInputs};
use crate::container_sprite_interaction::{
    IconInteractionInputs, IconObserverEvent, IconOperation,
};
use crate::container_sprite_lifetime::{
    ContainerSpriteDrawCandidate, ContainerSpriteLifetimes, NativeGuiClock,
};
use crate::container_sprite_session::{ContainerSpriteConstructor, ContainerSpriteMaterialization};
use crate::runtime_graphics::ClipRect;
use crate::{GameUiError, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct ReachedContainerSprite {
    pub draw: ContainerSpriteDrawCandidate,
    pub definitions: AdmittedContainerAnimations,
    pub gui_time: f32,
    pub effect: NativeButtonEffect,
    pub clip: ClipRect,
    pub policy: ContainerSpritePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContainerSpritePolicy {
    pub texture_normalization: Option<crate::container_sprite_geometry::SpriteTextureNormalization>,
    pub native_mxcsr: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct ContainerSpritePass {
    materialization: ContainerSpriteMaterialization,
    pub lifetimes: ContainerSpriteLifetimes,
    pub created: Vec<crate::container_sprite_lifetime::ContainerSpriteIdentity>,
    gui_clock: Option<NativeGuiClock>,
    policy: ContainerSpritePolicy,
    replay_only: bool,
    // A native queue may contain repeated calls for the same object. Preserve
    // every call; a second raster pass replays them instead of staging again.
    receipts: BTreeMap<Vec<u32>, Vec<ReachedContainerSprite>>,
    replay_cursors: BTreeMap<Vec<u32>, usize>,
    call_sequences: BTreeMap<Vec<u32>, Vec<ContainerSpriteCall>>,
    native_clips: BTreeMap<Vec<u32>, std::result::Result<Option<ClipRect>, String>>,
    native_draws: BTreeMap<Vec<u32>, std::result::Result<crate::container_sprite_paint::PreparedContainerSpritePaint, String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerSpriteCall {
    IconOperation(IconOperation),
    Draw(usize),
}

impl ContainerSpritePass {
    pub(crate) fn begin(
        materialization: ContainerSpriteMaterialization,
        presented: &ContainerSpriteLifetimes,
        gui_clock: Option<NativeGuiClock>,
        policy: ContainerSpritePolicy,
    ) -> Result<Self> {
        let staged = materialization
            .stage_lifetimes(presented)
            .map_err(GameUiError::parse)?;
        Ok(Self {
            materialization,
            lifetimes: staged.next,
            created: staged.created,
            gui_clock,
            policy,
            replay_only: false,
            receipts: BTreeMap::new(),
            replay_cursors: BTreeMap::new(),
            call_sequences: BTreeMap::new(),
            native_clips: BTreeMap::new(),
            native_draws: BTreeMap::new(),
        })
    }

    pub(crate) fn begin_raster_replay(&mut self) {
        self.replay_only = true;
        self.replay_cursors.clear();
    }

    pub(crate) fn materialization(&self) -> &ContainerSpriteMaterialization {
        &self.materialization
    }

    pub(crate) fn bind_native_draws(
        &mut self,
        draws: BTreeMap<Vec<u32>, std::result::Result<crate::container_sprite_paint::PreparedContainerSpritePaint, String>>,
    ) -> Result<()> {
        if self.replay_only || !self.call_sequences.is_empty() {
            return Err(GameUiError::parse("native draw inputs must precede reached draws"));
        }
        if draws.len() != self.materialization.elements.len()
            || self.materialization.elements.iter().any(|row| !draws.contains_key(&row.display_child_path))
        {
            return Err(GameUiError::parse("native draw input census differs from materialization"));
        }
        self.native_draws = draws;
        Ok(())
    }

    pub(crate) fn native_draw(
        &self,
        path: &[u32],
    ) -> Result<&crate::container_sprite_paint::PreparedContainerSpritePaint> {
        self.native_draws.get(path)
            .ok_or_else(|| GameUiError::parse("native sprite draw input was not materialized"))?
            .as_ref().map_err(|reason| GameUiError::parse(reason.clone()))
    }

    /// Bind native queue-gate results by this materialization's exact display
    /// path. Measured rejection, missing evidence and eligible clip differ.
    pub(crate) fn bind_native_clips(
        &mut self,
        clips: BTreeMap<Vec<u32>, std::result::Result<Option<ClipRect>, String>>,
    ) -> Result<()> {
        if self.replay_only || !self.receipts.is_empty() {
            return Err(GameUiError::parse(
                "native clips must be bound before reached draws",
            ));
        }
        self.native_clips = clips;
        Ok(())
    }

    /// A witnessed rejected queue gate is Ok(None), without a draw receipt or
    /// clock stamp. Absent and unavailable gates are errors. An explicit clip
    /// remains a caller-proven eligible native draw-list operand.
    pub(crate) fn reached_if_queued(
        &mut self,
        path: &[u32],
        sprite_name: &str,
        effect: Option<NativeButtonEffect>,
        clip: Option<ClipRect>,
    ) -> Result<Option<ReachedContainerSprite>> {
        self.materialization
            .elements
            .iter()
            .find(|row| row.display_child_path == path)
            .and_then(|row| row.identity.as_ref())
            .filter(|identity| identity.sprite_name == sprite_name)
            .ok_or_else(|| {
                GameUiError::parse("native queue gate: materialized sprite identity unavailable")
            })?;
        let clip = match clip {
            Some(clip) => {
                if let Some(bound) = self.native_clips.get(path) {
                    if !matches!(bound, Ok(Some(stored)) if *stored == clip) {
                        return Err(GameUiError::parse(
                            "explicit native clip conflicts with bound queue gate",
                        ));
                    }
                }
                clip
            }
            None => match self.native_clips.get(path) {
                Some(Ok(Some(clip))) => *clip,
                Some(Ok(None)) => return Ok(None),
                Some(Err(reason)) => {
                    return Err(GameUiError::parse(format!(
                        "container sprite {sprite_name} native clip at {path:?}: {reason}"
                    )))
                }
                None => {
                    return Err(GameUiError::parse(format!(
                    "container sprite {sprite_name} native clip at {path:?}: queue gate unavailable"
                )))
                }
            },
        };
        self.reached(path, sprite_name, effect, Some(clip))
            .map(Some)
    }

    /// The static recomposition deliberately omits whole list subtrees. For
    /// every path it does visit, however, all native calls must be replayed.
    /// A partial duplicate sequence cannot authorize the staged lifetime table.
    pub(crate) fn finish_raster_replay(&self) -> Result<()> {
        if !self.replay_only {
            return Err(GameUiError::parse(
                "container raster replay was not started",
            ));
        }
        for (path, consumed) in &self.replay_cursors {
            if self.call_sequences.get(path).map(Vec::len) != Some(*consumed) {
                return Err(GameUiError::parse(format!(
                    "container raster replay omitted a reached call at {path:?}"
                )));
            }
        }
        Ok(())
    }

    /// Call for an actual owning operation, before its reached draw. Fresh
    /// ordinary icons register with no observers; if the viewer attaches a
    /// handler, `dispatch` must reproduce that admitted handler's operations.
    /// Raster replay consumes the operation receipt without invoking it again.
    pub(crate) fn icon_operation(
        &mut self,
        path: &[u32],
        sprite_name: &str,
        operation: IconOperation,
        dispatch: impl FnMut(
            IconObserverEvent,
            &mut IconInteractionInputs,
            &mut SpriteEffectInputs,
        ) -> std::result::Result<(), String>,
    ) -> Result<()> {
        let row = self
            .materialization
            .elements
            .iter()
            .find(|row| row.display_child_path == path)
            .ok_or_else(|| {
                GameUiError::parse("icon operation: materialized object identity unavailable")
            })?;
        if row.constructor != ContainerSpriteConstructor::Generic {
            return Err(GameUiError::parse(
                "icon operation: generic CSprite constructor unavailable",
            ));
        }
        let identity = row
            .identity
            .as_ref()
            .filter(|identity| identity.sprite_name == sprite_name)
            .ok_or_else(|| {
                GameUiError::parse("icon operation: materialized sprite definition differs")
            })?;
        self.lifetimes
            .icon_effect(identity)
            .map_err(|reason| GameUiError::parse(format!("icon operation: {reason:?}")))?;
        if self.replay_only {
            let cursor = *self.replay_cursors.entry(path.to_vec()).or_default();
            if self
                .call_sequences
                .get(path)
                .and_then(|calls| calls.get(cursor))
                != Some(&ContainerSpriteCall::IconOperation(operation))
            {
                return Err(GameUiError::parse(
                    "icon raster replay operation order or operands changed",
                ));
            }
            self.replay_cursors.insert(path.to_vec(), cursor + 1);
            return Ok(());
        }
        let next = self
            .lifetimes
            .stage_icon_operation(identity, operation, dispatch)
            .map_err(|reason| GameUiError::parse(format!("icon operation: {reason:?}")))?;
        self.lifetimes = next;
        self.call_sequences
            .entry(path.to_vec())
            .or_default()
            .push(ContainerSpriteCall::IconOperation(operation));
        Ok(())
    }

    /// The caller must supply the native stored clipping result, not an
    /// intersection of this sprite's geometry with the software framebuffer.
    /// Unknown queue eligibility does not stamp clocks or create a receipt.
    pub(crate) fn reached(
        &mut self,
        path: &[u32],
        sprite_name: &str,
        effect: Option<NativeButtonEffect>,
        clip: Option<ClipRect>,
    ) -> Result<ReachedContainerSprite> {
        let fail = |detail: &str| {
            GameUiError::parse(format!(
                "container sprite {sprite_name} unavailable: {detail}"
            ))
        };
        let row = self
            .materialization
            .elements
            .iter()
            .find(|row| row.display_child_path == path)
            .ok_or_else(|| fail("materialized object identity"))?;
        if row.constructor != ContainerSpriteConstructor::Generic {
            return Err(fail("generic CSprite constructor"));
        }
        if let crate::container_sprite_lifetime::ContainerSpriteWidgetFactory::Unproven(reason) =
            &row.widget_factory
        {
            return Err(fail(&format!(
                "owning widget interaction factory: {reason}"
            )));
        }
        let identity = row
            .identity
            .as_ref()
            .ok_or_else(|| fail("definition identity"))?;
        if identity.sprite_name != sprite_name {
            return Err(fail("draw differs from materialized sprite definition"));
        }
        let clip = clip.ok_or_else(|| fail("native stored clip and queue eligibility"))?;
        let definitions = row
            .animations
            .as_ref()
            .map_err(|reason| fail(reason.as_str()))?;
        if self.replay_only {
            let cursor = *self.replay_cursors.entry(path.to_vec()).or_default();
            let Some(ContainerSpriteCall::Draw(draw_index)) = self
                .call_sequences
                .get(path)
                .and_then(|calls| calls.get(cursor))
            else {
                return Err(fail("raster replay native operation/draw order changed"));
            };
            let receipt = self
                .receipts
                .get(path)
                .and_then(|rows| rows.get(*draw_index))
                .ok_or_else(|| fail("raster replay has no reached native call"))?;
            // Each receipt owns the effect at that specific draw, even if a
            // later operation changed the retained object's final effect.
            let effect = match effect {
                Some(effect) => effect,
                None => {
                    self.lifetimes.icon_effect(identity).map_err(|reason| {
                        fail(&format!("owning widget interaction: {reason:?}"))
                    })?;
                    receipt.effect
                }
            };
            if receipt.effect != effect
                || receipt.clip != clip
                || receipt.definitions != *definitions
            {
                return Err(fail("raster replay operands changed"));
            }
            self.replay_cursors.insert(path.to_vec(), cursor + 1);
            return Ok(receipt.clone());
        }
        let effect = match effect {
            Some(effect) => effect,
            None => self
                .lifetimes
                .icon_effect(identity)
                .map_err(|reason| fail(&format!("owning widget interaction: {reason:?}")))?,
        };
        let gui_time = self
            .gui_clock
            .ok_or_else(|| fail("application GUI clock"))?
            .seconds();
        let rules = definitions
            .records
            .iter()
            .map(|record| record.rule())
            .collect::<Vec<_>>();
        let draw = self
            .lifetimes
            .stage_reached_draw(
                identity,
                gui_time,
                effect,
                !definitions.records.is_empty(),
                &rules,
            )
            .map_err(|reason| fail(&format!("native retained inputs: {reason:?}")))?;
        self.lifetimes = draw.next.clone();
        let receipt = ReachedContainerSprite {
            draw,
            definitions: definitions.clone(),
            gui_time,
            effect,
            clip,
            policy: self.policy,
        };
        let receipts = self.receipts.entry(path.to_vec()).or_default();
        self.call_sequences
            .entry(path.to_vec())
            .or_default()
            .push(ContainerSpriteCall::Draw(receipts.len()));
        receipts.push(receipt.clone());
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_sprite_lifetime::ContainerSpriteIdentity;
    use crate::container_sprite_session::MaterializedContainerSprite;
    use portable_content_manifest::{ContentLayerKind, UsedContentKey};

    fn census() -> ContainerSpriteMaterialization {
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: "interface/test.gfx".into(),
            stored_game_relative_path: "interface/test.gfx".into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: "b".repeat(64),
            payload_bytes: 1,
        };
        ContainerSpriteMaterialization {
            refusals: Vec::new(),
            elements: vec![MaterializedContainerSprite {
                element_name: "button".into(),
                display_child_path: vec![1],
                constructor: ContainerSpriteConstructor::Generic,
                widget_factory:
                    crate::container_sprite_lifetime::ContainerSpriteWidgetFactory::OrdinaryIcon2d,
                identity: Some(ContainerSpriteIdentity {
                    owner_key: vec!["conference".into(), "42".into()],
                    authored_child_path: vec![1],
                    sprite_name: "GFX_test".into(),
                    definition_source: content.clone(),
                }),
                animations: Ok(AdmittedContainerAnimations {
                    source_identity: content,
                    records: Vec::new(),
                }),
                definition_payload: None,
            }],
        }
    }

    fn clip() -> ClipRect {
        ClipRect {
            left: 0,
            top: 0,
            right: 16,
            bottom: 16,
        }
    }
    fn pass() -> ContainerSpritePass {
        let mut pass = ContainerSpritePass::begin(
            census(),
            &ContainerSpriteLifetimes::default(),
            Some(NativeGuiClock::from_application_elapsed_nanoseconds(
                3_000_000_000,
            )),
            ContainerSpritePolicy {
                texture_normalization: None,
                native_mxcsr: None,
            },
        )
        .unwrap();
        pass.icon_operation(&[1], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
            .unwrap();
        pass
    }

    #[test]
    fn native_draw_binding_preserves_each_materialized_refusal_and_requires_complete_census() {
        let mut pass = ContainerSpritePass::begin(
            census(), &ContainerSpriteLifetimes::default(), None,
            ContainerSpritePolicy { texture_normalization: None, native_mxcsr: None },
        ).unwrap();
        let initial = pass.lifetimes.clone();
        assert!(pass.bind_native_draws(BTreeMap::new()).is_err());
        let wrong_path = BTreeMap::from([(vec![2], Err("wrong row".into()))]);
        assert!(pass.bind_native_draws(wrong_path).is_err());
        pass.bind_native_draws(BTreeMap::from([
            (vec![1], Err("InitializationAuthorityUnavailable".into())),
        ])).unwrap();
        assert!(pass.native_draw(&[1]).unwrap_err().to_string()
            .contains("InitializationAuthorityUnavailable"));
        assert!(pass.native_draw(&[2]).unwrap_err().to_string()
            .contains("was not materialized"));
        assert_eq!(pass.lifetimes, initial);
        assert!(pass.receipts.is_empty() && pass.call_sequences.is_empty());
    }

    #[test]
    fn native_draw_inputs_cannot_change_after_a_reached_widget_operation_or_raster_replay() {
        let mut pass = pass();
        let initial = pass.lifetimes.clone();
        let inputs = || BTreeMap::from([(vec![1], Err("source unavailable".into()))]);
        assert!(pass.bind_native_draws(inputs()).is_err());
        assert_eq!(pass.lifetimes, initial);
        assert!(pass.native_draws.is_empty());
        pass.begin_raster_replay();
        assert!(pass.bind_native_draws(inputs()).is_err());
        assert_eq!(pass.lifetimes, initial);
    }

    #[test]
    fn unknown_queue_clip_or_interaction_preserves_both_lifetimes_and_call_census() {
        let mut pass = pass();
        let before = pass.lifetimes.clone();
        assert!(pass
            .reached(&[1], "GFX_test", Some(NativeButtonEffect::Disable), None)
            .is_err());
        assert!(pass
            .reached(&[1], "GFX_test", Some(NativeButtonEffect::Up), Some(clip()))
            .is_err());
        assert_eq!(pass.lifetimes, before);
        assert!(pass.receipts.is_empty());
        pass.gui_clock = None;
        assert!(pass
            .reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Disable),
                Some(clip())
            )
            .is_err());
        assert_eq!(pass.lifetimes, before);
    }

    #[test]
    fn native_duplicate_calls_are_retained_and_raster_replay_never_advances_again() {
        let mut pass = pass();
        let first = pass
            .reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Disable),
                Some(clip()),
            )
            .unwrap();
        let second = pass
            .reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Disable),
                Some(clip()),
            )
            .unwrap();
        assert_eq!(first.draw.draw_clock.to_bits(), 3.0_f32.to_bits());
        assert_eq!(
            second.draw.draw_clock.to_bits(),
            first.draw.draw_clock.to_bits()
        );
        assert_eq!(pass.receipts[&vec![1]].len(), 2);
        let presented_candidate = pass.lifetimes.clone();
        pass.begin_raster_replay();
        pass.icon_operation(&[1], "GFX_test", IconOperation::Disable, |_, _, _| {
            panic!("replay callback")
        })
        .unwrap();
        pass.gui_clock = Some(NativeGuiClock::from_application_elapsed_nanoseconds(
            19_000_000_000,
        ));
        for _ in 0..2 {
            let replay = pass
                .reached(
                    &[1],
                    "GFX_test",
                    Some(NativeButtonEffect::Disable),
                    Some(clip()),
                )
                .unwrap();
            assert_eq!(replay.draw.draw_clock.to_bits(), 3.0_f32.to_bits());
            assert_eq!(replay.gui_time.to_bits(), 3.0_f32.to_bits());
        }
        assert!(pass
            .reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Disable),
                Some(clip())
            )
            .is_err());
        assert_eq!(pass.lifetimes, presented_candidate);
    }

    #[test]
    fn failed_raster_replay_cannot_change_the_presented_or_staged_object() {
        let presented = ContainerSpriteLifetimes::default();
        let mut pass = ContainerSpritePass::begin(
            census(),
            &presented,
            Some(NativeGuiClock::from_application_elapsed_nanoseconds(
                5_000_000_000,
            )),
            ContainerSpritePolicy {
                texture_normalization: None,
                native_mxcsr: None,
            },
        )
        .unwrap();
        pass.icon_operation(&[1], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
            .unwrap();
        pass.reached(
            &[1],
            "GFX_test",
            Some(NativeButtonEffect::Disable),
            Some(clip()),
        )
        .unwrap();
        let staged = pass.lifetimes.clone();
        pass.begin_raster_replay();
        pass.icon_operation(&[1], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
            .unwrap();
        let mut changed = clip();
        changed.right -= 1;
        assert!(pass
            .reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Disable),
                Some(changed)
            )
            .is_err());
        assert!(pass
            .reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Over),
                Some(clip())
            )
            .is_err());
        assert_eq!(pass.lifetimes, staged);
        assert_eq!(presented, ContainerSpriteLifetimes::default());
    }

    #[test]
    fn replay_may_omit_whole_hidden_paths_but_not_part_of_a_repeated_path() {
        let mut pass = pass();
        assert!(pass.finish_raster_replay().is_err());
        for _ in 0..2 {
            pass.reached(
                &[1],
                "GFX_test",
                Some(NativeButtonEffect::Disable),
                Some(clip()),
            )
            .unwrap();
        }
        let staged = pass.lifetimes.clone();
        pass.begin_raster_replay();
        // The actual static pass hides complete list subtrees.
        assert!(pass.finish_raster_replay().is_ok());
        pass.icon_operation(&[1], "GFX_test", IconOperation::Disable, |_, _, _| Ok(()))
            .unwrap();
        pass.reached(
            &[1],
            "GFX_test",
            Some(NativeButtonEffect::Disable),
            Some(clip()),
        )
        .unwrap();
        assert!(pass.finish_raster_replay().is_err());
        pass.reached(
            &[1],
            "GFX_test",
            Some(NativeButtonEffect::Disable),
            Some(clip()),
        )
        .unwrap();
        assert!(pass.finish_raster_replay().is_ok());
        assert_eq!(pass.lifetimes, staged);
    }

    #[test]
    fn native_clip_gate_distinguishes_rejection_unknown_and_eligible_without_stamping() {
        let mut pass = pass();
        let before = pass.lifetimes.clone();
        assert!(pass
            .reached_if_queued(&[1], "GFX_test", None, None)
            .is_err());
        pass.bind_native_clips(BTreeMap::from([(
            vec![1],
            Err("native parent unavailable".into()),
        )]))
        .unwrap();
        assert!(pass
            .reached_if_queued(&[1], "GFX_test", None, None)
            .is_err());
        pass.bind_native_clips(BTreeMap::from([(vec![1], Ok(None))]))
            .unwrap();
        assert!(pass
            .reached_if_queued(&[1], "GFX_test", None, None)
            .unwrap()
            .is_none());
        assert!(pass
            .reached_if_queued(&[1], "GFX_test", None, Some(clip()))
            .is_err());
        assert!(pass
            .reached_if_queued(&[9], "GFX_test", None, None)
            .is_err());
        assert!(pass
            .reached_if_queued(&[1], "GFX_other", None, None)
            .is_err());
        assert_eq!(pass.lifetimes, before);
        assert!(pass.receipts.is_empty());
        pass.bind_native_clips(BTreeMap::from([(vec![1], Ok(Some(clip())))]))
            .unwrap();
        let reached = pass
            .reached_if_queued(&[1], "GFX_test", None, None)
            .unwrap()
            .unwrap();
        assert_eq!(reached.draw.draw_clock, 3.0);
        assert!(pass.bind_native_clips(BTreeMap::new()).is_err());
        let staged = pass.lifetimes.clone();
        pass.begin_raster_replay();
        pass.icon_operation(&[1], "GFX_test", IconOperation::Disable, |_, _, _| {
            panic!("replay")
        })
        .unwrap();
        assert!(pass.bind_native_clips(BTreeMap::new()).is_err());
        assert!(pass
            .reached_if_queued(&[1], "GFX_test", None, None)
            .unwrap()
            .is_some());
        assert!(pass.finish_raster_replay().is_ok());
        assert_eq!(pass.lifetimes, staged);
    }
}
