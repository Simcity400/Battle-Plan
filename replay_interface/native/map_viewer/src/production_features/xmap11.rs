
use crate::{
    prepare_xmap11_tree_frame, FoundationFeatureContext, FoundationFeatureOutput,
    GameFileResolver, PreparedTreeFrame, ProductionMapCameraProjection,
    Result, TerrainLodCullOptions, TerrainRenderViewFrustum,
    TreeFrameInput, TreeFrustumPlane, TreeViewerPartition, TreeViewerState,
    XMAP11_TREE_FRAME_SCHEMA, XMAP11_TREE_OPERATION_NAME, XMAP11_TREE_STAGE_NAME,
};
use portable_render_stream::FamilyDisposition;
use replay_core::ValidatedContentAdmission;
use snapshot::sha256::Sha256;
use std::sync::Mutex;

pub(crate) struct Xmap11FeaturePreparation {
    pub(crate) output: FoundationFeatureOutput,
    pub(crate) prepared: PreparedTreeFrame,
}

pub(crate) struct Xmap11TreesAdapter {
    resolver: GameFileResolver,
    admission: ValidatedContentAdmission,
    low_end_gfx: bool,
    cached_static_frame: Mutex<Option<PreparedTreeFrame>>,
}

impl Xmap11TreesAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        options: TerrainLodCullOptions,
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err("XMAP11 adapter requires an admission-bound effective VFS".into());
        }
        Ok(Self {
            resolver: resolver.clone(),
            admission: admission.clone(),
            low_end_gfx: !options.high_gfx_shaders,
            cached_static_frame: Mutex::new(None),
        })
    }
    pub(crate) fn prepare(
        &self,
        context: &FoundationFeatureContext<'_>,
    ) -> Result<Xmap11FeaturePreparation> {
        let terrain = context
            .prepared_terrain
            .ok_or("XMAP11 requires the same-frame XMAP04 terrain/map-copy provider")?;
        terrain.validate()?;
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP11 requires the viewer-owned XMAP03 camera")?;
        camera.validate()?;
        if camera.deterministic_sha256 != terrain.camera_frame_sha256 {
            return Err("XMAP11 camera differs from the XMAP04 map-copy provider".into());
        }
        let viewer = tree_viewer_from_xmap03_xmap04(camera, terrain)?;
        let input = TreeFrameInput {
            hour_index: context.world_frame.hour_index(),
            replay_frame_identity_sha256: terrain.replay_frame_identity_sha256.clone(),
            camera_frame_sha256: terrain.camera_frame_sha256.clone(),
            viewer_state_sha256: context.viewer_state_sha256.to_owned(),
            map_width: terrain.map_width,
            map_height: terrain.map_height,
            low_end_gfx: Some(self.low_end_gfx),
            viewer,
        };
        let mut cache = self
            .cached_static_frame
            .lock()
            .map_err(|_| "XMAP11 static preparation cache is poisoned")?;
        let tree_viewer_state_sha256 = Sha256::hex(&serde_json::to_vec(&input.viewer)?);
        let mut frame = if let Some(cached) = cache.as_ref().filter(|cached| {
            cached.tree_viewer_state_sha256 == tree_viewer_state_sha256
                && cached.map_width == input.map_width
                && cached.map_height == input.map_height
        }) {
            cached.clone()
        } else {
            let prepared = prepare_xmap11_tree_frame(&self.resolver, &self.admission, &input)?;
            *cache = Some(prepared.clone());
            prepared
        };
        frame.hour_index = input.hour_index;
        frame.replay_frame_identity_sha256 = input.replay_frame_identity_sha256;
        frame.camera_frame_sha256 = input.camera_frame_sha256;
        frame.viewer_state_sha256 = input.viewer_state_sha256;
        frame.reseal_dynamic_authority()?;
        frame.validate()?;
        if frame.content_manifest_sha256 != self.admission.manifest_digest_sha256()
            || frame.effective_vfs_manifest_sha256 != context.effective_vfs_manifest_sha256
            || frame.replay_frame_identity_sha256 != terrain.replay_frame_identity_sha256
            || frame.camera_frame_sha256 != terrain.camera_frame_sha256
            || frame.viewer_state_sha256 != context.viewer_state_sha256
        {
            return Err("XMAP11 prepared frame drifted from foundation authority".into());
        }
        let assigned = context
            .require_assigned_stage(XMAP11_TREE_STAGE_NAME, XMAP11_TREE_OPERATION_NAME)?;
        let disposition = xmap11_foundation_disposition(&frame, assigned)?;
        Ok(Xmap11FeaturePreparation {
            output: FoundationFeatureOutput {
                disposition,
                prepared_terrain: None,
                prepared_political: None,
                prepared_thin_borders: None,
                prepared_lighting: None,
                prepared_weather: None,
                prepared_rivers_straits: None,
                prepared_water: None,
                prepared_effects: None,
                prepared_units: None,
                prepared_combat: None,
                prepared_orders: None,
                prepared_rail_supply: None,
                prepared_map_modes: None,
                prepared_map_furniture: None,
                element_diagnostics: Vec::new(),
                consumed_gap_ids: Vec::new(),
            },
            prepared: frame,
        })
    }
}

