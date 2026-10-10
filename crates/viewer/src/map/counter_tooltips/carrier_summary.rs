//! CAirBase::summary@140C45850, shared by at-sea and port carrier controls.
//! Membership stays the base's registered pools, never wings merely located there.

use super::{Arg, Context, Localisation, Tooltip, append, localise, refuse};
use replay::rows::{AirBaseRegistrationRow, AirBaseRow, AirWingRow, CarrierAirPool, tables};
use std::collections::BTreeMap;

/// 140FFE9A0 has an exact single-mask switch. Unknown and combined masks
/// produce empty mission text, rather than selecting the first active bit.
fn mission_key(mask: u32) -> Option<&'static str> {
    Some(match mask {
        0x1 => "AIRWING_MISSION_TYPE_AIR_SUPERIORITY",
        0x2 => "AIRWING_MISSION_TYPE_CAS",
        0x4 => "AIRWING_MISSION_TYPE_INTERCEPTION",
        0x8 => "AIRWING_MISSION_TYPE_STRATEGIC_BOMBER",
        0x10 => "AIRWING_MISSION_TYPE_NAVAL_BOMBER",
        0x20 => "AIRWING_MISSION_TYPE_DROP_NUKE",
        0x40 => "AIRWING_MISSION_TYPE_PARADROP",
        0x80 => "AIRWING_MISSION_TYPE_NAVAL_KAMIKAZE",
        0x100 => "AIRWING_MISSION_TYPE_PORT_STRIKE",
        0x200 => "AIRWING_MISSION_TYPE_ATTACK_LOGISTICS",
        0x400 => "AIRWING_MISSION_TYPE_AIR_SUPPLY",
        0x800 => "AIRWING_MISSION_TYPE_TRAINING",
        0x1000 => "AIRWING_MISSION_TYPE_NAVAL_MINE_PLANTING",
        0x2000 => "AIRWING_MISSION_TYPE_NAVAL_MINE_SWEEPING",
        0x4000 => "AIRWING_MISSION_TYPE_RECON",
        0x8000 => "AIRWING_MISSION_TYPE_NAVAL_PATROL",
        0x10000 => "AIRWING_MISSION_TYPE_BARRAGE",
        0x20000 => "AIRWING_MISSION_TYPE_SAM",
        _ => return None,
    })
}

/// 140C3BA80 counts positive-plane WINGS by their native (pool short name,
/// status) string pair; 140C34660 sorts the pair by bytes. The caller passes
/// the authentic pool owner's name selection, independently of wing overrides.
pub(super) fn wing_groups<'a>(
    loc: &Localisation,
    kind: i32,
    carrier_mission: bool,
    wings: impl IntoIterator<Item = &'a AirWingRow>,
    mut pool_name: impl FnMut(&AirWingRow) -> Result<String, String>,
) -> Result<String, String> {
    let mut groups = BTreeMap::<(String, String), i32>::new();
    for wing in wings {
        if wing.planes < 1 {
            continue;
        }
        let status = if wing.mission_mask == 0
            || ((wing.mission_region.is_none() || wing.mission_active == 0)
                && wing.mission_mask & 0x800 == 0)
        {
            localise(
                loc,
                match (kind == 1, carrier_mission) {
                    (true, true) => "ROCKET_WING_STATUS_EQUIPMENT_MISSION",
                    (false, true) => "AIRWING_MISSION_WING_STATUS_CARRIER_MISSION",
                    _ => "AIRWING_MISSION_WING_STATUS_IDLE",
                },
                &[],
            )?
        } else {
            let mission = mission_key(wing.mission_mask)
                .map(|key| localise(loc, key, &[]))
                .transpose()?
                .unwrap_or_default();
            localise(
                loc,
                "AIRWING_MISSION_WING_STATUS_MISSION",
                &[("MIS", Arg::Text(&mission))],
            )?
        };
        let count = groups.entry((pool_name(wing)?, status)).or_default();
        *count = count.wrapping_add(1);
    }
    let mut text = String::new();
    for ((name, status), count) in groups {
        let key = if kind == 1 {
            "ROCKET_MISSION_WING_STATUS_1"
        } else if count == 1 {
            "AIRWING_MISSION_WINGS_STATUS_1"
        } else {
            "AIRWING_MISSION_WINGS_STATUS_2"
        };
        text.push_str(&localise(
            loc,
            key,
            &[
                ("NUM", Arg::Int32(count)),
                ("NAME", Arg::Text(&name)),
                ("STATUS", Arg::Text(&status)),
            ],
        )?);
        text.push('\n');
    }
    Ok(text)
}

/// Stateless tier-three header; native capacity comparison is signed, and
/// positive total occupancy alone admits the country-group text. This helper
/// leaves the caller's separately reached fuel footer and delayed body intact.
pub(super) fn full_header(
    loc: &Localisation,
    kind: i32,
    host_name: &str,
    planes: i32,
    capacity: i32,
    countries: &str,
) -> Result<String, String> {
    let mut text = base_header(loc, kind, host_name)?;
    text.push_str(&occupancy(loc, kind, planes, capacity)?);
    if planes > 0 {
        text.push_str(countries);
    }
    text.push('\n');
    Ok(text)
}

fn base_header(loc: &Localisation, kind: i32, host_name: &str) -> Result<String, String> {
    let key = match kind {
        0 => "AIR_BASE_LEVEL_UNKNOWN",
        1 => "ROCKET_SITE_LEVEL_UNKNOWN",
        2 => "GUN_EMPLACEMENT_LEVEL_UNKNOWN",
        _ => return Err(format!("CAirBase kind {kind} has no native summary header")),
    };
    let mut text = localise(loc, key, &[("NAME", Arg::Text(host_name))])?;
    crate::combat::text::ensure_newlines(&mut text, 2);
    Ok(text)
}

fn occupancy(loc: &Localisation, kind: i32, planes: i32, capacity: i32) -> Result<String, String> {
    let key = match kind {
        0 | 2 => "AIRBASE_AIRPLANES_KNOWN",
        1 => "ROCKETBASE_ROCKETS_KNOWN",
        _ => {
            return Err(format!(
                "CAirBase kind {kind} has no native occupancy header"
            ));
        }
    };
    let colour = if capacity < planes { 'R' } else { 'G' };
    let value = format!("\u{11}{colour}{planes}\u{11}!");
    let capacity = format!("\u{11}{colour}{capacity}\u{11}!");
    let mut text = localise(
        loc,
        key,
        &[
            ("VALUE", Arg::Text(&value)),
            ("CAPACITY", Arg::Text(&capacity)),
        ],
    )?;
    text.push('\n');
    Ok(text)
}

/// 140C45850's tier-zero/estimated title branch. `None` selects native
/// NO_INTEL; a missing reached estimate must instead be refused by the caller.
pub(super) fn low_header(
    loc: &Localisation,
    kind: i32,
    host_name: &str,
    estimate: Option<([i32; 2], bool)>,
    countries: &str,
) -> Result<String, String> {
    let mut text = if let Some((range, known)) = estimate {
        let value = count_text(range, known);
        localise(
            loc,
            "AIRBASE_AIRPLANES_ESTIMATED",
            &[("BASE", Arg::Text(host_name)), ("VALUE", Arg::Text(&value))],
        )?
    } else {
        let key = match kind {
            0 => "AIR_BASE_LEVEL_UNKNOWN",
            1 => "ROCKET_SITE_LEVEL_UNKNOWN",
            2 => "GUN_EMPLACEMENT_LEVEL_UNKNOWN",
            _ => return Err(format!("CAirBase kind {kind} has no native summary header")),
        };
        localise(loc, key, &[("NAME", Arg::Text(host_name))])?
    };
    crate::combat::text::ensure_newlines(&mut text, 2);
    text.push('\n');
    if estimate.is_none() {
        text.push_str(&localise(loc, "NO_INTEL", &[])?);
    }
    if !countries.is_empty() {
        text.push_str("\n\n");
        text.push_str(countries);
    }
    Ok(text)
}

