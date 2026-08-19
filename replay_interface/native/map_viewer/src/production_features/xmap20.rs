
use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    ProductionMapFeatureAdapter, Result,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    BufferSource, FamilyDisposition, MapVisualFamily, ParentAnchor, ParticlePacket, TextureBinding,
    Xmap20ActiveEffect, Xmap20DependencyReceipts, Xmap20EffectAdmission, Xmap20EffectFrame,
    XMAP20_EXECUTABLE_SHA256, XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA, XMAP20_PARTICLE_REGISTRY_RVA,
    XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::BTreeSet;

pub const XMAP20_PREPARED_FRAME_SCHEMA: &str = "hoi4.xmap20.prepared-effects.a729.v1";
pub const XMAP20_TRIGGER_RECEIPT_SCHEMA: &str = "hoi4.xmap20.authored-trigger.a729.v2";
pub const XMAP20_ASSIGNED_STAGE_NAME: &str = "particles_effects";
pub const XMAP20_ASSIGNED_OPERATION_NAME: &str = "particles.effects.draw";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20TriggerReceipt {
    pub schema: String,
    pub semantic_owner_packet: String,
    pub semantic_source_id: String,
    pub entity_instance_id: Option<u64>,
    pub source_content: UsedContentKey,
    pub entity_name: Option<String>,
    pub state_name: Option<String>,
    pub authored_event_index: u32,
    pub authored_effect_index: u32,
    pub particle_token: String,
    pub node_token: Option<String>,
    pub time_f32_bits: Option<u32>,
    pub keep_particle: Option<bool>,
    pub trigger_once: Option<bool>,
    pub owner_admission_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap20TriggerReceipt {
    pub fn seal(mut self) -> Result<Self> {
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP20 authored-trigger receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP20_TRIGGER_RECEIPT_SCHEMA
            || self.semantic_owner_packet.trim().is_empty()
            || self.semantic_source_id.trim().is_empty()
            || self.particle_token.trim().is_empty()
            || self
                .node_token
                .as_ref()
                .is_some_and(|node| node.trim().is_empty())
            || self
                .time_f32_bits
                .is_some_and(|bits| !f32::from_bits(bits).is_finite() || f32::from_bits(bits) < 0.0)
            || !valid_sha256(&self.owner_admission_sha256)
        {
            return Err("XMAP20 authored-trigger receipt is incomplete".into());
        }
        self.source_content
            .validate()
            .map_err(|error| format!("XMAP20 trigger content identity: {error}"))?;
        match (&self.entity_name, &self.state_name) {
            (Some(entity), Some(state))
                if !entity.trim().is_empty() && !state.trim().is_empty() => {}
            (None, None) if self.semantic_owner_packet != "XMAP13" => {}
            _ => {
                return Err(
                    "XMAP20 trigger entity/state identity disagrees with its semantic owner".into(),
                )
            }
        }
        match (self.semantic_owner_packet.as_str(), self.entity_instance_id) {
            ("XMAP13", Some(entity_instance_id)) if entity_instance_id != 0 => {}
            ("XMAP13", _) => {
                return Err("XMAP20 XMAP13 trigger omitted its exact entity instance".into())
            }
            (_, None) => {}
            _ => return Err("XMAP20 non-XMAP13 trigger claimed an entity instance".into()),
        }
        if self.semantic_owner_packet == "XMAP13"
            && self
                .node_token
                .as_ref()
                .is_none_or(|node| node.trim().is_empty())
        {
            return Err("XMAP20 XMAP13 trigger omitted its authored entity node".into());
        }
        let xmap19_identity = self.semantic_source_id.starts_with("xmap19.weather:");
        if (self.semantic_owner_packet == "XMAP19") != xmap19_identity {
            return Err("XMAP20 weather source identity is not owned exactly by XMAP19".into());
        }
        if xmap19_identity && !valid_xmap19_source_id(&self.semantic_source_id) {
            return Err("XMAP20 XMAP19 semantic source ID grammar is invalid".into());
        }
        Ok(())
    }
}

pub trait Xmap20TriggerProvider {
    fn trigger_for_effect(&self, effect: &Xmap20ActiveEffect) -> Result<Xmap20TriggerReceipt>;
}

pub struct Xmap20ProductionTriggerProvider<'a> {
    pub entity_identities: &'a crate::Xmap13EffectIdentityProvider,
    pub weather: &'a crate::PreparedXmap19WeatherFrame,
}

