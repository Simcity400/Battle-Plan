use super::captured;
use super::*;
use engine_native::msvc_string::Text;
use replay_data::rows::TableId;

const FIRST_HOUR: i32 = 100;

fn text<const N: usize>(value: &[u8]) -> Text<N> {
    let mut result = Text {
        len: value.len(),
        ..Text::default()
    };
    result.bytes[..value.len()].copy_from_slice(value);
    result
}

fn hour_frame(step: i32) -> Captured {
    let hour = FIRST_HOUR + step;
    let mut source = captured(hour);
    source.world_generation = 1;
    source.total_ns = 1_000 + step as u64;
    source.table_ns[TableId::Countries.index()] = 10 + step as u64;
    source.pool_bytes = 4096;
    source.countries[0] = engine_native::ccountry::Row {
        politics_witness: None,
        id: 7,
        canonical_slot: 1,
        tag: text(b"GER"),
        political_power_raw: Some(i64::from(step / 5)),
        command_power_raw: 0,
        stability_raw: 500_000,
        war_support_raw: 250_000,
        convoys_raw: Some(12),
        peace_conference_participant: 0,
        manpower_residual: 3,
        border_land_doctrine_level: None,
    };
    source.countries_result = if step == 7 {
        Outcome::Unavailable {
            reason: "synthetic country fault",
        }
    } else {
        Outcome::Rows { len: 1 }
    };
    source.links[0] = engine_native::ccountry::LinkRow {
        id: 2,
        canonical_slot: 1,
    };
    source.links_result = if step == 11 {
        Outcome::Truncated {
            len: 1,
            unread_rows: 4,
        }
    } else {
        Outcome::Rows { len: 1 }
    };
    source.provinces.rows[0] = engine_native::cprovince::Row {
        id: 6,
        state: 4,
        owner: 7,
        controller: if step >= 20 { 2 } else { 7 },
        victory_points: 3,
        ..Default::default()
    };
    source.provinces_result = Outcome::Rows { len: 1 };
    source.states[0] = engine_native::cstate::Row {
        id: 4,
        owner: 7,
        controller: 7,
        manpower_available: step / 3,
        ..Default::default()
    };
    source.states_result = Outcome::Rows { len: 1 };
    source.divisions[0] = engine_native::carmy::Row {
        id: 4713,
        roster_owner: 7,
        logical_owner: 7,
        expeditionary_owner: 0,
        province: 6,
        template: 9,
        hit_points: 10_000 * (1_000 - i64::from(step)),
        organisation: 5_000_000,
        orders_group: 0,
        reveal_skip: 0,
        theatre: Some(0),
    };
    source.divisions[1] = engine_native::carmy::Row {
        id: 51,
        hit_points: 9_000_000,
        ..source.divisions[0]
    };
    source.divisions_result = Outcome::Rows {
        len: if (12..16).contains(&step) { 1 } else { 2 },
    };
    source.wars[0] = engine_native::cwar_relation::Row {
        first_country: 2,
        second_country: 7,
        start_hour: 50,
        end_hour: -1,
        cancel: 0,
        first_casualties: i64::from(step / 4),
        second_casualties: 0,
        first_unknown_casualties: 0,
        second_unknown_casualties: 0,
        first_was_instigator: 1,
    };
    source.wars_result = Outcome::Rows { len: 1 };
    source.factions[0] = engine_native::cfaction::Row {
        id: 88,
        name_start: 0,
        name_len: 3,
        ..Default::default()
    };
    source.faction_names[..3].copy_from_slice(b"Axs");
    source.faction_names_len = 3;
    source.factions_result = Outcome::Rows { len: 1 };
    source.faction_members[0] = engine_native::cfaction_system::MemberRow {
        country: 7,
        faction: 88,
        ordinal: 0,
    };
    source.faction_members_result = Outcome::Rows { len: 1 };
    source
        .extended
        .set_intel_shadow_fixture(if step < 17 { 10 } else { 20 });
    source
}

