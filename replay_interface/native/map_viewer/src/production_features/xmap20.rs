
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

/// No recorder build emitted the key at all.
pub const XMAP20_CARRIER_ABSENT_GAP_ID: &str = "XMAP20.effect_lifecycle_carrier_absent";

/// The key reached the WorldFrame and XMAP00 still would not admit it - a
/// typed null, an unreadable payload, or a law the census failed.
///
/// A separate id because "carrier absent" is a statement about the RECORDER
/// and this one is a statement about the hour; a reader routed by the id
/// alone must not be sent to build a producer that already exists.
pub const XMAP20_CARRIER_UNUSABLE_GAP_ID: &str =
    "XMAP20.effect_lifecycle_authority_unavailable";

/// The XMAP00 authority input this family owns, and the wire key that feeds it.
///
/// Both are named in the refusal so a reader is told which recording input is
/// missing, not merely that "a carrier" is.
pub const XMAP20_EFFECT_LIFECYCLE_INPUT: &str = "effects.semantic_lifecycle";
pub const XMAP20_EFFECT_LIFECYCLE_WIRE_KEY: &str =
    replay_core::WorldFrame::XMAP20_EFFECT_LIFECYCLE_WIRE_KEY;

/// What the draw path must present before a frame that deferred XMAP20 may seal.
pub const XMAP20_EFFECT_DRAW_CONTRACT: &str = "XMAP20.effect_frame_receipt";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20AuthoredParticleBinding {
    pub instance_id: String,
    pub entity_id: String,
    pub authored_event_index: usize,
    pub particle_token: String,
    pub paired_light: Option<String>,
    pub light_phase: crate::particle_runtime::ParticleLightBinding,
    pub particle_phase: crate::particle_runtime::ParticleLightBinding,
}

pub struct Xmap20AuthoredEntityRuntime {
    lifecycle: crate::xmap09_light_lifecycle::ViewerLightLifecycle,
}

impl Default for Xmap20AuthoredEntityRuntime {
    fn default() -> Self {
        Self { lifecycle: crate::xmap09_light_lifecycle::ViewerLightLifecycle::for_authored_particles() }
    }
}

impl Xmap20AuthoredEntityRuntime {
    pub fn prepare(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[crate::xmap09_light_lifecycle::Xmap09PlacedEntity],
        viewer: &crate::ViewerOwnedMapState,
    ) -> std::result::Result<Vec<Xmap20AuthoredParticleBinding>, crate::xmap09_light_census::Xmap09LightCensusRefusal> {
        Ok(self.lifecycle.derive_light_instance_rows(resolver, placed, viewer)?.authored_particles)
    }
}

/// XMAP20 answers at DRAW time, and only for a recording that carries the
/// effect-lifecycle carrier.
///
/// Nothing about the answer is computable once per hour: an effect packet's
/// operation identity binds the live D3D main-scene resource identity, the
/// pre-particle compositor receipt and XMAP09's completed light/depth execution,
/// none of which exist at foundation time. So the family reports what it CAN
/// decide here — whether the recording admits `effects.semantic_lifecycle` at
/// all — and defers the rest under [`XMAP20_EFFECT_DRAW_CONTRACT`], which
/// `seal_readiness` will not let the frame skip.
pub struct Xmap20EffectsAdapter;

impl Xmap20EffectsAdapter {
    pub(crate) fn producer_registration_sha256() -> String {
        Sha256::hex(
            format!(
                "XMAP20\0{XMAP20_PREPARED_FRAME_SCHEMA}\0{XMAP20_EXECUTABLE_SHA256}\0{XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA:08x}\0{XMAP20_PARTICLE_REGISTRY_RVA:08x}\0{XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA:08x}"
            )
            .as_bytes(),
        )
    }

