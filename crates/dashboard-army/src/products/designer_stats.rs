//! The division designer's stats of a recorded template: the game's division stat calculator
//! ([`kernel`]) over the template's grouped definitions, each the composition country's
//! effective definition ([`definitions`]), and the designer's preview pool: for each required
//! equipment type, the first design of the country's production roster by the game's design
//! precedence that is module-valid and not excluded by the template, holding the whole
//! requirement (a type with no such design has no pool entry); with the template's own
//! manpower and full fuel. Nothing is applied after the calculator: the designer shows no
//! country, medal or commander modifier.
//!
//! A definition stored global stays global, with no bonus; a country-local one takes the
//! composition country's technologies and persistent unit bonuses. An operand the recording
//! lacks refuses the preview by name, never a zero.

pub mod definitions;
pub mod kernel;
pub mod rows;

use std::{collections::BTreeMap, sync::Arc};

use dashboard_base::products::equipment::designs;
use dashboard_text::{
    catalogue::{CachedResult, Catalogue},
    subunit_requirements::{Content, Definition as Identity, Sources},
    variant_names::{compare_variants, variant_order},
};
use installed_content_core::equipment::STAT_COUNT;
use replay_data::rows::{DivisionTemplateIconRow, tables};
use replay_frame::frame::DecodedFrame;

use definitions::Bonus;
use kernel::{Block, Group, Held, ONE, Operands, Slots};

/// The tables a preview reads besides the template's own.
pub const TABLES: [replay_data::rows::TableId; 13] = {
    use replay_data::rows::TableId;
    [
        TableId::AvailableEquipmentVariants,
        TableId::CountryLinks,
        TableId::CountryLocalSubunits,
        TableId::CountryUnitBonuses,
        TableId::LexicalTokens,
        TableId::ResearchTechnologies,
        TableId::SimulationFeatures,
        TableId::TechnologyDefinitions,
        TableId::VariantBonusSnapshots,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantStats,
        TableId::VariantUpgrades,
    ]
};

/// The roster the designer's preview selects designs from: the country's production roster.
const PRIMARY_ROSTER: u8 = 0;

/// What the calculator reads of a content stack, once per stack.
pub struct Calculator {
    pub content: Arc<Content>,
    pub slots: Slots,
    /// Terrain names: a unit bonus's terrain adjuster is no calculator word.
    pub terrains: Vec<Vec<u8>>,
    pub operands: Operands,
}

impl Calculator {
    pub fn read(catalogue: &Catalogue) -> Result<Arc<Self>, String> {
        catalogue
            .content
            .layers
            .derived::<CachedResult<Calculator>>()
            .get_or_init(|| Self::load(catalogue).map_err(std::io::Error::other))
            .map_err(|why| format!("the division stat calculator's content: {why}"))
    }

    fn load(catalogue: &Catalogue) -> Result<Self, String> {
        let layers = &catalogue.content.layers;
        let content = layers
            .derived::<CachedResult<Content>>()
            .get_or_init(|| Content::load(layers))
            .map_err(|why| format!("sub-units: {why}"))?;
        let defines = catalogue.defines()?;
        let fixed = |name: &str| {
            defines
                .fixed5("NMilitary", name)
                .map_err(|why| format!("NMilitary.{name}: {why}"))
        };
        let terrains = installed_content_core::terrain::read_definitions(layers)
            .map_err(|why| format!("terrains: {why}"))?
            .categories
            .into_iter()
            .map(|category| category.name)
            .collect();
        Ok(Self {
            content,
            slots: Slots::executable()?,
            terrains,
            operands: Operands {
                manpower_fill: ONE,
                fuel_fill: ONE,
                armor_vs_average: fixed("ARMOR_VS_AVERAGE")?,
                pen_vs_average: fixed("PEN_VS_AVERAGE")?,
                out_of_fuel_equipment: fixed("OUT_OF_FUEL_EQUIPMENT_MULT")?,
                fuel_capacity_hours: fixed("FUEL_CAPACITY_DEFAULT_HOURS")?,
                army_fuel_cost: fixed("ARMY_FUEL_COST_MULT")?,
                slowest_speed: fixed("SLOWEST_SPEED")?,
            },
        })
    }
}

