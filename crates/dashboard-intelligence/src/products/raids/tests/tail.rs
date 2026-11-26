use super::{At, Raids, at, make_frame, raid, recording, target_row};
use crate::products::raids::tail::{RaidLogTail, RaidLogTailParams, RaidLogTailReply, RaidStatus, dated_panel};
use dashboard_base::{
    Source, SourceKey,
    products::{Country, LoadContext, Product},
    test_fixtures::{CAMPAIGN, records},
};
use engine_native_air::craid_instance as native;
use replay::campaign::Replay;
use replay_capture_spool::spool::Record;
use replay_data::rows::{RaidRow, RaidScope, TableRows};
use replay_frame::frame::TableView;
use std::sync::{Arc, atomic::AtomicBool};

fn opened(name: &str, segments: &[Vec<Record>]) -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), segments).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(SourceKey { source_id: name.into(), revision: "1".into() }, "memory.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn ask(hour: i32) -> RaidLogTailParams {
    RaidLogTailParams { branch_id: "1".into(), hour: Some(hour), from: None, to: None, countries: None, group: None, search: None, sort: None, offset: None, limit: 64, raid_types: None, statuses: None }
}

fn load(source: &Source, params: &RaidLogTailParams) -> RaidLogTailReply {
    RaidLogTail::load(source, params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
}

fn whole(rows: Vec<RaidRow>) -> Raids {
    Raids::Whole(rows)
}

fn stepped_equals_cold(source: &Source, asked: serde_json::Value, hours: &[i32]) {
    let panel = dated_panel(source, &LoadContext { content: None }, &asked).unwrap();
    let exact = panel.exact().unwrap();
    for &hour in hours {
        let stepped = (exact.advance)(hour, true, &AtomicBool::new(false), None);
        assert_eq!(*stepped.value, (exact.cold)(hour, &AtomicBool::new(false)), "stepped vs cold at {hour}");
    }
}

#[test]
fn a_type_first_spelled_later_is_no_chip_and_no_match_before_it_is_read() {
    let prep = raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE);
    let source = opened("tail-types", &recording(&[vec![
        At { spelled: false, ..at(10, whole(vec![prep.clone()]), 20) },
        At { spelled: false, ..at(11, whole(vec![prep.clone()]), 20) },
        at(12, whole(vec![prep.clone()]), 20),
        at(13, whole(vec![prep]), 20),
    ]]));
    let early = load(&source, &ask(11));
    assert!(early.raid_types.is_empty(), "no type is spelled by 11");
    assert_eq!((early.rows[0].raid_type.as_deref(), early.rows[0].type_missing.is_some()), (None, true));
    assert_eq!(early.next_change, Some(12), "the type's first read is the next change");
    let typed = RaidLogTailParams { raid_types: Some(vec!["air_raid".into()]), ..ask(11) };
    assert_eq!(load(&source, &typed).total, "0", "a type not yet read matches no type");
    let read = load(&source, &RaidLogTailParams { hour: Some(12), ..typed });
    assert_eq!((read.total.as_str(), read.raid_types.iter().map(|chip| (chip.spelling.as_str(), chip.first_hour)).collect::<Vec<_>>()), ("1", vec![("air_raid", 12)]));
    assert_eq!(read.rows[0].raid_type.as_deref(), Some("air_raid"));
    stepped_equals_cold(&source, serde_json::json!({ "panel": "raid-log-tail", "branchId": "1", "limit": 64, "raidTypes": ["air_raid"] }), &[10, 11, 12, 13, 11, 13]);
}

#[test]
fn a_rows_raider_is_named_at_the_rows_hour_and_a_later_rename_does_not_reach_it() {
    let source = opened("tail-rename", &recording(&[vec![
        at(10, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)]), 30),
        at(11, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)]), 30),
        At { slot1: b"SCO", ..at(12, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE), raid(8, 1, native::PHASE_ASSEMBLING, native::OUTCOME_NONE)]), 30) },
        At { slot1: b"SCO", ..at(13, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE), raid(8, 1, native::PHASE_ASSEMBLING, native::OUTCOME_NONE)]), 30) },
    ]]));
    let reply = load(&source, &ask(13));
    let raiders: Vec<(i32, Option<&str>)> = reply.rows.iter().map(|row| (row.hour, row.raider.tag.as_deref())).collect();
    assert_eq!(raiders, [(12, Some("SCO")), (10, Some("ENG"))], "each raider is the slot's tag at its own row's hour");
    assert_eq!((reply.counts[0].key.as_str(), reply.counts[0].total.as_str(), reply.live.as_str()), ("active", "2", "2"));
    assert_eq!(reply.rows[1].victim.tag.as_deref(), Some("FRA"));
    assert_eq!(load(&source, &ask(11)).rows.iter().map(|row| row.raider.tag.as_deref()).collect::<Vec<_>>(), [Some("ENG")], "the raid of 12 does not exist at 11");
}

