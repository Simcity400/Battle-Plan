mod database;
pub use database::{AuthoredIconDatabase, SelectedAuthoredIcon};
mod authored;
pub use authored::{AuthoredIconPool, AuthoredIconPoolTypeMap};
use crate::ability_visibility::{
    ability_visibility, parse_allowed_body, AllowedBody, AllowedScope, AllowedWorldFacts, Operand,
    Verdict,
};
use crate::paradox_text::Value;
use replay_core::design_catalog::DesignU32Observation;
use map_viewer::authored_equipment::AuthoredEquipmentDefinition;
use std::cell::Cell;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemPoolKey<'a> {
    Country(&'a str),
    FactionMember(&'a str),
    CapitalRegion(u32),
    Default,
}

/// Rows retain pool traversal order; resource existence is evaluated after collection and sorting.
/// `capital_region` must authenticate the state lookup; `None` means witnessed invalid state.
pub fn collect_country_pools<T>(
    frame: &replay_core::WorldFrame,
    creator: &str,
    mut matching_rows: impl FnMut(ItemPoolKey<'_>) -> Result<Vec<T>, String>,
    capital_region: impl FnOnce(i32) -> Result<Option<u32>, String>,
) -> Result<Vec<T>, String> {
    fn append<T>(rows: &mut Vec<T>, mut next: Vec<T>) -> Result<(), String> {
        rows.try_reserve(next.len()).map_err(|_| "XUI13 item pool row allocation refused".to_owned())?;
        rows.append(&mut next);
        Ok(())
    }
    frame.country_identity_for_tag(creator)
        .map_err(|reason| format!("XUI13 item pool creator: {reason}"))?;
    let mut rows = matching_rows(ItemPoolKey::Country(creator))?;

    // 1409f1540 appends through its callback but returns zero; 1409f0bb0 continues every pool.
    let factions = match frame.ordered_faction_roster()
        .map_err(|reason| format!("XUI13 item pool faction order: {reason:?}"))? {
        replay_core::Leaf::Measured(rows) => rows,
        other => return Err(format!("XUI13 item pool faction order: {other:?}")),
    };
    if let Some(faction) = factions.iter().find(|row| row.members.iter().any(|member| member.tag == creator)) {
        for member in &faction.members {
            append(&mut rows, matching_rows(ItemPoolKey::FactionMember(&member.tag))?)?;
        }
    }

    let capital = frame.designer_effective_capital(creator)
        .map_err(|reason| format!("XUI13 item pool effective capital: {reason:?}"))?;
    if capital > 0 {
        if let Some(region) = capital_region(capital)? {
            append(&mut rows, matching_rows(ItemPoolKey::CapitalRegion(region))?)?;
        }
    }
    append(&mut rows, matching_rows(ItemPoolKey::Default)?)?;
    Ok(rows)
}

/// Borrow retained objects from one immutable admitted catalog; never re-resolve a copy by name.
#[derive(Clone, Copy, Debug)]
pub struct ItemPoolDefinition<'a> {
    pub definition: &'a AuthoredEquipmentDefinition,
    pub archetype: &'a AuthoredEquipmentDefinition,
}

pub fn collect_icon_pools<'a, T>(
    frame: &replay_core::WorldFrame,
    creator: &str,
    definition: ItemPoolDefinition<'a>,
    graphics_source: impl FnOnce() -> Result<ItemPoolDefinition<'a>, String>,
    mut matching_rows: impl FnMut(ItemPoolKey<'_>, &AuthoredEquipmentDefinition) -> Result<Vec<T>, String>,
    mut capital_region: impl FnMut(i32) -> Result<Option<u32>, String>,
) -> Result<Vec<T>, String> {
    let mut collect = |definition: ItemPoolDefinition<'_>| collect_country_pools(frame, creator, |key| {
        let mut rows = matching_rows(key, definition.definition)?;
        // 1409f13f0 follows the zero-return collector with +4d8 even when identities coincide.
        let mut inherited = matching_rows(key, definition.archetype)?;
        rows.try_reserve(inherited.len()).map_err(|_| "XUI13 item pool row allocation refused".to_owned())?;
        rows.append(&mut inherited);
        Ok(rows)
    }, &mut capital_region);
    let mut rows = collect(definition)?;
    let graphics = graphics_source()?;
    if !std::ptr::eq(graphics.definition, definition.definition) {
        let mut additional = collect(graphics)?;
        rows.try_reserve(additional.len()).map_err(|_| "XUI13 item pool row allocation refused".to_owned())?;
        rows.append(&mut additional);
    }
    Ok(rows)
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemPoolLimit(AllowedBody);

