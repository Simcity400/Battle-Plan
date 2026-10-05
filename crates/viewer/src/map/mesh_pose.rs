//! Source-ordered palette142399990 and quaternion matrix1424E7F50.
use installed_content::mesh::{Animation, JointPose, NativeJoint};
use std::io;

pub type Matrix = [[f32; 4]; 4];
pub const IDENTITY: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

///1422621F0/142399990 multiply four complete columns in this add order.
pub fn multiply(parent: Matrix, local: Matrix) -> Matrix {
    std::array::from_fn(|column| {
        std::array::from_fn(|row| {
            local[column][1] * parent[1][row]
                + local[column][0] * parent[0][row]
                + local[column][2] * parent[2][row]
                + local[column][3] * parent[3][row]
        })
    })
}

pub fn scale(value: f32) -> Matrix {
    [
        [value, 0., 0., 0.],
        [0., value, 0., 0.],
        [0., 0., value, 0.],
        [0., 0., 0., 1.],
    ]
}

/// The importer scale is baked into positions and skeleton translations only.
#[derive(Debug, Clone)]
pub struct Source {
    pub skeleton: Vec<NativeJoint>,
    pub mesh_scale: f32,
    pub entity_scale: f32,
    pub frozen: Option<std::sync::Arc<Animation>>,
    pub game_data: Option<Result<installed_content::mesh::EntityGameData, String>>,
}

pub struct Locals {
    pub poses: Vec<JointPose>,
    pub crossfade: bool,
}

pub struct Actor {
    pub world: Result<Matrix, String>,
    pub pose: Result<Pose, String>,
}

fn parent_world(
    rig: &super::entity_animation::Rig,
    actors: &[Actor],
    node: &super::entity_animation::Node,
) -> Result<Matrix, String> {
    let Some((parent, joint, _)) = &node.parent else {
        return Ok(IDENTITY);
    };
    let actor = &actors[*parent];
    let source = rig.nodes[*parent]
        .native
        .as_ref()
        .ok_or_else(|| "native parent actor source unavailable".to_owned())?;
    let joint_index = source
        .skeleton
        .iter()
        .position(|j| j.name == *joint)
        .ok_or_else(|| format!("native attachment joint {joint:?} unavailable"))?;
    let pose = actor.pose.as_ref().map_err(Clone::clone)?;
    Ok(multiply(
        *actor.world.as_ref().map_err(Clone::clone)?,
        pose.joints[joint_index],
    ))
}

/// Bind matrices stay actor-local. Attachment actors use joint world matrices,
/// not skin matrices, through1422621F0.
pub fn actors(
    rig: &super::entity_animation::Rig,
    player: Option<&super::entity_animation::Player>,
    frozen_animation: bool,
) -> Vec<Actor> {
    let mut actors: Vec<Actor> = Vec::with_capacity(rig.nodes.len());
    for (index, node) in rig.nodes.iter().enumerate() {
        let Some(source) = node.native.as_ref() else {
            let why = "native actor source unavailable".to_owned();
            actors.push(Actor {
                world: Err(why.clone()),
                pose: Err(why),
            });
            continue;
        };
        let world = parent_world(rig, &actors, node)
            .map(|parent| multiply(parent, scale(source.entity_scale)));
        let pose = if source.skeleton.is_empty() {
            Ok(Pose::default())
        } else {
            let locals = player.map_or_else(
                || {
                    source
                        .frozen
                        .as_ref()
                        .filter(|_| frozen_animation)
                        .map(|a| {
                            sample(a, 0., false).map(|poses| Locals {
                                poses,
                                crossfade: false,
                            })
                        })
                        .transpose()
                },
                |p| p.native_locals(rig, index),
            );
            locals.and_then(|locals| {
                let locals = locals.ok_or_else(|| io::Error::other("native skeleton has no played animation; its palette is uninitialized (142399610/142399990/14239B0B0)"))?;
                assemble(&source.skeleton, &locals.poses, source.mesh_scale, locals.crossfade)
            }).map_err(|e| e.to_string())
        };
        actors.push(Actor { world, pose });
    }
    actors
}

///1424E8D50: only the near-linear branch normalizes. Static channels never call it.
pub fn slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot = b[1] * a[1] + b[0] * a[0] + b[2] * a[2] + b[3] * a[3];
    if dot < 0. {
        b = b.map(|v| -v);
        dot = -dot;
    }
    if dot <= 0.9995 {
        //142B7E168
        let theta = dot.acos();
        let angle = theta * t;
        let b_weight = angle.sin() / theta.sin();
        let a_weight = angle.cos() - b_weight * dot;
        std::array::from_fn(|i| a[i] * a_weight + b[i] * b_weight)
    } else {
        let q: [f32; 4] = std::array::from_fn(|i| (b[i] - a[i]) * t + a[i]);
        let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        if length == 0. {
            [0.; 4]
        } else {
            q.map(|v| v / length)
        }
    }
}

