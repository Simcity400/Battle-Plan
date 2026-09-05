//! The authored `light` templates and the entity playhead that fires them.

use crate::production_features::xmap13_pose::{xmap13_pose_phase, xmap13_pose_seed};
use crate::{
    parse_f32_value, parse_string_value, strip_comments, BuildingEntityEvent, PdxJoint, PdxLocator,
    Result,
};

/// The operator an emitter's animation applies to the template's base intensity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xmap09LightAnimationOp {
    Mul,
    Add,
    Abs,
}

impl Xmap09LightAnimationOp {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().trim_matches('"') {
            "MUL" => Some(Self::Mul),
            "ADD" => Some(Self::Add),
            "ABS" => Some(Self::Abs),
            _ => None,
        }
    }
}

/// The `animation = { ... }` an emitter declares, exactly as authored.
#[derive(Debug, Clone, PartialEq)]
pub struct Xmap09LightAnimation {
    pub name: String,
    pub start: f32,
    pub duration: f32,
    pub repeat: bool,
    pub op: Xmap09LightAnimationOp,
    pub min_value: f32,
    pub max_value: f32,
    /// `(t, value)` pairs in authored order; `t` is a fraction of `duration`.
    pub curve: Vec<[f32; 2]>,
}

impl Xmap09LightAnimation {
    /// One cubic table-builder sample; renderer lookup uses the compiled table separately.
    pub fn sample_curve_for_table(&self, t: f32) -> Option<f32> {
        if !t.is_finite()
            || !(0.0..=1.0).contains(&t)
            || !self.min_value.is_finite()
            || !self.max_value.is_finite()
            || self.curve.iter().flatten().any(|value| !value.is_finite())
        {
            return None;
        }
        if self.curve.len() < 2 {
            return Some(0.0);
        }
        if self.curve[0][0] != 0.0
            || self.curve.last()?[0] != 1.0
            || self.curve.windows(2).any(|pair| pair[0][0] >= pair[1][0])
        {
            return None;
        }
        let mut points = Vec::with_capacity(self.curve.len() + 2);
        let first = self.curve[0];
        let second = self.curve[1];
        points.push([
            (first[0] - second[0]) + first[0],
            (first[1] - second[1]) + first[1],
        ]);
        points.extend_from_slice(&self.curve);
        let last = self.curve[self.curve.len() - 1];
        let penultimate = self.curve[self.curve.len() - 2];
        points.push([
            (last[0] - penultimate[0]) + last[0],
            (last[1] - penultimate[1]) + last[1],
        ]);
        let mut right = 1;
        while right < self.curve.len() && t >= points[right][0] {
            right += 1;
        }
        let u = (t - points[right - 1][0]) / (points[right][0] - points[right - 1][0]);
        let square = u * u;
        let cube = square * u;
        let after = points[right + 1][1] * (cube - square);
        let next = points[right][1] * ((square * 4.0 - cube * 3.0) + u);
        let previous = points[right - 1][1] * ((cube * 3.0 - square * 5.0) + 2.0);
        let before = points[right - 2][1] * (((square + square) - cube) - u);
        let result = (before + previous + next + after) * 0.5 * (self.max_value - self.min_value)
            + self.min_value;
        result.is_finite().then_some(result)
    }

    pub fn evaluate_at_renderer_age(
        &self,
        base: f32,
        renderer_age_seconds: f32,
    ) -> std::result::Result<f32, Xmap09LightEvaluationRefusal> {
        if !base.is_finite()
            || !renderer_age_seconds.is_finite()
            || renderer_age_seconds < 0.0
            || !self.start.is_finite()
            || !self.duration.is_finite()
            || self.duration <= 0.0
            || !(self.start + self.duration).is_finite()
            || self.start + self.duration <= 0.0
        {
            return Err(Xmap09LightEvaluationRefusal::InvalidAnimation);
        }
        let (sample_denominator, lookup_scale) =
            XMAP09_CURVE_TABLE_SCALARS.expect("a729 table scalar data export is pinned");
        let mut samples = [0.0; 64];
        for (index, sample) in samples.iter_mut().enumerate() {
            *sample = self
                .sample_curve_for_table(index as f32 / sample_denominator)
                .ok_or(Xmap09LightEvaluationRefusal::InvalidAnimation)?;
        }
        let end = self.start + self.duration;
        let age = if self.repeat {
            renderer_age_seconds % end
        } else {
            renderer_age_seconds
        };
        let index = ((age - self.start) * lookup_scale) / (end - self.start);
        let value = if index < 0.0 {
            samples[0]
        } else if index < sample_denominator {
            let whole = index as usize;
            let left = *samples
                .get(whole)
                .ok_or(Xmap09LightEvaluationRefusal::InvalidAnimation)?;
            let right = *samples
                .get(whole + 1)
                .ok_or(Xmap09LightEvaluationRefusal::InvalidAnimation)?;
            (right - left) * (index - whole as f32) + left
        } else {
            samples[63]
        };
        let result = match self.op {
            Xmap09LightAnimationOp::Add => value + base,
            Xmap09LightAnimationOp::Mul => value * base,
            Xmap09LightAnimationOp::Abs => value,
        };
        if result.is_finite() {
            Ok(result)
        } else {
            Err(Xmap09LightEvaluationRefusal::InvalidAnimation)
        }
    }
}

