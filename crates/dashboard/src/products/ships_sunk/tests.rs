use super::*;
use crate::catalogue::tests::{folder_catalogue, layers_catalogue};
use replay::rows::SunkShipEntry;

fn event(hour: i32, read: i32) -> sinkings::ShipEvent {
    sinkings::ShipEvent {
        entry: SunkShipEntry {
            ship_name: b"Recorded ship".to_vec(),
            class_name: b"recorded_cruiser".to_vec(),
            class_token: 77,
            sink_hour: hour,
            province: 300,
            province_present: 1,
            battle: (1_u64 << 63) + 42,
            ..Default::default()
        },
        read_hour: read,
        victim: Some("victim with spaces".into()),
        attacker: Some("opaque sinker".into()),
        victim_as_of: None,
        attacker_as_of: None,
    }
}

fn scanned(ships: Vec<sinkings::ShipEvent>) -> sinkings::Scanned {
    sinkings::Scanned {
        first_hour: 100,
        last_hour: 200,
        first_ship_read: Some(119),
        ships,
        tokens: [(77, "recorded_cruiser".to_owned())].into(),
        ..Default::default()
    }
}

#[test]
fn dates_are_event_hours_duplicates_survive_and_battle_names_are_not_guessed() {
    let mut oldest = event(90, 119);
    oldest.victim_as_of = Some(119);
    oldest.attacker_as_of = Some(119);
    let scan = scanned(vec![event(150, 167), oldest.clone(), oldest]);
    let names = named(&scan, None, &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply
            .rows
            .iter()
            .map(|row| row.sink_hour)
            .collect::<Vec<_>>(),
        [150, 90, 90]
    );
    assert_eq!(reply.rows[0].known_at, 167);
    assert_eq!(reply.rows.len(), 3);
    assert_eq!(reply.rows[1].victim.as_deref(), Some("victim with spaces"));
    assert_eq!(reply.rows[1].victim_as_of, Some(119));
    assert_eq!(reply.rows[1].attacker_as_of, Some(119));
    assert_eq!(reply.rows[0].victim_as_of, None);
    assert_eq!(reply.classes[0].key.as_deref(), Some("recorded_cruiser"));
    assert!(reply.classes[0].name.is_none());
    assert_eq!(
        reply.recorded_names[reply.rows[0].recorded_name_index.unwrap() as usize],
        "recorded_cruiser"
    );
}

#[test]
fn causes_and_assisted_flags_never_become_hostile_credits() {
    let mut cause = event(150, 167);
    cause.entry.cause_record = 1;
    cause.entry.assisted_kill = 1;
    cause.entry.attribution_name = b"Recorded mine description".to_vec();
    let mut assisted = event(151, 167);
    assisted.entry.assisted_kill = 1;
    let mut unknown = event(152, 167);
    unknown.attacker = None;
    let scan = scanned(vec![cause, assisted, unknown, event(153, 167)]);
    let (cat, _folder) = folder_catalogue(&[]);
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply.rows.iter().map(|row| row.cause).collect::<Vec<_>>(),
        [
            ShipSinkingCause::Cause,
            ShipSinkingCause::Assisted,
            ShipSinkingCause::Hostile,
            ShipSinkingCause::Hostile
        ]
    );
    assert_eq!(
        reply.rows[0].attribution_name.as_deref(),
        Some("Recorded mine description")
    );
    assert_eq!(reply.rows[0].name.as_deref(), Some("Recorded ship"));
    assert!(reply.rows[0].assisted_kill);
    // A missing event-hour country identity leaves the hostile attribution
    // unknown, rather than changing the recorded combat-source constructor.
    assert!(reply.rows[2].attacker.is_none());
}

#[test]
fn stored_ship_and_cause_names_are_refused_when_viewer_language_differs() {
    let mut ship = event(150, 167);
    ship.entry.cause_record = 1;
    ship.entry.attribution_name = b"Recorded accident description".to_vec();
    let scan = scanned(vec![ship]);
    let (mut cat, _folder) = folder_catalogue(&[]);
    cat.recorded_language = "other language".to_owned();
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.rows[0].cause, ShipSinkingCause::Cause);
    assert!(reply.rows[0].attribution_name.is_none());
    assert!(reply.rows[0].name.is_none());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("without its key")
                && warning.contains("1 Ships sunk fields"))
    );
    assert!(reply.warnings.iter().any(|warning| {
        warning.contains("ship name is text the game stored") && warning.contains("without its key")
    }));
    let mut hostile = event(151, 167);
    hostile.entry.attribution_name = b"Recorded attacker name".to_vec();
    let scan = scanned(vec![hostile]);
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = super::reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert!(reply.rows[0].attribution_name.is_none());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| { warning.contains("attribution ship name is text the game stored") })
    );
}

