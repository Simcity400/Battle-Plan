use super::test_support::*;
use super::*;
use content_reader_key_names::buildings::BuildingType;
use content_reader_raids::{raids::RaidType as Kind, raids::TargetType};
use replay_data::{
    rows::BuildingRow, rows::CountryBuildingResourceRow, rows::CountryLinkRow, rows::CountryRow,
    rows::FactionMemberRow, rows::LexicalTokenRow, rows::ProvinceRow, rows::RaidCooldownRow,
    rows::RaidCountryRow, rows::RaidPlayerAllowedTypeRow, rows::RaidTargetRow,
    rows::RaidTargetStatusRow, rows::ResourceDefinitionRow, rows::StateBaseResourceRow,
    rows::StateDynamicModifierRow, rows::StateResourceInputRow, rows::StateResourceRow,
    rows::StateRow, rows::TableId, rows::TableRows, rows::TableSchema,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use script::LeaderRoles;
use std::sync::Arc;
use std::sync::Mutex;
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::MapSize;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::icon_windows::PoolView;
use viewer_native_gui::native_resources::Context;
#[cfg(test)]
use viewer_native_gui::test_support::gui;
use viewer_runtime::parallel::Workers;
use dashboard_text::table_access::Refusals;

fn scripted(assets: &mut RaidAssets, text: &str) {
    let mut triggers = (*assets.trigger_content.triggers).clone();
    triggers.scripted = content_reader_script::scripted_trigger_grammar::ScriptedTriggers::parse(
        &[("common/scripted_triggers/test.txt", text.as_bytes())],
        &content_reader_script::key_names::KeyNames::default(),
    );
    assets.trigger_content.triggers = Arc::new(triggers);
}

#[test]
fn the_raid_priority_is_the_define_as_a_signed_byte() {
    assert_eq!(map_icon_priority(30.0), Ok(30));
    assert_eq!(map_icon_priority(286.0), Ok(30));
    assert_eq!(map_icon_priority(200.0), Ok(-56));
    assert_eq!(map_icon_priority(-1.0), Ok(-1));
    assert!(map_icon_priority(30.5).is_err());
}

#[test]
fn the_target_batch_is_at_least_one() {
    let batch = |source: &str| {
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        defines
            .load("common/defines/00_defines.lua", source.as_bytes())
            .unwrap();
        target_batch(&defines)
    };
    assert_eq!(
        batch("NDefines = { NRaids = { MAX_TARGETS_TO_UPDATE_PER_FRAME = 100 } }"),
        Ok(100)
    );
    assert_eq!(batch("NDefines = { NRaids = {} }"), Ok(1));
    assert_eq!(
        batch(r#"NDefines = { NRaids = { MAX_TARGETS_TO_UPDATE_PER_FRAME = "100" } }"#),
        Ok(1)
    );
    for below in ["0", "-7", "0.5"] {
        assert_eq!(
            batch(&format!(
                "NDefines = {{ NRaids = {{ MAX_TARGETS_TO_UPDATE_PER_FRAME = {below} }} }}"
            )),
            Ok(1)
        );
    }
    assert!(batch("NDefines = {}").is_err());
}

#[test]
fn membership_follows_ownership_detection_and_show_for() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::Raids>(
        &mut frame,
        vec![
            raid(1, 0, 0, 2),
            raid(2, 1, 0, 2),
            raid(3, 1, 1, 3),
            RaidRow {
                show_for: vec![10],
                outcome: 3,
                ..raid(4, 1, 2, 5)
            },
        ],
    );
    put::<tables::RaidTargets>(
        &mut frame,
        vec![
            target(1, 3, 0),
            target(2, 1, 0),
            target(3, 2, 1),
            target(4, 4, 0),
        ],
    );
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let by_province: BTreeMap<u32, u32> = icons
        .icons
        .iter()
        .map(|icon| (icon.province, icon.frame))
        .collect();
    assert_eq!(by_province.get(&3), Some(&2));
    assert_eq!(by_province.get(&2), Some(&4));
    assert!(!by_province.contains_key(&1));
    assert_eq!(by_province.get(&4), None, "the target is ENG's, not ours");
    let as_eng = icons_for(&assets, &frame, 1);
    assert_eq!(as_eng.get(&1), Some(&2), "ENG's active raid on 1");
    assert_eq!(as_eng.get(&3), None, "GER's raid on 3 is undetected");
    assert_eq!(
        as_eng.get(&4),
        None,
        "ENG's ended raid is shown to GER only"
    );
}

fn icons_for(assets: &RaidAssets, frame: &DecodedFrame, ours: u32) -> BTreeMap<u32, u32> {
    icons(assets, frame, ours, &launch::Unreconstructed)
        .icons
        .iter()
        .map(|icon| (icon.province, icon.frame))
        .collect()
}

#[test]
fn an_ended_raid_needs_show_for_and_an_uncancelled_outcome() {
    let assets = assets();
    let mut frame = frame();
    let mut ended = RaidRow {
        show_for: vec![10],
        outcome: 3,
        ..raid(1, 0, 0, 5)
    };
    put::<tables::RaidTargets>(&mut frame, vec![target(1, 3, 0)]);
    put::<tables::Raids>(&mut frame, vec![ended.clone()]);
    assert_eq!(icons_for(&assets, &frame, 0).get(&3), Some(&3));
    ended.outcome = 5;
    put::<tables::Raids>(&mut frame, vec![ended.clone()]);
    assert_eq!(icons_for(&assets, &frame, 0).get(&3), None);
    ended.outcome = 3;
    ended.show_for.clear();
    put::<tables::Raids>(&mut frame, vec![ended]);
    assert_eq!(icons_for(&assets, &frame, 0).get(&3), None);
}

#[test]
fn incoming_wins_and_the_first_registered_raid_binds() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::Raids>(
        &mut frame,
        vec![
            RaidRow {
                show_for: vec![10],
                outcome: 3,
                ..raid(1, 0, 0, 5)
            },
            raid(2, 1, 0, 2),
        ],
    );
    let mut own = target(1, 1, 0);
    own.scope.province = 1;
    put::<tables::RaidTargets>(&mut frame, vec![own, target(2, 1, 1)]);
    assert_eq!(icons_for(&assets, &frame, 0).get(&1), Some(&4));
}

#[test]
fn offers_need_a_valid_foreign_target_an_unlocked_type_and_show_target() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(
        &mut frame,
        vec![
            status(0, 1),
            status(1, 3),
            status(2, 4),
            RaidTargetStatusRow {
                valid: 0,
                ..status(3, 3)
            },
        ],
    );
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let offers: BTreeMap<u32, &RaidIcon> = icons
        .icons
        .iter()
        .map(|icon| (icon.province, icon))
        .collect();
    assert!(!offers.contains_key(&1));
    assert_eq!(offers[&3].glyph.as_deref(), Some(OTHER_GLYPH));
    assert!(offers[&3].plus);
    assert_eq!(
        offers[&4].glyph.as_deref(),
        Some("GFX_raid_type_icon_oilfield")
    );
    assert!(!offers[&4].plus);
    assert_eq!(offers[&4].frame, 0);
    assert!(!offers[&4].enabled);
    assert_eq!(icons.report.unverified_tint, 0);
    put::<tables::FactionMembers>(
        &mut frame,
        vec![
            FactionMemberRow {
                country: 10,
                faction: 7,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 11,
                faction: 7,
                ordinal: 1,
            },
        ],
    );
    reserved_faction(&mut frame, 7);
    assert!(icons_for(&assets, &frame, 0).is_empty());
}

#[test]
fn used_up_types_locked_categories_and_undetected_buildings_offer_nothing() {
    let mut assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3)]);
    let mut used = frame.clone();
    put::<tables::RaidCountries>(
        &mut used,
        vec![RaidCountryRow {
            canonical_slot: 0,
            country: 10,
            eligible_types: vec![COMMANDO],
            used_types: vec![COMMANDO],
        }],
    );
    put::<tables::RaidPlayerAllowedTypes>(
        &mut used,
        vec![RaidPlayerAllowedTypeRow {
            country: 10,
            allowed: vec![COMMANDO],
        }],
    );
    assert!(icons_for(&assets, &used, 0).is_empty());
    let dam = RaidTargetStatusRow {
        scope: RaidScope {
            building_type: DAM,
            building_location: 3,
            building_flags: 1,
            ..Default::default()
        },
        ..status(0, 0)
    };
    assets.catalog.types[0]
        .target_types
        .push(TargetType::Building(
            content_reader_raids::raids::BuildingSelector {
                types: vec!["dam".into()],
                ..Default::default()
            },
        ));
    put::<tables::RaidTargetStatuses>(&mut frame, vec![dam.clone()]);
    assert!(icons_for(&assets, &frame, 0).is_empty());
    put::<tables::RaidTargetStatuses>(
        &mut frame,
        vec![RaidTargetStatusRow {
            detected: vec![1],
            ..dam.clone()
        }],
    );
    put::<tables::RaidCooldowns>(
        &mut frame,
        vec![RaidCooldownRow {
            days: 3,
            owner_slot: 0,
            ordinal: 0,
            raid_type: OILFIELD,
            scope: dam.scope.clone(),
        }],
    );
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert_eq!(icons.icons.len(), 1);
    assert!(!icons.icons[0].enabled);
    assert_eq!(
        icons.report.unverified_tint, 0,
        "the cooldown proves it grey"
    );
    assets.trigger_content.dlcs.clear();
    assert!(icons_for(&assets, &frame, 0).is_empty());
}

#[test]
fn undeclared_glyphs_and_unresolved_targets_are_counted() {
    let mut assets = assets();
    assets.glyphs.remove("GFX_raid_type_icon_oilfield");
    assets.glyphs.remove(UNKNOWN_GLYPH);
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(
        &mut frame,
        vec![
            status(0, 4),
            RaidTargetStatusRow {
                scope: RaidScope::default(),
                ..status(1, 0)
            },
        ],
    );
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert_eq!(icons.report.missing_glyphs, 1);
    assert_eq!(icons.report.unresolved, 1);
    assert!(
        icons
            .report
            .reasons
            .contains_key(&format!("no interface file declares {UNKNOWN_GLYPH}"))
    );
}

fn all_offers(
    assets: &RaidAssets,
    frame: &DecodedFrame,
    ours: u32,
    paths: &(dyn launch::RaidPaths + Sync),
) -> offers::Offers {
    let world = World::new(frame, assets.trigger_content.facts()).unwrap();
    let sweep = offers::Sweep::new(assets, &world, ours, paths);
    let mut cycle = cycle::Cycle::new(frame.branch, ours);
    cycle.begin(assets, &sweep).unwrap();
    while !cycle.complete() {
        cycle.step(&sweep);
    }
    cycle.offers()
}

fn offered(assets: &RaidAssets, frame: &DecodedFrame) -> (Vec<String>, RaidReport) {
    let icons = icons(assets, frame, 0, &launch::Unreconstructed);
    let offers = all_offers(assets, frame, 0, &launch::Unreconstructed);
    let names = offers
        .provinces
        .get(&4)
        .map(|province| {
            province
                .pairs
                .iter()
                .map(|(index, _)| assets.catalog.types[*index].name.clone())
                .collect()
        })
        .unwrap_or_default();
    (names, icons.report)
}

#[test]
fn the_cache_country_uses_the_recorded_list_and_others_evaluate_allowed() {
    content_reader_script::test_game::enter_viewer();
    let mut assets = assets();
    assets.catalog.types[0].allowed = parse("always = no");
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    put::<tables::RaidPlayerAllowedTypes>(
        &mut frame,
        vec![RaidPlayerAllowedTypeRow {
            country: 10,
            allowed: vec![OILFIELD],
        }],
    );
    assert_eq!(offered(&assets, &frame).0, ["oilfield"]);
    put::<tables::RaidPlayerAllowedTypes>(
        &mut frame,
        vec![RaidPlayerAllowedTypeRow {
            country: 11,
            allowed: vec![OILFIELD],
        }],
    );
    assert!(offered(&assets, &frame).0.is_empty());
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries[0].id = 12;
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 12,
        canonical_slot: 0,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    put::<tables::RaidPlayerAllowedTypes>(
        &mut frame,
        vec![RaidPlayerAllowedTypeRow {
            country: 10,
            allowed: vec![OILFIELD],
        }],
    );
    assert!(offered(&assets, &frame).0.is_empty());
    assets.catalog.types[0].allowed = Vec::new();
    assert_eq!(
        offered(&assets, &frame).0,
        ["oilfield"],
        "an empty allowed allows"
    );
}

#[test]
fn an_undecided_allowed_draws_the_offer_grey_and_counts_it() {
    let mut assets = assets();
    assets.catalog.types[0].allowed = parse("is_ai = yes");
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let icon = icons.icons.iter().find(|icon| icon.province == 4).unwrap();
    assert!(!icon.enabled);
    assert_eq!(icons.report.unverified_tint, 1);
    assert!(
        icons
            .report
            .reasons
            .keys()
            .any(|reason| reason.contains("oilfield's allowed") && reason.contains("is_ai"))
    );
}

#[test]
fn an_unsupported_trigger_withholds_the_province_and_names_itself() {
    let mut assets = assets();
    assets.catalog.types[0].show_target = parse("is_ai = yes");
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert!(icons.icons.is_empty());
    assert_eq!(icons.report.withheld, 1);
    assert!(
        icons
            .report
            .reasons
            .keys()
            .any(|reason| reason.contains("is_ai"))
    );
}

#[test]
fn a_state_building_anchors_in_content_order_only_while_membership_matches() {
    let mut assets = assets();
    assets.trigger_content.buildings.types.insert(
        "airfield".into(),
        BuildingType {
            name: "airfield".into(),
            air_base: true,
            ..Default::default()
        },
    );
    set_counts(&mut assets);
    assets.sites.insert((20, Site::AirBase), vec![4, 3]);
    let mut frame = frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    tokens.push(LexicalTokenRow {
        token: 304,
        literal: b"airfield".to_vec(),
    });
    put::<tables::LexicalTokens>(&mut frame, tokens);
    let scope = RaidScope {
        building_type: 304,
        building_location: 20,
        building_flags: 2,
        ..Default::default()
    };
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(assets.anchor_province(&world, &scope), Ok(Some(3)));
    drop(world);
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[1].state = 20;
    put::<tables::Provinces>(&mut frame, provinces);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert!(assets.anchor_province(&world, &scope).is_err());
}

#[test]
fn a_kept_state_building_icon_follows_its_states_order() {
    let mut assets = assets();
    assets.batch = Ok(1);
    assets.trigger_content.buildings.types.insert(
        "airfield".into(),
        BuildingType {
            name: "airfield".into(),
            air_base: true,
            ..Default::default()
        },
    );
    set_counts(&mut assets);
    assets.sites.insert((20, Site::AirBase), vec![4, 3]);
    assets.catalog.types[0].target_types = vec![TargetType::Building(
        content_reader_raids::raids::BuildingSelector {
            types: vec!["airfield".into()],
            ..Default::default()
        },
    )];
    let mut frame = frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    tokens.push(LexicalTokenRow {
        token: 304,
        literal: b"airfield".to_vec(),
    });
    put::<tables::LexicalTokens>(&mut frame, tokens);
    let airfield = RaidTargetStatusRow {
        scope: RaidScope {
            building_type: 304,
            building_location: 20,
            building_flags: 2,
            ..Default::default()
        },
        ..status(0, 0)
    };
    put::<tables::RaidTargetStatuses>(&mut frame, vec![airfield, status(1, 1)]);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    let mut kept = IconCache::default();
    let mut hour = |frame: &DecodedFrame, cycle: &mut cycle::Cycle| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let layer = HourLayer::new(&assets, &world, 0, &launch::Unreconstructed);
        cycle.begin(&assets, &layer.sweep).unwrap();
        cycle.step(&layer.sweep);
        let carried = layer.icons(cycle, &mut kept);
        assert_eq!(
            carried.icons,
            layer.icons(cycle, &mut IconCache::default()).icons
        );
        assert_eq!(
            carried.report,
            layer.icons(cycle, &mut IconCache::default()).report
        );
        carried
    };
    let first = hour(&frame, &mut cycle);
    assert_eq!(
        first
            .icons
            .iter()
            .map(|icon| icon.province)
            .collect::<Vec<_>>(),
        [3]
    );
    frame.hour += 1;
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[1].state = 20;
    put::<tables::Provinces>(&mut frame, provinces);
    let second = hour(&frame, &mut cycle);
    assert!(second.icons.is_empty());
    assert_eq!(second.report.withheld, 1);
}

#[test]
fn an_hour_evaluated_again_keeps_the_rows_it_evaluated() {
    let mut assets = assets();
    assets.batch = Ok(1);
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4), status(1, 1)]);
    let mut cycle = cycle::Cycle::new(frame.branch, 0);
    let evaluate = |frame: &DecodedFrame, cycle: &mut cycle::Cycle| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let sweep = offers::Sweep::new(&assets, &world, 0, &launch::Unreconstructed);
        cycle.begin(&assets, &sweep).unwrap();
        cycle.step(&sweep);
        cycle.progress()
    };
    assert_eq!(evaluate(&frame, &mut cycle), (1, 2));
    assert_eq!(evaluate(&frame, &mut cycle), (2, 2), "the hour's next evaluation continues its cycle");
    assert_eq!(evaluate(&frame, &mut cycle), (2, 2), "and a complete hour stays complete");
    frame.hour += 1;
    assert_eq!(evaluate(&frame, &mut cycle), (1, 2), "the next hour begins again");
}

#[test]
fn a_state_target_resolves_to_the_province_nearest_its_centre() {
    let assets = assets();
    let scope = RaidScope {
        state: 20,
        ..Default::default()
    };
    assert_eq!(assets.province(&scope), Some(4));
}

#[test]
fn only_the_hours_own_raid_icons_are_drawn() {
    use std::sync::Arc;
    let icons = Arc::new(RaidIcons {
        icons: vec![RaidIcon {
            group: 3,
            province: 3,
            frame: 2,
            enabled: true,
            glyph: None,
            plus: false,
            tooltip: None,
        }],
        report: RaidReport::default(),
        built_for: Some((1, 40, 7)),
    });
    let same = RaidIcons::carried(Arc::clone(&icons), 1, 40, 7);
    assert!(Arc::ptr_eq(&same, &icons), "this hour's own icons");
    assert!(same.report.warnings().is_empty());
    for (branch, hour, ours) in [(1, 41, 7), (1, 39, 7), (1, 40, 8), (2, 40, 7)] {
        let other = RaidIcons::carried(Arc::clone(&icons), branch, hour, ours);
        assert!(other.icons.is_empty());
        assert!(other.report.pending);
        assert!(other.report.warnings()[0].contains("still being built"));
    }
    assert!(RaidIcons::carried(Arc::default(), 1, 40, 7).report.pending);
}

#[test]
fn a_type_the_port_cannot_use_leaves_its_targets_undecided_by_name() {
    let mut assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3), status(1, 4)]);
    assets.catalog.types[0].target_types = vec![TargetType::Provinces(vec![3])];
    assets.catalog.types.truncate(1);
    let declaration = kind("mine", TargetType::Provinces(vec![4]));
    assets.catalog.uncertain = vec![content_reader_raids::raids::UncertainType {
        name: "mine".into(),
        kind: content_reader_parser::DiagnosticKind::Refused,
        reason: "it is declared more than once".into(),
        declarations: vec![declaration.clone(), declaration],
    }];
    let first = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let provinces: Vec<u32> = first.icons.iter().map(|icon| icon.province).collect();
    assert_eq!(provinces, [3], "province 3 is decided");
    assert_eq!(first.report.withheld, 1);
    assert!(
        first
            .report
            .reasons
            .keys()
            .any(|reason| reason.contains("raid type mine") && reason.contains("more than once"))
    );
    assets.catalog.uncertain[0].declarations.clear();
    assert!(
        icons_for(&assets, &frame, 0).is_empty(),
        "both provinces are undecided"
    );
    assets.catalog.uncertain[0].declarations = vec![kind("mine", TargetType::Provinces(vec![9]))];
    let decided = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert_eq!(decided.icons.len(), 1);
    assert_eq!(decided.report.withheld, 0);
}

#[test]
fn without_the_player_cache_a_type_not_allowed_now_is_undecided() {
    let mut assets = assets();
    assets.catalog.types[0].allowed = parse("always = no");
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    assert!(
        icons_for(&assets, &frame, 0).is_empty(),
        "the cache decides it"
    );
    *frame.tables[TableId::RaidPlayerAllowedTypes.index()] = TableView::Unavailable {
        reason: "short".into(),
    };
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let icon = icons.icons.iter().find(|icon| icon.province == 4).unwrap();
    assert!(!icon.enabled);
    assert_eq!(icons.report.unverified_tint, 1);
    assert!(
        icons
            .report
            .reasons
            .keys()
            .any(|reason| reason.contains("player raid cache"))
    );
}

#[test]
fn a_country_without_a_raid_status_evaluates_to_no_offers() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    put::<tables::RaidCountries>(&mut frame, Vec::new());
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert!(icons.icons.is_empty());
    assert!(icons.report.cycle_complete());
}

