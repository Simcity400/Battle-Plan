use super::plan::{self, Backend, Owner, Plan};
use super::registry::{self, Descriptor, FamilyId};
use crate::prepared::series::catalogue_section;
use crate::products;
use dashboard_keyed::Observer as _;
use std::{collections::BTreeSet, io};

fn names(ids: &[FamilyId]) -> BTreeSet<&'static str> {
    ids.iter().map(|id| id.name()).collect()
}

fn refusing(plan: &Plan, product: &str) -> BTreeSet<&'static str> {
    plan.families()
        .iter()
        .filter(|family| family.queries.contains(&product))
        .map(|family| family.id.name())
        .collect()
}

fn family(name: &str) -> Owner {
    Owner::Family(FamilyId::parse(name).unwrap())
}

#[test]
fn every_family_has_one_backend_and_every_status_row_one_reporter() {
    let plan = plan::native();
    let native = super::native::names();
    for family in registry::FAMILIES {
        let name = family.id.name();
        let backend = if native.contains(&name) {
            Backend::Segments
        } else {
            Backend::Unavailable
        };
        assert_eq!(plan.backend(name), Some(backend), "{name}");
    }
    for &(row, _) in registry::STATUS {
        let reporters = registry::FAMILIES
            .iter()
            .filter(|family| family.status.contains(&row))
            .count()
            + usize::from(registry::job(row).is_some());
        assert_eq!(reporters, 1, "status row {row}");
    }
}

#[test]
fn the_division_log_is_the_divisions_family_by_either_spelling() {
    let divisions = FamilyId::parse("divisions").unwrap();
    assert_eq!(FamilyId::parse("division-log").unwrap(), divisions);
    assert_eq!(divisions.name(), "divisions");
    assert_eq!(
        registry::coverage_section(divisions.name(), 3),
        "preparation-family-divisions-3"
    );
    let rows = plan::native();
    assert_eq!(
        rows.owner("preparation-family-divisions-3"),
        Some(family("divisions"))
    );
    assert_eq!(
        rows.owner("division-log-counts-3"),
        Some(family("divisions"))
    );
    assert_eq!(rows.gating("division-log-manifest"), Some("divisions"));
    assert_eq!(
        rows.owner("preparation-family-division-log-3"),
        None,
        "an alias names no coverage page"
    );
    for unknown in ["division_log", "Divisions", ""] {
        assert_eq!(
            FamilyId::parse(unknown).unwrap_err().kind(),
            io::ErrorKind::InvalidInput,
            "{unknown:?}"
        );
    }
}

#[test]
fn every_section_has_one_exact_owner() {
    let rows = plan::native();
    for (section, owner) in [
        ("preparation-plan-v1", Some(Owner::Coordinator)),
        (
            "preparation-family-production-lines-0",
            Some(Owner::Job("production-lines")),
        ),
        ("event_feed-continuation-1", Some(family("event_feed"))),
        ("production-facts-v2/7", Some(family("production"))),
        (
            "production-lines-state-v2-1-0",
            Some(Owner::Job("production-lines")),
        ),
        (
            "production-index-v3-1",
            Some(Owner::Job("production-lines")),
        ),
        (
            "production-commits-v3-1",
            Some(Owner::Job("production-lines")),
        ),
        (
            "production-ledger-v2-1",
            Some(Owner::Job("production-lines")),
        ),
        (
            "production-chunks-v1-1",
            Some(Owner::Job("production-lines")),
        ),
        (
            "production-absence-v1-1",
            Some(Owner::Job("production-lines")),
        ),
        (
            "occupation-reserve-values-3",
            Some(family("occupation-reserve")),
        ),
        ("fuel_branches-values-1", Some(family("fuel_branches"))),
        ("air-values-1-0", Some(family("air"))),
        ("air-1-catalogue", Some(family("air"))),
        ("air-aces-1-windows", Some(family("aces"))),
        ("air-superiority-groups-1", Some(family("air_superiority"))),
        ("country-groups-1", Some(family("groups"))),
        ("country-group-state-1", Some(family("groups"))),
        ("event-feed-identities-1", Some(family("event_feed"))),
        ("country-identities-1", Some(family("identities"))),
        ("naval-battles-1", Some(family("naval"))),
        ("naval-battle-windows-1", Some(family("naval"))),
        ("war-casualties-state-1", Some(family("casualties"))),
        ("land-owners", Some(Owner::Job("land-owners"))),
        ("synthetic-alpha-1", None),
        ("airfield-1", None),
        // Spellings no writer builds.
        ("production-catalogue-values-3", None),
        ("production_catalogue-values-3", None),
        ("fuel-branches-values-1", None),
        ("fuel_branches-values-1", None),
        ("lend-lease-balance-1-catalogue", None),
        ("wars-series-1", None),
        (
            "preparation-facts-v2/ab/one/map-export-endpoints/keys",
            None,
        ),
        ("preparation-family-unknown-1", None),
    ] {
        assert_eq!(rows.owner(section), owner, "{section}");
    }
}

