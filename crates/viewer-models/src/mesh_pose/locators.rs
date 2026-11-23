
use std::io;

use content_reader_mesh::mesh::{self, JointPose, NativeJoint};

use super::{Matrix, Source, local, multiply};

#[derive(Debug, Clone, PartialEq)]
pub struct Locator {
    pub name: String,
    pub local: Matrix,
    pub parent: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupMode {
    Attachment,
    AttachmentDummy,
    Naval,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Binding<'a> {
    Locator(&'a Locator),
    Joint(usize),
}

impl Binding<'_> {
    pub fn local(self, joints: Result<Option<&[Matrix]>, String>) -> Result<Matrix, String> {
        match self {
            Self::Locator(locator) => {
                if let Some(why) = &locator.error {
                    return Err(why.clone());
                }
                let Some(parent) = locator.parent else {
                    return Ok(locator.local);
                };
                match joints? {
                    Some(joints) => joints
                        .get(parent)
                        .map(|joint| multiply(*joint, locator.local))
                        .ok_or_else(|| {
                            format!(
                                "native locator {:?}: parent ordinal {parent} is absent from the played palette",
                                locator.name
                            )
                        }),
                    None => Ok(locator.local),
                }
            }
            Self::Joint(index) => match joints? {
                None => Ok(super::IDENTITY),
                Some(joints) => joints.get(index).copied().ok_or_else(|| {
                    format!(
                        "native attachment joint ordinal {index} is absent from the played palette"
                    )
                }),
            },
        }
    }
}

fn latin1_equals_bytes(stored: &str, query: &[u8]) -> bool {
    stored
        .chars()
        .map(u32::from)
        .eq(query.iter().map(|byte| u32::from(*byte)))
}

fn script_parent(skeleton: &[NativeJoint], name: &str) -> Option<usize> {
    let query = content_reader_parser::parser::c_string(name.as_bytes());
    skeleton
        .iter()
        .position(|joint| latin1_equals_bytes(&joint.name, query))
}

fn invalid(name: &str, field: &str, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("binary mesh locator {name:?}: 1 refused {field}: {why}"),
    )
}

fn floats<const N: usize>(node: &mesh::Node, field: &str) -> io::Result<[f32; N]> {
    node.property(field)
        .and_then(mesh::Value::floats)
        .and_then(|values| values.get(..N))
        .and_then(|values| values.try_into().ok())
        .ok_or_else(|| {
            invalid(
                &node.name,
                field,
                "missing, nonfloat or short payload would cause a native overread",
            )
        })
}

pub fn binary(
    root: &mesh::Node,
    skeleton: &[NativeJoint],
    mesh_scale: f32,
) -> io::Result<Vec<Locator>> {
    let Some(block) = root.child("locator") else {
        return Ok(Vec::new());
    };
    block
        .children
        .iter()
        .map(|node| {
            if node.name.chars().count() > 63 || node.name.chars().any(|c| u32::from(c) > 255) {
                return Err(invalid(
                    &node.name,
                    "name",
                    "not a terminated native Latin1 name of at most 63 bytes",
                ));
            }
            let position = floats::<3>(node, "p")?;
            let rotation = floats::<4>(node, "q")?;
            let parent = node
                .property("pa")
                .map(|value| {
                    value
                        .strings()
                        .and_then(|values| values.first())
                        .map(|name| skeleton.iter().position(|joint| joint.name == *name))
                        .ok_or_else(|| {
                            invalid(
                                &node.name,
                                "pa",
                                "present but nonstring or empty payload would cause a native overread",
                            )
                        })
                })
                .transpose()?
                .flatten();
            Ok(Locator {
                name: node.name.clone(),
                local: local(
                    JointPose {
                        translation: position,
                        rotation,
                        scale: 1.0,
                    },
                    mesh_scale,
                ),
                parent,
                error: None,
            })
        })
        .collect()
}

