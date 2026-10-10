use super::*;
fn wing(id: u64, owner: u32, region: u32) -> AirWingRow {
    AirWingRow {
        id,
        owner,
        mission_region: Some(region),
        ..Default::default()
    }
}

fn registration(wing: u64, ordinal: u32) -> replay::rows::AirRegionalMissionRow {
    replay::rows::AirRegionalMissionRow { wing, ordinal }
}

#[test]
fn wings_fly_region_by_region_ours_first_then_both_arrays_in_registration_order() {
    let wings = [
        wing(1, 7, 5),
        wing(2, 7, 5),
        wing(3, 8, 5),
        wing(4, 9, 4),
        wing(5, 10, 5),
    ];
    let registrations = [
        registration(1, 1),
        registration(2, 0),
        registration(3, 0),
        registration(4, 0),
        registration(5, 0),
    ];
    let links = BTreeMap::from([(7, 7), (8, 8), (9, 9), (10, 10), (18, 8)]);
    let participants = AirParticipants {
        a: vec![18, 9],
        b: vec![8],
    };
    let order = visit_order(&wings, &registrations, &links, 7, &participants).unwrap();
    let ids: Vec<_> = order
        .iter()
        .map(|visit| (visit.region, visit.wing.id, visit.side))
        .collect();
    // Country 10 is no participant; country 8 sits in both arrays.
    assert_eq!(ids, [(4, 4, 0), (5, 2, 0), (5, 1, 0), (5, 3, 0), (5, 3, 1)]);
    // Each country entry is its own grouping call.
    let calls: Vec<_> = order.iter().map(|visit| visit.call).collect();
    assert!(calls.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_ne!(order[3].call, order[4].call);
    let gap = [registration(1, 1), registration(2, 2)];
    assert!(visit_order(&wings, &gap, &links, 7, &participants).is_err());
}

#[test]
fn recorded_vectors_and_hosts_build_the_viewpoints_arrays() {
    use replay::{
        frame::{Metrics, TableView},
        rows::{TableId, TableRows, VolunteerHostRow},
    };
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 42,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    };
    let links = BTreeMap::from([(101, 7), (202, 8), (203, 8), (303, 9), (404, 10)]);
    let relation = |country, kind, ordinal, other| AirCountryRelationRow {
        country,
        kind,
        ordinal,
        other,
    };
    *frame.tables[TableId::AirCountryRelations.index()] =
        TableView::Complete(TableRows::AirCountryRelations(vec![
            relation(7, 0, 0, 202),
            relation(7, 0, 1, 203),
            relation(7, 2, 0, 404),
            relation(8, 1, 0, 303),
        ]));
    *frame.tables[TableId::VolunteerHosts.index()] =
        TableView::Complete(TableRows::VolunteerHosts(vec![VolunteerHostRow {
            country: 101,
            position: 0,
            host: 202,
            ..Default::default()
        }]));
    let (arrays, gap) = participant_countries(&frame, 7, &links).unwrap();
    assert!(gap.is_none());
    assert_eq!(arrays.a, [202, 303]);
    assert_eq!(arrays.b, [404]);
    // A gap in a vector's positions is refused.
    *frame.tables[TableId::AirCountryRelations.index()] =
        TableView::Complete(TableRows::AirCountryRelations(vec![relation(7, 2, 1, 404)]));
    assert!(participant_countries(&frame, 7, &links).is_err());
    *frame.tables[TableId::AirCountryRelations.index()] = TableView::Unavailable {
        reason: "test".into(),
    };
    let (arrays, gap) = participant_countries(&frame, 7, &links).unwrap();
    assert!(arrays.a.is_empty() && arrays.b.is_empty() && gap.is_some());
}
/// Countries `1..=n` at war in two blocs: each allied to the rest of its
/// bloc, the enemy of the other and given two donors from its own bloc.
/// The viewing country 1 volunteers to host country `n`.
fn bloc_war(n: u32) -> (DecodedFrame, BTreeMap<u32, u32>) {
    use replay::{
        frame::{Metrics, TableView},
        rows::{TableId, TableRows, VolunteerHostRow},
    };
    let half = n / 2;
    let bloc = |country: u32| {
        if country <= half {
            1..=half
        } else {
            half + 1..=n
        }
    };
    let mut rows = Vec::new();
    for country in 1..=n {
        let own = bloc(country);
        let (first, len) = (*own.start(), own.end() - own.start() + 1);
        let donors = [1, 2].map(|step| first + (country - first + step) % len);
        let others = if country <= half {
            half + 1..=n
        } else {
            1..=half
        };
        let vectors: [Vec<u32>; 3] = [
            donors.into(),
            own.filter(|&other| other != country).collect(),
            others.collect(),
        ];
        for (kind, vector) in [
            AirCountryRelationRow::DONOR,
            AirCountryRelationRow::ALLY,
            AirCountryRelationRow::ENEMY,
        ]
        .into_iter()
        .zip(vectors)
        {
            rows.extend(
                (0..)
                    .zip(vector)
                    .map(|(ordinal, other)| AirCountryRelationRow {
                        country,
                        kind,
                        ordinal,
                        other,
                    }),
            );
        }
    }
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 42,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::AirCountryRelations.index()] =
        TableView::Complete(TableRows::AirCountryRelations(rows));
    *frame.tables[TableId::VolunteerHosts.index()] =
        TableView::Complete(TableRows::VolunteerHosts(vec![VolunteerHostRow {
            country: 1,
            position: 0,
            host: n,
            ..Default::default()
        }]));
    (frame, (1..=n).map(|country| (country, country)).collect())
}

