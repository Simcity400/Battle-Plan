use super::*;
use crate::mesh_pose;

pub fn rig(scale: f32, attached: bool, native: bool) -> Rig {
    let animation = |reverse: bool| {
        fn floats(bytes: &mut Vec<u8>, name: &str, values: &[f32]) {
            bytes.extend([b'!', name.len() as u8]);
            bytes.extend(name.bytes());
            bytes.push(b'f');
            bytes.extend((values.len() as u32).to_le_bytes());
            bytes.extend(values.iter().flat_map(|value| value.to_le_bytes()));
        }
        let mut bytes = mesh::MAGIC.to_vec();
        bytes.extend(b"[info\0");
        floats(&mut bytes, "fps", &[5.]);
        bytes.extend(b"!\x02sai\x01\0\0\0\x06\0\0\0");
        for name in if reverse {
            ["tip", "root"]
        } else {
            ["root", "tip"]
        } {
            bytes.extend(b"[[");
            bytes.extend(name.bytes());
            bytes.push(0);
            bytes.extend(b"!\x02sas\x01\0\0\0\x04\0\0\0tqs\0");
        }
        bytes.extend(b"[samples\0");
        let phase = if reverse { 0.7 } else { 0.1 };
        floats(
            &mut bytes,
            "t",
            &(0..12)
                .flat_map(|i| [i as f32 * 0.13 + phase, i as f32 * -0.05, 0.3])
                .collect::<Vec<_>>(),
        );
        floats(
            &mut bytes,
            "q",
            &(0..12)
                .flat_map(|i| [0.2, (i as f32 * 0.17 + phase).sin(), 0.3, 0.9])
                .collect::<Vec<_>>(),
        );
        floats(
            &mut bytes,
            "s",
            &(0..12).map(|i| 0.7 + i as f32 * 0.03).collect::<Vec<_>>(),
        );
        Arc::new(mesh::read_animation(&bytes).unwrap())
    };
    let skeleton = ["root", "tip"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| mesh::Joint {
            name: name.into(),
            index: i as u32,
            parent: (i > 0).then_some(0),
            inverse_bind: Some(Transform::from_trs(
                [-(i as f32), 0., 0.],
                [0., 0., 0., 1.],
                1.,
            )),
        })
        .collect();
    let node = Node {
        mesh: Arc::new(mesh::Mesh {
            skeleton,
            ..Default::default()
        }),
        native: native.then(|| {
            Arc::new(mesh_pose::Source {
                pdxmesh: true,
                skeleton: ["root", "tip"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| mesh::NativeJoint {
                        name: name.into(),
                        parent: if i == 0 { 255 } else { 0 },
                        inverse_bind: [1., 0., 0., 0., 1., 0., 0., 0., 1., -(i as f32), 0., 0.],
                    })
                    .collect(),
                entity_locators: Vec::new(),
                binary_locators: Vec::new(),
                mesh_scale: 0.75,
                entity_scale: scale,
                frozen: None,
                game_data: None,
            })
        }),
        clips: [("idle", false), ("move", true), ("attack", false)]
            .into_iter()
            .map(|(name, reverse)| Clip {
                state: EntityState {
                    name: name.into(),
                    chance: 1.,
                    speed: if reverse { 1.3 } else { 0.7 },
                    start: 0.125,
                    blend: 0.8,
                    looping: name != "attack",
                    next_state: (name == "attack").then(|| "idle".into()),
                    events: vec![mesh::EntityEvent {
                        node: "tip".into(),
                        particle: Some("smoke".into()),
                        time: 0.1,
                        keep_particle: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                animation: Some(animation(reverse)),
            })
            .collect(),
        default: "idle".into(),
        follows_parent: false,
        parent: None,
        scale,
        mesh_scale: 0.75,
        skin_base: Some(1),
        rigid: 0,
        locators: BTreeMap::new(),
    };
    let mut rig = Rig {
        nodes: vec![node],
        start: 0,
        end: 3,
    };
    if attached {
        let mut child = rig.nodes[0].clone();
        child.parent = Some((0, "tip".into(), "child".into()));
        child.follows_parent = true;
        child.rigid = 3;
        child.skin_base = Some(4);
        rig.nodes.push(child);
        rig.end = 6;
    }
    rig
}
