//! The resources map mode's province hover: what the province's state produces, resource by
//! resource, from the hour's recorded resource origins ([`resource_origins`]).
//!
//! The state's controller's own origin decides whether there is anything to show; each resource
//! reads the origin its rights choose. A resource the state's template lists, or that delivers
//! something, writes, in order:
//!
//! 1. `TRADEVIEW_RES_PRODUCED_DETAILS_LINE`: the delivered amount, the resource, the state and,
//!    for a resource whose rights the controller gave away, `TRADEVIEW_RES_PRODUCED_RIGHTHOLDER`
//!    with the holder. Below the nominal amount at an efficiency below one, the
//!    `_PENALTY` form (with the nominal amount and the efficiency) for a route that delivers,
//!    the `_LOCKED` form (with the nominal amount) for none or another kind.
//! 2. `TRADEVIEW_RES_PRODUCED_BY_BUILDING_DETAILS` with the building grants, when positive.
//! 3. `TRADEVIEW_RES_PRODUCED_DETAILS_LINE_ROUTE` with its delivery: `TRADE_ROUTE_DELIVERY_TYPE_LOCKED`
//!    without a route, `TRADE_ROUTE_DELIVERY_TYPE_SEA` with the convoys of a naval one, nothing
//!    for a land one.
//! 4. `TRADEVIEW_RES_PRODUCED_DETAILS_LINE_MODIFIERS` with the explanation of the origin's
//!    `local_resources_factor`, which walks the factor's non-zero sources even when they cancel
//!    to a zero total.
//! 5. `INFASTRUCTURE_DAMAGE` with the damaged levels' share, when infrastructure is damaged.
//! 6. The explanation of the state's temporary resources, when it holds some.
//!
//! A modifier's explanation lists each of its sources; the recording holds only the effective
//! totals, so the fourth line is refused by name in every resource's block, whatever the
//! recorded factor, and the sixth wherever the state holds temporary resources.

use super::{Context, line};
use installed_content::localisation::{Argument as Arg, Localisation};
use viewer_foundation::{
    resource_origins::{self, Hour, whole},
    tooltip_text::Tooltip,
};

/// A line's name for its refusals, and its text or why it is refused.
type Line = (&'static str, Result<String, String>);

const ONE: i64 = dashboard_text::state_resources::ONE;

/// Why the state-modifier line is refused.
pub(super) const MODIFIERS_UNRECORDED: &str = "the explanation of local_resources_factor lists each of its non-zero sources, even when they cancel, and the recording holds only the effective factor";
/// Why the temporary resources' line is refused.
pub(super) const TEMPORARY_UNRECORDED: &str = "the explanation of the state's temporary resources lists each of their sources, and the recording holds only the effective amount";

/// The resources mode's lines for `state`: nothing without the controller's origin.
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
    /// The resources mode's hover: the state's resources, without the location block.
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
