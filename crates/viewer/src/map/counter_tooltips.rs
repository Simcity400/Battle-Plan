//! Normal land-counter tooltip: 1418C00D0 -> 140C6AC80. Missing reached inputs are refused.

mod ability;
mod attrition;
mod children;
mod railway_gun;
mod supply;

use std::{collections::BTreeMap, sync::Arc};

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
            && held.control == hover.control
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

/// A diagnostics export visits each presented counter control, without needing a pointer.
pub fn count_refusals(tooltips: impl IntoIterator<Item = Tooltip>) -> BTreeMap<String, usize> {
    let mut refused = BTreeMap::new();
    for tooltip in tooltips {
        for (reason, count) in tooltip.refused {
            *refused.entry(reason).or_default() += count;
        }
    }
    refused
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
    nato: bool,
    content: Option<&'a document::Content>,
    dig_in_factor: Result<i64, String>,
    loc: &'a Localisation,
    recorded_language: &'a str,
    icons: &'a dyn TextIcons,
    names: &'a installed_content::division_names::Database,
    units: &'a Result<units::Content, String>,
    supply_grace: Result<i32, String>,
    comms_max_distance: Result<i32, String>,
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

    fn orders_group(&self, handle: u64) -> Result<&'a replay::rows::OrdersGroupRow, String> {
        let rows = self
            .frame
            .complete::<tables::OrdersGroups>()
            .map_err(|e| e.to_string())?;
        rows.binary_search_by_key(&handle, |row| row.handle)
            .map(|index| &rows[index])
            .map_err(|_| format!("OrdersGroups group {handle} is absent"))
    }

    fn group_input(
        &self,
        handle: u64,
    ) -> Result<&'a replay::rows::OrdersGroupCounterInputRow, String> {
        let rows = self
            .frame
            .complete::<tables::OrdersGroupCounterInputs>()
            .map_err(|e| e.to_string())?;
        rows.binary_search_by_key(&handle, |row| row.group)
            .map(|index| &rows[index])
            .map_err(|_| format!("OrdersGroupCounterInputs group {handle} is absent"))
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
        let status = self.status(id)?;
        let name =
            dashboard::unit_names::army_name(status, self.loc, self.recorded_language, || {
                Ok(self.names)
            })?;
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

    fn leader_name(&self, handle: u64) -> Result<String, String> {
        let leaders = self
            .frame
            .complete::<tables::UnitLeaders>()
            .map_err(|e| e.to_string())?;
        let leader = leaders
            .iter()
            .find(|row| u64::from(row.id) << 32 | u64::from(row.reference_type) == handle)
            .ok_or_else(|| format!("UnitLeaders leader {handle} is absent"))?;
        dashboard::scripted_text::stored_text(
            self.loc,
            self.recorded_language,
            &leader.inherited_text_b,
            "unit leader name",
        )
    }

    fn hq(&self, out: &mut Tooltip, candidates: &[u64]) {
        for &id in candidates {
            let result: Result<
                (String, Option<&replay::rows::OrdersGroupCounterInputRow>),
                String,
            > = (|| {
                let division = self.division(id)?;
                if division.orders_group == 0 {
                    return Ok((String::new(), None));
                }
                let group = self.orders_group(division.orders_group)?;
                if group.leader == 0 || group.hq_provider != id {
                    return Ok((String::new(), None));
                }
                let input = self.group_input(group.handle)?;
                let distance = input
                    .hq_distance
                    .require("OrdersGroupCounterInputs.hq_distance")?
                    .ok_or("OrdersGroupCounterInputs.hq_distance was unreadable when recorded")?;
                let maximum = self.comms_max_distance.as_ref().map_err(Clone::clone)?;
                let name = self.leader_name(group.leader)?;
                let key = if distance >= *maximum {
                    "UNIT_LEADER_FRONT_MAX_DISTANCE_TOOLTIP"
                } else if distance < 1 {
                    "UNIT_LEADER_TOOLTIP"
                } else {
                    "UNIT_LEADER_FRONT_DISTANCE_TOOLTIP"
                };
                let text = localise(
                    self.loc,
                    key,
                    &[
                        ("NAME", Arg::Text(&name)),
                        ("DISTANCE", Arg::Int32(distance)),
                    ],
                )?;
                Ok((text, Some(input)))
            })();
            match result {
                Ok((text, Some(input))) if !text.is_empty() => {
                    if !out.title.is_empty() && !out.title.ends_with('\n') {
                        out.title.push('\n');
                    }
                    append(out, self, false, "HQ leader", Ok(text));
                    if input.withdrawing != 0 {
                        let suffix = input.leader_enroute_days
                            .require("OrdersGroupCounterInputs.leader_enroute_days")
                            .and_then(|days| days.ok_or_else(|| "OrdersGroupCounterInputs.leader_enroute_days was unreadable when recorded".to_owned()))
                            .and_then(|days| localise(self.loc, if days == 1 {
                                "UNIT_LEADER_WITHDRAWING_SUFFIX_SINGULAR"
                            } else { "UNIT_LEADER_WITHDRAWING_SUFFIX" }, &[("DAYS", Arg::Int32(days))]));
                        append(out, self, false, "HQ leader withdrawing", suffix);
                    }
                }
                Ok(_) => (),
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

fn next_hours(members: &[(u64, u32, u32, &Timing)]) -> Result<Option<i32>, String> {
    let Some(&(id, province, _, timing)) = members.first() else {
        return Ok(None);
    };
    let kind = timing
        .kind
        .ok_or_else(|| format!("unit {id} stack kind is unavailable"))?;
    let mut minimum = None;
    for &(id, location, path_total, timing) in members {
        if location != province || path_total as i32 <= 0 {
            return Ok(None);
        }
        let other = timing
            .kind
            .ok_or_else(|| format!("unit {id} stack kind is unavailable"))?;
        if other != kind {
            return Ok(None);
        }
        let remaining = timing
            .next
            .ok_or_else(|| format!("unit {id} first-edge ETA is unavailable"))?;
        // 140BE8C70 -> 140BECF90 -> 14047BDE0 ceilings each first edge.
        let hours = engine::cunit::eta_hours(remaining) as i32;
        minimum = Some(minimum.map_or(hours, |previous: i32| previous.min(hours)));
    }
    Ok(minimum.filter(|&hours| hours > 0))
}

fn next_location(loc: &Localisation, hours: Option<i32>) -> Result<String, String> {
    let Some(hours) = hours else {
        return Ok(String::new());
    };
    let text = if hours / 24 == 0 {
        localise(
            loc,
            "UNIT_COUNTER_MOVE_HOURS",
            &[("HOURS", Arg::Int32(hours))],
        )?
    } else {
        localise(
            loc,
            "UNIT_COUNTER_MOVE_DAYS_HOURS",
            &[
                ("DAYS", Arg::Int32(hours / 24)),
                ("HOURS", Arg::Int32(hours % 24)),
            ],
        )?
    };
    Ok(format!("{text}\n\n"))
}

fn supply(
    context: &Context<'_>,
    known: &[(&DivisionRow, &DivisionStatusRow)],
) -> Result<String, String> {
    // 140C6C3E0 uses one getter; 140C6A690 averages known members with fixed5 division.
    let values = known
        .iter()
        .map(|(_, status)| {
            let historical = || {
                "maximum and remaining supply grace (CArmy+600/+610, 140C78780) are unrecorded"
                    .to_owned()
            };
            let maximum = *status
                .supply_grace_maximum
                .require("DivisionStatus.supply_grace_maximum")
                .map_err(|_| historical())?;
            if maximum < 1 {
                return Ok(0);
            }
            let remaining = *status
                .supply_grace_remaining
                .require("DivisionStatus.supply_grace_remaining")
                .map_err(|_| historical())?;
            let grace = context.supply_grace.as_ref().map_err(Clone::clone)?;
            Ok(counter_status::effective_supply(
                status.supply_ratio,
                maximum,
                remaining,
                *grace,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let Some(value) = counter_inputs::member_mean(values) else {
        return Ok(String::new());
    };
    if value >= counter_status::FIXED {
        return Ok(String::new());
    }
    localise(
        context.loc,
        "ARMY_UNIT_SUPPLY",
        &[("VALUE", Arg::Fixed5(value))],
    )
    .map(|text| format!("{text}\n"))
}

pub fn build(
    content: &document::Content,
    frame: &DecodedFrame,
    input: &CounterHover,
    vision: &Vision,
    nato: bool,
) -> Tooltip {
    let context = Context {
        nato,
        content: Some(content),
        dig_in_factor: content.counters.dig_in_factor.clone(),
        loc: &content.names.localisation,
        recorded_language: &content.recorded_language,
        icons: &content.icons.combat,
        names: &content.icons.combat.division_names,
        units: &content.province_hover.units,
        supply_grace: content.counters.supply_grace.clone(),
        comms_max_distance: content.counters.comms_max_distance.clone(),
        frame,
        vision,
        links: frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string()),
    };
    build_context(&context, input, nato)
}

fn build_context(context: &Context<'_>, input: &CounterHover, nato: bool) -> Tooltip {
    if let Some(out) = children::specific(context, input) {
        return out;
    }
    let mut out = Tooltip::default();
    if input.railway_gun {
        out = railway_gun::build(context, input);
        children::append(context, input, &mut out);
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
                context.hq(&mut out, &known);
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
            let before = out.body.len();
            append(&mut out, context, true, "arrival", arrival);
            if !out.body[before..]
                .trim_matches(['\n', '\t', '\r', ' '])
                .is_empty()
            {
                // 140BED3A0 fills the cache; 140BE9902 seeds five updates,
                // and 140BF08E0 in 140BF0300 decrements each entry's lifetime.
                refuse(
                    &mut out,
                    "destination ETA",
                    "uses the stored route (the division panel's value); the native counter's \
                     client-UI path-cache cost filled by hover/selection queries, retained for \
                     five unit updates, is unrecorded",
                );
            }
            let next = (|| {
                if input.members.len() != input.timing.len() {
                    return Err("division member timing is unavailable".into());
                }
                let members = input
                    .members
                    .iter()
                    .zip(input.timing.iter())
                    .map(|(&id, timing)| {
                        let division = context.division(id)?;
                        let movement = context.movement(id)?;
                        Ok((id, division.province, movement.path_total, timing))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                next_location(context.loc, next_hours(&members)?)
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
                    values.as_ref().map_err(Clone::clone).and_then(|values| {
                        statistics(context.loc, input.members.len() == 1, values)
                    }),
                );
                append(
                    &mut out,
                    context,
                    true,
                    "supply",
                    values.and_then(|values| supply(context, &values)),
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
    context.hq(&mut out, &[first]);
    children::append(context, input, &mut out);
    // Header fragments terminate their lines; the painter supplies the newline before its divider.
    if !out.body.is_empty() && out.title.ends_with('\n') {
        out.title.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{test_frame, test_put as put};
    use replay::rows::{
        ClockRow, CountryAppearanceRow, CountryLinkRow, CountryRow, DivisionTemplateIconRow,
        DivisionTemplateRow, LandCombatDivisionRow, LexicalTokenRow, NameOperands, PoliticsRow,
        Recorded, TableRows, TemplateSubunit,
    };

    fn loc() -> Localisation {
        [
            ("CURRENT_ORGANIZATION", "org $CUR|0$/$MAX|0$"),
            ("UNIT_AVERAGE_ORG", "avg org $VALUE|%0$"),
            ("UNIT_AVERAGE_STR", "str $VALUE|%0$"),
            ("ARMY_UNIT_SUPPLY", "supply $VALUE|%0$"),
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
                        // This baseline exercises historical missing-grace refusals;
                        // a recorded zero maximum instead proves native zero supply.
                        supply_grace_maximum: Recorded::NotRecorded,
                        supply_grace_remaining: Recorded::NotRecorded,
                        name: if named {
                            Recorded::Value(Some(b"Guard".to_vec()))
                        } else {
                            Recorded::NotRecorded
                        },
                        naming: if named {
                            Recorded::Value(Some(NameOperands {
                                custom: b"Guard".to_vec(),
                                ..Default::default()
                            }))
                        } else {
                            Recorded::NotRecorded
                        },
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
        put(&mut frame, TableRows::LandCombatDivisions(vec![]));
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
            nato: false,
            content: None,
            dig_in_factor: Err("unused".into()),
            loc: &loc,
            recorded_language: &loc.language,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            supply_grace: Ok(4),
            comms_max_distance: Ok(100),
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
                control: crate::map::counters::CounterControl::Normal,
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
    fn status_cached_name_is_authoritative_even_when_combat_has_a_name() {
        let mut frame = frame(true);
        put(
            &mut frame,
            TableRows::LandCombatDivisions(vec![LandCombatDivisionRow {
                division: 1,
                name_custom: b"Stale combat name".to_vec(),
                ..Default::default()
            }]),
        );
        let vision = Vision::sharing(1, []);
        let named = tooltip(&frame, &[1], false, false, &vision);
        assert!(named.title.contains("Guard"));
        assert!(!named.title.contains("Stale"));
        let mut status = frame.complete::<tables::DivisionStatus>().unwrap().to_vec();
        status[0].naming = Recorded::Value(None);
        put(&mut frame, TableRows::DivisionStatus(status.clone()));
        assert!(
            tooltip(&frame, &[1], false, false, &vision)
                .title
                .contains("Guard")
        );
        // A current cache failure cannot substitute usable generator operands.
        status[0].name = Recorded::Value(None);
        status[0].naming = Recorded::Value(Some(NameOperands {
            custom: b"Generator name".to_vec(),
            ..Default::default()
        }));
        put(&mut frame, TableRows::DivisionStatus(status));
        assert!(
            tooltip(&frame, &[1], false, false, &vision)
                .title
                .is_empty()
        );
        for naming in [
            Recorded::NotRecorded,
            Recorded::Value(None),
            Recorded::Value(Some(NameOperands {
                custom: vec![0xff],
                ..Default::default()
            })),
        ] {
            let mut status = frame.complete::<tables::DivisionStatus>().unwrap().to_vec();
            status[0].name = Recorded::NotRecorded;
            status[0].naming = naming;
            put(&mut frame, TableRows::DivisionStatus(status));
            let out = tooltip(&frame, &[1], false, false, &vision);
            assert!(out.title.is_empty());
            assert_eq!(
                out.refused
                    .iter()
                    .filter(|(reason, _)| reason.starts_with("counter division name:"))
                    .map(|(_, count)| count)
                    .sum::<usize>(),
                1
            );
        }
    }

    #[test]
    fn programmatic_status_names_respect_the_recording_language() {
        let vision = Vision::sharing(1, []);
        let mut frame = frame(true);
        let mut status = frame.complete::<tables::DivisionStatus>().unwrap().to_vec();
        status[0].name = Recorded::Value(Some(b"Stored Guard".to_vec()));
        status[0].naming = Recorded::Value(Some(NameOperands {
            custom: b"Stored Guard".to_vec(),
            programmatic: 1,
            ..Default::default()
        }));
        put(&mut frame, TableRows::DivisionStatus(status.clone()));
        let mut loc = loc();
        loc.language = "french".into();
        let names = installed_content::division_names::Database::default();
        let units = Err("synthetic composition is unavailable".into());
        let build = |frame: &DecodedFrame, recorded_language| {
            let context = Context {
                nato: false,
                content: None,
                dig_in_factor: Err("unused".into()),
                loc: &loc,
                recorded_language,
                icons: &crate::combat::binding::NoIcons,
                names: &names,
                units: &units,
                supply_grace: Ok(4),
                comms_max_distance: Ok(100),
                frame,
                vision: &vision,
                links: frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|e| e.to_string()),
            };
            build_context(
                &context,
                &CounterHover {
                    control: crate::map::counters::CounterControl::Normal,
                    rect: [0.0; 4],
                    members: vec![1].into(),
                    timing: vec![Timing {
                        next: None,
                        kind: None,
                    }]
                    .into(),
                    remaining: None,
                    ours: true,
                    coarse: false,
                    hidden: false,
                    railway_gun: false,
                },
                false,
            )
        };
        let foreign = build(&frame, "english");
        assert!(foreign.title.is_empty());
        assert_eq!(
            foreign
                .refused
                .iter()
                .filter(|(reason, _)| reason.starts_with("counter division name:")
                    && reason.contains("stored in english"))
                .map(|(_, count)| count)
                .sum::<usize>(),
            1,
        );
        assert!(build(&frame, "french").title.contains("Stored Guard"));
        if let Recorded::Value(Some(naming)) = &mut status[0].naming {
            naming.programmatic = 0;
        }
        put(&mut frame, TableRows::DivisionStatus(status));
        assert!(build(&frame, "english").title.contains("Stored Guard"));
    }

    #[test]
    fn supply_line_uses_grace_and_known_stack_average_and_refuses_historical_gaps() {
        let vision = Vision::sharing(1, []);
        let mut frame = frame(true);
        let mut status = frame.complete::<tables::DivisionStatus>().unwrap().to_vec();
        for row in &mut status {
            row.supply_ratio = 20_000;
            row.supply_grace_maximum = Recorded::Value(400_000);
            row.supply_grace_remaining = Recorded::Value(200_000);
        }
        status[1].supply_ratio = 100_000;
        put(&mut frame, TableRows::DivisionStatus(status.clone()));
        let single = tooltip(&frame, &[1], false, false, &vision);
        assert!(single.body.contains("str 50%\nsupply 50%\n"));
        let stack = tooltip(&frame, &[1, 2], false, false, &vision);
        assert!(stack.body.contains("supply 75%\n"));
        assert!(
            !single
                .refused
                .keys()
                .any(|key| key.starts_with("counter supply:"))
        );
        assert!(
            !stack
                .refused
                .keys()
                .any(|key| key.starts_with("counter supply:"))
        );
        status[0].supply_grace_remaining = Recorded::Value(400_000);
        put(&mut frame, TableRows::DivisionStatus(status.clone()));
        for members in [&[1][..], &[1, 2][..]] {
            let full = tooltip(&frame, members, false, false, &vision);
            assert!(!full.body.contains("supply "));
            assert!(
                !full
                    .refused
                    .keys()
                    .any(|key| key.starts_with("counter supply:"))
            );
        }
        status[0].supply_grace_remaining = Recorded::NotRecorded;
        put(&mut frame, TableRows::DivisionStatus(status.clone()));
        let historical = tooltip(&frame, &[1], false, false, &vision);
        let reason = "counter supply: maximum and remaining supply grace \
                      (CArmy+600/+610, 140C78780) are unrecorded";
        assert!(!historical.body.contains("supply "));
        assert_eq!(historical.refused[reason], 1);
        status[0].supply_grace_maximum = Recorded::Value(0);
        put(&mut frame, TableRows::DivisionStatus(status));
        let no_grace = tooltip(&frame, &[1], false, false, &vision);
        assert!(no_grace.body.contains("supply 0%\n"));
        assert!(!no_grace.refused.contains_key(reason));
    }

    #[test]
    fn single_tooltip_orders_native_lines_without_a_date_or_unrecorded_name() {
        let vision = Vision::sharing(1, []);
        let named = tooltip(&frame(true), &[1], false, false, &vision);
        assert_eq!(
            named.title,
            "unit \u{11}HGuard\u{11}! (\u{11}YAlpha\u{11}!)"
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
                .find(|(key, _)| key.starts_with("counter destination ETA:"))
                .map(|(_, &count)| count),
            Some(1)
        );
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
        let reason = "counter division name: DivisionStatus.naming not recorded";
        crate::app::tests::assert_diagnostics(
            || crate::app::tooltip_warning_lines(&unnamed.refused).collect(),
            &[format!("Tooltip: 1 × {reason}")],
        );
        // The dump/capture census builds these hovers without a pointer, counting each once.
        let refused = count_refusals([unnamed.clone(), unnamed]);
        assert_eq!(refused[reason], 2);
    }

    #[test]
    fn full_two_template_stack_sorts_headers_and_coarse_counter_bypasses_detail() {
        let frame = frame(false);
        let vision = Vision::sharing(1, []);
        let detailed = tooltip(&frame, &[1, 2], false, false, &vision);
        assert_eq!(detailed.title, "1 Second Alpha\n1 First Alpha");
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
    fn railway_gun_hovers_name_the_missing_census_before_division_inputs() {
        let frame = test_frame(600);
        let loc = loc();
        let vision = Vision::sharing(1, []);
        let names = installed_content::division_names::Database::default();
        let units = Err("unused".into());
        let context = Context {
            nato: false,
            content: None,
            dig_in_factor: Err("unused".into()),
            loc: &loc,
            recorded_language: &loc.language,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            supply_grace: Ok(4),
            comms_max_distance: Ok(100),
            frame: &frame,
            vision: &vision,
            links: Err("unavailable".into()),
        };
        for coarse in [false, true] {
            let hover = CounterHover {
                control: crate::map::counters::CounterControl::Normal,
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
            assert_eq!(out.refused.len(), 1);
            let (reason, count) = out.refused.iter().next().unwrap();
            assert!(reason.contains("RailwayGunCounterInputs"), "{reason}");
            assert_eq!(*count, 1);
        }
    }

    fn railway_tooltip(
        frame: &DecodedFrame,
        members: &[u64],
        coarse: bool,
        vision: &Vision,
        loc: &Localisation,
    ) -> Tooltip {
        let names = installed_content::division_names::Database::default();
        let units = Err("unused division content".into());
        let context = Context {
            nato: false,
            content: None,
            dig_in_factor: Err("unused".into()),
            loc,
            recorded_language: "english",
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            supply_grace: Err("unused division supply".into()),
            comms_max_distance: Err("unused HQ".into()),
            frame,
            vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string()),
        };
        build_context(
            &context,
            &CounterHover {
                control: crate::map::counters::CounterControl::Normal,
                rect: [0.0; 4],
                members: members.to_vec().into(),
                timing: vec![
                    Timing {
                        next: Some(100_001),
                        kind: Some(17)
                    };
                    members.len()
                ]
                .into(),
                coarse,
                hidden: false,
                railway_gun: true,
                remaining: None,
                ours: true,
            },
            false,
        )
    }

    fn railway_loc() -> Localisation {
        let mut loc: Localisation = [
            ("AAA", "Alpha"),
            ("BBB", "Beta"),
            ("RAILWAY_GUN_HEADER_SINGLE", "gun $NAME$ $COUNTRY$"),
            ("RAILWAY_GUN_HEADER_MULTIPLE", "$COUNT$ guns $COUNTRY$"),
            ("UNKNOWN_RAILWAY_GUN_DESC", "hidden guns\n"),
            ("KNOWN_RAILWAY_GUN_DESC", "known $KNOWN$\n"),
            ("COARSE_COUNTER_UNKNOWN_UNITS", "unknown"),
            ("COARSE_COUNTER_KNOWN_UNITS", "partly known"),
            ("COARSE_COUNTER_RAILWAY_GUNS_DESC", "$RAILWAY_GUNS$ guns"),
            ("UNIT_AVERAGE_STR", "str $VALUE|%0$"),
            ("UNIT_COUNTER_MOVE_HOURS", "next $HOURS$h"),
            ("UNIT_COUNTER_MOVE_DAYS_HOURS", "next $DAYS$d $HOURS$h"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        loc.language = "english".into();
        loc
    }

    fn railway_row(
        gun: u64,
        name: &[u8],
        strength: i64,
    ) -> replay::rows::RailwayGunCounterInputRow {
        replay::rows::RailwayGunCounterInputRow {
            gun,
            roster_owner: 11,
            province: 3,
            name: Recorded::Value(Some(name.to_vec())),
            current_strength: strength,
            maximum_strength: 10,
            ..Default::default()
        }
    }

    #[test]
    fn railway_gun_headers_and_strength_use_their_own_recorded_rows() {
        let mut frame = frame(false);
        *frame.tables[replay::rows::TableId::Divisions.index()] =
            replay::frame::TableView::Unavailable {
                reason: "unused division census".into(),
            };
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![
                railway_row(1, "Gün #1".as_bytes(), 5),
                railway_row(2, b"Second", 10),
            ]),
        );
        let loc = railway_loc();
        let vision = Vision::sharing(1, []);
        let single = railway_tooltip(&frame, &[1], false, &vision, &loc);
        assert_eq!(
            (single.title.as_str(), single.body.as_str()),
            ("gun Gün #1 Alpha", "str 50%")
        );
        assert!(single.refused.is_empty(), "{:?}", single.refused);
        let multiple = railway_tooltip(&frame, &[2, 1], false, &vision, &loc);
        assert_eq!(
            (multiple.title.as_str(), multiple.body.as_str()),
            ("2 guns Alpha", "str 75%")
        );
        assert!(multiple.refused.is_empty(), "{:?}", multiple.refused);
        let coarse = railway_tooltip(&frame, &[1, 2], true, &vision, &loc);
        assert_eq!(
            (coarse.title.as_str(), coarse.body.as_str()),
            ("2 guns\n", "")
        );
        assert!(coarse.refused.is_empty(), "{:?}", coarse.refused);
    }

    #[test]
    fn railway_gun_mixed_intel_keeps_first_member_name_and_only_known_strength() {
        let mut frame = frame(false);
        let mut first = railway_row(1, b"Foreign", 1);
        first.roster_owner = 22;
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![first, railway_row(2, b"Known", 9)]),
        );
        let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
        countries.push(CountryRow {
            id: 22,
            canonical_slot: 2,
            tag: b"BBB".to_vec(),
            ..Default::default()
        });
        put(&mut frame, TableRows::Countries(countries));
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 22,
                    canonical_slot: 2,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 11,
                    ..Default::default()
                },
                CountryAppearanceRow {
                    country: 22,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Politics(vec![
                PoliticsRow {
                    country: 11,
                    ..Default::default()
                },
                PoliticsRow {
                    country: 22,
                    ..Default::default()
                },
            ]),
        );
        let loc = railway_loc();
        let vision = Vision::sharing(1, []);
        let mixed = railway_tooltip(&frame, &[1, 2], false, &vision, &loc);
        assert_eq!(
            (mixed.title.as_str(), mixed.body.as_str()),
            ("hidden guns\nknown 1\ngun Foreign Beta", "str 90%")
        );
        assert!(mixed.refused.is_empty(), "{:?}", mixed.refused);
        let coarse = railway_tooltip(&frame, &[1, 2], true, &vision, &loc);
        assert_eq!(
            (coarse.title.as_str(), coarse.body.as_str()),
            ("partly known\n1 guns\n", "")
        );
        let unknown = railway_tooltip(&frame, &[1], false, &vision, &loc);
        assert_eq!(
            (unknown.title.as_str(), unknown.body.as_str()),
            ("hidden guns\n", "")
        );
        assert!(unknown.refused.is_empty(), "{:?}", unknown.refused);
    }

    #[test]
    fn railway_gun_name_failures_preserve_strength_and_empty_names_cross_languages() {
        let mut frame = frame(false);
        let vision = Vision::sharing(1, []);
        let mut loc = railway_loc();
        for name in [
            Recorded::NotRecorded,
            Recorded::Value(None),
            Recorded::Value(Some(vec![0xff])),
        ] {
            let mut row = railway_row(1, b"unused", 5);
            row.name = name;
            put(&mut frame, TableRows::RailwayGunCounterInputs(vec![row]));
            let text = railway_tooltip(&frame, &[1], false, &vision, &loc);
            assert_eq!((text.title.as_str(), text.body.as_str()), ("", "str 50%"));
            assert_eq!(text.refused.values().copied().sum::<usize>(), 1);
        }
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![railway_row(1, b"Stored", 5)]),
        );
        loc.language = "french".into();
        let foreign = railway_tooltip(&frame, &[1], false, &vision, &loc);
        assert_eq!(foreign.body, "str 50%");
        assert!(foreign.title.is_empty());
        assert!(
            foreign
                .refused
                .keys()
                .any(|key| key.contains("stored in english"))
        );
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![railway_row(1, b"", 5)]),
        );
        let empty = railway_tooltip(&frame, &[1], false, &vision, &loc);
        assert_eq!(empty.title, "gun  Alpha");
        assert!(empty.refused.is_empty(), "{:?}", empty.refused);
    }

    #[test]
    fn railway_gun_movement_precedes_strength_and_checks_only_first_target() {
        let mut frame = frame(false);
        let vision = Vision::sharing(1, []);
        let loc = railway_loc();
        let mut first = railway_row(1, b"First", 5);
        first.ordinary_path = vec![4];
        let mut other = railway_row(2, b"Other", 10);
        other.ordinary_path = vec![4];
        other.target = 4;
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![first.clone(), other.clone()]),
        );
        let moving = railway_tooltip(&frame, &[1, 2], false, &vision, &loc);
        assert_eq!(moving.body, "next 2h\n\nstr 75%");
        assert_eq!(moving.refused.len(), 1);
        assert!(
            moving
                .refused
                .keys()
                .next()
                .unwrap()
                .contains("client-UI path-cache")
        );
        other.ordinary_path.clear();
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![first.clone(), other.clone()]),
        );
        let stationary_member = railway_tooltip(&frame, &[1, 2], false, &vision, &loc);
        assert_eq!(stationary_member.body, "str 75%");
        assert_eq!(stationary_member.refused.len(), 1);
        first.target = 4;
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![first, other]),
        );
        let targeted_first = railway_tooltip(&frame, &[1, 2], false, &vision, &loc);
        assert_eq!(targeted_first.body, "str 75%");
        assert!(
            targeted_first.refused.is_empty(),
            "{:?}",
            targeted_first.refused
        );
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
            nato: false,
            content: None,
            dig_in_factor: Err("unused".into()),
            loc: &loc,
            recorded_language: &loc.language,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            supply_grace: Ok(4),
            comms_max_distance: Ok(100),
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
                .all(|key| !key.contains("selection hints"))
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
            let limitations = |out: &Tooltip| {
                out.refused
                    .iter()
                    .filter(|(key, _)| key.starts_with("counter destination ETA:"))
                    .map(|(_, &count)| count)
                    .sum::<usize>()
            };
            assert_eq!(limitations(&visible), 1);
            assert_eq!(limitations(&hidden), usize::from(ours == 1));
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
    fn destination_eta_limitation_requires_a_displayed_line() {
        let vision = Vision::sharing(1, []);
        let names = installed_content::division_names::Database::default();
        let units = Err("unused".into());
        for (remaining, moving, replacement) in [
            (Some(2_900_000), true, None),
            (Some(0), true, None),
            (None, true, None),
            (Some(2_900_000), false, None),
            (Some(2_900_000), true, Some("@AAA")),
            (Some(2_900_000), true, Some("")),
        ] {
            let mut frame = frame(true);
            if !moving {
                put(
                    &mut frame,
                    TableRows::DivisionMovement(vec![DivisionMovementRow {
                        division: 1,
                        ..Default::default()
                    }]),
                );
            }
            let loc = if let Some(text) = replacement {
                [("UNIT_MOVING_DAYS_HOURS".into(), text.into())]
                    .into_iter()
                    .collect()
            } else {
                loc()
            };
            let context = Context {
                nato: false,
                content: None,
                dig_in_factor: Err("unused".into()),
                loc: &loc,
                recorded_language: &loc.language,
                icons: &crate::combat::binding::NoIcons,
                names: &names,
                units: &units,
                supply_grace: Ok(4),
                comms_max_distance: Ok(100),
                frame: &frame,
                vision: &vision,
                links: frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|e| e.to_string()),
            };
            let hover = CounterHover {
                control: crate::map::counters::CounterControl::Normal,
                rect: [0.0; 4],
                members: vec![1].into(),
                timing: vec![Timing {
                    next: Some(100_001),
                    kind: Some(2),
                }]
                .into(),
                coarse: false,
                hidden: false,
                railway_gun: false,
                remaining,
                ours: true,
            };
            let out = build_context(&context, &hover, false);
            let count: usize = out
                .refused
                .iter()
                .filter(|(key, _)| key.starts_with("counter destination ETA:"))
                .map(|(_, &count)| count)
                .sum();
            assert_eq!(
                count,
                usize::from(remaining == Some(2_900_000) && moving && replacement.is_none())
            );
            assert_eq!(count == 1, out.body.starts_with("arrival"));
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
        let first = 1;
        let mut second = 1;
        let a = Timing {
            next: Some(100_001),
            kind: Some(2),
        };
        let mut b = Timing {
            next: Some(500_000),
            kind: Some(2),
        };
        assert_eq!(
            next_hours(&[(1, 3, first, &a), (2, 3, second, &b)]).unwrap(),
            Some(2)
        );
        b.next = Some(0);
        assert_eq!(
            next_hours(&[(1, 3, first, &a), (2, 3, second, &b)]).unwrap(),
            None
        );
        b.next = Some(500_000);
        b.kind = Some(8);
        assert_eq!(
            next_hours(&[(1, 3, first, &a), (2, 3, second, &b)]).unwrap(),
            None
        );
        b.kind = Some(2);
        second = 0;
        assert_eq!(
            next_hours(&[(1, 3, first, &a), (2, 3, second, &b)]).unwrap(),
            None
        );
        assert_eq!(
            next_hours(&[(1, 3, first, &a), (2, 4, first, &b)]).unwrap(),
            None
        );
    }
}
