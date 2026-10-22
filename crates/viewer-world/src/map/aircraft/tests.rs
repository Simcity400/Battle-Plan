use super::*;
use viewer_foundation::map::aircraft_model;
fn wing(id: u64, owner: u32, region: u32) -> AirWingRow {
    AirWingRow {
        id,
        owner,
        mission_region: Some(region),
        ..Default::default()
    }
}

fn registration(wing: u64, ordinal: u32) -> replay_data::rows::AirRegionalMissionRow {
    replay_data::rows::AirRegionalMissionRow {
        wing,
        ordinal,
        ..Default::default()
    }
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
    use replay_data::{rows::TableId, rows::TableRows, rows::VolunteerHostRow};
    use replay_frame::{frame::Metrics, frame::TableView};
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
    use replay_data::{rows::TableId, rows::TableRows, rows::VolunteerHostRow};
    use replay_frame::{frame::Metrics, frame::TableView};
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
    assert_eq!(roster.refused_declaration("plane"), None);
}

#[test]
fn lazy_explicit_aircraft_keep_named_counted_attachment_diagnostics_without_audio_refusals() {
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
        (&defines, &defines),
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
    let selected = aircraft_model::resolve_lazy(
        &replay_data::rows::VariantDefinitionRow {
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
        *kind == installed_content_core::DiagnosticKind::Refused
            && line.contains("AbsentFlap")
            && line.contains("(2)")
    }));
    // Its sound events play; they are no longer refused.
    assert!(typed.iter().all(|(_, line)| !line.contains("sound")));
    let report = AircraftPlan::default().report(&assets);
    assert!(
        report
            .iter()
            .any(|line| line.contains("AbsentFlap") && line.contains("(2)"))
    );
    assert!(report.iter().all(|line| !line.contains("sound")));
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
        roster.refused_declaration("plane"),
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
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Refused);
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

