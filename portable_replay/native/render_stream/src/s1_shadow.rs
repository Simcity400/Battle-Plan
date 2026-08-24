use super::{DrawCommand, PreparedRenderTarget, RenderExecutionRef, RenderSubmit};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const S1_SHADOW_SCHEMA: &str = "hoi4.production-s1-shadow.a729.v3";
pub const S1_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const S1_L02_AUTHORITY_SHA256: &str =
    "b5e7fc49bf112d566954d9711ab7227db3e48142c8dc6af00da2ae28f9aa2356";
pub const S1_L03_AUTHORITY_SHA256: &str =
    "544a74b950128f1ec23937fd5427a22fd5ee4faa5297f5792ff4c60dbed99d39";
pub const S1_CHAIN_AUTHORITY_SHA256: &str =
    "87bd8b3a0dbde7ef0bddae9305c6177e6a5cdcebcaf804b3a58534a3dcab12ca";
pub const S1_LIGHT_DEPTH_WIDTH: u32 = 2048;
pub const S1_LIGHT_DEPTH_HEIGHT: u32 = 2048;
pub const S1_LIGHT_DEPTH_DXGI_FORMAT: u32 = 41;
pub const S1_SCREEN_DXGI_FORMAT: u32 = 87;
pub const S1_TARGET_MIP_LEVELS: u32 = 1;
pub const S1_TARGET_BIND_FLAGS: u32 = 0x28;
pub const S1_PARTITION_PRIMARY: u8 = 1;
pub const S1_PARTITION_WRAP_WEST: u8 = 2;
pub const S1_PARTITION_WRAP_EAST: u8 = 4;
pub const S1_PARTITION_ALL: u8 =
    S1_PARTITION_PRIMARY | S1_PARTITION_WRAP_WEST | S1_PARTITION_WRAP_EAST;
