//! Authored equipment model pools, independent of the recording client's UI.
//!
//! The variant owner's ideology/culture are separate from the country used
//! for country/faction/continent pools. Visit equipment and its authored
//! family, then each context and archetype. Reverse pools and stably sort
//! by descending fixed5 weight; reverse each model list and keep the first
//! occurrence of each entity. No chosen model or eligible-pool cache is input.
//!
//! Each `graphic_db` file ([`document`]) has buckets for `default`, a continent
//! named by `map/default.map`, a declared three-letter country tag (upper-
//! cased), or `FACTION_` and three characters. Report other keys and skip
//! their blocks.

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
        decimal::Decimal,
        native::{self, Shape},
    },
};

pub mod aircraft;
pub mod context;
pub mod countries;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub entity: String,
    /// Signed native Fixed5 weight, shared with conventional entity candidates.
    pub weight: i64,
}

/// Legacy insertion is BEFORE equal scores, while authored graphic
/// insertion is AFTER equals. A duplicate never replaces the existing
/// entity, even if its later score would be higher.
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

/// Every optional value here means known absence, never a missing replay row.
/// Callers must establish complete simulation inputs before constructing this.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// A real equipment name, including an authored empty name.
    pub equipment: &'a str,
    /// None for the null object; Some("") is a real empty name.
    /// Their cached hashes differ, so they cannot match the same authored key.
    pub archetype: Option<&'a str>,
    /// Family equipment key and its optional archetype, if different from equipment.
    pub family: Option<(&'a str, Option<&'a str>)>,
    pub subunit: Option<&'a str>,
    /// Ruling ideology group of the variant owner, not its leader's subtype.
    pub ideology: Option<&'a str>,
    /// Graphical culture of the variant owner.
    pub culture: Option<&'a str>,
    /// The separately supplied country context, which can differ from the owner.
    pub country: Option<&'a str>,
    /// Native faction membership order, including the leader first.
    pub faction_tags: &'a [String],
    /// Continent of original capital if nonzero, otherwise current capital.
    pub capital_continent: Option<&'a str>,
}

/// A `graphic_db` file's top-level key, which context a lookup visits it in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Default,
    /// A declared country tag, upper-cased.
    Country(String),
    /// `FACTION_` and three characters, the three as written.
    Faction(String),
    /// A continent of `map/default.map`, folded.
    Continent(String),
}

impl std::fmt::Display for Bucket {
    /// The key as the lookup reads it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Country(tag) => f.write_str(tag),
            Self::Faction(tag) => write!(f, "FACTION_{tag}"),
            Self::Continent(continent) => f.write_str(continent),
        }
    }
}

/// One model a graphics pool lists ([`Content::models`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolModel<'a> {
    pub bucket: &'a Bucket,
    /// The equipment key the pool is declared under, ASCII-folded as lookups
    /// compare it.
    pub equipment: &'a str,
    /// The `graphic_db` file that declares the pool.
    pub file: &'a str,
    /// The model's entity or mesh name, exactly as authored.
    pub model: &'a str,
}

/// A pool `limit`, evaluated in an empty scope. Only scope-free predicates
/// can be decided; the rest stay [`Limit::Unsupported`] and fail the lookup
/// that meets them instead of dropping the pool.
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
                // Only scope-free equality forms are modelled. Built-ins require a
                // keyword token; quoted keys have no built-in, and scripted triggers
                // of those names are not resolved here.
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

    /// `Err` names the first predicate that cannot be decided.
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
            // NOT is NOR over its children.
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

/// Shared scope-free DLC predicates. Ambient admission also builds an
/// empty scope, so it uses the same supported conjunction forms.
pub fn empty_scope_condition(
    items: &[Item],
    has_dlc: impl Fn(&str) -> Option<bool>,
) -> io::Result<bool> {
    all(&Limit::parse(items)?, &has_dlc).map_err(io::Error::other)
}

/// A pool's authored limit, evaluated in an empty event scope. The map supplies
/// its recorded world; content-only consumers decide the scope-free DLC forms.
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
    /// The `graphic_db` file that declares the pool.
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
        // Zero is the native fallback sentinel, not an explicit zero score.
        if self.models_weight == 0 {
            self.weight
        } else {
            self.models_weight
        }
    }

    /// The icon sort weight: `icons_weight`, or `weight` when that is zero.
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

