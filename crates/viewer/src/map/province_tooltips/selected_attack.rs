//! Selected attack tooltip. Selection, hovered target and the single retained
//! route stay viewer-local.
mod arithmetic;
#[cfg(test)]
mod tests;

use super::{
    Arg, Context, Tooltip, localise,
    selection::{line, refuse},
};
use crate::map::division_eta::CounterQueries;
use installed_content::unit_path::{Fact, SelectedNeighbour, StraitFleet};
use replay::{
    frame::DecodedFrame,
    rows::{self, TableSchema, tables},
};
use std::collections::BTreeSet;

fn row<'a, T: TableSchema>(
    frame: &'a DecodedFrame,
    key: u64,
    key_of: impl Fn(&T::Row) -> u64,
    name: &str,
) -> Fact<&'a T::Row> {
    let rows = frame.complete::<T>().map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&key, key_of)
        .map(|i| &rows[i])
        .map_err(|_| format!("{name} member {key} is absent"))
}

fn field<T: Copy>(value: Option<T>, name: &str) -> Fact<T> {
    value.ok_or_else(|| format!("{name} was refused when recorded"))
}

fn fixed_scalar(
    frame: &DecodedFrame,
    value: Option<i64>,
    family: replay::fixed_layout::Family,
    name: &str,
) -> Fact<i64> {
    replay::fixed_layout::validate_recording(frame, family)?;
    field(value, name)
}

fn fresh(hour: i32, sample: i32, name: &str) -> Fact<()> {
    if sample == hour {
        Ok(())
    } else {
        Err(format!(
            "{name} sample hour {sample} differs from frame hour {hour}"
        ))
    }
}

fn division(frame: &DecodedFrame, handle: u64) -> Fact<&rows::DivisionRow> {
    row::<tables::Divisions>(frame, handle, |r| r.id, "Divisions")
}

fn province(frame: &DecodedFrame, id: u32) -> Fact<&rows::ProvinceRow> {
    row::<tables::Provinces>(frame, u64::from(id), |r| u64::from(r.id), "Provinces")
}

fn attack(frame: &DecodedFrame, handle: u64) -> Fact<&rows::DivisionAttackInputs> {
    let status = row::<tables::DivisionStatus>(frame, handle, |r| r.division, "DivisionStatus")?;
    let attack = status.attack.require("DivisionStatus.attack")?;
    fresh(frame.hour, attack.sample_hour, "DivisionStatus.attack")?;
    Ok(attack)
}

/// Each full handle substitutes for the exact wrapper only after the writer's
/// registry->same-wrapper certificate. Census agreement catches a corrupt join.
fn template(frame: &DecodedFrame, handle: u64) -> Fact<u64> {
    let template = field(
        attack(frame, handle)?.template,
        "DivisionStatus.attack.template",
    )?;
    if template != 0 && template as u32 != 52 {
        return Err("DivisionStatus.attack.template is not a full type52 handle".into());
    }
    if template != division(frame, handle)?.template {
        return Err("DivisionStatus.attack.template differs from the same-hour census".into());
    }
    Ok(template)
}

/// Resolve raw0 through the genuine reserved object; a positive alias to slot0
/// also reaches that object. Negative and absent links never imply native null.
fn slot(context: &Context<'_>, raw: u32) -> Fact<u32> {
    slot_frame(context.frame, raw)
}
fn slot_frame(frame: &DecodedFrame, raw: u32) -> Fact<u32> {
    if (raw as i32) < 0 {
        return Err("raw country mapping has a signed-negative unsupported reference".into());
    }
    if raw != 0 {
        // Complete typed normal input has priority. Unavailable or wrong-type
        // normal input cannot turn into an implicit missing alias.
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        if let Ok(index) = links.binary_search_by_key(&raw, |link| link.id) {
            let canonical = links[index].canonical_slot;
            if canonical == 0 {
                return Err("CountryLinks normal row has reserved mapping".into());
            }
            actual_raw_frame(frame, canonical)?;
            return Ok(canonical);
        }
    }
    let observed = raw_observation(frame, raw as i32)?;
    match observed.mapping {
        rows::RawCountryMapping::Observed {
            canonical_slot: 0,
            country_id: 0,
        } => Ok(0),
        rows::RawCountryMapping::Observed { .. } => {
            Err("omitted CountryLinks row has no exact reserved mapping".into())
        }
        rows::RawCountryMapping::Refused => {
            Err("raw country mapping was refused when recorded".into())
        }
        rows::RawCountryMapping::Unreached => Err("raw country mapping was not reached".into()),
    }
}

