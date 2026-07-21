//! Capture-independent D3D11 execution contract for the production map.
//!
//! This module deliberately contains no capture-bundle adapter and no asset
//! catalogue.  Static interpreters may submit a draw only after resolving all
//! of its inputs through the replay-admitted effective VFS.  A semantic content
//! binding without a complete draw descriptor is an omission, never a license
//! to invent pipeline state or pixels.

use crate::{
    production_main_scene_desc_from_outputs,
    production_map_interpreter::{materialize_static_program, unsupported_reason},
    validate_c03_camera_replay_binding, C03CameraReplayBinding, ExactRailD80Binding,
    PreparedProductionMapFrame,
    PreparedRuntimeDraw, ProductionMainSceneTargetDesc,
    ProductionMapResource, Result,
};
use replay_core::{
    BufferSource, C07TreeRuntimeCarrier, ConstantBufferBinding, M03ArrowDraw, M03ArrowFrame,
    MapnameD80State, RenderExecutionRef, RenderTargetDesc, RenderTargetInitialSource, StaticGeometrySource, StaticMapCommand,
    StaticMapOperation, StaticMapPassProgram, StaticShaderResourceSource, UsedContentKey,
    RailSupplyArrowFrame,
    ViewStatus,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;

pub type ProductionD3dStaticPassProgram = StaticMapPassProgram;
pub const PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA: &str =
    "hoi4.production_d3d_execution.v7";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dOmission {
    pub pass: String,
    pub reason: String,
}

/// GPU proof accumulated by the capture-independent product executor while it
/// binds H to every exact selected-submit SnowMudTexture consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnowMudD3dExecutionProof {
    /// Exact tight BGRA8 bytes copied from the mapped replay staging texture.
    /// This is retained only in explicit H2 evidence mode; a hash alone cannot
    /// serve as the raw replay-consumer observation required by G3.
    pub tight_readback_bytes: Vec<u8>,
    pub readback_sha256: String,
    pub mapped_row_pitch: u32,
    pub verified_row_count: u32,
    pub verified_tight_byte_count: u64,
    pub replaced_binding_count: u32,
}

/// Product-native proof that the exact occupation secondary packet was
/// uploaded, read back byte-for-byte, and bound to both reflected consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationSecondaryD3dExecutionProof {
    pub tight_readback_bytes: Vec<u8>,
    pub readback_sha256: String,
    pub mapped_row_pitch: u32,
    pub verified_row_count: u32,
    pub verified_tight_byte_count: u64,
    pub terrain_t6_binding_count: u32,
    pub water_t3_binding_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedProductionD3dFrame {
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    /// C03 receipt binding only. Exact D3D draw state already contains the
    /// recorded camera effects, so execution must not reapply raw events.
    pub c03_camera_replay: Option<C03CameraReplayBinding>,
    pub target: Option<ProductionMainSceneTargetDesc>,
    pub ordered_scene_constant_buffers: Vec<ConstantBufferBinding>,
    pub ordered_render_operations: Vec<RenderExecutionRef>,
    pub snow_mud_texture: Option<replay_core::SnowMudTexturePacket>,
    pub political_textures: Vec<replay_core::PoliticalTexturePacket>,
    pub tiled_lights: Vec<replay_core::TiledLightSubmission>,
    pub ordered_static_used_content: Vec<UsedContentKey>,
    pub ordered_static_programs: Vec<StaticMapPassProgram>,
    /// Exact D81 semantic binding to the retained D80 railroad operations.
    pub rail_ribbon: Option<ExactRailD80Binding>,
    /// B10 compatibility/admission state of the selected submit's D80 mapname
    /// operations. Legacy draws are omitted before this frame can execute.
    pub mapname_d80_state: MapnameD80State,
    /// Already-admitted C07 join. Every referenced operation must survive
    /// materialization and execute through the ordinary static draw bridge.
    pub tree_runtime: Option<C07TreeRuntimeCarrier>,
    /// Preparation-time seal over the exact materialized C07 operations and
    /// their positions in the unified execution stream.
    pub tree_materialized_execution_sha256: Option<String>,
    /// Exact S1 law/pass plan sealed to the materialized unified operations.
    /// `None` is an explicit feature omission, never a neutral-shadow fallback.
    pub s1_shadow: Option<PreparedS1ShadowExecution>,
    pub ordered_runtime_draws: Vec<PreparedRuntimeDraw>,
    pub m02_combat: Option<replay_core::M02CombatFrame>,
    pub authenticated_ui_pass: Option<replay_core::AuthenticatedUiPass>,
    pub m03_arrows: Option<M03ArrowFrame>,
    /// Exact semantic join to retained static normal/see-through railway
    /// supply-map draws. Execution remains in the ordinary static stream.
    pub rail_supply_arrows: Option<RailSupplyArrowFrame>,
    pub omissions: Vec<ProductionD3dOmission>,
}

/// Exact precedence contract between the opaque D80 operation stream and the
/// typed semantic building state exposed by D48.
///
/// `TypedSemanticStateOnly` is deliberately not pixel authority. It means the
/// current world frame can answer building-state queries while visible object
/// rendering remains omitted until an authenticated semantic producer joins
/// those rows to exact operations. An executable D80 stream always wins as a
/// complete ordered map program; pass tokens/content names are never used to
/// classify a building subpass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildingPresentationPriority {
    AuthenticatedD80Operations,
    TypedSemanticStateOnly,
    Unavailable,
}

pub fn has_authenticated_d80_map_operations(frame: &PreparedProductionD3dFrame) -> bool {
    frame.target.is_some()
        && (!frame.ordered_static_programs.is_empty()
            || !frame.ordered_runtime_draws.is_empty()
            || frame
                .m03_arrows
                .as_ref()
                .is_some_and(|m03| !m03.draws.is_empty()))
}

