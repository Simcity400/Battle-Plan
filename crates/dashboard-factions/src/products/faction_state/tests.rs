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
    state
        .quiet(102, 200, [true; prepared::CHANNELS.len()])
        .unwrap();
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
                    *f.tables[table.index()] = TableView::Complete(match table {
                        TableId::ReservedCountryNaming => {
                            TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                                tag: b"mod_reserved".to_vec(),
                                ..Default::default()
                            }])
                        }
                        TableId::LocalPlayer => TableRows::LocalPlayer(vec![Default::default()]),
                        _ => TableRows::empty(table),
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
fn programme_hours_walk_each_supplying_read_once_and_publish_as_read_alone_every_hour() {
    let reference =
        |id| ((id as u64) << 32) | u64::from(engine_projects::nproject_cprogram::REFERENCE_TYPE);
    let frames: Vec<_> = (100..=160)
        .map(|hour| {
            let mut f = frame(hour);
            *f.tables[TableId::FactionPrograms.index()] =
                TableView::Complete(TableRows::FactionPrograms(vec![FactionProgramRow {
                    faction: 9,
                    ordinal: 0,
                    reference: reference(if hour < 130 { 17 } else { 18 }),
                }]));
            let held: &[u32] = match hour {
                ..=123 => &[17],
                124..=147 => &[17, 18],
                _ => &[18],
            };
            *f.tables[TableId::ProjectPrograms.index()] = TableView::Complete(
                TableRows::ProjectPrograms(
                    held.iter()
                        .map(|&id| ProjectProgramRow {
                            id,
                            owner: 11,
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
            *f.tables[TableId::Projects.index()] =
                TableView::Complete(TableRows::Projects(Vec::new()));
            f.metrics.table_ns[TableId::ProjectPrograms.index()] =
                u64::from(matches!(hour, 100 | 124 | 148));
            f
        })
        .collect();
    // Each hour alone seeks its supplying read, as before; a block's hours walk each read once.
    let prepare = |alone: bool| {
        // A source each, so neither finds the other's reads in its frame cache.
        let source = source(frames.clone());
        let mut state = Prepared::default();
        state.point_seeks = alone;
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        let mut supplying = super::prepared::Supplying::new(&source, 1);
        let seeks = dashboard_base::prepared::query::frames::reads(&source).1;
        for frame in &frames {
            let changes = tracker.observe(frame, replay_data::format::FormatVersion::CURRENT);
            let context = LoadContext { content: None };
            let flag = AtomicBool::new(false);
            state
                .observe_supplied(&source, &context, frame, &changes, &mut supplying, &flag)
                .unwrap();
        }
        let seeks = dashboard_base::prepared::query::frames::reads(&source).1 - seeks;
        drop(supplying);
        let files = Fixture::new();
        publish(&mut state, &files);
        (state.supplying_reads, seeks, files)
    };
    let source = source(frames.clone());
    let (walked_reads, walked_seeks, kept) = prepare(false);
    let (every_reads, every_seeks, every) = prepare(true);
    assert_eq!(every_reads, frames.len());
    assert_eq!(walked_reads, 3, "one reconstruction for each supplying read");
    assert_eq!(walked_seeks, 0, "walked supplying reads seek no hour on its own");
    assert!(every_seeks > 0, "point reads seek their hours");
    dashboard_base::prepared::series::tests::assert_payloads_equal(
        &kept.payloads(),
        &every.payloads(),
        "faction-state",
    );
    for last in [101, 123, 124, 135, 147, 148, 160] {
        assert_eq!(
            serde_json::to_value(rules(&source, &kept, "9", last)).unwrap(),
            serde_json::to_value(rules(&source, &every, "9", last)).unwrap(),
        );
    }
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

/// Observes `frames` as continuing hours: a table changed only where its view did.
fn observe_continuing(
    state: &mut Prepared,
    source: &Source,
    context: &LoadContext,
    frames: &[DecodedFrame],
) {
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for frame in frames {
        let changes = tracker.observe(frame, replay_data::format::FormatVersion::CURRENT);
        state
            .observe(source, context, frame, &changes, &AtomicBool::new(false))
            .unwrap();
    }
}
fn manpower_history(source: &Source, files: &Fixture, faction: &str, last: i32) -> FactionManpowerReply {
    match reply(
        source,
        files,
        FactionStateParams::Manpower {
            branch_id: "1".into(),
            faction: faction.into(),
        },
        last,
        true,
    ) {
        FactionStateReply::Manpower(reply) => reply,
        _ => unreachable!(),
    }
}
fn manpower_spans(runs: &[FactionManpowerRun]) -> Vec<(i32, i32, Option<i32>)> {
    runs.iter()
        .map(|run| (run.first_hour, run.last_hour, run.value))
        .collect()
}

#[test]
fn a_reserved_naming_read_alone_closes_and_restores_the_reserved_manpower_member() {
    let mut first = frame(100);
    *first.tables[TableId::FactionManpower.index()] =
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
    *first.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"mod_reserved".to_vec(),
                ..Default::default()
            },
        ]));
    // Only the reserved naming read changes: lost at 101, read again at 102.
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "synthetic naming gap".into(),
    };
    let mut third = first.clone();
    third.hour = 102;
    let mut fourth = first.clone();
    fourth.hour = 103;
    let frames = [first, second, third, fourth];
    let source = source(frames.to_vec());
    let mut state = Prepared::default();
    observe_continuing(&mut state, &source, &LoadContext { content: None }, &frames);
    let files = Fixture::new();
    publish(&mut state, &files);
    let history = manpower_history(&source, &files, "9", 103);
    let reserved = history
        .members
        .iter()
        .find(|member| member.tag == "mod_reserved")
        .unwrap();
    assert_eq!(
        manpower_spans(&reserved.runs),
        [(100, 100, Some(-5)), (101, 101, None), (102, 103, Some(-5))]
    );
    assert!(reserved.runs[2].starts_segment);
    let member = history
        .members
        .iter()
        .find(|member| member.tag == "mod_a")
        .unwrap();
    assert_eq!(manpower_spans(&member.runs), [(100, 103, Some(-7))]);
    assert_eq!(manpower_spans(&history.total), [(100, 103, Some(-12))]);
    assert!(
        history
            .warnings
            .iter()
            .any(|why| why.starts_with("1 faction facts:")
                && why.contains("entry 0 country 0 identity is unavailable")),
        "{:?}",
        history.warnings
    );
}

