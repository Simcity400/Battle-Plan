//! Authored entity-light curves evaluated on the state event clock.
//! Event transforms follow curve evaluation; lights receive linear RGB times
//! intensity. Curve interpolation remains unverified
//! beyond linear segments; sampled native event timelines would settle it.
use super::{
    entity_animation::Event,
    pointlights::Light,
    weather_effects::{block, curve, get, number, text, tokens},
};
use installed_content_core::{ResolvedLayers, parser};
use std::{collections::BTreeMap, io};

struct Curve {
    start: f32,
    duration: f32,
    repeat: bool,
    op: String,
    knots: Vec<[f32; 2]>,
}
struct Parameter {
    base: f32,
    curve: Option<String>,
}
impl Parameter {
    fn read(items: &[parser::Item], key: &str, default: f32) -> Self {
        let values = get(items, key).map(tokens).unwrap_or_default();
        let mut value = values.first().copied().unwrap_or("").split(',');
        let base = value.next().and_then(|s| s.parse().ok()).unwrap_or(default);
        // `intensity = 1.5, fade` is a scalar followed by an anonymous
        // scalar in the shared grammar; do not lose the curve after the space.
        let curve = value
            .next()
            .and_then(|name| {
                if name.is_empty() {
                    let index = items
                        .iter()
                        .position(|i| i.key.as_deref() == Some(key.as_bytes()))?;
                    items
                        .get(index + 1)
                        .filter(|i| i.key.is_none())?
                        .value
                        .scalar_str()
                } else {
                    Some(name)
                }
            })
            .map(str::to_owned);
        Self { base, curve }
    }
    fn at(&self, curves: &BTreeMap<String, Curve>, age: f32) -> f32 {
        let Some(c) = self.curve.as_ref().and_then(|name| curves.get(name)) else {
            return self.base;
        };
        let time = (age - c.start) / c.duration.max(0.0001);
        let t = if c.repeat {
            time.rem_euclid(1.0)
        } else {
            time.clamp(0.0, 1.0)
        };
        let value = curve(&c.knots, t);
        match c.op.as_str() {
            "ADD" => self.base + value,
            "ABS" => value,
            _ => self.base * value,
        }
    }
}

struct Definition {
    color: [Parameter; 3],
    position: [Parameter; 3],
    intensity: Parameter,
    radius: Parameter,
    falloff: Parameter,
    curves: BTreeMap<String, Curve>,
}
#[derive(Default)]
pub struct Lights {
    definitions: BTreeMap<String, Definition>,
}
impl Lights {
    /// Every `light` the engine registers from `gfx/**/*.asset`, as the shared
    /// descriptor roster holds them.
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let roster = layers.descriptor_roster()?;
        let mut definitions = BTreeMap::new();
        for v in roster.lights.values() {
            let mut curves = BTreeMap::new();
            for a in v
                .iter()
                .filter(|i| i.key.as_deref() == Some(b"animation"))
                .filter_map(|i| i.value.items())
            {
                let values = get(a, "curve")
                    .map(tokens)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|v| v.parse::<f32>().ok())
                    .collect::<Vec<_>>();
                curves.insert(
                    text(a, "name").to_owned(),
                    Curve {
                        start: number(a, "start", 0.0),
                        duration: number(a, "duration", 1.0),
                        repeat: installed_content_core::parser::read_bool_field(a, "repeat", false),
                        op: text(a, "op").to_owned(),
                        knots: values.chunks_exact(2).map(|v| [v[0], v[1]]).collect(),
                    },
                );
            }
            definitions.insert(
                text(v, "name").to_owned(),
                Definition {
                    color: ["r", "g", "b"].map(|k| Parameter::read(block(v, "color"), k, 1.0)),
                    position: ["x", "y", "z"]
                        .map(|k| Parameter::read(block(v, "position"), k, 0.0)),
                    intensity: Parameter::read(v, "intensity", 1.0),
                    radius: Parameter::read(v, "radius", 1.0),
                    falloff: Parameter::read(v, "falloff", 1.0),
                    curves,
                },
            );
        }
        Ok(Self { definitions })
    }
    /// The lights the shown events give; a hidden event contributes none.
    pub fn evaluate(&self, events: &[Event]) -> Vec<Light> {
        events
            .iter()
            .filter(|event| !event.hidden)
            .filter_map(|event| {
                let definition = self.definitions.get(event.definition.light.as_ref()?)?;
                let at = |p: &Parameter| p.at(&definition.curves, event.age);
                let intensity = at(&definition.intensity);
                if intensity <= 0.0 {
                    return None;
                }
                Some(Light {
                    position: event
                        .transform
                        .transform_point(definition.position.each_ref().map(at)),
                    color: definition.color.each_ref().map(at),
                    intensity,
                    radius: at(&definition.radius),
                    falloff: at(&definition.falloff),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spaced_curve_reference_expires_the_flash() {
        let document = parser::parse(b"intensity = 1.5, fade radius = 5.7").unwrap();
        let parameter = Parameter::read(&document.items, "intensity", 1.0);
        let curves = BTreeMap::from([(
            "fade".into(),
            Curve {
                start: 0.0,
                duration: 0.5,
                repeat: false,
                op: "MUL".into(),
                knots: vec![[0.0, 0.0], [0.1, 1.0], [0.6, 0.0], [1.0, 0.0]],
            },
        )]);
        assert_eq!(parameter.at(&curves, 0.0), 0.0);
        assert_eq!(parameter.at(&curves, 0.05), 1.5);
        assert_eq!(parameter.at(&curves, 0.5), 0.0);
        assert_eq!(parameter.at(&curves, 100.0), 0.0);
    }
}
