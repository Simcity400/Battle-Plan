
use crate::sound::Notes;
use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt, parser};
use content_reader_defines::defines;
use std::io;

pub const FOLDER: &str = "sound/combat_sounds";

#[derive(Clone, Debug, PartialEq)]
pub struct CombatSound {
    pub name: String,
    pub effect: String,
    pub units: Vec<String>,
    pub min: i32,
    pub max: Option<i32>,
    pub source: String,
}

impl CombatSound {
    pub fn sounds(&self, count: usize) -> bool {
        let count = i64::try_from(count).unwrap_or(i64::MAX);
        count > 0
            && count >= i64::from(self.min)
            && self.max.is_none_or(|max| count <= i64::from(max))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layers {
    pub sounds: Vec<CombatSound>,
    pub slots: Vec<(String, Vec<usize>)>,
    pub notes: Notes,
}

impl Layers {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let listed = layers.descriptor_paths(FOLDER, &[".txt"])?;
        let paths = defines::load_order(&format!("{FOLDER}/"), ".txt", &listed);
        let mut out = Self::default();
        for file in paths {
            let bytes = file.read_uncovered(layers)?;
            let path = file.text();
            match parser::parse(&bytes) {
                Ok(document) => out.read_file(&document.items, &path),
                Err(error) => out
                    .notes
                    .note(format!("{path}: unreadable battle sound file ({error})")),
            }
        }
        for (index, sound) in out.sounds.iter().enumerate() {
            for unit in &sound.units {
                match out.slots.iter_mut().find(|(name, _)| name == unit) {
                    Some((_, sounds)) => sounds.push(index),
                    None => out.slots.push((unit.clone(), vec![index])),
                }
            }
        }
        Ok(out)
    }

    fn read_file(&mut self, items: &[parser::Item], path: &str) {
        for item in items {
            let name = item
                .key
                .as_deref()
                .map(|key| String::from_utf8_lossy(key).into_owned())
                .unwrap_or_default();
            let Some(fields) = item.value.items() else {
                self.notes.note(format!(
                    "{path}: refused battle sound {name} without a block"
                ));
                continue;
            };
            if self.sounds.iter().any(|sound| sound.name == name) {
                self.notes
                    .note(format!("{path}: refused repeated battle sound {name}"));
                continue;
            }
            let mut effect = None;
            let mut units = Vec::new();
            let mut range = None;
            for field in fields {
                if field.keyword_is("sound_effect") {
                    effect = field.value.scalar_str().map(str::to_owned);
                } else if field.keyword_is("units") {
                    units.extend(
                        field
                            .value
                            .items()
                            .unwrap_or_default()
                            .iter()
                            .filter(|unit| unit.key.is_none())
                            .filter_map(|unit| unit.value.scalar_str())
                            .map(str::to_owned),
                    );
                } else if field.keyword_is("divisions_range") {
                    let values: Option<Vec<i32>> = field.value.items().and_then(|values| {
                        values
                            .iter()
                            .map(|value| {
                                value
                                    .key
                                    .is_none()
                                    .then(|| value.value.scalar().and_then(crt::scan_f32))
                                    .flatten()
                                    .filter(|value| value.is_finite() && value.fract() == 0.0)
                                    .map(|value| value as i32)
                            })
                            .collect()
                    });
                    match values.as_deref() {
                        Some(&[min, max]) => range = Some((min, (max != -1).then_some(max))),
                        _ => self.notes.note(format!(
                            "{path}: battle sound {name} refused a malformed divisions_range"
                        )),
                    }
                } else {
                    self.notes.note(format!(
                        "{path}: battle sound {name} refused field {}",
                        field
                            .key
                            .as_deref()
                            .map(String::from_utf8_lossy)
                            .unwrap_or_default()
                    ));
                }
            }
            let (Some(effect), Some((min, max))) = (effect, range) else {
                self.notes.note(format!(
                    "{path}: refused battle sound {name} without a sound_effect and divisions_range"
                ));
                continue;
            };
            self.sounds.push(CombatSound {
                name,
                effect,
                units,
                min,
                max,
                source: path.to_owned(),
            });
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BattleDefines {
    pub effect: String,
    pub falloff_default: f32,
    pub min_unit_count: i32,
}

impl BattleDefines {
    pub fn load(defines: &defines::Defines) -> io::Result<Self> {
        let number = |key: &str| -> io::Result<f32> {
            let value = defines.number("NSound", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("NDefines.NSound.{key} is outside f32"),
                ))
            }
        };
        Ok(Self {
            effect: defines.text("NSound", "BATTLE_SOUND_NAME")?,
            falloff_default: number("BATTLE_SOUND_FALLOFF_DEFAULT")?,
            min_unit_count: defines.int32("NSound", "BATTLE_SOUND_MIN_UNIT_COUNT")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_layers::test_support::Stack;

    #[test]
    fn definitions_share_slots_by_sub_unit_type_and_refuse_by_name() {
        let stack = Stack::new("combat_sounds");
        let base = stack.base();
        stack.write(
            &base,
            "sound/combat_sounds/00_combat_sounds.txt",
            r#"rifles = { sound_effect = "rifle_layers" units = { infantry marine } divisions_range = { 5 -1 } }
               mgs = { sound_effect = "mg_layers" units = { infantry } divisions_range = { 40 60 } }
               rifles = { sound_effect = "other" units = { infantry } divisions_range = { 1 -1 } }
               broken = { sound_effect = "x" units = { tank } divisions_range = { 2 } }
               odd = { sound_effect = "y" units = { tank } divisions_range = { 2 -1 } tempo = fast }"#,
        );
        let layers = Layers::read(&stack.layers(&[])).unwrap();
        let names: Vec<_> = layers
            .sounds
            .iter()
            .map(|sound| sound.name.as_str())
            .collect();
        assert_eq!(names, ["rifles", "mgs", "odd"]);
        assert_eq!(
            layers.slots,
            [
                ("infantry".to_owned(), vec![0, 1]),
                ("marine".to_owned(), vec![0]),
                ("tank".to_owned(), vec![2]),
            ]
        );
        assert_eq!(layers.sounds[1].max, Some(60));
        assert_eq!(layers.sounds[0].max, None);
        assert!(layers.sounds[0].sounds(5) && !layers.sounds[0].sounds(4));
        assert!(layers.sounds[1].sounds(60) && !layers.sounds[1].sounds(61));
        let notes = layers.notes.lines();
        for expected in [
            "sound/combat_sounds/00_combat_sounds.txt: refused repeated battle sound rifles",
            "sound/combat_sounds/00_combat_sounds.txt: battle sound broken refused a malformed divisions_range",
            "sound/combat_sounds/00_combat_sounds.txt: battle sound odd refused field tempo",
        ] {
            assert!(
                notes.contains(&expected.to_owned()),
                "{expected}: {notes:?}"
            );
        }
    }

    #[test]
    fn escaped_quotes_stay_part_of_effect_and_sub_unit_names() {
        let stack = Stack::new("combat_sounds_quotes");
        stack.write(
            &stack.base(),
            "sound/combat_sounds/00_combat_sounds.txt",
            r#"quoted = { sound_effect = "\"layers\"" units = { "\"infantry\"" infantry } divisions_range = { 1 -1 } }"#,
        );
        let layers = Layers::read(&stack.layers(&[])).unwrap();
        assert_eq!(layers.sounds[0].effect, "\"layers\"");
        assert_eq!(layers.sounds[0].units, ["\"infantry\"", "infantry"]);
    }

    #[test]
    fn no_sub_unit_sounds_nothing_even_from_a_zero_minimum() {
        let sound = CombatSound {
            name: "any".into(),
            effect: "e".into(),
            units: vec!["infantry".into()],
            min: 0,
            max: None,
            source: String::new(),
        };
        assert!(!sound.sounds(0));
        assert!(sound.sounds(1));
    }
}