///142399990 uses double sample intervals, wraps the next index even when the
///playback time is clamped, and blends channels in source order.
pub fn sample(animation: &Animation, seconds: f32, looping: bool) -> io::Result<Vec<JointPose>> {
    if !animation.fps.is_finite()
        || animation.fps <= 0.
        || animation.samples == 0
        || !seconds.is_finite()
    {
        return Err(io::Error::other(
            "native animation requires finite positive fps, samples and playback time",
        ));
    }
    let interval = 1. / f64::from(animation.fps);
    let duration = f64::from(animation.samples.saturating_sub(1).max(1)) * interval;
    let time = if looping {
        f64::from(seconds).rem_euclid(duration)
    } else {
        f64::from(seconds).clamp(0., duration)
    };
    let index = (time / interval) as u32;
    let t = ((time - f64::from(index) * interval) / interval).clamp(0., 1.) as f32;
    let first = if index == animation.samples { 0 } else { index };
    let next = if first + 1 == animation.samples {
        0
    } else {
        first + 1
    };
    let a = animation.frame_trs(first);
    let b = animation.frame_trs(next);
    Ok(a.into_iter()
        .zip(b)
        .zip(&animation.joints)
        .map(|((a, b), joint)| {
            let mut result = a;
            if joint.channels.contains('t') {
                result.translation = [
                    (1. - t) * a.translation[0] + t * b.translation[0],
                    t * b.translation[1] + (1. - t) * a.translation[1],
                    t * b.translation[2] + (1. - t) * a.translation[2],
                ];
            }
            if joint.channels.contains('q') {
                result.rotation = slerp(a.rotation, b.rotation, t);
            }
            if joint.channels.contains('s') {
                result.scale = (1. - t) * a.scale + t * b.scale;
            }
            result
        })
        .collect())
}

/// State crossfade is also source ordered (142399990), never name-remapped.
pub fn blend(a: JointPose, b: JointPose, t: f32) -> JointPose {
    JointPose {
        translation: std::array::from_fn(|i| (1. - t) * a.translation[i] + t * b.translation[i]),
        rotation: slerp(a.rotation, b.rotation, t),
        scale: (1. - t) * a.scale + t * b.scale,
    }
}

fn local(pose: JointPose, mesh_scale: f32) -> Matrix {
    let [x, y, z, w] = pose.rotation;
    //1424E7F50 consumes the quaternion verbatim; no normalization.
    let twice = |value: f32| value + value;
    let basis = [
        [
            1. - twice(y * y + z * z),
            twice(w * z + y * x),
            twice(x * z - y * w),
        ],
        [
            twice(y * x - w * z),
            1. - twice(x * x + z * z),
            twice(w * x + y * z),
        ],
        [
            twice(y * w + x * z),
            twice(y * z - w * x),
            1. - twice(x * x + y * y),
        ],
    ];
    [
        [
            basis[0][0] * pose.scale,
            basis[0][1] * pose.scale,
            basis[0][2] * pose.scale,
            0.,
        ],
        [
            basis[1][0] * pose.scale,
            basis[1][1] * pose.scale,
            basis[1][2] * pose.scale,
            0.,
        ],
        [
            basis[2][0] * pose.scale,
            basis[2][1] * pose.scale,
            basis[2][2] * pose.scale,
            0.,
        ],
        [
            mesh_scale * pose.translation[0],
            mesh_scale * pose.translation[1],
            mesh_scale * pose.translation[2],
            1.,
        ],
    ]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub joints: Vec<Matrix>,
    pub bones: Vec<Matrix>,
}

