//! Strategic region names for keyed products: each region's title in the game's
//! language through the recorded content stack (mod overlays and replaced
//! region files included), read once per content and language binding. A
//! region whose title does not resolve keeps its named reason.

use crate::query::{Choice, Query};
use dashboard_base::catalogue::{self, CachedResult, KeyedResults, Refusals};
use dashboard_base::content;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// The strategic regions the content defines, with their titles.
pub struct RegionNames {
    names: BTreeMap<u32, Result<String, String>>,
    /// The region files' own reader notes.
    pub warnings: Vec<String>,
}

impl RegionNames {
    /// The query's content binding's region names; Err when the game folders
    /// are not chosen or the regions do not read.
    pub fn read(query: &Query<'_>) -> Result<Arc<Self>, String> {
        let options = query.content.ok_or("Game folders are not chosen")?;
        let key = content::localisation_key(options).map_err(|error| error.to_string())?;
        query
            .source
            .state::<KeyedResults<content::LocalisationKey, RegionNames>>()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_or_init(&key, |_| {
                let mut warnings = Vec::new();
                content::check_replay(
                    query.source.replay_ref(),
                    options.allow_unverified,
                    &mut warnings,
                )?;
                let resolved = query.source.content(&options.options)?;
                let layers = &resolved.layers;
                let definitions = layers
                    .derived::<CachedResult<installed_content_regions::regions::StrategicRegions>>()
                    .get_or_init(|| installed_content_regions::regions::read(layers))?;
                let localisation =
                    content::localisation(query.source, options).map_err(|error| error.to_string());
                let names = definitions
                    .regions
                    .iter()
                    .map(|(&id, region)| {
                        let name = localisation.as_ref().map_err(Clone::clone).and_then(|loc| {
                            if loc.get(&region.name_key).is_none() {
                                return Err(format!(
                                    "Localisation {} is absent from the recorded content stack",
                                    region.name_key
                                ));
                            }
                            catalogue::localised(loc, &region.name_key, catalogue::View::Plain)
                        });
                        (id, name)
                    })
                    .collect();
                warnings.extend(definitions.notes.clone());
                Ok(RegionNames { names, warnings })
            })
            .map_err(|error| error.to_string())
    }

    /// Names from an already resolved catalogue (tests and offline checks).
    pub fn new(names: BTreeMap<u32, Result<String, String>>, warnings: Vec<String>) -> Self {
        Self { names, warnings }
    }

    /// A region's title, or why it has none.
    pub fn name(&self, id: u32) -> Result<String, String> {
        self.names.get(&id).cloned().unwrap_or_else(|| {
            Err(format!(
                "Strategic region {id} is absent from the recorded content stack"
            ))
        })
    }

    /// Every region the content defines.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.names.keys().copied()
    }
}

/// Choices for the regions `ids`, named through `names`; a region without a
/// title is named unavailable and its reason counted.
pub fn choices(
    names: &Result<Arc<RegionNames>, String>,
    ids: &BTreeSet<u32>,
    notes: &mut Refusals,
) -> Vec<Choice> {
    ids.iter()
        .map(|&id| {
            let name = names
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|names| names.name(id))
                .unwrap_or_else(|why| {
                    notes.note(why, format!("region {id}"));
                    "Region name unavailable".into()
                });
            Choice {
                id: id.to_string(),
                name,
            }
        })
        .collect()
}