/// The native country group intro ends with ':\n'; the base title above
/// uses its own localisation and does not gain this punctuation.
pub(super) fn full_country_header(tag: &str, name: &str) -> String {
    format!("\u{12}{tag}\u{11}H{name}\u{11}!:\n")
}

/// 140C1F290's fuel quantity formatter. This is not the localisation
/// metric suffix modifier: its positive cutoffs and rounding differ.
fn fuel_amount(raw: i64) -> String {
    use installed_content::fixed5::format_fixed5;
    let (add, divisor, precision, suffix) = match raw {
        value if value < 0x5f51db0 => return format_fixed5(value, false, 0),
        value if value < 0x1748291620 => (100_000, 100_000_000, 1, 'K'),
        value if value < 0xe8d15c4da0 => (5_100_000, 100_000_000_000, 3, 'M'),
        _ => (55_100_000, 100_000_000_000, 2, 'M'),
    };
    let rounded = raw.wrapping_add(add);
    let value = if (rounded as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        rounded.wrapping_mul(100_000) / divisor
    } else {
        0xffff_ffff
    };
    format!("{}{suffix}", format_fixed5(value, false, precision))
}

/// 140C44DB0 supplies hourly stored amounts; 140C43070 displays daily
/// values. Native shows both numbers on surplus, not on shortage.
pub(super) fn fuel_header(
    loc: &Localisation,
    allocated: i64,
    required: i64,
) -> Result<String, String> {
    let mut text = localise(loc, "FUEL_TITLE", &[])?;
    text.push('\n');
    let allocated_raw = allocated.wrapping_mul(24);
    let required_raw = required.wrapping_mul(24);
    let required = fuel_amount(required_raw);
    if required_raw < allocated_raw {
        let allocated = fuel_amount(allocated_raw);
        text.push_str(&localise(
            loc,
            "FUEL_DAILY_REQUIRED_AND_RECEIVED",
            &[
                ("REQUIRED", Arg::Text(&required)),
                ("RECEIVED", Arg::Text(&allocated)),
            ],
        )?);
    } else {
        text.push_str(&localise(
            loc,
            "FUEL_DAILY_REQUIRED",
            &[("NUM", Arg::Text(&required))],
        )?);
    }
    text.push('\n');
    Ok(text)
}

/// 140C43070's maximum heading. The caller sums before multiplying by24;
/// the delayed description is formatted independently.
fn maximum_header(loc: &Localisation, maximum: i64) -> Result<String, String> {
    let maximum = fuel_amount(maximum.wrapping_mul(24));
    let mut title = localise(
        loc,
        "FUEL_DAILY_REQUIRED_MAX",
        &[("NUM", Arg::Text(&maximum))],
    )?;
    title.push('\n');
    Ok(title)
}

/// 140C2F510 reads its known byte independently from the signed range.
/// Carrier caller delimiter1427500A4 is exactly '-'.
pub(super) fn count_text([lower, upper]: [i32; 2], known: bool) -> String {
    if !known {
        "?".into()
    } else if lower == upper {
        lower.to_string()
    } else {
        format!("{lower}-{upper}")
    }
}

/// 140C42A10 emits a line for every subtype pool group, including empty
/// pools and signed-nonpositive plane totals. Fuel is summed before times24.
pub(super) fn fuel_line(
    loc: &Localisation,
    planes: i32,
    subtype: &str,
    category: u64,
    nato: bool,
    hourly: i64,
) -> Result<String, String> {
    let amount = fuel_amount(hourly.wrapping_mul(24));
    let amount = format!("\u{11}H{amount}\u{11}!");
    let fuel = localise(loc, "FUEL_DAILY", &[("AMOUNT", Arg::Text(&amount))])?;
    let icon = super::units::subunit_icon(subtype, category, nato);
    Ok(format!(" {planes}  {icon} {fuel}\n"))
}

fn registration<'a>(
    context: &Context<'a>,
    base: u64,
    slot: u32,
) -> Result<&'a AirBaseRegistrationRow, String> {
    optional_registration(context, base, slot)?
        .ok_or_else(|| format!("AirBaseRegistrations base {base} slot {slot} is absent"))
}

/// 140B00380 returns the viewer's indexed pointer; a null container is valid
/// absence in 140C45850. Registered-country occurrences still require a row.
fn optional_registration<'a>(
    context: &Context<'a>,
    base: u64,
    slot: u32,
) -> Result<Option<&'a AirBaseRegistrationRow>, String> {
    let rows = context
        .frame
        .complete::<tables::AirBaseRegistrations>()
        .map_err(|e| e.to_string())?;
    Ok(rows
        .binary_search_by_key(&(base, slot), |row| (row.base, row.slot))
        .ok()
        .map(|index| &rows[index]))
}

fn wing<'a>(context: &Context<'a>, id: u64) -> Result<&'a AirWingRow, String> {
    let rows = context
        .frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&id, |row| row.id)
        .map(|index| &rows[index])
        .map_err(|_| format!("AirWings wing {id} is absent"))
}

fn planes(context: &Context<'_>, ids: &[u64]) -> Result<i32, String> {
    ids.iter().try_fold(0_i32, |sum, &id| {
        Ok(sum.wrapping_add(wing(context, id)?.planes))
    })
}

fn slot(context: &Context<'_>, raw: u32) -> Result<u32, String> {
    if raw == 0 { Ok(0) } else { context.slot(raw) }
}

fn tag<'a>(context: &Context<'a>, raw: u32) -> Result<&'a str, String> {
    if raw == 0 {
        return installed_content::country_names::reserved_zero_tag(context.frame);
    }
    let rows = context
        .frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let country = rows
        .binary_search_by_key(&raw, |row| row.id)
        .map(|index| &rows[index])
        .map_err(|_| format!("Countries country {raw} is absent"))?;
    std::str::from_utf8(&country.tag)
        .map_err(|_| format!("Countries country {raw} tag is not UTF-8"))
}

fn seed(context: &Context<'_>, numeric_id: u32) -> Result<u32, String> {
    let [clock] = context
        .frame
        .complete::<tables::Clock>()
        .map_err(|e| e.to_string())?
    else {
        return Err("Clock singleton is unavailable for carrier count estimate".into());
    };
    Ok((clock.seed as u32).wrapping_add(numeric_id))
}

/// 140C448F0 falls back to controller zero for an absent host, but the
/// stateless summary's subsequent mission pretest dereferences that same host
/// and its task force (140C45850). Do not invent a completed null-host summary.
fn host(context: &Context<'_>, base: &AirBaseRow) -> Result<(u32, u32, bool), String> {
    if base.state.is_some() {
        return Err(format!(
            "CAirBase {} is state-backed, not a carrier summary",
            base.id
        ));
    }
    if base.host == 0 {
        return Err(format!(
            "CAirBase {} null host reaches the native-invalid mission pretest",
            base.id
        ));
    }
    let ships = context
        .frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string())?;
    let ship = ships
        .binary_search_by_key(&base.host, |row| row.id)
        .map(|index| &ships[index])
        .map_err(|_| {
            format!(
                "CAirBase {} stale host {} reaches the native-invalid mission pretest",
                base.id, base.host
            )
        })?;
    let forces = context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|e| e.to_string())?;
    let force = match forces.binary_search_by_key(&ship.task_force, |row| row.id) {
        Ok(index) if ship.task_force != 0 => &forces[index],
        _ => {
            return Err(format!(
                "CAirBase {} live host {} has a native-invalid task force {}",
                base.id, base.host, ship.task_force
            ));
        }
    };
    // 140C24B40 stores type at +8 and numeric id at +C. 140C45850
    // seeds the estimate from +C: the high word of the full reference.
    Ok((
        slot(context, force.roster_owner)?,
        (ship.id >> 32) as u32,
        force.mission != 0,
    ))
}