#[test]
fn a_bloc_war_reads_only_the_vectors_the_arrays_use() {
    let (frame, links) = bloc_war(40);
    let (arrays, gap) = participant_countries(&frame, 1, &links).unwrap();
    assert!(gap.is_none());
    // The same arrays as from every country's complete vectors.
    let Available::Complete(rows) = rows::<tables::AirCountryRelations>(&frame).unwrap() else {
        unreachable!("complete fixture")
    };
    let mut complete = BTreeMap::<u32, CountryRelations>::new();
    for row in rows {
        let relations = complete.entry(row.country).or_default();
        match row.kind {
            AirCountryRelationRow::DONOR => &mut relations.reciprocal_donors,
            AirCountryRelationRow::ALLY => &mut relations.wartime_allies,
            _ => &mut relations.enemies,
        }
        .push(row.other);
    }
    struct Complete(BTreeMap<u32, CountryRelations>);
    impl CountryVectors for Complete {
        fn vectors(&self, slot: u32) -> io::Result<CountryRelations> {
            Ok(self.0.get(&slot).cloned().unwrap_or_default())
        }
        fn donors(&self, slot: u32, donors: &mut Vec<u32>) -> io::Result<()> {
            *donors = self.vectors(slot)?.reciprocal_donors;
            Ok(())
        }
    }
    let expected = ordered_participants(1, &Complete(complete), &[40], &links).unwrap();
    assert_eq!(arrays, expected);
    // Own donors, then each ally and its donors; the host, then its allies.
    assert_eq!(arrays.a[..4], [2, 3, 4, 5]);
    assert_eq!((arrays.a.len(), arrays.b.len()), (40, 40));
    assert_eq!(arrays.b[..3], [21, 22, 23]);
}

/// The scale matrix, in release:
/// `cargo test -p viewer --release --bin viewer -- --ignored bloc_war_participants`.
#[test]
#[ignore]
fn bloc_war_participants_at_1500_and_5000_countries() {
    for n in [1_500, 5_000] {
        let (frame, links) = bloc_war(n);
        let mut best = std::time::Duration::MAX;
        let mut arrays = AirParticipants::default();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            arrays = participant_countries(&frame, 1, &links).unwrap().0;
            best = best.min(started.elapsed());
        }
        eprintln!(
            "participants, {n} countries at war in two blocs: {best:?} (best of 5), arrays of {} and {}",
            arrays.a.len(),
            arrays.b.len()
        );
    }
}

#[test]
fn country_keys_precede_bare_equipment_and_do_not_add_a_culture() {
    assert_eq!(
        candidates("GER", "small_airframe_2", "small_airframe", "light_plane"),
        [
            "GER_small_airframe_2_entity",
            "GER_small_airframe_entity",
            "GER_light_plane_entity",
            "small_airframe_2_entity",
            "small_airframe_entity",
            "light_plane_entity"
        ]
    );
}
#[test]
fn callback_arrays_numbers_and_native_hashes() {
    let hash = callback("supply", 0, 0, 3).unwrap();
    assert_eq!(callback("a_supply", 0, 2, 3), Some(hash));
    assert_eq!(callback("a_supply", 1, 2, 3), None);
    assert_eq!(callback("d_supply", 0, 2, 3), None);
    assert_eq!(callback("d_supply", 1, 2, 3), Some(hash));
    assert_eq!(callback("a_2_supply", 0, 0, 3), None);
    assert_eq!(callback("a_2_supply", 0, 1, 3), Some(hash));
    assert_eq!(callback("a_0_supply", 0, 0, 3), Some(hash));
    assert_eq!(callback("a_4_supply", 0, 0, 3), Some(hash));
    assert_eq!(callback("2_supply", 1, 1, 3), Some(hash));
    assert_eq!(
        callback("a_10_supply", 0, 0, 3),
        callback("_supply", 0, 0, 3)
    );
    assert_ne!(callback("a_SUPPLY", 0, 0, 3), Some(hash));
}

