use content_reader_mesh::mesh::MeshProducts as _;
use super::*;
use crate::equipment_graphics::Names;
use content_reader_layers::ResolvedLayers;
use std::path::{Path, PathBuf};

struct Root(PathBuf);

impl Root {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_aircraft_catalogue_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn folder(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        for (path, text) in files {
            let path = folder.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        folder
    }

    fn archive(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, stored_zip(files)).unwrap();
        path
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stored_zip(files: &[(&str, &str)]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let (mut file, mut central) = (Vec::new(), Vec::new());
    for (name, text) in files {
        let (name, data) = (name.as_bytes(), text.as_bytes());
        let offset = file.len() as u32;
        let mut fields = vec![20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        fields.extend(crc32(data).to_le_bytes());
        fields.extend((data.len() as u32).to_le_bytes());
        fields.extend((data.len() as u32).to_le_bytes());
        fields.extend((name.len() as u16).to_le_bytes());
        file.extend(0x0403_4B50_u32.to_le_bytes());
        file.extend(&fields);
        file.extend([0, 0]);
        file.extend(name);
        file.extend(data);
        central.extend(0x0201_4B50_u32.to_le_bytes());
        central.extend([20, 0]);
        central.extend(&fields);
        central.extend([0; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name);
    }
    let start = file.len() as u32;
    let count = (files.len() as u16).to_le_bytes();
    file.extend(&central);
    file.extend(0x0605_4B50_u32.to_le_bytes());
    file.extend([0, 0, 0, 0, count[0], count[1], count[0], count[1]]);
    file.extend((central.len() as u32).to_le_bytes());
    file.extend(start.to_le_bytes());
    file.extend([0, 0]);
    file
}

const EQUIPMENT: &str = "common/units/equipment/planes.txt";
const MESHES: &str = "gfx/models/planes.gfx";
const ENTITIES: &str = "gfx/entities/planes.asset";
const POOLS: &str = "gfx/interface/equipmentdesigner/graphic_db/00_planes.txt";

fn vanilla(root: &Root) -> PathBuf {
    root.folder(
        "vanilla",
        &[
            (
                EQUIPMENT,
                "equipments = {
                    fighter_equipment = { is_archetype = yes type = fighter sprite = fighter_sprite }
                    fighter_equipment_0 = { archetype = fighter_equipment }
                    fighter_equipment_1 = { archetype = fighter_equipment family = fighter_equipment_0 }
                    infantry_equipment = { is_archetype = yes type = infantry }
                    infantry_equipment_0 = { archetype = infantry_equipment }
                }",
            ),
            (
                MESHES,
                r#"objectTypes = {
                    pdxmesh = { name = "fighter_mesh" file = "gfx/models/fighter.mesh" }
                    pdxmesh = { name = "pool_mesh" file = "gfx/models/pool.mesh" }
                    pdxmesh = { name = "loose_mesh" file = "gfx/models/loose.mesh" }
                    pdxmesh = { name = "rifle_mesh" file = "gfx/models/rifle.mesh" }
                }"#,
            ),
            (
                ENTITIES,
                r#"
                entity = { name = "fighter_equipment_0_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "GER_fighter_equipment_1_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "fighter_sprite_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "light_plane_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "infantry_equipment_0_entity" pdxmesh = "rifle_mesh" }
                entity = { name = "spinner_entity" pdxmesh = "fighter_mesh"
                    state = { name = "idle" animation = "spin" animation_speed = 0 } }
                entity = { name = "GER_ace_plane_entity" pdxmesh = "fighter_mesh" }
                "#,
            ),
            (
                POOLS,
                "default = {
                    fighter_equipment = { pool = { models = { light_plane_entity pool_mesh missing_plane_entity spinner_entity } } }
                    infantry_equipment = { pool = { models = { infantry_equipment_0_entity } } }
                }
                GER = {
                    fighter_equipment_1 = { pool = { models = { GER_ace_plane_entity } } }
                }",
            ),
        ],
    )
}

fn dlc(root: &Root) -> PathBuf {
    root.archive(
        "dlc.zip",
        &[
            (
                ENTITIES,
                r#"
                entity = { name = "fighter_equipment_0_entity" pdxmesh = "pool_mesh" }
                entity = { name = "GER_fighter_equipment_1_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "fighter_sprite_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "light_plane_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "infantry_equipment_0_entity" pdxmesh = "rifle_mesh" }
                entity = { name = "spinner_entity" pdxmesh = "fighter_mesh"
                    state = { name = "idle" animation = "spin" animation_speed = 0 } }
                entity = { name = "GER_ace_plane_entity" pdxmesh = "fighter_mesh" }
                entity = { name = "ghost_plane_entity" }
                entity = { name = "unbound_plane_entity" pdxmesh = "no_such_mesh" }
                "#,
            ),
            (
                "gfx/interface/equipmentdesigner/graphic_db/01_dlc_planes.txt",
                "default = { fighter_equipment = { pool = { models = { ghost_plane_entity unbound_plane_entity } } } }",
            ),
        ],
    )
}

fn modded(root: &Root) -> PathBuf {
    root.folder(
        "mod",
        &[
            (
                "common/units/equipment/mod_planes.txt",
                "equipments = {
                    jet_equipment = { is_archetype = yes type = fighter }
                    jet_equipment_1 = { archetype = jet_equipment }
                }",
            ),
            (
                "gfx/models/mod_planes.gfx",
                r#"objectTypes = { pdxmesh = { name = "jet_mesh" file = "gfx/models/jet.mesh" } }"#,
            ),
            (
                "gfx/entities/zz_mod_planes.asset",
                r#"
                entity = { name = "SOV_jet_equipment_1_entity" pdxmesh = "jet_mesh" }
                entity = { name = "skin_plane_entity" clone = "light_plane_entity" scale = 2 }
                entity = { name = "deep_skin_entity" clone = "skin_plane_entity" }
                entity = { name = "custom_unreferenced_entity" pdxmesh = "jet_mesh" }
                "#,
            ),
            (
                "gfx/interface/equipmentdesigner/graphic_db/02_mod.txt",
                "default = { jet_equipment = { pool = { models = { jet_mesh } } } }",
            ),
        ],
    )
}

fn catalogue(layers: &ResolvedLayers, designs: &[(String, String)]) -> Catalogue {
    with_catalogue_inputs(layers, designs, Catalogue::build)
}

fn with_catalogue_inputs<T>(
    layers: &ResolvedLayers,
    designs: &[(String, String)],
    read: impl FnOnce(&Inputs<'_>) -> T,
) -> T {
    content_reader_script::test_game::enter();
    let roster = layers.descriptor_roster().unwrap();
    let equipment = equipment::Database::load_complete(layers).unwrap();
    let names = Names {
        countries: ["GER", "SOV"].map(str::to_owned).into(),
        continents: BTreeSet::new(),
    };
    let pools = Content::load(layers, &names).unwrap();
    read(&Inputs {
        roster: &roster,
        equipment: &equipment,
        pools: &pools,
        designs,
    })
}

#[test]
fn picker_reads_only_the_bodies_for_its_aircraft() {
    let root = Root::new("picker_bodies");
    let (vanilla, extra) = (vanilla(&root), modded(&root));
    let stack = layers(&[(&vanilla, &[]), (&extra, &[])]);
    with_catalogue_inputs(&stack, &[], |inputs| {
        let mut all_reads = Vec::new();
        let all = Catalogue::build_entries(inputs, true, |name| {
            all_reads.push(name.to_owned());
            refusal(inputs.roster, name)
        });
        let mut picker_reads = Vec::new();
        let picker = Catalogue::build_entries(inputs, false, |name| {
            picker_reads.push(name.to_owned());
            refusal(inputs.roster, name)
        });
        let aircraft: Vec<_> = all.entries.iter().filter(|entry| entry.aircraft()).cloned().collect();

        assert_eq!(all_reads.len(), all.entries.len());
        assert_eq!(picker_reads, aircraft.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>());
        assert!(picker_reads.len() < all_reads.len());
        assert_eq!(picker.entries, aircraft);
        assert_eq!(picker.diagnostics, all.diagnostics);
    });
}

fn entry<'a>(catalogue: &'a Catalogue, name: &str) -> &'a Entry {
    catalogue
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is listed"))
}

