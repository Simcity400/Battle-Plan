use super::Project;
use content_reader_parser::{crt::{Scanned, scan_i32}, parser::{Item, Value}};
use content_reader_politics::script_constants::Constants;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub country_effects: Vec<Item>,
    pub scientist_effects: Vec<Item>,
    pub facility_state_effects: Vec<Item>,
    pub sub_unit_bonus: Vec<Item>,
    pub equipment_bonus: Vec<Item>,
    pub enable_subunits: Vec<Item>,
    pub enable_equipments: Vec<Item>,
    pub enable_equipment_modules: Vec<Item>,
}

impl Output {
    pub(super) fn read(&mut self, fields: &[Item]) {
        for field in fields {
            let destination = match field.keyword().map(|key| key.folded()).as_deref() {
                Some(b"country_effects") => &mut self.country_effects,
                Some(b"scientist_effects") => &mut self.scientist_effects,
                Some(b"facility_state_effects") => &mut self.facility_state_effects,
                Some(b"sub_unit_bonus") => &mut self.sub_unit_bonus,
                Some(b"equipment_bonus") => &mut self.equipment_bonus,
                Some(b"enable_subunits") => &mut self.enable_subunits,
                Some(b"enable_equipments") => &mut self.enable_equipments,
                Some(b"enable_equipment_modules") => &mut self.enable_equipment_modules,
                _ => continue,
            };
            if field.keyword_is("enable_subunits")
                || field.keyword_is("enable_equipments")
                || field.keyword_is("enable_equipment_modules")
            {
                destination.push(field.clone());
            } else {
                destination.extend(field.value.items().unwrap_or_default().iter().cloned());
            }
        }
    }
}

fn constant(
    field: &Item,
    suffix: Option<&str>,
    constants: Option<&Constants>,
) -> Result<Option<i32>, String> {
    let bytes = field.value.read_string().unwrap_or_default();
    let text = std::str::from_utf8(content_reader_parser::parser::c_string(bytes))
        .map_err(|_| "project integer constant is not UTF-8")?;
    let mut parts: Vec<_> = text
        .strip_prefix("constant:")
        .unwrap_or(text)
        .split('.')
        .collect();
    if let Some(suffix) = suffix {
        parts.push(suffix);
    }
    constants
        .ok_or("project integer constant catalogue is unavailable")?
        .integer(&parts)
}

impl Project {
    pub fn prototype_time(&self, constants: Option<&Constants>) -> Result<i32, String> {
        let mut stored = 0;
        for field in &self.prototype_time {
            match scan_i32(field.value.read_string().unwrap_or_default()) {
                Scanned::Stored(value) => stored = value,
                Scanned::End => {}
                Scanned::NoMatch => {
                    if let Some(value) = constant(field, None, constants)? {
                        stored = value;
                    }
                }
            }
        }
        Ok(stored)
    }

    pub fn complexity(
        &self,
        constants: Option<&Constants>,
        default: i32,
    ) -> Result<(i32, i32), String> {
        let (mut min, mut max) = (default, default);
        for field in &self.complexity {
            match &field.value {
                Value::Block(fields) => {
                    for field in fields {
                        if let Scanned::Stored(value) =
                            scan_i32(field.value.read_string().unwrap_or_default())
                        {
                            if field.keyword_is("min") {
                                min = value;
                            } else if field.keyword_is("max") {
                                max = value;
                            }
                        }
                    }
                }
                _ => {
                    if let Some(value) = constant(field, Some("min"), constants)? {
                        min = value;
                    }
                    if let Some(value) = constant(field, Some("max"), constants)? {
                        max = value;
                    }
                }
            }
            if (min == 0 && max == 0) || (min < 0 && max <= 0) || max < min {
                (min, max) = (default, default);
            }
        }
        Ok((min, max))
    }
}

#[cfg(test)]
mod tests;
