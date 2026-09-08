mod limit;
pub use limit::ItemPoolLimit;

use portable_content_manifest::paradox_text::{Scalar, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredGraphicPool<L> {
    pub weights_raw: [i64; 3],
    pub icons: Vec<String>,
    pub models: Vec<String>,
    pub limit: L,
    sub_units: BTreeSet<String>,
    ideologies: BTreeSet<String>,
    cultures: BTreeSet<String>,
}

impl<L> AuthoredGraphicPool<L> {
    pub fn parse(value: &Value, parse_limit: impl FnOnce(&Value) -> Result<L, String>) -> Result<Self, String> {
        let fields = value.as_object().ok_or("graphic pool is not an object")?;
        let mut weights_raw = [100_000, 0, 0];
        let mut icons = Vec::new();
        let mut models = Vec::new();
        let mut limit = Vec::new();
        let mut sub_units = BTreeSet::new();
        let mut ideologies = BTreeSet::new();
        let mut cultures = BTreeSet::new();
        for field in fields {
            let key = field.key.as_deref().ok_or("graphic pool has an unnamed field")?;
            if key.starts_with('@') { continue; }
            if field.key_is_quoted {
                return Err(format!("graphic pool has quoted field keyword {key:?}"));
            }
            let keyword = key.to_ascii_lowercase();
            match keyword.as_str() {
                "weight" | "icons_weight" | "models_weight" => {
                    let text = field.value.as_scalar().ok_or_else(|| format!("graphic pool {key} is not scalar"))?;
                    let raw = crate::authored_fixed::parse_fixed5(text)
                        .ok_or_else(|| format!("graphic pool {key} is not a fixed-point operand"))?;
                    weights_raw[match keyword.as_str() { "icons_weight" => 1, "models_weight" => 2, _ => 0 }] = raw;
                }
                "icons" => icons.extend(tokens(&field.value, key)?),
                "models" => models.extend(tokens(&field.value, key)?),
                "limit" => limit.extend(field.value.as_object().ok_or("graphic pool limit is not an object")?.iter().cloned()),
                "sub_units" => sub_units.extend(tokens(&field.value, key)?),
                "ideologies" => ideologies.extend(tokens(&field.value, key)?),
                "cultures" => cultures.extend(tokens(&field.value, key)?),
                _ => return Err(format!("graphic pool unsupported field {key:?}")),
            }
        }
        Ok(Self { weights_raw, icons, models, limit: parse_limit(&Value::Object(limit))?, sub_units, ideologies, cultures })
    }

    pub fn matches(&self, matches_limit: impl FnOnce(&L) -> Result<bool, String>,
        sub_unit: impl FnOnce() -> Result<String, String>,
        ideology: impl FnOnce() -> Result<String, String>,
        culture: impl FnOnce() -> Result<String, String>,
    ) -> Result<bool, String> {
        if !matches_limit(&self.limit)? { return Ok(false); }
        if !self.sub_units.is_empty() && !self.sub_units.contains(&sub_unit()?) { return Ok(false); }
        if !self.ideologies.is_empty() && !self.ideologies.contains(&ideology()?) { return Ok(false); }
        if !self.cultures.is_empty() && !self.cultures.contains(&culture()?) { return Ok(false); }
        Ok(true)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredGraphicPoolTypeMap<L> {
    rows: BTreeMap<String, Result<Vec<AuthoredGraphicPool<L>>, String>>,
}

impl<L> Default for AuthoredGraphicPoolTypeMap<L> {
    fn default() -> Self { Self { rows: BTreeMap::new() } }
}

impl<L> AuthoredGraphicPoolTypeMap<L> {
    pub fn append(&mut self, value: &Value, parse_limit: impl Fn(&Value) -> Result<L, String>) -> Result<(), String> {
        let entries = value.as_object().ok_or("graphic type map is not an object")?;
        if entries.iter().any(|entry| entry.key.is_none()) {
            return Err("graphic type has no equipment token".into());
        }
        for entry in entries {
            let token = entry.key.as_deref().expect("checked equipment token");
            if token.starts_with('@') { continue; }
            let slot = self.rows.entry(token.to_owned()).or_insert_with(|| Ok(Vec::new()));
            let Ok(rows) = slot else { continue; };
            let parsed = (|| {
                let pools = entry.value.as_object().ok_or_else(|| format!("graphic type {token:?} is not an object"))?;
                let mut additions = Vec::new();
                for pool in pools {
                    if pool.key.as_deref().is_some_and(|key| key.starts_with('@')) { continue; }
                    if pool.key_is_quoted {
                        return Err(format!("graphic type {token:?} has quoted pool keyword {:?}", pool.key));
                    }
                    if !pool.key.as_deref().is_some_and(|key| key.eq_ignore_ascii_case("pool")) {
                        return Err(format!("graphic type {token:?} has unsupported pool field {:?}", pool.key));
                    }
                    let row = AuthoredGraphicPool::parse(&pool.value, &parse_limit)?;
                    if !row.icons.is_empty() || !row.models.is_empty() { additions.push(row); }
                }
                Ok(additions)
            })();
            match parsed {
                Ok(additions) => rows.extend(additions),
                Err(error) => *slot = Err(error),
            }
        }
        Ok(())
    }

    pub fn matching_rows(&self, token: &str, matches_limit: impl Fn(&L) -> Result<bool, String>,
        sub_unit: impl Fn() -> Result<String, String>,
        ideology: impl Fn() -> Result<String, String>,
        culture: impl Fn() -> Result<String, String>,
    ) -> Result<Vec<&AuthoredGraphicPool<L>>, String> {
        let mut rows = Vec::new();
        if let Some(pools) = self.rows.get(token) {
            // 1409f1540 visits the last appended pool first before the global stable sort.
            for pool in pools.as_ref().map_err(Clone::clone)?.iter().rev() {
                if pool.matches(&matches_limit, &sub_unit, &ideology, &culture)? {
                    rows.push(pool);
                }
            }
        }
        Ok(rows)
    }
}

fn tokens(value: &Value, field: &str) -> Result<Vec<String>, String> {
    value.as_object().ok_or_else(|| format!("graphic pool {field} is not a list"))?.iter().map(|entry| {
        if entry.key.is_some() { return Err(format!("graphic pool {field} has a keyed list entry")); }
        let token = entry.value.as_scalar().ok_or_else(|| format!("graphic pool {field} has a nonscalar entry"))?;
        if matches!(&entry.value, Value::Scalar(Scalar::Bare(_))) && token.starts_with('@') {
            return Err(format!("graphic pool {field} has unresolved variable {token:?}"));
        }
        Ok(token.to_owned())
    }).collect()
}


use crate::authored_equipment::AuthoredEquipmentDefinition;
use replay_core::designer_icon_context::{DesignerCapitalSlotObservation, CapitalSlotOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemPoolKey<'a> {
    Country(&'a str),
    FactionMember(&'a str),
    CapitalRegion(u32),
    Default,
}

pub fn collect_country_pools<T>(
    frame: &replay_core::WorldFrame,
    creator: &str,
    mut matching_rows: impl FnMut(ItemPoolKey<'_>) -> Result<Vec<T>, String>,
    capital_region: impl FnOnce(DesignerCapitalSlotObservation) -> Result<Option<u32>, String>,
) -> Result<Vec<T>, String> {
    fn append<T>(rows: &mut Vec<T>, mut next: Vec<T>) -> Result<(), String> {
        rows.try_reserve(next.len()).map_err(|_| "XUI13 item pool row allocation refused".to_owned())?;
        rows.append(&mut next);
        Ok(())
    }
    frame.country_identity_for_tag(creator)
        .map_err(|reason| format!("XUI13 item pool creator: {reason}"))?;
    let mut rows = matching_rows(ItemPoolKey::Country(creator))?;

    // 1409f1540 appends through its callback but returns zero; 1409f0bb0 continues every pool.
    let factions = match frame.ordered_faction_roster()
        .map_err(|reason| format!("XUI13 item pool faction order: {reason:?}"))? {
        replay_core::Leaf::Measured(rows) => rows,
        other => return Err(format!("XUI13 item pool faction order: {other:?}")),
    };
    if let Some(faction) = factions.iter().find(|row| row.members.iter().any(|member| member.tag == creator)) {
        for member in &faction.members {
            append(&mut rows, matching_rows(ItemPoolKey::FactionMember(&member.tag))?)?;
        }
    }

    let observation = frame.designer_capital_slot(creator)
        .map_err(|reason| format!("XUI13 item pool runtime GS state-slot: {reason:?}"))?;
    match observation.outcome() {
        CapitalSlotOutcome::NonPositive | CapitalSlotOutcome::OutOfRange { .. } | CapitalSlotOutcome::NullSlot { .. } => {}
        CapitalSlotOutcome::Present { .. } => {
            if let Some(region) = capital_region(observation)? {
                append(&mut rows, matching_rows(ItemPoolKey::CapitalRegion(region))?)?;
            }
        }
        outcome => return Err(format!("XUI13 item pool runtime GS state-slot: {outcome:?}")),
    }
    append(&mut rows, matching_rows(ItemPoolKey::Default)?)?;
    Ok(rows)
}

#[derive(Clone, Copy, Debug)]
pub struct ItemPoolDefinition<'a> {
    pub definition: &'a AuthoredEquipmentDefinition,
    pub archetype: &'a AuthoredEquipmentDefinition,
}

pub fn collect_equipment_pools<'a, T>(
    frame: &replay_core::WorldFrame,
    creator: &str,
    definition: ItemPoolDefinition<'a>,
    graphics_source: impl FnOnce() -> Result<ItemPoolDefinition<'a>, String>,
    mut matching_rows: impl FnMut(ItemPoolKey<'_>, &AuthoredEquipmentDefinition) -> Result<Vec<T>, String>,
    mut capital_region: impl FnMut(DesignerCapitalSlotObservation) -> Result<Option<u32>, String>,
) -> Result<Vec<T>, String> {
    let mut collect = |definition: ItemPoolDefinition<'_>| collect_country_pools(frame, creator, |key| {
        let mut rows = matching_rows(key, definition.definition)?;
        // 1409f13f0 follows the zero-return collector with +4d8 even when identities coincide.
        let mut inherited = matching_rows(key, definition.archetype)?;
        rows.try_reserve(inherited.len()).map_err(|_| "XUI13 item pool row allocation refused".to_owned())?;
        rows.append(&mut inherited);
        Ok(rows)
    }, &mut capital_region);
    let mut rows = collect(definition)?;
    let graphics = graphics_source()?;
    if !std::ptr::eq(graphics.definition, definition.definition) {
        let mut additional = collect(graphics)?;
        rows.try_reserve(additional.len()).map_err(|_| "XUI13 item pool row allocation refused".to_owned())?;
        rows.append(&mut additional);
    }
    Ok(rows)
}


#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GraphicPoolRootKey { Country(String), Faction(String), Continent(String), Default }

#[derive(Clone, Debug, PartialEq)]
pub struct NamedGraphicPool<T> {
    pub row: T,
    pub country: Option<String>,
    pub continent: Option<String>,
}

pub fn collect_named_equipment_pools<T>(
    equipment: &crate::authored_equipment::AuthoredEquipmentCatalog,
    frame: &replay_core::WorldFrame,
    pool_country: &str,
    token: &str,
    mut matching_rows: impl FnMut(&GraphicPoolRootKey, &AuthoredEquipmentDefinition) -> Result<Vec<T>, String>,
    mut capital_continent: impl FnMut(DesignerCapitalSlotObservation) -> Result<Option<String>, String>,
) -> Result<Vec<NamedGraphicPool<T>>, String> {
    fn pair<'a>(catalog: &'a crate::authored_equipment::AuthoredEquipmentCatalog,
        definition: &'a AuthoredEquipmentDefinition,
    ) -> Result<ItemPoolDefinition<'a>, String> {
        let archetype = catalog.archetype_identity(&definition.token).map_err(|error| error.to_string())?
            .ok_or_else(|| format!("graphic pool {:?} archetype object is null", definition.token))?;
        Ok(ItemPoolDefinition { definition, archetype: catalog.definition(&archetype.token).map_err(|error| error.to_string())? })
    }
    let definition = equipment.definition(token).map_err(|error| error.to_string())?;
    let mut continent_names = Vec::<String>::new();
    let continent_by_id = std::cell::RefCell::new(BTreeMap::<u32, String>::new());
    collect_equipment_pools(frame, pool_country, pair(equipment, definition)?,
        || pair(equipment, equipment.graphics_source(definition).map_err(|error| error.to_string())?),
        |key, definition| {
            let (root, country, continent) = match key {
                ItemPoolKey::Country(tag) => (GraphicPoolRootKey::Country(tag.into()), Some(pool_country.to_owned()), None),
                ItemPoolKey::FactionMember(tag) => (GraphicPoolRootKey::Faction(tag.into()), Some(pool_country.to_owned()), None),
                ItemPoolKey::CapitalRegion(id) => {
                    let name = continent_by_id.borrow().get(&id).cloned().ok_or("graphic pool local continent identity is absent")?;
                    (GraphicPoolRootKey::Continent(name.clone()), None, Some(name))
                },
                ItemPoolKey::Default => (GraphicPoolRootKey::Default, None, None),
            };
            matching_rows(&root, definition)?.into_iter().map(|row| Ok(NamedGraphicPool {
                row, country: country.clone(), continent: continent.clone(),
            })).collect()
        },
        |state| {
            capital_continent(state)?.map(|name| {
                // These handles join names in this lookup only, never native intern IDs.
                let id = match continent_names.iter().position(|known| known == &name) {
                    Some(index) => u32::try_from(index).map_err(|_| "graphic pool continent handle overflow")?,
                    None => {
                        let id = u32::try_from(continent_names.len()).map_err(|_| "graphic pool continent handle overflow")?;
                        continent_names.try_reserve(1).map_err(|_| "graphic pool continent name allocation refused")?;
                        continent_names.push(name.clone());
                        id
                    }
                };
                continent_by_id.borrow_mut().insert(id, name);
                Ok(id)
            }).transpose()
        })
}

pub fn graphic_pool_roots<P: Default>(
    sources: &[(String, String)],
    countries: &BTreeSet<String>,
    continents: &BTreeSet<String>,
    mut append: impl FnMut(&mut P, &Value) -> Result<(), String>,
) -> Result<BTreeMap<GraphicPoolRootKey, Result<P, String>>, String> {
        let mut roots = BTreeMap::<GraphicPoolRootKey, Result<P, String>>::new();
        for (path, text) in sources {
            let mut root = portable_content_manifest::paradox_text::parse_str_strict(text).map_err(|error| format!("{path}: {}", error.message))?;
            portable_content_manifest::paradox_text::expand_file_scoped_variables(&mut root).map_err(|error| format!("{path}: {error}"))?;
            for entry in root.as_object().ok_or("graphic database is not an object")? {
                let token = entry.key.as_deref().ok_or("graphic database has an unnamed root")?;
                if token.starts_with('@') { continue; }
                let key = if !entry.key_is_quoted && token.eq_ignore_ascii_case("default") { GraphicPoolRootKey::Default }
                    else if continents.contains(token) { GraphicPoolRootKey::Continent(token.into()) }
                    else if token.len() == 3 && countries.contains(&token.to_ascii_uppercase()) { GraphicPoolRootKey::Country(token.to_ascii_uppercase()) }
                    else if token.strip_prefix("FACTION_").is_some_and(|tag| tag.len() == 3 && !tag.contains('_')) { GraphicPoolRootKey::Faction(token[8..].into()) }
                    else { continue; };
                let slot = roots.entry(key).or_insert_with(|| Ok(P::default()));
                if let Ok(map) = slot {
                    if let Err(error) = append(map, &entry.value) { *slot = Err(format!("{path}/{token}: {error}")); }
                }
            }
        }
        Ok(roots)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicModelItem<'a> {
    pub token: &'a str,
    pub weight_raw: i64,
}

pub fn model_items<'a, L>(pools: &[&'a AuthoredGraphicPool<L>]) -> Result<Vec<GraphicModelItem<'a>>, String> {
    let mut ordered = Vec::new();
    ordered.try_reserve(pools.len()).map_err(|_| "graphic model pool order allocation refused")?;
    ordered.extend_from_slice(pools);
    let weight = |pool: &AuthoredGraphicPool<L>| {
        if pool.weights_raw[2] == 0 { pool.weights_raw[0] } else { pool.weights_raw[2] }
    };
    ordered.sort_by(|a, b| weight(b).cmp(&weight(a)));
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for pool in ordered {
        for token in pool.models.iter().rev() {
            // 1409efbd0 compares the complete bytes after hashing, including hash collisions.
            if seen.insert(token.as_str()) {
                items.try_reserve(1).map_err(|_| "graphic model item allocation refused")?;
                items.push(GraphicModelItem { token, weight_raw: weight(pool) });
            }
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::paradox_text;

    fn pools(text: &str) -> AuthoredGraphicPoolTypeMap<Value> {
        let mut root = paradox_text::parse_str_strict(text).unwrap();
        paradox_text::expand_file_scoped_variables(&mut root).unwrap();
        let mut pools = AuthoredGraphicPoolTypeMap::default();
        pools.append(&root, |value| Ok(value.clone())).unwrap();
        pools
    }

    #[test]
    fn named_model_pools_keep_helper_traversal_separate_from_filter_context() {
        use crate::test_support::{admitted_fixture_read_set, temp_test_dir, write_modvfs_fixture};
        use serde_json::json;
        let root = temp_test_dir("graphic-model-country-join");
        write_modvfs_fixture(&root, "common/units/equipment/model.txt", br#"equipments = {
            infantry = { is_archetype = yes visual_tech_level_addition = invalid }
        }"#);
        let raw_resolver = crate::GameFileResolver::vanilla(&root).unwrap();
        let (resolver, admission) = admitted_fixture_read_set(&raw_resolver, &Default::default(),
            &[portable_content_manifest::ContentNamespaceScope {
                directory: "common/units/equipment".into(), extension: Some("txt".into()), recursive: true,
            }], "graphic-model-country-join");
        let equipment = crate::authored_equipment::AuthoredEquipmentCatalog::load(&resolver).unwrap()
            .resolve_generated().unwrap();
        assert!(equipment.hierarchy_profile("infantry").is_err());
        let raw = json!({"tag_db_id_index":{"country_composition_version":2},
            "countries":{"GER":{"_r2_tag_db_id":7,"politics":{"capital_override_raw":112,"capital_state":64,
                "capital_slot":{"schema":1,"game_hour_raw":43800006,"observation_ordinal":19,"tag_db_id":7,
                    "operands":{"override_raw":112,"base_raw":64},
                    "outcome":{"kind":"present","slot_count":200,"state_id":73,"template_id":21}}}},
                "ITA":{"_r2_tag_db_id":3}},
            "diplomacy_world":{"domain53_extra":{"faction_member_order_schema":1,
                "faction_roster_status":"measured_nonempty","faction_rows":[{
                    "faction_id":1,"leader":"GER","leader_tag_db_id":7,"member_count":2,
                    "members":["GER","ITA"],"member_tag_db_ids":[7,3]}]}}});
        let world = replay_core::WorldFrame::from_raw_scheduled(replay_core::RawFrame {
            hour_index:7, date:None, save_version:None, schema_version:None,
            rest:raw.as_object().unwrap().clone().into(),
        },None,None,None,replay_core::BuildSchedule::Serial);
        let sources = [("graphic.txt".into(), r#"
            GER = { infantry = { pool = { models = { helper } cultures = { creator_culture } } } }
            ITA = { infantry = { pool = { models = { wrong_country } } } }
            FACTION_GER = { infantry = { pool = { models = { faction_first } } } }
            FACTION_ITA = { infantry = { pool = { models = { faction_second } } } }
            europe = { infantry = { pool = { models = { continent } } } }
            default = { infantry = { pool = { models = { ordinary } } } }
        "#.into())];
        let roots = graphic_pool_roots(&sources, &["GER".into(), "ITA".into()].into_iter().collect(),
            &["europe".into()].into_iter().collect(),
            |map: &mut AuthoredGraphicPoolTypeMap<Value>, value| map.append(value, |limit| Ok(limit.clone()))).unwrap();
        let collect = |culture: &str| collect_named_equipment_pools(&equipment, &world, "GER", "infantry",
            |root, definition| match roots.get(root) {
                Some(pools) => pools.as_ref().map_err(Clone::clone)?.matching_rows(&definition.token,
                    |_| Ok(true), || Ok("infantry".into()), || panic!("no ideology filter"), || Ok(culture.into())),
                None => Ok(Vec::new()),
            }, |slot| {
                assert_eq!(slot.tag_db_id(),7);
                assert_eq!(slot.outcome(),CapitalSlotOutcome::Present {slot_count:200,state_id:73,template_id:21});
                Ok(Some("europe".into()))
            });
        let rows = collect("creator_culture").unwrap();
        assert_eq!(rows.len(),10, "definition and archetype visits remain distinct even for one object");
        assert!(rows[..6].iter().all(|row| row.country.as_deref()==Some("GER")));
        assert!(rows[6..8].iter().all(|row| row.continent.as_deref()==Some("europe") && row.country.is_none()));
        assert!(rows[8..].iter().all(|row| row.country.is_none() && row.continent.is_none()));
        let pools = rows.iter().map(|row| row.row).collect::<Vec<_>>();
        assert_eq!(model_items(&pools).unwrap().iter().map(|item|item.token).collect::<Vec<_>>(),
            ["helper","faction_first","faction_second","continent","ordinary"]);
        let changed = collect("helper_culture").unwrap();
        let pools = changed.iter().map(|row|row.row).collect::<Vec<_>>();
        assert_eq!(model_items(&pools).unwrap().iter().map(|item|item.token).collect::<Vec<_>>(),
            ["faction_first","faction_second","continent","ordinary"]);
        drop((pools, changed, rows));
        drop((roots, equipment, admission, resolver, raw_resolver));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_kind_uses_its_weight_then_reverse_tokens_and_full_identity_deduplication() {
        let pools = pools(r#"tank = {
            pool = { weight = 100 models_weight = 1 models = { ignored_low duplicate } }
            pool = { weight = 2 models_weight = 0 models = { first duplicate } models = { last } }
            pool = { weight = -8 models_weight = 2 models = { newest duplicate } icons = { kept_icon } }
        }"#);
        let rows = pools.matching_rows("tank", |_| Ok(true), || Ok(String::new()),
            || Ok(String::new()), || Ok(String::new())).unwrap();
        let items = model_items(&rows).unwrap();
        assert_eq!(items, [
            GraphicModelItem { token: "duplicate", weight_raw: 200_000 },
            GraphicModelItem { token: "newest", weight_raw: 200_000 },
            GraphicModelItem { token: "last", weight_raw: 200_000 },
            GraphicModelItem { token: "first", weight_raw: 200_000 },
            GraphicModelItem { token: "ignored_low", weight_raw: 100_000 },
        ]);
        assert_eq!(rows[0].icons, ["kept_icon"]);
    }

    #[test]
    fn model_tokens_preserve_case_quotes_expansion_and_repeated_lists() {
        let pools = pools(r#"@model = "@Exact" tank = { pool = { models = { @model "@Exact" } models = { exact Exact } } }"#);
        let rows = pools.matching_rows("tank", |_| Ok(true), || Ok(String::new()),
            || Ok(String::new()), || Ok(String::new())).unwrap();
        assert_eq!(model_items(&rows).unwrap().iter().map(|row| row.token).collect::<Vec<_>>(),
            ["Exact", "exact", "@Exact"]);
    }

    #[test]
    fn independent_model_type_and_unreached_filters_survive_an_invalid_pool() {
        let mut undefined = paradox_text::parse_str_strict("bad = { pool = { models = { @missing } } }").unwrap();
        assert!(paradox_text::expand_file_scoped_variables(&mut undefined).is_err());
        let pools = pools("bad = { pool = { models = { invalid = value } } } good = { pool = { models = { visible } sub_units = { infantry } cultures = { unknown } } }");
        let never = || -> Result<String, String> { panic!("unreached filter") };
        assert!(pools.matching_rows("bad", |_| Ok(true), never, never, never).is_err());
        assert!(pools.matching_rows("good", |_| Ok(false), never, never, never).unwrap().is_empty());
        assert!(pools.matching_rows("good", |_| Ok(true), || Ok("armor".into()), never, never).unwrap().is_empty());
        assert_eq!(pools.matching_rows("good", |_| Ok(true), || Ok("infantry".into()), never,
            || Err("D18 culture unavailable".into())).unwrap_err(), "D18 culture unavailable");
    }
}