pub fn building_presentation_priority(
    frame: Option<&PreparedProductionD3dFrame>,
    buildings_status: ViewStatus,
) -> BuildingPresentationPriority {
    if frame.is_some_and(has_authenticated_d80_map_operations) {
        BuildingPresentationPriority::AuthenticatedD80Operations
    } else if buildings_status == ViewStatus::Present {
        BuildingPresentationPriority::TypedSemanticStateOnly
    } else {
        BuildingPresentationPriority::Unavailable
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dExecutedDraw {
    pub global_ordinal: u32,
    pub pass: String,
    pub draw_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dExecutionReceipt {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub c03_camera_replay: Option<C03CameraReplayBinding>,
    /// Sealed pointer-free C07 coverage/pass/bucket identity actually admitted
    /// by this execution. `None` means tree runtime was unavailable/omitted.
    pub tree_runtime_payload_sha256: Option<String>,
    /// Exact selected-frame C07 operation payload/order seal validated again
    /// immediately before backend execution.
    pub tree_materialized_execution_sha256: Option<String>,
    /// Sealed pointer-free S1 shadow execution identity admitted by this
    /// execution. This authenticates the carrier; it is not a pixel oracle.
    pub s1_shadow_payload_sha256: Option<String>,
    /// Exact seal over the materialized S1 operation references and payloads
    /// that passed the pre-execution validation. `None` is valid only for an
    /// absent or exact zero-fade S1 carrier.
    pub s1_materialized_execution_sha256: Option<String>,
    /// Distinguishes no carried mapname draw, a legacy draw omitted for lack of
    /// the authenticated CB1 semantic, and authenticated executed authority.
    pub mapname_d80_state: MapnameD80State,
    pub target: Option<ProductionMainSceneTargetDesc>,
    pub ordered_draws: Vec<ProductionD3dExecutedDraw>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub omissions: Vec<ProductionD3dOmission>,
    pub deterministic_sha256: String,
}

/// Backend-neutral ordered execution boundary.  The Windows implementation can
/// bind these calls to D3D11 while headless tests use a deterministic sink.
pub trait ProductionD3dExecutionBridge {
    fn begin_frame(&mut self, frame: &PreparedProductionD3dFrame) -> Result<()>;
    fn execute_static_operation(
        &mut self,
        global_ordinal: u32,
        pass: &StaticMapPassProgram,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        operation: &StaticMapOperation,
    ) -> Result<()>;
    fn execute_runtime_draw(
        &mut self,
        global_ordinal: u32,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        draw: &PreparedRuntimeDraw,
    ) -> Result<()>;
    fn execute_m03_draw(
        &mut self,
        global_ordinal: u32,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        draw: &M03ArrowDraw,
    ) -> Result<()>;
    fn finish_frame(&mut self) -> Result<()>;
}

fn validate_provenance_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

/// Join the production-map plan with independently generated static-pass
/// programs. Missing programs remain feature-local omissions. Present programs
/// fail closed on any order, descriptor, byte-range, or content-identity error.
pub fn prepare_production_d3d_frame(
    frame: &PreparedProductionMapFrame,
    resources: &BTreeMap<String, ProductionMapResource>,
) -> Result<PreparedProductionD3dFrame> {
    validate_provenance_sha256(&frame.content_manifest_sha256, "content manifest digest")?;
    validate_provenance_sha256(&frame.product_modset_sha256, "product modset digest")?;
    validate_provenance_sha256(
        &frame.effective_vfs_manifest_sha256,
        "effective VFS manifest digest",
    )?;
    if let Some(binding) = &frame.c03_camera_replay {
        validate_c03_camera_replay_binding(binding)?;
        if frame.selected_submit_index != Some(binding.submit_index)
            || frame.selected_submit_global_order != Some(binding.submit_global_order)
        {
            return Err("C03 camera replay binding belongs to a different selected submit".into());
        }
    }
    let mut ordered_static_programs = Vec::new();
    let mut omissions = Vec::new();
    let mut mapname_d80_state = MapnameD80State::Missing;
    if frame.m02_combat.is_some() && frame.authenticated_ui_pass.is_none() {
        omissions.push(ProductionD3dOmission {
            pass: "m02_combat_indicators".into(),
            reason: "exact M02 swapchain draws are carried, but the complete authenticated selected-Present UI-pass operation stream is absent; the entire M02 pixel route is omitted".into(),
        });
    }
    if frame.s1_shadow.is_none() {
        omissions.push(ProductionD3dOmission {
            pass: "c07_s1_shadow".into(),
            reason: "selected submit has no authenticated portable S1 carrier; light-depth, terrain-unlit, blur, and shadow-consumer execution remains omitted".into(),
        });
    }
    let mut omitted_static_ordinals = std::collections::BTreeSet::new();
    for program in &frame.ordered_static_map_programs {
        let program_mapname_state = program.mapname_d80_state()?;
        mapname_d80_state = mapname_d80_state.merge(program_mapname_state)?;
        let mut seen = std::collections::BTreeSet::new();
        if program
            .ordered_content_keys
            .iter()
            .any(|key| !seen.insert(key))
        {
            return Err("opaque D3D program content keys are duplicated".into());
        }
        if program.ordered_operations.is_empty() {
            omissions.push(ProductionD3dOmission {
                pass: program.pass_token.clone(),
                reason: "typed static-pass program contains no authenticated draws".into(),
            });
            continue;
        }
        if frame.tree_runtime.is_none()
            && program.ordered_operations.iter().any(|operation| {
                matches!(
                    &operation.command,
                    StaticMapCommand::Draw { draw }
                        if C07TreeRuntimeCarrier::is_retained_tree_shader_draw(draw)
                )
            })
        {
            omitted_static_ordinals.extend(
                program
                    .ordered_operations
                    .iter()
                    .map(|operation| operation.operation_ordinal),
            );
            omissions.push(ProductionD3dOmission {
                pass: program.pass_token.clone(),
                reason: "authenticated a729 tree shader program omitted because the selected submit has no C07 runtime visibility/model/LOD carrier".into(),
            });
            continue;
        }
        if program_mapname_state == MapnameD80State::LegacyUnstamped {
            omitted_static_ordinals.extend(
                program
                    .ordered_operations
                    .iter()
                    .map(|operation| operation.operation_ordinal),
            );
            omissions.push(ProductionD3dOmission {
                pass: program.pass_token.clone(),
                reason: "B10 legacy mapname program omitted: the draw lacks the authenticated reflected Transp_OffsetX CB1 semantic; reconstructed labels and slot/size inference are forbidden".into(),
            });
            continue;
        }
        let program = match materialize_static_program(program, resources) {
            Ok(program) => program,
            Err(error) => {
                if let Some(reason) = unsupported_reason(&error) {
                    if program_mapname_state == MapnameD80State::Authenticated {
                        return Err(format!(
                            "authenticated mapname program cannot be omitted during materialization: {reason}"
                        )
                        .into());
                    }
                    if frame.rail_ribbon.as_ref().is_some_and(|rail| {
                        program.ordered_operations.iter().any(|operation| {
                            rail.ordered_draws.iter().any(|binding| {
                                binding.operation_ordinal == operation.operation_ordinal
                            })
                        })
                    }) {
                        return Err(format!(
                            "authenticated D81/D80 railroad program cannot be omitted during materialization: {reason}"
                        )
                        .into());
                    }
                    if frame.rail_supply_arrows.as_ref().is_some_and(|rail| {
                        program.ordered_operations.iter().any(|operation| {
                            rail.ordered_draws.iter().any(|binding| {
                                binding.operation_ordinal == operation.operation_ordinal
                            })
                        })
                    }) {
                        return Err(format!(
                            "authenticated railway supply-map arrow program cannot be omitted during materialization: {reason}"
                        )
                        .into());
                    }
                    omitted_static_ordinals.extend(
                        program
                            .ordered_operations
                            .iter()
                            .map(|operation| operation.operation_ordinal),
                    );
                    omissions.push(ProductionD3dOmission {
                        pass: program.pass_token.clone(),
                        reason,
                    });
                    continue;
                }
                return Err(error.into());
            }
        };
        ordered_static_programs.push(program);
    }

    let mut omit_m03 = false;
    loop {
        let mut changed = false;
        if !omit_m03
            && frame.m03_arrows.as_ref().is_some_and(|m03| {
                m03.draws.iter().any(|packet| {
                    static_draw_omitted_dependency(&packet.draw, &omitted_static_ordinals, false)
                        .is_some()
                })
            })
        {
            omit_m03 = true;
            changed = true;
            omissions.push(ProductionD3dOmission {
                pass: "m03_order_movement_arrows".into(),
                reason: "M03 consumes a render-graph resource whose authenticated static producer was omitted; the entire dependent M03 feature is transitively omitted".into(),
            });
        }

        let mut retained = Vec::with_capacity(ordered_static_programs.len());
        for program in ordered_static_programs {
            if let Some(dependency) =
                static_program_omitted_dependency(&program, &omitted_static_ordinals, omit_m03)
            {
                omitted_static_ordinals.extend(
                    program
                        .ordered_operations
                        .iter()
                        .map(|operation| operation.operation_ordinal),
                );
                omissions.push(ProductionD3dOmission {
                    pass: program.pass_token.clone(),
                    reason: format!(
                        "static pass consumes {dependency}, whose authenticated producer was omitted; this dependent pass is transitively omitted"
                    ),
                });
                changed = true;
            } else {
                retained.push(program);
            }
        }
        ordered_static_programs = retained;
        if !changed {
            break;
        }
    }

    let retained_mapname_state = ordered_static_programs.iter().try_fold(
        MapnameD80State::Missing,
        |state, program| state.merge(program.mapname_d80_state()?),
    )?;
    match (mapname_d80_state, retained_mapname_state) {
        (MapnameD80State::Missing, MapnameD80State::Missing)
        | (MapnameD80State::LegacyUnstamped, MapnameD80State::Missing)
        | (MapnameD80State::Authenticated, MapnameD80State::Authenticated) => {}
        (MapnameD80State::Authenticated, _) => {
            return Err("authenticated mapname program was omitted before production execution".into())
        }
        _ => return Err("prepared mapname D80 state disagrees with retained programs".into()),
    }

    let mut ordered_static_used_content = Vec::new();
    let mut output_states = Vec::new();
    for program in &ordered_static_programs {
        let expected_consumer = format!("production_map.static_program/{}", program.payload_sha256);
        for key in &program.ordered_content_keys {
            let resource = resources
                .get(key)
                .ok_or_else(|| format!("opaque program references absent content key {key}"))?;
            ordered_static_used_content.push(resource.content.clone());
        }
        for operation in &program.ordered_operations {
            validate_static_operation(operation, resources, &expected_consumer)?;
            if let StaticMapCommand::Draw { draw } = &operation.command {
                output_states.push(draw.output_state.clone());
            }
        }
    }
    let m03_arrows = if omit_m03 {
        None
    } else {
        frame.m03_arrows.clone()
    };
    if let Some(m03) = &m03_arrows {
        for content in &m03.used_content {
            ordered_static_used_content.push(content.clone());
        }
        output_states.extend(m03.draws.iter().map(|draw| draw.draw.output_state.clone()));
    }
    let rail_supply_arrows = if let Some(rail) = &frame.rail_supply_arrows {
        rail.validate()
            .map_err(|error| format!("invalid rail supply-arrow carrier: {error}"))?;
        rail.validate_against_static_programs(&frame.ordered_static_map_programs)
            .map_err(|error| format!("rail supply-arrow source join is invalid: {error}"))?;
        rail.validate_against_static_programs(&ordered_static_programs)
            .map_err(|error| {
                format!(
                    "authenticated railway supply-map arrow program cannot be partially omitted: {error}"
                )
            })?;
        Some(rail.clone())
    } else {
        None
    };
    output_states.extend(frame.ordered_runtime_draws.iter().map(|draw| match draw {
        PreparedRuntimeDraw::Particle { packet, .. } => packet.output_state.clone(),
        PreparedRuntimeDraw::Entity { submission, .. }
        | PreparedRuntimeDraw::O04City { submission, .. }
        | PreparedRuntimeDraw::O05Building { submission, .. } => submission.output_state.clone(),
    }));
    let target = production_main_scene_desc_from_outputs(output_states.iter())?;
    if let Some(tree_runtime) = &frame.tree_runtime {
        validate_c07_executor_operations(
            tree_runtime,
            &ordered_static_programs,
            &frame.ordered_render_operations,
        )?;
    }

    let mut prepared = PreparedProductionD3dFrame {
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        product_modset_sha256: frame.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
        c03_camera_replay: frame.c03_camera_replay.clone(),
        target,
        ordered_scene_constant_buffers: frame.ordered_scene_constant_buffers.clone(),
        ordered_render_operations: frame
            .ordered_render_operations
            .iter()
            .filter(|reference| match reference {
                RenderExecutionRef::Static { operation_ordinal } => {
                    !omitted_static_ordinals.contains(operation_ordinal)
                }
                RenderExecutionRef::Runtime { .. } => true,
                RenderExecutionRef::M03 { .. } => !omit_m03,
                // M02 belongs to the complete swapchain UI pass. Until the
                // product carries every intervening UI ordinal, executing it
                // here would place it incorrectly after a flattened HUD.
                RenderExecutionRef::M02 { .. } => false,
            })
            .cloned()
            .collect(),
        snow_mud_texture: frame.snow_mud_texture.clone(),
        political_textures: frame.political_textures.clone(),
        tiled_lights: frame.tiled_lights.clone(),
        ordered_static_used_content,
        ordered_static_programs,
        rail_ribbon: frame.rail_ribbon.clone(),
        mapname_d80_state,
        tree_runtime: frame.tree_runtime.clone(),
        tree_materialized_execution_sha256: None,
        s1_shadow: None,
        ordered_runtime_draws: frame.ordered_runtime_draws.clone(),
        m02_combat: frame.m02_combat.clone(),
        authenticated_ui_pass: frame.authenticated_ui_pass.clone(),
        m03_arrows,
        rail_supply_arrows,
        omissions,
    };
    if let Some(carrier) = &frame.s1_shadow {
        prepared.s1_shadow = Some(prepare_s1_shadow_execution(carrier, &prepared)?);
    }
    prepared.tree_materialized_execution_sha256 = prepared
        .tree_runtime
        .as_ref()
        .map(|carrier| {
            c07_materialized_execution_sha256(
                carrier,
                &prepared.ordered_static_programs,
                &prepared.ordered_render_operations,
            )
        })
        .transpose()?;
    validate_exact_rail_d80_execution(&prepared)?;
    if let Some(rail) = &prepared.rail_supply_arrows {
        rail.validate_against_static_programs(&prepared.ordered_static_programs)
            .map_err(|error| format!("prepared rail supply-arrow join is invalid: {error}"))?;
    }
    Ok(prepared)
}

fn validate_exact_rail_d80_execution(frame: &PreparedProductionD3dFrame) -> Result<()> {
    let Some(binding) = &frame.rail_ribbon else {
        return Ok(());
    };
    if binding.authority != "exact_a729_d81_d80_railroad_registration_join"
        || binding.ordered_draws.is_empty()
    {
        return Err("D81/D80 railroad binding authority is incomplete".into());
    }
    let mut operations = BTreeMap::new();
    for program in &frame.ordered_static_programs {
        for operation in &program.ordered_operations {
            operations.insert(operation.operation_ordinal, (program, operation));
        }
    }
    for expected in &binding.ordered_draws {
        let (program, operation) = operations
            .get(&expected.operation_ordinal)
            .copied()
            .ok_or("D81/D80 railroad binding names an omitted static operation")?;
        if program.program_ordinal != expected.program_ordinal {
            return Err("D81/D80 railroad binding program ordinal drifted".into());
        }
        let StaticMapCommand::Draw { draw } = &operation.command else {
            return Err("D81/D80 railroad binding names a non-draw operation".into());
        };
        let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
            return Err("D81/D80 railroad binding lost exact engine geometry".into());
        };
        let Some(vertex) = geometry.ordered_vertex_buffers.first() else {
            return Err("D81/D80 railroad binding lost its vertex buffer".into());
        };
        let BufferSource::Inline { bytes, sha256 } = &vertex.source else {
            return Err("D81/D80 railroad binding vertex source is no longer inline".into());
        };
        if draw.payload_sha256 != expected.draw_payload_sha256
            || sha256 != &expected.vertex_payload_sha256
            || u64::try_from(bytes.len()).ok() != Some(expected.vertex_byte_count)
        {
            return Err("D81/D80 railroad binding payload identity drifted".into());
        }
        let occurrences = frame
            .ordered_render_operations
            .iter()
            .filter(|reference| {
                matches!(
                    reference,
                    RenderExecutionRef::Static { operation_ordinal }
                        if *operation_ordinal == expected.operation_ordinal
                )
            })
            .count();
        if occurrences != 1 {
            return Err("D81/D80 railroad draw lacks one exact global interleave position".into());
        }
    }
    Ok(())
}

pub fn execute_production_d3d_frame(
    frame: &PreparedProductionD3dFrame,
    bridge: &mut impl ProductionD3dExecutionBridge,
) -> Result<ProductionD3dExecutionReceipt> {
    validate_provenance_sha256(&frame.content_manifest_sha256, "content manifest digest")?;
    validate_provenance_sha256(&frame.product_modset_sha256, "product modset digest")?;
    validate_provenance_sha256(
        &frame.effective_vfs_manifest_sha256,
        "effective VFS manifest digest",
    )?;
    validate_exact_rail_d80_execution(frame)?;
    if let Some(rail) = &frame.rail_supply_arrows {
        rail.validate_against_static_programs(&frame.ordered_static_programs)
            .map_err(|error| format!("executable rail supply-arrow join is invalid: {error}"))?;
    }
    let retained_mapname_state = frame.ordered_static_programs.iter().try_fold(
        MapnameD80State::Missing,
        |state, program| state.merge(program.mapname_d80_state()?),
    )?;
    let legacy_omission = frame.omissions.iter().any(|omission| {
        omission
            .reason
            .starts_with("B10 legacy mapname program omitted:")
    });
    match (frame.mapname_d80_state, retained_mapname_state) {
        (MapnameD80State::Missing, MapnameD80State::Missing) if !legacy_omission => {}
        (MapnameD80State::LegacyUnstamped, MapnameD80State::Missing) if legacy_omission => {}
        (MapnameD80State::Authenticated, MapnameD80State::Authenticated)
            if !legacy_omission => {}
        _ => {
            return Err(
                "prepared mapname D80 state/omission disagrees with executable programs".into(),
            )
        }
    }
    if let Some(binding) = &frame.c03_camera_replay {
        validate_c03_camera_replay_binding(binding)?;
    }
    if let Some(s1_shadow) = &frame.s1_shadow {
        validate_s1_shadow_execution(s1_shadow, frame)?;
    }
    match (
        &frame.tree_runtime,
        &frame.tree_materialized_execution_sha256,
    ) {
        (Some(tree_runtime), Some(expected)) => {
            let actual = c07_materialized_execution_sha256(
                tree_runtime,
                &frame.ordered_static_programs,
                &frame.ordered_render_operations,
            )?;
            if &actual != expected {
                return Err("prepared C07 materialized execution stream drifted".into());
            }
        }
        (None, None) => {}
        _ => return Err("prepared C07 carrier/execution seal nullability mismatch".into()),
    }
    bridge.begin_frame(frame)?;
    let mut ordinal = 0u32;
    let mut ordered_draws = Vec::new();
    let mut used = BTreeMap::<String, UsedContentKey>::new();
    for content in &frame.ordered_static_used_content {
        insert_content(&mut used, content);
    }
    if frame.ordered_render_operations.is_empty() {
        if !frame.ordered_static_programs.is_empty() {
            return Err(
                "static map programs have no authoritative cross-family execution order".into(),
            );
        }
        if frame
            .m03_arrows
            .as_ref()
            .is_some_and(|m03| !m03.draws.is_empty())
        {
            return Err(
                "M03 arrow draws have no authoritative cross-family execution order".into(),
            );
        }
        for draw in &frame.ordered_runtime_draws {
            bridge.execute_runtime_draw(ordinal, &frame.ordered_scene_constant_buffers, draw)?;
            collect_runtime_draw_content(draw, &mut used);
            let pass = runtime_draw_pass(draw);
            ordered_draws.push(ProductionD3dExecutedDraw {
                global_ordinal: ordinal,
                pass: pass.into(),
                draw_token: runtime_draw_token(draw),
            });
            ordinal = ordinal
                .checked_add(1)
                .ok_or("production draw ordinal overflow")?;
        }
    } else {
        let static_count = frame
            .ordered_static_programs
            .iter()
            .map(|program| program.ordered_operations.len())
            .sum::<usize>();
        let m03_count = frame.m03_arrows.as_ref().map_or(0, |m03| m03.draws.len());
        if frame.ordered_render_operations.len()
            != static_count + frame.ordered_runtime_draws.len() + m03_count
        {
            return Err(
                "cross-family execution order does not cover every static/runtime operation".into(),
            );
        }
        let mut static_operations = BTreeMap::new();
        for program in &frame.ordered_static_programs {
            for operation in &program.ordered_operations {
                if static_operations
                    .insert(operation.operation_ordinal, (program, operation))
                    .is_some()
                {
                    return Err("static map programs repeat an operation ordinal".into());
                }
            }
        }
        let mut runtime_draws = BTreeMap::new();
        for draw in &frame.ordered_runtime_draws {
            let (PreparedRuntimeDraw::Particle { ordered_draw, .. }
            | PreparedRuntimeDraw::Entity { ordered_draw, .. }
            | PreparedRuntimeDraw::O04City { ordered_draw, .. }
            | PreparedRuntimeDraw::O05Building { ordered_draw, .. }) = draw;
            if runtime_draws.insert(ordered_draw, draw).is_some() {
                return Err("runtime draws repeat an ordered-draw reference".into());
            }
        }
        let mut seen_static = std::collections::BTreeSet::new();
        let mut seen_runtime = std::collections::BTreeSet::new();
        let mut seen_m03 = std::collections::BTreeSet::new();
        for reference in &frame.ordered_render_operations {
            match reference {
                RenderExecutionRef::Static { operation_ordinal } => {
                    if !seen_static.insert(*operation_ordinal) {
                        return Err("cross-family order repeats a static operation".into());
                    }
                    let (program, operation) = static_operations
                        .get(operation_ordinal)
                        .copied()
                        .ok_or("cross-family order references an absent static operation")?;
                    bridge.execute_static_operation(
                        ordinal,
                        program,
                        &frame.ordered_scene_constant_buffers,
                        operation,
                    )?;
                    collect_static_operation_content(operation, &mut used);
                    ordered_draws.push(ProductionD3dExecutedDraw {
                        global_ordinal: ordinal,
                        pass: program.pass_token.clone(),
                        draw_token: static_operation_token(operation),
                    });
                }
                RenderExecutionRef::Runtime { output } => {
                    if !seen_runtime.insert(output) {
                        return Err("cross-family order repeats a runtime draw".into());
                    }
                    let draw = *runtime_draws
                        .get(output)
                        .ok_or("cross-family order references an absent runtime draw")?;
                    bridge.execute_runtime_draw(
                        ordinal,
                        &frame.ordered_scene_constant_buffers,
                        draw,
                    )?;
                    collect_runtime_draw_content(draw, &mut used);
                    let pass = runtime_draw_pass(draw);
                    ordered_draws.push(ProductionD3dExecutedDraw {
                        global_ordinal: ordinal,
                        pass: pass.into(),
                        draw_token: runtime_draw_token(draw),
                    });
                }
                RenderExecutionRef::M02 { .. } => {
                    return Err(
                        "M02 overlay reached map/runtime execution without complete UI-pass interleave"
                            .into(),
                    );
                }
                RenderExecutionRef::M03 { output } => {
                    if !seen_m03.insert(output.clone()) {
                        return Err("cross-family order repeats an M03 draw".into());
                    }
                    let draw = frame
                        .m03_arrows
                        .as_ref()
                        .and_then(|m03| m03.draw(output))
                        .ok_or("cross-family order references an absent M03 draw")?;
                    bridge.execute_m03_draw(
                        ordinal,
                        &frame.ordered_scene_constant_buffers,
                        draw,
                    )?;
                    collect_m03_draw_content(draw, &mut used);
                    ordered_draws.push(ProductionD3dExecutedDraw {
                        global_ordinal: ordinal,
                        pass: "m03_order_movement_arrows".into(),
                        draw_token: draw.draw.draw_token.clone(),
                    });
                }
            }
            ordinal = ordinal
                .checked_add(1)
                .ok_or("production draw ordinal overflow")?;
        }
        if seen_static.len() != static_count
            || seen_runtime.len() != frame.ordered_runtime_draws.len()
            || seen_m03.len() != m03_count
        {
            return Err("cross-family execution order omits a static/runtime operation".into());
        }
    }
    bridge.finish_frame()?;

    let ordered_used_content = used.into_values().collect::<Vec<_>>();
    let mut receipt = ProductionD3dExecutionReceipt {
        schema: PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA.into(),
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        product_modset_sha256: frame.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
        c03_camera_replay: frame.c03_camera_replay.clone(),
        tree_runtime_payload_sha256: frame
            .tree_runtime
            .as_ref()
            .map(|carrier| carrier.payload_sha256.clone()),
        tree_materialized_execution_sha256: frame
            .tree_materialized_execution_sha256
            .clone(),
        s1_shadow_payload_sha256: frame
            .s1_shadow
            .as_ref()
            .map(|prepared| prepared.carrier.payload_sha256.clone()),
        s1_materialized_execution_sha256: frame
            .s1_shadow
            .as_ref()
            .and_then(|prepared| prepared.materialized_execution_sha256.clone()),
        mapname_d80_state: frame.mapname_d80_state,
        target: frame.target.clone(),
        ordered_draws,
        ordered_used_content,
        omissions: frame.omissions.clone(),
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
    Ok(receipt)
}

fn validate_c07_executor_operations(
    carrier: &C07TreeRuntimeCarrier,
    programs: &[StaticMapPassProgram],
    ordered_operations: &[RenderExecutionRef],
) -> Result<()> {
    let available = programs
        .iter()
        .flat_map(|program| {
            program
                .ordered_operations
                .iter()
                .map(|operation| operation.operation_ordinal)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut execution_positions = BTreeMap::new();
    for (position, reference) in ordered_operations.iter().enumerate() {
        let RenderExecutionRef::Static { operation_ordinal } = reference else {
            continue;
        };
        if execution_positions
            .insert(*operation_ordinal, position)
            .is_some()
        {
            return Err("C07 materialized execution order repeats a static operation".into());
        }
    }
    let required = carrier.required_operation_ordinals();
    let mut unique = std::collections::BTreeSet::new();
    let mut previous_position = None;
    for operation_ordinal in required {
        if !unique.insert(operation_ordinal) || !available.contains(&operation_ordinal) {
            return Err("C07 tree operation was duplicated or omitted during D3D preparation".into());
        }
        let position = *execution_positions
            .get(&operation_ordinal)
            .ok_or("C07 tree operation is absent from executor order")?;
        if previous_position.is_some_and(|previous| position <= previous) {
            return Err("C07 tree executor order differs from the retained three-pass contract".into());
        }
        previous_position = Some(position);
    }
    Ok(())
}

#[derive(Serialize)]
struct C07MaterializedExecutionRow<'a> {
    operation_ordinal: u32,
    execution_position: usize,
    operation: &'a StaticMapOperation,
}

fn c07_materialized_execution_sha256(
    carrier: &C07TreeRuntimeCarrier,
    programs: &[StaticMapPassProgram],
    ordered_operations: &[RenderExecutionRef],
) -> Result<String> {
    if carrier.computed_payload_sha256()? != carrier.payload_sha256 {
        return Err("prepared C07 carrier payload SHA-256 drifted after admission".into());
    }
    validate_c07_executor_operations(carrier, programs, ordered_operations)?;
    let mut materialized = BTreeMap::new();
    for operation in programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
    {
        if materialized
            .insert(operation.operation_ordinal, operation)
            .is_some()
        {
            return Err("C07 materialized programs repeat an operation ordinal".into());
        }
    }
    let execution_positions = ordered_operations
        .iter()
        .enumerate()
        .filter_map(|(position, reference)| match reference {
            RenderExecutionRef::Static { operation_ordinal } => {
                Some((*operation_ordinal, position))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let rows = carrier
        .required_operation_ordinals()
        .into_iter()
        .map(|operation_ordinal| {
            Ok(C07MaterializedExecutionRow {
                operation_ordinal,
                execution_position: *execution_positions
                    .get(&operation_ordinal)
                    .ok_or("C07 materialized operation has no execution position")?,
                operation: materialized
                    .get(&operation_ordinal)
                    .copied()
                    .ok_or("C07 materialized operation payload is absent")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Sha256::hex(&serde_json::to_vec(&rows)?))
}

fn collect_m03_draw_content(packet: &M03ArrowDraw, out: &mut BTreeMap<String, UsedContentKey>) {
    let operation = StaticMapOperation {
        operation_ordinal: packet.engine_draw_ordinal,
        payload_sha256: packet.draw.payload_sha256.clone(),
        command: StaticMapCommand::Draw {
            draw: packet.draw.clone(),
        },
    };
    collect_static_operation_content(&operation, out);
}

fn static_program_omitted_dependency(
    program: &StaticMapPassProgram,
    omitted_static_ordinals: &std::collections::BTreeSet<u32>,
    m03_omitted: bool,
) -> Option<String> {
    program.ordered_operations.iter().find_map(|operation| {
        let StaticMapCommand::Draw { draw } = &operation.command else {
            return None;
        };
        static_draw_omitted_dependency(draw, omitted_static_ordinals, m03_omitted)
    })
}

fn static_draw_omitted_dependency(
    draw: &replay_core::StaticMapDraw,
    omitted_static_ordinals: &std::collections::BTreeSet<u32>,
    m03_omitted: bool,
) -> Option<String> {
    draw.ordered_shader_resources.iter().find_map(|binding| {
        let StaticShaderResourceSource::PreparedTarget { producer, .. } = &binding.source else {
            return None;
        };
        match producer {
            RenderExecutionRef::Static { operation_ordinal }
                if omitted_static_ordinals.contains(operation_ordinal) =>
            {
                Some(format!("omitted static operation {operation_ordinal}"))
            }
            RenderExecutionRef::M03 { output } if m03_omitted => Some(format!(
                "omitted M03 draw {}",
                serde_json::to_string(output).expect("M03ArrowDrawRef serialization cannot fail")
            )),
            _ => None,
        }
    })
}

fn validate_static_operation(
    operation: &StaticMapOperation,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<()> {
    let StaticMapCommand::Draw { draw } = &operation.command else {
        let target = match &operation.command {
            StaticMapCommand::ClearColor { target, .. }
            | StaticMapCommand::ClearDepthStencil { target, .. } => target,
            StaticMapCommand::Draw { .. } => unreachable!(),
        };
        validate_render_target_content(target, resources, expected_consumer)?;
        return Ok(());
    };
    if draw.draw_token.trim().is_empty() {
        return Err("production D3D draw token is empty".into());
    }
    validate_shader_payload(
        &draw.shader_program.compiled_vertex_shader.bytecode,
        &draw.shader_program.compiled_vertex_shader.sha256,
        "vertex shader",
    )?;
    validate_shader_payload(
        &draw.shader_program.compiled_pixel_shader.bytecode,
        &draw.shader_program.compiled_pixel_shader.sha256,
        "pixel shader",
    )?;
    for (label, stage) in [
        (
            "geometry shader",
            draw.shader_program.geometry_shader.as_ref(),
        ),
        ("hull shader", draw.shader_program.hull_shader.as_ref()),
        ("domain shader", draw.shader_program.domain_shader.as_ref()),
        (
            "compute shader",
            draw.shader_program.compute_shader.as_ref(),
        ),
    ] {
        if let Some(stage) = stage {
            validate_shader_payload(&stage.compiled.bytecode, &stage.compiled.sha256, label)?;
        }
    }
    require_content(&draw.shader_program.source, resources)?;
    for binding in &draw.ordered_shader_resources {
        if let StaticShaderResourceSource::UsedContent {
            content, resource, ..
        } = &binding.source
        {
            require_content(content, resources)?;
            for subresource in &resource.ordered_subresources {
                validate_buffer_source(&subresource.source, resources)?;
            }
        }
    }
    match &draw.geometry {
        StaticGeometrySource::Exact { geometry } => {
            for vertex in &geometry.ordered_vertex_buffers {
                validate_buffer_source(&vertex.source, resources)?;
            }
            if let Some(index) = &geometry.index_buffer {
                validate_buffer_source(&index.source, resources)?;
            }
        }
        StaticGeometrySource::InterpreterOutput {
            ordered_input_content_keys,
            ..
        } => {
            for key in ordered_input_content_keys {
                if !resources.contains_key(key) {
                    return Err(format!(
                        "static geometry interpreter input key {key} is not admitted"
                    )
                    .into());
                }
            }
        }
    }
    for constant in &draw.ordered_constant_buffers {
        validate_shader_payload(&constant.raw_bytes, &constant.sha256, "constant buffer")?;
        if constant.byte_count == 0 {
            return Err("constant-buffer selected range is empty".into());
        }
        let end = usize::try_from(constant.byte_offset)?
            .checked_add(usize::try_from(constant.byte_count)?)
            .ok_or("constant-buffer selected range overflow")?;
        if end > constant.raw_bytes.len() {
            return Err("constant-buffer selected range exceeds carried bytes".into());
        }
    }
    for target in &draw.output_state.ordered_color_targets {
        validate_render_target_content(target, resources, expected_consumer)?;
    }
    if let Some(target) = &draw.output_state.depth_target {
        validate_render_target_content(target, resources, expected_consumer)?;
    }
    Ok(())
}

fn validate_render_target_content(
    target: &RenderTargetDesc,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<()> {
    if let Some(content) = &target.persistent_content {
        require_content_consumer(content, resources, expected_consumer)?;
    }
    let Some(exact) = &target.exact_resource else {
        return Ok(());
    };
    for initial in &exact.ordered_initial_subresources {
        match &initial.source {
            RenderTargetInitialSource::Inline { bytes, sha256 } => {
                if Sha256::hex(bytes) != *sha256 {
                    return Err("exact render-target inline initialization SHA-256 mismatch".into());
                }
            }
            RenderTargetInitialSource::UsedContentRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => {
                let resource = require_content_consumer(content, resources, expected_consumer)?;
                let start = usize::try_from(*source_byte_offset)
                    .map_err(|_| "exact render-target content offset does not fit usize")?;
                let count = usize::try_from(*byte_count)
                    .map_err(|_| "exact render-target content length does not fit usize")?;
                let end = start
                    .checked_add(count)
                    .ok_or("exact render-target content range overflows")?;
                let bytes = resource
                    .bytes
                    .get(start..end)
                    .ok_or("exact render-target content range exceeds admitted payload")?;
                if Sha256::hex(bytes) != *range_sha256 {
                    return Err("exact render-target content-range SHA-256 mismatch".into());
                }
            }
        }
    }
    Ok(())
}

fn require_content_consumer<'a>(
    content: &UsedContentKey,
    resources: &'a BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<&'a ProductionMapResource> {
    let resource = require_content(content, resources)?;
    if !resource
        .consumers
        .iter()
        .any(|consumer| consumer == expected_consumer)
    {
        return Err(format!(
            "exact render-target content {} is not bound to sealed consumer {expected_consumer}",
            content.normalized_game_relative_path
        )
        .into());
    }
    Ok(resource)
}

fn validate_shader_payload(bytes: &[u8], expected: &str, role: &str) -> Result<()> {
    if bytes.is_empty() {
        return Err(format!("{role} bytecode/payload is empty").into());
    }
    let actual = Sha256::hex(bytes);
    if actual != expected {
        return Err(format!("{role} SHA-256 mismatch: expected {expected}, got {actual}").into());
    }
    Ok(())
}

fn validate_buffer_source(
    source: &BufferSource,
    resources: &BTreeMap<String, ProductionMapResource>,
) -> Result<()> {
    match source {
        BufferSource::Inline { bytes, sha256 } => {
            validate_shader_payload(bytes, sha256, "inline buffer")
        }
        BufferSource::ParticleInstanceRecords => Err(
            "ParticleInstanceRecords buffer source is not valid in static map validation".into(),
        ),
        BufferSource::UsedContentRange {
            content,
            source_byte_offset,
            byte_count,
            range_sha256,
        } => {
            let resource = require_content(content, resources)?;
            let start = usize::try_from(*source_byte_offset)
                .map_err(|_| "used-content range offset does not fit usize")?;
            let len = usize::try_from(*byte_count)
                .map_err(|_| "used-content range length does not fit usize")?;
            let end = start
                .checked_add(len)
                .ok_or("used-content range overflow")?;
            let bytes = resource
                .bytes
                .get(start..end)
                .ok_or("used-content range is outside admitted payload")?;
            validate_shader_payload(bytes, range_sha256, "used-content range")
        }
    }
}

fn require_content<'a>(
    content: &UsedContentKey,
    resources: &'a BTreeMap<String, ProductionMapResource>,
) -> Result<&'a ProductionMapResource> {
    resources
        .values()
        .find(|resource| &resource.content == content)
        .ok_or_else(|| {
            format!(
                "draw references non-admitted content {}",
                content.normalized_game_relative_path
            )
            .into()
        })
}

fn collect_static_operation_content(
    operation: &StaticMapOperation,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    let StaticMapCommand::Draw { draw } = &operation.command else {
        let target = match &operation.command {
            StaticMapCommand::ClearColor { target, .. }
            | StaticMapCommand::ClearDepthStencil { target, .. } => target,
            StaticMapCommand::Draw { .. } => unreachable!(),
        };
        collect_render_target_content(target, out);
        return;
    };
    let mut insert = |value: &UsedContentKey| insert_content(out, value);
    insert(&draw.shader_program.source);
    for binding in &draw.ordered_shader_resources {
        if let StaticShaderResourceSource::UsedContent {
            content, resource, ..
        } = &binding.source
        {
            insert(content);
            for subresource in &resource.ordered_subresources {
                collect_buffer_source_content(&subresource.source, &mut insert);
            }
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for vertex in &geometry.ordered_vertex_buffers {
            collect_buffer_source_content(&vertex.source, &mut insert);
        }
        if let Some(index) = &geometry.index_buffer {
            collect_buffer_source_content(&index.source, &mut insert);
        }
    }
    for target in &draw.output_state.ordered_color_targets {
        collect_render_target_content_with(target, &mut insert);
    }
    if let Some(target) = &draw.output_state.depth_target {
        collect_render_target_content_with(target, &mut insert);
    }
}

fn collect_runtime_draw_content(
    draw: &PreparedRuntimeDraw,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    match draw {
        PreparedRuntimeDraw::Particle { packet, .. } => {
            collect_runtime_parts(
                &packet.shader_program.source,
                &packet.ordered_texture_bindings,
                &packet.geometry,
                &packet.output_state,
                out,
            );
        }
        PreparedRuntimeDraw::Entity { submission, .. }
        | PreparedRuntimeDraw::O04City { submission, .. }
        | PreparedRuntimeDraw::O05Building { submission, .. } => {
            collect_runtime_parts(
                &submission.shader_program.source,
                &submission.ordered_texture_bindings,
                &submission.geometry,
                &submission.output_state,
                out,
            );
            insert_content(out, &submission.content_identity.resolved_entity_asset);
            insert_content(out, &submission.content_identity.mesh);
        }
    }
}

fn collect_runtime_parts(
    shader: &UsedContentKey,
    textures: &[replay_core::TextureBinding],
    geometry: &replay_core::GeometryBinding,
    output: &replay_core::RenderOutputState,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    insert_content(out, shader);
    for texture in textures {
        insert_content(out, &texture.content);
        for subresource in &texture.resource.ordered_subresources {
            if let BufferSource::UsedContentRange { content, .. } = &subresource.source {
                insert_content(out, content);
            }
        }
    }
    for vertex in &geometry.ordered_vertex_buffers {
        if let BufferSource::UsedContentRange { content, .. } = &vertex.source {
            insert_content(out, content);
        }
    }
    if let Some(index) = &geometry.index_buffer {
        if let BufferSource::UsedContentRange { content, .. } = &index.source {
            insert_content(out, content);
        }
    }
    for target in &output.ordered_color_targets {
        collect_render_target_content(target, out);
    }
    if let Some(target) = &output.depth_target {
        collect_render_target_content(target, out);
    }
}

fn collect_render_target_content(
    target: &RenderTargetDesc,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    collect_render_target_content_with(target, &mut |content| insert_content(out, content));
}

fn collect_render_target_content_with(
    target: &RenderTargetDesc,
    insert: &mut impl FnMut(&UsedContentKey),
) {
    if let Some(content) = &target.persistent_content {
        insert(content);
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source {
                insert(content);
            }
        }
    }
}

fn static_operation_token(operation: &StaticMapOperation) -> String {
    match &operation.command {
        StaticMapCommand::ClearColor { .. } => "clear_color".into(),
        StaticMapCommand::ClearDepthStencil { .. } => "clear_depth_stencil".into(),
        StaticMapCommand::Draw { draw } => draw.draw_token.clone(),
    }
}

fn collect_buffer_source_content(source: &BufferSource, insert: &mut impl FnMut(&UsedContentKey)) {
    if let BufferSource::UsedContentRange { content, .. } = source {
        insert(content);
    }
}

fn insert_content(out: &mut BTreeMap<String, UsedContentKey>, value: &UsedContentKey) {
    let key = format!(
        "{}:{}:{}:{}",
        value.winning_layer_ordinal,
        value.layer_id,
        value.normalized_game_relative_path,
        value.payload_sha256
    );
    out.insert(key, value.clone());
}

fn runtime_draw_token(draw: &PreparedRuntimeDraw) -> String {
    match draw {
        PreparedRuntimeDraw::Particle { ordered_draw, .. }
        | PreparedRuntimeDraw::Entity { ordered_draw, .. }
        | PreparedRuntimeDraw::O04City { ordered_draw, .. }
        | PreparedRuntimeDraw::O05Building { ordered_draw, .. } => {
            serde_json::to_string(ordered_draw).expect("RenderOutputRef serialization cannot fail")
        }
    }
}

fn runtime_draw_pass(draw: &PreparedRuntimeDraw) -> &'static str {
    match draw {
        PreparedRuntimeDraw::O04City { .. } => "o04_city",
        PreparedRuntimeDraw::O05Building { .. } => "o05_building",
        PreparedRuntimeDraw::Particle { .. } | PreparedRuntimeDraw::Entity { .. } => {
            "runtime_submit"
        }
    }
}

#[cfg(windows)]
mod m02_overlay;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod native_draw;
mod s1_shadow;

pub use s1_shadow::{
    prepare_s1_shadow_execution, validate_s1_shadow_execution, PreparedS1ShadowExecution,
};

#[cfg(windows)]
pub use m02_overlay::{
    M02OverlayExecutedDraw, M02OverlayExecutionReceipt, NativeM02OverlayExecutor,
    PreparedM02OverlayPlan,
};
#[cfg(windows)]
pub use native::{NativeProductionD3dBackend, NativeProductionD3dExecutor, ProductionD3dReadback};

#[cfg(test)]
mod tests;
