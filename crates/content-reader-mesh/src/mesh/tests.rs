use super::descriptors::last_text;
use super::*;

#[test]
fn aliases_borrow_the_roster_and_override_only_the_named_entities() {
    let descriptors = parse_descriptors(
        br#"entity = { name = base default_state = idle }
            entity = { name = child clone = base }"#,
        "gfx/entities/aliases.asset",
    )
    .unwrap();
    let mut alias = descriptors.entities["base"].clone();
    alias.default_state = Some("damaged".into());
    let aliases = BTreeMap::from([("base".into(), alias)]);
    let view = DescriptorView::from(&descriptors).with_aliases(&aliases);

    assert!(std::ptr::eq(view.entity("child").unwrap(), &raw const descriptors.entities["child"]));
    assert!(std::ptr::eq(view.entity("base").unwrap(), &raw const aliases["base"]));
    assert_eq!(view.inherit("child").unwrap().default_state, Some("damaged"));
    assert_eq!(descriptors.inherit("child").unwrap().default_state, Some("idle"));
}

#[test]
fn entity_boolean_reads_keep_defaults_and_previous_assignments() {
    let descriptors = parse_descriptors(
        br#"entity = { name = e
            get_state_from_parent = yes get_state_from_parent = YES
            state = { name = idle looping = no looping = YES
                event = { keep_particle = no keep_particle = "yes" trigger_once = yes trigger_once = NO }
            }
        }
        entity = { name = defaults get_state_from_parent = YES state = { name = idle looping = YES } }
        "#,
        "gfx/entities/bools.asset",
    )
    .unwrap();
    let entity = descriptors.inherit("e").unwrap();
    assert_eq!(entity.get_state_from_parent, Some(true));
    let state = entity.states["idle"][0];
    assert!(!state.looping);
    assert!(state.events[0].keep_particle);
    assert!(state.events[0].trigger_once);
    let defaults = descriptors.inherit("defaults").unwrap();
    assert_eq!(defaults.get_state_from_parent, Some(false));
    assert!(defaults.states["idle"][0].looping);
}

#[test]
fn entity_shader_data_retains_allocation_defaults_and_authored_fields() {
    let source = parse_descriptors(br#"
        entity = { name = "base" game_data = { atlas_size = { 6 2 } texture_anim_speed = 0.5 } }
        entity = { name = "clone" clone = "base" }
        entity = { name = "override" clone = "base" game_data = { texture_anim_speed = 0.75 } }
        entity = { name = "empty" game_data = {} }
        entity = { name = "partial" game_data = { atlas_size = { x = 4 } } game_data = { texture_anim_speed = 2 } }
        entity = { name = "bad" game_data = { atlas_size = { y = 2 x = 3 } } }
    "#, "gfx/test.asset").unwrap();
    assert_eq!(
        source
            .inherit("clone")
            .unwrap()
            .game_data
            .unwrap()
            .as_ref()
            .unwrap(),
        &EntityGameData {
            texture_anim_speed: 0.5,
            atlas_size: [6, 2]
        }
    );
    assert_eq!(
        source
            .inherit("override")
            .unwrap()
            .game_data
            .unwrap()
            .as_ref()
            .unwrap(),
        &EntityGameData {
            texture_anim_speed: 0.75,
            atlas_size: [1, 1]
        }
    );
    assert_eq!(source.entities["empty"].game_data, None);
    assert_eq!(
        source.entities["partial"].game_data,
        Some(Ok(EntityGameData {
            texture_anim_speed: 2.,
            atlas_size: [4, 1]
        }))
    );
    assert!(source.entities["bad"].game_data.as_ref().unwrap().is_err());
}

#[test]
fn entity_numeric_fields_follow_the_crt_prefix_scan() {
    let descriptors = parse_descriptors(
        br#"entity = { name = e scale = "1.25tail"
            game_data = { texture_anim_speed = "0.5tail" atlas_size = { "6tail" "2tail" } }
        }"#,
        "gfx/test.asset",
    )
    .unwrap();
    let entity = descriptors.inherit("e").unwrap();
    assert_eq!(entity.scale, Some(1.25));
    assert_eq!(
        entity.game_data.unwrap().as_ref().unwrap(),
        &EntityGameData {
            texture_anim_speed: 0.5,
            atlas_size: [6, 2],
        }
    );
}

fn skeleton_root(skeletons: Vec<Vec<Node>>) -> Node {
    Node {
        children: vec![Node {
            name: "object".into(),
            children: skeletons
                .into_iter()
                .map(|children| Node {
                    children: vec![Node {
                        name: "skeleton".into(),
                        children,
                        ..Node::default()
                    }],
                    ..Node::default()
                })
                .collect(),
            ..Node::default()
        }],
        ..Node::default()
    }
}

#[test]
fn skeleton_keeps_source_order_and_first_nonempty_allocation() {
    let joint = |name: &str, index, parent| Node {
        name: name.into(),
        properties: vec![
            ("ix".into(), Value::Ints(vec![index])),
            ("pa".into(), Value::Ints(vec![parent])),
            (
                "tx".into(),
                Value::Floats((1..=12).map(|v| v as f32).collect()),
            ),
        ],
        ..Node::default()
    };
    let root = skeleton_root(vec![
        vec![],
        vec![joint("z", 9, 258), joint("a", 0, -1)],
        vec![joint("ignored", 1, 0)],
    ]);
    let result = native_skeleton(&root, 0.5).unwrap();
    assert_eq!(
        result
            .iter()
            .map(|j| (j.name.as_str(), j.parent))
            .collect::<Vec<_>>(),
        [("z", 2), ("a", 255)]
    );
    assert_eq!(
        result[0].inverse_bind,
        [1., 2., 3., 4., 5., 6., 7., 8., 9., 5., 5.5, 6.]
    );
    let root = skeleton_root(vec![
        vec![Node {
            name: "unfilled".into(),
            ..Node::default()
        }],
        vec![joint("unused", 0, -1)],
    ]);
    assert_eq!(
        native_skeleton(&root, 2.).unwrap(),
        [NativeJoint {
            name: "unfilled".into(),
            parent: 0,
            inverse_bind: [0.; 12]
        }]
    );
}

#[test]
fn the_decoded_skeleton_places_joints_as_the_importer_does() {
    let tx = |x: f32| [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -x, 0.0, 0.0];
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "lod0")
        .node(3, "skeleton")
        .node(2, "lod1")
        .node(3, "mesh")
        .floats("p", &[2.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0])
        .ints("tri", &[])
        .node(3, "skeleton")
        .node(4, "hand")
        .ints("ix", &[i32::MAX])
        .ints("pa", &[257])
        .floats("tx", &tx(2.0))
        .node(4, "root")
        .ints("ix", &[0])
        .ints("pa", &[-1])
        .floats("tx", &tx(0.0))
        .node(4, "bare")
        .node(2, "lod2")
        .node(3, "skeleton")
        .node(4, "ignored")
        .ints("ix", &[0])
        .floats("tx", &tx(9.0))
        .bytes;
    let mesh = read(&bytes).unwrap();
    let placed: Vec<_> = mesh
        .skeleton
        .iter()
        .map(|joint| (joint.name.as_str(), joint.index, joint.parent))
        .collect();
    assert_eq!(
        placed,
        [
            ("hand", 0, Some(1)),
            ("root", 1, None),
            ("bare", 2, Some(0))
        ]
    );
    let native = native_skeleton(&parse(&bytes).unwrap(), 1.0).unwrap();
    assert_eq!(
        native.iter().map(|joint| joint.parent).collect::<Vec<_>>(),
        [1, 255, 0]
    );
    let animation = read_animation(
        &Builder::new()
            .node(1, "info")
            .ints("sa", &[1])
            .node(2, "hand")
            .strings("sa", &[""])
            .floats("t", &[2.0, 1.0, 0.0])
            .floats("q", &[0.0, 0.0, 0.0, 1.0])
            .floats("s", &[1.0])
            .node(2, "root")
            .strings("sa", &[""])
            .floats("t", &[0.0, 0.0, 0.0])
            .floats("q", &[0.0, 0.0, 0.0, 1.0])
            .floats("s", &[1.0])
            .bytes,
    )
    .unwrap();
    let palette = mesh.palette(&animation, FIRST_FRAME);
    assert_eq!(palette.len(), 3);
    assert_eq!(palette[0].transform_point([2.0, 0.0, 0.0]), [2.0, 1.0, 0.0]);
    assert_eq!(palette[1], Transform::IDENTITY);
    assert_eq!(
        mesh.sampled_pose(&animation, &animation.sample(0.0, false))
            .len(),
        3
    );
}

#[test]
fn skeleton_names_and_missing_parent_follow_byte_fields() {
    let mut node = Node {
        name: "é".repeat(35),
        properties: vec![
            ("ix".into(), Value::Ints(vec![71])),
            ("tx".into(), Value::Floats(vec![1.; 12])),
        ],
        ..Node::default()
    };
    let result = native_skeleton(&skeleton_root(vec![vec![node.clone()]]), 2.).unwrap();
    assert_eq!(result[0].name, "é".repeat(31));
    assert_eq!(result[0].parent, 255);
    node.properties[1].1 = Value::Floats(vec![1.; 11]);
    assert!(
        native_skeleton(&skeleton_root(vec![vec![node]]), 1.)
            .unwrap_err()
            .to_string()
            .contains("fewer than12 tx floats")
    );
}

#[test]
fn entity_locators_keep_their_position_and_parent_joint() {
    let parsed = parse_descriptors(
        br#"entity = { name = "e" pdxmesh = "m"
            locator = { name = "a" position = { 1 2 3 } }
            locator = { name = "b" position = { 0 1 0 } scale = 2 parent_joint = "hand" } }"#,
        "gfx/entities/x.asset",
    )
    .unwrap();
    let locators = &parsed.entities["e"].locators;
    assert_eq!(locators[0].0, "a");
    assert_eq!(locators[0].1.transform.translation, [1.0, 2.0, 3.0]);
    assert_eq!(locators[0].1.parent_joint, None);
    assert_eq!(locators[1].1.parent_joint.as_deref(), Some("hand"));
    assert!((locators[0].1.transform.uniform_scale() - 1.0).abs() < 1e-6);
    assert!((locators[1].1.transform.uniform_scale() - 2.0).abs() < 1e-6);
}

#[test]
fn texture_lookup_is_byte_exact_by_basename_and_keeps_the_first_file() {
    let index = TextureIndex::new(
        [
            "gfx/models/buildings/a_diffuse.dds",
            "gfx/models/nospec.dds",
            "gfx/models/x/shared.dds",
            "gfx/models/y/shared.dds",
            "gfx/models/upper.DDS",
        ]
        .map(content_reader_layers::script_files::ScriptPath::from),
    );
    let resolve = |authored| index.resolve("gfx/models/units", authored).unwrap();
    assert_eq!(resolve("nospec.dds"), "gfx/models/nospec.dds");
    assert_eq!(
        resolve("a_diffuse.tga"),
        "gfx/models/buildings/a_diffuse.dds"
    );
    assert_eq!(resolve("Nospec.dds"), "gfx/models/units/Nospec.dds");
    assert_eq!(resolve("upper.DDS"), "gfx/models/units/upper.DDS");
    assert_eq!(resolve("shared.dds"), "gfx/models/x/shared.dds");
    let listed = [
        "gfx/models/a/deep/t.dds",
        "gfx/models/a/t.dds",
        "gfx/models/b.dds",
    ]
    .map(content_reader_layers::script_files::ScriptPath::from);
    let order = content_reader_defines::defines::load_order("gfx/models/", "", &listed);
    assert_eq!(
        TextureIndex::new(order).resolve("", "t.dds").unwrap(),
        "gfx/models/a/t.dds"
    );
    let index = TextureIndex::new([
        content_reader_layers::script_files::ScriptPath::new(b"gfx/models/\x80/odd.dds".to_vec()),
        content_reader_layers::script_files::ScriptPath::from("gfx/models/z/odd.dds"),
    ]);
    let refused = index.resolve("", "odd.dds").unwrap_err().to_string();
    assert!(refused.contains("texture odd.dds"), "{refused}");
}

#[test]
fn an_animation_in_a_descriptor_with_a_raw_file_name_registers() {
    use content_reader_layers::test_support::{Record, zip_records};
    let root = std::env::temp_dir().join(format!(
        "content-reader-roster-raw-name-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let archive = root.join("raw.zip");
    std::fs::write(
        &archive,
        zip_records(
            &[],
            &[Record {
                name: b"gfx/models/\x80.asset",
                host: 0,
                flags: 0,
                deflate: false,
                data: br#"animation = { name = "raw_anim" file = "raw.anim" }"#,
            }],
        ),
    )
    .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&archive, &[][..])]).unwrap();
    let roster = Roster::read(&layers).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        roster.descriptors.animations["raw_anim"].file,
        "gfx/models/raw.anim"
    );
}

