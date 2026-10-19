//! Navy mode's region control, access and supremacy explanation.
use super::{Arg, Context, Tooltip};
use replay_data::{rows::NavalSupremacyRow, rows::tables};
use viewer_foundation::map::naval_region_control::Snapshot;
use viewer_foundation::tooltip_text::ensure_newlines;

#[derive(Clone)]
pub(super) struct Defines {
    pub sources: Result<replay_derive::naval_dominance_derive::Defines, String>,
    pub ratio: Result<i64, String>,
    pub gain: Result<i64, String>,
    pub loss: Result<i64, String>,
    pub convoy: Result<i64, String>,
    pub spotting: Result<i64, String>,
    pub recovery: Result<i64, String>,
    pub ship_recovery: Result<i64, String>,
    pub mine_max: Result<i64, String>,
    pub mine_intel_low: Result<i64, String>,
    pub mine_intel_high: Result<i64, String>,
    pub mine_intel_factor: Result<i64, String>,
}
impl Default for Defines {
    fn default() -> Self {
        let refused = || Err("naval regional content is unavailable".into());
        Self {
            sources: Err("naval dominance content is unavailable".into()),
            ratio: refused(),
            gain: refused(),
            loss: refused(),
            convoy: refused(),
            spotting: refused(),
            recovery: refused(),
            ship_recovery: refused(),
            mine_max: refused(),
            mine_intel_low: refused(),
            mine_intel_high: refused(),
            mine_intel_factor: refused(),
        }
    }
}
impl Defines {
    pub(super) fn load(d: &installed_content_core::defines::Defines) -> Self {
        let fixed = |section, key| d.fixed5(section, key).map_err(|e| e.to_string());
        Self {
            sources: replay_derive::naval_dominance_derive::Defines::read(|section, key| {
                fixed(section, key)
            }),
            ratio: fixed("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO"),
            gain: fixed("NNavy", "DOMINANCE_DAILY_GAIN_FACTOR"),
            loss: fixed("NNavy", "DOMINANCE_DAILY_LOSS_FACTOR"),
            convoy: fixed("NCountry", "CONVOY_CONTROLLED_ROUTE_COST_REDUCTION_FACTOR"),
            spotting: fixed("NNavy", "NAVAL_DOMINANCE_SPOTTING_BONUS"),
            recovery: fixed("NNavy", "NAVAL_DOMINANCE_ORG_RECOVERY"),
            ship_recovery: fixed("NNavy", "NAVAL_DOMINANCE_SHIP_RECOVERY_CHANCE"),
            mine_max: fixed("NNavy", "NAVAL_MINES_IN_REGION_MAX"),
            mine_intel_low: fixed(
                "NIntel",
                "NAVY_INTEL_MINE_DAMAGE_REDUCTION_FACTOR_MIN_INTEL_FOR_BONUS",
            ),
            mine_intel_high: fixed(
                "NIntel",
                "NAVY_INTEL_MINE_DAMAGE_REDUCTION_FACTOR_MAX_INTEL_FOR_BONUS",
            ),
            mine_intel_factor: fixed("NNavy", "NAVAL_MINES_INTEL_DIFF_FACTOR"),
        }
    }
}

