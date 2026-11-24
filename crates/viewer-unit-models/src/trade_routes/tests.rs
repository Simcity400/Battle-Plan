use super::*;
use content_reader_map::province_graph::{Crossing, ProvinceGraph};
use viewer_gpu::ribbon_content::ArrowType;

fn links() -> Vec<CountryLinkRow> {
    [(10, 1), (11, 1), (20, 2), (30, 3)]
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .into()
}

fn countries(links: &[CountryLinkRow]) -> Countries<'_> {
    Countries::new(Available::Complete(links), 1)
}

fn route(country: u32, class: u8, other: u32, kind: u8) -> TradeRouteRow {
    TradeRouteRow {
        country,
        class,
        other,
        kind,
        from: 1,
        to: 2,
        path: vec![1, 2],
        source_state: Some(country),
        destination_state: Some(other),
    }
}

fn origin(state: u32, country: u32, list: u8, bad: bool) -> ResourceOriginRow {
    ResourceOriginRow {
        ordinal: state,
        state,
        country,
        list,
        bad,
        convoys_assigned: 3,
        ..Default::default()
    }
}

fn client(
    handle: u32,
    category: u8,
    owner: u32,
    other: u32,
    route: RouteKey,
    bad: bool,
) -> ConvoyClientRow {
    ConvoyClientRow {
        owner_ordinal: Some(handle),
        other_ordinal: Some(handle),
        handle,
        category,
        owner,
        other,
        route_country: route.0,
        route_class: route.1,
        route_other: route.2,
        bad,
        convoys_assigned: 7,
        ..Default::default()
    }
}

fn summary(lines: &[Line<'_>]) -> Vec<(Layer, RouteKey, bool, bool)> {
    lines
        .iter()
        .map(|line| {
            (
                line.layer,
                (line.route.country, line.route.class, line.route.other),
                line.bad,
                line.reverse,
            )
        })
        .collect()
}

#[test]
fn each_layer_takes_the_viewed_countrys_clients_and_its_route_kinds() {
    let links = links();
    let routes = vec![
        route(10, 0, 5, NAVAL_ROUTE),
        route(10, 0, 6, LAND_ROUTE),
        route(20, 0, 7, NAVAL_ROUTE),
        route(10, 0, 8, NAVAL_ROUTE),
        route(10, 2, 1, 3),
        route(20, 1, 10, NAVAL_ROUTE),
        route(11, 1, 20, NAVAL_ROUTE),
        route(10, 1, 20, LAND_ROUTE),
        route(20, 1, 30, LAND_ROUTE),
    ];
    let origins = vec![
        origin(5, 10, OWN_ORIGINS, true),
        origin(6, 10, OWN_ORIGINS, false),
        origin(7, 20, OWN_ORIGINS, false),
        origin(8, 10, 1, false),
    ];
    let clients = vec![
        client(1, SUPPLY_TRANSFER, 10, 0, (10, 2, 1), false),
        client(2, IMPORT, 10, 20, (20, 1, 10), false),
        client(3, IMPORT, 20, 11, (11, 1, 20), true),
        client(4, LEND_LEASE, 10, 20, (10, 1, 20), false),
        client(5, LEND_LEASE, 20, 10, (20, 1, 10), false),
        client(6, MARKET, 20, 10, (20, 1, 10), false),
        client(7, MARKET, 20, 10, (20, 1, 10), true),
        client(8, MARKET, 10, 20, (10, 1, 20), false),
        client(9, IMPORT, 20, 30, (20, 1, 30), false),
    ];
    let tables: Tables<'_> = Tables {
        origins: &origins,
        clients: &clients,
        routes: &routes,
        regional: &[],
    };
    let mut report = TradeRouteReport::default();
    let lines = lines(&tables, &countries(&links), &mut report);
    assert_eq!(
        summary(&lines),
        [
            (Layer::Production, (10, 0, 5), true, false),
            (Layer::SupplyTransfer, (10, 2, 1), false, false),
            (Layer::Export, (11, 1, 20), true, false),
            (Layer::Import, (20, 1, 10), false, false),
            (Layer::LendLeaseReceived, (20, 1, 10), false, false),
            (Layer::MarketBought, (20, 1, 10), true, false),
            (Layer::MarketSold, (10, 1, 20), false, false),
        ]
    );
    assert_eq!(report.lines, [2, 1, 1, 1, 1, 1, 1, 1]);
    assert_eq!(report.filtered, 2);
    assert!(report.refused.is_empty(), "{:?}", report.refused);
    assert_eq!(lines[5].convoys_assigned, 0);
    assert_eq!(lines[3].convoys_assigned, 7);
}

