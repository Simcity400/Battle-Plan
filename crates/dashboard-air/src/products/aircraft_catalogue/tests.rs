use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::{rows::LexicalTokenRow, rows::TableRows, rows::VariantDefinitionRow};
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn frame(branch: u64, hour: i32, read: bool, variants: &[(u64, u32, &str)]) -> DecodedFrame {
    spelled(
        branch,
        hour,
        read,
        &[(7, "fighter_equipment_1"), (8, "infantry_equipment_0")],
        variants,
    )
}

fn spelled(
    branch: u64,
    hour: i32,
    read: bool,
    spellings: &[(u32, &str)],
    variants: &[(u64, u32, &str)],
) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        spellings
            .iter()
            .map(|&(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    ));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            variants
                .iter()
                .map(|&(id, equipment_token, model)| VariantDefinitionRow {
                    id,
                    equipment_token,
                    explicit_entity: model.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    if read {
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    }
    frame
}

fn recording(branches: Vec<Vec<DecodedFrame>>) -> Source {
    let branches: Vec<_> = branches
        .into_iter()
        .enumerate()
        .map(|(at, frames)| records(at as u64 + 1, frames))
        .collect();
    Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "aircraft".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &branches).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn recorded_designs_are_every_saved_model_of_every_branch_read() {
    let source = recording(vec![
        vec![
            frame(
                1,
                100,
                true,
                &[
                    (1, 7, "designer_plane_entity"),
                    (2, 8, ""),
                    (3, 8, "rifle_entity"),
                ],
            ),
            frame(1, 101, false, &[(1, 7, "carried_entity")]),
        ],
        vec![frame(
            2,
            200,
            true,
            &[
                (1, 7, "designer_plane_entity"),
                (4, 7, "second_branch_entity"),
            ],
        )],
    ]);
    let designs = recorded_designs(&source, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        designs,
        [
            ("fighter_equipment_1", "designer_plane_entity"),
            ("fighter_equipment_1", "second_branch_entity"),
            ("infantry_equipment_0", "rifle_entity"),
        ]
        .map(|(kind, model)| (kind.to_owned(), model.to_owned()))
    );
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        recorded_designs(&source, &cancelled).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn recorded_designs_are_told_apart_by_their_spelling_not_their_session_numbers() {
    let source = recording(vec![
        vec![spelled(
            1,
            100,
            true,
            &[(7, "infantry_equipment_0")],
            &[(1, 7, "shared_entity")],
        )],
        vec![spelled(
            2,
            200,
            true,
            &[(7, "fighter_equipment_1")],
            &[(1, 7, "shared_entity")],
        )],
    ]);
    assert_eq!(
        recorded_designs(&source, &AtomicBool::new(false)).unwrap(),
        [
            ("fighter_equipment_1", "shared_entity"),
            ("infantry_equipment_0", "shared_entity"),
        ]
        .map(|(kind, model)| (kind.to_owned(), model.to_owned()))
    );
}

const PLANES: &[(&str, &str)] = &[
    (
        "map/default.map",
        "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt",
    ),
    ("map/continent.txt", "continents={europe}"),
    (
        "common/units/equipment/planes.txt",
        "equipments = {
            fighter_equipment = { is_archetype = yes type = fighter }
            fighter_equipment_1 = { archetype = fighter_equipment }
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_0 = { archetype = infantry_equipment }
        }",
    ),
    (
        "gfx/models/planes.gfx",
        r#"objectTypes = {
            pdxmesh = { name = "fighter_mesh" file = "gfx/models/fighter.mesh" }
            pdxmesh = { name = "bare_mesh" file = "gfx/models/bare.mesh" }
        }"#,
    ),
    (
        "gfx/entities/planes.asset",
        r#"
        entity = { name = "ENG_fighter_equipment_1_entity" pdxmesh = "fighter_mesh" }
        entity = { name = "light_plane_entity" pdxmesh = "fighter_mesh" }
        entity = { name = "ghost_plane_entity" }
        entity = { name = "rifle_entity" pdxmesh = "fighter_mesh" }
        "#,
    ),
    (
        "gfx/interface/equipmentdesigner/graphic_db/00_planes.txt",
        "default = { fighter_equipment = { pool = { models = { light_plane_entity ghost_plane_entity missing_plane_entity } } } }",
    ),
    (
        "localisation/planes_l_english.yml",
        "\u{feff}l_english:\n ENG_fighter_equipment_1:0 \"Spitfire\"\n fighter_equipment_1:0 \"Fighter I\"\n fighter_equipment:0 \"Fighter\"\n",
    ),
];