/// Each namespace by the names its writers build, through their own
/// constructors and constants wherever they have them.
#[test]
fn every_section_a_writer_names_belongs_to_its_publisher() {
    let rows = plan::native();
    let named = [
        // The coordinator's, and every coverage page.
        (
            crate::prepared::coverage::PLAN.to_owned(),
            Owner::Coordinator,
        ),
        ("source-metadata-v1".to_owned(), Owner::Coordinator),
        ("source-metadata-refusals-v1".to_owned(), Owner::Coordinator),
        (
            registry::coverage_section("fuel_branches", 1),
            family("fuel_branches"),
        ),
        (
            registry::coverage_section("production-lines", 1),
            Owner::Job("production-lines"),
        ),
        // Continuations, by the family's name.
        (
            registry::continuation_section(products::fuel_branches::FAMILY, 1),
            family("fuel_branches"),
        ),
        // Series and catalogues.
        (
            dashboard_base::prepared::roots::root_section(
                products::politics_state::history::FAMILY,
                1,
            ),
            family("politics_state"),
        ),
        (
            dashboard_base::prepared::roots::root_section(
                products::production_catalogue::FAMILY,
                1,
            ),
            family("production_catalogue"),
        ),
        (
            dashboard_economy::products::keyed::values_section(products::convoy_health::FAMILY, 1),
            family("convoys"),
        ),
        (
            dashboard_base::prepared::roots::root_section(products::style_runs::NAME, 1),
            family("country-styles"),
        ),
        (
            dashboard_base::prepared::roots::root_section(
                products::naval_losses::prepared::NAVAL,
                1,
            ),
            family("naval"),
        ),
        (
            catalogue_section("naval-battle-ship-keys", 1),
            family("naval"),
        ),
        (
            catalogue_section("division-log-count-keys", 1),
            family("divisions"),
        ),
        (
            catalogue_section("event-feed-facts", 1),
            family("event_feed"),
        ),
        (
            dashboard_base::prepared::roots::root_section("trade", 1),
            family("trade"),
        ),
        // Pages kept by name and branch.
        (
            format!("{}/1", crate::prepared_production::FACTS),
            family("production"),
        ),
        (
            format!("{}/1", crate::prepared_production::STATE),
            family("production"),
        ),
        ("production-spent-1".to_owned(), family("production")),
        (
            "production-lines-private-v3-1".to_owned(),
            Owner::Job("production-lines"),
        ),
        (
            "production-lines-state-v2-1-world".to_owned(),
            Owner::Job("production-lines"),
        ),
        // The land owners.
        ("land-owners".to_owned(), Owner::Job("land-owners")),
        // A family's own reads of another's state, under its own names.
        (
            "air-superiority-groups-1".to_owned(),
            family("air_superiority"),
        ),
        (
            catalogue_section("air-superiority-group-changes", 1),
            family("air_superiority"),
        ),
        (
            "air-superiority-group-state-1".to_owned(),
            family("air_superiority"),
        ),
        ("event-feed-identities-1".to_owned(), family("event_feed")),
    ];
    for (section, owner) in named {
        assert_eq!(rows.owner(&section), Some(owner), "{section}");
    }
}

#[test]
fn a_writer_refuses_every_section_another_publisher_owns() {
    let rows = plan::native();
    let refused = |name: &'static str, section: &str| {
        rows.guard(name)(section)
            .map(|()| false)
            .unwrap_or_else(|error| {
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{section}");
                true
            })
    };
    // A name no publisher registers is refused from every publisher, as the coordinator's are.
    for name in ["leaders", "production-lines", "land-owners"] {
        for section in [
            "synthetic-unregistered-1",
            "airfield-1",
            "preparation-family-unknown-1",
            "preparation-facts-v2/ab/1/unregistered-column/keys",
            "preparation-plan-v1",
        ] {
            assert!(refused(name, section), "{name} {section}");
        }
    }
    for section in [
        "unit-leaders-1",
        "unit-leaders-1-catalogue",
        "preparation-family-leaders-1",
        "leaders-continuation-1",
    ] {
        assert!(!refused("leaders", section), "{section}");
    }
    for section in [
        "stability-1",
        "preparation-family-stability-1",
        "faction-goals-7-1-catalogue",
        "experience-1",
        "preparation-family-supply-1",
        "land-owners",
    ] {
        assert!(refused("leaders", section), "{section}");
    }
    let lines = "production-lines";
    assert!(!refused(lines, "preparation-family-production-lines-1"));
    assert!(!refused(lines, "production-lines-history-v3-1"));
    assert!(refused(lines, "production-spent-1"));
    assert!(!refused("air_superiority", "air-superiority-groups-1"));
    assert!(refused("groups", "air-superiority-groups-1"));
    assert!(!refused("groups", "country-groups-1"));
    assert!(!refused("event_feed", "event-feed-identities-1"));
    // A same-hour dependent is refused with its source, which no query join does.
    let observers = registry::family_named("identities").unwrap().observers;
    assert_eq!(names(observers), BTreeSet::from(["event_feed"]));
    assert!(
        !registry::family_named("territory")
            .unwrap()
            .observers
            .contains(&FamilyId::parse("identities").unwrap())
    );
}

