use super::{check_sha256, decimal_u64, RenderOutputRef, UsedContentKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod optional_decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        let Some(text) = Option::<String>::deserialize(deserializer)? else {
            return Ok(None);
        };
        if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
            return Err(serde::de::Error::custom(
                "optional u64 text is not canonical decimal",
            ));
        }
        text.parse::<u64>()
            .map(Some)
            .map_err(serde::de::Error::custom)
    }
}

pub const O05_BUILDING_SCHEMA: &str = "hoi4.production-o05-building-semantics.v1";
pub const O05_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const O05_RUNTIME_PRODUCER_MANIFEST_SHA256: &str =
    "41d9d7ba2ed076958b57fc6b243731039dc066ad48ec569bb834bb73b21aee1f";
pub const O05_RUNTIME_CONSUMER_MANIFEST_SHA256: &str =
    "8e8f0e12082f11b543645ef7ef5ff4940dfb573fbfbfb34f564646a121e74946";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O05BuildingFrame {
    pub schema: String,
    pub executable_sha256: String,
    pub runtime_producer_manifest_sha256: String,
    pub runtime_consumer_manifest_sha256: String,
    pub map_buildings_source: UsedContentKey,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    pub complete_selected_present_scope: bool,
    pub records: Vec<O05BuildingSemantic>,
    pub ordered_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum O05BuildingObjectKind {
    ProvinceBuilding,
    SpecialMapBuilding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O05BuildingSemantic {
    #[serde(with = "decimal_u64")]
    pub event_sequence: u64,
    #[serde(with = "decimal_u64")]
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
    pub render_disposition: O05BuildingRenderDisposition,
    pub position_xyz_f32_bits: [u32; 3],
    pub yaw_f32_bits: u32,
    pub presentation_scale_f32_bits: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_decimal_u64"
    )]
    pub selected_entity_instance_id: Option<u64>,
    pub entity_asset: Option<UsedContentKey>,
    pub mesh: Option<UsedContentKey>,
    pub world_transform_4x4_f32_bits: Option<[u32; 16]>,
    pub referenced_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum O05BuildingRenderDisposition {
    NoEntitySelected,
    SelectedNotSubmitted,
    Submitted,
}

impl O05BuildingFrame {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != O05_BUILDING_SCHEMA {
            return Err("unsupported O05 building semantic schema".into());
        }
        for value in [
            &self.executable_sha256,
            &self.runtime_producer_manifest_sha256,
            &self.runtime_consumer_manifest_sha256,
        ] {
            check_sha256(value, None)?;
        }
        if self.executable_sha256 != O05_A729_EXECUTABLE_SHA256
            || self.runtime_producer_manifest_sha256 != O05_RUNTIME_PRODUCER_MANIFEST_SHA256
            || self.runtime_consumer_manifest_sha256 != O05_RUNTIME_CONSUMER_MANIFEST_SHA256
        {
            return Err("O05 building carrier static authority mismatch".into());
        }
        self.map_buildings_source.validate()?;
        if self.map_buildings_source.normalized_game_relative_path != "map/buildings.txt" {
            return Err("O05 building carrier has the wrong placement source".into());
        }
        if self.first_sequence > self.last_sequence
            || !self.complete_selected_present_scope
            || self.records.is_empty()
        {
            return Err("O05 building carrier is partial, empty, or sequence-reversed".into());
        }

        let mut runtime_ids = BTreeSet::new();
        let mut entity_ids = BTreeSet::new();
        let mut unique_outputs = BTreeSet::new();
        let mut flattened = Vec::new();
        for (ordinal, row) in self.records.iter().enumerate() {
            if row.event_sequence < self.first_sequence
                || row.event_sequence > self.last_sequence
                || row.runtime_object_id == 0
                || row.update_ordinal as usize != ordinal
                || !runtime_ids.insert(row.runtime_object_id)
            {
                return Err("O05 building record identity/order is invalid or duplicated".into());
            }
            if row.object_tag_u32 != 0x165
                || (row.building_kind_index_i32 == 0
                    && row.object_kind != O05BuildingObjectKind::ProvinceBuilding)
                || (row.building_kind_index_i32 != 0
                    && row.object_kind != O05BuildingObjectKind::SpecialMapBuilding)
            {
                return Err("O05 building record tag/kind differs from the a729 layout".into());
            }
            if row
                .position_xyz_f32_bits
                .iter()
                .chain([&row.yaw_f32_bits, &row.presentation_scale_f32_bits])
                .any(|bits| !f32::from_bits(*bits).is_finite())
                || f32::from_bits(row.presentation_scale_f32_bits) < 0.0
            {
                return Err(
                    "O05 building position/yaw/presentation scale is non-finite or negative".into(),
                );
            }

            let submitted = !row.referenced_outputs.is_empty();
            let selected = row.selected_entity_instance_id.is_some();
            let expected_disposition = match (selected, submitted) {
                (false, false) => O05BuildingRenderDisposition::NoEntitySelected,
                (true, false) => O05BuildingRenderDisposition::SelectedNotSubmitted,
                (true, true) => O05BuildingRenderDisposition::Submitted,
                (false, true) => {
                    return Err("O05 building submitted without a selected entity".into())
                }
            };
            if row.render_disposition != expected_disposition
                || submitted != row.entity_asset.is_some()
                || submitted != row.mesh.is_some()
                || selected != row.world_transform_4x4_f32_bits.is_some()
            {
                return Err("O05 building selection/submit/content fields disagree".into());
            }
            if let Some(matrix) = row.world_transform_4x4_f32_bits {
                if matrix.iter().any(|bits| !f32::from_bits(*bits).is_finite())
                    || matrix[3] != 0.0f32.to_bits()
                    || matrix[7] != 0.0f32.to_bits()
                    || matrix[11] != 0.0f32.to_bits()
                    || matrix[15] != 1.0f32.to_bits()
                {
                    return Err("O05 selected building transform is not finite affine".into());
                }
            }
            if !submitted {
                if let Some(id) = row.selected_entity_instance_id {
                    if id == 0 || !entity_ids.insert(id) {
                        return Err(
                            "O05 selected building entity identity is zero or duplicated".into(),
                        );
                    }
                }
                continue;
            }
            row.entity_asset
                .as_ref()
                .expect("checked present")
                .validate()?;
            row.mesh.as_ref().expect("checked present").validate()?;

            let expected_entity_id = row
                .selected_entity_instance_id
                .expect("submitted O05 entity was checked present");
            for output in &row.referenced_outputs {
                let id = match output {
                    RenderOutputRef::Entity {
                        entity_instance_id, ..
                    } => *entity_instance_id,
                    RenderOutputRef::Particle { .. } => {
                        return Err("O05 building references a particle output".into())
                    }
                };
                if id != expected_entity_id || !unique_outputs.insert(output.clone()) {
                    return Err("O05 building output is duplicated or crosses entities".into());
                }
                flattened.push(output.clone());
            }
            if expected_entity_id == 0 || !entity_ids.insert(expected_entity_id) {
                return Err("O05 building entity instance is shared by two records".into());
            }
        }
        if flattened.len() != self.ordered_outputs.len()
            || flattened.into_iter().collect::<BTreeSet<_>>()
                != self
                    .ordered_outputs
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
        {
            return Err("O05 ordered outputs differ from the per-record exact cover".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OptionalId {
        #[serde(default, with = "super::optional_decimal_u64")]
        value: Option<u64>,
    }

    #[test]
    fn optional_entity_identity_is_portable_canonical_decimal() {
        let encoded = serde_json::to_string(&OptionalId {
            value: Some(u64::MAX),
        })
        .unwrap();
        assert_eq!(encoded, format!(r#"{{"value":"{}"}}"#, u64::MAX));
        assert_eq!(
            serde_json::from_str::<OptionalId>(&encoded).unwrap(),
            OptionalId {
                value: Some(u64::MAX)
            }
        );
        assert!(serde_json::from_str::<OptionalId>(r#"{"value":1}"#).is_err());
        assert!(serde_json::from_str::<OptionalId>(r#"{"value":"01"}"#).is_err());
    }
}
