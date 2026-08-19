
use super::state_fixture::{
    compact_control_scheduling, drive_compact_frame, fixture_game_hour_raw, CollectorStateFixture,
};
use super::*;

mod at {
    pub(super) const GAME_STATE: usize = 0x0_0000;
    pub(super) const COUNTRY_TABLE: usize = 0x0_0800;
    pub(super) const LINK_TABLE: usize = 0x0_0900;
    pub(super) const COUNTRY_1: usize = 0x0_1000;
    pub(super) const COUNTRY_2: usize = 0x0_2400;
    pub(super) const RESOURCES_1: usize = 0x0_3800;
    pub(super) const RESOURCES_2: usize = 0x0_4000;
    pub(super) const POLITICS_1: usize = 0x0_4800;
    pub(super) const POLITICS_2: usize = 0x0_4C00;
    pub(super) const RELATION_DATA_1: usize = 0x0_5000;
    pub(super) const RELATION_DATA_2: usize = 0x0_5040;
    pub(super) const RELATION_ENTRY_1: usize = 0x0_5100;
    pub(super) const RELATION_ENTRY_2: usize = 0x0_5300;
    pub(super) const BUCKET_ROOT_1: usize = 0x0_5500;
    pub(super) const BUCKET_ROOT_2: usize = 0x0_5540;
    pub(super) const BUCKET_BODY_1: usize = 0x0_5580;
    pub(super) const BUCKET_BODY_2: usize = 0x0_55A0;
    pub(super) const IMPORT_BODY_1: usize = 0x0_55C0;
    pub(super) const IMPORT_BODY_2: usize = 0x0_55E0;
    pub(super) const ROUTE_1: usize = 0x0_5600;
    pub(super) const ROUTE_2: usize = 0x0_5A00;
    pub(super) const BASE_EXPORT_1: usize = 0x0_5E00;
    pub(super) const BASE_EXPORT_2: usize = 0x0_5E40;
    pub(super) const RESOURCE_DB: usize = 0x0_5F00;
    pub(super) const RESOURCE_DB_ITEMS: usize = 0x0_6000;
    pub(super) const RESOURCE_DB_KEYS: usize = 0x0_6040;
    pub(super) const RESOURCE_NULL_OBJECT: usize = 0x0_6080;
    pub(super) const RESOURCE_OIL: usize = 0x0_6200;
    pub(super) const LEXER_TOKEN_TABLE: usize = 0x0_6400;
    pub(super) const HEAP_LEN: usize = 0x0_8000;

    pub(super) const SEL_TAG_DB_ID: u32 = 5;
    pub(super) const BUY_TAG_DB_ID: u32 = 7;
    pub(super) const SEL_SLOT: u32 = 1;
    pub(super) const BUY_SLOT: u32 = 2;
    pub(super) const COUNTRY_EXTENT: i32 = 3;
    pub(super) const LINK_COUNT: i32 = 8;
    pub(super) const OIL_ENGINE_INDEX: i32 = 1;
    pub(super) const OIL_WIRE_INDEX: u32 = 0;
    pub(super) const OIL_TOKEN_ID: u32 = 1;
    pub(super) const OIL_FACTORY_COST_RAW: i64 = 100_000;

    pub(super) const REQUESTED_UNITS: i32 = 8;
    pub(super) const DELIVERED_RAW: i64 = 720_000;
    pub(super) const BASE_EXPORT_RAW: i64 = 1_600_000;
    pub(super) const EFFICIENCY_A_RAW: i64 = 100_000;
    pub(super) const EFFICIENCY_B_RAW: i64 = 90_000;
    pub(super) const CIVILIAN_FACTORIES: i32 = 3;
    pub(super) const CONVOYS_ASSIGNED: i32 = 4;
    pub(super) const CONVOYS_REQUIRED: i32 = 6;
}

