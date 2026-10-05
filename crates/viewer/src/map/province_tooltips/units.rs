//! Province composition: 140C6AC80 -> 140C634C0, with count ranges from
//! 14141BFA0 -> 141417870. The optional selected-unit/detail outputs are absent.

use std::{cmp::Reverse, collections::BTreeMap, io};

use installed_content::{
    defines::Defines,
    localisation::{Argument, Localisation},
    unit_leader::{SubUnit, SubUnits},
};
use replay::{
    frame::DecodedFrame,
    rows::{DivisionTemplateIconRow, tables},
};

use crate::{map::perspective::Vision, table_access::TokenNames};

pub const LOCALISATION_KEYS: &[&str] = &["SUB_UNIT_ACCURAT_VALUE", "SUB_UNIT_RANGED_VALUE"];

/// 141006BC0's glyph inventory, from the one parsed sub-unit database. Both
/// NATO frames use the same declaration; missing declarations draw nothing.
pub(crate) fn assets(subunits: &SubUnits) -> (crate::map::native_gui::Assets, Vec<String>) {
    let mut assets = crate::map::native_gui::Assets::default();
    let mut refused = Vec::new();
    for spelling in subunits.registered() {
        let result = (|| {
            let name = std::str::from_utf8(spelling).map_err(|_| {
                format!(
                    "province composition sub-unit {:?} name is not UTF-8",
                    spelling
                )
            })?;
            let category = subunits.category(spelling)?;
            let prefix = if category & 0x80004003C1 != 0 {
                "ship_"
            } else {
                "unit_"
            };
            Ok::<_, String>(format!("\u{13}{prefix}{name}_icon_small|1 "))
        })();
        match result {
            Ok(text) => assets.add_text(&text),
            Err(reason) => refused.push(format!("Province hover: {reason} (1 subunit_icon).")),
        }
    }
    (assets, refused)
}

#[derive(Clone, Debug)]
pub struct Content {
    subunits: SubUnits,
    defines: CountDefines,
}

#[derive(Clone, Copy, Debug)]
struct CountDefines {
    denominator: i64,
    minimum: i64,
    maximum: i64,
    range: i64,
}

impl Content {
    pub fn load(defines: &Defines, subunits: Result<&SubUnits, &str>) -> Result<Self, String> {
        let subunits = subunits.map_err(str::to_owned)?;
        Self::from_defines(defines, subunits).map_err(|error| error.to_string())
    }

    fn from_defines(defines: &Defines, subunits: &SubUnits) -> io::Result<Self> {
        let denominators = defines.fixed5_array("NIntel", "COUNTRY_LEVEL_INTEL_MAXIMUMS")?;
        if denominators.len() != 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NIntel.COUNTRY_LEVEL_INTEL_MAXIMUMS must hold four values",
            ));
        }
        Ok(Self {
            subunits: subunits.clone(),
            defines: CountDefines {
                denominator: denominators[1],
                minimum: defines.fixed5("NIntel", "ARMY_TEMPLATE_UNIT_COUNT_INTEL_MIN")?,
                maximum: defines.fixed5("NIntel", "ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX")?,
                range: defines.fixed5(
                    "NIntel",
                    "ARMY_TEMPLATE_UNIT_COUNT_INTEL_RANGE_AT_LOWEST_INTEL",
                )?,
            },
        })
    }

    pub fn warnings(&self) -> &[String] {
        self.subunits.warnings()
    }
}

#[derive(Clone, Copy, Debug)]
struct Unit {
    owner: u32,
    rank: u8,
    template: u64,
    intel: u16,
}