/// Each technology a country has researched with a unit bonus, in technology database order,
/// its words multiplied by its researched level ([`technology_multiplier`]). (The recorded
/// applied-bonus word is an activity flag, not a count.)
pub fn technology_bonuses(
    frame: &DecodedFrame,
    content: &Content,
    catalogue: &installed_content_core::technology::Catalog,
    terrains: &[Vec<u8>],
    country_slot: u32,
    links: &BTreeMap<u32, u32>,
) -> Result<Vec<Bonus>, String> {
    let definitions = frame
        .complete::<tables::TechnologyDefinitions>()
        .map_err(|why| why.to_string())?;
    let mut held: Vec<_> = frame
        .complete::<tables::ResearchTechnologies>()
        .map_err(|why| why.to_string())?
        .iter()
        .filter(|row| row.researched != 0 && links.get(&row.country) == Some(&country_slot))
        .collect();
    held.sort_by_key(|row| row.database_index);
    let mut bonuses = Vec::new();
    for row in held {
        let definition = definitions
            .binary_search_by_key(&row.database_index, |definition| definition.database_index)
            .map(|at| &definitions[at])
            .map_err(|_| {
                format!(
                    "technology occurrence {} is not recorded",
                    row.database_index
                )
            })?;
        let name =
            String::from_utf8_lossy(installed_content_core::parser::c_string(&definition.id)).into_owned();
        let template = catalogue
            .get(&definition.id)?
            .ok_or_else(|| format!("technology {name} is not in the content"))?;
        if template.unit_bonus.is_empty() {
            continue;
        }
        bonuses.push(definitions::authored_bonus(
            content,
            &template.unit_bonus,
            terrains,
            technology_multiplier(row.level),
            format!("technology {name}"),
        )?);
    }
    Ok(bonuses)
}

/// The fixed5 multiplier of a researched technology's unit bonus: its recorded level when
/// positive (a repeatable or multilevel technology's words apply once per level), else one.
pub fn technology_multiplier(level: i32) -> i64 {
    if level > 0 { kernel::count(level) } else { ONE }
}

/// The source kinds of a recorded persistent bonus, by their stored value.
const SOURCE_KINDS: [&str; 7] = [
    "an unnamed source",
    "a special project",
    "an effect",
    "a grand doctrine",
    "a sub-doctrine",
    "a doctrine reward",
    "a doctrine milestone",
];

/// The country's persistent unit bonuses in stored order, from the latest daily read of the
/// list. A country never read, or read in a failed walk, has none known: the read is refused.
pub fn persistent_bonuses(frame: &DecodedFrame, country_slot: u32) -> Result<Vec<Bonus>, String> {
    let rows = frame
        .complete::<tables::CountryUnitBonuses>()
        .map_err(|why| format!("the country's persistent unit bonuses: {why}"))?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let tokens =
        dashboard_text::table_access::TokenNames::new(frame).map_err(|why| why.to_string())?;
    let spell = |token: u32| {
        tokens
            .name(token)
            .map(str::to_owned)
            .map_err(|why| format!("a persistent unit bonus: {why}"))
    };
    let mut held: Vec<_> = rows
        .iter()
        .filter(|row| dashboard_text::metadata::link_slot(links, row.country) == Some(country_slot))
        .collect();
    held.sort_by_key(|row| row.ordinal);
    held.into_iter()
        .map(|row| {
            let kind = SOURCE_KINDS
                .get(usize::from(row.source_kind))
                .ok_or_else(|| {
                    format!(
                        "a persistent unit bonus has source kind {}",
                        row.source_kind
                    )
                })?;
            let blocks = row
                .targets
                .iter()
                .map(|target| {
                    Ok(definitions::Block {
                        target: spell(target.token)?,
                        category: target.category,
                        stats: target
                            .stats
                            .iter()
                            .map(|stat| (stat.selector as usize, stat.value_raw))
                            .collect(),
                        combat_width: target.combat_width_raw,
                        grants: target
                            .grants
                            .iter()
                            .map(|grant| {
                                Ok(kernel::Grant {
                                    category: spell(grant.category_token)?,
                                    add: grant.add != 0,
                                    stats: grant
                                        .stats
                                        .iter()
                                        .map(|stat| (stat.selector as usize, stat.value_raw))
                                        .collect(),
                                })
                            })
                            .collect::<Result<_, String>>()?,
                    })
                })
                .collect::<Result<_, String>>()?;
            Ok(Bonus {
                blocks,
                multiplier: ONE,
                source: format!("a persistent unit bonus from {kind}"),
            })
        })
        .collect()
}

/// Where a country-local definition's persistent unit bonuses come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persistent {
    /// The recorded list.
    Recorded,
    /// None: a diagnostic of the validation alone, which shows what technologies explain.
    AssumedNone,
}