#[test]
fn an_owner_witnessed_again_or_first_witnessed_gets_its_empty_pool_and_closes_unknown_rules() {
    let mut first = frame(100);
    *first.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"mod_rule".to_vec(),
        }]));
    *first.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(vec![FactionRuleRow {
            faction: 10,
            ordinal: 0,
            token: 1,
        }]));
    // Only the roster witnesses 10: unreadable at 101, when 10's rule also ends.
    let mut second = first.clone();
    second.hour = 101;
    *second.tables[TableId::Factions.index()] = TableView::Unavailable {
        reason: "synthetic roster gap".into(),
    };
    *second.tables[TableId::FactionRules.index()] =
        TableView::Complete(TableRows::FactionRules(Vec::new()));
    // 10 is listed again and 12 first listed; no manpower or rule row changes.
    let mut third = first.clone();
    third.hour = 102;
    third.tables[TableId::FactionRules.index()] =
        second.tables[TableId::FactionRules.index()].clone();
    *third.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
        [9, 10, 12]
            .into_iter()
            .map(|id| FactionRow {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    let mut fourth = third.clone();
    fourth.hour = 103;
    let frames = [first, second, third, fourth];
    let source = source(frames.to_vec());
    let mut state = Prepared::default();
    observe_continuing(&mut state, &source, &LoadContext { content: None }, &frames);
    let files = Fixture::new();
    publish(&mut state, &files);
    assert_eq!(
        manpower_spans(&manpower_history(&source, &files, "10", 103).total),
        [(100, 100, Some(0)), (101, 101, None), (102, 103, Some(0))]
    );
    assert_eq!(
        manpower_spans(&manpower_history(&source, &files, "12", 103).total),
        [(102, 103, Some(0))]
    );
    let history = rules(&source, &files, "10", 103);
    let runs = &history.rules[0].runs;
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 101), (102, 103)]
    );
    assert_eq!(runs[0].token.as_deref(), Some("mod_rule"));
    assert!(runs[1].reason.as_ref().unwrap().contains("owner is unavailable"));
    assert_eq!((&runs[2].token, &runs[2].reason), (&None, &None));
}

