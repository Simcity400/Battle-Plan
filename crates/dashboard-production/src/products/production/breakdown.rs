use dashboard_world::products::equipment_names::{self as names, Named};

use dashboard_base::products::{Country, validate_country};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget}};
use engine_executable::modifier_keys as keys;
use dashboard_query::identity::Holder;
use dashboard_query::read::row;
use replay_data::{fixed_point::fixed5_to_q15, fixed_point::q15_to_fixed5};
use replay_data::{
    rows::CountryProductionModifiersRow, rows::ProductionLineModifiersRow, rows::ProductionModifierTree,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub type RawRun = replay_data::rows::ProductionLineRow;

const EFFICIENCY_NOTE: &str = "The native average of the admitted factory caches. Current source parameters below do not identify the causes of all past growth or switches.";
const OUTPUT_NOTE: &str = "Parts include factory output, conversion and refit bonuses where applicable. Other / not identified reconciles licence, rounding or earlier retained state with the recorded IC rate.";

pub(super) const FACTORS: &[(&str, &str)] = &[
    ("efficiency", "Stored factory efficiency"),
    ("start", "Starting efficiency"),
    ("cap", "Efficiency cap"),
    ("gain", "Efficiency growth coefficient"),
    ("daily", "Native daily efficiency increase"),
    ("retention-0", "Variant switch retention"),
    ("retention-1", "Parent switch retention"),
    ("retention-2", "Family switch retention"),
    ("retention-3", "Archetype switch retention"),
    ("output", "Recorded daily production IC"),
    ("power", "Effective powered-factory fraction"),
    ("resources", "Resource output factor"),
    ("conversion", "Conversion speed parameter"),
    ("refit", "Refit speed parameter"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawModifierRun {
    pub country: Arc<CountryProductionModifiersRow>,
    pub line: ProductionLineModifiersRow,
    pub run: RawRun,
    pub organisation: Option<String>,
    pub localisation: Option<Arc<content_reader_localisation::localisation::Localisation>>,
    pub recorded_language: Arc<str>,
}
impl RawModifierRun {
    pub fn project(&self, selected: Option<&str>) -> ProductionModifierRun {
        at(self, selected)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionModifierPart {
    pub id: String,
    pub label: Option<String>,
    pub raw: String,
    pub unknown: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionModifierGroup {
    pub id: String,
    pub label: String,
    #[doc = " Exact sum of the displayed signed parts; null when an operand failed."]
    pub total_raw: Option<String>,
    pub decimals: u8,
    pub unit: String,
    pub parts: Vec<ProductionModifierPart>,
    pub note: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionModifierRun {
    pub groups: Vec<ProductionModifierGroup>,
    pub warnings: Vec<String>,
}

fn part(id: impl Into<String>, label: impl Into<String>, raw: i64) -> ProductionModifierPart {
    ProductionModifierPart {
        id: id.into(),
        label: Some(label.into()),
        raw: raw.to_string(),
        unknown: None,
    }
}
fn residual(parts: &mut Vec<ProductionModifierPart>, total: i64, reason: &str) {
    let sum = parts
        .iter()
        .map(|part| {
            part.raw
                .parse::<i128>()
                .expect("native integer or presentation residual")
        })
        .sum::<i128>();
    let value = i128::from(total) - sum;
    if value != 0 {
        parts.push(ProductionModifierPart {
            id: "other".into(),
            label: None,
            raw: value.to_string(),
            unknown: Some(reason.into()),
        });
    }
}
fn group(
    id: &str,
    total: Option<i64>,
    mut parts: Vec<ProductionModifierPart>,
    unit: &str,
    note: &str,
) -> ProductionModifierGroup {
    if let Some(total) = total {
        residual(
            &mut parts,
            total,
            "Native clamp, rounding, retained state or an unidentified contribution",
        );
    }
    let label = FACTORS
        .iter()
        .find(|(factor, _)| *factor == id)
        .expect("production factor")
        .1;
    ProductionModifierGroup {
        id: id.into(),
        label: label.into(),
        total_raw: total.map(|value| value.to_string()),
        decimals: 5,
        unit: unit.into(),
        parts,
        note: note.into(),
    }
}
fn tree(country: &CountryProductionModifiersRow, key: u32) -> Option<&ProductionModifierTree> {
    country
        .trees
        .binary_search_by_key(&key, |tree| tree.key)
        .ok()
        .map(|index| &country.trees[index])
}
fn value(country: &CountryProductionModifiersRow, key: u32) -> Option<i64> {
    let tree = tree(country, key)?;
    tree.failure
        .is_empty()
        .then(|| tree.nodes.first().map(|node| node.value))
        .flatten()
}
fn subtree_end(nodes: &[replay_data::rows::ModifierBreakdownNode], start: usize) -> usize {
    let mut pending = 1_usize;
    let mut end = start;
    while pending != 0 && end < nodes.len() {
        pending = pending - 1 + nodes[end].children as usize;
        end += 1;
    }
    end
}
fn sources(
    country: &CountryProductionModifiersRow,
    key: u32,
    depth: i32,
    scale: impl Fn(i64) -> i64,
    localisation: Option<&content_reader_localisation::localisation::Localisation>,
    recorded_language: &str,
    warnings: &mut BTreeSet<String>,
) -> Vec<ProductionModifierPart> {
    let Some(tree) = tree(country, key) else {
        warnings.insert(format!("Modifier key {key:#x} was not recorded"));
        return Vec::new();
    };
    if !tree.failure.is_empty() {
        warnings.insert(format!(
            "Modifier key {key:#x}: {}",
            String::from_utf8_lossy(&tree.failure)
        ));
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut occurrences = BTreeMap::<Vec<u8>, usize>::new();
    let mut pending = vec![(0_usize, depth)];
    while let Some((index, depth)) = pending.pop() {
        let Some(node) = tree.nodes.get(index) else {
            continue;
        };
        if depth == 0 || node.children == 0 {
            let occurrence = occurrences.entry(node.name.clone()).or_default();
            *occurrence += 1;
            let occurrence = *occurrence;
            if node.value == 0 {
                continue;
            }
            let label = localisation
                .ok_or_else(|| "Native source label has no localisation context".to_owned())
                .and_then(|localisation| {
                    dashboard_text::scripted_text::stored_text(
                        localisation,
                        recorded_language,
                        &node.name,
                        "production modifier source",
                    )
                })
                .and_then(|text| dashboard_text::catalogue::display_text(text.as_bytes()))
                .and_then(|label| {
                    if label.is_empty() || label.contains('$') || label.contains('[') {
                        Err("Native source label is empty or needs scripted expansion".to_owned())
                    } else {
                        Ok(label)
                    }
                });
            let (label, unknown) = match label {
                Ok(label) => (Some(label), None),
                Err(reason) => (None, Some(reason)),
            };
            if let Some(reason) = &unknown {
                warnings.insert(format!("Modifier key {key:#x}, source {index}: {reason}"));
            }
            let identity = std::str::from_utf8(&node.name)
                .map(|name| format!("text:{name}"))
                .unwrap_or_else(|_| format!("bytes:{:?}", node.name));
            result.push(ProductionModifierPart {
                id: format!("{key:x}/{identity}#{occurrence}"),
                label,
                raw: scale(node.value).to_string(),
                unknown,
            });
        } else {
            let mut children = Vec::with_capacity(node.children as usize);
            let mut child = index + 1;
            for _ in 0..node.children {
                children.push((child, depth.wrapping_sub(1)));
                child = subtree_end(&tree.nodes, child);
            }
            pending.extend(children.into_iter().rev());
        }
    }
    result
}
fn percent(value: i64) -> i64 {
    value.wrapping_mul(100)
}
fn multiply(a: i64, b: i64, denominator: i64) -> Option<i64> {
    a.wrapping_mul(b).checked_div(denominator)
}

fn factory_gain(current: i64, cap: i64, gain: i64, balance: i64) -> Option<i64> {
    if balance <= 0 || current == 0 {
        return Some(0);
    }
    let numerator = ((cap.wrapping_mul(cap) >> 15).wrapping_mul(gain)) >> 15;
    let denominator = ((current.wrapping_mul(0x13880000) >> 15).wrapping_mul(balance)) >> 15;
    numerator.wrapping_shl(15).checked_div(denominator)
}
fn daily_gain(
    line: &ProductionLineModifiersRow,
    run: &RawRun,
    start: i64,
    cap: i64,
    gain: i64,
    balance: i64,
) -> Option<i64> {
    let count = run.active_factories.min(run.max_factories).max(0) as usize;
    if line.factory_efficiencies.len() < count {
        return None;
    }
    let mut sum = 0_i64;
    for &current in &line.factory_efficiencies[..count] {
        let next = current.wrapping_add(factory_gain(current, cap, gain, balance)?);
        let next = if next < start {
            start
        } else if next > cap {
            cap
        } else {
            next
        };
        sum = sum.wrapping_add(next.wrapping_sub(current));
    }
    if run.active_factories == 0 {
        Some(0)
    } else {
        sum.wrapping_shl(15)
            .checked_div(i64::from(run.active_factories).wrapping_shl(15))
    }
}
fn factory_speed(
    country: &CountryProductionModifiersRow,
    power_modifier: i64,
    naval: bool,
) -> Option<i64> {
    let power = multiply(
        power_modifier.wrapping_add(100000),
        country.power_raw,
        100000,
    )?
    .max(0);
    let [unpowered, powered] = if naval {
        [country.factory_speed[2], country.factory_speed[3]]
    } else {
        [country.factory_speed[0], country.factory_speed[1]]
    };
    Some(unpowered.wrapping_add(multiply(power, powered.wrapping_sub(unpowered), 100000)?))
}
pub fn factory_output(country: &CountryProductionModifiersRow, naval: bool) -> Option<i64> {
    let power_modifier = value(country, keys::OUT_OF_POWER_IMPACT_FACTOR)?;
    let base = factory_speed(country, power_modifier, naval)?;
    let (output, powered) = if naval {
        (
            keys::INDUSTRIAL_CAPACITY_DOCKYARD,
            keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED,
        )
    } else {
        (
            keys::INDUSTRIAL_CAPACITY_FACTORY,
            keys::INDUSTRIAL_CAPACITY_FACTORY_POWERED,
        )
    };
    let modifier = value(country, output)?;
    let mut total = base.wrapping_add(multiply(base, modifier, 100000)?);
    if country.power_raw > 99999 {
        total = total.wrapping_add(multiply(base, value(country, powered)?, 100000)?);
    } else if modifier > 0 {
        let power = multiply(
            power_modifier.wrapping_add(100000),
            country.power_raw,
            100000,
        )?
        .max(0);
        let adjustment = multiply(
            100000_i64.wrapping_sub(power),
            modifier.wrapping_neg(),
            100000,
        )?;
        total = total.wrapping_add(multiply(base, adjustment, 100000)?);
    }
    Some(total)
}
fn output_base(
    country: &CountryProductionModifiersRow,
    line: &ProductionLineModifiersRow,
    run: &RawRun,
) -> Option<i64> {
    let base = factory_speed(country, value(country, 0xa8)?, line.naval_output)?;
    let factories = run.active_factories;
    let speed = multiply(i64::from(factories).wrapping_mul(100000), base, 100000)?;
    let speed = multiply(
        speed,
        line.resource_penalty_raw.wrapping_add(100000),
        100000,
    )?;
    let efficiency = multiply(q15_to_fixed5(run.efficiency_raw), 1000, 100000)?;
    multiply(speed, efficiency, 100000)
}

fn at(raw: &RawModifierRun, selected: Option<&str>) -> ProductionModifierRun {
    let country = &raw.country;
    let line = &raw.line;
    let run = &raw.run;
    let organisation = raw.organisation.as_deref();
    let localisation = raw.localisation.as_deref();
    let recorded_language = raw.recorded_language.as_ref();
    let mut warnings = BTreeSet::new();
    let wants = |id: &str| selected.is_none_or(|selected| selected == id);
    let mut groups = Vec::new();
    let mio_part = |raw: i64| ProductionModifierPart {
        id: format!("mio/{}", line.organisation),
        label: if line.organisation == 0 {
            Some("No organisation".into())
        } else {
            organisation.map(str::to_owned)
        },
        raw: raw.to_string(),
        unknown: (line.organisation != 0 && organisation.is_none())
            .then(|| "Recorded content does not name this organisation".into()),
    };
    if line.organisation != 0 && organisation.is_none() {
        warnings.insert(format!(
            "Organisation {} has no recorded content name",
            line.organisation
        ));
    }
    if line.organisation_dirty {
        warnings.insert(
            "Organisation bonuses were pending refresh at this read; retained values are shown"
                .into(),
        );
    }
    if line.licensed && wants("output") {
        warnings.insert("Licence output source attribution lacks exporter-specific accumulator, year and faction operands; its exact difference is Other / not identified".into());
    }
    if run.max_factories < 0 {
        warnings.insert("Native maximum factory count is negative".into());
    }
    let start = if line.naval_efficiency {
        Some(10000000)
    } else {
        value(country, 0xab).map(|value| {
            percent(value).wrapping_add(i64::from(country.base_start).wrapping_mul(100000))
        })
    };
    let cap = if line.naval_efficiency {
        Some(10000000)
    } else {
        value(country, 0xa9).map(|value| {
            percent(value)
                .wrapping_add(i64::from(country.base_cap).wrapping_mul(100000))
                .wrapping_add(percent(line.bonuses[2]))
                .max(0)
        })
    };
    if wants("efficiency") {
        let mut current_parts = Vec::new();
        if let Some(start) = start {
            current_parts.push(part(
                "start",
                "Current starting-efficiency parameter",
                start,
            ));
        }
        let current = q15_to_fixed5(run.efficiency_raw);
        residual(
            &mut current_parts,
            current,
            "Historical growth, switching retention and factory assignments are not individually recorded",
        );
        if current_parts.iter().all(|part| part.id != "other") {
            current_parts.push(ProductionModifierPart { id: "other".into(), label: None, raw: "0".into(),
            unknown: Some("Historical growth and switching may cancel; individual causes are not recorded".into()) });
        }
        groups.push(group("efficiency", Some(current), current_parts, "%",
        EFFICIENCY_NOTE));
    }
    if wants("start") {
        let start_parts = if line.naval_efficiency {
            vec![part("naval", "Native naval efficiency", 10000000)]
        } else {
            let mut parts = vec![part(
                "base",
                "Base starting efficiency",
                i64::from(country.base_start).wrapping_mul(100000),
            )];
            parts.extend(sources(
                country,
                0xab,
                -1,
                percent,
                localisation,
                recorded_language,
                &mut warnings,
            ));
            parts
        };
        groups.push(group(
            "start",
            start,
            start_parts,
            "%",
            "Applied to new factory caches; a current parameter, not the line's original value.",
        ));
    }
    if wants("cap") {
        let cap_parts = if line.naval_efficiency {
            vec![part("naval", "Native naval efficiency", 10000000)]
        } else {
            let mut parts = vec![part(
                "base",
                "Base efficiency cap",
                i64::from(country.base_cap).wrapping_mul(100000),
            )];
            parts.extend(sources(
                country,
                0xa9,
                -1,
                percent,
                localisation,
                recorded_language,
                &mut warnings,
            ));
            parts.push(mio_part(percent(line.bonuses[2])));
            parts
        };
        groups.push(group(
            "cap",
            cap,
            cap_parts,
            "%",
            "The native cap clamps the sum at zero; naval types use the native fixed cap.",
        ));
    }
    let gain = value(country, 0xaa).map(|country_gain| {
        fixed5_to_q15(country.gain_raw)
            .wrapping_add(fixed5_to_q15(country_gain))
            .wrapping_add(fixed5_to_q15(line.resource_penalty_raw))
            .wrapping_add(fixed5_to_q15(line.bonuses[3]))
    });
    if wants("gain") {
        let mut gain_parts = vec![part(
            "base",
            "Base efficiency gain",
            percent(q15_to_fixed5(fixed5_to_q15(country.gain_raw))),
        )];
        gain_parts.extend(sources(
            country,
            0xaa,
            -1,
            |value| percent(q15_to_fixed5(fixed5_to_q15(value))),
            localisation,
            recorded_language,
            &mut warnings,
        ));
        gain_parts.push(part(
            "resources",
            "Resource penalty",
            percent(q15_to_fixed5(fixed5_to_q15(line.resource_penalty_raw))),
        ));
        gain_parts.push(mio_part(percent(q15_to_fixed5(fixed5_to_q15(
            line.bonuses[3],
        )))));
        groups.push(group("gain", gain.map(|value| percent(q15_to_fixed5(value))), gain_parts, "%",
        "Growth also depends on each admitted factory's current efficiency, cap and the recorded balance factor; it is not an additive increase in the displayed efficiency."));
    }
    if wants("daily") {
        let daily = start
            .zip(cap)
            .zip(gain)
            .and_then(|((start, cap), gain)| {
                daily_gain(
                    line,
                    run,
                    fixed5_to_q15(start),
                    fixed5_to_q15(cap),
                    gain,
                    fixed5_to_q15(country.balance_raw),
                )
            })
            .map(q15_to_fixed5);
        groups.push(group("daily", daily, daily.map(|value| vec![part("native", "Per-factory growth and clamp", value)]).unwrap_or_default(), "%",
        "Per-factory tooltip calculation; summed admitted gains are divided by the active factory count."));
    }
    for index in 0..country.retention.len() {
        let id = format!("retention-{index}");
        if !wants(&id) {
            continue;
        }
        let base = country.retention[index];
        let retained = value(country, 0xa5)
            .and_then(|modifier| {
                multiply(
                    100000_i64.wrapping_sub(modifier).max(0),
                    100000_i64.wrapping_sub(base),
                    100000,
                )
            })
            .map(|loss| percent(100000_i64.wrapping_sub(loss)));
        let mut parts = vec![part("base", "Base switch retention", percent(base))];
        parts.extend(sources(
            country,
            0xa5,
            -1,
            |modifier| multiply(modifier, 100000_i64.wrapping_sub(base), 1000).unwrap_or(0),
            localisation,
            recorded_language,
            &mut warnings,
        ));
        groups.push(group(&id, retained, parts, "%",
            "Current native switch parameter. Actual switch family, module compatibility and intermediate switches are not inferred from daily reads."));
    }
    if wants("output") {
        let base = output_base(country, line, run);
        let mut output_parts = base
            .map(|value| {
                vec![part(
                    "base",
                    "Factories × powered IC × resources × stored efficiency",
                    value,
                )]
            })
            .unwrap_or_default();
        if let Some(base) = base {
            let key = if line.naval_output { 0xd6 } else { 0xd4 };
            output_parts.extend(sources(
                country,
                key,
                1,
                |modifier| multiply(base, modifier, 100000).unwrap_or(0),
                localisation,
                recorded_language,
                &mut warnings,
            ));
            if country.power_raw > 99999 {
                output_parts.extend(sources(
                    country,
                    key + 1,
                    1,
                    |modifier| multiply(base, modifier, 100000).unwrap_or(0),
                    localisation,
                    recorded_language,
                    &mut warnings,
                ));
            } else if let Some(modifier) = value(country, key).filter(|value| *value > 0)
                && let Some(power) = value(country, 0xa8)
                    .and_then(|modifier| {
                        multiply(modifier.wrapping_add(100000), country.power_raw, 100000)
                    })
                    .map(|value| value.max(0))
            {
                let adjustment = multiply(
                    100000_i64.wrapping_sub(power),
                    modifier.wrapping_neg(),
                    100000,
                )
                .and_then(|value| multiply(base, value, 100000));
                if let Some(value) = adjustment {
                    output_parts.push(part(
                        "power-adjustment",
                        "Native partial-power output adjustment",
                        value,
                    ));
                }
            }
            output_parts.push(mio_part(
                multiply(base, line.bonuses[1], 100000).unwrap_or(0),
            ));
            if line.refit_output {
                output_parts.extend(sources(
                    country,
                    0x1d5,
                    1,
                    |modifier| multiply(base, modifier, 100000).unwrap_or(0),
                    localisation,
                    recorded_language,
                    &mut warnings,
                ));
            }
            if line.conversion_output {
                output_parts.push(part(
                    "conversion-base",
                    "Base conversion bonus",
                    multiply(base, country.conversion_raw, 100000).unwrap_or(0),
                ));
                output_parts.extend(sources(
                    country,
                    0xdc,
                    1,
                    |modifier| multiply(base, modifier, 100000).unwrap_or(0),
                    localisation,
                    recorded_language,
                    &mut warnings,
                ));
                output_parts.push(mio_part(
                    multiply(base, line.bonuses[6], 100000).unwrap_or(0),
                ));
                if let Some(last) = output_parts.last_mut() {
                    last.id.push_str("/conversion");
                }
            }
        }
        residual(
            &mut output_parts,
            run.speed_raw,
            "Licence, native clamp/rounding or retained speed not identified by the supplying operands",
        );
        if line.licensed && output_parts.iter().all(|part| part.id != "other") {
            output_parts.push(ProductionModifierPart { id: "other".into(), label: None, raw: "0".into(),
            unknown: Some("Licence contributor operands were not recorded, even though the net residual is zero".into()) });
        }
        groups.push(group("output", Some(run.speed_raw), output_parts, "IC",
        OUTPUT_NOTE));
    }
    if wants("power") {
        let mut power_parts = vec![part(
            "base",
            "Stored powered-factory fraction",
            percent(country.power_raw),
        )];
        power_parts.extend(sources(
            country,
            0xa8,
            -1,
            |modifier| multiply(country.power_raw, modifier, 1000).unwrap_or(0),
            localisation,
            recorded_language,
            &mut warnings,
        ));
        let power = value(country, 0xa8)
            .and_then(|modifier| multiply(modifier.wrapping_add(100000), country.power_raw, 1000))
            .map(|value| value.max(0));
        groups.push(group("power", power, power_parts, "%", "Native powered-factory bonus and zero clamp; this factor changes the military or naval base IC rate."));
    }
    if wants("resources") {
        groups.push(group("resources", Some(percent(100000_i64.wrapping_add(line.resource_penalty_raw))),
        vec![part("base", "Base output factor", 10000000), part("resources", "Stored resource penalty", percent(line.resource_penalty_raw))], "%", "Line-local retained resource factor; also participates in the native efficiency growth coefficient."));
    }
    if wants("conversion") {
        let conversion = value(country, 0xdc).map(|value| {
            percent(
                country
                    .conversion_raw
                    .wrapping_add(value)
                    .wrapping_add(line.bonuses[6]),
            )
        });
        let mut conversion_parts = vec![part(
            "base",
            "Base conversion bonus",
            percent(country.conversion_raw),
        )];
        conversion_parts.extend(sources(
            country,
            0xdc,
            1,
            percent,
            localisation,
            recorded_language,
            &mut warnings,
        ));
        conversion_parts.push(mio_part(percent(line.bonuses[6])));
        groups.push(group("conversion", conversion, conversion_parts, "%",
        "Current conversion speed bonus. The output breakdown applies it only to equipment eligible for conversion under the recorded DLC settings."));
    }
    if wants("refit") {
        let refit = value(country, 0x1d5).map(percent);
        let refit_parts = sources(
            country,
            0x1d5,
            1,
            percent,
            localisation,
            recorded_language,
            &mut warnings,
        );
        groups.push(group("refit", refit, refit_parts, "%",
        "Current refit speed bonus. The output breakdown applies it only to eligible refit lines."));
    }
    ProductionModifierRun {
        groups,
        warnings: warnings.into_iter().collect(),
    }
}


pub struct ProductionLineBreakdown;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionBreakdownParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The line's handle in `production-lines-at` at `hour`."]
    pub handle: String,
    #[doc = " The factor shown in full."]
    pub group: String,
    #[doc = " The clicked hour: the line, its operands and its organisation are read at it."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionModifierChoice {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionLineBreakdownReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub groups: Vec<ProductionModifierChoice>,
    #[doc = " The asked factor at the hour; null with `missing` when its operands were not recorded."]
    pub group: Option<ProductionModifierGroup>,
    pub missing: Option<String>,
    #[doc = " The line's own notes at the hour: pending organisation bonuses, unnamed sources, operands not recorded."]
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProductionLineBreakdownReply);

type LineAt = (RawRun, Result<ProductionLineModifiersRow, String>, Result<CountryProductionModifiersRow, String>, Option<u32>);

fn witnessed(run: &RawRun, id: &str, reason: &str) -> Option<ProductionModifierGroup> {
    let (total, unit, note) = match id {
        "efficiency" => (q15_to_fixed5(run.efficiency_raw), "%", EFFICIENCY_NOTE),
        "output" => (run.speed_raw, "IC", OUTPUT_NOTE),
        _ => return None,
    };
    let mut parts = Vec::new();
    residual(&mut parts, total, reason);
    Some(group(id, Some(total), parts, unit, note))
}

impl Product for ProductionLineBreakdown {
    const KEY: &'static str = "production-line-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ProductionBreakdownParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, handle, group, hour: Read];
    type Reply = ProductionLineBreakdownReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ProductionBreakdownParams::decl(),
            ProductionLineBreakdownReply::decl(),
            ProductionModifierPart::decl(),
            ProductionModifierGroup::decl(),
            ProductionModifierChoice::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)?;
        if params.handle.parse::<u64>().is_err() {
            return Err("Invalid production line");
        }
        if !FACTORS.iter().any(|(id, _)| *id == params.group) {
            return Err("Unknown production factor");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let handle = params.handle.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid production line"))?;
        let hour = params.hour;
        let query = dashboard_base::queries::query(source, branch)?;
        let stretch = query.timeline.within(hour, hour).next().ok_or_else(|| dashboard_core::refused("Production lines were not recorded at this hour"))?;
        let Some(Holder::Found { slot, .. }) = query.holders.at(&params.country, hour).map(|run| run.holder) else {
            return Err(dashboard_core::refused("This country is not held by any country at this hour"));
        };
        let recording = query.replay.recording();
        let format = recording.format();
        let key = u128::from(handle);
        let tables = [(TableId::ProductionLines, None), (TableId::ProductionLineModifiers, None), (TableId::CountryProductionModifiers, None), (TableId::CountryLinks, None)];
        let mut read: Option<LineAt> = None;
        let mut unread: Option<String> = None;
        dashboard_query::read::walk(recording, stretch.run, &tables, hour, hour, |_, readers| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let line = match row::<tables::ProductionLines>(&readers[0], key, format) {
                Ok(Some(line)) => line,
                Ok(None) => return Ok(()),
                Err(reason) => {
                    unread = Some(reason);
                    return Ok(());
                }
            };
            let linked = match row::<tables::CountryLinks>(&readers[3], u128::from(line.country), format) {
                Ok(link) => link.and_then(|link| dashboard_text::metadata::link_slot(std::slice::from_ref(&link), line.country)),
                Err(reason) => {
                    unread = Some(format!("{reason}, so the line's country is not known"));
                    return Ok(());
                }
            };
            let modifiers = row::<tables::ProductionLineModifiers>(&readers[1], key, format)
                .and_then(|row| row.ok_or_else(|| "This line's production modifiers were not recorded at this hour".to_owned()));
            let country = row::<tables::CountryProductionModifiers>(&readers[2], u128::from(line.country), format)
                .and_then(|row| row.ok_or_else(|| "The country's production modifiers were not recorded at this hour".to_owned()));
            read = Some((line, modifiers, country, linked));
            Ok(())
        })?;
        if let Some(reason) = unread {
            return Err(dashboard_core::refused(reason));
        }
        let (line, modifiers, country, linked) = read.ok_or_else(|| dashboard_core::refused("This production line does not exist at this hour"))?;
        if linked != Some(slot) {
            return Err(dashboard_core::refused("This production line is not the country's at this hour"));
        }
        let mut warnings = BTreeSet::new();
        let (group, missing) = match (modifiers, country) {
            (Ok(modifiers), Ok(country)) => {
                let content = names::context(source, context)?;
                let mut named = Named::default();
                if line.mio != 0 {
                    names::name_at(recording, stretch.run, hour, &[], &[line.mio], &content, &mut named)?;
                }
                warnings.extend(std::mem::take(&mut named.warnings));
                let raw = RawModifierRun {
                    organisation: (line.mio != 0).then(|| named.organisations.get(&line.mio).and_then(|organisation| organisation.name.clone())).flatten(),
                    country: Arc::new(country),
                    line: modifiers,
                    run: line,
                    localisation: content.as_ref().ok().map(|content| Arc::clone(&content.catalogue.localisation)),
                    recorded_language: content.as_ref().ok().map_or_else(|| Arc::from(""), |content| Arc::from(content.catalogue.recorded_language.as_str())),
                };
                let mut run = raw.project(Some(&params.group));
                warnings.extend(run.warnings);
                (run.groups.pop(), None)
            }
            (Err(reason), _) | (Ok(_), Err(reason)) => {
                let kept = witnessed(&line, &params.group, &reason);
                let missing = match kept {
                    Some(_) => {
                        warnings.insert(reason);
                        None
                    }
                    None => Some(reason),
                };
                (kept, missing)
            }
        };
        let reply = ProductionLineBreakdownReply {
            branch_id: params.branch_id.clone(),
            tag: query.holders.name(&params.country, hour),
            hour,
            groups: FACTORS.iter().map(|(id, label)| ProductionModifierChoice { id: (*id).into(), label: (*label).into() }).collect(),
            group,
            missing,
            warnings: warnings.into_iter().collect(),
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clamped_growth_is_averaged_over_active_not_admitted_factories() {
        let line = ProductionLineModifiersRow {
            factory_efficiencies: vec![3276800, 1638400],
            ..Default::default()
        };
        let run = RawRun {
            active_factories: 4,
            max_factories: 2,
            ..Default::default()
        };
        let first = factory_gain(3276800, 3276800, 32768, 32768).unwrap();
        let second = factory_gain(1638400, 3276800, 32768, 32768).unwrap();
        assert!(first > 0);
        assert_eq!(
            daily_gain(&line, &run, 0, 3276800, 32768, 32768),
            Some(second / 4)
        );
    }
    #[test]
    fn signed_residual_reconciles_without_discarding_corrections() {
        let mut parts = vec![part("source", "Source", 120)];
        residual(&mut parts, 100, "unidentified");
        assert_eq!(parts[1].raw, "-20");
        assert_eq!(
            parts
                .iter()
                .map(|part| part.raw.parse::<i64>().unwrap())
                .sum::<i64>(),
            100
        );
    }
    #[test]
    fn presentation_residual_reconciles_the_wide_signed_sum_without_native_wrapping() {
        let mut parts = vec![
            part("first", "First source", i64::MAX),
            part("second", "Second source", 1),
        ];
        residual(&mut parts, i64::MIN, "unidentified");
        assert_eq!(parts.last().unwrap().raw, "-18446744073709551616");
        assert_eq!(
            parts
                .iter()
                .map(|part| part.raw.parse::<i128>().unwrap())
                .sum::<i128>(),
            i128::from(i64::MIN)
        );
    }
    #[test]
    fn selected_history_does_not_expand_or_warn_about_unrelated_modifier_roots() {
        let raw = RawModifierRun {
            country: Arc::new(CountryProductionModifiersRow {
                trees: vec![ProductionModifierTree {
                    key: 0xa9,
                    failure: b"synthetic unrelated cap failure".to_vec(),
                    nodes: vec![],
                }],
                ..Default::default()
            }),
            line: ProductionLineModifiersRow::default(),
            run: RawRun::default(),
            organisation: None,
            localisation: None,
            recorded_language: "english".into(),
        };
        let projected = raw.project(Some("resources"));
        assert_eq!(
            projected
                .groups
                .iter()
                .map(|group| group.id.as_str())
                .collect::<Vec<_>>(),
            ["resources"]
        );
        assert!(projected.warnings.is_empty());
    }
    #[test]
    fn factory_output_applies_power_and_output_modifiers_as_the_output_factor_does() {
        use replay_data::rows::ModifierBreakdownNode as Node;
        let country = |power_raw, values: &[(u32, i64)]| CountryProductionModifiersRow {
            power_raw,
            factory_speed: [200000, 400000, 600000, 1000000],
            trees: values
                .iter()
                .map(|&(key, value)| ProductionModifierTree {
                    key,
                    failure: vec![],
                    nodes: vec![Node {
                        children: 0,
                        value,
                        name: b"Synthetic source".to_vec(),
                    }],
                })
                .collect(),
            ..Default::default()
        };
        let half = country(50000, &[(0xa8, 0), (0xd4, 20000), (0xd6, 20000)]);
        assert_eq!(factory_output(&half, false), Some(330000));
        assert_eq!(factory_output(&half, true), Some(880000));
        let malus = country(50000, &[(0xa8, 0), (0xd6, -10000)]);
        assert_eq!(factory_output(&malus, true), Some(720000));
        let full = country(100000, &[(0xa8, 0), (0xd6, 10000), (0xd7, 5000)]);
        assert_eq!(factory_output(&full, true), Some(1150000));
        let impact = country(50000, &[(0xa8, -50000), (0xd6, 0)]);
        assert_eq!(factory_output(&impact, true), Some(700000));
        assert_eq!(
            factory_output(&country(100000, &[(0xa8, 0), (0xd6, 0)]), true),
            None
        );
        let mut failed = country(50000, &[(0xa8, 0), (0xd6, 0)]);
        failed.trees[1].nodes.clear();
        failed.trees[1].failure = b"unreadable".to_vec();
        assert_eq!(factory_output(&failed, true), None);
    }
    #[test]
    fn military_and_naval_partial_power_remove_the_unpowered_bonus_without_a_residual() {
        use replay_data::rows::ModifierBreakdownNode as Node;
        for (naval, base, adjustment, speed) in [
            (false, 300000, -30000, 330000),
            (true, 800000, -80000, 880000),
        ] {
            let output_key = if naval { 0xd6 } else { 0xd4 };
            let trees = [0xa8, 0xd4, 0xd6]
                .into_iter()
                .map(|key| ProductionModifierTree {
                    key,
                    failure: vec![],
                    nodes: vec![Node {
                        children: 0,
                        value: if key == output_key { 20000 } else { 0 },
                        name: b"Synthetic source".to_vec(),
                    }],
                })
                .collect();
            let country = CountryProductionModifiersRow {
                power_raw: 50000,
                base_cap: 100,
                factory_speed: [200000, 400000, 600000, 1000000],
                trees,
                ..Default::default()
            };
            let line = ProductionLineModifiersRow {
                naval_output: naval,
                factory_efficiencies: vec![3276800],
                ..Default::default()
            };
            let run = RawRun {
                active_factories: 1,
                max_factories: 1,
                efficiency_raw: 3276800,
                speed_raw: speed,
                ..Default::default()
            };
            let groups = RawModifierRun {
                country: Arc::new(country),
                line,
                run,
                organisation: None,
                localisation: None,
                recorded_language: "english".into(),
            }
            .project(Some("output"))
            .groups;
            let output = groups.iter().find(|group| group.id == "output").unwrap();
            assert_eq!(
                output
                    .parts
                    .iter()
                    .find(|part| part.id == "base")
                    .unwrap()
                    .raw,
                base.to_string()
            );
            assert_eq!(
                output
                    .parts
                    .iter()
                    .find(|part| part.id == "power-adjustment")
                    .unwrap()
                    .raw,
                adjustment.to_string()
            );
            assert!(output.parts.iter().all(|part| part.id != "other"));
            assert_eq!(output.total_raw, Some(speed.to_string()));
        }
    }
    #[test]
    fn formatter_depth_omits_zero_rows_keeps_aliases_and_ignores_nonzero_interior_totals() {
        use replay_data::rows::ModifierBreakdownNode as Node;
        let node = |children, value, name: &[u8]| Node {
            children,
            value,
            name: name.to_vec(),
        };
        let country = CountryProductionModifiersRow {
            trees: vec![ProductionModifierTree {
                key: 0xa9,
                failure: vec![],
                nodes: vec![
                    node(4, 9, b"root"),
                    node(1, 7, b"interior"),
                    node(0, 0, b"alias"),
                    node(0, -2, b"alias"),
                    node(0, 0, b""),
                    node(0, 3, b"alias"),
                ],
            }],
            ..Default::default()
        };
        let mut localisation = content_reader_localisation::localisation::Localisation::default();
        localisation.language = "english".into();
        let mut warnings = BTreeSet::new();
        let parts = sources(
            &country,
            0xa9,
            2,
            |value| value,
            Some(&localisation),
            "english",
            &mut warnings,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            parts
                .iter()
                .map(|part| (part.id.as_str(), part.raw.as_str()))
                .collect::<Vec<_>>(),
            [("a9/text:alias#2", "-2"), ("a9/text:alias#3", "3")]
        );
        let mut parts = parts;
        residual(&mut parts, 9, "native nonlinear total");
        assert_eq!(parts.last().unwrap().raw, "8");
        let scaled = sources(
            &country,
            0xa9,
            2,
            |_| 0,
            Some(&localisation),
            "english",
            &mut warnings,
        );
        assert_eq!(scaled.len(), 2);
        assert!(scaled.iter().all(|part| part.raw == "0"));
    }
    #[test]
    fn zero_sources_add_no_rows_or_warnings_to_output_or_refit() {
        use replay_data::rows::ModifierBreakdownNode as Node;
        let zeros = 24;
        let tree = |key: u32, named: Option<i64>| {
            let mut nodes = vec![Node {
                children: zeros + u32::from(named.is_some()),
                value: named.unwrap_or(0),
                name: Vec::new(),
            }];
            nodes.extend((0..zeros).map(|_| Node::default()));
            if let Some(value) = named {
                nodes.push(Node {
                    children: 0,
                    value,
                    name: b"Synthetic factory output".to_vec(),
                });
            }
            ProductionModifierTree {
                key,
                failure: vec![],
                nodes,
            }
        };
        let country = CountryProductionModifiersRow {
            power_raw: 100000,
            factory_speed: [200000, 400000, 600000, 1000000],
            trees: vec![
                tree(0xa8, None),
                tree(0xd4, Some(10000)),
                tree(0xd5, None),
                tree(0x1d5, None),
            ],
            ..Default::default()
        };
        let mut localisation = content_reader_localisation::localisation::Localisation::default();
        localisation.language = "english".into();
        let raw = RawModifierRun {
            country: Arc::new(country),
            line: ProductionLineModifiersRow {
                factory_efficiencies: vec![3276800],
                refit_output: true,
                ..Default::default()
            },
            run: RawRun {
                active_factories: 1,
                max_factories: 1,
                efficiency_raw: 3276800,
                speed_raw: 440000,
                ..Default::default()
            },
            organisation: None,
            localisation: Some(Arc::new(localisation)),
            recorded_language: "english".into(),
        };
        let output = raw.project(Some("output"));
        assert!(output.warnings.is_empty());
        assert_eq!(
            output.groups[0]
                .parts
                .iter()
                .map(|part| (part.id.as_str(), part.raw.as_str()))
                .collect::<Vec<_>>(),
            [
                ("base", "400000"),
                ("d4/text:Synthetic factory output#1", "40000"),
                ("mio/0", "0"),
            ]
        );
        let refit = raw.project(Some("refit"));
        assert!(refit.warnings.is_empty());
        assert!(refit.groups[0].parts.is_empty());
        assert_eq!(refit.groups[0].total_raw.as_deref(), Some("0"));
    }
    #[test]
    fn source_labels_share_the_recorded_text_guard_without_refusing_exact_amounts() {
        use replay_data::rows::ModifierBreakdownNode as Node;
        let mut country = CountryProductionModifiersRow {
            trees: vec![ProductionModifierTree {
                key: 0xa9,
                failure: vec![],
                nodes: vec![Node {
                    children: 0,
                    value: -2,
                    name: b"Synthetic spirit".to_vec(),
                }],
            }],
            ..Default::default()
        };
        let mut english = content_reader_localisation::localisation::Localisation::default();
        english.language = "english".into();
        let mut german = content_reader_localisation::localisation::Localisation::default();
        german.language = "german".into();
        let mut warnings = BTreeSet::new();
        let rows = sources(
            &country,
            0xa9,
            -1,
            |value| value,
            Some(&english),
            "english",
            &mut warnings,
        );
        assert_eq!(rows[0].label.as_deref(), Some("Synthetic spirit"));
        assert_eq!(rows[0].raw, "-2");
        assert!(warnings.is_empty());
        let expected = dashboard_text::scripted_text::stored_text(
            &german,
            "english",
            b"Synthetic spirit",
            "production modifier source",
        )
        .unwrap_err();
        let rows = sources(
            &country,
            0xa9,
            -1,
            |value| value,
            Some(&german),
            "english",
            &mut warnings,
        );
        assert_eq!(rows[0].unknown.as_deref(), Some(expected.as_str()));
        assert_eq!(rows[0].raw, "-2");
        assert!(rows[0].label.is_none());
        assert!(warnings.iter().any(|warning| warning.contains(&expected)));
        country.trees[0].nodes[0].name = vec![0xff];
        let rows = sources(
            &country,
            0xa9,
            -1,
            |value| value,
            Some(&english),
            "english",
            &mut warnings,
        );
        assert_eq!(
            rows[0].unknown.as_deref(),
            Some("production modifier source is not UTF-8")
        );
        assert_eq!(rows[0].raw, "-2");
        let rows = sources(
            &country,
            0xa9,
            -1,
            |value| value,
            None,
            "english",
            &mut warnings,
        );
        assert_eq!(
            rows[0].unknown.as_deref(),
            Some("Native source label has no localisation context")
        );
    }
}
