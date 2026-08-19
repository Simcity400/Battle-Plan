use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    base64_bytes, A729_EXECUTABLE_SHA256, SNOW_MUD_PRODUCER_FUNCTION_RVA,
    SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256,
};

pub const XMAP07_PASSIVE_SNOW_MUD_SCHEMA: &str = "hoi4.xmap07.passive-snow-mud.a729.v1";
pub const XMAP07_PASSIVE_SNOW_MUD_KEY: &str = "_xmap07_snow_mud_runtime";
pub const XMAP07_PASSIVE_SNOW_MUD_SCOPE: &str = "world_global";
pub const XMAP07_SNOW_MUD_FADE_UPDATER_RVA: u32 = 0x0126_7270;
pub const XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256: &str =
    "baacd7d1c462d74b44d8a793805049ddfa6cf1e9f7459177e663f1c9953f5575";
pub const XMAP07_SNOW_MUD_SCENE_COPY_RVA: u32 = 0x00ff_3fc0;
pub const XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256: &str =
    "a995d76eef9a6a295002bd1cad06b4902b86d5a3dd60fb6d10c63bc904f3084f";
pub const XMAP07_SNOW_MUD_FORMAT: &str = "DXGI_FORMAT_B8G8R8A8_UNORM";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap07PassiveSnowMudCarrier {
    pub schema: String,
    pub scope: String,
    pub executable_sha256: String,
    pub texture_producer_rva: u32,
    pub texture_producer_instructions_sha256: String,
    pub fade_updater_rva: u32,
    pub fade_updater_instructions_sha256: String,
    pub scene_copy_rva: u32,
    pub scene_copy_instructions_sha256: String,
    pub process_epoch: String,
    pub campaign_epoch: String,
    pub hook_epoch: String,
    pub reset_epoch_ordinal: u64,
    pub capture_frontier_ordinal: u64,
    pub texture_observation_ordinal: u64,
    pub fade_observation_ordinal: u64,
    pub reset_hour: i64,
    pub initial_snapshot_hour: i64,
    pub payload_origin_hour: i64,
    pub texture_observation_hour: i64,
    pub fade_observation_hour: i64,
    pub displayed_hour: i64,
    pub carry_forward: bool,
    pub reset_complete: bool,
    pub initial_snapshot_complete: bool,
    pub texture_complete: bool,
    pub fade_complete: bool,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub bytes_per_texel: u32,
    pub format: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    pub payload_sha256: String,
    pub payload_blob_sha256: String,
    pub fade_f32_bits: u32,
    pub authority_sha256: String,
}

