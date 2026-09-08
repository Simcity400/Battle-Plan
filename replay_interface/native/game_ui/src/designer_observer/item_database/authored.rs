use super::{ItemPoolLimit, ItemPoolRow};
use crate::design_catalog::DesignerRowIcon;
use crate::paradox_text::Value;
use map_viewer::authored_graphic_pool::{AuthoredGraphicPool, AuthoredGraphicPoolTypeMap};
use replay_core::design_catalog::DesignU32Observation;

fn icon_row(pool: &AuthoredGraphicPool<ItemPoolLimit>) -> ItemPoolRow<DesignerRowIcon> {
    ItemPoolRow {
        weights_raw: pool.weights_raw,
        icons: pool.icons.iter().map(|token| {
            if token.contains('/') || token.contains('.') {
                DesignerRowIcon::Texture(token.clone())
            } else {
                DesignerRowIcon::Sprite(token.clone())
            }
        }).collect(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuthoredIconPoolTypeMap {
    pools: AuthoredGraphicPoolTypeMap<ItemPoolLimit>,
}

impl AuthoredIconPoolTypeMap {
    pub fn append(&mut self, value: &Value) -> Result<(), String> {
        self.pools.append(value, ItemPoolLimit::parse)
    }

    pub fn matching_rows(&self, token: &str, runtime: &DesignU32Observation,
        sub_unit: impl Fn() -> Result<String, String>,
        ideology: impl Fn() -> Result<String, String>,
        culture: impl Fn() -> Result<String, String>,
    ) -> Result<Vec<ItemPoolRow<DesignerRowIcon>>, String> {
        self.pools.matching_rows(token, |limit| limit.matches(runtime), sub_unit, ideology, culture)
            .map(|rows| rows.into_iter().map(icon_row).collect())
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
struct AuthoredIconPool {
    row: ItemPoolRow<DesignerRowIcon>,
    pool: AuthoredGraphicPool<ItemPoolLimit>,
}

#[cfg(test)]
impl AuthoredIconPool {
    fn parse(value: &Value) -> Result<Self, String> {
        let pool = AuthoredGraphicPool::parse(value, ItemPoolLimit::parse)?;
        Ok(Self { row: icon_row(&pool), pool })
    }

    fn matches(&self, runtime: &DesignU32Observation,
        sub_unit: impl FnOnce() -> Result<String, String>,
        ideology: impl FnOnce() -> Result<String, String>,
        culture: impl FnOnce() -> Result<String, String>,
    ) -> Result<bool, String> {
        self.pool.matches(|limit| limit.matches(runtime), sub_unit, ideology, culture)
    }
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
    fn quoted_at_literal_and_quoted_variable_value_preserve_item_identity() {
        let mut document = crate::paradox_text::parse_str(
            r#"@chosen = "@Literal.TGA" pool = { icons = { "@Literal.TGA" @chosen } }"#);
        crate::paradox_text::expand_file_scoped_variables(&mut document.root).unwrap();
        let pool = AuthoredIconPool::parse(document.root.first_value_ci("pool").unwrap()).unwrap();
        assert_eq!(pool.row.icons, [DesignerRowIcon::Texture("@Literal.TGA".into()),
            DesignerRowIcon::Texture("@Literal.TGA".into())]);
    }

    #[test]
    fn bare_unresolved_value_stays_named_without_poisoning_independent_type() {
        let mut map = AuthoredIconPoolTypeMap::default();
        map.append(&crate::paradox_text::parse_str(
            r#"tank = { pool = { icons = { @missing } } } ship = { pool = { icons = { "@Literal.TGA" } } }"#).root).unwrap();
        let context = || Ok(String::new());
        assert_eq!(map.matching_rows("tank", &runtime(), context, context, context),
            Err("graphic pool icons has unresolved variable \"@missing\"".into()));
        let rows = map.matching_rows("ship", &runtime(), context, context, context).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].icons, [DesignerRowIcon::Texture("@Literal.TGA".into())]);
    }

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
    fn ascii_keywords_preserve_value_case_and_repeated_field_order() {
        let lower = parse("weight = 9 weight = 2 icons_weight = 7 icons_weight = 3 models_weight = -4 icons = { GFX_Mixed } icons = { Path/Exact.TGA } models = { ModelExact } limit = {} sub_units = { ArmorExact } ideologies = { IdeologyExact } cultures = { CultureExact }");
        let mixed = parse("WeIgHt = 9 WEIGHT = 2 IcOnS_WeIgHt = 7 ICONS_WEIGHT = 3 MoDeLs_WeIgHt = -4 IcOnS = { GFX_Mixed } ICONS = { Path/Exact.TGA } MoDeLs = { ModelExact } LiMiT = {} SuB_UnItS = { ArmorExact } IdEoLoGiEs = { IdeologyExact } CuLtUrEs = { CultureExact }");
        assert_eq!(mixed, lower);
        assert_eq!(mixed.row.weights_raw, [200_000, 300_000, -400_000]);
        assert_eq!(mixed.row.icons, [DesignerRowIcon::Sprite("GFX_Mixed".into()), DesignerRowIcon::Texture("Path/Exact.TGA".into())]);
        assert!(!mixed.pool.models.is_empty());
        assert!(mixed.matches(&runtime(), || Ok("ArmorExact".into()), || Ok("IdeologyExact".into()), || Ok("CultureExact".into())).unwrap());
        assert!(!mixed.matches(&runtime(), || Ok("armorexact".into()), || panic!("unreached ideology"), || panic!("unreached culture")).unwrap());
        assert!(!mixed.matches(&runtime(), || Ok("ArmorExact".into()), || Ok("ideologyexact".into()), || panic!("unreached culture")).unwrap());
        assert!(!mixed.matches(&runtime(), || Ok("ArmorExact".into()), || Ok("IdeologyExact".into()), || Ok("cultureexact".into())).unwrap());
        let mut map = AuthoredIconPoolTypeMap::default();
        map.append(&crate::paradox_text::parse_str("TankExact = { PoOl = { IcOnS = { GFX_Old } } POOL = { ICONS = { GFX_New } } }").root).unwrap();
        let context = || Ok(String::new());
        let rows = map.matching_rows("TankExact", &runtime(), context, context, context).unwrap();
        assert_eq!(rows.iter().map(|row| row.icons[0].clone()).collect::<Vec<_>>(),
            [DesignerRowIcon::Sprite("GFX_New".into()), DesignerRowIcon::Sprite("GFX_Old".into())]);
        assert!(map.matching_rows("tankexact", &runtime(), context, context, context).unwrap().is_empty());
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
    fn quoted_pool_is_not_admitted_and_unrelated_type_keeps_its_bare_pool() {
        let mut map = AuthoredIconPoolTypeMap::default();
        map.append(&crate::paradox_text::parse_str(r#"tank = { pool = { icons = { GFX_Old } } "pool" = { icons = { GFX_NotAdmitted } } } ship = { PoOl = { icons = { GFX_Ship } } }"#).root).unwrap();
        let context = || Ok(String::new());
        assert!(map.matching_rows("tank", &runtime(), context, context, context).unwrap_err().contains("quoted pool keyword"));
        let rows = map.matching_rows("ship", &runtime(), context, context, context).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].icons, [DesignerRowIcon::Sprite("GFX_Ship".into())]);
        map.append(&crate::paradox_text::parse_str("tank = { pool = { icons = { GFX_Later } } }").root).unwrap();
        assert!(map.matching_rows("tank", &runtime(), context, context, context).is_err());
    }

    #[test]
    fn quoted_field_keywords_refuse_the_reached_type_without_changing_value_identity() {
        for field in ["weight = 1", "icons_weight = 2", "models_weight = 3",
            "icons = { GFX_Exact }", "models = { Model_Exact }", "limit = {}",
            "sub_units = { Armor_Exact }", "ideologies = { Ideology_Exact }", "cultures = { Culture_Exact }"] {
            let (key, value) = field.split_once(" = ").unwrap();
            let mut map = AuthoredIconPoolTypeMap::default();
            let text = format!(r#"tank = {{ pool = {{ icons = {{ GFX_Old }} }} pool = {{ "{key}" = {value} }} }} ship = {{ pool = {{ icons = {{ "GFX_Exact" }} }} }}"#);
            map.append(&crate::paradox_text::parse_str(&text).root).unwrap();
            let context = || Ok(String::new());
            assert!(map.matching_rows("tank", &runtime(), context, context, context)
                .unwrap_err().contains("quoted field keyword"), "{key}");
            let rows = map.matching_rows("ship", &runtime(), context, context, context).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].icons, [DesignerRowIcon::Sprite("GFX_Exact".into())]);
            map.append(&crate::paradox_text::parse_str("tank = { pool = { icons = { GFX_Later } } }").root).unwrap();
            assert!(map.matching_rows("tank", &runtime(), context, context, context).is_err());
        }
        let pool = parse(r#"@weight = 9 "@icons" = { ignored } IcOnS = { "GFX_Exact" }"#);
        assert_eq!(pool.row.weights_raw, [100_000, 0, 0]);
        assert_eq!(pool.row.icons, [DesignerRowIcon::Sprite("GFX_Exact".into())]);
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
