#[cfg(feature = "test-support")]
use super::descriptors::last_text;
use super::*;

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
        .map(String::from),
    );
    let resolve = |authored| index.resolve("gfx/models/units", authored);
    assert_eq!(resolve("nospec.dds"), "gfx/models/nospec.dds");
    assert_eq!(
        resolve("a_diffuse.tga"),
        "gfx/models/buildings/a_diffuse.dds"
    );
    assert_eq!(resolve("Nospec.dds"), "gfx/models/units/Nospec.dds");
    assert_eq!(resolve("upper.DDS"), "gfx/models/units/upper.DDS");
    assert_eq!(resolve("shared.dds"), "gfx/models/x/shared.dds");
    // Enumeration order: a directory's files before its subdirectories.
    let order = crate::defines::load_order(
        "gfx/models/",
        "",
        [
            "gfx/models/a/deep/t.dds",
            "gfx/models/a/t.dds",
            "gfx/models/b.dds",
        ]
        .into_iter(),
    );
    assert_eq!(
        TextureIndex::new(order).resolve("", "t.dds"),
        "gfx/models/a/t.dds"
    );
}

/// A stack's descriptor files, the overlay's second.
#[cfg(feature = "test-support")]
pub(super) fn roster_of(name: &str, base: &[(&str, &[u8])], overlay: &[(&str, &[u8])]) -> Roster {
    let root = std::env::temp_dir().join(format!("installed-content-roster-{name}-{}", std::process::id()));
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
        crate::ResolvedLayers::from_folders(&[(&under, &[][..]), (&over, &[][..])]).unwrap();
    let roster = Roster::read(&layers).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    roster
}

/// The roster reads every `gfx/**/*.asset` in recursive directory
/// order (each directory's files, then its subdirectories, across
/// layers); a clone copies its parent as then registered or is dropped,
/// and a repeated name is replaced unless its `version` is lower than the
/// stored one, a clone storing its parent's.
#[cfg(feature = "test-support")]
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
    let entities = &roster.descriptors.entities;
    let mesh = |name: &str| entities[name].mesh.as_deref();
    assert!(!entities.contains_key("early"));
    assert_eq!(roster.logged.len(), 1, "{:?}", roster.logged);
    assert!(roster.logged[0].starts_with("early in gfx/entities/a.asset"));
    assert_eq!(mesh("after_first"), Some("f"));
    assert_eq!(mesh("base"), Some("n"));
    assert_eq!(entities["base"].scale, None);
    // The clone kept the base it copied, not the later one.
    assert_eq!(mesh("child"), Some("m"));
    assert_eq!(entities["child"].scale, Some(2.0));
    assert_eq!(entities["child"].clone, None);
    assert_eq!(mesh("cross"), Some("o"));
    assert_eq!(entities["cross"].scale, Some(2.0));
    assert_eq!(entities["cross"].attachments.len(), 1);
    assert_eq!(mesh("v"), Some("o"));
    // "vc" stored v's version 2, so its version 1 redeclaration is kept out.
    assert_eq!(mesh("vc"), Some("o"));
    assert!(!entities.contains_key("quoted"));
    assert!(roster.unreadable.is_empty());
}

/// A clone of a name an unreadable file holds may copy that file's
/// declaration, so it may be declared there too, and so may its clones.
#[cfg(feature = "test-support")]
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
        roster.descriptors.entities.keys().collect::<Vec<_>>(),
        ["plain"]
    );
}

/// A clone's attachments replace the parent's of the same name where
/// they stand and append new ones. Parent handles 1, 2, 4, 3 keep that
/// order when the clone declares them 1, 4, 2, 3. The replacement changes
/// each held value without moving its slot. An unnamed one matches by
/// joint. A mesh override no `pdxmesh` registers keeps the parent's
/// mesh and is logged; a redefinition's keeps its
/// own.
#[cfg(feature = "test-support")]
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
    let entities = &roster.descriptors.entities;
    let slots: Vec<_> = entities["child"]
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
    assert_eq!(entities["child"].mesh.as_deref(), Some("m"));
    assert_eq!(entities["plain"].mesh.as_deref(), Some("n"));
    assert!(
        entities
            .values()
            .all(|entity| entity.unbound_mesh.is_none())
    );
    assert_eq!(roster.logged.len(), 2, "{:?}", roster.logged);
    assert!(roster.logged[0].starts_with("child in gfx/entities/a.asset: no pdxmesh missing"));
}

