use super::prepared::Prepared;
use super::*;
use dashboard_base::{
    Source, SourceKey,
    prepared::{
        coverage::{Coverage, SCHEMA},
        frame_changes::FrameChanges,
        prefix::Prefix,
        series::tests::Fixture,
    },
    test_fixtures as fixtures,
};
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = fixtures::source(1, hour, Some(0));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"mod_a".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 101,
                canonical_slot: 7,
            },
        ]));
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![
        FactionRow {
            id: 9,
            ..Default::default()
        },
        FactionRow {
            id: 10,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 11,
            faction: 9,
            ordinal: 0,
        }]));
    *frame.tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(vec![FactionAccountRow {
            account: 3,
            country: 101,
            influence_refresh: 1,
            influence_numerator_raw: 1,
            influence_denominator_raw: 4,
            influence_faction: 9,
            ..Default::default()
        }]));
    *frame.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(Vec::new()));
    *frame.tables[TableId::FactionPrograms.index()] =
        TableView::Complete(TableRows::FactionPrograms(Vec::new()));
    *frame.tables[TableId::FactionMinimumUpgrades.index()] =
        TableView::Complete(TableRows::FactionMinimumUpgrades(Vec::new()));
    *frame.tables[TableId::FactionDoctrine.index()] =
        TableView::Complete(TableRows::FactionDoctrine(Vec::new()));
    *frame.tables[TableId::FactionManpower.index()] =
        TableView::Complete(TableRows::FactionManpower(vec![
            FactionManpowerRow {
                faction: 9,
                ordinal: 0,
                country: 11,
                value: i32::MAX,
            },
            FactionManpowerRow {
                faction: 9,
                ordinal: 1,
                country: 101,
                value: 1,
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(Vec::new()));
    frame
}
fn source(frames: Vec<DecodedFrame>) -> Source {
    let records = fixtures::records(1, frames);
    Source::from_replay(
        SourceKey {
            source_id: "faction-test".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(
            replay::container::Replay::from_records(fixtures::CAMPAIGN.into(), &[records]).unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn observe(state: &mut Prepared, source: &Source, frame: &DecodedFrame) {
    state
        .observe(
            source,
            &LoadContext { content: None },
            frame,
            &FrameChanges::alone(frame, replay_data::format::FormatVersion::CURRENT),
            &AtomicBool::new(false),
        )
        .unwrap();
}
fn publish(state: &mut Prepared, files: &Fixture) {
    let mut writer = files.writer();
    state
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    state.save(&mut writer, 1).unwrap();
    writer.commit().unwrap();
}
fn reply(
    source: &Source,
    files: &Fixture,
    params: FactionStateParams,
    last: i32,
    complete: bool,
) -> FactionStateReply {
    let prefix = Prefix::new(
        source,
        1,
        Coverage {
            schema: SCHEMA,
            first_hour: 100,
            last_hour: last,
            complete,
        },
        None,
    );
    prepared::query(
        source,
        &params,
        &LoadContext { content: None },
        &files.store,
        &files.key,
        prefix,
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn influence(source: &Source, files: &Fixture, faction: &str, last: i32) -> FactionInfluenceReply {
    match reply(
        source,
        files,
        FactionStateParams::Influence {
            branch_id: "1".into(),
            faction: faction.into(),
        },
        last,
        true,
    ) {
        FactionStateReply::Influence(reply) => reply,
        _ => unreachable!(),
    }
}
#[test]
fn faction_influence_matches_existing_detail_and_preserves_native_zero_denominator() {
    let first = frame(100);
    let mut second = frame(101);
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *second.tables[TableId::FactionAccounts.index()]
    {
        rows[0].influence_denominator_raw = 0;
    }
    let source = source(vec![first.clone(), second.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &first);
    observe(&mut state, &source, &second);
    let files = Fixture::new();
    publish(&mut state, &files);
    let history = influence(&source, &files, "9", 101);
    assert_eq!(history.accounts.len(), 1);
    assert_eq!(
        history.accounts[0]
            .runs
            .iter()
            .map(|run| run.share_raw.as_deref())
            .collect::<Vec<_>>(),
        [Some("25000"), Some("0")]
    );
    for hour in [100, 101] {
        let detail = dashboard_base::products::faction_details::FactionDetails::load(
            &source,
            &dashboard_base::products::faction_details::FactionDetailParams {
                branch_id: "1".into(),
                hour,
                faction: "9".into(),
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let account = &detail.accounts.unwrap()[0];
        assert_eq!(
            history.accounts[0]
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .unwrap()
                .share_raw
                .as_deref(),
            Some(account.influence_share_raw.as_str())
        );
    }
}
#[test]
fn retained_accounts_need_provenance_and_do_not_leak_into_other_factions() {
    let first = frame(100);
    let mut second = frame(101);
    *second.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    let mut third = frame(102);
    *third.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 11,
            faction: 10,
            ordinal: 0,
        }]));
    let source = source(vec![first.clone(), second.clone(), third.clone()]);
    let files = Fixture::new();
    let mut state = Prepared::default();
    for frame in [&first, &second, &third] {
        observe(&mut state, &source, frame);
    }
    publish(&mut state, &files);
    let old = influence(&source, &files, "9", 102);
    assert_eq!(old.accounts[0].member[0].last_hour, 100);
    assert_eq!(old.accounts[0].retained[0].first_hour, 101);
    assert_eq!(old.accounts[0].retained[0].last_hour, 101);
    assert!(old.accounts[0].runs.iter().all(|run| run.last_hour < 102));
    let current = influence(&source, &files, "10", 102);
    assert_eq!(current.accounts.len(), 1);
    assert!(current.accounts[0].retained.is_empty());
    assert_eq!(current.accounts[0].member[0].first_hour, 102);
}
#[test]
fn unknown_membership_and_recording_gaps_are_not_zero_or_membership_events() {
    let first = frame(100);
    let mut second = frame(101);
    *second.tables[TableId::FactionAccounts.index()] = TableView::Unavailable {
        reason: "synthetic unavailable accounts".into(),
    };
    let third = frame(103);
    let source = source(vec![first.clone(), second.clone(), third.clone()]);
    let files = Fixture::new();
    let mut state = Prepared::default();
    for frame in [&first, &second, &third] {
        observe(&mut state, &source, frame);
    }
    publish(&mut state, &files);
    let reply = influence(&source, &files, "9", 103);
    assert!(
        reply.accounts[0]
            .runs
            .iter()
            .any(|run| run.first_hour == 101 && run.share_raw.is_none())
    );
    assert!(
        reply.accounts[0]
            .runs
            .iter()
            .any(|run| run.first_hour == 103 && run.starts_segment)
    );
    assert!(
        reply
            .unobserved
            .iter()
            .any(|span| span.first_hour <= 102 && span.last_hour >= 102)
    );
}
#[test]
fn manpower_keeps_repeated_alias_occurrences_and_signed_wrapping_total() {
    let frame = frame(100);
    let source = source(vec![frame.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &frame);
    let files = Fixture::new();
    publish(&mut state, &files);
    let FactionStateReply::Manpower(reply) = reply(
        &source,
        &files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        100,
        true,
    ) else {
        unreachable!()
    };
    assert_eq!(reply.members.len(), 2);
    assert_eq!(reply.members[0].tag, reply.members[1].tag);
    assert_ne!(reply.members[0].key, reply.members[1].key);
    for member in &reply.members {
        assert_eq!((member.slot, member.country_id), (Some(7), Some(101)));
    }
    assert_eq!(reply.total[0].value, Some(i32::MIN));
    assert_eq!(reply.members[0].runs[0].value, Some(i32::MAX));
}
#[test]
fn native_country_replacement_splits_member_lanes_even_when_the_tag_and_slot_are_reused() {
    let mut first = frame(100);
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *first.tables[TableId::FactionAccounts.index()]
    {
        rows[0].upgrade_groups = vec![1];
        rows[0].upgrade_tokens = vec![2];
    }
    *first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"mod_group".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"mod_upgrade".to_vec(),
            },
        ]));
    *first.tables[TableId::FactionManpower.index()] =
        TableView::Complete(TableRows::FactionManpower(vec![FactionManpowerRow {
            faction: 9,
            ordinal: 0,
            country: 11,
            value: 5,
        }]));
    let mut second = first.clone();
    second.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].id = 102;
    }
    let source = source(vec![first.clone(), second.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &first);
    observe(&mut state, &source, &second);
    let files = Fixture::new();
    publish(&mut state, &files);
    let influence = influence(&source, &files, "9", 101);
    assert_eq!(influence.accounts.len(), 2);
    assert_ne!(influence.accounts[0].key, influence.accounts[1].key);
    assert_eq!(influence.accounts[0].tag, influence.accounts[1].tag);
    assert_eq!(influence.accounts[0].runs[0].last_hour, 100);
    assert_eq!(influence.accounts[1].runs[0].first_hour, 101);
    assert_eq!(
        influence
            .accounts
            .iter()
            .map(|member| (member.slot, member.country_id))
            .collect::<Vec<_>>(),
        [(Some(7), Some(101)), (Some(7), Some(102))]
    );
    let rules = rules(&source, &files, "9", 101);
    assert_eq!(rules.members.len(), 2);
    assert_ne!(rules.members[0].key, rules.members[1].key);
    assert_eq!(rules.members[0].tag, rules.members[1].tag);
    assert_eq!(
        rules
            .members
            .iter()
            .map(|member| (member.slot, member.country_id))
            .collect::<Vec<_>>(),
        [(Some(7), Some(101)), (Some(7), Some(102))]
    );
    let FactionStateReply::Manpower(manpower) = reply(
        &source,
        &files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        101,
        true,
    ) else {
        unreachable!()
    };
    assert_eq!(manpower.members.len(), 2);
    assert_ne!(manpower.members[0].key, manpower.members[1].key);
    assert_eq!(manpower.members[0].tag, manpower.members[1].tag);
    assert_eq!(manpower.members[0].runs[0].last_hour, 100);
    assert_eq!(manpower.members[1].runs[0].first_hour, 101);
    assert_eq!(
        manpower
            .members
            .iter()
            .map(|member| (member.slot, member.country_id))
            .collect::<Vec<_>>(),
        [(Some(7), Some(101)), (Some(7), Some(102))]
    );
}

#[test]
fn cold_prefix_is_pending_not_empty_known_and_cancellation_never_returns_a_reply() {
    let source = source(vec![frame(100), frame(101)]);
    let files = Fixture::new();
    let FactionStateReply::Influence(reply) = reply(
        &source,
        &files,
        FactionStateParams::Influence {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        99,
        false,
    ) else {
        unreachable!()
    };
    assert!(reply.accounts.is_empty());
    assert!(reply.unobserved.is_empty());
    assert_eq!(reply.pending[0].first_hour, 100);
    assert_eq!(reply.pending[0].last_hour, 101);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: SCHEMA,
            first_hour: 100,
            last_hour: 99,
            complete: false,
        },
        None,
    );
    assert!(
        prepared::query(
            &source,
            &FactionStateParams::Influence {
                branch_id: "1".into(),
                faction: "9".into()
            },
            &LoadContext { content: None },
            &files.store,
            &files.key,
            prefix,
            &AtomicBool::new(true)
        )
        .is_err()
    );
}
#[test]
fn continuation_and_append_publish_the_same_observations_as_uninterrupted_preparation() {
    let first = frame(100);
    let mut second = frame(101);
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *second.tables[TableId::FactionAccounts.index()]
    {
        rows[0].influence_numerator_raw = 3;
    }
    let source = source(vec![first.clone(), second.clone()]);
    let full_files = Fixture::new();
    let resumed_files = Fixture::new();
    let mut full = Prepared::default();
    observe(&mut full, &source, &first);
    observe(&mut full, &source, &second);
    publish(&mut full, &full_files);
    let mut resumed = Prepared::default();
    observe(&mut resumed, &source, &first);
    publish(&mut resumed, &resumed_files);
    let file = resumed_files
        .store
        .read(&resumed_files.key, &prepared::state_section(1), 0)
        .unwrap()
        .unwrap();
    let mut resumed: Prepared =
        serde_json::from_reader(zstd::stream::Decoder::new(file).unwrap()).unwrap();
    observe(&mut resumed, &source, &second);
    publish(&mut resumed, &resumed_files);
    assert_eq!(
        serde_json::to_value(influence(&source, &full_files, "9", 101)).unwrap(),
        serde_json::to_value(influence(&source, &resumed_files, "9", 101)).unwrap()
    );
}
#[test]
fn five_thousand_country_accounts_are_sparse_and_have_no_cross_faction_duplication() {
    let mut frame = frame(100);
    let count = 5000u32;
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..count)
            .map(|id| CountryRow {
                id: id + 1,
                canonical_slot: id + 1,
                tag: format!("mod_{id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..count)
            .map(|id| CountryLinkRow {
                id: id + 1,
                canonical_slot: id + 1,
            })
            .collect(),
    ));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            (0..count)
                .map(|id| FactionMemberRow {
                    country: id + 1,
                    faction: if id % 2 == 0 { 9 } else { 10 },
                    ordinal: id / 2,
                })
                .collect(),
        ));
    *frame.tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(
            (0..count)
                .map(|id| FactionAccountRow {
                    account: id,
                    country: id + 1,
                    influence_share_raw: id as i64,
                    ..Default::default()
                })
                .collect(),
        ));
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    {
        rows.sort_by_key(RowCodec::key);
    }
    let mut second = frame.clone();
    second.hour = 101;
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *second.tables[TableId::FactionAccounts.index()]
    {
        rows[0].influence_share_raw = 999;
    }
    let source = source(vec![frame.clone(), second]);
    let mut state = Prepared::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    source
        .replay_ref()
        .read_block_alone(
            &source.replay_ref().index[0],
            block::TABLES,
            |raw, frame| {
                let changes = tracker.observe_updates(frame, raw, source.replay_ref().format);
                state.observe(
                    &source,
                    &LoadContext { content: None },
                    frame,
                    &changes,
                    &AtomicBool::new(false),
                )
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        state.account_projections, 5001,
        "only the changed account header is projected from the recorded delta"
    );
    let files = Fixture::new();
    publish(&mut state, &files);
    let a = influence(&source, &files, "9", 100);
    let b = influence(&source, &files, "10", 100);
    assert_eq!(a.accounts.len() + b.accounts.len(), count as usize);
    assert_eq!(a.accounts.len(), 2500);
    let keys: std::collections::BTreeSet<_> =
        a.accounts.iter().map(|account| &account.key).collect();
    assert!(
        b.accounts
            .iter()
            .all(|account| !keys.contains(&account.key))
    );
}