/// The template's grouped definitions, each effective for its scope.
pub fn definitions(
    calculator: &Calculator,
    frame: &DecodedFrame,
    technologies: &installed_content_core::technology::Catalog,
    template: &DivisionTemplateIconRow,
    persistent: Persistent,
) -> Result<Vec<(kernel::Definition, i32)>, String> {
    if template.model_unreadable != 0 {
        return Err("the template's grouped sub-units were not readable".into());
    }
    let sources = Sources::read(frame).map_err(|why| why.to_string())?;
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    // The composition country's bonuses, read once: one country scopes every local clone.
    let mut local: Option<Vec<Bonus>> = None;
    let mut groups = Vec::new();
    for unit in template.subunits.iter().filter(|unit| unit.token != 0) {
        let group = calculator
            .content
            .group(template, &sources, unit.token, unit.local, unit.count)
            .map_err(|why| why.to_string())?;
        let bonuses: Vec<&Bonus> = match group.identity {
            Identity::Global(_) => Vec::new(),
            Identity::Local { country, .. } => {
                if local.is_none() {
                    let mut bonuses = technology_bonuses(
                        frame,
                        &calculator.content,
                        technologies,
                        &calculator.terrains,
                        country,
                        &links,
                    )?;
                    if persistent == Persistent::Recorded {
                        bonuses.extend(persistent_bonuses(frame, country)?);
                    }
                    local = Some(bonuses);
                }
                local.iter().flatten().collect()
            }
        };
        let definition = definitions::effective(
            &calculator.content,
            &calculator.slots,
            group.subunit,
            &group.requirements,
            &bonuses,
        )?;
        groups.push((definition, unit.count));
    }
    Ok(groups)
}

/// A design's effective stats, from its recorded design and the content.
pub fn design_stats(
    frame: &DecodedFrame,
    content: &Content,
    variant: u64,
) -> Result<[i64; STAT_COUNT], String> {
    designs::sheet(frame, variant, &content.equipment).map(|sheet| sheet.values)
}

/// The designer's preview pool for `required` (archetype, fixed5 amount) needs: per archetype,
/// the first module-valid, not excluded design of `owner`'s production roster by the game's
/// precedence, holding the whole requirement.
pub fn preview_pool(
    frame: &DecodedFrame,
    content: &Content,
    owner: u32,
    excluded: &[u64],
    required: &[(usize, i64)],
) -> Result<Vec<Held>, String> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let owner_slot = dashboard_text::metadata::link_slot(links, owner)
        .ok_or("the template's roster owner has no country link")?;
    let definitions = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|why| why.to_string())?;
    let tokens =
        dashboard_text::table_access::TokenNames::new(frame).map_err(|why| why.to_string())?;
    let mut candidates: Vec<(usize, dashboard_text::variant_names::VariantOrder, u64)> = Vec::new();
    for row in frame
        .complete::<tables::AvailableEquipmentVariants>()
        .map_err(|why| why.to_string())?
        .iter()
        .filter(|row| row.roster_kind == PRIMARY_ROSTER)
        .filter(|row| dashboard_text::metadata::link_slot(links, row.country) == Some(owner_slot))
    {
        let definition = definitions
            .binary_search_by_key(&row.variant, |definition| definition.id)
            .map(|at| &definitions[at])
            .map_err(|_| format!("design {} has no recorded definition", row.variant))?;
        let key = content.equipment_key(&tokens, definition)?;
        let index = content.equipment.get(key).ok_or_else(|| {
            format!(
                "design {} names equipment {key} absent from the content",
                row.variant
            )
        })?;
        let order = variant_order(&content.equipment, index, definition, frame)?;
        candidates.push((order.archetype_ordinal, order, row.variant));
    }
    candidates
        .sort_by(|left, right| compare_variants(owner as i32, owner_slot as i32, left.1, right.1));
    let mut pool = Vec::new();
    for &(archetype, amount) in required {
        let mut chosen = None;
        for (candidate_archetype, _, variant) in &candidates {
            if *candidate_archetype != archetype || excluded.contains(variant) {
                continue;
            }
            let validity =
                designs::queries::structural_validity(frame, *variant, &content.equipment)?;
            if validity.failures.is_empty() {
                chosen = Some(*variant);
                break;
            }
        }
        // A type with no design the preview could hold has no pool entry.
        let Some(variant) = chosen else {
            continue;
        };
        pool.push(Held {
            stats: design_stats(frame, content, variant)?,
            archetype,
            amount,
        });
    }
    Ok(pool)
}