#[test]
fn an_incoming_raid_of_an_unknown_type_counts_its_stand_in_glyph() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::Raids>(&mut frame, vec![raid(2, 1, 0, 2)]);
    put::<tables::RaidTargets>(
        &mut frame,
        vec![RaidTargetRow {
            raid_type: DAM,
            ..target(2, 1, 1)
        }],
    );
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert_eq!(icons.icons[0].glyph.as_deref(), Some(UNKNOWN_GLYPH));
    assert_eq!(icons.report.unknown_glyphs, 1);
    assert_eq!(
        icons.report.refused_tokens,
        Refusals::from([("raid type dam is not in the content".to_owned(), 1)])
    );
}

#[test]
fn a_scripted_trigger_with_parameters_reads_as_no_like_the_game() {
    content_reader_script::test_game::enter_viewer();
    let mut assets = assets();
    scripted(
        &mut assets,
        "is_germany = { tag = GER } is_param = { tag = $who$ } no_param = { always = yes }",
    );
    let frame = positive_frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let eval = |source: &str| world.evaluate(&parse(source), Scopes::of(Scope::Country(2)));
    assert_eq!(eval("is_germany = yes"), script::Eval::True);
    assert_eq!(eval("is_germany = no"), script::Eval::False);
    assert_eq!(eval("is_germany = { who = GER }"), script::Eval::False);
    assert_eq!(eval("no_param = { who = GER }"), script::Eval::False);
    match eval("is_param = { who = GER }") {
        script::Eval::Unknown(reasons) => {
            assert!(
                reasons
                    .iter()
                    .any(|reason| reason.contains("$who$") && reason.contains("no script variables row")),
                "{reasons:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn has_tech_is_unknown_when_a_completed_row_has_no_definition() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::ResearchTechnologyRow, rows::TechnologyDefinitionRow};
    let assets = assets();
    let mut frame = frame();
    put::<tables::TechnologyDefinitions>(
        &mut frame,
        vec![TechnologyDefinitionRow {
            database_index: 1,
            technology_handle: 1,
            id: b"radar".to_vec(),
            ..Default::default()
        }],
    );
    let completed = |handle| ResearchTechnologyRow {
        database_index: (handle) as i32,
        country: 10,
        technology_handle: handle,
        researched: 1,
        classifier: engine_research::ctechnology::CLASSIFIER_COMPLETED,
        ..Default::default()
    };
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(0)))
    };
    put::<tables::ResearchTechnologies>(&mut frame, vec![completed(1)]);
    assert_eq!(eval(&frame, "has_tech = radar"), script::Eval::True);
    assert_eq!(eval(&frame, "has_tech = sonar"), script::Eval::False);
    put::<tables::ResearchTechnologies>(&mut frame, vec![completed(1), completed(2)]);
    assert_eq!(eval(&frame, "has_tech = radar"), script::Eval::True);
    assert!(matches!(
        eval(&frame, "has_tech = sonar"),
        script::Eval::Unknown(_)
    ));
}

#[test]
fn has_tech_uses_first_indexed_case_insensitive_name_and_exact_classifier() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::ResearchTechnologyRow, rows::TechnologyDefinitionRow};
    let assets = assets();
    let mut frame = frame();
    put::<tables::TechnologyDefinitions>(
        &mut frame,
        vec![
            TechnologyDefinitionRow {
                database_index: 1,
                technology_handle: 7,
                id: b"radar".to_vec(),
                ..Default::default()
            },
            TechnologyDefinitionRow {
                database_index: 2,
                technology_handle: 7,
                id: b"RADAR".to_vec(),
                ..Default::default()
            },
        ],
    );
    let row = |database_index, classifier, researched| ResearchTechnologyRow {
        country: 10,
        database_index,
        technology_handle: 7,
        classifier,
        researched,
        ..Default::default()
    };
    let eval = |frame: &DecodedFrame| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse("has_tech = RaDaR"), Scopes::of(Scope::Country(0)))
    };
    put::<tables::ResearchTechnologies>(&mut frame, vec![row(1, 0, 0), row(2, 4, 1)]);
    assert_eq!(eval(&frame), script::Eval::False);
    put::<tables::ResearchTechnologies>(&mut frame, vec![row(1, 4, 0), row(2, 0, 1)]);
    assert_eq!(eval(&frame), script::Eval::True);
}

#[test]
fn missing_trigger_inputs_are_unknown_rather_than_false() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let mut frame = frame();
    let unknown = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        matches!(
            world.evaluate(&parse(source), Scopes::of(Scope::Country(0))),
            script::Eval::Unknown(_)
        )
    };
    assert!(unknown(&frame, "num_of_nukes > 0"));
    assert!(unknown(&frame, "has_completed_focus = { }"));
    put::<tables::FactionMembers>(
        &mut frame,
        vec![replay_data::rows::FactionMemberRow {
            country: 99,
            faction: 1,
            ordinal: 0,
        }],
    );
    assert!(unknown(&frame, "is_in_faction_with = ENG"));
    put::<tables::FactionMembers>(&mut frame, Vec::new());
    put::<tables::WarRelations>(
        &mut frame,
        vec![replay_data::rows::WarRelationRow {
            first_country: 98,
            second_country: 11,
            start_hour: 1,
            ..Default::default()
        }],
    );
    assert!(unknown(&frame, "has_war_with = ENG"));
    put::<tables::WarRelations>(&mut frame, Vec::new());
    assert!(!unknown(&frame, "has_war_with = ENG"));
}

#[test]
fn military_access_reads_only_the_owner_asked() {
    use content_reader_unit_path::unit_path::MoveWorld;
    use replay_data::rows::RelationLinkRow;
    let assets = assets();
    let mut frame = frame();
    put::<tables::RelationLinks>(
        &mut frame,
        vec![
            RelationLinkRow {
                country: 10,
                target_index: 1,
                links: RelationLinkRow::MILITARY_ACCESS,
                transport_subject: Default::default(),
            },
            RelationLinkRow {
                country: 11,
                target_index: 0,
                links: 1,
                transport_subject: Default::default(),
            },
            RelationLinkRow {
                country: 99,
                target_index: 0,
                links: RelationLinkRow::MILITARY_ACCESS,
                transport_subject: Default::default(),
            },
        ],
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let cells = reach::Cells::default();
    let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
    assert_eq!(moves.military_access(0, 1), Ok(true));
    assert_eq!(moves.military_access(1, 0), Ok(false));
    assert_eq!(moves.military_access(1, 1), Ok(false));
    assert!(moves.military_access(0, 0).is_err());
}

#[test]
fn incomplete_raid_tables_withhold_the_hours_offers() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3), status(1, 4)]);
    *frame.tables[TableId::RaidCountries.index()] = TableView::Unavailable {
        reason: "short".into(),
    };
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert!(icons.icons.is_empty());
    assert!(icons.report.cycle_complete());
    assert!(
        all_offers(&assets, &frame, 0, &launch::Unreconstructed)
            .reasons
            .iter()
            .any(|reason| reason.contains("raid offers are withheld"))
    );
}

#[test]
fn unlinked_focus_project_and_party_rows_leave_their_triggers_unknown() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::CompletedFocusRow, rows::PoliticalPartyRow};
    let mut assets = assets();
    assets.trigger_content.ideologies = Some(raid_ideologies());
    set_counts(&mut assets);
    let mut frame = positive_frame();
    let evaluate = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(2)))
    };
    put::<tables::CompletedFocuses>(
        &mut frame,
        vec![CompletedFocusRow {
            country: 99,
            token: b"focus".to_vec(),
            ..Default::default()
        }],
    );
    put::<tables::PoliticalParties>(
        &mut frame,
        vec![PoliticalPartyRow {
            country: 99,
            id: 7,
            is_ruling: 1,
            token: b"mod_party".to_vec(),
            ..Default::default()
        }],
    );
    assert!(matches!(
        evaluate(&frame, "has_completed_focus = focus"),
        script::Eval::Unknown(_)
    ));
    assert!(matches!(
        evaluate(&frame, "has_government = mod_party"),
        script::Eval::Unknown(_)
    ));
    put::<tables::CompletedFocuses>(&mut frame, Vec::new());
    let mut parties = frame
        .complete::<tables::PoliticalParties>()
        .unwrap()
        .to_vec();
    parties.push(PoliticalPartyRow {
        country: 10,
        id: 7,
        is_ruling: 1,
        token: b"mod_party".to_vec(),
        ..Default::default()
    });
    put::<tables::PoliticalParties>(&mut frame, parties);
    assert_eq!(
        evaluate(&frame, "has_completed_focus = focus"),
        script::Eval::False
    );
    assert_eq!(
        evaluate(&frame, "has_government = mod_party"),
        script::Eval::True
    );
    let mut parties = frame
        .complete::<tables::PoliticalParties>()
        .unwrap()
        .to_vec();
    parties.remove(0);
    put::<tables::PoliticalParties>(&mut frame, parties);
    assert_eq!(
        evaluate(&frame, "has_government = mod_party"),
        script::Eval::True
    );
}

fn raid_ideologies() -> content_reader_key_names::ideologies::Ideologies {
    let root = std::env::temp_dir().join(format!(
        "raid-ideologies-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = root.join("common/ideologies/synthetic.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "ideologies = { mod_party = { types = { mod_type = {} } } mod_other = { types = { other_type = {} } } }").unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let ideologies = content_reader_key_names::ideologies::read(&layers).unwrap().0;
    std::fs::remove_dir_all(root).unwrap();
    ideologies
}

#[test]
fn raid_facts_keep_loaded_ideologies_for_government_and_leader_subtypes() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::CountryAppearanceRow, rows::PoliticalPartyRow};
    use script::Eval::{False, True};
    let mut assets = unnamed_assets();
    let mut frame = positive_frame();
    put::<tables::PoliticalParties>(
        &mut frame,
        vec![
            PoliticalPartyRow {
                country: 10,
                id: 7,
                is_ruling: 1,
                token: b"mod_party".to_vec(),
                ..Default::default()
            },
            PoliticalPartyRow {
                country: 10,
                id: 8,
                token: b"mod_other".to_vec(),
                ..Default::default()
            },
        ],
    );
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![CountryAppearanceRow {
            country: 10,
            leader_ideology: b"mod_type".to_vec(),
            ..Default::default()
        }],
    );
    let evaluate = |assets: &RaidAssets, source: &str| {
        World::new(&frame, assets.trigger_content.facts())
            .unwrap()
            .evaluate(&parse(source), Scopes::of(Scope::Country(2)))
    };
    assert!(matches!(
        evaluate(&assets, "has_government = mod_party"),
        script::Eval::Unknown(reasons)
            if reasons.iter().any(|reason| reason.contains("raid ideologies are not loaded"))
    ));
    assets.trigger_content.ideologies = Some(raid_ideologies());
    set_counts(&mut assets);
    assert_eq!(evaluate(&assets, "has_government = mod_party"), True);
    assert_eq!(evaluate(&assets, "has_government = mod_other"), False);
    assert_eq!(
        evaluate(&assets, "has_country_leader_ideology = mod_type"),
        True
    );
    assert_eq!(
        evaluate(&assets, "has_country_leader_ideology = other_type"),
        False
    );
}

#[test]
fn raid_type_tokens_without_a_literal_leave_the_types_undecided() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3)]);
    let mut used = frame.clone();
    put::<tables::RaidCountries>(
        &mut used,
        vec![RaidCountryRow {
            canonical_slot: 0,
            country: 10,
            eligible_types: vec![COMMANDO],
            used_types: vec![0xdead],
        }],
    );
    let offers = all_offers(&assets, &used, 0, &launch::Unreconstructed);
    assert!(offers.withheld.contains(&3));
    put::<tables::RaidPlayerAllowedTypes>(
        &mut frame,
        vec![RaidPlayerAllowedTypeRow {
            country: 10,
            allowed: vec![0xdead],
        }],
    );
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let icon = icons.icons.iter().find(|icon| icon.province == 3).unwrap();
    assert!(!icon.enabled);
    assert!(icons.report.unverified_tint > 0);
}

fn drawn(height: f32, viewport: [u32; 2], ui_scale: f32) -> (usize, Batch) {
    drawn_in(Some(RaidReach::Normal), height, viewport, ui_scale)
}

fn drawn_in(
    reach: Option<RaidReach>,
    height: f32,
    viewport: [u32; 2],
    ui_scale: f32,
) -> (usize, Batch) {
    let icon = |province, frame| RaidIcon {
        group: province,
        province,
        frame,
        enabled: true,
        glyph: Some("glyph".into()),
        plus: false,
        tooltip: None,
    };
    let icons = RaidIcons {
        icons: vec![icon(1, 0), icon(2, 4)],
        ..RaidIcons::default()
    };
    let anchors = BTreeMap::from([(1, [3000.0, 10.0, 1400.0]), (2, [3000.0, 10.0, 1600.0])]);
    let anchor = |province: u32| anchors.get(&province).copied();
    let mut batch = Batch::default();
    let count = draw(
        &RaidInputs {
            anchor: &anchor,
            camera: Camera {
                center: [3000.0, 1500.0],
                height,
                map: MapSize::TEST,
                defines: MapDefines::test(),
                pose: viewer_gpu::camera::Pose::Map,
            },
            viewport,
            ui_scale,
            reach,
        },
        &icons,
        &mut batch,
    );
    (count, batch)
}

#[test]
fn the_camera_band_includes_both_edges() {
    content_reader_script::test_game::enter_viewer();
    let viewport = [2048, 1152];
    assert!(drawn(10.0, viewport, 1.0).0 > 0);
    assert_eq!(drawn(500.0, viewport, 1.0).0, 2);
    assert_eq!(drawn(9.99, viewport, 1.0).0, 0);
    assert_eq!(drawn(500.01, viewport, 1.0).0, 0);
}

#[test]
fn the_map_mode_decides_whether_and_how_far_raid_icons_show() {
    content_reader_script::test_game::enter_viewer();
    let viewport = [2048, 1152];
    let raids = Some(RaidReach::RaidMode);
    assert_eq!(drawn_in(raids, 1600.0, viewport, 1.0).0, 2);
    assert_eq!(drawn_in(raids, 1600.01, viewport, 1.0).0, 0);
    assert_eq!(drawn_in(raids, 9.99, viewport, 1.0).0, 0);
    assert_eq!(drawn(1600.0, viewport, 1.0).0, 0);
    let outside = drawn(500.01, viewport, 1.0);
    for height in [10.0, 300.0, 500.0] {
        let hidden = drawn_in(None, height, viewport, 1.0);
        assert_eq!(hidden.0, 0);
        assert_eq!(hidden.1.quads, outside.1.quads);
        assert_eq!(
            drawn_in(Some(RaidReach::Normal), height, viewport, 1.0)
                .1
                .quads,
            drawn(height, viewport, 1.0).1.quads
        );
    }
}

const OIL: u32 = 300;
const REFINERY: u32 = 301;
const DAMAGED: u32 = 302;

fn resource_frame() -> DecodedFrame {
    let mut frame = frame();
    let mut tokens = match &*frame.tables[TableId::LexicalTokens.index()] {
        TableView::Complete(TableRows::LexicalTokens(rows)) => rows.clone(),
        _ => unreachable!(),
    };
    for (token, name) in [
        (OIL, "oil"),
        (REFINERY, "refinery"),
        (DAMAGED, "refinery_damaged_modifier"),
    ] {
        tokens.push(LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        });
    }
    put::<tables::LexicalTokens>(&mut frame, tokens);
    put::<tables::ResourceDefinitions>(
        &mut frame,
        vec![ResourceDefinitionRow {
            id: 1,
            token: OIL,
            active: 1,
            factory_cost: 0,
        }],
    );
    put::<tables::StateResourceInputs>(
        &mut frame,
        vec![StateResourceInputRow {
            local_manpower: Default::default(),
            local_noncore_manpower: Default::default(),
            recruitment_add: Default::default(),
            recruitment_factor: Default::default(),

            state: 10,
            gate: 0,
            global_factor: 0,
            origin: 1,
            infrastructure_level: Some(3),
            infrastructure_modifier: 0,
            supply_route: 1,
        }],
    );
    put::<tables::StateResources>(
        &mut frame,
        vec![StateResourceRow {
            state: 10,
            resource: OIL,
            pool: 300_000,
            ..Default::default()
        }],
    );
    put::<tables::StateBaseResources>(
        &mut frame,
        vec![StateBaseResourceRow {
            state: 10,
            resource: OIL,
            ordinal: 0,
            amount: 2_000_000,
        }],
    );
    put::<tables::StateBuildings>(
        &mut frame,
        vec![BuildingRow {
            location: 10,
            building_type: REFINERY,
            level: 3,
            healthy_level: 2,
            ..Default::default()
        }],
    );
    frame
}

fn resource_assets() -> RaidAssets {
    let mut assets = assets();
    assets.trigger_content.buildings.types.insert(
        "refinery".into(),
        BuildingType {
            name: "refinery".into(),
            local_resources: vec![("oil".into(), 200_000)],
            ..Default::default()
        },
    );
    set_counts(&mut assets);
    assets
}

fn on_state(assets: &RaidAssets, frame: &DecodedFrame, source: &str) -> script::Eval {
    let world = World::new(frame, assets.trigger_content.facts()).unwrap();
    world.evaluate(&parse(source), Scopes::of(Scope::State(10)))
}

#[test]
fn oil_and_the_damaged_modifier_decide_the_state_selector() {
    content_reader_script::test_game::enter_viewer();
    let assets = resource_assets();
    let mut frame = resource_frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(world.state_resource(10, OIL), Ok(4_402_000));
    drop(world);
    let selector =
        "oil > 44 NOT = { has_dynamic_modifier = { modifier = refinery_damaged_modifier } }";
    assert_eq!(on_state(&assets, &frame, selector), script::Eval::False);
    assert_eq!(on_state(&assets, &frame, "oil > 43"), script::Eval::True);
    put::<tables::StateDynamicModifiers>(
        &mut frame,
        vec![
            StateDynamicModifierRow {
                state: 10,
                ordinal: 0,
                token: b"refinery_damaged_modifier".to_vec(),
                scope_country: 11,
                scope_state: 0,
            },
            StateDynamicModifierRow {
                state: 10,
                ordinal: 1,
                token: b"refinery_damaged_modifier".to_vec(),
                scope_country: 0,
                scope_state: 0,
            },
        ],
    );
    let damaged = "has_dynamic_modifier = { modifier = refinery_damaged_modifier }";
    assert_eq!(on_state(&assets, &frame, damaged), script::Eval::False);
    put::<tables::StateDynamicModifiers>(
        &mut frame,
        vec![StateDynamicModifierRow {
            state: 10,
            ordinal: 0,
            token: b"refinery_damaged_modifier".to_vec(),
            scope_country: 0,
            scope_state: 0,
        }],
    );
    assert_eq!(on_state(&assets, &frame, damaged), script::Eval::True);
}

#[test]
fn controller_overrides_replace_building_grants_by_their_first_entry() {
    let assets = resource_assets();
    let mut frame = resource_frame();
    let override_row = |ordinal, entry, count, amount| CountryBuildingResourceRow {
        country: 10,
        ordinal,
        entry,
        building: REFINERY,
        count,
        resource: if count == 0 { 0 } else { OIL },
        amount,
    };
    put::<tables::CountryBuildingResources>(&mut frame, vec![override_row(0, 0, 1, -300_000)]);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(world.state_resource(10, OIL), Ok(4_002_000));
    drop(world);
    put::<tables::CountryBuildingResources>(
        &mut frame,
        vec![override_row(0, 0, 0, 0), override_row(1, 1, 1, -300_000)],
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(world.state_resource(10, OIL), Ok(4_402_000));
}

#[test]
fn missing_inputs_and_unsupported_forms_stay_unknown() {
    content_reader_script::test_game::enter_viewer();
    let assets = resource_assets();
    let mut frame = resource_frame();
    assert!(matches!(
        on_state(&assets, &frame, "oil >= 3"),
        script::Eval::Unknown(_)
    ));
    *frame.tables[TableId::StateResources.index()] = TableView::Unavailable {
        reason: "short".into(),
    };
    assert!(matches!(
        on_state(&assets, &frame, "oil > 3"),
        script::Eval::Unknown(_)
    ));
    let frame = resource_frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert!(
        world
            .evaluate(&parse("oil > 3"), Scopes::of(Scope::State(20)))
            .ne(&script::Eval::True)
    );
}

#[test]
fn every_content_building_type_is_a_count_trigger() {
    content_reader_script::test_game::enter_viewer();
    let mut assets = resource_assets();
    assets.trigger_content.buildings.types.insert(
        "dock".into(),
        BuildingType {
            name: "dock".into(),
            province_max: Some(10),
            ..Default::default()
        },
    );
    set_counts(&mut assets);
    let mut frame = resource_frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    tokens.push(LexicalTokenRow {
        token: 303,
        literal: b"dock".to_vec(),
    });
    put::<tables::LexicalTokens>(&mut frame, tokens);
    put::<tables::ProvinceBuildings>(
        &mut frame,
        [(1, 2), (2, 3), (3, 7)]
            .map(|(location, level)| BuildingRow {
                location,
                building_type: 303,
                level,
                healthy_level: 1,
                ..Default::default()
            })
            .into(),
    );
    assert_eq!(on_state(&assets, &frame, "dock > 4"), script::Eval::True);
    assert_eq!(on_state(&assets, &frame, "dock > 5"), script::Eval::False);
    assert_eq!(on_state(&assets, &frame, "dock = 5"), script::Eval::True);
    assert_eq!(
        on_state(&assets, &frame, "refinery < 4"),
        script::Eval::True
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(
        world.evaluate(&parse("dock = 5"), Scopes::of(Scope::Country(0))),
        script::Eval::True
    );
    assert_eq!(
        world.evaluate(&parse("dock = 7"), Scopes::of(Scope::Country(1))),
        script::Eval::True
    );
    drop(world);
    assets.trigger_content.buildings.types.remove("dock");
    set_counts(&mut assets);
    assert_eq!(on_state(&assets, &frame, "dock > 4"), script::Eval::True);
    scripted(
        &mut assets,
        "refinery = { always = no } is_coastal = { always = yes }",
    );
    assert_eq!(
        on_state(&assets, &frame, "refinery < 4"),
        script::Eval::True
    );
    assert!(matches!(
        on_state(&assets, &frame, "is_coastal = yes"),
        script::Eval::Unknown(_)
    ));
}

#[test]
fn exists_reads_the_owned_state_count() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::rows::CountryCounterInputRow;
    let assets = assets();
    let mut frame = positive_frame();
    let exists = |frame: &DecodedFrame, slot| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse("exists = yes"), Scopes::of(Scope::Country(slot)))
    };
    put::<tables::CountryCounterInputs>(&mut frame, Vec::new());
    assert!(matches!(exists(&frame, 2), script::Eval::Unknown(_)));
    put::<tables::CountryCounterInputs>(
        &mut frame,
        vec![
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 10,
                owned_state_count: 2,
                ..Default::default()
            },
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 11,
                owned_state_count: 0,
                ..Default::default()
            },
        ],
    );
    assert_eq!(exists(&frame, 2), script::Eval::True);
    assert_eq!(exists(&frame, 1), script::Eval::False);
}

