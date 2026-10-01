//! Authored equipment model pools, independent of the recording client's UI.
//!
//! `14191A090` supplies the variant owner's ideology/culture separately from
//! the country used for country/faction/continent pools. `1409F2B80` visits the
//! equipment and its authored family; `1409F0BB0`/`1409F13F0` visit each context
//! and archetype. `1409F1540` reverses pools, `1409F52D0` stably sorts descending
//! Fixed5 weight, and `1409F3010` reverses each model list and keeps the first
//! occurrence of each entity. No chosen model or eligible-pool cache is input.
//!
//! Each `graphic_db` file is read as `1409F6100` reads it ([`document`]): a
//! bucket is the `default` keyword, a continent `map/default.map` names, a
//! three-letter country tag `common/country_tags` declares (upper-cased), or
//! `FACTION_` and three characters; the game reports any other key and skips
//! its block.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    ResolvedLayers,
    fixed5::script_fixed5,
    parser::{
        self, Item, Operator, Value,
        native::{self, Shape},
    },
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
                if item.operator != Operator::Equals {
                    return Ok(Self::Unsupported(format!(
                        "{name} {}",
                        String::from_utf8_lossy(item.operator.text())
                    )));
                }
                // Only the scope-free equality forms are modelled, and a
                // built-in only on its keyword token: LoadKey (1405434F0)
                // gives a quoted key (token 0xF) no built-in, and whether a
                // scripted trigger of that name exists is not resolved here.
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
    /// The `graphic_db` file that declares the pool.
    file: String,
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