fn layers(folders: &[(&Path, &[&str])]) -> ResolvedLayers {
    ResolvedLayers::from_folders(folders).unwrap()
}

#[test]
fn the_catalogue_lists_every_referenced_plane_of_the_whole_stack() {
    let root = Root::new("stack");
    let (vanilla, dlc, modded) = (vanilla(&root), dlc(&root), modded(&root));
    let stack = layers(&[(&vanilla, &[]), (&dlc, &[]), (&modded, &[])]);
    let designs = [
        (
            "fighter_equipment_1".to_owned(),
            "designer_plane_entity".to_owned(),
        ),
        (
            "infantry_equipment_0".to_owned(),
            "custom_unreferenced_entity".to_owned(),
        ),
        ("fighter_equipment_0".to_owned(), String::new()),
        ("removed_equipment".to_owned(), "lost_entity".to_owned()),
    ];
    let catalogue = catalogue(&stack, &designs);
    let aircraft: Vec<&str> = catalogue
        .entries
        .iter()
        .filter(|entry| entry.aircraft())
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        aircraft,
        [
            "GER_ace_plane_entity",
            "GER_fighter_equipment_1_entity",
            "SOV_jet_equipment_1_entity",
            "deep_skin_entity",
            "designer_plane_entity",
            "fighter_equipment_0_entity",
            "fighter_sprite_entity",
            "ghost_plane_entity",
            "jet_mesh",
            "light_plane_entity",
            "missing_plane_entity",
            "pool_mesh",
            "skin_plane_entity",
            "spinner_entity",
            "unbound_plane_entity",
        ]
    );
    assert_eq!(
        entry(&catalogue, "GER_ace_plane_entity").references,
        BTreeSet::from([Reference::Pool {
            bucket: "GER".into(),
            equipment: "fighter_equipment_1".into(),
            file: POOLS.into(),
        }])
    );
    assert_eq!(
        entry(&catalogue, "GER_fighter_equipment_1_entity").references,
        BTreeSet::from([Reference::Ladder {
            key: "fighter_equipment_1".into(),
            prefix: Some("GER".into()),
        }])
    );
    assert!(
        entry(&catalogue, "fighter_sprite_entity")
            .references
            .contains(&Reference::Ladder {
                key: "fighter_sprite".into(),
                prefix: None
            })
    );
    assert_eq!(
        entry(&catalogue, "SOV_jet_equipment_1_entity").references,
        BTreeSet::from([Reference::Ladder {
            key: "jet_equipment_1".into(),
            prefix: Some("SOV".into()),
        }])
    );
    let jet = entry(&catalogue, "jet_mesh");
    assert_eq!((jet.kind, &jet.refusal), (Kind::Mesh, &None));
    assert_eq!(
        entry(&catalogue, "designer_plane_entity").references,
        BTreeSet::from([Reference::Design {
            equipment: "fighter_equipment_1".into()
        }])
    );
    assert!(entry(&catalogue, "designer_plane_entity").kind == Kind::Unregistered);
    assert!(!entry(&catalogue, "custom_unreferenced_entity").aircraft());
    assert!(!entry(&catalogue, "infantry_equipment_0_entity").aircraft());
    assert!(
        catalogue
            .diagnostics
            .iter()
            .any(|line| line.contains("1 recorded design types")
                && line.contains("removed_equipment"))
    );
    assert_eq!(
        entry(&catalogue, "skin_plane_entity").references,
        BTreeSet::from([Reference::CloneOf {
            of: "light_plane_entity".into()
        }])
    );
    assert_eq!(
        entry(&catalogue, "deep_skin_entity").references,
        BTreeSet::from([Reference::CloneOf {
            of: "light_plane_entity".into()
        }])
    );
}