#[test]
fn a_daily_fact_decides_from_its_latest_recorded_rows() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let mut frame = frame();
    put::<tables::CompletedFocuses>(
        &mut frame,
        vec![replay_data::rows::CompletedFocusRow {
            country: 10,
            ordinal: 0,
            token: b"raid_focus".to_vec(),
        }],
    );
    let eval = |frame: &DecodedFrame| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(
            &parse("has_completed_focus = raid_focus"),
            Scopes::of(Scope::Country(0)),
        )
    };
    assert_eq!(eval(&frame), script::Eval::True);
}

#[test]
fn a_memo_row_found_from_a_hint_is_the_binary_searchs() {
    let rows: Vec<replay_data::rows::RaidPathDistanceRow> = (1..=20)
        .map(|at| replay_data::rows::RaidPathDistanceRow {
            dispatcher: replay_data::rows::RaidPathDispatcher::Naval,
            source: 7,
            target: at * 2,
            distance: 1,
            ok: 1,
        })
        .collect();
    for hint in 0..25 {
        for to in 0..=42 {
            assert_eq!(
                launch::target_row(&rows, to, hint),
                rows.binary_search_by_key(&to, |row| row.target).ok(),
                "hint {hint} target {to}"
            );
        }
    }
    assert_eq!(launch::target_row(&[], 4, 0), None);
}

#[test]
fn if_else_chains_take_the_first_branch_whose_limit_holds() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let frame = positive_frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let eval = |source: &str| world.evaluate(&parse(source), Scopes::of(Scope::Country(2)));
    assert_eq!(
        eval("if = { limit = { tag = GER } always = yes } else = { always = no }"),
        script::Eval::True
    );
    assert_eq!(
        eval(
            "if = { limit = { tag = ENG } always = yes } else_if = { limit = { tag = GER } always = no } else = { always = yes }"
        ),
        script::Eval::False
    );
    assert_eq!(
        eval("if = { limit = { tag = ENG } always = no }"),
        script::Eval::True,
        "no branch taken holds"
    );
    assert!(matches!(
        eval("if = { limit = { is_ai = yes } always = no } else = { always = yes }"),
        script::Eval::Unknown(_)
    ));
    assert_eq!(
        eval("if = { limit = { synthetic_unported_trigger = yes } always = yes } else = { always = yes }"),
        script::Eval::True,
        "both branches agree"
    );
}

#[test]
fn from_and_prev_follow_the_scope_chain() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let frame = positive_frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let raid = Scopes {
        from: Some(Scope::Country(1)),
        ..Scopes::of(Scope::Country(2))
    };
    let eval = |source: &str, scopes| world.evaluate(&parse(source), scopes);
    assert_eq!(eval("FROM = { tag = ENG }", raid), script::Eval::True);
    assert_eq!(eval("tag = FROM", raid), script::Eval::False);
    assert_eq!(
        eval("ENG = { PREV = { tag = GER } }", raid),
        script::Eval::True
    );
    assert_eq!(eval("ENG = { tag = PREV }", raid), script::Eval::False);
    assert_eq!(
        eval("ENG = { GER = { tag = PREV } }", raid),
        script::Eval::False
    );
    assert_eq!(
        eval("ENG = { GER = { PREV = { tag = ENG } } }", raid),
        script::Eval::True
    );
    let bare = Scopes::of(Scope::Country(2));
    assert_eq!(eval("FROM = { always = yes }", bare), script::Eval::False);
    assert_eq!(eval("PREV = { always = yes }", bare), script::Eval::False);
}

#[test]
fn a_state_scope_from_resolves_to_the_state_owner() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let frame = frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let from_self = Scopes {
        from: Some(Scope::State(20)),
        ..Scopes::of(Scope::State(20))
    };
    assert_eq!(
        world.evaluate(&parse("is_owned_by = FROM"), from_self),
        script::Eval::True
    );
}

fn spelled(token: u32, literal: &str) -> DecodedFrame {
    let mut frame = frame();
    let mut rows = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    rows.retain(|row| row.token != token);
    rows.push(LexicalTokenRow {
        token,
        literal: literal.as_bytes().to_vec(),
    });
    rows.sort_by_key(|row| row.token);
    put::<tables::LexicalTokens>(&mut frame, rows);
    frame
}

#[test]
fn raid_type_tokens_match_under_the_lexers_fold() {
    let assets = assets();
    let frame = spelled(OILFIELD, "OilField");
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(assets.raid_type(&world, OILFIELD), Ok(TypeToken::Type(0)));
    let frame = spelled(OILFIELD, "oilfiEld\u{130}");
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert!(assets.raid_type(&world, OILFIELD).is_err());
    let mut frame = spelled(OILFIELD, "OILFIELD");
    put::<tables::Raids>(&mut frame, vec![raid(1, 0, 0, 2)]);
    put::<tables::RaidTargets>(&mut frame, vec![target(1, 3, 0)]);
    let drawn = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert_eq!(
        drawn.icons[0].glyph.as_deref(),
        Some("GFX_raid_type_icon_oilfield")
    );
    assert!(drawn.report.refused_tokens.is_empty());
}