pub(super) fn append(
    context: &Context<'_>,
    sources: &viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap,
    out: &mut Tooltip,
) {
    let Some(&region) = context.content.region_of.get(&context.province.id) else {
        return;
    };
    merge_sections(
        context,
        out,
        super::naval_mines::build(context.content, context.frame, context.vision, region, true),
        1,
    );
    access_section(context, out, access(context, region));
    let region_content = match context.content.strategic_regions.get(&region) {
        Some(region) => region,
        None => {
            out.refuse(
                "naval regional statistics",
                "strategic region content is unavailable".into(),
            );
            return;
        }
    };
    if !region_content.provinces.iter().any(|province| {
        context.content.kinds.get(province) == Some(&installed_content_core::map::ProvinceKind::Sea)
    }) {
        return;
    }
    let terrain = (|| -> Result<_, String> {
        let terrain = context
            .content
            .movement_terrain
            .as_ref()
            .map_err(Clone::clone)?;
        let index = region_content
            .naval_terrain
            .as_ref()
            .and_then(|name| terrain.category(name.as_bytes()))
            .or_else(|| terrain.category(b"ocean"))
            .ok_or("naval terrain and ocean fallback are absent from recorded content")?;
        Ok(&terrain.categories[index])
    })();
    let terrain = match terrain {
        Ok(terrain) => terrain,
        Err(reason) => {
            out.refuse("naval regional terrain", reason);
            return;
        }
    };
    section(
        context,
        out,
        "naval regional terrain",
        context
            .loc(std::str::from_utf8(&terrain.name).unwrap_or(""), &[])
            .map(|name| format!("\u{11}H{name}\u{11}!\n")),
    );
    let defines = &context.content.province_hover.navy;
    if terrain.minimum_seazone_dominance > 0 {
        out.line(
            false,
            0,
            "naval minimum dominance",
            defines.ratio.clone().and_then(|ratio| {
                localized_line(
                    context,
                    "MINIMUM_SEAZONE_DOMINANCE_NEEDED",
                    &[(
                        "VALUE",
                        Arg::Fixed5(minimum_text(terrain.minimum_seazone_dominance, ratio)),
                    )],
                )
            }),
        );
    }
    let snapshot = (|| {
        let snapshot = Snapshot::read(
            context.frame,
            context.vision.ours(),
            region,
            terrain.minimum_seazone_dominance,
            &context.links,
        )?;
        let control =
            snapshot.control(terrain.minimum_seazone_dominance, defines.ratio.clone()?)?;
        Ok::<_, String>((snapshot, control))
    })();
    match snapshot {
        Ok((snapshot, control)) => {
            section(
                context,
                out,
                "naval regional control",
                localized_line(
                    context,
                    match control {
                        1 => "NAVAL_DOMINANCE_FRIENDLY_CONTROLLED",
                        2 => "NAVAL_DOMINANCE_ENEMY_CONTROLLED",
                        _ => "NAVAL_DOMINANCE_CONTESTED",
                    },
                    &[],
                ),
            );
            let row = snapshot
                .rows
                .iter()
                .find(|row| context.slot(row.country) == Ok(context.vision.ours()))
                .copied()
                .cloned()
                .unwrap_or_default();
            out.line(
                false,
                1,
                "naval current dominance",
                snapshot.share().and_then(|share| {
                    current_text(&row, share, defines, |key, args| context.loc(key, args))
                }),
            );
            out.line(
                false,
                1,
                "naval building source explanation",
                building_text(context, region, &row),
            );
            out.line(
                false,
                1,
                "naval regional source explanation",
                source_text(context, region, sources),
            );
            out.title.push('\n');
            section(
                context,
                out,
                "naval convoy reduction",
                defines.convoy.clone().and_then(|value| {
                    localized_line(
                        context,
                        "CONVOY_DOMINANCE_COST_REDUCTION_EXPLANATION",
                        &[("REDUCTION", Arg::Fixed5(value))],
                    )
                }),
            );
            if control == 1 {
                out.line(
                    false,
                    1,
                    "naval dominance benefits",
                    localized_line(context, "NAVAL_DOMINANCE_BENEFITS", &[]),
                );
                for (key, value) in [
                    (
                        "SPOTTING_CHANCE_FOR_NAVAL_DOMINANCE",
                        defines.spotting.clone(),
                    ),
                    ("ORG_RECOVERY_FOR_NAVAL_DOMINANCE", defines.recovery.clone()),
                    (
                        "SHIP_RECOVERY_CHANCE_FOR_NAVAL_DOMINANCE",
                        defines.ship_recovery.clone(),
                    ),
                ] {
                    out.line(
                        false,
                        1,
                        "naval dominance benefit",
                        value.and_then(|value| {
                            localized_line(context, key, &[("VAL", Arg::Fixed5(value))])
                        }),
                    );
                }
                out.title.push('\n');
            }
        }
        Err(reason) => out.refuse("naval regional dominance", reason),
    }
}

