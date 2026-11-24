
use std::{cmp::Reverse, collections::BTreeMap, io};
use viewer_runtime::count_math::{count_range_seeded, ratio};

use content_reader_defines::defines::Defines;
use content_reader_script::sub_units::{SubUnit, SubUnits};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::DivisionTemplateIconRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use viewer_foundation::perspective::Vision;
use dashboard_text::table_access::TokenNames;

#[derive(Clone, Debug)]
pub struct Content {
    subunits: SubUnits,
    defines: CountDefines,
    template_reveal: Result<viewer_foundation::visibility::template_reveal::Defines, String>,
}

#[derive(Clone, Copy, Debug)]
struct CountDefines {
    denominator: i64,
    minimum: i64,
    maximum: i64,
    range: i64,
}

impl Content {
    pub(crate) fn template_reveal_defines(
        &self,
    ) -> Result<&viewer_foundation::visibility::template_reveal::Defines, String> {
        self.template_reveal.as_ref().map_err(Clone::clone)
    }
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
            template_reveal: viewer_foundation::visibility::template_reveal::Defines::load(
                defines,
            )
            .map_err(|error| error.to_string()),
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

    pub(crate) fn subunit(&self, index: u32) -> Result<(&str, u64), String> {
        if index == 0 {
            return Ok(("", 0));
        }
        let name = self.subunits.name(index).ok_or_else(|| {
            format!("sub-unit database index {index} is absent from the content")
        })??;
        Ok((name, self.subunits.category(name.as_bytes())?))
    }
}

#[derive(Clone, Copy, Debug)]
struct Unit {
    owner: u32,
    rank: u8,
    template: u64,
    intel: u16,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedMembers {
    pub disclosed: bool,
    pub groups: Vec<(u64, i32)>,
}

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
    let land: Vec<_> = order
        .mixed
        .iter()
        .copied()
        .filter(|unit| divisions.binary_search_by_key(unit, |r| r.id).is_ok())
        .collect();
    let canonical = |raw| links.get(&raw).copied();
    let prepared = prepare_members(frame, &land, &canonical, vision)?;
    summary_prepared(content, frame, &prepared, &canonical, vision, loc, nato)
}

pub(crate) fn summary_prepared(
    content: &Content,
    frame: &DecodedFrame,
    prepared: &PreparedMembers,
    links: &dyn Fn(u32) -> Option<u32>,
    vision: &Vision,
    loc: &Localisation,
    nato: bool,
) -> Result<String, String> {
    if prepared.groups.is_empty() {
        return Ok(String::new());
    }
    let templates = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|e| e.to_string())?;
    let clock = frame
        .complete::<tables::Clock>()
        .map_err(|e| e.to_string())?
        .first()
        .ok_or("unit composition Clock row is absent")?;
    let tokens = TokenNames::new(frame).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut indices = BTreeMap::new();
    for &(handle, count) in &prepared.groups {
        let template = templates
            .binary_search_by_key(&handle, |r| r.template)
            .map(|i| &templates[i])
            .map_err(|_| format!("DivisionTemplateIcons template {handle} is absent"))?;
        if template.model_unreadable != 0 {
            return Err(format!("template {handle} unit composition was unreadable"));
        }
        let country = u32::try_from(template.country)
            .ok()
            .and_then(links)
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
                    r.target_index == country && links(r.owner_country) == Some(vision.ours())
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

pub(crate) fn prepare_members(
    frame: &DecodedFrame,
    members: &[u64],
    links: &dyn Fn(u32) -> Option<u32>,
    vision: &Vision,
) -> Result<PreparedMembers, String> {
    let Some(first) = members.first() else {
        return Ok(PreparedMembers::default());
    };
    let divisions = frame
        .complete::<tables::Divisions>()
        .map_err(|e| e.to_string())?;
    let division = |handle| {
        divisions
            .binary_search_by_key(handle, |r| r.id)
            .map(|i| &divisions[i])
            .map_err(|_| format!("Divisions member {handle} is absent"))
    };
    let first = division(first)?;
    let canonical =
        |raw| links(raw).ok_or_else(|| format!("country {raw} has no CountryLinks row"));
    let disclosed = vision
        .shares_vision_proven(canonical(first.roster_owner)?)
        .map_err(|e| e.describe())?;
    let mut units = Vec::with_capacity(members.len());
    for member in members {
        let division = division(member)?;
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
                .ok_or_else(|| format!("division {} unit intel unavailable", division.id))?,
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
    Ok(group_templates(units, disclosed, fallback))
}

fn group_templates(mut units: Vec<Unit>, disclosed: bool, fallback: u64) -> PreparedMembers {
    units.sort_by_key(|unit| Reverse((unit.rank, unit.owner, (unit.template >> 32) as u32)));
    let mut prepared = PreparedMembers {
        disclosed,
        ..Default::default()
    };
    let mut positions = BTreeMap::new();
    for unit in units.into_iter().filter(|u| u.intel >= 250) {
        let template = if disclosed { unit.template } else { fallback };
        let i = *positions.entry(template).or_insert_with(|| {
            prepared.groups.push((template, 0));
            prepared.groups.len() - 1
        });
        prepared.groups[i].1 = prepared.groups[i].1.wrapping_add(1);
    }
    prepared
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    category: u64,
    bounds: [i32; 2],
}

type Identity = (u32, Option<u32>);

#[expect(clippy::too_many_arguments, reason = "one template's recorded facts and the entries and index it adds to")]
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
    for subunit in &template.subunits {
        let (index, name, category) = if subunit.token == 0 {
            (0, "", 0)
        } else {
            let spelling = tokens.name(subunit.token).map_err(|e| e.to_string())?;
            let index = match content.subunits.get(spelling) {
                SubUnit::Index(index) => index,
                SubUnit::Unknown => {
                    return Err(format!(
                        "unit composition sub-unit {spelling} is absent from content"
                    ));
                }
                SubUnit::Uncertain(reason) => {
                    return Err(format!("unit composition sub-unit {spelling}: {reason}"));
                }
            };
            let name = content
                .subunits
                .name(index)
                .ok_or_else(|| format!("unit composition sub-unit index {index} has no name"))??;
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
        text.push_str(&subunit_icon(&entry.name, entry.category, nato));
    }
    Ok(text)
}

pub(crate) fn subunit_icon(name: &str, category: u64, nato: bool) -> String {
    let prefix = if category & 0x80004003C1 != 0 {
        "ship_"
    } else {
        "unit_"
    };
    format!("\u{13}{prefix}{name}_icon_small|{} ", u8::from(nato) + 1)
}

fn count_range(
    count: i32,
    index: u32,
    intel: i64,
    seed: i32,
    hour: i32,
    d: CountDefines,
) -> [i32; 2] {
    count_range_seeded(
        count,
        intel,
        index.wrapping_add(0x4A2).wrapping_add(seed as u32),
        hour,
        [d.minimum, d.maximum, d.range],
    )
}

#[cfg(test)]
mod tests;