impl Xmap07PassiveSnowMudCarrier {
    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let carrier: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid passive SnowMud JSON: {error}"))?;
        carrier.validate()?;
        Ok(carrier)
    }

    pub fn decode_value(value: &serde_json::Value) -> Result<Self, String> {
        let carrier: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid passive SnowMud value: {error}"))?;
        carrier.validate()?;
        Ok(carrier)
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn payload_blob_sha256(bytes: &[u8]) -> String {
        sha256(base64_bytes::encode(bytes).as_bytes())
    }

    pub fn authority_digest(&self) -> String {
        let mut preimage = Vec::new();
        append(&mut preimage, &self.schema);
        append(&mut preimage, &self.scope);
        append(&mut preimage, &self.executable_sha256);
        preimage.extend_from_slice(&self.texture_producer_rva.to_le_bytes());
        append(&mut preimage, &self.texture_producer_instructions_sha256);
        preimage.extend_from_slice(&self.fade_updater_rva.to_le_bytes());
        append(&mut preimage, &self.fade_updater_instructions_sha256);
        preimage.extend_from_slice(&self.scene_copy_rva.to_le_bytes());
        append(&mut preimage, &self.scene_copy_instructions_sha256);
        append(&mut preimage, &self.process_epoch);
        append(&mut preimage, &self.campaign_epoch);
        append(&mut preimage, &self.hook_epoch);
        for value in [
            self.reset_epoch_ordinal,
            self.capture_frontier_ordinal,
            self.texture_observation_ordinal,
            self.fade_observation_ordinal,
        ] {
            preimage.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            self.reset_hour,
            self.initial_snapshot_hour,
            self.payload_origin_hour,
            self.texture_observation_hour,
            self.fade_observation_hour,
            self.displayed_hour,
        ] {
            preimage.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            self.carry_forward,
            self.reset_complete,
            self.initial_snapshot_complete,
            self.texture_complete,
            self.fade_complete,
        ] {
            preimage.push(value as u8);
        }
        preimage.extend_from_slice(&self.width.to_le_bytes());
        preimage.extend_from_slice(&self.height.to_le_bytes());
        preimage.extend_from_slice(&self.row_pitch.to_le_bytes());
        preimage.extend_from_slice(&self.bytes_per_texel.to_le_bytes());
        append(&mut preimage, &self.format);
        append(&mut preimage, &self.payload_sha256);
        append(&mut preimage, &self.payload_blob_sha256);
        preimage.extend_from_slice(&self.fade_f32_bits.to_le_bytes());
        sha256(&preimage)
    }

    pub fn validate_for_hour(&self, displayed_hour: i64) -> Result<(), String> {
        self.validate()?;
        if self.displayed_hour != displayed_hour {
            return Err(format!(
                "passive SnowMud displayed hour {} does not match WorldFrame hour {displayed_hour}",
                self.displayed_hour
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP07_PASSIVE_SNOW_MUD_SCHEMA
            || self.scope != XMAP07_PASSIVE_SNOW_MUD_SCOPE
            || self.executable_sha256 != A729_EXECUTABLE_SHA256
            || self.texture_producer_rva != SNOW_MUD_PRODUCER_FUNCTION_RVA
            || self.texture_producer_instructions_sha256 != SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256
            || self.fade_updater_rva != XMAP07_SNOW_MUD_FADE_UPDATER_RVA
            || self.fade_updater_instructions_sha256
                != XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256
            || self.scene_copy_rva != XMAP07_SNOW_MUD_SCENE_COPY_RVA
            || self.scene_copy_instructions_sha256 != XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256
        {
            return Err("passive SnowMud producer authority is not exact a729".into());
        }
        for (label, epoch) in [
            ("process", self.process_epoch.as_str()),
            ("campaign", self.campaign_epoch.as_str()),
            ("hook", self.hook_epoch.as_str()),
        ] {
            if epoch.is_empty()
                || epoch.len() > 128
                || !epoch.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err(format!("passive SnowMud {label} epoch is invalid"));
            }
        }
        if self.reset_epoch_ordinal == 0
            || self.capture_frontier_ordinal == 0
            || self.texture_observation_ordinal == 0
            || self.fade_observation_ordinal == 0
            || self.texture_observation_ordinal > self.capture_frontier_ordinal
            || self.fade_observation_ordinal > self.capture_frontier_ordinal
        {
            return Err("passive SnowMud frontier/observation authority is invalid".into());
        }
        if !self.reset_complete
            || !self.initial_snapshot_complete
            || !self.texture_complete
            || !self.fade_complete
            || self.reset_hour > self.initial_snapshot_hour
            || self.initial_snapshot_hour > self.payload_origin_hour
            || self.payload_origin_hour > self.displayed_hour
            || self.texture_observation_hour != self.payload_origin_hour
            || self.fade_observation_hour != self.displayed_hour
            || (self.carry_forward && self.payload_origin_hour >= self.displayed_hour)
            || (!self.carry_forward && self.payload_origin_hour != self.displayed_hour)
        {
            return Err("passive SnowMud reset/hour/carry-forward completeness is invalid".into());
        }
        let expected_pitch = self
            .width
            .checked_mul(4)
            .ok_or_else(|| "passive SnowMud row pitch overflows".to_string())?;
        let expected_len = usize::try_from(expected_pitch)
            .ok()
            .and_then(|pitch| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| pitch.checked_mul(height))
            })
            .ok_or_else(|| "passive SnowMud payload length overflows".to_string())?;
        if self.width == 0
            || self.height == 0
            || self.bytes_per_texel != 4
            || self.row_pitch != expected_pitch
            || self.format != XMAP07_SNOW_MUD_FORMAT
            || self.bytes.len() != expected_len
            || self.payload_sha256 != sha256(&self.bytes)
            || self.payload_blob_sha256 != Self::payload_blob_sha256(&self.bytes)
        {
            return Err("passive SnowMud texture payload is incomplete or inauthentic".into());
        }
        let fade = f32::from_bits(self.fade_f32_bits);
        if !fade.is_finite() || !(0.0..=1.0).contains(&fade) {
            return Err("passive SnowMud fade is not a finite unit interval value".into());
        }
        if self.authority_sha256 != self.authority_digest() {
            return Err("passive SnowMud authority digest mismatch".into());
        }
        Ok(())
    }
}

