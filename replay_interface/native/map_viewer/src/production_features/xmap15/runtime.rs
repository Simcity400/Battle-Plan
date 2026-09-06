use super::*;
use portable_render_stream::{Xmap15PlanSemanticRow, Xmap15PreparedRoute,
    Xmap15ViewerRuntimeFrame, Xmap15ViewerPlanDrawInputs, Xmap15ViewerPlanRefusal,
    Xmap15ViewerMissingInput as Missing};

#[derive(Clone)]
struct RetainedPlan {
    route: Xmap15PreparedRoute,
    base_colour: [u32; 4],
    outer_colour: [u32; 4],
    selection_unproven: bool,
}

#[derive(Clone, Default)]
pub(super) struct RetainedPlans {
    objects: BTreeMap<Xmap15PlanKey, RetainedPlan>,
    sequence: u64,
}

fn key(row: &crate::Xmap15DerivedOrder) -> Xmap15PlanKey {
    Xmap15PlanKey {
        owner_kind: match row.key.owner.kind {
            ArmyBattleplanOwnerKind::Army => portable_render_stream::Xmap15OwnerKind::Army,
            ArmyBattleplanOwnerKind::ArmyGroup => portable_render_stream::Xmap15OwnerKind::ArmyGroup,
        }, owner_id: row.key.owner.id, engine_order_instance_id: row.key.engine_order_instance_id,
        generation: row.key.generation, slot_index: row.slot_index,
    }
}

impl RetainedPlans {
    pub(super) fn observe_selection(&mut self, context: &FoundationFeatureContext<'_>) {
        if context.xmap15_selection.selected_owner.is_some() || context.xmap15_selection.selected_plan.is_some()
            || !context.viewer.selected_object_ids.is_empty() {
            for object in self.objects.values_mut() { object.selection_unproven = true; }
        }
    }

    pub(super) fn prepare_inputs(&mut self, context: &FoundationFeatureContext<'_>,
        provider: &Xmap15PlanProvider, resolver: &GameFileResolver, admission: &ValidatedContentAdmission,
    ) -> Result<Option<Xmap15FrameInputs>> {
        let Xmap15PlanProvider::Present(census) = provider else { return Ok(None) };
        let units = context.xmap15_units.ok_or("XMAP15 current identity provider absent")?;
        if units.hour_index() != census.hour_index { return Err("XMAP15 unit census hour mismatch".into()); }
        let dimensions = admitted(resolver, admission, XMAP15_MAP_DIMENSIONS_PATH)?;
        let bytes = resolver.read_admitted_used_content(&dimensions)?;
        let (width, _) = crate::production_foundation::bmp_dimensions(&bytes)?;
        let mut names = census.orders.iter().filter(|row| row.order_type == 4)
            .filter_map(|row| row.definition_name.ok().map(str::to_owned)).collect();
        let (definitions, _) = prepare_plan_definitions(resolver, admission, &mut names)?;
        self.advance(context, census, units, &definitions, width).map(Some)
    }