/// The names a bucket key can be.
pub struct Names {
    /// Every tag `common/country_tags` declares, as written.
    pub countries: BTreeSet<String>,
    /// `map/default.map`'s continents, folded.
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

    /// The bucket a top-level key names; None marks a key the game reports
    /// and skips.
    fn bucket(&self, key: &[u8], default: bool) -> Option<Bucket> {
        if default {
            return Some(Bucket::Default);
        }
        let key = std::str::from_utf8(parser::c_string(key)).ok()?;
        // Continent names must be valid entries; lexer lookup folds their case.
        let folded = key.to_ascii_lowercase();
        if self.continents.contains(&folded) {
            return Some(Bucket::Continent(folded));
        }
        if key.len() == 3 {
            // Uppercase the three-letter key before resolving the country tag.
            let tag = key.to_ascii_uppercase();
            return self
                .countries
                .contains(&tag)
                .then_some(Bucket::Country(tag));
        }
        // Split on `_`: exactly `FACTION` and three characters,
        // hashed as written (FNV-1a), whether or not a country has the tag.
        match key.split('_').collect::<Vec<_>>()[..] {
            ["FACTION", tag] if tag.len() == 3 => Some(Bucket::Faction(tag.to_owned())),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    /// Equipment keys within one context bucket.
    Bucket,
    /// Equipment fields read `pool` only when its value opens a block.
    Equipment,
    /// Fields within one authored graphics pool.
    Pool,
}

/// Read one `graphic_db` file through the persistent reader. The file
/// loop runs to the end of the text and reads buckets as objects.
fn document(bytes: &[u8], names: &Names, key_names: &KeyNames) -> io::Result<Vec<Item>> {
    // Skip the leading mark when constructing each file's lexer.
    let read = native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Node::File),
        Some(key_names),
        &mut |node, field| match node {
            Node::File => match names.bucket(&field.key.text, field.keyword_is("default")) {
                // Read a recognized bucket as an object.
                Some(_) => Shape::Object(Node::Bucket),
                // Unknown buckets are reported and skipped.
                None => Shape::Skip,
            },
            // Anything but `{` reports an error.
            Node::Bucket => Shape::ObjectOrSkip(Node::Equipment),
            Node::Equipment if field.keyword_is("pool") => Shape::ObjectOrSkip(Node::Pool),
            Node::Equipment => Shape::Skip,
            Node::Pool => match field.keyword().as_deref() {
                // Read each of the three weights as fixed5.
                Some(b"weight" | b"models_weight" | b"icons_weight") => Shape::One,
                // The lexer's tokens to `}`, `@` names resolved, only after `{`.
                Some(b"models" | b"icons") if field.opens() => Shape::RawList,
                // Read the value slot's tokens through `}`.
                Some(b"ideologies" | b"cultures" | b"sub_units") => Shape::List,
                // The pool's limit is a conjunction trigger.
                Some(b"limit") => Shape::Trigger,
                // Report unexpected fields and skip their values.
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
        // List direct files in filename byte order. Covered reads verify every
        // file's bytes against the recorded stack.
        for path in layers.directory_files("gfx/interface/equipmentdesigner/graphic_db", ".txt")? {
            document(&layers.read_uncovered(&path)?, names, &key_names)
                .and_then(|items| content.read_document(&items, names, &path))
                .map_err(|error| invalid(format!("{path}: {error}")))?;
        }
        Ok(content)
    }

    /// Authored texts through the same native reader as files, for fixtures.
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
                // Equipment keys use ASCII-folded hashing and comparison; entity
                // identifiers below stay exact.
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
                        // A failed fixed5 conversion keeps the slot.
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
                            // Any other value reports an error and leaves the list unread.
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
                                // A scalar trigger body adds no children, as the shared reader says.
                                pool.limit.extend(
                                    crate::triggers::body(&field.value).unwrap_or_default(),
                                );
                            }
                            _ => {}
                        }
                    }
                    // Each lookup visits only pools holding its kind of artwork.
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

    /// Every model every pool lists, whatever context or limit would visit it:
    /// by bucket and folded equipment key, each key's pools in declaration
    /// order and each pool's models as authored. A model listed several times
    /// is listed each time.
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

    /// Every pool limit predicate the lookup refuses, with its file, bucket,
    /// equipment, authored pool ordinal and limit-tree child path. Each `[n]`
    /// is a 1-based sibling position, including supported predicates.
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

    /// Return the native graphic candidates, ready for legacy insertion.
    /// Unknown relevant predicates fail this lookup rather than silently
    /// discarding a potentially higher-priority model.
    pub fn lookup(
        &self,
        context: &Context<'_>,
        has_dlc: impl Fn(&str) -> Option<bool>,
        entity_exists: impl Fn(&str) -> bool,
    ) -> Result<Vec<Candidate>, String> {
        self.lookup_with_limits(context, &has_dlc, entity_exists)
    }

    /// Model candidates with the caller's evaluation of each authored limit.
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
                // Deduplicate candidates before checking whether their assets exist.
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

    /// Collect context, equipment and family pools, stably sort descending
    /// model weight, then scan each model list backwards. Return the first
    /// declared name directly without inserting or deduplicating legacy
    /// candidates. A refused declaration still exists.
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

    /// The icon the production view shows: pools matching `context` that have
    /// icons and no `sub_units` filter (the context's `subunit` should be None),
    /// stably sorted by descending `icons_weight`, or `weight` when zero. Try
    /// each pool's icons last to first; the first sprite or texture path that
    /// `exists` wins. Err names an undecidable limit, leaving the icon unknown.
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

    /// The pools `context` visits that `wanted` keeps, in the native visit order:
    /// for the equipment, then its family, each country, faction, continent and
    /// default bucket; the equipment's then its archetype's pools, last first.
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
                // Repeat this visit when keys coincide. The null equipment object uses
                // a sentinel hash; a real empty name uses the ordinary empty-string hash.
                // Only the latter can match an authored empty key.
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

