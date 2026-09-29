use super::*;
use crate::table_access::Refusals;
use installed_content::{
    buildings::BuildingType,
    raids::{RaidCategory, RaidType as Kind, TargetType},
    triggers::{self, Triggers},
};
use replay::{
    frame::{DecodedFrame, Metrics, TableView},
    rows::{
        BuildingRow, CountryBuildingResourceRow, CountryLinkRow, CountryRow, FactionMemberRow,
        LexicalTokenRow, ProvinceRow, RaidCooldownRow, RaidCountryRow, RaidPlayerAllowedTypeRow,
        RaidTargetRow, RaidTargetStatusRow, ResourceDefinitionRow, StateBaseResourceRow,
        StateDynamicModifierRow, StateResourceInputRow, StateResourceRow, StateRow, TableId,
        TableRows, TableSchema,
    },
};

// Tokens.
const OILFIELD: u32 = 100;
const COMMANDO: u32 = 101;
const NUKE: u32 = 102;
const DAM: u32 = 200;

fn parse(source: &str) -> Triggers {
    triggers::parse(&installed_content::parser::parse(source.as_bytes()).unwrap().items).unwrap()
}

fn sheet(name: &str, texture: usize, size: [u32; 2], frames: u32) -> SpriteSheet {
    SpriteSheet {
        name: name.into(),
        texture,
        frames,
        size,
    }
}

fn kind(name: &str, target: TargetType) -> Kind {
    Kind {
        name: name.into(),
        category: Some("air".into()),
        show_target: parse("NOT = { is_in_faction_with = FROM }"),
        target_types: vec![target],
        ..Default::default()
    }
}

/// Provinces 1..=4 in state 10 (1, 2) and state 20 (3, 4), in a 4 x 1 map.
fn assets() -> RaidAssets {
    let child = |texture| Child {
        position: [1.0, 1.0],
        centred: true,
        sheet: Some(sheet("strip", texture, [52 * 6, 52], 6)),
    };
    let mut geometry = ProvinceGeometry {
        width: 4,
        height: 1,
        ..Default::default()
    };
    for province in 1..=4 {
        let x = province as i32 - 1;
        geometry.centroids.insert(province, [x, 0]);
        geometry.bounds.insert(province, [x, 0, 1, 1]);
    }
    RaidAssets {
        frame: child(0),
        glyph: Child {
            sheet: None,
            ..child(1)
        },
        plus: Child {
            sheet: Some(sheet("plus", 2, [32, 32], 1)),
            ..child(2)
        },
        glyphs: BTreeMap::from([
            (
                "GFX_raid_type_icon_oilfield".to_owned(),
                sheet("oil", 3, [32, 32], 1),
            ),
            (
                "GFX_raid_type_icon_commando".to_owned(),
                sheet("commando", 4, [32, 32], 1),
            ),
            (UNKNOWN_GLYPH.to_owned(), sheet("unknown", 5, [32, 32], 1)),
            (OTHER_GLYPH.to_owned(), sheet("other", 6, [32, 32], 1)),
        ]),
        catalog: Catalog {
            types: vec![
                kind("oilfield", TargetType::Provinces(vec![1, 2, 3, 4])),
                Kind {
                    fire_only_once: true,
                    ..kind("commando", TargetType::Provinces(vec![2, 3]))
                },
                Kind {
                    any_province: true,
                    ..kind("nuke", TargetType::Provinces(vec![1]))
                },
            ],
            categories: BTreeMap::from([(
                "air".into(),
                RaidCategory {
                    name: "air".into(),
                    visible: parse("has_dlc = Gotterdammerung"),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        },
        priority: 30,
        buildings: installed_content::buildings::Buildings {
            types: BTreeMap::from([(
                "dam".into(),
                BuildingType {
                    name: "dam".into(),
                    need_detection: true,
                    ..Default::default()
                },
            )]),
            ..Default::default()
        },
        dlcs: BTreeSet::from(["Gotterdammerung".to_owned()]),
        state_provinces: BTreeMap::from([(10, vec![1, 2]), (20, vec![3, 4])]),
        neighbor_states: Ok(BTreeMap::from([(10, vec![20]), (20, vec![10])])),
        impassable: BTreeSet::new(),
        terrain: None,
        resource_bonuses: [15_000, 20_000],
        geometry,
        paths: Err("no province graph in the test".into()),
        moves: Err("no province graph in the test".into()),
        recorded_indexes: Default::default(),
        table_indexes: Default::default(),
        batch: Ok(100),
        type_keys: Default::default(),
        template_names: Default::default(),
        glyph_keys: Default::default(),
        template_keys: Default::default(),
        source_distances: Default::default(),
        regions: BTreeMap::new(),
        combat_minimum_time: 4,
        avoid_enemy: true,
        sites: BTreeMap::new(),
        refused_sites: None,
        equipment: Default::default(),
        leader_roles: Ok(LeaderRoles::new(Default::default())),
        invasion_prepare_days: Ok(60),
        defines: Default::default(),
        warnings: Vec::new(),
    }
}

/// CountryCommandPower rows for `slots`: nothing allocated, both capacity
/// modifiers and both ace country modifiers registered at zero.
pub(crate) fn registered_command_power(
    slots: impl IntoIterator<Item = u32>,
) -> Vec<replay::rows::CountryCommandPowerRow> {
    slots
        .into_iter()
        .map(|canonical_slot| replay::rows::CountryCommandPowerRow {
            canonical_slot,
            allocated_command_power_raw: 0,
            max_command_power: Some(0),
            max_command_power_factor: Some(0),
            ace_effectiveness_factor: Some(0),
            air_ace_bonuses_factor: Some(0),
        })
        .collect()
}

/// A RaidCountryInputs row whose two launch modifiers are registered at zero.
pub(crate) fn registered_launch_inputs() -> replay::rows::RaidCountryInputRow {
    replay::rows::RaidCountryInputRow {
        air_range_factor: Some(0),
        naval_commando_range_factor: Some(0),
        ..Default::default()
    }
}

fn put<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn status(ordinal: u32, province: u32) -> RaidTargetStatusRow {
    RaidTargetStatusRow {
        ordinal,
        valid: 1,
        scope: RaidScope {
            province,
            ..Default::default()
        },
        detected: Vec::new(),
    }
}

/// Slots 0 (GER, raw 10) and 1 (ENG, raw 11); GER controls state 10, ENG 20.
fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 5,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    put::<tables::Countries>(
        &mut frame,
        vec![
            CountryRow {
                id: 10,
                canonical_slot: 0,
                tag: b"GER".to_vec(),
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
    put::<tables::CountryLinks>(
        &mut frame,
        vec![
            CountryLinkRow {
                id: 10,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            },
        ],
    );
    put::<tables::Provinces>(
        &mut frame,
        (1..=4)
            .map(|id| {
                let raw = if id <= 2 { 10 } else { 11 };
                ProvinceRow {
                    id,
                    state: if id <= 2 { 10 } else { 20 },
                    owner: raw,
                    controller: raw,
                    ..Default::default()
                }
            })
            .collect(),
    );
    put::<tables::States>(
        &mut frame,
        vec![
            StateRow {
                id: 10,
                owner: 10,
                controller: 10,
                ..Default::default()
            },
            StateRow {
                id: 20,
                owner: 11,
                controller: 11,
                ..Default::default()
            },
        ],
    );
    put::<tables::LexicalTokens>(
        &mut frame,
        [
            (OILFIELD, "oilfield"),
            (COMMANDO, "commando"),
            (NUKE, "nuke"),
            (DAM, "dam"),
        ]
        .into_iter()
        .map(|(token, name)| LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        })
        .collect(),
    );
    put::<tables::RaidCountries>(
        &mut frame,
        vec![
            RaidCountryRow {
                canonical_slot: 0,
                country: 10,
                eligible_types: vec![OILFIELD, COMMANDO, NUKE],
                used_types: Vec::new(),
            },
            RaidCountryRow {
                canonical_slot: 1,
                country: 11,
                eligible_types: vec![OILFIELD],
                used_types: Vec::new(),
            },
        ],
    );
    frame
}

fn raid(id: u32, parent: u32, ordinal: u32, phase: i32) -> RaidRow {
    RaidRow {
        reference_type: 84,
        id,
        parent_slot: parent,
        ordinal,
        phase,
        ..Default::default()
    }
}

fn target(id: u32, province: u32, detected: u8) -> RaidTargetRow {
    RaidTargetRow {
        reference_type: 84,
        id,
        scope: RaidScope {
            province,
            ..Default::default()
        },
        raid_type: OILFIELD,
        detected,
    }
}

#[test]
fn the_raid_priority_is_the_define_as_a_signed_byte() {
    assert_eq!(map_icon_priority(30.0), Ok(30));
    // The byte table keeps the low byte: 286 is 30.
    assert_eq!(map_icon_priority(286.0), Ok(30));
    // A byte above 127 is negative, and sorts below zero.
    assert_eq!(map_icon_priority(200.0), Ok(-56));
    assert_eq!(map_icon_priority(-1.0), Ok(-1));
    assert!(map_icon_priority(30.5).is_err());
}

#[test]
fn membership_follows_ownership_detection_and_show_for() {
    let assets = assets();
    let mut frame = frame();
    // GER raids ENG's province 3; ENG raids GER's 1 undetected and 2 detected;
    // an ended ENG raid on 4 is shown to GER and another is cancelled.
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
    // Ours on 3, the detected incoming on 2; the undetected one draws nothing.
    assert_eq!(by_province.get(&3), Some(&2));
    assert_eq!(by_province.get(&2), Some(&4));
    assert!(!by_province.contains_key(&1));
    // An ended raid shown to us draws the ended frame; ENG's own raid on 4 is
    // its own, so as ENG it is an ended own raid only if shown to ENG.
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
    // Both on province 1: GER's own ended raid and ENG's detected active one.
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
    // Province 1 is ours; 3 has oilfield and commando (two glyphs collapse to
    // other_raid, with the plus); 4 has oilfield only.
    assert!(!offers.contains_key(&1));
    assert_eq!(offers[&3].glyph.as_deref(), Some(OTHER_GLYPH));
    assert!(offers[&3].plus);
    assert_eq!(
        offers[&4].glyph.as_deref(),
        Some("GFX_raid_type_icon_oilfield")
    );
    assert!(!offers[&4].plus);
    assert_eq!(offers[&4].frame, 0);
    // No unit requirement admits a unit, so no offer is launchable.
    assert!(!offers[&4].enabled);
    assert_eq!(icons.report.unverified_tint, 0);
    // A faction partner's targets are not shown.
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
    // GER is the player cache's country, and its list holds the commando only.
    put::<tables::RaidPlayerAllowedTypes>(
        &mut used,
        vec![RaidPlayerAllowedTypeRow {
            country: 10,
            allowed: vec![COMMANDO],
        }],
    );
    assert!(icons_for(&assets, &used, 0).is_empty());
    // A dam needs detection; a cooldown then proves the offer grey.
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
        .push(TargetType::Building(installed_content::raids::BuildingSelector {
            types: vec!["dam".into()],
            ..Default::default()
        }));
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
    // A locked category offers nothing.
    assets.dlcs.clear();
    assert!(icons_for(&assets, &frame, 0).is_empty());
}

/// A glyph no interface file declares is counted: the game would keep the
/// button's previous sprite. A target with no province is counted too.
#[test]
fn undeclared_glyphs_and_unresolved_targets_are_counted() {
    let mut assets = assets();
    // An undeclared type glyph falls back to the unknown glyph, itself undeclared.
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

/// The hour's offers for `ours` with every target evaluated.
fn all_offers(
    assets: &RaidAssets,
    frame: &DecodedFrame,
    ours: u32,
    paths: &(dyn launch::RaidPaths + Sync),
) -> offers::Offers {
    let world = World::new(frame, assets.facts()).unwrap();
    let sweep = offers::Sweep::new(assets, &world, ours, paths);
    let mut cycle = cycle::Cycle::new(frame.branch, ours);
    cycle.begin(assets, &sweep).unwrap();
    while !cycle.complete() {
        cycle.step(&sweep);
    }
    cycle.offers()
}

/// The shown offers' raid types for GER at province 4.
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

/// The player cache's own country is offered the recorded list
/// (`CPlayerStatusCache+0x10`), `allowed` as of the load or the last player
/// switch; every other country, and the same tag held by another country
/// after a switch, evaluates each type's `allowed` at the shown hour.
#[test]
fn the_cache_country_uses_the_recorded_list_and_others_evaluate_allowed() {
    let mut assets = assets();
    // Oilfield is not allowed now; the cache still lists it from the load.
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
    // Another country: allowed at this hour, which the oilfield fails.
    put::<tables::RaidPlayerAllowedTypes>(
        &mut frame,
        vec![RaidPlayerAllowedTypeRow {
            country: 11,
            allowed: vec![OILFIELD],
        }],
    );
    assert!(offered(&assets, &frame).0.is_empty());
    // GER's tag now held by another country (raw 12): the cache's raw 10
    // is not it, so it is evaluated too.
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
    assets.catalog.types[0].allowed = Triggers::default();
    assert_eq!(
        offered(&assets, &frame).0,
        ["oilfield"],
        "an empty allowed allows"
    );
}

/// An `allowed` the evaluator cannot decide draws the offer grey and counts
/// it, never a proved tint.
#[test]
fn an_undecided_allowed_draws_the_offer_grey_and_counts_it() {
    let mut assets = assets();
    assets.catalog.types[0].allowed = parse("future_trigger = yes");
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 4)]);
    let icons = icons(&assets, &frame, 0, &launch::Unreconstructed);
    let icon = icons.icons.iter().find(|icon| icon.province == 4).unwrap();
    assert!(!icon.enabled);
    assert_eq!(icons.report.unverified_tint, 1);
    assert!(
        icons.report.reasons.keys().any(
            |reason| reason.contains("oilfield's allowed") && reason.contains("future_trigger")
        )
    );
}

#[test]
fn an_unsupported_trigger_withholds_the_province_and_names_itself() {
    let mut assets = assets();
    assets.catalog.types[0].show_target = parse("future_trigger = yes");
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
            .any(|reason| reason.contains("future_trigger"))
    );
}

/// A state building stands on the first province of the state's runtime
/// list with a site for it; that list keeps the content's order only while
/// its membership equals the content's.
#[test]
fn a_state_building_anchors_in_content_order_only_while_membership_matches() {
    let mut assets = assets();
    assets.buildings.types.insert(
        "airfield".into(),
        BuildingType {
            name: "airfield".into(),
            air_base: true,
            ..Default::default()
        },
    );
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
    let world = World::new(&frame, assets.facts()).unwrap();
    assert_eq!(assets.anchor_province(&world, &scope), Ok(Some(3)));
    drop(world);
    // Province 2 moved into state 20: the native order is unknown.
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[1].state = 20;
    put::<tables::Provinces>(&mut frame, provinces);
    let world = World::new(&frame, assets.facts()).unwrap();
    assert!(assets.anchor_province(&world, &scope).is_err());
}

/// A kept offer icon on a state building is laid out again when its
/// state's province order changes, though its offer is unchanged; kept
/// icons equal icons laid out afresh.
#[test]
fn a_kept_state_building_icon_follows_its_states_order() {
    let mut assets = assets();
    assets.batch = Ok(1);
    assets.buildings.types.insert(
        "airfield".into(),
        BuildingType {
            name: "airfield".into(),
            air_base: true,
            ..Default::default()
        },
    );
    assets.sites.insert((20, Site::AirBase), vec![4, 3]);
    assets.catalog.types[0].target_types =
        vec![TargetType::Building(installed_content::raids::BuildingSelector {
            types: vec!["airfield".into()],
            ..Default::default()
        })];
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
    // Each hour evaluates one row: the airfield, then the other target.
    let mut hour = |frame: &DecodedFrame, cycle: &mut cycle::Cycle| {
        let world = World::new(frame, assets.facts()).unwrap();
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
    // Province 2 moved into state 20: the native order is unknown.
    frame.hour += 1;
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[1].state = 20;
    put::<tables::Provinces>(&mut frame, provinces);
    let second = hour(&frame, &mut cycle);
    assert!(second.icons.is_empty());
    assert_eq!(second.report.withheld, 1);
}

#[test]
fn a_state_target_resolves_to_the_province_nearest_its_centre() {
    let assets = assets();
    let scope = RaidScope {
        state: 20,
        ..Default::default()
    };
    // State 20 spans x 2..4; the centre x 3 is province 4's pixel.
    assert_eq!(assets.province(&scope), Some(4));
}

#[test]
fn icons_draw_at_the_top_priority_inside_the_height_band_nearest_last() {
    let assets = assets();
    let icons = RaidIcons {
        icons: vec![
            RaidIcon {
                province: 1,
                frame: 0,
                enabled: true,
                glyph: None,
                plus: false,
            },
            RaidIcon {
                province: 2,
                frame: 4,
                enabled: true,
                glyph: None,
                plus: false,
            },
        ],
        report: RaidReport::default(),
        built_for: None,
    };
    let anchors = BTreeMap::from([(1, [3000.0, 10.0, 1400.0]), (2, [3000.0, 10.0, 1600.0])]);
    let anchor = |province: u32| anchors.get(&province).copied();
    let draw_at = |height: f32| {
        let mut batch = Batch::default();
        let count = draw(
            &RaidInputs {
                assets: &assets,
                anchor: &anchor,
                camera: Camera {
                    center: [3000.0, 1500.0],
                    height,
                    map: crate::map::camera::MapSize::TEST,
                    defines: crate::map::camera::MapDefines::test(),
                },
                viewport: [2048, 1152],
                ui_scale: 1.0,
            },
            &icons,
            &mut batch,
        );
        (count, batch)
    };
    assert_eq!(draw_at(600.0).0, 0);
    let (count, batch) = draw_at(300.0);
    assert_eq!(count, 2);
    assert!(
        batch
            .quads
            .iter()
            .all(|quad| sprites::layer_priority(quad.layer) == 30)
    );
    // The eye leans south, so province 1 is nearer and draws last.
    let last = batch.quads.iter().max_by_key(|quad| quad.layer).unwrap();
    assert_eq!(last.uv, assets.frame.sheet.as_ref().unwrap().frame_uv(0));
}

/// Until an hour's raid icons are built the window draws none, never
/// another hour's (`Hour::hour_tables_with_raids`, owner decision).
#[test]
fn only_the_hours_own_raid_icons_are_drawn() {
    use std::sync::Arc;
    let icons = Arc::new(RaidIcons {
        icons: vec![RaidIcon {
            province: 3,
            frame: 2,
            enabled: true,
            glyph: None,
            plus: false,
        }],
        report: RaidReport::default(),
        built_for: Some((1, 40, 7)),
    });
    let same = RaidIcons::carried(Arc::clone(&icons), 1, 40, 7);
    assert!(Arc::ptr_eq(&same, &icons), "this hour's own icons");
    assert!(same.report.warnings().is_empty());
    // Another hour, branch or country draws nothing until its own are built.
    for (branch, hour, ours) in [(1, 41, 7), (1, 39, 7), (1, 40, 8), (2, 40, 7)] {
        let other = RaidIcons::carried(Arc::clone(&icons), branch, hour, ours);
        assert!(other.icons.is_empty());
        assert!(other.report.pending);
        assert!(other.report.warnings()[0].contains("still being built"));
    }
    assert!(RaidIcons::carried(Arc::default(), 1, 40, 7).report.pending);
}

/// A raid type the port cannot use (declared twice, or unreadable) is still
/// in the game's database: a target it may match has its offers withheld,
/// naming the type, rather than missing.
#[test]
fn a_type_the_port_cannot_use_leaves_its_targets_undecided_by_name() {
    let mut assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3), status(1, 4)]);
    // Oilfield only on province 3 now; the uncertain type on 4 alone.
    assets.catalog.types[0].target_types = vec![TargetType::Provinces(vec![3])];
    assets.catalog.types.truncate(1);
    let declaration = kind("mine", TargetType::Provinces(vec![4]));
    assets.catalog.uncertain = vec![installed_content::raids::UncertainType {
        name: "mine".into(),
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
    // An unreadable type may match anything.
    assets.catalog.uncertain[0].declarations.clear();
    assert!(
        icons_for(&assets, &frame, 0).is_empty(),
        "both provinces are undecided"
    );
    // A target no declaration matches is decided without it.
    assets.catalog.uncertain[0].declarations = vec![kind("mine", TargetType::Provinces(vec![9]))];
    let decided = icons(&assets, &frame, 0, &launch::Unreconstructed);
    assert_eq!(decided.icons.len(), 1);
    assert_eq!(decided.report.withheld, 0);
}

/// Without the player cache the viewing country may be the cache's own,
/// whose list holds `allowed` as of the load: a type not allowed now is
/// undecided, drawn grey and counted, not dropped.
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

/// A viewing country with no raid status has no offers.
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

/// An incoming raid of a type not in the content draws the unknown glyph
/// where the game draws the type's own: counted and named.
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

/// `CScriptedTrigger` reads yes or no alone; any other value, a parameter
/// block included, loads as no, and `$PARAM$` is not substituted
/// (`14138D3D0`, `vf22@14138D030`).
#[test]
fn a_scripted_trigger_with_parameters_reads_as_no_like_the_game() {
    let mut assets = assets();
    assets
        .catalog
        .scripted_triggers
        .insert("is_germany".into(), parse("tag = GER"));
    assets
        .catalog
        .scripted_triggers
        .insert("is_param".into(), parse("tag = $who$"));
    let frame = frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    let eval = |source: &str| world.evaluate(&parse(source), Scopes::of(Scope::Country(0)));
    assert_eq!(eval("is_germany = yes"), script::Eval::True);
    assert_eq!(eval("is_germany = no"), script::Eval::False);
    assert_eq!(eval("is_germany = { who = GER }"), script::Eval::False);
    // Unsubstituted, `$who$` names no country: no, then negated.
    assert_eq!(eval("is_param = { who = GER }"), script::Eval::True);
}

/// A researched technology without a recorded definition name could be the
/// one asked for: only a named match is True, and False needs every row named.
#[test]
fn has_tech_is_unknown_when_a_researched_row_has_no_definition() {
    use replay::rows::{ResearchTechnologyRow, TechnologyDefinitionRow};
    let assets = assets();
    let mut frame = frame();
    put::<tables::TechnologyDefinitions>(
        &mut frame,
        vec![TechnologyDefinitionRow {
            technology_handle: 1,
            id: b"radar".to_vec(),
            ..Default::default()
        }],
    );
    let researched = |handle| ResearchTechnologyRow {
        country: 10,
        technology_handle: handle,
        researched: 1,
        ..Default::default()
    };
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(0)))
    };
    put::<tables::ResearchTechnologies>(&mut frame, vec![researched(1)]);
    assert_eq!(eval(&frame, "has_tech = radar"), script::Eval::True);
    assert_eq!(eval(&frame, "has_tech = sonar"), script::Eval::False);
    // Handle 2 has no definition row: it may be sonar.
    put::<tables::ResearchTechnologies>(&mut frame, vec![researched(1), researched(2)]);
    assert_eq!(eval(&frame, "has_tech = radar"), script::Eval::True);
    assert!(matches!(
        eval(&frame, "has_tech = sonar"),
        script::Eval::Unknown(_)
    ));
}

