//! Every product `query_data` dispatches has its hours classified ([`Hours::of`]): none reaches its
//! readers undeclared, and none is refused as an unknown query.
use super::*;
use std::collections::BTreeSet;

/// Products `query_data`'s arms name by constant.
const CONSTANTS: &[(&str, &str)] = &[(
    "dashboard_base::products::country_styles::FACTS_PRODUCT",
    dashboard_base::products::country_styles::FACTS_PRODUCT,
)];

/// Every product `query_data`'s top-level arms dispatch, as its source names them.
fn dispatched() -> BTreeSet<&'static str> {
    let source = include_str!("../query.rs");
    let start = source.find("\nfn query_data(").expect("query_data");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").expect("query_data's end")];
    let mut products = BTreeSet::new();
    for line in body.lines() {
        // Its arms' patterns start at this depth; their bodies are deeper.
        let Some(pattern) = line.strip_prefix("        ") else {
            continue;
        };
        if pattern.starts_with(' ') {
            continue;
        }
        let pattern = pattern.split("=>").next().unwrap_or_default();
        let mut quoted = pattern.split('"').skip(1).step_by(2).peekable();
        if quoted.peek().is_some() {
            products.extend(quoted);
            continue;
        }
        let path = pattern.trim().trim_start_matches('|').trim();
        if path.contains("::")
            && path
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_:".contains(character))
        {
            let (_, product) = CONSTANTS
                .iter()
                .find(|(name, _)| *name == path)
                .unwrap_or_else(|| panic!("name `{path}` in CONSTANTS"));
            products.insert(*product);
        }
    }
    products
}

#[test]
fn every_dispatched_product_has_its_hours_classified() {
    let products = dispatched();
    assert!(
        products.contains("series-state") && products.contains("country-facts"),
        "{products:?}"
    );
    for product in &products {
        let request = ProductRequest {
            source_id: String::new(),
            revision: String::new(),
            product: (*product).to_owned(),
            params: serde_json::json!({}),
        };
        assert!(Hours::of(&request).is_ok(), "{product} has no hours");
    }
    let unknown = ProductRequest {
        source_id: String::new(),
        revision: String::new(),
        product: "no-such-product".into(),
        params: serde_json::json!({}),
    };
    assert!(Hours::of(&unknown).is_err());
}