#[test]
fn layer_kind_filters_match_the_game() {
    let admitted = |layer: Layer| {
        [NO_ROUTE, LAND_ROUTE, NAVAL_ROUTE, 3]
            .into_iter()
            .filter(|&kind| layer.admits(kind))
            .collect::<Vec<_>>()
    };
    assert_eq!(admitted(Layer::Production), [NAVAL_ROUTE]);
    assert_eq!(
        admitted(Layer::SupplyTransfer),
        [NO_ROUTE, LAND_ROUTE, NAVAL_ROUTE, 3]
    );
    for layer in [
        Layer::Export,
        Layer::Import,
        Layer::MarketBought,
        Layer::MarketSold,
    ] {
        assert_eq!(admitted(layer), [LAND_ROUTE, NAVAL_ROUTE], "{layer:?}");
    }
    for layer in [Layer::LendLeaseGiven, Layer::LendLeaseReceived] {
        assert_eq!(admitted(layer), [NAVAL_ROUTE], "{layer:?}");
    }
    let convoys: Vec<_> = Layer::ALL
        .into_iter()
        .filter(|layer| layer.carries_convoys())
        .collect();
    assert_eq!(convoys, Layer::ALL[..6]);
}

#[test]
fn import_and_lend_lease_direction_uses_the_native_client_country() {
    let links = links();
    let routes = vec![
        route(20, 1, 10, NAVAL_ROUTE),
        route(10, 1, 20, NAVAL_ROUTE),
        route(10, 2, 3, NAVAL_ROUTE),
    ];
    let clients = vec![
        client(1, IMPORT, 11, 20, (10, 1, 20), false),
        client(2, IMPORT, 10, 20, (20, 1, 10), false),
        client(3, LEND_LEASE, 10, 20, (10, 1, 20), false),
        client(4, SUPPLY_TRANSFER, 10, 0, (10, 2, 3), false),
        client(5, MARKET, 10, 20, (10, 1, 20), false),
    ];
    let tables: Tables<'_> = Tables {
        origins: &[],
        clients: &clients,
        routes: &routes,
        regional: &[],
    };
    let mut report = TradeRouteReport::default();
    let lines = lines(&tables, &countries(&links), &mut report);
    let reversed: Vec<_> = summary(&lines)
        .into_iter()
        .map(|(layer, _, _, reverse)| (layer, reverse))
        .collect();
    assert_eq!(
        reversed,
        [
            (Layer::SupplyTransfer, false),
            (Layer::Import, true),
            (Layer::Import, false),
            (Layer::LendLeaseGiven, false),
            (Layer::MarketSold, false),
        ]
    );
}