fn built(designs: &[(String, String)]) -> (Built, impl Drop) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, folder) = dashboard_base::catalogue::tests::folder_catalogue(PLANES);
    (Built::new(&Arc::new(catalogue), designs, &BTreeMap::new()), folder)
}

fn page(
    built: &Built,
    query: &str,
    first: u32,
    count: u32,
    origin: Option<AircraftOriginFilter>,
) -> AircraftCatalogueReply {
    built.page(&AircraftCatalogueParams {
        query: query.into(),
        first,
        count,
        origin,
    })
}

fn names(reply: &AircraftCatalogueReply) -> Vec<&str> {
    reply
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect()
}

#[test]
fn pages_list_flyable_aircraft_first_alphabetically_with_their_titles_and_counted_refusals() {
    let designs = [(
        "fighter_equipment_1".to_owned(),
        "designer_plane_entity".to_owned(),
    )];
    let (built, _folder) = built(&designs);
    let reply = page(&built, "", 0, 100, None);
    assert_eq!(
        names(&reply),
        [
            "light_plane_entity",
            "ENG_fighter_equipment_1_entity",
            "ghost_plane_entity",
            "missing_plane_entity",
            "designer_plane_entity",
        ],
        "flyable then refused, each by the first title without case, then by name"
    );
    assert_eq!((reply.matching, reply.first), (5, 0));
    assert_eq!(reply.entries[0].titles, ["Fighter"]);
    assert_eq!(reply.entries[1].titles, ["Spitfire"]);
    assert_eq!(
        reply.entries[4].references,
        [AircraftReference::Design {
            equipment: "fighter_equipment_1".into()
        }]
    );
    assert_eq!(reply.entries[4].titles, ["Fighter I"]);
    assert_eq!(
        reply.refusals,
        [
            AircraftRefusalCount {
                kind: AircraftRefusalKind::Unregistered,
                label: "not registered by any descriptor".into(),
                count: 2,
            },
            AircraftRefusalCount {
                kind: AircraftRefusalKind::NoGeometry,
                label: "no geometry to draw".into(),
                count: 1,
            },
        ]
    );
    assert_eq!(
        reply.entries[2].refusal,
        Some(AircraftRefusal {
            kind: AircraftRefusalKind::NoGeometry,
            reason: "it names no pdxmesh".into()
        })
    );
    let folder_mod = AircraftOrigin::Mod {
        id: "test0".into(),
        name: "test0".into(),
    };
    assert_eq!(reply.entries[2].origin, Some(folder_mod.clone()));
    assert_eq!(reply.entries[4].origin, None, "no descriptor registers it");
    assert_eq!(
        reply.origins,
        [AircraftOriginCount {
            origin: folder_mod,
            count: 3
        }]
    );
    assert_eq!(
        page(&built, "designer", 0, 100, None).refusals,
        [AircraftRefusalCount {
            kind: AircraftRefusalKind::Unregistered,
            label: "not registered by any descriptor".into(),
            count: 1,
        }],
        "refusals count only what matches"
    );
}

#[test]
fn queries_match_names_titles_and_references_without_case_and_pages_slice_them() {
    let (built, _folder) = built(&[]);
    assert_eq!(
        names(&page(&built, "SPIT", 0, 10, None)),
        ["ENG_fighter_equipment_1_entity"]
    );
    assert_eq!(
        names(&page(&built, " fighter_equipment ", 0, 10, None)),
        [
            "light_plane_entity",
            "ENG_fighter_equipment_1_entity",
            "ghost_plane_entity",
            "missing_plane_entity"
        ]
    );
    assert_eq!(
        names(&page(&built, "Ghost plane", 0, 10, None)),
        ["ghost_plane_entity"],
        "the readable label made from a name finds its model"
    );
    assert_eq!(
        names(&page(&built, "eng fighter equipment 1", 0, 10, None)),
        ["ENG_fighter_equipment_1_entity"]
    );
    let all = page(&built, "", 0, 100, None);
    assert!(
        !all.entries.iter().any(|entry| entry.name == "rifle_entity" || entry.name == "bare_mesh"),
        "only aircraft are listed"
    );
    let whole = names(&all).join(" ");
    let mut paged = Vec::new();
    for first in (0..all.matching).step_by(3) {
        let reply = page(&built, "", first, 3, None);
        assert_eq!((reply.first, reply.matching), (first, all.matching));
        paged.extend(reply.entries.into_iter().map(|entry| entry.name));
    }
    assert_eq!(paged.join(" "), whole);
    assert!(page(&built, "", all.matching, 3, None).entries.is_empty());
    assert_eq!(page(&built, "no such plane", 0, 3, None).matching, 0);
}