pub(super) fn roster_of(name: &str, base: &[(&str, &[u8])], overlay: &[(&str, &[u8])]) -> Roster {
    let root = std::env::temp_dir().join(format!("content-reader-roster-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (under, over) = (root.join("base"), root.join("over"));
    for (dir, files) in [(&under, base), (&over, overlay)] {
        std::fs::create_dir_all(dir).unwrap();
        for (path, bytes) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }
    let layers =
        content_reader_layers::ResolvedLayers::from_folders(&[(&under, &[][..]), (&over, &[][..])]).unwrap();
    let roster = Roster::read(&layers).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    roster
}

#[test]
fn the_roster_registers_entities_in_declaration_order() {
    let roster = roster_of(
        "entities",
        &[
            (
                "gfx/models/meshes.gfx",
                b"objectTypes = { pdxmesh = { name = f file = f.mesh } pdxmesh = { name = m file = m.mesh }
                    pdxmesh = { name = n file = n.mesh } pdxmesh = { name = o file = o.mesh } }",
            ),
            (
                "gfx/entities/a.asset",
                br#"entity = { name = "base" pdxmesh = "m" scale = 2 }
                    entity = { name = "early" clone = "late" }
                    entity = { name = "after_first" clone = "first" }
                    Entity = { name = "v" pdxmesh = "m" version = 2 }
                    entity = { name = "child" clone = "base" attach = { gun = "rifle" } }"#,
            ),
            (
                "gfx/entities/b.asset",
                br#"entity = { name = "late" pdxmesh = "m" }
                    entity = { name = "v" pdxmesh = "n" version = 1 }
                    entity = { name = "base" pdxmesh = "n" }
                    entity = { name = "cross" clone = "child" pdxmesh = "o" }"#,
            ),
            (
                "gfx/entities/sub/c.asset",
                br#"entity = { name = "v" pdxmesh = "o" version = 2 }
                    entity = { name = "vc" clone = "v" version = 1 }
                    entity = { name = "vc" pdxmesh = "m" version = 1 }
                    "entity" = { name = "quoted" pdxmesh = "m" }"#,
            ),
        ],
        &[(
            "gfx/entities/0.asset",
            br#"entity = { name = "first" pdxmesh = "f" }"#,
        )],
    );
    let job = roster.descriptor_job();
    let entities = job.view();
    let mesh = |name: &str| entities.entity(name).unwrap().mesh.as_deref();
    assert!(!entities.has_entity("early"));
    assert_eq!(roster.logged.len(), 1, "{:?}", roster.logged);
    assert!(roster.logged[0].starts_with("early in gfx/entities/a.asset"));
    assert_eq!(mesh("after_first"), Some("f"));
    assert_eq!(mesh("base"), Some("n"));
    assert_eq!(entities.entity("base").unwrap().scale, None);
    assert_eq!(mesh("child"), Some("m"));
    assert_eq!(entities.entity("child").unwrap().scale, Some(2.0));
    assert_eq!(entities.entity("child").unwrap().clone, None);
    assert_eq!(mesh("cross"), Some("o"));
    assert_eq!(entities.entity("cross").unwrap().scale, Some(2.0));
    assert_eq!(entities.entity("cross").unwrap().attachments.len(), 1);
    assert_eq!(mesh("v"), Some("o"));
    assert_eq!(mesh("vc"), Some("o"));
    assert!(!entities.has_entity("quoted"));
    assert!(roster.unreadable.is_empty());
    assert_eq!(
        roster.clones,
        BTreeMap::from(
            [
                ("after_first", "first"),
                ("child", "base"),
                ("cross", "child"),
                ("vc", "v")
            ]
            .map(|(child, parent)| (child.to_owned(), parent.to_owned()))
        )
    );
}

#[test]
fn a_plain_redeclaration_ends_a_copy() {
    let roster = roster_of(
        "redeclared-clones",
        &[
            (
                "gfx/models/meshes.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } pdxmesh = { name = n file = n.mesh } }",
            ),
            (
                "gfx/entities/a.asset",
                br#"entity = { name = "base" pdxmesh = "m" }
                    entity = { name = "other" pdxmesh = "n" }
                    entity = { name = "skin" clone = "base" }
                    entity = { name = "skin" pdxmesh = "n" }
                    entity = { name = "again" pdxmesh = "n" }
                    entity = { name = "again" clone = "other" }"#,
            ),
        ],
        &[],
    );
    assert_eq!(
        roster.descriptor_job().view().entity("skin").unwrap().mesh.as_deref(),
        Some("n")
    );
    assert!(!roster.clones.contains_key("skin"));
    assert_eq!(roster.clones["again"], "other");
}

#[test]
fn a_clone_of_an_unreadable_name_may_be_declared_there_too() {
    let roster = roster_of(
        "unreadable",
        &[
            (
                "gfx/models/meshes.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            (
                "gfx/entities/a.asset",
                br#"entity = { name = "maybe" } }{ = ="#,
            ),
            (
                "gfx/entities/b.asset",
                br#"entity = { name = "child" clone = "maybe" }
                    entity = { name = "grandchild" clone = "child" }
                    entity = { name = "plain" pdxmesh = "m" }"#,
            ),
        ],
        &[],
    );
    let file = |name: &str| roster.unreadable_names.get(name).map(String::as_str);
    assert_eq!(roster.unreadable.len(), 1);
    assert_eq!(file("maybe"), Some("gfx/entities/a.asset"));
    assert_eq!(file("child"), Some("gfx/entities/a.asset"));
    assert_eq!(file("grandchild"), Some("gfx/entities/a.asset"));
    assert_eq!(file("plain"), None);
    assert_eq!(roster.logged.len(), 2, "{:?}", roster.logged);
    assert_eq!(
        roster.entity_names().collect::<Vec<_>>(),
        ["plain"]
    );
}

#[test]
fn a_clone_merges_attachments_in_place_and_keeps_an_unregistered_mesh() {
    let roster = roster_of(
        "merge",
        &[
            (
                "gfx/models/a.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } pdxmesh = { name = n file = n.mesh } }",
            ),
            (
                "gfx/entities/a.asset",
                br#"entity = { name = "base" pdxmesh = "m"
                        attach = { name = rifle1 a = x1 } attach = { name = rifle2 a = x2 }
                        attach = { name = rifle4 a = x4 } attach = { name = rifle3 a = x3 }
                        attach = { b = joint_only } }
                    entity = { name = "child" clone = "base" pdxmesh = "missing"
                        attach = { name = rifle1 a = y1 } attach = { name = rifle4 a = y4 }
                        attach = { name = rifle2 a = y2 } attach = { name = rifle3 a = y3 }
                        attach = { b = joint_new } attach = { name = extra a = z } }
                    entity = { name = "plain" pdxmesh = "n" }
                    entity = { name = "plain" pdxmesh = "gone" }"#,
            ),
        ],
        &[],
    );
    let job = roster.descriptor_job();
    let entities = job.view();
    let slots: Vec<_> = entities.entity("child").unwrap()
        .attachments
        .iter()
        .map(|attachment| (attachment.name.as_str(), attachment.entity.as_str()))
        .collect();
    assert_eq!(
        slots,
        [
            ("rifle1", "y1"),
            ("rifle2", "y2"),
            ("rifle4", "y4"),
            ("rifle3", "y3"),
            ("", "joint_new"),
            ("extra", "z"),
        ]
    );
    assert_eq!(entities.entity("child").unwrap().mesh.as_deref(), Some("m"));
    assert_eq!(entities.entity("plain").unwrap().mesh.as_deref(), Some("n"));
    assert!(
        job
            .entities()
            .all(|entity| entity.unbound_mesh.is_none())
    );
    assert_eq!(roster.logged.len(), 2, "{:?}", roster.logged);
    assert!(roster.logged[0].starts_with("child in gfx/entities/a.asset: no pdxmesh missing"));
}

#[test]
fn unbound_mesh_diagnostics_follow_registration() {
    let roster = roster_of(
        "unbound",
        &[
            (
                "gfx/models/a.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            (
                "gfx/entities/a.asset",
                br#"
                entity = { name = empty pdxmesh = absent version = 1 }
                entity = { name = child clone = empty }
                entity = { name = empty pdxmesh = m version = 0 }
                entity = { name = repaired pdxmesh = absent }
                entity = { name = repaired pdxmesh = m }
                entity = { name = bare }
                entity = { name = cleared pdxmesh = absent }
                entity = { name = cleared clone = bare }
            "#,
            ),
        ],
        &[],
    );
    for name in ["empty", "child"] {
        assert_eq!(
            roster.descriptor_job().view().entity(name).unwrap().unbound_mesh.as_deref(),
            Some("absent")
        );
    }
    for name in ["bare", "cleared", "repaired"] {
        assert!(roster.descriptor_job().view().entity(name).unwrap().unbound_mesh.is_none());
    }
    for name in ["empty", "child", "bare", "cleared"] {
        assert!(roster.descriptor_job().view().entity(name).unwrap().mesh.is_none());
    }
    assert_eq!(
        roster.descriptor_job().view().entity("repaired").unwrap().mesh.as_deref(),
        Some("m")
    );
}

#[test]
fn the_roster_keeps_the_last_object_of_a_name() {
    let roster = roster_of(
        "objects",
        &[
            (
                "gfx/models/a.gfx",
                br#"objectTypes = {
                        pdxmesh = { name = "m" file = "a.mesh" }
                        pdxmesh = { name = "shadowed" file = "s.mesh" }
                        pdxmesh = { name = "kept" file = "k.mesh" }
                    }
                    entity = { name = "in_gfx" pdxmesh = "m" }"#,
            ),
            (
                "gfx/models/b.gfx",
                br#"objectTypes = {
                        pdxmesh = { name = "m" file = "b.mesh" }
                        pdxparticle = { name = "shadowed" type = "x" }
                    }"#,
            ),
            (
                "gfx/models/c.asset",
                br#"objectTypes = { pdxmesh = { name = "kept" file = "c.mesh" } }"#,
            ),
        ],
        &[],
    );
    let meshes = &roster.descriptors.meshes;
    assert_eq!(meshes["m"].file, "b.mesh");
    assert_eq!(meshes["kept"].file, "k.mesh");
    assert!(!meshes.contains_key("shadowed"));
    assert!(roster.entity_names().next().is_none());
}

#[test]
fn the_roster_keeps_the_last_animation_that_loads() {
    let clip: &[u8] = b"@@b@[info\0[samples\0";
    let roster = roster_of(
        "animations",
        &[
            (
                "gfx/models/x.asset",
                br#"animation = { name = "walk" file = "walk.anim" }
                    animation = { name = "run" file = "sub/run.anim" }
                    animation = { name = "same" file = "same.anim" }
                    light = { name = "lamp" intensity = 1 }"#,
            ),
            (
                "gfx/models/y.asset",
                br#"animation = { name = "walk" file = "missing.anim" }
                    animation = { name = "run" file = "broken.anim" }
                    animation = { name = "same" file = "same.anim" }
                    light = { name = "lamp" intensity = 2 }"#,
            ),
            ("gfx/models/walk.anim", clip),
            ("gfx/models/sub/run.anim", clip),
            ("gfx/models/broken.anim", b"@@b@[info\0"),
        ],
        &[],
    );
    let file = |name: &str| roster.descriptors.animations[name].file.as_str();
    assert_eq!(file("walk"), "gfx/models/walk.anim");
    assert_eq!(file("run"), "gfx/models/sub/run.anim");
    assert_eq!(file("same"), "gfx/models/same.anim");
    let lamp = &roster.lights["lamp"];
    assert_eq!(last_text(lamp, "intensity").as_deref(), Some("2"));
}

#[test]
fn lexical_names_keep_quoted_and_bare_words_outside_comments() {
    let names = lexical_names(
        b"# header\rentity = { name = \"GER_plane_entity\" pdxmesh=plane_mesh # hidden_entity
          clone = \"a b\\\"c\" } } {",
    );
    for name in [
        "entity",
        "name",
        "GER_plane_entity",
        "pdxmesh",
        "plane_mesh",
        "clone",
    ] {
        assert!(names.contains(name), "{name}");
    }
    assert!(names.contains("a b\"c"));
    assert!(!names.contains("a b\\\"c"));
    assert!(!names.contains("hidden_entity"));
}

#[test]
fn sampled_pose_uses_first_joint_definition_across_lods() {
    let joint = Joint {
        name: "root".into(),
        index: 0,
        parent: None,
        inverse_bind: Some(Transform::IDENTITY),
    };
    let mesh = Mesh {
        skeleton: vec![
            joint.clone(),
            Joint {
                inverse_bind: Some(Transform {
                    basis: [[0.0; 3]; 3],
                    translation: [0.0; 3],
                }),
                ..joint
            },
        ],
        ..Mesh::default()
    };
    let animation = Animation {
        fps: 2.0,
        samples: 3,
        joints: vec![AnimationJoint {
            name: "root".into(),
            channels: "tq".into(),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            ..AnimationJoint::default()
        }],
        translations: vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        rotations: vec![0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0],
        ..Animation::default()
    };
    let sampled = mesh.sampled_pose(&animation, &animation.sample(0.0, true));
    assert_eq!(sampled.len(), 1);
    assert_eq!(sampled, mesh.palette(&animation, 0));
    let middle = mesh.sampled_pose(&animation, &animation.sample(0.25, true))[0];
    assert!((middle.translation[0] - 1.0).abs() < 1e-6);
    assert!((middle.determinant() - 1.0).abs() < 1e-5);
    assert_eq!(
        animation.sample(animation.duration(), true),
        animation.sample(0.0, true)
    );
    assert_eq!(
        animation.sample(100.0, false),
        animation.sample(animation.duration(), false)
    );
}