impl Xmap20TriggerProvider for Xmap20ProductionTriggerProvider<'_> {
    fn trigger_for_effect(&self, effect: &Xmap20ActiveEffect) -> Result<Xmap20TriggerReceipt> {
        match &effect.parent_anchor {
            ParentAnchor::Entity { .. } => self.entity_identities.trigger_for_effect(effect),
            ParentAnchor::World { .. } => {
                let trigger = self.weather.weather_trigger(
                    &effect.producer_instance_id,
                    effect.authored_event_index,
                    effect.authored_effect_index,
                )?;
                Xmap20TriggerReceipt {
                    schema: XMAP20_TRIGGER_RECEIPT_SCHEMA.into(),
                    semantic_owner_packet: "XMAP19".into(),
                    semantic_source_id: trigger.semantic_source_id,
                    entity_instance_id: None,
                    source_content: trigger.source_content,
                    entity_name: None,
                    state_name: None,
                    authored_event_index: trigger.authored_event_index,
                    authored_effect_index: trigger.authored_effect_index,
                    particle_token: trigger.particle_token,
                    node_token: trigger.node_token,
                    time_f32_bits: trigger.time_f32_bits,
                    keep_particle: trigger.keep_particle,
                    trigger_once: trigger.trigger_once,
                    owner_admission_sha256: trigger.owner_admission_sha256,
                    deterministic_sha256: String::new(),
                }
                .seal()
            }
        }
    }
}

pub struct Xmap20EffectsAdapter {
    prepared: PreparedXmap20Frame,
}

impl Xmap20EffectsAdapter {
    pub fn new(prepared: PreparedXmap20Frame) -> Result<Self> {
        prepared.validate()?;
        Ok(Self { prepared })
    }
}

