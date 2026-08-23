//! Viewer-side derivation joining runtime particle draws to the recorded
//! authoritative effect state (ledger rows `carrier.render_submit.particles`
//! and `carrier.render_submit.entities`, XMAP20 re-judge class DERIVE_missing).
//!
//! The defect class: the runtime draw path (`production_map_renderer`
//! `ordered_draws`) resolves `RenderOutputRef`s straight back into the carried
//! `submit.particles` / `submit.entities` products and clones them verbatim -
//! the sealed carrier doubles as its own authority, while the XMAP20 family
//! simultaneously registers `CarrierAbsent` because no effect-lifecycle carrier
//! exists on the wire. This module breaks that product-authority at the grain
//! the rows name: given the admitted hourly [`Xmap20EffectFrame`] (the recorded
//! authoritative effect census), every packet that will be executed must JOIN
//! the census through its identity laws, and per-submit copies become derivable
//! from (retireable in favour of) the authoritative hour instead of trusted.
//!
//! Honest scope of the word "derive" here: the hourly carrier is the authority;
//! there is NO independent viewer-side source for definition-derived setup
//! fields (content parsing of particle definitions does not exist; gate G6
//! records the same fact for unattributed effects). So the SETUP FIELDS remain
//! KEEP inside the authoritative carrier; what this module derives is the
//! packet-to-authority join - which packet setups are lawful for this hour and
//! which carried copies are redundant against them.
//!
//! What stays carried, with the gate or source named:
//! * per-frame RUNTIME state (`canonical_packet_112_bytes`,
//!   `packet_matrix_4x4_f32_bits`, `ordered_gpu_instance_records`, `live_count`,
//!   `elapsed_f32_bits`, `scale_f32_bits`, `flags_u16`, plus two packet fields
//!   that are PER-INSTANCE by engine law: `ordered_constant_buffers` (exact
//!   a729 particle.shader binds CB2 to Scale and CB3 to InstanceWorldMatrix +
//!   vLocalTime) and the geometry's instanced draw count (the promotion law
//!   requires it to equal live_count), so two simultaneous instances of one
//!   definition lawfully differ there) - engine-runtime-only, never
//!   content-derived;
//! * definition-static draw-setup fields per SUBSYSTEM (shader program, texture
//!   bindings, pipeline, output state, atlas dims) INSIDE the
//!   authoritative carrier - no independent source exists yet;
//! * the whole entity side: no entity-lifecycle authority exists anywhere on
//!   any wire (the XMAP13 entity census was never built), so entity submissions
//!   keep their carrier;
//! * the lifecycle fields `authored_event_index`, `authored_effect_index`,
//!   `elapsed_f32_bits` are UNSOURCED under gate XMAP20-G7 (see
//!   `replay_interface/re_evidence/rendering_laws/XMAP20/effect_census_authority.json`),
//!   alongside G2/G5/G6 - this module does not re-litigate those gates, it
//!   joins what the census proves sourced and names the rest.

use crate::Result;
use portable_render_stream::{
    ConstantBufferBinding, DrawCommand, GeometryBinding, ParticlePacket, PipelineDesc,
    RenderOutputState, ShaderProgram,
    TextureBinding, Xmap20EffectFrame,
};

/// The one lawful particle program path (engine constant already pinned by the
/// XMAP20 frame validator's `PARTICLE_SHADER_PATH` law).
pub const XMAP20_PARTICLE_SHADER_PATH: &str = "gfx/fx/particle.shader";

/// The definition-derived draw setup, derived from the authoritative frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedParticleDrawSetup<'a> {
    pub shader_program: &'a ShaderProgram,
    pub ordered_texture_bindings: &'a [TextureBinding],
    pub pipeline: &'a PipelineDesc,
    pub geometry: &'a GeometryBinding,
    pub ordered_constant_buffers: &'a [ConstantBufferBinding],
    pub output_state: &'a RenderOutputState,
    pub atlas_rows: u32,
    pub atlas_columns: u32,
}