pub fn assemble(
    skeleton: &[NativeJoint],
    locals: &[JointPose],
    mesh_scale: f32,
    crossfade: bool,
) -> io::Result<Pose> {
    if locals.len() < skeleton.len() {
        return Err(io::Error::other(format!(
            "native animation has {} local records for {} skeleton records",
            locals.len(),
            skeleton.len()
        )));
    }
    //Native writes all locals first. A forward parent therefore reads that
    //parent's local value, not a recursively evaluated future world value.
    let mut joints: Vec<_> = locals
        .iter()
        .take(skeleton.len())
        //14239A123/137/145 scale crossfade translation by joint scale;
        //the single-clip branch uses meshscale.
        .map(|v| local(*v, if crossfade { v.scale } else { mesh_scale }))
        .collect();
    let mut bones = Vec::with_capacity(joints.len());
    for (index, joint) in skeleton.iter().enumerate() {
        if joint.parent != 255 {
            let parent = joints
                .get(usize::from(joint.parent))
                .copied()
                .ok_or_else(|| {
                    io::Error::other(format!(
                        "native skeleton joint{} parent{} leaves allocated palette",
                        index, joint.parent
                    ))
                })?;
            joints[index] = multiply(parent, joints[index]);
        }
        let b = joint.inverse_bind;
        bones.push(multiply(
            joints[index],
            [
                [b[0], b[1], b[2], 0.],
                [b[3], b[4], b[5], 0.],
                [b[6], b[7], b[8], 0.],
                [b[9], b[10], b[11], 1.],
            ],
        ));
    }
    Ok(Pose { joints, bones })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_uses_native_joint_scale_for_translation() {
        let joint = NativeJoint {
            name: "root".into(),
            parent: 255,
            inverse_bind: [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.],
        };
        let pose = JointPose {
            translation: [2., 4., 6.],
            rotation: [0., 0., 0., 1.],
            scale: 3.,
        };
        let normal = assemble(std::slice::from_ref(&joint), &[pose], 0.5, false).unwrap();
        let blended = assemble(&[joint], &[pose], 0.5, true).unwrap();
        assert_eq!(normal.joints[0][3], [1., 2., 3., 1.]);
        assert_eq!(blended.joints[0][3], [6., 12., 18., 1.]);
        assert_eq!(normal.joints[0][0], blended.joints[0][0]);
    }

    #[test]
    fn static_channels_do_not_take_animated_quaternion_normalization() {
        let mut animation = Animation::default();
        animation.fps = 1.;
        animation.samples = 1;
        animation.joints.push(installed_content::mesh::AnimationJoint {
            name: "source ordinal, not a skeleton lookup".into(),
            rotation: [0., 0., 0., 2.],
            translation: [3., 5., 7.],
            scale: 1.,
            ..Default::default()
        });
        assert_eq!(
            sample(&animation, 0., false).unwrap()[0].rotation,
            [0., 0., 0., 2.]
        );
        animation.joints[0].channels = "q".into();
        assert_eq!(
            sample(&animation, 0., false).unwrap()[0].rotation,
            [0., 0., 0., 1.]
        );
        //1424E8D50's spherical branch does not normalize a nonunit result.
        assert_eq!(
            slerp([0., 0., 0., 0.5], [0.5, 0., 0., 0.], 0.),
            [0., 0., 0., 0.5]
        );
        assert_eq!(
            slerp([0., 0., 0., 1.], [0., 0., 0., -1.], 0.5),
            [0., 0., 0., 1.]
        );
    }

    #[test]
    fn palette_requires_initialized_source_records() {
        let joint = NativeJoint {
            name: "unavailable".into(),
            parent: 3,
            inverse_bind: [0.; 12],
        };
        assert!(
            assemble(std::slice::from_ref(&joint), &[], 1., false)
                .unwrap_err()
                .to_string()
                .contains("0 local records")
        );
        let local = JointPose {
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            scale: 1.,
        };
        assert!(
            assemble(&[joint], &[local], 1., false)
                .unwrap_err()
                .to_string()
                .contains("parent3 leaves allocated palette")
        );
    }
    #[test]
    fn native_static_quaternion_and_homogeneous_scale_are_preserved() {
        let matrix = local(
            JointPose {
                translation: [3., 5., 7.],
                rotation: [0., 0., 0.5, 0.5],
                scale: 2.,
            },
            0.5,
        );
        assert_eq!(
            matrix,
            [
                [1., 1., 0., 0.],
                [-1., 1., 0., 0.],
                [0., 0., 2., 0.],
                [1.5, 2.5, 3.5, 1.]
            ]
        );
        assert_eq!(multiply(IDENTITY, matrix), matrix);
        assert_eq!(scale(0.)[3][3], 1.);
    }

    #[test]
    fn forward_parents_read_source_order_locals_and_skin_stays_actor_local() {
        let joint = |parent| NativeJoint {
            name: String::new(),
            parent,
            inverse_bind: [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.],
        };
        let poses = [1., 2., 4.].map(|x| JointPose {
            translation: [x, 0., 0.],
            rotation: [0., 0., 0., 1.],
            scale: 1.,
        });
        let pose = assemble(&[joint(1), joint(2), joint(255)], &poses, 0.5, false).unwrap();
        assert_eq!(
            pose.joints.iter().map(|m| m[3][0]).collect::<Vec<_>>(),
            [1.5, 3., 2.]
        );
        assert_eq!(pose.joints, pose.bones);
    }
}
