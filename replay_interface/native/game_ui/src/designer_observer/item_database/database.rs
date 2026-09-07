use super::{AuthoredIconPoolTypeMap, ItemPoolDefinition, ItemPoolKey, collect_icon_pools, resolve_collected_icon};
use crate::{assets::AssetResolver, design_catalog::DesignerRowIcon, paradox_text};
use map_viewer::authored_equipment::{AuthoredEquipmentCatalog, AuthoredEquipmentDefinition};
use replay_core::{WorldFrame, country_name_semantics::NamingInput, design_catalog::DesignU32Observation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RootKey { Country(String), Faction(String), Continent(String), Default }

#[derive(Clone, Debug, Default)]
pub struct AuthoredIconDatabase {
    roots: BTreeMap<RootKey, Result<AuthoredIconPoolTypeMap, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedAuthoredIcon {
    pub icon: DesignerRowIcon,
    pub country: Option<String>,
    pub continent: Option<String>,
}

impl AuthoredIconDatabase {
    pub fn from_sources(sources: &[(String, String)], countries: &BTreeSet<String>, continents: &BTreeSet<String>) -> Result<Self, String> {
        let mut result = Self::default();
        for (path, text) in sources {
            let mut root = paradox_text::parse_str_strict(text).map_err(|error| format!("XUI13 {path}: {}", error.message))?;
            paradox_text::expand_file_scoped_variables(&mut root).map_err(|error| format!("XUI13 {path}: {error}"))?;
            for entry in root.as_object().ok_or("XUI13 graphic database is not an object")? {
                let token = entry.key.as_deref().ok_or("XUI13 graphic database has an unnamed root")?;
                if token.starts_with('@') { continue; }
                let key = if !entry.key_is_quoted && token.eq_ignore_ascii_case("default") { RootKey::Default }
                    else if continents.contains(token) { RootKey::Continent(token.into()) }
                    else if token.len() == 3 && countries.contains(&token.to_ascii_uppercase()) { RootKey::Country(token.to_ascii_uppercase()) }
                    else if token.strip_prefix("FACTION_").is_some_and(|tag| tag.len() == 3 && !tag.contains('_')) { RootKey::Faction(token[8..].into()) }
                    else { continue; };
                let slot = result.roots.entry(key).or_insert_with(|| Ok(AuthoredIconPoolTypeMap::default()));
                if let Ok(map) = slot {
                    if let Err(error) = map.append(&entry.value) { *slot = Err(format!("XUI13 {path}/{token}: {error}")); }
                }
            }
        }
        Ok(result)
    }

    pub(crate) fn load(assets: &AssetResolver, countries: &BTreeSet<String>, continents: &BTreeSet<String>) -> Result<Self, String> {
        let files = assets.list_files_immediate("gfx/interface/equipmentdesigner/graphic_db", Some("txt"))
            .map_err(|error| error.to_string())?;
        let sources = files.iter().map(|file| {
            let bytes = assets.read_file(file).map_err(|error| error.to_string())?;
            Ok((file.rel_path.clone(), bytes.into_iter().map(char::from).collect()))
        }).collect::<Result<Vec<_>, String>>()?;
        Self::from_sources(&sources, countries, continents)
    }

    pub fn resolve(&self, equipment: &AuthoredEquipmentCatalog, frame: &WorldFrame, creator: &str,
        token: &str, runtime: &DesignU32Observation,
        mut capital_continent: impl FnMut(replay_core::designer_icon_context::DesignerCapitalSlotObservation) -> Result<Option<String>, String>,
        mut available: impl FnMut(&DesignerRowIcon) -> Result<bool, String>,
    ) -> Result<Option<SelectedAuthoredIcon>, String> {
        let definition = equipment.definition(token).map_err(|error| error.to_string())?;
        let context = frame.country_name_semantics(creator);
        let mut continent_names = Vec::<String>::new();
        let continent_by_id = std::cell::RefCell::new(BTreeMap::<u32, String>::new());
        let rows = collect_icon_pools(frame, creator, pool_definition(equipment, definition)?,
            || pool_definition(equipment, equipment.graphics_source(definition).map_err(|error| error.to_string())?),
            |key, definition| {
                let (root, country, continent) = match key {
                    ItemPoolKey::Country(tag) => (RootKey::Country(tag.into()), Some(creator.to_owned()), None),
                    ItemPoolKey::FactionMember(tag) => (RootKey::Faction(tag.into()), Some(creator.to_owned()), None),
                    ItemPoolKey::CapitalRegion(id) => {
                        let name = continent_by_id.borrow().get(&id).cloned().ok_or("XUI13 internal continent identity is absent")?;
                        (RootKey::Continent(name.clone()), None, Some(name))
                    },
                    ItemPoolKey::Default => (RootKey::Default, None, None),
                };
                let Some(pools) = self.roots.get(&root) else { return Ok(Vec::new()); };
                let rows = pools.as_ref().map_err(Clone::clone)?.matching_rows(&definition.token, runtime,
                    || Ok(String::new()), || naming_token(&context.ruling_party_group, "creator ruling_party_group"),
                    || naming_token(&context.graphical_culture, "creator graphical_culture"))?;
                Ok(rows.into_iter().map(|row| super::ItemPoolRow { weights_raw: row.weights_raw,
                    icons: row.icons.into_iter().map(|icon| SelectedAuthoredIcon { icon, country: country.clone(), continent: continent.clone() }).collect() }).collect())
            },
            |state| {
                capital_continent(state)?.map(|name| {
                    // Local handles only join names within this immutable lookup, never native intern IDs.
                    let id = match continent_names.iter().position(|known| known == &name) {
                        Some(index) => u32::try_from(index).map_err(|_| "XUI13 continent handle overflow")?,
                        None => { let id = u32::try_from(continent_names.len()).map_err(|_| "XUI13 continent handle overflow")?; continent_names.push(name.clone()); id }
                    };
                    continent_by_id.borrow_mut().insert(id, name);
                    Ok(id)
                }).transpose()
            })?;
        Ok(resolve_collected_icon(&rows, |row| available(&row.icon))?.cloned())
    }
}

fn naming_token(value: &NamingInput<String>, field: &str) -> Result<String, String> {
    match value {
        NamingInput::Measured(value) => Ok(value.clone()),
        NamingInput::Absent => Ok(String::new()),
        NamingInput::Unavailable(reason) => Err(format!("XUI13 {field}: {reason:?}")),
    }
}

fn pool_definition<'a>(catalog: &'a AuthoredEquipmentCatalog, definition: &'a AuthoredEquipmentDefinition) -> Result<ItemPoolDefinition<'a>, String> {
    let hierarchy = catalog.hierarchy_profile(&definition.token).map_err(|error| error.to_string())?;
    let archetype = hierarchy.archetype.ok_or_else(|| format!("XUI13 {:?} archetype object is null", definition.token))?;
    Ok(ItemPoolDefinition { definition, archetype: catalog.definition(&archetype.token).map_err(|error| error.to_string())? })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn roots(text: &str) -> AuthoredIconDatabase {
        AuthoredIconDatabase::from_sources(&[("root.txt".into(),text.into())],
            &["GER".into(),"ITA".into()].into_iter().collect(), &["GER".into(),"europe".into()].into_iter().collect()).unwrap()
    }
    #[test]
    fn roots_classify_continent_before_country_and_keep_exact_faction_case() {
        let db = roots("GER = {} ita = {} europe = {} FACTION_GER = {} FACTION_ger = {} faction_GER = {} FACTION_G_E = {} unknown = {}");
        assert_eq!(db.roots.keys().cloned().collect::<BTreeSet<_>>(), [RootKey::Continent("GER".into()), RootKey::Country("ITA".into()),
            RootKey::Continent("europe".into()),RootKey::Faction("GER".into()),RootKey::Faction("ger".into())].into_iter().collect());
    }
    #[test]
    fn bare_default_and_quoted_continent_default_keep_distinct_ordered_pools() {
        let source = r#"
            default = { tank = { pool = { icons = { GFX_BareFirst } } } }
            "default" = { tank = { pool = { icons = { GFX_Continent } } } }
            DeFaUlT = { tank = { pool = { icons = { GFX_BareLast } } } }
            "Default" = { tank = { pool = { icons = { GFX_CaseExact } } } }
        "#;
        let db = AuthoredIconDatabase::from_sources(&[("roots.txt".into(), source.into())],
            &BTreeSet::new(), &["default".into(), "Default".into()].into_iter().collect()).unwrap();
        assert_eq!(db.roots.len(), 3);
        for (key, expected) in [
            (RootKey::Default, vec!["GFX_BareLast", "GFX_BareFirst"]),
            (RootKey::Continent("default".into()), vec!["GFX_Continent"]),
            (RootKey::Continent("Default".into()), vec!["GFX_CaseExact"]),
        ] {
            let context = || -> Result<String, String> { panic!("unfiltered pool needs no country facts") };
            let rows = db.roots[&key].as_ref().unwrap().matching_rows("tank",
                &DesignU32Observation::Measured { raw_u32: 0 }, context, context, context).unwrap();
            assert_eq!(rows.iter().flat_map(|row| row.icons.iter().cloned()).collect::<Vec<_>>(),
                expected.into_iter().map(|name| DesignerRowIcon::Sprite(name.into())).collect::<Vec<_>>());
        }
    }

    #[test]
    fn quoted_country_and_continent_failures_do_not_poison_independent_default() {
        let source = r#"
            default = { tank = { pool = { icons = { GFX_Default } } } }
            "default" = scalar
            "ita" = { tank = { pool = { icons = { GFX_Italy } } } }
            "GER" = { tank = { pool = { icons = { GFX_Continent } } } }
            "FACTION_ger" = { tank = { pool = { icons = { GFX_Faction } } } }
        "#;
        let db = AuthoredIconDatabase::from_sources(&[("roots.txt".into(), source.into())],
            &["GER".into(), "ITA".into()].into_iter().collect(),
            &["default".into(), "GER".into()].into_iter().collect()).unwrap();
        assert_eq!(db.roots.len(), 5);
        assert!(db.roots[&RootKey::Continent("default".into())].as_ref().unwrap_err().contains("not an object"));
        assert!(!db.roots.contains_key(&RootKey::Country("GER".into())));
        for (key, expected) in [(RootKey::Default, "GFX_Default"),
            (RootKey::Country("ITA".into()), "GFX_Italy"),
            (RootKey::Continent("GER".into()), "GFX_Continent"),
            (RootKey::Faction("ger".into()), "GFX_Faction")] {
            let context = || -> Result<String, String> { panic!("unfiltered pool needs no country facts") };
            let rows = db.roots[&key].as_ref().unwrap().matching_rows("tank",
                &DesignU32Observation::Measured { raw_u32: 0 }, context, context, context).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].icons, [DesignerRowIcon::Sprite(expected.into())]);
        }
    }

    #[test]
    fn malformed_reached_root_is_retained_without_erasing_other_roots() {
        let db = roots("default = { tank = { pool = { icons = { GFX_default } } } } ITA = scalar");
        assert!(db.roots[&RootKey::Default].is_ok());
        assert!(db.roots[&RootKey::Country("ITA".into())].is_err());
    }
}
