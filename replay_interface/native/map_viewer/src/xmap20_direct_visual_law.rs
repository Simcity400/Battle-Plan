//! XMAP20 direct-visual producer derivation.
//!
//! QJ row `carrier.render_submit.direct_visuals` (XMAP20,
//! forbidden_product_authority -> DERIVE_missing): the recorded producer
//! lifecycle rows (strategic-air / naval-combat / trade-convoy scene
//! producers) were wire-only - validated, never consumed by any viewer module
//! - and no production writer ever emitted a single row, so the carrier was
//! retired from the wire (production-render-submit v14). The viewer-side law
//! lives HERE and runs on the presentation path
//! (`replay_app::submitted_draw::execute_submitted_draws` calls it before any
//! draw): every pixel a direct-scene producer presents arrives as an ordinary
//! same-submit particle or entity output, so direct-scene presentation derives
//! entirely from those recorded outputs and their exact ordered cover.
//! Producer bookkeeping (lifecycle ops, rng seed/counter, live phase)
//! contributes no pixels; effect randomness is viewer-local per the
//! 2026-08-22 ruling.

use portable_render_stream::{RenderOutputRef, RenderSubmit};
use crate::Result;
use std::collections::BTreeSet;

/// Identity key of one submitted particle output.
pub type ParticleOutputKey = (u64, u32, u32, u32);
/// Identity key of one submitted entity output.
pub type EntityOutputKey = (u32, u64, u32);

/// The derived direct-scene presentation coverage of one submit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedDirectVisualCoverage {
    pub particle_output_count: usize,
    pub entity_output_count: usize,
    /// Length of the recorded ordered draw list; equals
    /// `particle_output_count + entity_output_count` exactly.
    pub ordered_draw_count: usize,
}

/// Derive direct-scene presentation for one recorded submit. Fails closed
/// before any draw executes when the recorded outputs and their ordered cover
/// disagree.
pub fn derive_direct_visual_coverage(
    submit: &RenderSubmit,
) -> Result<DerivedDirectVisualCoverage> {
    let particle_keys = submit
        .particles
        .iter()
        .map(|packet| {
            (
                packet.effect_instance_id,
                packet.bucket,
                packet.packet_ordinal,
                packet.draw_order,
            )
        })
        .collect::<Vec<_>>();
    let entity_keys = submit
        .entities
        .iter()
        .map(|entity| {
            (
                entity.pass_ordinal,
                entity.entity_instance_id,
                entity.draw_order,
            )
        })
        .collect::<Vec<_>>();
    derive_direct_visual_coverage_from_parts(&particle_keys, &entity_keys, &submit.ordered_draws)
}