#[test]
fn entity_states_keep_variants_empty_states_and_effect_events() {
    let descriptors=parse_descriptors(br#"
      entity = { name = parent
        state = { name = move animation = walk chance = 2 animation_blend_time = 0.4 looping = no next_state = idle
          event = { time = 0.2 node = muzzle light = flash particle = fire keep_particle = yes trigger_once = yes }
          propagate_state = { weapon = fire }
        }
        state = { name = move animation = march chance = 1 }
        state = { name = idle }
      }
      entity = { name = child clone = parent state = { name = idle animation = rest } }
    "#,"gfx/entities/test.asset").unwrap();
    let entity = descriptors.inherit("child").unwrap();
    assert_eq!(entity.states["move"].len(), 2);
    assert_eq!(entity.states["idle"][0].animation, "rest");
    let state = entity.states["move"][0];
    assert_eq!(state.chance, 2.0);
    assert_eq!(state.blend, 0.4);
    assert!(!state.looping);
    assert_eq!(state.next_state.as_deref(), Some("idle"));
    assert_eq!(state.propagate, vec![("weapon".into(), "fire".into())]);
    let event = &state.events[0];
    assert_eq!(event.time, 0.2);
    assert_eq!(event.light.as_deref(), Some("flash"));
    assert!(event.keep_particle && event.trigger_once);
}

#[derive(Default)]
struct Builder {
    bytes: Vec<u8>,
}

impl Builder {
    fn new() -> Self {
        Self {
            bytes: MAGIC.to_vec(),
        }
    }

    fn node(mut self, depth: usize, name: &str) -> Self {
        self.bytes.extend(std::iter::repeat_n(b'[', depth));
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.push(0);
        self
    }

    fn key(&mut self, name: &str, kind: u8, count: usize) {
        self.bytes.push(b'!');
        self.bytes.push(name.len() as u8);
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.push(kind);
        self.bytes.extend_from_slice(&(count as u32).to_le_bytes());
    }

    fn ints(mut self, name: &str, values: &[i32]) -> Self {
        self.key(name, b'i', values.len());
        for value in values {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    fn floats(mut self, name: &str, values: &[f32]) -> Self {
        self.key(name, b'f', values.len());
        for value in values {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    fn strings(mut self, name: &str, values: &[&str]) -> Self {
        self.key(name, b's', values.len());
        for value in values {
            self.bytes
                .extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
            self.bytes.extend_from_slice(value.as_bytes());
            self.bytes.push(0);
        }
        self
    }
}

fn triangle_mesh() -> Vec<u8> {
    Builder::new()
        .ints("pdxasset", &[1, 0])
        .node(1, "object")
        .floats("loddist", &[0.0, 100.0])
        .node(2, "planeShape")
        .ints("lod", &[0])
        .node(3, "mesh")
        .floats("p", &[-1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 0.0, 0.5, 1.0])
        .floats("n", &[0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0])
        .floats(
            "ta",
            &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0],
        )
        .floats("u0", &[0.0, 0.0, 1.0, 0.0, 0.5, 1.0])
        .floats("u1", &[0.25, 0.75, 0.25, 0.75, 0.25, 0.75])
        .ints("tri", &[0, 1, 2])
        .node(4, "aabb")
        .floats("min", &[-1.0, 0.0, -1.0])
        .floats("max", &[1.0, 0.5, 1.0])
        .node(4, "material")
        .strings("shader", &["PdxMeshAdvancedSnow"])
        .strings("diff", &["leaf_diffuse.dds"])
        .strings("n", &["leaf_normal.dds"])
        .strings("spec", &["nospec.dds"])
        .node(1, "locator")
        .node(2, "attach_01")
        .bytes
}

#[test]
fn a_synthetic_container_round_trips_into_typed_streams() {
    let mesh = read(&triangle_mesh()).unwrap();
    assert_eq!(mesh.lod_distances, vec![0.0, 100.0]);
    assert_eq!(mesh.locators, vec!["attach_01".to_string()]);
    assert!(mesh.skeleton.is_empty());
    assert_eq!(mesh.shapes.len(), 1);
    let shape = &mesh.shapes[0];
    assert_eq!(shape.name, "planeShape");
    assert_eq!(shape.lod, Some(0));
    let sub = &shape.meshes[0];
    assert_eq!(sub.vertex_count(), 3);
    assert_eq!(sub.triangle_count(), 1);
    assert_eq!(sub.positions[2], [0.0, 0.5, 1.0]);
    assert_eq!(sub.tangents[2], [1.0, 0.0, 0.0, -1.0]);
    assert_eq!(sub.uv0[1], [1.0, 0.0]);
    assert_eq!(sub.uv1.as_ref().unwrap()[0], [0.25, 0.75]);
    assert_eq!(sub.indices, vec![0, 1, 2]);
    assert_eq!(
        sub.bounds,
        Some(Bounds {
            min: [-1.0, 0.0, -1.0],
            max: [1.0, 0.5, 1.0],
        })
    );
    assert_eq!(
        sub.material,
        Some(Material {
            shader: Some("PdxMeshAdvancedSnow".into()),
            shader_file: None,
            diffuse: "leaf_diffuse.dds".into(),
            normal: "leaf_normal.dds".into(),
            specular: "nospec.dds".into(),
        })
    );
    assert!(sub.skin.is_none());
    assert_eq!(sub.normals.len(), sub.vertex_count());
    assert_eq!(sub.tangents.len(), sub.vertex_count());
    assert_eq!(sub.uv0.len(), sub.vertex_count());
    let lods = mesh.lods();
    assert_eq!(lods.len(), 1);
    assert_eq!(lods[0][0].name, "planeShape");
}

#[test]
fn the_node_tree_nests_by_depth_alone() {
    let root = parse(&triangle_mesh()).unwrap();
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0].name, "object");
    assert_eq!(root.children[1].name, "locator");
    let shape = &root.children[0].children[0];
    assert_eq!(shape.children[0].name, "mesh");
    assert_eq!(shape.children[0].children.len(), 2);
    assert_eq!(
        root.property("pdxasset").and_then(Value::ints),
        Some([1, 0].as_slice())
    );
}

#[test]
fn a_skin_is_recorded_and_a_skeleton_decoded() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "bodyShape")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0])
        .ints("tri", &[])
        .node(4, "skin")
        .ints("bones", &[2])
        .ints("ix", &[1, 0, -1, -1])
        .floats("w", &[0.75, 0.25, 0.0, 0.0])
        .node(3, "skeleton")
        .node(4, "root")
        .ints("ix", &[0])
        .node(4, "hand")
        .ints("ix", &[1])
        .ints("pa", &[0])
        .floats(
            "tx",
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -2.0, -3.0, 0.0],
        )
        .bytes;
    let mesh = read(&bytes).unwrap();
    assert_eq!(mesh.skeleton.len(), 2);
    assert_eq!(mesh.skeleton[1].name, "hand");
    assert_eq!(mesh.skeleton[1].parent, Some(0));
    assert_eq!(mesh.bind_pose("hand").unwrap().translation, [2.0, 3.0, 0.0]);
    assert_eq!(mesh.skeleton[0].inverse_bind, None);
    assert_eq!(mesh.skeleton[0].parent, Some(0));
    assert_eq!(mesh.bind_pose("root"), None);
    let skin = mesh.shapes[0].meshes[0].skin.as_ref().unwrap();
    assert_eq!(skin.bones, 2);
    assert_eq!(skin.joints, vec![[1, 0, -1, -1]]);
    assert_eq!(skin.influences(0), vec![(1, 0.75), (0, 0.25)]);
    assert!(skin.influences(1).is_empty());
    assert!(mesh.shapes[0].meshes[0].uv1.is_none());
}

#[test]
fn a_skin_stream_that_does_not_cover_every_vertex_four_times_is_refused() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "bodyShape")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0, 0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0, 0.0, 0.0])
        .ints("bones", &[1])
        .node(4, "skin")
        .ints("bones", &[1])
        .ints("ix", &[0, 0])
        .floats("w", &[1.0, 1.0])
        .bytes;
    let error = read(&bytes).unwrap_err().to_string();
    assert!(error.contains("not 8 of each"), "{error}");
}

#[test]
fn bad_input_is_refused_without_panicking() {
    assert!(parse(b"").is_err());
    assert!(parse(b"nope").is_err());
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"!\x01xq\x01\x00\x00\x00\x00");
    assert!(parse(&bytes).is_err());
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"!\x01xi\xff\xff\xff\x0f");
    assert!(parse(&bytes).is_err());
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"[object");
    assert!(parse(&bytes).is_err());
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"[[deep\x00");
    assert!(parse(&bytes).is_err());
    let mut bytes = MAGIC.to_vec();
    bytes.push(b'?');
    assert!(parse(&bytes).is_err());
}

fn mesh_bits(mesh: &Mesh) -> Vec<String> {
    fn flat<const N: usize>(rows: &[[f32; N]]) -> Vec<u32> {
        rows.iter().flatten().map(|value| value.to_bits()).collect()
    }
    let Mesh {
        lod_distances,
        shapes,
        skeleton,
        locators,
    } = mesh;
    let lod_distances: Vec<_> = lod_distances.iter().map(|value| value.to_bits()).collect();
    let mut out = vec![format!("{lod_distances:?} {locators:?}")];
    for Joint {
        name,
        index,
        parent,
        inverse_bind,
    } in skeleton
    {
        let bind = inverse_bind
            .map(|Transform { basis, translation }| (flat(&basis), translation.map(f32::to_bits)));
        out.push(format!("{name:?} {index} {parent:?} {bind:?}"));
    }
    for Shape { name, lod, meshes } in shapes {
        out.push(format!("{name:?} {lod:?}"));
        for SubMesh {
            child_index,
            positions,
            normals,
            tangents,
            uv0,
            uv1,
            uv2,
            uv3,
            indices,
            bounds,
            material,
            skin,
        } in meshes
        {
            let bounds =
                bounds.map(|Bounds { min, max }| (min.map(f32::to_bits), max.map(f32::to_bits)));
            let skin = skin.as_ref().map(
                |Skin {
                     bones,
                     joints,
                     weights,
                 }| (bones, joints, flat(weights)),
            );
            out.push(format!(
                "{child_index} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {indices:?} {bounds:?} \
                 {material:?} {skin:?}",
                flat(positions),
                flat(normals),
                flat(tangents),
                flat(uv0),
                uv1.as_deref().map(flat),
                uv2.as_deref().map(flat),
                uv3.as_deref().map(flat),
            ));
        }
    }
    out
}

const ODD_FLOATS: [f32; 6] = [
    0.0,
    -0.0,
    f32::from_bits(0x7fc0_0123),
    f32::from_bits(0xffc0_4567),
    f32::INFINITY,
    f32::from_bits(1),
];

fn mixed_mesh() -> Vec<u8> {
    let tx = [ODD_FLOATS, ODD_FLOATS].concat();
    Builder::new()
        .node(1, "object")
        .floats("loddist", &ODD_FLOATS)
        .node(2, "skeleton")
        .node(3, "root")
        .ints("ix", &[0])
        .floats("tx", &tx)
        .node(3, "child")
        .ints("ix", &[1])
        .ints("pa", &[0])
        .node(3, "unindexed")
        .node(2, "properties")
        .ints("lod", &[1])
        .node(2, "empty")
        .node(3, "notmesh")
        .node(2, "s")
        .ints("lod", &[0])
        .node(3, "mesh")
        .floats("p", &ODD_FLOATS[..3])
        .floats("n", &ODD_FLOATS[3..])
        .floats("u2", &ODD_FLOATS[1..])
        .ints("tri", &[0, 0, 0])
        .node(4, "aabb")
        .floats("min", &ODD_FLOATS[..3])
        .floats("max", &[1.0, 2.0])
        .node(4, "skin")
        .ints("bones", &[-3])
        .ints("ix", &[2, -1, 0, -1])
        .floats("w", &ODD_FLOATS[2..])
        .node(3, "skeleton")
        .node(4, "j")
        .ints("ix", &[2])
        .node(1, "locator")
        .node(2, "a")
        .node(2, "b")
        .bytes
}

fn one_mesh(streams: impl FnOnce(Builder) -> Builder) -> Vec<u8> {
    streams(
        Builder::new()
            .node(1, "object")
            .node(2, "s")
            .node(3, "mesh"),
    )
    .bytes
}

fn projection_cases() -> Vec<(&'static str, Vec<u8>)> {
    let vertex = |builder: Builder| builder.floats("p", &[0.0, 1.0, 2.0]);
    let mut bad_magic = triangle_mesh();
    bad_magic[0] = b'#';
    let mut truncated = triangle_mesh();
    truncated.truncate(truncated.len() - 3);
    let mut unknown_type = MAGIC.to_vec();
    unknown_type.extend_from_slice(b"!\x01xq\x01\x00\x00\x00\x00");
    vec![
        ("triangle", triangle_mesh()),
        ("mixed", mixed_mesh()),
        (
            "ragged optional streams",
            one_mesh(|b| {
                vertex(b)
                    .floats("n", &[1.0])
                    .floats("ta", &[1.0, 2.0])
                    .floats("u1", &[3.0])
            }),
        ),
        ("no object", MAGIC.to_vec()),
        ("ragged positions", one_mesh(|b| b.floats("p", &[0.0, 0.0]))),
        (
            "odd triangle list",
            one_mesh(|b| vertex(b).ints("tri", &[0, 0])),
        ),
        (
            "index outside the streams",
            one_mesh(|b| vertex(b).ints("tri", &[0, 1, 2])),
        ),
        (
            "negative index",
            one_mesh(|b| vertex(b).ints("tri", &[-1, 0, 0])),
        ),
        (
            "short skin",
            one_mesh(|b| {
                vertex(b)
                    .node(4, "skin")
                    .ints("ix", &[0, 0])
                    .floats("w", &[1.0])
            }),
        ),
        (
            "a later shape refuses after a decoded one",
            one_mesh(|b| {
                vertex(b)
                    .ints("tri", &[0, 0, 0])
                    .node(2, "t")
                    .node(3, "mesh")
                    .floats("p", &[0.0])
            }),
        ),
        ("bad magic", bad_magic),
        ("truncated", truncated),
        ("unknown property type", unknown_type),
    ]
}

