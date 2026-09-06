use super::{ItemPoolLimit, ItemPoolRow};
use crate::design_catalog::DesignerRowIcon;
use crate::paradox_text::Value;
use replay_core::design_catalog::DesignU32Observation;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredIconPool {
    pub row: ItemPoolRow<DesignerRowIcon>,
    limit: ItemPoolLimit,
    sub_units: BTreeSet<String>,
    ideologies: BTreeSet<String>,
    cultures: BTreeSet<String>,
    has_models: bool,
}

impl AuthoredIconPool {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let fields = value.as_object().ok_or("XUI13 graphic pool is not an object")?;
        let mut row = ItemPoolRow { weights_raw: [100_000, 0, 0], icons: Vec::new() };
        let mut limit = Vec::new();
        let mut sub_units = BTreeSet::new();
        let mut ideologies = BTreeSet::new();
        let mut cultures = BTreeSet::new();
        let mut has_models = false;
        for field in fields {
            let key = field.key.as_deref().ok_or("XUI13 graphic pool has an unnamed field")?;
            if key.starts_with('@') { continue; }
            match key {
                "weight" | "icons_weight" | "models_weight" => {
                    let text = field.value.as_scalar().ok_or_else(|| format!("XUI13 graphic pool {key} is not scalar"))?;
                    let raw = map_viewer::authored_fixed::parse_fixed5(text)
                        .ok_or_else(|| format!("XUI13 graphic pool {key} is not a fixed-point operand"))?;
                    row.weights_raw[match key { "icons_weight" => 1, "models_weight" => 2, _ => 0 }] = raw;
                }
                "icons" => {
                    for token in tokens(&field.value, key)? {
                        row.icons.push(if token.contains('/') || token.contains('.') {
                            DesignerRowIcon::Texture(token)
                        } else { DesignerRowIcon::Sprite(token) });
                    }
                }
                "models" => { has_models |= !tokens(&field.value, key)?.is_empty(); }
                "limit" => limit.extend(field.value.as_object().ok_or("XUI13 graphic pool limit is not an object")?.iter().cloned()),
                "sub_units" => sub_units.extend(tokens(&field.value, key)?),
                "ideologies" => ideologies.extend(tokens(&field.value, key)?),
                "cultures" => cultures.extend(tokens(&field.value, key)?),
                _ => return Err(format!("XUI13 graphic pool unsupported field {key:?}")),
            }
        }
        Ok(Self { row, limit: ItemPoolLimit::parse(&Value::Object(limit))?, sub_units, ideologies, cultures, has_models })
    }

    pub fn matches(&self, runtime: &DesignU32Observation,
        sub_unit: impl FnOnce() -> Result<String, String>,
        ideology: impl FnOnce() -> Result<String, String>,
        culture: impl FnOnce() -> Result<String, String>,
    ) -> Result<bool, String> {
        if !self.limit.matches(runtime)? { return Ok(false); }
        if !self.sub_units.is_empty() && !self.sub_units.contains(&sub_unit()?) { return Ok(false); }
        if !self.ideologies.is_empty() && !self.ideologies.contains(&ideology()?) { return Ok(false); }
        if !self.cultures.is_empty() && !self.cultures.contains(&culture()?) { return Ok(false); }
        Ok(true)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthoredIconPoolTypeMap {
    rows: BTreeMap<String, Result<Vec<AuthoredIconPool>, String>>,
}

impl AuthoredIconPoolTypeMap {
    pub fn append(&mut self, value: &Value) -> Result<(), String> {
        let entries = value.as_object().ok_or("XUI13 graphic type map is not an object")?;
        if entries.iter().any(|entry| entry.key.is_none()) {
            return Err("XUI13 graphic type has no equipment token".into());
        }
        for entry in entries {
            let token = entry.key.as_deref().expect("checked equipment token");
            if token.starts_with('@') { continue; }
            let slot = self.rows.entry(token.to_owned()).or_insert_with(|| Ok(Vec::new()));
            let Ok(rows) = slot else { continue; };
            let parsed = (|| {
                let pools = entry.value.as_object().ok_or_else(|| format!("XUI13 graphic type {token:?} is not an object"))?;
                let mut additions = Vec::new();
                for pool in pools {
                    if pool.key.as_deref().is_some_and(|key| key.starts_with('@')) { continue; }
                    if pool.key.as_deref() != Some("pool") {
                        return Err(format!("XUI13 graphic type {token:?} has unsupported pool field {:?}", pool.key));
                    }
                    let row = AuthoredIconPool::parse(&pool.value)?;
                    if !row.row.icons.is_empty() || row.has_models { additions.push(row); }
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

    pub fn matching_rows(&self, token: &str, runtime: &DesignU32Observation,
        sub_unit: impl Fn() -> Result<String, String>,
        ideology: impl Fn() -> Result<String, String>,
        culture: impl Fn() -> Result<String, String>,
    ) -> Result<Vec<ItemPoolRow<DesignerRowIcon>>, String> {
        let mut rows = Vec::new();
        if let Some(pools) = self.rows.get(token) {
            // 1409f1540 visits the last appended pool first before the global stable sort.
            for pool in pools.as_ref().map_err(Clone::clone)?.iter().rev() {
                if pool.matches(runtime, &sub_unit, &ideology, &culture)? {
                    rows.push(pool.row.clone());
                }
            }
        }
        Ok(rows)
    }
}

fn tokens(value: &Value, field: &str) -> Result<Vec<String>, String> {
    value.as_object().ok_or_else(|| format!("XUI13 graphic pool {field} is not a list"))?.iter().map(|entry| {
        if entry.key.is_some() { return Err(format!("XUI13 graphic pool {field} has a keyed list entry")); }
        let token = entry.value.as_scalar().ok_or_else(|| format!("XUI13 graphic pool {field} has a nonscalar entry"))?;
        if token.starts_with('@') { return Err(format!("XUI13 graphic pool {field} has unresolved variable {token:?}")); }
        Ok(token.to_owned())
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(text: &str) -> AuthoredIconPool {
        let mut document = crate::paradox_text::parse_str(&format!("pool = {{ {text} }}"));
        crate::paradox_text::expand_file_scoped_variables(&mut document.root).unwrap();
        AuthoredIconPool::parse(document.root.first_value_ci("pool").unwrap()).unwrap()
    }
    fn runtime() -> DesignU32Observation { DesignU32Observation::Measured { raw_u32: 0 } }

    #[test]
    fn repeated_icon_lists_append_and_kind_is_slash_or_dot_not_gfx_prefix() {
        let pool = parse("icons = { GFX_a unknown_sprite } icons = { image.dds path/icon } models = { ignored_entity }");
        assert_eq!(pool.row.weights_raw,[100_000,0,0]);
        assert_eq!(pool.row.icons,[DesignerRowIcon::Sprite("GFX_a".into()),DesignerRowIcon::Sprite("unknown_sprite".into()),
            DesignerRowIcon::Texture("image.dds".into()),DesignerRowIcon::Texture("path/icon".into())]);
    }

    #[test]
    fn exact_independent_weights_keep_native_numeric_prefix_and_overwrite_order() {
        let pool = parse("weight = 1 weight = 0x10 models_weight = -2.5 icons_weight = -0.005");
        assert_eq!(pool.row.weights_raw,[1_600_000,-500,-250_000]);
    }

    #[test]
    fn filters_and_default_limit_short_circuit_without_reading_unreached_context() {
        let no_context = || -> Result<String,String> { panic!("unreached context") };
        assert!(parse("icons = { GFX_a }").matches(&runtime(),no_context,no_context,no_context).unwrap());
        let blocked = parse("limit = { has_country_flag = no_scope } cultures = { western }");
        assert!(!blocked.matches(&runtime(),no_context,no_context,no_context).unwrap());
        let pool = parse("sub_units = { armor } ideologies = { democratic } cultures = { western }");
        assert!(!pool.matches(&runtime(),|| Ok(String::new()),no_context,no_context).unwrap());
        assert!(!pool.matches(&runtime(),|| Ok("armor".into()),|| Ok("fascism".into()),no_context).unwrap());
        assert!(pool.matches(&runtime(),|| Ok("armor".into()),|| Ok("democratic".into()),|| Ok("western".into())).unwrap());
        assert_eq!(pool.matches(&runtime(),|| Ok("armor".into()),|| Ok("democratic".into()),|| Err("D18 culture unknown".into())),Err("D18 culture unknown".into()));
    }

    #[test]
    fn malformed_and_unknown_fields_cannot_become_empty_icon_pools() {
        for text in ["icons = scalar", "icons = { x = y }", "icons = { @unknown }", "invented_pool_rule = yes"] {
            let document = crate::paradox_text::parse_str(&format!("pool = {{ {text} }}"));
            assert!(AuthoredIconPool::parse(document.root.first_value_ci("pool").unwrap()).is_err(),"{text}");
        }
    }

    #[test]
    fn repeated_types_and_files_append_but_lookup_visits_newest_pool_first() {
        let mut map = AuthoredIconPoolTypeMap::default();
        for text in ["tank = { pool = { icons = { GFX_old } } } tank = { pool = { icons = { GFX_middle } } }",
                     "tank = { pool = { icons = { GFX_new } } }"] {
            map.append(&crate::paradox_text::parse_str(text).root).unwrap();
        }
        let context = || Ok(String::new());
        let rows = map.matching_rows("tank", &runtime(), context, context, context).unwrap();
        assert_eq!(rows.iter().map(|row| row.icons[0].clone()).collect::<Vec<_>>(),
            [DesignerRowIcon::Sprite("GFX_new".into()), DesignerRowIcon::Sprite("GFX_middle".into()), DesignerRowIcon::Sprite("GFX_old".into())]);
        assert_eq!(super::super::resolve_collected_icon(&rows, |_| Ok(true)).unwrap(),
            Some(&DesignerRowIcon::Sprite("GFX_new".into())));
    }

    #[test]
    fn empty_pools_are_not_inserted_but_model_only_pools_keep_their_filter() {
        let mut map = AuthoredIconPoolTypeMap::default();
        map.append(&crate::paradox_text::parse_str("tank = { pool = { cultures = { unknown } } }").root).unwrap();
        let unavailable = || Err("culture unavailable".into());
        assert!(map.matching_rows("tank", &runtime(), unavailable, unavailable, unavailable).unwrap().is_empty());
        map.append(&crate::paradox_text::parse_str("tank = { pool = { models = { model } cultures = { unknown } } }").root).unwrap();
        assert_eq!(map.matching_rows("tank", &runtime(), unavailable, unavailable, unavailable), Err("culture unavailable".into()));
        assert!(map.matching_rows("absent", &runtime(), unavailable, unavailable, unavailable).unwrap().is_empty());
    }

    #[test]
    fn malformed_type_stays_unknown_without_erasing_independent_equipment() {
        let mut map = AuthoredIconPoolTypeMap::default();
        map.append(&crate::paradox_text::parse_str("tank = { pool = { icons = { GFX_old } } } ship = { pool = { icons = { GFX_previous_ship } } }").root).unwrap();
        map.append(&crate::paradox_text::parse_str("tank = { pool = { icons = { GFX_new } } } ship = { pool = scalar }").root).unwrap();
        let context = || Ok(String::new());
        let rows = map.matching_rows("tank", &runtime(), context, context, context).unwrap();
        assert_eq!(rows.iter().map(|row| row.icons[0].clone()).collect::<Vec<_>>(),
            [DesignerRowIcon::Sprite("GFX_new".into()), DesignerRowIcon::Sprite("GFX_old".into())]);
        assert!(map.matching_rows("ship", &runtime(), context, context, context).unwrap_err().contains("not an object"));
        map.append(&crate::paradox_text::parse_str("ship = { pool = { icons = { GFX_later } } }").root).unwrap();
        assert!(map.matching_rows("ship", &runtime(), context, context, context).is_err());
        assert!(map.matching_rows("absent", &runtime(), context, context, context).unwrap().is_empty());
    }

}