/// A trigger input that is missing is unknown, never read as false.
#[test]
fn missing_trigger_inputs_are_unknown_rather_than_false() {
    let assets = assets();
    let mut frame = frame();
    let unknown = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        matches!(
            world.evaluate(&parse(source), Scopes::of(Scope::Country(0))),
            script::Eval::Unknown(_)
        )
    };
    // No Nukes row for the country.
    assert!(unknown(&frame, "num_of_nukes > 0"));
    // No focus named.
    assert!(unknown(&frame, "has_completed_focus = { }"));
    // A faction row whose country links to no slot may be ours.
    put::<tables::FactionMembers>(
        &mut frame,
        vec![replay::rows::FactionMemberRow {
            country: 99,
            faction: 1,
            ordinal: 0,
        }],
    );
    assert!(unknown(&frame, "is_in_faction_with = ENG"));
    // A war row between unlinked countries likewise.
    put::<tables::FactionMembers>(&mut frame, Vec::new());
    put::<tables::WarRelations>(
        &mut frame,
        vec![replay::rows::WarRelationRow {
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

/// Military access reads `RelationLinks` for the owner asked by binary
/// search; a row of an unlinked country leaves an absent answer unknown.
#[test]
fn military_access_reads_only_the_owner_asked() {
    use installed_content::unit_path::MoveWorld;
    use replay::rows::RelationLinkRow;
    let assets = assets();
    let mut frame = frame();
    put::<tables::RelationLinks>(
        &mut frame,
        vec![
            RelationLinkRow {
                country: 10,
                target_index: 1,
                links: RelationLinkRow::MILITARY_ACCESS,
            },
            RelationLinkRow {
                country: 11,
                target_index: 0,
                links: 1,
            },
            RelationLinkRow {
                country: 99,
                target_index: 0,
                links: RelationLinkRow::MILITARY_ACCESS,
            },
        ],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    let cells = reach::Cells::default();
    let moves = reach::Moves::new(&world, &assets, &cells);
    assert_eq!(moves.military_access(0, 1), Ok(true));
    assert_eq!(moves.military_access(1, 0), Ok(false));
    assert_eq!(moves.military_access(1, 1), Ok(false));
    // An unlinked country's row toward slot 0 may be anyone's.
    assert!(moves.military_access(0, 0).is_err());
}

/// Incomplete raid tables withhold the hour's offers, by name: nothing is
/// left to evaluate at that hour.
#[test]
fn incomplete_raid_tables_withhold_the_hours_offers() {
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

/// Rows whose country links to no slot leave the triggers that read them
/// unknown where they could hold the answer.
#[test]
fn unlinked_focus_project_and_party_rows_leave_their_triggers_unknown() {
    use replay::rows::{CompletedFocusRow, PoliticalPartyRow};
    let assets = assets();
    let mut frame = frame();
    let unknown = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        matches!(
            world.evaluate(&parse(source), Scopes::of(Scope::Country(0))),
            script::Eval::Unknown(_)
        )
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
            is_ruling: 1,
            token: b"fascism".to_vec(),
            ..Default::default()
        }],
    );
    assert!(unknown(&frame, "has_completed_focus = focus"));
    assert!(unknown(&frame, "has_government = fascism"));
    put::<tables::CompletedFocuses>(&mut frame, Vec::new());
    put::<tables::PoliticalParties>(&mut frame, Vec::new());
    assert!(!unknown(&frame, "has_completed_focus = focus"));
    assert!(!unknown(&frame, "has_government = fascism"));
}

/// A used type or a player-cache type whose token has no literal may be any
/// type: the types it could be are undecided rather than dropped.
#[test]
fn raid_type_tokens_without_a_literal_leave_the_types_undecided() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::RaidTargetStatuses>(&mut frame, vec![status(0, 3)]);
    // Commando is fire_only_once; an unnamed used token may be it.
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
    // GER's recorded cache lists only an unnamed token: any type may be
    // allowed, so the province's offers are undecided, never dropped.
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

/// Unverified tints and withheld icons are listed for diagnostics mode.
#[test]
fn the_report_lists_unverified_and_withheld_icons_with_their_reasons() {
    let mut report = RaidReport {
        unverified_tint: 2,
        withheld: 1,
        ..RaidReport::default()
    };
    report.note(["the path search is unknown".to_owned()]);
    let lines = report.warnings();
    assert!(lines[0].contains("2 offers drawn grey with an unverified tint"));
    assert!(lines[0].contains("1 provinces withheld"));
    assert!(lines[1].contains("the path search is unknown (1)"));
    assert!(RaidReport::default().warnings().is_empty());
}

/// Two icons drawn with `ui_scale` at `height` over a viewport of `viewport`.
fn drawn(height: f32, viewport: [u32; 2], ui_scale: f32) -> (usize, Batch) {
    let assets = assets();
    let icon = |province, frame| RaidIcon {
        province,
        frame,
        enabled: true,
        glyph: None,
        plus: false,
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
            assets: &assets,
            anchor: &anchor,
            camera: Camera {
                center: [3000.0, 1500.0],
                height,
                map: crate::map::camera::MapSize::TEST,
                defines: crate::map::camera::MapDefines::test(),
            },
            viewport,
            ui_scale,
        },
        &icons,
        &mut batch,
    );
    (count, batch)
}

/// `ProvincePresentation_UpdateIcons@14127D260` hides the icons only below
/// `RAID_MAP_ICON_DRAW_DISTANCE_MIN` or above `..._MAX` (10 and 500 in the
/// test defines): both edges draw.
#[test]
fn the_camera_band_includes_both_edges() {
    let viewport = [2048, 1152];
    // Near the ground one anchor leaves the view; inside the band some draw.
    assert!(drawn(10.0, viewport, 1.0).0 > 0);
    assert_eq!(drawn(500.0, viewport, 1.0).0, 2);
    assert_eq!(drawn(9.99, viewport, 1.0).0, 0);
    assert_eq!(drawn(500.01, viewport, 1.0).0, 0);
}

/// Icons are laid out in interface pixels: doubling the UI scale over a
/// viewport of twice the device pixels places and sizes them the same.
#[test]
fn icons_lay_out_in_interface_pixels_at_any_ui_scale() {
    let (count, single) = drawn(300.0, [2048, 1152], 1.0);
    let (_, double) = drawn(300.0, [4096, 2304], 2.0);
    assert_eq!(count, 2);
    let rects = |batch: &Batch| batch.quads.iter().map(|quad| quad.rect).collect::<Vec<_>>();
    let (a, b) = (rects(&single), rects(&double));
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        for axis in 0..4 {
            assert!((a[axis] - b[axis]).abs() < 1e-3, "{a:?} {b:?}");
        }
    }
    // At 1.5 the same device viewport holds fewer interface pixels.
    let (_, scaled) = drawn(300.0, [2048, 1152], 1.5);
    assert!(rects(&scaled)[0][0] < a[0][0]);
}