/// Index over the recorded authoritative effect census.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Xmap20DefinitionAuthority {
    definitions:
        std::collections::BTreeMap<(String, String, u32, u32), DefinitionEntry>,
    effects: std::collections::BTreeMap<u64, EffectIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DefinitionEntry {
    shader_program: ShaderProgram,
    ordered_texture_bindings: Vec<TextureBinding>,
    pipeline: PipelineDesc,
    ordered_constant_buffers: Vec<ConstantBufferBinding>,
    geometry: GeometryBinding,
    output_state: RenderOutputState,
    atlas_rows: u32,
    atlas_columns: u32,
}

/// The census-proven identity of one live effect, as the frame records it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EffectIdentity {
    registration_token: String,
    registration_hash_u32: u32,
    resolved_definition_sha256: String,
}

/// The census identity fields shared by the frame's lifecycle events and the
/// admission's active-effect records.
pub trait CensusIdentity {
    fn census_identity(&self) -> (u64, &str, u32, &str);
}

impl CensusIdentity for portable_render_stream::Xmap20LifecycleEvent {
    fn census_identity(&self) -> (u64, &str, u32, &str) {
        (
            self.effect_instance_id,
            &self.registration_token,
            self.registration_hash_u32,
            &self.resolved_definition_sha256,
        )
    }
}

impl CensusIdentity for portable_render_stream::Xmap20ActiveEffect {
    fn census_identity(&self) -> (u64, &str, u32, &str) {
        (
            self.effect_instance_id,
            &self.registration_token,
            self.registration_hash_u32,
            &self.resolved_definition_sha256,
        )
    }
}


/// The subsystem-grained definition identity: registration token, definition
/// digest, render bucket, and packet ordinal. `packet_ordinal` IS the engine's
/// subsystem selector (`validate_packet` law: packet_ordinal ==
/// content_identity.subsystem_selector_u32), so distinct subsystems of one
/// definition are distinct lawful setups and must not be merged.
fn definition_key(packet: &ParticlePacket) -> (String, String, u32, u32) {
    (
        packet.registration_token.clone(),
        packet.resolved_definition_sha256.clone(),
        packet.bucket,
        packet.packet_ordinal,
    )
}


/// The DEFINITION-STATIC equality between two stored setups: shader program,
/// texture bindings, pipeline, output state and atlas dims compare fully;
/// constant buffers compare by LAYOUT and geometry by mesh resources plus
/// draw-command shape (per-instance CB bytes and instance counts excluded by
/// the engine laws cited on the helpers below).
fn static_setup_eq(a: &DefinitionEntry, b: &DefinitionEntry) -> bool {
    a.shader_program == b.shader_program
        && a.ordered_texture_bindings == b.ordered_texture_bindings
        && a.pipeline == b.pipeline
        && a.output_state == b.output_state
        && a.atlas_rows == b.atlas_rows
        && a.atlas_columns == b.atlas_columns
        && constant_buffer_layouts_match(
            &a.ordered_constant_buffers,
            &b.ordered_constant_buffers,
        )
        && geometry_static_eq(&a.geometry, &b.geometry)
}

/// Constant-buffer LAYOUT law: binding descriptors (stage/slot/range/semantic
/// layout) are definition-static; raw_bytes/sha256 carry per-instance values
/// (CB2 Scale, CB3 InstanceWorldMatrix + vLocalTime) and must NOT gate the join.
fn constant_buffer_layouts_match(packet: &[ConstantBufferBinding], authoritative: &[ConstantBufferBinding]) -> bool {
    let layout = |b: &ConstantBufferBinding| {
        (b.stage_mask_u8, b.slot, b.byte_offset, b.byte_count, b.semantic_layout.clone())
    };
    packet.iter().map(layout).eq(authoritative.iter().map(layout))
}

