//! Normal land-counter tooltip: 1418C00D0 -> 140C6AC80. Missing reached inputs are refused.

use std::sync::Arc;

use installed_content::{
    country_names::CountryNames,
    localisation::{Argument as Arg, Localisation},
};
use replay::{
    frame::DecodedFrame,
    rows::{CountryLinkRow, DivisionMovementRow, DivisionRow, DivisionStatusRow, tables},
};

use super::{
    counter_eta, counter_inputs, counter_status, counters::CounterHover, perspective::Vision,
    province_tooltips::units, raids::tooltips::Tooltip,
};
use crate::{
    combat::{
        binding::{TextIcons, displayable},
        text::localise,
    },
    document,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub next: Option<i64>,
    pub kind: Option<u32>,
}

#[derive(Default)]
pub struct Cache {
    held: Option<(CounterHover, bool, Arc<Tooltip>)>,
    source: Option<(Arc<document::Content>, Arc<DecodedFrame>, Arc<Vision>)>,
}

impl Cache {
    pub fn request(
        &mut self,
        content: &Arc<document::Content>,
        frame: &Arc<DecodedFrame>,
        hover: &CounterHover,
        vision: &Arc<Vision>,
        nato: bool,
    ) -> Arc<Tooltip> {
        if let Some((held_content, held_frame, held_vision)) = &self.source
            && Arc::ptr_eq(content, held_content)
            && Arc::ptr_eq(frame, held_frame)
            && Arc::ptr_eq(vision, held_vision)
            && let Some((held, held_nato, text)) = &self.held
            && *held_nato == nato
            && held.members == hover.members
            && held.timing == hover.timing
            && held.coarse == hover.coarse
            && held.remaining == hover.remaining
            && held.ours == hover.ours
            && held.hidden == hover.hidden
            && held.railway_gun == hover.railway_gun
        {
            return Arc::clone(text);
        }
        let text = Arc::new(build(content, frame, hover, vision, nato));
        self.source = Some((Arc::clone(content), Arc::clone(frame), Arc::clone(vision)));
        self.held = Some((hover.clone(), nato, Arc::clone(&text)));
        text
    }
}

fn refuse(out: &mut Tooltip, line: &str, reason: impl Into<String>) {
    *out.refused
        .entry(format!("counter {line}: {}", reason.into()))
        .or_default() += 1;
}

fn append(
    out: &mut Tooltip,
    context: &Context<'_>,
    body: bool,
    line: &str,
    value: Result<String, String>,
) {
    match value.and_then(|text| displayable(context.icons, text)) {
        Ok(text) => {
            if body {
                out.body.push_str(&text);
            } else {
                out.title.push_str(&text);
            }
        }
        Err(reason) => refuse(out, line, reason),
    }
}

struct Context<'a> {
    loc: &'a Localisation,
    icons: &'a dyn TextIcons,
    names: &'a installed_content::division_names::Database,
    units: &'a Result<units::Content, String>,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    links: Result<&'a [CountryLinkRow], String>,
}