pub const S1_BLUR_CAMERA_DISTANCE_F32_BITS: u32 = 0x442f_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S1SelectedLight {
    Sun,
    Moon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S1TypedOwner {
    GenericPdxmesh,
    M01Unit,
    O04City,
    O05StaticFixed,
    M04RailwayBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S1ShadowPipeline {
    StandardUnskinned,
    StandardSkinned,
    FixedMapFurnitureBorder,
    TreeShadow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub enum S1DrawArguments {
    Draw {
        vertex_count: u32,
        start_vertex: u32,
    },
    DrawIndexed {
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index: u32,
        base_vertex: i32,
        start_instance: u32,
    },
}

impl S1DrawArguments {
    fn is_empty(self) -> bool {
        match self {
            Self::Draw { vertex_count, .. } => vertex_count == 0,
            Self::DrawIndexed { index_count, .. } => index_count == 0,
            Self::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                ..
            } => index_count_per_instance == 0 || instance_count == 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S1TargetBinding {
    pub stable_resource_id: String,
    pub prepared_target: PreparedRenderTarget,
    pub width: u32,
    pub height: u32,
    pub dxgi_format_u32: u32,
    pub mip_levels: u32,
    pub bind_flags_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "producer_family",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum S1LightDepthSubmission {
    Pdxmesh {
        partition_mask: u8,
        stable_typed_upstream_id: String,
        owner: S1TypedOwner,
        world_matrix_f32_bits: [u32; 16],
        selected_mesh: String,
        selected_submesh_material: String,
        group_ordinal: u32,
        submesh_ordinal: u32,
        pipeline: S1ShadowPipeline,
        draw: S1DrawArguments,
        execution: RenderExecutionRef,
    },
    Tree {
        partition_mask: u8,
        source_row_ordinal: u32,
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        selected_mesh_ordinal: u32,
        selected_lod_index: u32,
        selected_mesh: String,
        selected_material: String,
        pipeline: S1ShadowPipeline,
        draw: S1DrawArguments,
        execution: RenderExecutionRef,
    },
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

impl S1LightDepthSubmission {
    pub fn execution(&self) -> &RenderExecutionRef {
        match self {
            Self::Pdxmesh { execution, .. } | Self::Tree { execution, .. } => execution,
        }
    }

    pub fn draw(&self) -> S1DrawArguments {
        match self {
            Self::Pdxmesh { draw, .. } | Self::Tree { draw, .. } => *draw,
        }
    }

    fn validate(&self, enabled_partition_mask: u8) -> Result<(), String> {
        let (partition_mask, mesh, material) = match self {
            Self::Pdxmesh {
                partition_mask,
                stable_typed_upstream_id,
                world_matrix_f32_bits,
                selected_mesh,
                selected_submesh_material,
                pipeline,
                ..
            } => {
                if stable_typed_upstream_id.trim().is_empty()
                    || world_matrix_f32_bits
                        .iter()
                        .any(|bits| !f32::from_bits(*bits).is_finite())
                {
                    return Err("S1 PdxMesh identity/matrix is absent or non-finite".into());
                }
                if *pipeline == S1ShadowPipeline::TreeShadow {
                    return Err("S1 PdxMesh submission selects the tree pipeline".into());
                }
                (*partition_mask, selected_mesh, selected_submesh_material)
            }
            Self::Tree {
                partition_mask,
                selected_mesh,
                selected_material,
                pipeline,
                execution,
                ..
            } => {
                if *pipeline != S1ShadowPipeline::TreeShadow {
                    return Err("S1 tree submission does not select the tree pipeline".into());
                }
                if !matches!(execution, RenderExecutionRef::Static { .. }) {
                    return Err("S1 tree submission is not an exact static operation".into());
                }
                (*partition_mask, selected_mesh, selected_material)
            }
        };
        if partition_mask == 0
            || partition_mask.count_ones() != 1
            || partition_mask & !S1_PARTITION_ALL != 0
            || partition_mask & enabled_partition_mask == 0
            || mesh.trim().is_empty()
            || material.trim().is_empty()
        {
            return Err("S1 submission partition/asset selection is invalid".into());
        }
        let draw = self.draw();
        if draw.is_empty() {
            return Err("S1 submission carries an empty exact draw".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S1ShadowCarrier {
    pub schema: String,
    pub executable_sha256: String,
    pub l02_authority_sha256: String,
    pub l03_authority_sha256: String,
    pub s1_chain_authority_sha256: String,
    pub shadow_pass_active: bool,
    pub blur_camera_factor_f32_bits: u32,
    pub selected_light: Option<S1SelectedLight>,
    pub light_view_projection_f32_bits: Option<[u32; 16]>,
    pub shadow_map_texture_matrix_f32_bits: Option<[u32; 16]>,
    pub camera_y_f32_bits: Option<u32>,
    pub enabled_partition_mask: Option<u8>,
    pub light_depth_target: Option<S1TargetBinding>,
    pub producer_and_final_target: Option<S1TargetBinding>,
    pub blur_intermediate_target: Option<S1TargetBinding>,
    pub light_depth_clear: Option<RenderExecutionRef>,
    pub ordered_light_depth_submissions: Vec<S1LightDepthSubmission>,
    pub ordered_terrain_unlit_draws: Vec<RenderExecutionRef>,
    pub horizontal_blur_draw: Option<RenderExecutionRef>,
    pub vertical_blur_draw: Option<RenderExecutionRef>,
    pub ordered_shadow_consumers: Vec<RenderExecutionRef>,
    pub payload_sha256: String,
}

impl S1ShadowCarrier {
    pub fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        super::canonical_json_sha256(&canonical)
    }

    pub fn is_inactive(&self) -> bool {
        !self.shadow_pass_active
    }

    pub fn validate_against_submit(&self, submit: &RenderSubmit) -> Result<(), String> {
        if self.schema != S1_SHADOW_SCHEMA
            || self.executable_sha256 != S1_A729_EXECUTABLE_SHA256
            || self.l02_authority_sha256 != S1_L02_AUTHORITY_SHA256
            || self.l03_authority_sha256 != S1_L03_AUTHORITY_SHA256
            || self.s1_chain_authority_sha256 != S1_CHAIN_AUTHORITY_SHA256
        {
            return Err("S1 carrier schema/executable/evidence identity mismatch".into());
        }
        super::check_sha256(&self.payload_sha256, None)?;
        if self.payload_sha256 != self.computed_payload_sha256()? {
            return Err("S1 carrier payload SHA-256 mismatch".into());
        }
        // The retired wire join between the C07 tree runtime carrier and this
        // S1 chain, re-established against the portable C07 derivation: every
        // admission of a carried S1 carrier must agree with the tree shadow
        // operations derivable from the same recorded submit. A derivation
        // error here means the recorded static tree family is incomplete or
        // tampered; no derived trees simply skips the join (matching the old
        // both-carriers-present semantics).
        let trees = crate::C07TreeRuntimeCarrier::from_authenticated_selected_submit(submit)?;
        if let Some(trees) = &trees {
            if self.is_inactive() {
                return Err(
                    "C07 tree-shadow operations conflict with an inactive S1 skip".into(),
                );
            }
            trees.validate_s1_tree_shadow_join(self)?;
        }
        let blur_camera_factor = f32::from_bits(self.blur_camera_factor_f32_bits);
        if !blur_camera_factor.is_finite() || !(0.0..=1.0).contains(&blur_camera_factor) {
            return Err("S1 blur camera factor is non-finite or outside [0,1]".into());
        }
        if self.is_inactive() {
            if self.selected_light.is_some()
                || self.light_view_projection_f32_bits.is_some()
                || self.shadow_map_texture_matrix_f32_bits.is_some()
                || self.camera_y_f32_bits.is_some()
                || self.enabled_partition_mask.is_some()
                || self.light_depth_target.is_some()
                || self.producer_and_final_target.is_some()
                || self.blur_intermediate_target.is_some()
                || self.light_depth_clear.is_some()
                || !self.ordered_light_depth_submissions.is_empty()
                || !self.ordered_terrain_unlit_draws.is_empty()
                || self.horizontal_blur_draw.is_some()
                || self.vertical_blur_draw.is_some()
                || !self.ordered_shadow_consumers.is_empty()
            {
                return Err("inactive S1 carrier contains active shadow work".into());
            }
            return Ok(());
        }

        let light_view_projection = self
            .light_view_projection_f32_bits
            .as_ref()
            .ok_or("active S1 carrier has no L02 light-view-projection matrix")?;
        let texture_matrix = self
            .shadow_map_texture_matrix_f32_bits
            .as_ref()
            .ok_or("active S1 carrier has no L02 shadow texture matrix")?;
        if self.selected_light.is_none()
            || light_view_projection
                .iter()
                .chain(texture_matrix.iter())
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("active S1 selected-light/matrix state is absent or non-finite".into());
        }
        validate_texture_matrix_composition(light_view_projection, texture_matrix)?;
        let camera_y = self
            .camera_y_f32_bits
            .map(f32::from_bits)
            .filter(|value| value.is_finite())
            .ok_or("active S1 camera Y is absent or non-finite")?;
        let expected_blur_camera_factor =
            (camera_y / f32::from_bits(S1_BLUR_CAMERA_DISTANCE_F32_BITS)).min(1.0);
        if expected_blur_camera_factor.to_bits() != self.blur_camera_factor_f32_bits {
            return Err(
                "active S1 blur camera factor disagrees with exact a729 producer law".into(),
            );
        }
        let enabled = self
            .enabled_partition_mask
            .ok_or("active S1 partition mask is absent")?;
        if enabled & S1_PARTITION_PRIMARY == 0 || enabled & !S1_PARTITION_ALL != 0 {
            return Err("active S1 partition mask is invalid".into());
        }

        let light = self
            .light_depth_target
            .as_ref()
            .ok_or("active S1 light-depth target is absent")?;
        let screen_a = self
            .producer_and_final_target
            .as_ref()
            .ok_or("active S1 producer/final target is absent")?;
        let screen_b = self
            .blur_intermediate_target
            .as_ref()
            .ok_or("active S1 blur-intermediate target is absent")?;
        validate_target(
            light,
            S1_LIGHT_DEPTH_WIDTH,
            S1_LIGHT_DEPTH_HEIGHT,
            S1_LIGHT_DEPTH_DXGI_FORMAT,
            "light-depth",
        )?;
        validate_target(
            screen_a,
            screen_a.width,
            screen_a.height,
            S1_SCREEN_DXGI_FORMAT,
            "screen A",
        )?;
        validate_target(
            screen_b,
            screen_a.width,
            screen_a.height,
            S1_SCREEN_DXGI_FORMAT,
            "screen B",
        )?;
        if screen_a.width == 0
            || screen_a.height == 0
            || light.prepared_target == screen_a.prepared_target
            || light.prepared_target == screen_b.prepared_target
            || screen_a.prepared_target == screen_b.prepared_target
            || light.stable_resource_id == screen_a.stable_resource_id
            || light.stable_resource_id == screen_b.stable_resource_id
            || screen_a.stable_resource_id == screen_b.stable_resource_id
        {
            return Err("active S1 target identities alias or screen dimensions are empty".into());
        }

        let clear = self
            .light_depth_clear
            .as_ref()
            .ok_or("active S1 light-depth clear reference is absent")?;
        let horizontal = self
            .horizontal_blur_draw
            .as_ref()
            .ok_or("active S1 horizontal-blur reference is absent")?;
        let vertical = self
            .vertical_blur_draw
            .as_ref()
            .ok_or("active S1 vertical-blur reference is absent")?;
        if self.ordered_light_depth_submissions.is_empty()
            || self.ordered_terrain_unlit_draws.is_empty()
            || self.ordered_shadow_consumers.is_empty()
        {
            return Err("active S1 caster, producer, or consumer draw set is empty".into());
        }

        for submission in &self.ordered_light_depth_submissions {
            submission.validate(enabled)?;
        }
        validate_l03_projection_order(&self.ordered_light_depth_submissions)?;
        let mut chain = Vec::new();
        chain.push(clear);
        chain.extend(
            self.ordered_light_depth_submissions
                .iter()
                .map(S1LightDepthSubmission::execution),
        );
        chain.extend(self.ordered_terrain_unlit_draws.iter());
        chain.push(horizontal);
        chain.push(vertical);
        chain.extend(self.ordered_shadow_consumers.iter());
        validate_execution_order(submit, &chain)?;

        let operations = static_operations(submit)?;
        validate_light_depth_clear(clear, &operations, light)?;
        for submission in &self.ordered_light_depth_submissions {
            validate_light_depth_draw(submit, submission, light)?;
        }
        validate_matrix_constant_buffer(
            execution_constant_buffers(
                submit,
                self.ordered_light_depth_submissions[0].execution(),
                "first light-depth draw",
            )?,
            0x03,
            0,
            416,
            light_view_projection,
            "first light-depth VS/PS CB0",
        )?;
        let last_light_writer = self
            .ordered_light_depth_submissions
            .last()
            .map(S1LightDepthSubmission::execution)
            .unwrap_or(clear);
        for draw in &self.ordered_terrain_unlit_draws {
            validate_static_edge(
                draw,
                &operations,
                light,
                last_light_writer,
                Some(screen_a),
                "terrain-unlit",
            )?;
            validate_matrix_constant_buffer(
                execution_constant_buffers(submit, draw, "terrain-unlit draw")?,
                0x03,
                1,
                112,
                texture_matrix,
                "terrain-unlit VS/PS CB1",
            )?;
        }
        let last_terrain = self
            .ordered_terrain_unlit_draws
            .last()
            .ok_or("active S1 terrain-unlit draw set became empty")?;
        validate_static_edge(
            horizontal,
            &operations,
            screen_a,
            last_terrain,
            Some(screen_b),
            "horizontal blur",
        )?;
        validate_static_edge(
            vertical,
            &operations,
            screen_b,
            horizontal,
            Some(screen_a),
            "vertical blur",
        )?;
        for consumer in &self.ordered_shadow_consumers {
            validate_static_edge(
                consumer,
                &operations,
                screen_a,
                vertical,
                None,
                "shadow consumer",
            )?;
        }
        Ok(())
    }
}

fn validate_texture_matrix_composition(
    light_view_projection: &[u32; 16],
    texture_matrix: &[u32; 16],
) -> Result<(), String> {
    let mut expected = [0u32; 16];
    for row in 0..4 {
        let x = f32::from_bits(light_view_projection[row * 4]);
        let y = f32::from_bits(light_view_projection[row * 4 + 1]);
        expected[row * 4] = (x * 0.5 + if row == 3 { 0.5 } else { 0.0 }).to_bits();
        expected[row * 4 + 1] = (y * -0.5 + if row == 3 { 0.5 } else { 0.0 }).to_bits();
        expected[row * 4 + 2] = light_view_projection[row * 4 + 2];
        expected[row * 4 + 3] = light_view_projection[row * 4 + 3];
    }
    if &expected != texture_matrix {
        return Err("S1 L02 texture matrix differs from exact clip-to-texture composition".into());
    }
    Ok(())
}

fn execution_constant_buffers<'a>(
    submit: &'a RenderSubmit,
    reference: &RenderExecutionRef,
    role: &str,
) -> Result<&'a [super::ConstantBufferBinding], String> {
    match reference {
        RenderExecutionRef::Static { operation_ordinal } => {
            let operation = submit
                .ordered_static_map_programs
                .iter()
                .flat_map(|program| program.ordered_operations.iter())
                .find(|operation| operation.operation_ordinal == *operation_ordinal)
                .ok_or_else(|| format!("S1 {role} static operation is absent"))?;
            let super::StaticMapCommand::Draw { draw } = &operation.command else {
                return Err(format!("S1 {role} is not a draw"));
            };
            Ok(&draw.ordered_constant_buffers)
        }
        RenderExecutionRef::Runtime { output } => {
            let entity = match output {
                super::RenderOutputRef::Entity {
                    pass_ordinal,
                    entity_instance_id,
                    draw_order,
                } => submit.entities.iter().find(|entity| {
                    entity.pass_ordinal == *pass_ordinal
                        && entity.entity_instance_id == *entity_instance_id
                        && entity.draw_order == *draw_order
                }),
                super::RenderOutputRef::Particle { .. } => None,
            }
            .ok_or_else(|| format!("S1 {role} runtime entity draw is absent"))?;
            Ok(&entity.ordered_constant_buffers)
        }
        RenderExecutionRef::M03 { .. } => {
            Err(format!("S1 {role} uses an unsupported execution family"))
        }
    }
}

fn validate_matrix_constant_buffer(
    bindings: &[super::ConstantBufferBinding],
    stage_mask_u8: u8,
    slot: u8,
    byte_count: u32,
    prefix_bits: &[u32; 16],
    role: &str,
) -> Result<(), String> {
    let matches = bindings
        .iter()
        .filter(|binding| {
            binding.stage_mask_u8 == stage_mask_u8
                && binding.slot == slot
                && binding.byte_offset == 0
                && binding.byte_count == byte_count
                && binding.raw_bytes.len() == byte_count as usize
                && binding.raw_bytes[..64]
                    .chunks_exact(4)
                    .zip(prefix_bits.iter())
                    .all(|(bytes, bits)| bytes == bits.to_le_bytes())
        })
        .count();
    if matches != 1 {
        return Err(format!(
            "S1 {role} does not carry exactly one recovered matrix payload"
        ));
    }
    Ok(())
}

fn validate_l03_projection_order(rows: &[S1LightDepthSubmission]) -> Result<(), String> {
    let partition_rank = |mask| match mask {
        S1_PARTITION_PRIMARY => Some(0u8),
        S1_PARTITION_WRAP_WEST => Some(1u8),
        S1_PARTITION_WRAP_EAST => Some(2u8),
        _ => None,
    };
    let mut prior_partition = None;
    let mut saw_tree = false;
    let mut object_identities = BTreeMap::<String, (S1TypedOwner, [u32; 16], String)>::new();
    let mut object_draw_keys = BTreeSet::new();
    let mut tree_source_keys = BTreeSet::new();

    for row in rows {
        let mask = match row {
            S1LightDepthSubmission::Pdxmesh { partition_mask, .. }
            | S1LightDepthSubmission::Tree { partition_mask, .. } => *partition_mask,
        };
        let rank = partition_rank(mask).ok_or("S1 L03 row has a non-canonical partition")?;
        if prior_partition.is_some_and(|prior| rank < prior) {
            return Err("S1 L03 rows change canonical primary/west/east partition order".into());
        }
        prior_partition = Some(rank);

        match row {
            S1LightDepthSubmission::Pdxmesh {
                stable_typed_upstream_id,
                owner,
                world_matrix_f32_bits,
                selected_mesh,
                group_ordinal,
                submesh_ordinal,
                ..
            } => {
                if saw_tree {
                    return Err("S1 L03 PdxMesh row occurs after the first tree row".into());
                }
                let identity = (*owner, *world_matrix_f32_bits, selected_mesh.clone());
                if let Some(previous) = object_identities.get(stable_typed_upstream_id) {
                    if previous != &identity {
                        return Err(
                            "S1 L03 stable object ID changes owner, transform, or mesh".into()
                        );
                    }
                } else {
                    object_identities.insert(stable_typed_upstream_id.clone(), identity);
                }
                if !object_draw_keys.insert((
                    mask,
                    stable_typed_upstream_id.clone(),
                    *group_ordinal,
                    *submesh_ordinal,
                )) {
                    return Err("S1 L03 object group/submesh key repeats in a partition".into());
                }
            }
            S1LightDepthSubmission::Tree {
                source_row_ordinal,
                selected_mesh_ordinal,
                ..
            } => {
                saw_tree = true;
                if !tree_source_keys.insert((mask, *source_row_ordinal, *selected_mesh_ordinal)) {
                    return Err(
                        "S1 L03 tree source row and selected mesh repeat in a partition".into(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_target(
    target: &S1TargetBinding,
    width: u32,
    height: u32,
    format: u32,
    role: &str,
) -> Result<(), String> {
    if target.stable_resource_id.trim().is_empty()
        || matches!(
            &target.prepared_target,
            PreparedRenderTarget::SwapchainBackBuffer
        )
        || target.width != width
        || target.height != height
        || target.dxgi_format_u32 != format
        || target.mip_levels != S1_TARGET_MIP_LEVELS
        || target.bind_flags_u32 != S1_TARGET_BIND_FLAGS
    {
        return Err(format!("S1 {role} target identity/descriptor mismatch"));
    }
    Ok(())
}

fn validate_execution_order(
    submit: &RenderSubmit,
    chain: &[&RenderExecutionRef],
) -> Result<(), String> {
    let positions = submit
        .ordered_render_operations
        .iter()
        .enumerate()
        .map(|(position, reference)| (reference.clone(), position))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for reference in chain {
        if !seen.insert((*reference).clone()) {
            return Err("S1 operation chain repeats an execution reference".into());
        }
        let position = positions
            .get(*reference)
            .copied()
            .ok_or("S1 operation chain references an absent unified operation")?;
        if previous.is_some_and(|prior| position <= prior) {
            return Err("S1 operation chain differs from unified execution order".into());
        }
        previous = Some(position);
    }
    Ok(())
}

fn static_operations(
    submit: &RenderSubmit,
) -> Result<BTreeMap<u32, &super::StaticMapOperation>, String> {
    let mut out = BTreeMap::new();
    for operation in submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
    {
        if out.insert(operation.operation_ordinal, operation).is_some() {
            return Err("S1 static operation inventory repeats an ordinal".into());
        }
    }
    Ok(out)
}

fn static_ordinal(reference: &RenderExecutionRef, role: &str) -> Result<u32, String> {
    match reference {
        RenderExecutionRef::Static { operation_ordinal } => Ok(*operation_ordinal),
        _ => Err(format!("S1 {role} must reference a static operation")),
    }
}

fn validate_light_depth_clear(
    reference: &RenderExecutionRef,
    operations: &BTreeMap<u32, &super::StaticMapOperation>,
    target: &S1TargetBinding,
) -> Result<(), String> {
    let operation = operations
        .get(&static_ordinal(reference, "light-depth clear")?)
        .ok_or("S1 light-depth clear operation is absent")?;
    let super::StaticMapCommand::ClearColor { target: actual, .. } = &operation.command else {
        return Err("S1 light-depth clear is not a color clear".into());
    };
    validate_output_target(actual, target, "light-depth clear")
}

fn validate_light_depth_draw(
    submit: &RenderSubmit,
    submission: &S1LightDepthSubmission,
    target: &S1TargetBinding,
) -> Result<(), String> {
    let (command, output) = match submission.execution() {
        RenderExecutionRef::Static { operation_ordinal } => {
            let operation = submit
                .ordered_static_map_programs
                .iter()
                .flat_map(|program| program.ordered_operations.iter())
                .find(|operation| operation.operation_ordinal == *operation_ordinal)
                .ok_or("S1 light-depth static draw is absent")?;
            let super::StaticMapCommand::Draw { draw } = &operation.command else {
                return Err("S1 light-depth reference is not a draw".into());
            };
            let command = match &draw.geometry {
                super::StaticGeometrySource::Exact { geometry } => &geometry.draw,
                super::StaticGeometrySource::InterpreterOutput { draw, .. } => draw,
            };
            (command, &draw.output_state)
        }
        RenderExecutionRef::Runtime { output } => {
            let entity = match output {
                super::RenderOutputRef::Entity {
                    pass_ordinal,
                    entity_instance_id,
                    draw_order,
                } => submit.entities.iter().find(|entity| {
                    entity.pass_ordinal == *pass_ordinal
                        && entity.entity_instance_id == *entity_instance_id
                        && entity.draw_order == *draw_order
                }),
                super::RenderOutputRef::Particle { .. } => None,
            }
            .ok_or("S1 light-depth runtime reference is not an exact entity draw")?;
            if let S1LightDepthSubmission::Pdxmesh {
                world_matrix_f32_bits,
                selected_mesh,
                pipeline,
                ..
            } = submission
            {
                if entity.world_transform_4x4_f32_bits != *world_matrix_f32_bits {
                    return Err("S1 L03/runtime entity world matrix differs".into());
                }
                if entity
                    .content_identity
                    .mesh
                    .normalized_game_relative_path
                    .as_str()
                    != selected_mesh.as_str()
                {
                    return Err("S1 L03/runtime entity selected mesh differs".into());
                }
                let skinned = matches!(&entity.skinning, super::Skinning::Palette { .. });
                if (*pipeline == S1ShadowPipeline::StandardSkinned) != skinned {
                    return Err("S1 L03/runtime entity skinning pipeline differs".into());
                }
            }
            (&entity.geometry.draw, &entity.output_state)
        }
        _ => return Err("S1 light-depth draw has unsupported execution family".into()),
    };
    if output.ordered_color_targets.as_slice().len() != 1 {
        return Err("S1 light-depth draw does not own exactly one color target".into());
    }
    validate_output_target(&output.ordered_color_targets[0], target, "light-depth draw")?;
    validate_draw_arguments(command, submission.draw())
}

fn validate_draw_arguments(actual: &DrawCommand, expected: S1DrawArguments) -> Result<(), String> {
    let matches = match (actual, expected) {
        (
            DrawCommand::Draw {
                vertex_count,
                start_vertex,
            },
            S1DrawArguments::Draw {
                vertex_count: expected_vertex_count,
                start_vertex: expected_start_vertex,
            },
        ) => *vertex_count == expected_vertex_count && *start_vertex == expected_start_vertex,
        (
            DrawCommand::DrawIndexed {
                index_count,
                start_index,
                base_vertex,
            },
            S1DrawArguments::DrawIndexed {
                index_count: expected_index_count,
                start_index: expected_start_index,
                base_vertex: expected_base_vertex,
            },
        ) => {
            *index_count == expected_index_count
                && *start_index == expected_start_index
                && *base_vertex == expected_base_vertex
        }
        (
            DrawCommand::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                start_index,
                base_vertex,
                start_instance,
            },
            S1DrawArguments::DrawIndexedInstanced {
                index_count_per_instance: expected_index_count,
                instance_count: expected_instance_count,
                start_index: expected_start_index,
                base_vertex: expected_base_vertex,
                start_instance: expected_start_instance,
            },
        ) => {
            *index_count_per_instance == expected_index_count
                && *instance_count == expected_instance_count
                && *start_index == expected_start_index
                && *base_vertex == expected_base_vertex
                && *start_instance == expected_start_instance
        }
        _ => false,
    };
    if !matches {
        return Err("S1 L03 draw arguments differ from executable submission".into());
    }
    Ok(())
}

fn validate_static_edge(
    reference: &RenderExecutionRef,
    operations: &BTreeMap<u32, &super::StaticMapOperation>,
    source: &S1TargetBinding,
    producer: &RenderExecutionRef,
    output: Option<&S1TargetBinding>,
    role: &str,
) -> Result<(), String> {
    let operation = operations
        .get(&static_ordinal(reference, role)?)
        .ok_or_else(|| format!("S1 {role} operation is absent"))?;
    let super::StaticMapCommand::Draw { draw } = &operation.command else {
        return Err(format!("S1 {role} reference is not a draw"));
    };
    let matching_sources = draw
        .ordered_shader_resources
        .iter()
        .filter(|binding| match &binding.source {
            super::StaticShaderResourceSource::PreparedTarget {
                target,
                resource,
                view,
                producer: actual_producer,
            } => {
                target == &source.prepared_target
                    && actual_producer == producer
                    && texture_resource_matches(resource, source)
                    && srv_matches(view, source)
            }
            _ => false,
        })
        .count();
    if matching_sources != 1 {
        return Err(format!(
            "S1 {role} does not have exactly one true-producer source edge"
        ));
    }
    if let Some(output) = output {
        if draw.output_state.ordered_color_targets.len() != 1 {
            return Err(format!("S1 {role} does not own exactly one color target"));
        }
        validate_output_target(&draw.output_state.ordered_color_targets[0], output, role)?;
    }
    Ok(())
}

fn validate_output_target(
    actual: &super::RenderTargetDesc,
    expected: &S1TargetBinding,
    role: &str,
) -> Result<(), String> {
    let Some(exact) = &actual.exact_resource else {
        return Err(format!(
            "S1 {role} target lacks an exact resource descriptor"
        ));
    };
    if actual.prepared_target != expected.prepared_target
        || actual.width != expected.width
        || actual.height != expected.height
        || actual.dxgi_format_u32 != expected.dxgi_format_u32
        || actual.mip_level != 0
        || actual.sample_count != 1
        || actual.sample_quality != 0
        || actual.persistent_content.is_some()
        || !exact.ordered_initial_subresources.is_empty()
        || exact.resource.dimension != super::TextureResourceDimension::Texture2d
        || exact.resource.width != expected.width
        || exact.resource.height != expected.height
        || exact.resource.depth != 1
        || exact.resource.mip_levels != expected.mip_levels
        || exact.resource.array_size != 1
        || exact.resource.dxgi_format_u32 != expected.dxgi_format_u32
        || exact.resource.sample_count != 1
        || exact.resource.sample_quality != 0
        || exact.resource.usage_u32 != 0
        || exact.resource.bind_flags_u32 != expected.bind_flags_u32
        || exact.resource.cpu_access_flags_u32 != 0
        || exact.resource.misc_flags_u32 != 0
    {
        return Err(format!("S1 {role} target/resource descriptor drift"));
    }
    let rtv_count = exact
        .ordered_views
        .iter()
        .filter(|view| {
            matches!(
                view,
                super::RenderTargetViewDesc::RenderTarget {
                    dxgi_format_u32,
                    view: super::RenderTargetAttachmentView::Texture2d { mip_slice: 0 },
                } if *dxgi_format_u32 == expected.dxgi_format_u32
            )
        })
        .count();
    let srv_count = exact
        .ordered_views
        .iter()
        .filter(|view| {
            matches!(
                view,
                super::RenderTargetViewDesc::ShaderResource { view }
                    if srv_matches(view, expected)
            )
        })
        .count();
    if exact.ordered_views.len() != 2 || rtv_count != 1 || srv_count != 1 {
        return Err(format!(
            "S1 {role} target does not carry exact RTV/SRV views"
        ));
    }
    Ok(())
}

fn texture_resource_matches(
    actual: &super::TextureResourceDesc,
    expected: &S1TargetBinding,
) -> bool {
    actual.dimension == super::TextureResourceDimension::Texture2d
        && actual.width == expected.width
        && actual.height == expected.height
        && actual.depth == 1
        && actual.mip_levels == expected.mip_levels
        && actual.array_size == 1
        && actual.dxgi_format_u32 == expected.dxgi_format_u32
        && actual.sample_count == 1
        && actual.sample_quality == 0
        && actual.usage_u32 == 0
        && actual.bind_flags_u32 == expected.bind_flags_u32
        && actual.cpu_access_flags_u32 == 0
        && actual.misc_flags_u32 == 0
        && actual.ordered_subresources.is_empty()
}

fn srv_matches(actual: &super::ShaderResourceViewDesc, expected: &S1TargetBinding) -> bool {
    matches!(
        actual,
        super::ShaderResourceViewDesc::Texture2d {
            dxgi_format_u32,
            most_detailed_mip: 0,
            mip_levels,
        } if *dxgi_format_u32 == expected.dxgi_format_u32
            && *mip_levels == expected.mip_levels
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inactive_carrier() -> S1ShadowCarrier {
        S1ShadowCarrier {
            schema: S1_SHADOW_SCHEMA.into(),
            executable_sha256: S1_A729_EXECUTABLE_SHA256.into(),
            l02_authority_sha256: S1_L02_AUTHORITY_SHA256.into(),
            l03_authority_sha256: S1_L03_AUTHORITY_SHA256.into(),
            s1_chain_authority_sha256: S1_CHAIN_AUTHORITY_SHA256.into(),
            shadow_pass_active: false,
            blur_camera_factor_f32_bits: 0.0f32.to_bits(),
            selected_light: None,
            light_view_projection_f32_bits: None,
            shadow_map_texture_matrix_f32_bits: None,
            camera_y_f32_bits: None,
            enabled_partition_mask: None,
            light_depth_target: None,
            producer_and_final_target: None,
            blur_intermediate_target: None,
            light_depth_clear: None,
            ordered_light_depth_submissions: Vec::new(),
            ordered_terrain_unlit_draws: Vec::new(),
            horizontal_blur_draw: None,
            vertical_blur_draw: None,
            ordered_shadow_consumers: Vec::new(),
            payload_sha256: String::new(),
        }
    }

    #[test]
    fn inactive_payload_seals_without_active_resource_state() {
        let mut carrier = inactive_carrier();
        carrier.payload_sha256 = carrier.computed_payload_sha256().unwrap();
        assert!(carrier.is_inactive());
        assert_eq!(carrier.payload_sha256.len(), 64);
    }

    #[test]
    fn exact_target_gate_rejects_swapchain_and_descriptor_drift() {
        let mut target = S1TargetBinding {
            stable_resource_id: "screen-a".into(),
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 7,
            },
            width: 1920,
            height: 1080,
            dxgi_format_u32: S1_SCREEN_DXGI_FORMAT,
            mip_levels: S1_TARGET_MIP_LEVELS,
            bind_flags_u32: S1_TARGET_BIND_FLAGS,
        };
        validate_target(&target, 1920, 1080, S1_SCREEN_DXGI_FORMAT, "fixture").unwrap();
        target.prepared_target = PreparedRenderTarget::SwapchainBackBuffer;
        assert!(
            validate_target(&target, 1920, 1080, S1_SCREEN_DXGI_FORMAT, "fixture")
                .unwrap_err()
                .contains("descriptor")
        );
    }

    #[test]
    fn l03_projection_shape_rejects_tree_before_pdxmesh() {
        let draw = S1DrawArguments::DrawIndexed {
            index_count: 3,
            start_index: 0,
            base_vertex: 0,
        };
        let tree = S1LightDepthSubmission::Tree {
            partition_mask: S1_PARTITION_PRIMARY,
            source_row_ordinal: 7,
            selected_mesh_ordinal: 0,
            selected_lod_index: 0,
            selected_mesh: "gfx/models/tree.mesh".into(),
            selected_material: "tree_shadow".into(),
            pipeline: S1ShadowPipeline::TreeShadow,
            draw,
            execution: RenderExecutionRef::Static {
                operation_ordinal: 1,
            },
        };
        assert!(!serde_json::to_string(&tree)
            .unwrap()
            .contains("selected_mesh_ordinal"));
        let mut second_mesh = tree.clone();
        if let S1LightDepthSubmission::Tree {
            selected_mesh_ordinal,
            ..
        } = &mut second_mesh
        {
            *selected_mesh_ordinal = 1;
        }
        assert!(serde_json::to_string(&second_mesh)
            .unwrap()
            .contains("\"selected_mesh_ordinal\":1"));
        assert!(validate_l03_projection_order(&[tree.clone(), second_mesh]).is_ok());
        let object = S1LightDepthSubmission::Pdxmesh {
            partition_mask: S1_PARTITION_PRIMARY,
            stable_typed_upstream_id: "m01:1".into(),
            owner: S1TypedOwner::M01Unit,
            world_matrix_f32_bits: [0; 16],
            selected_mesh: "gfx/models/unit.mesh".into(),
            selected_submesh_material: "unit_shadow".into(),
            group_ordinal: 0,
            submesh_ordinal: 0,
            pipeline: S1ShadowPipeline::StandardUnskinned,
            draw,
            execution: RenderExecutionRef::Static {
                operation_ordinal: 2,
            },
        };
        assert!(validate_l03_projection_order(&[object.clone(), tree.clone()]).is_ok());
        assert!(validate_l03_projection_order(&[tree, object])
            .unwrap_err()
            .contains("after the first tree"));
    }
}
