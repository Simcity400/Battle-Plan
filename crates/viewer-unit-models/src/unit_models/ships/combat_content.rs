
use std::{collections::BTreeMap, io};

use content_reader_parser::{Diagnostic, DiagnosticKind, parser};
use content_reader_mesh::mesh;
use content_reader_script::native;

use viewer_models::{entity_animation::Rig, mesh_pose};

use super::NAVAL_COMBAT_FILE;

#[derive(Debug, Clone)]
pub(super) struct Config {
    formation: Vec<u8>,
    formation_scale: f32,
    combat_formation: Vec<u8>,
    combat_scale: f32,
    death_time: f32,
    random_start_time: f32,
    effects: [Vec<Vec<u8>>; 4],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            formation: Vec::new(),
            formation_scale: 1.0,
            combat_formation: Vec::new(),
            combat_scale: 1.0,
            death_time: 10.0,
            random_start_time: 1.0,
            effects: Default::default(),
        }
    }
}

impl Config {
    pub(super) fn read(bytes: &[u8], warnings: &mut Vec<Diagnostic>) -> Self {
        let mut config = Self::default();
        let mut reports = BTreeMap::<Diagnostic, usize>::new();
        let mut report = |kind, reason| *reports.entry((kind, reason)).or_default() += 1;
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::Persistent(()),
            None,
            &mut |(), field| {
                let keyword = field.keyword();
                if keyword.as_deref().and_then(effect_index).is_some() {
                    return native::Shape::List;
                }
                let stored_string = match keyword.as_deref() {
                    Some(b"naval_formation") => Some(&mut config.formation),
                    Some(b"naval_combat_formation") => Some(&mut config.combat_formation),
                    _ => None,
                };
                if let Some(destination) = stored_string {
                    *destination = parser::read_string(&field.value.text).to_vec();
                    if field.value.c_str().contains(&b'\n') {
                        report(
                            DiagnosticKind::Misread,
                            format!(
                                "line {}: {:?}: newline stored as Unreadable String",
                                field.key.line,
                                String::from_utf8_lossy(field.key.c_str())
                            ),
                        );
                        return native::Shape::Skip;
                    }
                    return native::Shape::One;
                }
                let stored_float = match keyword.as_deref() {
                    Some(b"naval_formation_scale") => Some(&mut config.formation_scale),
                    Some(b"naval_combat_formation_scale") => Some(&mut config.combat_scale),
                    Some(b"naval_death_time") => Some(&mut config.death_time),
                    Some(b"naval_random_start_time") => Some(&mut config.random_start_time),
                    _ => None,
                };
                if let Some(destination) = stored_float {
                    if let Some(value) = content_reader_parser::crt::scan_f32(&field.value.text) {
                        *destination = value;
                        return native::Shape::One;
                    }
                    report(
                        DiagnosticKind::Misread,
                        format!(
                            "line {}: {:?}: no float stored, previous value retained",
                            field.key.line,
                            String::from_utf8_lossy(field.key.c_str())
                        ),
                    );
                    return if field.skips_block() {
                        native::Shape::Skip
                    } else {
                        native::Shape::One
                    };
                }
                report(
                    DiagnosticKind::Misread,
                    format!(
                        "line {}: unexpected token {:?}",
                        field.key.line,
                        String::from_utf8_lossy(field.key.c_str())
                    ),
                );
                native::Shape::Skip
            },
        );
        for item in &read.items {
            let Some(index) = item
                .keyword()
                .and_then(|keyword| effect_index(&keyword.folded()))
            else {
                continue;
            };
            for entry in item.value.items().into_iter().flatten() {
                if let Some(raw) = entry.value.scalar() {
                    if parser::c_string(raw).contains(&b'\n') {
                        report(
                            DiagnosticKind::Misread,
                            format!(
                                "{}: effect-list newline stored as Unreadable String",
                                String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
                            ),
                        );
                    }
                    config.effects[index].push(parser::read_string(raw).to_vec());
                }
            }
        }
        for misread in read.misreads {
            report(
                misread.kind,
                format!("line {}: {}", misread.line, misread.what),
            );
        }
        warnings.extend(reports.into_iter().map(|((kind, reason), count)| {
            (
                kind,
                format!("{NAVAL_COMBAT_FILE}: {count} reader reports: {reason}"),
            )
        }));
        config
    }

    pub(super) fn formation(&self) -> io::Result<(String, f32)> {
        let formation = text(&self.formation, "naval_formation")?;
        finite(self.formation_scale, "naval_formation_scale")?;
        Ok((formation, self.formation_scale))
    }
}

