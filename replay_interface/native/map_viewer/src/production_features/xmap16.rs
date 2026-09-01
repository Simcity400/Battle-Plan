
use crate::rail_files::{
    exact_supply_arrow_mesh_from_resolver, supply_hub_content_identity_from_resolver,
    ExactRailPairTransitionState, RailGeometryDefines, SupplyHubContentIdentity, DEFINES_SOURCE,
    GRAPHICS_DEFINES_SOURCE, RAIL_HEIGHTMAP_SOURCE, RAIL_SHADER_SOURCE, RAIL_TEXTURE_SOURCE,
    SUPPLY_GFX_SOURCE, SUPPLY_MAPICONS_GUI_SOURCE,
};
use crate::production_features::xmap06::XMAP06_SELECTED_PROVINCE_ID_PREFIX;
use crate::xmap16_hub_derivation::{
    derive_xmap16_hub_effective_cutoffs, derive_xmap16_hub_frames, Xmap16HubViewerInputs,
    XMAP16_STATE_FIRST_HUB_ELIGIBILITY_GATES, XMAP16_STATE_FIRST_HUB_UNDERIVED_OPERANDS,
};
use crate::{
    prepare_exact_rail_replay_carrier, AssignedStage, ExactRailReplayCarrier,
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    ProductionMapFeatureAdapter, Result,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    FamilyDisposition, FoundationGap, MapVisualFamily, RailSupplyArrowFrame, RailSupplyMapArrowMesh,
};
use replay_core::{
    RailHubEffectiveCutoffs, RailHubStateFirstVisual, RailHubVisibilityCutoffClass,
    ValidatedContentAdmission, ViewStatus,
};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub const XMAP16_RAIL_SUPPLY_SCHEMA: &str = "hoi4.xmap16.rail-supply.a729.v1";
pub const XMAP16_HUB_STAGE_NAME: &str = "xmap16.supply_hubs.execute";
pub const XMAP16_HUB_OPERATION_NAME: &str = "rail.supply-hubs.draw";
const XMAP16_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const OWNED_GAPS: [&str; 3] = [
    "logistics.rail_hub_frames",
    "logistics.rail_topology_state",
    "mapmode.supply_state",
];
const RAIL_USED_PIXEL_SAMPLERS: [(&str, &str); 6] = [
    ("RailTexture", "XMAP16"),
    ("ProvinceSecondaryColorMap", "XMAP05"),
    ("SnowMudTexture", "XMAP07"),
    ("GradientBorderChannel1", "XMAP05"),
    ("GradientBorderChannel2", "XMAP05"),
    ("ShadowMap", "XMAP10"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap16RailSampler {
    pub symbol: String,
    pub shader_register: u32,
    pub owner_packet: String,
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
    pub sampler_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap16RailProgram {
    pub effect_file: String,
    pub effect_name: String,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub vertex_translation_unit_sha256: String,
    pub pixel_translation_unit_sha256: String,
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub samplers: Vec<PreparedXmap16RailSampler>,
    pub map_width: u32,
    pub map_height: u32,
    pub vertex_stride_bytes: u32,
    pub blend_source: String,
    pub blend_destination: String,
    pub color_write_mask: u8,
    pub depth_enabled: bool,
    pub stencil_enabled: bool,
    pub deterministic_sha256: String,
}

impl PreparedXmap16RailProgram {
    pub fn validate(&self) -> Result<()> {
        if self.effect_file != RAIL_SHADER_SOURCE
            || self.effect_name != crate::RAIL_SHADER_EFFECT
            || self.ordered_shader_content.len() != 7
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
            || self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
            || self.map_width == 0
            || self.map_height == 0
            || self.vertex_stride_bytes != crate::A729_RAILROAD_VERTEX_STRIDE_BYTES
            || self.blend_source != "src_alpha"
            || self.blend_destination != "inv_src_alpha"
            || self.color_write_mask != 0x07
            || self.depth_enabled
            || self.stencil_enabled
            || self.deterministic_sha256 != rail_program_digest(self)
        {
            return Err("XMAP16 executable railroad program is invalid".into());
        }
        let vertex = crate::parse_dxbc_reflection(&self.vertex_bytecode)?;
        let pixel = crate::parse_dxbc_reflection(&self.pixel_bytecode)?;
        if vertex.target >> 16 != 0xfffe
            || pixel.target >> 16 != 0xffff
            || vertex
                .inputs
                .iter()
                .filter(|input| {
                    input.semantic_name.eq_ignore_ascii_case("POSITION")
                        && input.semantic_index == 0
                        && input.mask == 0x0f
                })
                .count()
                != 1
            || vertex
                .inputs
                .iter()
                .filter(|input| {
                    input.semantic_name.eq_ignore_ascii_case("TEXCOORD")
                        && input.semantic_index == 0
                        && input.mask & 0x03 == 0x03
                })
                .count()
                != 1
        {
            return Err("XMAP16 railroad DXBC has the wrong stage/input contract".into());
        }
        let expected = RAIL_USED_PIXEL_SAMPLERS
            .iter()
            .map(|(symbol, owner)| ((*symbol).to_string(), (*owner).to_string()))
            .collect::<BTreeSet<_>>();
        let actual = self
            .samplers
            .iter()
            .map(|sampler| (sampler.symbol.clone(), sampler.owner_packet.clone()))
            .collect::<BTreeSet<_>>();
        if actual != expected
            || self.samplers.iter().any(|sampler| {
                sampler.symbol.is_empty()
                    || sampler.owner_packet.is_empty()
                    || sampler.mag_filter.is_empty()
                    || sampler.min_filter.is_empty()
                    || sampler.mip_filter.is_empty()
                    || sampler.address_u.is_empty()
                    || sampler.address_v.is_empty()
                    || sampler.sampler_type.is_empty()
            })
        {
            return Err("XMAP16 railroad sampler ownership is incomplete".into());
        }
        let reflected = pixel
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 2)
            .map(|resource| {
                (
                    resource
                        .name
                        .strip_suffix("_Texture")
                        .unwrap_or(&resource.name)
                        .to_string(),
                    resource.bind_point,
                )
            })
            .collect::<BTreeSet<_>>();
        let declared = self
            .samplers
            .iter()
            .map(|sampler| (sampler.symbol.clone(), sampler.shader_register))
            .collect::<BTreeSet<_>>();
        if reflected != declared {
            return Err("XMAP16 railroad reflected texture slots drifted".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedXmap16Frame {
    pub schema: String,
    pub hour_index: i64,
    pub rail_stage: AssignedStage,
    pub supply_arrow_stage: AssignedStage,
    pub hub_stage: AssignedStage,
    pub program: Arc<PreparedXmap16RailProgram>,
    pub rail: ExactRailReplayCarrier,
    pub supply_arrows: RailSupplyArrowFrame,
    pub supply_definitions: Arc<[crate::PreparedXmap16SupplyArrowDefinition]>,
    pub supply_program: Arc<crate::PreparedXmap16SupplyArrowProgram>,
    pub supply_mesh: RailSupplyMapArrowMesh,
    pub hub_effective_cutoffs: RailHubEffectiveCutoffs,
    pub hub_visual_states: BTreeMap<(u32, u32, u8), RailHubStateFirstVisual>,
    pub hub_content: Arc<SupplyHubContentIdentity>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub deterministic_sha256: String,
}

impl PreparedXmap16Frame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP16_RAIL_SUPPLY_SCHEMA
            || validate_assigned_stages(&self.rail_stage, &self.supply_arrow_stage, &self.hub_stage)
                .is_err()
            || self.program.validate().is_err()
            || self.rail.authority != "exact_a729_supply_simulation_tables"
            || self.rail.ordered_route_records.len() != self.rail.ribbon_program.ordered_draws.len()
            || self.supply_arrows.presentation_authority != "state_first_mode_neutral"
            || self.deterministic_sha256 != prepared_digest(self)
        {
            return Err("XMAP16 prepared rail/supply frame is invalid".into());
        }
        validate_rail_draw_program(&self.rail.ordered_route_records, &self.rail.ribbon_program)?;
        if self.rail.ribbon_program.ordered_draws.is_empty()
            != self.rail.ribbon_program.vertices.is_empty()
        {
            return Err("XMAP16 measured-empty ribbon payload is inconsistent".into());
        }
        self.supply_arrows
            .validate()
            .map_err(|error| format!("XMAP16 supply-arrow carrier: {error}"))?;
        if self.supply_definitions.len() != 3
            || self
                .supply_definitions
                .iter()
                .any(|definition| definition.validate().is_err())
            || self.supply_program.validate().is_err()
            || self.supply_mesh.ordered_draws.len() != self.supply_arrows.rows.len()
        {
            return Err("XMAP16 prepared supply-arrow executable payload is invalid".into());
        }
        validate_hub_cutoffs(
            &self.hub_effective_cutoffs,
            &self.hub_visual_states,
            &self.rail.defines,
        )?;
        if &self.rail.hub_content != self.hub_content.as_ref() {
            return Err("XMAP16 rail and hub carriers resolved different effective content".into());
        }
        let mut paths = BTreeSet::new();
        if self
            .ordered_used_content
            .iter()
            .any(|content| !paths.insert(content.normalized_game_relative_path.as_str()))
        {
            return Err("XMAP16 used-content closure is duplicated".into());
        }
        for required in [
            &self.rail.railroad_shader,
            &self.rail.railroad_texture,
            &self.rail.railroad_heightmap,
        ] {
            if !self.ordered_used_content.iter().any(|content| {
                content
                    .normalized_game_relative_path
                    .eq_ignore_ascii_case(&required.source)
                    && content.payload_sha256 == required.sha256
                    && content.payload_bytes == required.byte_len
            }) {
                return Err(
                    format!("XMAP16 used-content closure omitted {}", required.source).into(),
                );
            }
        }
        for required in &self.supply_arrows.used_content {
            if !self.ordered_used_content.contains(required) {
                return Err(format!(
                    "XMAP16 used-content closure omitted supply input {}",
                    required.normalized_game_relative_path
                )
                .into());
            }
        }
        if !self
            .ordered_used_content
            .contains(&self.hub_effective_cutoffs.graphics_defines_content)
        {
            return Err(
                "XMAP16 used-content closure omitted hub effective graphics defines".into(),
            );
        }
        validate_hub_used_content(&self.ordered_used_content, &self.hub_content)?;
        Ok(())
    }
}

struct Xmap16RuntimeState {
    previous_alpha: f32,
    last_hour_index: Option<i64>,
    last_rail: Option<ExactRailReplayCarrier>,
}

pub(crate) struct Xmap16RailSupplyAdapter {
    resolver: GameFileResolver,
    ordered_used_content: Vec<UsedContentKey>,
    program: Arc<PreparedXmap16RailProgram>,
    rail_defines: RailGeometryDefines,
    /// The effective `00_graphics.lua` the supply-icon cutoffs are read from.
    /// Held so the derived cutoff table names the exact content it came from,
    /// the same way the retired carrier's `graphics_defines_content` did.
    graphics_defines_content: UsedContentKey,
    hub_content: Arc<SupplyHubContentIdentity>,
    supply_definitions: Arc<[crate::PreparedXmap16SupplyArrowDefinition]>,
    supply_program: Arc<crate::PreparedXmap16SupplyArrowProgram>,
    runtime: Mutex<Xmap16RuntimeState>,
}

impl Xmap16RailSupplyAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
    ) -> Result<Self> {
        let hub_content = Arc::new(supply_hub_content_identity_from_resolver(resolver)?);
        let mut paths = BTreeSet::new();
        for (path, _) in
            crate::shader_compile::effect_translation_unit_files(resolver, RAIL_SHADER_SOURCE)?
        {
            paths.insert(path);
        }
        paths.extend([
            RAIL_TEXTURE_SOURCE.into(),
            RAIL_HEIGHTMAP_SOURCE.into(),
            GRAPHICS_DEFINES_SOURCE.into(),
            DEFINES_SOURCE.into(),
            SUPPLY_MAPICONS_GUI_SOURCE.into(),
            SUPPLY_GFX_SOURCE.into(),
            portable_render_stream::RAIL_SUPPLY_MAPARROWS_PATH.into(),
            portable_render_stream::RAIL_SUPPLY_PROVINCES_BMP_PATH.into(),
        ]);
        paths.extend([
            hub_content.background.sprite.texture_file.clone(),
            hub_content.common_icon.sprite.texture_file.clone(),
            hub_content.naval_overlay.sprite.texture_file.clone(),
        ]);
        let mut ordered_used_content = paths
            .into_iter()
            .map(|path| {
                admission
                    .require_used_content(&path)
                    .map(Clone::clone)
                    .map_err(|error| {
                        format!("XMAP16 requires admitted effective content {path}: {error}").into()
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let supply_definitions =
            crate::prepare_xmap16_supply_arrow_definitions(resolver, admission)?;
        let supply_program = crate::prepare_xmap16_supply_arrow_program(resolver, admission)?;
        for content in supply_definitions
            .iter()
            .flat_map(|definition| definition.ordered_used_content.iter())
            .chain(supply_program.ordered_shader_content.iter())
        {
            if !ordered_used_content.contains(content) {
                ordered_used_content.push(content.clone());
            }
        }
        ordered_used_content.sort_by(|left, right| {
            left.normalized_game_relative_path
                .cmp(&right.normalized_game_relative_path)
        });
        let (map_width, map_height) = crate::foundation_map_dimensions(resolver, admission)?;
        let program = Arc::new(prepare_rail_program(
            resolver,
            admission,
            &ordered_used_content,
            map_width,
            map_height,
        )?);
        let rail_defines = crate::rail_geometry_defines_from_resolver(resolver)?;
        let graphics_defines_content = admission
            .require_used_content(GRAPHICS_DEFINES_SOURCE)
            .map(Clone::clone)
            .map_err(|error| {
                format!(
                    "XMAP16 requires admitted effective content {GRAPHICS_DEFINES_SOURCE}: {error}"
                )
            })?;
        Ok(Self {
            resolver: resolver.clone(),
            ordered_used_content,
            program,
            rail_defines,
            graphics_defines_content,
            hub_content,
            supply_definitions: supply_definitions.into(),
            supply_program: Arc::new(supply_program),
            runtime: Mutex::new(Xmap16RuntimeState {
                previous_alpha: crate::RAILWAY_ALPHA_INITIAL,
                last_hour_index: None,
                last_rail: None,
            }),
        })
    }
}

/// The gap id XMAP16 refuses under when its own DERIVED visual family cannot be
/// completed.
///
/// Distinct from `XMAP16.state_first_input_unavailable`, which is the id for an hour
/// whose XMAP00 inputs are missing. Sharing one id made a viewer-side derivation gap
/// read as a recording gap, which is the exact confusion this lane closed.
pub const XMAP16_DERIVED_VISUAL_GAP_ID: &str = "XMAP16.derived_visual_family_incomplete";

/// The two viewer-local words the supply-hub frame law reads, plus the perspective the
/// hub census is keyed by.
struct Xmap16HubDerivationInputs {
    perspective_country_id: u32,
    law: Xmap16HubViewerInputs,
}

/// Read the hub law's viewer-local operands out of the viewer's own state.
///
/// The outer `Result` is a contract violation; the inner `Err(String)` is a named
/// viewer-state condition the family reports as a gap rather than an error, because a
/// viewer that has selected no country is a legitimate viewer state and not a defect.
fn xmap16_hub_viewer_inputs(
    context: &FoundationFeatureContext<'_>,
) -> Result<std::result::Result<Xmap16HubDerivationInputs, String>> {
    let Some(tag) = context.viewer.observer_country_id.as_deref() else {
        return Ok(Err(
            "the supply-hub frame law compares the hub's owner with the country the viewer is \
             watching, and this viewer has selected none"
                .into(),
        ));
    };
    // PARTIAL: one country's identity, not a census. A country the recorder could not
    // name this hour does not make the watched one less named; the excluded keys ride
    // the refusal so "absent from D18" and "in the frame but unnameable" stay apart.
    // An hour whose D18 roster cannot be read is a SHORT recording, not a defect in this
    // layer: it is the same shape of absence as a viewer that has selected no country, and
    // it is refused the same way. Raising it would take the whole composed frame down over
    // one family's missing input.
    let roster = context.world_frame.d18_country_roster();
    let partial = match roster.partial() {
        Ok(partial) => partial,
        Err(refusal) => {
            return Ok(Err(format!(
                "the supply-hub frame law compares the hub's owner with the country the viewer is \
                 watching, and this hour's D18 country roster cannot name either: {refusal}"
            )))
        }
    };
    let Some(perspective_country_id) = partial
        .resolved
        .iter()
        .find(|entry| entry.tag == tag)
        .map(|entry| entry.tag_db_id)
    else {
        let unnamed = if partial.excluded.is_empty() {
            String::new()
        } else {
            format!(
                " ({} emitted country key(s) carry no D18 identity this hour: {})",
                partial.excluded.len(),
                partial.excluded.join(",")
            )
        };
        return Ok(Err(format!(
            "the viewer is watching {tag}, which is absent from this hour's D18 roster{unnamed}, \
             so the hub frame law has no canonical identity to compare owners with"
        )));
    };
    let mut selected = Vec::new();
    for id in &context.viewer.selected_object_ids {
        let Some(raw) = id.strip_prefix(XMAP06_SELECTED_PROVINCE_ID_PREFIX) else {
            continue;
        };
        let province = raw.parse::<u32>().map_err(|error| {
            format!("XMAP16 viewer selection {id:?} has no province id: {error}")
        })?;
        if province == 0 {
            return Err(format!("XMAP16 viewer selection {id:?} names province 0").into());
        }
        selected.push(province);
    }
    selected.sort_unstable();
    selected.dedup();
    // The engine's law has ONE selected province and no answer for two. Picking one of
    // them, or treating the set as a membership test, would both be viewer choices the
    // engine never makes, so a multi-province selection is named rather than resolved.
    let selected_province_id = match selected.len() {
        0 => None,
        1 => Some(selected[0]),
        count => {
            return Ok(Err(format!(
                "the supply-hub frame law admits at most one selected province and this viewer \
                 holds {count}"
            )))
        }
    };
    Ok(Ok(Xmap16HubDerivationInputs {
        perspective_country_id,
        law: Xmap16HubViewerInputs {
            selected_province_id,
            effective_country_id: perspective_country_id,
        },
    }))
}

impl Xmap16RailSupplyAdapter {
    /// Refuse the family for a DERIVED-side reason, with the reason named.
    ///
    /// It consumes no XMAP00 gap id on purpose: nothing the map authority refused is
    /// what stopped this frame, and claiming one would report a recording gap that does
    /// not exist.
    fn derived_family_unavailable(
        &self,
        context: &FoundationFeatureContext<'_>,
        this_frame: Option<String>,
        derived_hub_frame_count: Option<usize>,
    ) -> FoundationFeatureOutput {
        let hour = context.world_frame.hour_index();
        let applied = match derived_hub_frame_count {
            Some(count) => format!(
                "the hub frame law is applied and produced {count} frame(s) for the viewer's own \
                 perspective"
            ),
            None => "the hub frame law was not reached this frame".into(),
        };
        let this_frame = this_frame
            .map(|reason| format!("{reason}; "))
            .unwrap_or_default();
        let gates = XMAP16_STATE_FIRST_HUB_ELIGIBILITY_GATES
            .iter()
            .map(|(id, why)| format!("{id} ({why})"))
            .collect::<Vec<_>>()
            .join("; ");
        let reason = format!(
            "XMAP16 derived visual family is incomplete: {this_frame}{applied}. The hub \
             visibility law IS decoded -- its cutoff class is the engine's own arithmetic \
             over two supply terms (map_viewer::xmap16_hub_visibility_cutoff_class, pinned \
             in portable_engine_evidence::xmap16_hub_visibility) -- and what is left is \
             named rather than deferred. First, that law applies to the terms the engine \
             RESOLVES for a province, and which of D81's subtype rows it resolves to is \
             undecoded. Second, the two eligibility flags {}: their law is decoded and the \
             arms do NOT share a second disjunct -- the railway arm is (mode == 5 || scope) \
             and the supply arm is (mode == 5 || (presentation_word && scope && \
             country_context)) -- so the map mode and the scope are the viewer's own while \
             the supply arm carries two further operands, one of them a country context \
             whose viewer-locality is unproven. Every undecoded value is one gate -- \
             {gates}. Third, the rail ribbon needs the engine-selected connector, tangent \
             lane did not read, and the supply-arrow layer needs the sealed state-first \
             arrow frame, which lane RL adjudicated as requiring its own a729 proof before \
             any viewer work starts",
            XMAP16_STATE_FIRST_HUB_UNDERIVED_OPERANDS.join(" and "),
        );
        FoundationFeatureOutput {
            disposition: FamilyDisposition::Unavailable {
                gap: FoundationGap {
                    gap_id: XMAP16_DERIVED_VISUAL_GAP_ID.into(),
                    first_unavailable_hour: hour,
                    last_unavailable_hour: hour,
                    witness_sha256: Sha256::hex(
                        format!("{XMAP16_DERIVED_VISUAL_GAP_ID}\n{hour}\n{reason}").as_bytes(),
                    ),
                    reason,
                },
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_lighting: None,
            prepared_rail_supply: None,
            prepared_weather: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids: Vec::new(),
        }
    }
}

impl ProductionMapFeatureAdapter for Xmap16RailSupplyAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::RailSupply
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let owned_gaps = context
            .map_authority_gaps
            .iter()
            .filter(|gap| OWNED_GAPS.contains(&gap.input_id.as_str()))
            .map(|gap| gap.input_id.clone())
            .collect::<Vec<_>>();
        if !owned_gaps.is_empty() {
            // XMAP00's blocker rides along, and so does what THIS hour observed:
            // naming the input alone sent a reader looking for a missing key,
            // while `logistics.rail_hub_frames` is a DERIVED family whose
            // measured input is the supply-hub census inside a carrier the
            // recording usually does hold. Which of the two it is on this hour
            // is a per-hour fact and is read, never asserted.
            let rail = context.world_frame.rail_replay();
            let observed = format!(
                "this hour's map.rail_replay_state_live is {:?} and the measured inputs its \
                 derived visual frames are built from are {:?}",
                rail.status,
                rail.derived_visual_frame_inputs_status()
            );
            let reason = format!(
                "XMAP16 exact state-first carrier unavailable ({observed}): {}",
                owned_gaps
                    .iter()
                    .map(|input| {
                        let blocker = context
                            .map_authority_gaps
                            .iter()
                            .find(|gap| &gap.input_id == input)
                            .map(|gap| gap.reason.as_str())
                            .unwrap_or("XMAP00 holds no blocker for this input");
                        format!("{input}: {blocker}")
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            let gap_id = "XMAP16.state_first_input_unavailable".to_string();
            return Ok(FoundationFeatureOutput {
                disposition: FamilyDisposition::Unavailable {
                    gap: FoundationGap {
                        gap_id: gap_id.clone(),
                        first_unavailable_hour: context.world_frame.hour_index(),
                        last_unavailable_hour: context.world_frame.hour_index(),
                        witness_sha256: Sha256::hex(
                            format!("{gap_id}\n{}\n{reason}", context.world_frame.hour_index())
                                .as_bytes(),
                        ),
                        reason,
                    },
                },
                prepared_terrain: None,
                prepared_political: None,
                prepared_thin_borders: None,
                prepared_lighting: None,
                prepared_rail_supply: None,
                prepared_weather: None,
                prepared_rivers_straits: None,
                prepared_water: None,
                prepared_units: None,
                prepared_combat: None,
                prepared_orders: None,
                prepared_map_modes: None,
                element_diagnostics: Vec::new(),
                consumed_gap_ids: owned_gaps,
            });
        }

        let view = context.world_frame.rail_replay();
        if view.status != ViewStatus::Present {
            return Err("XMAP16 map authority admitted an absent rail replay view".into());
        }

        // ---- The hub visual family, DERIVED here ----
        //
        // The frames a hub draws with are a function of one measured simulation
        // row plus viewer-local inputs, so they are this layer's to compute and
        // no recording carries them. Everything the derivation needs is either
        // in the admitted D81 carrier or in the viewer's own state.
        let hub_effective_cutoffs = derive_xmap16_hub_effective_cutoffs(
            &self.rail_defines,
            self.graphics_defines_content.clone(),
        )?;
        let hub_inputs = match xmap16_hub_viewer_inputs(context)? {
            Ok(inputs) => inputs,
            Err(reason) => {
                return Ok(self.derived_family_unavailable(context, Some(reason), None))
            }
        };
        let derived_hub_frames =
            match derive_xmap16_hub_frames(view, hub_inputs.perspective_country_id, hub_inputs.law)
            {
                Ok(derived) => derived,
                Err(reason) => {
                    return Ok(self.derived_family_unavailable(context, Some(reason), None))
                }
            };

        // ---- What is still owed, and to whom ----
        //
        // These three are the rest of the derived visual family. None of them
        // is a recorder gap: the recording is correct to carry no rendered
        // frame. Each is a viewer derivation that cannot be written because an
        // engine law this build has not decoded chooses its operands. The
        // refusal names them so a reader is not sent looking for a missing key.
        if !view.pair_transitions_complete
            || !view.supply_arrows_complete
            || !view.hub_visual_states_complete
        {
            return Ok(self.derived_family_unavailable(
                context,
                None,
                Some(derived_hub_frames.frames.len()),
            ));
        }
        let pair_transitions = view
            .pair_transitions
            .as_ref()
            .ok_or("XMAP16 exact pair transitions are absent")?
            .iter()
            .map(|(identity, source)| {
                (
                    *identity,
                    ExactRailPairTransitionState {
                        from_province_id: source.from_province_id,
                        to_province_id: source.to_province_id,
                        connector: source.connector,
                        tangent: source.tangent,
                        edge_type: source.edge_type,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let supply_arrows = view
            .supply_arrows
            .as_ref()
            .ok_or("XMAP16 state-first supply-arrow frame is absent")?
            .clone();
        if u32::try_from(supply_arrows.state_first_map_width_world).ok()
            != Some(self.program.map_width)
        {
            return Err(
                "XMAP16 supply-arrow map width disagrees with effective map/provinces.bmp".into(),
            );
        }
        let hub_visual_states = view
            .hub_visual_states
            .as_ref()
            .ok_or("XMAP16 state-first hub visual rows are absent")?
            .clone();
        validate_hub_cutoffs(
            &hub_effective_cutoffs,
            &hub_visual_states,
            &self.rail_defines,
        )?;
        let supply_profiles = self
            .supply_definitions
            .iter()
            .map(|definition| definition.geometry_profile())
            .collect::<Result<Vec<_>>>()?;
        let supply_mesh = exact_supply_arrow_mesh_from_resolver(
            &self.resolver,
            &supply_arrows,
            &supply_profiles,
        )?;
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP16 rail presentation requires the viewer-owned camera")?;
        let rail_stage = context
            .require_assigned_stage(
                crate::XMAP16_RAIL_STAGE_NAME,
                crate::XMAP16_RAIL_OPERATION_ID,
            )?
            .clone();
        let supply_arrow_stage = context
            .require_assigned_stage(
                crate::XMAP16_SUPPLY_ARROW_OPERATION_ID,
                crate::XMAP16_SUPPLY_ARROW_OPERATION_NAME,
            )?
            .clone();
        let hub_stage = context
            .require_assigned_stage(XMAP16_HUB_STAGE_NAME, XMAP16_HUB_OPERATION_NAME)?
            .clone();
        validate_assigned_stages(&rail_stage, &supply_arrow_stage, &hub_stage)?;
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "XMAP16 runtime state mutex is poisoned")?;
        // The cached-hour arm reuses the previous frame's geometry but NOT its
        // hub visuals: the frame law reads the viewer's selected province and
        // effective country, both of which change between frames inside one
        // recorded hour, so a cached hub visual would be another frame's.
        let rail = if runtime.last_hour_index == Some(context.world_frame.hour_index()) {
            let mut rail = runtime
                .last_rail
                .clone()
                .ok_or("XMAP16 cached hour omitted its exact rail carrier")?;
            rail.hub_visuals = derived_hub_frames
                .frames
                .iter()
                .map(|((province_id, subtype), row)| crate::ExactRailHubVisualState {
                    province_id: *province_id,
                    subtype: *subtype,
                    background_button_frame: row.background_button_frame,
                    common_icon_frame: row.common_icon_frame,
                    naval_overlay_visible: row.naval_overlay_visible,
                })
                .collect();
            rail.railway_alpha = crate::railway_alpha_step(
                runtime.previous_alpha,
                context.viewer.frame_delta_seconds(),
                camera.current_eye()[1],
                &rail.defines,
            )
            .map_err(|error| format!("XMAP16 railway alpha recurrence: {error}"))?;
            rail.railway_draw_enabled = crate::railway_alpha_draws(rail.railway_alpha)
                .map_err(|error| format!("XMAP16 railway alpha draw gate: {error}"))?;
            rail
        } else {
            prepare_exact_rail_replay_carrier(
                &self.resolver,
                view,
                context.world_frame.production_by_country(),
                &pair_transitions,
                &derived_hub_frames.frames,
                runtime.previous_alpha,
                context.viewer.frame_delta_seconds(),
                camera.current_eye()[1],
            )?
        };
        runtime.previous_alpha = rail.railway_alpha;
        runtime.last_hour_index = Some(context.world_frame.hour_index());
        runtime.last_rail = Some(rail.clone());
        drop(runtime);

        let mut ordered_used_content = self.ordered_used_content.clone();
        for content in &supply_arrows.used_content {
            if !ordered_used_content.contains(content) {
                ordered_used_content.push(content.clone());
            }
        }
        ordered_used_content.sort_by(|left, right| {
            left.normalized_game_relative_path
                .cmp(&right.normalized_game_relative_path)
        });
        let mut prepared = PreparedXmap16Frame {
            schema: XMAP16_RAIL_SUPPLY_SCHEMA.into(),
            hour_index: context.world_frame.hour_index(),
            rail_stage,
            supply_arrow_stage,
            hub_stage,
            program: Arc::clone(&self.program),
            rail,
            supply_arrows,
            supply_definitions: Arc::clone(&self.supply_definitions),
            supply_program: Arc::clone(&self.supply_program),
            supply_mesh,
            hub_effective_cutoffs,
            hub_visual_states,
            hub_content: Arc::clone(&self.hub_content),
            ordered_used_content,
            deterministic_sha256: String::new(),
        };
        prepared.deterministic_sha256 = prepared_digest(&prepared);
        prepared.validate()?;
        let registration = xmap16_producer_registration_sha256();
        let resources_sha256 = xmap16_resource_order_sha256(&prepared.ordered_used_content);
        let target_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}",
                camera.viewport_width, camera.viewport_height, camera.deterministic_sha256
            )
            .as_bytes(),
        );
        let rail_operation = prepared
            .rail_stage
            .foundation_operation_identity(
                registration.clone(),
                resources_sha256.clone(),
                target_sha256.clone(),
            )
            .map_err(|error| format!("XMAP16 rail assigned-stage identity: {error}"))?;
        let supply_operation = prepared
            .supply_arrow_stage
            .foundation_operation_identity(
                registration.clone(),
                resources_sha256.clone(),
                target_sha256.clone(),
            )
            .map_err(|error| format!("XMAP16 supply-arrow assigned-stage identity: {error}"))?;
        let hub_operation = prepared
            .hub_stage
            .foundation_operation_identity(registration.clone(), resources_sha256, target_sha256)
            .map_err(|error| format!("XMAP16 hub assigned-stage identity: {error}"))?;
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP16_RAIL_SUPPLY_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![rail_operation, supply_operation, hub_operation],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_lighting: None,
            prepared_rail_supply: Some(prepared),
            prepared_weather: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids: Vec::new(),
        })
    }
}

fn prepared_digest(prepared: &PreparedXmap16Frame) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(prepared.schema.as_bytes());
    identity.extend_from_slice(&prepared.hour_index.to_le_bytes());
    for stage in [
        &prepared.rail_stage,
        &prepared.supply_arrow_stage,
        &prepared.hub_stage,
    ] {
        identity.extend_from_slice(stage.assignment_sha256.as_bytes());
    }
    identity.extend_from_slice(prepared.program.deterministic_sha256.as_bytes());
    identity.extend_from_slice(prepared.rail.authority.as_bytes());
    identity.extend_from_slice(&prepared.rail.railway_alpha.to_bits().to_le_bytes());
    identity.push(prepared.rail.railway_draw_enabled as u8);
    for record in &prepared.rail.ordered_route_records {
        identity.extend_from_slice(&record.previous_province_id.to_le_bytes());
        identity.extend_from_slice(&record.center_province_id.to_le_bytes());
        identity.extend_from_slice(&record.next_province_id.to_le_bytes());
        identity.extend_from_slice(&record.effective_level.to_le_bytes());
    }
    for vertex in &prepared.rail.ribbon_program.vertices {
        for value in vertex.position.into_iter().chain(vertex.uv) {
            identity.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    for draw in &prepared.rail.ribbon_program.ordered_draws {
        identity.extend_from_slice(&draw.route_record_ordinal.to_le_bytes());
        identity.extend_from_slice(&draw.first_vertex.to_le_bytes());
        identity.extend_from_slice(&draw.vertex_count.to_le_bytes());
        identity.extend_from_slice(&draw.effective_level.to_le_bytes());
    }
    // The DERIVED hub frames are part of this payload and so are part of its identity.
    // They change between frames of one recorded hour -- the frame law reads the viewer's
    // selected province and effective country -- so leaving them out would report an
    // unchanged payload for a frame that draws different hubs.
    for hub in &prepared.rail.hub_visuals {
        identity.extend_from_slice(&hub.province_id.to_le_bytes());
        identity.extend_from_slice(&[
            hub.subtype,
            hub.background_button_frame,
            hub.common_icon_frame,
            hub.naval_overlay_visible as u8,
        ]);
    }
    identity.extend_from_slice(
        Sha256::hex(&serde_json::to_vec(&prepared.supply_arrows).unwrap_or_default()).as_bytes(),
    );
    identity.extend_from_slice(prepared.supply_program.deterministic_sha256.as_bytes());
    for definition in prepared.supply_definitions.iter() {
        identity.extend_from_slice(definition.deterministic_sha256.as_bytes());
    }
    for vertex in &prepared.supply_mesh.vertices {
        for value in vertex
            .position
            .into_iter()
            .chain([vertex.padding])
            .chain(vertex.uv_is_head_variant)
        {
            identity.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    for draw in &prepared.supply_mesh.ordered_draws {
        identity.extend_from_slice(&draw.row_ordinal.to_le_bytes());
        identity.extend_from_slice(&draw.first_vertex.to_le_bytes());
        identity.extend_from_slice(&draw.vertex_count.to_le_bytes());
        identity.extend_from_slice(&draw.total_length_f32_bits.to_le_bytes());
    }
    identity.extend_from_slice(
        prepared
            .hub_effective_cutoffs
            .graphics_defines_content
            .payload_sha256
            .as_bytes(),
    );
    for ((perspective, province, subtype), hub) in &prepared.hub_visual_states {
        identity.extend_from_slice(&perspective.to_le_bytes());
        identity.extend_from_slice(&province.to_le_bytes());
        identity.push(*subtype);
        for bits in hub.final_world_bits {
            identity.extend_from_slice(&bits.to_le_bytes());
        }
        identity.extend_from_slice(&[
            hub.background_button_frame,
            hub.common_icon_frame,
            hub.naval_overlay_visible as u8,
            hub.normal_eligible as u8,
            hub.supply_eligible as u8,
        ]);
        identity.extend_from_slice(&hub.visibility_cutoff.value_bits.to_le_bytes());
    }
    identity.extend_from_slice(hub_content_digest(&prepared.hub_content).as_bytes());
    for content in &prepared.ordered_used_content {
        identity.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        identity.extend_from_slice(content.payload_sha256.as_bytes());
        identity.extend_from_slice(&content.payload_bytes.to_le_bytes());
    }
    Sha256::hex(&identity)
}

pub(crate) fn xmap16_resource_order_sha256(content: &[UsedContentKey]) -> String {
    Sha256::hex(
        &content
            .iter()
            .flat_map(|row| row.payload_sha256.bytes())
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn xmap16_producer_registration_sha256() -> String {
    Sha256::hex(
        format!("XMAP16\0{XMAP16_RAIL_SUPPLY_SCHEMA}\0{XMAP16_EXECUTABLE_SHA256}").as_bytes(),
    )
}

fn validate_rail_draw_program(
    records: &[crate::rail_files::ExactRailRouteRecord],
    program: &crate::rail_files::ExactRailRibbonProgram,
) -> Result<()> {
    if records.len() != program.ordered_draws.len() {
        return Err("XMAP16 ribbon route/draw census is inconsistent".into());
    }
    let vertex_count = u32::try_from(program.vertices.len())
        .map_err(|_| "XMAP16 ribbon vertex count exceeds u32")?;
    let mut expected_first_vertex = 0u32;
    for (ordinal, (record, draw)) in records.iter().zip(&program.ordered_draws).enumerate() {
        if draw.route_record_ordinal != ordinal as u32
            || draw.first_vertex != expected_first_vertex
            || draw.vertex_count == 0
            || draw.effective_level != record.effective_level
        {
            return Err("XMAP16 ribbon draw range is invalid".into());
        }
        expected_first_vertex = expected_first_vertex
            .checked_add(draw.vertex_count)
            .ok_or("XMAP16 ribbon draw range overflows u32")?;
        if expected_first_vertex > vertex_count {
            return Err("XMAP16 ribbon draw range exceeds its vertex payload".into());
        }
    }
    if expected_first_vertex != vertex_count {
        return Err("XMAP16 ribbon draws do not cover the exact vertex payload".into());
    }
    Ok(())
}

fn validate_hub_used_content(
    ordered_used_content: &[UsedContentKey],
    hub: &SupplyHubContentIdentity,
) -> Result<()> {
    let require = |path: &str, payload_sha256: &str, payload_bytes: Option<u64>| -> Result<()> {
        let content = ordered_used_content
            .iter()
            .find(|content| {
                content
                    .normalized_game_relative_path
                    .eq_ignore_ascii_case(path)
            })
            .ok_or_else(|| format!("XMAP16 hub content closure omitted {path}"))?;
        content
            .validate()
            .map_err(|error| format!("XMAP16 hub content {path}: {error}"))?;
        if content.payload_sha256 != payload_sha256
            || payload_bytes.is_some_and(|bytes| content.payload_bytes != bytes)
        {
            return Err(format!("XMAP16 hub content identity drifted for {path}").into());
        }
        Ok(())
    };
    require(SUPPLY_MAPICONS_GUI_SOURCE, &hub.mapicons_gui_sha256, None)?;
    require(SUPPLY_GFX_SOURCE, &hub.supply_gfx_sha256, None)?;
    for binding in [&hub.background, &hub.common_icon, &hub.naval_overlay] {
        require(
            &binding.sprite.texture_file,
            &binding.texture_sha256,
            Some(binding.texture_bytes),
        )?;
        if hub.container.control(&binding.control_name).is_none() {
            return Err(
                format!("XMAP16 hub GUI container omitted {}", binding.control_name).into(),
            );
        }
    }
    Ok(())
}

fn hub_content_digest(content: &SupplyHubContentIdentity) -> String {
    let mut identity = b"XMAP16_HUB_CONTENT_V1\0".to_vec();
    identity.extend_from_slice(content.mapicons_gui_sha256.as_bytes());
    identity.extend_from_slice(content.supply_gfx_sha256.as_bytes());
    identity.extend_from_slice(content.container.name.as_bytes());
    identity.extend_from_slice(content.container.source_file.as_bytes());
    identity.extend_from_slice(&content.container.source_line.to_le_bytes());
    if let Some(position) = content.container.position {
        identity.push(1);
        identity.extend_from_slice(&position.x.to_le_bytes());
        identity.extend_from_slice(&position.y.to_le_bytes());
    } else {
        identity.push(0);
    }
    if let Some(size) = content.container.size {
        identity.push(1);
        identity.extend_from_slice(&size.width.to_le_bytes());
        identity.extend_from_slice(&size.height.to_le_bytes());
    } else {
        identity.push(0);
    }
    for binding in [
        &content.background,
        &content.common_icon,
        &content.naval_overlay,
    ] {
        identity.extend_from_slice(binding.control_name.as_bytes());
        identity.extend_from_slice(binding.sprite.name.as_bytes());
        identity.extend_from_slice(binding.sprite.texture_file.as_bytes());
        identity.extend_from_slice(&binding.sprite.no_of_frames.to_le_bytes());
        identity.push(binding.sprite.frame_direction_y as u8);
        identity.extend_from_slice(&binding.sprite.texture_width.to_le_bytes());
        identity.extend_from_slice(&binding.sprite.texture_height.to_le_bytes());
        identity.extend_from_slice(binding.sprite.dds_format.as_bytes());
        identity.extend_from_slice(&binding.sprite.mip_count.to_le_bytes());
        identity.extend_from_slice(&binding.sprite.frame_width.to_le_bytes());
        identity.extend_from_slice(&binding.sprite.frame_height.to_le_bytes());
        for frame in &binding.sprite.frames {
            identity.extend_from_slice(&frame.index.to_le_bytes());
            identity.extend_from_slice(&frame.rect.x.to_le_bytes());
            identity.extend_from_slice(&frame.rect.y.to_le_bytes());
            identity.extend_from_slice(&frame.rect.width.to_le_bytes());
            identity.extend_from_slice(&frame.rect.height.to_le_bytes());
            for value in [frame.uv.u0, frame.uv.v0, frame.uv.u1, frame.uv.v1] {
                identity.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        identity.extend_from_slice(binding.texture_sha256.as_bytes());
        identity.extend_from_slice(&binding.texture_bytes.to_le_bytes());
        if let Some(control) = content.container.control(&binding.control_name) {
            identity.push(1);
            if let Some(position) = control.position {
                identity.push(1);
                identity.extend_from_slice(&position.x.to_le_bytes());
                identity.extend_from_slice(&position.y.to_le_bytes());
            } else {
                identity.push(0);
            }
            if let Some(size) = control.size {
                identity.push(1);
                identity.extend_from_slice(&size.width.to_le_bytes());
                identity.extend_from_slice(&size.height.to_le_bytes());
            } else {
                identity.push(0);
            }
        } else {
            identity.push(0);
        }
    }
    Sha256::hex(&identity)
}

fn validate_assigned_stages(
    rail: &AssignedStage,
    supply_arrow: &AssignedStage,
    hub: &AssignedStage,
) -> Result<()> {
    for (stage, stage_name, operation_name, label) in [
        (
            rail,
            crate::XMAP16_RAIL_STAGE_NAME,
            crate::XMAP16_RAIL_OPERATION_ID,
            "rail",
        ),
        (
            supply_arrow,
            crate::XMAP16_SUPPLY_ARROW_OPERATION_ID,
            crate::XMAP16_SUPPLY_ARROW_OPERATION_NAME,
            "supply-arrow",
        ),
        (hub, XMAP16_HUB_STAGE_NAME, XMAP16_HUB_OPERATION_NAME, "hub"),
    ] {
        stage
            .validate()
            .map_err(|error| format!("XMAP16 {label} assigned stage: {error}"))?;
        if stage.stage_name != stage_name || stage.operation_name != operation_name {
            return Err(format!("XMAP16 {label} assigned-stage identity drifted").into());
        }
    }
    if rail.topology_sha256 != supply_arrow.topology_sha256
        || rail.topology_sha256 != hub.topology_sha256
        || rail.assigned_ordinal >= supply_arrow.assigned_ordinal
    {
        return Err("XMAP16 assigned-stage topology/order is invalid".into());
    }
    Ok(())
}

fn validate_hub_cutoffs(
    cutoffs: &RailHubEffectiveCutoffs,
    rows: &BTreeMap<(u32, u32, u8), RailHubStateFirstVisual>,
    defines: &RailGeometryDefines,
) -> Result<()> {
    for (carried, effective) in [
        (cutoffs.supply_icon, defines.supply_icon_cutoff),
        (cutoffs.unused, defines.supply_icon_unused_cutoff),
        (
            cutoffs.disconnected,
            defines.supply_icon_disconnected_cutoff,
        ),
        (cutoffs.ok, defines.supply_icon_ok_cutoff),
        (cutoffs.numbers, defines.supply_icon_numbers_cutoff),
        (cutoffs.end, defines.supply_icon_end_cutoff),
    ] {
        if carried.value_bits != effective.to_bits()
            || carried.value.to_bits() != effective.to_bits()
        {
            return Err(
                "XMAP16 hub cutoff carrier disagrees with effective 00_graphics.lua".into(),
            );
        }
    }
    for row in rows.values() {
        let expected = match row.visibility_cutoff_class {
            RailHubVisibilityCutoffClass::SupplyIcon => defines.supply_icon_cutoff,
            RailHubVisibilityCutoffClass::Unused => defines.supply_icon_unused_cutoff,
            RailHubVisibilityCutoffClass::Disconnected => defines.supply_icon_disconnected_cutoff,
            RailHubVisibilityCutoffClass::End => defines.supply_icon_end_cutoff,
        };
        if row.visibility_cutoff.value_bits != expected.to_bits() {
            return Err("XMAP16 hub row cutoff disagrees with effective 00_graphics.lua".into());
        }
    }
    Ok(())
}

fn prepare_rail_program(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    admitted_content: &[UsedContentKey],
    map_width: u32,
    map_height: u32,
) -> Result<PreparedXmap16RailProgram> {
    let translation_unit =
        crate::shader_compile::effect_translation_unit_files(resolver, RAIL_SHADER_SOURCE)?;
    let parsed = translation_unit
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP16 railroad translation unit is empty")?;
    let effect = parsed
        .effect(crate::RAIL_SHADER_EFFECT)
        .ok_or("effective railroad.shader omits Effect railroad")?;
    if effect.vertex_shader.is_none()
        || effect.pixel_shader.is_none()
        || effect.defines != ["PDX_SNOW", "PDX_GRADIENT_BORDERS"]
    {
        return Err("effective railroad Effect stage/define contract drifted".into());
    }
    let constants =
        crate::derive_a729_engine_map_shader_constants(map_width, map_height, false, 0)?;
    let defines = crate::engine_map_shader_defines(constants);
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        RAIL_SHADER_SOURCE,
        crate::RAIL_SHADER_EFFECT,
        crate::ShaderStage::Vertex,
        &defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        RAIL_SHADER_SOURCE,
        crate::RAIL_SHADER_EFFECT,
        crate::ShaderStage::Pixel,
        &defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) = compile_rail_program_stages(&vertex_hlsl, &pixel_hlsl)?;
    let pixel_stage = crate::shader_compile::merged_effect_translation_unit_stage(
        &translation_unit,
        crate::ShaderStage::Pixel,
    );
    let owners = RAIL_USED_PIXEL_SAMPLERS
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let required = |value: &Option<String>, symbol: &str, field: &str| -> Result<String> {
        value
            .clone()
            .ok_or_else(|| format!("XMAP16 sampler {symbol} omits {field}").into())
    };
    let mut samplers = pixel_stage
        .samplers
        .iter()
        .filter_map(|sampler| {
            owners
                .get(sampler.name.as_str())
                .map(|owner| (sampler, owner))
        })
        .map(|(sampler, owner)| {
            Ok(PreparedXmap16RailSampler {
                symbol: sampler.name.clone(),
                shader_register: sampler.index,
                owner_packet: (*owner).into(),
                mag_filter: required(&sampler.mag_filter, &sampler.name, "MagFilter")?,
                min_filter: required(&sampler.min_filter, &sampler.name, "MinFilter")?,
                mip_filter: required(&sampler.mip_filter, &sampler.name, "MipFilter")?,
                address_u: required(&sampler.address_u, &sampler.name, "AddressU")?,
                address_v: required(&sampler.address_v, &sampler.name, "AddressV")?,
                sampler_type: sampler
                    .sampler_type
                    .clone()
                    .unwrap_or_else(|| "Texture2D".into()),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    samplers.sort_by_key(|sampler| sampler.shader_register);
    let ordered_shader_content = translation_unit
        .iter()
        .map(|(path, _)| path.as_str())
        .chain([crate::shader_compile::DX11_PLATFORM_HEADER])
        .map(|path| {
            admission
                .require_used_content(path)
                .map(Clone::clone)
                .map_err(|error| format!("XMAP16 railroad program requires {path}: {error}").into())
        })
        .collect::<Result<Vec<_>>>()?;
    if ordered_shader_content
        .iter()
        .any(|content| !admitted_content.contains(content))
    {
        return Err("XMAP16 railroad program escaped its admitted content closure".into());
    }
    let mut program = PreparedXmap16RailProgram {
        effect_file: RAIL_SHADER_SOURCE.into(),
        effect_name: crate::RAIL_SHADER_EFFECT.into(),
        ordered_shader_content,
        vertex_translation_unit_sha256: Sha256::hex(vertex_hlsl.as_bytes()),
        pixel_translation_unit_sha256: Sha256::hex(pixel_hlsl.as_bytes()),
        vertex_bytecode_sha256: Sha256::hex(&vertex_bytecode),
        pixel_bytecode_sha256: Sha256::hex(&pixel_bytecode),
        vertex_bytecode: vertex_bytecode.into(),
        pixel_bytecode: pixel_bytecode.into(),
        samplers,
        map_width,
        map_height,
        vertex_stride_bytes: crate::A729_RAILROAD_VERTEX_STRIDE_BYTES,
        blend_source: "src_alpha".into(),
        blend_destination: "inv_src_alpha".into(),
        color_write_mask: 0x07,
        depth_enabled: false,
        stencil_enabled: false,
        deterministic_sha256: String::new(),
    };
    program.deterministic_sha256 = rail_program_digest(&program);
    program.validate()?;
    Ok(program)
}

#[cfg(windows)]
fn compile_rail_program_stages(vertex_hlsl: &str, pixel_hlsl: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        crate::d3d_compile_stage(
            vertex_hlsl,
            crate::ShaderStage::Vertex,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
        crate::d3d_compile_stage(
            pixel_hlsl,
            crate::ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
    ))
}

#[cfg(not(windows))]
fn compile_rail_program_stages(
    _vertex_hlsl: &str,
    _pixel_hlsl: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    Err("XMAP16 executable railroad-program handoff requires Windows D3DCompile".into())
}

fn rail_program_digest(program: &PreparedXmap16RailProgram) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(program.effect_file.as_bytes());
    identity.extend_from_slice(program.effect_name.as_bytes());
    identity.extend_from_slice(program.vertex_translation_unit_sha256.as_bytes());
    identity.extend_from_slice(program.pixel_translation_unit_sha256.as_bytes());
    identity.extend_from_slice(program.vertex_bytecode_sha256.as_bytes());
    identity.extend_from_slice(program.pixel_bytecode_sha256.as_bytes());
    for content in &program.ordered_shader_content {
        identity.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        identity.extend_from_slice(content.payload_sha256.as_bytes());
    }
    for sampler in &program.samplers {
        identity.extend_from_slice(sampler.symbol.as_bytes());
        identity.extend_from_slice(&sampler.shader_register.to_le_bytes());
        identity.extend_from_slice(sampler.owner_packet.as_bytes());
        identity.extend_from_slice(sampler.mag_filter.as_bytes());
        identity.extend_from_slice(sampler.min_filter.as_bytes());
        identity.extend_from_slice(sampler.mip_filter.as_bytes());
        identity.extend_from_slice(sampler.address_u.as_bytes());
        identity.extend_from_slice(sampler.address_v.as_bytes());
        identity.extend_from_slice(sampler.sampler_type.as_bytes());
    }
    identity.extend_from_slice(&program.map_width.to_le_bytes());
    identity.extend_from_slice(&program.map_height.to_le_bytes());
    identity.extend_from_slice(&program.vertex_stride_bytes.to_le_bytes());
    identity.extend_from_slice(program.blend_source.as_bytes());
    identity.extend_from_slice(program.blend_destination.as_bytes());
    identity.push(program.color_write_mask);
    identity.push(program.depth_enabled as u8);
    identity.push(program.stencil_enabled as u8);
    Sha256::hex(&identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CounterSpriteAsset, GuiContainerLayout, GuiControlLayout, SupplyHubSpriteBinding};
    use portable_content_manifest::ContentLayerKind;

    fn route(level: u32) -> crate::rail_files::ExactRailRouteRecord {
        crate::rail_files::ExactRailRouteRecord {
            previous_province_id: 1,
            center_province_id: 2,
            next_province_id: 3,
            effective_level: level,
        }
    }

    fn vertex() -> crate::rail_files::ExactRailRibbonVertex {
        crate::rail_files::ExactRailRibbonVertex {
            position: [0.0; 4],
            uv: [0.0; 2],
        }
    }

    #[test]
    fn rail_draw_program_requires_contiguous_exact_route_ranges() {
        let records = vec![route(2), route(4)];
        let program = crate::rail_files::ExactRailRibbonProgram {
            vertices: vec![vertex(), vertex(), vertex()],
            ordered_draws: vec![
                crate::rail_files::ExactRailRibbonDraw {
                    route_record_ordinal: 0,
                    first_vertex: 0,
                    vertex_count: 1,
                    effective_level: 2,
                },
                crate::rail_files::ExactRailRibbonDraw {
                    route_record_ordinal: 1,
                    first_vertex: 1,
                    vertex_count: 2,
                    effective_level: 4,
                },
            ],
        };
        validate_rail_draw_program(&records, &program).unwrap();

        let mut gap = program.clone();
        gap.ordered_draws[1].first_vertex = 2;
        assert!(validate_rail_draw_program(&records, &gap).is_err());

        let mut wrong_level = program;
        wrong_level.ordered_draws[1].effective_level = 3;
        assert!(validate_rail_draw_program(&records, &wrong_level).is_err());
    }

    fn cutoff_defines() -> RailGeometryDefines {
        RailGeometryDefines {
            y_offset: 0.5,
            camera_cutoff: 100.0,
            camera_cutoff_speed: 2.0,
            railway_icon_cutoff: 90.0,
            supply_icon_cutoff: 80.0,
            supply_icon_unused_cutoff: 70.0,
            supply_icon_disconnected_cutoff: 60.0,
            supply_icon_ok_cutoff: 50.0,
            supply_icon_numbers_cutoff: 40.0,
            supply_icon_end_cutoff: 30.0,
            bridge_width: 1.0,
            bridge_large_width: 2.0,
            bridge_gap_width: 0.5,
            bridge_gap_large_width: 1.0,
            arrow_thin_threshold: 1.0,
            arrow_medium_threshold: 2.0,
            arrow_thick_threshold: 3.0,
            supply_icon_shift: [0.0, 0.0, 0.0],
        }
    }

    fn defines_content() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            normalized_game_relative_path: GRAPHICS_DEFINES_SOURCE.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "2".repeat(64),
            payload_sha256: "2".repeat(64),
            payload_bytes: 1,
        }
    }

    /// The DERIVED cutoff table is exactly what the frame validator demands.
    ///
    /// The table used to arrive as a `hub_effective_cutoffs` carrier no producer ever
    /// wrote, and the validator compared its words with the effective `00_graphics.lua`.
    /// Now the same words are derived from the same file, and this asserts the two sides
    /// still agree word for word rather than merely looking similar -- a derivation that
    /// read a different define would be caught here and nowhere else.
    ///
    /// kills: reading the railway icon cutoff (a separate define) as the supply icon one,
    /// and swapping any two of the six classes.
    #[test]
    fn the_derived_hub_cutoffs_are_the_effective_graphics_defines() {
        let defines = cutoff_defines();
        let derived =
            crate::xmap16_hub_derivation::derive_xmap16_hub_effective_cutoffs(
                &defines,
                defines_content(),
            )
            .expect("derived cutoffs");
        assert_eq!(derived.graphics_defines_content, defines_content());
        validate_hub_cutoffs(&derived, &BTreeMap::new(), &defines)
            .expect("the derived table must satisfy the frame validator");

        for (carried, expected) in [
            (derived.supply_icon, defines.supply_icon_cutoff),
            (derived.unused, defines.supply_icon_unused_cutoff),
            (derived.disconnected, defines.supply_icon_disconnected_cutoff),
            (derived.ok, defines.supply_icon_ok_cutoff),
            (derived.numbers, defines.supply_icon_numbers_cutoff),
            (derived.end, defines.supply_icon_end_cutoff),
        ] {
            assert_eq!(carried.value_bits, expected.to_bits());
            assert_eq!(carried.value.to_bits(), expected.to_bits());
        }
        // The railway icon cutoff is a different define and must not appear.
        assert!([
            derived.supply_icon,
            derived.unused,
            derived.disconnected,
            derived.ok,
            derived.numbers,
            derived.end,
        ]
        .iter()
        .all(|row| row.value_bits != defines.railway_icon_cutoff.to_bits()));

        let mut drifted = defines;
        drifted.supply_icon_ok_cutoff = 999.0;
        assert!(
            validate_hub_cutoffs(&derived, &BTreeMap::new(), &drifted).is_err(),
            "a cutoff table built from other defines must be refused"
        );
    }

    fn control(name: &str) -> GuiControlLayout {
        GuiControlLayout {
            kind: "iconType".into(),
            name: name.into(),
            source_file: SUPPLY_MAPICONS_GUI_SOURCE.into(),
            source_line: 1,
            sprite_type: None,
            quad_texture_sprite: None,
            parent_size: None,
            position: None,
            effective_position_f32_bits: None,
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            format: None,
            orientation: None,
            max_slots_vertical: None,
            rotation: None,
            scale: None,
            frame: None,
            center_position: None,
        }
    }

    fn binding(control_name: &str, path: &str, hash: char) -> SupplyHubSpriteBinding {
        SupplyHubSpriteBinding {
            control_name: control_name.into(),
            sprite: CounterSpriteAsset {
                name: format!("GFX_{control_name}"),
                texture_file: path.into(),
                no_of_frames: 1,
                frame_direction_y: false,
                texture_width: 36,
                texture_height: 36,
                dds_format: "BGRA8".into(),
                mip_count: 1,
                frame_width: 36,
                frame_height: 36,
                frames: Vec::new(),
                source_file: SUPPLY_GFX_SOURCE.into(),
                source_line: 1,
            },
            texture_sha256: hash.to_string().repeat(64),
            texture_bytes: 128,
        }
    }

    fn hub_content() -> SupplyHubContentIdentity {
        SupplyHubContentIdentity {
            container: GuiContainerLayout {
                name: "supply_node_map_icon".into(),
                source_file: SUPPLY_MAPICONS_GUI_SOURCE.into(),
                source_line: 1,
                position: None,
                size: None,
                controls: vec![
                    control("background_button"),
                    control("supply_hub_overlay_icon"),
                    control("supply_hub_overlay_naval"),
                ],
            },
            mapicons_gui_sha256: "1".repeat(64),
            supply_gfx_sha256: "2".repeat(64),
            background: binding("background_button", "mod/hubs/background.dds", '3'),
            common_icon: binding("supply_hub_overlay_icon", "mod/hubs/common.dds", '4'),
            naval_overlay: binding("supply_hub_overlay_naval", "mod/hubs/naval.dds", '5'),
        }
    }

    fn used(path: &str, payload_sha256: &str, payload_bytes: u64) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 1,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "fixture".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: payload_sha256.into(),
            payload_bytes,
        }
    }

    #[test]
    fn hub_closure_tracks_effective_dynamic_sprite_paths_and_seals_them() {
        let hub = hub_content();
        let mut content = vec![
            used(SUPPLY_MAPICONS_GUI_SOURCE, &hub.mapicons_gui_sha256, 10),
            used(SUPPLY_GFX_SOURCE, &hub.supply_gfx_sha256, 20),
            used(
                &hub.background.sprite.texture_file,
                &hub.background.texture_sha256,
                hub.background.texture_bytes,
            ),
            used(
                &hub.common_icon.sprite.texture_file,
                &hub.common_icon.texture_sha256,
                hub.common_icon.texture_bytes,
            ),
            used(
                &hub.naval_overlay.sprite.texture_file,
                &hub.naval_overlay.texture_sha256,
                hub.naval_overlay.texture_bytes,
            ),
        ];
        validate_hub_used_content(&content, &hub).unwrap();
        let original_digest = hub_content_digest(&hub);

        content.pop();
        assert!(validate_hub_used_content(&content, &hub).is_err());

        let mut changed = hub;
        changed.background.sprite.texture_file = "mod/hubs/replacement.dds".into();
        assert_ne!(hub_content_digest(&changed), original_digest);
    }
}
