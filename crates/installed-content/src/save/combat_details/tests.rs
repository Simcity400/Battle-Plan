use super::*;
fn fields(text: &[u8]) -> Vec<Item> {
    parser::parse(text).unwrap().items
}
#[test]
fn naval_side_keeps_saved_damage_matrix_baseline_and_named_totals() {
    let matrix = (0..36)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let input = format!(
        "attacker={{total_initial_strength=10.5 total_damage_dealt=3.25 damage_dealt_by_gun_types={{{matrix}}} damage_dealt_by_ship_types={{destroyer=2.5 submarine=0.75}}}} defender={{}}"
    );
    let row = naval(&fields(input.as_bytes())).unwrap();
    assert_eq!(row.original_health[0].as_ref().unwrap().raw, 1_050_000);
    assert_eq!(row.total_damage[0].as_ref().unwrap().raw, 325_000);
    assert_eq!(row.damage_history[0].as_ref().unwrap().len(), 36);
    assert_eq!(row.damage_history[0].as_ref().unwrap()[35].raw, 3_500_000);
    assert_eq!(row.damage_by_type[0].as_ref().unwrap()[0].0, b"destroyer");
    assert_eq!(row.damage_by_type[0].as_ref().unwrap()[1].1.raw, 75_000);
    assert!(row.damage_by_type[1].is_none());
}
#[test]
fn land_preserves_order_full_handles_precision_and_absence() {
    let side=land_side(&fields(br#"front={id=7 type=8} front={id=9 type=10} reserves={id=7 type=11} air_damage_str=0.00001 prevented_damage_org=-0.25 org_loss_summary={1.125 -0.00001 0} org_loss_summary_index=2 num_org_losses=25 tactic=12 air_plane={amount=17}"#), false).unwrap();
    assert_eq!(side.committed, vec![(7_u64 << 32) | 8, (9_u64 << 32) | 10]);
    assert_eq!(side.reserves, vec![(7_u64 << 32) | 11]);
    assert_eq!(side.damage[0].as_ref().unwrap().raw, 1);
    assert_eq!(side.damage[5].as_ref().unwrap().raw, -25000);
    assert!(side.damage[1].is_none());
    assert!(side.anti_air.is_none());
    assert_eq!(
        side.organisation_loss
            .unwrap()
            .iter()
            .map(|x| x.raw)
            .collect::<Vec<_>>(),
        [112500, -1, 0]
    );
    assert_eq!(side.air_counts, [17]);
    assert!(land_side(&fields(b"tactic=1 tactic=2"), false).is_err());
    assert!(land_side(&fields(b"org_loss_summary={bad=1}"), false).is_err());
}
#[test]
fn naval_hits_keep_order_signed_slot_and_exact_damage() {
    let value=naval_member(&fields(br#"cached_info={tag="ENG" index=6 type=128 sprite="submarine"} hours_to_arrive=-1 naval_hit={target=-1 name="Convoy" convoy=yes damage=0.00001 strength=60 last_hit=no} damage_received_by_gun_types={0 1 2 3 4 5} damage_received={{ship={id=8 type=9} tag="FRA" damage=0.00002} {ship={id=0 type=0} tag="ITA" damage=0.05014}}"#)).unwrap();
    assert_eq!(value.arrival_hours, Some(-1));
    assert_eq!(value.hits[0].target, u32::MAX);
    assert_eq!(value.hits[0].damage.raw, 1);
    assert_eq!(value.damage_entries[0].ship, (8_u64 << 32) | 9);
    assert_eq!(value.damage_entries[0].damage.raw, 2);
    assert_eq!(value.damage_entries[1].ship, 0);
    assert_eq!(value.damage_entries[1].country, b"ITA");
    assert!(value.escape_progress.is_none());
}
#[test]
fn retired_live_slots_do_not_change_legacy_participant_census() {
    let items = fields(
        br#"id={id=7 type=60} location=1 progress=0.00001
          attacker={positioning=1 anti_air=0 group={
            member={unique_id=3 state=1 cached_info={tag=GER}}
            members_to_delete={unique_id=4 state=2 cached_info={tag=ENG}}
          }} defender={positioning=1 anti_air=0}"#,
    );
    let mut participants = BTreeMap::new();
    let value = parse_naval_combat(&items, &mut participants).unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants.values().next().unwrap().battle_slot, 3);
    assert_eq!(value.details.retired[0][0].battle_slot, 4);
    assert_eq!(value.details.progress.unwrap().raw, 1);
    assert_eq!(value.details.group_counts, [1, 0]);
}
#[test]
fn plane_and_result_air_records_preserve_distinct_names_and_conditional_kills() {
    let live = naval_plane(
        &fields(
            br#"carrier=-2 air_base={id=17 type=69}
            naval_strike={id=3 type=69} naval_strike={id=2 type=69}
            air_wing={id=4 type=69} max=10 alive=8 casualties=2 time_duration=3
            tag=GER last_external_wave_date=1939.1.1.1 external_wave_complete=yes external=no
            name="Live Wing" damage_mult=-0.00001"#,
        ),
        1,
    )
    .unwrap();
    assert_eq!(live.carrier, Some(-2));
    assert_eq!(live.wings[0], [(3_u64 << 32) | 69, (2_u64 << 32) | 69]);
    assert_eq!(live.damage_mult.raw, -1);
    let retained=result_side(&fields(br#"air_wing={max=5 alive=4 tag=GER equipment_variant_index={id=9 type=54} air_base="Retained Wing"}
            air_wing={max=8 alive=3 killed=5 tag=ENG equipment_variant_index={id=10 type=54} air_base="Second"}"#)).unwrap();
    assert_eq!(retained.planes[0].name, b"Retained Wing");
    assert_eq!(retained.planes[0].lost, None);
    assert_eq!(retained.planes[1].lost, Some(5));
}
/// Format-23 fresh save: carrier wings without an external wave keep CFEXAir's
#[test]
fn plane_without_external_wave_keeps_the_engine_invalid_date() {
    let items = |date: &str| {
        fields(
            format!(
                "carrier=1 naval_strike={{id=467 type=69}} max=36 alive=36 casualties=0 \
                     time_duration=0 tag=ENG last_external_wave_date=\"{date}\" \
                     external_wave_complete=no external=no name=\"Wing\" damage_mult=1.000"
            )
            .as_bytes(),
        )
    };
    let plane = naval_plane(&items("-1.1.1.1"), 0).unwrap();
    assert_eq!(plane.date, SavedDate::Invalid);
    assert_eq!(plane.date.engine_hour().unwrap(), 43_791_240);
    let dated = naval_plane(&items("1944.7.20.12"), 0).unwrap();
    assert_eq!(
        dated.date,
        SavedDate::Calendar(GameDate::parse(b"1944.7.20.12").unwrap())
    );
    assert!(naval_plane(&items("-2.1.1.1"), 0).is_err());
}

#[test]
fn border_war_save_keeps_raw_event_identifiers_country_and_state() {
    let row = land(&fields(b"change_state_after_war=no attacker={tag=OTHER state=11 on_win=border.001 on_lose=border.2} defender={tag=SECOND state=13}"), true).unwrap();
    assert_eq!(row.transfer_state, Some(false));
    assert_eq!(
        row.sides[0].border_country.as_deref(),
        Some(b"OTHER".as_slice())
    );
    assert_eq!(row.sides[0].border_state, Some(11));
    assert_eq!(
        row.sides[0].border_win_event.as_deref(),
        Some(b"border.001".as_slice())
    );
    assert!(row.sides[1].border_lose_event.is_none());
    let ordinary = land(&fields(b"attacker={} defender={}"), false).unwrap();
    assert!(ordinary.transfer_state.is_none());
}

#[test]
fn border_war_width_keeps_signed_fixed5_zero_and_absence() {
    for (text, expected) in [
        ("combat_width=80", Some(8_000_000)),
        ("combat_width=-0.00001", Some(-1)),
        ("combat_width=0", Some(0)),
        ("", None),
    ] {
        let items = fields(format!("{text} attacker={{}} defender={{}}").as_bytes());
        let border = land(&items, true).unwrap();
        assert_eq!(border.combat_width.map(|width| width.raw), expected);
        assert!(land(&items, false).unwrap().combat_width.is_none());
    }
    for text in [
        "combat_width=0.000001",
        "combat_width={}",
        "combat_width=1 combat_width=2",
    ] {
        assert!(
            land(
                &fields(format!("{text} attacker={{}} defender={{}}").as_bytes()),
                true
            )
            .is_err(),
            "{text}"
        );
    }
}