/// Geometry LAYOUT law: mesh resources (vertex/index/stream-out bindings) are
/// definition-static; the instanced DRAW COUNT inside the draw command is
/// per-instance (the promotion law requires instance_count == live_count), so
/// two lawful instances of one subsystem differ exactly there.
fn geometry_static_eq(packet: &GeometryBinding, authoritative: &GeometryBinding) -> bool {
    packet.ordered_vertex_buffers == authoritative.ordered_vertex_buffers
        && packet.index_buffer == authoritative.index_buffer
        && packet.stream_output_targets == authoritative.stream_output_targets
        && draw_command_static_eq(&packet.draw, &authoritative.draw)
}

/// Draw commands equal modulo per-instance counts.
fn draw_command_static_eq(packet: &DrawCommand, authoritative: &DrawCommand) -> bool {
    use DrawCommand as D;
    match (packet, authoritative) {
        (
            D::Draw {
                vertex_count: pv,
                start_vertex: ps,
            },
            D::Draw {
                vertex_count: av,
                start_vertex: avs,
            },
        ) => pv == av && ps == avs,
        (
            D::DrawIndexed {
                index_count: pi,
                start_index: psi,
                base_vertex: pb,
            },
            D::DrawIndexed {
                index_count: ai,
                start_index: asi,
                base_vertex: ab,
            },
        ) => pi == ai && psi == asi && pb == ab,
        (
            D::DrawInstanced {
                vertex_count_per_instance: pv,
                start_vertex: ps,
                start_instance: pi,
                ..
            },
            D::DrawInstanced {
                vertex_count_per_instance: av,
                start_vertex: avs,
                start_instance: ai,
                ..
            },
        ) => pv == av && ps == avs && pi == ai,
        (
            D::DrawIndexedInstanced {
                index_count_per_instance: pi,
                start_index: psi,
                base_vertex: pb,
                start_instance: pai,
                ..
            },
            D::DrawIndexedInstanced {
                index_count_per_instance: ai,
                start_index: asi,
                base_vertex: ab,
                start_instance: aai,
                ..
            },
        ) => pi == ai && psi == asi && pb == ab && pai == aai,
        _ => false,
    }
}

impl Xmap20DefinitionAuthority {
    /// Build the authority from the admitted hourly effect frame.
    ///
    /// Every packet the frame carries contributes its definition-derived setup
    /// exactly once per subsystem-grained definition key
    /// `(registration_token, resolved_definition_sha256, bucket, packet_ordinal)`;
    /// conflicting setups for one identity refuse rather than pick a winner.
    #[allow(dead_code)]
    pub fn from_frame(frame: &Xmap20EffectFrame) -> Result<Self> {
        frame
            .validate()
            .map_err(|error| format!("XMAP20 derivation requires a valid effect frame: {error}"))?;
        Self::from_census(&frame.active_effects, &frame.particles)
    }

    /// Same construction over a prepared admission's census parts (the shape
    /// the XMAP20 family adapter holds at prepare time).
    pub fn from_census<E: CensusIdentity>(
        active_effects: &[E],
        particles: &[ParticlePacket],
    ) -> Result<Self> {
        let mut definitions = std::collections::BTreeMap::new();
        let mut effects = std::collections::BTreeMap::new();
        for effect in active_effects {
            let (effect_instance_id, registration_token, registration_hash_u32, digest) =
                effect.census_identity();
            effects.insert(
                effect_instance_id,
                EffectIdentity {
                    registration_token: registration_token.to_string(),
                    registration_hash_u32,
                    resolved_definition_sha256: digest.to_string(),
                },
            );
        }
        for packet in particles {
            let entry = DefinitionEntry {
                shader_program: packet.shader_program.clone(),
                ordered_texture_bindings: packet.ordered_texture_bindings.clone(),
                pipeline: packet.pipeline.clone(),
                ordered_constant_buffers: packet.ordered_constant_buffers.clone(),
                geometry: packet.geometry.clone(),
                output_state: packet.output_state.clone(),
                atlas_rows: packet.atlas_rows,
                atlas_columns: packet.atlas_columns,
            };
            let key = definition_key(packet);
            if let Some(prior) = definitions.insert(key.clone(), entry) {
                let inserted = &definitions[&key];
                if !static_setup_eq(&prior, inserted) {
                    return Err(format!(
                        "XMAP20 definition {} (bucket {}, subsystem ordinal {}) carries two different draw setups",
                        key.0, key.2, key.3
                    )
                    .into());
                }
            }
        }
        Ok(Self {
            definitions,
            effects,
        })
    }