    /// This recording does not admit the effect-lifecycle carrier for this hour.
    ///
    /// `authority_blocker` is XMAP00's own words for the input, so the refusal
    /// distinguishes "the recorder never emitted the key" from "the key is there
    /// and unusable" without this module restating either.
    pub(crate) fn carrier_absent_output(
        hour: i64,
        disposition: crate::map_authority::MapAuthorityRequirementDisposition,
        authority_blocker: &str,
        consumed_gap_ids: Vec<String>,
    ) -> Result<FoundationFeatureOutput> {
        use crate::map_authority::MapAuthorityRequirementDisposition as Disposition;
        let (gap_id, what) = match disposition {
            Disposition::Absent => (
                XMAP20_CARRIER_ABSENT_GAP_ID,
                "carries no key {key} at all (root scope)",
            ),
            Disposition::Unavailable => (
                XMAP20_CARRIER_UNUSABLE_GAP_ID,
                "carries key {key} (root scope) and XMAP00 will not admit it",
            ),
            // The projection files a row as a gap only when it is Absent or
            // Unavailable, so this is a contradiction in the caller and not a
            // fact about the recording. Minting an Unavailable row that says
            // "is Admitted" would file the defect as an outcome.
            Disposition::Admitted => {
                return Err(format!(
                    "XMAP20 was asked to refuse {XMAP20_EFFECT_LIFECYCLE_INPUT} while the \
                     projection reports it Admitted"
                )
                .into())
            }
        };
        let what = what.replace("{key}", XMAP20_EFFECT_LIFECYCLE_WIRE_KEY);
        let reason = format!(
            "this recording {what}, so XMAP20 authority input \
             {XMAP20_EFFECT_LIFECYCLE_INPUT} is {disposition:?}: {authority_blocker}; the producer \
             is held by {}",
            XMAP20_OPEN_CARRIER_GATES.join(", ")
        );
        let witness_sha256 =
            Sha256::hex(format!("{gap_id}\n{hour}\n{hour}\n{reason}").as_bytes());
        Ok(FoundationFeatureOutput::without_payload(
            FamilyDisposition::Unavailable {
                gap: portable_render_stream::FoundationGap {
                    gap_id: gap_id.into(),
                    first_unavailable_hour: hour,
                    last_unavailable_hour: hour,
                    witness_sha256,
                    reason,
                },
            },
            consumed_gap_ids,
        ))
    }

    pub(crate) fn deferred_to_draw_output() -> FoundationFeatureOutput {
        FoundationFeatureOutput::without_payload(
            FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: Self::producer_registration_sha256(),
                draw_contract_id: XMAP20_EFFECT_DRAW_CONTRACT.into(),
                reason: format!(
                    "the recording admits {XMAP20_EFFECT_LIFECYCLE_INPUT}, but an effect packet's \
                     operation identity binds the live main-scene resource identity, the \
                     pre-particle compositor receipt and XMAP09's completed light/depth \
                     execution, none of which exist at foundation time"
                ),
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
        // Owned inputs come from the XMAP00 owner table, not from a literal
        // here: a family that keeps its own list can silently disagree with the
        // table the global accounting uses.
        let owned = crate::production_foundation::map_authority_inputs_owned_by("XMAP20");
        let consumed_gap_ids = crate::account_owned_map_authority_inputs(
            "XMAP20",
            &owned,
            context.map_authority_gaps,
            context.map_authority_admitted,
        )?;
        if consumed_gap_ids.is_empty() {
            return Ok(Self::deferred_to_draw_output());
        }
        let gap = context
            .map_authority_gaps
            .iter()
            .find(|gap| gap.input_id == XMAP20_EFFECT_LIFECYCLE_INPUT)
            .ok_or("XMAP20 owned-input accounting reported a gap the projection does not hold")?;
        Self::carrier_absent_output(
            context.world_frame.hour_index(),
            gap.disposition,
            &gap.reason,
            consumed_gap_ids,
        )
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
            stored_game_relative_path: "gfx/entities/units.asset".into(),
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

    /// XMAP20's own authority record, read at test time.
    ///
    /// The blocker is checked against THIS, not against the production constant
    /// beside it: a constant compared with itself accepts any pair of edits that
    /// agree, which is exactly how a gate the authority excludes would get into
    /// the refusal.
    fn xmap20_effect_census_authority() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../re_evidence/rendering_laws/XMAP20/effect_census_authority.json");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_slice(&bytes).expect("XMAP20 authority record is JSON")
    }

    #[test]
    fn the_row_follows_the_recording_and_names_the_input_when_it_is_missing() {
        use crate::map_authority::{
            MapAuthorityAdmittedInput, MapAuthorityGap, MapAuthorityRequirementDisposition,
        };
        use crate::production_features::xmap15::Xmap15ViewerSelection;
        use crate::production_foundation::{FoundationFeatureContext, ViewerOwnedMapState};
        use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};

        struct EmptySource;
        impl ReplaySource for EmptySource {
            fn game_id(&self) -> &str {
                "xmap20-carrier-seam"
            }
            fn hours(&self) -> &[i64] {
                &[11, 12]
            }
            fn warnings(&self) -> &[String] {
                &[]
            }
            fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
                Ok(RawFrame {
                    hour_index,
                    date: None,
                    save_version: None,
                    schema_version: None,
                    rest: serde_json::Map::new().into(),
                })
            }
        }

