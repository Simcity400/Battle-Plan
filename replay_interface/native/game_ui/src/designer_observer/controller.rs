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
                record: variant.record.clone(), designer_window_tokens: windows, flag_country_tag: selected_country.into(),
                icon: item_icon_with_fallback(&variant.item_icon_override, ||
                    authored.fallback_icon(&equipment, frame, &native.runtime_dlc_mask, variant)),
                missions: mission_mask(&equipment, &native.runtime_dlc_mask, variant),
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


fn mission_mask(equipment: &map_viewer::authored_equipment::AuthoredEquipmentCatalog,
    runtime: &replay_core::design_catalog::DesignU32Observation,
    variant: &replay_core::design_catalog::NativeDesignerVariantInputs,
) -> Result<u32, String> {
    if !equipment.designer_classification(&variant.equipment_definition_token).map_err(|error|error.to_string())?.plane {
        // 140bc1190 initializes +410; 140bcff10 leaves non-aircraft storage unchanged.
        return Ok(0);
    }
    if variant.record.module_completeness != replay_core::FieldCompleteness::Complete {
        return Err("XUI13 mission modules were not completely observed".into());
    }
    let authored = equipment.designer_mission_masks(&variant.equipment_definition_token).map_err(|error|error.to_string())?;
    let mut allowed = authored.allowed;
    for module in &variant.record.modules {
        allowed |= equipment.module_mission_mask(&module.module_token).map_err(|error|error.to_string())?;
    }
    let mut mask = allowed & !authored.forbidden;
    if mask & 0x7200 == 0 { return Ok(mask); }
    let raw = match runtime {
        replay_core::design_catalog::DesignU32Observation::Measured {raw_u32} => *raw_u32,
        replay_core::design_catalog::DesignU32Observation::Unavailable {reason} =>
            return Err(format!("XUI13 mission runtime_dlc_mask: {reason:?}")),
    };
    for (bit, stat) in [(0x1000,0x23),(0x2000,0x24)] {
        if mask & bit == 0 { continue; }
        if raw & (1 << 4) == 0 { mask &= !bit; continue; }
        let operand = variant.mission_stat_operands.iter().find(|row|row.group_key==bit && row.stat_key==stat)
            .ok_or_else(||format!("XUI13 mission operand {bit:#x}/{stat:#x} is absent"))?;
        if operand.effective_raw_i64().map_err(|reason|format!("XUI13 mission operand {bit:#x}/{stat:#x}: {reason:?}"))? <= 0 {
            mask &= !bit;
        }
    }
    if raw & (1 << 5) == 0 { mask &= !0x4000; }
    if raw & (1 << 7) == 0 { mask &= !0x200; }
    Ok(mask)
}

#[cfg(test)]
fn item_icon(icon: &DesignItemIconObservation) -> Result<DesignerRowIcon, String> {
    item_icon_with_fallback(icon, || Err("XUI13 item-database fixture has no provider".into()))
}

