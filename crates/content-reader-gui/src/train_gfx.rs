
use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt, parser};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub const DATABASE_DIRECTORY: &str = "gfx/train_gfx_database";
pub const CULTURE_FILE: &str = "common/graphicalculturetype.txt";
pub const NULL_CULTURE_NAME: &str = "noCultureType";
pub const CULTURE_SUFFIX: &str = "_gfx";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wagon {
    pub entity: String,
    pub scale: f32,
    pub rotation: f32,
    pub shift: [f32; 3],
    pub length: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Consist {
    pub wagon_0: Option<usize>,
    pub wagon_1: Option<usize>,
    pub wagon_2: Option<usize>,
    pub wagon_last: Option<usize>,
    pub wagons: Vec<usize>,
}

impl Consist {
    pub fn fixed(&self) -> impl Iterator<Item = usize> + '_ {
        [self.wagon_0, self.wagon_1, self.wagon_2]
            .into_iter()
            .flatten()
    }
}

pub type Table = BTreeMap<String, Consist>;

pub fn token_key(key: &str) -> String {
    key.to_ascii_lowercase()
}

#[derive(Clone, Debug, Default)]
pub struct Database {
    pub wagons: Vec<Wagon>,
    pub default: Table,
    pub countries: BTreeMap<String, Table>,
    pub cultures: BTreeMap<String, Table>,
    pub diagnostics: Vec<String>,
}

pub struct Registries<'a> {
    pub is_tag: &'a dyn Fn(&str) -> bool,
    pub cultures: &'a BTreeSet<String>,
    pub has_entity: &'a dyn Fn(&str) -> bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Country,
    Culture,
    Default,
}

impl Database {
    pub fn read(layers: &ResolvedLayers, registries: &Registries<'_>) -> io::Result<Self> {
        let mut database = Self::default();
        let mut names = BTreeMap::new();
        for path in layers.directory_files(DATABASE_DIRECTORY, "txt")? {
            let bytes = path.read_uncovered(layers)?;
            database.add(&bytes, &path.text(), registries, &mut names)?;
        }
        Ok(database)
    }

