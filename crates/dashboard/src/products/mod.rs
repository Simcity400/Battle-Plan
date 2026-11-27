use dashboard_base::products::Registered;

pub mod battle_names;

pub static PRODUCTS: std::sync::LazyLock<Vec<Registered>> = std::sync::LazyLock::new(|| {
    let mut products = [
        dashboard_air::registrations::PRODUCTS,
        dashboard_economy::registrations::PRODUCTS,
        dashboard_intelligence::registrations::PRODUCTS,
        dashboard_land::registrations::PRODUCTS,
        dashboard_naval::registrations::PRODUCTS,
        dashboard_players::registrations::PRODUCTS,
        dashboard_politics::registrations::PRODUCTS,
        dashboard_production::registrations::PRODUCTS,
        dashboard_research::registrations::PRODUCTS,
        dashboard_wars::registrations::PRODUCTS,
        dashboard_world::registrations::PRODUCTS,
        battle_names::PRODUCTS,
    ].concat();
    products.sort_unstable_by_key(|product| product.key);
    products
});

const SERVICE_ONLY: &[&str] = &[
    dashboard_players::registrations::CAMPAIGN_CLOCK_CAMPAIGNCLOCK.key,
    dashboard_players::registrations::PLAYER_APM_PLAYERAPM.key,
    dashboard_politics::registrations::TECHNOLOGY_ARTWORK_TECHNOLOGYARTWORK.key,
    dashboard_world::registrations::COUNTRY_FLAGS_COUNTRYFLAGS.key,
    dashboard_land::registrations::DOCTRINES_DOCTRINES.key,
    dashboard_air::registrations::AIRCRAFT_CATALOGUE_AIRCRAFTCATALOGUE.key,
    dashboard_world::registrations::ARTWORK_SPRITEARTWORK.key,
    dashboard_air::registrations::AIR_LEDGER_BREAKDOWN_BOMBINGDAMAGEBREAKDOWN.key,
    dashboard_production::registrations::PRODUCTION_SPENDING_MONTHS_PRODUCTIONSPENDINGMONTHS.key,
];

pub fn in_viewer(key: &str) -> bool {
    find(key).is_some() && !SERVICE_ONLY.contains(&key)
}

pub fn find(key: &str) -> Option<&'static Registered> {
    PRODUCTS.binary_search_by_key(&key, |product| product.key).ok().map(|index| &PRODUCTS[index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::{CountryParams, validate_tag};

    #[test]
    fn exported_product_type_names_do_not_hide_different_shapes() {
        let mut declarations = std::collections::BTreeMap::new();
        for product in PRODUCTS.iter() {
            for declaration in product.types.expect("every exported product registers its types")() {
                let name = declaration.strip_prefix("type ").unwrap().split_whitespace().next().unwrap().to_owned();
                if let Some(previous) = declarations.insert(name.clone(), declaration.clone()) {
                    assert_eq!(previous, declaration, "different exported types named {name}");
                }
            }
        }
    }

    #[test]
    fn tag_validation_preserves_long_recorded_lookup_keys() {
        for tag in ["x".repeat(129), "å›½".repeat(129)] {
            assert_eq!(validate_tag(&tag), Ok(()));
            assert_eq!(
                CountryParams {
                    branch_id: "1".into(),
                    country: tag.into(),
                }
                .validate(),
                Ok(())
            );
        }
        for tag in ["", "\0", "tag\0suffix"] {
            assert_eq!(validate_tag(tag), Err("Invalid country tag"));
        }
    }

    #[test]
    fn browser_interactive_products_match_service_admission() {
        let contract = crate::typescript_contract();
        let declaration = contract
            .lines()
            .find(|line| line.starts_with("export const INTERACTIVE_PRODUCTS"))
            .expect("browser admission needs the service's product lanes");
        let replays = contract
            .lines()
            .find(|line| line.starts_with("export const REPLAY_PRODUCTS"))
            .expect("browser admission needs the service's replaying pages");
        for product in PRODUCTS.iter() {
            assert_eq!(
                declaration.contains(&format!("{:?}", product.key)),
                product.interactive,
                "{} admission differs between browser and service",
                product.key
            );
            assert_eq!(replays.contains(&format!("{:?}", product.key)), product.replays, "{} replays differently in browser and service", product.key);
        }
    }

    #[test]
    fn product_keys_are_sorted_and_unique() {
        assert!(PRODUCTS.windows(2).all(|pair| pair[0].key < pair[1].key));
    }

}