#[test]
fn lend_lease_delivery_keeps_giver_to_receiver_direction_in_both_views() {
    let links = links();
    let routes = vec![route(10, 1, 20, NAVAL_ROUTE)];
    let clients = vec![client(1, LEND_LEASE, 10, 20, (10, 1, 20), false)];
    let tables: Tables<'_> = Tables {
        origins: &[],
        clients: &clients,
        routes: &routes,
        regional: &[],
    };
    for (view, layer) in [(1, Layer::LendLeaseGiven), (2, Layer::LendLeaseReceived)] {
        let mut report = TradeRouteReport::default();
        let countries = Countries::new(Available::Complete(&links), view);
        let lines = lines(&tables, &countries, &mut report);
        assert_eq!(summary(&lines), [(layer, (10, 1, 20), false, false)]);
        assert!(report.refused.is_empty());
    }
    let clients = vec![client(1, LEND_LEASE, 10, 11, (10, 1, 11), false)];
    let routes = vec![route(10, 1, 11, NAVAL_ROUTE)];
    let tables: Tables<'_> = Tables {
        clients: &clients,
        routes: &routes,
        ..tables
    };
    let mut report = TradeRouteReport::default();
    let lines = lines(&tables, &countries(&links), &mut report);
    assert_eq!(
        summary(&lines),
        [
            (Layer::LendLeaseGiven, (10, 1, 11), false, true),
            (Layer::LendLeaseReceived, (10, 1, 11), false, true),
        ]
    );
    assert!(report.refused.is_empty());
}

#[test]
fn an_unresolved_country_is_refused_by_name() {
    let links = links();
    let clients = vec![client(1, IMPORT, 99, 20, (20, 1, 99), false)];
    let tables: Tables<'_> = Tables {
        origins: &[],
        clients: &clients,
        routes: &[],
        regional: &[],
    };
    let mut report = TradeRouteReport::default();
    assert!(lines(&tables, &countries(&links), &mut report).is_empty());
    assert!(
        report
            .refused
            .keys()
            .any(|reason| reason.contains("country 99 has no CountryLinks row")),
        "{:?}",
        report.refused
    );
}

fn defines() -> TradeRouteDefines {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "graphics.lua",
            br"NDefines = { NGraphics = {
                TRADEROUTE_SMOOTHNESS = 0.5,
                TRADEROUTE_SMOOTHEN_PASSES = 1,
                TRADE_ROUTE_LINE_OFFSET = 0.25,
                TRADE_ROUTE_MAX_LINES = 4,
                TRADE_ROUTE_INTERSECT_DISTANCE_MULT = 10,
                TRADE_ROUTE_REGIONAL_BAD_EFFICIENCY_THRESHOLD = 0.5,
                TRADE_ROUTE_BAD_EFFICIENCY_ROUTE_COLOR = { 0.9, 0.9, 0.1, 0.5 },
                TRADE_ROUTE_BAD_EFFICIENCY_HOTSPOT_COLOR = { 0.9, 0.1, 0.1, 0.5 },
                TRADE_ROUTE_PRODUCTION_TRANSFER_COLOR = { 0.1, 0, 0, 0.5 },
                TRADE_ROUTE_SUPPLIES_TRANSFER_COLOR = { 0.25, 0.5, 0.8 },
                TRADE_ROUTE_RESOURCE_EXPORT_COLOR = { 0.3, 0, 0, 0.5 },
                TRADE_ROUTE_RESOURCE_IMPORT_COLOR = { 0.4, 0, 0, 0.5 },
                TRADE_ROUTE_LEND_LEASE_EXPORT_COLOR = { 0.5, 0, 0, 0.5 },
                TRADE_ROUTE_LEND_LEASE_IMPORT_COLOR = { 0.6, 0, 0, 0.5 },
                TRADE_ROUTE_INTERNATIONAL_MARKET_COLOR = { 0.7, 0, 0, 0.5 },
                TRADE_ROUTE_NUM_CONVOYS_SCALE_FACTOR = 0.25,
                TRADE_ROUTE_MAX_NUM_CONVOYS = 3,
                TRADE_ROUTE_CONVOY_SPEED = 2,
                TRADE_ROUTE_CONVOY_SLEEP_TIME = 1,
                TRADE_ROUTE_CONVOY_ROUTE_OFFSET = 0.5,
                UNITS_DISTANCE_CUTOFF = 100,
            } }",
        )
        .unwrap();
    TradeRouteDefines::read(&defines).unwrap()
}

