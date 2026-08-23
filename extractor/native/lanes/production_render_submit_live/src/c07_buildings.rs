use crate::PromotionError;
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    O05BuildingFrame, O05BuildingObjectKind, O05BuildingRenderDisposition, O05BuildingSemantic,
    RenderOutputRef, RenderSubmit, O05_A729_EXECUTABLE_SHA256, O05_BUILDING_SCHEMA,
    O05_RUNTIME_CONSUMER_MANIFEST_SHA256, O05_RUNTIME_PRODUCER_MANIFEST_SHA256,
};
use std::collections::BTreeSet;

pub const A729_O05_RUNTIME_RECORD_BYTES: usize = 0x48;
pub const A729_O05_RUNTIME_RECORD_TAG: u32 = 0x165;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawO05BuildingFrame {
    pub map_buildings_source: UsedContentKey,
    pub update_index: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub complete_selected_present_scope: bool,
    pub records: Vec<RawO05BuildingRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawO05BuildingRecord {
    pub event_sequence: u64,
    pub runtime_object_id: u64,
    pub update_ordinal: u32,
    pub object_kind: O05BuildingObjectKind,
    pub object_tag_u32: u32,
    pub building_type_id: u32,
    pub building_kind_index_i32: i32,
    pub state_handle_u16: u16,
    pub province_handle_primary_i32: i32,
    pub province_handle_secondary_i32: i32,
    pub building_order_u8: u8,
    pub selected_entity_ordinal_u8: u8,
    pub destroyed_variant: bool,
    pub entity_visual_state_u8: u8,
    pub type_visibility_flag_u8: u8,
    pub position_xyz_f32_bits: [u32; 3],
    pub yaw_f32_bits: u32,
    pub presentation_scale_f32_bits: u32,
    pub selected_entity_instance_id: Option<u64>,
    pub world_transform_4x4_f32_bits: Option<[u32; 16]>,
}

impl RawO05BuildingRecord {
    pub fn decode_a729_post_update(
        event_sequence: u64,
        runtime_object_id: u64,
        update_ordinal: u32,
        record: &[u8; A729_O05_RUNTIME_RECORD_BYTES],
        selected_entity_instance_id: Option<u64>,
        world_transform_4x4_f32_bits: Option<[u32; 16]>,
    ) -> Result<Self, PromotionError> {
        let building_type_id = le_u32(record, 0x00);
        let object_tag_u32 = le_u32(record, 0x04);
        let selected_definition_ptr = le_u64(record, 0x08);
        let selected_entity_ptr = le_u64(record, 0x10);
        let building_kind_index_i32 = le_i32(record, 0x18);
        if runtime_object_id == 0
            || object_tag_u32 != A729_O05_RUNTIME_RECORD_TAG
            || (selected_entity_ptr == 0)
                != (selected_entity_instance_id.is_none() && world_transform_4x4_f32_bits.is_none())
            || (selected_entity_ptr != 0
                && (selected_definition_ptr == 0
                    || selected_entity_instance_id == Some(0)
                    || selected_entity_instance_id.is_none()
                    || world_transform_4x4_f32_bits.is_none()))
        {
            return Err(invalid(
                "O05 post-update record tag/pointer/stable-entity join is incomplete",
            ));
        }
        let presentation_scale_f32_bits = le_u32(record, 0x40);
        let finite = [
            le_u32(record, 0x20),
            le_u32(record, 0x24),
            le_u32(record, 0x28),
            le_u32(record, 0x2c),
            presentation_scale_f32_bits,
        ]
        .into_iter()
        .all(|bits| f32::from_bits(bits).is_finite());
        if !finite || f32::from_bits(presentation_scale_f32_bits) < 0.0 {
            return Err(invalid(
                "O05 post-update record has non-finite or negative transform scalars",
            ));
        }
        Ok(Self {
            event_sequence,
            runtime_object_id,
            update_ordinal,
            object_kind: if building_kind_index_i32 == 0 {
                O05BuildingObjectKind::ProvinceBuilding
            } else {
                O05BuildingObjectKind::SpecialMapBuilding
            },
            object_tag_u32,
            building_type_id,
            building_kind_index_i32,
            state_handle_u16: le_u16(record, 0x38),
            province_handle_primary_i32: le_i32(record, 0x30),
            province_handle_secondary_i32: le_i32(record, 0x34),
            building_order_u8: record[0x1c],
            selected_entity_ordinal_u8: record[0x1d],
            destroyed_variant: record[0x3a] != 0,
            entity_visual_state_u8: record[0x3b],
            type_visibility_flag_u8: record[0x3d],
            position_xyz_f32_bits: [
                le_u32(record, 0x20),
                le_u32(record, 0x24),
                le_u32(record, 0x28),
            ],
            yaw_f32_bits: le_u32(record, 0x2c),
            presentation_scale_f32_bits,
            selected_entity_instance_id,
            world_transform_4x4_f32_bits,
        })
    }
}

pub fn promote_o05_building_frame(
    raw: &RawO05BuildingFrame,
    display_submit: &RenderSubmit,
) -> Result<O05BuildingFrame, PromotionError> {
    raw.map_buildings_source
        .validate()
        .map_err(|error| invalid(format!("invalid O05 map/buildings source: {error}")))?;
    if raw.map_buildings_source.normalized_game_relative_path != "map/buildings.txt"
        || !raw.complete_selected_present_scope
        || raw.records.is_empty()
        || raw.first_sequence > raw.last_sequence
    {
        return Err(invalid(
            "O05 observation is partial, empty, reversed, or uses the wrong source",
        ));
    }

    let ordered_draws = display_submit
        .derive_ordered_draws()
        .map_err(PromotionError::Invalid)?;
    let mut records = Vec::with_capacity(raw.records.len());
    let mut selected_outputs = BTreeSet::new();
    for row in &raw.records {
        let outputs = match row.selected_entity_instance_id {
            None => Vec::new(),
            Some(entity_instance_id) => ordered_draws
                .iter()
                .filter(|output| {
                    matches!(
                        output,
                        RenderOutputRef::Entity {
                            entity_instance_id: candidate,
                            ..
                        } if *candidate == entity_instance_id
                    )
                })
                .cloned()
                .collect::<Vec<_>>(),
        };
        let (entity_asset, mesh, transform, render_disposition) = if outputs.is_empty() {
            match (
                row.selected_entity_instance_id,
                row.world_transform_4x4_f32_bits,
            ) {
                (None, None) | (Some(_), Some(_)) => {}
                _ => {
                    return Err(invalid(
                        "O05 selected entity and building-setter transform disagree",
                    ))
                }
            }
            (
                None,
                None,
                row.world_transform_4x4_f32_bits,
                if row.selected_entity_instance_id.is_some() {
                    O05BuildingRenderDisposition::SelectedNotSubmitted
                } else {
                    O05BuildingRenderDisposition::NoEntitySelected
                },
            )
        } else {
            let expected_transform = row.world_transform_4x4_f32_bits.ok_or_else(|| {
                invalid("O05 submitted record lacks the producer transform observation")
            })?;
            let mut first = None;
            for output in &outputs {
                let RenderOutputRef::Entity {
                    pass_ordinal,
                    entity_instance_id,
                    draw_order,
                } = output
                else {
                    unreachable!("entity-filtered O05 output")
                };
                let entity = display_submit
                    .entities
                    .iter()
                    .find(|entity| {
                        entity.pass_ordinal == *pass_ordinal
                            && entity.entity_instance_id == *entity_instance_id
                            && entity.draw_order == *draw_order
                    })
                    .ok_or_else(|| invalid("O05 output references an absent entity submission"))?;
                if entity.world_transform_4x4_f32_bits != expected_transform {
                    return Err(invalid(
                        "O05 producer transform differs from a submitted entity draw",
                    ));
                }
                let identity = (
                    entity.content_identity.resolved_entity_asset.clone(),
                    entity.content_identity.mesh.clone(),
                );
                if first.as_ref().is_some_and(|existing| existing != &identity) {
                    return Err(invalid(
                        "O05 entity passes disagree on entity-asset or mesh identity",
                    ));
                }
                first = Some(identity);
            }
            let (entity_asset, mesh) = first.expect("nonempty O05 outputs");
            (
                Some(entity_asset),
                Some(mesh),
                Some(expected_transform),
                O05BuildingRenderDisposition::Submitted,
            )
        };
        selected_outputs.extend(outputs.iter().cloned());
        records.push(O05BuildingSemantic {
            event_sequence: row.event_sequence,
            runtime_object_id: row.runtime_object_id,
            update_ordinal: row.update_ordinal,
            object_kind: row.object_kind,
            object_tag_u32: row.object_tag_u32,
            building_type_id: row.building_type_id,
            building_kind_index_i32: row.building_kind_index_i32,
            state_handle_u16: row.state_handle_u16,
            province_handle_primary_i32: row.province_handle_primary_i32,
            province_handle_secondary_i32: row.province_handle_secondary_i32,
            building_order_u8: row.building_order_u8,
            selected_entity_ordinal_u8: row.selected_entity_ordinal_u8,
            destroyed_variant: row.destroyed_variant,
            entity_visual_state_u8: row.entity_visual_state_u8,
            type_visibility_flag_u8: row.type_visibility_flag_u8,
            render_disposition,
            position_xyz_f32_bits: row.position_xyz_f32_bits,
            yaw_f32_bits: row.yaw_f32_bits,
            presentation_scale_f32_bits: row.presentation_scale_f32_bits,
            selected_entity_instance_id: row.selected_entity_instance_id,
            entity_asset,
            mesh,
            world_transform_4x4_f32_bits: transform,
            referenced_outputs: outputs,
        });
    }

    let ordered_outputs = ordered_draws
        .iter()
        .filter(|output| selected_outputs.contains(*output))
        .cloned()
        .collect::<Vec<_>>();
    if ordered_outputs.len() != selected_outputs.len() {
        return Err(invalid(
            "O05 selected output cover is absent or duplicated in display order",
        ));
    }

    let frame = O05BuildingFrame {
        schema: O05_BUILDING_SCHEMA.into(),
        executable_sha256: O05_A729_EXECUTABLE_SHA256.into(),
        runtime_producer_manifest_sha256: O05_RUNTIME_PRODUCER_MANIFEST_SHA256.into(),
        runtime_consumer_manifest_sha256: O05_RUNTIME_CONSUMER_MANIFEST_SHA256.into(),
        map_buildings_source: raw.map_buildings_source.clone(),
        update_index: raw.update_index,
        first_sequence: raw.first_sequence,
        last_sequence: raw.last_sequence,
        complete_selected_present_scope: raw.complete_selected_present_scope,
        records,
        ordered_outputs,
    };
    frame
        .validate()
        .map_err(|error| invalid(format!("promoted O05 carrier is invalid: {error}")))?;
    Ok(frame)
}

fn invalid(message: impl Into<String>) -> PromotionError {
    PromotionError::Invalid(message.into())
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounded O05 u16"),
    )
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded O05 u32"),
    )
}