#[test]
fn every_registered_entity_and_bare_mesh_is_listed_with_its_layer_precedence() {
    let root = Root::new("all");
    let (vanilla, dlc, modded) = (vanilla(&root), dlc(&root), modded(&root));
    let stack = layers(&[(&vanilla, &[]), (&dlc, &[]), (&modded, &[])]);
    let catalogue = catalogue(&stack, &[]);
    for (name, kind) in [
        ("custom_unreferenced_entity", Kind::Entity),
        ("infantry_equipment_0_entity", Kind::Entity),
        ("loose_mesh", Kind::Mesh),
        ("rifle_mesh", Kind::Mesh),
        ("pool_mesh", Kind::Mesh),
        ("missing_plane_entity", Kind::Unregistered),
    ] {
        assert_eq!(entry(&catalogue, name).kind, kind, "{name}");
    }
    assert_eq!(entry(&catalogue, "fighter_mesh").kind, Kind::Mesh);
    let roster = stack.descriptor_roster().unwrap();
    assert_eq!(
        roster.descriptor_job().view().entity("fighter_equipment_0_entity").unwrap()
            .mesh
            .as_deref(),
        Some("pool_mesh")
    );
    let without = catalogue_without_dlc(&vanilla, &modded);
    assert!(
        without
            .entries
            .iter()
            .all(|entry| entry.name != "ghost_plane_entity")
    );
    assert!(
        !without
            .entries
            .iter()
            .any(|entry| entry.name == "unbound_plane_entity")
    );
    assert_eq!(roster.clones["skin_plane_entity"], "light_plane_entity");
    assert_eq!(roster.clones["deep_skin_entity"], "skin_plane_entity");
    assert_eq!(
        Catalogue::refusal_counts(&catalogue.entries),
        BTreeMap::from([
            (RefusalKind::Unregistered, 1),
            (RefusalKind::Declaration, 1),
            (RefusalKind::NoGeometry, 2),
        ])
    );
}