fn base_planes() -> Vec<(&'static str, &'static str)> {
    vec![
        PLANES[0],
        PLANES[1],
        PLANES[2],
        PLANES[6],
        (
            "gfx/models/planes.gfx",
            r#"objectTypes = {
                pdxmesh = { name = "fighter_mesh" file = "gfx/models/fighter.mesh" }
                pdxmesh = { name = "bomber_mesh" file = "gfx/models/bomber.mesh" }
            }"#,
        ),
        ("gfx/models/fighter.mesh", "mesh"),
        ("gfx/models/bomber.mesh", "mesh"),
        (
            "gfx/entities/planes.asset",
            r#"
            entity = { name = "vanilla_plane_entity" pdxmesh = "fighter_mesh" }
            entity = { name = "reskinned_plane_entity" pdxmesh = "bomber_mesh" }
            "#,
        ),
    ]
}

#[test]
fn origins_name_the_mod_or_dlc_supplying_a_declaration_or_its_geometry() {
    let folder = |name: &str, base: bool, files: &[(&str, &str)]| {
        let root = std::env::temp_dir().join(format!(
            "battleplan_aircraft_origins_{}_{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        if base {
            dashboard_base::catalogue::texture_tests::write_sprite_program(&root);
        }
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        dashboard_base::catalogue::tests::Folder(root)
    };
    let vanilla = folder("vanilla", true, &base_planes());
    let dlc = folder(
        "dlc",
        false,
        &[(
            "gfx/entities/dlc_planes.asset",
            r#"entity = { name = "dlc_plane_entity" pdxmesh = "fighter_mesh" }"#,
        )],
    );
    let modded = folder(
        "mod",
        false,
        &[
            (
                "gfx/entities/mod_planes.asset",
                r#"entity = { name = "mod_plane_entity" pdxmesh = "fighter_mesh" }"#,
            ),
            ("gfx/models/bomber.mesh", "reskinned mesh"),
        ],
    );
    let mut layers = content_reader_layers::ResolvedLayers::from_folders(&[
        (&vanilla.0, &[][..]),
        (&dlc.0, &[][..]),
        (&modded.0, &[][..]),
    ])
    .unwrap();
    layers.layers[0].role = LayerRole::Vanilla;
    layers.layers[1].role = LayerRole::Dlc;
    let catalogue = Arc::new(dashboard_base::catalogue::tests::layers_catalogue(layers));
    let designs = [
        "vanilla_plane_entity",
        "reskinned_plane_entity",
        "dlc_plane_entity",
        "mod_plane_entity",
        "missing_plane_entity",
    ]
    .map(|model| ("fighter_equipment_1".to_owned(), model.to_owned()));
    let mods = BTreeMap::from([("test2".to_owned(), "Modern Wings".to_owned())]);
    let built = Built::new(&catalogue, &designs, &mods);

    let game = AircraftOrigin::Game { dlc: None };
    let modern = AircraftOrigin::Mod {
        id: "test2".into(),
        name: "Modern Wings".into(),
    };
    let all = page(&built, "", 0, 100, None);
    let origin = |name: &str| {
        all.entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .origin
            .clone()
    };
    assert_eq!(origin("vanilla_plane_entity"), Some(game.clone()));
    assert_eq!(
        origin("dlc_plane_entity"),
        Some(AircraftOrigin::Game {
            dlc: Some("test1".into())
        }),
        "a DLC counts as the base game, named by its id when its name is not given"
    );
    assert_eq!(origin("mod_plane_entity"), Some(modern.clone()));
    assert_eq!(
        origin("reskinned_plane_entity"),
        Some(modern.clone()),
        "a base-game declaration drawn from a mod's geometry is the mod's"
    );
    assert_eq!(origin("missing_plane_entity"), None);
    assert_eq!(
        all.origins,
        [
            AircraftOriginCount {
                origin: game.clone(),
                count: 2
            },
            AircraftOriginCount {
                origin: modern.clone(),
                count: 2
            },
        ]
    );
    let vanilla_only = page(&built, "", 0, 100, Some(AircraftOriginFilter::Game));
    assert_eq!(
        names(&vanilla_only),
        ["dlc_plane_entity", "vanilla_plane_entity"]
    );
    assert_eq!(
        (vanilla_only.matching, vanilla_only.found),
        (2, 5),
        "found counts every origin, the unregistered model's none included"
    );
    assert_eq!(
        names(&page(
            &built,
            "",
            0,
            100,
            Some(AircraftOriginFilter::Mod("test2".into()))
        )),
        ["mod_plane_entity", "reskinned_plane_entity"]
    );
    let dlc_as_mod = page(
        &built,
        "",
        0,
        100,
        Some(AircraftOriginFilter::Mod("test1".into())),
    );
    assert_eq!(
        (dlc_as_mod.matching, dlc_as_mod.origins.len()),
        (0, 2),
        "a DLC is no origin of its own"
    );
    assert_eq!(
        page(&built, "reskinned", 0, 100, None)
            .origins
            .iter()
            .map(|origin| origin.count)
            .collect::<Vec<_>>(),
        [0, 1],
        "every origin stays listed, counting what matches the query"
    );
}

#[test]
fn a_visual_mod_reskinning_a_plane_names_it_as_the_viewer_draws_it() {
    use dashboard_base::test_fixtures::visual_mods::{Machine, select};
    let machine = Machine::new("aircraft_origins");
    for (path, text) in base_planes() {
        machine.write(path, text.as_bytes());
    }
    machine.write_mod("Wings", "gfx/models/bomber.mesh", b"reskinned mesh");
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["Wings"]);
    let catalogue = dashboard_base::catalogue::catalogue(&source, &options).unwrap();
    let designs = ["vanilla_plane_entity", "reskinned_plane_entity"]
        .map(|model| ("fighter_equipment_1".to_owned(), model.to_owned()));
    let reply = page(&Built::new(&catalogue, &designs, &BTreeMap::new()), "", 0, 100, None);
    let origin = |name: &str| {
        reply
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .origin
            .clone()
    };
    assert_eq!(origin("vanilla_plane_entity"), Some(AircraftOrigin::Game { dlc: None }));
    assert!(
        matches!(origin("reskinned_plane_entity"), Some(AircraftOrigin::Mod { name, .. }) if name == "Wings"),
        "the shown visual mod draws this plane: {:?}",
        origin("reskinned_plane_entity")
    );
}