/// A group's critical-equipment fill: one without critical equipment; otherwise, over its
/// critical types in order, each type's template-wide holding (clamped to zero and to its
/// requirement) and its requirement, each times the type's own build cost and truncated, summed
/// apart; the held total over the required total by the guarded quotient (one when nothing is
/// required); then no more than `manpower_fill`. `cost` is a type's own fixed5 build cost.
pub fn critical_fill(
    definition: &kernel::Definition,
    required: &[(usize, i64)],
    pool: &[Held],
    manpower_fill: i64,
    cost: &dyn Fn(usize) -> i64,
) -> i64 {
    if definition.critical.is_empty() {
        return ONE;
    }
    let (mut held_total, mut required_total) = (0_i64, 0_i64);
    for &archetype in &definition.critical {
        let need = required
            .iter()
            .filter(|(held, _)| *held == archetype)
            .map(|(_, amount)| *amount)
            .fold(0_i64, i64::wrapping_add);
        let held = pool
            .iter()
            .filter(|design| design.archetype == archetype)
            .map(|design| design.amount)
            .fold(0_i64, i64::wrapping_add);
        let clamped = if held < 0 { 0 } else { held.min(need) };
        let ic = cost(archetype);
        held_total = held_total.wrapping_add(kernel::mul(ic, clamped));
        required_total = required_total.wrapping_add(kernel::mul(ic, need));
    }
    let fill = if required_total > 0 {
        kernel::div(held_total, required_total)
    } else {
        ONE
    };
    fill.min(manpower_fill)
}

/// The designer's preview of a template: its calculated stats, or why the stats that depend on
/// its equipment are unknown, and what does not depend on its equipment.
#[derive(Clone, Debug)]
pub struct Preview {
    pub block: Result<Block, String>,
    pub manpower: i32,
    pub training_time: i32,
    /// The template's requirement of each equipment type, by type in the equipment database's
    /// order: its name and fixed5 amount.
    pub requirements: Vec<(String, i64)>,
}

/// The designer's preview of `template`, whose roster owner is `owner`.
pub fn preview(
    calculator: &Calculator,
    frame: &DecodedFrame,
    technologies: &installed_content_core::technology::Catalog,
    template: &DivisionTemplateIconRow,
    owner: u32,
) -> Result<Preview, String> {
    let groups = definitions(
        calculator,
        frame,
        technologies,
        template,
        Persistent::Recorded,
    )?;
    let mut required: Vec<(usize, i64)> = Vec::new();
    for (definition, count) in &groups {
        for &(archetype, amount) in &definition.requirements {
            let need = amount.wrapping_mul(i64::from(*count));
            match required
                .iter_mut()
                .find(|(existing, _)| *existing == archetype)
            {
                Some((_, total)) => *total = total.wrapping_add(need),
                None => required.push((archetype, need)),
            }
        }
    }
    let manpower = groups.iter().fold(0_i32, |sum, (definition, count)| {
        sum.wrapping_add(definition.manpower.wrapping_mul(*count))
    });
    let training_time = groups
        .iter()
        .map(|(definition, _)| definition.training_time)
        .fold(0, i32::max);
    let mut ordered = required.clone();
    ordered.sort_by_key(|(archetype, _)| *archetype);
    let requirements = ordered
        .iter()
        .map(|&(archetype, amount)| {
            let name = calculator.content.equipment.equipment[archetype]
                .name_str()
                .map_err(|_| "an equipment type's name is not text".to_owned())?;
            Ok((name.to_owned(), amount))
        })
        .collect::<Result<_, String>>()?;
    let block = (|| {
        let pool = preview_pool(
            frame,
            &calculator.content,
            owner,
            &template.excluded_variants,
            &required,
        )?;
        let groups: Vec<Group<'_>> = groups
            .iter()
            .map(|(definition, count)| Group {
                    definition,
                    count: *count,
                    critical: critical_fill(
                        definition,
                        &required,
                        &pool,
                        calculator.operands.manpower_fill,
                        &|archetype| {
                            calculator.content.equipment.equipment[archetype].stats
                                [calculator.slots.build_cost_ic]
                        },
                    ),
            })
            .collect();
        Ok(kernel::calculate(
            &calculator.slots,
            &groups,
            &pool,
            &calculator.operands,
        ))
    })();
    Ok(Preview {
        block,
        manpower,
        training_time,
        requirements,
    })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod validation;