impl<'a> Context<'a> {
    fn slot(&self, raw: u32) -> Result<u32, String> {
        let rows = self.links.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&raw, |row| row.id)
            .map(|i| rows[i].canonical_slot)
            .map_err(|_| format!("CountryLinks country {raw} is absent"))
    }

    fn country_name(&self, slot: u32) -> Result<String, String> {
        CountryNames::read_selected(self.frame, self.loc, [slot]).name(slot)
    }

    fn division(&self, id: u64) -> Result<&'a DivisionRow, String> {
        let rows = self
            .frame
            .complete::<tables::Divisions>()
            .map_err(|e| e.to_string())?;
        rows.binary_search_by_key(&id, |row| row.id)
            .map(|i| &rows[i])
            .map_err(|_| format!("Divisions division {id} is absent"))
    }

    fn status(&self, id: u64) -> Result<&'a DivisionStatusRow, String> {
        let rows = self
            .frame
            .complete::<tables::DivisionStatus>()
            .map_err(|e| e.to_string())?;
        rows.binary_search_by_key(&id, |row| row.division)
            .map(|i| &rows[i])
            .map_err(|_| format!("DivisionStatus division {id} is absent"))
    }

    fn movement(&self, id: u64) -> Result<&'a DivisionMovementRow, String> {
        let rows = self
            .frame
            .complete::<tables::DivisionMovement>()
            .map_err(|e| e.to_string())?;
        rows.binary_search_by_key(&id, |row| row.division)
            .map(|i| &rows[i])
            .map_err(|_| format!("DivisionMovement division {id} is absent"))
    }

    fn known(&self, members: &[u64]) -> Result<Vec<u64>, String> {
        let mut known = Vec::new();
        for &id in members {
            let division = self.division(id)?;
            let owner = self.slot(division.roster_owner)?;
            let intel = self
                .vision
                .land_byte_known(id, owner, division.province)
                .ok_or_else(|| format!("division {id} unit intel (1416068C0) is unavailable"))?;
            if intel >= 250 {
                known.push(id);
            }
        }
        Ok(known)
    }

    fn template(&self, handle: u64) -> Result<(u32, String), String> {
        let rows = self
            .frame
            .complete::<tables::DivisionTemplates>()
            .map_err(|e| e.to_string())?;
        let row = rows
            .binary_search_by_key(&handle, |row| row.template)
            .map(|i| &rows[i])
            .map_err(|_| format!("DivisionTemplates template {handle} is absent"))?;
        // 140B88010/140C6BD90 use composition +1D4 (primary +1EC), not the census owner.
        // The country is recorded even when the composition model was unreadable.
        let icons = self
            .frame
            .complete::<tables::DivisionTemplateIcons>()
            .map_err(|e| e.to_string())?;
        let icon = icons
            .binary_search_by_key(&handle, |icon| icon.template)
            .map(|i| &icons[i])
            .map_err(|_| format!("DivisionTemplateIcons template {handle} is absent"))?;
        let raw = u32::try_from(icon.country)
            .map_err(|_| format!("template {handle} country {} is unresolved", icon.country))?;
        let slot = self.slot(raw)?;
        // GetDisplayName@140B91170 -> 140BBA780: literal +20, else key with the raw +1D4 tag.
        // A name_key older recordings lack refuses only the key fallback.
        let name_key: &[u8] = if row.name.is_empty() {
            row.name_key.require("DivisionTemplates.name_key")?
        } else {
            &[]
        };
        let name = if !name_key.is_empty() {
            let key =
                std::str::from_utf8(name_key).map_err(|_| "template name key is not UTF-8")?;
            let countries = self
                .frame
                .complete::<tables::Countries>()
                .map_err(|e| e.to_string())?;
            let country = countries
                .binary_search_by_key(&raw, |country| country.id)
                .map(|i| &countries[i])
                .map_err(|_| format!("Countries country {raw} is absent"))?;
            crate::combat::text::country_localised(
                self.loc,
                std::str::from_utf8(&country.tag)
                    .map_err(|_| "template country's tag is not UTF-8"),
                key,
            )?
        } else {
            std::str::from_utf8(&row.name)
                .map_err(|_| "template name is not UTF-8")?
                .to_owned()
        };
        Ok((slot, name))
    }

    fn single_header(&self, id: u64) -> Result<String, String> {
        let rows = self
            .frame
            .complete::<tables::LandCombatDivisions>()
            .map_err(|e| {
                format!(
                    "division {id} name operands are unrecorded outside LandCombatDivisions: {e}"
                )
            })?;
        let row = rows
            .iter()
            .find(|row| row.division == id)
            .ok_or_else(|| {
                format!("division {id} name operands are unrecorded outside LandCombatDivisions")
            })?;
        // Both callers use GetName@140C72F00's +60 cache; the combat reader renders its operands.
        let name =
            crate::combat::details::division_name(row, self.names).map_err(|e| e.to_string())?;
        let division = self.division(id)?;
        // 140C6CCA0 uses the roster country; 1406F3A00 picks its red/yellow control.
        let owner = self.slot(division.roster_owner)?;
        let country = self.country_name(owner)?;
        let colour = if self.vision.at_war_proven(owner).map_err(|e| e.describe())? {
            'R'
        } else {
            'Y'
        };
        let mut text = localise(
            self.loc,
            "UNIT_COUNTER_TOOLTIP",
            &[("DIVISION", Arg::Text(&name))],
        )?;
        text.push_str(&format!(" (\u{11}{colour}{country}\u{11}!)\n"));
        Ok(text)
    }

    fn template_headers(&self, prepared: &units::PreparedMembers) -> Result<String, String> {
        let mut text = String::new();
        for &(handle, amount) in &prepared.groups {
            let (country, template) = self.template(handle)?;
            let disclosed = self
                .vision
                .shares_vision_proven(country)
                .map_err(|e| e.describe())?;
            let key = template_header_key(country == self.vision.ours(), disclosed, || {
                self.vision.at_war_proven(country).map_err(|e| e.describe())
            })?;
            let country = self.country_name(country)?;
            text.push_str(&localise(
                self.loc,
                key,
                &[
                    ("AMOUNT", Arg::Int32(amount)),
                    ("TEMPLATE", Arg::Text(&template)),
                    ("COUNTRY", Arg::Text(&country)),
                ],
            )?);
            text.push('\n');
        }
        Ok(text)
    }

    fn attrition(&self, known: &[u64]) -> Result<String, String> {
        let mut count = 0_i32;
        let mut maximum = 0_i64;
        for &id in known {
            let status = self.status(id)?;
            if status.attrition_value > status.attrition_threshold {
                count = count.wrapping_add(1);
                maximum = maximum.max(status.attrition_value);
            }
        }
        if maximum <= 0 {
            return Ok(String::new());
        }
        // 140C6A3A0: COUNT integer, MAX fixed5, then one newline.
        localise(
            self.loc,
            "ATTRITION_SOME_UNITS",
            &[("COUNT", Arg::Int32(count)), ("MAX", Arg::Fixed5(maximum))],
        )
        .map(|text| format!("{text}\n"))
    }

    fn hq_gaps(&self, out: &mut Tooltip, candidates: &[u64]) {
        for &id in candidates {
            let result: Result<bool, String> = (|| {
                let division = self.division(id)?;
                if division.orders_group == 0 {
                    return Ok(false);
                }
                let groups = self
                    .frame
                    .complete::<tables::OrdersGroups>()
                    .map_err(|e| e.to_string())?;
                let group = groups
                    .binary_search_by_key(&division.orders_group, |row| row.handle)
                    .map(|i| &groups[i])
                    .map_err(|_| {
                        format!("OrdersGroups group {} is absent", division.orders_group)
                    })?;
                Ok(group.leader != 0 && group.hq_provider == id)
            })();
            match result {
                Ok(true) => refuse(
                    out,
                    "HQ leader",
                    format!(
                        "division {id} leader distance (COrdersGroup+1C4, 1418C00D0) is unrecorded"
                    ),
                ),
                Ok(false) => (),
                Err(reason) => refuse(out, "HQ leader", reason),
            }
        }
    }
}

