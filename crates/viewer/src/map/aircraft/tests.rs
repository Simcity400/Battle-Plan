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
        refusal: None,
    };
    let batches = |planes| fighters.batches(fighters.sizing.units(planes));
    assert_eq!(batches(0), None);
    assert_eq!(batches(29), Some((1, 1)));
    assert_eq!(batches(60), Some((1, 2)));
    assert_eq!(batches(100), Some((3, 1)));
    assert_eq!(batches(400), Some((3, 4)));
    // A modded transport step of two singles drops the odd unit; the
    // loops that compare it with their wings refuse it.
    let paired = SteppedFamily {
        single: 2,
        ..fighters
    };
    assert_eq!(paired.batches(1), None);
    assert_eq!(paired.batches(2), Some((1, 1)));
    assert_eq!(paired.counting_wings("refused").refusal, Some("refused"));
    assert_eq!(fighters.counting_wings("refused"), fighters);
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
fn only_the_a_side_flies_naval_patrol_as_kamikaze() {
    let patrol = AIR_MISSION_NAVAL_PATROL | 1;
    assert_eq!(scenario_mission("fighter", "patrol", patrol, 0), "kamikaze");
    // The B loop's kinds 0/1: the same key, and so the same cap, as any
    // fighter patrol.
    let b = scenario_mission("fighter", "patrol", patrol, 1);
    assert_eq!(rig_key(&format!("fighter_3_{b}"), 0), "fighter_3_patrol");
    assert_eq!(scenario_mission("fighter", "patrol", 1, 0), "patrol");
}

/// `14122EFD0`'s kamikaze path ends on too few units without the cap
/// check (`141232056`), unlike the other viewer/A fighters and scouts.
#[test]
fn a_zero_unit_kamikaze_wing_ends_its_loop_unchecked() {
    let fighters = SteppedFamily {
        sizing: FamilyDefines {
            factor: 30,
            triple: 3,
            limit: 2,
        },
        single: 1,
        refusal: None,
    };
    let patrol = AIR_MISSION_NAVAL_PATROL | 1;
    assert_eq!(fighters.batches(fighters.sizing.units(0)), None);
    assert!(!runs_single_cap(Category::Fighter, 0, patrol));
    assert!(runs_single_cap(Category::Fighter, 0, 1));
    assert!(!runs_single_cap(Category::Fighter, 1, 1));
    assert!(runs_single_cap(Category::Scout, 0, patrol));
    assert!(!runs_single_cap(Category::Transport, 0, 0));
}

/// `14122EFD0`'s unpaired bomber loops go on after a refused batch
/// (`141231AF2 -> 141231B6E`); the stepped loops break.
#[test]
fn a_refused_unpaired_bomber_skips_only_itself() {
    let mut caps = Caps::default();
    let bombers = (5, 0, 0);
    assert!(caps.admit(bombers, "bomber", "bomber_1_CAS", 1, 1));
    assert!(!caps.admit(bombers, "bomber", "bomber_1_CAS", 1, 1));
    assert!(!caps.stopped.contains(&bombers));
    // The next bomber, of a kind whose cap is free, still flies.
    assert!(caps.admit(bombers, "bomber", "bomber_3_bombing", 1, 1));
    // A refused fighter batch ends that side's fighter loop, whatever
    // kind its next wing would fly.
    let fighters = (5, 0, 1);
    assert!(caps.admit(fighters, "fighter", "fighter_1_patrol", 1, 2));
    assert!(!caps.admit(fighters, "fighter", "fighter_1_patrol", 1, 1));
    assert!(caps.stopped.contains(&fighters));
    // The cap is checked before the batch, so a batch can exceed it.
    assert_eq!(caps.counts[&(5, "fighter_1_patrol".to_owned())], 2);
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
        assert!(groups.admit(&visit(0, 0), 1));
    }
    assert!(!groups.admit(&visit(0, 0), 1));
    // Other groups of the same call still fill.
    assert!(groups.admit(&visit(0, 0), 4));
    groups.counts[0] = [GROUP_CAPACITY; 5];
    // A new call on a full side is skipped whole; side B is separate.
    assert!(!groups.admit(&visit(1, 0), 4));
    assert!(groups.admit(&visit(2, 1), 1));
    // A new region starts empty.
    let other = Visit {
        region: 6,
        ..visit(3, 0)
    };
    assert!(groups.admit(&other, 1));
}

#[test]
fn take_off_and_landing_scale_with_the_content_distances() {
    let sizing = FamilyDefines {
        factor: 1,
        triple: 3,
        limit: 2,
    };
    let stepped = SteppedFamily {
        sizing,
        single: 1,
        refusal: None,
    };
    let mut defines = FlightDefines {
        fighter: stepped,
        scout: stepped,
        bomber: sizing,
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
fn supported_missions_follow_priority_and_training_is_separate() {
    assert_eq!(
        family(air_reachability::category(4, 0, 0).unwrap(), 4),
        Some(("fighter", "patrol"))
    );
    assert_eq!(
        family(air_reachability::category(2 | 8, 17, 0).unwrap(), 2 | 8),
        Some(("bomber", "CAS"))
    );
    assert_eq!(
        family(air_reachability::category(1 << 14, 17, 0).unwrap(), 1 << 14),
        Some(("scout", "patrol"))
    );
    assert_eq!(
        family(air_reachability::category(1 << 10, 17, 0).unwrap(), 1 << 10),
        Some(("transport", "supplying"))
    );
    assert_eq!(air_reachability::category(1 << 10, 0, 0), None);
    assert_eq!(air_reachability::category(0, 0, 0), None);
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

mod training;
