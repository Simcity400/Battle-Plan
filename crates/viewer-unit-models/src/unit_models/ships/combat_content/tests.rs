use std::sync::Arc;

use super::*;
use viewer_models::{entity_animation::Node, mesh_pose::Source};

fn read(bytes: &[u8]) -> Config {
    Config::read(bytes, &mut Vec::new())
}

fn registered(name: &str) -> mesh::Descriptors {
    let mut descriptors = mesh::Descriptors::default();
    descriptors.entities.insert(
        name.to_owned(),
        mesh::EntityDefinition {
            name: name.to_owned(),
            ..Default::default()
        },
    );
    descriptors
}

#[test]
fn naval_combat_reader_keeps_native_defaults_and_independent_formation_projection() {
    let config = read(b"");
    assert_eq!(config.formation().unwrap(), (String::new(), 1.0));
    assert_eq!(config.combat_scale, 1.0);
    assert_eq!(config.random_start_time, 1.0);
    assert_eq!(config.death_time, 10.0);
    assert!(config.effects.iter().all(Vec::is_empty));

    let config = read(
        b"naval_formation = ordinary naval_formation_scale = .5 \
          naval_combat_formation = \"\xfe\" naval_combat_formation_scale = nan \
          naval_hit_effects_big = { \"\xfe\" }",
    );
    assert_eq!(config.formation().unwrap(), ("ordinary".into(), 0.5));
    assert!(
        CombatAssets::load(&registered("ordinary"), &config)
            .unwrap_err()
            .to_string()
            .contains("1 refused naval_combat_formation")
    );
}

#[test]
fn naval_combat_reader_overwrites_scalars_but_appends_resolved_effect_lists() {
    let mut warnings = Vec::new();
    let config = Config::read(
        br#"
            @entity = combat
            @effect = resolved
            naval_combat_formation = first naval_combat_formation = @entity
            naval_combat_formation_scale = .75 naval_combat_formation_scale = bad
            naval_death_time = 12 naval_random_start_time = 0x1.8p0
            naval_hit_effects_big = { first @effect first }
            naval_hit_effects_big = { last }
            naval_miss_effects_small = { "a
b" }
            unknown = { naval_death_time = 99 }
            } naval_death_time = 100
        "#,
        &mut warnings,
    );
    let assets = CombatAssets::load(&registered("combat"), &config).unwrap();
    assert_eq!(assets.template_entity, "combat");
    assert_eq!(assets.scale, 0.75);
    assert_eq!(assets.death_time, 12.0);
    assert_eq!(assets.random_start_time, 1.5);
    assert_eq!(assets.effects[0], ["first", "resolved", "first", "last"]);
    assert!(assets.effects[1].is_empty());
    assert!(assets.effects[2].is_empty());
    assert_eq!(assets.effects[3], ["Unreadable String"]);
    for fragment in ["no float stored", "effect-list newline", "unexpected token"] {
        assert!(
            warnings
                .iter()
                .any(|(_, warning)| warning.contains(fragment)),
            "{fragment}: {warnings:?}"
        );
    }
}

#[test]
fn naval_combat_reader_replays_native_list_and_failed_float_consumption() {
    let config = read(
        b"naval_hit_effects_small = ignored naval_formation = list_token } \
          naval_combat_formation = kept \
          naval_death_time = { naval_death_time = 99 } naval_random_start_time = .25",
    );
    assert_eq!(config.formation().unwrap().0, "");
    assert_eq!(
        config.effects[1],
        [
            b"naval_formation".to_vec(),
            b"=".to_vec(),
            b"list_token".to_vec()
        ]
    );
    let assets = CombatAssets::load(&registered("kept"), &config).unwrap();
    assert_eq!(assets.death_time, 10.0);
    assert_eq!(assets.random_start_time, 0.25);
}

#[test]
fn naval_combat_fields_refuse_only_unrepresentable_stored_values() {
    for field in [
        "naval_combat_formation_scale",
        "naval_death_time",
        "naval_random_start_time",
    ] {
        let config = read(format!("naval_combat_formation = rig {field} = 1e39").as_bytes());
        let reason = CombatAssets::load(&registered("rig"), &config)
            .unwrap_err()
            .to_string();
        assert!(reason.contains(&format!("1 refused {field}")), "{reason}");
    }
    let config = read(
        b"naval_combat_formation = rig naval_combat_formation_scale = -.5 \
          naval_death_time = 0 naval_random_start_time = -2",
    );
    let assets = CombatAssets::load(&registered("rig"), &config).unwrap();
    assert_eq!(
        (assets.scale, assets.death_time, assets.random_start_time),
        (-0.5, 0.0, -2.0)
    );

    let config = read(b"naval_combat_formation = rig naval_miss_effects_big = { \"\xfe\" }");
    assert!(
        CombatAssets::load(&registered("rig"), &config)
            .unwrap_err()
            .to_string()
            .contains("1 refused naval_miss_effects_big")
    );
}

#[test]
fn naval_combat_template_uses_the_entity_registry_and_supports_meshless_rigs() {
    let config = read(b"naval_combat_formation = rig");
    let mut descriptors = registered("rig");
    assert!(CombatAssets::load(&descriptors, &config).is_ok());
    descriptors.entities.remove("rig");
    assert!(CombatAssets::load(&descriptors, &config).is_err());
}

