use std::io;

use super::{Transform, Value, invalid, parse, triple};

/// A decoded `.anim` file: the `[info]` rest pose and the `[samples]` frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    /// `fps`, the sample rate.
    pub fps: f32,
    /// `sa`, the number of frames.
    pub samples: u32,
    /// One entry per joint, in skeleton index order.
    pub joints: Vec<AnimationJoint>,
    /// `[samples]` `t`: three floats per frame per joint that declares `t`,
    /// **frame-major** and in joint order.
    pub(super) translations: Vec<f32>,
    /// `[samples]` `q`: four floats, the same way.
    pub(super) rotations: Vec<f32>,
    /// `[samples]` `s`: one float, the same way.
    pub(super) scales: Vec<f32>,
}

impl Animation {
    /// The rest pose of one joint, as a transform in its parent's space.
    ///
    /// This is the `[info]` block's own `t`, `q` and `s`, read verbatim. It is
    /// separate from [`FIRST_FRAME`], which reads the first sample of each
    /// animated channel. A file can repeat its rest values at frame zero,
    /// but each channel retains its authored samples independently. Keeping
    /// the two readings apart also allows [`Animation::local`] to select
    /// later frames while static channels keep their rest values.
    #[cfg(test)]
    pub(super) fn rest(&self, name: &str) -> Option<Transform> {
        let joint = self.joints.iter().find(|joint| joint.name == name)?;
        Some(Transform::from_trs(
            joint.translation,
            joint.rotation,
            joint.scale,
        ))
    }

    /// How many joints declare each of the three channels, in `tqs` order.
    ///
    /// The `[samples]` streams carry only the joints that declare the channel,
    /// so these are the strides one frame advances by.
    pub fn channel_joints(&self) -> [usize; 3] {
        let count = |channel: char| {
            self.joints
                .iter()
                .filter(|joint| joint.channels.contains(channel))
                .count()
        };
        [count('t'), count('q'), count('s')]
    }

    /// Every joint's transform in its parent's space at one frame, in file
    /// order.
    ///
    /// A channel the joint does not animate contributes its `[info]` value, and
    /// so does any channel whose stream is short of this frame, which is what
    /// makes a file with no `[samples]` block at all evaluate to its rest pose.
    pub fn frame(&self, frame: u32) -> Vec<Transform> {
        self.frame_trs(frame)
            .iter()
            .map(JointPose::transform)
            .collect()
    }

    /// Authored channel values before quaternion-to-matrix conversion.
    pub fn frame_trs(&self, frame: u32) -> Vec<JointPose> {
        let [strides_t, strides_q, strides_s] = self.channel_joints();
        let frame = frame as usize;
        let (mut slot_t, mut slot_q, mut slot_s) = (0usize, 0usize, 0usize);
        let mut out = Vec::with_capacity(self.joints.len());
        for joint in &self.joints {
            let mut translation = joint.translation;
            let mut rotation = joint.rotation;
            let mut scale = joint.scale;
            if joint.channels.contains('t') {
                let at = (frame * strides_t + slot_t) * 3;
                if let Some(values) = self.translations.get(at..at + 3) {
                    translation = [values[0], values[1], values[2]];
                }
                slot_t += 1;
            }
            if joint.channels.contains('q') {
                let at = (frame * strides_q + slot_q) * 4;
                if let Some(values) = self.rotations.get(at..at + 4) {
                    rotation = [values[0], values[1], values[2], values[3]];
                }
                slot_q += 1;
            }
            if joint.channels.contains('s') {
                let at = frame * strides_s + slot_s;
                if let Some(value) = self.scales.get(at) {
                    scale = *value;
                }
                slot_s += 1;
            }
            out.push(JointPose {
                translation,
                rotation,
                scale,
            });
        }
        out
    }

    /// The loop period is the last sample's time, (samples - 1) / fps;
    /// a single-sample clip lasts one frame interval.
    pub fn duration(&self) -> f32 {
        if self.fps > 0.0 {
            self.samples.saturating_sub(1).max(1) as f32 / self.fps
        } else {
            0.0
        }
    }

    pub fn sample(&self, seconds: f32, looping: bool) -> Vec<JointPose> {
        let duration = self.duration();
        let time = if looping && duration > 0.0 {
            seconds.rem_euclid(duration)
        } else {
            seconds.clamp(0.0, duration)
        };
        let frame = time * self.fps;
        let a = (frame.floor() as u32).min(self.samples.saturating_sub(1));
        let b = (a + 1).min(self.samples.saturating_sub(1));
        let weight = frame.fract();
        self.frame_trs(a)
            .into_iter()
            .zip(self.frame_trs(b))
            .map(|(a, b)| a.blend(b, weight))
            .collect()
    }

    /// One joint's transform in its parent's space at one frame.
    #[cfg(test)]
    pub(super) fn local(&self, name: &str, frame: u32) -> Option<Transform> {
        let index = self.joints.iter().position(|joint| joint.name == name)?;
        self.frame(frame).into_iter().nth(index)
    }
}

