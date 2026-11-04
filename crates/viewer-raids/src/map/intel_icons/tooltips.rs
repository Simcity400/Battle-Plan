//! The entries' hover text. The click footers the replay cannot act on
//! (`OPERATION_ONMAP_CLICK_FOR_DETAILS`, `OPERATION_ONMAP_CLICK_FOR_MORE`) are left out, as the
//! other map hovers leave theirs; a line whose operands are not recorded is refused by name.

use super::build::{Hour, Shared};
use super::values::{self, ONE};
use super::*;
use installed_content_localisation::localisation::Argument as Arg;
use replay_data::rows::CharacterOperativeRow;
use viewer_foundation::tooltip_text::localise;

/// The More entry's hover: only its click footer, which is left out.
pub(super) fn more() -> Tooltip {
    Tooltip::default()
}

/// The shield's hover: the defense breakdown, the agency defense description, a blank line, then
/// the activity level.
pub(super) fn shield(
    hour: &Hour<'_, '_>,
    slot: u32,
    shared: &Shared<'_, '_>,
    defense: Result<i64, String>,
    level: Result<usize, String>,
) -> Tooltip {
    let mut text = Tooltip::default();
    text.line(
        false,
        0,
        "agency defense breakdown",
        defense_breakdown(hour, slot, shared),
    );
    let loc = hour.localisation;
    text.line(
        false,
        1,
        "AGENCY_DEFENSE_LEVEL_DESC",
        defense.and_then(|defense| {
            localise(
                loc,
                "AGENCY_DEFENSE_LEVEL_DESC",
                &[("DEFENSE", Arg::Fixed5(defense))],
            )
        }),
    );
    text.line(
        false,
        2,
        "COUNTERINTELLIGENCE_ACTIVITY_LEVEL",
        level.and_then(|level| {
            Ok(format!(
                "{}{}",
                localise(loc, "COUNTERINTELLIGENCE_ACTIVITY_LEVEL", &[])?,
                localise(
                    loc,
                    &format!("COUNTERINTELLIGENCE_ACTIVITY_LEVEL_{level}"),
                    &[]
                )?
            ))
        }),
    );
    text
}

fn defense_breakdown(
    hour: &Hour<'_, '_>,
    slot: u32,
    shared: &Shared<'_, '_>,
) -> Result<String, String> {
    use dashboard_text::modifier_text::{self, Breakdown};
    let row = shared
        .tables
        .defense
        .as_ref()
        .map_err(Clone::clone)?
        .iter()
        .find(|row| hour.slot(row.country) == Some(slot))
        .ok_or("country counterintelligence operands unavailable")?;
    if row.strategic_present == 0 {
        return Ok(String::new());
    }
    let defines = hour.defines.counter.as_ref().map_err(Clone::clone)?;
    let rating = values::counter_rating(row, defines)?;
    let header = localise(
        hour.localisation,
        "AGENCY_DEFENSE_LEVEL",
        &[("LEVEL", Arg::Fixed5(rating))],
    )?;
    let mut text = values::coloured('H', &header);
    text.push('\n');
    if defines.base != 0 {
        let base = installed_content_core::fixed5::fixed5_to_string(defines.base, 1);
        text.push_str(&localise(
            hour.localisation,
            "MODIFIER_BASE_VALUE_STR",
            &[("VALUE", Arg::Text(&base))],
        )?);
        text.push('\n');
    }
    let nodes = row
        .defense_tree
        .as_deref()
        .ok_or("country counterintelligence modifier sources were not captured")?;
    if !nodes.is_empty() {
        text.push_str(&modifier_text::breakdown(
            nodes,
            Breakdown {
                depth: -1,
                prefix: "",
                suffix: "",
            },
            |node| {
                let mut text = dashboard_text::scripted_text::stored_text(
                    hour.localisation,
                    hour.language,
                    &node.name,
                    "counterintelligence source",
                )?;
                if !text.is_empty() {
                    text.push_str(": ");
                }
                let definition = hour
                    .frame
                    .complete::<tables::ModifierDefinitions>()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .find(|definition| {
                        definition.index
                            == engine_executable::modifier_keys::INTELLIGENCE_AGENCY_DEFENSE
                    })
                    .ok_or("counterintelligence modifier definition unavailable")?;
                text.push_str(&modifier_text::value_text(
                    definition,
                    node.value,
                    2,
                    false,
                    false,
                    |key, _| {
                        let key = std::str::from_utf8(key)
                            .map_err(|_| "counterintelligence modifier key is not UTF-8")?;
                        localise(hour.localisation, key, &[])
                    },
                )?);
                Ok(text)
            },
        )?);
    }
    let contribution = values::counter_contributions(row, defines)?
        .into_iter()
        .fold(0i64, |sum, (_, value)| sum.wrapping_add(value));
    if contribution > 0 {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&localise(
            hour.localisation,
            "MODIFIER_FROM_OPERATIVE",
            &[],
        )?);
        text.push_str(": ");
        text.push_str(&values::coloured(
            'H',
            &installed_content_core::fixed5::fixed5_to_string(contribution, 1),
        ));
        text.push('\n');
    }
    Ok(text)
}