/// A list's entries as stored text, `@` names already resolved.
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

/// Authored fixed5 decimals keep the first five fractional digits. Save
/// Decimal refuses lossy input, so authored digits past the fifth are
/// truncated before it is used.
pub fn authored_fixed5(bytes: &[u8]) -> io::Result<i64> {
    let bytes = if let Some(dot) = bytes.iter().position(|byte| *byte == b'.')
        && bytes.len() > dot + 6
        && bytes[dot + 1..].iter().all(u8::is_ascii_digit)
    {
        &bytes[..dot + 6]
    } else {
        bytes
    };
    Ok(Decimal::parse(bytes)?.raw)
}

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

    #[test]
    fn the_graphic_db_trigger_reader_keeps_content_names() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        // Collection precedes selection, weights are stable, strings run
        // backwards, and this selector does not deduplicate repeated names.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        // Weight order; reversed model lists; equal weights keep visit order.
        assert_eq!(
            entities,
            ["ger", "allied", "base_b", "base_a", "continental"]
        );
        // No country context visits only the default bucket.
        let found = pools
            .lookup(&context(None, &[]), |_| Some(true), |_| true)
            .unwrap();
        assert_eq!(found.len(), 2);
        // Uppercase a three-letter key before country lookup; report and skip
        // keys that name no bucket.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        // A quoted `"has_dlc"` has no built-in token, so its result is undecided.
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
        // Each refused predicate, nested ones included, is named with its file.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        // Fixed5 conversion truncates past five digits and keeps the slot when it
        // fails; `models` is read only from `{`; `@` names resolve.
        assert_eq!(
            found,
            [("equal", 100_000), ("fixed", 12_345), ("4", 12_345)]
        );
    }
    #[test]
    fn icons_come_from_pools_without_a_sub_unit_filter_by_icon_weight_last_first() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        // icons_weight 7 beats weight 5; within it, last to first, the first that exists.
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
        // Models ignore icon-only pools and icons ignore model-only pools.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
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
        // A real empty archetype and family name retain their ordinary authored
        // name hash, distinct from a null equipment object's sentinel.
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
