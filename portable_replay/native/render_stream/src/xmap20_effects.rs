//! Presentation-independent XMAP20 effect lifecycle and particle snapshot.
//!
//! This carrier is intentionally not a view over [`ProductionRenderFrame`].
//! That older frame selects a recorder Present and cannot be normal-product map
//! authority. XMAP20 instead consumes one unconditional game-hour lifecycle
//! census plus the exact post-simulation particle packets at the engine submit
//! boundary. Viewer-owned scene constants and render targets are supplied only
//! by the XMAP03/XMAP09/XMAP10 production providers at execution time.

use crate::{
    canonical_json_sha256, check_sha256, validate_constant_buffers,
    validate_runtime_draw_vfs_authority, validate_texture_bindings, EffectOperation,
    ParentAnchor, ParticlePacket, RenderOutputRef, Xmap20DependencyReceipts,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP20_EFFECT_FRAME_SCHEMA: &str = "hoi4.xmap20.effect-frame.a729.v1";
pub const XMAP20_EFFECT_ADMISSION_SCHEMA: &str = "hoi4.xmap20.effects-admission.a729.v1";
pub const XMAP20_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA: u32 = 0x0228_24a0;
pub const XMAP20_PARTICLE_REGISTRY_RVA: u32 = 0x023e_8590;
pub const XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA: u32 = 0x0227_f420;
const PARTICLE_SHADER_PATH: &str = "gfx/fx/particle.shader";

/// One unconditional, complete semantic/game-state effect census for an hour.
///
/// `particles` retain the old packet's authenticated draw descriptor only as
/// evidence for its exact program/resource/geometry state. Product execution
/// never consumes the packet's recorder output target or recorder SceneCBs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20EffectFrame {
    pub schema: String,
    pub executable_sha256: String,
    pub hour_index: i64,
    #[serde(with = "crate::decimal_u64")]
    pub update_index: u64,
    #[serde(with = "crate::decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "crate::decimal_u64")]
    pub last_sequence: u64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub producer_receipt_sha256: String,
    pub complete_enumeration_sha256: String,
    pub ordered_lifecycle_events: Vec<Xmap20LifecycleEvent>,
    pub active_effects: Vec<Xmap20LifecycleEvent>,
    pub particles: Vec<ParticlePacket>,
    pub ordered_particle_outputs: Vec<RenderOutputRef>,
    pub lifecycle_sha256: String,
    pub active_effects_sha256: String,
    pub ordered_particles_sha256: String,
    pub deterministic_sha256: String,
}

/// XMAP20's presentation-independent lifecycle row. It is separate from the
/// legacy selected-submit lifecycle type so exact authored-effect identity
/// does not silently change production-render schema v13.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20LifecycleEvent {
    #[serde(with = "crate::decimal_u64")]
    pub sequence: u64,
    #[serde(with = "crate::decimal_u64")]
    pub update_index: u64,
    #[serde(with = "crate::decimal_u64")]
    pub effect_instance_id: u64,
    pub operation: EffectOperation,
    pub registration_token: String,
    pub registration_hash_u32: u32,
    pub resolved_definition_sha256: String,
    pub producer_kind: String,
    pub producer_instance_id: String,
    pub authored_event_index: u32,
    /// Zero-based source-order ordinal among every effect invocation inside
    /// the authored event block, including non-particle effects.
    pub authored_effect_index: u32,
    pub parent_anchor: ParentAnchor,
    pub initial_rng_seed_u32: u32,
    pub initial_rng_counter_u32: u32,
    pub elapsed_f32_bits: u32,
    pub time_scale_f32_bits: u32,
    pub active_flags_u32: u32,
}