impl ProductionMapFeatureAdapter for Xmap20EffectsAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::ParticlesEffects
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        self.prepared.validate()?;
        if context.world_frame.hour_index() != self.prepared.admission.hour_index {
            return Err("XMAP20 prepared effects belong to a different game hour".into());
        }
        let gap_id = "effects.semantic_lifecycle";
        if !context
            .map_authority_gaps
            .iter()
            .any(|gap| gap.input_id == gap_id)
        {
            return Err("XMAP20 semantic-lifecycle authority gap is absent".into());
        }
        let assigned = context
            .require_assigned_stage(XMAP20_ASSIGNED_STAGE_NAME, XMAP20_ASSIGNED_OPERATION_NAME)?;
        let producer_registration_sha256 = Sha256::hex(
            format!(
                "XMAP20\0{XMAP20_PREPARED_FRAME_SCHEMA}\0{XMAP20_EXECUTABLE_SHA256}\0{XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA:08x}\0{XMAP20_PARTICLE_REGISTRY_RVA:08x}\0{XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA:08x}"
            )
            .as_bytes(),
        );
        let disposition = if self.prepared.admission.active_effects.is_empty()
            && self.prepared.ordered_particles.is_empty()
        {
            FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256,
                enumerator_witness_sha256: self
                    .prepared
                    .admission
                    .complete_enumeration_sha256
                    .clone(),
            }
        } else {
            let program_sha256 = Sha256::hex(&serde_json::to_vec(
                &self
                    .prepared
                    .ordered_particles
                    .iter()
                    .map(|packet| &packet.shader_program)
                    .collect::<Vec<_>>(),
            )?);
            let ordered_resources_sha256 = Sha256::hex(&serde_json::to_vec(&(
                &self.prepared.trigger_receipts,
                &self.prepared.ordered_used_content,
                &self.prepared.admission.ordered_particle_submissions,
                &self.prepared.ordered_particles,
            ))?);
            let operation = assigned.foundation_operation_identity(
                program_sha256,
                ordered_resources_sha256,
                self.prepared.admission.provider_receipts_sha256.clone(),
            )?;
            FamilyDisposition::Present {
                payload_schema: XMAP20_PREPARED_FRAME_SCHEMA.into(),
                producer_registration_sha256,
                payload_sha256: self.prepared.deterministic_sha256.clone(),
                ordered_operations: vec![operation],
                ordered_used_content: self.prepared.ordered_used_content.clone(),
            }
        };
        Ok(FoundationFeatureOutput {
            disposition,
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_effects: Some(self.prepared.clone()),
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            consumed_gap_ids: vec![gap_id.into()],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedXmap20Frame {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub admission: Xmap20EffectAdmission,
    pub trigger_receipts: Vec<Xmap20TriggerReceipt>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub ordered_particles: Vec<ParticlePacket>,
    pub deterministic_sha256: String,
}

impl PreparedXmap20Frame {
    pub fn validate(&self) -> Result<()> {
        self.admission
            .validate()
            .map_err(|error| format!("XMAP20 portable admission: {error}"))?;
        for digest in [
            &self.content_manifest_sha256,
            &self.product_modset_sha256,
            &self.effective_vfs_manifest_sha256,
        ] {
            if !valid_sha256(digest) {
                return Err("XMAP20 prepared frame has invalid content identity".into());
            }
        }
        if self.schema != XMAP20_PREPARED_FRAME_SCHEMA
            || self.trigger_receipts.len() != self.admission.active_effects.len()
            || self.ordered_particles.len() != self.admission.ordered_particle_submissions.len()
        {
            return Err("XMAP20 prepared frame is incomplete".into());
        }
        for (trigger, effect) in self
            .trigger_receipts
            .iter()
            .zip(&self.admission.active_effects)
        {
            validate_trigger_join(effect, trigger)?;
        }
        let mut content = BTreeSet::new();
        for key in &self.ordered_used_content {
            key.validate()
                .map_err(|error| format!("XMAP20 used-content identity: {error}"))?;
            if key.effective_vfs_manifest_sha256 != self.effective_vfs_manifest_sha256 {
                return Err("XMAP20 used content belongs to a different effective VFS".into());
            }
            if !content.insert(key.clone()) {
                return Err("XMAP20 prepared content list contains a duplicate".into());
            }
        }
        let mut expected_content = self
            .trigger_receipts
            .iter()
            .map(|trigger| trigger.source_content.clone())
            .collect::<BTreeSet<_>>();
        for packet in &self.ordered_particles {
            collect_packet_content(packet, &mut expected_content);
        }
        if content != expected_content {
            return Err("XMAP20 prepared used-content census is incomplete or extraneous".into());
        }
        portable_render_stream::validate_xmap20_admitted_particles(
            &self.admission,
            &self.ordered_particles,
            &self.effective_vfs_manifest_sha256,
        )?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP20 prepared-frame digest mismatch".into());
        }
        Ok(())
    }
}

pub fn prepare_xmap20_effects(
    resolver: &GameFileResolver,
    frame: &Xmap20EffectFrame,
    dependencies: &Xmap20DependencyReceipts,
    triggers: &dyn Xmap20TriggerProvider,
) -> Result<PreparedXmap20Frame> {
    if !resolver.has_content_admission() {
        return Err("XMAP20 requires validated effective-VFS admission".into());
    }
    let admission = portable_render_stream::admit_xmap20_effects(frame, dependencies)
        .map_err(|error| format!("XMAP20 portable admission: {error}"))?;
    let mut trigger_receipts = Vec::with_capacity(admission.active_effects.len());
    let mut used = BTreeSet::new();
    for effect in &admission.active_effects {
        let trigger = triggers.trigger_for_effect(effect)?;
        validate_trigger_join(effect, &trigger)?;
        resolver.read_admitted_used_content(&trigger.source_content)?;
        used.insert(trigger.source_content.clone());
        trigger_receipts.push(trigger);
    }
    for packet in &frame.particles {
        collect_packet_content(packet, &mut used);
    }
    for content in &used {
        resolver.read_admitted_used_content(content)?;
    }
    let mut prepared = PreparedXmap20Frame {
        schema: XMAP20_PREPARED_FRAME_SCHEMA.into(),
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        product_modset_sha256: frame.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
        admission,
        trigger_receipts,
        ordered_used_content: used.into_iter().collect(),
        ordered_particles: frame.particles.clone(),
        deterministic_sha256: String::new(),
    };
    prepared.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&prepared)?);
    prepared.validate()?;
    Ok(prepared)
}