fn expected_projection(label: &str, bytes: &[u8]) -> Result<Mesh, String> {
    let shape = |name: &str, lod, meshes| Shape {
        name: name.into(),
        lod,
        meshes,
    };
    match label {
        "triangle" => Ok(Mesh {
            lod_distances: vec![0., 100.],
            shapes: vec![shape("planeShape", Some(0), vec![SubMesh {
                child_index: 0,
                positions: vec![[-1., 0., -1.], [1., 0., -1.], [0., 0.5, 1.]],
                normals: vec![[0., 1., 0.]; 3],
                tangents: vec![[1., 0., 0., 1.], [1., 0., 0., 1.], [1., 0., 0., -1.]],
                uv0: vec![[0., 0.], [1., 0.], [0.5, 1.]],
                uv1: Some(vec![[0.25, 0.75]; 3]),
                uv2: None,
                uv3: None,
                indices: vec![0, 1, 2],
                bounds: Some(Bounds { min: [-1., 0., -1.], max: [1., 0.5, 1.] }),
                material: Some(Material {
                    shader: Some("PdxMeshAdvancedSnow".into()),
                    shader_file: None,
                    diffuse: "leaf_diffuse.dds".into(),
                    normal: "leaf_normal.dds".into(),
                    specular: "nospec.dds".into(),
                }),
                skin: None,
            }])],
            skeleton: vec![],
            locators: vec!["attach_01".into()],
        }),
        "mixed" => {
            let [a, b, c, d, e, f] = ODD_FLOATS;
            Ok(Mesh {
                lod_distances: ODD_FLOATS.to_vec(),
                shapes: vec![shape("empty", None, vec![]), shape("s", Some(0), vec![SubMesh {
                    child_index: 0,
                    positions: vec![[a, b, c]],
                    normals: vec![[d, e, f]],
                    tangents: vec![],
                    uv0: vec![],
                    uv1: None,
                    uv2: Some(vec![]),
                    uv3: None,
                    indices: vec![0, 0, 0],
                    bounds: None,
                    material: None,
                    skin: Some(Skin {
                        bones: 4,
                        joints: vec![[2, -1, 0, -1]],
                        weights: vec![[c, d, e, f]],
                    }),
                }])],
                skeleton: vec![
                    Joint { name: "j".into(), index: 0, parent: Some(0), inverse_bind: None },
                ],
                locators: vec!["a".into(), "b".into()],
            })
        }
        "ragged optional streams" => Ok(Mesh {
            shapes: vec![shape("s", None, vec![SubMesh {
                positions: vec![[0., 1., 2.]],
                uv1: Some(vec![]),
                ..SubMesh::default()
            }])],
            ..Mesh::default()
        }),
        "no object" => Err("a PDX mesh carries no [object] node".into()),
        "ragged positions" => Err("the PDX mesh shape \"s\" has 2 floats in \"p\", not a multiple of 3".into()),
        "odd triangle list" => Err("the PDX mesh shape \"s\" has 2 triangle indices, not a multiple of three".into()),
        "index outside the streams" => Err("the PDX mesh shape \"s\" indexes vertex 1 of 1".into()),
        "negative index" => Err("the PDX mesh shape \"s\" indexes vertex -1 of 1".into()),
        "short skin" => Err("the PDX mesh shape \"s\" has 2 skin indices and 1 weights for 1 vertices, not 4 of each".into()),
        "a later shape refuses after a decoded one" => Err("the PDX mesh shape \"t\" has 1 floats in \"p\", not a multiple of 3".into()),
        "bad magic" => Err("not a PDX binary asset: the file does not start with the magic \"@@b@\"".into()),
        "truncated" => Err(format!("a PDX asset node name at offset {} has no end", bytes.len() - 7)),
        "unknown property type" => Err("the PDX asset property \"x\" has the unknown type byte 0x71".into()),
        _ => panic!("no fixed projection for {label}"),
    }
}

#[test]
fn a_parsed_tree_projects_fixed_mesh_bits_and_ordered_refusals() {
    let (mut decoded, mut refused, mut unparsed) = (0, 0, 0);
    for (label, bytes) in projection_cases() {
        let expected = expected_projection(label, &bytes)
            .map(|mesh| mesh_bits(&mesh))
            .map_err(|error| (std::io::ErrorKind::InvalidData, error));
        let direct = read(&bytes)
            .map(|mesh| mesh_bits(&mesh))
            .map_err(|error| (error.kind(), error.to_string()));
        assert_eq!(direct, expected, "byte entry: {label}");
        let tree = match parse(&bytes) {
            Ok(tree) => tree,
            Err(error) => {
                assert_eq!(
                    Err((error.kind(), error.to_string())),
                    expected,
                    "parse: {label}"
                );
                unparsed += 1;
                continue;
            }
        };
        for _ in 0..2 {
            let projected = read_tree(&tree)
                .map(|mesh| mesh_bits(&mesh))
                .map_err(|error| (error.kind(), error.to_string()));
            assert_eq!(projected, expected, "borrowed projection: {label}");
        }
        if expected.is_ok() {
            decoded += 1;
        } else {
            refused += 1;
        }
    }
    assert_eq!((decoded, refused, unparsed), (3, 7, 3));
}

#[test]
fn optional_stream_counts_are_preserved_and_invalid_triangles_are_named() {
    let short = Builder::new()
        .node(1, "object")
        .node(2, "s")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0, 0.0, 0.0])
        .bytes;
    let decoded = read(&short).unwrap();
    assert_eq!(decoded.shapes[0].meshes[0].normals.len(), 1);
    assert_eq!(decoded.shapes[0].meshes[0].vertex_count(), 2);

    let hull = Builder::new()
        .node(1, "object")
        .node(2, "collision_boxShape")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
        .ints("tri", &[0, 1, 2])
        .bytes;
    let mesh = read(&hull).unwrap();
    let sub = &mesh.shapes[0].meshes[0];
    assert_eq!(sub.vertex_count(), 3);
    assert!(sub.normals.is_empty());
    assert!(sub.tangents.is_empty());
    assert!(sub.uv0.is_empty());

    let ragged = Builder::new()
        .node(1, "object")
        .node(2, "s")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0])
        .bytes;
    assert!(
        read(&ragged)
            .unwrap_err()
            .to_string()
            .contains("not a multiple of 3")
    );

    let out_of_range = Builder::new()
        .node(1, "object")
        .node(2, "s")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0])
        .ints("tri", &[0, 1, 2])
        .bytes;
    assert!(
        read(&out_of_range)
            .unwrap_err()
            .to_string()
            .contains("indexes vertex 1 of 1")
    );

    let odd = Builder::new()
        .node(1, "object")
        .node(2, "s")
        .node(3, "mesh")
        .floats("p", &[0.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0])
        .ints("tri", &[0, 0])
        .bytes;
    assert!(
        read(&odd)
            .unwrap_err()
            .to_string()
            .contains("not a multiple of three")
    );
    assert!(read(&MAGIC).unwrap_err().to_string().contains("[object]"));
}

#[test]
fn extra_uv_streams_and_empty_uv1_keep_their_authored_presence() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "shape")
        .node(3, "mesh")
        .floats("p", &[1., 2., 3.])
        .floats("u1", &[])
        .floats("u2", &[4., 5.])
        .floats("u3", &[6., 7.])
        .bytes;
    let decoded = read(&bytes).unwrap();
    let sub = &decoded.shapes[0].meshes[0];
    assert_eq!(sub.uv1, Some(Vec::new()));
    assert_eq!(sub.uv2, Some(vec![[4., 5.]]));
    assert_eq!(sub.uv3, Some(vec![[6., 7.]]));
}

#[test]
fn ragged_optional_properties_are_omitted_without_losing_uv1_presence() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "shape")
        .node(3, "mesh")
        .floats("p", &[1., 2., 3.])
        .floats("n", &[4., 5.])
        .floats("ta", &[6., 7., 8.])
        .floats("u0", &[9.])
        .floats("u1", &[10.])
        .floats("u2", &[11.])
        .floats("u3", &[12.])
        .bytes;
    let decoded = read(&bytes).unwrap();
    let sub = &decoded.shapes[0].meshes[0];
    assert!(sub.normals.is_empty());
    assert!(sub.tangents.is_empty());
    assert!(sub.uv0.is_empty());
    assert_eq!(sub.uv1, Some(Vec::new()));
    assert_eq!(sub.uv2, Some(Vec::new()));
    assert_eq!(sub.uv3, Some(Vec::new()));
}

#[test]
fn material_keeps_missing_shader_and_source_override_distinct() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "shape")
        .node(3, "mesh")
        .node(4, "material")
        .node(3, "mesh")
        .node(4, "material")
        .strings("shader", &[""])
        .bytes;
    let decoded = read(&bytes).unwrap();
    let shape = &decoded.shapes[0];
    assert_eq!(shape.meshes[0].material.as_ref().unwrap().shader, None);
    assert_eq!(
        shape.meshes[1].material.as_ref().unwrap().shader.as_deref(),
        Some("")
    );
    let descriptors = parse_descriptors(
        br#"objectTypes = { pdxmesh = { name = "m" file = "m.mesh"
            meshsettings = { name = "shape" index = 0 shader_file = "" }
            meshsettings = { name = "shape" index = 0 shader_file = "later.shader" }
            meshsettings = { name = "shape" index = 1
                shader_file = "old.shader" shader_file = "gfx/FX/custom.shader" }
        } }"#,
        "fixture.gfx",
    )
    .unwrap();
    let definition = &descriptors.meshes["m"];
    assert_eq!(
        definition.material("shape", &shape.meshes[0]).shader_file,
        None
    );
    assert_eq!(
        definition
            .material("shape", &shape.meshes[1])
            .shader_file
            .as_deref(),
        Some("gfx/FX/custom.shader")
    );
}

#[test]
fn material_overrides_follow_first_match_and_raw_child_order() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "shape")
        .node(3, "auxiliary")
        .node(3, "mesh")
        .node(4, "material")
        .strings("shader", &["base_effect"])
        .strings("diff", &["base_diff"])
        .strings("n", &["base_normal"])
        .strings("spec", &["base_spec"])
        .node(3, "mesh")
        .node(4, "material")
        .strings("shader", &["second_effect"])
        .bytes;
    let decoded = read(&bytes).unwrap();
    let shape = &decoded.shapes[0];
    assert_eq!(
        shape
            .meshes
            .iter()
            .map(|s| s.child_index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    let descriptors = parse_descriptors(
        br#"
        objectTypes = { pdxmesh = { name = "m" file = "m.mesh"
            meshsettings = { name = "shape" index = -2 texture_diffuse = "wrong-negative" }
            meshsettings = { name = "shape" index = 0 texture_diffuse = "wrong-filtered-index" }
            meshsettings = { name = "shape" index = 1 texture_diffuse = "first"
                texture_normal = "" texture_specular = "" shader = "" }
            meshsettings = { name = "shape" index = 1 texture_diffuse = "later-exact" }
            meshsettings = { name = "shape" shader = "wildcard" }
        } }"#,
        "fixture.gfx",
    )
    .unwrap();
    let definition = &descriptors.meshes["m"];
    let material = definition.material("shape", &shape.meshes[0]);
    assert_eq!(
        material,
        Material {
            shader: Some("base_effect".into()),
            shader_file: None,
            diffuse: "first".into(),
            normal: "base_normal".into(),
            specular: "base_spec".into(),
        }
    );
    assert_eq!(
        definition
            .material("shape", &shape.meshes[1])
            .shader
            .as_deref(),
        Some("wildcard")
    );
    assert_eq!(
        definition.material("other", &shape.meshes[0]),
        shape.meshes[0].material.clone().unwrap()
    );
}

#[test]
fn meshsettings_keep_unnamed_rows_signed_wildcards_and_last_field_writes() {
    let descriptors = parse_descriptors(
        br#"
        objectTypes = { pdxmesh = { name = "m" file = "m.mesh"
            meshsettings = { name = "old" name = "" index = -2 shader = "first" }
            meshsettings = { name = "shape" index = 3 shader = "later" }
            meshsettings = { name = "shape" index = -1 }
            meshsettings = { name = "shape" index = 2 index = 0x10tail index = invalid }
            meshsettings = { name = "shape" index = 010 }
            meshsettings = { name = "shape" index = 4294967295 }
            meshsettings = { texture_diffuse = "old" texture_diffuse = "" }
        } }"#,
        "fixture.gfx",
    )
    .unwrap();
    let definition = &descriptors.meshes["m"];
    assert_eq!(definition.settings.len(), 7);
    assert!(definition.settings[0].name.is_empty());
    assert!(definition.settings[0].applies("unrelated", 999));
    assert!(definition.settings[2].applies("shape", 999));
    assert!(!definition.settings[2].applies("unrelated", 999));
    assert_eq!(definition.settings[3].index, 16);
    assert_eq!(definition.settings[4].index, 8);
    assert_eq!(definition.settings[5].index, -1);
    assert_eq!(definition.settings[6].diffuse.as_deref(), Some(""));
    let sub = SubMesh {
        child_index: 3,
        ..Default::default()
    };
    assert_eq!(
        definition.material("shape", &sub).shader.as_deref(),
        Some("first")
    );
}

#[test]
fn the_gfx_grammar_maps_an_object_name_to_its_mesh_and_scale() {
    let descriptors = parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = {
                name = "survey_marker"
                file = "gfx/models/fixtures/marker.mesh"
                scale = 0.625
            }
            pdxmesh = {
                name = "display_stand"
                file = "gfx/models/fixtures/stand.mesh"
                meshsettings = {
                    name = "stand_panel"
                    index = 2
                    texture_diffuse = "panel_color.dds"
                    shader = "FixtureSurface"
                }
            }
        }
        "#,
        "gfx/entities/fixtures.gfx",
    )
    .unwrap();
    let resolved = descriptors.resolve("survey_marker").unwrap();
    assert_eq!(resolved.definition.file, "gfx/models/fixtures/marker.mesh");
    assert!((resolved.scale - 0.625).abs() < 1e-6);
    let stand = &descriptors.meshes["display_stand"];
    assert_eq!(stand.scale, 1.0);
    assert_eq!(
        stand.settings,
        vec![MeshSetting {
            name: "stand_panel".into(),
            index: 2,
            diffuse: Some("panel_color.dds".into()),
            normal: None,
            specular: None,
            shader: Some("FixtureSurface".into()),
            shader_file: None,
        }]
    );
    assert!(descriptors.resolve("absent").is_none());
}