impl Xmap20EffectFrame {
    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let frame: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("XMAP20 effect-frame JSON: {error}"))?;
        frame.validate()?;
        Ok(frame)
    }

    /// Decode from a JSON value that has already passed the replay source's
    /// duplicate-key parse boundary.
    pub fn decode_value(value: &serde_json::Value) -> Result<Self, String> {
        let frame: Self = Self::deserialize(value)
            .map_err(|error| format!("XMAP20 effect-frame JSON: {error}"))?;
        frame.validate()?;
        Ok(frame)
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn seal(mut self) -> Result<Self, String> {
        self.lifecycle_sha256 = canonical_json_sha256(&self.ordered_lifecycle_events)?;
        self.active_effects_sha256 = canonical_json_sha256(&self.active_effects)?;
        self.ordered_particles_sha256 = canonical_json_sha256(&self.particles)?;
        self.deterministic_sha256.clear();
        self.deterministic_sha256 = canonical_json_sha256(&self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP20_EFFECT_FRAME_SCHEMA
            || self.executable_sha256 != XMAP20_EXECUTABLE_SHA256
            || self.hour_index < 0
            || self.update_index == 0
            || self.first_sequence == 0
            || self.last_sequence < self.first_sequence
        {
            return Err("XMAP20 effect frame authority/frontier is invalid".into());
        }
        for (label, value) in [
            ("content manifest", &self.content_manifest_sha256),
            ("product modset", &self.product_modset_sha256),
            ("effective VFS", &self.effective_vfs_manifest_sha256),
            ("producer receipt", &self.producer_receipt_sha256),
            ("complete enumeration", &self.complete_enumeration_sha256),
        ] {
            validate_sha256(value, label)?;
        }
        if self.lifecycle_sha256 != canonical_json_sha256(&self.ordered_lifecycle_events)?
            || self.active_effects_sha256 != canonical_json_sha256(&self.active_effects)?
            || self.ordered_particles_sha256 != canonical_json_sha256(&self.particles)?
        {
            return Err("XMAP20 effect-frame component digest mismatch".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_json_sha256(&unsigned)? {
            return Err("XMAP20 effect-frame deterministic digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20ActiveEffect {
    #[serde(with = "crate::decimal_u64")]
    pub effect_instance_id: u64,
    pub registration_token: String,
    pub registration_hash_u32: u32,
    pub resolved_definition_sha256: String,
    pub producer_kind: String,
    pub producer_instance_id: String,
    pub authored_event_index: u32,
    pub authored_effect_index: u32,
    pub parent_anchor: ParentAnchor,
    pub initial_rng_seed_u32: u32,
    pub initial_rng_counter_u32: u32,
    pub elapsed_f32_bits: u32,
    pub time_scale_f32_bits: u32,
    pub active_flags_u32: u32,
    pub last_operation: EffectOperation,
    #[serde(with = "crate::decimal_u64")]
    pub last_sequence: u64,
    #[serde(with = "crate::decimal_u64")]
    pub last_update_index: u64,
    /// Complete packet keys for this instance at the hour checkpoint. Empty is
    /// an authenticated no-live-particles state, never an omitted carrier.
    pub ordered_packet_keys: Vec<[u32; 3]>,
}

impl Xmap20ActiveEffect {
    fn from_checkpoint(
        event: &EffectLifecycleEvent,
        packets: &[&ParticlePacket],
    ) -> Xmap20ActiveEffect {
        Self {
            effect_instance_id: event.effect_instance_id,
            registration_token: event.registration_token.clone(),
            registration_hash_u32: event.registration_hash_u32,
            resolved_definition_sha256: event.resolved_definition_sha256.clone(),
            producer_kind: event.producer_kind.clone(),
            producer_instance_id: event.producer_instance_id.clone(),
            authored_event_index: event.authored_event_index,
            authored_effect_index: event.authored_effect_index,
            parent_anchor: event.parent_anchor.clone(),
            initial_rng_seed_u32: event.initial_rng_seed_u32,
            initial_rng_counter_u32: event.initial_rng_counter_u32,
            elapsed_f32_bits: event.elapsed_f32_bits,
            time_scale_f32_bits: event.time_scale_f32_bits,
            active_flags_u32: event.active_flags_u32,
            last_operation: event.operation,
            last_sequence: event.sequence,
            last_update_index: event.update_index,
            ordered_packet_keys: packets
                .iter()
                .map(|packet| [packet.bucket, packet.packet_ordinal, packet.draw_order])
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20ParticleSubmissionRef {
    #[serde(with = "crate::decimal_u64")]
    pub effect_instance_id: u64,
    pub bucket: u32,
    pub packet_ordinal: u32,
    pub draw_order: u32,
    pub definition_sha256: String,
    pub texture_sha256: String,
    pub shader_sha256: String,
    pub instance_records_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20EffectAdmission {
    pub schema: String,
    pub hour_index: i64,
    #[serde(with = "crate::decimal_u64")]
    pub update_index: u64,
    pub particle_definition_lookup_rva: u32,
    pub particle_registry_rva: u32,
    pub particle_submit_boundary_rva: u32,
    pub active_effects: Vec<Xmap20ActiveEffect>,
    pub ordered_particle_submissions: Vec<Xmap20ParticleSubmissionRef>,
    pub provider_receipts_sha256: String,
    pub producer_receipt_sha256: String,
    pub complete_enumeration_sha256: String,
    pub lifecycle_sha256: String,
    pub ordered_submissions_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap20EffectAdmission {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP20_EFFECT_ADMISSION_SCHEMA
            || self.hour_index < 0
            || self.update_index == 0
            || self.particle_definition_lookup_rva != XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA
            || self.particle_registry_rva != XMAP20_PARTICLE_REGISTRY_RVA
            || self.particle_submit_boundary_rva != XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA
            || self.lifecycle_sha256 != canonical_json_sha256(&self.active_effects)?
            || self.ordered_submissions_sha256
                != canonical_json_sha256(&self.ordered_particle_submissions)?
        {
            return Err("XMAP20 effect admission identity/component digest mismatch".into());
        }
        validate_sha256(&self.provider_receipts_sha256, "provider receipts")?;
        validate_sha256(&self.producer_receipt_sha256, "producer receipt")?;
        validate_sha256(
            &self.complete_enumeration_sha256,
            "complete enumeration",
        )?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_json_sha256(&unsigned)? {
            return Err("XMAP20 effect admission deterministic digest mismatch".into());
        }
        Ok(())
    }
}

pub fn admit_xmap20_effects(
    frame: &Xmap20EffectFrame,
    dependencies: &Xmap20DependencyReceipts,
) -> Result<Xmap20EffectAdmission, String> {
    frame.validate()?;
    dependencies.validate()?;
    if dependencies.hour_index != frame.hour_index
        || dependencies.content_manifest_sha256 != frame.content_manifest_sha256
        || dependencies.product_modset_sha256 != frame.product_modset_sha256
        || dependencies.effective_vfs_manifest_sha256 != frame.effective_vfs_manifest_sha256
    {
        return Err("XMAP20 provider receipts belong to a different hour/content epoch".into());
    }
    let active_effects = validate_lifecycle(frame, dependencies)?;
    let ordered_particle_submissions = validate_particles(frame, &active_effects)?;
    let provider_receipts_sha256 = canonical_json_sha256(dependencies)?;
    let lifecycle_sha256 = canonical_json_sha256(&active_effects)?;
    let ordered_submissions_sha256 = canonical_json_sha256(&ordered_particle_submissions)?;
    let mut admission = Xmap20EffectAdmission {
        schema: XMAP20_EFFECT_ADMISSION_SCHEMA.into(),
        hour_index: frame.hour_index,
        update_index: frame.update_index,
        particle_definition_lookup_rva: XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA,
        particle_registry_rva: XMAP20_PARTICLE_REGISTRY_RVA,
        particle_submit_boundary_rva: XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA,
        active_effects,
        ordered_particle_submissions,
        provider_receipts_sha256,
        producer_receipt_sha256: frame.producer_receipt_sha256.clone(),
        complete_enumeration_sha256: frame.complete_enumeration_sha256.clone(),
        lifecycle_sha256,
        ordered_submissions_sha256,
        deterministic_sha256: String::new(),
    };
    admission.deterministic_sha256 = canonical_json_sha256(&admission)?;
    admission.validate()?;
    Ok(admission)
}

fn validate_lifecycle(
    frame: &Xmap20EffectFrame,
    dependencies: &Xmap20DependencyReceipts,
) -> Result<Vec<Xmap20ActiveEffect>, String> {
    let mut prior_sequence = None;
    let mut observed: BTreeMap<u64, Xmap20ActiveEffect> = BTreeMap::new();
    let mut destroyed = BTreeSet::new();
    for event in &frame.ordered_lifecycle_events {
        validate_event(event, dependencies)?;
        if event.sequence < frame.first_sequence
            || event.sequence > frame.last_sequence
            || prior_sequence.is_some_and(|prior| event.sequence <= prior)
        {
            return Err(
                "XMAP20 lifecycle sequence is outside its frontier or non-monotonic".into(),
            );
        }
        prior_sequence = Some(event.sequence);
        match event.operation {
            EffectOperation::Spawn => {
                if destroyed.contains(&event.effect_instance_id)
                    || observed
                        .insert(
                            event.effect_instance_id,
                            Xmap20ActiveEffect::from_checkpoint(event, &[]),
                        )
                        .is_some()
                {
                    return Err("XMAP20 effect instance is spawned more than once".into());
                }
            }
            EffectOperation::Destroy => {
                if let Some(prior) = observed.remove(&event.effect_instance_id) {
                    validate_identity_unchanged(&prior, event)?;
                }
                if !destroyed.insert(event.effect_instance_id) {
                    return Err("XMAP20 effect instance is destroyed more than once".into());
                }
            }
            EffectOperation::Activate
            | EffectOperation::Deactivate
            | EffectOperation::StopEmission
            | EffectOperation::Reparent => {
                if destroyed.contains(&event.effect_instance_id) {
                    return Err("XMAP20 lifecycle mutates an already-destroyed effect".into());
                }
                if let Some(prior) = observed.get(&event.effect_instance_id) {
                    validate_identity_unchanged(prior, event)?;
                    validate_frontier_forward(prior, event)?;
                    if event.operation != EffectOperation::Reparent
                        && prior.parent_anchor != event.parent_anchor
                    {
                        return Err(
                            "XMAP20 effect anchor changed outside an exact reparent operation"
                                .into(),
                        );
                    }
                }
                observed.insert(
                    event.effect_instance_id,
                    Xmap20ActiveEffect::from_checkpoint(event, &[]),
                );
            }
        }
    }

    let mut particles_by_effect: BTreeMap<u64, Vec<&ParticlePacket>> = BTreeMap::new();
    for packet in &frame.particles {
        particles_by_effect
            .entry(packet.effect_instance_id)
            .or_default()
            .push(packet);
    }
    let mut checkpoint = BTreeMap::new();
    for event in &frame.active_effects {
        validate_event(event, dependencies)?;
        if event.operation == EffectOperation::Destroy
            || destroyed.contains(&event.effect_instance_id)
            || event.update_index != frame.update_index
            || event.sequence > frame.last_sequence
        {
            return Err("XMAP20 active checkpoint has an invalid frontier/state".into());
        }
        if let Some(prior) = observed.get(&event.effect_instance_id) {
            validate_identity_unchanged(prior, event)?;
            validate_frontier_forward(prior, event)?;
            if event.sequence != prior.last_sequence
                || event.operation != prior.last_operation
                || event.parent_anchor != prior.parent_anchor
            {
                return Err(
                    "XMAP20 active checkpoint differs from its final lifecycle operation".into(),
                );
            }
        }
        let packets = particles_by_effect
            .get(&event.effect_instance_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if checkpoint
            .insert(
                event.effect_instance_id,
                Xmap20ActiveEffect::from_checkpoint(event, packets),
            )
            .is_some()
        {
            return Err("XMAP20 active checkpoint repeats an effect instance".into());
        }
    }
    if observed
        .keys()
        .any(|identity| !destroyed.contains(identity) && !checkpoint.contains_key(identity))
        || particles_by_effect
            .keys()
            .any(|identity| !checkpoint.contains_key(identity))
    {
        return Err(
            "XMAP20 checkpoint omits a live lifecycle or submitted particle instance".into(),
        );
    }
    Ok(checkpoint.into_values().collect())
}

fn validate_event(
    event: &EffectLifecycleEvent,
    dependencies: &Xmap20DependencyReceipts,
) -> Result<(), String> {
    if event.effect_instance_id == 0
        || event.sequence == 0
        || event.registration_token.trim().is_empty()
        || event.registration_hash_u32 != fnv1a32(event.registration_token.as_bytes())
        || event.resolved_definition_sha256.len() != 64
        || event.producer_kind.trim().is_empty()
        || event.producer_instance_id.trim().is_empty()
    {
        return Err("XMAP20 lifecycle event has incomplete identity".into());
    }
    validate_sha256(
        &event.resolved_definition_sha256,
        "resolved particle definition",
    )?;
    let elapsed = f32::from_bits(event.elapsed_f32_bits);
    let scale = f32::from_bits(event.time_scale_f32_bits);
    if !elapsed.is_finite() || elapsed < 0.0 || !scale.is_finite() {
        return Err("XMAP20 lifecycle event has invalid elapsed/time scale".into());
    }
    match &event.parent_anchor {
        ParentAnchor::Entity {
            entity_instance_id,
            node_token,
        } => {
            if *entity_instance_id == 0
                || node_token.trim().is_empty()
                || !dependencies.admits_entity_semantic_source(&event.producer_instance_id)
            {
                return Err(
                    "XMAP20 entity anchor is not joined to XMAP13 semantic identity".into(),
                );
            }
        }
        ParentAnchor::World {
            world_transform_4x4_f32_bits,
        } => {
            if !world_transform_4x4_f32_bits
                .iter()
                .all(|bits| f32::from_bits(*bits).is_finite())
                || !dependencies.admits_weather_semantic_source(&event.producer_instance_id)
            {
                return Err(
                    "XMAP20 world anchor is non-finite or not joined to XMAP19 identity".into(),
                );
            }
        }
    }
    Ok(())
}

fn validate_identity_unchanged(
    prior: &Xmap20ActiveEffect,
    event: &EffectLifecycleEvent,
) -> Result<(), String> {
    if prior.registration_token != event.registration_token
        || prior.registration_hash_u32 != event.registration_hash_u32
        || prior.resolved_definition_sha256 != event.resolved_definition_sha256
        || prior.producer_kind != event.producer_kind
        || prior.producer_instance_id != event.producer_instance_id
        || prior.authored_event_index != event.authored_event_index
        || prior.authored_effect_index != event.authored_effect_index
        || prior.initial_rng_seed_u32 != event.initial_rng_seed_u32
        || prior.initial_rng_counter_u32 != event.initial_rng_counter_u32
    {
        return Err("XMAP20 stable effect identity changed during its lifetime".into());
    }
    Ok(())
}

fn validate_frontier_forward(
    prior: &Xmap20ActiveEffect,
    event: &EffectLifecycleEvent,
) -> Result<(), String> {
    if event.update_index < prior.last_update_index {
        return Err("XMAP20 effect update frontier moved backwards".into());
    }
    Ok(())
}

fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

fn validate_particles(
    frame: &Xmap20EffectFrame,
    active_effects: &[Xmap20ActiveEffect],
) -> Result<Vec<Xmap20ParticleSubmissionRef>, String> {
    let active = active_effects
        .iter()
        .map(|effect| (effect.effect_instance_id, effect))
        .collect::<BTreeMap<_, _>>();
    let mut keys = BTreeSet::new();
    let packets = frame
        .particles
        .iter()
        .map(|packet| {
            if !keys.insert((
                packet.effect_instance_id,
                packet.bucket,
                packet.packet_ordinal,
                packet.draw_order,
            )) {
                return Err("XMAP20 repeats a particle packet key".into());
            }
            let effect = active
                .get(&packet.effect_instance_id)
                .ok_or("XMAP20 particle packet has no active lifecycle instance")?;
            validate_packet(packet, effect, &frame.effective_vfs_manifest_sha256)?;
            Ok(Xmap20ParticleSubmissionRef {
                effect_instance_id: packet.effect_instance_id,
                bucket: packet.bucket,
                packet_ordinal: packet.packet_ordinal,
                draw_order: packet.draw_order,
                definition_sha256: packet.resolved_definition_sha256.clone(),
                texture_sha256: packet
                    .content_identity
                    .texture_content
                    .payload_sha256
                    .clone(),
                shader_sha256: packet.shader_program.source.payload_sha256.clone(),
                instance_records_sha256: packet.gpu_instance_records_sha256.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ordered_refs = frame
        .ordered_particle_outputs
        .iter()
        .map(|output| match output {
            RenderOutputRef::Particle {
                effect_instance_id,
                bucket,
                packet_ordinal,
                draw_order,
            } => Ok((*effect_instance_id, *bucket, *packet_ordinal, *draw_order)),
            RenderOutputRef::Entity { .. } => {
                Err("XMAP20 particle order contains a non-particle output".to_string())
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let packet_refs = packets
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
    if ordered_refs != packet_refs {
        return Err("XMAP20 particle vector differs from authoritative submit order".into());
    }
    Ok(packets)
}

/// Revalidate a prepared packet vector against its portable admission.
///
/// Prepared frames are public transport values, so a deterministic digest is
/// not a substitute for repeating the structural/content checks at the next
/// trust boundary.
pub fn validate_xmap20_admitted_particles(
    admission: &Xmap20EffectAdmission,
    particles: &[ParticlePacket],
    effective_vfs_manifest_sha256: &str,
) -> Result<(), String> {
    admission.validate()?;
    validate_sha256(effective_vfs_manifest_sha256, "effective VFS")?;
    if particles.len() != admission.ordered_particle_submissions.len() {
        return Err("XMAP20 prepared packet count differs from portable admission".into());
    }
    let mut prior_effect_id = None;
    let active = admission
        .active_effects
        .iter()
        .map(|effect| {
            validate_active_effect(effect)?;
            if prior_effect_id.is_some_and(|prior| prior >= effect.effect_instance_id) {
                return Err(
                    "XMAP20 admitted active effects are duplicated or out of order".to_string(),
                );
            }
            prior_effect_id = Some(effect.effect_instance_id);
            Ok((effect.effect_instance_id, effect))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let mut packet_keys_by_effect: BTreeMap<u64, Vec<[u32; 3]>> = BTreeMap::new();
    for packet in particles {
        packet_keys_by_effect
            .entry(packet.effect_instance_id)
            .or_default()
            .push([packet.bucket, packet.packet_ordinal, packet.draw_order]);
    }
    for effect in active.values() {
        if effect.ordered_packet_keys
            != packet_keys_by_effect
                .get(&effect.effect_instance_id)
                .cloned()
                .unwrap_or_default()
        {
            return Err("XMAP20 active-effect packet census differs from prepared packets".into());
        }
    }
    for (packet, reference) in particles
        .iter()
        .zip(&admission.ordered_particle_submissions)
    {
        let effect = active
            .get(&packet.effect_instance_id)
            .ok_or("XMAP20 prepared packet has no admitted active effect")?;
        validate_packet(packet, effect, effective_vfs_manifest_sha256)?;
        if packet.effect_instance_id != reference.effect_instance_id
            || packet.bucket != reference.bucket
            || packet.packet_ordinal != reference.packet_ordinal
            || packet.draw_order != reference.draw_order
            || packet.resolved_definition_sha256 != reference.definition_sha256
            || packet.content_identity.texture_content.payload_sha256
                != reference.texture_sha256
            || packet.shader_program.source.payload_sha256 != reference.shader_sha256
            || packet.gpu_instance_records_sha256 != reference.instance_records_sha256
        {
            return Err("XMAP20 prepared packet differs from portable admission".into());
        }
    }
    Ok(())
}

fn validate_active_effect(effect: &Xmap20ActiveEffect) -> Result<(), String> {
    if effect.effect_instance_id == 0
        || effect.registration_token.trim().is_empty()
        || effect.registration_hash_u32 != fnv1a32(effect.registration_token.as_bytes())
        || effect.producer_kind.trim().is_empty()
        || effect.producer_instance_id.trim().is_empty()
        || effect.last_sequence == 0
        || effect.last_update_index == 0
        || effect.last_operation == EffectOperation::Destroy
    {
        return Err("XMAP20 admitted active effect has incomplete identity/frontier".into());
    }
    validate_sha256(
        &effect.resolved_definition_sha256,
        "resolved particle definition",
    )?;
    let elapsed = f32::from_bits(effect.elapsed_f32_bits);
    let time_scale = f32::from_bits(effect.time_scale_f32_bits);
    if !elapsed.is_finite() || elapsed < 0.0 || !time_scale.is_finite() {
        return Err("XMAP20 admitted active effect has invalid time state".into());
    }
    match &effect.parent_anchor {
        ParentAnchor::Entity {
            entity_instance_id,
            node_token,
        } if *entity_instance_id != 0 && !node_token.trim().is_empty() => {}
        ParentAnchor::World {
            world_transform_4x4_f32_bits,
        } if world_transform_4x4_f32_bits
            .iter()
            .all(|bits| f32::from_bits(*bits).is_finite()) => {}
        _ => return Err("XMAP20 admitted active effect has an invalid parent anchor".into()),
    }
    Ok(())
}

fn validate_packet(
    packet: &ParticlePacket,
    effect: &Xmap20ActiveEffect,
    authority_manifest_sha256: &str,
) -> Result<(), String> {
    if packet.registration_token != effect.registration_token
        || packet.resolved_definition_sha256 != effect.resolved_definition_sha256
        || packet.elapsed_f32_bits != effect.elapsed_f32_bits
        || packet
            .shader_program
            .source
            .normalized_game_relative_path
            .to_ascii_lowercase()
            != PARTICLE_SHADER_PATH
    {
        return Err(
            "XMAP20 packet identity/program/time differs from its active lifecycle instance".into(),
        );
    }
    if packet.canonical_packet_112_bytes.len() != 112
        || packet.canonical_packet_112_bytes[0x48..0x50] != [0; 8]
        || packet.record_stride != 48
        || packet.live_count > packet.max_count
        || packet.ordered_gpu_instance_records.len()
            != usize::try_from(packet.live_count)
                .ok()
                .and_then(|count| count.checked_mul(48))
                .ok_or("XMAP20 particle instance byte count overflow")?
    {
        return Err("XMAP20 canonical packet/instance layout is invalid".into());
    }
    check_sha256(
        &packet.canonical_packet_sha256,
        Some(&packet.canonical_packet_112_bytes),
    )?;
    check_sha256(
        &packet.gpu_instance_records_sha256,
        Some(&packet.ordered_gpu_instance_records),
    )?;
    packet.content_identity.validate()?;
    if packet.content_identity.computed_sha256()? != packet.content_identity_sha256
        || packet.resolved_definition_sha256
            != packet.content_identity.definition_content.payload_sha256
        || packet.packet_ordinal != packet.content_identity.subsystem_selector_u32
    {
        return Err("XMAP20 particle content/subsystem identity mismatch".into());
    }
    packet.shader_program.validate()?;
    validate_texture_bindings(&packet.ordered_texture_bindings)?;
    packet.pipeline.validate()?;
    packet.geometry.validate(&packet.pipeline, true)?;
    validate_constant_buffers(&packet.ordered_constant_buffers)?;
    packet.output_state.validate()?;
    validate_runtime_draw_vfs_authority(
        &[
            &packet.content_identity.definition_content,
            &packet.content_identity.texture_content,
        ],
        &packet.shader_program,
        &packet.ordered_texture_bindings,
        &packet.geometry,
        &packet.output_state,
        authority_manifest_sha256,
        "XMAP20 particle",
    )?;
    let selected_texture = packet
        .ordered_texture_bindings
        .iter()
        .find(|binding| binding.content == packet.content_identity.texture_content)
        .ok_or("XMAP20 selected texture is absent from exact bindings")?;
    if packet.shader_program.effect_token != packet.content_identity.engine_effect_token
        || packet.atlas_columns == 0
        || packet.atlas_rows == 0
        || packet.atlas_columns != packet.content_identity.atlas_columns_u32
        || packet.atlas_rows != packet.content_identity.atlas_rows_u32
        || packet.content_identity.atlas_half_pixel_width_f32_bits
            != (0.5f32 / selected_texture.resource.width as f32).to_bits()
        || packet.content_identity.atlas_half_pixel_height_f32_bits
            != (0.5f32 / selected_texture.resource.height as f32).to_bits()
    {
        return Err("XMAP20 shader/atlas identity mismatch".into());
    }
    let mut required = BTreeSet::from([(1u8, 64u32), (2, 16), (3, 96)]);
    for binding in &packet.ordered_constant_buffers {
        if binding.stage_mask_u8 != 0b1
            || binding.byte_offset != 0
            || !required.contains(&(binding.slot, binding.byte_count))
        {
            return Err(
                "XMAP20 particle packet has a non-VS or unexpected per-draw constant buffer"
                    .into(),
            );
        }
        required.remove(&(binding.slot, binding.byte_count));
    }
    if !required.is_empty() {
        return Err("XMAP20 particle packet omits exact CB1/CB2/CB3 payloads".into());
    }
    if let ParentAnchor::World {
        world_transform_4x4_f32_bits,
    } = &effect.parent_anchor
    {
        if world_transform_4x4_f32_bits != &packet.packet_matrix_4x4_f32_bits {
            return Err("XMAP20 world-anchor transform differs from packet matrix".into());
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
    use crate::{
        Xmap20DependencyReceipt, XMAP20_CAMERA_OWNER, XMAP20_ENTITY_IDENTITY_OWNER,
        XMAP20_LIGHTING_OWNER, XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER,
        XMAP20_PROVIDER_RECEIPTS_SCHEMA, XMAP20_WEATHER_IDENTITY_OWNER,
    };

    fn receipt(owner: &str, byte: char) -> Xmap20DependencyReceipt {
        Xmap20DependencyReceipt {
            owner_packet: owner.into(),
            provider_schema: format!("hoi4.{owner}.provider.v1"),
            payload_sha256: byte.to_string().repeat(64),
            descriptor_sha256: "a".repeat(64),
            execution_sha256: "b".repeat(64),
        }
    }

    fn dependencies() -> Xmap20DependencyReceipts {
        Xmap20DependencyReceipts {
            schema: XMAP20_PROVIDER_RECEIPTS_SCHEMA.into(),
            hour_index: 42,
            content_manifest_sha256: "c".repeat(64),
            product_modset_sha256: "d".repeat(64),
            effective_vfs_manifest_sha256: "e".repeat(64),
            camera: receipt(XMAP20_CAMERA_OWNER, '1'),
            lighting: receipt(XMAP20_LIGHTING_OWNER, '2'),
            scene_compositor_pre_particles: receipt(
                XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER,
                '3',
            ),
            entity_identity: receipt(XMAP20_ENTITY_IDENTITY_OWNER, '4'),
            weather_identity: receipt(XMAP20_WEATHER_IDENTITY_OWNER, '5'),
            ordered_entity_semantic_source_ids: vec!["division:7".into()],
            ordered_weather_semantic_source_ids: vec![
                "xmap19.weather:r1:e0:c2:p1:primary".into(),
            ],
        }
    }

    fn event(sequence: u64, operation: EffectOperation) -> EffectLifecycleEvent {
        EffectLifecycleEvent {
            sequence,
            update_index: 7,
            effect_instance_id: 9,
            operation,
            registration_token: "mg_muzzle_smoke".into(),
            registration_hash_u32: fnv1a32(b"mg_muzzle_smoke"),
            resolved_definition_sha256: "f".repeat(64),
            producer_kind: "unit".into(),
            producer_instance_id: "division:7".into(),
            authored_event_index: 3,
            authored_effect_index: 0,
            parent_anchor: ParentAnchor::Entity {
                entity_instance_id: 77,
                node_token: "muzzle".into(),
            },
            initial_rng_seed_u32: 11,
            initial_rng_counter_u32: 12,
            elapsed_f32_bits: 1.0f32.to_bits(),
            time_scale_f32_bits: 1.0f32.to_bits(),
            active_flags_u32: 1,
        }
    }

    fn frame(
        ordered_lifecycle_events: Vec<EffectLifecycleEvent>,
        active_effects: Vec<EffectLifecycleEvent>,
    ) -> Xmap20EffectFrame {
        Xmap20EffectFrame {
            schema: XMAP20_EFFECT_FRAME_SCHEMA.into(),
            executable_sha256: XMAP20_EXECUTABLE_SHA256.into(),
            hour_index: 42,
            update_index: 7,
            first_sequence: 1,
            last_sequence: 2,
            content_manifest_sha256: "c".repeat(64),
            product_modset_sha256: "d".repeat(64),
            effective_vfs_manifest_sha256: "e".repeat(64),
            producer_receipt_sha256: "1".repeat(64),
            complete_enumeration_sha256: "2".repeat(64),
            ordered_lifecycle_events,
            active_effects,
            particles: Vec::new(),
            ordered_particle_outputs: Vec::new(),
            lifecycle_sha256: String::new(),
            active_effects_sha256: String::new(),
            ordered_particles_sha256: String::new(),
            deterministic_sha256: String::new(),
        }
        .seal()
        .unwrap()
    }

    #[test]
    fn admits_authenticated_empty_active_effect_without_selected_present() {
        let spawn = event(1, EffectOperation::Spawn);
        let checkpoint = event(2, EffectOperation::Activate);
        let carrier = frame(vec![spawn, checkpoint.clone()], vec![checkpoint]);
        let encoded = carrier.encode_json().unwrap();
        let decoded = Xmap20EffectFrame::decode_json(&encoded).unwrap();
        assert_eq!(decoded, carrier);
        let admission = admit_xmap20_effects(&decoded, &dependencies()).unwrap();
        assert_eq!(admission.active_effects.len(), 1);
        assert!(admission.active_effects[0].ordered_packet_keys.is_empty());
    }

    #[test]
    fn rejects_stable_identity_mutation() {
        let spawn = event(1, EffectOperation::Spawn);
        let mut activate = event(2, EffectOperation::Activate);
        activate.registration_token = "different_effect".into();
        activate.registration_hash_u32 = fnv1a32(activate.registration_token.as_bytes());
        let error = admit_xmap20_effects(
            &frame(vec![spawn, activate.clone()], vec![activate]),
            &dependencies(),
        )
        .unwrap_err();
        assert!(error.contains("stable effect identity changed"));
    }

    #[test]
    fn anchor_changes_require_explicit_reparent_operation() {
        let spawn = event(1, EffectOperation::Spawn);
        let mut activate = event(2, EffectOperation::Activate);
        activate.parent_anchor = ParentAnchor::Entity {
            entity_instance_id: 77,
            node_token: "different_node".into(),
        };
        let error = admit_xmap20_effects(
            &frame(vec![spawn, activate.clone()], vec![activate]),
            &dependencies(),
        )
        .unwrap_err();
        assert!(error.contains("outside an exact reparent"));
    }

    #[test]
    fn rejects_nonfinite_world_anchor() {
        let mut spawn = event(1, EffectOperation::Spawn);
        spawn.parent_anchor = ParentAnchor::World {
            world_transform_4x4_f32_bits: {
                let mut matrix = [0u32; 16];
                matrix[5] = f32::NAN.to_bits();
                matrix
            },
        };
        let error = admit_xmap20_effects(&frame(vec![spawn.clone()], vec![spawn]), &dependencies())
            .unwrap_err();
        assert!(error.contains("non-finite transform"));
    }
}
