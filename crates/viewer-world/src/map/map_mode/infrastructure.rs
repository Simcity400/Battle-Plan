//! The infrastructure mode: each state coloured by its infrastructure level over its maximum.
//!
//! The mode and its hover read one building type: the first one in database order declared
//! `infrastructure = yes` ([`installed_content::buildings::Buildings::infrastructure`]). A state's level
//! is that type's level in the state's building status (zero without an entry). The game holds
//! no maximum; it builds one on demand, and for this type in a state it is the smaller of:
//!
//! - the type's own maximum: a positive `province_max`, else a positive `state_max`, else
//!   `NBuildings.MAX_BUILDING_LEVELS`;
//! - the maximum the state's live category (States `category`) sets for the type in its
//!   `buildings_max_level`, when it sets one;
//! - when some technology's `enable_building` names the type, the largest level the state
//!   controller's completed technologies enable for it (zero when none does).
//!
//! Each state's colour blends `MAP_MODE_INFRA_RANGE_COLOR_FROM` to `_TO` by the level over the
//! maximum, clamped to the range, in single precision. A state whose building status cannot hold
//! the type (a provincial or coastal-only type) or whose maximum is not positive takes opaque
//! black. Every region takes `GRADIENT_BORDERS_THICKNESS_STATE`, the constant gradient 241, no
//! camera override and the state outline cutoff.
//!
//! The game's limit has further operands, which no installed mod reaches for this type and which
//! recordings do not hold. A content form that reaches one is refused by name, with a count, and
//! the layer is not drawn: a category maximum for the type that is not one integer, a strategic
//! location naming it (a state's placed locations), a land or naval fort
//! weight (a game-wide fort level override), `exclusive_with`, `shares_slots` and `group_by` (other
//! buildings' levels and constructions, and the state's free slots), and a repeatable enabling
//! technology (the game applies it at each completed level but, after a load, only once the
//! technology is maxed). A state whose recorded building status names a country is refused and
//! counted: real states never do, and the game lifts such a status's limit. So is a state whose
//! category read was refused, when some category sets the type's maximum.
//!
//! The hover shows `MAPMODE_TOOLTIP_INFRASTRUCTURE` in place of the location block, with `INF`
//! the hovered province's state's level of the type, zero for a province outside every state
//! ([`Content::hover_level`]).

