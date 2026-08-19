use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const FOUNDATION_RENDER_SCHEMA: &str = "hoi4.viewer-map-foundation.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapVisualFamily {
    CameraView,
    Terrain,
    PoliticalFillGradients,
    FogOfWar,
    ThinBorders,
    WaterIce,
    RiversStraits,
    LightingShadows,
    GradingPostFx,
    Trees,
    CitiesBuildingsMapObjects,
    UnitsCounters,
    CombatMarkers,
    OrdersFrontsArrows,
    RailSupply,
    LabelsMapText,
    MapModes,
    WeatherAtmosphere,
    ParticlesEffects,
    MapFurnitureIcons,
}

impl MapVisualFamily {
    pub const ALL: [Self; 20] = [
        Self::CameraView,
        Self::Terrain,
        Self::PoliticalFillGradients,
        Self::FogOfWar,
        Self::ThinBorders,
        Self::WaterIce,
        Self::RiversStraits,
        Self::LightingShadows,
        Self::GradingPostFx,
        Self::Trees,
        Self::CitiesBuildingsMapObjects,
        Self::UnitsCounters,
        Self::CombatMarkers,
        Self::OrdersFrontsArrows,
        Self::RailSupply,
        Self::LabelsMapText,
        Self::MapModes,
        Self::WeatherAtmosphere,
        Self::ParticlesEffects,
        Self::MapFurnitureIcons,
    ];