#[test]
fn a_bad_naval_line_takes_the_hotspot_variant_where_the_viewed_country_is_inefficient() {
    let links = links();
    let regional = vec![
        RegionalConvoyEfficiencyRow {
            country: 10,
            region: 100,
            efficiency: 79_999,
        },
        RegionalConvoyEfficiencyRow {
            country: 11,
            region: 101,
            efficiency: 80_000,
        },
        RegionalConvoyEfficiencyRow {
            country: 20,
            region: 102,
            efficiency: 1,
        },
        RegionalConvoyEfficiencyRow {
            country: 10,
            region: 103,
            efficiency: 1,
        },
    ];
    let mut report = TradeRouteReport::default();
    let efficiency = regional_efficiency(&regional, &countries(&links), &mut report);
    assert_eq!(
        efficiency,
        BTreeMap::from([(100, 79_999), (101, 80_000), (103, 1)])
    );
    let stops = Stops {
        provinces: vec![1, 2, 3, 4],
        regions: vec![100, 101, 102, 103],
    };
    let naval = route(10, 0, 5, NAVAL_ROUTE);
    fn line(bad: bool, route: &TradeRouteRow) -> Line<'_> {
        Line {
            layer: Layer::Production,
            route,
            bad,
            reverse: false,
            convoys_assigned: 0,
            assigned: super::Assigned::None,
            client: 0,
        }
    }
    assert_eq!(
        control_variants(&line(true, &naval), &stops, &efficiency, 80_000),
        [1, 0, 0, 0, 0, 0, 1]
    );
    assert_eq!(
        control_variants(&line(false, &naval), &stops, &efficiency, 80_000),
        [0; 7]
    );
    let land = route(10, 0, 5, LAND_ROUTE);
    assert_eq!(
        control_variants(&line(true, &land), &stops, &efficiency, 80_000),
        [0; 7]
    );
    let adjacent_hotspots = Stops {
        provinces: vec![1, 2, 3],
        regions: vec![100, 103, 101],
    };
    assert_eq!(
        control_variants(&line(true, &naval), &adjacent_hotspots, &efficiency, 80_000),
        [1, 1, 1, 0, 0]
    );
}

#[test]
fn convoy_counts_round_the_scaled_assignment_into_one_to_the_maximum() {
    let defines = ConvoyDefines {
        scale_factor: 0.3,
        max: 4,
        speed: 0.6,
        sleep: 3.0,
        route_offset: 0.5,
        cutoff: 120.0,
    };
    let counts: Vec<u32> = [0, 1, 2, 5, 7, 10, 100]
        .into_iter()
        .map(|assigned| convoy_count(assigned, &defines))
        .collect();
    assert_eq!(counts, [0, 1, 1, 2, 2, 3, 4]);
    assert!(convoys_shown(119.9, &defines, true, true));
    assert!(!convoys_shown(120.0, &defines, true, true));
    assert!(!convoys_shown(50.0, &defines, false, true));
    assert!(!convoys_shown(50.0, &defines, true, false));
}

#[test]
fn convoys_made_again_from_the_lines_kept_are_counted_and_numbered_as_a_whole_build_does() {
    let defines = ConvoyDefines { scale_factor: 0.3, max: 4, speed: 0.6, sleep: 3.0, route_offset: 0.5, cutoff: 120.0 };
    let sailing = |assigned| Sailing { assigned, points: vec![[1., 2.], [3., 4.]], map_width: 8. };
    let kept = TradeRoutes {
        sailing: vec![sailing(Assigned::Origin(0)), sailing(Assigned::Client(1)), sailing(Assigned::Client(0)), sailing(Assigned::None)],
        convoy_entity: Some("convoy".to_owned()),
        ..TradeRoutes::default()
    };
    let made = sailed(&kept, &defines, &[0], &[4, 10]).expect("the entity the build resolved");
    assert_eq!(made.iter().map(|line| (line.seed, line.count)).collect::<Vec<_>>(), [(0, convoy_count(10, &defines)), (1, convoy_count(4, &defines))]);
    assert!(made.iter().all(|line| line.entity == "convoy" && line.points == [[1., 2.], [3., 4.]] && line.map_width == 8.));
    assert!(sailed(&TradeRoutes { sailing: kept.sailing.clone(), ..TradeRoutes::default() }, &defines, &[0], &[4, 10]).is_none());
    assert_eq!(sailed(&kept, &defines, &[0], &[0, 0]).expect("no line sails"), []);
}