fn validate_trigger_join(
    effect: &Xmap20ActiveEffect,
    trigger: &Xmap20TriggerReceipt,
) -> Result<()> {
    trigger.validate()?;
    if trigger.semantic_source_id != effect.producer_instance_id
        || trigger.authored_event_index != effect.authored_event_index
        || trigger.authored_effect_index != effect.authored_effect_index
        || trigger.particle_token != effect.registration_token
    {
        return Err("XMAP20 authored trigger differs from lifecycle identity".into());
    }
    match &effect.parent_anchor {
        ParentAnchor::Entity {
            entity_instance_id,
            node_token,
        } => {
            if trigger.semantic_owner_packet != "XMAP13"
                || effect.producer_kind != "unit"
                || trigger.entity_instance_id != Some(*entity_instance_id)
                || trigger.node_token.as_deref() != Some(node_token.as_str())
            {
                return Err("XMAP20 entity trigger differs from XMAP13 node identity".into());
            }
        }
        ParentAnchor::World { .. } => {
            if effect.producer_kind != "weather"
                || trigger.semantic_owner_packet == "XMAP13"
                || trigger.entity_instance_id.is_some()
                || trigger.node_token.is_some()
            {
                return Err(
                    "XMAP20 world trigger cannot claim XMAP13 ownership or an entity node".into(),
                );
            }
        }
    }
    Ok(())
}

fn collect_packet_content(packet: &ParticlePacket, out: &mut BTreeSet<UsedContentKey>) {
    out.insert(packet.content_identity.definition_content.clone());
    out.insert(packet.content_identity.texture_content.clone());
    out.insert(packet.shader_program.source.clone());
    for texture in &packet.ordered_texture_bindings {
        collect_texture_content(texture, out);
    }
    for vertex in &packet.geometry.ordered_vertex_buffers {
        collect_buffer_content(&vertex.source, out);
    }
    if let Some(index) = &packet.geometry.index_buffer {
        collect_buffer_content(&index.source, out);
    }
}

fn collect_texture_content(texture: &TextureBinding, out: &mut BTreeSet<UsedContentKey>) {
    out.insert(texture.content.clone());
    for subresource in &texture.resource.ordered_subresources {
        collect_buffer_content(&subresource.source, out);
    }
}