#[test]
fn meshless_aircraft_use_the_shared_actor_player() {
    let descriptors = mesh::parse_descriptors(br#"
            entity = { name = plane
                state = { name = supply event = { particle = drop light = lamp sound = { soundeffect = hum } } }
            }
        "#, "fixture.asset").unwrap();
    let roster = mesh::Roster {
        descriptors,
        ..Default::default()
    };
    assert_eq!(plane_refusal(&roster, "plane", &BTreeSet::new()), None);
}

#[test]
fn lazy_explicit_aircraft_keep_named_counted_attachment_and_audio_diagnostics() {
    let layers = ResolvedLayers::default();
    let map = MapData {
        width: 1,
        height: 1,
        province_ids: vec![0],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let defines = Defines::new().unwrap();
    let mut assets = AircraftAssets::load(
        &layers,
        Decode::default(),
        &map,
        &defines,
        &BTreeMap::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assets.roster = Arc::new(roster(
        r#"
        entity = { name = OddAircraft
            attach = { left = AbsentFlap }
            attach = { right = AbsentFlap }
            state = { name = idle
                event = { sound = { soundeffect = hum } }
                event = { sound = { soundeffect = buzz } }
            }
        }
    "#,
    ));
    let selected = super::super::aircraft_model::resolve_lazy(
        &replay::rows::VariantDefinitionRow {
            explicit_entity: b"OddAircraft".to_vec(),
            ..Default::default()
        },
        0,
        &|| panic!("unreached designer"),
        &|| panic!("unreached graphical override"),
        &|| panic!("unreached equipment ladder"),
        &|name| assets.roster.descriptors.entities.contains_key(name),
    )
    .unwrap()
    .unwrap();
    let slot = assets.resolve(&selected).unwrap();
    assert_eq!(assets.library().meshes[slot].name, "OddAircraft");
    let diagnostics = assets.refusals();
    assert!(
        diagnostics
            .iter()
            .any(|line| { line.contains("OddAircraft -> AbsentFlap") && line.contains("(2)") }),
        "{diagnostics:?}"
    );
    let typed = assets.lazy_diagnostics();
    assert!(typed.iter().any(|(kind, line)| {
        *kind == installed_content::DiagnosticKind::Refused
            && line.contains("AbsentFlap")
            && line.contains("(2)")
    }));
    assert!(typed.iter().any(|(kind, line)| {
        *kind == installed_content::DiagnosticKind::Warning
            && line.contains("OddAircraft (2 sound events refused)")
    }));
    let report = AircraftPlan::default().report(&assets);
    assert!(
        report
            .iter()
            .any(|line| line.contains("AbsentFlap") && line.contains("(2)"))
    );
    assert!(
        report
            .iter()
            .any(|line| line.contains("OddAircraft (2 sound events refused)"))
    );
    assert_eq!(assets.resolve(&selected), Ok(slot));
    assert_eq!(assets.refusals(), diagnostics);
    assert_eq!(assets.lazy_diagnostics(), typed);
}

#[test]
fn a_plane_attachment_cycle_is_refused_by_name() {
    let roster = roster(
        "entity = { name = plane attach = { root = child } }
         entity = { name = child attach = { root = plane } }",
    );
    assert_eq!(
        plane_refusal(&roster, "plane", &BTreeSet::new()),
        Some("its attached entity child is refused: its attached entity plane is refused: attachment cycle through plane".into())
    );
}

#[test]
fn a_refused_entity_stops_the_ladder_instead_of_falling_through() {
    let ladder = candidates("GER", "small_airframe_2", "small_airframe", "light_plane");
    let generic = |entity: &str| (entity == "small_airframe_entity").then_some(4);
    // A country entity the engine would find, refused here, is named
    // rather than replaced by the generic plane below it.
    let refused = format!("it {}", why());
    let unusable = BTreeMap::from([("GER_small_airframe_entity".to_owned(), refused.clone())]);
    assert_eq!(
        resolve_entity(&ladder, generic, &unusable),
        Err(Unresolved::Refused {
            entity: "GER_small_airframe_entity".into(),
            reason: refused.clone(),
        })
    );
    // Entities the game lacks are still skipped.
    assert_eq!(
        resolve_entity(&ladder, generic, &BTreeMap::new()),
        Ok(Some(4))
    );
    assert_eq!(
        resolve_entity(&ladder, |_| None, &BTreeMap::new()),
        Ok(None)
    );
    // Each affected wing is counted under its entity and reason.
    let mut refusals = Refusals::default();
    let mut diagnostics = Vec::new();
    for wing in [1, 2] {
        let error = resolve_entity(&ladder, generic, &unusable).unwrap_err();
        refusals.omit(wing, error, &mut diagnostics);
    }
    refusals.omit(3, Unresolved::Identity, &mut diagnostics);
    assert_eq!(diagnostics, ["wing 3 missing equipment identity"]);
    assert_eq!(
        refusals.diagnostics().collect::<Vec<_>>(),
        [format!(
            "2 wings omitted: their aircraft entity GER_small_airframe_entity is refused: {refused}"
        )]
    );
}

#[test]
fn a_roster_whose_every_plane_is_refused_keeps_each_reason() {
    let mut roster = roster(
        r#"entity = { name = "one_entity" pdxmesh = "m" }
            entity = { name = "two_entity" pdxmesh = "m" }"#,
    );
    for name in ["one_entity", "two_entity"] {
        roster.unreadable_names.insert(name.into(), BROKEN.into());
    }
    let declared = ["one_entity".to_owned(), "two_entity".to_owned()];
    let mut warnings = Vec::new();
    let (library, unusable) = plane_library(
        &ResolvedLayers::default(),
        Decode::default(),
        &roster,
        &declared,
        &mut warnings,
    )
    .unwrap();
    assert!(library.meshes.is_empty());
    let refused = format!("it {}", why());
    assert_eq!(
        unusable,
        BTreeMap::from([
            ("one_entity".to_owned(), refused.clone()),
            ("two_entity".to_owned(), refused.clone()),
        ])
    );
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].0, installed_content::DiagnosticKind::Refused);
    assert!(
        warnings[0]
            .1
            .contains(&format!("one_entity ({refused}); two_entity"))
    );
    let ladder = candidates("", "one", "", "");
    assert_eq!(
        resolve_entity(&ladder, |entity| library.slot(entity), &unusable),
        Err(Unresolved::Refused {
            entity: "one_entity".into(),
            reason: refused,
        })
    );
}

/// A descriptor file that does not parse, read before the others.
const BROKEN: &str = "gfx/entities/a_broken.asset";

fn why() -> String {
    format!("may be declared in {BROKEN}, which does not parse")
}

/// One planes descriptor's text as a parsed roster holds it.
fn roster(text: &str) -> mesh::Roster {
    mesh::Roster {
        descriptors: mesh::parse_descriptors(text.as_bytes(), "gfx/entities/planes.asset").unwrap(),
        ..Default::default()
    }
}

/// `Roster::read` over [`DEPENDENCIES_GFX`], [`DEPENDENCIES`] and a
/// [`BROKEN`] file holding `hidden`.
fn read_roster(name: &str, hidden: &[&str]) -> mesh::Roster {
    let root = std::env::temp_dir().join(format!(
        "viewer-aircraft-roster-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let broken = format!(
        "entity = {{ hidden = {{ {} }} }} }}{{ = =",
        hidden.join(" ")
    );
    for (path, text) in [
        ("gfx/models/planes.gfx", DEPENDENCIES_GFX),
        ("gfx/entities/planes.asset", DEPENDENCIES),
        (BROKEN, broken.as_str()),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let roster = mesh::Roster::read(&layers).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(roster.unreadable.len(), 1, "{:?}", roster.unreadable);
    roster
}

const DEPENDENCIES_GFX: &str = r#"
        objectTypes = {
            pdxmesh = { name = "clean_mesh" file = "clean.mesh" }
            pdxmesh = { name = "hidden_mesh" file = "hidden.mesh" }
            pdxmesh = {
                name = "animated_mesh"
                file = "animated.mesh"
                animation = { id = "idle" type = "clean_animation" }
                animation = { id = "fly" type = "hidden_animation" }
            }
        }
    "#;

const DEPENDENCIES: &str = r#"
        entity = { name = "hidden_parent_entity" pdxmesh = "clean_mesh" }
        entity = { name = "middle_entity" clone = "hidden_parent_entity" }
        entity = { name = "GER_plane_entity" clone = "middle_entity" }
        entity = { name = "hidden_mesh_plane_entity" pdxmesh = "hidden_mesh" }
        entity = { name = "inherits_hidden_mesh_entity" clone = "hidden_mesh_plane_entity" }
        entity = {
            name = "animated_plane_entity"
            pdxmesh = "animated_mesh"
            state = { name = "idle" animation = "idle" }
            state = { name = "fly" animation = "fly" }
        }
        entity = {
            name = "carrier_plane_entity"
            pdxmesh = "clean_mesh"
            attach = { name = "gun" gun_joint = "hidden_mesh_plane_entity" }
        }
        entity = { name = "clean_plane_entity" pdxmesh = "clean_mesh" }
        entity = { name = "fighter_1_patrol_entity" clone = "hidden_parent_entity" }
        entity = {
            name = "fighter_3_patrol_entity"
            pdxmesh = "animated_mesh"
            default_state = "idle"
            state = { name = "default" animation = "fly" }
        }
        animation = { name = "clean_animation" file = "clean.anim" }
        animation = { name = "hidden_animation" file = "hidden.anim" }
    "#;

/// A plane is refused, with the declaration named, when it, a clone
/// ancestor it copied, the mesh it names, an animation its states play or
/// an attached entity may be declared in a file that does not parse; the
/// wings it would draw are counted under that reason.
#[test]
fn a_plane_using_a_possibly_hidden_declaration_is_refused_with_that_declaration() {
    let roster = read_roster(
        "planes",
        &["hidden_parent_entity", "hidden_mesh", "hidden_animation"],
    );
    let reason = |entity: &str| plane_refusal(&roster, entity, &BTreeSet::new());
    let copied = format!("it {}", why());
    assert_eq!(reason("GER_plane_entity").as_deref(), Some(copied.as_str()));
    let mesh = format!("its mesh hidden_mesh {}", why());
    assert_eq!(
        reason("hidden_mesh_plane_entity").as_deref(),
        Some(mesh.as_str())
    );
    assert_eq!(
        reason("inherits_hidden_mesh_entity").as_deref(),
        Some(mesh.as_str())
    );
    assert_eq!(
        reason("animated_plane_entity"),
        Some(format!("its animation hidden_animation {}", why()))
    );
    assert_eq!(
        reason("carrier_plane_entity"),
        Some(format!(
            "its attached entity hidden_mesh_plane_entity is refused: {mesh}"
        ))
    );
    assert_eq!(reason("clean_plane_entity"), None);

    let declared = [
        "GER_plane_entity",
        "hidden_mesh_plane_entity",
        "clean_plane_entity",
    ]
    .map(str::to_owned);
    let mut warnings = Vec::new();
    let (library, unusable) = plane_library(
        &ResolvedLayers::default(),
        Decode::default(),
        &roster,
        &declared,
        &mut warnings,
    )
    .unwrap();
    assert_eq!(unusable["GER_plane_entity"], copied);
    assert_eq!(unusable["hidden_mesh_plane_entity"], mesh);
    // No layer holds the clean plane's mesh file, so it is refused as
    // not loaded rather than drawn.
    assert_eq!(unusable["clean_plane_entity"], NOT_LOADED);
    assert_eq!(warnings[0].0, installed_content::DiagnosticKind::Refused);
    assert!(
        warnings[0]
            .1
            .contains(&format!("GER_plane_entity ({copied})"))
    );
    assert!(
        warnings[0]
            .1
            .contains(&format!("hidden_mesh_plane_entity ({mesh})"))
    );

    // A country plane whose parent is refused stops the ladder, and each
    // wing it would draw is counted under that reason.
    let ladder = candidates("GER", "plane", "", "");
    let mut refusals = Refusals::default();
    let mut diagnostics = Vec::new();
    for wing in [1, 2, 3] {
        let error = resolve_entity(&ladder, |entity| library.slot(entity), &unusable).unwrap_err();
        refusals.omit(wing, error, &mut diagnostics);
    }
    assert!(diagnostics.is_empty());
    assert_eq!(
        refusals.diagnostics().collect::<Vec<_>>(),
        [format!(
            "3 wings omitted: their aircraft entity GER_plane_entity is refused: {copied}"
        )]
    );
}

/// A country plane declared only in a descriptor file that does not
/// parse is refused with that file named; the wing does not fall
/// through to the valid generic plane. A readable plane whose mesh that
/// file may declare is refused too.
#[test]
fn a_name_an_unreadable_descriptor_holds_is_refused_not_skipped() {
    let broken = br#"entity = { name = "GER_plane_entity" pdxmesh = "ger_mesh" } }{ = ="#;
    let mut roster = roster(
        r#"
            objectTypes = { pdxmesh = { name = "clean_mesh" file = "clean.mesh" } }
            entity = { name = "plane_entity" pdxmesh = "clean_mesh" }
            entity = { name = "JAP_plane_entity" pdxmesh = "ger_mesh" }
            "#,
    );
    let path = "gfx/entities/broken.asset";
    for name in mesh::lexical_names(broken) {
        roster.unreadable_names.insert(name, path.to_owned());
    }
    let why = format!("may be declared in {path}, which does not parse");
    assert_eq!(
        plane_refusal(&roster, "GER_plane_entity", &BTreeSet::new()),
        Some(format!("it {why}"))
    );
    assert_eq!(
        plane_refusal(&roster, "JAP_plane_entity", &BTreeSet::new()),
        Some(format!("its mesh ger_mesh {why}"))
    );
    assert_eq!(
        plane_refusal(&roster, "plane_entity", &BTreeSet::new()),
        None
    );

    // The loader's candidate filter admits the unreadable name, so the
    // ladder stops there instead of drawing the generic plane.
    let declared = declared_planes(&roster, &BTreeSet::from(["plane"]));
    assert!(declared.contains(&"GER_plane_entity".to_owned()));
    assert!(declared.contains(&"plane_entity".to_owned()));
    let mut warnings = Vec::new();
    let (_, unusable) = plane_library(
        &ResolvedLayers::default(),
        Decode::default(),
        &roster,
        declared,
        &mut warnings,
    )
    .unwrap();
    assert_eq!(warnings[0].0, installed_content::DiagnosticKind::Refused);
    assert!(
        warnings[0]
            .1
            .contains(&format!("GER_plane_entity (it {why})"))
    );
    let generic = |entity: &str| (entity == "plane_entity").then_some(4);
    let ladder = candidates("GER", "plane", "", "");
    let mut refusals = Refusals::default();
    let mut diagnostics = Vec::new();
    for wing in [1, 2] {
        let error = resolve_entity(&ladder, generic, &unusable).unwrap_err();
        refusals.omit(wing, error, &mut diagnostics);
    }
    assert_eq!(
        refusals.diagnostics().collect::<Vec<_>>(),
        [format!(
            "2 wings omitted: their aircraft entity GER_plane_entity is refused: it {why}"
        )]
    );
    // Without the unreadable file the generic plane is drawn.
    assert_eq!(
        resolve_entity(&ladder, generic, &BTreeMap::new()),
        Ok(Some(4))
    );
}

/// The unreadable file's names are read as the parser's lexer reads
/// them: a bare CR ends a comment, and a quoted name's escapes are
/// decoded. Either way the country plane is refused, its wings are
/// counted and none falls through to the valid generic plane.
#[test]
fn an_unreadable_file_s_cr_comments_and_escaped_names_still_refuse() {
    let path = "gfx/entities/broken.asset";
    let why = format!("may be declared in {path}, which does not parse");
    for (broken, tag, key) in [
        (
            &b"# header\rentity = { name = \"GER_plane_entity\" } = }"[..],
            "GER",
            "plane",
        ),
        (
            &b"entity = { name = \"ITA_pl\\\\ane_entity\" } = }"[..],
            "ITA",
            "pl\\ane",
        ),
    ] {
        let country = format!("{tag}_{key}_entity");
        let generic = format!("{key}_entity");
        let mut roster = roster(&format!(
            r#"objectTypes = {{ pdxmesh = {{ name = "clean_mesh" file = "clean.mesh" }} }}
                    entity = {{ name = "{}" pdxmesh = "clean_mesh" }}"#,
            generic.replace('\\', "\\\\")
        ));
        assert!(roster.descriptors.entities.contains_key(&generic));
        for name in mesh::lexical_names(broken) {
            roster.unreadable_names.insert(name, path.to_owned());
        }
        let declared = declared_planes(&roster, &BTreeSet::from([key]));
        assert!(declared.contains(&country), "{country}");
        let mut warnings = Vec::new();
        let (_, unusable) = plane_library(
            &ResolvedLayers::default(),
            Decode::default(),
            &roster,
            declared,
            &mut warnings,
        )
        .unwrap();
        let drawn = |entity: &str| (entity == generic).then_some(4);
        let ladder = candidates(tag, key, "", "");
        let mut refusals = Refusals::default();
        let mut diagnostics = Vec::new();
        for wing in [1, 2, 3] {
            let error = resolve_entity(&ladder, drawn, &unusable).unwrap_err();
            refusals.omit(wing, error, &mut diagnostics);
        }
        assert!(diagnostics.is_empty());
        assert_eq!(
            refusals.diagnostics().collect::<Vec<_>>(),
            [format!(
                "3 wings omitted: their aircraft entity {country} is refused: it {why}"
            )]
        );
    }
}

/// A scenario is refused by a clone ancestor it copied, its mesh and the
/// animation of the state it plays, not by an animation of another state.
#[test]
fn a_scenario_using_a_possibly_hidden_parent_mesh_or_animation_is_refused() {
    let roster = read_roster("scenario", &["hidden_parent_entity", "hidden_animation"]);
    assert_eq!(
        scenario_refusal(&roster, "fighter_1_patrol_entity", "default"),
        Some(format!("it {}", why()))
    );
    assert_eq!(
        scenario_refusal(&roster, "fighter_3_patrol_entity", "default"),
        Some(format!("its animation hidden_animation {}", why()))
    );
    let roster = read_roster("scenario_mesh", &["animated_mesh"]);
    assert_eq!(
        scenario_refusal(&roster, "fighter_3_patrol_entity", "default"),
        Some(format!("its mesh animated_mesh {}", why()))
    );
    let roster = read_roster("scenario_other", &["clean_animation"]);
    assert_eq!(
        scenario_refusal(&roster, "fighter_3_patrol_entity", "default"),
        None
    );
}

#[test]
fn wing_batch_discards_remainder_after_choosing_triples() {
    let fighters = SteppedFamily {
        sizing: FamilyDefines {
            factor: 30,
            triple: 3,
            limit: 2,
        },
        single: 1,
    };
    let batches = |planes| fighters.batches(fighters.sizing.units(planes));
    assert_eq!(batches(0), None);
    assert_eq!(batches(29), Some((1, 1)));
    assert_eq!(batches(60), Some((1, 2)));
    assert_eq!(batches(100), Some((3, 1)));
    assert_eq!(batches(400), Some((3, 4)));
    // A modded transport step of two singles drops the odd unit; the
    // loops that compare it with their wings use the grouped array count.
    let paired = SteppedFamily {
        single: 2,
        ..fighters
    };
    assert_eq!(paired.batches(1), None);
    assert_eq!(paired.batches(2), Some((1, 1)));
    // A negative division factor floors every positive count to one unit.
    let negative = FamilyDefines {
        factor: -30,
        ..fighters.sizing
    };
    assert_eq!((negative.units(400), negative.units(0)), (1, 0));
}

#[test]
fn naval_patrol_follows_141237d20() {
    let kinds = BTreeMap::from([
        (10, ProvinceKind::Sea),
        (11, ProvinceKind::Land),
        (12, ProvinceKind::Sea),
        (20, ProvinceKind::Sea),
    ]);
    let sources = |sunk: i64, lost: i64| NavalPatrolSources {
        // Country 7's first task force in combat is in region 2, its
        // second in region 1 at province 12.
        combats: BTreeMap::from([(7, vec![(20, 2), (12, 1)])]),
        sums: BTreeMap::from([((3, 1), (sunk, lost))]),
    };
    let reached = [11, 12, 10];
    let choose = |sources: &NavalPatrolSources, viewer| {
        let mut rng = Random::new(SEED);
        let start = reached[Random::new(SEED).next() as usize % reached.len()];
        (
            sources.choose(7, 1, viewer, &reached, &mut rng, &kinds),
            start,
        )
    };
    // Ships sunk and fighters lost: the combat's province, success.
    let ((outcome, province), _) = choose(&sources(100_000, 100_000), 3);
    assert_eq!((outcome, province), (SUCCESS, 12));
    // Sunk but none lost, or a viewer without history: the draw, default.
    let ((outcome, province), start) = choose(&sources(100_000, 99_999), 3);
    assert_eq!((outcome, province), (0, start));
    let ((outcome, province), start) = choose(&sources(100_000, 100_000), 4);
    assert_eq!((outcome, province), (0, start));
    // None sunk, a fighter lost: another reached sea province, success.
    let ((outcome, province), _) = choose(&sources(99_999, 100_000), 3);
    assert_eq!((outcome, province), (SUCCESS, 10));
    // A negative sunk count falls back.
    let ((outcome, _), _) = choose(&sources(-100_000, 100_000), 3);
    assert_eq!(outcome, 0);
}

#[test]
fn region_draws_keep_first_listings_like_add_province() {
    assert_eq!(first_listings(&[5, 3, 5, 9, 3]), [5, 3, 9]);
}

#[test]
fn a_full_side_skips_the_next_grouping_call() {
    let wing = AirWingRow::default();
    let visit = |call, side| Visit {
        call,
        side,
        own: false,
        region: 5,
        wing: &wing,
    };
    let mut groups = Groups::default();
    for _ in 0..GROUP_CAPACITY {
        assert!(groups.admit(&visit(0, 0), 1, 0));
    }
    assert!(!groups.admit(&visit(0, 0), 1, 0));
    // Other groups of the same call still fill.
    assert!(groups.admit(&visit(0, 0), 4, 0));
    groups.counts[0] = [GROUP_CAPACITY; 5];
    // A new call on a full side is skipped whole; side B is separate.
    assert!(!groups.admit(&visit(1, 0), 4, 0));
    assert!(groups.admit(&visit(2, 1), 1, 0));
    // A new region starts empty.
    let other = Visit {
        region: 6,
        ..visit(3, 0)
    };
    assert!(groups.admit(&other, 1, 0));
}

#[test]
fn take_off_and_landing_scale_with_the_content_distances() {
    let sizing = FamilyDefines {
        factor: 1,
        triple: 3,
        limit: 2,
    };
    let stepped = SteppedFamily { sizing, single: 1 };
    let mut defines = FlightDefines {
        fighter: stepped,
        scout: stepped,
        bomber: sizing,
        bomber_single: 1,
        dogfight: stepped,
        fire_threshold: 300_000,
        carpet_threshold: 200_000,
        transport: stepped,
        training: stepped,
        clock_speed: 0.22,
        curve_density: 2.0,
        curve_extrapolation: 20.0,
        curve_elevation: 4.0,
        scale_takeoff: 0.1,
        scale_landing: 0.9,
        scale_min: 0.1,
    };
    assert_eq!(defines.takeoff_scale(0.0), 0.1);
    assert!((defines.takeoff_scale(0.05) - 0.5).abs() < 1e-6);
    assert_eq!(defines.takeoff_scale(0.5), 1.0);
    assert_eq!(defines.landing_scale(0.5), 1.0);
    assert!((defines.landing_scale(0.95) - 0.5).abs() < 1e-5);
    assert_eq!(defines.landing_scale(1.0), 0.1);
    // Degenerate distances follow the native IEEE arithmetic.
    defines.scale_takeoff = 0.0;
    defines.scale_landing = 1.0;
    assert!(defines.takeoff_scale(0.0).is_nan());
    assert_eq!(defines.takeoff_scale(0.5), 1.0);
    assert!(defines.landing_scale(1.0).is_nan());
}

#[test]
fn regions_skip_undefined_provinces_and_count_duplicates() {
    let kinds = BTreeMap::from([
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Sea),
        (3, ProvinceKind::Lake),
    ]);
    let mut warnings = Vec::new();
    let (members, fractions) = region_membership(
        [(7, vec![1, 2, 2, 3, 99]), (8, vec![98])].into_iter(),
        &kinds,
        &mut warnings,
    );
    assert_eq!(members[&7], [1, 2, 2, 3]);
    assert!(members[&8].is_empty());
    assert_eq!(
        fractions[&7],
        RegionFractions {
            land_raw: 25_000,
            sea_raw: 50_000
        }
    );
    assert_eq!(fractions[&8], RegionFractions::default());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].0, installed_content::DiagnosticKind::Warning);
    assert!(warnings[0].1.contains("list 2 provinces"));
}

