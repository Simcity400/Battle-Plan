use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, ProductionMapFeatureAdapter, Result,
    VIEWER_MAP_CAMERA_SCHEMA, XMAP03_A729_EXECUTABLE_SHA256, XMAP03_CAMERA_LAW_SCHEMA,
};
use portable_render_stream::{FamilyDisposition, FoundationOperationIdentity, MapVisualFamily};
use snapshot::sha256::Sha256;

pub(crate) struct Xmap03CameraAdapter;

impl ProductionMapFeatureAdapter for Xmap03CameraAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::CameraView
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP03 product camera state is unavailable")?;
        camera.validate()?;
        let registration = Sha256::hex(
            format!(
                "XMAP03\0{VIEWER_MAP_CAMERA_SCHEMA}\0{XMAP03_CAMERA_LAW_SCHEMA}\0{XMAP03_A729_EXECUTABLE_SHA256}"
            )
            .as_bytes(),
        );
        let content_identity = Sha256::hex(&serde_json::to_vec(&camera.graphics_defines_content)?);
        let target_identity = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}",
                camera.map_width, camera.map_height, camera.viewport_width, camera.viewport_height
            )
            .as_bytes(),
        );
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: VIEWER_MAP_CAMERA_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: camera.deterministic_sha256.clone(),
                ordered_operations: vec![FoundationOperationIdentity {
                    operation_id: "viewer.camera.prepare".into(),
                    global_order: 0,
                    program_sha256: registration,
                    ordered_resources_sha256: content_identity,
                    target_sha256: target_identity,
                }],
                ordered_used_content: vec![camera.graphics_defines_content.clone()],
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: None,
            prepared_rail_supply: None,
            prepared_weather: None,
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            consumed_gap_ids: Vec::new(),
        })
    }
}
