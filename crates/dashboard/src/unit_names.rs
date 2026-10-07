//! Names returned by the native Army getter, shared by scripted UNIT names,
//! political counters, combat entries and recorded-unit products.

use installed_content::{division_names::Database, localisation::Localisation};
use replay::rows::{DivisionStatusRow, NameOperands, Recorded};

/// CArmy::GetName140C72F00 returns the retained CNameGroupMember+0x60 string;
/// it does not run the generator. A recorded cache is authoritative, including
/// empty text, and a current capture failure never substitutes generator output.
/// Only historical absence uses the recorded generator operands and content.
///
/// The database is acquired lazily, so readable cached text needs neither a
/// name-group database nor successful naming operands. A nonprogrammatic+0x88
/// literal whose bytes equal the cache proves literal provenance (1409BBD40 /
/// 1409B4C40); all other cached text keeps the recording-language guard.
pub fn army_name<'a>(
    status: &DivisionStatusRow,
    localisation: &Localisation,
    recorded_language: &str,
    legacy_names: impl FnOnce() -> Result<&'a Database, String>,
) -> Result<String, String> {
    match &status.name {
        Recorded::Value(Some(bytes)) => {
            let literal = matches!(
                &status.naming,
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
            "DivisionStatus.name unavailable for UNIT {:#x}: guarded read or name arena refusal (CArmy::GetName140C72F00)",
            status.division
        )),
        Recorded::NotRecorded => {
            let naming = status
                .naming
                .require("DivisionStatus.naming")?
                .as_ref()
                .ok_or_else(|| format!(
                    "DivisionStatus.naming unavailable for UNIT {:#x}: guarded read or arena shortage (1409B5640)",
                    status.division
                ))?;
            legacy_army_name(naming, localisation, recorded_language, legacy_names)
        }
    }
}

/// The historical generator-operand path. It never assigns another ordinal or
/// runs the legacy allocator when authored names are exhausted (1409B4CE0).
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
            "name type {} is not an Army naming form (1409B7290)",
            naming.kind
        ));
    }
    names()?
        .resolve(&naming.group, naming.order, naming.ordered != 0)
        .map_err(|why| match why {
            installed_content::division_names::Unresolved::NoGroup
            | installed_content::division_names::Unresolved::NoFallback => {
                "legacy name allocation inputs are not recorded (1409B4CE0 -> 140AE3750; no recorded visible override)".into()
            }
            _ => why.to_string(),
        })
}

#[cfg(test)]
mod tests;