#[test]
fn the_convoy_entity_falls_back_to_the_engines_own() {
    let declared =
        |name: &str| name == "east_gfx_traderoute_convoy" || name == FALLBACK_CONVOY_ENTITY;
    assert_eq!(
        convoy_entity("east_gfx", &declared).unwrap(),
        "east_gfx_traderoute_convoy"
    );
    assert_eq!(
        convoy_entity("west_gfx", &declared).unwrap(),
        FALLBACK_CONVOY_ENTITY
    );
    assert!(convoy_entity("west_gfx", &|_| false).is_err());
}

#[test]
fn convoy_offset_changes_only_interior_controls_using_unnormalized_deltas() {
    assert_eq!(
        convoys::convoy_path(&[[0., 0.], [2., 0.], [2., 4.], [5., 4.]], 0.5),
        [[0., 0.], [4., 0.], [2., 2.5], [5., 4.]]
    );
}

#[test]
fn overlapping_lines_alternate_sides_modulo_the_line_count() {
    let offsets: Vec<f32> = (0..8).map(|index| overlap_offset(index, 6, 0.5)).collect();
    assert_eq!(offsets, [0.0, -0.5, 1.0, -1.5, 2.0, -2.5, 0.0, -0.5]);
    let smoothing = Smoothing {
        smoothness: 0,
        passes: 0,
    };
    let straight = [[0.0, 0.0], [10.0, 0.0]];
    let shaped = geometry::shape(&straight, &[0, 0], false, smoothing);
    assert_eq!(shaped.controls, [[0, 0], [1_000_000, 0]]);
}

#[test]
fn shaping_preserves_region_boundary_controls_in_both_directions() {
    let smoothing = Smoothing {
        smoothness: 0,
        passes: 1,
    };
    let points = [
        [0.0, 0.0],
        [5.0, 0.0],
        [10.0, 0.0],
        [15.0, 0.0],
        [20.0, 0.0],
    ];
    let variants = [0, 0, 1, 1, 1];
    let forward = geometry::shape(&points, &variants, false, smoothing);
    assert_eq!(forward.controls.len(), 9);
    assert_eq!(forward.variants, [0, 0, 0, 1, 1, 1, 1, 1, 1]);
    let backward = geometry::shape(&points, &variants, true, smoothing);
    assert_eq!(backward.controls[0], [2_000_000, 0]);
    assert_eq!(backward.variants, [1, 1, 1, 1, 1, 1, 0, 0, 0]);
}

#[test]
fn subdivision_takes_the_maximum_on_both_sides_of_an_isolated_hotspot() {
    let points = [
        [0.0, 0.0],
        [5.0, 0.0],
        [10.0, 0.0],
        [15.0, 0.0],
        [20.0, 0.0],
    ];
    let variants = [0, 0, 1, 0, 0];
    for (passes, expected) in [
        (0, vec![0, 0, 1, 0, 0]),
        (1, vec![0, 0, 0, 1, 1, 1, 0, 0, 0]),
        (2, vec![0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0]),
    ] {
        let shaped = geometry::shape(
            &points,
            &variants,
            false,
            Smoothing {
                smoothness: 0,
                passes,
            },
        );
        assert_eq!(shaped.variants, expected, "subdivision passes {passes}");
        assert_eq!(shaped.controls.len(), shaped.variants.len());
    }
}