fn le_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded O05 i32"),
    )
}

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("bounded O05 u64"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_record() -> [u8; A729_O05_RUNTIME_RECORD_BYTES] {
        let mut record = [0u8; A729_O05_RUNTIME_RECORD_BYTES];
        record[0x00..0x04].copy_from_slice(&7u32.to_le_bytes());
        record[0x04..0x08].copy_from_slice(&A729_O05_RUNTIME_RECORD_TAG.to_le_bytes());
        record[0x18..0x1c].copy_from_slice(&0i32.to_le_bytes());
        record[0x40..0x44].copy_from_slice(&1.0f32.to_bits().to_le_bytes());
        record
    }

    #[test]
    fn a729_decoder_consumes_the_complete_pointer_free_record_layout() {
        let record = base_record();
        let decoded =
            RawO05BuildingRecord::decode_a729_post_update(9, 11, 0, &record, None, None).unwrap();
        assert_eq!(decoded.event_sequence, 9);
        assert_eq!(decoded.runtime_object_id, 11);
        assert_eq!(decoded.building_type_id, 7);
        assert_eq!(decoded.object_tag_u32, A729_O05_RUNTIME_RECORD_TAG);
        assert_eq!(decoded.object_kind, O05BuildingObjectKind::ProvinceBuilding);
        assert_eq!(decoded.presentation_scale_f32_bits, 1.0f32.to_bits());
        assert_eq!(decoded.selected_entity_instance_id, None);
        assert_eq!(decoded.world_transform_4x4_f32_bits, None);
    }

    #[test]
    fn a729_decoder_rejects_an_unjoined_transient_entity_pointer() {
        let mut record = base_record();
        record[0x08..0x10].copy_from_slice(&0x1111u64.to_le_bytes());
        record[0x10..0x18].copy_from_slice(&0x2222u64.to_le_bytes());
        assert!(
            RawO05BuildingRecord::decode_a729_post_update(9, 11, 0, &record, None, None,).is_err()
        );
    }
}