fn template_header_key(
    ours: bool,
    disclosed: bool,
    hostile: impl FnOnce() -> Result<bool, String>,
) -> Result<&'static str, String> {
    // 140C6BD90 checks disclosure before hostility, except for our own country.
    Ok(if disclosed && !ours {
        "DIVISION_TEMPLATE_HEADER_FRIEND"
    } else if hostile()? {
        "DIVISION_TEMPLATE_HEADER_HOSTILE"
    } else if ours {
        "DIVISION_TEMPLATE_HEADER"
    } else {
        "DIVISION_TEMPLATE_HEADER_NEUTRAL"
    })
}

fn counts(loc: &Localisation, known: i32, unknown: i32, coarse: bool) -> Result<String, String> {
    let mut text = String::new();
    if unknown > 0 {
        if coarse {
            let key = if known == 0 {
                "COARSE_COUNTER_UNKNOWN_UNITS"
            } else {
                "COARSE_COUNTER_KNOWN_UNITS"
            };
            text.push_str(&localise(loc, key, &[])?);
            text.push('\n');
        } else {
            text.push_str(&localise(
                loc,
                "UNKNOWN_ARMY_DESC",
                &[("UNKNOWN", Arg::Int32(unknown))],
            )?);
            text.push('\n');
            if known > 0 {
                text.push_str(&localise(
                    loc,
                    "KNOWN_ARMY_DESC",
                    &[("KNOWN", Arg::Int32(known))],
                )?);
                text.push('\n');
            }
        }
    }
    if coarse && known > 0 {
        text.push_str(&localise(
            loc,
            "COARSE_COUNTER_ARMIES_DESC",
            &[("ARMIES", Arg::Int32(known))],
        )?);
        text.push('\n');
    }
    Ok(text)
}

fn statistics(
    loc: &Localisation,
    single: bool,
    known: &[(&DivisionRow, &DivisionStatusRow)],
) -> Result<String, String> {
    if known.is_empty() {
        return Ok(String::new());
    }
    let mut text = if single {
        let (division, status) = known[0];
        // 140C6C3E0: absolute CUR and native getter MAX, never a percentage.
        localise(
            loc,
            "CURRENT_ORGANIZATION",
            &[
                ("CUR", Arg::Fixed5(division.organisation)),
                (
                    "MAX",
                    Arg::Fixed5(counter_status::effective_maximum(
                        status.maximum_organisation,
                    )),
                ),
            ],
        )?
    } else {
        // 140C6A690: mean of known members' vslot+138 ratios, with native wrapping/truncation.
        let value = counter_inputs::member_mean(known.iter().map(|(division, status)| {
            counter_status::ratio(division.organisation, status.maximum_organisation)
        }))
        .expect("known members are nonempty");
        localise(loc, "UNIT_AVERAGE_ORG", &[("VALUE", Arg::Fixed5(value))])?
    };
    text.push('\n');
    let strength = counter_inputs::member_mean(known.iter().map(|(division, status)| {
        counter_status::modified_strength(
            division.hit_points,
            status.maximum_hit_points,
            status.strength_factor,
        )
    }))
    .expect("known members are nonempty");
    text.push_str(&localise(
        loc,
        "UNIT_AVERAGE_STR",
        &[("VALUE", Arg::Fixed5(strength))],
    )?);
    text.push('\n');
    Ok(text)
}

