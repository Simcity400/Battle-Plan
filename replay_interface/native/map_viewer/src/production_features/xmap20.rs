
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
            // An unattributed scene-node effect has no owner packet to ASK,
            // because its producer reported obtaining no join to XMAP13 or
            // XMAP19 — a statement about the lookup, not about who owns it. A receipt still needs a
            // resolved `source_content` key, and the only path to one is the
            // effect's own particle packets, which this per-effect signature
            // does not receive. Refusing names that gap instead of minting a
            // receipt over invented content.
            ParentAnchor::SceneNode { .. } => Err(XMAP20_UNATTRIBUTED_TRIGGER_GAP.into()),
        }
    }
}

/// Why an unattributed scene-node effect cannot yet be given a trigger receipt.
///
/// This is the one open item the scene-node anchor leaves for the producer
/// lane: the receipt's `source_content` must be a resolved `UsedContentKey`,
/// and an effect whose producer obtained no semantic join can only get one from its own
/// particle packets. `Xmap20TriggerProvider::trigger_for_effect` takes an
/// effect without its packets, so closing this is a producer-side signature
/// decision, not a further reverse-engineering task.
pub const XMAP20_UNATTRIBUTED_TRIGGER_GAP: &str =
    "XMAP20 unattributed scene-node effects have no authored-trigger provider: their producer \
     reported obtaining no semantic join, so there is no owner packet to ask for a receipt, and \
     their resolved content key must come from their own particle packets, which the per-effect \
     trigger signature does not carry";

/// The gates that stop an effect-lifecycle carrier from existing at all.
///
/// Lane OI proved the engine read path, PJ mapped the owner chain, and PO
/// closed the live gate and amended the schema. Three of the original five are
/// gone and one is new, so what remains is genuinely producer-side work:
///
/// * `G1-xmap13-join` is ANSWERED, not closed by building XMAP13. The census
///   cannot name a semantic owner, and PJ's weather-side shortcut does not open
///   either, so the carrier records the engine's own scene-node identity
///   instead (`ParentAnchor::SceneNode` with `producer_kind` `"unattributed"`)
///   and no longer needs the entity census to exist. That name is deliberate:
///   it records that no join was OBTAINED, and does not assert the engine owns
///   the effect. The carrier cannot verify that a producer genuinely attempted
///   the join before falling back here, so G1 stays open on that obligation.
/// * `G3-live-confirmation` is CLOSED: one bounded read against a matching
///   running build walked manager to census to instance to node to descriptor
///   to authored type, and every self-check passed.
/// * `G4-initial-versus-current-prng-state` is RESOLVED by the owner's ruling
///   that effect randomness is viewer-local. The carrier records no RNG state,
///   so there is nothing left to satisfy.
///
/// The four below are what a producer still has to answer. G7 in particular is
/// why "the carrier is no longer blocked" would be the wrong reading of the
/// amendment: three required wire fields are still unsourced, and the validator
/// accepts a zero for each, so a producer landing today would invent them. See
/// `replay_interface/re_evidence/rendering_laws/XMAP20/effect_census_authority.json`.
pub const XMAP20_OPEN_CARRIER_GATES: [&str; 4] = [
    "XMAP20-G2-lifecycle-sequencing",
    "XMAP20-G5-census-completeness-and-snapshot-safety",
    "XMAP20-G6-unattributed-trigger-receipt",
    "XMAP20-G7-unsourced-required-wire-fields",
];

pub const XMAP20_CARRIER_ABSENT_GAP_ID: &str = "XMAP20.effect_lifecycle_carrier_absent";

/// Registration sites build their adapter set once, before any frame exists,
/// so this adapter cannot demand an already-prepared frame at construction.
/// `CarrierAbsent` is what production registers today: it makes the family
/// report its real blocker to the readiness graph instead of being invisible
/// to it.
pub enum Xmap20EffectsAdapter {
    Prepared(Box<PreparedXmap20Frame>),
    CarrierAbsent,
}

impl Xmap20EffectsAdapter {
    pub fn new(prepared: PreparedXmap20Frame) -> Result<Self> {
        prepared.validate()?;
        Ok(Self::Prepared(Box::new(prepared)))
    }

    /// No effect-lifecycle carrier exists on the wire, so the family refuses
    /// and says exactly which gates hold it shut.
    pub fn carrier_absent() -> Self {
        Self::CarrierAbsent
    }

    pub(crate) fn carrier_absent_output(hour: i64) -> FoundationFeatureOutput {
        let reason = format!(
            "no effect-lifecycle carrier exists on the wire: the XMAP20 producer is held by {}",
            XMAP20_OPEN_CARRIER_GATES.join(", ")
        );
        let witness_sha256 = Sha256::hex(
            format!("{XMAP20_CARRIER_ABSENT_GAP_ID}\n{hour}\n{hour}\n{reason}").as_bytes(),
        );
        FoundationFeatureOutput::without_payload(
            FamilyDisposition::Unavailable {
                gap: portable_render_stream::FoundationGap {
                    gap_id: XMAP20_CARRIER_ABSENT_GAP_ID.into(),
                    first_unavailable_hour: hour,
                    last_unavailable_hour: hour,
                    witness_sha256,
                    reason,
                },
            },
            Vec::new(),
        )
    }
}

