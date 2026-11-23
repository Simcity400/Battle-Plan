use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{fields::invalid, parser::c_string};
use content_reader_script::{key_names::KeyNames, native::{self, Shape}};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locations {
    pub locations: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
    pub gap: Option<String>,
}

impl Locations {
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
    for path in script_files::paths(layers, "common/strategic_locations/", Some(".txt"))? {
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
        content_reader_script::test_game::enter();
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