#[test]
fn an_entity_follows_its_mesh_and_its_clone_chain() {
    let descriptors = parse_descriptors(
        br#"
        objectTypes = { pdxmesh = { name = "m" file = "a.mesh" scale = 2.0 } }
        entity = { name = "base" pdxmesh = "m" }
        entity = { name = "child" clone = "base" scale = 1.5 }
        entity = { name = "loop_a" clone = "loop_b" }
        entity = { name = "loop_b" clone = "loop_a" }
        entity = { name = "own_mesh" clone = "scaled" pdxmesh = "m" }
        entity = { name = "scaled" pdxmesh = "other" scale = 0.8 }
        "#,
        "gfx/entities/units_infantry.asset",
    )
    .unwrap();
    assert_eq!(descriptors.resolve("base").unwrap().scale, 2.0);
    assert_eq!(descriptors.resolve("child").unwrap().scale, 3.0);
    assert_eq!(descriptors.entities["child"].clone, Some("base".into()));
    assert!(descriptors.resolve("loop_a").is_none());
    let own = descriptors.resolve("own_mesh").unwrap();
    assert_eq!(own.definition.file, "a.mesh");
    assert!((own.scale - 1.6).abs() < 1e-6);
}

#[test]
fn an_entity_inherits_its_clone_s_attachments_by_handle_and_its_states() {
    let descriptors = parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = {
                name = "rig"
                file = "rig.mesh"
                animation = { id = "idle" type = "rig_idle_animation" }
                animation = { id = "move" type = "rig_move_animation" }
            }
        }
        entity = {
            name = "base"
            pdxmesh = "rig"
            default_state = "idle"
            state = { name = "idle" animation = "idle" }
            state = { name = "move" animation = "move" }
            attach = { name = "gun" artillery = "generic_gun_entity" }
            attach = { name = "crew" infantry = "generic_crew_entity" }
        }
        entity = {
            clone = "base"
            name = "german"
            pdxmesh = "rig"
            attach = { name = "crew" infantry = "GER_crew_entity" }
        }
        animation = { name = "rig_idle_animation" file = "rig_idle.anim" }
        "#,
        "gfx/entities/units_artillery.asset",
    )
    .unwrap();
    let german = descriptors.inherit("german").unwrap();
    assert_eq!(german.default_state, Some("idle"));
    assert_eq!(
        german
            .attachments
            .iter()
            .map(|attachment| (attachment.joint.as_str(), attachment.entity.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("artillery", "generic_gun_entity"),
            ("infantry", "GER_crew_entity"),
        ]
    );
    assert_eq!(german.states["idle"][0].animation, "idle");
    let resolved = descriptors.resolve_entity("german").unwrap();
    assert_eq!(
        resolved.animation.map(|animation| animation.file.as_str()),
        Some("gfx/entities/rig_idle.anim")
    );
}

#[test]
fn an_animation_block_resolves_against_its_own_directory() {
    let descriptors = parse_descriptors(
        br#"
        animation = { name = "rig_idle_animation" file = "rig_idle.anim" }
        "#,
        "gfx/models/units/vehicles/animation_vehicles.asset",
    )
    .unwrap();
    assert_eq!(
        descriptors.animations["rig_idle_animation"].file,
        "gfx/models/units/vehicles/rig_idle.anim"
    );
}

#[test]
fn a_rest_pose_composes_its_translation_rotation_and_scale() {
    let half = std::f32::consts::FRAC_PI_4;
    let transform = Transform::from_trs([1.0, 0.0, 0.0], [0.0, half.sin(), 0.0, half.cos()], 2.0);
    let point = transform.transform_point([1.0, 0.0, 0.0]);
    assert!((point[0] - 1.0).abs() < 1e-5, "{point:?}");
    assert!(point[1].abs() < 1e-5, "{point:?}");
    assert!((point[2] + 2.0).abs() < 1e-5, "{point:?}");
    assert!((transform.uniform_scale() - 2.0).abs() < 1e-5);
    let hidden = Transform::from_trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], 0.0);
    assert_eq!(hidden.uniform_scale(), 0.0);
    assert!(hidden.inverse().is_none());
    let wild = Transform::from_columns(&[1e30; 12]).unwrap();
    assert!(wild.inverse().is_none());
}

fn animation_bytes() -> Vec<u8> {
    Builder::new()
        .node(1, "info")
        .floats("fps", &[30.0])
        .ints("sa", &[3])
        .ints("j", &[2])
        .node(2, "root")
        .strings("sa", &[""])
        .floats("t", &[0.0, 0.0, 0.0])
        .floats("q", &[0.0, 0.0, 0.0, 1.0])
        .floats("s", &[1.0])
        .node(2, "slider")
        .strings("sa", &["tq"])
        .floats("t", &[1.0, 0.0, 2.0])
        .floats("q", &[0.0, 0.0, 0.0, 1.0])
        .floats("s", &[0.5])
        .node(1, "samples")
        .floats(
            "t",
            &[
                1.0, 0.0, 2.0,
                2.0, 0.0, 2.0,
                3.0, 0.0, 2.0,
            ],
        )
        .floats(
            "q",
            &[
                0.0, 0.0, 0.0, 1.0,
                0.0, 0.0, 0.0, 1.0,
                0.0, 0.0, 0.0, 1.0,
            ],
        )
        .bytes
}

#[test]
fn an_animation_info_block_decodes_its_rest_pose() {
    let animation = read_animation(&animation_bytes()).unwrap();
    assert_eq!(animation.fps, 30.0);
    assert_eq!(animation.samples, 3);
    assert_eq!(animation.joints.len(), 2);
    assert_eq!(animation.joints[1].channels, "tq");
    let rest = animation.rest("slider").unwrap();
    assert_eq!(rest.translation, [1.0, 0.0, 2.0]);
    assert!((rest.uniform_scale() - 0.5).abs() < 1e-5);
    assert!(animation.rest("absent").is_none());
}

#[test]
fn the_samples_block_is_frame_major_over_the_joints_that_declare_the_channel() {
    let animation = read_animation(&animation_bytes()).unwrap();
    assert_eq!(animation.channel_joints(), [1, 1, 0]);
    assert_eq!(
        animation.local("slider", FIRST_FRAME),
        animation.rest("slider")
    );
    assert_eq!(
        animation.local("slider", 2).unwrap().translation,
        [3.0, 0.0, 2.0]
    );
    assert!((animation.local("slider", 2).unwrap().uniform_scale() - 0.5).abs() < 1e-5);
    assert_eq!(animation.local("root", 2), animation.rest("root"));
    assert_eq!(animation.local("slider", 99), animation.rest("slider"));
    assert_eq!(animation.local("absent", 0), None);
}

#[test]
fn a_samples_stream_of_the_wrong_length_is_a_named_error() {
    let bytes = Builder::new()
        .node(1, "info")
        .floats("fps", &[30.0])
        .ints("sa", &[3])
        .node(2, "vehicle")
        .strings("sa", &["t"])
        .floats("t", &[0.0, 0.0, 0.0])
        .node(1, "samples")
        .floats("t", &[0.0, 0.0, 0.0])
        .bytes;
    let error = read_animation(&bytes).unwrap_err().to_string();
    assert!(error.contains("needs 9 floats and has 3"), "{error}");
}

fn arm_bytes() -> Vec<u8> {
    Builder::new()
        .node(1, "object")
        .node(2, "armShape")
        .node(3, "mesh")
        .floats("p", &[2.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0])
        .ints("tri", &[])
        .node(4, "skin")
        .ints("bones", &[1])
        .ints("ix", &[1, -1, -1, -1])
        .floats("w", &[1.0, 0.0, 0.0, 0.0])
        .node(3, "skeleton")
        .node(4, "root")
        .ints("ix", &[0])
        .floats(
            "tx",
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        )
        .node(4, "hand")
        .ints("ix", &[1])
        .ints("pa", &[0])
        .floats(
            "tx",
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -2.0, 0.0, 0.0],
        )
        .bytes
}

#[test]
fn a_palette_poses_a_joint_and_undoes_its_bind() {
    let mesh = read(&arm_bytes()).unwrap();
    let animation = read_animation(
        &Builder::new()
            .node(1, "info")
            .ints("sa", &[1])
            .node(2, "root")
            .strings("sa", &[""])
            .floats("t", &[0.0, 0.0, 0.0])
            .floats("q", &[0.0, 0.0, 0.0, 1.0])
            .floats("s", &[1.0])
            .node(2, "hand")
            .strings("sa", &[""])
            .floats("t", &[2.0, 1.0, 0.0])
            .floats("q", &[0.0, 0.0, 0.0, 1.0])
            .floats("s", &[1.0])
            .bytes,
    )
    .unwrap();
    assert_eq!(
        mesh.pose(&animation, FIRST_FRAME, "hand")
            .unwrap()
            .translation,
        [2.0, 1.0, 0.0]
    );
    let palette = mesh.palette(&animation, FIRST_FRAME);
    assert_eq!(palette.len(), 2);
    let skin = mesh.shapes[0].meshes[0].skin.as_ref().unwrap();
    assert_eq!(skin.influences(0), vec![(1, 1.0)]);
    assert_eq!(palette[1].transform_point([2.0, 0.0, 0.0]), [2.0, 1.0, 0.0]);
    assert_eq!(palette[0], Transform::IDENTITY);
}

#[test]
fn a_palette_under_no_animation_leaves_every_vertex_in_its_bind_pose() {
    let mesh = read(&arm_bytes()).unwrap();
    let palette = mesh.palette(&Animation::default(), FIRST_FRAME);
    for (joint, matrix) in palette.iter().enumerate() {
        let at = matrix.transform_point([2.0, 0.0, 0.0]);
        assert!(
            at.iter()
                .zip([2.0, 0.0, 0.0])
                .all(|(value, want)| (value - want).abs() < 1e-5),
            "joint {joint} moved the vertex to {at:?}"
        );
    }
}

#[test]
fn a_joint_with_an_unusable_inverse_bind_takes_the_identity() {
    let bytes = Builder::new()
        .node(1, "object")
        .node(2, "s")
        .node(3, "mesh")
        .floats("p", &[1.0, 0.0, 0.0])
        .floats("n", &[0.0, 1.0, 0.0])
        .floats("ta", &[1.0, 0.0, 0.0, 1.0])
        .floats("u0", &[0.0, 0.0])
        .node(3, "skeleton")
        .node(4, "zero")
        .ints("ix", &[0])
        .floats("tx", &[0.0; 12])
        .node(4, "huge")
        .ints("ix", &[1])
        .floats(
            "tx",
            &[
                1e12, 0.0, 0.0, 0.0, 1e12, 0.0, 0.0, 0.0, 1e12, -1e30, 0.0, 0.0,
            ],
        )
        .bytes;
    let mesh = read(&bytes).unwrap();
    let palette = mesh.palette(&Animation::default(), FIRST_FRAME);
    assert_eq!(palette, vec![Transform::IDENTITY; 2]);
}