fn catalogue_without_dlc(vanilla: &Path, modded: &Path) -> Catalogue {
    let stack = layers(&[(vanilla, &[]), (modded, &[])]);
    catalogue(&stack, &[])
}

#[test]
fn refusals_name_what_cannot_be_flown() {
    let root = Root::new("refusals");
    let (vanilla, dlc, modded) = (vanilla(&root), dlc(&root), modded(&root));
    let stack = layers(&[(&vanilla, &[]), (&dlc, &[]), (&modded, &[])]);
    let catalogue = catalogue(&stack, &[]);
    let refusal = |name: &str| entry(&catalogue, name).refusal.clone();
    assert_eq!(
        refusal("missing_plane_entity"),
        Some(Refusal {
            kind: RefusalKind::Unregistered,
            reason: "no descriptor registers it".into()
        })
    );
    assert_eq!(
        refusal("ghost_plane_entity"),
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason: "it names no pdxmesh".into()
        })
    );
    assert_eq!(
        refusal("unbound_plane_entity"),
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason: "no pdxmesh no_such_mesh is registered".into()
        })
    );
    assert_eq!(
        refusal("spinner_entity"),
        Some(Refusal {
            kind: RefusalKind::Declaration,
            reason: "state idle has unsupported non-positive animation_speed 0".into()
        })
    );
    for flyable in [
        "light_plane_entity",
        "skin_plane_entity",
        "pool_mesh",
        "custom_unreferenced_entity",
    ] {
        assert_eq!(refusal(flyable), None, "{flyable}");
    }
}

#[test]
fn a_name_an_unreadable_descriptor_may_declare_is_refused_and_named() {
    let root = Root::new("unreadable");
    let vanilla = vanilla(&root);
    let broken = root.folder(
        "broken",
        &[(
            "gfx/entities/a_broken.asset",
            "entity = { hidden = { GER_fighter_equipment_0_entity } } }{ = =",
        )],
    );
    let stack = layers(&[(&vanilla, &[]), (&broken, &[])]);
    let catalogue = catalogue(&stack, &[]);
    let hidden = entry(&catalogue, "GER_fighter_equipment_0_entity");
    assert!(hidden.aircraft());
    assert_eq!(hidden.kind, Kind::Unregistered);
    assert_eq!(
        hidden.refusal,
        Some(Refusal {
            kind: RefusalKind::Declaration,
            reason: "it may be declared in gfx/entities/a_broken.asset, which does not parse"
                .into()
        })
    );
    assert!(
        catalogue
            .diagnostics
            .iter()
            .any(|line| line.starts_with("1 descriptor files do not parse"))
    );
}