fn raw_observation(frame: &DecodedFrame, raw: i32) -> Fact<&rows::RawCountryObservation> {
    let [reserved] = frame
        .complete::<tables::ReservedCountryNaming>()
        .map_err(|e| e.to_string())?
    else {
        return Err("raw country observation requires the complete naming singleton".into());
    };
    let packet = reserved
        .raw_country_observations
        .require("ReservedCountryNaming.raw_country_observations")?
        .as_ref()
        .ok_or("raw country observation packet was refused when recorded")?;
    packet.entry(frame.hour, raw).map_err(str::to_owned)
}

fn actual_raw(context: &Context<'_>, slot: u32) -> Fact<u32> {
    actual_raw_frame(context.frame, slot)
}
fn actual_raw_frame(frame: &DecodedFrame, slot: u32) -> Fact<u32> {
    if slot == 0 {
        return Ok(0);
    }
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let mut found = countries.iter().filter(|r| r.canonical_slot == slot);
    let id = found
        .next()
        .ok_or_else(|| format!("Countries lacks canonical slot {slot}"))?
        .id;
    if found.next().is_some() {
        return Err(format!("Countries duplicates canonical slot {slot}"));
    }
    Ok(id)
}

fn country_attack<'a>(context: &Context<'a>, raw: u32) -> Fact<&'a rows::CountryAttackInputs> {
    let canonical = slot(context, raw)?;
    let packet = if canonical == 0 {
        match context
            .frame
            .complete::<tables::ReservedCountryNaming>()
            .map_err(|e| e.to_string())?
        {
            [reserved] => reserved
                .attack
                .require("ReservedCountryNaming.attack")?
                .as_ref()
                .ok_or("ReservedCountryNaming.attack was refused when recorded")?,
            _ => return Err("ReservedCountryNaming has no complete singleton".into()),
        }
    } else {
        row::<tables::CountryCounterInputs>(
            context.frame,
            u64::from(actual_raw(context, canonical)?),
            |r| u64::from(r.country),
            "CountryCounterInputs",
        )?
        .attack
        .require("CountryCounterInputs.attack")?
    };
    fresh(context.frame.hour, packet.sample_hour, "country attack")?;
    Ok(packet)
}

fn commander(frame: &DecodedFrame, group: u64) -> Fact<Option<&rows::CommanderAttackInputs>> {
    if group == 0 {
        return Ok(None);
    }
    let row = row::<tables::OrdersGroupCounterInputs>(
        frame,
        group,
        |r| r.group,
        "OrdersGroupCounterInputs",
    )?;
    let packet = row.attack.require("OrdersGroupCounterInputs.attack")?;
    fresh(
        frame.hour,
        packet.sample_hour,
        "OrdersGroupCounterInputs.attack",
    )?;
    let commander = packet
        .commander
        .as_ref()
        .ok_or("OrdersGroupCounterInputs.attack direct commander selector was refused")?;
    Ok(commander.as_ref())
}

fn leader_scalar(
    leader: &Fact<Option<&rows::CommanderAttackInputs>>,
    get: impl Fn(&rows::CommanderAttackInputs) -> Option<i64>,
    name: &str,
) -> Fact<i64> {
    match leader {
        Ok(None) => Ok(0),
        Ok(Some(leader)) => field(get(leader), name),
        Err(e) => Err(e.clone()),
    }
}

fn terrain(domain: &Option<Vec<rows::AttackTerrain>>, index: i32, name: &str) -> Fact<i64> {
    let domain = domain
        .as_ref()
        .ok_or_else(|| format!("{name} terrain domain was refused"))?;
    if !domain.windows(2).all(|p| p[0].terrain < p[1].terrain) {
        return Err(format!("{name} terrain domain is not sorted and unique"));
    }
    let i = domain
        .binary_search_by_key(&index, |entry| entry.terrain)
        .map_err(|_| format!("{name} terrain {index} is outside the authenticated live domain"))?;
    field(domain[i].attack, name)
}