use super::{
    Inputs, LEVEL_GRADIENT, ModeDefines, ModeLayer, complete, finish, region_table_len, style,
};
use installed_content::{
    buildings::Buildings, defines::Defines, state_categories, strategic_locations,
    technology::Catalog,
};
use replay::{
    frame::DecodedFrame,
    rows::{Recorded, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_foundation::{
    map::tables::{core, rows},
    table_access::TokenNames,
};

/// The colour of a state whose status cannot hold the type or whose maximum is not positive.
pub(super) const NO_MAXIMUM: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// What the mode reads from the content stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    /// The type, or why the mode cannot read one.
    building: Result<Building, String>,
    /// Content forms that reach a limit operand recordings do not hold; each refuses the layer.
    refused: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Building {
    name: String,
    /// The type's own maximum.
    maximum: i32,
    /// Whether a state's building status can hold the type: neither provincial nor coastal-only.
    in_states: bool,
    /// Each category that sets the type's maximum, by folded name, with that maximum.
    categories: BTreeMap<Vec<u8>, i32>,
    /// Every technology declaration in database order; `None` when no technology's
    /// `enable_building` names the type.
    technologies: Option<Technologies>,
}

/// Every technology declaration in database order, the order the loader appends and indexes
/// them in: each one's folded id and the largest level its own `enable_building` entries give
/// the type, if any. Each declaration is its own record, so a repeated id keeps each one's
/// unlock; a recorded `database_index` (one-based) names the declaration at that index minus one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Technologies {
    occurrences: Vec<(Vec<u8>, Option<i32>)>,
}

/// The id of the declaration at a recorded one-based index.
fn unjoined_name(technologies: &Technologies, database_index: i32) -> String {
    usize::try_from(database_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| technologies.occurrences.get(index))
        .map_or_else(
            || format!("technology {database_index}"),
            |(id, _)| String::from_utf8_lossy(id).into_owned(),
        )
}

/// The hour's technology limits by controller slot, and the slots set aside.
#[derive(Default)]
struct Limits {
    limits: BTreeMap<u32, i32>,
    /// Slots whose limit depends on a declaration the recording does not resolve.
    unknown: BTreeSet<u32>,
    /// The technologies those slots depend on.
    reasons: BTreeSet<String>,
}

impl Default for Content {
    fn default() -> Self {
        Self {
            building: Err("no building type declares infrastructure = yes".to_owned()),
            refused: Vec::new(),
        }
    }
}

fn names(list: &[&[u8]]) -> String {
    list.iter()
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

impl Content {
    pub fn new(
        buildings: &Buildings,
        defines: &Defines,
        categories: &state_categories::Database,
        locations: &strategic_locations::Locations,
        technologies: Result<&Catalog, String>,
    ) -> io::Result<Self> {
        let building = match buildings.infrastructure() {
            Ok(Some(building)) => building,
            Ok(None) => return Ok(Self::default()),
            Err(why) => {
                return Ok(Self {
                    building: Err(why),
                    refused: Vec::new(),
                });
            }
        };
        let name = building.name.clone();
        let in_states = !building.provincial() && !building.only_costal;
        let mut refused = Vec::new();
        let mut enabling = None;
        let mut capping = BTreeMap::new();
        // A type no state status can hold takes the same colour whatever its limit's operands.
        if in_states {
            if building.land_fort > 0 || building.naval_fort > 0 {
                refused.push(format!(
                    "the infrastructure building {name} is a fort, so a game-wide fort level, which is not recorded, can replace its maximum"
                ));
            }
            for (key, present) in [
                ("exclusive_with", !building.exclusive_with.is_empty()),
                ("shares_slots", building.shares_slots),
                ("group_by", building.group_by.is_some()),
            ] {
                if present {
                    refused.push(format!(
                        "the infrastructure building {name} declares {key}, whose limit reads other buildings' levels and constructions and the state's free slots, which are not derived"
                    ));
                }
            }
            match categories.capping(&name) {
                Ok(list) => {
                    for (category, maximum) in list {
                        match maximum {
                            Ok(maximum) => {
                                capping.insert(category.to_ascii_lowercase(), maximum);
                            }
                            Err(why) => refused.push(why),
                        }
                    }
                }
                Err(why) => refused.push(why),
            }
            match locations.naming(&name) {
                Ok(naming) if naming.is_empty() => {}
                Ok(naming) => refused.push(format!(
                    "{} strategic locations give the infrastructure building {name} extra levels ({}), and a state's placed locations are not recorded",
                    naming.len(),
                    names(&naming)
                )),
                Err(why) => refused.push(why),
            }
            match technologies {
                Ok(catalog) if catalog.refusal.is_some() => refused.push(format!(
                    "the technologies that may enable the infrastructure building {name} are not all read: {}",
                    catalog.refusal.as_deref().unwrap_or_default()
                )),
                Ok(catalog) => {
                    // Loading appends each parent's entries to its child before any
                    // instance reads them.
                    let inherited = catalog.inherited_buildings();
                    let occurrences = catalog
                        .definitions
                        .iter()
                        .zip(&inherited)
                        .map(|(definition, buildings)| {
                            let level = buildings
                                .iter()
                                .filter(|(building, _)| {
                                    building.eq_ignore_ascii_case(name.as_bytes())
                                })
                                .map(|&(_, level)| level)
                                .max();
                            (
                                installed_content::parser::c_string(&definition.id).to_ascii_lowercase(),
                                level,
                            )
                        })
                        .collect::<Vec<_>>();
                    if occurrences.iter().any(|(_, level)| level.is_some()) {
                        enabling = Some(Technologies { occurrences });
                    }
                }
                Err(why) => refused.push(format!(
                    "the technologies that may enable the infrastructure building {name} are unread: {why}"
                )),
            }
        }
        Ok(Self {
            building: Ok(Building {
                maximum: building.base_maximum(defines)?,
                name,
                in_states,
                categories: capping,
                technologies: enabling,
            }),
            refused,
        })
    }

    /// The building type's token in this frame's dictionary, `None` when no row can name it.
    fn token(building: &Building, frame: &DecodedFrame) -> Result<Option<u32>, String> {
        let names = TokenNames::new(frame).map_err(|error| error.to_string())?;
        let [token] = names.tokens_of([building.name.as_str()]);
        match token? {
            Some(token) => Ok(Some(token)),
            // A dictionary missing rows may hold the type's token among them.
            None if !rows::<tables::LexicalTokens>(frame)
                .map_err(|error| error.to_string())?
                .is_complete() =>
            {
                Err(format!(
                    "the recorded dictionary is incomplete this hour and does not name the infrastructure building {}",
                    building.name
                ))
            }
            None => Ok(None),
        }
    }

    /// The level of the type in the hovered province's state: zero for a province outside every
    /// state or a state without an entry.
    pub fn hover_level(&self, frame: &DecodedFrame, province: u32) -> Result<i32, String> {
        let building = self.building.as_ref().map_err(Clone::clone)?;
        let provinces = frame
            .complete::<tables::Provinces>()
            .map_err(|error| error.to_string())?;
        let state = provinces
            .iter()
            .find(|row| row.id == province)
            .map_or(0, |row| row.state);
        if state == 0 {
            return Ok(0);
        }
        let Some(token) = Self::token(building, frame)? else {
            return Ok(0);
        };
        let rows = frame
            .complete::<tables::StateBuildings>()
            .map_err(|error| error.to_string())?;
        Ok(rows
            .iter()
            .find(|row| row.location == state && row.building_type == token)
            .map_or(0, |row| i32::from(row.level)))
    }

    /// Each controller slot's technology limit: the largest level its completed enabling
    /// technology declarations enable, zero for a slot that completed none. A slot whose limit
    /// depends on a declaration the recording does not resolve is set aside with the
    /// technologies it depends on.
    fn technology_limits(
        technologies: &Technologies,
        frame: &DecodedFrame,
        mode: &str,
    ) -> Result<Limits, String> {
        let links = core::<tables::CountryLinks>(frame)
            .map_err(|error| error.to_string())?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect::<BTreeMap<_, _>>();
        let enabling = technologies
            .occurrences
            .iter()
            .filter(|(_, level)| level.is_some())
            .map(|(id, _)| id.as_slice())
            .collect::<BTreeSet<_>>();
        let definitions = complete::<tables::TechnologyDefinitions>(frame, mode)?;
        // Each recorded declaration of an enabling id, joined to the content declaration at its
        // index where that declaration has the recorded id: its level and whether it repeats.
        let mut joined = BTreeMap::new();
        let mut unjoined = BTreeMap::new();
        for row in definitions {
            let id = installed_content::parser::c_string(&row.id).to_ascii_lowercase();
            if !enabling.contains(id.as_slice()) {
                continue;
            }
            let declaration = usize::try_from(row.database_index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| technologies.occurrences.get(index));
            match declaration {
                Some((content_id, level)) if *content_id == id => {
                    if let Some(level) = *level {
                        joined.insert(row.database_index, (level, row.max_level > 1));
                    }
                }
                _ => {
                    unjoined.insert(
                        row.database_index,
                        String::from_utf8_lossy(&id).into_owned(),
                    );
                }
            }
        }
        let mut limits = Limits::default();
        for row in complete::<tables::ResearchTechnologies>(frame, mode)? {
            let completed = row.classifier == engine::ctechnology::CLASSIFIER_COMPLETED;
            // A completion was applied, or for a partly researched one may have been.
            let applied = completed || row.level > 0;
            let unknown = if let Some(name) = unjoined.get(&row.database_index) {
                applied.then(|| format!("{name} (its declaration is not resolved)"))
            } else if let Some(&(level, repeatable)) = joined.get(&row.database_index) {
                if completed {
                    let slot = Self::slot(&links, row.country)?;
                    let limit = limits.limits.entry(slot).or_insert(0);
                    *limit = (*limit).max(level);
                    None
                } else {
                    // The game applies a repeatable technology at each completed level, but
                    // after a load only once it is maxed.
                    (repeatable && row.level > 0).then(|| {
                        format!(
                            "{} (repeatable and partly researched)",
                            unjoined_name(technologies, row.database_index)
                        )
                    })
                }
            } else {
                None
            };
            if let Some(why) = unknown {
                let slot = Self::slot(&links, row.country)?;
                limits.unknown.insert(slot);
                limits.reasons.insert(why);
            }
        }
        Ok(limits)
    }

    fn slot(links: &BTreeMap<u32, u32>, country: u32) -> Result<u32, String> {
        links.get(&country).copied().ok_or_else(|| {
            format!("a researched technology's country {country} is not in CountryLinks")
        })
    }

    /// Every state, coloured by its level over its maximum.
    pub(super) fn layer(
        &self,
        defines: &ModeDefines,
        inputs: &Inputs,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<ModeLayer>> {
        const MODE: &str = "infrastructure";
        let building = match &self.building {
            Ok(building) => building,
            Err(why) => {
                warnings.push(format!("The infrastructure map mode is not drawn: {why}"));
                return Ok(None);
            }
        };
        if !self.refused.is_empty() {
            warnings.extend(
                self.refused
                    .iter()
                    .map(|why| format!("The infrastructure map mode is not drawn: {why}")),
            );
            return Ok(None);
        }
        let frame = inputs.frame;
        let states = match complete::<tables::States>(frame, MODE) {
            Ok(states) => states,
            Err(why) => {
                warnings.push(why);
                return Ok(None);
            }
        };
        let mut levels = BTreeMap::new();
        let mut status_countries = BTreeSet::new();
        if building.in_states {
            let rows = match complete::<tables::StateBuildings>(frame, MODE) {
                Ok(rows) => rows,
                Err(why) => {
                    warnings.push(why);
                    return Ok(None);
                }
            };
            for row in rows {
                if row.country as i32 > 0 {
                    status_countries.insert(row.location);
                }
            }
            match Self::token(building, frame) {
                Ok(Some(token)) => {
                    for row in rows.iter().filter(|row| row.building_type == token) {
                        levels.insert(row.location, i32::from(row.level));
                    }
                }
                Ok(None) => {}
                Err(why) => {
                    warnings.push(format!("The infrastructure map mode is not drawn: {why}"));
                    return Ok(None);
                }
            }
        }
        let limits = match (&building.technologies, building.in_states) {
            (Some(technologies), true) => {
                match Self::technology_limits(technologies, frame, MODE) {
                    Ok(limits) => Some(limits),
                    Err(why) => {
                        warnings.push(format!("The infrastructure map mode is not drawn: {why}"));
                        return Ok(None);
                    }
                }
            }
            _ => None,
        };
        let links = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect::<BTreeMap<_, _>>();
        let names = if building.in_states && !building.categories.is_empty() {
            Some(TokenNames::new(frame)?)
        } else {
            None
        };
        let [from, to] = defines.infrastructure_colors;
        let mut styles = BTreeMap::new();
        let mut unresolved = 0;
        let mut uncategorised = 0;
        let mut unknown = 0;
        for state in states {
            if building.in_states && status_countries.contains(&state.id) {
                continue;
            }
            // A category maximum replaces the type's own, which still bounds it.
            let own = match &names {
                Some(names) => match &state.category {
                    Recorded::NotRecorded => {
                        warnings.push(
                            "The infrastructure map mode needs each state's live category, which this recording does not hold".to_owned(),
                        );
                        return Ok(None);
                    }
                    Recorded::Value(Some(token)) => match names.name(*token) {
                        Ok(name) => building
                            .categories
                            .get(name.to_ascii_lowercase().as_bytes())
                            .copied()
                            .unwrap_or(building.maximum)
                            .min(building.maximum),
                        Err(_) => {
                            uncategorised += 1;
                            continue;
                        }
                    },
                    Recorded::Value(None) => {
                        uncategorised += 1;
                        continue;
                    }
                },
                None => building.maximum,
            };
            let maximum = if !building.in_states {
                0
            } else if let Some(limits) = &limits {
                let Some(slot) = links.get(&state.controller) else {
                    unresolved += 1;
                    continue;
                };
                if limits.unknown.contains(slot) {
                    unknown += 1;
                    continue;
                }
                own.min(limits.limits.get(slot).copied().unwrap_or(0))
            } else {
                own
            };
            let color = if maximum > 0 {
                let level = levels.get(&state.id).copied().unwrap_or(0);
                let t = (level as f32 / maximum as f32).max(0.0).min(1.0);
                std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
            } else {
                NO_MAXIMUM
            };
            styles.insert(state.id + 1, style(color));
        }
        if !status_countries.is_empty() {
            warnings.push(format!(
                "The infrastructure map mode leaves {} states unfilled whose recorded building status names a country, which lifts the game's limit",
                status_countries.len()
            ));
        }
        if uncategorised != 0 {
            warnings.push(format!(
                "The infrastructure map mode leaves {uncategorised} states unfilled whose category read was refused or names no recorded token"
            ));
        }
        if unresolved != 0 {
            warnings.push(format!(
                "The infrastructure map mode leaves {unresolved} states unfilled whose controller is not in CountryLinks"
            ));
        }
        if unknown != 0 {
            let reasons = limits
                .as_ref()
                .map(|limits| {
                    limits
                        .reasons
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            warnings.push(format!(
                "The infrastructure map mode leaves {unknown} states unfilled whose controller researched a technology whose infrastructure unlock is not known: {reasons}"
            ));
        }
        let provinces = core::<tables::Provinces>(frame)?;
        let mut region_of =
            vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
        for province in provinces {
            if province.state != 0 {
                region_of[province.id as usize] = province.state + 1;
            }
        }
        Ok(Some(finish(
            region_of,
            styles,
            Some(defines.state_thickness),
            Some(LEVEL_GRADIENT),
            [0.0, defines.state_channel[1]],
        )))
    }
}

#[cfg(test)]
pub(super) fn for_tests(
    name: &str,
    maximum: i32,
    in_states: bool,
    technologies: Option<Vec<(&str, Option<i32>)>>,
    refused: Vec<String>,
) -> Content {
    Content {
        building: Ok(Building {
            name: name.to_owned(),
            maximum,
            in_states,
            categories: BTreeMap::new(),
            technologies: technologies.map(|occurrences| Technologies {
                occurrences: occurrences
                    .into_iter()
                    .map(|(id, level)| (id.as_bytes().to_vec(), level))
                    .collect(),
            }),
        }),
        refused,
    }
}

#[cfg(test)]
impl Content {
    /// The same content with these categories setting the type's maximum.
    pub(super) fn with_categories(mut self, categories: &[(&str, i32)]) -> Self {
        if let Ok(building) = &mut self.building {
            building.categories = categories
                .iter()
                .map(|&(name, maximum)| (name.as_bytes().to_ascii_lowercase(), maximum))
                .collect();
        }
        self
    }
}

#[cfg(test)]
mod tests;