fn host_name(context: &Context<'_>, base: &AirBaseRow) -> Result<String, String> {
    let inputs = base
        .carrier
        .require("AirBases.carrier")?
        .as_ref()
        .ok_or_else(|| {
            format!(
                "CAirBase {} carrier inputs were unreadable when recorded",
                base.id
            )
        })?;
    let name = inputs.host_name.as_ref().ok_or_else(|| {
        format!(
            "CAirBase {} host name was unreadable when recorded",
            base.id
        )
    })?;
    dashboard::scripted_text::stored_text(
        context.loc,
        context.recorded_language,
        name,
        "carrier host retained name",
    )
}

fn pools(registration: &AirBaseRegistrationRow) -> Result<&[CarrierAirPool], String> {
    let inputs = registration
        .carrier
        .require("AirBaseRegistrations.carrier")?
        .as_ref()
        .ok_or_else(|| {
            format!(
                "carrier registration {}:{} inputs were unreadable when recorded",
                registration.base, registration.slot
            )
        })?;
    let pools = inputs.pools.as_deref().ok_or_else(|| {
        format!(
            "carrier registration {}:{} pool inventory was unreadable when recorded",
            registration.base, registration.slot
        )
    })?;
    let mut end = 0_usize;
    for pool in pools {
        if pool.wing_start as usize != end {
            return Err(
                "carrier pool ranges do not partition the registered wings in order".into(),
            );
        }
        end = end
            .checked_add(pool.wing_count as usize)
            .filter(|&end| end <= registration.wings.len())
            .ok_or("carrier pool range exceeds the registered wings")?;
    }
    if end != registration.wings.len() {
        return Err("carrier pool ranges leave registered wings unconsumed".into());
    }
    Ok(pools)
}

fn pool_ids<'a>(registration: &'a AirBaseRegistrationRow, pool: &CarrierAirPool) -> &'a [u64] {
    let start = pool.wing_start as usize;
    &registration.wings[start..start + pool.wing_count as usize]
}

fn country_groups(
    context: &Context<'_>,
    registration: &AirBaseRegistrationRow,
    raw: u32,
    kind: i32,
    mission: bool,
) -> Result<String, String> {
    let pools = pools(registration)?;
    let inputs = registration
        .carrier
        .require("AirBaseRegistrations.carrier")?
        .as_ref()
        .expect("pools checked");
    if inputs.country != raw || slot(context, raw)? != registration.slot {
        return Err(
            "carrier registered-country occurrence disagrees with its indexed container".into(),
        );
    }
    let mut wings = Vec::new();
    for pool in pools {
        for &id in pool_ids(registration, pool) {
            let wing = wing(context, id)?;
            if wing.planes > 0 {
                let subtype = wing
                    .pool_subunit
                    .require("AirWings.pool_subunit")?
                    .as_ref()
                    .ok_or_else(|| {
                        format!("AirWings wing {id} pool subtype was unreadable when recorded")
                    })?;
                if *subtype != pool.subunit_index || slot(context, wing.owner)? != registration.slot
                {
                    return Err(format!(
                        "AirWings wing {id} does not authenticate its registered pool"
                    ));
                }
            }
            wings.push(wing);
        }
    }
    wing_groups(context.loc, kind, mission, wings, |wing| {
        let index = wing
            .pool_subunit
            .require("AirWings.pool_subunit")?
            .as_ref()
            .expect("positive wing checked");
        let (name, _) = context
            .units
            .as_ref()
            .map_err(Clone::clone)?
            .subunit(*index)?;
        crate::combat::wing_short_name(context.loc, tag(context, raw)?, name)
    })
}

/// Bound flag validation is used only for the carrier country fragment.
fn append_country(out: &mut Tooltip, context: &Context<'_>, text: String) {
    match crate::combat::binding::displayable_flags(context.icons, text) {
        Ok(text) => out.title.push_str(&text),
        Err(reason) => refuse(out, "carrier country text", reason),
    }
}

/// Both carrier controls share this stateless CAirBase consumer. Captured
/// simulation facts are independent of the viewer's local selection and NATO.
pub(super) fn summary(context: &Context<'_>, id: u64, out: &mut Tooltip) {
    let reached = (|| {
        let rows = context
            .frame
            .complete::<tables::AirBases>()
            .map_err(|e| e.to_string())?;
        let base = rows
            .binary_search_by_key(&id, |row| row.id)
            .map(|index| &rows[index])
            .map_err(|_| format!("AirBases base {id} is absent"))?;
        let (controller, numeric_id, mission) = host(context, base)?;
        context
            .vision
            .shares_vision_or_fog(controller)
            .map_err(|reason| reason.describe())?;
        let tier = context.vision.air_port_level(controller).ok_or_else(|| {
            format!("CAirBase {id} controller {controller} air intel tier is unavailable")
        })?;
        Ok::<_, String>((base, controller, numeric_id, mission, tier))
    })();
    let (base, controller, numeric_id, mission, tier) = match reached {
        Ok(value) => value,
        Err(reason) => {
            refuse(out, "carrier summary", reason);
            return;
        }
    };
    let name = host_name(context, base);
    if tier == 0 {
        // 140C46957 -> 140C4774B: lower tiers finish at 140C47B50,
        // bypassing the full-detail fuel gate at 140C474D4.
        append(
            out,
            context,
            false,
            "carrier no-intel header",
            name.and_then(|name| low_header(context.loc, base.kind, &name, None, "")),
        );
        return;
    }
    let mut countries = Vec::new();
    let totals = (|| {
        let inputs = base
            .carrier
            .require("AirBases.carrier")?
            .as_ref()
            .ok_or_else(|| format!("CAirBase {id} carrier inputs were unreadable when recorded"))?;
        let occurrences = inputs.registered_countries.as_deref().ok_or_else(|| {
            format!("CAirBase {id} registered countries were unreadable when recorded")
        })?;
        let mut total = Ok(0_i32);
        for occurrence in occurrences {
            let country = (|| {
                if slot(context, occurrence.country)? != occurrence.slot {
                    return Err("carrier occurrence country and native slot disagree".into());
                }
                let row = registration(context, id, occurrence.slot)?;
                Ok::<_, String>((row, planes(context, &row.wings)?))
            })();
            match country {
                Ok((row, count)) => {
                    total = total.map(|total| total.wrapping_add(count));
                    if tier > 2 {
                        let text =
                            country_groups(context, row, occurrence.country, base.kind, mission)
                                .and_then(|groups| {
                                    if groups.is_empty() {
                                        return Ok(groups);
                                    }
                                    let tag = tag(context, occurrence.country)?;
                                    let name = context.country_name(occurrence.slot)?;
                                    Ok(full_country_header(tag, &name) + &groups)
                                });
                        match text {
                            Ok(text) if !text.is_empty() => countries.push(text),
                            Ok(_) => (),
                            Err(reason) => refuse(out, "carrier country wing groups", reason),
                        }
                    } else {
                        let text = (|| {
                            let estimate = context
                                .vision
                                .air_count_range(
                                    controller,
                                    count,
                                    seed(context, occurrence.slot)?,
                                    context.frame.hour,
                                )
                                .ok_or("carrier country count estimate is unavailable")?;
                            if estimate.0[1] <= 0 {
                                return Ok(String::new());
                            }
                            // The country branch forces the known byte true.
                            let amount = count_text(estimate.0, true);
                            let name = context.country_name(occurrence.slot)?;
                            let mut text = localise(
                                context.loc,
                                "AIRWING_COUNTRY_HIDDEN",
                                &[
                                    ("COUNTRY", Arg::Text(&name)),
                                    ("AMOUNT", Arg::Text(&amount)),
                                ],
                            )?;
                            text.push('\n');
                            Ok::<_, String>(text)
                        })();
                        match text {
                            Ok(text) if !text.is_empty() => countries.push(text),
                            Ok(_) => (),
                            Err(reason) => refuse(out, "carrier country count estimate", reason),
                        }
                    }
                }
                Err(reason) => {
                    refuse(out, "carrier registered-country planes", reason.clone());
                    total = Err(reason);
                }
            }
        }
        total
    })();
    if tier < 3 {
        let title = (|| {
            let estimate = context
                .vision
                .air_count_range(
                    controller,
                    totals?,
                    seed(context, numeric_id)?,
                    context.frame.hour,
                )
                .ok_or("carrier total count estimate is unavailable")?;
            low_header(context.loc, base.kind, &name?, Some(estimate), "")
        })();
        append(out, context, false, "carrier estimated header", title);
        if !countries.is_empty() {
            out.title.push_str("\n\n");
            for country in countries {
                append_country(out, context, country);
            }
        }
        return;
    }
    let positive_total = totals.as_ref().is_ok_and(|&total| total > 0);
    match (name, totals) {
        (Ok(name), Ok(total)) => {
            let title = full_header(context.loc, base.kind, &name, total, base.capacity, "").map(
                |mut title| {
                    title.pop();
                    title
                },
            );
            append(out, context, false, "carrier full header", title);
        }
        (name, total) => {
            append(
                out,
                context,
                false,
                "carrier base header",
                name.and_then(|name| base_header(context.loc, base.kind, &name)),
            );
            append(
                out,
                context,
                false,
                "carrier occupancy",
                total
                    .map_err(|reason| format!("carrier total planes are unknown: {reason}"))
                    .and_then(|total| occupancy(context.loc, base.kind, total, base.capacity)),
            );
        }
    }
    if positive_total {
        for country in countries {
            append_country(out, context, country);
        }
    }
    out.title.push('\n');
    if base.kind == 1 {
        return;
    }
    match context.vision.shares_vision_or_fog(controller) {
        Ok(true) => footer(context, id, out),
        Ok(false) => (),
        Err(reason) => refuse(out, "carrier fuel gate", reason.describe()),
    }
}