/// A plane the visual roster cannot decide is drawn from the recorded roster only when
/// that roster registers it without a refused declaration; an absent or refused one stays
/// refused by name rather than being baked from the recording.
#[test]
fn a_plane_the_mods_leave_undecided_uses_only_an_accepted_recorded_declaration() {
    use crate::map::models::tests::{MESHES_GFX, PLANE_ASSET, triangle_mesh, visual_stack};

    let (root, layers) = visual_stack(
        "aircraft_undecided",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/entities/plane.asset", PLANE_ASSET),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            (
                "base/gfx/entities/refused.asset",
                br#"entity = { name = refused_plane pdxmesh = body
                    state = { name = idle animation_speed = 0 } }"#,
            ),
            (
                "art/gfx/entities/broken.asset",
                br#"entity = { name = plane refused_plane zeppelin } }{ = ="#,
            ),
        ],
    );
    let presentation = layers.presentation();
    let roster = presentation.descriptor_roster().unwrap();
    let recorded = presentation.recorded_descriptor_roster().unwrap().unwrap();
    assert_eq!(recorded.refused_declaration("plane"), None);
    assert_eq!(
        recorded.refused_declaration("refused_plane").as_deref(),
        Some("state idle has unsupported non-positive animation_speed 0")
    );
    let declared = ["plane", "refused_plane", "zeppelin"].map(str::to_owned);
    for entity in &declared {
        assert!(roster.refused_declaration(entity).is_some(), "{entity}");
    }
    let (library, unusable) = plane_library(
        presentation,
        Decode::default(),
        &roster,
        &declared,
        &mut Vec::new(),
    )
    .unwrap();
    let slot = library.slot("plane").unwrap();
    assert!(!library.meshes[slot].visual);
    assert!(!unusable.contains_key("plane"));
    for entity in ["refused_plane", "zeppelin"] {
        assert_eq!(library.slot(entity), None);
        assert_eq!(
            unusable.get(entity),
            roster.refused_declaration(entity).as_ref()
        );
        assert!(
            presentation
                .refused()
                .contains(&format!("model entity {entity}"))
        );
    }
    assert!(!presentation.refused().contains("model entity plane"));
    let refusals = presentation.read_warnings().overlay_refusals().unwrap();
    assert!(refusals.contains("model plane ("), "{refusals}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The ordinary and lazy paths share the same plane library; the rest of these assets
/// needs no content for a test that chooses its aircraft by name.
fn aircraft_with_planes(layers: &ResolvedLayers, declared: &[String]) -> AircraftAssets {
    let map = MapData {
        width: 1,
        height: 1,
        province_ids: vec![0],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let defines = Defines::new().unwrap();
    let mut assets = AircraftAssets::load(
        &ResolvedLayers::default(),
        Decode::default(),
        &map,
        (&defines, &defines),
        &BTreeMap::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assets.layers = layers.clone();
    assets.roster = layers.descriptor_roster().unwrap();
    let (library, unusable) = plane_library(
        layers,
        Decode::default(),
        &assets.roster,
        declared,
        &mut Vec::new(),
    )
    .unwrap();
    assets.library = RwLock::new(library);
    assets.unusable = Mutex::new(unusable);
    assets
}

#[test]
fn fly_uses_a_recorded_plane_when_the_mods_refuse_its_declaration() {
    use crate::map::models::tests::{MESHES_GFX, triangle_mesh, visual_stack};

    let (root, layers) = visual_stack(
        "fly_recorded_declaration",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            (
                "base/gfx/entities/planes.asset",
                br#"entity = { name = plane pdxmesh = body state = { name = idle } }
                    entity = { name = lazy_plane clone = plane }"#,
            ),
            (
                "art/gfx/entities/planes.asset",
                br#"entity = { name = plane pdxmesh = body
                        state = { name = idle animation_speed = 0 } }
                    entity = { name = lazy_plane clone = plane }
                    entity = { name = unrecoverable_plane clone = plane }"#,
            ),
        ],
    );
    let presentation = layers.presentation();
    let assets = aircraft_with_planes(presentation, &["plane".to_owned()]);
    let slot = assets.library().slot("plane").unwrap();
    assert!(!assets.library().meshes[slot].visual);
    assert_eq!(assets.player("plane").unwrap().slot, slot);
    assert!(assets.library().slot("lazy_plane").is_none());
    let lazy = assets.player("lazy_plane").unwrap();
    assert!(!assets.library().meshes[lazy.slot].visual);
    let refusals = presentation.read_warnings().overlay_refusals().unwrap();
    for name in ["plane", "lazy_plane"] {
        assert!(refusals.contains(&format!("model {name} (")), "{refusals}");
        assert!(
            !presentation
                .refused()
                .contains(&format!("model entity {name}"))
        );
    }
    assert!(assets.player("unrecoverable_plane").is_err());
    assert!(
        presentation
            .refused()
            .contains("model entity unrecoverable_plane")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn recorded_mesh_retries_and_draw_counterparts_keep_the_planes_declaration_check() {
    use crate::map::models::tests::{MESHES_GFX, triangle_mesh, visual_stack};

    let (root, layers) = visual_stack(
        "aircraft_recorded_mesh_admission",
        &[
            ("base/gfx/models/meshes.gfx", MESHES_GFX),
            ("base/gfx/models/plane.mesh", &triangle_mesh(1.)),
            (
                "base/gfx/entities/planes.asset",
                br#"entity = { name = plane pdxmesh = body
                        state = { name = idle animation_speed = 0 } }
                    entity = { name = lazy_plane clone = plane }
                    entity = { name = counterpart_plane clone = plane }"#,
            ),
            (
                "art/gfx/entities/planes.asset",
                br#"entity = { name = plane pdxmesh = broken_body state = { name = idle } }
                    entity = { name = lazy_plane clone = plane }
                    entity = { name = counterpart_plane pdxmesh = body
                        state = { name = idle } }"#,
            ),
            (
                "art/gfx/models/broken.gfx",
                br#"objectTypes = { pdxmesh = { name = broken_body
                    file = "gfx/models/broken.mesh" } }"#,
            ),
            ("art/gfx/models/broken.mesh", b"not a mesh"),
            ("art/gfx/models/plane.mesh", &triangle_mesh(2.)),
        ],
    );
    let presentation = layers.presentation();
    let assets = aircraft_with_planes(
        presentation,
        &["plane".to_owned(), "counterpart_plane".to_owned()],
    );
    assert_eq!(assets.library().slot("plane"), None);
    assert!(assets.player("plane").is_err());
    assert_eq!(assets.library().slot("lazy_plane"), None);
    assert!(assets.player("lazy_plane").is_err());
    assert_eq!(assets.library().slot("lazy_plane"), None);
    let shown = assets.player("counterpart_plane").unwrap();
    let library = assets.library();
    assert!(library.meshes[shown.slot].visual);
    assert!(!library.alternatives.contains_key(&shown.slot));
    for name in ["plane", "lazy_plane"] {
        assert!(
            presentation
                .refused()
                .contains(&format!("model entity {name}"))
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

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
    let reason = |entity: &str| roster.refused_declaration(entity);
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
    // not loaded rather than drawn, with why.
    assert!(
        unusable["clean_plane_entity"].starts_with(&format!(
            "{NOT_LOADED}: clean_plane_entity: pdxmesh clean_mesh (clean.mesh) cannot be loaded: "
        )),
        "{}",
        unusable["clean_plane_entity"]
    );
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Refused);
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
        roster.refused_declaration("GER_plane_entity"),
        Some(format!("it {why}"))
    );
    assert_eq!(
        roster.refused_declaration("JAP_plane_entity"),
        Some(format!("its mesh ger_mesh {why}"))
    );
    assert_eq!(roster.refused_declaration("plane_entity"), None);

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
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Refused);
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
fn naval_patrol_uses_combat_location_and_recorded_losses() {
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
fn region_draws_keep_first_listings() {
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
    let mut defines = paired::defines();
    defines.clock_speed = 0.22;
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
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Warning);
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

mod band;
mod model_inputs;
mod launch_effects;
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
    use replay_data::{
        rows::AirNavalPatrolHistoryRow, rows::Recorded, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
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
    use replay_data::{
        rows::AirNavalPatrolHistoryRow, rows::Recorded, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
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
    use replay_data::{
        rows::AirNavalPatrolHistoryRow, rows::Recorded, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    let (mut frame, _) = bloc_war(2);
    let limit = engine_air_history::air_kamikaze::SUM_LIMIT;
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
    let descriptors = mesh::parse_descriptors(
        br#"
        entity = { name = scenario state = { name = default
            event = { time = 2 id = d_1_success }
            event = { time = 3 id = d_2_fail }
            event = { time = 4 id = a_2_success }
            event = { time = 5 id = idle }
            event = { time = 6 id = d_9_success }
        } }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    let actor = paired::actor_rig();
    let bones = scenario_bones(14);
    let rig = Rig {
        sides: bones.iter().map(|bone| bone.0).collect(),
        tracks: vec![Vec::new(); 6],
        fps: 1.0,
        duration: 10.0,
        speed: 2.0,
        unbound: vec![None; 6],
        lands: vec![true; 6],
        rockets: vec![false; 6],
        events: descriptors.inherit("scenario").unwrap().states["default"][0]
            .events
            .clone(),
    };
    let route = Route {
        points: Vec::new(),
        arcs: Vec::new(),
    };
    let flight = Flight {
        mission: AirMission::Combat,
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
        impact: None,
        lands: true,
        explode: Explode::Absent,
        terrain: None,
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
    use crate::map::mesh_pose::Source;
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
    let (tracks, absent) =
        scenario_tracks(&source, &animation, &[(0, "attachment".into())]).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].len(), 2);
    assert_eq!(tracks[0][0].translation, [18.0, 24.0, 30.0]);
    assert_eq!(tracks[0][0].basis[0], [3.0, 0.0, 0.0]);
    assert_eq!(absent, [false]);
    // An absent node is the entity's root for every sample: the rig origin
    // at the entity's scale, as the game reads for an unfound node.
    let (tracks, absent) = scenario_tracks(
        &source,
        &animation,
        &[(0, "absent".into()), (1, "attachment".into())],
    )
    .unwrap();
    assert_eq!(absent, [true, false]);
    assert_eq!(tracks[0].len(), 2);
    for pose in &tracks[0] {
        assert_eq!(pose.translation, [0.0; 3]);
        assert_eq!(
            pose.basis,
            [[3.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 3.0]]
        );
    }
    assert_eq!(tracks[1][0].translation, [18.0, 24.0, 30.0]);
}

#[test]
fn scenario_rigs_log_an_absent_node_and_read_rocket_speed_and_band_only_where_needed() {
    use crate::map::mesh_pose::Source;
    let source = Source {
        pdxmesh: true,
        skeleton: vec![mesh::NativeJoint {
            name: "joint1".into(),
            parent: 255,
            inverse_bind: [0.0; 12],
        }],
        entity_locators: Vec::new(),
        binary_locators: Vec::new(),
        mesh_scale: 1.0,
        entity_scale: 0.12,
        frozen: None,
        game_data: None,
    };
    let mut animation = mesh::Animation::default();
    animation.samples = 3;
    animation.fps = 2.0;
    animation.joints = vec![mesh::AnimationJoint {
        name: "joint1".into(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        ..Default::default()
    }];
    let band = Band {
        offset: 0.0,
        explosion_delay: 0.6,
    };
    let rig = |(kind, outcome): (u8, u8),
               bones: Vec<(u8, String)>,
               speed: Result<f32, &'static str>,
               band: Result<Band, &'static str>| {
        let mut warnings = Vec::new();
        let rig = scenario_rig(
            "scenario_entity",
            (kind, outcome),
            &bones,
            (&source, &animation),
            1.0,
            Vec::new(),
            (&speed, &band),
            &mut warnings,
        );
        (rig, warnings)
    };
    // Vanilla's rocket rig: the node is absent, the game's log line is kept
    // and the rocket rides the root at ROCKET_SPEED, without a landing.
    let (missile, warnings) = rig((15, 0), scenario_bones(15), Ok(15.0), Ok(band));
    let missile = missile.unwrap();
    assert_eq!(
        (missile.rockets.as_slice(), missile.lands.as_slice()),
        (&[true][..], &[false][..])
    );
    assert_eq!(missile.unbound, [Some(15.0)]);
    assert_eq!(missile.duration, 1.0);
    assert_eq!(
        warnings,
        [(
            installed_content_core::DiagnosticKind::Logged,
            "Aircraft scenario scenario_entity has no attachment node rocket: the game logs it and that actor rides the scenario entity's root".to_owned()
        )]
    );
    // A missing ROCKET_SPEED refuses only that rig.
    let (missile, warnings) = rig(
        (15, 0),
        scenario_bones(15),
        Err("NDefines.NAirGfx.ROCKET_SPEED is unreadable"),
        Ok(band),
    );
    assert!(missile.is_none());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Refused);
    assert!(warnings[0].1.contains("ROCKET_SPEED is unreadable"));
    let (bomber, warnings) = rig(
        (6, 0),
        vec![(0, "bomber".into())],
        Err("NDefines.NAirGfx.ROCKET_SPEED is unreadable"),
        Ok(band),
    );
    assert_eq!(bomber.unwrap().unbound, [Some(UNBOUND_SPEED)]);
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Logged);
    let (bound, _) = rig((6, 0), vec![(0, "joint1".into())], Err("unused"), Ok(band));
    assert_eq!(bound.unwrap().unbound, [None]);
    // A rocket on a real node flies: the near-ground band is ported.
    let (rocket, warnings) = rig((15, 0), vec![(0, "joint1".into())], Ok(15.0), Ok(band));
    let rocket = rocket.unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        (rocket.unbound.as_slice(), rocket.lands.as_slice()),
        (&[None][..], &[false][..])
    );
    // A rocket on its node never rides the root, so an unreadable
    // ROCKET_SPEED leaves it flying.
    let (rocket, warnings) = rig(
        (15, 0),
        vec![(0, "joint1".into())],
        Err("NDefines.NAirGfx.ROCKET_SPEED is unreadable"),
        Ok(band),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(rocket.unwrap().unbound, [None]);
    // Unreadable band defines refuse only a rig with an actor that does not
    // land, by name; a landing rig never reads them.
    let missing = Err("NDefines.NAirGfx.AIRPLANES_GROUND_COLLISION_OFFSET_Y is unreadable");
    let (rocket, warnings) = rig((15, 0), vec![(0, "joint1".into())], Ok(15.0), missing);
    assert!(rocket.is_none());
    assert_eq!(warnings[0].0, installed_content_core::DiagnosticKind::Refused);
    assert!(
        warnings[0]
            .1
            .contains("AIRPLANES_GROUND_COLLISION_OFFSET_Y is unreadable")
    );
    let (bomber, warnings) = rig((6, 0), vec![(0, "joint1".into())], Err("unused"), missing);
    assert!(bomber.is_some() && warnings.is_empty());
}

/// The scenario analysis builds no landing route for the rocket, kamikaze
/// fighters that succeed, defending slots of a successful interception or
/// dogfight and attacking slots of a lost dogfight. The viewer used to fly
/// all of them home.
#[test]
fn the_slots_the_game_gives_no_landing_route() {
    let landing = |kind: u8, outcome: u8| {
        scenario_bones(kind)
            .iter()
            .map(|(side, _)| lands(kind, outcome, *side))
            .collect::<Vec<_>>()
    };
    assert_eq!(landing(15, 0), [false]);
    for kind in [16, 17] {
        assert!(landing(kind, 0).iter().all(|lands| *lands));
        assert!(landing(kind, SUCCESS).iter().all(|lands| !lands));
    }
    for kind in [12, 18] {
        assert_eq!(landing(kind, 1), [true, true]);
        assert_eq!(landing(kind, SUCCESS), [true, false]);
    }
    assert_eq!(landing(13, 1), [true, true]);
    assert_eq!(landing(13, SUCCESS), [true, false]);
    assert_eq!(landing(13, FAIL), [false, true]);
    assert_eq!(
        landing(14, SUCCESS),
        [true, true, true, false, false, false]
    );
    assert_eq!(landing(14, FAIL), [false, false, false, true, true, true]);
    for kind in (0..=11).chain([19, 20]) {
        assert!(landing(kind, 0).iter().all(|lands| *lands), "kind {kind}");
    }
}

/// One scenario of one actor that takes off for a second and plays `poses`
/// at `fps`, over flat terrain at height 10 (heightmap byte 100), anchored
/// at x = 2, z = 2 on that terrain.
fn one_run(
    poses: Vec<Transform>,
    fps: f32,
    lands: bool,
    rocket: bool,
    explode: Explode,
) -> (AircraftPlan, BTreeMap<String, Rig>) {
    let duration = (poses.len() - 1) as f32 / fps;
    let rig = Rig {
        tracks: vec![poses],
        sides: vec![0],
        fps,
        duration,
        events: Vec::new(),
        speed: 1.0,
        unbound: vec![None],
        lands: vec![lands],
        rockets: vec![rocket],
    };
    let heights = HeightField {
        width: 8,
        height: 8,
        values: &[100; 64],
    };
    let route = Route {
        points: Vec::new(),
        arcs: Vec::new(),
    };
    let plan = AircraftPlan {
        flights: vec![Flight {
            mission: AirMission::Missile,
            mesh: 3,
            wing: 1,
            region: 7,
            rig: "rig".into(),
            track: 0,
            origin: [2.0, 10.0, 2.0],
            yaw: 0.0,
            outgoing: route.clone(),
            incoming: route,
            travel: 1.0,
            delay: 0.0,
            group_travel: 1.0,
            phase: 0.0,
            requests: Arc::default(),
            impact: Some((9, [1.0, 2.0, 3.0], [0.0, 0.0, 1.0, 1.0])),
            lands,
            explode,
            terrain: (!lands).then(|| Arc::new(Terrain::capture(&heights, (0.0, 6.0), (0.0, 6.0)))),
        }],
        runs: vec![Run {
            flights: 0..1,
            start: 0.0,
            cycle: 0,
            grounds: vec![Ground::Following],
        }],
        ..AircraftPlan::default()
    };
    (plan, BTreeMap::from([("rig".to_owned(), rig)]))
}

fn height(y: f32) -> Transform {
    Transform {
        translation: [0.0, y, 0.0],
        ..Transform::IDENTITY
    }
}

const BAND: Band = Band {
    offset: 0.0,
    explosion_delay: 0.6,
};

/// The game tests the band on the pose an actor draws at each of its
/// frames. A 120 Hz rig dips into the terrain between two 60 Hz frames: no
/// frame draws it there, so nothing collides; a dip a frame draws ends the
/// rocket's scenario on that frame and plays its explosion. Before, the
/// viewer tested only the rig's samples, and then a dense path between
/// them, finding collisions no frame draws.
#[test]
fn the_band_is_met_at_the_viewer_s_frames() {
    let defines = FlightDefines::for_tests();
    let dip = |sample: usize| {
        let mut poses = vec![height(3.0); 13];
        poses[sample] = height(-1.0);
        one_run(poses, 120.0, false, true, Explode::Absent)
    };
    let frames: Vec<f32> = (0..=7).map(|frame| 1.0 + frame as f32 / 60.0).collect();
    // Sample 5 (rig time 5/120) lies between the frames at 2/60 and 3/60.
    let (mut plan, rigs) = dip(5);
    for &frame in &frames {
        plan.advance(&rigs, &defines, Some(BAND), frame, &|_| None);
    }
    assert_eq!((plan.runs[0].cycle, plan.finished.len()), (0, 0));
    // Sample 6 is the frame at 3/60.
    let (mut plan, rigs) = dip(6);
    let mut ended = None;
    for &frame in &frames {
        plan.advance(&rigs, &defines, Some(BAND), frame, &|_| None);
        if ended.is_none() && plan.runs[0].cycle == 1 {
            ended = Some(frame);
        }
    }
    assert!((ended.unwrap() - (1.0 + 3.0 / 60.0)).abs() < 1e-5);
    assert_eq!(plan.finished.len(), 1);
    assert_eq!(plan.finished[0].id, 1);
    // A clock that goes back restarts the run at its phase.
    plan.advance(&rigs, &defines, Some(BAND), 0.5, &|_| None);
    assert_eq!(plan.runs[0].cycle, 0);
}

/// In the band an actor with an `explode` state requests it and stays at
/// the pose it drew; it is done, and hidden for good, once its own player
/// reads the band's delay. Without the state it is hidden at once, and done
/// while it stays at or below the band. Actors with a landing route never
/// end a scenario early.
#[test]
fn an_actor_in_the_band_explodes_or_hides_by_its_own_player() {
    let defines = FlightDefines::for_tests();
    let falling = || vec![height(3.0), height(1.0), height(-1.0), height(-1.0)];
    let (mut plan, rigs) = one_run(falling(), 1.0, false, false, Explode::Plays);
    // Frames at rig times 0.5 and 1.5: the second draws it at y 10, on the
    // terrain and so in the band.
    plan.advance(&rigs, &defines, Some(BAND), 1.5, &|_| None);
    assert_eq!(plan.runs[0].grounds[0], Ground::Following);
    plan.advance(&rigs, &defines, Some(BAND), 2.5, &|_| None);
    let Ground::Exploding {
        pose,
        entry,
        hidden,
    } = plan.runs[0].grounds[0]
    else {
        panic!("{:?}", plan.runs[0].grounds);
    };
    assert_eq!(
        (pose.translation, entry, hidden),
        ([2.0, 10.0, 2.0], 2.5, false)
    );
    // Its player reads below the delay: still drawn, frozen.
    plan.advance(&rigs, &defines, Some(BAND), 2.6, &|_| {
        Some(Some((0.5, 2.0)))
    });
    let placed = plan.placed_runs(&rigs, &defines, 2.6);
    assert_eq!(placed[0].1.position, [2.0, 10.0, 2.0]);
    // Reading the delay hides it and, as the scenario's only actor, ends it.
    plan.advance(&rigs, &defines, Some(BAND), 2.7, &|key| {
        assert_eq!(key, (3, 0, 0));
        Some(Some((0.6, 2.0)))
    });
    assert_eq!((plan.runs[0].cycle, plan.finished.len()), (1, 1));
    // Without `explode` it is hidden on its first frame in the band.
    let (mut plan, rigs) = one_run(falling(), 1.0, false, false, Explode::Absent);
    plan.advance(&rigs, &defines, Some(BAND), 2.5, &|_| None);
    assert_eq!(plan.runs[0].cycle, 1);
    // A landing actor never ends it early, and lands.
    let (mut plan, rigs) = one_run(falling(), 1.0, true, false, Explode::Absent);
    for frame in [1.5, 2.5, 3.5] {
        plan.advance(&rigs, &defines, Some(BAND), frame, &|_| None);
    }
    assert_eq!((plan.runs[0].cycle, plan.finished.len()), (0, 0));
    // The cycle is 1 + 3 + 1 seconds; its landing ends at 5.
    plan.advance(&rigs, &defines, Some(BAND), 5.1, &|_| None);
    assert_eq!((plan.runs[0].cycle, plan.finished.len()), (1, 1));
}

/// A formation whose actors read the explosion delay on successive frames
/// ends on the frame the last one does: an actor that has read it stays
/// done and placed, hidden, so its player keeps stepping. Before, the first
/// to finish lost its placement and player, read as unfinished, and the
/// scenario never ended.
#[test]
fn a_formation_finishing_on_successive_frames_ends_with_its_last_actor() {
    let defines = FlightDefines::for_tests();
    let falling = vec![height(3.0), height(1.0), height(-1.0), height(-1.0)];
    let (mut plan, rigs) = one_run(falling, 1.0, false, false, Explode::Plays);
    let flight = plan.flights[0].clone();
    plan.flights = vec![flight.clone(), flight.clone(), flight];
    plan.runs[0].flights = 0..3;
    plan.runs[0].grounds = vec![Ground::Following; 3];
    plan.advance(&rigs, &defines, Some(BAND), 2.5, &|_| None);
    assert!(
        plan.runs[0]
            .grounds
            .iter()
            .all(|ground| matches!(ground, Ground::Exploding { hidden: false, .. }))
    );
    // Each actor's player reads the delay on its own frame, and no other.
    for (frame, actor) in [(2.6, 0), (2.7, 1)] {
        plan.advance(&rigs, &defines, Some(BAND), frame, &|(_, index, _)| {
            (index == actor).then_some(Some((0.6, 2.0)))
        });
        assert_eq!(plan.runs[0].cycle, 0, "{frame}");
    }
    // The finished ones stay placed, frozen, for their players.
    assert_eq!(plan.placed_runs(&rigs, &defines, 2.75).len(), 3);
    plan.advance(&rigs, &defines, Some(BAND), 2.8, &|(_, index, _)| {
        (index == 2).then_some(Some((0.6, 2.0)))
    });
    assert_eq!((plan.runs[0].cycle, plan.finished.len()), (1, 3));
}

/// The shared player keeps reading the clip a pdxmesh root played before it
/// entered an `explode` state that plays no animation, past that clip's end
/// when it does not loop: entered 0.25 s into a 1 s idle, it reads the 2 s
/// delay 1.75 s later. The band reads that player, so the explosion ends.
#[test]
fn a_retained_nonlooping_clip_reads_the_explosion_delay() {
    use crate::map::entity_animation::{Clip, Clock, Node, Player, Request, Rig as ActorRig};
    use crate::map::mesh_pose::Source;
    let mut animation = mesh::Animation::default();
    animation.fps = 1.0;
    animation.samples = 2;
    animation.joints = vec![mesh::AnimationJoint {
        name: "joint1".into(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        ..Default::default()
    }];
    let clip = |name: &str, animation: Option<Arc<mesh::Animation>>| Clip {
        state: mesh::EntityState {
            name: name.into(),
            chance: 1.0,
            speed: 1.0,
            looping: false,
            ..mesh::EntityState::default()
        },
        animation,
    };
    let rig = ActorRig {
        nodes: vec![Node {
            mesh: Arc::new(mesh::Mesh::default()),
            native: Some(Arc::new(Source {
                pdxmesh: true,
                skeleton: vec![mesh::NativeJoint {
                    name: "joint1".into(),
                    parent: 255,
                    inverse_bind: [0.0; 12],
                }],
                entity_locators: Vec::new(),
                binary_locators: Vec::new(),
                mesh_scale: 1.0,
                entity_scale: 1.0,
                frozen: None,
                game_data: None,
            })),
            clips: vec![
                clip("idle", Some(Arc::new(animation))),
                clip(EXPLODE_STATE, None),
            ],
            default: "idle".into(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
        start: 0,
        end: 1,
    };
    let requests = Arc::new(vec![Request {
        time: 0.25,
        phase_time: None,
        state: EXPLODE_STATE.into(),
    }]);
    let mut player = Player::new(&rig);
    let mut palette = [crate::map::models::PaletteJoint::from_transform(
        &Transform::IDENTITY,
    )];
    let read =
        |player: &mut Player, palette: &mut [crate::map::models::PaletteJoint], time: f32| {
            player.evaluate_clock(
                &rig,
                ModelInstance::default(),
                &Clock {
                    time,
                    phase: None,
                    requests: Arc::clone(&requests),
                },
                palette,
                &mut Vec::new(),
            );
            player.root_play_position(&rig).unwrap().0
        };
    for step in 0..=8 {
        read(&mut player, &mut palette, step as f32 * 0.25);
    }
    assert!(read(&mut player, &mut palette, 2.0) >= 2.0);
    let mut early = Player::new(&rig);
    for step in 0..=7 {
        read(&mut early, &mut palette, step as f32 * 0.25);
    }
    assert!(read(&mut early, &mut palette, 1.75) < 2.0);
}

/// The game eases each actor's drawn transform toward its pose every frame:
/// per elapsed 30 Hz frame the position moves
/// `AIRPLANES_SMOOTH_INTERPOLATION_MOVE` of the way and the heading and up
/// axis `_TURN` times the clock's speed. The drawn actor lags a node that
/// moves; the band and the runs keep reading the pose. A new cycle starts
/// on its take-off route's first point, and a later hour's plan carries the
/// transforms on. Before, the viewer drew the pose itself.
#[test]
fn the_drawn_transform_eases_toward_the_pose_and_lags_it() {
    let defines = FlightDefines::for_tests();
    let along = |x: f32| Transform {
        translation: [x, 3.0, 0.0],
        ..Transform::IDENTITY
    };
    let (mut plan, rigs) = one_run(
        vec![along(0.0), along(0.0), along(6.0), along(6.0)],
        1.0,
        true,
        false,
        Explode::Absent,
    );
    plan.flights[0].outgoing = Route {
        points: vec![[-10.0, 10.0, 2.0], [2.0, 10.0, 2.0]],
        arcs: vec![0.0, 12.0],
    };
    plan.flights[0].incoming = Route {
        points: vec![[2.0, 13.0, 2.0], [-10.0, 10.0, 2.0]],
        arcs: vec![0.0, 153.0_f32.sqrt()],
    };
    let frame = 1.0 / 30.0;
    let drawn = |plan: &mut AircraftPlan, seconds: f32| {
        plan.advance(&rigs, &defines, Some(BAND), seconds, &|_| None);
        let pose = plan.placed_runs(&rigs, &defines, seconds)[0].1;
        let eased = plan.eased_runs(&rigs, &defines, seconds)[0].1;
        (pose, eased)
    };
    // First seen mid-rig, it stands on its pose.
    let (pose, eased) = drawn(&mut plan, 2.0);
    assert_eq!(eased.position, pose.position);
    // One frame later the node has moved 0.2 along x; the drawn actor 13% of
    // that.
    let (pose, eased) = drawn(&mut plan, 2.0 + frame);
    assert!((pose.position[0] - 2.2).abs() < 1e-4);
    assert!(
        (eased.position[0] - (2.0 + 0.2 * 0.13)).abs() < 1e-4,
        "{:?}",
        eased.position
    );
    // Two frames' time in one frame step twice as far, never past the pose.
    let before = eased.position[0];
    let (pose, eased) = drawn(&mut plan, 2.0 + 3.0 * frame);
    let expected = before + (pose.position[0] - before) * 0.26;
    assert!((eased.position[0] - expected).abs() < 1e-4);
    // A long pause brings it onto the pose.
    let (pose, eased) = drawn(&mut plan, 4.9);
    assert!((eased.position[0] - pose.position[0]).abs() < 1e-3);
    // The next cycle starts on the take-off route's first point and eases
    // from there.
    let (pose, eased) = drawn(&mut plan, 5.02);
    let start = [-10.0, 10.0, 2.0];
    let moved = (0.13 * 0.12 * 1000.0 / EASING_FRAME_MS).min(1.0);
    let expected = add(start, mul(sub(pose.position, start), moved));
    assert!(
        (0..3).all(|axis| (eased.position[axis] - expected[axis]).abs() < 1e-3),
        "{:?} {expected:?}",
        eased.position
    );
    assert_ne!(eased.position, pose.position);
    // A later plan of the same flight carries the transform on.
    let mut next = plan.clone();
    next.eased.clear();
    next.eased_at = None;
    next.carry_easing(&mut plan);
    let (pose, carried) = drawn(&mut next, 5.02 + frame);
    assert_ne!(carried.position, pose.position);
    // Without it the new plan stands on the pose.
    let mut fresh = next.clone();
    fresh.eased.clear();
    fresh.eased_at = None;
    let (pose, snapped) = drawn(&mut fresh, 5.02 + 2.0 * frame);
    assert_eq!(snapped.position, pose.position);
}

/// A missile scenario plays its explosion once per cycle, when its landing
/// ends or its rocket ends it, keyed by its flight and the cycle that
/// follows; a long gap plays only the last.
#[test]
fn a_missile_scenario_finishes_once_per_cycle() {
    let defines = FlightDefines::for_tests();
    let (mut plan, rigs) = one_run(vec![height(3.0); 4], 1.0, true, false, Explode::Absent);
    plan.advance(&rigs, &defines, Some(BAND), 4.9, &|_| None);
    assert!(plan.missile_explosions(0.0, 4.9).is_empty());
    plan.advance(&rigs, &defines, Some(BAND), 5.0, &|_| None);
    let played = plan.missile_explosions(4.9, 5.0);
    assert_eq!(played.len(), 1);
    assert_eq!((played[0].province, played[0].id), (9, 1));
    assert!(plan.missile_explosions(5.0, 5.0).is_empty());
    plan.advance(&rigs, &defines, Some(BAND), 31.0, &|_| None);
    let played = plan.missile_explosions(5.0, 31.0);
    assert_eq!(played.len(), 1);
    assert_eq!(played[0].id, 6);
}

#[test]
fn a_default_outcome_scenario_without_a_default_state_plays_its_default_state() {
    let descriptors = mesh::parse_descriptors(
        br#"
        entity = { name = missile_1_bombing_entity default_state = idle state = { name = idle animation = idle } }
        entity = { name = fighter_1_patrol_entity state = { name = default animation = default } }
        entity = { name = bare_entity state = { name = launch animation = launch } }
        entity = { name = paired_entity state = { name = pass animation = pass } }
    "#,
        "synthetic.asset",
    )
    .unwrap();
    assert_eq!(
        scenario_state(&descriptors, "missile_1_bombing_entity", 0),
        "idle"
    );
    assert_eq!(
        scenario_state(&descriptors, "fighter_1_patrol_entity", 0),
        "default"
    );
    // Without a declared default state the engine's own default stands.
    assert_eq!(
        scenario_state(&descriptors, "bare_entity", 0),
        mesh::DEFAULT_STATE
    );
    // Outcome states are never replaced.
    assert_eq!(scenario_state(&descriptors, "paired_entity", 3), "fail");
}

/// A scenario rig whose mesh or animation file does not read is refused by
/// name, and the assets still load: the missile rig's mesh file is missing,
/// and the conventional patrol rig's mesh reads but its animation file is
/// missing. Before, either failure aborted the whole load, and with it the
/// document, even with no missile or patrol flight to draw.
#[test]
fn a_scenario_rig_whose_files_do_not_read_is_refused_and_the_assets_load() {
    let root = std::env::temp_dir().join(format!(
        "viewer-aircraft-unreadable-rigs-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let mut mesh = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 1., 0.]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        mesh.extend([b'!', name.len() as u8]);
        mesh.extend(name.as_bytes());
        mesh.push(b'f');
        mesh.extend((values.len() as u32).to_le_bytes());
        mesh.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    mesh.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    for (path, bytes) in [
        (
            "gfx/models/scenarios.gfx",
            br#"
            objectTypes = {
                pdxmesh = {
                    name = "missile_mesh"
                    file = "gfx/models/absent_missile.mesh"
                    animation = { id = "idle" type = "missile_animation" }
                }
                pdxmesh = {
                    name = "patrol_mesh"
                    file = "gfx/models/patrol.mesh"
                    animation = { id = "fly" type = "patrol_animation" }
                }
            }
            "#
            .as_slice(),
        ),
        (
            "gfx/entities/scenarios.asset",
            br#"
            entity = {
                name = "missile_1_bombing_entity"
                pdxmesh = "missile_mesh"
                state = { name = "idle" animation = "idle" }
            }
            entity = {
                name = "fighter_1_patrol_entity"
                pdxmesh = "patrol_mesh"
                state = { name = "default" animation = "fly" }
            }
            animation = { name = "missile_animation" file = "missile.anim" }
            animation = { name = "patrol_animation" file = "absent_patrol.anim" }
            "#
            .as_slice(),
        ),
        ("gfx/models/patrol.mesh", mesh.as_slice()),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let map = MapData {
        width: 1,
        height: 1,
        province_ids: vec![0],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let mut warnings = Vec::new();
    let assets = AircraftAssets::load(
        &layers,
        Decode::default(),
        &map,
        (&Defines::new().unwrap(), &Defines::new().unwrap()),
        &BTreeMap::new(),
        &mut warnings,
    );
    let _ = std::fs::remove_dir_all(&root);
    let assets = assets.unwrap();
    for (entity, file) in [
        (
            "missile_1_bombing_entity",
            "its mesh file gfx/models/absent_missile.mesh",
        ),
        (
            "fighter_1_patrol_entity",
            "its animation file gfx/entities/absent_patrol.anim",
        ),
    ] {
        let refused =
            format!("Aircraft scenario {entity} is refused and not flown: {file} does not read");
        assert_eq!(
            warnings
                .iter()
                .filter(|(kind, line)| {
                    *kind == installed_content_core::DiagnosticKind::Refused && line.starts_with(&refused)
                })
                .count(),
            1,
            "{refused}: {warnings:?}"
        );
    }
    assert!(!assets.rigs.contains_key("missile_1_bombing"));
    assert!(!assets.rigs.contains_key("fighter_1_patrol"));
}

/// The flying mode's crash reads the content's plane ground contact: its height above the surface
/// and how long the explosion plays before the entity goes.
#[test]
fn ground_contact_reads_the_content_defines_and_names_a_missing_one() {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "common/defines/00_graphics.lua",
            b"NDefines = { NAirGfx = { AIRPLANES_GROUND_COLLISION_OFFSET_Y = 0.25, AIRPLANES_GROUND_EXPLOSION_TIME_DELAY = 0.75 } }",
        )
        .unwrap();
    assert_eq!(
        GroundContact::load(&defines).unwrap(),
        GroundContact {
            offset: 0.25,
            explosion: 0.75
        }
    );
    let missing = Defines::new().unwrap();
    missing
        .load(
            "common/defines/00_graphics.lua",
            b"NDefines = { NAirGfx = { AIRPLANES_GROUND_COLLISION_OFFSET_Y = 0.0 } }",
        )
        .unwrap();
    let error = GroundContact::load(&missing).unwrap_err().to_string();
    assert!(
        error.contains("AIRPLANES_GROUND_EXPLOSION_TIME_DELAY"),
        "{error}"
    );
}

#[test]
fn an_explosion_has_played_once_its_own_player_reads_the_delay() {
    let contact = GroundContact {
        offset: 0.0,
        explosion: 4.0,
    };
    // The play position, not the clip's length, meets the delay.
    assert!(!contact.exploded(Some(Some((3.9, 10.0)))));
    assert!(contact.exploded(Some(Some((4.0, 2.0)))));
    // Before its player exists nothing has played; a root that plays no animation reads zero.
    assert!(!contact.exploded(None));
    assert!(!contact.exploded(Some(None)));
    assert!(
        GroundContact {
            explosion: 0.0,
            ..contact
        }
        .exploded(Some(None))
    );
}

/// A chosen aircraft bakes once from the map's own roster, an entity or a bare
/// mesh. Declaration refusals precede file reads; a meshless actor is baked and
/// retained, then refused for Fly. Repeated rejection keeps one named refusal.
/// Catalogue reasons and mesh-file failures stay named and refused.
#[test]
fn a_chosen_aircraft_bakes_lazily_or_is_refused_with_why() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!("viewer-aircraft-player-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut mesh = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 1., 0.]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        mesh.extend([b'!', name.len() as u8]);
        mesh.extend(name.as_bytes());
        mesh.push(b'f');
        mesh.extend((values.len() as u32).to_le_bytes());
        mesh.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    mesh.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    mesh.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    for (path, bytes) in [
        (
            "gfx/models/planes.gfx",
            br#"objectTypes = {
                pdxmesh = { name = "plane_mesh" file = "gfx/models/plane.mesh" }
                pdxmesh = { name = "absent_mesh" file = "gfx/models/absent.mesh" }
            }"#
            .as_slice(),
        ),
        (
            "gfx/entities/planes.asset",
            br#"
            entity = { name = "plane_entity" pdxmesh = "plane_mesh" }
            entity = { name = "ghost_entity" }
            entity = { name = "absent_file_entity" pdxmesh = "absent_mesh" }
            entity = { name = "spinner_entity" pdxmesh = "plane_mesh"
                state = { name = "idle" animation = "spin" animation_speed = 0 } }
            "#
            .as_slice(),
        ),
        ("gfx/models/plane.mesh", mesh.as_slice()),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let map = MapData {
        width: 1,
        height: 1,
        province_ids: vec![0],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let assets = AircraftAssets::load(
        &layers,
        Decode::default(),
        &map,
        (&Defines::new().unwrap(), &Defines::new().unwrap()),
        &BTreeMap::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(assets.library().slot("plane_entity"), None);
    let plane = assets.player("plane_entity").unwrap();
    assert_eq!(plane.name, "plane_entity");
    assert_eq!(assets.library().slot("plane_entity"), Some(plane.slot));
    assert_eq!(assets.player("plane_entity"), Ok(plane.clone()));
    let bare = assets.player("plane_mesh").unwrap();
    assert_ne!(bare.slot, plane.slot);
    assert_eq!(
        assets.player("ghost_entity"),
        Err("it names no pdxmesh".into())
    );
    let ghost_slot = assets.library().slot("ghost_entity").unwrap();
    assert_eq!(
        assets.player("no_such_entity"),
        Err("no descriptor registers it".into())
    );
    assert_eq!(
        assets.player("spinner_entity"),
        Err("state idle has unsupported non-positive animation_speed 0".into())
    );
    let absent = assets.player("absent_file_entity").unwrap_err();
    assert!(
        absent.starts_with(&format!(
            "{NOT_LOADED}: absent_file_entity: pdxmesh absent_mesh (gfx/models/absent.mesh) cannot be loaded: "
        )),
        "{absent}"
    );
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(assets.player("absent_file_entity"), Err(absent));
    let refusals = layers.refused();
    assert_eq!(
        refusals
            .iter()
            .filter(|key| key.as_str() == "model entity ghost_entity")
            .count(),
        1
    );
    assert_eq!(
        assets.player("ghost_entity"),
        Err("it names no pdxmesh".into())
    );
    assert_eq!(assets.library().slot("ghost_entity"), Some(ghost_slot));
    assert_eq!(layers.refused(), refusals);
}

/// A scenario rig the visual mods break (an animation of theirs that does not read) is flown
/// from the recorded layers, named; one neither builds is refused by its stable name for the
/// map's fallback, with the shown stack's reason.
#[test]
fn a_scenario_rig_the_mods_break_is_flown_from_the_recording() {
    let layers = ResolvedLayers::default();
    let mut attempts = Vec::new();
    let mut warnings = Vec::new();
    let flown = shown_or_recorded(
        &layers,
        "fighter_1_patrol_entity",
        true,
        |from_recording, notes| {
            attempts.push(from_recording);
            if from_recording {
                notes.push((
                    installed_content_core::DiagnosticKind::Logged,
                    "recorded rig".into(),
                ));
                Some("recorded")
            } else {
                notes.push((
                    installed_content_core::DiagnosticKind::Refused,
                    "its animation file gfx/entities/patrol.anim does not read".into(),
                ));
                None
            }
        },
        &mut warnings,
    );
    assert_eq!(flown, Some("recorded"));
    assert_eq!(attempts, [false, true]);
    assert_eq!(
        warnings,
        [(
            installed_content_core::DiagnosticKind::Logged,
            "recorded rig".to_owned()
        )]
    );
    assert!(layers.refused().is_empty());
    // Neither stack builds it: refused by name, with the shown stack's reason.
    let mut warnings = Vec::new();
    let flown = shown_or_recorded(
        &layers,
        "fighter_1_patrol_entity",
        true,
        |from_recording, notes| {
            notes.push((
                installed_content_core::DiagnosticKind::Refused,
                format!("refused from the recording: {from_recording}"),
            ));
            None::<&str>
        },
        &mut warnings,
    );
    assert_eq!(flown, None);
    assert_eq!(
        warnings,
        [(
            installed_content_core::DiagnosticKind::Refused,
            "refused from the recording: false".to_owned()
        )]
    );
    assert!(
        layers
            .refused()
            .contains("aircraft scenario fighter_1_patrol_entity")
    );
}