/// Local translation, quaternion and uniform scale. Blend before composing parents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointPose {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: f32,
}
impl JointPose {
    pub fn transform(&self) -> Transform {
        Transform::from_trs(self.translation, self.rotation, self.scale)
    }
    pub fn blend(self, mut other: Self, t: f32) -> Self {
        if t <= 0.0 {
            return self;
        }
        if t >= 1.0 {
            return other;
        }
        let mut dot: f32 = self
            .rotation
            .iter()
            .zip(other.rotation)
            .map(|(a, b)| a * b)
            .sum();
        if dot < 0.0 {
            other.rotation.iter_mut().for_each(|q| *q = -*q);
            dot = -dot;
        }
        let (a, b) = if dot > 0.9995 {
            (1.0 - t, t)
        } else {
            let theta = dot.clamp(-1.0, 1.0).acos();
            (
                ((1.0 - t) * theta).sin() / theta.sin(),
                (t * theta).sin() / theta.sin(),
            )
        };
        let mut rotation = std::array::from_fn(|i| a * self.rotation[i] + b * other.rotation[i]);
        let norm = rotation.iter().map(|q| q * q).sum::<f32>().sqrt();
        if norm > 0.0 {
            rotation.iter_mut().for_each(|q| *q /= norm);
        }
        Self {
            translation: std::array::from_fn(|i| {
                self.translation[i] + (other.translation[i] - self.translation[i]) * t
            }),
            rotation,
            scale: self.scale + (other.scale - self.scale) * t,
        }
    }
}

/// The frame of a state animation a still hour shows.
///
/// A recording holds one sample an hour and no animation clock, so an entity
/// has no elapsed time in its default state: what the engine draws at time 0 is
/// sample 0. Nothing here advances it.
pub const FIRST_FRAME: u32 = 0;

/// One joint's rest pose inside an `.anim`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationJoint {
    pub name: String,
    /// `sa`, the animated channels: a subset of `"tqs"` in that order. Empty
    /// means the joint is static and keeps its rest transform at every
    /// sampled frame.
    pub channels: String,
    /// `t`, the rest translation.
    pub translation: [f32; 3],
    /// `q`, the rest rotation quaternion `(x, y, z, w)`.
    pub rotation: [f32; 4],
    /// `s`, the rest uniform scale. **Zero hides the joint's attachment**:
    /// an animation can select between attached objects by giving the
    /// visible joint a nonzero scale and the other joints a zero scale.
    /// Their authored transforms remain available even while hidden.
    pub scale: f32,
}

/// Decodes the `[info]` and `[samples]` blocks of a `.anim` file.
///
/// Errors are named `InvalidData` for anything [`parse`] refuses, for a file
/// with no `[info]` node, and for a `[samples]` stream whose length is not the
/// frame count times the number of joints declaring that channel.
pub fn read_animation(bytes: &[u8]) -> io::Result<Animation> {
    let root = parse(bytes)?;
    let info = root
        .child("info")
        .ok_or_else(|| invalid("a PDX animation carries no [info] node"))?;
    let single = |name: &str| {
        info.property(name)
            .and_then(Value::floats)
            .and_then(|values| values.first().copied())
    };
    let joints = info
        .children
        .iter()
        .map(|child| {
            let translation = child
                .property("t")
                .and_then(triple)
                .unwrap_or([0.0, 0.0, 0.0]);
            let rotation = child
                .property("q")
                .and_then(Value::floats)
                .and_then(|values| values.get(..4))
                .map_or([0.0, 0.0, 0.0, 1.0], |values| {
                    [values[0], values[1], values[2], values[3]]
                });
            AnimationJoint {
                name: child.name.clone(),
                channels: child.string("sa").unwrap_or_default().to_string(),
                translation,
                rotation,
                scale: child
                    .property("s")
                    .and_then(Value::floats)
                    .and_then(|values| values.first().copied())
                    .unwrap_or(1.0),
            }
        })
        .collect();
    let mut animation = Animation {
        fps: single("fps").unwrap_or(0.0),
        samples: info
            .property("sa")
            .and_then(Value::ints)
            .and_then(|values| values.first().copied())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        joints,
        ..Animation::default()
    };
    let Some(samples) = root.child("samples") else {
        return Ok(animation);
    };
    let stream = |name: &str| {
        samples
            .property(name)
            .and_then(Value::floats)
            .unwrap_or(&[])
            .to_vec()
    };
    animation.translations = stream("t");
    animation.rotations = stream("q");
    animation.scales = stream("s");
    let [joints_t, joints_q, joints_s] = animation.channel_joints();
    let frames = animation.samples as usize;
    for (name, have, want) in [
        ("t", animation.translations.len(), 3 * frames * joints_t),
        ("q", animation.rotations.len(), 4 * frames * joints_q),
        ("s", animation.scales.len(), frames * joints_s),
    ] {
        if have != want {
            return Err(invalid(format!(
                "a PDX animation of {frames} frames declares {} joints with a \"{name}\" channel, \
                 so its [samples] \"{name}\" needs {want} floats and has {have}",
                match name {
                    "t" => joints_t,
                    "q" => joints_q,
                    _ => joints_s,
                }
            )));
        }
    }
    Ok(animation)
}