impl ProductionMapFeatureAdapter for Xmap20EffectsAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::ParticlesEffects
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let Self::Prepared(prepared) = self else {
            return Ok(Self::carrier_absent_output(
                context.world_frame.hour_index(),
            ));
        };
        let prepared = prepared.as_ref();
        prepared.validate()?;
        // Derivation join: every prepared packet must join its authoritative
        // definition through the recorded census laws (identity equality,
        // FNV-1a-32 registration hash, definition digest, one setup per
        // definition identity, single gfx/fx/particle.shader program path).
        // The setup fields stay KEEP inside the authoritative carrier; this
        // gate requires every executed packet to present the lawful census
        // join before its content may run.
        let derivation_authority = super::xmap20_derive::Xmap20DefinitionAuthority::from_census(
            &prepared.admission.active_effects,
            &prepared.ordered_particles,
        )?;
        for packet in &prepared.ordered_particles {
            derivation_authority.derive_packet_draw_setup(packet)?;
        }
        if context.world_frame.hour_index() != prepared.admission.hour_index {
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
        let disposition = if prepared.admission.active_effects.is_empty()
            && prepared.ordered_particles.is_empty()
        {
            FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256,
                enumerator_witness_sha256: prepared
                    .admission
                    .complete_enumeration_sha256
                    .clone(),
            }
        } else {
            let program_sha256 = Sha256::hex(&serde_json::to_vec(
                &prepared
                    .ordered_particles
                    .iter()
                    .map(|packet| &packet.shader_program)
                    .collect::<Vec<_>>(),
            )?);
            let ordered_resources_sha256 = Sha256::hex(&serde_json::to_vec(&(
                &prepared.trigger_receipts,
                &prepared.ordered_used_content,
                &prepared.admission.ordered_particle_submissions,
                &prepared.ordered_particles,
            ))?);
            let operation = assigned.foundation_operation_identity(
                program_sha256,
                ordered_resources_sha256,
                prepared.admission.provider_receipts_sha256.clone(),
            )?;
            FamilyDisposition::Present {
                payload_schema: XMAP20_PREPARED_FRAME_SCHEMA.into(),
                producer_registration_sha256,
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![operation],
                ordered_used_content: prepared.ordered_used_content.clone(),
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
            prepared_effects: Some(prepared.clone()),
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            prepared_map_furniture: None,
            element_diagnostics: Vec::new(),
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
    // Derivation join on the PRODUCT preparation route: every packet that will
    // be executed must join its authoritative definition through the recorded
    // census laws (identity equality, FNV-1a-32 registration hash, definition
    // digest, one setup per definition identity, single gfx/fx/particle.shader
    // program path). Execution consumes these packets directly; the setup
    // fields stay KEEP inside the authoritative carrier, and this gate requires
    // the lawful census join before the packets may execute.
    let derivation_authority = super::xmap20_derive::Xmap20DefinitionAuthority::from_census(
        &prepared.admission.active_effects,
        &prepared.ordered_particles,
    )?;
    for packet in &prepared.ordered_particles {
        derivation_authority.derive_packet_draw_setup(packet)?;
    }
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
        // The unattributed scene-node case. The trigger is XMAP20's own: it names no
        // semantic owner packet other than XMAP20, claims no entity, and its
        // node token must be the same engine-synthesised token the anchor
        // carries, so the receipt cannot drift from the anchor it describes.
        ParentAnchor::SceneNode { node_token, .. } => {
            if effect.producer_kind != portable_render_stream::XMAP20_PRODUCER_KIND_UNATTRIBUTED
                || trigger.semantic_owner_packet != "XMAP20"
                || trigger.entity_instance_id.is_some()
                || trigger.node_token.as_deref() != Some(node_token.as_str())
            {
                return Err(
                    "XMAP20 scene-node trigger must be unattributed and carry its exact node token"
                        .into(),
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

#[cfg(test)]
mod carrier_tests {
    use super::*;

    // kills: reporting XMAP20 as a generic unpromoted adapter instead of naming
    // the gates that actually hold the producer shut, and silently dropping a
    // gate from the list as gates are closed. The count is deliberately not
    // written here — it has changed once already (five to four) and a number in
    // a comment is a second place for it to go stale.
    #[test]
    fn the_refusal_names_every_open_carrier_gate() {
        let output = Xmap20EffectsAdapter::carrier_absent_output(11);
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("XMAP20 must refuse while no carrier exists");
        };
        assert_eq!(gap.gap_id, XMAP20_CARRIER_ABSENT_GAP_ID);
        assert!(!gap.gap_id.contains("adapter_unimplemented"));
        for gate in XMAP20_OPEN_CARRIER_GATES {
            assert!(gap.reason.contains(gate), "reason omitted {gate}");
        }
        assert!(output.prepared_effects.is_none());
        assert!(output.consumed_gap_ids.is_empty());
        let other = Xmap20EffectsAdapter::carrier_absent_output(12);
        let FamilyDisposition::Unavailable { gap: other_gap } = &other.disposition else {
            unreachable!()
        };
        assert_ne!(gap.witness_sha256, other_gap.witness_sha256);
    }
}
