use super::plan::{self, Backend, Lane, Owner, Plan};
use super::registry::{self, Descriptor, FamilyId};
use crate::prepared::series::catalogue_section;
use crate::products::{self, parts_race_prepared::Family as _};
use dashboard_keyed::Observer as _;
use replay_data::rows::TableId;
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
    let rows = plan::rows();
    for family in registry::FAMILIES {
        let backend = rows.backend(family.id.name());
        assert!(backend.is_some(), "{} has no backend", family.id.name());
        assert_eq!(
            backend == Some(Backend::Own),
            crate::prepared::pipeline::independent::row_names().contains(&family.id.name()),
            "{}",
            family.id.name()
        );
    }
    assert_eq!(rows.own().len(), registry::FAMILIES.len() - 13);
    for segments in [
        "stability",
        "supply",
        "army",
        "division-equipment",
        "country-styles",
        "politics_ledger",
        "stockpile",
        "fuel",
    ] {
        assert_eq!(
            rows.backend(segments),
            Some(Backend::Unavailable),
            "{segments}"
        );
    }
    for walked in ["resources", "land", "game_score", "autonomy", "measure"] {
        assert_eq!(rows.backend(walked), Some(Backend::Walk), "{walked}");
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
    let rows = plan::rows();
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
    let rows = plan::rows();
    for (section, owner) in [
        ("preparation-checkpoint-v1", Some(Owner::Coordinator)),
        ("preparation-map-progress-v1", Some(Owner::Coordinator)),
        ("preparation-coverage-2", Some(Owner::Walk)),
        ("preparation-branch-2", Some(Owner::Walk)),
        ("preparation-family-map-pages-12", Some(family("map-pages"))),
        (
            "preparation-family-production-lines-0",
            Some(Owner::Job("production-lines")),
        ),
        ("construction-continuation-4", Some(family("construction"))),
        ("event_feed-continuation-1", Some(family("event_feed"))),
        (
            "faction-state-continuation-2",
            Some(family("faction_state")),
        ),
        ("production-facts-v2/7", Some(family("production"))),
        (
            "production_catalogue-values-3",
            Some(family("production_catalogue")),
        ),
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
        ("occupation-3", Some(family("occupation"))),
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
        ("territory-identities-1", Some(family("territory"))),
        ("event-feed-identities-1", Some(family("event_feed"))),
        ("country-identities-1", Some(family("identities"))),
        ("naval-battles-1", Some(family("naval"))),
        ("naval-battle-windows-1", Some(family("naval"))),
        ("naval-repairs-values-1", Some(family("naval-repairs"))),
        ("war-casualties-state-1", Some(family("casualties"))),
        ("wars-series-1", Some(family("wars"))),
        ("map.checkpoints.3", Some(Owner::Job("map"))),
        ("map-pages.state.1.2", Some(family("map-pages"))),
        ("map-export-v3/endpoints/1", Some(Owner::Job("map-export"))),
        ("land-owners", Some(Owner::Job("land-owners"))),
        ("synthetic-alpha-1", None),
        ("airfield-1", None),
        // Spellings no writer builds.
        ("production-catalogue-values-3", None),
        ("fuel-branches-values-1", None),
        ("lend-lease-balance-1-catalogue", None),
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
    let rows = plan::rows();
    let facts = dashboard_base::prepared::facts::Namespace::new(
        &"a7".repeat(32),
        1,
        dashboard_base::map_export::ENDPOINT_FACTS,
    )
    .unwrap()
    .section();
    let air = products::air_force::prepared::Air::NAME;
    let navy = products::navy::prepared::Navy::NAME;
    let goals = format!("faction-goals-{}", 12_884_901_976_u64);
    let named = [
        // The coordinator's, the walk's, and every coverage page.
        ("preparation-checkpoint-v1".to_owned(), Owner::Coordinator),
        (
            crate::prepared::pipeline::MAP_PROGRESS.to_owned(),
            Owner::Coordinator,
        ),
        ("preparation-side-work-v1".to_owned(), Owner::Coordinator),
        ("source-metadata-v1".to_owned(), Owner::Coordinator),
        ("source-metadata-refusals-v1".to_owned(), Owner::Coordinator),
        (crate::prepared::coverage::coverage_section(1), Owner::Walk),
        ("preparation-branch-1".to_owned(), Owner::Walk),
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
        (
            registry::continuation_section(products::faction_state::prepared::FAMILY, 1),
            family("faction_state"),
        ),
        (registry::continuation_section(air, 1), family("air")),
        // Series and catalogues.
        (catalogue_section(air, 1), family("air")),
        (format!("{air}-parts-1-0"), family("air")),
        (format!("{navy}-maximum-1-0"), family("navy")),
        (catalogue_section(navy, 1), family("navy")),
        (
            catalogue_section(products::politics_state::history::FAMILY, 1),
            family("politics_state"),
        ),
        ("politics-state-1".to_owned(), family("politics_state")),
        (
            catalogue_section(products::production_catalogue::FAMILY, 1),
            family("production_catalogue"),
        ),
        (
            catalogue_section(products::lend_lease_balance::FAMILY, 1),
            family("lend_lease_balance"),
        ),
        (
            dashboard_economy::products::keyed::values_section(products::fuel_branches::FAMILY, 1),
            family("fuel_branches"),
        ),
        (
            dashboard_economy::products::keyed::values_section(products::market::FAMILY, 1),
            family("market"),
        ),
        (
            dashboard_economy::products::keyed::values_section(products::convoy_health::FAMILY, 1),
            family("convoys"),
        ),
        (
            dashboard_keyed::series_section(products::air_missions::AirMissionsObserver::NAME, 1),
            family("air-missions"),
        ),
        (
            catalogue_section(products::regional_weather::block::Observer::NAME, 1),
            family("regional-weather"),
        ),
        (
            dashboard_base::prepared::roots::root_section(products::style_runs::NAME, 1),
            family("country-styles"),
        ),
        (
            catalogue_section("military-air-losses", 1),
            family("air_ledger"),
        ),
        ("military-bombing-1-series".to_owned(), family("air_ledger")),
        (catalogue_section("naval-losses", 1), family("naval")),
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
        (catalogue_section("trade-flows", 1), family("trade")),
        (
            catalogue_section("territory-breakdown", 1),
            family("territory"),
        ),
        (
            catalogue_section("occupation-trend", 1),
            family("occupation"),
        ),
        (
            catalogue_section("resource-balance", 1),
            family("resources"),
        ),
        (catalogue_section("world-tension", 1), family("tension")),
        (
            format!(
                "{}-1",
                products::special_projects::prepared::completion_stream::FAMILY
            ),
            family("projects"),
        ),
        (
            catalogue_section(products::diplomacy_state::prepared::FAMILY_NAME, 1),
            family("diplomacy_state"),
        ),
        // Stability's parts and each faction's goals.
        ("parts-1".to_owned(), family("parts")),
        ("parts-1-names".to_owned(), family("parts")),
        (catalogue_section("parts", 1), family("parts")),
        (catalogue_section(&goals, 1), family("faction_state")),
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
        // The map, its pages and its export, and the land owners.
        ("map.checkpoints.1".to_owned(), Owner::Job("map")),
        ("map.gaps.1".to_owned(), Owner::Job("map")),
        ("map.resume".to_owned(), Owner::Job("map")),
        (
            dashboard_base::prepared_map::pages::section(1, 20),
            family("map-pages"),
        ),
        (
            dashboard_base::prepared_map::pages::progress_section(1),
            family("map-pages"),
        ),
        (
            dashboard_base::prepared_map::pages::state_section(1, 40),
            family("map-pages"),
        ),
        (
            dashboard_base::map_export::payload_section(1).unwrap(),
            Owner::Job("map-export"),
        ),
        (
            dashboard_base::map_export::payload_section(dashboard_base::map_export::TABLE_SECTION)
                .unwrap(),
            Owner::Job("map-export"),
        ),
        (facts.clone(), Owner::Job("map-export")),
        (format!("{facts}/map-keys"), Owner::Job("map-export")),
        (format!("{facts}/map-values"), Owner::Job("map-export")),
        ("land-owners".to_owned(), Owner::Job("land-owners")),
        // Reads of another family's state, under the reader's names.
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
        ("territory-identities-1".to_owned(), family("territory")),
        ("event-feed-identities-1".to_owned(), family("event_feed")),
    ];
    for (section, owner) in named {
        assert_eq!(rows.owner(&section), Some(owner), "{section}");
    }
}

#[test]
fn a_writer_refuses_every_section_another_publisher_owns() {
    let rows = plan::rows();
    let refused = |lane: Lane, section: &str| {
        rows.guard(lane)(section)
            .map(|()| false)
            .unwrap_or_else(|error| {
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{section}");
                true
            })
    };
    for section in [
        "measure-pp-1",
        "preparation-branch-1",
        "preparation-coverage-1",
        "land-owners",
        "map.checkpoints.1",
        "map-export-v2/endpoints/1",
    ] {
        assert!(!refused(Lane::Walk, section), "{section}");
    }
    for section in [
        "stability-1",
        "air-1-catalogue",
        "preparation-family-stability-1",
        "preparation-checkpoint-v1",
        "production-lines-state-v2-1-0",
        "map-pages.1.20",
    ] {
        assert!(refused(Lane::Walk, section), "{section}");
    }
    // A name no publisher registers is refused from every lane.
    for lane in [
        Lane::Walk,
        Lane::Own("parts"),
        Lane::Own("production-lines"),
    ] {
        for section in [
            "synthetic-unregistered-1",
            "airfield-1",
            "preparation-family-unknown-1",
            "preparation-facts-v2/ab/1/unregistered-column/keys",
        ] {
            assert!(refused(lane, section), "{lane:?} {section}");
        }
    }
    let parts = Lane::Own("parts");
    for section in [
        "parts-1",
        "parts-1-names",
        "parts-1-catalogue",
        "preparation-family-parts-1",
        "parts-continuation-1",
    ] {
        assert!(!refused(parts, section), "{section}");
    }
    for section in [
        "stability-1",
        "preparation-family-stability-1",
        "faction-goals-7-1-catalogue",
        "experience-1",
        "preparation-family-supply-1",
        "preparation-branch-1",
        "preparation-checkpoint-v1",
        "land-owners",
    ] {
        assert!(refused(parts, section), "{section}");
    }
    let lines = Lane::Own("production-lines");
    assert!(!refused(lines, "preparation-family-production-lines-1"));
    assert!(!refused(lines, "production-lines-history-v3-1"));
    assert!(refused(lines, "production-spent-1"));
}

#[test]
fn a_private_read_belongs_to_its_reader_while_the_reader_is_in_the_walk() {
    let rows = plan::rows();
    assert!(!rows.hourly("groups") && !rows.hourly("identities"));
    assert_eq!(rows.kept_as("country-groups"), "air-superiority-groups");

    let groups = plan::test_plan(&[], &["groups"]).unwrap();
    assert!(groups.registered("groups") && groups.hourly("groups"));
    assert!(!groups.registered("air_superiority") && groups.hourly("air_superiority"));
    assert_eq!(groups.kept_as("country-groups"), "air-superiority-groups");
    assert_eq!(
        groups.kept_as("country-group-changes"),
        "air-superiority-group-changes"
    );
    let walk = groups.guard(Lane::Walk);
    assert!(walk("air-superiority-groups-1").is_ok());
    assert!(walk("country-groups-1").is_err());
    assert!(groups.guard(Lane::Own("groups"))("air-superiority-groups-1").is_err());
    assert!(groups.guard(Lane::Own("groups"))("country-groups-1").is_ok());

    // The feed makes its own read of the identities; territory's is kept.
    let identities = plan::test_plan(&[], &["identities", "event_feed"]).unwrap();
    assert!(identities.hourly("identities"));
    assert!(!identities.hourly("event_feed"));
    assert_eq!(
        identities.kept_as("country-identities"),
        "territory-identities"
    );
    assert!(identities.guard(Lane::Walk)("territory-identities-1").is_ok());
    assert!(identities.guard(Lane::Own("event_feed"))("event-feed-identities-1").is_ok());

    let both = plan::test_plan(&[], &["identities", "territory", "event_feed"]).unwrap();
    assert!(!both.hourly("identities"));
    // A same-hour dependent is refused with its source, which no query join does.
    let observers = registry::family_named("identities").unwrap().observers;
    assert_eq!(
        names(observers),
        BTreeSet::from(["event_feed", "territory"])
    );
    assert!(
        !registry::family_named("territory")
            .unwrap()
            .observers
            .contains(&FamilyId::parse("identities").unwrap())
    );
}

#[test]
fn shared_queries_name_every_supplier_of_their_coverage_and_refusals() {
    let rows = plan::rows();
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
    plan::with_test_hourly(|| {
        let hourly = plan::current();
        // Supply health still joins the segment-prepared supply with the walk's identities.
        assert!(hourly.joint("event-feed").is_none());
        assert_eq!(names(hourly.joint("supply-health").unwrap()), supply);
        assert_eq!(hourly.gating("event-choices"), None);
        assert_eq!(
            hourly.gating("production-line-detail"),
            Some("production-lines")
        );
        let walk = vec![crate::prepared::coverage::WALK];
        assert_eq!(crate::prepared::pipeline::gate("event-feed"), walk);
        assert_eq!(
            crate::prepared::pipeline::gate("supply-health"),
            vec!["identities", "supply"]
        );
    });
}

/// Ordinary preparation's policies for a representative set of families and
/// jobs, written out rather than read back from the registry.
#[test]
fn ordinary_preparation_plans_exactly_these_policies() {
    let rows = plan::rows();
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
        ("country-measure", None),
        ("game-score", None),
        ("resource-balance", None),
        ("land-battles", None),
        ("autonomy", None),
        ("scoreboard", None),
        ("event-feed", None),
        ("territory", None),
        ("country-nukes", None),
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
        ("army-size", Some(vec!["army", "identities"])),
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

    let walked = ["resources", "land", "game_score", "autonomy", "measure"];
    for family in registry::FAMILIES {
        let name = family.id.name();
        let observers: &[&str] = match name {
            "identities" => &["territory", "event_feed"],
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
        let private: Option<(Vec<&str>, &[(&str, &str)])> = match name {
            "identities" => Some((
                vec!["territory"],
                &[("country-identities", "territory-identities")][..],
            )),
            "groups" => Some((
                vec!["air_superiority"],
                &[
                    ("country-groups", "air-superiority-groups"),
                    ("country-group-changes", "air-superiority-group-changes"),
                ][..],
            )),
            _ => None,
        };
        assert_eq!(
            family.private.map(|read| (
                read.readers.iter().map(|id| id.name()).collect::<Vec<_>>(),
                read.kept
            )),
            private,
            "private read of {name}"
        );
        assert_eq!(rows.hourly(name), walked.contains(&name), "{name} hourly");
    }

    assert_eq!(
        rows.walk_tables().into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::CountryAppearance,
            TableId::LexicalTokens,
            TableId::Politics,
            TableId::PoliticalParties,
            TableId::Autonomy,
            TableId::ReservedCountryNaming,
            TableId::Subjects,
            TableId::States,
            TableId::LandCombats,
            TableId::LandCombatSides,
            TableId::LandCombatProgress,
            TableId::LandCombatResults,
            TableId::LandEventCoverage,
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantUpgrades,
            TableId::VariantChanges,
            TableId::CountryGameScoreInputs,
            TableId::AirWingGameScoreStats,
            TableId::GameScoreMultipliers,
            TableId::CountryFactories,
            TableId::AirBaseRegistrations,
            TableId::Factions,
            TableId::FactionMembers,
            TableId::WeatherProvinces,
        ]),
        "the walk's tables"
    );
}

