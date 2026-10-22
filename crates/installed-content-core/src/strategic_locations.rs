//! `common/strategic_locations/*.txt`: each strategic location's building
//! entries, the extra maximum levels a location gives the buildings it names
//! once an effect places it in a province or a state.
//!
//! Only which buildings each location names is kept, for consumers that refuse
//! a building whose maximum a location can change. Direct files load in
//! filename order with lexer mode 0; every top-level block is one location.

use crate::{
    ResolvedLayers,
    fields::invalid,
    key_names::KeyNames,
    parser::{
        c_string,
        native::{self, Shape},
    },
    script_files,
};
use std::io;

/// Every declared strategic location, in load order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locations {
    /// Each location's name and the building names its entries list, as authored.
    pub locations: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    /// Why a file could not be read; the named buildings are then unknown.
    pub gap: Option<String>,
}

impl Locations {
    /// The locations whose entries name `building` under the lexer's ASCII fold.
    pub fn naming(&self, building: &str) -> Result<Vec<&[u8]>, String> {
        if let Some(why) = &self.gap {
            return Err(format!("strategic location definitions uncertain: {why}"));
        }
        Ok(self
            .locations
            .iter()
            .filter(|(_, buildings)| {
                buildings
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(building.as_bytes()))
            })
            .map(|(name, _)| name.as_slice())
            .collect())
    }
}

/// One file's locations.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Locations> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Location,
    }
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, _field| match node {
            Node::File => Shape::Object(Node::Location),
            // Each building entry consumes one value.
            Node::Location => Shape::One,
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("strategic locations: {cause}")));
    }
    Ok(Locations {
        locations: read
            .items
            .iter()
            .map(|location| {
                (
                    c_string(location.key.as_deref().unwrap_or_default()).to_vec(),
                    location
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .map(|entry| c_string(entry.key.as_deref().unwrap_or_default()).to_vec())
                        .collect(),
                )
            })
            .collect(),
        gap: None,
    })
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Locations> {
    let names = layers.key_names()?;
    let mut all = Locations::default();
    for path in script_files::paths(layers, "common/strategic_locations/", Some(".txt")) {
        match parse(&path.read(layers)?, &names) {
            Ok(file) => all.locations.extend(file.locations),
            Err(why) => {
                all.gap.get_or_insert_with(|| format!("{path}: {why}"));
            }
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_list_the_buildings_they_name() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let locations = parse(
            b"natural_harbor={naval_base=2} high_pass={Roads=1 bunker=-2} quiet={}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            locations.naming("roads").unwrap(),
            [b"high_pass".as_slice()]
        );
        assert!(locations.naming("rails").unwrap().is_empty());
        let uncertain = Locations {
            gap: Some("unreadable".into()),
            ..Locations::default()
        };
        assert!(uncertain.naming("roads").is_err());
    }
}
