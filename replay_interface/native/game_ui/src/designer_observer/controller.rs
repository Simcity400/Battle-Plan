use super::authored_inputs::{designer_candidate_base, designer_family_matches, DesignerAuthoredInputs};
use crate::design_catalog::{DesignerRowIcon, ObserverDesignerCandidate, ObserverDesignerCatalog,
    DesignerUnresolvedSuffix, ObserverDesignerCountryFilterRow, ObserverDesignerViewState};
use replay_core::{ObserverDesignerFamily, WorldFrame};
use replay_core::design_catalog::{DesignItemIconObservation, DesignTextObservation, NativeDesignerInputs};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn admit(authored: &DesignerAuthoredInputs, frame: &WorldFrame,
    native: Result<NativeDesignerInputs, String>, owner: &str, family: ObserverDesignerFamily,
    viewer: &ObserverDesignerViewState,
) -> Result<ObserverDesignerCatalog, String> {
    let native = native?;
    if i64::try_from(native.hour).ok() != Some(frame.hour_index()) { return Err("XUI13 A27 hour disagrees with frame".into()) }
    let equipment = authored.for_frame(frame).map_err(|error| error.to_string())?;
    if !native.countries.iter().any(|country| country.tag == owner) {
        return Err(format!("XUI13 selected country {owner:?} is absent from A27"));
    }
    let mut excluded = BTreeSet::new();
    for tag in &viewer.excluded_country_filter_tags {
        if !native.countries.iter().any(|country| country.tag == *tag) || !excluded.insert(tag.as_str()) {
            return Err(format!("XUI13 excluded country {tag:?} is absent or duplicate"));
        }
    }
    let mut field_refusals = BTreeMap::new();
    let mut anchor_design_id = String::new();
    if viewer.anchor_design_id.is_empty() {
        for variant in &native.variants {
            match equipment.designer_classification(&variant.equipment_definition_token) {
                Ok(classification) if designer_family_matches(classification, family) => {
                    anchor_design_id = variant.design_id.clone();
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    field_refusals.insert("anchor_design".into(), format!("XUI13 anchor at {}: {error}", variant.design_id));
                    break;
                }
            }
        }
    } else {
        let anchor = native.variants.iter().find(|row| row.design_id == viewer.anchor_design_id)
            .ok_or_else(|| format!("XUI13 anchor {:?} is absent", viewer.anchor_design_id))?;
        match equipment.designer_classification(&anchor.equipment_definition_token) {
            Ok(classification) if designer_family_matches(classification, family) => anchor_design_id = anchor.design_id.clone(),
            Ok(_) => return Err(format!("XUI13 anchor {:?} does not belong to {family:?}", anchor.design_id)),
            Err(error) => { field_refusals.insert("anchor_design".into(), format!("XUI13 anchor {}: {error}", anchor.design_id)); }
        }
    }
    let mut occurrences = BTreeMap::new();
    for variant in &native.variants {
        for occurrence in &variant.source_occurrences {
            let list = match occurrence.source_list.as_str() { "A" => 0u8, "B" => 1, _ => return Err("XUI13 invalid source list".into()) };
            if occurrences.insert((occurrence.source_country_registry_index, list, occurrence.source_list_ordinal), variant).is_some() {
                return Err("XUI13 duplicate source-list slot".into());
            }
        }
    }
    let countries: BTreeMap<_, _> = native.countries.iter().map(|country| (country.registry_index, country)).collect();
    let mut country_admission = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut country_windows = BTreeMap::<String, Vec<Vec<String>>>::new();
    let mut ordered = Vec::new();
    let mut unresolved_suffix = None;
    for ((country_index, list, ordinal), variant) in occurrences {
        if seen.contains(variant.design_id.as_str()) { continue }
        let classification = equipment.designer_classification(&variant.equipment_definition_token);
        if classification.as_ref().is_ok_and(|classification| !designer_family_matches(*classification, family)) { continue }
        let candidate = (|| -> Result<Option<ObserverDesignerCandidate>, String> {
            let admitted = country_admission.entry(country_index).or_insert_with(|| {
                let country = countries.get(&country_index).ok_or_else(|| format!("XUI13 source country {country_index} is absent"))?;
                if country.tag == owner { Ok(true) } else {
                    authored.thresholds.admits_recorded_foreign_country(frame, &native, family, owner, &country.tag)
                        .map_err(|error| format!("XUI13 country {:?}: {error:?}", country.tag))
                }
            });
            if !*admitted.as_ref().map_err(Clone::clone)? { return Ok(None) }
            seen.insert(variant.design_id.as_str());
            let classification = classification.map_err(|error| error.to_string())?;
            if !designer_family_matches(classification, family)
                || !designer_candidate_base(&equipment, &native.runtime_dlc_mask, variant).map_err(|error| error.to_string())? { return Ok(None) }
            let windows = equipment.designer_window_tokens(&variant.equipment_definition_token).map_err(|error| error.to_string())?;
            if !authored.admits_window(&windows, owner).map_err(|error| error.to_string())? { return Ok(None) }
            let selected_country = variant.model.origin_override_tag.as_deref().unwrap_or(&variant.model.creator_tag);
            let is_our = selected_country == owner;
            if viewer.foreign_filter_enabled && !is_our {
                country_windows.entry(selected_country.into()).or_default().push(windows.clone());
            }
            if (!viewer.our_filter_enabled && is_our)
                || (!viewer.foreign_filter_enabled && (variant.model.origin_override_tag.is_some() || !is_our))
                || (!viewer.obsolete_filter_enabled && variant.record.lifecycle_state == Some(replay_core::DesignLifecycleState::Obsolete))
                || excluded.contains(selected_country) { return Ok(None) }
            Ok(Some(ObserverDesignerCandidate {
                record: variant.record.clone(), designer_window_tokens: windows,
                icon: item_icon(&variant.item_icon_override),
                missions: Err("XUI13 authored mission mask and mutable mission-stat operands are not yet bound".into()),
            }))
        })();
        match candidate {
            Ok(Some(row)) => ordered.push(row),
            Ok(None) => {}
            Err(reason) => {
                unresolved_suffix = Some(DesignerUnresolvedSuffix {
                    source_country_registry_index: country_index, source_list: if list == 0 { "A" } else { "B" }.into(),
                    source_list_ordinal: ordinal, design_id: variant.design_id.clone(), reason,
                });
                break;
            }
        }
    }
    let country_filter_rows = (|| -> Result<Vec<ObserverDesignerCountryFilterRow>, String> {
        if viewer.foreign_filter_enabled && unresolved_suffix.is_some() {
            return Err("XUI13 unresolved row suffix may change pointer-ordered country-filter membership".into());
        }
        let mut rows = Vec::new();
        for country in &native.countries {
            if let Some(windows) = country_windows.remove(&country.tag) {
                let ordinal = country.designer_pointer_order_ordinal.ok_or_else(|| format!("XUI13 country {:?} has no pointer-order observation", country.tag))?;
                rows.push((ordinal, ObserverDesignerCountryFilterRow { country_tag: country.tag.clone(), candidate_designer_window_tokens: windows }));
            }
        }
        if !country_windows.is_empty() { return Err("XUI13 filter creator is absent from A27".into()) }
        rows.sort_by_key(|(ordinal, _)| *ordinal);
        if rows.windows(2).any(|rows| rows[0].0 == rows[1].0) { return Err("XUI13 country pointer order is ambiguous".into()) }
        let rows: Vec<_> = rows.into_iter().map(|(_, row)| row).collect();
        if excluded.iter().any(|tag| !rows.iter().any(|row| row.country_tag == *tag)) {
            return Err("XUI13 selected country filter is absent from controller strip".into());
        }
        Ok(rows)
    })().unwrap_or_else(|reason| { field_refusals.insert("country_filters".into(), reason); Vec::new() });
    Ok(ObserverDesignerCatalog { source_record_id: native.source_record_id.clone(), anchor_design_id,
        country_filter_rows, ordered, unresolved_suffix, field_refusals })
}

