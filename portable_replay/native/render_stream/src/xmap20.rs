//! XMAP20's typed joins to every production provider particles consume.
//!
//! These receipts carry identities, not replacement constants. XMAP20 never
//! patches a neutral light/fog/entity value when an owner is unavailable.

use serde::{Deserialize, Serialize};

pub const XMAP20_PROVIDER_RECEIPTS_SCHEMA: &str = "hoi4.xmap20.provider-receipts.v1";
pub const XMAP20_CAMERA_OWNER: &str = "XMAP03";
pub const XMAP20_LIGHTING_OWNER: &str = "XMAP09";
pub const XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER: &str = "XMAP10";
pub const XMAP20_ENTITY_IDENTITY_OWNER: &str = "XMAP13";
pub const XMAP20_WEATHER_IDENTITY_OWNER: &str = "XMAP19";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20DependencyReceipt {
    pub owner_packet: String,
    pub provider_schema: String,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
    pub execution_sha256: String,
}

impl Xmap20DependencyReceipt {
    fn validate(&self, expected_owner: &str) -> Result<(), String> {
        if self.owner_packet != expected_owner || self.provider_schema.trim().is_empty() {
            return Err(format!(
                "XMAP20 dependency receipt is not owned by {expected_owner}"
            ));
        }
        for (label, value) in [
            ("payload", &self.payload_sha256),
            ("descriptor", &self.descriptor_sha256),
            ("execution", &self.execution_sha256),
        ] {
            validate_sha256(value, &format!("{expected_owner} {label}"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20DependencyReceipts {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    /// Viewer camera/projection provider. Recorder camera state is forbidden.
    /// Its descriptor digest is the exact 64-byte viewer projection CB1
    /// payload digest bound during native execution.
    pub camera: Xmap20DependencyReceipt,
    pub lighting: Xmap20DependencyReceipt,
    /// XMAP10's pre-particle SceneCB0/MainScene state. This receipt must not
    /// include XMAP10's post-particle bright-pass/bloom/restore tail, which
    /// consumes XMAP20 output and would create a dependency cycle.
    pub scene_compositor_pre_particles: Xmap20DependencyReceipt,
    pub entity_identity: Xmap20DependencyReceipt,
    pub weather_identity: Xmap20DependencyReceipt,
    /// XMAP13-authenticated semantic source identities for the current hour.
    /// XMAP20 joins authored entity-state events through these stable strings;
    /// it never treats a process-local renderer pointer as semantic identity.
    pub ordered_entity_semantic_source_ids: Vec<String>,
    /// XMAP19-authenticated world/weather source identities in its exact
    /// producer order.
    pub ordered_weather_semantic_source_ids: Vec<String>,
}

impl Xmap20DependencyReceipts {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP20_PROVIDER_RECEIPTS_SCHEMA || self.hour_index < 0 {
            return Err("XMAP20 provider receipt schema mismatch".into());
        }
        validate_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(&self.effective_vfs_manifest_sha256, "effective VFS")?;
        self.camera.validate(XMAP20_CAMERA_OWNER)?;
        self.lighting.validate(XMAP20_LIGHTING_OWNER)?;
        self.scene_compositor_pre_particles
            .validate(XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER)?;
        self.entity_identity
            .validate(XMAP20_ENTITY_IDENTITY_OWNER)?;
        self.weather_identity
            .validate(XMAP20_WEATHER_IDENTITY_OWNER)?;
        validate_semantic_source_census(
            &self.ordered_entity_semantic_source_ids,
            "XMAP13",
        )?;
        validate_semantic_source_census(
            &self.ordered_weather_semantic_source_ids,
            "XMAP19",
        )?;
        Ok(())
    }

    pub fn admits_entity_semantic_source(&self, identity: &str) -> bool {
        self.ordered_entity_semantic_source_ids
            .iter()
            .any(|candidate| candidate == identity)
    }

    pub fn admits_weather_semantic_source(&self, identity: &str) -> bool {
        self.ordered_weather_semantic_source_ids
            .iter()
            .any(|candidate| candidate == identity)
    }
}

fn validate_semantic_source_census(values: &[String], owner: &str) -> Result<(), String> {
    let mut unique = std::collections::BTreeSet::new();
    if values
        .iter()
        .any(|identity| identity.trim().is_empty() || !unique.insert(identity))
    {
        return Err(format!(
            "XMAP20 {owner} semantic-source identities are blank or duplicated"
        ));
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

    fn receipt(owner: &str) -> Xmap20DependencyReceipt {
        Xmap20DependencyReceipt {
            owner_packet: owner.into(),
            provider_schema: format!("hoi4.{owner}.provider.v1"),
            payload_sha256: "a".repeat(64),
            descriptor_sha256: "b".repeat(64),
            execution_sha256: "c".repeat(64),
        }
    }

    #[test]
    fn provider_receipts_require_exact_owners_and_unique_ordered_censuses() {
        let mut receipts = Xmap20DependencyReceipts {
            schema: XMAP20_PROVIDER_RECEIPTS_SCHEMA.into(),
            hour_index: 12,
            content_manifest_sha256: "d".repeat(64),
            product_modset_sha256: "e".repeat(64),
            effective_vfs_manifest_sha256: "f".repeat(64),
            camera: receipt(XMAP20_CAMERA_OWNER),
            lighting: receipt(XMAP20_LIGHTING_OWNER),
            scene_compositor_pre_particles: receipt(XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER),
            entity_identity: receipt(XMAP20_ENTITY_IDENTITY_OWNER),
            weather_identity: receipt(XMAP20_WEATHER_IDENTITY_OWNER),
            ordered_entity_semantic_source_ids: vec!["division:1".into(), "division:2".into()],
            ordered_weather_semantic_source_ids: vec![
                "xmap19.weather:r1:e0:c2:p1:primary".into(),
            ],
        };
        receipts.validate().unwrap();
        assert!(receipts.admits_entity_semantic_source("division:2"));
        assert!(receipts
            .admits_weather_semantic_source("xmap19.weather:r1:e0:c2:p1:primary"));
        receipts.ordered_entity_semantic_source_ids.swap(0, 1);
        receipts.validate().unwrap();
        receipts.ordered_entity_semantic_source_ids.push("division:1".into());
        assert!(receipts.validate().is_err());
    }
}
