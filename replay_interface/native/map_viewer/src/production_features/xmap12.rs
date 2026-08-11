//! XMAP12 cities, buildings, and ambient/fixed-object production adapter.
//!
//! City population is regenerated from admitted effective installed content.
//! Building presentation and ambient lifecycle are accepted only through their
//! authenticated per-frame carriers. Missing carriers produce one explicit
//! unavailable family row; logical D48 values, recorder draws, and defaults
//! are never substituted.

use crate::{
    prepare_xmap12_buildings, prepare_xmap12_cities, prepare_xmap12_fixed_objects,
    prepare_xmap12_render_frame, resolve_xmap12_entity_chain, FoundationFeatureContext,
    FoundationFeatureOutput, GameFileResolver, PreparedXmap12RenderFrame, Result,
    Xmap12BuildingVisualFrame, Xmap12FixedRuntimeState, XMAP12_RENDER_FRAME_SCHEMA,
};
use portable_render_stream::{FamilyDisposition, FoundationGap};
use replay_core::{UsedContentKey, ValidatedContentAdmission, ViewStatus};
use snapshot::sha256::Sha256;
use std::collections::BTreeSet;

pub(crate) const XMAP12_EXACT_INPUT_GAP_ID: &str = "XMAP12.exact_per_frame_inputs_unavailable";

pub(crate) struct Xmap12FeaturePreparation {
    pub output: FoundationFeatureOutput,
    pub prepared: Option<PreparedXmap12RenderFrame>,
}

pub(crate) struct Xmap12CitiesBuildingsMapObjectsAdapter {
    resolver: GameFileResolver,
    admission: ValidatedContentAdmission,
    texture_quality: u8,
}

impl Xmap12CitiesBuildingsMapObjectsAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        terrain_options: crate::TerrainLodCullOptions,
    ) -> Result<Self> {
        static_content_paths(terrain_options.texture_quality)?;
        resolver.require_complete_admitted_dir(
            "",
            Some("asset"),
            true,
            "XMAP12 asset namespace",
        )?;
        resolver.require_complete_admitted_dir("", Some("gfx"), true, "XMAP12 GFX namespace")?;
        Ok(Self {
            resolver: resolver.clone(),
            admission: admission.clone(),
            texture_quality: terrain_options.texture_quality,
        })
    }

    pub(crate) fn prepare(
        &self,
        context: &FoundationFeatureContext<'_>,
        building_visual: Option<&Xmap12BuildingVisualFrame>,
        fixed_runtime: Option<&Xmap12FixedRuntimeState>,
    ) -> Result<Xmap12FeaturePreparation> {
        let consumed_gap_ids = owned_gap_ids(context);
        let mut missing = Vec::new();
        if context.viewer.camera.is_none() {
            missing.push("the XMAP03 viewer camera");
        }
        if context.world_frame.d48_buildings_status() != ViewStatus::Present {
            missing.push("the present D48 building domain");
        }
        if building_visual.is_none() {
            missing.push("the authenticated per-placement building visual carrier");
        }
        if fixed_runtime.is_none() {
            missing.push("the authenticated ambient/fixed lifecycle carrier");
        }
        if !missing.is_empty() {
            let reason = format!(
                "XMAP12 cannot prepare cities/buildings/map objects because {} is unavailable; Replay will not infer presentation from logical D48 state, reuse a prior hour, or synthesize ambient lifecycle",
                missing.join(", ")
            );
            return Ok(Xmap12FeaturePreparation {
                output: empty_output(
                    FamilyDisposition::Unavailable {
                        gap: unavailable_gap(context.world_frame.hour_index(), reason),
                    },
                    consumed_gap_ids,
                ),
                prepared: None,
            });
        }

        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP12 exact input admission lost the XMAP03 viewer camera")?;
        let building_visual = building_visual
            .ok_or("XMAP12 exact input admission lost the building visual carrier")?;
        let fixed_runtime =
            fixed_runtime.ok_or("XMAP12 exact input admission lost the fixed runtime carrier")?;
        building_visual.validate()?;
        let replay_frame_identity_sha256 =
            crate::xmap04_replay_frame_identity_sha256(context.world_frame)?;
        if building_visual.replay_frame_identity_sha256 != replay_frame_identity_sha256 {
            return Err(
                "XMAP12 building visual carrier belongs to a different replay frame".into(),
            );
        }
        fixed_runtime.validate_for_replay(&replay_frame_identity_sha256)?;

        let cities = prepare_xmap12_cities(&self.resolver, &self.admission, camera)?;
        let buildings = prepare_xmap12_buildings(
            &self.resolver,
            &self.admission,
            context.world_frame,
            camera,
            building_visual,
        )?;
        let fixed =
            prepare_xmap12_fixed_objects(&self.resolver, &self.admission, camera, fixed_runtime)?;
        let [fow_noise_path, fow_height_path, environment_map_path] =
            static_content_paths(self.texture_quality)?;
        let fow_noise_content =
            admitted_static_content(&self.resolver, &self.admission, &fow_noise_path)?;
        let fow_height_content =
            admitted_static_content(&self.resolver, &self.admission, &fow_height_path)?;
        let environment_map_content =
            admitted_static_content(&self.resolver, &self.admission, &environment_map_path)?;
        let prepared = prepare_xmap12_render_frame(
            &cities,
            &buildings,
            &fixed,
            fow_noise_content,
            fow_height_content,
            environment_map_content,
        )?;
        prepared.validate()?;
        if prepared.replay_frame_identity_sha256 != replay_frame_identity_sha256 {
            return Err("XMAP12 prepared payload belongs to a different replay frame".into());
        }
        if prepared.viewer_state_sha256 != camera.deterministic_sha256 {
            return Err("XMAP12 prepared payload belongs to a different viewer camera".into());
        }

        let ordered_used_content = ordered_used_content(
            &self.resolver,
            &self.admission,
            &cities,
            &buildings,
            &fixed,
            &prepared,
        )?;
        let registration = Sha256::hex(
            format!(
                "XMAP12\0{XMAP12_RENDER_FRAME_SCHEMA}\0cities\0buildings\0ambient_objects\0MainScene\0L03"
            )
            .as_bytes(),
        );
        Ok(Xmap12FeaturePreparation {
            output: empty_output(
                FamilyDisposition::Present {
                    payload_schema: XMAP12_RENDER_FRAME_SCHEMA.into(),
                    producer_registration_sha256: registration,
                    payload_sha256: prepared.deterministic_sha256.clone(),
                    // XMAP12 consumes MainScene and XMAP09's L03 pass. Neither
                    // owns a new assigned-stage edge, so no operation is
                    // fabricated here.
                    ordered_operations: Vec::new(),
                    ordered_used_content,
                },
                consumed_gap_ids,
            ),
            prepared: Some(prepared),
        })
    }
}

