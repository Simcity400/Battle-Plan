mod database;
pub use database::{AuthoredIconDatabase, SelectedAuthoredIcon};
mod authored;
pub use authored::AuthoredIconPoolTypeMap;
pub use map_viewer::authored_graphic_pool::{
    ItemPoolLimit, ItemPoolKey, ItemPoolDefinition, collect_country_pools, collect_equipment_pools,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemPoolRow<T> {
    pub weights_raw: [i64; 3],
    pub icons: Vec<T>,
}

pub fn resolve_collected_icon<T>(
    rows: &[ItemPoolRow<T>],
    mut available: impl FnMut(&T) -> Result<bool, String>,
) -> Result<Option<&T>, String> {
    let mut ordered = Vec::new();
    ordered.try_reserve(rows.len()).map_err(|_| "XUI13 item pool sort allocation refused".to_owned())?;
    ordered.extend(rows);
    let weight = |row: &ItemPoolRow<T>| if row.weights_raw[1] != 0 { row.weights_raw[1] } else { row.weights_raw[0] };
    ordered.sort_by(|a,b| weight(b).cmp(&weight(a)));
    for row in ordered {
        for icon in row.icons.iter().rev() {
            if available(icon)? { return Ok(Some(icon)); }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod traversal_tests {
    use super::*;
    use replay_core::designer_icon_context::CapitalSlotOutcome;
    use serde_json::{json, Value};

    pub(super) fn raw() -> Value {
        json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7,"politics":{"capital_override_raw":112,"capital_state":64,
                    "capital_slot":{"schema":1,"game_hour_raw":43800006,"observation_ordinal":19,"tag_db_id":7,
                        "operands":{"override_raw":112,"base_raw":64},
                        "outcome":{"kind":"present","slot_count":200,"state_id":73,"template_id":21}}}},
                "FRA":{"_r2_tag_db_id":9},"ITA":{"_r2_tag_db_id":3}},
            "diplomacy_world":{"domain53_extra":{"faction_member_order_schema":1,
                "faction_roster_status":"measured_nonempty","faction_rows":[{
                    "faction_id":1,"leader":"GER","leader_tag_db_id":7,"member_count":3,
                    "members":["GER","FRA","ITA"],"member_tag_db_ids":[7,9,3]}]}}})
    }

    pub(super) fn frame(raw: Value) -> replay_core::WorldFrame {
        replay_core::WorldFrame::from_raw_scheduled(replay_core::source::RawFrame {
            hour_index:7,date:None,save_version:None,schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        },None,None,None,replay_core::BuildSchedule::Serial)
    }

    #[test]
    fn nonempty_country_and_faction_pools_append_every_member_region_and_default() {
        let rows=collect_country_pools(&frame(raw()),"GER",|key| Ok(vec![format!("{key:?}")]),
            |state| { assert_eq!(state.operands().effective(),Some(112)); assert_eq!(state.outcome(),CapitalSlotOutcome::Present {slot_count:200,state_id:73,template_id:21}); Ok(Some(0xdeadbeef)) }).unwrap();
        assert_eq!(rows,["Country(\"GER\")","FactionMember(\"GER\")","FactionMember(\"FRA\")",
            "FactionMember(\"ITA\")","CapitalRegion(3735928559)","Default"]);
    }

    #[test]
    fn original_member_order_changes_append_order_without_deduplicating_matches() {
        for (members,ids,expected) in [(json!(["GER","FRA","ITA"]),json!([7,9,3]),["GER","FRA","ITA"]),
            (json!(["GER","ITA","FRA"]),json!([7,3,9]),["GER","ITA","FRA"])] {
            let mut raw=raw();
            raw["diplomacy_world"]["domain53_extra"]["faction_rows"][0]["members"]=members;
            raw["diplomacy_world"]["domain53_extra"]["faction_rows"][0]["member_tag_db_ids"]=ids;
            let rows=collect_country_pools(&frame(raw),"GER",|key| Ok(match key {
                ItemPoolKey::FactionMember(tag) => vec![tag.to_owned(),tag.to_owned()],
                _ => vec![],
            }), |_| Ok(None)).unwrap();
            assert_eq!(rows,expected.into_iter().flat_map(|tag| [tag.to_owned(),tag.to_owned()]).collect::<Vec<_>>());
        }
    }

    #[test]
    fn known_country_rows_do_not_certify_icon_when_later_pool_or_roster_is_unknown() {
        let error=collect_country_pools(&frame(raw()),"GER",|key| match key {
            ItemPoolKey::Country(_) => Ok(vec!["known country row"]),
            ItemPoolKey::FactionMember("GER") => Err("unwitnessed faction pool namespace".into()),
            _ => panic!("unknown pool was skipped"),
        }, |_| panic!("unknown pool was skipped")).unwrap_err();
        assert_eq!(error,"unwitnessed faction pool namespace");
        for extra in [Value::Null,json!({"faction_roster_status":"unavailable","faction_rows":[]}),
            json!({"faction_roster_status":"measured_empty","faction_rows":[]})] {
            let mut raw=raw();raw["diplomacy_world"]["domain53_extra"]=extra;
            let error=collect_country_pools(&frame(raw),"GER",|key| {
                assert_eq!(key,ItemPoolKey::Country("GER"));Ok(vec!["known row"])
            }, |_| panic!("unwitnessed roster was skipped")).unwrap_err();
            assert!(error.contains("faction order"));
        }
    }

    #[test]
    fn only_witnessed_invalid_capital_skips_region_and_still_collects_default() {
        for capital in [0,-1,i32::MIN] {
            let mut raw=raw();raw["countries"]["GER"]["politics"]=json!({"capital_override_raw":capital,"capital_state":0,"capital_slot":{"schema":1,"game_hour_raw":43800006,"observation_ordinal":19,"tag_db_id":7,"operands":{"override_raw":capital,"base_raw":0},"outcome":{"kind":"non_positive"}}});
            let rows=collect_country_pools(&frame(raw),"GER",|key| {
                assert!(!matches!(key,ItemPoolKey::CapitalRegion(_)));
                Ok(if key==ItemPoolKey::Default {vec![1]} else {vec![]})
            }, |_| panic!("nonpositive state reached region lookup"));
            assert_eq!(rows,Ok(vec![1]));
        }
        assert_eq!(collect_country_pools::<()>(&frame(raw()),"GER",|_| Ok(vec![]), |_| Ok(None)),Ok(vec![]));
        let error=collect_country_pools(&frame(raw()),"GER",|key| {
            assert_ne!(key,ItemPoolKey::Default);Ok(vec!["known row"])
        }, |_| Err("state namespace is unobserved".into())).unwrap_err();
        assert_eq!(error,"state namespace is unobserved");
    }

    #[test]
    fn measured_empty_roster_skips_members_but_collects_region_and_default() {
        let mut raw=raw();raw["diplomacy_world"]["domain53_extra"]=json!({
            "faction_member_order_schema":1,"faction_roster_status":"measured_empty","faction_rows":[]});
        let rows=collect_country_pools(&frame(raw),"GER",|key| {
            assert!(!matches!(key,ItemPoolKey::FactionMember(_)));Ok(vec![format!("{key:?}")])
        }, |_| Ok(Some(9))).unwrap();
        assert_eq!(rows,["Country(\"GER\")","CapitalRegion(9)","Default"]);
    }

    #[test]
    fn domain_or_creator_refusal_cannot_be_hidden_by_previously_collected_rows() {
        for domain in [14,53] {
            let mut raw=raw();raw["provenance"]=json!({"collector":{"domain_problems":[{"domain_id":domain,"problem":"partial_data"}]}});
            let result=collect_country_pools(&frame(raw),"GER",|_| Ok(vec!["known row"]),
                |_| panic!("domain refusal was bypassed"));
            assert!(result.is_err(),"domain {domain}");
        }
        let mut raw=raw();raw["countries"]["GER"].as_object_mut().unwrap().remove("_r2_tag_db_id");
        assert!(collect_country_pools::<()>(&frame(raw),"GER",|_| panic!("creator identity was bypassed"),
            |_| panic!("creator identity was bypassed")).unwrap_err().contains("creator"));
    }

    #[test]
    fn authenticated_null_and_out_of_range_skip_region_but_unknown_slot_cannot_skip_competitors() {
        for outcome in [json!({"kind":"null_slot","slot_count":200}),json!({"kind":"out_of_range","slot_count":112})] {
            let mut input=raw();input["countries"]["GER"]["politics"]["capital_slot"]["outcome"]=outcome;
            let rows=collect_country_pools(&frame(input),"GER",|key|Ok(if key==ItemPoolKey::Default {vec![1]} else {vec![]}),
                |_|panic!("witnessed null/out-of-range reached admitted state lookup")).unwrap();
            assert_eq!(rows,[1]);
        }
        for outcome in [json!({"kind":"failed","reason":"state_unreadable"}),
            json!({"kind":"template_unavailable","slot_count":200,"state_id":73,"reason":"membership_unverified"})] {
            let mut input=raw();input["countries"]["GER"]["politics"]["capital_slot"]["outcome"]=outcome;
            let result=collect_country_pools(&frame(input),"GER",|key|{assert_ne!(key,ItemPoolKey::Default);Ok(vec![1])},
                |_|panic!("unknown slot reached admitted lookup"));
            assert!(result.unwrap_err().contains("runtime GS state-slot"));
        }
    }

}