#[test]
fn shared_queries_name_every_supplier_of_their_coverage_and_refusals() {
    let rows = plan::native();
    let feed = BTreeSet::from([
        "event_feed",
        "groups",
        "subjects",
        "ideas",
        "focus",
        "nukes",
        "projects",
    ]);
    assert_eq!(names(rows.joint("event-feed").unwrap()), feed);
    assert_eq!(refusing(rows, "event-feed"), feed);
    assert_eq!(
        crate::prepared::pipeline::gate("event-feed"),
        feed.iter().copied().collect::<Vec<_>>()
    );
    let supply = BTreeSet::from(["supply", "identities"]);
    assert_eq!(names(rows.joint("supply-health").unwrap()), supply);
    assert_eq!(refusing(rows, "supply-health"), supply);
    assert_eq!(
        crate::prepared::pipeline::gate("supply-health"),
        vec!["identities", "supply"]
    );
    assert_eq!(rows.gating("event-choices"), Some("event_feed"));
    assert_eq!(
        rows.gating("production-line-detail"),
        Some("production-lines")
    );
}

/// Preparation's policies for a representative set of families and
/// jobs, written out rather than read back from the registry.
#[test]
fn ordinary_preparation_plans_exactly_these_policies() {
    let rows = plan::native();
    for (product, publisher) in [
        ("air-missions", Some("air-missions")),
        ("country-colours", Some("country-styles")),
        ("division-equipment-history", Some("division-equipment")),
        ("faction-state", Some("faction_state")),
        ("focus-order-page", Some("focus")),
        ("production-select", Some("production")),
        ("country-identities", Some("identities")),
        ("country-groups", Some("groups")),
        ("dockyard-spending", Some("repairs")),
        ("event-choices", Some("event_feed")),
        ("decisions", Some("event_feed")),
        ("division-log-manifest", Some("divisions")),
        ("fuel-branches", Some("fuel_branches")),
        ("market-ledger", Some("market")),
        ("politics-state-history", Some("politics_state")),
        ("stability-parts", Some("parts")),
        ("country-stability", Some("stability")),
        ("casualty-pairs", Some("casualties")),
        ("occupation-trend", Some("occupation")),
        ("production-line-breakdown", Some("production-lines")),
        ("volunteers-expeditionaries", Some("army")),
        ("army-size", None),
        ("country-measure", Some("measure")),
        ("game-score", Some("game_score")),
        ("resource-balance", None),
        ("land-battles", Some("land")),
        ("autonomy", None),
        ("scoreboard", None),
        ("event-feed", None),
        ("territory", None),
        ("country-nukes", None),
        ("world-tension", None),
        ("trade-flows", None),
    ] {
        assert_eq!(rows.gating(product), publisher, "gate of {product}");
    }

    let joint = |product: &str| {
        rows.joint(product)
            .map(|suppliers| suppliers.iter().map(|id| id.name()).collect::<Vec<_>>())
    };
    for (product, suppliers) in [
        (
            "event-feed",
            Some(vec![
                "event_feed",
                "groups",
                "subjects",
                "ideas",
                "focus",
                "nukes",
                "projects",
            ]),
        ),
        ("civil-wars", Some(vec!["politics_state", "event_feed"])),
        (
            "faction-totals",
            Some(vec!["groups", "identities", "territory"]),
        ),
        (
            "faction-totals-factories",
            Some(vec!["groups", "identities", "factories"]),
        ),
        ("dockyard-spending", Some(vec!["production", "repairs"])),
        ("supply-health", Some(vec!["supply", "identities"])),
        ("country-nukes", Some(vec!["nukes", "identities"])),
        ("world-tension", Some(vec!["tension", "identities"])),
        ("autonomy", Some(vec!["autonomy", "identities"])),
        ("trade-flows", Some(vec!["trade", "identities"])),
        ("army-size", Some(vec!["army", "identities"])),
        ("air-force-size", Some(vec!["air", "identities"])),
        ("navy-size", Some(vec!["navy", "identities"])),
        (
            "scoreboard",
            Some(vec![
                "air",
                "army",
                "casualties",
                "factories",
                "identities",
                "measure",
                "navy",
                "nukes",
                "production",
                "stability",
                "territory",
            ]),
        ),
        ("country-stability", None),
    ] {
        assert_eq!(joint(product), suppliers, "suppliers of {product}");
    }

    for (product, families) in [
        (
            "scoreboard",
            &[
                "air",
                "army",
                "casualties",
                "factories",
                "identities",
                "measure",
                "navy",
                "nukes",
                "production",
                "stability",
                "territory",
            ][..],
        ),
        (
            "event-feed",
            &[
                "event_feed",
                "focus",
                "groups",
                "ideas",
                "nukes",
                "projects",
                "subjects",
            ][..],
        ),
        ("civil-wars", &["event_feed", "politics_state"][..]),
        ("country-people", &["manpower", "politics_state"][..]),
        (
            "faction-totals",
            &["factories", "groups", "identities", "territory"][..],
        ),
        ("naval-losses", &["identities", "naval"][..]),
        ("country-groups", &["groups"][..]),
    ] {
        assert_eq!(
            refusing(rows, product),
            families.iter().copied().collect::<BTreeSet<_>>(),
            "refusals of {product}"
        );
    }

    for family in registry::FAMILIES {
        let name = family.id.name();
        let observers: &[&str] = match name {
            "identities" => &["event_feed"],
            "groups" => &["air_superiority"],
            _ => &[],
        };
        assert_eq!(
            family
                .observers
                .iter()
                .map(|id| id.name())
                .collect::<Vec<_>>(),
            observers,
            "observers of {name}"
        );
    }

}