    #[allow(dead_code)]
    pub fn admits_definition(
        &self,
        registration_token: &str,
        digest: &str,
        bucket: u32,
        packet_ordinal: u32,
    ) -> bool {
        self.definitions.contains_key(&(
            registration_token.to_string(),
            digest.to_string(),
            bucket,
            packet_ordinal,
        ))
    }

    /// Join a carried packet to its authoritative definition and return that
    /// definition's draw setup.
    ///
    /// The packet is NOT the authority: it must first satisfy the census laws
    /// against the frame - registration-token equality with its lifecycle
    /// record, the FNV-1a-32 registration-hash law, and the resolved-definition
    /// digest. Its own definition-derived setup fields are then REQUIRED to
    /// equal the authoritative definition's (a packet whose setup disagrees
    /// with its recorded definition is refused, never silently substituted).
    /// The setup fields themselves remain KEEP inside the authoritative
    /// carrier; this method derives the lawful JOIN, not an independent source.
    pub fn derive_packet_draw_setup<'a>(
        &'a self,
        packet: &'a ParticlePacket,
    ) -> Result<DerivedParticleDrawSetup<'a>> {
        self.join_identity(
            packet.effect_instance_id,
            &packet.registration_token,
            &packet.resolved_definition_sha256,
        )?;
        let key = definition_key(packet);
        let entry = self.definitions.get(&key).ok_or_else(|| {
            format!(
                "carried particle packet {} is absent from the authoritative definition set",
                key.0
            )
        })?;
        if packet.shader_program != entry.shader_program
            || packet.ordered_texture_bindings != entry.ordered_texture_bindings
            || packet.pipeline != entry.pipeline
            || packet.output_state != entry.output_state
            || packet.atlas_rows != entry.atlas_rows
            || packet.atlas_columns != entry.atlas_columns
            || !constant_buffer_layouts_match(&packet.ordered_constant_buffers, &entry.ordered_constant_buffers)
            || !geometry_static_eq(&packet.geometry, &entry.geometry)
        {
            return Err(format!(
                "carried particle packet {} carries a draw setup that disagrees with its \
                 recorded authoritative definition; refusing rather than trusting either copy",
                key.0
            )
            .into());
        }
        lawful_program_source_path(&entry.shader_program.source)?;
        Ok(DerivedParticleDrawSetup {
            shader_program: &entry.shader_program,
            ordered_texture_bindings: &entry.ordered_texture_bindings,
            pipeline: &entry.pipeline,
            geometry: &packet.geometry,
            ordered_constant_buffers: &packet.ordered_constant_buffers,
            output_state: &entry.output_state,
            atlas_rows: entry.atlas_rows,
            atlas_columns: entry.atlas_columns,
        })
    }

    /// The identity half of the join: census membership, token/digest equality
    /// and the FNV-1a-32 registration-hash law.
    fn join_identity(
        &self,
        packet_effect_id: u64,
        packet_token: &str,
        packet_digest: &str,
    ) -> Result<&EffectIdentity> {
        let effect = self.effects.get(&packet_effect_id).ok_or_else(|| {
            format!(
                "carried particle packet {packet_token} names an effect instance absent from \
                 the authoritative census"
            )
        })?;
        if packet_token != effect.registration_token || packet_digest != effect.resolved_definition_sha256
        {
            return Err(format!(
                "carried particle packet identity disagrees with its authoritative lifecycle \
                 record for effect {packet_effect_id}"
            ).into());
        }
        if !registration_hash_lawful(packet_token, effect.registration_hash_u32) {
            return Err(format!(
                "carried particle packet {packet_token} fails the FNV-1a-32 registration-hash law"
            ).into());
        }
        Ok(effect)
    }
}