#[test]
fn the_conversion_helpers_reproduce_single_thread_conversion_whatever_the_plan() {
    use collector_capture::extended::HELPER_PACKS;
    use collector_capture::extended::PACK_NAMES;
    use collector_capture::extended::capture_live_shape;
    use collector_capture::pack_conversion::{MAX_THREADS, Plan};
    use collector_projection::projection::{Projection, ProjectionScratch};
    let pool = [
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
    ];
    let helper_steps = Steps::counted();
    let packs = PACK_NAMES.len();
    let later_first: [Option<u64>; PACK_NAMES.len()] =
        std::array::from_fn(|pack| Some(pack as u64 + 1));
    let earlier_first: [Option<u64>; PACK_NAMES.len()] =
        std::array::from_fn(|pack| Some((packs - pack) as u64));
    for ordinary_failure in [None, Some("synthetic ordinary acquisition failure")] {
        let mut source = hour_frame(3);
        // SAFETY: The isolated fixture owns observer preparation.
        unsafe { capture_live_shape(&mut source.extended) };
        source.extended.set_ordinary_failure(ordinary_failure);
        let clock = source.clock.as_ref().unwrap().copy_owned().unwrap();
        let encoder = FrameEncoder::new();
        let convert = |helper| {
            let scratch = ProjectionScratch::default();
            let mut projection = Projection::new(&encoder, 1, clock.hour, false, &scratch);
            decoded_with_helper_steps(
                &source,
                clock.borrowed(),
                1,
                &mut SpoolConversion {
                    variables_baseline: None,
                    variables_scratch: &mut ScriptVariableComparison::default(),
                    characters: None,
                    lexical: None,
                    intel: None,
                    projection: Some(&mut projection),
                    helper,
                },
                &helper_steps,
            )
        };
        let (serial, _) = convert(None);
        for estimates in [&later_first, &earlier_first] {
            for helpers in 1..=pool.len() {
                let plan = Plan::new(estimates, helpers);
                assert_eq!(plan.helpers(), helpers);
                let phase = collector_workers::spool_phases::Phase::ConvertPacks;
                let before = helper_steps.entries(phase);
                let (split, costs) = convert(Some(HelperSplit {
                    helpers: collector_workers::spool_helper::Helpers {
                        pool: &pool,
                        backlogged: helpers % 2 == 0,
                    },
                    plan,
                }));
                assert_eq!(helper_steps.entries(phase) - before, helpers);
                assert_eq!(
                    split.tables, serial.tables,
                    "{ordinary_failure:?}, {helpers}"
                );
                assert_eq!(split.metrics, serial.metrics);
                for ((name, movable), converted) in PACK_NAMES
                    .iter()
                    .zip(HELPER_PACKS)
                    .zip(costs.converted_by())
                {
                    let (_, thread) = converted.unwrap_or_else(|| panic!("{name} not converted"));
                    if movable {
                        assert!(thread <= helpers, "{name} on thread {thread}");
                    } else {
                        assert_eq!(thread, collector_capture::pack_conversion::SPOOL, "{name}");
                    }
                }
                let walls: Vec<bool> = costs.walls().map(|wall| wall.is_some()).collect();
                let expected: Vec<bool> =
                    (0..MAX_THREADS).map(|thread| thread <= helpers).collect();
                assert_eq!(walls, expected);
            }
        }
        if let Some(reason) = ordinary_failure {
            assert!(serial.tables.iter().any(|table| matches!(
                table,
                TableInput::Sampled(TableView::Unavailable { reason: actual }) if actual == reason
            )));
        }
        assert!(matches!(
            serial.tables[TableId::Ships.index()],
            TableInput::Sampled(TableView::Complete(_))
        ));
    }
    assert!(helper_steps.entries(collector_workers::spool_phases::Phase::ConvertPacks) >= 24);
}
