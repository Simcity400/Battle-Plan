
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    fixed5::script_fixed5,
    key_names::KeyNames,
    parser::{
        self, Item, Operator, Value,
        native::{self, Shape},
    },
};

pub mod aircraft;
pub mod context;
pub mod countries;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub entity: String,
    pub weight: i64,
}

pub fn insert_legacy(candidates: &mut Vec<Candidate>, candidate: Candidate) -> bool {
    insert(candidates, candidate, false)
}

fn insert(candidates: &mut Vec<Candidate>, candidate: Candidate, after_equal: bool) -> bool {
    if candidates
        .iter()
        .any(|entry| entry.entity == candidate.entity)
    {
        return false;
    }
    let at = candidates.partition_point(|entry| {
        entry.weight > candidate.weight || (after_equal && entry.weight == candidate.weight)
    });
    candidates.insert(at, candidate);
    true
}

#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub equipment: &'a str,
    pub archetype: Option<&'a str>,
    pub family: Option<(&'a str, Option<&'a str>)>,
    pub subunit: Option<&'a str>,
    pub ideology: Option<&'a str>,
    pub culture: Option<&'a str>,
    pub country: Option<&'a str>,
    pub faction_tags: &'a [String],
    pub capital_continent: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Default,
    Country(String),
    Faction(String),
    Continent(String),
}

impl std::fmt::Display for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Country(tag) => f.write_str(tag),
            Self::Faction(tag) => write!(f, "FACTION_{tag}"),
            Self::Continent(continent) => f.write_str(continent),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolModel<'a> {
    pub bucket: &'a Bucket,
    pub equipment: &'a str,
    pub file: &'a str,
    pub model: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Limit {
    Dlc(String),
    All(Vec<Limit>),
    Any(Vec<Limit>),
    Not(Vec<Limit>),
    Unsupported(String),
}

impl Limit {
    fn parse(items: &[Item]) -> io::Result<Vec<Self>> {
        items
            .iter()
            .map(|item| {
                let name = key(item)?;
                if item.operator != Operator::Equals {
                    return Ok(Self::Unsupported(format!(
                        "{name} {}",
                        String::from_utf8_lossy(item.operator.text())
                    )));
                }
                let keyword = item.keyword().map(parser::Keyword::folded);
                Ok(match (keyword.as_deref(), &item.value) {
                    (Some(b"has_dlc"), value) => match value.scalar_str() {
                        Some(dlc) if !dlc.starts_with('@') => Self::Dlc(dlc.to_owned()),
                        _ => Self::Unsupported(format!("has_dlc {value:?}")),
                    },
                    (Some(b"and"), Value::Block(inner)) => Self::All(Self::parse(inner)?),
                    (Some(b"or"), Value::Block(inner)) => Self::Any(Self::parse(inner)?),
                    (Some(b"not"), Value::Block(inner)) => Self::Not(Self::parse(inner)?),
                    (None, _) if item.key_quoted => {
                        Self::Unsupported(format!("quoted {name:?} (no built-in token)"))
                    }
                    _ => Self::Unsupported(name.to_owned()),
                })
            })
            .collect()
    }