pub fn entity(
    entries: &[(String, mesh::Locator)],
    skeleton: &[NativeJoint],
    current_mesh: Option<&str>,
) -> Vec<Locator> {
    entries
        .iter()
        .map(|(name, locator)| {
            let error = (locator.parent_joint.is_some()
                && locator.origin_mesh.as_deref() != current_mesh)
                .then(|| {
                    format!(
                        "native entity locator {name:?}: 1 refused parent ordinal from registration mesh {:?} after pdxmesh changed to {current_mesh:?}",
                        locator.origin_mesh
                    )
                });
            let parent = if error.is_none() {
                locator.parent_joint.as_deref().and_then(|name| script_parent(skeleton, name))
            } else {
                None
            };
            let transform = locator.transform;
            Locator {
                name: name.clone(),
                local: [
                    [transform.basis[0][0], transform.basis[0][1], transform.basis[0][2], 0.0],
                    [transform.basis[1][0], transform.basis[1][1], transform.basis[1][2], 0.0],
                    [transform.basis[2][0], transform.basis[2][1], transform.basis[2][2], 0.0],
                    [transform.translation[0], transform.translation[1], transform.translation[2], 1.0],
                ],
                parent,
                error,
            }
        })
        .collect()
}

pub fn lookup<'a>(source: &'a Source, name: &str, mode: LookupMode) -> Option<Binding<'a>> {
    let name = name.split('\0').next().unwrap_or_default();
    let entity = || {
        match mode {
            LookupMode::Attachment | LookupMode::AttachmentDummy => {
                source.entity_locators.iter().rev().find(|l| l.name == name)
            }
            LookupMode::Naval | LookupMode::Event => {
                source.entity_locators.iter().find(|l| l.name == name)
            }
        }
        .map(Binding::Locator)
    };
    let mesh = || {
        source
            .binary_locators
            .iter()
            .find(|l| latin1_equals_bytes(&l.name, name.as_bytes()))
            .map(Binding::Locator)
            .or_else(|| script_parent(&source.skeleton, name).map(Binding::Joint))
    };
    match mode {
        LookupMode::Attachment | LookupMode::Naval => entity().or_else(mesh),
        LookupMode::AttachmentDummy => entity(),
        LookupMode::Event => mesh().or_else(entity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton(names: &[&str]) -> Vec<NativeJoint> {
        names
            .iter()
            .map(|name| NativeJoint {
                name: (*name).into(),
                parent: 255,
                inverse_bind: [0.0; 12],
            })
            .collect()
    }

    fn point(name: &str, x: f32) -> Locator {
        Locator {
            name: name.into(),
            local: local(
                JointPose {
                    translation: [x, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: 1.0,
                },
                1.0,
            ),
            parent: None,
            error: None,
        }
    }

    fn block(name: &str, position: Vec<f32>, rotation: Vec<f32>) -> mesh::Node {
        mesh::Node {
            name: name.into(),
            properties: vec![
                ("p".into(), mesh::Value::Floats(position)),
                ("q".into(), mesh::Value::Floats(rotation)),
            ],
            ..Default::default()
        }
    }

    fn tree(children: Vec<mesh::Node>) -> mesh::Node {
        mesh::Node {
            children: vec![mesh::Node {
                name: "locator".into(),
                children,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn binary_locators_preserve_nonunit_quaternions_and_scale_only_translation() {
        let root = tree(vec![block(
            "nonunit",
            vec![1.0, 2.0, 3.0, 99.0],
            vec![0.0, 0.0, 2.0, 1.0, 99.0],
        )]);
        let negative = binary(&root, &[], -2.0).unwrap();
        assert_eq!(
            negative[0].local,
            [
                [-7.0, 4.0, 0.0, 0.0],
                [-4.0, -7.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-2.0, -4.0, -6.0, 1.0],
            ]
        );
        let zero = binary(&root, &[], 0.0).unwrap();
        assert_eq!(&zero[0].local[..3], &negative[0].local[..3]);
        assert_eq!(zero[0].local[3], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn binary_locators_keep_first_properties_first_block_and_parent_ordinals() {
        let mut first = block("duplicate", vec![3.0, 2.0, 1.0], vec![0.0; 4]);
        first
            .properties
            .push(("p".into(), mesh::Value::Floats(vec![99.0; 3])));
        first
            .properties
            .push(("pa".into(), mesh::Value::Strings(vec!["same".into()])));
        let mut root = tree(vec![first, block("duplicate", vec![5.0; 3], vec![0.0; 4])]);
        root.children.push(mesh::Node {
            name: "locator".into(),
            children: vec![block("ignored", vec![], vec![])],
            ..Default::default()
        });
        let held = binary(&root, &skeleton(&["same", "same"]), 1.0).unwrap();
        assert_eq!(held.len(), 2);
        assert_eq!(held[0].local[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(held[0].local[3], [3.0, 2.0, 1.0, 1.0]);
        assert_eq!(held[0].parent, Some(0));
        assert_eq!(held[1].parent, None);
    }

    #[test]
    fn binary_locators_refuse_native_overread_forms_and_count_latin1_bytes() {
        let mut integer = block("bad", vec![1.0; 3], vec![0.0; 4]);
        integer.properties[0].1 = mesh::Value::Ints(vec![1; 3]);
        for node in [
            integer,
            block("short", vec![1.0; 2], vec![0.0; 4]),
            block("shortq", vec![1.0; 3], vec![0.0; 3]),
        ] {
            assert!(
                binary(&tree(vec![node]), &[], 1.0)
                    .unwrap_err()
                    .to_string()
                    .contains("1 refused")
            );
        }
        assert!(
            binary(
                &tree(vec![block(&"é".repeat(63), vec![0.0; 3], vec![0.0; 4])]),
                &[],
                1.0
            )
            .is_ok()
        );
        assert!(
            binary(
                &tree(vec![block(&"é".repeat(64), vec![0.0; 3], vec![0.0; 4])]),
                &[],
                1.0
            )
            .is_err()
        );
        let mut absent = block("parent", vec![0.0; 3], vec![0.0; 4]);
        absent
            .properties
            .push(("pa".into(), mesh::Value::Strings(Vec::new())));
        assert!(binary(&tree(vec![absent.clone()]), &[], 1.0).is_err());
        absent.properties.last_mut().unwrap().1 = mesh::Value::Strings(vec![String::new()]);
        assert_eq!(
            binary(&tree(vec![absent]), &skeleton(&[""]), 1.0).unwrap()[0].parent,
            Some(0)
        );
    }

    #[test]
    fn lookup_preserves_caller_specific_duplicate_and_alias_precedence() {
        let mut source = Source {
            pdxmesh: true,
            skeleton: skeleton(&["same", "same", "joint_only"]),
            entity_locators: vec![point("same", 1.0), point("same", 2.0)],
            binary_locators: vec![point("same", 3.0), point("same", 4.0)],
            mesh_scale: 1.0,
            entity_scale: 1.0,
            frozen: None,
            game_data: None,
        };
        let x = |mode| {
            lookup(&source, "same", mode)
                .unwrap()
                .local(Ok(None))
                .unwrap()[3][0]
        };
        assert_eq!(x(LookupMode::Attachment), 2.0);
        assert_eq!(x(LookupMode::AttachmentDummy), 2.0);
        assert_eq!(x(LookupMode::Naval), 1.0);
        assert_eq!(x(LookupMode::Event), 3.0);
        assert_eq!(
            lookup(&source, "same\0suffix", LookupMode::Naval),
            lookup(&source, "same", LookupMode::Naval)
        );
        source.binary_locators.clear();
        assert_eq!(
            lookup(&source, "same", LookupMode::Event),
            Some(Binding::Joint(0))
        );
        assert_eq!(
            lookup(&source, "joint_only", LookupMode::Naval),
            Some(Binding::Joint(2))
        );
        assert_eq!(lookup(&source, "absent", LookupMode::Attachment), None);
        assert_eq!(
            lookup(&source, "joint_only", LookupMode::AttachmentDummy),
            None
        );
        source.entity_locators.clear();
        assert_eq!(lookup(&source, "same", LookupMode::AttachmentDummy), None);
    }

    #[test]
    fn locator_lookup_compares_binary_latin1_and_script_utf8_as_original_bytes() {
        let mut source = Source {
            pdxmesh: true,
            skeleton: skeleton(&["é", "Ã©"]),
            entity_locators: Vec::new(),
            binary_locators: vec![point("é", 1.0), point("Ã©", 2.0)],
            mesh_scale: 1.0,
            entity_scale: 1.0,
            frozen: None,
            game_data: None,
        };
        assert_eq!(
            lookup(&source, "é", LookupMode::Naval),
            Some(Binding::Locator(&source.binary_locators[1]))
        );
        source.binary_locators.clear();
        assert_eq!(
            lookup(&source, "é", LookupMode::Naval),
            Some(Binding::Joint(1))
        );
        source.skeleton.truncate(1);
        assert_eq!(lookup(&source, "é", LookupMode::Naval), None);
        source.entity_locators.push(point("é", 3.0));
        assert_eq!(
            lookup(&source, "é", LookupMode::Naval),
            Some(Binding::Locator(&source.entity_locators[0]))
        );

        let entries = vec![(
            "script".into(),
            mesh::Locator {
                parent_joint: Some("é".into()),
                origin_mesh: Some("mesh".into()),
                ..Default::default()
            },
        )];
        let joints = skeleton(&["é", "Ã©"]);
        assert_eq!(entity(&entries, &joints, Some("mesh"))[0].parent, Some(1));
        let mut binary_node = block("binary", vec![0.0; 3], vec![0.0; 4]);
        binary_node
            .properties
            .push(("pa".into(), mesh::Value::Strings(vec!["é".into()])));
        assert_eq!(
            binary(&tree(vec![binary_node]), &joints, 1.0).unwrap()[0].parent,
            Some(0)
        );
    }

    #[test]
    fn locator_parent_uses_played_pose_and_keeps_root_when_palette_is_absent() {
        let mut locator = point("mounted", 2.0);
        locator.parent = Some(1);
        let posed = [point("wrong", 50.0).local, point("right", 5.0).local];
        assert_eq!(
            Binding::Locator(&locator).local(Ok(Some(&posed))).unwrap()[3][0],
            7.0
        );
        assert_eq!(
            Binding::Locator(&locator).local(Ok(None)).unwrap()[3][0],
            2.0
        );
        assert!(
            Binding::Locator(&locator)
                .local(Ok(Some(&posed[..1])))
                .is_err()
        );
        assert_eq!(
            Binding::Joint(1).local(Ok(None)),
            Ok(super::super::IDENTITY)
        );
        assert_eq!(Binding::Joint(1).local(Ok(Some(&posed))), Ok(posed[1]));
        assert!(Binding::Joint(1).local(Ok(Some(&posed[..1]))).is_err());
        let failure = "actual played pose unavailable".to_owned();
        assert_eq!(
            Binding::Locator(&locator).local(Err(failure.clone())),
            Err(failure.clone())
        );
        assert_eq!(
            Binding::Joint(1).local(Err(failure.clone())),
            Err(failure.clone())
        );
        locator.parent = None;
        assert_eq!(
            Binding::Locator(&locator).local(Err(failure)),
            Ok(locator.local)
        );
    }

    #[test]
    fn cloned_entity_locator_refuses_changed_parent_origin_only_when_reached() {
        let entries = vec![
            (
                "parented".into(),
                mesh::Locator {
                    parent_joint: Some("mount".into()),
                    origin_mesh: Some("original".into()),
                    ..Default::default()
                },
            ),
            (
                "unparented".into(),
                mesh::Locator {
                    origin_mesh: Some("original".into()),
                    ..Default::default()
                },
            ),
        ];
        let same = entity(&entries, &skeleton(&["x", "mount"]), Some("original"));
        assert_eq!(same[0].parent, Some(1));
        let changed = entity(&entries, &skeleton(&["mount", "x"]), Some("changed"));
        assert_eq!(changed[0].parent, None);
        assert!(
            Binding::Locator(&changed[0])
                .local(Ok(None))
                .unwrap_err()
                .contains("1 refused parent ordinal")
        );
        assert!(Binding::Locator(&changed[1]).local(Ok(None)).is_ok());
    }
}