fn footer(context: &Context<'_>, base: u64, out: &mut Tooltip) {
    let row = match optional_registration(context, base, context.vision.ours()) {
        Ok(Some(row)) => row,
        Ok(None) => return,
        Err(reason) => {
            refuse(out, "carrier viewer fuel container", reason);
            return;
        }
    };
    let inputs = row
        .carrier
        .require("AirBaseRegistrations.carrier")
        .and_then(|value| {
            let inputs = value.as_ref().ok_or_else(|| {
                "carrier viewer fuel inputs were unreadable when recorded".to_owned()
            })?;
            if slot(context, inputs.country)? != row.slot {
                return Err("carrier viewer fuel country disagrees with its indexed slot".into());
            }
            Ok(inputs)
        });
    let inputs = match inputs {
        Ok(inputs) => inputs,
        Err(reason) => {
            refuse(out, "carrier viewer fuel inputs", reason);
            fuel_description(context, out);
            return;
        }
    };
    let header = (|| {
        let allocated = inputs
            .fuel_allocated_raw
            .ok_or("carrier allocated fuel was unreadable when recorded")?;
        let demand = inputs
            .fuel_demand_raw
            .ok_or("carrier demanded fuel was unreadable when recorded")?;
        fuel_header(context.loc, allocated, demand)
    })();
    append(out, context, false, "carrier fuel header", header);
    let inventory = match pools(row) {
        Ok(pools) => pools,
        Err(reason) => {
            refuse(out, "carrier fuel pool inventory", reason);
            fuel_description(context, out);
            return;
        }
    };
    let mut groups = BTreeMap::<i32, Vec<&CarrierAirPool>>::new();
    // 140C427A0's complete sort is stable, including its >=33 low-buffer
    // merges/rotations. Equal signed keys keep original inventory order;
    // 140C42A10 uses the first pool's definition even when DB indices differ.
    for pool in inventory {
        groups.entry(pool.type_index).or_default().push(pool);
    }
    for maximum in [false, true] {
        if maximum {
            crate::combat::text::ensure_newlines(&mut out.title, 2);
            let total = inventory.iter().try_fold(0_i64, |sum, pool| {
                pool.maximum_fuel_raw
                    .map(|value| sum.wrapping_add(value))
                    .ok_or_else(|| {
                        "carrier maximum fuel total contains an unreadable pool".to_owned()
                    })
            });
            append(
                out,
                context,
                false,
                "carrier maximum fuel total",
                total.and_then(|total| maximum_header(context.loc, total)),
            );
        }
        for pools in groups.values() {
            let line = (|| {
                let first = pools[0];
                let mut count = 0_i32;
                let mut fuel = 0_i64;
                for pool in pools {
                    count = count.wrapping_add(planes(context, pool_ids(row, pool))?);
                    let value = if maximum {
                        pool.maximum_fuel_raw
                    } else {
                        pool.current_fuel_raw
                    }
                    .ok_or_else(|| {
                        format!(
                            "carrier subtype {} {} fuel was unreadable when recorded",
                            first.type_index,
                            if maximum { "maximum" } else { "current" }
                        )
                    })?;
                    fuel = fuel.wrapping_add(value);
                }
                let (name, category) = context
                    .units
                    .as_ref()
                    .map_err(Clone::clone)?
                    .subunit(first.subunit_index)?;
                fuel_line(context.loc, count, name, category, context.nato, fuel)
            })();
            append(
                out,
                context,
                false,
                if maximum {
                    "carrier pool maximum fuel"
                } else {
                    "carrier pool current fuel"
                },
                line,
            );
        }
    }
    fuel_description(context, out);
}