fn rules(source: &Source, files: &Fixture, faction: &str, last: i32) -> FactionRulesReply {
    match reply(
        source,
        files,
        FactionStateParams::Rules {
            branch_id: "1".into(),
            faction: faction.into(),
        },
        last,
        true,
    ) {
        FactionStateReply::Rules(reply) => reply,
        _ => unreachable!(),
    }
}
#[test]
fn unavailable_rules_do_not_hide_programmes_minimums_or_doctrines_and_refusals_are_scoped() {
    let mut first = frame(100);
    *first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"mod_rule".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"mod_group".to_vec(),
            },
            LexicalTokenRow {
                token: 3,
                literal: b"mod_upgrade".to_vec(),
            },
            LexicalTokenRow {
                token: 4,
                literal: b"mod_doctrine".to_vec(),
            },
        ]));
    *first.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(vec![
            FactionRuleRow {
                faction: 9,
                ordinal: 0,
                token: 1,
            },
            FactionRuleRow {
                faction: 9,
                ordinal: 1,
                token: 1,
            },
        ]));
    *first.tables[TableId::FactionMinimumUpgrades.index()] =
        TableView::Complete(TableRows::FactionMinimumUpgrades(vec![
            FactionMinimumUpgradeRow {
                faction: 9,
                ordinal: 0,
                group: 2,
                token: 3,
            },
        ]));
    *first.tables[TableId::FactionDoctrine.index()] =
        TableView::Complete(TableRows::FactionDoctrine(vec![FactionDoctrineRow {
            faction: 9,
            ordinal: 0,
            token: 4,
        }]));
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::FactionRules.index()] = TableView::Unavailable {
        reason: "synthetic missing rules".into(),
    };
    let source = source(vec![first.clone(), second.clone()]);
    let files = Fixture::new();
    let mut state = Prepared::default();
    observe(&mut state, &source, &first);
    observe(&mut state, &source, &second);
    publish(&mut state, &files);
    let history = rules(&source, &files, "9", 101);
    assert_eq!(history.rules.len(), 2);
    assert_eq!(history.rules[0].runs[0].token.as_deref(), Some("mod_rule"));
    assert!(history.rules[0].runs[1].token.is_none());
    assert!(
        history.rules[0].runs[1]
            .reason
            .as_ref()
            .unwrap()
            .contains("synthetic missing rules")
    );
    assert_eq!(
        history.minimums[0].runs[0].token.as_deref(),
        Some("mod_upgrade")
    );
    assert_eq!(history.groups[0].group, 2);
    let detail = dashboard_base::products::faction_details::FactionDetails::load(
        &source,
        &dashboard_base::products::faction_details::FactionDetailParams {
            branch_id: "1".into(),
            hour: 100,
            faction: "9".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    let doctrines = detail.doctrine_folders.unwrap();
    let doctrine = &history.doctrines[0].runs[0];
    assert_eq!(doctrine.token.as_deref(), Some("mod_doctrine"));
    assert_eq!(doctrine.token, doctrines[0].token);
    assert_eq!(doctrine.name, doctrines[0].name);
    assert_eq!(doctrine.reason, doctrines[0].reason);
    assert_eq!(
        history
            .channels
            .iter()
            .find(|c| c.channel == "rules")
            .unwrap()
            .known[0]
            .last_hour,
        100
    );
    assert_eq!(
        history
            .channels
            .iter()
            .find(|c| c.channel == "programmes")
            .unwrap()
            .known[0]
            .last_hour,
        101
    );
    let unrelated = rules(&source, &files, "10", 101);
    assert!(
        unrelated
            .warnings
            .iter()
            .all(|why| !why.contains("upgrade group"))
    );
}
#[test]
fn balance_only_hours_reuse_country_joins_and_project_only_changed_account_headers() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(123);
    }
    let mut third = second.clone();
    third.hour = 102;
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *third.tables[TableId::FactionAccounts.index()]
    {
        rows[0].influence_numerator_raw = 2;
    }
    let source = source(vec![first.clone(), second.clone(), third.clone()]);
    let mut state = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in [&first, &second] {
        let changes = tracker.observe(frame, replay_data::format::FormatVersion::CURRENT);
        state
            .observe(
                &source,
                &LoadContext { content: None },
                frame,
                &changes,
                &AtomicBool::new(false),
            )
            .unwrap();
    }
    assert_eq!(state.country_rebuilds(), 1);
    assert_eq!(state.account_projections, 1);
    let changes = tracker.observe(&third, replay_data::format::FormatVersion::CURRENT);
    state
        .observe(
            &source,
            &LoadContext { content: None },
            &third,
            &changes,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(state.account_projections, 2);
}
#[test]
fn disappeared_accounts_or_missing_identity_do_not_keep_a_stale_faction_association() {
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(Vec::new()));
    let mut third = first.clone();
    third.hour = 102;
    *third.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *third.tables[TableId::FactionAccounts.index()]
    {
        rows[0].influence_faction = 0;
    }
    let source = source(vec![first.clone(), second.clone(), third.clone()]);
    let mut state = Prepared::default();
    for frame in [&first, &second, &third] {
        observe(&mut state, &source, frame);
    }
    let files = Fixture::new();
    publish(&mut state, &files);
    assert!(
        influence(&source, &files, "9", 102).accounts[0]
            .runs
            .iter()
            .all(|run| run.last_hour == 100)
    );
}
#[test]
fn owner_witnesses_are_independent_but_an_unproven_empty_manpower_pool_is_never_zero() {
    let mut first = frame(100);
    *first.tables[TableId::Factions.index()] = TableView::Unavailable {
        reason: "synthetic missing names".into(),
    };
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    *second.tables[TableId::FactionManpower.index()] =
        TableView::Complete(TableRows::FactionManpower(Vec::new()));
    let source = source(vec![first.clone(), second.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &first);
    observe(&mut state, &source, &second);
    let files = Fixture::new();
    publish(&mut state, &files);
    let FactionStateReply::Manpower(history) = reply(
        &source,
        &files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        101,
        true,
    ) else {
        unreachable!()
    };
    assert_eq!(history.total[0].value, Some(i32::MIN));
    assert_eq!(history.total[1].value, None);
}
#[test]
fn a_short_query_prefix_cannot_leak_a_later_selection_or_its_warning() {
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(vec![FactionRuleRow {
            faction: 9,
            ordinal: 0,
            token: 999,
        }]));
    let source = source(vec![first.clone(), second.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &first);
    observe(&mut state, &source, &second);
    let files = Fixture::new();
    publish(&mut state, &files);
    let early = rules(&source, &files, "9", 100);
    assert!(early.rules.is_empty());
    assert!(early.warnings.is_empty());
    assert_eq!(rules(&source, &files, "9", 101).rules.len(), 1);
}
#[test]
fn an_unknown_manpower_span_can_cover_many_hours_without_numeric_bucket_overflow() {
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::FactionManpower.index()] = TableView::Unavailable {
        reason: "synthetic".into(),
    };
    let source = source(
        std::iter::once(first.clone())
            .chain((101..=200).map(|hour| {
                let mut held = second.clone();
                held.hour = hour;
                held
            }))
            .collect(),
    );
    let mut state = Prepared::default();
    observe(&mut state, &source, &first);
    observe(&mut state, &source, &second);
    state.quiet(102, 200, [true; 7]).unwrap();
    let files = Fixture::new();
    publish(&mut state, &files);
    let FactionStateReply::Manpower(history) = reply(
        &source,
        &files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        200,
        true,
    ) else {
        unreachable!()
    };
    assert_eq!(history.total.last().unwrap().value, None);
    assert_eq!(history.total.last().unwrap().last_hour, 200);
}