        let timeline = WorldTimeline::open(Box::new(EmptySource)).expect("timeline");
        let viewer = ViewerOwnedMapState {
            sky_enabled: true,
            viewport_width: 16,
            viewport_height: 16,
            camera: None,
            observer_country_id: None,
            map_mode_id: "not_a_mode".to_string(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 0,
            frame_delta_seconds_f32_bits: 0,
        };
        let selection = Xmap15ViewerSelection {
            selected_plan: None,
        };
        // The projection fills a gap's reason from the requirement's own
        // blocker (map_authority.rs, the Absent/Unavailable arm), so the fixture
        // carries the REAL blocker. A stand-in string here would prove the
        // format slot passes something through and nothing about what the wire
        // actually says.
        let authority_blocker =
            crate::map_authority::map_authority_requirement_metadata(XMAP20_EFFECT_LIFECYCLE_INPUT)
                .expect("XMAP20 owns this authority input")
                .blocker;
        let gapped = vec![MapAuthorityGap {
            input_id: XMAP20_EFFECT_LIFECYCLE_INPUT.to_string(),
            disposition: MapAuthorityRequirementDisposition::Absent,
            reason: authority_blocker.to_string(),
        }];
        let admitted = vec![MapAuthorityAdmittedInput {
            input_id: XMAP20_EFFECT_LIFECYCLE_INPUT.to_string(),
            owner_packet: "XMAP20".to_string(),
            payload_schema: "test".to_string(),
            cadence: "hourly".to_string(),
            identity_order: "test".to_string(),
            source: crate::MapAuthorityAdmissionSource::RecordedCarrier,
            derivation: None,
        }];

        // Arm 1 - the recording does NOT admit the input: refuse, and name the
        // input, the wire key, XMAP00's own blocker and every open gate.
        let frame = timeline.frame(11).expect("frame");
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame,
            &viewer,
            &gapped,
            &[],
            &selection,
        );
        let output = Xmap20EffectsAdapter.prepare(&context).expect("row");
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("XMAP20 must refuse while the recording does not admit its input");
        };
        assert_eq!(gap.gap_id, XMAP20_CARRIER_ABSENT_GAP_ID);
        assert!(gap.reason.contains("carries no key"), "{}", gap.reason);
        assert!(!gap.gap_id.contains("adapter_unimplemented"));
        assert!(gap.reason.contains(XMAP20_EFFECT_LIFECYCLE_INPUT), "{}", gap.reason);
        assert!(gap.reason.contains(XMAP20_EFFECT_LIFECYCLE_WIRE_KEY), "{}", gap.reason);
        for gate in XMAP20_OPEN_CARRIER_GATES {
            assert!(gap.reason.contains(gate), "reason omitted {gate}");
        }
        assert_eq!(
            output.consumed_gap_ids,
            vec![XMAP20_EFFECT_LIFECYCLE_INPUT.to_string()]
        );
        assert_eq!(gap.first_unavailable_hour, 11);
        assert_eq!(gap.last_unavailable_hour, 11);

        // The blocker the refusal quotes states the EXACT missing inputs, and no
        // retired one, and the refusal on the wire CARRIES it whole.
        //
        // kills: a refusal that keeps demanding effect randomness - owner ruling
        // 2026-08-22 assigned it to the viewer, XMAP20-G4 is RESOLVED, and the
        // carrier records no RNG state, so a reader routed by this text would be
        // sent to produce something nobody owes; a blocker that names a gate
        // which does NOT hold the carrier shut, which sends a producer to close
        // something the authority excludes (G1 is open and blocks a join, not
        // the carrier); one that names G7 without naming the three fields, which
        // says a gate is open but not what to go and measure; and dropping the
        // blocker from the refusal's own format string, after which the wire
        // says only that four gates exist and none of what they ask for.
        let blocker = authority_blocker;
        assert!(gap.reason.contains(blocker), "{}", gap.reason);

        // The AUTHORITY decides which gates hold the carrier shut and what each
        // asks for. The WHOLE blocker is composed from it and compared EQUAL,
        // not merely searched: a containment test passes any text that keeps the
        // required substrings, including "the producer that would is NOT held
        // by ..." and any extra sentence appended after them.
        let authority = xmap20_effect_census_authority();
        let carrier_gates = &authority["open_carrier_gates"];
        let authoritative_ids = carrier_gates["ids"]
            .as_array()
            .expect("open_carrier_gates.ids is an array")
            .iter()
            .map(|id| id.as_str().expect("gate id is a string").to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            XMAP20_OPEN_CARRIER_GATES.to_vec(),
            authoritative_ids,
            "the production gate constant disagrees with the authority record"
        );
        let clauses = carrier_gates["carrier_blocker_clauses"]
            .as_object()
            .expect("open_carrier_gates.carrier_blocker_clauses is an object");
        let rendered = authoritative_ids
            .iter()
            .map(|id| {
                let clause = clauses[id]
                    .as_str()
                    .unwrap_or_else(|| panic!("the authority record has no clause for {id}"));
                format!("{id} ({clause})")
            })
            .collect::<Vec<_>>();
        let (last, leading) = rendered
            .split_last()
            .expect("the carrier is held by at least one gate");
        let composed = format!(
            "{}{}{}{last}",
            clauses["preamble"]
                .as_str()
                .expect("the authority record carries the blocker preamble"),
            leading.join(", "),
            if leading.is_empty() { "" } else { " and " },
        );
        assert_eq!(blocker, composed);
        // The retired obligation, by every name it goes under, checked on the
        // AUTHORITY's own words so the evidence record cannot reintroduce it
        // either.
        let lowered = composed.to_ascii_lowercase();
        for retired in ["seed", "rng", "random", "prng", "g4"] {
            assert!(!lowered.contains(retired), "the blocker demands {retired:?}");
        }

        // The witness must bind the hour, or two hours would share one receipt.
        let later = timeline.frame(12).expect("frame");
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &later,
            &viewer,
            &gapped,
            &[],
            &selection,
        );
        let other = Xmap20EffectsAdapter.prepare(&context).expect("row");
        let FamilyDisposition::Unavailable { gap: other_gap } = &other.disposition else {
            unreachable!()
        };
        assert_ne!(gap.witness_sha256, other_gap.witness_sha256);

        // Arm 1b - the key IS on the wire and XMAP00 still will not admit it.
        // That is a different defect class from "no recorder emits the key", and
        // a reader routed by the gap id alone must not be sent to build a
        // producer that already exists (Codex review, round 1).
        let unusable = vec![MapAuthorityGap {
            input_id: XMAP20_EFFECT_LIFECYCLE_INPUT.to_string(),
            disposition: MapAuthorityRequirementDisposition::Unavailable,
            reason: "the hour's payload failed the census law".to_string(),
        }];
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame,
            &viewer,
            &unusable,
            &[],
            &selection,
        );
        let output = Xmap20EffectsAdapter.prepare(&context).expect("row");
        let FamilyDisposition::Unavailable { gap } = &output.disposition else {
            panic!("an unusable carrier must still refuse");
        };
        assert_eq!(gap.gap_id, XMAP20_CARRIER_UNUSABLE_GAP_ID);
        assert_ne!(gap.gap_id, XMAP20_CARRIER_ABSENT_GAP_ID);
        assert!(gap.reason.contains("will not admit it"), "{}", gap.reason);
        assert!(
            gap.reason.contains("the hour's payload failed the census law"),
            "{}",
            gap.reason
        );

        // ...and an Admitted input is a contradiction in the caller, not a row.
        let contradiction = match Xmap20EffectsAdapter::carrier_absent_output(
            11,
            MapAuthorityRequirementDisposition::Admitted,
            "unreachable",
            Vec::new(),
        ) {
            Ok(_) => panic!("refusing an admitted input must abort"),
            Err(error) => error.to_string(),
        };
        assert!(contradiction.contains("reports it Admitted"), "{contradiction}");

        // Arm 2 - the recording DOES admit the input: the family stops refusing
        // and defers under its draw contract. This is the arm the old
        // constructor-shaped adapter could never reach.
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame,
            &viewer,
            &[],
            &admitted,
            &selection,
        );
        let output = Xmap20EffectsAdapter.prepare(&context).expect("row");
        let FamilyDisposition::DeferredToDraw {
            draw_contract_id,
            reason,
            producer_registration_sha256,
        } = &output.disposition
        else {
            panic!("an admitted carrier must defer, not refuse: {:?}", output.disposition);
        };
        assert_eq!(draw_contract_id, XMAP20_EFFECT_DRAW_CONTRACT);
        assert_eq!(
            producer_registration_sha256,
            &Xmap20EffectsAdapter::producer_registration_sha256()
        );
        assert!(reason.contains("main-scene resource identity"), "{reason}");
        assert!(output.consumed_gap_ids.is_empty());

        // Arm 3 - a row that is both admitted and gapped is a contradiction the
        // adapter must abort on rather than pick a side.
        let mut both = gapped.clone();
        both.push(MapAuthorityGap {
            input_id: XMAP20_EFFECT_LIFECYCLE_INPUT.to_string(),
            disposition: MapAuthorityRequirementDisposition::Unavailable,
            reason: "second opinion".to_string(),
        });
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame,
            &viewer,
            &both,
            &admitted,
            &selection,
        );
        let error = match Xmap20EffectsAdapter.prepare(&context) {
            Ok(_) => panic!("a double-booked input must abort"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("both admitted and gapped"), "{error}");
    }
}