fn classic_deal_world() -> CollectorStateFixture {
    use compact_contract::build_profile as p;

    let mut fixture = CollectorStateFixture::new(
        &[
            C_GAME_STATE_DAILY_UPDATE_OFFSET,
            GET_LONG_STRING_OFFSET,
            p::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
            CSTRATEGIC_RESOURCE_DATABASE_PTR_RVA,
            PDX_LEXER_INITIALIZED_RVA,
            PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA,
        ],
        0x1000,
        at::HEAP_LEN,
    );
    fixture.stage_profile_sentinels();
    fixture.stage_scheduler_clock_at(at::GAME_STATE);

    let base = fixture.module.module_base();
    let country_vtable = base + p::DOMAIN57_COUNTRY_VTABLE_RVA;
    let country_1 = fixture.heap.at(at::COUNTRY_1);
    let country_2 = fixture.heap.at(at::COUNTRY_2);
    let resources_1 = fixture.heap.at(at::RESOURCES_1);
    let resources_2 = fixture.heap.at(at::RESOURCES_2);
    let route_1 = fixture.heap.at(at::ROUTE_1);
    let route_2 = fixture.heap.at(at::ROUTE_2);
    let oil = fixture.heap.at(at::RESOURCE_OIL);
    let null_object = fixture.heap.at(at::RESOURCE_NULL_OBJECT);

    fixture.heap.write_u64(
        at::GAME_STATE + CGAMESTATE_COUNTRY_TABLE,
        fixture.heap.at(at::COUNTRY_TABLE) as u64,
    );
    fixture.heap.write_i32(
        at::GAME_STATE + CGAMESTATE_COUNTRY_COUNT,
        at::COUNTRY_EXTENT,
    );
    fixture.heap.write_u64(
        at::GAME_STATE + p::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_PTR,
        fixture.heap.at(at::LINK_TABLE) as u64,
    );
    fixture.heap.write_i32(
        at::GAME_STATE + p::DOMAIN57_CGAMESTATE_TAG_EQUIVALENCE_COUNT,
        at::LINK_COUNT,
    );
    fixture.heap.write_u64(
        at::COUNTRY_TABLE + 8 * at::SEL_SLOT as usize,
        country_1 as u64,
    );
    fixture.heap.write_u64(
        at::COUNTRY_TABLE + 8 * at::BUY_SLOT as usize,
        country_2 as u64,
    );
    fixture.heap.write_i32(
        at::LINK_TABLE + 4 * at::SEL_TAG_DB_ID as usize,
        at::SEL_SLOT as i32,
    );
    fixture.heap.write_i32(
        at::LINK_TABLE + 4 * at::BUY_TAG_DB_ID as usize,
        at::BUY_SLOT as i32,
    );

    for (
        country_at,
        resources_at,
        politics_at,
        relation_data_at,
        relation_entry_at,
        counterpart_slot,
    ) in [
        (
            at::COUNTRY_1,
            at::RESOURCES_1,
            at::POLITICS_1,
            at::RELATION_DATA_1,
            at::RELATION_ENTRY_1,
            at::BUY_SLOT,
        ),
        (
            at::COUNTRY_2,
            at::RESOURCES_2,
            at::POLITICS_2,
            at::RELATION_DATA_2,
            at::RELATION_ENTRY_2,
            at::SEL_SLOT,
        ),
    ] {
        fixture.heap.write_u64(country_at, country_vtable as u64);
        let resources = fixture.heap.at(resources_at);
        let politics = fixture.heap.at(politics_at);
        let relation_data = fixture.heap.at(relation_data_at);
        let relation_entry = fixture.heap.at(relation_entry_at);
        fixture
            .heap
            .write_u64(country_at + CCOUNTRY_RESOURCES_OFF, resources as u64);
        fixture
            .heap
            .write_u64(country_at + CCOUNTRY_POLITICS_STATUS_OFF, politics as u64);
        fixture
            .heap
            .write_u64(politics_at + 8, relation_data as u64);
        fixture.heap.write_u64(
            relation_data_at + 8 * counterpart_slot as usize,
            relation_entry as u64,
        );
    }

    for (
        resources_at,
        raw_tag,
        base_export_at,
        bucket_root_at,
        bucket_body_at,
        import_body_at,
        exported_route,
        imported_route,
    ) in [
        (
            at::RESOURCES_1,
            at::SEL_TAG_DB_ID,
            at::BASE_EXPORT_1,
            at::BUCKET_ROOT_1,
            at::BUCKET_BODY_1,
            at::IMPORT_BODY_1,
            route_1,
            route_2,
        ),
        (
            at::RESOURCES_2,
            at::BUY_TAG_DB_ID,
            at::BASE_EXPORT_2,
            at::BUCKET_ROOT_2,
            at::BUCKET_BODY_2,
            at::IMPORT_BODY_2,
            route_2,
            route_1,
        ),
    ] {
        fixture.heap.write_i32(resources_at + 0x20, raw_tag as i32);
        fixture.heap.write_u64(
            resources_at + CCOUNTRY_RESOURCES_BASE_EXPORT_VALUES_OFF,
            fixture.heap.at(base_export_at) as u64,
        );
        fixture.heap.write_i64(
            base_export_at + 0x10 * at::OIL_ENGINE_INDEX as usize,
            at::BASE_EXPORT_RAW,
        );
        fixture.heap.write_u64(
            resources_at + CCOUNTRY_RESOURCES_EXPORT_ROUTE_BUCKETS_OFF,
            fixture.heap.at(bucket_root_at) as u64,
        );
        let bucket = bucket_root_at + 0x18 * at::OIL_ENGINE_INDEX as usize;
        fixture
            .heap
            .write_u64(bucket, fixture.heap.at(bucket_body_at) as u64);
        fixture.heap.write_i32(bucket + 0xC, 1);
        fixture
            .heap
            .write_u64(bucket_body_at, exported_route as u64);
        fixture.heap.write_u64(
            resources_at + CCOUNTRY_RESOURCES_IMPORT_ROUTES_OFF,
            fixture.heap.at(import_body_at) as u64,
        );
        fixture
            .heap
            .write_i32(resources_at + CCOUNTRY_RESOURCES_IMPORT_ROUTE_COUNT_OFF, 1);
        fixture
            .heap
            .write_u64(import_body_at, imported_route as u64);
    }

    for (route_at, exporter_resources, importer_raw_tag) in [
        (at::ROUTE_1, resources_1, at::BUY_TAG_DB_ID),
        (at::ROUTE_2, resources_2, at::SEL_TAG_DB_ID),
    ] {
        fixture.heap.write_u64(
            route_at + CRESOURCE_DELIVERY_ROUTE_EXPORTER_OFF,
            exporter_resources as u64,
        );
        fixture.heap.write_i32(
            route_at + CRESOURCE_DELIVERY_ROUTE_IMPORTER_TAG_OFF,
            importer_raw_tag as i32,
        );
        fixture
            .heap
            .write_u64(route_at + CRESOURCE_DELIVERY_ROUTE_RESOURCE_OFF, oil as u64);
        fixture.heap.write_i64(
            route_at + CRESOURCE_DELIVERY_ROUTE_DELIVERED_RAW_OFF,
            at::DELIVERED_RAW,
        );
        fixture.heap.write_i32(
            route_at + CRESOURCE_DELIVERY_ROUTE_REQUESTED_INPUT_OFF,
            at::REQUESTED_UNITS,
        );
        fixture.heap.write_i32(
            route_at + CRESOURCE_DELIVERY_ROUTE_CIVILIAN_FACTORIES_OFF,
            at::CIVILIAN_FACTORIES,
        );
        fixture.heap.write_i32(
            route_at + CRESOURCE_DELIVERY_ROUTE_CONVOYS_ASSIGNED_OFF,
            at::CONVOYS_ASSIGNED,
        );
        fixture.heap.write_i32(
            route_at + CRESOURCE_DELIVERY_ROUTE_CONVOYS_REQUIRED_OFF,
            at::CONVOYS_REQUIRED,
        );
        fixture.heap.write_i64(
            route_at + CRESOURCE_DELIVERY_ROUTE_EFFICIENCY_A_OFF,
            at::EFFICIENCY_A_RAW,
        );
        fixture.heap.write_i64(
            route_at + CRESOURCE_DELIVERY_ROUTE_EFFICIENCY_B_OFF,
            at::EFFICIENCY_B_RAW,
        );
    }

    fixture
        .heap
        .write_u8(at::RESOURCE_NULL_OBJECT + CSTRATEGIC_RESOURCE_ACTIVE_OFF, 1);
    fixture
        .heap
        .write_i32(at::RESOURCE_NULL_OBJECT + CSTRATEGIC_RESOURCE_INDEX_OFF, 0);
    fixture
        .heap
        .write_u8(at::RESOURCE_OIL + CSTRATEGIC_RESOURCE_ACTIVE_OFF, 1);
    fixture.heap.write_i32(
        at::RESOURCE_OIL + CSTRATEGIC_RESOURCE_INDEX_OFF,
        at::OIL_ENGINE_INDEX,
    );
    fixture.heap.write_u32(
        at::RESOURCE_OIL + CSTRATEGIC_RESOURCE_TOKEN_ID_OFF,
        at::OIL_TOKEN_ID,
    );
    fixture.heap.write_i64(
        at::RESOURCE_OIL + CSTRATEGIC_RESOURCE_FACTORY_COST_RAW_OFF,
        at::OIL_FACTORY_COST_RAW,
    );
    fixture
        .heap
        .write_u64(at::RESOURCE_DB_ITEMS, null_object as u64);
    fixture
        .heap
        .write_u64(at::RESOURCE_DB_ITEMS + 8, oil as u64);
    fixture
        .heap
        .write_u32(at::RESOURCE_DB_KEYS, at::OIL_TOKEN_ID);
    fixture.heap.write_u64(
        at::RESOURCE_DB + CSTRATEGIC_RESOURCE_DATABASE_ITEMS_OFF,
        fixture.heap.at(at::RESOURCE_DB_ITEMS) as u64,
    );
    fixture
        .heap
        .write_i32(at::RESOURCE_DB + CSTRATEGIC_RESOURCE_DATABASE_COUNT_OFF, 2);
    fixture.heap.write_u64(
        at::RESOURCE_DB + CSTRATEGIC_RESOURCE_DATABASE_KEYS_OFF,
        fixture.heap.at(at::RESOURCE_DB_KEYS) as u64,
    );
    fixture.heap.write_i32(
        at::RESOURCE_DB + CSTRATEGIC_RESOURCE_DATABASE_KEY_COUNT_OFF,
        1,
    );
    fixture.heap.write_u64(at::RESOURCE_DB + 0x60, oil as u64);
    fixture
        .heap
        .write_u64(at::RESOURCE_DB + 0x68, null_object as u64);
    fixture
        .heap
        .write_u64(at::RESOURCE_DB + 0x70, null_object as u64);
    fixture.module.write_u64_at_rva(
        CSTRATEGIC_RESOURCE_DATABASE_PTR_RVA,
        fixture.heap.at(at::RESOURCE_DB) as u64,
    );

    let oil_entry = at::LEXER_TOKEN_TABLE + PDX_LEXER_TOKEN_NAME_STRIDE * at::OIL_TOKEN_ID as usize;
    fixture.heap.write_bytes(oil_entry, b"oil");
    fixture.heap.write_u64(oil_entry + 0x10, 3);
    fixture.heap.write_u64(oil_entry + 0x18, 15);
    fixture
        .module
        .write_bytes_at_rva(PDX_LEXER_INITIALIZED_RVA, &[1u8]);
    fixture.module.write_u64_at_rva(
        PDX_LEXER_TOKEN_NAME_TABLE_PTR_RVA,
        fixture.heap.at(at::LEXER_TOKEN_TABLE) as u64,
    );
    fixture
        .module
        .write_bytes_at_rva(PDX_LEXER_MAX_TOKEN_ID_RVA, &1u32.to_le_bytes());
    fixture
        .module
        .write_bytes_at_rva(PDX_LEXER_TOKEN_NAME_TABLE_CAPACITY_RVA, &2u32.to_le_bytes());

    fixture.set_verified_country_roster(
        "gamestate_country_table_verified",
        country_vtable,
        vec![
            test_country_row("SEL", at::SEL_TAG_DB_ID, country_1),
            test_country_row("BUY", at::BUY_TAG_DB_ID, country_2),
        ],
    );
    fixture
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MtrdRow {
    owner_tag_db_id: u32,
    counterpart_tag_db_id: u32,
    resource_index: u32,
    visible_order: u32,
    direction: u32,
    route_kind: u32,
    flags: u32,
    route_status_flags: u32,
    deal_exporter_tag_db_id: u32,
    deal_importer_tag_db_id: u32,
    deal_ordinal: u32,
    requested_raw: i64,
    delivered_raw: i64,
    available_export_raw: i64,
    factory_cost_raw: i64,
    efficiency_raw: i64,
    civilian_factories: i32,
    convoys_assigned: i32,
    convoys_required: i32,
}

fn decode_mtrd(payload: &[u8]) -> Option<(Vec<u32>, Vec<u32>, Vec<MtrdRow>)> {
    let offset = payload
        .windows(4)
        .position(|window| window == COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC)?;
    let u16_at = |at: usize| u16::from_le_bytes(payload[at..at + 2].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(payload[at..at + 8].try_into().unwrap());
    let i64_at = |at: usize| i64::from_le_bytes(payload[at..at + 8].try_into().unwrap());
    let i32_at = |at: usize| i32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
    assert_eq!(
        u16_at(offset + 4),
        COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_EMITTED_VERSION
    );
    let header_len = u16_at(offset + 6) as usize;
    let row_len = u16_at(offset + 8) as usize;
    let row_count = u32_at(offset + 12) as usize;
    let witness_count = u32_at(offset + 24) as usize;
    assert_eq!(u32_at(offset + 16) as usize, witness_count);
    let incomplete_count = u32_at(offset + 28) as usize;
    let mut cursor = offset + header_len;
    let witnesses = (0..witness_count)
        .map(|index| u32_at(cursor + index * 4))
        .collect::<Vec<_>>();
    cursor += witness_count * 4;
    let incomplete = (0..incomplete_count)
        .map(|index| u32_at(cursor + index * 4))
        .collect::<Vec<_>>();
    cursor += incomplete_count * 4;
    let rows = (0..row_count)
        .map(|index| {
            let row = cursor + index * row_len;
            MtrdRow {
                owner_tag_db_id: u32_at(row),
                counterpart_tag_db_id: u32_at(row + 4),
                resource_index: u32_at(row + 8),
                visible_order: u32_at(row + 12),
                direction: u32_at(row + 16),
                route_kind: u32_at(row + 20),
                flags: u32_at(row + 24),
                route_status_flags: u32_at(row + 28),
                deal_exporter_tag_db_id: u32_at(row + 32),
                deal_importer_tag_db_id: u32_at(row + 36),
                deal_ordinal: u32_at(row + 92),
                requested_raw: i64_at(row + 40),
                delivered_raw: i64_at(row + 48),
                available_export_raw: i64_at(row + 56),
                factory_cost_raw: i64_at(row + 64),
                efficiency_raw: i64_at(row + 72),
                civilian_factories: i32_at(row + 80),
                convoys_assigned: i32_at(row + 84),
                convoys_required: i32_at(row + 88),
            }
        })
        .collect::<Vec<_>>();
    Some((witnesses, incomplete, rows))
}

fn deal_id(exporter: u32, importer: u32) -> (u32, u32, u32, u32) {
    (exporter, importer, at::OIL_WIRE_INDEX, 1)
}

fn mtrd_row_deal_id(row: &MtrdRow) -> (u32, u32, u32, u32) {
    (
        row.deal_exporter_tag_db_id,
        row.deal_importer_tag_db_id,
        row.resource_index,
        row.deal_ordinal,
    )
}

fn produced_row_deal_id(row: &CompactMarketTradeClassicDealRow) -> (u32, u32, u32, u32) {
    (
        row.deal_exporter_tag_db_id,
        row.deal_importer_tag_db_id,
        row.resource_index,
        row.deal_ordinal,
    )
}

fn drive(fixture: &CollectorStateFixture) -> super::state_fixture::FrameOutcome {
    drive_compact_frame(
        Some(&fixture.collector_state()),
        &compact_control_scheduling(&[RAW_MARKET_TRADE_LIVE_DOMAIN]),
        fixture_game_hour_raw(),
        11,
    )
}

/// MUTATION RUN (identity check, `--release`): replaced the mapped comparison
/// MUTATION RUN (renumbering, `--release`): made
#[test]
fn a_classic_deal_world_makes_domain43_emit_classic_deal_rows_offline() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let fixture = classic_deal_world();
    let outcome = drive(&fixture);
    let data = outcome
        .domain_data(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted a DOMAIN_DATA record");
    let (witnesses, incomplete, rows) =
        decode_mtrd(&data.payload).expect("the payload carries an MTRD trailer");

    assert_eq!(
        witnesses,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "the trailer speaks for both census countries, under D18's own keys"
    );
    assert!(
        incomplete.is_empty(),
        "and claims both lists exhaustive, which is what makes this the control \
         for the per-owner completeness test below: {incomplete:?}"
    );
    assert_eq!(
        rows.len(),
        4,
        "the classic-deal family published rows: two routes, two halves each"
    );

    let staged = |owner: u32,
                  counterpart: u32,
                  visible_order: u32,
                  direction: u32,
                  deal: (u32, u32, u32, u32)| {
        assert_eq!(
            deal.2,
            at::OIL_WIRE_INDEX,
            "every staged deal in this fixture is the one oil resource"
        );
        MtrdRow {
            owner_tag_db_id: owner,
            counterpart_tag_db_id: counterpart,
            resource_index: at::OIL_WIRE_INDEX,
            visible_order,
            direction,
            route_kind: COMPACT_MARKET_TRADE_CLASSIC_DEAL_ROUTE_SEA,
            flags: COMPACT_MARKET_TRADE_CLASSIC_DEAL_FLAG_EXACT,
            route_status_flags: 0,
            deal_exporter_tag_db_id: deal.0,
            deal_importer_tag_db_id: deal.1,
            deal_ordinal: deal.3,
            requested_raw: i64::from(at::REQUESTED_UNITS) * 100_000,
            delivered_raw: at::DELIVERED_RAW,
            available_export_raw: at::BASE_EXPORT_RAW,
            factory_cost_raw: i64::from(at::REQUESTED_UNITS) * 10_000_000_000
                / at::OIL_FACTORY_COST_RAW,
            efficiency_raw: at::EFFICIENCY_A_RAW * at::EFFICIENCY_B_RAW / 100_000,
            civilian_factories: at::CIVILIAN_FACTORIES,
            convoys_assigned: at::CONVOYS_ASSIGNED,
            convoys_required: at::CONVOYS_REQUIRED,
        }
    };
    let sel_to_buy = deal_id(at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID);
    let buy_to_sel = deal_id(at::BUY_TAG_DB_ID, at::SEL_TAG_DB_ID);
    let mut sorted = rows.clone();
    sorted.sort_by_key(|row| (row.owner_tag_db_id, row.visible_order));
    assert_eq!(
        sorted,
        vec![
            staged(
                at::SEL_TAG_DB_ID,
                at::BUY_TAG_DB_ID,
                0,
                COMPACT_MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT,
                sel_to_buy
            ),
            staged(
                at::SEL_TAG_DB_ID,
                at::BUY_TAG_DB_ID,
                1,
                COMPACT_MARKET_TRADE_CLASSIC_DEAL_DIRECTION_IMPORT,
                buy_to_sel
            ),
            staged(
                at::BUY_TAG_DB_ID,
                at::SEL_TAG_DB_ID,
                0,
                COMPACT_MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT,
                buy_to_sel
            ),
            staged(
                at::BUY_TAG_DB_ID,
                at::SEL_TAG_DB_ID,
                1,
                COMPACT_MARKET_TRADE_CLASSIC_DEAL_DIRECTION_IMPORT,
                sel_to_buy
            ),
        ],
        "the emitted MTRD rows must be exactly the staged deals"
    );

    for name in [
        "d43_classic_skipped",
        "d43_classic_excluded_routes",
        "d43_classic_excluded_countries",
        "d43_classic_unpaired_routes",
        "d43_classic_incomplete_countries",
        "d43_classic_failure_stage",
    ] {
        assert_eq!(
            outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, name),
            Some(0),
            "{name} must be zero on a world the walk read completely"
        );
    }
    outcome.assert_diagnostics_agree_with_production(COMPACT_DOMAIN_MARKET_TRADE_LIVE);
}

/// MUTATION RUN (`--release`): made `compact_market_trade_classic_deal_renumber`
#[test]
fn d43_classic_deal_payload_satisfies_the_folds_mtrd_invariants() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let fixture = classic_deal_world();
    let outcome = drive(&fixture);
    let data = outcome
        .domain_data(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted a DOMAIN_DATA record");
    let (witnesses, incomplete, rows) =
        decode_mtrd(&data.payload).expect("the payload carries an MTRD trailer");
    let witness_set = witnesses.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(witness_set.len(), witnesses.len(), "witnesses are distinct");
    assert!(
        incomplete.windows(2).all(|pair| pair[0] < pair[1]),
        "the incomplete-owner block is strictly ascending: {incomplete:?}"
    );
    assert!(
        incomplete.iter().all(|owner| witness_set.contains(owner)),
        "and every owner it names is a witness: {incomplete:?}"
    );

    let mut by_owner: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut halves: BTreeMap<(u32, u32, u32, u32), Vec<MtrdRow>> = BTreeMap::new();
    for row in &rows {
        assert!(
            witness_set.contains(&row.owner_tag_db_id)
                && witness_set.contains(&row.counterpart_tag_db_id),
            "both parties of a row must be witnesses"
        );
        assert_ne!(
            row.owner_tag_db_id, row.counterpart_tag_db_id,
            "a country does not trade with itself"
        );
        assert_eq!(
            row.requested_raw % 100_000,
            0,
            "requested is a whole number of units"
        );
        assert!(
            (0..=100_000).contains(&row.efficiency_raw),
            "efficiency is a fraction of the fixed scale"
        );
        let exporter = row.deal_exporter_tag_db_id;
        let importer = row.deal_importer_tag_db_id;
        assert_ne!(row.deal_ordinal, 0, "the deal ordinal is one-based");
        assert!(
            row.deal_ordinal
                <= compact_contract::build_profile::MAX_MARKET_TRADE_CLASSIC_DEAL_ORDINAL,
            "the deal ordinal is inside the export bucket's transport budget"
        );
        let (deal_owner, deal_counterpart) =
            if row.direction == COMPACT_MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT {
                (exporter, importer)
            } else {
                (importer, exporter)
            };
        assert_eq!(
            (deal_owner, deal_counterpart),
            (row.owner_tag_db_id, row.counterpart_tag_db_id),
            "the deal id names the row's own two parties"
        );
        by_owner
            .entry(row.owner_tag_db_id)
            .or_default()
            .push(row.visible_order);
        halves.entry(mtrd_row_deal_id(row)).or_default().push(*row);
    }
    for (owner, mut orders) in by_owner {
        let seen = orders.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(
            seen.len(),
            orders.len(),
            "owner {owner} must carry each visible_order once: {:?} seen twice",
            orders
                .iter()
                .find(|order| orders.iter().filter(|other| other == order).count() > 1)
                .copied()
                .unwrap_or_default()
        );
        orders.sort_unstable();
        assert!(
            orders
                .iter()
                .enumerate()
                .all(|(index, order)| *order == index as u32),
            "owner {owner}'s visible_order must be dense 0..n, got {orders:?}"
        );
    }
    for (deal, sides) in halves {
        assert_eq!(sides.len(), 2, "deal {deal:?} must have exactly two halves");
        assert_ne!(
            sides[0].direction, sides[1].direction,
            "deal {deal:?}'s halves must be opposite directions"
        );
        let stripped = |row: MtrdRow| MtrdRow {
            owner_tag_db_id: 0,
            counterpart_tag_db_id: 0,
            visible_order: 0,
            direction: 0,
            ..row
        };
        assert_eq!(
            stripped(sides[0]),
            stripped(sides[1]),
            "deal {deal:?}'s halves must agree on every value"
        );
    }
}

/// MUTATION RUN (`--release`): changed the per-route exclusion in pass 1 back to
/// MUTATION RUN (`--release`): changed the per-route exclusion in PASS 2
/// MUTATION RUN (`--release`): made
/// MUTATION RUN (`--release`): dropped the `route_affected_countries` arm of the
#[test]
fn one_unattributable_route_is_excluded_accounted_and_published_short() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = classic_deal_world();
    const STRAY_RAW_TAG: u32 = 4;
    fixture.heap.write_i32(
        at::LINK_TABLE + 4 * STRAY_RAW_TAG as usize,
        at::COUNTRY_EXTENT,
    );
    fixture.heap.write_i32(
        at::ROUTE_2 + CRESOURCE_DELIVERY_ROUTE_IMPORTER_TAG_OFF,
        STRAY_RAW_TAG as i32,
    );

    let outcome = drive(&fixture);
    let data = outcome
        .domain_data(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted a DOMAIN_DATA record");
    let (witnesses, incomplete, rows) = decode_mtrd(&data.payload).expect(
        "a partial classic-deal snapshot PUBLISHES: v3 states completeness per \
         owner, so the rows the walk proved no longer cost the hour",
    );
    assert_eq!(
        witnesses,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "the witness list is still the whole census -- MTRF/MTRH/MTRR are built \
         over the same census and `fold_market_trade` rejects the payload if the \
         four disagree"
    );
    assert_eq!(
        incomplete,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "route 2 is a LIVE deal that did not publish, so BOTH of its parties are \
         named short -- SEL lost an import, BUY lost an export"
    );
    let sel_to_buy_deal = deal_id(at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID);
    let buy_to_sel_deal = deal_id(at::BUY_TAG_DB_ID, at::SEL_TAG_DB_ID);
    assert_eq!(
        rows.len(),
        2,
        "and the route the walk DID read is on the wire, which is the whole \
         point of publishing short: {rows:?}"
    );
    assert!(
        rows.iter()
            .all(|row| mtrd_row_deal_id(row) == sel_to_buy_deal),
        "they are route 1's two halves"
    );
    assert!(
        rows.iter()
            .all(|row| mtrd_row_deal_id(row) != buy_to_sel_deal),
        "route 2 is absent -- and it is absent UNDER A NAMED INCOMPLETENESS, \
         which is the difference between this and publishing a deletion"
    );
    for owner in [at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID] {
        let mut orders = rows
            .iter()
            .filter(|row| row.owner_tag_db_id == owner)
            .map(|row| row.visible_order)
            .collect::<Vec<_>>();
        orders.sort_unstable();
        assert!(
            orders.iter().copied().eq(0..orders.len() as u32),
            "owner {owner}'s visible order is dense over what it published: {orders:?}"
        );
    }

    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_classic_excluded_routes"
        ),
        Some(1),
        "one route did not reach the wire, and it is counted once"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_classic_excluded_countries"
        ),
        Some(0),
        "no country's route set was unreadable"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_classic_unpaired_routes"
        ),
        Some(0),
        "the refused claim never entered the import order, so nothing is unpaired"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_classic_failure_stage"
        ),
        Some(u64::from(
            domain43_classic_stage::IMPORTER_ROUTE_DISAGREEMENT
        )),
        "and the FIRST exclusion names the check that refused it"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_classic_failure_tag_db_id"
        ),
        Some(u64::from(at::SEL_TAG_DB_ID)),
        "under the census key of the country whose claim was refused"
    );
    assert_eq!(
        outcome.diagnostic_unit(
            COMPACT_DOMAIN_MARKET_TRADE_LIVE,
            "d43_classic_incomplete_countries"
        ),
        Some(2),
        "and the diagnostic agrees with the wire about who paid for it"
    );
    assert_eq!(
        outcome.diagnostic_unit(COMPACT_DOMAIN_MARKET_TRADE_LIVE, "d43_classic_skipped"),
        Some(0),
        "the family was NOT skipped: it published, short and said so"
    );
    let perf = outcome
        .perf(COMPACT_DOMAIN_MARKET_TRADE_LIVE)
        .expect("d43 emitted its perf record");
    assert_ne!(
        perf.status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "an excluded route is still a partial frame and still says so"
    );

    let state = fixture.collector_state();
    let descriptors = compact_market_trade_resource_descriptors(&state)
        .expect("the staged resource database discovers its one resource");
    let census = BTreeMap::from([
        (at::SEL_TAG_DB_ID, fixture.heap.at(at::COUNTRY_1)),
        (at::BUY_TAG_DB_ID, fixture.heap.at(at::COUNTRY_2)),
    ]);
    let (walked_rows, walked_witnesses, walked_incomplete) =
        compact_market_trade_classic_deal_rows(&state, &census, &descriptors)
            .expect("the walk completes over both countries rather than aborting at the first route it cannot attribute");
    assert_eq!(
        walked_incomplete,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "and it names both owners short rather than reporting one dirty bit for \
         the whole family"
    );
    assert_eq!(
        walked_witnesses,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "over the whole census"
    );
    assert_eq!(walked_rows.len(), 2, "with route 1's two halves proven");
    assert!(
        walked_rows
            .iter()
            .all(|row| produced_row_deal_id(row) == sel_to_buy_deal),
        "and they are the route that was readable"
    );
}

