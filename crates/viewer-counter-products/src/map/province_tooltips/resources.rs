
use super::{Context, line};
use content_reader_localisation::{localisation::Argument as Arg, localisation::Localisation};
use viewer_runtime::resource_origins::{self, Hour, whole};
use viewer_foundation::tooltip_text::Tooltip;

type Line = (&'static str, Result<String, String>);

const ONE: i64 = dashboard_text::state_resources::ONE;

pub(super) const MODIFIERS_UNRECORDED: &str = "the explanation of local_resources_factor lists each of its non-zero sources, even when they cancel, and the recording holds only the effective factor";
pub(super) const TEMPORARY_UNRECORDED: &str = "the explanation of the state's temporary resources lists each of their sources, and the recording holds only the effective amount";

pub(super) fn lines(
    hour: &Hour<'_>,
    state: u32,
    localisation: &Localisation,
    state_name: &dyn Fn() -> Result<String, String>,
    country_name: &dyn Fn(u32) -> Result<String, String>,
) -> Result<Vec<Line>, String> {
    let loc = |key: &str, args: &[(&str, Arg<'_>)]| {
        localisation
            .localise_with(key.as_bytes(), args)
            .map_err(|error| error.to_string())
    };
    let mut out = Vec::new();
    if hour.controlled(state)?.origin.is_none() {
        return Ok(out);
    }
    for (index, definition) in hour.definitions().iter().enumerate() {
        let amounts = match hour.amounts(state, index) {
            Ok(amounts) => amounts,
            Err(why) => {
                out.push(("resource produced", Err(why)));
                continue;
            }
        };
        if !(hour.listed(state, index) || amounts.delivered > 0)
            || (amounts.nominal == 0 && amounts.delivered == 0)
        {
            continue;
        }
        let origin = amounts.origin;
        let route = hour.route_kind(origin);
        let short = amounts.delivered < amounts.nominal && origin.efficiency < ONE;
        let produced = (|| -> Result<String, String> {
            let penalty = short && !matches!(route.clone()?, 0 | 3);
            let key = if !short {
                "TRADEVIEW_RES_PRODUCED_DETAILS_LINE"
            } else if penalty {
                "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_PENALTY"
            } else {
                "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_LOCKED"
            };
            let name = loc(&definition.name_key(), &[])?;
            let place = state_name()?;
            let holder = match amounts.holder {
                None => String::new(),
                Some(holder) => {
                    let country = country_name(holder)?;
                    loc(
                        "TRADEVIEW_RES_PRODUCED_RIGHTHOLDER",
                        &[("COUNTRY", Arg::Text(&country))],
                    )?
                }
            };
            let mut args = vec![
                ("NUM", Arg::Int32(whole(amounts.delivered))),
                ("TYPE", Arg::Text(&name)),
                ("LOC", Arg::Text(&place)),
                ("ORIGINCONTROLLER", Arg::Text(&holder)),
            ];
            if short {
                args.push(("MAX", Arg::Int32(whole(amounts.nominal))));
            }
            if penalty {
                args.push(("PEN", Arg::Fixed5(origin.efficiency)));
            }
            loc(key, &args)
        })();
        out.push(("resource produced", produced));
        let grants = amounts.operands.grants;
        if grants > 0 {
            out.push((
                "resource buildings",
                loc(
                    "TRADEVIEW_RES_PRODUCED_BY_BUILDING_DETAILS",
                    &[("NUM", Arg::Int32((grants / ONE) as i32))],
                ),
            ));
        }
        let delivery = route.and_then(|kind| match kind {
            0 | 3 => loc("TRADE_ROUTE_DELIVERY_TYPE_LOCKED", &[]),
            2 => loc(
                "TRADE_ROUTE_DELIVERY_TYPE_SEA",
                &[
                    ("VAL", Arg::Int32((amounts.delivered / ONE) as i32)),
                    ("CON", Arg::Int32(origin.convoys_assigned)),
                    ("TOT", Arg::Int32(origin.convoys_required)),
                ],
            ),
            1 => Ok(String::new()),
            kind => Err(format!("route kind {kind} is not one the game draws")),
        });
        out.push((
            "resource route",
            delivery.and_then(|delivery| {
                loc(
                    "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_ROUTE",
                    &[("DEL", Arg::Text(&delivery))],
                )
            }),
        ));
        out.push(("resource modifiers", Err(MODIFIERS_UNRECORDED.to_owned())));
        if amounts.damaged > 0 {
            out.push((
                "infrastructure damage",
                loc(
                    "INFASTRUCTURE_DAMAGE",
                    &[("VALUE", Arg::Fixed5(amounts.operands.damage.wrapping_neg()))],
                ),
            ));
        }
        if hour
            .temporary(state, index)
            .is_some_and(|amount| amount != 0)
        {
            out.push(("temporary resources", Err(TEMPORARY_UNRECORDED.to_owned())));
        }
    }
    Ok(out)
}

impl Context<'_> {
    pub(super) fn resource_lines(&self, out: &mut Tooltip) {
        let content = match &self.content.province_hover.resources {
            Ok(content) => content,
            Err(why) => {
                out.refuse("resources map mode hover", why.clone());
                return;
            }
        };
        let lines = resource_origins::Hour::new(self.frame, content).and_then(|hour| {
            lines(
                &hour,
                self.province.state,
                &self.content.localisation,
                &|| self.state_name(),
                &|raw| self.countries.name(self.slot(raw)?),
            )
        });
        match lines {
            Ok(lines) => {
                for (name, text) in lines {
                    line(out, name, 0, text);
                }
            }
            Err(why) => out.refuse("resources map mode hover", why),
        }
    }
}

#[cfg(test)]
mod tests;