fn scalars(frame: &DecodedFrame) -> Fact<&rows::PoliticalRuntimeScalars> {
    let [row] = frame
        .complete::<tables::SimulationFeatures>()
        .map_err(|e| e.to_string())?
    else {
        return Err("SimulationFeatures has no complete singleton".into());
    };
    let packet = row
        .runtime_scalars
        .require("SimulationFeatures.runtime_scalars")?;
    fresh(
        frame.hour,
        packet.sample_hour,
        "SimulationFeatures.runtime_scalars",
    )?;
    Ok(packet)
}

fn weather_region(frame: &DecodedFrame, region: u32) -> Fact<&rows::WeatherRegionRow> {
    row::<tables::WeatherRegions>(
        frame,
        u64::from(region),
        |r| u64::from(r.id),
        "WeatherRegions",
    )
}

fn weather_province(frame: &DecodedFrame, id: u32) -> Fact<&rows::WeatherProvinceRow> {
    row::<tables::WeatherProvinces>(
        frame,
        u64::from(id),
        |r| u64::from(r.id),
        "WeatherProvinces",
    )
}

fn pair(values: [Option<i64>; 2], name: &str) -> Fact<[i64; 2]> {
    Ok([
        field(values[0], &format!("{name} A"))?,
        field(values[1], &format!("{name} B"))?,
    ])
}

/// The sentinel region row certifies a present strategic-air object. Complete
/// absence is native-null; an absent region pair on that object is native zero.
fn enemy_air(context: &Context<'_>, division: &rows::DivisionRow) -> Fact<bool> {
    let owner = slot(context, division.logical_owner)?;
    let region = context
        .content
        .icons
        .raids
        .selected_region(division.province)?;
    let rows = context
        .frame
        .complete::<tables::RaidAirScores>()
        .map_err(|e| e.to_string())?;
    let mut enemy = None;
    let mut present = false;
    for row in rows {
        // A strategic-air country reference may be an alias; its producer checks
        // the canonical physical slot. Match existing intel::scores without
        // demanding unrelated country links in this fragment.
        let row_owner = if row.country == 0 {
            Some(0)
        } else {
            context.links.get(&row.country).copied()
        };
        if row_owner != Some(owner) {
            continue;
        }
        present = true;
        if row.region == region && enemy.replace(row.enemy).is_some() {
            return Err("RaidAirScores duplicates the logical-owner/current-region pair".into());
        }
    }
    Ok(present && enemy.unwrap_or(0) > 0)
}