mod scans;

#[test]
fn the_initial_gate_uses_the_wings_own_operands() {
    let mut wing = AirWingRow {
        mission_mask: 4,
        mission_region: Some(90),
        mission_active: 1,
        eligibility_counter: 3,
        eligibility_limit: 3,
        ..AirWingRow::default()
    };
    assert!(passes_initial_gate(&wing));
    wing.mission_active = 0;
    assert!(!passes_initial_gate(&wing));
    wing.mission_active = 1;
    wing.eligibility_veto = 1;
    assert!(!passes_initial_gate(&wing));
    wing.eligibility_veto = 0;
    wing.eligibility_counter = 4;
    assert!(!passes_initial_gate(&wing));
    wing.eligibility_counter = 3;
    wing.mission_region = None;
    assert!(!passes_initial_gate(&wing));
    wing.mission_mask = 0x800;
    assert!(passes_initial_gate(&wing));
}

mod paired;
mod training;

#[test]
fn bit17_base_equipment_has_only_the_coarse_group_call_gate() {
    let wing = AirWingRow {
        mission_mask: 0x20000,
        ..Default::default()
    };
    let visit = Visit {
        call: 0,
        side: 0,
        own: true,
        region: 7,
        wing: &wing,
    };
    let mut groups = Groups::default();
    for _ in 0..GROUP_CAPACITY + 1 {
        assert!(groups.admit(&visit, 2, 0));
    }
    assert_eq!(groups.counts[0][2], GROUP_CAPACITY + 1);
    // Its earlier bomber-definition arm still applies the local cap.
    let overlapping = AirWingRow {
        mission_mask: 0x20002,
        ..wing.clone()
    };
    assert!(!groups.admit(
        &Visit {
            wing: &overlapping,
            ..visit
        },
        2,
        9
    ));
    assert!(groups.admit(
        &Visit {
            wing: &overlapping,
            ..visit
        },
        2,
        0
    ));
    groups.counts[0][..4].fill(GROUP_CAPACITY);
    assert!(!groups.admit(&Visit { call: 1, ..visit }, 2, 0));
}