    fn holds(&self, has_dlc: &impl Fn(&str) -> Option<bool>) -> Result<bool, String> {
        match self {
            Self::Dlc(name) => {
                has_dlc(name).ok_or_else(|| format!("DLC availability is unknown for {name}"))
            }
            Self::All(inner) => all(inner, has_dlc),
            Self::Any(inner) => {
                for limit in inner {
                    if limit.holds(has_dlc)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(inner) => {
                for limit in inner {
                    if limit.holds(has_dlc)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Unsupported(name) => Err(format!("unsupported empty-scope predicate {name}")),
        }
    }
}

fn all(limits: &[Limit], has_dlc: &impl Fn(&str) -> Option<bool>) -> Result<bool, String> {
    for limit in limits {
        if !limit.holds(has_dlc)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn empty_scope_condition(
    items: &[Item],
    has_dlc: impl Fn(&str) -> Option<bool>,
) -> io::Result<bool> {
    all(&Limit::parse(items)?, &has_dlc).map_err(io::Error::other)
}

pub trait PoolLimits {
    fn holds(&self, pool: usize, items: &[Item]) -> Result<bool, String>;
}

impl<F: Fn(&str) -> Option<bool>> PoolLimits for F {
    fn holds(&self, _pool: usize, items: &[Item]) -> Result<bool, String> {
        empty_scope_condition(items, self).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug)]
struct Pool {
    id: usize,
    weight: i64,
    models_weight: i64,
    icons_weight: i64,
    icons: Vec<String>,
    models: Vec<String>,
    subunits: BTreeSet<String>,
    ideologies: BTreeSet<String>,
    cultures: BTreeSet<String>,
    limit: Vec<Item>,
    file: String,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            id: 0,
            weight: 100_000,
            models_weight: 0,
            icons_weight: 0,
            icons: Vec::new(),
            models: Vec::new(),
            subunits: BTreeSet::new(),
            ideologies: BTreeSet::new(),
            cultures: BTreeSet::new(),
            limit: Vec::new(),
            file: String::new(),
        }
    }
}

impl Pool {
    fn weight(&self) -> i64 {
        if self.models_weight == 0 {
            self.weight
        } else {
            self.models_weight
        }
    }

    fn icons_weight(&self) -> i64 {
        if self.icons_weight == 0 {
            self.weight
        } else {
            self.icons_weight
        }
    }

    fn matches(&self, context: &Context<'_>) -> bool {
        let accepts = |allowed: &BTreeSet<String>, value: Option<&str>| {
            allowed.is_empty() || allowed.contains(&value.unwrap_or_default().to_ascii_lowercase())
        };
        accepts(&self.subunits, context.subunit)
            && accepts(&self.ideologies, context.ideology)
            && accepts(&self.cultures, context.culture)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    buckets: BTreeMap<Bucket, BTreeMap<String, Vec<Pool>>>,
    pool_count: usize,
}

pub struct Names {
    pub countries: BTreeSet<String>,
    pub continents: BTreeSet<String>,
}

impl Names {
    pub fn read(
        layers: &ResolvedLayers,
        default_map: &crate::terrain::DefaultMap,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let index = crate::country_tags::country_tag_index(layers)?;
        warnings.extend(index.warning());
        Ok(Self {
            countries: index.files.into_keys().collect(),
            continents: context::continent_names(&layers.read(&default_map.file("continent")?)?)?
                .into_iter()
                .collect(),
        })
    }

    fn bucket(&self, key: &[u8], default: bool) -> Option<Bucket> {
        if default {
            return Some(Bucket::Default);
        }
        let key = std::str::from_utf8(parser::c_string(key)).ok()?;
        let folded = key.to_ascii_lowercase();
        if self.continents.contains(&folded) {
            return Some(Bucket::Continent(folded));
        }
        if key.len() == 3 {
            let tag = key.to_ascii_uppercase();
            return self
                .countries
                .contains(&tag)
                .then_some(Bucket::Country(tag));
        }
        match key.split('_').collect::<Vec<_>>()[..] {
            ["FACTION", tag] if tag.len() == 3 => Some(Bucket::Faction(tag.to_owned())),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Bucket,
    Equipment,
    Pool,
}

fn document(bytes: &[u8], names: &Names, key_names: &KeyNames) -> io::Result<Vec<Item>> {
    let read = native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Node::File),
        Some(key_names),
        &mut |node, field| match node {
            Node::File => match names.bucket(&field.key.text, field.keyword_is("default")) {
                Some(_) => Shape::Object(Node::Bucket),
                None => Shape::Skip,
            },
            Node::Bucket => Shape::ObjectOrSkip(Node::Equipment),
            Node::Equipment if field.keyword_is("pool") => Shape::ObjectOrSkip(Node::Pool),
            Node::Equipment => Shape::Skip,
            Node::Pool => match field.keyword().as_deref() {
                Some(b"weight" | b"models_weight" | b"icons_weight") => Shape::One,
                Some(b"models" | b"icons") if field.opens() => Shape::RawList,
                Some(b"ideologies" | b"cultures" | b"sub_units") => Shape::List,
                Some(b"limit") => Shape::Trigger,
                _ => Shape::Skip,
            },
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(cause.to_string()));
    }
    Ok(read.items)
}

impl Content {
    pub fn load(layers: &ResolvedLayers, names: &Names) -> io::Result<Self> {
        let mut content = Self::default();
        let key_names = layers.key_names()?;
        for path in layers.directory_files("gfx/interface/equipmentdesigner/graphic_db", ".txt")? {
            document(&path.read_uncovered(layers)?, names, &key_names)
                .and_then(|items| content.read_document(&items, names, &path.text()))
                .map_err(|error| invalid(format!("{path}: {error}")))?;
        }
        Ok(content)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_texts(texts: &[&[u8]], names: &Names) -> io::Result<Self> {
        let mut content = Self::default();
        let key_names = KeyNames {
            countries: names
                .countries
                .iter()
                .map(|tag| tag.as_bytes().to_vec())
                .collect(),
            ..KeyNames::default()
        };
        for text in texts {
            content.read_document(&document(text, names, &key_names)?, names, "fixture")?;
        }
        Ok(content)
    }

    fn read_document(&mut self, items: &[Item], names: &Names, file: &str) -> io::Result<()> {
        for root in items {
            let Some(bucket) = names.bucket(
                root.key.as_deref().unwrap_or_default(),
                root.keyword_is("default"),
            ) else {
                continue;
            };
            let definitions = self.buckets.entry(bucket).or_default();
            for equipment in root.value.items().unwrap_or_default() {
                let Some(items) = equipment.value.items() else {
                    continue;
                };
                let pools = definitions
                    .entry(key(equipment)?.to_ascii_lowercase())
                    .or_default();
                for item in items.iter().filter(|item| item.keyword_is("pool")) {
                    let Some(fields) = item.value.items() else {
                        continue;
                    };
                    let mut pool = Pool {
                        file: file.to_owned(),
                        ..Pool::default()
                    };
                    for field in fields {
                        let fixed = |slot: &mut i64| {
                            if let Some(value) =
                                script_fixed5(field.value.scalar().unwrap_or_default())
                            {
                                *slot = value;
                            }
                        };
                        match field.keyword().map(parser::Keyword::folded).as_deref() {
                            Some(b"weight") => fixed(&mut pool.weight),
                            Some(b"models_weight") => fixed(&mut pool.models_weight),
                            Some(b"icons_weight") => fixed(&mut pool.icons_weight),
                            Some(b"icons") if field.value.items().is_some() => {
                                pool.icons.extend(strings(&field.value)?)
                            }
                            Some(b"models") if field.value.items().is_some() => {
                                pool.models.extend(strings(&field.value)?)
                            }
                            Some(b"sub_units") => {
                                pool.subunits.extend(folded_strings(&field.value)?)
                            }
                            Some(b"ideologies") => {
                                pool.ideologies.extend(folded_strings(&field.value)?)
                            }
                            Some(b"cultures") => {
                                pool.cultures.extend(folded_strings(&field.value)?)
                            }
                            Some(b"limit") => {
                                pool.limit.extend(
                                    crate::triggers::body(&field.value).unwrap_or_default(),
                                );
                            }
                            _ => {}
                        }
                    }
                    if !pool.models.is_empty() || !pool.icons.is_empty() {
                        pool.id = self.pool_count;
                        self.pool_count += 1;
                        pools.push(pool);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn models(&self) -> impl Iterator<Item = PoolModel<'_>> {
        self.buckets.iter().flat_map(|(bucket, definitions)| {
            definitions.iter().flat_map(move |(equipment, pools)| {
                pools.iter().flat_map(move |pool| {
                    pool.models.iter().map(move |model| PoolModel {
                        bucket,
                        equipment,
                        file: &pool.file,
                        model,
                    })
                })
            })
        })
    }

    pub fn refusals(&self) -> Vec<(&str, String)> {
        fn walk<'a>(limits: &'a [Limit], path: &str, out: &mut Vec<(&'a str, String)>) {
            for (index, limit) in limits.iter().enumerate() {
                let (name, inner) = match limit {
                    Limit::Unsupported(name) => {
                        out.push((name, format!("{path}{name}[{}]", index + 1)));
                        continue;
                    }
                    Limit::All(inner) => ("AND", inner),
                    Limit::Any(inner) => ("OR", inner),
                    Limit::Not(inner) => ("NOT", inner),
                    Limit::Dlc(_) => continue,
                };
                walk(inner, &format!("{path}{name}[{}] > ", index + 1), out);
            }
        }
        let mut refusals = Vec::new();
        for (bucket, definitions) in &self.buckets {
            for (equipment, pools) in definitions {
                for (index, pool) in pools.iter().enumerate() {
                    let subject = format!(
                        "{bucket:?} pool {} of {} for {equipment}",
                        index + 1,
                        pools.len()
                    );
                    let limits = match Limit::parse(&pool.limit) {
                        Ok(limits) => limits,
                        Err(error) => {
                            refusals.push((pool.file.as_str(), format!("{subject}: {error}")));
                            continue;
                        }
                    };
                    let mut names = Vec::new();
                    walk(&limits, "", &mut names);
                    for (name, path) in names {
                        refusals.push((
                            pool.file.as_str(),
                            format!(
                                "{subject}, limit {path}: unsupported empty-scope predicate {name}"
                            ),
                        ));
                    }
                }
            }
        }
        refusals
    }

    pub fn lookup(
        &self,
        context: &Context<'_>,
        has_dlc: impl Fn(&str) -> Option<bool>,
        entity_exists: impl Fn(&str) -> bool,
    ) -> Result<Vec<Candidate>, String> {
        self.lookup_with_limits(context, &has_dlc, entity_exists)
    }

    pub fn lookup_with_limits(
        &self,
        context: &Context<'_>,
        limits: &impl PoolLimits,
        entity_exists: impl Fn(&str) -> bool,
    ) -> Result<Vec<Candidate>, String> {
        let mut matches = self.matching(context, limits, |pool| !pool.models.is_empty())?;
        matches.sort_by_key(|pool| std::cmp::Reverse(pool.weight()));
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for pool in matches {
            for entity in pool.models.iter().rev() {
                if seen.insert(entity.as_str()) && entity_exists(entity) {
                    insert(
                        &mut candidates,
                        Candidate {
                            entity: entity.clone(),
                            weight: pool.weight(),
                        },
                        true,
                    );
                }
            }
        }
        Ok(candidates)
    }

    pub fn aircraft_override(
        &self,
        context: &Context<'_>,
        limits: &impl PoolLimits,
        declared: impl Fn(&str) -> bool,
    ) -> Result<Option<String>, String> {
        let mut pools = self.matching(context, limits, |pool| !pool.models.is_empty())?;
        pools.sort_by_key(|pool| std::cmp::Reverse(pool.weight()));
        Ok(pools
            .into_iter()
            .flat_map(|pool| pool.models.iter().rev())
            .find(|name| declared(name))
            .cloned())
    }

    pub fn lookup_icon(
        &self,
        context: &Context<'_>,
        has_dlc: impl Fn(&str) -> Option<bool>,
        exists: impl Fn(&str) -> bool,
    ) -> Result<Option<String>, String> {
        let mut matches = self.matching(context, &has_dlc, |pool| !pool.icons.is_empty())?;
        matches.sort_by_key(|pool| std::cmp::Reverse(pool.icons_weight()));
        Ok(matches
            .iter()
            .flat_map(|pool| pool.icons.iter().rev())
            .find(|icon| exists(icon))
            .cloned())
    }

    fn matching(
        &self,
        context: &Context<'_>,
        limits: &impl PoolLimits,
        wanted: impl Fn(&Pool) -> bool,
    ) -> Result<Vec<&Pool>, String> {
        let mut buckets = Vec::new();
        if let Some(country) = context.country {
            buckets.push(Bucket::Country(country.to_owned()));
            buckets.extend(context.faction_tags.iter().cloned().map(Bucket::Faction));
            if let Some(continent) = context.capital_continent {
                buckets.push(Bucket::Continent(continent.to_ascii_lowercase()));
            }
        }
        buckets.push(Bucket::Default);
        let mut contexts = vec![(context.equipment, context.archetype)];
        if let Some(family) = context.family
            && family.0 != context.equipment
        {
            contexts.push(family);
        }
        let mut matches = Vec::new();
        for (equipment, archetype) in contexts {
            for bucket in &buckets {
                let Some(definitions) = self.buckets.get(bucket) else {
                    continue;
                };
                for name in [Some(equipment), archetype].into_iter().flatten() {
                    let Some(pools) = definitions.get(&name.to_ascii_lowercase()) else {
                        continue;
                    };
                    for pool in pools
                        .iter()
                        .rev()
                        .filter(|pool| wanted(pool) && pool.matches(context))
                    {
                        if limits.holds(pool.id, &pool.limit).map_err(|reason| {
                            format!(
                                "equipment graphics pool for {name} in {}: {reason}",
                                pool.file
                            )
                        })? {
                            matches.push(pool);
                        }
                    }
                }
            }
        }
        Ok(matches)
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn key(item: &Item) -> io::Result<&str> {
    let bytes = item
        .key
        .as_deref()
        .ok_or_else(|| invalid("unnamed equipment graphics entry"))?;
    std::str::from_utf8(bytes).map_err(|_| invalid("equipment graphics key is not UTF-8"))
}

fn block(value: &Value) -> io::Result<&[Item]> {
    match value {
        Value::Block(items) => Ok(items),
        _ => Err(invalid("equipment graphics value is not an ordinary block")),
    }
}

fn strings(value: &Value) -> io::Result<Vec<String>> {
    block(value)?
        .iter()
        .map(|item| {
            item.value
                .c_str()
                .map(str::to_owned)
                .map_err(|error| invalid(format!("equipment graphics list: {error}")))
        })
        .collect()
}

pub use crate::fixed5::authored_fixed5;

fn folded_strings(value: &Value) -> io::Result<Vec<String>> {
    Ok(strings(value)?
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        Names {
            countries: ["GER", "EN2"].map(str::to_owned).into(),
            continents: ["europe"].map(str::to_owned).into(),
        }
    }

    fn content(text: &[u8]) -> io::Result<Content> {
        let mut content = Content::default();
        let key_names = KeyNames {
            countries: [b"AAA".to_vec()].into(),
            ..KeyNames::default()
        };
        content.read_document(
            &document(text, &names(), &key_names)?,
            &names(),
            "graphic_db/a.txt",
        )?;
        Ok(content)
    }

    fn enter_pool_test_tables() {
        use content_reader_core::{test_game, trigger_keys::Reader};
        use engine_executable::executable_tables::{self, Tables};
        static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
        let tables = TABLES.get_or_init(|| {
            test_game::executable()
                .trigger("tag", 0, Reader::Assign, 0)
                .build()
        });
        executable_tables::enter(Some(*tables));
    }

    #[test]
    fn the_graphic_db_trigger_reader_keeps_content_names() {
        content_reader_core::test_game::enter();
        let key_names = KeyNames {
            countries: [b"EST".to_vec()].into(),
            ..KeyNames::default()
        };
        let items = document(
            b"default = { plane = { pool = { limit = { EST = { always = yes } } } } }",
            &names(),
            &key_names,
        )
        .unwrap();
        let mut fields = items.as_slice();
        for key in ["default", "plane", "pool", "limit", "EST"] {
            assert_eq!(fields[0].key.as_deref(), Some(key.as_bytes()));
            fields = fields[0].value.items().unwrap();
        }
        assert_eq!(fields[0].key.as_deref(), Some(b"always".as_slice()));
        assert_eq!(fields[0].value.scalar(), Some(b"yes".as_slice()));
    }

    fn context<'a>(country: Option<&'a str>, factions: &'a [String]) -> Context<'a> {
        Context {
            equipment: "tank_1",
            archetype: Some("tank"),
            family: None,
            subunit: Some("light_armor"),
            ideology: Some("fascism"),
            culture: Some("western_european_gfx"),
            country,
            faction_tags: factions,
            capital_continent: Some("Europe"),
        }
    }

    #[test]
    fn aircraft_override_keeps_pool_visit_ties_and_reverse_strings() {
        content_reader_core::test_game::enter();
        let names = Names {
            countries: ["AAA".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        };
        let pools = Content::from_texts(
            &[br#"
            AAA = {
                plane_1 = {
                    pool = { models = { shared country_first } }
                    pool = { models = { country_last_a country_last_b } }
                }
                plane = { pool = { models = { country_archetype } } }
            }
            FACTION_BBB = {
                plane_1 = { pool = { models = { faction } } }
                family_1 = { pool = { models = { family_faction } } }
            }
            europe = { plane_1 = { pool = { models = { continent } } } }
            default = {
                plane_1 = { pool = { models_weight = 3 models = { highest } } }
                plane = { pool = { models = { default_archetype shared } } }
                family_1 = { pool = { models = { family_default } } }
            }
        "#],
            &names,
        )
        .unwrap();
        let factions = ["BBB".to_owned()];
        let context = Context {
            equipment: "plane_1",
            archetype: Some("plane"),
            family: Some(("family_1", Some("family"))),
            subunit: None,
            ideology: None,
            culture: None,
            country: Some("AAA"),
            faction_tags: &factions,
            capital_continent: Some("Europe"),
        };
        let visited = std::cell::RefCell::new(Vec::new());
        assert_eq!(
            pools.aircraft_override(&context, &|_: &str| Some(true), |name| {
                visited.borrow_mut().push(name.to_owned());
                false
            }),
            Ok(None)
        );
        assert_eq!(
            *visited.borrow(),
            [
                "highest",
                "country_last_b",
                "country_last_a",
                "country_first",
                "shared",
                "country_archetype",
                "faction",
                "continent",
                "shared",
                "default_archetype",
                "family_faction",
                "family_default"
            ]
        );
        assert_eq!(
            pools.aircraft_override(&context, &|_: &str| Some(true), |name| {
                ["country_last_a", "country_first", "family_default"].contains(&name)
            }),
            Ok(Some("country_last_a".to_owned()))
        );
    }

    #[test]
    fn aircraft_override_refuses_unknown_relevant_limits_before_selecting() {
        enter_pool_test_tables();
        let pools = content(
            br#"default = { tank = {
            pool = { models_weight = 3 models = { declared } }
            pool = { models = { unknown } limit = { has_dlc = unresolved } }
            pool = { sub_units = { other } models = { irrelevant } limit = { tag = GER } }
            pool = { icons = { irrelevant_icon } limit = { tag = GER } }
        } }"#,
        )
        .unwrap();
        let error = pools
            .aircraft_override(&context(None, &[]), &|_: &str| None, |_| {
                panic!("all relevant pool limits are decided before any entity lookup")
            })
            .unwrap_err();
        assert!(error.contains("graphic_db/a.txt"));
        assert!(error.contains("DLC availability is unknown for unresolved"));
        assert_eq!(
            pools.aircraft_override(&context(None, &[]), &|_: &str| Some(false), |name| {
                name == "declared"
            }),
            Ok(Some("declared".to_owned()))
        );
    }

    #[test]
    fn buckets_accept_declared_tags_factions_and_continents_and_skip_the_rest() {
        content_reader_core::test_game::enter();
        let factions = ["EN2".to_owned()];
        let pools = content(
            br#"
            default = { tank = { pool = { models = { base_a base_b } } } }
            GER = { tank_1 = { pool = { models_weight = 3 models = { ger } } } }
            FACTION_EN2 = { tank = { pool = { weight = 2 models = { allied } } } }
            europe = { tank = { pool = { weight = 0.5 models = { continental } } } }
        "#,
        )
        .unwrap();
        let found = pools
            .lookup(&context(Some("GER"), &factions), |_| Some(true), |_| true)
            .unwrap();
        let entities: Vec<_> = found.iter().map(|c| c.entity.as_str()).collect();
        assert_eq!(
            entities,
            ["ger", "allied", "base_b", "base_a", "continental"]
        );
        let found = pools
            .lookup(&context(None, &[]), |_| Some(true), |_| true)
            .unwrap();
        assert_eq!(found.len(), 2);
        let pools = content(
            br#"
            ger = { tank = { pool = { models = { lower } } } }
            NOPE = { tank = { pool = { models = { nope } } } }
            FACTION_XX = { tank = { pool = { models = { short } } } }
            "default" = { tank = { pool = { models = { quoted } } } }
            africa = { tank = { pool = { models = { africa } } } }
            FACTION_XYZ = { tank = { pool = { models = { unknown_faction } } } }
        "#,
        )
        .unwrap();
        let found = pools
            .lookup(&context(Some("GER"), &factions), |_| Some(true), |_| true)
            .unwrap();
        let entities: Vec<_> = found.iter().map(|c| c.entity.as_str()).collect();
        assert_eq!(entities, ["lower"]);
    }

    #[test]
    fn filters_and_dlc_limits_decide_pools_and_unknown_predicates_fail() {
        enter_pool_test_tables();
        let pools = content(
            br#"
            default = { tank = {
                pool = { sub_units = { medium_armor } models = { wrong_unit } }
                pool = { ideologies = { FASCISM } cultures = { western_european_gfx } models = { right } }
                pool = { limit = { has_dlc = "No Step Back" } models = { dlc } }
                pool = { limit = { NOT = { has_dlc = "No Step Back" } } models = { no_dlc } }
                pool = { limit = { OR = { has_dlc = a has_dlc = "No Step Back" } } models = { either } }
            } }
        "#,
        )
        .unwrap();
        let found = pools
            .lookup(
                &context(None, &[]),
                |name| Some(name == "No Step Back"),
                |_| true,
            )
            .unwrap();
        let entities: BTreeSet<_> = found.iter().map(|c| c.entity.as_str()).collect();
        assert_eq!(entities, BTreeSet::from(["right", "dlc", "either"]));
        let unknown = pools.lookup(&context(None, &[]), |_| None, |_| true);
        assert!(unknown.unwrap_err().contains("DLC availability is unknown"));
        let scoped =
            content(b"default = { tank = { pool = { limit = { tag = GER } models = { x } } } }")
                .unwrap();
        let quoted = content(
            br#"default = { tank = { pool = { limit = { "has_dlc" = x } models = { q } } } }"#,
        )
        .unwrap();
        assert!(
            quoted
                .lookup(&context(None, &[]), |_| Some(true), |_| true)
                .unwrap_err()
                .contains("quoted")
        );
        assert!(
            scoped
                .lookup(&context(None, &[]), |_| Some(true), |_| true)
                .is_err()
        );
        let nested = content(
            b"default = { tank = { pool = { limit = { has_dlc = a OR = { has_dlc = b \
                AND = { NOT = { has_dlc = c tag = GER } } } } models = { x } } } }",
        )
        .unwrap();
        assert_eq!(
            nested.refusals(),
            [(
                "graphic_db/a.txt",
                "Default pool 1 of 1 for tank, limit OR[2] > AND[2] > NOT[1] > tag[2]: unsupported empty-scope predicate tag".to_owned()
            )]
        );
        assert!(
            content(b"default = { tank = { pool = { models = { x } } } }")
                .unwrap()
                .refusals()
                .is_empty()
        );
    }

    #[test]
    fn refusals_identify_each_pool_and_repeated_predicate() {
        enter_pool_test_tables();
        let pools = content(
            br#"
            default = { plane = {
                pool = { icons = { first } limit = { OR = {
                    AAA = { has_cosmetic_tag = costume }
                    AAA = { has_cosmetic_tag = costume }
                } } }
                pool = { icons = { second } limit = { OR = {
                    AAA = { has_cosmetic_tag = costume }
                    AAA = { has_cosmetic_tag = costume }
                } } }
            } }
        "#,
        )
        .unwrap();
        let refusals = pools.refusals();
        assert_eq!(refusals.len(), 4);
        let distinct: BTreeSet<_> = refusals.iter().map(|(_, message)| message).collect();
        assert_eq!(distinct.len(), 4);
        for ((file, message), (pool, child)) in
            refusals.iter().zip([(1, 1), (1, 2), (2, 1), (2, 2)])
        {
            assert_eq!(*file, "graphic_db/a.txt");
            assert_eq!(
                message,
                &format!(
                    "Default pool {pool} of 2 for plane, limit OR[1] > AAA[{child}]: unsupported empty-scope predicate AAA"
                )
            );
        }
    }

    #[test]
    fn limit_parse_failure_refusals_identify_the_pool() {
        enter_pool_test_tables();
        let mut pools = content(
            b"default = { plane = { \
                pool = { icons = { first } limit = { tag = AAA } } \
                pool = { icons = { second } limit = { tag = AAA } } } }",
        )
        .unwrap();
        for pool in pools
            .buckets
            .get_mut(&Bucket::Default)
            .unwrap()
            .get_mut("plane")
            .unwrap()
        {
            pool.limit[0].key = None;
        }
        assert_eq!(
            pools.refusals(),
            [
                (
                    "graphic_db/a.txt",
                    "Default pool 1 of 2 for plane: unnamed equipment graphics entry".to_owned()
                ),
                (
                    "graphic_db/a.txt",
                    "Default pool 2 of 2 for plane: unnamed equipment graphics entry".to_owned()
                )
            ]
        );
    }

    #[test]
    fn duplicates_keep_their_first_entry_and_legacy_inserts_before_equals() {
        let mut candidates = Vec::new();
        for (entity, weight) in [("a", 5), ("b", 5), ("a", 9)] {
            insert(
                &mut candidates,
                Candidate {
                    entity: entity.into(),
                    weight,
                },
                true,
            );
        }
        insert_legacy(
            &mut candidates,
            Candidate {
                entity: "c".into(),
                weight: 5,
            },
        );
        let order: Vec<_> = candidates.iter().map(|c| c.entity.as_str()).collect();
        assert_eq!(order, ["c", "a", "b"]);
    }

    #[test]
    fn pools_follow_the_field_readers() {
        content_reader_core::test_game::enter();
        let pools = content(
            br#"
            @weight = 4
            default = {
                tank = {
                    pool = { weight = 0.1234567 models = { @weight fixed } }
                    pool = { weight = bad models_weight = x models = kept_out unknown = { a } }
                    other = { models = { skipped } }
                    pool = { models = { equal } }
                }
                tank_1 = scalar
            }
        "#,
        )
        .unwrap();
        let found = pools
            .lookup(&context(None, &[]), |_| Some(true), |_| true)
            .unwrap();
        let found: Vec<_> = found
            .iter()
            .map(|c| (c.entity.as_str(), c.weight))
            .collect();
        assert_eq!(
            found,
            [("equal", 100_000), ("fixed", 12_345), ("4", 12_345)]
        );
    }
    #[test]
    fn icons_come_from_pools_without_a_sub_unit_filter_by_icon_weight_last_first() {
        enter_pool_test_tables();
        let pools = content(
            br#"
            default = { tank = {
                pool = { weight = 5 icons = { low_a low_b } }
                pool = { sub_units = { light_armor } weight = 9 icons = { hq_only } }
                pool = { icons_weight = 7 weight = 1 icons = { high_a high_b missing } }
                pool = { models = { model_only } }
            } }
        "#,
        )
        .unwrap();
        let mut icon_context = context(None, &[]);
        icon_context.subunit = None;
        let exists = |name: &str| name != "missing";
        assert_eq!(
            pools.lookup_icon(&icon_context, |_| Some(true), exists),
            Ok(Some("high_b".to_owned()))
        );
        assert_eq!(
            pools.lookup_icon(
                &icon_context,
                |_| Some(true),
                |name| name.starts_with("low")
            ),
            Ok(Some("low_b".to_owned()))
        );
        let models = pools
            .lookup(&context(None, &[]), |_| Some(true), |_| true)
            .unwrap();
        assert_eq!(
            models.iter().map(|c| c.entity.as_str()).collect::<Vec<_>>(),
            ["model_only"]
        );
        let limited =
            content(b"default = { tank = { pool = { limit = { tag = GER } icons = { x } } } }")
                .unwrap();
        assert!(
            limited
                .lookup_icon(&icon_context, |_| Some(true), |_| true)
                .is_err()
        );
        assert!(
            limited
                .lookup(&context(None, &[]), |_| Some(true), |_| true)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn null_equipment_names_do_not_match_authored_empty_pools() {
        content_reader_core::test_game::enter();
        let pools = content(
            br#"default = {
                "" = { pool = { weight = 9 models = { null_model } icons = { null_icon } } }
                tank_1 = { pool = { models = { own_model } icons = { own_icon } } }
            }"#,
        )
        .unwrap();
        let mut null_archetype = context(None, &[]);
        null_archetype.archetype = None;
        null_archetype.subunit = None;
        assert_eq!(
            pools.lookup_icon(&null_archetype, |_| Some(true), |_| true),
            Ok(Some("own_icon".to_owned()))
        );
        let models = pools
            .lookup(&null_archetype, |_| Some(true), |_| true)
            .unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.entity.as_str())
                .collect::<Vec<_>>(),
            ["own_model"]
        );
        null_archetype.archetype = Some("");
        assert_eq!(
            pools.lookup_icon(&null_archetype, |_| Some(true), |_| true),
            Ok(Some("null_icon".to_owned()))
        );
        null_archetype.archetype = None;
        null_archetype.family = Some(("", None));
        assert_eq!(
            pools.lookup_icon(&null_archetype, |_| Some(true), |_| true),
            Ok(Some("null_icon".to_owned()))
        );
        null_archetype.family = None;
        null_archetype.equipment = "";
        assert_eq!(
            pools.lookup_icon(&null_archetype, |_| Some(true), |_| true),
            Ok(Some("null_icon".to_owned()))
        );
    }
}
