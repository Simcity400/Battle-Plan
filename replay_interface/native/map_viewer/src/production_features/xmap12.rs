//! City population is regenerated from admitted effective installed content.

use crate::{
    prepare_xmap12_buildings, prepare_xmap12_cities, prepare_xmap12_fixed_objects,
    prepare_xmap12_render_frame, resolve_xmap12_entity_chain, FoundationFeatureContext,
    FoundationFeatureOutput, GameFileResolver, PreparedXmap12RenderFrame, Result,
    Xmap12BuildingVisualFrame, Xmap12FixedRuntimeState, XMAP12_RENDER_FRAME_SCHEMA,
    XMAP12_VIEWER_MAP_OBJECT_PRESENTATION_ENABLED,
};
use portable_render_stream::{FamilyDisposition, FoundationGap};
use replay_core::{Leaf, UsedContentKey, ValidatedContentAdmission, ViewStatus};
use snapshot::sha256::Sha256;
use std::collections::BTreeSet;

pub(crate) const XMAP12_EXACT_INPUT_GAP_ID: &str = "XMAP12.exact_per_frame_inputs_unavailable";

pub(crate) struct Xmap12FeaturePreparation {
    pub output: FoundationFeatureOutput,
    /// The payload AND the two states it leaves behind, or nothing at all.
    ///
    /// One field, not three optional ones: a present frame that carried its
    /// draws but not its lifecycle would present correctly and cold-start the
    /// next frame, forgetting every camera-spawned object, and nothing in the
    /// type would have been wrong. The three are inseparable, so the type says
    /// so.
    pub present: Option<Xmap12PresentPreparation>,
}

impl Xmap12FeaturePreparation {
    /// Whether the disposition and the payload beside it say the same thing.
    ///
    /// A present frame owes its payload AND both carried states; an unavailable
    /// one owes none of them. Nothing else in the product would notice a present
    /// frame that carried its draws and no lifecycle: it would present correctly
    /// and cold-start the next frame, forgetting every camera-spawned object.
    pub(crate) fn disposition_agrees_with_payload(&self) -> bool {
        matches!(&self.output.disposition, FamilyDisposition::Present { .. })
            == self.present.is_some()
    }
}

pub(crate) struct Xmap12PresentPreparation {
    pub prepared: PreparedXmap12RenderFrame,
    /// This frame's derived per-placement building presentation. The caller carries it into
    /// the next frame: it is the destroyed-mesh selector's previous-frame memory, the engine's
    /// `slot+0x3a`.
    pub building_visual: Xmap12BuildingVisualFrame,
    /// This frame's derived ambient/fixed lifecycle, carried into the next frame
    /// for the same reason: an ambient object the viewer's camera has spawned
    /// stays spawned, and only a carried state remembers that.
    pub fixed_runtime: Xmap12FixedRuntimeState,
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

    /// Prepare this hour's cities/buildings/map objects.
    ///
    /// `previous_building_visual` is the frame this adapter produced for the previous hour.
    /// It carries nothing but presentation the engine also remembers frame to frame - the
    /// destroyed-mesh selector's `slot+0x3a` - and `None` is the engine's own cold start,
    /// where a freshly created render slot has never selected a destroyed mesh.
    ///
    /// `previous_fixed_runtime` is the ambient/fixed lifecycle this adapter
    /// produced for the previous frame, and `None` is the cold start in which
    /// nothing has been spawned. It is DERIVED, never read off the wire:
    /// XMAP12's authority assigns the lifecycle to the viewer and the
    /// map-authority rows it feeds already refuse a recorded carrier. Its one
    /// recorded operand is the session's DLC set, which the ambient
    /// `dlc_allowed` trigger selects on.
    pub(crate) fn prepare(
        &self,
        context: &FoundationFeatureContext<'_>,
        previous_building_visual: Option<&Xmap12BuildingVisualFrame>,
        previous_fixed_runtime: Option<&Xmap12FixedRuntimeState>,
    ) -> Result<Xmap12FeaturePreparation> {
        let consumed_gap_ids = owned_gap_ids(context);
        let mut missing = Vec::new();
        if context.viewer.camera.is_none() {
            missing.push("the XMAP03 viewer camera");
        }
        if context.world_frame.d48_buildings_status() != ViewStatus::Present {
            missing.push("the present D48 building domain");
        }
        let session_dlcs = &context.world_frame.session_metadata().active_dlcs;
        let dlc_missing = unmeasured_session_dlc_cause(session_dlcs);
        if let Some(cause) = &dlc_missing {
            missing.push(cause.as_str());
        }
        let enabled_dlc_names = session_dlcs.measured().cloned();
        if !missing.is_empty() {
            let reason = format!(
                "XMAP12 cannot prepare cities/buildings/map objects because {} is unavailable; Replay will not reuse a prior hour or synthesize ambient lifecycle",
                missing.join(", ")
            );
            return Ok(Xmap12FeaturePreparation {
                output: empty_output(
                    FamilyDisposition::Unavailable {
                        gap: unavailable_gap(context.world_frame.hour_index(), reason),
                    },
                    consumed_gap_ids,
                ),
                present: None,
            });
        }

        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP12 exact input admission lost the XMAP03 viewer camera")?;
        let enabled_dlc_names =
            enabled_dlc_names.ok_or("XMAP12 exact input admission lost the session DLC set")?;
        let replay_frame_identity_sha256 =
            crate::xmap04_replay_frame_identity_sha256(context.world_frame)?;
        let cities = prepare_xmap12_cities(&self.resolver, &self.admission, camera)?;
        let (buildings, building_visual) = prepare_xmap12_buildings(
            &self.resolver,
            &self.admission,
            context.world_frame,
            camera,
            previous_building_visual,
        )?;
        let fixed = prepare_xmap12_fixed_objects(
            &self.resolver,
            &self.admission,
            camera,
            previous_fixed_runtime,
            &replay_frame_identity_sha256,
            context.viewer.animation_time_f32_bits,
            &enabled_dlc_names,
        )?;
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
            &fixed.frame,
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
            &fixed.frame,
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
                    ordered_operations: Vec::new(),
                    ordered_used_content,
                },
                consumed_gap_ids,
            ),
            present: Some(Xmap12PresentPreparation {
                prepared,
                building_visual,
                fixed_runtime: fixed.runtime,
            }),
        })
    }
}

