use content_reader_mesh::mesh::MeshProducts as _;
use viewer_models::entity_animation::Event;
use viewer_scene_base::pointlights::Light;
use super::weather_effects::{Curve, block, get, text, tokens};
use content_reader_layers::ResolvedLayers;
use content_reader_parser::parser;
use std::{collections::BTreeMap, io};

struct Parameter {
    base: f32,
    curve: Option<String>,
}
impl Parameter {
    fn read(items: &[parser::Item], key: &str, default: f32) -> Self {
        let values = get(items, key).map(tokens).unwrap_or_default();
        let mut value = values.first().copied().unwrap_or("").split(',');
        let base = value.next().and_then(|s| s.parse().ok()).unwrap_or(default);
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
        c.apply(self.base, age)
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
    pub fn load(layers: &ResolvedLayers, withheld: &mut Vec<String>) -> io::Result<Self> {
        let roster = layers.descriptor_roster()?;
        let mut definitions = BTreeMap::new();
        for v in roster.lights.values() {
            let light_name = text(v, "name");
            let mut curves = BTreeMap::new();
            let mut failed = false;
            for a in v
                .iter()
                .filter(|i| i.key.as_deref() == Some(b"animation"))
                .filter_map(|i| i.value.items())
            {
                match Curve::parse(a) {
                    Ok(curve) => {
                        curves.insert(text(a, "name").to_owned(), curve);
                    }
                    Err(error) => {
                        withheld.push(format!("entity light {light_name:?}: {error}"));
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                continue;
            }
            definitions.insert(
                light_name.to_owned(),
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
        let animation = parser::parse(
            b"name = fade start = 0 duration = 0.5 repeat = no op = MUL \
              minValue = 0 maxValue = 1 curve = { 0 0  0.1 1  0.6 0  1 0 }",
        )
        .unwrap();
        let curve = Curve::parse(&animation.items).unwrap();
        let expected_peak = 1.5 * curve.sample(0.05);
        let curves = BTreeMap::from([("fade".into(), curve)]);
        assert_eq!(parameter.at(&curves, 0.0), 0.0);
        assert!((parameter.at(&curves, 0.05) - expected_peak).abs() < 1e-6);
        assert!(parameter.at(&curves, 0.05) > 1.4);
        assert_eq!(parameter.at(&curves, 0.5), 0.0);
        assert_eq!(parameter.at(&curves, 100.0), 0.0);
    }

    #[test]
    fn light_curves_use_native_catmull_rom_lut_repeat_and_operations() {
        let parse_curve = |src: &[u8]| Curve::parse(&parser::parse(src).unwrap().items);
        let plateau = parse_curve(
            b"name = plateau start = 0 duration = 1 op = mul \
              minValue = 0 maxValue = 1 curve = { 0 0  0.25 1  0.75 1  1 0 }",
        )
        .unwrap();
        let curves = BTreeMap::from([("plateau".into(), plateau)]);
        let parameter = Parameter {
            base: 1.0,
            curve: Some("plateau".into()),
        };
        assert!(parameter.at(&curves, 0.5) > 1.12);

        let remapped = parse_curve(
            b"name = remap start = 0 duration = 4 repeat = yes op = MUL \
              minValue = 0.8 maxValue = 1.0 curve = { 0 0.8  0.4 1.0  1 0.8 }",
        )
        .unwrap();
        let curves = BTreeMap::from([("remap".into(), remapped)]);
        let parameter = Parameter {
            base: 3.0,
            curve: Some("remap".into()),
        };
        assert!((parameter.at(&curves, 0.0) - 3.0 * (0.8 + 0.8 * 0.2)).abs() < 1e-5);
        assert!((parameter.at(&curves, 4.0) - parameter.at(&curves, 0.0)).abs() < 1e-5);

        let repeated = parse_curve(
            b"name = rep start = 1 duration = 2 repeat = yes op = add \
              minValue = 0 maxValue = 1 curve = { 0 0  1 1 }",
        )
        .unwrap();
        let curves = BTreeMap::from([("rep".into(), repeated)]);
        let parameter = Parameter {
            base: 10.0,
            curve: Some("rep".into()),
        };
        assert!((parameter.at(&curves, 3.5) - 10.0).abs() < 1e-6);
        assert!((parameter.at(&curves, 5.0) - (10.0 + 32.0 / 63.0)).abs() < 1e-5);

        let replaced = parse_curve(
            b"name = abs_curve start = 0 duration = 0 op = abs \
              minValue = 0 maxValue = 2 curve = { 0 0.5  1 0.5 }",
        )
        .unwrap();
        let curves = BTreeMap::from([("abs_curve".into(), replaced)]);
        let parameter = Parameter {
            base: 10.0,
            curve: Some("abs_curve".into()),
        };
        assert!((parameter.at(&curves, 0.5) - 1.0).abs() < 1e-6);

        let short = parse_curve(b"name = short op = mul curve = { 0 1 }").unwrap();
        let curves = BTreeMap::from([("short".into(), short)]);
        let parameter = Parameter {
            base: 10.0,
            curve: Some("short".into()),
        };
        assert_eq!(parameter.at(&curves, 0.5), 0.0);

        let bad_op = parse_curve(b"name = bad op = sub curve = { 0 0  1 1 }")
            .unwrap_err()
            .to_string();
        assert!(bad_op.contains("bad") && bad_op.contains("sub"), "{bad_op}");

        let bad_domain = parse_curve(b"name = dup op = mul curve = { 0.5 0  0.5 1 }")
            .unwrap_err()
            .to_string();
        assert!(bad_domain.contains("dup"), "{bad_domain}");
    }

    #[test]
    fn an_unsupported_light_curve_withholds_only_that_light() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_entity_light_withheld_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("gfx/entities/test_lights.asset");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            b"light = {\n\
                name = \"good_light\"\n\
                color = { r = { 1 } g = { 0.5 } b = { 0.25 } }\n\
                intensity = 2\n\
                radius = 10\n\
                falloff = 2\n\
                position = { x = 0 y = 0 z = 0 }\n\
              }\n\
              light = {\n\
                name = \"unused_bad_light\"\n\
                color = { r = { 1 } g = { 1 } b = { 1 } }\n\
                intensity = 1, bad_curve\n\
                radius = 5\n\
                falloff = 1\n\
                position = { x = 0 y = 0 z = 0 }\n\
                animation = {\n\
                  name = \"bad_curve\"\n\
                  start = 0 duration = 1 op = \"sub\"\n\
                  curve = { 0 0 1 1 }\n\
                }\n\
              }\n",
        )
        .unwrap();
        let layers =
            ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let mut withheld = Vec::new();
        let lights = Lights::load(&layers, &mut withheld).unwrap();
        assert!(lights.definitions.contains_key("good_light"));
        assert!(!lights.definitions.contains_key("unused_bad_light"));
        assert_eq!(withheld.len(), 1, "{withheld:?}");
        assert!(
            withheld[0].contains("unused_bad_light") && withheld[0].contains("sub"),
            "{:?}",
            withheld[0]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
