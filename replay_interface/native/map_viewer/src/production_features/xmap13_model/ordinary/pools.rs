use super::*;
use crate::authored_graphic_pool::{
    AuthoredGraphicPool, AuthoredGraphicPoolTypeMap, GraphicPoolRootKey, ItemPoolLimit,
    collect_named_equipment_pools, graphic_pool_roots,
};
use crate::{ContentAdmissionLease, GameFileResolver};
use portable_content_manifest::{UsedContentKey, ValidatedContentAdmission};
use replay_core::country_name_semantics::NamingInput;
use std::collections::BTreeSet;

pub(super) struct ModelPools {
    lease: ContentAdmissionLease,
    sources: Vec<UsedContentKey>,
    roots: BTreeMap<GraphicPoolRootKey, std::result::Result<AuthoredGraphicPoolTypeMap<ItemPoolLimit>, String>>,
    context: crate::authored_icon_context::AuthoredIconContext,
    cultures: std::sync::OnceLock<std::result::Result<super::culture::CultureRegistry, String>>,
}

impl ModelPools {
    pub(super) fn load(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Result<Self> {
        let lease = resolver.content_admission_lease().ok_or("XMAP13 model pools require content admission")?;
        if lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256()) {
            return Err("XMAP13 model pool resolver and admission disagree".into());
        }
        let countries = crate::country_tags_from_admission_lease(&lease)?;
        let context = crate::authored_icon_context::AuthoredIconContext::from_admission_lease(&lease)?;
        let sources = lease.require_complete_admitted_dir("gfx/interface/equipmentdesigner/graphic_db",
            Some("txt"), false, "XMAP13 model graphic pools")?;
        let mut text = Vec::new();
        text.try_reserve(sources.len())?;
        for source in &sources {
            text.push((source.normalized_game_relative_path.clone(),
                crate::latin1_bytes_to_string(&lease.read_used_content(source)?)));
        }
        let roots = parse_roots(&text, &countries, &context.continent_names())?;
        Ok(Self { lease, sources, roots, context, cultures: std::sync::OnceLock::new() })
    }

    pub(super) fn checked_rows(
        &self, resolver: &GameFileResolver, admission: &ValidatedContentAdmission,
        equipment: &AuthoredEquipmentCatalog, world: &replay_core::WorldFrame,
        country: &str, variant: &NativeDesignerVariantInputs, subunit_name: &str,
        runtime: &snapshot::design_catalog::DesignU32Observation,
    ) -> Result<Vec<AuthoredGraphicPool<ItemPoolLimit>>> {
        if resolver.content_admission_lease().as_ref() != Some(&self.lease)
            || self.lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256()) {
            return Err("XMAP13 model pools belong to another content admission".into());
        }
        let equipment_sources = self.lease.require_complete_admitted_dir("common/units/equipment",
            Some("txt"), true, "XMAP13 model equipment source binding")?;
        if !equipment.sources().iter().map(|row| &row.source).eq(equipment_sources.iter()) {
            return Err("XMAP13 equipment catalog disagrees with the complete ordered admitted source set".into());
        }
        for source in &self.sources {
            if admission.require_used_content(&source.normalized_game_relative_path)? != source {
                return Err("XMAP13 model pool source identity changed".into());
            }
        }
        // 14191a090 keeps creator filters separate from the helper country passed to 1409f0bb0.
        let context = world.country_name_semantics(&variant.model.creator_tag);
        let rows = collect_named_equipment_pools(equipment, world, country,
            &variant.equipment_definition_token,
            |root, definition| {
                let Some(rows) = self.roots.get(root) else { return Ok(Vec::new()); };
                let rows = rows.as_ref().map_err(Clone::clone)?.matching_rows(&definition.token,
                    |limit| limit.matches(runtime), || Ok(subunit_name.to_owned()),
                    || naming_token(&context.ruling_party_group, "creator ruling-party group"),
                    || self.cultures.get_or_init(|| super::culture::CultureRegistry::load(resolver, admission)
                        .map_err(|error| error.to_string())).as_ref().map_err(Clone::clone)?
                        .name(resolver, admission, &context.graphical_culture).map(str::to_owned))?;
                let mut output = Vec::new();
                output.try_reserve(rows.len()).map_err(|_| "XMAP13 matching model pools allocation refused")?;
                output.extend(rows.into_iter().cloned());
                Ok(output)
            },
            |observation| {
                let replay_core::designer_icon_context::CapitalSlotOutcome::Present { template_id, .. }
                    = observation.outcome() else { return Err("XMAP13 capital callback requires a present authenticated state template".into()); };
                self.context.state_continents().map_err(|error| error.to_string())?
                    .get(&(template_id as u32)).cloned()
                    .ok_or_else(|| format!("XMAP13 capital template {template_id} is absent from admitted state definitions"))
            })?;
        let mut output = Vec::new();
        output.try_reserve(rows.len())?;
        output.extend(rows.into_iter().map(|row| row.row));
        Ok(output)
    }
}