    pub fn add(
        &mut self,
        bytes: &[u8],
        path: &str,
        registries: &Registries<'_>,
        names: &mut BTreeMap<String, usize>,
    ) -> io::Result<()> {
        let document = parser::parse(bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
        for item in &document.items {
            let Some(key) = item.key.as_deref() else {
                return Err(invalid(format!(
                    "{path}: an unkeyed top-level value is not a train database form"
                )));
            };
            let key = String::from_utf8_lossy(key).into_owned();
            let Some(items) = item.value.items() else {
                return Err(invalid(format!(
                    "{path}: {key} is not a block, which the train database does not support"
                )));
            };
            if key == "wagon_data" {
                self.wagon(items, path, registries, names);
                continue;
            }
            let table = if key == "default" {
                &mut self.default
            } else if key.len() == 3 && (registries.is_tag)(&key) {
                self.countries.entry(key.clone()).or_default()
            } else if registries.cultures.contains(&key) {
                self.cultures.entry(key.clone()).or_default()
            } else if registries
                .cultures
                .contains(&format!("{key}{CULTURE_SUFFIX}"))
            {
                self.cultures
                    .entry(format!("{key}{CULTURE_SUFFIX}"))
                    .or_default()
            } else {
                self.diagnostics.push(format!(
                    "{path}: {key} is neither a country tag nor a graphical culture; skipped"
                ));
                continue;
            };
            let diagnostics = &mut self.diagnostics;
            for entry in items {
                let (Some(equipment), Some(fields)) = (entry.key.as_deref(), entry.value.items())
                else {
                    diagnostics.push(format!(
                        "{path}: {key} holds a value that is not an equipment block; skipped"
                    ));
                    continue;
                };
                let equipment = String::from_utf8_lossy(equipment).into_owned();
                let consist = table.entry(token_key(&equipment)).or_default();
                for field in fields {
                    let name = field.key.as_deref().unwrap_or_default();
                    let mut resolve = |wagon: &str| {
                        let found = names.get(wagon).copied();
                        if found.is_none() {
                            diagnostics.push(format!(
                                "{path}: {key}.{equipment} names unknown wagon {wagon}"
                            ));
                        }
                        found
                    };
                    let slot = match name {
                        b"wagon_0" => &mut consist.wagon_0,
                        b"wagon_1" => &mut consist.wagon_1,
                        b"wagon_2" => &mut consist.wagon_2,
                        b"wagon_last" => &mut consist.wagon_last,
                        b"wagons" => {
                            for value in field.value.items().unwrap_or_default() {
                                if let Some(wagon) = value.value.scalar_str()
                                    && let Some(index) = resolve(wagon)
                                {
                                    consist.wagons.push(index);
                                }
                            }
                            continue;
                        }
                        _ => {
                            diagnostics.push(format!(
                                "{path}: {key}.{equipment} has unexpected field {}",
                                String::from_utf8_lossy(name)
                            ));
                            continue;
                        }
                    };
                    if let Some(index) = field.value.scalar_str().and_then(&mut resolve) {
                        *slot = Some(index);
                    }
                }
            }
        }
        Ok(())
    }

    fn wagon(
        &mut self,
        items: &[parser::Item],
        path: &str,
        registries: &Registries<'_>,
        names: &mut BTreeMap<String, usize>,
    ) {
        let mut wagon = Wagon {
            entity: String::new(),
            scale: 1.0,
            rotation: 0.0,
            shift: [0.0; 3],
            length: 2.0,
        };
        let mut registered = false;
        for field in items {
            let name = field.key.as_deref().unwrap_or_default();
            let float = |value: &parser::Value| {
                value
                    .scalar_str()
                    .and_then(|text| crt::scan_f32(text.as_bytes()))
                    .filter(|value| value.is_finite())
            };
            let mut bad = || {
                self.diagnostics.push(format!(
                    "{path}: wagon_data field {} is unreadable; its default is kept",
                    String::from_utf8_lossy(name)
                ))
            };
            match name {
                b"entity" => {
                    wagon.entity = field.value.scalar_str().unwrap_or_default().to_owned();
                    registered = (registries.has_entity)(&wagon.entity);
                }
                b"scale" => match float(&field.value) {
                    Some(value) => wagon.scale = value,
                    None => bad(),
                },
                b"rotation" => match float(&field.value) {
                    Some(value) => wagon.rotation = value.to_radians(),
                    None => bad(),
                },
                b"length" => match float(&field.value) {
                    Some(value) => wagon.length = value,
                    None => bad(),
                },
                b"shift" => {
                    for (out, value) in wagon
                        .shift
                        .iter_mut()
                        .zip(field.value.items().unwrap_or_default())
                    {
                        match float(&value.value) {
                            Some(value) => *out = value,
                            None => bad(),
                        }
                    }
                }
                _ => bad(),
            }
        }
        if !registered {
            self.diagnostics.push(format!(
                "{path}: wagon_data entity {:?} is not a registered entity; the wagon is discarded",
                wagon.entity
            ));
            return;
        }
        names.insert(wagon.entity.clone(), self.wagons.len());
        self.wagons.push(wagon);
    }

    pub fn lookup(
        &self,
        tag: Option<&str>,
        culture: Option<&str>,
        equipment: &str,
        archetype: &str,
    ) -> Option<(Level, &Consist)> {
        fn find<'a>(table: &'a Table, keys: &[String; 2]) -> Option<&'a Consist> {
            keys.iter()
                .find_map(|key| table.get(key).filter(|entry| entry.wagon_0.is_some()))
        }
        let keys = [token_key(equipment), token_key(archetype)];
        let find = |table| find(table, &keys);
        if let Some(tag) = tag {
            if let Some(found) = self.countries.get(tag).and_then(find) {
                return Some((Level::Country, found));
            }
            if let Some(found) = culture.and_then(|c| self.cultures.get(c)).and_then(find) {
                return Some((Level::Culture, found));
            }
        }
        find(&self.default).map(|found| (Level::Default, found))
    }
}

