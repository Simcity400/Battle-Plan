//! Supply health as the registry and the TypeScript contract publish it.

use super::{Product, find};
use crate::products::supply_health::{SupplyHealth, SupplyHealthParams};

#[test]
fn supply_health_registry_no_country_cap_and_prepared_only() {
    assert!(find("supply-health").unwrap().content_dependent);
    let params = SupplyHealthParams {
        branch_id: "1".into(),
        tags: (0..5_000).map(|i| format!("opaque {i}")).collect(),
    };
    assert!(SupplyHealth::validate(&params).is_ok());
    for name in [
        "SupplyHealthParams",
        "SupplyHealthRun",
        "SupplyHealthSeries",
        "SupplyHealthReply",
    ] {
        assert!(crate::typescript_contract().contains(&format!("export type {name} =")));
    }
}