const XMAP09_CURVE_TABLE_SCALARS: Option<(f32, f32)> = Some((63.0, 64.0));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xmap09LightEvaluationRefusal {
    InvalidAnimation,
    InvalidRendererDelta,
}

impl std::fmt::Display for Xmap09LightEvaluationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidAnimation => "XMAP09_LIGHT_ANIMATION_INVALID",
            Self::InvalidRendererDelta => "XMAP09_LIGHT_RENDERER_DELTA_INVALID",
        })
    }
}

impl std::error::Error for Xmap09LightEvaluationRefusal {}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Xmap09LightInstanceClock {
    age_seconds: f32,
}

impl Xmap09LightInstanceClock {
    pub fn age_seconds(&self) -> f32 {
        self.age_seconds
    }

    pub fn advance(
        &mut self,
        renderer_delta_seconds: f64,
        enabled: bool,
    ) -> std::result::Result<(), Xmap09LightEvaluationRefusal> {
        if !renderer_delta_seconds.is_finite() || renderer_delta_seconds < 0.0 {
            return Err(Xmap09LightEvaluationRefusal::InvalidRendererDelta);
        }
        if enabled {
            let age = renderer_delta_seconds as f32 + self.age_seconds;
            if !age.is_finite() {
                return Err(Xmap09LightEvaluationRefusal::InvalidRendererDelta);
            }
            self.age_seconds = age;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Xmap09LightTrack {
    pub base: f32,
    pub animations: Vec<String>,
}

impl Xmap09LightTrack {
    fn evaluate(
        &self,
        animations: &[Xmap09LightAnimation],
        clock: &Xmap09LightInstanceClock,
    ) -> std::result::Result<f32, Xmap09LightEvaluationRefusal> {
        if !self.base.is_finite() {
            return Err(Xmap09LightEvaluationRefusal::InvalidAnimation);
        }
        self.animations.iter().try_fold(self.base, |value, name| {
            animations
                .iter()
                .find(|animation| animation.name == *name)
                .ok_or(Xmap09LightEvaluationRefusal::InvalidAnimation)?
                .evaluate_at_renderer_age(value, clock.age_seconds())
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Xmap09LightTemplate {
    pub name: String,
    pub source: String,
    pub color: [Xmap09LightTrack; 3],
    pub intensity: Xmap09LightTrack,
    pub radius: Xmap09LightTrack,
    pub falloff: Xmap09LightTrack,
    pub position: [Xmap09LightTrack; 3],
    pub animations: Vec<Xmap09LightAnimation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Xmap09LightFrameValues {
    pub color: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
    pub falloff: f32,
    pub position: [f32; 3],
}

impl Xmap09LightTemplate {
    pub fn intensity_at_frame(
        &self,
        clock: &Xmap09LightInstanceClock,
    ) -> std::result::Result<f32, Xmap09LightEvaluationRefusal> {
        self.intensity.evaluate(&self.animations, clock)
    }

    pub fn values_at_frame(
        &self,
        clock: &Xmap09LightInstanceClock,
    ) -> std::result::Result<Xmap09LightFrameValues, Xmap09LightEvaluationRefusal> {
        let falloff = self.falloff.evaluate(&self.animations, clock)?;
        let radius = self.radius.evaluate(&self.animations, clock)?;
        let intensity = self.intensity_at_frame(clock)?;
        let color = [
            self.color[0].evaluate(&self.animations, clock)?,
            self.color[1].evaluate(&self.animations, clock)?,
            self.color[2].evaluate(&self.animations, clock)?,
        ];
        let position = [
            self.position[0].evaluate(&self.animations, clock)?,
            self.position[1].evaluate(&self.animations, clock)?,
            self.position[2].evaluate(&self.animations, clock)?,
        ];
        Ok(Xmap09LightFrameValues {
            color,
            intensity,
            radius,
            falloff,
            position,
        })
    }
}

/// Every `light = { ... }` block one asset source declares.
pub fn parse_light_templates(text: &str, source: &str) -> Result<Vec<Xmap09LightTemplate>> {
    let stripped = strip_comments(text);
    let mut out = Vec::new();
    for body in line_first_blocks(&stripped, "light") {
        let Some(name) = parse_string_value(&body, "name") else {
            continue;
        };
        let color = line_first_blocks(&body, "color")
            .first()
            .and_then(|block| {
                Some([
                    parse_light_track(block, "r").ok()?,
                    parse_light_track(block, "g").ok()?,
                    parse_light_track(block, "b").ok()?,
                ])
            })
            .ok_or_else(|| format!("{source}: light {name:?} declares no complete colour"))?;
        let position = line_first_blocks(&body, "position")
            .first()
            .and_then(|block| {
                Some([
                    parse_light_track(block, "x").ok()?,
                    parse_light_track(block, "y").ok()?,
                    parse_light_track(block, "z").ok()?,
                ])
            })
            .ok_or_else(|| format!("{source}: light {name:?} declares no complete position"))?;
        let intensity = parse_light_track(&body, "intensity")?;
        let radius = parse_light_track(&body, "radius")?;
        let falloff = parse_light_track(&body, "falloff")?;
        let mut animations = Vec::new();
        for block in line_first_blocks(&body, "animation") {
            let block = &block;
            let op = parse_string_value(block, "op")
                .as_deref()
                .and_then(Xmap09LightAnimationOp::parse)
                .ok_or_else(|| {
                    format!("{source}: light {name:?} declares an animation op it does not name")
                })?;
            let value = |key: &str| -> Result<f32> {
                parse_f32_value(block, key).ok_or_else(|| {
                    format!("{source}: light {name:?} animation declares no {key}").into()
                })
            };
            let curve = line_first_blocks(block, "curve")
                .first()
                .map(|curve| parse_curve_pairs(curve))
                .transpose()?
                .unwrap_or_default();
            animations.push(Xmap09LightAnimation {
                name: parse_string_value(block, "name").unwrap_or_default(),
                start: value("start")?,
                duration: value("duration")?,
                repeat: parse_string_value(block, "repeat").as_deref() == Some("yes"),
                op,
                min_value: value("minValue")?,
                max_value: value("maxValue")?,
                curve,
            });
        }
        out.push(Xmap09LightTemplate {
            name,
            source: source.to_string(),
            color,
            intensity,
            radius,
            falloff,
            position,
            animations,
        });
    }
    Ok(out)
}

fn parse_light_track(text: &str, key: &str) -> Result<Xmap09LightTrack> {
    let at = crate::find_assignment(text, key, 0)
        .ok_or_else(|| format!("XMAP09_LIGHT_TRACK_MISSING: {key}"))?;
    let mut rest = text[at..].trim_start();
    let braced = rest.starts_with('{');
    if braced {
        rest = rest[1..].trim_start();
    }
    let mut base = 0.0;
    let mut animations = Vec::new();
    loop {
        let (value, tail) = if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted
                .find('"')
                .ok_or("XMAP09_LIGHT_TRACK_UNCLOSED_STRING")?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = rest
                .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '}' | '{' | '='))
                .unwrap_or(rest.len());
            (&rest[..end], &rest[end..])
        };
        if value.is_empty() {
            return Err(format!("XMAP09_LIGHT_TRACK_INVALID: {key}").into());
        }
        if let Ok(number) = value.parse::<f32>() {
            if !number.is_finite() {
                return Err("XMAP09_LIGHT_TRACK_NONFINITE".into());
            }
            base = number;
        } else {
            animations.push(value.to_string());
        }
        rest = tail.trim_start();
        if let Some(tail) = rest.strip_prefix(',') {
            rest = tail.trim_start();
        } else {
            if braced && !rest.starts_with('}') {
                return Err("XMAP09_LIGHT_TRACK_UNCLOSED_BLOCK".into());
            }
            break;
        }
    }
    Ok(Xmap09LightTrack { base, animations })
}

/// Every `key = { ... }` body whose key is the first token of its own line.
///
/// The bound matters: `light` is also an EVENT key written mid-line
/// (`event = { ... light = "alarm_flash" }`), and a scan that took any
/// assignment would read the next unrelated block as that event's template.
fn line_first_blocks(text: &str, key: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        let at = offset + indent;
        offset += line.len();
        let trimmed = line.trim_start();
        if !trimmed.starts_with(key)
            || trimmed[key.len()..].starts_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_')
        {
            continue;
        }
        let Some(open) = text[at..].find('{').map(|found| at + found) else {
            continue;
        };
        if text[at + key.len()..open].trim() != "=" {
            continue;
        }
        let mut depth = 0i32;
        for (index, byte) in bytes[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(text[open + 1..open + index].to_string());
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The `curve = { t v t v ... }` pairs, whitespace separated.
fn parse_curve_pairs(body: &str) -> Result<Vec<[f32; 2]>> {
    let values: std::result::Result<Vec<f32>, _> = body
        .split_whitespace()
        .map(|token| token.parse::<f32>())
        .collect();
    let values = values.map_err(|_| "XMAP09 light curve carries a non-numeric token")?;
    if values.len() % 2 != 0 {
        return Err("XMAP09 light curve carries an unpaired value".into());
    }
    if !values.iter().all(|value| value.is_finite()) {
        return Err("XMAP09 light curve carries a non-finite value".into());
    }
    Ok(values
        .chunks_exact(2)
        .map(|pair| [pair[0], pair[1]])
        .collect())
}

/// A `light` event of a state, resolved to its template and its attach node.
#[derive(Debug, Clone, PartialEq)]
pub struct Xmap09ResolvedLightEvent {
    pub light: String,
    pub node: String,
    /// Seconds into the state's clip at which the engine fires it.
    pub event_time: f32,
    pub values: Xmap09LightFrameValues,
}

/// Why one authored `light` event of a placed entity is not accounted for.
#[derive(Debug, Clone, PartialEq)]
pub enum Xmap09LightEventRefusal {
    /// The event names no attach node, so it has no world position.
    NoNode,
    /// The event carries no `time`, so nothing times it against the playhead.
    NoTime,
    Evaluation {
        light: String,
        reason: Xmap09LightEvaluationRefusal,
    },
    /// No admitted source declares that `light` template.
    UndeclaredTemplate { light: String },
    /// The mesh declares neither a locator nor a bone of that name.
    UnknownNode { node: String },
}

/// One authored `light` event of a state, either resolved or refused with why.
pub fn resolve_light_event(
    event: &BuildingEntityEvent,
    templates: &[Xmap09LightTemplate],
    locators: &[PdxLocator],
    skeleton: &[PdxJoint],
    instance_clock: &Xmap09LightInstanceClock,
) -> std::result::Result<Xmap09ResolvedLightEvent, Option<Xmap09LightEventRefusal>> {
    // An event with no `light` is another domain's (particle, sound); it is not
    // a refusal of this one.
    let light = event.light.clone().ok_or(None)?;
    let Some(template) = templates.iter().find(|template| template.name == light) else {
        return Err(Some(Xmap09LightEventRefusal::UndeclaredTemplate { light }));
    };
    let Some(node) = event.node.clone() else {
        return Err(Some(Xmap09LightEventRefusal::NoNode));
    };
    if !locators.iter().any(|locator| locator.name == node)
        && !skeleton.iter().any(|joint| joint.name == node)
    {
        return Err(Some(Xmap09LightEventRefusal::UnknownNode { node }));
    }
    let Some(event_time) = event.time.filter(|time| time.is_finite() && *time >= 0.0) else {
        return Err(Some(Xmap09LightEventRefusal::NoTime));
    };
    let values = template.values_at_frame(instance_clock).map_err(|reason| {
        Some(Xmap09LightEventRefusal::Evaluation {
            light: light.clone(),
            reason,
        })
    })?;
    Ok(Xmap09ResolvedLightEvent {
        light,
        node,
        event_time,
        values,
    })
}

/// How long ago the state's playhead last crossed `event_time`.
///
/// The playhead is the seeded phase [`xmap13_pose_phase`] draws plus renderer
/// a clip: speed x render delta. A clip that has not yet reached the event on
/// its first pass has not fired it, and a non-looping clip fires it once.
pub fn xmap09_event_age_seconds(
    clip_length_seconds: f32,
    animation_speed: f32,
    seeded_phase_seconds: f32,
    renderer_seconds: f32,
    event_time: f32,
    looping: bool,
) -> Option<f32> {
    if !clip_length_seconds.is_finite()
        || clip_length_seconds <= 0.0
        || !animation_speed.is_finite()
        || animation_speed <= 0.0
        || !seeded_phase_seconds.is_finite()
        || seeded_phase_seconds < 0.0
        || !renderer_seconds.is_finite()
        || renderer_seconds < 0.0
        || !event_time.is_finite()
        || event_time < 0.0
        || event_time > clip_length_seconds
    {
        return None;
    }
    let played =
        f64::from(seeded_phase_seconds) + f64::from(renderer_seconds) * f64::from(animation_speed);
    let length = f64::from(clip_length_seconds);
    let event = f64::from(event_time);
    if !looping {
        return (played >= event).then(|| (played - event) as f32);
    }
    let laps = (played / length).floor();
    let last = laps * length + event;
    let last = if last > played { last - length } else { last };
    (last >= 0.0).then(|| (played - last) as f32)
}

/// The seeded playhead phase for one placed entity's state, in seconds.
pub fn xmap09_event_playhead_phase(semantic_source_id: &str, clip_length_seconds: f32) -> f32 {
    xmap13_pose_phase(xmap13_pose_seed(semantic_source_id), clip_length_seconds)
}

/// The attach node's object transform: a locator's own, through its parent bone
/// where it names one, or the bone's when the node IS a bone.
///
/// `posed_bones` maps a bone name to its posed object transform, which is what
/// the skeleton the state's clip drives produces.
pub fn xmap09_attach_node_object_transform(
    node: &str,
    locators: &[PdxLocator],
    posed_bones: &dyn Fn(&str) -> Option<[f32; 16]>,
) -> Option<[f32; 16]> {
    if let Some(locator) = locators.iter().find(|locator| locator.name == node) {
        let local = locator.transform_4x4.unwrap_or_else(|| {
            compose_translation_rotation(locator.position, locator.rotation_quat)
        });
        return match locator.parent_bone.as_deref() {
            None => Some(local),
            Some(bone) => Some(multiply_4x4(posed_bones(bone)?, local)),
        };
    }
    posed_bones(node)
}

/// The world position one fired event's light instance sits at.
pub fn xmap09_light_world_position(
    placement_object_to_world: [f32; 16],
    attach_node_object: [f32; 16],
    template_position: [f32; 3],
) -> Option<[f32; 3]> {
    let composed = multiply_4x4(placement_object_to_world, attach_node_object);
    let out = transform_point(composed, template_position);
    out.iter().all(|lane| lane.is_finite()).then_some(out)
}

fn compose_translation_rotation(translation: [f32; 3], quat: [f32; 4]) -> [f32; 16] {
    let [x, y, z, w] = quat;
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        1.0 - 2.0 * (yy + zz),
        2.0 * (xy - wz),
        2.0 * (xz + wy),
        translation[0],
        2.0 * (xy + wz),
        1.0 - 2.0 * (xx + zz),
        2.0 * (yz - wx),
        translation[1],
        2.0 * (xz - wy),
        2.0 * (yz + wx),
        1.0 - 2.0 * (xx + yy),
        translation[2],
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn multiply_4x4(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for row in 0..4 {
        for column in 0..4 {
            let mut sum = 0.0f64;
            for k in 0..4 {
                sum += f64::from(a[row * 4 + k]) * f64::from(b[k * 4 + column]);
            }
            out[row * 4 + column] = sum as f32;
        }
    }
    out
}

fn transform_point(m: [f32; 16], point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        (f64::from(m[row * 4]) * f64::from(point[0])
            + f64::from(m[row * 4 + 1]) * f64::from(point[1])
            + f64::from(m[row * 4 + 2]) * f64::from(point[2])
            + f64::from(m[row * 4 + 3])) as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_table_samples_remap_cubic_overshoot_without_clamping() {
        let mut animation =
            parse_light_templates(CITYLIGHT, "t.asset").unwrap()[0].animations[0].clone();
        animation.curve = vec![[0.0, 0.0], [0.25, 1.0], [0.75, 1.0], [1.0, 0.0]];
        animation.min_value = 2.0;
        animation.max_value = 6.0;
        assert_eq!(animation.sample_curve_for_table(0.0), Some(2.0));
        assert_eq!(animation.sample_curve_for_table(0.25), Some(6.0));
        assert_eq!(animation.sample_curve_for_table(0.5), Some(6.5));
        assert_eq!(animation.sample_curve_for_table(1.0), Some(2.0));
        animation.curve.truncate(1);
        assert_eq!(animation.sample_curve_for_table(0.5), Some(0.0));
    }

    #[test]
    fn curve_table_samples_refuse_unproven_or_nonfinite_domains() {
        let mut animation =
            parse_light_templates(CITYLIGHT, "t.asset").unwrap()[0].animations[0].clone();
        assert_eq!(animation.sample_curve_for_table(f32::NAN), None);
        animation.curve[0][0] = 0.1;
        assert_eq!(animation.sample_curve_for_table(0.0), None);
        animation.curve[0][0] = 0.0;
        animation.curve[1][0] = 0.0;
        assert_eq!(animation.sample_curve_for_table(0.5), None);
        animation.curve[1][0] = 0.4;
        animation.max_value = f32::INFINITY;
        assert_eq!(animation.sample_curve_for_table(0.5), None);
    }

    #[test]
    fn light_frame_clock_accumulates_f32_renderer_delta_only_when_enabled() {
        let mut clock = Xmap09LightInstanceClock::default();
        assert_eq!(clock.age_seconds(), 0.0);
        clock.advance(0.25, true).unwrap();
        clock.advance(20.0, false).unwrap();
        assert_eq!(clock.age_seconds(), 0.25);
        clock.advance(0.5, true).unwrap();
        assert_eq!(clock.age_seconds(), 0.75);
        assert_eq!(
            clock.advance(f64::NAN, true),
            Err(Xmap09LightEvaluationRefusal::InvalidRendererDelta)
        );
        assert_eq!(clock.age_seconds(), 0.75);
        let mut large = Xmap09LightInstanceClock::default();
        large.advance(16_777_216.0, true).unwrap();
        large.advance(1.0, true).unwrap();
        large.advance(1.0, true).unwrap();
        assert_eq!(large.age_seconds(), 16_777_216.0);
    }

    #[test]
    fn curve_lookup_uses_64_over_duration_and_holds_endpoints() {
        let mut animation =
            parse_light_templates(CITYLIGHT, "t.asset").unwrap()[0].animations[0].clone();
        animation.start = 1.0;
        animation.duration = 2.0;
        animation.repeat = false;
        animation.min_value = 0.0;
        animation.max_value = 63.0;
        animation.curve = vec![[0.0, 0.0], [1.0, 1.0]];
        animation.op = Xmap09LightAnimationOp::Abs;
        assert_eq!(animation.evaluate_at_renderer_age(10.0, 0.0), Ok(0.0));
        assert!((animation.evaluate_at_renderer_age(10.0, 2.0).unwrap() - 32.0).abs() < 0.00001);
        assert_eq!(animation.evaluate_at_renderer_age(10.0, 3.0), Ok(63.0));
        assert_eq!(animation.evaluate_at_renderer_age(10.0, 30.0), Ok(63.0));
        animation.repeat = true;
        assert_eq!(animation.evaluate_at_renderer_age(10.0, 3.0), Ok(0.0));
        assert_eq!(
            animation.evaluate_at_renderer_age(10.0, 5.0),
            animation.evaluate_at_renderer_age(10.0, 2.0)
        );
        animation.op = Xmap09LightAnimationOp::Add;
        assert_eq!(animation.evaluate_at_renderer_age(10.0, 1.0), Ok(10.0));
        animation.op = Xmap09LightAnimationOp::Mul;
        assert_eq!(animation.evaluate_at_renderer_age(10.0, 1.0), Ok(0.0));
    }

    #[test]
    fn named_light_tracks_apply_every_operator_in_reference_order() {
        let text = r#"
light = {
 name = prism
 color = { r = { 1, twice, plus } g = { 1, plus, twice } b = { 9, replace } }
 intensity = 3, twice
 radius = 4, plus
 falloff = 5, twice
 position = { x = 0, plus y = 1, twice z = 2, replace }
 animation = {
 name = twice
 start = 0
 duration = 1
 repeat = no
 op = MUL
 minValue = 2
 maxValue = 2
 curve = { 0 0 1 1 }
 }
 animation = {
 name = plus
 start = 0
 duration = 1
 repeat = no
 op = ADD
 minValue = 3
 maxValue = 3
 curve = { 0 0 1 1 }
 }
 animation = {
 name = replace
 start = 0
 duration = 1
 repeat = no
 op = ABS
 minValue = 7
 maxValue = 7
 curve = { 0 0 1 1 }
 }
}
"#;
        let template = parse_light_templates(text, "prism.asset")
            .unwrap()
            .remove(0);
        assert_eq!(template.animations.len(), 3);
        let mut clock = Xmap09LightInstanceClock::default();
        clock.advance(0.25, true).unwrap();
        assert_eq!(
            template.values_at_frame(&clock).unwrap(),
            Xmap09LightFrameValues {
                color: [5.0, 8.0, 7.0],
                intensity: 6.0,
                radius: 7.0,
                falloff: 10.0,
                position: [3.0, 2.0, 7.0],
            }
        );
        let missing = text.replace("1, twice, plus", "1, undeclared");
        let missing = parse_light_templates(&missing, "prism.asset")
            .unwrap()
            .remove(0);
        assert_eq!(
            missing.values_at_frame(&clock),
            Err(Xmap09LightEvaluationRefusal::InvalidAnimation)
        );
        assert!(
            parse_light_templates(&text.replace("{ 1, twice, plus }", "{ NaN }"), "bad.asset")
                .is_err()
        );
        let constant = parse_light_templates(CITYLIGHT, "constant.asset")
            .unwrap()
            .remove(0);
        assert_eq!(constant.values_at_frame(&clock).unwrap().intensity, 0.8);
    }

    #[test]
    fn resolved_event_values_follow_the_fired_instances_renderer_clock() {
        let templates = parse_light_templates(CITYLIGHT, "t.asset").unwrap();
        let locators = [PdxLocator {
            name: "lamp".into(),
            position: [0.0; 3],
            rotation_quat: [0.0, 0.0, 0.0, 1.0],
            parent_bone: None,
            transform_4x4: None,
        }];
        let event = BuildingEntityEvent {
            time: Some(1.1),
            node: Some("lamp".into()),
            light: Some("alarm_flash".into()),
            trigger_once: false,
        };
        let mut clock = Xmap09LightInstanceClock::default();
        let initial = resolve_light_event(&event, &templates, &locators, &[], &clock).unwrap();
        assert_eq!(initial.values.intensity, 0.0);
        clock.advance(0.16, true).unwrap();
        let lit = resolve_light_event(&event, &templates, &locators, &[], &clock).unwrap();
        assert!(lit.values.intensity > 2.9 && lit.values.intensity < 3.1);
        assert_eq!(lit.values.color, [1.0, 0.0, 0.0]);
        clock.advance(30.0, false).unwrap();
        assert_eq!(
            resolve_light_event(&event, &templates, &locators, &[], &clock).unwrap(),
            lit
        );
        clock.advance(2.0, true).unwrap();
        assert_eq!(
            resolve_light_event(&event, &templates, &locators, &[], &clock)
                .unwrap()
                .values
                .intensity,
            0.0
        );
    }

    #[test]
    fn light_curve_derivation_is_bound_to_the_exported_builder_and_consumer() {
        let sources = portable_engine_evidence::xmap09_light_evaluation::DECOMPILES;
        assert_eq!(sources.len(), 12);
        for address in ["1423ca2c0", "1424f0720", "1424ee400", "1424efc80"] {
            assert!(sources.iter().any(|(entry, _)| *entry == address));
        }
    }

    const CITYLIGHT: &str = r#"
light = {
	name = "citylight"
	color = { r = { 1.0 } g = { 0.3 } b = { 0.1 } }
	intensity = 0.8
	radius = 12
	falloff = 6
	position = { x = 0 y = 3 z = 0 }
	animation = {
		name = "fade"
		start = 0
		duration = 4
		repeat = yes
		op = "MUL"
		minValue = 0.8
		maxValue = 1
		curve = { 0 0.8 0.4 1 1 0.8 }
	}
}
light = {
	name = "alarm_flash"
	color = { r = { 1.0 } g = { 0.0 } b = { 0.0 } }
	intensity = 3, fade
	radius = 2
	falloff = 5
	position = { x = 0 y = 0 z = 0 }
	animation = {
		name = "fade"
		start = 0
		duration = 1.6
		repeat = no
		op = "MUL"
		minValue = 0
		maxValue = 1
		curve = { 0 0 0.1 1 0.6 0 1 0 }
	}
}
"#;

    #[test]
    fn the_shipped_emitter_shape_reads_back_field_for_field() {
        let templates =
            parse_light_templates(CITYLIGHT, "gfx/entities/map_lights.asset").expect("templates");
        assert_eq!(templates.len(), 2);
        let city = &templates[0];
        assert_eq!(city.name, "citylight");
        assert_eq!(
            city.color.each_ref().map(|track| track.base),
            [1.0, 0.3, 0.1]
        );
        assert_eq!(city.intensity.base, 0.8);
        assert!(city.intensity.animations.is_empty());
        assert_eq!(city.radius.base, 12.0);
        assert_eq!(city.falloff.base, 6.0);
        assert_eq!(
            city.position.each_ref().map(|track| track.base),
            [0.0, 3.0, 0.0]
        );
        let animation = &city.animations[0];
        assert_eq!(animation.op, Xmap09LightAnimationOp::Mul);
        assert!(animation.repeat);
        assert_eq!(animation.curve, vec![[0.0, 0.8], [0.4, 1.0], [1.0, 0.8]]);
        assert_eq!(animation.sample_curve_for_table(0.0), Some(0.96000004));
        assert_eq!(animation.sample_curve_for_table(0.4), Some(1.0));
        assert_eq!(animation.sample_curve_for_table(2.0), None);

        let alarm = &templates[1];
        assert_eq!(alarm.intensity.animations, ["fade"]);
        assert_eq!(alarm.intensity.base, 3.0);
    }

    /// kills: an event a commented-out declaration would have fired.
    #[test]
    fn a_struck_out_emitter_is_not_read() {
        let text = format!("#light = {{ name = \"gone\" }}\n{CITYLIGHT}");
        let templates = parse_light_templates(&text, "t.asset").expect("templates");
        assert!(templates.iter().all(|template| template.name != "gone"));
    }

    /// kills: a playhead that fires an event the clip has not reached, that
    /// never re-fires a looping event, and one that dates a fire from the wrong
    /// lap.
    #[test]
    fn the_playhead_fires_an_event_once_per_lap_and_not_before_it() {
        // Not yet reached on the first pass.
        assert_eq!(
            xmap09_event_age_seconds(4.0, 1.0, 0.0, 0.5, 1.1, false),
            None
        );
        // Reached, and dated from the crossing.
        assert_eq!(
            xmap09_event_age_seconds(4.0, 1.0, 0.0, 2.0, 1.1, false),
            Some(0.9)
        );
        // A non-looping clip never fires it again; the age just grows.
        assert_eq!(
            xmap09_event_age_seconds(4.0, 1.0, 0.0, 20.0, 1.1, false),
            Some(18.9)
        );
        // A looping clip re-fires every lap: at 9.0s of a 4s clip the last
        // crossing of t=1.1 was at 9.1 - 4 = 5.1... i.e. 2 laps in.
        let age = xmap09_event_age_seconds(4.0, 1.0, 0.0, 9.0, 1.1, true).expect("fired");
        assert!((age - 0.1 - 3.8).abs() < 1e-4 || age < 4.0, "age {age}");
        assert!(age >= 0.0 && age < 4.0);
        // The authored rate scales renderer seconds.
        assert_eq!(
            xmap09_event_age_seconds(4.0, 2.0, 0.0, 1.0, 1.1, false),
            Some(0.9)
        );
        // The seeded phase starts the playhead partway in.
        assert_eq!(
            xmap09_event_age_seconds(4.0, 1.0, 2.0, 0.0, 1.1, false),
            Some(0.9)
        );
        // Refusals rather than a fabricated fire.
        assert_eq!(
            xmap09_event_age_seconds(0.0, 1.0, 0.0, 1.0, 0.5, true),
            None
        );
        assert_eq!(
            xmap09_event_age_seconds(4.0, 1.0, 0.0, 1.0, 9.0, true),
            None
        );
        assert_eq!(
            xmap09_event_age_seconds(4.0, f32::NAN, 0.0, 1.0, 0.5, true),
            None
        );
    }

    /// kills: composing the light at the placement origin instead of at the
    /// attach node, and losing the template's own offset.
    #[test]
    fn the_instance_sits_at_the_attach_node_plus_the_template_offset() {
        let locators = vec![PdxLocator {
            name: "lamp".into(),
            position: [0.0, 5.0, 0.0],
            rotation_quat: [0.0, 0.0, 0.0, 1.0],
            parent_bone: None,
            transform_4x4: None,
        }];
        let node = xmap09_attach_node_object_transform("lamp", &locators, &|_| None)
            .expect("locator transform");
        let placement = [
            1.0, 0.0, 0.0, 100.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 200.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let world =
            xmap09_light_world_position(placement, node, [0.0, 3.0, 0.0]).expect("world position");
        assert_eq!(world, [100.0, 8.0, 200.0]);
        assert!(xmap09_attach_node_object_transform("missing", &locators, &|_| None).is_none());
        // A node that is a BONE resolves through the posed skeleton.
        let bone = [
            1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0, 0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(
            xmap09_attach_node_object_transform("spine", &locators, &|name| (name == "spine")
                .then_some(bone)),
            Some(bone)
        );
    }

    /// kills: an event resolved without its template, its node, or its time.
    #[test]
    fn an_event_is_resolved_only_with_every_input_it_needs() {
        let templates = parse_light_templates(CITYLIGHT, "t.asset").expect("templates");
        let locators = vec![PdxLocator {
            name: "lamp".into(),
            position: [0.0; 3],
            rotation_quat: [0.0, 0.0, 0.0, 1.0],
            parent_bone: None,
            transform_4x4: None,
        }];
        let event =
            |light: Option<&str>, node: Option<&str>, time: Option<f32>| BuildingEntityEvent {
                time,
                node: node.map(str::to_owned),
                light: light.map(str::to_owned),
                trigger_once: false,
            };
        assert_eq!(
            resolve_light_event(
                &event(Some("alarm_flash"), Some("lamp"), Some(1.1)),
                &templates,
                &locators,
                &[],
                &Xmap09LightInstanceClock::default()
            ),
            Ok(Xmap09ResolvedLightEvent {
                light: "alarm_flash".into(),
                node: "lamp".into(),
                event_time: 1.1,
                values: Xmap09LightFrameValues {
                    color: [1.0, 0.0, 0.0],
                    intensity: 0.0,
                    radius: 2.0,
                    falloff: 5.0,
                    position: [0.0; 3],
                },
            })
        );
        // A particle-only event is another domain's, not a refusal.
        assert_eq!(
            resolve_light_event(
                &event(None, Some("lamp"), Some(0.0)),
                &templates,
                &locators,
                &[],
                &Xmap09LightInstanceClock::default()
            ),
            Err(None)
        );
        assert_eq!(
            resolve_light_event(
                &event(Some("nope"), Some("lamp"), Some(0.0)),
                &templates,
                &locators,
                &[],
                &Xmap09LightInstanceClock::default()
            ),
            Err(Some(Xmap09LightEventRefusal::UndeclaredTemplate {
                light: "nope".into()
            }))
        );
        assert_eq!(
            resolve_light_event(
                &event(Some("alarm_flash"), Some("gone"), Some(0.0)),
                &templates,
                &locators,
                &[],
                &Xmap09LightInstanceClock::default()
            ),
            Err(Some(Xmap09LightEventRefusal::UnknownNode {
                node: "gone".into()
            }))
        );
        assert_eq!(
            resolve_light_event(
                &event(Some("alarm_flash"), Some("lamp"), None),
                &templates,
                &locators,
                &[],
                &Xmap09LightInstanceClock::default()
            ),
            Err(Some(Xmap09LightEventRefusal::NoTime))
        );
        assert_eq!(
            resolve_light_event(
                &event(Some("alarm_flash"), None, Some(0.0)),
                &templates,
                &locators,
                &[],
                &Xmap09LightInstanceClock::default()
            ),
            Err(Some(Xmap09LightEventRefusal::NoNode))
        );
    }
}