#[test]
fn complete_empty_and_absent_history_are_different_and_coverage_is_preserved() {
    let mut scan = scanned(vec![]);
    scan.ship_unread = vec![HourSpan {
        first_hour: 130,
        last_hour: 142,
    }];
    let names = named(&scan, None, &AtomicBool::new(false)).unwrap();
    let present = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert!(present.available);
    assert!(present.rows.is_empty());
    assert_eq!(present.unread, scan.ship_unread);
    scan.first_ship_read = None;
    assert!(
        !reply("1", &scan, &names, &AtomicBool::new(false))
            .unwrap()
            .available
    );
}

#[test]
fn modded_sub_unit_and_renamed_sea_zone_use_the_effective_content_stack() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (_, base) = folder_catalogue(&[
        (
            "common/units/base.txt",
            "sub_units={base_ship={type=naval}}",
        ),
        (
            "map/strategicregions/zone.txt",
            "strategic_region={id=8 name=original_sea provinces={300} naval_terrain=water}",
        ),
        (
            "localisation/base_l_english.yml",
            "\u{feff}l_english:\n original_sea:0 \"Original sea\"\n",
        ),
    ]);
    let (_, modded) = folder_catalogue(&[
        (
            "common/units/mod.txt",
            "sub_units={extra_warship={type=naval}}",
        ),
        (
            "map/strategicregions/zone.txt",
            "strategic_region={id=8 name=renamed_sea provinces={300} naval_terrain=water}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n extra_warship:0 \"Modded warship\"\n renamed_sea:0 \"Renamed mod sea\"\n",
        ),
        (
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_unit_extra_warship_icon_medium texturefile=gfx/mod.tga}}",
        ),
    ]);
    // One synthetic uncompressed true-colour pixel, BGR; no game artwork.
    let mut tga = vec![0_u8; 18];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 1;
    tga[16] = 24;
    tga[17] = 0x20;
    tga.extend([70, 50, 30]);
    std::fs::create_dir_all(modded.0.join("gfx")).unwrap();
    std::fs::write(modded.0.join("gfx/mod.tga"), tga).unwrap();
    let cat = layers_catalogue(
        installed_content::ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&modded.0, &[][..])])
            .unwrap(),
    );
    let mut ship = event(150, 167);
    ship.entry.class_name = b"extra_warship".to_vec();
    let mut differently_named = ship.clone();
    differently_named.entry.class_name = b"second recorded design".to_vec();
    let mut scan = scanned(vec![ship, differently_named]);
    // Dictionary case differs: native token identity folds ASCII.
    scan.tokens.insert(77, "EXTRA_WARSHIP".to_owned());
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.classes[0].key.as_deref(), Some("extra_warship"));
    assert_eq!(
        reply.classes.len(),
        1,
        "one content lookup and artwork payload per token"
    );
    assert_eq!(
        reply.recorded_names,
        ["extra_warship", "second recorded design"]
    );
    assert_eq!(reply.rows[0].class_index, reply.rows[1].class_index);
    assert_ne!(
        reply.rows[0].recorded_name_index,
        reply.rows[1].recorded_name_index
    );
    assert_eq!(reply.classes[0].name.as_deref(), Some("Modded warship"));
    assert!(
        reply.classes[0]
            .icon
            .as_ref()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert_eq!(
        reply.rows[0]
            .place_index
            .map(|index| reply.places[index as usize].as_str()),
        Some("Renamed mod sea")
    );
}

#[test]
fn repeated_provinces_in_one_region_keep_the_sinking_place() {
    let (cat, _folder) = folder_catalogue(&[
        (
            "map/strategicregions/zone.txt",
            "strategic_region={id=8 name=mod_sea provinces={300 300} provinces={300}}",
        ),
        (
            "localisation/sea_l_english.yml",
            "\u{feff}l_english:\n mod_sea:0 \"Mod sea\"\n",
        ),
    ]);
    let scan = scanned(vec![event(150, 167)]);
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply.rows[0]
            .place_index
            .map(|index| reply.places[index as usize].as_str()),
        Some("Mod sea")
    );
    assert!(
        !reply
            .warnings
            .iter()
            .any(|warning| warning.contains("multiple content strategic regions"))
    );
}