pub fn cultures(layers: &ResolvedLayers) -> io::Result<BTreeSet<String>> {
    parse_cultures(&layers.read(CULTURE_FILE)?)
}

pub fn parse_cultures(bytes: &[u8]) -> io::Result<BTreeSet<String>> {
    let document = parser::parse(bytes).map_err(|e| invalid(format!("{CULTURE_FILE}: {e}")))?;
    let mut cultures = BTreeSet::from([NULL_CULTURE_NAME.to_owned()]);
    cultures.extend(
        document
            .items
            .iter()
            .map(|item| match (&item.key, item.value.scalar_str()) {
                (None, Some(name)) => Ok(name.to_owned()),
                _ => Err(invalid(format!(
                    "{CULTURE_FILE}: only bare culture names are supported"
                ))),
            })
            .collect::<io::Result<BTreeSet<String>>>()?,
    );
    Ok(cultures)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(files: &[&str], entities: &[&str]) -> Database {
        let cultures = BTreeSet::from(["western_european_gfx".to_owned(), "asian_gfx".to_owned()]);
        let is_tag = |tag: &str| matches!(tag, "GER" | "SOV");
        let has_entity = |entity: &str| entities.contains(&entity);
        let registries = Registries {
            is_tag: &is_tag,
            cultures: &cultures,
            has_entity: &has_entity,
        };
        let mut database = Database::default();
        let mut names = BTreeMap::new();
        for (index, file) in files.iter().enumerate() {
            database
                .add(
                    file.as_bytes(),
                    &format!("f{index}.txt"),
                    &registries,
                    &mut names,
                )
                .unwrap();
        }
        database
    }

    #[test]
    fn keys_classify_as_default_tag_culture_or_are_skipped() {
        let database = read(
            &["wagon_data = { entity = loco scale = 0.5 rotation = 90 length = 1.8 shift = { 0 0.6 0 } }
               default = { train_equipment_1 = { wagon_0 = loco } }
               GER = { train_equipment = { wagon_0 = loco } }
               western_european = { train_equipment_1 = { wagon_0 = loco } }
               asian_gfx = { train_equipment_1 = { wagon_0 = loco } }
               XYZ = { train_equipment_1 = { wagon_0 = loco } }"],
            &["loco"],
        );
        let wagon = &database.wagons[0];
        assert_eq!(
            (wagon.scale, wagon.length, wagon.shift),
            (0.5, 1.8, [0.0, 0.6, 0.0])
        );
        assert!((wagon.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(database.default.contains_key("train_equipment_1"));
        assert!(database.countries["GER"].contains_key("train_equipment"));
        assert_eq!(
            database.cultures.keys().collect::<Vec<_>>(),
            ["asian_gfx", "western_european_gfx"]
        );
        assert_eq!(database.diagnostics.len(), 1, "{:?}", database.diagnostics);
    }

    #[test]
    fn wagon_numbers_follow_the_crt_prefix_scan() {
        let database = read(
            &["wagon_data = { entity = loco scale = \"1.25tail\" }"],
            &["loco"],
        );
        assert_eq!(database.wagons[0].scale, 1.25);
    }

    #[test]
    fn unregistered_wagons_are_discarded_and_names_resolve_in_scan_order() {
        let database = read(
            &[
                "default = { t = { wagon_0 = late } }
                 wagon_data = { entity = gone }
                 wagon_data = { entity = early length = 3 }",
                "wagon_data = { entity = late }
                 wagon_data = { entity = early length = 4 }
                 default = { t = { wagon_0 = early wagons = { early gone late } } }
                 default = { t = { wagons = { early } wagon_last = late } }",
            ],
            &["early", "late"],
        );
        assert_eq!(database.wagons.len(), 3);
        let consist = &database.default["t"];
        assert_eq!(database.wagons[consist.wagon_0.unwrap()].length, 4.0);
        assert_eq!(consist.wagons, [2, 1, 2]);
        assert_eq!(consist.wagon_last, Some(1));
        assert!(
            database
                .diagnostics
                .iter()
                .any(|d| d.starts_with("f0.txt") && d.contains("unknown wagon late"))
        );
    }

    #[test]
    fn lookup_tries_type_then_archetype_per_level_and_needs_wagon_0() {
        let database = read(
            &["wagon_data = { entity = a } wagon_data = { entity = b } wagon_data = { entity = c }
               GER = { train_equipment = { wagon_0 = a } train_equipment_2 = { wagon_1 = a } }
               western_european_gfx = { train_equipment_1 = { wagon_0 = b } }
               default = { train_equipment_1 = { wagon_0 = c } }"],
            &["a", "b", "c"],
        );
        let lookup = |tag, culture, kind| {
            database
                .lookup(tag, culture, kind, "train_equipment")
                .map(|(level, consist)| (level, consist.wagon_0))
        };
        assert_eq!(
            lookup(
                Some("GER"),
                Some("western_european_gfx"),
                "train_equipment_1"
            ),
            Some((Level::Country, Some(0)))
        );
        assert_eq!(
            lookup(Some("GER"), None, "train_equipment_2"),
            Some((Level::Country, Some(0)))
        );
        assert_eq!(
            lookup(
                Some("FRA"),
                Some("western_european_gfx"),
                "train_equipment_1"
            ),
            Some((Level::Culture, Some(1)))
        );
        assert_eq!(
            lookup(None, Some("western_european_gfx"), "train_equipment_1"),
            Some((Level::Default, Some(2)))
        );
        assert_eq!(lookup(None, None, "train_equipment_3"), None);
    }

    #[test]
    fn entries_and_lookups_follow_the_lexers_ascii_fold() {
        let database = read(
            &[
                "wagon_data = { entity = a } wagon_data = { entity = b }
               default = { Train_Equipment_1 = { wagon_0 = a } }
               default = { train_equipment_1 = { wagon_1 = b } TRAIN_EQUIPMENT = { wagon_0 = b } }",
            ],
            &["a", "b"],
        );
        assert_eq!(database.default.len(), 2);
        let consist = &database.default[&token_key("train_equipment_1")];
        assert_eq!((consist.wagon_0, consist.wagon_1), (Some(0), Some(1)));
        assert_eq!(
            database
                .lookup(None, None, "train_equipment_1", "train_equipment")
                .map(|(level, consist)| (level, consist.wagon_0)),
            Some((Level::Default, Some(0)))
        );
        assert_eq!(
            database
                .lookup(None, None, "train_equipment_2", "Train_Equipment")
                .map(|(_, consist)| consist.wagon_0),
            Some(Some(1))
        );
    }

    #[test]
    fn the_culture_registry_is_a_list_of_bare_names() {
        assert_eq!(
            parse_cultures(b"western_european_gfx\nwestern_european_2d").unwrap(),
            BTreeSet::from([
                NULL_CULTURE_NAME.into(),
                "western_european_2d".into(),
                "western_european_gfx".into(),
            ])
        );
        assert!(parse_cultures(b"a = b").is_err());
    }

    #[test]
    fn null_culture_is_registered_without_an_authored_declaration() {
        let cultures = parse_cultures(b"").unwrap();
        assert_eq!(cultures, BTreeSet::from([NULL_CULTURE_NAME.to_owned()]));
        assert!(!cultures.contains("NoCultureType"));
        let mut database = Database::default();
        database
            .add(
                format!(
                    "wagon_data = {{ entity = car }} {NULL_CULTURE_NAME} = {{ equipment = {{ wagon_0 = car }} }}"
                )
                .as_bytes(),
                "null-culture-fixture.txt",
                &Registries {
                    is_tag: &|_| false,
                    cultures: &cultures,
                    has_entity: &|_| true,
                },
                &mut BTreeMap::new(),
            )
            .unwrap();
        assert!(database.diagnostics.is_empty());
        assert_eq!(
            database
                .lookup(
                    Some("OWN"),
                    Some(NULL_CULTURE_NAME),
                    "equipment",
                    "archetype"
                )
                .map(|(level, consist)| (level, consist.wagon_0)),
            Some((Level::Culture, Some(0)))
        );
    }
}
