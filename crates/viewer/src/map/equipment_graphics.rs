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
    key_names::KeyNames,
    parser::{
        self, Item, Value,
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

/// Per-hour memo view; native selects anew on every lookup (1409F1540).
pub struct Limits<'a> {
    evaluate: &'a dyn Fn(&[Item], dashboard::triggers::Scopes) -> dashboard::triggers::Eval,
    values: &'a LimitValues,
}

/// A memo owned by the hour, never by a unit, country or content load.
#[derive(Default)]
pub struct LimitValues(std::cell::RefCell<BTreeMap<usize, dashboard::triggers::Eval>>);

impl<'a> Limits<'a> {
    pub fn new(
        values: &'a LimitValues,
        evaluate: &'a dyn Fn(&[Item], dashboard::triggers::Scopes) -> dashboard::triggers::Eval,
    ) -> Self {
        Self { evaluate, values }
    }

    fn holds(&self, pool: &Pool) -> Result<bool, String> {
        self.values
            .0
            .borrow_mut()
            .entry(pool.id)
            .or_insert_with(|| {
                // CTrigger::Check starts a fresh temp context even for
                // the empty scope (140540570 -> 1405354A0 -> 140544190).
                (self.evaluate)(&pool.limit, dashboard::triggers::Scopes::empty())
            })
            .clone()
            .decided()
    }
}