#[test]
fn the_ribbon_widens_and_takes_the_secondary_colour_in_hotspot_regions() {
    let arrow = ArrowType {
        name: TRADE_ROUTE_TYPE_NAME.into(),
        body_width: [0.8, 0.8],
        body_uv_scale: 0.5,
        secondary_body_uv_scale: 1.0,
        texture_loop: 3.0,
        ..Default::default()
    };
    let values = vec![95u8; 64 * 64];
    let heights = HeightField {
        width: 64,
        height: 64,
        values: &values,
    };
    let shaped = Shaped {
        controls: vec![[0, 0], [1_000_000, 0], [2_000_000, 0]],
        variants: vec![0, 1, 1],
    };
    let mut field = OrderField::default();
    let points = geometry::emit(&mut field, &arrow, &shaped, [0.1; 4], [0.9; 4], &heights).unwrap();
    assert_eq!(points.len(), 3);
    let draw = &field.draws[0];
    assert_eq!(draw.kind, dashboard_core::protocol::PlanKind::TradeRoute);
    assert_eq!((draw.color, draw.secondary_color), ([0.1; 4], [0.9; 4]));
    assert_eq!(draw.uv_to_length, 3.0);
    let across = |pair: usize| {
        let [a, b] = [&field.vertices[pair * 2], &field.vertices[pair * 2 + 1]];
        (a.position[2] - b.position[2], a.uv_head_variant[3])
    };
    let (regular, variant) = across(0);
    assert!((regular - 0.8).abs() < 1e-5 && variant == 0.0, "{regular}");
    let (hotspot, variant) = across(2);
    assert!((hotspot - 1.6).abs() < 1e-5 && variant == 1.0, "{hotspot}");
    assert!((field.vertices[0].position[1] - 9.51).abs() < 1e-4);
}

fn graph(edges: &[(u32, Vec<u32>)], crossings: &[((u32, u32), Crossing)]) -> ProvinceGraph {
    ProvinceGraph {
        edges: edges
            .iter()
            .map(|(from, to)| (*from, to.iter().map(|&to| (to, Some(1))).collect()))
            .collect(),
        crossings: crossings.iter().cloned().collect(),
        ..Default::default()
    }
}

#[test]
fn trade_search_visits_provinces_and_inserts_crossings_instead_of_region_waypoints() {
    let graph = graph(
        &[
            (1, vec![2]),
            (2, vec![3, 4]),
            (3, vec![5]),
            (4, vec![5]),
            (5, vec![6]),
            (6, vec![]),
        ],
        &[],
    );
    let geography = content_reader_map::province_geometry::ProvinceGeometry {
        width: 100,
        height: 100,
        centroids: (1..=6).map(|p| (p, [p as i32 * 10, 0])).collect(),
        land: [1, 6].into(),
        ..Default::default()
    };
    let region_of = BTreeMap::from([(1, 99), (2, 100), (3, 100), (4, 100), (5, 101), (6, 102)]);
    let region_provinces = BTreeMap::from([(100, vec![2, 3, 4]), (101, vec![5])]);
    let state_provinces = BTreeMap::from([(7, vec![1]), (8, vec![6])]);
    let state_of = (1..=6).map(|p| (p, if p == 6 { 8 } else { 7 })).collect();
    let ports = BTreeMap::from([(1, 2), (6, 5)]);
    let pixels = |p| Some(if p == 4 { 200 } else { 1 });
    let position = |p| Some([p as f32 * 10., 0., 0.]);
    let crossing = |a, b| Some([((a + b) * 5) as i32, 2]);
    let content = RouteContent {
        graph: &graph,
        geography: &geography,
        region_of: &region_of,
        region_provinces: &region_provinces,
        state_provinces: &state_provinces,
        state_of: &state_of,
        ports: &ports,
        pixels: &pixels,
        position: &position,
        crossing: &crossing,
    };
    let route = TradeRouteRow {
        kind: NAVAL_ROUTE,
        from: 1,
        to: 6,
        path: vec![100, 101],
        ..Default::default()
    };
    let stops = stops(&route, &content).unwrap().unwrap();
    assert_eq!(stops.provinces, [1, 2, 3, 5, 6]);
    assert_eq!(stops.regions, [99, 100, 100, 101, 102]);
    let mut shared = BTreeMap::new();
    let first = geometry::province_uses(&stops, &mut shared);
    let points = geometry::points(&stops, &content, 100., &first, 6, 0.5).unwrap();
    assert_eq!(points.len(), 9);
    assert_eq!(points[1], [15., 2.]);
    let other = Stops {
        provinces: vec![2, 3, 5, 6],
        regions: vec![],
    };
    assert_eq!(geometry::province_uses(&other, &mut shared), [1, 1, 1, 1]);
    let displaced = geometry::points(&stops, &content, 100., &[1; 5], 6, 0.5).unwrap();
    assert_eq!(displaced[0], points[0]);
    assert_eq!(displaced[8], points[8]);
    assert_ne!(displaced[4], points[4]);
    assert_eq!(
        geometry::shape(
            &points,
            &vec![0; points.len()],
            false,
            Smoothing {
                smoothness: 65_000,
                passes: 2
            }
        )
        .controls
        .len(),
        33
    );
    let mut restricted = graph.clone();
    restricted.crossings.insert(
        (3, 5),
        Crossing {
            kind: content_reader_map::adjacencies::SEA_CROSSING,
            through: 9,
            rule: None,
        },
    );
    let content = RouteContent {
        graph: &restricted,
        ..content
    };
    assert!(
        border_province(100, 101, &content)
            .unwrap_err()
            .contains("restricted trade crossing")
    );
}