#[cfg(feature = "test-support")]
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
            roster.descriptors.entities[name].unbound_mesh.as_deref(),
            Some("absent")
        );
    }
    for name in ["bare", "cleared", "repaired"] {
        assert!(roster.descriptors.entities[name].unbound_mesh.is_none());
    }
    for name in ["empty", "child", "bare", "cleared"] {
        assert!(roster.descriptors.entities[name].mesh.is_none());
    }
    assert_eq!(
        roster.descriptors.entities["repaired"].mesh.as_deref(),
        Some("m")
    );
}

/// The last `.gfx` object of a name is the one the engine finds, and only
/// a `pdxmesh` is a mesh; `.gfx` files declare no entities and `.asset`
/// files no meshes.
#[cfg(feature = "test-support")]
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
    assert!(roster.descriptors.entities.is_empty());
}

/// An animation name keeps the last declaration whose file loads, and its
/// file is always beside the descriptor; a light keeps its last.
#[cfg(feature = "test-support")]
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
    // The last light of a name rewrites the record.
    let lamp = &roster.lights["lamp"];
    assert_eq!(last_text(lamp, "intensity").as_deref(), Some("2"));
}

#[test]
fn lexical_names_keep_quoted_and_bare_words_outside_comments() {
    // A bare CR ends a comment as the parser's lexer ends it, and a
    // quoted name's escapes are decoded as a parsed name's are.
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

/// Builds a container by hand, so the tests do not depend on installed content.
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

/// One shape, one LOD, one triangle, with every stream the tree pass reads.
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
    // `aabb` and `material` are depth 4, so they are children of `mesh`.
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
        // Four slots for the one vertex, whatever `bones` says.
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
    // `tx` is the world inverse bind, so inverting it is the bind pose.
    assert_eq!(mesh.bind_pose("hand").unwrap().translation, [2.0, 3.0, 0.0]);
    // A joint the file gives no `tx` has no bind pose at all.
    assert_eq!(mesh.skeleton[0].inverse_bind, None);
    assert_eq!(mesh.bind_pose("root"), None);
    let skin = mesh.shapes[0].meshes[0].skin.as_ref().unwrap();
    assert_eq!(skin.bones, 2);
    assert_eq!(skin.joints, vec![[1, 0, -1, -1]]);
    // The unused slots are dropped and the `-1` never reaches a palette.
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
        // `bones = 1` does not make the stream one wide.
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
    // A property type byte the container does not define.
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"!\x01xq\x01\x00\x00\x00\x00");
    assert!(parse(&bytes).is_err());
    // A count that runs past the end.
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"!\x01xi\xff\xff\xff\x0f");
    assert!(parse(&bytes).is_err());
    // A node name with no terminator.
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"[object");
    assert!(parse(&bytes).is_err());
    // A depth that skips a level.
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(b"[[deep\x00");
    assert!(parse(&bytes).is_err());
    // A byte that starts no token.
    let mut bytes = MAGIC.to_vec();
    bytes.push(b'?');
    assert!(parse(&bytes).is_err());
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

    // A stream the file omits entirely is a collision hull, not an error.
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
    // A cyclic clone chain terminates instead of looping.
    assert!(descriptors.resolve("loop_a").is_none());
    // An entity that declares its own mesh still inherits the clone's
    // scale from the parent whose mesh it overrides.
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
        // The child's handle replaces the parent's where it stands.
        vec![
            ("artillery", "generic_gun_entity"),
            ("infantry", "GER_crew_entity"),
        ]
    );
    assert_eq!(german.states["idle"][0].animation, "idle");
    // The default state picks the animation id, the mesh the animation
    // name and the descriptor the file.
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
    // A quarter turn about +Y at scale 2: +X lands on -Z.
    let half = std::f32::consts::FRAC_PI_4;
    let transform = Transform::from_trs([1.0, 0.0, 0.0], [0.0, half.sin(), 0.0, half.cos()], 2.0);
    let point = transform.transform_point([1.0, 0.0, 0.0]);
    assert!((point[0] - 1.0).abs() < 1e-5, "{point:?}");
    assert!(point[1].abs() < 1e-5, "{point:?}");
    assert!((point[2] + 2.0).abs() < 1e-5, "{point:?}");
    assert!((transform.uniform_scale() - 2.0).abs() < 1e-5);
    // A zero scale is the switch that hides an attachment, and it has no
    // inverse.
    let hidden = Transform::from_trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], 0.0);
    assert_eq!(hidden.uniform_scale(), 0.0);
    assert!(hidden.inverse().is_none());
    // So does a matrix whose entries are not finite.
    let wild = Transform::from_columns(&[1e30; 12]).unwrap();
    assert!(wild.inverse().is_none());
}

