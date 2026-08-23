//! XMAP20 entity-presentation derivation.
//!
//! QJ row `carrier.render_submit.entity_presentations` (XMAP20,
//! forbidden_product_authority -> DERIVE_missing): the recorded presentation
//! events were wire-only - validated on the wire, never consumed by any viewer
//! module - and no production writer ever emitted a single row, so the carrier
//! was retired from the wire (production-render-submit v14). Presented-entity
//! visual state derives HERE instead, from the authoritative recorded
//! [`EntitySubmission`] state: the content identity fixes WHAT is presented,
//! the selector-native packed state flags fix the semantic state tuple, and
//! animation phase/speed advance viewer-locally per the 2026-08-22
//! effect-randomness ruling.

use portable_render_stream::{EntityNativeSelection, EntitySubmission};
use crate::Result;

/// The derived presented-entity state of one recorded submission. Every field
/// comes from the submission itself; nothing else on the wire contributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedEntityPresentation {
    pub entity_instance_id: u64,
    /// Presented asset identity (payload hash of the resolved entity asset).
    pub resolved_entity_sha256: String,
    pub registration_token: String,
    pub entity_group_index: u32,
    pub mesh_draw_list_index: u32,
    pub entity_variant_u8: u8,
    pub mesh_draw_variant_u8: u8,
    pub pass_selector_i32: i32,
    pub alternate_list_u8: u8,
    pub bind_renderer_target_u8: u8,
    /// The semantic state tuple exactly as the engine packed it.
    pub semantic_state_flags: u32,
}

/// Derive presented-entity visual state for one recorded submission. Fails
/// closed when the recorded carrier is internally inconsistent (zero instance,
/// blank token, hash or packed-state disagreement), never substitutes.
pub fn derive_entity_presentation(entity: &EntitySubmission) -> Result<DerivedEntityPresentation> {
    derive_entity_presentation_from_parts(
        entity.entity_instance_id,
        &entity.resolved_entity_sha256,
        &entity.content_identity.resolved_entity_asset.payload_sha256,
        &entity.content_identity.native_selection,
        entity.state_flags,
    )
}

/// Pure core of the presentation law over the exact recorded inputs.
pub fn derive_entity_presentation_from_parts(
    entity_instance_id: u64,
    resolved_entity_sha256: &str,
    resolved_entity_asset_payload_sha256: &str,
    native_selection: &EntityNativeSelection,
    state_flags: u32,
) -> Result<DerivedEntityPresentation> {
    if entity_instance_id == 0 {
        return Err("presented entity has a zero instance id".into());
    }
    if native_selection.entity_registration_token.trim().is_empty() {
        return Err("presented entity has a blank registration token".into());
    }
    if resolved_entity_sha256 != resolved_entity_asset_payload_sha256 {
        return Err(
            "presented entity hash disagrees with its admitted asset payload hash".into(),
        );
    }
    let packed = native_selection.packed_state_flags();
    if state_flags != packed {
        return Err("presented entity state flags disagree with the packed tuple".into());
    }
    Ok(DerivedEntityPresentation {
        entity_instance_id,
        resolved_entity_sha256: resolved_entity_sha256.to_string(),
        registration_token: native_selection.entity_registration_token.clone(),
        entity_group_index: native_selection.entity_group_index,
        mesh_draw_list_index: native_selection.mesh_draw_list_index,
        entity_variant_u8: native_selection.entity_variant_u8,
        mesh_draw_variant_u8: native_selection.mesh_draw_variant_u8,
        pass_selector_i32: native_selection.pass_selector_i32,
        alternate_list_u8: native_selection.alternate_list_u8,
        bind_renderer_target_u8: native_selection.bind_renderer_target_u8,
        semantic_state_flags: state_flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> EntityNativeSelection {
        EntityNativeSelection {
            entity_registration_token: "infantry_entity".into(),
            entity_group_index: 1,
            mesh_draw_list_index: 2,
            entity_variant_u8: 3,
            mesh_draw_variant_u8: 4,
            pass_selector_i32: -5,
            alternate_list_u8: 6,
            bind_renderer_target_u8: 7,
        }
    }

    // kills: QJ carrier.render_submit.entity_presentations DERIVE_missing -
    // presented entity state now derives from the recorded submission's own
    // authoritative fields (identity + packed selector state).
    #[test]
    fn derives_presented_state_from_recorded_entity_parts() {
        let selection = selection();
        let derived = derive_entity_presentation_from_parts(
            41,
            "ab".repeat(32).as_str(),
            "ab".repeat(32).as_str(),
            &selection,
            selection.packed_state_flags(),
        )
        .expect("derive");
        assert_eq!(derived.entity_instance_id, 41);
        assert_eq!(derived.registration_token, "infantry_entity");
        assert_eq!(derived.semantic_state_flags, selection.packed_state_flags());
        assert_eq!(derived.pass_selector_i32, -5);
        assert_eq!(derived.bind_renderer_target_u8, 7);
    }

    // kills: a derivation that trusted a tampered carrier - asset-hash and
    // packed-state disagreement must fail closed, never present.
    #[test]
    fn refuses_hash_or_state_disagreement_and_blank_identity() {
        let selection = selection();
        let flags = selection.packed_state_flags();
        assert!(derive_entity_presentation_from_parts(
            41,
            "ab".repeat(32).as_str(),
            "cd".repeat(32).as_str(),
            &selection,
            flags
        )
        .is_err());
        assert!(derive_entity_presentation_from_parts(
            41,
            "ab".repeat(32).as_str(),
            "ab".repeat(32).as_str(),
            &selection,
            flags ^ 0x100
        )
        .is_err());
        let mut blank = selection.clone();
        blank.entity_registration_token = "  ".into();
        assert!(derive_entity_presentation_from_parts(
            41,
            "ab".repeat(32).as_str(),
            "ab".repeat(32).as_str(),
            &blank,
            flags
        )
        .is_err());
        assert!(derive_entity_presentation_from_parts(
            0,
            "ab".repeat(32).as_str(),
            "ab".repeat(32).as_str(),
            &selection,
            flags
        )
        .is_err());
    }
}
