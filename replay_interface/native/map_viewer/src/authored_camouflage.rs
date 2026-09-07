use crate::{ContentAdmissionLease, GameFileResolver, Result};
use portable_content_manifest::{OptionalContentAdmission, UsedContentKey, paradox_text::{self, Value}};
use std::collections::BTreeSet;

const CATALOG_PATH: &str = "common/acclimatation.txt";

#[derive(Clone, Debug)]
pub struct CamouflageCategory {
    name: String,
    authored: Value,
    mappings: CamouflageMappings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CamouflageMappings {
    change_when: std::result::Result<Vec<String>, String>,
    forbid_when: std::result::Result<Vec<String>, String>,
}

impl CamouflageCategory {
    pub fn name(&self) -> &str { &self.name }
    pub fn authored(&self) -> &Value { &self.authored }
    pub fn mappings(&self) -> &CamouflageMappings { &self.mappings }
}

impl CamouflageMappings {
    pub fn change_when(&self) -> Result<&[String]> {
        self.change_when.as_deref().map_err(|reason| reason.clone().into())
    }
    pub fn forbid_when(&self) -> Result<&[String]> {
        self.forbid_when.as_deref().map_err(|reason| reason.clone().into())
    }
}

#[derive(Clone, Debug)]
pub struct AuthoredCamouflageCatalog {
    lease: ContentAdmissionLease,
    source: UsedContentKey,
    categories: Vec<CamouflageCategory>,
}

#[derive(Clone, Copy, Debug)]
pub struct BoundCamouflageCategory<'a> {
    pub category: &'a CamouflageCategory,
    pub weight_raw: i64,
}

impl AuthoredCamouflageCatalog {
    pub fn load(resolver: &GameFileResolver) -> Result<Self> {
        let lease = resolver.content_admission_lease()
            .ok_or("authored camouflage requires content admission")?;
        let source = match lease.optional_content(CATALOG_PATH)? {
            OptionalContentAdmission::Present(key) => key.clone(),
            outcome => return Err(format!("authored camouflage {CATALOG_PATH}: {outcome:?}").into()),
        };
        let text = crate::latin1_bytes_to_string(&lease.read_used_content(&source)?);
        Ok(Self { categories: categories_from_source(&text)?, lease, source })
    }

    pub fn source(&self) -> &UsedContentKey { &self.source }
    pub fn admission_lease(&self) -> &ContentAdmissionLease { &self.lease }
    pub fn categories(&self) -> &[CamouflageCategory] { &self.categories }

    /// The caller first admits the current D20 row and its hour/domain custody.
    pub fn bind_observed_categories<'a, 'b>(
        &'a self, observed: impl IntoIterator<Item = (&'b str, i64)>,
    ) -> Result<Vec<BoundCamouflageCategory<'a>>> {
        bind_categories(&self.categories, observed)
    }
}

fn bind_categories<'a, 'b>(
    categories: &'a [CamouflageCategory], observed: impl IntoIterator<Item = (&'b str, i64)>,
) -> Result<Vec<BoundCamouflageCategory<'a>>> {
    let mut observed = observed.into_iter();
    let mut seen = BTreeSet::new();
    let mut bound = Vec::with_capacity(categories.len());
    for category in categories {
        let Some((name, weight_raw)) = observed.next() else {
            return Err("camouflage D20 category census is shorter than admitted registry".into());
        };
        if category.name != name || name.is_empty() || !seen.insert(name) {
            return Err(format!("camouflage D20 category identity/order mismatch at {}", bound.len()).into());
        }
        bound.push(BoundCamouflageCategory { category, weight_raw });
    }
    if observed.next().is_some() {
        return Err("camouflage D20 category census is longer than admitted registry".into());
    }
    Ok(bound)
}

fn categories_from_source(text: &str) -> Result<Vec<CamouflageCategory>> {
    let mut root = paradox_text::parse_str_strict(text)
        .map_err(|error| format!("camouflage syntax {}:{}: {}", error.line, error.column, error.message))?;
    paradox_text::expand_file_scoped_variables(&mut root)
        .map_err(|error| format!("camouflage variable: {error}"))?;
    let mut categories = Vec::new();
    for entry in root.fields() {
        let name = entry.key.as_deref().ok_or("camouflage category has no name")?;
        if name.starts_with('@') { continue; }
        if entry.value.as_object().is_none() {
            return Err(format!("camouflage category {name} is not a block").into());
        }
        categories.push(CamouflageCategory {
            name: name.to_owned(), authored: entry.value.clone(),
            mappings: mappings(name, &entry.value),
        });
    }
    Ok(categories)
}

