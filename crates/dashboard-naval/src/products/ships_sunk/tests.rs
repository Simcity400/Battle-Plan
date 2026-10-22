use super::*;
use dashboard_text::catalogue::tests::{folder_catalogue, layers_catalogue};
use replay_data::rows::SunkShipEntry;

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

/// Class icons a prepared payload stored with the recording's art are drawn at query time as the
/// request shows them. A selected mod's own drawing is shown. A mod that does not declare the
/// class's icon leaves the recording's declaration, drawn normally with nothing named; one that
/// declares it but cannot draw it is drawn as the recording draws it, named on every reply.
#[test]
fn stored_class_icons_follow_the_selection_at_query_time() {
    use dashboard_base::test_fixtures::visual_mods::{
        BLUE, Machine, RED, fallback_line, first_pixel, png, select,
    };
    let machine = Machine::new("stored_classes");
    machine.write("common/units/ships.txt", b"sub_units = { destroyer = { type = naval } }");
    let sprite = |texture: &str| {
        format!(
            "spriteTypes = {{ spriteType = {{ name = GFX_unit_destroyer_icon_medium              textureFile = {texture} }} }}"
        )
    };
    machine.write("interface/units.gfx", sprite("gfx/destroyer.png").as_bytes());
    machine.write("gfx/destroyer.png", &png(RED));
    machine.write_mod("art", "gfx/destroyer.png", &png(BLUE));
    machine.write_mod("other", "gfx/unrelated.png", &png(BLUE));
    machine.write_mod("broken", "interface/units.gfx", sprite("gfx/broken.png").as_bytes());
    machine.write_mod("broken", "gfx/broken.png", b"not a png");
    let (source, mut options) = machine.open();
    let stored = vec![
        ShipClassInfo {
            token: 3,
            key: Some("destroyer".into()),
            name: Some("Destroyer".into()),
            icon: Some("stored".into()),
        },
        ShipClassInfo {
            token: 4,
            key: None,
            name: None,
            icon: None,
        },
    ];
    let shown = |options: &dashboard_core::config::ContentOptions| {
        let mut classes = stored.clone();
        let ((), line) =
            fallback_line(|| present_stored(&source, Some(options), &mut classes).unwrap());
        (classes, line)
    };
    let icons = |classes: &[ShipClassInfo]| -> Vec<Option<String>> {
        classes.iter().map(|class| class.icon.clone()).collect()
    };
    select(&source, &mut options, &[]);
    let (classes, line) = shown(&options);
    assert_eq!((icons(&classes), line), (icons(&stored), None), "as stored");
    select(&source, &mut options, &["art"]);
    let (classes, line) = shown(&options);
    assert_eq!((first_pixel(classes[0].icon.as_deref().unwrap()), line), (BLUE, None));
    assert_eq!(classes[1].icon, None, "a class without a key keeps no icon");
    select(&source, &mut options, &["other"]);
    let (classes, line) = shown(&options);
    assert_eq!((first_pixel(classes[0].icon.as_deref().unwrap()), line), (RED, None));
    select(&source, &mut options, &["broken"]);
    for _ in 0..2 {
        let (classes, line) = shown(&options);
        assert_eq!(first_pixel(classes[0].icon.as_deref().unwrap()), RED);
        assert!(
            line.as_ref()
                .is_some_and(|line| line.contains("GFX_unit_destroyer_icon_medium")),
            "{line:?}"
        );
    }
}

/// A selected mod redeclares the class's icon with a declaration the sprite registry refuses: the
/// lookup itself fails, so the recording's own is drawn and named, on a memoised reply too,
/// never left without an icon.
#[test]
fn a_refused_class_sprite_redeclaration_falls_back_to_the_recording() {
    use dashboard_base::test_fixtures::visual_mods::{
        Machine, RED, fallback_line, first_pixel, png, select,
    };
    let machine = Machine::new("refused_class_sprite");
    machine.write("common/units/ships.txt", b"sub_units = { destroyer = { type = naval } }");
    machine.write(
        "interface/units.gfx",
        b"spriteTypes = { spriteType = { name = GFX_unit_destroyer_icon_medium \
          textureFile = gfx/destroyer.png } }",
    );
    machine.write("gfx/destroyer.png", &png(RED));
    machine.write_mod(
        "refused",
        "interface/units.gfx",
        b"spriteTypes = { spriteType = { name = GFX_unit_destroyer_icon_medium \
          textureFile = gfx/destroyer.png noOfFrames = -1 } }",
    );
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["refused"]);
    for _ in 0..2 {
        let mut classes = vec![ShipClassInfo {
            token: 3,
            key: Some("destroyer".into()),
            name: None,
            icon: None,
        }];
        let ((), line) =
            fallback_line(|| present_stored(&source, Some(&options), &mut classes).unwrap());
        assert_eq!(first_pixel(classes[0].icon.as_deref().unwrap()), RED);
        assert!(
            line.as_ref()
                .is_some_and(|line| line.contains("class destroyer icon")),
            "{line:?}"
        );
    }
}

/// The ordinary ships sunk product draws its class icons from the request's selection, as the
/// prepared query does.
#[test]
fn the_ships_sunk_product_draws_class_icons_from_the_selection() {
    use dashboard_base::test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select};
    use replay_data::rows::{LexicalTokenRow, SunkShipsRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    let machine = Machine::new("ships_sunk_product");
    machine.write("common/units/ships.txt", b"sub_units = { destroyer = { type = naval } }");
    machine.write(
        "interface/units.gfx",
        b"spriteTypes = { spriteType = { name = GFX_unit_destroyer_icon_medium \
          textureFile = gfx/destroyer.png } }",
    );
    machine.write("gfx/destroyer.png", &png(RED));
    machine.write_mod("art", "gfx/destroyer.png", &png(BLUE));
    let mut frame = dashboard_base::test_fixtures::source(1, 100, Some(100_000));
    *frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![SunkShipEntry {
                ship_name: b"Fixture".to_vec(),
                class_token: 77,
                sink_hour: 100,
                ..Default::default()
            }],
        }]));
    frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"destroyer".to_vec(),
        }]));
    let (source, mut options) = machine.open_frames(vec![frame]);
    let icon = |options: &dashboard_core::config::ContentOptions| {
        let reply = ShipsSunk::load(
            &source,
            &BranchParams {
                branch_id: "1".into(),
            },
            &LoadContext {
                content: Some(options.clone()),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let class = reply.classes.iter().find(|class| class.token == 77).unwrap();
        first_pixel(class.icon.as_deref().unwrap())
    };
    select(&source, &mut options, &[]);
    assert_eq!(icon(&options), RED);
    select(&source, &mut options, &["art"]);
    assert_eq!(icon(&options), BLUE);
}

#[test]
fn modded_sub_unit_and_renamed_sea_zone_use_the_effective_content_stack() {
    engine_executable::executable_tables::initialize_for_tests()
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
        installed_content_core::ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&modded.0, &[][..])])
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
    engine_executable::executable_tables::initialize_for_tests()
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