#[test]
fn a_selected_slots_untagged_raid_is_listed_with_its_reason_and_a_victim_selects_only_from_its_read() {
    let scope = RaidScope { province: 10, ..Default::default() };
    let untagged = vec![(20, 2, b"GER".as_slice())];
    let frames = (10..=11)
        .map(|hour| make_frame(1, hour, untagged.clone(), 20, TableView::Complete(TableRows::Raids(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)])), vec![target_row(7, scope.clone(), 40)]))
        .collect();
    let source = opened("tail-untagged", &[records(1, frames)]);
    let chosen = load(&source, &RaidLogTailParams { countries: Some(vec![Country::Slot(1)]), ..ask(11) });
    assert_eq!(chosen.total, "1", "a raid by a chosen slot is listed though no tag names its raider");
    let raider = &chosen.rows[0].raider;
    assert_eq!((raider.tag.as_deref(), raider.slot, raider.missing.is_some()), (None, Some(1), true));

    let moved = opened("tail-victim", &recording(&[vec![
        at(10, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)]), 30),
        at(11, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)]), 30),
        at(12, whole(vec![raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE)]), 20),
    ]]));
    let germany = |hour| RaidLogTailParams { countries: Some(vec![Country::Tag("GER".into())]), ..ask(hour) };
    let before = load(&moved, &germany(11));
    assert_eq!((before.total.as_str(), before.next_change), ("0", Some(12)), "the target is first read as Germany's at 12");
    let after = load(&moved, &germany(12));
    assert_eq!((after.total.as_str(), after.rows[0].victim.tag.as_deref(), after.rows[0].victim.hour), ("1", Some("GER"), 12));
}

#[test]
fn an_end_dated_row_names_the_raider_of_the_observation_that_recorded_it() {
    let won = raid(5, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS);
    let mut dated = won.clone();
    dated.end_hour = Some(5);
    dated.completion_recorded = RaidRow::END_HOUR_RECORDED;
    let source = opened("tail-end", &recording(&[(4..=20)
        .map(|hour| At { slot1: if hour < 10 { b"GER" } else { b"DDR" }, ..at(hour, whole(match hour { 4..=9 => vec![], 10..=19 => vec![won.clone()], _ => vec![dated.clone()] }), 30) })
        .collect()]));
    let observed = load(&source, &ask(15));
    let row = &observed.rows[0];
    assert_eq!((row.status, row.live, row.raider.tag.as_deref()), (RaidStatus::Succeeded, false, Some("DDR")));
    let end = row.end.as_ref().unwrap();
    assert_eq!((end.hour, end.ended_hour, end.raider.as_deref()), (10, 10, Some("DDR")));
    assert_eq!(observed.next_change, Some(20), "the native end date is read at 20");
    let recorded = load(&source, &ask(20));
    let end = recorded.rows[0].end.as_ref().unwrap();
    assert_eq!((end.hour, end.ended_hour, end.raider.as_deref()), (20, 5, Some("GER")), "the recorded end of hour 5 names the slot's tag at 5");
    assert_eq!(recorded.rows[0].raider.tag.as_deref(), Some("DDR"), "the row keeps its own hour's raider");
}

#[test]
fn statuses_and_the_range_apply_at_the_hour_before_the_count_and_the_slice() {
    let prep = |id| raid(id, 1, native::PHASE_PREPARING, native::OUTCOME_NONE);
    let won = |id| raid(id, 1, native::PHASE_ENDED, native::OUTCOME_SUCCESS);
    let source = opened("tail-status", &recording(&[vec![
        at(10, whole(vec![prep(1)]), 30),
        at(11, whole(vec![prep(1), prep(2)]), 30),
        at(12, whole(vec![won(1), prep(2), prep(3)]), 30),
        at(13, Raids::Unread(vec![]), 30),
        at(14, whole(vec![won(1), prep(2), prep(3)]), 30),
    ]]));
    let preparing = |hour| RaidLogTailParams { statuses: Some(vec![RaidStatus::Preparing]), limit: 1, ..ask(hour) };
    let at_11 = load(&source, &preparing(11));
    assert_eq!((at_11.total.as_str(), at_11.rows.len(), at_11.rows[0].hour), ("2", 1, 11), "two raids prepare at 11; the window holds the newest");
    let at_12 = load(&source, &preparing(12));
    assert_eq!((at_12.total.as_str(), at_12.counts[0].total.as_str()), ("2", "2"), "raid 1 succeeded at 12 and leaves the filter");
    assert_eq!(load(&source, &RaidLogTailParams { statuses: Some(vec![RaidStatus::NotRead]), ..ask(13) }).total, "2", "an unread hour is not read, never still preparing");
    let ranged = load(&source, &RaidLogTailParams { from: Some(11), to: Some(11), ..ask(14) });
    assert_eq!((ranged.total.as_str(), ranged.rows[0].hour), ("1", 11));
    let all = load(&source, &ask(14));
    assert_eq!(all.rows.iter().map(|row| (row.hour, row.live)).collect::<Vec<_>>(), [(12, true), (11, true), (10, false)], "live raids first, newest first, then history");
    assert_eq!(all.missing.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect::<Vec<_>>(), [(13, 13)]);
    assert!(load(&source, &ask(12)).missing.is_empty(), "the unread hour of 13 is not in a reply at 12");
    stepped_equals_cold(&source, serde_json::json!({ "panel": "raid-log-tail", "branchId": "1", "limit": 2 }), &[10, 11, 12, 13, 14, 11, 14]);
}

