
use content_reader_division_names::division_names::Database;
use content_reader_localisation::localisation::Localisation;
#[cfg(test)]
use replay_data::rows::DivisionStatusRow;
use replay_data::{rows::NameOperands, rows::Recorded, rows::TaskForceCounterInputRow};

pub fn task_force_name(
    row: &TaskForceCounterInputRow,
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<String, String> {
    let bytes = row
        .name
        .require("name of task force counter inputs")?
        .as_deref()
        .ok_or_else(|| {
            format!(
                "name of task force counter inputs unavailable for task force {:#x}: guarded read or name arena refusal",
                row.task_force
            )
        })?;
    crate::scripted_text::stored_text(
        localisation,
        recorded_language,
        bytes,
        "task force cached name",
    )
}

pub fn army_name<'a, 'b>(
    status: impl Into<replay_frame::frame::DivisionName<'b>>,
    localisation: &Localisation,
    recorded_language: &str,
    legacy_names: impl FnOnce() -> Result<&'a Database, String>,
) -> Result<String, String> {
    let status = status.into();
    match status.name {
        Recorded::Value(Some(bytes)) => {
            let literal = matches!(
                status.naming,
                Recorded::Value(Some(naming))
                    if naming.programmatic == 0 && naming.custom == *bytes
            );
            if literal {
                String::from_utf8(bytes.clone())
                    .map_err(|_| "division cached literal name is not UTF-8".into())
            } else {
                crate::scripted_text::stored_text(
                    localisation,
                    recorded_language,
                    bytes,
                    "division cached name",
                )
            }
        }
        Recorded::Value(None) => Err(format!(
            "name of division status unavailable for UNIT {:#x}: guarded read or name arena refusal",
            status.division
        )),
        Recorded::NotRecorded => {
            let naming = status
                .naming
                .require("naming of division status")?
                .as_ref()
                .ok_or_else(|| format!(
                    "naming of division status unavailable for UNIT {:#x}: guarded read or arena shortage",
                    status.division
                ))?;
            legacy_army_name(naming, localisation, recorded_language, legacy_names)
        }
    }
}

fn legacy_army_name<'a>(
    naming: &NameOperands,
    localisation: &Localisation,
    recorded_language: &str,
    names: impl FnOnce() -> Result<&'a Database, String>,
) -> Result<String, String> {
    if !naming.custom.is_empty() && (naming.programmatic == 0 || naming.group.is_empty()) {
        if naming.programmatic != 0 {
            return crate::scripted_text::stored_text(
                localisation,
                recorded_language,
                &naming.custom,
                "programmatic division name",
            );
        }
        return String::from_utf8(naming.custom.clone())
            .map_err(|_| "custom division name is not UTF-8".into());
    }
    if naming.kind != 0 {
        return Err(format!(
            "name type {} is not an Army naming form",
            naming.kind
        ));
    }
    names()?
        .resolve(&naming.group, naming.order, naming.ordered != 0)
        .map_err(|why| match why {
            content_reader_division_names::division_names::Unresolved::NoGroup
            | content_reader_division_names::division_names::Unresolved::NoFallback => {
                "legacy name allocation inputs are not recorded (no recorded visible override)"
                    .into()
            }
            _ => why.to_string(),
        })
}

pub fn template_name(
    parts: replay_data::rows::TemplateDisplayNameParts<'_>,
    raw_country: i32,
    localise_for_country: impl FnOnce(i32, &[u8]) -> Result<String, String>,
) -> Result<String, String> {
    template_name_with(parts, raw_country, |text| text, localise_for_country)
}

pub fn template_name_with<T>(
    parts: replay_data::rows::TemplateDisplayNameParts<'_>,
    raw_country: i32,
    text: impl FnOnce(String) -> T,
    localise_for_country: impl FnOnce(i32, &[u8]) -> Result<T, String>,
) -> Result<T, String> {
    if !parts.literal.is_empty() {
        if parts.fallback_key.is_some() {
            return Err("template display fallback demand is noncanonical".into());
        }
        return std::str::from_utf8(parts.literal)
            .map(|literal| text(literal.to_owned()))
            .map_err(|_| "template literal name is not UTF-8".into());
    }
    let key = parts
        .fallback_key
        .ok_or("template empty literal lacks fallback-key observation")?;
    if key.is_empty() {
        return Ok(text(String::new()));
    }
    localise_for_country(raw_country, key)
}
#[cfg(test)]
mod tests;