#[cfg(test)]
mod collection_tests {
    use super::*;
    use super::traversal_tests::{frame, raw};

    fn catalog() -> map_viewer::authored_equipment::AuthoredEquipmentCatalog {
        let root = crate::designer_observer::tests::TestRoot(std::env::temp_dir().join(format!(
            "xui13_pool_objects_{}_{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())));
        std::fs::create_dir_all(root.0.join("common/units/equipment")).unwrap();
        let path = "common/units/equipment/pools.txt";
        std::fs::write(root.0.join(path), "equipments = { tank = {} tank_base = {} base = {} other_graphics = {} other_base = {} }").unwrap();
        let (resolver, _) = crate::test_admission::admit_vanilla(&root.0, "english", &[path], "item-pool-object-identity");
        map_viewer::authored_equipment::AuthoredEquipmentCatalog::load(&resolver).unwrap()
    }

    fn pair<'a>(catalog: &'a map_viewer::authored_equipment::AuthoredEquipmentCatalog,
        token: &str, archetype: &str) -> ItemPoolDefinition<'a> {
        ItemPoolDefinition { definition: catalog.definition(token).unwrap(), archetype: catalog.definition(archetype).unwrap() }
    }

    fn row(icon: &'static str, weight: i64) -> ItemPoolRow<&'static str> {
        ItemPoolRow { weights_raw:[weight,0,0],icons:vec![icon] }
    }

    #[test]
    fn later_higher_weight_pool_beats_nonempty_country_and_equal_weights_keep_faction_order() {
        let catalog = catalog();
        let rows=collect_equipment_pools(&frame(raw()),"GER",pair(&catalog,"tank","tank_base"),
            || Ok(pair(&catalog,"tank","tank_base")),|key,definition| {
                if definition.token!="tank" { return Ok(vec![]); }
                Ok(match key {
                    ItemPoolKey::Country(_) => vec![row("country",10)],
                    ItemPoolKey::FactionMember("FRA") => vec![row("FRA",20)],
                    ItemPoolKey::FactionMember("ITA") => vec![row("ITA",20)],
                    ItemPoolKey::Default => vec![row("default",5)],
                    _ => vec![],
                })
            }, |_| Ok(None)).unwrap();
        assert_eq!(resolve_collected_icon(&rows, |_| Ok(true)).unwrap(),Some(&"FRA"));
        let mut reordered=raw();
        reordered["diplomacy_world"]["domain53_extra"]["faction_rows"][0]["members"]=serde_json::json!(["GER","ITA","FRA"]);
        reordered["diplomacy_world"]["domain53_extra"]["faction_rows"][0]["member_tag_db_ids"]=serde_json::json!([7,3,9]);
        let rows=collect_country_pools(&frame(reordered),"GER",|key| Ok(match key {
            ItemPoolKey::FactionMember("FRA") => vec![row("FRA",20)],
            ItemPoolKey::FactionMember("ITA") => vec![row("ITA",20)],
            _ => vec![],
        }), |_| Ok(None)).unwrap();
        assert_eq!(resolve_collected_icon(&rows, |_| Ok(true)).unwrap(),Some(&"ITA"));
    }

    #[test]
    fn same_graphics_identity_skips_second_pass_but_same_archetype_is_not_deduplicated() {
        let catalog = catalog();
        let mut calls=Vec::new();
        let rows=collect_equipment_pools(&frame(raw()),"GER",pair(&catalog,"base","base"),
            || Ok(pair(&catalog,"base","base")),|key,definition| {
                calls.push((format!("{key:?}"),definition.token.clone()));Ok(vec![definition.token.clone()])
            }, |_| Ok(None)).unwrap();
        assert_eq!(calls.len(),10);
        assert!(calls.chunks_exact(2).all(|pair| pair[0]==pair[1]));
        assert_eq!(rows,vec!["base";10]);
    }

    #[test]
    fn distinct_graphics_source_and_its_archetype_append_after_original_pass() {
        let catalog = catalog();
        let rows=collect_equipment_pools(&frame(raw()),"GER",pair(&catalog,"tank","base"),
            || Ok(pair(&catalog,"other_graphics","other_base")),|key,definition| {
                Ok(if key==ItemPoolKey::Default {vec![definition.token.clone()]} else {vec![]})
            }, |_| Ok(None)).unwrap();
        assert_eq!(rows,["tank","base","other_graphics","other_base"]);
        assert!(collect_equipment_pools(&frame(raw()),"GER",pair(&catalog,"tank","base"),
            || Err("unresolved graphics source".into()),|_,_| Ok(vec!["known original row"]),
            |_| Ok(None)).unwrap_err().contains("graphics source"));
    }

    #[test]
    fn equal_name_and_authored_values_do_not_collapse_distinct_retained_objects() {
        let catalog = catalog();
        let original = catalog.definition("tank").unwrap();
        let retained_copy = original.clone();
        assert_eq!(original, &retained_copy);
        assert!(!std::ptr::eq(original, &retained_copy));
        let rows = collect_equipment_pools(&frame(raw()), "GER", pair(&catalog,"tank","base"),
            || Ok(ItemPoolDefinition {definition:&retained_copy,archetype:catalog.definition("base").unwrap()}),
            |key, definition| Ok(if key == ItemPoolKey::Default && definition.token == "tank" {
                vec![if std::ptr::eq(definition,original) { "original" } else {
                    assert!(std::ptr::eq(definition,&retained_copy)); "retained copy"
                }]
            } else { vec![] }), |_| Ok(None)).unwrap();
        assert_eq!(rows,["original","retained copy"]);
    }

    #[test]
    fn sorting_uses_signed_icon_weight_or_generic_and_reverse_icons_without_skipping_unknown() {
        let mut rows=vec![ItemPoolRow {weights_raw:[i64::MAX,-1,i64::MAX],icons:vec!["negative"]},
            ItemPoolRow {weights_raw:[5,0,0],icons:vec!["first","last"]},
            ItemPoolRow {weights_raw:[i64::MIN,4,0],icons:vec!["four"]}];
        let mut visited=Vec::new();
        assert_eq!(resolve_collected_icon(&rows,|icon| {visited.push(*icon);Ok(*icon=="first")}).unwrap(),Some(&"first"));
        assert_eq!(visited,["last","first"]);
        visited.clear();
        assert_eq!(resolve_collected_icon(&rows,|icon| {visited.push(*icon);Ok(*icon=="four")}).unwrap(),Some(&"four"));
        assert_eq!(visited,["last","first","four"]);
        visited.clear();
        assert_eq!(resolve_collected_icon(&rows,|icon| {
            visited.push(*icon);
            if *icon=="last" { Err("unwitnessed icon resource".into()) } else { Ok(true) }
        }),Err("unwitnessed icon resource".into()));
        assert_eq!(visited,["last"]);
        rows.extend((0..40).map(|_| row("later equal",5)));
        assert_eq!(resolve_collected_icon(&rows, |_| Ok(true)).unwrap(),Some(&"last"));
        assert_eq!(resolve_collected_icon(&rows, |_| Ok(false)).unwrap(),None);
    }
}