/// An ended incoming raid shown to us draws frame 5 (`SetFrameState@141E71F70`).
#[test]
fn an_ended_incoming_raid_draws_frame_5() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::Raids>(
        &mut frame,
        vec![RaidRow {
            show_for: vec![10],
            outcome: 3,
            ..raid(1, 1, 0, 5)
        }],
    );
    put::<tables::RaidTargets>(&mut frame, vec![target(1, 1, 1)]);
    assert_eq!(icons_for(&assets, &frame, 0).get(&1), Some(&5));
}

const OIL: u32 = 300;
const REFINERY: u32 = 301;
const DAMAGED: u32 = 302;

/// State 10 (GER) holds oil: template 20, pool 3, a refinery at healthy
/// level 2 granting 2 each, an origin with infrastructure 3 and a route.
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
            state: 10,
            gate: 0,
            global_factor: 0,
            origin: 1,
            infrastructure_level: Some(3),
            infrastructure_modifier: 0,
            supply_route: 1,
            // Read at the frame's hour.
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
    assets.buildings.types.insert(
        "refinery".into(),
        BuildingType {
            name: "refinery".into(),
            local_resources: vec![("oil".into(), 200_000)],
            ..Default::default()
        },
    );
    assets
}

fn on_state(assets: &RaidAssets, frame: &DecodedFrame, source: &str) -> script::Eval {
    let world = World::new(frame, assets.facts()).unwrap();
    world.evaluate(&parse(source), Scopes::of(Scope::State(10)))
}

#[test]
fn state_resource_amounts_follow_the_native_order_and_truncation() {
    use script::{OriginFactors, ResourceOperands, state_resource_amount as amount};
    let plain = ResourceOperands {
        template: 2_000_000,
        pool: 300_000,
        additive: 100_000,
        ..Default::default()
    };
    // The additive key counts only under the gate.
    assert_eq!(amount(&plain), 2_300_000);
    assert_eq!(
        amount(&ResourceOperands {
            gate: true,
            ..plain
        }),
        2_400_000
    );
    // Factor then global factor, each truncated; a factor below -1 is zero.
    let factored = ResourceOperands {
        factor: -50_000,
        global_factor: 33_333,
        ..plain
    };
    assert_eq!(amount(&factored), 1_150_000 * 133_333 / 100_000);
    assert_eq!(
        amount(&ResourceOperands {
            factor: -200_000,
            cost: 50_000,
            grants: 7,
            ..plain
        }),
        -49_993
    );
    // Origin factors: 1 + 3 * 0.15, a positive modifier scaled by the
    // level, a negative one added as is; the supply route adds 0.2.
    let origin = |modifier, supply_route| OriginFactors {
        infrastructure: Some((3, modifier)),
        supply_route,
        bonuses: [15_000, 20_000],
    };
    let with = |origin| {
        amount(&ResourceOperands {
            origin: Some(origin),
            ..plain
        })
    };
    assert_eq!(with(origin(0, false)), 2_300_000 * 145_000 / 100_000);
    assert_eq!(with(origin(10_000, false)), 2_300_000 * 175_000 / 100_000);
    assert_eq!(
        with(origin(-20_000, true)),
        2_300_000 * 125_000 / 100_000 * 120_000 / 100_000
    );
    assert_eq!(
        with(OriginFactors {
            infrastructure: None,
            ..origin(-20_000, false)
        }),
        2_300_000
    );
    // A negative scaled amount clamps to zero before cost and grants.
    assert_eq!(
        amount(&ResourceOperands {
            template: -5_000_000,
            cost: 1,
            grants: 10,
            ..plain
        }),
        9
    );
}

#[test]
fn resource_comparisons_truncate_to_whole_32_bit_units() {
    use installed_content::triggers::Comparison::{Equal, Greater, GreaterEqual, Less};
    use script::resource_comparison as compare;
    assert_eq!(compare(2_499_999, Greater, 24), Some(false));
    assert_eq!(compare(2_500_000, Greater, 24), Some(true));
    assert_eq!(compare(-150_000, Equal, -1), Some(true));
    assert_eq!(compare(-150_000, Less, 0), Some(true));
    assert_eq!(compare(((1_i64 << 32) + 5) * 100_000, Equal, 5), Some(true));
    assert_eq!(compare(2_500_000, GreaterEqual, 24), None);
}

#[test]
fn building_grants_use_healthy_levels_and_keep_positive_override_sums() {
    use script::building_grant as grant;
    assert_eq!(grant(0, 100_000, None), 0);
    assert_eq!(grant(-1, 100_000, Some(1)), 0);
    assert_eq!(grant(2, -100_000, None), -200_000);
    assert_eq!(grant(2, 100_000, Some(-100_000)), 0);
    assert_eq!(grant(2, 100_000, Some(50_000)), 300_000);
    assert_eq!(
        grant(i16::MAX, i64::MAX, None),
        i64::MAX.wrapping_mul(32767)
    );
}

#[test]
fn oil_and_the_damaged_modifier_decide_the_state_selector() {
    let assets = resource_assets();
    let mut frame = resource_frame();
    // (20 + 3) * 1.45 * 1.2 + 2 * 2 = 44.02
    let world = World::new(&frame, assets.facts()).unwrap();
    assert_eq!(world.state_resource(10, OIL), Ok(4_402_000));
    drop(world);
    let selector =
        "oil > 44 NOT = { has_dynamic_modifier = { modifier = refinery_damaged_modifier } }";
    assert_eq!(on_state(&assets, &frame, selector), script::Eval::False);
    assert_eq!(on_state(&assets, &frame, "oil > 43"), script::Eval::True);
    // Only the first entry of a definition counts, and only unscoped.
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
    // An override of -3 leaves 2 - 3 < 0: the refinery grants nothing.
    put::<tables::CountryBuildingResources>(&mut frame, vec![override_row(0, 0, 1, -300_000)]);
    let world = World::new(&frame, assets.facts()).unwrap();
    assert_eq!(world.state_resource(10, OIL), Ok(4_002_000));
    drop(world);
    // An empty first entry means no override, whatever later entries hold.
    put::<tables::CountryBuildingResources>(
        &mut frame,
        vec![override_row(0, 0, 0, 0), override_row(1, 1, 1, -300_000)],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    assert_eq!(world.state_resource(10, OIL), Ok(4_402_000));
}

#[test]
fn missing_inputs_and_unsupported_forms_stay_unknown() {
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
    // A state with no inputs row is not decided either.
    let frame = resource_frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    assert!(
        world
            .evaluate(&parse("oil > 3"), Scopes::of(Scope::State(20)))
            .ne(&script::Eval::True)
    );
}

/// `CTrigger::LoadKey@1405434F0` makes every building type of the content a
/// count trigger (`CBuildingCountTrigger::GetValue@14138FEB0`): a state
/// building reads the state's level, a province building sums its levels
/// over the state's recorded provinces, damage included. A name no loader
/// knows is refused by name, never read as zero.
#[test]
fn every_content_building_type_is_a_count_trigger() {
    let mut assets = resource_assets();
    assets.buildings.types.insert(
        "dock".into(),
        BuildingType {
            name: "dock".into(),
            province_max: Some(10),
            ..Default::default()
        },
    );
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
    // Provinces 1 and 2 are state 10's: 2 + 3 levels, damaged ones included.
    assert_eq!(on_state(&assets, &frame, "dock > 4"), script::Eval::True);
    assert_eq!(on_state(&assets, &frame, "dock > 5"), script::Eval::False);
    assert_eq!(on_state(&assets, &frame, "dock = 5"), script::Eval::True);
    // The refinery is a state building: level 3.
    assert_eq!(
        on_state(&assets, &frame, "refinery < 4"),
        script::Eval::True
    );
    // Country scope sums over what the country owns.
    let world = World::new(&frame, assets.facts()).unwrap();
    assert_eq!(
        world.evaluate(&parse("dock = 5"), Scopes::of(Scope::Country(0))),
        script::Eval::True
    );
    assert_eq!(
        world.evaluate(&parse("dock = 7"), Scopes::of(Scope::Country(1))),
        script::Eval::True
    );
    drop(world);
    // Without the building in the content the name is refused, by name.
    assets.buildings.types.remove("dock");
    match on_state(&assets, &frame, "dock > 4") {
        script::Eval::Unknown(reasons) => {
            assert!(reasons.iter().any(|reason| reason.contains("dock")));
        }
        other => panic!("{other:?}"),
    }
    // A registered trigger wins over a scripted one of the same name, and a
    // building type over a scripted trigger.
    assets
        .catalog
        .scripted_triggers
        .insert("refinery".into(), parse("always = no"));
    assets
        .catalog
        .scripted_triggers
        .insert("is_coastal".into(), parse("always = yes"));
    assert_eq!(
        on_state(&assets, &frame, "refinery < 4"),
        script::Eval::True
    );
    // The registered trigger reads the content map, absent here.
    assert!(matches!(
        on_state(&assets, &frame, "is_coastal = yes"),
        script::Eval::Unknown(_)
    ));
}

/// `CExistsTrigger::vf22@1403D08A0` tests the owned state count
/// (`CCountry+0x484`); a Countries row alone does not exist.
#[test]
fn exists_reads_the_owned_state_count() {
    use replay::rows::CountryCounterInputRow;
    let assets = assets();
    let mut frame = frame();
    let exists = |frame: &DecodedFrame, slot| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse("exists = yes"), Scopes::of(Scope::Country(slot)))
    };
    assert!(matches!(exists(&frame, 0), script::Eval::Unknown(_)));
    put::<tables::CountryCounterInputs>(
        &mut frame,
        vec![
            CountryCounterInputRow {
                country: 10,
                owned_state_count: 2,
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 11,
                owned_state_count: 0,
                ..Default::default()
            },
        ],
    );
    assert_eq!(exists(&frame, 0), script::Eval::True);
    assert_eq!(exists(&frame, 1), script::Eval::False);
}