#[test]
fn swap_removed_origins_keep_native_shared_corridor_displacement() {
    let links = links();
    let mut origins = vec![
        origin(1, 10, 0, false),
        origin(2, 10, 0, false),
        origin(3, 10, 0, false),
    ];
    for (ordinal, row) in origins.iter_mut().enumerate() {
        row.ordinal = ordinal as u32;
    }
    origins.swap_remove(0);
    for (ordinal, row) in origins.iter_mut().enumerate() {
        row.ordinal = ordinal as u32;
    }
    origins.sort_by_key(|row| row.state);
    let routes = vec![route(10, 0, 2, NAVAL_ROUTE), route(10, 0, 3, NAVAL_ROUTE)];
    let tables: Tables<'_> = Tables {
        origins: &origins,
        clients: &[],
        routes: &routes,
        regional: &[],
    };
    let mut report = TradeRouteReport::default();
    let lines = lines(&tables, &countries(&links), &mut report);
    let corridor = Stops {
        provinces: vec![7, 8],
        regions: vec![10, 10],
    };
    let mut shared = BTreeMap::new();
    let displaced: Vec<_> = lines
        .iter()
        .map(|line| {
            (
                line.route.other,
                geometry::province_uses(&corridor, &mut shared),
            )
        })
        .collect();
    assert_eq!(displaced, [(3, vec![0, 0]), (2, vec![1, 1])]);
    assert!(report.refused.is_empty());
}

#[test]
fn directional_order_precedes_handles_and_missing_positions_are_refused() {
    let links = links();
    let routes = vec![route(20, 1, 10, NAVAL_ROUTE), route(30, 1, 10, NAVAL_ROUTE)];
    for category in [LEND_LEASE, MARKET] {
        let mut clients = vec![
            client(1, category, 20, 10, (20, 1, 10), false),
            client(2, category, 30, 10, (30, 1, 10), false),
        ];
        clients[0].other_ordinal = Some(1);
        clients[1].other_ordinal = Some(0);
        let tables: Tables<'_> = Tables {
            origins: &[],
            clients: &clients,
            routes: &routes,
            regional: &[],
        };
        let mut report = TradeRouteReport::default();
        let ordered = lines(&tables, &countries(&links), &mut report);
        assert_eq!(
            ordered
                .iter()
                .map(|line| line.route.country)
                .collect::<Vec<_>>(),
            [30, 20]
        );
        clients[1].other_ordinal = None;
        let tables: Tables<'_> = Tables {
            origins: &[],
            clients: &clients,
            routes: &routes,
            regional: &[],
        };
        let mut report = TradeRouteReport::default();
        let ordered = lines(&tables, &countries(&links), &mut report);
        assert_eq!(ordered.len(), 1);
        assert!(
            report
                .refused
                .keys()
                .any(|reason| reason.contains("native directional list position unavailable"))
        );
    }
}