/// Two joints, three frames. `root` animates nothing, `slider` animates
/// `t` and `q`, so only `slider` appears in the `[samples]` streams.
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
        // Frame-major: frame 0 repeats the `[info]` values, then the
        // joint walks along +x while its other coordinates stay fixed.
        .floats(
            "t",
            &[
                1.0, 0.0, 2.0, // frame 0
                2.0, 0.0, 2.0, // frame 1
                3.0, 0.0, 2.0, // frame 2
            ],
        )
        .floats(
            "q",
            &[
                0.0, 0.0, 0.0, 1.0, // frame 0
                0.0, 0.0, 0.0, 1.0, // frame 1
                0.0, 0.0, 0.0, 1.0, // frame 2
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
    // One joint declares `t` and `q`, none declares `s`.
    assert_eq!(animation.channel_joints(), [1, 1, 0]);
    // Frame 0 is the `[info]` block, which is what a still hour draws.
    assert_eq!(
        animation.local("slider", FIRST_FRAME),
        animation.rest("slider")
    );
    assert_eq!(
        animation.local("slider", 2).unwrap().translation,
        [3.0, 0.0, 2.0]
    );
    // A channel the joint does not animate keeps its rest value at every
    // frame, and so does every channel of a joint with no channels at all.
    assert!((animation.local("slider", 2).unwrap().uniform_scale() - 0.5).abs() < 1e-5);
    assert_eq!(animation.local("root", 2), animation.rest("root"));
    // A frame past the end falls back to the rest pose rather than
    // reading another joint's samples.
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

/// A two-joint arm: `root` at the origin and `hand` bound two units out
/// along +x, with one vertex at the hand.
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
    // An animation that lifts the hand a unit and leaves the root alone.
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
    // The joint is where the animation puts it, which is where an
    // attachment on it hangs.
    assert_eq!(
        mesh.pose(&animation, FIRST_FRAME, "hand")
            .unwrap()
            .translation,
        [2.0, 1.0, 0.0]
    );
    // And the vertex bound to it moves with it: bind (2, 0, 0) to
    // (2, 1, 0), the same one unit up.
    let palette = mesh.palette(&animation, FIRST_FRAME);
    assert_eq!(palette.len(), 2);
    let skin = mesh.shapes[0].meshes[0].skin.as_ref().unwrap();
    assert_eq!(skin.influences(0), vec![(1, 1.0)]);
    assert_eq!(palette[1].transform_point([2.0, 0.0, 0.0]), [2.0, 1.0, 0.0]);
    // The root is unmoved, so its own matrix is the identity.
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
    // An all-zero `tx` and an overflowing diagonal both lack a usable
    // inverse, so each takes the same identity fallback.
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
