//! Authored equipment model pools, independent of the recording client's UI.
//!
//! `14191A090` supplies the variant owner's ideology/culture separately from
//! the country used for country/faction/continent pools. `1409F2B80` visits the
//! equipment and its authored family; `1409F0BB0`/`1409F13F0` visit each context
//! and archetype. `1409F1540` reverses pools, `1409F52D0` stably sorts descending
//! Fixed5 weight, and `1409F3010` reverses each model list and keeps the first
//! occurrence of each entity. No chosen model or eligible-pool cache is input.
//!
//! A pool bucket is `default`, a continent `map/default.map` names, a country
//! tag `common/country_tags` declares, or `FACTION_` and such a tag; anything
//! else is refused rather than guessed.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    ResolvedLayers,
    parser::{self, Item, Value},
    save::Decimal,
};

pub mod bridge;
pub mod context;
pub mod legacy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub entity: String,
    /// Signed native Fixed5 weight, shared with conventional entity candidates.
    pub weight: i64,
}

/// The native legacy insertion is BEFORE equal scores (`140B884F0`), unlike
/// authored graphic insertion AFTER equals (`14191A910`). A duplicate never
/// replaces the existing entity, even if the later score would be higher.
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
    pub equipment: &'a str,
    pub archetype: &'a str,
    /// Family equipment key and its archetype, if different from equipment.
    pub family: Option<(&'a str, &'a str)>,
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
    Default,
    Country(String),
    Faction(String),
    Continent(String),
}