#[test]
fn first_window_history_distinguishes_older_absence_from_captured_zero() {
    use replay::{
        frame::TableView,
        rows::{AirNavalPatrolHistoryRow, Recorded, TableId, TableRows},
    };
    let (mut frame, _) = bloc_war(2);
    assert_eq!(
        ScenarioHistory::read(&frame, 1).unwrap().err(),
        Some("first-window aircraft scenario channels were not recorded")
    );
    let witness = AirNavalPatrolHistoryRow::availability_witness(1);
    *frame.tables[TableId::AirNavalPatrolHistories.index()] =
        TableView::Complete(TableRows::AirNavalPatrolHistories(vec![witness.clone()]));
    assert_eq!(
        ScenarioHistory::read(&frame, 1)
            .unwrap()
            .unwrap()
            .region(7)
            .unwrap(),
        [0; 8]
    );
    let older = AirNavalPatrolHistoryRow {
        country: 1,
        region: 7,
        scenario_channels: Recorded::NotRecorded,
        ..Default::default()
    };
    *frame.tables[TableId::AirNavalPatrolHistories.index()] =
        TableView::Complete(TableRows::AirNavalPatrolHistories(vec![older, witness]));
    let history = ScenarioHistory::read(&frame, 1).unwrap().unwrap();
    assert_eq!(
        history.region(7),
        Err("first-window aircraft scenario channels were not recorded")
    );
    assert_eq!(history.region(8), Ok([0; 8]));
}