#[test]
fn ambiguous_and_unknown_raid_tokens_are_refused_and_counted() {
    let mut assets = assets();
    assets
        .catalog
        .types
        .push(kind("OILFIELD", TargetType::Provinces(vec![9])));
    let frame = frame();
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let ambiguous = assets.raid_type(&world, OILFIELD).unwrap_err();
    assert!(ambiguous.contains("oilfield") && ambiguous.contains("OILFIELD"));
    let assets = self::assets();
    let unknown = spelled(OILFIELD, "no_such_raid");
    let world = World::new(&unknown, assets.trigger_content.facts()).unwrap();
    assert!(
        assets
            .raid_type(&world, OILFIELD)
            .unwrap_err()
            .contains("no_such_raid is not in the content")
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert!(
        assets
            .raid_type(&world, 999)
            .unwrap_err()
            .contains("has no recorded literal")
    );
    let mut frame = spelled(OILFIELD, "no_such_raid");
    put::<tables::Raids>(&mut frame, vec![raid(1, 0, 0, 2), raid(2, 0, 1, 2)]);
    put::<tables::RaidTargets>(&mut frame, vec![target(1, 3, 0), target(2, 4, 0)]);
    let drawn = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert!(drawn.icons.is_empty());
    assert_eq!(drawn.report.withheld, 2);
    assert_eq!(
        drawn.report.refused_tokens,
        Refusals::from([("raid type no_such_raid is not in the content".to_owned(), 2)])
    );
    assert!(drawn.report.warnings().iter().any(|line| line
        == "Raid icons: 2 refused tokens: raid type no_such_raid is not in the content."));
}

#[test]
fn a_refused_used_type_leaves_fire_only_once_types_undecided() {
    let mut assets = assets();
    assets.catalog.types[0].fire_only_once = true;
    let mut frame = frame();
    let mut countries = frame.complete::<tables::RaidCountries>().unwrap().to_vec();
    countries[0].used_types = vec![999];
    put::<tables::RaidCountries>(&mut frame, countries);
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    let drawn = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert!(drawn.icons.is_empty(), "province 4's only type is oilfield");
    assert_eq!(drawn.report.withheld, 1);
    assert!(drawn.report.reasons.keys().any(|reason| {
        reason.contains("a used raid type may be this one") && reason.contains("999")
    }));
}

#[test]
fn building_template_tokens_match_under_the_fold_or_are_refused() {
    let mut assets = assets();
    assets.catalog.types[0].target_types = vec![TargetType::Building(
        content_reader_raids::raids::BuildingSelector {
            types: vec!["dam".into()],
            ..Default::default()
        },
    )];
    let target = |literal: &str| {
        let mut frame = spelled(DAM, literal);
        put::<tables::RaidTargetStatuses>(
            &mut frame,
            vec![RaidTargetStatusRow {
                scope: RaidScope {
                    building_type: DAM,
                    building_location: 4,
                    building_flags: 1,
                    ..Default::default()
                },
                detected: vec![1],
                ..status(0, 0)
            }],
        );
        icons(&assets, &frame, 0, &launch::Unreconstructed)
    };
    let folded = target("DAM");
    assert_eq!(folded.icons.len(), 1);
    assert!(folded.report.refused_tokens.is_empty());
    let unknown = target("no_such_building");
    assert!(unknown.icons.is_empty());
    assert_eq!(unknown.report.withheld, 1);
    assert_eq!(
        unknown.report.refused_tokens,
        Refusals::from([(
            "building template no_such_building is not an authored building type".to_owned(),
            1
        )])
    );
}

mod launch_gate {
    use super::*;
    use content_reader_raids::{
        raids::EquipmentRequirement, raids::SourceKind, raids::StartingPoint,
        raids::UnitRequirement,
    };
    use replay_data::{
        rows::AirBaseRow, rows::AirWingRow, rows::DivisionRow, rows::DivisionTemplateIconRow,
        rows::HumanControllerRow, rows::NukeRow, rows::RaidAirSiteRow, rows::RaidCountryInputRow,
        rows::RaidSource, rows::SimulationFeatureRow, rows::TemplateSubunit,
        rows::VariantDefinitionRow, rows::WingEquipmentRow,
    };

    const LARGE: u32 = 300;
    const MARINE: u32 = 301;
    const BASE: u64 = 0x0000_0001_0000_0041;
    const WING: u64 = 0x0000_0002_0000_0045;
    const VARIANT: u64 = 0x0000_0003_0000_0046;
    const DIVISION: u64 = 0x0000_0004_0000_0006;
    const TEMPLATE: u64 = 0x0000_0005_0000_0034;

    fn launch_assets() -> RaidAssets {
        let mut assets = assets();
        assets.equipment = content_reader_key_names::equipment::fixture::load(&[(
            "common/units/equipment/a.txt",
            "equipments = { large_plane_airframe = { is_archetype = yes type = strategic_bomber } convoy = { is_archetype = yes type = convoy } }",
        )])
        .0;
        assets.defines = RaidDefines {
            base_max_command_power: 100 * 100_000,
            naval_invasion_priority: 1,
            map_scale: 711_400,
            naval_commando_distance: 1500 * 100_000,
            ace_wing_size: 100 * 100_000,
            ace_wing_size_max_bonus: 300_000,
            unit_speeds: None,
        };
        assets
    }

    fn bomber() -> Kind {
        Kind {
            command_power_raw: 20 * 100_000,
            unit_requirements: vec![UnitRequirement {
                equipment: vec![EquipmentRequirement {
                    types: vec!["strategic_bomber".into()],
                    min: Some(10),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            starting_point: StartingPoint {
                types: vec![SourceKind::AirBase],
                ..Default::default()
            },
            ..kind("oilfield", TargetType::Provinces(vec![3, 4]))
        }
    }

    fn launch_frame() -> DecodedFrame {
        let mut frame = frame();
        put::<tables::Countries>(
            &mut frame,
            vec![
                CountryRow {
                    id: 10,
                    canonical_slot: 0,
                    tag: b"GER".to_vec(),
                    command_power_raw: 50 * 100_000,
                    convoys_raw: Some(30 * 100_000),
                    ..Default::default()
                },
                CountryRow {
                    id: 11,
                    canonical_slot: 1,
                    tag: b"ENG".to_vec(),
                    ..Default::default()
                },
            ],
        );
        put::<tables::RaidCountryInputs>(
            &mut frame,
            (0..2)
                .map(|slot| RaidCountryInputRow {
                    canonical_slot: slot,
                    autonomous: Some(0),
                    convoy_reservations: vec![1, 20, 2, 5],
                    ..registered_launch_inputs()
                })
                .collect(),
        );
        put::<tables::CountryCommandPower>(&mut frame, registered_command_power(0..2));
        put::<tables::HumanControllers>(
            &mut frame,
            vec![HumanControllerRow {
                country: 10,
                count: 1,
            }],
        );
        put::<tables::SimulationFeatures>(&mut frame, vec![SimulationFeatureRow::default()]);
        put::<tables::Nukes>(
            &mut frame,
            (0..2)
                .map(|nuke_type| NukeRow {
                    country: 10,
                    nuke_type,
                    ..Default::default()
                })
                .collect(),
        );
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.extend([(LARGE, "large_plane_airframe"), (MARINE, "marine")].map(
            |(token, name)| LexicalTokenRow {
                token,
                literal: name.as_bytes().to_vec(),
            },
        ));
        put::<tables::LexicalTokens>(&mut frame, tokens);
        put::<tables::AirBases>(
            &mut frame,
            vec![AirBaseRow {
                id: BASE,
                state: Some(10),
                province: Some(1),
                ..Default::default()
            }],
        );
        put::<tables::RaidAirSites>(
            &mut frame,
            vec![RaidAirSiteRow {
                canonical_slot: 0,
                ordinal: 0,
                list: 0,
                base: BASE,
                container_capacity_raw: Some(100_000),
            }],
        );
        put::<tables::AirWings>(
            &mut frame,
            vec![AirWingRow {
                id: WING,
                owner: 10,
                air_base: BASE,
                planes: 50,
                base_range_raw: 1000 * 100_000,
                ..Default::default()
            }],
        );
        put::<tables::WingEquipment>(
            &mut frame,
            vec![WingEquipmentRow {
                wing: WING,
                ordinal: 0,
                variant: VARIANT,
                amount_raw: 20 * 100_000,
            }],
        );
        put::<tables::VariantDefinitions>(
            &mut frame,
            vec![VariantDefinitionRow {
                id: VARIANT,
                archetype_token: LARGE,
                ..Default::default()
            }],
        );
        frame
    }

    fn gate(assets: &RaidAssets, frame: &DecodedFrame, kind: &Kind, province: u32) -> Eval {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let launcher = launch::Launcher::new(assets, &world, 0, &launch::Unreconstructed);
        let scope = RaidScope {
            province,
            ..Default::default()
        };
        launcher.launchable(kind, &scope, Scopes::of(script::Scope::Country(0)))
    }

    struct Reaching;

    impl launch::RaidPaths for Reaching {
        fn distance(
            &self,
            _: &World<'_>,
            _: &launch::PathSource,
            _: &RaidScope,
        ) -> Result<Option<i64>, String> {
            Err("no path search".into())
        }
        fn note(&self, _: &launch::PathSource, _: &RaidScope) {}
        fn recorded_rows(&self) -> Option<&[replay_data::rows::RaidPathDistanceRow]> {
            None
        }
        fn path_target(&self, _: u8, _: &RaidScope) -> Result<Option<u32>, String> {
            Err("no path search".into())
        }
        fn army_reaches(&self, _: &World<'_>, _: u64, _: u32) -> Eval {
            Eval::True
        }
    }

    fn airborne(values: [Option<i64>; 10]) -> Eval {
        use replay_data::{
            rows::StockpileArchetypeRow, rows::StockpileRow, rows::VariantDefinitionRow,
            rows::VariantStatsRow,
        };
        const PARATROOPER: u32 = 310;
        const TRANSPORT: u32 = 311;
        const TRANSPORT_VARIANT: u64 = 0x0000_0008_0000_0046;
        let mut assets = launch_assets();
        assets.equipment = content_reader_key_names::equipment::fixture::load(&[(
            "common/units/equipment/a.txt",
            "equipments = { transport_plane = { is_archetype = yes type = air_transport } }",
        )])
        .0;
        let kind = Kind {
            command_power_raw: 20 * 100_000,
            essential_equipment: vec![("transport_plane".into(), 1)],
            unit_requirements: vec![UnitRequirement {
                battalions: vec![content_reader_raids::raids::BattalionRequirement {
                    name: "paratrooper".into(),
                    min: Some(1),
                    max: None,
                }],
                ..Default::default()
            }],
            starting_point: StartingPoint {
                types: vec![SourceKind::AirBase],
                ..Default::default()
            },
            ..kind("airborne", TargetType::Provinces(vec![3]))
        };
        let mut frame = launch_frame();
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.extend(
            [(PARATROOPER, "paratrooper"), (TRANSPORT, "transport_plane")].map(|(token, name)| {
                LexicalTokenRow {
                    token,
                    literal: name.as_bytes().to_vec(),
                }
            }),
        );
        put::<tables::LexicalTokens>(&mut frame, tokens);
        put::<tables::Divisions>(
            &mut frame,
            vec![DivisionRow {
                id: DIVISION,
                roster_owner: 10,
                logical_owner: 10,
                province: 1,
                template: TEMPLATE,
                ..Default::default()
            }],
        );
        put::<tables::DivisionTemplateIcons>(
            &mut frame,
            vec![DivisionTemplateIconRow {
                template: TEMPLATE,
                subunits: vec![TemplateSubunit {
                    token: PARATROOPER,
                    local: 0,
                    count: 1,
                }],
                ..Default::default()
            }],
        );
        put::<tables::StockpileArchetypes>(
            &mut frame,
            vec![StockpileArchetypeRow {
                canonical_slot: 0,
                archetype: TRANSPORT,
                units: 10,
            }],
        );
        put::<tables::Stockpiles>(
            &mut frame,
            vec![StockpileRow {
                country: 10,
                ordinal: 0,
                variant_handle: TRANSPORT_VARIANT,
                amount_raw: 10 * 100_000,
            }],
        );
        put::<tables::VariantDefinitions>(
            &mut frame,
            vec![VariantDefinitionRow {
                id: TRANSPORT_VARIANT,
                archetype_token: TRANSPORT,
                classifier_flags: 1 << 20,
                ..Default::default()
            }],
        );
        put::<tables::VariantStats>(
            &mut frame,
            vec![VariantStatsRow {
                variant: TRANSPORT_VARIANT,
                values,
                ..Default::default()
            }],
        );
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let launcher = launch::Launcher::new(&assets, &world, 0, &Reaching);
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        launcher.launchable(&kind, &scope, Scopes::of(script::Scope::Country(0)))
    }

    #[test]
    fn transport_range_reads_the_first_variant_stat() {
        content_reader_script::test_game::enter_viewer();
        let mut long_first = [Some(1); 10];
        long_first[0] = Some(100 * 100_000);
        assert_eq!(airborne(long_first), Eval::True);
        let mut long_second = [Some(1); 10];
        long_second[1] = Some(100 * 100_000);
        assert_eq!(airborne(long_second), Eval::False);
        let mut withheld = [Some(100 * 100_000); 10];
        withheld[0] = None;
        match airborne(withheld) {
            Eval::Unknown(reasons) => {
                assert!(
                    reasons.iter().any(|reason| reason.contains("air_range")),
                    "{reasons:?}"
                );
            }
            other => panic!("withheld air_range must stay unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_lost_equipment_database_leaves_only_equipment_queries_unknown() {
        content_reader_script::test_game::enter_viewer();
        let mut assets = launch_assets();
        assert_eq!(
            assets.equipment_category("convoy"),
            Ok(content_reader_script::test_game::category("convoy")),
        );
        assets.equipment_lost = Some("the equipment database cannot be followed past x".into());
        let unknown = assets.equipment_category("convoy").unwrap_err();
        assert!(unknown.contains("followed past x"), "{unknown}");
        assert!(assets.equipment().is_err());
        let lines = tooltips::requirements::build(
            &Default::default(),
            assets.equipment(),
            &bomber(),
            |name| Ok(name.into()),
            |_| Ok(true),
        );
        assert!(lines[0].1.as_ref().unwrap_err().contains("followed past x"));
    }

    #[test]
    fn a_refused_cooldown_type_leaves_its_target_undecided() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::RaidCooldowns>(
            &mut frame,
            vec![RaidCooldownRow {
                days: 3,
                owner_slot: 0,
                ordinal: 0,
                raid_type: 999,
                scope: RaidScope {
                    province: 3,
                    ..Default::default()
                },
            }],
        );
        match gate(&assets, &frame, &bomber(), 3) {
            Eval::Unknown(reasons) => assert!(
                reasons
                    .iter()
                    .any(|reason| reason.contains("raid type token 999"))
            ),
            other => panic!("expected undecided, got {other:?}"),
        }
        assert_eq!(gate(&assets, &frame, &bomber(), 4), Eval::True);
    }

    #[test]
    fn a_bomber_wing_at_an_air_base_in_range_makes_the_offer_launchable() {
        content_reader_script::test_game::enter_viewer();
        assert_eq!(
            gate(&launch_assets(), &launch_frame(), &bomber(), 3),
            Eval::True
        );
    }

    #[test]
    fn a_requirement_naming_an_equipment_group_admits_through_its_members() {
        content_reader_script::test_game::enter_viewer();
        let mut assets = launch_assets();
        let (_, groups) = content_reader_key_names::equipment::fixture::load(&[(
            "common/equipment_groups/raids.txt",
            "heavy = { equipment_type = { Large_Plane_Airframe } }
             by_category = { equipment_type = { STRATEGIC_BOMBER } }
             ships = { equipment_type = { convoy } }",
        )]);
        assets.equipment_groups = groups;
        let with = |group: &str| {
            let mut kind = bomber();
            kind.unit_requirements[0].equipment[0].types = vec![group.to_owned()];
            kind
        };
        for group in ["HEAVY", "by_category"] {
            assert_eq!(
                gate(&assets, &launch_frame(), &with(group), 3),
                Eval::True,
                "{group}"
            );
        }
        assert_eq!(
            gate(&assets, &launch_frame(), &with("ships"), 3),
            Eval::False
        );
    }

    #[test]
    fn command_power_below_the_cost_or_the_free_capacity_disables_the_offer() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut kind = bomber();
        kind.command_power_raw = 60 * 100_000;
        let mut frame = launch_frame();
        put::<tables::CountryCommandPower>(&mut frame, Vec::new());
        assert_eq!(gate(&assets, &frame, &kind, 3), Eval::False);
        let mut frame = launch_frame();
        put::<tables::CountryCommandPower>(
            &mut frame,
            vec![replay_data::rows::CountryCommandPowerRow {
                allocated_command_power_raw: 90 * 100_000,
                ..registered_command_power([0])[0]
            }],
        );
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::False);
    }

    #[test]
    fn an_unregistered_capacity_modifier_leaves_the_command_power_gate_unknown() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::CountryCommandPower>(
            &mut frame,
            vec![replay_data::rows::CountryCommandPowerRow {
                max_command_power: None,
                ..registered_command_power([0])[0]
            }],
        );
        let Eval::Unknown(reasons) = gate(&assets, &frame, &bomber(), 3) else {
            panic!("an unregistered key is unknown")
        };
        assert!(
            reasons.contains("max_command_power is not a registered modifier"),
            "{reasons:?}"
        );
    }

    #[test]
    fn convoys_count_after_reservations_up_to_the_invasion_priority() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut kind = bomber();
        kind.essential_equipment = vec![("convoy".into(), 10)];
        assert_eq!(gate(&assets, &launch_frame(), &kind, 3), Eval::True);
        kind.essential_equipment = vec![("convoy".into(), 11)];
        assert_eq!(gate(&assets, &launch_frame(), &kind, 3), Eval::False);
    }

    #[test]
    fn an_assigned_small_or_unmatched_wing_is_not_a_raid_unit() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::Raids>(
            &mut frame,
            vec![RaidRow {
                air_wing: WING,
                phase: 2,
                ..raid(9, 0, 0, 2)
            }],
        );
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::False);
        let mut small = launch_frame();
        put::<tables::WingEquipment>(
            &mut small,
            vec![WingEquipmentRow {
                wing: WING,
                ordinal: 0,
                variant: VARIANT,
                amount_raw: 9 * 100_000,
            }],
        );
        assert_eq!(gate(&assets, &small, &bomber(), 3), Eval::False);
        let mut fighter = bomber();
        fighter.unit_requirements[0].equipment[0].types = vec!["fighter".into()];
        assert_eq!(gate(&assets, &launch_frame(), &fighter, 3), Eval::False);
        let mut none = bomber();
        none.unit_requirements.clear();
        assert_eq!(gate(&assets, &launch_frame(), &none, 3), Eval::False);
    }

    #[test]
    fn range_and_max_distance_bound_the_source_unless_the_ai_is_unlimited() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        let mut wings = frame.complete::<tables::AirWings>().unwrap().to_vec();
        wings[0].base_range_raw = 100_000;
        put::<tables::AirWings>(&mut frame, wings);
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::False);
        let mut unlimited = bomber();
        unlimited.unlimited_ai_range = true;
        assert_eq!(gate(&assets, &frame, &unlimited, 3), Eval::False);
        put::<tables::HumanControllers>(
            &mut frame,
            vec![HumanControllerRow {
                country: 10,
                count: 0,
            }],
        );
        let mut inputs = frame
            .complete::<tables::RaidCountryInputs>()
            .unwrap()
            .to_vec();
        inputs[0].autonomous = Some(1);
        put::<tables::RaidCountryInputs>(&mut frame, inputs);
        assert_eq!(gate(&assets, &frame, &unlimited, 3), Eval::True);
        let mut capped = bomber();
        capped.max_distance_raw = Some(1);
        assert_eq!(gate(&assets, &launch_frame(), &capped, 3), Eval::False);
    }

    #[test]
    fn own_sources_demand_faction_members_only_on_native_faction_branches() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::FactionMembers>(
            &mut frame,
            vec![FactionMemberRow {
                country: 0,
                faction: 7,
                ordinal: 0,
            }],
        );
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);
        *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
            reason: "unreached faction roster refused".into(),
        };
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);

        let mut buildings = bomber();
        buildings.starting_point.building_types = vec!["dam".into()];
        assert_eq!(gate(&assets, &frame, &buildings, 3), Eval::True);
        buildings.starting_point.types.clear();
        buildings.starting_point.allow_faction_buildings = true;
        for kind in [
            buildings,
            Kind {
                starting_point: StartingPoint {
                    types: vec![SourceKind::NavalBase],
                    ..Default::default()
                },
                ..bomber()
            },
        ] {
            match gate(&assets, &frame, &kind, 3) {
                Eval::Unknown(reasons) => assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("FactionMembers"))
                ),
                other => panic!("expected the reached faction roster refusal, got {other:?}"),
            }
        }
    }

    fn marine_sources(frame: &mut DecodedFrame, naval: bool) -> Kind {
        put::<tables::Divisions>(
            frame,
            vec![DivisionRow {
                id: DIVISION,
                roster_owner: 10,
                province: 1,
                template: TEMPLATE,
                ..Default::default()
            }],
        );
        put::<tables::DivisionTemplateIcons>(
            frame,
            vec![DivisionTemplateIconRow {
                template: TEMPLATE,
                subunits: vec![TemplateSubunit {
                    token: MARINE,
                    local: 0,
                    count: 2,
                }],
                ..Default::default()
            }],
        );
        Kind {
            starting_point: StartingPoint {
                types: if naval {
                    vec![SourceKind::NavalBase]
                } else {
                    Vec::new()
                },
                building_types: if naval {
                    Vec::new()
                } else {
                    vec!["dam".into()]
                },
                allow_faction_buildings: true,
            },
            unit_requirements: vec![UnitRequirement {
                battalions: vec![content_reader_raids::raids::BattalionRequirement {
                    name: "marine".into(),
                    min: Some(2),
                    max: None,
                }],
                ..Default::default()
            }],
            ..bomber()
        }
    }

    fn launch_zero(
        frame: &mut DecodedFrame,
        count: i32,
        bases: Option<Vec<replay_data::rows::LaunchNavalBase>>,
        states: Option<Vec<u32>>,
    ) {
        use replay_data::{rows::Recorded, rows::ReservedCountryNamingRow, rows::ReservedLaunchSources};
        let hour = frame.hour;
        put::<tables::ReservedCountryNaming>(
            frame,
            vec![ReservedCountryNamingRow {
                faction: Recorded::Value(Some(7)),
                owned_state_count: Recorded::Value(Some(count)),
                launch_sources: Recorded::Value(Some(ReservedLaunchSources {
                    sample_hour: replay_data::rows::PacketHour::Stamped(hour),
                    naval_bases: bases.map_or(
                        replay_data::rows::LaunchNavalBases::Unavailable,
                        replay_data::rows::LaunchNavalBases::Bases,
                    ),
                    owned_states: states,
                })),
                ..Default::default()
            }],
        );
        put::<tables::FactionMembers>(
            frame,
            vec![
                FactionMemberRow {
                    faction: 7,
                    country: 0,
                    ordinal: 0,
                },
                FactionMemberRow {
                    faction: 7,
                    country: 10,
                    ordinal: 1,
                },
            ],
        );
    }

    #[derive(Default)]
    struct SourceVisits {
        distances: Mutex<Vec<(u8, u32)>>,
        reaches: Mutex<Vec<u32>>,
        notes: Mutex<Vec<u32>>,
        usable: Option<u32>,
    }

    impl launch::RaidPaths for SourceVisits {
        fn distance(
            &self,
            _: &World<'_>,
            source: &launch::PathSource,
            _: &RaidScope,
        ) -> Result<Option<i64>, String> {
            self.distances
                .lock()
                .unwrap()
                .push((source.kind, source.province));
            Ok(Some(0))
        }
        fn note(&self, source: &launch::PathSource, _: &RaidScope) {
            self.notes.lock().unwrap().push(source.province);
        }
        fn recorded_rows(&self) -> Option<&[replay_data::rows::RaidPathDistanceRow]> {
            None
        }
        fn path_target(&self, _: u8, _: &RaidScope) -> Result<Option<u32>, String> {
            panic!("these fixtures do not supply a recorded path memo")
        }
        fn army_reaches(&self, _: &World<'_>, _: u64, province: u32) -> Eval {
            self.reaches.lock().unwrap().push(province);
            Eval::from_bool(self.usable == Some(province))
        }
    }

    fn sources_gate(
        assets: &RaidAssets,
        frame: &DecodedFrame,
        kind: &Kind,
        paths: &SourceVisits,
    ) -> Eval {
        let world = World::read(frame, assets.trigger_content.facts());
        let launcher = launch::Launcher::new(assets, &world, 0, paths);
        launcher.launchable(
            kind,
            &RaidScope {
                province: 3,
                ..Default::default()
            },
            Scopes::of(script::Scope::Country(0)),
        )
    }

    #[test]
    fn mixed_air_success_defers_later_faction_and_prefetch_preserves_it() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
            reason: "later faction roster refused".into(),
        };
        for buildings in [false, true] {
            let mut kind = bomber();
            if buildings {
                kind.starting_point.building_types = vec!["dam".into()];
                kind.starting_point.allow_faction_buildings = true;
            } else {
                kind.starting_point.types.push(SourceKind::NavalBase);
            }
            assert_eq!(gate(&assets, &frame, &kind, 3), Eval::True);
            let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
            let paths = SourceVisits::default();
            let launcher = launch::Launcher::new(&assets, &world, 0, &paths);
            let hour = paths::HourPaths::new(&assets, &frame);
            launcher.prefetch(&[&kind], &hour, &Workers::default());
            assert_eq!(
                launcher.launchable(
                    &kind,
                    &RaidScope {
                        province: 3,
                        ..Default::default()
                    },
                    Scopes::of(script::Scope::Country(0)),
                ),
                Eval::True
            );
            assert!(paths.distances.lock().unwrap().is_empty());
            assert!(paths.notes.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn mixed_air_failure_reaches_the_later_faction_refusal() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
            reason: "reached faction roster refused".into(),
        };
        let mut kind = bomber();
        kind.max_distance_raw = Some(0);
        kind.starting_point.types.push(SourceKind::NavalBase);
        assert!(
            matches!(gate(&assets, &frame, &kind, 3), Eval::Unknown(reasons)
            if reasons.iter().any(|reason| reason.contains("FactionMembers")))
        );
    }

    #[test]
    fn mixed_earlier_unknown_still_queries_the_later_source() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::rows::NavalBaseRow;
        let assets = launch_assets();
        let mut frame = launch_frame();
        let mut kind = marine_sources(&mut frame, true);
        kind.starting_point.types.insert(0, SourceKind::AirBase);
        put::<tables::FactionMembers>(
            &mut frame,
            vec![FactionMemberRow {
                faction: 7,
                country: 10,
                ordinal: 0,
            }],
        );
        put::<tables::NavalBases>(
            &mut frame,
            vec![NavalBaseRow {
                province: 2,
                country: 10,
                ..Default::default()
            }],
        );
        *frame.tables[TableId::AirBases.index()] = TableView::Unavailable {
            reason: "earlier air source census refused".into(),
        };
        let paths = SourceVisits {
            usable: Some(2),
            ..Default::default()
        };
        assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::True);
        assert_eq!(*paths.distances.lock().unwrap(), vec![(2, 2)]);
        let paths = SourceVisits::default();
        assert!(matches!(
            sources_gate(&assets, &frame, &kind, &paths),
            Eval::Unknown(_)
        ));
        assert_eq!(*paths.distances.lock().unwrap(), vec![(2, 2)]);
    }

    #[test]
    fn mixed_naval_success_defers_later_building_inputs() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::rows::NavalBaseRow;
        let assets = launch_assets();
        let mut frame = launch_frame();
        let mut kind = marine_sources(&mut frame, true);
        kind.starting_point.building_types = vec!["dam".into()];
        put::<tables::FactionMembers>(
            &mut frame,
            vec![FactionMemberRow {
                faction: 7,
                country: 10,
                ordinal: 0,
            }],
        );
        put::<tables::NavalBases>(
            &mut frame,
            vec![NavalBaseRow {
                province: 2,
                country: 10,
                ..Default::default()
            }],
        );
        *frame.tables[TableId::States.index()] = TableView::Unavailable {
            reason: "unreached building states refused".into(),
        };
        *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Unavailable {
            reason: "unreached buildings refused".into(),
        };
        let paths = SourceVisits {
            usable: Some(2),
            ..Default::default()
        };
        assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::True);
        assert_eq!(*paths.distances.lock().unwrap(), vec![(2, 2)]);
    }

    #[test]
    fn mixed_naval_refusal_can_reach_independent_own_buildings() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        let mut kind = marine_sources(&mut frame, true);
        kind.starting_point.building_types = vec!["dam".into()];
        kind.starting_point.allow_faction_buildings = false;
        *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
            reason: "naval faction roster refused".into(),
        };
        put::<tables::ProvinceBuildings>(
            &mut frame,
            vec![BuildingRow {
                location: 1,
                building_type: DAM,
                ..Default::default()
            }],
        );
        let paths = SourceVisits {
            usable: Some(1),
            ..Default::default()
        };
        assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::True);
        assert_eq!(*paths.distances.lock().unwrap(), vec![(5, 1)]);
        kind.starting_point.allow_faction_buildings = true;
        let paths = SourceVisits {
            usable: Some(1),
            ..Default::default()
        };
        assert!(
            matches!(sources_gate(&assets, &frame, &kind, &paths), Eval::Unknown(reasons)
            if reasons.iter().any(|reason| reason.contains("FactionMembers")))
        );
        assert!(paths.distances.lock().unwrap().is_empty());
    }

    #[test]
    fn actual_zero_naval_occurrences_keep_member_order_and_repeats() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::{rows::LaunchNavalBase, rows::NavalBaseRow};
        let assets = launch_assets();
        for zero_at in 0..3 {
            let mut frame = launch_frame();
            let kind = marine_sources(&mut frame, true);
            launch_zero(
                &mut frame,
                0,
                Some(vec![
                    LaunchNavalBase {
                        base_province: 4,
                        country: 0,
                    },
                    LaunchNavalBase {
                        base_province: 1,
                        country: 999,
                    },
                    LaunchNavalBase {
                        base_province: 4,
                        country: 11,
                    },
                ]),
                None,
            );
            let mut members = vec![10, 11];
            members.insert(zero_at, 0);
            put::<tables::FactionMembers>(
                &mut frame,
                members
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(ordinal, &country)| FactionMemberRow {
                        faction: 7,
                        country,
                        ordinal: ordinal as u32,
                    })
                    .collect(),
            );
            put::<tables::NavalBases>(
                &mut frame,
                vec![
                    NavalBaseRow {
                        province: 2,
                        country: 10,
                        ..Default::default()
                    },
                    NavalBaseRow {
                        province: 3,
                        country: 11,
                        ..Default::default()
                    },
                ],
            );
            let paths = SourceVisits::default();
            assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::False);
            let expected: Vec<_> = members
                .iter()
                .flat_map(|country| match country {
                    0 => vec![4, 1, 4],
                    10 => vec![2],
                    11 => vec![3],
                    _ => unreachable!(),
                })
                .map(|province| (2, province))
                .collect();
            assert_eq!(*paths.distances.lock().unwrap(), expected);
        }
    }

    #[test]
    fn actual_zero_null_sources_skip_provider_path_and_army_demands() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::rows::LaunchNavalBase;
        let assets = launch_assets();
        for stored in [0, u32::MAX, 0x8000_0000] {
            let mut frame = launch_frame();
            let kind = marine_sources(&mut frame, true);
            launch_zero(
                &mut frame,
                0,
                Some(vec![LaunchNavalBase {
                    base_province: stored,
                    country: 999,
                }]),
                None,
            );
            *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
                reason: "unreached positive provider refused".into(),
            };
            let paths = SourceVisits::default();
            assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::False);
            assert!(paths.distances.lock().unwrap().is_empty());
            assert!(paths.reaches.lock().unwrap().is_empty());
            *frame.tables[TableId::CountryCommandPower.index()] = TableView::Unavailable {
                reason: "command power refused".into(),
            };
            assert!(matches!(
                sources_gate(&assets, &frame, &kind, &paths),
                Eval::Unknown(_)
            ));
            assert!(paths.notes.lock().unwrap().is_empty());
        }
        for stored in [2, 500] {
            let mut frame = launch_frame();
            let kind = marine_sources(&mut frame, true);
            launch_zero(
                &mut frame,
                0,
                Some(vec![LaunchNavalBase {
                    base_province: stored,
                    country: 0,
                }]),
                None,
            );
            let rows = frame
                .complete::<tables::Provinces>()
                .unwrap()
                .iter()
                .filter(|row| row.id != stored)
                .cloned()
                .collect();
            put::<tables::Provinces>(&mut frame, rows);
            let paths = SourceVisits::default();
            assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::False);
            assert!(paths.distances.lock().unwrap().is_empty());
            assert!(paths.reaches.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn actual_zero_native_null_stays_unknown_and_never_uses_global_owner_zero_bases() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::{rows::LaunchNavalBases, rows::NavalBaseRow};

        let assets = launch_assets();
        let mut frame = launch_frame();
        let kind = marine_sources(&mut frame, true);
        launch_zero(&mut frame, 0, None, Some(vec![]));
        put::<tables::NavalBases>(
            &mut frame,
            vec![
                NavalBaseRow {
                    province: 2,
                    country: 0,
                    ..Default::default()
                },
                NavalBaseRow {
                    province: 1,
                    country: 10,
                    ..Default::default()
                },
            ],
        );
        for provider in [
            LaunchNavalBases::NativeNull,
            LaunchNavalBases::Bases(vec![]),
        ] {
            let is_null = matches!(provider, LaunchNavalBases::NativeNull);
            let mut rows = frame
                .complete::<tables::ReservedCountryNaming>()
                .unwrap()
                .to_vec();
            rows[0]
                .launch_sources
                .require_mut("fixture")
                .unwrap()
                .as_mut()
                .unwrap()
                .naval_bases = provider;
            put::<tables::ReservedCountryNaming>(&mut frame, rows);
            let paths = SourceVisits::default();
            let result = sources_gate(&assets, &frame, &kind, &paths);
            if is_null {
                assert!(matches!(result, Eval::Unknown(reasons)
                    if reasons.iter().any(|reason| reason.contains("native provider is null"))));
            } else {
                assert_eq!(result, Eval::False);
            }
            assert_eq!(*paths.distances.lock().unwrap(), vec![(2, 1)]);
            assert_eq!(*paths.reaches.lock().unwrap(), vec![1]);
        }
    }

    #[test]
    fn actual_zero_positive_provider_and_packet_gaps_stay_named_at_their_position() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::{rows::LaunchNavalBase, rows::NavalBaseRow, rows::Recorded};
        let assets = launch_assets();
        let mut base = launch_frame();
        let kind = marine_sources(&mut base, true);
        launch_zero(
            &mut base,
            0,
            Some(vec![LaunchNavalBase {
                base_province: 2,
                country: 0,
            }]),
            None,
        );
        put::<tables::NavalBases>(
            &mut base,
            vec![NavalBaseRow {
                province: 1,
                country: 10,
                ..Default::default()
            }],
        );
        for mode in 0..6 {
            let mut frame = base.clone();
            let expected = match mode {
                0 => {
                    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
                        reason: "positive provider refused".into(),
                    };
                    "Provinces"
                }
                1 => {
                    *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
                        rows: tables::Provinces::wrap(vec![ProvinceRow {
                            id: 2,
                            ..Default::default()
                        }]),
                        missing: 1,
                    };
                    "Provinces"
                }
                _ => {
                    let mut rows = frame
                        .complete::<tables::ReservedCountryNaming>()
                        .unwrap()
                        .to_vec();
                    match mode {
                        2 => rows[0].launch_sources = Recorded::NotRecorded,
                        3 => rows[0].launch_sources = Recorded::Value(None),
                        4 => {
                            rows[0]
                                .launch_sources
                                .require_mut("fixture")
                                .unwrap()
                                .as_mut()
                                .unwrap()
                                .naval_bases = replay_data::rows::LaunchNavalBases::Unavailable
                        }
                        5 => {
                            rows[0]
                                .launch_sources
                                .require_mut("fixture")
                                .unwrap()
                                .as_mut()
                                .unwrap()
                                .sample_hour =
                                replay_data::rows::PacketHour::Stamped(frame.hour - 1)
                        }
                        _ => unreachable!(),
                    }
                    put::<tables::ReservedCountryNaming>(&mut frame, rows);
                    "ReservedCountryNaming.launch_sources"
                }
            };
            let paths = SourceVisits::default();
            assert!(
                matches!(sources_gate(&assets, &frame, &kind, &paths), Eval::Unknown(reasons)
                if reasons.iter().any(|reason| reason.contains(expected)))
            );
            assert_eq!(*paths.distances.lock().unwrap(), vec![(2, 1)]);
        }
        let mut members = base.complete::<tables::FactionMembers>().unwrap().to_vec();
        members.insert(
            1,
            FactionMemberRow {
                faction: 7,
                country: 999,
                ordinal: 1,
            },
        );
        members[2].ordinal = 2;
        put::<tables::FactionMembers>(&mut base, members);
        let paths = SourceVisits::default();
        assert!(
            matches!(sources_gate(&assets, &base, &kind, &paths), Eval::Unknown(reasons)
            if reasons.iter().any(|reason| reason.contains("country 999")))
        );
        assert_eq!(*paths.distances.lock().unwrap(), vec![(2, 2), (2, 1)]);
    }

    #[test]
    fn actual_zero_state_sources_use_ordered_roster_and_independent_domains() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        let kind = marine_sources(&mut frame, false);
        launch_zero(&mut frame, 3, None, Some(vec![20, 10, 20]));
        let mut reserved = frame
            .complete::<tables::ReservedCountryNaming>()
            .unwrap()
            .to_vec();
        reserved[0]
            .launch_sources
            .require_mut("fixture")
            .unwrap()
            .as_mut()
            .unwrap()
            .naval_bases = replay_data::rows::LaunchNavalBases::NativeNull;
        put::<tables::ReservedCountryNaming>(&mut frame, reserved);
        let states = frame
            .complete::<tables::States>()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut row| {
                row.owner = 0;
                row
            })
            .collect();
        put::<tables::States>(&mut frame, states);
        put::<tables::ProvinceBuildings>(
            &mut frame,
            vec![
                BuildingRow {
                    location: 3,
                    building_type: DAM,
                    ..Default::default()
                },
                BuildingRow {
                    location: 1,
                    building_type: DAM,
                    ..Default::default()
                },
            ],
        );
        let paths = SourceVisits::default();
        assert_eq!(sources_gate(&assets, &frame, &kind, &paths), Eval::False);
        assert_eq!(
            *paths.distances.lock().unwrap(),
            vec![(5, 3), (5, 1), (5, 3)]
        );
        for mode in 0..6 {
            let mut refused = frame.clone();
            let mut rows = refused
                .complete::<tables::ReservedCountryNaming>()
                .unwrap()
                .to_vec();
            match mode {
                0 => {
                    rows[0]
                        .launch_sources
                        .require_mut("fixture")
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .owned_states = None
                }
                1 => rows[0].owned_state_count = replay_data::rows::Recorded::Value(None),
                2 => rows[0].owned_state_count = replay_data::rows::Recorded::Value(Some(2)),
                3 => {
                    let mut states = refused.complete::<tables::States>().unwrap().to_vec();
                    states.retain(|row| row.id != 20);
                    put::<tables::States>(&mut refused, states);
                }
                4 => rows[0].owned_state_count = replay_data::rows::Recorded::Value(Some(-1)),
                5 => rows[0].owned_state_count = replay_data::rows::Recorded::NotRecorded,
                _ => unreachable!(),
            }
            put::<tables::ReservedCountryNaming>(&mut refused, rows);
            assert!(
                matches!(sources_gate(&assets, &refused, &kind, &SourceVisits::default()),
                Eval::Unknown(reasons) if reasons.iter().any(|reason| reason.contains("ReservedCountryNaming")))
            );
        }
        let mut rows = frame
            .complete::<tables::ReservedCountryNaming>()
            .unwrap()
            .to_vec();
        rows[0].owned_state_count = replay_data::rows::Recorded::Value(Some(0));
        rows[0].launch_sources = replay_data::rows::Recorded::NotRecorded;
        put::<tables::ReservedCountryNaming>(&mut frame, rows);
        let mut empty = kind.clone();
        empty.unit_requirements = vec![UnitRequirement::default()];
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "unreached building tokens".into(),
        };
        *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Unavailable {
            reason: "unreached buildings".into(),
        };
        assert_eq!(
            sources_gate(&assets, &frame, &empty, &SourceVisits::default()),
            Eval::False
        );
    }

    #[test]
    fn actual_zero_buildings_require_the_full_province_census_after_nonzero_count() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        let kind = marine_sources(&mut frame, false);
        launch_zero(&mut frame, 1, None, Some(vec![10]));
        let states = frame
            .complete::<tables::States>()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut row| {
                row.owner = 0;
                row
            })
            .collect();
        put::<tables::States>(&mut frame, states);
        put::<tables::ProvinceBuildings>(
            &mut frame,
            vec![BuildingRow {
                location: 1,
                building_type: DAM,
                ..Default::default()
            }],
        );
        let provinces = frame
            .complete::<tables::Provinces>()
            .unwrap()
            .iter()
            .filter(|row| row.id != 4)
            .cloned()
            .collect();
        *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
            rows: tables::Provinces::wrap(provinces),
            missing: 1,
        };
        let paths = SourceVisits::default();
        assert!(matches!(sources_gate(&assets, &frame, &kind, &paths),
            Eval::Unknown(reasons) if reasons.iter().any(|reason| reason.contains("Provinces"))));
        assert!(paths.distances.lock().unwrap().is_empty());
        assert!(paths.reaches.lock().unwrap().is_empty());

        let mut rows = frame
            .complete::<tables::ReservedCountryNaming>()
            .unwrap()
            .to_vec();
        rows[0].owned_state_count = replay_data::rows::Recorded::Value(Some(0));
        rows[0].launch_sources = replay_data::rows::Recorded::NotRecorded;
        put::<tables::ReservedCountryNaming>(&mut frame, rows);
        *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
            reason: "unreached provider census".into(),
        };
        *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Unavailable {
            reason: "unreached building census".into(),
        };
        assert_eq!(
            sources_gate(&assets, &frame, &kind, &SourceVisits::default()),
            Eval::False
        );
    }

    #[test]
    fn launch_zero_packet_and_provider_changes_do_not_reuse_other_frame_sources() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::rows::LaunchNavalBase;
        let assets = launch_assets();
        let mut first = launch_frame();
        let kind = marine_sources(&mut first, true);
        launch_zero(
            &mut first,
            0,
            Some(vec![LaunchNavalBase {
                base_province: 2,
                country: 0,
            }]),
            None,
        );
        let a = SourceVisits::default();
        assert_eq!(sources_gate(&assets, &first, &kind, &a), Eval::False);
        assert_eq!(*a.distances.lock().unwrap(), vec![(2, 2)]);
        let mut hole = first.clone();
        hole.branch += 1;
        let provinces = hole
            .complete::<tables::Provinces>()
            .unwrap()
            .iter()
            .filter(|row| row.id != 2)
            .cloned()
            .collect();
        put::<tables::Provinces>(&mut hole, provinces);
        let b = SourceVisits::default();
        assert_eq!(sources_gate(&assets, &hole, &kind, &b), Eval::False);
        assert!(b.distances.lock().unwrap().is_empty());
        let mut next = first.clone();
        next.hour += 1;
        launch_zero(
            &mut next,
            0,
            Some(vec![
                LaunchNavalBase {
                    base_province: 4,
                    country: 0,
                },
                LaunchNavalBase {
                    base_province: 1,
                    country: 999,
                },
            ]),
            None,
        );
        let c = SourceVisits::default();
        assert_eq!(sources_gate(&assets, &next, &kind, &c), Eval::False);
        assert_eq!(*c.distances.lock().unwrap(), vec![(2, 4), (2, 1)]);
    }

    #[test]
    fn air_sources_in_range_decide_and_types_share_only_an_equal_range() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let frame = launch_frame();
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let launcher = launch::Launcher::new(&assets, &world, 0, &launch::Unreconstructed);
        let launch = |kind: &Kind, province: u32| {
            launcher.launchable(
                kind,
                &RaidScope {
                    province,
                    ..Default::default()
                },
                Scopes::of(script::Scope::Country(0)),
            )
        };
        let named = |name: &str, max_distance: i64| Kind {
            name: name.into(),
            max_distance_raw: Some(max_distance),
            ..bomber()
        };
        let (far, near, twin) = (
            named("far", 70_000),
            named("near", 60_000),
            named("twin", 70_000),
        );
        assert_eq!(launch(&far, 3), Eval::True);
        assert_eq!(launch(&near, 3), Eval::False);
        assert_eq!(launch(&twin, 3), Eval::True);
        assert_eq!(launch(&near, 2), Eval::True);
        match launch(&near, 9) {
            Eval::Unknown(reasons) => {
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("no map centroid"))
                )
            }
            other => panic!("expected an unmeasured distance, got {other:?}"),
        }
    }

    #[test]
    fn a_marine_division_waits_on_the_path_search_but_wings_cannot_use_a_naval_base() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::NavalBases>(
            &mut frame,
            vec![replay_data::rows::NavalBaseRow {
                province: 2,
                country: 10,
                ..Default::default()
            }],
        );
        let mut naval = bomber();
        naval.starting_point.types = vec![SourceKind::NavalBase];
        assert_eq!(gate(&assets, &frame, &naval, 3), Eval::False);
        naval.unit_requirements = vec![UnitRequirement {
            battalions: vec![content_reader_raids::raids::BattalionRequirement {
                name: "marine".into(),
                min: Some(2),
                max: None,
            }],
            ..Default::default()
        }];
        put::<tables::Divisions>(
            &mut frame,
            vec![DivisionRow {
                id: DIVISION,
                roster_owner: 10,
                province: 1,
                template: TEMPLATE,
                ..Default::default()
            }],
        );
        put::<tables::DivisionTemplateIcons>(
            &mut frame,
            vec![DivisionTemplateIconRow {
                template: TEMPLATE,
                subunits: vec![TemplateSubunit {
                    token: MARINE,
                    local: 0,
                    count: 2,
                }],
                ..Default::default()
            }],
        );
        assert!(matches!(gate(&assets, &frame, &naval, 3), Eval::Unknown(_)));
        put::<tables::DivisionTemplateIcons>(
            &mut frame,
            vec![DivisionTemplateIconRow {
                template: TEMPLATE,
                subunits: vec![TemplateSubunit {
                    token: MARINE,
                    local: 0,
                    count: 1,
                }],
                ..Default::default()
            }],
        );
        assert_eq!(gate(&assets, &frame, &naval, 3), Eval::False);
    }

    #[test]
    fn launch_inputs_read_before_the_hour_still_decide_the_gate() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);
        let ace = 0x0000_0009_0000_0047;
        let mut wings = frame.complete::<tables::AirWings>().unwrap().to_vec();
        wings[0].ace = ace;
        put::<tables::AirWings>(&mut frame, wings);
        let row = replay_data::rows::AceRangeInputRow {
            canonical_slot: 1,
            ace,
            wing: WING,
            alive: 1,
            range: 10_000,
            ..Default::default()
        };
        put::<tables::AceRangeInputs>(&mut frame, vec![row.clone()]);
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);
        put::<tables::AceRangeInputs>(
            &mut frame,
            vec![replay_data::rows::AceRangeInputRow { alive: 0, ..row }],
        );
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);
        put::<tables::AceRangeInputs>(&mut frame, Vec::new());
        assert!(matches!(
            gate(&assets, &frame, &bomber(), 3),
            Eval::Unknown(_)
        ));
    }

    #[test]
    fn a_carrier_launching_a_live_raid_is_no_source() {
        content_reader_script::test_game::enter_viewer();
        let assets = launch_assets();
        let mut frame = launch_frame();
        const SHIP: u64 = 0x0000_0007_0000_0033;
        put::<tables::AirBases>(
            &mut frame,
            vec![AirBaseRow {
                id: BASE,
                host: SHIP,
                ..Default::default()
            }],
        );
        let mut carrier = bomber();
        carrier.starting_point.types = vec![SourceKind::Carrier];
        assert_eq!(gate(&assets, &frame, &carrier, 3), Eval::True);
        put::<tables::Raids>(
            &mut frame,
            vec![RaidRow {
                source: RaidSource {
                    ship_reference_type: SHIP as u32,
                    ship_reference_id: (SHIP >> 32) as u32,
                    ..Default::default()
                },
                ..raid(9, 0, 0, 3)
            }],
        );
        assert_eq!(gate(&assets, &frame, &carrier, 3), Eval::False);
    }
}

