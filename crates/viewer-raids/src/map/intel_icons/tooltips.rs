//! The entries' hover text. The click footers the replay cannot act on
//! (`OPERATION_ONMAP_CLICK_FOR_DETAILS`, `OPERATION_ONMAP_CLICK_FOR_MORE`) are left out, as the
//! other map hovers leave theirs; a line whose operands are not recorded is refused by name.

use super::build::{
    BOOST_IDEOLOGY, BUILD_NETWORK, CONTROL_TRADE, COUNTER_INTELLIGENCE, DIPLOMATIC_PRESSURE, Hour,
    PROPAGANDA, QUIET_NETWORK, ROOT_OUT, Shared,
};
use super::values::{self, ONE};
use super::*;
use installed_content::localisation::Argument as Arg;
use replay::rows::CharacterOperativeRow;
use viewer_foundation::map::native_text::FLAG;
use viewer_foundation::tooltip_text::localise;

/// Why a secondary line is not drawn: its layout and some of its operands are not established.
const UNESTABLISHED: &str = "its separators and operands are not established for this hover";

/// The More entry's hover: only its click footer, which is left out.
pub(super) fn more() -> Tooltip {
    Tooltip::default()
}

/// The shield's hover: the defense breakdown, the agency defense description, a blank line, then
/// the activity level.
pub(super) fn shield(
    hour: &Hour<'_, '_>,
    defense: Result<i64, String>,
    level: Result<usize, String>,
) -> Tooltip {
    let mut text = Tooltip::default();
    text.refuse("the agency defense breakdown", UNESTABLISHED.into());
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

/// The operative's name and codename, two newlines, then its mission's tooltip.
pub(super) fn operative<'h, 'f>(
    hour: &Hour<'h, 'f>,
    row: &OperativeMissionRow,
    character: Option<&CharacterOperativeRow>,
    shared: &Shared<'h, 'f>,
) -> Tooltip {
    let mut text = Tooltip::default();
    let loc = hour.localisation;
    let header = (|| -> Result<String, String> {
        let character = character.ok_or("the operative has no linked character")?;
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
    // A relocating operative carries its pending mission.
    let (variant, country, state) = if row.assignment == 3 {
        (row.variant, row.target_country, row.target_state)
    } else {
        (row.pending_variant, row.pending_country, row.pending_state)
    };
    let Some(variant) = variant else {
        return text;
    };
    let country = country.and_then(|raw| hour.slot(raw));
    let names = &shared.names;
    let country_name = |suffix: &str| -> Result<String, String> {
        let slot = country.ok_or("the mission's target country has no CountryLinks row")?;
        names.name_with(slot, suffix)
    };
    let flag = || -> Result<String, String> {
        let slot = country.ok_or("the mission's target country has no CountryLinks row")?;
        let tag = hour
            .world
            .tag(slot)
            .ok_or("the mission's target country has no tag")?;
        Ok(format!("{FLAG}{}", String::from_utf8_lossy(tag)))
    };
    let state_name = || -> Result<String, String> {
        let state = state
            .filter(|&state| state > 0)
            .ok_or("the mission has no target state")?;
        hour.state_name(shared, state as u32)
    };
    let key = match variant {
        BUILD_NETWORK => "OPERATIVE_MISSION_BUILD_INTEL_NETWORK_TT",
        QUIET_NETWORK => "OPERATIVE_MISSION_QUIET_INTEL_NETWORK_TT",
        COUNTER_INTELLIGENCE => "OPERATIVE_MISSION_COUNTER_INTELLIGENCE_TT",
        ROOT_OUT => "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE_TT",
        BOOST_IDEOLOGY => "OPERATIVE_MISSION_BOOST_IDEOLOGY_TT",
        CONTROL_TRADE => "OPERATIVE_MISSION_CONTROL_TRADE_TT",
        DIPLOMATIC_PRESSURE => "OPERATIVE_MISSION_DIPLOMATIC_PRESSURE_TT",
        PROPAGANDA => "OPERATIVE_MISSION_PROPAGANDA_TT",
        _ => return text,
    };
    let first = match variant {
        BUILD_NETWORK | QUIET_NETWORK => (|| -> Result<String, String> {
            let (definite, flag, state) = (country_name("_DEF")?, flag()?, state_name()?);
            localise(
                loc,
                key,
                &[
                    ("COUNTRY_DEF", Arg::Text(&definite)),
                    ("FLAG", Arg::Text(&flag)),
                    ("STATE", Arg::Text(&state)),
                ],
            )
        })(),
        COUNTER_INTELLIGENCE => localise(loc, key, &[]),
        ROOT_OUT => (|| -> Result<String, String> {
            let (state, adjective, flag) = (state_name()?, country_name("_ADJ")?, flag()?);
            localise(
                loc,
                key,
                &[
                    ("STATE", Arg::Text(&state)),
                    ("COUNTRY_ADJ", Arg::Text(&adjective)),
                    ("FLAG", Arg::Text(&flag)),
                ],
            )
        })(),
        DIPLOMATIC_PRESSURE => (|| -> Result<String, String> {
            let (definite, flag) = (country_name("_DEF")?, flag()?);
            localise(
                loc,
                key,
                &[
                    ("COUNTRY_DEF", Arg::Text(&definite)),
                    ("FLAG", Arg::Text(&flag)),
                ],
            )
        })(),
        // Their VALUE, WAR_SUPPORT and STABILITY operands are the missions' daily effects,
        // which are not recorded.
        _ => Err("its mission's daily effect operands are not recorded".to_owned()),
    };
    text.line(false, 2, key, first);
    if variant == BUILD_NETWORK {
        let gain = hour.network(row, shared).and_then(|network| {
            localise(
                loc,
                "OPERATIVE_INTEL_NETWORK_GAIN",
                &[("GAIN", Arg::Fixed5(network.map_or(0, |row| row.gain_raw)))],
            )
        });
        text.line(false, 1, "OPERATIVE_INTEL_NETWORK_GAIN", gain);
    }
    for line in secondary(variant) {
        text.refuse(line, UNESTABLISHED.into());
    }
    text
}

/// The breakdown lines after a mission's first, which this hover refuses by name.
fn secondary(variant: u32) -> &'static [&'static str] {
    match variant {
        BUILD_NETWORK => &["MODIFIER_OPERATIVE_STACKING_FACTOR"],
        COUNTER_INTELLIGENCE => &[
            "OPERATIVE_INTELLIGENCE_AGENCY_DEFENSE",
            "MODIFIER_OPERATIVE_STACKING_FACTOR",
        ],
        ROOT_OUT => &[
            "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE_EFFORT",
            "MODIFIER_OPERATIVE_STACKING_FACTOR",
            "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE_STATE_MODIFIER",
        ],
        BOOST_IDEOLOGY | PROPAGANDA => &[
            "the propaganda effort and drift lines",
            "MODIFIER_NATIONAL_COVERAGE_FACTOR",
            "MODIFIER_OPERATIVE_STACKING_FACTOR",
            "MODIFIER_TARGET_INTELLIGENCE_DEFENSE",
        ],
        CONTROL_TRADE => &[
            "MODIFIER_OPERATIVE_STACKING_FACTOR",
            "CONTROL_TRADE_PROGRESS",
        ],
        DIPLOMATIC_PRESSURE => &[
            "the opinion and tension drift lines",
            "MODIFIER_OPERATIVE_STACKING_FACTOR",
        ],
        _ => &[],
    }
}

/// The operative's codename: its stored custom name, as the game stores it. A name drawn from
/// a name group is refused.
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
        return Err(format!(
            "{field} is drawn from a name group, which this hover does not read"
        ));
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
    definition: Option<&installed_content::intelligence::Operation>,
    state: OperationState,
    value: Option<Result<i64, String>>,
    _shared: &Shared<'_, '_>,
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
        (OperationState::Running { .. }, _) => Err(
            "OPERATION_ONMAP_STATUS_IN_PROGRESS: the end date's localised form is not built"
                .to_owned(),
        ),
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