#[test]
fn a_scripted_programme_name_reads_its_supplying_hours_world_as_the_faction_details_do() {
    let machine = dashboard_base::test_fixtures::visual_mods::Machine::new("faction_programme_text");
    machine.write(
        "common/defines/mod.lua",
        b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
    );
    machine.write(
        "common/special_projects/projects/mod.txt",
        b"mod_project={specialization=mod_kind}",
    );
    machine.write(
        "common/special_projects/specialization/mod.txt",
        b"mod_kind={color={1 2 3}}",
    );
    machine.write(
        "localisation/english/mod_l_english.yml",
        "\u{feff}l_english:\n mod_project:0 \"[ROOT.GetRulingParty] programme\"\n MOD_PARTY:0 \"Mod party\"\n"
            .as_bytes(),
    );
    let reference =
        (17_u64 << 32) | u64::from(engine_projects::nproject_cprogram::REFERENCE_TYPE);
    let mut f = frame(100);
    *f.tables[TableId::FactionPrograms.index()] =
        TableView::Complete(TableRows::FactionPrograms(vec![FactionProgramRow {
            faction: 9,
            ordinal: 0,
            reference,
        }]));
    *f.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 17,
            owner: 11,
            active_project: Some(5),
            ..Default::default()
        }]));
    *f.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![ProjectRow {
        owner: 11,
        local_id: 5,
        definition_id: 8,
        program_reference: reference,
        started: 1,
        ..Default::default()
    }]));
    *f.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        }]));
    // The name's environment: tables outside the programme's own supplying read.
    *f.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![PoliticsRow {
        country: 11,
        ruling_party: Some(3),
        ..Default::default()
    }]));
    *f.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 11,
            id: 3,
            name_default: 0,
            name: b"MOD_PARTY".to_vec(),
            ..Default::default()
        }]));
    f.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
    let (source, options) = machine.open_frames(vec![f.clone()]);
    let context = LoadContext {
        content: Some(options),
    };
    let flag = AtomicBool::new(false);
    let detail = dashboard_base::products::faction_details::FactionDetails::load(
        &source,
        &dashboard_base::products::faction_details::FactionDetailParams {
            branch_id: "1".into(),
            hour: 100,
            faction: "9".into(),
        },
        &context,
        &flag,
    )
    .unwrap();
    let shown = &detail.programmes.unwrap()[0];
    assert_eq!(shown.project.as_deref(), Some("Mod party programme"));
    for alone in [false, true] {
        let mut state = Prepared::default();
        state.point_seeks = alone;
        let changes = FrameChanges::alone(&f, replay_data::format::FormatVersion::CURRENT);
        state.observe(&source, &context, &f, &changes, &flag).unwrap();
        let files = Fixture::new();
        publish(&mut state, &files);
        let history = rules(&source, &files, "9", 100);
        let run = &history.programmes[0].runs[0];
        assert_eq!(
            (&run.name, &run.reason),
            (&shown.project, &shown.reason),
            "point seeks {alone}"
        );
    }
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