/// MUTATION RUN (`--release`): removed that arm, leaving `excluded_countries`
#[test]
fn an_unreadable_export_bucket_root_names_its_counterparts_short() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = classic_deal_world();
    fixture.heap.write_u64(
        at::RESOURCES_2 + CCOUNTRY_RESOURCES_EXPORT_ROUTE_BUCKETS_OFF,
        1,
    );

    let state = fixture.collector_state();
    let descriptors = compact_market_trade_resource_descriptors(&state)
        .expect("the staged resource database discovers its one resource");
    let census = BTreeMap::from([
        (at::SEL_TAG_DB_ID, fixture.heap.at(at::COUNTRY_1)),
        (at::BUY_TAG_DB_ID, fixture.heap.at(at::COUNTRY_2)),
    ]);
    let (rows, _witnesses, incomplete) =
        compact_market_trade_classic_deal_rows(&state, &census, &descriptors)
            .expect("the walk still completes over the country it could read");

    assert_eq!(
        incomplete,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "SEL imports a route BUY can no longer offer, and nothing charged that \
         route -- only SEL's own claim on it says the list is short"
    );
    assert_eq!(
        DOMAIN43_CLASSIC_EXCLUDED_COUNTRIES.load(Ordering::Relaxed),
        1,
        "BUY is the one country charged, and it is charged once"
    );
    assert_eq!(
        DOMAIN43_CLASSIC_EXCLUDED_ROUTES.load(Ordering::Relaxed),
        0,
        "no ROUTE was excluded: pass 2 never reached one"
    );
    assert_eq!(
        DOMAIN43_CLASSIC_UNPAIRED_ROUTES.load(Ordering::Relaxed),
        1,
        "route 2 is the residue -- claimed as an import, never reached from an \
         exporter's buckets"
    );
    let sel_to_buy = deal_id(at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID);
    assert_eq!(
        rows.len(),
        2,
        "and the route BUY's failure does not touch still publishes: {rows:?}"
    );
    assert!(
        rows.iter()
            .all(|row| produced_row_deal_id(row) == sel_to_buy),
        "namely SEL's export and BUY's import of route 1"
    );
}