fn fuel_description(context: &Context<'_>, out: &mut Tooltip) {
    crate::combat::text::ensure_newlines(&mut out.title, 2);
    append(
        out,
        context,
        true,
        "carrier maximum fuel description",
        localise(context.loc, "FUEL_DAILY_REQUIRED_MAX_DESC_NAVY", &[]),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{test_frame, test_put as put};
    use replay::{frame::DecodedFrame, rows::*};

    const BASE: u64 = 40;
    const SHIP: u64 = (100_u64 << 32) | 0x33;

    fn fixture() -> DecodedFrame {
        let mut frame = test_frame(60_000_000);
        put(
            &mut frame,
            TableRows::Clock(vec![ClockRow {
                hour: 60_000_000,
                seed: i32::MAX,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 1001,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 2002,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 3003,
                    canonical_slot: 3,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Countries(
                [(1001, 1, "AAA"), (2002, 2, "BBB"), (3003, 3, "CCC")]
                    .into_iter()
                    .map(|(id, canonical_slot, tag)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::CountryAppearance(
                [1001, 2002, 3003]
                    .into_iter()
                    .map(|country| CountryAppearanceRow {
                        country,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::Politics(
                [1001, 2002, 3003]
                    .into_iter()
                    .map(|country| PoliticsRow {
                        country,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(&mut frame, TableRows::PoliticalParties(vec![]));
        put(&mut frame, TableRows::Autonomy(vec![]));
        put(&mut frame, TableRows::FactionMembers(vec![]));
        put(&mut frame, TableRows::Subjects(vec![]));
        put(&mut frame, TableRows::WarRelations(vec![]));
        put(&mut frame, TableRows::UnitIntel(vec![]));
        put(&mut frame, TableRows::CountryVisibility(vec![]));
        put(
            &mut frame,
            TableRows::CountryIntel(vec![CountryIntelRow {
                owner_country: 1001,
                target_index: 3,
                air: 35_000,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: 0x20,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                roster_owner: 3003,
                mission: 2,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: SHIP,
                task_force: 7,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::ShipModels(vec![ShipModelRow {
                ship: SHIP,
                formation_flags: 1 << 22,
                counter: Some(ShipCounterInputs {
                    carrier_air_base: BASE,
                    definition_valid: 1,
                    ..Default::default()
                }),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::AirBases(vec![AirBaseRow {
                id: BASE,
                host: SHIP,
                capacity: 6,
                carrier: Recorded::Value(Some(AirBaseCarrierInputs {
                    host_name: Some(b"Host".to_vec()),
                    registered_countries: Some(vec![
                        AirBaseCountryOccurrence {
                            country: 2002,
                            slot: 2,
                        },
                        AirBaseCountryOccurrence {
                            country: 1001,
                            slot: 1,
                        },
                        AirBaseCountryOccurrence {
                            country: 2002,
                            slot: 2,
                        },
                    ]),
                })),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::AirWings(vec![
                AirWingRow {
                    id: 101,
                    owner: 1001,
                    planes: 1,
                    country_override: 3003,
                    pool_subunit: Recorded::Value(Some(1)),
                    ..Default::default()
                },
                AirWingRow {
                    id: 102,
                    owner: 2002,
                    planes: 3,
                    country_override: 1001,
                    pool_subunit: Recorded::Value(Some(1)),
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::AirBaseRegistrations(vec![
                AirBaseRegistrationRow {
                    base: BASE,
                    slot: 1,
                    capacity_raw: 1,
                    wings: vec![101],
                    transport_planes: Recorded::Value(Some(vec![Some(0), Some(1)])),
                    carrier: Recorded::Value(Some(AirBaseRegistrationCarrierInputs {
                        country: 1001,
                        fuel_demand_raw: Some(100_000),
                        fuel_allocated_raw: Some(300_000),
                        pools: Some(vec![
                            CarrierAirPool {
                                type_index: -5,
                                subunit_index: 2,
                                current_fuel_raw: Some(0),
                                maximum_fuel_raw: Some(0),
                                ..Default::default()
                            },
                            CarrierAirPool {
                                type_index: 2,
                                subunit_index: 1,
                                wing_count: 1,
                                current_fuel_raw: Some(100_000),
                                maximum_fuel_raw: Some(200_000),
                                ..Default::default()
                            },
                        ]),
                    })),
                },
                AirBaseRegistrationRow {
                    base: BASE,
                    slot: 2,
                    capacity_raw: 1,
                    wings: vec![102],
                    transport_planes: Recorded::Value(Some(vec![Some(3)])),
                    carrier: Recorded::Value(Some(AirBaseRegistrationCarrierInputs {
                        country: 2002,
                        pools: Some(vec![CarrierAirPool {
                            subunit_index: 1,
                            wing_count: 1,
                            ..Default::default()
                        }]),
                        ..Default::default()
                    })),
                },
            ]),
        );
        frame
    }

    fn with_context(
        frame: &DecodedFrame,
        fog: bool,
        nato: bool,
        recorded: Option<&str>,
        run: impl FnOnce(&Context<'_>),
    ) {
        let base = localisation();
        let loc: Localisation = base
            .iter()
            .map(|(key, text)| (key.to_owned(), text.to_owned()))
            .chain(
                [
                    ("AAA", "Alpha"),
                    ("BBB", "Beta"),
                    ("CCC", "Host country"),
                    // A recorded name_tag selects a key; let the refusal fixture reach text validation.
                    ("Broken\u{14}", "Broken\u{14}"),
                    ("AAA_DeckPlane_short", "Own plane"),
                    ("BBB_DeckPlane_short", "Foreign plane"),
                    ("AIRBASE_AIRPLANES_ESTIMATED", "$BASE$: $VALUE$"),
                    ("AIRWING_COUNTRY_HIDDEN", "$COUNTRY$: $AMOUNT$"),
                    ("NO_INTEL", "No intel"),
                    ("FUEL_TITLE", "Fuel"),
                    ("FUEL_DAILY_REQUIRED", "required $NUM$"),
                    (
                        "FUEL_DAILY_REQUIRED_AND_RECEIVED",
                        "required $REQUIRED$ received $RECEIVED$",
                    ),
                    ("FUEL_DAILY_REQUIRED_MAX", "maximum $NUM$"),
                    ("FUEL_DAILY_REQUIRED_MAX_DESC_NAVY", "maximum details"),
                    ("FUEL_DAILY", "$AMOUNT$/day"),
                ]
                .into_iter()
                .map(|(key, text)| (key.to_owned(), text.to_owned())),
            )
            .collect();
        let defines = installed_content::defines::Defines::new().unwrap();
        defines.load("synthetic.lua", b"NDefines={NIntel={COUNTRY_LEVEL_INTEL_MAXIMUMS={1,1,1,1},ARMY_TEMPLATE_UNIT_COUNT_INTEL_MIN=0.2,ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX=0.8,ARMY_TEMPLATE_UNIT_COUNT_INTEL_RANGE_AT_LOWEST_INTEL=1}}").unwrap();
        let mut subunits = installed_content::unit_leader::SubUnits::default();
        subunits.read(
            b"sub_units={DeckPlane={type=cas} EmptyDeck={type=naval_bomber}}",
            "synthetic.txt",
        );
        let units = super::super::units::Content::load(&defines, Ok(&subunits));
        let fog_content = crate::map::visibility::FogContent {
            topology: Default::default(),
            defines: crate::map::visibility::Defines {
                intel_maximums: [100_000; 4],
                radar_effect: 0,
                map_cutoffs: vec![],
                air_icon_threshold: 0,
                air_detail_thresholds: [0, 20_000, 50_000, 80_000],
                air_plane_fuzzy_range: 100_000,
                naval_detail_thresholds: [0; 5],
            },
        };
        let links = [(1001, 1), (2002, 2), (3003, 3)].into_iter().collect();
        let tags = [(1, "AAA".into()), (2, "BBB".into()), (3, "CCC".into())]
            .into_iter()
            .collect();
        let vision = crate::map::perspective::Vision::read(
            frame,
            &links,
            &tags,
            &crate::map::perspective::Choice {
                country: crate::map::perspective::Country::Tag("AAA".into()),
                fog,
            },
            Ok(&fog_content),
        )
        .unwrap();
        let names = installed_content::division_names::Database::default();
        run(&Context {
            nato,
            content: None,
            dig_in_factor: Err("unreached".into()),
            loc: &loc,
            recorded_language: recorded.unwrap_or(&loc.language),
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            supply_grace: Err("unreached".into()),
            comms_max_distance: Err("unreached".into()),
            frame,
            vision: &vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string()),
        });
    }

    fn render(frame: &DecodedFrame, fog: bool, nato: bool, recorded: Option<&str>) -> Tooltip {
        let mut out = Tooltip::default();
        with_context(frame, fog, nato, recorded, |context| {
            summary(context, BASE, &mut out)
        });
        out
    }

    fn registrations(frame: &DecodedFrame) -> Vec<AirBaseRegistrationRow> {
        frame
            .complete::<tables::AirBaseRegistrations>()
            .unwrap()
            .to_vec()
    }

    #[test]
    fn composed_carrier_keeps_dense_repeats_pool_owner_names_and_empty_fuel_pools() {
        let out = render(&fixture(), false, false, None);
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        let first = out.title.find("\u{12}BBB").unwrap();
        let own = out.title.find("\u{12}AAA").unwrap();
        let last = out.title.rfind("\u{12}BBB").unwrap();
        assert!(first < own && own < last);
        assert_eq!(out.title.matches("one Foreign plane carrier").count(), 2);
        assert_eq!(out.title.matches("one Own plane carrier").count(), 1);
        assert!(out.title.contains("\u{11}R7\u{11}!/\u{11}R6\u{11}!"));
        assert!(out.title.contains("required 24 received 72"));
        assert!(out.title.contains(" 0  \u{13}unit_EmptyDeck_icon_small|1"));
        assert!(out.title.contains("maximum 48"));
        assert_eq!(out.body, "maximum details");
    }

    #[test]
    fn sea_and_port_controls_share_composed_carrier_and_viewer_nato() {
        let frame = fixture();
        with_context(&frame, false, true, None, |context| {
            let sea = super::super::task_force_children::specific(
                context,
                &super::super::task_force::Hover {
                    control: super::super::task_force::Control::Carrier,
                    unit_hit: 0,
                    rect: [0.; 4],
                    hit: None,
                    authored: None,
                    supported: true,
                    ours: true,
                    coarse: false,
                    hidden: false,
                    members: vec![7].into(),
                },
            )
            .unwrap();
            let port = super::super::task_force_port::build(
                context,
                &super::super::task_force_port::Hover {
                    control: super::super::task_force_port::Control::Carrier,
                    unit_hit: 0,
                    rect: [0.; 4],
                    hit: None,
                    authored: None,
                    supported: true,
                    members: vec![7].into(),
                    metadata: super::super::task_force_port::Metadata {
                        country: 1,
                        visibility: 255,
                        enemy: Ok(false),
                        hold_fire: None,
                        idle: None,
                        no_regions: None,
                        pride: None,
                    },
                },
            );
            assert_eq!(sea.title, port.title);
            assert_eq!(sea.body, port.body);
            assert_eq!(sea.refused, port.refused);
            assert!(sea.refused.is_empty(), "{:?}", sea.refused);
            assert!(sea.title.contains("unit_DeckPlane_icon_small|2"));
            assert!(!sea.title.contains("unit_DeckPlane_icon_small|1"));
        });
    }

    #[test]
    fn current_fuel_failure_keeps_maximum_and_other_signed_groups() {
        let mut frame = fixture();
        let mut rows = registrations(&frame);
        let Recorded::Value(Some(inputs)) = &mut rows[0].carrier else {
            unreachable!()
        };
        inputs.pools.as_mut().unwrap()[0].current_fuel_raw = None;
        put(&mut frame, TableRows::AirBaseRegistrations(rows));
        let out = render(&frame, false, false, None);
        assert_eq!(out.refused.len(), 1, "{:?}", out.refused);
        assert!(out.title.contains("unit_DeckPlane_icon_small|1  \u{11}H24"));
        assert!(out.title.contains("unit_EmptyDeck_icon_small|1  \u{11}H0"));
        assert!(out.title.contains("maximum 48"));
        assert_eq!(out.body, "maximum details");
    }

    #[test]
    fn repeated_wings_and_equal_signed_fuel_groups_wrap_before_daily_scaling() {
        let mut frame = fixture();
        let mut wings = frame.complete::<tables::AirWings>().unwrap().to_vec();
        wings[0].planes = i32::MAX;
        put(&mut frame, TableRows::AirWings(wings));
        let mut rows = registrations(&frame);
        rows[0].wings.push(101);
        let Recorded::Value(Some(inputs)) = &mut rows[0].carrier else {
            unreachable!()
        };
        let pools = inputs.pools.as_mut().unwrap();
        pools[1].current_fuel_raw = Some(i64::MAX);
        pools[1].maximum_fuel_raw = Some(50_000);
        pools.push(CarrierAirPool {
            type_index: 2,
            subunit_index: 1,
            wing_start: 1,
            wing_count: 1,
            current_fuel_raw: Some(100_000),
            maximum_fuel_raw: Some(50_000),
        });
        put(&mut frame, TableRows::AirBaseRegistrations(rows));
        let out = render(&frame, false, false, None);
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        assert!(out.title.contains("\u{11}G4\u{11}!/\u{11}G6\u{11}!"));
        assert!(
            out.title
                .contains(" -2  \u{13}unit_DeckPlane_icon_small|1  \u{11}H23\u{11}!/day")
        );
        assert!(out.title.contains("maximum 24"));
        assert!(
            out.title.find("unit_EmptyDeck").unwrap() < out.title.find("unit_DeckPlane").unwrap()
        );
    }

    #[test]
    fn large_equal_key_group_uses_first_empty_definition_and_keeps_mixed_indices_and_signed_counts()
    {
        let mut frame = fixture();
        let mut wings = frame.complete::<tables::AirWings>().unwrap().to_vec();
        wings[0].planes = -3;
        put(&mut frame, TableRows::AirWings(wings));
        let mut rows = registrations(&frame);
        let Recorded::Value(Some(inputs)) = &mut rows[0].carrier else {
            unreachable!()
        };
        let mut pools = vec![
            CarrierAirPool {
                type_index: 2,
                subunit_index: 2,
                current_fuel_raw: Some(0),
                maximum_fuel_raw: Some(0),
                ..Default::default()
            },
            CarrierAirPool {
                type_index: 2,
                subunit_index: 1,
                wing_count: 1,
                current_fuel_raw: Some(100_000),
                maximum_fuel_raw: Some(200_000),
                ..Default::default()
            },
        ];
        for index in 0..33 {
            pools.push(CarrierAirPool {
                type_index: 2,
                subunit_index: 1 + index % 2,
                wing_start: 1,
                current_fuel_raw: Some(0),
                maximum_fuel_raw: Some(0),
                ..Default::default()
            });
        }
        // Separate authentic null definition: zero category/name, zero planes.
        pools.push(CarrierAirPool {
            type_index: 0,
            subunit_index: 0,
            wing_start: 1,
            current_fuel_raw: Some(0),
            maximum_fuel_raw: Some(0),
            ..Default::default()
        });
        inputs.pools = Some(pools);
        put(&mut frame, TableRows::AirBaseRegistrations(rows));
        let out = render(&frame, false, false, None);
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        assert!(out.title.contains("\u{11}G3\u{11}!/\u{11}G6\u{11}!"));
        assert!(
            out.title
                .contains(" -3  \u{13}unit_EmptyDeck_icon_small|1  \u{11}H24\u{11}!/day")
        );
        assert!(
            out.title
                .contains(" -3  \u{13}unit_EmptyDeck_icon_small|1  \u{11}H48\u{11}!/day")
        );
        assert!(
            out.title
                .contains(" 0  \u{13}unit__icon_small|1  \u{11}H0\u{11}!/day")
        );
        assert!(!out.title.contains("unit_DeckPlane_icon_small"));
        assert!(
            out.title.find("unit__icon_small").unwrap()
                < out.title.find("unit_EmptyDeck_icon_small").unwrap()
        );
    }

    #[test]
    fn maximum_failure_preserves_current_groups_and_other_maximum_groups() {
        let mut frame = fixture();
        let mut rows = registrations(&frame);
        let Recorded::Value(Some(inputs)) = &mut rows[0].carrier else {
            unreachable!()
        };
        inputs.pools.as_mut().unwrap()[1].maximum_fuel_raw = None;
        put(&mut frame, TableRows::AirBaseRegistrations(rows));
        let out = render(&frame, false, false, None);
        assert!(out.title.contains("unit_DeckPlane_icon_small|1  \u{11}H24"));
        assert_eq!(out.title.matches("unit_EmptyDeck_icon_small|1").count(), 2);
        assert!(!out.title.contains("maximum 48"));
        assert_eq!(out.body, "maximum details");
        assert_eq!(out.refused.len(), 2, "{:?}", out.refused);
    }

    #[test]
    fn empty_inventory_keeps_container_fuel_without_requiring_wings_or_subtype_content() {
        let mut frame = fixture();
        let mut rows = registrations(&frame);
        for row in &mut rows {
            row.wings.clear();
            let Recorded::Value(Some(inputs)) = &mut row.carrier else {
                unreachable!()
            };
            inputs.pools = Some(vec![]);
        }
        put(&mut frame, TableRows::AirBaseRegistrations(rows));
        frame.tables[TableId::AirWings.index()] = replay::frame::TableView::Unavailable {
            reason: "unreached empty inventory".into(),
        }
        .into();
        let out = render(&frame, false, false, None);
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        assert!(out.title.contains("required 24 received 72"));
        assert!(out.title.contains("maximum 0"));
        assert!(!out.title.contains('\u{12}'));
    }

    #[test]
    fn low_tier_uses_complete_clock_host_numeric_id_and_native_country_slots() {
        let frame = fixture();
        with_context(&frame, true, false, None, |context| {
            assert_eq!(context.vision.air_port_level(3), Some(1));
            assert_eq!(
                host(context, &frame.complete::<tables::AirBases>().unwrap()[0])
                    .unwrap()
                    .1,
                100
            );
            assert_eq!(seed(context, 2).unwrap(), (i32::MAX as u32).wrapping_add(2));
            let estimate = context
                .vision
                .air_count_range(3, 7, seed(context, 100).unwrap(), frame.hour)
                .unwrap();
            let out = render(&frame, true, false, None);
            assert!(out.refused.is_empty(), "{:?}", out.refused);
            assert!(
                out.title
                    .starts_with(&format!("Host: {}", count_text(estimate.0, estimate.1)))
            );
            assert_eq!(out.title.matches("Beta:").count(), 2);
            assert!(!out.title.contains("Fuel"));
        });
        let mut missing = frame;
        missing.tables[TableId::Clock.index()] = replay::frame::TableView::Unavailable {
            reason: "missing Clock witness".into(),
        }
        .into();
        let out = render(&missing, true, false, None);
        assert!(out.title.is_empty());
        assert!(out.refused.keys().any(|reason| reason.contains("Clock")));
    }

    #[test]
    fn absent_viewer_fuel_container_is_known_absence_in_a_complete_census() {
        let mut frame = fixture();
        let mut bases = frame.complete::<tables::AirBases>().unwrap().to_vec();
        let Recorded::Value(Some(inputs)) = &mut bases[0].carrier else {
            unreachable!()
        };
        inputs
            .registered_countries
            .as_mut()
            .unwrap()
            .retain(|entry| entry.slot != 1);
        put(&mut frame, TableRows::AirBases(bases));
        let mut rows = registrations(&frame);
        rows.retain(|row| row.slot != 1);
        put(&mut frame, TableRows::AirBaseRegistrations(rows));
        let out = render(&frame, false, false, None);
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        assert!(out.title.contains("Foreign plane"));
        assert!(!out.title.contains("Fuel"));
        assert!(out.body.is_empty());
        frame.tables[TableId::AirBaseRegistrations.index()] =
            replay::frame::TableView::Unavailable {
                reason: "unreadable fuel census".into(),
            }
            .into();
        let out = render(&frame, false, false, None);
        assert!(
            out.refused
                .keys()
                .any(|reason| reason.contains("carrier viewer fuel container"))
        );
    }

    #[test]
    fn no_intel_does_not_require_unreached_estimates_and_unknown_sharing_cannot_choose_a_tier() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::CountryIntel(vec![CountryIntelRow {
                owner_country: 1001,
                target_index: 3,
                air: 0,
                ..Default::default()
            }]),
        );
        frame.tables[TableId::Clock.index()] = replay::frame::TableView::Unavailable {
            reason: "unreached estimate".into(),
        }
        .into();
        let out = render(&frame, true, false, None);
        assert_eq!(out.title, "Host deck\n\n\nNo intel");
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        frame.tables[TableId::WarRelations.index()] = replay::frame::TableView::Unavailable {
            reason: "missing sharing witness".into(),
        }
        .into();
        let out = render(&frame, true, false, None);
        assert!(out.title.is_empty());
        assert!(
            out.refused
                .keys()
                .any(|reason| reason.contains("WarRelations"))
        );
    }

    #[test]
    fn historical_pool_absence_and_bad_ranges_do_not_erase_known_plane_totals_or_fuel_scalars() {
        for bad_range in [false, true] {
            let mut frame = fixture();
            let mut rows = registrations(&frame);
            if bad_range {
                let Recorded::Value(Some(inputs)) = &mut rows[0].carrier else {
                    unreachable!()
                };
                inputs.pools.as_mut().unwrap()[1].wing_start = 1;
            } else {
                rows[0].carrier = Recorded::NotRecorded;
            }
            put(&mut frame, TableRows::AirBaseRegistrations(rows));
            let out = render(&frame, false, false, None);
            assert!(out.title.contains("\u{11}R7\u{11}!"));
            assert_eq!(out.title.matches("one Foreign plane carrier").count(), 2);
            assert_eq!(out.body, "maximum details");
            if bad_range {
                assert!(out.title.contains("required 24 received 72"));
            }
            assert!(
                out.refused
                    .keys()
                    .any(|reason| reason.contains(if bad_range {
                        "partition"
                    } else {
                        "not recorded"
                    }))
            );
        }
    }

    #[test]
    fn host_language_failure_preserves_independent_occupancy_countries_and_fuel() {
        let out = render(&fixture(), false, false, Some("other_language"));
        assert!(!out.title.contains("Host deck"));
        assert!(out.title.contains("\u{11}R7\u{11}!"));
        assert!(out.title.contains("\u{12}BBB"));
        assert!(out.title.contains("required 24 received 72"));
        assert!(
            out.refused
                .keys()
                .any(|reason| reason.contains("other_language"))
        );
    }

    #[test]
    fn refused_country_fragment_keeps_ordered_known_siblings_at_both_intel_tiers() {
        let mut frame = fixture();
        let mut appearances = frame
            .complete::<tables::CountryAppearance>()
            .unwrap()
            .to_vec();
        appearances[1].name_tag = b"Broken\x14".to_vec();
        put(&mut frame, TableRows::CountryAppearance(appearances));
        for fog in [false, true] {
            let out = render(&frame, fog, false, None);
            assert!(out.title.contains("Alpha"));
            assert!(!out.title.contains("Broken"));
            assert_eq!(out.refused.values().sum::<usize>(), 2, "{:?}", out.refused);
            assert!(out.refused.keys().all(|reason| reason.contains("0x14")));
            if !fog {
                assert!(out.title.contains("required 24 received 72"));
            }
        }
    }

    #[test]
    fn null_stale_and_live_broken_hosts_keep_native_invalid_refusals() {
        for (host, task_force) in [(0, 7), (SHIP + 1, 7), (SHIP, 0)] {
            let mut frame = fixture();
            let mut bases = frame.complete::<tables::AirBases>().unwrap().to_vec();
            bases[0].host = host;
            put(&mut frame, TableRows::AirBases(bases));
            let mut ships = frame.complete::<tables::Ships>().unwrap().to_vec();
            ships[0].task_force = task_force;
            put(&mut frame, TableRows::Ships(ships));
            let out = render(&frame, false, false, None);
            assert!(out.title.is_empty());
            assert!(
                out.refused
                    .keys()
                    .any(|reason| reason.contains("native-invalid"))
            );
        }
    }

    #[test]
    fn subtype_database_index_distinguishes_null_unknown_and_registered_content() {
        with_context(&fixture(), false, false, None, |context| {
            let units = context.units.as_ref().unwrap();
            assert_eq!(units.subunit(0).unwrap(), ("", 0));
            assert_eq!(units.subunit(1).unwrap().0, "DeckPlane");
            assert!(units.subunit(999).unwrap_err().contains("index 999"));
        });
    }

    #[test]
    fn pool_owner_zero_uses_the_recorded_reserved_tag() {
        let mut frame = fixture();
        put(
            &mut frame,
            TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                ..Default::default()
            }]),
        );
        with_context(&frame, false, false, None, |context| {
            assert_eq!(slot(context, 0), Ok(0));
            assert_eq!(tag(context, 0), Ok("ZERO"));
            // Constructor-era display cache stays empty independently of tag.
            assert_eq!(context.country_name(0), Ok(String::new()));
        });
    }

    fn localisation() -> Localisation {
        [
            ("AIRWING_MISSION_WING_STATUS_IDLE", "idle"),
            ("AIRWING_MISSION_WING_STATUS_CARRIER_MISSION", "carrier"),
            ("ROCKET_WING_STATUS_EQUIPMENT_MISSION", "equipment"),
            ("AIRWING_MISSION_WING_STATUS_MISSION", "mission $MIS$"),
            ("AIRWING_MISSION_TYPE_TRAINING", "training"),
            ("AIRWING_MISSION_TYPE_CAS", "CAS"),
            ("AIRWING_MISSION_WINGS_STATUS_1", "one $NAME$ $STATUS$"),
            ("AIRWING_MISSION_WINGS_STATUS_2", "$NUM$ $NAME$ $STATUS$"),
            (
                "ROCKET_MISSION_WING_STATUS_1",
                "rocket $NUM$ $NAME$ $STATUS$",
            ),
            ("AIR_BASE_LEVEL_UNKNOWN", "$NAME$ deck"),
            ("AIRBASE_AIRPLANES_KNOWN", "$VALUE$/$CAPACITY$"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    #[test]
    fn carrier_groups_count_wings_and_sort_localised_name_status_pairs() {
        let loc = localisation();
        let wings = [
            AirWingRow {
                id: 1,
                planes: 1,
                mission_mask: 2,
                mission_active: 1,
                mission_region: Some(7),
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                planes: 70,
                mission_mask: 2,
                mission_active: 1,
                mission_region: Some(7),
                ..Default::default()
            },
            AirWingRow {
                id: 3,
                planes: 5,
                ..Default::default()
            },
            AirWingRow {
                id: 4,
                planes: 0,
                ..Default::default()
            },
            AirWingRow {
                id: 5,
                planes: -4,
                ..Default::default()
            },
            AirWingRow {
                id: 6,
                planes: 1,
                mission_mask: 0x800,
                ..Default::default()
            },
        ];
        let text = wing_groups(&loc, 0, true, &wings, |wing| {
            assert!(
                !matches!(wing.id, 4 | 5),
                "nonpositive wings never reach naming"
            );
            Ok(if wing.id == 3 { "Alpha" } else { "Zulu" }.into())
        })
        .unwrap();
        assert_eq!(
            text,
            "one Alpha carrier\n2 Zulu mission CAS\none Zulu mission training\n"
        );
    }

    #[test]
    fn combined_mission_masks_keep_native_empty_mission_and_no_region_uses_default() {
        let loc = localisation();
        let mut wing = AirWingRow {
            planes: 1,
            mission_mask: 3,
            mission_active: 1,
            mission_region: Some(8),
            ..Default::default()
        };
        assert_eq!(
            wing_groups(&loc, 0, false, [&wing], |_| Ok("Type".into())).unwrap(),
            "one Type mission \n"
        );
        wing.mission_region = None;
        assert_eq!(
            wing_groups(&loc, 0, false, [&wing], |_| Ok("Type".into())).unwrap(),
            "one Type idle\n"
        );
    }

    #[test]
    fn stateless_header_keeps_native_newlines_and_signed_capacity_colour() {
        let loc = localisation();
        assert_eq!(
            full_header(&loc, 0, "Carrier", 6, 5, "country\n").unwrap(),
            "Carrier deck\n\n\u{11}R6\u{11}!/\u{11}R5\u{11}!\ncountry\n\n"
        );
        assert_eq!(
            full_header(&loc, 0, "", -1, 0, "unreached").unwrap(),
            " deck\n\n\u{11}G-1\u{11}!/\u{11}G0\u{11}!\n\n"
        );
        assert!(
            full_header(&loc, 7, "", 0, 0, "")
                .unwrap_err()
                .contains("kind 7")
        );
    }

    #[test]
    fn fuel_numbers_keep_native_cutoffs_rounding_and_large_value_overflow() {
        assert_eq!(fuel_amount(49_999), "0");
        assert_eq!(fuel_amount(50_000), "0");
        assert_eq!(fuel_amount(-150_000), "-1");
        assert_eq!(fuel_amount(0x5f51db0 - 1), "999");
        assert_eq!(fuel_amount(0x5f51db0), "1.0K");
        assert_eq!(fuel_amount(0x1748291620), "1.000M");
        assert_eq!(fuel_amount(0xe8d15c4da0), "10.00M");
        assert_eq!(fuel_amount(i64::MAX), "0.00M");
    }

    #[test]
    fn fuel_header_uses_native_surplus_predicate_and_separate_delayed_description() {
        let loc: Localisation = [
            ("FUEL_TITLE", "Fuel"),
            ("FUEL_DAILY_REQUIRED", "required $NUM$"),
            (
                "FUEL_DAILY_REQUIRED_AND_RECEIVED",
                "required $REQUIRED$ received $RECEIVED$",
            ),
            ("FUEL_DAILY_REQUIRED_MAX", "maximum $NUM$"),
            ("FUEL_DAILY_REQUIRED_MAX_DESC_NAVY", "maximum details"),
        ]
        .into_iter()
        .map(|(key, text)| (key.into(), text.into()))
        .collect();
        assert_eq!(
            fuel_header(&loc, 100_000, 200_000).unwrap(),
            "Fuel\nrequired 48\n"
        );
        assert_eq!(
            fuel_header(&loc, 300_000, 200_000).unwrap(),
            "Fuel\nrequired 48 received 72\n"
        );
        assert_eq!(maximum_header(&loc, 200_000).unwrap(), "maximum 48\n");
        assert_eq!(
            localise(&loc, "FUEL_DAILY_REQUIRED_MAX_DESC_NAVY", &[]).unwrap(),
            "maximum details"
        );
    }

    #[test]
    fn unknown_count_is_not_rendered_as_known_zero_or_as_a_range() {
        assert_eq!(count_text([0, 0], false), "?");
        assert_eq!(count_text([5, 9], false), "?");
        assert_eq!(count_text([0, 0], true), "0");
        assert_eq!(count_text([-3, 7], true), "-3-7");
    }

    #[test]
    fn fuel_pool_lines_keep_empty_signed_totals_and_viewer_nato_frame() {
        let loc: Localisation = [("FUEL_DAILY".into(), "$AMOUNT$/day".into())]
            .into_iter()
            .collect();
        assert_eq!(
            fuel_line(&loc, 0, "DeckPlane", 0, false, 0).unwrap(),
            " 0  \u{13}unit_DeckPlane_icon_small|1  \u{11}H0\u{11}!/day\n",
        );
        assert_eq!(
            fuel_line(&loc, -3, "SupportHull", 1, true, -100_000).unwrap(),
            " -3  \u{13}ship_SupportHull_icon_small|2  \u{11}H-24\u{11}!/day\n",
        );
    }

    #[test]
    fn low_headers_keep_base_argument_question_mark_and_country_spacing() {
        let loc: Localisation = [
            ("AIR_BASE_LEVEL_UNKNOWN", "$NAME$ deck"),
            ("AIRBASE_AIRPLANES_ESTIMATED", "$BASE$: $VALUE$"),
            ("NO_INTEL", "No intel"),
        ]
        .into_iter()
        .map(|(key, text)| (key.into(), text.into()))
        .collect();
        assert_eq!(
            low_header(&loc, 0, "Host", None, "").unwrap(),
            "Host deck\n\n\nNo intel"
        );
        assert_eq!(
            low_header(&loc, 0, "Host", Some(([0, 0], false)), "").unwrap(),
            "Host: ?\n\n\n"
        );
        assert_eq!(
            low_header(&loc, 0, "Host", Some(([2, 5], true)), "country\n").unwrap(),
            "Host: 2-5\n\n\n\n\ncountry\n",
        );
        assert_eq!(
            full_country_header("SYN", "Country"),
            "\u{12}SYN\u{11}HCountry\u{11}!:\n"
        );
    }
}
