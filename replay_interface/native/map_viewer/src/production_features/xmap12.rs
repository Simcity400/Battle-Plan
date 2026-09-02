//! City population is regenerated from admitted effective installed content.

use crate::{
    admit_xmap12_ambient_lifecycle, prepare_xmap12_buildings, prepare_xmap12_cities,
    prepare_xmap12_fixed_objects, prepare_xmap12_render_frame, resolve_xmap12_entity_chain,
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    PreparedXmap12RenderFrame, Result, Xmap12BuildingVisualFrame, Xmap12FixedRuntimeState,
    XMAP12_RENDER_FRAME_SCHEMA,
};
use crate::map_authority::AmbientLifecycleRefusal;
use portable_render_stream::{FamilyDisposition, FoundationGap};
use replay_core::{UsedContentKey, ValidatedContentAdmission, ViewStatus};
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
        // THE ADMISSION GATES, THEN THE DERIVATION READS. The lifecycle's one
        // recorded operand - the session DLC set - reaches
        // `prepare_xmap12_fixed_objects` only as the token its XMAP00 row mints,
        // so an hour the row refuses cannot reach the derivation at all, and
        // there is no second read of the leaf here for the two to disagree over.
        let lifecycle = admit_xmap12_ambient_lifecycle(context);
        let dlc_missing = lifecycle
            .as_ref()
            .err()
            .map(|refused| unmeasured_session_dlc_cause(*refused));
        if let Some(cause) = &dlc_missing {
            missing.push(cause.as_str());
        }
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
        let lifecycle =
            lifecycle.map_err(|_| "XMAP12 exact input admission lost the session DLC set")?;
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
            lifecycle,
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
/// from the lifecycle row's own refusal.
///
/// The three states are different facts and say so: the key was never written,
/// it was written as an explicit non-reading, or it was measured with a blank
/// name the trigger cannot be matched against. None is an empty DLC set, which
/// is a measurement.
fn unmeasured_session_dlc_cause(refused: AmbientLifecycleRefusal) -> String {
    let state = match refused {
        AmbientLifecycleRefusal::Absent => "and this frame carries no such key".to_string(),
        AmbientLifecycleRefusal::Unobtained => {
            "and this frame carries it as an explicit unobtained".to_string()
        }
        AmbientLifecycleRefusal::BlankName { index } => format!(
            "and this frame carries it with a blank name at index {index}, which no dlc_allowed \
             trigger can be matched against"
        ),
    };
    format!(
        "the measured session DLC set (WorldFrame session.active_dlcs, which the container's own \
         modset binds onto every frame, {state}), without which the ambient dlc_allowed trigger \
         cannot be answered"
    )
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

/// The map-authority rows this family owns, and may therefore consume a gap for.
///
/// An explicit list, not an `objects.` prefix. The prefix said "every input whose
/// id begins with objects." and MEANT "the three XMAP12 owns", and those stopped
/// being the same statement the moment a fourth could be added under that prefix
/// by another family. Consuming another family's gap is not a cosmetic error:
/// the foundation counts each gap as consumed exactly once, so the family that
/// really owns it would then be told its own gap was already claimed.
///
/// Two of the three are viewer derivations. `objects.city_population` can no
/// longer BE a gap - it has no recorded operand, so the authority admits it on
/// every hour - and it stays in the list because the list says what this family
/// owns, not what happens to be refused today. `objects.generic_population_lifecycle`
/// still can: its one recorded operand is the session DLC set, and an hour that
/// did not measure it is filed as this family's gap and consumed here.
pub(crate) const XMAP12_OWNED_AUTHORITY_INPUTS: [&str; 3] = [
    "objects.building_state_damage",
    "objects.city_population",
    "objects.generic_population_lifecycle",
];

fn owned_gap_ids(context: &FoundationFeatureContext<'_>) -> Vec<String> {
    context
        .map_authority_gaps
        .iter()
        .filter(|gap| XMAP12_OWNED_AUTHORITY_INPUTS.contains(&gap.input_id.as_str()))
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

    /// The family owns exactly the `objects.` rows, and the promotion means two
    /// of the three can never be gaps.
    ///
    /// kills: claiming an `objects.` gap another family owns - the foundation
    /// counts each gap consumed once, so the real owner would be told its gap
    /// was already claimed; and letting the owned list drift away from the
    /// authority's own `objects.` census in either direction.
    #[test]
    fn the_family_owns_exactly_the_object_rows_and_two_of_them_can_never_gap() {
        use crate::map_authority::{
            map_authority_owner_proven_admission, map_authority_viewer_derived_admission,
            MapAuthorityGap, MapAuthorityRequirementDisposition, MAP_AUTHORITY_REQUIRED_INPUTS,
        };

        let census: Vec<&str> = MAP_AUTHORITY_REQUIRED_INPUTS
            .iter()
            .copied()
            .filter(|id| id.starts_with("objects."))
            .collect();
        assert_eq!(
            census,
            XMAP12_OWNED_AUTHORITY_INPUTS.to_vec(),
            "the owned list and the authority's objects. census disagree"
        );

        // The two promoted rows are viewer derivations; the third is not. The
        // city row has no recorded operand, so it can never be a gap; the
        // lifecycle row's one recorded operand can be unmeasured, so it can.
        for promoted in [
            "objects.city_population",
            "objects.generic_population_lifecycle",
        ] {
            let derived = map_authority_viewer_derived_admission(promoted)
                .unwrap_or_else(|| panic!("{promoted} is no longer a viewer derivation"));
            assert_eq!(derived.owner_packet, "XMAP12", "{promoted}");
            assert!(map_authority_owner_proven_admission(promoted).is_none(), "{promoted}");
        }
        assert!(map_authority_viewer_derived_admission("objects.building_state_damage").is_none());
        assert!(!map_authority_viewer_derived_admission("objects.city_population")
            .unwrap()
            .can_report_absent);
        assert!(
            map_authority_viewer_derived_admission("objects.generic_population_lifecycle")
                .unwrap()
                .can_report_absent
        );

        // A gap belonging to another family under the same prefix is NOT claimed,
        // and this family's own - the recorded carrier AND the lifecycle whose
        // DLC operand went unmeasured - still are.
        let gap = |input_id: &str| MapAuthorityGap {
            input_id: input_id.to_string(),
            disposition: MapAuthorityRequirementDisposition::Absent,
            reason: "a stated cause".to_string(),
        };
        let gaps = [
            gap("objects.building_state_damage"),
            gap("objects.generic_population_lifecycle"),
            gap("objects.some_other_familys_row"),
        ];
        let viewer = crate::test_support::bare_viewer();
        let selection = crate::production_features::xmap15::Xmap15ViewerSelection {
            selected_plan: None,
        };
        let frame = crate::test_support::frame_with_rest(serde_json::json!({}));
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame, &viewer, &gaps, &[], &selection,
        );
        assert_eq!(
            owned_gap_ids(&context),
            vec![
                "objects.building_state_damage".to_string(),
                "objects.generic_population_lifecycle".to_string(),
            ]
        );
    }

    /// kills: collapsing the two unmeasured leaf states onto one cause, which
    /// would tell a reader an hour "has no DLC set" when the recording
    /// explicitly said it did not read one - and admitting an unmeasured leaf as
    /// an empty set, which would silently deactivate every DLC-gated ambient
    /// type instead of refusing the frame.
    #[test]
    fn an_unmeasured_session_dlc_set_names_which_unmeasured_state_it_is() {
        let absent = unmeasured_session_dlc_cause(AmbientLifecycleRefusal::Absent);
        let unobtained = unmeasured_session_dlc_cause(AmbientLifecycleRefusal::Unobtained);
        let blank = unmeasured_session_dlc_cause(AmbientLifecycleRefusal::BlankName { index: 2 });
        assert_ne!(absent, unobtained);
        assert_ne!(unobtained, blank);
        assert!(absent.contains("no such key"), "{absent}");
        assert!(unobtained.contains("explicit unobtained"), "{unobtained}");
        assert!(blank.contains("blank name at index 2"), "{blank}");
        for cause in [&absent, &unobtained, &blank] {
            assert!(cause.contains("session.active_dlcs"), "{cause}");
        }
    }

    /// The lifecycle's admission IS its XMAP00 row's status, and the token
    /// carries the set the status certified.
    ///
    /// kills: minting from a status the mint did not read - a row that says
    /// Present while the token is built off a second read of the leaf, or the
    /// reverse; refusing an EMPTY measured set as if it were unmeasured, which
    /// would refuse every vanilla-only session; admitting a measured set the
    /// derivation then refuses (a blank name), which would hard-fail the frame
    /// behind an admitted row; and letting the city row, which has no recorded
    /// operand, follow the lifecycle's refusal.
    #[test]
    fn the_lifecycle_admission_is_the_rows_own_status_and_carries_its_measured_set() {
        use crate::map_authority::map_authority_viewer_derived_input_status;
        use crate::test_support::{bare_viewer, frame_with_rest};
        use crate::xmap12_population::xmap12_canonical_dlc_names;
        const LIFECYCLE: &str = "objects.generic_population_lifecycle";
        const CITY: &str = "objects.city_population";

        let cases: [(&str, serde_json::Value, ViewStatus, Option<Vec<&str>>); 7] = [
            ("no session key at all", serde_json::json!({}), ViewStatus::DomainAbsent, None),
            (
                "an explicit non-reading",
                serde_json::json!({"session": {"active_dlcs": null}}),
                ViewStatus::DomainUnavailable,
                None,
            ),
            (
                "a measured set with a blank name",
                serde_json::json!({"session": {"active_dlcs": ["Together for Victory", ""]}}),
                ViewStatus::DomainUnavailable,
                None,
            ),
            (
                "a measured set with a whitespace name",
                serde_json::json!({"session": {"active_dlcs": [" "]}}),
                ViewStatus::DomainUnavailable,
                None,
            ),
            (
                "a measured empty set",
                serde_json::json!({"session": {"active_dlcs": []}}),
                ViewStatus::Present,
                Some(Vec::new()),
            ),
            (
                "a measured set",
                serde_json::json!({"session": {"active_dlcs": ["Together for Victory", "Waking the Tiger"]}}),
                ViewStatus::Present,
                Some(vec!["Together for Victory", "Waking the Tiger"]),
            ),
            (
                "an unsorted, duplicated set, carried canonical",
                serde_json::json!({"session": {"active_dlcs": ["Waking the Tiger", "Together for Victory", "Waking the Tiger"]}}),
                ViewStatus::Present,
                Some(vec!["Together for Victory", "Waking the Tiger"]),
            ),
        ];
        for (label, rest, expected_status, expected_names) in cases {
            let frame = frame_with_rest(rest);
            let viewer = bare_viewer();
            let selection = crate::production_features::xmap15::Xmap15ViewerSelection {
                selected_plan: None,
            };
            let context = FoundationFeatureContext::new_for_owned_accounting_tests(
                &frame, &viewer, &[], &[], &selection,
            );
            let row_status = map_authority_viewer_derived_input_status(&frame, LIFECYCLE)
                .expect("the lifecycle is a viewer derivation");
            assert_eq!(row_status, expected_status, "{label}");
            // What the admission certifies is exactly what the derivation
            // canonicalizes: a measured set is admitted if and only if the
            // derivation's own canonical form exists for it.
            let measured = frame.session_metadata().active_dlcs.measured();
            match admit_xmap12_ambient_lifecycle(&context) {
                Ok(admitted) => {
                    assert_eq!(row_status, ViewStatus::Present, "{label}");
                    let names: Vec<&str> = admitted
                        .canonical_dlc_names()
                        .iter()
                        .map(String::as_str)
                        .collect();
                    assert_eq!(Some(names), expected_names, "{label}");
                    // The token carries the canonicalizer's own answer for the
                    // recorded list, already canonical.
                    assert_eq!(
                        xmap12_canonical_dlc_names(measured.expect("an admitted set is measured")),
                        Ok(admitted.canonical_dlc_names().to_vec()),
                        "{label}: the token does not carry the canonicalizer's answer"
                    );
                }
                Err(refused) => {
                    assert_eq!(refused.status(), row_status, "{label}");
                    assert_eq!(expected_names, None, "{label}");
                    let cause = unmeasured_session_dlc_cause(refused);
                    assert!(cause.contains("session.active_dlcs"), "{label}: {cause}");
                    if let Some(names) = measured {
                        let AmbientLifecycleRefusal::BlankName { index } = refused else {
                            panic!("{label}: a measured set was refused as {refused:?}");
                        };
                        assert_eq!(
                            xmap12_canonical_dlc_names(names),
                            Err(index),
                            "{label}: the admission refused a set the canonicalizer accepts"
                        );
                        assert!(cause.contains("blank name"), "{label}: {cause}");
                    }
                }
            }
            assert_eq!(
                map_authority_viewer_derived_input_status(&frame, CITY),
                Some(ViewStatus::Present),
                "{label}: the city row has no recorded operand to refuse on"
            );
        }
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