#[derive(Clone, Debug)]
struct Pool {
    id: usize,
    weight: i64,
    models_weight: i64,
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
fn document(bytes: &[u8], names: &Names, key_names: &KeyNames) -> io::Result<Vec<Item>> {
    // 1409F8C30 constructs each file's lexer in mode 1 (1424A6370).
    let read = native::read(
        bytes,
        native::Mode::SkipMark,
        native::Top::File(Node::File),
        Some(key_names),
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
        let key_names = layers.key_names()?;
        // 1409F8C30 lists the directory as a database loader does
        // (enumerate_directory_wrapper 1424C73B0, no recursion, byte order);
        // it is covered, so every file's bytes are verified.
        for path in layers.directory_files("gfx/interface/equipmentdesigner/graphic_db", ".txt")? {
            document(&layers.read_uncovered(&path)?, names, &key_names)
                .and_then(|items| content.read_document(&items, names, &path))
                .map_err(|error| invalid(format!("{path}: {error}")))?;
        }
        Ok(content)
    }

    /// One `graphic_db` document; for other modules' fixtures.
    #[cfg(test)]
    pub(crate) fn from_test_document(text: &[u8], names: &Names) -> io::Result<Self> {
        let mut content = Self::default();
        content.read_document(&document(text, names, &KeyNames::default())?, names, "test")?;
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
            for equipment in root.value.items().unwrap_or_default() {
                let Some(items) = equipment.value.items() else {
                    continue;
                };
                // CHashedString uses ASCII-folded FNV and _stricmp
                // (140BBA1B0 / 14253D9F8); entity identifiers below stay exact.
                let equipment_key = key(equipment)?.to_ascii_lowercase();
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
                                // CTrigger::LoadBody (1405431C0): a scalar
                                // adds no children, as the shared reader says.
                                pool.limit.extend(
                                    installed_content::triggers::body(&field.value).unwrap_or_default(),
                                );
                            }
                            _ => {}
                        }
                    }
                    // Icon-only pools contribute no model entries in native.
                    if !pool.models.is_empty() {
                        pool.id = self.pool_count;
                        self.pool_count += 1;
                        self.buckets
                            .entry(bucket.clone())
                            .or_default()
                            .entry(equipment_key.clone())
                            .or_default()
                            .push(pool);
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
        limits: &Limits<'_>,
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
                        if limits.holds(pool).map_err(|reason| {
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
pub(crate) mod tests {
    use super::*;

    pub(crate) fn with_world<T>(
        frame: &replay::frame::DecodedFrame,
        dlcs: &[&str],
        run: impl FnOnce(Limits<'_>) -> T,
    ) -> T {
        let dlcs = dlcs.iter().map(|s| (*s).to_owned()).collect();
        let aliases = std::sync::Arc::default();
        let names = installed_content::trigger_keys::TriggerNames {
            counts: Some(Default::default()),
            ..Default::default()
        };
        let world = dashboard::triggers::World::read(
            frame,
            dashboard::triggers::ContentFacts::content_only(&dlcs, &names, &aliases),
        );
        let evaluate = |items: &[Item], scopes| world.evaluate(items, scopes);
        run(Limits::new(&LimitValues::default(), &evaluate))
    }

    pub(crate) fn empty_frame() -> replay::frame::DecodedFrame {
        use replay::{
            frame::{DecodedFrame, TableView},
            rows::{TableId, TableRows},
        };
        DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Default::default(),
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
        }
    }

    fn country_frame(count: u32, cosmetic: &str) -> replay::frame::DecodedFrame {
        use replay::{
            frame::TableView,
            rows::{
                CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
                PoliticalPartyRow, TableId, TableRows,
            },
        };
        let mut frame = empty_frame();
        let tag = |i: u32| {
            vec![
                b'A' + (i / 676) as u8,
                b'A' + (i / 26 % 26) as u8,
                b'A' + (i % 26) as u8,
            ]
        };
        for (id, rows) in [
            (
                TableId::Countries,
                TableRows::Countries(
                    (0..count)
                        .map(|i| CountryRow {
                            id: i + 1,
                            canonical_slot: i,
                            tag: tag(i),
                            ..Default::default()
                        })
                        .collect(),
                ),
            ),
            (
                TableId::CountryLinks,
                TableRows::CountryLinks(
                    (0..count)
                        .map(|i| CountryLinkRow {
                            id: i + 1,
                            canonical_slot: i,
                        })
                        .collect(),
                ),
            ),
            (
                TableId::CountryCounterInputs,
                TableRows::CountryCounterInputs(
                    (0..count)
                        .map(|i| CountryCounterInputRow {
                            country: i + 1,
                            owned_state_count: 1,
                            ..Default::default()
                        })
                        .collect(),
                ),
            ),
            (
                TableId::CountryAppearance,
                TableRows::CountryAppearance(
                    (0..count)
                        .map(|i| CountryAppearanceRow {
                            country: i + 1,
                            name_tag: tag(i),
                            cosmetic_tag: cosmetic.as_bytes().to_vec(),
                            ..Default::default()
                        })
                        .collect(),
                ),
            ),
            (
                TableId::PoliticalParties,
                TableRows::PoliticalParties(
                    (0..count)
                        .map(|i| PoliticalPartyRow {
                            country: i + 1,
                            token: b"government".to_vec(),
                            is_ruling: 1,
                            ..Default::default()
                        })
                        .collect(),
                ),
            ),
        ] {
            *frame.tables[id.index()] = TableView::Complete(rows);
        }
        frame
    }

    #[test]
    fn pool_choice_reads_this_hours_world_and_keeps_unknown_named() {
        let pools = content(br#"default = { tank = {
            pool = { weight = 1 models = { fallback } }
            pool = { weight = 2 limit = { AAA = { has_cosmetic_tag = costume has_government = government } } models = { changed } }
        }}"#).unwrap();
        for (cosmetic, expected) in [("costume", "changed"), ("other", "fallback")] {
            with_world(&country_frame(5_000, cosmetic), &[], |limits| {
                for _ in 0..100 {
                    assert_eq!(
                        pools
                            .lookup(&context(None, &[]), &limits, |_| true)
                            .unwrap()[0]
                            .entity,
                        expected
                    );
                }
                // One result per pool, independent of unit count.
                assert_eq!(limits.values.0.borrow().len(), 2);
            });
        }
        let mut missing = country_frame(1, "costume");
        *missing.tables[replay::rows::TableId::CountryAppearance.index()] =
            replay::frame::TableView::Unavailable {
                reason: "not sampled".into(),
            };
        with_world(&missing, &[], |limits| {
            let why = pools
                .lookup(&context(None, &[]), &limits, |_| true)
                .unwrap_err();
            assert!(why.contains("CountryAppearance"), "{why}");
            assert!(why.contains("graphic_db/a.txt"), "{why}");
        });
    }

    #[test]
    fn pool_check_starts_a_fresh_variable_context() {
        let pools = content(
            br#"default = { tank = {
            pool = { limit = { check_variable = { global.pool_ready = 1 } } models = { ready } }
        }}"#,
        )
        .unwrap();
        let mut frame = empty_frame();
        use replay::{
            frame::TableView,
            rows::{ScriptVariableEntry, ScriptVariablesRow, TableRows, tables},
        };
        *frame.tables[replay::rows::TableId::ScriptVariables.index()] =
            TableView::Complete(TableRows::from_rows::<tables::ScriptVariables>(vec![
                ScriptVariablesRow {
                    scope_kind: 0,
                    scope_id: 0,
                    entries: vec![ScriptVariableEntry {
                        name: b"pool_ready".to_vec(),
                        value_raw: 100_000,
                    }],
                },
            ]));
        with_world(&frame, &[], |limits| {
            // Check clears prior temps (140540570 -> 140544190), so this
            // operand reaches the recorded global variable store.
            assert_eq!(
                pools
                    .lookup(&context(None, &[]), &limits, |_| true)
                    .unwrap()[0]
                    .entity,
                "ready"
            );
        });
        *frame.tables[replay::rows::TableId::ScriptVariables.index()] =
            replay::frame::TableView::Unavailable {
                reason: "not sampled".into(),
            };
        with_world(&frame, &[], |limits| {
            let why = pools
                .lookup(&context(None, &[]), &limits, |_| true)
                .unwrap_err();
            assert!(why.contains("ScriptVariables"), "{why}");
        });
    }

    #[test]
    #[ignore = "release timing; run without concurrent builds"]
    fn pool_limits_builder_budget() {
        let mut text = String::from("default = { tank = {");
        for i in 0..510 {
            text.push_str(&format!("pool = {{ limit = {{ AAA = {{ has_cosmetic_tag = costume has_government = government }} }} models = {{ m{i} }} }}"));
        }
        text.push_str("} }");
        let content = content(text.as_bytes()).unwrap();
        let pools: Vec<_> = content
            .buckets
            .values()
            .flat_map(|bucket| bucket.values())
            .flatten()
            .collect();
        for countries in [1_500, 5_000] {
            let frame = country_frame(countries, "costume");
            let start = std::time::Instant::now();
            let hours = 128;
            for _ in 0..hours {
                with_world(&frame, &[], |limits| {
                    for pool in &pools {
                        assert!(std::hint::black_box(limits.holds(pool).unwrap()));
                    }
                    assert_eq!(limits.values.0.borrow().len(), pools.len());
                });
            }
            println!(
                "POOL_LIMITS countries={countries} pools={} hours={hours} ms_per_hour={:.3}",
                pools.len(),
                start.elapsed().as_secs_f64() * 1000.0 / f64::from(hours)
            );
        }
    }

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
        let found = with_world(&empty_frame(), &[], |limits| {
            pools.lookup(&context(Some("GER"), &factions), &limits, |_| true)
        })
        .unwrap();
        let entities: Vec<_> = found.iter().map(|c| c.entity.as_str()).collect();
        // Weight order; reversed model lists; equal weights keep visit order.
        assert_eq!(
            entities,
            ["ger", "allied", "base_b", "base_a", "continental"]
        );
        // No country context visits only the default bucket.
        let found = with_world(&empty_frame(), &[], |limits| {
            pools.lookup(&context(None, &[]), &limits, |_| true)
        })
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
        let found = with_world(&empty_frame(), &[], |limits| {
            pools.lookup(&context(Some("GER"), &factions), &limits, |_| true)
        })
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
        let found = with_world(&empty_frame(), &["No Step Back"], |limits| {
            pools.lookup(&context(None, &[]), &limits, |_| true)
        })
        .unwrap();
        let entities: BTreeSet<_> = found.iter().map(|c| c.entity.as_str()).collect();
        assert_eq!(entities, BTreeSet::from(["right", "dlc", "either"]));
        // Scope-masked predicates are false before they read any input.
        let scoped =
            content(b"default = { tank = { pool = { limit = { tag = GER } models = { x } } } }")
                .unwrap();
        assert!(
            with_world(&empty_frame(), &[], |limits| {
                scoped.lookup(&context(None, &[]), &limits, |_| true)
            })
            .unwrap()
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
        let found = with_world(&empty_frame(), &[], |limits| {
            pools.lookup(&context(None, &[]), &limits, |_| true)
        })
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