#[test]
fn bound_sampled_pose_matches_name_join_for_reordered_partial_and_duplicate_joints() {
    let translated = |x| Some(Transform::from_trs([x, 0.25, -0.5], [0., 0., 0., 1.], 1.));
    let mut mesh = Mesh {
        skeleton: vec![
            Joint {
                name: "tip".into(),
                index: 5,
                parent: Some(2),
                inverse_bind: translated(-2.),
            },
            Joint {
                name: "root".into(),
                index: 2,
                parent: None,
                inverse_bind: translated(-1.),
            },
            Joint {
                name: "unanimated".into(),
                index: 7,
                parent: Some(2),
                inverse_bind: translated(-3.),
            },
            Joint {
                name: "ignored duplicate index".into(),
                index: 5,
                parent: None,
                inverse_bind: None,
            },
            Joint {
                name: "absent bind".into(),
                index: 8,
                parent: Some(7),
                inverse_bind: None,
            },
        ],
        ..Default::default()
    };
    let animation = Animation {
        fps: 7.5,
        samples: 9,
        joints: ["extra", "tip", "root", "tip"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| AnimationJoint {
                name: name.into(),
                channels: "tqs".into(),
                translation: [i as f32, -0.0, 0.2],
                rotation: [0., 0.1, 0., 0.9],
                scale: 1.25,
            })
            .collect(),
        translations: (0..9 * 4 * 3).map(|i| (i as f32 * 0.37).sin()).collect(),
        rotations: (0..9 * 4)
            .flat_map(|i| [0., (i as f32 * 0.1).sin(), 0., 0.8])
            .collect(),
        scales: (0..9 * 4).map(|i| 0.2 + i as f32 * 0.03).collect(),
    };
    let bits = |pose: &[Transform]| {
        pose.iter()
            .map(|pose| {
                (
                    pose.basis.map(|column| column.map(f32::to_bits)),
                    pose.translation.map(f32::to_bits),
                )
            })
            .collect::<Vec<_>>()
    };
    for cyclic in [false, true] {
        if cyclic {
            mesh.skeleton[1].parent = Some(5);
        }
        let binding = PoseBinding::new(&mesh, &animation);
        let (mut pose, mut memo) = (Vec::new(), Vec::new());
        for time in [-0.4, 0., 0.01, 0.125, 0.7, 1.1, 15.] {
            for looping in [false, true] {
                let locals = animation.sample(time, looping);
                for count in 0..=locals.len() {
                    binding.sample_into(&locals[..count], &mut pose, &mut memo);
                    assert_eq!(
                        bits(&pose),
                        bits(&mesh.sampled_pose(&animation, &locals[..count])),
                        "cyclic={cyclic} time={time} looping={looping} locals={count}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_mesh_s_cull_distance_is_squared_after_each_assignment_and_is_the_engine_s_when_unset() {
    let parsed = parse_descriptors(
        b"objectTypes = { pdxmesh = { name = plain file = p.mesh }
            pdxmesh = { name = far file = f.mesh cull_distance = 600 }
            pdxmesh = { name = twice file = t.mesh cull_distance = 600 cull_distance = 50 }
            pdxmesh = { name = bad file = b.mesh cull_distance = 20 cull_distance = x } }",
        "gfx/models/meshes.gfx",
    )
    .unwrap();
    let squared = |name: &str| parsed.meshes.get(name).unwrap().cull_distance_squared;
    assert_eq!(squared("plain"), DEFAULT_CULL_DISTANCE_SQUARED);
    assert_eq!(squared("far"), 600.0 * 600.0);
    assert_eq!(squared("twice"), 50.0 * 50.0, "the last assignment");
    assert_eq!(squared("bad"), 400.0 * 400.0, "a failed conversion squares the value it keeps");
}

#[test]
fn collision_keeps_the_last_collision_shaded_mesh_and_narrows_its_indices() {
    let mesh = |shader: &str, p: Vec<f32>, tri: Vec<i32>| Node {
        name: "mesh".into(),
        properties: vec![("p".into(), Value::Floats(p)), ("tri".into(), Value::Ints(tri))],
        children: vec![Node {
            name: "material".into(),
            properties: vec![("shader".into(), Value::Strings(vec![shader.into()]))],
            ..Node::default()
        }],
    };
    let shape = |meshes: Vec<Node>| Node { name: "shape".into(), children: meshes, ..Node::default() };
    let root = |shapes: Vec<Node>| Node {
        children: vec![Node { name: "object".into(), children: shapes, ..Node::default() }],
        ..Node::default()
    };
    let triangle = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let found = collision(&root(vec![
        shape(vec![mesh("Collision", vec![9.0; 9], vec![0, 1, 2])]),
        shape(vec![
            mesh("PdxMeshStandard", vec![5.0; 9], vec![0, 1, 2]),
            mesh("Collision", triangle.clone(), vec![2, 1, 0x1_0000, 1]),
        ]),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(found.positions, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    assert_eq!(found.indices, [2, 1, 0]);
    assert_eq!(collision(&root(vec![shape(vec![mesh("PdxMeshStandard", triangle.clone(), vec![0, 1, 2])])])).unwrap(), None);
    assert!(collision(&root(vec![shape(vec![mesh("Collision", triangle, vec![0, 1, 3])])])).is_err());
}

#[test]
fn entity_and_mesh_scale_multiply_and_follow_native_clone_and_field_rules() {
    let descriptors = parse_descriptors(
        br#"
        objectTypes = {
            pdxmesh = { name = "unit_mesh" file = "u.mesh" }
            pdxmesh = { name = "mesh_only" file = "m.mesh" scale = 2.5 }
            pdxmesh = { name = "alt_mesh" file = "a.mesh" scale = 0.75 }
            pdxmesh = { name = "mesh_last_wins" file = "l.mesh" scale = 2.0 SCALE = 4.5 scale = bad }
        }
        entity = { name = "only_mesh" pdxmesh = "mesh_only" }
        entity = { name = "only_entity" pdxmesh = "unit_mesh" scale = 3.25 }
        entity = { name = "combined" pdxmesh = "mesh_only" scale = 0.4 }
        entity = { name = "inherited" clone = "combined" }
        entity = { name = "grandchild" clone = "inherited" }
        entity = { name = "overridden" clone = "combined" scale = 1.6 }
        entity = { name = "switched_mesh" clone = "combined" pdxmesh = "alt_mesh" }
        entity = { name = "entity_last_wins" pdxmesh = "mesh_only" scale = 1.5 SCALE = 0.6 scale = bad }
        entity = { name = "clone_bad_scale" clone = "combined" scale = bad }
        "#,
        "gfx/entities/scales.asset",
    )
    .unwrap();
    let approx = |actual: f32, expected: f32, label: &str| {
        assert!(
            (actual - expected).abs() < 1e-6,
            "{label}: expected {expected}, got {actual}"
        );
    };
    approx(descriptors.resolve("mesh_only").unwrap().scale, 2.5, "bare mesh");
    approx(descriptors.resolve("only_mesh").unwrap().scale, 2.5, "entity unscaled");
    approx(descriptors.resolve("only_entity").unwrap().scale, 3.25, "mesh unscaled");
    approx(descriptors.resolve("combined").unwrap().scale, 1.0, "2.5 * 0.4");
    approx(descriptors.resolve("inherited").unwrap().scale, 1.0, "clone inherits");
    approx(descriptors.resolve("grandchild").unwrap().scale, 1.0, "grandchild inherits");
    approx(descriptors.resolve("overridden").unwrap().scale, 4.0, "clone overrides 2.5 * 1.6");
    approx(descriptors.resolve("switched_mesh").unwrap().scale, 0.3, "clone switches mesh 0.75 * 0.4");
    approx(descriptors.resolve("mesh_last_wins").unwrap().scale, 4.5, "mesh repeated/case/bad scale");
    approx(descriptors.resolve("entity_last_wins").unwrap().scale, 1.5, "entity repeated/case/bad scale");
    approx(descriptors.resolve("clone_bad_scale").unwrap().scale, 2.5, "clone malformed scale resets entity factor to 1.0");
}

#[test]
fn roster_clone_inheritance_multiplies_entity_and_mesh_scale_factors() {
    let roster = roster_of(
        "scales",
        &[
            (
                "gfx/models/meshes.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh scale = 2.5 }
                    pdxmesh = { name = n file = n.mesh scale = 0.75 } }",
            ),
            (
                "gfx/entities/scales.asset",
                br#"entity = { name = "base" pdxmesh = "m" scale = 0.4 }
                    entity = { name = "inherited" clone = "base" }
                    entity = { name = "overridden" clone = "base" scale = 1.6 }
                    entity = { name = "switched" clone = "base" pdxmesh = "n" }
                    entity = { name = "bad_override" clone = "base" scale = bad }"#,
            ),
        ],
        &[],
    );
    let resolve = |name: &str| roster.descriptor_job().view().resolve(name).unwrap().scale;
    assert!((resolve("base") - 1.0).abs() < 1e-6);
    assert!((resolve("inherited") - 1.0).abs() < 1e-6);
    assert!((resolve("overridden") - 4.0).abs() < 1e-6);
    assert!((resolve("switched") - 0.3).abs() < 1e-6);
    assert!((resolve("bad_override") - 2.5).abs() < 1e-6);
}

#[test]
fn file_variables_resolve_mesh_and_cloned_entity_scales_in_file_order() {
    let roster = roster_of(
        "var_scales",
        &[
            (
                "gfx/models/meshes.gfx",
                br#"@mesh_factor = 2.5
                    @mesh_factor = 9.0
                    objectTypes = {
                        pdxmesh = { name = early file = e.mesh scale = @late_mesh }
                        pdxmesh = { name = var_mesh file = m.mesh scale = @mesh_factor }
                        pdxmesh = { name = quoted_mesh file = q.mesh scale = 3.0 scale = "@mesh_factor" }
                    }
                    @late_mesh = 4.0"#,
            ),
            (
                "gfx/models/variants.gfx",
                br#"objectTypes = {
                        pdxmesh = { name = choices file = choices.mesh
                            variant = { @factor = 3 weight = 1 pdxmesh = alternate }
                        }
                        pdxmesh = { name = target file = target.mesh scale = @factor }
                    }"#,
            ),
            (
                "gfx/entities/scales.asset",
                br#"@base_scale = 0.4
                    @override_scale = 1.6
                    entity = { name = "base" pdxmesh = "var_mesh" scale = @base_scale }
                    entity = { name = "inherited" clone = "base" }
                    entity = { name = "overridden" clone = "base" scale = @override_scale }
                    entity = { name = "forward" clone = "base" scale = @after_clone }
                    @after_clone = 5.0
                    entity = { name = "nested_state_before_name" pdxmesh = "var_mesh" state = { @factor = 3 name = "idle" } scale = @factor }
                    entity = { name = "before_nested" pdxmesh = "var_mesh" scale = @late_nested state = { @late_nested = 4.0 name = "idle" } }
                    entity = { name = "after_nested" pdxmesh = "var_mesh" scale = @late_nested }
                    entity = { name = "orphan" clone = "missing_parent" @from_orphan = 1.2 state = { @from_orphan_state = 2.0 name = "idle" } }
                    entity = { name = "uses_orphan" pdxmesh = "var_mesh" scale = @from_orphan }
                    entity = { name = "uses_orphan_state" pdxmesh = "var_mesh" scale = @from_orphan_state }
                    entity = { name = "versioned" pdxmesh = "var_mesh" version = 2 }
                    entity = { name = "versioned" pdxmesh = "var_mesh" @from_older = 0.8 version = 1 }
                    entity = { name = "uses_older" pdxmesh = "var_mesh" scale = @from_older }"#,
            ),
            (
                "gfx/entities/state_trailing_var.asset",
                br#"entity = { name = "nested_state_after_name" pdxmesh = "var_mesh" state = { name = "idle" @factor = 3 } scale = @factor }"#,
            ),
            (
                "gfx/entities/skipped_unknown_block.asset",
                br#"entity = { name = "skipped_unknown" pdxmesh = "var_mesh" unused_options = { @factor = 3 } @factor = 2 scale = @factor }"#,
            ),
            (
                "gfx/entities/skipped_animation_block.asset",
                br#"animation = { name = "ignored_anim" file = "a.anim" @factor = 3 }
                    @factor = 2
                    entity = { name = "after_animation" pdxmesh = "var_mesh" scale = @factor }"#,
            ),
            (
                "gfx/entities/hash_collision.asset",
                br#"@costarring = 3
                    entity = { name = "hash_collide" pdxmesh = "var_mesh" scale = @liquid }"#,
            ),
        ],
        &[],
    );
    let resolve = |name: &str| roster.descriptor_job().view().resolve(name).unwrap().scale;
    assert!((resolve("early") - 1.0).abs() < 1e-6);
    assert!((resolve("var_mesh") - 2.5).abs() < 1e-6);
    assert!((resolve("quoted_mesh") - 3.0).abs() < 1e-6);
    assert!((resolve("base") - 1.0).abs() < 1e-6);
    assert!((resolve("inherited") - 1.0).abs() < 1e-6);
    assert!((resolve("overridden") - 4.0).abs() < 1e-6);
    assert!((resolve("forward") - 2.5).abs() < 1e-6);
    assert!((resolve("nested_state_before_name") - 7.5).abs() < 1e-6);
    assert!((resolve("nested_state_after_name") - 2.5).abs() < 1e-6);
    assert!((resolve("before_nested") - 2.5).abs() < 1e-6);
    assert!((resolve("after_nested") - 10.0).abs() < 1e-6);
    assert!((resolve("uses_orphan") - 3.0).abs() < 1e-6);
    assert!((resolve("uses_orphan_state") - 5.0).abs() < 1e-6);
    assert!((resolve("uses_older") - 2.0).abs() < 1e-6);
    assert!((resolve("skipped_unknown") - 5.0).abs() < 1e-6);
    assert!((resolve("after_animation") - 5.0).abs() < 1e-6);
    assert!((resolve("hash_collide") - 7.5).abs() < 1e-6);
    assert!((resolve("target") - 3.0).abs() < 1e-6);
}

#[test]
fn unsettled_entity_read_is_refused_and_falls_back_to_recorded_file() {
    use content_reader_layers::content::{ResolveOptions, visual};
    use std::sync::atomic::AtomicBool;

    let unsettled = br#"
        entity = {
            game_data = { atlas_size = { @extra = overlay_only x = 4 } }
            name = @extra
            pdxmesh = "m"
        }
        entity = {
            game_data = { atlas_size = { @redef = overlay_redef_one x = 4 } }
            pdxmesh = "m"
        }
        @redef = overlay_redef_two
        @redef_copy = @redef
        entity = { name = "other_file_valid" name = @redef_copy pdxmesh = "m" }
        entity = { name = "other_file_valid" name = "overwritten_final" pdxmesh = "m" }
        entity = { name = "first" pdxmesh = "m" scale = 9.0 locator = { name = "point" position = { @x = 2 0 0 } } }
        entity = { name = "second" pdxmesh = "m" scale = 9.0 }
        @after_extra = overlay_after
        entity = { name = @after_extra pdxmesh = "m" }
    "#;
    assert!(parse_descriptors(unsettled, "gfx/entities/a.asset").is_err());

    let copy_revisit = br#"
        entity = { game_data = { atlas_size = { @v = copy_wanted x = 1 } } pdxmesh = "m" }
        @copy = @v
        entity = { name = @copy pdxmesh = "m" }
    "#;
    assert!(parse_descriptors(copy_revisit, "gfx/entities/copy_revisit.asset").is_err());

    let embedded_end = b"@id = wanted entity = { game_data = { atlas_size = { loose k = 1\xff x = 1 } } name = @id pdxmesh = \"m\" }";
    assert!(parse_descriptors(embedded_end, "gfx/entities/embedded.asset").is_err());

    let embedded_def_end = b"@id = wanted_after_def entity = { game_data = { atlas_size = { @x = 1 \xff } } name = @id pdxmesh = \"m\" }";
    assert!(parse_descriptors(embedded_def_end, "gfx/entities/embedded_def.asset").is_err());

    let root = std::env::temp_dir().join(format!(
        "content-reader-unsettled-overlay-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    };
    let game_dir = root.join("game");
    let user_dir = root.join("user");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::create_dir_all(user_dir.join("mod")).unwrap();
    write(
        "base/gfx/models/m.gfx",
        b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
    );
    write(
        "base/gfx/entities/a.asset",
        br#"
            @model = first
            entity = { name = @model pdxmesh = "m" scale = 2.0 }
            entity = { name = "second" pdxmesh = "m" scale = 4.0 }
        "#,
    );
    write(
        "base/gfx/entities/broken.asset",
        br#"
            @broken_var = broken_second
            entity = { name = "broken_first" pdxmesh = "m" locator = { name = "point" position = { @x = 2 0 0 } } }
            entity = { name = @broken_var pdxmesh = "m" }
        "#,
    );
    write(
        "base/gfx/entities/other.asset",
        b"entity = { name = \"other_file_valid\" pdxmesh = \"m\" scale = 3.0 }",
    );
    let overlay_dir = root.join("overlay");
    write("overlay/gfx/entities/a.asset", unsettled);
    write("overlay/gfx/entities/copy_revisit.asset", copy_revisit);
    write("overlay/gfx/entities/embedded.asset", embedded_end);
    write("overlay/gfx/entities/embedded_def.asset", embedded_def_end);
    let descriptor = format!(
        "name=\"VisualOverride\"\npath=\"{}\"\n",
        overlay_dir.display().to_string().replace('\\', "/")
    );
    write("user/mod/visual_override.mod", descriptor.as_bytes());
    let base = root.join("base");
    let mut layers = content_reader_layers::ResolvedLayers::from_folders(&[(&base, &[][..])]).unwrap();
    let options = ResolveOptions {
        game_dir,
        user_dir,
        cache_dir: root.join("cache"),
    };
    let plan = visual::apply(
        &mut layers,
        &options,
        &[visual::Choice {
            id: "mod/visual_override.mod".into(),
            name: "VisualOverride".into(),
        }],
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(plan.order.len(), 1, "{:?}", plan.warnings);
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let roster = Roster::read(layers.presentation()).unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(roster.descriptor_job().view().resolve("first").unwrap().scale, 2.0);
    assert_eq!(roster.descriptor_job().view().resolve("second").unwrap().scale, 4.0);
    assert_eq!(roster.refused_declaration("first"), None);
    assert_eq!(roster.refused_declaration("second"), None);
    assert_eq!(roster.refused_declaration("other_file_valid"), None);
    assert_eq!(roster.unreadable_names.get("other_file_valid"), None);
    assert_eq!(roster.unreadable_names.get("m"), None);
    assert_eq!(roster.unreadable_names.get("@model"), None);
    assert_eq!(
        roster.unreadable_names.get("@extra").map(String::as_str),
        Some("gfx/entities/a.asset")
    );
    assert!(roster.refused_declaration("@extra").is_some());
    assert_eq!(roster.unreadable_names.get("@after_extra"), None);
    assert_eq!(roster.unreadable_names.get("@broken_var"), None);
    assert_eq!(roster.unreadable_names.get("@redef"), None);
    assert_eq!(roster.unreadable_names.get("@redef_copy"), None);
    assert_eq!(roster.unreadable_names.get("@id"), None);
    assert_eq!(roster.unreadable_names.get("@copy"), None);
    assert!(roster.refused_declaration("overlay_only").is_some());
    assert!(roster.refused_declaration("overlay_redef_one").is_some());
    assert!(roster.refused_declaration("overlay_redef_two").is_some());
    assert!(roster.refused_declaration("overwritten_final").is_some());
    assert!(roster.refused_declaration("overlay_after").is_some());
    assert!(roster.refused_declaration("copy_wanted").is_some());
    assert!(roster.refused_declaration("wanted").is_some());
    assert!(roster.refused_declaration("wanted_after_def").is_some());
    assert!(roster.refused_declaration("broken_first").is_some());
    assert!(roster.refused_declaration("broken_second").is_some());
    assert_eq!(
        roster.unreadable_names.get("overlay_only").map(String::as_str),
        Some("gfx/entities/a.asset")
    );
    assert_eq!(
        roster
            .unreadable_names
            .get("overlay_redef_one")
            .map(String::as_str),
        Some("gfx/entities/a.asset")
    );
    assert_eq!(
        roster
            .unreadable_names
            .get("overlay_redef_two")
            .map(String::as_str),
        Some("gfx/entities/a.asset")
    );
    assert_eq!(
        roster
            .unreadable_names
            .get("overwritten_final")
            .map(String::as_str),
        Some("gfx/entities/a.asset")
    );
    assert_eq!(
        roster.unreadable_names.get("copy_wanted").map(String::as_str),
        Some("gfx/entities/copy_revisit.asset")
    );
    assert_eq!(
        roster.unreadable_names.get("wanted").map(String::as_str),
        Some("gfx/entities/embedded.asset")
    );
    assert_eq!(
        roster
            .unreadable_names
            .get("wanted_after_def")
            .map(String::as_str),
        Some("gfx/entities/embedded_def.asset")
    );
    assert!(!roster.has_entity("broken_first"));
    assert!(!roster.has_entity("broken_second"));
    assert_eq!(
        roster.unreadable_names.get("broken_first").map(String::as_str),
        Some("gfx/entities/broken.asset")
    );
    assert_eq!(
        roster.unreadable_names.get("broken_second").map(String::as_str),
        Some("gfx/entities/broken.asset")
    );
    assert!(
        roster
            .unreadable
            .iter()
            .any(|entry| entry.contains("gfx/entities/broken.asset")),
        "{:?}",
        roster.unreadable
    );
}

#[test]
fn mixed_unknown_dispatch_recovery_handles_brace_shifts_variables_and_lexer_rules() {
    let mut mixed_brace_file = Vec::new();
    mixed_brace_file.extend_from_slice(
        br#"
        entity = {
            game_data = { atlas_size = { @a = 1 } }
        }
        }
        locator = { position = { @b = 1 } }
        name = "mixed_moved_in"
        pdxmesh = "m"
        }
        entity = {
            game_data = { atlas_size = { @c = 1 } }
            name = "mixed_kept_early"
            locator = { position = { @d = 1 } }
            name = "all_skipped_late"
            pdxmesh = "m"
        }
        entity = {
            game_data = { atlas_size = { @liquid = id_first x = 1 } }
            locator = { position = { @costarring = id_middle x = 1 } }
        }
        @liquid = id_last
        entity = { name = @liquid pdxmesh = "m" }
        entity = {
            game_data = { atlas_size = { @op = scalar_first x = 1 } }
            locator = { position = { @op = { } x = 1 } }
        }
        entity = {
            name = "opener_kept"
            locator = { position = @op }
            name = "opener_after"
            pdxmesh = "m"
        }
        entity = {
            name = @late_unused
            name = "final_overwrites_var"
            game_data = { atlas_size = { @late_unused = never_declared_1 x = 1 } }
            pdxmesh = "m"
        }
        @late_unused = never_declared_2
        "#,
    );

    let embedded_precision =
        b"entity = { name = unrelated_mesh game_data = { atlas_size = { a = 1 one two k = 1\xff } name = embedded_real } name = embedded_real pdxmesh = \"m\" }";

    let quoted_keys = br#"
        "@q_entity" = quoted_key_entity
        entity = {
            game_data = { atlas_size = { "@q_inner" = quoted_inner_entity x = 1 } }
            name = @q_entity
            pdxmesh = "m"
        }
        entity = { name = @q_inner pdxmesh = "m" }
        "@q_anim" = quoted_key_anim
        animation = { name = @q_anim file = "a.anim" }
    "#;

    let semicolon_comments = br#"
        ; entity = { name = semicolon_commented_out pdxmesh = "m" }
        ; {
        entity = {
            game_data = { atlas_size = { @sc = 1 } }
            name = semicolon_split_entity;trailing_comment
            pdxmesh = "m"
        }
        animation = { name = semicolon_split_anim;trailing_comment file = "a.anim" }
    "#;

    let retained_name = br#"
        entity = {
            game_data = { atlas_size = { @ret = retained_from_block x = 1 } }
            @ret = retained_from_outer
            pdxmesh = @ret
            name trailing_token
        }
        entity = {
            game_data = { atlas_size = { retained_from_list } }
            name trailing_token
        }
        animation = {
            file = "retained_anim_name"
            name trailing_token
        }
    "#;

    let unbound_blocks = br#"
        animation = { name = @unbound_anim file = "a.anim" }
        light = { name = @unbound_light }
        entity = { game_data = { atlas_size = { @x = 1 } } }
    "#;

    let roster = roster_of(
        "mixed-dispatch-recovery",
        &[
            (
                "gfx/models/m.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            ("gfx/entities/mixed.asset", &mixed_brace_file),
            ("gfx/entities/embedded.asset", embedded_precision),
            ("gfx/entities/quoted.asset", quoted_keys),
            ("gfx/entities/semicolon.asset", semicolon_comments),
            ("gfx/entities/retained.asset", retained_name),
            ("gfx/entities/unbound.asset", unbound_blocks),
        ],
        &[],
    );

    for (name, file) in [
        ("mixed_moved_in", "gfx/entities/mixed.asset"),
        ("mixed_kept_early", "gfx/entities/mixed.asset"),
        ("all_skipped_late", "gfx/entities/mixed.asset"),
        ("id_first", "gfx/entities/mixed.asset"),
        ("id_middle", "gfx/entities/mixed.asset"),
        ("id_last", "gfx/entities/mixed.asset"),
        ("opener_kept", "gfx/entities/mixed.asset"),
        ("opener_after", "gfx/entities/mixed.asset"),
        ("final_overwrites_var", "gfx/entities/mixed.asset"),
        ("embedded_real", "gfx/entities/embedded.asset"),
        ("quoted_key_entity", "gfx/entities/quoted.asset"),
        ("quoted_inner_entity", "gfx/entities/quoted.asset"),
        ("@q_inner", "gfx/entities/quoted.asset"),
        ("quoted_key_anim", "gfx/entities/quoted.asset"),
        ("semicolon_split_entity", "gfx/entities/semicolon.asset"),
        ("semicolon_split_anim", "gfx/entities/semicolon.asset"),
        ("retained_from_block", "gfx/entities/retained.asset"),
        ("retained_from_outer", "gfx/entities/retained.asset"),
        ("retained_from_list", "gfx/entities/retained.asset"),
        ("retained_anim_name", "gfx/entities/retained.asset"),
        ("@unbound_anim", "gfx/entities/unbound.asset"),
        ("@unbound_light", "gfx/entities/unbound.asset"),
    ] {
        assert_eq!(
            roster.unreadable_names.get(name).map(String::as_str),
            Some(file),
            "expected {name} to be refused from {file}; refused={:?}",
            roster.unreadable_names
        );
    }

    for absent in [
        "unrelated_mesh",
        "never_declared_1",
        "never_declared_2",
        "semicolon_commented_out",
        "semicolon_split_entity;trailing_comment",
        "semicolon_split_anim;trailing_comment",
    ] {
        assert_eq!(
            roster.unreadable_names.get(absent),
            None,
            "did not expect {absent} to be refused"
        );
    }
}

#[test]
fn recovery_merges_identical_unknown_dispatch_states_without_exponential_blowup() {
    let mut benign = String::from("entity = { name = \"converging_entity\" pdxmesh = \"m\"\n");
    for i in 0..64 {
        benign.push_str(&format!(
            "locator = {{ name = \"l{i}\" position = {{ @dead_{i} = {i} x = 1 y = 2 }} }}\n"
        ));
    }
    benign.push_str("locator = { name = \"tail\" position = { @unsettled = 1 } }\n");
    benign.push_str("}\nentity = { name = \"after_many_blocks\" pdxmesh = \"m\" }\n");

    let mut pathological = String::from("entity = { name = \"many_live_vars\" pdxmesh = \"m\"\n");
    for i in 0..24 {
        pathological.push_str(&format!(
            "locator = {{ name = \"v{i}\" position = {{ @live_{i} = v_{i} x = 1 }} }}\n"
        ));
    }
    for i in 0..24 {
        pathological.push_str(&format!("unused_{i} = @live_{i}\n"));
    }
    pathological.push_str("}\nentity = { name = \"after_budget_overflow\" pdxmesh = \"m\" }\n");

    let roster = roster_of(
        "recovery-state-merge-perf",
        &[
            (
                "gfx/models/m.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            ("gfx/entities/benign.asset", benign.as_bytes()),
            ("gfx/entities/pathological.asset", pathological.as_bytes()),
        ],
        &[],
    );

    assert_eq!(
        roster
            .unreadable_names
            .get("converging_entity")
            .map(String::as_str),
        Some("gfx/entities/benign.asset")
    );
    assert_eq!(
        roster
            .unreadable_names
            .get("after_many_blocks")
            .map(String::as_str),
        Some("gfx/entities/benign.asset")
    );
    assert_eq!(
        roster
            .unreadable_names
            .get("after_budget_overflow")
            .map(String::as_str),
        Some("gfx/entities/pathological.asset")
    );
}

#[test]
fn recovery_handles_astra_r1_edge_cases() {
    use content_reader_layers::content::{ResolveOptions, visual};
    use std::sync::atomic::AtomicBool;

    let s1_scalar_game_data =
        b"entity = { state = { game_data = scalar } } name = recovered_after_scalar_game_data";
    let s2_balanced_retained = br#"
        entity = { locator = { position = { kept_from_balanced k = 1 } } name trailing }
        entity = { game_data = { atlas_size = { @u = 1 } } }
    "#;
    let s4_animation_first = br#"
        animation = { name = first_anim name = second_anim file = "a.anim" }
        entity = { game_data = { atlas_size = { @u = 1 } } }
    "#;
    let s4_mesh_first = br#"
        objectTypes = { pdxmesh = { name = first_mesh name = second_mesh file = "m.mesh" } }
        entity = { state = scalar }
    "#;
    let s5_known_one_block = br#"
        entity = { game_data = { atlas_size = { @u = 1 } } }
        entity = { name = unrelated_s5_mesh pdxmesh = { name = real_s5_entity } }
    "#;

    let roster = roster_of(
        "astra-r1-recovery",
        &[
            (
                "gfx/models/m.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            ("gfx/models/s4_mesh.gfx", s4_mesh_first),
            ("gfx/entities/s1.asset", s1_scalar_game_data),
            ("gfx/entities/s2.asset", s2_balanced_retained),
            ("gfx/entities/s4_anim.asset", s4_animation_first),
            ("gfx/entities/s5.asset", s5_known_one_block),
        ],
        &[],
    );

    for (name, file) in [
        ("recovered_after_scalar_game_data", "gfx/entities/s1.asset"),
        ("kept_from_balanced", "gfx/entities/s2.asset"),
        ("first_anim", "gfx/entities/s4_anim.asset"),
        ("first_mesh", "gfx/models/s4_mesh.gfx"),
        ("real_s5_entity", "gfx/entities/s5.asset"),
    ] {
        assert_eq!(
            roster.unreadable_names.get(name).map(String::as_str),
            Some(file),
            "expected {name} from {file}; refused={:?}",
            roster.unreadable_names
        );
    }

    for absent in ["second_anim", "second_mesh", "unrelated_s5_mesh"] {
        assert_eq!(
            roster.unreadable_names.get(absent),
            None,
            "did not expect {absent} to be refused; refused={:?}",
            roster.unreadable_names
        );
    }

    let root = std::env::temp_dir().join(format!(
        "content-reader-astra-r1-overlay-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    };
    let game_dir = root.join("game");
    let user_dir = root.join("user");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::create_dir_all(user_dir.join("mod")).unwrap();
    write(
        "base/gfx/models/m.gfx",
        b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
    );
    write(
        "base/gfx/entities/s6.asset",
        br#"entity = { name = "base_only" pdxmesh = "target_in_other_file" }"#,
    );
    write(
        "base/gfx/entities/other.asset",
        br#"entity = { name = "target_in_other_file" pdxmesh = "m" }"#,
    );
    write(
        "base/gfx/entities/s4_rec.asset",
        br#"animation = { name = "recorded_first" name = "recorded_second" file = "a.anim" }"#,
    );
    let overlay_dir = root.join("overlay");
    write(
        "overlay/gfx/entities/s6.asset",
        br#"entity = { name = "target_in_other_file" pdxmesh = "m" } }{ = ="#,
    );
    write(
        "overlay/gfx/entities/s4_rec.asset",
        br#"
            animation = { name = "recorded_second" file = "a.anim" }
            entity = { game_data = { atlas_size = { @u = 1 } } }
        "#,
    );
    let descriptor = format!(
        "name=\"VisualOverride\"\npath=\"{}\"\n",
        overlay_dir.display().to_string().replace('\\', "/")
    );
    write("user/mod/visual_override.mod", descriptor.as_bytes());
    let base = root.join("base");
    let mut layers = content_reader_layers::ResolvedLayers::from_folders(&[(&base, &[][..])]).unwrap();
    let options = ResolveOptions {
        game_dir,
        user_dir,
        cache_dir: root.join("cache"),
    };
    visual::apply(
        &mut layers,
        &options,
        &[visual::Choice {
            id: "mod/visual_override.mod".into(),
            name: "VisualOverride".into(),
        }],
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    let overlay_roster = Roster::read(layers.presentation()).unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        overlay_roster
            .unreadable_names
            .get("target_in_other_file")
            .map(String::as_str),
        Some("gfx/entities/s6.asset")
    );
    assert_eq!(overlay_roster.unreadable_names.get("base_only"), None);
    assert_eq!(
        overlay_roster
            .unreadable_names
            .get("recorded_second")
            .map(String::as_str),
        Some("gfx/entities/s4_rec.asset")
    );
    assert_eq!(overlay_roster.unreadable_names.get("recorded_first"), None);
}

#[test]
fn recovery_handles_astra_r2_edge_cases() {
    use content_reader_layers::content::{ResolveOptions, visual};
    use std::sync::atomic::AtomicBool;

    let s2_mixed_and_def = br#"
        entity = { locator = { position = { kept_r2 k = 1 } } name trailing }
        entity = { locator = { position = { phantom_from_def_list @v = 1 } } name trailing }
        entity = { game_data = { atlas_size = { @u = 1 } } }
    "#;
    let s4_quoted_unsettled_name = br#"
        animation = { file = "clip.anim" "name" { } }
        entity = { game_data = { atlas_size = { @u = 1 } } }
    "#;

    let roster = roster_of(
        "astra-r2-recovery",
        &[
            (
                "gfx/models/m.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            ("gfx/entities/s2_r2.asset", s2_mixed_and_def),
            ("gfx/entities/s4_r2.asset", s4_quoted_unsettled_name),
        ],
        &[],
    );

    assert_eq!(
        roster.unreadable_names.get("kept_r2").map(String::as_str),
        Some("gfx/entities/s2_r2.asset")
    );
    assert_eq!(
        roster.unreadable_names.get("phantom_from_def_list"),
        None,
        "balanced scan must decline block containing variable definition; refused={:?}",
        roster.unreadable_names
    );
    assert_eq!(
        roster.unreadable_names.get("clip.anim").map(String::as_str),
        Some("gfx/entities/s4_r2.asset"),
        "FirstExact must apply on Shape::Unsettled one-value branch; refused={:?}",
        roster.unreadable_names
    );

    let root = std::env::temp_dir().join(format!(
        "content-reader-astra-r2-overlay-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    };
    let game_dir = root.join("game");
    let user_dir = root.join("user");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::create_dir_all(user_dir.join("mod")).unwrap();
    write(
        "base/gfx/models/m.gfx",
        b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
    );
    write(
        "base/gfx/models/no_file_mesh.gfx",
        b"objectTypes = { pdxmesh = { name = target_mesh_no_file } }",
    );
    write(
        "base/gfx/entities/s6_r2.asset",
        br#"
            animation = { name = @walk file = "a.anim" }
            light = { name = @lamp intensity = 1 }
            animation = { name = target_no_file }
        "#,
    );
    let overlay_dir = root.join("overlay");
    write(
        "overlay/gfx/models/no_file_mesh.gfx",
        b"objectTypes = { pdxmesh = { name = target_mesh_no_file file = o.mesh } } entity = { state = scalar }",
    );
    write(
        "overlay/gfx/entities/s6_r2.asset",
        br#"
            animation = { name = @walk file = "a.anim" }
            light = { name = @lamp intensity = 2 }
            animation = { name = target_no_file file = "o.anim" }
            entity = { game_data = { atlas_size = { @u = 1 } } }
        "#,
    );
    let descriptor = format!(
        "name=\"VisualOverride\"\npath=\"{}\"\n",
        overlay_dir.display().to_string().replace('\\', "/")
    );
    write("user/mod/visual_override.mod", descriptor.as_bytes());
    let base = root.join("base");
    let mut layers = content_reader_layers::ResolvedLayers::from_folders(&[(&base, &[][..])]).unwrap();
    let options = ResolveOptions {
        game_dir,
        user_dir,
        cache_dir: root.join("cache"),
    };
    visual::apply(
        &mut layers,
        &options,
        &[visual::Choice {
            id: "mod/visual_override.mod".into(),
            name: "VisualOverride".into(),
        }],
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    let overlay_roster = Roster::read(layers.presentation()).unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(overlay_roster.unreadable_names.get("@walk"), None);
    assert_eq!(overlay_roster.unreadable_names.get("@lamp"), None);
    assert_eq!(
        overlay_roster
            .unreadable_names
            .get("target_no_file")
            .map(String::as_str),
        Some("gfx/entities/s6_r2.asset")
    );
    assert_eq!(
        overlay_roster
            .unreadable_names
            .get("target_mesh_no_file")
            .map(String::as_str),
        Some("gfx/models/no_file_mesh.gfx")
    );
}

#[test]
fn recovery_handles_astra_r3_edge_cases() {
    use content_reader_layers::content::{ResolveOptions, visual};
    use std::sync::atomic::AtomicBool;

    let root = std::env::temp_dir().join(format!(
        "content-reader-astra-r3-overlay-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    };
    let game_dir = root.join("game");
    let user_dir = root.join("user");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::create_dir_all(user_dir.join("mod")).unwrap();
    write(
        "base/gfx/models/m.gfx",
        b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
    );
    write(
        "base/gfx/entities/00_bad.asset",
        br#"entity = { name = "order_uncertain" pdxmesh = "m" game_data = { atlas_size = { @u = 1 } } }"#,
    );
    write(
        "base/gfx/entities/0_other.asset",
        br#"
            entity = { name = "dropped_clone_target" pdxmesh = "m" }
            entity = { name = "chain_parent" pdxmesh = "m" }
            entity = { name = "order_parent" pdxmesh = "m" }
            entity = { name = "order_uncertain" pdxmesh = "m" }
        "#,
    );
    write(
        "base/gfx/entities/1_clone.asset",
        br#"
            entity = { name = "dropped_clone_target" clone = "absent_parent" }
            entity = { name = "chain_child" clone = "chain_parent" }
            entity = { name = "chain_grandchild" clone = "chain_child" }
        "#,
    );
    write(
        "base/gfx/entities/2_order.asset",
        br#"
            entity = { name = "order_child" clone = "order_parent" }
            entity = { name = "order_parent" clone = "order_uncertain" }
        "#,
    );
    let overlay_dir = root.join("overlay");
    write(
        "overlay/gfx/entities/1_clone.asset",
        br#"
            entity = { name = "dropped_clone_target" pdxmesh = "m" game_data = { atlas_size = { @u = 1 } } }
            entity = { name = "chain_parent" pdxmesh = "m" }
            entity = { name = "chain_child" clone = "chain_parent" }
            entity = { name = "chain_grandchild" clone = "chain_child" }
        "#,
    );
    write(
        "overlay/gfx/entities/2_order.asset",
        br#"
            entity = { name = "order_child" clone = "order_parent" game_data = { atlas_size = { @u = 1 } } }
            entity = { name = "order_parent" clone = "order_uncertain" }
        "#,
    );
    let descriptor = format!(
        "name=\"VisualOverride\"\npath=\"{}\"\n",
        overlay_dir.display().to_string().replace('\\', "/")
    );
    write("user/mod/visual_override.mod", descriptor.as_bytes());
    let base = root.join("base");
    let mut layers = content_reader_layers::ResolvedLayers::from_folders(&[(&base, &[][..])]).unwrap();
    let options = ResolveOptions {
        game_dir,
        user_dir,
        cache_dir: root.join("cache"),
    };
    visual::apply(
        &mut layers,
        &options,
        &[visual::Choice {
            id: "mod/visual_override.mod".into(),
            name: "VisualOverride".into(),
        }],
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    let overlay_roster = Roster::read(layers.presentation()).unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        overlay_roster
            .unreadable_names
            .get("dropped_clone_target")
            .map(String::as_str),
        Some("gfx/entities/1_clone.asset"),
        "dropped recorded clone must not exempt overlay declaration; refused={:?}",
        overlay_roster.unreadable_names
    );
    assert!(
        overlay_roster
            .refused_declaration("dropped_clone_target")
            .is_some()
    );
    for descendant in ["chain_parent", "chain_child", "chain_grandchild"] {
        assert_eq!(
            overlay_roster
                .unreadable_names
                .get(descendant)
                .map(String::as_str),
            Some("gfx/entities/1_clone.asset"),
            "same-file clone refusal must propagate to {descendant}; refused={:?}",
            overlay_roster.unreadable_names
        );
        assert!(overlay_roster.refused_declaration(descendant).is_some());
    }
    assert_eq!(
        overlay_roster
            .unreadable_names
            .get("order_parent")
            .map(String::as_str),
        Some("gfx/entities/00_bad.asset")
    );
    assert!(overlay_roster.refused_declaration("order_parent").is_some());
    assert_eq!(
        overlay_roster.unreadable_names.get("order_child"),
        None,
        "later refusal in same file must not propagate backward to earlier clone; refused={:?}",
        overlay_roster.unreadable_names
    );
    assert_eq!(overlay_roster.refused_declaration("order_child"), None);

    let mut many_short_positions =
        String::from("entity = { name = \"many_short_positions\" pdxmesh = \"m\"\n");
    for _ in 0..4_000 {
        many_short_positions.push_str("locator = { position = { x = 1 } }\n");
    }
    many_short_positions.push_str("game_data = { atlas_size = { @u = 1 } }\n}\n");
    let perf_roster = roster_of(
        "astra-r3-many-short-positions",
        &[
            (
                "gfx/models/m.gfx",
                b"objectTypes = { pdxmesh = { name = m file = m.mesh } }",
            ),
            (
                "gfx/entities/many_positions.asset",
                many_short_positions.as_bytes(),
            ),
        ],
        &[],
    );
    assert_eq!(
        perf_roster
            .unreadable_names
            .get("many_short_positions")
            .map(String::as_str),
        Some("gfx/entities/many_positions.asset")
    );
}