fn projection(context: &Context<'_>, handle: u64, source: u32, target: u32) -> arithmetic::Preview {
    // Preserve independent failures. No failed packet is converted to zeros.
    let frame = context.frame;
    let unit = division(frame, handle);
    let packet = attack(frame, handle);
    let leader = unit
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|unit| commander(frame, unit.orders_group));
    let neighbour = context
        .content
        .icons
        .raids
        .selected_neighbour(source, target);
    let target_region = context.content.icons.raids.selected_region(target);
    let target_terrain = context
        .content
        .movement
        .as_deref()
        .ok_or_else(|| "selected attack terrain content is unavailable".to_owned())
        .and_then(|content| {
            content
                .map
                .terrain(target)
                .ok_or_else(|| format!("target province {target} has no content terrain"))
        });
    let terrain_value = (|| {
        let index = target_terrain.as_ref().map_err(Clone::clone)?.index;
        let attack = terrain(
            &packet.as_ref().map_err(Clone::clone)?.terrain,
            index,
            "division attack",
        )?;
        let adjustment = match &leader {
            Ok(None) => 0,
            Ok(Some(leader)) => terrain(&leader.terrain, index, "direct commander attack")?,
            Err(reason) => return Err(reason.clone()),
        };
        Ok(attack.wrapping_add(adjustment))
    })();
    let river = (|| {
        let movement = context
            .content
            .movement
            .as_deref()
            .ok_or_else(|| "selected attack river content is unavailable".to_owned())?;
        let Some(kind) = movement.map.neighbour_kind(source, target)? else {
            return Ok(None);
        };
        let define = match kind {
            2 => field(scalars(frame)?.river_small, "runtime river_small")?,
            3 => field(scalars(frame)?.river_large, "runtime river_large")?,
            _ => return Ok(None),
        };
        let unit = pair(
            packet.as_ref().map_err(Clone::clone)?.stats.river,
            "division river attack",
        )?;
        let leader = leader_scalar(&leader, |r| r.river, "commander river attack")?;
        Ok(Some(arithmetic::river(unit, leader, define)))
    })();
    let fort = (|| {
        let province = province(frame, target)?;
        let fort = province.attack.require("Provinces.attack")?;
        fresh(frame.hour, fort.sample_hour, "Provinces.attack")?;
        let healthy = field(fort.fort_sum, "province healthy fort sum")?;
        if healthy <= 0 {
            return Ok(None);
        }
        let country = fixed_scalar(
            frame,
            country_attack(context, province.controller)?.land_bunker_effectiveness_factor,
            replay::fixed_layout::SELECTED_ATTACK_FORT,
            "country land bunker effectiveness",
        )?;
        let state = field(
            fort.land_bunker_effectiveness_factor,
            "state land bunker effectiveness",
        )?;
        let define = field(scalars(frame)?.fort, "runtime fort")?;
        let base = arithmetic::fort_base(Some(country), state, define);
        let unit = pair(
            packet.as_ref().map_err(Clone::clone)?.stats.fort,
            "division fort attack",
        )?;
        let leader = leader_scalar(&leader, |r| r.fort, "commander fort attack")?;
        Ok(arithmetic::fort(unit, leader, healthy, base))
    })();
    let snow = (|| {
        if neighbour.as_ref().map_err(Clone::clone)?.is_none() {
            return Ok(None);
        }
        let weather = weather_province(frame, target)?;
        let ceiling =
            weather_region(frame, *target_region.as_ref().map_err(Clone::clone)?)?.max_snow;
        if !arithmetic::has_snow(weather.snow, weather.permanent_snow, ceiling) {
            return Ok(None);
        }
        let unit = field(
            packet.as_ref().map_err(Clone::clone)?.stats.snow,
            "division snow attack",
        )?;
        let leader = leader_scalar(&leader, |r| r.snow, "commander snow attack")?;
        Ok(Some(arithmetic::mul(
            unit.wrapping_add(leader),
            arithmetic::ONE,
        )))
    })();
    let weather = (|| {
        let row = weather_region(frame, *target_region.as_ref().map_err(Clone::clone)?)?;
        let weather = row.attack.require("WeatherRegions.attack")?;
        fresh(frame.hour, weather.sample_hour, "WeatherRegions.attack")?;
        let pair = [
            fixed_scalar(
                frame,
                weather.attack,
                replay::fixed_layout::SELECTED_ATTACK_WEATHER_ATTACK,
                "region weather attack",
            )?,
            fixed_scalar(
                frame,
                weather.breakthrough,
                replay::fixed_layout::SELECTED_ATTACK_WEATHER_BREAKTHROUGH,
                "region weather breakthrough",
            )?,
        ];
        // Both native products are exactly zero for every possible factor.
        if pair == [0, 0] {
            return Ok(None);
        }
        let factor = field(
            packet.as_ref().map_err(Clone::clone)?.acclimatization,
            "fresh attack acclimatization",
        )?;
        Ok(Some(arithmetic::weather(pair, factor)))
    })();
    let night = (|| {
        let factor = *weather_province(frame, target)?
            .night_factor
            .require("WeatherProvinces.night_factor")?;
        if factor <= 80_000 {
            return Ok(None);
        }
        let unit = unit.as_ref().map_err(Clone::clone)?;
        let owner = if unit.expeditionary_owner as i32 > 0 {
            unit.expeditionary_owner
        } else {
            unit.roster_owner
        };
        let stat = field(
            packet.as_ref().map_err(Clone::clone)?.stats.night,
            "division night attack",
        )?;
        let define = field(scalars(frame)?.night, "runtime night")?;
        let country = fixed_scalar(
            frame,
            country_attack(context, owner)?.land_night_attack,
            replay::fixed_layout::SELECTED_ATTACK_NIGHT,
            "country night attack",
        )?;
        let leader = leader_scalar(&leader, |r| r.night, "commander night attack")?;
        Ok(Some(arithmetic::night(stat, define, country, leader)))
    })();
    let air = unit
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|unit| enemy_air(context, unit));
    arithmetic::rows(arithmetic::Projection {
        terrain: terrain_value,
        river,
        fort,
        snow,
        weather,
        night,
        enemy_air_positive: air,
    })
}