fn next_hours(
    members: &[(u64, u32, &DivisionMovementRow, &Timing)],
) -> Result<Option<i32>, String> {
    let Some(&(id, province, _, timing)) = members.first() else {
        return Ok(None);
    };
    let kind = timing
        .kind
        .ok_or_else(|| format!("division {id} stack kind is unavailable"))?;
    let mut minimum = None;
    for &(id, location, movement, timing) in members {
        if location != province || movement.path_total as i32 <= 0 {
            return Ok(None);
        }
        let other = timing
            .kind
            .ok_or_else(|| format!("division {id} stack kind is unavailable"))?;
        if other != kind {
            return Ok(None);
        }
        let remaining = timing
            .next
            .ok_or_else(|| format!("division {id} first-edge ETA is unavailable"))?;
        // 140BE8C70 -> 140BECF90 -> 14047BDE0 ceilings each first edge.
        let hours = engine::cunit::eta_hours(remaining) as i32;
        minimum = Some(minimum.map_or(hours, |previous: i32| previous.min(hours)));
    }
    Ok(minimum.filter(|&hours| hours > 0))
}

pub fn build(
    content: &document::Content,
    frame: &DecodedFrame,
    input: &CounterHover,
    vision: &Vision,
    nato: bool,
) -> Tooltip {
    let context = Context {
        loc: &content.names.localisation,
        icons: &content.icons.combat,
        names: &content.icons.combat.division_names,
        units: &content.province_hover.units,
        frame,
        vision,
        links: frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string()),
    };
    build_context(&context, input, nato)
}