#[test]
fn refused_scenario_history_keeps_other_regions_and_original_naval_channels() {
    use replay::{
        frame::TableView,
        rows::{AirNavalPatrolHistoryRow, Recorded, TableId, TableRows},
    };
    let (mut frame, _) = bloc_war(2);
    let refused = AirNavalPatrolHistoryRow {
        country: 1,
        region: 7,
        index: 0,
        enemy_ships_sunk_raw: 300_000,
        our_fighters_lost_raw: 200_000,
        scenario_channels: Recorded::Value(None),
    };
    let healthy = AirNavalPatrolHistoryRow {
        country: 1,
        region: 8,
        index: 0,
        our_fighters_lost_raw: 22,
        scenario_channels: Recorded::Value(Some([1, 2, 3, 4, 5, 6, 7])),
        ..Default::default()
    };
    *frame.tables[TableId::AirNavalPatrolHistories.index()] =
        TableView::Complete(TableRows::AirNavalPatrolHistories(vec![
            refused,
            healthy,
            AirNavalPatrolHistoryRow::availability_witness(1),
        ]));
    let history = ScenarioHistory::read(&frame, 1).unwrap().unwrap();
    assert_eq!(
        history.region(7),
        Err("first-window aircraft scenario channels unavailable")
    );
    assert_eq!(history.region(8), Ok([1, 2, 22, 3, 4, 5, 6, 7]));
    let naval = NavalPatrolSources::read(&frame).unwrap().unwrap();
    assert_eq!(naval.sums[&(1, 7)], (300_000, 200_000));
    assert_eq!(naval.sums[&(1, 8)], (0, 22));
}