fn static_content_paths(texture_quality: u8) -> Result<[String; 3]> {
    if texture_quality > 2 {
        return Err(format!(
            "XMAP12 texture_quality must be in the exact engine domain 0..=2, got {texture_quality}"
        )
        .into());
    }
    Ok([
        format!("map/terrain/fow_noise_{texture_quality}.dds"),
        "map/terrain/fow_rgb_waterspec_a.dds".into(),
        "map/terrain/reflection.dds".into(),
    ])
}

fn admitted_static_content(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    path: &str,
) -> Result<UsedContentKey> {
    let content = admission
        .require_used_content(path)
        .map_err(|error| format!("XMAP12 requires admitted engine texture {path}: {error}"))?
        .clone();
    resolver.read_admitted_used_content(&content)?;
    Ok(content)
}

fn empty_output(
    disposition: FamilyDisposition,
    consumed_gap_ids: Vec<String>,
) -> FoundationFeatureOutput {
    FoundationFeatureOutput {
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
        consumed_gap_ids,
    }
}

fn owned_gap_ids(context: &FoundationFeatureContext<'_>) -> Vec<String> {
    context
        .map_authority_gaps
        .iter()
        .filter(|gap| gap.input_id.starts_with("objects."))
        .map(|gap| gap.input_id.clone())
        .collect()
}

fn unavailable_gap(hour: i64, reason: String) -> FoundationGap {
    let witness_sha256 =
        Sha256::hex(format!("{XMAP12_EXACT_INPUT_GAP_ID}\n{hour}\n{hour}\n{reason}").as_bytes());
    FoundationGap {
        gap_id: XMAP12_EXACT_INPUT_GAP_ID.into(),
        first_unavailable_hour: hour,
        last_unavailable_hour: hour,
        witness_sha256,
        reason,
    }
}

fn ordered_used_content(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    cities: &crate::PreparedXmap12CityFrame,
    buildings: &crate::PreparedXmap12BuildingFrame,
    fixed: &crate::PreparedXmap12FixedFrame,
    prepared: &PreparedXmap12RenderFrame,
) -> Result<Vec<UsedContentKey>> {
    let mut ordered = Vec::new();
    let mut paths = BTreeSet::new();
    let mut push = |content: &UsedContentKey| {
        let path = content
            .normalized_game_relative_path
            .replace('\\', "/")
            .to_ascii_lowercase();
        if paths.insert(path) {
            ordered.push(content.clone());
        }
    };
    for content in [
        &cities.cities_source,
        &cities.types_source,
        &cities.terrain_source,
        &cities.rivers_source,
        &cities.heightmap_source,
        &cities.province_source,
        &cities.definition_source,
    ] {
        push(content);
    }
    for content in &cities.terrain_definition_sources {
        push(content);
    }
    push(&buildings.map_buildings_source);
    for content in &buildings.common_building_sources {
        push(content);
    }
    for content in &buildings.defines_sources {
        push(content);
    }
    push(&prepared.fow_noise_content);
    push(&prepared.fow_height_content);
    push(&prepared.environment_map_content);
    push(&fixed.default_map_source);
    push(&fixed.ambient_source);
    for draw in cities
        .ordered_visible_draws
        .iter()
        .chain(&buildings.ordered_visible_draws)
        .chain(&fixed.ordered_visible_draws)
    {
        let chain = resolve_xmap12_entity_chain(resolver, admission, &draw.entity_token)?;
        push(&chain.entity_source);
        push(&chain.pdxmesh_source);
        push(&chain.mesh_source);
        push(&chain.shader_source);
        for texture in &chain.ordered_texture_sources {
            push(&texture.content);
        }
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_map_resources_follow_exact_quality_selection_and_fixed_paths() {
        for quality in 0..=2 {
            assert_eq!(
                static_content_paths(quality).unwrap(),
                [
                    format!("map/terrain/fow_noise_{quality}.dds"),
                    "map/terrain/fow_rgb_waterspec_a.dds".into(),
                    "map/terrain/reflection.dds".into(),
                ]
            );
        }
        assert!(static_content_paths(3).is_err());
    }
}