const EFFECT_FIELDS: [&str; 4] = [
    "naval_hit_effects_big",
    "naval_hit_effects_small",
    "naval_miss_effects_big",
    "naval_miss_effects_small",
];

fn effect_index(keyword: &[u8]) -> Option<usize> {
    EFFECT_FIELDS
        .iter()
        .position(|field| field.as_bytes() == keyword)
}

fn invalid(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

fn text(bytes: &[u8], field: &str) -> io::Result<String> {
    parser::text(bytes)
        .map(str::to_owned)
        .map_err(|why| invalid(format!("{NAVAL_COMBAT_FILE}: 1 refused {field}: {why}")))
}

fn finite(value: f32, field: &str) -> io::Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(invalid(format!(
            "{NAVAL_COMBAT_FILE}: 1 refused {field}: non-finite stored value {value}"
        )))
    }
}

#[derive(Debug, Clone)]
pub(super) struct CombatAssets {
    pub template_entity: String,
    pub scale: f32,
    pub death_time: f32,
    pub random_start_time: f32,
    pub effects: [Vec<String>; 4],
}

impl CombatAssets {
    pub(super) fn load(descriptors: &mesh::Descriptors, config: &Config) -> io::Result<Self> {
        let template_entity = text(&config.combat_formation, "naval_combat_formation")?;
        for (value, field) in [
            (config.combat_scale, "naval_combat_formation_scale"),
            (config.death_time, "naval_death_time"),
            (config.random_start_time, "naval_random_start_time"),
        ] {
            finite(value, field)?;
        }
        if !descriptors.entities.contains_key(&template_entity) {
            return Err(invalid(format!(
                "{NAVAL_COMBAT_FILE}: 1 refused naval_combat_formation: entity {template_entity:?} is not registered"
            )));
        }
        let mut effects: [Vec<String>; 4] = Default::default();
        for (index, stored) in config.effects.iter().enumerate() {
            for entry in stored {
                effects[index].push(text(entry, EFFECT_FIELDS[index])?);
            }
        }
        Ok(Self {
            template_entity,
            scale: config.combat_scale,
            death_time: config.death_time,
            random_start_time: config.random_start_time,
            effects,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Attachment {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SideLocators {
    pub big: Option<Attachment>,
    pub small: Vec<Attachment>,
}

impl SideLocators {
    pub(super) fn choose(
        &self,
        random: &mut impl FnMut() -> u32,
    ) -> io::Result<[Option<Attachment>; 4]> {
        if self.small.len() < 3 {
            return Err(invalid(format!(
                "naval_combat_formation: 1 refused support selection: {} positions; native selection requires three",
                self.small.len()
            )));
        }
        let mut slots = std::array::from_fn(|_| None);
        slots[0] = self.big.clone();
        let mut remaining: Vec<usize> = (0..self.small.len()).collect();
        for slot in &mut slots[1..] {
            let index = ((random() & 0x7fff_ffff) as usize) % remaining.len();
            *slot = Some(self.small[remaining.swap_remove(index)].clone());
        }
        Ok(slots)
    }
}

pub(super) fn select_locators(rig: &Rig) -> io::Result<[SideLocators; 2]> {
    let root = rig.nodes.first().ok_or_else(|| {
        invalid("naval_combat_formation: 1 refused template: baked rig has no root actor")
    })?;
    let source = root.native.as_ref().ok_or_else(|| {
        invalid("naval_combat_formation: 1 refused template: root actor source is unavailable")
    })?;
    let attachment = |name: &str| -> io::Result<Option<Attachment>> {
        let binding = mesh_pose::lookup(source, name, mesh_pose::LookupMode::Naval);
        if let Some(mesh_pose::Binding::Locator(locator)) = binding
            && let Some(why) = &locator.error
        {
            return Err(invalid(format!("naval_combat_formation: {why}")));
        }
        Ok(binding.map(|_| Attachment {
            name: name.to_owned(),
        }))
    };
    let side = |prefix: &str| -> io::Result<SideLocators> {
        let mut big = None;
        for number in 1..10 {
            big = attachment(&format!("{prefix}_big_{number}"))?;
            if big.is_some() {
                break;
            }
        }
        let mut small = Vec::new();
        for number in 1..10 {
            if let Some(found) = attachment(&format!("{prefix}_small_{number}"))? {
                small.push(found);
            }
        }
        if small.len() < 3 {
            return Err(invalid(format!(
                "naval_combat_formation: 1 refused {prefix}: {} support positions; native three-slot selection would read out of bounds",
                small.len()
            )));
        }
        Ok(SideLocators { big, small })
    };
    Ok([side("defend_position")?, side("attack_position")?])
}

#[cfg(test)]
#[path = "combat_content/tests.rs"]
mod tests;