#[test]
fn first_window_history_saturates_each_channel_after_each_ordered_record() {
    use replay::{
        frame::TableView,
        rows::{AirNavalPatrolHistoryRow, Recorded, TableId, TableRows},
    };
    let (mut frame, _) = bloc_war(2);
    let limit = engine::air_kamikaze::SUM_LIMIT;
    let rows = vec![
        AirNavalPatrolHistoryRow {
            country: 1,
            region: 7,
            index: 2,
            our_fighters_lost_raw: -20,
            scenario_channels: Recorded::Value(Some([-20; 7])),
            ..Default::default()
        },
        AirNavalPatrolHistoryRow {
            country: 1,
            region: 7,
            index: 0,
            our_fighters_lost_raw: limit,
            scenario_channels: Recorded::Value(Some([limit; 7])),
            ..Default::default()
        },
        AirNavalPatrolHistoryRow {
            country: 1,
            region: 7,
            index: 1,
            our_fighters_lost_raw: 10,
            scenario_channels: Recorded::Value(Some([10; 7])),
            ..Default::default()
        },
        AirNavalPatrolHistoryRow::availability_witness(1),
    ];
    *frame.tables[TableId::AirNavalPatrolHistories.index()] =
        TableView::Complete(TableRows::AirNavalPatrolHistories(rows));
    assert_eq!(
        ScenarioHistory::read(&frame, 1)
            .unwrap()
            .unwrap()
            .region(7)
            .unwrap(),
        [limit - 20; 8]
    );
}