/// FNV-1a-32 registration-hash law: the recorded hash must equal
/// FNV-1a-32(token) (census-authority JSON: type+0x80 == FNV-1a-32 of the
/// token, live-confirmed on hoi4.exe 1.19.2.0.a729).
fn registration_hash_lawful(token: &str, recorded_hash_u32: u32) -> bool {
    fnv1a32(token.as_bytes()) == recorded_hash_u32
}

/// Single-particle-program-path law: every lawful particle program sources
/// `gfx/fx/particle.shader` (the XMAP20 frame validator's PARTICLE_SHADER_PATH).
fn lawful_program_source_path(source: &portable_content_manifest::UsedContentKey) -> Result<()> {
    let path = source.normalized_game_relative_path.to_ascii_lowercase();
    if path != XMAP20_PARTICLE_SHADER_PATH {
        return Err(format!(
            "authoritative particle program path is {path}, not {XMAP20_PARTICLE_SHADER_PATH}"
        ).into());
    }
    Ok(())
}

/// FNV-1a-32, the engine's own registration-hash law (census-authority JSON,
/// descriptor/type registration: type+0x80 == FNV-1a-32 of the token).
fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Why an entity submission cannot yet be derived (KEEP, gates named).
///
/// There is NO entity-side authority anywhere on any wire: the entity-lifecycle
/// census (XMAP13) was never built, and the effect-lifecycle producer that would
/// carry entity semantics is held open by the four XMAP20 carrier gates. Until
/// an owner-proven entity authority exists, entity submissions KEEP their
/// sealed-carrier transport and nothing may claim to derive them.
pub const XMAP20_ENTITY_DERIVATION_KEEP_REASON: &str =
    "no entity-lifecycle authority exists on any wire: the XMAP13 entity census was never \
     built and the effect-lifecycle producer is held by XMAP20-G2-lifecycle-sequencing, \
     XMAP20-G5-census-completeness-and-snapshot-safety, XMAP20-G6-unattributed-trigger-receipt \
     and XMAP20-G7-unsourced-required-wire-fields; entity submissions keep their sealed-carrier \
     transport until an owner-proven authority lands";

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::UsedContentKey;

    fn content_key() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            normalized_game_relative_path: "gfx/fx/particle.shader".into(),
            winning_layer_ordinal: 0,
            layer_kind: portable_content_manifest::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "a".repeat(64),
            payload_sha256: "a".repeat(64),
            payload_bytes: 1,
        }
    }

    #[test]
    fn entity_side_is_kept_with_gates_named_not_claimed_derivable() {
        assert!(XMAP20_ENTITY_DERIVATION_KEEP_REASON.contains("XMAP13"));
        assert!(XMAP20_ENTITY_DERIVATION_KEEP_REASON.contains("G7"));
    }

    #[test]
    fn fnv_matches_the_census_registration_law() {
        // Census authority: type+0x80 == FNV-1a-32(token); live-confirmed on
        // hoi4.exe 1.19.2.0.a729 ('rain_small.asset' family reads).
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
    }

    struct TestEffect {
        id: u64,
        token: String,
        hash: u32,
        digest: String,
    }
    impl CensusIdentity for TestEffect {
        fn census_identity(&self) -> (u64, &str, u32, &str) {
            (self.id, &self.token, self.hash, &self.digest)
        }
    }

    fn effect_record(id: u64, token: &str, digest: String) -> TestEffect {
        effect_record_with_hash(id, token, fnv1a32(token.as_bytes()), digest)
    }

    fn effect_record_with_hash(id: u64, token: &str, hash: u32, digest: String) -> TestEffect {
        TestEffect {
            id,
            token: token.into(),
            hash,
            digest,
        }
    }

    #[test]
    fn identity_join_enforces_membership_token_digest_and_hash_laws() {
        let token = "gfx/particles/environment/rain_small.asset";
        let authority = Xmap20DefinitionAuthority::from_census(
            &[effect_record(7, token, "d".repeat(64))],
            &[],
        )
        .expect("effects-only census builds");
        // Missing census membership refuses.
        assert!(authority.join_identity(9, token, "d".repeat(64).as_str()).is_err());
        // Token mismatch refuses.
        assert!(authority
            .join_identity(7, "gfx/particles/other.asset", "d".repeat(64).as_str())
            .is_err());
        // Definition-digest mismatch refuses.
        assert!(authority
            .join_identity(7, token, "e".repeat(64).as_str())
            .is_err());
        // A recorded hash that violates the FNV law refuses the join even with
        // matching token+digest.
        let forged = Xmap20DefinitionAuthority::from_census(
            &[effect_record_with_hash(7, token, 0xdead_beef, "d".repeat(64))],
            &[],
        )
        .expect("forged census builds");
        assert!(forged
            .join_identity(7, token, "d".repeat(64).as_str())
            .is_err());
        // The lawful combination joins.
        assert!(authority
            .join_identity(7, token, "d".repeat(64).as_str())
            .is_ok());
    }

    #[test]
    fn constant_buffer_join_compares_layout_not_per_instance_bytes() {
        use portable_render_stream::ConstantBufferBinding;
        let layout = |slot: u8| ConstantBufferBinding {
            stage_mask_u8: 1,
            slot,
            byte_offset: 0,
            byte_count: 64,
            raw_bytes: vec![1, 2, 3],
            sha256: "a".repeat(64),
            semantic_layout: "ScaleCB".into(),
        };
        // Same binding layout, different per-instance bytes: joins.
        let mut instance2 = layout(2);
        instance2.raw_bytes = vec![9, 9];
        instance2.sha256 = "b".repeat(64);
        assert!(constant_buffer_layouts_match(&[layout(2)], &[instance2]));
        // Different slot/layout: refuses.
        assert!(!constant_buffer_layouts_match(&[layout(2)], &[layout(3)]));
    }

    #[test]
    fn geometry_join_ignores_only_the_per_instance_draw_count() {
        use portable_render_stream::{
            BufferSource, DrawCommand, GeometryBinding, IndexFormat, VertexBufferBinding,
        };
        let geometry = |instance_count: u32, input_slot: u32| GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot,
                stride_bytes: 48,
                byte_offset: 0,
                source: BufferSource::ParticleInstanceRecords,
            }],
            index_buffer: None,
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawInstanced {
                vertex_count_per_instance: 4,
                instance_count,
                start_vertex: 0,
                start_instance: 0,
            },
        };
        // Lawful: two instances of one subsystem differing ONLY in
        // instance_count (= live_count).
        assert!(geometry_static_eq(&geometry(12, 0), &geometry(34, 0)));
        // Different mesh resources: refuses.
        assert!(!geometry_static_eq(&geometry(12, 0), &geometry(12, 1)));
        // Different draw shape beyond counts: refuses.
        let indexed = GeometryBinding {
            draw: DrawCommand::DrawIndexed {
                index_count: 4,
                start_index: 0,
                base_vertex: 0,
            },
            ..geometry(12, 0)
        };
        assert!(!geometry_static_eq(&indexed, &geometry(12, 0)));
        assert_eq!(IndexFormat::R32Uint, IndexFormat::R32Uint);
    }

    #[test]
    fn program_path_law_refuses_anything_but_the_particle_shader() {
        let mut lawful = content_key();
        assert!(lawful_program_source_path(&lawful).is_ok());
        lawful.normalized_game_relative_path = "gfx/fx/world.shader".into();
        assert!(lawful_program_source_path(&lawful).is_err());
    }
}