#[test]
fn a_plane_a_visual_mod_breaks_is_flown_from_the_recording_as_the_viewer_does() {
    use dashboard_base::test_fixtures::visual_mods::{Machine, select};
    let machine = Machine::new("aircraft_recorded_fallback");
    for (path, text) in base_planes() {
        machine.write(path, text.as_bytes());
    }
    machine.write_mod(
        "Broken",
        "gfx/entities/planes.asset",
        br#"
        entity = { name = "vanilla_plane_entity" pdxmesh = "fighter_mesh" state = { name = idle animation_speed = 0 } }
        entity = { name = "reskinned_plane_entity" }
        entity = { name = "mod_plane_entity" pdxmesh = "fighter_mesh" }
        "#,
    );
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["Broken"]);
    let catalogue = dashboard_base::catalogue::catalogue(&source, &options).unwrap();
    let designs = ["vanilla_plane_entity", "reskinned_plane_entity", "mod_plane_entity"]
        .map(|model| ("fighter_equipment_1".to_owned(), model.to_owned()));
    let reply = page(&Built::new(&catalogue, &designs, &BTreeMap::new()), "", 0, 100, None);
    let entry = |name: &str| reply.entries.iter().find(|entry| entry.name == name).unwrap();
    let broken = |model: &str| matches!(&entry(model).origin, Some(AircraftOrigin::Mod { name, .. }) if name == "Broken");
    assert_eq!(
        (&entry("vanilla_plane_entity").refusal, &entry("vanilla_plane_entity").origin),
        (&None, &Some(AircraftOrigin::Game { dlc: None })),
        "the viewer flies the recorded declaration the mod's refused one shadows"
    );
    assert_eq!(
        entry("reskinned_plane_entity").refusal.as_ref().map(|refusal| refusal.kind),
        Some(AircraftRefusalKind::NoGeometry),
        "the viewer loads the mod's meshless entity and refuses to fly it, so no recorded fallback applies"
    );
    assert!(broken("reskinned_plane_entity"), "{:?}", entry("reskinned_plane_entity").origin);
    assert!(entry("mod_plane_entity").refusal.is_none() && broken("mod_plane_entity"));
    assert_eq!(
        reply.refusals.iter().map(|refusal| (refusal.kind, refusal.count)).collect::<Vec<_>>(),
        [(AircraftRefusalKind::NoGeometry, 1)]
    );
}

#[test]
fn an_empty_page_is_refused_before_loading() {
    let params = AircraftCatalogueParams {
        query: String::new(),
        first: 0,
        count: 0,
        origin: None,
    };
    assert!(AircraftCatalogue::validate(&params).is_err());
    assert!(AircraftCatalogue::validate(&AircraftCatalogueParams { count: 1, ..params }).is_ok());
}