impl Names {
    /// The bucket a top-level key names (`1409F6100`), `None` for a key the
    /// game reports and skips.
    fn bucket(&self, key: &[u8], default: bool) -> Option<Bucket> {
        if default {
            return Some(Bucket::Default);
        }
        let key = std::str::from_utf8(parser::c_string(key)).ok()?;
        // FindTokenFromSsoString (1424A7060) and the continent entry's valid
        // flag (140AAE420, +0x28): the lexer's lookup folds case.
        let folded = key.to_ascii_lowercase();
        if self.continents.contains(&folded) {
            return Some(Bucket::Continent(folded));
        }
        if key.len() == 3 {
            // SsoStringToUpperInPlace, then FindCountryIndexByTag (14070F050).
            let tag = key.to_ascii_uppercase();
            return self
                .countries
                .contains(&tag)
                .then_some(Bucket::Country(tag));
        }
        // SplitStringOnChar on `_`: exactly `FACTION` and three characters,
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
    /// CEquipmentGraphicPoolTypeMap: each key through `1409F9EA0`.
    Bucket,
    /// `1409F9EA0`'s loop: `pool` (0x4B29) read when its value is `{`.
    Equipment,
    /// CEquipmentGraphicPool::vf4 (`1409F93C0`).
    Pool,
}

/// One `graphic_db` file through the native reader replay: `1409F6100`
/// loops to the end of the text; its buckets are CPersistent objects.
fn document(bytes: &[u8], names: &Names) -> io::Result<Vec<Item>> {
    // 1409F8C30 constructs each file's lexer in mode 1 (1424A6370).
    let read = native::read(
        bytes,
        native::Mode::SkipMark,
        Node::File,
        &mut |node, field| match node {
            Node::File => match names.bucket(&field.key.text, field.keyword_is("default")) {
                // ReadObjectFromParser (1424ABB40).
                Some(_) => Shape::Object(Node::Bucket),
                // RecordError (1424ACF90).
                None => Shape::Skip,
            },
            // Anything but `{` is RecordError.
            Node::Bucket => Shape::ObjectOrSkip(Node::Equipment),
            Node::Equipment if field.keyword_is("pool") => Shape::ObjectOrSkip(Node::Pool),
            Node::Equipment => Shape::Skip,
            Node::Pool => match field.keyword().as_deref() {
                // ReadFixed5 (1424ABB10) into the three weights (+0x60).
                Some(b"weight" | b"models_weight" | b"icons_weight") => Shape::One,
                // The lexer's tokens to `}`, `@` names resolved, only after `{`.
                Some(b"models" | b"icons") if field.opens() => Shape::RawList,
                // 1409F3710, 1409F3A70, 1409F33B0: the value slot's tokens to `}`.
                Some(b"ideologies" | b"cultures" | b"sub_units") => Shape::List,
                // The pool's CAndTrigger (+8, vtable +0x28).
                Some(b"limit") => Shape::Trigger,
                // ReportUnexpectedToken (1424AD100).
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
        // 1409F8C30 lists the directory as a database loader does
        // (enumerate_directory_wrapper 1424C73B0, no recursion, byte order);
        // it is covered, so every file's bytes are verified.
        for path in layers.directory_files("gfx/interface/equipmentdesigner/graphic_db", ".txt")? {
            document(&layers.read_uncovered(&path)?, names)
                .and_then(|items| content.read_document(&items, names, &path))
                .map_err(|error| invalid(format!("{path}: {error}")))?;
        }
        Ok(content)
    }

    /// One `graphic_db` document; for other modules' fixtures.
    #[cfg(test)]
    pub(crate) fn from_test_document(text: &[u8], names: &Names) -> io::Result<Self> {
        let mut content = Self::default();
        content.read_document(&document(text, names)?, names, "test")?;
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
                // CHashedString uses ASCII-folded FNV and _stricmp
                // (140BBA1B0 / 14253D9F8); entity identifiers below stay exact.
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
                        // ReadFixed5: a failed conversion keeps the slot.
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
                            // Any other value is RecordError, the list unread.
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
                                pool.limit.extend(Limit::parse(block(&field.value)?)?)
                            }
                            _ => {}
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

    /// Every pool limit predicate the lookup refuses, with the file and
    /// the bucket and equipment key its pool is under: a lookup that meets
    /// the pool fails.
    pub fn refusals(&self) -> Vec<(&str, String)> {
        fn walk<'a>(limits: &'a [Limit], out: &mut Vec<&'a str>) {
            for limit in limits {
                match limit {
                    Limit::Unsupported(name) => out.push(name),
                    Limit::All(inner) | Limit::Any(inner) | Limit::Not(inner) => walk(inner, out),
                    Limit::Dlc(_) => {}
                }
            }
        }
        let mut refusals = Vec::new();
        for (bucket, definitions) in &self.buckets {
            for (equipment, pools) in definitions {
                for pool in pools {
                    let mut names = Vec::new();
                    walk(&pool.limit, &mut names);
                    for name in names {
                        refusals.push((
                            pool.file.as_str(),
                            format!(
                                "{bucket:?} pool for {equipment}: unsupported empty-scope predicate {name}"
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
        content.read_document(&document(text, &names())?, &names(), "graphic_db/a.txt")?;
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
    fn buckets_accept_declared_tags_factions_and_continents_and_skip_the_rest() {
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
        // 1409F6100 upper-cases a three-letter key before the country lookup,
        // and reports and skips a key that names no bucket.
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
        // A quoted `"has_dlc"` is no built-in (LoadKey 1405434F0): undecided.
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
            b"default = { tank = { pool = { limit = { OR = { tag = GER has_dlc = a } } models = { x } } } }",
        )
        .unwrap();
        assert_eq!(
            nested.refusals(),
            [(
                "graphic_db/a.txt",
                "Default pool for tank: unsupported empty-scope predicate tag".to_owned()
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
    fn pools_follow_the_native_field_readers() {
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
        // ReadFixed5 truncates past five digits and keeps the slot when it
        // fails; `models` is read only from `{`; `@` names resolve.
        assert_eq!(
            found,
            [("equal", 100_000), ("fixed", 12_345), ("4", 12_345)]
        );
    }
}