impl ItemPoolLimit {
    pub fn parse(value: &Value) -> Result<Self, String> {
        parse_allowed_body(value).map(Self)
            .map_err(|reason| format!("XUI13 item pool limit: {reason}"))
    }

    pub fn matches(&self, runtime: &DesignU32Observation) -> Result<bool, String> {
        let facts = RuntimeFacts { runtime, unexpected: Cell::new(None) };
        let result = self.evaluate(&facts);
        if let Some(field) = facts.unexpected.get() {
            return Err(format!("XUI13 item pool default scope requested {field}"));
        }
        result
    }

    fn evaluate(&self, facts: &dyn AllowedWorldFacts) -> Result<bool, String> {
        // 1409f1540 constructs a fresh 140528800 scope immediately before the limit call.
        let scope = AllowedScope { country_tag: None, caster: None, formation_selected: false };
        match ability_visibility("", false, &self.0, &scope, facts) {
            Verdict::Visible => Ok(true),
            Verdict::Hidden => Ok(false),
            Verdict::Refused(reason) => Err(format!("XUI13 item pool limit: {reason}")),
        }
    }
}

struct RuntimeFacts<'a> {
    runtime: &'a DesignU32Observation,
    unexpected: Cell<Option<&'static str>>,
}

impl AllowedWorldFacts for RuntimeFacts<'_> {
    fn dlc_mask(&self) -> Operand<u32> {
        match self.runtime {
            DesignU32Observation::Measured { raw_u32 } => Operand::Recorded(*raw_u32),
            DesignU32Observation::Unavailable { .. } => Operand::Unrecorded("A27 runtime_dlc_mask"),
        }
    }

    fn completed_focus(&self, _: &str, _: &str) -> Operand<bool> {
        self.unexpected.set(Some("country completed focus"));
        Operand::Unrecorded("item pool scope has no country")
    }

    fn country_flag_value(&self, _: &str, _: &str) -> Operand<Option<i64>> {
        self.unexpected.set(Some("country flag"));
        Operand::Unrecorded("item pool scope has no country")
    }

    fn trait_unlocks_ability(&self, _: &str, _: &str) -> Option<bool> {
        self.unexpected.set(Some("trait ability"));
        None
    }

    fn ability_exists(&self, _: &str) -> bool {
        self.unexpected.set(Some("ability catalog"));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;
    use replay_core::design_catalog::DesignFieldRefusal;

    fn limit(text: &str) -> ItemPoolLimit {
        let doc = parse_str(&format!("limit = {{ {text} }}"));
        ItemPoolLimit::parse(doc.root.first_value_ci("limit").unwrap()).unwrap()
    }

    struct NoWorldFacts;
    impl AllowedWorldFacts for NoWorldFacts {
        fn dlc_mask(&self) -> Operand<u32> { panic!("unreached runtime fact") }
        fn completed_focus(&self, _: &str, _: &str) -> Operand<bool> { panic!("creator country was installed") }
        fn country_flag_value(&self, _: &str, _: &str) -> Operand<Option<i64>> { panic!("creator country was installed") }
        fn trait_unlocks_ability(&self, _: &str, _: &str) -> Option<bool> { panic!("ability clause was enabled") }
        fn ability_exists(&self, _: &str) -> bool { panic!("ability clause was enabled") }
    }

    #[test]
    fn default_scope_never_reads_creator_or_ability_facts() {
        for text in ["has_country_flag = creator_flag", "has_completed_focus = creator_focus",
            "has_ability = force_attack", "OWNER = { has_country_flag = creator_flag }",
            "is_leading_army_group = no", "is_border_war = no"] {
            assert_eq!(limit(text).evaluate(&NoWorldFacts), Ok(false), "{text}");
        }
        assert_eq!(limit("").evaluate(&NoWorldFacts), Ok(true));
        assert_eq!(limit("NOT = { has_country_flag = creator_flag }").evaluate(&NoWorldFacts), Ok(true));
    }

    #[test]
    fn runtime_feature_mask_drives_dlc_trigger_without_content_name_substitution() {
        let query = limit(r#"has_dlc = "Man the Guns""#);
        assert_eq!(query.matches(&DesignU32Observation::Measured { raw_u32: 0x10 }), Ok(true));
        assert_eq!(query.matches(&DesignU32Observation::Measured { raw_u32: 0x80 }), Ok(false));
        let unknown = DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "runtime_dlc_mask".into(),
        }};
        assert!(query.matches(&unknown).unwrap_err().contains("DLC"));
        assert_eq!(limit(r#"has_dlc = "man the guns""#).matches(&unknown), Ok(false));
        assert_eq!(limit("").matches(&unknown), Ok(true));
    }

    #[test]
    fn short_circuit_and_correlated_unknowns_reuse_existing_trigger_law() {
        assert_eq!(limit(r#"has_country_flag = no_scope has_dlc = "Man the Guns""#)
            .evaluate(&NoWorldFacts), Ok(false));
        let unknown = DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "runtime_dlc_mask".into(),
        }};
        assert_eq!(limit(r#"OR = { has_dlc = "Man the Guns" NOT = { has_dlc = "Man the Guns" } }"#)
            .matches(&unknown), Ok(true));
        assert_eq!(limit(r#"has_dlc = "Man the Guns" NOT = { has_dlc = "Man the Guns" }"#)
            .matches(&unknown), Ok(false));
    }

    #[test]
    fn unsupported_authored_trigger_is_named_instead_of_admitted() {
        let doc = parse_str("limit = { unproven_icon_trigger = yes }");
        assert!(ItemPoolLimit::parse(doc.root.first_value_ci("limit").unwrap())
            .unwrap_err().contains("unproven_icon_trigger"));
    }
}

#[cfg(test)]
mod traversal_tests {
    use super::*;
    use serde_json::{json, Value};

    pub(super) fn raw() -> Value {
        json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7,"politics":{"capital_override_raw":112,"capital_state":64}},
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
            |state| { assert_eq!(state,112); Ok(Some(0xdeadbeef)) }).unwrap();
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
            let mut raw=raw();raw["countries"]["GER"]["politics"]=json!({"capital_override_raw":capital,"capital_state":0});
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
        let rows=collect_icon_pools(&frame(raw()),"GER",pair(&catalog,"tank","tank_base"),
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
        let rows=collect_icon_pools(&frame(raw()),"GER",pair(&catalog,"base","base"),
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
        let rows=collect_icon_pools(&frame(raw()),"GER",pair(&catalog,"tank","base"),
            || Ok(pair(&catalog,"other_graphics","other_base")),|key,definition| {
                Ok(if key==ItemPoolKey::Default {vec![definition.token.clone()]} else {vec![]})
            }, |_| Ok(None)).unwrap();
        assert_eq!(rows,["tank","base","other_graphics","other_base"]);
        assert!(collect_icon_pools(&frame(raw()),"GER",pair(&catalog,"tank","base"),
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
        let rows = collect_icon_pools(&frame(raw()), "GER", pair(&catalog,"tank","base"),
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
