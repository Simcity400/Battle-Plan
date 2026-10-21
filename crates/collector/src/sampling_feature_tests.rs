//! Collector feature defaults are checked where the manifest is owned.

/// The defaults reuse unchanged data, checked by the hourly slices: no
/// burned-in family's full-verify feature is a default. A family in its
/// live burn-in keeps full verification on by default; it is the only
/// kind of `*_verify` default.
#[test]
fn default_features_sample_instead_of_verifying_everything() {
    // Visibility and raid intel graduated by owner approval on 2026-10-20.
    const BURNED_IN: [&str; 48] = [
        "relation",
        "opinion",
        "history",
        "character",
        "portrait",
        "advisor",
        "flag",
        "building",
        "advisor_pool",
        "party_leader",
        "party",
        "wing",
        "roster",
        "political_status",
        "mission",
        "territory",
        "variable",
        "rail",
        "navy_ledger",
        "country_input",
        "appearance",
        "counter",
        "war",
        "raid",
        "area",
        "world_input",
        "local_subunit",
        "ship_model",
        "naval_tree",
        "retreat_ban",
        "unit_leader",
        "appointment",
        "character_record",
        "technology",
        "focus",
        "projects",
        "air_base",
        "air_record",
        "template",
        "air_base_carry",
        "census",
        "unit_leader_trait",
        "province",
        "army_stat",
        "province_unit",
        "weather",
        "visibility",
        "raid_intel",
    ];
    const IN_BURN_IN: [&str; 0] = [];
    let manifest = include_str!("../Cargo.toml");
    let default = manifest
        .lines()
        .find(|line| line.starts_with("default = "))
        .expect("default features");
    let features: Vec<&str> = default.split('"').skip(1).step_by(2).collect();
    for family in BURNED_IN.into_iter().chain(IN_BURN_IN) {
        let changes = format!("{family}_changes");
        assert!(features.contains(&changes.as_str()), "{family}");
    }
    for family in BURNED_IN {
        let verify = format!("{family}_verify");
        assert!(!features.contains(&verify.as_str()), "{family}");
    }
    for family in IN_BURN_IN {
        let verify = format!("{family}_verify");
        assert!(features.contains(&verify.as_str()), "{family}");
    }
    for feature in &features {
        if let Some(family) = feature.strip_suffix("_verify") {
            assert!(IN_BURN_IN.contains(&family), "{feature} is a default");
        }
    }
}
