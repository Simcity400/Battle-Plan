//! Exact O04 city-instance semantic carrier.
//!
//! This carrier joins the captured city generator row to the already-carried
//! entity submissions. It never creates another draw or reconstructs a city
//! from map inputs.

use super::{check_sha256, decimal_u64, RenderOutputRef, UsedContentKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const O04_CITY_SCHEMA: &str = "hoi4.production-o04-city-semantics.v1";
pub const O04_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const O04_EVIDENCE_MANIFEST_SHA256: &str =
    "58e4a942db6aa78bfbf5c429fdd5241bb42a62f74866b78ea52e5747253b38f7";
pub const O04_LIVE_CITY_SUBMIT_BINDING_SHA256: &str =
    "9305872f42bd710e9bf060d6c7582ac87ed1fbe82bd83e26d7966d03c246899e";
pub const O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256: &str =
    "f08877f7eeecc234785de128b5fa8cb5d6eb386421f7fb1a25fd344d540dfedc";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O04CityFrame {
    pub schema: String,
    pub executable_sha256: String,
    pub evidence_manifest_sha256: String,
    pub live_city_submit_binding_sha256: String,
    pub live_city_submit_verification_sha256: String,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    pub cities: Vec<O04CitySemantic>,
    /// Exact display-submit order of every output referenced by `cities`.
    pub ordered_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O04CitySemantic {
    #[serde(with = "decimal_u64")]
    pub event_sequence: u64,
    pub generator_ordinal: u32,
    pub candidate_xy: [u32; 2],
    pub province_id: u32,
    pub color_index: u8,
    pub density_f32_bits: u32,
    pub entity_scale_f32_bits: u32,
    pub entity_asset: UsedContentKey,
    pub mesh: UsedContentKey,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub citylight_state: O04CitylightState,
    /// Complete output set for this one entity instance, in display order.
    pub referenced_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum O04CitylightState {
    NotRegistered,
    RegisteredCitylight,
}

impl O04CityFrame {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != O04_CITY_SCHEMA {
            return Err("unsupported O04 city semantic schema".into());
        }
        check_sha256(&self.executable_sha256, None)?;
        check_sha256(&self.evidence_manifest_sha256, None)?;
        check_sha256(&self.live_city_submit_binding_sha256, None)?;
        check_sha256(&self.live_city_submit_verification_sha256, None)?;
        if self.executable_sha256 != O04_A729_EXECUTABLE_SHA256
            || self.evidence_manifest_sha256 != O04_EVIDENCE_MANIFEST_SHA256
            || self.live_city_submit_binding_sha256 != O04_LIVE_CITY_SUBMIT_BINDING_SHA256
            || self.live_city_submit_verification_sha256 != O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256
        {
            return Err("O04 city carrier evidence authority mismatch".into());
        }
        if self.first_sequence > self.last_sequence {
            return Err("O04 sequence range is reversed".into());
        }
        if self.cities.is_empty() || self.ordered_outputs.is_empty() {
            return Err("O04 city carrier is empty".into());
        }

        let mut previous_generator_ordinal = None;
        let mut generator_ordinals = BTreeSet::new();
        let mut entity_instance_ids = BTreeSet::new();
        let mut flattened = Vec::new();
        let mut unique_outputs = BTreeSet::new();
        for city in &self.cities {
            if city.event_sequence < self.first_sequence || city.event_sequence > self.last_sequence
            {
                return Err("O04 city event sequence is outside frame range".into());
            }
            if previous_generator_ordinal.is_some_and(|ordinal| city.generator_ordinal <= ordinal)
                || !generator_ordinals.insert(city.generator_ordinal)
            {
                return Err("O04 generator ordinals are not strictly increasing and unique".into());
            }
            previous_generator_ordinal = Some(city.generator_ordinal);

            let density = f32::from_bits(city.density_f32_bits);
            let scale = f32::from_bits(city.entity_scale_f32_bits);
            if !density.is_finite() || density < 0.0 || !scale.is_finite() || scale <= 0.0 {
                return Err("O04 density/scale raw bits are outside the captured domain".into());
            }
            if city
                .world_transform_4x4_f32_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
                || city.world_transform_4x4_f32_bits[3] != 0.0f32.to_bits()
                || city.world_transform_4x4_f32_bits[7] != 0.0f32.to_bits()
                || city.world_transform_4x4_f32_bits[11] != 0.0f32.to_bits()
                || city.world_transform_4x4_f32_bits[15] != 1.0f32.to_bits()
            {
                return Err("O04 city transform is not an exact finite affine matrix".into());
            }
            city.entity_asset.validate()?;
            city.mesh.validate()?;
            if city.referenced_outputs.is_empty() {
                return Err("O04 city has no submitted entity outputs".into());
            }
            let mut row_entity_instance_id = None;
            for output in &city.referenced_outputs {
                let entity_instance_id = match output {
                    RenderOutputRef::Entity {
                        entity_instance_id, ..
                    } => *entity_instance_id,
                    RenderOutputRef::Particle { .. } => {
                        return Err("O04 city references a non-entity output".into())
                    }
                };
                if row_entity_instance_id.is_some_and(|expected| expected != entity_instance_id) {
                    return Err("O04 city joins more than one entity instance".into());
                }
                row_entity_instance_id = Some(entity_instance_id);
                if !unique_outputs.insert(output.clone()) {
                    return Err("O04 city output is duplicated".into());
                }
                flattened.push(output.clone());
            }
            if !entity_instance_ids.insert(row_entity_instance_id.expect("nonempty outputs")) {
                return Err("O04 city entity instance is duplicated".into());
            }
        }
        if flattened != self.ordered_outputs {
            return Err("O04 ordered outputs differ from the per-city exact cover".into());
        }
        Ok(())
    }
}