fn goals_params(faction: &str, first_hour: Option<i32>, last_hour: Option<i32>) -> FactionStateParams {
    FactionStateParams::Goals {
        branch_id: "1".into(),
        faction: faction.into(),
        first_hour,
        last_hour,
    }
}
fn goals_reply(
    source: &Source,
    files: &Fixture,
    context: &LoadContext,
    params: FactionStateParams,
    last: i32,
    complete: bool,
) -> FactionGoalsReply {
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
    match prepared::query(
        source,
        &params,
        context,
        &files.store,
        &files.key,
        prefix,
        &AtomicBool::new(false),
    )
    .unwrap()
    {
        FactionStateReply::Goals(reply) => reply,
        _ => unreachable!(),
    }
}
fn goal_frame(hour: i32, entries: Vec<FactionGoalEntryRow>) -> DecodedFrame {
    let mut frame = frame(hour);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"mod_goal".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"mod_scripted".to_vec(),
            },
        ]));
    *frame.tables[TableId::FactionGoalStatus.index()] =
        TableView::Complete(TableRows::FactionGoalStatus(vec![FactionGoalStatusRow {
            faction: 9,
            manifest: None,
        }]));
    *frame.tables[TableId::FactionGoalEntries.index()] =
        TableView::Complete(TableRows::FactionGoalEntries(entries));
    frame
}
fn held(faction: u64, kind: u8, ordinal: u32, token: u32, status: i32, progress_raw: i64) -> FactionGoalEntryRow {
    FactionGoalEntryRow {
        faction,
        kind,
        ordinal,
        token: Some(token),
        status,
        progress_raw,
        ..Default::default()
    }
}
/// What the widget shows of one goal.
fn shown(definition: &dashboard_base::products::faction_details::FactionGoalDefinition) -> serde_json::Value {
    serde_json::json!([definition.token, definition.name, definition.description, definition.icon,
        definition.category, definition.category_name, definition.reason])
}
/// The history's lists at `hour` as the widget reads them; None leaves the hour to its details.
fn goals_at(reply: &FactionGoalsReply, hour: i32) -> Option<serde_json::Value> {
    let run = reply
        .runs
        .iter()
        .find(|run| run.first_hour <= hour && hour <= run.last_hour)?;
    let list = &reply.lists[run.list as usize];
    if list.absent {
        return Some(serde_json::json!("absent"));
    }
    if let Some(reason) = &list.reason {
        return Some(serde_json::json!({ "reason": reason }));
    }
    let definition = |index: u32| shown(&reply.definitions[index as usize]);
    let goals: Vec<_> = list
        .goals
        .iter()
        .map(|goal| {
            let progress = reply
                .progress
                .iter()
                .find(|lane| lane.ordinal == goal.ordinal)
                .and_then(|lane| {
                    lane.runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                });
            serde_json::json!([
                goal.ordinal,
                definition(goal.definition),
                goal.status,
                progress.map(|run| run.progress_raw.clone()),
                progress.and_then(|run| run.progress_percent)
            ])
        })
        .collect();
    Some(serde_json::json!({
        "goals": goals,
        "completed": list.completed.iter().map(|index| definition(*index)).collect::<Vec<_>>(),
        "cancelled": list.cancelled.iter().map(|index| definition(*index)).collect::<Vec<_>>(),
    }))
}
/// The same projection of the hour's own faction-details reply.
fn details_at(source: &Source, context: &LoadContext, faction: &str, hour: i32) -> Result<serde_json::Value, String> {
    let reply = dashboard_base::products::faction_details::FactionDetails::load(
        source,
        &dashboard_base::products::faction_details::FactionDetailParams {
            branch_id: "1".into(),
            hour,
            faction: faction.into(),
        },
        context,
        &AtomicBool::new(false),
    )
    .map_err(|why| why.to_string())?;
    if let Some(reason) = reply.goal_reason {
        return Ok(serde_json::json!({ "reason": reason }));
    }
    Ok(serde_json::json!({
        "goals": reply.goals.iter().map(|goal| serde_json::json!([goal.ordinal, shown(&goal.definition), goal.status,
            Some(goal.progress_raw.clone()), goal.progress_percent])).collect::<Vec<_>>(),
        "completed": reply.completed.iter().map(shown).collect::<Vec<_>>(),
        "cancelled": reply.cancelled.iter().map(shown).collect::<Vec<_>>(),
    }))
}
/// Prepares each block through the family's change pre-pass and fold, with `restore` continuing from saved state.
fn prepare_by_blocks(source: &Source, context: &LoadContext, restore: bool) -> Fixture {
    use replay::container::BlockChange;
    let replay = source.replay_ref();
    let files = Fixture::new();
    let flag = AtomicBool::new(false);
    let fields = (block::FAMILY.fields.unwrap())().unwrap();
    let mut state = Prepared::default();
    let mut previous: Option<i32> = None;
    for index in &replay.index {
        let mut extract = (block::FAMILY.start)(replay.format);
        extract.read_by_changes();
        replay
            .read_block_changes(
                index,
                &fields,
                |change| match change {
                    BlockChange::Changed { frame, rebuilt, .. } => {
                        let continues =
                            previous.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
                        previous = Some(frame.hour);
                        let changes =
                            FrameChanges::rebuilt(frame, replay.format, continues, |table| {
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
        block::fold(
            &mut state,
            extract.finish().unwrap(),
            source,
            context,
            replay,
            index,
            &mut |_, result| result.map(|_| true),
            &mut || Ok(()),
            &flag,
        )
        .unwrap();
        publish(&mut state, &files);
        if restore {
            let file = files
                .store
                .read(&files.key, &prepared::state_section(1), 0)
                .unwrap()
                .unwrap();
            state = serde_json::from_reader(zstd::stream::Decoder::new(file).unwrap()).unwrap();
        }
    }
    files
}
const GOAL_CONTENT: &str = "mod_goal = { name = MOD_GOAL description = MOD_GOAL_DESC category = short_term group = mod_group }\nmod_scripted = { name = MOD_SCRIPTED category = long_term }\n";
const GOAL_NAMES: &str = "\u{feff}l_english:\n MOD_GOAL:0 \"Mod goal\"\n MOD_GOAL_DESC:0 \"A modded goal\"\n MOD_SCRIPTED:0 \"[ROOT.GetTag] pact goal\"\n SHORT_TERM:0 \"Mod short term\"\n";
/// Faction 11 founded at 103 after an unreadable roster at 102, the dictionary lost at 104, the viewer changing from 109.
fn goal_hour(hour: i32) -> DecodedFrame {
    let mut entries = match hour {
        100 => vec![held(9, 0, 0, 1, 0, 25_000)],
        101 => vec![held(9, 0, 0, 1, 0, 50_000)],
        _ => vec![held(9, 0, 0, 2, 0, 1_000 * i64::from(hour)), held(9, 1, 0, 1, 0, 0)],
    };
    if hour >= 105 {
        entries.push(held(11, 0, 0, 1, 4, i64::MIN));
    }
    let mut frame = goal_frame(hour, entries);
    let mut status = vec![FactionGoalStatusRow {
        faction: 9,
        manifest: None,
    }];
    if hour >= 103 {
        status.push(FactionGoalStatusRow {
            faction: 11,
            manifest: None,
        });
    }
    *frame.tables[TableId::FactionGoalStatus.index()] =
        TableView::Complete(TableRows::FactionGoalStatus(status));
    let mut factions = vec![9, 10];
    if hour >= 103 {
        factions.push(11);
    }
    *frame.tables[TableId::Factions.index()] = if hour == 102 {
        TableView::Unavailable {
            reason: "synthetic roster gap".into(),
        }
    } else {
        TableView::Complete(TableRows::Factions(
            factions
                .into_iter()
                .map(|id| FactionRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ))
    };
    if hour == 104 {
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "synthetic dictionary gap".into(),
        };
    }
    if hour >= 109 {
        *frame.tables[TableId::LocalPlayer.index()] =
            TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                country: 101,
                observer_country: 0,
            }]));
    }
    if hour >= 110
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"mod_b".to_vec();
    }
    frame
}
/// Four recorded blocks of three hours each, the first one's replay given.
fn goal_blocks(first: Arc<replay::container::Replay>, revision: &str) -> Source {
    let hours: Vec<_> = (100..112).map(goal_hour).collect();
    let mut replay = first;
    for chunk in hours[3..].chunks(3) {
        replay = Arc::new(
            replay
                .append(&[fixtures::continuation_records(&replay, 1, chunk.to_vec())])
                .unwrap(),
        );
    }
    assert_eq!(replay.index.len(), 4);
    Source::from_replay(
        SourceKey {
            source_id: "faction-goal-parity".into(),
            revision: revision.into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap()
}
/// The only hour any faction's history may leave to its details: 11's, before founding, over an unreadable roster.
const UNPROVEN: (&str, i32) = ("11", 102);
/// Every hour of factions 9, 10 and 11 answered by the history, equal to that hour's own faction details.
fn assert_goal_parity(source: &Source, context: &LoadContext, files: &Fixture, run: &str) {
    for faction in ["9", "10", "11"] {
        let reply = goals_reply(source, files, context, goals_params(faction, None, None), 111, true);
        for hour in 100..112 {
            let at = format!("{run}: faction {faction} hour {hour}");
            let history = goals_at(&reply, hour);
            let details = details_at(source, context, faction, hour);
            if (faction, hour) == UNPROVEN {
                assert_eq!(history, None, "an unreadable roster is no proof of absence: {at}");
                assert!(details.is_ok(), "{at}");
                continue;
            }
            let history = history.unwrap_or_else(|| panic!("the history must answer {at}"));
            match details {
                Ok(details) => assert_eq!(history, details, "{at}"),
                Err(why) => {
                    assert!(why.contains("absent"), "{at}: {why}");
                    assert_eq!(history, serde_json::json!("absent"), "{at}");
                }
            }
        }
    }
    assert_eq!(
        goals_at(&goals_reply(source, files, context, goals_params("11", None, None), 111, true), 101),
        Some(serde_json::json!("absent")),
        "{run}: a complete roster without the faction proves it absent"
    );
    let nine = goals_reply(source, files, context, goals_params("9", None, None), 111, true);
    // The lost dictionary refuses its names at 104; its return names them again.
    assert_ne!(goals_at(&nine, 104), goals_at(&nine, 105), "{run}");
    assert_eq!(
        goals_at(&nine, 103),
        goals_at(&nine, 105).map(|mut value| {
            // The unavailable viewer's refusal names the reached hour, not the goal's sample.
            if context.content.is_some() {
                let reason = |hour| serde_json::json!(format!(
                    "Faction goal artwork: group is not authored; hour {hour} LocalPlayer is unavailable: outside country fixture"
                ));
                assert_eq!(value["goals"][0][1][6], reason(105), "{run}");
                value["goals"][0][1][6] = reason(103);
            }
            value["goals"][0][3] = serde_json::json!("103000");
            value["goals"][0][4] = serde_json::json!(103.0);
            value
        }),
        "{run}"
    );
}
#[test]
fn goal_history_without_content_shows_every_hour_as_that_hours_faction_details_reply() {
    let first = (100..103).map(goal_hour).collect();
    let first = Arc::new(
        replay::container::Replay::from_records(
            fixtures::CAMPAIGN.into(),
            &[fixtures::records(1, first)],
        )
        .unwrap(),
    );
    let source = goal_blocks(first, "synthetic");
    let context = LoadContext { content: None };
    for restore in [false, true] {
        let files = prepare_by_blocks(&source, &context, restore);
        assert_goal_parity(&source, &context, &files, &format!("synthetic, restore {restore}"));
    }
}
#[test]
fn goal_history_with_mod_content_shows_every_hour_as_that_hours_faction_details_reply() {
    let machine = dashboard_base::test_fixtures::visual_mods::Machine::new("faction_goal_parity");
    machine.write("common/factions/goals/mod.txt", GOAL_CONTENT.as_bytes());
    machine.write("localisation/english/mod_l_english.yml", GOAL_NAMES.as_bytes());
    let (first, options) = machine.open_frames((100..103).map(goal_hour).collect());
    let source = goal_blocks(first.replay(), "mod content");
    let context = LoadContext {
        content: Some(options),
    };
    for restore in [false, true] {
        let files = prepare_by_blocks(&source, &context, restore);
        assert_goal_parity(&source, &context, &files, &format!("mod content, restore {restore}"));
    }
}
#[test]
fn a_goal_prefix_names_later_hours_pending_and_leaks_no_later_list() {
    let hours = vec![
        goal_frame(100, vec![held(9, 0, 0, 1, 0, 25_000)]),
        goal_frame(101, vec![held(9, 1, 0, 1, 0, 0)]),
    ];
    let source = source(hours.clone());
    let mut state = Prepared::default();
    for frame in &hours {
        observe(&mut state, &source, frame);
    }
    let files = Fixture::new();
    publish(&mut state, &files);
    let context = LoadContext { content: None };
    let reply = goals_reply(&source, &files, &context, goals_params("9", None, None), 100, false);
    assert_eq!(reply.pending[0].first_hour, 101);
    assert!(reply.runs.iter().all(|run| run.last_hour <= 100));
    assert_eq!(reply.lists.len(), 1);
    assert!(reply.lists[0].completed.is_empty());
    assert!(goals_at(&reply, 101).is_none());
    // Asked hours clip the runs, the progress and the pending hours alike.
    let early = goals_reply(&source, &files, &context, goals_params("9", Some(100), Some(100)), 101, true);
    let late = goals_reply(&source, &files, &context, goals_params("9", Some(101), None), 101, true);
    assert_eq!((early.runs.len(), early.runs[0].last_hour), (1, 100));
    assert!(early.lists[0].completed.is_empty());
    assert_eq!(late.runs[0].first_hour, 101);
    assert_eq!(late.lists[0].completed.len(), 1);
    assert!(late.progress.is_empty());
    assert!(early.pending.is_empty() && late.pending.is_empty());
}