#[test]
fn a_ready_raid_keeps_the_preparation_last_read_and_only_raids_in_preparation_are_counted() {
    let mut prep = raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE);
    let ready = raid(7, 1, native::PHASE_PREPARED, native::OUTCOME_NONE);
    let going = raid(8, 1, native::PHASE_IN_PROGRESS, native::OUTCOME_NONE);
    let source = opened("tail-ready", &recording(&[vec![
        at(10, whole(vec![prep.clone()]), 30),
        at(11, whole(vec![{ prep.preparation_hours = 2; prep }]), 30),
        at(12, whole(vec![ready.clone(), going.clone()]), 30),
        at(13, whole(vec![ready, going]), 30),
    ]]));
    let reply = load(&source, &ask(13));
    let row = reply.rows.iter().find(|row| row.status == RaidStatus::Prepared).expect("the raid read as ready to launch");
    let preparation = row.preparation.as_ref().expect("a raid ready to launch shows the preparation it was last read with");
    assert_eq!((preparation.since, preparation.hours, preparation.per_hour, preparation.through), (10, 1, 1, Some(11)));
    assert_eq!((reply.live.as_str(), reply.counts[0].total.as_str()), ("2", "1"), "a raid in progress is live but not in preparation");
    stepped_equals_cold(&source, serde_json::json!({ "panel": "raid-log-tail", "branchId": "1", "limit": 64 }), &[10, 11, 12, 13, 10, 13]);
}

#[test]
fn a_tokens_spelling_is_dated_from_its_first_read_and_follows_a_change_of_text() {
    let prep = raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE);
    let frames = (10..=20)
        .map(|hour| {
            let mut frame = super::frame_at(1, &At { spelled: hour >= 12, ..at(hour, whole(vec![prep.clone()]), 30) });
            if hour >= 16
                && let TableView::Complete(TableRows::LexicalTokens(tokens)) = &mut *frame.tables[replay_data::rows::TableId::LexicalTokens.index()]
            {
                tokens.iter_mut().filter(|token| token.token == 40).for_each(|token| token.literal = b"sky_raid".to_vec());
            }
            frame
        })
        .collect();
    let source = opened("tail-spellings", &[records(1, frames)]);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let run = query.timeline.stretches[0].run;
    let spellings = crate::products::raids::dated_spellings(source.replay_ref().recording(), run, &[40u128, 41, 99].into_iter().collect(), 10, 20).unwrap();
    assert_eq!(spellings[&40], [(12, "air_raid".to_owned()), (16, "sky_raid".to_owned())], "each spelling from the hour it was first read");
    assert_eq!(spellings[&41], [(10, "dam_raid".to_owned())]);
    assert!(!spellings.contains_key(&99), "a token never spelled has no spelling");
}

#[test]
fn a_spelling_read_before_an_unread_or_truncated_dictionary_keeps_its_first_hour() {
    let prep = raid(7, 1, native::PHASE_PREPARING, native::OUTCOME_NONE);
    let frames = (10..=20)
        .map(|hour| {
            let mut frame = super::frame_at(1, &at(hour, whole(vec![prep.clone()]), 30));
            if (13..=17).contains(&hour) {
                *frame.tables[replay_data::rows::TableId::LexicalTokens.index()] = TableView::Unavailable { reason: "not read this hour".into() };
            }
            if hour >= 18 {
                *frame.tables[replay_data::rows::TableId::LexicalTokens.index()] =
                    TableView::Truncated { rows: TableRows::LexicalTokens(vec![replay_data::rows::LexicalTokenRow { token: 41, literal: b"dam_raid".to_vec() }]), missing: 1 };
            }
            frame
        })
        .collect();
    let source = opened("tail-spelling-gap", &[records(1, frames)]);
    let run = dashboard_base::queries::query(&source, 1).unwrap().timeline.stretches[0].run;
    let spellings = crate::products::raids::dated_spellings(source.replay_ref().recording(), run, &[40u128].into_iter().collect(), 10, 20).unwrap();
    assert_eq!(spellings[&40], [(10, "air_raid".to_owned())]);
}