#[test]
fn market_groups_native_state_endpoints_across_country_keys_and_keeps_the_first_route() {
    let links = vec![
        CountryLinkRow {
            id: 10,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 20,
            canonical_slot: 2,
        },
        CountryLinkRow {
            id: 30,
            canonical_slot: 3,
        },
        CountryLinkRow {
            id: 40,
            canonical_slot: 4,
        },
        CountryLinkRow {
            id: 50,
            canonical_slot: 5,
        },
    ];
    for (view, layer, buyer_side) in [
        (1, Layer::MarketSold, false),
        (2, Layer::MarketBought, true),
    ] {
        let keys = if buyer_side {
            [(10, 1, 20), (30, 1, 20), (40, 1, 20), (50, 1, 20)]
        } else {
            [(10, 1, 20), (10, 1, 30), (10, 1, 40), (10, 1, 50)]
        };
        let mut routes: Vec<_> = keys
            .iter()
            .enumerate()
            .map(|(i, &(country, class, other))| {
                route(
                    country,
                    class,
                    other,
                    if i == 2 { LAND_ROUTE } else { NAVAL_ROUTE },
                )
            })
            .collect();
        for route in &mut routes {
            route.source_state = Some(70);
            route.destination_state = Some(80);
        }
        routes[1].from = 3;
        routes[1].to = 4;
        routes[1].path = vec![9];
        routes[3].source_state = Some(80);
        routes[3].destination_state = Some(70);
        let mut clients: Vec<_> = [1, 0, 2, 3]
            .into_iter()
            .enumerate()
            .map(|(i, k)| client(i as u32 + 1, MARKET, keys[k].0, keys[k].2, keys[k], k == 1))
            .collect();
        clients[0].owner_ordinal = Some(1);
        clients[0].other_ordinal = Some(1);
        clients[1].owner_ordinal = Some(0);
        clients[1].other_ordinal = Some(0);
        let tables: Tables<'_> = Tables {
            origins: &[],
            clients: &clients,
            routes: &routes,
            regional: &[],
        };
        let countries = Countries::new(Available::Complete(&links), view);
        let mut report = TradeRouteReport::default();
        let grouped = lines(&tables, &countries, &mut report);
        assert_eq!(
            summary(&grouped),
            [
                (layer, keys[0], true, false),
                (layer, keys[2], false, false),
                (layer, keys[3], false, false),
            ]
        );
        assert!(std::ptr::eq(grouped[0].route, &raw const routes[0]));
        let corridor = Stops {
            provinces: vec![7, 8],
            regions: vec![],
        };
        let mut shared = BTreeMap::new();
        let uses: Vec<_> = grouped
            .iter()
            .map(|_| geometry::province_uses(&corridor, &mut shared))
            .collect();
        assert_eq!(uses, [vec![0, 0], vec![1, 1], vec![2, 2]]);
        assert_eq!(report.lines[layer.index()], 3);
        assert!(report.refused.is_empty());
        let mut missing_routes = routes.clone();
        missing_routes[0].source_state = None;
        missing_routes[1].destination_state = None;
        let tables: Tables<'_> = Tables {
            origins: &[],
            clients: &clients,
            routes: &missing_routes,
            regional: &[],
        };
        let mut report = TradeRouteReport::default();
        assert_eq!(lines(&tables, &countries, &mut report).len(), 2);
        assert_eq!(
            report.refused[&format!(
                "a {} line's native endpoint states unavailable",
                layer.name()
            )],
            2
        );
    }
}