/// The region's stored state vector is built by unique state identity from
/// member provinces. Building status rows preserve the active aggregate
/// vector and its signed total levels, including provincial building types.
fn building_counts(
    context: &Context<'_>,
    region: u32,
) -> Result<replay_derive::naval_dominance_derive::BuildingCounts, String> {
    use replay_derive::naval_dominance_derive::BuildingCounts;
    let country = context
        .frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.canonical_slot == context.vision.ours())
        .ok_or("naval building viewer country is unavailable")?;
    if country.id as i32 <= 0 {
        return Ok(BuildingCounts::default());
    }
    let definition = context
        .content
        .strategic_regions
        .get(&region)
        .ok_or("naval building region content is unavailable")?;
    let provinces = context
        .frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?;
    let mut states = std::collections::BTreeSet::new();
    for &province in &definition.provinces {
        let row = provinces
            .binary_search_by_key(&province, |row| row.id)
            .ok()
            .map(|index| &provinces[index])
            .ok_or("naval building province state is unavailable")?;
        if row.state != 0 {
            states.insert(row.state);
        }
    }
    let tokens = viewer_foundation::table_access::TokenNames::new(context.frame)
        .map_err(|e| e.to_string())?;
    let facts = &context.content.building_facts;
    let mut count = BuildingCounts::default();
    let buildings = context
        .frame
        .complete::<tables::StateBuildings>()
        .map_err(|e| e.to_string())?;
    for state in states {
        let first = buildings.partition_point(|row| row.location < state);
        let end = first + buildings[first..].partition_point(|row| row.location == state);
        for building in &buildings[first..end] {
            if !building.active || context.slot(building.country)? != context.vision.ours() {
                continue;
            }
            let token = tokens
                .name(building.building_type)
                .map_err(|e| e.to_string())?;
            let key = facts
                .keys
                .resolve(token)?
                .ok_or("naval dominance building type is absent from recorded content")?;
            let kind = facts
                .index
                .get(key)
                .and_then(|&index| facts.ordered.get(index))
                .ok_or("naval dominance building type definition is unavailable")?;
            let level = i32::from(building.level);
            if kind.is_port {
                count.naval_base = count.naval_base.wrapping_add(level);
            }
            if kind.air_base {
                count.air_base = count.air_base.wrapping_add(level);
            }
            if kind.radar {
                count.radar = count.radar.wrapping_add(level);
            }
        }
    }
    Ok(count)
}

fn building_text(
    context: &Context<'_>,
    region: u32,
    row: &NavalSupremacyRow,
) -> Result<String, String> {
    let counts = building_counts(context, region)?;
    let defines = *context
        .content
        .province_hover
        .navy
        .sources
        .as_ref()
        .map_err(Clone::clone)?;
    let modifiers = counts.modifiers(defines);
    let factor = (modifiers[0]
        .wrapping_add(100_000)
        .wrapping_mul(modifiers[1].wrapping_add(100_000))
        / 100_000)
        .wrapping_mul(modifiers[2].wrapping_add(100_000))
        / 100_000;
    let mut text = String::new();
    if row.target > 0 && factor > 0 {
        for (count, modifier, key, arg) in [
            (
                counts.naval_base,
                modifiers[0],
                "NAVAL_DOMINANCE_NAVALBASE_DESC",
                "NAVALBASE",
            ),
            (
                counts.air_base,
                modifiers[1],
                "NAVAL_DOMINANCE_AIRBASE_DESC",
                "AIRBASE",
            ),
            (
                counts.radar,
                modifiers[2],
                "NAVAL_DOMINANCE_RADAR_DESC",
                "RADAR",
            ),
        ] {
            if modifier > 0 {
                text.push_str(&context.loc(
                    key,
                    &[
                        ("PERCENT", Arg::Fixed5(modifier)),
                        (
                            "VALUE",
                            Arg::Fixed5(row.base_target.wrapping_mul(modifier) / 100_000),
                        ),
                        (arg, Arg::Int32(count)),
                    ],
                )?);
                ensure_newlines(&mut text, 1);
            }
        }
    }
    Ok(text)
}