/// MUTATION RUN (`--release`): removed the `contested_exports.insert(route)`,
/// MUTATION RUN (`--release`): dropped the `importer_order` arm of the
/// MUTATION RUN (`--release`): dropped the `route_affected_countries` arm.
#[test]
fn a_route_two_export_claims_both_authenticate_is_excluded_from_both() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    let mut fixture = classic_deal_world();
    let route_1 = fixture.heap.at(at::ROUTE_1) as u64;
    fixture.heap.write_u64(at::BUCKET_BODY_1 + 8, route_1);
    let bucket = at::BUCKET_ROOT_1 + 0x18 * at::OIL_ENGINE_INDEX as usize;
    fixture.heap.write_i32(bucket + 0xC, 2);

    let state = fixture.collector_state();
    let descriptors = compact_market_trade_resource_descriptors(&state)
        .expect("the staged resource database discovers its one resource");
    let census = BTreeMap::from([
        (at::SEL_TAG_DB_ID, fixture.heap.at(at::COUNTRY_1)),
        (at::BUY_TAG_DB_ID, fixture.heap.at(at::COUNTRY_2)),
    ]);
    let (rows, _witnesses, incomplete) =
        compact_market_trade_classic_deal_rows(&state, &census, &descriptors)
            .expect("the walk still completes");

    assert_eq!(
        incomplete,
        vec![at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID],
        "a withdrawn pair shortens BOTH parties' lists, so both are named"
    );
    let sel_to_buy = deal_id(at::SEL_TAG_DB_ID, at::BUY_TAG_DB_ID);
    assert!(
        rows.iter()
            .all(|row| produced_row_deal_id(row) != sel_to_buy),
        "a route two claims both authenticate is an ambiguity, and the walk must \
         not award it to whichever came first: {rows:?}"
    );
    let buy_to_sel = deal_id(at::BUY_TAG_DB_ID, at::SEL_TAG_DB_ID);
    assert_eq!(
        rows.iter()
            .filter(|row| produced_row_deal_id(row) == buy_to_sel)
            .count(),
        2,
        "and the uncontested route is untouched"
    );
    assert_eq!(
        DOMAIN43_CLASSIC_EXCLUDED_ROUTES.load(Ordering::Relaxed),
        1,
        "the contested route is charged, once"
    );
}