#[test]
fn a_plan_refuses_what_its_registry_or_backends_cannot_prepare() {
    let families = || registry::FAMILIES.iter().collect::<Vec<_>>();
    for (native, error) in [
        (
            &["nonexistent"][..],
            "A native backend prepares the unknown family \"nonexistent\"",
        ),
        (&["groups", "groups"][..], "\"groups\" has two native backends"),
    ] {
        let refused = Plan::new(families(), native)
            .err()
            .expect("a refused plan");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(refused.to_string(), error);
    }
    let owned: &'static [&'static str] = registry::FAMILIES
        .iter()
        .find(|family| !family.sections.is_empty())
        .map(|family| &family.sections[..1])
        .expect("a family owning sections");
    let twice: &'static Descriptor = Box::leak(Box::new(Descriptor {
        sections: owned,
        ..registry::family("echo")
    }));
    let mut doubled = families();
    doubled.push(twice);
    assert!(Plan::new(doubled, &[]).is_err(), "a stem owned twice");
    let gating: &'static Descriptor = Box::leak(Box::new(Descriptor {
        gates: &["country-groups"],
        ..registry::family("echo")
    }));
    let mut doubled = families();
    doubled.push(gating);
    assert!(Plan::new(doubled, &[]).is_err(), "a product gated twice");
    let alias: &'static Descriptor = Box::leak(Box::new(Descriptor {
        aliases: &["divisions"],
        ..registry::family("echo")
    }));
    let mut doubled = families();
    doubled.push(alias);
    assert!(
        Plan::new(doubled, &[]).is_err(),
        "a spelling of two families"
    );
}

#[test]
fn a_stamp_moves_with_its_family_s_version_and_its_same_hour_suppliers_alone() {
    let rows = plan::native();
    let native = super::native::names();
    let bumped = |name: &str, version: u32| -> &'static Descriptor {
        Box::leak(Box::new(Descriptor {
            version,
            ..*registry::family_named(name).unwrap()
        }))
    };
    let plan = plan::test_native(&[bumped("identities", 2)], &native).unwrap();
    for (name, moved) in [
        ("identities", true),
        ("territory", false),
        ("event_feed", true),
        ("groups", false),
        ("air_superiority", false),
        ("stability", false),
    ] {
        assert_eq!(plan.stamp(name) != rows.stamp(name), moved, "{name}");
    }
    let plan = plan::test_native(&[bumped("groups", 3)], &native).unwrap();
    assert_ne!(plan.stamp("air_superiority"), rows.stamp("air_superiority"));
    assert_eq!(plan.stamp("identities"), rows.stamp("identities"));
    let plan = plan::test_native(&[bumped("land", 2)], &native).unwrap();
    assert_ne!(plan.stamp("land"), rows.stamp("land"));
    assert_eq!(plan.stamp("stability"), rows.stamp("stability"));
    assert_eq!(rows.stamp("nonexistent"), None);
    for (section, owner) in [
        ("preparation-root-stability-3", Some(family("stability"))),
        (
            "preparation-ledger-division-equipment-0",
            Some(family("division-equipment")),
        ),
        (
            "preparation-carry-production-lines-1",
            Some(Owner::Job("production-lines")),
        ),
        ("preparation-root-unknown-1", None),
    ] {
        assert_eq!(rows.owner(section), owner, "{section}");
    }
}