/// A pool `limit`. The pools use an empty CEventScope (`1409F1540`), so only
/// scope-free predicates can be decided; the rest stay [`Limit::Unsupported`]
/// and fail the lookup that meets them instead of dropping the pool.
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
                // A comparison other than `=` arrives as an unkeyed operator
                // item; only the scope-free equality forms are modelled.
                Ok(match (name.to_ascii_lowercase().as_str(), &item.value) {
                    ("has_dlc", value) => match value.scalar_str() {
                        Some(dlc) if !dlc.starts_with('@') => Self::Dlc(dlc.to_owned()),
                        _ => Self::Unsupported(format!("has_dlc {value:?}")),
                    },
                    ("and", Value::Block(inner)) => Self::All(Self::parse(inner)?),
                    ("or", Value::Block(inner)) => Self::Any(Self::parse(inner)?),
                    ("not", Value::Block(inner)) => Self::Not(Self::parse(inner)?),
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

/// Shared scope-free DLC predicates. Ambient admission140B47AF0 also builds an
/// empty event scope; unresolved predicates remain named errors.
pub(super) fn empty_scope_condition(
    items: &[Item],
    has_dlc: impl Fn(&str) -> Option<bool>,
) -> io::Result<bool> {
    all(&Limit::parse(items)?, &has_dlc).map_err(io::Error::other)
}

#[derive(Clone, Debug)]
struct Pool {
    weight: i64,
    models_weight: i64,
    models: Vec<String>,
    subunits: BTreeSet<String>,
    ideologies: BTreeSet<String>,
    cultures: BTreeSet<String>,
    limit: Vec<Limit>,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            weight: 100_000,
            models_weight: 0,
            models: Vec::new(),
            subunits: BTreeSet::new(),
            ideologies: BTreeSet::new(),
            cultures: BTreeSet::new(),
            limit: Vec::new(),
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
}

/// The names a bucket key can be.
pub struct Names {
    /// Every tag `common/country_tags` declares, as written.
    pub countries: BTreeSet<String>,
    /// `map/default.map`'s continents, folded.
    pub continents: BTreeSet<String>,
}

impl Content {
    pub fn load(layers: &ResolvedLayers, names: &Names) -> io::Result<Self> {
        let mut content = Self::default();
        // Recorded (layer, path) order with replace_path; the directory is
        // covered, so every file's bytes are verified.
        for path in
            layers.descriptor_paths("gfx/interface/equipmentdesigner/graphic_db", &[".txt"])?
        {
            let document = parser::parse(&layers.read_uncovered(&path)?)?;
            content
                .read_document(&document, names)
                .map_err(|error| invalid(format!("{path}: {error}")))?;
        }
        Ok(content)
    }

    /// One `graphic_db` document; for other modules' fixtures.
    #[cfg(test)]
    pub(crate) fn from_test_document(text: &[u8], names: &Names) -> io::Result<Self> {
        let mut content = Self::default();
        content.read_document(&parser::parse(text)?, names)?;
        Ok(content)
    }

    fn read_document(&mut self, document: &parser::Document, names: &Names) -> io::Result<()> {
        for root in &document.items {
            let name = key(root)?;
            let bucket = if name.eq_ignore_ascii_case("default") {
                Bucket::Default
            } else if names.continents.contains(&name.to_ascii_lowercase()) {
                Bucket::Continent(name.to_ascii_lowercase())
            } else if names.countries.contains(name) {
                Bucket::Country(name.to_owned())
            } else if let Some(tag) = name.strip_prefix("FACTION_")
                && names.countries.contains(tag)
            {
                Bucket::Faction(tag.to_owned())
            } else {
                return Err(invalid(format!(
                    "unsupported equipment graphics context {name:?}"
                )));
            };
            let definitions = self.buckets.entry(bucket).or_default();
            for equipment in block(&root.value)? {
                let name = key(equipment)?;
                // CHashedString uses ASCII-folded FNV and _stricmp
                // (140BBA1B0 / 14253D9F8); entity identifiers below stay exact.
                let pools = definitions.entry(name.to_ascii_lowercase()).or_default();
                for item in block(&equipment.value)? {
                    if !key(item)?.eq_ignore_ascii_case("pool") {
                        return Err(invalid(
                            "equipment graphics definition requires pool blocks",
                        ));
                    }
                    let mut pool = Pool::default();
                    for field in block(&item.value)? {
                        match key(field)?.to_ascii_lowercase().as_str() {
                            "weight" => pool.weight = fixed(&field.value)?,
                            "models_weight" => pool.models_weight = fixed(&field.value)?,
                            "icons_weight" => {
                                fixed(&field.value)?;
                            }
                            "models" => pool.models.extend(strings(&field.value)?),
                            "icons" => {
                                strings(&field.value)?;
                            }
                            "sub_units" => pool.subunits.extend(folded_strings(&field.value)?),
                            "ideologies" => pool.ideologies.extend(folded_strings(&field.value)?),
                            "cultures" => pool.cultures.extend(folded_strings(&field.value)?),
                            "limit" => pool.limit.extend(Limit::parse(block(&field.value)?)?),
                            other => {
                                return Err(invalid(format!(
                                    "unsupported equipment graphics pool field {other:?}"
                                )));
                            }
                        }
                    }
                    // Icon-only pools contribute no model entries in native.
                    if !pool.models.is_empty() {
                        pools.push(pool);
                    }
                }
            }
        }
        Ok(())
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
                // Native deliberately repeats this visit when keys coincide.
                for name in [equipment, archetype] {
                    let Some(pools) = definitions.get(&name.to_ascii_lowercase()) else {
                        continue;
                    };
                    for pool in pools.iter().rev().filter(|pool| pool.matches(context)) {
                        if all(&pool.limit, &has_dlc).map_err(|reason| {
                            format!("equipment graphics pool for {name}: {reason}")
                        })? {
                            matches.push(pool);
                        }
                    }
                }
            }
        }
        matches.sort_by_key(|pool| std::cmp::Reverse(pool.weight()));
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for pool in matches {
            for entity in pool.models.iter().rev() {
                // `1409F3010` deduplicates before `14191A090` checks assets.
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
            if item.key.is_some() {
                return Err(invalid("equipment graphics list contains a keyed entry"));
            }
            let text = item
                .value
                .scalar_str()
                .ok_or_else(|| invalid("equipment graphics list contains a non-string"))?;
            if text.starts_with('@') {
                return Err(invalid(
                    "equipment graphics script variables are unsupported",
                ));
            }
            Ok(text.to_owned())
        })
        .collect()
}

fn fixed(value: &Value) -> io::Result<i64> {
    authored_fixed5(
        value
            .scalar()
            .ok_or_else(|| invalid("equipment graphics weight is not a scalar"))?,
    )
}

/// An authored decimal as the engine reads it: CReader::ReadFixed5 ->
/// 1424B0480 keeps the first five fractional digits. Save Decimal refuses
/// lossy input, so authored digits past the fifth are truncated first.
pub(crate) fn authored_fixed5(bytes: &[u8]) -> io::Result<i64> {
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
        content.read_document(&parser::parse(text).unwrap(), &names())?;
        Ok(content)
    }

    fn context<'a>(country: Option<&'a str>, factions: &'a [String]) -> Context<'a> {
        Context {
            equipment: "tank_1",
            archetype: "tank",
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
    fn buckets_accept_declared_tags_factions_and_continents_and_refuse_the_rest() {
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
        for rejected in [&b"NOPE = {}"[..], b"FACTION_XX = {}", b"africa = {}"] {
            assert!(content(rejected).is_err());
        }
    }

    #[test]
    fn filters_and_dlc_limits_decide_pools_and_unknown_predicates_fail() {
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
        assert!(
            scoped
                .lookup(&context(None, &[]), |_| Some(true), |_| true)
                .is_err()
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
        assert_eq!(
            fixed(&Value::Scalar(b"0.1234567".to_vec())).unwrap(),
            12_345
        );
    }
}