fn parse_roots(sources: &[(String, String)], countries: &BTreeSet<String>, continents: &BTreeSet<String>)
    -> Result<BTreeMap<GraphicPoolRootKey, std::result::Result<AuthoredGraphicPoolTypeMap<ItemPoolLimit>, String>>> {
    graphic_pool_roots(sources, countries, continents,
        |map: &mut AuthoredGraphicPoolTypeMap<ItemPoolLimit>, value| map.append(value, ItemPoolLimit::parse))
        .map_err(Into::into)
}

fn naming_token(value: &NamingInput<String>, field: &str) -> std::result::Result<String, String> {
    match value {
        NamingInput::Measured(value) => Ok(value.clone()),
        NamingInput::Absent => Ok(String::new()),
        NamingInput::Unavailable(reason) => Err(format!("XMAP13 {field}: {reason:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapshot::design_catalog::{DesignFieldRefusal, DesignU32Observation};

    fn culture_fixture(label: &str, culture: &[u8], pools: &[u8])
        -> (std::path::PathBuf, GameFileResolver, ValidatedContentAdmission, AuthoredEquipmentCatalog) {
        use crate::test_support::{admitted_fixture_read_set, temp_test_dir, write_modvfs_fixture};
        use portable_content_manifest::ContentNamespaceScope;
        let root = temp_test_dir(label);
        for (path, bytes) in [
            ("common/units/equipment/model.txt", &b"equipments = { infantry = { is_archetype = yes } }"[..]),
            ("common/country_tags/test.txt", &br#"GER = "countries/Germany.txt" FRA = "countries/France.txt""#[..]),
            ("map/default.map", &b"continent = continent.txt definitions = definition.csv provinces = provinces.bmp"[..]),
            ("map/continent.txt", &b"continents = { europe }"[..]),
            ("common/graphicalculturetype.txt", culture),
            ("gfx/interface/equipmentdesigner/graphic_db/model.txt", pools),
        ] { write_modvfs_fixture(&root, path, bytes); }
        let raw = GameFileResolver::vanilla(&root).unwrap();
        let scopes = [("common/units/equipment", true), ("common/country_tags", false),
            ("gfx/interface/equipmentdesigner/graphic_db", false)].map(|(directory, recursive)| ContentNamespaceScope {
                directory: directory.into(), extension: Some("txt".into()), recursive,
            });
        let files = ["map/default.map", "map/continent.txt", "common/graphicalculturetype.txt"]
            .map(str::to_owned).into_iter().collect();
        let (resolver, admission) = admitted_fixture_read_set(&raw, &files, &scopes, label);
        let equipment = AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
        (root, resolver, admission, equipment)
    }

    fn culture_world(creator: serde_json::Value) -> replay_core::WorldFrame {
        use serde_json::json;
        let raw = json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7,"name_identity":{"graphical_culture":creator}},
                "FRA":{"_r2_tag_db_id":8,
                    "name_identity":{"graphical_culture":{"status":"measured","value":"french"}},
                    "politics":{"capital_state":0,"capital_override_raw":0,
                        "capital_slot":{"schema":1,"game_hour_raw":43800006,"observation_ordinal":19,"tag_db_id":8,
                            "operands":{"override_raw":0,"base_raw":0},"outcome":{"kind":"non_positive"}}}}},
            "diplomacy_world":{"domain53_extra":{"faction_member_order_schema":1,
                "faction_roster_status":"measured_empty","faction_rows":[]}}});
        replay_core::WorldFrame::from_raw_scheduled(replay_core::RawFrame {
            hour_index:7, date:None, save_version:None, schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        }, None, None, None, replay_core::BuildSchedule::Serial)
    }

    #[test]
    fn admitted_model_pool_culture_uses_current_creator_not_helper_or_previous_frame() {
        use serde_json::json;
        let fixture = culture_fixture("model-pool-current-culture", b"german french @name\n", br#"
            FRA = { infantry = {
                pool = { models = { creator_model } cultures = { german } }
                pool = { models = { helper_model } cultures = { french } }
                pool = { models = { null_model } cultures = { "" } }
                pool = { models = { literal_model } cultures = { "@name" } }
            } }
            GER = { infantry = { pool = { models = { wrong_country_root } } } }
        "#);
        let pools = ModelPools::load(&fixture.1, &fixture.2).unwrap();
        assert!(pools.cultures.get().is_none());
        let variant = super::super::tests::variant(1, "infantry", "GER", 0);
        let runtime = DesignU32Observation::Measured { raw_u32:0 };
        for (token, expected) in [("german", "creator_model"), ("unknown", "null_model"),
            ("", "null_model"), ("GERMAN", "null_model"), ("@name", "literal_model")] {
            let world = culture_world(json!({"status":"measured","value":token}));
            let rows = pools.checked_rows(&fixture.1, &fixture.2, &fixture.3,
                &world, "FRA", &variant, "infantry", &runtime).unwrap();
            assert_eq!(rows.len(), 2, "definition/archetype both append: {token}");
            assert!(rows.iter().all(|row| row.models == [expected]), "{token}: {rows:?}");
        }
        for (observation, reason) in [(json!({"status":"absent"}), "absent"),
            (json!({"status":"unavailable","reason":"changed"}), "Changed"),
            (serde_json::Value::Null, "Malformed")] {
            let world = culture_world(observation);
            let error = pools.checked_rows(&fixture.1, &fixture.2, &fixture.3,
                &world, "FRA", &variant, "infantry", &runtime).unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        }
        assert!(pools.cultures.get().unwrap().is_ok());
        let root = fixture.0.clone();
        drop((pools, fixture));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreached_culture_registry_remains_lazy_and_reached_refusal_is_cached() {
        use serde_json::json;
        let fixture = culture_fixture("model-pool-lazy-culture", b"\xff", br#"
            default = { infantry = {
                pool = { models = { visible } }
                pool = { models = { hidden } sub_units = { other } cultures = { german } }
            } }
        "#);
        let pools = ModelPools::load(&fixture.1, &fixture.2).unwrap();
        let variant = super::super::tests::variant(1, "infantry", "GER", 0);
        let runtime = DesignU32Observation::Measured { raw_u32:0 };
        let world = culture_world(json!({"status":"absent"}));
        let rows = pools.checked_rows(&fixture.1, &fixture.2, &fixture.3,
            &world, "FRA", &variant, "infantry", &runtime).unwrap();
        assert!(rows.iter().all(|row| row.models == ["visible"]));
        assert_eq!(rows.len(), 2);
        assert!(pools.cultures.get().is_none());
        for _ in 0..2 {
            let error = pools.checked_rows(&fixture.1, &fixture.2, &fixture.3,
                &world, "FRA", &variant, "other", &runtime).unwrap_err();
            assert!(error.to_string().contains("byte FF"), "{error}");
            assert!(pools.cultures.get().unwrap().is_err());
        }
        let root = fixture.0.clone();
        drop((rows, pools, fixture));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loaded_model_pools_reject_a_foreign_equipment_catalog_before_country_traversal() {
        use crate::test_support::{admitted_fixture_read_set, temp_test_dir, write_modvfs_fixture};
        use portable_content_manifest::ContentNamespaceScope;
        use serde_json::json;
        let make = |label: &str, definitions: &[u8]| {
            let root = temp_test_dir(label);
            write_modvfs_fixture(&root,"common/units/equipment/model.txt",definitions);
            write_modvfs_fixture(&root,"common/country_tags/test.txt",b"GER = countries/Germany.txt");
            write_modvfs_fixture(&root,"map/default.map",b"continent = continent.txt definitions = definition.csv provinces = provinces.bmp");
            write_modvfs_fixture(&root,"map/continent.txt",b"continents = { europe }");
            write_modvfs_fixture(&root,"gfx/interface/equipmentdesigner/graphic_db/model.txt",
                b"default = { infantry = { pool = { models = { ordinary } } } }");
            let raw = GameFileResolver::vanilla(&root).unwrap();
            let scopes = [("common/units/equipment",true),("common/country_tags",false),
                ("gfx/interface/equipmentdesigner/graphic_db",false)].map(|(directory,recursive)| ContentNamespaceScope {
                    directory:directory.into(), extension:Some("txt".into()), recursive,
                });
            let files = ["map/default.map".into(),"map/continent.txt".into()].into_iter().collect();
            let (resolver,admission) = admitted_fixture_read_set(&raw,&files,&scopes,label);
            let equipment = AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
            (root,raw,resolver,admission,equipment)
        };
        let a = make("model-pools-equipment-a",b"equipments = { infantry = { is_archetype = yes } }");
        let b = make("model-pools-equipment-b",b"equipments = { artillery = { is_archetype = yes } infantry = { archetype = artillery } }");
        let pools = ModelPools::load(&a.2,&a.3).unwrap();
        let raw = json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7,"politics":{"capital_state":0,"capital_override_raw":0,
                "capital_slot":{"schema":1,"game_hour_raw":43800006,"observation_ordinal":19,"tag_db_id":7,
                    "operands":{"override_raw":0,"base_raw":0},"outcome":{"kind":"non_positive"}}}}},
            "diplomacy_world":{"domain53_extra":{"faction_member_order_schema":1,
                "faction_roster_status":"measured_empty","faction_rows":[]}}});
        let world = replay_core::WorldFrame::from_raw_scheduled(replay_core::RawFrame {
            hour_index:7,date:None,save_version:None,schema_version:None,rest:raw.as_object().unwrap().clone().into(),
        },None,None,None,replay_core::BuildSchedule::Serial);
        let variant = super::super::tests::variant(1,"infantry","GER",0);
        let runtime = DesignU32Observation::Measured { raw_u32:0 };
        let rows = pools.checked_rows(&a.2,&a.3,&a.4,&world,"GER",&variant,"infantry",&runtime).unwrap();
        assert_eq!(rows.len(),2,"definition and same-object archetype both append");
        assert_eq!(rows[0].models,["ordinary"]);
        let error = pools.checked_rows(&a.2,&a.3,&b.4,&world,"not-a-country",&variant,"infantry",&runtime).unwrap_err();
        assert!(error.to_string().contains("complete ordered admitted source set"),"{error}");
        assert!(pools.checked_rows(&b.2,&b.3,&b.4,&world,"GER",&variant,"infantry",&runtime)
            .unwrap_err().to_string().contains("another content admission"));
        assert!(pools.checked_rows(&a.2,&b.3,&a.4,&world,"GER",&variant,"infantry",&runtime).is_err());
        let roots = [a.0.clone(),b.0.clone()];
        drop((rows,pools,a,b));
        for root in roots { portable_test_fs_retry::remove_dir_all(root).unwrap(); }
    }

    #[test]
    fn default_models_use_shared_pool_limit_order_and_refuse_only_reached_filter_inputs() {
        let roots = parse_roots(&[("model.txt".into(), r#"
            default = { infantry = {
                pool = { models_weight = 2 models = { lower } }
                pool = { models_weight = 4 models = { first second } }
                pool = { models_weight = 9 models = { hidden } limit = { has_country_flag = absent } cultures = { unavailable } }
            } }
            GER = { infantry = { pool = { models = { needs_culture } cultures = { german } } } }
        "#.into())], &["GER".into()].into_iter().collect(), &BTreeSet::new()).unwrap();
        let runtime = DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "unreached runtime mask".into(),
        } };
        let unknown = || -> std::result::Result<String, String> { Err("unavailable filter".into()) };
        let rows = roots[&GraphicPoolRootKey::Default].as_ref().unwrap().matching_rows("infantry",
            |limit| limit.matches(&runtime), unknown, unknown, unknown).unwrap();
        let mut list = ModelList::default();
        for row in crate::authored_graphic_pool::model_items(&rows).unwrap() {
            list.graphic(row, &mut |token| Ok(Some(token.to_owned()))).unwrap();
        }
        assert_eq!(list.rows.iter().map(|row| (row.token.as_str(), row.weight_raw)).collect::<Vec<_>>(),
            [("second",400_000),("first",400_000),("lower",200_000)]);
        let error = roots[&GraphicPoolRootKey::Country("GER".into())].as_ref().unwrap().matching_rows("infantry",
            |limit| limit.matches(&runtime), unknown, unknown, unknown).unwrap_err();
        assert_eq!(error, "unavailable filter");
    }
}
