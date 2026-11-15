//! Occupation (family 54): the occupied states at an hour ([`states::OccupiedStates`]), each with
//! its resistance and compliance followed as live cells, its garrison and its effective law; and
//! the occupation trend, the declared series `occupier-totals`, whose hover reads the occupiers'
//! totals and collaboration at an hour ([`detail::OccupationDetail`]).

pub mod detail;
mod law;
pub mod states;

#[cfg(test)]
mod tests;

use crate::{
    Source,
    catalogue::{self, CachedResult, Catalogue},
    products::LoadContext,
};
use installed_content_core::{defines::Defines, equipment::Database, occupation_laws::Laws};
use replay_data::rows::{CountryLinkRow, StateResistanceRow};
use std::{collections::HashMap, sync::{Arc, Mutex, OnceLock}};

pub use law::OccupationLawInfo;

/// A state's names as last shown: its custom and owner names, and the name they make.
type Named = (Vec<u8>, Vec<u8>, Result<String, String>);

/// What the occupation replies read of the recorded content, bound once per request or follow;
/// each part says why it is missing.
pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    laws: Result<Arc<Laws>, String>,
    /// Read when a garrison's equipment is first checked: most hours need none.
    equipment: OnceLock<Result<Arc<Database>, String>>,
    /// The displayed strength below which a garrison shows red.
    red: Option<i64>,
    warnings: Vec<String>,
    names: Mutex<HashMap<u32, Named>>,
}

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> Self {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| format!("Recorded content stack unavailable; choose matching game and user folders ({why})"));
        let mut warnings = Vec::new();
        let (mut laws, mut red) = (Err("Occupation laws need the recorded content stack".to_owned()), None);
        if let Ok(cat) = &catalogue {
            warnings.extend(cat.verification.iter().cloned());
            warnings.extend(cat.warnings.iter().cloned());
            let layers = &cat.content.layers;
            laws = layers.derived::<CachedResult<Laws>>().get_or_init(|| installed_content_core::occupation_laws::read(layers)).map_err(|why| why.to_string());
            match &laws {
                Ok(laws) if !laws.warnings.is_empty() => warnings.push(format!("{} occupation-law files could not be read: {}", laws.warnings.len(), laws.warnings.join("; "))),
                Ok(_) => {}
                Err(why) => warnings.push(format!("Occupation law content unavailable: {why}")),
            }
            let defines: Result<Arc<Defines>, String> = layers.derived::<CachedResult<Defines>>().get_or_init(|| installed_content_core::defines::read(layers)).map_err(|why| why.to_string());
            match defines {
                Ok(defines) => red = defines.fixed5("NInterface", "GARRISON_STRENGTH_TO_SHOW_RED").map_err(|why| warnings.push(format!("Garrison red threshold unavailable: {why}"))).ok(),
                Err(why) => warnings.push(format!("Garrison defines unavailable: {why}")),
            }
        }
        Self { catalogue, laws, equipment: OnceLock::new(), red, warnings, names: Mutex::default() }
    }

    fn equipment(&self) -> Result<&Database, String> {
        let read = self.equipment.get_or_init(|| {
            let cat = self.catalogue.as_ref().map_err(|_| "Garrison equipment needs the recorded content stack".to_owned())?;
            let layers = &cat.content.layers;
            layers.derived::<CachedResult<Database>>().get_or_init(|| Database::load_complete(layers)).map_err(|why| why.to_string())
        });
        read.as_deref().map_err(Clone::clone)
    }

    fn catalogue(&self) -> Result<&Catalogue, &str> {
        self.catalogue.as_deref().map_err(String::as_str)
    }
}

/// Every authored occupation law's name and icon drawn through `cat`, an icon not drawn refused on
/// it; the law files that could not be read.
pub fn check_laws(cat: &Catalogue) -> Result<Vec<String>, String> {
    let laws = installed_content_core::occupation_laws::read(&cat.content.layers).map_err(|why| why.to_string())?;
    if laws.warnings.is_empty() {
        for token in laws.definitions.keys() {
            if let Some(why) = law::info(Ok(cat), Ok(&laws), token).missing {
                cat.refuse(&why, &format!("law {token}"));
            }
        }
    }
    Ok(laws.warnings)
}

/// The game's occupied test: active resistance against a positive occupied-country reference.
pub(super) fn occupied(row: &StateResistanceRow) -> bool {
    row.active != 0 && (row.country as i32) > 0
}

/// `raw`'s canonical slot through the hour's links.
fn slot(links: &[CountryLinkRow], raw: u32) -> Option<u32> {
    dashboard_text::metadata::link_slot(links, raw)
}