fn rig(names: &[&str]) -> Rig {
    Rig {
        nodes: vec![Node {
            mesh: Arc::new(mesh::Mesh::default()),
            native: Some(Arc::new(Source {
                pdxmesh: true,
                skeleton: names
                    .iter()
                    .map(|name| mesh::NativeJoint {
                        name: (*name).to_owned(),
                        parent: 255,
                        inverse_bind: [0.0; 12],
                    })
                    .collect(),
                entity_locators: Vec::new(),
                binary_locators: Vec::new(),
                mesh_scale: 1.0,
                entity_scale: 1.0,
                frozen: None,
                game_data: None,
            })),
            clips: Vec::new(),
            default: "idle".to_owned(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
        ..Default::default()
    }
}

const POSITIONS: [&str; 10] = [
    "defend_position_big_5",
    "defend_position_big_2",
    "defend_position_small_9",
    "defend_position_small_2",
    "defend_position_small_4",
    "defend_position_small_5",
    "defend_position_small_8",
    "attack_position_small_1",
    "attack_position_small_3",
    "attack_position_small_7",
];

#[test]
fn naval_combat_positions_keep_numbered_holes_and_choose_distinct_supports() {
    let rig = rig(&POSITIONS);
    let sides = select_locators(&rig).unwrap();
    assert_eq!(sides[0].big.as_ref().unwrap().name, "defend_position_big_2");
    assert_eq!(
        mesh_pose::lookup(
            rig.nodes[0].native.as_ref().unwrap(),
            "defend_position_big_2",
            mesh_pose::LookupMode::Naval
        ),
        Some(mesh_pose::Binding::Joint(1))
    );
    assert!(sides[1].big.is_none());
    assert_eq!(
        sides[0]
            .small
            .iter()
            .map(|position| position.name.as_str())
            .collect::<Vec<_>>(),
        [
            "defend_position_small_2",
            "defend_position_small_4",
            "defend_position_small_5",
            "defend_position_small_8",
            "defend_position_small_9"
        ]
    );
    let mut draws = [4, 0, 0].into_iter();
    let chosen = sides[0].choose(&mut || draws.next().unwrap()).unwrap();
    assert_eq!(
        chosen.map(|position| position.unwrap().name),
        [
            "defend_position_big_2",
            "defend_position_small_9",
            "defend_position_small_2",
            "defend_position_small_8"
        ]
    );
    assert!(draws.next().is_none());
}

#[test]
fn naval_combat_admission_uses_shared_entity_binary_and_joint_lookup() {
    let mut rig = rig(&POSITIONS);
    let point = |name: &str, x: f32| mesh_pose::Locator {
        name: name.into(),
        local: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, 3.0, 2.0, 1.0],
        ],
        parent: None,
        error: None,
    };
    let source = Arc::make_mut(rig.nodes[0].native.as_mut().unwrap());
    source.entity_locators = vec![
        point("defend_position_big_2", 4.0),
        point("defend_position_big_2", 5.0),
    ];
    source.binary_locators = vec![
        point("defend_position_big_2", 20.0),
        point("attack_position_small_7", 30.0),
    ];
    source
        .skeleton
        .retain(|joint| joint.name != "attack_position_small_7");
    let sides = select_locators(&rig).unwrap();
    assert_eq!(sides[0].big.as_ref().unwrap().name, "defend_position_big_2");
    assert_eq!(
        sides[1].small.last().unwrap().name,
        "attack_position_small_7"
    );
    let source = rig.nodes[0].native.as_ref().unwrap();
    let native = |name| {
        mesh_pose::lookup(source, name, mesh_pose::LookupMode::Naval)
            .unwrap()
            .local(Ok(None))
            .unwrap()
    };
    assert_eq!(native("defend_position_big_2")[3][0], 4.0);
    assert_eq!(native("attack_position_small_7")[3][0], 30.0);

    Arc::make_mut(rig.nodes[0].native.as_mut().unwrap())
        .entity_locators
        .clear();
    assert_eq!(
        select_locators(&rig).unwrap()[0].big.as_ref().unwrap().name,
        "defend_position_big_2"
    );
    assert_eq!(
        mesh_pose::lookup(
            rig.nodes[0].native.as_ref().unwrap(),
            "defend_position_big_2",
            mesh_pose::LookupMode::Naval
        )
        .unwrap()
        .local(Ok(None))
        .unwrap()[3][0],
        20.0
    );
}

#[test]
fn naval_combat_admission_refuses_only_reached_locator_origin_errors() {
    let mut rig = rig(&POSITIONS);
    let source = Arc::make_mut(rig.nodes[0].native.as_mut().unwrap());
    let bad = |name: &str| mesh_pose::Locator {
        name: name.into(),
        local: mesh_pose::IDENTITY,
        parent: None,
        error: Some(format!(
            "native entity locator {name:?}: 1 refused parent ordinal from changed origin"
        )),
    };
    source.entity_locators = vec![bad("defend_position_big_5")];
    assert!(select_locators(&rig).is_ok());
    Arc::make_mut(rig.nodes[0].native.as_mut().unwrap())
        .entity_locators
        .push(bad("defend_position_big_2"));
    let reason = select_locators(&rig).unwrap_err().to_string();
    assert!(reason.contains("defend_position_big_2"), "{reason}");
    assert!(reason.contains("1 refused parent ordinal"), "{reason}");
}

#[test]
fn naval_combat_refuses_unsafe_short_support_arrays_without_consuming_rng() {
    let short = rig(&["defend_position_small_1", "defend_position_small_2"]);
    let reason = select_locators(&short).unwrap_err().to_string();
    assert!(reason.contains("2 support positions"), "{reason}");
    let side = SideLocators {
        big: None,
        small: Vec::new(),
    };
    assert!(
        side.choose(&mut || panic!("unsafe selection must not read RNG"))
            .is_err()
    );
}