fn mappings(name: &str, value: &Value) -> CamouflageMappings {
    let mut result = CamouflageMappings { change_when: Ok(Vec::new()), forbid_when: Ok(Vec::new()) };
    for entry in value.fields() {
        if entry.key_is_quoted { continue; }
        let Some(key) = entry.key.as_deref() else { continue; };
        let destination = if key.eq_ignore_ascii_case("change_camo_when") {
            &mut result.change_when
        } else if key.eq_ignore_ascii_case("forbid_camo_when") {
            &mut result.forbid_when
        } else { continue; };
        if let Ok(tokens) = destination {
            if let Err(reason) = append_mapping(name, key, &entry.value, tokens) {
                *destination = Err(reason);
            }
        }
    }
    result
}

fn append_mapping(name: &str, key: &str, value: &Value, tokens: &mut Vec<String>) -> std::result::Result<(), String> {
    let entries = value.as_object()
        .ok_or_else(|| format!("camouflage {name}.{key}: list shape is unbound"))?;
    for token in entries {
        if token.key.is_some() {
            return Err(format!("camouflage {name}.{key}: keyed list token is unbound"));
        }
        tokens.push(token.value.as_scalar()
            .ok_or_else(|| format!("camouflage {name}.{key}: nested list token is unbound"))?.to_owned());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct CamouflageThresholds {
    show_at_raw: std::result::Result<i64, String>,
    state_fraction_raw: std::result::Result<i64, String>,
    sources: Vec<UsedContentKey>,
    lease: ContentAdmissionLease,
}

impl CamouflageThresholds {
    pub fn load(resolver: &GameFileResolver) -> Result<Self> {
        let lease = resolver.content_admission_lease()
            .ok_or("camouflage thresholds require content admission")?;
        let (sources, contents) = crate::xmap09_lighting_law::admitted_define_sources_from_lease(
            &lease, "camouflage thresholds")?;
        let defines = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;
        let show_at_raw = threshold(&defines, "ACCLIMATIZATION_CAMO_SHOW_AT");
        let state_fraction_raw = threshold(&defines, "ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE");
        Ok(Self { show_at_raw, state_fraction_raw, sources: contents, lease })
    }
    pub fn show_at_raw(&self) -> Result<i64> {
        self.show_at_raw.as_ref().copied().map_err(|reason| reason.clone().into())
    }
    pub fn state_fraction_raw(&self) -> Result<i64> {
        self.state_fraction_raw.as_ref().copied().map_err(|reason| reason.clone().into())
    }
    pub fn sources(&self) -> &[UsedContentKey] { &self.sources }
    pub fn admission_lease(&self) -> &ContentAdmissionLease { &self.lease }
}

fn threshold(defines: &crate::xmap09_lighting_law::EffectiveDefines, key: &str) -> std::result::Result<i64, String> {
    defines.number("NGraphics", key).map(native_threshold).map_err(|error| error.to_string())
}

fn native_threshold(value: f64) -> i64 {
    let raw = crate::fog_derivation_law::fixed_scale_lua_number(value);
    // CVTTSS2SI returns integer-indefinite; Rust saturates positive overflow and maps NaN to zero.
    // No finite f32 in range truncates to i64::MAX, so that result identifies positive overflow.
    if (value as f32).is_nan() || raw == i64::MAX { i64::MIN } else { raw }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_repeated_mapping_blocks_retain_source_order_and_exact_tokens() {
        let categories = categories_from_source(r#"
            hot = { gain_when = { weather_hot = 1 } change_camo_when = { Desert snow }
                CHANGE_CAMO_WHEN = { desert "@literal" } forbid_camo_when = { snow } }
            "cold" = { change_camo_when = { snow } }
            empty = {}
        "#).unwrap();
        assert_eq!(categories.iter().map(CamouflageCategory::name).collect::<Vec<_>>(), ["hot", "cold", "empty"]);
        assert_eq!(categories[0].mappings().change_when().unwrap(), ["Desert", "snow", "desert", "@literal"]);
        assert_eq!(categories[0].mappings().forbid_when().unwrap(), ["snow"]);
        assert!(categories[0].authored().first_value_ci("gain_when").is_some());
        assert!(categories[2].mappings().change_when().unwrap().is_empty());
    }

    #[test]
    fn quoted_fields_do_not_dispatch_and_variables_keep_literal_kind() {
        let categories = categories_from_source(r#"
            @terrain = desert
            hot = { "change_camo_when" = { ignored } change_camo_when = { @terrain "@terrain" } }
        "#).unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].mappings().change_when().unwrap(), ["desert", "@terrain"]);
    }

    #[test]
    fn malformed_reached_mapping_is_local_and_does_not_erase_valid_categories() {
        let categories = categories_from_source("good = { change_camo_when = { snow } } bad = { change_camo_when = { x = y } }").unwrap();
        assert_eq!(categories[0].mappings().change_when().unwrap(), ["snow"]);
        assert!(categories[1].mappings().change_when().unwrap_err().to_string().contains("bad.change_camo_when"));
        assert_eq!(bind_categories(&categories, [("good", 0), ("bad", -1)]).unwrap().len(), 2);
    }

    #[test]
    fn observed_join_requires_exact_full_order_and_preserves_signed_weights() {
        let categories = categories_from_source("cold = {} hot = {}").unwrap();
        let bound = bind_categories(&categories, [("cold", i64::MIN), ("hot", i64::MAX)]).unwrap();
        assert!(std::ptr::eq(bound[0].category, &categories[0]));
        assert_eq!(bound.iter().map(|row| row.weight_raw).collect::<Vec<_>>(), [i64::MIN, i64::MAX]);
        assert!(bind_categories(&categories, [("hot", 1), ("cold", 2)]).is_err());
        assert!(bind_categories(&categories, [("cold", 1)]).is_err());
        assert!(bind_categories(&categories, [("cold", 1), ("hot", 2), ("extra", 3)]).is_err());
        assert!(bind_categories(&categories, [("Cold", 1), ("hot", 2)]).is_err());
    }

    #[test]
    fn duplicate_authored_objects_are_retained_but_cannot_certify_unique_d20_names() {
        let categories = categories_from_source("cold = {} cold = {}").unwrap();
        assert_eq!(categories.len(), 2);
        assert!(!std::ptr::eq(&categories[0], &categories[1]));
        assert!(bind_categories(&categories, [("cold", 1), ("cold", 2)]).is_err());
        assert!(bind_categories(&[], std::iter::empty()).unwrap().is_empty());
    }

    #[test]
    fn production_loads_only_the_sealed_catalog_and_define_winners() {
        use std::fs;
        let root = crate::test_support::temp_test_dir("camouflage_admission");
        fs::create_dir_all(root.join("common/defines")).unwrap();
        fs::write(root.join(CATALOG_PATH), "cold = { change_camo_when = { snow } }").unwrap();
        fs::write(root.join("common/defines/camo.lua"), "NDefines = { NGraphics = { ACCLIMATIZATION_CAMO_SHOW_AT = 0.5, ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE = 0.2 } }").unwrap();
        let resolver = GameFileResolver::vanilla(&root).unwrap();
        assert!(AuthoredCamouflageCatalog::load(&resolver).is_err());
        let (admitted, admission) = crate::test_support::admitted_vanilla_resolver(
            &resolver, &BTreeSet::from([CATALOG_PATH.to_owned(), "common/defines/camo.lua".to_owned()]), "camouflage-admission");
        fs::write(root.join(CATALOG_PATH), "changed = {}").unwrap();
        fs::write(root.join("common/defines/camo.lua"), "NDefines = {}").unwrap();
        let catalog = AuthoredCamouflageCatalog::load(&admitted).unwrap();
        let thresholds = CamouflageThresholds::load(&admitted).unwrap();
        assert_eq!(catalog.categories()[0].name(), "cold");
        assert_eq!(catalog.categories()[0].mappings().change_when().unwrap(), ["snow"]);
        assert_eq!(thresholds.show_at_raw().unwrap(), 50_000);
        assert_eq!(thresholds.state_fraction_raw().unwrap(), 20_000);
        assert_eq!(thresholds.sources().len(), 1);
        assert_eq!(catalog.admission_lease(), thresholds.admission_lease());
        drop((catalog, thresholds, admitted, admission, resolver));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn thresholds_use_effective_lua_order_and_exact_f32_rounding() {
        let defines = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&[
            ("base.lua".into(), "NDefines = { NGraphics = { ACCLIMATIZATION_CAMO_SHOW_AT = 0.5, ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE = 0.2 } }".into()),
            ("mod.lua".into(), "NDefines.NGraphics.ACCLIMATIZATION_CAMO_SHOW_AT = 0.000065".into()),
        ]).unwrap();
        assert_eq!(threshold(&defines, "ACCLIMATIZATION_CAMO_SHOW_AT").unwrap(), 7);
        assert_eq!(threshold(&defines, "ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE").unwrap(), 20_000);
        let missing = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&[
            ("missing.lua".into(), "NDefines = { NGraphics = {} }".into())]).unwrap();
        assert!(threshold(&missing, "ACCLIMATIZATION_CAMO_SHOW_AT").is_err());
        assert_eq!(native_threshold(-0.000065), -7);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e30, -1e30] {
            assert_eq!(native_threshold(value), i64::MIN);
        }
    }

    #[test]
    fn admitted_unreached_list_and_define_errors_do_not_hide_known_operands() {
        use std::fs;
        for (index, catalog_text, define_text) in [
            (0, "cold={forbid_camo_when={snow} change_camo_when={bad=value}}",
                "ACCLIMATIZATION_CAMO_SHOW_AT=0.5"),
            (1, "cold={forbid_camo_when={bad=value} change_camo_when={snow}}",
                "ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE=0.2"),
        ] {
            let root = crate::test_support::temp_test_dir(&format!("camouflage_independent_{index}"));
            fs::create_dir_all(root.join("common/defines")).unwrap();
            fs::write(root.join(CATALOG_PATH), catalog_text).unwrap();
            fs::write(root.join("common/defines/camo.lua"), format!("NDefines={{NGraphics={{{define_text}}}}}")).unwrap();
            let resolver = GameFileResolver::vanilla(&root).unwrap();
            let (admitted, admission) = crate::test_support::admitted_vanilla_resolver(
                &resolver, &BTreeSet::from([CATALOG_PATH.to_owned(), "common/defines/camo.lua".to_owned()]), "camouflage-independent");
            let catalog = AuthoredCamouflageCatalog::load(&admitted).unwrap();
            let thresholds = CamouflageThresholds::load(&admitted).unwrap();
            let mappings = catalog.categories()[0].mappings();
            if index == 0 {
                assert_eq!(thresholds.show_at_raw().unwrap(), 50_000);
                assert!(thresholds.state_fraction_raw().unwrap_err().to_string().contains("ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE"));
                assert_eq!(mappings.forbid_when().unwrap(), ["snow"]);
                assert!(mappings.change_when().unwrap_err().to_string().contains("cold.change_camo_when"));
            } else {
                assert!(thresholds.show_at_raw().unwrap_err().to_string().contains("ACCLIMATIZATION_CAMO_SHOW_AT"));
                assert_eq!(thresholds.state_fraction_raw().unwrap(), 20_000);
                assert_eq!(mappings.change_when().unwrap(), ["snow"]);
                assert!(mappings.forbid_when().unwrap_err().to_string().contains("cold.forbid_camo_when"));
            }
            drop((catalog, thresholds, admitted, admission, resolver));
            portable_test_fs_retry::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cvttss2si_boundary_and_nan_bits_remain_integer_indefinite() {
        for (bits, expected) in [
            (0x56a7_c5ab, 9_223_370_937_343_148_032),
            (0x56a7_c5ac, i64::MIN),
            (0x56a7_c5ad, i64::MIN),
            (0xd6a7_c5ab, -9_223_370_937_343_148_032),
            (0xd6a7_c5ac, i64::MIN),
            (0xd6a7_c5ad, i64::MIN),
            (0x0000_0000, 0), (0x8000_0000, 0),
            (0x7fc0_0001, i64::MIN), (0xffc0_0001, i64::MIN),
            (0x7f80_0001, i64::MIN), (0xff80_0001, i64::MIN),
        ] {
            assert_eq!(native_threshold(f32::from_bits(bits) as f64), expected, "{bits:08x}");
        }
    }

}