#[test]
fn a_plan_refuses_what_its_registry_or_backends_cannot_prepare() {
    let families = || registry::FAMILIES.iter().collect::<Vec<_>>();
    let rows = crate::prepared::pipeline::independent::row_names();
    for (own, error) in [
        (
            &["nonexistent"][..],
            "Unknown prepared family \"nonexistent\"",
        ),
        (
            &["army"][..],
            "\"army\" has no row implementation of its own",
        ),
        (&["groups", "groups"][..], "\"groups\" is selected twice"),
    ] {
        let refused = Plan::new(families(), own, &rows)
            .err()
            .expect("a refused plan");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(refused.to_string(), error);
    }
    let twice: &'static Descriptor = Box::leak(Box::new(Descriptor {
        sections: &["stability"],
        ..registry::family("echo")
    }));
    let mut doubled = families();
    doubled.push(twice);
    assert!(Plan::new(doubled, &[], &[]).is_err(), "a stem owned twice");
    let gating: &'static Descriptor = Box::leak(Box::new(Descriptor {
        gates: &["country-groups"],
        ..registry::family("echo")
    }));
    let mut doubled = families();
    doubled.push(gating);
    assert!(Plan::new(doubled, &["groups", "echo"], &["groups", "echo"]).is_err());
    let alias: &'static Descriptor = Box::leak(Box::new(Descriptor {
        aliases: &["divisions"],
        ..registry::family("echo")
    }));
    let mut doubled = families();
    doubled.push(alias);
    assert!(
        Plan::new(doubled, &[], &[]).is_err(),
        "a spelling of two families"
    );
}