/// Pure core of the coverage law over the exact recorded identities: the
/// ordered draw list must reference each submitted output exactly once - no
/// duplicate draw, no missing output, no reference to an output that was
/// never submitted. Any producer-only visual channel would show up here as a
/// missing or foreign reference and fail closed.
pub fn derive_direct_visual_coverage_from_parts(
    particle_keys: &[ParticleOutputKey],
    entity_keys: &[EntityOutputKey],
    ordered_draws: &[RenderOutputRef],
) -> Result<DerivedDirectVisualCoverage> {
    #[derive(PartialOrd, Ord, PartialEq, Eq)]
    enum SeenKey {
        Particle(ParticleOutputKey),
        Entity(EntityOutputKey),
    }
    let mut submitted = BTreeSet::new();
    for key in particle_keys {
        if !submitted.insert(SeenKey::Particle(*key)) {
            return Err("direct-scene presentation submits an identical particle output twice"
                .into());
        }
    }
    for key in entity_keys {
        if !submitted.insert(SeenKey::Entity(*key)) {
            return Err("direct-scene presentation submits an identical entity output twice"
                .into());
        }
    }
    let mut drawn = BTreeSet::new();
    for draw in ordered_draws {
        let key = match draw {
            RenderOutputRef::Particle {
                effect_instance_id,
                bucket,
                packet_ordinal,
                draw_order,
            } => SeenKey::Particle((*effect_instance_id, *bucket, *packet_ordinal, *draw_order)),
            RenderOutputRef::Entity {
                pass_ordinal,
                entity_instance_id,
                draw_order,
            } => SeenKey::Entity((*pass_ordinal, *entity_instance_id, *draw_order)),
        };
        // A reference to an output that was never submitted is foreign; a
        // repeat reference draws one output twice. Both fail closed.
        if !submitted.contains(&key) || !drawn.insert(key) {
            return Err("direct-scene presentation ordered draws are not the exact submitted-output cover".into());
        }
    }
    if drawn.len() != particle_keys.len() + entity_keys.len() {
        return Err(
            "direct-scene presentation ordered draws do not cover every submitted output".into(),
        );
    }
    Ok(DerivedDirectVisualCoverage {
        particle_output_count: particle_keys.len(),
        entity_output_count: entity_keys.len(),
        ordered_draw_count: ordered_draws.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_render_stream::RenderOutputRef;

    // kills: QJ carrier.render_submit.direct_visuals DERIVE_missing - the
    // viewer proves, per submit on the presentation path, that direct-scene
    // presentation is fully carried by the ordinary recorded outputs (exact
    // identity-cover law re-derived at the point of presentation).
    #[test]
    fn derives_exact_identity_coverage_over_recorded_outputs() {
        let particles = vec![(7u64, 0u32, 0u32, 0u32), (8, 0, 0, 1)];
        let entities = vec![(3u32, 9u64, 2u32)];
        let draws = vec![
            particle_draw(8, 1),
            entity_draw(3, 9, 2),
            particle_draw(7, 0),
        ];
        let coverage =
            derive_direct_visual_coverage_from_parts(&particles, &entities, &draws).expect("coverage");
        assert_eq!(coverage.particle_output_count, 2);
        assert_eq!(coverage.entity_output_count, 1);
        assert_eq!(coverage.ordered_draw_count, 3);
        let empty =
            derive_direct_visual_coverage_from_parts(&[], &[], &[]).expect("empty");
        assert_eq!(
            empty,
            DerivedDirectVisualCoverage {
                particle_output_count: 0,
                entity_output_count: 0,
                ordered_draw_count: 0,
            }
        );
    }

    // kills: a producer-only visual channel sneaking past presentation - a
    // duplicated, missing, or FOREIGN reference fails closed instead of
    // presenting. The foreign/mixed cases keep the draw COUNT exact (3 draws
    // for 2+1 outputs) so only identity checking can reject them: a count-only
    // law would admit them and this test would fail to kill that defect.
    #[test]
    fn refuses_duplicate_missing_or_foreign_draws() {
        let particles = vec![(7u64, 0u32, 0u32, 0u32), (8, 0, 0, 1)];
        let entities = vec![(3u32, 9u64, 2u32)];
        let same = particle_draw(7, 0);
        assert!(
            derive_direct_visual_coverage_from_parts(&particles, &entities, &[same.clone(), same])
                .is_err()
        );
        // count matches (3 draws) but two references are foreign identities.
        let foreign = vec![
            particle_draw(99, 0),
            particle_draw(98, 1),
            entity_draw(3, 9, 2),
        ];
        assert!(derive_direct_visual_coverage_from_parts(&particles, &entities, &foreign).is_err());
        // mixed real+foreign with matching total still refuses.
        let mixed = vec![
            particle_draw(7, 0),
            particle_draw(99, 5),
            entity_draw(3, 9, 2),
        ];
        assert!(derive_direct_visual_coverage_from_parts(&particles, &entities, &mixed).is_err());
        // a submitted output never drawn refuses.
        let short = vec![particle_draw(7, 0)];
        assert!(derive_direct_visual_coverage_from_parts(&particles, &entities, &short).is_err());
        assert!(derive_direct_visual_coverage_from_parts(&particles, &entities, &[]).is_err());
        // identical submitted identities refuse outright.
        let dup_particles = vec![(7u64, 0u32, 0u32, 0u32), (7, 0, 0, 0)];
        assert!(derive_direct_visual_coverage_from_parts(&dup_particles, &[], &[]).is_err());
    }

    fn particle_draw(effect_instance_id: u64, draw_order: u32) -> RenderOutputRef {
        RenderOutputRef::Particle {
            effect_instance_id,
            bucket: 0,
            packet_ordinal: 0,
            draw_order,
        }
    }

    fn entity_draw(pass_ordinal: u32, entity_instance_id: u64, draw_order: u32) -> RenderOutputRef {
        RenderOutputRef::Entity {
            pass_ordinal,
            entity_instance_id,
            draw_order,
        }
    }
}