/// A daily fact decides from its latest recorded rows, whatever hour they
/// were read at.
#[test]
fn a_daily_fact_decides_from_its_latest_recorded_rows() {
    let assets = assets();
    let mut frame = frame();
    put::<tables::CompletedFocuses>(
        &mut frame,
        vec![replay::rows::CompletedFocusRow {
            country: 10,
            ordinal: 0,
            token: b"raid_focus".to_vec(),
        }],
    );
    let eval = |frame: &DecodedFrame| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(
            &parse("has_completed_focus = raid_focus"),
            Scopes::of(Scope::Country(0)),
        )
    };
    assert_eq!(eval(&frame), script::Eval::True);
}

/// A memo row searched outward from any hint is the one a binary search
/// finds, and a target the rows lack is none.
#[test]
fn a_memo_row_found_from_a_hint_is_the_binary_searchs() {
    let rows: Vec<replay::rows::RaidPathDistanceRow> = (1..=20)
        .map(|at| replay::rows::RaidPathDistanceRow {
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

/// An IF with its ELSE_IF and ELSE is one trigger: the first branch whose
/// limit holds decides, and an unknown limit leaves it unknown unless every
/// branch it could take agrees.
#[test]
fn if_else_chains_take_the_first_branch_whose_limit_holds() {
    let assets = assets();
    let frame = frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    let eval = |source: &str| world.evaluate(&parse(source), Scopes::of(Scope::Country(0)));
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
        eval("if = { limit = { future_trigger = yes } always = no } else = { always = yes }"),
        script::Eval::Unknown(_)
    ));
    assert_eq!(
        eval("if = { limit = { future_trigger = yes } always = yes } else = { always = yes }"),
        script::Eval::True,
        "both branches agree"
    );
}

/// FROM is the target country and PREV the scope before the last change;
/// an absent scope is false, as in the game.
#[test]
fn from_and_prev_follow_the_scope_chain() {
    let assets = assets();
    let frame = frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    let raid = Scopes {
        from: Some(Scope::Country(1)),
        ..Scopes::of(Scope::Country(0))
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
    let bare = Scopes::of(Scope::Country(0));
    assert_eq!(eval("FROM = { always = yes }", bare), script::Eval::False);
    assert_eq!(eval("PREV = { always = yes }", bare), script::Eval::False);
}

#[test]
fn a_state_scope_from_resolves_to_the_state_owner() {
    let assets = assets();
    let frame = frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    let from_self = Scopes {
        from: Some(Scope::State(20)),
        ..Scopes::of(Scope::State(20))
    };
    assert_eq!(
        world.evaluate(&parse("is_owned_by = FROM"), from_self),
        script::Eval::True
    );
}

// --- Launch gate --------------------------------------------------------

/// Frame [`frame`] with the lexer's literal of `token` replaced.
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

/// Raid type tokens match the content's type names under the lexer's fold,
/// ASCII A-Z only, as the game finds one token per folded key.
#[test]
fn raid_type_tokens_match_under_the_lexers_fold() {
    let assets = assets();
    let frame = spelled(OILFIELD, "OilField");
    let world = World::new(&frame, assets.facts()).unwrap();
    assert_eq!(assets.raid_type(&world, OILFIELD), Ok(TypeToken::Type(0)));
    // A non-ASCII letter does not fold.
    let frame = spelled(OILFIELD, "oilfiEld\u{130}");
    let world = World::new(&frame, assets.facts()).unwrap();
    assert!(assets.raid_type(&world, OILFIELD).is_err());
    // An own raid of a folded spelling draws its type's glyph.
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

/// A spelling two content types share under the fold, a name the content
/// lacks and a token with no literal are refused by name and counted, and
/// what depends on them is undecided rather than dropped.
#[test]
fn ambiguous_and_unknown_raid_tokens_are_refused_and_counted() {
    let mut assets = assets();
    assets
        .catalog
        .types
        .push(kind("OILFIELD", TargetType::Provinces(vec![9])));
    let frame = frame();
    let world = World::new(&frame, assets.facts()).unwrap();
    let ambiguous = assets.raid_type(&world, OILFIELD).unwrap_err();
    assert!(ambiguous.contains("oilfield") && ambiguous.contains("OILFIELD"));
    let assets = self::assets();
    let unknown = spelled(OILFIELD, "no_such_raid");
    let world = World::new(&unknown, assets.facts()).unwrap();
    assert!(
        assets
            .raid_type(&world, OILFIELD)
            .unwrap_err()
            .contains("no_such_raid is not in the content")
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    assert!(
        assets
            .raid_type(&world, 999)
            .unwrap_err()
            .contains("has no recorded literal")
    );
    // An own raid of an unknown type is withheld and counted as refused.
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

/// A used type whose token is refused may be any type: a `fire_only_once`
/// type's offers are undecided by name, not shown as unused.
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

/// Building template tokens match the content's building keys under the
/// fold; an unknown template leaves its target undecided by name.
#[test]
fn building_template_tokens_match_under_the_fold_or_are_refused() {
    let mut assets = assets();
    assets.catalog.types[0].target_types =
        vec![TargetType::Building(installed_content::raids::BuildingSelector {
            types: vec!["dam".into()],
            ..Default::default()
        })];
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
                // A dam needs detection: ours.
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
    use installed_content::raids::{EquipmentRequirement, SourceKind, StartingPoint, UnitRequirement};
    use replay::rows::{
        AirBaseRow, AirWingRow, DivisionRow, DivisionTemplateIconRow, HumanControllerRow, NukeRow,
        RaidAirSiteRow, RaidCountryInputRow, RaidSource, SimulationFeatureRow, TemplateSubunit,
        VariantDefinitionRow, WingEquipmentRow,
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
        assets.equipment = installed_content::equipment::parse(
            &[installed_content::parser::parse(
                b"equipments = { large_plane_airframe = { is_archetype = yes type = strategic_bomber } convoy = { is_archetype = yes type = convoy } }",
            )
            .unwrap()],
            &[],
            &[],
        )
        .unwrap();
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

    /// A bomber raid on provinces 3 and 4 from GER's air base in province 1.
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
                // 1000 km.
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
        let world = World::new(frame, assets.facts()).unwrap();
        let launcher = launch::Launcher::new(assets, &world, 0, &launch::Unreconstructed);
        let scope = RaidScope {
            province,
            ..Default::default()
        };
        launcher.launchable(
            kind,
            &scope,
            Scopes::of(script::Scope::Country(0)),
            launch::Asked::Certain,
        )
    }

    /// Answers every army reach true and no path search.
    struct Reaching;

    impl launch::RaidPaths for Reaching {
        fn distance(
            &self,
            _: &World<'_>,
            _: &launch::PathSource,
            _: &RaidScope,
            _: launch::Asked,
        ) -> Result<Option<i64>, String> {
            Err("no path search".into())
        }
        fn note(&self, _: &launch::PathSource, _: &RaidScope, _: launch::Asked) {}
        fn recorded_rows(&self) -> Option<&[replay::rows::RaidPathDistanceRow]> {
            None
        }
        fn path_target(&self, _: u8, _: &RaidScope) -> Result<Option<u32>, String> {
            Err("no path search".into())
        }
        fn army_reaches(&self, _: &World<'_>, _: u64, _: u32) -> Eval {
            Eval::True
        }
    }

    /// A paratrooper raid from GER's air base on 1 against province 3,
    /// ranged by the best stockpiled transport plane: its variant stats'
    /// `values`.
    fn airborne(values: [i64; 10]) -> Eval {
        use replay::rows::{
            StockpileArchetypeRow, StockpileRow, VariantDefinitionRow, VariantStatsRow,
        };
        const PARATROOPER: u32 = 310;
        const TRANSPORT: u32 = 311;
        const TRANSPORT_VARIANT: u64 = 0x0000_0008_0000_0046;
        let mut assets = launch_assets();
        assets.equipment = installed_content::equipment::parse(
            &[installed_content::parser::parse(
                b"equipments = { transport_plane = { is_archetype = yes type = air_transport } }",
            )
            .unwrap()],
            &[],
            &[],
        )
        .unwrap();
        let kind = Kind {
            command_power_raw: 20 * 100_000,
            essential_equipment: vec![("transport_plane".into(), 1)],
            unit_requirements: vec![UnitRequirement {
                battalions: vec![installed_content::raids::BattalionRequirement {
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
                quantity_raw: 10 * 100_000,
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
            }],
        );
        let world = World::new(&frame, assets.facts()).unwrap();
        let launcher = launch::Launcher::new(&assets, &world, 0, &Reaching);
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        launcher.launchable(
            &kind,
            &scope,
            Scopes::of(script::Scope::Country(0)),
            launch::Asked::Certain,
        )
    }

    /// `GetVariantMapRange@1415A1530` reads the first stat: 100 km of range
    /// reaches province 3, and a range of 0.00001 km in the second stat
    /// cannot stand in for it, nor the other way round.
    #[test]
    fn transport_range_reads_the_first_variant_stat() {
        let mut long_first = [1; 10];
        long_first[0] = 100 * 100_000;
        assert_eq!(airborne(long_first), Eval::True);
        let mut long_second = [1; 10];
        long_second[1] = 100 * 100_000;
        assert_eq!(airborne(long_second), Eval::False);
    }

    /// A cooldown whose type token is refused may be of any type: a target
    /// it covers is undecided by name, one it does not is unaffected.
    #[test]
    fn a_refused_cooldown_type_leaves_its_target_undecided() {
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::RaidCooldowns>(
            &mut frame,
            vec![RaidCooldownRow {
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
        assert_eq!(
            gate(&launch_assets(), &launch_frame(), &bomber(), 3),
            Eval::True
        );
    }

    #[test]
    fn command_power_below_the_cost_or_the_free_capacity_disables_the_offer() {
        let assets = launch_assets();
        let mut kind = bomber();
        kind.command_power_raw = 60 * 100_000;
        // Current power 50 is short even with no other input recorded.
        let mut frame = launch_frame();
        put::<tables::CountryCommandPower>(&mut frame, Vec::new());
        assert_eq!(gate(&assets, &frame, &kind, 3), Eval::False);
        // Capacity (100 + 0) * (1 + 0) - 90 allocated leaves 10 for a cost of 20.
        let mut frame = launch_frame();
        put::<tables::CountryCommandPower>(
            &mut frame,
            vec![replay::rows::CountryCommandPowerRow {
                allocated_command_power_raw: 90 * 100_000,
                ..registered_command_power([0])[0]
            }],
        );
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::False);
    }

    /// A launch modifier the running game does not register by name is
    /// unknown, with the name, never zero.
    #[test]
    fn an_unregistered_capacity_modifier_leaves_the_command_power_gate_unknown() {
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::CountryCommandPower>(
            &mut frame,
            vec![replay::rows::CountryCommandPowerRow {
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
        let assets = launch_assets();
        let mut kind = bomber();
        // 30 convoys less the priority-1 reservation of 20; priority 2 is kept.
        kind.essential_equipment = vec![("convoy".into(), 10)];
        assert_eq!(gate(&assets, &launch_frame(), &kind, 3), Eval::True);
        kind.essential_equipment = vec![("convoy".into(), 11)];
        assert_eq!(gate(&assets, &launch_frame(), &kind, 3), Eval::False);
    }

    #[test]
    fn an_assigned_small_or_unmatched_wing_is_not_a_raid_unit() {
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
        let assets = launch_assets();
        let mut frame = launch_frame();
        let mut wings = frame.complete::<tables::AirWings>().unwrap().to_vec();
        // 1 km: 0.14 pixels, a third of which is under the 0.67 to province 3.
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
    fn air_sources_in_range_decide_and_types_share_only_an_equal_range() {
        let assets = launch_assets();
        let frame = launch_frame();
        let world = World::new(&frame, assets.facts()).unwrap();
        let launcher = launch::Launcher::new(&assets, &world, 0, &launch::Unreconstructed);
        let launch = |kind: &Kind, province: u32| {
            launcher.launchable(
                kind,
                &RaidScope {
                    province,
                    ..Default::default()
                },
                Scopes::of(script::Scope::Country(0)),
                launch::Asked::Certain,
            )
        };
        // The base in province 1 is 0.33 from province 2 and 0.67 from 3.
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
        // The same sources and units, but not the same range.
        assert_eq!(launch(&near, 3), Eval::False);
        assert_eq!(launch(&twin, 3), Eval::True);
        assert_eq!(launch(&near, 2), Eval::True);
        // A target without a map centroid may be in range of every site.
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
        let assets = launch_assets();
        let mut frame = launch_frame();
        put::<tables::NavalBases>(
            &mut frame,
            vec![replay::rows::NavalBaseRow {
                province: 2,
                country: 10,
                ..Default::default()
            }],
        );
        let mut naval = bomber();
        naval.starting_point.types = vec![SourceKind::NavalBase];
        // Only the wing matches, and a wing has no air base at a naval base.
        assert_eq!(gate(&assets, &frame, &naval, 3), Eval::False);
        naval.unit_requirements = vec![UnitRequirement {
            battalions: vec![installed_content::raids::BattalionRequirement {
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
        // One marine battalion is short of the minimum.
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

    /// The launch inputs are daily slices (the game re-checks raid offers in
    /// a rolling sweep, so a day's lag is no harsher), and the gate reads
    /// them whatever hour their owner was last read at.
    #[test]
    fn launch_inputs_read_before_the_hour_still_decide_the_gate() {
        let assets = launch_assets();
        let mut frame = launch_frame();
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);
        // With an ace of a country read before this hour, alive or dead.
        let ace = 0x0000_0009_0000_0047;
        let mut wings = frame.complete::<tables::AirWings>().unwrap().to_vec();
        wings[0].ace = ace;
        put::<tables::AirWings>(&mut frame, wings);
        let row = replay::rows::AceRangeInputRow {
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
            vec![replay::rows::AceRangeInputRow { alive: 0, ..row }],
        );
        assert_eq!(gate(&assets, &frame, &bomber(), 3), Eval::True);
        // A wing whose ace has no row is not guessed.
        put::<tables::AceRangeInputs>(&mut frame, Vec::new());
        assert!(matches!(
            gate(&assets, &frame, &bomber(), 3),
            Eval::Unknown(_)
        ));
    }

    #[test]
    fn a_carrier_launching_a_live_raid_is_no_source() {
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
        // The ship is gone, so its province is none and the distance zero.
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

/// Provinces 1..=4 in a row, every edge 50000; 1 and 2 are sea, 3 and 4 land.
fn path_assets() -> RaidAssets {
    let mut graph = installed_content::province_graph::ProvinceGraph::default();
    for id in 1..=4_u32 {
        let next: Vec<_> = [id.wrapping_sub(1), id + 1]
            .into_iter()
            .filter(|n| (1..=4).contains(n))
            .map(|n| (n, Some(50_000)))
            .collect();
        graph.edges.insert(id, next);
    }
    let mut assets = assets();
    assets.geometry.land = BTreeSet::from([3, 4]);
    assets.moves = Ok(installed_content::unit_path::MoveGraph::new(
        &graph,
        &assets.geometry,
    ));
    assets.paths = Ok(paths::PathSearch::new(graph));
    assets
}

#[test]
fn raid_path_distances_prefer_the_recorded_memo_and_resolve_naval_targets() {
    use launch::{PathSource, RaidPaths};
    let assets = path_assets();
    let mut frame = frame();
    // Province 3 has no base; its state 20's first based province is 4.
    put::<tables::NavalBases>(
        &mut frame,
        vec![replay::rows::NavalBaseRow {
            province: 4,
            level: 1,
            ..Default::default()
        }],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    let target = |province| RaidScope {
        province,
        ..Default::default()
    };
    let source = |kind, province| PathSource {
        kind,
        province,
        country: 0,
    };
    let paths = paths::HourPaths::new(&assets, &frame);
    // Naval: sea 1 -> sea 2 -> the target itself, resolved from 3 to 4.
    assert_eq!(
        paths.distance(&world, &source(2, 1), &target(3), launch::Asked::Certain),
        Ok(None),
        "land 3 is not the target, so it blocks the way to 4"
    );
    assert_eq!(
        paths.distance(&world, &source(2, 3), &target(4), launch::Asked::Certain),
        Ok(Some(50_000))
    );
    // Land: only land provinces are admitted.
    assert_eq!(
        paths.distance(&world, &source(5, 3), &target(4), launch::Asked::Certain),
        Ok(Some(50_000))
    );
    assert_eq!(
        paths.distance(&world, &source(5, 1), &target(4), launch::Asked::Certain),
        Ok(None)
    );
    // The session memo answers a pair whatever kind asks.
    put::<tables::RaidPathDistances>(
        &mut frame,
        vec![replay::rows::RaidPathDistanceRow {
            source: 1,
            target: 4,
            distance: 7,
            ok: 1,
        }],
    );
    let world = World::new(&frame, assets.facts()).unwrap();
    let paths = paths::HourPaths::new(&assets, &frame);
    assert_eq!(
        paths.distance(&world, &source(5, 1), &target(4), launch::Asked::Certain),
        Ok(Some(7))
    );
    assert!(
        paths
            .distance(&world, &source(0, 1), &target(4), launch::Asked::Certain)
            .is_err()
    );
}

#[test]
fn army_reach_reads_the_recorded_hour_and_answers_once_per_source() {
    use launch::RaidPaths;
    use replay::rows::{DivisionMovementRow, DivisionRow, DivisionStatusRow, StateFlagRow};
    let assets = path_assets();
    let mut frame = frame();
    // A division of country 11 (slot 1) in its own province 3.
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
        }]
    };
    put::<tables::StateFlags>(&mut frame, flags(0));
    let reach = |frame: &DecodedFrame| {
        let world = World::new(frame, assets.facts()).unwrap();
        let paths = paths::HourPaths::new(&assets, frame);
        let first = paths.army_reaches(&world, 0x77, 4);
        assert_eq!(paths.army_reaches(&world, 0x77, 4), first);
        first
    };
    assert_eq!(reach(&frame), Eval::True);
    // A demilitarized state closes its provinces to the army.
    put::<tables::StateFlags>(&mut frame, flags(1));
    assert_eq!(reach(&frame), Eval::False);
    // An unrecorded division is unknown, never guessed.
    let world = World::new(&frame, assets.facts()).unwrap();
    let paths = paths::HourPaths::new(&assets, &frame);
    assert!(matches!(
        paths.army_reaches(&world, 0x99, 4),
        Eval::Unknown(_)
    ));
}

/// Countries 10..=13 in slots 0..=3; country 10 at war with 11 and 13, and
/// once with 12 (ended at hour 3).
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
    let war = |first, second, end_hour| replay::rows::WarRelationRow {
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

fn enemies_of(frame: &DecodedFrame, slot: u32) -> installed_content::unit_path::Fact<BTreeSet<u32>> {
    use installed_content::unit_path::MoveWorld;
    let assets = assets();
    let world = World::new(frame, assets.facts()).unwrap();
    let cells = reach::Cells::default();
    let moves = reach::Moves::new(&world, &assets, &cells);
    Ok(moves.enemies(slot)?.into_iter().collect())
}

/// `RebuildWartimeAllies@140D31E20` builds, as a set, the other endpoint of
/// each active war relation the country owns; the derivation from WarRelations
/// equals the vector the game stores, and a vector the collector read (a
/// civil-war revolter or a retired-war owner) is used as recorded.
#[test]
fn enemy_sets_derive_from_war_relations_unless_the_vector_was_read() {
    let mut frame = war_frame();
    assert_eq!(enemies_of(&frame, 0), Ok(BTreeSet::from([1, 3])));
    assert_eq!(enemies_of(&frame, 3), Ok(BTreeSet::from([0])));
    assert_eq!(enemies_of(&frame, 2), Ok(BTreeSet::new()));
    // The native vector in its own order, read as the collector reads it.
    let read = |country, enemies: Vec<u32>| replay::rows::CountryWarRow {
        country,
        can_occupy_non_war: 1,
        enemies_read: 1,
        enemies,
    };
    put::<tables::CountryWars>(&mut frame, vec![read(10, vec![13, 11])]);
    assert_eq!(enemies_of(&frame, 0), Ok(BTreeSet::from([1, 3])));
    // A revolter whose list lags keeps its recorded list.
    put::<tables::CountryWars>(&mut frame, vec![read(12, vec![])]);
    put::<tables::WarRelations>(
        &mut frame,
        vec![replay::rows::WarRelationRow {
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
/// 1 land, 2 sea, 3 land: the naval search reaches 3 through 2, the land
/// search cannot.
fn strait_assets() -> RaidAssets {
    let mut graph = installed_content::province_graph::ProvinceGraph::default();
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

/// The session memo keeps a pair's first asked kind (`RaidPathDistances`).
/// A pair an undecided offer may have asked is decided later only when both
/// kinds agree; a certain ask fixes the kind, answered or not.
#[test]
fn a_pair_an_undecided_offer_may_have_asked_needs_both_kinds_to_agree() {
    use paths::{Asked, Distances};
    let assets = strait_assets();
    let mut frame = frame();
    put::<tables::NavalBases>(
        &mut frame,
        vec![replay::rows::NavalBaseRow {
            province: 3,
            level: 1,
            ..Default::default()
        }],
    );
    let scope = RaidScope {
        province: 3,
        ..Default::default()
    };
    let (naval, land) = (2, 5);
    let fresh = || Distances::new(&assets, &frame);
    let distances = fresh();
    assert!(matches!(
        distances.distance(naval, 1, &scope, Asked::Maybe),
        Ok(Some(_))
    ));
    assert!(distances.distance(land, 1, &scope, Asked::Certain).is_err());
    assert!(
        distances
            .distance(naval, 1, &scope, Asked::Certain)
            .is_err()
    );
    // Asked for certain, the first kind answers the other.
    let distances = fresh();
    assert_eq!(
        distances.distance(land, 1, &scope, Asked::Certain),
        Ok(None)
    );
    assert_eq!(
        distances.distance(naval, 1, &scope, Asked::Certain),
        Ok(None)
    );
    // A measured source no unit could use still fixes the kind.
    let distances = fresh();
    distances.note(naval, 1, &scope, Asked::Certain);
    assert!(matches!(
        distances.distance(land, 1, &scope, Asked::Certain),
        Ok(Some(_))
    ));
    // A maybe-asked pair whose kinds agree stays decided.
    let distances = fresh();
    distances.note(naval, 2, &scope, Asked::Maybe);
    assert_eq!(
        distances.distance(naval, 2, &scope, Asked::Certain),
        distances.distance(naval, 2, &scope, Asked::Maybe)
    );
}

mod recorded_paths {
    use super::*;
    use installed_content::raids::{BattalionRequirement, SourceKind, StartingPoint, UnitRequirement};
    use launch::{Asked, PathSource, RaidPaths};
    use replay::rows::{
        DivisionRow, DivisionTemplateIconRow, NukeRow, RaidPathDistanceRow, TemplateSubunit,
    };

    /// The hour's distances, with every division reaching every source.
    struct Reaching<'a>(paths::HourPaths<'a>);

    impl RaidPaths for Reaching<'_> {
        fn distance(
            &self,
            world: &World<'_>,
            source: &PathSource,
            target: &RaidScope,
            asked: Asked,
        ) -> Result<Option<i64>, String> {
            self.0.distance(world, source, target, asked)
        }
        fn note(&self, source: &PathSource, target: &RaidScope, asked: Asked) {
            self.0.note(source, target, asked);
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

    /// A naval commando raid from GER's naval base on 1 against province 3,
    /// with the pair's distance in the recorded memo.
    fn gate(memo: Vec<RaidPathDistanceRow>) -> Eval {
        gate_in(&assets(), &hour_with(memo), 1)
    }

    /// The hour: GER's naval base on 1 and ENG's on 3, the memo as given.
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
            vec![replay::rows::RaidCountryInputRow {
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
                .map(|province| replay::rows::NavalBaseRow {
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

    /// The gate asked `times` times in one hour: past `INDEX_AFTER` lookups
    /// the answer comes from the memo index the assets share across hours.
    fn gate_in(assets: &RaidAssets, frame: &DecodedFrame, times: usize) -> Eval {
        let kind = commando();
        let world = World::new(frame, assets.facts()).unwrap();
        let paths = Reaching(paths::HourPaths::new(assets, frame));
        let launcher = launch::Launcher::new(assets, &world, 0, &paths);
        let scope = RaidScope {
            province: 3,
            ..Default::default()
        };
        let mut answers: Vec<Eval> = (0..times)
            .map(|_| {
                launcher.launchable(&kind, &scope, Scopes::of(Scope::Country(0)), Asked::Certain)
            })
            .collect();
        answers.dedup();
        assert_eq!(answers.len(), 1, "every lookup answers alike");
        answers.remove(0)
    }

    /// The memo index is shared by the hours whose memo table is the same
    /// allocation, and a changed memo builds its own.
    #[test]
    fn the_memo_index_is_shared_only_while_the_memo_is_unchanged() {
        let assets = assets();
        let near = hour_with(memo(100_000, 1));
        assert_eq!(gate_in(&assets, &near, 70), Eval::True);
        // The next hour shares the memo table: the same index answers.
        let mut next = near.clone();
        next.hour += 1;
        assert_eq!(gate_in(&assets, &next, 70), Eval::True);
        // A new memo allocation with the pair beyond range is indexed anew.
        let far = hour_with(memo(100_001, 1));
        assert_eq!(gate_in(&assets, &far, 70), Eval::False);
        assert_eq!(gate_in(&assets, &near, 70), Eval::True);
    }

    fn memo(distance: i64, ok: u8) -> Vec<RaidPathDistanceRow> {
        vec![RaidPathDistanceRow {
            source: 1,
            target: 3,
            distance,
            ok,
        }]
    }

    /// Every path source's distance recorded decides the offer without a
    /// search; a pair the memo lacks is searched (here without a province
    /// graph, so it stays unknown).
    #[test]
    fn a_recorded_memo_decides_the_path_sources_without_searching() {
        assert_eq!(gate(memo(100_000, 1)), Eval::True);
        assert_eq!(gate(memo(100_001, 1)), Eval::False, "beyond max_distance");
        assert_eq!(gate(memo(0, 0)), Eval::False, "no path");
        assert!(matches!(gate(Vec::new()), Eval::Unknown(_)));
    }
}

mod bench;

mod view_bench;

/// Missing `CountryLinks`: a row whose non-empty country reference links to
/// no slot may be any country's, so no answer it could change is definite.
mod unlinked {
    use super::*;
    use replay::rows::DynamicModifierRow;

    /// A frame whose province 3 is controlled by country 99, which has no
    /// `CountryLinks` row.
    fn unlinked_controller() -> DecodedFrame {
        let mut frame = frame();
        let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
        provinces[2].controller = 99;
        put::<tables::Provinces>(&mut frame, provinces);
        frame
    }

    /// An offer target whose controller does not resolve may be ours: its
    /// offers are withheld by name, not drawn as foreign.
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
        // An empty reference is no country: the target is foreign.
        let mut empty = frame.clone();
        let mut provinces = empty.complete::<tables::Provinces>().unwrap().to_vec();
        provinces[2].controller = 0;
        put::<tables::Provinces>(&mut empty, provinces);
        let drawn = icons(&assets, &empty, 0, &launch::Unreconstructed);
        assert_eq!(drawn.report.withheld, 0);
    }

    /// A foreign raid whose target controller does not resolve may be
    /// aimed at us: withheld and counted, not dropped.
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

    /// `has_dynamic_modifier`: a row of an unlinked country with the name
    /// may be this country's first entry.
    #[test]
    fn an_unlinked_modifier_owner_leaves_the_trigger_undecided() {
        let assets = assets();
        let mut frame = frame();
        let row = |country: u32, token: &[u8]| DynamicModifierRow {
            country,
            token: token.to_vec(),
            ..Default::default()
        };
        put::<tables::DynamicModifiers>(&mut frame, vec![row(10, b"held"), row(99, b"maybe")]);
        let world = World::new(&frame, assets.facts()).unwrap();
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

/// GER (raw 10) holds theatre 7 with one regular group whose front order
/// (type 1, province 1) leads to an unactivated naval invasion (type 3)
/// through provinces 3 and 4, both in ENG's state 20.
fn invasion_frame(army_group: u8) -> DecodedFrame {
    use replay::rows::{
        ArmyOrderPlanRow, ArmyOrderRow, LandTheatreRow, OrdersGroupPlanRow, OrdersGroupRow,
    };
    let mut frame = frame();
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

/// `has_naval_invasion_against_state` finds type-3 orders through the
/// successor graph of a regular group's roots, by path province state; the
/// army-group roster is not read, and a preparation bound stays unknown
/// only when some order touches the state.
#[test]
fn naval_invasions_are_found_through_the_successor_graph() {
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str, state: u32| {
        let world = World::new(frame, assets.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(0));
        scopes.target_state = Some(state);
        world.evaluate(&parse(source), scopes)
    };
    let frame = invasion_frame(0);
    let simple = "has_naval_invasion_against_state = var:target_state";
    assert_eq!(eval(&frame, simple, 20), script::Eval::True);
    // The front order's own province is in state 10, but it is not type 3.
    assert_eq!(eval(&frame, simple, 10), script::Eval::False);
    let numbered = "has_naval_invasion_against_state = 20";
    assert_eq!(eval(&frame, numbered, 0), script::Eval::True);
    // A state that does not exist is false, as the native log-and-return.
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
    // ENG has no theatre.
    let world = World::new(&frame, assets.facts()).unwrap();
    let mut scopes = Scopes::of(Scope::Country(1));
    scopes.target_state = Some(20);
    assert_eq!(world.evaluate(&parse(simple), scopes), script::Eval::False);
    // The same group on the army-group roster is not read.
    assert_eq!(eval(&invasion_frame(1), simple, 20), script::Eval::False);
}

/// `is_ally_with` needs both countries to own a state, then the same
/// country, or a faction or direct subject relation without an active war.
#[test]
fn is_ally_with_follows_the_friend_relation() {
    use replay::rows::{CountryCounterInputRow, SubjectRow, WarRelationRow};
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(0)))
    };
    let mut frame = frame();
    let owned = |eng| {
        vec![
            CountryCounterInputRow {
                country: 10,
                owned_state_count: 1,
                ..Default::default()
            },
            CountryCounterInputRow {
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
    // A country that owns no state is nobody's ally.
    put::<tables::CountryCounterInputs>(&mut frame, owned(0));
    assert_eq!(eval(&frame, "is_ally_with = ENG"), script::Eval::False);
}

/// `any_state_in` reads a global array's `^num` and `^<i>` entries, in the
/// scoped state encoding or as whole fixed5 ids, and needs `count` passes.
#[test]
fn any_state_in_reads_a_global_array() {
    use replay::rows::{ScriptVariableEntry, ScriptVariablesRow};
    let mut assets = assets();
    assets
        .catalog
        .scripted_triggers
        .insert("helper".into(), parse("always = yes"));
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(0)))
    };
    let array = |entries: &[(&str, i64)]| {
        let mut frame = frame();
        put::<tables::ScriptVariables>(
            &mut frame,
            vec![ScriptVariablesRow {
                scope_kind: 0,
                scope_id: 0,
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
    // In a real state the scripted trigger's body runs.
    let helper = |value: &str| format!("any_state_in = {{ array = global.wall helper = {value} }}");
    assert_eq!(eval(&frame, &helper("yes")), script::Eval::True);
    assert_eq!(eval(&frame, &helper("no")), script::Eval::False);
    // An absent array is empty.
    assert_eq!(eval(&array(&[]), english), script::Eval::False);
    // Only the first entry is read when ^num is 1.
    let one = array(&[
        ("wall^num", 100_000),
        ("wall^0", encoded),
        ("wall^1", 2_000_000),
    ]);
    assert_eq!(eval(&one, english), script::Eval::False);
    // An entry naming no state is the null state: every trigger with a
    // scope mask is false there, a mask-0 one evaluates.
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
        // A scripted trigger's template check (mask 0x3FFE) fails in the
        // null state before its body runs.
        assert_eq!(eval(&null, &body("helper = yes")), script::Eval::False);
        assert_eq!(eval(&null, &body("helper = no")), script::Eval::True);
    }
    // A state the content has but the recording lacks is unknown.
    let mut gapped = super::tests::assets();
    gapped.state_provinces.insert(99, vec![1]);
    let missing = array(&[("wall^num", 100_000), ("wall^0", 9_900_000)]);
    let world = World::new(&missing, gapped.facts()).unwrap();
    assert!(matches!(
        world.evaluate(&parse(english), Scopes::of(Scope::Country(0))),
        script::Eval::Unknown(_)
    ));
    // A scope's own array is unknown.
    let local = "any_state_in = { array = wall is_owned_by = ENG }";
    assert!(matches!(eval(&frame, local), script::Eval::Unknown(_)));
}

/// `any_neighbor_state` and `all_neighbor_state` walk the scope state's
/// content neighbours in state scope: `any` needs `count` passes, `all` fails
/// at the first failure, and a list the content or the recording cannot give
/// is unknown.
#[test]
fn neighbour_state_triggers_walk_the_content_neighbours() {
    let frame = frame();
    let eval = |assets: &RaidAssets, scope: Scope, source: &str| {
        let world = World::new(&frame, assets.facts()).unwrap();
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
    // A count below one is met by the first neighbour, whatever the body.
    let none = "any_neighbor_state = { count = 0 is_owned_by = GER }";
    assert_eq!(eval(&base, german, none), script::Eval::True);
    // Outside state scope, or without content neighbours, it is unknown.
    let country = eval(&base, Scope::Country(0), &any("ENG"));
    assert!(matches!(country, script::Eval::Unknown(_)));
    let refused = RaidAssets {
        neighbor_states: Err("no graph".into()),
        ..assets()
    };
    assert!(matches!(
        eval(&refused, german, &any("ENG")),
        script::Eval::Unknown(_)
    ));
    // A neighbour the recording lacks is undecided, not a failure.
    let unrecorded = RaidAssets {
        neighbor_states: Ok(BTreeMap::from([(10, vec![20, 30])])),
        ..assets()
    };
    assert_eq!(eval(&unrecorded, german, &any("ENG")), script::Eval::True);
    assert!(matches!(
        eval(&unrecorded, german, &any("GER")),
        script::Eval::Unknown(_)
    ));
    assert!(matches!(
        eval(&unrecorded, german, &all("ENG")),
        script::Eval::Unknown(_)
    ));
    // A state with no neighbours: none passes, all pass.
    let alone = RaidAssets {
        neighbor_states: Ok(BTreeMap::from([(10, Vec::new())])),
        ..assets()
    };
    assert_eq!(eval(&alone, german, &any("ENG")), script::Eval::False);
    assert_eq!(eval(&alone, german, &all("GER")), script::Eval::True);
}

/// `custom_trigger_tooltip` and `custom_override_tooltip` are an AND of
/// every child but their `tooltip` and `not_tooltip` text, and
/// `is_subject_of` reads the recorded subject relation's overlord.
#[test]
fn custom_tooltips_and_is_subject_of() {
    use replay::rows::SubjectRow;
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(0)))
    };
    let mut frame = frame();
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
    let formatted = "custom_override_tooltip = { tooltip = { localization_key = KEY } tag = GER }";
    assert_eq!(eval(&frame, formatted), script::Eval::True);
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
    // The overlord is not a subject of its subject.
    let world = World::new(&frame, assets.facts()).unwrap();
    let english = Scopes::of(Scope::Country(1));
    assert_eq!(
        world.evaluate(&parse("is_subject_of = GER"), english),
        script::Eval::False
    );
    // An ended relation no longer counts.
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

/// A unit leader row of GER (raw 10) with reference id `id`.
fn leader_row(id: u32, owner: u32, captured: u8) -> replay::rows::UnitLeaderRow {
    replay::rows::UnitLeaderRow {
        id,
        reference_type: 7,
        owner,
        captured,
        ..Default::default()
    }
}

/// `has_any_captured_general`, `is_leader_visible` and `tag` in a unit
/// leader's scope read the recorded leader owner, captured byte and
/// visibility inputs; a leader with a visibility choice whose character is
/// not recorded stays undecided.
#[test]
fn captured_general_triggers_read_the_recorded_leaders() {
    let assets = assets();
    let mut frame = frame();
    let reference = (5_u64 << 32) | 7;
    let eval = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.facts()).unwrap();
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
    // A leader with its own visibility choice and no recorded character is
    // undecided, captured or not.
    let mut chosen = leader_row(5, 10, 1);
    chosen.visibility_choice = 1;
    put::<tables::UnitLeaders>(&mut frame, vec![chosen]);
    assert!(matches!(eval(&frame, rescue, 0), script::Eval::Unknown(_)));
    assert!(matches!(eval(&frame, any, 0), script::Eval::Unknown(_)));
    // Another captured and visible leader decides it anyway.
    let mut chosen = leader_row(5, 10, 1);
    chosen.visibility_choice = 1;
    put::<tables::UnitLeaders>(&mut frame, vec![chosen, leader_row(8, 10, 1)]);
    assert_eq!(eval(&frame, any, 0), script::Eval::True);
    // An unrecorded target leader is no leader.
    put::<tables::UnitLeaders>(&mut frame, Vec::new());
    assert_eq!(
        eval(&frame, "is_leader_visible = var:target_leader", 0),
        script::Eval::False
    );
    // The native trigger ignores its operand: `= no` reads as `= yes`.
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

/// `is_core_of` reads the state's recorded cores through CountryLinks.
#[test]
fn is_core_of_reads_the_recorded_state_cores() {
    use replay::rows::StateCoreRow;
    let assets = assets();
    let mut frame = frame();
    let on_state = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::State(20)))
    };
    // No row for the state: undecided.
    assert!(matches!(
        on_state(&frame, "is_core_of = GER"),
        script::Eval::Unknown(_)
    ));
    put::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 20,
            cores: vec![11, 10],
        }],
    );
    assert_eq!(on_state(&frame, "is_core_of = GER"), script::Eval::True);
    assert_eq!(on_state(&frame, "is_core_of = ENG"), script::Eval::True);
    put::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 20,
            cores: vec![11],
        }],
    );
    assert_eq!(on_state(&frame, "is_core_of = GER"), script::Eval::False);
    // A core that links to no country may be the one asked about.
    put::<tables::StateCores>(
        &mut frame,
        vec![StateCoreRow {
            state: 20,
            cores: vec![99],
        }],
    );
    assert!(matches!(
        on_state(&frame, "is_core_of = GER"),
        script::Eval::Unknown(_)
    ));
}

/// runs in the owner's raid scopes, `launchable_from` in the source
/// country's, and a cooldown or a used-up type blocks it.
#[test]
fn a_prepared_raids_launch_triggers_follow_the_native_chain() {
    let mut assets = assets();
    let mut frame = frame();
    let scope = RaidScope {
        province: 3,
        ..Default::default()
    };
    let decide =
        |assets: &RaidAssets, frame: &DecodedFrame, source: Result<Option<u32>, String>| {
            let world = World::new(frame, assets.facts()).unwrap();
            arrows::launch_triggers(assets, &world, 0, &assets.catalog.types[0], &scope, &source)
        };
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::True);
    // `launchable` sees the owner as ROOT and the target as FROM.
    assets.catalog.types[0].launchable = parse("tag = GER FROM = { tag = ENG }");
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::True);
    assets.catalog.types[0].launchable = parse("tag = ENG");
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::False);
    assets.catalog.types[0].launchable = Triggers::default();
    // `launchable_from` sees the source's country as ROOT.
    assets.catalog.types[0].launchable_from = parse("tag = ENG");
    assert_eq!(decide(&assets, &frame, Ok(Some(1))), Eval::True);
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::False);
    assert!(matches!(
        decide(&assets, &frame, Ok(None)),
        Eval::Unknown(_)
    ));
    assets.catalog.types[0].launchable_from = Triggers::default();
    // `available` then a cooldown on this target and type.
    assets.catalog.types[0].available = parse("always = no");
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::False);
    assets.catalog.types[0].available = Triggers::default();
    put::<tables::RaidCooldowns>(
        &mut frame,
        vec![RaidCooldownRow {
            owner_slot: 0,
            ordinal: 0,
            raid_type: OILFIELD,
            scope: scope.clone(),
        }],
    );
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::False);
    put::<tables::RaidCooldowns>(&mut frame, Vec::new());
    // A used fire-only-once type.
    assets.catalog.types[0].fire_only_once = true;
    let mut used = frame.clone();
    put::<tables::RaidCountries>(
        &mut used,
        vec![RaidCountryRow {
            canonical_slot: 0,
            country: 10,
            eligible_types: vec![OILFIELD],
            used_types: vec![OILFIELD],
        }],
    );
    assert_eq!(decide(&assets, &used, Ok(Some(0))), Eval::False);
    assert_eq!(decide(&assets, &frame, Ok(Some(0))), Eval::True);
}

/// `is_leader_visible` and `has_any_captured_general` evaluate the role
/// `visible` of the recorded template the leader's character was built from,
/// in its owner's scope; without the template columns they are undecided.
#[test]
fn leader_visibility_evaluates_the_recorded_template_role() {
    use replay::rows::CharacterRow;
    use script::RecordedTemplate;
    let path = "common/characters/X.txt";
    let mut assets = assets();
    assets.leader_roles = Ok(LeaderRoles::new(
        installed_content::characters::parse_leader_templates(
            path,
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
    ));
    let mut frame = frame();
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
        let world = World::new(&frame, assets.facts()).unwrap();
        let visible = |row: &replay::rows::UnitLeaderRow, template| {
            world.leader_visible_from(row, template, 0)
        };
        // The role's visible in the owner's scope, by either input.
        for (choice, template) in [(1, 1), (0, 1), (1, 0)] {
            let rommel = Ok(Some(recorded(501, 2)));
            assert_eq!(visible(&leader(10, choice, template), rommel.clone()), True);
            assert_eq!(visible(&leader(11, choice, template), rommel), False);
        }
        // The recorded instance decides, found by its allowed's closing line
        // and its tag under the lexer's fold.
        assert_eq!(visible(&leader(11, 1, 1), Ok(Some(recorded(502, 4)))), True);
        assert_eq!(
            visible(&leader(10, 1, 1), Ok(Some(recorded(502, 4)))),
            False
        );
        // An instance without a role leaves the leader visible through the
        // template alone; with its own choice set it cannot have built it.
        assert_eq!(visible(&leader(10, 0, 1), Ok(Some(recorded(502, 5)))), True);
        undecided(
            visible(&leader(10, 1, 1), Ok(Some(recorded(502, 5)))),
            "has no unit leader role",
        );
        // A role with no visible holds.
        assert_eq!(visible(&leader(11, 1, 1), Ok(Some(recorded(507, 8)))), True);
        undecided(
            visible(&leader(10, 1, 1), Ok(Some(recorded(504, 7)))),
            "refused",
        );
        // A wrong line or tag, another file, a generated template or none.
        undecided(
            visible(&leader(10, 1, 1), Ok(Some(recorded(501, 3)))),
            "no content character template",
        );
        undecided(
            visible(&leader(10, 1, 1), Ok(Some(recorded(502, 2)))),
            "no content character template",
        );
        let elsewhere = RecordedTemplate {
            file: b"common/characters/Y.txt".to_vec(),
            ..recorded(501, 2)
        };
        undecided(
            visible(&leader(10, 1, 1), Ok(Some(elsewhere))),
            "no content character template",
        );
        let generated = RecordedTemplate {
            generated: true,
            ..recorded(501, 2)
        };
        undecided(
            visible(&leader(10, 1, 1), Ok(Some(generated))),
            "generate_character",
        );
        undecided(visible(&leader(10, 1, 1), Ok(None)), "no template");
    }
    // Through the triggers, the character's recorded template columns: a
    // conditioned leader whose character has no template is undecided by
    // name, one with its template evaluates the role, and an unconditioned
    // one is visible.
    let reference = (5_u64 << 32) | 7;
    let through_trigger = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(slot));
        scopes.target_leader = Some(reference);
        world.evaluate(&parse(source), scopes)
    };
    put::<tables::UnitLeaders>(&mut frame, vec![leader(10, 1, 1)]);
    undecided(
        through_trigger(&frame, "is_leader_visible = var:target_leader", 0),
        "no template",
    );
    undecided(
        through_trigger(&frame, "has_any_captured_general = yes", 0),
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
        through_trigger(&with_template, "is_leader_visible = var:target_leader", 0),
        True
    );
    put::<tables::UnitLeaders>(&mut frame, vec![leader(10, 0, 0)]);
    assert_eq!(
        through_trigger(&frame, "is_leader_visible = var:target_leader", 0),
        True
    );
    // Unread content leaves every conditioned leader undecided.
    assets.leader_roles = Err("no characters in the test".into());
    let world = World::new(&frame, assets.facts()).unwrap();
    undecided(
        world.leader_visible_from(&leader(10, 1, 1), Ok(Some(recorded(501, 2))), 0),
        "no characters in the test",
    );
}

/// The character-scope triggers are false in a scope without a character;
/// in a unit leader's scope `has_character_flag` reads the character's
/// recorded flags. `hidden_trigger` is a conjunction, `has_global_flag` reads
/// the global flags, `OVERLORD` enters the recorded overlord and
/// `has_cosmetic_tag` compares the recorded cosmetic tag.
#[test]
fn character_scope_global_flag_overlord_and_cosmetic_triggers() {
    use replay::rows::{CountryAppearanceRow, ScriptFlagRow};
    let assets = assets();
    let mut frame = frame();
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
        let world = World::new(frame, assets.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(0));
        scopes.this = this;
        scopes.target_leader = Some(reference);
        world.evaluate(&parse(source), scopes)
    };
    use script::Eval::{False, True};
    let country = Scope::Country(0);
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
    // Character flags: every complete table holds the character scope, so a
    // character without a row has none.
    assert_eq!(
        eval(&frame, "has_character_flag = exiled", in_leader),
        False
    );
    put::<tables::ScriptFlags>(&mut frame, vec![flag(3, 9)]);
    assert_eq!(
        eval(&frame, "has_character_flag = exiled", in_leader),
        False
    );
    put::<tables::ScriptFlags>(&mut frame, vec![flag(3, 44), flag(0, 0)]);
    assert_eq!(eval(&frame, "has_character_flag = exiled", in_leader), True);
    assert!(matches!(
        eval(&frame, "is_country_leader = no", in_leader),
        script::Eval::Unknown(_)
    ));
    // Global flags in any scope; hidden_trigger is a conjunction.
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
    // OVERLORD and has_cosmetic_tag read CountryAppearance.
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

/// `has_country_leader`, `has_country_leader_with_trait`, `has_civil_war`,
/// `has_idea` and `num_of_controlled_states` over the planned stored values
/// (the rules; `recorded_political_tables_decide_country_leader_triggers`
/// reads the same values from the recorded tables).
#[test]
fn country_leader_civil_war_and_idea_triggers() {
    use replay::rows::{
        ActiveIdeaRow, CharacterRow, CountryAppearanceRow, HiredAdvisorRow, WarRelationRow,
    };
    use script::{PartyLeader, PartyLeaders, Planned};
    let leader = |character: u32, id: i32| PartyLeader {
        character: Some(character),
        id,
    };
    let assets = assets();
    let mut frame = frame();
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
    // The recorded tables decide every trigger here
    // (`recorded_political_tables_decide_country_leader_triggers`); an idea
    // that is no recorded advisor token is an active idea.
    {
        let world = World::new(&frame, assets.facts()).unwrap();
        assert_eq!(
            world.evaluate(&parse(sources[3]), Scopes::of(Scope::Country(0))),
            script::Eval::True
        );
    }
    let planned = || Planned {
        parties: BTreeMap::from([
            (
                0,
                vec![
                    PartyLeaders {
                        ruling: false,
                        leaders: vec![leader(2, 12)],
                    },
                    PartyLeaders {
                        ruling: true,
                        leaders: vec![leader(1, 11), leader(2, 12)],
                    },
                ],
            ),
            (
                1,
                vec![PartyLeaders {
                    ruling: true,
                    leaders: Vec::new(),
                }],
            ),
        ]),
        // Sorted by token: the first GER_karl record is character 1.
        characters: BTreeMap::from([(0, vec![Some(1), Some(3), Some(2)])]),
        traits: BTreeMap::from([(0, vec![702])]),
        civil_wars: BTreeMap::from([(11, vec![10])]),
        advisor_tokens: BTreeSet::from([b"advisor_a".to_vec()]),
        faction_advisors: BTreeMap::new(),
        controlled: BTreeMap::from([(0, 3)]),
    };
    let eval = |frame: &DecodedFrame, source: &str, slot: u32| {
        let mut world = World::new(frame, assets.facts()).unwrap();
        world.planned = Some(planned());
        world.evaluate(&parse(source), Scopes::of(Scope::Country(slot)))
    };
    // has_country_leader: the country's first active character of the token
    // is the ruling party's first leader; without ruling_only any party's.
    assert_eq!(eval(&frame, sources[0], 0), True);
    assert_eq!(
        eval(
            &frame,
            "has_country_leader = { character = GER_otto ruling_only = yes }",
            0
        ),
        False
    );
    assert_eq!(
        eval(
            &frame,
            "has_country_leader = { character = GER_otto ruling_only = no }",
            0
        ),
        True
    );
    assert_eq!(
        eval(&frame, "has_country_leader = { character = GER_otto }", 0),
        True
    );
    // GER_otto is not among ENG's active characters; a ruling party without
    // leaders fails first.
    assert_eq!(
        eval(&frame, "has_country_leader = { character = GER_karl }", 1),
        False
    );
    // `id` matches CCountryLeader+0x148; `name` the character's display
    // name, which an empty one leaves undecided.
    assert_eq!(eval(&frame, "has_country_leader = { id = 12 }", 0), True);
    assert_eq!(
        eval(
            &frame,
            "has_country_leader = { id = 12 ruling_only = yes }",
            0
        ),
        False
    );
    assert_eq!(eval(&frame, "has_country_leader = { id = 0 }", 0), False);
    // An empty display name is what its localised fallback gives again.
    assert_eq!(
        eval(&frame, "has_country_leader = { name = \"Karl\" }", 0),
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
            0
        ),
        False
    );
    assert_eq!(
        eval(&named, "has_country_leader = { name = \"Otto\" }", 0),
        True
    );
    // num_of_controlled_states reads CCountry+0x46C.
    assert_eq!(eval(&frame, "num_of_controlled_states > 2", 0), True);
    assert_eq!(eval(&frame, "num_of_controlled_states > 3", 0), False);
    // has_country_leader_with_trait: the first leader's traits; no leader
    // or an unknown trait is false.
    assert_eq!(eval(&frame, sources[1], 0), True);
    assert_eq!(
        eval(&frame, "has_country_leader_with_trait = other", 0),
        False
    );
    assert_eq!(eval(&frame, sources[1], 1), False);
    // has_civil_war: ENG's own list names an existing GER, or GER's enemy
    // ENG lists it.
    let mut counters = frame
        .complete::<tables::CountryCounterInputs>()
        .unwrap()
        .to_vec();
    for country in [10, 11] {
        counters.push(replay::rows::CountryCounterInputRow {
            country,
            owned_state_count: 1,
            ..Default::default()
        });
    }
    put::<tables::CountryCounterInputs>(&mut frame, counters);
    assert_eq!(eval(&frame, "has_civil_war = yes", 1), True);
    assert_eq!(eval(&frame, "has_civil_war = yes", 0), False);
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
    assert_eq!(eval(&frame, "has_civil_war = yes", 0), True);
    assert_eq!(eval(&frame, "has_civil_war = no", 0), False);
    // has_idea: an advisor token reads the hired advisors (then the
    // faction's), anything else the active ideas.
    assert_eq!(eval(&frame, "has_idea = idea_a", 0), True);
    assert_eq!(eval(&frame, "has_idea = idea_a", 1), False);
    assert_eq!(eval(&frame, "has_idea = advisor_a", 1), True);
    assert_eq!(eval(&frame, "has_idea = advisor_a", 0), False);
}

/// `has_country_leader` and `has_country_leader_with_trait` from the
/// recorded tables: each country's parties (PoliticalParties, the ruling one
/// by `is_ruling`) and their leaders (PartyLeaders), its active characters
/// (CountryCharacters) and its ruling first leader's traits
/// (CountryAppearance), with the planned values of
/// `country_leader_civil_war_and_idea_triggers`.
#[test]
fn recorded_political_tables_decide_country_leader_triggers() {
    use replay::rows::{
        CharacterRow, CountryAppearanceRow, CountryCharacterRow, PartyLeaderRow, PoliticalPartyRow,
    };
    let assets = assets();
    let mut frame = frame();
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
                leader_traits: vec![702],
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
        ],
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
        let world = World::new(&frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(slot)))
    };
    use script::Eval::{False, True};
    for (source, slot, expected) in [
        (
            "has_country_leader = { character = GER_karl ruling_only = yes }",
            0,
            True,
        ),
        (
            "has_country_leader = { character = GER_otto ruling_only = yes }",
            0,
            False,
        ),
        (
            "has_country_leader = { character = GER_otto ruling_only = no }",
            0,
            True,
        ),
        ("has_country_leader = { character = GER_otto }", 0, True),
        // ENG's ruling party has no leaders.
        ("has_country_leader = { character = GER_karl }", 1, False),
        ("has_country_leader = { id = 12 }", 0, True),
        (
            "has_country_leader = { id = 12 ruling_only = yes }",
            0,
            False,
        ),
        ("has_country_leader = { id = 0 }", 0, False),
        // The first leader's traits; no leader or an unknown trait is false.
        ("has_country_leader_with_trait = dictator", 0, True),
        ("has_country_leader_with_trait = other", 0, False),
        ("has_country_leader_with_trait = dictator", 1, False),
    ] {
        assert_eq!(eval(source, slot), expected, "{source} at {slot}");
    }
    // has_civil_war: ENG's own list (CountryCivilWars, by raw country) names
    // an existing GER; GER is at war with nobody and lists nobody.
    let mut wars = frame.clone();
    put::<tables::CountryCivilWars>(
        &mut wars,
        vec![replay::rows::CountryCivilWarRow {
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
        counters.push(replay::rows::CountryCounterInputRow {
            country,
            owned_state_count: 1,
            ..Default::default()
        });
    }
    put::<tables::CountryCounterInputs>(&mut wars, counters);
    let eval_wars = |slot: u32| {
        let world = World::new(&wars, assets.facts()).unwrap();
        world.evaluate(
            &parse("has_civil_war = yes"),
            Scopes::of(Scope::Country(slot)),
        )
    };
    assert_eq!(eval_wars(1), True);
    assert_eq!(eval_wars(0), False);
    // has_idea: a recorded advisor token reads the hired advisors (then the
    // faction's), anything else the active ideas.
    let mut ideas = frame.clone();
    put::<tables::AdvisorTokens>(
        &mut ideas,
        vec![replay::rows::AdvisorTokenRow {
            ordinal: 0,
            token: b"advisor_a".to_vec(),
        }],
    );
    put::<tables::HiredAdvisors>(
        &mut ideas,
        vec![replay::rows::HiredAdvisorRow {
            country: 11,
            ordinal: 0,
            character: 2,
            idea: b"advisor_a".to_vec(),
            ..Default::default()
        }],
    );
    put::<tables::ActiveIdeas>(
        &mut ideas,
        vec![replay::rows::ActiveIdeaRow {
            country: 10,
            ordinal: 0,
            token: b"advisor_a".to_vec(),
        }],
    );
    let eval_ideas = |frame: &DecodedFrame, source: &str, slot: u32| {
        let world = World::new(frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(slot)))
    };
    assert_eq!(eval_ideas(&ideas, "has_idea = advisor_a", 1), True);
    // GER's active idea of that name does not count: it is an advisor token.
    assert_eq!(eval_ideas(&ideas, "has_idea = advisor_a", 0), False);
    // ...but a slot of GER's faction (FactionAdvisors) holding it does.
    let faction = (7_u64 << 32) | 88;
    put::<tables::FactionMembers>(
        &mut ideas,
        vec![replay::rows::FactionMemberRow {
            country: 10,
            faction,
            ordinal: 0,
        }],
    );
    put::<tables::FactionAdvisors>(
        &mut ideas,
        vec![replay::rows::FactionAdvisorRow {
            faction,
            ordinal: 3,
            token: b"advisor_a".to_vec(),
        }],
    );
    assert_eq!(eval_ideas(&ideas, "has_idea = advisor_a", 0), True);
    // num_of_controlled_states reads CountryCounterInputs.controlled_state_count.
    let mut controlled = frame.clone();
    put::<tables::CountryCounterInputs>(
        &mut controlled,
        vec![replay::rows::CountryCounterInputRow {
            country: 10,
            owned_state_count: 1,
            controlled_state_count: 3,
            ..Default::default()
        }],
    );
    assert_eq!(
        eval_ideas(&controlled, "num_of_controlled_states > 2", 0),
        True
    );
    assert_eq!(
        eval_ideas(&controlled, "num_of_controlled_states > 3", 0),
        False
    );
    // A leader of a party the recording does not list is refused by name.
    let mut orphan = frame.clone();
    put::<tables::PartyLeaders>(&mut orphan, vec![leader(9, 0, 1, 11)]);
    let world = World::new(&orphan, assets.facts()).unwrap();
    match world.evaluate(
        &parse("has_country_leader = { id = 11 }"),
        Scopes::of(Scope::Country(0)),
    ) {
        script::Eval::Unknown(reasons) => assert!(
            reasons
                .iter()
                .any(|r| r.contains("no PoliticalParties row")),
            "{reasons:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// `any_country_with_original_tag` and `all_country_with_original_tag`
/// visit the dynamic countries whose original tag (`CountryAppearance.name_tag`)
/// is the checked country's, then that country.
#[test]
fn countries_with_original_tag_visit_dynamic_countries_then_the_original() {
    use replay::rows::CountryAppearanceRow;
    let assets = assets();
    let mut frame = frame();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries.push(CountryRow {
        id: 12,
        canonical_slot: 2,
        tag: b"D01".to_vec(),
        ..Default::default()
    });
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(CountryLinkRow {
        id: 12,
        canonical_slot: 2,
    });
    put::<tables::CountryLinks>(&mut frame, links);
    let appearance = |country: u32, name_tag: &[u8]| CountryAppearanceRow {
        country,
        name_tag: name_tag.to_vec(),
        ..Default::default()
    };
    put::<tables::CountryAppearance>(
        &mut frame,
        vec![
            appearance(10, b""),
            appearance(11, b""),
            appearance(12, b"GER"),
        ],
    );
    let eval = |source: &str| {
        let world = World::new(&frame, assets.facts()).unwrap();
        world.evaluate(&parse(source), Scopes::of(Scope::Country(1)))
    };
    use script::Eval::{False, True};
    let any = |inner: &str| {
        eval(&format!(
            "any_country_with_original_tag = {{ original_tag_to_check = GER {inner} }}"
        ))
    };
    assert_eq!(any("tag = D01"), True);
    assert_eq!(any("tag = GER"), True);
    assert_eq!(any("tag = ENG"), False);
    // ROOT stays the outer scope.
    assert_eq!(any("tag = D01 ROOT = { tag = ENG }"), True);
    assert_eq!(
        eval("all_country_with_original_tag = { original_tag_to_check = GER original_tag = GER }"),
        True
    );
    assert_eq!(
        eval("all_country_with_original_tag = { original_tag_to_check = GER tag = GER }"),
        False
    );
    // `count`: whole units, down by one per passing country, checked after
    // each visited one; `tooltip` is text.
    assert_eq!(any("count = 2 original_tag = GER"), True);
    assert_eq!(any("count = 3 original_tag = GER"), False);
    assert_eq!(any("count = 2.9 tooltip = x original_tag = GER"), True);
    assert_eq!(any("count = 0 tag = ENG"), True);
    assert_eq!(
        eval("any_country_with_original_tag = { original_tag_to_check = FRA count = 0 }"),
        False
    );
    // Checking a dynamic country checks its original tag.
    assert_eq!(
        eval("any_country_with_original_tag = { original_tag_to_check = D01 tag = GER }"),
        True
    );
    assert_eq!(
        eval("any_country_with_original_tag = { original_tag_to_check = ENG tag = D01 }"),
        False
    );
}

/// The preparation form of `has_naval_invasion_against_state` computes the
/// order's preparation from `InvasionPreparations` and the content define
/// (60 days in the test assets): the required hours are
/// `trunc(A * B / 1e10) * 24`, and the leader term counts only when it
/// applies. A withheld table leaves the bound undecided by name, while the
/// scalar form still passes on any touching order.
#[test]
fn naval_invasion_preparation_reads_the_recorded_operands() {
    use replay::rows::InvasionPreparationRow;
    let assets = assets();
    let eval = |frame: &DecodedFrame, source: &str| {
        let world = World::new(frame, assets.facts()).unwrap();
        let mut scopes = Scopes::of(Scope::Country(0));
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
    // 60 days = 1440 hours required.
    assert_eq!(script::required_invasion_hours(&row(0, None, 0), 60), 1440);
    let full = with(row(1440, None, 0));
    assert_eq!(eval(&full, prepared), script::Eval::True);
    assert_eq!(eval(&full, unprepared), script::Eval::False);
    let half = with(row(720, None, 0));
    assert_eq!(eval(&half, prepared), script::Eval::False);
    // 720 hours is exactly 0.5, which `<` does not pass.
    assert_eq!(eval(&half, unprepared), script::Eval::False);
    let early = with(row(100, None, 0));
    assert_eq!(eval(&early, unprepared), script::Eval::True);
    // A leader term of -0.5 halves the required hours only when it applies.
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
    // Absent relation terms add nothing; present ones add to the factor.
    let mut terms = row(0, None, 0);
    terms.country_term = 10_000;
    terms.relation_term = Some(10_000);
    terms.relation_bonus = Some(-40_000);
    terms.days_term = 1_000_000;
    // (1e5 - 20000) * (60e5 + 1e6) / 1e10 = 56, times 24.
    assert_eq!(script::required_invasion_hours(&terms, 60), 56 * 24);
    // A withheld table: the bound is undecided, the scalar form passes.
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
    // An order the table lacks is undecided for a bound.
    let mut missing = invasion_frame(0);
    put::<tables::InvasionPreparations>(&mut missing, Vec::new());
    assert!(matches!(eval(&missing, prepared), script::Eval::Unknown(_)));
}