fn item_icon(icon: &DesignItemIconObservation) -> Result<DesignerRowIcon, String> {
    match icon {
        DesignItemIconObservation::Measured { kind, token: DesignTextObservation::Measured { value, .. }, .. } => {
            match (*kind, value.as_str()) {
                (1, ".") => Err("XUI13 authored item-database fallback is not yet bound".into()),
                (0, value) => Ok(DesignerRowIcon::Sprite(value.into())),
                (1, value) => Ok(DesignerRowIcon::Texture(value.into())),
                _ => Err("XUI13 item icon has an invalid kind".into()),
            }
        }
        DesignItemIconObservation::Measured { token: DesignTextObservation::Unavailable { reason }, .. }
        | DesignItemIconObservation::Unavailable { reason } => Err(format!("XUI13 item_icon_override: {reason:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{admitted_production_renderer, designer_catalog_value, production_fixture, world_from_catalog};
    use serde_json::{json, Value};

    fn viewer() -> ObserverDesignerViewState {
        ObserverDesignerViewState {
            family: crate::readonly_design_peace::DesignerViewKind::Tank,
            anchor_design_id: String::new(), anchor_catalogue_hour: -1,
            anchor_catalogue_owner_tag: crate::stored_country_selection::StoredCountrySelection::of("GER"),
            anchor_catalogue_family: crate::readonly_design_peace::DesignerViewKind::Tank,
            selected_design_id: None, our_filter_enabled: true, foreign_filter_enabled: true,
            obsolete_filter_enabled: true, excluded_country_filter_tags: vec![],
            scroll_wheel_steps: 0, country_scroll_wheel_steps: 0,
        }
    }

    fn inputs() -> (Value, Value) {
        let mut raw = designer_catalog_value();
        raw["country_count"] = json!(3);
        raw["countries"] = json!([
            {"registry_index":0,"tag":"GER","designer_pointer_order_ordinal":2},
            {"registry_index":1,"tag":"ITA","designer_pointer_order_ordinal":1},
            {"registry_index":2,"tag":"USA","designer_pointer_order_ordinal":0}]);
        let base = raw["variants"][0].clone();
        let mut rows = Vec::new();
        for (index, tag) in ["GER","ITA","USA"].into_iter().enumerate() {
            let mut row = base.clone();
            row["id"] = json!(7+index); row["catalog_scan_ordinal"] = json!(index);
            row["first_ui_discovery_ordinal"] = json!(index);
            row["creator_tag"] = json!(tag); row["owner_tag"] = json!(tag); row["selected_country_tag"] = json!(tag);
            row["obsolete"] = json!(index==2);
            row["source_occurrences"] = match index {
                0 => json!([{"source_country_tag":"GER","source_country_registry_index":0,"source_list":"B","source_list_ordinal":2}]),
                1 => json!([{"source_country_tag":"GER","source_country_registry_index":0,"source_list":"A","source_list_ordinal":0},
                    {"source_country_tag":"ITA","source_country_registry_index":1,"source_list":"B","source_list_ordinal":0}]),
                _ => json!([{"source_country_tag":"ITA","source_country_registry_index":1,"source_list":"A","source_list_ordinal":0}]),
            };
            rows.push(row);
        }
        raw["variants"] = Value::Array(rows);
        let slots: Vec<_> = [800,27,99].into_iter().enumerate().map(|(i,tag)|
            json!({"physical_slot":i,"status":"accepted","tag_db_id":tag})).collect();
        let owners: Vec<_> = [800,27,99].into_iter().enumerate().map(|(i,tag)|
            json!({"owner_physical_slot":i,"owner_tag_db_id":tag,"table_row_count":3,"copied_prefix_slots":3,
                "status":"measured","cause":null,"lane2_raw":[0,5000001,5000002]})).collect();
        let rest = json!({"countries":{"GER":{"_r2_tag_db_id":800},"ITA":{"_r2_tag_db_id":27},"USA":{"_r2_tag_db_id":99}},
            "diplomacy_world":{"domain53_extra":{"intel_raw_lane2":{"schema":1,
                "census":{"country_count":3,"header_stable":true,"allocation_complete":true,"slots":slots},
                "status":"measured","cause":null,"owners":owners}}}});
        (raw, rest)
    }

    fn projected(authored: &DesignerAuthoredInputs, frame: &WorldFrame, viewer: &ObserverDesignerViewState) -> Result<ObserverDesignerCatalog,String> {
        admit(authored, frame, frame.design_catalog().unwrap().native_designer_inputs_at_hour(900), "GER", ObserverDesignerFamily::Tank, viewer)
    }

    #[test]
    fn native_ab_order_dedup_filters_and_pointer_order_use_authored_classifier_and_exact_intel() {
        let (root, _) = production_fixture();
        let renderer = admitted_production_renderer(&root.0,None);
        let authored = DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let (raw, rest) = inputs(); let frame = world_from_catalog(raw, rest);
        let mut state = viewer();
        let result = projected(&authored,&frame,&state).unwrap();
        assert_eq!(result.ordered.iter().map(|row| row.record.design_id.as_str()).collect::<Vec<_>>(), ["8:70","7:70","9:70"]);
        assert_eq!(result.country_filter_rows.iter().map(|row| row.country_tag.as_str()).collect::<Vec<_>>(), ["USA","ITA"]);
        assert_eq!(result.ordered[0].designer_window_tokens, ["light_tank_chassis_1","light_tank_chassis_1"]);
        assert!(matches!(&result.ordered[0].icon, Ok(DesignerRowIcon::Sprite(token)) if token=="GFX_xui13_test"));
        state.obsolete_filter_enabled=false;
        assert_eq!(projected(&authored,&frame,&state).unwrap().ordered.len(),2);
        state.foreign_filter_enabled=false;
        let ours=projected(&authored,&frame,&state).unwrap();
        assert_eq!(ours.ordered[0].record.design_id,"7:70"); assert_eq!(ours.ordered.len(),1);
        state.foreign_filter_enabled=true; state.our_filter_enabled=false;
        state.excluded_country_filter_tags=vec!["ITA".into()];
        assert!(projected(&authored,&frame,&state).unwrap().ordered.is_empty());
    }

    #[test]
    fn measured_intel_cannot_bypass_ledger_unknown_header_or_signed_threshold() {
        let (root, _) = production_fixture(); let renderer=admitted_production_renderer(&root.0,None);
        let authored=DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let (raw,rest)=inputs();
        for cause in [json!({"collector":{"domain_problems":[{"domain_id":53,"problem":"partial_data"}]}}),json!({"collector":null})] {
            let mut rest=rest.clone(); rest["provenance"]=cause;
            let result = projected(&authored,&world_from_catalog(raw.clone(),rest),&viewer()).unwrap();
            assert_eq!(result.ordered.iter().map(|row|row.record.design_id.as_str()).collect::<Vec<_>>(), ["8:70","7:70"]);
            assert_eq!(result.unresolved_suffix.as_ref().unwrap().design_id, "9:70");
            assert!(result.field_refusals.contains_key("country_filters"));
        }
        let mut unknown=rest.clone(); unknown["diplomacy_world"]["domain53_extra"]["intel_raw_lane2"]["census"]["header_stable"]=json!(false);
        let result = projected(&authored,&world_from_catalog(raw.clone(),unknown),&viewer()).unwrap();
        assert_eq!(result.ordered.len(), 2);
        assert!(result.unresolved_suffix.as_ref().unwrap().reason.contains("Census"));
        let mut bounded=rest; bounded["diplomacy_world"]["domain53_extra"]["intel_raw_lane2"]["owners"][0]["lane2_raw"]=json!([0,5000000,i64::MIN]);
        let rows=projected(&authored,&world_from_catalog(raw,bounded),&viewer()).unwrap();
        assert_eq!(rows.ordered.iter().map(|row|row.record.design_id.as_str()).collect::<Vec<_>>(),["8:70","7:70"]);
    }

    #[test]
    fn full_byte_nonzero_and_runtime_feature_gate_remain_independent_of_authored_dlc_names() {
        let (root,_) = production_fixture(); let renderer=admitted_production_renderer(&root.0,None);
        let authored=DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let (raw,rest)=inputs();
        for byte in [1,128,255] {
            let mut current=raw.clone(); current["variants"][0]["definition"]["serialized_variant_flag_2b9d"]["raw_u8"]=json!(byte);
            let result=projected(&authored,&world_from_catalog(current,rest.clone()),&viewer()).unwrap();
            assert!(result.ordered.iter().all(|row|row.record.design_id!="7:70"));
        }
        for mask in [0u32,1,1<<4,1<<8,u32::MAX & !(1<<7)] {
            let mut current=raw.clone(); current["runtime_dlc_mask"]["raw_u32"]=json!(mask);
            let result=projected(&authored,&world_from_catalog(current,rest.clone()),&viewer()).unwrap();
            assert!(result.ordered.is_empty(),"{mask}");
        }
        let mut unknown=raw; unknown["runtime_dlc_mask"]=json!({"status":"unavailable","reason":{"kind":"unreadable","field":"runtime_dlc_mask"}});
        let result=projected(&authored,&world_from_catalog(unknown,rest),&viewer()).unwrap();
        assert!(result.ordered.is_empty());
        assert!(result.unresolved_suffix.unwrap().reason.contains("runtime_dlc_mask"));
    }

    fn owner_prefix_inputs() -> (Value, Value) {
        let (mut raw, mut rest) = inputs();
        for (index, tag) in ["GER", "ITA", "USA"].into_iter().enumerate() {
            raw["variants"][index]["source_occurrences"] = json!([{
                "source_country_tag":tag,"source_country_registry_index":index,"source_list":"A","source_list_ordinal":0}]);
        }
        rest["countries"]["GER"]["cosmetic_tag"] = Value::Null;
        rest["countries"]["GER"]["politics"] = json!({"ruling_party":"fascism"});
        (raw, rest)
    }

    #[test]
    fn known_owner_prefix_is_painted_and_foreign_unknown_suffix_never_compacted() {
        let (root,_) = production_fixture(); let renderer=admitted_production_renderer(&root.0,None);
        let authored=DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let (raw, mut rest)=owner_prefix_inputs();
        rest["provenance"] = json!({"collector":{"domain_problems":[{"domain_id":53,"problem":"partial_data"}]}});
        let frame=world_from_catalog(raw.clone(),rest.clone());
        let mut state=viewer();
        let result=projected(&authored,&frame,&state).unwrap();
        assert_eq!(result.ordered.iter().map(|row|row.record.design_id.as_str()).collect::<Vec<_>>(),["7:70"]);
        let suffix=result.unresolved_suffix.unwrap();
        assert_eq!((suffix.source_country_registry_index,suffix.source_list.as_str(),suffix.source_list_ordinal,suffix.design_id.as_str()),(1,"A",0,"8:70"));
        assert!(suffix.reason.contains("ITA"));
        assert!(result.field_refusals.contains_key("country_filters"));
        let partial=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&state,160,120).unwrap();
        assert_eq!(partial.visible_design_ids,["7:70"]);
        assert!(!partial.membership_complete); assert!(!partial.country_filters_complete);
        assert!(partial.field_refusals.contains_key("row_suffix"));
        assert!(!partial.field_refusals.contains_key("row_scroll"));
        let mut baseline=raw.clone(); baseline["variants"].as_array_mut().unwrap().truncate(1);
        let baseline=world_from_catalog(baseline,rest.clone());
        let complete=renderer.compose_xui13_designer_surface(&baseline,baseline.named_country("GER").unwrap(),&state,160,120).unwrap();
        assert!(complete.membership_complete); assert!(complete.country_filters_complete);
        assert_eq!(partial.output.image,complete.output.image,"known prefix pixels cannot depend on unknown suffix size");
        state.scroll_wheel_steps=1;
        let scrolled=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&state,160,120).unwrap();
        assert!(scrolled.visible_design_ids.is_empty());
        assert_eq!(scrolled.ordered_design_ids,["7:70"]);
        assert!(scrolled.field_refusals.contains_key("row_scroll"));
        assert!(scrolled.output.report.drawn_elements>0,"known panel chrome remains visible");
        // Move the same unknown source before the own row: no row may jump into its ordinal.
        let mut before=raw;
        for occurrence in before["variants"][0]["source_occurrences"].as_array_mut().unwrap() {
            occurrence["source_country_tag"]=json!("USA"); occurrence["source_country_registry_index"]=json!(2);
            occurrence["source_list_ordinal"]=json!(1);
        }
        let first_unknown=projected(&authored,&world_from_catalog(before,rest),&viewer()).unwrap();
        assert!(first_unknown.ordered.is_empty());
        assert_eq!(first_unknown.unresolved_suffix.unwrap().design_id,"8:70");
    }

    #[test]
    fn classification_failure_stops_at_its_source_boundary_without_blanking_prior_rows() {
        let (root,_) = production_fixture(); let renderer=admitted_production_renderer(&root.0,None);
        let authored=DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let (mut raw,rest)=owner_prefix_inputs();
        raw["variants"][1]["definition"]["archetype_token"]=json!("unbound");
        raw["variants"][1]["definition"]["archetype_token_utf8_hex"]=json!("756e626f756e64");
        let result=projected(&authored,&world_from_catalog(raw.clone(),rest.clone()),&viewer()).unwrap();
        assert_eq!(result.ordered.iter().map(|row|row.record.design_id.as_str()).collect::<Vec<_>>(),["7:70"]);
        assert_eq!(result.unresolved_suffix.unwrap().design_id,"8:70");
        raw["variants"][0]["definition"]["archetype_token"]=json!("unbound");
        raw["variants"][0]["definition"]["archetype_token_utf8_hex"]=json!("756e626f756e64");
        let frame=world_from_catalog(raw,rest);
        let result=projected(&authored,&frame,&viewer()).unwrap();
        assert!(result.ordered.is_empty()); assert!(result.unresolved_suffix.is_some());
        assert!(result.field_refusals.contains_key("anchor_design"));
        let surface=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        assert!(surface.visible_design_ids.is_empty()); assert!(!surface.membership_complete);
        assert!(surface.field_refusals.contains_key("row_suffix"));
    }

    #[test]
    fn icon_header_active_and_empty_do_not_trigger_dot_sentinel_fallback() {
        for (kind,value,expected) in [(0,"",DesignerRowIcon::Sprite("".into())),(0,".",DesignerRowIcon::Sprite(".".into())),
            (1,"",DesignerRowIcon::Texture("".into())),(1,"gfx/custom.dds",DesignerRowIcon::Texture("gfx/custom.dds".into()))] {
            for active in [false,true] {
                let icon=DesignItemIconObservation::Measured { active, kind, token:DesignTextObservation::Measured {
                    value:value.into(),utf8_hex:value.as_bytes().iter().map(|b|format!("{b:02x}")).collect() } };
                assert_eq!(item_icon(&icon),Ok(expected.clone()));
            }
        }
        let sentinel=DesignItemIconObservation::Measured { active:false,kind:1,token:DesignTextObservation::Measured { value:".".into(),utf8_hex:"2e".into() } };
        assert!(item_icon(&sentinel).unwrap_err().contains("item-database"));
    }
}