#[test]
fn path_cost_reads_the_owners_native_cache_and_independent_zero_war() {
    use content_reader_unit_path::unit_path::MoveWorld;
    use replay_data::{rows::CountryCounterInputRow, rows::Recorded};
    let assets = assets();
    for (present, retired, flag, mut targets, expected) in [
        (0, 0, 0, vec![], [false, false]),
        (1, 0, 0, vec![], [true, false]),
        (1, 128, 0, vec![2], [false, true]),
        (1, 128, 1, vec![], [true, false]),
    ] {
        let mut frame = frame();
        if flag != 0 {
            targets.insert(0, 0);
        }
        put::<tables::CountryCounterInputs>(
            &mut frame,
            vec![CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 11,
                zero_war: Some([present, retired]).into(),
                co_belligerents: Some(targets).into(),
                ..Default::default()
            }],
        );
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let cells = reach::Cells::default();
        let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
        assert_eq!(
            [moves.path_friendly(1, 0), moves.path_friendly(1, 2)],
            expected.map(Ok)
        );
        assert!(moves.path_friendly(2, 0).is_err());
    }

    let mut frame = frame();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries.push(CountryRow {
        id: 12,
        canonical_slot: 2,
        tag: b"OTHER".to_vec(),
        ..Default::default()
    });
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 12,
        canonical_slot: 2,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    put::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 11,
            co_belligerents: Some(vec![0, 2]).into(),
            zero_war: Recorded::Value(None),
            ..Default::default()
        }],
    );
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "independent war read refused".into(),
    };
    {
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let cells = reach::Cells::default();
        let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
        assert_eq!(moves.path_friendly(1, 0), Ok(true));
        assert_eq!(moves.path_friendly(1, 2), Ok(true));
    }

    put::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 11,
            co_belligerents: Recorded::NotRecorded,
            zero_war: Some([1, 0]).into(),
            ..Default::default()
        }],
    );
    put::<tables::WarRelations>(
        &mut frame,
        vec![replay_data::rows::WarRelationRow {
            first_country: 11,
            second_country: 12,
            ..Default::default()
        }],
    );
    {
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let cells = reach::Cells::default();
        let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
        assert_eq!(moves.path_friendly(1, 0), Ok(true));
        assert_eq!(moves.path_friendly(1, 2), Ok(true));
    }

    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries
        .iter_mut()
        .find(|row| row.canonical_slot == 1)
        .unwrap()
        .id = u32::MAX;
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: u32::MAX,
        canonical_slot: 1,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    put::<tables::CountryCounterInputs>(
        &mut frame,
        vec![CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: u32::MAX,
            co_belligerents: Some(vec![]).into(),
            zero_war: Some([1, 0]).into(),
            ..Default::default()
        }],
    );
    put::<tables::WarRelations>(
        &mut frame,
        vec![replay_data::rows::WarRelationRow {
            first_country: u32::MAX,
            second_country: 12,
            ..Default::default()
        }],
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let cells = reach::Cells::default();
    let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
    assert_eq!(world.at_war(1, 2), Ok(true));
    assert_eq!(moves.path_friendly(1, 0), Ok(false));
    assert_eq!(moves.path_friendly(1, 2), Ok(false));
}

#[test]
fn raid_path_distances_prefer_the_recorded_memo_and_resolve_naval_targets() {
    use launch::{PathSource, RaidPaths};
    let assets = path_assets();
    let mut frame = frame();
    put::<tables::NavalBases>(
        &mut frame,
        vec![replay_data::rows::NavalBaseRow {
            province: 4,
            level: 1,
            ..Default::default()
        }],
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let target = |province| RaidScope {
        province,
        ..Default::default()
    };
    let source = |kind, province| PathSource { kind, province };
    let paths = paths::HourPaths::new(&assets, &frame);
    assert_eq!(
        paths.distance(&world, &source(2, 1), &target(3)),
        Ok(None),
        "land 3 is not the target, so it blocks the way to 4"
    );
    assert_eq!(
        paths.distance(&world, &source(2, 3), &target(4)),
        Ok(Some(50_000))
    );
    assert_eq!(
        paths.distance(&world, &source(5, 3), &target(4)),
        Ok(Some(50_000))
    );
    assert_eq!(paths.distance(&world, &source(5, 1), &target(4)), Ok(None));
    put::<tables::RaidPathDistances>(
        &mut frame,
        vec![
            replay_data::rows::RaidPathDistanceRow {
                dispatcher: replay_data::rows::RaidPathDispatcher::Land,
                source: 1,
                target: 4,
                distance: 7,
                ok: 1,
            },
            replay_data::rows::RaidPathDistanceRow {
                dispatcher: replay_data::rows::RaidPathDispatcher::Naval,
                source: 1,
                target: 4,
                distance: 19,
                ok: 1,
            },
        ],
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let paths = paths::HourPaths::new(&assets, &frame);
    assert_eq!(
        paths.distance(&world, &source(5, 1), &target(4)),
        Ok(Some(7))
    );
    assert_eq!(
        paths.distance(&world, &source(2, 1), &target(4)),
        Ok(Some(19))
    );
    assert!(paths.distance(&world, &source(0, 1), &target(4)).is_err());
}

#[test]
fn army_reach_reads_the_recorded_hour_and_answers_once_per_source() {
    use launch::RaidPaths;
    use replay_data::{
        rows::DivisionMovementRow, rows::DivisionRow, rows::DivisionStatusRow, rows::StateFlagRow,
    };
    let assets = path_assets();
    let mut frame = frame();
    put::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: 0x77,
            roster_owner: 11,
            logical_owner: 11,
            province: 3,
            hit_points: 100_000,
            organisation: 500_000,
            ..Default::default()
        }],
    );
    put::<tables::DivisionMovement>(
        &mut frame,
        vec![DivisionMovementRow {
            division: 0x77,
            ..Default::default()
        }],
    );
    put::<tables::DivisionStatus>(
        &mut frame,
        vec![DivisionStatusRow {
            division: 0x77,
            ..Default::default()
        }],
    );
    put::<tables::LandCombats>(&mut frame, Vec::new());
    put::<tables::WarRelations>(&mut frame, Vec::new());
    let flags = |demilitarized| {
        vec![StateFlagRow {
            state: 20,
            demilitarized,
            ..Default::default()
        }]
    };
    put::<tables::StateFlags>(&mut frame, flags(0));
    let reach = |frame: &DecodedFrame| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let paths = paths::HourPaths::new(&assets, frame);
        let first = paths.army_reaches(&world, 0x77, 4);
        assert_eq!(paths.army_reaches(&world, 0x77, 4), first);
        first
    };
    assert_eq!(reach(&frame), Eval::True);
    put::<tables::StateFlags>(&mut frame, flags(1));
    assert_eq!(reach(&frame), Eval::False);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let paths = paths::HourPaths::new(&assets, &frame);
    assert!(matches!(
        paths.army_reaches(&world, 0x99, 4),
        Eval::Unknown(_)
    ));
}

fn war_frame() -> DecodedFrame {
    let mut frame = frame();
    put::<tables::CountryLinks>(
        &mut frame,
        (0..4)
            .map(|slot| CountryLinkRow {
                id: 10 + slot,
                canonical_slot: slot,
            })
            .collect(),
    );
    let war = |first, second, end_hour| replay_data::rows::WarRelationRow {
        first_country: first,
        second_country: second,
        start_hour: 1,
        end_hour,
        ..Default::default()
    };
    put::<tables::WarRelations>(
        &mut frame,
        vec![war(10, 11, 0), war(13, 10, 0), war(10, 12, 3)],
    );
    put::<tables::CountryWars>(&mut frame, Vec::new());
    frame
}

fn enemies_of(
    frame: &DecodedFrame,
    slot: u32,
) -> content_reader_unit_path::unit_path::Fact<BTreeSet<u32>> {
    use content_reader_unit_path::unit_path::MoveWorld;
    let assets = assets();
    let world = World::new(frame, assets.trigger_content.facts()).unwrap();
    let cells = reach::Cells::default();
    let moves = reach::Moves::new(&world, assets.movement_reads(), &cells);
    Ok(moves.enemies(slot)?.into_iter().collect())
}