fn fallback_key(bytes: &[u8]) -> Fact<&str> {
    std::str::from_utf8(installed_content::parser::c_string(bytes))
        .map_err(|_| "template fallback localisation key is not UTF-8".to_owned())
}

/// Preserve the original-byte empty gate before this callback. An originally
/// nonempty key whose C string is empty still receives qualification.
fn localise_template_fallback(
    localisation: &installed_content::localisation::Localisation,
    tag: impl AsRef<[u8]>,
    bytes: &[u8],
) -> Fact<String> {
    // Concatenate the full raw bytes before treating the candidate as a C
    // string. UTF-8 admission follows the first-NUL cut independently of bare
    // fallback. A defined empty qualified value stays authoritative.
    let tag = tag.as_ref();
    let length = tag
        .len()
        .checked_add(1)
        .and_then(|len| len.checked_add(bytes.len()))
        .ok_or("template qualified localisation key length overflow")?;
    let mut qualified = Vec::new();
    engine::allocation::reserve_exact(&mut qualified, length).map_err(|e| e.to_string())?;
    qualified.extend_from_slice(tag);
    qualified.push(b'_');
    qualified.extend_from_slice(bytes);
    let candidate = fallback_key(&qualified)?;
    let key = if localisation.get(candidate).is_some() {
        candidate
    } else {
        fallback_key(bytes)?
    };
    crate::combat::text::localise(localisation, key, &[])
}

fn heading(context: &Context<'_>, template: u64) -> Fact<String> {
    let icon = row::<tables::DivisionTemplateIcons>(
        context.frame,
        template,
        |r| r.template,
        "DivisionTemplateIcons",
    )?;
    // Only SameAs needs the complete daily dictionary. Changed is self-contained.
    let display = icon
        .display_name
        .require("DivisionTemplateIcons.display_name")?
        .as_ref()
        .ok_or("DivisionTemplateIcons.display_name was refused when recorded")?;
    let daily = if matches!(
        &display.value,
        rows::TemplateNameValue::SameAsDivisionTemplates
    ) {
        Some(row::<tables::DivisionTemplates>(
            context.frame,
            template,
            |r| r.template,
            "DivisionTemplates",
        )?)
    } else {
        None
    };
    let parts = icon
        .display_name_inputs(context.frame.hour, daily)
        .map_err(str::to_owned)?;
    let name = dashboard::unit_names::template_name(parts, icon.country, |raw, key| {
        // This callback is not called for literal or known empty fallback.
        // Exact signed-country tag_string proof is required; no <=0 plain-key shortcut.
        let observed = raw_observation(context.frame, raw)?;
        let tag = match &observed.tag {
            rows::RawCountryTag::Bytes(bytes) => bytes.as_slice(),
            rows::RawCountryTag::Refused => {
                return Err("template exact raw country tag was refused when recorded".into());
            }
            rows::RawCountryTag::Unreached => {
                return Err("template exact raw country tag was not reached".into());
            }
        };
        localise_template_fallback(&context.content.names.localisation, tag, key)
    })?;
    localise(
        &context.content.names.localisation,
        "LAND_COMBAT_ATTACK_PROVINCE_MODIFIERS",
        &[("DIVISION", Arg::Text(&name))],
    )
}

fn row_text(context: &Context<'_>, row: arithmetic::Row) -> Fact<String> {
    let key = crate::combat::modifier_name_key(row.kind)?;
    let name = localise(&context.content.names.localisation, key, &[])?;
    Ok(arithmetic::modifier_text(&name, row.value))
}

fn blocked(
    context: &Context<'_>,
    queries: &CounterQueries<'_>,
    handle: u64,
    current: u32,
    target: u32,
) -> Fact<bool> {
    let Some(SelectedNeighbour { through }) = context
        .content
        .icons
        .raids
        .selected_neighbour(current, target)?
    else {
        return Ok(false);
    };
    let Some(through) = through? else {
        return Ok(false);
    };
    let owner = division(context.frame, handle)?.logical_owner;
    Ok(queries.selected_strait(through, owner)? == StraitFleet::Visible)
}