#[test]
fn absent_class_art_and_ambiguous_places_are_not_invented() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (cat, _folder) = folder_catalogue(&[
        (
            "common/units/classes.txt",
            "sub_units={recorded_cruiser={type=naval} unnamed_ship={type=naval}}",
        ),
        (
            "map/strategicregions/zones.txt",
            "strategic_region={id=8 name=one provinces={300}} strategic_region={id=9 name=two provinces={300}}",
        ),
        (
            "localisation/classes_l_english.yml",
            "\u{feff}l_english:\n recorded_cruiser:0 \"Named class\"\n one:0 \"One sea\"\n two:0 \"Two sea\"\n",
        ),
    ]);
    let mut unnamed = event(151, 167);
    unnamed.entry.class_token = 78;
    unnamed.entry.class_name = b"Recorded unnamed class".to_vec();
    let mut scan = scanned(vec![event(150, 167), unnamed]);
    scan.tokens.insert(78, "unnamed_ship".to_owned());
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.classes[0].name.as_deref(), Some("Named class"));
    assert!(reply.classes[0].icon.is_none());
    assert!(reply.classes[1].name.is_none());
    assert_eq!(
        reply.recorded_names[reply.rows[1].recorded_name_index.unwrap() as usize],
        "Recorded unnamed class"
    );
    assert!(reply.warnings.iter().any(|warning| {
        warning.contains("class localisation is unavailable") && warning.contains("unnamed_ship")
    }));
    assert!(reply.rows[0].place_index.is_none());
    assert!(reply.warnings.iter().any(|warning| {
        warning.contains("multiple content strategic regions") && warning.contains("province 300")
    }));
}

#[test]
fn content_retention_marks_nonpositive_unavailable_and_long_unread_spans() {
    let mut scan = scanned(vec![event(150, 167)]);
    scan.ship_unread = vec![HourSpan {
        first_hour: 200,
        last_hour: 20_000,
    }];
    scan.ship_breaks = scan.ship_unread.clone();
    let (missing, _missing_folder) = folder_catalogue(&[]);
    let names = named(&scan, Some(&missing), &AtomicBool::new(false)).unwrap();
    assert!(names.warnings.iter().any(|warning| {
        warning.contains("NAVAL_COMBAT_RESULT_TIMEOUT_YEARS is unavailable")
            && warning.contains("completeness is unknown")
    }));
    let (zero, _zero_folder) = folder_catalogue(&[(
        "common/defines/retention.lua",
        "NDefines={NNavy={NAVAL_COMBAT_RESULT_TIMEOUT_YEARS=0}}",
    )]);
    let names = named(&scan, Some(&zero), &AtomicBool::new(false)).unwrap();
    assert!(
        names
            .warnings
            .iter()
            .any(|warning| warning.contains("nonpositive")
                && warning.contains("completeness is unknown"))
    );
    let (positive, _positive_folder) = folder_catalogue(&[(
        "common/defines/retention.lua",
        "NDefines={NNavy={NAVAL_COMBAT_RESULT_TIMEOUT_YEARS=1}}",
    )]);
    let names = named(&scan, Some(&positive), &AtomicBool::new(false)).unwrap();
    assert!(
        names
            .warnings
            .iter()
            .any(|warning| warning.contains("1 unread ship-history spans")
                && warning.contains("cannot be counted"))
    );
}

#[test]
#[ignore = "large reply fixture is run by the lead at the end"]
fn long_war_with_five_thousand_opaque_country_tags_fits_the_reply_budget() {
    let count = 50_000;
    let hours = 12_000;
    let ships = (0..count)
        .map(|index| {
            let mut ship = event(100 + index % hours, 100 + index % hours);
            ship.victim = Some(format!("country slot {}", index % 5_000));
            ship.attacker = Some(format!("country slot {}", (index + 1) % 5_000));
            ship
        })
        .collect();
    let mut scan = scanned(ships);
    scan.last_hour = scan.first_hour + hours - 1;
    scan.first_ship_read = Some(scan.first_hour);
    let (cat, _folder) = folder_catalogue(&[]);
    let names = named(&scan, Some(&cat), &AtomicBool::new(false)).unwrap();
    let reply = reply("1", &scan, &names, &AtomicBool::new(false)).unwrap();
    let bytes = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    assert_eq!(reply.rows.len(), count as usize);
    assert!(bytes < MAX_PRODUCT_REPLY_BYTES);
    println!("ships_sunk: 50,000 sinkings / 5,000 countries; {bytes} JSON bytes");
}

#[test]
fn missing_class_index_is_a_named_refusal() {
    let scan = scanned(vec![event(150, 167)]);
    let mut names = named(&scan, None, &AtomicBool::new(false)).unwrap();
    names.class_indices.clear();
    assert!(
        reply("1", &scan, &names, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("class token 77 has no resolved class index")
    );
}

#[test]
fn oversized_reply_is_refused_and_cancellation_is_observed() {
    let scan = scanned(vec![event(150, 167)]);
    let mut names = named(&scan, None, &AtomicBool::new(false)).unwrap();
    names.classes[0].icon = Some("x".repeat(MAX_PRODUCT_REPLY_BYTES));
    assert!(
        reply("1", &scan, &names, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("reply budget")
    );
    assert_eq!(
        reply("1", &scan, &names, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}
