//! Viewer-created CSprite lifetimes, owned by the container presentation candidate.
//!
//! A missing row is not a constructor witness. The container's construction pass
//! explicitly enrolls authored instances; reached draws can only stage those rows.

use crate::container_sprite_inputs::{
    AnimationAdmissionRule, NativeButtonEffect, ReachedAnimation, SpriteAnimationInputs,
    SpriteEffectInputs, SpriteInputRefusal,
};
use crate::container_sprite_interaction::{
    IconInteractionInputs, IconObserverEvent, IconOperation,
};
use portable_content_manifest::UsedContentKey;
use std::collections::BTreeMap;

/// The current viewer application timer projected through the native GUI
/// update's f64 accumulator/division then CVTSD2SS. This is a viewer operation,
/// not an observation of another process's GUI time or a widget fade clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeGuiClock {
    seconds_bits: u32,
}

impl NativeGuiClock {
    /// The caller supplies integer nanoseconds since its witnessed application
    /// timer construction/resume origin (142340d80 -> 142238d50).
    pub fn from_application_elapsed_nanoseconds(elapsed_ns: i64) -> Self {
        let seconds = (0.0_f64 + elapsed_ns as f64 / 1_000_000_000.0) as f32;
        Self {
            seconds_bits: seconds.to_bits(),
        }
    }