/// Pure text from a recorded hour. CountryIntel keeps its actual daily sample;
/// no hourly freshness is asserted by reading that carried table.
pub fn summary(
    content: &Content,
    frame: &DecodedFrame,
    province: u32,
    links: &BTreeMap<u32, u32>,
    vision: &Vision,
    loc: &Localisation,
    nato: bool,
) -> Result<String, String> {
    let orders = frame
        .complete::<tables::ProvinceUnitOrder>()
        .map_err(|e| e.to_string())?;
    let order = orders
        .binary_search_by_key(&province, |r| r.province)
        .map(|i| &orders[i])
        .map_err(|_| format!("ProvinceUnitOrder province {province} is absent"))?;
    let divisions = frame
        .complete::<tables::Divisions>()
        .map_err(|e| e.to_string())?;
    // 140E65210/140E6B110 maintain the land array in the mixed array's order.
    let land: Vec<_> = order
        .mixed
        .iter()
        .filter_map(|unit| {
            divisions
                .binary_search_by_key(unit, |r| r.id)
                .ok()
                .map(|i| &divisions[i])
        })
        .collect();
    let Some(&first) = land.first() else {
        return Ok(String::new());
    };
    let canonical = |raw| {
        links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("province composition country {raw} has no CountryLinks row"))
    };
    // 140C6AC80 tests FIRST INPUT's roster owner before sorting, even if hidden.
    let disclosed = vision
        .shares_vision_proven(canonical(first.roster_owner)?)
        .map_err(|e| e.describe())?;
    let mut units = Vec::with_capacity(land.len());
    for division in land {
        let owner = canonical(division.roster_owner)?;
        let rank = if owner == vision.ours() {
            3
        } else if vision
            .shares_vision_proven(owner)
            .map_err(|e| e.describe())?
        {
            2
        } else {
            u8::from(!vision.at_war_proven(owner).map_err(|e| e.describe())?)
        };
        units.push(Unit {
            owner,
            rank,
            template: division.template,
            intel: vision
                .land_byte_known(division.id, owner, division.province)
                .ok_or_else(|| {
                    format!(
                        "division {} province composition unit intel (1416068C0) unavailable",
                        division.id
                    )
                })?,
        });
    }
    let fallback = if disclosed {
        first.template
    } else {
        let icons = frame
            .complete::<tables::ArmyIconInputs>()
            .map_err(|e| e.to_string())?;
        let icon = icons
            .binary_search_by_key(&first.id, |r| r.division)
            .map(|i| &icons[i])
            .map_err(|_| format!("ArmyIconInputs division {} is absent", first.id))?;
        if icon.alternate_template == 0 {
            first.template
        } else {
            icon.alternate_template
        }
    };
    let groups = group_templates(units, disclosed, fallback);
    if groups.is_empty() {
        return Ok(String::new());
    }
    let templates = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|e| e.to_string())?;
    let clock = frame
        .complete::<tables::Clock>()
        .map_err(|e| e.to_string())?
        .first()
        .ok_or("province composition Clock row is absent")?;
    let tokens = TokenNames::new(frame).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut indices = BTreeMap::new();
    for (handle, count) in groups {
        let template = templates
            .binary_search_by_key(&handle, |r| r.template)
            .map(|i| &templates[i])
            .map_err(|_| format!("DivisionTemplateIcons template {handle} is absent"))?;
        if template.model_unreadable != 0 {
            return Err(format!(
                "template {handle} province composition was unreadable"
            ));
        }
        let country = u32::try_from(template.country)
            .ok()
            .and_then(|raw| links.get(&raw))
            .copied()
            .ok_or_else(|| {
                format!(
                    "template {handle} country {} is unresolved",
                    template.country
                )
            })?;
        let intel = if !vision.fog()
            || vision
                .shares_vision_proven(country)
                .map_err(|e| e.describe())?
        {
            100_000
        } else {
            let rows = frame
                .complete::<tables::CountryIntel>()
                .map_err(|e| e.to_string())?;
            let raw = rows
                .iter()
                .find(|r| {
                    r.target_index == country && links.get(&r.owner_country) == Some(&vision.ours())
                })
                .map_or(0, |r| r.army);
            ratio(raw, content.defines.denominator)
        };
        aggregate(
            content,
            template,
            count,
            tokens,
            country,
            intel,
            clock.seed,
            frame.hour,
            &mut entries,
            &mut indices,
        )?;
    }
    format_entries(&entries, loc, nato)
}