fn source_text(
    context: &Context<'_>,
    region: u32,
    sources: &viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap,
) -> Result<String, String> {
    let source = viewer_foundation::map::naval_dominance_sources::region(sources, region)?;
    let mut text = String::new();
    if source.strike_multiplier > 100_000 && source.patrol > 0 {
        let percent = source.strike_multiplier.wrapping_sub(100_000);
        text.push_str(&context.loc(
            "NAVAL_DOMINANCE_STRIKEFORCE_DESC",
            &[
                ("PERCENT", Arg::Fixed5(percent)),
                (
                    "VALUE",
                    Arg::Fixed5(source.patrol.wrapping_mul(percent) / 100_000),
                ),
            ],
        )?);
        ensure_newlines(&mut text, 1);
    }
    if source.enemy_raiding > 0 {
        text.push_str(&context.loc(
            "NAVAL_DOMINANCE_CONVOY_RAIDING_DESC",
            &[("VALUE", Arg::Fixed5(source.enemy_raiding.wrapping_neg()))],
        )?);
        ensure_newlines(&mut text, 1);
    }
    Ok(text)
}

fn access(context: &Context<'_>, region: u32) -> Result<String, String> {
    let rows = context
        .frame
        .complete::<tables::NavalRegionAccess>()
        .map_err(|e| e.to_string())?;
    let mode = rows
        .iter()
        .find(|row| row.region == region && context.slot(row.country) == Ok(context.vision.ours()))
        .map_or(0, |row| row.mode);
    let (level, description) = match mode {
        0 => return Ok(String::new()),
        1 => (
            "NAVAL_REGION_ACCESS_AVOID",
            "NAVAL_REGION_ACCESS_AVOID_DESC",
        ),
        2 => (
            "NAVAL_REGION_ACCESS_BLOCKED",
            "NAVAL_REGION_ACCESS_BLOCKED_DESC",
        ),
        _ => return Ok(String::new()),
    };
    let level = context.loc(level, &[])?;
    let mut text = context.loc("NAVAL_REGION_ACCESS_LEVEL", &[("LVL", Arg::Text(&level))])?;
    ensure_newlines(&mut text, 1);
    text.push_str(&context.loc(description, &[])?);
    Ok(text)
}

fn minimum_text(minimum: i64, ratio: i64) -> i64 {
    let value = viewer_foundation::naval_math::dominance_value(minimum, ratio);
    i64::from(
        value
            .wrapping_add(if value < 0 { -50_000 } else { 50_000 })
            .wrapping_div(100_000) as i32,
    )
    .wrapping_mul(100_000)
}