#[test]
fn shared_changes_only_block_fold_matches_hourly_observation_without_query_decodes() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay::container::BlockChange;
    let frames: Vec<_> = (100..110)
        .map(|hour| {
            let mut f = frame(hour);
            // Complete optional inputs let this fixture prove genuinely quiet
            // balance-only reads, rather than repeated unavailable acquisitions.
            for &table in block::TABLES {
                if table != TableId::Campaign
                    && !matches!(*f.tables[table.index()], TableView::Complete(_))
                {
                    *f.tables[table.index()] =
                        TableView::Complete(if table == TableId::ReservedCountryNaming {
                            TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                                tag: b"mod_reserved".to_vec(),
                                ..Default::default()
                            }])
                        } else {
                            TableRows::empty(table)
                        });
                }
            }
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *f.tables[TableId::Countries.index()]
            {
                rows[0].political_power_raw = Some(hour as i64);
            }
            f
        })
        .collect();
    let source = source(frames);
    let replay = source.replay_ref();
    let index = &replay.index[0];
    let flag = AtomicBool::new(false);
    let mut hourly = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    replay
        .read_block_alone(
            index,
            block::TABLES,
            |raw, frame| {
                let changes = tracker.observe_updates(frame, raw, replay.format);
                hourly.observe(
                    &source,
                    &LoadContext { content: None },
                    frame,
                    &changes,
                    &flag,
                )
            },
            || Ok(()),
        )
        .unwrap();
    let mut extract = (block::FAMILY.start)(replay.format);
    extract.read_by_changes();
    let fields = (block::FAMILY.fields.unwrap())().unwrap();
    let mut previous: Option<i32> = None;
    let mut kept = 0;
    replay
        .read_block_changes(
            index,
            &fields,
            |change| match change {
                BlockChange::Changed { frame, rebuilt, .. } => {
                    kept += 1;
                    let continues = previous.and_then(|h| h.checked_add(1)) == Some(frame.hour);
                    previous = Some(frame.hour);
                    let changes = FrameChanges::rebuilt(frame, replay.format, continues, |table| {
                        rebuilt.contains(&table)
                    });
                    extract.observe(replay, frame, &changes)
                }
                BlockChange::Quiet(hours) => {
                    previous = Some(hours.last());
                    extract.quiet(replay, hours, 0..hours.hours().len())
                }
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(kept, 1, "balance-only hours are quiet for this family");
    let mut folded = Prepared::default();
    let (how, reached) = block::fold(
        &mut folded,
        extract.finish().unwrap(),
        &source,
        &LoadContext { content: None },
        replay,
        index,
        &mut |_, result| result.map(|_| true),
        &mut || Ok(()),
        &flag,
    )
    .unwrap();
    assert!(matches!(
        how,
        dashboard_base::prepared::prepass::Folded::Fast
    ));
    assert_eq!(reached, Some(109));
    let a = Fixture::new();
    let b = Fixture::new();
    publish(&mut hourly, &a);
    publish(&mut folded, &b);
    for params in [
        FactionStateParams::Influence {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        FactionStateParams::Rules {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "9".into(),
        },
    ] {
        let expected = reply(&source, &a, params.clone(), 109, true);
        let (actual, decoded) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                reply(&source, &b, params, 109, true)
            });
        assert!(!decoded);
        assert_eq!(
            serde_json::to_value(expected).unwrap(),
            serde_json::to_value(actual).unwrap()
        );
    }
}
#[test]
fn programme_references_use_only_their_existing_daily_supplier_and_match_the_detail_join() {
    let reference =
        |id| ((id as u64) << 32) | u64::from(engine_projects::nproject_cprogram::REFERENCE_TYPE);
    let frames: Vec<_> = (100..=124)
        .map(|hour| {
            let mut f = frame(hour);
            *f.tables[TableId::FactionPrograms.index()] =
                TableView::Complete(TableRows::FactionPrograms(vec![FactionProgramRow {
                    faction: 9,
                    ordinal: 0,
                    reference: reference(if hour == 100 { 17 } else { 18 }),
                }]));
            *f.tables[TableId::ProjectPrograms.index()] =
                TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                    id: if hour < 124 { 17 } else { 18 },
                    owner: 11,
                    ..Default::default()
                }]));
            *f.tables[TableId::Projects.index()] =
                TableView::Complete(TableRows::Projects(Vec::new()));
            f.metrics.table_ns[TableId::ProjectPrograms.index()] =
                u64::from(hour == 100 || hour == 124);
            f
        })
        .collect();
    let source = source(frames.clone());
    let mut state = Prepared::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for frame in &frames {
        let changes = tracker.observe(frame, replay_data::format::FormatVersion::CURRENT);
        state
            .observe(
                &source,
                &LoadContext { content: None },
                frame,
                &changes,
                &AtomicBool::new(false),
            )
            .unwrap();
    }
    let files = Fixture::new();
    publish(&mut state, &files);
    let early = rules(&source, &files, "9", 101);
    let late = rules(&source, &files, "9", 124);
    let old = &early.programmes[0].runs[1];
    assert_eq!(
        old.token.as_deref(),
        Some(reference(18).to_string()).as_deref()
    );
    assert!(old.reason.as_ref().unwrap().contains("not recorded"));
    assert!(late.programmes[0].runs.last().unwrap().reason.is_none());
    assert_eq!(late.programmes[0].runs.last().unwrap().first_hour, 124);
    let detail = dashboard_base::products::faction_details::FactionDetails::load(
        &source,
        &dashboard_base::products::faction_details::FactionDetailParams {
            branch_id: "1".into(),
            hour: 101,
            faction: "9".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(detail.programmes_read_hour, Some(100));
    assert_eq!(detail.programmes.unwrap()[0].reason, old.reason);
}

#[test]
fn an_oversized_faction_reply_refuses_instead_of_truncating_lanes() {
    let mut f = frame(100);
    *f.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(vec![FactionRuleRow {
            faction: 9,
            ordinal: 0,
            token: 1,
        }]));
    let source = source(vec![f.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &f);
    // The hour's runs are committed before the saved state is edited (the edit
    // round-trips the state, which keeps only what a commit saves); a second commit
    // then writes the catalogue with the oversized name.
    let files = Fixture::new();
    let mut writer = files.writer();
    state
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let mut saved = serde_json::to_value(&state).unwrap();
    let lane = saved["catalogue"]["lanes"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .find(|lane| lane["kind"] == "rule")
        .unwrap();
    lane["choices"][0]["name"] =
        serde_json::Value::String("x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES));
    let mut state: Prepared = serde_json::from_value(saved).unwrap();
    publish(&mut state, &files);
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: SCHEMA,
            first_hour: 100,
            last_hour: 100,
            complete: true,
        },
        None,
    );
    let error = prepared::query(
        &source,
        &FactionStateParams::Rules {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        &LoadContext { content: None },
        &files.store,
        &files.key,
        prefix,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("transport reply budget"));
}

#[test]
fn manpower_negative_values_and_the_zero_country_keep_the_native_total_and_reserved_tag() {
    let mut frame = frame(100);
    *frame.tables[TableId::FactionManpower.index()] =
        TableView::Complete(TableRows::FactionManpower(vec![
            FactionManpowerRow {
                faction: 9,
                ordinal: 0,
                country: 0,
                value: -5,
            },
            FactionManpowerRow {
                faction: 9,
                ordinal: 1,
                country: 101,
                value: -7,
            },
        ]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"mod_reserved".to_vec(),
                ..Default::default()
            },
        ]));
    let source = source(vec![frame.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &frame);
    let files = Fixture::new();
    publish(&mut state, &files);
    let FactionStateReply::Manpower(history) = reply(
        &source,
        &files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "9".into(),
        },
        100,
        true,
    ) else {
        unreachable!()
    };
    assert_eq!(history.total[0].value, Some(-12));
    assert!(
        history
            .members
            .iter()
            .any(|member| member.tag == "mod_reserved" && member.runs[0].value == Some(-5))
    );
    let reserved = history
        .members
        .iter()
        .find(|member| member.tag == "mod_reserved")
        .unwrap();
    assert!(!reserved.key.is_empty());
    assert_eq!((reserved.slot, reserved.country_id), (None, None));
    let member = history
        .members
        .iter()
        .find(|member| member.tag == "mod_a")
        .unwrap();
    assert_ne!(reserved.key, member.key);
    assert_eq!((member.slot, member.country_id), (Some(7), Some(101)));
    let detail = dashboard_base::products::faction_details::FactionDetails::load(
        &source,
        &dashboard_base::products::faction_details::FactionDetailParams {
            branch_id: "1".into(),
            hour: 100,
            faction: "9".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(detail.manpower_total, history.total[0].value);
    let FactionStateReply::Manpower(empty) = reply(
        &source,
        &files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: "10".into(),
        },
        100,
        true,
    ) else {
        unreachable!()
    };
    assert_eq!(empty.total[0].value, Some(0));
}

#[test]
fn an_unrelated_unresolved_membership_does_not_erase_a_witnessed_account_share() {
    let mut f = frame(100);
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *f.tables[TableId::FactionMembers.index()]
    {
        rows.push(FactionMemberRow {
            faction: 10,
            ordinal: 0,
            country: 999,
        });
    }
    let source = source(vec![f.clone()]);
    let mut state = Prepared::default();
    observe(&mut state, &source, &f);
    let files = Fixture::new();
    publish(&mut state, &files);
    let result = influence(&source, &files, "9", 100);
    assert_eq!(
        result.accounts[0].runs[0].share_raw.as_deref(),
        Some("25000")
    );
    assert_eq!(result.accounts[0].member[0].first_hour, 100);
    assert!(influence(&source, &files, "10", 100).accounts.is_empty());
}

#[test]
fn a_recovered_rule_starts_a_new_segment_after_an_unavailable_read() {
    let mut first = frame(100);
    *first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"mod_rule".to_vec(),
        }]));
    *first.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(vec![FactionRuleRow {
            faction: 9,
            ordinal: 0,
            token: 1,
        }]));
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::FactionRules.index()] = TableView::Unavailable {
        reason: "synthetic gap".into(),
    };
    let mut third = first.clone();
    third.hour = 102;
    let source = source(vec![first.clone(), second.clone(), third.clone()]);
    let mut state = Prepared::default();
    for frame in [&first, &second, &third] {
        observe(&mut state, &source, frame);
    }
    let files = Fixture::new();
    publish(&mut state, &files);
    let result = rules(&source, &files, "9", 102);
    let runs = &result.rules[0].runs;
    assert_eq!(runs.len(), 3);
    assert!(runs[1].token.is_none());
    assert_eq!(runs[2].token.as_deref(), Some("mod_rule"));
    assert!(runs[2].starts_segment);
}

#[test]
fn an_intervening_native_holder_cannot_restore_an_old_departed_account_owner() {
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].id = 102;
    }
    *second.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    if let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *second.tables[TableId::FactionAccounts.index()]
    {
        rows[0].influence_faction = 0;
    }
    let mut third = second.clone();
    third.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *third.tables[TableId::Countries.index()]
    {
        rows[0].id = 101;
    }
    let source = source(vec![first.clone(), second.clone(), third.clone()]);
    let mut state = Prepared::default();
    for frame in [&first, &second, &third] {
        observe(&mut state, &source, frame);
    }
    let files = Fixture::new();
    publish(&mut state, &files);
    let result = influence(&source, &files, "9", 102);
    assert_eq!(result.accounts.len(), 1);
    assert_eq!(result.accounts[0].runs.last().unwrap().last_hour, 100);
    assert!(result.accounts[0].retained.is_empty());
}