fn group_templates(mut units: Vec<Unit>, disclosed: bool, fallback: u64) -> Vec<(u64, i32)> {
    // 140C5DAC0/140C5C900: descending rank, slot, template +0xC numeric ID.
    // Equal keys have the same actual template, so tie permutations cannot
    // change the grouped template order or multiplicity (140C6AC80).
    units.sort_by_key(|unit| Reverse((unit.rank, unit.owner, (unit.template >> 32) as u32)));
    let mut groups = Vec::<(u64, i32)>::new();
    let mut positions = BTreeMap::new();
    for unit in units.into_iter().filter(|u| u.intel >= 250) {
        let template = if disclosed { unit.template } else { fallback };
        let i = *positions.entry(template).or_insert_with(|| {
            groups.push((template, 0));
            groups.len() - 1
        });
        groups[i].1 = groups[i].1.wrapping_add(1);
    }
    groups
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    category: u64,
    bounds: [i32; 2],
}

// Index plus canonical clone scope identifies native pointers. Local clones
// retain their global database index; a matching glyph is not an identity.
type Identity = (u32, Option<u32>);

#[allow(clippy::too_many_arguments)]
fn aggregate(
    content: &Content,
    template: &DivisionTemplateIconRow,
    multiplicity: i32,
    tokens: TokenNames<'_>,
    country: u32,
    intel: i64,
    seed: i32,
    hour: i32,
    entries: &mut Vec<Entry>,
    indices: &mut BTreeMap<Identity, usize>,
) -> Result<(), String> {
    // 140C634C0: global aggregation in grouped-template/subunit first order.
    for subunit in &template.subunits {
        let (index, name, category) = if subunit.token == 0 {
            (0, "", 0)
        } else {
            let spelling = tokens.name(subunit.token).map_err(|e| e.to_string())?;
            let index = match content.subunits.get(spelling) {
                SubUnit::Index(index) => index,
                SubUnit::Unknown => {
                    return Err(format!(
                        "province composition sub-unit {spelling} is absent from content"
                    ));
                }
                SubUnit::Uncertain(reason) => {
                    return Err(format!(
                        "province composition sub-unit {spelling}: {reason}"
                    ));
                }
            };
            let name = content.subunits.name(index).ok_or_else(|| {
                format!("province composition sub-unit index {index} has no name")
            })??;
            (index, name, content.subunits.category(name.as_bytes())?)
        };
        let identity = (index, (subunit.local != 0).then_some(country));
        let count = multiplicity.wrapping_mul(subunit.count);
        let bounds = count_range(count, index, intel, seed, hour, content.defines);
        let i = *indices.entry(identity).or_insert_with(|| {
            entries.push(Entry {
                name: name.to_owned(),
                category,
                bounds: [0; 2],
            });
            entries.len() - 1
        });
        for (total, value) in entries[i].bounds.iter_mut().zip(bounds) {
            *total = total.wrapping_add(value);
        }
    }
    Ok(())
}

fn format_entries(entries: &[Entry], loc: &Localisation, nato: bool) -> Result<String, String> {
    let mut text = String::new();
    for (i, entry) in entries.iter().enumerate() {
        if i != 0 {
            text.push_str("   ");
        }
        let [minimum, maximum] = entry.bounds;
        let value = if minimum == maximum {
            loc.localise_with(
                b"SUB_UNIT_ACCURAT_VALUE",
                &[("VALUE", Argument::Int32(minimum))],
            )
        } else {
            loc.localise_with(
                b"SUB_UNIT_RANGED_VALUE",
                &[
                    ("MIN", Argument::Int32(minimum)),
                    ("MAX", Argument::Int32(maximum)),
                ],
            )
        }
        .map_err(|e| e.to_string())?;
        text.push_str(&value);
        // 141006BC0: executable prefixes/mask and settings +0x2AC frame.
        let prefix = if entry.category & 0x80004003C1 != 0 {
            "ship_"
        } else {
            "unit_"
        };
        text.push_str(&format!(
            "\u{13}{prefix}{}_icon_small|{} ",
            entry.name,
            u8::from(nato) + 1
        ));
    }
    Ok(text)
}

