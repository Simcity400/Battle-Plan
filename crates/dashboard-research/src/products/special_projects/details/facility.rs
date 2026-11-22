
use super::{Namer, ProjectLabel};
use engine_projects::program_details as kinds;
use replay_data::rows::{ProjectFacilityOperand, ProjectProgramRow, Recorded, tables};
use replay_frame::frame::DecodedFrame;
use serde::Serialize;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FacilitySource {
    pub kind: String,
    pub unit: String,
    pub label: Option<ProjectLabel>,
    pub modifier: Option<ProjectLabel>,
    pub assignment: u32,
    pub scientist_reference: String,
    pub value_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FacilityDetails {
    pub speed_raw: Option<String>,
    pub sources: Vec<FacilitySource>,
    pub unavailable: Option<String>,
}

fn multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}

fn total(program: &ProjectProgramRow, sources: &[ProjectFacilityOperand]) -> Result<i64, String> {
    let scalar = |kind| sources.iter().find(|source| source.kind == kind).map(|source| source.raw).ok_or_else(|| "Facility speed lacks a required scalar".to_string());
    let skills = sources.iter().filter(|source| source.kind == kinds::SKILL).fold(100_000, |value, source| multiply(value, source.raw));
    let assignments = match &program.assignments {
        Recorded::Value(Some(values)) => values,
        _ => return Err("Facility speed lacks its assignment occurrences".into()),
    };
    if program.active_project.is_some() && sources.iter().filter(|source| source.kind == kinds::SKILL).count() != assignments.len() {
        return Err("Facility speed lacks an assigned scientist skill factor".into());
    }
    let mut modifiers = 100_000;
    for country in sources.iter().filter(|source| source.kind == kinds::COUNTRY) {
        let state = sources.iter().find(|source| source.kind == kinds::STATE && source.ordinal == country.ordinal).ok_or("Facility speed lacks a state modifier")?;
        modifiers = multiply(state.raw, multiply(country.raw, modifiers));
        let mut scientists = 100_000;
        for ordinal in 0..assignments.len() {
            let mut scientist = sources
                .iter()
                .find(|source| source.kind == kinds::INJURY && source.ordinal == country.ordinal && source.assignment as usize == ordinal)
                .map_or(100_000, |source| source.raw);
            for source in sources.iter().filter(|source| source.kind == kinds::TRAIT && source.ordinal == country.ordinal && source.assignment as usize == ordinal) {
                scientist = multiply(scientist, source.raw);
            }
            scientists = multiply(scientists, scientist);
        }
        modifiers = multiply(scientists, modifiers);
    }
    Ok(multiply(multiply(multiply(multiply(scalar(kinds::SUPPLY)?, skills), modifiers.max(0)), scalar(kinds::RESOURCES)?), scalar(kinds::BUILDING)?))
}

fn kind(value: u32) -> (&'static str, &'static str) {
    match value {
        kinds::BUILDING => ("Building health", "factor"),
        kinds::SUPPLY => ("Supply utilization", "factor"),
        kinds::RESOURCES => ("Acquired resources", "factor"),
        kinds::BASIC => ("Basic research speed", "factor"),
        kinds::SKILL => ("Scientist skill", "factor"),
        kinds::COUNTRY => ("Country modifier", "factor"),
        kinds::STATE => ("State modifier", "factor"),
        kinds::TRAIT => ("Scientist trait", "factor"),
        kinds::INJURY => ("Scientist injury", "factor"),
        kinds::BREAKTHROUGH => ("Daily breakthrough progress", "progress"),
        kinds::TECHNOLOGY => ("Technology research bonus", "bonus"),
        kinds::EXPERIENCE => ("Daily scientist experience", "experience"),
        _ => ("Unknown facility operand", "unknown"),
    }
}

pub(super) fn present(frame: &DecodedFrame, program: &ProjectProgramRow, namer: &mut Namer<'_, '_, '_>, unavailable: &mut Vec<String>) -> Option<FacilityDetails> {
    let Recorded::Value(Some(values)) = &program.facility else { return None };
    let speed = total(program, values);
    let modifiers = frame.complete::<tables::ModifierDefinitions>().ok();
    let sources = values
        .iter()
        .map(|value| {
            let (kind, unit) = kind(value.kind);
            let label = (value.token != 0).then(|| namer.token(frame, value.token, unavailable));
            let modifier = (value.key != 0).then(|| {
                let definition = modifiers.and_then(|rows| rows.binary_search_by_key(&value.key, |row| row.index).ok().map(|at| &rows[at]));
                let key = definition.map_or_else(|| value.key.to_string(), |row| String::from_utf8_lossy(&row.name_key).into_owned());
                namer.label(key)
            });
            let raw = if value.kind == kinds::BREAKTHROUGH { value.raw.wrapping_mul(100_000) / 10_000_000 } else { value.raw };
            FacilitySource {
                kind: kind.into(),
                unit: unit.into(),
                label,
                modifier,
                assignment: value.assignment,
                scientist_reference: value.scientist.to_string(),
                value_raw: raw.to_string(),
            }
        })
        .collect();
    Some(FacilityDetails { speed_raw: speed.as_ref().ok().map(ToString::to_string), unavailable: speed.err(), sources })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::ProjectScientistAssignment;

    #[test]
    fn facility_speed_keeps_native_tag_order_scientist_grouping_and_duplicate_occurrences() {
        let mut program = ProjectProgramRow { active_project: Some(7), assignments: Recorded::Value(Some(vec![ProjectScientistAssignment::default(); 2])), ..Default::default() };
        let operand = |kind, ordinal, assignment, raw| ProjectFacilityOperand { kind, ordinal, assignment, raw, ..Default::default() };
        let mut values = vec![
            operand(kinds::BUILDING, 0, 0, 75123),
            operand(kinds::SUPPLY, 0, 0, 87654),
            operand(kinds::RESOURCES, 0, 0, 65432),
            operand(kinds::SKILL, 0, 0, 112345),
            operand(kinds::SKILL, 0, 1, 103456),
        ];
        for ordinal in 0..2 {
            values.extend([
                operand(kinds::COUNTRY, ordinal, 0, 125678),
                operand(kinds::STATE, ordinal, 0, 92345),
                operand(kinds::INJURY, ordinal, 0, 50000),
                operand(kinds::TRAIT, ordinal, 0, 112345),
                operand(kinds::TRAIT, ordinal, 0, 110001),
                operand(kinds::INJURY, ordinal, 1, 100000),
                operand(kinds::TRAIT, ordinal, 1, 104321),
            ]);
        }
        let one_scientist = multiply(multiply(50000, 112345), 110001);
        let scientists = multiply(one_scientist, 104321);
        let modifier = |previous| multiply(scientists, multiply(92345, multiply(125678, previous)));
        let expected = multiply(multiply(multiply(multiply(87654, multiply(112345, 103456)), modifier(modifier(100000))), 65432), 75123);
        assert_eq!(total(&program, &values).unwrap(), expected);
        values.retain(|source| source.kind != kinds::SKILL || source.assignment == 0);
        assert!(total(&program, &values).is_err());
        program.active_project = None;
        values.retain(|source| matches!(source.kind, kinds::BUILDING | kinds::SUPPLY | kinds::RESOURCES));
        assert_eq!(total(&program, &values).unwrap(), multiply(multiply(87654, 65432), 75123));
    }
}