/// Raw gates precede canonical and directed-war operands. In particular a
/// A positive raw alias to actual zero preserves positivity; this direct
/// check has no positive receiver-ID veto.
fn outer_gate(
    source_raw: u32,
    mut target_raw: impl FnMut() -> Fact<u32>,
    mut canonical: impl FnMut(u32) -> Fact<u32>,
    mut direct_war: impl FnMut(u32) -> Fact<bool>,
) -> Fact<bool> {
    if source_raw as i32 <= 0 {
        return Ok(false);
    }
    let target_raw = target_raw()?;
    if target_raw as i32 <= 0 || source_raw == target_raw {
        return Ok(false);
    }
    let target_slot = canonical(target_raw)?;
    if canonical(source_raw)? == target_slot {
        return Ok(false);
    }
    direct_war(target_slot)
}

fn first_template(
    identity: Fact<u64>,
    seen: &mut BTreeSet<u64>,
    earlier_unknown: &mut bool,
) -> Fact<Option<u64>> {
    let template = match identity {
        Ok(template) => template,
        Err(reason) => {
            *earlier_unknown = true;
            return Err(reason);
        }
    };
    if !seen.insert(template) {
        return Ok(None);
    }
    if *earlier_unknown {
        return Err("an earlier unknown template may own this first representative".into());
    }
    Ok(Some(template))
}

/// One borrowed SelectionTravel.route for ALL representatives. A duration
/// fallback never replaces this route. The closure is the shared same-hour
/// directed-war reader; it must refuse every uncertified target.
pub(super) fn append(
    out: &mut Tooltip,
    context: &Context<'_>,
    members: &[u64],
    route: &Fact<Vec<u32>>,
    queries: &CounterQueries<'_>,
    mut direct_war: impl FnMut(u32, u32) -> Fact<bool>,
) {
    let locations: Vec<_> = members
        .iter()
        .map(|&handle| division(context.frame, handle).map(|unit| unit.province))
        .collect();
    let current = match arithmetic::same_current(&locations) {
        Ok(Some(current)) => current,
        Ok(None) => return,
        Err(reason) => {
            refuse(out, "attack same-source gate", reason);
            return;
        }
    };
    let mut seen = BTreeSet::new();
    let mut earlier_unknown = false;
    for &handle in members {
        let template = match first_template(
            template(context.frame, handle),
            &mut seen,
            &mut earlier_unknown,
        ) {
            Ok(Some(template)) => template,
            Ok(None) => continue,
            Err(reason) => {
                refuse(out, "attack representative", reason);
                continue;
            }
        };
        let edge = match arithmetic::retained_edge(
            current,
            route.as_ref().map(Vec::as_slice).map_err(Clone::clone),
        ) {
            Ok(edge) => edge,
            Err(reason) => {
                refuse(out, "attack retained route", reason);
                continue;
            }
        };
        let Some((source, target)) = edge else {
            match blocked(context, queries, handle, current, context.province.id) {
                Ok(true) => match localise(
                    &context.content.names.localisation,
                    "UNIT_MOVEMENT_BLOCKED_BY_STRAIT",
                    &[],
                ) {
                    Ok(text) if !text.is_empty() => line(out, "blocked strait", 1, Ok(text)),
                    Ok(_) => {}
                    Err(reason) => refuse(out, "blocked strait", reason),
                },
                Ok(false) => {}
                Err(reason) => refuse(out, "blocked strait", reason),
            }
            continue;
        };
        let gate = (|| {
            let source_raw = province(context.frame, source)?.controller;
            outer_gate(
                source_raw,
                || province(context.frame, target).map(|p| p.controller),
                |raw| slot(context, raw),
                |target_slot| {
                    direct_war(division(context.frame, handle)?.logical_owner, target_slot)
                },
            )
        })();
        match gate {
            Ok(false) => continue,
            Err(reason) => {
                refuse(out, "attack outer gate", reason);
                continue;
            }
            Ok(true) => {}
        }
        let preview = projection(context, handle, source, target);
        if !preview.rows.is_empty() {
            line(out, "attack heading", 1, heading(context, template));
        } else if !preview.refusals.is_empty() {
            refuse(
                out,
                "attack heading gate",
                "the native modifier-list count is unavailable".into(),
            );
        }
        for row in preview.rows {
            line(out, "attack modifier", 2, row_text(context, row));
        }
        for (name, reason) in preview.refusals {
            refuse(out, &format!("attack {name}"), reason);
        }
    }
}