/// The operative's name and codename, two newlines, then its mission's tooltip.
pub(super) fn operative<'h, 'f>(
    hour: &Hour<'h, 'f>,
    row: &'f OperativeMissionRow,
    character: Result<Option<&CharacterOperativeRow>, String>,
    shared: &Shared<'h, 'f>,
) -> Tooltip {
    let mut text = Tooltip::default();
    let loc = hour.localisation;
    let header = (|| -> Result<String, String> {
        let character = character?.ok_or("the operative has no linked character")?;
        let name = character
            .name
            .require("CharacterOperatives.name")?
            .as_ref()
            .ok_or("the operative's name was not captured")?;
        let name =
            String::from_utf8(name.clone()).map_err(|_| "the operative's name is not UTF-8")?;
        let codename = codename(hour, character)?;
        localise(
            loc,
            "OPERATIVE_NAME_AND_CODENAME",
            &[
                ("NAME", Arg::Text(&name)),
                ("CODENAME", Arg::Text(&codename)),
            ],
        )
    })();
    text.line(false, 0, "OPERATIVE_NAME_AND_CODENAME", header);
    super::missions::append(hour, row, shared, &mut text);
    text
}

pub(super) fn offer(
    hour: &Hour<'_, '_>,
    row: &replay_data::rows::AgencyBufferedOffer,
    definition: Result<&installed_content_core::intelligence::Operation, String>,
) -> Tooltip {
    let mut text = Tooltip::default();
    text.line(
        false,
        0,
        "operation offer name",
        definition
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|definition| {
                let name = localise(hour.localisation, &definition.name, &[])?;
                localise(hour.localisation, "HEADER", &[("KEY", Arg::Text(&name))])
            }),
    );
    text.line(
        true,
        0,
        "operation offer description",
        definition.and_then(|definition| {
            use dashboard_text::scripted_text::{
                EventObject, EventScope, EventScopes, Link, ScriptedText,
            };
            let text = localise(hour.localisation, &definition.description, &[])?;
            if !text.contains('[') {
                return Ok(text);
            }
            let target = hour
                .slot(row.country)
                .ok_or("operation offer target has no CountryLinks row")?;
            let content = hour
                .assets
                .scripted
                .as_ref()
                .ok_or("operation offer scripted content unavailable")?
                .as_ref()
                .as_ref()
                .map_err(Clone::clone)?;
            let mut owner = EventScope::of(EventObject::Country(hour.ours));
            owner.from = Link::Scope(1);
            let scopes =
                EventScopes::new(vec![owner, EventScope::of(EventObject::Country(target))]);
            ScriptedText::new(hour.frame, hour.localisation, content, hour.ours)
                .expand_in(&text, &scopes)
        }),
    );
    text
}

fn codename(hour: &Hour<'_, '_>, character: &CharacterOperativeRow) -> Result<String, String> {
    let field = "operative codename";
    let name = character
        .codename
        .require(field)?
        .as_ref()
        .ok_or_else(|| format!("{field} capture unavailable"))?;
    if name.kind != 2 {
        return Err(format!("{field} has naming kind {}, expected 2", name.kind));
    }
    if name.custom.is_empty() {
        let group =
            std::str::from_utf8(&name.group).map_err(|_| format!("{field} group is not UTF-8"))?;
        return hour
            .raids
            .tooltip
            .unit_names
            .as_ref()
            .map_err(Clone::clone)?
            .name(group, name.order, name.ordered != 0);
    }
    if name.programmatic != 0 {
        return dashboard_text::scripted_text::stored_text(
            hour.localisation,
            hour.language,
            &name.custom,
            field,
        );
    }
    String::from_utf8(name.custom.clone()).map_err(|_| format!("{field} custom name is not UTF-8"))
}

/// The operation's highlighted name, then its status.
pub(super) fn operation(
    hour: &Hour<'_, '_>,
    row: &OperationRow,
    definition: Option<&installed_content_core::intelligence::Operation>,
    state: OperationState,
    value: Option<Result<i64, String>>,
) -> Tooltip {
    let mut text = Tooltip::default();
    let loc = hour.localisation;
    let name = match definition {
        Some(definition) => {
            localise(loc, &definition.name, &[]).map(|name| values::coloured('H', &name))
        }
        None => Err(hour
            .definition(row)
            .err()
            .unwrap_or_else(|| "the operation's definition is unknown".to_owned())),
    };
    text.line(false, 0, "the operation's name", name);
    let status = match (state, value) {
        (OperationState::Finished, _) | (OperationState::Running { remaining: 0, .. }, _) => {
            localise(loc, "OPERATION_ONMAP_STATUS_COMPLETED", &[])
        }
        (OperationState::Running { .. }, _) => {
            let end = row
                .start_hour
                .wrapping_add(row.duration_days.wrapping_mul(24));
            operation_date(end, loc).and_then(|date| {
                localise(
                    loc,
                    "OPERATION_ONMAP_STATUS_IN_PROGRESS",
                    &[("DATE", Arg::Text(&date))],
                )
            })
        }
        (OperationState::Preparing, Some(Ok(ONE))) => {
            localise(loc, "OPERATION_ONMAP_STATUS_READY", &[])
        }
        (OperationState::Preparing, Some(Ok(_))) => {
            localise(loc, "OPERATION_ONMAP_STATUS_NOT_READY", &[])
        }
        (OperationState::Preparing, Some(Err(why))) => {
            Err(format!("its preparation is undecided: {why}"))
        }
        (OperationState::Preparing, None) => Err("its preparation is undecided".to_owned()),
    };
    text.line(false, 1, "the operation's status", status);
    text
}

fn operation_date(hour: i32, localisation: &Localisation) -> Result<String, String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let elapsed = hour.wrapping_sub(engine_native::cdate::HOUR_EPOCH);
    let year = elapsed / (365 * 24);
    let mut day = elapsed / 24 % 365;
    for (index, length) in engine_native::cdate::MONTH_LENGTHS.into_iter().enumerate() {
        if day < length {
            let month = localise(localisation, MONTHS[index], &[])?;
            return Ok(format!("{} {month} {year}", day + 1));
        }
        day -= length;
    }
    Err("operation end date has no calendar month".into())
}