fn current_text(
    row: &NavalSupremacyRow,
    share: i64,
    defines: &Defines,
    mut loc: impl FnMut(&str, &[(&str, Arg<'_>)]) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = loc(
        "NAVAL_DOMINANCE_CURRENT_VALUE",
        &[
            ("CURRENT", Arg::Fixed5(row.current)),
            ("RATIO", Arg::Fixed5(share)),
        ],
    )?;
    text.push_str("\n\n");
    if row.current != row.target {
        let increasing = row.current < row.target;
        let divisor = if increasing {
            row.target.wrapping_mul(defines.gain.clone()?) / 100_000
        } else {
            row.decline_from.wrapping_mul(defines.loss.clone()?) / 100_000
        };
        let difference = if increasing {
            row.target.wrapping_sub(row.current)
        } else {
            row.current.wrapping_sub(row.target)
        };
        let days = if divisor == 0 {
            0xffff_ffff
        } else {
            viewer_foundation::map::naval_region_control::quotient(difference, divisor)?
        };
        if days > 0 {
            text.push_str(&loc(
                if increasing {
                    "NAVAL_DOMINANCE_TARGET_VALUE_INCREASING"
                } else {
                    "NAVAL_DOMINANCE_TARGET_VALUE_DECREASING"
                },
                &[
                    ("TARGET", Arg::Fixed5(row.target)),
                    ("DAYS", Arg::Fixed5(days)),
                ],
            )?);
        }
    }
    ensure_newlines(&mut text, 1);
    text.push_str(&loc(
        "NAVAL_DOMINANCE_BASE",
        &[("VALUE", Arg::Fixed5(row.base_target))],
    )?);
    Ok(text)
}

fn localized_line(
    context: &Context<'_>,
    key: &str,
    args: &[(&str, Arg<'_>)],
) -> Result<String, String> {
    context.loc(key, args).map(|text| format!("{text}\n"))
}
fn access_section(context: &Context<'_>, out: &mut Tooltip, value: Result<String, String>) {
    match value {
        Ok(value) if !value.is_empty() => {
            separation(context, out);
            ensure_newlines(&mut out.title, 1);
            out.title.push_str(&value);
        }
        Ok(_) => {}
        Err(reason) => out.refuse("naval region access", reason),
    }
}

fn separation(context: &Context<'_>, out: &mut Tooltip) {
    if out.title.is_empty() {
        return;
    }
    if !out.title.trim_end_matches(' ').ends_with('\n') {
        out.title.push('\n');
    }
    match context.loc("SEPARATION_LINE", &[]) {
        Ok(text) => out.title.push_str(&text),
        Err(reason) => out.refuse("regional section separator", reason),
    }
}
fn section(context: &Context<'_>, out: &mut Tooltip, name: &str, value: Result<String, String>) {
    match value {
        Ok(value) if !value.is_empty() => {
            separation(context, out);
            out.title.push_str(&value);
        }
        Ok(_) => {}
        Err(reason) => out.refuse(name, reason),
    }
}
pub(super) fn merge_sections(
    context: &Context<'_>,
    out: &mut Tooltip,
    value: Tooltip,
    _gap: usize,
) {
    if !value.title.is_empty() {
        separation(context, out);
        out.title.push_str(&value.title);
    }
    for (reason, count) in value.refused {
        *out.refused.entry(reason).or_default() += count;
    }
}

#[cfg(test)]
pub(super) fn merge(out: &mut Tooltip, value: Tooltip, gap: usize) {
    if !value.title.is_empty() {
        ensure_newlines(&mut out.title, gap);
        out.title.push_str(&value.title);
    }
    for (reason, count) in value.refused {
        *out.refused.entry(reason).or_default() += count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(row: NavalSupremacyRow) -> String {
        let loc: installed_content_localisation::localisation::Localisation = [
            (
                "NAVAL_DOMINANCE_CURRENT_VALUE",
                "current $CURRENT|5$ share $RATIO|5$",
            ),
            (
                "NAVAL_DOMINANCE_TARGET_VALUE_INCREASING",
                "grow $TARGET|5$ days $DAYS|5$",
            ),
            (
                "NAVAL_DOMINANCE_TARGET_VALUE_DECREASING",
                "fall $TARGET|5$ days $DAYS|5$",
            ),
            ("NAVAL_DOMINANCE_BASE", "base $VALUE|5$"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        current_text(
            &row,
            33_333,
            &Defines {
                gain: Ok(2_000),
                loss: Ok(4_000),
                ..Default::default()
            },
            |key, args| {
                loc.localise_with(key.as_bytes(), args)
                    .map_err(|e| e.to_string())
            },
        )
        .unwrap()
    }
    #[test]
    fn target_eta_uses_target_to_gain_and_decline_origin_to_lose() {
        assert!(
            text(NavalSupremacyRow {
                current: 50_000,
                target: 100_000,
                base_target: 7,
                ..Default::default()
            })
            .contains("grow 1.00000 days 25.00000")
        );
        assert!(
            text(NavalSupremacyRow {
                current: 100_000,
                target: 50_000,
                decline_from: 200_000,
                ..Default::default()
            })
            .contains("fall 0.50000 days 6.25000")
        );
    }
    #[test]
    fn equal_targets_do_not_demand_unused_cadence_defines_and_gap_merge_keeps_refusals() {
        let row = NavalSupremacyRow::default();
        assert!(current_text(&row, 0, &Defines::default(), |key, _| Ok(key.into())).is_ok());
        let mut out = Tooltip {
            title: "first".into(),
            ..Default::default()
        };
        let mut second = Tooltip {
            title: "next".into(),
            ..Default::default()
        };
        second.refuse("exact operand", "missing".into());
        merge(&mut out, second, 2);
        assert_eq!(out.title, "first\n\nnext");
        assert_eq!(out.refused["exact operand: missing"], 1);
    }
}