#[test]
fn a_stamp_moves_with_its_family_s_version_and_its_same_hour_suppliers_alone() {
    let rows = plan::rows();
    let own = crate::prepared::pipeline::independent::row_names();
    let bumped = |name: &str, version: u32| -> &'static Descriptor {
        Box::leak(Box::new(Descriptor {
            version,
            ..*registry::family_named(name).unwrap()
        }))
    };
    let plan = plan::test_plan(&[bumped("identities", 2)], &own).unwrap();
    for (name, moved) in [
        ("identities", true),
        ("territory", true),
        ("event_feed", true),
        ("groups", false),
        ("air_superiority", false),
        ("stability", false),
    ] {
        assert_eq!(plan.stamp(name) != rows.stamp(name), moved, "{name}");
    }
    assert_eq!(plan.walk_stamp(), rows.walk_stamp());
    let plan = plan::test_plan(&[bumped("groups", 3)], &own).unwrap();
    assert_ne!(plan.stamp("air_superiority"), rows.stamp("air_superiority"));
    assert_eq!(plan.stamp("identities"), rows.stamp("identities"));
    // A family the walk prepares moves the walk's stamp, and no other family's.
    let plan = plan::test_plan(&[bumped("resources", 2)], &own).unwrap();
    assert_ne!(plan.walk_stamp(), rows.walk_stamp());
    assert_eq!(plan.stamp("stability"), rows.stamp("stability"));
    assert_eq!(rows.stamp("nonexistent"), None);
    for (section, owner) in [
        ("preparation-root-stability-3", Some(family("stability"))),
        (
            "preparation-ledger-division-equipment-0",
            Some(family("division-equipment")),
        ),
        ("preparation-state-map-pages-2", Some(family("map-pages"))),
        (
            "preparation-carry-production-lines-1",
            Some(Owner::Job("production-lines")),
        ),
        ("preparation-root-unknown-1", None),
    ] {
        assert_eq!(rows.owner(section), owner, "{section}");
    }
}