#[test]
fn paired_rig_callbacks_use_defending_side_local_ordinals() {
    use super::super::entity_animation::{Clip, Node, Rig as ActorRig};
    let descriptors = mesh::parse_descriptors(br#"
        entity = { name = actor state = { name = idle } state = { name = success } state = { name = fail } }
        entity = { name = scenario state = { name = default
            event = { time = 2 id = d_1_success }
            event = { time = 3 id = d_2_fail }
            event = { time = 4 id = a_2_success }
            event = { time = 5 id = idle }
            event = { time = 6 id = d_9_success }
        } }
    "#, "synthetic.asset").unwrap();
    let actor = descriptors.inherit("actor").unwrap();
    let actor = ActorRig {
        nodes: vec![Node {
            mesh: Arc::new(mesh::Mesh::default()),
            native: None,
            clips: actor
                .states
                .values()
                .flatten()
                .map(|state| Clip {
                    state: (*state).clone(),
                    animation: None,
                })
                .collect(),
            default: "idle".into(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
        ..Default::default()
    };
    let bones = scenario_bones(14);
    let rig = Rig {
        sides: bones.iter().map(|bone| bone.0).collect(),
        tracks: vec![Vec::new(); 6],
        fps: 1.0,
        duration: 10.0,
        speed: 2.0,
        events: descriptors.inherit("scenario").unwrap().states["default"][0]
            .events
            .clone(),
    };
    let route = Route {
        points: Vec::new(),
        arcs: Vec::new(),
    };
    let flight = Flight {
        mesh: 0,
        wing: 1,
        region: 7,
        rig: String::new(),
        track: 4,
        origin: [0.0; 3],
        yaw: 0.0,
        outgoing: route.clone(),
        incoming: route,
        travel: 3.0,
        delay: 1.0,
        group_travel: 4.0,
        phase: 0.0,
        requests: Arc::default(),
    };
    let requests = flight_requests(&flight, &rig, &actor, 2.0);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.state.as_str())
            .collect::<Vec<_>>(),
        ["fail", "idle", "success"]
    );
    assert_eq!(
        requests
            .iter()
            .map(|request| request.time)
            .collect::<Vec<_>>(),
        [2.25, 2.75, 3.0]
    );
    assert_eq!(rig_key(scenario_name(14), 3), "fighter_3_vs_fighter_3_fail");
    assert_eq!(
        scenario_bones(13),
        [(0, "fighter1".into()), (1, "fighter2".into())]
    );
}

#[test]
fn strategic_threshold_conversion_uses_native_f32_rounding() {
    assert_eq!(threshold_raw(0.000005), 1);
    assert_eq!(threshold_raw(-0.000005), -1);
    assert_eq!(threshold_raw(1.5), 150_000);
    assert_eq!(threshold_raw(f32::NAN), i64::MIN);
    assert_eq!(threshold_raw(f32::INFINITY), i64::MIN);
}

#[test]
// Animation's private channel storage requires constructing this fixture via Default.
#[allow(clippy::field_reassign_with_default)]
fn scenario_tracks_bind_native_source_ordinals_without_animation_name_aliases() {
    use super::super::mesh_pose::Source;
    let source = Source {
        pdxmesh: true,
        skeleton: vec![
            mesh::NativeJoint {
                name: "root".into(),
                parent: 255,
                inverse_bind: [0.0; 12],
            },
            mesh::NativeJoint {
                name: "attachment".into(),
                parent: 0,
                inverse_bind: [0.0; 12],
            },
        ],
        entity_locators: Vec::new(),
        binary_locators: Vec::new(),
        mesh_scale: 2.0,
        entity_scale: 3.0,
        frozen: None,
        game_data: None,
    };
    let mut animation = mesh::Animation::default();
    animation.samples = 2;
    animation.fps = 1.0;
    animation.joints = vec![
        mesh::AnimationJoint {
            name: "different_root".into(),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            ..Default::default()
        },
        mesh::AnimationJoint {
            name: "different_child".into(),
            translation: [3.0, 4.0, 5.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            ..Default::default()
        },
    ];
    let tracks = scenario_tracks(&source, &animation, &[(0, "attachment".into())]).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].len(), 2);
    assert_eq!(tracks[0][0].translation, [18.0, 24.0, 30.0]);
    assert_eq!(tracks[0][0].basis[0], [3.0, 0.0, 0.0]);
    let error = scenario_tracks(&source, &animation, &[(0, "absent".into())]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("attachment node absent is absent")
    );
}