fn collect_buffer_content(source: &BufferSource, out: &mut BTreeSet<UsedContentKey>) {
    if let BufferSource::UsedContentRange { content, .. } = source {
        out.insert(content.clone());
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_xmap19_source_id(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 6 || parts[0] != "xmap19.weather" || !matches!(parts[5], "primary" | "fading")
    {
        return false;
    }
    let Some(region) = canonical_prefixed_u64(parts[1], 'r') else {
        return false;
    };
    let Some(_epoch) = canonical_prefixed_u64(parts[2], 'e') else {
        return false;
    };
    let Some(_code) = canonical_prefixed_u64(parts[3], 'c') else {
        return false;
    };
    let Some(position) = canonical_prefixed_u64(parts[4], 'p') else {
        return false;
    };
    region != 0 && position != 0
}

fn canonical_prefixed_u64(value: &str, prefix: char) -> Option<u64> {
    let digits = value.strip_prefix(prefix)?;
    if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
        return None;
    }
    let parsed = digits.parse::<u64>().ok()?;
    (parsed.to_string() == digits).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;
    use portable_render_stream::{EffectOperation, ParentAnchor};

    fn content() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: "gfx/entities/units.asset".into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 64,
        }
    }

    fn effect() -> Xmap20ActiveEffect {
        Xmap20ActiveEffect {
            effect_instance_id: 7,
            registration_token: "mg_muzzle_smoke".into(),
            registration_hash_u32: 1,
            resolved_definition_sha256: "d".repeat(64),
            producer_kind: "unit".into(),
            producer_instance_id: "division:12".into(),
            authored_event_index: 4,
            authored_effect_index: 1,
            parent_anchor: ParentAnchor::Entity {
                entity_instance_id: 12,
                node_token: "muzzle".into(),
            },
            initial_rng_seed_u32: 2,
            initial_rng_counter_u32: 3,
            elapsed_f32_bits: 1.0f32.to_bits(),
            time_scale_f32_bits: 1.0f32.to_bits(),
            active_flags_u32: 1,
            last_operation: EffectOperation::Activate,
            last_sequence: 5,
            last_update_index: 6,
            ordered_packet_keys: Vec::new(),
        }
    }

    fn trigger() -> Xmap20TriggerReceipt {
        Xmap20TriggerReceipt {
            schema: XMAP20_TRIGGER_RECEIPT_SCHEMA.into(),
            semantic_owner_packet: "XMAP13".into(),
            semantic_source_id: "division:12".into(),
            entity_instance_id: Some(12),
            source_content: content(),
            entity_name: Some("GER_infantry".into()),
            state_name: Some("idle".into()),
            authored_event_index: 4,
            authored_effect_index: 1,
            particle_token: "mg_muzzle_smoke".into(),
            node_token: Some("muzzle".into()),
            time_f32_bits: Some(0.0f32.to_bits()),
            keep_particle: Some(false),
            trigger_once: Some(true),
            owner_admission_sha256: "e".repeat(64),
            deterministic_sha256: String::new(),
        }
        .seal()
        .unwrap()
    }

    #[test]
    fn entity_trigger_requires_exact_xmap13_authored_row() {
        validate_trigger_join(&effect(), &trigger()).unwrap();
        let mut wrong = trigger();
        wrong.particle_token = "different".into();
        wrong = wrong.seal().unwrap();
        assert!(validate_trigger_join(&effect(), &wrong).is_err());

        let mut wrong_entity = trigger();
        wrong_entity.entity_instance_id = Some(13);
        wrong_entity = wrong_entity.seal().unwrap();
        assert!(validate_trigger_join(&effect(), &wrong_entity).is_err());

        let mut wrong_kind = effect();
        wrong_kind.producer_kind = "weather".into();
        assert!(validate_trigger_join(&wrong_kind, &trigger()).is_err());
    }

    #[test]
    fn xmap13_trigger_requires_entity_and_state_identity() {
        let mut incomplete = trigger();
        incomplete.state_name = None;
        incomplete.deterministic_sha256.clear();
        assert!(incomplete.seal().is_err());
    }

    #[test]
    fn world_weather_trigger_preserves_omitted_node_and_flags() {
        let mut weather_effect = effect();
        weather_effect.producer_kind = "weather".into();
        weather_effect.producer_instance_id = "xmap19.weather:r12:e3:c2:p1:primary".into();
        weather_effect.authored_event_index = 0;
        weather_effect.authored_effect_index = 0;
        weather_effect.parent_anchor = ParentAnchor::World {
            world_transform_4x4_f32_bits: [0.0f32.to_bits(); 16],
        };
        let weather_trigger = Xmap20TriggerReceipt {
            schema: XMAP20_TRIGGER_RECEIPT_SCHEMA.into(),
            semantic_owner_packet: "XMAP19".into(),
            semantic_source_id: weather_effect.producer_instance_id.clone(),
            entity_instance_id: None,
            source_content: content(),
            entity_name: Some("weather_rain_light".into()),
            state_name: Some("default".into()),
            authored_event_index: 0,
            authored_effect_index: 0,
            particle_token: weather_effect.registration_token.clone(),
            node_token: None,
            time_f32_bits: Some(0.0f32.to_bits()),
            keep_particle: Some(true),
            trigger_once: None,
            owner_admission_sha256: "e".repeat(64),
            deterministic_sha256: String::new(),
        }
        .seal()
        .unwrap();
        validate_trigger_join(&weather_effect, &weather_trigger).unwrap();
    }
}