    pub(crate) fn seconds(self) -> f32 {
        f32::from_bits(self.seconds_bits)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContainerSpriteIdentity {
    /// Current typed semantic owner of a materialized template. Display/list
    /// ordinals are not object identity and must not be used here.
    pub owner_key: Vec<String>,
    pub authored_child_path: Vec<u32>,
    pub definition_source: UsedContentKey,
    pub sprite_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RetainedSprite {
    identity: ContainerSpriteIdentity,
    animation_count: usize,
    effect: SpriteEffectInputs,
    widget_factory: ContainerSpriteWidgetFactory,
    icon: Option<IconInteractionInputs>,
    animation: SpriteAnimationInputs,
    geometry: crate::container_sprite_geometry::ContainerSpriteGeometryState,
}

/// The authenticated owning GUI factory, independently of drawable subtype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContainerSpriteWidgetFactory {
    OrdinaryIcon2d,
    Unproven(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContainerSpriteLifetimes {
    rows: BTreeMap<(Vec<String>, Vec<u32>), RetainedSprite>,
}

/// Result of one complete viewer-owned construction operation. New object
/// identities retain that operation's input order, independently of the map
/// used to retain surviving objects. This is not a native draw ordinal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StagedContainerSpriteLifetimes {
    pub next: ContainerSpriteLifetimes,
    pub created: Vec<ContainerSpriteIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContainerSpriteLifetimeRefusal {
    ConstructorIdentityRepeated,
    ConstructorNotWitnessed,
    DefinitionChangedWithoutConstruction,
    WidgetFactoryChangedWithoutConstruction,
    WidgetFactoryUnproven(String),
    IconEffectDiffersFromRetainedOperation,
    IconRequiresOwningOperation,
    ObserverUnavailable(String),
    NativeInputs(SpriteInputRefusal),
}

#[derive(Clone, Debug)]
pub(crate) struct ContainerSpriteDrawCandidate {
    pub next: ContainerSpriteLifetimes,
    pub draw_clock: f32,
    pub animations: [Option<ReachedAnimation>; 2],
    pub geometry: crate::container_sprite_geometry::ContainerSpriteGeometryState,
}

impl ContainerSpriteLifetimes {
    /// Called by an actual new-container/new-widget construction operation.
    /// The complete input census is supplied by authenticated materialization;
    /// a draw is never allowed to insert a missing constructor implicitly.
    pub(crate) fn construct(
        instances: impl IntoIterator<
            Item = (ContainerSpriteIdentity, usize, ContainerSpriteWidgetFactory),
        >,
    ) -> Result<Self, ContainerSpriteLifetimeRefusal> {
        let mut rows = BTreeMap::new();
        for (identity, count, widget_factory) in instances {
            let key = (
                identity.owner_key.clone(),
                identity.authored_child_path.clone(),
            );
            if rows.contains_key(&key) {
                return Err(ContainerSpriteLifetimeRefusal::ConstructorIdentityRepeated);
            }
            let animation = SpriteAnimationInputs::fresh(count)
                .map_err(ContainerSpriteLifetimeRefusal::NativeInputs)?;
            rows.insert(
                key,
                RetainedSprite {
                    identity,
                    animation_count: count,
                    effect: SpriteEffectInputs::fresh(),
                    icon: (widget_factory == ContainerSpriteWidgetFactory::OrdinaryIcon2d)
                        .then(IconInteractionInputs::from_icon_constructor),
                    widget_factory,
                    animation,
                    geometry: crate::container_sprite_geometry::ContainerSpriteGeometryState::from_csprite_constructor(),
                },
            );
        }
        Ok(Self { rows })
    }

    /// Apply the viewer's complete authenticated widget materialization
    /// operation. Retained semantic owners keep their object state when their
    /// displayed ordinal changes. New rows have an explicit construction here;
    /// absence from a draw pass never manufactures or deletes a lifetime.
    /// The caller must not invoke this with an unavailable/filtered census.
    pub(crate) fn stage_materialization(
        &self,
        instances: impl IntoIterator<
            Item = (ContainerSpriteIdentity, usize, ContainerSpriteWidgetFactory),
        >,
    ) -> Result<StagedContainerSpriteLifetimes, ContainerSpriteLifetimeRefusal> {
        let instances = instances.into_iter().collect::<Vec<_>>();
        // Validate the entire census before reporting any new construction.
        // A later duplicate or changed identity cannot publish a prefix.
        let mut candidate = Self::construct(instances.iter().cloned())?;
        for (key, row) in &mut candidate.rows {
            if let Some(retained) = self.rows.get(key) {
                if retained.widget_factory != row.widget_factory {
                    return Err(
                        ContainerSpriteLifetimeRefusal::WidgetFactoryChangedWithoutConstruction,
                    );
                }
                if retained.identity != row.identity {
                    return Err(
                        ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction,
                    );
                }
                if retained.animation_count != row.animation_count {
                    return Err(ContainerSpriteLifetimeRefusal::NativeInputs(
                        SpriteInputRefusal::AnimationCensusChanged,
                    ));
                }
                // Construction admitted the new definition/count. Identity
                // binds its complete authenticated source, not only its name.
                *row = retained.clone();
            }
        }
        let created = instances.into_iter().filter_map(|(identity, _, _)| {
            let key = (identity.owner_key.clone(), identity.authored_child_path.clone());
            (!self.rows.contains_key(&key)).then_some(identity)
        }).collect();
        Ok(StagedContainerSpriteLifetimes { next: candidate, created })
    }

    pub(crate) fn icon_effect(
        &self,
        identity: &ContainerSpriteIdentity,
    ) -> Result<NativeButtonEffect, ContainerSpriteLifetimeRefusal> {
        let current = self.checked_row(identity)?;
        current.icon.map(|icon| icon.effect()).ok_or_else(|| {
            ContainerSpriteLifetimeRefusal::WidgetFactoryUnproven(format!(
                "{:?}",
                current.widget_factory
            ))
        })
    }

    pub(crate) fn geometry(
        &self,
        identity: &ContainerSpriteIdentity,
    ) -> Result<crate::container_sprite_geometry::ContainerSpriteGeometryState, ContainerSpriteLifetimeRefusal> {
        Ok(self.checked_row(identity)?.geometry)
    }

    fn checked_row(
        &self,
        identity: &ContainerSpriteIdentity,
    ) -> Result<&RetainedSprite, ContainerSpriteLifetimeRefusal> {
        let key = (
            identity.owner_key.clone(),
            identity.authored_child_path.clone(),
        );
        let row = self
            .rows
            .get(&key)
            .ok_or(ContainerSpriteLifetimeRefusal::ConstructorNotWitnessed)?;
        if row.identity != *identity {
            return Err(ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction);
        }
        Ok(row)
    }

    /// Widget and drawable operations are staged together. Observer failure
    /// discards every write in this candidate, including intermediate setters.
    pub(crate) fn stage_icon_operation(
        &self,
        identity: &ContainerSpriteIdentity,
        operation: IconOperation,
        dispatch: impl FnMut(
            IconObserverEvent,
            &mut IconInteractionInputs,
            &mut SpriteEffectInputs,
        ) -> Result<(), String>,
    ) -> Result<Self, ContainerSpriteLifetimeRefusal> {
        self.icon_effect(identity)?;
        let key = (
            identity.owner_key.clone(),
            identity.authored_child_path.clone(),
        );
        let mut next = self.clone();
        let row = next.rows.get_mut(&key).expect("validated above");
        row.icon
            .as_mut()
            .expect("validated icon owner")
            .apply(&mut row.effect, operation, dispatch)
            .map_err(ContainerSpriteLifetimeRefusal::ObserverUnavailable)?;
        // Observer callbacks borrow both operands. Refuse a callback that
        // changes only the primitive's final effect and leaves owner metadata
        // inconsistent; this check never changes the candidate's dirty latch.
        let mut checked_effect = row.effect;
        checked_effect.set_effect(row.icon.expect("validated icon owner").effect() as i32);
        if checked_effect != row.effect {
            return Err(ContainerSpriteLifetimeRefusal::IconEffectDiffersFromRetainedOperation);
        }
        Ok(next)
    }

    /// The owning draw-list route has already established reachability. Its
    /// +88 getter supplies the clean fifth operand; a changed effect is stamped
    /// by the reached draw, not by hover input or an unsuccessful presentation.
    pub(crate) fn stage_effect_assignments(
        &self,
        identity: &ContainerSpriteIdentity,
        assignments: &[i32],
    ) -> Result<Self, ContainerSpriteLifetimeRefusal> {
        let key = (
            identity.owner_key.clone(),
            identity.authored_child_path.clone(),
        );
        let current = self
            .rows
            .get(&key)
            .ok_or(ContainerSpriteLifetimeRefusal::ConstructorNotWitnessed)?;
        if current.identity != *identity {
            return Err(ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction);
        }
        if current.icon.is_some() {
            return Err(ContainerSpriteLifetimeRefusal::IconRequiresOwningOperation);
        }
        let mut next = self.clone();
        let row = next.rows.get_mut(&key).expect("validated above");
        // A forced update may temporarily disable and then restore a sprite.
        // Native SetEffect keeps the dirty latch from either changed write;
        // comparing only the final effect loses the reached draw's clock reset.
        for effect in assignments {
            row.effect.set_effect(*effect);
        }
        Ok(next)
    }

    /// Draw staging follows all already-reached widget setter operations.
    pub(crate) fn stage_reached_draw(
        &self,
        identity: &ContainerSpriteIdentity,
        gui_now: f32,
        effect: NativeButtonEffect,
        animations_enabled: bool,
        rules: &[AnimationAdmissionRule],
    ) -> Result<ContainerSpriteDrawCandidate, ContainerSpriteLifetimeRefusal> {
        let key = (
            identity.owner_key.clone(),
            identity.authored_child_path.clone(),
        );
        let current = self
            .rows
            .get(&key)
            .ok_or(ContainerSpriteLifetimeRefusal::ConstructorNotWitnessed)?;
        if current.identity != *identity {
            return Err(ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction);
        }
        let mut effect_inputs = current.effect;
        if let Some(icon) = current.icon {
            if effect != icon.effect() {
                return Err(ContainerSpriteLifetimeRefusal::IconEffectDiffersFromRetainedOperation);
            }
        } else {
            effect_inputs.set_effect(effect as i32);
        }
        let staged_effect = effect_inputs.stage_generic_draw(gui_now);
        let staged_animation = current
            .animation
            .stage_draw(animations_enabled, gui_now, effect, rules)
            .map_err(ContainerSpriteLifetimeRefusal::NativeInputs)?;
        let mut next = self.clone();
        let row = next.rows.get_mut(&key).expect("validated above");
        row.effect = staged_effect.next;
        row.animation = staged_animation.next;
        Ok(ContainerSpriteDrawCandidate {
            next,
            draw_clock: staged_effect.draw_clock,
            animations: staged_animation.reached,
            geometry: current.geometry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn identity(path: &[u32]) -> ContainerSpriteIdentity {
        ContainerSpriteIdentity {
            owner_key: vec!["conference".into(), "participant-a".into()],
            authored_child_path: path.into(),
            sprite_name: "GFX_test".into(),
            definition_source: UsedContentKey {
                effective_vfs_manifest_sha256: "a".repeat(64),
                stored_game_relative_path: "interface/test.gfx".into(),
                normalized_game_relative_path: "interface/test.gfx".into(),
                winning_layer_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: "b".repeat(64),
                payload_sha256: "b".repeat(64),
                payload_bytes: 1,
            },
        }
    }

    #[test]
    fn missing_constructor_and_conflicting_census_never_create_a_row() {
        let id = identity(&[0, 2]);
        assert!(matches!(
            ContainerSpriteLifetimes::default().stage_reached_draw(
                &id,
                1.0,
                NativeButtonEffect::Disable,
                false,
                &[],
            ),
            Err(ContainerSpriteLifetimeRefusal::ConstructorNotWitnessed)
        ));
        assert_eq!(
            ContainerSpriteLifetimes::construct([
                (
                    id.clone(),
                    0,
                    ContainerSpriteWidgetFactory::Unproven("primitive test".into())
                ),
                (
                    id,
                    0,
                    ContainerSpriteWidgetFactory::Unproven("primitive test".into())
                )
            ]),
            Err(ContainerSpriteLifetimeRefusal::ConstructorIdentityRepeated)
        );
    }

    #[test]
    fn changed_definition_refuses_without_erasing_the_presented_lifetime() {
        let id = identity(&[0, 2]);
        let state = ContainerSpriteLifetimes::construct([(
            id.clone(),
            0,
            ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
        )])
        .unwrap();
        let mut changed = id.clone();
        changed.definition_source.payload_sha256 = "c".repeat(64);
        assert!(matches!(
            state.stage_reached_draw(&changed, 1.0, NativeButtonEffect::Disable, false, &[],),
            Err(ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction)
        ));
        assert_eq!(
            state
                .stage_reached_draw(&id, 3.0, NativeButtonEffect::Disable, false, &[],)
                .unwrap()
                .draw_clock,
            3.0
        );
    }

    #[test]
    fn rejected_candidate_leaves_both_clocks_and_other_sprite_unchanged() {
        let a = identity(&[0, 2]);
        let b = identity(&[0, 3]);
        let state = ContainerSpriteLifetimes::construct([
            (
                a.clone(),
                0,
                ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
            ),
            (
                b.clone(),
                0,
                ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
            ),
        ])
        .unwrap();
        let rejected = state
            .stage_reached_draw(&a, 7.0, NativeButtonEffect::Disable, false, &[])
            .unwrap();
        assert_ne!(rejected.next, state);
        let accepted = state
            .stage_reached_draw(&a, 9.0, NativeButtonEffect::Disable, false, &[])
            .unwrap();
        assert_eq!(accepted.draw_clock, 9.0);
        let repeated = accepted
            .next
            .stage_reached_draw(&a, 11.0, NativeButtonEffect::Disable, false, &[])
            .unwrap();
        assert_eq!(repeated.draw_clock, 9.0);
        assert_eq!(repeated.next, accepted.next);
        let b_key = (b.owner_key.clone(), b.authored_child_path.clone());
        assert_eq!(repeated.next.rows[&b_key], state.rows[&b_key]);
    }

    #[test]
    fn complete_materialization_preserves_semantic_owner_across_display_reordering() {
        let a = identity(&[2]);
        let mut b = a.clone();
        b.owner_key[1] = "participant-b".into();
        let original = ContainerSpriteLifetimes::construct([
            (
                a.clone(),
                0,
                ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
            ),
            (
                b.clone(),
                0,
                ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
            ),
        ])
        .unwrap();
        let presented = original
            .stage_reached_draw(&a, 7.0, NativeButtonEffect::Disable, false, &[])
            .unwrap()
            .next;
        let reordered = presented
            .stage_materialization([
                (
                    b.clone(),
                    0,
                    ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
                ),
                (
                    a.clone(),
                    0,
                    ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
                ),
            ])
            .unwrap();
        assert!(reordered.created.is_empty());
        assert_eq!(reordered.next, presented);
        let removed = reordered.next
            .stage_materialization([(
                b.clone(),
                0,
                ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
            )])
            .unwrap();
        assert!(removed.created.is_empty());
        assert!(matches!(
            removed.next.stage_reached_draw(&a, 8.0, NativeButtonEffect::Disable, false, &[]),
            Err(ContainerSpriteLifetimeRefusal::ConstructorNotWitnessed)
        ));
        let recreated = removed.next
            .stage_materialization([
                (
                    b,
                    0,
                    ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
                ),
                (
                    a.clone(),
                    0,
                    ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
                ),
            ])
            .unwrap();
        assert_eq!(recreated.created, [a.clone()]);
        assert_eq!(
            recreated
                .next
                .stage_reached_draw(&a, 9.0, NativeButtonEffect::Disable, false, &[])
                .unwrap()
                .draw_clock,
            9.0
        );
        assert_eq!(
            presented
                .stage_reached_draw(&a, 9.0, NativeButtonEffect::Disable, false, &[])
                .unwrap()
                .draw_clock,
            7.0
        );
    }

    #[test]
    fn new_construction_order_survives_the_retention_map_and_rejected_candidate() {
        let a = identity(&[1]);
        let b = identity(&[2]);
        let c = identity(&[3]);
        let instance = |id| (id, 0, ContainerSpriteWidgetFactory::OrdinaryIcon2d);
        let original = ContainerSpriteLifetimes::construct([instance(b.clone())]).unwrap();
        let first = original.stage_materialization([
            instance(c.clone()), instance(b.clone()), instance(a.clone()),
        ]).unwrap();
        // The BTreeMap order would be a,b,c. Creation follows the actual
        // supplied operation and excludes the already retained b.
        assert_eq!(first.created, [c.clone(), a.clone()]);
        let retry = original.stage_materialization([
            instance(a.clone()), instance(b.clone()), instance(c.clone()),
        ]).unwrap();
        assert_eq!(retry.created, [a.clone(), c.clone()]);
        assert_eq!(first.next, retry.next);
        assert_eq!(original.rows.len(), 1);
        let retained = first.next.stage_materialization([
            instance(a), instance(c), instance(b),
        ]).unwrap();
        assert!(retained.created.is_empty());
        assert_eq!(retained.next, first.next);
    }

    #[test]
    fn construction_delta_never_publishes_a_prefix_before_a_census_conflict() {
        let a = identity(&[1]);
        let b = identity(&[2]);
        let instance = |id, count| (id, count, ContainerSpriteWidgetFactory::OrdinaryIcon2d);
        let original = ContainerSpriteLifetimes::construct([instance(a.clone(), 0)]).unwrap();
        assert_eq!(original.stage_materialization([
            instance(b.clone(), 0), instance(b.clone(), 0), instance(a.clone(), 0),
        ]).unwrap_err(), ContainerSpriteLifetimeRefusal::ConstructorIdentityRepeated);
        let mut changed = a.clone();
        changed.sprite_name = "changed-definition".into();
        assert_eq!(original.stage_materialization([
            instance(b.clone(), 0), instance(changed, 0),
        ]).unwrap_err(), ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction);
        assert_eq!(original.stage_materialization([
            instance(b.clone(), 0), instance(a.clone(), 1),
        ]).unwrap_err(), ContainerSpriteLifetimeRefusal::NativeInputs(
            SpriteInputRefusal::AnimationCensusChanged,
        ));
        assert_eq!(original.rows.len(), 1);
        assert_eq!(original.stage_materialization([
            instance(b.clone(), 0), instance(a, 0),
        ]).unwrap().created, [b]);
    }

    #[test]
    fn temporary_effect_assignment_survives_restoration_and_candidate_rollback() {
        let id = identity(&[2]);
        let initial = ContainerSpriteLifetimes::construct([(
            id.clone(),
            0,
            ContainerSpriteWidgetFactory::Unproven("primitive test".into()),
        )])
        .unwrap();
        let presented = initial
            .stage_reached_draw(&id, 3.0, NativeButtonEffect::Up, false, &[])
            .unwrap()
            .next;
        let changed = presented.stage_effect_assignments(&id, &[4, 1]).unwrap();
        let candidate = changed
            .stage_reached_draw(&id, 7.0, NativeButtonEffect::Up, false, &[])
            .unwrap();
        assert_eq!(candidate.draw_clock.to_bits(), 7.0_f32.to_bits());
        // Dropping that candidate leaves the previously presented clock intact.
        assert_eq!(
            presented
                .stage_reached_draw(&id, 8.0, NativeButtonEffect::Up, false, &[],)
                .unwrap()
                .draw_clock
                .to_bits(),
            3.0_f32.to_bits()
        );
        let repeated = candidate
            .next
            .stage_effect_assignments(&id, &[1, 1])
            .unwrap();
        assert_eq!(
            repeated
                .stage_reached_draw(&id, 9.0, NativeButtonEffect::Up, false, &[],)
                .unwrap()
                .draw_clock
                .to_bits(),
            7.0_f32.to_bits()
        );
        let mut wrong = id.clone();
        wrong.definition_source.payload_sha256 = "f".repeat(64);
        assert!(matches!(
            presented.stage_effect_assignments(&wrong, &[4]),
            Err(ContainerSpriteLifetimeRefusal::DefinitionChangedWithoutConstruction)
        ));
        assert!(matches!(
            ContainerSpriteLifetimes::default().stage_effect_assignments(&id, &[4]),
            Err(ContainerSpriteLifetimeRefusal::ConstructorNotWitnessed)
        ));
    }
}