#[test]
fn enemy_sets_derive_from_war_relations_unless_the_vector_was_read() {
    let mut frame = war_frame();
    assert_eq!(enemies_of(&frame, 0), Ok(BTreeSet::from([1, 3])));
    assert_eq!(enemies_of(&frame, 3), Ok(BTreeSet::from([0])));
    assert_eq!(enemies_of(&frame, 2), Ok(BTreeSet::new()));
    let read = |country, enemies: Vec<u32>| replay_data::rows::CountryWarRow {
        country,
        can_occupy_non_war: 1,
        enemies_read: 1,
        enemies,
        ordered: Default::default(),
    };
    put::<tables::CountryWars>(&mut frame, vec![read(10, vec![13, 11])]);
    assert_eq!(enemies_of(&frame, 0), Ok(BTreeSet::from([1, 3])));
    put::<tables::CountryWars>(&mut frame, vec![read(12, vec![])]);
    put::<tables::WarRelations>(
        &mut frame,
        vec![replay_data::rows::WarRelationRow {
            first_country: 12,
            second_country: 11,
            start_hour: 1,
            ..Default::default()
        }],
    );
    assert_eq!(enemies_of(&frame, 2), Ok(BTreeSet::new()));
    assert_eq!(enemies_of(&frame, 1), Ok(BTreeSet::from([2])));
}

mod reach_bench;
fn strait_assets() -> RaidAssets {
    let mut graph = content_reader_map::province_graph::ProvinceGraph::default();
    for (id, next) in [(1, vec![2]), (2, vec![1, 3]), (3, vec![2])] {
        graph
            .edges
            .insert(id, next.into_iter().map(|n| (n, Some(50_000))).collect());
    }
    let mut assets = assets();
    assets.geometry.land = BTreeSet::from([1, 3]);
    assets.paths = Ok(paths::PathSearch::new(graph));
    assets
}

#[test]
fn earlier_queries_never_mix_dispatchers_for_the_same_pair() {
    use paths::Distances;
    let assets = strait_assets();
    let mut frame = frame();
    put::<tables::RaidPathDistances>(&mut frame, Vec::new());
    put::<tables::NavalBases>(
        &mut frame,
        vec![replay_data::rows::NavalBaseRow {
            province: 3,
            level: 1,
            ..Default::default()
        }],
    );
    let scope = RaidScope {
        province: 3,
        ..Default::default()
    };
    let distances = Distances::new(&assets, &frame);
    assert_eq!(distances.distance(2, 1, &scope), Ok(Some(100_000)));
    assert_eq!(distances.distance(5, 1, &scope), Ok(None));
    assert_eq!(distances.distance(2, 1, &scope), Ok(Some(100_000)));
    let distances = Distances::new(&assets, &frame);
    distances.note(5, 1, &scope);
    assert_eq!(distances.distance(2, 1, &scope), Ok(Some(100_000)));
    assert_eq!(distances.distance(5, 1, &scope), Ok(None));
}

mod recorded_paths {
    use super::*;
    use content_reader_raids::{
        raids::BattalionRequirement, raids::SourceKind, raids::StartingPoint,
        raids::UnitRequirement,
    };
    use launch::{PathSource, RaidPaths};
    use replay_data::{
        rows::DivisionRow, rows::DivisionTemplateIconRow, rows::NukeRow, rows::RaidPathDistanceRow,
        rows::TemplateSubunit,
    };

    struct Reaching<'a>(paths::HourPaths<'a>);

    impl RaidPaths for Reaching<'_> {
        fn distance(
            &self,
            world: &World<'_>,
            source: &PathSource,
            target: &RaidScope,
        ) -> Result<Option<i64>, String> {
            self.0.distance(world, source, target)
        }
        fn note(&self, source: &PathSource, target: &RaidScope) {
            self.0.note(source, target);
        }
        fn recorded_rows(&self) -> Option<&[RaidPathDistanceRow]> {
            self.0.recorded_rows()
        }
        fn path_target(&self, kind: u8, target: &RaidScope) -> Result<Option<u32>, String> {
            self.0.path_target(kind, target)
        }
        fn army_reaches(&self, _: &World<'_>, _: u64, _: u32) -> Eval {
            Eval::True
        }
    }

    const MARINE: u32 = 400;

    fn gate(memo: Vec<RaidPathDistanceRow>) -> Eval {
        gate_in(&assets(), &hour_with(memo), 1)
    }

    fn hour_with(memo: Vec<RaidPathDistanceRow>) -> DecodedFrame {
        let mut frame = frame();
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.push(LexicalTokenRow {
            token: MARINE,
            literal: b"marine".to_vec(),
        });
        put::<tables::LexicalTokens>(&mut frame, tokens);
        put::<tables::Countries>(
            &mut frame,
            vec![CountryRow {
                id: 10,
                canonical_slot: 0,
                tag: b"GER".to_vec(),
                command_power_raw: 50 * 100_000,
                ..Default::default()
            }],
        );
        put::<tables::RaidCountryInputs>(
            &mut frame,
            vec![replay_data::rows::RaidCountryInputRow {
                canonical_slot: 0,
                ..registered_launch_inputs()
            }],
        );
        put::<tables::CountryCommandPower>(&mut frame, registered_command_power([0]));
        put::<tables::Nukes>(
            &mut frame,
            (0..2)
                .map(|nuke_type| NukeRow {
                    country: 10,
                    nuke_type,
                    ..Default::default()
                })
                .collect(),
        );
        put::<tables::NavalBases>(
            &mut frame,
            [1, 3]
                .map(|province| replay_data::rows::NavalBaseRow {
                    province,
                    country: if province == 1 { 10 } else { 11 },
                    level: 1,
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::Divisions>(
            &mut frame,
            vec![DivisionRow {
                id: 7,
                roster_owner: 10,
                province: 1,
                template: 8,
                ..Default::default()
            }],
        );
        put::<tables::DivisionTemplateIcons>(
            &mut frame,
            vec![DivisionTemplateIconRow {
                template: 8,
                subunits: vec![TemplateSubunit {
                    token: MARINE,
                    local: 0,
                    count: 1,
                }],
                ..Default::default()
            }],
        );
        put::<tables::RaidPathDistances>(&mut frame, memo);
        frame
    }

    fn commando() -> RaidType {
        RaidType {
            starting_point: StartingPoint {
                types: vec![SourceKind::NavalBase],
                ..Default::default()
            },
            unit_requirements: vec![UnitRequirement {
                battalions: vec![BattalionRequirement {
                    name: "marine".into(),
                    min: Some(1),
                    max: None,
                }],
                ..Default::default()
            }],
            max_distance_raw: Some(100_000),
            ..kind("commando", TargetType::Provinces(vec![3]))
        }
    }

    fn gate_in(assets: &RaidAssets, frame: &DecodedFrame, times: usize) -> Eval {
        let kind = commando();
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let paths = Reaching(paths::HourPaths::new(assets, frame));
        let launcher = launch::Launcher::new(assets, &world, 0, &paths);
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        let mut answers: Vec<Eval> = (0..times)
            .map(|_| launcher.launchable(&kind, &scope, Scopes::of(Scope::Country(0))))
            .collect();
        answers.dedup();
        assert_eq!(answers.len(), 1, "every lookup answers alike");
        answers.remove(0)
    }

    #[test]
    fn the_memo_index_is_shared_only_while_the_memo_is_unchanged() {
        let assets = assets();
        let near = hour_with(memo(100_000, 1));
        assert_eq!(gate_in(&assets, &near, 70), Eval::True);
        let mut next = near.clone();
        next.hour += 1;
        assert_eq!(gate_in(&assets, &next, 70), Eval::True);
        let far = hour_with(memo(100_001, 1));
        assert_eq!(gate_in(&assets, &far, 70), Eval::False);
        assert_eq!(gate_in(&assets, &near, 70), Eval::True);
    }

    #[test]
    fn indexed_launch_uses_only_the_sources_dispatcher() {
        use replay_data::rows::RaidPathDispatcher;
        let assets = assets();
        for (land, naval, expected) in [(7, 100_001, Eval::False), (100_001, 7, Eval::True)] {
            let rows = [RaidPathDispatcher::Land, RaidPathDispatcher::Naval]
                .into_iter()
                .zip([land, naval])
                .map(|(dispatcher, distance)| RaidPathDistanceRow {
                    dispatcher,
                    source: 1,
                    target: 3,
                    distance,
                    ok: 1,
                })
                .collect();
            let frame = hour_with(rows);
            assert_eq!(gate_in(&assets, &frame, 70), expected);
        }
        let frame = hour_with(vec![RaidPathDistanceRow {
            dispatcher: RaidPathDispatcher::Land,
            source: 1,
            target: 3,
            distance: 7,
            ok: 1,
        }]);
        assert!(matches!(gate_in(&assets, &frame, 70), Eval::Unknown(_)));
    }

    fn memo(distance: i64, ok: u8) -> Vec<RaidPathDistanceRow> {
        vec![RaidPathDistanceRow {
            dispatcher: replay_data::rows::RaidPathDispatcher::Naval,
            source: 1,
            target: 3,
            distance,
            ok,
        }]
    }

    #[test]
    fn a_recorded_memo_decides_the_path_sources_without_searching() {
        assert_eq!(gate(memo(100_000, 1)), Eval::True);
        assert_eq!(gate(memo(100_001, 1)), Eval::False, "beyond max_distance");
        assert_eq!(gate(memo(0, 0)), Eval::False, "no path");
        assert!(matches!(gate(Vec::new()), Eval::Unknown(_)));
    }
}

mod view_bench;

mod unlinked {
    use super::*;
    use replay_data::rows::DynamicModifierRow;

    fn unlinked_controller() -> DecodedFrame {
        let mut frame = frame();
        let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
        provinces[2].controller = 99;
        put::<tables::Provinces>(&mut frame, provinces);
        frame
    }

    #[test]
    fn an_unresolved_target_owner_withholds_its_offers() {
        let assets = assets();
        let mut frame = unlinked_controller();
        put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3), status(1, 4)]);
        let drawn = icons(&assets, &frame, 0, &launch::Unreconstructed);
        let provinces: Vec<u32> = drawn.icons.iter().map(|icon| icon.province).collect();
        assert_eq!(provinces, [4]);
        assert_eq!(drawn.report.withheld, 1);
        assert!(
            drawn
                .report
                .reasons
                .contains_key("a raid target's country 99 has no CountryLinks row")
        );
        let mut empty = frame.clone();
        let mut provinces = empty.complete::<tables::Provinces>().unwrap().to_vec();
        provinces[2].controller = 0;
        put::<tables::Provinces>(&mut empty, provinces);
        let drawn = icons(&assets, &empty, 0, &launch::Unreconstructed);
        assert_eq!(drawn.report.withheld, 0);
    }

    #[test]
    fn an_unresolved_raid_target_owner_is_withheld_not_dropped() {
        let assets = assets();
        let mut frame = unlinked_controller();
        put::<tables::Raids>(&mut frame, vec![raid(2, 1, 0, 2)]);
        put::<tables::RaidTargets>(&mut frame, vec![target(2, 3, 1)]);
        let drawn = icons(&assets, &frame, 0, &launch::Unreconstructed);
        assert!(drawn.icons.is_empty());
        assert_eq!(drawn.report.withheld, 1);
    }

    #[test]
    fn an_unlinked_modifier_owner_leaves_the_trigger_undecided() {
        content_reader_script::test_game::enter_viewer();
        let assets = assets();
        let mut frame = frame();
        let row = |country: u32, token: &[u8]| DynamicModifierRow {
            country,
            token: token.to_vec(),
            ..Default::default()
        };
        put::<tables::DynamicModifiers>(&mut frame, vec![row(10, b"held"), row(99, b"maybe")]);
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let has = |name: &str| {
            world.evaluate(
                &parse(&format!("has_dynamic_modifier = {{ modifier = {name} }}")),
                Scopes::of(script::Scope::Country(0)),
            )
        };
        assert_eq!(has("held"), Eval::True);
        assert_eq!(has("absent"), Eval::False);
        assert!(matches!(has("maybe"), Eval::Unknown(_)));
    }
}

fn invasion_frame(army_group: u8) -> DecodedFrame {
    use replay_data::{
        rows::ArmyOrderPlanRow, rows::ArmyOrderRow, rows::LandTheatreRow, rows::OrdersGroupPlanRow,
        rows::OrdersGroupRow,
    };
    let mut frame = positive_frame();
    put::<tables::LandTheatres>(
        &mut frame,
        vec![LandTheatreRow {
            id: 7,
            owner: 10,
            ..Default::default()
        }],
    );
    put::<tables::OrdersGroups>(
        &mut frame,
        vec![OrdersGroupRow {
            handle: 0x100,
            country: 10,
            is_army_group: army_group,
            theatre: 7,
            theatre_ordinal: Some(0),
            ..Default::default()
        }],
    );
    put::<tables::OrdersGroupPlans>(
        &mut frame,
        vec![OrdersGroupPlanRow {
            group: 0x100,
            primary_orders: vec![1],
            ..Default::default()
        }],
    );
    let order = |instance, order_type, path: Vec<u32>| ArmyOrderRow {
        owner_group: 0x100,
        instance,
        country: 10,
        order_type,
        path,
        ..Default::default()
    };
    put::<tables::ArmyOrders>(
        &mut frame,
        vec![order(1, 1, vec![1]), order(2, 3, vec![3, 4])],
    );
    let plan = |instance, successors| ArmyOrderPlanRow {
        owner_group: 0x100,
        instance,
        successors,
        ..Default::default()
    };
    put::<tables::ArmyOrderPlans>(&mut frame, vec![plan(1, vec![2]), plan(2, Vec::new())]);
    frame
}

#[test]
fn naval_invasions_are_found_through_the_successor_graph() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str, state: u32| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(2));
        scopes.target_state = Some(state);
        world.evaluate(&parse(source), scopes)
    };
    let frame = invasion_frame(0);
    let simple = "has_naval_invasion_against_state = var:target_state";
    assert_eq!(eval(&frame, simple, 20), script::Eval::True);
    assert_eq!(eval(&frame, simple, 10), script::Eval::False);
    let numbered = "has_naval_invasion_against_state = 20";
    assert_eq!(eval(&frame, numbered, 0), script::Eval::True);
    let absent = "has_naval_invasion_against_state = 99";
    assert_eq!(eval(&frame, absent, 0), script::Eval::False);
    let activated =
        "has_naval_invasion_against_state = { state = var:target_state activated = yes }";
    assert_eq!(eval(&frame, activated, 20), script::Eval::False);
    let prepared =
        "has_naval_invasion_against_state = { state = var:target_state preparation > 0.1 }";
    assert!(matches!(
        eval(&frame, prepared, 20),
        script::Eval::Unknown(_)
    ));
    assert_eq!(eval(&frame, prepared, 10), script::Eval::False);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let mut scopes = Scopes::of(Scope::Country(1));
    scopes.target_state = Some(20);
    assert_eq!(world.evaluate(&parse(simple), scopes), script::Eval::False);
    assert_eq!(eval(&invasion_frame(1), simple, 20), script::Eval::False);
}

#[test]
fn is_ally_with_follows_the_friend_relation() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{
        rows::CountryCounterInputRow, rows::Recorded, rows::ReservedCountryNamingRow,
        rows::SubjectRow, rows::WarRelationRow,
    };
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(2)))
    };
    let mut frame = positive_frame();
    let owned = |eng| {
        vec![
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 10,
                owned_state_count: 1,
                ..Default::default()
            },
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 11,
                owned_state_count: eng,
                ..Default::default()
            },
        ]
    };
    put::<tables::CountryCounterInputs>(&mut frame, owned(1));
    assert_eq!(eval(&frame, "is_ally_with = ROOT"), script::Eval::True);
    assert_eq!(eval(&frame, "is_ally_with = ENG"), script::Eval::False);
    let faction = |country| FactionMemberRow {
        country,
        faction: 1,
        ordinal: 0,
    };
    put::<tables::FactionMembers>(&mut frame, vec![faction(10), faction(11)]);
    assert_eq!(eval(&frame, "is_ally_with = ENG"), script::Eval::True);
    put::<tables::WarRelations>(
        &mut frame,
        vec![WarRelationRow {
            first_country: 10,
            second_country: 11,
            start_hour: 1,
            ..Default::default()
        }],
    );
    assert_eq!(eval(&frame, "is_ally_with = ENG"), script::Eval::False);
    put::<tables::WarRelations>(&mut frame, Vec::new());
    put::<tables::FactionMembers>(&mut frame, Vec::new());
    put::<tables::Subjects>(
        &mut frame,
        vec![SubjectRow {
            overlord: 11,
            subject: 10,
            start_hour: 1,
            ..Default::default()
        }],
    );
    assert_eq!(eval(&frame, "is_ally_with = ENG"), script::Eval::True);
    put::<tables::CountryCounterInputs>(&mut frame, owned(0));
    assert!(matches!(
        eval(&frame, "is_ally_with = ENG"),
        script::Eval::Unknown(reasons)
            if reasons.iter().any(|reason| reason.contains("reserved country naming"))
    ));
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![], vec![]])),
            ..Default::default()
        }],
    );
    assert_eq!(eval(&frame, "is_ally_with = ENG"), script::Eval::False);
}

#[test]
fn any_state_in_reads_a_global_array() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::ScriptVariableEntry, rows::ScriptVariablesRow};
    let mut assets = assets();
    scripted(&mut assets, "helper = { always = yes }");
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(2)))
    };
    let array = |entries: &[(&str, i64)]| {
        let mut frame = positive_frame();
        put::<tables::ScriptVariables>(
            &mut frame,
            vec![ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
                seed: replay_data::rows::Recorded::Value(None),
                entries: entries
                    .iter()
                    .map(|(name, value_raw)| ScriptVariableEntry {
                        name: name.as_bytes().to_vec(),
                        value_raw: *value_raw,
                    })
                    .collect(),
            }],
        );
        frame
    };
    let encoded = (0xffff_ffff_c000_0000_u64 | 10) as i64;
    let frame = array(&[
        ("wall^num", 200_000),
        ("wall^0", encoded),
        ("wall^1", 2_000_000),
    ]);
    let english = "any_state_in = { array = global.wall is_owned_by = ENG }";
    let german = "any_state_in = { array = global.wall is_owned_by = GER }";
    assert_eq!(eval(&frame, english), script::Eval::True);
    assert_eq!(eval(&frame, german), script::Eval::True);
    let both = "any_state_in = { array = global.wall count = 2 is_owned_by = ENG }";
    assert_eq!(eval(&frame, both), script::Eval::False);
    let helper = |value: &str| format!("any_state_in = {{ array = global.wall helper = {value} }}");
    assert_eq!(eval(&frame, &helper("yes")), script::Eval::True);
    assert_eq!(eval(&frame, &helper("no")), script::Eval::False);
    assert_eq!(eval(&array(&[]), english), script::Eval::False);
    let one = array(&[
        ("wall^num", 100_000),
        ("wall^0", encoded),
        ("wall^1", 2_000_000),
    ]);
    assert_eq!(eval(&one, english), script::Eval::False);
    for raw in [0, 9_900_000, -100_000] {
        let null = array(&[("wall^num", 100_000), ("wall^0", raw)]);
        assert_eq!(eval(&null, english), script::Eval::False, "{raw}");
        let body = |body: &str| format!("any_state_in = {{ array = global.wall {body} }}");
        for (source, expected) in [
            ("always = yes", script::Eval::True),
            ("always = no", script::Eval::False),
            ("NOT = { is_coastal = yes }", script::Eval::True),
            ("is_coastal = no", script::Eval::False),
            ("hidden_trigger = { always = yes }", script::Eval::False),
            ("has_dlc = Gotterdammerung", script::Eval::True),
            ("owner = { always = yes }", script::Eval::False),
            ("ROOT = { tag = GER }", script::Eval::True),
            ("any_neighbor_state = { always = yes }", script::Eval::False),
            ("dam > 0", script::Eval::False),
        ] {
            assert_eq!(eval(&null, &body(source)), expected, "{raw}: {source}");
        }
        let two = "any_state_in = { array = global.wall count = 2 always = yes }";
        assert_eq!(eval(&null, two), script::Eval::False);
        assert_eq!(eval(&null, &body("helper = yes")), script::Eval::False);
        assert_eq!(eval(&null, &body("helper = no")), script::Eval::True);
    }
    let mut gapped = super::test_support::assets();
    gapped.trigger_content.state_provinces.insert(99, vec![1]);
    let missing = array(&[("wall^num", 100_000), ("wall^0", 9_900_000)]);
    let world = World::new(&missing, gapped.trigger_content.facts()).unwrap();
    assert!(matches!(
        world.evaluate(&parse(english), Scopes::of(Scope::Country(2))),
        script::Eval::Unknown(_)
    ));
    let local = "any_state_in = { array = wall is_owned_by = ENG }";
    assert!(matches!(eval(&frame, local), script::Eval::Unknown(_)));
}