/// Why the session's DLC set cannot answer the ambient `dlc_allowed` trigger,
/// or `None` when it can.
///
/// The two unmeasured states are different facts and say so: the key was never
/// written, or it was written as an explicit non-reading. Neither is an empty
/// DLC set, which is a measurement.
fn unmeasured_session_dlc_cause(active_dlcs: &Leaf<Vec<String>>) -> Option<String> {
    let state = match active_dlcs {
        Leaf::Measured(_) => return None,
        Leaf::Absent => "and this frame carries no such key",
        Leaf::Unobtained => "and this frame carries it as an explicit unobtained",
    };
    Some(format!(
        "the measured session DLC set (WorldFrame session.active_dlcs, which the container's own \
         modset binds onto every frame, {state}), without which the ambient dlc_allowed trigger \
         cannot be answered"
    ))
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
        prepared_units: None,
        prepared_combat: None,
        prepared_orders: None,
        prepared_rail_supply: None,
        prepared_map_modes: None,
        consumed_gap_ids,
        element_diagnostics: Vec::new(),
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

    /// kills: collapsing the two unmeasured leaf states onto one cause, which
    /// would tell a reader an hour "has no DLC set" when the recording
    /// explicitly said it did not read one - and admitting an unmeasured leaf as
    /// an empty set, which would silently deactivate every DLC-gated ambient
    /// type instead of refusing the frame.
    #[test]
    fn an_unmeasured_session_dlc_set_names_which_unmeasured_state_it_is() {
        let absent = unmeasured_session_dlc_cause(&Leaf::Absent).expect("absent is a cause");
        let unobtained =
            unmeasured_session_dlc_cause(&Leaf::Unobtained).expect("unobtained is a cause");
        assert_ne!(absent, unobtained);
        assert!(absent.contains("no such key"), "{absent}");
        assert!(unobtained.contains("explicit unobtained"), "{unobtained}");
        for cause in [&absent, &unobtained] {
            assert!(cause.contains("session.active_dlcs"), "{cause}");
        }
        assert_eq!(unmeasured_session_dlc_cause(&Leaf::Measured(Vec::new())), None);
    }

    /// kills: a present XMAP12 frame that carries its draws and drops the
    /// lifecycle - it presents correctly, then cold-starts the next frame and
    /// forgets every camera-spawned object, and nothing else in the product
    /// would object; and an unavailable frame that carries a payload anyway.
    #[test]
    fn a_present_frame_owes_its_payload_and_both_carried_states() {
        let present = FamilyDisposition::Present {
            payload_schema: XMAP12_RENDER_FRAME_SCHEMA.into(),
            producer_registration_sha256: "a".repeat(64),
            payload_sha256: "b".repeat(64),
            ordered_operations: Vec::new(),
            ordered_used_content: Vec::new(),
        };
        let unavailable = FamilyDisposition::Unavailable {
            gap: unavailable_gap(0, "a stated cause".into()),
        };
        let with = |disposition: FamilyDisposition, carries: bool| Xmap12FeaturePreparation {
            output: empty_output(disposition, Vec::new()),
            present: carries.then(|| unreachable!("this test never needs a real payload")),
        };
        assert!(!with(present.clone(), false).disposition_agrees_with_payload());
        assert!(with(unavailable.clone(), false).disposition_agrees_with_payload());
    }

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