fn build_context(context: &Context<'_>, input: &CounterHover, nato: bool) -> Tooltip {
    let mut out = Tooltip::default();
    if input.railway_gun {
        refuse(
            &mut out,
            "railway gun",
            "railway-gun branch of 1418C00D0 is not ported",
        );
        return out;
    }
    if input.members.is_empty() {
        return out;
    }
    if let Err(reason) = &context.links {
        refuse(&mut out, "inputs", reason.clone());
        return out;
    }
    let frame = context.frame;
    let vision = context.vision;
    let canonical = |raw| context.slot(raw).ok();
    if input.coarse {
        match context.known(&input.members) {
            Ok(known) => {
                let unknown = input.members.len().wrapping_sub(known.len()) as i32;
                append(
                    &mut out,
                    context,
                    false,
                    "intel header",
                    counts(context.loc, known.len() as i32, unknown, true),
                );
                context.hq_gaps(&mut out, &known);
            }
            Err(reason) => refuse(&mut out, "intel header/stat eligibility", reason),
        }
        return out;
    }
    let first = input.members[0];
    // 1418C1983/140BE8C70 test the first member's route and +1E8 target before both lines.
    match context.movement(first) {
        Ok(movement) if movement.path_total as i32 > 0 && movement.target == 0 => {
            let arrival = (|| {
                if !input.ours && input.hidden {
                    return Ok(String::new());
                }
                let remaining = input
                    .remaining
                    .ok_or_else(|| format!("division {first} ETA is unavailable"))?;
                counter_eta::arrival(remaining).map_or_else(
                    || Ok(String::new()),
                    |arrival| {
                        counter_eta::sentence(context.loc, &arrival, input.ours)
                            .map(|text| format!("{text}\n"))
                    },
                )
            })();
            append(&mut out, context, true, "arrival", arrival);
            let next = (|| {
                let members = input
                    .members
                    .iter()
                    .zip(input.timing.iter())
                    .map(|(&id, timing)| {
                        let division = context.division(id)?;
                        let movement = context.movement(id)?;
                        Ok((id, division.province, movement, timing))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let Some(hours) = next_hours(&members)? else {
                    return Ok(String::new());
                };
                let text = if hours / 24 == 0 {
                    localise(
                        context.loc,
                        "UNIT_COUNTER_MOVE_HOURS",
                        &[("HOURS", Arg::Int32(hours))],
                    )?
                } else {
                    localise(
                        context.loc,
                        "UNIT_COUNTER_MOVE_DAYS_HOURS",
                        &[
                            ("DAYS", Arg::Int32(hours / 24)),
                            ("HOURS", Arg::Int32(hours % 24)),
                        ],
                    )?
                };
                Ok(format!("{text}\n\n"))
            })();
            append(&mut out, context, true, "next location", next);
        }
        Ok(_) => (),
        Err(reason) => refuse(&mut out, "movement", reason),
    }
    match context.known(&input.members) {
        Ok(known) => {
            let unknown = input.members.len().wrapping_sub(known.len()) as i32;
            let prepared = units::prepare_members(frame, &input.members, &canonical, vision);
            append(
                &mut out,
                context,
                false,
                "intel header",
                counts(context.loc, known.len() as i32, unknown, false),
            );
            let disclosed = prepared.as_ref().ok().map(|prepared| prepared.disclosed);
            if input.members.len() == 1 {
                if !known.is_empty() {
                    append(
                        &mut out,
                        context,
                        false,
                        "division name",
                        context.single_header(first),
                    );
                }
                if disclosed == Some(true) {
                    let real = (|| {
                        let division = context.division(first)?;
                        if division.reveal_skip == 0 {
                            return Ok(String::new());
                        }
                        let icons = frame
                            .complete::<tables::ArmyIconInputs>()
                            .map_err(|e| e.to_string())?;
                        let icon = icons
                            .binary_search_by_key(&first, |row| row.division)
                            .map(|i| &icons[i])
                            .map_err(|_| format!("ArmyIconInputs division {first} is absent"))?;
                        if icon.alternate_template == 0 {
                            return Ok(String::new());
                        }
                        let (_, name) = context.template(icon.alternate_template)?;
                        localise(
                            context.loc,
                            "FAKE_ARMY_REAL_INFO",
                            &[("REAL_TEMPLATE", Arg::Text(&name))],
                        )
                    })();
                    append(&mut out, context, false, "real template", real);
                }
            } else {
                append(
                    &mut out,
                    context,
                    false,
                    "template headers",
                    prepared
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|prepared| context.template_headers(prepared)),
                );
                append(
                    &mut out,
                    context,
                    false,
                    "attrition",
                    context.attrition(&known),
                );
            }
            // Single undisclosed (140C6C3E0) and zero-known (140C6A690) never reach supply.
            if !known.is_empty() && (input.members.len() != 1 || disclosed == Some(true)) {
                let values = known
                    .iter()
                    .map(|&id| Ok((context.division(id)?, context.status(id)?)))
                    .collect::<Result<Vec<_>, String>>();
                append(
                    &mut out,
                    context,
                    true,
                    "organisation/strength",
                    values.and_then(|values| {
                        statistics(context.loc, input.members.len() == 1, &values)
                    }),
                );
                // 140C78780 first returns zero when max grace<=0;
                // no received ratio decides this missing gate.
                refuse(
                    &mut out,
                    "supply",
                    "maximum and remaining supply grace (CArmy+600/+610, 140C78780) are unrecorded",
                );
            }
            let composition = prepared.and_then(|prepared| {
                let units = context.units.as_ref().map_err(Clone::clone)?;
                units::summary_prepared(
                    units,
                    frame,
                    &prepared,
                    &canonical,
                    vision,
                    context.loc,
                    nato,
                )
            });
            append(&mut out, context, true, "composition", composition);
        }
        Err(reason) => refuse(&mut out, "intel header/stat eligibility", reason),
    }
    context.hq_gaps(&mut out, &[first]);
    if let Ok(first) = context.division(first)
        && first.province != 0
        && input.members.iter().all(|&id| {
            context
                .division(id)
                .is_ok_and(|division| division.province == first.province)
        })
    {
        refuse(
            &mut out,
            "selection hints",
            "native selected-unit history and click actions (141E6AB00 -> 140C72F60) are unavailable in replay",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{test_frame, test_put as put};
    use replay::rows::{
        ClockRow, CountryAppearanceRow, CountryLinkRow, CountryRow, DivisionTemplateIconRow,
        DivisionTemplateRow, LandCombatDivisionRow, LexicalTokenRow, PoliticsRow, TableRows,
        TemplateSubunit,
    };
    use std::collections::BTreeMap;

    fn loc() -> Localisation {
        [
            ("CURRENT_ORGANIZATION", "org $CUR|0$/$MAX|0$"),
            ("UNIT_AVERAGE_ORG", "avg org $VALUE|%0$"),
            ("UNIT_AVERAGE_STR", "str $VALUE|%0$"),
            ("UNKNOWN_ARMY_DESC", "hidden $UNKNOWN$"),
            ("KNOWN_ARMY_DESC", "known $KNOWN$"),
            ("COARSE_COUNTER_UNKNOWN_UNITS", "unknown"),
            ("COARSE_COUNTER_KNOWN_UNITS", "partly known"),
            ("COARSE_COUNTER_ARMIES_DESC", "$ARMIES$ divisions"),
            ("UNIT_COUNTER_TOOLTIP", "unit $DIVISION|H$"),
            ("DIVISION_TEMPLATE_HEADER", "$AMOUNT$ $TEMPLATE$ $COUNTRY$"),
            ("UNIT_MOVING_DAYS_HOURS", "arrival $DAYS$d $HOURS$h"),
            (
                "UNIT_MOVING_DAYS_HOURS_NONPLAYER",
                "other arrival $DAYS$d $HOURS$h",
            ),
            ("UNIT_COUNTER_MOVE_HOURS", "next $HOURS$h"),
            ("SUB_UNIT_ACCURAT_VALUE", "$VALUE$"),
            ("AAA", "Alpha"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    fn frame(named: bool) -> DecodedFrame {
        let mut frame = test_frame(600);
        put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 11,
                canonical_slot: 1,
                tag: b"AAA".to_vec(),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 11,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Politics(vec![PoliticsRow {
                country: 11,
                ..Default::default()
            }]),
        );
        put(&mut frame, TableRows::PoliticalParties(vec![]));
        put(&mut frame, TableRows::Autonomy(vec![]));
        put(
            &mut frame,
            TableRows::Divisions(
                [1, 2]
                    .into_iter()
                    .map(|id| DivisionRow {
                        id,
                        roster_owner: 11,
                        province: 3,
                        template: ((if id == 1 { 1 } else { 9 }) << 32) | 7,
                        organisation: 1_000_000,
                        hit_points: 1_000_000,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::DivisionStatus(
                [1, 2]
                    .into_iter()
                    .map(|division| DivisionStatusRow {
                        division,
                        maximum_organisation: 2_000_000,
                        maximum_hit_points: 1_000_000,
                        strength_factor: 50_000,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::DivisionMovement(
                [1, 2]
                    .into_iter()
                    .map(|division| DivisionMovementRow {
                        division,
                        path_total: 1,
                        path: vec![4],
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::LandCombatDivisions(if named {
                vec![LandCombatDivisionRow {
                    division: 1,
                    name_custom: b"Guard".to_vec(),
                    ..Default::default()
                }]
            } else {
                vec![]
            }),
        );
        put(
            &mut frame,
            TableRows::DivisionTemplates(
                [(1, "First"), (9, "Second")]
                    .into_iter()
                    .map(|(id, name)| DivisionTemplateRow {
                        template: id << 32 | 7,
                        owner: 11,
                        name: name.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::DivisionTemplateIcons(
                [1, 9]
                    .into_iter()
                    .map(|id| DivisionTemplateIconRow {
                        template: id << 32 | 7,
                        country: 11,
                        subunits: vec![TemplateSubunit {
                            token: 1,
                            local: 0,
                            count: 2,
                        }],
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"Foot".to_vec(),
            }]),
        );
        put(&mut frame, TableRows::Clock(vec![ClockRow::default()]));
        frame
    }

    fn tooltip(
        frame: &DecodedFrame,
        members: &[u64],
        coarse: bool,
        hidden: bool,
        vision: &Vision,
    ) -> Tooltip {
        let loc = loc();
        let defines = installed_content::defines::Defines::new().unwrap();
        defines
            .load("synthetic.lua", b"NDefines={NIntel={COUNTRY_LEVEL_INTEL_MAXIMUMS={1,1,1,1},ARMY_TEMPLATE_UNIT_COUNT_INTEL_MIN=0.25,ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX=0.75,ARMY_TEMPLATE_UNIT_COUNT_INTEL_RANGE_AT_LOWEST_INTEL=1.5}}")
            .unwrap();
        let mut subunits = installed_content::unit_leader::SubUnits::default();
        subunits.read(b"sub_units={Foot={type=infantry}}", "synthetic.txt");
        let units = units::Content::load(&defines, Ok(&subunits));
        let names = installed_content::division_names::Database::default();
        let context = Context {
            loc: &loc,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            frame,
            vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string()),
        };
        let timing = members
            .iter()
            .map(|_| Timing {
                next: Some(100_001),
                kind: Some(2),
            })
            .collect::<Vec<_>>();
        build_context(
            &context,
            &CounterHover {
                rect: [0.0; 4],
                members: members.to_vec().into(),
                coarse,
                hidden,
                railway_gun: false,
                remaining: Some(2_900_000),
                ours: vision.ours() == 1,
                timing: timing.into(),
            },
            false,
        )
    }

    #[test]
    fn single_tooltip_orders_native_lines_without_a_date_or_unrecorded_name() {
        let vision = Vision::sharing(1, []);
        let named = tooltip(&frame(true), &[1], false, false, &vision);
        assert_eq!(
            named.title,
            "unit \u{11}HGuard\u{11}! (\u{11}YAlpha\u{11}!)\n"
        );
        assert_eq!(
            named.body,
            "arrival 1d 5h\nnext 2h\n\norg 10/20\nstr 50%\n2\u{13}unit_Foot_icon_small|1 "
        );
        assert_eq!(named.refused.len(), 2);
        assert_eq!(named.refused.values().copied().sum::<usize>(), 2);
        assert_eq!(
            named
                .refused
                .iter()
                .find(|(key, _)| key.starts_with("counter supply:"))
                .map(|(_, &count)| count),
            Some(1)
        );
        let unnamed = tooltip(&frame(false), &[1], false, false, &vision);
        assert_eq!(unnamed.title, "");
        assert_eq!(unnamed.body, named.body);
        assert_eq!(
            unnamed
                .refused
                .iter()
                .find(|(key, _)| key.contains("division name"))
                .map(|(_, &count)| count),
            Some(1)
        );
    }

    #[test]
    fn full_two_template_stack_sorts_headers_and_coarse_counter_bypasses_detail() {
        let frame = frame(false);
        let vision = Vision::sharing(1, []);
        let detailed = tooltip(&frame, &[1, 2], false, false, &vision);
        assert_eq!(detailed.title, "1 Second Alpha\n1 First Alpha\n");
        assert_eq!(
            detailed.body,
            "arrival 1d 5h\nnext 2h\n\navg org 50%\nstr 50%\n4\u{13}unit_Foot_icon_small|1 "
        );
        assert!(
            !detailed
                .refused
                .keys()
                .any(|key| key.contains("division name"))
        );
        let coarse = tooltip(&frame, &[1, 2], true, false, &vision);
        assert_eq!(coarse.title, "2 divisions\n");
        assert_eq!(coarse.body, "");
        assert!(coarse.refused.is_empty());
    }

    #[test]
    fn invalid_orders_child_does_not_change_normal_or_coarse_hovers() {
        let mut frame = frame(true);
        let vision = Vision::sharing(1, []);
        let expected = [
            tooltip(&frame, &[1], false, false, &vision),
            tooltip(&frame, &[1], true, false, &vision),
        ];
        let mut statuses = frame.complete::<tables::DivisionStatus>().unwrap().to_vec();
        statuses[0].invalid_orders = 1;
        put(&mut frame, TableRows::DivisionStatus(statuses));
        for (coarse, expected) in [false, true].into_iter().zip(expected) {
            assert_eq!(tooltip(&frame, &[1], coarse, false, &vision), expected);
        }
    }

    #[test]
    fn railway_gun_hovers_refuse_their_branch_once_before_division_inputs() {
        let frame = test_frame(600);
        let loc = loc();
        let vision = Vision::sharing(1, []);
        let names = installed_content::division_names::Database::default();
        let units = Err("unused".into());
        let context = Context {
            loc: &loc,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            frame: &frame,
            vision: &vision,
            links: Err("unavailable".into()),
        };
        for coarse in [false, true] {
            let hover = CounterHover {
                rect: [0.0; 4],
                members: vec![1].into(),
                timing: vec![].into(),
                coarse,
                hidden: false,
                railway_gun: true,
                remaining: None,
                ours: true,
            };
            let out = build_context(&context, &hover, false);
            assert_eq!(out.title, "");
            assert_eq!(out.body, "");
            assert_eq!(
                out.refused,
                BTreeMap::from([(
                    "counter railway gun: railway-gun branch of 1418C00D0 is not ported".into(),
                    1
                )])
            );
        }
    }

    #[test]
    fn template_country_uses_recorded_composition_instead_of_census_owner_or_alias() {
        let mut frame = frame(false);
        let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
        countries.insert(
            0,
            CountryRow {
                id: 10,
                canonical_slot: 1,
                tag: b"BBB".to_vec(),
                ..Default::default()
            },
        );
        countries.push(CountryRow {
            id: 12,
            canonical_slot: 2,
            tag: b"CCC".to_vec(),
            ..Default::default()
        });
        put(&mut frame, TableRows::Countries(countries));
        put(
            &mut frame,
            TableRows::CountryLinks(
                [(10, 1), (11, 1), (12, 2)]
                    .into_iter()
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ),
        );
        let mut templates = frame
            .complete::<tables::DivisionTemplates>()
            .unwrap()
            .to_vec();
        templates[0].name.clear();
        templates[0].name_key = b"FORM".to_vec().into();
        templates[0].owner = 10;
        put(&mut frame, TableRows::DivisionTemplates(templates));
        let mut icons = frame
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap()
            .to_vec();
        icons[0].model_unreadable = 1;
        icons[1].country = 12;
        put(&mut frame, TableRows::DivisionTemplateIcons(icons));
        let loc = [
            ("AAA_FORM".into(), "Right".into()),
            ("BBB_FORM".into(), "Wrong".into()),
        ]
        .into_iter()
        .collect();
        let vision = Vision::sharing(1, []);
        let names = installed_content::division_names::Database::default();
        let units = Err("unused".into());
        let context = Context {
            loc: &loc,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            frame: &frame,
            vision: &vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string()),
        };
        assert_eq!(context.template(1 << 32 | 7).unwrap(), (1, "Right".into()));
        assert_eq!(context.template(9 << 32 | 7).unwrap(), (2, "Second".into()));
    }

    #[test]
    fn missing_unit_intel_preserves_movement_with_one_eligibility_gap() {
        let frame = frame(true);
        let vision = Vision::sharing(2, []).with_truncated_unit_intel([]);
        let detailed = tooltip(&frame, &[1], false, false, &vision);
        assert_eq!(detailed.title, "");
        assert_eq!(detailed.body, "other arrival 1d 5h\nnext 2h\n\n");
        assert_eq!(
            detailed
                .refused
                .iter()
                .find(|(key, _)| key.contains("intel header/stat eligibility"))
                .map(|(_, &count)| count),
            Some(1)
        );
        assert_eq!(detailed.refused.len(), 2);
        assert!(
            detailed
                .refused
                .keys()
                .any(|key| key.contains("selection hints"))
        );
        let coarse = tooltip(&frame, &[1], true, false, &vision);
        assert_eq!(coarse.title, "");
        assert_eq!(coarse.body, "");
        assert_eq!(coarse.refused.len(), 1);
        assert_eq!(coarse.refused.values().copied().sum::<usize>(), 1);
    }

    #[test]
    fn single_absolute_org_and_multi_average_ratios_keep_native_order() {
        let first = DivisionRow {
            organisation: 1_000_000,
            hit_points: 1_000_000,
            ..Default::default()
        };
        let first_status = DivisionStatusRow {
            maximum_organisation: 2_000_000,
            maximum_hit_points: 1_000_000,
            strength_factor: 50_000,
            ..Default::default()
        };
        let second = DivisionRow {
            organisation: 1_000_000,
            hit_points: 1_000_000,
            ..Default::default()
        };
        let second_status = DivisionStatusRow {
            maximum_organisation: 4_000_000,
            maximum_hit_points: 1_000_000,
            strength_factor: 100_000,
            ..Default::default()
        };
        assert_eq!(
            statistics(&loc(), true, &[(&first, &first_status)]).unwrap(),
            "org 10/20\nstr 50%\n"
        );
        assert_eq!(
            statistics(
                &loc(),
                false,
                &[(&first, &first_status), (&second, &second_status)]
            )
            .unwrap(),
            "avg org 37%\nstr 75%\n"
        );
        assert_eq!(statistics(&loc(), false, &[]).unwrap(), "");
    }

    #[test]
    fn only_hidden_foreign_stacks_suppress_the_arrival_line() {
        let frame = frame(true);
        for ours in [1, 2] {
            let vision = Vision::sharing(ours, [1]);
            let visible = tooltip(&frame, &[1], false, false, &vision);
            let hidden = tooltip(&frame, &[1], false, true, &vision);
            if ours == 1 {
                assert_eq!(hidden.body, visible.body);
            } else {
                assert!(visible.body.starts_with("other arrival 1d 5h\n"));
                assert_eq!(
                    hidden.body,
                    "next 2h\n\norg 10/20\nstr 50%\n2\u{13}unit_Foot_icon_small|1 "
                );
                assert!(
                    !hidden
                        .refused
                        .keys()
                        .any(|key| key.starts_with("counter arrival:"))
                );
            }
        }
    }

    #[test]
    fn mixed_and_coarse_headers_do_not_invent_unknown_counts_or_detailed_statistics() {
        assert_eq!(counts(&loc(), 2, 1, false).unwrap(), "hidden 1\nknown 2\n");
        assert_eq!(counts(&loc(), 2, 0, false).unwrap(), "");
        assert_eq!(counts(&loc(), 0, 5, true).unwrap(), "unknown\n");
        assert_eq!(
            counts(&loc(), 2, 1, true).unwrap(),
            "partly known\n2 divisions\n"
        );
        assert_eq!(counts(&loc(), 2, 0, true).unwrap(), "2 divisions\n");
        assert_eq!(
            template_header_key(false, true, || {
                panic!("a friend heading never checks hostility")
            })
            .unwrap(),
            "DIVISION_TEMPLATE_HEADER_FRIEND"
        );
        assert_eq!(
            template_header_key(true, true, || Ok(false)).unwrap(),
            "DIVISION_TEMPLATE_HEADER"
        );
        assert_eq!(
            template_header_key(false, false, || Ok(true)).unwrap(),
            "DIVISION_TEMPLATE_HEADER_HOSTILE"
        );
        assert_eq!(
            template_header_key(false, false, || Ok(false)).unwrap(),
            "DIVISION_TEMPLATE_HEADER_NEUTRAL"
        );
    }

    #[test]
    fn next_location_minimum_requires_members_location_kind_and_routes_to_agree() {
        let first = DivisionMovementRow {
            path_total: 1,
            path: vec![2],
            ..Default::default()
        };
        let mut second = DivisionMovementRow {
            path_total: 1,
            path: vec![9],
            target: 9,
            ..Default::default()
        };
        let a = Timing {
            next: Some(100_001),
            kind: Some(2),
        };
        let mut b = Timing {
            next: Some(500_000),
            kind: Some(2),
        };
        assert_eq!(
            next_hours(&[(1, 3, &first, &a), (2, 3, &second, &b)]).unwrap(),
            Some(2)
        );
        b.next = Some(0);
        assert_eq!(
            next_hours(&[(1, 3, &first, &a), (2, 3, &second, &b)]).unwrap(),
            None
        );
        b.next = Some(500_000);
        b.kind = Some(8);
        assert_eq!(
            next_hours(&[(1, 3, &first, &a), (2, 3, &second, &b)]).unwrap(),
            None
        );
        b.kind = Some(2);
        second.path_total = 0;
        assert_eq!(
            next_hours(&[(1, 3, &first, &a), (2, 3, &second, &b)]).unwrap(),
            None
        );
        assert_eq!(
            next_hours(&[(1, 3, &first, &a), (2, 4, &first, &b)]).unwrap(),
            None
        );
    }
}