    pub const fn owner_packet(self) -> &'static str {
        match self {
            Self::CameraView => "XMAP03",
            Self::Terrain => "XMAP04",
            Self::PoliticalFillGradients | Self::FogOfWar => "XMAP05",
            Self::ThinBorders => "XMAP06",
            Self::WaterIce => "XMAP07",
            Self::RiversStraits => "XMAP08",
            Self::LightingShadows => "XMAP09",
            Self::GradingPostFx => "XMAP10",
            Self::Trees => "XMAP11",
            Self::CitiesBuildingsMapObjects => "XMAP12",
            Self::UnitsCounters => "XMAP13",
            Self::CombatMarkers => "XMAP14",
            Self::OrdersFrontsArrows => "XMAP15",
            Self::RailSupply => "XMAP16",
            Self::LabelsMapText => "XMAP17",
            Self::MapModes => "XMAP18",
            Self::WeatherAtmosphere => "XMAP19",
            Self::ParticlesEffects => "XMAP20",
            Self::MapFurnitureIcons => "XMAP21",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationOperationIdentity {
    pub operation_id: String,
    pub global_order: u32,
    pub program_sha256: String,
    pub ordered_resources_sha256: String,
    pub target_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationGap {
    pub gap_id: String,
    pub first_unavailable_hour: i64,
    pub last_unavailable_hour: i64,
    pub witness_sha256: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum FamilyDisposition {
    Present {
        payload_schema: String,
        producer_registration_sha256: String,
        payload_sha256: String,
        ordered_operations: Vec<FoundationOperationIdentity>,
        ordered_used_content: Vec<UsedContentKey>,
    },
    AuthenticatedEmpty {
        producer_registration_sha256: String,
        enumerator_witness_sha256: String,
    },
    NotApplicable {
        applicability_witness_sha256: String,
        reason: String,
    },
    Unavailable {
        gap: FoundationGap,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyCoverageRow {
    pub family: MapVisualFamily,
    pub owner_packet: String,
    pub disposition: FamilyDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationRenderEnvelope {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub families: Vec<FamilyCoverageRow>,
}

impl FoundationRenderEnvelope {
    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("foundation render envelope JSON: {error}"))?;
        value.validate()?;
        Ok(value)
    }

    pub fn deterministic_sha256(&self) -> Result<String, String> {
        Ok(format!("{:x}", Sha256::digest(self.encode_json()?)))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FOUNDATION_RENDER_SCHEMA {
            return Err(format!(
                "unsupported foundation render schema {:?}",
                self.schema
            ));
        }
        validate_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(&self.effective_vfs_manifest_sha256, "effective VFS")?;
        validate_sha256(
            &self.foundation_content_operation_sha256,
            "foundation content operation",
        )?;
        if self.families.len() != MapVisualFamily::ALL.len() {
            return Err(format!(
                "foundation family coverage is incomplete: expected {}, got {}",
                MapVisualFamily::ALL.len(),
                self.families.len()
            ));
        }
        for (expected, row) in MapVisualFamily::ALL.iter().zip(&self.families) {
            if &row.family != expected || row.owner_packet != expected.owner_packet() {
                return Err(format!(
                    "foundation family identity/order mismatch at {:?}",
                    expected
                ));
            }
            validate_disposition(row)?;
        }
        Ok(())
    }

    pub fn gaps(&self) -> impl Iterator<Item = (&MapVisualFamily, &FoundationGap)> {
        self.families
            .iter()
            .filter_map(|row| match &row.disposition {
                FamilyDisposition::Unavailable { gap } => Some((&row.family, gap)),
                _ => None,
            })
    }

    pub fn is_complete(&self) -> bool {
        self.families
            .iter()
            .all(|row| !matches!(row.disposition, FamilyDisposition::Unavailable { .. }))
    }
}

fn validate_disposition(row: &FamilyCoverageRow) -> Result<(), String> {
    match &row.disposition {
        FamilyDisposition::Present {
            payload_schema,
            producer_registration_sha256,
            payload_sha256,
            ordered_operations,
            ordered_used_content,
        } => {
            if payload_schema.trim().is_empty() {
                return Err(format!("{:?} has a blank payload schema", row.family));
            }
            validate_sha256(producer_registration_sha256, "family producer registration")?;
            validate_sha256(payload_sha256, "family payload")?;
            let mut operation_ids = BTreeSet::new();
            let mut prior_order = None;
            for operation in ordered_operations {
                if operation.operation_id.trim().is_empty()
                    || !operation_ids.insert(operation.operation_id.as_str())
                    || prior_order.is_some_and(|prior| operation.global_order <= prior)
                {
                    return Err(format!(
                        "{:?} operations are blank, duplicated, or not strictly ordered",
                        row.family
                    ));
                }
                validate_sha256(&operation.program_sha256, "operation program")?;
                validate_sha256(
                    &operation.ordered_resources_sha256,
                    "operation resource order",
                )?;
                validate_sha256(&operation.target_sha256, "operation target")?;
                prior_order = Some(operation.global_order);
            }
            let mut used_paths = BTreeSet::new();
            for content in ordered_used_content {
                content.validate()?;
                if !used_paths.insert(content.normalized_game_relative_path.as_str()) {
                    return Err(format!("{:?} repeats a UsedContent path", row.family));
                }
            }
        }
        FamilyDisposition::AuthenticatedEmpty {
            producer_registration_sha256,
            enumerator_witness_sha256,
        } => {
            validate_sha256(
                producer_registration_sha256,
                "empty-family producer registration",
            )?;
            validate_sha256(enumerator_witness_sha256, "empty-family enumerator witness")?;
        }
        FamilyDisposition::NotApplicable {
            applicability_witness_sha256,
            reason,
        } => {
            validate_sha256(
                applicability_witness_sha256,
                "not-applicable family witness",
            )?;
            if reason.trim().is_empty() {
                return Err(format!(
                    "{:?} has a blank not-applicable reason",
                    row.family
                ));
            }
        }
        FamilyDisposition::Unavailable { gap } => {
            if gap.gap_id.trim().is_empty()
                || gap.reason.trim().is_empty()
                || gap.first_unavailable_hour > gap.last_unavailable_hour
            {
                return Err(format!("{:?} has an invalid unavailable gap", row.family));
            }
            validate_sha256(&gap.witness_sha256, "unavailable family witness")?;
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> FoundationRenderEnvelope {
        FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: 42,
            content_manifest_sha256: "b".repeat(64),
            product_modset_sha256: "c".repeat(64),
            effective_vfs_manifest_sha256: "d".repeat(64),
            foundation_content_operation_sha256: "e".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: FamilyDisposition::Unavailable {
                        gap: FoundationGap {
                            gap_id: format!("{}.adapter_unimplemented", family.owner_packet()),
                            first_unavailable_hour: 42,
                            last_unavailable_hour: 42,
                            witness_sha256: "a".repeat(64),
                            reason: "adapter has not promoted".into(),
                        },
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn exhaustive_family_envelope_round_trips_byte_exactly() {
        let expected = envelope();
        let first = expected.encode_json().unwrap();
        let decoded = FoundationRenderEnvelope::decode_json(&first).unwrap();
        assert_eq!(decoded.encode_json().unwrap(), first);
        assert_eq!(decoded, expected);
        assert_eq!(decoded.gaps().count(), MapVisualFamily::ALL.len());
    }

    #[test]
    fn missing_reordered_or_recorder_presentation_state_fails_closed() {
        let mut missing = envelope();
        missing.families.pop();
        assert!(missing.validate().unwrap_err().contains("incomplete"));
        let mut reordered = envelope();
        reordered.families.swap(0, 1);
        assert!(reordered.validate().unwrap_err().contains("identity/order"));
        let mut value = serde_json::to_value(envelope()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("selected_present".into(), serde_json::json!({}));
        assert!(serde_json::from_value::<FoundationRenderEnvelope>(value).is_err());
    }
}