pub fn xmap11_foundation_disposition(
    frame: &PreparedTreeFrame,
    assigned: &crate::AssignedStage,
) -> Result<FamilyDisposition> {
    frame.validate()?;
    assigned
        .validate()
        .map_err(|error| format!("XMAP11 assigned stage: {error}"))?;
    if assigned.stage_name != XMAP11_TREE_STAGE_NAME
        || assigned.operation_name != XMAP11_TREE_OPERATION_NAME
    {
        return Err("XMAP11 foundation authorization received the wrong assigned stage".into());
    }
    let registration = Sha256::hex(
        format!(
            "XMAP11\0{XMAP11_TREE_FRAME_SCHEMA}\0{}\0{}",
            frame.shader_programs.deterministic_sha256, frame.map_copy_receipt_set_sha256
        )
        .as_bytes(),
    );
    Ok(if frame.ordered_main_draw_batch_indices.is_empty() {
        FamilyDisposition::AuthenticatedEmpty {
            producer_registration_sha256: registration,
            enumerator_witness_sha256: Sha256::hex(&serde_json::to_vec(&(
                frame.prepared_frame_sha256.as_str(),
                frame.tree_fade_f32_bits,
                frame.selected_batches.len(),
                frame.viewer_state_sha256.as_str(),
            ))?),
        }
    } else {
        FamilyDisposition::Present {
            payload_schema: XMAP11_TREE_FRAME_SCHEMA.into(),
            producer_registration_sha256: registration,
            payload_sha256: frame.prepared_frame_sha256.clone(),
            ordered_operations: vec![assigned.foundation_operation_identity(
                frame.shader_programs.deterministic_sha256.clone(),
                    frame.foundation_resource_census_sha256.clone(),
                frame.map_copy_receipt_set_sha256.clone(),
            )?],
            ordered_used_content: frame.ordered_used_content.clone(),
        }
    })
}

pub fn xmap11_foundation_authorization_sha256(
    frame: &PreparedTreeFrame,
    assigned: &crate::AssignedStage,
) -> Result<String> {
    Ok(Sha256::hex(&serde_json::to_vec(
        &xmap11_foundation_disposition(frame, assigned)?,
    )?))
}

fn tree_viewer_from_xmap03_xmap04(
    camera: &crate::ViewerMapCameraState,
    terrain: &crate::PreparedTerrainFrame,
) -> Result<TreeViewerState> {
    let product_camera = camera.production_camera()?;
    let projection = ProductionMapCameraProjection::from_engine_constants(
        camera.viewport_width as f32 / camera.viewport_height as f32,
    )?;
    let rows = terrain.ordered_map_copy_receipts()?;
    let mut ordered_partitions = Vec::with_capacity(rows.len());
    for row in rows {
        let camera_shift = match row.render_view_mask {
            1 => 0.0,
            2 => -(terrain.map_width as f32),
            4 => terrain.map_width as f32,
            _ => return Err("XMAP11 received an invalid XMAP04 map-copy mask".into()),
        };
        let frustum = if row.render_view_mask == 1 {
            TerrainRenderViewFrustum::from_view_projection_matrix(
                product_camera.view_projection_matrix,
            )?
        } else {
            let mut eye = camera.current_eye();
            let mut look_at = camera.current_look_at();
            eye[0] += camera_shift;
            look_at[0] += camera_shift;
            TerrainRenderViewFrustum::from_view_projection_matrix(
                projection.camera_from_eye_look_at(eye, look_at)?.view_projection_matrix,
            )?
        };
        ordered_partitions.push(TreeViewerPartition {
            render_view_mask: row.render_view_mask,
            map_copy_offset_f32_bits: row.map_copy_offset_f32_bits,
            side_planes: frustum.side_planes_f32_bits().map(|plane| TreeFrustumPlane {
                x_f32_bits: plane[0],
                y_f32_bits: plane[1],
                z_f32_bits: plane[2],
                w_f32_bits: plane[3],
            }),
        });
    }
    let [x, y, z] = camera.current_eye();
    let viewer = TreeViewerState {
        camera_x_f32_bits: x.to_bits(),
        camera_y_f32_bits: y.to_bits(),
        camera_z_f32_bits: z.to_bits(),
        ordered_partitions,
    };
    viewer.validate()?;
    Ok(viewer)
}