fn invalid_fixed(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
}

fn ratio(numerator: i64, denominator: i64) -> i64 {
    if denominator == 0 || invalid_fixed(numerator) {
        0xffff_ffff
    } else {
        numerator.wrapping_mul(100_000).wrapping_div(denominator)
    }
}

fn fixed15(value: i64) -> i64 {
    // Fixed5ToFixed15@1424D8420 truncates the quotient and fractional remainder.
    (value / 100_000)
        .wrapping_mul(32_768)
        .wrapping_add((value % 100_000).wrapping_mul(1 << 30) / 3_276_800_000)
}

fn count_range(
    count: i32,
    index: u32,
    intel: i64,
    seed: i32,
    hour: i32,
    d: CountDefines,
) -> [i32; 2] {
    let value = i64::from(count) << 15;
    if value <= 0 {
        return [0; 2];
    }
    let delta = intel.wrapping_sub(d.minimum);
    let denominator = d.maximum.wrapping_sub(d.minimum);
    if denominator == 0 || invalid_fixed(delta) {
        return [count; 2];
    }
    let ratio = ratio(delta, denominator);
    if ratio >= 100_000 {
        return [count; 2];
    }
    let width = (100_000_i64.wrapping_sub(ratio.max(0))).wrapping_mul(d.range) / 100_000;
    let spread = value
        .wrapping_mul(fixed15(width))
        .wrapping_shr(15)
        .wrapping_abs();
    let date = hour.wrapping_sub(43_800_000);
    let mut day = (date / 24) % 365;
    let mut month = 0_i32;
    // GetMonthZeroBased@140177140, native month table 14306D210.
    for length in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    let salt = index
        .wrapping_add(0x4A2)
        .wrapping_add(seed as u32)
        .wrapping_add(count as u32)
        .wrapping_add(month as u32)
        .wrapping_add((date / 8760).wrapping_mul(12) as u32);
    let deviation = |next| {
        let mut hash = 0x5EA6BA9F_u32
            .wrapping_sub(salt)
            .wrapping_add(salt.wrapping_mul(0xB5297A4D))
            .wrapping_add(next);
        hash = (hash >> 8 ^ hash).wrapping_add(0x68E31DA4);
        hash = (hash.wrapping_mul(0x100) ^ hash).wrapping_mul(0x1B56C4E9);
        hash = (hash >> 8 ^ hash).wrapping_mul(0x92D68CA2);
        let random = i64::from(((hash >> 8 ^ hash) & 0x7fff_ffff) % 100_000);
        let root = sqrt_fixed5(random);
        fixed15((100_000 - root).wrapping_mul(20_000) / 100_000 + root)
    };
    let lower = value
        .wrapping_sub(spread.wrapping_mul(deviation(0)) >> 15)
        .min(value);
    let upper = value
        .wrapping_add(spread.wrapping_mul(deviation(0xB5297A4D)) >> 15)
        .max(value)
        .max(lower);
    [lower, upper].map(|bound| (bound.max(32_768).wrapping_add(0x4000) >> 15) as i32)
}

fn sqrt_fixed5(input: i64) -> i64 {
    // 1424DAE00, called directly by 141417870 (not floating-point sqrt).
    if input < 2 {
        return 0;
    }
    let mut root = input.wrapping_mul(100_000) / 200_000;
    let tolerance = input.wrapping_mul(1_000) / 100_000;
    loop {
        let error = (root.wrapping_mul(root) / 100_000).wrapping_sub(input);
        let step = ratio(error, root.wrapping_mul(2));
        root = root.wrapping_sub(step);
        if (-tolerance..=tolerance).contains(&error) || step == 0 {
            return root;
        }
    }
}

#[cfg(test)]
mod tests;