fn append(preimage: &mut Vec<u8>, value: &str) {
    preimage.extend_from_slice(&(value.len() as u64).to_le_bytes());
    preimage.extend_from_slice(value.as_bytes());
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carrier() -> Xmap07PassiveSnowMudCarrier {
        let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut value = Xmap07PassiveSnowMudCarrier {
            schema: XMAP07_PASSIVE_SNOW_MUD_SCHEMA.into(),
            scope: XMAP07_PASSIVE_SNOW_MUD_SCOPE.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            texture_producer_rva: SNOW_MUD_PRODUCER_FUNCTION_RVA,
            texture_producer_instructions_sha256: SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256.into(),
            fade_updater_rva: XMAP07_SNOW_MUD_FADE_UPDATER_RVA,
            fade_updater_instructions_sha256: XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256
                .into(),
            scene_copy_rva: XMAP07_SNOW_MUD_SCENE_COPY_RVA,
            scene_copy_instructions_sha256: XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256.into(),
            process_epoch: "process-1".into(),
            campaign_epoch: "campaign-1".into(),
            hook_epoch: "hook-1".into(),
            reset_epoch_ordinal: 1,
            capture_frontier_ordinal: 13,
            texture_observation_ordinal: 11,
            fade_observation_ordinal: 13,
            reset_hour: 10,
            initial_snapshot_hour: 10,
            payload_origin_hour: 11,
            texture_observation_hour: 11,
            fade_observation_hour: 12,
            displayed_hour: 12,
            carry_forward: true,
            reset_complete: true,
            initial_snapshot_complete: true,
            texture_complete: true,
            fade_complete: true,
            width: 2,
            height: 1,
            row_pitch: 8,
            bytes_per_texel: 4,
            format: XMAP07_SNOW_MUD_FORMAT.into(),
            payload_sha256: sha256(&bytes),
            payload_blob_sha256: Xmap07PassiveSnowMudCarrier::payload_blob_sha256(&bytes),
            bytes,
            fade_f32_bits: 0.5f32.to_bits(),
            authority_sha256: String::new(),
        };
        value.authority_sha256 = value.authority_digest();
        value
    }

    #[test]
    fn exact_carrier_round_trips_and_rejects_authority_mutations() {
        let value = carrier();
        let encoded = value.encode_json().unwrap();
        assert_eq!(
            Xmap07PassiveSnowMudCarrier::decode_json(&encoded).unwrap(),
            value
        );

        let mut bad = value.clone();
        bad.capture_frontier_ordinal = 1;
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.reset_complete = false;
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.fade_f32_bits = f32::NAN.to_bits();
        assert!(bad.validate().is_err());
        let mut bad = value;
        bad.width = 3;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn fresh_and_carried_hours_are_explicit_and_unknown_fields_fail_closed() {
        let mut fresh = carrier();
        fresh.payload_origin_hour = fresh.displayed_hour;
        fresh.texture_observation_hour = fresh.displayed_hour;
        fresh.carry_forward = false;
        fresh.authority_sha256 = fresh.authority_digest();
        fresh.validate().unwrap();

        let mut raw: serde_json::Value =
            serde_json::from_slice(&fresh.encode_json().unwrap()).unwrap();
        raw.as_object_mut().unwrap().insert(
            "selected_present_sha256".into(),
            serde_json::Value::String("forbidden".into()),
        );
        assert!(Xmap07PassiveSnowMudCarrier::decode_value(&raw).is_err());
    }
}