#[test]
fn a_mod_replace_path_hides_the_planes_below_it() {
    let root = Root::new("replace");
    let (vanilla, modded) = (vanilla(&root), modded(&root));
    let stack = layers(&[(&vanilla, &[]), (&modded, &["gfx/entities"])]);
    let catalogue = catalogue(&stack, &[]);
    assert!(
        catalogue
            .entries
            .iter()
            .all(|entry| entry.name != "GER_fighter_equipment_1_entity")
    );
    assert!(
        catalogue
            .entries
            .iter()
            .all(|entry| entry.name != "skin_plane_entity")
    );
    assert!(
        catalogue
            .diagnostics
            .iter()
            .any(|line| line.contains("finding no entity light_plane_entity to clone yet"))
    );
    assert_eq!(
        entry(&catalogue, "light_plane_entity").kind,
        Kind::Unregistered
    );
}

#[test]
fn the_ladder_needs_a_nonempty_prefix_and_a_whole_key() {
    let keys = BTreeSet::from(["fighter_equipment_1".to_owned(), "equipment_1".to_owned()]);
    assert_eq!(
        ladder_keys("GER_fighter_equipment_1_entity", &keys),
        [
            ("fighter_equipment_1", Some("GER")),
            ("equipment_1", Some("GER_fighter"))
        ]
    );
    assert_eq!(
        ladder_keys("fighter_equipment_1_entity", &keys),
        [
            ("fighter_equipment_1", None),
            ("equipment_1", Some("fighter"))
        ]
    );
    assert_eq!(
        ladder_keys("_fighter_equipment_1_entity", &keys),
        [("equipment_1", Some("_fighter"))]
    );
    let keys = BTreeSet::from(["fighter_equipment_1".to_owned()]);
    assert!(ladder_keys("_fighter_equipment_1_entity", &keys).is_empty());
    assert!(ladder_keys("fighter_equipment_1", &keys).is_empty());
    assert!(ladder_keys("xfighter_equipment_1_entity", &keys).is_empty());
}

#[test]
fn a_meshless_entity_with_a_drawable_attachment_can_be_flown() {
    let root = Root::new("attachments");
    let stack = root.folder(
        "stack",
        &[
            (
                MESHES,
                r#"objectTypes = { pdxmesh = { name = "wing_mesh" file = "gfx/models/wing.mesh" } }"#,
            ),
            (
                ENTITIES,
                r#"
                entity = { name = "wing_entity" pdxmesh = "wing_mesh" }
                entity = { name = "hub_entity" attach = { root = "wing_entity" } }
                entity = { name = "composite_plane_entity" attach = { root = "hub_entity" } }
                entity = { name = "bare_part_plane_entity" attach = { root = "wing_mesh" } }
                entity = { name = "ghost_part_entity" }
                entity = { name = "empty_composite_entity" attach = { root = "ghost_part_entity" } }
                "#,
            ),
        ],
    );
    let roster = layers(&[(&stack, &[])]).descriptor_roster().unwrap();
    for flyable in ["composite_plane_entity", "hub_entity", "wing_mesh"] {
        assert_eq!(refusal(&roster, flyable), None, "{flyable}");
    }
    assert_eq!(
        refusal(&roster, "bare_part_plane_entity"),
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason: "it names no pdxmesh, and nothing attached to it draws one".into()
        })
    );
    assert_eq!(
        refusal(&roster, "empty_composite_entity"),
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason: "it names no pdxmesh, and nothing attached to it draws one".into()
        })
    );
    assert_eq!(
        refusal(&roster, "ghost_part_entity"),
        Some(Refusal {
            kind: RefusalKind::NoGeometry,
            reason: "it names no pdxmesh".into()
        })
    );
}