#[test]
fn neighbour_state_triggers_walk_the_content_neighbours() {
    content_reader_script::test_game::enter_viewer();
    let frame = positive_frame();
    let eval = |assets: &RaidAssets, scope: Scope, source: &str| {
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(scope))
    };
    let base = assets();
    let german = Scope::State(10);
    let any = |owner: &str| format!("any_neighbor_state = {{ is_owned_by = {owner} }}");
    let all = |owner: &str| format!("all_neighbor_state = {{ is_owned_by = {owner} }}");
    assert_eq!(eval(&base, german, &any("ENG")), script::Eval::True);
    assert_eq!(eval(&base, german, &any("GER")), script::Eval::False);
    assert_eq!(eval(&base, german, &all("ENG")), script::Eval::True);
    assert_eq!(
        eval(&base, Scope::State(20), &all("ENG")),
        script::Eval::False
    );
    let two = "any_neighbor_state = { count = 2 is_owned_by = ENG }";
    assert_eq!(eval(&base, german, two), script::Eval::False);
    let none = "any_neighbor_state = { count = 0 is_owned_by = GER }";
    assert_eq!(eval(&base, german, none), script::Eval::True);
    let country = eval(&base, Scope::Country(2), &any("ENG"));
    assert_eq!(country, script::Eval::False);
    let mut refused = assets();
    refused.trigger_content.neighbor_states = Err("no graph".into());
    assert!(matches!(
        eval(&refused, german, &any("ENG")),
        script::Eval::Unknown(_)
    ));
    let mut unrecorded = assets();
    unrecorded.trigger_content.neighbor_states = Ok(BTreeMap::from([(10, vec![20, 30])]));
    assert_eq!(eval(&unrecorded, german, &any("ENG")), script::Eval::True);
    assert!(matches!(
        eval(&unrecorded, german, &any("GER")),
        script::Eval::Unknown(_)
    ));
    assert!(matches!(
        eval(&unrecorded, german, &all("ENG")),
        script::Eval::Unknown(_)
    ));
    let mut alone = assets();
    alone.trigger_content.neighbor_states = Ok(BTreeMap::from([(10, Vec::new())]));
    assert_eq!(eval(&alone, german, &any("ENG")), script::Eval::False);
    assert_eq!(eval(&alone, german, &all("GER")), script::Eval::True);
}

#[test]
fn custom_tooltips_and_is_subject_of() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::rows::SubjectRow;
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(2)))
    };
    let mut frame = positive_frame();
    let wrapped = |inner: &str| {
        format!(
            "custom_trigger_tooltip = {{ tooltip = SOME_KEY not_tooltip = SOME_KEY_NOT {inner} }}"
        )
    };
    assert_eq!(eval(&frame, &wrapped("tag = GER")), script::Eval::True);
    assert_eq!(
        eval(&frame, &wrapped("tag = GER tag = ENG")),
        script::Eval::False
    );
    let formatted = |tag: &str| {
        format!("custom_override_tooltip = {{ tooltip = {{ localization_key = KEY }} tag = {tag} }}")
    };
    assert_eq!(eval(&frame, &formatted("GER")), script::Eval::True);
    assert_eq!(eval(&frame, &formatted("ENG")), script::Eval::False);
    assert_eq!(
        eval(&frame, "custom_trigger_tooltip = { tooltip = KEY }"),
        script::Eval::True
    );
    assert_eq!(eval(&frame, "is_subject_of = ENG"), script::Eval::False);
    put::<tables::Subjects>(
        &mut frame,
        vec![SubjectRow {
            overlord: 11,
            subject: 10,
            start_hour: 1,
            ..Default::default()
        }],
    );
    assert_eq!(eval(&frame, "is_subject_of = ENG"), script::Eval::True);
    assert_eq!(
        eval(&frame, &wrapped("is_subject_of = ENG")),
        script::Eval::True
    );
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    let english = Scopes::of(Scope::Country(1));
    assert_eq!(
        world.evaluate(&parse("is_subject_of = GER"), english),
        script::Eval::False
    );
    put::<tables::Subjects>(
        &mut frame,
        vec![SubjectRow {
            overlord: 11,
            subject: 10,
            start_hour: 1,
            end_hour: 3,
            ..Default::default()
        }],
    );
    assert_eq!(eval(&frame, "is_subject_of = ENG"), script::Eval::False);
}

fn leader_row(id: u32, owner: u32, captured: u8) -> replay_data::rows::UnitLeaderRow {
    replay_data::rows::UnitLeaderRow {
        id,
        reference_type: 7,
        character_id: id,
        owner,
        captured,
        ..Default::default()
    }
}

#[test]
fn captured_general_triggers_read_the_recorded_leaders() {
    content_reader_script::test_game::enter_viewer();
    let assets = assets();
    let mut frame = frame();
    let reference = (5_u64 << 32) | 7;
    let eval = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(slot));
        scopes.target_leader = Some(reference);
        world.evaluate(&parse(source), scopes)
    };
    let any = "has_any_captured_general = yes";
    put::<tables::UnitLeaders>(&mut frame, vec![leader_row(5, 10, 0), leader_row(6, 11, 1)]);
    assert_eq!(eval(&frame, any, 0), script::Eval::False);
    assert_eq!(eval(&frame, any, 1), script::Eval::True);
    let rescue = "is_leader_visible = var:target_leader var:target_leader = { tag = ROOT }";
    assert_eq!(eval(&frame, rescue, 0), script::Eval::True);
    assert_eq!(eval(&frame, rescue, 1), script::Eval::False);
    let mut duplicate = leader_row(6, 11, 1);
    duplicate.character_id = 5;
    put::<tables::UnitLeaders>(&mut frame, vec![leader_row(5, 10, 0), duplicate]);
    let script::Eval::Unknown(reasons) = eval(&frame, rescue, 0) else {
        panic!("ambiguous current leader role must be refused");
    };
    assert!(
        reasons
            .iter()
            .any(|why| why.contains("more than one unit leaders row")),
        "{reasons:?}"
    );
    let mut chosen = leader_row(5, 10, 1);
    chosen.visibility_choice = 1;
    put::<tables::UnitLeaders>(&mut frame, vec![chosen]);
    assert!(matches!(eval(&frame, rescue, 0), script::Eval::Unknown(_)));
    assert!(matches!(eval(&frame, any, 0), script::Eval::Unknown(_)));
    let mut chosen = leader_row(5, 10, 1);
    chosen.visibility_choice = 1;
    put::<tables::UnitLeaders>(&mut frame, vec![chosen, leader_row(8, 10, 1)]);
    assert_eq!(eval(&frame, any, 0), script::Eval::True);
    put::<tables::UnitLeaders>(&mut frame, Vec::new());
    assert_eq!(
        eval(&frame, "is_leader_visible = var:target_leader", 0),
        script::Eval::False
    );
    put::<tables::UnitLeaders>(&mut frame, vec![leader_row(6, 11, 1)]);
    assert_eq!(
        eval(&frame, "has_any_captured_general = no", 1),
        script::Eval::True
    );
    assert_eq!(
        eval(&frame, "has_any_captured_general = no", 0),
        script::Eval::False
    );
}

#[test]
fn is_core_of_reads_the_recorded_state_cores() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::rows::StateCoreRow;
    let assets = assets();
    let mut frame = positive_frame();
    let on_state = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::State(20)))
    };
    assert!(matches!(
        on_state(&frame, "is_core_of = GER"),
        script::Eval::Unknown(_)
    ));
    put::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 20,
            cores: vec![11, 10],
            ..Default::default()
        }],
    );
    assert_eq!(on_state(&frame, "is_core_of = GER"), script::Eval::True);
    assert_eq!(on_state(&frame, "is_core_of = ENG"), script::Eval::True);
    put::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 20,
            cores: vec![11],
            ..Default::default()
        }],
    );
    assert_eq!(on_state(&frame, "is_core_of = GER"), script::Eval::False);
    put::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 20,
            cores: vec![99],
            ..Default::default()
        }],
    );
    assert!(matches!(
        on_state(&frame, "is_core_of = GER"),
        script::Eval::Unknown(_)
    ));
}

#[test]
fn a_prepared_raids_launch_triggers_follow_the_native_chain() {
    let mut assets = assets();
    let mut frame = positive_frame();
    let scope = RaidScope {
        province: 3,
        ..Default::default()
    };
    let decide = |assets: &RaidAssets,
                  frame: &DecodedFrame,
                  source: Result<Option<u32>, String>| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        eligibility::launch_triggers(assets, &world, 2, &assets.catalog.types[0], &scope, &source)
    };
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::True);
    assets.catalog.types[0].launchable = parse("tag = GER FROM = { tag = ENG }");
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::True);
    assets.catalog.types[0].launchable = parse("tag = ENG");
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::False);
    assets.catalog.types[0].launchable = Vec::new();
    assets.catalog.types[0].launchable_from = parse("tag = ENG");
    assert_eq!(decide(&assets, &frame, Ok(Some(1))), Eval::True);
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::False);
    assert!(matches!(
        decide(&assets, &frame, Ok(None)),
        Eval::Unknown(_)
    ));
    assets.catalog.types[0].launchable_from = Vec::new();
    assets.catalog.types[0].available = parse("always = no");
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::False);
    assets.catalog.types[0].available = Vec::new();
    put::<tables::RaidCooldowns>(
        &mut frame,
        vec![RaidCooldownRow {
            days: 3,
            owner_slot: 2,
            ordinal: 0,
            raid_type: OILFIELD,
            scope: scope.clone(),
        }],
    );
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::False);
    put::<tables::RaidCooldowns>(&mut frame, Vec::new());
    assets.catalog.types[0].fire_only_once = true;
    let mut used = frame.clone();
    put::<tables::RaidCountries>(
        &mut used,
        vec![RaidCountryRow {
            canonical_slot: 2,
            country: 10,
            eligible_types: vec![OILFIELD],
            used_types: vec![OILFIELD],
        }],
    );
    assert_eq!(decide(&assets, &used, Ok(Some(2))), Eval::False);
    assert_eq!(decide(&assets, &frame, Ok(Some(2))), Eval::True);
}

#[test]
fn leader_visibility_evaluates_the_recorded_template_role() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::rows::CharacterRow;
    use script::RecordedTemplate;
    let path = "common/characters/X.txt";
    let mut assets = assets();
    assets.trigger_content.leader_roles = Ok(Arc::new(LeaderRoles::new(
        content_reader_key_names::characters::parse_leader_templates(
            &path.into(),
            br#"characters = {
    GER_rommel = { field_marshal = { visible = { tag = GER } } }
    GER_split = {
        instance = { allowed = { } corps_commander = { visible = { tag = ENG } } }
        instance = { allowed = { } advisor = { slot = army_chief } }
    }
    GER_broken = { corps_commander = yes }
    GER_plain = { corps_commander = { skill = 1 } }
}"#,
        )
        .unwrap(),
    )));
    let mut frame = positive_frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    for (token, name) in [
        (501, "GER_rommel"),
        (502, "ger_split"),
        (504, "GER_broken"),
        (507, "GER_plain"),
    ] {
        tokens.push(LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        });
    }
    put::<tables::LexicalTokens>(&mut frame, tokens);
    put::<tables::Characters>(
        &mut frame,
        vec![CharacterRow {
            id: 1,
            name_token_id: 501,
            ..Default::default()
        }],
    );
    let leader = |owner: u32, choice: u8, template: u8| {
        let mut row = leader_row(5, owner, 1);
        (
            row.character_id,
            row.visibility_choice,
            row.template_visibility,
        ) = (1, choice, template);
        row
    };
    let recorded = |tag: u32, line: i32| RecordedTemplate {
        tag,
        file: path.as_bytes().to_vec(),
        line,
        generated: false,
    };
    let undecided = |value: script::Eval, why: &str| match value {
        script::Eval::Unknown(reasons) => {
            assert!(
                reasons.iter().any(|reason| reason.contains(why)),
                "{reasons:?}"
            )
        }
        other => panic!("{other:?}"),
    };
    use script::Eval::{False, True};
    {
        let visible = |row: &replay_data::rows::UnitLeaderRow,
                       template: Option<RecordedTemplate>| {
            let mut observed = frame.clone();
            put::<tables::UnitLeaders>(&mut observed, vec![row.clone()]);
            let mut character = CharacterRow {
                id: 1,
                ..Default::default()
            };
            if let Some(template) = template {
                character.template_present = 1;
                character.template_tag = template.tag;
                character.template_file = template.file;
                character.template_line = template.line;
                character.template_generated = u8::from(template.generated);
            }
            put::<tables::Characters>(&mut observed, vec![character]);
            let world = World::new(&observed, assets.trigger_content.facts()).unwrap();
            let mut scopes = Scopes::of(Scope::Country(2));
            scopes.target_leader = Some(5_u64 << 32 | 7);
            world.evaluate(&parse("is_leader_visible = var:target_leader"), scopes)
        };
        for (choice, template) in [(1, 1), (0, 1), (1, 0)] {
            let rommel = Some(recorded(501, 2));
            assert_eq!(visible(&leader(10, choice, template), rommel.clone()), True);
            assert_eq!(visible(&leader(11, choice, template), rommel), False);
        }
        assert_eq!(visible(&leader(11, 1, 1), Some(recorded(502, 4))), True);
        assert_eq!(visible(&leader(10, 1, 1), Some(recorded(502, 4))), False);
        assert_eq!(visible(&leader(10, 0, 1), Some(recorded(502, 5))), True);
        undecided(
            visible(&leader(10, 1, 1), Some(recorded(502, 5))),
            "has no unit leader role",
        );
        assert_eq!(visible(&leader(11, 1, 1), Some(recorded(507, 8))), True);
        undecided(
            visible(&leader(10, 1, 1), Some(recorded(504, 7))),
            "refused",
        );
        undecided(
            visible(&leader(10, 1, 1), Some(recorded(501, 3))),
            "no content character template",
        );
        undecided(
            visible(&leader(10, 1, 1), Some(recorded(502, 2))),
            "no content character template",
        );
        let elsewhere = RecordedTemplate {
            file: b"common/characters/Y.txt".to_vec(),
            ..recorded(501, 2)
        };
        undecided(
            visible(&leader(10, 1, 1), Some(elsewhere)),
            "no content character template",
        );
        let generated = RecordedTemplate {
            generated: true,
            ..recorded(501, 2)
        };
        undecided(
            visible(&leader(10, 1, 1), Some(generated)),
            "generate_character",
        );
        undecided(visible(&leader(10, 1, 1), None), "no template");
    }
    let reference = (5_u64 << 32) | 7;
    let through_trigger = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(slot));
        scopes.target_leader = Some(reference);
        world.evaluate(&parse(source), scopes)
    };
    put::<tables::UnitLeaders>(&mut frame, vec![leader(10, 1, 1)]);
    undecided(
        through_trigger(&frame, "is_leader_visible = var:target_leader", 2),
        "no template",
    );
    undecided(
        through_trigger(&frame, "has_any_captured_general = yes", 2),
        "no template",
    );
    let mut with_template = frame.clone();
    put::<tables::Characters>(
        &mut with_template,
        vec![CharacterRow {
            id: 1,
            name_token_id: 501,
            template_present: 1,
            template_tag: 501,
            template_file: path.as_bytes().to_vec(),
            template_line: 2,
            ..Default::default()
        }],
    );
    assert_eq!(
        through_trigger(&with_template, "is_leader_visible = var:target_leader", 2),
        True
    );
    put::<tables::UnitLeaders>(&mut frame, vec![leader(10, 0, 0)]);
    assert_eq!(
        through_trigger(&frame, "is_leader_visible = var:target_leader", 2),
        True
    );
    assets.trigger_content.leader_roles = Err("no characters in the test".into());
    let world = World::new(&with_template, assets.trigger_content.facts()).unwrap();
    let mut scopes = Scopes::of(Scope::Country(2));
    scopes.target_leader = Some(reference);
    undecided(
        world.evaluate(&parse("is_leader_visible = var:target_leader"), scopes),
        "no characters in the test",
    );
}

#[test]
fn character_scope_global_flag_overlord_and_cosmetic_triggers() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::CountryAppearanceRow, rows::ScriptFlagRow};
    let assets = assets();
    let mut frame = positive_frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    tokens.push(LexicalTokenRow {
        token: 600,
        literal: b"exiled".to_vec(),
    });
    put::<tables::LexicalTokens>(&mut frame, tokens);
    let flag = |scope_kind: u8, scope_id: u32| ScriptFlagRow {
        scope_kind,
        scope_id,
        token: 600,
        value: 1,
        ..Default::default()
    };
    let mut leader = leader_row(5, 10, 0);
    leader.character_id = 44;
    put::<tables::UnitLeaders>(&mut frame, vec![leader]);
    let reference = (5_u64 << 32) | 7;
    let eval = |frame: &DecodedFrame, source: &str, this: Scope| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(2));
        scopes.this = this;
        scopes.target_leader = Some(reference);
        world.evaluate(&parse(source), scopes)
    };
    use script::Eval::{False, True};
    let country = Scope::Country(2);
    let in_leader = Scope::Leader(reference);
    for source in [
        "has_character_flag = exiled",
        "is_country_leader = no",
        "is_country_leader = yes",
    ] {
        assert_eq!(eval(&frame, source, country), False, "{source}");
    }
    assert_eq!(
        eval(&frame, "NOT = { has_character_flag = exiled }", country),
        True
    );
    assert_eq!(
        eval(&frame, "has_character_flag = exiled", in_leader),
        False
    );
    put::<tables::ScriptFlags>(&mut frame, vec![flag(3, 9)]);
    assert_eq!(
        eval(&frame, "has_character_flag = exiled", in_leader),
        False
    );
    put::<tables::ScriptFlags>(&mut frame, vec![flag(0, 0), flag(3, 44)]);
    assert_eq!(eval(&frame, "has_character_flag = exiled", in_leader), True);
    assert!(matches!(
        eval(&frame, "is_country_leader = no", in_leader),
        script::Eval::Unknown(_)
    ));
    assert_eq!(eval(&frame, "has_global_flag = exiled", country), True);
    assert_eq!(eval(&frame, "has_global_flag = other", country), False);
    assert_eq!(
        eval(
            &frame,
            "hidden_trigger = { has_global_flag = exiled tag = ENG }",
            country
        ),
        False
    );
    assert_eq!(
        eval(
            &frame,
            "hidden_trigger = { has_global_flag = exiled }",
            country
        ),
        True
    );
    assert!(matches!(
        eval(&frame, "OVERLORD = { tag = ENG }", country),
        script::Eval::Unknown(_)
    ));
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![
            CountryAppearanceRow {
                country: 10,
                overlord: 11,
                cosmetic_tag: b"jimma".to_vec(),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
        ],
    );
    assert_eq!(eval(&frame, "OVERLORD = { tag = ENG }", country), True);
    assert_eq!(eval(&frame, "overlord = { tag = GER }", country), False);
    assert_eq!(
        eval(&frame, "OVERLORD = { always = yes }", Scope::Country(1)),
        False
    );
    assert_eq!(eval(&frame, "has_cosmetic_tag = jimma", country), True);
    assert_eq!(eval(&frame, "has_cosmetic_tag = Jimma", country), False);
    assert_eq!(
        eval(&frame, "has_cosmetic_tag = jimma", Scope::Country(1)),
        False
    );
}

