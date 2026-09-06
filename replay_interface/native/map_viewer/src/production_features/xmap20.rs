use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    ProductionMapFeatureAdapter, Result,
};
use portable_content_manifest::UsedContentKey;
#[cfg(test)]
use portable_render_stream::ParentAnchor;
use portable_render_stream::{
    FamilyDisposition, MapVisualFamily, Xmap20ActiveEffect, XMAP20_EXECUTABLE_SHA256,
    XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA, XMAP20_PARTICLE_REGISTRY_RVA,
    XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

pub const XMAP20_AUTHORED_FRAME_SCHEMA: &str = "hoi4.xmap20.authored-effects.a729.v1";
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

pub const XMAP20_EFFECT_LIFECYCLE_INPUT: &str = "effects.semantic_lifecycle";

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
        Self {
            lifecycle: crate::xmap09_light_lifecycle::ViewerLightLifecycle::for_authored_particles(
            ),
        }
    }
}

impl Xmap20AuthoredEntityRuntime {
    pub fn prepare(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[crate::xmap09_light_lifecycle::Xmap09PlacedEntity],
        viewer: &crate::ViewerOwnedMapState,
    ) -> std::result::Result<
        Vec<Xmap20AuthoredParticleBinding>,
        crate::xmap09_light_census::Xmap09LightCensusRefusal,
    > {
        Ok(self
            .lifecycle
            .derive_light_instance_rows(resolver, placed, viewer, None)?
            .authored_particles)
    }
}

pub struct Xmap20EffectsAdapter;

impl Xmap20EffectsAdapter {
    pub(crate) fn producer_registration_sha256() -> String {
        Sha256::hex(
            format!(
                "XMAP20\0{XMAP20_AUTHORED_FRAME_SCHEMA}\0{XMAP20_EXECUTABLE_SHA256}\0{XMAP20_PARTICLE_DEFINITION_LOOKUP_RVA:08x}\0{XMAP20_PARTICLE_REGISTRY_RVA:08x}\0{XMAP20_PARTICLE_SUBMIT_BOUNDARY_RVA:08x}"
            )
            .as_bytes(),
        )
    }

    pub(crate) fn deferred_to_draw_output() -> FoundationFeatureOutput {
        FoundationFeatureOutput::without_payload(
            FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: Self::producer_registration_sha256(),
                draw_contract_id: XMAP20_EFFECT_DRAW_CONTRACT.into(),
                reason: "authored particle runtime submission requires the current main-scene resource identity, shader providers and completed pre-particle scene".into(),
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
        if !consumed_gap_ids.is_empty() {
            return Err("XMAP20 viewer-derived lifecycle was incorrectly classified as a captured input gap".into());
        }
        let mut output = Self::deferred_to_draw_output();
        for (branch, detail) in [
            ("unit-event-phase-unbound", "XMAP13 prepared unit animation phase is not yet joined to the retained authored event clock; unit particle instances are unavailable, not measured empty"),
            ("weather-event-phase-unbound", "XMAP19 primary/fading entity epochs, visibility and playback rate are not yet joined to the retained authored event clock; weather particle instances are unavailable, not measured empty"),
        ] {
            if branch == "weather-event-phase-unbound" && context.weather_particles_bound {
                continue;
            }
            output.element_diagnostics.push(portable_render_stream::ElementDiagnostic::family_exception(
                MapVisualFamily::ParticlesEffects, branch, 0, detail.into(),
            ));
        }
        Ok(output)
    }
}

#[cfg(test)]
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
mod authored_admission_tests {
    use super::*;

    #[test]
    fn authored_lifecycle_defers_without_a_captured_particle_carrier() {
        use crate::production_features::xmap15::Xmap15ViewerSelection;
        use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
        struct Source;
        impl ReplaySource for Source {
            fn game_id(&self) -> &str {
                "authored-particles"
            }
            fn hours(&self) -> &[i64] {
                &[11]
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
        let timeline = WorldTimeline::open(Box::new(Source)).unwrap();
        let frame = timeline.frame(11).unwrap();
        assert!(frame.xmap20_effects().is_none());
        let viewer = crate::ViewerOwnedMapState {
            sky_enabled: true,
            viewport_width: 16,
            viewport_height: 16,
            camera: None,
            observer_country_id: None,
            map_mode_id: "test".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 0,
            frame_delta_seconds_f32_bits: 0,
        };
        let admission = crate::map_authority::map_authority_viewer_derived_admission(
            XMAP20_EFFECT_LIFECYCLE_INPUT,
        )
        .unwrap();
        assert_eq!((admission.status)(&frame), replay_core::ViewStatus::Present);
        let admitted = [crate::MapAuthorityAdmittedInput {
            input_id: admission.input_id.into(),
            owner_packet: admission.owner_packet.into(),
            payload_schema: admission.payload_schema.into(),
            cadence: admission.cadence.into(),
            identity_order: admission.identity_order.into(),
            source: crate::MapAuthorityAdmissionSource::ViewerDerivation,
            derivation: Some(admission.derivation.into()),
        }];
        let selection = Xmap15ViewerSelection {
            presentation: None,
            selected_owner: None,
            selected_plan: None,
        };
        let mut context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame,
            &viewer,
            &[],
            &admitted,
            &selection,
        );
        let output = Xmap20EffectsAdapter.prepare(&context).unwrap();
        assert!(
            matches!(output.disposition, FamilyDisposition::DeferredToDraw { ref draw_contract_id, .. } if draw_contract_id == XMAP20_EFFECT_DRAW_CONTRACT)
        );
        assert_eq!(output.element_diagnostics.iter().map(|d| d.cause_id.as_str()).collect::<Vec<_>>(),
            ["unit-event-phase-unbound", "weather-event-phase-unbound"]);
        for diagnostic in output.element_diagnostics {
            diagnostic.validate().unwrap();
        }
        context.weather_particles_bound = true;
        let bound = Xmap20EffectsAdapter.prepare(&context).unwrap();
        assert_eq!(bound.element_diagnostics.iter().map(|d| d.cause_id.as_str()).collect::<Vec<_>>(),
            ["unit-event-phase-unbound"]);
        assert!(
            matches!(bound.disposition, FamilyDisposition::DeferredToDraw { ref draw_contract_id, .. } if draw_contract_id == XMAP20_EFFECT_DRAW_CONTRACT)
        );
        bound.element_diagnostics[0].validate().unwrap();
    }
}