/// MUTATION RUN (`--release`): restored the arbitrate-then-authenticate order.
/// MUTATION RUN (`--release`): made the union charge every route in
#[test]
fn a_false_import_claim_does_not_evict_the_true_one_in_either_census_order() {
    let _bound_trip_registry = compact_contract::bound_trips::lock_registry_for_tests();
    for (label, import_body_at, false_route_at) in [
        ("false claimant second", at::IMPORT_BODY_2, at::ROUTE_2),
        ("false claimant first", at::IMPORT_BODY_1, at::ROUTE_1),
    ] {
        let mut fixture = classic_deal_world();
        let false_route = fixture.heap.at(false_route_at) as u64;
        fixture.heap.write_u64(import_body_at + 8, false_route);
        let resources_at = if import_body_at == at::IMPORT_BODY_1 {
            at::RESOURCES_1
        } else {
            at::RESOURCES_2
        };
        fixture
            .heap
            .write_i32(resources_at + CCOUNTRY_RESOURCES_IMPORT_ROUTE_COUNT_OFF, 2);

        let state = fixture.collector_state();
        let descriptors = compact_market_trade_resource_descriptors(&state)
            .expect("the staged resource database discovers its one resource");
        let census = BTreeMap::from([
            (at::SEL_TAG_DB_ID, fixture.heap.at(at::COUNTRY_1)),
            (at::BUY_TAG_DB_ID, fixture.heap.at(at::COUNTRY_2)),
        ]);
        let (rows, _witnesses, incomplete) =
            compact_market_trade_classic_deal_rows(&state, &census, &descriptors)
                .expect("the walk completes");

        assert_eq!(
            rows.len(),
            4,
            "a false import claim must not cost the route its real one ({label})"
        );
        assert!(
            incomplete.is_empty(),
            "and a claim that was refused on identity, on a route that still \
             published, leaves nothing excluded and nobody short ({label}): \
             {incomplete:?}"
        );
    }
}

#[test]
fn the_classic_deal_worlds_census_keys_are_link_indices_not_country_table_slots() {
    let fixture = classic_deal_world();
    let state = fixture.collector_state();
    let links = CompactMarketTradeCountryLinks::resolve(&state)
        .expect("the world stages CGameState+0x340/0x34C/0x31C");
    assert_eq!(
        links.canonical(at::SEL_TAG_DB_ID),
        Some(at::SEL_SLOT),
        "SEL's census key maps to its country-table slot"
    );
    assert_eq!(
        links.canonical(at::BUY_TAG_DB_ID),
        Some(at::BUY_SLOT),
        "BUY's census key maps to its country-table slot"
    );
    assert_ne!(
        at::SEL_TAG_DB_ID,
        at::SEL_SLOT,
        "and the key is not the slot, or neither test above distinguishes the two"
    );
    assert_ne!(at::BUY_TAG_DB_ID, at::BUY_SLOT);
}