fn country_leader_traits() -> content_reader_military::leader_traits::LeaderTraits {
    let root = std::env::temp_dir().join(format!(
        "raid-ruling-trait-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let file = root.join("common/country_leader/traits.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "leader_traits = { dictator = {} }").unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let traits = content_reader_military::leader_traits::read(&layers).unwrap().0;
    std::fs::remove_dir_all(root).unwrap();
    traits
}

#[test]
fn country_leader_civil_war_and_idea_triggers() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{
        rows::ActiveIdeaRow, rows::CharacterRow, rows::CountryAppearanceRow, rows::HiredAdvisorRow,
        rows::WarRelationRow,
    };
    let mut assets = assets();
    assets.trigger_content.character_traits =
        Some((Default::default(), Ok(Arc::new(country_leader_traits()))));
    let mut frame = positive_frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    for (token, name) in [(700, "GER_karl"), (701, "GER_otto"), (702, "dictator")] {
        tokens.push(LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        });
    }
    put::<tables::LexicalTokens>(&mut frame, tokens);
    put::<tables::Characters>(
        &mut frame,
        [(1, 700), (2, 701), (3, 700)]
            .into_iter()
            .map(|(id, name_token_id)| CharacterRow {
                id,
                name_token_id,
                ..Default::default()
            })
            .collect(),
    );
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![
            CountryAppearanceRow {
                country: 10,
                leader_ideology: b"despotism".to_vec(),
                leader_character: Some(1),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
        ],
    );
    put::<tables::ActiveIdeas>(
        &mut frame,
        vec![ActiveIdeaRow {
            country: 10,
            ordinal: 0,
            token: b"idea_a".to_vec(),
        }],
    );
    put::<tables::HiredAdvisors>(
        &mut frame,
        vec![HiredAdvisorRow {
            country: 11,
            ordinal: 0,
            character: 2,
            idea: b"advisor_a".to_vec(),
            ..Default::default()
        }],
    );
    use script::Eval::{False, True};
    let sources = [
        "has_country_leader = { character = GER_karl ruling_only = yes }",
        "has_country_leader_with_trait = dictator",
        "has_civil_war = yes",
        "has_idea = idea_a",
    ];
    {
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        assert_eq!(
            world.evaluate(&parse(sources[3]), Scopes::of(Scope::Country(2))),
            script::Eval::True
        );
    }
    use replay_data::{rows::CountryCharacterRow, rows::PartyLeaderRow, rows::PoliticalPartyRow};
    let party = |country, id, is_ruling| PoliticalPartyRow {
        country,
        id,
        is_ruling,
        ..Default::default()
    };
    put::<tables::PoliticalParties>(
        &mut frame,
        vec![party(10, 1, 0), party(10, 2, 1), party(11, 3, 1)],
    );
    let leader = |party, ordinal, character, leader_id| PartyLeaderRow {
        country: 10,
        party,
        ordinal,
        character: Some(character),
        leader_id,
        role: Some(party as i32),
    };
    put::<tables::PartyLeaders>(
        &mut frame,
        vec![
            leader(1, 0, 2, 12),
            leader(2, 0, 1, 11),
            leader(2, 1, 2, 12),
        ],
    );
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![
            CountryAppearanceRow {
                country: 10,
                leader_ideology: b"despotism".to_vec(),
                leader_character: Some(1),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
        ],
    );
    put::<tables::CountryLeaderRoles>(
        &mut frame,
        vec![replay_data::rows::CountryLeaderRoleRow {
            character: 1,
            country: 10,
            ideology: 2,
            ..Default::default()
        }],
    );
    put::<tables::CharacterRoleTraits>(
        &mut frame,
        vec![replay_data::rows::CharacterRoleTraitRow {
            character: 1,
            kind: replay_data::rows::CharacterRoleTraitRow::COUNTRY_LEADER,
            role: 2,
            tokens: vec![702],
            definitions: vec![1],
        }],
    );
    put::<tables::CountryCharacters>(
        &mut frame,
        [Some(1), Some(3), Some(2)]
            .into_iter()
            .enumerate()
            .map(|(ordinal, character)| CountryCharacterRow {
                country: 10,
                ordinal: ordinal as u32,
                character,
            })
            .collect(),
    );
    put::<tables::CountryCivilWars>(
        &mut frame,
        vec![replay_data::rows::CountryCivilWarRow {
            country: 11,
            ordinal: 0,
            target: 10,
        }],
    );
    put::<tables::AdvisorTokens>(
        &mut frame,
        vec![replay_data::rows::AdvisorTokenRow {
            token: b"advisor_a".to_vec(),
            ..Default::default()
        }],
    );
    put::<tables::CountryCounterInputs>(
        &mut frame,
        vec![replay_data::rows::CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 10,
            controlled_state_count: 3,
            ..Default::default()
        }],
    );
    let eval = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(slot)))
    };
    assert_eq!(eval(&frame, sources[0], 2), True);
    assert_eq!(
        eval(
            &frame,
            "has_country_leader = { character = GER_otto ruling_only = yes }",
            2
        ),
        False
    );
    assert_eq!(
        eval(
            &frame,
            "has_country_leader = { character = GER_otto ruling_only = no }",
            2
        ),
        True
    );
    assert_eq!(
        eval(&frame, "has_country_leader = { character = GER_otto }", 2),
        True
    );
    assert_eq!(
        eval(&frame, "has_country_leader = { character = GER_karl }", 1),
        False
    );
    assert_eq!(eval(&frame, "has_country_leader = { id = 12 }", 2), True);
    assert_eq!(
        eval(
            &frame,
            "has_country_leader = { id = 12 ruling_only = yes }",
            2
        ),
        False
    );
    assert_eq!(eval(&frame, "has_country_leader = { id = 0 }", 2), False);
    assert_eq!(
        eval(&frame, "has_country_leader = { name = \"Karl\" }", 2),
        False
    );
    let mut named = frame.clone();
    put::<tables::Characters>(
        &mut named,
        [(1, 700, "Karl"), (2, 701, "Otto"), (3, 700, "Other")]
            .into_iter()
            .map(|(id, name_token_id, name)| CharacterRow {
                id,
                name_token_id,
                name: name.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    );
    assert_eq!(
        eval(
            &named,
            "has_country_leader = { name = \"Otto\" ruling_only = yes }",
            2
        ),
        False
    );
    assert_eq!(
        eval(&named, "has_country_leader = { name = \"Otto\" }", 2),
        True
    );
    assert_eq!(eval(&frame, "num_of_controlled_states > 2", 2), True);
    assert_eq!(eval(&frame, "num_of_controlled_states > 3", 2), False);
    assert_eq!(eval(&frame, sources[1], 2), True);
    assert_eq!(
        eval(&frame, "has_country_leader_with_trait = other", 2),
        False
    );
    assert_eq!(eval(&frame, sources[1], 1), False);
    let mut counters = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    for country in [10, 11] {
        counters.push(replay_data::rows::CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country,
            owned_state_count: 1,
            ..Default::default()
        });
    }
    put::<tables::CountryCounterInputs>(&mut frame, counters);
    assert_eq!(eval(&frame, "has_civil_war = yes", 1), True);
    assert_eq!(eval(&frame, "has_civil_war = yes", 2), False);
    put::<tables::WarRelations>(
        &mut frame,
        vec![WarRelationRow {
            first_country: 10,
            second_country: 11,
            start_hour: 1,
            end_hour: -1,
            ..Default::default()
        }],
    );
    assert_eq!(eval(&frame, "has_civil_war = yes", 2), True);
    assert_eq!(eval(&frame, "has_civil_war = no", 2), False);
    assert_eq!(eval(&frame, "has_idea = idea_a", 2), True);
    assert_eq!(eval(&frame, "has_idea = idea_a", 1), False);
    assert_eq!(eval(&frame, "has_idea = advisor_a", 1), True);
    assert_eq!(eval(&frame, "has_idea = advisor_a", 2), False);
}

#[test]
fn recorded_political_tables_decide_country_leader_triggers() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{
        rows::CharacterRow, rows::CountryAppearanceRow, rows::CountryCharacterRow,
        rows::PartyLeaderRow, rows::PoliticalPartyRow,
    };
    let mut assets = assets();
    assets.trigger_content.character_traits =
        Some((Default::default(), Ok(Arc::new(country_leader_traits()))));
    let mut frame = positive_frame();
    let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
    for (token, name) in [(700, "GER_karl"), (701, "GER_otto"), (702, "dictator")] {
        tokens.push(LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        });
    }
    put::<tables::LexicalTokens>(&mut frame, tokens);
    put::<tables::Characters>(
        &mut frame,
        [(1, 700), (2, 701), (3, 700)]
            .into_iter()
            .map(|(id, name_token_id)| CharacterRow {
                id,
                name_token_id,
                ..Default::default()
            })
            .collect(),
    );
    let party = |country, id, is_ruling| PoliticalPartyRow {
        country,
        id,
        is_ruling,
        ..Default::default()
    };
    put::<tables::PoliticalParties>(
        &mut frame,
        vec![party(10, 1, 0), party(10, 2, 1), party(11, 3, 1)],
    );
    let leader = |party, ordinal, character, leader_id| PartyLeaderRow {
        country: 10,
        party,
        ordinal,
        character: Some(character),
        leader_id,
        role: Some(party as i32),
    };
    put::<tables::PartyLeaders>(
        &mut frame,
        vec![
            leader(1, 0, 2, 12),
            leader(2, 0, 1, 11),
            leader(2, 1, 2, 12),
        ],
    );
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![
            CountryAppearanceRow {
                country: 10,
                leader_ideology: b"despotism".to_vec(),
                leader_character: Some(1),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
        ],
    );
    put::<tables::CountryLeaderRoles>(
        &mut frame,
        vec![replay_data::rows::CountryLeaderRoleRow {
            character: 1,
            country: 10,
            ideology: 2,
            ..Default::default()
        }],
    );
    put::<tables::CharacterRoleTraits>(
        &mut frame,
        vec![replay_data::rows::CharacterRoleTraitRow {
            character: 1,
            kind: replay_data::rows::CharacterRoleTraitRow::COUNTRY_LEADER,
            role: 2,
            tokens: vec![702],
            definitions: vec![1],
        }],
    );
    put::<tables::CountryCharacters>(
        &mut frame,
        [Some(1), Some(3), Some(2)]
            .into_iter()
            .enumerate()
            .map(|(ordinal, character)| CountryCharacterRow {
                country: 10,
                ordinal: ordinal as u32,
                character,
            })
            .collect(),
    );
    let eval = |source: &str, slot: u32| {
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(slot)))
    };
    use script::Eval::{False, True};
    for (source, slot, expected) in [
        (
            "has_country_leader = { character = GER_karl ruling_only = yes }",
            2,
            True,
        ),
        (
            "has_country_leader = { character = GER_otto ruling_only = yes }",
            2,
            False,
        ),
        (
            "has_country_leader = { character = GER_otto ruling_only = no }",
            2,
            True,
        ),
        ("has_country_leader = { character = GER_otto }", 2, True),
        ("has_country_leader = { character = GER_karl }", 1, False),
        ("has_country_leader = { id = 12 }", 2, True),
        (
            "has_country_leader = { id = 12 ruling_only = yes }",
            2,
            False,
        ),
        ("has_country_leader = { id = 0 }", 2, False),
        ("has_country_leader_with_trait = dictator", 2, True),
        ("has_country_leader_with_trait = other", 2, False),
        ("has_country_leader_with_trait = dictator", 1, False),
    ] {
        assert_eq!(eval(source, slot), expected, "{source} at {slot}");
    }
    let mut wars = frame.clone();
    put::<tables::CountryCivilWars>(
        &mut wars,
        vec![replay_data::rows::CountryCivilWarRow {
            country: 11,
            ordinal: 0,
            target: 10,
        }],
    );
    let mut counters = wars
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    for country in [10, 11] {
        counters.push(replay_data::rows::CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country,
            owned_state_count: 1,
            ..Default::default()
        });
    }
    put::<tables::CountryCounterInputs>(&mut wars, counters);
    let eval_wars = |slot: u32| {
        let world = World::new(&wars, assets.trigger_content.facts()).unwrap();
        world.evaluate(
            &parse("has_civil_war = yes"),
            Scopes::of(Scope::Country(slot)),
        )
    };
    assert_eq!(eval_wars(1), True);
    assert_eq!(eval_wars(2), False);
    let mut ideas = frame.clone();
    put::<tables::AdvisorTokens>(
        &mut ideas,
        vec![replay_data::rows::AdvisorTokenRow {
            ordinal: 0,
            token: b"advisor_a".to_vec(),
        }],
    );
    put::<tables::HiredAdvisors>(
        &mut ideas,
        vec![replay_data::rows::HiredAdvisorRow {
            country: 11,
            ordinal: 0,
            character: 2,
            idea: b"advisor_a".to_vec(),
            ..Default::default()
        }],
    );
    put::<tables::ActiveIdeas>(
        &mut ideas,
        vec![replay_data::rows::ActiveIdeaRow {
            country: 10,
            ordinal: 0,
            token: b"advisor_a".to_vec(),
        }],
    );
    let eval_ideas = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(slot)))
    };
    assert_eq!(eval_ideas(&ideas, "has_idea = advisor_a", 1), True);
    assert_eq!(eval_ideas(&ideas, "has_idea = advisor_a", 2), False);
    let faction = (7_u64 << 32) | 88;
    put::<tables::FactionMembers>(
        &mut ideas,
        vec![replay_data::rows::FactionMemberRow {
            country: 10,
            faction,
            ordinal: 0,
        }],
    );
    put::<tables::FactionAdvisors>(
        &mut ideas,
        vec![replay_data::rows::FactionAdvisorRow {
            faction,
            ordinal: 3,
            token: b"advisor_a".to_vec(),
        }],
    );
    assert_eq!(eval_ideas(&ideas, "has_idea = advisor_a", 2), True);
    let mut controlled = frame.clone();
    put::<tables::CountryCounterInputs>(
        &mut controlled,
        vec![replay_data::rows::CountryCounterInputRow {
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
            country: 10,
            owned_state_count: 1,
            controlled_state_count: 3,
            ..Default::default()
        }],
    );
    assert_eq!(
        eval_ideas(&controlled, "num_of_controlled_states > 2", 2),
        True
    );
    assert_eq!(
        eval_ideas(&controlled, "num_of_controlled_states > 3", 2),
        False
    );
    let mut orphan = frame.clone();
    put::<tables::PartyLeaders>(&mut orphan, vec![leader(9, 0, 1, 11)]);
    let world = World::new(&orphan, assets.trigger_content.facts()).unwrap();
    match world.evaluate(
        &parse("has_country_leader = { id = 11 }"),
        Scopes::of(Scope::Country(2)),
    ) {
        script::Eval::Unknown(reasons) => assert!(
            reasons
                .iter()
                .any(|r| r.contains("no political parties row")),
            "{reasons:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn countries_with_original_tag_visit_the_recorded_bucket_then_append_the_selected_raw() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::{rows::CountryAppearanceRow, rows::Recorded, rows::ReservedCountryNamingRow};
    let assets = assets();
    let mut frame = positive_frame();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries.push(CountryRow {
        id: 12,
        canonical_slot: 3,
        tag: b"D01".to_vec(),
        ..Default::default()
    });
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 12,
        canonical_slot: 3,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    let appearance = |country: u32, original: i32, original_tag: &[u8]| CountryAppearanceRow {
        country,
        original_tag_reference: Recorded::Value(Some(original)),
        original_tag: Recorded::Value(Some(original_tag.to_vec())),
        ..Default::default()
    };
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![
            appearance(10, 0, b""),
            appearance(11, 0, b""),
            appearance(12, 10, b"GER"),
        ],
    );
    put::<tables::ReservedCountryNaming>(
        &mut frame,
        vec![ReservedCountryNamingRow {
            original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![], vec![12], vec![]])),
            ..Default::default()
        }],
    );
    let mut counters = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    counters.push(replay_data::rows::CountryCounterInputRow {
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
        country: 12,
        owned_state_count: 1,
        ..Default::default()
    });
    put::<tables::CountryCounterInputs>(&mut frame, counters);
    let eval_frame = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(1)))
    };
    let eval = |source: &str| eval_frame(&frame, source);
    use script::Eval::{False, True};
    let any = |inner: &str| {
        eval(&format!(
            "any_country_with_original_tag = {{ original_tag_to_check = GER {inner} }}"
        ))
    };
    assert_eq!(any("tag = D01"), True);
    assert_eq!(any("tag = GER"), True);
    assert_eq!(any("tag = ENG"), False);
    assert_eq!(any("tag = D01 ROOT = { tag = ENG }"), True);
    assert_eq!(
        eval("all_country_with_original_tag = { original_tag_to_check = GER original_tag = GER }"),
        True
    );
    assert_eq!(
        eval("all_country_with_original_tag = { original_tag_to_check = GER tag = GER }"),
        False
    );
    assert_eq!(any("count = 2 original_tag = GER"), True);
    assert_eq!(any("count = 3 original_tag = GER"), False);
    assert_eq!(any("count = 2.9 tooltip = x original_tag = GER"), True);
    assert_eq!(any("count = 0 tag = ENG"), True);
    let unknown = eval("any_country_with_original_tag = { original_tag_to_check = FRA count = 0 }");
    assert!(matches!(&unknown,
        script::Eval::Unknown(reasons) if reasons.iter().any(|reason| reason.contains("no script variables row"))
    ), "{unknown:?}");
    assert_eq!(
        eval("any_country_with_original_tag = { original_tag_to_check = D01 tag = GER }"),
        True
    );
    assert_eq!(
        eval("any_country_with_original_tag = { original_tag_to_check = ENG tag = D01 }"),
        False
    );
    let mut repeated = frame.clone();
    put::<tables::ReservedCountryNaming>(
        &mut repeated,
        vec![ReservedCountryNamingRow {
            original_tag_buckets: Recorded::Value(Some(vec![
                vec![],
                vec![],
                vec![12, 10, 12],
                vec![],
            ])),
            ..Default::default()
        }],
    );
    let world = World::new(&repeated, assets.trigger_content.facts()).unwrap();
    assert_eq!(
        world.recorded().original_tags().bucket(2),
        Ok(&[12, 10, 12][..])
    );
    for (inner, expected) in [
        ("count = 4 always = yes", True),
        ("count = 5 always = yes", False),
        ("count = 2 tag = D01", True),
        ("count = 3 tag = D01", False),
    ] {
        assert_eq!(
            eval_frame(
                &repeated,
                &format!(
                    "any_country_with_original_tag = {{ original_tag_to_check = GER {inner} }}"
                )
            ),
            expected,
            "{inner}"
        );
    }
    for (buckets, why) in [
        (
            Recorded::NotRecorded,
            "original tag buckets of reserved country naming not recorded",
        ),
        (
            Recorded::Value(None),
            "original tag buckets of reserved country naming capture refused",
        ),
    ] {
        let mut missing = frame.clone();
        put::<tables::ReservedCountryNaming>(
            &mut missing,
            vec![ReservedCountryNamingRow {
                original_tag_buckets: buckets,
                ..Default::default()
            }],
        );
        assert!(matches!(
            eval_frame(
                &missing,
                "any_country_with_original_tag = { original_tag_to_check = GER always = yes }"
            ),
            script::Eval::Unknown(reasons)
                if reasons.iter().any(|reason| reason.contains(why))
        ));
    }
}

#[test]
fn naval_invasion_preparation_reads_the_recorded_operands() {
    content_reader_script::test_game::enter_viewer();
    use replay_data::rows::InvasionPreparationRow;
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.trigger_content.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(2));
        scopes.target_state = Some(20);
        world.evaluate(&parse(source), scopes)
    };
    let row = |progress_hours: i64, leader_term: Option<i64>, leader_applies: u8| {
        InvasionPreparationRow {
            owner_group: 0x100,
            instance: 2,
            progress: progress_hours * 100_000,
            target: 11,
            leader_term,
            leader_applies,
            ..Default::default()
        }
    };
    let prepared =
        "has_naval_invasion_against_state = { state = var:target_state preparation > 0.95 }";
    let unprepared =
        "has_naval_invasion_against_state = { state = var:target_state preparation < 0.5 }";
    let with = |row: InvasionPreparationRow| {
        let mut frame = invasion_frame(0);
        put::<tables::InvasionPreparations>(&mut frame, vec![row]);
        frame
    };
    assert_eq!(script::required_invasion_hours(&row(0, None, 0), 60), 1440);
    let full = with(row(1440, None, 0));
    assert_eq!(eval(&full, prepared), script::Eval::True);
    assert_eq!(eval(&full, unprepared), script::Eval::False);
    let half = with(row(720, None, 0));
    assert_eq!(eval(&half, prepared), script::Eval::False);
    assert_eq!(eval(&half, unprepared), script::Eval::False);
    let early = with(row(100, None, 0));
    assert_eq!(eval(&early, unprepared), script::Eval::True);
    assert_eq!(
        script::required_invasion_hours(&row(0, Some(-50_000), 1), 60),
        720
    );
    assert_eq!(
        script::required_invasion_hours(&row(0, Some(-50_000), 0), 60),
        1440
    );
    assert_eq!(
        eval(&with(row(720, Some(-50_000), 1)), prepared),
        script::Eval::True
    );
    assert_eq!(
        eval(&with(row(720, Some(-50_000), 0)), prepared),
        script::Eval::False
    );
    let mut terms = row(0, None, 0);
    terms.country_term = 10_000;
    terms.relation_term = Some(10_000);
    terms.relation_bonus = Some(-40_000);
    terms.days_term = 1_000_000;
    assert_eq!(script::required_invasion_hours(&terms, 60), 56 * 24);
    let mut withheld = invasion_frame(0);
    *withheld.tables[tables::InvasionPreparations::ID.index()] = TableView::Unavailable {
        reason: "fixed-key refusal: invasion preparation".into(),
    };
    assert!(matches!(
        eval(&withheld, prepared),
        script::Eval::Unknown(_)
    ));
    assert_eq!(
        eval(
            &withheld,
            "has_naval_invasion_against_state = var:target_state"
        ),
        script::Eval::True
    );
    let mut missing = invasion_frame(0);
    put::<tables::InvasionPreparations>(&mut missing, Vec::new());
    assert!(matches!(eval(&missing, prepared), script::Eval::Unknown(_)));
}

#[test]
fn selector_names_match_as_case_folded_tokens() {
    let building = BuildingType {
        name: "custom_fort".into(),
        tags: vec!["Coastal_Guns".into()],
        ..Default::default()
    };
    let selector = |types: &[&str], tags: &[&str]| content_reader_raids::raids::BuildingSelector {
        types: types.iter().map(|name| (*name).to_owned()).collect(),
        tags: tags.iter().map(|name| (*name).to_owned()).collect(),
        ..Default::default()
    };
    assert_eq!(
        offers::selected(&selector(&["CUSTOM_FORT"], &[]), &building),
        (true, false)
    );
    assert_eq!(
        offers::selected(&selector(&[], &["coastal_guns"]), &building),
        (false, true)
    );
    assert_eq!(
        offers::selected(&selector(&["fort"], &["guns"]), &building),
        (false, false)
    );
}

fn draw(inputs: &RaidInputs<'_>, icons: &RaidIcons, batch: &mut Batch) -> usize {
    let resources = draw_resources();
    let windows = gui::windows(DRAW_GUI);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: gui_resources_viewport(inputs.viewport, inputs.ui_scale),
        time: Some(0.),
    };
    let prepared = prepare(inputs, icons);
    let mut report = icons.report.clone();
    prepared.draw(
        &context,
        assets().priority,
        &mut PoolView::default(),
        batch,
        &mut Vec::new(),
        &mut report, None, None,
    )
}

#[test]
fn captive_target_country_resolves_negative_raw_bits_and_zero_suppresses_province() {
    let assets = assets();
    let mut frame = frame();
    let scope = RaidScope {
        leader_country: Some(-1),
        leader_province: 1,
        ..Default::default()
    };
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(
        target_country(&world, &scope),
        Err("a raid target's country 4294967295 has no CountryLinks row".into())
    );
    assert_eq!(
        target_country(
            &world,
            &RaidScope {
                leader_country: Some(0),
                leader_province: 1,
                ..Default::default()
            }
        ),
        Ok(None)
    );
    drop(world);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: u32::MAX,
        canonical_slot: 1,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
    assert_eq!(target_country(&world, &scope), Ok(Some(1)));
}