    fn advance(&mut self, context: &FoundationFeatureContext<'_>, census: &crate::Xmap15OrderDerivation,
        units: &dyn Xmap15UnitIdentityProvider, definitions: &[PreparedXmap15PlanDefinition], width: u32,
    ) -> Result<Xmap15FrameInputs> {
        let actual = context.world_frame.army_battleplans();
        let source = actual.orders.iter().map(|row| (&row.key, row)).collect::<BTreeMap<_, _>>();
        let owners = census.orders.iter().map(|row| row.key.owner).collect::<BTreeSet<_>>();
        let mut runtime = Xmap15ViewerRuntimeFrame { retained_plans: Vec::new(),
            draw_inputs: Vec::new(), refusals: vec![Xmap15ViewerPlanRefusal {
                key: None, input: Missing::MovementRuntime }] };
        let mut plans = BTreeMap::new();
        let mut current = BTreeSet::new();
        for owner in owners {
            let owner_rows = census.orders.iter().filter(|row| row.key.owner == owner).collect::<Vec<_>>();
            let traversal = census.owner_traversal(&owner, false, false);
            let matches = context.world_frame.countries().keys().map(|country| (country,
                units.formation_identity_match_count(owner.kind == ArmyBattleplanOwnerKind::ArmyGroup,
                    owner.id, country))).filter(|(_, count)| *count != 0).collect::<Vec<_>>();
            let country = match matches.as_slice() { [(country, 1)] => Some(*country), _ => None };
            let owner_missing = if country.is_none() || context.viewer.observer_country_id.is_none() { Some(Missing::OwnerIdentity) }
                else if context.viewer.map_mode_id != "political" { Some(Missing::VirtualPresentation) }
                else { None };
            if let Some(input) = owner_missing {
                for row in owner_rows {
                    let key = key(row); current.insert(key.clone());
                    runtime.refusals.push(Xmap15ViewerPlanRefusal { key: Some(key), input });
                }
                continue;
            }
            let country = country.unwrap();
            if context.viewer.observer_country_id.as_ref() != Some(country) {
                // Ordinary owner admission does not visit another country's pool.
                continue;
            }
            let traversal = match traversal {
                Ok(rows) => rows,
                Err(_) => {
                    for row in owner_rows {
                        let key = key(row); current.insert(key.clone());
                        runtime.refusals.push(Xmap15ViewerPlanRefusal { key: Some(key), input: Missing::RootTraversal });
                    }
                    continue;
                }
            };
            for row in traversal {
                let identity = key(row); current.insert(identity.clone());
                let result = (|| -> std::result::Result<_, Missing> {
                    match row.front_binding.status {
                        1 => {}, 2 => return Err(Missing::CurrentFrontResolver),
                        _ => return Err(Missing::FrontBinding),
                    }
                    if row.order_type != 4 { return Err(Missing::RouteGeometry); }
                    let style = source.get(&row.key).and_then(|row| row.style).ok_or(Missing::OwnerOverride)?;
                    if style.owner_override { return Err(Missing::OwnerOverride); }
                    if style.virtual_line { return Err(Missing::VirtualPresentation); }
                    let colour = row.owner_colour.as_ref().map_err(|_| Missing::OwnerColour)?;
                    if colour.status != 1 { return Err(Missing::OwnerColour); }
                    let anchors = row.anchors.as_ref().map_err(|_| Missing::RouteGeometry)?;
                    let anchors: &[_; 2] = anchors.as_slice().try_into().map_err(|_| Missing::RouteGeometry)?;
                    let endpoint_ids = anchors.map(|anchor| anchor.province_id);
                    let old = self.objects.get(&identity);
                    let route = if let Some(old) = old.filter(|old| old.route.ordered_province_ids == endpoint_ids) {
                        old.route.clone()
                    } else {
                        Xmap15PreparedRoute::from_paradrop_province_anchors(endpoint_ids,
                            anchors.map(|anchor| [anchor.engine_x, anchor.engine_y]), width,
                            old.map_or(0, |old| old.route.wrap_offset_world)).map_err(|_| Missing::RouteGeometry)?
                    };
                    let object = self.objects.entry(identity.clone()).or_insert_with(|| RetainedPlan {
                        route: route.clone(), base_colour: colour.rgba_f32_bits,
                        outer_colour: [1.0f32.to_bits(), 1.0f32.to_bits(), 1.0f32.to_bits(), colour.rgba_f32_bits[3]],
                        selection_unproven: false,
                    });
                    object.route = route;
                    object.selection_unproven |= context.xmap15_selection.selected_owner.is_some()
                        || context.xmap15_selection.selected_plan.is_some() || !context.viewer.selected_object_ids.is_empty();
                    if object.selection_unproven { return Err(Missing::SelectedPresentation); }
                    if row.active { return Err(Missing::ActiveReadiness); }
                    let assigned = !row.member_division_ids.is_empty();
                    let texture_variant = if assigned {
                        // With no owner override, native count excludes at most its single deployed leader unit.
                        let distinct = row.member_division_ids.iter().copied().collect::<BTreeSet<_>>();
                        if distinct.len() < 2 || distinct.len() != row.member_division_ids.len()
                            || distinct.iter().any(|id| units.division_identity_match_count(*id, country) != 1) {
                            return Err(Missing::AssignedUnitFilter);
                        }
                        0
                    } else { 1 };
                    let definition = definitions.iter().find(|definition| definition.name == "paradrop")
                        .ok_or(Missing::Definition)?;
                    if definition.effective_texture_variant_count().map_err(|_| Missing::Definition)? <= u32::from(texture_variant) {
                        return Err(Missing::Definition);
                    }
                    if definition.culling_direction.is_some_and(|direction| direction != 0) {
                        return Err(Missing::CullingDirection);
                    }
                    let semantic = Xmap15PlanSemanticRow {
                        key: identity.clone(), owner_country: country.clone(),
                        selector: portable_render_stream::Xmap15PlanDefinitionSelector {
                            order_type: 4, move_special_118: false, withdraw_119: false,
                            fallback_298: false, virtual_299: false, blitz_118: false,
                            army_group_style_39_and_378: false,
                        }, resolved_definition_name: "paradrop".into(),
                        ordered_assigned_division_ids: row.member_division_ids.clone(), assigned, active: false,
                        route: object.route.clone(), manager_insertion_ordinal: None,
                        definition_group_ordinal: None, final_engine_draw_ordinal: None,
                        object_width_multiplier_f32_bits: 0, comparator_tie_17c_f32_bits: 0,
                        fade_in_f32_bits: definition.fade_in_f32_bits, fade_out_f32_bits: definition.fade_out_f32_bits,
                        culling_offset_f32_bits: 0, inner_rgba_f32_bits: colour.rgba_f32_bits,
                        outer_rgba_f32_bits: object.outer_colour, texture_variant,
                        see_through: definition.technique.ends_with("Seethrough"), visible: true,
                        exact_head_vertices_f32_bits: None,
                        lifecycle_tombstone_sha256: None,
                    };
                    let speed = definition.texture_anim_speed_f32_bits.map(f32::from_bits).unwrap_or(0.0);
                    let draw = Xmap15ViewerPlanDrawInputs { key: identity.clone(), base_rgba_f32_bits: object.base_colour,
                        inner_highlight: false, animation_speed_f32_bits:
                            if speed.abs() > f32::from_bits(0x38d1b717) { speed.to_bits() } else { 0 } };
                    Ok((semantic, draw))
                })();
                match result {
                    Ok(value) => { plans.insert(identity, value); },
                    Err(input) => runtime.refusals.push(Xmap15ViewerPlanRefusal { key: Some(identity), input }),
                }
            }
        }
        self.objects.retain(|key, _| current.contains(key));
        runtime.retained_plans = self.objects.keys().cloned().collect();
        let (plans, draw_inputs) = plans.into_values().unzip();
        runtime.draw_inputs = draw_inputs;
        // Repeated roots visit the same object; diagnostics retain identity once.
        runtime.refusals.sort_by_key(|row| (row.key.clone(), format!("{:?}", row.input)));
        runtime.refusals.dedup();
        self.sequence = self.sequence.checked_add(1).ok_or("XMAP15 viewer sequence exhausted")?;
        let semantic_rows = census.orders.iter().map(|row| {
            let original = source[&row.key];
            serde_json::json!({
                "key": key(row), "order_type": row.order_type, "active": row.active,
                "assigned_division_ids": row.member_division_ids,
                "style": original.style.map(|style| [style.move_special, style.withdraw,
                    style.fallback, style.virtual_line, style.owner_override, style.owner_grouped]),
                "links": original.linked_orders,
                "definition": row.definition_name.map_err(|reason| reason.reason()),
                "anchors": row.anchors.as_ref().map(|anchors| anchors.iter().map(|anchor|
                    (anchor.province_id, anchor.engine_x, anchor.engine_y)).collect::<Vec<_>>())
                    .map_err(|reason| reason.reason()),
                "owner_colour": row.owner_colour.as_ref().map_err(|reason| format!("{reason:?}")),
            })
        }).collect::<Vec<_>>();
        let semantic_input_sha256 = Sha256::hex(&serde_json::to_vec(&(
            "XMAP15.viewer-plan-inputs.v1", census.hour_index, semantic_rows,
            units.census_sha256(), units.admission_sha256()))?);
        let enumerator_witness_sha256 = Sha256::hex(&serde_json::to_vec(&(
            &semantic_input_sha256, &runtime, self.sequence))?);
        Ok(Xmap15FrameInputs { plans, movements: Vec::new(), observation_sequence: self.sequence,
            semantic_input_sha256, enumerator_witness_sha256, viewer_runtime: Some(runtime) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrow_submission_order::{ArrowFrameInput, ArrowSubmissionOrder, ArrowViewerSortState};

    struct Units;
    impl Xmap15UnitIdentityProvider for Units {
        fn hour_index(&self) -> i64 { 0 }
        fn census_sha256(&self) -> &str { "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
        fn admission_sha256(&self) -> &str { self.census_sha256() }
        fn formation_identity_match_count(&self, group: bool, id: u32, country: &str) -> usize {
            usize::from(!group && id == 17 && country == "GER")
        }
        fn division_identity_match_count(&self, id: u32, country: &str) -> usize {
            usize::from(country == "GER" && matches!(id, 301 | 205))
        }
    }

    fn raw(colour: [u32; 4]) -> serde_json::Value {
        let mut raw = crate::xmap15_orders::tests::recorded_raw_with_colour(serde_json::json!(0), "exact",
            Some(serde_json::json!({"version": 3, "rows": [{"owner_kind": 0, "owner_id": 17,
                "status": 1, "rgba_f32_bits": colour}]})));
        raw["countries"] = serde_json::json!({"GER": {}});
        let row = &mut raw["order_of_battle"]["extra"]["army_battleplans_depth_live"]["orders"][0];
        row["order_type"] = serde_json::json!(4);
        row["assigned_division_ids"] = serde_json::json!([]);
        row["linked_orders"] = serde_json::json!({"version": 5, "status": 1,
            "root_status": 1, "root_positions": [0, null, null],
            "front_binding": {"status": 1, "words": [2147483647, 2147483647, 0]},
            "predecessors": [], "successors": [], "virtual_predecessors": [], "virtual_successors": []});
        raw
    }

    fn frame(input: Xmap15FrameInputs, mut base: PreparedXmap15Frame) -> PreparedXmap15Frame {
        base.portable.hour_index = 0;
        base.portable.observation_sequence = input.observation_sequence;
        base.portable.semantic_input_sha256 = input.semantic_input_sha256;
        base.portable.enumerator_witness_sha256 = input.enumerator_witness_sha256;
        base.portable.viewer_runtime = input.viewer_runtime;
        base.portable.ordered_rows = input.plans.into_iter().map(|semantic| Xmap15PreparedRow::Plan {
            semantic, definition_sha256: base.plan_definitions[0].deterministic_sha256.clone(),
        }).collect();
        if base.portable.ordered_rows.is_empty() {
            base.plan_definitions.clear(); base.paradrop_profile = None; base.heightmap = None;
            base.map_dimensions_content = None; base.map_width = 0; base.map_height = 0;
            base.portable.ordered_used_content.clear();
        }
        base.portable = base.portable.seal().unwrap(); base.validate().unwrap(); base
    }

    #[test]
    fn authenticated_adapter_prepares_nonempty_d54_and_preserves_runtime_transitions() {
        use crate::test_support::{write_modvfs_fixture as write, write_modvfs_provinces};
        let root = crate::test_support::temp_test_dir("arrow-authenticated-runtime");
        write_modvfs_provinces(&root, 16, 8);
        let mut bmp = std::fs::read(root.join("map/provinces.bmp")).unwrap();
        for y in 0..8 { for x in 8..16 {
            bmp[54 + y * 48 + x * 3..57 + y * 48 + x * 3].copy_from_slice(&[6, 5, 4]);
        } }
        write(&root, "map/provinces.bmp", bmp);
        write(&root, "map/definition.csv", "0;0;0;0;land;false;plains;0\n1;1;2;3;land;false;plains;0\n2;4;5;6;land;false;plains;0\n");
        write(&root, "common/defines/fixture.lua", "NDefines = { NGraphics = { ARROW_PARADROP_HEIGHT_TO_LEN = 0.1, ARROW_PARADROP_HEIGHT_MAX = 10 } }");
        write(&root, XMAP15_MAPARROWS_PATH, r#"arrowType = {
            name = "paradrop" technique = "MapArrowNoHead"
            texture_masks = { "gfx/maparrows/fixture.dds" "gfx/maparrows/fixture.dds" }
            texture_patterns = { "gfx/maparrows/fixture.dds" "gfx/maparrows/fixture.dds" }
            body_width = { 1 1 } body_uv_scale = 1 mask_inner = { 1 1 1 } mask_outer = { 1 1 1 }
            global_scale_min_size = 1 global_scale_max_points = 1 texture_variant_count = 2
        }"#);
        let mut dds = vec![0u8; 132];
        dds[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [(4,124u32),(8,0x100f),(12,1),(16,1),(20,4),(76,32),(80,0x41),
            (88,32),(92,0xff),(96,0xff00),(100,0xff0000),(104,0xff000000),(108,0x1000),(128,u32::MAX)] {
            dds[offset..offset+4].copy_from_slice(&value.to_le_bytes());
        }
        write(&root, "gfx/maparrows/fixture.dds", dds);
        let mut shader = String::from("VertexShader = { MainCode ArrowVertexShader [[ float4 main(float4 p : POSITION) : SV_Position { return p; } ]] }\nPixelShader = { Samplers = {\n");
        for sampler in XMAP15_PLAN_SAMPLERS {
            shader.push_str(&format!("{} = {{ Index = {} MagFilter = {} MinFilter = {} MipFilter = {} AddressU = {} AddressV = {}\n",
                sampler.name, sampler.index, sampler.mag, sampler.min, sampler.mip, sampler.address, sampler.address));
            if let Some(value) = sampler.sampler_type { shader.push_str(&format!("Type = {value}\n")); }
            for (field, value) in [("MipMapLodBias", sampler.lod_bias), ("MipMapMinLod", sampler.min_lod), ("MipMapMaxLod", sampler.max_lod)] {
                if let Some(value) = value { shader.push_str(&format!("{field} = {value}\n")); }
            }
            shader.push_str("}\n");
        }
        shader.push_str("} MainCode ArrowPixelShaderNoHead [[ float4 main() : SV_Target { return 1; } ]] }\nDepthStencilState DepthStencilState {\n");
        for (field, value) in xmap15_depth_contract("DepthStencilState").unwrap() {
            shader.push_str(&format!("{field} = {value}\n"));
        }
        shader.push_str("}\nEffect MapArrowNoHead { VertexShader = \"ArrowVertexShader\" PixelShader = \"ArrowPixelShaderNoHead\" DepthStencilState = \"DepthStencilState\" Defines = { \"ANIM_TEXTURE\" } }\n");
        write(&root, XMAP15_MAPARROW_SHADER_PATH, shader);
        write(&root, crate::shader_compile::DX11_PLATFORM_HEADER, "// authenticated fixture platform header\n");
        let raw_resolver = crate::test_support::fixture_game_resolver(&root);
        let paths = raw_resolver.audit_effective_dir_files_recursive("", None).unwrap().into_iter().collect();
        let (resolver, admission) = crate::test_support::admitted_vanilla_resolver(&raw_resolver, &paths, "arrow-authenticated-runtime");
        let adapter = Xmap15OrdersAdapter::new(&resolver, &admission);
        let topology = portable_render_stream::AssignedStageTopology::seal(vec![
            portable_render_stream::AssignedStageIdentity::new(XMAP15_ASSIGNED_STAGE_NAME, XMAP15_ASSIGNED_OPERATION_NAME)], vec![]).unwrap();
        let schedule = portable_render_stream::AssignedStageSchedule::from_topology(&topology).unwrap();
        let gaps = [crate::MapAuthorityGap { input_id: XMAP15_AUTHORITY_GAP_ID.into(),
            disposition: crate::MapAuthorityRequirementDisposition::Unavailable, reason: "native sidecar not recorded".into() }];
        let mut viewer = crate::test_support::bare_viewer();
        viewer.map_mode_id = "political".into(); viewer.observer_country_id = Some("GER".into());
        let unselected = Xmap15ViewerSelection::default();
        let make_world = |colour, generation| {
            let mut value = raw(colour);
            let row = &mut value["order_of_battle"]["extra"]["army_battleplans_depth_live"]["orders"][0];
            row["path_provinces"] = serde_json::json!([1, 2]);
            row["generation"] = serde_json::json!(generation);
            crate::test_support::frame_with_rest(value)
        };
        let prepare = |world: &WorldFrame, selection: &Xmap15ViewerSelection| {
            assert!(world.xmap15_orders_fronts_movement_semantic().is_none());
            let mut context = FoundationFeatureContext::new_for_owned_accounting_tests(world, &viewer, &gaps, &[], selection)
                .with_test_schedule(&schedule);
            context.xmap15_units = Some(&Units);
            adapter.prepare(&context).unwrap()
        };
        let original = [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits);
        let world = make_world(original, 4);
        let output = prepare(&world, &unselected);
        assert!(matches!(output.disposition, FamilyDisposition::Present { .. }), "{:?}", output.disposition);
        let prepared = output.prepared_orders.unwrap().native.unwrap();
        prepared.validate().unwrap();
        assert_eq!(prepared.portable.ordered_rows.len(), 1);
        assert_eq!((prepared.map_width, prepared.map_height), (16, 8));
        for path in [XMAP15_MAPARROWS_PATH, XMAP15_MAPARROW_SHADER_PATH, XMAP15_HEIGHTMAP_PATH,
            XMAP15_MAP_DIMENSIONS_PATH, "common/defines/fixture.lua", "gfx/maparrows/fixture.dds"] {
            let key = prepared.portable.ordered_used_content.iter().find(|key| key.normalized_game_relative_path == path.to_ascii_lowercase()).unwrap_or_else(|| panic!("prepared content omitted {path}"));
            assert!(!resolver.read_admitted_used_content(key).unwrap().is_empty());
        }
        let Xmap15PreparedRow::Plan { semantic, .. } = &prepared.portable.ordered_rows[0] else { panic!() };
        assert_eq!(semantic.route.ordered_province_ids, [2, 1]);
        assert_eq!(semantic.lifecycle_tombstone_sha256, None);
        assert_eq!(semantic.manager_insertion_ordinal, None);
        let height = crate::image_load::decode_bmp_indexed(&resolver.read_admitted_used_content(prepared.heightmap.as_ref().unwrap()).unwrap(),
            std::path::Path::new(XMAP15_HEIGHTMAP_PATH)).unwrap();
        let mesh = semantic.build_paradrop_map_arrow_mesh(prepared.plan_definition("paradrop").unwrap().geometry_profile().unwrap(),
            prepared.map_width, prepared.paradrop_profile.as_ref().unwrap().parameters,
            |x, z| Ok(crate::rail_files::sample_exact_supply_arrow_height(&height, x, z))).unwrap();
        assert!(mesh.body_vertex_count > 0);
        let mut lifecycle = Xmap15LifecycleState::default();
        let witness = lifecycle.apply(&prepared).unwrap();
        assert_eq!(witness.created.len(), 1);
        let mut sort = ArrowViewerSortState::default();
        let submitted = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&prepared), ArrowFrameInput::ViewInactive,
            &mut sort, Some(&witness), &"a".repeat(64));
        assert_eq!(submitted.submissions.len(), 1, "{:?}", submitted.refusals);
        assert_eq!(submitted.submissions[0].registration_ordinal, None);
        let changed_colour = [0.75f32, 0.25, 0.5, 0.5].map(f32::to_bits);
        let changed_world = make_world(changed_colour, 4);
        let changed = prepare(&changed_world, &unselected).prepared_orders.unwrap().native.unwrap();
        let Xmap15PreparedRow::Plan { semantic: changed_row, .. } = &changed.portable.ordered_rows[0] else { panic!() };
        assert_eq!(changed_row.inner_rgba_f32_bits, changed_colour);
        assert_eq!(changed_row.outer_rgba_f32_bits, semantic.outer_rgba_f32_bits);
        assert_eq!(changed.plan_draw_inputs(&changed_row.key).unwrap().base_rgba_f32_bits, original);
        let edited = lifecycle.apply(&changed).unwrap();
        assert_eq!(edited.edited.len(), 1); assert!(edited.created.is_empty() && edited.deleted.is_empty());
        #[cfg(windows)] {
            let inner = crate::xmap15_d3d_handoff::plan_cb3_for_test(&changed, changed_row, mesh.total_length_f32_bits, false).unwrap();
            let outer = crate::xmap15_d3d_handoff::plan_cb3_for_test(&changed, changed_row, mesh.total_length_f32_bits, true).unwrap();
            let rgba = |bytes: &[u8], offset: usize| std::array::from_fn::<_, 4, _>(|i|
                u32::from_le_bytes(bytes[offset+i*4..offset+i*4+4].try_into().unwrap()));
            assert_eq!(rgba(&inner, 16), changed_colour); assert_eq!(rgba(&inner, 32), original);
            assert_eq!(rgba(&outer, 16), semantic.outer_rgba_f32_bits);
            assert_eq!(rgba(&outer, 32), semantic.outer_rgba_f32_bits);
        }
        let gap_world = crate::test_support::frame_with_rest(serde_json::json!({}));
        let selected = Xmap15ViewerSelection { selected_owner: Some((portable_render_stream::Xmap15OwnerKind::Army, 17)), selected_plan: None };
        let gap = prepare(&gap_world, &selected);
        assert!(matches!(gap.disposition, FamilyDisposition::Unavailable { .. }));
        let recovered = prepare(&world, &unselected);
        assert!(matches!(recovered.disposition, FamilyDisposition::Unavailable { .. }));
        assert!(format!("{:?}", recovered.disposition).contains("SelectedPresentation"));
        assert_eq!(adapter.runtime.lock().unwrap().objects.len(), 1);
        let fresh_world = make_world(changed_colour, 5);
        let fresh_output = prepare(&fresh_world, &unselected);
        assert!(matches!(fresh_output.disposition, FamilyDisposition::Present { .. }));
        let fresh = fresh_output.prepared_orders.unwrap().native.unwrap();
        let replaced = lifecycle.apply(&fresh).unwrap();
        assert_eq!(replaced.created.len(), 1); assert_eq!(replaced.deleted.len(), 1);
        assert_eq!(adapter.runtime.lock().unwrap().objects.len(), 1);
        let fresh_submission = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&fresh),
            ArrowFrameInput::ViewInactive, &mut sort, Some(&replaced), &"a".repeat(64));
        assert_eq!(fresh_submission.submissions.len(), 1, "{:?}", fresh_submission.refusals);
        for (members, expected_variant) in [
            (vec![301, 205], Some(0)), (vec![205, 301], Some(0)),
            (vec![301], None), (vec![301, 999], None), (vec![301, 301], None),
        ] {
            let mut assigned_raw = raw(changed_colour);
            let row = &mut assigned_raw["order_of_battle"]["extra"]["army_battleplans_depth_live"]["orders"][0];
            row["path_provinces"] = serde_json::json!([1, 2]);
            row["generation"] = serde_json::json!(6);
            row["assigned_division_ids"] = serde_json::json!(members);
            let assigned_world = crate::test_support::frame_with_rest(assigned_raw);
            let assigned_output = prepare(&assigned_world, &unselected);
            if let Some(variant) = expected_variant {
                assert!(matches!(assigned_output.disposition, FamilyDisposition::Present { .. }));
                let prepared = assigned_output.prepared_orders.unwrap().native.unwrap();
                let Xmap15PreparedRow::Plan { semantic, .. } = &prepared.portable.ordered_rows[0] else { panic!() };
                assert_eq!(semantic.texture_variant, variant);
                assert!(semantic.assigned && !semantic.active);
                assert_eq!(semantic.ordered_assigned_division_ids, members);
                let witness = lifecycle.apply(&prepared).unwrap();
                let submitted = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&prepared),
                    ArrowFrameInput::ViewInactive, &mut sort, Some(&witness), &"a".repeat(64));
                assert_eq!(submitted.submissions.len(), 1, "{:?}", submitted.refusals);
            } else {
                assert!(matches!(assigned_output.disposition, FamilyDisposition::Unavailable { .. }));
                assert!(assigned_output.prepared_orders.unwrap().native.is_none());
            }
        }
        drop(adapter); drop(resolver); drop(raw_resolver);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recorded_roots_create_retained_paradrop_mesh_and_real_ordered_submission() {
        let original_colour = [0.25f32.to_bits(), 0.5f32.to_bits(), 0.75f32.to_bits(), 1.0f32.to_bits()];
        let world = crate::test_support::frame_with_rest(raw(original_colour));
        let anchors = crate::xmap15_orders::tests::anchors();
        let provider = derive_xmap15_orders(&world, Some(&anchors));
        let Xmap15PlanProvider::Present(census) = &provider else { panic!("{provider:?}") };
        assert_eq!(census.orders.len(), 1);
        assert!(world.xmap15_orders_fronts_movement_semantic().is_none());
        let mut viewer = crate::test_support::bare_viewer();
        viewer.map_mode_id = "political".into(); viewer.observer_country_id = Some("GER".into());
        let selection = Xmap15ViewerSelection::default();
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(&world, &viewer, &[], &[], &selection);
        let mut base = crate::arrow_submission_order::tests::prepared_orders(1, true, None);
        let definition = &mut base.plan_definitions[0];
        definition.name = "paradrop".into(); definition.texture_variant_count = Some(2);
        definition.ordered_texture_masks.push(definition.ordered_texture_masks[0].clone());
        definition.ordered_texture_patterns.push(definition.ordered_texture_patterns[0].clone());
        definition.deterministic_sha256.clear();
        definition.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&serde_json::to_value(&definition).unwrap()).unwrap());
        let mut defines = base.heightmap.clone().unwrap();
        defines.normalized_game_relative_path = "common/defines/fixture.lua".into();
        defines.stored_game_relative_path = defines.normalized_game_relative_path.clone();
        let profile = portable_render_stream::Xmap15ParadropProfile {
            height_to_length_f32_bits: 0.1f32.to_bits(), max_height_f32_bits: 10.0f32.to_bits() };
        base.paradrop_profile = Some(PreparedXmap15ParadropProfile { parameters: profile, ordered_used_content: vec![defines.clone()] });
        base.portable.ordered_used_content.push(defines); base.portable.ordered_used_content.sort();
        base.map_width = 16; base.map_height = 8;
        let mut runtime = RetainedPlans::default();
        let input = runtime.advance(&context, census, &Units, &base.plan_definitions, 16).unwrap();
        assert_eq!(input.plans.len(), 1, "{:?}", input.viewer_runtime);
        let prepared = frame(input, base.clone());
        let Xmap15PreparedRow::Plan { semantic, .. } = &prepared.portable.ordered_rows[0] else { panic!() };
        assert_eq!(semantic.route.ordered_province_ids, [102, 101]);
        assert_eq!(semantic.manager_insertion_ordinal, None);
        let one = 1.0f32.to_bits();
        let mesh = semantic.build_paradrop_map_arrow_mesh(portable_render_stream::Xmap15PlanGeometryProfile {
            body_start_width_f32_bits: one, body_end_width_f32_bits: one,
            runtime_width_scale_f32_bits: 0, body_uv_scale_f32_bits: one,
            width_class_zero_f32_bits: one, width_class_one_f32_bits: one,
            global_scale_min_size_f32_bits: one, global_scale_max_points: 1, head_length_f32_bits: 0,
        }, 16, profile, |_, _| Ok(0.0)).unwrap();
        assert!(mesh.body_vertex_count > 0);
        assert!(mesh.vertices.iter().all(|vertex| vertex.position.iter().all(|value| value.is_finite())));
        let mut lifecycle = Xmap15LifecycleState::default();
        let witness = lifecycle.apply(&prepared).unwrap();
        assert_eq!(witness.created.len(), 1);
        let mut sort = ArrowViewerSortState::default();
        let submitted = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&prepared),
            ArrowFrameInput::ViewInactive, &mut sort, Some(&witness), &"a".repeat(64));
        assert_eq!(submitted.submissions.len(), 1, "{:?}", submitted.refusals);
        assert_eq!(submitted.submissions[0].registration_ordinal, None);
        assert_eq!(prepared.plan_draw_inputs(&semantic.key).unwrap().base_rgba_f32_bits, original_colour);

        #[cfg(windows)]
        {
            let inner = crate::xmap15_d3d_handoff::plan_cb3_for_test(&prepared, semantic, mesh.total_length_f32_bits, false).unwrap();
            let outer = crate::xmap15_d3d_handoff::plan_cb3_for_test(&prepared, semantic, mesh.total_length_f32_bits, true).unwrap();
            let rgba = |bytes: &[u8], offset: usize| std::array::from_fn::<_, 4, _>(|i|
                u32::from_le_bytes(bytes[offset+i*4..offset+i*4+4].try_into().unwrap()));
            assert_eq!(rgba(&inner, 16), original_colour); assert_eq!(rgba(&inner, 32), original_colour);
            assert_eq!(rgba(&outer, 16), semantic.outer_rgba_f32_bits);
            assert_eq!(rgba(&outer, 32), semantic.outer_rgba_f32_bits);
            assert_eq!(u32::from_le_bytes(inner[52..56].try_into().unwrap()), 0);
            assert_eq!(u32::from_le_bytes(inner[72..76].try_into().unwrap()), 0);
            assert_eq!(u32::from_le_bytes(inner[48..52].try_into().unwrap()), 42.0f32.to_bits());
        }
        let changed_colour = [0.75f32.to_bits(), 0.25f32.to_bits(), 0.5f32.to_bits(), 0.5f32.to_bits()];
        let changed_world = crate::test_support::frame_with_rest(raw(changed_colour));
        let changed_provider = derive_xmap15_orders(&changed_world, Some(&anchors));
        let Xmap15PlanProvider::Present(changed_census) = &changed_provider else { panic!() };
        let changed_context = FoundationFeatureContext::new_for_owned_accounting_tests(&changed_world, &viewer, &[], &[], &selection);
        let changed = frame(runtime.advance(&changed_context, changed_census, &Units, &base.plan_definitions, 16).unwrap(), base.clone());
        let Xmap15PreparedRow::Plan { semantic: changed_row, .. } = &changed.portable.ordered_rows[0] else { panic!() };
        assert_eq!(changed_row.inner_rgba_f32_bits, changed_colour);
        assert_eq!(changed_row.outer_rgba_f32_bits, semantic.outer_rgba_f32_bits);
        assert_eq!(changed.plan_draw_inputs(&changed_row.key).unwrap().base_rgba_f32_bits, original_colour);
        let edited = lifecycle.apply(&changed).unwrap();
        assert_eq!(edited.edited.len(), 1); assert!(edited.created.is_empty() && edited.deleted.is_empty());
        assert_eq!(semantic.lifecycle_tombstone_sha256, None);
        let root = crate::test_support::temp_test_dir("arrow-runtime-gap");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("fixture.txt"), b"fixture").unwrap();
        let resolver = crate::test_support::fixture_game_resolver(&root);
        let (resolver, admission) = crate::test_support::admitted_vanilla_resolver(&resolver,
            &BTreeSet::from(["fixture.txt".into()]), "arrow-runtime-gap");
        let adapter = Xmap15OrdersAdapter::new(&resolver, &admission);
        adapter.anchors.set(Ok(crate::xmap15_orders::tests::anchors())).unwrap();
        *adapter.runtime.lock().unwrap() = runtime.clone();
        let gap_world = crate::test_support::frame_with_rest(serde_json::json!({}));
        let gap_selection = Xmap15ViewerSelection { selected_owner: Some((portable_render_stream::Xmap15OwnerKind::Army, 17)), selected_plan: None };
        let topology = portable_render_stream::AssignedStageTopology::seal(vec![
            portable_render_stream::AssignedStageIdentity::new(XMAP15_ASSIGNED_STAGE_NAME, XMAP15_ASSIGNED_OPERATION_NAME)], vec![]).unwrap();
        let schedule = portable_render_stream::AssignedStageSchedule::from_topology(&topology).unwrap();
        let gaps = [crate::MapAuthorityGap { input_id: XMAP15_AUTHORITY_GAP_ID.into(),
            disposition: crate::MapAuthorityRequirementDisposition::Unavailable, reason: "test census gap".into() }];
        let mut gap_context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &gap_world, &viewer, &gaps, &[], &gap_selection).with_test_schedule(&schedule);
        gap_context.xmap15_units = Some(&Units);
        let output = adapter.prepare(&gap_context).unwrap();
        assert!(matches!(output.disposition, FamilyDisposition::Unavailable { .. }));
        assert!(output.prepared_orders.unwrap().native.is_none());
        let mut resumed = adapter.runtime.lock().unwrap();
        assert_eq!(resumed.sequence, runtime.sequence);
        assert_eq!(resumed.objects.len(), 1);
        let recovered = resumed.advance(&context, census, &Units, &base.plan_definitions, 16).unwrap();
        assert!(recovered.plans.is_empty(), "selection during a D54 gap survives the adapter's unavailable return");
        assert!(recovered.viewer_runtime.unwrap().refusals.iter().any(|row| row.input == Missing::SelectedPresentation));
        drop(resumed); drop(adapter); drop(resolver);
        portable_test_fs_retry::remove_dir_all(root).unwrap();

        let selected = Xmap15ViewerSelection { selected_owner: Some((portable_render_stream::Xmap15OwnerKind::Army, 17)), selected_plan: None };
        let selected_context = FoundationFeatureContext::new_for_owned_accounting_tests(&world, &viewer, &[], &[], &selected);
        let input = runtime.advance(&selected_context, census, &Units, &base.plan_definitions, 16).unwrap();
        assert!(input.plans.is_empty());
        assert!(input.viewer_runtime.as_ref().unwrap().refusals.iter().any(|row| row.input == Missing::SelectedPresentation));
        let refused = frame(input, base.clone());
        let witness = lifecycle.apply(&refused).unwrap();
        assert!(witness.created.is_empty() && witness.deleted.is_empty());
        assert!(!witness.authenticated_empty);
        let input = runtime.advance(&context, census, &Units, &base.plan_definitions, 16).unwrap();
        assert!(input.plans.is_empty(), "returning to unselected cannot invent the prior colour-factor state");
        assert_eq!(runtime.objects.len(), 1);
    }
}
