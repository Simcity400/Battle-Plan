
use replay_data::{
    rows::BuildingRow, rows::CountryLinkRow, rows::CountryOilFactorRow, rows::LexicalTokenRow,
    rows::ResourceDefinitionRow, rows::ResourceOriginRow, rows::ResourceRightRow,
    rows::StateBaseResourceRow, rows::StateResourceInputRow, rows::StateResourceRow,
    rows::StateRow, rows::StateTemporaryResourceRow, rows::TableId, rows::TableRows,
    rows::TradeRouteRow,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

use super::Content;

pub const GER: u32 = 5;
pub const GER_ALIAS: u32 = 15;
pub const AFG: u32 = 9;
pub const GER_SLOT: u32 = 0;
pub const AFG_SLOT: u32 = 1;

pub const OIL: u32 = 101;
pub const STEEL: u32 = 102;
pub const RUBBER: u32 = 103;
pub const REFINERY: u32 = 104;
pub const INFRASTRUCTURE: u32 = engine_native::cbuilding_database::INFRASTRUCTURE_TOKEN;

pub const MIXED: u32 = 1;
pub const EMPTY: u32 = 2;
pub const GATED: u32 = 3;
pub const COSTED: u32 = 4;
pub const ORPHAN: u32 = 5;
pub const GRANTED: u32 = 6;
pub const GIVEN: u32 = 7;
pub const ALIASED: u32 = 8;

pub fn content() -> Content {
    let mut buildings = content_reader_key_names::buildings::Buildings::default();
    for (name, grants) in [
        ("infrastructure", Vec::new()),
        ("refinery", vec![("oil".to_owned(), 100_000)]),
    ] {
        buildings.types.insert(
            name.to_owned(),
            content_reader_key_names::buildings::BuildingType {
                name: name.to_owned(),
                local_resources: grants,
                ..Default::default()
            },
        );
    }
    Content {
        resources: ["oil", "steel", "rubber"]
            .into_iter()
            .enumerate()
            .map(|(at, token)| content_reader_key_names::resources::Resource {
                token: token.to_owned(),
                icon_frame: Some(at as u32 + 1),
            })
            .collect(),
        buildings,
        bonuses: [15_000, 20_000],
        oil: "oil".to_owned(),
        warnings: Vec::new(),
    }
}

pub fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn origin(state: u32, country: u32, list: u8, efficiency: i64) -> ResourceOriginRow {
    ResourceOriginRow {
        state,
        country,
        list,
        efficiency,
        route_factor: 100_000,
        ..Default::default()
    }
}

fn input(state: u32, gate: u8, infrastructure_level: Option<i32>) -> StateResourceInputRow {
    StateResourceInputRow {
        state,
        gate,
        origin: 1,
        infrastructure_level,
        ..Default::default()
    }
}

fn base(state: u32, resource: u32, amount: i64) -> StateBaseResourceRow {
    StateBaseResourceRow {
        state,
        resource,
        ordinal: 0,
        amount,
    }
}

fn building(location: u32, building_type: u32, level: i16, healthy_level: i16) -> BuildingRow {
    BuildingRow {
        location,
        building_type,
        level,
        healthy_level,
        active: true,
        ..Default::default()
    }
}

pub fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: GER,
                canonical_slot: GER_SLOT,
            },
            CountryLinkRow {
                id: AFG,
                canonical_slot: AFG_SLOT,
            },
            CountryLinkRow {
                id: GER_ALIAS,
                canonical_slot: GER_SLOT,
            },
        ]),
    );
    let mut tokens: Vec<_> = [
        (OIL, "oil"),
        (STEEL, "steel"),
        (RUBBER, "rubber"),
        (REFINERY, "refinery"),
        (INFRASTRUCTURE, "infrastructure"),
    ]
    .into_iter()
    .map(|(token, name)| LexicalTokenRow {
        token,
        literal: name.as_bytes().to_vec(),
    })
    .collect();
    tokens.sort_by_key(|row| row.token);
    put(&mut frame, TableRows::LexicalTokens(tokens));
    put(
        &mut frame,
        TableRows::ResourceDefinitions(
            [OIL, STEEL, RUBBER]
                .into_iter()
                .enumerate()
                .map(|(at, token)| ResourceDefinitionRow {
                    id: at as u32 + 1,
                    token,
                    active: 1,
                    factory_cost: 0,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::States(
            [
                (MIXED, GER),
                (EMPTY, GER),
                (GATED, GER),
                (COSTED, AFG),
                (ORPHAN, AFG),
                (GRANTED, GER),
                (GIVEN, GER),
                (ALIASED, GER),
            ]
            .into_iter()
            .map(|(id, controller)| StateRow {
                id,
                owner: controller,
                controller,
                ..Default::default()
            })
            .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::StateResourceInputs(vec![
            input(MIXED, 0, Some(4)),
            input(EMPTY, 0, None),
            input(GATED, 1, None),
            input(COSTED, 0, None),
            input(ORPHAN, 0, None),
            input(GRANTED, 0, None),
            input(GIVEN, 0, None),
            input(ALIASED, 0, None),
        ]),
    );
    put(
        &mut frame,
        TableRows::StateBaseResources(vec![
            base(MIXED, OIL, 200_000),
            base(MIXED, STEEL, 400_000),
            base(ORPHAN, STEEL, 300_000),
            base(GIVEN, STEEL, 100_000),
            base(ALIASED, STEEL, 100_000),
        ]),
    );
    put(
        &mut frame,
        TableRows::StateResources(vec![
            StateResourceRow {
                state: MIXED,
                resource: RUBBER,
                cost: 30_000,
                ..Default::default()
            },
            StateResourceRow {
                state: COSTED,
                resource: RUBBER,
                cost: 10_000,
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::StateBuildings(vec![
            building(MIXED, INFRASTRUCTURE, 4, 2),
            building(MIXED, REFINERY, 1, 1),
            building(GRANTED, REFINERY, 2, 2),
        ]),
    );
    put(
        &mut frame,
        TableRows::ResourceOrigins(vec![
            ResourceOriginRow {
                local_resources_factor: 10_000,
                convoys_assigned: 3,
                convoys_required: 5,
                ..origin(MIXED, GER, 0, 80_000)
            },
            origin(MIXED, AFG, 1, 100_000),
            origin(EMPTY, GER, 0, 100_000),
            origin(GATED, GER, 0, 100_000),
            origin(COSTED, AFG, 0, 100_000),
            origin(GRANTED, GER, 0, 100_000),
            origin(GIVEN, GER, 0, 100_000),
            origin(GIVEN, AFG, 1, 100_000),
            origin(ALIASED, GER, 0, 100_000),
        ]),
    );
    put(
        &mut frame,
        TableRows::ResourceRights(vec![
            ResourceRightRow {
                ordinal: 0,
                state: MIXED,
                resource: 1,
                holder: AFG,
            },
            ResourceRightRow {
                ordinal: 0,
                state: GIVEN,
                resource: 2,
                holder: AFG,
            },
            ResourceRightRow {
                ordinal: 0,
                state: ALIASED,
                resource: 2,
                holder: GER_ALIAS,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::CountryOilFactors(vec![CountryOilFactorRow {
            country: GER,
            factor: 50_000,
        }]),
    );
    put(
        &mut frame,
        TableRows::StateTemporaryResources(vec![StateTemporaryResourceRow {
            state: MIXED,
            resource: OIL,
            amount: 90_000,
        }]),
    );
    put(
        &mut frame,
        TableRows::TradeRoutes(vec![TradeRouteRow {
            country: GER,
            class: super::ORIGIN_ROUTE_CLASS,
            other: MIXED,
            kind: 2,
            ..Default::default()
        }]),
    );
    frame
}