fn item_icon_with_fallback(icon: &DesignItemIconObservation,
    fallback: impl FnOnce() -> Result<DesignerRowIcon, String>,
) -> Result<DesignerRowIcon, String> {
    match icon {
        DesignItemIconObservation::Measured { active: false, kind: 0, .. } => Ok(DesignerRowIcon::Authored),
        DesignItemIconObservation::Measured { active, kind, token: DesignTextObservation::Measured { value, .. } } => {
            match (*kind, value.as_str()) {
                (1, ".") => fallback(),
                (1, _) if !active => Ok(DesignerRowIcon::Authored),
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
    use super::super::tests::{admitted_production_renderer, admitted_production_renderer_with_assets, designer_catalog_value, production_fixture, world_from_catalog};
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
    fn actual_sentinel_fallback_draws_later_family_pool_and_preserves_unknown_resource_refusal() {
        let (root, _) = production_fixture();
        std::fs::create_dir_all(root.0.join("gfx/interface/equipmentdesigner/graphic_db")).unwrap();
        std::fs::create_dir_all(root.0.join("common/country_tags")).unwrap();
        std::fs::create_dir_all(root.0.join("map")).unwrap();
        std::fs::write(root.0.join("common/country_tags/tags.txt"), br#"GER = "countries/Germany.txt""#).unwrap();
        std::fs::write(root.0.join("map/default.map"), br#"continent = "continent.txt" definitions = "definition.csv" provinces = "provinces.bmp""#).unwrap();
        std::fs::write(root.0.join("map/continent.txt"), b"continents = { europe }").unwrap();
        std::fs::write(root.0.join("common/units/equipment/designer.txt"), b"equipments = { family_source = { is_archetype = yes type = armor } light_tank_chassis_1 = { is_archetype = yes type = armor module_slots = { main = {} } family = family_source } }").unwrap();
        let mut texture = std::fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap();
        for pixel in texture[18..].chunks_exact_mut(4) { pixel.copy_from_slice(&[0,255,0,255]); }
        std::fs::write(root.0.join("gfx/interface/fallback_green.tga"), texture).unwrap();
        std::fs::write(root.0.join("gfx/interface/equipmentdesigner/graphic_db/default.txt"),
            b"default = { light_tank_chassis_1 = { pool = { icons = { GFX_xui13_test } weight = 1 } } family_source = { pool = { icons = { gfx/interface/fallback_green.tga } weight = 2 } pool = { icons = { GFX_witnessed_missing } weight = 3 } } }").unwrap();
        std::fs::create_dir_all(root.0.join("history/states")).unwrap();
        std::fs::write(root.0.join("history/states/capital.txt"), b"state={id=17 provinces={1}}").unwrap();
        std::fs::write(root.0.join("map/definition.csv"), b"0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n").unwrap();
        let mut province_bmp=vec![0u8;66];
        province_bmp[..2].copy_from_slice(b"BM");
        province_bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        province_bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        province_bmp[18..22].copy_from_slice(&4i32.to_le_bytes());
        province_bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
        province_bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        province_bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        for pixel in province_bmp[54..].chunks_exact_mut(3) { pixel.copy_from_slice(&[0,0,1]); }
        std::fs::write(root.0.join("map/provinces.bmp"),province_bmp).unwrap();
        let mut blue=std::fs::read(root.0.join("gfx/interface/fallback_green.tga")).unwrap();
        for pixel in blue[18..].chunks_exact_mut(4) { pixel.copy_from_slice(&[255,0,0,255]); }
        std::fs::write(root.0.join("gfx/interface/capital_blue.tga"),blue).unwrap();
        std::fs::write(root.0.join("gfx/interface/equipmentdesigner/graphic_db/region.txt"),
            b"europe={light_tank_chassis_1={pool={icons={gfx/interface/capital_blue.tga} weight=20}}}").unwrap();
        let renderer = admitted_production_renderer(&root.0, None);
        let omitted = admitted_production_renderer_with_assets(&root.0, Some("gfx/interface/fallback_green.tga"), &[]);
        let (mut raw, mut rest) = owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        rest["countries"]["GER"]["politics"] = json!({"capital_override_raw":-1,"capital_state":null,
            "capital_slot":{"schema":1,"game_hour_raw":43800899,"observation_ordinal":11,"tag_db_id":800,
                "operands":{"override_raw":-1,"base_raw":null},"outcome":{"kind":"non_positive"}}});
        rest["diplomacy_world"] = json!({"domain53_extra":{"faction_member_order_schema":1,"faction_roster_status":"measured_empty","faction_rows":[]}});
        let original = world_from_catalog(raw.clone(), rest.clone());
        raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":false,"kind":1,
            "token":{"status":"measured","value":".","utf8_hex":"2e"}});
        let fallback = world_from_catalog(raw.clone(), rest.clone());
        let mut positive_capital = rest.clone();
        positive_capital["countries"]["GER"]["politics"]["capital_override_raw"] = json!(1);
        let unknown_state_slot = world_from_catalog(raw.clone(), positive_capital.clone());
        positive_capital["countries"]["GER"]["politics"]["capital_slot"]=json!({"schema":1,
            "game_hour_raw":43800899,"observation_ordinal":55,"tag_db_id":800,
            "operands":{"override_raw":1,"base_raw":null},
            "outcome":{"kind":"present","slot_count":2,"state_id":73,"template_id":17}});
        let present_state_slot = world_from_catalog(raw.clone(), positive_capital.clone());
        positive_capital["countries"]["GER"]["politics"]["capital_slot"]["outcome"]["template_id"]=json!(73);
        let missing_template = world_from_catalog(raw.clone(), positive_capital);
        let mut blue_raw=raw.clone();let blue_token="gfx/interface/capital_blue.tga";
        blue_raw["variants"][0]["definition"]["item_icon_override"]=json!({"status":"measured","active":true,"kind":1,
            "token":{"status":"measured","value":blue_token,"utf8_hex":blue_token.as_bytes().iter().map(|byte|format!("{byte:02x}")).collect::<String>()}});
        let direct_blue=world_from_catalog(blue_raw,rest.clone());
        let token = "gfx/interface/fallback_green.tga";
        raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":true,"kind":1,
            "token":{"status":"measured","value":token,"utf8_hex":token.as_bytes().iter().map(|byte|format!("{byte:02x}")).collect::<String>()}});
        let direct = world_from_catalog(raw, rest);
        let compose = |renderer: &crate::production_renderer::ProductionUiRenderer, frame: &WorldFrame| {
            renderer.compose_xui13_designer_surface(frame, frame.named_country("GER").unwrap(), &viewer(), 160, 120).unwrap()
        };
        let base = compose(&renderer, &original);
        let actual = compose(&renderer, &fallback);
        let expected = compose(&renderer, &direct);
        assert_eq!(actual.visible_design_ids,["7:70"]);
        assert!(!actual.field_refusals.contains_key("7:70/equipment_icon"), "{:?}",actual.field_refusals);
        assert_eq!(row_element_pixels(&actual,"equipment"),row_element_pixels(&expected,"equipment"));
        assert_ne!(row_element_pixels(&actual,"equipment"),row_element_pixels(&base,"equipment"));
        assert_eq!(row_element_pixels(&actual,"creator_flag"),row_element_pixels(&base,"creator_flag"));
        let unknown_later_pool = compose(&renderer, &unknown_state_slot);
        assert_eq!(unknown_later_pool.visible_design_ids, actual.visible_design_ids);
        assert!(unknown_later_pool.field_refusals["7:70/equipment_icon"].contains("runtime GS state-slot"));
        assert_eq!(row_element_pixels(&unknown_later_pool,"creator_flag"),row_element_pixels(&actual,"creator_flag"));
        let regional=compose(&renderer,&present_state_slot);
        assert!(!regional.field_refusals.contains_key("7:70/equipment_icon"),"{:?}",regional.field_refusals);
        assert_eq!(row_element_pixels(&regional,"equipment"),row_element_pixels(&compose(&renderer,&direct_blue),"equipment"));
        assert_ne!(row_element_pixels(&regional,"equipment"),row_element_pixels(&actual,"equipment"));
        assert_eq!(row_element_pixels(&regional,"creator_flag"),row_element_pixels(&actual,"creator_flag"));
        let missing=compose(&renderer,&missing_template);
        assert_eq!(missing.visible_design_ids,actual.visible_design_ids);
        assert!(missing.field_refusals["7:70/equipment_icon"].contains("template 73"));
        let unavailable = compose(&omitted, &fallback);
        assert_eq!(unavailable.visible_design_ids,actual.visible_design_ids);
        assert!(unavailable.membership_complete);
        assert!(unavailable.field_refusals["7:70/equipment_icon"].contains("capability"));
        assert_eq!(row_element_pixels(&unavailable,"creator_flag"),row_element_pixels(&actual,"creator_flag"));
    }

    #[test]
    fn authenticated_quoted_at_texture_and_variable_keep_pixels_and_bare_refusal_local() {
        let (root, _) = production_fixture();
        for directory in ["gfx/interface/equipmentdesigner/graphic_db", "common/country_tags", "map"] {
            std::fs::create_dir_all(root.0.join(directory)).unwrap();
        }
        std::fs::write(root.0.join("common/country_tags/tags.txt"), br#"GER="countries/Germany.txt""#).unwrap();
        std::fs::write(root.0.join("map/default.map"), br#"continent="continent.txt" definitions="definition.csv" provinces="provinces.bmp""#).unwrap();
        std::fs::write(root.0.join("map/continent.txt"), b"continents={europe}").unwrap();
        let path = "@Literal.TGA";
        let rgba = [40, 180, 90, 255];
        let mut texture = std::fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap()[..18].to_vec();
        texture[12..14].copy_from_slice(&8u16.to_le_bytes());
        texture[14..16].copy_from_slice(&8u16.to_le_bytes());
        for _ in 0..64 { texture.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]); }
        std::fs::write(root.0.join(path), texture).unwrap();
        let (mut raw, mut rest) = owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        rest["countries"]["GER"]["cosmetic_tag"] = Value::Null;
        rest["countries"]["GER"]["politics"] = json!({"ruling_party":"neutrality","capital_override_raw":-1,"capital_state":null,
            "capital_slot":{"schema":1,"game_hour_raw":43800899,"observation_ordinal":11,"tag_db_id":800,
                "operands":{"override_raw":-1,"base_raw":null},"outcome":{"kind":"non_positive"}}});
        rest["diplomacy_world"] = json!({"domain53_extra":{"faction_member_order_schema":1,"faction_roster_status":"measured_empty","faction_rows":[]}});
        raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":false,"kind":1,
            "token":{"status":"measured","value":".","utf8_hex":"2e"}});
        let frame = world_from_catalog(raw, rest);
        let mut surfaces = Vec::new();
        for (definitions, token) in [
            ("", "\"@Literal.TGA\""),
            ("@chosen=\"@Literal.TGA\"", "@chosen"),
            ("", "@missing"),
        ] {
            let source = format!(r#"{definitions} default={{ light_tank_chassis_1={{pool={{icons={{{token}}}}}}} unrelated_type={{pool={{"icons"={{GFX_Refused}}}}}} }}"#);
            std::fs::write(root.0.join("gfx/interface/equipmentdesigner/graphic_db/default.txt"), source).unwrap();
            let renderer = admitted_production_renderer_with_assets(&root.0, None, &[path]);
            let decoded = renderer.resolver().decode_texture(path).unwrap().image;
            assert_eq!((decoded.width, decoded.height), (8, 8));
            assert!(decoded.pixels.chunks_exact(4).all(|pixel| pixel == rgba));
            let surface = renderer.compose_xui13_designer_surface(&frame, frame.named_country("GER").unwrap(), &viewer(), 160, 120).unwrap();
            assert_eq!(surface.ordered_design_ids, ["7:70"]);
            assert_eq!(surface.visible_design_ids, ["7:70"]);
            assert!(surface.membership_complete && surface.country_filters_complete);
            surfaces.push(surface);
        }
        for known in &surfaces[..2] {
            assert!(known.field_refusals.is_empty(), "{:?}", known.field_refusals);
            let pixels = row_element_pixels(known, "equipment");
            assert_eq!(pixels.len(), 8 * 8 * 4);
            assert!(pixels.chunks_exact(4).all(|pixel| pixel == rgba));
        }
        assert_eq!(surfaces[0].output.image, surfaces[1].output.image);
        let refused = &surfaces[2];
        assert_eq!(refused.field_refusals.keys().map(String::as_str).collect::<Vec<_>>(), ["7:70/equipment_icon"]);
        assert!(refused.field_refusals["7:70/equipment_icon"].contains("@missing"));
        assert!(refused.field_refusals["7:70/equipment_icon"].contains("does not define"));
        let rect = surfaces[0].output.debug_rects.iter().find(|rect|
            rect.name.contains("7:70") && rect.name.ends_with("::equipment")).unwrap();
        let expected = &surfaces[0].output.image;
        let actual = &refused.output.image;
        assert_eq!((actual.width, actual.height), (expected.width, expected.height));
        assert_ne!(actual, expected);
        for (offset, (pixel, known)) in actual.pixels.chunks_exact(4).zip(expected.pixels.chunks_exact(4)).enumerate() {
            let (x, y) = (offset % actual.width, offset / actual.width);
            let equipment = (rect.x as usize..(rect.x + rect.w) as usize).contains(&x)
                && (rect.y as usize..(rect.y + rect.h) as usize).contains(&y);
            if equipment { assert_ne!(pixel, rgba); }
            else { assert_eq!(pixel, known, "known row/flag/chrome pixel {x},{y}"); }
        }
    }

    #[test]
    fn authenticated_ascii_icon_keywords_keep_pixels_and_last_weight_assignment() {
        let (root, _) = production_fixture();
        std::fs::create_dir_all(root.0.join("gfx/interface/equipmentdesigner/graphic_db")).unwrap();
        std::fs::create_dir_all(root.0.join("common/country_tags")).unwrap();
        std::fs::create_dir_all(root.0.join("map")).unwrap();
        std::fs::write(root.0.join("common/country_tags/tags.txt"), br#"GER = "countries/Germany.txt""#).unwrap();
        std::fs::write(root.0.join("map/default.map"), br#"continent = "continent.txt" definitions = "definition.csv" provinces = "provinces.bmp""#).unwrap();
        std::fs::write(root.0.join("map/continent.txt"), b"continents = { europe }").unwrap();
        let mut texture = std::fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap();
        for pixel in texture[18..].chunks_exact_mut(4) { pixel.copy_from_slice(&[0,255,0,255]); }
        let green = "gfx/interface/Keyword_Green.tga";
        std::fs::write(root.0.join(green), texture).unwrap();
        let (mut raw, mut rest) = owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        rest["countries"]["GER"]["politics"] = json!({"capital_override_raw":-1,"capital_state":null,
            "capital_slot":{"schema":1,"game_hour_raw":43800899,"observation_ordinal":11,"tag_db_id":800,
                "operands":{"override_raw":-1,"base_raw":null},"outcome":{"kind":"non_positive"}}});
        rest["diplomacy_world"] = json!({"domain53_extra":{"faction_member_order_schema":1,"faction_roster_status":"measured_empty","faction_rows":[]}});
        raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":false,"kind":1,
            "token":{"status":"measured","value":".","utf8_hex":"2e"}});
        let frame = world_from_catalog(raw.clone(), rest.clone());
        let lower = "default = { light_tank_chassis_1 = { pool = { icons = { gfx/interface/Keyword_Green.tga } weight = 99 icons_weight = 9 icons_weight = 2 models_weight = 100 } pool = { icons = { GFX_xui13_test } weight = 3 } } }";
        let mixed = "DeFaUlT = { light_tank_chassis_1 = { PoOl = { IcOnS = { gfx/interface/Keyword_Green.tga } WeIgHt = 99 IcOnS_WeIgHt = 9 ICONS_WEIGHT = 2 MoDeLs_WeIgHt = 100 } POOL = { ICONS = { GFX_xui13_test } WEIGHT = 3 } } }";
        let reversed = mixed.replace("IcOnS_WeIgHt = 9 ICONS_WEIGHT = 2", "IcOnS_WeIgHt = 2 ICONS_WEIGHT = 9");
        let compose = |renderer: &crate::production_renderer::ProductionUiRenderer, frame: &WorldFrame| {
            renderer.compose_xui13_designer_surface(frame, frame.named_country("GER").unwrap(), &viewer(), 160, 120).unwrap()
        };
        let mut surfaces = Vec::new();
        for source in [lower, mixed, reversed.as_str()] {
            std::fs::write(root.0.join("gfx/interface/equipmentdesigner/graphic_db/default.txt"), source).unwrap();
            let renderer = admitted_production_renderer_with_assets(&root.0, None, &[green]);
            let surface = compose(&renderer, &frame);
            assert_eq!(surface.visible_design_ids, ["7:70"]);
            assert!(surface.membership_complete);
            assert!(!surface.field_refusals.contains_key("7:70/equipment_icon"), "{:?}", surface.field_refusals);
            surfaces.push(surface);
        }
        assert_eq!(surfaces[0].output.image.pixels, surfaces[1].output.image.pixels);
        assert_eq!(surfaces[0].field_refusals, surfaces[1].field_refusals);
        assert_ne!(row_element_pixels(&surfaces[1], "equipment"), row_element_pixels(&surfaces[2], "equipment"));
        assert_eq!(row_element_pixels(&surfaces[1], "creator_flag"), row_element_pixels(&surfaces[2], "creator_flag"));
        let renderer = admitted_production_renderer_with_assets(&root.0, None, &[green]);
        for (surface, kind, token) in [(&surfaces[0], 0, "GFX_xui13_test"), (&surfaces[2], 1, green)] {
            raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":true,"kind":kind,
                "token":{"status":"measured","value":token,"utf8_hex":token.as_bytes().iter().map(|byte|format!("{byte:02x}")).collect::<String>()}});
            let direct = compose(&renderer, &world_from_catalog(raw.clone(), rest.clone()));
            assert_eq!(row_element_pixels(surface, "equipment"), row_element_pixels(&direct, "equipment"));
        }
    }

    #[test]
    fn authenticated_quoted_pool_and_fields_refuse_only_icon_and_retain_known_row() {
        let (root, _) = production_fixture();
        std::fs::create_dir_all(root.0.join("gfx/interface/equipmentdesigner/graphic_db")).unwrap();
        std::fs::create_dir_all(root.0.join("common/country_tags")).unwrap();
        std::fs::create_dir_all(root.0.join("map")).unwrap();
        std::fs::write(root.0.join("common/country_tags/tags.txt"), br#"GER = "countries/Germany.txt""#).unwrap();
        std::fs::write(root.0.join("map/default.map"), br#"continent = "continent.txt" definitions = "definition.csv" provinces = "provinces.bmp""#).unwrap();
        std::fs::write(root.0.join("map/continent.txt"), b"continents = { europe }").unwrap();
        let green = "gfx/interface/Quoted_Green.tga";
        let mut texture = std::fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap();
        for pixel in texture[18..].chunks_exact_mut(4) { pixel.copy_from_slice(&[0,255,0,255]); }
        std::fs::write(root.0.join(green), texture).unwrap();
        let (mut raw, mut rest) = owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        rest["countries"]["GER"]["politics"] = json!({"capital_override_raw":-1,"capital_state":null,
            "capital_slot":{"schema":1,"game_hour_raw":43800899,"observation_ordinal":11,"tag_db_id":800,
                "operands":{"override_raw":-1,"base_raw":null},"outcome":{"kind":"non_positive"}}});
        rest["diplomacy_world"] = json!({"domain53_extra":{"faction_member_order_schema":1,"faction_roster_status":"measured_empty","faction_rows":[]}});
        raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":false,"kind":1,
            "token":{"status":"measured","value":".","utf8_hex":"2e"}});
        let frame = world_from_catalog(raw.clone(), rest.clone());
        let mut surfaces = Vec::new();
        for (pool_key, icons_key, weight_key) in [
            ("PoOl", "IcOnS", "WeIgHt"), ("\"pool\"", "icons", "weight"),
            ("pool", "\"icons\"", "weight"), ("pool", "icons", "\"weight\""),
        ] {
            let source = format!("default = {{ light_tank_chassis_1 = {{ {pool_key} = {{ {weight_key} = 1 {icons_key} = {{ {green} }} }} }} }}");
            std::fs::write(root.0.join("gfx/interface/equipmentdesigner/graphic_db/default.txt"), source).unwrap();
            let renderer = admitted_production_renderer_with_assets(&root.0, None, &[green]);
            surfaces.push(renderer.compose_xui13_designer_surface(&frame, frame.named_country("GER").unwrap(), &viewer(), 160, 120).unwrap());
        }
        let known = &surfaces[0];
        assert!(!known.field_refusals.contains_key("7:70/equipment_icon"), "{:?}", known.field_refusals);
        for (refused, reason) in surfaces[1..].iter().zip(["quoted pool keyword", "quoted field keyword", "quoted field keyword"]) {
            assert!(refused.field_refusals["7:70/equipment_icon"].contains(reason));
            assert!(refused.membership_complete);
            assert_eq!(refused.ordered_design_ids, ["7:70"]);
            assert_eq!(refused.visible_design_ids, known.visible_design_ids);
            assert_eq!(row_element_pixels(refused, "creator_flag"), row_element_pixels(known, "creator_flag"));
            assert!(!refused.output.debug_rects.iter().any(|rect| rect.name.contains("7:70") && rect.name.ends_with("::equipment")));
        }
        let renderer = admitted_production_renderer_with_assets(&root.0, None, &[green]);
        raw["variants"][0]["definition"]["item_icon_override"] = json!({"status":"measured","active":true,"kind":1,
            "token":{"status":"measured","value":green,"utf8_hex":green.as_bytes().iter().map(|byte|format!("{byte:02x}")).collect::<String>()}});
        let direct = world_from_catalog(raw, rest);
        let expected = renderer.compose_xui13_designer_surface(&direct, direct.named_country("GER").unwrap(), &viewer(), 160, 120).unwrap();
        assert_eq!(row_element_pixels(known, "equipment"), row_element_pixels(&expected, "equipment"));
    }

    #[test]
    fn missions_use_recorded_signed_operands_and_independent_runtime_feature_bits() {
        use replay_core::design_catalog::{DesignI64Observation, DesignMissionOverrideObservation, DesignFieldRefusal, DesignU32Observation};
        let (root,_) = production_fixture();
        std::fs::write(root.0.join("common/units/equipment/designer.txt"),
            b"equipments = { light_tank_chassis_1 = { is_archetype = yes type = fighter module_slots = { main = {} } allow_mission_type = { air_superiority naval_mines_planting naval_mines_sweeping recon attack_logistics } } }").unwrap();
        let renderer=admitted_production_renderer(&root.0,None);
        let authored=DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let frame=world_from_catalog(designer_catalog_value(),json!({}));
        let equipment=authored.for_frame(&frame).unwrap();
        let mut native=frame.design_catalog().unwrap().native_designer_inputs_at_hour(900).unwrap();
        let variant=&mut native.variants[0];
        let full=DesignU32Observation::Measured { raw_u32:(1<<4)|(1<<5)|(1<<7) };
        variant.mission_stat_operands[0].fallback=DesignI64Observation::Measured {raw_i64:i64::MIN};
        variant.mission_stat_operands[1].fallback=DesignI64Observation::Measured {raw_i64:i64::MAX};
        assert_eq!(mission_mask(&equipment,&full,variant).unwrap(),0x6201);
        variant.mission_stat_operands[0].fallback=DesignI64Observation::Unavailable {reason:DesignFieldRefusal {kind:"unreadable".into(),field:"fallback".into()}};
        variant.mission_stat_operands[0].override_lookup=DesignMissionOverrideObservation::Measured {
            raw_i64:1,group_count:2,group_key_prefix:vec![4096],stat_count:1,stat_key_prefix:vec![35] };
        assert_eq!(mission_mask(&equipment,&full,variant).unwrap(),0x7201);
        variant.mission_stat_operands[0].override_lookup=DesignMissionOverrideObservation::Unavailable {reason:DesignFieldRefusal {kind:"unreadable".into(),field:"override".into()}};
        assert!(mission_mask(&equipment,&full,variant).unwrap_err().contains("override"));
        assert_eq!(mission_mask(&equipment,&DesignU32Observation::Measured {raw_u32:0},variant).unwrap(),1);
        assert_eq!(mission_mask(&equipment,&DesignU32Observation::Measured {raw_u32:(1<<5)|(1<<7)},variant).unwrap(),0x4201);
        variant.record.module_completeness=replay_core::FieldCompleteness::Unknown;
        assert!(mission_mask(&equipment,&full,variant).unwrap_err().contains("modules"));
    }

    #[test]
    fn unknown_mission_operand_hides_only_missions_and_keeps_known_row_pixels() {
        let (root,_) = production_fixture();
        std::fs::write(root.0.join("common/units/equipment/designer.txt"),
            b"equipments = { light_tank_chassis_1 = { is_archetype = yes type = fighter module_slots = { main = {} } allow_mission_type = naval_mines_planting } }").unwrap();
        let renderer=admitted_production_renderer(&root.0,None);
        let mut raw=designer_catalog_value();raw["runtime_dlc_mask"]["raw_u32"]=json!((1<<8)|(1<<4));
        let rest=json!({"countries":{"GER":{"cosmetic_tag":null,"politics":{"ruling_party":"fascism"}}}});
        let known=world_from_catalog(raw.clone(),rest.clone());
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]=json!({"status":"unavailable","reason":{"kind":"unreadable","field":"mission_group"}});
        let unknown=world_from_catalog(raw,rest);
        let mut state=viewer();state.family=crate::readonly_design_peace::DesignerViewKind::Plane;state.anchor_catalogue_family=state.family;
        let complete=renderer.compose_xui13_designer_surface(&known,known.named_country("GER").unwrap(),&state,160,120).unwrap();
        let partial=renderer.compose_xui13_designer_surface(&unknown,unknown.named_country("GER").unwrap(),&state,160,120).unwrap();
        assert_eq!(partial.ordered_design_ids,["7:70"]);
        assert_eq!(partial.visible_design_ids,["7:70"]);
        assert!(partial.membership_complete);
        assert!(partial.field_refusals["7:70/missions"].contains("mission_group"));
        assert!(complete.field_refusals.is_empty());
        assert_eq!(row_element_pixels(&partial,"equipment"),row_element_pixels(&complete,"equipment"));
        assert_eq!(row_element_pixels(&partial,"creator_flag"),row_element_pixels(&complete,"creator_flag"));
        assert!(partial.output.report.drawn_elements>0);
    }

    #[test]
    fn mission_grid_paints_more_than_six_frames_with_authored_wrap_and_capacity() {
        for (horizontal,limit,expected) in [(true,3,7usize),(false,3,7),(true,2,6)] {
            let (root,_) = production_fixture();
            std::fs::write(root.0.join("common/units/equipment/designer.txt"),
                b"equipments = { light_tank_chassis_1 = { is_archetype = yes type = fighter module_slots = { main = {} } allow_mission_type = { air_superiority cas interception strategic_bomber naval_bomber drop_nuke paradrop training } } }").unwrap();
            let path=root.0.join("interface/designer.gui");
            let gui=std::fs::read_to_string(&path).unwrap()
                .replace("width = 40 height = 24", "width = 40 height = 48")
                .replace("width = 40 height = 10", "width = 40 height = 32")
                .replace("max_slots_horizontal = 6 add_horizontal = yes",&format!("max_slots_horizontal = 3 max_slots_vertical = {limit} add_horizontal = {}",if horizontal {"yes"} else {"no"}))
                .replace("name = \"mission_icon\" size = { width = 8 height = 8 } spriteType = \"GFX_xui13_test\"",
                    "name = \"mission_icon\" size = { width = 8 height = 8 } spriteType = \"GFX_mission_frames\"");
            std::fs::write(path,gui).unwrap();
            let path=root.0.join("interface/designer.gfx");
            let gfx=std::fs::read_to_string(&path).unwrap()+"\nspriteTypes = { spriteType = { name = GFX_mission_frames texturefile = gfx/interface/missions.tga noOfFrames = 18 } }";
            std::fs::write(path,gfx).unwrap();
            let mut tga=vec![0,0,2,0,0,0,0,0,0,0,0,0,18,0,1,0,32,0x20];
            for bit in 0..18u8 {tga.extend_from_slice(&[40+bit,80+bit,120+bit,255]);}
            std::fs::write(root.0.join("gfx/interface/missions.tga"),tga).unwrap();
            let renderer=admitted_production_renderer_with_assets(&root.0,None,&["gfx/interface/missions.tga"]);
            let mut raw=designer_catalog_value();raw["runtime_dlc_mask"]["raw_u32"]=json!(1<<8);
            let frame=world_from_catalog(raw,json!({"countries":{"GER":{"cosmetic_tag":null,"politics":{"ruling_party":"fascism"}}}}));
            let mut state=viewer();state.family=crate::readonly_design_peace::DesignerViewKind::Plane;
            state.anchor_catalogue_family=state.family;
            let surface=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&state,160,120).unwrap();
            assert_eq!(surface.visible_design_ids,["7:70"]);
            assert!(surface.field_refusals.is_empty(),"{:?}",surface.field_refusals);
            let rects:Vec<_>=surface.output.debug_rects.iter().filter(|r|r.name.contains("7:70")&&r.name.ends_with("_mission_icon")).collect();
            assert_eq!(rects.len(),expected);
            let first=rects.iter().find(|r|r.name.ends_with("mission_0_mission_icon")).unwrap();
            for bit in 0..expected {
                let element=format!("mission_{bit}_mission_icon");
                let rect=rects.iter().find(|r|r.name.ends_with(&element)).unwrap();
                let (x,y)=if horizontal {(bit%3,bit/3)} else {(bit/3,bit%3)};
                assert_eq!((rect.x-first.x,rect.y-first.y),(x as i32*8,y as i32*8));
                let pixels=row_element_pixels(&surface,&element);
                assert!(pixels.chunks_exact(4).all(|p|p==[120+bit as u8,80+bit as u8,40+bit as u8,255]),"frame {bit}");
            }
            assert!(!surface.output.debug_rects.iter().any(|r|r.name.contains("mission_11_")));
        }
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

    fn row_element_pixels(surface: &super::super::ObserverDesignerSurface, element: &str) -> Vec<u8> {
        let rect = surface.output.debug_rects.iter().find(|rect|
            rect.name.contains("7:70") && rect.name.ends_with(&format!("::{element}")))
            .unwrap_or_else(|| panic!("missing row element {element}: {:?}", surface.output.debug_rects));
        let image = &surface.output.image;
        assert!(rect.x >= 0 && rect.y >= 0 && rect.w > 0 && rect.h > 0);
        assert!(rect.x as usize + rect.w as usize <= image.width);
        assert!(rect.y as usize + rect.h as usize <= image.height);
        (rect.y as usize..(rect.y + rect.h) as usize).flat_map(|y| {
            let start = (y * image.width + rect.x as usize) * 4;
            image.pixels[start..start + rect.w as usize * 4].iter().copied()
        }).collect()
    }

    #[test]
    fn row_flag_uses_origin_override_while_record_owner_remains_creator() {
        let (root,_) = production_fixture();
        let mut italian = std::fs::read(root.0.join("gfx/flags/small/GER.tga")).unwrap();
        italian[18..].copy_from_slice(&[0,255,0,255,0,255,0,255]);
        std::fs::write(root.0.join("gfx/flags/small/ITA.tga"), italian).unwrap();
        let renderer = admitted_production_renderer_with_assets(&root.0,None,&["gfx/flags/small/ITA.tga"]);
        let authored=DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let (mut raw,mut rest)=owner_prefix_inputs();
        rest["countries"]["ITA"]["cosmetic_tag"] = Value::Null;
        rest["countries"]["ITA"]["politics"] = json!({"ruling_party":"fascism"});
        raw["variants"].as_array_mut().unwrap().truncate(1);
        raw["variants"][0]["creator_tag"]=json!("ITA");
        raw["variants"][0]["owner_tag"]=json!("ITA");
        raw["variants"][0]["origin_override_tag"]=json!("GER");
        raw["variants"][0]["selected_country_tag"]=json!("GER");
        let frame=world_from_catalog(raw.clone(),rest.clone());
        let result=projected(&authored,&frame,&viewer()).unwrap();
        assert_eq!(result.ordered[0].record.owner.as_deref(),Some("ITA"));
        assert_eq!(result.ordered[0].flag_country_tag,"GER");
        let surface=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        assert_eq!(surface.visible_design_ids,["7:70"]);
        assert!(!surface.field_refusals.contains_key("7:70/flag"));
        let actual = row_element_pixels(&surface, "creator_flag");
        raw["variants"][0]["origin_override_tag"]=Value::Null;
        raw["variants"][0]["creator_tag"]=json!("GER");
        raw["variants"][0]["owner_tag"]=json!("GER");
        let german=world_from_catalog(raw.clone(),rest.clone());
        let german=renderer.compose_xui13_designer_surface(&german,german.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        raw["variants"][0]["creator_tag"]=json!("ITA");
        raw["variants"][0]["owner_tag"]=json!("ITA");
        raw["variants"][0]["selected_country_tag"]=json!("ITA");
        let italian=world_from_catalog(raw,rest);
        let italian=renderer.compose_xui13_designer_surface(&italian,italian.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        assert!(!italian.field_refusals.contains_key("7:70/flag"));
        assert_eq!(actual, row_element_pixels(&german, "creator_flag"));
        assert_ne!(actual, row_element_pixels(&italian, "creator_flag"));
    }

    #[test]
    fn missing_row_flag_preserves_membership_chrome_and_other_row_pixels() {
        let (root,_) = production_fixture();
        let renderer=admitted_production_renderer(&root.0,None);
        let missing=admitted_production_renderer(&root.0,Some("gfx/flags/small/GER.tga"));
        let (mut raw,rest)=owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        let frame=world_from_catalog(raw,rest);
        let complete=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        let partial=missing.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        assert_eq!(partial.ordered_design_ids,["7:70"]);
        assert_eq!(partial.visible_design_ids,["7:70"]);
        assert!(partial.membership_complete);
        assert!(partial.country_filters_complete);
        assert!(partial.field_refusals["7:70/flag"].contains("GER"));
        assert!(!partial.field_refusals.contains_key("7:70/equipment_icon"));
        assert!(partial.output.report.drawn_elements>0);
        assert_eq!(row_element_pixels(&partial,"equipment"),row_element_pixels(&complete,"equipment"));
        let flag = complete.output.debug_rects.iter().find(|rect|
            rect.name.contains("7:70") && rect.name.ends_with("::creator_flag")).unwrap();
        assert_ne!(partial.output.image,complete.output.image);
        for y in 0..complete.output.image.height {
            for x in 0..complete.output.image.width {
                if x >= flag.x as usize && x < (flag.x+flag.w) as usize &&
                    y >= flag.y as usize && y < (flag.y+flag.h) as usize { continue; }
                let offset=(y*complete.output.image.width+x)*4;
                assert_eq!(&partial.output.image.pixels[offset..offset+4],
                    &complete.output.image.pixels[offset..offset+4],"unrelated pixel {x},{y}");
            }
        }
    }

    #[test]
    fn selected_and_unselected_row_names_have_only_authored_glyphs() {
        let (root,_) = production_fixture();
        let renderer=admitted_production_renderer(&root.0,None);
        let (mut raw,rest)=owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        let frame=world_from_catalog(raw,rest);
        for selected in [false,true] {
            let mut state=viewer();
            state.selected_design_id=selected.then(|| "7:70".into());
            let surface=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&state,160,120).unwrap();
            assert_eq!(surface.selected_design_id,state.selected_design_id);
            assert_eq!(surface.visible_design_ids,["7:70"]);
            assert!(surface.output.report.missing_glyphs.is_empty(),
                "selected={selected}: unexpected characters in authored name: {:?}",surface.output.report.missing_glyphs);
            let pixels=row_element_pixels(&surface,"name");
            assert!(pixels.chunks_exact(4).any(|pixel|pixel[3]!=0),"name must actually render");
        }
    }

    #[test]
    fn missing_foreign_flag_preserves_country_strip_and_known_owner_row() {
        let (root,_) = production_fixture();
        std::fs::copy(root.0.join("gfx/flags/small/GER.tga"),root.0.join("gfx/flags/small/ITA.tga")).unwrap();
        let renderer=admitted_production_renderer_with_assets(&root.0,None,&["gfx/flags/small/ITA.tga"]);
        let missing=admitted_production_renderer_with_assets(&root.0,Some("gfx/flags/small/ITA.tga"),&["gfx/flags/small/ITA.tga"]);
        let (mut raw,mut rest)=owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(2);
        rest["countries"]["ITA"]["cosmetic_tag"]=Value::Null;
        rest["countries"]["ITA"]["politics"]=json!({"ruling_party":"fascism"});
        let frame=world_from_catalog(raw,rest);
        let complete=renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        let partial=missing.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap();
        assert_eq!(partial.ordered_design_ids,["7:70","8:70"]);
        assert_eq!(partial.visible_design_ids,complete.visible_design_ids);
        assert!(partial.membership_complete && partial.country_filters_complete);
        assert_eq!(partial.ordered_country_filter_tags,["ITA"]);
        assert_eq!(partial.visible_country_filter_tags,["ITA"]);
        assert_eq!(partial.ordered_country_filter_tags,complete.ordered_country_filter_tags);
        assert!(partial.field_refusals["8:70/flag"].contains("ITA"));
        assert!(partial.field_refusals["country_filters/ITA/flag"].contains("ITA"));
        assert!(!partial.field_refusals.contains_key("7:70/flag"));
        assert_eq!(row_element_pixels(&partial,"creator_flag"),row_element_pixels(&complete,"creator_flag"));
        assert_eq!(row_element_pixels(&partial,"equipment"),row_element_pixels(&complete,"equipment"));
        assert!(partial.output.report.drawn_elements>0);
    }

    #[test]
    fn icon_header_active_and_empty_do_not_trigger_dot_sentinel_fallback() {
        for (kind,value,expected) in [(0,"",DesignerRowIcon::Sprite("".into())),(0,".",DesignerRowIcon::Sprite(".".into())),
            (1,"",DesignerRowIcon::Texture("".into())),(1,"gfx/custom.dds",DesignerRowIcon::Texture("gfx/custom.dds".into()))] {
            for active in [false,true] {
                let icon=DesignItemIconObservation::Measured { active, kind, token:DesignTextObservation::Measured {
                    value:value.into(),utf8_hex:value.as_bytes().iter().map(|b|format!("{b:02x}")).collect() } };
                assert_eq!(item_icon(&icon),Ok(if active { expected.clone() } else { DesignerRowIcon::Authored }));
            }
        }
        let sentinel=DesignItemIconObservation::Measured { active:false,kind:1,token:DesignTextObservation::Measured { value:".".into(),utf8_hex:"2e".into() } };
        assert!(item_icon(&sentinel).unwrap_err().contains("item-database"));
    }
    #[test]
    fn inactive_override_preserves_actual_authored_pixels_before_active_override_changes_them() {
        let (root,_) = production_fixture();
        let mut texture=std::fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap();
        for pixel in texture[18..].chunks_exact_mut(4) { pixel.copy_from_slice(&[0,255,0,255]); }
        std::fs::write(root.0.join("gfx/interface/xui13_override.tga"),texture).unwrap();
        let gfx=root.0.join("interface/designer.gfx");
        let mut definitions=std::fs::read_to_string(&gfx).unwrap();
        definitions.push_str(r#"spriteTypes = { spriteType = { name = "GFX_xui13_override" texturefile = "gfx/interface/xui13_override.tga" } }"#);
        std::fs::write(gfx,definitions).unwrap();
        let renderer=admitted_production_renderer_with_assets(&root.0,None,&["gfx/interface/xui13_override.tga"]);
        let (mut raw,rest)=owner_prefix_inputs();
        raw["variants"].as_array_mut().unwrap().truncate(1);
        let compose=|raw:Value| {
            let frame=world_from_catalog(raw,rest.clone());
            renderer.compose_xui13_designer_surface(&frame,frame.named_country("GER").unwrap(),&viewer(),160,120).unwrap()
        };
        let baseline=compose(raw.clone());
        raw["variants"][0]["definition"]["item_icon_override"]=json!({"status":"measured","active":false,"kind":0,
            "token":{"status":"measured","value":"GFX_xui13_override","utf8_hex":"4746585f78756931335f6f76657272696465"}});
        let inactive=compose(raw.clone());
        assert_eq!(inactive.ordered_design_ids,baseline.ordered_design_ids);
        assert_eq!(inactive.visible_design_ids,["7:70"]);
        assert!(!inactive.field_refusals.contains_key("7:70/equipment_icon"));
        assert_eq!(row_element_pixels(&inactive,"equipment"),row_element_pixels(&baseline,"equipment"));
        raw["variants"][0]["definition"]["item_icon_override"]["active"]=json!(true);
        let active=compose(raw);
        assert_ne!(row_element_pixels(&active,"equipment"),row_element_pixels(&inactive,"equipment"));
        assert_eq!(row_element_pixels(&active,"creator_flag"),row_element_pixels(&inactive,"creator_flag"));
    }

    #[test]
    fn inactive_sprite_does_not_need_token_but_texture_must_exclude_sentinel() {
        let mut value=serde_json::from_value::<DesignItemIconObservation>(json!({"status":"measured","active":false,"kind":0,
            "token":{"status":"unavailable","reason":{"kind":"unreadable","field":"icon.token"}}})).unwrap();
        assert_eq!(item_icon(&value),Ok(DesignerRowIcon::Authored));
        if let DesignItemIconObservation::Measured {kind,..}=&mut value { *kind=1; }
        assert!(item_icon(&value).is_err());
    }

}
