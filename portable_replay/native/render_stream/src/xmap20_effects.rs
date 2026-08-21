use crate::{
    canonical_json_sha256, check_sha256, validate_constant_buffers,
    validate_runtime_draw_vfs_authority, validate_texture_bindings, EffectOperation, ParentAnchor,
    ParticlePacket, RenderOutputRef, Xmap20DependencyReceipts,
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

pub const XMAP20_PRODUCER_KIND_UNIT: &str = "unit";
pub const XMAP20_PRODUCER_KIND_WEATHER: &str = "weather";

/// The producer kind for an effect whose semantic owner was NOT OBTAINED.
///
/// It is deliberately not spelled "engine". "engine" reads as a positive claim
/// that the engine itself owns the effect, and the recorder cannot support
/// that: nothing readable on the census instance, the scene node, or the
/// particle descriptor distinguishes a unit-owned effect from a weather-owned
/// one, so an effect landing here means only that no join was obtained.
/// Encoding "not obtained" as a positive ownership claim is the truth-state
/// error this name avoids.
///
/// What the validator CAN prove is much narrower than two earlier drafts of
/// this comment claimed, and both drafts were refuted under review. It is not a
/// laundering defence, and it is not namespace separation either. It is a
/// single collision check: the identity is not one that either semantic census,
/// AS CARRIED BY THE PROVIDER RECEIPTS IN HAND, already claims. Nothing follows
/// from that about the namespaces in general — XMAP13 does not constrain the
/// shape of its identities at all — and nothing follows about ownership: a
/// producer that mislabels a genuinely unit-owned effect emits the engine's
/// node token, which no census would have listed, and nothing downstream can
/// detect it. Nor is it proof of no owner, since the XMAP13 entity census that
/// would settle that does not exist. All of these residuals sit in gate
/// XMAP20-G1, and the "did the producer actually try to join" half is a
/// producer obligation, not a carrier one.
///
/// `producer_kind` is a SCHEMA CLASSIFICATION chosen by the producer, not a
/// value read out of the engine. Only `producer_instance_id` — the engine's
/// synthesised attachment token — is engine-held.
pub const XMAP20_PRODUCER_KIND_UNATTRIBUTED: &str = "unattributed";

// A DELETED LAW, recorded because the deletion is the finding.
//
// A previous revision defined XMAP20_SEMANTIC_IDENTITY_RESERVED_SEPARATOR = ':'
// and refused any unattributed identity containing it, on the stated ground
// that "both semantic grammars are colon-delimited". That ground is FALSE.
// XMAP19 identities are colon-delimited, but XMAP13's are not constrained at
// all: its receipt validator requires only that a semantic source id be
// non-blank and unique, and replay_core's `value_to_id_string` renders an
// integer id as a bare decimal string, so "2" is a lawful XMAP13 semantic
// source id. A rule asserting the namespaces are disjoint by construction was
// therefore asserting something untrue, which is worse than asserting nothing.
// It is deleted rather than weakened; what remains is the census-membership
// check in the SceneNode arm of `validate_event`, which is true, and which
// makes no disjointness claim.

/// The only assigned bit of `active_flags_u32`.
///
/// Bit 0 is the engine's enable byte at effect instance+0x54: set by
/// 0x1423e9150, driven from the owning scene node's enable bit (node+0x60 & 1)
/// by 0x14227ecb0, and read back as the enabled/disabled discriminator by the
/// engine's own census statistics walk 0x1423e8500. A live read of build a729
/// observed a node carrying flags byte 0x06 (bit 0 clear) paired with
/// instance+0x54 == 0. That pair is CONSISTENT with the statically proven
/// propagation and rules out the constructor's default, which writes 1 to
/// instance+0x54 — but one sample of a settled pair is not a traced transition,
/// so it does not prove the causal link on its own.
///
/// The second flag byte at instance+0x55 is deliberately NOT assigned a bit.
/// The instance constructor 0x1423e8190 does not initialise it, so between
/// allocation and the first 0x1423e9020 call it holds whatever the allocator
/// left. A producer cannot tell a latched flag from allocator residue, so
/// encoding it would put an unproven value on the wire.
///
/// Every other bit is unassigned and a decoder REFUSES an event that sets one.
pub const XMAP20_ACTIVE_FLAGS_ASSIGNED_MASK: u32 = 0b1;
pub const XMAP20_ACTIVE_FLAG_ENABLED: u32 = 0b1;

fn validate_active_flags(active_flags_u32: u32) -> Result<(), String> {
    if active_flags_u32 & !XMAP20_ACTIVE_FLAGS_ASSIGNED_MASK != 0 {
        return Err("XMAP20 active flags set a bit this schema leaves unassigned".into());
    }
    Ok(())
}

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
    pub authored_effect_index: u32,
    pub parent_anchor: ParentAnchor,
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
        // Every law that does NOT need the provider receipts is enforced HERE,
        // at decode and seal, not only later at admission. A digest only proves
        // a payload is the one its producer sealed; it says nothing about
        // whether the payload is lawful. Without this pass a producer could
        // seal a frame whose events set unassigned flag bits or carry
        // non-finite time, and every decoder short of `admit_xmap20_effects`
        // would accept it — which is exactly the "decoder rejects unassigned
        // bits" law not being enforced by the decoder.
        for event in self
            .ordered_lifecycle_events
            .iter()
            .chain(&self.active_effects)
        {
            validate_event_independent_of_dependencies(event)?;
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
    pub elapsed_f32_bits: u32,
    pub time_scale_f32_bits: u32,
    pub active_flags_u32: u32,
    pub last_operation: EffectOperation,
    #[serde(with = "crate::decimal_u64")]
    pub last_sequence: u64,
    #[serde(with = "crate::decimal_u64")]
    pub last_update_index: u64,
    pub ordered_packet_keys: Vec<[u32; 3]>,
}

impl Xmap20ActiveEffect {
    fn from_checkpoint(
        event: &Xmap20LifecycleEvent,
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
        validate_sha256(&self.complete_enumeration_sha256, "complete enumeration")?;
        // The admission's digests only prove it is the one its producer sealed.
        // Without this loop a correctly resealed admission carrying active
        // effects with unassigned flag bits, non-finite time or a malformed
        // anchor passed THIS public validator and was caught only if someone
        // went on to call `validate_xmap20_admitted_particles`.
        for effect in &self.active_effects {
            validate_active_effect(effect)?;
        }
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

/// Every per-event law that does NOT need the provider receipts.
///
/// Split out so the frame decoder can enforce it. `Xmap20EffectFrame::validate`
/// runs it over both event vectors, so a frame that sets an unassigned flag bit
/// or carries non-finite time is refused at decode and at seal, not merely at
/// admission — the admission path re-runs it through `validate_event`.
fn validate_event_independent_of_dependencies(
    event: &Xmap20LifecycleEvent,
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
    validate_active_flags(event.active_flags_u32)?;
    // The anchor's own shape, independent of any census join.
    match &event.parent_anchor {
        ParentAnchor::Entity {
            entity_instance_id,
            node_token,
        } => {
            if event.producer_kind != XMAP20_PRODUCER_KIND_UNIT
                || *entity_instance_id == 0
                || node_token.trim().is_empty()
            {
                return Err("XMAP20 entity anchor is malformed".into());
            }
        }
        ParentAnchor::World {
            world_transform_4x4_f32_bits,
        } => {
            if event.producer_kind != XMAP20_PRODUCER_KIND_WEATHER
                || !world_transform_4x4_f32_bits
                    .iter()
                    .all(|bits| f32::from_bits(*bits).is_finite())
            {
                return Err("XMAP20 world anchor is malformed or non-finite".into());
            }
        }
        ParentAnchor::SceneNode {
            node_token,
            node_transform_4x4_f32_bits,
        } => {
            if event.producer_kind != XMAP20_PRODUCER_KIND_UNATTRIBUTED
                || node_token.trim().is_empty()
                || node_token != &event.producer_instance_id
                || !node_transform_4x4_f32_bits
                    .iter()
                    .all(|bits| f32::from_bits(*bits).is_finite())
            {
                return Err(
                    "XMAP20 scene-node anchor is non-finite or disagrees with its engine identity"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

fn validate_event(
    event: &Xmap20LifecycleEvent,
    dependencies: &Xmap20DependencyReceipts,
) -> Result<(), String> {
    validate_event_independent_of_dependencies(event)?;
    match &event.parent_anchor {
        ParentAnchor::Entity { .. } => {
            if !dependencies.admits_entity_semantic_source(&event.producer_instance_id) {
                return Err(
                    "XMAP20 entity anchor is not joined to XMAP13 semantic identity".into(),
                );
            }
        }
        ParentAnchor::World { .. } => {
            if !dependencies.admits_weather_semantic_source(&event.producer_instance_id) {
                return Err(
                    "XMAP20 world anchor is non-finite or not joined to XMAP19 identity".into(),
                );
            }
        }
        // The UNATTRIBUTED case: the recorder saw this effect in the engine's
        // particle census and could not join it to any semantic owner.
        //
        // BE PRECISE ABOUT WHAT THIS CHECK IS, because two earlier drafts were
        // not and both were refuted under adversarial review.
        //
        // It is NOT a laundering defence. An unattributed identity IS the
        // engine's synthesised node token, and a producer that simply MISLABELS
        // a real unit effect emits a token no census was ever going to contain.
        // Nothing downstream can detect that.
        //
        // Nor is it namespace SEPARATION. The second draft added a rule
        // refusing a colon in an unattributed identity, claiming both semantic
        // grammars reserve one. XMAP19's does; XMAP13's does not constrain its
        // identities at all, and an integer entity id renders as a bare "2".
        // That rule asserted a disjointness that does not hold and has been
        // deleted.
        //
        // What is left is exactly one true statement: the identity is not one
        // that either semantic census, as carried by THESE provider receipts,
        // already claims. That is a collision check against a known set, not a
        // property of the namespaces, and it can only run where the receipts
        // are in hand - which is here, at admission. `validate_active_effect`
        // deliberately does not repeat it, because an admission revalidated
        // later has no receipts to check against; see that function's note.
        //
        // Whether the producer genuinely attempted a join before falling back
        // here is a PRODUCER obligation the carrier cannot verify. Gate
        // XMAP20-G1 stays open on it and the producer lane must discharge it
        // with a witness of its lookup.
        ParentAnchor::SceneNode { .. } => {
            if dependencies.admits_entity_semantic_source(&event.producer_instance_id)
                || dependencies.admits_weather_semantic_source(&event.producer_instance_id)
            {
                return Err(
                    "XMAP20 unattributed identity is already claimed by a semantic census".into(),
                );
            }
        }
    }
    Ok(())
}

fn validate_identity_unchanged(
    prior: &Xmap20ActiveEffect,
    event: &Xmap20LifecycleEvent,
) -> Result<(), String> {
    if prior.registration_token != event.registration_token
        || prior.registration_hash_u32 != event.registration_hash_u32
        || prior.resolved_definition_sha256 != event.resolved_definition_sha256
        || prior.producer_kind != event.producer_kind
        || prior.producer_instance_id != event.producer_instance_id
        || prior.authored_event_index != event.authored_event_index
        || prior.authored_effect_index != event.authored_effect_index
    {
        return Err("XMAP20 stable effect identity changed during its lifetime".into());
    }
    Ok(())
}

fn validate_frontier_forward(
    prior: &Xmap20ActiveEffect,
    event: &Xmap20LifecycleEvent,
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
            || packet.content_identity.texture_content.payload_sha256 != reference.texture_sha256
            || packet.shader_program.source.payload_sha256 != reference.shader_sha256
            || packet.gpu_instance_records_sha256 != reference.instance_records_sha256
        {
            return Err("XMAP20 prepared packet differs from portable admission".into());
        }
    }
    Ok(())
}

/// The dependency-INDEPENDENT laws for an admitted active effect.
///
/// It deliberately does NOT repeat the semantic-census collision check that the
/// SceneNode arm of `validate_event` performs. That check is a lookup against
/// the provider receipts, and an admission revalidated later does not carry
/// them — it keeps only their digest. Re-running it here is impossible rather
/// than merely omitted, and pretending otherwise would be the third false
/// claim this arm has attracted. The census join is verified once, at
/// `admit_xmap20_effects`, and the admission's `provider_receipts_sha256` is
/// what ties the result to the receipts that were used.
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
    validate_active_flags(effect.active_flags_u32)?;
    match &effect.parent_anchor {
        ParentAnchor::Entity {
            entity_instance_id,
            node_token,
        } if effect.producer_kind == XMAP20_PRODUCER_KIND_UNIT
            && *entity_instance_id != 0
            && !node_token.trim().is_empty() => {}
        ParentAnchor::World {
            world_transform_4x4_f32_bits,
        } if effect.producer_kind == XMAP20_PRODUCER_KIND_WEATHER
            && world_transform_4x4_f32_bits
                .iter()
                .all(|bits| f32::from_bits(*bits).is_finite()) => {}
        ParentAnchor::SceneNode {
            node_token,
            node_transform_4x4_f32_bits,
        } if effect.producer_kind == XMAP20_PRODUCER_KIND_UNATTRIBUTED
            && !node_token.trim().is_empty()
            && node_token == &effect.producer_instance_id
            && node_transform_4x4_f32_bits
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
                "XMAP20 particle packet has a non-VS or unexpected per-draw constant buffer".into(),
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
            scene_compositor_pre_particles: receipt(XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER, '3'),
            entity_identity: receipt(XMAP20_ENTITY_IDENTITY_OWNER, '4'),
            weather_identity: receipt(XMAP20_WEATHER_IDENTITY_OWNER, '5'),
            ordered_entity_semantic_source_ids: vec!["division:7".into()],
            ordered_weather_semantic_source_ids: vec!["xmap19.weather:r1:e0:c2:p1:primary".into()],
        }
    }

    fn event(sequence: u64, operation: EffectOperation) -> Xmap20LifecycleEvent {
        Xmap20LifecycleEvent {
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
            elapsed_f32_bits: 1.0f32.to_bits(),
            time_scale_f32_bits: 1.0f32.to_bits(),
            active_flags_u32: 1,
        }
    }

    fn frame(
        ordered_lifecycle_events: Vec<Xmap20LifecycleEvent>,
        active_effects: Vec<Xmap20LifecycleEvent>,
    ) -> Xmap20EffectFrame {
        unsealed_frame(ordered_lifecycle_events, active_effects)
            .seal()
            .unwrap()
    }

    /// The same frame WITHOUT sealing, so a test can assert that `seal` itself
    /// refuses an unlawful event rather than having the helper panic.
    fn unsealed_frame(
        ordered_lifecycle_events: Vec<Xmap20LifecycleEvent>,
        active_effects: Vec<Xmap20LifecycleEvent>,
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
        spawn.producer_kind = "weather".into();
        spawn.producer_instance_id = "xmap19.weather:r1:e0:c2:p1:primary".into();
        spawn.parent_anchor = ParentAnchor::World {
            world_transform_4x4_f32_bits: {
                let mut matrix = [0u32; 16];
                matrix[5] = f32::NAN.to_bits();
                matrix
            },
        };
        // A non-finite transform is a dependency-independent defect, so the
        // producer is stopped at seal and never reaches the XMAP19 join.
        let error = unsealed_frame(vec![spawn.clone()], vec![spawn.clone()])
            .seal()
            .unwrap_err();
        assert_eq!(error, "XMAP20 world anchor is malformed or non-finite");

        // The join itself is still enforced: a finite world anchor whose
        // identity no XMAP19 census claims is refused at admission.
        let mut unjoined = spawn;
        unjoined.parent_anchor = ParentAnchor::World {
            world_transform_4x4_f32_bits: [0; 16],
        };
        unjoined.producer_instance_id = "xmap19.weather:r9:e9:c2:p9:primary".into();
        let error = admit_xmap20_effects(
            &frame(vec![unjoined.clone()], vec![unjoined]),
            &dependencies(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "XMAP20 world anchor is non-finite or not joined to XMAP19 identity"
        );
    }

    /// The engine-synthesised attachment token a live read of build a729
    /// observed the shape of: descriptor+0x10 held a heap `std::string` of
    /// length 19, which is the fixed 17-byte literal plus a two-digit counter.
    fn unattributed_node_token() -> String {
        "particle_attach_47".into()
    }

    fn unattributed_event(sequence: u64, operation: EffectOperation) -> Xmap20LifecycleEvent {
        let mut event = event(sequence, operation);
        event.producer_kind = XMAP20_PRODUCER_KIND_UNATTRIBUTED.into();
        event.producer_instance_id = unattributed_node_token();
        event.parent_anchor = ParentAnchor::SceneNode {
            node_token: unattributed_node_token(),
            node_transform_4x4_f32_bits: {
                // The live read's own node+0x10 block: identity rotation with a
                // real map translation at the fourth row.
                let mut matrix = [0u32; 16];
                matrix[0] = 1.0f32.to_bits();
                matrix[5] = 1.0f32.to_bits();
                matrix[10] = 1.0f32.to_bits();
                matrix[12] = 3086.3105f32.to_bits();
                matrix[13] = 9.5f32.to_bits();
                matrix[14] = 1356.6702f32.to_bits();
                matrix[15] = 1.0f32.to_bits();
                matrix
            },
        };
        event
    }

    /// The whole point of the scene-node anchor: an effect the recorder read out
    /// of the engine's particle census, whose identity is absent from both
    /// receipt censuses, is admitted on its own engine-held node token.
    ///
    /// Note the claim is about ABSENCE FROM THE CENSUSES, not about ownership.
    /// The carrier cannot establish that no packet owns the effect.
    ///
    /// kills: deleting the `ParentAnchor::SceneNode` arm of `validate_event`
    /// makes this a compile error; making that arm demand an XMAP13 or XMAP19
    /// census entry the way the other two arms do makes it fail, because the
    /// dependency receipts here list neither this token as an entity source nor
    /// as a weather source.
    #[test]
    fn admits_an_unattributed_effect_whose_identity_is_absent_from_both_censuses() {
        let dependencies = dependencies();
        assert!(!dependencies.admits_entity_semantic_source(&unattributed_node_token()));
        assert!(!dependencies.admits_weather_semantic_source(&unattributed_node_token()));
        let spawn = unattributed_event(1, EffectOperation::Spawn);
        let checkpoint = unattributed_event(2, EffectOperation::Activate);
        let carrier = frame(vec![spawn, checkpoint.clone()], vec![checkpoint]);
        let decoded = Xmap20EffectFrame::decode_json(&carrier.encode_json().unwrap()).unwrap();
        let admission = admit_xmap20_effects(&decoded, &dependencies).unwrap();
        assert_eq!(admission.active_effects.len(), 1);
        assert_eq!(
            admission.active_effects[0].producer_kind,
            XMAP20_PRODUCER_KIND_UNATTRIBUTED
        );
    }

    /// The anchor's token and the event's producer identity are the SAME engine
    /// value, so they cannot be allowed to disagree.
    ///
    /// kills: dropping `node_token != &event.producer_instance_id` from the
    /// `SceneNode` arm lets this frame admit.
    #[test]
    fn rejects_an_unattributed_effect_whose_identity_is_not_its_node_token() {
        let mut spawn = unattributed_event(1, EffectOperation::Spawn);
        spawn.producer_instance_id = "particle_attach_48".into();
        let error = unsealed_frame(vec![spawn.clone()], vec![spawn])
            .seal()
            .unwrap_err();
        assert_eq!(
            error,
            "XMAP20 scene-node anchor is non-finite or disagrees with its engine identity"
        );
    }

    /// A scene-node anchor is the "no join was obtained" case by definition, so
    /// it may not also assert a semantic producer kind.
    ///
    /// kills: dropping the producer-kind equality from the `SceneNode`
    /// arm lets a "weather" effect carry a scene-node anchor while skipping the
    /// XMAP19 census check the `World` arm enforces.
    #[test]
    fn rejects_a_scene_node_anchor_that_claims_a_semantic_producer_kind() {
        for kind in ["unit", "weather"] {
            let mut spawn = unattributed_event(1, EffectOperation::Spawn);
            spawn.producer_kind = kind.into();
            let error = unsealed_frame(vec![spawn.clone()], vec![spawn])
                .seal()
                .unwrap_err();
            assert_eq!(
                error,
                "XMAP20 scene-node anchor is non-finite or disagrees with its engine identity",
                "{kind}"
            );
        }
    }

    /// Only bit 0 of `active_flags_u32` is assigned. Every other bit is
    /// unassigned and the decoder refuses it, which is what keeps a later
    /// producer from quietly widening the encoding.
    ///
    /// kills: deleting either `validate_active_flags` call makes the
    /// corresponding half of this test admit.
    #[test]
    fn rejects_active_flags_outside_the_one_assigned_bit() {
        assert_eq!(XMAP20_ACTIVE_FLAGS_ASSIGNED_MASK, XMAP20_ACTIVE_FLAG_ENABLED);
        for flags in [0b10, 0b1 | 0b10, 1 << 31] {
            let mut spawn = unattributed_event(1, EffectOperation::Spawn);
            spawn.active_flags_u32 = flags;

            // The producer cannot even seal it.
            let error = unsealed_frame(vec![spawn.clone()], vec![spawn.clone()])
                .seal()
                .unwrap_err();
            assert_eq!(
                error, "XMAP20 active flags set a bit this schema leaves unassigned",
                "seal {flags:#b}"
            );

            // And a hostile producer that recomputes every digest so the payload
            // is internally consistent STILL cannot get it past the decoder.
            // This is the case that mattered: before the frame validator ran the
            // per-event laws, a correctly re-sealed frame like this decoded
            // cleanly and was only caught much later, at admission.
            let mut tampered = unsealed_frame(vec![spawn.clone()], vec![spawn]);
            tampered.lifecycle_sha256 =
                canonical_json_sha256(&tampered.ordered_lifecycle_events).unwrap();
            tampered.active_effects_sha256 =
                canonical_json_sha256(&tampered.active_effects).unwrap();
            tampered.ordered_particles_sha256 =
                canonical_json_sha256(&tampered.particles).unwrap();
            tampered.deterministic_sha256.clear();
            tampered.deterministic_sha256 = canonical_json_sha256(&tampered).unwrap();
            let bytes = serde_json::to_vec(&tampered).unwrap();
            let error = Xmap20EffectFrame::decode_json(&bytes).unwrap_err();
            assert_eq!(
                error, "XMAP20 active flags set a bit this schema leaves unassigned",
                "decode {flags:#b}"
            );
        }
        // Both assigned states still seal, decode and admit.
        for flags in [0, XMAP20_ACTIVE_FLAG_ENABLED] {
            let mut spawn = unattributed_event(1, EffectOperation::Spawn);
            spawn.active_flags_u32 = flags;
            admit_xmap20_effects(&frame(vec![spawn.clone()], vec![spawn]), &dependencies())
                .unwrap();
        }
    }

    /// A frame whose events are unlawful cannot be sealed, so the whole class of
    /// dependency-independent laws is enforced at the producer's own boundary
    /// and again at every decoder, not only at admission.
    ///
    /// kills: removing the per-event loop from `Xmap20EffectFrame::validate`
    /// makes both halves of this test seal and decode successfully.
    #[test]
    fn the_frame_validator_enforces_every_dependency_independent_event_law() {
        let mut nonfinite = unattributed_event(1, EffectOperation::Spawn);
        nonfinite.time_scale_f32_bits = f32::NAN.to_bits();
        assert_eq!(
            unsealed_frame(vec![nonfinite.clone()], vec![nonfinite])
                .seal()
                .unwrap_err(),
            "XMAP20 lifecycle event has invalid elapsed/time scale"
        );

        let mut mismatched_hash = unattributed_event(1, EffectOperation::Spawn);
        mismatched_hash.registration_hash_u32 ^= 1;
        assert_eq!(
            unsealed_frame(vec![mismatched_hash.clone()], vec![mismatched_hash])
                .seal()
                .unwrap_err(),
            "XMAP20 lifecycle event has incomplete identity"
        );
    }

    /// The ONE true statement the SceneNode arm makes: the identity is not one
    /// the provider receipts' semantic censuses already claim.
    ///
    /// Two stronger readings were tried and both were refuted. It is not a
    /// laundering defence (a mislabelling producer emits an engine node token no
    /// census would list). It is not namespace separation either: a previous
    /// revision refused a colon in an unattributed identity on the ground that
    /// both semantic grammars reserve one, but XMAP13 does not constrain its
    /// identities at all and an integer entity id renders as a bare "2". That
    /// rule was deleted rather than weakened, so this test now checks only the
    /// collision, which is what the code actually enforces.
    ///
    /// kills: deleting the two `admits_*_semantic_source` calls from the
    /// `SceneNode` arm of `validate_event` makes the first half admit.
    #[test]
    fn an_unattributed_identity_may_not_be_one_a_semantic_census_already_claims() {
        let claimed = "division:7";
        let mut dependencies = dependencies();
        assert!(dependencies.admits_entity_semantic_source(claimed));

        let unattributed_with = |identity: &str| {
            let mut spawn = unattributed_event(1, EffectOperation::Spawn);
            spawn.producer_instance_id = identity.into();
            spawn.parent_anchor = ParentAnchor::SceneNode {
                node_token: identity.into(),
                node_transform_4x4_f32_bits: [0; 16],
            };
            spawn
        };

        let spawn = unattributed_with(claimed);
        assert_eq!(
            admit_xmap20_effects(&frame(vec![spawn.clone()], vec![spawn]), &dependencies)
                .unwrap_err(),
            "XMAP20 unattributed identity is already claimed by a semantic census"
        );

        // Once no census claims it, the SAME identity admits. That is the
        // honest shape of this rule: it is a lookup against a known set, not a
        // property of the identity, and an XMAP13 census is free to contain a
        // colonless integer identity that would collide just as readily.
        dependencies.ordered_entity_semantic_source_ids.clear();
        let spawn = unattributed_with(claimed);
        admit_xmap20_effects(&frame(vec![spawn.clone()], vec![spawn]), &dependencies).unwrap();

        // A bare integer identity is lawful for XMAP13, which is why the
        // deleted colon rule could not have separated the namespaces.
        let mut dependencies = dependencies.clone();
        dependencies.ordered_entity_semantic_source_ids = vec!["2".into()];
        let spawn = unattributed_with("2");
        assert_eq!(
            admit_xmap20_effects(&frame(vec![spawn.clone()], vec![spawn]), &dependencies)
                .unwrap_err(),
            "XMAP20 unattributed identity is already claimed by a semantic census"
        );
    }

    /// The admission's own public validator must enforce the per-effect laws,
    /// not merely its digests. Before this, a correctly resealed admission
    /// carrying unassigned flag bits reported success from `validate()` and was
    /// caught only if the caller went on to validate particles.
    ///
    /// kills: removing the `validate_active_effect` loop from
    /// `Xmap20EffectAdmission::validate` makes this admission validate cleanly.
    #[test]
    fn the_admission_validator_enforces_the_per_effect_laws_not_only_its_digests() {
        let spawn = unattributed_event(1, EffectOperation::Spawn);
        let mut admission =
            admit_xmap20_effects(&frame(vec![spawn.clone()], vec![spawn]), &dependencies())
                .unwrap();
        admission.validate().unwrap();

        admission.active_effects[0].active_flags_u32 = 0b10;
        admission.lifecycle_sha256 = canonical_json_sha256(&admission.active_effects).unwrap();
        admission.deterministic_sha256.clear();
        admission.deterministic_sha256 = canonical_json_sha256(&admission).unwrap();
        assert_eq!(
            admission.validate().unwrap_err(),
            "XMAP20 active flags set a bit this schema leaves unassigned"
        );
    }

    /// The owner ruled effect randomness VIEWER-LOCAL, so the carrier records no
    /// RNG state at all. `deny_unknown_fields` is what makes that stick: a
    /// producer that kept emitting the retired fields is refused rather than
    /// silently ignored.
    ///
    /// kills: re-adding either field to `Xmap20LifecycleEvent` makes this parse
    /// succeed.
    #[test]
    fn refuses_a_carrier_that_still_carries_the_retired_rng_fields() {
        let carrier = frame(
            vec![unattributed_event(1, EffectOperation::Spawn)],
            vec![unattributed_event(1, EffectOperation::Spawn)],
        );
        let mut value: serde_json::Value =
            serde_json::from_slice(&carrier.encode_json().unwrap()).unwrap();
        value["ordered_lifecycle_events"][0]["initial_rng_seed_u32"] = serde_json::json!(11);
        let error = Xmap20EffectFrame::decode_value(&value).unwrap_err();
        assert!(
            error.contains("initial_rng_seed_u32"),
            "unexpected error: {error}"
        );
    }
}
